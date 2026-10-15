//! 曲线展平：把贝塞尔转成折线，误差有**可断言的上界**。
//!
//! # 误差判据是弦高，不是控制点距离
//!
//! 常见做法是按控制多边形长度或 `2^n` 固定层数切分，但两者都与实际误差无关：
//! 一条控制点很长的**直线**段会被切出很多点，而一条控制点很短的**急弯**只切
//! 几层就明显失真。这里用**弦高**（控制点到弦的最大垂直距离）作为判据 —— 它
//! 直接就是"曲线与折线的最大偏差"的上界，于是有了可断言的性质：
//!
//! > 对任意输入，展平后每条曲线与其折线近似的最大偏差 ≤ [`TOLERANCE`]。

use super::{Contour, AREA_EPS};
use crate::core::Point;
use crate::icon::Segment;

/// 默认弦高容差（**屏幕像素 / DIP**）。
///
/// # 为什么是 0.25 而不是更小
///
/// 24 DIP 的图标在 200% 缩放下是 48 物理像素，0.25 DIP 对应 0.5 物理像素——
/// 小于一个像素，且远低于 MSAA 能体现的粒度。再往下压只是白费 CPU：一个典型
/// 图标（齿轮，30+ 段）会从 ~200 点涨到 ~600 点，而三角化是O(n²) 的耳切。
///
/// 反过来也不能放大：0.5 DIP 在 24px 图标上会让圆的边缘出现可见的多边形棱。
pub const TOLERANCE: f32 = 0.25;

/// 递归细分的**深度上限**。
///
/// 存在它不是为了限制精度，而是为了**保证终止**：弦高判据对病态输入
/// （坐标含NaN / Inf、或极小半径的多圈螺旋）可能永远不满足，没有硬上限的话
/// 表现为栈溢出 —— 一个畸形图标资源就能让整个界面崩掉。
///
/// 16 层 =65536 段，远超任何真实需要，而深度耗尽时多边形化的后果只是边缘略糙。
const MAX_DEPTH: u32 = 16;

/// 展平结果：若干条闭合轮廓。
pub type FlatPath = Vec<Contour>;

/// 把路径段展平成折线轮廓。
///
/// - [`Segment::MoveTo`] 开一条新轮廓；
/// - 未显式 [`Segment::Close`] 的子路径在填充时按 SVG 规范**视为闭合**，
///   所以这里也生成闭合轮廓；
/// - 落在一个点上的子路径（`MoveTo` 后无任何绘制命令）被丢弃 —— 它没有面积。
pub fn flatten(segments: &[Segment], tolerance: f32) -> FlatPath {
    let tol = if tolerance > 0.0 {
        tolerance
    } else {
        TOLERANCE
    };
    let mut out: FlatPath = Vec::new();
    let mut cur: Vec<Point> = Vec::new();
    let mut cursor = Point::default();
    let mut sub_start = Point::default();

    for seg in segments {
        match *seg {
            Segment::MoveTo(x, y) => {
                flush(&mut cur, &mut out);
                cursor = Point { x, y };
                sub_start = cursor;
                cur.push(cursor);
            }
            Segment::LineTo(x, y) => {
                let p = Point { x, y };
                // 隐式 MoveTo：path 以绘制命令开头（SVG 规范允许但罕见）。
                // 按 (0,0) 起笔画会让形状整体偏移，所以显式记一笔。
                if cur.is_empty() {
                    cur.push(sub_start);
                }
                push_if_distinct(&mut cur, p);
                cursor = p;
            }
            Segment::CubicTo(c1x, c1y, c2x, c2y, x, y) => {
                if cur.is_empty() {
                    cur.push(sub_start);
                }
                let p3 = Point { x, y };
                flatten_cubic(
                    cursor,
                    Point { x: c1x, y: c1y },
                    Point { x: c2x, y: c2y },
                    p3,
                    tol,
                    0,
                    &mut cur,
                );
                cursor = p3;
            }
            Segment::QuadraticTo(cx, cy, x, y) => {
                if cur.is_empty() {
                    cur.push(sub_start);
                }
                let p2 = Point { x, y };
                flatten_quadratic(cursor, Point { x: cx, y: cy }, p2, tol, 0, &mut cur);
                cursor = p2;
            }
            Segment::Close => {
                push_if_distinct(&mut cur, sub_start);
                flush(&mut cur, &mut out);
                cursor = sub_start;
                // Close 之后允许继续绘制（SVG 允许），那属于新的子路径。
                cur.push(sub_start);
            }
        }
    }
    flush(&mut cur, &mut out);
    out
}

/// 收尾一条轮廓：点数足够、有面积就收下。
fn flush(cur: &mut Vec<Point>, out: &mut FlatPath) {
    if cur.len() >= 3 {
        let c = Contour::new(core::mem::take(cur));
        // 面积判据而非仅点数：一个点集可能退化到共线（例如三次贝塞尔被展成一条
        // 直线），点数够但面积为零，交给耳切法会死循环。
        if c.len() >= 3 && super::abs_area(c.points()) > AREA_EPS {
            out.push(c);
        }
        return;
    }
    cur.clear();
}

fn push_if_distinct(cur: &mut Vec<Point>, p: Point) {
    if let Some(last) = cur.last() {
        if super::same_point_public(*last, p) {
            return;
        }
    }
    cur.push(p);
}

/// 三次贝塞尔 → 折线。
fn flatten_cubic(
    p0: Point,
    p1: Point,
    p2: Point,
    p3: Point,
    tol: f32,
    depth: u32,
    out: &mut Vec<Point>,
) {
    if depth >= MAX_DEPTH || cubic_flat_enough(p0, p1, p2, p3, tol) {
        out.push(p3);
        return;
    }
    // de Casteljau 在 t=0.5 处一分为二
    let (p01, p12, p23) = (mid(p0, p1), mid(p1, p2), mid(p2, p3));
    let (p012, p123) = (mid(p01, p12), mid(p12, p23));
    let mid_pt = mid(p012, p123);
    flatten_cubic(p0, p01, p012, mid_pt, tol, depth + 1, out);
    flatten_cubic(mid_pt, p123, p23, p3, tol, depth + 1, out);
}

/// 二次贝塞尔 → 折线。
///
/// 提升为三次再递归：提权公式 `c1 = p0 + 2/3 (c - p0)`、`c2 = p2 + 2/3 (c - p2)`
/// 是精确的（不是近似），所以比单独写一套二次递归更短且误差界一致。
fn flatten_quadratic(p0: Point, c: Point, p2: Point, tol: f32, depth: u32, out: &mut Vec<Point>) {
    let c1 = Point {
        x: p0.x + (2.0 / 3.0) * (c.x - p0.x),
        y: p0.y + (2.0 / 3.0) * (c.y - p0.y),
    };
    let c2 = Point {
        x: p2.x + (2.0 / 3.0) * (c.x - p2.x),
        y: p2.y + (2.0 / 3.0) * (c.y - p2.y),
    };
    flatten_cubic(p0, c1, c2, p2, tol, depth, out);
}

/// 三次贝塞尔的弦高判据。
///
/// 用控制点到弦的最大垂直距离。数学上：曲线上的点都是控制点的凸组合，
/// 而到一条直线的距离是**线性函数**，所以曲线上任一点到弦的距离不超过控制点
/// 到弦距离的最大值。于是这个量就是误差上界本身，不是启发式估计。
fn cubic_flat_enough(p0: Point, p1: Point, p2: Point, p3: Point, tol: f32) -> bool {
    // 弦退化成点（p0≈p3）：距离判据失效，退化为控制点到 p0 的距离。
    if (p3.x - p0.x).abs() <= super::POINT_EPS && (p3.y - p0.y).abs() <= super::POINT_EPS {
        let d1 = dist(p1, p0);
        let d2 = dist(p2, p0);
        return d1.max(d2) <= tol;
    }
    let d1 = distance_to_line(p1, p0, p3);
    let d2 = distance_to_line(p2, p0, p3);
    d1.max(d2) <= tol
}

fn mid(a: Point, b: Point) -> Point {
    Point {
        x: (a.x + b.x) * 0.5,
        y: (a.y + b.y) * 0.5,
    }
}

fn dist(a: Point, b: Point) -> f32 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt()
}

fn distance_to_line(p: Point, a: Point, b: Point) -> f32 {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let len = (dx * dx + dy * dy).sqrt();
    if len <= super::POINT_EPS {
        return dist(p, a);
    }
    ((p.x - a.x) * dy - (p.y - a.y) * dx).abs() / len
}

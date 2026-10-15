//! 描边轮廓化：把"中心线 + 宽度"变成可填充的闭合轮廓。
//!
//! # 描边为什么不能"照着中心线画一条粗线"
//!
//! 中心线上的一个点会变成一个**矩形片**（法向两侧各推半个宽度），转角处还要补上
//! **接头**（joint）把两个矩形片之间的楔形缺口填上，端点还要补**端帽**（cap）。
//! 漏掉任何一个的症状都不是"少一块"，而是"描边在某处变细/变粗然后接缝处裂开"
//! ——而那种错误极难定位，因为它只在某些角度出现。
//!
//! # 本模块只负责"轮廓"，不负责"三角化"
//!
//! 输出仍是 [`Contour`] 列表，与填充路径共用同一个三角化器。这是刻意的：描边与
//! 填充的三角化是完全相同的问题，**写两遍必然漂移**。
//!
//! # 两种拓扑
//!
//! | 中心线 | 描边区域 | 本模块产出 |
//! |---|---|---|
//! | 开放（一段线） | 单一"跑道形"多边形 | 一个轮廓 |
//! | 闭合（一圈线） | **带孔**的环带 | 外轮廓 + 内孔 |
//!
//! 闭合时内孔的绕向必须与外轮廓相反，否则非零填充会把它填实——症状是"描边看起来
//! 是实心块"，很容易被误判成"填充规则实现错了"。

use super::{Contour, TOLERANCE};
use crate::core::Point;

/// 端点样式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineCap {
    /// 平截（SVG 默认）。端点垂直于中心线方向切平。
    Butt,
    /// 半圆。
    Round,
    /// 方形：向外延伸半个宽度。
    Square,
}

/// 接头样式（转角处）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineJoin {
    /// 尖角延长到两条偏移线的交点；超过 [`StrokeStyle::miter_limit`] 时退化为
    /// [`LineJoin::Bevel`]。
    Miter,
    /// 圆弧。
    Round,
    /// 平切。
    Bevel,
}

/// 描边参数。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StrokeStyle {
    /// 线宽（DIP）。
    pub width: f32,
    /// 端点样式。
    pub cap: LineCap,
    /// 接头样式。
    pub join: LineJoin,
    /// 尖角外伸长度 / 线宽 的上限。`4.0` 是 SVG 默认。
    ///
    /// `1.0` 意味着"尖角最多外伸一个线宽"，等价于强制 bevel。
    pub miter_limit: f32,
}

impl Default for StrokeStyle {
    fn default() -> Self {
        Self {
            width: 1.0,
            cap: LineCap::Butt,
            join: LineJoin::Miter,
            miter_limit: 4.0,
        }
    }
}

/// 生成描边区域的闭合轮廓。
///
/// `closed` 指出中心线是否首尾相连。展平出来的每条 [`Contour`] 都应传 `true`
/// ——SVG 规范规定未显式 `Z` 的子路径在填充时也视为闭合。
///
/// # 返回值的顺序
///
/// 闭合中心线返回 `[外轮廓, 内孔]`，开放中心线返回一个轮廓。调用方按
/// [`crate::icon::FillRule::NonZero`] 交给三角化器即可，内孔靠相反绕向自动挖空。
pub fn stroke_outline(points: &[Point], closed: bool, style: &StrokeStyle) -> Vec<Contour> {
    let hw = style.width * 0.5;
    if !hw.is_finite() || hw <= 0.0 {
        return Vec::new();
    }
    let path = dedup(points);
    // 闭合中心线至少要三个点：两点构成的"闭合"是一条来回重叠的线段，
    // 描边出来是零面积。
    if closed && path.len() < 3 {
        return Vec::new();
    }
    if path.len() < 2 {
        return Vec::new();
    }

    let front = offset_side(&path, hw, style, true, closed);
    let mut back = offset_side(&path, hw, style, false, closed);

    let mut out: Vec<Contour> = Vec::new();
    if closed {
        // 闭合：两条偏移线里，**面积大的那条是外轮廓**，小的是内孔。
        //
        // # 为什么不能靠"正侧就是外侧"来省掉这次比较
        //
        // 正侧的偏移量是 `(p.x − d.y·hw, p.y + d.x·hw)`。对方向 `d = (1, 0)`
        // （向右），它给出 `p.y + hw` —— 在 **y 向下**的坐标系里那是**向下**，
        // 也就是**指向图形内部**。所以"正侧 = 外侧"这个直觉在 SVG 坐标系里
        // 是反的：实测正方形中心线 `(0,0)(10,0)(10,10)(0,10)`、宽 2 时，
        // 正侧环是 8×8 = 64（内孔），负侧环才是 12×12 = 144（外轮廓）。
        //
        // 于是这里按**面积**判定内外 —— 它与绕向、坐标朝向都无关，
        // 是唯一不会因为换了坐标约定而静默画反的判据。
        let front_area = super::abs_area(&front);
        let back_area = super::abs_area(&back);
        let (outer_pts, mut inner_pts) = if front_area >= back_area {
            (front, back)
        } else {
            (back, front)
        };
        // 内孔必须与外轮廓**反向**，否则非零填充会把它填实 —— 症状是
        // "描边看起来是实心块"，很容易被误判成填充规则实现错了。
        if super::signed_area2(&outer_pts).signum() == super::signed_area2(&inner_pts).signum() {
            inner_pts.reverse();
        }
        // 两条都归一到"shoelace 为正"，于是"哪条是外"不再依赖调用者传的绕向。
        if super::signed_area2(&outer_pts) < 0.0 {
            let mut o = outer_pts;
            o.reverse();
            push_ring(&mut out, o);
        } else {
            push_ring(&mut out, outer_pts);
        }
        push_ring(&mut out, inner_pts);
        return out;
    }

    // ---- 开放中心线 ----
    //
    // 跑道形 = 正侧 + 末端帽 + 负侧反向 + 起始帽。三种端帽里只有**方形**需要改
    // 中心线本身（首尾各延长半宽），另两种是在偏移线末端补点——所以方形单独
    // 处理，另两种在这里补。
    if style.cap == LineCap::Square {
        // 方形端帽：延长中心线，再照常偏移。于是两端都外伸半宽。
        let mut path2 = path.clone();
        let d0 = first_dir(&path);
        let d1 = last_dir(&path);
        let a = path2[0];
        let b = path2[path2.len() - 1];
        path2[0] = Point {
            x: a.x - d0.x * hw,
            y: a.y - d0.y * hw,
        };
        let n = path2.len();
        path2[n - 1] = Point {
            x: b.x + d1.x * hw,
            y: b.y + d1.y * hw,
        };
        let front = offset_side(&path2, hw, style, true, false);
        let mut back = offset_side(&path2, hw, style, false, false);
        back.reverse();
        let mut ring = front;
        ring.extend(back);
        push_ring(&mut out, ring);
        return out;
    }

    // Butt / Round：末端帽在偏移线末端补
    let mut ring = front;
    append_cap(
        &mut ring,
        path[path.len() - 1],
        last_dir(&path),
        hw,
        style.cap,
    );
    back.reverse();
    ring.extend(back);
    append_cap(&mut ring, path[0], first_dir(&path), hw, style.cap);
    push_ring(&mut out, ring);
    out
}

/// 单侧偏移线，含全部接头。
///
/// `positive` 选法向哪一侧。两侧用**同一个** `sign`，于是正侧与负侧天然等距 ——
/// 描边宽度就等于两者的间距，可以直接量出来验证。
///
/// # 环由「接头」顺序拼成，不逐点输出偏移点
///
/// 每个顶点输出一次接头（见 [`append_join`]），相邻两个接头之间的那段偏移线
/// 本身就是直线，**不需要**再补点。这一点是本函数与朴素写法的全部差别，也是
/// 闭合描边能不能三角化的关键（见 [`append_join`] 里关于尖刺的说明）。
fn offset_side(
    path: &[Point],
    hw: f32,
    style: &StrokeStyle,
    positive: bool,
    closed: bool,
) -> Vec<Point> {
    let n = path.len();
    let sign = if positive { 1.0 } else { -1.0 };

    // 第 `seg` 段的左法向（`sign` 决定取哪一侧）。
    let left_normal = |seg: usize| -> Point {
        let d = segment_dir(path, seg, closed);
        Point {
            x: -d.y * sign,
            y: d.x * sign,
        }
    };
    let shifted = |at: usize, seg: usize| -> Point {
        let nv = left_normal(seg);
        let p = path[at];
        Point {
            x: p.x + nv.x * hw,
            y: p.y + nv.y * hw,
        }
    };
    // 顶点 `i` 在**第 `i` 段**偏移线上的起点。
    let line_start = |i: usize| -> Point { shifted(i, i) };
    // 顶点 `i` 在**第 `i-1` 段**偏移线上的终点。
    let line_end_at = |i: usize| -> Point { shifted(i, if i == 0 { n - 1 } else { i - 1 }) };

    let mut out: Vec<Point> = Vec::with_capacity(n + 8);

    if !closed {
        // 开放：首尾两端没有接头，各补一个端点。中间每个顶点一次接头。
        out.push(line_start(0));
        for i in 1..n - 1 {
            append_join(
                &mut out,
                line_end_at(i),
                path[i],
                line_start(i),
                segment_dir(path, i - 1, false),
                segment_dir(path, i, false),
                hw,
                style,
            );
        }
        out.push(line_end_at(n - 1));
        return out;
    }

    // 闭合：环的首点就是顶点 0 的接头，依次拼到顶点 n-1，末端与首点自动闭合。
    for i in 0..n {
        let seg_prev = if i == 0 { n - 1 } else { i - 1 };
        append_join(
            &mut out,
            line_end_at(i),
            path[i],
            line_start(i),
            segment_dir(path, seg_prev, true),
            segment_dir(path, i, true),
            hw,
            style,
        );
    }
    out
}
#[allow(clippy::too_many_arguments)]
// 参数是画一次所需的全部输入，彼此没有可成组打包的语义 —— 凑一个 struct 只是把
// 8 个参数搬进另一个名字，读的人反而要多跳一层。见 AGENTS.md「代码自解释优先」。
/// 在 `prev_end` 与 `next_start` 之间补接头。
///
/// - `prev_end`：入边（第 `i-1` 段）偏移线的**终点**（顶点 i 处）
/// - `next_start`：出边（第 `i` 段）偏移线的**起点**（顶点 i 处）
///
/// # 为什么 miter 合格时**只输出尖角**，不输出 `prev_end` / `next_start`
///
/// 这是本模块最容易错的一处，症状是"闭合描边三角化直接失败"，而且失败点在
/// 三角化器里，跟描边八竿子打不着，容易查错方向。
///
/// 朴素写法是"输出尖角，再输出 `next_start`"（本函数早期版本就是这么写的）。
/// 错在**凸角处 `next_start` 落在环外**：
///
/// ```text
///   外轮廓（负侧，宽 2，中心线10×10）      内孔（正侧）
///   (0,-1) ──────── (11,-1)                (1,1) ────── (9,1)
///   (11,0) ←──多余── (11,-1)                (9,0) ←多余─ (9,1)
///   (11,11)                                (9,9)
/// ```
///
/// 两条相邻偏移线在凸角处相交于尖角，而 `next_start` 是**出边偏移线自己的端点**，
/// 它落在尖角**之前**（沿偏移线方向）。先输出尖角再输出它，环就会"退回去一截"
/// 形成一个**面积为零的尖刺**（spike）。
///
/// 尖刺在非零填充下不算错（环绕数相消，环带面积仍对），但它是**自交退化多边形**：
/// 三角化器的耳测试要求「三角形内不含其他顶点」，尖刺的重复顶点会把每一个候选耳
/// 都否掉，于是报 `找不到可切的耳`。实测正方形环带的内孔（只有8 个点、单个孔）
/// 单独三角化就是这个下场。
///
/// 只输出尖角之后，环变成 `… (1,1) → (9,1) → (9,9) → (1,9) → 闭合` ——
/// 四个点，就是那个 8×8 的方，一点不多。
///
/// # 为什么相邻两个接头之间不用补点
///
/// 偏移线本身是直线，接头 i 的输出与接头 i+1 的输出之间的连线就是第 i 段偏移线
/// 的全部，中间没有信息。补点只会引入上面那种尖刺。
///
/// # miter 交点用**两条偏移线的直线求交**，不是角平分线放大
///
/// 角平分线式的启发式在转角接近 180°（折返）时给出的点不在任何一条偏移线上，
/// 尖角会明显偏离两侧边缘；直线求交对所有非退化情形都精确。折返时两线平行，
/// `line_intersection` 返回 `None`，退化为 bevel —— 这正是 SVG 的规定行为。
fn append_join(
    out: &mut Vec<Point>,
    prev_end: Point,
    vertex: Point,
    next_start: Point,
    d_in: Point,
    d_out: Point,
    hw: f32,
    style: &StrokeStyle,
) {
    // 共线：入边与出边偏移线在顶点处重合，无需接头。
    if (prev_end.x - next_start.x).abs() <= super::POINT_EPS
        && (prev_end.y - next_start.y).abs() <= super::POINT_EPS
    {
        return;
    }
    match style.join {
        LineJoin::Bevel => {
            out.push(prev_end);
            out.push(next_start);
        }
        LineJoin::Round => {
            for p in arc_between(prev_end, vertex, next_start, hw, arc_segments(hw)) {
                out.push(p);
            }
        }
        LineJoin::Miter => {
            match line_intersection(prev_end, d_in, next_start, d_out) {
                Some(tip) => {
                    // 交点距顶点 = hw / cos(θ/2)，除以 hw 即 1/cos(θ/2)——
                    // 正是 SVG 用线宽为单位的 miter-length。
                    let ratio = dist(tip, vertex) / hw;
                    if ratio.is_finite() && ratio <= style.miter_limit.max(1.0) {
                        // 只输出尖角（理由见函数文档）。
                        out.push(tip);
                    } else {
                        // 超限（或退化）→ 平切到出边，即 bevel。
                        out.push(prev_end);
                        out.push(next_start);
                    }
                }
                // 偏移线平行（折返 180°）：无交点，退化为 bevel。
                None => {
                    out.push(prev_end);
                    out.push(next_start);
                }
            }
        }
    }
}

/// 端帽：把 `at` 处的偏移线末端按样式补齐。
fn append_cap(out: &mut Vec<Point>, at: Point, dir: Point, hw: f32, cap: LineCap) {
    match cap {
        LineCap::Butt => {}
        LineCap::Square => {
            if let Some(last) = out.last_mut() {
                last.x += dir.x * hw;
                last.y += dir.y * hw;
            }
        }
        LineCap::Round => {
            let n = Point {
                x: -dir.y * hw,
                y: dir.x * hw,
            };
            for p in arc_between(
                Point {
                    x: at.x + n.x,
                    y: at.y + n.y,
                },
                at,
                Point {
                    x: at.x - n.x,
                    y: at.y - n.y,
                },
                hw,
                arc_segments(hw),
            ) {
                out.push(p);
            }
        }
    }
}

/// 把点列表收成一条轮廓（点数与面积不达标就丢弃）。
fn push_ring(out: &mut Vec<Contour>, pts: Vec<Point>) {
    let c = Contour::new(pts);
    if c.len() >= 3 && c.is_fillable() {
        out.push(c);
    }
}

fn segment_dir(path: &[Point], i: usize, closed: bool) -> Point {
    let n = path.len();
    let (a, b) = if closed {
        (path[i], path[(i + 1) % n])
    } else if i + 1 < n {
        (path[i], path[i + 1])
    } else {
        (path[i - 1], path[i])
    };
    unit_dir(a, b)
}

fn first_dir(path: &[Point]) -> Point {
    unit_dir(path[0], path[1])
}

fn last_dir(path: &[Point]) -> Point {
    unit_dir(path[path.len() - 2], path[path.len() - 1])
}

fn unit_dir(a: Point, b: Point) -> Point {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let len = (dx * dx + dy * dy).sqrt();
    if len <= super::POINT_EPS {
        Point { x: 1.0, y: 0.0 }
    } else {
        Point {
            x: dx / len,
            y: dy / len,
        }
    }
}

/// 两条直线（过 `a` 方向 `da`；过 `b` 方向 `db`）的交点。平行时 `None`。
fn line_intersection(a: Point, da: Point, b: Point, db: Point) -> Option<Point> {
    let denom = da.x * db.y - da.y * db.x;
    if denom.abs() <= super::POINT_EPS {
        return None;
    }
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let t = (dx * db.y - dy * db.x) / denom;
    Some(Point {
        x: a.x + da.x * t,
        y: a.y + da.y * t,
    })
}

/// 在圆上从 `a` 沿短弧插点到 `b`（圆心 `c`、半径 `r`）。用于 round 接头与端帽。
fn arc_between(a: Point, c: Point, b: Point, r: f32, segs: usize) -> Vec<Point> {
    let a0 = (a.y - c.y).atan2(a.x - c.x);
    let mut a1 = (b.y - c.y).atan2(b.x - c.x);
    // 取短弧
    while a1 - a0 > core::f32::consts::PI {
        a1 -= core::f32::consts::TAU;
    }
    while a0 - a1 > core::f32::consts::PI {
        a1 += core::f32::consts::TAU;
    }
    (1..segs)
        .map(|i| {
            let t = a0 + (a1 - a0) * (i as f32 / segs as f32);
            Point {
                x: c.x + r * t.cos(),
                y: c.y + r * t.sin(),
            }
        })
        .collect()
}

/// 圆弧段数：半径越大、容差越小，需要越多段。
///
/// # 上限 24 的理由
///
/// 直径 24 DIP 的圆弧，24 段已经让弦高误差低于 0.01 像素——远超 MSAA 能体现的
/// 粒度。再多是纯 CPU 浪费，而圆弧段数直接乘进三角化规模。
fn arc_segments(r: f32) -> usize {
    let tol = TOLERANCE.max(r * 1e-3);
    (std::f32::consts::PI / (2.0 * (tol / r.max(tol)).clamp(1e-6, 2.0)).acos())
        .ceil()
        .clamp(2.0, 24.0) as usize
}

fn dist(a: Point, b: Point) -> f32 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt()
}

fn dedup(points: &[Point]) -> Vec<Point> {
    let mut out: Vec<Point> = Vec::with_capacity(points.len());
    for &p in points {
        if out.last().is_some_and(|q| super::same_point_public(*q, p)) {
            continue;
        }
        out.push(p);
    }
    out
}

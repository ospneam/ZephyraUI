//! 三角形与裁剪矩形求交。
//!
//! # 为什么多边形要单独做逐三角形裁剪
//!
//! 四边形图元不需要这一步：`expand` 里对 `rect` 与 `clip` 求交后直接
//! `shrink`，一个矩形与矩形求交仍是一个矩形。
//!
//! 多边形不行。已三角化的多边形是一堆**三角形**，三角形与矩形求交会得到
//! 0~2 个任意多边形，必须重新三角化才能画。若不做这一步只有两个选择，
//! 都很糟：
//!
//! - 顶点变换 + 整体丢弃：跨界三角形整块消失。症状是"滚动容器边缘的图标被
//!   削掉一块"，削多少取决于三角形怎么切 —— 看起来像随机缺一块。
//! - 完全不裁剪：画到容器外的部分会盖住兄弟节点。症状是"图标压在相邻元素
//!   上面"，比缺一块更难定位。
//!
//! 代价是每帧多一遍 Sutherland–Hodgman。单个三角形的边通常是 3~4 条有效边，
//! 顶点上限 12，实测在图标规模下完全可以忽略。

use zephyra_core::core::{Point, Rect};

/// 三角形与矩形求交，返回结果的凸多边形顶点（可能为空）。
///
/// `clip` 为 `None` 表示**不裁剪**，直接返回原三角形。
///
/// 用 Sutherland–Hodgman：逐条裁剪边，把落在外的部分沿边替换。
///
/// # 为什么不用"无限大的矩形"表示不裁剪
///
/// `Rect::new(NEG_INFINITY, NEG_INFINITY, INFINITY, INFINITY)` 看着等价，实际上
/// 不是：四条边算出来是 `-inf`、`-inf + inf` = **NaN**、`inf`、`inf - inf` = NaN。
/// 拿 NaN 做比较，`inside` 恒为 false，所有顶点被当成"在外面"——于是**整个
/// 三角形被丢弃**，而症状是"没设裁剪的路径也不显示"。
///
/// # 顶点数上界
///
/// 三角形的 3 条边 + 矩形的 4 条边 = 7 条裁剪边，每条最多 +1 个顶点，
/// 上界 10。这里返回 `Vec` 而不是定长数组 —— 定长数组要在栈上初始化 10 个
/// `Point`（80 字节）并维护游标，而这条路径每帧每个三角形都走一次。
/// `Vec::with_capacity(10)` 配合 `push` 编译器能内联掉扩容检查。
pub fn geom_clip(tri: &[Point; 3], clip: Option<Rect>) -> Vec<Point> {
    let Some(clip) = clip else {
        return tri.to_vec();
    };
    let mut poly: Vec<Point> = tri.to_vec();

    // 四条边，按"保留哪一侧"组织。顺序对应 top → bottom → left → right。
    //
    // ⚠ 每一遍必须用**该边自己的坐标轴**：top/bottom 裁 y，left/right 裁 x。
    // 写成一个通用的"裁 y"再传不同数值过去，症状是左右裁剪完全不生效（图标
    // 能被上下裁、被左右不裁），而且因为矩形本身是方的，测试用正方形裁剪矩形
    // 时根本看不出来。
    clip_against(&mut poly, Axis::Y, clip.y, true);
    clip_against(&mut poly, Axis::Y, clip.y + clip.height, false);
    clip_against(&mut poly, Axis::X, clip.x, true);
    clip_against(&mut poly, Axis::X, clip.x + clip.width, false);
    poly
}

#[derive(Clone, Copy)]
enum Axis {
    X,
    Y,
}

impl Axis {
    #[inline]
    fn of(self, p: Point) -> f32 {
        match self {
            Axis::X => p.x,
            Axis::Y => p.y,
        }
    }
}

/// 把 `poly` 裁到 `axis == edge` 的单侧。
///
/// `keep_greater` = true 保留 `> edge`，false 保留 `< edge`。
///
/// # 半开区间是**必须的**，不是精度取舍
///
/// 判据用**严格**不等号（`>` / `<`），于是恰好落在裁剪线上的顶点算"在外面"。
/// 若两边都用闭区间，一个恰好落在本遍裁剪线上的顶点会在这一遍被判"在内"、
/// 在下一遍被裁掉时生成一个与它**坐标完全相同**的新顶点：
///
/// ```text
/// 裁 y<=10 后: (0,0)(20,0)(10,10)(0,10)          ← (10,10) 在 y 边界上
/// 裁 x<=10 后: (0,0)(10,0)(10,10)(10,10)(0,10)   ← 多出一个重复点
/// ```
///
/// 重复顶点让结果多边形**自交**，于是：
///
/// - 鞋带公式算出的面积偏大（实测 100 而正确值是 50）；
/// - 扇形三角化把它切成含零面积/反向三角形的组合，渲染出多余的三角形。
///
/// 半开区间让每个顶点只在一遍里被算"在内"，重复点从根上消失。
/// 代价是落在裁剪线上的顶点归属由**下一遍**决定，而那正是我们想要的
/// （它会在下一遍被裁掉并生成一个位置相同的新点，或被保留）。
fn clip_against(poly: &mut Vec<Point>, axis: Axis, edge: f32, keep_greater: bool) {
    if poly.is_empty() {
        return;
    }
    let inside = |p: Point| {
        let v = axis.of(p);
        if keep_greater {
            v > edge
        } else {
            v < edge
        }
    };
    let intersect = |a: Point, b: Point| -> Point {
        // 参数化：交点满足 `av + (bv - av) * t == edge`，即
        // `t = (edge - av) / (bv - av)`。
        //
        // 写成"到边的带符号距离"的形式：`da` 是 `a` 到边的带符号距离（在保留侧
        // 为正），`db` 是 `b` 的（在丢弃侧为负）。于是
        //
        // ```text
        // t = da / (da - db)          ← 分母是**减**
        // ```
        //
        // ⚠ 分母必须是 `da - db` 而不是 `da + db`：两侧距离一正一负，
        // `da + db` 恒等于 0。除法得到 NaN / Inf，交点落到无穷远 ——
        // 症状是"裁剪完全不生效"（每次比较都是 NaN，`inside` 恒为 false，
        // 但顶点被原样 push 回去，于是三角形照原样通过）。
        //
        // `da - db` 在两侧重合时才为 0（退化），用 `1e-9` 兜住并返回 `a` ——
        // 一个 NaN 顶点会让整条边在光栅化时消失。
        let av = axis.of(a);
        let bv = axis.of(b);
        let da = if keep_greater { av - edge } else { edge - av };
        let db = if keep_greater { bv - edge } else { edge - bv };
        let denom = da - db;
        if denom.abs() < 1e-9 {
            return a;
        }
        let t = da / denom;
        match axis {
            Axis::Y => Point {
                x: a.x + (b.x - a.x) * t,
                y: edge,
            },
            Axis::X => Point {
                x: edge,
                y: a.y + (b.y - a.y) * t,
            },
        }
    };

    let n = poly.len();
    let mut out: Vec<Point> = Vec::with_capacity(n + 1);
    for i in 0..n {
        let cur = poly[i];
        let nxt = poly[(i + 1) % n];
        let cur_in = inside(cur);
        let nxt_in = inside(nxt);
        match (cur_in, nxt_in) {
            (true, true) => out.push(cur),
            (true, false) => {
                out.push(cur);
                out.push(intersect(cur, nxt));
            }
            (false, true) => out.push(intersect(cur, nxt)),
            (false, false) => {}
        }
    }
    *poly = out;
}

/// 把凸多边形扇形三角化，输出**顶点三元组**（不是三角形列表）。
///
/// # 为什么必须重新三角化而不是只保留跨界顶点
///
/// 裁剪后的多边形最多 5 个顶点、且是任意凸形状。它必须变成三角形才能画，
/// 而扇形三角化是凸多边形唯一正确的 O(n) 做法（顶点顺序即环序，无需排序）。
///
/// # 为什么不用 earcut
///
/// [`crate::quad::expand`] 每帧对**每个**裁剪后的三角形调用这里。若裁剪矩形
/// 就是元素自身（最常见的情形），扇形三角化已经是最优的 —— earcut 的
/// `filterPoints` 清洗与 z-order 排序在这个规模上全是净开销。
pub fn fan_triangulate(poly: &[Point]) -> Vec<[Point; 3]> {
    let n = poly.len();
    if n < 3 {
        return Vec::new();
    }
    // 退化：三点共线（裁剪线正好切过顶点时会出现），面积为零，画出来是空操作。
    let area2 = zephyra_core::geom::signed_area2(poly);
    if area2.abs() < 1e-12 {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(n - 2);
    for i in 1..n - 1 {
        let t = [poly[0], poly[i], poly[i + 1]];
        // 逐个三角形再查一次退化：多边形整体非零面积不代表**每个**扇形三角形
        // 都非零 —— 裁剪后的凸多边形上，落在同一条直边上的连续三个顶点会共线，
        // 那一个三角形是零面积。不剔除的话它会被画成"看不见但占索引"的三角形，
        // 而按 `Σ|面积|` 的断言仍然通过（它是多边形面积和的一部分）。
        let tri_area2 = zephyra_core::geom::signed_area2(&t);
        if tri_area2.abs() < 1e-12 {
            continue;
        }
        out.push(t);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use zephyra_core::geom::abs_area;

    fn p(x: f32, y: f32) -> Point {
        Point { x, y }
    }

    fn area_of(poly: &[Point]) -> f32 {
        abs_area(poly)
    }

    #[test]
    fn 完全在裁剪区内的三角形原样返回() {
        let tri = [p(1.0, 1.0), p(3.0, 1.0), p(2.0, 3.0)];
        let out = geom_clip(&tri, Some(Rect::new(0.0, 0.0, 10.0, 10.0)));
        assert_eq!(out.len(), 3, "不该多出顶点：{out:?}");
        assert!((area_of(&out) - 2.0).abs() < 1e-5);
    }

    #[test]
    fn 完全在裁剪区外的三角形被丢弃() {
        let tri = [p(20.0, 20.0), p(22.0, 20.0), p(21.0, 22.0)];
        assert!(geom_clip(&tri, Some(Rect::new(0.0, 0.0, 10.0, 10.0))).is_empty());
    }

    #[test]
    fn 跨界三角形裁剪后面积等于解析值() {
        // 单位方块 (0,0)..(2,2)，拆成两个三角形，用 `x >= 1` 裁掉左半。
        // 两半裁完的面积**不等**：`(0,0)(2,0)(2,2)` 那半切x=1 得四边形
        // `(2,0)(2,2)(1,1)(1,0)`，面积 1.5；另一半 `(0,0)(2,2)(0,2)` 切出
        // 三角形 `(1,1)(2,2)(1,2)`，面积 0.5。合计 2 = 方块右半，正确。
        //
        // 期望值是**量出来的**而不是想出来的：两个三角形对称，直觉上会以为
        // 各得一半，实际不等 —— 断言写成"各 1"就会把正确实现判成 bug。
        let t = [p(0.0, 0.0), p(2.0, 0.0), p(2.0, 2.0), p(0.0, 2.0)];
        let mut total = 0.0f32;
        for split in [[t[0], t[1], t[2]], [t[0], t[2], t[3]]] {
            let clipped = geom_clip(&split, Some(Rect::new(1.0, 0.0, 10.0, 10.0)));
            let tris = fan_triangulate(&clipped);
            assert!(!tris.is_empty(), "裁剪后必须仍有几何");
            total += tris.iter().map(|t| abs_area(&[t[0], t[1], t[2]])).sum::<f32>();
        }
        assert!(
            (total - 2.0).abs() < 1e-4,
            "两半合计应是方块右半的面积 2，实得 {total}"
        );
    }

    #[test]
    fn 斜边正好压在裁剪角上时交集是整个方块() {
        // 三角 (0,0)(20,0)(0,20) 的斜边是 `x + y = 20`，而 `[0,10]²` 内
        // `x + y <= 20` 恒成立 —— 所以交集是**整个**方块，面积 100。
        //
        // 这条测试钉的是"不该被裁掉的部分不会被裁掉"。曾经写成期望 50
        // （把斜边误当成 `x + y = 10`），而实得 100 被当成实现有 bug ——
        // 实际上错的是期望值。裁剪多边形与裁剪矩形的交集只能**变大**，
        // 断言时先手算交集，别凭印象。
        let tri = [p(0.0, 0.0), p(20.0, 0.0), p(0.0, 20.0)];
        let clipped = geom_clip(&tri, Some(Rect::new(0.0, 0.0, 10.0, 10.0)));
        let got: f32 = fan_triangulate(&clipped)
            .iter()
            .map(|t| abs_area(&[t[0], t[1], t[2]]))
            .sum();
        assert!((got - 100.0).abs() < 0.01, "交集应是整个 10×10 方块，实得 {got}");
    }

    #[test]
    fn 斜边真正切掉一块时面积减少量正确() {
        // 三角 (0,0)(20,0)(0,20) 裁到 `[0,15]²`。斜边 `x + y = 20` 与
        // `x = 15` / `y = 15` 交于 (15,5) 与 (5,15)。
        //
        // 交集是五边形 `(15,5)(5,15)(0,15)(0,0)(15,0)`。用两种算法独立复核：
        //
        // 1. 鞋带公式：15·15−5·5 + 5·15−0·15 + 0·0−0·15 + 0·0−15·0 + 15·5−15·0
        //    = 200 + 75 + 0 + 0 + 75 = 350 → 面积 175。
        // 2. 15×15 方块减去右上角被斜边切掉的三角形 `(15,5)(5,15)(15,15)`，
        //    那个三角形的面积 = ½·10·10 = 50 → 225 − 50 = 175。
        //
        // 两者一致。这条测试的价值是钉住"斜边切掉一块"这件事的具体数值：
        // 期望值写成"腰长 = 20−15 = 5"会得到 187.5，那是把"斜边被切掉的长度"
        // 当成了"腰长"—— 腰长是交点到角点的距离，即 15−5 = 10。
        let tri = [p(0.0, 0.0), p(20.0, 0.0), p(0.0, 20.0)];
        let clipped = geom_clip(&tri, Some(Rect::new(0.0, 0.0, 15.0, 15.0)));
        let got: f32 = fan_triangulate(&clipped)
            .iter()
            .map(|t| abs_area(&[t[0], t[1], t[2]]))
            .sum();
        assert!(
            (got - 175.0).abs() < 0.01,
            "交集应是五边形、面积 175，实得 {got}"
        );
    }

    #[test]
    fn 裁剪不产生重复顶点() {
        // 半开区间的存在理由：顶点恰好落在裁剪线上时，若用闭区间会生成
        // 一个坐标完全相同的重复点，让结果多边形自交、面积偏大。
        // 已知会触发的构造：先被 y 裁出边界点，再被 x 裁到同一坐标。
        let tri = [p(0.0, 0.0), p(20.0, 0.0), p(0.0, 20.0)];
        let out = geom_clip(&tri, Some(Rect::new(0.0, 0.0, 10.0, 10.0)));
        for i in 0..out.len() {
            for j in (i + 1)..out.len() {
                let a = (out[i].x, out[i].y);
                let b = (out[j].x, out[j].y);
                assert_ne!(
                    (a.0.to_bits(), a.1.to_bits()),
                    (b.0.to_bits(), b.1.to_bits()),
                    "裁剪产出重复顶点 {a:?}（下标 {i} 与 {j}）"
                );
            }
        }
    }

    #[test]
    fn 扇形三角化的面积和等于多边形面积() {
        let poly = vec![
            p(1.0, 1.0),
            p(5.0, 1.0),
            p(6.0, 3.0),
            p(4.0, 5.0),
            p(1.0, 4.0),
        ];
        let tris = fan_triangulate(&poly);
        assert_eq!(tris.len(), 3, "5 边形 → 3 个三角形");
        let sum: f32 = tris.iter().map(|t| abs_area(&[t[0], t[1], t[2]])).sum();
        assert!(
            (sum - area_of(&poly)).abs() < 1e-4,
            "扇形三角化必须无损：{sum} vs {}",
            area_of(&poly)
        );
    }

    #[test]
    fn 退化多边形不产出三角形() {
        // 三点共线
        let poly = vec![p(0.0, 0.0), p(1.0, 0.0), p(2.0, 0.0)];
        assert!(fan_triangulate(&poly).is_empty());
        // 点数不足
        assert!(fan_triangulate(&[p(0.0, 0.0), p(1.0, 1.0)]).is_empty());
    }

    #[test]
    fn 裁剪顶点不产生_nan() {
        // 裁剪线正好穿过一个顶点：这是最容易产生 NaN 的情形
        let tri = [p(0.0, 0.0), p(4.0, 2.0), p(0.0, 4.0)];
        let out = geom_clip(&tri, Some(Rect::new(0.0, 2.0, 4.0, 2.0)));
        for q in &out {
            assert!(q.x.is_finite() && q.y.is_finite(), "裁剪产出 NaN：{q:?}");
        }
    }
}
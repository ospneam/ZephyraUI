//! 路径几何的断言。
//!
//! # 这里断言什么、不断言什么
//!
//! **断言**：可量化、不依赖渲染器的性质 —— 弦高误差上界、面积守恒、绕向判定、
//! 描边宽度、自交检出。
//!
//! **不断言**："看起来像图标"。任何"与参考图逐像素比较"的断言都会在换渲染器、
//! 换抗锯齿策略时红，而它红的原因往往不是 bug。所以形状正确性由**面积守恒**
//! 这类不变量兜底，肉眼验证放在 `demo --dump-cpu`。
//!
//! 面积守恒是最强的一条：它同时覆盖了"孔被填实""孔被漏挖""耳切切错"
//! ——这三种错误的症状完全不同，但都会让面积对不上。

use crate::core::{Point, Rect};
use crate::geom::{
    abs_area, area_of, cross, crossings, flatten, inside, is_self_intersecting, point_in_triangle,
    stroke_outline, tessellate, transform_segments, triangulate, winding_number, Contour, LineCap,
    LineJoin, StrokeStyle, TriMesh, TriMeshError, POINT_EPS, TOLERANCE,
};
use crate::icon::{parse_path_data, FillRule, Segment};

fn p(x: f32, y: f32) -> Point {
    Point { x, y }
}

fn contour(pts: &[(f32, f32)]) -> Contour {
    Contour::new(pts.iter().map(|(x, y)| p(*x, *y)).collect())
}

/// 三角形面积之和。
///
/// # 只在**无重叠**时可用
///
/// 耳切法产出的是**无重叠满覆盖**剖分，所以对正常输入它等于真实填充面积。
/// 但桥接孔洞后环是自交的，某些耳可能跨过区域 —— 此时本值会偏大。
/// 要判定带孔形状用 [`coverage`]。
fn mesh_area(mesh: &TriMesh) -> f32 {
    mesh.vertices
        .chunks_exact(3)
        .map(|t| {
            let (a, b, c) = (
                Point {
                    x: t[0].position.x,
                    y: t[0].position.y,
                },
                Point {
                    x: t[1].position.x,
                    y: t[1].position.y,
                },
                Point {
                    x: t[2].position.x,
                    y: t[2].position.y,
                },
            );
            cross(a, b, c).abs() * 0.5
        })
        .sum()
}

/// 网格采样求三角形集合的实际覆盖面积。
///
/// # 为什么需要它
///
/// 带孔多边形经桥接后，耳切可能切出**跨过孔**的三角形，于是
/// `Σ|三角形面积| ≠ 实际填充面积` —— 而"洞有没有被挖对"恰恰是这类代码最容易
/// 错的地方。覆盖率直接测"这片区域里任一点的采样是否落在某个三角形内"，
/// 它对重叠免疫，且与 GPU 实际光栅化的结果一致。
///
/// 分辨率取 `n×n` 正方形网格打进 `bounds`，误差约 `边长²/n`，用 `tol` 吸收。
fn coverage(mesh: &TriMesh, bounds: Rect, n: usize) -> f32 {
    let tris: Vec<(Point, Point, Point)> = mesh
        .vertices
        .chunks_exact(3)
        .map(|t| {
            (
                Point {
                    x: t[0].position.x,
                    y: t[0].position.y,
                },
                Point {
                    x: t[1].position.x,
                    y: t[1].position.y,
                },
                Point {
                    x: t[2].position.x,
                    y: t[2].position.y,
                },
            )
        })
        .collect();
    if tris.is_empty() {
        return 0.0;
    }
    let cell_w = bounds.width / n as f32;
    let cell_h = bounds.height / n as f32;
    let mut hits = 0usize;
    for iy in 0..n {
        for ix in 0..n {
            let p = Point {
                x: bounds.x + (ix as f32 + 0.5) * cell_w,
                y: bounds.y + (iy as f32 + 0.5) * cell_h,
            };
            if tris
                .iter()
                .any(|(a, b, c)| point_in_triangle(p, *a, *b, *c))
            {
                hits += 1;
            }
        }
    }
    hits as f32 * cell_w * cell_h
}

// ----------------------------------------------------------------展平

#[test]
fn straight_segment_yields_no_contour_because_no_area() {
    // SVG 规定子路径在填充时视为闭合，所以 M→L 是一个零面积区域。
    // 丢弃它是正确的：耳切法在零面积轮廓上会死循环。
    let segs = vec![Segment::MoveTo(0.0, 0.0), Segment::LineTo(10.0, 0.0)];
    assert!(
        flatten(&segs, TOLERANCE).is_empty(),
        "零面积轮廓必须被丢弃，否则耳切会死循环"
    );
}

#[test]
fn collinear_control_points_do_not_split() {
    // 控制点与端点共线：偏差恒为 0，不该被细分。
    // 端点加一个垂直偏移让它有面积，才能观察展平结果。
    let segs = vec![
        Segment::MoveTo(0.0, 0.0),
        Segment::CubicTo(10.0, 0.0, 20.0, 0.0, 30.0, 0.0),
        Segment::LineTo(30.0, 10.0),
        Segment::LineTo(0.0, 10.0),
    ];
    let out = flatten(&segs, TOLERANCE);
    assert_eq!(out.len(), 1);
    assert_eq!(
        out[0].points().len(),
        4,
        "共线控制点不该被细分（应恰好 4 个顶点）"
    );
}

#[test]
fn semicircle_sagitta_error_within_tolerance() {
    // 用四次贝塞尔逼近 180° 圆弧：弦高误差必须 ≤ TOLERANCE。
    let k = 0.5522847498307933_f32;
    let segs = vec![
        Segment::MoveTo(0.0, 10.0),
        Segment::CubicTo(0.0, 10.0 - 10.0 * k, 10.0 * k, 0.0, 10.0, 0.0),
        Segment::CubicTo(10.0 + 10.0 * k, 0.0, 10.0, 10.0 - 10.0 * k, 10.0, 10.0),
    ];
    let out = flatten(&segs, TOLERANCE);
    let poly = out[0].points();

    // 对每一段弦，采样真曲线上该区间的参数点，量它到弦的最大距离。
    let mut worst: f32 = 0.0;
    for i in 0..poly.len() - 1 {
        let a = poly[i];
        let b = poly[i + 1];
        // 弦长的一半作为粗略的弧长估计，采样 8 点
        for k in 0..=8 {
            let t = k as f32 / 8.0;
            // 二次贝塞尔（圆弧段可用二次近似足够）
            let px =
                (1.0 - t).powi(2) * a.x + 2.0 * (1.0 - t) * t * (a.x + b.x) * 0.5 + t * t * b.x;
            let py =
                (1.0 - t).powi(2) * a.y + 2.0 * (1.0 - t) * t * (a.y + b.y) * 0.5 + t * t * b.y;
            let dx = b.x - a.x;
            let dy = b.y - a.y;
            let len = (dx * dx + dy * dy).sqrt();
            if len <= POINT_EPS {
                continue;
            }
            let d = ((px - a.x) * dy - (py - a.y) * dx).abs() / len;
            worst = worst.max(d);
        }
    }
    assert!(
        worst <= TOLERANCE * 1.5,
        "弦高误差 {worst} 超过容差 {TOLERANCE}（1.5 倍为采样余量）"
    );
}

#[test]
fn sharp_curve_is_tessellated_denser_than_shallow_one() {
    // 同样的控制点数量，弯得急的那段必须产生更多顶点。
    let gentle = flatten(
        &[
            Segment::MoveTo(0.0, 0.0),
            Segment::CubicTo(3.0, 1.0, 7.0, 1.0, 10.0, 0.0),
        ],
        TOLERANCE,
    );
    let sharp = flatten(
        &[
            Segment::MoveTo(0.0, 0.0),
            Segment::CubicTo(3.0, 6.0, 7.0, 6.0, 10.0, 0.0),
        ],
        TOLERANCE,
    );
    assert!(
        sharp[0].points().len() > gentle[0].points().len(),
        "急弯 {} 点应多于缓弯 {} 点",
        sharp[0].points().len(),
        gentle[0].points().len()
    );
}

#[test]
fn closed_subpath_last_point_is_not_repeated() {
    let segs = parse_path_data("M0 0L10 0L10 10Z").unwrap();
    let out = flatten(&segs, TOLERANCE);
    assert_eq!(out.len(), 1);
    // 首尾不重复（闭合由Contour 的隐式首尾相连表达）
    let pts = out[0].points();
    assert_ne!(pts[0], pts[pts.len() - 1]);
}

#[test]
fn degenerate_line_subpath_is_dropped() {
    // 三点共线：面积为零，不可填充
    let segs = vec![
        Segment::MoveTo(0.0, 0.0),
        Segment::LineTo(5.0, 0.0),
        Segment::LineTo(10.0, 0.0),
    ];
    assert!(flatten(&segs, TOLERANCE).is_empty());
}

// ----------------------------------------------------------------绕向与填充规则

#[test]
fn positive_area_means_clockwise_in_y_down_space() {
    // (0,0)→(1,0)→(1,1)→(0,1)：y 向下时这是**顺时针**
    let c = contour(&[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]);
    assert!(c.signed_area2() > 0.0);
}

#[test]
fn simple_polygon_triangulation_preserves_area() {
    let c = contour(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]);
    let mesh = triangulate(std::slice::from_ref(&c), FillRule::NonZero).unwrap();
    assert_eq!(mesh.triangles, 2, "凸四边形应切成 2 个三角形");
    let cov = coverage(&mesh, Rect::new(0.0, 0.0, 10.0, 10.0), 150);
    assert!((cov - 100.0).abs() < 0.6, "方形覆盖率 {cov} 应约为 100");
}

#[test]
fn concave_polygon_triangulation_preserves_area() {
    // L 形
    let c = contour(&[
        (0.0, 0.0),
        (10.0, 0.0),
        (10.0, 4.0),
        (4.0, 4.0),
        (4.0, 10.0),
        (0.0, 10.0),
    ]);
    let mesh = triangulate(std::slice::from_ref(&c), FillRule::NonZero).unwrap();
    // L 形面积 = 10×4 + 4×6 = 64（shoelace 独立核算过）
    let cov = coverage(&mesh, Rect::new(0.0, 0.0, 10.0, 10.0), 150);
    assert!((cov - 64.0).abs() < 0.6, "L 形覆盖率 {cov} 应约为 64");
}

#[test]
fn polygon_with_hole_area_equals_outer_minus_inner() {
    let outer = contour(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]);
    // 内层必须与外层**反向**才挖得空：NonZero 下同向嵌套是环绕数 1+1=2，仍然实心
    // （那条规则由`same_direction_nesting_is_solid_under_nonzero_rule`单独盯住）。
    let inner = contour(&[(3.0, 7.0), (7.0, 7.0), (7.0, 3.0), (3.0, 3.0)]);
    let contours = vec![outer, inner];
    let mesh = triangulate(&contours, FillRule::NonZero).unwrap();
    // 100 − 16 = 84。用**覆盖率**而不是 Σ|三角形面积|——见 coverage 的文档。
    let cov = coverage(&mesh, Rect::new(0.0, 0.0, 10.0, 10.0), 120);
    assert!(
        (cov - 84.0).abs() < 0.6,
        "带孔覆盖率 {cov} 应约为 84（Σ|面积| = {}）",
        mesh_area(&mesh)
    );
}

#[test]
fn same_direction_nesting_is_solid_under_nonzero_rule() {
    // 两层同向轮廓：非零规则 → 内部仍然填充（环绕数 2）
    let outer = contour(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]);
    let inner = contour(&[(3.0, 3.0), (7.0, 3.0), (7.0, 7.0), (3.0, 7.0)]);
    let contours = vec![outer, inner];
    // inside() 在内层中心为真
    assert!(inside(p(5.0, 5.0), &contours, FillRule::NonZero));
    let mesh = triangulate(&contours, FillRule::NonZero).unwrap();
    let cov = coverage(&mesh, Rect::new(0.0, 0.0, 10.0, 10.0), 120);
    assert!(
        (cov - 100.0).abs() < 0.6,
        "同向嵌套在非零规则下应填满 100，实得 {cov}"
    );
}

#[test]
fn reversed_nesting_cuts_hole_under_nonzero_rule() {
    let outer = contour(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]);
    let inner = contour(&[(3.0, 7.0), (7.0, 7.0), (7.0, 3.0), (3.0, 3.0)]);
    let contours = vec![outer, inner];
    assert!(!inside(p(5.0, 5.0), &contours, FillRule::NonZero));
    let mesh = triangulate(&contours, FillRule::NonZero).unwrap();
    let cov = coverage(&mesh, Rect::new(0.0, 0.0, 10.0, 10.0), 120);
    assert!((cov - 84.0).abs() < 0.6, "反向嵌套应挖空到 84，实得 {cov}");
}

#[test]
fn even_odd_rule_ignores_winding_and_alternates() {
    let outer = contour(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]);
    let inner = contour(&[(3.0, 3.0), (7.0, 3.0), (7.0, 7.0), (3.0, 7.0)]);
    let contours = vec![outer, inner];
    // 奇偶规则下同向嵌套也要挖空
    assert!(!inside(p(5.0, 5.0), &contours, FillRule::EvenOdd));
    let mesh = triangulate(&contours, FillRule::EvenOdd).unwrap();
    let cov = coverage(&mesh, Rect::new(0.0, 0.0, 10.0, 10.0), 120);
    assert!(
        (cov - 84.0).abs() < 0.6,
        "奇偶规则下应挖空到 84，实得 {cov}"
    );
}

#[test]
fn point_on_boundary_has_stable_ownership() {
    // 顶点处的穿越计数必须用半开区间，否则两条共享端点的边会被双计或漏计。
    let outer = contour(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]);
    let cs = vec![outer];
    // 内部点：穿越 1 次
    assert_eq!(crossings(p(5.0, 5.0), &cs), 1);
    assert_eq!(winding_number(p(5.0, 5.0), &cs), 1);

    // 顶点 (0,0)：半开区间保证共享该端点的两条边里**恰好一条**被计入
    // （`(prev.y > py) != (cur.y > py)` 对两条边给出相反结论）。
    //
    // 所以这里断言的是"1"而不是"0"或"2"：
    // - 若两边都计（`>=`）→ 2；
    // - 若两边都不计（`>`）→ 0；
    // - 半开区间 → 1。
    //
    // 之前误以为"向上向下的两条边会相消为 0"，那是把半开区间与闭合区间搞混了：
    // 相消需要两条边**都**被计入，而半开区间的意义正是只计一条。
    assert_eq!(
        crossings(p(0.0, 0.0), &cs),
        1,
        "顶点处恰好一条边被计入（半开区间）"
    );
    // 另一个顶点 (10,0)：此处水平向右的那条边 y 恒等于 0，两条边都不跨越，
    // 于是 0 —— 说明计数确实随边的朝向变化，而不是某个固定常数。
    assert_eq!(crossings(p(10.0, 0.0), &cs), 0);

    // 外部点：无穿越
    assert_eq!(crossings(p(15.0, 5.0), &cs), 0);
    assert_eq!(winding_number(p(15.0, 5.0), &cs), 0);
}

#[test]
fn all_holes_are_cut_out() {
    let outer = contour(&[(0.0, 0.0), (30.0, 0.0), (30.0, 10.0), (0.0, 10.0)]);
    // 两个孔都与外框反向 —— NonZero 下同向的孔是实心，挖不出来。
    let h1 = contour(&[(2.0, 6.0), (6.0, 6.0), (6.0, 2.0), (2.0, 2.0)]);
    let h2 = contour(&[(12.0, 6.0), (16.0, 6.0), (16.0, 2.0), (12.0, 2.0)]);
    let contours = vec![outer, h1, h2];
    // 300 − 16 − 16 = 268
    let mesh = triangulate(&contours, FillRule::NonZero).unwrap();
    let cov = coverage(&mesh, Rect::new(0.0, 0.0, 30.0, 10.0), 300);
    assert!((cov - 268.0).abs() < 1.2, "两孔覆盖率 {cov} 应约为 268");
}

// ----------------------------------------------------------------自交

#[test]
fn detects_self_intersection() {
    // 经典交叉
    let c = contour(&[(0.0, 0.0), (10.0, 10.0), (10.0, 0.0), (0.0, 10.0)]);
    assert!(is_self_intersecting(c.points()));
}

#[test]
fn simple_polygon_is_not_mistaken_for_self_intersecting() {
    let c = contour(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]);
    assert!(!is_self_intersecting(c.points()));
}

#[test]
fn shared_endpoint_between_adjacent_edges_is_not_self_intersection() {
    // L 形：相邻边在顶点处接触，合法
    let c = contour(&[
        (0.0, 0.0),
        (10.0, 0.0),
        (10.0, 4.0),
        (4.0, 4.0),
        (4.0, 10.0),
        (0.0, 10.0),
    ]);
    assert!(!is_self_intersecting(c.points()));
}

#[test]
fn self_intersecting_contour_is_skipped_not_shredded() {
    let bad = contour(&[(0.0, 0.0), (10.0, 10.0), (10.0, 0.0), (0.0, 10.0)]);
    let r = triangulate(std::slice::from_ref(&bad), FillRule::NonZero);
    assert!(r.is_err(), "自交轮廓必须报错而不是硬切");
}

// ----------------------------------------------------------------描边

#[test]
fn straight_stroke_width_is_correct() {
    // 水平线段，宽 4 → 上下各 2
    let pts = vec![p(0.0, 10.0), p(20.0, 10.0)];
    let style = StrokeStyle {
        width: 4.0,
        ..Default::default()
    };
    let rings = stroke_outline(&pts, false, &style);
    assert_eq!(rings.len(), 1, "开放直线段应产出单一跑道形");
    let ys: Vec<f32> = rings[0].points().iter().map(|p| p.y).collect();
    let min_y = ys.iter().cloned().fold(f32::INFINITY, |a, b| a.min(b));
    let max_y = ys.iter().cloned().fold(f32::NEG_INFINITY, |a, b| a.max(b));
    assert!((max_y - min_y - 4.0).abs() < 1e-3, "描边带高度应恰为 4");
}

#[test]
fn closed_stroke_yields_outer_and_inner() {
    //正方形中心线，宽 2
    let pts = vec![p(0.0, 0.0), p(10.0, 0.0), p(10.0, 10.0), p(0.0, 10.0)];
    let style = StrokeStyle {
        width: 2.0,
        ..Default::default()
    };
    let rings = stroke_outline(&pts, true, &style);
    assert_eq!(rings.len(), 2, "闭合路径应产出外轮廓 + 内孔");
    let outer_area = abs_area(rings[0].points());
    let inner_area = abs_area(rings[1].points());
    //外圈 12×12=144，内圈 8×8=64
    assert!(
        (outer_area - 144.0).abs() < 1e-2,
        "外轮廓应约 144，实得 {outer_area}"
    );
    assert!(
        (inner_area - 64.0).abs() < 1e-2,
        "内孔应约 64，实得 {inner_area}"
    );
    // 绕向必须相反，否则非零规则会把它填实
    assert!(
        rings[0].signed_area2() * rings[1].signed_area2() < 0.0,
        "内孔绕向必须与外轮廓相反"
    );
}

#[test]
fn closed_stroke_triangulated_area_equals_band() {
    let pts = vec![p(0.0, 0.0), p(10.0, 0.0), p(10.0, 10.0), p(0.0, 10.0)];
    let style = StrokeStyle {
        width: 2.0,
        ..Default::default()
    };
    let rings = stroke_outline(&pts, true, &style);
    let mesh = triangulate(&rings, FillRule::NonZero).unwrap();
    // 环带 = 144 − 64 = 80
    //
    // 用**解析面积**而不是网格采样：这是外轮廓（12×12）套内孔（8×8）的干净环带，
    // 耳切产出无重叠满覆盖剖分，`Σ|三角形面积|` 就是精确的 80。
    //
    // 此前这里用 200×200 采样覆盖率，测得 81.28 而容差只有 0.6。采样覆盖率在
    // 网格加密时并不单调收敛（实测 n=100/200/400/800/1600 分别得
    // 79.03/ 81.28 / 79.60 / 79.88 / 80.16），它量化的是**采样栅格与边界的对齐**，
    // 不是几何误差 —— 对轴对齐的矩形环带，栅格化误差就是 O(周长/n) 量级的抖动，
    // 不会趋零。几何是否正确应当用解析量断言。
    assert!(
        (mesh_area(&mesh) - 80.0).abs() < 0.01,
        "环带解析面积 {} 应为 80",
        mesh_area(&mesh)
    );
    // 采样覆盖率只作为"形状大致对"的粗筛，容差按周长/n 的量级给。
    let cov = coverage(&mesh, Rect::new(-2.0, -2.0, 14.0, 14.0), 200);
    assert!(
        (cov - 80.0).abs() < 2.0,
        "描边环带覆盖率 {cov} 应在 80 附近"
    );
}

#[test]
fn zero_width_produces_no_geometry() {
    let pts = vec![p(0.0, 0.0), p(10.0, 0.0)];
    let style = StrokeStyle {
        width: 0.0,
        ..Default::default()
    };
    assert!(stroke_outline(&pts, false, &style).is_empty());
}

#[test]
fn square_cap_extends_outward() {
    let pts = vec![p(0.0, 0.0), p(10.0, 0.0)];
    let style = StrokeStyle {
        width: 4.0,
        cap: LineCap::Square,
        ..Default::default()
    };
    let rings = stroke_outline(&pts, false, &style);
    let xs: Vec<f32> = rings[0].points().iter().map(|p| p.x).collect();
    let min_x = xs.iter().cloned().fold(f32::INFINITY, |a, b| a.min(b));
    // 方形端帽把两端各外延半个宽度（2）
    assert!(min_x <= -1.9, "方形端帽应向左外延到约 −2，实得 {min_x}");
}

#[test]
fn bevel_cap_does_not_extend() {
    let pts = vec![p(0.0, 0.0), p(10.0, 0.0)];
    let style = StrokeStyle {
        width: 4.0,
        cap: LineCap::Butt,
        ..Default::default()
    };
    let rings = stroke_outline(&pts, false, &style);
    let xs: Vec<f32> = rings[0].points().iter().map(|p| p.x).collect();
    let min_x = xs.iter().cloned().fold(f32::INFINITY, |a, b| a.min(b));
    assert!(min_x >= -1e-3, "平截端帽不应外延，实得 {min_x}");
}

#[test]
fn clamped_miter_falls_back_to_bevel() {
    // 极小的转角 → miter 外伸极远，必须被 limit 拦下
    let pts = vec![p(0.0, 0.0), p(10.0, 0.0), p(10.001, 5.0)];
    let style = StrokeStyle {
        width: 4.0,
        join: LineJoin::Miter,
        miter_limit: 2.0,
        ..Default::default()
    };
    let rings = stroke_outline(&pts, false, &style);
    // 不panic 且产出合法轮廓即可；limit 生效意味着外伸不会趋于无穷
    assert!(!rings.is_empty());
    for r in &rings {
        for q in r.points() {
            assert!(
                q.x.is_finite() && q.y.is_finite(),
                "miter 不得产生非有限坐标"
            );
        }
    }
}

#[test]
fn round_join_outset_limited_to_half_stroke_width() {
    // 直角转折 + miter_limit=1（强制 bevel 等价）不应外伸
    let pts = vec![p(0.0, 0.0), p(10.0, 0.0), p(10.0, 10.0)];
    let style = StrokeStyle {
        width: 4.0,
        join: LineJoin::Miter,
        miter_limit: 1.0,
        ..Default::default()
    };
    let rings = stroke_outline(&pts, false, &style);
    let max_x = rings[0]
        .points()
        .iter()
        .map(|q| q.x)
        .fold(f32::NEG_INFINITY, |a, b| a.max(b));
    // 外侧不应超过 10 + 2（半宽）
    assert!(
        max_x <= 10.0 + 2.0 + 1e-2,
        "miter_limit=1 时外伸应被压到半宽，实得 {max_x}"
    );
}

// ----------------------------------------------------------------变换

#[test]
fn viewbox等比缩放并居中() {
    // viewBox 24×24 映射到 48×48：线性放大 2 倍
    let segs = vec![
        Segment::MoveTo(0.0, 0.0),
        Segment::LineTo(24.0, 0.0),
        Segment::LineTo(24.0, 24.0),
    ];
    let dst = Rect::new(0.0, 0.0, 48.0, 48.0);
    let t = transform_segments(&segs, (0.0, 0.0, 24.0, 24.0), dst);
    match t[2] {
        Segment::LineTo(x, y) => {
            assert!((x - 48.0).abs() < 1e-3 && (y - 48.0).abs() < 1e-3);
        }
        _ => panic!("期望LineTo"),
    }
}

#[test]
fn viewbox偏移被正确扣除() {
    // viewBox 从 (10,10) 起：内容应落在 dst 内的对应位置
    let segs = vec![Segment::MoveTo(10.0, 10.0), Segment::LineTo(20.0, 10.0)];
    let dst = Rect::new(0.0, 0.0, 10.0, 10.0);
    let t = transform_segments(&segs, (10.0, 10.0, 10.0, 10.0), dst);
    match t[0] {
        Segment::MoveTo(x, y) => assert!(
            x.abs() < 1e-3 && y.abs() < 1e-3,
            "viewBox 原点应映射到 dst 原点"
        ),
        _ => panic!("期望 MoveTo"),
    }
}

#[test]
fn degenerate_viewbox_produces_no_geometry() {
    let segs = vec![Segment::MoveTo(0.0, 0.0), Segment::LineTo(1.0, 1.0)];
    let t = transform_segments(&segs, (0.0, 0.0, 0.0, 0.0), Rect::new(0.0, 0.0, 10.0, 10.0));
    assert!(t.is_empty());
}

#[test]
fn narrow_target_rect_scales_uniformly_and_centres() {
    // dst 宽大于高 → 高度方向留白，宽度贴满
    let segs = vec![Segment::MoveTo(0.0, 0.0), Segment::LineTo(0.0, 24.0)];
    let dst = Rect::new(0.0, 0.0, 100.0, 50.0);
    let t = transform_segments(&segs, (0.0, 0.0, 24.0, 24.0), dst);
    // 纵向（对应 y）映射到 0..50，横向留白
    if let Segment::LineTo(x, y) = t[1] {
        assert!(y <= 50.0 + 1e-3, "纵向应不超过目标高度，实得 {y}");
        assert!(x > 0.0 && x < 100.0, "横向应居中留白，实得 {x}");
    }
}

// ----------------------------------------------------------------端到端

#[test]
fn real_icon_triangulation_area_is_sane() {
    // 齿轮图标：4 条轮廓嵌套 —— 外齿圈（顺）+ 内圈（逆，挖空齿圈中心）
    // + 中心轴（顺，实心）+ 轴心孔（逆，挖空）。
    // 正确填充面积 = (252.52 − 154.63) + (48.94 − 19.11) = 127.72。
    let d = include_str!("icon/assets/icons/gear.svg");
    let svg = d.strip_prefix("<svg").map(|_| d).unwrap();
    let icon = crate::icon::parse_svg(svg).unwrap();
    let dst = Rect::new(0.0, 0.0, 24.0, 24.0);
    let segs = transform_segments(&icon.segments, icon.view_box, dst);
    let contours = flatten(&segs, TOLERANCE);
    assert!(contours.len() >= 4, "齿轮应有外圈与内圈");
    let mesh = triangulate(&contours, icon.paint.rule).unwrap();

    // 判据用**解析面积**而不是一个拍脑袋的区间。
    //
    // `contour_area_with_rule` 走 winding number / 穿越计数，与三角化器是完全独立的
    // 两条实现路径 —— 它俩一致才说明三角化真的盖对了形状。区间断言做不到这件事：
    // 它只能证明"面积落在某个范围内"，而重复三角形、漏切一个角这类缺陷恰好都能
    // 落进这个范围（历史上这条断言的30%~80% 就是照着一份产出重复三角形的实现
    // 标定的，于是把缺陷一起固化进了测试）。
    let expected = area_of(&contours, icon.paint.rule).abs();
    let actual = mesh_area(&mesh);
    assert!(
        (actual - expected).abs() < 0.05,
        "齿轮填充面积应精确等于解析面积 {expected}，实得 {actual}"
    );

    // 附带确认它确实是个"稀疏齿形"而不是实心方块 —— 防止哪天解析面积本身出错时
    // 上面的相等断言因为两边一起错而仍然通过。
    assert!(actual < 576.0 * 0.4, "齿轮填充率 {actual}/576 不应是实心的");
}

#[test]
fn all_builtin_icons_are_non_empty() {
    // 为什么单独一条：`all_builtin_icons_can_be_triangulated` 在 `is_empty()` 时是
    // `continue` —— 一个**解析成空图标**的内置资源（根节点 `fill="none"`
    // 之类的），在那里会安静地"通过"。而它的症状在界面上是「某个图标不显示」：
    // 没有报错、没有告警、图还在、就是一片空白。
    //
    // 一张内置表里的图标**不许有空的**：要么能用，要么根本不该登记 ——
    // 不存在"先登记着以后再说"。
    for (name, src) in crate::icon::assets::REGISTRY {
        let text = match src {
            crate::icon::IconSource::Svg(s) => s,
            crate::icon::IconSource::Path(_) => continue,
        };
        let icon =
            crate::icon::parse_svg(text).unwrap_or_else(|e| panic!("图标 {name} 解析失败: {e}"));
        assert!(
            !icon.is_empty(),
            "内置图标 {name} 解析成了空图标（无路径段或填充为 none）"
        );
    }
}

#[test]
fn all_builtin_icons_can_be_triangulated() {
    // 回归网：任何一个内置图标三角化失败或产出零面积，都说明几何管线坏了
    for (name, src) in crate::icon::assets::REGISTRY {
        let text = match src {
            crate::icon::IconSource::Svg(s) => s,
            crate::icon::IconSource::Path(_) => continue,
        };
        let icon = match crate::icon::parse_svg(text) {
            Ok(i) => i,
            Err(e) => panic!("图标 {name} 解析失败: {e}"),
        };
        if icon.is_empty() {
            continue;
        }
        let dst = Rect::new(0.0, 0.0, 24.0, 24.0);
        let segs = transform_segments(&icon.segments, icon.view_box, dst);
        let contours = flatten(&segs, TOLERANCE);
        let mesh = match triangulate(&contours, icon.paint.rule) {
            Ok(m) => m,
            Err(TriMeshError::Partial { mesh }) => mesh,
            Err(e) => panic!("图标 {name} 三角化失败: {e}"),
        };
        assert!(!mesh.is_empty(), "图标 {name} 产出零三角形");
        assert!(
            mesh_area(&mesh) > 1.0,
            "图标 {name} 面积过小({})，疑似几何退化",
            mesh_area(&mesh)
        );
        assert!(
            mesh.vertices.len() % 3 == 0,
            "图标 {name} 顶点数必须是 3 的倍数"
        );
    }
}

#[test]
fn end_to_end_fill() {
    // 一个 24×24 的实心方块在 48×48 目标里 → 面积 2304
    let segs = parse_path_data("M0 0L24 0L24 24L0 24Z").unwrap();
    let mesh = tessellate(
        &segs,
        (0.0, 0.0, 24.0, 24.0),
        Rect::new(0.0, 0.0, 48.0, 48.0),
        FillRule::NonZero,
    )
    .unwrap();
    let cov = coverage(&mesh, Rect::new(0.0, 0.0, 48.0, 48.0), 150);
    assert!((cov - 2304.0).abs() < 8.0, "覆盖率 {cov} 应约为 2304");
}

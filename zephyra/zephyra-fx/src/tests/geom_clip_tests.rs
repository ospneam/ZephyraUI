// 本文件由 geom_clip.rs 的 #[cfg(test)] mod tests 抽出 —— 测试代码与生产代码分离。
// 模块路径不变（src 里的声明用 #[path] 指过来），所以 use super::* 与私有可见性都照旧。

    use super::*;
    use zephyra_core::geom::abs_area;

    fn p(x: f32, y: f32) -> Point {
        Point { x, y }
    }

    fn area_of(poly: &[Point]) -> f32 {
        abs_area(poly)
    }

    #[test]
    fn triangle_fully_inside_clip_returned_unchanged() {
        let tri = [p(1.0, 1.0), p(3.0, 1.0), p(2.0, 3.0)];
        let out = geom_clip(&tri, Some(Rect::new(0.0, 0.0, 10.0, 10.0)));
        assert_eq!(out.len(), 3, "不该多出顶点：{out:?}");
        assert!((area_of(&out) - 2.0).abs() < 1e-5);
    }

    #[test]
    fn triangle_fully_outside_clip_is_dropped() {
        let tri = [p(20.0, 20.0), p(22.0, 20.0), p(21.0, 22.0)];
        assert!(geom_clip(&tri, Some(Rect::new(0.0, 0.0, 10.0, 10.0))).is_empty());
    }

    #[test]
    fn clipped_straddling_triangle_area_equals_analytic_value() {
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
    fn diagonal_exactly_on_clip_corner_intersects_whole_box() {
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
    fn area_decrease_is_correct_when_diagonal_actually_cuts() {
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
    fn clipping_produces_no_duplicate_vertices() {
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
    fn sector_triangulation_area_sums_to_polygon_area() {
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
    fn degenerate_polygon_yields_no_triangles() {
        // 三点共线
        let poly = vec![p(0.0, 0.0), p(1.0, 0.0), p(2.0, 0.0)];
        assert!(fan_triangulate(&poly).is_empty());
        // 点数不足
        assert!(fan_triangulate(&[p(0.0, 0.0), p(1.0, 1.0)]).is_empty());
    }

    #[test]
    fn clipped_vertices_have_no_nan() {
        // 裁剪线正好穿过一个顶点：这是最容易产生 NaN 的情形
        let tri = [p(0.0, 0.0), p(4.0, 2.0), p(0.0, 4.0)];
        let out = geom_clip(&tri, Some(Rect::new(0.0, 2.0, 4.0, 2.0)));
        for q in &out {
            assert!(q.x.is_finite() && q.y.is_finite(), "裁剪产出 NaN：{q:?}");
        }
    }

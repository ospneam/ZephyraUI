// 本文件由 cpu_raster.rs 的 #[cfg(test)] mod tests 抽出 —— 测试代码与生产代码分离。
// 模块路径不变（src 里的声明用 #[path] 指过来），所以 use super::* 与私有可见性都照旧。

    use super::*;

    /// 一个 `Command::Path` 批次：画布中央那个三角形。
    ///
    /// ⚠️ [`zephyra_core::paint::Vertex::position`] 是 **DIP 屏幕坐标**（不是
    /// NDC）—— DIP→NDC 的换算在 `quad::expand` 里做。写成 NDC 会让三角形落到
    /// 画布外，而症状是「画布是空的」，看起来像光栅器坏了。
    fn path_batch() -> quad::QuadBatch {
        let mut l = zephyra_core::paint::DisplayList::new();
        let v = |x: f32, y: f32| zephyra_core::paint::Vertex {
            position: zephyra_core::core::Point::new(x, y),
            uv: zephyra_core::core::Point::new(0.0, 0.0),
        };
        l.push(
            zephyra_core::paint::SortKey {
                layer: zephyra_core::paint::Layer::Content,
                z: 0,
                order: 0,
            },
            zephyra_core::paint::Command::Path {
                vertices: std::sync::Arc::new([v(30.0, 10.0), v(70.0, 10.0), v(50.0, 40.0)]),
                fill: zephyra_core::core::Color::rgb(1.0, 0.0, 0.0),
                filter: Default::default(),
                blend: Default::default(),
            },
        );
        l.sort();
        quad::expand(&l, (100, 100), |_| true)
    }

    /// 矢量图标（`Command::Path`）必须真的出现在 CPU 参考图里。
    ///
    /// # 这条在钉什么
    ///
    /// `draw_batch` 曾经只遍历 `solid` 与 `images` 两段，`paths` 整段从未被
    /// 光栅化 —— 与历史上 `images` 那段同型的洞。症状安静到无法靠人眼发现：
    /// `report.path_triangles` 照常有数（统计看着很健康），图里却一个图标都没有，
    /// 而 `--dump-cpu` 是本仓唯一的像素级自验手段。
    ///
    /// 判据分两半，缺一不可：
    /// 1. 三角形的**内部**像素必须是白色（证明确实画了）；
    /// 2. 三角形**外面**必须保持清屏色（证明没有把整块乱糊）。
    /// 只断言第 1 条的话，「无条件填满画布」也能过。
    #[test]
    fn polygon_rasterizes_onto_canvas() {
        let batch = path_batch();
        assert_eq!(
            batch.report.path_triangles, 1,
            "这个批次应当正好是一个三角形（否则下面的判据在测别的东西）"
        );
        assert_eq!(
            batch.paths.len(),
            3,
            "`paths` 段应当带着那 3 个顶点（`report` 与顶点流必须对得上）"
        );
        let mut canvas = CpuCanvas::new(100, 100, [0.0; 4]);
        draw_batch(&mut canvas, &batch, None);

        let at = |x: u32, y: u32| canvas.pixels[(y as usize * 100 + x as usize) * 4 + 3];
        assert!(
            at(50, 20) > 200,
            "三角形内部 (50,20) 的 alpha = {}，应当接近不透明 —— `paths` 段没有光栅化",
            at(50, 20)
        );
        assert_eq!(
            at(2, 2),
            0,
            "三角形外的 (2,2) 应当仍是清屏色（alpha=0）—— 说明光栅化把整块都糊上了"
        );
    }

    /// 把 `batches` 里**第一张图像**的 `uv` 换成整张纹理。
    ///
    /// 图像**不采样纹理**（见 `kind::IMAGE` 分支的注释），所以 uv 对像素没有影响 ——
    /// 但把它规整成 `FULL` 让构造读起来不误导：像是「一张铺满的图」而不是
    /// 「一张 uv 乱写的图」。
    fn image_batch(dst: zephyra_core::core::Rect, radius: f32) -> quad::QuadBatch {
        let mut l = zephyra_core::paint::DisplayList::new();
        l.push(
            zephyra_core::paint::SortKey {
                layer: zephyra_core::paint::Layer::Content,
                z: 0,
                order: 0,
            },
            zephyra_core::paint::Command::Image {
                texture: zephyra_core::paint::TextureRef::new("t"),
                src: Some(zephyra_core::core::UvRect::FULL),
                dst,
                radius: zephyra_core::core::CornerRadius { radius },
                tint: zephyra_core::core::Color::rgb(1.0, 1.0, 1.0),
                filter: Default::default(),
                blend: Default::default(),
            },
        );
        l.sort();
        quad::expand(&l, (100, 100), |_| true)
    }

    /// 圆角图像的**四角必须是空的**。
    ///
    /// # 为什么这条能钉住 CPU 侧的遮罩
    ///
    /// 本参考光栅器**不采样纹理**，所以图像的可见形状**全部**来自这个遮罩。
    /// 遮罩若没施加（或只在半径 > 0 时才施加的判据写反），画出来就是一块
    /// **方**块 —— 而「方块」在「有没有画出来」这条判据下与圆角**无法区分**。
    /// 所以判据必须是「四角空、中心实」这种**只看形状**的断言。
    #[test]
    fn rounded_image_corners_are_cut_off() {
        let dst = zephyra_core::core::Rect::new(10.0, 10.0, 80.0, 80.0);
        let mut canvas = CpuCanvas::new(100, 100, [0.0; 4]);
        draw_batch(&mut canvas, &image_batch(dst, 20.0), None);

        let at = |x: u32, y: u32| canvas.pixels[(y as usize * 100 + x as usize) * 4 + 3];
        // 四个**紧贴矩形角**的像素：圆角 20 下它们在形状之外。
        for (x, y) in [(11u32, 11u32), (88, 11), (11, 88), (88, 88)] {
            assert_eq!(
                at(x, y),
                0,
                "({x},{y}) 是圆角矩形的角，应该被切掉却是实心的 —— 遮罩没生效"
            );
        }
        // 中心必须是实的 —— 否则这条会因「什么都没画」而恒真。
        assert!(at(50, 50) > 200, "中心必须画上，否则上面四条断言是恒真的");
        // 矩形中边上、贴着上沿的点：不在角上，应���是实的。
        assert!(at(50, 12) > 200, "上边中点应��是实的");
    }

    /// 半径为 0 时**不施加**遮罩 —— 整块实心。
    ///
    /// ⚠️ 这条与 GPU 的 `corner.x > 0` 分支是**同一个陷阱**：
    /// SDF 的 `d == 0` 恰好落在矩形边界，`1 - smoothstep(-px, px, 0) == 0.5` ——
    /// 无条件施加会让最外一圈像素压到 50% alpha，两边就分叉了。
    /// 症状是「所有直角图像的边缘发暗一圈」，且没有任何计数器会变。
    #[test]
    fn square_corner_image_applies_no_mask() {
        let dst = zephyra_core::core::Rect::new(10.0, 10.0, 80.0, 80.0);
        let mut canvas = CpuCanvas::new(100, 100, [0.0; 4]);
        draw_batch(&mut canvas, &image_batch(dst, 0.0), None);
        // 紧贴四边的像素必须是**不透明**的，而不是 50%。
        for (x, y) in [(10u32, 10u32), (10, 89), (89, 10), (89, 89)] {
            let a = canvas.pixels[(y as usize * 100 + x as usize) * 4 + 3];
            assert!(
                a > 240,
                "({x},{y}) 的 alpha 只有 {a} —— 遮罩被无条件施加了（应 ≈255）"
            );
        }
    }

    /// 中心满、边界半、界外空 —— 覆盖率必须是 `d` 的单调函数。
    ///
    /// `d` 的定义就是「到边界的带符号距离」，所以覆盖率只能由它决定。
    #[test]
    fn sdf_覆盖率随到边界的距离单调下降() {
        let half = [10.0, 10.0]; // 20×20 的四边形
        // 边界在 local.x = 20，从内往外每 0.25 走一步。
        let samples: [([f32; 2], f32); 5] = [
            ([18.0, 10.0], 1.0),   // 深内部
            ([19.25, 10.0], 1.0), // 过渡带内侧
            ([19.75, 10.0], 0.741),
            ([20.0, 10.0], 0.5), // 抗锯齿中点
            ([20.75, 10.0], 0.0), // 界外
        ];
        let mut prev = 2.0f32;
        for (p, expect) in samples {
            let cov = sdf_coverage(p, half, [0.0, 0.0]);
            println!("  local={p:?}  cov={cov:.4}（期望约 {expect}）");
            assert!(
                cov <= prev + 1e-6,
                "覆盖率应当随靠近边界而单调不增，但 {p:?} 处是 {cov}，上一处是 {prev}"
            );
            assert!(
                (cov - expect).abs() < 0.02,
                "local={p:?} 处覆盖率应约 {expect}，实际 {cov}"
            );
            prev = cov;
        }
    }

    /// **`half_size` 必须是四边形的常量属性**，不是从逐片元坐标推出来的。
    ///
    /// 这条测试是那个 bug 的判据，也是修好它的判据。
    ///
    /// 早先的 HLSL 写 `half_size = i.local * 0.5`，于是它逐片元变化、且恰好
    /// `center == half_size`，`d` 退化成常数 `r(√2−1)`。后果是**同一个「到边界的距离」在
    /// 不同尺寸的四边形上给出不同的覆盖率** —— 因为当时 `d` 根本不等于那个距离。
    ///
    /// 所以这里换两种尺寸、停在同样的距边界 0.25 像素处，覆盖率必须逐位相同。
    #[test]
    fn sdf_覆盖率只取决于到边界的距离_与四边形尺寸无关() {
        // 20×20，边界在 x=20，往里 0.25 → x=19.75
        let small = sdf_coverage([19.75, 10.0], [10.0, 10.0], [0.0, 0.0]);
        // 100×100，边界在 x=100，往里 0.25 → x=99.75
        let big = sdf_coverage([99.75, 50.0], [50.0, 50.0], [0.0, 0.0]);
        assert!(
            (small - big).abs() < 1e-5,
            "同样距边界 0.25 像素，覆盖率必须一致，实际 {small} vs {big} —— \
             若不等，说明 half_size 又是从 local 推的（那个 bug）"
        );
    }

    /// 中心必须**满**。旧实现里中心的 `d` 恒为 0 → alpha 恒为 0.5，整块半透明。
    #[test]
    fn sdf_中心是满的_且_圆角半径_必须真正切掉角() {
        for (name, half) in [("20×20", [10.0f32, 10.0]), ("100×100", [50.0, 50.0])] {
            let center = sdf_coverage(half, half, [0.0, 0.0]);
            assert!(
                center > 0.99,
                "{name} 的中心覆盖率应为 1，实际 {center} —— \
                 若约 0.5，说明 half_size 又是从 local 推的（那个 bug）"
            );
        }

        // 角上：20×20、半径 8。取 local=(0.5,0.5)，即离左上和上边各 0.5 像素。
        let half = [10.0, 10.0];
        let p = [0.5, 0.5];
        let sharp = sdf_coverage(p, half, [0.0, 0.0]);
        let rounded = sdf_coverage(p, half, [8.0, 0.0]);
        println!("  方角 {sharp:.4} → 圆角8 {rounded:.4}");
        assert!(
            sharp > 0.9,
            "方角时 (0.5,0.5) 几乎贴着边界，覆盖率应很高，实际 {sharp}"
        );
        assert!(
            rounded < 0.1,
            "半径 8 的圆角应当把这个角切掉（离角最近的边界只有约 2.6 像素），实际 {rounded}"
        );
    }

    /// 描边只画**环带**，中心应当是空的。
    ///
    /// 这条测试钉住 `1.0 - smoothstep` 那个多余的反相。
    #[test]
    fn sdf_描边只画环带() {
        let half = [10.0, 10.0];
        let stroke = [0.0, 2.0]; // 无圆角，描边 2 像素
        assert!(
            sdf_coverage([10.0, 10.0], half, stroke) < 0.05,
            "描边模式下中心应当透明（环带只贴边）"
        );
        assert!(
            sdf_coverage([19.5, 10.0], half, stroke) > 0.5,
            "描边模式下贴近边界处应当不透明"
        );
        // 比描边宽度更深的内侧应当空 —— 环带只有那么宽。
        assert!(
            sdf_coverage([17.0, 10.0], half, stroke) < 0.05,
            "描边宽度 2 像素，所以 d = -3（x=17）处应当已经在环带之外"
        );
    }

    /// 四边形必须留出 AA 余量，过渡带的两端才都在可见范围内。
    ///
    /// # 这条测试原来是「钉住缺陷」，现已反转为「钉住修复」
    ///
    /// 之前 `Vertex::quad` 让四边形恰好等于矩形，于是 `local - half_size` 每个轴都
    /// 落在 `[-half, +half]` 内，`abs(p) - half ≤ 0` ⇒ **`d ≤ 0` 恒成立**。填充的
    /// 过渡带在 `d ∈ (0, 0.75]`、描边的 `sa` 在 `d > 0` 时饱和为 1 —— 那半边
    /// **永远不会被采样**，所以所有矩形边缘都是硬边。
    ///
    /// 现在四边形按 [`crate::quad::AA_BAND`] 向外扩，`half_size` 跟着扩，于是片元
    /// 能落到 `d ∈ (0, AA_BAND]`，`smoothstep` 的两端都可见。
    #[test]
    fn quad_reserves_aa_margin_so_upper_half_of_band_is_sampleable() {
        let rect = zephyra_core::core::Rect::new(10.0, 20.0, 64.0, 32.0);
        let vs = crate::quad::Vertex::from_rect(rect, (10.0, 20.0), [1.0; 4], [0.0, 0.0]);
        let band = crate::quad::AA_BAND;
        let half = vs[0].half_size;

        // `half_size` 与**扩后**几何对齐 —— 否则 `d == 0` 会偏移，描边环带长错位置。
        assert_eq!(
            half,
            [rect.width * 0.5 + band, rect.height * 0.5 + band],
            "half_size 必须等于扩后四边形的一半"
        );
        // `local` 的原点在**扩后**四边形的左上角，所以是 0；
        // 真实矩形对应 local ∈ [AA_BAND, AA_BAND + 尺寸] 这个更窄的区间。
        assert!(
            vs[0].local[0].abs() < 1e-5 && vs[0].local[1].abs() < 1e-5,
            "左上角 local 应为 0（扩后四边形的原点），实际 {:?}",
            vs[0].local
        );

        // 关键判据：四边形外沿（`d ≈ +AA_BAND`，在边界内侧一点点）必须**仍然是半透明**。
        // 这正是「原本该被裁掉的那半边」—— 它现在可被采样，所以读到 0.5 而不是 1。
        // 若四边形又缩回矩形大小，这里会读到 1.0（AA 又丢了）。
        let on_outer_band = [2.0 * half[0], half[1]];
        let cov = sdf_coverage(on_outer_band, half, [0.0, 0.0]);
        assert!(
            (cov - 0.5).abs() < 0.05,
            "四边形外沿（d ≈ +{band}）覆盖率应约 0.5，实际 {cov} —— \
             若为 1.0 说明四边形又缩回去了（抗锯齿又丢了）"
        );

        // 描边同理：外侧过渡带现在也可见。四边形外沿 `d = +AA_BAND` 恰好是
        // `smoothstep(-AA, AA, d)` 的**中点**，所以读到 0.5 —— 半透明，不是实色。
        //
        // 关键对比：修之前这里是 1.0（环带外侧饱和），于是描边多出一圈没有
        // 抗锯齿的实色，比设定值粗一圈。
        let stroke = [0.0, 2.0];
        let stroke_outer = sdf_coverage(on_outer_band, half, stroke);
        assert!(
            (stroke_outer - 0.5).abs() < 0.05,
            "描边模式下四边形外沿应是过渡带中点（约 0.5），实际 {stroke_outer} —— \
             若为 1.0 说明描边外侧没有淡出（描边会粗一圈且无抗锯齿）"
        );
        // 更外侧一点必须明显淡出，不能再有饱和的实色。
        //
        // ⚠ 取点要用**扩后半宽**做基准：片元里 `p = local - half_size`，而
        // `half_size` 已含 pad，所以「四边形右沿」在 `local.x == 2*half.x`。
        //
        // 这里读到的是**渐变过程中的一个值**而不是 0：该点 `d ≈ +0.375`，
        // 正落在 `smoothstep(-0.75, 0.75, ·)` 的上升段。判据是「远低于中点
        // 0.5」—— 修之前这里读到 1.0（饱和实色）。
        let beyond = [2.0 * half[0] + band * 0.5, half[1]];
        let beyond_cov = sdf_coverage(beyond, half, stroke);
        assert!(
            beyond_cov < 0.25,
            "四边形之外应明显淡出（读到 {beyond_cov}），\
             否则描边外侧是一圈硬边实色"
        );

        // 反过来，矩形**真实**边界（`d == 0`，即 local.x = half.x + band）
        // 上，填充必须是满的 —— AA 不该啃掉形状本身。
        let real_edge = [half[0] + band, half[1]];
        assert!(
            sdf_coverage(real_edge, half, [0.0, 0.0]) > 0.99,
            "矩形真实边界（d==0）填充应满，实际 {} —— AA 余量把形状啃小了",
            sdf_coverage(real_edge, half, [0.0, 0.0])
        );
    }

    #[test]
    fn blank_canvas_is_clear_colour() {
        let c = CpuCanvas::new(4, 4, [1.0, 0.5, 0.0, 1.0]);
        assert_eq!(&c.pixels[0..4], &[255, 128, 0, 255]);
    }

    #[test]
    fn atlas_sampling_takes_the_alpha_channel() {
        let mut px = vec![0u8; 4 * 4 * 4];
        px[3] = 200; // (0,0) 的 alpha
        let a = AtlasBitmap {
            pixels: px,
            size: 4,
        };
        assert!((a.sample_alpha([0.0, 0.0]) - 200.0 / 255.0).abs() < 0.01);
        // 越界的 uv 必须返回 0 而不是回绕（回绕会采到图集另一侧的字）。
        assert_eq!(a.sample_alpha([1.0, 0.0]), 0.0);
        assert_eq!(a.sample_alpha([1.5, 0.5]), 0.0);
    }

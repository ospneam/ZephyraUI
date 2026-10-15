// 本文件由 quad.rs 的 #[cfg(test)] mod tests 抽出 —— 测试代码与生产代码分离。
// 模块路径不变（src 里的声明用 #[path] 指过来），所以 use super::* 与私有可见性都照旧。

    use super::*;
    use zephyra_core::paint::SortKey;
    use zephyra_core::paint::Layer;

    fn list_of(cmds: Vec<Command>) -> DisplayList {
        let mut l = DisplayList::new();
        for (i, c) in cmds.into_iter().enumerate() {
            l.push(
                SortKey {
                    layer: Layer::Content,
                    z: 0,
                    order: i as u32,
                },
                c,
            );
        }
        l.sort();
        l
    }

    fn quad(rect: Rect, fill: Color) -> Command {
        Command::Quad {
            rect,
            radius: zephyra_core::core::CornerRadius { radius: 0.0 },
            fill,
            stroke: None,
            filter: zephyra_core::filter::FilterChain::none(),
            blend: zephyra_core::filter::BlendMode::default(),
        }
    }

    /// 图像命令（测试用替身：无滤镜、默认混合、**直角**）。
    ///
    /// 直角是**默认**：绝大多数测试只想验「图像进了 images 段」，而圆角会多出
    /// 外扩与遮罩，把那些判据搅浑。需要圆角的测试用 [`image_rounded`]。
    fn image(name: &'static str, dst: Rect) -> Command {
        image_rounded(name, dst, 0.0)
    }

    /// 带圆角的图像命令。
    fn image_rounded(name: &'static str, dst: Rect, radius: f32) -> Command {
        Command::Image {
            texture: zephyra_core::paint::TextureRef::new(name),
            src: None,
            dst,
            radius: zephyra_core::core::CornerRadius { radius },
            tint: Color::rgb(1.0, 1.0, 1.0),
            filter: zephyra_core::filter::FilterChain::none(),
            blend: zephyra_core::filter::BlendMode::default(),
        }
    }

    // ================= 复用缓冲 =================

    /// 复用缓冲跑很多帧，几何量与统计**逐帧不变**。
    ///
    /// # 这条在钉什么
    ///
    /// [`Scratch`] 让 `expand_into` 跨帧复用同一批 `Vec`，而
    /// [`QuadBatch::append_to`] 是往末尾**追加**的 —— 于是"忘了清"这件事的后果是
    /// 顶点流逐帧变长：第 N 帧的 `vertices` 有 N 帧的量。
    ///
    /// 那个 bug 极难靠别的手段发现：
    ///
    /// - **单帧是对的**（`--dump-cpu` 只跑一帧，看不出）；
    /// - **画面不会明显错**（重叠 N 层与重叠 1 层在截图里几乎一样）；
    /// - **不报任何错**（`RenderReport` 里 `quads` 一直是对的，因为它数的是
    ///   `solid.len()/4`，而 `solid` 被 `take` 掏空后重建，量是真的）。
    ///
    /// 唯一可靠的信号就是"帧数越多，缓冲越大"。所以判据取**逐帧相等**，而不是
    /// "某个上限之内" —— 后者对缓慢增长完全放过。
    #[test]
    fn reused_scratch_does_not_grow_across_frames() {
        let list = list_of(vec![
            quad(Rect::new(0.0, 0.0, 100.0, 100.0), Color::rgb(1.0, 0.0, 0.0)),
            quad(Rect::new(10.0, 10.0, 50.0, 50.0), Color::rgb(0.0, 1.0, 0.0)),
        ]);
        let mut scratch = Scratch::new();
        let mut first: Option<(usize, usize, usize, usize)> = None;
        for frame in 0..8 {
            let report = expand_into(&list, (800, 600), &|_| true, &mut scratch);
            assert_eq!(
                report.quads, 2,
                "四边形数不该随帧变（帧 {frame}）—— 它变了说明 solid 被重复累加"
            );
            let snapshot = (
                scratch.solid.len(),
                scratch.images.len(),
                scratch.paths.len(),
                scratch.vertices.len(),
            );
            match first {
                None => first = Some(snapshot),
                Some(f) => assert_eq!(
                    snapshot, f,
                    "第 {frame} 帧的顶点量与第 0 帧不同：{snapshot:?} vs {f:?} —— \
                     复用缓冲没有清干净（见 Scratch::reset 的说明：漏清的症状是 \
                     逐帧线性增长，且**不报任何错**）"
                ),
            }
        }
    }

    /// 复用路径与一次性路径**逐字节**同产。
    ///
    /// 复用版与纯函数版共用同一份遍历，所以它们必须给出完全相同的几何与统计。
    /// 一旦不共用了（比如为了性能把图像分批单独重写一遍），分叉的症状是
    /// "某类图元画错位置" —— 那与"这次改动只是想省点分配"完全无关，
    /// 几乎不可能被联想到。
    #[test]
    fn reused_path_byte_identical_to_one_shot_path() {
        let mut list = DisplayList::new();
        for i in 0..6u32 {
            list.push(
                SortKey {
                    layer: Layer::Content,
                    z: 0,
                    order: i,
                },
                quad(
                    Rect::new(i as f32 * 3.0, 0.0, 40.0, 20.0),
                    Color::rgb(0.2, 0.4, 0.6).with_alpha(1.0),
                ),
            );
        }
        // 插一段文本命令，它走的是另一条分支（字形四边形）。
        list.push(
            SortKey {
                layer: Layer::Content,
                z: 0,
                order: 6,
            },
            Command::Text {
                atlas: zephyra_core::paint::TextureRef::new("glyph-atlas"),
                glyphs: vec![zephyra_core::paint::GlyphQuad {
                    uv: UvRect::new(0.1, 0.2, 0.05, 0.05),
                    dst: Rect::new(0.0, 0.0, 8.0, 8.0),
                    color_glyph: false,
                }]
                .into(),
                offset: Point::new(0.0, 0.0),
                color: Color::rgb(1.0, 1.0, 1.0),
                filter: zephyra_core::filter::FilterChain::none(),
                blend: zephyra_core::filter::BlendMode::default(),
            },
        );
        list.sort();

        let fresh = expand(&list, (800, 600), |_| true);

        let mut scratch = Scratch::new();
        let report = expand_into(&list, (800, 600), &|_| true, &mut scratch);
        // 跑两遍：第二遍验证"复用的缓冲被上一帧的成果污染"这种可能性。
        expand_into(&list, (800, 600), &|_| true, &mut scratch);

        assert_eq!(
            scratch.solid, fresh.solid,
            "复用路径的 solid 与一次性路径不同"
        );
        assert_eq!(
            scratch.images, fresh.images,
            "复用路径的 images 与一次性路径不同"
        );
        assert_eq!(scratch.paths, fresh.paths, "路径段不同");
        assert_eq!(report.quads, fresh.report.quads, "quads 统计不同");
        assert_eq!(
            scratch.image_first,
            fresh.solid.len() as u32,
            "图像段起点必须紧随 solid"
        );
    }

    /// 顶点结构必须与着色器输入描述符逐字节对齐。
    ///
    /// 这条断言值不值一条测试：`Vertex` 是 `Pod`，字段顺序或类型一变，尺寸就变，
    /// 而 D3D11 不会检查 —— 症状是**画面整体错位**（颜色当 uv 读），不报任何错。
    #[test]
    fn vertex_field_offsets_match_the_input_layout_description() {        assert_eq!(
            std::mem::size_of::<Vertex>(),
            60,
            "15 个 f32。改了字段数必须同步改 shader.rs 的输入描述符"
        );
        // 这些偏移就是 `shader.rs` 里 `D3D11_INPUT_ELEMENT_DESC::AlignedByteOffset`
        // 的取值 —— 两处必须一起改，D3D11 不会替你发现不一致。
        let v = Vertex {
            position: [0.0; 2],
            local: [0.0; 2],
            uv: [0.0; 2],
            color: [0.0; 4],
            corner: [0.0; 2],
            half_size: [0.0; 2],
            kind: 0.0,
        };
        let base = &v as *const Vertex as usize;
        // 纯指针算术（在 `&v` 借用其存活期内），不需要 unsafe —— unsafe 只在
        // 解引用指针时才是必需的，这里没有解引用。
        let off = |p: *const u8| p as usize - base;
        assert_eq!(off(&v.position as *const _ as *const u8), 0);
        assert_eq!(off(&v.local as *const _ as *const u8), 8);
        assert_eq!(off(&v.uv as *const _ as *const u8), 16);
        assert_eq!(off(&v.color as *const _ as *const u8), 24);
        assert_eq!(off(&v.corner as *const _ as *const u8), 40);
        assert_eq!(off(&v.half_size as *const _ as *const u8), 48);
        assert_eq!(off(&v.kind as *const _ as *const u8), 56);
    }

    /// 四个顶点的 `half_size` 必须**完全相同**，且等于矩形尺寸的一半。
    ///
    /// 这条测试钉住 SDF 那个 bug 在**生产端**的一环。着色器原先没有这个通道，
    /// 改用 `local * 0.5` 现场推 —— 逐片元变化，于是 SDF 退化成常数。
    ///
    /// 「四个顶点相同」不是可有可无的细节：着色器里它标了 `nointerpolation`，
    /// 若哪天有人给四个顶点填了不同的值（按各自的 `local` 推、或者打算支持
    /// 透视校正之类），这里会立刻失败，而不是变成一个「某些矩形圆角是歪的」
    /// 且无任何报错的现象。
    #[test]
    fn half_size_是整个四边形的常量() {
        let r = Rect::new(3.0, 7.0, 40.0, 24.0);
        let vs = Vertex::from_rect(r, (3.0, 7.0), [1.0; 4], [6.0, 0.0]);
        // SDF 类别的四边形按 AA_BAND 外扩，所以 half_size 是**扩后**尺寸的一半。
        let want = [r.width * 0.5 + AA_BAND, r.height * 0.5 + AA_BAND];
        assert_eq!(want, [20.75, 12.75], "先确认这个用例本身算对了");
        for (i, v) in vs.iter().enumerate() {
            assert_eq!(
                v.half_size, want,
                "第 {i} 个顶点的 half_size 是 {:?}，应为 {want:?} —— \
                 它必须逐顶点相同，否则插值后就不再是常量",
                v.half_size
            );
        }
        // 反过来核对：`local` 的整个范围恰好是 `half_size * 2`（扩后四边形的尺寸），
        // 也就是片元能走遍 `p ∈ [-half, +half]` 全域 —— 这是过渡带两端都可见的前提。
        for (i, v) in vs.iter().enumerate() {
            assert_eq!(
                [v.local[0], v.local[1]],
                [
                    if i == 1 || i == 2 { want[0] * 2.0 } else { 0.0 },
                    if i == 2 || i == 3 { want[1] * 2.0 } else { 0.0 }
                ],
                "第 {i} 个顶点的 local 与 half_size*2 对不上，SDF 的两个输入单位不一致"
            );
        }
    }

    #[test]
    fn a_rect_becomes_one_quad_of_four_vertices() {
        let l = list_of(vec![quad(Rect::new(0.0, 0.0, 10.0, 10.0), Color::rgb(1.0, 0.0, 0.0))]);
        let b = expand(&l, (100, 100), |_| true);
        assert_eq!(b.solid.len(), 4);
        assert_eq!(b.report.quads, 1);
        assert_eq!(b.report.skipped, 0);
    }

    #[test]
    fn a_stroke_adds_a_second_ring_quad() {
        let mut l = DisplayList::new();
        l.push(
            SortKey {
                layer: Layer::Content,
                z: 0,
                order: 0,
            },
            Command::Quad {
                rect: Rect::new(0.0, 0.0, 10.0, 10.0),
                radius: zephyra_core::core::CornerRadius { radius: 2.0 },
                fill: Color::rgb(1.0, 1.0, 1.0),
                stroke: Some((Color::rgb(0.0, 0.0, 0.0), 1.0)),
                filter: zephyra_core::filter::FilterChain::none(),
                blend: zephyra_core::filter::BlendMode::default(),
            },
        );
        let b = expand(&l, (100, 100), |_| true);
        assert_eq!(b.report.quads, 2, "填充与描边是两个独立图元");
        // 描边那一段必须带描边宽度，否则片元里分不出环带。
        let stroke = &b.solid[4];
        assert_eq!(stroke.corner, [2.0, 1.0], "radius + stroke width");
    }

    #[test]
    fn fully_transparent_quads_produce_nothing() {
        let l = list_of(vec![quad(
            Rect::new(0.0, 0.0, 10.0, 10.0),
            Color::rgba(1.0, 1.0, 1.0, 0.0),
        )]);
        let b = expand(&l, (100, 100), |_| true);
        assert!(b.solid.is_empty());
        assert_eq!(b.report.quads, 0);
    }

    // ------------------------------------------------------ 多边形（Command::Path）

    /// 造一个 `Command::Path`：给定 DIP 空间的三角形顶点。
    fn path_cmd(tris: &[[f32; 2]; 3], fill: Color) -> Command {
        let verts: std::sync::Arc<[zephyra_core::paint::Vertex]> = tris
            .iter()
            .map(|p| zephyra_core::paint::Vertex {
                position: zephyra_core::core::Point { x: p[0], y: p[1] },
                uv: zephyra_core::core::Point { x: 0.0, y: 0.0 },
            })
            .collect::<Vec<_>>()
            .into();
        Command::Path {
            vertices: verts,
            fill,
            filter: zephyra_core::filter::FilterChain::default(),
            blend: zephyra_core::filter::BlendMode::default(),
        }
    }

    /// 无裁剪快路径与"走完整裁剪链"必须产出**逐顶点相同**的结果。
    ///
    /// # 这条在钉什么
    ///
    /// [`expand_into`] 的 `Command::Path` 分支对 `clip_rect == None` 走一条快路径
    /// （直接变换 NDC，零堆分配），对 `Some(_)` 走 Sutherland–Hodgman +
    /// 扇形三角化。两条路必须是**同一个映射**：
    ///
    /// - 快路径只做一遍退化检查（`signed_area2 ≈ 0` 的三角形跳过）；
    /// - 完整链里 `geom_clip(_, None)` 返回原三角形，`fan_triangulate` 对三点
    ///   多边形也只做那一次退化检查。
    ///
    /// 两条路都"看起来对"过 —— 症状是难查的那种：图标整体位置/大小正常，
    /// 只是**顶点顺序或退化三角形处理**与原实现不同。这类差异不崩、不报错，
    /// 也不会被"画出来了吗"这类检查抓到。
    ///
    /// 所以判据取逐顶点相等，没有第二条路能蒙混过去。
    #[test]
    fn unclipped_fast_path_matches_full_clip_vertex_by_vertex() {
        // 三组形状：常规、共线（退化）、跨越大画布。
        let cases: [&[[f32; 2]; 3]; 3] = [
            &[[0.0, 0.0], [10.0, 0.0], [0.0, 10.0]],
            // 三点共线：两条路都必须把它剔除（`path_triangles` 为 0）。
            &[[0.0, 0.0], [5.0, 0.0], [10.0, 0.0]],
            // 顶点远在画布之外：NDC 会远大于 1，两条路都不该因此丢三角形
            //（裁剪矩形为 None 时没有"画布边界"这个概念）。
            &[[-500.0, -500.0], [900.0, -500.0], [-500.0, 900.0]],
        ];
        for (i, tri) in cases.iter().enumerate() {
            let l = list_of(vec![path_cmd(tri, Color::rgba(1.0, 0.0, 0.0, 1.0))]);
            let fast = expand(&l, (100, 100), |_| true);

            // 同一条命令，但显式包一层"覆盖整个画布"的 PushClip ——
            // 这样 `clip_rect` 是 Some，而裁剪本身不改变任何顶点。
            let l2 = list_of(vec![
                Command::PushClip(Rect::new(-1000.0, -1000.0, 3000.0, 3000.0)),
                path_cmd(tri, Color::rgba(1.0, 0.0, 0.0, 1.0)),
                Command::PopClip,
            ]);
            let full = expand(&l2, (100, 100), |_| true);

            assert_eq!(
                fast.paths.len(),
                full.paths.len(),
                "第 {i} 组：快路径产出 {} 个顶点，完整链产出 {} 个",
                fast.paths.len(),
                full.paths.len()
            );
            assert_eq!(
                fast.report.path_triangles, full.report.path_triangles,
                "第 {i} 组：三角形计数不同"
            );
            for (j, (a, b)) in fast.paths.iter().zip(full.paths.iter()).enumerate() {
                assert_eq!(
                    a.position, b.position,
                    "第 {i} 组第 {j} 个顶点的位置不同：{:?} vs {:?}",
                    a.position, b.position
                );
                assert_eq!(a.kind, b.kind, "第 {i} 组第 {j} 个顶点的 kind 不同");
            }
        }
    }

    /// 退化三角形（面积为零）必须被剔除 —— 快路径与完整链**都**要剔。
    ///
    /// # 为什么单列一条
    ///
    /// 快路径手写了退化检查（`signed_area2 ≈ 0` 就 `continue`）。它是**行为
    /// 等价**的一部分，不是优化 —— 漏掉它的症状是"看不见但占索引"的三角形
    /// 进入顶点流：`path_triangles` 计数偏大，而按 `Σ|面积|` 的断言**仍然通过**
    /// （它本来就是面积和的一部分），所以只有这条能抓到。
    #[test]
    fn degenerate_triangle_is_culled_on_both_paths() {
        let collinear = list_of(vec![path_cmd(
            &[[0.0, 0.0], [5.0, 0.0], [10.0, 0.0]],
            Color::rgba(1.0, 0.0, 0.0, 1.0),
        )]);
        let fast = expand(&collinear, (100, 100), |_| true);
        assert_eq!(
            fast.report.path_triangles, 0,
            "共线三角形应被剔除，却产出了 {} 个三角形",
            fast.report.path_triangles
        );
        assert!(fast.paths.is_empty(), "被剔除的三角形不该留下顶点");

        let clipped = list_of(vec![
            Command::PushClip(Rect::new(-1000.0, -1000.0, 3000.0, 3000.0)),
            path_cmd(
                &[[0.0, 0.0], [5.0, 0.0], [10.0, 0.0]],
                Color::rgba(1.0, 0.0, 0.0, 1.0),
            ),
            Command::PopClip,
        ]);
        let full = expand(&clipped, (100, 100), |_| true);
        assert_eq!(
            full.report.path_triangles, 0,
            "完整裁剪链应同样剔除共线三角形，却产出了 {} 个",
            full.report.path_triangles
        );
    }

    #[test]
    fn path_triangles_land_in_their_own_segment_not_in_solid() {
        let l = list_of(vec![path_cmd(
            &[[0.0, 0.0], [10.0, 0.0], [0.0, 10.0]],
            Color::rgba(1.0, 0.0, 0.0, 1.0),
        )]);
        let b = expand(&l, (100, 100), |_| true);
        assert_eq!(b.paths.len(), 3, "一个三角形 → 3 个顶点");
        assert_eq!(b.report.path_triangles, 1);
        // ⚠ 必须**不在** solid 段：solid 的索引按"每 4 个顶点一个四边形"展开，
        // 混进 3 个顶点会让它后面所有图元的基址错位。
        assert!(b.solid.is_empty());
        assert_eq!(b.report.quads, 0);
        assert_eq!(b.report.skipped, 0, "Path 已接通，不该再计入 skipped");
    }

    #[test]
    fn path_vertices_are_in_ndc_and_carry_the_path_kind() {
        let l = list_of(vec![path_cmd(
            &[[0.0, 0.0], [100.0, 0.0], [0.0, 100.0]],
            Color::rgba(0.0, 1.0, 0.0, 1.0),
        )]);
        let b = expand(&l, (100, 100), |_| true);
        // 目标100×100像素：左上角 (0,0) → NDC (-1, +1)；右下 (100,100) → (+1,-1)。
        assert_eq!(b.paths[0].position, [-1.0, 1.0]);
        assert_eq!(b.paths[2].position, [-1.0, -1.0]);
        for v in &b.paths {
            assert_eq!(v.kind, kind::PATH);
            assert_eq!(v.color, [0.0, 1.0, 0.0, 1.0]);
            // 未初始化的通道会让 MSAA 边缘采到垃圾字节。
            assert_eq!(v.local, [0.0; 2]);
            assert_eq!(v.uv, [0.0; 2]);
            assert_eq!(v.half_size, [0.0; 2]);
        }
    }

    #[test]
    fn path_indices_are_two_segments_and_the_path_base_is_the_quad_count() {
        // 2 个四边形 + 1 个路径三角形
        let l = list_of(vec![
            quad(Rect::new(0.0, 0.0, 10.0, 10.0), Color::rgba(1.0, 1.0, 1.0, 1.0)),
            quad(
                Rect::new(20.0, 0.0, 10.0, 10.0),
                Color::rgba(1.0, 1.0, 1.0, 1.0),
            ),
            path_cmd(
                &[[0.0, 0.0], [5.0, 0.0], [0.0, 5.0]],
                Color::rgba(0.0, 0.0, 1.0, 1.0),
            ),
        ]);
        let b = expand(&l, (100, 100), |_| true);
        let (all, _, _) = b.flatten();
        let quads = (b.solid.len() + b.images.len()) / 4;
        let (idx, path_start) = expand_indices_with_paths(quads, b.paths.len() / 3);
        assert_eq!(quads, 2);
        assert_eq!(path_start as usize, 2 * 6, "路径段紧接在四边形段之后");
        assert_eq!(idx.len(), 2 * 6 + 3);
        // 路径段的基址 = 四边形数 × 4（顶点流里 solid+images 在前）。
        assert_eq!(&idx[path_start as usize..], &[8, 9, 10]);
        // 所有索引都指向真实存在的顶点。
        for &i in &idx {
            assert!((i as usize) < all.len(), "索引 {i} 越界（共{} 顶点）", all.len());
        }
    }

    #[test]
    fn a_clipped_path_drops_the_triangles_outside_the_clip_rect() {
        let clip = Command::PushClip(Rect::new(0.0, 0.0, 10.0, 10.0));
        // 三角 (0,0)(20,0)(0,20)，斜边 x+y=20。裁剪区 `[0,10]²` 内
        // x+y 恒 ≤ 20，所以交集是**整个** 10×10 方块 —— 面积 100。
        // 这里的重点是"跨界三角形被裁剪后**仍然出几何**"，而不是被整块丢弃；
        // 面积取 100 是几何事实，不是实现约定。
        let l = list_of(vec![
            clip,
            path_cmd(
                &[[0.0, 0.0], [20.0, 0.0], [0.0, 20.0]],
                Color::rgba(1.0, 1.0, 1.0, 1.0),
            ),
        ]);
        let b = expand(&l, (100, 100), |_| true);
        assert!(b.paths.len() >= 3, "跨界三角形应被裁剪而不是整块丢弃");
        let area: f32 = b
            .paths
            .chunks_exact(3)
            .map(|t| {
                let pts: Vec<zephyra_core::core::Point> =
                    t.iter().map(|v| dip_of(v.position)).collect();
                zephyra_core::geom::abs_area(&pts)
            })
            .sum();
        assert!(
            (area - 100.0).abs() < 0.01,
            "交集是整个 10×10 方块（面积 100），实得 {area}"
        );
    }

    #[test]
    fn a_path_fully_inside_the_clip_is_not_changed() {
        let l = list_of(vec![
            Command::PushClip(Rect::new(0.0, 0.0, 100.0, 100.0)),
            path_cmd(
                &[[10.0, 10.0], [20.0, 10.0], [10.0, 20.0]],
                Color::rgba(1.0, 1.0, 1.0, 1.0),
            ),
        ]);
        let b = expand(&l, (100, 100), |_| true);
        assert_eq!(b.paths.len(), 3, "完全在内的三角形原样通过");
        let area: f32 = b
            .paths
            .chunks_exact(3)
            .map(|t| {
                let pts: Vec<zephyra_core::core::Point> =
                    t.iter().map(|v| dip_of(v.position)).collect();
                zephyra_core::geom::abs_area(&pts)
            })
            .sum();
        assert!((area - 50.0).abs() < 0.01, "面积 50，实得 {area}");
    }

    #[test]
    fn a_path_fully_outside_the_clip_produces_nothing() {
        let l = list_of(vec![
            Command::PushClip(Rect::new(0.0, 0.0, 10.0, 10.0)),
            path_cmd(
                &[[50.0, 50.0], [60.0, 50.0], [50.0, 60.0]],
                Color::rgba(1.0, 1.0, 1.0, 1.0),
            ),
        ]);
        let b = expand(&l, (100, 100), |_| true);
        assert!(b.paths.is_empty(), "完全在外的路径不该产出任何顶点");
        assert_eq!(b.report.path_triangles, 0);
    }

    /// NDC → DIP（目标 100×100，用于从展开结果还原坐标）。
    fn dip_of(ndc: [f32; 2]) -> zephyra_core::core::Point {
        zephyra_core::core::Point {
            x: (ndc[0] + 1.0) * 50.0,
            y: (1.0 - ndc[1]) * 50.0,
        }
    }

    #[test]
    fn images_split_into_batches_per_texture_without_reordering() {
        // a b a  →  三个批次。合并不连续的 a 会把第三段提到第二段前面，
        // 而图像常是半透明的，重排直接改变可见结果。
        let img = |name: &'static str| image(name, Rect::new(0.0, 0.0, 4.0, 4.0));
        let l = list_of(vec![img("a"), img("b"), img("a")]);
        let b = expand(&l, (100, 100), |_| true);
        let names: Vec<&str> = b.batches.iter().map(|(n, _)| &**n).collect();
        assert_eq!(names, ["a", "b", "a"], "同纹理但不连续就必须分成两批");
        assert_eq!(b.report.image_batches, 3);
    }

    #[test]
    fn consecutive_same_texture_images_merge_into_one_batch() {
        let img = |name: &'static str| image(name, Rect::new(0.0, 0.0, 4.0, 4.0));
        let l = list_of(vec![img("a"), img("a"), img("a")]);
        let b = expand(&l, (100, 100), |_| true);
        assert_eq!(b.report.image_batches, 1, "连续同纹理合并");
        assert_eq!(b.batches[0].1, 12, "3 个四边形 = 12 个顶点");
    }

    /// 只合并**相邻**的同混合模式段 —— 中间隔着别的段时不许跨过去合并。
///
/// # 为什么"跨过去合并"是错的
///
/// 那需要把后面的命令重排到前面才能凑到一起。而半透明图形互相叠加时，重排会改变
/// 可见结果。所以 draw 次数是命令顺序的**上界**，不是最小值 —— 这是刻意的取舍。
///
/// # ⚠ 别拿"隔着别的**图元类型**"当例子
///
/// 比如「矩形 / 三角形 / 矩形」并不会拆成三段：合并流里 `solid` 整段在前、
/// `paths` 整段在后（见 [`QuadBatch::flatten`]），所以两个矩形在段内本来就是相邻的，
/// 合并是正确的。真正需要拆的是**同一段内混合模式不同**的情形 —— 合成器会照
/// `runs` 逐段设混合状态，跨过去合并会让中间那一段套上错误的方程。
#[test]
fn runs_不跨过其它段合并() {
    use zephyra_core::filter::BlendMode;
    let l = list_of(vec![
        quad_with_blend(Rect::new(0.0, 0.0, 10.0, 10.0), BlendMode::Multiply),
        quad(Rect::new(20.0, 0.0, 10.0, 10.0), Color::rgb(0.0, 1.0, 0.0)),
        quad_with_blend(Rect::new(40.0, 0.0, 10.0, 10.0), BlendMode::Multiply),
    ]);
    let b = expand(&l, (100, 100), |_| true);
    let starts: Vec<u32> = b.runs.iter().map(|r| r.first_vertex).collect();
    assert_eq!(
        starts,
        vec![0, 4, 8],
        "两个 Multiply 之间夹着一个 SrcOver ⇒ 三段，不得把两个 Multiply 凑成一段：{:?}",
        b.runs
    );
    let blends: Vec<BlendMode> = b.runs.iter().map(|r| r.blend).collect();
    assert_eq!(
        blends,
        vec![BlendMode::Multiply, BlendMode::default(), BlendMode::Multiply],
        "每段的混合模式必须与来源命令一致"
    );
}

/// 多个纹理分批时，**全部**图像顶点都必须留在合并流里。
    ///
    /// `batches` 报的是「每批多少顶点」，渲染器靠它从 `image_first` 起逐批推进；
    /// 而 `images` 段只有把每一批的顶点都留着才对得上。少一个就是渲染器读到
    /// 别的图元的位置上 —— 症状是图像错位/消失，且 `batches` 的统计完全正常。
    #[test]
    fn all_image_vertices_survive_multi_texture_batching() {
        let img = |name: &'static str| image(name, Rect::new(0.0, 0.0, 4.0, 4.0));
        let l = list_of(vec![img("a"), img("b"), img("a")]);
        let b = expand(&l, (100, 100), |_| true);
        assert_eq!(b.report.image_batches, 3, "a/b/a 分三批");
        let claimed: u32 = b.batches.iter().map(|(_, n)| *n).sum();
        assert_eq!(
            b.images.len() as u32,
            claimed,
            "batches 声称 {claimed} 个图像顶点，images 段却只有 {}",
            b.images.len()
        );
        assert_eq!(b.images.len(), 12, "3 张图 × 4 顶点");
    }

    #[test]
    fn an_unregistered_texture_is_counted_but_does_not_split_the_batch() {
        let img = |name: &'static str| image(name, Rect::new(0.0, 0.0, 4.0, 4.0));
        // a miss a → 拼错的名字不该把前后两段 a 拆成两次 draw。
        let l = list_of(vec![img("a"), img("miss"), img("a")]);
        let b = expand(&l, (100, 100), |n| n == "a");
        assert_eq!(b.report.image_batches, 1, "缺失项不打断当前批");
        assert_eq!(b.report.missing_textures, 1, "但必须被计数，否则拼错无声无息");
        assert_eq!(b.report.quads, 2, "缺失的那条不产出四边形");
    }

    #[test]
    fn clip_rect_cuts_the_quad_down() {
        let mut l = DisplayList::new();
        for (i, c) in [
            Command::PushClip(Rect::new(0.0, 0.0, 5.0, 5.0)),
            quad(Rect::new(0.0, 0.0, 100.0, 100.0), Color::rgb(1.0, 1.0, 1.0)),
            Command::PopClip,
        ]
        .into_iter()
        .enumerate()
        {
            l.push(
                SortKey {
                    layer: Layer::Content,
                    z: 0,
                    order: i as u32,
                },
                c,
            );
        }
        let b = expand(&l, (100, 100), |_| true);
        assert_eq!(b.solid.len(), 4, "仍是一个四边形，但尺寸被裁剪");
        // 裁剪把100×100 缩到 5×5，再收缩半个 AA 带（0.375），扩回后总跨度
        // = 5 - 0.75 + 1.5 = 5.75。local 走到 2*half_size = 5.75。
        let local = b.solid[2].local;
        assert_eq!(
            local,
            [5.75, 5.75],
            "local 必须反映裁剪 + 收缩半个 AA 带 + 外扩后的总跨度"
        );
        // 更要紧的是：裁剪后**不能**再画出裁剪线外的东西。
        // 四边形的右沿落在 local = 5.75，而 half_size = 2.875，
        // 于是 p.x 最大2.875 - ... 换算回屏幕坐标：右沿 = 0 + 5.75 = 5.75。
        // 真实矩形右沿是 5.0，超出0.75 —— 那正是淡出的过渡带（被 clip 之外
        // 的 scissor 裁掉一半，剩下一半渐隐），不是硬边。
        assert!(
            b.solid[2].position[0] > -1.0,
            "裁剪后四边形仍在视口内"
        );
    }

    // ==================================================================
    // 圆角图像
    // ==================================================================

    /// 只带圆角图像的列表。
    fn rounded_image_list(dst: Rect, radius: f32) -> DisplayList {
        let mut l = DisplayList::new();
        l.push(
            SortKey {
                layer: Layer::Content,
                z: 0,
                order: 0,
            },
            image_rounded("t", dst, radius),
        );
        l.sort();
        l
    }

    /// 圆角为 0 时**逐字节等价于** [`Vertex::from_uv_rect`] —— 一格都不许差。
    ///
    /// 这条是「圆角是可选能力」的根据。若这条挂了，最可能的成因是 `pad` 的
    /// `match` 写成了无条件 `AA_BAND`，于是直角图像白裁一圈；而症状是
    /// 「所有图像边缘发暗 / 内容缩小一点」，看起来像采样器或 uv 的问题，
    /// 排查方向完全错。
    ///
    /// # ⚠️ 必须直接比两个函数，**不能**拿两条 `expand` 结果互比
    ///
    /// 第一版写的是「`expand(半径 0 的命令)` vs `expand(直角命令)`」——
    /// 而测试辅助里的 `image()` 就是 `image_rounded(..., 0.0)`，**两条命令完全相同**，
    /// 于是两边走同一条路，那条测试只证明了「函数是确定性的」，恒真。
    ///
    /// 变异测试把它抓出来了：把 `if !(radius > 0.0)` 改成 `if false`（强制半径 0
    /// 也走外插路径）时，68 条测试**仍然全绿**。改成直接比两个函数的输出才抓住。
    #[test]
    fn zero_radius_is_byte_identical_to_from_uv_rect() {
        let dst = Rect::new(10.0, 20.0, 64.0, 32.0);
        let px = (200.0, 200.0);
        let want = Vertex::from_uv_rect(dst, px, [1.0; 4], UvRect::FULL, kind::IMAGE);
        for radius in [0.0, -1.0, f32::NAN] {
            let got = Vertex::from_rounded_uv_rect(dst, px, [1.0; 4], UvRect::FULL, radius);
            assert_eq!(
                got, want,
                "半径 {radius} 必须退化成 from_uv_rect —— 图像边缘会无缘无故发暗一圈"
            );
        }
    }

    /// 圆角图像必须按 `AA_BAND` 外扩，否则圆角是硬边。
    ///
    /// 判据是 `half_size`：它等于「扩后四边形」的一半，所以外扩量对它直接可见。
    /// ⚠️ 不能改成断言 `position` —— NDC 上 64.75/200 与 64/200 差得很小，
    /// 那种断言在改成别的目标尺寸时会静默失效。
    #[test]
    fn rounded_image_outsets_by_one_aa_band() {
        let dst = Rect::new(10.0, 20.0, 64.0, 32.0);
        let b = expand(&rounded_image_list(dst, 8.0), (200, 200), |_| true);
        assert_eq!(b.images.len(), 4, "一个图像四边形 = 4 个顶点");
        let hs = b.images[0].half_size;
        assert_eq!(
            hs,
            [64.0 / 2.0 + AA_BAND, 32.0 / 2.0 + AA_BAND],
            "half_size 必须是「扩后」四边形的一半，否则 SDF 的 d==0 落错位置"
        );
    }

    /// uv 必须**外插**而不是跟着外扩一起裁掉图像内容。
    ///
    /// # 判据是**具体值**
    ///
    /// 正确做法：让线性插值在**未外扩**的 `rect.x` 处恰好等于 `uv.x`。
    /// 于是「外扩处」算出来的 uv 是 `uv.x - (uv1-uv0) * AA_BAND / width` ——
    /// 稍微**小于** `uv.x`。
    ///
    /// 错误做法（把 uv 铺在扩后四边形上）：`uv.x` 本身，于是原矩形落在 quad
    /// 内缩 AA_BAND 处 = **裁掉并放大**一圈，32px 的图裁 4.7%，肉眼可见。
    ///
    /// 两者都「看起来有 uv」，所以必须断言符号。
    #[test]
    fn rounded_image_uv_is_extrapolated_not_cropped() {
        let dst = Rect::new(0.0, 0.0, 64.0, 64.0);
        let b = expand(&rounded_image_list(dst, 8.0), (200, 200), |_| true);
        let v = &b.images[0];
        // 左上角顶点：uv 应略小于 0（`UvRect::FULL` 的左上角是 (0,0)）。
        assert!(
            v.uv[0] < 0.0 && v.uv[1] < 0.0,
            "左上 uv 应外插到负值，实际 {:?} —— 这意味着图像被裁了一圈",
            v.uv
        );
        // 外插量恰好是 AA_BAND/边长 乘 uv 跨度。
        let expect = -AA_BAND / 64.0;
        assert!(
            (v.uv[0] - expect).abs() < 1e-6,
            "外插量应为 {expect}，实际 {}",
            v.uv[0]
        );
        // 右下角顶点：对称地略大于 1。
        let v2 = &b.images[2];
        assert!(
            v2.uv[0] > 1.0 && (v2.uv[0] - (1.0 - expect)).abs() < 1e-6,
            "右下 uv 应外插到 1 以上，实际 {:?}",
            v2.uv
        );
    }

    /// 半径要真的填进顶点 —— 否则片元里的 `corner.x > 0` 分支永不进，
    /// 圆角参数被静默吞掉（图像变成直角，且不报错）。
    #[test]
    fn corner_radius_reaches_the_vertex() {
        let b = expand(
            &rounded_image_list(Rect::new(0.0, 0.0, 40.0, 40.0), 12.0),
            (200, 200),
            |_| true,
        );
        for v in &b.images {
            assert_eq!(v.corner[0], 12.0, "corner.x 必须带圆角半径");
            assert_eq!(v.corner[1], 0.0, "图像没有描边，corner.y 应为 0");
        }
    }

    #[test]
    fn a_zero_area_target_does_not_divide_by_zero_into_nan() {
        // 最小化时客户区是 0×0。若不钳，NDC 变换会 0/0 → NaN，整帧静默变黑。
        let l = list_of(vec![quad(Rect::new(0.0, 0.0, 10.0, 10.0), Color::rgb(1.0, 1.0, 1.0))]);
        let b = expand(&l, (0, 0), |_| true);
        for v in &b.solid {
            for f in v.position {
                assert!(f.is_finite(), "0×0 目标下位置不该是 NaN/Inf：{v:?}");
            }
        }
    }

    #[test]
    fn ndc_maps_top_left_to_the_top_left_corner() {
        // y 翻转：核心原点左上，NDC 原点左下。
        //
        // 用字形类别（不外扩）验证映射本身 —— 它是几何基准。
        let v = Vertex::from_uv_rect(
            Rect::new(0.0, 0.0, 100.0, 100.0),
            (100.0, 100.0),
            [1.0; 4],
            UvRect::FULL,
            kind::GLYPH,
        );
        assert_eq!(v[0].position, [-1.0, 1.0], "左上 → NDC 左上");
        assert_eq!(v[2].position, [1.0, -1.0], "右下 → NDC 右下");
    }

    /// SDF 类别必须**略微越过**视口边界 —— 这正是 AA 余量。
    ///
    /// 与上一条严格对照：字形/图像不外扩（位图自带边缘），SDF 类别外扩
    /// [`AA_BAND`]。若这一条失败，多半是外扩被误加到了所有类别上 ——
    /// 那会让字形采样到图块外的像素（描边发白、图像边缘串色）。
    #[test]
    fn rect_kind_outsets_past_viewport_while_glyph_kind_does_not() {
        let rect = Rect::new(0.0, 0.0, 100.0, 100.0);
        let px = (100.0, 100.0);
        let sdf = Vertex::from_rect(rect, px, [1.0; 4], [0.0; 2]);
        assert!(
            sdf[0].position[0] < -1.0 && sdf[0].position[1] > 1.0,
            "SDF 四边形左上应越过 NDC 左上（外扩了 AA 带），实际 {:?}",
            sdf[0].position
        );

        let glyph = Vertex::from_uv_rect(rect, px, [1.0; 4], UvRect::FULL, kind::GLYPH);
        assert_eq!(
            glyph[0].position,
            [-1.0, 1.0],
            "字形类别不外扩 —— 位图自带抗锯齿，扩了会把边角啃掉"
        );

        let image = Vertex::from_uv_rect(rect, px, [1.0; 4], UvRect::FULL, kind::IMAGE);
        assert_eq!(
            image[0].position,
            [-1.0, 1.0],
            "图像类别不外扩 —— 扩了会把 uv 拉出图块"
        );
    }

    /// 外扩量必须恰好是 AA_BAND，且字形/图像**不受影响**。
    #[test]
    fn outset_amount_is_exactly_aa_band() {
        let rect = Rect::new(0.0, 0.0, 100.0, 100.0);
        let px = (100.0, 100.0);
        let sdf = Vertex::from_rect(rect, px, [1.0; 4], [0.0; 2]);
        // 左上角 NDC.x：外扩后左边在 -0.75 DIP 处。
        let expect_x = -AA_BAND / px.0 * 2.0 - 1.0;
        assert!(
            (sdf[0].position[0] - expect_x).abs() < 1e-5,
            "外扩量应恰好是 AA_BAND={AA_BAND}，NDC.x 期望 {expect_x}，实际 {}",
            sdf[0].position[0]
        );
    }

    /// 图集 uv 在归一化空间，核心给的是像素空间。
    #[test]
    fn out_of_range_uv_is_clamped_before_it_reaches_the_sampler() {
        let clamped = UvRect::new(0.3, 0.0, 0.9, 1.0).clamped();
        assert!(clamped.right() <= 1.0 && clamped.bottom() <= 1.0);
        assert_eq!(clamped.x, 0.3);
        assert_eq!(clamped.width, 0.7);
    }

    #[test]
    fn intersect_of_disjoint_rects_is_empty_not_negative() {
        let r = intersect(Rect::new(0.0, 0.0, 10.0, 10.0), Rect::new(50.0, 50.0, 10.0, 10.0));
        assert_eq!(r.width, 0.0);
        assert_eq!(r.height, 0.0);
    }
// ---- E3：`runs` 分段元数据 + `translate` 平移 ----

    fn quad_with_blend(rect: Rect, blend: zephyra_core::filter::BlendMode) -> Command {
        Command::Quad {
            rect,
            radius: zephyra_core::core::CornerRadius { radius: 0.0 },
            fill: Color::rgb(1.0, 1.0, 1.0),
            stroke: None,
            filter: zephyra_core::filter::FilterChain::none(),
            blend,
        }
    }

    fn image_with_blend(
        name: &'static str,
        dst: Rect,
        blend: zephyra_core::filter::BlendMode,
    ) -> Command {
        Command::Image {
            texture: zephyra_core::paint::TextureRef::new(name),
            src: None,
            dst,
            radius: zephyra_core::core::CornerRadius { radius: 0.0 },
            tint: Color::rgb(1.0, 1.0, 1.0),
            filter: zephyra_core::filter::FilterChain::none(),
            blend,
        }
    }

    /// **零回归判据**：按 `runs` 逐段切出来的顶点流拼起来 == 未切分的合并流。
    ///
    /// 这是 E3 存在的全部理由 —— 加了分段元数据之后，"合成器照着 `runs` 画"与
    /// "合成器照着整条流画"必须画出**同一个东西**。漏一段的症状是图元不出现，
    /// 两段重叠的症状是图元重复画（半透明时颜色变深，且不报任何错）。
    #[test]
    fn runs_spliced_rebuild_the_unsplit_stream() {
        use zephyra_core::filter::BlendMode;
        let l = list_of(vec![
            quad(Rect::new(0.0, 0.0, 10.0, 10.0), Color::rgb(1.0, 0.0, 0.0)),
            quad_with_blend(Rect::new(20.0, 0.0, 10.0, 10.0), BlendMode::Multiply),
            image("t1", Rect::new(0.0, 20.0, 10.0, 10.0)),
            image_with_blend("t1", Rect::new(20.0, 20.0, 10.0, 10.0), BlendMode::Screen),
            path_cmd(
                &[[0.0, 40.0], [10.0, 40.0], [10.0, 50.0]],
                Color::rgb(0.0, 1.0, 0.0),
            ),
        ]);
        let b = expand(&l, (100, 100), |_| true);
        let (all, _, _) = b.flatten();

        // 结构：连续、完整覆盖
        assert!(!b.runs.is_empty(), "这批命令里每种图元都有，runs 不该为空");
        assert_eq!(
            b.runs[0].first_vertex, 0,
            "第一段必须从流头开始，否则开头的图元没人画"
        );
        for w in b.runs.windows(2) {
            assert_eq!(
                w[1].first_vertex,
                w[0].end(),
                "段与段之间不许有缝也不许重叠：{:?} 之后是 {:?}",
                w[0],
                w[1]
            );
        }
        assert_eq!(
            b.runs.last().unwrap().end() as usize,
            all.len(),
            "最后一段必须正好到流尾，否则尾部的图元没人画（或被多画一次）"
        );

        // 判据本体：逐段切出来拼接 == 整条流
        let mut spliced = Vec::new();
        for r in &b.runs {
            let s = r.first_vertex as usize;
            spliced.extend_from_slice(&all[s..s + r.vertex_count as usize]);
        }
        assert_eq!(spliced, all, "按 runs 切分再拼起来必须与合并流逐顶点相同");
    }

    /// 混合模式全默认时 `runs` 就是原来的三段 —— 合批行为**没有**被 E3 改动。
    ///
    /// ⚠ 刻意放**两个**矩形：不合并的话这里会是 4 段而不是 3 段，所以这条钉住的
    /// 是「相邻同混合模式的段被合并」这件事，而不只是「段数等于三」。
    #[test]
    fn default_blend_yields_exactly_three_runs() {
        let l = list_of(vec![
            quad(Rect::new(0.0, 0.0, 10.0, 10.0), Color::rgb(1.0, 0.0, 0.0)),
            quad(Rect::new(20.0, 0.0, 10.0, 10.0), Color::rgb(0.0, 1.0, 0.0)),
            image("t1", Rect::new(0.0, 20.0, 10.0, 10.0)),
            path_cmd(
                &[[0.0, 40.0], [10.0, 40.0], [10.0, 50.0]],
                Color::rgb(0.0, 0.0, 1.0),
            ),
        ]);
        let b = expand(&l, (100, 100), |_| true);
        let (all, image_first, path_first) = b.flatten();
        assert_eq!(b.runs.len(), 3, "solid / images / paths 各一段：{:?}", b.runs);
        assert_eq!(b.runs[0].first_vertex, 0);
        assert_eq!(b.runs[0].end(), image_first, "第一段正好是 solid");
        assert_eq!(b.runs[1].first_vertex, image_first);
        assert_eq!(b.runs[1].end(), path_first, "第二段正好是 images");
        assert_eq!(b.runs[2].first_vertex, path_first);
        assert_eq!(b.runs[2].end() as usize, all.len());
        assert!(
            b.runs[0].texture.is_none() && b.runs[2].texture.is_none(),
            "矩形与多边形不换纹理绑定：{:?}",
            b.runs
        );
        assert_eq!(
            b.runs[1].texture.as_deref(),
            Some("t1"),
            "只有 images 段换绑定：{:?}",
            b.runs
        );
    }

    /// 相邻**同纹理**但混合模式不同的图像必须拆段。
    ///
    /// 只按纹理合批是不够的：合进一批后 base 只能给两段套同一个混合方程，症状是
    /// 「有一段的效果没生效」—— 而顶点数、缺失纹理数这些统计全都正常。
    #[test]
    fn same_texture_different_blend_splits_runs() {
        use zephyra_core::filter::BlendMode;
        let l = list_of(vec![
            image("t1", Rect::new(0.0, 0.0, 10.0, 10.0)),
            image_with_blend("t1", Rect::new(20.0, 0.0, 10.0, 10.0), BlendMode::Multiply),
        ]);
        let b = expand(&l, (100, 100), |_| true);
        let image_runs: Vec<&DrawRun> = b
            .runs
            .iter()
            .filter(|r| r.texture.is_some())
            .collect();
        assert_eq!(
            image_runs.len(),
            2,
            "同纹理但混合模式不同 ⇒ 两段：{:?}",
            image_runs
        );
        assert_eq!(image_runs[0].blend, BlendMode::default());
        assert_eq!(image_runs[1].blend, BlendMode::Multiply);
        assert!(
            b.runs.iter().all(|r| r.texture.as_deref() == Some("t1")),
            "两段都是同一张纹理"
        );
        // `batches` 是渲染器实际用来绑定纹理的那份，它也必须拆开 ——
        // 只拆 `runs` 而不拆 `batches`，渲染器仍会把两段当一批。
        assert_eq!(
            b.batches.len(),
            2,
            "batches 也必须按混合模式拆：{:?}",
            b.batches
        );
    }

    /// 零平移必须与「不传平移」**逐顶点相同**。
    #[test]
    fn zero_translate_matches_no_translate() {
        let l = list_of(vec![
            quad(Rect::new(1.0, 2.0, 10.0, 10.0), Color::rgb(1.0, 0.0, 0.0)),
            image("t1", Rect::new(3.0, 4.0, 10.0, 10.0)),
            path_cmd(
                &[[0.0, 40.0], [10.0, 40.0], [10.0, 50.0]],
                Color::rgb(0.0, 1.0, 0.0),
            ),
        ]);
        let a = expand(&l, (128, 128), |_| true);
        let b = expand_translated(&l, (128, 128), |_| true, Point::ZERO);
        assert_eq!(a.solid, b.solid, "零平移不该改变任何顶点");
        assert_eq!(a.images, b.images);
        assert_eq!(a.paths, b.paths);
        assert_eq!(a.runs, b.runs);
    }

    /// translate_moves_geometry_into_target_space —— 坐标刻意取 f32 可精确表示的值，于是可以断言**相等**。
    ///
    /// 尺寸 128、三角形 (16,16)-(48,16)-(48,80)、平移 (-16,-16) ⇒ 落到 (0,0)
    /// / (32,0) / (32,64)；除以 128 再乘 2 全是 0.25 / 0.5 / 1.0 这些二进制精确值。
    #[test]
    fn translate_moves_geometry_into_target_space() {
        let tri = [[16.0, 16.0], [48.0, 16.0], [48.0, 80.0]];
        let l = list_of(vec![path_cmd(&tri, Color::rgb(0.0, 1.0, 0.0))]);
        let b = expand_translated(&l, (128, 128), |_| true, Point::new(-16.0, -16.0));
        let got: Vec<[f32; 2]> = b.paths.iter().map(|v| v.position).collect();
        assert_eq!(
            got,
            vec![[-1.0, 1.0], [-0.5, 1.0], [-0.5, 0.0]],
            "平移后的 NDC：(0,0)→(-1,1)、(32,0)→(-0.5,1)、(32,64)→(-0.5,0)"
        );
        // 未平移的同一批几何必须**不同**，否则这条测试恒真。
        let plain = expand(&l, (128, 128), |_| true);
        let plain_pos: Vec<[f32; 2]> = plain.paths.iter().map(|v| v.position).collect();
        assert_ne!(
            got, plain_pos,
            "前提核对：不传平移的 NDC 必须与平移后不同（未平移首点是 (-0.75,0.75)）"
        );
    }

    /// 平移必须**同时**作用于裁剪矩形。
    ///
    /// 用「预先平移的等价列表」当判据，而不是手算 NDC —— 这样断言的是
    /// 「平移对所有几何一视同仁」，包括裁剪。
    #[test]
    fn translate_also_applies_to_clip_rect() {
        use zephyra_core::paint::Command::{PopClip, PushClip};
        let t = Point::new(-16.0, -16.0);
        let clip = Rect::new(0.0, 0.0, 64.0, 128.0);
        let body = Rect::new(0.0, 0.0, 128.0, 128.0);

        // A：屏幕坐标 + 平移；B：预先平移 + 不平移。两者必须完全等价。
        let a = expand_translated(
            &list_of(vec![
                PushClip(clip),
                quad(body, Color::rgb(1.0, 1.0, 1.0)),
                PopClip,
            ]),
            (128, 128),
            |_| true,
            t,
        );
        let b = expand(
            &list_of(vec![
                PushClip(shift_rect(clip, t)),
                quad(shift_rect(body, t), Color::rgb(1.0, 1.0, 1.0)),
                PopClip,
            ]),
            (128, 128),
            |_| true,
        );
        assert_eq!(
            a.solid, b.solid,
            "裁剪矩形没跟着平移的话，交集算在两个不同的空间里，结果是错的段"
        );
    }

    /// 裁剪确实**生效**（而不只是「平移了裁剪矩形」）。
    ///
    /// 宽度断言：128 宽的矩形被 64 宽的裁剪区切一半（再各内缩 `AA_BAND/2`）。
    /// 若裁剪矩形根本没参与，这个四边形会是满 128 宽。
    #[test]
    fn clipping_still_applies_after_translate() {
        use zephyra_core::paint::Command::{PopClip, PushClip};
        let b = expand_translated(
            &list_of(vec![
                PushClip(Rect::new(0.0, 0.0, 64.0, 128.0)),
                quad(Rect::new(0.0, 0.0, 128.0, 128.0), Color::rgb(1.0, 1.0, 1.0)),
                PopClip,
            ]),
            (128, 128),
            |_| true,
            Point::new(-16.0, -16.0),
        );
        assert_eq!(b.solid.len(), 4, "只应剩一个四边形");
        // 两个 x 顶点分别是裁剪区左右边界（各再内缩 AA_BAND/2）。
        let xs = [b.solid[0].position[0], b.solid[1].position[0]];
        let span_pixels = (xs[1] - xs[0]).abs() / 2.0 * 128.0;
        assert!(
            (span_pixels - (64.0 + AA_BAND)).abs() < 1e-3,
            "裁剪后跨度应约 64+AA_BAND 像素，实得 {span_pixels}"
        );
    }

    /// 平移必须作用到**图像**上。
///
/// ⚠ 这条是被变异测试逼出来的：图像走的是"先入队、后按纹理分批展开"这条完全不同的
/// 路径（`image_quads` → `pending`），前几条平移测试只覆盖了矩形/字形/多边形，删掉
/// 图像那一处的平移时**没有任何测试失败**。
///
/// 坐标取 f32 可精确表示的值（尺寸 128、矩形 32、平移 -16），于是可以断言相等。
#[test]
fn translate_also_applies_to_images() {
    let l = list_of(vec![image(
        "t1",
        Rect::new(16.0, 16.0, 32.0, 32.0),
    )]);
    let b = expand_translated(&l, (128, 128), |_| true, Point::new(-16.0, -16.0));
    let got: Vec<[f32; 2]> = b.images.iter().map(|v| v.position).collect();
    // 图像半径为 0 ⇒ `kind::IMAGE` 的 `pad` 为 0，不外扩，所以顶点正好在 (0,0)-(32,32)。
    assert_eq!(
        got,
        vec![[-1.0, 1.0], [-0.5, 1.0], [-0.5, 0.5], [-1.0, 0.5]],
        "平移后图像落在 (0,0)-(32,32)"
    );
    let plain = expand(&l, (128, 128), |_| true);
    assert_ne!(
        got,
        plain.images.iter().map(|v| v.position).collect::<Vec<_>>(),
        "前提核对：不传平移必须给出不同的 NDC"
    );
}

/// 平移必须作用到**字形**上。
#[test]
fn translate_also_applies_to_glyphs() {
    let l = list_of(vec![Command::Text {
        atlas: zephyra_core::paint::TextureRef::new("glyph-atlas"),
        glyphs: vec![zephyra_core::paint::GlyphQuad {
            uv: UvRect::new(0.1, 0.2, 0.05, 0.05),
            dst: Rect::new(16.0, 16.0, 32.0, 32.0),
            color_glyph: false,
        }]
        .into(),
        offset: Point::new(0.0, 0.0),
        color: Color::rgb(1.0, 1.0, 1.0),
        filter: zephyra_core::filter::FilterChain::none(),
        blend: zephyra_core::filter::BlendMode::default(),
    }]);
    let b = expand_translated(&l, (128, 128), |_| true, Point::new(-16.0, -16.0));
    let got: Vec<[f32; 2]> = b.solid.iter().map(|v| v.position).collect();
    // `kind::GLYPH` 不外扩（位图自带抗锯齿边缘），所以顶点正好在 (0,0)-(32,32)。
    assert_eq!(
        got,
        vec![[-1.0, 1.0], [-0.5, 1.0], [-0.5, 0.5], [-1.0, 0.5]],
        "平移后字形落在 (0,0)-(32,32)"
    );
}

/// `shift_rect` 只搬动原点，宽高原样。
    #[test]
    fn shift_rect_moves_origin_only() {
        let r = Rect::new(1.0, 2.0, 3.0, 4.0);
        assert_eq!(
            shift_rect(r, Point::new(10.0, 20.0)),
            Rect::new(11.0, 22.0, 3.0, 4.0),
            "宽高必须原样 —— 改了会让 SDF 的 half_size 与 uv 外插全错"
        );
        assert_eq!(
            shift_rect(r, Point::ZERO),
            r,
            "零平移是恒等（这条让调用方能无脑传 Point::ZERO）"
        );
    }

    /// 复用缓冲时 `runs` 不增长。
    ///
    /// 漏清 `run_tally` 的症状与漏清顶点段一模一样：段数每帧累加，合成器按
    /// `runs` 画就每帧多画一层，且所有几何统计仍然正常。
    #[test]
    fn runs_do_not_grow_with_reused_scratch() {
        let l = list_of(vec![
            quad(Rect::new(0.0, 0.0, 10.0, 10.0), Color::rgb(1.0, 0.0, 0.0)),
            image("t1", Rect::new(0.0, 20.0, 10.0, 10.0)),
            path_cmd(
                &[[0.0, 40.0], [10.0, 40.0], [10.0, 50.0]],
                Color::rgb(0.0, 1.0, 0.0),
            ),
        ]);
        let mut s = Scratch::new();
        let mut report = expand_translated_into(&l, (100, 100), &|_| true, Point::ZERO, &mut s);
        let first = s.runs.len();
        assert!(first > 0, "前提：这批命令产出若干段");
        for _ in 0..5 {
            report = expand_translated_into(&l, (100, 100), &|_| true, Point::ZERO, &mut s);
            assert_eq!(
                s.runs.len(),
                first,
                "runs 逐帧增长 ⇒ run_tally / runs 忘了 clear"
            );
        }
        assert_eq!(report.quads, 2, "几何统计也不该变（1 矩形 + 1 图像；Path 计入 path_triangles）");
    }


/// `unimplemented_blends` 只数**还不能表达**的混合模式 —— E4.1 的判据。
    ///
    /// 逐个跑**全部 29 个**而不是抽样：判据是"整张表"，抽样只能证明抽样的那几个，
    /// 而漏掉一个模式的后果正是"那个模式静默失效"。
    #[test]
    fn unimplemented_blends_counts_only_modes_without_coeffs() {
        use zephyra_core::filter::BlendMode;
        // 按 `filter.rs` 里枚举的**声明顺序**逐项列出 —— 顺序本身就是契约的一部分
        // （`LAST_COEFF_MODE` / `LAST_SEPARABLE_MODE` 都是靠 `<=` 定义的）。
        let all: [BlendMode; 29] = [
            BlendMode::Clear,        //  0
            BlendMode::Src,          //  1
            BlendMode::Dst,          //  2
            BlendMode::SrcOver,      //  3  ← 默认
            BlendMode::DstOver,      //  4
            BlendMode::SrcIn,        //  5
            BlendMode::DstIn,        //  6
            BlendMode::SrcOut,       //  7
            BlendMode::DstOut,       //  8
            BlendMode::SrcATop,      //  9
            BlendMode::DstATop,      // 10
            BlendMode::Xor,          // 11
            BlendMode::Plus,         // 12
            BlendMode::Modulate,     // 13
            BlendMode::Screen,       // 14  ← 最后一个系数可表达的
            BlendMode::Overlay,      // 15
            BlendMode::Darken,       // 16
            BlendMode::Lighten,      // 17
            BlendMode::ColorDodge,   // 18
            BlendMode::ColorBurn,    // 19
            BlendMode::HardLight,    // 20
            BlendMode::SoftLight,    // 21
            BlendMode::Difference,   // 22
            BlendMode::Exclusion,    // 23
            BlendMode::Multiply,     // 24  ← 最后一个可分离的
            BlendMode::Hue,          // 25
            BlendMode::Saturation,   // 26
            BlendMode::Color,        // 27
            BlendMode::Luminosity,   // 28
        ];
        assert_eq!(all.len(), BlendMode::COUNT, "枚举个数与 COUNT 不符");

        for (i, &m) in all.iter().enumerate() {
            let b = expand(
                &list_of(vec![Command::Quad {
                    rect: Rect::new(0.0, 0.0, 10.0, 10.0),
                    radius: zephyra_core::core::CornerRadius { radius: 0.0 },
                    fill: Color::rgb(1.0, 1.0, 1.0),
                    stroke: None,
                    filter: zephyra_core::filter::FilterChain::none(),
                    blend: m,
                }]),
                (100, 100),
                |_| true,
            );
            let expect = if m.as_coeff().is_some() { 0 } else { 1 };
            assert_eq!(
                b.report.unimplemented_blends, expect,
                "{m:?}（下标 {i}）：能系数表达 ⇒ 不该计入；不能 ⇒ 该计入"
            );
        }
    }

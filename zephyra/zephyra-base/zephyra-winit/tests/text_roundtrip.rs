//! 文字渲染的 CPU 端闭环验证：**栅格化 → 打包 → uv → 顶点 → 采样**。
//!
//! # 为什么要有这个测试
//!
//! GPU 侧在本机无法验证（D3D12 资源创建整体不可用，见
//! `zephyra-winit-d3d12/tests/minimal.rs`；D3D11 视口不生效，见
//! `tests/d3d11_draw.rs`）。但文字渲染这条链**绝大部分不在 GPU 上**：
//!
//! ```text
//!  ab_glyph 轮廓            CPU，可测
//!    ↓ rasterize()
//!  GlyphAtlas（RGBA 位图）  CPU，可测
//!    ↓ uv 归一化
//!  GlyphQuad{uv, dst}      CPU，可测
//!    ↓ quad::expand
//!  Vertex{position, uv}    CPU，可测
//!    ↓ ★ 片元着色器做的事
//!  采样图集                 GPU —— 这里用 CPU 代替
//! ```
//!
//! 最后一个箭头是唯一的 GPU 环节。而它做的事是「**按 uv 去位图里取值**」——
//! 这一步完全可以在 CPU 上重演一遍。
//!
//! # 这个测试因此能证明什么
//!
//! 如果采样出来的像素**确实有墨**，那么「字画出来是空白」这个症状就不可能来自
//! 上面五步中的任何一步 —— 剩下的只有 GPU 环节（纹理格式、采样器、混合）。
//! 这正是本项目需要的判据：把能测的部分测干净，让 GPU 侧的故障有 narrower 的嫌疑范围。
//!
//! # 反过来，这个测试**不能**证明什么
//!
//! 不能证明 GPU 侧的采样器设置正确、不能证明混合正确、不能证明图集纹理真的上传成功。
//! 它证明的是「几何与数据是对的」。

#![cfg(windows)]
// 本测试要建真实 D3D11 设备，非 Windows 上没有这条路径。
// 宿主 crate 整个是 `#![cfg(windows)]`，而集成测试是**独立目标**、不受那个门禁保护
// —— 不在这里补一道，`cargo test --workspace` 在 Linux 上就编不过（实测 2026-10）。

use zephyra_core::core::Point;
use zephyra_core::paint::{Command, DisplayList, Layer, SortKey};
use zephyra_core::text::atlas::ATLAS_SIZE;
use zephyra_core::text::{TextRender, TextStack, TextStyle};

use zephyra_winit::quad::{self, Vertex};
use zephyra_winit::text::{from_system_font, AbGlyphSource};

/// 目标尺寸（DIP）。
const TARGET: (u32, u32) = (400, 200);

/// 字号（DIP）。
const SIZE: f32 = 32.0;

/// 建一个真实的字体栈；没有系统字体就跳过（而不是假装通过）。
fn stack() -> Option<TextStack<AbGlyphSource>> {
    let (mut s, _desc) = from_system_font();
    // `disabled` 的栈没有字形可用 —— 那时后面的断言毫无意义，直接跳过。
    if s.atlas_stats().glyphs == 0 && s.atlas().pixels().iter().all(|&v| v == 0) {
        // 还没排版过，图集当然是空的。用一次探测排版来判断是否有真字体。
        let shaped = zephyra_core::text::TextEngine::shape(
            &mut s,
            "Hi",
            &TextStyle {
                size: SIZE,
                ..Default::default()
            },
            None,
        );
        if shaped.lines.iter().all(|l| l.glyphs.is_empty()) {
            eprintln!("跳过：这台机器上没有可用的系统字体");
            return None;
        }
    }
    Some(s)
}

/// 按 uv 在图集里取覆盖率（**重演片元着色器的 `g_tex.Sample(...).a`**）。
///
/// 采样取 uv 对应的**像素中心**：`floor(uv * ATLAS_SIZE)`。
fn sample_atlas(atlas: &[u8], u: f32, v: f32) -> u8 {
    let x = ((u * ATLAS_SIZE as f32) as usize).min(ATLAS_SIZE as usize - 1);
    let y = ((v * ATLAS_SIZE as f32) as usize).min(ATLAS_SIZE as usize - 1);
    atlas[(y * ATLAS_SIZE as usize + x) * 4 + 3]
}

/// 把 `DisplayList` 展开成顶点流。
fn expand(list: &DisplayList) -> quad::QuadBatch {
    quad::expand(list, TARGET, |_| true)
}

#[test]
fn glyphs_line_up_from_rasterization_through_sampling() {
    let Some(mut stack) = stack() else { return };

    let style = TextStyle {
        size: SIZE,
        ..Default::default()
    };
    let shaped = zephyra_core::text::TextEngine::shape(&mut stack, "Hg", &style, None);

    // ---- 1. 排版阶段要有东西 ----
    assert!(
        !shaped.lines.is_empty() && !shaped.lines[0].glyphs.is_empty(),
        "排版 `\"Hg\"` 后一个字形都没有 —— 字形来源不可用"
    );

    // ---- 2. 降级成 GlyphQuad ----
    let (_handle, quads) = stack
        .glyph_quads(&shaped, Point::new(10.0, 40.0), &style)
        .expect("有字体时 glyph_quads 不该返回 None");
    assert_eq!(
        quads.len(),
        shaped.lines[0].glyphs.len(),
        "\"Hg\" 是两个非空白字形，图集里应当各有一个四边形"
    );

    // ---- 3. uv 全部落在 0..1，且面积合理 ----
    let atlas_pixels = stack.atlas().pixels();
    for (i, q) in quads.iter().enumerate() {
        assert!(
            q.uv.x >= 0.0 && q.uv.y >= 0.0,
            "第 {i} 个字形的 uv 原点 ({}, {}) 为负 —— 归一化时符号搞反了",
            q.uv.x,
            q.uv.y
        );
        assert!(
            q.uv.right() <= 1.0 && q.uv.bottom() <= 1.0,
            "第 {i} 个字形的 uv 右下 ({}, {}) 越过了 1.0 —— 说明它指向图集之外",
            q.uv.right(),
            q.uv.bottom()
        );
        assert!(
            q.uv.width > 0.0 && q.uv.height > 0.0,
            "第 {i} 个字形的 uv 面积为零"
        );

        // uv 反推成像素，应当落在图集内，且尺寸与 dst 一致（uv 与 dst 是同一个位图）。
        let px_w = q.uv.width * ATLAS_SIZE as f32;
        let px_h = q.uv.height * ATLAS_SIZE as f32;
        assert!(
            (px_w - q.dst.width).abs() < 0.5,
            "第 {i} 个字形：uv 反推宽度 {px_w} 与 dst 宽度 {} 不符 —— \
             两者本该是同一块位图",
            q.dst.width
        );
        assert!(
            (px_h - q.dst.height).abs() < 0.5,
            "第 {i} 个字形：uv 反推高度 {px_h} 与 dst 高度 {} 不符",
            q.dst.height
        );
    }

    // ---- 4. ★ 闭环：按 uv 去图集采样，必须有墨 ----
    //
    // 这一步是整个测试的核心。它重演了片元着色器 `kind == GLYPH` 分支做的事：
    // `g_tex.Sample(g_smp, uv).a`。若采样恒为 0，那么字在屏幕上必然是空白 ——
    // 而此时几何与 uv 都已验证正确，嫌疑范围就只剩 GPU 环节。
    let mut inked = 0;
    for (i, q) in quads.iter().enumerate() {
        // 在字形内部取 3×3 个采样点（避开边缘的抗锯齿带）。
        let (ux0, uy0) = (q.uv.x, q.uv.y);
        let (ux1, uy1) = (q.uv.right(), q.uv.bottom());
        let mut max_cov = 0u8;
        for sy in 1..4 {
            for sx in 1..4 {
                let u = ux0 + (ux1 - ux0) * (sx as f32 + 0.5) / 4.0;
                let v = uy0 + (uy1 - uy0) * (sy as f32 + 0.5) / 4.0;
                max_cov = max_cov.max(sample_atlas(atlas_pixels, u, v));
            }
        }
        if max_cov > 0 {
            inked += 1;
        } else {
            eprintln!(
                "[t] 第 {i} 个字形采样全为 0：uv=({:.4},{:.4})-({:.4},{:.4}) → 像素 ({:.0},{:.0})",
                ux0,
                uy0,
                ux1,
                uy1,
                ux0 * ATLAS_SIZE as f32,
                uy0 * ATLAS_SIZE as f32
            );
        }
    }
    assert_eq!(
        inked,
        quads.len(),
        "{inked}/{} 个字形按 uv 采样全透明 —— 栅格化/打包/uv 这条链断了。\
         注意这是**图集本身**没有墨，不是 GPU 采样问题（这里没碰 GPU）。",
        quads.len()
    );

    // ---- 5. 走一遍 quad::expand，确认顶点流里的 kind 与 uv 一致 ----
    let mut list = DisplayList::new();
    list.push(
        SortKey {
            layer: Layer::Content,
            z: 0,
            order: 0,
        },
        Command::Text {
            atlas: _handle.clone(),
            glyphs: quads.clone().into(),
            // 上面 `glyph_quads` 用的原点是 (10, 40)，所以这里的 `quads` **已经**
            // 是绝对坐标 —— `offset` 必须是零，否则会平移两次。
            //
            // 绘制趟走的是另一条路（`origin` = (0,0) 缓存 + `offset` 带位置），
            // 见 `Command::Text::glyphs`。这里不跟随：这条测试的断言建立在
            // 「四边形就是最终位置」上，改了形状就得连带改断言，而那不是
            // 本次改动要验的东西。
            offset: Point::new(0.0, 0.0),
            color: zephyra_core::core::Color::rgba(1.0, 1.0, 1.0, 1.0),
            filter: zephyra_core::filter::FilterChain::none(),
            blend: zephyra_core::filter::BlendMode::default(),
        },
    );
    let batch = expand(&list);
    assert_eq!(
        batch.solid.len(),
        quads.len() * 4,
        "{} 个字形应当产生 {} 个顶点，实际 {}",
        quads.len(),
        quads.len() * 4,
        batch.solid.len()
    );

    for (i, chunk) in batch.solid.chunks_exact(4).enumerate() {
        let [a, b, c, d] = chunk else { unreachable!() };
        // 四个顶点必须构成一个**轴对齐**的矩形 —— 否则索引化三角形画出来是斜的。
        assert!(
            (a.position[0] - d.position[0]).abs() < 1e-5
                && (a.position[1] - b.position[1]).abs() < 1e-5,
            "第 {i} 个字形的顶点不是轴对齐矩形：{:?} {:?} {:?} {:?}",
            a.position,
            b.position,
            c.position,
            d.position
        );
        // kind 必须是 GLYPH，否则片元着色器走错分支。
        assert!(
            (a.kind - quad::kind::GLYPH).abs() < f32::EPSILON,
            "第 {i} 个字形的 kind = {}，应为 GLYPH = {}",
            a.kind,
            quad::kind::GLYPH
        );
        // uv 必须与 GlyphQuad 的一致 —— `expand` 不得改写 uv。
        assert!(
            (a.uv[0] - quads[i].uv.x).abs() < 1e-6,
            "第 {i} 个字形的 uv.x 被 expand 改写了：{} → {}",
            quads[i].uv.x,
            a.uv[0]
        );
        // 顶点位置必须落在 NDC 内。
        for v in chunk {
            assert!(
                (-1.0..=1.0).contains(&v.position[0]) && (-1.0..=1.0).contains(&v.position[1]),
                "第 {i} 个字形的顶点位置 {:?} 超出 NDC [-1,1] —— 目标尺寸与文本原点不一致",
                v.position
            );
        }
    }
}

/// 空白字符（空格）**不该**产生四边形。
///
/// 这是个真实的坑：步进非零但墨迹为零的字形若被当成有尺寸处理，会在屏幕上画出
/// 一个矩形色块 —— 表现为"每个词后面跟着一块脏东西"。
#[test]
fn space_produces_no_quads() {
    let Some(mut stack) = stack() else { return };
    let style = TextStyle {
        size: SIZE,
        ..Default::default()
    };
    // `"a a"`：两个 `a` 之间有一个空格。总共应得 2 个四边形，不是 3 个。
    let shaped = zephyra_core::text::TextEngine::shape(&mut stack, "a a", &style, None);
    let (_h, quads) = stack
        .glyph_quads(&shaped, Point::new(0.0, 50.0), &style)
        .expect("有字体时不该是 None");
    assert_eq!(
        quads.len(),
        2,
        "\"a a\" 应当只有 2 个四边形（两个 a），实际 {} 个 —— 空格被当成了可见字形",
        quads.len()
    );
}

/// 图集用量必须单调增长，且不会溢出。
#[test]
fn atlas_usage_is_sane_and_does_not_overflow() {
    let Some(mut stack) = stack() else { return };
    let style = TextStyle {
        size: SIZE,
        ..Default::default()
    };
    let shaped = zephyra_core::text::TextEngine::shape(&mut stack, "Hello, Zephyra!", &style, None);
    let _ = stack.glyph_quads(&shaped, Point::new(0.0, 50.0), &style);

    let stats = stack.atlas_stats();
    assert!(
        stats.usage() <= 1.0,
        "图集用量 {} 超过 100% —— 打包器写越界了",
        stats.usage()
    );
    assert!(
        stats.glyphs >= 8,
        "\"Hello, Zephyra!\" 有 8 个不同的非空白字符，图集里只记了 {} 个条目",
        stats.glyphs
    );
    // 同一段文本排版两次不应产生新条目（缓存命中）。
    let before = stack.atlas_stats().glyphs;
    let _ = stack.glyph_quads(&shaped, Point::new(0.0, 50.0), &style);
    assert_eq!(
        stack.atlas_stats().glyphs,
        before,
        "第二次排版同一段文本新增了图集条目 —— 缓存没命中"
    );
}

/// `Vertex` 的 uv 必须原样传到顶点流，不被归一化或夹取。
///
/// 这一条看似显然，但它是"图集采样到错误像素"最常见的成因：某个中间层觉得
/// "uv 应该在 0..1"于是又除了一次 `ATLAS_SIZE`。
#[test]
fn vertex_uv_matches_atlas_entry_verbatim() {
    let Some(mut stack) = stack() else { return };
    let style = TextStyle {
        size: SIZE,
        ..Default::default()
    };
    let shaped = zephyra_core::text::TextEngine::shape(&mut stack, "M", &style, None);
    let (_h, quads) = stack
        .glyph_quads(&shaped, Point::new(0.0, 50.0), &style)
        .expect("有字体时不该是 None");
    let q = quads.first().expect("\"M\" 应当有一个四边形");

    // 反推图集里的像素矩形，与图集自己记的条目对照。
    let pixels = stack.atlas().pixels();
    let px_x = (q.uv.x * ATLAS_SIZE as f32).round() as usize;
    let px_y = (q.uv.y * ATLAS_SIZE as f32).round() as usize;

    // 该像素上应该有墨（因为它是字形左上角附近，而 M 的左上角有笔画）。
    let any_ink = (0..8).any(|dy| {
        (0..8).any(|dx| {
            let x = px_x + dx;
            let y = px_y + dy;
            x < ATLAS_SIZE as usize
                && y < ATLAS_SIZE as usize
                && pixels[(y * ATLAS_SIZE as usize + x) * 4 + 3] > 0
        })
    });
    assert!(
        any_ink,
        "按 uv 反推的图集位置 ({px_x}, {px_y}) 附近 8×8 内没有墨 —— uv 与图集对不上"
    );
}

/// 目标尺寸之外不该有字形顶点。
///
/// 若文本原点或 dst 计算有误，这里会先炸 —— 它比"屏幕上字跑到窗口外"好查得多。
#[test]
fn vertex_position_matches_dst_rect() {
    let Some(mut stack) = stack() else { return };
    let style = TextStyle {
        size: SIZE,
        ..Default::default()
    };
    let origin = Point::new(10.0, 60.0);
    let shaped = zephyra_core::text::TextEngine::shape(&mut stack, "H", &style, None);
    let (_h, quads) = stack
        .glyph_quads(&shaped, origin, &style)
        .expect("有字体时不该是 None");
    let q = quads[0];

    // dst 左上角 = (基线 x + offset.x, 基线 y - offset.y - 高)
    // DIP → NDC： nx = x / w * 2 - 1；ny = 1 - y / h * 2
    let nx = |x: f32| x / TARGET.0 as f32 * 2.0 - 1.0;
    let ny = |y: f32| 1.0 - y / TARGET.1 as f32 * 2.0;
    let expect = Vertex::from_uv_rect(
        q.dst,
        (TARGET.0 as f32, TARGET.1 as f32),
        [1.0; 4],
        q.uv,
        quad::kind::GLYPH,
    );
    // 只比对左上角与右下角的 NDC —— 中间两个顶点的顺序由 `quad` 自己保证。
    let e0 = expect[0].position;
    let e2 = expect[2].position;
    assert!(
        (e0[0] - nx(q.dst.x)).abs() < 1e-5,
        "左上角 x：顶点算得 {}，按 dst.x={} 应为 {}",
        e0[0],
        q.dst.x,
        nx(q.dst.x)
    );
    assert!(
        (e0[1] - ny(q.dst.y)).abs() < 1e-5,
        "左上角 y：顶点算得 {}，按 dst.y={} 应为 {}（注意 y 轴翻转）",
        e0[1],
        q.dst.y,
        ny(q.dst.y)
    );
    assert!(
        (e2[1] - ny(q.dst.y + q.dst.height)).abs() < 1e-5,
        "右下角 y：顶点算得 {}，应为 {}",
        e2[1],
        ny(q.dst.y + q.dst.height)
    );
}

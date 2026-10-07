//! 把**字形图集**本身导成 BMP —— 用来人眼确认「字形栅格化对不对」。
//!
//! # 为什么需要它
//!
//! 文字渲染出问题时，「画面是空的」这条信息**几乎不含信息量**：可能是图集全空、
//! 可能��顶点全错、可能是 GPU 没画。而图集是一张**纯 CPU 位图**，它可以直接看。
//!
//! 有了它，排查顺序就固定下来了：
//!
//! ```text
//! 1. 图集里有正确的字形吗？   ← 本测试
//! 2. 顶点流引用了正确的 uv 吗？
//! 3. GPU 画出来了吗？
//! ```
//!
//! # 它在本项目里已经抓到过一次真 bug
//!
//! `offset` 被减了三次（`ab_glyph` 的回调坐标本就相对 `px_bounds.min`，实现里又减了
//! 两次），于是大写字母的每个像素都被越界判断跳过 —— **图集全空**。而那与「GPU 画不出
//! 东西」在屏幕上完全无法区分。
//!
//! 详见 `zephyra/src/text/atlas_tests.rs` 里用假字形来源钉死的契约测试。
//!
//! # 跑法
//!
//! ```powershell
//! cargo test -p zephyra-winit --test glyph_atlas_bmp -- --ignored --nocapture
//! ```
//!
//! 默认 `#[ignore]`：它要读系统字体、并且会写文件，不适合进无差别的 `cargo test`。

use zephyra_core::core::Point;
use zephyra_core::text::atlas::ATLAS_SIZE;
use zephyra_core::text::{TextEngine, TextRender, TextStyle};

use zephyra_winit::text::from_system_font;

/// 要导出的文本。用 demo 里真实出现过的内容，而不是 `"Hello"` —— 真实内容里
/// 有 CJK、有标点、有空格，三类在图集上的行为都不同。
const SAMPLE: &str = "ZephyraUI 渲染 DisplayList";

/// 字号。32 是 demo 用的尺寸，导出后放大 3 倍还能看清笔画。
const SIZE: f32 = 32.0;

/// 导出图集里有多大一块。字号越大，块就越要放大 —— 只截一小块会只看到半个字。
const REGION: u32 = 360;

/// 放大倍数。1:1 看不清字形边缘，而边缘正是最容易出错的地方。
const SCALE: u32 = 3;

#[test]
#[ignore = "需要系统字体、且会写文件；默认不进无差别测试。手动跑：--ignored --nocapture"]
fn 字形图集里有正确的字形() {
    let (mut stack, note) = from_system_font();
    println!("{note}");

    let style = TextStyle {
        size: SIZE,
        ..Default::default()
    };
    let shaped = TextEngine::shape(&mut stack, SAMPLE, &style, None);
    assert!(
        !shaped.lines.is_empty() && !shaped.lines[0].glyphs.is_empty(),
        "排版 `{SAMPLE}` 后一个字形都没有 —— 这台机器上没有可用字体"
    );

    let (_handle, quads) = TextRender::glyph_quads(&mut stack, &shaped, Point::new(0.0, 40.0), &style)
        .expect("有字体时 glyph_quads 不该返回 None");
    assert!(!quads.is_empty(), "没有产出任何字形四边形");

    let atlas = stack.atlas();
    let stats = stack.atlas_stats();
    println!(
        "字形 {} 个，四边形 {} 个，用量 {:.1}%",
        stats.glyphs,
        quads.len(),
        stats.usage() * 100.0
    );

    // ---- 判据 1：图集里必须有墨 ----
    //
    // 这是最重要的一条：图集全空时，屏幕上是"文字完全不出现"，而那与 GPU 故障
    // 无法区分。这里直接把「有没有墨」摆出来。
    let inked = atlas.pixels().chunks_exact(4).filter(|p| p[3] > 0).count();
    assert!(
        inked > 0,
        "图集里一个非透明像素都没有 —— 栅格化或打包这一步就断了，**别去查 GPU**"
    );
    println!("图集中有墨的像素：{inked}");

    // ---- 判据 2：uv 必须落在图集内且非零 ----
    for (i, q) in quads.iter().enumerate() {
        assert!(
            q.uv.x >= 0.0 && q.uv.y >= 0.0 && q.uv.right() <= 1.0 && q.uv.bottom() <= 1.0,
            "第 {i} 个字形的 uv ({}, {})-({}, {}) 越界",
            q.uv.x, q.uv.y, q.uv.right(), q.uv.bottom()
        );
    }

    // ---- 导出给人看 ----
    let region = REGION.min(ATLAS_SIZE);
    let out_w = region * SCALE;
    let out_h = region * SCALE;
    let mut out = vec![0u8; (out_w * out_h * 4) as usize];
    for y in 0..out_h {
        for x in 0..out_w {
            let sx = x / SCALE;
            let sy = y / SCALE;
            let si = ((sy * ATLAS_SIZE + sx) * 4) as usize;
            let di = ((y * out_w + x) * 4) as usize;
            // 用**灰度**输出（覆盖率同时写进四个通道，取一个即可）。把背景涂成
            // 深红，这样"没有墨"的地方一眼能看出是空洞而不是没导出。
            let v = atlas.pixels()[si];
            out[di] = if v > 0 { v } else { 40 };
            out[di + 1] = v;
            out[di + 2] = v;
            out[di + 3] = 255;
        }
    }

    let path = std::path::Path::new(&std::env::temp_dir())
        .join("zephyra-atlas-preview.bmp")
        .display()
        .to_string();
    zephyra_winit::write_bmp(
        &path,
        &zephyra_winit::surface::PixelRead {
            width: out_w,
            height: out_h,
            row_pitch: out_w * 4,
            declared_width: out_w,
            declared_height: out_h,
            data: out,
        },
        // 图集里存的是**线性**覆盖率，不是线性光。写给人看时**不做** sRGB 编码 ——
        // 那会把抗锯齿的灰阶拉亮，让笔画看起来比实际更粗。
        false,
    )
    .expect("写 BMP 失败");
    println!("已写出 {path}（{}×{}，放大 {SCALE}×）", out_w, out_h);
}
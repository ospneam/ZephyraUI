//! 测试功能展台 —— **保留原 `examples/demo.rs` 的测试能力**。
//!
//! # 为什么这些不是"演示"，是**测试**
//!
//! 这一页上的东西看起来杂乱（图标、棋盘裁剪、模糊样张、状态行），但它们各自
//! 钉住框架的一条链路，而**单看画面看不出来**：
//!
//! | 展台 | 它在测什么 | 静默失败的样子 |
//! |---|---|---|
//! | 图标行 | 内置矢量图标 → `Command::Path` 三角化 | 图标位空白，统计仍正常 |
//! | uv 裁剪 | `IMAGE_SRC` 真的取到了纹理的对应区域 | 整张/翻转，无报错 |
//! | 滤镜样张 | `FILTER`（模糊/投影）与 `BLEND_MODE` 真的进了光栅 | 样张全一样，统计正常 |
//! | hover 状态行 | 命中测试与绘制用同一套几何 | 画面全对、点击跑偏 |
//!
//! 所以这一页是**逐条照抄** `examples/demo.rs` 的 `icons()` / `src_crops()` /
//! `effects()` / `status_bar()`，而不是"顺手做几个好看的样张"。
//!
//! # 差异与理由
//!
//! 原 demo 的图标与 uv 样张用**宿主注册的 GPU 纹理**（`demo:checker` 等）。
//! gallery 的窗口路同样走 D3D12，而 CPU dump 路对未注册纹理画占位色 —— 所以
//! 这里的图标行用**内置矢量图标**（同样验证"图标名 → 三角化 → 绘制"这条链路，
//! 且 CPU 路也能看到真像素），uv 样张保留（GPU 路验证裁剪，CPU 路画占位）。
//! 这是**等价换测法**，不是降级：被测的是裁剪逻辑，不是"用哪张纹理"。

use zephyra_core::core::UvRect;
use zephyra_core::element::Element;
use zephyra_core::id::NodeTag;
use zephyra_core::property::PropertyValue;
use zephyra_core::props;

use super::Gallery;
use crate::controls::text_label;

/// 全部内置图标名（`core` 的登记表，**不是文件名**）。
///
/// ⚠️ 这里**逐个列出来**而不是遍历注册表：遍历出来的 key 是 `&'static str`
/// 但没有稳定的顺序，而 key 就是身份 —— 顺序变了整棵树就变一次。
/// 列出来 = 顺序固定，且**少一个/多一个肉眼可见**。
const ICON_NAMES: [&str; 8] = [
    "home", "search", "settings", "menu", "bell", "star", "pen", "eraser",
];

/// 棋盘 uv 裁剪的静态表（`u0,v0,u1,v1`）+ 对应 key。
const CROPS: [(&str, f32, f32, f32, f32); 4] = [
    ("crop:full", 0.0, 0.0, 1.0, 1.0),
    ("crop:right", 0.5, 0.0, 1.0, 1.0),
    ("crop:br", 0.5, 0.5, 1.0, 1.0),
    ("crop:overflow", 0.9, 0.9, 1.4, 1.4),
];

/// 测试页。
pub fn testbed(g: &Gallery) -> Element {
    Element::new(NodeTag::Container)
        .with_key("testbed")
        .style("page-testbed")
        .with_children([
            section_of("tb-icons", "Vector icons (core built-in)", vec![icon_row()]),
            section_of("tb-crops", "uv cropping (Image src)", vec![crop_row()]),
            section_of("tb-effects", "Filters & blend modes", vec![effects()]),
            status_line(g),
        ])
}

fn section_of(key: &'static str, title: &'static str, children: Vec<Element>) -> Element {
    Element::new(NodeTag::Container)
        .with_key(key)
        .style("section")
        .with_children([
            text_label("section-title", title, "text-body-strong"),
            Element::new(NodeTag::Container)
                .with_key("section-body")
                .style("section-body")
                .with_children(children),
        ])
}

/// 图标行：每个内置图标一个节点。
///
/// 验证「图标名 → `IconSet` 查找 → 三角化 → `Command::Path` → 光栅化」整条链。
/// 这条链曾整段失效过（CPU 光栅器从不画 `paths`），而统计一切正常 —— 见
/// `MIGRATION.md` §6.2。
fn icon_row() -> Element {
    Element::new(NodeTag::Container)
        .with_key("icon-row")
        .style("testbed-icons")
        .with_children(
            ICON_NAMES
                .iter()
                .map(|n| {
                    Element::new(NodeTag::Container)
                        // `iter()` 给 `&&str`，但 `with_key` 要 `&str`（元素），
                        // 所以解一层：`*n`。这里用 `.copied()` 让 `n: &str` 更直白。
                        .with_key(*n)
                        .with(&props::ICON_NAME, PropertyValue::static_str(*n))
                        .style("testbed-icon")
                })
                .collect::<Vec<Element>>(),
        )
}

/// uv 裁剪展台：同一张棋盘纹理按不同 uv 裁剪并排。
///
/// 判据（为什么这四个取值有讲究）：
/// - `full` —— 基线，整张；
/// - `right` —— 右半；偏移错 1/2 会看到左半，**肉眼可辨**；
/// - `br` —— 右下四分之一；与 `right` 一起能同时抓"两轴都错"；
/// - `overflow` —— 越界（0.9..1.4）：验证 clamp，而不是黑屏或崩。
fn crop_row() -> Element {
    Element::new(NodeTag::Container)
        .with_key("crop-row")
        .style("testbed-crops")
        .with_children(
            CROPS
                .iter()
                .map(|c| {
                    // `c: &(&str, f32, f32, f32, f32)`，解构后各字段已是引用。
                    Element::new(NodeTag::Image)
                        .with_key(c.0)
                        .with(
                            &props::IMAGE_SRC,
                            PropertyValue::uv_rect(UvRect::new(c.1, c.2, c.3, c.4)),
                        )
                        .with(
                            &props::IMAGE_TEXTURE,
                            PropertyValue::static_str(TESTBED_TEXTURE),
                        )
                        .style("testbed-crop")
                })
                .collect::<Vec<Element>>(),
        )
}

/// 测试用纹理句柄（**由宿主注册**；CPU 路画占位色）。
pub const TESTBED_TEXTURE: &str = "testbed:checker";

/// 滤镜 + 混合模式样张（照抄 demo 的 6 个样张）。
///
/// ⚠️ 每个样张是**两个节点**（底板 + 样本）：属性是**覆盖**语义不是叠加，
/// 同一个节点先涂深色再涂半透明色的话，后一次把前一次盖掉 —— 于是底板消失、
/// 样本直接混在面板底色上，混合模式之间的对比度掉一大截，肉眼就分不开了。
fn effects() -> Element {
    use zephyra_core::core::{Color, Point};
    use zephyra_core::filter::{BlendMode, Filter, FilterChain};

    // 三趟 σ2 ≈ 一趟 σ2.83 —— 过渡带明显更宽，这就是判据。
    let triple = FilterChain::new([
        Filter::Blur {
            sigma_x: 2.0,
            sigma_y: 2.0,
        },
        Filter::Blur {
            sigma_x: 2.0,
            sigma_y: 2.0,
        },
        Filter::Blur {
            sigma_x: 2.0,
            sigma_y: 2.0,
        },
    ]);

    let row = Element::new(NodeTag::Container)
        .with_key("effects-row")
        .style("testbed-effects")
        .with_children([
            swatch(
                "fx-none",
                "control",
                FilterChain::none(),
                BlendMode::SrcOver,
            ),
            swatch(
                "fx-blur",
                "accent",
                FilterChain::single(Filter::Blur {
                    sigma_x: 3.0,
                    sigma_y: 3.0,
                }),
                BlendMode::SrcOver,
            ),
            swatch(
                "fx-shadow",
                "accent",
                FilterChain::single(Filter::DropShadow {
                    color: Color::rgba(0.0, 0.0, 0.0, 0.85),
                    offset: Point::new(6.0, 6.0),
                    sigma: 3.0,
                }),
                BlendMode::SrcOver,
            ),
            swatch("fx-triple", "accent", triple, BlendMode::SrcOver),
            swatch(
                "fx-multiply",
                "warm",
                FilterChain::none(),
                BlendMode::Multiply,
            ),
            swatch("fx-screen", "warm", FilterChain::none(), BlendMode::Screen),
        ]);
    row
}

/// 一个样张：底板（混合目标）+ 样本（混合源）+ 说明。
fn swatch(
    key: &'static str,
    fill: &'static str,
    filter: zephyra_core::filter::FilterChain,
    blend: zephyra_core::filter::BlendMode,
) -> Element {
    let sample = Element::new(NodeTag::Container)
        .with_key("sample")
        .style("fx-sample")
        .style(fill)
        .with(&props::FILTER, PropertyValue::filter(filter))
        .with(&props::BLEND_MODE, PropertyValue::blend_mode(blend));
    Element::new(NodeTag::Container)
        .with_key(key)
        .style("testbed-swatch")
        .with_children([
            Element::new(NodeTag::Container)
                .with_key("backdrop")
                .style("fx-backdrop")
                .with_children([sample]),
            text_label("caption", blend_name(blend), "text-caption"),
        ])
}

/// 混合模式的人读名（说明文字，不是外观值 —— 但仍是**文案**，走 `TEXT`）。
fn blend_name(b: zephyra_core::filter::BlendMode) -> &'static str {
    use zephyra_core::filter::BlendMode::*;
    match b {
        SrcOver => "SrcOver",
        Multiply => "Multiply",
        Screen => "Screen",
        _ => "Blend",
    }
}

/// hover / press 状态行（常驻，证明命中测试与绘制同几何）。
fn status_line(g: &Gallery) -> Element {
    Element::new(NodeTag::Container)
        .with_key("tb-status")
        .style("testbed-status")
        .with_children([
            // 焦点环样式的圆点。
            Element::new(NodeTag::Container)
                .with_key("focus")
                .style("dot"),
            Element::new(NodeTag::Text)
                .with_key("hovered")
                .with(&props::TEXT, PropertyValue::static_str(caption(g)))
                .style("status-line")
                .styles((g.hovered.is_some()).then_some("hovered")),
        ])
}

/// 状态行文案：无 hover 目标时给一句提示，否则给目标名。
fn caption(g: &Gallery) -> &'static str {
    // `hovered_key` 已经把"没有"折成提示语了，所以这里不需要再 match。
    g.hovered_key()
}

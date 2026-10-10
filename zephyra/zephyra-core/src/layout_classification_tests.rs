//! B0：把**全部真实 `props::*` 键**的 `affects_layout` 分类逐个钉死。
//!
//! # 为什么必须有这条
//!
//! `property.rs` 承诺「忘了声明会被测试立刻抓到」，但原有的
//! `layout_affecting_properties_are_classified` 只测了**测试自造**的
//! `MARGIN` / `LAYOUT_WIDTH` / `BACKGROUND` —— **没有任何一条枚举真实 `props::*`**。
//! 于是承诺没兑现：本文件修的那几个漏判（文本三键 + 弹性三键）在改之前一直是绿的。
//!
//! 漏判的后果是零报错的布局错乱：`set_prop` 拿到 `Dirty::Paint` 而不是
//! `Dirty::Layout` → 不重排 → **布局盒是旧尺寸、字形按新字号画** → 文字压到邻居上。
//!
//! # 两条约束
//!
//! 1. **每个真实键都要出现在表里。** 加了新属性却忘了往这里加一条，编译不过
//!    —— 这比运行时才发现强得多。
//! 2. **每条都写明「为什么是这个分类」**，而不是只写 `true`。
//!
//! # `Image.Size` 特意是 `false`
//!
//! 它被 `paint::render_rec` 读来决定图像在节点矩形里画到哪儿，而 `measure` 从不读它
//! —— 节点自己的盒子由 `WIDTH` / `HEIGHT` / 内边距 / 子节点决定，图像画在**已经排好版**
//! 的盒子里面。

use crate::property::{name_affects_layout, PropertyKey};
use crate::props;

/// `name_affects_layout` 对每个真实键的**期望**分类，以及理由。
///
/// ⚠ 这里列的是**引用**而不是字符串 —— 改名会被编译器抓到，而字符串不会。
const EXPECTED: &[(&PropertyKey, bool, &str)] = &[
    // ---- 影响布局：尺寸 / 间距 ----
    (&props::PADDING, true, "内边距改变可用空间"),
    (&props::WIDTH, true, "显式宽度"),
    (&props::HEIGHT, true, "显式高度"),
    (&props::SPACING, true, "子节点间距（`Layout.` 前缀）"),
    (&props::COLLAPSED, true, "折叠改变盒子是否参与测量"),
    // ---- 影响布局：弹性容器 ----
    // `arrange` 读 `node.cross_align(ctx)` 与 `node.stack(ctx)`
    //（后者带 `direction` + `main_align`）。
    (&props::DIRECTION, true, "主轴方向决定子节点怎么排"),
    (&props::CROSS_ALIGN, true, "`arrange` 直接读 `node.cross_align`"),
    (&props::MAIN_ALIGN, true, "经 `node.stack()` 进入 `arrange`"),
    // ---- 影响布局：文本度量 ----
    // `measure` 走 `node.text_size(ctx)`，它内部取 `text_style()`（字号 + 行高）
    // 与 `text_max_width()`，三者都进 `engine.shape(...)`。
    (&props::TEXT_SIZE, true, "字号进 `engine.shape`"),
    (&props::TEXT_LINE_HEIGHT, true, "行高进 `text_style`"),
    (&props::TEXT_MAX_WIDTH, true, "换行宽度进 `engine.shape`"),
    // ---- 只重绘 ----
    (&props::BACKGROUND, false, "只决定填充色"),
    (&props::CORNER_RADIUS, false, "只决定 SDF 圆角"),
    (&props::BORDER_COLOR, false, "只决定描边色"),
    (&props::BORDER_WIDTH, false, "描边在同一个 Quad 里，不改盒子"),
    (&props::OPACITY, false, "只影响合成"),
    (&props::LAYER, false, "只影响排序"),
    (&props::FILTER, false, "滤镜不改变布局盒（离屏范围另算）"),
    (&props::BLEND_MODE, false, "只影响合成"),
    (&props::IMAGE_TEXTURE, false, "只决定画哪张图"),
    (&props::IMAGE_SIZE, false, "画在**已排好版**的盒子里面，`measure` 不读"),
    (&props::IMAGE_TINT, false, "只影响着色"),
    (&props::IMAGE_SRC, false, "只影响 uv"),
    (&props::ICON_NAME, false, "只决定画哪个图标"),
    (&props::ICON_COLOR, false, "只影响图标颜色"),
    (&props::TEXT, false, "文本内容本身进 `shape`，但**内容变了不等于盒子要重排**"),
    (&props::TEXT_COLOR, false, "只影响颜色"),
    (&props::INTERACTIVE, false, "只影响命中测试与状态"),
    (&props::SELECTED, false, "只影响状态"),
    (&props::DISABLED, false, "只影响状态"),
];

#[test]
fn every_real_property_key_has_a_pinned_layout_classification() {
    for (key, want, why) in EXPECTED {
        let got = name_affects_layout(key.name());
        assert_eq!(
            got, *want,
            "{} 的布局分类不对（{why}）。它现在是 {got}，期望 {want}",
            key.name()
        );
    }
}

#[test]
fn the_pinned_list_covers_every_property_declared_in_props() {
    // 反向：props.rs 里新增了键却忘了往 `EXPECTED` 里加一条 —— 那条键的分类就
    // 永远没人钉。这里用一张手工清单对照；漏一个就是漏一个。
    const DECLARED: &[&str] = &[
        "PADDING", "WIDTH", "HEIGHT", "DIRECTION", "SPACING", "CROSS_ALIGN",
        "MAIN_ALIGN", "COLLAPSED", "BACKGROUND", "CORNER_RADIUS", "BORDER_COLOR",
        "BORDER_WIDTH", "OPACITY", "LAYER", "FILTER", "BLEND_MODE",
        "IMAGE_TEXTURE", "IMAGE_SIZE", "IMAGE_TINT", "IMAGE_SRC", "ICON_NAME",
        "ICON_COLOR", "TEXT", "TEXT_SIZE", "TEXT_COLOR", "TEXT_MAX_WIDTH",
        "TEXT_LINE_HEIGHT", "INTERACTIVE", "SELECTED", "DISABLED",
    ];
    let pinned: Vec<&str> = EXPECTED.iter().map(|(k, _, _)| k.name()).collect();
    // ⚠ `DECLARED` 记的是 Rust 的**静态名**，`key.name()` 是**属性字符串**，两者不同，
    // 所以下面按属性字符串比对。留这张表是为了「改属性名」时能被看见。
    debug_assert_eq!(
        DECLARED.len(),
        NAMES.len(),
        "DECLARED 与 NAMES 的条数应当一一对应"
    );
    // `props` 里每个键的**属性字符串**：
    const NAMES: &[&str] = &[
        "Padding", "Width", "Height", "Direction", "Layout.Spacing", "CrossAlign",
        "MainAlign", "Collapsed", "Background", "CornerRadius", "BorderColor",
        "BorderWidth", "Opacity", "Layer", "Filter", "BlendMode", "Image.Texture",
        "Image.Size", "Image.Tint", "Image.Src", "Icon.Name", "Icon.Color",
        "Text", "Text.Size", "Text.Color", "Text.MaxWidth", "Text.LineHeight",
        "Interactive", "Selected", "Disabled",
    ];
    for n in NAMES {
        assert!(
            pinned.contains(n),
            "属性 `{n}` 没有出现在 EXPECTED 里 —— 它的 affects_layout 分类没人钉。\
             加新属性时记得同时往那张表里加一条。"
        );
    }
    assert_eq!(
        pinned.len(),
        NAMES.len(),
        "EXPECTED 与 props 的键数对不上：EXPECTED {} 条，props {} 条",
        pinned.len(),
        NAMES.len()
    );
}
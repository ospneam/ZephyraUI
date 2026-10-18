//! 卡片族：Card / SettingCard / ClipCard / Expander。
//!
//! 照 RinUI 的 `Frame`（cardColor 底 + 1px cardBorder 描边 + 小圆角）与
//! `SettingCard`（左文右控、两端对齐）。两端对齐用的是 core 布局的
//! `main-align: space-between` —— 一个布局属性，由样式表给，不需要
//! "右浮动"这类第二个概念。

use zephyra_core::element::Element;
use zephyra_core::id::NodeTag;
use zephyra_core::property::PropertyValue;
use zephyra_core::props;

/// 通用卡片。`extra` 给调用方追加一个语义分类名（如 `link-card`），
/// 该词的长相由应用样式表说。
pub fn card(key: &'static str, extra: &'static str, children: Vec<Element>) -> Element {
    let mut el = Element::new(NodeTag::Container).with_key(key).style("card");
    if !extra.is_empty() {
        el = el.style(extra);
    }
    el.with_children(children)
}

/// 设置卡（RinUI SettingCard）：标题 + 描述在左，控件位在右。
pub fn setting_card(
    key: &'static str,
    title: &'static str,
    desc: &'static str,
    control: Element,
) -> Element {
    Element::new(NodeTag::Container)
        .with_key(key)
        .style("setting-card")
        .with_children([
            Element::new(NodeTag::Container)
                .with_key("texts")
                .style("setting-texts")
                .with_children([
                    Element::new(NodeTag::Text)
                        .with_key("title")
                        .with(&props::TEXT, PropertyValue::static_str(title))
                        .style("setting-title"),
                    Element::new(NodeTag::Text)
                        .with_key("desc")
                        .with(&props::TEXT, PropertyValue::static_str(desc))
                        .style("setting-desc"),
                ]),
            Element::new(NodeTag::Container)
                .with_key("control-slot")
                .style("setting-control")
                .with_children([control]),
        ])
}

/// 控件缩略卡（RinUI 的 ControlClip）：图标 + 标题 + 一句说明。
pub fn clip_card(
    key: &'static str,
    icon: &'static str,
    title: &'static str,
    desc: &'static str,
) -> Element {
    Element::new(NodeTag::Container)
        .with_key(key)
        .style("clip-card")
        .with_children([
            Element::new(NodeTag::Container)
                .with_key("icon")
                .with(&props::ICON_NAME, PropertyValue::static_str(icon))
                .style("clip-icon"),
            Element::new(NodeTag::Container)
                .with_key("texts")
                .style("clip-texts")
                .with_children([
                    Element::new(NodeTag::Text)
                        .with_key("title")
                        .with(&props::TEXT, PropertyValue::static_str(title))
                        .style("clip-title"),
                    Element::new(NodeTag::Text)
                        .with_key("desc")
                        .with(&props::TEXT, PropertyValue::static_str(desc))
                        .style("clip-desc"),
                ]),
        ])
}

/// 展开器（RinUI Expander）：卡头（标题 + 箭头）+ 可折叠内容。
///
/// 折叠走 `COLLAPSED`（布局属性）：折叠的子节点不占位、不绘制，
/// 对应 WinUI 的 `Visibility=Collapsed`。它不在"外观禁写"清单里 ——
/// 可见性是**结构**，不是长相。
pub fn expander(
    key: &'static str,
    title: &'static str,
    expanded: bool,
    children: Vec<Element>,
) -> Element {
    Element::new(NodeTag::Container)
        .with_key(key)
        .style("expander")
        .with_children([
            // ⚠ 可点区域的 key 就是控件自己的 key，不是内部名 `header`。
            //
            // 曾经写成 `"header"`：树里每个 expander 的可点节点都叫 `header`
            // （同一页有两个就撞名），输入回调只会报 `header` —— 应用无法
            // 分辨是哪一个被点了，而画廊的 `set-exp` / `set-adv` 分支永远
            // 收不到点击。症状正是「展开器点了没反应」，且不崩不报错。
            Element::new(NodeTag::Container)
                .with_key(key)
                .with(&props::INTERACTIVE, PropertyValue::bool(true))
                .style("expander-header")
                .with_children([
                    Element::new(NodeTag::Text)
                        .with_key("title")
                        .with(&props::TEXT, PropertyValue::static_str(title))
                        .style("expander-title"),
                    Element::new(NodeTag::Container)
                        .with_key("chevron")
                        .with(&props::ICON_NAME, PropertyValue::static_str("chevron-down"))
                        .style("expander-chevron")
                        .style(if expanded { "open" } else { "closed" }),
                ]),
            Element::new(NodeTag::Container)
                .with_key("content")
                .style("expander-content")
                .with(&props::COLLAPSED, PropertyValue::bool(!expanded))
                .with_children(children),
        ])
}

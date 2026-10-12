//! 徽章（RinUI InfoBadge）：16px 药丸 + 严重级配色，dot 变体 4×4。
//!
//! 严重级（info / success / caution / error）是分类名：具体用哪个色板
//! 由样式表说，源码只发"这一枚是什么级别"。

use zephyra_core::element::Element;
use zephyra_core::id::NodeTag;
use zephyra_core::property::PropertyValue;
use zephyra_core::props;

use super::with_classes;

/// 文字徽章。`severity` ∈ `info` / `success` / `caution` / `error`。
pub fn info_badge(key: &'static str, text: &'static str, severity: &'static str, solid: bool) -> Element {
    let mut classes: Vec<&'static str> = vec![severity];
    if !solid {
        classes.push("outline");
    }
    // 文字也要拿到同一组词：solid 的字色是 `on_accent` 系，outline 的是
    // 严重级色 —— 两者都只看徽章自己的分类名（无后代组合器）。
    let text = with_classes(
        Element::new(NodeTag::Text)
            .with_key("text")
            .with(&props::TEXT, PropertyValue::static_str(text)),
        &classes,
    )
    .style("badge-text");
    with_classes(
        Element::new(NodeTag::Control).with_key(key).style("badge"),
        &classes,
    )
    .with_children([text])
}

/// 圆点徽章（4×4，无文字）。
pub fn dot_badge(key: &'static str, severity: &'static str) -> Element {
    Element::new(NodeTag::Container)
        .with_key(key)
        .style("badge-dot")
        .style(severity)
}

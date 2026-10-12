//! 按钮族：Button / ToolButton / PillButton。
//!
//! 形状照 RinUI 的 `BasicInput`：圆角 5、padding 6/5/7、hover 变次级填充、
//! pressed 变三级填充。accent 变体 = highlighted（强调底 + `on_accent` 字），
//! subtle 变体 = flat 透明底。
//!
//! # 变体与状态为什么要下发给子节点
//!
//! 选择器**没有后代组合器**（本版本只有 `Type.A.B`），子节点选不中父的状态。
//! 而按钮里的图标色 / 字色恰恰随变体与状态变（accent 上的图标要
//! `on_accent`）。所以控件把同一组分类名发给每个子节点 —— 那是**分类**，
//! 不是值，源码仍然一行外观都没有。
//!
//! # 特异性顺序
//!
//! `Control.button.accent`（2 名）与 `Control.button.hovered`（2 名）同级，
//! 按出现次序后者赢 —— 所以样式表里**状态规则必须放在变体规则后面**，
//! 且 generic 状态规则不许写 `background`（那会把 accent 按钮hover 成灰）。
//! 真需要变体 × 状态交叉的（standard 的 hover 底）用 3 名规则：

use zephyra_core::element::Element;
use zephyra_core::id::NodeTag;
use zephyra_core::property::PropertyValue;
use zephyra_core::props;

use super::state;

/// 把一组分类名贴到一个元素上。
pub fn with_classes(mut el: Element, classes: &[&'static str]) -> Element {
    for c in classes {
        el = el.style(c);
    }
    el
}

/// 标准按钮（RinUI Button）。
///
/// `variant` ∈ `standard` / `accent` / `subtle`，按语义给词。
pub fn button(
    key: &'static str,
    label: &'static str,
    icon: Option<&'static str>,
    variant: &'static str,
    hovered: bool,
    pressed: bool,
) -> Element {
    let mut classes = vec![variant];
    classes.extend(state(hovered, pressed));
    let mut children = Vec::new();
    if let Some(icon) = icon {
        children.push(with_classes(
            Element::new(NodeTag::Container)
                .with_key("icon")
                .with(&props::ICON_NAME, PropertyValue::static_str(icon)),
            &classes,
        ));
    }
    children.push(with_classes(
        Element::new(NodeTag::Text)
            .with_key("label")
            .with(&props::TEXT, PropertyValue::static_str(label)),
        &classes,
    ));
    let children = relabel_children(children, "btn-icon", "btn-label");
    with_classes(
        Element::new(NodeTag::Control)
            .with_key(key)
            .with(&props::INTERACTIVE, PropertyValue::bool(true))
            .style("button"),
        &classes,
    )
    .with_children(children)
}

/// 工具按钮：40×40 圆角，只有一个图标（RinUI ToolButton）。
pub fn tool_button(key: &'static str, icon: &'static str, hovered: bool, pressed: bool) -> Element {
    let classes = state(hovered, pressed);
    with_classes(
        Element::new(NodeTag::Control)
            .with_key(key)
            .with(&props::INTERACTIVE, PropertyValue::bool(true))
            .style("tool-button")
            .with_children([with_classes(
                Element::new(NodeTag::Container)
                    .with_key("icon")
                    .with(&props::ICON_NAME, PropertyValue::static_str(icon)),
                &classes,
            )
            .style("tool-icon")]),
        &classes,
    )
}

/// 药丸按钮（RinUI PillButton = RoundButton + checkable）。
///
/// `checked` 走 `checked` 分类名：选中即强调底，与 hover/pressed 同一机制。
pub fn pill_button(
    key: &'static str,
    label: &'static str,
    icon: Option<&'static str>,
    checked: bool,
    hovered: bool,
    pressed: bool,
) -> Element {
    let mut classes: Vec<&'static str> = Vec::new();
    if checked {
        classes.push("checked");
    }
    classes.extend(state(hovered, pressed));
    let mut children = Vec::new();
    if let Some(icon) = icon {
        children.push(with_classes(
            Element::new(NodeTag::Container)
                .with_key("icon")
                .with(&props::ICON_NAME, PropertyValue::static_str(icon)),
            &classes,
        ));
    }
    children.push(with_classes(
        Element::new(NodeTag::Text)
            .with_key("label")
            .with(&props::TEXT, PropertyValue::static_str(label)),
        &classes,
    ));
    let children = relabel_children(children, "pill-icon", "pill-label");
    with_classes(
        Element::new(NodeTag::Control)
            .with_key(key)
            .with(&props::INTERACTIVE, PropertyValue::bool(true))
            .style("pill"),
        &classes,
    )
    .with_children(children)
}

/// 给子节点补上各自的部件分类名（`btn-icon` / `btn-label`、`pill-icon` /
/// `pill-label`…）。
///
/// 单独一步而不是就地写：部件名（这是图标位）与状态词（hovered）是两个
/// 维度的词，混在一个循环里早晚会漏发一个。
fn relabel_children(
    children: Vec<Element>,
    icon_part: &'static str,
    text_part: &'static str,
) -> Vec<Element> {
    children
        .into_iter()
        .map(|c| match c.tag() {
            NodeTag::Text => c.style(text_part),
            NodeTag::Container => c.style(icon_part),
            _ => c.style(text_part),
        })
        .collect()
}

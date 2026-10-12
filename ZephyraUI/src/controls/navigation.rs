//! Navigation 第二批：NavigationSubItem / SelectorBar / SelectorBarItem /
//! ErrorPage。
//!
//! 度量出处（RinUI）：
//! - NavigationSubItem：`Navigation/NavigationSubItem.qml` —— 高 40、
//!   `buttonRadius / 2` 圆角、相对父项缩进一档。
//! - SelectorBar / SelectorBarItem：`Navigation/SelectorBar.qml` +
//!   `SegmentedItem.qml` —— TabBar 无容器底；项高 32、宽
//!   `max(内容 + 26, 40)`，选中项 `controlFillColor` 底 + 1px 边，
//!   未选中透明（ hover 时 `subtleSecondary`）。
//! - ErrorPage：`Navigation/ErrorPage.qml` —— 居中列：64px 留白 + 副标题 +
//!   正文（0.8 宽、居中）+ Retry 按钮。
//!
//! # 缩进怎么来
//!
//! core 布局没有 margin。缩进 = 一个**常驻的空档容器**（`nav-indent`，
//! 宽度由样式表给）—— 与"左侧强调条常驻"同一手法：布局位一直在，
//! 不靠增删结构表达状态。

use zephyra_core::element::Element;
use zephyra_core::id::NodeTag;
use zephyra_core::property::PropertyValue;
use zephyra_core::props;

use super::{glyph, state, with_classes};
use crate::controls::button;

/// 导航子项（RinUI NavigationSubItem）：比父项矮一档、缩进一档。
pub fn nav_sub_item(
    key: &'static str,
    label: &'static str,
    icon: Option<&'static str>,
    selected: bool,
    hovered: bool,
    pressed: bool,
) -> Element {
    let mut classes: Vec<&'static str> = Vec::new();
    if selected {
        classes.push("selected");
    }
    classes.extend(state(hovered, pressed));
    let mut children: Vec<Element> = vec![
        // 缩进空档：常驻，宽度在样式表里。
        Element::new(NodeTag::Container)
            .with_key("indent")
            .style("nav-indent"),
    ];
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
    let children: Vec<Element> = children
        .into_iter()
        .map(|c| match c.tag() {
            NodeTag::Text => c.style("sub-label"),
            NodeTag::Container if c.style_names().contains(&"nav-indent") => c,
            _ => c.style("sub-icon"),
        })
        .collect();
    with_classes(
        Element::new(NodeTag::Control)
            .with_key(key)
            .with(&props::INTERACTIVE, PropertyValue::bool(true))
            .style("nav-sub"),
        &classes,
    )
    .with_children(children)
}

/// 底部选择栏（RinUI SelectorBar）：横排若干 Tab 项，无容器底。
///
/// 与 `segmented` 的差别：项各自独立（选中态自己给底与边），整条没有外壳。
pub fn selector_bar(
    key: &'static str,
    items: &'static [(&'static str, &'static str)],
    selected: Option<&'static str>,
    hovered: Option<&str>,
    pressed: Option<&str>,
) -> Element {
    let children: Vec<Element> = items
        .iter()
        .map(|(id, label)| {
            let is_sel = Some(*id) == selected;
            let mut classes: Vec<&'static str> = Vec::new();
            if is_sel {
                classes.push("selected");
            }
            classes.extend(state(hovered == Some(*id), pressed == Some(*id)));
            let label = with_classes(
                Element::new(NodeTag::Text)
                    .with_key("label")
                    .with(&props::TEXT, PropertyValue::static_str(label)),
                &classes,
            )
            .style("selector-label");
            with_classes(
                Element::new(NodeTag::Control)
                    .with_key(*id)
                    .with(&props::INTERACTIVE, PropertyValue::bool(true))
                    .style("selector-item")
                    .with_children([label]),
                &classes,
            )
        })
        .collect();
    Element::new(NodeTag::Container)
        .with_key(key)
        .style("selector-bar")
        .with_children(children)
}

/// 错误页（RinUI ErrorPage）：居中列 + Retry 按钮。
pub fn error_page(
    key: &'static str,
    title: &'static str,
    message: &'static str,
    retry: &'static str,
    retry_key: &'static str,
    hovered_retry: bool,
    pressed_retry: bool,
) -> Element {
    Element::new(NodeTag::Container)
        .with_key(key)
        .style("error-page")
        .with_children([
            // 顶部留白（RinUI 的 Item height 64）。
            Element::new(NodeTag::Container)
                .with_key("top-gap")
                .style("error-gap"),
            glyph("icon", "close", "error-icon"),
            Element::new(NodeTag::Text)
                .with_key("title")
                .with(&props::TEXT, PropertyValue::static_str(title))
                .style("error-title"),
            Element::new(NodeTag::Text)
                .with_key("message")
                .with(&props::TEXT, PropertyValue::static_str(message))
                .style("error-message"),
            button(
                retry_key,
                retry,
                None,
                "accent",
                hovered_retry,
                pressed_retry,
            ),
        ])
}

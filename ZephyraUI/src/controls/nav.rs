//! 导航项（RinUI NavigationItem）：37px 高的扁平按钮，选中态次级底 +
//! 左侧 3px 强调条。
//!
//! 左侧强调条是**常驻**子节点：未选中时它的背景透明（样式表给
//! `transparent`），于是不产生绘制命令也不占视觉 —— 但布局位一直在，
//! 选中/未选中切换不引起结构 diff。这是"状态不增删节点"的又一例。

use zephyra_core::element::Element;
use zephyra_core::id::NodeTag;
use zephyra_core::property::PropertyValue;
use zephyra_core::props;

use super::{state, with_classes};

/// 导航项。`page` 是点击后要去的路由 id（也是事件 key）。
pub fn nav_item(
    key: &'static str,
    label: &'static str,
    icon: &'static str,
    selected: bool,
    hovered: bool,
    pressed: bool,
) -> Element {
    // 选中/状态同样要下发到图标与文字：选中态的图标与字色都不同，
    // 而选择器没有后代组合器（见 button.rs 的同款说明）。
    let mut classes: Vec<&'static str> = Vec::new();
    if selected {
        classes.push("selected");
    }
    classes.extend(state(hovered, pressed));
    let mut bar = Element::new(NodeTag::Container)
        .with_key("bar")
        .style("nav-bar");
    if selected {
        bar = bar.style("selected");
    }
    with_classes(
        Element::new(NodeTag::Control)
            .with_key(key)
            .with(&props::INTERACTIVE, PropertyValue::bool(true))
            .style("nav-item")
            .with_children([
                bar,
                with_classes(
                    Element::new(NodeTag::Container)
                        .with_key("icon")
                        .with(&props::ICON_NAME, PropertyValue::static_str(icon)),
                    &classes,
                )
                .style("nav-icon"),
                with_classes(
                    Element::new(NodeTag::Text)
                        .with_key("label")
                        .with(&props::TEXT, PropertyValue::static_str(label)),
                    &classes,
                )
                .style("nav-label"),
            ]),
        &classes,
    )
}

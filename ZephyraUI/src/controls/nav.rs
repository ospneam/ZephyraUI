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
///
/// # `compact`
///
/// 紧凑档（官方断点的「小」）只留图标、隐藏文字 —— 这正是 WinUI NavigationView
/// 的 `LeftCompact` 窗格模式，出处见 `docs/responsive-and-touch.md` §6 引
/// `responsive-design` 页：「瞬态 UI（导航窗格及其精简版）适用于较小的屏幕，
/// 但在较大屏幕上，最好选择使用选项卡」。
///
/// 用 [`props::COLLAPSED`] 折叠文字，而**不是不建这个节点**：两者视觉相同，
/// 但折叠保留身份，于是展开/收起时 `label` 节点的 key 不变，diff 能复用；
/// 不建则每次切换都要重建一次文字节点，而文字的重建要走一遍排版与图集。
pub fn nav_item(
    key: &'static str,
    label: &'static str,
    icon: &'static str,
    selected: bool,
    hovered: bool,
    pressed: bool,
    compact: bool,
) -> Element {
    // 选中/状态同样要下发到图标与文字：选中态的图标与字色都不同，
    // 而选择器没有后代组合器（见 button.rs 的同款说明）。
    let mut classes: Vec<&'static str> = Vec::new();
    if selected {
        classes.push("selected");
    }
    if compact {
        classes.push("compact");
    }
    classes.extend(state(hovered, pressed));
    let mut bar = Element::new(NodeTag::Container)
        .with_key("bar")
        .style("nav-bar");
    if selected {
        bar = bar.style("selected");
    }
    let label_node = with_classes(
        Element::new(NodeTag::Text)
            .with_key("label")
            .with(&props::TEXT, PropertyValue::static_str(label))
            .with(&props::COLLAPSED, PropertyValue::bool(compact)),
        &classes,
    )
    .style("nav-label");
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
                label_node,
            ]),
        &classes,
    )
}

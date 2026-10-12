//! MenusAndToolbars 第一批：MenuBar / Menu / MenuItem / MenuItemGroup。
//!
//! 度量出处（RinUI）：
//! - MenuBar：`MenusAndToolbars/MenuBar.qml` —— padding 4、项高 32、
//!   `subtleSecondaryColor` 底（= 我们的 `control_secondary`）、radius 5。
//! - Menu：`MenusAndToolbars/Menu.qml` —— 弹出卡（宽 ≥ 80）。**浮层不做**
//!   （core 无 overlay）：这里是同规格的内联卡，宿主决定摆在哪。
//! - MenuItem：`MenusAndToolbars/MenuItem.qml` —— 行高 34、左右 margin 16、
//!   可选 check 指示器（`MenuItemGroup` 互斥选择）与子菜单箭头（chevron）。
//! - ToolSeparator / MenuSeparator：`controls::separator`（已随首批落地）。
//!
//! # 互斥选择（MenuItemGroup）怎么表达
//!
//! RinUI 的 group 是运行时互斥；这里**退化成一个分类名**：被选中的项发
//! `checked`（与 CheckBox 同一机制），互斥语义由宿主状态保证 —— 这正是
//! "状态是分类名"规矩的延伸，源码里没有"选中了谁"的判断。

use zephyra_core::element::Element;
use zephyra_core::id::NodeTag;
use zephyra_core::property::PropertyValue;
use zephyra_core::props;

use super::{state, with_classes};

/// 菜单栏（RinUI MenuBar）：横排若干顶级项。
///
/// `items` 是顶级项标签；`open` 是当前展开项的**下标**（`None` = 全收起）。
/// 展开出来的 Menu 由宿主挂在旁边（core 无浮层）—— 这里只画栏本身。
pub fn menu_bar(key: &'static str, items: &'static [&'static str], open: Option<usize>) -> Element {
    let children: Vec<Element> = items
        .iter()
        .enumerate()
        .map(|(i, label)| {
            menu_bar_item(
                bar_item_key(i),
                label,
                open == Some(i),
                false,
                false,
            )
        })
        .collect();
    Element::new(NodeTag::Container)
        .with_key(key)
        .style("menu-bar")
        .with_children(children)
}

/// 菜单栏的一项（高 32、radius 5、次级底、14px 字）。
pub fn menu_bar_item(
    key: &'static str,
    label: &'static str,
    open: bool,
    hovered: bool,
    pressed: bool,
) -> Element {
    let mut classes = Vec::new();
    if open {
        classes.push("open");
    }
    classes.extend(state(hovered, pressed));
    with_classes(
        Element::new(NodeTag::Container)
            .with_key(key)
            .with(&props::INTERACTIVE, PropertyValue::bool(true))
            .style("menu-bar-item")
            .with_children([Element::new(NodeTag::Text)
                .with_key("label")
                .with(&props::TEXT, PropertyValue::static_str(label))
                .style("bar-item-label")]),
        &classes,
    )
}

/// 菜单卡（RinUI Menu 的内联形态）：竖排菜单项 + 分隔线。
///
/// `entries` 每项 `(label, key)`。分隔线用空标签表示（与 RinUI 的
/// MenuSeparator 同一手法：数据里说"这里是条线"）。
pub fn menu(key: &'static str, entries: &'static [(&'static str, &'static str)]) -> Element {
    let mut children: Vec<Element> = Vec::new();
    for (i, (label, item_key)) in entries.iter().enumerate() {
        if label.is_empty() {
            children.push(
                Element::new(NodeTag::Container)
                    .with_key(menu_sep_key(i))
                    .style("menu-separator"),
            );
            continue;
        }
        children.push(menu_item(item_key, label, false, false, false, false));
    }
    Element::new(NodeTag::Container)
        .with_key(key)
        .style("menu")
        .with_children(children)
}

/// 菜单项（RinUI MenuItem）：行高 34、左 check 指示器（可空）、右箭头（可空）。
///
/// `checked` 是互斥组里的选中态（分类名，宿主状态驱动）；`arrow` 表示有子菜单
/// （chevron 常驻，无箭头时颜色透明、不占视觉）。
pub fn menu_item(
    key: &'static str,
    label: &'static str,
    checked: bool,
    arrow: bool,
    hovered: bool,
    pressed: bool,
) -> Element {
    let mut classes = Vec::new();
    if checked {
        classes.push("checked");
    }
    classes.extend(state(hovered, pressed));
    let mut check = Element::new(NodeTag::Container)
        .with_key("check")
        .style("menu-check");
    if checked {
        check = check.style("checked");
    }
    let mut chevron = Element::new(NodeTag::Container)
        .with_key("arrow")
        .with(&props::ICON_NAME, PropertyValue::static_str("chevron-down"))
        .style("menu-arrow");
    if arrow {
        chevron = chevron.style("on");
    }
    with_classes(
        Element::new(NodeTag::Container)
            .with_key(key)
            .with(&props::INTERACTIVE, PropertyValue::bool(true))
            .style("menu-item")
            .with_children([
                check,
                Element::new(NodeTag::Text)
                    .with_key("label")
                    .with(&props::TEXT, PropertyValue::static_str(label))
                    .style("menu-item-label"),
                chevron,
            ]),
        &classes,
    )
}

fn bar_item_key(i: usize) -> &'static str {
    ["bar:0", "bar:1", "bar:2", "bar:3"][i.min(3)]
}

fn menu_sep_key(i: usize) -> &'static str {
    ["msep:0", "msep:1", "msep:2", "msep:3", "msep:4", "msep:5"][i.min(5)]
}

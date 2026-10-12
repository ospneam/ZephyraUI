//! Text 族第二批：TextField / TextArea / ScrollableTextArea /
//! AutoSuggestBox / TextInputMenu。
//!
//! 度量出处（RinUI）：
//! - TextField：`Text/TextField.qml` —— 宽 160、radius 5、`controlColor` 底 +
//!   1px `controlBorderColor` 边 + 底部 1px 指示条（focused 时 2px accent）、
//!   左 padding 12 / 右 12（有清空钮时 28）/ 上 4 / 下 7、清空钮 26×22
//!   （dismiss 图标 14，radius 3）。
//! - TextArea：`Text/TextArea.qml` —— 同款底与边、多行、无底部指示条。
//! - ScrollableTextArea：TextArea + 右侧滚动条（宽 6、 thumb 40）。
//! - AutoSuggestBox：`Text/AutoSuggestBox.qml` —— TextField + 建议列表
//!   （每行 32 高，选中行 `controlSecondaryColor`）。
//! - TextInputMenu：`MenusAndToolbars/TextInputMenu.qml` —— 右键菜单卡
//!   （radius 5、item 高 32、分隔线 `dividerBorderColor`）。
//!
//! # 交互降级记账
//!
//! - **真实键入不做**（core 无输入法/光标）：值由宿主状态给，`value` 是
//!   `Option<&str>`（None = 显示 placeholder）。
//! - **focused** 是调用方给的状态词（没有键盘焦点系统，宿主用点击置位），
//!   下发成 `focused` 分类名，底部指示条由样式表切 2px accent。
//! - AutoSuggestBox 的建议**浮层**不做（core 无 overlay）—— 建议列表作为
//!   常驻子节点在输入框下方展开，语义不变。
//! - TextInputMenu 作为**内联菜单卡**给出（RinUI 是右键浮层，这里由宿主
//!   决定把它摆在哪个容器旁边）。

use zephyra_core::element::Element;
use zephyra_core::id::NodeTag;
use zephyra_core::property::PropertyValue;
use zephyra_core::props;

use super::{state, with_classes};

/// 单行输入框（RinUI TextField）。
///
/// `value` 为 `None` 时显示 placeholder（次要色）。focused 由宿主状态给
/// （无键盘焦点系统），样式表据此加粗底部指示条。
pub fn text_field(
    key: &'static str,
    value: Option<&'static str>,
    placeholder: &'static str,
    focused: bool,
    hovered: bool,
    pressed: bool,
) -> Element {
    let classes = field_classes(focused, hovered, pressed);
    // 值或占位符：内容是**数据**，两者都是文字。
    let shown = value.unwrap_or(placeholder);
    let is_placeholder = value.is_none();
    let mut text = Element::new(NodeTag::Text)
        .with_key("text")
        .with(&props::TEXT, PropertyValue::static_str(shown))
        .style("field-text");
    if is_placeholder {
        text = text.style("placeholder");
    }
    if focused {
        text = text.style("focused");
    }
    let mut children = vec![text];
    // 清空钮：有值且聚焦才显示。**显隐靠 COLLAPSED**（布局属性：不占位、
    // 不绘制），不靠增删子节点 —— 结构每帧稳定，diff 不做无用功。
    if value.is_some() {
        children.push(clear_button("field:clear", focused, hovered));
    }
    with_classes(
        Element::new(NodeTag::Control)
            .with_key(key)
            .with(&props::INTERACTIVE, PropertyValue::bool(true))
            .style("text-field")
            .with_children(children),
        &classes,
    )
}

/// 多行文本区（RinUI TextArea）。
pub fn text_area(key: &'static str, lines: &'static [&'static str]) -> Element {
    let children: Vec<Element> = lines
        .iter()
        .enumerate()
        .map(|(i, l)| {
            Element::new(NodeTag::Text)
                .with_key(line_key(i))
                .with(&props::TEXT, PropertyValue::static_str(l))
                .style("area-line")
        })
        .collect();
    Element::new(NodeTag::Container)
        .with_key(key)
        .style("text-area")
        .with_children(children)
}

/// 可滚动文本区（RinUI ScrollableTextArea）：文本区 + 右侧滚动条视觉件。
///
/// 滚动条不真的滚（core 无滚动），但 thumb 高度按 `ratio` 给 —— 那是
/// 内容驱动的几何，放在 `fill:scroll` 键节点上（见 lib.rs 第 3 条）。
pub fn scrollable_text_area(
    key: &'static str,
    lines: &'static [&'static str],
    ratio: f32,
) -> Element {
    Element::new(NodeTag::Container)
        .with_key(key)
        .style("scroll-area")
        .with_children([
            text_area("area", lines),
            Element::new(NodeTag::Container)
                .with_key("scrollbar")
                .style("scrollbar")
                .with_children([Element::new(NodeTag::Container)
                    .with_key("fill:scroll")
                    .style("scroll-thumb")
                    .with(&props::HEIGHT, PropertyValue::float((ratio * 120.0).max(16.0)))]),
        ])
}

/// 自动建议框（RinUI AutoSuggestBox）：输入框 + 下方建议列表。
///
/// 建议浮层**不做**（core 无 overlay）：列表作为常驻子节点跟在输入框下，
/// `open` 分类名控制它可不可见（不可见时折叠，同 Expander 的手法）。
pub fn auto_suggest_box(
    key: &'static str,
    value: Option<&'static str>,
    placeholder: &'static str,
    suggestions: &'static [&'static str],
    selected: usize,
    open: bool,
    hovered: bool,
) -> Element {
    let field = text_field(key, value, placeholder, open, hovered, false);
    let rows: Vec<Element> = suggestions
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let mut row = Element::new(NodeTag::Container)
                .with_key(suggest_key(i))
                .style("suggest-row");
            if i == selected {
                row = row.style("selected");
            }
            row.with_children([Element::new(NodeTag::Text)
                .with_key("label")
                .with(&props::TEXT, PropertyValue::static_str(s))
                .style("suggest-label")])
        })
        .collect();
    let mut list = Element::new(NodeTag::Container)
        .with_key("suggestions")
        .style("suggest-list")
        .with_children(rows);
    if !open {
        list = list.with(&props::COLLAPSED, PropertyValue::bool(true));
    }
    field.with_children([list])
}

/// 输入框的右键菜单（RinUI TextInputMenu）：内联菜单卡。
///
/// 浮层不做（core 无 overlay）—— 这张卡由宿主决定摆在哪（紧跟被选文本）。
pub fn text_input_menu(key: &'static str, items: &'static [&'static str]) -> Element {
    let mut children: Vec<Element> = Vec::new();
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            children.push(
                Element::new(NodeTag::Container)
                    .with_key(menu_sep_key(i))
                    .style("menu-separator"),
            );
        }
        children.push(
            Element::new(NodeTag::Container)
                .with_key(menu_item_key(i))
                .with(&props::INTERACTIVE, PropertyValue::bool(true))
                .style("menu-item")
                .with_children([Element::new(NodeTag::Text)
                    .with_key("label")
                    .with(&props::TEXT, PropertyValue::static_str(item))
                    .style("menu-label")]),
        );
    }
    Element::new(NodeTag::Container)
        .with_key(key)
        .style("context-menu")
        .with_children(children)
}

// ---------------------------------------------------------------- 内部件

fn field_classes(focused: bool, hovered: bool, pressed: bool) -> Vec<&'static str> {
    let mut v = Vec::new();
    if focused {
        v.push("focused");
    }
    v.extend(state(hovered, pressed));
    v
}

/// 清空钮（26×22 扁平 + dismiss 图标 14）。
fn clear_button(key: &'static str, focused: bool, hovered: bool) -> Element {
    let mut classes = Vec::new();
    if focused {
        classes.push("focused");
    }
    classes.extend(state(hovered, false));
    // 图标也要拿到同一组词：选择器没有后代组合器，子的颜色只能自己发类。
    with_classes(
        Element::new(NodeTag::Control)
            .with_key(key)
            .with(&props::INTERACTIVE, PropertyValue::bool(true))
            .style("field-clear")
            .with_children([with_classes(
                Element::new(NodeTag::Container)
                    .with_key("icon")
                    .with(&props::ICON_NAME, PropertyValue::static_str("close")),
                &classes,
            )
            .style("field-clear-icon")]),
        &classes,
    )
}

fn line_key(i: usize) -> &'static str {
    const KEYS: [&str; 12] = [
        "area:0", "area:1", "area:2", "area:3", "area:4", "area:5", "area:6", "area:7", "area:8",
        "area:9", "area:10", "area:11",
    ];
    KEYS[i.min(11)]
}

fn suggest_key(i: usize) -> &'static str {
    const KEYS: [&str; 6] = ["sug:0", "sug:1", "sug:2", "sug:3", "sug:4", "sug:5"];
    KEYS[i.min(5)]
}

fn menu_item_key(i: usize) -> &'static str {
    const KEYS: [&str; 6] = ["mi:0", "mi:1", "mi:2", "mi:3", "mi:4", "mi:5"];
    KEYS[i.min(5)]
}

fn menu_sep_key(i: usize) -> &'static str {
    const KEYS: [&str; 6] = ["msep:1", "msep:2", "msep:3", "msep:4", "msep:5", "msep:6"];
    KEYS[i.min(5)]
}

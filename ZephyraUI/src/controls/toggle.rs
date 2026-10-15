//! 选中族：CheckBox / RadioButton / ToggleSwitch。
//!
//! 三个控件共用同一套「指示器 + 标签」结构，差别全在分类名上：
//! `.check-*` 是圆角方框，`.radio-*` 是圆点框，`.switch-*` 是药丸轨道 +
//! 把手。**源码不画勾、不画点**——选中态由样式表按 `.checked` 分类名切换
//! 指示器填充与内部标记的可见性（标记是常驻子节点，未选中时颜色透明）。

use zephyra_core::element::Element;
use zephyra_core::id::NodeTag;
use zephyra_core::property::PropertyValue;
use zephyra_core::props;

use super::{state, with_classes};

/// 复选框（RinUI CheckBox）：20×20 圆角指示器 + 选中标记 + 文字。
pub fn check_box(
    key: &'static str,
    label: &'static str,
    checked: bool,
    hovered: bool,
    pressed: bool,
) -> Element {
    check_like(
        key,
        label,
        "check-box",
        "check-indicator",
        "check-mark",
        checked,
        hovered,
        pressed,
    )
}

/// 单选按钮（RinUI RadioButton）：指示器是圆（radius 99），标记是小圆点。
pub fn radio_button(
    key: &'static str,
    label: &'static str,
    checked: bool,
    hovered: bool,
    pressed: bool,
) -> Element {
    check_like(
        key,
        label,
        "radio-box",
        "radio-indicator",
        "radio-dot",
        checked,
        hovered,
        pressed,
    )
}

/// 三个选中控件的公共骨架。`row` / `indicator` / `mark` 都是分类名 ——
/// 三种长相（方/圆/药丸）由样式表按这三组词分别说。
fn check_like(
    key: &'static str,
    label: &'static str,
    row: &'static str,
    indicator: &'static str,
    mark: &'static str,
    checked: bool,
    hovered: bool,
    pressed: bool,
) -> Element {
    // 标签也要拿到 checked / hovered：未选中的标签与选中态同色就分不出
    // 勾没勾（ RinUI 的 CheckBox 文字色不随状态变，但我们的 `.check-label`
    // 规则按 `.checked` 给主色 —— 让"选中"在文字上也留痕）。
    let mut classes: Vec<&'static str> = Vec::new();
    if checked {
        classes.push("checked");
    }
    classes.extend(state(hovered, pressed));
    let el = with_classes(
        Element::new(NodeTag::Control)
            .with_key(key)
            .with(&props::INTERACTIVE, PropertyValue::bool(true))
            .style(row),
        &classes,
    );
    // 指示器：常驻。选中与否由它自己与标记的分类名表达 —— 状态**不**通过
    // 增删子节点表达，那会让 diff 每帧重建结构（保留式要避免的）。
    let mut indicator_el = Element::new(NodeTag::Container)
        .with_key("indicator")
        .style(indicator);
    if checked {
        indicator_el = indicator_el.style("checked");
    }
    let mut mark_el = Element::new(NodeTag::Container)
        .with_key("mark")
        .style(mark);
    if checked {
        mark_el = mark_el.style("checked");
    }
    el.with_children([
        indicator_el.with_children([mark_el]),
        with_classes(
            Element::new(NodeTag::Text)
                .with_key("label")
                .with(&props::TEXT, PropertyValue::static_str(label)),
            &classes,
        )
        .style("check-label"),
    ])
}

/// 开关（RinUI ToggleSwitch）：药丸轨道 + 把手。
///
/// 把手的左右位置由 `main-align` 决定，而 `main-align` 是**布局属性**、
/// 可以由样式表的 `.on` 规则给 —— 所以"开在右、关在左"不需要源码写数值，
/// 也没有数据几何。
pub fn toggle_switch(key: &'static str, checked: bool, hovered: bool, pressed: bool) -> Element {
    let mut el = Element::new(NodeTag::Control)
        .with_key(key)
        .with(&props::INTERACTIVE, PropertyValue::bool(true))
        .style("switch");
    for s in state(hovered, pressed) {
        el = el.style(s);
    }
    let mut track = Element::new(NodeTag::Container)
        .with_key("track")
        .style("switch-track");
    if checked {
        track = track.style("on");
    }
    el.with_children([track.with_children([Element::new(NodeTag::Container)
        .with_key("knob")
        .style("switch-knob")])])
}

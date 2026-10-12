//! 分段控件（RinUI Segmented）：药丸容器 + 若干段，选中段强调底。
//!
//! `items` 每项 `(id, label)`。段的 key = `seg:<id>`：id 是调用方的
//! 词汇（点击路由用），label 是显示词（内容）。

use zephyra_core::element::Element;
use zephyra_core::id::NodeTag;
use zephyra_core::property::PropertyValue;
use zephyra_core::props;

use super::{state, with_classes};

/// 分段控件。`selected` 是选中项的 id（`None` = 一个都不选）。
pub fn segmented(
    key: &'static str,
    items: &[(&'static str, &'static str)],
    selected: Option<&'static str>,
    hovered_id: Option<&str>,
    pressed_id: Option<&str>,
) -> Element {
    let children: Vec<Element> = items
        .iter()
        .map(|(id, label)| {
            let is_sel = Some(*id) == selected;
            let mut classes: Vec<&'static str> = Vec::new();
            if is_sel {
                classes.push("selected");
            }
            classes.extend(state(hovered_id == Some(*id), pressed_id == Some(*id)));
            let label = with_classes(
                Element::new(NodeTag::Text)
                    .with_key("label")
                    .with(&props::TEXT, PropertyValue::static_str(label)),
                &classes,
            )
            .style("seg-label");
            with_classes(
                Element::new(NodeTag::Control)
                    .with_key(*id)
                    .with(&props::INTERACTIVE, PropertyValue::bool(true))
                    .style("seg-item")
                    .with_children([label]),
                &classes,
            )
        })
        .collect();
    Element::new(NodeTag::Container)
        .with_key(key)
        .style("segmented")
        .with_children(children)
}

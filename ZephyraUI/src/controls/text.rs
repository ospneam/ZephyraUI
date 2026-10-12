//! 文本档位（RinUI Typography：title / subtitle / body / bodyStrong / caption）。
//!
//! 字号与字色是**样式的事**：这里只按语义档位发分类名。档位词与 RinUI 的
//! 枚举同名（小写连字符），对照表在 `README.md`。

use zephyra_core::element::Element;
use zephyra_core::id::NodeTag;
use zephyra_core::property::PropertyValue;
use zephyra_core::props;

/// 按档位造一个文本节点。`level` ∈ `text-title` / `text-subtitle` /
/// `text-body` / `text-body-strong` / `text-caption`。
pub fn text_label(key: &'static str, text: &'static str, level: &'static str) -> Element {
    Element::new(NodeTag::Text)
        .with_key(key)
        .with(&props::TEXT, PropertyValue::static_str(text))
        .style(level)
}

/// 12px 次要说明字的快捷方式（caption 档）。
pub fn caption(key: &'static str, text: &'static str) -> Element {
    text_label(key, text, "text-caption")
}

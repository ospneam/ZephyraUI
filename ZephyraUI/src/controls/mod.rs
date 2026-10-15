//! 控件层总览。
//!
//! # 一条规矩贯穿所有文件
//!
//! **控件只发分类名与内容，不发外观。** 一个控件函数能写的只有：
//!
//! - `NodeTag` / `ElementKey`（身份）
//! - `props::INTERACTIVE`（可交互才有 hover/press）
//! - `props::TEXT` / `props::ICON_NAME` / 自定义数据键（内容）
//! - `.style(..)` 分类名（外观的分类，不是值）
//! - 结构（children）
//!
//! 唯一例外是**填充宽度**：滑条与进度条的填充宽度由值决定（`fill:*` 键），
//! 那是内容不是版式，见 [`crate::lib`] 的第 3 条。
//!
//! # 状态分类名的优先级
//!
//! `state()` 里 pressed 盖过 hovered（与 `examples/demo.rs` 同一形状）：
//! 两者同时为真时按"正在被按"显示，那更接近按下时的视觉事实。

use zephyra_core::element::Element;
use zephyra_core::id::{ElementKey, NodeTag};
use zephyra_core::property::PropertyValue;
use zephyra_core::props;

pub mod badge;
pub mod button;
pub mod card;
pub mod collections;
pub mod dialogs;
pub mod forms;
pub mod menu;
pub mod nav;
pub mod navigation;
pub mod progress;
pub mod segmented;
pub mod status;
pub mod text;
pub mod text_input;
pub mod toggle;

pub use badge::{dot_badge, info_badge};
pub use button::{button, pill_button, tool_button, with_classes};
pub use card::{card, clip_card, expander, setting_card};
pub use collections::{avatar, list_view, setting_expander, setting_item, swipe_view, table_view};
pub use dialogs::{acrylic_card, date_picker, dialog, rounded_corner_overlay, time_picker};
pub use forms::{color_picker, combo_box, drop_down_button, hyperlink, round_button, spin_box};
pub use menu::{menu, menu_bar, menu_bar_item, menu_item};
pub use nav::nav_item;
pub use navigation::{error_page, nav_sub_item, selector_bar};
pub use progress::{progress_bar, slider};
pub use segmented::segmented;
pub use status::{busy_indicator, info_bar, page_indicator, progress_ring, toast, tool_tip};
pub use text::{caption, text_label};
pub use text_input::{
    auto_suggest_box, scrollable_text_area, text_area, text_field, text_input_menu,
};
pub use toggle::{check_box, radio_button, toggle_switch};

/// hover / pressed → 状态分类名。无状态返回空（基础规则自己给默认长相）。
///
/// pressed 优先：按下时同时显示两态会互相盖，而"正在被按"是更强的信号。
pub fn state(hovered: bool, pressed: bool) -> Vec<&'static str> {
    let mut v = Vec::new();
    if pressed {
        v.push("pressed");
    } else if hovered {
        v.push("hovered");
    }
    v
}

/// 一个矢量图标位。
///
/// `ICON_NAME` 是**内容**（像文字），不是外观：具体长什么样（尺寸、颜色）
/// 全部由样式表的 `.glyph*` 规则说。
pub fn glyph(key: &'static str, icon: &'static str, class: &'static str) -> Element {
    Element::new(NodeTag::Container)
        .with_key(key)
        .with(&props::ICON_NAME, PropertyValue::static_str(icon))
        .style(class)
}

/// 分隔线。`orientation` 是分类名（`divider` / `divider-v`），不是方向值 ——
/// 源码不说"横的还是竖的"这种版式判断，调用方按语义传词。
pub fn separator(key: &'static str, orientation: &'static str) -> Element {
    Element::new(NodeTag::Container)
        .with_key(key)
        .style(orientation)
}

/// 该 key 现在是否被悬停。控件与页面都靠它把 `InputState` 翻成布尔。
pub fn hovered_key(hovered: &Option<ElementKey>, key: &str) -> bool {
    hovered.as_ref().and_then(|k| k.as_name()) == Some(key)
}

/// 该 key 现在是否被按下（捕获语义：按下去就归它，直到抬起）。
pub fn pressed_key(pressed: &Option<ElementKey>, key: &str) -> bool {
    pressed.as_ref().and_then(|k| k.as_name()) == Some(key)
}

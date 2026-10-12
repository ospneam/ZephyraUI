//! 控件展台页 —— **每条 catalog 条目一页**（照抄 Rin-UI `pages/controls/*.qml`）。
//!
//! # 为什么单独一个模块而不是塞进 `pages.rs`
//!
//! `pages.rs` 是 **10 个大页面**（每个都已很长）；这里是 **44 个小展台**
//! （每页 5~15 行）。混在一起两个文件都会失控，且两者的演进节奏不同 ——
//! 前者跟布局，后者跟控件库。
//!
//! # 每页的共同形状
//!
//! ```text
//! section(控件名)
//!   ├ 控件实例 × 1~3（真控件，不是截图）
//!   └ 说明文字（原版 desc，英文原文）
//! ```
//!
//! # 降级是**写在页面上的**
//!
//! 做不到的控件，其展台页**明确写出能力边界**（见 [`downgrade_reason`]），
//! 而不是给空壳或假装能做。理由：demo 的价值是"照抄"，而照抄的前提是
//! **诚实标出哪些没照抄、为什么**。空页面会被当成渲染坏了 —— 那才是真正的坑。

use zephyra_core::element::Element;
use zephyra_core::id::NodeTag;

use super::catalog;
use super::pages::section;
use super::Gallery;
use crate::controls::{
    auto_suggest_box, avatar, button, check_box, color_picker, combo_box, date_picker, dialog,
    dot_badge, drop_down_button, expander, hyperlink, info_badge, info_bar, list_view, menu,
    menu_bar, nav_item, page_indicator, pill_button, progress_bar, progress_ring, radio_button,
    round_button, scrollable_text_area, segmented, selector_bar, setting_card, slider, spin_box,
    swipe_view, table_view, text_area, text_field, text_label, time_picker, toast, toggle_switch,
    tool_tip,
};

/// 按 catalog 的 `page` id 造控件展台页。
///
/// 找不到映射时返回 [`degraded`]（**不返回空页**）。
pub fn showcase(g: &Gallery, page: &str) -> Element {
    let body: Vec<Element> = match page {
        // ---------------- Basic input ----------------
        "Button" => vec![row_of(vec![
            button("sc:btn-std", "Standard", None, "standard", g.hovers("sc:btn-std"), g.holds("sc:btn-std")),
            button("sc:btn-acc", "Accent", None, "accent", g.hovers("sc:btn-acc"), g.holds("sc:btn-acc")),
            button("sc:btn-sub", "Subtle", None, "subtle", g.hovers("sc:btn-sub"), g.holds("sc:btn-sub")),
        ])],
        "CheckBox" => vec![stack_of(vec![
            check_box("sc:cb1", "Option A", g.checked[0], g.hovers("sc:cb1"), g.holds("sc:cb1")),
            check_box("sc:cb2", "Option B", g.checked[1], g.hovers("sc:cb2"), g.holds("sc:cb2")),
            check_box("sc:cb3", "Option C", g.checked[2], g.hovers("sc:cb3"), g.holds("sc:cb3")),
        ])],
        "RadioButton" => vec![stack_of(vec![
            radio_button("sc:r1", "Choice A", g.round_on, g.hovers("sc:r1"), g.holds("sc:r1")),
            radio_button("sc:r2", "Choice B", !g.round_on, g.hovers("sc:r2"), g.holds("sc:r2")),
        ])],
        "Switch" => vec![stack_of(vec![
            toggle_switch("sc:sw1", g.switch_on, g.hovers("sc:sw1"), g.holds("sc:sw1")),
            toggle_switch("sc:sw2", !g.switch_on, g.hovers("sc:sw2"), g.holds("sc:sw2")),
        ])],
        "ToggleButton" => vec![row_of(vec![pill_button(
            "sc:tg", "Toggle me", None, g.round_on, g.hovers("sc:tg"), g.holds("sc:tg"),
        )])],
        "Slider" => vec![stack_of(vec![
            slider("sc:sl1", g.volume, Some("Volume"), g.hovers("sc:sl1"), g.holds("sc:sl1")),
            slider("sc:sl2", 0.25, None, g.hovers("sc:sl2"), g.holds("sc:sl2")),
        ])],
        "ComboBox" => vec![row_of(vec![combo_box(
            "sc:cbx",
            super::COMBO_OPTIONS[g.combo.min(super::COMBO_OPTIONS.len() - 1)],
            g.hovers("sc:cbx"),
            g.holds("sc:cbx"),
        )])],
        "SpinBox" => vec![row_of(vec![spin_box(
            "sc:spin", "42", g.hovers("sc:spin"), g.holds("sc:spin"),
            g.hovers("sc:spin:up"), g.holds("sc:spin:up"),
            g.hovers("sc:spin:down"), g.holds("sc:spin:down"),
        )])],
        "ColorPicker" => vec![color_picker("sc:color", 0)],
        "Hyperlink" => vec![row_of(vec![hyperlink(
            "sc:link", "ZephyraUI docs", "", g.hovers("sc:link"),
        )])],
        "DropDownButton" => vec![row_of(vec![drop_down_button(
            "sc:dd", "Actions", None, g.dd_open, g.hovers("sc:dd"), g.holds("sc:dd"),
        )])],
        "PillButton" => vec![row_of(vec![
            pill_button("sc:pill1", "Recent", Some("home"), true, g.hovers("sc:pill1"), g.holds("sc:pill1")),
            pill_button("sc:pill2", "Favorite", Some("star"), false, g.hovers("sc:pill2"), g.holds("sc:pill2")),
        ])],
        "RoundButton" => vec![row_of(vec![round_button(
            "sc:round", "Add", Some("pen"), g.round_on, g.hovers("sc:round"), g.holds("sc:round"),
        )])],
        // ---------------- Text ----------------
        "TextField" => vec![stack_of(vec![
            text_field("sc:tf1", Some("ZephyraUI"), "Type here", true, g.hovers("sc:tf1"), g.holds("sc:tf1")),
            text_field("sc:tf2", None, "Placeholder only", false, g.hovers("sc:tf2"), g.holds("sc:tf2")),
        ])],
        "TextArea" => vec![text_area("sc:ta", AREA_LINES)],
        "AutoSuggestBox" => vec![auto_suggest_box(
            "sc:asb", None, "Search controls", SUGGESTIONS, 0, true, g.hovers("sc:asb"),
        )],
        // ---------------- Status & info ----------------
        "InfoBadge" => vec![row_of(vec![
            info_badge("sc:ib1", "Info", "info", true),
            info_badge("sc:ib2", "Success", "success", true),
            info_badge("sc:ib3", "Warning", "caution", true),
            info_badge("sc:ib4", "Error", "critical", true),
            dot_badge("sc:dot", "critical"),
        ])],
        "InfoBar" => vec![
            info_bar("sc:ibar1", "Heads up", "Something happened.", "info", true),
            info_bar("sc:ibar2", "Done", "Saved successfully.", "success", false),
        ],
        "ProgressBar" => vec![
            progress_bar("sc:pb1", 0.35, "normal"),
            progress_bar("sc:pb2", 0.7, "success"),
        ],
        "ProgressRing" => vec![row_of(vec![
            progress_ring("sc:pr1", "35%", "normal", true),
            progress_ring("sc:pr2", "done", "success", false),
        ])],
        "PageIndicator" => vec![page_indicator("sc:pi", 5, 2)],
        "ToolTip" => vec![tool_tip("sc:tip", "A tooltip with a short hint.")],
        "Toast" => vec![toast("sc:toast", "Saved", "Your changes are live.", "success")],
        // ---------------- Collections ----------------
        "ListView" => vec![list_view("sc:lv", LIST_ITEMS, Some("Alpha"), g.hovered_name(), g.pressed_name())],
        "TableView" => vec![table_view("sc:tv", TABLE_HEADS, TABLE_ROWS)],
        "SwipeView" => vec![swipe_view("sc:sv", SWIPE_PAGES, 0)],
        // ---------------- Menus & toolbars ----------------
        "Menu" => vec![menu("sc:menu", MENU_ENTRIES)],
        "MenuBar" => vec![menu_bar("sc:mb", MENU_BAR_ITEMS, None)],
        // ---------------- Navigation ----------------
        "NavigationView" => vec![stack_of(vec![
            nav_item("sc:nav1", "Home", "home", true, g.hovers("sc:nav1"), g.holds("sc:nav1")),
            nav_item("sc:nav2", "Settings", "settings", false, g.hovers("sc:nav2"), g.holds("sc:nav2")),
        ])],
        "SelectorBar" => vec![selector_bar("sc:sb", SELECTOR_ITEMS, Some("nav"), g.hovered_name(), g.pressed_name())],
        // ---------------- Dialogs ----------------
        "Dialog" => vec![dialog(
            "sc:dialog", "Delete file?", "This cannot be undone.",
            DIALOG_BUTTONS, g.hovered_name(), g.pressed_name(),
        )],
        // ---------------- Settings ----------------
        "SettingCard" => vec![setting_card(
            "sc:sc1", "Notifications", "Get notified about updates",
            toggle_switch("sc:sc1:sw", true, g.hovers("sc:sc1:sw"), g.holds("sc:sc1:sw")),
        )],
        "SettingExpander" => vec![expander(
            "sc:se", "Advanced", g.expanded,
            vec![text_label("sc:se:body", "Less common settings live here.", "text-body")],
        )],
        "Segmented" => vec![segmented("sc:seg", SEG_ITEMS, Some("day"), g.hovered_name(), g.pressed_name())],
        // ---------------- Date & time ----------------
        "DatePicker" => vec![date_picker("sc:dp", MONTH_CELLS, 12)],
        "TimePicker" => vec![time_picker("sc:tp", HOURS, "10:00", MINUTES, "30")],
        // ---------------- Styles ----------------
        "Avatar" => vec![row_of(vec![
            avatar("sc:av1", "ZP", true),
            avatar("sc:av2", "AB", false),
        ])],
        "ScrollableTextArea" => vec![scrollable_text_area("sc:sta", AREA_LINES, 0.4)],
        // ---------------- 明确降级 ----------------
        _ => vec![
            text_label("sc-why", downgrade_reason(page), "text-body"),
            // 放一个真控件，证明页面是活的（不是渲染坏了）。
            row_of(vec![button(
                "sc-ok", "This button really works", None, "standard",
                g.hovers("sc-ok"), g.holds("sc-ok"),
            )]),
        ],
    };

    // 标题/描述/key 都要 `&'static`（进 `static_str` / `with_key`）。
    // catalog 命中时用它的静态字段；未命中时用固定兜底串。
    // ⚠️ **不能 `unwrap_or(page)`** —— `page: &str` 的生命周期比 `&'static` 短，
    //   那样会「借用的数据逃出函数」（E0521）。
    let (title, desc, key) = match catalog::entry(page) {
        Some(e) => (e.title, e.desc, e.page),
        None => ("Unknown control", "", "sc-unknown"),
    };
    let mut kids = body;
    if !desc.is_empty() && page != "Toast" {
        kids.push(text_label("sc-desc", desc, "text-body"));
    }
    Element::new(NodeTag::Container)
        .with_key(key)
        .style("page-showcase")
        .with_children([section("sc-sec", title, vec![section("sc-body", "", kids)])])
}

/// 降级原因（按控件给**具体**理由，不是一句"不支持"）。
fn downgrade_reason(page: &str) -> &'static str {
    match page {
        "Flyout" => "Demo-level: Flyout needs a detached overlay window; core has no overlay/second-window concept. The menu below is the inline equivalent.",
        "Popup" => "Demo-level: Popup needs a modal overlay + focus trap (core has neither). Inline stand-in.",
        "CalendarView" => "Demo-level: a real month grid needs date math + per-cell hit testing; core layout has no grid. DatePicker on the Date & time group shows the date path we DO support.",
        "CalendarDatePicker" => "Demo-level: combines CalendarView + text field; CalendarView is out of reach (see its page).",
        "TitleBar" => "Already real, just not here: the gallery window HAS a title bar (search box + window buttons) at the shell level.",
        "Iconography" => "Already real, just not here: every nav item and card carries a vector icon from core's built-in table.",
        _ => "Not migrated in this pass.",
    }
}

// ------------------------------------------------------------------ 小工具

fn row_of(children: Vec<Element>) -> Element {
    super::pages::row(0, children)
}

fn stack_of(children: Vec<Element>) -> Element {
    super::pages::stack(0, children)
}

// ------------------------------------------------------------------ 静态表

const AREA_LINES: &[&str] = &[
    "A TextArea shows multiple lines of text.",
    "Line two, kept short on purpose.",
    "Line three so the block is visibly taller.",
    "Line four.",
    "Line five.",
];

const SUGGESTIONS: &[&str] = &["Button", "CheckBox", "Slider", "ComboBox"];

const LIST_ITEMS: &[(&str, &str)] = &[("Alpha", "lv:alpha"), ("Beta", "lv:beta"), ("Gamma", "lv:gamma")];

const TABLE_HEADS: &[&str] = &["Control", "Size", "Note"];
const TABLE_ROWS: &[&[&str]] = &[
    &["Button", "32px", "accent variant"],
    &["CheckBox", "20px", "check is a class"],
    &["Slider", "24px", "fill width is the value"],
];

const SWIPE_PAGES: &[(&str, &str)] = &[
    ("Page one", "SwipeView pages horizontally."),
    ("Page two", "Pips show the current page."),
    ("Page three", "Drag is host-driven."),
];

const MENU_ENTRIES: &[(&str, &str)] = &[("Open", "m:open"), ("", ""), ("Save", "m:save")];
const MENU_BAR_ITEMS: &[&str] = &["File", "Edit", "View"];

const SELECTOR_ITEMS: &[(&str, &str)] = &[("nav", "Home"), ("tabs", "Tabs")];

const DIALOG_BUTTONS: &[(&str, &str, bool)] = &[("Cancel", "d:cancel", false), ("Delete", "d:ok", true)];

const SEG_ITEMS: &[(&str, &str)] = &[("day", "Day"), ("week", "Week"), ("month", "Month")];

const HOURS: &[(&str, &str)] = &[("09:00", "09:00"), ("10:00", "10:00"), ("11:00", "11:00")];
const MINUTES: &[(&str, &str)] = &[("00", "00"), ("30", "30"), ("45", "45")];

/// 月历格子：`None` = 空位（月首偏移），`Some(d)` = 该日。
const MONTH_CELLS: &[Option<u8>] = &[
    None, None, Some(1), Some(2), Some(3), Some(4), Some(5),
    Some(6), Some(7), Some(8), Some(9), Some(10), Some(11), Some(12),
    Some(13), Some(14), Some(15), Some(16), Some(17), Some(18), Some(19),
    Some(20), Some(21), Some(22), Some(23), Some(24), Some(25), Some(26),
    Some(27), Some(28), Some(29), Some(30), Some(31),
];
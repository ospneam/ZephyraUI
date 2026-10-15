//! Layout / Styles / Dialogs / DateAndTime（批 G）。
//!
//! 度量出处（RinUI）：
//! - AcrylicBrush：`Styles/AcrylicBrush.qml` —— 半透明亚克力卡。
//! - RoundedCornerOverlay：`Layout/RoundedCornerOverlay.qml` —— 圆角遮罩容器
//!   （RinUI 用它给内容卡裁四角；这里是同规格的圆角边框容器）。
//! - Dialog：`DialogsAndFlyouts/Dialog.qml` —— 居中卡：标题（title 档）+
//!   正文 + 按钮行（DialogButtonBox：主按钮在右）。
//! - DatePicker：`DateAndTime/DatePicker.qml` —— 月份网格：表头（日..六）+
//!   5×7 日期格，选中日 = accent 圆底。
//! - TimePicker：`DateAndTime/TimePicker.qml` —— 时/分两列 + 选中项高亮。
//!
//! # 降级记账
//!
//! - AcrylicBrush 的**真实亚克力材质**（背景模糊 + 噪点）不做：core 的滤镜
//!   挂在节点上、不对着桌面。这里是"半透明卡"的语义降级。
//! - Dialog 的**模态遮罩与焦点圈闭**不做（core 无 overlay）：卡片本身可迁移。
//! - DatePicker / TimePicker 的**滚动选值**不做（core 无滚动）：点选由宿主
//!   按 key 路由（日期格 1..31 都有 key）。

use zephyra_core::element::Element;
use zephyra_core::id::NodeTag;
use zephyra_core::property::PropertyValue;
use zephyra_core::props;

use super::button;

/// 亚克力卡（RinUI AcrylicBrush 的语义降级形态）：半透明卡 + 边框。
pub fn acrylic_card(key: &'static str, children: Vec<Element>) -> Element {
    Element::new(NodeTag::Container)
        .with_key(key)
        .style("acrylic-card")
        .with_children(children)
}

/// 圆角遮罩容器（RinUI RoundedCornerOverlay）：透明底 + 圆角 + 边。
pub fn rounded_corner_overlay(
    key: &'static str,
    radius_class: &'static str,
    children: Vec<Element>,
) -> Element {
    Element::new(NodeTag::Container)
        .with_key(key)
        .style("corner-overlay")
        .style(radius_class)
        .with_children(children)
}

/// 对话框（RinUI Dialog）：标题 + 正文 + 按钮行。
pub fn dialog(
    key: &'static str,
    title: &'static str,
    text: &'static str,
    buttons: &'static [(&'static str, &'static str, bool)],
    hovered: Option<&str>,
    pressed: Option<&str>,
) -> Element {
    let rows: Vec<Element> = buttons
        .iter()
        .map(|(id, label, primary)| {
            button(
                id,
                label,
                None,
                if *primary { "accent" } else { "standard" },
                hovered == Some(*id),
                pressed == Some(*id),
            )
        })
        .collect();
    Element::new(NodeTag::Container)
        .with_key(key)
        .style("dialog")
        .with_children([
            Element::new(NodeTag::Text)
                .with_key("title")
                .with(&props::TEXT, PropertyValue::static_str(title))
                .style("dialog-title"),
            Element::new(NodeTag::Text)
                .with_key("text")
                .with(&props::TEXT, PropertyValue::static_str(text))
                .style("dialog-text"),
            Element::new(NodeTag::Container)
                .with_key("buttons")
                .style("dialog-buttons")
                .with_children(rows),
        ])
}

/// 日期选择器（RinUI DatePicker）：表头 + 5×7 日期格。
///
/// `days` 是当月 1..=31 中哪些天可见（`None` = 空格）。选中态由 `selected`
/// 给（1-based 日号）。
pub fn date_picker(key: &'static str, cells: &'static [Option<u8>], selected: u8) -> Element {
    let heads = ["日", "一", "二", "三", "四", "五", "六"];
    let mut children = vec![Element::new(NodeTag::Container)
        .with_key("heads")
        .style("dp-heads")
        .with_children(
            heads
                .iter()
                .enumerate()
                .map(|(i, h)| {
                    Element::new(NodeTag::Text)
                        .with_key(dp_head_key(i))
                        .with(&props::TEXT, PropertyValue::static_str(h))
                        .style("dp-head")
                })
                .collect::<Vec<_>>(),
        )];
    // 35 格（5 行 × 7 列），手排行。
    for (r, week) in cells.chunks(7).enumerate() {
        let row: Vec<Element> = week
            .iter()
            .enumerate()
            .map(|(c, day)| {
                let mut cell = Element::new(NodeTag::Container)
                    .with_key(dp_cell_key(r, c))
                    .style("dp-cell");
                if let Some(d) = day {
                    let mut filled = Element::new(NodeTag::Container)
                        .with_key("day")
                        .style("dp-day");
                    let mut label = Element::new(NodeTag::Text)
                        .with_key("label")
                        .with(&props::TEXT, PropertyValue::static_str(day_text(*d)))
                        .style("dp-day-label");
                    if *d == selected {
                        filled = filled.style("selected");
                        label = label.style("selected");
                    }
                    filled = filled.with_children([label]);
                    cell = cell.with_children([filled]);
                }
                cell
            })
            .collect();
        children.push(
            Element::new(NodeTag::Container)
                .with_key(dp_row_key(r))
                .style("dp-row")
                .with_children(row),
        );
    }
    Element::new(NodeTag::Container)
        .with_key(key)
        .style("date-picker")
        .with_children(children)
}

/// 时间选择器（RinUI TimePicker）：时/分两列 + 选中项高亮。
///
/// 滚动列表的降级：**可见窗口由宿主给**（`&'static [(&'static str, &str)]`
/// 的 `(key, 文字)` 切片），控件只画窗口 + 高亮 `hour` / `minute` 指定的那项。
/// 谁来滚、滚到哪（真做滚动后窗口要跟着滚动位置走）是宿主的事 —— 这也让控件
/// 不必为 key 做 `format!`（`view()` 每帧跑一次，泄漏会无界增长）。
pub fn time_picker(
    key: &'static str,
    hours: &'static [(&'static str, &'static str)],
    hour: &'static str,
    minutes: &'static [(&'static str, &'static str)],
    minute: &'static str,
) -> Element {
    let col =
        |id: &'static str, entries: &'static [(&'static str, &'static str)], sel: &'static str| {
            let items: Vec<Element> = entries
                .iter()
                .map(|(item_key, label)| tp_item(item_key, label, *label == sel))
                .collect();
            Element::new(NodeTag::Container)
                .with_key(id)
                .style("tp-col")
                .with_children(items)
        };
    Element::new(NodeTag::Container)
        .with_key(key)
        .style("time-picker")
        .with_children([col("hours", hours, hour), col("minutes", minutes, minute)])
}

// ---------------------------------------------------------------- 内部

fn tp_item(item_key: &'static str, text: &'static str, selected: bool) -> Element {
    let mut el = Element::new(NodeTag::Container)
        .with_key(item_key)
        .style("tp-item");
    let mut label = Element::new(NodeTag::Text)
        .with_key("label")
        .with(&props::TEXT, PropertyValue::static_str(text))
        .style("tp-label");
    if selected {
        el = el.style("selected");
        label = label.style("selected");
    }
    el.with_children([label])
}

fn dp_head_key(i: usize) -> &'static str {
    [
        "dph:0", "dph:1", "dph:2", "dph:3", "dph:4", "dph:5", "dph:6",
    ][i.min(6)]
}

fn dp_row_key(r: usize) -> &'static str {
    ["dpr:0", "dpr:1", "dpr:2", "dpr:3", "dpr:4"][r.min(4)]
}

fn dp_cell_key(r: usize, c: usize) -> &'static str {
    const KEYS: [&str; 35] = [
        "dpc:00", "dpc:01", "dpc:02", "dpc:03", "dpc:04", "dpc:05", "dpc:06", "dpc:10", "dpc:11",
        "dpc:12", "dpc:13", "dpc:14", "dpc:15", "dpc:16", "dpc:20", "dpc:21", "dpc:22", "dpc:23",
        "dpc:24", "dpc:25", "dpc:26", "dpc:30", "dpc:31", "dpc:32", "dpc:33", "dpc:34", "dpc:35",
        "dpc:36", "dpc:40", "dpc:41", "dpc:42", "dpc:43", "dpc:44", "dpc:45", "dpc:46",
    ];
    KEYS[(r * 7 + c).min(34)]
}

/// 日期数字的静态文本（1..31）。内容是编译期常量。
fn day_text(d: u8) -> &'static str {
    const T: [&str; 31] = [
        "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12", "13", "14", "15", "16",
        "17", "18", "19", "20", "21", "22", "23", "24", "25", "26", "27", "28", "29", "30", "31",
    ];
    T[(d as usize).saturating_sub(1).min(30)]
}

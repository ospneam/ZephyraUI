//! 对话框与选择器：Dialog / DatePicker / TimePicker / ToolTip / AcrylicCard。
//!
//! # ⚠ 这一页全是**静态摆件**
//!
//! core 没有浮层（`Layer::Popup` 有类型但没有生产调用方，`PushClip` 同理），
//! 所以 Dialog 在这里是**页面里的一块内容**，不是盖在上面的弹窗。
//!
//! 别把它做成"点一下弹出" —— 那会做出一个"点击后页面内容被替换掉"的东西，
//! 而它看起来像弹窗、行为不是，是最难查的一类 bug。真要做浮层，先做
//! `Layer::Popup` 那条路。

use zephyra_core::element::Element;

use super::Gallery;
use crate::app::pages;
use zephyra_ui::controls;

/// TimePicker 的可见窗口（`(key, 文字)`）。
const HOURS: &[(&str, &str)] = &[("tph:13", "13"), ("tph:14", "14"), ("tph:15", "15")];
const MINUTES: &[(&str, &str)] = &[("tpm:00", "00"), ("tpm:05", "05"), ("tpm:10", "10")];

/// DatePicker 的星期头。
///
/// 控件**不**提供默认表头（文案归应用，见 `AGENTS.md` 边界 3），所以这里
/// 必须自己给。ASCII 单字母是各日历控件的通行写法，也与本页其余文案的
/// 语言一致 —— 控件层原来硬编码的是「日一二三四五六」，切英文时表头不会跟着切。
const WEEK_HEADS: &[&str] = &["S", "M", "T", "W", "T", "F", "S"];

/// DatePicker 的格子（`None` = 空位）。7 列 × 3 周够看形态。
/// # 为什么是 `pub`
///
/// `Gallery::click` 的 `dpc:` 分支要把格子序号换算成**日期值**——
/// 查这张表是唯一真相（页面与分派共享一份日历）。
pub const CALENDAR: &[Option<u8>] = &[
    Some(1),
    Some(2),
    Some(3),
    Some(4),
    Some(5),
    Some(6),
    Some(7),
    Some(8),
    Some(9),
    Some(10),
    Some(11),
    Some(12),
    Some(13),
    Some(14),
    Some(15),
    Some(16),
    Some(17),
    Some(18),
    Some(19),
    Some(20),
    Some(21),
];

pub fn page(g: &Gallery) -> Element {
    pages::page_root(
        "dialogs",
        "page-dialogs",
        vec![
            pages::section(
                "sec-dialog",
                "Dialog",
                vec![zephyra_ui::controls::caption(
                    "dlg-note",
                    "Rendered inline — the core has no overlay layer yet.",
                )],
            ),
            pages::section(
                "sec-picker",
                "Pickers",
                vec![
                    pages::demo(
                        // 选中日走 `Gallery::date_sel`：格子（`dpc:RC`）可点，
                        // `click` 的 `dpc:` 分支把格子序号换算成 CALENDAR 里的
                        // 日期值回写。选不存在的格（None）不响应——那与
                        // 「日历上空白格本来就不是日期」一致。
                        "Date",
                        controls::date_picker("dp", WEEK_HEADS, CALENDAR, g.date_sel),
                    ),
                    pages::demo(
                        // 小时/分钟各走一个字段：两个列是**独立**的选择器，
                        // 共用一个下标会出现「选了 15 点，分钟跳到 05」的
                        // 幽灵联动。
                        "Time",
                        controls::time_picker("tp", HOURS, g.time_hour, MINUTES, g.time_minute),
                    ),
                ],
            ),
            pages::section(
                "sec-tip",
                "Tool tip",
                vec![pages::demo(
                    "Sample",
                    controls::tool_tip("tip", "A tooltip"),
                )],
            ),
            pages::section(
                "sec-acrylic",
                "Translucent card",
                vec![controls::acrylic_card(
                    "acrylic",
                    vec![
                        controls::text_label("acr-t", "Translucent", "text-body-strong"),
                        controls::caption("acr-d", "See the effects page."),
                    ],
                )],
            ),
        ],
    )
}

/// 对话框按钮表被 `Gallery::click` 用到时保持可达（见下方 `dlg` 构造）。
const _: () = {};

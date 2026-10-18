//! 状态与信息：Badge / InfoBar / Toast / Progress / Busy。
//!
//! # 三类语义，一页看完
//!
//! | 控件 | 回答的问题 |
//! |---|---|
//! | Badge / InfoBar / Toast | 「发生了什么」 |
//! | ProgressBar / ProgressRing / Busy | 「进行到哪了」 |
//!
//! 分节而不是平铺：这几类的**时长**完全不同 —— 瞬时 / 持续。
//! 混在一列里扫的时候会把"常驻的进度条"误当成"提示消息"。
//!
//! ⚠ `PageIndicator` 搬去了 [`crate::app::pages::paging`]（与 SwipeView 同族）：
//! 它回答的「一共有多少页」只在分页语境下成立，而参考页无滚动，一屏装不下
//! 就得靠分族腾地方。

use zephyra_core::element::Element;

use super::Gallery;
use crate::app::pages;
use zephyra_ui::controls;

/// Toast 的三条内容（`(title, text)`）。
const TOASTS: &[(&str, &str)] = &[
    ("Saved", "Your changes are on disk."),
    ("Offline", "No network connection."),
    ("Synced", "Everything is up to date."),
];

pub fn page(g: &Gallery) -> Element {
    pages::page_root(
        "status",
        "page-status",
        vec![
            pages::section(
                "sec-badge",
                "Badge",
                vec![pages::demo_many(
                    "Severities",
                    vec![
                        controls::info_badge("badge-info", "12", "info", true),
                        controls::info_badge("badge-ok", "OK", "success", true),
                        controls::info_badge("badge-warn", "!", "caution", true),
                        controls::info_badge("badge-err", "×", "error", true),
                    ],
                )],
            ),
            pages::section(
                "sec-bar",
                "Info bar",
                // 关闭钮（`bar:close`）真的可点：`click` 把 `infobar_open`
                // 置 false，条件渲染把可关的那条收掉（CLOSED：不占位）。
                // 下面那个 pill 是**恢复**入口——没有它，关掉之后这一节
                // 就变成「永远少一条」而无法复现。
                vec![
                    if g.infobar_open {
                        Some(controls::info_bar(
                            "infobar",
                            "Update available",
                            "Version 0.2 is ready to install.",
                            "info",
                            true,
                        ))
                    } else {
                        None
                    },
                    Some(controls::info_bar(
                        "infobar-err",
                        "Disk almost full",
                        "Less than 100 MB remaining.",
                        "error",
                        false,
                    )),
                    Some(pages::demo(
                        "Restore",
                        controls::pill_button(
                            "infobar-restore",
                            "Show info bar",
                            None,
                            false,
                            g.hovers("infobar-restore"),
                            g.holds("infobar-restore"),
                        ),
                    )),
                ]
                .into_iter()
                .flatten()
                .collect(),
            ),
            pages::section(
                "sec-toast",
                "Toast",
                vec![pages::page_root(
                    "toasts",
                    "card-grid",
                    TOASTS
                        .iter()
                        .enumerate()
                        .map(|(i, (t, d))| {
                            controls::toast(toast_key(i), t, d, ["info", "caution", "success"][i])
                        })
                        .collect(),
                )],
            ),
            pages::section(
                "sec-progress",
                "Progress",
                vec![
                    pages::demo(
                        "Bar",
                        controls::progress_bar("progress", g.volume, "running"),
                    ),
                    pages::demo_many(
                        "States",
                        vec![
                            controls::progress_bar("progress-p", 0.4, "paused"),
                            controls::progress_bar("progress-e", 0.8, "error"),
                        ],
                    ),
                    pages::demo(
                        "Ring",
                        controls::progress_ring("ring", "62%", "running", true),
                    ),
                    pages::demo("Busy", controls::busy_indicator("busy", 1)),
                ],
            ),
        ],
    )
}

/// Toast 的 key（不能 `format!`：见 `pages::basics::spin_text` 的理由）。
const fn toast_key(i: usize) -> &'static str {
    match i {
        0 => "toast:0",
        1 => "toast:1",
        _ => "toast:2",
    }
}

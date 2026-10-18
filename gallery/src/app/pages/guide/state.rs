//! 状态管理：哪些控件真的接了状态，哪些是展示型。
//!
//! # 这一章教什么
//!
//! 开发者最常踩的坑：**点了没反应**。而 core 的当前边界是——只有部分控件
//! 接了状态。这一章把边界画清楚，让开发者知道「能交互」与「只是展示」的
//! 分别在哪里，不用靠试。
//!
//! # 为什么「点了没反应」是最难查的 bug
//!
//! 它在画面上完全正常：控件画出来了、hover 有反馈、就是点了不改任何东西。
//! 而「core 还没把滑条做成输入设备」这件事，从画面上看不出来。
//! 所以这里**明说**，并指向 `docs/dev-log/README.md` 的记账。

use zephyra_core::element::Element;
use zephyra_ui::controls;

use crate::app::pages;
use crate::app::Gallery;

/// state 章 ComboBox 的当前值（查表，越界回落第一项——理由同 basics 的
/// `combo_text`：`demo_combo` 是公开字段，最坏情况是「显示不对」不是「崩」）。
fn combo_text(i: usize) -> &'static str {
    match i {
        0 => "One",
        1 => "Two",
        _ => "Three",
    }
}

pub fn page(g: &Gallery) -> Element {
    pages::page_root(
        "state",
        "page-state",
        vec![
            controls::text_label(
                "state-blurb",
                "Wired controls change state on click. Display-only controls don't — that's a core boundary, not a bug.",
                "text-body",
            ),
            // ---- 真的接了状态 ----
            //
            // 开发者看到这几行会想「点一下试试」。点下去界面确实会变 ——
            // 而它们改的状态**只影响本节**，不会污染别的页。
            pages::section(
                "state-wired",
                "Wired to state",
                vec![
                    pages::demo("CheckBox", controls::check_box(
                        "state-check",
                        "Toggle me",
                        g.demo_check,
                        g.hovers("state-check"),
                        g.holds("state-check"),
                    )),
                    pages::demo("Toggle", controls::toggle_switch(
                        "state-switch",
                        g.switch_on,
                        g.hovers("state-switch"),
                        g.holds("state-switch"),
                    )),
                    pages::demo("Segmented", controls::segmented(
                        "state-seg",
                        &[
                            (crate::app::DEMO_SEGS[0], "A"),
                            (crate::app::DEMO_SEGS[1], "B"),
                            (crate::app::DEMO_SEGS[2], "C"),
                        ],
                        Some(g.demo_seg),
                        g.hovered_name(),
                        g.pressed_name(),
                    )),
                    pages::demo("ComboBox", controls::combo_box(
                        "state-combo",
                        combo_text(g.demo_combo),
                        g.hovers("state-combo"),
                        g.holds("state-combo"),
                    )),
                ],
            ),
            // ---- 展示型：core 还没做成输入设备 ----
            //
            // ⚠ 这不是漏接。core 的当前边界就是这些控件只画不收。
            // 开发者如果给它们编一个「假动作」，症状是「点了有反应、切页就没了」——
            // 那比「点了没反应」更难查。
            pages::section(
                "state-display",
                "Display-only (core boundary)",
                vec![
                    pages::demo("Slider", controls::slider(
                        "state-slider",
                        0.5,
                        None,
                        g.hovers("state-slider"),
                        g.holds("state-slider"),
                    )),
                    pages::demo("ProgressBar", controls::progress_bar(
                        "state-progress",
                        0.65,
                        "running",
                    )),
                    pages::demo("ToolTip", controls::tool_tip(
                        "state-tip",
                        "I appear on hover",
                    )),
                ],
            ),
        ],
    )
}

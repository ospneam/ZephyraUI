//! 选择类：CheckBox / RadioButton / ToggleSwitch / Segmented。
//!
//! # 为什么这四样要同页
//!
//! 它们回答的是同一个问题——「在一组选项里表示状态」——而**区别只在形状**：
//! 多选方框、单选圆点、开关、分段。把它们分到四页会让人以为这是四种
//! 不相干的能力，而它们其实是同一个能力的四种视觉编码。
//!
//! 分段控件特意排在最后：它长得最像按钮，一眼看过去容易以为该归
//! 「基础输入」那一页 —— 放在这页末尾，两边都照顾到。

use zephyra_core::element::Element;

use super::Gallery;
use crate::app::pages;
use zephyra_ui::controls;

/// 分段控件的项（`(id, 标签)`）。
///
/// id 与 `Gallery::click` 里的三条 `seg:*` 分支一一对应（门禁盯着）。
const SEGS: &[(&str, &str)] = &[
    ("seg:grid", "Grid"),
    ("seg:list", "List"),
    ("seg:card", "Card"),
];

/// 单选组的选项。
///
/// # 为什么 id 是 `radio:*` 而不是复用复选框的 `check:*`
///
/// 单选与多选在**键上**就分开，是因为 `Gallery::click` 里它们是两条不同的分支：
/// 复选框是 `self.checked[i] = !self.checked[i]`（取反），单选是 `self.radio = i`
/// （赋值）。共用一套 id 会让两个分支抢同一个键，而 `match` 只走第一条 —— 症状是
/// "单选那一列点不动"，而复选框一切正常。
///
/// 门禁 `segment_keys_are_all_wired` 与 `every_interactive_key_has_a_click_arm`
/// 一起盯着这一层。
pub const RADIO_ITEMS: &[(&str, &str)] = &[
    ("radio:a", "Option A"),
    ("radio:b", "Option B"),
    ("radio:c", "Option C"),
];

pub fn page(g: &Gallery) -> Element {
    pages::page_root(
        "toggles",
        "page-toggles",
        vec![
            pages::section(
                "sec-check",
                "Checkbox",
                vec![pages::demo_many(
                    "Multiple",
                    vec![
                        controls::check_box(
                            "check:alpha",
                            "Enable feature",
                            g.checked[0],
                            g.hovers("check:alpha"),
                            g.holds("check:alpha"),
                        ),
                        controls::check_box(
                            "check:beta",
                            "Send analytics",
                            g.checked[1],
                            g.hovers("check:beta"),
                            g.holds("check:beta"),
                        ),
                        controls::check_box(
                            "check:gamma",
                            "Beta channel",
                            g.checked[2],
                            g.hovers("check:gamma"),
                            g.holds("check:gamma"),
                        ),
                    ],
                )],
            ),
            pages::section(
                "sec-radio",
                "Radio",
                vec![pages::demo_many(
                    "Single",
                    RADIO_ITEMS
                        .iter()
                        .enumerate()
                        .map(|(i, (id, label))| {
                            controls::radio_button(
                                id,
                                label,
                                g.radio == i,
                                g.hovers(id),
                                g.holds(id),
                            )
                        })
                        .collect(),
                )],
            ),
            pages::section(
                "sec-switch",
                "Switch",
                vec![pages::demo(
                    "One state",
                    controls::toggle_switch(
                        "switch:master",
                        g.switch_on,
                        g.hovers("switch:master"),
                        g.holds("switch:master"),
                    ),
                )],
            ),
            pages::section(
                "sec-seg",
                "Segmented",
                vec![pages::demo(
                    "Pick one",
                    controls::segmented(
                        "segbar",
                        SEGS,
                        Some(g.seg),
                        g.hovered_name(),
                        g.pressed_name(),
                    ),
                )],
            ),
        ],
    )
}

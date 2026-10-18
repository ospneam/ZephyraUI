//! 基础输入：Button / TextField / ComboBox / SpinBox / Slider / RoundButton。
//!
//! # 这一页的编排原则
//!
//! **同族控件放一节，节内一行一族。** 而每行是「左标签 + 右控件」——
//! 标签固定一小截，于是所有控件的**左边**对齐，扫第二列时不会因为第一个
//! 控件特别宽而全部错开。
//!
//! 变体（variant）不单独占一行，而是**并排**摆在一行里：`standard` /
//! `accent` / `subtle` 摆成一行才看得出它们是同一控件的三个档；竖排三次
//! 反而会让人以为是三个不同的东西。

use zephyra_core::element::Element;

use zephyra_ui::controls;

use crate::app::pages;
use crate::app::Gallery;
use crate::app::COMBO_OPTIONS;

pub fn page(g: &Gallery) -> Element {
    pages::page_root(
        "basics",
        "page-basics",
        vec![
            pages::section(
                "sec-btns",
                "Button",
                vec![
                    pages::demo_many(
                        "Variants",
                        vec![
                            controls::button(
                                "btn-standard",
                                "Standard",
                                None,
                                "standard",
                                g.hovers("btn-standard"),
                                g.holds("btn-standard"),
                            ),
                            controls::button(
                                "btn-accent",
                                "Accent",
                                None,
                                "accent",
                                g.hovers("btn-accent"),
                                g.holds("btn-accent"),
                            ),
                            controls::button(
                                "btn-subtle",
                                "Subtle",
                                None,
                                "subtle",
                                g.hovers("btn-subtle"),
                                g.holds("btn-subtle"),
                            ),
                        ],
                    ),
                    pages::demo_many(
                        "With icon",
                        vec![
                            controls::button(
                                "btn-icon",
                                "Send",
                                Some("pen"),
                                "accent",
                                g.hovers("btn-icon"),
                                g.holds("btn-icon"),
                            ),
                            controls::pill_button(
                                "pill-a",
                                "Pill",
                                Some("star"),
                                g.checked[1],
                                g.hovers("pill-a"),
                                g.holds("pill-a"),
                            ),
                        ],
                    ),
                    pages::demo_many(
                        "Round",
                        vec![
                            controls::round_button(
                                "round-a",
                                "",
                                Some("chevron-up"),
                                false,
                                g.hovers("round-a"),
                                g.holds("round-a"),
                            ),
                            controls::round_button(
                                "round-b",
                                "",
                                Some("close"),
                                true,
                                g.hovers("round-b"),
                                g.holds("round-b"),
                            ),
                        ],
                    ),
                ],
            ),
            pages::section(
                "sec-text",
                "Text input",
                vec![
                    pages::demo(
                        "TextField",
                        controls::text_field(
                            "field-name",
                            None,
                            "Your name",
                            false,
                            g.hovers("field-name"),
                            g.holds("field-name"),
                        ),
                    ),
                    pages::demo(
                        "TextArea",
                        controls::text_area(
                            "area-notes",
                            &["Multi-line notes", "second line", "third line"],
                        ),
                    ),
                ],
            ),
            pages::section(
                "sec-choice",
                "Value pickers",
                vec![
                    pages::demo(
                        "ComboBox",
                        controls::combo_box(
                            "combo-material",
                            // 这里取的正是 `Gallery::click` 推进下标时跳的那张表
                            // —— 页面自己定义一份就会被点的时候绕回自己，
                            // 而画面上完全正常。
                            combo_text(g.combo),
                            g.hovers("combo-material"),
                            g.holds("combo-material"),
                        ),
                    ),
                    pages::demo(
                        "SpinBox",
                        controls::spin_box(
                            "spin-count",
                            spin_text(g.spin),
                            g.hovers("spin-count"),
                            g.holds("spin-count"),
                            g.hovers("spin:up"),
                            g.holds("spin:up"),
                            g.hovers("spin:down"),
                            g.holds("spin:down"),
                        ),
                    ),
                    pages::demo(
                        "Slider",
                        controls::slider(
                            "slider-volume",
                            g.volume,
                            Some(slider_text(g.volume)),
                            g.hovers("slider-volume"),
                            g.holds("slider-volume"),
                        ),
                    ),
                    pages::demo("ColorPicker", controls::color_picker("picker-color", 4)),
                ],
            ),
        ],
    )
}

/// ComboBox 当前值的文字。
///
/// # 为什么查表而不是 `COMBO_ITEMS[i]`
///
/// 那样写要一次**运行期下标**，而 `COMBO_ITEMS[i]` 在越界时 panic —— 而
/// `Gallery::combo` 是公开可写的字段，改代码的人很容易把它设成 99。
/// 这里让越界回落到第一项，于是最坏情况是"显示得不对"而不是"整页崩"。
fn combo_text(i: usize) -> &'static str {
    COMBO_OPTIONS.get(i).copied().unwrap_or("Mica")
}

/// SpinBox 的显示文本。
///
/// # 为什么用查表而不是 `format!`
///
/// `view()` 每帧重建整棵树，任何运行期拼出来的字符串都会变成泄漏
/// （`Box::leak` 每帧一次，见 `element.rs` 的记录）。所以这里是一张
/// 小表 + 越界回落，而不是 `format!("{}", n)`。
fn spin_text(n: i32) -> &'static str {
    match n {
        0 => "0",
        1 => "1",
        2 => "2",
        3 => "3",
        4 => "4",
        5 => "5",
        6 => "6",
        7 => "7",
        8 => "8",
        9 => "9",
        10 => "10",
        _ => "42",
    }
}

/// 滑条右侧的数值文本。同样是查表（理由见 [`spin_text`]）。
fn slider_text(v: f32) -> &'static str {
    if v < 0.2 {
        "0"
    } else if v < 0.4 {
        "25"
    } else if v < 0.6 {
        "50"
    } else if v < 0.8 {
        "75"
    } else {
        "100"
    }
}

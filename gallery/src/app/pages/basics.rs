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
            // 触发类按钮（`act:*`）的反馈**不**在这里画：那份状态唯一的视图
            // 在外壳页头（`shell.rs` 的 `last-action`），全路由可见。放这里
            // 会变成第二个真相 —— 而且「点了没反应」的判断不该因页面而异。
            pages::section(
                "sec-btns",
                "Button",
                vec![
                    pages::demo_many(
                        "Variants",
                        vec![
                            controls::button(
                                "act:standard",
                                "Standard",
                                None,
                                "standard",
                                g.hovers("act:standard"),
                                g.holds("act:standard"),
                            ),
                            controls::button(
                                "act:accent",
                                "Accent",
                                None,
                                "accent",
                                g.hovers("act:accent"),
                                g.holds("act:accent"),
                            ),
                            controls::button(
                                "act:subtle",
                                "Subtle",
                                None,
                                "subtle",
                                g.hovers("act:subtle"),
                                g.holds("act:subtle"),
                            ),
                        ],
                    ),
                    pages::demo_many(
                        "With icon / round",
                        vec![
                            controls::button(
                                "act:send",
                                "Send",
                                Some("pen"),
                                "accent",
                                g.hovers("act:send"),
                                g.holds("act:send"),
                            ),
                            controls::pill_button(
                                "pill-a",
                                "Pill",
                                Some("star"),
                                g.pill_on,
                                g.hovers("pill-a"),
                                g.holds("pill-a"),
                            ),
                            // round-a / round-b 并排展示「普通 / checked」两种外观
                            // （不是开关，见门禁白名单）。与上两个按钮同一行：
                            // 参考页高度固定（core 无滚动），多一行会把最下面的
                            // 色板推出客户区 —— `hit_test_agrees_with_the_painted_rects`
                            // 会因「画面上被吃掉、点不到」红。
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
                // ⚠ 两个都是**展示型**：core 没有文本输入（无键盘事件路由、
                // 无光标），画的是静态内容。门禁白名单里有它们；
                // state 章把这条边界教给开发者。
                //
                // 同一行而不是两行：参考页高度固定（无滚动），字段与多行框各占
                // 一行会把这一族的色板推出客户区（真实字体下实测）。
                vec![pages::demo_many(
                    "Text input (display)",
                    vec![
                        controls::text_field(
                            "field-name",
                            None,
                            "Your name",
                            false,
                            g.hovers("field-name"),
                            g.holds("field-name"),
                        ),
                        controls::text_area(
                            "area-notes",
                            &["Multi-line notes", "second line", "third line"],
                        ),
                    ],
                )],
            ),
            pages::section(
                "sec-choice",
                "Value pickers",
                vec![
                    // ComboBox 与 SpinBox 同一行：两者都是「选一个值」，
                    // 而参考页高度固定（无滚动）—— 各占一行会把下面推出客户区。
                    pages::demo_many(
                        "Value",
                        vec![
                            controls::combo_box(
                                // 这里取的正是 `Gallery::click` 推进下标时跳的那张表
                                // —— 页面自己定义一份就会被点的时候绕回自己，
                                // 而画面上完全正常。
                                "combo-material",
                                combo_text(g.combo),
                                g.hovers("combo-material"),
                                g.holds("combo-material"),
                            ),
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
                        ],
                    ),
                    pages::demo(
                        // ⚠ 展示型。core 的 InputState 没有连续指针位置，
                        // 拖动无法表达 —— 点了不滑是**正确的**。门禁
                        // `every_interactive_key_is_wired_or_whitelisted`
                        // 的白名单里有它；state 章把这条边界教给开发者。
                        "Slider (display)",
                        controls::slider(
                            "slider-volume",
                            g.volume,
                            Some(slider_text(g.volume)),
                            g.hovers("slider-volume"),
                            g.holds("slider-volume"),
                        ),
                    ),
                    pages::demo(
                        "ColorPicker",
                        controls::color_picker("picker-color", g.color_idx),
                    ),
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
/// 小表，而不是 `format!("{}", n)`。
///
/// # 为什么值域是 0..=20 而不是 0..=100
///
/// 表的长度由**值域**决定：零分配要求每个可能的值都有静态文本，100 项
/// 的表里 90 项永远不会被点到。而「点 ＋ 显示不变」正是本轮要消灭的
/// 失效类别——`spin_text` 曾只映射 0..=10、其余回落 `"42"`，默认值
/// 42 上点 ＋/− 文本纹丝不动，看起来就像按钮没接线。
/// 值域与表**同源**：`click` 的 `spin:up/down` 边界、默认值、这张表，
/// 三处必须同步（门禁 `every_interactive_key_is_wired_or_whitelisted`
/// 通过「点击后树指纹变化」钉住这条）。
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
        11 => "11",
        12 => "12",
        13 => "13",
        14 => "14",
        15 => "15",
        16 => "16",
        17 => "17",
        18 => "18",
        19 => "19",
        _ => "20",
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

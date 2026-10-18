//! 设置页：主题 / 音量 / 语言。
//!
//! # 它是唯一直接反映应用状态的页面
//!
//! 其它页面的控件多半是"演示"，只有这里的每一项都**真的在改什么**：
//! 主题切换会改整棵树的颜色、音量拖动会改数字、语言切换会改全屏文案。
//!
//! 所以这一页也是"状态有没有接上"的活检查 —— 任何一项点了没反应，
//! 那不是控件坏了，是 `Gallery::click` 里少了一条分支。

use zephyra_core::element::Element;

use super::Gallery;
use crate::app::pages;
use zephyra_ui::controls;

/// 语言分段（id 必须与 `SUPPORTED` 一致，见 `Gallery::click`）。
const LANGS: &[(&str, &str)] = &[("en", "English"), ("zh-Hans", "中文"), ("ja", "日本語")];

pub fn page(g: &Gallery) -> Element {
    pages::page_root(
        "settings",
        "page-settings",
        vec![
            pages::section(
                "sec-theme",
                "Appearance",
                vec![controls::setting_card(
                    "set-theme",
                    g.tr("set.theme.title"),
                    g.tr("set.theme.desc"),
                    controls::toggle_switch(
                        "switch:master",
                        g.wants_dark,
                        g.hovers("switch:master"),
                        g.holds("switch:master"),
                    ),
                )],
            ),
            pages::section(
                "sec-notify",
                "Notifications",
                vec![
                    controls::setting_card(
                        "set-notify",
                        g.tr("set.notify.title"),
                        g.tr("set.notify.desc"),
                        controls::toggle_switch(
                            "switch:notify",
                            g.notify_on,
                            g.hovers("switch:notify"),
                            g.holds("switch:notify"),
                        ),
                    ),
                    controls::setting_expander(
                        "set-adv",
                        g.tr("set.adv.title"),
                        g.tr("set.adv.hwaccel"),
                        g.expanded,
                        vec![
                            controls::setting_item(
                                "set-adv-log",
                                "sliders",
                                g.tr("set.adv.logging"),
                                "Diagnostic logging",
                            ),
                            controls::setting_item(
                                "set-adv-hw",
                                "camera",
                                g.tr("set.adv.hwaccel"),
                                "Hardware acceleration",
                            ),
                            controls::setting_item(
                                "set-adv-keys",
                                "menu",
                                g.tr("set.adv.shortcuts"),
                                "Keyboard shortcuts",
                            ),
                        ],
                    ),
                ],
            ),
            pages::section(
                "sec-audio",
                "Volume",
                vec![pages::demo(
                    "Level",
                    controls::slider(
                        "slider-volume",
                        g.volume,
                        Some(volume_text(g.volume)),
                        g.hovers("slider-volume"),
                        g.holds("slider-volume"),
                    ),
                )],
            ),
            pages::section(
                "sec-lang",
                "Language",
                vec![pages::demo(
                    "Interface",
                    controls::segmented(
                        "langbar",
                        LANGS,
                        Some(g.locale()),
                        g.hovered_name(),
                        g.pressed_name(),
                    ),
                )],
            ),
        ],
    )
}

/// 音量数值的显示文本（查表，理由见 `pages::basics::spin_text`）。
fn volume_text(v: f32) -> &'static str {
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

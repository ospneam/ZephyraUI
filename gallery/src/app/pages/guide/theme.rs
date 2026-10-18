//! 主题与国际化：暗/亮、Mica、三语切换。
//!
//! # 这一章教什么
//!
//! 开发者会问三件事：主题怎么切、语言怎么切、Mica 半透明怎么开。这一章把
//! 三个开关摆在一起，每个背后是一段可以直接照抄的接线。
//!
//! # 为什么主题走「意图」而不是直接引用
//!
//! `&Theme` 由宿主持有（窗口里它是 `&'static`）。View 只发意图
//! （`wants_dark`），宿主每帧对账后执行 `set_theme` —— 于是「想切的」与
//! 「在用的」永远可分开观测，不会出现「点了没反应也不知道为什么」。
//! 意图走一个 `Arc<AtomicU8>` 共享格，View 写、Bootstrap 读。

use zephyra_core::element::Element;
use zephyra_ui::controls;

use crate::app::pages;
use crate::app::{Gallery, SUPPORTED};

pub fn page(g: &Gallery) -> Element {
    pages::page_root(
        "theme",
        "page-theme",
        vec![
            controls::text_label(
                "theme-blurb",
                "Theme is an intent, not a reference. Language negotiates from system prefs, then manual choice wins.",
                "text-body",
            ),
            // ---- 暗/亮 ----
            //
            // 开发者点这个开关，看到的不是「颜色变了」—— 而是「令牌值变了」。
            // 颜色全在 `.zstyle` 的 `@令牌` 里，所以切换主题不需要碰源码。
            pages::section(
                "theme-darklight",
                "Dark / Light",
                vec![pages::demo(
                    "Toggle theme",
                    controls::toggle_switch(
                        "theme-toggle",
                        g.wants_dark,
                        g.hovers("theme-toggle"),
                        g.holds("theme-toggle"),
                    ),
                )],
            ),
            // ---- 三语 ----
            //
            // 语言协商：系统偏好 → 手动选择。`Gallery::tr` 查文案表，
            // 三级回退（当前语言 → 语言段 → 默认语言）。开发者切语言时
            // 不需要重启 —— 下一帧整棵树就按新语言重建。
            //
            // ⚠ 语言标签必须 `&'static`：`view()` 每帧重建树，运行期拼字符串
            // 会每帧泄漏。所以这里用 `SUPPORTED` 常量表，不用 `format!`。
            pages::section(
                "theme-i18n",
                "Language (en / zh-Hans / ja)",
                vec![
                    pages::demo("English", controls::pill_button(
                        "lang:en",
                        "English",
                        None,
                        g.lang == 0,
                        g.hovers("lang:en"),
                        g.holds("lang:en"),
                    )),
                    pages::demo("中文", controls::pill_button(
                        "lang:zh-Hans",
                        "中文",
                        None,
                        g.lang == 1,
                        g.hovers("lang:zh-Hans"),
                        g.holds("lang:zh-Hans"),
                    )),
                    pages::demo("日本語", controls::pill_button(
                        "lang:ja",
                        "日本語",
                        None,
                        g.lang == 2,
                        g.hovers("lang:ja"),
                        g.holds("lang:ja"),
                    )),
                ],
            ),
            // ---- Mica 半透明标题栏 ----
            //
            // 「我想要 Mica 外观」是应用层的表达；「平台能不能开」由平台层回答。
            // 所以这里无条件发 `.mica`：平台不支持时，半透明标题栏叠在不透明
            // clear 上仍是可读的半透明色 —— 降级不崩。
            pages::section(
                "theme-mica",
                "Mica backdrop",
                vec![pages::demo(
                    "Toggle Mica",
                    controls::toggle_switch(
                        "theme-mica",
                        g.mica,
                        g.hovers("theme-mica"),
                        g.holds("theme-mica"),
                    ),
                )],
            ),
            // ---- 当前语言的自称 ----
            //
            // 开发者会问「当前是哪门语言」。答案是：自称不进文案表
            // （任何语言的用户看到的都是同一组「English / 中文 / 日本語」），
            // 它直接来自 `LANG_SELFS` 常量。
            pages::section(
                "theme-current",
                "Current locale",
                vec![pages::demo(
                    "You are here",
                    controls::text_label(
                        "theme-locale",
                        SUPPORTED[g.lang.min(SUPPORTED.len() - 1)],
                        "text-body-strong",
                    ),
                )],
            ),
        ],
    )
}

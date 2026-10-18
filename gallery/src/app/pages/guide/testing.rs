//! 测试与出图：这个 repo 怎么自验。
//!
//! # 这一章教什么
//!
//! 开发者改完代码，怎么知道没搞坏？这个 repo 的回答是**三层自验**，从快到慢：
//!
//! | 层 | 命令 | 防什么 |
//! |---|---|---|
//! | 样式 | `cargo run -p zstylec -- check <file.zstyle>` | 样式表语法错 |
//! | 出图 | `cargo run -p zephyra-gallery --bin gallery -- --dump-cpu --route <id>` | 链路没接上（滤镜、图标、字形） |
//! | 门禁 | `cargo test -p zephyra-gallery` | 结构性回归（树、交互、断点） |
//!
//! 关键不是命令本身，而是**它们各自防的那类静默失败**。

use zephyra_core::element::Element;
use zephyra_ui::controls;

use crate::app::pages;
use crate::app::{Gallery, SUPPORTED};

/// 语言切换按钮的 key（**不能 `format!`**：`view()` 每帧重建树）。
const fn lang_key(i: usize) -> &'static str {
    match i {
        0 => "lang:en",
        1 => "lang:zh-Hans",
        _ => "lang:ja",
    }
}

/// 语言的自称（下标与 [`SUPPORTED`] 对齐，见 [`LANG_SELFS`] 的理由）。
const fn lang_self(i: usize) -> &'static str {
    match i {
        0 => "English",
        1 => "中文",
        _ => "日本語",
    }
}

pub fn page(g: &Gallery) -> Element {
    pages::page_root(
        "testing",
        "page-testing",
        vec![
            controls::text_label(
                "testing-blurb",
                "Three layers of self-verification, from fastest to slowest. Each guards a different class of silent failure.",
                "text-body",
            ),
            // 当前状态：开发者跑命令时最想知道的三个值。
            pages::section(
                "testing-current",
                "Current state",
                vec![
                    pages::demo("Route", controls::text_label(
                        "testing-route",
                        g.route,
                        "text-body",
                    )),
                    pages::demo("Language", controls::text_label(
                        "testing-lang",
                        SUPPORTED[g.lang.min(SUPPORTED.len() - 1)],
                        "text-body",
                    )),
                    pages::demo("Theme", controls::text_label(
                        "testing-theme",
                        if g.wants_dark { "Dark" } else { "Light" },
                        "text-body",
                    )),
                ],
            ),
            // ---- 第一层：样式表语法 ----
            //
            // 开发者改了 `.zstyle`，第一条红线就是 zstylec 的语法检查。
            // 它防的是**拼错属性名** —— 那种错会让整条规则静默失效，
            // 而画面上只是「这个控件看起来不对」。
            pages::section(
                "testing-zstyle",
                "1. Stylesheet syntax",
                vec![pages::demo(
                    "Check a stylesheet",
                    controls::text_label(
                        "testing-zstyle-cmd",
                        "cargo run -p zstylec -- check pages.zstyle",
                        "text-caption",
                    ),
                )],
            ),
            // ---- 第二层：CPU 出图 ----
            //
            // 不开窗口、不碰 GPU，画第一帧写 BMP。开发者没有显示器也能跑。
            // 它防的是**链路没接上**：滤镜命令数、图标名查不到、字形缺失 ——
            // 每一项都对应一段可能静默失败的代码。
            //
            // ⚠ `--route` 必须显式给：不给只出第一页，后面几页的样式错误
            // 要等窗口路才暴露。而**未知路由直接退出**，不静默回首页 ——
            // 出图工具静默给错图，比直接失败恶劣得多。
            pages::section(
                "testing-dump",
                "2. CPU raster dump",
                vec![
                    pages::demo(
                        "Dump current route",
                        controls::text_label(
                            "testing-dump-cmd",
                            "cargo run -p zephyra-gallery --bin gallery -- --dump-cpu",
                            "text-caption",
                        ),
                    ),
                    pages::demo(
                        "Dump a specific route",
                        controls::text_label(
                            "testing-dump-route",
                            "  --route reference --lang zh-Hans",
                            "text-caption",
                        ),
                    ),
                    pages::demo(
                        // 参考层的一族一屏：`--route reference` 只出第 0 族，
                        // 另外几族要靠 `--family` 才看得见（无滚动的页面里，
                        // 「装不下被裁掉」只在逐族出图时暴露）。
                        "Dump one reference family",
                        controls::text_label(
                            "testing-dump-family",
                            "  --family 3   # --list-families 列合法值",
                            "text-caption",
                        ),
                    ),
                ],
            ),
            // ---- 第三层：树级门禁 ----
            //
            // 开发者改完结构（加页面、改状态、动样式表），跑一次门禁。
            // 它们全部按**整棵树 × 全部页面 × 全部交互态**判 —— 逐函数查
            // 只能查到「我记得去查的那个」。
            pages::section(
                "testing-gates",
                "3. Tree-level gates",
                vec![
                    pages::demo(
                        "Run all gallery gates",
                        controls::text_label(
                            "testing-gates-cmd",
                            "cargo test -p zephyra-gallery",
                            "text-caption",
                        ),
                    ),
                    pages::demo(
                        "What they guard",
                        controls::text_label(
                            "testing-gates-what",
                            "Style tokens, dead rules, hit-test, breakpoints, no appearance in source",
                            "text-caption",
                        ),
                    ),
                ],
            ),
            // ---- 试一试：语言切换 ----
            //
            // `--lang` 的效果在这里**手动就能看**：点一下切语言，下一帧整棵树
            // 按新语言重建 —— 与出图路的 `--lang` 参数共享同一个状态字段。
            // 这不是装饰：门禁 `every_page_carries_its_own_controls` 要求每页
            // 至少一个真控件，而「切语言看出图对不对」恰好是这一页该有的动作。
            pages::section(
                "testing-try",
                "Try it: switch language (same state as --lang)",
                vec![pages::demo_many(
                    "Language",
                    SUPPORTED
                        .iter()
                        .enumerate()
                        .map(|(i, _)| {
                            controls::pill_button(
                                lang_key(i),
                                lang_self(i),
                                None,
                                g.lang == i,
                                g.hovers(lang_key(i)),
                                g.holds(lang_key(i)),
                            )
                        })
                        .collect(),
                )],
            ),
        ],
    )
}

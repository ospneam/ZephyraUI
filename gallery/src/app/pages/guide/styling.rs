//! 样式表：外观不进源码。
//!
//! # 这一章教什么
//!
//! 本仓的纪律：**颜色、尺寸、圆角全在 `.zstyle`，颜色走 `@令牌` →
//! `Theme::role()`**。源码只发 `key`（事件身份）与分类名（选择器）。
//! 这一章把这条纪律讲清楚，并给一个能直接照抄的形状。
//!
//! # 为什么「外观不进源码」比「少写注释」重要
//!
//! 外观写进源码不会报错、不会崩、编译照过 —— 它的代价全部在**下游**：
//! 主题换色时漏改一处、深色模式下一块看不见、设计系统与实现漂移。
//! 这些症状在「门禁全绿」的时候一样会出现。所以纪律由门禁执行，
//! 不靠自觉：`the_whole_tree_writes_no_appearance_in_source` 遍历整棵树。

use zephyra_core::element::Element;
use zephyra_ui::controls;

use crate::app::pages;
use crate::app::Gallery;

pub fn page(g: &Gallery) -> Element {
    pages::page_root(
        "styling",
        "page-styling",
        vec![
            controls::text_label(
                "styling-blurb",
                "Appearance lives in .zstyle. Source sends keys and class names only — no colors, no sizes.",
                "text-body",
            ),
            // ---- 颜色走令牌 ----
            //
            // 开发者会问「这个蓝色是哪来的」。答案是：`pages.zstyle` 里
            // `Text.styling-note { text-color: @content_secondary }` 这一行，
            // 而 `@content_secondary` 的值由当前主题给。所以换主题时颜色自己变，
            // 源码一个字都不用改 —— 而这是门禁保证的，不是约定。
            pages::section(
                "styling-tokens",
                "Colors come from theme tokens",
                vec![pages::demo(
                    "Token-colored text",
                    controls::text_label(
                        "styling-note",
                        "This color is @content_secondary",
                        "styling-note",
                    ),
                )],
            ),
            // ---- 分类名是选择器 ----
            //
            // 同一个控件发不同的分类名，样式表就能给它不同的外观 ——
            // 而源码里看不出差别。这是「选择器」这个词的字面含义。
            pages::section(
                "styling-classes",
                "Class names are selectors",
                vec![
                    pages::demo("Primary", controls::button(
                        "act:style-primary",
                        "Primary",
                        None,
                        "accent",
                        g.hovers("act:style-primary"),
                        g.holds("act:style-primary"),
                    )),
                    pages::demo("Subtle", controls::button(
                        "act:style-subtle",
                        "Subtle",
                        None,
                        "subtle",
                        g.hovers("act:style-subtle"),
                        g.holds("act:style-subtle"),
                    )),
                    pages::demo("Standard", controls::button(
                        "act:style-standard",
                        "Standard",
                        None,
                        "standard",
                        g.hovers("act:style-standard"),
                        g.holds("act:style-standard"),
                    )),
                ],
            ),
            // ---- 间距与布局 ----
            //
            // `affects_layout` 是样式表向门禁声明的一个元信息：「这个属性变了
            // 要重排」。漏声明的后果是「只改颜色也触发全树重排」—— 画面上
            // 看不出来，只在 benchmark 里现形。
            pages::section(
                "styling-layout",
                "Layout-affecting properties",
                vec![pages::demo(
                    "Spacing demo",
                    controls::text_label(
                        "styling-layout-note",
                        "Spacing is declared affects_layout so color-only changes skip relayout",
                        "text-caption",
                    ),
                )],
            ),
        ],
    )
}

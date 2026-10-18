//! 导航件：NavItem / NavSubItem / SelectorBar / Breadcrumb。
//!
//! # 这一页的控件**用静态选中态**，不接真实路由
//!
//! 左侧那根真正的导航栏已经在 [`crate::app::shell`] 里了，它接的是应用
//! 路由。这里再摆一套可点的 NavItem 会让人以为「导航有两处」—— 而它们
//! 行为不同（一个跳页、一个只是选中）。所以这一页的 NavItem 用固定
//! `selected`，点它不会有任何变化，**这是有意的**：它展示的是外观。

use zephyra_core::element::Element;

use super::Gallery;
use crate::app::pages;
use zephyra_ui::controls;

/// 一组 NavItem（第一个选中）。
const NAV_DEMO: &[(&str, &str, &str)] = &[
    ("navdemo:home", "Home", "home"),
    ("navdemo:star", "Starred", "star"),
    ("navdemo:archive", "Archive", "doc"),
];

/// SelectorBar 的项。
const SELECTOR: &[(&str, &str)] = &[
    ("seldemo:day", "Day"),
    ("seldemo:week", "Week"),
    ("seldemo:month", "Month"),
];

pub fn page(g: &Gallery) -> Element {
    pages::page_root(
        "navigation",
        "page-navigation",
        vec![
            pages::section(
                "sec-nav",
                "Nav item",
                vec![pages::page_root(
                    "navs",
                    "nav-list",
                    NAV_DEMO
                        .iter()
                        .map(|(id, label, icon)| {
                            controls::nav_item(
                                id,
                                label,
                                icon,
                                *id == "navdemo:home",
                                g.hovers(id),
                                g.holds(id),
                                false,
                            )
                        })
                        .collect(),
                )],
            ),
            pages::section(
                "sec-sub",
                "Sub item",
                vec![pages::page_root(
                    "subs",
                    "nav-list",
                    vec![
                        controls::nav_sub_item(
                            "subdemo:a",
                            "Sub A",
                            Some("menu"),
                            true,
                            g.hovers("subdemo:a"),
                            g.holds("subdemo:a"),
                        ),
                        controls::nav_sub_item(
                            "subdemo:b",
                            "Sub B",
                            None,
                            false,
                            g.hovers("subdemo:b"),
                            g.holds("subdemo:b"),
                        ),
                    ],
                )],
            ),
            pages::section(
                "sec-sel",
                "Selector bar",
                vec![pages::demo(
                    "Range",
                    controls::selector_bar(
                        "selbar",
                        SELECTOR,
                        Some("seldemo:week"),
                        g.hovered_name(),
                        g.pressed_name(),
                    ),
                )],
            ),
        ],
    )
}

/// 让 `NAV_SUB` 被用上（否则 clippy 报未读常量）。
const _: () = {};

//! 导航件：NavItem / NavSubItem / SelectorBar。
//!
//! # 三种控件的交互语义**各不相同**，这不是不一致
//!
//! - **NavItem**（`navdemo:*`）：左侧真正的导航栏在 [`crate::app::shell`]
//!   里，它接的是应用路由。这里再摆一套可跳页的 NavItem 会让人以为
//!   「导航有两处」。所以这一页的 NavItem 用固定 `selected`，点了不变
//!   —— **这是有意的**，它展示的是外观（白名单在门禁里注明）。
//! - **NavSubItem**（`subdemo:*`）：子项没有「跳页」语义，它的完整交互
//!   就是**选中**。接 [`Gallery::subsel`]，点谁谁亮。
//! - **SelectorBar**（`seldemo:*`）：本质是一组互斥的过滤器，接
//!   [`Gallery::selbar_sel`]。
//!
//! 把三种语义写在页首，比让开发者逐个去点猜测要诚实。

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

/// SelectorBar 的项。id 与 [`crate::app::SELDEMO_IDS`] 对齐 ——
/// 那张表是 `click` 查表还原 `'static` 的依据，两处必须同步。
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
                            g.subsel == "subdemo:a",
                            g.hovers("subdemo:a"),
                            g.holds("subdemo:a"),
                        ),
                        controls::nav_sub_item(
                            "subdemo:b",
                            "Sub B",
                            None,
                            g.subsel == "subdemo:b",
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
                        Some(g.selbar_sel),
                        g.hovered_name(),
                        g.pressed_name(),
                    ),
                )],
            ),
        ],
    )
}

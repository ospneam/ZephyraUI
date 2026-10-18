//! 集合：ListView / TableView / SwipeView / Avatar。
//!
//! # 三个控件的差别是**取数据的形状**
//!
//! - `ListView` 吃 `&[(&str, &str)]` —— 一维，label + key；
//! - `TableView` 吃 `&[&[&str]]` —— 二维，表头与行；
//! - `SwipeView` 吃 `&[(&str, &str)]` 但只显示**当前**那一页。
//!
//! 把它们放一页是因为"装一串同类东西"是同一个需求，差别只在数据形状 —
//! 而数据形状正是控件函数签名里能直接看出来的。

use zephyra_core::element::Element;

use super::Gallery;
use crate::app::pages;
use zephyra_ui::controls;

/// ListView 的数据（`&'static`：控件签名收静态切片）。
const LIST_ITEMS: &[(&str, &str)] = &[
    ("list:a", "Acrylic"),
    ("list:b", "Mica"),
    ("list:c", "Solid"),
];

/// TableView 的表头与行。
const TABLE_HEAD: &[&str] = &["Backdrop", "Cost", "Available"];
const TABLE_ROWS: &[&[&str]] = &[
    &["Mica", "Low", "Win11"],
    &["Acrylic", "High", "Win10+"],
    &["Solid", "None", "All"],
];

/// SwipeView 的页面（只显示当前那一张）。
const SWIPE: &[(&str, &str)] = &[
    ("swipe:0", "First card"),
    ("swipe:1", "Second card"),
    ("swipe:2", "Third card"),
];

pub fn page(g: &Gallery) -> Element {
    pages::page_root(
        "collections",
        "page-collections",
        vec![
            pages::section(
                "sec-list",
                "List",
                vec![controls::list_view(
                    "list",
                    LIST_ITEMS,
                    Some("list:b"),
                    g.hovered_name(),
                    g.pressed_name(),
                )],
            ),
            pages::section(
                "sec-table",
                "Table",
                vec![controls::table_view("table", TABLE_HEAD, TABLE_ROWS)],
            ),
            pages::section(
                "sec-swipe",
                "Swipe",
                vec![
                    controls::swipe_view("swipe", SWIPE, 1),
                    controls::page_indicator("pips", SWIPE.len(), 1),
                ],
            ),
            pages::section(
                "sec-avatar",
                "Avatar",
                vec![pages::demo_many(
                    "Sizes",
                    vec![
                        controls::avatar("avatar:a", "ZY", false),
                        controls::avatar("avatar:b", "AB", true),
                    ],
                )],
            ),
        ],
    )
}

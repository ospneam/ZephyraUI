//! 集合：ListView / TableView / Avatar。
//!
//! # 这一族的选型判据是**取数据的形状**
//!
//! - `ListView` 吃 `&[(&str, &str)]` —— 一维，label + key；
//! - `TableView` 吃 `&[&[&str]]` —— 二维，表头与行；
//! - `Avatar` 只吃一个缩写串 —— 身份，不是列表。
//!
//! 放一页是因为「装一串同类东西」是同一个需求，差别只在数据形状 ——
//! 而数据形状正是控件函数签名里能直接看出来的。
//!
//! ⚠ `SwipeView` 从这一族搬去了 [`crate::app::pages::paging`]：参考页
//! 无滚动，四节内容在真实字体下会把 Avatar 整节顶出客户区。分页控件的
//! 语义（翻到第几页）也更贴近导航。

use zephyra_core::element::Element;

use super::Gallery;
use crate::app::pages;
use zephyra_ui::controls;

/// ListView 的数据（`&'static`：控件签名收静态切片）。
///
/// # 为什么是 `pub`
///
/// `Gallery::click` 的 `list:` 分支要按 id 查回下标（`list_sel` 是 usize），
/// 直接引用这张表 —— 页面与分派共享一份 id 清单，改 id 时编译器会同时
/// 提醒两处。
pub const LIST_ITEMS: &[(&str, &str)] = &[
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
                    // 选中行走 `Gallery::list_sel` —— 行的 key 是 items 的 id，
                    // 点击经 `click` 的 `list:` 分支回写下标。循环一次列表的
                    // 下标在两次渲染之间是稳定的（LIST_ITEMS 是常量）。
                    LIST_ITEMS
                        .get(g.list_sel)
                        .map(|(id, _)| *id)
                        .or(Some("list:b")),
                    g.hovered_name(),
                    g.pressed_name(),
                )],
            ),
            pages::section(
                "sec-table",
                "Table",
                vec![
                    // ⚠ 数据格（`td:RC`）是可点的：点击经 `click` 的 `td:`
                    // 分支写 `table_cell`，下面的 caption 显示选中格的内容。
                    controls::table_view("table", TABLE_HEAD, TABLE_ROWS),
                    pages::demo(
                        "Selected cell",
                        controls::text_label(
                            "table-cell",
                            table_cell_text(g.table_cell),
                            "text-body-strong",
                        ),
                    ),
                ],
            ),
            pages::section(
                "sec-avatar",
                "Avatar",
                // 头像是**展示**件：它的语义是「这个用户长这样」，没有点击
                // 动作可言。白名单在门禁里注明。
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

/// 选中数据格的内容（查表，理由同 [`crate::app::pages::basics::spin_text`]）。
///
/// 下标来自 `click` 的 `td:` 分支（行 ×4 + 列），表宽 4 与 TABLE_ROWS 的
/// 行宽一致——改 TABLE_ROWS 的列数时这里同步。
fn table_cell_text(i: usize) -> &'static str {
    const FLAT: [&str; 9] = [
        "Mica / Low",
        "Low",
        "Win11",
        "Acrylic / High",
        "High",
        "Win10+",
        "Solid / None",
        "None",
        "All",
    ];
    FLAT.get(i).copied().unwrap_or("Mica / Low")
}

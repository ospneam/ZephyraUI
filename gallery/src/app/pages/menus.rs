//! 菜单与工具栏：MenuBar / Menu / ToolButton / DropDownButton。
//!
//! # 交互说明
//!
//! 这一页的展开态由 [`Gallery::menu_open`] 与 [`Gallery::dd_open`] 驱动，
//! 点击在 `Gallery::click` 里处理。再点一次当前项 = 收起（菜单栏的常规行为）。
//!
//! ⚠ core 的菜单是**内联卡**而不是浮层 —— 没有弹窗层，于是菜单展开时
//! 它会把下面的内容顶开。这是已知取舍（`Command::PushClip` / `Layer::Popup`
//! 都还没有生产调用方），页面里留出空间免得展开时看起来像"内容炸开了"。

use zephyra_core::element::Element;

use super::Gallery;
use crate::app::pages;
use zephyra_ui::controls;

/// 菜单栏的三项（点第几项 = 打开第几个菜单）。
const BAR_ITEMS: &[&str] = &["File", "Edit", "View"];

/// 每个菜单里的条目（`label` 为空 = 一条分隔线）。
const MENU_ENTRIES: &[&[(&str, &str)]] = &[
    &[
        ("New", "m:new"),
        ("", ""),
        ("Open…", "m:open"),
        ("Save", "m:save"),
    ],
    &[
        ("Undo", "m:undo"),
        ("Redo", "m:redo"),
        ("", ""),
        ("Cut", "m:cut"),
    ],
    &[
        ("Zoom In", "m:zoom-in"),
        ("Zoom Out", "m:zoom-out"),
        ("", ""),
        ("Reset", "m:reset"),
    ],
];

/// 工具栏图标按钮（图标名取自 core 内置图标集）。
const TOOLS: &[(&str, &str)] = &[
    ("tool:save", "doc"),
    ("tool:search", "search"),
    ("tool:gear", "settings"),
];

pub fn page(g: &Gallery) -> Element {
    let open = g.menu_open;
    pages::page_root(
        "menus",
        "page-menus",
        vec![
            pages::section(
                "sec-bar",
                "Menu bar",
                vec![controls::menu_bar("menubar", BAR_ITEMS, open)],
            ),
            // 只有展开了才画菜单 —— 否则内联卡会把整页顶出一个空位。
            match open {
                // ⚠ `unwrap_or` 传的是 `MENU_ENTRIES[0]` 而不是 `&MENU_ENTRIES[0]`：
                // `.copied()` 已经把 `Option<&&[..]>` 收成 `Option<&[..]>`，
                // 再加一层 `&` 会被立刻解引用（clippy `needless_borrow` 会红）。
                Some(i) => controls::menu(
                    "menu",
                    MENU_ENTRIES.get(i).copied().unwrap_or(MENU_ENTRIES[0]),
                ),
                None => pages::stack(
                    "menu-hint",
                    vec![zephyra_ui::controls::caption(
                        "menu-hint",
                        "Click a menu bar item to open it.",
                    )],
                ),
            },
            pages::section(
                "sec-tools",
                "Toolbar",
                vec![pages::demo_many(
                    "Tools",
                    TOOLS
                        .iter()
                        .map(|(id, icon)| {
                            controls::tool_button(id, icon, g.hovers(id), g.holds(id))
                        })
                        .collect(),
                )],
            ),
            pages::section(
                "sec-dd",
                "Drop-down",
                vec![pages::demo(
                    "Button",
                    controls::drop_down_button(
                        "dd:demo",
                        "Options",
                        Some("menu"),
                        g.dd_open,
                        g.hovers("dd:demo"),
                        g.holds("dd:demo"),
                    ),
                )],
            ),
        ],
    )
}

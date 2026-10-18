//! 控件参考：一页内分九族，segmented 切换。
//!
//! # 为什么是一页而不是九条路由
//!
//! 九条路由平铺进导航栏装不下 —— 导航栏高 756，项高 40 + 间距 6 = 46，
//! 九项 = 414，加上指南六章 + Settings 就爆了。而 core 的二级导航
//! `nav_sub_item` **不折叠**，只是缩进显示，九个子项常驻一样撑爆。
//!
//! 所以参考层收进**一页**，页内用 segmented 切族。segmented 是 core 真的
//! 接了状态的控件（`Gallery::click` 里已有 `seg:*` 分支），页内切换是
//! 现成模式。每族内容复用现有页面模块的 `page()`，各自独立一屏，装得下。
//!
//! # 族顺序即 segmented 段顺序
//!
//! [`crate::app::REF_FAMILIES`] 是唯一真相：segmented 的段、`click()` 的
//! 查表、门禁的遍历都从它来。加一族只改那张表。

use zephyra_core::element::Element;
use zephyra_ui::controls;

use crate::app::pages;
use crate::app::{Gallery, REF_FAMILIES};

pub fn page(g: &Gallery) -> Element {
    // segmented 段：九族。段 id 带 `ref:` 前缀 —— `Gallery::click` 的查表按
    // `strip_prefix("ref:")` 解析，所以发裸名 `basics` 会**静默**分派不出去
    // （画面上段画出来了、点了没反应）。显示名走 [`family_label`] 查表。
    let items: Vec<(&str, &str)> = REF_FAMILIES
        .iter()
        .map(|f| (seg_key(f), family_label(f)))
        .collect();
    let selected = REF_FAMILIES.get(g.ref_family).copied().map(seg_key);
    let segmented = controls::segmented(
        "ref-family",
        &items,
        selected,
        g.hovered_name(),
        g.pressed_name(),
    );

    // 选中族的内容。每族的 `page()` 返回一个 `page_root(...)` 包过的容器 ——
    // 嵌进来只是多一层竖排容器，不影响布局语义（竖排 + 间距）。
    //
    // ⚠ 这里**不**调 `pages::page_root` 再包一层：那会给内容区加一个额外的
    // 外层间距，而内容区高度是固定的 756，多一层就多一份被吃掉的风险。
    let content = family_content(g);

    pages::page_root(
        "reference",
        "page-reference",
        vec![
            controls::text_label(
                "reference-blurb",
                "Control reference — pick a family to see its controls.",
                "text-body",
            ),
            segmented,
            content,
        ],
    )
}

/// 选中族的内容。查表而不是 match —— 加一族只改 [`REF_FAMILIES`] 与
/// 这个函数的分支（编译器会因函数未使用而提醒漏了一处）。
fn family_content(g: &Gallery) -> Element {
    match REF_FAMILIES.get(g.ref_family).copied() {
        Some("basics") => crate::app::pages::basics::page(g),
        Some("toggles") => crate::app::pages::toggles::page(g),
        Some("layout") => crate::app::pages::layout::page(g),
        Some("collections") => crate::app::pages::collections::page(g),
        Some("menus") => crate::app::pages::menus::page(g),
        Some("navigation") => crate::app::pages::navigation::page(g),
        Some("status") => crate::app::pages::status::page(g),
        Some("dialogs") => crate::app::pages::dialogs::page(g),
        Some("effects") => crate::app::pages::effects::page(g),
        _ => crate::app::pages::basics::page(g),
    }
}

/// 族的显示名。查表而不是 match —— 与 [`REF_FAMILIES`] 一一对应。
fn family_label(id: &str) -> &'static str {
    match id {
        "basics" => "Basic Input",
        "toggles" => "Selection",
        "layout" => "Layout",
        "collections" => "Collections",
        "menus" => "Menus",
        "navigation" => "Navigation",
        "status" => "Status",
        "dialogs" => "Dialogs",
        "effects" => "Liquid Glass",
        // ⚠ 不能写 `_ => id`：`id: &str` 的生命周期不是 `'static`，而函数
        // 返回 `&'static str`。未知 id 显示占位符 —— 与 [`family_content`]
        // 的兜底行为一致（回落到 basics），画面上看得见。
        _ => "Unknown",
    }
}

/// 族的 segmented key（`ref:<族名>`）。
///
/// # 为什么不是 `const fn`
///
/// `&str` 上的 `match` 在常量上下文里还不允许（E0658，与
/// `effects.rs::pane_title_key` 同一限制）。它每帧只调九次，运行期比较的
/// 开销可忽略。
fn seg_key(fam: &str) -> &'static str {
    match fam {
        "basics" => "ref:basics",
        "toggles" => "ref:toggles",
        "layout" => "ref:layout",
        "collections" => "ref:collections",
        "menus" => "ref:menus",
        "navigation" => "ref:navigation",
        "status" => "ref:status",
        "dialogs" => "ref:dialogs",
        "effects" => "ref:effects",
        _ => "ref:unknown",
    }
}

//! 页面分发 —— 一页一个模块，这里只做**查表**。
//!
//! # 为什么不是 `match`
//!
//! 旧版是一个 `match g.route { ... }` 同时承担「分发」与「组页」，于是
//! 1254 行里混着 10 个互不相干的页面。加上分组索引与控件页之后，那个 `match`
//! 要在前面插两次前置判断（分组路由、控件路由都不是字面量），漏一条就静默
//! 落到 `_ => home` —— 而症状是「点了没反应，界面完全正常」。
//!
//! 现在每个页面是 [`Page`] 的一项，[`page`] 只查表。新增页面 = 加一个模块 +
//! 在 [`PAGES`] 加一行，漏不了的：编译器会因 `mod` 未被使用而提醒。
//!
//! # `fn` 还是 `&Gallery`
//!
//! 全部 `&Gallery`（不 `&mut`）：页面**只读**应用状态，交互由 `Gallery::click`
//! 统一处理。页面要改状态就发一个带 key 的 `Control`，不直接改 —— 这样
//! 「哪些 key 会改状态」始终只有 `click()` 一处真相。

use zephyra_core::element::Element;
use zephyra_ui::controls;

use super::Gallery;

pub mod basics;
pub mod collections;
pub mod dialogs;
pub mod effects;
pub mod home;
pub mod layout;
pub mod menus;
pub mod navigation;
pub mod settings;
pub mod status;
pub mod toggles;

/// 一个页面：标题由 [`Gallery`] 统一取，这里只管产出内容。
///
/// # 为什么不返回 `Element` 而是这种 trait object
///
/// 早期版本是 `fn(&Gallery) -> Element` 的具名函数。10 个函数写在同一个文件里，
/// 想让「页面」成为可枚举的东西（导航栏、测试遍历、`--list-routes`）就得再写
/// 一份列表 —— 于是有了「路由表」与「分发」两份真相。
///
/// 现在 [`PAGES`] 就是**唯一**那份：导航栏遍历它、测试遍历它、`page()` 遍历它。
pub struct Page {
    /// 路由 id（与 [`ROUTES`](super::ROUTES) 的第一项一致）。
    pub id: &'static str,
    /// 导航栏与首页入口显示的标题。
    ///
    /// # 为什么是 `&'static str` 而不在这里做 i18n
    ///
    /// 这里是**英文**原名，供门禁与调试用。画面上的文字走
    /// [`Gallery::tr`](super::Gallery::tr) —— 那条路查 `CATALOG` 文案表。
    /// 两者分开是为了让「路由元数据」不依赖「用户的语言」：门禁断言列出的
    /// 是稳定的 id 与英文名，而用户看到的是本地化后的标题。
    pub title: &'static str,
    /// 图标名（core 内置图标集）。
    pub icon: &'static str,
    /// 构造函数。
    pub build: fn(&Gallery) -> Element,
}

/// 全部页面，按导航栏顺序。
///
/// ⚠ 顺序即导航栏从上到下的顺序；**Settings 在最后**（它在导航栏里被置底，
/// 但它仍然是一个页面，所以仍然在这里 —— 见下面的「为什么 Settings 在里面」）。
///
/// # 为什么 Settings 在 `PAGES` 里，尽管它不在 `ROUTES` 里
///
/// `ROUTES` 是**导航栏的项**（Settings 由 `shell::nav_pane` 单独 push 到末尾，
/// 所以不在表里）；`PAGES` 是**能构造出来的页面**。两者不是一回事。
///
/// 第一版把 Settings 排除在 `PAGES` 外，于是 `pages::page()` 对 `settings`
/// 路由查不到构造器、静默回落到首页 —— 画面上标题写着「Settings」、侧栏高亮
/// Settings、**正文却是首页**，而且没有任何报错（门禁全绿：首页的树足够大、
/// 每个节点都在客户区内）。所以那条「查不到就回首页」的兜底是有代价的，
/// 现在由门禁 `every_route_renders_its_own_page` 直接钉住。
///
/// # 为什么 `PAGES` 与 [`ROUTES`](super::ROUTES) 并存
///
/// 两份表看着重复，其实分工不同：`ROUTES` 是**导航栏**用的（只需要 id +
/// 标题 + 图标），`PAGES` 还多一个构造函数 —— 首页要遍历它拿图标与标题。
///
/// 真要合并成一份，就把 `ROUTES` 删掉、导航栏改遍历 `PAGES`。没这么做是因为
/// 导航栏那份需要的字段更少，而让两条路径各自只依赖自己要的东西更不容易漂移。
/// 门禁 `route_table_and_page_table_agree` 钉住两份表的 id 集合相同。
pub const PAGES: &[Page] = &[
    Page {
        id: "home",
        title: "Overview",
        icon: "home",
        build: home::page,
    },
    Page {
        id: "basics",
        title: "Basic Input",
        icon: "pen",
        build: basics::page,
    },
    Page {
        id: "toggles",
        title: "Selection",
        icon: "check",
        build: toggles::page,
    },
    Page {
        id: "layout",
        title: "Layout",
        icon: "grid",
        build: layout::page,
    },
    Page {
        id: "collections",
        title: "Collections",
        icon: "doc",
        build: collections::page,
    },
    Page {
        id: "menus",
        title: "Menus",
        icon: "settings",
        build: menus::page,
    },
    Page {
        id: "navigation",
        title: "Navigation",
        icon: "menu",
        build: navigation::page,
    },
    Page {
        id: "status",
        title: "Status",
        icon: "bell",
        build: status::page,
    },
    Page {
        id: "dialogs",
        title: "Dialogs",
        icon: "calendar",
        build: dialogs::page,
    },
    Page {
        id: "effects",
        title: "Liquid Glass",
        icon: "star",
        build: effects::page,
    },
    // Settings 排在最后，但**不是** `ROUTES` 的一员（它由 shell 置底）。
    Page {
        id: "settings",
        title: "Settings",
        icon: "settings",
        build: settings::page,
    },
];

/// 按路由 id 产出页面。未知 id 回首页 —— 兜底而不是 `panic!`：
/// 一个 typo 不该让整个应用崩掉，而「回到首页」在画面上立刻看得出是路由错了。
pub fn page(g: &Gallery) -> Element {
    match PAGES.iter().find(|p| p.id == g.route) {
        Some(p) => (p.build)(g),
        None => home::page(g),
    }
}

/// 一页的根容器。**每个页面都必须用它**，否则 `page-*` 的间距分类名不生效。
///
/// # 为什么不给默认样式
///
/// `page()` 直接发一个 `Element`，分类名由调用方给 —— 于是"这一页用哪个
/// 间距"在源码里可见。共用一个 `fn page_of(id)` 造根容器会让间距藏在
/// 一张表里，而那张表又得跟着样式表一起改。
pub fn page_root(key: &'static str, class: &'static str, children: Vec<Element>) -> Element {
    Element::new(zephyra_core::id::NodeTag::Container)
        .with_key(key)
        .style(class)
        .with_children(children)
}

/// 一个分区：标题 + 内容。
pub fn section(key: &'static str, title: &'static str, children: Vec<Element>) -> Element {
    page_root(
        key,
        "section",
        vec![
            controls::text_label("section-title", title, "text-body-strong"),
            page_root("section-body", "section-body", children),
        ],
    )
}

/// 横排行（`id` 进 key：同一父下的多个行要靠它区分）。
pub fn row(id: &'static str, children: Vec<Element>) -> Element {
    page_root(id, "row", children)
}

/// 竖排栈（`id` 进 key，同 [`row`]）。
pub fn stack(id: &'static str, children: Vec<Element>) -> Element {
    page_root(id, "stack", children)
}

/// 控件的一行样例：左标签、右控件。**每页的基础编排单元**。
///
/// # 为什么标签与控件**不等宽**
///
/// 这行是给人"扫一眼这是哪个控件"的，所以标签固定一小截、控件跟在后面 ——
/// 而控件之间的横向位置因此**一致**，扫第二列时不会因为第一个控件特别宽而
/// 全部错开。
pub fn demo(label: &'static str, control: Element) -> Element {
    page_root(
        label,
        "demo-row",
        vec![
            controls::text_label(label, label, "demo-label"),
            page_root(label, "demo-slot", vec![control]),
        ],
    )
}

/// 同 [`demo`]，但一行摆多个控件（它们之间的间距由样式表给）。
pub fn demo_many(label: &'static str, controls_: Vec<Element>) -> Element {
    page_root(
        label,
        "demo-row",
        vec![
            controls::text_label(label, label, "demo-label"),
            page_root(label, "demo-slot", controls_),
        ],
    )
}

//! 页面：RinUI gallery 的分页重写。
//!
//! 每页一个函数，只产 `Element`。所有可交互件的 key 都是**字面量**
//! （点击路由靠它认），列表类内容用 `Id(n)` 当 key（顺序即身份，与
//! `examples/demo.rs` 的 `SRC_CROP_KEYS` 同一手法）。
//!
//! # 网格是手排的
//!
//! core 没有 Grid：`ROWS` 常量为每个手排行定卡片数，行容器就是一排
//! 横排容器。RinUI 那边按窗口宽算列数，我们按固定画布定 —— 见 README
//! 的范围节。

use zephyra_core::element::Element;
use zephyra_core::id::NodeTag;

use super::{catalog, Gallery, COMBO_OPTIONS, LANG_SELFS, SUPPORTED};
use crate::controls::{
    acrylic_card, auto_suggest_box, avatar, badge, busy_indicator, button, caption, card,
    check_box, color_picker, combo_box, date_picker, dialog, drop_down_button, error_page,
    expander, hyperlink, info_badge, info_bar, list_view, menu, menu_bar, nav_item, nav_sub_item,
    page_indicator, pill_button, progress_bar, progress_ring, radio_button, round_button,
    rounded_corner_overlay, scrollable_text_area, segmented, selector_bar, setting_card,
    setting_expander, setting_item, slider, spin_box, swipe_view, table_view, text_area,
    text_field, text_input_menu, text_label, time_picker, toast, toggle_switch, tool_button,
    tool_tip,
};

/// **基准画布**（[`crate::app::BASE_VIEWPORT`] = 1400×800）下每个手排行的卡片数。
///
/// 它**不是**排布用的常量 —— 排布列数由 [`Gallery::card_columns`] 按可用宽度现算
/// （窄窗口降列、宽窗口加列，见官方的「重新排列」）。这里的用途是「基准外观不能变」
/// 的锚点：`tests/gates::基准画布下列数与锚点一致` 断言默认 Gallery 算出的列数仍等于它。
///
/// 历史上它直接驱动 `clip_grid_c`，于是窗口变化时内容仍按这个数排 ——
/// 宽窗口右侧留白、窄窗口溢出被裁，正是那两条症状的来源。
pub const BASE_COLUMNS: usize = 5;

/// 按路由分发。
pub fn page(g: &Gallery) -> Element {
    // 分组索引页（`group:<id>`）：14 个路由，不是字面量，所以要在 match 之前拦。
    // 必须拦在前面 —— `match` 的 `_` 分支会把它们当 home 静默吞掉。
    if catalog::group_of_route(g.route).is_some() {
        return group_page(g, g.route);
    }
    // 控件卡直接落到**它所属分组**的索引页（并把该控件标成当前项）。
    //
    // ⚠️ 这里**不**静默回 Home —— 那会让"点 44 张卡"里绝大多数毫无反应，
    // 而界面看起来完全正常。落到所属分组页至少是有内容、且用户能看懂的结果。
    // 逐控件独立页面（42 个）是批 8 的工作。
    if catalog::entry(g.route).is_some() {
        return super::showcase::showcase(g, g.route);
    }
    match g.route {
        "all" => all_samples(g),
        "basic-input" => basic_input(g),
        "collections" => collections(g),
        "menus" => menus_page(g),
        "status" => status_info(g),
        "text" => text_page(g),
        "navigation" => navigation_page(g),
        "dialogs" => dialogs_page(g),
        "testbed" => super::testbed::testbed(g),
        "error" => error_page_route(g),
        "settings" => settings_page(g),
        _ => home(g),
    }
}

/// 一个分区：标题 + 内容。
pub fn section(key: &'static str, title: &'static str, children: Vec<Element>) -> Element {
    Element::new(NodeTag::Container)
        .with_key(key)
        .style("section")
        .with_children([
            text_label("section-title", title, "text-body-strong"),
            Element::new(NodeTag::Container)
                .with_key("section-body")
                .style("section-body")
                .with_children(children),
        ])
}

/// 横排行（间距 12）。`id` 进 key：同一父下的多个行要靠它区分。
pub fn row(id: u64, children: Vec<Element>) -> Element {
    Element::new(NodeTag::Container)
        .with_key(id)
        .style("row")
        .with_children(children)
}

/// 竖排栈（间距 8）。同上，`id` 进 key。
pub fn stack(id: u64, children: Vec<Element>) -> Element {
    Element::new(NodeTag::Container)
        .with_key(id)
        .style("stack")
        .with_children(children)
}

/// 首页三张链接卡的 key。
const LINK_KEYS: [&str; 3] = ["linkcard:0", "linkcard:1", "linkcard:2"];

/// ComboBox 当前值的文字（下标 → 文案由**宿主**映射，控件不持有选项表）。
fn combo_label(index: usize) -> &'static str {
    COMBO_OPTIONS.get(index).copied().unwrap_or("—")
}

/// TimePicker 的可见窗口（`(key, 文字)` 静态表 —— 不 `format!`，见 clip_grid 注释）。
const HOUR_WINDOW: [(&str, &str); 6] = [
    ("tph:13", "13"),
    ("tph:14", "14"),
    ("tph:15", "15"),
    ("tph:16", "16"),
    ("tph:17", "17"),
    ("tph:18", "18"),
];
const MINUTE_WINDOW: [(&str, &str); 6] = [
    ("tpm:00", "00"),
    ("tpm:05", "05"),
    ("tpm:10", "10"),
    ("tpm:15", "15"),
    ("tpm:20", "20"),
    ("tpm:30", "30"),
];

// ---------------------------------------------------------------- Home

/// 首页：横幅 + 链接卡 + 分段 + 最近添加。
fn home(g: &Gallery) -> Element {
    Element::new(NodeTag::Container)
        .with_key("home")
        .style("page-home")
        .with_children([
            banner(),
            link_row(),
            pill_row(g),
            // "Recently added" 的数据来自 **catalog 的 `added: true`**（12 条），
            // 不是手抄的一批控件名 —— 与 Rin-UI `getRecentlyAddedItems()` 同源。
            // `seg == "favorite"` 时换成收藏视图（本仓收藏集为空，故显示空态提示）。
            if g.seg == "recent" {
                section("sec-recent", "Recently added", recent_grid(g))
            } else {
                section("sec-fav", "Favorite", favorite_grid(g))
            },
        ])
}

/// Recently added 网格：catalog 里 `added: true` 的条目。
///
/// # 为什么只取前 [`HOME_RECENT_LIMIT`] 条
///
/// catalog 有 12 条 `added`，5 列排 3 行；Home 页从 banner（350）往下到视口底
/// 只有约 420px 放得下**两行**半。第 3 行会掉到视口外——而溢出的部分
/// 不会报错，只是"看不见"，除非专门去量（`hit_test_agrees_with_the_painted_rects`
/// 就是因为元素掉出视口才红）。
///
/// Rin-UI 那边是**滚动页**（`ScrollView`），core 无滚动（见 MIGRATION 能力边界），
/// 所以只能"少放几条"这个诚实的降级，而不是假装能滚。
const HOME_RECENT_LIMIT: usize = 10;

fn recent_grid(g: &Gallery) -> Vec<Element> {
    let cards: Vec<Element> = catalog::ENTRIES
        .iter()
        .filter(|e| e.added)
        .take(HOME_RECENT_LIMIT)
        .map(|e| catalog::clip_card(e.page, e.title, e.desc, catalog::icon_for(e), g))
        .collect();
    clip_grid_c(cards, g.card_columns())
}

/// 收藏视图。
///
/// ⚠️ Rin-UI 的收藏存**后端**（`Backend.getFavorites()`，跨会话持久化）；
/// 本仓**没有**后端，也不该为 demo 引入持久化。降级为**明确的空态提示**
/// 而不是"看起来像坏了的空网格" —— 后者会被当成渲染缺陷。
fn favorite_grid(_g: &Gallery) -> Vec<Element> {
    vec![text_label(
        "fav-empty",
        "收藏需要后端持久化，本 Demo 未接入（原版走 Backend.getFavorites）",
        "text-body",
    )]
}

/// 横幅：RinUI 首页的 banner 图在这里是**一块深蓝纯色卡**（没有位图资源，
/// 且渐变不在 core 能力内 —— 见 README 范围节）。两行白字标题。
fn banner() -> Element {
    Element::new(NodeTag::Container)
        .with_key("banner")
        .style("banner")
        .with_children([
            text_label(
                "banner-kicker",
                "A Fluent Design-like UI library, in Rust",
                "text-subtitle",
            ),
            text_label("banner-title", "ZephyraUI Gallery", "banner-title"),
            text_label(
                "banner-sub",
                "Widgets on zephyra-core: class names in, stylesheet out",
                "banner-sub",
            ),
        ])
}

/// 链接卡一排三张（RinUI Home 的 `LinkClip`）。
///
/// 标题与说明照抄 `Home.qml` 里的 Repeater model；说明文字换成中文因为
/// 原版靠 `qsTr` 做本地化，而本仓没有翻译表 —— **保留英文原文**是"照抄"
/// 的下限，改写则是另一次设计决定。
fn link_row() -> Element {
    let links = [
        (
            "gear",
            "Getting Started",
            "Get started with ZephyraUI and explore detailed documentation.",
        ),
        (
            "doc",
            "Documentation",
            "Explore the comprehensive documentation for ZephyraUI components.",
        ),
        (
            "star",
            "RinUI on GitHub",
            "Explore the RinUI source code and repository.",
        ),
    ];
    let cards: Vec<Element> = links
        .iter()
        .enumerate()
        .map(|(i, (icon, title, desc))| {
            card(
                LINK_KEYS[i.min(LINK_KEYS.len() - 1)],
                "link-card",
                vec![
                    Element::new(NodeTag::Container)
                        .with_key("icon")
                        .with(
                            &zephyra_core::props::ICON_NAME,
                            zephyra_core::property::PropertyValue::static_str(icon),
                        )
                        .style("link-icon"),
                    text_label("link-title", title, "text-body-strong"),
                    // 不用 `caption()`：它没有 `text-max-width`，而 `Text` 按**整行
                    // 文本宽度**参与布局（既不换行也不裁剪），于是 60 字符的说明
                    // 会横向溢出到相邻卡片上。折行宽度是排版几何，归样式表。
                    text_label("link-desc", desc, "link-desc"),
                ],
            )
        })
        .collect();
    row(0, cards)
}

/// Recent / Favorite 两个药丸按钮（RinUI Home 的 PillButton 组）。
fn pill_row(g: &Gallery) -> Element {
    row(
        1,
        vec![
            pill_button(
                "seg:recent",
                "Recent",
                Some("home"),
                g.seg == "recent",
                g.hovers("seg:recent"),
                g.holds("seg:recent"),
            ),
            pill_button(
                "seg:favorite",
                "Favorite",
                Some("star"),
                g.seg == "favorite",
                g.hovers("seg:favorite"),
                g.holds("seg:favorite"),
            ),
        ],
    )
}

// ---------------------------------------------------------- All samples

/// All samples：把 catalog 的 **44 条**按原版顺序铺成卡片网格。
///
/// # 为什么不用 `clip_grid` 那套手排常量
///
/// 上一版这里用的是 `ALL_CLIPS`（手抄的一批控件名）。那是**假数据** ——
/// 和原版的 44 条没有对应关系，"照抄 demo"从这一页开始就不成立了。
/// 现在直接消费 [`catalog`]：条目数、分组、描述全部来自那份脚本提取的表，
/// 而表本身有门禁钉住（`catalog_matches_the_rinui_original`）。
fn all_samples(g: &Gallery) -> Element {
    // 卡片 key 用 `page`（原版 .qml 文件名）—— 天然唯一，且点卡片能跳到该控件页。
    let cards: Vec<Element> = catalog::hits(catalog::search(""))
        .map(|e| catalog::clip_card(e.page, e.title, e.desc, catalog::icon_for(e), g))
        .collect();
    Element::new(NodeTag::Container)
        .with_key("all")
        .style("page-all")
        .with_children([
            // 14 个分组 chip 条：点进分组索引页。这是 Rin-UI 分类导航的等价物 ——
            // 放在 All 页里而不是侧栏，因为 core 无滚动 / 无折叠，侧栏放不下 14 项。
            section("sec-groups", "Categories", vec![group_chip_row(g)]),
            section(
                "sec-all",
                "All controls",
                clip_grid_c(cards, g.card_columns()),
            ),
        ])
}

/// 14 个分组 chip（两行横排）。
fn group_chip_row(g: &Gallery) -> Element {
    let chips: Vec<Element> = catalog::GROUPS
        .iter()
        .filter_map(|grp| {
            let route = catalog::group_route(grp.id)?;
            Some(pill_button(
                route,
                grp.title,
                Some(grp.icon),
                g.is_route(route),
                g.hovers(route),
                g.holds(route),
            ))
        })
        .collect();
    // 分两行：14 项一行放不下（每 chip ~120px，内容区约 1111 → 一行 9 个）。
    // core 没有 flex-wrap，所以**手排**两行。
    //
    // ⚠️ 不能用 `chunks()`：core 的 `Element` 不实现 `Clone`，克隆即深拷贝。
    // 用 `split_off` 逐行取走，每张 chip 只被移动一次。
    let per_row = 7;
    let mut chips = chips;
    let mut rows: Vec<Element> = Vec::new();
    while !chips.is_empty() {
        let take = chips.len().min(per_row);
        let rest = chips.split_off(take);
        let chunk = std::mem::replace(&mut chips, rest);
        rows.push(
            Element::new(NodeTag::Container)
                .with_key(700 + rows.len() as u64)
                .style("row")
                .with_children(chunk),
        );
    }
    Element::new(NodeTag::Container)
        .with_key("group-chips")
        .style("group-chips")
        .with_children(rows)
}

/// 分组索引页：`group:<id>` —— 列出该组全部控件（卡片形式）。
fn group_page(g: &Gallery, route: &'static str) -> Element {
    let Some(group_id) = catalog::group_of_route(route) else {
        return error_page_route(g); // 非法路由 → 回退到 Error 页（诚实，不假装正常）
    };
    let Some(grp) = catalog::group(group_id) else {
        return error_page_route(g);
    };
    let cards: Vec<Element> = catalog::entries_in(group_id)
        .map(|e| catalog::clip_card(e.page, e.title, e.desc, catalog::icon_for(e), g))
        .collect();
    Element::new(NodeTag::Container)
        .with_key(route)
        .style("page-group")
        .with_children([section(
            "sec-group-cards",
            grp.title,
            clip_grid_c(cards, g.card_columns()),
        )])
}

/// 把卡片按 `COLUMNS` 切成若干横排行。
///
/// 与 `clip_grid` 的区别：那个收**静态**的 `&[(&str,&str,&str)]` + 静态 key 表，
/// 因为内容是编译期常量；这个收**运行时**收集的 `Vec<Element>`（key 来自
/// catalog 的静态字符串，所以 key 仍是 `'static` —— 身份稳定，diff 能正确复用）。
///
/// ⚠️ 不能用 `chunks()`：core 的 `Element` **不是 `Clone`** —— 它持有 `Cow`
/// 与 `Vec`，克隆一份就是一次多余深拷贝。所以这里用 `drain` 逐行取走，
/// 每张卡片只被移动一次、从不复制。
/// 把卡片切成若干横排行，每行 `columns` 张。
///
/// # `columns` 是断点的直接产物（官方的「重新排列」）
///
/// `docs/developer/responsive-and-touch.md` §6 引官方 `responsive-design` 页：当改用更大的
/// 屏幕时可以添加列。core 没有 Grid 也没有 flex-wrap，所以列数只能在视图层按
/// 可用宽度算 —— 这正是 [`Gallery::card_columns`] 的理由：
///
/// - 基准画布下仍是 5 列（`columns_for(1136, 200, 12) == 5`，有单测钉住）；
/// - 窄窗口自动降到 3/2/1 列，不再溢出被裁；
/// - 宽窗口上到 6/7 列，不再右侧留白。
///
/// # 为什么用 `split_off` 而不是 `chunks()`
///
/// core 的 `Element` 不是 `Clone`（持 `Cow` 与 `Vec`，克隆即深拷贝）。
/// 逐行取走则每张卡片只被移动一次。
///
/// 下面的旧注释保留作为历史记录：这个函数原来收无参、按常量 `COLUMNS` 切分。
pub fn clip_grid_c(mut cards: Vec<Element>, columns: usize) -> Vec<Element> {
    // 列数至少 1：否则 `split_off(0)` 反复切成空行，行容器无限增长。
    let columns = columns.max(1);
    let mut rows: Vec<Element> = Vec::new();
    while !cards.is_empty() {
        let take = cards.len().min(columns);
        let rest = cards.split_off(take);
        let chunk = std::mem::replace(&mut cards, rest);
        rows.push(
            Element::new(NodeTag::Container)
                .with_key(900 + rows.len() as u64)
                .style("clip-grid-row")
                .with_children(chunk),
        );
    }
    rows
}

// ---------------------------------------------------------- Basic input

fn basic_input(g: &Gallery) -> Element {
    Element::new(NodeTag::Container)
        .with_key("basic-input")
        .style("page-basic")
        .with_children([
            section(
                "sec-buttons",
                "Button",
                vec![row(
                    10,
                    vec![
                        button(
                            "btn-standard",
                            "Standard",
                            Some("pen"),
                            "standard",
                            g.hovers("btn-standard"),
                            g.holds("btn-standard"),
                        ),
                        button(
                            "btn-accent",
                            "Accent",
                            None,
                            "accent",
                            g.hovers("btn-accent"),
                            g.holds("btn-accent"),
                        ),
                        button(
                            "btn-subtle",
                            "Subtle",
                            Some("star"),
                            "subtle",
                            g.hovers("btn-subtle"),
                            g.holds("btn-subtle"),
                        ),
                    ],
                )],
            ),
            section(
                "sec-tools",
                "ToolButton",
                vec![row(
                    11,
                    vec![
                        tool_button(
                            "tool:menu",
                            "menu",
                            g.hovers("tool:menu"),
                            g.holds("tool:menu"),
                        ),
                        tool_button("tool:pen", "pen", g.hovers("tool:pen"), g.holds("tool:pen")),
                        tool_button(
                            "tool:eraser",
                            "eraser",
                            g.hovers("tool:eraser"),
                            g.holds("tool:eraser"),
                        ),
                        tool_button(
                            "tool:undo",
                            "undo",
                            g.hovers("tool:undo"),
                            g.holds("tool:undo"),
                        ),
                        tool_button(
                            "tool:image",
                            "image",
                            g.hovers("tool:image"),
                            g.holds("tool:image"),
                        ),
                        tool_button(
                            "tool:camera",
                            "camera",
                            g.hovers("tool:camera"),
                            g.holds("tool:camera"),
                        ),
                        tool_button(
                            "tool:gear",
                            "gear",
                            g.hovers("tool:gear"),
                            g.holds("tool:gear"),
                        ),
                        tool_button(
                            "tool:settings",
                            "settings",
                            g.hovers("tool:settings"),
                            g.holds("tool:settings"),
                        ),
                    ],
                )],
            ),
            section(
                "sec-selection",
                "Selection",
                vec![row(12, vec![check_col(g), radio_col(g), switch_col(g)])],
            ),
            section(
                "sec-slider",
                "Slider",
                vec![row(
                    13,
                    vec![
                        slider(
                            "slider:demo",
                            0.42,
                            Some("42"),
                            g.hovers("slider:demo"),
                            g.holds("slider:demo"),
                        ),
                        slider(
                            "slider:demo2",
                            0.78,
                            Some("78"),
                            g.hovers("slider:demo2"),
                            g.holds("slider:demo2"),
                        ),
                    ],
                )],
            ),
            section(
                "sec-more-buttons",
                "More buttons",
                vec![row(
                    14,
                    vec![
                        round_button(
                            "round:toggle",
                            "Star",
                            Some("star"),
                            g.round_on,
                            g.hovers("round:toggle"),
                            g.holds("round:toggle"),
                        ),
                        hyperlink(
                            "link:docs",
                            "ZephyraUI docs",
                            "https://ui.rinlit.cn/",
                            g.hovers("link:docs"),
                        ),
                        drop_down_button(
                            "dd:demo",
                            "Options",
                            Some("sliders"),
                            g.dd_open,
                            g.hovers("dd:demo"),
                            g.holds("dd:demo"),
                        ),
                    ],
                )],
            ),
            section(
                "sec-value-input",
                "Value input",
                vec![row(
                    15,
                    vec![
                        combo_box(
                            "combo:demo",
                            combo_label(g.combo),
                            g.hovers("combo:demo"),
                            g.holds("combo:demo"),
                        ),
                        spin_box(
                            "spin:demo",
                            spin_text(g.spin),
                            g.hovers("spin:demo"),
                            g.holds("spin:demo"),
                            g.hovers("spin:up"),
                            g.holds("spin:up"),
                            g.hovers("spin:down"),
                            g.holds("spin:down"),
                        ),
                    ],
                )],
            ),
            section(
                "sec-color",
                "ColorPicker",
                vec![color_picker("picker:demo", 5)],
            ),
        ])
}

/// SpinBox 的显示值（内容是数据；演示范围 0..100）。
fn spin_text(v: i32) -> &'static str {
    const TEXTS: [&str; 101] = [
        "0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12", "13", "14", "15", "16",
        "17", "18", "19", "20", "21", "22", "23", "24", "25", "26", "27", "28", "29", "30", "31",
        "32", "33", "34", "35", "36", "37", "38", "39", "40", "41", "42", "43", "44", "45", "46",
        "47", "48", "49", "50", "51", "52", "53", "54", "55", "56", "57", "58", "59", "60", "61",
        "62", "63", "64", "65", "66", "67", "68", "69", "70", "71", "72", "73", "74", "75", "76",
        "77", "78", "79", "80", "81", "82", "83", "84", "85", "86", "87", "88", "89", "90", "91",
        "92", "93", "94", "95", "96", "97", "98", "99", "100",
    ];
    TEXTS[v.clamp(0, 100) as usize]
}

/// 复选框一列。
fn check_col(g: &Gallery) -> Element {
    let labels = ["Enable alpha", "Enable beta", "Enable gamma"];
    let children: Vec<Element> = labels
        .iter()
        .enumerate()
        .map(|(i, label)| {
            let key = super::CHECKS[i];
            check_box(key, label, g.checked[i], g.hovers(key), g.holds(key))
        })
        .collect();
    stack(20, children)
}

/// 单选一列（选中态是演示数据，点击不切换 —— 单选互斥需要组状态，属于交互
/// 语义而非外观，这一版只展示长相）。
fn radio_col(g: &Gallery) -> Element {
    let children: Vec<Element> = [
        ("radio:a", "Option A", true),
        ("radio:b", "Option B", false),
    ]
    .iter()
    .map(|(key, label, checked)| radio_button(key, label, *checked, g.hovers(key), g.holds(key)))
    .collect();
    stack(21, children)
}

/// 开关一列。
fn switch_col(g: &Gallery) -> Element {
    let children: Vec<Element> = [("switch:a", true), ("switch:b", false)]
        .iter()
        .map(|(key, on)| toggle_switch(key, *on, g.hovers(key), g.holds(key)))
        .collect();
    stack(22, children)
}

// ------------------------------------------------------------ Collections

/// Collections 页：卡 / 展开器 / 分段 + 批 F 的列表、表格、滑动、头像、设置件。
fn collections(g: &Gallery) -> Element {
    let cards: Vec<Element> = ["card:a", "card:b", "card:c"]
        .iter()
        .enumerate()
        .map(|(i, key)| {
            card(
                key,
                "demo-card",
                vec![
                    text_label("card-title", "Frame", "text-body-strong"),
                    caption("card-desc", "cardColor 底 + 1px cardBorder 描边"),
                    badge::dot_badge("card-dot", if i == 2 { "caution" } else { "success" }),
                ],
            )
        })
        .collect();
    let list: &[(&str, &str)] = &[
        ("li:overview", "Overview"),
        ("li:gpu", "GPU 渲染"),
        ("li:style", "样式表"),
    ];
    let table_rows: &[&[&str]] = &[
        &["Button", "32px", "accent"],
        &["Slider", "4px 轨道", "fill:*"],
        &["Badge", "16px", "severity"],
    ];
    Element::new(NodeTag::Container)
        .with_key("collections")
        .style("page-collections")
        .with_children([
            section("sec-cards", "Cards", vec![row(30, cards)]),
            section(
                "sec-expander",
                "Expander",
                vec![expander(
                    "exp:details",
                    "Details",
                    g.expanded,
                    vec![
                        check_box(
                            "check:alpha",
                            "Indeterminate content",
                            g.checked[0],
                            g.hovers("check:alpha"),
                            g.holds("check:alpha"),
                        ),
                        caption("exp-caption", "折叠走 COLLAPSED：不占位、不绘制"),
                    ],
                )],
            ),
            section(
                "sec-segmented",
                "Segmented",
                vec![segmented(
                    "seg:view",
                    &[("list", "List"), ("grid", "Grid"), ("chart", "Chart")],
                    Some("grid"),
                    g.hovered.as_ref().and_then(|k| k.as_name()),
                    g.pressed.as_ref().and_then(|k| k.as_name()),
                )],
            ),
            section(
                "sec-list",
                "ListView & TableView",
                vec![row(
                    34,
                    vec![
                        list_view(
                            "list:demo",
                            list,
                            Some("li:style"),
                            g.hovered.as_ref().and_then(|k| k.as_name()),
                            g.pressed.as_ref().and_then(|k| k.as_name()),
                        ),
                        table_view("table:demo", &["Control", "Size", "Note"], table_rows),
                    ],
                )],
            ),
            section(
                "sec-swipe",
                "SwipeView & Avatar",
                vec![row(
                    35,
                    vec![
                        swipe_view(
                            "swipe:demo",
                            &[
                                ("Overview", "保留式 + DisplayList"),
                                ("Styling", "类名进，样式表出"),
                                ("Hosts", "D3D12 / CPU 参考"),
                            ],
                            1,
                        ),
                        stack(
                            36,
                            vec![
                                avatar("avatar:a", "ZU", true),
                                avatar("avatar:b", "RS", false),
                                setting_item("si:account", "gear", "Account", "本地账户"),
                            ],
                        ),
                    ],
                )],
            ),
            section(
                "sec-setting-expander",
                "SettingExpander",
                vec![setting_expander(
                    "sex:advanced",
                    "Advanced",
                    "展开后才看见的内容",
                    g.expanded,
                    vec![caption(
                        "sex-desc",
                        "折叠走 COLLAPSED，与 Expander 同一机制",
                    )],
                )],
            ),
        ])
}

/// Menus & Toolbars 页（批 D）。
fn menus_page(g: &Gallery) -> Element {
    Element::new(NodeTag::Container)
        .with_key("menus")
        .style("page-menus")
        .with_children([
            section(
                "sec-menus",
                "MenuBar & Menu",
                vec![row(
                    31,
                    vec![
                        menu_bar("menubar:demo", &["File", "Edit", "View"], g.menu_open),
                        menu(
                            "menu:file",
                            &[
                                ("New", "mi:new"),
                                ("Open…", "mi:open"),
                                ("", ""),
                                ("Preferences", "mi:prefs"),
                            ],
                        ),
                    ],
                )],
            ),
            section(
                "sec-menu-items",
                "MenuItem",
                vec![menu(
                    "menu:items",
                    &[
                        ("Show grid", "mi:grid"),
                        ("Snap to edges", "mi:snap"),
                        ("", ""),
                        ("More options", "mi:more"),
                    ],
                )],
            ),
        ])
}

/// Navigation 页（批 E）。
fn navigation_page(g: &Gallery) -> Element {
    Element::new(NodeTag::Container)
        .with_key("navigation")
        .style("page-navigation")
        .with_children([
            section(
                "sec-selector",
                "SelectorBar",
                vec![selector_bar(
                    "selector:demo",
                    &[("home", "Home"), ("all", "All"), ("settings", "Settings")],
                    Some("all"),
                    g.hovered.as_ref().and_then(|k| k.as_name()),
                    g.pressed.as_ref().and_then(|k| k.as_name()),
                )],
            ),
            section(
                "sec-sub-items",
                "NavigationSubItem",
                vec![stack(
                    32,
                    vec![
                        nav_sub_item(
                            "sub:overview",
                            "Overview",
                            Some("grid"),
                            true,
                            g.hovers("sub:overview"),
                            g.holds("sub:overview"),
                        ),
                        nav_sub_item(
                            "sub:shortcuts",
                            "Shortcuts",
                            Some("menu"),
                            false,
                            g.hovers("sub:shortcuts"),
                            g.holds("sub:shortcuts"),
                        ),
                    ],
                )],
            ),
        ])
}

/// Dialogs & Pickers 页（批 G）。
fn dialogs_page(g: &Gallery) -> Element {
    // 2026 年 10 月：10 月 1 日是周四（列 4）。
    let cells: &[Option<u8>] = &[
        None,
        None,
        None,
        None,
        Some(1),
        Some(2),
        Some(3),
        Some(4),
        Some(5),
        Some(6),
        Some(7),
        Some(8),
        Some(9),
        Some(10),
        Some(11),
        Some(12),
        Some(13),
        Some(14),
        Some(15),
        Some(16),
        Some(17),
        Some(18),
        Some(19),
        Some(20),
        Some(21),
        Some(22),
        Some(23),
        Some(24),
        Some(25),
        Some(26),
        Some(27),
        Some(28),
        Some(29),
        Some(30),
        Some(31),
    ];
    Element::new(NodeTag::Container)
        .with_key("dialogs")
        .style("page-dialogs")
        .with_children([
            section(
                "sec-dialog",
                "Dialog",
                vec![dialog(
                    "dialog:demo",
                    "Delete this file?",
                    "This file will be removed from this device. This action cannot be undone.",
                    &[("dlg:no", "Cancel", false), ("dlg:yes", "Delete", true)],
                    g.hovered.as_ref().and_then(|k| k.as_name()),
                    g.pressed.as_ref().and_then(|k| k.as_name()),
                )],
            ),
            section(
                "sec-pickers",
                "DatePicker & TimePicker",
                vec![row(
                    60,
                    vec![
                        stack(
                            61,
                            vec![
                                date_picker("dp:demo", cells, 12),
                                caption("dp-caption", "点选日期由宿主按 key 路由"),
                            ],
                        ),
                        stack(
                            62,
                            vec![
                                time_picker("tp:demo", &HOUR_WINDOW, "14", &MINUTE_WINDOW, "30"),
                                caption("tp-caption", "滚动选值降级为列表 + 选中高亮"),
                            ],
                        ),
                    ],
                )],
            ),
            section(
                "sec-materials",
                "Acrylic & Overlay",
                vec![row(
                    63,
                    vec![
                        acrylic_card(
                            "acrylic:demo",
                            vec![
                                text_label("acrylic-title", "AcrylicBrush", "text-body-strong"),
                                caption("acrylic-desc", "半透明卡（真实材质超出 core 能力）"),
                            ],
                        ),
                        rounded_corner_overlay(
                            "overlay:demo",
                            "r7",
                            vec![
                                text_label(
                                    "overlay-title",
                                    "RoundedCornerOverlay",
                                    "text-body-strong",
                                ),
                                caption("overlay-desc", "透明底 + 圆角 + 边"),
                            ],
                        ),
                    ],
                )],
            ),
        ])
}

/// 错误页（RinUI gallery 的 "Error Interface Test" 路由）。
fn error_page_route(g: &Gallery) -> Element {
    Element::new(NodeTag::Container)
        .with_key("error")
        .style("page-error")
        .with_children([section(
            "sec-error",
            "ErrorPage",
            vec![error_page(
                "error:demo",
                "Sorry, something went wrong!",
                "unexist/page load failed! Because of the stylesheet did not load. Please try again later.",
                "Retry",
                "retry:demo",
                g.hovers("retry:demo"),
                g.holds("retry:demo"),
            )],
        )])
}

// --------------------------------------------------------- Status & info

fn status_info(_g: &Gallery) -> Element {
    Element::new(NodeTag::Container)
        .with_key("status")
        .style("page-status")
        .with_children([
            section(
                "sec-progress",
                "ProgressBar",
                vec![stack(
                    40,
                    vec![
                        progress_bar("prog:running", 0.62, "running"),
                        progress_bar("prog:paused", 0.35, "paused"),
                        progress_bar("prog:error", 0.80, "error"),
                    ],
                )],
            ),
            section(
                "sec-badges",
                "InfoBadge",
                vec![
                    row(
                        41,
                        vec![
                            info_badge("badge:info", "12", "info", true),
                            info_badge("badge:success", "New", "success", true),
                            info_badge("badge:caution", "3", "caution", true),
                            info_badge("badge:error", "!", "error", true),
                        ],
                    ),
                    row(
                        42,
                        vec![
                            info_badge("badge:o-info", "12", "info", false),
                            info_badge("badge:o-success", "New", "success", false),
                            info_badge("badge:o-caution", "3", "caution", false),
                            info_badge("badge:o-error", "!", "error", false),
                        ],
                    ),
                    row(
                        43,
                        vec![
                            badge::dot_badge("dot:info", "info"),
                            badge::dot_badge("dot:success", "success"),
                            badge::dot_badge("dot:caution", "caution"),
                            badge::dot_badge("dot:error", "error"),
                        ],
                    ),
                ],
            ),
            section(
                "sec-ring",
                "ProgressRing & BusyIndicator",
                vec![row(
                    44,
                    vec![
                        progress_ring("ring:running", "62", "running", true),
                        progress_ring("ring:paused", "35", "paused", true),
                        progress_ring("ring:error", "80", "error", true),
                        busy_indicator("busy:0", 0),
                        busy_indicator("busy:1", 1),
                        busy_indicator("busy:2", 2),
                    ],
                )],
            ),
            section(
                "sec-infobar",
                "InfoBar & Toast",
                vec![stack(
                    45,
                    vec![
                        info_bar(
                            "bar:info",
                            "Heads up",
                            "样式表没加载上时界面长这样。",
                            "info",
                            true,
                        ),
                        info_bar(
                            "bar:success",
                            "Saved",
                            "28 条外观全部搬进了 demo.zstyle。",
                            "success",
                            true,
                        ),
                        toast(
                            "toast:caution",
                            "Caution",
                            "令牌拼错会静默退回默认值。",
                            "caution",
                        ),
                    ],
                )],
            ),
            section(
                "sec-indicators",
                "ToolTip & PageIndicator",
                vec![row(
                    46,
                    vec![
                        tool_tip("tip:demo", "Hover 目标的名字会显示在这里"),
                        page_indicator("pips:demo", 4, 1),
                    ],
                )],
            ),
        ])
}

// ------------------------------------------------------------------ Text

fn text_page(_g: &Gallery) -> Element {
    let lines: &[&str] = &[
        "ZephyraUI 文本族：",
        "TextField / TextArea / ScrollableTextArea /",
        "AutoSuggestBox / TextInputMenu。",
        "（核心没有输入法，值由宿主状态给出）",
    ];
    let suggestions: &[&str] = &["Mica", "Acrylic", "Tabbed", "None"];
    Element::new(NodeTag::Container)
        .with_key("text")
        .style("page-text")
        .with_children([
            section(
                "sec-typo",
                "Typography",
                vec![stack(
                    50,
                    vec![
                        text_label("typo-title", "Title — 20px", "text-title"),
                        text_label("typo-subtitle", "Subtitle — 16px", "text-subtitle"),
                        text_label("typo-body", "Body — 14px", "text-body"),
                        text_label("typo-strong", "BodyStrong — 14px", "text-body-strong"),
                        text_label("typo-caption", "Caption — 12px", "text-caption"),
                    ],
                )],
            ),
            section(
                "sec-field",
                "TextField",
                vec![stack(
                    51,
                    vec![
                        text_field("field:name", None, "Placeholder text", false, false, false),
                        text_field(
                            "field:value",
                            Some("Filled value"),
                            "Placeholder",
                            true,
                            false,
                            false,
                        ),
                    ],
                )],
            ),
            section(
                "sec-area",
                "TextArea",
                vec![stack(52, vec![text_area("area:demo", lines)])],
            ),
            section(
                "sec-scroll-area",
                "ScrollableTextArea",
                vec![scrollable_text_area("scroll:demo", lines, 0.4)],
            ),
            section(
                "sec-suggest",
                "AutoSuggestBox & TextInputMenu",
                vec![row(
                    53,
                    vec![
                        auto_suggest_box(
                            "suggest:demo",
                            None,
                            "Search controls...",
                            suggestions,
                            1,
                            true,
                            false,
                        ),
                        text_input_menu("menu:demo", &["Cut", "Copy", "Paste", "Select all"]),
                    ],
                )],
            ),
        ])
}

// -------------------------------------------------------------- Settings

fn settings_page(g: &Gallery) -> Element {
    Element::new(NodeTag::Container)
        .with_key("settings")
        .style("page-settings")
        .with_children([
            // 语言选择放在最上面一条：i18n 是"应用级"设置，与主题同级，
            // 而设置页其它条目都是内容/系统偏好。位置即语义。
            setting_card(
                "set-lang",
                g.tr("action.language"),
                // 这条描述刻意**不**翻译：它描述的是本 demo 的实现状态
                // （哪些接了 i18n、哪些还没），属于开发注记而不是产品文案。
                "Switching re-renders every string in the shell and settings",
                language_picker(g),
            ),
            setting_card(
                "set-theme",
                g.tr("set.theme.title"),
                g.tr("set.theme.desc"),
                toggle_switch(
                    "switch:master",
                    g.switch_on,
                    g.hovers("switch:master"),
                    g.holds("switch:master"),
                ),
            ),
            setting_card(
                "set-volume",
                g.tr("set.volume.title"),
                g.tr("set.volume.desc"),
                slider(
                    "slider:volume",
                    g.volume,
                    Some("62"),
                    g.hovers("slider:volume"),
                    g.holds("slider:volume"),
                ),
            ),
            setting_card(
                "set-notify",
                g.tr("set.notify.title"),
                g.tr("set.notify.desc"),
                toggle_switch(
                    "switch:notify",
                    !g.switch_on,
                    g.hovers("switch:notify"),
                    g.holds("switch:notify"),
                ),
            ),
            section(
                "sec-adv",
                g.tr("set.adv.title"),
                vec![
                    check_box(
                        "check:alpha",
                        g.tr("set.adv.logging"),
                        g.checked[0],
                        g.hovers("check:alpha"),
                        g.holds("check:alpha"),
                    ),
                    check_box(
                        "check:beta",
                        g.tr("set.adv.hwaccel"),
                        g.checked[1],
                        g.hovers("check:beta"),
                        g.holds("check:beta"),
                    ),
                    nav_item(
                        "nav-shortcut",
                        g.tr("set.adv.shortcuts"),
                        "menu",
                        false,
                        g.hovers("nav-shortcut"),
                        g.holds("nav-shortcut"),
                        false,
                    ),
                ],
            ),
        ])
}

/// 语言选择器：一个**分段控件**。
///
/// # 为什么是 `segmented` 而不是 `combo_box`
///
/// `combo_box` 只显示一个当前值、不表达"还有哪些可选"，而语言切换必须让人
/// 看见其它选项 —— 否则这个动作无从发现。`segmented` 正好是"多选一 + 全部可见"，
/// 而且**已经存在**：复用它等于不新增任何外观原语，也不用为它写新的 zstyle 规则。
/// 选中态由 `segmented` 自己发分类名，源码仍然不发颜色。
///
/// # 选项文案用**自称**而不是标签或英文名
///
/// "English / 中文 / 日本語"，不是 "en / zh-Hans / ja"，也不是"英语/中文/日语"。
/// 一个法国人看不懂 `zh-Hans`，也不一定认得"日语"的英文写法，但看得懂"日本語"。
/// 这是 i18n 的通行做法，不是本仓的风格选择。
///
/// # 段 id 用**语言标签**而不是下标
///
/// 下标（`0`/`1`/`2`）会在 `SUPPORTED` 增删时整体错位，而标签不会 ——
/// 错位的症状是"选了中文得到日文"，画面上完全看不出原因。
/// 点击路由因此是 `lang:zh-Hans` 这种形状（见 `Gallery` 的 `lang:` 分派）。
fn language_picker(g: &Gallery) -> Element {
    let items: Vec<(&'static str, &'static str)> = SUPPORTED
        .iter()
        .zip(LANG_SELFS)
        .map(|(tag, self_name)| (*tag, *self_name))
        .collect();
    segmented(
        "seg:lang",
        &items,
        Some(g.locale()),
        // `segmented` 收的是「当前 hover 的 key 名」而不是谓词：段的 key 就是
        // 语言标签，所以整条判定在控件内部做（`hovered_id == Some(*id)`）。
        g.hovered.as_ref().and_then(|k| k.as_name()),
        g.pressed.as_ref().and_then(|k| k.as_name()),
    )
}

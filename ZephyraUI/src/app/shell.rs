//! 窗口外壳：标题栏 + 导航栏 + 内容卡（对照 RinUI 的 FluentWindow /
//! NavigationView / FluentPage 三层）。
//!
//! # 固定画布
//!
//! RinUI gallery 是 1400×800。core 布局没有"百分比铺满"，所以这里同样按固定
//! 画布设计：所有外壳尺寸都是常量，且**样式表里的对应值由门禁逐字比对**
//! （`tests/gates.rs` 的 `shell_metrics_match_stylesheet`），不会出现
//! 「代码里 240、样式表里 220」这种两处真相。
//!
//! # "置底"怎么实现
//!
//! core 没有 flex-grow。RinUI 把 Settings 钉在导航栏底部，这里用一个
//! **定高占位**把 Settings 推到底 —— 占位高度是布局几何（不是外观），
//! 且有一条几何门禁 (`settings_lands_near_the_bottom`) 真去量布局后的矩形，
//! 偏差过大就红。改导航项数量时改常量，门禁会告诉你改得对不对。

use zephyra_core::element::Element;
use zephyra_core::id::NodeTag;
use zephyra_core::property::PropertyValue;
use zephyra_core::props;

use super::{pages, route_key, Gallery, ROUTES, SETTINGS_ROUTE};
use crate::controls::{badge, button, nav, text_label, tool_button};

/// 画布宽（RinUI gallery 的窗口宽）。
///
/// ⚠️ 它只是**基准画布**的宽度，不是窗口的宽度。窗口宽度由宿主按 `inner_size()`
/// 给（见 [`Gallery::viewport`]），外壳的宽度方向全部靠 `cross-align: stretch`
/// 铺满 —— 这里保留常量只为让基准尺寸有名字可引用。
pub const CANVAS_W: f32 = 1400.0;
/// 画布高。同 [`CANVAS_W`]：基准值，不是窗口高。
pub const CANVAS_H: f32 = 800.0;
/// 标题栏高（RinUI appearance.windowTitleBarHeight）。
pub const TITLEBAR_H: f32 = 48.0;
/// 导航栏宽（RinUI gallery 的 navExpandWidth 280 → 收一档取 240）。
pub const NAV_W: f32 = 240.0;
/// **紧凑档**的导航栏宽：只留图标列（WinUI NavigationView 的 `LeftCompact`）。
///
/// 出处见 `docs/responsive-and-touch.md` §6 引 `responsive-design` 页。取 56
/// 而不是更小：图标列本身是 32 宽（**4 的倍数**，官方要求基单位为 4 epx），
/// 加两侧 padding 后不能把图标压到 32 以下。
pub const NAV_W_COMPACT: f32 = 56.0;
/// 导航栏高 = 画布高 - 标题栏高。
pub const NAV_H: f32 = CANVAS_H - TITLEBAR_H;
/// 内容卡宽 = 画布宽 - 导航宽 - 1px 接缝（RinUI StackView 的 leftMargin）。
pub const CONTENT_W: f32 = CANVAS_W - NAV_W - 1.0;
/// 把置底项推到页面底部的定高占位（布局几何，由几何门禁校准）。
///
/// 校准值随路由项数变化：每加一项（含间距 41px）就把这里减 41。
/// 当前 10 项 + Settings：289。改错了 `shell_layout_geometry` 立刻红。
pub const NAV_SPACER: f32 = 248.0;

/// 整窗。
pub fn window(g: &Gallery) -> Element {
    Element::new(NodeTag::Container)
        .with_key("window")
        .style("window")
        .with_children([titlebar(g), body(g)])
}

/// 标题栏：站名 + 搜索框（视觉件）+ 三个窗口按钮。
fn titlebar(g: &Gallery) -> Element {
    Element::new(NodeTag::Container)
        .with_key("titlebar")
        .style("titlebar")
        // 「我想要 Mica 外观」是**应用层**的表达；「平台能不能开」由平台层回答。
        // 所以这里无条件发 `.mica`：平台不支持时，半透明标题栏叠在不透明 clear
        // 上仍是可读的半透明色 —— 降级不崩、且不引入 `cfg`。
        // （见 docs/win/02-mica.md §3）
        .styles(g.mica.then_some("mica"))
        .with_children([
            text_label("app-title", "ZephyraUI Gallery", "text-body-strong"),
            search_box(),
            caption_buttons(g),
        ])
}

/// 搜索框。**不可交互**：core 没有文本输入（见 README 的范围节）。
/// 它仍然给 hover 视觉 —— 那证明 hover 链对非 Control 容器也成立。
fn search_box() -> Element {
    Element::new(NodeTag::Container)
        .with_key("search")
        .style("search")
        .with_children([
            Element::new(NodeTag::Container)
                .with_key("search-icon")
                .with(&props::ICON_NAME, PropertyValue::static_str("search"))
                .style("search-icon"),
            text_label("search-hint", "Search controls and samples...", "text-caption"),
        ])
}

/// 窗口按钮：46×48 三枚。点击是 no-op（宿主没把窗口操作暴露给 View），
/// 但 hover / pressed 视觉是真的。
fn caption_buttons(g: &Gallery) -> Element {
    let caps = [("win:min", "minimize"), ("win:max", "move"), ("win:close", "close")];
    let children: Vec<Element> = caps
        .iter()
        .map(|(key, icon)| {
            let mut el = Element::new(NodeTag::Control)
                .with_key(*key)
                .with(&props::INTERACTIVE, PropertyValue::bool(true))
                .style("caption-btn");
            if *key == "win:close" {
                el = el.style("close");
            }
            for s in crate::controls::state(g.hovers(key), g.holds(key)) {
                el = el.style(s);
            }
            el.with_children([Element::new(NodeTag::Container)
                .with_key("icon")
                .with(&props::ICON_NAME, PropertyValue::static_str(icon))
                .style("caption-icon")])
        })
        .collect();
    Element::new(NodeTag::Container)
        .with_key("caption")
        .style("caption-bar")
        .with_children(children)
}

/// 主体：左导航 + 右内容卡。
///
/// # 高度为什么不在这里下发
///
/// core 没有 flex-grow（垂直 StackPanel 的第二个子项不占满剩余高），所以
/// `body` 的高度只能由**内容**决定：`max(导航内容高, 内容卡高)`。要真正跟随
/// 窗口高，缺的是 core 的主轴 fill 语义 —— 已记账在
/// `docs/responsive-and-touch.md` §8，不与本轮混做。
///
/// 这里只加 `cross-align: stretch`（合法地让宽度铺满，前者是交叉轴）。
fn body(g: &Gallery) -> Element {
    Element::new(NodeTag::Container)
        .with_key("body")
        .style("body")
        .with_children([nav_pane(g), content_card(g)])
}

/// 导航栏：顶部路由项 + 定高占位 + 置底的 Settings。
///
/// # 紧凑档
///
/// 官方的「自适应布局」示例正是这一处（`docs/responsive-and-touch.md` §6）：
/// 导航窗格的**精简版**适用于较小屏幕。这里把导航收成图标列（文字用
/// `COLLAPSED` 折叠、宽度交给样式表的 `.nav-pane.compact`），内容区随即
/// 拿到全宽 —— 而不是把 240 单位的侧栏浪费在窄窗口里。
fn nav_pane(g: &Gallery) -> Element {
    let compact = g.is_compact();
    let mut items: Vec<Element> = ROUTES
        .iter()
        .map(|(id, _en, icon)| {
            nav::nav_item(
                id,
                g.tr(route_key(id)),
                icon,
                g.is_route(id),
                g.hovers(id),
                g.holds(id),
                compact,
            )
        })
        .collect();
    items.push(
        Element::new(NodeTag::Container)
            .with_key("nav-spacer")
            .style("nav-spacer"),
    );
    let (id, _en, icon) = SETTINGS_ROUTE;
    items.push(nav::nav_item(
        id,
        g.tr(route_key(id)),
        icon,
        g.is_route(id),
        g.hovers(id),
        g.holds(id),
        compact,
    ));
    Element::new(NodeTag::Container)
        .with_key("nav")
        .style("nav-pane")
        // 紧凑是**外观**尺寸的来源（宽度收窄），所以这一份分类名必须发：
        // 窄宽由样式表的 `.nav-pane.compact` 给，这里只负责"我知道自己在紧凑档"。
        //
        // ⚠️ 宽度**不能**在这里下发：`tests/gates.rs::only_fill_nodes_carry_local_width`
        // 钉住"源码里写 WIDTH 只允许 fill:* 形态"，而它的注释明说
        // 「黑名单会烂，形状不会」。那正是本仓「源码零外观」规矩的一部分。
        .styles(compact.then_some("compact"))
        .with_children(items)
}

/// 内容卡：页头（标题 + 徽章 + 工具行）+ 页体。
///
/// 宽高都不写死：宽度由 `cross-align: stretch` 铺满导航之外的剩余。
/// 高度仍是 zstyle 的 752 —— 那会让 page 内容超出时被裁，而这是**已知边界**：
/// core 既无 flex-grow 也无滚动，固定高度是唯一的取舍（`MIGRATION.md` §7.4
/// 已经为"Home 只能放 10 条"记过同一笔）。本轮记账见
/// `docs/responsive-and-touch.md` §8。
fn content_card(g: &Gallery) -> Element {
    Element::new(NodeTag::Container)
        .with_key("content")
        .style("content-card")
        .with_children([page_header(g), page_body(g)])
}

/// 页头：左标题 + 徽章，右工具行（主题切换 / 反馈 / 收藏）。
fn page_header(g: &Gallery) -> Element {
    let (title_key, _icon) = current_route(g);
    Element::new(NodeTag::Container)
        .with_key("header")
        .style("page-header")
        .with_children([
            Element::new(NodeTag::Container)
                .with_key("header-title")
                .style("header-title")
                .with_children([
                    text_label("page-title", g.tr(title_key), "text-title"),
                    badge::info_badge("page-badge", "12", "info", true),
                ]),
            Element::new(NodeTag::Container)
                .with_key("header-tools")
                .style("header-tools")
                .with_children([
                    tool_button("theme-toggle", "light", g.hovers("theme-toggle"), g.holds("theme-toggle")),
                    button(
                        "feedback",
                        g.tr("action.feedback"),
                        Some("bell"),
                        "subtle",
                        g.hovers("feedback"),
                        g.holds("feedback"),
                    ),
                ]),
        ])
}

/// 当前路由的标题与图标。
fn current_route(g: &Gallery) -> (&'static str, &'static str) {
    ROUTES
        .iter()
        .chain(std::iter::once(&SETTINGS_ROUTE))
        .find(|(id, _, _)| *id == g.route)
        .map(|(id, _, i)| (route_key(id), *i))
        .unwrap_or((route_key("home"), "home"))
}

/// 页体：按路由分发。
fn page_body(g: &Gallery) -> Element {
    let page = pages::page(g);
    Element::new(NodeTag::Container)
        .with_key("page")
        .style("page")
        .with_children([page])
}

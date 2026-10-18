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
use zephyra_ui::controls::{badge, button, nav, text_label, tool_button};

/// 画布宽（RinUI gallery 的窗口宽）。
///
/// ⚠️ 它只是**基准画布**的宽度，不是窗口的宽度。窗口宽度由宿主按 `inner_size()`
/// 给（见 `Gallery::viewport`），外壳的宽度方向全部靠 `cross-align: stretch`
/// 铺满 —— 这里保留常量只为让基准尺寸有名字可引用。
pub const CANVAS_W: f32 = 1400.0;
/// 画布高。同 [`CANVAS_W`]：基准值，不是窗口高。
pub const CANVAS_H: f32 = 800.0;
/// 标题栏高（Apple HIG toolbar 高度）。
pub const TITLEBAR_H: f32 = 44.0;
/// 导航栏宽（Apple HIG 侧栏宽度）。
pub const NAV_W: f32 = 260.0;
/// **紧凑档**的导航栏宽：只留图标列（WinUI NavigationView 的 `LeftCompact`）。
///
/// 出处见 `docs/developer/responsive-and-touch.md` §6 引 `responsive-design` 页。取 56
/// 而不是更小：图标列本身是 32 宽（**4 的倍数**，官方要求基单位为 4 epx），
/// 加两侧 padding 后不能把图标压到 32 以下。
pub const NAV_W_COMPACT: f32 = 56.0;
/// 导航栏高 = 画布高 - 标题栏高。
pub const NAV_H: f32 = CANVAS_H - TITLEBAR_H;
/// 内容卡宽 = 画布宽 - 导航宽 - 1px 接缝（RinUI StackView 的 leftMargin）。
pub const CONTENT_W: f32 = CANVAS_W - NAV_W - 1.0;
/// 把置底项推到页面底部的定高占位（布局几何，由几何门禁校准）。
///
/// # 校准公式
///
/// ```text
/// NAV_H - 2*padding - (NAV_ITEMS*item_h + NAV_ITEMS*spacing) - item_h - 底部留白
/// ```
///
/// 当前 8 个导航项（7 路由 + Settings 置底）、项高 40、间距 6、上下 padding 各 12：
/// `756 - 24 - 7*40 - 6*6 - 40 - 44 = 364`（7 项之间只有 6 个间距）。
///
/// ⚠ 改路由数量时**必须**重算这里：差一项（46）时 `shell_layout_geometry`
/// 会立刻红，而症状在画面上只是「置底项浮在半空」，不看几何门禁很容易归咎
/// 成「间距不对」而去改样式表。
pub const NAV_SPACER: f32 = 364.0;

/// 导航项数（= `ROUTES` 的长度 + Settings 置底项）。
///
/// 显式写成一个常量而不是 `ROUTES.len()`：几何门禁要比对**算出来的数**与
/// **样式表里的数**，用 `.len()` 会让两边永远相等，于是改路由数量时门禁失去作用。
pub const NAV_ITEMS: usize = 8;

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
        // （见 docs/developer/win/02-mica.md §3）
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
            text_label(
                "search-hint",
                "Search controls and samples...",
                "text-caption",
            ),
        ])
}

/// 窗口按钮：46×48 三枚。点击是 no-op（宿主没把窗口操作暴露给 View），
/// 但 hover / pressed 视觉是真的。
fn caption_buttons(g: &Gallery) -> Element {
    let caps = [
        ("win:min", "minimize"),
        ("win:max", "move"),
        ("win:close", "close"),
    ];
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
            for s in zephyra_ui::controls::state(g.hovers(key), g.holds(key)) {
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
/// `docs/developer/responsive-and-touch.md` §8，不与本轮混做。
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
/// 官方的「自适应布局」示例正是这一处（`docs/developer/responsive-and-touch.md` §6）：
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
/// `docs/developer/responsive-and-touch.md` §8。
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
                    // **全应用**的动作回显：任何 `act:` 按钮点了都把 key 写进
                    // `Gallery::last_action`，这一行是那份状态唯一的视图。
                    //
                    // # 为什么放在外壳而不是各页页首
                    //
                    // 反馈必须**在当前页可见**。放在 basics 页首的话，layout /
                    // styling / menus 章的按钮点了会改状态、但开发者眼前的树
                    // 纹丝不动 —— 与「按钮没接线」无法区分。外壳里的这一行
                    // 与路由无关，于是「点击 → 状态 → 视图」在每一页都能当场
                    // 验证。（门禁 `every_interactive_key_is_wired_or_whitelisted`
                    // 正是按「本页树指纹变没变」判的，它先一步抓出了这个坑。）
                    text_label("last-action", g.last_action, "text-caption"),
                    tool_button(
                        "theme-toggle",
                        "light",
                        g.hovers("theme-toggle"),
                        g.holds("theme-toggle"),
                    ),
                    button(
                        "act:feedback",
                        g.tr("action.feedback"),
                        Some("bell"),
                        "subtle",
                        g.hovers("act:feedback"),
                        g.holds("act:feedback"),
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
        .unwrap_or((route_key("guide"), "home"))
}

/// 页体：按路由分发。
///
/// # 入场动画
///
/// 只发 `.enter-<n>` 分类名，**不**在源码里写 opacity / translate。
/// 每一档长什么样全在 zstyle 的 `.enter-1` … `.enter-8` 里。
///
/// 为什么动画挂在**页体**而不是内容卡：页卡是圆角 + 描边的"卡片"，
/// 让它半透明会透出底下的导航与背景，而整页一起淡入才是官方页面刷新的形态
/// （`motion/page-transitions`：向上滑动 + 淡入的组合）。
fn page_body(g: &Gallery) -> Element {
    let page = pages::page(g);
    Element::new(NodeTag::Container)
        .with_key("page")
        .style("page")
        // `styles` 而不是 `style`：`.page` 是常驻分类，`.enter-<n>` 是这帧的动画档。
        // 两者都要，所以走"追加"那个入口（`style` 是覆盖）。
        .styles(g.enter_class())
        .with_children([page])
}

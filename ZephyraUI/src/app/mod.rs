//! Gallery 应用：RinUI 示例应用（gallery）的 Zephyra 重写版。
//!
//! # 状态从哪来
//!
//! `InputState` 只给 hover / pressed（见 `zephyra_fx::InputState`）。click 是
//! 按下与抬起的配对，而配对信息在 `on_input` 的两次调用之间 —— 所以这里自己
//! 记一份 `last_pressed`，在「上一帧按着、这一帧松了」时还原出 click。
//! 与 `InputRouter` 的 `RoutedEvent::Click` 同一条判据（同键才算点中），
//! 放在 View 侧不丢信息，也不需要改宿主。
//!
//! # 主题为什么是「意图」
//!
//! `&Theme` 由宿主持有（窗口里它是 `&'static`）。View 只发意图
//! （`wants_dark`），宿主每帧对账后执行 `set_theme` —— 于是"想切的"与
//! "在用的"永远可分开观测，不会出现"点了没反应但也不知道为什么"。

use crate::i18n::{Messages, Preferences, Tag};
use zephyra_core::id::ElementKey;
use zephyra_core::{context::Context, element::Element};
use zephyra_fx::{InputState, View};

pub mod catalog;
pub mod pages;
pub mod shell;
pub mod showcase;
pub mod testbed;

/// 路由表：`(id, 标题, 图标)`。导航栏从左到右、Settings 置底。
pub const ROUTES: &[(&str, &str, &str)] = &[
    ("home", "Home", "home"),
    ("all", "All samples", "grid"),
    ("basic-input", "Basic Input", "pen"),
    ("collections", "Collections", "doc"),
    ("menus", "Menus & Toolbars", "settings"),
    ("status", "Status & Info", "bell"),
    ("text", "Text", "search"),
    ("navigation", "Navigation", "menu"),
    ("dialogs", "Dialogs & Pickers", "calendar"),
    ("error", "Error Test", "close"),
    // 测试功能页：原 examples/demo 的 7 项测试能力保留在这里。
    ("testbed", "Test Features", "sliders"),
];

/// 基准画布尺寸（物理像素）= 宿主建窗口时的初值。
///
/// 它同时是[`Gallery`] 在 dump 路下的默认视口，于是静态图与窗口路共用同一套
/// 断点。Rin-UI 原版按桌面大窗口设计，所以基准落在官方"大"档（1120 epx）。
pub const BASE_VIEWPORT: (u32, u32) = (1400, 800);

/// 内容区两侧的页边距（布局单位）。与 `pages.rs` 里的 section 间距同源，
/// 由 `tests::gates::content_width_follows_breakpoint` 间接钉住。
pub const PAGE_GUTTER: f32 = 28.0;

/// 本 demo 支持的语言（有序，**第一项是默认/回退语言**）。
///
/// 顺序有语义：协商匹配不到任何用户偏好时回落第一项 —— 这与浏览器把默认
/// 语言放在 "supported locales" 同一形状（`Intl` 的 preference-based fallback
/// 即按这个顺序取第一个匹配）。
///
/// 顺带这也是 `Messages::select` 的 `fallback` 参数。
pub const SUPPORTED: &[&str] = &["en", "zh-Hans", "ja"];

/// 各语言的自称，下标与 [`SUPPORTED`] 对齐。
///
/// # 为什么**不**进文案表
///
/// 自称不是翻译：任何语言的用户看到的都是同一组"English / 中文 / 日本語"。
/// 放进表里就得给 3 种语言各填 3 条共 9 条，而那 9 条内容两两相同 ——
/// 表格里的重复迟早会被误改成"翻译"，而那正是错的。
///
/// 对齐关系由门禁 `i18n_tests::self_described_locale_count_matches_supported_list` 钉住：
/// 下标错位会让用户选了"English"却得到日文，而那在画面上完全看不出原因。
pub const LANG_SELFS: &[&str] = &["English", "中文", "日本語"];

/// 文案表：`(locale, key, value)` 三元组，一行一条。
///
/// # 为什么是一张平表而不是 `HashMap<locale, HashMap<key, value>>`
///
/// 两种都能取到值，但平表让「门禁枚举漏翻」变成一行 `CATALOG.iter()`
/// —— 而漏翻是 i18n 唯一真正危险的东西（其余问题最多是显示难看）。
/// 分语言分文件是**将来**运行期加载时才需要的形态，见 `docs/developer/i18n.md` §4。
///
/// # 覆盖范围与未覆盖范围（不要误读这张表）
///
/// 已接进视图：导航标题（11 条路由）、页头标题、应用名、页头工具按钮、
/// 设置页七条。**未接**：控件展台 44 张卡与各页正文 —— 那些仍是源码里的
/// 中文硬编码。这是**记账的债**，不是"已支持"：门禁
/// `i18n_tests::no_locale_is_missing_keys` 覆盖的是本表列出的键，
/// `i18n_tests::未接入的硬编码文案已记账` 钉住剩下的数量，见 `docs/developer/i18n.md` §5。
const CATALOG: &[(&str, &str, &str)] = &[
    // ---- en（默认语言）----
    ("en", "app.title", "ZephyraUI Gallery"),
    ("en", "route.home", "Home"),
    ("en", "route.all", "All samples"),
    ("en", "route.basic-input", "Basic Input"),
    ("en", "route.collections", "Collections"),
    ("en", "route.menus", "Menus & Toolbars"),
    ("en", "route.status", "Status & Info"),
    ("en", "route.text", "Text"),
    ("en", "route.navigation", "Navigation"),
    ("en", "route.dialogs", "Dialogs & Pickers"),
    ("en", "route.error", "Error Test"),
    ("en", "route.testbed", "Test Features"),
    ("en", "route.settings", "Settings"),
    ("en", "action.feedback", "Feedback"),
    ("en", "action.language", "Language"),
    ("en", "set.theme.title", "Dark mode"),
    (
        "en",
        "set.theme.desc",
        "Switching theme only swaps token values",
    ),
    ("en", "set.volume.title", "Volume"),
    ("en", "set.volume.desc", "Fill width = value x track width"),
    ("en", "set.notify.title", "Notifications"),
    (
        "en",
        "set.notify.desc",
        "Selected state travels as a class name",
    ),
    ("en", "set.adv.title", "Advanced"),
    ("en", "set.adv.logging", "Diagnostic logging"),
    ("en", "set.adv.hwaccel", "Hardware acceleration"),
    ("en", "set.adv.shortcuts", "Shortcuts"),
    // ---- zh-Hans ----
    ("zh-Hans", "app.title", "ZephyraUI 示例库"),
    ("zh-Hans", "route.home", "主页"),
    ("zh-Hans", "route.all", "全部示例"),
    ("zh-Hans", "route.basic-input", "基础输入"),
    ("zh-Hans", "route.collections", "集合"),
    ("zh-Hans", "route.menus", "菜单与工具栏"),
    ("zh-Hans", "route.status", "状态与信息"),
    ("zh-Hans", "route.text", "文本"),
    ("zh-Hans", "route.navigation", "导航"),
    ("zh-Hans", "route.dialogs", "对话框与选择器"),
    ("zh-Hans", "route.error", "错误测试"),
    ("zh-Hans", "route.testbed", "测试功能"),
    ("zh-Hans", "route.settings", "设置"),
    ("zh-Hans", "action.feedback", "反馈"),
    ("zh-Hans", "action.language", "语言"),
    ("zh-Hans", "set.theme.title", "深色模式"),
    ("zh-Hans", "set.theme.desc", "换主题只换令牌值"),
    ("zh-Hans", "set.volume.title", "音量"),
    ("zh-Hans", "set.volume.desc", "填充宽度 = 值 × 轨道宽"),
    ("zh-Hans", "set.notify.title", "通知"),
    ("zh-Hans", "set.notify.desc", "选中态走分类名"),
    ("zh-Hans", "set.adv.title", "高级"),
    ("zh-Hans", "set.adv.logging", "诊断日志"),
    ("zh-Hans", "set.adv.hwaccel", "硬件加速"),
    ("zh-Hans", "set.adv.shortcuts", "快捷键"),
    // ---- ja ----
    ("ja", "app.title", "ZephyraUI ギャラリー"),
    ("ja", "route.home", "ホーム"),
    ("ja", "route.all", "すべてのサンプル"),
    ("ja", "route.basic-input", "基本入力"),
    ("ja", "route.collections", "コレクション"),
    ("ja", "route.menus", "メニューとツールバー"),
    ("ja", "route.status", "ステータスと情報"),
    ("ja", "route.text", "テキスト"),
    ("ja", "route.navigation", "ナビゲーション"),
    ("ja", "route.dialogs", "ダイアログとピッカー"),
    ("ja", "route.error", "エラーテスト"),
    ("ja", "route.testbed", "テスト機能"),
    ("ja", "route.settings", "設定"),
    ("ja", "action.feedback", "フィードバック"),
    ("ja", "action.language", "言語"),
    ("ja", "set.theme.title", "ダークモード"),
    (
        "ja",
        "set.theme.desc",
        "テーマを切り替えてもトークンの値だけが変わる",
    ),
    ("ja", "set.volume.title", "音量"),
    ("ja", "set.volume.desc", "塗りの幅 = 値 x トラックの幅"),
    ("ja", "set.notify.title", "通知"),
    ("ja", "set.notify.desc", "選択状態はクラス名で表す"),
    ("ja", "set.adv.title", "詳細"),
    ("ja", "set.adv.logging", "診断ログ"),
    ("ja", "set.adv.hwaccel", "ハードウェアアクセラレーション"),
    ("ja", "set.adv.shortcuts", "ショートカット"),
];

/// 本表里出现过的**全部**键（按首次出现去重）。
///
/// 门禁拿它当"应该存在的键集合"：一张平表天然可以枚举，这是选平表而不是
/// 分语言分桶的**唯一**理由（见 [`CATALOG`] 的说明）。
pub fn catalog_keys() -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for (_, k, _) in CATALOG {
        if !out.contains(k) {
            out.push(k);
        }
    }
    out
}

/// 应用内的消息表。
///
/// # 为什么文案在这里而不在控件里
///
/// 源码里**没有面向用户的字符串** —— 与"源码零外观"是同一条纪律：
/// 控件发分类名、不发颜色；页面发 key、不发文案。漏翻由门禁枚举
/// （`i18n_tests::no_locale_is_missing_keys`），不靠肉眼。
pub fn messages() -> Messages {
    let mut m = Messages::new();
    for (locale, key, value) in CATALOG {
        m.insert(locale, key, value);
    }
    m
}
/// 路由 id → 文案键（`home` ⇒ `route.home`）。
///
/// # 为什么是**查表**而不是 `concat!("route.{id}")`
///
/// `view()` 每帧重建整棵树，任何运行期拼出来的字符串都会变成泄漏
/// （`Box::leak` 每帧一次，见 `element.rs` 的记录）。而键必须 `&'static`
/// 才能喂给 [`Gallery::tr`]。查表天然返回 `&'static str`。
///
/// 找不到时**返回 id 本身**而不是 `panic!` 或空串：调用方随后会拿它当键去
/// `tr()`，而 `tr()` 的最后一档正是"返回键名"，于是画面上出现路由 id ——
/// 看得见的降级。真正拦住这件事的是门禁
/// `i18n_tests::every_route_has_a_copy_key`（新增路由忘了加译文会红）。
pub fn route_key(id: &str) -> &'static str {
    // 只在默认语言那一组里找键 —— 键集合与语言无关，看第一组就够且更快。
    for (locale, key, _) in CATALOG {
        if *locale != SUPPORTED[0] {
            continue;
        }
        if let Some(rest) = key.strip_prefix("route.") {
            if rest == id {
                return key;
            }
        }
    }
    // 静态回退：id 是 `&'static`（来自 `ROUTES` / `SETTINGS_ROUTE`），
    // 所以这里不需要额外分配。若调用方传了非静态串，门禁会先拦住。
    ROUTES
        .iter()
        .chain(std::iter::once(&SETTINGS_ROUTE))
        .find(|(rid, _, _)| *rid == id)
        .map(|(rid, _, _)| *rid)
        .unwrap_or("route.unknown")
}

/// 解析一个 key 是否为合法路由，是则返回该路由 id（`&'static`）。
///
/// # 路由来源有三处，缺一不可
///
/// 1. 10 个页面路由（[`ROUTES`]）—— 导航栏项；
/// 2. 14 个分组索引路由（[`catalog::GROUP_ROUTES`]）—— All 页的分组条；
/// 3. 44 个控件卡（[`catalog::ENTRIES`] 的 `page`）—— 卡片点击进该控件页。
///
/// 三处都用**查表**而非 `match` 列举：任一处新增 id 都只改那一处表，
/// 不用回来改这个函数（枚举式会把新路由漏在 match 的 `_` 分支里，静默回 Home）。
///
/// 返回 `&'static str`：路由要存进 [`Gallery::route`]（`&'static`），
/// 不能在运行时拼字符串 —— 那会每帧泄漏。
pub fn resolve_route(key: &str) -> Option<&'static str> {
    // 1) 页面路由与 Settings。
    if ROUTES.iter().any(|(id, _, _)| *id == key) || key == SETTINGS_ROUTE.0 {
        return Some(key_of_static(key));
    }
    // 2) 分组索引路由：`group:<id>`。
    if catalog::group_of_route(key).is_some() {
        return Some(key_of_static(key));
    }
    // 3) 控件卡：`page` 直接就是路由 id（点卡片进该控件页）。
    if catalog::entry(key).is_some() {
        return Some(key_of_static(key));
    }
    None
}

/// 把一个"刚查过表存在"的 `&str` 收窄成 `&'static str`。
///
/// # 为什么要这个函数而不是直接返回 `key`
///
/// 调用方传进来的是 `&str`（来自 `ElementKey::as_name()`），但三张表里存的都是
/// `&'static str`。这里在**确认存在之后**返回表里那一份静态引用（不是 `key`
/// 本身），从而把生命周期安全地收回 `&'static`。
///
/// `key_of_static` 走三张表找同一个 key；找不到返回 `"home"`（兜底）——
/// 走到这里的 `key` 必然在某张表里，所以兜底**不会**被触发，
/// 但保留它让函数对任何输入都返回合法路由（`match` 里不留 panic）。
fn key_of_static(key: &str) -> &'static str {
    if let Some((id, _, _)) = ROUTES.iter().find(|(id, _, _)| *id == key) {
        return id;
    }
    if key == SETTINGS_ROUTE.0 {
        return SETTINGS_ROUTE.0;
    }
    if let Some((route, _)) = catalog::GROUP_ROUTES.iter().find(|(r, _)| *r == key) {
        return route;
    }
    if let Some(e) = catalog::entry(key) {
        return e.page;
    }
    "home"
}
/// 列出全部合法路由（`--list-routes` 用）。
///
/// 之所以要"列出来"而不是让人猜：出图/调试时拼错一个 id 的后果是
/// **静默回到 home**（历史上就发生过，且 20 张图全同还没人察觉）。
/// 把全集打印出来，拼错立刻可见。
pub fn list_routes() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = ROUTES.iter().map(|(id, _, _)| *id).collect();
    v.push(SETTINGS_ROUTE.0);
    v.extend(catalog::GROUP_ROUTES.iter().map(|(r, _)| *r));
    v.extend(catalog::ENTRIES.iter().map(|e| e.page));
    v
}

/// Settings 路由（置底那一项）。
pub const SETTINGS_ROUTE: (&str, &str, &str) = ("settings", "Settings", "settings");

/// 演示开关的三个 id（Basic Input 页）。
pub const CHECKS: [&str; 3] = ["check:alpha", "check:beta", "check:gamma"];

/// ComboBox 演示选项（`'static`：控件签名收静态切片，与 demo 的常量同手法）。
pub const COMBO_OPTIONS: &[&str] = &["Mica", "Acrylic", "Tabbed", "None"];

/// Gallery 应用状态。
#[derive(Debug, Clone)]
pub struct Gallery {
    /// 当前路由 id。
    pub route: &'static str,
    /// 当前 hover 的 key（来自 InputState）。
    pub hovered: Option<ElementKey>,
    /// 当前按住的 key（捕获语义）。
    pub pressed: Option<ElementKey>,
    /// 上一帧的 pressed —— 用来还原 click。
    pub last_pressed: Option<ElementKey>,
    /// 想要深色主题（意图；宿主执行）。
    pub wants_dark: bool,
    /// 主题意图的共享格（窗口路用；dump 路直接读 `wants_dark`）。
    pub theme_intent: Option<std::sync::Arc<std::sync::atomic::AtomicU8>>,
    /// Home 页分段选中项。
    pub seg: &'static str,
    /// Basic Input 页的三个复选框。
    pub checked: [bool; 3],
    /// 设置页总开关。
    pub switch_on: bool,
    /// 设置页滑条值（0..1）。
    pub volume: f32,
    /// 设置页展开器是否展开。
    pub expanded: bool,
    /// Basic Input 页：ComboBox 选中项下标。
    pub combo: usize,
    /// Basic Input 页：SpinBox 的数值。
    pub spin: i32,
    /// Basic Input 页：RoundButton 的选中态。
    pub round_on: bool,
    /// Basic Input 页：DropDownButton 的展开态。
    pub dd_open: bool,
    /// 是否请求 Mica 材质外观（标题栏半透明）。true = 应用想要；false = 实色。
    /// 平台能不能真的开由 zephyra-platform-win 回答，不影响这里。
    pub mica: bool,
    /// Menus 演示：菜单栏当前展开项（None = 全收起）。
    pub menu_open: Option<usize>,
    /// 客户区视口，**物理像素**。宿主每帧经 `on_input` 推进来。
    ///
    /// 初始值取基准画布尺寸（见 [`crate::app::BASE_VIEWPORT`]）：dump 路没有宿主，
    /// 于是静态渲染下的断点与 1400×800 窗口一致 —— 否则 `--dump-cpu` 出的图
    /// 会按"小型窗口"排，与窗口路对不上，而那条对照正是参考图的全部价值。
    pub viewport: (f32, f32),
    /// 视口缩放比。它与 [`Self::viewport`] 一起才能算出有效像素（断点口径）。
    /// 初始 1.0 = 「dump 路当作未缩放处理」，与上面同源。
    pub scale: f32,
    /// 当前语言在 [SUPPORTED] 里的**下标**。
    ///
    /// 存下标而不是 Tag：Tag 每帧都要重新构造，而下标是 Copy 且能直接当
    /// selected 传给控件 —— 与 combo / spin 同一手法。
    pub lang: usize,
    /// 用户的系统语言偏好（有序，最偏好的在前）。
    ///
    /// 首帧由宿主经 on_input 推进来。空表 = 读不到 ⇒ 回落 [SUPPORTED] 第一项，
    /// **不**把界面钉死在某一门语言上（与「读不到系统主题就不跟随」同一纪律）。
    pub prefs: Preferences,
    /// 入场动画已过去的**毫秒**数。
    ///
    /// # 为什么是应用层的字段而不是 core 的
    ///
    /// 动画是**状态**（与 `route` / `hovered` 同层），而 core 的每帧入口只有
    /// `view()` 与 `on_input()`。所以时钟必须是 `Gallery` 的字段，由
    /// `View::on_input` 用 `InputState.dt_ms` 推进 —— 那条路径**每帧必调**，
    /// 不需要给 core 加"每帧回调"。
    ///
    /// # 为什么按**路由**归零
    ///
    /// 换路由 = 换内容 = 重放入场，这正是官方页面刷新的定义（`docs/developer/animation.md` §1）。
    /// 自己额外记一个"上一页 id"就要多一个字段与一次比较，而那样还会漏掉
    /// "同一路由内内容变了"（测试页的状态更新就不该重放入场）。
    pub enter_ms: f32,
    /// 上一次推进时钟时的路由（`&'static`，与 `route` 同类）。
    ///
    /// 存在的唯一理由是**要在推进之前**发现路由变了。归零后把它记成新路由，
    /// 下一帧才不会又归零一次（症状是"入场动画一帧都不播"）。
    /// 不存它就得靠"进入的这一刻"传参，而 `on_input` 每帧只被调一次、拿不到
    /// "事件"——只有状态。
    entered_route: &'static str,
}

impl Gallery {
    /// 有效像素宽度（官方断点的口径）。
    ///
    /// 宿主给的视口是物理像素，这里除 scale。判据见
    /// `docs/developer/responsive-and-touch.md` §2 与 [`crate::responsive::WidthClass`]。
    pub fn width_epx(&self) -> f32 {
        let scale = if self.scale.is_finite() && self.scale > 0.0 {
            self.scale
        } else {
            1.0
        };
        self.viewport.0 / scale
    }

    /// 当前断点档位。
    pub fn width_class(&self) -> crate::responsive::WidthClass {
        crate::responsive::WidthClass::of_viewport(self.viewport, self.scale)
    }

    /// 是否为紧凑档（小屏，导航折叠、单列布局）。
    ///
    /// 这一条是**结构**判据，不是外观判据：折叠不折叠、几列、显不显示某块，
    /// 都是树拓扑，属于视图职责；外观值仍在 zstyle（见
    /// `docs/developer/responsive-and-touch.md` §7）。
    pub fn is_compact(&self) -> bool {
        self.width_class().is_compact()
    }

    /// 内容区在**布局单位**下的可用宽度。
    ///
    /// 基准画布下 = 1400(视口) − [`crate::app::shell::NAV_W`] − 28(页边距) = 1136，
    /// 与现有卡片网格 5 列一致（`columns_for(1136, 200, 12) == 5`）。
    ///
    /// 导航折叠时导航宽度归零（内容区扩大到满宽）—— 这也是官方「重新排列」的做法：
    /// 小屏不把 [`NAV_W`](crate::app::shell::NAV_W) 单位浪费在一个看不见的侧栏上。
    ///
    /// # 为什么导航宽要引用常量而不是写死
    ///
    /// 这里曾经写死 `236.0`，而 `shell::NAV_W` 实际是 232。列数于是按错 4 单位算 ——
    /// 而 4 单位小到看不出，只在某些宽度上让「差一点点放不下第 5 列」。
    /// 那条错是被 `tests::gates::content_width_follows_breakpoint` 抓到的，不是被眼看出来的。
    pub fn content_width(&self) -> f32 {
        let nav = if self.is_compact() {
            0.0
        } else {
            crate::app::shell::NAV_W
        };
        (self.viewport.0 - nav - PAGE_GUTTER).max(0.0)
    }

    /// 卡片网格列数：按可用宽度算，且不小于当前档位的下限。
    ///
    /// 卡宽取 200（本仓现有值），因此基准画布下仍是 **5 列** —— 这一条
    /// 既是"没破坏基准外观"的证据，也是列数边界值的回归：
    /// 见 `responsive.rs` 的 `columns_for(1136, 200, 12) == 5`。
    pub fn card_columns(&self) -> usize {
        use crate::responsive::columns_for;
        let n = columns_for(self.content_width(), 200.0, 12.0);
        n.max(self.width_class().min_columns())
    }

    /// 入场动画的**档位**（`Some(n)` = 发 `.enter-<n>`；`None` = 不发分类名）。
    ///
    /// # 外观值一个都不在这里
    ///
    /// 代码只决定"第几档"，第 n 档长什么样（opacity / translate-y）全在 zstyle
    /// 的 `.enter-1` … `.enter-8` 里。这与"hover 走 `.hovered`"是同一条模式的
    /// 延伸，也是不违反 `tests::gates::源码里没有外观` 的前提。
    ///
    /// # 为什么返回 `Option` 而不是 `u32`
    ///
    /// 两个"没有分类名"的情形都要表达：**进度为 0**（这一档与常驻外观没有差异，
    /// 多发一个分类名会让样式表多一条永不命中的规则，而门禁会要求它存在）与
    /// **已经放完**（`anim::step(1.0)` 会返回 `Some(8)`，不单独判就会永远停在
    /// 0.91 不透明度上）。
    pub fn enter_class(&self) -> Option<&'static str> {
        // ⚠ 必须先判"放完"再算进度。
        //
        // `anim::step(1.0)` 返回 `Some(8)`（`ceil(1.0 × 8)` = 8），所以不加这一句，
        // 动画结束后分类名会**永远**停在 `.enter-8` —— 而那一档是不透明度 0.91，
        // 症状是"页面始终蒙着一层灰"，且没有任何报错。
        //
        // 撤掉分类名之后画面回到 1.0，于是最后一跳是 0.91 → 1.0（9%）。
        // 这是 8 档量化的固有代价：要么最后跳 9%，要么 `.enter-8` 写 1.0
        // 而从 `.enter-7`(0.78) 跳 22%。选前者，理由写在
        // `ZephyraUI/tests/anim_wiring_tests.rs` 的模块文档里。
        if self.enter_ms >= crate::anim::ENTER_MS {
            return None;
        }
        let p = crate::anim::progress(
            self.enter_ms,
            crate::anim::ENTER_MS,
            crate::anim::Easing::PointToPoint,
        );
        crate::anim::step(p).map(|n| match n {
            1 => "enter-1",
            2 => "enter-2",
            3 => "enter-3",
            4 => "enter-4",
            5 => "enter-5",
            6 => "enter-6",
            7 => "enter-7",
            _ => "enter-8",
        })
    }

    /// 当前语言的 BCP 47 标签（`zh-Hans` 这类）。
    pub fn locale(&self) -> &'static str {
        SUPPORTED[self.lang.min(SUPPORTED.len() - 1)]
    }

    /// 协商出初始语言：按 [`crate::i18n::negotiate`] 的用户优先序匹配可用表。
    ///
    /// # 为什么在 `on_input` 里做而不是构造时做
    ///
    /// 宿主才知道系统语言偏好（视图拿不到窗口），而 `on_input` 每帧必调 ——
    /// 于是首帧就能定下来，不必另加一条启动钩子。
    ///
    /// 协商不出来时回落**第一项**：空表会让整个界面全空白，而那比
    /// "界面是英文"严重得多。
    fn pick_initial_language(prefs: &Preferences) -> usize {
        let available: Vec<Tag> = SUPPORTED.iter().filter_map(|s| Tag::parse(s)).collect();
        match crate::i18n::negotiate(prefs, &available) {
            Some(tag) => SUPPORTED
                .iter()
                .position(|s| *s == tag.as_str())
                .unwrap_or(0),
            None => 0,
        }
    }

    /// 当前语言对应的偏好表。
    ///
    /// # 为什么不直接复用系统偏好
    ///
    /// 用户手动切过语言之后，系统那个表就**不再是**他要的了。手动选择优先。
    ///
    /// 只含**一项**：当前语言。回退由 [`crate::i18n::Messages::select`] 内部的
    /// 默认语言那一档负责，不需要在偏好表里堆回退项 ——
    /// 把回退项塞进偏好表会让 `negotiate` 也去匹配它们，
    /// 而协商与取文案是两件事，各有一条链更清楚。
    pub fn prefs_for_current(&self) -> Preferences {
        let mut p = Preferences::new();
        if let Some(t) = Tag::parse(self.locale()) {
            p.push(t);
        }
        p
    }

    /// 取一条文案。**所有**面向用户的字符串都走这里 —— 源码里不出文案。
    ///
    /// # 返回 `&'static str` 而不是 `String`
    ///
    /// `view()` 每帧重建整棵树，返回 `String` 就等于每帧一次分配 + 一次
    /// 泄漏（`Box::leak`），而文案在编译期就定好了、根本不是运行期数据。
    /// 于是链路终点是 `PropertyValue::static_str` —— 属性系统为此专门留了一个
    /// 变体（见 `property.rs`），整条路零分配。
    ///
    /// # 找不到时返回键名本身
    ///
    /// 三级回退（见 [`Messages::select`]）：当前语言 → 语言段 → 默认语言。
    /// 连默认语言也没有才轮到这里，返回**键名**而不是空串：空串会让"漏翻"
    /// 看起来像"这里本来就是空的"，而键名在画面上立刻可辨。
    /// 真正发现漏翻的是门禁，不是肉眼（`i18n_tests::no_locale_is_missing_keys`）。
    pub fn tr(&self, key: &'static str) -> &'static str {
        messages()
            .select(&self.prefs_for_current(), SUPPORTED[0], key)
            .unwrap_or(key)
    }

    /// 触控目标边长换算成**布局单位**（= 视觉上的 epx × scale）。
    ///
    /// 只在触控路径上用：鼠标路径的目标尺寸不变（本仓基准未动，见
    /// `docs/developer/responsive-and-touch.md` §5）。
    pub fn touch_target(&self, epx: f32) -> f32 {
        crate::responsive::touch_px(epx, self.scale)
    }
}

impl Default for Gallery {
    fn default() -> Self {
        Self {
            route: "home",
            hovered: None,
            pressed: None,
            last_pressed: None,
            wants_dark: true,
            theme_intent: None,
            seg: "recent",
            checked: [true, false, false],
            switch_on: true,
            volume: 0.62,
            expanded: false,
            combo: 0,
            spin: 42,
            round_on: false,
            dd_open: false,
            mica: true,
            menu_open: None,
            // dump 路默认走基准画布：静态图必须与 1400×800 窗口路同基准，
            // 否则参考图和真实画面始终差一档，而没人会发现差在哪。
            viewport: (BASE_VIEWPORT.0 as f32, BASE_VIEWPORT.1 as f32),
            scale: 1.0,
            lang: 0,
            prefs: Preferences::new(),
            enter_ms: 0.0,
            // 初值 = `route` 的初值（"home"），否则第一帧会被判成"路由变了"
            // 而归零一次 —— 那恰好是想要的，所以两者必须**相同**。
            entered_route: ROUTES[0].0,
        }
    }
}

impl Gallery {
    /// 带主题意图格的构造（窗口路：View 与 Bootstrap 共享这一格）。
    pub fn with_theme_intent(
        theme_intent: std::sync::Arc<std::sync::atomic::AtomicU8>,
        dark: bool,
    ) -> Self {
        Self {
            theme_intent: Some(theme_intent),
            wants_dark: dark,
            ..Self::default()
        }
    }

    /// 某路由当前是否被选中。
    pub fn is_route(&self, id: &str) -> bool {
        self.route == id
    }

    /// 某 key 是否正被悬停。
    pub fn hovers(&self, key: &str) -> bool {
        self.hovered.as_ref().and_then(|k| k.as_name()) == Some(key)
    }

    /// 某 key 是否正被按住。
    /// 当前 hover 的 key 名字（给需要 Option<&str> 的控件用：列表/选择器/对话框）。
    pub fn hovered_name(&self) -> Option<&str> {
        self.hovered.as_ref().and_then(|k| k.as_name())
    }
    /// 当前按住的 key 名字（见 [Self::hovered_name]）。
    /// 当前 hover 的 key 名（无 hover 时提示语）。测试页状态行用。
    pub fn hovered_key(&self) -> &'static str {
        self.hovered
            .as_ref()
            .and_then(|k| k.as_name())
            .unwrap_or("hover anything")
    }
    pub fn pressed_name(&self) -> Option<&str> {
        self.pressed.as_ref().and_then(|k| k.as_name())
    }
    pub fn holds(&self, key: &str) -> bool {
        self.pressed.as_ref().and_then(|k| k.as_name()) == Some(key)
    }

    /// 应用一次 click（按下与抬起落在同一 key 上）。
    fn click(&mut self, key: &str) {
        // 导航。
        //
        // 用**查表**而不是 `match` 列举：路由来源有三处（10 个页面路由
        // + 14 个分组索引路由 + 44 个控件卡），列举必漏且有两处真相。
        if let Some(route) = resolve_route(key) {
            self.route = route;
            return;
        }
        // 语言切换（分段控件的段 id 是语言标签，见 `pages::language_picker`）。
        //
        // ⚠️ 必须在 `resolve_route` **之后**、`match` **之前**：段 id 是
        // `zh-Hans` 这种形状，不带 `lang:` 前缀，所以这里按 `SUPPORTED`
        // 查表而不是切前缀 —— 切前缀要 `strip_prefix` 再 `concat!`，后者
        // 在每帧重建的视图里就是一次泄漏。
        if let Some(i) = SUPPORTED.iter().position(|s| *s == key) {
            self.lang = i;
            return;
        }
        match key {
            "theme-toggle" => {
                self.wants_dark = !self.wants_dark;
                // 窗口路：把意图写进共享格，Bootstrap 每帧对账后执行。
                if let Some(cell) = &self.theme_intent {
                    cell.store(
                        if self.wants_dark { 0 } else { 1 },
                        std::sync::atomic::Ordering::Relaxed,
                    );
                }
            }
            "seg:recent" => self.seg = "recent",
            "seg:favorite" => self.seg = "favorite",
            "check:alpha" => self.checked[0] = !self.checked[0],
            "check:beta" => self.checked[1] = !self.checked[1],
            "check:gamma" => self.checked[2] = !self.checked[2],
            "switch:master" => self.switch_on = !self.switch_on,
            "exp:details" => self.expanded = !self.expanded,
            // ---- Batch A 的演示交互 ----
            "combo:demo" => self.combo = (self.combo + 1) % COMBO_OPTIONS.len(),
            "spin:up" => self.spin = (self.spin + 1).min(100),
            "spin:down" => self.spin = (self.spin - 1).max(0),
            "round:toggle" => self.round_on = !self.round_on,
            "dd:demo" => self.dd_open = !self.dd_open,
            "bar:0" | "bar:1" | "bar:2" => {
                let idx = match key {
                    "bar:0" => 0,
                    "bar:1" => 1,
                    _ => 2,
                };
                // 再点一次当前项 = 收起（MenuBar 的常规行为）。
                self.menu_open = if self.menu_open == Some(idx) {
                    None
                } else {
                    Some(idx)
                };
            }
            // 窗口按钮（win:*）在这个构建里只做悬停/按下视觉：宿主没有把
            // 最小化/最大化/关闭暴露给 View（那是窗口能力，不是样式能力）。
            _ => {}
        }
    }
}

impl View for Gallery {
    fn on_input(&mut self, state: &InputState) {
        // 时钟输入。取一次，全函数共用 —— 同一帧里读两次 `state.dt_ms`
        // 不会得到不同值，但两处各写一遍会让"这个数字是哪来的"变成两处真相。
        let dt = state.dt_ms;
        // click 还原：上一帧按着、这一帧松了，且 key 相同（按住期间不放）。
        let click_key = match (&self.last_pressed, &state.pressed) {
            (Some(prev), None) => Some(prev.as_name().unwrap_or_default().to_string()),
            _ => None,
        };
        if let Some(key) = click_key {
            self.click(&key);
        }
        self.last_pressed = state.pressed.clone();
        self.hovered = state.hovered.clone();
        self.pressed = state.pressed.clone();

        // 入场动画时钟。
        //
        // ⚠ 顺序很要紧：**先**归零再累加，否则换路由的那一帧会从上一条路由的
        // 末尾接着走 —— 症状是"切页之后动画几乎不播"（尤其在两条路由之间反复切换
        // 时，`enter_ms` 已经很大，归零前先加就等于直接跳过入场）。
        let route_changed = self.entered_route != self.route;
        if route_changed {
            self.enter_ms = 0.0;
            self.entered_route = self.route;
        } else {
            self.enter_ms += dt;
        }
        // 夹到时长以上：动画放完之后不必再累加（`f32` 无限增长终会丢精度，
        // 而一个跑了几小时的窗口正是最可能暴露这件事的场景）。
        if self.enter_ms > crate::anim::ENTER_MS {
            self.enter_ms = crate::anim::ENTER_MS;
        }
        // 视口与缩放比：断点判据的唯一来源。宿主每帧开头推一次，于是 resize 之后
        // **下一帧**的树就按新断点造 —— 这正是 WinUI AdaptiveTrigger 的节奏
        // （状态改变 → 声明改变 → diff 落到保留树），不需要任何回调。
        self.viewport = state.viewport;
        self.scale = state.scale;
        // 系统语言偏好：只记**第一次**收到的，之后用户手动切了就以其为准。
        //
        // 判据用 `is_empty()` 而不是 `Option`：空表就是"读不到"，而读到时表
        // 至少有一项。手动切语言不修改 `prefs`、只改 `lang` —— 否则再收到一次
        // 输入就把手动选择冲掉了。
        if self.prefs.is_empty() && !state.languages.is_empty() {
            self.prefs = crate::i18n::Preferences::from_owned(&state.languages);
            self.lang = Self::pick_initial_language(&self.prefs);
        }
    }

    fn view(&self, _ctx: &Context<'_>) -> Element {
        shell::window(self)
    }
}

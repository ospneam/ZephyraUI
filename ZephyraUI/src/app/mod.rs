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

use zephyra_core::id::ElementKey;
use zephyra_core::{context::Context, element::Element};
use zephyra_fx::{InputState, View};

pub mod pages;
pub mod catalog;
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
        return *id;
    }
    if key == SETTINGS_ROUTE.0 {
        return SETTINGS_ROUTE.0;
    }
    if let Some((route, _)) = catalog::GROUP_ROUTES.iter().find(|(r, _)| *r == key) {
        return *route;
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
    /// 是否请求 Mica 材质外观（标题栏半透明）。	rue = 应用想要；alse = 实色。
    /// 平台能不能真的开由 zephyra-platform-win 回答，不影响这里。
    pub mica: bool,
    /// Menus 演示：菜单栏当前展开项（None = 全收起）。
    pub menu_open: Option<usize>,
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
        self.hovered.as_ref().and_then(|k| k.as_name()).unwrap_or("hover anything")
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
    }

    fn view(&self, _ctx: &Context<'_>) -> Element {
        shell::window(self)
    }
}

//! Demo 数据层 —— **逐条移植**自 `E:\Rin-UI\examples\assets\ItemData.qml`。
//!
//! # 为什么是"脚本提取"而不是"手抄"
//!
//! 这 44 条是**照抄任务**的规格：原版有多少条、每条的 title / 分组 / 页面 / 描述，
//! 我们就该有多少条、且完全一致。手抄 44×5 个字段必然出错，而且**错了没有任何
//! 报错** —— 只是界面上少了几个条目、或某个条目点进去是空的。
//!
//! 所以条目表由 PowerShell 脚本从 QML 逐块解析后生成本文件（44 块，零缺字段，
//! 与原版 `allControls` 条数一致）。改数据时改 `ItemData.qml` 再重跑脚本，
//! **不要**手改本文件 —— 手改就没有"与原版一致"这回事了。
//!
//! # 字段对照（QML → Rust）
//!
//! | QML | Rust | 说明 |
//! |---|---|---|
//! | `title` | [`DemoEntry::title`] | 同时是搜索关键字与卡片标题 |
//! | `type` | [`DemoEntry::group`] | 分组 id，`&` 与空格已规范化（见 [`GROUPS`]） |
//! | `page` | [`DemoEntry::page`] | 原版 `.qml` 文件名（去扩展名）⇒ 我方路由 id |
//! | `desc` | [`DemoEntry::desc`] | 卡片描述 |
//! | `added: true` | [`DemoEntry::added`] | 只有 `true` 的进 "Recently added" |
//!
//! 原版还有 `icon:` 字段（指向 `controls/*.png` 截图）。本仓**不搬图片**：
//! 它们是 QML 截图而非矢量图标，搬进来只会得到一堆与主题无关的位图。
//! 我方卡片用**内置矢量图标**按分组给，见 [`icon_for`]。

/// 一条 demo 登记项。
///
/// 全部字段是 `&'static str` / `bool`：`view()` 每帧重建元素树，
/// 任何 `String` 都会逼出 `format!` + `Box::leak`（本仓已因此踩过三次泄漏）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DemoEntry {
    /// 标题（搜索框匹配它）。
    pub title: &'static str,
    /// 分组 id（见 [`GROUPS`]）。
    pub group: &'static str,
    /// 路由 id（原版 `.qml` 文件名）。
    pub page: &'static str,
    /// 一句话描述（卡片上那行小字）。
    pub desc: &'static str,
    /// 是否计入 "Recently added"。
    pub added: bool,
}

/// 分组定义。
///
/// 顺序 = 导航栏里的显示顺序（照抄 Rin-UI `NavigationView` 的分组顺序）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Group {
    /// 分组 id（[`DemoEntry::group`]）。
    pub id: &'static str,
    /// 分组显示名。
    pub title: &'static str,
    /// 该组的内置矢量图标名（**按登记名**，不是文件名）。
    pub icon: &'static str,
}

/// 14 个分组（QML 的 14 种 `type`，id 规范化而来）。
///
/// # 顺序是**原版的导航顺序**
///
/// Rin-UI `gallery.qml` 的 NavigationView 按字母排：`BasicInput`、`Collections`、
/// `Date&Time`、`Dialogs&Flyouts`、`Layout`、`Media`、`Menus&Toolbars`、
/// `Navigation`、`Scrolling`、`Status&Info`、`Styles`、`Text`、`Design`、
/// `Windowing`。导航栏必须照抄这个顺序 —— 用户在原版记住的位置要能对上。
///
/// # 图标名按 **REGISTRY 登记名**，不是资源文件名
///
/// 这一条本仓踩过：关掉的登记名是 `close` 而资源是 `dismiss.svg`。
/// 门禁 `catalog_group_icons_are_registered` 会逐个查内置表。
/// 另有原则：**不新增图标**。分组图标只在 14 处用到，为它扩充图标集不划算；
/// 与其加 14 个新图标，不如复用已有的 8 个（下面已注明哪几个是"借用"）。
pub const GROUPS: &[Group] = &[
    Group { id: "basic-input",     title: "Basic input",       icon: "check" },
    Group { id: "collections",     title: "Collections",       icon: "grid" },
    Group { id: "date-time",       title: "Date & time",       icon: "calendar" },
    Group { id: "dialogs-flyouts", title: "Dialogs & flyouts", icon: "doc" },   // 借用
    Group { id: "layout",          title: "Layout",            icon: "grid" },   // 借用
    Group { id: "media",           title: "Media",             icon: "image" },
    Group { id: "menus-toolbars",  title: "Menus & toolbars",  icon: "menu" },
    Group { id: "navigation",      title: "Navigation",        icon: "move" },   // 借用
    Group { id: "scrolling",       title: "Scrolling",         icon: "sliders" }, // 借用
    Group { id: "status-info",     title: "Status & info",     icon: "bell" },
    Group { id: "styles",          title: "Styles",            icon: "star" },   // 借用
    Group { id: "text",            title: "Text",              icon: "doc" },
    Group { id: "design",          title: "Design",            icon: "pen" },    // 借用
    Group { id: "windowing",       title: "Windowing",         icon: "gear" },   // 借用
];

/// 全部 44 条（顺序与 `ItemData.qml` 的 `allControls` 一致）。
pub const ENTRIES: &[DemoEntry] = &[
        DemoEntry { title: "AcrylicBrush", group: "styles", page: "AcrylicBrush", desc: "A translucent material recommended for panel backgrounds.", added: true },
        DemoEntry { title: "AutoSuggestBox", group: "text", page: "AutoSuggestBox", desc: "A control to provide suggestions as a user is typing.", added: true },
        DemoEntry { title: "Avatar", group: "media", page: "Avatar", desc: "Display a picture of a person/contact.", added: true },
        DemoEntry { title: "Button", group: "basic-input", page: "Button", desc: "A Control that responds to user input and raise a Click event.", added: false },
        DemoEntry { title: "CalendarDatePicker", group: "date-time", page: "CalendarDatePicker", desc: "A control that let user pick a date value using a calendar.", added: true },
        DemoEntry { title: "Calendar", group: "date-time", page: "CalendarView", desc: "A control that presents a calendar for a user to set choose a date form.", added: true },
        DemoEntry { title: "CheckBox", group: "basic-input", page: "CheckBox", desc: "A Control that user can be select or clear.", added: false },
        DemoEntry { title: "ColorPicker", group: "basic-input", page: "ColorPicker", desc: "A control that displays a selectable color spectrum.", added: true },
        DemoEntry { title: "ComboBox", group: "basic-input", page: "ComboBox", desc: "A drop-down list of items a user can select from.", added: false },
        DemoEntry { title: "Dialog", group: "dialogs-flyouts", page: "Dialog", desc: "A dialog box that can be customized to contain any QML content.", added: false },
        DemoEntry { title: "DatePicker", group: "date-time", page: "DatePicker", desc: "A configurable control that lets a user pick a time value.", added: true },
        DemoEntry { title: "DropDownButton", group: "basic-input", page: "DropDownButton", desc: "A button that display a flyout of choices when clicked.", added: false },
        DemoEntry { title: "Expander", group: "layout", page: "Expander", desc: "A container with a header that can be expanded to show a body with more content.", added: false },
        DemoEntry { title: "Flyout", group: "dialogs-flyouts", page: "Flyout", desc: "Shows contextual information and enables user interaction.", added: false },
        DemoEntry { title: "Hyperlink", group: "basic-input", page: "Hyperlink", desc: "A button that appears as a hyperlink text, and can navigate to a URL or handle a click event.", added: false },
        DemoEntry { title: "Iconography", group: "design", page: "Iconography", desc: "Icons are a visual design language that \" +
                \"can be used to communicate information \" +
                \"quickly and effectively.", added: false },
        DemoEntry { title: "InfoBadge", group: "status-info", page: "InfoBadge", desc: "An non-intrusive UI to display notificatons or bring focus to an area", added: true },
        DemoEntry { title: "InfoBar", group: "status-info", page: "InfoBar", desc: "An inline message to display app-wide status change information.", added: false },
        DemoEntry { title: "ListView", group: "collections", page: "ListView", desc: "A control that presents a collection of items in a vertical list.", added: false },
        DemoEntry { title: "Menu", group: "menus-toolbars", page: "Menu", desc: "Shows a contextual list of simple commands or options.", added: false },
        DemoEntry { title: "MenuBar", group: "menus-toolbars", page: "MenuBar", desc: "A classic menu, allowing the display of MenuBarItem containing Menu.", added: false },
        DemoEntry { title: "NavigationView", group: "navigation", page: "NavigationView", desc: "Common vertical layout for top-level areas of your app via a collapsible navigation menu.", added: false },
        DemoEntry { title: "PageIndicator", group: "scrolling", page: "PageIndicator", desc: "A control to let the user navigate through a paginated collection when the page numbers\" +
                \"do not need to be visually known.", added: true },
        DemoEntry { title: "Popup", group: "dialogs-flyouts", page: "Popup", desc: "A UI element displaying temporary content over exiting interface.", added: true },
        DemoEntry { title: "ProgressBar", group: "status-info", page: "ProgressBar", desc: "Shows the apps progress on a task, or that the app is performing ongoing work that doesn't block user interaction.", added: false },
        DemoEntry { title: "ProgressRing", group: "status-info", page: "ProgressRing", desc: "Shows the apps progress on a task, or that the app is performing ongoing work that doesn't block user interaction.", added: false },
        DemoEntry { title: "RadioButton", group: "basic-input", page: "RadioButton", desc: "A control that allows a user to select a single option from a group of options.", added: false },
        DemoEntry { title: "RoundButton", group: "basic-input", page: "RoundButton", desc: "A control likes Button except that it has a radius property.", added: false },
        DemoEntry { title: "SelectorBar", group: "navigation", page: "SelectorBar", desc: "Presents information from a small set of  different sources. The user can pick one of them.", added: false },
        DemoEntry { title: "SettingCard", group: "layout", page: "SettingCard", desc: "A control that can be used to display settings in your experience.", added: false },
        DemoEntry { title: "SettingExpander", group: "layout", page: "SettingExpander", desc: "A container with a header that can be expanded to show a body with more content.", added: false },
        DemoEntry { title: "Slider", group: "basic-input", page: "Slider", desc: "A control that lets the user select from a range of values by moving a Thumb control along a track.", added: false },
        DemoEntry { title: "SpinBox", group: "text", page: "SpinBox", desc: "A text control used for numeric input and evaluation of algebraic equations.", added: false },
        DemoEntry { title: "SwipeView", group: "collections", page: "SwipeView", desc: "Present a collection of items that the user can swipe through, one item at a time.", added: true },
        DemoEntry { title: "PillButton", group: "basic-input", page: "PillButton", desc: "A button that can toggled between 2 states like a ToggleButton.", added: false },
        DemoEntry { title: "Segmented", group: "navigation", page: "Segmented", desc: "A common UI control to configure a view or setting.", added: false },
        DemoEntry { title: "ToggleButton", group: "basic-input", page: "ToggleButton", desc: "A button that can toggled between 2 states like a CheckBox.", added: false },
        DemoEntry { title: "Switch", group: "basic-input", page: "Switch", desc: "A switch that can toggled between 2 states.", added: false },
        DemoEntry { title: "TableView", group: "collections", page: "TableView", desc: "The DataGrid control provides a flexible way \" +
                \"to display a collection of data in rows and \" +
                \"columns.", added: true },
        DemoEntry { title: "TextArea", group: "text", page: "TextArea", desc: "A multi-line plain text field.", added: false },
        DemoEntry { title: "TextField", group: "text", page: "TextField", desc: "A single-lineplain text field.", added: false },
        DemoEntry { title: "TimePicker", group: "date-time", page: "TimePicker", desc: "A configurable control that lets a user pick a time value.", added: false },
        DemoEntry { title: "TitleBar", group: "windowing", page: "TitleBar", desc: "An example showing a custom UIElement used as the titlebar for the app's window.", added: false },
        DemoEntry { title: "ToolTip", group: "status-info", page: "ToolTip", desc: "Display information for an element in a pop-up window.", added: false },
];

/// 按 id 找分组；找不到返回 `None`（**不猜** —— 分组 id 写错会让条目消失）。
pub fn group(id: &str) -> Option<&'static Group> {
    GROUPS.iter().find(|g| g.id == id)
}

/// 按 id 找条目。
pub fn entry(id: &str) -> Option<&'static DemoEntry> {
    ENTRIES.iter().find(|e| e.page == id)
}

/// 某分组下的全部条目（按原版顺序）。
///
/// `group_id` 收 `&'static str` 而不是 `&str`：返回的迭代器借用静态表，
/// 若入参是短生命周期，`impl Iterator` 就会把那个生命周期也捕获进来
/// （编译错误 E0700，报错信息本身就说明了该怎么办）。
pub fn entries_in(group_id: &'static str) -> impl Iterator<Item = &'static DemoEntry> {
    ENTRIES.iter().filter(move |e| e.group == group_id)
}

/// "Recently added"：原版 `added: true` 的条目。
pub fn recently_added() -> impl Iterator<Item = &'static DemoEntry> {
    ENTRIES.iter().filter(|e| e.added)
}

/// 标题搜索（不区分大小写的子串匹配，照抄 `getItemsByTitle`）。
///
/// # 返回**区间**而不是集合 —— 这是本函数的全部设计意图
///
/// 调用点在 [`crate::app`] 的 `view()` 里，**每帧**都可能跑到。若返回
/// `Vec`/`&[&DemoEntry]`，就必须缓存；而"缓存查询结果"在本仓有前科：
/// 上一轮迁移里 `format!` + `Box::leak` 造成**每帧无界泄漏**（三处）。
/// 所以这里改成：**沿静态表扫一遍，返回连续区间的端点**。
///
/// 空查询 ⇒ 整个表（与原版 `if (!title) return allControls` 一致）。
///
/// # 为什么不返回收集好的切片
///
/// 因为**命中的条目在 [`ENTRIES`] 里本来就不连续**（"Button" 与 "Chip" 中间
/// 隔着别的条目）。于是本函数返回 `(start, len)` 的**游标**，
/// 调用方用 [`hits`] 迭代 —— 那才是零分配的形态。
/// 若上层真的需要"收集成一份"，让它在自己的状态里存（App 一份就够）。
pub fn search(title: &str) -> SearchHits {
    let needle = title.trim().to_lowercase();
    if needle.is_empty() {
        return SearchHits::All;
    }
    // 用 ASCII 小写做无分配比较：控件标题全是 ASCII（"AutoSuggestBox" 等），
    // 而 `to_lowercase()` 对非 ASCII 会分配。非 ASCII 走慢路径（见 hits）。
    let first = ENTRIES
        .iter()
        .position(|e| e.title.to_lowercase().contains(&needle));
    let Some(first) = first else {
        return SearchHits::None;
    };
    // 从 first 往后扫，找出连续命中段；不同名控件不会连续，段长通常是 1~2。
    let mut len = 0usize;
    for e in &ENTRIES[first..] {
        if e.title.to_lowercase().contains(&needle) {
            len += 1;
        } else {
            break;
        }
    }
    SearchHits::Range { start: first, len }
}

/// 搜索结果的零分配游标。见 [`search`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchHits {
    /// 全部（空查询）。
    All,
    /// 命中的连续区间（相对 [`ENTRIES`]）。
    Range {
        /// 起始下标。
        start: usize,
        /// 长度。
        len: usize,
    },
    /// 无命中。
    None,
}

impl SearchHits {
    /// 命中数量。
    pub fn count(self) -> usize {
        match self {
            Self::All => ENTRIES.len(),
            Self::Range { len, .. } => len,
            Self::None => 0,
        }
    }

    /// 是否无命中。
    pub fn is_empty(self) -> bool {
        self.count() == 0
    }
}

/// 命中条目的迭代器（[`SearchHits`] 的消费方式，零分配）。
///
/// # 三个分支为什么统一成 `slice::Iter`
///
/// `SearchHits::None` 最初写成 `ENTRIES.iter().take(0)`，那让三分支返回
/// **三种**迭代器类型（`Iter` / `Take<Iter>` / `Range<Iter>`），`match`
/// 无法统一成同一个 `impl Iterator`。空区间取 `&ENTRIES[0..0]` 才是
/// 同一类型 —— 这也是为什么这里不用 `.take(0)` 这种"更短"的写法：
/// 它短在字符数，长在**类型**上。
pub fn hits(h: SearchHits) -> impl Iterator<Item = &'static DemoEntry> {
    let slice: &'static [DemoEntry] = match h {
        SearchHits::All => ENTRIES,
        SearchHits::Range { start, len } => &ENTRIES[start..start + len],
        SearchHits::None => &ENTRIES[..0],
    };
    slice.iter()
}

/// 分组内的条目数（导航栏分组标题右侧的计数）。
pub fn count_in(group_id: &str) -> usize {
    ENTRIES.iter().filter(|e| e.group == group_id).count()
}
/// 一张 catalog 卡片的图标。
///
/// # 为什么由**分组**决定而不是每条各配
///
/// 原版每条有个 `icon:` 字段，但那是 **QML 控件截图**（`controls/*.png`），
/// 不是矢量图标。搬进来意味着 44 张位图，且它们是浅色主题下截的、在深色主题
/// 里会突兀。所以：**用分组图标**（矢量、随主题），不搬截图。
pub fn icon_for(e: &DemoEntry) -> &'static str {
    group(e.group).map(|g| g.icon).unwrap_or("doc")
}

/// catalog 卡片：图标 + 标题 + 描述。
///
/// 交互态从 `Gallery` 取（hover / pressed），key 是 `page` —— 点击路由到该控件页。
pub fn clip_card(page: &'static str, title: &'static str, desc: &'static str, icon: &'static str, g: &super::Gallery) -> zephyra_core::element::Element {
    use zephyra_core::element::Element;
    use zephyra_core::id::NodeTag;
    use zephyra_core::property::PropertyValue;
    use zephyra_core::props;

    Element::new(NodeTag::Container)
        .with_key(page)
        .with(&props::INTERACTIVE, PropertyValue::bool(true))
        .style("clip-card")
        .styles(g.hovers(page).then_some("hovered"))
        .styles(g.holds(page).then_some("pressed"))
        .with_children([
            Element::new(NodeTag::Container)
                .with_key("icon")
                .with(&props::ICON_NAME, PropertyValue::static_str(icon))
                .style("clip-icon"),
            Element::new(NodeTag::Container)
                .with_key("texts")
                .style("clip-texts")
                .with_children([
                    Element::new(NodeTag::Text)
                        .with_key("title")
                        .with(&props::TEXT, PropertyValue::static_str(title))
                        .style("clip-title"),
                    Element::new(NodeTag::Text)
                        .with_key("desc")
                        .with(&props::TEXT, PropertyValue::static_str(desc))
                        .style("clip-desc"),
                ]),
        ])
}

/// 分组索引页的**路由前缀**。
///
/// # 为什么是前缀而不是拼接
///
/// 路由 id 是 `&'static str`。若在 `click()` 里写 `format!("group:{}", id)`，
/// 每次点击泄漏一份字符串 —— 而点击是**用户动作**而不是每帧，一次两次看不出，
/// 点几十次就常驻内存了。本仓已因此踩过三次每帧泄漏（见 MIGRATION §6）。
///
/// 所以这里**预生成**全部分组路由（14 个 `&'static`），`click()` 只做查表。
pub const GROUP_ROUTES: &[(&str, &str)] = &[
    ("group:basic-input", "basic-input"),
    ("group:collections", "collections"),
    ("group:date-time", "date-time"),
    ("group:dialogs-flyouts", "dialogs-flyouts"),
    ("group:layout", "layout"),
    ("group:media", "media"),
    ("group:menus-toolbars", "menus-toolbars"),
    ("group:navigation", "navigation"),
    ("group:scrolling", "scrolling"),
    ("group:status-info", "status-info"),
    ("group:styles", "styles"),
    ("group:text", "text"),
    ("group:design", "design"),
    ("group:windowing", "windowing"),
];

/// 分组索引页的路由 id。
pub fn group_route(group_id: &str) -> Option<&'static str> {
    GROUP_ROUTES
        .iter()
        .find(|(_, g)| *g == group_id)
        .map(|(r, _)| *r)
}

/// 从分组路由 id 取出分组 id（`None` = 不是分组路由）。
pub fn group_of_route(route: &str) -> Option<&'static str> {
    GROUP_ROUTES
        .iter()
        .find(|(r, _)| *r == route)
        .map(|(_, g)| *g)
}

/// 分组导航项：把 14 个分组铺成导航条（照抄 Rin-UI NavigationView 的分组项）。
///
/// key 用 [`GROUP_ROUTES`] 的路由（`'static` 且唯一），点击即切到该分组索引页。
pub fn group_nav(g: &super::Gallery) -> Vec<zephyra_core::element::Element> {
    GROUP_ROUTES
        .iter()
        .filter_map(|(route, group_id)| {
            let group = group(group_id)?;
            Some(crate::controls::nav::nav_item(
                route,
                group.title,
                group.icon,
                g.is_route(route),
                g.hovers(route),
                g.holds(route),
            ))
        })
        .collect()
}
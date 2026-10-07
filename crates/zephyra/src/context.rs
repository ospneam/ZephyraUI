//! 只读上下文：布局与绘制共用的外部输入。
//!
//! # 为什么传上下文而不是把主题存在节点上
//!
//! 主题是**应用状态**，不是节点状态。存在节点上意味着切主题要遍历全树改字段（正是
//! 本框架要避免的）；存进上下文后，切主题只是换一个 `&Theme`——树完全不动。
//!
//! 也因此属性里的资源键**保持为键**，直到读取那一刻才解析（见
//! [`crate::theme::resolve_resource`]）。

/// 布局与绘制所需的只读上下文。
#[derive(Clone, Copy)]
pub struct Context<'a> {
    /// 当前主题。
    pub theme: &'a crate::theme::Theme,
    /// 文本引擎。`None` 表示「本进程没有文本能力」——此时文本节点不产出任何
    /// 命令，且尺寸按零算。
    ///
    /// 类型是 [`crate::text::TextRender`] 而非 `TextEngine`：降级成
    /// `Command::Text` 需要图集，而只有能绘制的实现才有。这也让"只做布局预计算"
    /// 的宿主无法被误接进来 —— 它会在 `render` 阶段才暴露问题。
    ///
    /// 用 `&RefCell` 而非 `&mut`：布局是递归的（`measure` 逐层向下），把
    /// `&mut dyn TextRender` 一路穿下去会在第一次递归就撞上借用检查器。
    /// `&RefCell` 让 `Context` 保持 `Copy`，递归照常，引擎仍是货真价实的 `&mut`。
    pub engine: Option<&'a std::cell::RefCell<dyn crate::text::TextRender>>,
    /// 图标集（名字 → 图标）。
    ///
    /// # 为什么走 `Context` 而不是全局静态
    ///
    /// 图标是**宿主资源**，正如纹理句柄是宿主资源。放进全局 `static` 会让两个
    /// 进程内并存的图标集无法共存（多窗口、不同图标库、测试替身），而且测试之间
    /// 会互相污染 —— 那类"单测之间互相影响"的失败最难查。
    ///
    /// `None` 表示「本进程没有图标能力」—— 与 `engine` 同一套降级语义：
    /// 图标节点不产出命令，其余部分完全不受影响。
    ///
    /// 类型是 `&'static IconSet`（[`IconSet::get`] 返回 `&'static Icon`，
    /// 因为解析结果被 `OnceLock` 固化、全程共享），所以借用它零拷贝。
    pub icons: Option<&'static crate::icon::IconSet>,
}

impl std::fmt::Debug for Context<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Context")
            .field("theme", &self.theme.variant)
            .field("engine", &self.engine.map(|_| "TextEngine"))
            .field("icons", &self.icons.map(|_| "IconSet"))
            .finish()
    }
}

impl<'a> Context<'a> {
    /// 用给定主题构造（无文本、无图标能力）。
    pub fn new(theme: &'a crate::theme::Theme) -> Self {
        Self {
            theme,
            engine: None,
            icons: None,
        }
    }

    /// 同时提供主题与文本引擎（无图标能力）。
    pub fn with_engine(
        theme: &'a crate::theme::Theme,
        engine: &'a std::cell::RefCell<dyn crate::text::TextRender>,
    ) -> Self {
        Self {
            theme,
            engine: Some(engine),
            icons: None,
        }
    }

    /// 同时提供主题、文本引擎与图标集 —— **宿主该用的那个**。
    pub fn with_all(
        theme: &'a crate::theme::Theme,
        engine: &'a std::cell::RefCell<dyn crate::text::TextRender>,
        icons: &'static crate::icon::IconSet,
    ) -> Self {
        Self {
            theme,
            engine: Some(engine),
            icons: Some(icons),
        }
    }

    /// 图标集（若本进程有图标能力）。
    pub fn icon_set(&self) -> Option<&'static crate::icon::IconSet> {
        self.icons
    }

    /// 挂上 / 摘掉图标集。
    ///
    /// 分开提供是因为大部分调用点（布局、命中测试）不需要图标，而让它们为了
    /// 用 [`Self::with_engine`] 就必须准备一个图标集，是没道理的要求。
    pub fn with_icons(mut self, icons: &'static crate::icon::IconSet) -> Self {
        self.icons = Some(icons);
        self
    }
}

/// 进程级共享的内置主题。
///
/// `Context::dark()` 之类的便捷函数需要返回 `&'static Theme`，而主题是纯数据
/// （一堆 `Color`），构造一次就该全程复用。这里用 `OnceLock` 惰性建好并**只借不借出**
/// 所有权，于是既不会泄漏内存，也不会有人拿到 `&mut` 把它改坏。
static DARK_THEME: std::sync::OnceLock<crate::theme::Theme> = std::sync::OnceLock::new();
static LIGHT_THEME: std::sync::OnceLock<crate::theme::Theme> = std::sync::OnceLock::new();

/// 深色主题的上下文。
///
/// 共享同一个 `Theme` 实例：色板不可变，多份副本只会让人误以为改一份能影响另一份。
pub fn dark_context() -> Context<'static> {
    Context::new(DARK_THEME.get_or_init(crate::theme::Theme::dark))
}

/// 浅色主题的上下文。
pub fn light_context() -> Context<'static> {
    Context::new(LIGHT_THEME.get_or_init(crate::theme::Theme::light))
}

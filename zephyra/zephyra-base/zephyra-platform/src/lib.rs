#![deny(rustdoc::broken_intra_doc_links)]
//! 平台能力抽象层 —— **本 crate 一行平台代码都没有**。
//!
//! # 为什么要有这一层
//!
//! 「Windows 专属优化」与「通用代码混在一起」是本仓明令禁止的（见
//! [`REWRITE.md`](../../../../ZephyraUI/REWRITE.md) 的 D1）。但完全不抽象也不行：
//! 应用要能问一句「这台机器支持 Mica 吗」，而答案在 Linux 上是「不支持」。
//!
//! 于是这里只做一件事：**定义"能力"是什么**，不含任何 `cfg(windows)`。
//! 实现分两个 crate：
//!
//! - `zephyra_platform_win` —— Windows 实现（DWM / 版本探测 / 系统主题）；
//! - [`stub`] —— 其它平台的空实现（**Linux 适配只需改这一个文件**）。
//!
//! # 上层怎么用（这决定了本 crate 的形状）
//!
//! ```ignore
//! let caps = platform.capabilities();
//! if caps.backdrop { /* 画 Mica */ } else { /* 画实色 —— 降级不是错误 */ }
//! ```
//!
//! ⚠️ **能力位缺失必须是"降级"，不能是"报错"**。理由：桌面 Linux / Wayland / X11
//! 上没有 Mica 是常态，不是故障。把"平台不支持"当成错误上报，等于让 Linux 上
//! 每个应用一启动就红。
//!
//! # 为什么窗口句柄用 `usize` 而不是 `HWND`
//!
//! 本 crate 不能 `use windows::Win32::...`（那是 Windows 代码），而 `HWND` 只在
//! Windows 上有意义。折中：这里只承诺"这是一个**不透明**的窗口句柄，Windows 上恰好
//! 是 `HWND`"，由实现 crate 负责转换。上层因此**只传句柄、不解释句柄**。

#![forbid(unsafe_code)]
#![deny(missing_debug_implementations)]

use std::fmt;

pub mod stub;

/// 不透明窗口句柄。
///
/// Windows 上等于 `HWND`。构造见 [`WindowHandle::from_raw`]。
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct WindowHandle(usize);

impl WindowHandle {
    /// 由平台原生指针构造。
    ///
    /// # 为什么收 `usize` 而不是泛型
    ///
    /// 见模块文档：这个 crate 不能出现 `HWND`。`usize` 是唯一在**所有**平台都
    /// 同宽、且能无损承载指针的类型。
    pub const fn from_raw(raw: usize) -> Self {
        Self(raw)
    }

    /// 平台原生指针。
    pub const fn raw(self) -> usize {
        self.0
    }

    /// 是否是"有句柄"（创建失败时上层会拿到空句柄）。
    pub const fn is_valid(self) -> bool {
        self.0 != 0
    }
}

impl fmt::Debug for WindowHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // 句柄是裸地址：调试输出里直接打地址没有信息量，还会在日志里指向
        // 可能已释放的窗口。只报"有/无"。
        f.debug_tuple("WindowHandle")
            .field(&if self.is_valid() { &"valid" } else { &"null" })
            .finish()
    }
}

/// 系统深浅色偏好。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemePreference {
    /// 浅色。
    Light,
    /// 深色。
    Dark,
    /// 知道但未定（系统设置为"自动"且当前无判断依据）。
    Unknown,
}

impl ThemePreference {
    /// 人类可读形式（trace 用）。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
            Self::Unknown => "unknown",
        }
    }
}

/// 窗口圆角偏好。
///
/// 取值与 Windows 的 `DWM_WINDOW_CORNER_PREFERENCE` **一一对应**
/// （0..3），见 `docs/win/01-window-attributes.md`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CornerPreference {
    /// 由系统决定（0）。**默认值**：应用不表态，行为随系统与版本变化。
    #[default]
    Default,
    /// 永不圆角（1）。
    DoNotRound,
    /// 大圆角（2）。
    Round,
    /// 小圆角（3）。
    RoundSmall,
}

impl CornerPreference {
    /// 平台原始取值。
    pub const fn raw(self) -> i32 {
        match self {
            Self::Default => 0,
            Self::DoNotRound => 1,
            Self::Round => 2,
            Self::RoundSmall => 3,
        }
    }

    /// 由平台原始取值构造；未知取值按 [`Self::Default`] 处理。
    ///
    /// 未知即默认，是这里刻意的：新版 Windows 加了第五档时，我们不会把窗口
    /// 弄坏，只是没拿到新效果。
    pub const fn from_raw(raw: i32) -> Self {
        match raw {
            1 => Self::DoNotRound,
            2 => Self::Round,
            3 => Self::RoundSmall,
            _ => Self::Default,
        }
    }
}

/// 背景材质。
///
/// ⚠️ 这些是**材质类别**，不是像素效果。Windows 文档对 `DWMSBT_MAINWINDOW` 的
/// 措辞是「For Windows 11, this corresponds to Mica in its default variant.
/// **The material effect might change with future Windows releases.**」
/// —— 所以"选 Mica"表达的是"要那一类材质"，不是"要那个颜色"。
///
/// 材质是**系统画在窗口背后**的，因此只在"应用没把那块涂满不透明色"时可见。
/// 平台层只回答"能不能开"，"要不要留透明"是应用/样式层的责任。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BackdropKind {
    /// 不画任何背景材质（对应 `DWMSBT_NONE`）。
    ///
    /// ⚠️ 必须是**显式的"不画"**，不能映射到 `DWMSBT_AUTO`：文档说 AUTO 由
    /// DWM 决定、可能仍然画材质 —— 那样"关掉 Mica"就关不掉。
    #[default]
    None,
    /// Mica 默认变体（`DWMSBT_MAINWINDOW`）。Win11 22621+。
    Mica,
    /// Mica Alt（`DWMSBT_TABBEDWINDOW`），适合带标签页标题栏的窗口。
    MicaAlt,
    /// Acrylic（`DWMSBT_TRANSIENTWINDOW`）。Win11 22621+。
    Acrylic,
}

/// 应用向平台**请求**的窗口外观。
///
/// 只收**意图**，不收平台类型；平台不认的字段会被忽略并在
/// [`ApplyOutcome`] 里如实报告 —— 这就是"降级可见"的地方。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WindowChrome {
    /// 是否要求暗色模式（标题栏/边框配色）。
    pub dark: bool,
    /// 圆角偏好。
    pub corner: CornerPreference,
    /// 标题栏颜色（0xRRGGBB；`None` = 交给系统）。
    pub caption_color: Option<u32>,
    /// 窗口边框颜色（0xRRGGBB；`None` = 交给系统）。
    pub border_color: Option<u32>,
    /// 背景材质。
    pub backdrop: BackdropKind,
}

impl WindowChrome {
    /// 只要求暗色 + 圆角，其余交给系统。
    pub const fn minimal(dark: bool, corner: CornerPreference) -> Self {
        Self {
            dark,
            corner,
            caption_color: None,
            border_color: None,
            backdrop: BackdropKind::None,
        }
    }
}

/// 一次外观请求的**逐项**结果。
///
/// 为什么逐项而不是一个总结果：混着报"成功"时，某一项被系统静默拒绝
/// （老版本不支持）就永远查不出来 —— 而这正是窗口外观问题的常见形态
/// （界面全对，就是标题栏没变暗）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApplyOutcome {
    /// 暗色模式是否生效。
    pub dark: bool,
    /// 圆角是否生效。
    pub corner: bool,
    /// 标题栏颜色是否生效。
    pub caption_color: bool,
    /// 边框颜色是否生效。
    pub border_color: bool,
    /// 背景材质是否生效。
    pub backdrop: bool,
}

impl ApplyOutcome {
    /// 全部生效（用于测试与"降级了吗"的快速判断）。
    pub const fn all_applied(self) -> bool {
        self.dark && self.corner && self.caption_color && self.border_color && self.backdrop
    }
}

impl fmt::Display for ApplyOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // 一行写清"哪些没生效"。trace 直接打这行。
        write!(
            f,
            "dark={} corner={} caption={} border={} backdrop={}",
            self.dark, self.corner, self.caption_color, self.border_color, self.backdrop
        )
    }
}

/// 平台能力位。
///
/// 用**结构体字段**而不是位标志（`u32` bitfield）：新增能力时编译器会强制
/// 所有构造点补上默认值，位标志则容易静默错位。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Capabilities {
    /// 能否按请求设置暗色模式。
    pub dark_mode: bool,
    /// 能否按请求设置圆角。
    pub corner_preference: bool,
    /// 能否按请求设置标题栏颜色。
    pub caption_color: bool,
    /// 能否按请求设置边框颜色。
    pub border_color: bool,
    /// 能否设置系统背景材质（Mica / Mica Alt / Acrylic）。
    ///
    /// ⚠️ `true` 只表示"**调用会被接受**"，不表示"画面上看得见" ——
    /// 材质画在窗口**背后**，应用必须给根留透明。见 [`BackdropKind`]。
    pub backdrop: bool,
    /// 能否**查询**系统深浅色（不能查询 ≠ 不能设置）。
    pub system_theme_query: bool,
    /// 能否**查询**系统语言偏好（不能查询 => 回落默认语言，不钉死界面）。
    pub system_language_query: bool,
}

impl Capabilities {
    /// 「什么都不支持」。
    pub const fn none() -> Self {
        Self {
            dark_mode: false,
            corner_preference: false,
            caption_color: false,
            border_color: false,
            backdrop: false,
            system_theme_query: false,
            system_language_query: false,
        }
    }
}

/// 平台接口。
///
/// # 为什么是 trait 而不是 `cfg` 分支
///
/// `cfg` 分支意味着"Linux 编译"只能靠"删掉 Windows 代码"来达成 —— 那不是编译，
/// 是绕过。trait + stub 意味着 Linux 路径**真的会被编译、真的会被测试**，
/// 只是实现返回"不支持"。
pub trait Platform {
    /// 平台名（trace / 诊断用）。
    fn name(&self) -> &'static str;

    /// OS 版本号 `(major, minor, build)`；拿不到时 `None`。
    fn version(&self) -> Option<(u32, u32, u32)>;

    /// 当前平台**支持哪些能力**。
    fn capabilities(&self) -> Capabilities;

    /// 查系统深浅色偏好。
    ///
    /// 返回 `None` = "查不到"（不是"浅色"）。上层据此**不跟随**系统主题，
    /// 而不是跟随成浅色 —— 后者会让不支持的平台上主题莫名被钉死。
    fn system_theme(&self) -> Option<ThemePreference>;

    /// 查系统语言偏好（按优先级降序）。
    ///
    /// 对应浏览器的 `navigator.languages`：BCP 47 标签的有序表，最偏好的在前。
    /// 返回空表 = "读不到" ⇒ 上层回落默认语言，**不**把界面钉死。
    ///
    /// # Windows 上的权威来源（写本方法前必查，勿凭印象）
    ///
    /// `GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, …)`（`winnls.h`）——
    /// 返回线程的首选 UI 语言名列表，正是「有序、语言名标签」这一形状。
    /// 它与 `GetSystemDefaultUILanguage` 的区别：后者只有一项且是系统安装语言，
    /// 不是**用户偏好**。文档：`docs/i18n.md` §3。
    fn system_languages(&self) -> Vec<String>;

    /// 对窗口施加外观。
    ///
    /// 实现**必须**：逐项如实返回成功与否，且不支持的项返回 `false`
    /// 而**不**返回错误（不支持不是故障）。
    fn apply_window_chrome(&self, window: WindowHandle, chrome: WindowChrome) -> ApplyOutcome;
}

/// 取出当前编译目标的 [`Platform`] 实现。
///
/// 走**函数**而不是单例常量：实现里可能有状态（缓存的版本探测结果、
/// 注册过的系统主题监听），而且单例会引入 `static mut`/`OnceLock` 的
/// 初始化顺序问题 —— 那类问题在窗口创建早期最难查。
pub fn current() -> &'static dyn Platform {
    crate::stub::platform()
}

#[cfg(test)]
#[path = "tests/lib_tests.rs"]
mod tests;

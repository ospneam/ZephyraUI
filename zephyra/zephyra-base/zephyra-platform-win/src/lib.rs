#![deny(rustdoc::broken_intra_doc_links)]
//! Windows 实现 [`zephyra_platform::Platform`]。
//!
//! # 这里只写**文档确证**的 API
//!
//! 每一条对应的文档出处写在 docs/developer/win/01-window-attributes.md，本文件的
//! 注释里也逐条标注。**未文档化的属性一律不用** —— 参照物 Rin-UI 用了
//! `DWMWA_MICA_EFFECT = 1029`（官方枚举里没有），本仓不跟。
//!
//! # 跨 crate 边界的三条自律
//!
//! 1. **本 crate 才有 `unsafe`**：`zephyra-platform` 是 `#![forbid(unsafe_code)]`；
//! 2. **本 crate 不碰渲染**：`DisplayList` / 交换链 / damage 一个都不管 ——
//!    那是 `zephyra-winit-d3d12` 的事；本 crate 只回答"平台支持什么"、
//!    "把窗口外观设成什么样"；
//! 3. **本 crate 不决定 UI**：颜色、圆角值由应用给；本 crate 只负责能不能。
//!
//! # 平台不支持时怎么办
//!
//! 逐项返回 `false`，**不返回错误**（见 [`zephyra_platform::ApplyOutcome`]）。

#![deny(missing_debug_implementations)]

use zephyra_platform::{
    ApplyOutcome, BackdropKind, Capabilities, Platform, ThemePreference, WindowChrome, WindowHandle,
};

mod live_resize;
pub use live_resize::{
    install as install_live_resize, restore as restore_live_resize, set_render_hook, wm_size_count,
    wm_size_skipped, LiveResizeError, RenderHook,
};
mod locale;
mod version;
pub use version::build_number;

/// Windows 平台实现。
///
/// 无状态：版本号与系统主题都是**读一次**的外部事实，缓存在 [`std::sync::OnceLock`] 里
/// （见 [`version::build_number`] 与 [`sys_theme`]），因此本结构体可以零成本复制。
#[derive(Debug, Clone, Copy, Default)]
pub struct Windows;

impl Platform for Windows {
    fn name(&self) -> &'static str {
        "windows"
    }

    fn version(&self) -> Option<(u32, u32, u32)> {
        Some(version::os_version())
    }

    fn capabilities(&self) -> Capabilities {
        let build = build_number();
        // 门槛全部来自文档（docs/win/01）：
        //   dark mode  / corner  → Windows 11 Build 22000
        //   backdrop（ Mica/Acrylic）→ Windows 11 Build 22621
        //   caption/border color   → 文档未在本页标版本，按 22000 保守处理
        //
        // ⚠️ 本仓**不实现 backdrop**：官方路线要求 Windows App SDK，本仓不引入。
        //    所以 `backdrop: false` 是**当前事实**，不是"还没查"。
        let w11 = build >= 22_000;
        // 材质门槛比其它属性高一档：`DWMWA_SYSTEMBACKDROP_TYPE` 文档写明
        // "supported starting with Windows 11 Build 22621"（22H2），
        // 而 dark mode / corner 是 22000。差一个 build，所以分开判。
        let mica = build >= 22_621;
        Capabilities {
            dark_mode: w11,
            corner_preference: w11,
            caption_color: w11,
            border_color: w11,
            backdrop: mica,
            system_theme_query: true,
            // `GetUserPreferredUILanguages` 自 Windows 2000 起可用，
            // 与 build 无关 —— 所以这里恒 true，不跟着 `w11` 走。
            system_language_query: true,
        }
    }

    fn system_theme(&self) -> Option<ThemePreference> {
        sys_theme()
    }

    fn system_languages(&self) -> Vec<String> {
        locale::preferred_ui_languages()
    }
    fn apply_window_chrome(&self, window: WindowHandle, chrome: WindowChrome) -> ApplyOutcome {
        #[cfg(windows)]
        {
            self::apply(window, chrome)
        }
        #[cfg(not(windows))]
        {
            // 理论到不了的分支：这个 crate 是 Windows 专属的。但写出来，
            // 是为了让"跨平台编译"时错误发生在**这一行**而不是某个调用点。
            let _ = (window, chrome);
            ApplyOutcome {
                dark: false,
                corner: false,
                caption_color: false,
                border_color: false,
                backdrop: false,
            }
        }
    }
}

static WINDOWS: Windows = Windows;

/// 取 [`Windows`] 单例。
pub fn platform() -> &'static dyn Platform {
    &WINDOWS
}

/// 当前编译目标的 [`Platform`] 实现。
///
/// Windows 上给 [`Windows`]，否则给 `zephyra_platform::current()` 的空实现
/// —— 这样**应用不需要任何 `cfg`** 就能编译，只在 Linux 上拿到"全 false"。
pub fn current() -> &'static dyn Platform {
    if cfg!(windows) {
        &WINDOWS
    } else {
        zephyra_platform::current()
    }
}

// ---------------------------------------------------------------- Windows 实现

/// 真正调 DWM 的那段。
///
/// 抽成独立函数是为了让 `apply_window_chrome` 保持可读；也因为这一段是本 crate
/// **唯一**允许出现 `unsafe` 的地方 —— review 时只需要盯这几十行。
#[cfg(windows)]
fn apply(window: WindowHandle, chrome: WindowChrome) -> ApplyOutcome {
    use windows::Win32::Foundation::HWND;

    let caps = Windows.capabilities();
    // ⚠️ 句柄无效时**什么都不做**。0 号 HWND 传给 DWM 不一定崩，但"崩了才修"
    // 是错的 —— 无效句柄属于调用方的时序问题（窗口还没建好），必须静默跳过。
    let hwnd = HWND(window.raw() as *mut std::ffi::c_void);
    if !window.is_valid() {
        return ApplyOutcome {
            dark: false,
            corner: false,
            caption_color: false,
            border_color: false,
            backdrop: false,
        };
    }

    let dark = if caps.dark_mode {
        // DWMWA_USE_IMMERSIVE_DARK_MODE = 20，BOOL。
        // 文档：Use with DwmSetWindowAttribute；取值类型 BOOL。
        set_dwm(
            hwnd,
            DwmAttr::UseImmersiveDarkMode,
            &BOOL32(chrome.dark as u32),
        )
    } else {
        false
    };

    let corner = if caps.corner_preference {
        // DWMWA_WINDOW_CORNER_PREFERENCE = 33，DWM_WINDOW_CORNER_PREFERENCE（int32）。
        set_dwm(
            hwnd,
            DwmAttr::WindowCornerPreference,
            &I32(chrome.corner.raw()),
        )
    } else {
        false
    };

    let caption_color = match chrome.caption_color {
        // DWMWA_CAPTION_COLOR = 35，COLORREF。
        Some(rgb) if caps.caption_color => set_dwm(hwnd, DwmAttr::CaptionColor, &COLORREF(rgb)),
        _ => false,
    };

    let border_color = match chrome.border_color {
        // DWMWA_BORDER_COLOR = 34，COLORREF。
        Some(rgb) if caps.border_color => set_dwm(hwnd, DwmAttr::BorderColor, &COLORREF(rgb)),
        _ => false,
    };

    // 背景材质：`DWMWA_SYSTEMBACKDROP_TYPE = 38`（Build 22621+）。
    //
    // 能力位保护：低于 22621 时 DWM 不认这个属性，调用会返回 `E_INVALIDARG`。
    // 我们不靠报错来判断（那会把"版本不够"和"真失败"混在一起），
    // 而是**先问能力位**。
    //
    // `BackdropKind::None` **也要写**（`DWMSBT_NONE`）：不写的话属性停留在
    // 默认 `DWMSBT_AUTO`，而文档说 AUTO 由 DWM 决定、**可能仍然画**材质 ——
    // 于是"关掉 Mica"关不掉。
    let backdrop = if caps.backdrop {
        match dwm_backdrop(chrome.backdrop) {
            Some(kind) => set_dwm(hwnd, DwmAttr::SystemBackdropType, &kind),
            None => false,
        }
    } else {
        false
    };

    ApplyOutcome {
        dark,
        corner,
        caption_color,
        border_color,
        backdrop,
    }
}

/// `DwmSetWindowAttribute` 的薄封装。
///
/// # 为什么收 `i32` 而不是 `DWMWINDOWATTRIBUTE`
///
/// 文档枚举里只有 `DWMWA_NCRENDERING_ENABLED = 1` 与
/// `DWMWA_USE_IMMERSIVE_DARK_MODE = 20`、`DWMWA_WINDOW_CORNER_PREFERENCE = 33`
/// 是显式赋值，其余是隐式递增 —— 而 `windows` crate 的 `DWMWINDOWATTRIBUTE`
/// 是**强类型 newtype**。传裸 `i32` 会让"属性号写错"变成运行期静默失败
/// （DWM 返回 `E_INVALIDARG`，我们返回 `false`，上层只看到"没生效"）。
/// 所以这里**保持强类型**：调用点必须写 `DwmAttr::USE_IMMERSIVE_DARK_MODE`
/// 这种带名字的东西，写错数字编不过。
#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DwmAttr {
    /// `DWMWA_USE_IMMERSIVE_DARK_MODE` = 20，`BOOL`。
    UseImmersiveDarkMode,
    /// `DWMWA_BORDER_COLOR` = 34，`COLORREF`。
    BorderColor,
    /// `DWMWA_CAPTION_COLOR` = 35，`COLORREF`。
    CaptionColor,
    /// `DWMWA_WINDOW_CORNER_PREFERENCE` = 33，`DWM_WINDOW_CORNER_PREFERENCE`。
    WindowCornerPreference,
    /// `DWMWA_SYSTEMBACKDROP_TYPE` = 38，`DWM_SYSTEMBACKDROP_TYPE`。
    ///
    /// 门槛比上面几个高一档：Build 22621（22H2）。见 [`Platform::capabilities`]。
    SystemBackdropType,
}

/// [`BackdropKind`] → DWM 材质枚举。
///
/// 取值来自 `DWM_SYSTEMBACKDROP_TYPE`（`docs/win/02-mica.md` §1），用
/// `windows` crate 的**强类型常量**，不写裸数字。
#[cfg(windows)]
fn dwm_backdrop(
    kind: BackdropKind,
) -> Option<windows::Win32::Graphics::Dwm::DWM_SYSTEMBACKDROP_TYPE> {
    use windows::Win32::Graphics::Dwm::{
        DWMSBT_MAINWINDOW, DWMSBT_NONE, DWMSBT_TABBEDWINDOW, DWMSBT_TRANSIENTWINDOW,
    };
    Some(match kind {
        // `BackdropKind::None` 的语义是"**不**画材质" ⇒ `DWMSBT_NONE`，
        // 不能用 `DWMSBT_AUTO`：文档说 AUTO 会"由 DWM 决定"，可能仍然画。
        // 显式 NONE 才是"确定不画"。
        BackdropKind::None => DWMSBT_NONE,
        // DWMSBT_MAINWINDOW：长生命周期窗口 = Mica（默认变体）。
        BackdropKind::Mica => DWMSBT_MAINWINDOW,
        // DWMSBT_TABBEDWINDOW：Mica Alt（备用变体）。
        BackdropKind::MicaAlt => DWMSBT_TABBEDWINDOW,
        // DWMSBT_TRANSIENTWINDOW：Acrylic。
        BackdropKind::Acrylic => DWMSBT_TRANSIENTWINDOW,
    })
}

#[cfg(windows)]
impl DwmAttr {
    /// 转为 `windows` crate 的枚举 newtype。
    fn raw(self) -> windows::Win32::Graphics::Dwm::DWMWINDOWATTRIBUTE {
        use windows::Win32::Graphics::Dwm::{
            DWMWA_BORDER_COLOR, DWMWA_CAPTION_COLOR, DWMWA_SYSTEMBACKDROP_TYPE,
            DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE,
        };
        match self {
            Self::UseImmersiveDarkMode => DWMWA_USE_IMMERSIVE_DARK_MODE,
            Self::BorderColor => DWMWA_BORDER_COLOR,
            Self::CaptionColor => DWMWA_CAPTION_COLOR,
            Self::WindowCornerPreference => DWMWA_WINDOW_CORNER_PREFERENCE,
            Self::SystemBackdropType => DWMWA_SYSTEMBACKDROP_TYPE,
        }
    }
}

/// 写一个 DWM 窗口属性；成功返回 `true`。
#[cfg(windows)]
fn set_dwm(
    hwnd: windows::Win32::Foundation::HWND,
    attr: DwmAttr,
    value: &(impl AsDwmValue + ?Sized),
) -> bool {
    use windows::Win32::Graphics::Dwm::DwmSetWindowAttribute;
    let bytes = value.as_bytes();
    // SAFETY:
    // - `hwnd` 已由调用点判过非空；
    // - `bytes` 是本函数栈上 4 字节，DWM 按 `cbAttribute` 同步读取，生命周期覆盖调用；
    // - `attr` 全部来自文档枚举（见 [`DwmAttr`]），值类型与文档一致
    //   （BOOL / COLORREF / 枚举 int32）。
    unsafe {
        DwmSetWindowAttribute(hwnd, attr.raw(), bytes.as_ptr().cast(), bytes.len() as u32).is_ok()
    }
}

/// 能被当作 DWM 属性值的 4 字节标量。
#[cfg(windows)]
trait AsDwmValue {
    fn as_bytes(&self) -> [u8; 4];
}

#[cfg(windows)]
impl AsDwmValue for BOOL32 {
    fn as_bytes(&self) -> [u8; 4] {
        self.0.to_le_bytes()
    }
}

#[cfg(windows)]
impl AsDwmValue for I32 {
    fn as_bytes(&self) -> [u8; 4] {
        self.0.to_le_bytes()
    }
}

#[cfg(windows)]
impl AsDwmValue for COLORREF {
    fn as_bytes(&self) -> [u8; 4] {
        self.0.to_le_bytes()
    }
}

/// `DWM_SYSTEMBACKDROP_TYPE` 是 newtype 包 `i32`，按 4 字节传。
#[cfg(windows)]
impl AsDwmValue for windows::Win32::Graphics::Dwm::DWM_SYSTEMBACKDROP_TYPE {
    fn as_bytes(&self) -> [u8; 4] {
        self.0.to_le_bytes()
    }
}

#[cfg(windows)]
#[derive(Debug, Clone, Copy)]
struct BOOL32(u32);

#[cfg(windows)]
#[derive(Debug, Clone, Copy)]
struct I32(i32);

#[cfg(windows)]
#[derive(Debug, Clone, Copy)]
struct COLORREF(u32);

// ---------------------------------------------------------------- 系统主题

/// 读系统深浅色偏好。
///
/// # 数据源与依据
///
/// 注册表 `HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize`：
/// `AppsUseLightTheme`（`DWORD`，0 = 深色）。
///
/// ⚠️ **本轮尚未查证**：Microsoft 目前没有公开文档页描述这两个注册表值
/// （它们是**实现细节**）。Rin-UI 用的是 `darkdetect`（读同一处）。
/// 按本仓「写 Windows 优化先查文档」的规矩，这条记为**未确证**，
/// 因此实现上：
///
/// - 读不到 / 键不存在 ⇒ 返回 `None`（**不猜**）；
/// - 不设置任何东西（只读）；
/// - 门禁只要求"键存在时返回值与注册表一致"，不存在时不失败。
///
/// 批 3 若要接变更通知（`WM_SETTINGCHANGE`），需先补一篇文档；见
/// `docs/win/04-sys-theme.md`。
fn sys_theme() -> Option<ThemePreference> {
    #[cfg(windows)]
    {
        use windows::core::PCWSTR;
        use windows::Win32::System::Registry::*;

        /// 键路径的 NUL 结尾 UTF-16 缓冲。
        ///
        /// ⚠️ 必须是 `static`：`PCWSTR` 只是裸指针，若底层缓冲是局部 `Vec`，
        ///    函数返回后指针悬垂 —— 而 `RegOpenKeyExW` 此刻已经读了它，
        ///    于是表现为**偶发的**栈上乱码路径。这类 bug 在 debug 下几乎抓不到。
        ///
        /// 写成十六进制数组而不是 `"...".encode_utf16()` 后 `as_ptr()`，
        /// 就是为了让"它是 static"这件事在代码上可见。
        static KEY_PATH: &[u16] = &[
            0x0053, 0x006f, 0x0066, 0x0074, 0x0077, 0x0061, 0x0072, 0x0065, // Software
            0x005c, // \
            0x004d, 0x0069, 0x0063, 0x0072, 0x006f, 0x0073, 0x006f, 0x0074, // Microsoft
            0x005c, // \
            0x0057, 0x0069, 0x006e, 0x0064, 0x006f, 0x0077, 0x0073, // Windows
            0x005c, // \
            0x0043, 0x0075, 0x0072, 0x0072, 0x0065, 0x006e, 0x0074, // Current
            0x005c, // \
            0x0056, 0x0065, 0x0072, 0x0073, 0x0069, 0x006f, 0x006e, // Version
            0x005c, // \
            0x0054, 0x0068, 0x0065, 0x006d, 0x0065, 0x0073, // Themes
            0x005c, // \
            0x0050, 0x0065, 0x0072, 0x0073, 0x006f, 0x006e, 0x0061, 0x006c, 0x0069, 0x007a,
            0x0065, // Personalize
            0x0000,
        ];
        static VALUE_NAME: &[u16] = &[
            0x0041, 0x0070, 0x0070, 0x0073, 0x0055, 0x0073, 0x0065, 0x004c, 0x0069, 0x0067, 0x0068,
            0x0074, 0x0054, 0x0068, 0x0065, 0x006d, 0x0065, // AppsUseLightTheme
            0x0000,
        ];
        // SAFETY: 键路径与值名都是 NUL 结尾的 `static` 缓冲（生命周期 = 整个进程），
        // ulOptions = Some(0) 表示不重定向，phkResult 指向本函数的栈上 HKEY。
        let mut hkey = HKEY::default();
        let opened = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(KEY_PATH.as_ptr()),
                Some(0),
                KEY_READ,
                &mut hkey,
            )
        };
        if opened.is_err() {
            return None;
        }

        let mut value: u32 = 0;
        let mut len = std::mem::size_of::<u32>() as u32;
        // SAFETY: hkey 已打开且由本函数拥有；缓冲区是栈上 4 字节，len 与之相符；
        // lpreserved / lptype / lpData 三项传 None 表示不接收（签名是
        // `Option<*const u32>` / `Option<*mut REG_VALUE_TYPE>`，不是裸指针）。
        let read = unsafe {
            RegQueryValueExW(
                hkey,
                PCWSTR(VALUE_NAME.as_ptr()),
                None,
                None,
                Some(std::ptr::addr_of_mut!(value).cast()),
                Some(&mut len),
            )
        };
        // SAFETY: hkey 由本函数打开，成功与否都必须关，否则注册表句柄泄漏。
        unsafe {
            let _ = RegCloseKey(hkey);
        }
        if read.is_err() || len < 4 {
            return None;
        }
        Some(if value != 0 {
            ThemePreference::Light
        } else {
            ThemePreference::Dark
        })
    }
    #[cfg(not(windows))]
    {
        let _ = ();
        None
    }
}

#[cfg(test)]
#[path = "tests/lib_tests.rs"]
mod tests;

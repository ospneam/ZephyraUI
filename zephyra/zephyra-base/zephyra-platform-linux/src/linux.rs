//! Linux 实现的 [`Platform`]。读法与 Windows 那份逐条对应，判据写在类型上。

use std::sync::OnceLock;

use zephyra_platform::{
    ApplyOutcome, Capabilities, Platform, ThemePreference, WindowChrome, WindowHandle,
};

/// 系统深浅色探测结果在首访时定下，之后不变。
///
/// # 为什么用 `OnceLock` 而不是 `static LINUX: Linux = ...`
///
/// [`Linux::new`] 要读文件、可能要拉一次子进程 —— 那不是 `const` 能做的事。
/// 而每次调用都新建一份等于每次重读 `/etc/os-release`；`Box::leak` 则是把泄漏
/// 放在一个**会被反复调用**的位置。`OnceLock` 两头都躲开：初始化一次，之后
/// 返回同一个引用。
static LINUX: OnceLock<Linux> = OnceLock::new();

/// Linux 平台实现。
#[derive(Debug)]
pub struct Linux {
    /// 深浅色探测结果。`None` = 读不到（**不是**「浅色」）。
    theme: Option<ThemePreference>,
    /// 语言偏好，有序，最偏好的在前。空表 = 读不到。
    languages: Vec<String>,
    /// OS 版本。`None` = 读不到。
    version: Option<(u32, u32, u32)>,
}

impl Linux {
    /// 探测一次并缓存全部「启动期的外部事实」。
    ///
    /// 三个探测互相独立、互不依赖：一个失败不该让另两个也返回「读不到」。
    pub fn new() -> Self {
        Self {
            theme: probe_theme(),
            languages: probe_languages(),
            version: probe_version(),
        }
    }

    /// 不做任何探测的实现。给测试与「已知事实由调用方提供」的场合。
    pub fn with_facts(
        theme: Option<ThemePreference>,
        languages: Vec<String>,
        version: Option<(u32, u32, u32)>,
    ) -> Self {
        Self {
            theme,
            languages,
            version,
        }
    }
}

impl Default for Linux {
    fn default() -> Self {
        Self::new()
    }
}

impl Platform for Linux {
    fn name(&self) -> &'static str {
        "linux"
    }

    fn version(&self) -> Option<(u32, u32, u32)> {
        self.version
    }

    fn capabilities(&self) -> Capabilities {
        // 只有「查」是真的；「按请求设置」逐项为 false。理由逐条写在模块文档的表里。
        //
        // ⚠ 这里**不能**因为「Linux 上有 gsettings 能改主题」就把 `dark_mode` 报 true：
        // 那改的是**整个桌面**的主题，不是这一个窗口。应用据此决定要不要画 Mica，
        // 报 true 而实际什么都没发生，比报 false 难查得多。
        Capabilities {
            dark_mode: false,
            corner_preference: false,
            caption_color: false,
            border_color: false,
            backdrop: false,
            system_theme_query: true,
            system_language_query: true,
        }
    }

    fn system_theme(&self) -> Option<ThemePreference> {
        self.theme
    }

    fn system_languages(&self) -> Vec<String> {
        self.languages.clone()
    }

    fn apply_window_chrome(&self, window: WindowHandle, chrome: WindowChrome) -> ApplyOutcome {
        // 不读 window、不写 window，连句柄都不碰。
        //
        // 为什么不「尽力而为」地往 X11 上设 `_GTK_FRAME_EXTENTS` 之类私有约定：
        // 那类约定要么无效、要么在别的合成器上产生别的行为，而**返回值只会说
        // 「设上了」** —— 一个说谎的 `true` 比诚实的 `false` 贵得多。
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

/// 取本机唯一的 [`Linux`] 实例。
///
/// 返回 `&'static`：`OnceLock` 的内容一旦初始化就不再释放，所以内部引用
/// 本身就是 `'static`。`get_or_init` 保证**一定有值**，所以这里没有 `Option`
/// —— 探测失败已经体现在每个字段里（`theme: None` / 空表），而不是让调用点
/// 再判一次「拿到实现了吗」。
pub fn current() -> &'static Linux {
    LINUX.get_or_init(Linux::new)
}

// ── 探测 ────────────────────────────────────────────────────────────────

/// OS 版本：`/etc/os-release` 优先（发行版版本），退化到内核版本。
///
/// 为什么要发行版版本而不是内核版本：本 trait 回答的是「这台机器的系统版本」，
/// 而 UI 能力门槛（如「22H2 才有 Mica」）是按**发行版**说的。内核更细，但它
/// 答的是另一个问题。
fn probe_version() -> Option<(u32, u32, u32)> {
    if let Ok(text) = std::fs::read_to_string("/etc/os-release") {
        if let Some(v) = parse_os_release(&text) {
            return Some(v);
        }
    }
    std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .ok()
        .and_then(|s| parse_version_prefix(&s))
}

/// 从 `os-release` 文本取 `VERSION_ID`，解析成 `(major, minor, build)`。
///
/// 取 `VERSION_ID` 而不是 `VERSION`：后者的值形如 `24.04.3 LTS (Noble Numbat)`，
/// 里面带发行版自己的修饰语，解析它就得写一个「尽量取开头数字」的启发式 ——
/// 而 `VERSION_ID` 本来就是规范里给机器读的那一项。
pub fn parse_os_release(text: &str) -> Option<(u32, u32, u32)> {
    for line in text.lines() {
        let Some(rest) = line.strip_prefix("VERSION_ID=") else {
            continue;
        };
        let value = rest.trim().trim_matches('"');
        return parse_version_prefix(value);
    }
    None
}

/// 从一段文本开头取 `major[.minor[.build]]`。
///
/// 只认**开头**的数字，且三段都必须是纯数字：内核版本尾部的
/// `-microsoft-standard-WSL2` 是发行版后缀，不是版本号的一部分。
pub fn parse_version_prefix(text: &str) -> Option<(u32, u32, u32)> {
    let mut parts = text.trim().split('.').map(str::trim);
    let major = parse_component(parts.next()?)?;
    let minor = parts.next().map(parse_component).unwrap_or(Some(0))?;
    let build = parts.next().map(parse_component).unwrap_or(Some(0))?;
    Some((major, minor, build))
}

/// 一段版本号：允许 `22621` 这种**不带点**的 build（Windows 的写法，Linux 上少见但合法）。
fn parse_component(text: &str) -> Option<u32> {
    let digits: String = text.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse().ok()
}

/// 深浅色：`gsettings` 优先，退化到 `GTK_THEME`，再退化到 `None`。
///
/// # 为什么要先看「有没有会话总线」
///
/// `gsettings` 读的是 GSettings，而 GSettings 的后端在 GNOME 上是 **D-Bus**。
/// 没有会话总线的环境（容器、ssh、cron）里这条命令要么不存在、要么几秒后超时 ——
/// 而首帧等它几秒是最坏的时机。所以先看 `DBUS_SESSION_BUS_ADDRESS` / `XDG_RUNTIME_DIR`。
fn probe_theme() -> Option<ThemePreference> {
    if session_bus_present() {
        if let Ok(out) = std::process::Command::new("gsettings")
            .args(["get", "org.gnome.desktop.interface", "color-scheme"])
            .output()
        {
            if out.status.success() {
                if let Some(t) = parse_gsettings_theme(&String::from_utf8_lossy(&out.stdout)) {
                    return Some(t);
                }
            }
        }
    }
    std::env::var("GTK_THEME")
        .ok()
        .and_then(|t| parse_gtk_theme(&t))
}

/// 有没有会话总线的迹象。
fn session_bus_present() -> bool {
    std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some()
        || std::env::var_os("XDG_RUNTIME_DIR").is_some()
}

/// 解析 `gsettings get org.gnome.desktop.interface color-scheme` 的输出。
///
/// 输出是带引号的 GVariant 字符串：`'prefer-dark'`。认两种取值：
/// `prefer-dark` ⇒ 深色，`default` ⇒ 浅色（GNOME 的「浅色」就是 default）。
/// 其它取值（未来新增档）返回 `None` —— 上层据此**不跟随**系统主题，
/// 而不是跟随成浅色。
pub fn parse_gsettings_theme(stdout: &str) -> Option<ThemePreference> {
    let v = stdout.trim().trim_matches('\'').trim();
    match v {
        "prefer-dark" => Some(ThemePreference::Dark),
        "default" | "prefer-light" => Some(ThemePreference::Light),
        _ => None,
    }
}

/// 从 `GTK_THEME` 猜深浅色。
///
/// 约定俗成的后缀是 `-dark`（`Adwaita-dark`、`Yaru-dark:...`）。这是**启发式**，
/// 因此只在 `gsettings` 读不到时才用。
pub fn parse_gtk_theme(value: &str) -> Option<ThemePreference> {
    let theme = value.split(':').next().unwrap_or(value).trim();
    if theme.is_empty() {
        return None;
    }
    if theme
        .rsplit('-')
        .next()
        .is_some_and(|last| last.eq_ignore_ascii_case("dark"))
    {
        Some(ThemePreference::Dark)
    } else {
        Some(ThemePreference::Light)
    }
}

/// 语言偏好：`LANGUAGE` 优先（**它本来就是有序表**），退化到 `LC_ALL` / `LC_MESSAGES` / `LANG`。
///
/// # 为什么 `LANGUAGE` 是最像的那一项
///
/// trait 要求的是「对应 `navigator.languages` 的有序 BCP 47 标签表」。
/// `LANGUAGE` 是 gettext 的标准环境变量，语义正是「冒号分隔、按优先级排列」——
/// 那是 Linux 上唯一一个**为这个形状设计过**的来源。`LANG` 只有一个值，
/// 它给出的是「兜底语言」而不是「偏好顺序」。
fn probe_languages() -> Vec<String> {
    for key in ["LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"] {
        let Ok(raw) = std::env::var(key) else {
            continue;
        };
        let list: Vec<String> = raw.split(':').filter_map(normalize_locale).collect();
        if !list.is_empty() {
            return list;
        }
    }
    Vec::new()
}

/// `zh_CN.UTF-8` → `zh-CN`；去掉 `@modifier`；`C` / `POSIX` 返回 `None`。
///
/// `C` / `POSIX` 不是语言，是「没设置」。报成 `zh` 之类会让上层以为界面该钉在
/// 某门上 —— 空表才表示「读不到」，上层据此回落默认语言。
pub fn normalize_locale(tag: &str) -> Option<String> {
    let tag = tag.trim();
    if tag.is_empty() {
        return None;
    }
    let base = tag.split(['.', '@']).next().unwrap_or(tag).trim();
    if base.is_empty() || base == "C" || base == "POSIX" {
        return None;
    }
    Some(base.replace('_', "-"))
}

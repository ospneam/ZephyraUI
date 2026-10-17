#![deny(rustdoc::broken_intra_doc_links)]
//! Linux 实现 [`zephyra_platform::Platform`] —— 与 `zephyra-platform-win` **同级**，
//! 两个平台互不认识。
//!
//! # 三条 crate 边界（与 Windows 实现逐条相同）
//!
//! 1. **不碰渲染**：`DisplayList` / 交换链 / damage 一个都不管，那是宿主的事；
//! 2. **不决定 UI**：深浅色、圆角由应用给，能力层只回答「能不能」；
//! 3. **不引控件层**：只依赖 `zephyra-platform`。
//!
//! # 与 Windows 实现的**真实差别**，以及为什么能力位大半是 `false`
//!
//! | 能力 | Windows | Linux | 原因 |
//! |---|---|---|---|
//! | 暗色（**按窗口**） | DWM 属性 | ❌ | Wayland 下合成器不接逐窗口配色请求；X11 也没有对应协议 |
//! | 圆角（按窗口） | `DWMWCP_*` | ❌ | 圆角由合成器 / CSD 决定，应用侧没有可移植的请求方式 |
//! | 标题栏 / 边框配色 | DWM 属性 | ❌ | 同上 |
//! | 系统背景材质（Mica） | DWM 材质 | ❌ | Linux 侧**没有**等价物（见下） |
//! | **查**系统深浅色 | 注册表 | ✅ | `gsettings` / `GTK_THEME` |
//! | **查**语言偏好 | `GetUserPreferredUILanguages` | ✅ | `LANGUAGE`（gettext 约定，天然是有序表）|
//!
//! 「按请求设置」这一列在 Linux 上几乎全是 `false`，而「查询」几乎都是 `true`。
//! 这不是偷懒，是本层契约写明的形状：**能力位缺失必须是降级，不能是报错**
//! （见 `zephyra-platform` 的模块文档）。应用该做的是「界面退回实色」，而那正是
//! 本 crate 返回 `false` 时它拿到的答案。
//!
//! # 关于「背景材质」为什么不算 `true`
//!
//! Linux 上让窗口透明 + 合成器在后面画东西（GNOME 的透出、KDE 的模糊）是**做得到**
//! 的，但它的前提是**应用自己有一张没画满的窗口**，而「系统替你画材质」这件事
//! 在 X11/Wayland 上没有可移植接口。报 `true` 会让上层以为「设了就能看见」——
//! 那正是 `BackdropKind` 文档里点名要避免的误读（「`true` 只表示调用会被接受，
//! 不表示画面上看得见」）。
//!
//! # 提问方式：只用 `std`，不引依赖
//!
//! - 版本：`/etc/os-release` 的 `VERSION_ID`，退化到 `/proc/sys/kernel/osrelease`；
//! - 深浅色：`gsettings get org.gnome.desktop.interface color-scheme`，退化到
//!   `GTK_THEME`，再退化到 `None`；
//! - 语言：`LANGUAGE`（gettext 约定，**冒号分隔且有顺序**），退化到
//!   `LC_ALL` → `LC_MESSAGES` → `LANG`。
//!
//! 拉 `gsettings` 是**子进程**，所以结果**只在首次查询后缓存**（[`current`]
//! 的单例里）。这是刻意的：主题在进程存活期间不会变，而每帧拉一次子进程会把
//! 「问一次事实」变成「每帧 fork 一次」。

#![deny(missing_debug_implementations)]

mod linux;

pub use linux::Linux;

/// 取 [`Linux`] 单例。
///
/// 与 `zephyra_platform_win::current()` 同一形状：返回 `&'static dyn Platform`，
/// 应用因此**只调 trait 方法**，不需要知道底下是哪个平台。
///
/// # 为什么内部是 `OnceLock` 而这里能返回 `&'static`
///
/// 探测（读 `/etc/os-release`、可能拉一次 `gsettings`）不是 `const` 能做的事，
/// 所以单例用 `OnceLock` 懒初始化，取出来的引用天然是 `&'static`（内容永不
/// 释放）。每次 `current()` 重读文件、或用 `Box::leak` 每次泄漏一份，都是把代价
/// 放在一个**会被反复调用**的位置 —— 详见 [`Linux::new`] 的说明。
pub fn current() -> &'static dyn zephyra_platform::Platform {
    linux::current()
}

#[cfg(test)]
#[path = "tests/lib_tests.rs"]
mod tests;

//! 空实现：**非 Windows 平台与"平台实现 crate 未启用"时生效**。
//!
//! # 这是 Linux 适配的接入点
//!
//! Linux 上要做的是：
//!
//! - [`Stub::version`] 读 `uname` / `kernel.osrelease`（要不要给由调用方定）；
//! - [`Stub::system_theme`] 查 `org.freedesktop.appearance color-scheme`
//!   （portal 规范，见 `docs/portability.md`）；
//! - [`Stub::apply_window_chrome`] 按 compositor 能力设圆角（Wayland 通常交给
//!   合成器，X11 用 `_GTK_FRAME_EXTENTS` 一类的私有约定）。
//!
//! 全部改动**只在这个文件里**，上层与 `zephyra-platform` 的类型不用动。
//!
//! # 为什么这里返回 `false` 而不是 `todo!()`
//!
//! `todo!()` 会让任何跑到这里的测试直接 panic，而"平台不支持"是**正常运行路径**
//! —— Linux 上没 Mica、没 DWM 是常态。把它做成 panic，等于逼着上层到处写
//! `cfg` 绕过抽象，而那正是本层要消灭的东西。

use crate::{
    ApplyOutcome, Capabilities, Platform, ThemePreference, WindowChrome, WindowHandle,
};

/// 空实现。
#[derive(Debug, Clone, Copy, Default)]
pub struct Stub;

impl Platform for Stub {
    fn name(&self) -> &'static str {
        "stub"
    }

    fn version(&self) -> Option<(u32, u32, u32)> {
        // 拿不到就报"拿不到"。**不返回 `(0,0,0)`** —— 那是个看起来合法的版本号，
        // 而"版本未知"与"版本是 0"在能力判定上的差别很大：0 会让所有
        // `build >= 22000` 的门槛判定为 false（保守、正确），
        // 但会让调用方以为"探测成功且版本很低"。
        None
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::none()
    }

    fn system_theme(&self) -> Option<ThemePreference> {
        None
    }

    fn system_languages(&self) -> Vec<String> {
        // 空表 = 读不到。上层据此回落默认语言，而不是把界面钉在某一门语言上。
        Vec::new()
    }

    fn apply_window_chrome(&self, window: WindowHandle, chrome: WindowChrome) -> ApplyOutcome {
        // 不支持就是不支持；**不读 window、不写 window**，连句柄都不碰。
        // （万一将来有人把 stub 接到 Windows 上，至少不会碰 0 号地址。）
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

static STUB: Stub = Stub;

/// 取 [`Stub`] 单例。
///
/// # 为什么是 `static` 而不是 `Box::leak`
///
/// `Box::leak` 每次调用泄漏一份 —— 而 `current()` 会被热路径（每帧问能力）
/// 调用，那是最糟的泄漏位置。
pub fn platform() -> &'static dyn Platform {
    &STUB
}
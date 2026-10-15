//! OS 版本探测。
//!
//! # 为什么必须用 `RtlGetVersion` 而不是 `GetVersionExW`
//!
//! `GetVersionExW` 的文档明确写着：
//!
//! > The value returned by the GetVersionEx function now **depends on how the
//! > application is manifested**. Applications not manifested for Windows 8.1 or
//! > Windows 10 will **return the Windows 8 OS version value (6.2)**.
//!
//! （出处 `https://learn.microsoft.com/en-us/windows/win32/api/sysinfoapi/nf-sysinfoapi-getversionexw`）
//!
//! 本仓**没有 manifest**，于是 `GetVersionExW` 会说"我是 6.2 / Windows 8"。
//! 而本仓所有能力门槛都是 `build >= 22000`（Win11）—— 拿到 6.2 就意味着
//! **暗色模式、圆角全部静默失效**，而且没有任何报错。
//!
//! `RtlGetVersion` 是 ntdll 里的等价物，**不受 manifest 影响**：
//!
//! > `NTSTATUS RtlGetVersion(_Out_ PRTL_OSVERSIONINFOW lpVersionInformation);`
//! > DLL: `Ntdll.dll`；Header: `Ntddk.h`；Minimum client: Windows 2000。
//!
//! （出处 `https://learn.microsoft.com/en-us/windows/win32/devnotes/rtlgetversion`）
//!
//! # 为什么自己声明 extern
//!
//! `windows` crate 0.62 **没有**绑定 `RtlGetVersion`（`Win32` 与 `Wdk` 两个
//! 命名空间都没有，已 grep 确认）。按文档给的头/库自己声明是唯一的路，
//! 也是最诚实的路：它让我们无法"顺手"用到一个签名不符的绑定。
//!
//! # 缓存
//!
//! 结果缓存进 [`OnceLock`]：版本在进程生命周期内不变，而
//! [`super::Windows::capabilities`] 会被每帧调用（热路径）。

use std::sync::OnceLock;

/// 取真实 build 号；拿不到时 `0`（于是所有 `>= 22000` 的门槛为假 = 保守）。
///
/// 返回 `0` 而不是 panic：能力判定的方向是"不确定就不开"，不是"不确定就崩"。
pub fn build_number() -> u32 {
    static CACHE: OnceLock<u32> = OnceLock::new();
    *CACHE.get_or_init(|| os_version().2)
}

/// 取 `(major, minor, build)`。
pub fn os_version() -> (u32, u32, u32) {
    #[cfg(windows)]
    {
        #[repr(C)]
        #[derive(Default)]
        struct RtlOsVersionInfoW {
            size: u32,
            major: u32,
            minor: u32,
            build: u32,
            platform_id: u32,
        }

        /// 尾部的 `wchar_t csdVersion[128]`。
        ///
        /// 单独拆出来而不是 `#[derive(Default)]` 带过：`[u16; 128]` 在
        /// `Default` 之前不稳定实现。手动 `[0; 128]` 顺带让
        /// `size` 的语义更明显：这里填的是**整个结构体**的大小，而 DWM/内核
        /// 正是靠它区分 OSVERSIONINFO 与 OSVERSIONINFOEXW。
        #[repr(C)]
        struct RtlOsVersionInfoWFull {
            head: RtlOsVersionInfoW,
            /// ⚠️ 长度必须是 128（wchar[128]）。文档说调用方用 `dwOSVersionInfoSize`
            /// 指明用的是哪个结构体版本，长度不对会返回非 STATUS_SUCCESS。
            csd: [u16; 128],
        }

        #[link(name = "ntdll")]
        extern "system" {
            fn RtlGetVersion(lp_version_information: *mut RtlOsVersionInfoWFull) -> i32;
        }

        let mut info = RtlOsVersionInfoWFull {
            head: RtlOsVersionInfoW {
                size: std::mem::size_of::<RtlOsVersionInfoWFull>() as u32,
                ..Default::default()
            },
            csd: [0; 128],
        };
        // SAFETY:
        // - extern 签名按文档核对：`NTSTATUS RtlGetVersion(PRTL_OSVERSIONINFOW)`，
        //   NTSTATUS 是 i32；导出存在于 ntdll（Windows 2000 起，MinClient 已确证）；
        // - `info` 指向栈上有效内存且 `size` 已按约定填好（整结构体大小）；
        // - 无线程局部状态，函数是纯查询。
        let status = unsafe { RtlGetVersion(&mut info) };
        // STATUS_SUCCESS == 0。
        if status == 0 {
            (info.head.major, info.head.minor, info.head.build)
        } else {
            (0, 0, 0)
        }
    }
    #[cfg(not(windows))]
    {
        (0, 0, 0)
    }
}

#[cfg(test)]
#[path = "tests/version_tests.rs"]
mod tests;
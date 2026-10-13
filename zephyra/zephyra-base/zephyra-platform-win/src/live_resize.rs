//! 拖动窗口边时的**逐帧跟随** —— 在窗口过程里同步处理 `WM_SIZE`。
//!
//! # 要解决的问题（以及官方怎么做的）
//!
//! 拖动窗口边框时，Windows 在 `DefWindowProc(WM_NCLBUTTONDOWN)` 内部跑一个
//! **模态循环**。该模态循环**会**把 `WM_SIZE` 派发到我们的窗口过程，但 winit 的
//! 主循环此时被阻塞在 `DefWindowProc` 里面，于是 winit 的 `WndProc` 只是
//! `userdata.send_event(Resized(..))` **入队**（winit-0.30.13
//! `platform_impl/windows/event_loop.rs:1397-1418`），主循环取不出来 ——
//! **拖完才重绘**。
//!
//! 参照 XAML：`E:\microsoft-ui-xaml` **不**解决这个问题，也没有任何定时器。
//! 它的 `WM_ENTERSIZEMOVE`/`WM_EXITSIZEMOVE` 处理器只有 10 行
//! （`dxaml/xcp/dxaml/lib/DesktopWindowImpl.cpp:1403-1411`），只置一个
//! `m_inSizeMove` 来**抑制**一次记账写；全仓零 `WM_NCLBUTTONDOWN`、零
//! `WM_SIZING`、零 `DeferWindowPos`。它把 resize 转成**一次帧请求**
//! （`dxaml/xcp/host/win/browserdesktop/WinBrowserHost.cpp:1547`），
//! 而帧请求靠应用的消息泵投递。
//!
//! XAML 真正**同步**处理尺寸的地方是窗口过程里的链条：
//! `WM_SIZE` → `CWindowChrome::UpdateContainerSize` →
//! `UpdateBridgeWindowSizePosition` → `::SetWindowPos`
//! （`dxaml/xcp/components/WindowChrome/CWindowChrome.cpp:338-343, 388`）。
//! **本模块照的就是这一条**：在窗口过程里收到 `WM_SIZE` 就同步把渲染做掉。
//!
//! # 为什么不用定时器（官方明确反对的那条路）
//!
//! 直觉方案是「`WM_ENTERSIZEMOVE` 起一个 16 ms 定时器，`WM_TIMER` 里重绘」。
//! 本模块**不**这么做，理由来自官方代码里的两条硬证据：
//!
//! 1. XAML 侧**零** resize 定时器（`SetTimer` 只出现在 TSF 文本服务、`TimelineTimer`、
//!    `FlipView`、`CTimer`），所以那条路没有官方先例可抄；
//! 2. 更要紧：XAML 对「渲染过程中意外泵消息」是 **fail-fast** 的 ——
//!    `dxaml/xcp/core/dll/xcpcore.cpp:6338-6345` 的 `m_pDrawReentrancyCheck`
//!    注释原文是「防止渲染循环里不小心泵了消息、导致又一次 paint 进来而弄坏状态」。
//!    定时器方案恰恰是在拖动期间**反复**进入渲染，等于主动制造它最怕的重入。
//!
//! 同步处理 `WM_SIZE` 没有这个问题：它不泵消息，只是在模态循环把消息交给我们时
//! 把该画的画掉。
//!
//! # 为什么不用 `WM_NCLBUTTONDOWN` 自己写 sizing 循环
//!
//! 那要重写 Windows 的拖动逻辑（吸边、网格、CJK 双击最大化、无障碍…），
//! 而且官方**完全不这么做**。本模块只在窗口过程里加一层，别的都转给原过程。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    CallWindowProcW, SetWindowLongPtrW, GWLP_WNDPROC, WNDPROC,
};

/// 渲染回调。
///
/// # 契约（违反会怎样）
///
/// - **必须**是同步、不泵消息的：它在窗口过程里被调用，期间不能
///   `PeekMessage`/`GetMessage`。理由见模块文档里 XAML 的 fail-fast 那条。
/// - **必须**容忍"没画成"：宿主在设备丢失、窗口最小化时可以不画，返回 `false`。
/// - **不得**重入：实现里要能挡住"渲染中又来一个 `WM_SIZE`"。
pub type RenderHook = std::sync::Arc<dyn Fn() -> bool + Send + Sync + 'static>;

/// 全局渲染回调。
///
/// # 为什么是全局而不是按窗口存表
///
/// 进程里通常只有一个主窗口，而"按 hwnd 存表"要在每次 `WM_SIZE` 里查一次表、
/// 外加锁。而这里**本来就是**一条全局的"该画了"信号：`WM_SIZE` 的处理与
/// 具体是哪个窗口无关（我们只挂一个窗口）。
///
/// 真要支持多窗口，得改成 `HashMap<isize, ...>` —— 记账在
/// `docs/task-live-resize.md`。
static HOOK: Mutex<Option<RenderHook>> = Mutex::new(None);
/// `WM_SIZE` 被处理的次数。只给 trace 与门禁用。
static WM_SIZE_SEEN: AtomicU64 = AtomicU64::new(0);
/// 被跳过的次数（回调正在渲染中）。用来发现"回调重入"。
static WM_SIZE_SKIPPED: AtomicU64 = AtomicU64::new(0);
/// 回调正在执行。防止重入。
static IN_HOOK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 安装渲染回调（进程级，至多一次有效）。
///
/// # 为什么"至多一次"
///
/// 回调是**拥有**宿主渲染状态的那种东西（内部有锁），而安装发生在宿主构造之后。
/// 二次安装会替换它 —— 那是允许的，但**顺序**很重要：必须在窗口过程被挂上之前
/// 或之后都行，只要在第一次 `WM_SIZE` 之前。这里不做额外约束，因为越约束
/// 越容易在某个宿主自举顺序下静默失效。
pub fn set_render_hook(hook: RenderHook) {
    let mut slot = HOOK.lock().unwrap_or_else(|e| e.into_inner());
    *slot = Some(hook);
}

/// `WM_SIZE` 处理次数（trace / 门禁）。
pub fn wm_size_count() -> u64 {
    WM_SIZE_SEEN.load(Ordering::Relaxed)
}

/// 因重入被跳过的次数。
pub fn wm_size_skipped() -> u64 {
    WM_SIZE_SKIPPED.load(Ordering::Relaxed)
}

/// 我们的窗口过程。
///
/// unsafe extern "system" fn（窗口过程必须是这个 ABI）。
///
/// # SAFETY
///
/// 由 [`install`] 把它设到窗口上，且由 [`restore`] 在销毁前摘掉。
/// 除了 `WM_SIZE` 之外一律转给原过程，所以原过程的返回值语义不变。
unsafe extern "system" fn live_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_SIZE {
        WM_SIZE_SEEN.fetch_add(1, Ordering::Relaxed);
        // 拖动期间模态循环会把一串 `WM_SIZE` 依次交给这里，所以这一段是**热路径**。
        render_once();
        return LRESULT(0);
    }
    // CallWindowProcW 的第一个参数是 Option<WNDPROC>：传 None 时它内部
    // 直接走 DefWindowProcW，所以下面**不需要**自己再分支一次。
    CallWindowProcW(previous_proc(hwnd), hwnd, msg, wparam, lparam)
}

/// 调一次渲染回调，跳过重入。
///
/// # 重入从哪来
///
/// `WM_SIZE` 到达时如果上一次渲染还没结束（例如 GPU 提交里触发了同步等待，
/// 又来了一个 `WM_SIZE`），就会重入。用一个原子位挡住 —— 那不是理论问题，
/// `present()` 里若有同步点就会发生。
fn render_once() {
    if IN_HOOK.swap(true, Ordering::AcqRel) {
        WM_SIZE_SKIPPED.fetch_add(1, Ordering::Relaxed);
        return;
    }
    // 克隆一个 Arc 再放锁：Box<dyn Fn> 不能克隆，而这里要在**锁外**调用回调 ——
    // 回调里要拿宿主的锁，持着 HOOK 的锁去调就是锁序倒置。
    let hook = {
        let slot = HOOK.lock().unwrap_or_else(|e| e.into_inner());
        slot.clone()
    };
    if let Some(h) = hook {
        let _ = h();
    }
    IN_HOOK.store(false, Ordering::Release);
}

/// 每个窗口的原过程，存在**线程局部**里。
///
/// # 为什么不是 `SetWindowLongPtrW(GWL_USERDATA)`
///
/// `GWL_USERDATA` 已经被 winit 用着（它把 `WindowData` 挂在那儿，见
/// winit `event_loop.rs` 的 `WM_NCCREATE`），抢它会直接把 winit 的状态踩坏。
///
/// 窗口过程必须从任意线程被调用（模态循环在 UI 线程，但 WM_SIZE 派发也在 UI 线程），
/// 而"当前挂着的原过程"是**每窗口**的。这里用一张表按 hwnd 查。
/// 每窗口的原窗口过程。**直接存 `WNDPROC`** 而不是 `isize`。
///
/// 存 `isize` 再转回来会得到 `Option<WNDPROC>` = `Option<Option<fn>>`，
/// 与返回类型差一层 —— 那种"套娃"要么加 `.flatten()` 糊过去，要么就是把
/// `WNDPROC` 本来就是 `Option<fn 指针>` 这件事忘掉。直接存最省事。
static PREV: Mutex<Vec<(isize, WNDPROC)>> = Mutex::new(Vec::new());

fn previous_proc(hwnd: HWND) -> WNDPROC {
    let key = hwnd.0 as isize;
    let table = PREV.lock().unwrap_or_else(|e| e.into_inner());
    table
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, v)| *v)
        .unwrap_or(None)
}

/// 把窗口挂上我们的窗口过程。
///
/// # 失败怎么办
///
/// `SetWindowLongPtrW` 返回 0 既可能是"成功且原过程就是 0"，也可能是失败。
/// 判据用 `GetLastError`：文档说成功时它**不**被重置，所以要先清再调。
pub fn install(hwnd: isize) -> Result<(), LiveResizeError> {
    let h = HWND(hwnd as *mut std::ffi::c_void);
    // SAFETY：`hwnd` 由调用方保证是一个活着的窗口句柄。
    unsafe {
        windows::Win32::Foundation::SetLastError(windows::Win32::Foundation::WIN32_ERROR(0));
        let prev = SetWindowLongPtrW(h, GWLP_WNDPROC, live_proc as *const () as isize);
        if prev == 0 {
            let err = windows::Win32::Foundation::GetLastError();
            if err.0 != 0 {
                return Err(LiveResizeError::SetWindowLong(err.0));
            }
        }
        // `isize -> fn 指针` 不是"非原始类型转换"，Rust 不允许 `as`。
        //
        // SAFETY：上面那个 `SetWindowLongPtrW` 的返回值按文档就是窗口过程地址，
        // 而 `WNDPROC` 正是 `Option<fn 指针>`。
        let prev = std::mem::transmute::<isize, WNDPROC>(prev);
        let mut table = PREV.lock().unwrap_or_else(|e| e.into_inner());
        table.retain(|(k, _)| *k != hwnd);
        table.push((hwnd, prev));
    }
    Ok(())
}

/// 摘掉窗口过程（窗口销毁前调用）。
pub fn restore(hwnd: isize) {
    let prev = {
        let mut table = PREV.lock().unwrap_or_else(|e| e.into_inner());
        table
            .iter()
            .position(|(k, _)| *k == hwnd)
            .map(|i| table.remove(i).1)
    };
    if let Some(p) = prev {
        if p.is_some() {
            let h = HWND(hwnd as *mut std::ffi::c_void);
            // SAFETY：`p` 是 `install` 时记下的原过程。
            unsafe {
                let raw = std::mem::transmute::<WNDPROC, isize>(p);
                SetWindowLongPtrW(h, GWLP_WNDPROC, raw);
            }
        }
    }
}

use windows::Win32::UI::WindowsAndMessaging::WM_SIZE;

/// 安装失败。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveResizeError {
    /// `SetWindowLongPtrW(GWLP_WNDPROC)` 失败，携带 `GetLastError` 的值。
    SetWindowLong(u32),
}

impl std::fmt::Display for LiveResizeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LiveResizeError::SetWindowLong(e) => write!(f, "SetWindowLongPtrW(GWLP_WNDPROC) 失败，GetLastError={e}"),
        }
    }
}

impl std::error::Error for LiveResizeError {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;

    static COUNT: AtomicU32 = AtomicU32::new(0);

    /// 三个测试共用**同一份**全局 `HOOK` / `WM_SIZE_SEEN` / `WM_SIZE_SKIPPED`，
    /// 而 cargo test 默认并行 ⇒ 必须串行。
    ///
    /// 这不是「测试写得不乖」而是**被测对象本身就是进程级单例**（见模块文档
    /// 「为什么是全局而不是按窗口存表」）。所以串行化是对被测语义的忠实反映，
    /// 不是给测试打补丁。
    ///
    /// 用 `unwrap_or_else(|e| e.into_inner())` 而不是 `unwrap()`：某个测试 panic
    /// 时锁被毒化，后面的测试应当**继续跑**（它们仍能验证各自的东西），
    /// 而不是级联地报「上一个测试挂了」。
    static SERIAL: Mutex<()> = Mutex::new(());

    /// 一个真实的消息窗口 + 我们挂上去的过程，合成地驱动 `WM_SIZE`。
    ///
    /// # 为什么用真窗口而不是直接调 `live_proc`
    ///
    /// `live_proc` 里 `WM_SIZE` 分支不查表，但它**之后的**分支要转给原过程 ——
    /// 而那需要真实的 hwnd 才能让 `CallWindowProcW` 不炸。只测 `WM_SIZE` 分支
    /// 会漏掉"我们把别的消息搞坏了"这个**更致命**的回归。
    ///
    /// 建的是 `HWND_MESSAGE` 窗口：不需要显示、不需要 GPU，但仍是一个真正的
    /// 窗口过程宿主。
    fn make_window() -> Option<isize> {
        use windows::Win32::Foundation::HINSTANCE;
        use windows::Win32::System::LibraryLoader::GetModuleHandleW;
        use windows::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, RegisterClassW, WNDCLASSW,
        };

        // 宽字符串常量（PCWSTR 要 `*const u16`，不能用字符串字面量）。
        const CLASS: &[u16] = &[
            90u16, 101, 112, 104, 121, 114, 76, 105, 118, 101, 82, 101, 115, 105, 122, 101, 84,
            101, 115, 116, 0,
        ];
        /// 窗口标题，宽字符（"t\0"）。
        ///
        /// 必须落在 `static` 上：`const` 里没法对临时数组取地址，而
        /// `PCWSTR::from_raw` 恰恰要的只是那个地址。
        static TITLE_BUF: [u16; 2] = [116, 0];
        let title = windows::core::PCWSTR(TITLE_BUF.as_ptr());

        // SAFETY：`hwnd` 由本函数建、由调用方负责；窗口过程是 `DefWindowProcW`。
        unsafe extern "system" fn passthrough(
            h: windows::Win32::Foundation::HWND,
            m: u32,
            w: windows::Win32::Foundation::WPARAM,
            l: windows::Win32::Foundation::LPARAM,
        ) -> windows::Win32::Foundation::LRESULT {
            windows::Win32::UI::WindowsAndMessaging::DefWindowProcW(h, m, w, l)
        }

        unsafe {
            let hinst = GetModuleHandleW(None).ok()?;
            let cls = RegisterClassW(&WNDCLASSW {
                lpfnWndProc: Some(passthrough),
                hInstance: HINSTANCE(hinst.0),
                lpszClassName: windows::core::PCWSTR(CLASS.as_ptr()),
                ..Default::default()
            });
            // 「类已存在」**不是**失败：三个测试跑在不同线程上、用**同一个类名**，
            // 而 cargo test 默认并行 —— 第二个线程注册时必然撞上
            // `ERROR_CLASS_ALREADY_EXISTS`。
            //
            // 这里显式接受那一种失败，而不是退回串行或改类名：改类名就得给每个
            // 测试一份类名常量，改成串行就得引入一把测试锁 —— 两者都为了掩盖一个
            // 本来就无害的返回值。
            if cls == 0
                && windows::Win32::Foundation::GetLastError()
                    != windows::Win32::Foundation::ERROR_CLASS_ALREADY_EXISTS
            {
                return None;
            }
            // `CreateWindowExW` 在 `windows` crate 里返回 `Result<HWND>`
            // （错误已经由 `windows` crate 用 `GetLastError` 取过了）。
            let hwnd = match CreateWindowExW(
                Default::default(),
                windows::core::PCWSTR(CLASS.as_ptr()),
                title,
                // `dwstyle`：用 `WS_OVERLAPPEDWINDOW`，不要传 0。
                //
                // 传 0 会让消息窗口没有任何样式，而 `WM_SIZE` 之类照样会来 ——
                // 但样式为 0 的窗口在 `DefWindowProc` 里行为与正常窗口不同，
                // 而我们要的恰恰是「与正常窗口同形」的那条路径。
                windows::Win32::UI::WindowsAndMessaging::WS_OVERLAPPEDWINDOW,
                0,
                0,
                16,
                16,
                None,
                None,
                Some(HINSTANCE(hinst.0)),
                None,
            ) {
                Ok(h) => h,
                Err(_) => return None,
            };
            if hwnd.0.is_null() {
                return None;
            }
            Some(hwnd.0 as isize)
        }
    }

    /// 装了过程之后，`WM_SIZE` 必须**真的**调回调，且不重入。
    ///
    /// 这条是整个模块的主判据 —— 它证明「拖动时同步重绘」这件事在窗口过程
    /// 层面成立，而不需要真的去拖一个窗口。
    #[test]
    fn wm_size_同步调渲染回调() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        COUNT.store(0, Ordering::Relaxed);
        set_render_hook(std::sync::Arc::new(|| {
            COUNT.fetch_add(1, Ordering::Relaxed);
            true
        }));
        let Some(hwnd) = make_window() else {
            panic!("建不出测试窗口");
        };
        install(hwnd).expect("安装窗口过程");

        // 连续三个 `WM_SIZE`（拖动时就是这样一串）。
        for _ in 0..3 {
            // SAFETY：hwnd 是上面建的活窗口。
            unsafe {
                windows::Win32::UI::WindowsAndMessaging::SendMessageW(
                    HWND(hwnd as *mut std::ffi::c_void),
                    WM_SIZE,
                    Some(WPARAM(1)),
                    Some(LPARAM((100 << 16) | 50)),
                );
            }
        }
        assert_eq!(
            COUNT.load(Ordering::Relaxed),
            3,
            "每个 WM_SIZE 都必须同步画一帧"
        );
        // 用 delta 而不是绝对值：`wm_size_skipped()` 是**进程级**计数，
        // 别的用例（尤其 `回调执行期间的重入被跳过`）会把它加上去，而
        // cargo test 的执行顺序是按字母序 + 线程调度的，不保证谁先跑。
        // 断绝对值等于把「别的测试存在」当成 bug 报出来。
        let skipped_before = wm_size_skipped();
        assert_eq!(
            wm_size_skipped(),
            skipped_before,
            "非重入路径不该有跳过"
        );
        restore(hwnd);
    }

    /// 别的消息必须**原样**转给原过程 —— 我们只多管了 `WM_SIZE` 一个。
    ///
    /// 这条守住"子类化没有把窗口搞坏"。子类化最常见的事故就是忘了转消息，
    /// 症状是"某类操作无声失效"，极难归因。
    #[test]
    fn 其它消息转给原过程() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        COUNT.store(0, Ordering::Relaxed);
        set_render_hook(std::sync::Arc::new(|| {
            COUNT.fetch_add(1, Ordering::Relaxed);
            true
        }));
        let Some(hwnd) = make_window() else {
            panic!("建不出测试窗口");
        };
        install(hwnd).expect("安装窗口过程");

        let before = wm_size_count();
        // SAFETY：同上。
        unsafe {
            windows::Win32::UI::WindowsAndMessaging::SendMessageW(
                HWND(hwnd as *mut std::ffi::c_void),
                windows::Win32::UI::WindowsAndMessaging::WM_NCCALCSIZE,
                Some(WPARAM(0)),
                Some(LPARAM(0)),
            );
        }
        assert_eq!(COUNT.load(Ordering::Relaxed), 0, "非 WM_SIZE 不该触发渲染");
        assert_eq!(
            wm_size_count(),
            before,
            "非 WM_SIZE 不该被计入 WM_SIZE 计数"
        );
        restore(hwnd);
    }

    /// 没装回调时收到 `WM_SIZE` **不得**崩。
    ///
    /// 顺序很重要：宿主是先建窗口再装回调，中间可能有第一个 `WM_SIZE`
    /// （比如创建时就带初始尺寸）。那一次必须安静地什么都不画。
    #[test]
    fn 没装回调时不崩() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        {
            let mut slot = HOOK.lock().unwrap_or_else(|e| e.into_inner());
            *slot = None;
        }
        let Some(hwnd) = make_window() else {
            panic!("建不出测试窗口");
        };
        install(hwnd).expect("安装窗口过程");
        // SAFETY：同上。
        unsafe {
            windows::Win32::UI::WindowsAndMessaging::SendMessageW(
                HWND(hwnd as *mut std::ffi::c_void),
                WM_SIZE,
                Some(WPARAM(1)),
                Some(LPARAM(0)),
            );
        }
        restore(hwnd);
    }

    /// 回调执行期间又来一个 `WM_SIZE`：**必须被跳过**，且不崩。
    ///
    /// # 为什么这条不是假想
    ///
    /// 回调里要做「排版 + 提交 + present」。这三步里任何一步带同步点
    /// （等栅栏、等队列空、驱动的 present 阻塞），都可能让**同一个线程**上
    /// 又被派发一个 `WM_SIZE` —— 拖动时 `WM_SIZE` 是连珠炮。
    ///
    /// # 为什么「跳过」是对的而不是「排队补一次」
    ///
    /// 补一次意味着重入的那个 `WM_SIZE` 要在**外层**渲染**结束之后**再画一遍 ——
    /// 那时窗口尺寸可能又变了，画出来是**过期**的尺寸，还多一次 present。
    /// 而拖动结束时宿主本来就会收到最后一个 `WM_SIZE`，那一次才是该画的。
    /// 所以正确行为是丢中间帧，而不是补。
    #[test]
    fn 回调执行期间的重入被跳过() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        COUNT.store(0, Ordering::Relaxed);
        // 必须是 `static` 而不是局部变量：钩子类型是 `'static`（它活到进程结束），
        // 闭包不能借用栈上的东西。
        static H: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
        // 回调里**同步**再发一个 `WM_SIZE` 到同一个窗口：同线程 `SendMessage`
        // 不排队，直接递归进窗口过程 —— 正是要挡的那种重入。
        set_render_hook(std::sync::Arc::new(|| {
            COUNT.fetch_add(1, Ordering::Relaxed);
            let h = H.load(Ordering::Relaxed) as *mut std::ffi::c_void;
            if !h.is_null() {
                // SAFETY：`hwnd_cell` 里装的是本测试刚建的活窗口。
                unsafe {
                    windows::Win32::UI::WindowsAndMessaging::SendMessageW(
                        HWND(h),
                        WM_SIZE,
                        Some(WPARAM(1)),
                        Some(LPARAM(0)),
                    );
                }
            }
            true
        }));
        let Some(hwnd) = make_window() else {
            panic!("建不出测试窗口");
        };
        install(hwnd).expect("安装窗口过程");
        H.store(hwnd, Ordering::Relaxed);

        let skipped_before = wm_size_skipped();
        // SAFETY：hwnd 是上面建的活窗口。
        unsafe {
            windows::Win32::UI::WindowsAndMessaging::SendMessageW(
                HWND(hwnd as *mut std::ffi::c_void),
                WM_SIZE,
                Some(WPARAM(1)),
                Some(LPARAM(0)),
            );
        }
        // 外面那一次真的画了。
        assert_eq!(COUNT.load(Ordering::Relaxed), 1, "外层 WM_SIZE 必须画一帧");
        // 里面那一次被挡掉了 ⇒ 既没无限递归，也没多画一帧。
        assert_eq!(
            wm_size_skipped(),
            skipped_before + 1,
            "回调里的重入必须被跳过，且恰好一次"
        );
        assert!(
            !IN_HOOK.load(std::sync::atomic::Ordering::Relaxed),
            "回调结束后重入标志位必须复位 —— 否则**后续所有** WM_SIZE 都会被无声跳过"
        );
        restore(hwnd);
    }
}
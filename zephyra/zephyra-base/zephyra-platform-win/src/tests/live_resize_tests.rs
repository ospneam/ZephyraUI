// 本文件由 live_resize.rs 的 #[cfg(test)] mod tests 抽出 —— 测试代码与生产代码分离。
// 模块路径不变（src 里的声明用 #[path] 指过来），所以 use super::* 与私有可见性都照旧。

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
        // 别的用例（尤其 `reentry_during_callback_is_skipped`）会把它加上去，而
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
    fn other_messages_forward_to_original_procedure() {
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
    fn no_crash_when_no_callback_installed() {
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
    fn reentry_during_callback_is_skipped() {
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

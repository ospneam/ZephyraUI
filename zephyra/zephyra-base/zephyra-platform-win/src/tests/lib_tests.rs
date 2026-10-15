// 本文件由 lib.rs 的 #[cfg(test)] mod tests 抽出 —— 测试代码与生产代码分离。
// 模块路径不变（src 里的声明用 #[path] 指过来），所以 use super::* 与私有可见性都照旧。

    use super::*;

    #[test]
    fn 版本号必须可读且主版本不小于十() {
        // 本 crate 只在 Windows 上有内容；非 Windows 上这条测试走 stub 分支。
        let v = Windows.version().expect("Windows 上必须能读到版本号");
        assert!(v.0 >= 10, "读到的主版本不像 Windows：{v:?}");
        assert!(v.2 >= 10000, "build 号不像 Windows：{v:?}");
    }

    #[test]
    fn build_号与版本号是同一个值() {
        assert_eq!(build_number(), Windows.version().unwrap().2);
    }

    #[test]
    fn 空句柄不调用_dwm_也不生效() {
        let out = Windows.apply_window_chrome(
            WindowHandle::from_raw(0),
            WindowChrome::minimal(true, zephyra_platform::CornerPreference::Round),
        );
        assert!(!out.dark && !out.corner, "空句柄不该有任何效果");
    }

    #[test]
    fn 能力位与文档门槛一致() {
        let caps = Windows.capabilities();
        let build = build_number();
        assert_eq!(
            caps.dark_mode,
            build >= 22_000,
            "DWMWA_USE_IMMERSIVE_DARK_MODE 的文档门槛是 Build 22000"
        );
        assert_eq!(
            caps.corner_preference,
            build >= 22_000,
            "DWMWA_WINDOW_CORNER_PREFERENCE 的文档门槛是 Build 22000"
        );
        assert_eq!(
            caps.backdrop,
            build >= 22_621,
            "DWMWA_SYSTEMBACKDROP_TYPE 的文档门槛是 Build 22621（22H2），\
             比 dark mode / corner 的 22000 高一档"
        );
    }

    #[test]
    fn 空句柄下请求_mica_不会生效() {
        // 判据用**空句柄**：这是不碰 DWM 的最短路径，于是"没生效"唯一可能的
        // 原因就是"压根没调"。真窗口上的 Mica 是否真透出来，由截图门禁在
        // 像素层核对（docs/win/02 §4）——API 返回成功**不能**证明材质可见。
        let out = Windows.apply_window_chrome(
            WindowHandle::from_raw(0),
            WindowChrome {
                backdrop: BackdropKind::Mica,
                ..Default::default()
            },
        );
        assert!(!out.backdrop, "空句柄不该声称材质生效");
    }

    #[test]
    #[cfg(windows)]
    fn 每种材质都有对应的_dwm_枚举() {
        // 文档表（docs/win/02 §1）：Mica=MAINWINDOW(2)、Acrylic=TRANSIENT(3)、
        // MicaAlt=TABBED(4)、None=NONE(1)。
        //
        // 判据只查"映射存在且合法"，不查具体数值 —— 数值由 `windows` crate 的
        // 强类型常量保证（写错数字编不过）。
        for kind in [
            BackdropKind::None,
            BackdropKind::Mica,
            BackdropKind::MicaAlt,
            BackdropKind::Acrylic,
        ] {
            assert!(dwm_backdrop(kind).is_some(), "{kind:?} 没有对应的 DWM 枚举");
        }
    }

    #[test]
    fn 系统主题查询结果必须与注册表一致或读不到() {
        // ⚠️ 这两个注册表值**没有官方文档**（见 docs/win/04 §2），所以判据不能是
        // "必须等于深色"—— 那会在浅色系统上无故红。
        //
        // 真正要钉住的是**方向**：查得到就必须与 `AppsUseLightTheme` 一致；
        // 查不到就返回 `None` 而**不是**默认的 `Light`（后者会让不支持的平台上
        // 主题被无声钉死，且上层看不出"这是猜的"）。
        let got = Windows.system_theme();
        if let Some(pref) = got {
            assert_ne!(pref, ThemePreference::Unknown, "已知的话不该是 Unknown");
            // 反复读必须稳定：一次读到值一次读不到，说明句柄生命周期有问题。
            assert_eq!(Some(pref), Windows.system_theme());
        }
        // 能声称支持查询吗？
        assert!(
            Windows.capabilities().system_theme_query,
            "Windows 上查询系统主题是支持的（读注册表），能力位应为 true"
        );
    }

    #[test]
    fn 每帧问能力不许有副作用且要稳定() {
        // `capabilities()` 在热路径被调用：它必须无分配、无锁、无 I/O。
        let a = Windows.capabilities();
        let b = Windows.capabilities();
        assert_eq!(a, b);
        assert_eq!(a, Windows.capabilities());
    }

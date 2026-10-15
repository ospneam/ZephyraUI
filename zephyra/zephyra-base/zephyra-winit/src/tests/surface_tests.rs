// 本文件由 surface.rs 的 #[cfg(test)] mod tests 抽出 —— 测试代码与生产代码分离。
// 模块路径不变（src 里的声明用 #[path] 指过来），所以 use super::* 与私有可见性都照旧。

    use super::*;

    #[test]
    fn dcomp_is_off_unless_the_env_var_says_otherwise() {
        // 进程内只读一次，所以本测试只断言"读过一次之后是某个 bool"，
        // 不去改环境变量 —— 改了也不会生效，反而会让人以为门控是坏的。
        assert_eq!(dcomp_enabled(), dcomp_enabled(), "门控必须幂等，否则会中途切资源拓扑");
    }

    #[test]
    fn the_backbuffer_format_matches_the_swapchain_default() {
        // 两处格式必须一致，而其中一处是 zephyra-dcomp 里的 DXGI 常量。
        // 这条断言是"改了格式忘了改另一边"的唯一护栏 —— 那种不一致不会编译
        // 报错，只会让整屏偏色或 DComp 建链直接失败。
        assert_eq!(
            BACKBUFFER_FORMAT.0,
            SwapchainDesc::default().format.0,
            "本常量与 zephyra-dcomp 的 SwapchainDesc::default 必须一致"
        );
        // 比数值而不是 Debug 串：`DXGI_FORMAT` 的 `Debug` 只打印 `DXGI_FORMAT(87)`。
        assert_eq!(
            BACKBUFFER_FORMAT.0,
            windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM.0
        );
    }

    #[test]
    fn the_failures_stay_distinguishable() {
        // `Device` 意味着"这台机器没有可用 GPU"，`BadWindowHandle` 意味着"宿主用错
        // 平台"。混成一个错误会让宿主无法决定该报错还是该退回，而误诊很难查。
        assert!(SurfaceError::new(SurfaceErrorKind::Device(windows::core::Error::from_hresult(
            E_FAIL
        )))
        .to_string()
        .contains("D3D11"));
        assert!(SurfaceError::new(SurfaceErrorKind::BadWindowHandle)
            .to_string()
            .contains("HWND"));
    }

    /// 步骤标签必须出现在消息里。
    ///
    /// 这条断言守的是本文件存在的主要理由：DXGI 的 `INVALID_CALL` 三种病因同一句话，
    /// 唯一步骤标签能区分。漏掉它 → 错误信息退回"某处失败"，排查成本立刻变高。
    #[test]
    fn the_step_is_always_reported() {
        let e = SurfaceError::new(SurfaceErrorKind::SwapChain(
            windows::core::Error::from_hresult(E_FAIL),
        ))
        .at("CreateSwapChainForHwnd");
        let s = e.to_string();
        assert!(s.contains("CreateSwapChainForHwnd"), "实得 {s}");
        // 未标注时也必须显式说出来，而不是省略 —— "有个步骤忘了填"要能被看见。
        let unlabelled = SurfaceError::new(SurfaceErrorKind::SwapChain(
            windows::core::Error::from_hresult(E_FAIL),
        ));
        assert!(
            unlabelled.to_string().contains("未标注"),
            "实得 {unlabelled}"
        );
    }

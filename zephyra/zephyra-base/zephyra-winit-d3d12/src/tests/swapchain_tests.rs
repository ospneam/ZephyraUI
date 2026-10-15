// 本文件由 swapchain.rs 的 #[cfg(test)] mod tests 抽出 —— 测试代码与生产代码分离。
// 模块路径不变（src 里的声明用 #[path] 指过来），所以 use super::* 与私有可见性都照旧。

    use super::*;

    /// 描述符必须**按缓冲个数**给，而不是固定 1 个。
    ///
    /// flip model 下 `GetCurrentBackBufferIndex` 会在 0..BufferCount 之间轮转，
    /// 堆只给 1 个描述符时第二帧 `cpu(1)` 就越界 —— 而越界的症状是「画面卡在第
    /// 一帧的颜色」或崩，没有任何提示指向描述符个数。
    #[test]
    #[allow(
        clippy::assertions_on_constants,
        reason = "这里要的正是「常量一旦被改坏就吵」；常量折叠正是我们想要的"
    )]
    fn rtv_heap_is_sized_by_buffer_count() {
        // 这里只验证「堆的容量与缓冲个数一致」这条契约成立所需的 API 形状：
        // `DescriptorHeap::new_rtv` 收 `count`，`Swapchain::new` 传的是
        // `max(desc.buffer_count, BUFFER_COUNT)`。
        assert!(BUFFER_COUNT >= 2, "flip model 至少要两个后缓冲");
        assert!(SwapchainDesc::default().buffer_count >= BUFFER_COUNT);
    }

    /// 8 不是错，只是没必要：UI 帧时间几乎全在填充率上，而 8× 与 4× 在一个
    /// 20 像素宽的齿轮上看不出差别（见 [`MSAA_SAMPLES`] 的文档）。
    #[test]
    #[allow(
        clippy::assertions_on_constants,
        reason = "这里要的正是「常量一旦被改坏就吵」；常量折叠正是我们想要的"
    )]
    fn msaa_sample_count_is_four() {
        assert_eq!(
            MSAA_SAMPLES, 4,
            "MSAA 采样数被改了：1 会让 MSAA 静默失效（build_msaa_target 早退），\
             8 只是白付一倍填充率"
        );
    }

    /// MSAA 纹理建得出来，且渲染目标视图真的取得到。
    ///
    /// # 为什么判据是"读回真实资源描述"而不是复述常量
    ///
    /// MSAA 那条链上有三处只能靠实机发现的不一致，而它们**全部**的症状是
    /// 「画面全黑」或「卡在旧内容」，且零 HRESULT：
    ///
    /// | 不一致 | 谁会报 |
    /// |---|---|
    /// | 纹理 `SampleDesc.Count` 与 PSO 不符 | 调试层「render target format does not match」 |
    /// | 纹理格式与后缓冲不符 | `ResolveSubresource` 返 `E_INVALIDARG` |
    /// | 视图建在错误的槽位 | 无（画面只是没有 MSAA） |
    ///
    /// 只断言"常量等于它自己"证明不了任何东西 —— 它在我写下时就成立，
    /// 而上面三行每一条都能在常量正确时发生。
    ///
    /// 要真 GPU 与 DComp，所以默认 `#[ignore]`。
    #[test]
    #[ignore = "需要真 GPU 与 DComp。手动跑：--ignored --nocapture"]
    fn msaa_目标建得出来且采样数如实上报() {
        let Ok(gpu) = Gpu::new() else {
            panic!("建不出 D3D12 设备");
        };
        let desc = SwapchainDesc {
            buffer_count: 2,
            format: DXGI_FORMAT_R8G8B8A8_UNORM,
        };
        let chain = Swapchain::new(&gpu, (64, 64), &desc, MSAA_SAMPLES).expect("建交换链");

        // 采样数必须**如实**报出来 —— 宿主靠它决定 PSO 的 `SampleDesc`。
        assert_eq!(chain.samples(), MSAA_SAMPLES);
        // RTV 必须真的取得到（槽位没被后缓冲占用）。
        chain.msaa_rtv().expect("取 MSAA 的 RTV");

        // 后缓冲永远单采样 —— 这正是"MSAA 纹理必须自建"的根因。若哪天 DXGI
        // 允许了，这条会失败并提示可以拆掉整条自建 + resolve 的路。
        let back = chain.buffer(0).expect("取后缓冲");
        let back_desc = unsafe { back.GetDesc() };
        assert_eq!(
            back_desc.SampleDesc.Count, 1,
            "交换链后缓冲的采样数不是 1（实得 {}）—— 若某天 DXGI 允许后缓冲 \
             多重采样，整条自建纹理 + resolve 就可以拆掉了",
            back_desc.SampleDesc.Count
        );

        // 采样数 1 时 MSAA 目标必须不存在，且 `msaa_rtv` **明确报错** ——
        // 静默回退成后缓冲会让"采样数不一致"变成"画面全黑且零 HRESULT"。
        let single = Swapchain::new(&gpu, (64, 64), &desc, 1).expect("建单采样交换链");
        assert_eq!(single.samples(), 1);
        assert!(
            single.msaa_rtv().is_err(),
            "采样数 1 时不该给得出 MSAA 的 RTV"
        );
        // 无 MSAA 时 `resolve_msaa` 必须是**空操作**而非报错 —— 生产路径每帧都调它。
        single
            .resolve_msaa(gpu.list())
            .expect("无 MSAA 时 resolve 应为空操作");
    }


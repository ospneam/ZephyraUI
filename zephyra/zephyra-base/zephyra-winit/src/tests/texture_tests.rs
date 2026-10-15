// 本文件由 texture.rs 的 #[cfg(test)] mod tests 抽出 —— 测试代码与生产代码分离。
// 模块路径不变（src 里的声明用 #[path] 指过来），所以 use super::* 与私有可见性都照旧。

    use super::*;

    /// 建一个测试用设备。失败就跳过 —— 这是 GPU 相关的 crate，没有设备的机器上
    /// 本来就测不了。
    fn test_device() -> Option<(ID3D11Device, ID3D11DeviceContext)> {
        use windows::Win32::Graphics::Direct3D::{
            D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0,
        };
        use windows::Win32::Graphics::Direct3D11::{
            D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION,
        };
        let mut dev = None;
        let mut ctx = None;
        unsafe {
            windows::Win32::Graphics::Direct3D11::D3D11CreateDevice::<
                Option<&windows::Win32::Graphics::Dxgi::IDXGIAdapter>,
            >(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                windows::Win32::Foundation::HMODULE(std::ptr::null_mut()),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&[D3D_FEATURE_LEVEL_11_0]),
                D3D11_SDK_VERSION,
                Some(&mut dev as *mut _),
                None,
                Some(&mut ctx as *mut _),
            )
        }
        .ok()?;
        Some((dev?, ctx?))
    }

    /// 像素不足必须**报错**，不能建成一张内容是垃圾的纹理。
    ///
    /// 这条不是洁癖：`create_rgba_texture` 的写入循环按 `width * height * 4` 从
    /// `pixels` 里取，越界读到的字节会被原样上传 —— 症状是一张底部带随机色块的
    /// 图，**没有任何错误**。
    #[test]
    fn insufficient_pixels_must_error_not_build_junk_texture() {
        let Some((device, context)) = test_device() else {
            eprintln!("跳过：建不出 D3D11 设备");
            return;
        };
        let err = create_rgba_texture(&device, &context, &[0u8; 16], 4, 4);
        assert!(
            err.is_err(),
            "4×4 RGBA 需要 64 字节，给 16 字节必须报错 —— \
             否则越界读到的字节会被原样上传成随机色块，且不报任何错"
        );
    }

    /// 完整走一遍：建纹理 → 建 SRV → 登记 → 查 → 注销。
    #[test]
    fn register_and_query() {
        let Some((device, context)) = test_device() else {
            eprintln!("跳过：建不出 D3D11 设备");
            return;
        };
        let w = 4u32;
        let h = 4u32;
        let pixels = vec![200u8; (w * h * 4) as usize];

        let mut table = TextureTable::new();
        assert!(table.is_empty());
        assert!(!table.contains("a.png"), "未登记的名字必须查不到");

        let tex = create_rgba_texture(&device, &context, &pixels, w, h).expect("建纹理");
        let srv = create_srv(&device, &tex).expect("建 SRV");
        table.insert("a.png", srv, tex);

        assert!(table.contains("a.png"));
        assert_eq!(table.len(), 1);
        assert!(table.srv("a.png").is_some(), "登记过的必须能取回 SRV");
        assert!(table.srv("b.png").is_none(), "没登记的不该取到 SRV");

        table.remove("a.png");
        assert!(!table.contains("a.png"));
        assert_eq!(table.len(), 0);
    }

    /// `update` 对未登记的名字必须返回 `false` 且**不碰任何东西**。
    ///
    /// 它取 `&self` 而不是 `&mut self`，所以「更新一个不存在的纹理」在类型上就是
    /// 可能的 —— 忘了判空的话症状是「视频帧偶尔不刷新」，且不报错。
    #[test]
    fn update_未登记的名字_返回_false() {
        let Some((device, context)) = test_device() else {
            eprintln!("跳过：建不出 D3D11 设备");
            return;
        };
        let table = TextureTable::new();
        assert!(
            !table.update(&context, "missing.png", &[0u8; 64], 4, 4),
            "未登记的纹理必须返回 false，调用方据此自己重建"
        );
        let _ = device;
    }

    /// `update` 像素不足时必须返回 `false`，而不是写一半。
    #[test]
    fn update_像素不足_返回_false() {
        let Some((device, context)) = test_device() else {
            eprintln!("跳过：建不出 D3D11 设备");
            return;
        };
        let w = 4u32;
        let h = 4u32;
        let mut table = TextureTable::new();
        let tex = create_rgba_texture(&device, &context, &vec![0u8; (w * h * 4) as usize], w, h)
            .expect("建纹理");
        let srv = create_srv(&device, &tex).expect("建 SRV");
        table.insert("a.png", srv, tex);

        assert!(
            !table.update(&context, "a.png", &[0u8; 16], w, h),
            "4×4 需要 64 字节，给 16 字节必须返回 false"
        );
        assert!(
            table.update(&context, "a.png", &vec![1u8; (w * h * 4) as usize], w, h),
            "字节数够时必须成功"
        );
    }

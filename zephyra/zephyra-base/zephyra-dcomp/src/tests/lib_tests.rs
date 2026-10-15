// 本文件由 lib.rs 的 #[cfg(test)] mod tests 抽出 —— 测试代码与生产代码分离。
// 模块路径不变（src 里的声明用 #[path] 指过来），所以 use super::* 与私有可见性都照旧。

use super::*;

/// swapchain 描述的每个字段都是 P0 实测 / Chrome 对照确认过的，**不是默认值**。
///
/// 之所以值得钉住：`SwapEffect` 从 `DISCARD` 换成 `FLIP_SEQUENTIAL` 不会编译
/// 报错、不会 panic，只会让合成路径退化甚至黑屏；`AlphaMode` 漏设 `PREMULTIPLIED`
/// 则会让半透明内容整体发黑。这类字段只能靠断言守住。
#[test]
fn swapchain_desc_fields_match_the_verified_p0_conclusion() {
    let d = base_swapchain_desc(1920, 1080, SwapchainDesc::default().format, 2);

    assert_eq!(d.BufferUsage, DXGI_USAGE_RENDER_TARGET_OUTPUT);
    assert_eq!(d.BufferCount, 2);
    assert_eq!(d.SwapEffect, DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL);
    assert_eq!(d.AlphaMode, DXGI_ALPHA_MODE_PREMULTIPLIED);
    assert_eq!(d.Scaling, DXGI_SCALING_STRETCH);
    assert_eq!(d.Flags, 0);
    assert_eq!(d.SampleDesc.Count, 1);
    assert_eq!(d.SampleDesc.Quality, 0);
    assert!(!d.Stereo.as_bool());
    assert_eq!(d.Width, 1920);
    assert_eq!(d.Height, 1080);
}

/// 默认描述就是 B8G8R8A8 双缓冲 —— 这是"现成的可用配置"，改动它等于让
/// 所有宿主静默换掉呈现格式。
#[test]
fn default_swapchain_desc_is_double_buffered_bgra8() {
    let d = SwapchainDesc::default();
    assert_eq!(d.buffer_count, 2);
    assert_eq!(
        d.format,
        windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM
    );
}

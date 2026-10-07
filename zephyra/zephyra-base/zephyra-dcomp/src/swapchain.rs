//! composition swapchain 创建（DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL + PREMULTIPLIED）。

use windows::core::IUnknown;
use windows::Win32::Graphics::Dxgi::IDXGISwapChain1;

use crate::{base_swapchain_desc, create_factory, SwapchainDesc};

/// 创建 composition swapchain。
///
/// * `device`：D3D12 command queue 或 D3D11 device 的 `IUnknown`。
///
///   P0 结论：本机 D3D12 queue 路线 DComp 设备创建失败（`E_NOINTERFACE`），但
///   **composition swapchain 本身可以挂在 D3D12 queue 上**
///   （`CreateSwapChainForComposition` 的 `pDevice` 接受 queue 的 `IUnknown`）
///   —— D3D12 宿主走此路线并配 [`crate::DcompCompositor::new_from_rendering_device`]；
///   D3D11 路线传 device 即可。
/// * `size`：与窗口客户区**物理像素**一致（由调用方负责 DPI 换算）。不一致时
///   DComp 会缩放采样，画面整体发虚 —— 图标最先暴露这个问题。
pub fn create_composition_swapchain(
    device: &IUnknown,
    size: (u32, u32),
    desc: &SwapchainDesc,
) -> windows::core::Result<IDXGISwapChain1> {
    let factory = create_factory()?;
    let dxgi_desc = base_swapchain_desc(size.0, size.1, desc.format, desc.buffer_count);
    unsafe { factory.CreateSwapChainForComposition(device, &dxgi_desc, None) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 零尺寸不是合法请求，但也不能让它 panic —— `width.max(1)` 的钳制放在
    /// 调用方（surface 构造）是有意的：这里断言"本函数不隐式改写尺寸"，
    /// 也就是钳制责任有唯一归属处，不会出现两处各钳一次导致 resize 判定漂移。
    #[test]
    fn zero_sized_request_is_passed_through_unchanged() {
        // 不建真实 swapchain（需要设备与工厂），只验证描述组装这一段。
        let d = base_swapchain_desc(0, 0, SwapchainDesc::default().format, 2);
        assert_eq!(d.Width, 0);
        assert_eq!(d.Height, 0);
    }
}

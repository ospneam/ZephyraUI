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
#[path = "tests/swapchain_tests.rs"]
mod tests;

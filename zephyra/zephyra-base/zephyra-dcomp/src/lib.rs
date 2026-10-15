//! `zephyra-dcomp`：DirectComposition 呈现后端。
//!
//! 一个窗口的**呈现**这件事由本 crate 负责：把渲染好的后缓冲挂进 DComp visual 树，
//! 并把它交给系统合成器上屏。框架的其余部分（布局、绘制、命中）与它无关 ——
//! 核心只产出 `zephyra_core::paint::DisplayList`，本 crate 只负责让它出现在屏幕上。
//!
//! （此处刻意不写成 intra-doc 链接：按上文"本 crate 不依赖 `zephyra-core`"的理由，
//! `zephyra-core` 不在本 crate 的依赖图里，链接无法解析。）
//!
//! # 三条硬约束（违反即窗口全白 / AV，均已实测）
//!
//! 这三条不是文档洁癖，是本机踩过的坑。**改动本 crate 前先读这一节。**
//!
//! ## 1. target / visual 的 COM 引用必须保活到窗口销毁
//!
//! `IDCompositionTarget` 一旦被释放，DComp target 对象即销毁，visual 树与 HWND
//! 的绑定被拆除 —— 现象是**窗口全白**，且不报任何错。
//!
//! 因此 [`DcompCompositor`] 把 `device` / `target` / `root` 全部作为字段持有，
//! 并在 `Drop` 中显式解绑。宿主侧的容器（如持有 `IDXGISwapChain1` 的 surface）
//! 也**必须**让 `DcompCompositor` 比它活得久 —— 顺序反了同样全白。
//!
//! ## 2. DComp 设备用 IDXGIDevice 路线创建
//!
//! 本机 D3D12 command queue 路线 `E_NOINTERFACE`（v2 / v3 均如此）。IDXGIDevice
//! 与 null rendering device 实测均可合成。
//!
//! 所以 [`DcompCompositor::new_from_rendering_device`] 走 **v2 → v3 → null** 的
//! fallback 链，而不是只试一次就失败。null 兜底能成功这件事本身就是反直觉的，
//! 记在这里以免后来者"简化"掉最后一级。
//!
//! ## 3. `Present1` 必须传默认 `DXGI_PRESENT_PARAMETERS`
//!
//! 传 `nullptr` 直接 AV。必须走本 crate 的 [`present_one`]，不要自己调
//! `IDXGISwapChain1::Present1`。
//!
//! # 为什么不放进 `zephyra-core` 核心
//!
//! 核心的零依赖与 `#![forbid(unsafe_code)]` 是本框架**唯一不可让步**的约束
//! （见 `zephyra/zephyra-core/Cargo.toml`）。本 crate 需要 `unsafe`（COM 与 DXGI 全是
//! FFI）且需要 `windows` 依赖，两者都与该约束冲突，所以独立成包。
//!
//! 这也意味着：**核心一行都不需要为呈现改动**。核心产出 `DisplayList`，本 crate
//! 消费它 —— 换呈现后端（DXGI 传统路径 / DComp）核心无感。
//!
//! # 与 Viewstage 的关系
//!
//! 代码自 `Viewstage-dev/crates/viewcomp` 迁入，那边是同一份实现的副本。
//! 两侧并存是有意的：先在本仓把落点与验证做实，再让 Viewstage 切换依赖。

#![cfg(windows)]

use windows::Win32::Graphics::Dxgi::{
    DXGI_PRESENT, DXGI_PRESENT_PARAMETERS, DXGI_SCALING_STRETCH, DXGI_SWAP_CHAIN_DESC1,
    DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL, DXGI_USAGE_RENDER_TARGET_OUTPUT,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_PREMULTIPLIED, DXGI_SAMPLE_DESC,
};

pub mod compositor;
pub mod swapchain;

pub use compositor::{DcompCompositor, VisualSlot};
pub use swapchain::create_composition_swapchain;

/// composition swapchain 参数（P1 默认 B8G8R8A8 双缓冲；P3 加 NV12）。
#[derive(Debug, Clone, Copy)]
pub struct SwapchainDesc {
    pub buffer_count: u32,
    pub format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT,
}

impl Default for SwapchainDesc {
    fn default() -> Self {
        Self {
            buffer_count: 2,
            format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
        }
    }
}

/// DComp 设备创建：IDXGIDevice 路线（Chrome 路线）。
///
/// 调用方用自己的渲染设备 QI 出 `IDXGIDevice` 后传入（D3D11 device 可直接 QI）。
/// P0 实测本机 D3D12 command queue 不能作为渲染设备来源。
///
/// [`DcompCompositor::new_from_rendering_device`] 是本函数的带 fallback 版本 ——
/// 除非你确信拿到的是 IDXGIDevice，否则优先用那个。
pub fn create_device(
    dxgi_device: &windows::Win32::Graphics::Dxgi::IDXGIDevice,
) -> windows::core::Result<windows::Win32::Graphics::DirectComposition::IDCompositionDevice> {
    unsafe {
        windows::Win32::Graphics::DirectComposition::DCompositionCreateDevice2::<
            _,
            windows::Win32::Graphics::DirectComposition::IDCompositionDevice,
        >(dxgi_device)
    }
}

/// composition swapchain 双缓冲默认描述（P0 验证参数）。
pub(crate) fn base_swapchain_desc(
    width: u32,
    height: u32,
    format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT,
    buffer_count: u32,
) -> DXGI_SWAP_CHAIN_DESC1 {
    DXGI_SWAP_CHAIN_DESC1 {
        Width: width,
        Height: height,
        Format: format,
        // `DXGI_SWAP_CHAIN_DESC1::Stereo` 是裸 `BOOL`（不是 0.62 高层 API 那样的
        // `bool`），所以这里必须用 `BOOL(0)`。这个不一致不是笔误 —— 它说明"统一
        // 成 bool"的重构只做到了高层绑定层，结构体字段没跟上。
        Stereo: windows::core::BOOL(0),
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
        BufferCount: buffer_count,
        Scaling: DXGI_SCALING_STRETCH,
        SwapEffect: DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL,
        AlphaMode: DXGI_ALPHA_MODE_PREMULTIPLIED,
        Flags: 0,
    }
}

/// `IDCompositionDevice` 的公共 re-export（调用方类型标注用）。
pub type DcompDevice = windows::Win32::Graphics::DirectComposition::IDCompositionDevice;
pub type DcompTarget = windows::Win32::Graphics::DirectComposition::IDCompositionTarget;
pub type DcompVisual = windows::Win32::Graphics::DirectComposition::IDCompositionVisual;

/// Present 辅助：硬约束 #3 的封装（params 必须非空）。
///
/// # Safety
///
/// swapchain 必须有效。
pub unsafe fn present_one(
    swapchain: &windows::Win32::Graphics::Dxgi::IDXGISwapChain1,
    sync_interval: u32,
) -> windows::core::Result<()> {
    // windows-rs 0.58 的 Present1 第二个参数类型为 `DXGI_PRESENT`（自定义 flag
    // 位域），零值即"不设置任何 present 附加标志"。它必须**非空**：
    // 传 nullptr 会 AV（硬约束 #3）。
    let params = DXGI_PRESENT_PARAMETERS::default();
    swapchain
        .Present1(sync_interval, DXGI_PRESENT(0), &params)
        .ok()
}

/// 工厂创建（swapchain 前置）。
pub(crate) fn create_factory(
) -> windows::core::Result<windows::Win32::Graphics::Dxgi::IDXGIFactory2> {
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory2, DXGI_CREATE_FACTORY_FLAGS};

    // 零 flags：不用调试层（会改变性能特征），不用内部工厂。
    unsafe {
        CreateDXGIFactory2::<windows::Win32::Graphics::Dxgi::IDXGIFactory2>(
            DXGI_CREATE_FACTORY_FLAGS(0),
        )
    }
}

#[cfg(test)]
#[path = "tests/lib_tests.rs"]
mod tests;

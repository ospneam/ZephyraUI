//! ZephyraUI 的 Linux 渲染后端（裸 Vulkan）：窗口 + 渲染器 + 帧循环。
//!
//! # 与 wgpu 版（zephyra-winit-vk）的关系
//!
//! wgpu 版已删除。本 crate 取代它，接口完全兼容（同一个 `HostWindow` trait），
//! demo / gallery / zephyra-ui 的接入代码一字不改。
//!
//! # 模块划分
//!
//! - [`surface`]：Instance / Device / Queue / Swapchain / 读回
//! - [`renderer`]：顶点流 → GPU 一帧
//! - [`host`]：窗口、事件循环、帧调度
//!
//! # 为什么三个渲染后端互不依赖
//!
//! D3D11 / D3D12 / Vulkan 是三套不同的词汇，硬并会让 `#[cfg]` 分支淹没真正的
//! 逻辑，而三个后端各自的「静默不画」类 bug 长得一模一样。后端由**特性开关**选，
//! 同一个二进制只会有一个。

mod host;
mod renderer;
mod surface;

pub use host::HostWindow;
pub use renderer::{RenderCtx, Renderer, TextureTable};
pub use surface::{Gpu, SurfaceError};

//! # zephyra-winit
//!
//! ZephyraUI 的 winit + wgpu 宿主适配层（**占位**）。
//!
//! ## 职责
//!
//! 1. 用 winit 建窗口、收事件，把指针/触摸/键盘/滚轮翻译成核心的输入类型；
//! 2. 用 wgpu 建表面与设备，把 [`zephyra::paint::DisplayList`] 回放成一帧。
//!
//! ## 不做什么
//!
//! - 不做布局、不做命中、不做文本整形之外的任何决策——那些在核心里；
//! - 不缓存字形图集的生存期语义（那是 wgpu 侧的资源管理）。
//!
//! ## 为什么它存在
//!
//! 这是框架的**最终目标宿主**。它与 `zephyra-iced`（过渡期，在 Viewstage 仓）
//! 消费的是同一个 [`zephyra::paint::DisplayList`]，因此核心无需为迁移改动一行。
//! 当 Viewstage 把渲染层收敛到 winit+wgpu，就是切到这个 crate。
//!
//! 当前为骨架阶段，暂无实现。

#![forbid(unsafe_code)]

pub use zephyra;

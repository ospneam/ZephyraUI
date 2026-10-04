#![forbid(unsafe_code)]
//! # ZephyraUI — 保留/声明式、GPU 优先的 UI 框架核心
//!
//! 一个把「状态」降到「像素」的过程拆成可替换的两半的框架：核心负责**决定画什么**，
//! 宿主负责**怎么画**。两者之间只有一个契约：[`paint::DisplayList`]。
//!
//! ## 数据流
//!
//! ```text
//!   Element 树   ──diff──▶  Node 树  ──layout──▶  带几何的 Node 树
//!    (声明)         (保留)      (状态)      (测量/排列)
//!                                                      │
//!                                                      ▼
//!                                               DisplayList
//!                                            (排序后的绘制命令)
//!                                                      │
//!                            ┌─────────────────────────┴──────────────┐
//!                            ▼                                        ▼
//!                    zephyra-iced                          zephyra-winit
//!                 (塞进 iced 的 Scene)              (自己的 wgpu renderer)
//! ```
//!
//! ## 为什么是保留式而不是即时式
//!
//! 即时模式（egui 风格）每帧从代码重建 UI，实现快，但代价是**布局与绘制融合**、
//! 没有可复用的 GPU 纹理缓存、也没有"只有这一小块变脏"的概念。文档批注这类
//! 应用在 GPU 占用率上是第一约束，需要能说清"这一帧重算了什么"。
//!
//! 保留式（GPUI / rgui 风格）让你描述一棵元素树，框架 diff 后生成 DisplayList。
//! 代价是要自建布局、绘制与命中测试——这就是本 crate 存在的原因。
//!
//! ## 模块
//!
//! | 模块 | 职责 | 对应 WinUI 概念 |
//! |---|---|---|
//! | [`core`] | 几何与排版基元（Thickness / CornerRadius / Spacing / Color） | Thickness、CornerRadius |
//! | [`property`] | 属性定义、存储、继承、变更通知、样式化 | DependencyProperty |
//! | [`theme`] | 设计令牌 + Light/Dark + 运行时切换 | ThemeResource、ResourceDictionary |
//! | [`tree`] | Element → Node 的构建与 diff | Visual Tree |
//! | [`layout`] | Measure / Arrange 协议 | Grid、StackPanel |
//! | [`paint`] | Node → DisplayList | **Compositor** |
//! | [`text`] | 文本测量与排版（抽象 trait，不绑定实现） | Typography |
//! | [`hit`] | 命中测试 | — |
//!
//! 控件库（Button / ToggleSwitch / Slider…）与模板系统属于**上层**，建立在
//! [`property`] 与 [`theme`] 之上，尚未实现。
//!
//! ## 当前状态
//!
//! 骨架阶段：模块边界与契约已定，实现待补。`examples/` 里的验收标准是——
//! 能把一棵 Node 树降为 [`paint::DisplayList`] 并断言其内容。

pub mod core;
pub mod hit;
pub mod layout;
pub mod paint;
pub mod property;
pub mod text;
pub mod theme;
pub mod tree;

#[cfg(test)]
mod tests;

/// 框架版本。
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

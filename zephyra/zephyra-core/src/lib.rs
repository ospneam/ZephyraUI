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
//!                    zephyra-宿主                          zephyra-winit
//!                 (塞进 宿主 的 Scene)              (自己的 D3D11 renderer)
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
//! 每个模块只做一件事。同一层内的依赖是单向的：数据先在 [`element`]/[`node`] 里
//! 成形，再被 [`style`] 解读，再被 [`layout`] 摆位，最后由 [`paint`] 降级成命令。
//!
//! | 模块 | 职责 | 对应 WinUI 概念 |
//! |---|---|---|
//! | [`core`] | 几何与排版基元（Thickness / CornerRadius / Spacing / Color） | Thickness、CornerRadius |
//! | [`property`] | 属性定义、存储、继承、变更通知、样式化 | DependencyProperty |
//! | [`theme`] | 设计令牌 + Light/Dark + 运行时切换 | ThemeResource、ResourceDictionary |
//! | [`props`] | 本框架认识的属性键（含各自是否影响布局） | — |
//! | [`id`] | 身份：`NodeTag` / `ElementKey` / `ElementKind` | — |
//! | [`element`] | **声明**：一棵可丢弃的 `Element` 树 | — |
//! | [`node`] | **保留**：跨帧存活的状态 + 脏标记 | Visual Tree |
//! | [`diff`] | 新旧对账，产出 `DiffStats` | — |
//! | [`frame`] | **帧缓存**：跨帧存树与列表，回答「这一帧画什么（含『与上次相同』）」 | — |
//! | [`context`] | 只读上下文（主题 + 可选文本引擎） | — |
//! | [`style`] | 属性表 + 上下文 → 具体值 | — |
//! | [`layout`] | Measure / Arrange 协议**与遍历节点树的两趟驱动** | Grid、StackPanel |
//! | [`paint`] | `DisplayList` **与节点树 → DisplayList 的降级** | **Compositor** |
//! | [`filter`] | 效果描述：`Filter` / `FilterChain` / 29 个 `BlendMode` | ImageFilter、BlendMode |
//! | [`geom`] | 路径几何：展平 / 笔画展开 / 多孔耳切三角化 | PathMeasure、PathOp |
//! | [`text`] | 文本测量与排版（抽象 trait，不绑定实现） | Typography |
//! | [`hit`] | 命中测试 | — |
//! | [`input`] | 手势识别与事件路由 | Pointer / Gesture |
//! | [`icon`] | 路径、SVG 解析与图标注册表 | — |
//!
//! 控件库（Button / ToggleSwitch / Slider…）与模板系统属于**上层**，建立在
//! [`property`] 与 [`theme`] 之上，尚未实现。

pub mod context;
pub mod core;
pub mod diff;
pub mod element;
pub mod filter;
pub mod frame;
pub mod geom;
pub mod hit;
pub mod icon;
pub mod id;
pub mod input;
pub mod layout;
pub mod node;
pub mod paint;
pub mod props;
pub mod property;
pub mod style;
pub mod text;
pub mod theme;

// 测试模块必须 gate 在 `cfg(test)` 下。不加的话它们会编进正式产物，而其中的
// `#[test]` 函数在非测试构建里被剥掉，于是所有 `use` 变成"未使用"——
// 一批纯属噪声、且会淹没真正警告的告警。
#[cfg(test)]
mod frame_tests;
#[cfg(test)]
mod geom_tests;
#[cfg(test)]
mod icon_tests;
#[cfg(test)]
mod input_tests;
#[cfg(test)]
mod layout_tests;
#[cfg(test)]
mod layout_classification_tests;
#[cfg(test)]
mod paint_tests;
#[cfg(test)]
mod tests;

/// 框架版本。
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

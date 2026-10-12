//! # ZephyraUI — core 之上的 UI 控件层
//!
//! 这一层只做一件事：把 [`zephyra_core`] 的属性 / 布局 / 样式机制**组装成控件**。
//! 它对应 WinUI 的"控件库"那一层，也是 `zephyra-core/src/lib.rs` 里
//! 「控件库与模板系统属于上层，尚未实现」中缺失的那一层。
//!
//! # 三条不可协商的约束
//!
//! 1. **源码里没有样式。** 控件只发「分类名」（`.style(..)`）与内容属性
//!    （`TEXT` / `ICON_NAME`），不写颜色、圆角、间距、边框。外观全部在
//!    `ui.zstyle`（控件）与 `gallery.zstyle`（应用版式）里。
//! 2. **状态也是分类名。** hover / pressed / checked 是 `.hovered` /
//!    `.pressed` / `.checked`，由样式表的特异性自动层叠，源码不发值。
//! 3. **几何只有一处例外。** 滑条 / 进度条的填充宽度由**值**决定 —— 那是内容
//!    （像文字一样），不是版式。允许的形态被门禁钉死：只有 key 以
//!    `fill:` 开头的节点可以在源码里写 `WIDTH`，其余一概不许。
//!    （见 `tests/gates.rs` 的 `only_fill_nodes_carry_local_width`。）
//!
//! # 与 RinUI 的关系
//!
//! 视觉与度量参照 `E:\Rin-UI`（Fluent Design-like，Qt Quick）。
//! 这里是**照着它的样子重写**，不是移植：令牌名走 Zephyra 的 `@role` 体系，
//! 交互走保留式的 hover/press/click，布局用 core 的单趟 flex。
//! 对照表在 `README.md`。

pub mod controls;
pub mod app;
pub mod tokens;

pub use controls::*;

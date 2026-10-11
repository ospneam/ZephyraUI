//! 样式注入点。
//!
//! # 为什么需要一个 trait 而不是直接把 `StyleSheet` 传进来
//!
//! `zephyra-core` 是**零依赖**的（`cargo tree -p zephyra-core --depth 1` 只输出自身
//! 一行，这是计划里写明的不变量）。而解析器（`cssparser`）只能住在 dev 工具
//! `zstylec` 里。所以 core **不能**认识 `StyleSheet`。
//!
//! 于是用 trait 拆开：core 只认「有一个东西能在 diff 之后、布局之前改我的节点」，
//! 那个东西长什么样由 `zephyra-style` 实现。
//!
//! # 为什么注入点必须在 diff 之后、布局之前
//!
//! `diff` 在 tag 变化时**整个替换节点**（`*node = Node::from_element(...)`），
//! 替换会把 `Styled` 层一起丢掉。所以：
//!
//! - 排在 diff **之前** → 刚写进去的样式被重建丢掉，那一帧读到空样式层
//! - 排在 layout **之后** → 改尺寸的样式要等下一帧才生效
//!
//! 两个位置都会产出「看起来像偶发」的 bug，所以顺序是**硬契约**，不是偏好。

use crate::node::Node;

/// 能在每帧的固定时机改节点样式的东西。
///
/// # 实现必须幂等
///
/// 它每帧都会被调用，而声明树每帧重建。所以「同样的输入 → 同样的节点状态」
/// 是硬要求：不幂等的话，每帧都在往 `Styled` 层累积。
pub trait Styler {
    /// 在 diff 之后、布局之前应用样式。
    fn apply(&mut self, root: &mut Node);
}

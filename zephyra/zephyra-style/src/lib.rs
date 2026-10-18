#![deny(rustdoc::broken_intra_doc_links)]
//! 声明层：样式表 → 属性值。
//!
//! # 这个 crate 负责什么
//!
//! 一件半事：
//!
//! 1. **完整的那件**：把 `.zstyle`（CSS 语法）解析成 [`ir::StyleSheet`]，
//!    再把这份 IR `apply` 到 `zephyra_core::node::Node` 上。
//! 2. **半件**：`apply` 需要的 [`zephyra_core::property::PropertyKey`] / [`zephyra_core::property::PropertyValue`] 全部来自
//!    `zephyra-core`——本 crate **不定义任何属性**。
//!
//! # 为什么不并进 core
//!
//! `zephyra-core` 是零依赖的（`cargo tree -p zephyra-core --depth 1` 只输出自身
//! 一行，这是计划里写明的不变量）。CSS 解析器迟早要引 crate，所以它必须住在这里。
//!
//! # 两种模式，一份 IR
//!
//! ```text
//! dev:     .zstyle 文本  ──cssparser──▶  StyleSheet(IR) ──apply──▶ Node
//! release: .zstyle 字节 ──反序列化────▶  StyleSheet(IR) ──apply──▶ Node
//!                                       ^^^^^^^^^^^^^^^^^^^
//!                                       两边产出同一个类型
//! ```
//!
//! ⚠️ `apply` 与 IR **只有一份实现**。这是本设计唯一的命门——如果 release 走一条
//! 独立的、更简化的路径（比如烘焙成 style block），那 `apply` 就要写两遍，
//! 而两套规则会漂移：症状是「dev 里对、release 里错」，极难定位。

pub mod anim;
pub mod apply;
pub mod ir;

#[cfg(test)]
mod tests;

pub use apply::SheetStyler;
pub use ir::{Declaration, Selector, StyleRule, StyleSheet};

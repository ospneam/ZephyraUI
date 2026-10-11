//! `zstylec` 的库入口：解析 `.zstyle` 文本。
//!
//! # 为什么既是 bin 又是 lib
//!
//! bin 是命令行工具（构建期把 `.zstyle` 编成二进制）。
//! lib 是给 **dev 模式**用的：宿主要在运行期直接解析文本、换样式表看效果，
//! 不可能每改一次就去敲一次命令行。
//!
//! ```text
//! dev:     .zstyle 文本 ──zstylec::parse()──▶ StyleSheet ──apply──▶ Node
//! release: .zstyle.bin  ──from_bytes()────▶ StyleSheet ──apply──▶ Node
//!                    ^^^^^^^^^^^ 尚未实现（B5）
//! ```
//!
//! ⚠️ 生产二进制链这个 crate 才拿得到 dev 模式。若只想要 release，
//! 只链 `zephyra-style` 即可 —— 它不含 `cssparser`。

mod errors;
mod parse;
mod props_map;
mod value;

pub use parse::{parse, ParseResult};

#[cfg(test)]
mod tests;

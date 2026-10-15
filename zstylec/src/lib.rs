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

mod check;
mod errors;
mod parse;
mod props_map;
mod value;

pub use check::{check, check_file, check_text, has_errors, Diag, FileReport, Level};
pub use parse::{parse, ParseResult};

/// 全部受支持的 CSS 属性名（`zstylec props` 的数据源）。
///
/// 为什么暴露成公开 API：CLI 要列它们，而「CLI 与编译器看到的属性表不一致」正是
/// 那种「改了 A 结果 B 不变」���坑 —— 走同一个函数就没有第二条路径。
pub fn supported_props() -> Vec<&'static str> {
    props_map::all_css_names().collect()
}

#[cfg(test)]
mod tests;

//! `props` —— 列受支持的样式表属性。
//!
//! 走 `zstylec::supported_props()`，与 `zephyra check` / `zstylec props` **同一个
//! 函数**。三条路径给三份不同的属性表，是那种「改了 A、B 不变」的经典坑。

use super::{CliError, Write};
use crate::args::Args;
use crate::ExitCode;

pub fn run(a: &Args, w: &mut dyn Write) -> Result<ExitCode, CliError> {
    let mut names: Vec<&'static str> = zstylec::supported_props();
    names.sort_unstable();

    // 有筛选词就过滤 —— 属性一多，列全表不如查。
    let filter = a.positional_at(0);
    for n in names {
        if let Some(f) = filter {
            if !n.contains(f) {
                continue;
            }
        }
        w.line(n);
    }
    Ok(ExitCode::Success)
}

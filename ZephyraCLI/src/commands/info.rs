//! `info` —— 报版本与工具链状态。
//!
//! # 为什么需要它
//!
//! 出问题时第一句要问的永远是「你装的什么版本」。而且要报**路径**：同一个
//! `zstylec` 在 PATH 上和本地 workspace 里可能是两份，报不出路径就分不清是哪份在
//! 跑 —— 而那正是「CLI 说没错、编译说���错」那类不一致的根因之一。

use super::{cargo_path, CliError, Write};
use crate::args::Args;
use crate::ExitCode;

pub fn run(_a: &Args, w: &mut dyn Write) -> Result<ExitCode, CliError> {
    w.line(&format!("zephyra-cli       {}", crate::VERSION));
    w.line(&format!("zephyra-cli path  {}", current_exe_display()));
    w.line(&format!("zstylec           {}", zstylec_display()));

    match cargo_path() {
        Ok(p) => w.line(&format!("cargo             {}", p.display())),
        Err(_) => w.line("cargo             NOT FOUND  (see https://rustup.rs)"),
    }
    if let Some(tc) = rustc_version() {
        w.line(&format!("rustc             {tc}"));
    }

    let n = zstylec::supported_props().len();
    w.line(&format!("supported props   {n}"));

    // cargo 缺失是**信息**，不是失败 —— `info` 就该在坏环境下也能跑完。
    Ok(ExitCode::Success)
}

fn current_exe_display() -> String {
    std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|e| format!("<unknown: {e}>"))
}

fn zstylec_display() -> String {
    // zstylec 是**库**依赖，拿不到它的可执行文件路径；这里报我们编译进去的那一份的
    // 版本与来源，免得让人去 PATH 上猜是哪一份。
    format!("{} (linked)", env!("CARGO_PKG_VERSION"))
}

fn rustc_version() -> Option<String> {
    let out = std::process::Command::new("rustc")
        .arg("--version")
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

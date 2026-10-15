//! `run` / `build` —— 转交 cargo。
//!
//! # 为什么只是转交
//!
//! 这两个命令的价值是**省掉记住 cargo 那串标志**，而不是替 cargo 做构建。转交时
//! 故意用 `status()` 让子进程**直接继承** stdout/stderr —— 缓冲一份再转发会让
//! 编译错误看起来「半天才出来」，而 cargo 的输出是逐行有用的。
//!
//! 未知标志**原样透传**给 cargo（`zephyra run -- --nocapture`），因为 cargo 的标志
//! 比我们多，而且会变 —— 在我们这儿白名单一遍等于给用户设上限。

use super::{cargo_path, spawn_inherit, CliError, Write};
use crate::args::Args;
use crate::ExitCode;

pub fn run(a: &Args, w: &mut dyn Write) -> Result<ExitCode, CliError> {
    dispatch(a, w, false)
}

pub fn build(a: &Args, w: &mut dyn Write) -> Result<ExitCode, CliError> {
    dispatch(a, w, true)
}

fn dispatch(a: &Args, w: &mut dyn Write, is_build: bool) -> Result<ExitCode, CliError> {
    let cargo = cargo_path()?;
    let mut args: Vec<String> = vec!["run".into()];
    if is_build {
        args[0] = "build".into();
    }
    if a.has("release") {
        args.push("--release".into());
    }
    if let Some(p) = a.option("manifest-path") {
        args.push("--manifest-path".into());
        args.push(p.to_string());
    }
    // `--` 之后的原样交给 cargo。
    if let Some(i) = a.positional.iter().position(|x| x == "--") {
        args.extend(a.positional[i + 1..].iter().cloned());
    }

    w.err_line(&format!("$ cargo {}", args.join(" ")));

    let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    spawn_inherit(&cargo, &refs)
}

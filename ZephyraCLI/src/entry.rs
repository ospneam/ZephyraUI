//! `main` 的真实实现。放在 lib 里，让两个薄壳（`zephyra` 与 `cargo-zephyra`）
//! 共用同一份 —— 之前它们都是各自指向 `src/main.rs`，Cargo 会为同一个 path
//! 编译两次并**告警**。
//!
//! 两个壳存在的原因：`cargo zephyra dev` 之所以能用，靠的是 Cargo 那个
//! 「`cargo-` + 子命令名」的查找约定 —— 它在 PATH 上找 **`cargo-zephyra`**，
//! 并把 `zephyra dev` 原样传进去。所以要有两个可执行文件名，而逻辑只能有一份。

use std::process::ExitCode as ProcessExit;

use crate::args::{self, ParseOutcome};
use crate::commands::{self, Io, Write};
use crate::ExitCode;

pub fn main_entry() -> ProcessExit {
    let mut argv: Vec<String> = std::env::args().skip(1).collect();
    // `cargo zephyra dev` 会把 `zephyra` 一并传进来 —— 剥掉，两个入口就等价了。
    crate::strip_cargo_subcommand(&mut argv);

    let bin = display_name(std::env::args().next());

    let stdout = std::io::stdout();
    let stderr = std::io::stderr();
    let mut out = stdout.lock();
    let mut err = stderr.lock();
    let mut io = Io {
        out: &mut out,
        err: &mut err,
    };

    ProcessExit::from(dispatch(&bin, argv, &mut io).code())
}

fn dispatch(bin: &str, argv: Vec<String>, io: &mut dyn Write) -> ExitCode {
    let parsed = match args::parse(argv) {
        ParseOutcome::Help => {
            io.line(&commands::render_help(bin, crate::VERSION));
            return ExitCode::Success;
        }
        ParseOutcome::Version => {
            io.line(&format!("{bin} {}", crate::VERSION));
            return ExitCode::Success;
        }
        ParseOutcome::Usage(m) => {
            io.err_line(&format!("error: {m}"));
            io.err_line(&format!("try `{bin} --help`"));
            return ExitCode::Usage;
        }
        ParseOutcome::Parsed(p) => p,
    };

    let Some(name) = parsed.command.as_deref() else {
        io.err_line(&commands::render_help(bin, crate::VERSION));
        return ExitCode::Usage;
    };

    let Some(cmd) = commands::find(name) else {
        io.err_line(&format!("error: unknown command `{name}`"));
        // 顺手给最接近的 —— 拼错命令名是这个 CLI 最常见的用法错误。
        for c in commands::COMMANDS {
            if c.name.starts_with(name) || name.starts_with(c.name) {
                io.err_line(&format!("  · did you mean `{}`?", c.name));
            }
        }
        io.err_line(&format!("try `{bin} --help`"));
        return ExitCode::Usage;
    };

    // ⚠ 未知标志**立刻**报用法错，不静默忽略 —— 见 `Command::flags` 的说明。
    // `--` 之后的位置参数不算标志（它们原样透传给 cargo）。
    let after_ddash = parsed.positional.iter().position(|x| x == "--").is_some();
    if !after_ddash {
        for f in &parsed.flags {
            if !cmd.flags.contains(&f.as_str()) {
                io.err_line(&format!(
                    "error: unknown flag `--{f}` for `{bin} {}`",
                    cmd.name
                ));
                io.err_line(&format!(
                    "       accepted: {}",
                    if cmd.flags.is_empty() {
                        "(none)".to_string()
                    } else {
                        cmd.flags
                            .iter()
                            .map(|s| format!("--{s}"))
                            .collect::<Vec<_>>()
                            .join(" ")
                    }
                ));
                io.err_line(&format!("try `{bin} --help`"));
                return ExitCode::Usage;
            }
        }
        for (k, _) in &parsed.options {
            let known = matches!(k.as_str(), "format" | "path" | "manifest-path" | "styles");
            if !known {
                io.err_line(&format!(
                    "error: unknown option `--{k}` for `{bin} {}`",
                    cmd.name
                ));
                return ExitCode::Usage;
            }
        }
    }

    match (cmd.run)(&parsed, io) {
        Ok(code) => code,
        Err(e) => {
            io.err_line(&format!("error: {}", e.message));
            e.code
        }
    }
}
/// 帮助里显示的名字。
///
/// `cargo zephyra dev` 时 `argv[0]` 是 `cargo-zephyra.exe`，而用户敲的是
/// `cargo zephyra dev` —— 原样打印会让帮助里出现一个**没人会敲**的命令
/// （照着抄会得到 `cargo-zephyra.exe dev`，而那个名字不保证在 PATH 上）。
///
/// # ⚠ 为什么**不**用 `Path::file_name()`
///
/// `Path` 用平台的分隔符：Unix 上 `\` 是普通字符。于是
/// `Path::new(r"E:\tools\zephyra.exe").file_name()` 在 Linux 上返回**整串**
/// （分隔符是 `/`），帮助里就会显示 `E:\tools\zephyra` —— 一个没人会敲、而且在
/// Linux 上根本不存在的东西。症状是「帮助里的名字对不上」，而它只在**跨平台那一侧**
/// 显形（本仓 2026-10 第一次在 Linux 上跑 `cargo test --workspace` 时被这条测试逮到）。
///
/// 而这不只是「测试数据写成了 Windows 路径」：WSL 互操作里从 bash 调 Windows 版
/// 可执行文件时，`argv[0]` **真的**就是 `E:\...`。所以两个分隔符都要认。
pub fn display_name(argv0: Option<String>) -> String {
    let raw = argv0
        .map(|s| {
            // 两个平台的分隔符都当分隔符，见函数文档。
            let tail = s.rsplit(['/', '\\']).next().unwrap_or(&s);
            if tail.is_empty() { "zephyra" } else { tail }.to_string()
        })
        .unwrap_or_else(|| "zephyra".to_string());
    let stem = raw.strip_suffix(".exe").unwrap_or(&raw);
    match stem.strip_prefix("cargo-") {
        Some(rest) => format!("cargo {rest}"),
        None => stem.to_string(),
    }
}

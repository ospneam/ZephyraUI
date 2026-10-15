//! 子命令表。`--help` 与执行都查这一张表，所以两者不可能不同步。
//!
//! # 命令的取舍
//!
//! 参考 Tauri CLI 的形状（`init/dev/build/add/info` 那套），但只保留对 ZephyraUI
//! 真的有意义的：
//!
//! | 命令 | 干什么 | 为什么要 |
//! |---|---|---|
//! | `new` | 生成一个可跑的空工程 | 「上手成本」是一切的前置 |
//! | `check` | 查样式表 | 唯一能在不启动 GUI 的情况下抓错的一步，必须能进 CI |
//! | `run` | 跑起来 | |
//! | `build` | 构建 | |
//! | `info` | 报版本与工具链状态 | 出问题时第一句要问的就是「你装的什么版本」 |
//! | `props` | 列受支持的属性 | 与 `zstylec props` 同源，不制造第二条路径 |
//!
//! 没有 `migrate` / `sign` / `updater` —— 那些对应 Tauri 的分发体系，ZephyraUI
//! 现在没有，加进来就是空壳。

use crate::args::Args;
use crate::ExitCode;

pub struct Command {
    pub name: &'static str,
    pub about: &'static str,
    /// 位置参数说明。
    pub args: &'static str,
    /// 这个命令**认**的布尔标志。
    ///
    /// ⚠ 这张表存在的唯一理由是「未知标志要报错」。
    /// 实测踩过：`zephyra check --strictness` 曾经安静通过、exit 0，看起来
    /// 像是「开了严格模式跑过了」，其实那个标志根本没生效 —— 而用户以为自己
    /// 已经收紧了检查。这类静默忽略比报错糟得多。
    pub flags: &'static [&'static str],
    pub run: fn(&Args, &mut dyn Write) -> Result<ExitCode, CliError>,
}

/// 统一的输出端。抽出来是为了**能测**：命令往 stdout/stderr 写东西时，
/// 把它接到内存里就不必起子进程去验。
pub trait Write {
    fn line(&mut self, s: &str);
    fn err_line(&mut self, s: &str);
}

pub struct Io<'a> {
    pub out: &'a mut dyn std::io::Write,
    pub err: &'a mut dyn std::io::Write,
}

impl Write for Io<'_> {
    fn line(&mut self, s: &str) {
        let _ = writeln!(self.out, "{s}");
    }
    fn err_line(&mut self, s: &str) {
        let _ = writeln!(self.err, "{s}");
    }
}

/// 命令级错误。带退出码，所以「用法错」和「环境缺」不会混成一种。
#[derive(Debug)]
pub struct CliError {
    pub message: String,
    pub code: ExitCode,
}

impl CliError {
    pub fn usage(msg: impl Into<String>) -> Self {
        Self {
            message: msg.into(),
            code: ExitCode::Usage,
        }
    }
    pub fn missing(msg: impl Into<String>) -> Self {
        Self {
            message: msg.into(),
            code: ExitCode::MissingEnv,
        }
    }
    pub fn failure(msg: impl Into<String>) -> Self {
        Self {
            message: msg.into(),
            code: ExitCode::Failure,
        }
    }
}

pub const COMMANDS: &[Command] = &[
    Command {
        name: "new",
        about: "scaffold a new ZephyraUI project",
        args: "<name> [--path <dir>]",
        flags: &["force"],
                     run: new::run,
    },
    Command {
        name: "check",
        about: "check one or more stylesheets",
        args: "<file.zstyle>... [--format human|json]",
        flags: &[],
                     run: check::run,
    },
    Command {
        name: "dev",
        about: "run in dev mode: tracing on, restarts when a stylesheet changes",
        args: "[--release] [--styles <dir>] [--no-watch] [--no-trace] [--manifest-path <p>]",
        flags: &["release", "no-watch", "no-trace"],
        run: dev::run,
    },
    Command {
        name: "run",
        about: "run a ZephyraUI app (debug)",
        args: "[--release] [--manifest-path <p>]",
        flags: &["release"],
                     run: run::run,
    },
    Command {
        name: "build",
        about: "build a ZephyraUI app",
        args: "[--release] [--manifest-path <p>]",
        flags: &["release"],
                     run: build::build,
    },
    Command {
        name: "info",
        about: "print versions and toolchain status",
        args: "",
        flags: &[],
                     run: info::run,
    },
    Command {
        name: "props",
        about: "list every supported stylesheet property",
        args: "",
        flags: &[],
                     run: props::run,
    },
];

pub fn find(name: &str) -> Option<&'static Command> {
    COMMANDS.iter().find(|c| c.name == name)
}

/// 渲染帮助。**从表里生成**，所以「帮助里有的」一定「真能跑」。
pub fn render_help(bin: &str, version: &str) -> String {
    let mut s = String::new();
    s.push_str(&format!("{bin} {version}\nZephyraUI command line interface\n\nUSAGE\n    {bin} <command> [options]\n\nCOMMANDS\n"));
    let w = COMMANDS.iter().map(|c| c.name.len()).max().unwrap_or(0);
    for c in COMMANDS {
        s.push_str(&format!("    {:w$}    {}\n", c.name, c.about, w = w));
    }
    s.push_str("\nOPTIONS\n");
    s.push_str("    --help, -h        this text\n");
    s.push_str("    --version, -V     print version\n");
    s.push('\n');
    for c in COMMANDS {
        if !c.args.is_empty() {
            s.push_str(&format!("USAGE\n    {bin} {} {}\n\n", c.name, c.args));
        }
    }
    s.push_str("EXIT CODES\n");
    s.push_str("    0  success\n");
    s.push_str("    1  the action failed (check found errors, build failed, ...)\n");
    s.push_str("    2  bad usage (unknown command, missing argument)\n");
    s.push_str("    3  environment is missing (no cargo, directory already exists)\n");
    s
}

mod build;
mod check;
mod dev;
mod info;
mod new;
mod props;
mod run;

/// 找 cargo。没有它时给**明确**的缺环境错误，而不是让调用方看到一条奇怪的 spawn 失败。
pub fn cargo_path() -> Result<std::path::PathBuf, CliError> {
    // 先看 CARGO（cargo 跑我们时设的），再看 PATH 里的 cargo。
    if let Ok(p) = std::env::var("CARGO") {
        let pb = std::path::PathBuf::from(p);
        if pb.exists() {
            return Ok(pb);
        }
    }
    which_cargo().ok_or_else(|| {
        CliError::missing(
            "cargo not found. Install Rust from https://rustup.rs and make sure `cargo` is on PATH.",
        )
    })
}

fn which_cargo() -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let c = dir.join("cargo.exe");
        if c.exists() {
            return Some(c);
        }
        let c = dir.join("cargo");
        if c.exists() {
            return Some(c);
        }
    }
    None
}

/// 起一个子进程并把它的输出**继承**出去（直接透传，不缓冲）。
pub fn spawn_inherit(program: &std::path::Path, args: &[&str]) -> Result<ExitCode, CliError> {
    let status = std::process::Command::new(program)
        .args(args)
        .status()
        .map_err(|e| CliError::missing(format!("cannot run {}: {e}", program.display())))?;
    Ok(if status.success() {
        ExitCode::Success
    } else {
        ExitCode::Failure
    })
}
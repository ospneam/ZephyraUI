//! `zephyra-cli` 的库入口：命令表、参数解析、退出码。
//!
//! # 为什么命令表是数据而不是 `match`
//!
//! `--help`、shell 补全、以及「有哪些子命令」这几个问题要的是**同一份**信息。写成
//! `match` 的话，帮助文本与实际支持的动作就得手工保持同步 —— 而那种不同步通常
//! 表现为「帮助里写了但一跑就报未知命令」，属于最烦人的一类 bug。
//!
//! 所以子命令是一张表，`--help` 从表里生成，执行也查表。
//!
//! # 退出码契约（CI 与脚本依赖，别改）
//!
//! | 码 | 含义 |
//! |---|---|
//! | 0 | 成功 |
//! | 1 | 动作本身失败（检查不过、构建失败…） |
//! | 2 | 用法错（未知命令、缺参数…） |
//! | 3 | 环境缺失（找不到 cargo、目录已存在…） |

pub mod args;
pub mod entry;
pub mod commands;

pub use args::{Args, ParseOutcome};
pub use commands::COMMANDS;
pub use entry::main_entry;

/// 库版本，进 `--version`。
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// 被 cargo 当子命令调用时，cargo 会把子命令名**也**传进来：
/// `cargo zephyra dev` → `argv = ["zephyra", "dev"]`。
///
/// 所以直接当 `zephyra dev` 调时多了一个 `zephyra`。这里无条件剥掉第一个参数
/// —— 无论它是 cargo 传的还是用户传的，剥掉之后 `zephyra dev` 与
/// `cargo zephyra dev` 走到完全相同的位置。
pub fn strip_cargo_subcommand(argv: &mut Vec<String>) {
    if argv.first().is_some_and(|a| a == "zephyra" || a.ends_with("cargo-zephyra")) {
        argv.remove(0);
    }
}

/// 统一的退出码。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ExitCode {
    Success = 0,
    Failure = 1,
    Usage = 2,
    MissingEnv = 3,
}

impl ExitCode {
    pub fn code(self) -> u8 {
        self as u8
    }
}
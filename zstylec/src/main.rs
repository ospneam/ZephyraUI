//! `zstylec` 命令行入口。
//!
//! # 用法
//!
//! ```text
//! zstylec check <file.zstyle>...   # 检查（默认子命令）
//! zstylec check --format json ...   # 机器可读输出，给编辑器 / CI 用
//! zstylec props                     # 列出全部受支持的属性名
//! zstylec --help / --version
//! ```
//!
//! # 退出码（CI 依赖，别改）
//!
//! | 码 | 含义 |
//! |---|---|
//! | 0 | 全部文件干净（**警告不算脏**） |
//! | 1 | 至少一个文件有**错误** |
//! | 2 | 用法错、或文件读不了 |
//!
//! 2 与 1 分开的理由：「样式表写错了」和「文件名打错了」在 CI 里要能分别处理，
//! 前者该改代码、后者该改流水线。

use std::path::PathBuf;
use std::process::ExitCode;

use zstylec::{check as check_all, has_errors, Diag, FileReport, Level};

const USAGE: &str = "\
zstylec -- ZephyraUI stylesheet compiler

USAGE
    zstylec check <file.zstyle>...        check one or more stylesheets
    zstylec check --format json <file>... machine-readable diagnostics
    zstylec props                         list every supported property
    zstylec --help                        this text
    zstylec --version

EXIT CODES
    0  all files clean (warnings do not count as dirty)
    1  at least one file has an error
    2  bad usage, or a file could not be read
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        eprint!("{USAGE}");
        return ExitCode::from(2);
    }
    match args[0].as_str() {
        "--help" | "-h" | "help" => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        "--version" | "-V" | "version" => {
            println!("zstylec {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        "props" => {
            let mut names: Vec<&str> = zstylec::supported_props().to_vec();
            names.sort_unstable();
            for n in names {
                println!("{n}");
            }
            ExitCode::SUCCESS
        }
        _ => run_check(&args),
    }
}

fn run_check(args: &[String]) -> ExitCode {
    // `check` 可省略（`zstylec foo.zstyle` 等价于 `zstylec check foo.zstyle`），
    // 那是历史用法，别让人踩到。
    let rest = if args[0] == "check" { &args[1..] } else { args };

    let mut json = false;
    let mut paths: Vec<PathBuf> = Vec::new();
    let mut i = 0;
    while i < rest.len() {
        match rest[i].as_str() {
            "--format" => {
                i += 1;
                match rest.get(i).map(|s| s.as_str()) {
                    Some("json") => json = true,
                    Some(other) => {
                        eprintln!("error: unknown --format `{other}` (only `json`)");
                        return ExitCode::from(2);
                    }
                    None => {
                        eprintln!("error: --format needs an argument");
                        return ExitCode::from(2);
                    }
                }
            }
            "-h" | "--help" => {
                print!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            other if other.starts_with('-') => {
                eprintln!("error: unknown flag `{other}`");
                eprint!("{USAGE}");
                return ExitCode::from(2);
            }
            other => paths.push(PathBuf::from(other)),
        }
        i += 1;
    }

    if paths.is_empty() {
        eprintln!("error: no input files");
        eprint!("{USAGE}");
        return ExitCode::from(2);
    }

    let reports = match check_all(&paths) {
        Ok(r) => r,
        Err((path, e)) => {
            // 读不到 = 用法/IO 问题 ⇒ 2，不是 1。
            eprintln!("error: cannot read {}: {e}", path.display());
            return ExitCode::from(2);
        }
    };

    if json {
        emit_json(&reports);
    } else {
        emit_human(&reports);
    }

    if has_errors(&reports) {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

fn emit_human(reports: &[FileReport]) {
    let mut errors = 0usize;
    let mut warnings = 0usize;
    for r in reports {
        if r.diagnostics.is_empty() {
            println!("ok  {} ({} rules)", r.name, r.rules.unwrap_or(0));
            continue;
        }
        for d in &r.diagnostics {
            render(&r.name, d);
            match d.level {
                Level::Error => errors += 1,
                Level::Warning => warnings += 1,
            }
        }
        println!(
            "{}  {} ({} rules, {} error(s), {} warning(s))",
            r.name,
            if errors_or(r) { "FAIL" } else { "warn" },
            r.rules.unwrap_or(0),
            r.diagnostics
                .iter()
                .filter(|d| d.level == Level::Error)
                .count(),
            r.diagnostics
                .iter()
                .filter(|d| d.level == Level::Warning)
                .count()
        );
    }
    eprintln!(
        "\n{} file(s): {} error(s), {} warning(s)",
        reports.len(),
        errors,
        warnings
    );
}

fn errors_or(r: &FileReport) -> bool {
    r.diagnostics.iter().any(|d| d.level == Level::Error)
}

/// 单条诊断：位置 + 原文 + 插字符 + 说清什么错 + 最接近的。
///
/// 格式对齐 `errors.rs` 里写死的规格（文件名/行/列、原文与下划线、最近似项、建议）。
fn render(file: &str, d: &Diag) {
    let e = &d.error;
    if e.at.line == 0 {
        // 语义级诊断（重复声明）没有精确位置，就不给假坐标。
        eprintln!("{}: {}: {}", file, d.level, e.headline);
        if !e.hint.is_empty() {
            eprintln!("  hint: {}", e.hint);
        }
        eprintln!();
        return;
    }
    eprintln!(
        "{}:{}:{}: {}: {}",
        file,
        e.at.line,
        e.at.col,
        d.level,
        e.headline
    );
    if !e.line_text.is_empty() {
        eprintln!("  {}", e.line_text);
        let caret = e.underline_len.max(1);
        eprintln!("  {}{}", " ".repeat(e.at.col.saturating_sub(1)), "^".repeat(caret));
    }
    if !e.near.is_empty() {
        for (what, where_) in &e.near {
            eprintln!("  · did you mean `{what}`  ({where_})");
        }
    }
    if !e.hint.is_empty() {
        eprintln!("  hint: {}", e.hint);
    }
    eprintln!();
}

/// JSON 输出。字段名用 snake_case，够编辑器与 CI 用，不做 schema 兼容承诺。
///
/// ⚠ 这里**手写**转义而不是引 serde：`zstylec` 的依赖面很小，为一个只为 CI/编辑器
/// 服务的输出引入 serde 及其派生宏不划算。真要引，那该单独评估一次。
fn emit_json(reports: &[FileReport]) {
    println!("{{\"files\":[");
    for (fi, r) in reports.iter().enumerate() {
        println!(
            "  {{\"file\":{},\"rules\":{},\"diagnostics\":[",
            json_str(&r.name),
            r.rules.unwrap_or(0)
        );
        for (di, d) in r.diagnostics.iter().enumerate() {
            let e = &d.error;
            println!(
                "    {{\"level\":\"{}\",\"line\":{},\"col\":{},\"message\":{},\"hint\":{}{}}}",
                d.level,
                e.at.line,
                e.at.col,
                json_str(&e.headline),
                json_str(&e.hint),
                if di + 1 == r.diagnostics.len() { "" } else { "," }
            );
        }
        println!("  ]}}");
        if fi + 1 != reports.len() {
            println!(",");
        }
    }
    println!("]}}");
}

fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

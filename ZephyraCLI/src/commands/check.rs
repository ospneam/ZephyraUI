//! `check` —— 查样式表。
//!
//! # 为什么不在这里重新实现检查
//!
//! 直接调 `zstylec` 的库。CLI 与编译器**必须**给出同一份诊断 —— 两套实现意味着
//! 「CLI 说没错、cargo build 说有错」，而那种不一致极难归因（用户会怀疑其中一个
//! 装错了版本）。
//!
//! 退出码照搬 `zstylec` 的契约：0 干净 / 1 有错误 / 2 读不了文件。

use super::{CliError, Write};
use crate::args::Args;
use crate::ExitCode;

pub fn run(a: &Args, w: &mut dyn Write) -> Result<ExitCode, CliError> {
    let format = a.option("format").unwrap_or("human");
    if format != "human" && format != "json" {
        return Err(CliError::usage(format!(
            "unknown --format `{format}` (expected `human` or `json`)"
        )));
    }

    if a.positional.is_empty() {
        return Err(CliError::usage(
            "check needs at least one file. Try `zephyra check app.zstyle`.",
        ));
    }

    let paths: Vec<std::path::PathBuf> = a.positional.iter().map(std::path::PathBuf::from).collect();
    // ⚠ 读不到文件是**用法错**（退出码 2），不是「样式表有问题」（1）。
    // 两者在 CI 里要能分别处理：前者改流水线/路径，后者改样式表。
    let reports = zstylec::check(&paths)
        .map_err(|(p, e)| CliError::usage(format!("cannot read {}: {e}", p.display())))?;

    if format == "json" {
        emit_json(&reports, w);
    } else {
        emit_human(&reports, w);
    }

    let (errors, warnings) = counts(&reports);
    w.err_line(&format!(
        "\n{} file(s): {errors} error(s), {warnings} warning(s)",
        reports.len()
    ));

    Ok(if zstylec::has_errors(&reports) {
        ExitCode::Failure
    } else {
        ExitCode::Success
    })
}

fn counts(rs: &[zstylec::FileReport]) -> (usize, usize) {
    let mut e = 0;
    let mut wn = 0;
    for r in rs {
        for d in &r.diagnostics {
            match d.level {
                zstylec::Level::Error => e += 1,
                zstylec::Level::Warning => wn += 1,
            }
        }
    }
    (e, wn)
}

fn emit_human(rs: &[zstylec::FileReport], w: &mut dyn Write) {
    for r in rs {
        if r.diagnostics.is_empty() {
            w.line(&format!("ok  {} ({} rules)", r.name, r.rules.unwrap_or(0)));
            continue;
        }
        let has_err = r.diagnostics.iter().any(|d| d.level == zstylec::Level::Error);
        for d in &r.diagnostics {
            let e = &d.error;
            w.err_line(&format!(
                "{}:{}:{}: {}: {}",
                r.name, e.at.line, e.at.col, d.level, e.headline
            ));
            if !e.line_text.is_empty() {
                w.err_line(&format!("  {}", e.line_text));
                let caret = e.underline_len.max(1);
                w.err_line(&format!(
                    "  {}{}",
                    " ".repeat(e.at.col.saturating_sub(1)),
                    "^".repeat(caret)
                ));
            }
            if !e.near.is_empty() {
                for (what, where_) in &e.near {
                    w.err_line(&format!("  · did you mean `{what}`  ({where_})"));
                }
            }
            if !e.hint.is_empty() {
                w.err_line(&format!("  hint: {}", e.hint));
            }
            w.err_line("");
        }
        w.line(&format!(
            "{}  {} ({} rules)",
            r.name,
            if has_err { "FAIL" } else { "warn" },
            r.rules.unwrap_or(0)
        ));
    }
}

/// JSON。字段名与 `zstylec --format json` 一致 —— 两个入口给出同一份形状，
/// 下游脚本才不用分支。
fn emit_json(rs: &[zstylec::FileReport], w: &mut dyn Write) {
    w.line("{\"files\":[");
    for (fi, r) in rs.iter().enumerate() {
        w.line(&format!(
            "  {{\"file\":{},\"rules\":{},\"diagnostics\":[",
            esc(&r.name),
            r.rules.unwrap_or(0)
        ));
        for (di, d) in r.diagnostics.iter().enumerate() {
            let e = &d.error;
            w.line(&format!(
                "    {{\"level\":\"{}\",\"line\":{},\"col\":{},\"message\":{},\"hint\":{}{}}}",
                d.level,
                e.at.line,
                e.at.col,
                esc(&e.headline),
                esc(&e.hint),
                if di + 1 == r.diagnostics.len() { "" } else { "," }
            ));
        }
        w.line("  ]}}");
        if fi + 1 != rs.len() {
            w.line(",");
        }
    }
    w.line("]}");
}

fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}
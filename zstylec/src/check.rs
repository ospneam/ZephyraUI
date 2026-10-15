//! 检查入口：给 CLI 与构建期调用方一个「一次查多个文件」的接口。
//!
//! # 为什么不在 CLI 里现拼
//!
//! `parse` 只吃文本与文件名。CLI 要做的事（读盘、逐个检查、汇总、决定退出码）如果
//! 只写在 `main.rs` 里，就**没法测** —— 而「多文件时第 2 个文件出错，返回码仍然是
//! 1，且第 1 个文件的结果照样打出来」正是最容易写错的地方。所以它进 lib。
//!
//! # 退出码契约（CLI 依赖它，别改）
//!
//! | 码 | 含义 |
//! |---|---|
//! | 0 | 全部文件干净 |
//! | 1 | 至少一个文件有诊断（错误或警告） |
//! | 2 | 用法错、或文件读不了 —— **不是**「样式表有问题」 |
//!
//! 2 与 1 必须分开：CI 里「样式表写错了」和「我把文件名打错了」要能分别处理。

use std::path::{Path, PathBuf};

use crate::errors::Error;
use crate::parse;

/// 单个文件的检查结果。
#[derive(Debug)]
pub struct FileReport {
    /// 传入的路径（原样）。
    pub path: PathBuf,
    /// 短名（打印用）。
    pub name: String,
    /// 解析出的规则数。读不了文件时是 `None`。
    pub rules: Option<usize>,
    /// 该文件的诊断（错误 + 警告）。
    pub diagnostics: Vec<Diag>,
}

/// 一条诊断，带严重级别。
///
/// # 为什么要分两级
///
/// 「属性名拼错」和「同一个块里把 `width` 写了两遍」性质不同：前者一定错，后者
/// 后面那条**一定生效**（`upsert` 语义），只是前面那条白写了。只报错误会漏掉
/// 后者 —— 而后者正是样式表腐化的起点。
#[derive(Debug)]
pub struct Diag {
    /// 错误还是警告。
    pub level: Level,
    /// 具体报错本体。
    pub error: Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Error,
    Warning,
}

impl std::fmt::Display for Level {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Level::Error => "error",
            Level::Warning => "warning",
        })
    }
}

/// 检查一个文件。读不了文件时返回 `Err`（那是用法/IO 问题，对应退出码 2）。
pub fn check_file(path: &Path) -> Result<FileReport, std::io::Error> {
    let css = std::fs::read_to_string(path)?;
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("<unnamed>")
        .to_string();
    Ok(check_text(path, name, &css))
}

/// 检查一段文本（文件名只用于报错定位）。测试直接走这条，不必落盘。
pub fn check_text(path: &Path, name: String, css: &str) -> FileReport {
    let result = parse::parse(&name, css);
    let mut diagnostics: Vec<Diag> = result
        .errors
        .into_iter()
        .map(|error| Diag {
            level: Level::Error,
            error,
        })
        .collect();

    // 同一个块里重复声明同一个属性 = 前一条被覆盖。
    //
    // 为什么能查：`StyleSheet` 的规则里声明是有序的，而属性键的身份是**地址**
    // （`PropertyKey` 没有 `PartialEq`，全仓一律 `ptr::eq` 比对，见 property.rs）。
    diagnostics.extend(duplicate_declarations(&name, &result.sheet));

    diagnostics.sort_by_key(|d| (d.error.at.line, d.error.at.col));

    FileReport {
        path: path.to_path_buf(),
        name,
        rules: Some(result.sheet.rules().len()),
        diagnostics,
    }
}

/// 检查多个文件。**读不着的文件立刻返回 `Err`** —— 那是用法错，不是样式表错。
pub fn check(paths: &[PathBuf]) -> Result<Vec<FileReport>, (PathBuf, std::io::Error)> {
    let mut out = Vec::with_capacity(paths.len());
    for p in paths {
        match check_file(p) {
            Ok(r) => out.push(r),
            Err(e) => return Err((p.clone(), e)),
        }
    }
    Ok(out)
}

/// 有没有任何一条**错误**级诊断（警告不算）。
pub fn has_errors(reports: &[FileReport]) -> bool {
    reports
        .iter()
        .flat_map(|r| &r.diagnostics)
        .any(|d| d.level == Level::Error)
}

fn duplicate_declarations(name: &str, sheet: &zephyra_style::ir::StyleSheet) -> Vec<Diag> {
    let mut out = Vec::new();
    for rule in sheet.rules() {
        let mut seen: Vec<&'static zephyra_core::property::PropertyKey> = Vec::new();
        for decl in &rule.declarations {
            if seen.iter().any(|k| std::ptr::eq(*k, decl.key)) {
                out.push(Diag {
                    level: Level::Warning,
                    error: Error {
                        file: name.to_string(),
                        line_text: String::new(),
                        at: crate::errors::Pos { line: 0, col: 0 },
                        underline_len: 0,
                        headline: format!(
                            "duplicate declaration of `{}` in the same block; the later one wins.",
                            decl.key.name()
                        ),
                        near: Vec::new(),
                        hint: "remove the earlier one -- writing a property twice in one block \
                               is never an override, it is just the first one being dropped."
                            .into(),
                    },
                });
            } else {
                seen.push(decl.key);
            }
        }
    }
    out
}
//! 报错信息。
//!
//! # 规格（计划「报错信息」一节，硬要求）
//!
//! 新框架的报错质量直接决定开发者能不能上手。每条报错必须带齐：
//!
//! | 要素 | 例 |
//! |---|---|
//! | **文件名 + 行号 + 列号** | `button.zstyle 第 12 行第 5 列` |
//! | **原文 + 下划线** | `Control { … }` + `^^^^` |
//! | 说清是什么错 | 「未知的属性名」 |
//! | **最接近的几个**（带各自位置） | `· corner-radius  (button.zstyle:18)` |
//! | **建议** | 「属性名要写成已支持的那个」 |
//!
//! ⚠️ 缺「最接近的」的报错是半废的——拼错字是 90% 的情况，
//! 而这才是那句报错里唯一真正有用的部分。编辑距离很便宜，必须给。

use std::fmt;

/// 源文件里的一个位置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pos {
    /// 1 起的行号。
    pub line: u32,
    /// 1 起的列号。
    pub col: usize,
}

impl fmt::Display for Pos {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}, col {}", self.line, self.col)
    }
}

/// 一条编译错误。
#[derive(Debug, Clone)]
pub struct Error {
    /// 出错的文件。
    pub file: String,
    /// 出错的源码行（用于「原文」那一行）。
    pub line_text: String,
    /// 出错的位置。
    pub at: Pos,
    /// 下划线要盖多少个字符。
    pub underline_len: usize,
    /// 一句结论：「未知的属性名」。
    pub headline: String,
    /// 最接近的几个候选，带它们各自的行号（**报错里唯一真正有用的部分**）。
    pub near: Vec<(String, u32)>,
    /// 下一步该做什么。
    pub hint: String,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "error: {} {}", self.file, self.at)?;
        // 原文那一行。行首加行号，出错的字符下面盖 `^`。
        writeln!(f, "    {}", self.line_text)?;
        let pad = " ".repeat(self.at.col.saturating_sub(1));
        let carets = "^".repeat(self.underline_len.max(1));
        writeln!(f, "    {pad}{carets}")?;
        writeln!(f, "{}", self.headline)?;
        if !self.near.is_empty() {
            writeln!(f, "closest matches:")?;
            for (name, line) in &self.near {
                writeln!(f, "  · {name}  ({}:{})", self.file, line)?;
            }
        }
        write!(f, "hint: {}", self.hint)
    }
}

/// 两个字符串的编辑距离（Levenshtein，只用 `usize` 的一维滚动数组）。
///
/// # 为什么自己写而不引 crate
///
/// `zstylec` 是 dev 工具，能少一个依赖就少一个。而且这个函数不到 20 行。
fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    // 滚动两行就够，不必留整个矩阵。
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur[j + 1] = (prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// 从候选里挑最接近的几个。
///
/// ⚠️ **按距离升序**，同距离按字典序——否则每次编译候选项顺序不定，
/// 同一份输入会输出不同的报错文本，那没法做快照测试。
pub fn nearest(want: &str, candidates: &[&str], limit: usize) -> Vec<String> {
    let mut scored: Vec<(usize, &str)> = candidates
        .iter()
        .map(|c| (edit_distance(want, c), *c))
        .collect();
    scored.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(b.1)));
    scored.truncate(limit);
    scored.into_iter().map(|(_, s)| s.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_distance_orders_candidates_correctly() {
        let near = nearest("coner-radius", &["corner-radius", "background", "width"], 1);
        assert_eq!(near, vec!["corner-radius"]);
    }

    #[test]
    fn edit_distance_of_identical_strings_is_zero() {
        assert_eq!(edit_distance("corner-radius", "corner-radius"), 0);
    }

    #[test]
    fn edit_distance_counts_swaps_as_two() {
        assert_eq!(edit_distance("ab", "ba"), 2);
    }

    #[test]
    fn nearest_is_stable_for_ties() {
        // 同距离时必须按字典序，否则同一份输入两次编译报不同错。
        let near = nearest("x", &["b", "a"], 2);
        assert_eq!(near, vec!["a", "b"]);
    }

    #[test]
    fn error_renders_with_file_line_and_caret() {
        let e = Error {
            file: "button.zstyle".into(),
            line_text: "Control { coner-radius: 4px; }".into(),
            at: Pos { line: 3, col: 12 },
            underline_len: 13,
            headline: "unknown property name 'coner-radius'.".into(),
            near: vec![("corner-radius".into(), 7)],
            hint: "property name must be a supported one.".into(),
        };
        let s = e.to_string();
        assert!(s.contains("button.zstyle"), "must contain the file name");
        assert!(s.contains("line 3, col 12"), "must give line and col");
        assert!(s.contains("^^^^^^^^^^^^^"), "must have the caret");
        assert!(s.contains("closest matches:"), "must list candidates");
        assert!(s.contains("corner-radius"), "candidates must appear");
        assert!(s.contains("hint:"), "must give a hint");
    }
}

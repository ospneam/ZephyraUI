//! 参数解析。手写，但**有测试**。
//!
//! # 为什么手写
//!
//! 子命令十几个、标志十来个，全是「位置参数 + 布尔开关」，没有嵌套子命令、没有
//! 变参类型推导。clap 那套 derive 的价值在这里用不上，而它带来的是一棵依赖树和
//! 一层编译期宏。手写解析器换来的是：**错误文案完全可控**（这项目对报错质量要求
//! 很高，见 `zstylec/src/errors.rs` 写死的规格），以及零依赖。
//!
//! # 约定
//!
//! - `--flag` 与 `--flag=value` 都收；`-f` 形式只给少数短标志。
//! - `--` 之后的一切当位置参数，不再解析标志（`zephyra new -- --weird-name`）。
//! - 未知标志**立刻**报用法错（退出码 2），不静默忽略 —— 静默忽略会让
//!   `zephyra check --strictness` 看起来跑成功了，其实什么都没开。

/// 解析后的参数。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Args {
    /// 子命令名（`check` / `new` / …）。`None` 表示没给。
    pub command: Option<String>,
    /// 位置参数，按出现顺序。
    pub positional: Vec<String>,
    /// `--name` → true。
    pub flags: Vec<String>,
    /// `--name=value` → value。
    pub options: Vec<(String, String)>,
    /// `--name` 出现次数（重复给要能查出来，而不是静默取最后一个）。
    pub counts: Vec<(String, usize)>,
}

/// **取值型**标志：这些 `--name` 后面跟一个独立的参数。
///
/// # 为什么要有这张表
///
/// 实测踩过：`--manifest-path x/Cargo.toml`（空格形式）曾被解析成「一个叫
/// `manifest-path` 的布尔标志 + 一个位置参数」，于是 `run` 把工程路径当成了位置
/// 参数 —— 而**命令表里的文档明明写着 `--manifest-path <p>`**。文档说了、解析器
/// 做不到，那比两个都不做更糟：使用者会以为路径已经生效了。
///
/// 所以取值型标志必须**显式列出**。没列进去的 `--foo bar` 里，`bar` 会被当位置
/// 参数 —— 这是刻意的：`--` 之后要原样透传给 cargo 的东西正是靠这条规则保持形状。
pub const VALUE_FLAGS: &[&str] = &["format", "path", "manifest-path", "styles"];

/// 解析结果：要么是参数，要么是一条**用法错误文案**。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseOutcome {
    Parsed(Args),
    /// 需要打印帮助（`--help` / 无参数）。
    Help,
    Version,
    Usage(String),
}

pub fn parse<I: IntoIterator<Item = String>>(argv: I) -> ParseOutcome {
    let mut out = Args::default();
    let mut it = argv.into_iter().peekable();
    let mut literal = false;

    while let Some(a) = it.next() {
        if literal {
            push_positional(&mut out, a);
            continue;
        }
        if a == "--" {
            literal = true;
            continue;
        }
        if let Some(rest) = a.strip_prefix("--") {
            if rest.is_empty() {
                return ParseOutcome::Help;
            }
            match rest.split_once('=') {
                Some((k, v)) => {
                    if k.is_empty() {
                        return ParseOutcome::Usage(format!("bad flag `--{rest}`"));
                    }
                    out.options.push((k.to_string(), v.to_string()));
                }
                None => {
                    if rest == "help" {
                        return ParseOutcome::Help;
                    }
                    if rest == "version" {
                        return ParseOutcome::Version;
                    }
                    if VALUE_FLAGS.contains(&rest) {
                        // `--name value`：吃掉下一个参数。
                        match it.next() {
                            Some(v) => out.options.push((rest.to_string(), v)),
                            None => {
                                return ParseOutcome::Usage(format!("`--{rest}` needs an argument"))
                            }
                        }
                        continue;
                    }
                    out.flags.push(rest.to_string());
                }
            }
            continue;
        }
        if a.len() > 1 && a.starts_with('-') {
            // 短标志只认单字母，不做聚簇（`check -vq`）。够用且没有歧义。
            let k = a.trim_start_matches('-');
            if k.is_empty() {
                return ParseOutcome::Usage(format!("bad flag `{a}`"));
            }
            out.flags.push(k.to_string());
            continue;
        }
        push_positional(&mut out, a);
    }

    // 第一个位置参数当子命令。
    if !out.positional.is_empty() {
        out.command = Some(out.positional.remove(0));
    }

    // 统计重复标志。
    for f in &out.flags {
        match out.counts.iter_mut().find(|(k, _)| k == f) {
            Some((_, c)) => *c += 1,
            None => out.counts.push((f.clone(), 1)),
        }
    }

    ParseOutcome::Parsed(out)
}

fn push_positional(out: &mut Args, a: String) {
    // 子命令位置之前的第一个位置参数已经被当成 command 了；这里只负责后续的。
    out.positional.push(a);
}

impl Args {
    pub fn has(&self, flag: &str) -> bool {
        self.flags.iter().any(|f| f == flag)
    }

    pub fn count(&self, flag: &str) -> usize {
        self.counts
            .iter()
            .find(|(k, _)| k == flag)
            .map(|(_, c)| *c)
            .unwrap_or(0)
    }

    pub fn option(&self, name: &str) -> Option<&str> {
        self.options
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    pub fn positional_at(&self, i: usize) -> Option<&str> {
        self.positional.get(i).map(|s| s.as_str())
    }
}

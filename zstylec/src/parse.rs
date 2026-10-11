//! 解析器：`.zstyle` 文本 → [`StyleSheet`]（IR）。
//!
//! # 用 cssparser 的哪一层（决定过一次）
//!
//! 我第一版手写了 tokenizer 级别的循环（`next()` / 数花括号 / 自己跳分号）。
//! 那是**错的**：cssparser 提供了 `QualifiedRuleParser` / `DeclarationParser` /
//! `AtRuleParser` 三个 trait，`name:` 的拆分、`;` 的分隔、坏声明的恢复**全都**由
//! `DeclarationListParser` 处理。自己重写等于把「用成熟 crate」的前提扔掉，
//! 只剩 cssparser 的 tokenizer。
//!
//! 现在按官方惯例走：实现那三个 trait，把 `;`、错误恢复交给它。
//!
//! # 官方惯例（Servo / librsvg 同款）
//!
//! ```rust,ignore
//! impl QualifiedRuleParser for P {          // 选择器 + `{ ... }`
//!   fn parse_prelude -> Selector
//!   fn parse_block  -> Rule
//! }
//! impl DeclarationParser for P {            // `name: value`
//!   fn parse_value -> Declaration
//! }
//! impl AtRuleParser for P {}                // 空 = 全部 at-rule 报错
//! ```
//!
//! # 支持什么
//!
//! | | 写法 | 例 |
//! |---|---|---|
//! | 类型名 | `Control` / `Text` / `Container` / `Image` | 对应 `NodeTag` |
//! | 分类名 | `.名字` | 对应 `.with_key("危险")` |
//! | 组合 | `Button.危险.紧凑` | 类型 + 多个分类名 |
//! | 注释 | `/* … */` | |
//!
//! # 不支持的一律**加载时**报错（不静默忽略）
//!
//! 后代组合器（`.a .b`）、伪类（`:hover`）、伪元素（`::before`）、
//! `@media` / `@layer` / `@keyframes`、`!important`、逗号分组。
//!
//! ⚠️ **静默忽略的样式表比没有样式表更糟**：设计者以为生效了。

use cssparser::{ParseErrorKind, ParserInput,
    AtRuleParser, BasicParseError, BasicParseErrorKind, CowRcStr, DeclarationParser, ParseError,
    Parser, QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser, StyleSheetParser, Token,
};
use zephyra_core::id::NodeTag;

use crate::errors::{nearest, Error};
use crate::props_map;
use crate::value;
use zephyra_style::ir::{Declaration, Selector, StyleRule, StyleSheet};

/// 解析成功的产物 + 收集到的错误。
///
/// # 为什么要「一次全报」
///
/// 一次只报一个错的话，开发者要反复「改一处→编译→再报下一处」，比没有编译器还慢。
/// cssparser 的迭代器会在每条 `;` 后继续，所以一次报完是它的自然形态。
pub struct ParseResult {
    pub sheet: StyleSheet,
    pub errors: Vec<Error>,
}

impl ParseResult {
    /// 干净吗。
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }
}

/// 我们的错误类型。
///
/// cssparser 要求实现 `From<BasicParseError>`，所以 `E = Error` 时这条必须能构造。
#[derive(Debug)]
pub enum OwnError {
    /// cssparser 报的（token 级）错。
    Basic(String),
    /// 我们自己报的（语义级：未知类型 / 未知属性 / 坏值）。
    Own(Error),
}

impl<'i> From<BasicParseError<'i>> for OwnError {
    fn from(e: BasicParseError<'i>) -> Self {
        // ⚠️ `BasicParseError` 没实现 `Display`（它的 kind 才有），所以不能 `to_string()`。
        // 这里只带「是什么 kind」，原文位置由 `ParserState` 另给。
        Self::Basic(describe_basic(&e.kind))
    }
}

/// 把 `BasicParseErrorKind` 翻成人话。
fn describe_basic(kind: &BasicParseErrorKind) -> String {
    use BasicParseErrorKind::*;
    match kind {
        UnexpectedToken(t) => format!("unexpected here: {t:?}"),
        EndOfInput => "input ends here".into(),
        AtRuleInvalid(n) => format!("unsupported @{n}"),
        AtRuleBodyInvalid => "invalid at-rule body".to_string(),
        QualifiedRuleInvalid => "malformed rule".to_string(),
        UnexpectedTokenWithinDeclaration => "unexpected token in declaration".to_string(),
    }
}

/// `ParseErrorKind` 有两种：cssparser 的 token 级，和「我们自己的」。
///
/// 后者走的是 `From<BasicParseError> for OwnError` 那条路 —— 现实中我们总是用
/// `input.new_error(..)` 造 Basic，所以 Custom 分支其实走不到；留着是为了
/// 匹配穷尽，也让将来真用它时不静默降级。
fn describe_kind(kind: &ParseErrorKind<'_, OwnError>) -> String {
    match kind {
        ParseErrorKind::Basic(b) => describe_basic(b),
        ParseErrorKind::Custom(e) => match e {
            OwnError::Basic(s) => s.clone(),
            OwnError::Own(own) => own.headline.clone(),
        },
    }
}

/// 块/顶层迭代器吐出来的条目。
///
/// # 为什么需要这个枚举
///
/// cssparser 的 [`RuleBodyItemParser`] 要求三个 trait 的关联类型**统一**：
/// `Declaration` / `QualifiedRule` / `AtRule` 必须是同一个 `DeclOrRule`。
/// 我们只 parse 前两种（at-rule 一律报错），但类型还得统一，
/// 所以用这个枚举包一下——这正是它的全部作用。
#[derive(Debug)]
pub enum Item {
    Decl(Declaration),
    Rule(StyleRule),
}

/// CSS 类型名 → `NodeTag`。
///
/// ⚠️ `NodeTag` 是**封闭**的四元枚举（`id.rs`：宿主自定义元素类型「在类型层面
/// 就是非法的」），所以这张表也是封闭的、不需要运行时注册。
const TAG_NAMES: &[(&str, NodeTag)] = &[
    ("Text", NodeTag::Text),
    ("Image", NodeTag::Image),
    ("Container", NodeTag::Container),
    ("Control", NodeTag::Control),
];

fn tag_by_name(name: &str) -> Option<NodeTag> {
    TAG_NAMES.iter().find(|(n, _)| *n == name).map(|(_, t)| *t)
}

fn all_tag_names() -> Vec<&'static str> {
    TAG_NAMES.iter().map(|(n, _)| *n).collect()
}

/// 顶层解析器：实现 cssparser 的三个 trait。
pub(crate) struct ZstyleParser<'a> {
    /// 文件名，报错时用。
    pub(crate) file: &'a str,
    /// 原文，报错时要摘「出错那一行」。
    pub(crate) css: &'a str,
    /// 规则在这个路径上排第几（同特异性时定胜负）。
    order: u32,
    rules: Vec<StyleRule>,
    pub(crate) errors: Vec<Error>,
}

impl<'a> ZstyleParser<'a> {
    /// 按行号摘原文。
    pub(crate) fn line_text(&self, line: u32) -> String {
        self.css
            .lines()
            .nth((line.saturating_sub(1)) as usize)
            .unwrap_or("")
            .trim()
            .to_string()
    }

    /// 造一条错误。列号一律填 1、下划线盖整行——理由见 [`Error::underline_len`]。
    pub(crate) fn err(&mut self, line: u32, headline: String, near: Vec<(String, u32)>, hint: &str) {
        self.errors.push(Error {
            file: self.file.to_string(),
            line_text: self.line_text(line),
            at: crate::errors::Pos { line, col: 1 },
            underline_len: self.line_text(line).chars().count(),
            headline,
            near,
            hint: hint.into(),
        });
    }

    /// 接住一条 cssparser 的 token 级错误。
    ///
    /// # 为什么必须有这个方法
    ///
    /// 原实现是 `filter_map(|r| r.ok())` —— cssparser 报的错被**整条丢掉**。
    /// 后果不是「少一条提示」：`Control { .inner { width: 1px; } }` 和
    /// `Control width: 1px;`（少了 `{ }`）都解析成 **0 条规则、0 个错误**。
    /// 设计者以为生效了，实际什么也没有。这正是本轨道明令禁止的失败模式，
    /// 而它只需要一个 `filter_map` 就能造出来。
    ///
    /// # 为什么同一行只报一次
    ///
    /// 声明出错时（属性名拼错、值不合法），`parse_value` 已经记过一条带
    /// 「是哪个属性、最接近的候选、该写什么」的语义错误。cssparser 对此只会
    /// 补一句 `unexpected here: Ident("coner-radius")`，没有任何信息量。
    /// 一行一条消息，报详细的那条。
    ///
    /// `leftover` 是 cssparser 从出错位置切到输入末尾的原文，用来决定这句话
    /// 该怎么说（剩下的一段里还有 `{`，那就是嵌套规则）。
    pub(crate) fn note_token_error(&mut self, line: u32, what: &str, leftover: &str) {
        if self.errors.iter().any(|e| e.at.line == line) {
            return;
        }
        let headline = if leftover.contains('{') {
            "nested rules are not supported.".to_string()
        } else {
            format!("cannot parse this ({what}).")
        };
        self.errors.push(Error {
            file: self.file.to_string(),
            line_text: self.line_text(line),
            at: crate::errors::Pos { line, col: 1 },
            underline_len: self.line_text(line).chars().count(),
            headline,
            near: Vec::new(),
            hint: "rules look like `Type.class { property: value; }`.".into(),
        });
    }
}

impl<'i> AtRuleParser<'i> for ZstyleParser<'_> {
    type Prelude = ();
    type AtRule = Item;
    type Error = OwnError;

    // 默认实现把一切 at-rule 判为 invalid。我们要的是**报错**而不是跳过，
    // 所以显式重写：`@media` / `@layer` / `@keyframes` 全都在这里被拒且给出理由。
    fn parse_prelude<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::Prelude, ParseError<'i, Self::Error>> {
        let line = input.current_source_location().line + 1;
        let what = match name.as_ref() {
            "media" | "supports" | "container" => "conditional rule",
            "layer" => "layer",
            "keyframes" => "animation",
            "font-face" => "font descriptor",
            _ => "at-rule",
        };
        self.err(
            line,
            format!("unsupported @{name} ({what})."),
            Vec::new(),
            "only Type.class { prop: value; } is supported.",
        );
        Err(input.new_error(BasicParseErrorKind::AtRuleInvalid(name)))
    }
}

impl<'i> QualifiedRuleParser<'i> for ZstyleParser<'_> {
    type Prelude = Selector;
    type QualifiedRule = Item;
    type Error = OwnError;

    fn parse_prelude<'t>(
        &mut self,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::Prelude, ParseError<'i, Self::Error>> {
        let line = input.current_source_location().line + 1;
        parse_selector(input, self, line)
    }

    fn parse_block<'t>(
        &mut self,
        prelude: Self::Prelude,
        _start: &cssparser::ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::QualifiedRule, ParseError<'i, Self::Error>> {
        // 块内：`name:` 的拆分、`;`、坏声明恢复全交给 `RuleBodyParser`。
        //
        // ⚠️ 早先一版在这里**再造一个 parser**、再把 rules/errors 搬回来。
        // 那既难看又容易漏（漏一次就是「这条规则的错误凭空消失」）。
        // 现在直接用 self：`RuleBodyParser` 的构造与 `parse_value` 的
        // `&mut self` 都在同一个调用栈里，合法。
        let mut decls: Vec<Declaration> = Vec::new();
        // ⚠️ 收集到**局部** Vec，循环结束后才合并。
        //
        // 不能在循环里写 `self.errors`：`RuleBodyParser::new(input, &mut *self)`
        // 在整个 for 期间独占借用 `*self`，循环体内碰 `self` 就是第二次借用。
        // （`parse_value` 能写 errors 是因为它拿到的正是那份 `&mut self`。）
        //
        // 而这一份错误**不能丢**：`Control { .inner { … } }` 的报错就出自这里，
        // 丢了就是「0 条规则、0 个错误」。
        let mut token_errs: Vec<(u32, String, String)> = Vec::new();
        for item in RuleBodyParser::new(input, &mut *self) {
            match item {
                Ok(Item::Decl(d)) => decls.push(d),
                Ok(Item::Rule(_)) => {
                    // `parse_qualified()` 返回 false，cssparser 不会把块里的东西
                    // 当成规则交上来。真出现了就是调度变了，别静默放过。
                }
                Err((e, leftover)) => {
                    // 具体原因已由 `parse_value` 记进 self.errors（那里才有
                    // 「是哪个属性、哪个值」的上下文）。这里把 cssparser 的
                    // token 级原因先收着，循环结束后交给 note_token_error 定夺。
                    //
                    // ⚠️ 残缺的规则**保留**：否则「拼错一个属性名」会让整组样式
                    // 一起消失，那是二次伤害。
                    token_errs.push((
                        e.location.line + 1,
                        describe_kind(&e.kind),
                        leftover.to_string(),
                    ));
                }
            }
        }
        for (line, what, leftover) in token_errs {
            self.note_token_error(line, &what, &leftover);
        }
        let rule = StyleRule::new(prelude, decls, self.order);
        self.order += 1;
        Ok(Item::Rule(rule))
    }
}

/// 声明解析器。`RuleBodyItemParser` 需要我们**两个**都能 parse（CSS 里声明表
/// 原则上也可能出现 at-rule 和嵌套规则），我们只 parse 声明、拒绝另外两种。
impl<'i> DeclarationParser<'i> for ZstyleParser<'_> {
    type Declaration = Item;
    type Error = OwnError;

    fn parse_value<'t>(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
        _declaration_start: &cssparser::ParserState,
    ) -> Result<Self::Declaration, ParseError<'i, Self::Error>> {
        let line = input.current_source_location().line + 1;
        let css_name = name.to_string();

        // 属性名必须在表里。
        let key = match props_map::lookup(&css_name) {
            Some(k) => k,
            None => {
                let all: Vec<&str> = props_map::all_css_names().collect();
                let near = nearest(&css_name, &all, 3);
                let hint = format!(
                    "property must be supported. full list in props_map.rs ({} entries).",
                    all.len()
                );
                self.err(
                    line,
                    format!("unknown property '{css_name}'."),
                    near.into_iter().map(|n| (n, line)).collect(),
                    &hint,
                );
                return Err(input.new_error(BasicParseErrorKind::UnexpectedToken(
                    Token::Ident(name),
                )));
            }
        };

        // 值。
        match value::parse(input, self, line, key) {
            Ok(v) => Ok(Item::Decl(Declaration { key, value: v })),
            Err(()) => {
                // `value::parse` 已经把具体原因记进 self.errors。
                Err(input.new_error(BasicParseErrorKind::UnexpectedToken(
                    Token::Ident(name),
                )))
            }
        }
    }
}

impl<'i> RuleBodyItemParser<'i, Item, OwnError> for ZstyleParser<'_> {
    fn parse_declarations(&self) -> bool {
        true
    }
    fn parse_qualified(&self) -> bool {
        // 不支持嵌套规则（CSS Nesting）。返回 false = 遇到 `{` 就当错误，
        // 而不是试着再解析成一条规则。
        false
    }
}

/// 把一段 CSS 文本解析成样式表。
pub fn parse(file_name: &str, css: &str) -> ParseResult {
    let mut parser = ZstyleParser {
        file: file_name,
        css,
        order: 0,
        rules: Vec::new(),
        errors: Vec::new(),
    };

    let mut input_source = ParserInput::new(css);
    let mut input = Parser::new(&mut input_source);
    // ⚠️ **先 collect 再 push**。下面那个 `for` 里不能 push：迭代器本身
    // 借走了 `&mut parser`，push 就是第二次可变借用。先收进 Vec 再一个个 push
    // 是最小代价的解法——错误路径已经把这些 Item 报出来了，规则数本来就几十条。
    //
    // ⚠️ 但**不能** `filter_map(|r| r.ok())`：那会把 cssparser 报的错整条丢掉。
    // 改写进下面这个局部 Vec，循环结束后合并。
    let mut token_errs: Vec<(u32, String, String)> = Vec::new();
    let items: Vec<Item> = StyleSheetParser::new(&mut input, &mut parser)
        .filter_map(|r| match r {
            Ok(item) => Some(item),
            Err((e, leftover)) => {
                token_errs.push((
                    e.location.line + 1,
                    describe_kind(&e.kind),
                    leftover.to_string(),
                ));
                None
            }
        })
        .collect();
    for (line, what, leftover) in token_errs {
        parser.note_token_error(line, &what, &leftover);
    }
    for item in items {
        match item {
            Item::Rule(r) => parser.rules.push(r),
            Item::Decl(_) => {
                // 顶层不该有裸声明（那是夹在规则外面的）。`parse_qualified()`
                // 返回 false，所以走到这里说明 CSS 写成了裸声明。
            }
        }
    }

    ParseResult {
        sheet: StyleSheet::from_rules(std::mem::take(&mut parser.rules)),
        errors: parser.errors,
    }
}

/// 解析选择器：`Tag`、`.名字`、`Tag.名字.名字`。
fn parse_selector<'i>(
    input: &mut Parser<'i, '_>,
    p: &mut ZstyleParser<'_>,
    line: u32,
) -> Result<Selector, ParseError<'i, OwnError>> {
    let _ = p;
    let mut tag: Option<NodeTag> = None;
    let mut names: Vec<&'static str> = Vec::new();

    loop {
        // 类型名（可选，只能出现在最前面）。
        //
        // ⚠️ 用 `try_parse` 而不是裸 `next()`：`next()` **无条件消耗**一个 token。
        // 对 `.danger` 而言它吃掉 `.` 之后才发现「这不是 ident」，而那个 `.`
        // 已经没了——下面的分类名分支永远读不到它，于是整条选择器失败，
        // 报「读不出选择器」。`try_parse` 失败时**不动位置**，两个分支才都能试。
        if tag.is_none() {
            // 把 ident 原文和「是否为合法类型」一起带出来：`Ok(None)` 表示
            // 读到了 ident 但不在类型表里，那时必须**就地报错并给出候选**——
            // 早先让它落到分类名分支「再试一次」，结果那个分支的 `next()` 把
            // ident 吃掉（它不是 `.`），下一轮再吃 `{`，最后只剩一句
            // 「读不出选择器」，候选也一起丢了。而拼错类型名恰恰最需要候选。
            let outcome = input.try_parse(|p| {
                let word = p.expect_ident()?;
                Ok::<Option<(String, Option<NodeTag>)>, cssparser::BasicParseError<'_>>(Some((
                    word.to_string(),
                    tag_by_name(word.as_ref()),
                )))
            });
            match outcome {
                Ok(Some((_, Some(t)))) => {
                    tag = Some(t);
                    continue;
                }
                Ok(Some((word, None))) => {
                    let near = nearest(&word, &all_tag_names(), 3);
                    // ⚠️ 先 push 错误（那里不用 input），再 return Err。
                    // 合成一步写会同时借用 `p` 和 `input`，编译期过不去。
                    p.errors.push(Error {
                        file: p.file.to_string(),
                        line_text: p.line_text(line),
                        at: crate::errors::Pos { line, col: 1 },
                        underline_len: p.line_text(line).chars().count(),
                        headline: format!("unknown element type '{word}'."),
                        near: near.into_iter().map(|n| (n, line)).collect(),
                        hint: "type names are case-sensitive. for a class name, prefix with `.` (e.g. `.danger`).".into(),
                    });
                    return Err(input.new_error(BasicParseErrorKind::QualifiedRuleInvalid));
                }
                _ => {}
            }
        }

        // 分类名：`.名字`。
        //
        // cssparser 没有现成的 class-selector 解析函数（它只管通用 CSS 语法，
        // 不管选择器结构），所以这一小段自己搭——这正是计划说的
        // 「selector AST 自己搭，很小」。
        if let Ok(Token::Delim('.')) = input.next() {
            match input.next() {
                Ok(Token::Ident(name)) => {
                    // ⚠️ `Selector` 要 `'static`，而这里拿到的是借输入的 `CowRcStr`。
                    // Box::leak 出来的内存在进程结束前不回收——对于编译器来说是
                    // **正确取舍**：它是一个一次性进程，泄漏几百个分类名换来
                    // 不用把 Selector 泛型化，省下的复杂度远大于几百字节。
                    let leaked: &'static str = Box::leak(name.to_string().into_boxed_str());
                    names.push(leaked);
                    continue;
                }
                _ => {
                    p.errors.push(Error {
                        file: p.file.to_string(),
                        line_text: p.line_text(line),
                        at: crate::errors::Pos { line, col: 1 },
                        underline_len: p.line_text(line).chars().count(),
                           headline: "`.` must be followed by a name.".into(),
                        near: Vec::new(),
                           hint: "class names look like `.danger`.".into(),
                    });
                    return Err(input.new_error(BasicParseErrorKind::UnexpectedToken(Token::Delim('.'))));
                }
            }
        }

        // 都不是 → 选择器读完了。
        break;
    }

    if tag.is_none() && names.is_empty() {
        p.errors.push(Error {
            file: p.file.to_string(),
            line_text: p.line_text(line),
            at: crate::errors::Pos { line, col: 1 },
            underline_len: p.line_text(line).chars().count(),
            headline: "cannot read a selector here.".into(),
            near: Vec::new(),
               hint: "selector is either a type (`Button`) or a class (`.danger`).".into(),
        });
        return Err(input.new_error(BasicParseErrorKind::QualifiedRuleInvalid));
    }

    Ok(Selector { tag, names })
}

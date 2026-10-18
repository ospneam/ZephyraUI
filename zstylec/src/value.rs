//! CSS 值 → [`PropertyValue`]。
//!
//! # 支持哪些，全部**显式**列出来
//!
//! | CSS 写法 | 变成 |
//! |---|---|
//! | `12` | `Float(12.0)` |
//! | `12px` | `Float(12.0)`（单位记都不记） |
//! | `#rgb` | `Color` |
//! | `#rrggbb` | `Color` |
//! | `#rrggbbaa` | `Color` |
//! | `transparent` | `Color(0,0,0,0)` |
//! | `@令牌` | `Resource("令牌")` |
//!
//! # 其余一律报错，**不猜**
//!
//! 猜是最坏的选项：`background: 233px` 会变成一个看起来合法的颜色，
//! 而设计者看到的是错的东西。宁可编译不过。

use cssparser::{Parser, Token};
use zephyra_core::core::Color;
use zephyra_core::property::{PropertyKey, PropertyValue};

use super::errors::Error;
use super::parse::ZstyleParser;

/// 解析一个值。出错时把原因记进 `p`，返回 `Err(())`。
///
/// # 为什么返回 `Err(())` 而不是 `Err<Error>`
///
/// 因为错误已经记进去了；再包一层只会让调用方多做一次转发。
pub fn parse(
    input: &mut Parser,
    p: &mut ZstyleParser<'_>,
    line: u32,
    key: &'static PropertyKey,
) -> Result<PropertyValue, ()> {
    // `filter` 是**函数列表**而不是单个 token，必须在取 token 之前分流：
    // `Token::Function` 之后要递归进它的参数解析器，而通用分支只看表层。
    //
    // ⚠ 这条分流必须放在 `input.next()` **之前** —— 一旦取了 token，函数
    // 内部的参数解析器就没法从外层那个 `input` 上正确推进了。
    if std::ptr::eq(key, &zephyra_core::props::FILTER) {
        return parse_filter_chain(input, p, line);
    }
    let value = input.next().map_err(|_: cssparser::BasicParseError| ())?;
    match value {
        // ---- 数字 ----
        Token::Number { value, .. } => Ok(PropertyValue::float(*value as f64 as f32)),
        // `12px` —— Dimension 是「数字 + 单位」，CSS 里 px 等价于我们的 DIP。
        Token::Dimension { value, unit, .. } => {
            let _ = unit;
            Ok(PropertyValue::float(*value as f64 as f32))
        }
        // ---- 颜色：`#rgb` / `#rrggbb` / `#rrggbbaa` ----
        Token::IDHash(h) | Token::Hash(h) => match parse_hex_color(h) {
            Some(c) => Ok(PropertyValue::color(c)),
            None => {
                p.errors.push(unexpected(
                    p,
                    line,
                    &format!("'#{h}' is not a valid color."),
                    &["#rgb", "#rrggbb", "#rrggbbaa"],
                ));
                Err(())
            }
        },
        // ---- 命名颜色 / 关键字 ----
        Token::Ident(word) => {
            let w = word.as_ref();
            if w.eq_ignore_ascii_case("transparent") {
                return Ok(PropertyValue::color(Color::rgba(0.0, 0.0, 0.0, 0.0)));
            }
            // 枚举值：按 key 认。`direction: vertical` / `main-align: center` 一类。
            if let Some(v) = enum_value(key, w) {
                return Ok(v);
            }
            // 命名颜色（red / tomato…）**不做**。理由：整套表约 148 个名字，
            // 而这套框架用 `@` 令牌引用主题色，命名颜色会和令牌体系冲突
            // （到底该写 `red` 还是 `@danger`？）。需要时再加，且只加窄的一批。
            p.errors.push(unexpected(
                p,
                line,
                &format!("unknown value '{w}'."),
                &["transparent", "#rrggbb", "@theme-token"],
            ));
            Err(())
        }
        // ---- 令牌引用：`@control_fill` ----
        Token::AtKeyword(tok) => {
            // 与 `theme::roles::*` 同型：存键而不是字面值，主题切换时只需重解析。
            Ok(PropertyValue::Resource(tok.to_string().into()))
        }
        // ---- 字符串 ----
        Token::QuotedString(s) => Ok(PropertyValue::Str(s.to_string().into())),
        other => {
            p.errors.push(unexpected(
                p,
                line,
                &format!("unexpected here: {other:?}"),
                &["number", "#rrggbb", "@token", "transparent"],
            ));
            Err(())
        }
    }
}

/// 解析 `filter:` 的值：逗号分隔的滤镜函数列表。
///
/// # 支持的写法
///
/// ```text
/// filter: none
/// filter: blur(4px)
/// filter: blur(4px, 6px)
/// filter: refraction(16px, 0.3)
/// filter: blur(3px) refraction(12px, 0.25)
/// filter: drop-shadow(0px 4px 6px #000000a0)
/// ```
///
/// # 为什么用 CSS 的函数语法而不是自定义写法
///
/// `blur(...)` / `drop-shadow(...)` 是 **CSS 原生**的 filter 语法。本仓既然
/// 已经有 `filter` 这个属性名，用同一个语法可以让样式表在别的 CSS 引擎里
/// 大致也能读 —— 而自定义语法只会让"迁移一份样式表"这件事变成逐行重写。
///
/// # `refraction` 是本仓**扩展**，不是 CSS 标准
///
/// 它没有 CSS 对应物，因为主流 CSS 里还没有这一档效果。放在这里而不是另起
/// 一套语法，是为了让它能和 `blur` 混在同一条链里（玻璃面板通常两者都要）。
///
/// # 为什么 `dispersion` 可省
///
/// 只写 `refraction(16px)` 时色散为 0 —— 单通道位移。多数场景够用，且参数
/// 越少越不容易写出"看不出差别"的样式。
fn parse_filter_chain(
    input: &mut Parser,
    p: &mut ZstyleParser<'_>,
    line: u32,
) -> Result<PropertyValue, ()> {
    use zephyra_core::filter::{Filter, FilterChain};

    let mut filters: Vec<Filter> = Vec::new();
    loop {
        // ⚠ token 必须先落进一个**独立变量**再 match。
        //
        // 直接 `match input.next() { ... }` 时，scrutinee 的临时值活到整个
        // match 结束，于是分支里再调 `input.parse_nested_block(...)` 就是
        // 「可变借用 + 不可变借用」⇒ E0499。
        let token = input.next();
        let function_name: Option<String> = match token {
            Ok(Token::Ident(w)) if w.eq_ignore_ascii_case("none") => {
                // `none` 清空已收集的。`filter: blur(2px) none` 按 CSS 是无效的，
                // 这里宽松处理 —— 报错只会让设计者困惑而不会让他改对。
                filters.clear();
                continue;
            }
            // 函数名取**自有**副本：`token` 在下一轮就被丢掉了，而
            // `parse_nested_block` 的闭包还要用它。
            Ok(Token::Function(name)) => Some(name.as_ref().to_ascii_lowercase()),
            // 逗号分隔符本身跳过。
            Ok(Token::Comma) => continue,
            // 其余（含闭括号）= 列表结束。空白不是 token（cssparser 默认跳过）。
            Ok(_) | Err(_) => break,
        };
        let Some(lower) = function_name else {
            p.errors.push(unexpected(
                p,
                line,
                "expected a filter function here.",
                &[
                    "blur(4px)",
                    "refraction(24px, 20px, 0.3, 1.5)",
                    "drop-shadow(0px 4px 6px #000000a0)",
                    "none",
                ],
            ));
            return Err(());
        };
        // ⚠ `parse_nested_block` 是**解析器**上的方法，不是函数名上的 ——
        // 它负责把「已经进入 `(` 之后」这个状态传给闭包，并吃掉闭括号。
        // 直接在 `input.next()` 上读参数会立刻拿到 `Err`，于是所有函数都被
        // 判成「参数缺失」—— 症状是每一条 filter 规则都报错，而函数名明明对。
        let got = input.parse_nested_block(|args| {
            // 闭包返回 `ParseError<()>`（而不是 `Result<_, ()>`）—— cssparser
            // 的 HRTB 要求错误类型与它自己的类型一致。这里从不用它报错：
            // 解析失败一律回 `None`，由调用方统一生成**带提示**的那条错误。
            Ok::<_, cssparser::ParseError<()>>(filter_by_name(&lower, args))
        });
        match got {
            Ok(Some(f)) => filters.push(f),
            _ => {
                p.errors.push(unexpected(
                    p,
                    line,
                    &format!("unknown or malformed filter '{lower}(...)'."),
                    &[
                        "blur(4px)",
                        "blur(4px, 6px)",
                        "refraction(16px, 0.3)",
                        "drop-shadow(0px 4px 6px #000000a0)",
                        "none",
                    ],
                ));
                return Err(());
            }
        }
    }
    Ok(PropertyValue::filter(FilterChain::new(filters)))
}

/// 折射倒角带的默认宽度（像素）。
///
/// 20px 的选择不是一个美术值，而是“能看见弯曲、但不超过圆角半径”的上限：
/// 倒角带超过圆角半径时，圆角区也会被拉入算出，视觉上就变成了“整块玻璃”
/// 而不是“倒角”。这个值的上限在着色器里又用 `min(radius, half_size)` 取了一次。
const DEFAULT_BEZEL_PX: f32 = 20.0;

/// 默认折射率（普通冠牌玻璃约 1.5）。
const DEFAULT_IOR: f32 = 1.5;

/// 按函数名解析一个滤镜的参数（`args` 已在函数体内）。
fn filter_by_name(name: &str, args: &mut Parser) -> Option<zephyra_core::filter::Filter> {
    use zephyra_core::core::{Color, Point};
    use zephyra_core::filter::Filter;

    let mut nums: Vec<f32> = Vec::new();
    let mut col: Option<Color> = None;
    loop {
        match args.next() {
            // `12` 与 `12px` 都收 —— 本仓 1 布局单位 = 1 DIP，px 即 DIP。
            Ok(Token::Number { value, .. }) => nums.push(*value),
            Ok(Token::Dimension { value, .. }) => nums.push(*value),
            Ok(Token::Percentage { unit_value, .. }) => nums.push(*unit_value),
            Ok(Token::IDHash(h) | Token::Hash(h)) => col = parse_hex_color(h),
            Ok(Token::Comma) => {}
            // 闭括号 / 出错 / 任何别的东西都停。
            _ => break,
        }
    }
    let dim = |i: usize| nums.get(i).copied();
    match name {
        // 一个数 = 各向同性；两个 = x / y 分别。
        "blur" => match (dim(0), dim(1)) {
            (Some(x), Some(y)) => Some(Filter::Blur {
                sigma_x: x,
                sigma_y: y,
            }),
            (Some(x), None) => Some(Filter::Blur {
                sigma_x: x,
                sigma_y: x,
            }),
            _ => None,
        },
        // `refraction(厚度, 倒角宽?, 色散?, 折射率?)`
        //
        // 三个可选参数**都有默认值**，而不是只允许省略。原因是它们形成一个
        // **完整的算法**（见 `zephyra.hlsl` 的 `PSRefraction`）：厚度与倒角宽合起来决定面凸度，
        // 折射率决定光线偏转的角度。只给一个数就起不了算式。
        //
        // 默认值选的是普通皃封款直角几何形状：倒角宽 = 深度 20px、折射率 1.5。
        "refraction" => dim(0).map(|strength| Filter::Refraction {
            strength,
            bezel: dim(1).unwrap_or(DEFAULT_BEZEL_PX),
            dispersion: dim(2).unwrap_or(0.0),
            ior: dim(3).unwrap_or(DEFAULT_IOR),
        }),
        // CSS 顺序是 `offset-x offset-y blur-radius color`。
        // 至少要两个数（偏移），颜色缺省给一个能看见的。
        "drop-shadow" if nums.len() >= 2 => Some(Filter::DropShadow {
            color: col.unwrap_or(Color::rgba(0.0, 0.0, 0.0, 0.4)),
            offset: Point::new(nums[0], nums[1]),
            sigma: dim(2).unwrap_or(0.0),
        }),
        _ => None,
    }
}

/// 把关键字按属性翻成对应的枚举值。
///
/// # 为什么按 key 认，而不是全局认
///
/// 同一个词在不同属性里含义不同：`center` 在 `main-align` 是居中，
/// 在 `cross-align` 也是居中（目前巧合相同），但 `horizontal` 在 `direction`
/// 是横向排布，在别处没有意义。全局认会把「非法组合」静默变成合法值。
///
/// 目前只覆盖**真的出现在样式表里的枚举属性**。加属性时往这里加一行，
/// 不在这张表里的属性收到关键字就报错。
fn enum_value(key: &PropertyKey, word: &str) -> Option<PropertyValue> {
    use zephyra_core::layout::{AlignItems, Direction, MainAxisAlign};
    let eq = |s: &str| s.eq_ignore_ascii_case(word);

    if key.name() == "Direction" {
        if eq("horizontal") {
            return Some(PropertyValue::direction(Direction::Horizontal));
        }
        if eq("vertical") {
            return Some(PropertyValue::direction(Direction::Vertical));
        }
        return None;
    }
    if key.name() == "CrossAlign" {
        if eq("start") {
            return Some(PropertyValue::cross_align(AlignItems::Start));
        }
        if eq("center") {
            return Some(PropertyValue::cross_align(AlignItems::Center));
        }
        if eq("end") {
            return Some(PropertyValue::cross_align(AlignItems::End));
        }
        if eq("stretch") {
            return Some(PropertyValue::cross_align(AlignItems::Stretch));
        }
        return None;
    }
    if key.name() == "MainAlign" {
        if eq("start") {
            return Some(PropertyValue::main_align(MainAxisAlign::Start));
        }
        if eq("center") {
            return Some(PropertyValue::main_align(MainAxisAlign::Center));
        }
        if eq("end") {
            return Some(PropertyValue::main_align(MainAxisAlign::End));
        }
        if eq("space-between") {
            return Some(PropertyValue::main_align(MainAxisAlign::SpaceBetween));
        }
        return None;
    }
    None
}

/// 造一条「值不对」的错误。
fn unexpected(p: &ZstyleParser<'_>, line: u32, msg: &str, want: &[&str]) -> Error {
    Error {
        file: p.file.to_string(),
        line_text: p.line_text(line),
        at: crate::errors::Pos { line, col: 1 },
        underline_len: p.line_text(line).chars().count(),
        headline: format!("property {msg}"),
        near: Vec::new(),
        hint: format!(
            "this property accepts: {}.",
            if want.is_empty() {
                "see props_map.rs".to_string()
            } else {
                want.join(" / ")
            }
        ),
    }
}

/// `#rgb` / `#rrggbb` / `#rrggbbaa` → `Color`。
fn parse_hex_color(hex: &str) -> Option<Color> {
    let h = hex.trim_start_matches('#');
    let byte = |s: &str| u8::from_str_radix(s, 16).ok();
    match h.len() {
        3 => {
            let r = u32::from_str_radix(&h[0..1], 16).ok()?;
            let g = u32::from_str_radix(&h[1..2], 16).ok()?;
            let b = u32::from_str_radix(&h[2..3], 16).ok()?;
            // `#abc` 的每一位要摊成两位（a → aa）。
            Some(Color::rgba(
                (r * 17) as f32 / 255.0,
                (g * 17) as f32 / 255.0,
                (b * 17) as f32 / 255.0,
                1.0,
            ))
        }
        6 => {
            let _ = byte;
            Some(Color::rgba(
                u32::from_str_radix(&h[0..2], 16).ok()? as f32 / 255.0,
                u32::from_str_radix(&h[2..4], 16).ok()? as f32 / 255.0,
                u32::from_str_radix(&h[4..6], 16).ok()? as f32 / 255.0,
                1.0,
            ))
        }
        8 => Some(Color::rgba(
            u32::from_str_radix(&h[0..2], 16).ok()? as f32 / 255.0,
            u32::from_str_radix(&h[2..4], 16).ok()? as f32 / 255.0,
            u32::from_str_radix(&h[4..6], 16).ok()? as f32 / 255.0,
            u32::from_str_radix(&h[6..8], 16).ok()? as f32 / 255.0,
        )),
        _ => None,
    }
}

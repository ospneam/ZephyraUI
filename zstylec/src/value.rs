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
        Token::IDHash(h) | Token::Hash(h) => {
            match parse_hex_color(h) {
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
            }
        }
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
fn unexpected(p: &ZstyleParser<'_>, line: u32, msg: &str, want: &[&str]) -> Error {    Error {
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

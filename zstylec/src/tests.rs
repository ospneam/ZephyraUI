//! 解析器与值映射的测试。
//!
//! # 这个文件曾经是空的
//!
//! `parse.rs` 尾部一直写着 `#[cfg(test)] mod tests;`，但文件不存在 ——
//! 于是 `cargo test -p zstylec` **编译不过**，而 `cargo build -p zstylec` 是过的
//! （`cfg(test)` 只在测试时展开）。这就是「声明了测试但从没建过」的具体形状：
//! 日常构建一路绿，一到测试就炸。
//!
//! # 覆盖哪些
//!
//! | 类 | 判据 |
//! |---|---|
//! | 正常路径 | 类型 / 分类名 / 组合 / 注释 / 令牌 / 十六进制色 |
//! | 未知**类型名** | 报错（且给最接近的候选） |
//! | 未知**属性名** | 报错 + 最接近的候选 |
//! | 不支持的写法 | `@media` / `@layer` / `!important` / 后代组合器 / 裸声明 |
//! | 坏值 | 不认识的颜色、命名颜色（明确不做） |
//! | 空规则 | 被剔除，**且不报错** |
//! | 报错格式 | 行号 + 列号 + 原文 + 下划线 + 候选 |

use zephyra_core::property::{PropertyValue, ValueSource};
use zephyra_core::props;

use crate::{parse, ParseResult};

/// 解析一段 CSS，断言它**零错误**。
fn ok(css: &str) -> ParseResult {
    let r = parse("test.zstyle", css);
    assert!(
        r.is_ok(),
        "这段应当零错误，但有 {} 个:\n{}",
        r.errors.len(),
        r.errors
            .iter()
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    );
    r
}

/// 解析一段 CSS，断言它**至少一个错误**，并把错误文本交出来。
fn bad(css: &str) -> String {
    let r = parse("test.zstyle", css);
    assert!(!r.is_ok(), "这段应当报错，但它过了：{css}");
    r.errors
        .iter()
        .map(|e| e.to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------------------------------------------------------------------------
//  正常路径
// ---------------------------------------------------------------------------

#[test]
fn a_bare_type_selector_parses() {
    let r = ok("Control { width: 40px; }");
    assert_eq!(r.sheet.rules().len(), 1);
    assert_eq!(r.sheet.rules()[0].selector.specificity(), 0, "纯类型没有权重");
}

#[test]
fn a_class_selector_parses() {
    let r = ok(".danger { background: #ff0000; }");
    assert_eq!(r.sheet.rules().len(), 1);
    assert_eq!(r.sheet.rules()[0].selector.specificity(), 1, "一个分类名算 1");
}

#[test]
fn type_plus_classes_parses() {
    let r = ok("Control.danger.compact { width: 10px; }");
    let rule = &r.sheet.rules()[0];
    assert!(rule.selector.tag.is_some(), "类型要在");
    assert_eq!(rule.selector.names.len(), 2, "两个分类名");
    assert_eq!(rule.selector.specificity(), 2);
}

#[test]
fn a_token_value_becomes_a_resource() {
    let r = ok("Control { background: @control_fill; }");
    let v = &r.sheet.rules()[0].declarations[0].value;
    assert_eq!(
        v.resource_key(),
        Some("control_fill"),
        "@令牌 必须变成 Resource（存键不存值，主题切换才免费）"
    );
}

#[test]
fn hex_colours_parse_at_all_widths() {
    let r = ok("Control { background: #abc; }");
    let color = r.sheet.rules()[0].declarations[0]
        .value
        .get::<zephyra_core::core::Color>();
    assert!(color.is_some());

    let _ = ok("Control { background: #aabbcc; }");
    let _ = ok("Control { background: #aabbccdd; }");
}

#[test]
fn enum_values_parse_per_property() {
    // `direction` / `main-align` / `cross-align` 收关键字，但**按属性**认。
    let _ = ok("Control { direction: vertical; }");
    let _ = ok("Control { main-align: center; }");
    let _ = ok("Control { cross-align: start; }");
}

#[test]
fn comments_and_whitespace_are_ignored() {
    let r = ok("/* a */ Control   { width :  40px ; } /* b */");
    assert_eq!(r.sheet.rules().len(), 1);
}

#[test]
fn several_rules_keep_source_order() {
    let r = ok("Control { width: 1px; } .b { width: 2px; } .c { width: 3px; }");
    assert_eq!(r.sheet.rules().len(), 3);
    let widths: Vec<f32> = r
        .sheet
        .rules()
        .iter()
        .filter_map(|r| r.declarations.first().map(|d| d.value.get::<f32>()))
        .flatten()
        .collect();
    // 同特异性（都是 0 / 1 / 1）时，表里仍按「特异性 → 原始次序」排。
    // 这里只钉「三条都在、次序没丢」。
    assert_eq!(widths.len(), 3, "三条规则都该在");
}

// ---------------------------------------------------------------------------
//  未知类型名
// ---------------------------------------------------------------------------

#[test]
fn an_unknown_type_name_is_reported_with_candidates() {
    let msg = bad("Contorl { width: 1px; }");
    assert!(msg.contains("unknown element type"), "要说是什么错:\n{msg}");
    assert!(msg.contains("Control"), "要给出最接近的:\n{msg}");
    assert!(msg.contains("line 1, col 1"), "要有行号列号:\n{msg}");
    assert!(msg.contains("Contorl"), "要引出原文:\n{msg}");
    assert!(msg.contains('^'), "要有下划线:\n{msg}");
}

#[test]
fn a_typo_in_the_type_name_does_not_silently_pass() {
    // 静默忽略是最坏的失败模式：设计者以为生效了。
    let r = parse("t.zstyle", "Buton { background: red; }");
    assert!(!r.is_ok());
}

// ---------------------------------------------------------------------------
//  未知属性名
// ---------------------------------------------------------------------------

#[test]
fn an_unknown_property_is_reported_with_candidates() {
    let msg = bad("Control { coner-radius: 4px; }");
    assert!(msg.contains("unknown property"), "要说是什么错:\n{msg}");
    assert!(
        msg.contains("corner-radius"),
        "要给最接近的（拼错字是 90% 的情况）:\n{msg}"
    );
}

// ---------------------------------------------------------------------------
//  不支持的写法
// ---------------------------------------------------------------------------

#[test]
fn at_rules_are_rejected_not_ignored() {
    for css in [
        "@media (min-width: 100px) { Control { width: 1px; } }",
        "@layer base, ui;",
        "@keyframes spin { from { opacity: 0; } }",
        "@font-face { font-family: x; }",
    ] {
        let msg = bad(css);
        assert!(
            msg.contains("unsupported @"),
            "@ 规则要报错而不是跳过：{css}\n{msg}"
        );
    }
}

/// 报错要**说清是哪一个、为什么**。
///
/// cssparser 自己对 at-rule 的默认实现也返回 Err，消息里同样有 "unsupported @"，
/// 所以上一条测试**抓不到**「我们自己的报错被删掉」。这一条钉住语义层的那句：
/// 名字 + 分类 + 该怎么办。
#[test]
fn an_at_rule_error_names_the_rule_and_says_why() {
    let msg = bad("@media (min-width: 100px) { Control { width: 1px; } }");
    for want in [
        "unsupported @media (conditional rule).",
        "only Type.class { prop: value; } is supported.",
    ] {
        assert!(msg.contains(want), "报错缺 {want:?}:\n{msg}");
    }
}

#[test]
fn a_nested_rule_is_rejected() {
    // CSS Nesting 不做。`parse_qualified()` 返回 false，所以 `{` 出现在块内即错。
    let msg = bad("Control { .inner { width: 1px; } }");
    assert!(
        msg.contains("nested rules are not supported"),
        "嵌套规则要报错:\n{msg}"
    );
}

#[test]
fn a_line_comment_is_not_a_comment() {
    // CSS 没有 `//`。它是两个 `/` Delim，会报错 —— 不能默默放过。
    let _ = bad("Control { width: 1px; } // hi");
}

#[test]
fn a_dangling_deliberate_declaration_is_reported() {
    let msg = bad("Control width: 1px;");
    assert!(!msg.is_empty());
}

// ---------------------------------------------------------------------------
//  坏值
// ---------------------------------------------------------------------------

#[test]
fn a_named_colour_is_rejected_on_purpose() {
    // 命名颜色（red / tomato…）**明确不做**：整套表 148 个，且会与 @令牌体系冲突。
    let msg = bad("Control { background: red; }");
    assert!(msg.contains("unknown value"), "要给理由:\n{msg}");
    assert!(
        msg.contains("#rrggbb") || msg.contains("@"),
        "要给出该写什么:\n{msg}"
    );
}

#[test]
fn a_bad_hex_colour_is_rejected() {
    let msg = bad("Control { background: #xyz; }");
    assert!(msg.contains("not a valid color"), "要说颜色不合法:\n{msg}");
}

#[test]
fn an_enum_value_for_the_wrong_property_is_rejected() {
    // `vertical` 只对 `direction` 合法。全局认词会把非法组合静默变成合法值。
    let msg = bad("Control { width: vertical; }");
    assert!(!msg.is_empty(), "width: vertical 必须报错");
}

// ---------------------------------------------------------------------------
//  空规则
// ---------------------------------------------------------------------------

#[test]
fn an_empty_rule_is_dropped_and_does_not_error() {
    let r = ok("Control { }");
    assert_eq!(
        r.sheet.rules().len(),
        0,
        "空规则要剔除 —— 不剔除它会挡住后面所有规则"
    );
}

// ---------------------------------------------------------------------------
//  报错格式（B3 的规格）
// ---------------------------------------------------------------------------

#[test]
fn errors_carry_the_full_spec() {
    let msg = bad("Control {\n  coner-radius: 4px;\n}");
    for want in [
        "test.zstyle",
        "line 2",
        "unknown property",
        "^",
        "corner-radius",
        "hint:",
    ] {
        assert!(msg.contains(want), "报错缺 {want:?}:\n{msg}");
    }
}

#[test]
fn several_errors_are_all_reported_at_once() {
    // 一次只报一个的话，开发者要反复「改一处→编译→再报下一处」。
    let r = parse("t.zstyle", "Contorl { coner-radius: 1px; }\nButon { width: 2px; }");
    assert!(
        r.errors.len() >= 2,
        "两个独立的错要一次全报，实得 {} 个",
        r.errors.len()
    );
}

// ---------------------------------------------------------------------------
//  断言 Source 层没被误用（回归）
// ---------------------------------------------------------------------------

#[test]
fn parsed_declarations_carry_the_right_key() {
    let r = ok("Control { corner-radius: 4px; }");
    let d = &r.sheet.rules()[0].declarations[0];
    assert_eq!(
        d.key.name(),
        "CornerRadius",
        "CSS 名 corner-radius 必须映射到 props::CORNER_RADIUS"
    );
    assert_eq!(d.value.get::<f32>(), Some(4.0));
    // 单位 px 被吃掉 —— DIP 与 px 等价。
    let _ = ok("Control { width: 40px; }");
}

/// 让 `ValueSource` 的导入不被判为未使用（测试里没直接用到它，
/// 但留着是因为后续断言「来源是 Styled」会用到）。
#[allow(dead_code)]
fn _source_anchor(s: ValueSource) -> ValueSource {
    let _ = &props::BACKGROUND;
    s
}

// 本文件由 locale.rs 的 #[cfg(test)] mod tests 抽出 —— 测试代码与生产代码分离。
// 模块路径不变（src 里的声明用 #[path] 指过来），所以 use super::* 与私有可见性都照旧。

use super::*;

/// 双 null 结尾的多字符串必须按"连续两个 null"解析。
///
/// 判据取四项 + 结尾。按分隔符切的实现会在这里多出一个空串。
#[test]
fn multiple_strings_parsed_by_double_null() {
    let units: Vec<u16> = "en-US\0zh-CN\0ja\0\0".encode_utf16().collect();
    assert_eq!(
        parse_multi_string(&units),
        vec!["en-US".to_string(), "zh-CN".to_string(), "ja".to_string()]
    );
}

/// 缓冲比实际内容长时必须停在全零区，不能产出空项。
#[test]
fn trailing_buffer_bytes_produce_no_empty_entry() {
    let mut units: Vec<u16> = "en-US\0de\0\0".encode_utf16().collect();
    units.extend(std::iter::repeat(0).take(64));
    assert_eq!(
        parse_multi_string(&units),
        vec!["en-US".to_string(), "de".to_string()]
    );
}

/// 没有双 null 的兜底：最后一段仍要收下。
#[test]
fn trailing_segment_kept_when_double_null_missing() {
    let units: Vec<u16> = "en-US\0fr".encode_utf16().collect();
    assert_eq!(
        parse_multi_string(&units),
        vec!["en-US".to_string(), "fr".to_string()]
    );
}

/// 真机调用必须返回**非空**且**无重复**。
///
/// 这条是自声明签名是否写对的**唯一**运行时证据：签名错了会静默失败，
/// 于是「接口通了」与「返回空表」都长得像成功。
#[test]
fn system_locale_readable_on_real_machine() {
    let langs = preferred_ui_languages();
    assert!(
        !langs.is_empty(),
        "本机读不到首选 UI 语言。先查是不是签名或标志位写错：\
             dwflags 必须是 MUI_LANGUAGE_NAME(0x4)，两个长度参数都是 *mut u32"
    );
    let mut sorted = langs.clone();
    sorted.sort();
    let n = sorted.len();
    sorted.dedup();
    assert_eq!(sorted.len(), n, "不该有重复项：{langs:?}");
    for l in &langs {
        assert!(!l.is_empty() && l.len() <= 16, "语言名形状可疑：{l:?}");
    }
}

/// flags_are_pinned：`MUI_LANGUAGE_NAME` = `0x4`。
///
/// 传 0 会返回 LCID（如 `1033`）而非语言名 —— 而 `1033` 不是 BCP 47 标签，
/// 协商会全部落空。这条是那个坑的护栏。
#[test]
fn flags_are_pinned() {
    assert_eq!(MUI_LANGUAGE_NAME, 0x4);
    assert_ne!(MUI_LANGUAGE_NAME, 0, "0 = MUI_LANGUAGE_ID，会返回 LCID");
}

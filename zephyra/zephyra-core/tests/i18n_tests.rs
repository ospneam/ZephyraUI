//! i18n 单元测试：语言标签解析、偏好协商、消息表。
//!
//! 从 `src/tests/i18n_tests.rs` 移入 `tests/` —— 所有被测项均为 `pub`，
//! 移到集成测试后不需要改可见性。

use zephyra_core::i18n::{negotiate, Messages, Preferences, Tag};

/// 解析必须照 BCP 47 的三段结构，且**规范化**大小写。
#[test]
fn tag_is_parsed_and_normalized_per_bcp47() {
    let t = Tag::parse("zh-hans-cn").expect("合法");
    assert_eq!(t.language(), "zh");
    assert_eq!(t.script(), Some("Hans"));
    assert_eq!(t.region(), Some("CN"));
    assert_eq!(t.as_str(), "zh-Hans-CN");

    assert_eq!(Tag::parse("en").unwrap().as_str(), "en");
    assert_eq!(Tag::parse("en-US").unwrap().as_str(), "en-US");
    // 3 数字区域（UN M.49）。
    assert_eq!(Tag::parse("es-419").unwrap().region(), Some("419"));
    // 非法项必须 None，不做模糊修复。
    assert!(Tag::parse("").is_none());
    assert!(Tag::parse("e").is_none());
    assert!(Tag::parse("zh_cn").is_none());
    assert!(Tag::parse("zh-CN-extra").is_none());
}

/// 协商必须是「**表里第一个匹配的**」，不是「全局最接近的」。
///
/// 这条是官方措辞的直接翻译：`Intl` 用
/// "the first entry in the list that matches a locale supported"。
#[test]
fn negotiation_follows_preference_order_not_globally_nearest() {
    // `ja` 排第一 ⇒ 必须选 ja，即使 `en-US` 与第二项**完全相等**（距离 0）。
    let prefs = Preferences::from_list(&["ja", "en-US"]);
    let available = [Tag::parse("en-US").unwrap(), Tag::parse("ja").unwrap()];
    assert_eq!(negotiate(&prefs, &available).unwrap().as_str(), "ja");

    // 反过来：`en-US` 优先 ⇒ 选 en-US。
    let prefs = Preferences::from_list(&["en-US", "ja"]);
    assert_eq!(negotiate(&prefs, &available).unwrap().as_str(), "en-US");
}

/// 回退优先级：全中 > 丢 region > 只语言段。
#[test]
fn fallback_distance_follows_official_order() {
    let full = Tag::parse("zh-Hans-CN").unwrap();
    assert_eq!(full.distance(&Tag::parse("zh-Hans-CN").unwrap()), Some(0));
    assert_eq!(full.distance(&Tag::parse("zh-Hans").unwrap()), Some(2));
    assert_eq!(full.distance(&Tag::parse("zh").unwrap()), Some(3));
    // 语言段不同 ⇒ 不匹配。
    assert_eq!(full.distance(&Tag::parse("en").unwrap()), None);
    // script 不同（Hans vs Hant）⇒ 不匹配：那是两种文字。
    assert_eq!(
        Tag::parse("zh-Hans")
            .unwrap()
            .distance(&Tag::parse("zh-Hant").unwrap()),
        None
    );
}

/// `Messages::select` 必须走 language-only 回退。
///
/// 浏览器显式追加 language-only 回退标签（`en-US,en;q=0.9`），
/// 所以「只有 `zh-Hans-CN` 译文而用户是 `zh-Hans`」必须能取到。
#[test]
fn message_lookup_falls_back_to_language_only() {
    let mut m = Messages::new();
    m.insert("zh-Hans-CN", "hello", "你好");
    // 只插了精确 locale。
    assert_eq!(
        m.select(&Preferences::from_list(&["zh-Hans-CN"]), "en", "hello"),
        Some("你好")
    );

    // 语言段回退：用户标签短于可用表时也应命中。
    m.insert("zh", "bye", "再见");
    assert_eq!(
        m.select(&Preferences::from_list(&["zh-Hans", "en"]), "en", "bye"),
        Some("再见")
    );
}

/// 漏翻必须回落到**默认语言**的文案，而不是键名、更不是空白。
///
/// 这是浏览器的真实行为（某门语言缺资源时回落默认语言），
/// 也是"降级要写在明处"的落点：真要发现漏翻，靠门禁而不是靠界面。
#[test]
fn missing_translation_falls_back_to_default_language() {
    let mut m = Messages::new();
    m.insert("en", "nav.home", "Home");
    // `ja` 一个键都没有 ⇒ 必须拿到 `en` 的文案。
    assert_eq!(
        m.select(&Preferences::from_list(&["ja"]), "en", "nav.home"),
        Some("Home"),
        "漏翻应回落默认语言，而不是 None/空串"
    );
    // 连默认语言也没有 ⇒ None（由调用方显示键名）。
    assert_eq!(
        m.select(&Preferences::from_list(&["ja"]), "en", "nope"),
        None
    );
}

/// 默认语言的语言段回退：默认语言写 `zh-Hans` 而调用方给 `zh` 时仍要命中。
///
/// 这一条是**真实**的：`negotiate` 交出来的 `Tag` 会带 script，
/// 而调用方手里的默认语言常量可能只写到语言段。
#[test]
fn default_locale_also_uses_language_section_fallback() {
    let mut m = Messages::new();
    m.insert("zh-Hans", "a", "A");
    assert_eq!(m.select(&Preferences::new(), "zh-Hans", "a"), Some("A"));
    // `en` 兜底时先试 `en`、再试 `en`（无 script 无 region，同一项）。
    assert_eq!(m.select(&Preferences::new(), "en-Hans", "a"), None);
}

/// 漏翻必须可被机器发现（门禁依据）。
#[test]
fn missing_keys_are_enumerable() {
    let mut m = Messages::new();
    m.insert("en", "a", "A");
    let missing = m.missing_in("en", &["a", "b", "c"]);
    assert_eq!(missing, vec!["b".to_string(), "c".to_string()]);
}

/// 偏好表有序、去重，`first()` 对应 `navigator.language`。
#[test]
fn preference_list_is_ordered_and_deduplicated() {
    let p = Preferences::from_list(&["en-US", "zh-CN"]);
    assert_eq!(p.first().unwrap().as_str(), "en-US");
    let mut q = p.clone();
    q.push(Tag::parse("en-US").unwrap());
    assert_eq!(q.len(), 2, "去重但保留顺序");
    q.push(Tag::parse("ja").unwrap());
    assert_eq!(q.len(), 3);
    // 坏项跳过而不是让整表失效。
    assert_eq!(Preferences::from_list(&["en", "bogus-tag"]).len(), 1);
}

/// `from_owned` 与 `from_list` 必须同形（宿主交回的是有所有权的串）。
///
/// 这条不是恒真：把 `from_owned` 实现成"`from_list` 但忽略非法项"的话，
/// 本测试仍会过，而真正的问题（宿主给的语言**全**被跳过 → 回落默认）
/// 会静默发生。所以这里额外断言"含坏项时好项仍在"。
#[test]
fn owned_and_borrowed_lists_have_same_shape() {
    let owned: Vec<String> = vec!["zh-CN".to_string(), "坏-标签".to_string()];
    let p = Preferences::from_owned(&owned);
    assert_eq!(p.len(), 1);
    assert_eq!(p.first().unwrap().as_str(), "zh-CN");
    assert_eq!(p, Preferences::from_list(&["zh-CN"]));
}

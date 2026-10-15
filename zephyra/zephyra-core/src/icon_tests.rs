//! 图标注册表的行为断言。
//!
//! 这批测试盯的是**注册表**这一层——解析器自身的断言随 `path.rs` / `svg.rs`
//! 一起迁过来了（23 条）。这里关心的是注册表特有的、且容易出错的地方：覆盖语义、
//! 未知名字的回落、以及"身份必须稳定"这条被缓存依赖的前提。

use crate::icon::{assets, Icon, IconSet, IconSource, Paint, Segment};

// ============================== 通用字形 ==============================

#[test]
fn every_builtin_asset_parses() {
    // 这条是最有价值的一条：13 个 SVG 是编译期内嵌的常量，若其中任何一个写坏了
    // path data 或 viewBox，`freeze()` 会在启动时 panic——而不是等到用户点开
    // 那个界面才发现图标不见了。
    //
    // 用 `Box::leak` 造一个 `'static`：注册表本来就只该以 `'static` 形态存在
    // （见 `IconSet::get` 的签名说明——缓存以图标数据地址为键）。
    let set: &'static IconSet = Box::leak(Box::new(IconSet::with_builtin()));
    let names = set.names();
    assert_eq!(names.len(), assets::REGISTRY.len());
    for name in names {
        let icon = set.get(name);
        assert!(!icon.is_empty(), "内置图标 {name} 解析后没有任何可填充内容");
        assert!(
            icon.view_box.2 > 0.0 && icon.view_box.3 > 0.0,
            "内置图标 {name} 的 viewBox 宽高必须为正：{:?}",
            icon.view_box
        );
    }
}

#[test]
fn a_builtin_name_resolves_to_a_distinct_icon() {
    let set: &'static IconSet = Box::leak(Box::new(IconSet::with_builtin()));
    // `close` 在资源文件里叫 `dismiss.svg`。这条钉住"名字是调用方的词汇，
    // 不必与文件名一致"——否则有人按文件名去查注册表就会以为少了���个图标。
    assert!(!set.get("close").is_empty());
    // 形状不同：两条不同路径不该解析出同一段数据。
    assert_ne!(set.get("pen").segments, set.get("eraser").segments);
}

#[test]
fn path_sources_use_a_24x24_view_box() {
    // 裸 path data 没有 viewBox，约定按 0 0 24 24 处理。与 SVG 分支必须同源，
    // 否则同一份 path data 走两条路会得到不同缩放。
    let set: &'static IconSet = Box::leak(Box::new(
        IconSet::new().with("x", IconSource::Path("M0 0L24 0L24 24Z")),
    ));
    let icon = set.get("x");
    assert_eq!(icon.view_box, (0.0, 0.0, 24.0, 24.0));
    assert_eq!(icon.paint, Paint::CURRENT_COLOR);
    assert_eq!(icon.segments.len(), 4, "M + L + L + Z");
    assert!(matches!(icon.segments[3], Segment::Close));
}

// ============================== 覆盖与回落 ==============================

#[test]
fn a_later_registration_overrides_an_earlier_one() {
    // 应用可以覆盖框架自带字形（比如换一套更符合品牌规范的 menu），而不必先删。
    // 取"第一个"会让 `with` 的语义变成"只在没被占用时生效"，那与追加式 API 的
    // 直觉相反。
    let set: &'static IconSet = Box::leak(Box::new(
        IconSet::with_builtin().with("menu", IconSource::Path("M0 0L1 0L1 1Z")),
    ));
    let overridden = set.get("menu");
    assert_eq!(overridden.segments.len(), 4, "应解析成新的那段 path data");
    assert_eq!(set.names().iter().filter(|n| **n == "menu").count(), 1);
}

#[test]
fn an_unknown_name_falls_back_to_the_shared_empty_icon() {
    let set: &'static IconSet = Box::leak(Box::new(IconSet::with_builtin()));
    let a = set.get("no_such_icon");
    let b = set.get("another_missing_one");
    assert!(a.is_empty());
    // 必须是**同一个**实例：栅格化缓存以数据地址为键，而"所有未知名字共享一个
    // 空图标"意味着一个未知名字只占一个（永远填不上内容的）缓存条目。
    assert_eq!(
        std::ptr::from_ref(a),
        std::ptr::from_ref(b),
        "未知名字应回落到同一个 static"
    );
    // 刻意**不**断言 `a` 等于 `&Icon::EMPTY`：`Icon::EMPTY` 是 const，
    // 每次取地址都被常量提升成新的匿名分配，与 crate 内部那个 `static EMPTY`
    // 不是同一个对象。要断言"共享"只能比较两次 `get` 的结果（上面那条）。
    let _ = Icon::EMPTY;
}

#[test]
fn an_empty_icon_is_reported_as_empty() {
    // `fill="none"` 的图标与未知名字都必须走"不画"而不是"画一块空白纹理"。
    // 判据收敛到一个函数上，免得渲染端各写一套。
    let set: &'static IconSet =
        Box::leak(Box::new(IconSet::new().with("blank", IconSource::Path(""))));
    assert!(set.get("blank").is_empty());
}

// ============================== 解析时机 ==============================

#[test]
fn freezing_early_means_the_first_query_does_no_work() {
    // 解析耗时要落在启动里而不是第一帧——这是 `freeze` 存在的全部理由。
    let set: &'static IconSet = Box::leak(Box::new(IconSet::with_builtin()));
    set.freeze();
    // 二次 freeze 不 panic（OnceLock 幂等）。
    set.freeze();
    assert!(!set.get("pen").is_empty());
}

#[test]
#[should_panic(expected = "解析失败")]
fn a_broken_icon_panics_rather_than_vanishing() {
    // 后果是"某个图标不显示"，而没有任何其他症状会指向解析器——那是最难查的一类
    // 故障。注册表是编译期常量，解析失败意味着某个写死的资源坏了，属于必须
    // 当场暴露的构建缺陷。
    let set: &'static IconSet = Box::leak(Box::new(
        IconSet::new().with("broken", IconSource::Path("M0 0 Q")),
    ));
    let _ = set.get("broken");
}

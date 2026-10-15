//! hover / press 改**尺寸**时必须重排，而不只是重绘。
//!
//! # 这条测试回答的是「为什么只有 resize 以后布局才更新」
//!
//! `FrameCache::frame` 的重排判据原先只有三项：首帧、视口变了、`diff` 说要重排。
//! 而 `apply` 排在 diff **之后**（`zephyra-style/src/apply.rs` 的硬契约），
//! 所以样式层写出的布局键 —— hover / press 改 `padding` 就是最典型的一种 ——
//! 标了 `Dirty::Layout` 却没人读：只重绘、不重排。
//!
//! 鼠标移动只翻分类名，于是这条路径**只能**靠 resize 或整树改动触发。
//! 修法是把判据换成直接读树上的脏标记（`frame.rs` 的 `tree_needs_layout`）。
//!
//! # 判据为什么取矩形
//!
//! 命令数在旧代码下也会变（重绘发生了），分不出「重排了没有」。
//! 矩形是 layout 的产物，只有真重排才会变。

use std::sync::Arc;

use zephyra_core::context::Context;
use zephyra_core::element::Element;
use zephyra_core::frame::{FrameCache, FrameOutcome};
use zephyra_core::id::NodeTag;
use zephyra_core::node::Node;
use zephyra_core::property::PropertyValue;
use zephyra_core::props;
use zephyra_core::theme::{Theme, ThemeVariant};
use zephyra_style::ir::StyleSheet;
use zephyra_style::SheetStyler;

/// 只声明一件事的表：`.grown` 给 20 padding，基类给 0。
///
/// 基类**必须**显式声明 `padding: 0px`：否则折叠时那个键无人提及，
/// 走的是 `retain_styled_props` 的拆键路径，而两条路径（写 / 拆）都该被测到。
const CSS: &str = r#"
    Container { padding: 0px; }
    Container.grown { padding: 20px; }
"#;

/// 装好样式表的缓存。
///
/// 抽成函数而不是在三个测试里各写一遍：构造写三遍，改一次一定漏两遍。
fn cache_with(name: &str) -> FrameCache {
    let parsed = zstylec::parse(name, CSS);
    assert!(parsed.is_ok(), "stylesheet must parse: {:?}", parsed.errors);
    let sheet = Arc::new(StyleSheet::new().merge(parsed.sheet));
    let mut cache = FrameCache::new();
    cache.set_styler(Box::new(SheetStyler(sheet)));
    cache
}

/// 一个 chip：外层容器 + 一个子方块。
///
/// 子节点是必须的 —— `padding` 的效果只能在**子项矩形**上观察到，
/// 而 [`Node::rect`] 是 arrange 给的外盒，含 padding 前后不变。
fn chip(grown: bool) -> Element {
    let inner = Element::new(NodeTag::Container)
        .with_key("inner")
        .with(&props::WIDTH, PropertyValue::float(30.0))
        .with(&props::HEIGHT, PropertyValue::float(30.0));
    let mut el = Element::new(NodeTag::Container)
        .with_key("outer")
        .with(&props::WIDTH, PropertyValue::float(120.0))
        .with(&props::HEIGHT, PropertyValue::float(60.0))
        .with_children([inner]);
    if grown {
        el = el.style("grown");
    }
    el
}

/// 取子项的 x —— padding 是否生效的**唯一**可观测量。
fn inner_x(root: &Node) -> f32 {
    root.children().first().expect("chip 有子节点").rect().x
}

#[test]
fn hover_改尺寸要重排而不只是重绘() {
    let mut cache = cache_with("hover-layout.zstyle");
    let theme = Theme::builtin(ThemeVariant::Dark);
    let ctx = Context::new(&theme);
    let bounds = (400.0, 300.0);

    cache.frame(&chip(false), ctx, bounds);
    let before = inner_x(cache.node().expect("树已建"));
    assert_eq!(before, 0.0, "未展开时子项应贴左边缘");

    // 第二帧：只翻分类名 —— 等价于鼠标移入（hover）或按下（press）。
    let second = cache.frame(&chip(true), ctx, bounds);
    assert!(
        matches!(second, FrameOutcome::Updated { .. }),
        "分类名变了至少要重绘"
    );
    assert_eq!(
        inner_x(cache.node().expect("树还在")),
        20.0,
        "padding 20 必须让子项左缘落在 20；仍是 0 说明没有重排 —— \
         正是「只有 resize 才更新布局」的缺陷"
    );
}

/// 折返（取消 hover）也必须重排：这条走的是 apply 的**拆键**路径。
///
/// 拆键与写键是两条不同代码路径（`retain_styled_props` vs `set_styled_prop`），
/// 只测一边会漏掉「取消 hover 后布局弹不回去」这类缺陷。
#[test]
fn unhover_still_triggers_relayout() {
    let mut cache = cache_with("hover-layout-back.zstyle");
    let theme = Theme::builtin(ThemeVariant::Dark);
    let ctx = Context::new(&theme);
    let bounds = (400.0, 300.0);

    cache.frame(&chip(true), ctx, bounds);
    assert_eq!(inner_x(cache.node().unwrap()), 20.0);
    cache.frame(&chip(false), ctx, bounds);
    assert_eq!(
        inner_x(cache.node().unwrap()),
        0.0,
        "取消 hover 后必须回到 0；仍是 20 说明拆键路径没有触发重排"
    );
}

/// 稳态仍然要真的静止：这条修法不能把「静止时每帧重排」带回来。
///
/// 判据是同一状态连续两帧 ⇒ 第二帧必须 `Unchanged`。若 `apply` 每帧都标脏，
/// 这里会立刻红 —— 而那正是「样式表导致 UI 永远不静止」的成因。
#[test]
fn steady_state_stays_quiet_after_fix() {
    let mut cache = cache_with("hover-layout-steady.zstyle");
    let theme = Theme::builtin(ThemeVariant::Dark);
    let ctx = Context::new(&theme);
    let bounds = (400.0, 300.0);

    cache.frame(&chip(true), ctx, bounds);
    let second = cache.frame(&chip(true), ctx, bounds);
    assert!(
        matches!(second, FrameOutcome::Unchanged),
        "同一状态第二帧必须 Unchanged，否则 UI 永远不静止"
    );
}

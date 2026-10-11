//! controls.zstyle 与 demo 原来写的那堆属性的**等价性**测试。
//!
//! # 为什么必须有这条
//!
//! B3 的门禁是「demo 换掉那些 `.with(...)` 之后**像素不变**」。但 demo 走
//! 窗口路径，`--dump-cpu` 走的是 `CpuHost`（不经 `HostWindow`，所以样式表根本
//! 没加载）—— 两条路都不好自动比像素。
//!
//! 于是换一个更硬的判据：**把两边解析出的属性值逐个比**。这比像素强在：
//! 像素只告诉你「看起来一样」，而属性值直接告诉你「是不是同一个数」。
//!
//! ⚠️ 这条测试同时是**回归**：以后谁改了 `controls.zstyle` 或改了 demo 的
//! `.with(...)`，两边的差会立刻暴露，而不是等到有人肉眼看出按钮变样。

use zephyra_core::core::Color;
use zephyra_core::id::NodeTag;
use zephyra_core::element::Element;
use zephyra_core::node::Node;
use zephyra_core::property::{PropertyValue, ValueSource};
use zephyra_core::props;
use zephyra_style::SheetStyler;

/// demo 原来写死的那些值。**改这里 = 改控制点**：它与 controls.zstyle 必须一致。
const BTN: f32 = 40.0;
const EXPECTED: &[(&zephyra_core::property::PropertyKey, PropertyValue)] = &[
    (&props::WIDTH, PropertyValue::float(BTN)),
    (&props::HEIGHT, PropertyValue::float(BTN)),
    (&props::CORNER_RADIUS, PropertyValue::float(6.0)),
];

/// 造一个 Control 节点，施加样式表，回读某个属性的（值, 来源）。
fn styled(key: &'static zephyra_core::property::PropertyKey) -> (Option<PropertyValue>, ValueSource) {
    let css = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/controls.zstyle"
    ))
    .expect("controls.zstyle must exist");
    let parsed = zstylec::parse("controls.zstyle", &css);
    assert!(parsed.is_ok(), "controls.zstyle must parse: {:?}", parsed.errors);

    let mut node = Node::from_element(&Element::new(NodeTag::Control));
    let mut styler = SheetStyler(std::sync::Arc::new(parsed.sheet));
    zephyra_core::styler::Styler::apply(&mut styler, &mut node);

    node.props()
        .lookup(key)
        .map(|(v, s)| (Some(v.clone()), s))
        .unwrap_or((None, ValueSource::Default))
}

#[test]
fn the_stylesheet_gives_the_button_its_size_back() {
    for (key, want) in EXPECTED {
        let (got, source) = styled(key);
        assert_eq!(
            got.as_ref(),
            Some(want),
            "{} from controls.zstyle ({got:?}) must equal what demo used to write",
            key.name()
        );
        assert_eq!(
            source,
            ValueSource::Styled,
            "{} must come from the Styled layer",
            key.name()
        );
    }
}

#[test]
fn the_stylesheet_gives_the_button_its_colours_from_tokens() {
    // Colour tokens are `Resource` -- the token *name*, not a resolved colour.
    // That is what makes theme switching free.
    let (bg, source) = styled(&props::BACKGROUND);
    assert_eq!(
        bg.as_ref().and_then(|v| v.resource_key().map(|s| s.to_string())),
        Some("control_fill".into()),
        "background must be the @control_fill token, not a baked-in colour"
    );
    assert_eq!(source, ValueSource::Styled);

    let (stroke, _) = styled(&props::BORDER_COLOR);
    assert_eq!(
        stroke.as_ref().and_then(|v| v.resource_key().map(|s| s.to_string())),
        Some("stroke".into()),
        "border-color must be the @stroke token"
    );
}

#[test]
fn the_stylesheet_sets_the_layout_props_the_button_needs() {
    let (dir, _) = styled(&props::DIRECTION);
    assert!(
        dir.is_some(),
        "direction must be set -- without it the icon and label sit side by side"
    );
    let (spacing, _) = styled(&props::SPACING);
    assert_eq!(
        spacing.as_ref().and_then(|v| v.get::<f32>()),
        Some(3.0),
        "spacing must still be 3px"
    );
}

/// A `Container` node must **not** pick up the button's look.
///
/// The rule in `controls.zstyle` is a bare `Control { }` selector, so it
/// applies to type only. This pins that the selector is not accidentally
/// matching everything.
#[test]
fn other_element_types_are_left_alone() {
    let css = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/controls.zstyle"
    ))
    .expect("controls.zstyle must exist");
    let parsed = zstylec::parse("controls.zstyle", &css);
    assert!(parsed.is_ok());

    let mut node = Node::from_element(&Element::new(NodeTag::Container));
    let mut styler = SheetStyler(std::sync::Arc::new(parsed.sheet));
    zephyra_core::styler::Styler::apply(&mut styler, &mut node);

    assert_eq!(
        node.props().lookup(&props::BACKGROUND),
        None,
        "a Container must not get the button's background"
    );
    assert_eq!(
        node.props().lookup(&props::WIDTH),
        None,
        "a Container must not get the button's width"
    );
}

/// Unused on purpose: keeps `Color` imported for the token assertions above
/// should they ever need a literal colour.
#[allow(dead_code)]
fn _colour_anchor(c: Color) -> Color {
    c
}

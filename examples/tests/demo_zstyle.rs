//! `demo.zstyle` 的**值**门禁。
//!
//! # 为什么不能只靠 demo.rs 里那几条
//!
//! `demo.rs` 的测试钉的是**源码这边**：Local 层里没有外观、分类名都打得通、
//! 每个元素都能被选中。它们都过了，**删掉样式表里的一条声明照样全绿** ——
//! 实测过：删掉 `Container.status { background: ... }` 后 6 条测试全过，
//! 而状态栏的背景已经没了，没有任何东西报错。
//!
//! 这就是「源码干净」和「样式表正确」是两件事：前者靠 `demo.rs` 的测试，
//! 后者必须靠**钉住产出值**，也就是这个文件。
//!
//! # 表格即控制点
//!
//! 每条规则至少钉一个属性。加新规则时往里加一行 —— 不加就是新的空白，
//! 而空白的样子正是上面那个例子：测试全绿，画面少了东西。

use zephyra_core::core::Color;
use zephyra_core::element::Element;
use zephyra_core::id::NodeTag;
use zephyra_core::node::Node;
use zephyra_core::property::{PropertyValue, ValueSource};
use zephyra_core::props;
use zephyra_style::SheetStyler;

/// 一次期望：什么元素、哪个属性、应该是什么值。
struct Case {
    /// 元素类型。
    tag: NodeTag,
    /// 分类名（源码的 `.style(...)` 发出来的那些）。
    names: Vec<&'static str>,
    /// 查哪个属性。
    key: &'static zephyra_core::property::PropertyKey,
    /// 期望值。`None` 表示「这条规则不该动它」。
    want: Option<PropertyValue>,
    /// 这条在钉什么（断言失败时打出来）。
    what: &'static str,
}

/// 造一个带分类名的节点，施加**合并后**的样式表，回读（值, 来源）。
fn styled_tag(
    t: NodeTag,
    names: &[&'static str],
    key: &'static zephyra_core::property::PropertyKey,
) -> (Option<PropertyValue>, ValueSource) {
    let sheet = load_sheets();
    let mut node = Node::from_element(&Element::new(t).styles(names.iter().copied()));
    let mut styler = SheetStyler(std::sync::Arc::new(sheet));
    zephyra_core::styler::Styler::apply(&mut styler, &mut node);
    node.props()
        .lookup(key)
        .map(|(v, s)| (Some(v.clone()), s))
        .unwrap_or((None, ValueSource::Default))
}

/// 按文件名排序读入并合并 —— 与 `load_stylesheet` 同一条规则，
/// 否则测试看到的世界和 demo 看到的不一样。
fn load_sheets() -> zephyra_style::ir::StyleSheet {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut paths: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
        .expect("examples dir must be listable")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("zstyle"))
        .collect();
    paths.sort();

    let mut sheet = zephyra_style::ir::StyleSheet::new();
    for path in &paths {
        let css = std::fs::read_to_string(path).expect("stylesheet must be readable");
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("x.zstyle");
        let result = zstylec::parse(name, &css);
        assert!(result.is_ok(), "{name} must parse: {:?}", result.errors);
        sheet = sheet.merge(result.sheet);
    }
    sheet
}

#[test]
fn every_rule_in_demo_zstyle_is_pinned() {
    let cases: Vec<Case> = vec![
        // ---- 根与工具栏 ----
        Case {
            tag: NodeTag::Container,
            names: vec!["root"],
            key: &props::BACKGROUND,
            want: Some(PropertyValue::Resource("surface".into())),
            what: "root takes @surface",
        },
        Case {
            tag: NodeTag::Container,
            names: vec!["root"],
            key: &props::DIRECTION,
            want: Some(PropertyValue::direction(
                zephyra_core::layout::Direction::Vertical,
            )),
            what: "root stacks",
        },
        Case {
            tag: NodeTag::Container,
            names: vec!["toolbar"],
            key: &props::BACKGROUND,
            want: Some(PropertyValue::Resource("surface_alt".into())),
            what: "toolbar takes @surface_alt",
        },
        Case {
            tag: NodeTag::Container,
            names: vec!["toolbar"],
            key: &props::DIRECTION,
            want: Some(PropertyValue::direction(
                zephyra_core::layout::Direction::Horizontal,
            )),
            what: "toolbar is a row",
        },
        Case {
            tag: NodeTag::Container,
            names: vec!["toolbar"],
            key: &props::PADDING,
            want: Some(PropertyValue::float(8.0)),
            what: "toolbar padding 8",
        },
        Case {
            tag: NodeTag::Container,
            names: vec!["toolbar"],
            key: &props::SPACING,
            want: Some(PropertyValue::float(12.0)),
            what: "toolbar spacing 12",
        },
        // ---- 图标墙 ----
        Case {
            tag: NodeTag::Container,
            names: vec!["icon-row"],
            key: &props::SPACING,
            want: Some(PropertyValue::float(16.0)),
            what: "icon row spacing 16",
        },
        Case {
            tag: NodeTag::Image,
            names: vec!["icon"],
            key: &props::WIDTH,
            want: Some(PropertyValue::float(56.0)),
            what: "icon 56 wide",
        },
        Case {
            tag: NodeTag::Image,
            names: vec!["icon"],
            key: &props::CORNER_RADIUS,
            want: Some(PropertyValue::float(8.0)),
            what: "icon rounded 8",
        },
        // ---- uv 裁剪 ----
        Case {
            tag: NodeTag::Container,
            names: vec!["crop-row"],
            key: &props::SPACING,
            want: Some(PropertyValue::float(8.0)),
            what: "crop row spacing 8",
        },
        Case {
            tag: NodeTag::Image,
            names: vec!["crop"],
            key: &props::WIDTH,
            want: Some(PropertyValue::float(40.0)),
            what: "crop 40 wide",
        },
        Case {
            tag: NodeTag::Image,
            names: vec!["crop"],
            key: &props::HEIGHT,
            want: Some(PropertyValue::float(40.0)),
            what: "crop 40 tall",
        },
        // ---- 说明面板 ----
        Case {
            tag: NodeTag::Container,
            names: vec!["content"],
            key: &props::BACKGROUND,
            want: Some(PropertyValue::Resource("surface".into())),
            what: "content takes @surface",
        },
        Case {
            tag: NodeTag::Container,
            names: vec!["content"],
            key: &props::PADDING,
            want: Some(PropertyValue::float(16.0)),
            what: "content padding 16",
        },
        Case {
            tag: NodeTag::Container,
            names: vec!["hero"],
            key: &props::BACKGROUND,
            want: Some(PropertyValue::Resource("accent_fill".into())),
            what: "hero takes @accent_fill",
        },
        Case {
            tag: NodeTag::Container,
            names: vec!["hero"],
            key: &props::CORNER_RADIUS,
            want: Some(PropertyValue::float(12.0)),
            what: "hero rounded 12",
        },
        Case {
            tag: NodeTag::Text,
            names: vec!["hero-line"],
            key: &props::TEXT_SIZE,
            want: Some(PropertyValue::float(16.0)),
            what: "hero text 16",
        },
        Case {
            tag: NodeTag::Text,
            names: vec!["hero-line"],
            key: &props::TEXT_COLOR,
            want: Some(PropertyValue::Resource("on_accent_fill".into())),
            what: "hero text @on_accent_fill",
        },
        // ---- 效果样张 ----
        Case {
            tag: NodeTag::Container,
            names: vec!["effects"],
            key: &props::BACKGROUND,
            want: Some(PropertyValue::Resource("surface_alt".into())),
            what: "effects takes @surface_alt",
        },
        Case {
            tag: NodeTag::Container,
            names: vec!["effects"],
            key: &props::PADDING,
            want: Some(PropertyValue::float(12.0)),
            what: "effects padding 12",
        },
        Case {
            tag: NodeTag::Container,
            names: vec!["effects-row"],
            key: &props::SPACING,
            want: Some(PropertyValue::float(12.0)),
            what: "effects row spacing 12",
        },
        Case {
            tag: NodeTag::Container,
            names: vec!["fx"],
            key: &props::SPACING,
            want: Some(PropertyValue::float(6.0)),
            what: "fx spacing 6",
        },
        Case {
            tag: NodeTag::Container,
            names: vec!["backdrop"],
            key: &props::BACKGROUND,
            want: Some(PropertyValue::color(Color::rgba(
                26.0 / 255.0,
                51.0 / 255.0,
                107.0 / 255.0,
                1.0,
            ))),
            what: "backdrop is the literal #1a336b",
        },
        Case {
            tag: NodeTag::Container,
            names: vec!["backdrop"],
            key: &props::WIDTH,
            want: Some(PropertyValue::float(96.0)),
            what: "backdrop 96 wide",
        },
        Case {
            tag: NodeTag::Container,
            names: vec!["backdrop"],
            key: &props::HEIGHT,
            want: Some(PropertyValue::float(64.0)),
            what: "backdrop 64 tall",
        },
        Case {
            tag: NodeTag::Container,
            names: vec!["backdrop"],
            key: &props::PADDING,
            want: Some(PropertyValue::float(12.0)),
            what: "backdrop padding 12",
        },
        Case {
            tag: NodeTag::Container,
            names: vec!["swatch"],
            key: &props::WIDTH,
            want: Some(PropertyValue::float(72.0)),
            what: "swatch 72 wide -- 0 would collapse it",
        },
        Case {
            tag: NodeTag::Container,
            names: vec!["swatch"],
            key: &props::CORNER_RADIUS,
            want: Some(PropertyValue::float(6.0)),
            what: "swatch rounded 6",
        },
        Case {
            tag: NodeTag::Text,
            names: vec!["caption"],
            key: &props::TEXT_SIZE,
            want: Some(PropertyValue::float(12.0)),
            what: "caption 12",
        },
        // ---- 状态栏 ----
        Case {
            tag: NodeTag::Container,
            names: vec!["status"],
            key: &props::BACKGROUND,
            want: Some(PropertyValue::Resource("surface_alt".into())),
            what: "status takes @surface_alt",
        },
        Case {
            tag: NodeTag::Container,
            names: vec!["status"],
            key: &props::PADDING,
            want: Some(PropertyValue::float(8.0)),
            what: "status padding 8",
        },
        Case {
            tag: NodeTag::Container,
            names: vec!["dot"],
            key: &props::BACKGROUND,
            want: Some(PropertyValue::Resource("focus_ring".into())),
            what: "dot takes @focus_ring",
        },
        Case {
            tag: NodeTag::Container,
            names: vec!["dot"],
            key: &props::WIDTH,
            want: Some(PropertyValue::float(10.0)),
            what: "dot 10 wide",
        },
        Case {
            tag: NodeTag::Text,
            names: vec!["status-line"],
            key: &props::TEXT_SIZE,
            want: Some(PropertyValue::float(12.0)),
            what: "status text 12",
        },
        // ---- 交互态 ----
        Case {
            tag: NodeTag::Container,
            names: vec!["glyph"],
            key: &props::ICON_COLOR,
            want: Some(PropertyValue::Resource("content_secondary".into())),
            what: "idle icon colour",
        },
        Case {
            tag: NodeTag::Container,
            names: vec!["glyph", "hovered"],
            key: &props::ICON_COLOR,
            want: Some(PropertyValue::Resource("content".into())),
            what: "hover brightens the icon",
        },
        Case {
            tag: NodeTag::Container,
            names: vec!["glyph", "pressed"],
            key: &props::ICON_COLOR,
            want: Some(PropertyValue::Resource("content".into())),
            what: "press brightens the icon",
        },
        Case {
            tag: NodeTag::Text,
            names: vec!["label"],
            key: &props::TEXT_COLOR,
            want: Some(PropertyValue::Resource("content_secondary".into())),
            what: "idle label colour",
        },
        Case {
            tag: NodeTag::Text,
            names: vec!["label", "hovered"],
            key: &props::TEXT_COLOR,
            want: Some(PropertyValue::Resource("content".into())),
            what: "hover brightens the label",
        },
        Case {
            tag: NodeTag::Text,
            names: vec!["label", "compact"],
            key: &props::TEXT_SIZE,
            want: Some(PropertyValue::float(9.0)),
            what: "long names drop to 9",
        },
        Case {
            tag: NodeTag::Text,
            names: vec!["label"],
            key: &props::TEXT_SIZE,
            want: Some(PropertyValue::float(11.0)),
            what: "short names stay 11",
        },
        Case {
            tag: NodeTag::Text,
            names: vec!["status-line"],
            key: &props::TEXT_COLOR,
            want: Some(PropertyValue::Resource("content_disabled".into())),
            what: "no hover -> disabled colour",
        },
        Case {
            tag: NodeTag::Text,
            names: vec!["status-line", "hovered"],
            key: &props::TEXT_COLOR,
            want: Some(PropertyValue::Resource("content".into())),
            what: "hover -> content colour",
        },
        // ---- 控件（controls.zstyle）----
        Case {
            tag: NodeTag::Control,
            names: vec![],
            key: &props::WIDTH,
            want: Some(PropertyValue::float(40.0)),
            what: "button 40 wide",
        },
        Case {
            tag: NodeTag::Control,
            names: vec!["pressed"],
            key: &props::BORDER_WIDTH,
            want: Some(PropertyValue::float(2.0)),
            what: "pressed border 2",
        },
        Case {
            tag: NodeTag::Control,
            names: vec![],
            key: &props::BORDER_WIDTH,
            want: Some(PropertyValue::float(1.0)),
            what: "idle border 1",
        },
    ];

    assert!(
        cases.len() > 40,
        "只有 {} 条 —— 表格太薄，这条测试会变成恒真",
        cases.len()
    );

    let mut failed = Vec::new();
    for c in &cases {
        let (got, source) = styled_tag(c.tag, &c.names, c.key);
        if got.as_ref() != c.want.as_ref() {
            failed.push(format!(
                "{} / {:?} / .{}: want {:?}, got {:?}",
                c.what,
                c.tag,
                c.names.join("."),
                c.want,
                got
            ));
        } else if source != ValueSource::Styled {
            failed.push(format!(
                "{} / .{}: value matches but source is {source:?}, not Styled",
                c.what,
                c.names.join(".")
            ));
        }
    }
    assert!(
        failed.is_empty(),
        "{} 条不符：\n  {}",
        failed.len(),
        failed.join("\n  ")
    );
}

/// 表格里没覆盖到的属性，**必须**与「这条规则没写过」一致。
///
/// ⚠️ 判据是 **来源**，不是值：`lookup()` 拿不到时会回落到属性默认值，
/// 而 `BORDER_WIDTH` 的默认值恰好是 1。所以「没有边框」看上去像
/// `Some(1)` —— 用值判断会把默认值误判成规则写的东西。
/// `ValueSource::Default` 才是「没人写过」的正确表达。
#[test]
fn rules_do_not_set_anything_they_should_not() {
    // 根节点不该有边框 —— 它不是一个可交互控件。
    let (v, s) = styled_tag(NodeTag::Container, &["root"], &props::BORDER_WIDTH);
    assert_eq!(
        (v, s),
        (None, ValueSource::Default),
        "root must not gain a border"
    );
    // 图标墙是透明的行，不该有背景。
    let (v, s) = styled_tag(NodeTag::Container, &["icon-row"], &props::BACKGROUND);
    assert_eq!(
        (v, s),
        (None, ValueSource::Default),
        "icon row must stay transparent"
    );
    // 分类名不该影响别的分类名：.crop 不该把 .icon 的尺寸也带走。
    let (v, s) = styled_tag(NodeTag::Image, &["crop"], &props::CORNER_RADIUS);
    assert_eq!(
        (v, s),
        (None, ValueSource::Default),
        "crop must not inherit icon's corner radius"
    );
    // 类型不该串：Container 上的分类名不该命中 Image。
    let (v, s) = styled_tag(NodeTag::Image, &["toolbar"], &props::BACKGROUND);
    assert_eq!(
        (v, s),
        (None, ValueSource::Default),
        "Image must not match Container.toolbar"
    );
    // 控件分类名不该串到 Container 上。
    let (v, s) = styled_tag(NodeTag::Container, &["pressed"], &props::BORDER_WIDTH);
    assert_eq!(
        (v, s),
        (None, ValueSource::Default),
        "Container must not match Control.pressed"
    );
}

/// Unused on purpose: keeps the `tag` helper compiling once the table above is
/// the only user of `Case`. If this ever goes unused, `Vec<&'static str>` in
/// `Case` is the thing to drop.
#[allow(dead_code)]
fn _anchor(c: Case) -> Case {
    c
}

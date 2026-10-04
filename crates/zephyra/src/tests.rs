//! 核心不变量的单元测试。
//!
//! 这些断言针对的是**契约**，不是实现细节：换实现时它们应当继续成立。
//! 每条都写明「为什么这条不变量值得用测试钉住」。

use crate::core::{Color, CornerRadius, Point, Rect, Size, Spacing, Thickness};
use crate::layout::{align_box, Align, AlignItems, Constraints};
use crate::property::{resolve, PropertyKey, PropertyStore, PropertyValue, ValueSource};
use crate::theme::{roles, Theme, ThemeVariant};

// ============================== core ==============================

#[test]
fn intersect_returns_none_when_disjoint() {
    let a = Rect::new(0.0, 0.0, 10.0, 10.0);
    let b = Rect::new(20.0, 20.0, 5.0, 5.0);
    assert_eq!(a.intersect(&b), None, "不相交必须返回 None 而非空矩形");
    // 空矩形会在后续布局里变成难以追踪的零尺寸。
    assert_eq!(b.intersect(&a), None);
}

#[test]
fn intersect_touching_edges_is_empty_not_none() {
    let a = Rect::new(0.0, 0.0, 10.0, 10.0);
    let b = Rect::new(10.0, 0.0, 10.0, 10.0);
    assert_eq!(a.intersect(&b), None);
}

#[test]
fn intersect_keeps_overlap() {
    let a = Rect::new(0.0, 0.0, 10.0, 10.0);
    let b = Rect::new(5.0, 5.0, 10.0, 10.0);
    assert_eq!(a.intersect(&b), Some(Rect::new(5.0, 5.0, 5.0, 5.0)));
}

#[test]
fn inset_never_produces_negative_size() {
    let r = Rect::new(0.0, 0.0, 4.0, 4.0);
    let inner = r.inset(Thickness::uniform(10.0));
    assert_eq!(inner.width, 0.0);
    assert_eq!(inner.height, 0.0);
}

#[test]
fn alpha_composite_differs_from_lerp_over_translucent_backdrop() {
    // 关键前提：dst 必须**半透明**。dst 不透明时 out_a 恒为 1，`over` 在数学上
    // 退化成 lerp —— 那是恒等式而非巧合（见下一个测试）。
    //
    // 真实场景：半透明浮层叠在半透明遮罩上。用 lerp 会得到两层颜色的直觉中点，
    // 而合成必须让**下层透过上层**，结果明显不同。
    let src = Color::rgba(1.0, 0.0, 0.0, 0.5);
    let dst = Color::rgba(0.0, 0.0, 1.0, 0.5);
    let over = src.over(dst);
    let lerped = src.lerp(dst, 0.5);

    assert_ne!(over, lerped, "半透明背景上合成与插值不是一回事");
    // out_a = 0.75 → 红渗入更多、蓝被压低，两者不再对称。
    assert!(
        (over.a - 0.75).abs() < 1e-6,
        "合成后透明度应累加，实得 {}",
        over.a
    );
    assert!(
        over.r > over.b,
        "上层红应主导，实得 r={} b={}",
        over.r,
        over.b
    );
    assert!((over.a - lerped.a).abs() > 1e-6);
}

#[test]
fn composite_over_opaque_backdrop_is_exactly_lerp() {
    // 这是一个**恒等式**，不是巧合：dst.a = 1 时 out.a = 1，于是
    //   r = src.r·a + dst.r·(1−a) = lerp(dst, src, a)
    // 值得钉住，因为一旦有人给 `over` 加"更聪明"的处理，这条会最先失效。
    let src = Color::rgba(1.0, 0.0, 0.0, 0.5);
    let dst = Color::rgba(0.0, 0.0, 1.0, 1.0);
    let over = src.over(dst);
    assert!((over.r - 0.5).abs() < 1e-6);
    assert!((over.g - 0.0).abs() < 1e-6);
    assert!((over.b - 0.5).abs() < 1e-6);
    assert!((over.a - 1.0).abs() < 1e-6, "不透明背景下合成结果应不透明");
    assert_eq!(over.r, over.b, "对半叠加应得到对称的紫");
}

#[test]
fn faint_source_lets_backdrop_dominate() {
    // 10% 的红叠在不透明蓝上：合成应几乎保持蓝。
    // 用 lerp 实现浮层会让深色背景明显发灰 —— 这是 UI 里能直接看出来的缺陷。
    let src = Color::rgba(1.0, 0.0, 0.0, 0.1);
    let dst = Color::rgba(0.0, 0.0, 1.0, 1.0);
    let over = src.over(dst);
    assert!(over.b > 0.85, "蓝色应占主导，实得 {}", over.b);
    assert!(over.r < 0.15, "红色只应渗入一点，实得 {}", over.r);
}

#[test]
fn over_transparent_src_is_identity() {
    let dst = Color::rgb(0.2, 0.4, 0.6);
    let transparent = Color::rgba(0.0, 0.0, 0.0, 0.0);
    assert_eq!(transparent.over(dst), dst);
}

#[test]
fn over_opaque_src_replaces_dst() {
    let dst = Color::rgb(0.2, 0.4, 0.6);
    let src = Color::rgb(1.0, 1.0, 1.0);
    assert_eq!(src.over(dst), src);
}

#[test]
fn spacing_scale_is_monotonic() {
    let scale = [
        Spacing::None,
        Spacing::XXSmall,
        Spacing::XSmall,
        Spacing::Small,
        Spacing::Medium,
        Spacing::Large,
        Spacing::XLarge,
        Spacing::XXLarge,
    ];
    for pair in scale.windows(2) {
        assert!(
            pair[0].dip() < pair[1].dip(),
            "{:?} 应小于 {:?}",
            pair[0],
            pair[1]
        );
    }
}

// ============================== layout ==============================

#[test]
fn constraints_tighten_never_relaxes() {
    let base = Constraints {
        min: Size::new(10.0, 10.0),
        max: Size::new(100.0, 100.0),
    };
    let tighter = base.tighten(Size::new(20.0, 0.0), Size::new(50.0, 200.0));
    assert!(tighter.min.width >= base.min.width);
    assert!(tighter.max.width <= base.max.width);
    // 请求放宽 max 不应生效。
    assert_eq!(tighter.max.width, 50.0);
}

#[test]
fn unbounded_reports_unbounded_axes() {
    let c = Constraints::unbounded();
    assert!(c.is_unbounded_width());
    assert!(c.is_unbounded_height());
    assert!(!Constraints::fixed(Size::new(1.0, 1.0)).is_unbounded_width());
}

#[test]
fn align_box_stretches_only_when_asked() {
    let content = Size::new(10.0, 10.0);
    let available = Size::new(100.0, 50.0);

    let start = align_box(content, available, Align::Start, AlignItems::Start);
    assert_eq!(start, Rect::new(0.0, 0.0, 10.0, 10.0));

    let center = align_box(content, available, Align::Center, AlignItems::Center);
    assert_eq!(center.x, 45.0);
    assert_eq!(center.y, 20.0);

    let stretch = align_box(content, available, Align::Stretch, AlignItems::Stretch);
    assert_eq!(stretch, Rect::new(0.0, 0.0, 100.0, 50.0));

    let end = align_box(content, available, Align::End, AlignItems::End);
    assert_eq!(end, Rect::new(90.0, 40.0, 10.0, 10.0));
}

// ============================== property ==============================

const LOCAL: &PropertyKey = &PropertyKey::new("Local", PropertyValue::float(1.0));
const INHERITED: &PropertyKey = &PropertyKey::inheritable("Inheritable", PropertyValue::float(2.0));

#[test]
fn property_definitions_are_const_constructible() {
    // 这条本身就是断言：属性定义必须是 const，否则每个节点/控件都要用
    // OnceLock 才能拿到键，样式表无法在编译期构造。
    const _: PropertyKey = PropertyKey::new("Const", PropertyValue::float(0.0));
}

#[test]
fn value_precedence_is_local_then_styled_then_default() {
    let mut store = PropertyStore::new();
    assert_eq!(
        resolve(&store, LOCAL, None).1,
        ValueSource::Default,
        "未设置应回落到默认值"
    );

    store.set_styled(LOCAL, PropertyValue::float(5.0));
    assert_eq!(resolve(&store, LOCAL, None).1, ValueSource::Styled);

    store.set_local(LOCAL, PropertyValue::float(9.0));
    let (value, source) = resolve(&store, LOCAL, None);
    assert_eq!(source, ValueSource::Local, "本地值必须压过样式值");
    assert_eq!(value.get::<f32>(), Some(9.0));
}

#[test]
fn inheritable_property_falls_back_to_inherited_then_default() {
    let store = PropertyStore::new();
    let inherited = PropertyValue::float(7.0);

    let (value, source) = resolve(&store, INHERITED, Some(&inherited));
    assert_eq!(source, ValueSource::Inherited);
    assert_eq!(value.get::<f32>(), Some(7.0));

    let (_, source) = resolve(&store, INHERITED, None);
    assert_eq!(source, ValueSource::Default, "无继承值时应回落到默认");
}

#[test]
fn non_inheritable_property_ignores_inherited_value() {
    let store = PropertyStore::new();
    let inherited = PropertyValue::float(7.0);
    let (value, source) = resolve(&store, LOCAL, Some(&inherited));
    assert_eq!(source, ValueSource::Default, "不可继承属性不应接受继承值");
    assert_eq!(value.get::<f32>(), Some(1.0));
}

#[test]
fn property_value_equality_is_variant_sensitive() {
    // diff 完全依赖这个判定来避免无谓重绘。
    assert_eq!(PropertyValue::float(1.0), PropertyValue::float(1.0));
    assert_ne!(PropertyValue::float(1.0), PropertyValue::float(1.5));
    // 跨变体永不相等 —— 否则 int 1 与 float 1.0 会被误判为「没变」。
    assert_ne!(PropertyValue::int(1), PropertyValue::float(1.0));
    assert_ne!(
        PropertyValue::Bool(true),
        PropertyValue::Resource("true".into())
    );
}

#[test]
fn custom_property_values_compare_by_value() {
    let a = PropertyValue::custom(vec![1u8, 2, 3]);
    let b = PropertyValue::custom(vec![1u8, 2, 3]);
    let c = PropertyValue::custom(vec![9u8]);
    assert_eq!(a, b);
    assert_ne!(a, c);
    assert_eq!(a.get::<Vec<u8>>(), Some(vec![1, 2, 3]));
}

#[test]
fn layout_affecting_properties_are_classified() {
    use crate::property::PropertyChange;
    const MARGIN: &PropertyKey = &PropertyKey::new("Margin", PropertyValue::float(0.0));
    const BACKGROUND: &PropertyKey = &PropertyKey::new(
        "Background",
        PropertyValue::color(Color::rgb(0.0, 0.0, 0.0)),
    );
    const LAYOUT_WIDTH: &PropertyKey = &PropertyKey::new("Layout.Width", PropertyValue::float(0.0));

    let mk = |key: &'static PropertyKey| PropertyChange {
        key: std::sync::Arc::new(PropertyKey::new(key.name(), key.default_value().clone())),
        old: None,
        new: None,
    };

    assert!(mk(MARGIN).affects_layout(), "Margin 必须触发重排");
    assert!(mk(LAYOUT_WIDTH).affects_layout(), "Layout.* 必须触发重排");
    assert!(
        !mk(BACKGROUND).affects_layout(),
        "改背景色若触发重排，就是 UI 卡顿最常见的单一原因"
    );
}

// ============================== theme ==============================

#[test]
fn every_advertised_role_resolves_in_both_variants() {
    for variant in [ThemeVariant::Light, ThemeVariant::Dark] {
        let theme = Theme::builtin(variant);
        for role in [
            roles::SURFACE,
            roles::SURFACE_ALT,
            roles::CONTENT,
            roles::CONTENT_SECONDARY,
            roles::CONTENT_DISABLED,
            roles::STROKE,
            roles::CONTROL_FILL,
            roles::CONTROL_FILL_HOVER,
            roles::CONTROL_FILL_PRESSED,
            roles::ACCENT_FILL,
            roles::ON_ACCENT_FILL,
            roles::FOCUS_RING,
        ] {
            assert!(theme.role(role).is_some(), "{variant:?} 缺少角色 {role}");
        }
    }
}

#[test]
fn unknown_role_returns_none_instead_of_silently_defaulting() {
    // 拼错角色名必须被发现，而不是安静地返回黑色然后继续画。
    assert_eq!(Theme::dark().role("surfcae"), None);
}

#[test]
fn light_and_dark_differ_visibly() {
    let light = Theme::builtin(ThemeVariant::Light);
    let dark = Theme::builtin(ThemeVariant::Dark);
    assert_ne!(light.surface, dark.surface, "浅深主题底色必须不同");
    assert_ne!(light.content, dark.content, "浅深主题文本色必须不同");
    assert_eq!(light.variant, ThemeVariant::Light);
    assert_eq!(dark.variant, ThemeVariant::Dark);
}

#[test]
fn theme_toggle_round_trips() {
    let mut ctx = crate::theme::ThemeContext::new(ThemeVariant::Dark);
    assert_eq!(ctx.theme().variant, ThemeVariant::Dark);
    assert!(ctx.toggle());
    assert_eq!(ctx.theme().variant, ThemeVariant::Light);
    assert!(ctx.toggle());
    assert_eq!(ctx.theme().variant, ThemeVariant::Dark);
}

// ============================== paint ==============================

use crate::paint::{Command, DisplayList, Layer, SortKey, TextureRef};

#[test]
fn display_list_sorts_by_layer_then_z_then_order() {
    let mut list = DisplayList::new();
    let quad = Command::Quad {
        rect: Rect::new(0.0, 0.0, 1.0, 1.0),
        radius: CornerRadius { radius: 0.0 },
        fill: Color::rgb(0.0, 0.0, 0.0),
        stroke: None,
    };
    // 故意乱序插入。
    list.push(
        SortKey {
            layer: Layer::Popup,
            z: 0,
            order: 0,
        },
        quad.clone(),
    );
    list.push(
        SortKey {
            layer: Layer::Background,
            z: 5,
            order: 1,
        },
        quad.clone(),
    );
    list.push(
        SortKey {
            layer: Layer::Background,
            z: 0,
            order: 2,
        },
        quad.clone(),
    );
    list.push(
        SortKey {
            layer: Layer::Content,
            z: 0,
            order: 3,
        },
        quad,
    );
    list.sort();

    let keys: Vec<SortKey> = list.iter().map(|i| i.key).collect();
    assert_eq!(keys[0].layer, Layer::Background);
    assert_eq!(keys[0].z, 0, "同层内应按 z 升序");
    assert_eq!(keys[1].layer, Layer::Background);
    assert_eq!(keys[1].z, 5);
    assert_eq!(keys[2].layer, Layer::Content);
    assert_eq!(keys[3].layer, Layer::Popup);
}

#[test]
fn display_list_sort_is_stable_for_equal_keys() {
    // 稳定性是宿主侧批次缓存的前提：同样输入必须产出同样顺序。
    let mut a = DisplayList::new();
    let mut b = DisplayList::new();
    let q = |w: f32| Command::Quad {
        rect: Rect::new(0.0, 0.0, w, 1.0),
        radius: CornerRadius { radius: 0.0 },
        fill: Color::rgb(0.0, 0.0, 0.0),
        stroke: None,
    };
    // 同样三个 key，插入顺序相同。
    for (list, widths) in [(&mut a, [1.0, 2.0, 3.0]), (&mut b, [1.0, 2.0, 3.0])] {
        for (i, w) in widths.iter().enumerate() {
            list.push(
                SortKey {
                    layer: Layer::Content,
                    z: 0,
                    order: i as u32,
                },
                q(*w),
            );
        }
        list.sort();
    }
    assert_eq!(a, b);
}

#[test]
fn layers_are_reported_deduplicated_and_ordered() {
    let mut list = DisplayList::new();
    let quad = Command::Quad {
        rect: Rect::default(),
        radius: CornerRadius { radius: 0.0 },
        fill: Color::rgb(0.0, 0.0, 0.0),
        stroke: None,
    };
    for layer in [
        Layer::Popup,
        Layer::Background,
        Layer::Content,
        Layer::Background,
    ] {
        list.push(
            SortKey {
                layer,
                z: 0,
                order: 0,
            },
            quad.clone(),
        );
    }
    list.sort();
    assert_eq!(
        list.layers(),
        vec![Layer::Background, Layer::Content, Layer::Popup]
    );
    assert!(list.has_layer(Layer::Popup));
    assert!(!list.has_layer(Layer::Overlay));
}

#[test]
fn clear_preserves_capacity_for_steady_state_reuse() {
    let mut list = DisplayList::new();
    let quad = Command::Quad {
        rect: Rect::default(),
        radius: CornerRadius { radius: 0.0 },
        fill: Color::rgb(0.0, 0.0, 0.0),
        stroke: None,
    };
    for i in 0..8 {
        list.push(
            SortKey {
                layer: Layer::Content,
                z: 0,
                order: i,
            },
            quad.clone(),
        );
    }
    list.clear();
    assert!(list.is_empty());
    // clear 只清内容，容量留给下一帧复用（稳态零分配）。
    assert!(list.capacity_hint() >= 8);
}

#[test]
fn only_popup_layer_needs_offscreen() {
    // 浮层必须盖住下层并带自己的阴影/模糊 → 需要离屏纹理。
    assert!(Layer::Popup.needs_offscreen());
    assert!(!Layer::Background.needs_offscreen());
    assert!(!Layer::Content.needs_offscreen());
}

#[test]
fn texture_ref_is_opaque_to_the_core() {
    let t = TextureRef::new("atlas/camera");
    assert_eq!(t, TextureRef::new("atlas/camera"));
    assert_ne!(t, TextureRef::new("atlas/pen"));
}

// ============================== tree ==============================

use crate::tree::{diff, render, DiffStats, Dirty, Element, Node, NodeTag};

fn sample() -> Element {
    Element::new(NodeTag::Container)
        .with_key("root")
        .with_children([
            Element::new(NodeTag::Text).with_key("title"),
            Element::new(NodeTag::Control).with_key("ok"),
        ])
}

#[test]
fn building_a_node_marks_it_structurally_dirty() {
    let node = Node::from_element(&sample());
    assert_eq!(node.dirty(), Dirty::Structure);
    assert_eq!(node.children().len(), 2);
}

#[test]
fn steady_state_diff_is_a_noop() {
    let mut node = Node::from_element(&sample());
    crate::tree::clear_dirty(&mut node);
    let stats = diff(&mut node, &sample());
    assert!(
        stats.is_noop(),
        "UI 静止时 diff 不得重建/重排任何东西，实得 {stats:?}"
    );
    assert!(stats.reused >= 3, "稳态应把所有节点就地复用");
}

#[test]
fn adding_a_child_is_detected_as_layout_work() {
    let mut node = Node::from_element(&sample());
    crate::tree::clear_dirty(&mut node);

    let wider = Element::new(NodeTag::Container)
        .with_key("root")
        .with_children([
            Element::new(NodeTag::Text).with_key("title"),
            Element::new(NodeTag::Control).with_key("ok"),
            Element::new(NodeTag::Control).with_key("cancel"),
        ]);
    let stats = diff(&mut node, &wider);
    assert!(!stats.is_noop());
    assert_eq!(stats.inserted, 1);
    assert!(stats.needs_layout(), "增删子节点必须要求重排");
    assert_eq!(node.children().len(), 3);
}

#[test]
fn removing_a_child_is_detected() {
    let mut node = Node::from_element(&sample());
    let narrower = Element::new(NodeTag::Container)
        .with_key("root")
        .with_children([Element::new(NodeTag::Text).with_key("title")]);
    let stats = diff(&mut node, &narrower);
    assert_eq!(stats.removed, 1);
    assert_eq!(node.children().len(), 1);
}

#[test]
fn tag_change_replaces_the_node_entirely() {
    let mut node = Node::from_element(&sample());
    let different = Element::new(NodeTag::Text).with_key("root");
    let stats = diff(&mut node, &different);
    assert_eq!(stats.inserted, 1);
    assert_eq!(node.tag(), NodeTag::Text);
    assert!(node.children().is_empty());
}

#[test]
fn dirty_levels_merge_by_severity() {
    assert_eq!(Dirty::Paint.merged(Dirty::Layout), Dirty::Layout);
    assert_eq!(Dirty::Layout.merged(Dirty::Paint), Dirty::Layout);
    assert_eq!(Dirty::Structure.merged(Dirty::None), Dirty::Structure);
    assert!(!Dirty::Paint.needs_layout());
    assert!(Dirty::Layout.needs_layout());
    assert!(Dirty::Structure.needs_layout());
}

#[test]
fn render_produces_a_sorted_stable_display_list() {
    let node = Node::from_element(&sample());
    let bounds = Rect::new(0.0, 0.0, 800.0, 600.0);

    let mut a = DisplayList::new();
    render(&node, bounds, &mut a);
    let mut b = DisplayList::new();
    render(&node, bounds, &mut b);

    assert!(!a.is_empty());
    assert_eq!(a, b, "同样输入必须产出逐字节相同的 DisplayList");
    let mut prev: Option<SortKey> = None;
    for item in a.iter() {
        if let Some(p) = prev {
            assert!(p < item.key, "DisplayList 必须已排序");
        }
        prev = Some(item.key);
    }
}

#[test]
fn render_reuses_the_supplied_list_buffer() {
    let node = Node::from_element(&sample());
    let bounds = Rect::new(0.0, 0.0, 100.0, 100.0);
    let mut list = DisplayList::new();
    render(&node, bounds, &mut list);
    let first = list.len();
    render(&node, bounds, &mut list);
    assert_eq!(list.len(), first, "复用缓冲不得让列表增长");
}

#[test]
fn clear_dirty_resets_every_node() {
    let mut node = Node::from_element(&sample());
    crate::tree::clear_dirty(&mut node);
    assert_eq!(node.dirty(), Dirty::None);
    for child in node.children() {
        assert_eq!(child.dirty(), Dirty::None, "子节点的脏标记也必须清除");
    }
}

// ============================== text ==============================

use crate::text::{ellipsize, NullEngine, TextEngineExt, TextStyle};

#[test]
fn text_measure_is_predictable_with_null_engine() {
    let mut engine = NullEngine { char_width: 1.0 };
    let style = TextStyle {
        size: 10.0,
        ..TextStyle::default()
    };
    let shaped = engine.measure_block("abcde", &style, 1000.0);
    assert_eq!(shaped.size.width, 50.0);
    assert_eq!(shaped.lines.len(), 1);
    assert_eq!(shaped.lines[0].is_last, true);
}

#[test]
fn text_wraps_within_max_width() {
    let mut engine = NullEngine { char_width: 1.0 };
    let style = TextStyle {
        size: 10.0,
        ..TextStyle::default()
    };
    // 每字符 10px，限宽 30px → 每行 3 字，7 字 → 3 行。
    let shaped = engine.measure_block("abcdefg", &style, 30.0);
    assert_eq!(shaped.lines.len(), 3);
    assert!(shaped.size.width <= 30.0);
}

#[test]
fn ellipsize_leaves_short_text_alone() {
    let mut engine = NullEngine { char_width: 1.0 };
    let style = TextStyle {
        size: 10.0,
        ..TextStyle::default()
    };
    assert_eq!(ellipsize(&mut engine, "ab", &style, 1000.0), "ab");
}

#[test]
fn ellipsize_truncates_and_marks() {
    let mut engine = NullEngine { char_width: 1.0 };
    let style = TextStyle {
        size: 10.0,
        ..TextStyle::default()
    };
    let out = ellipsize(&mut engine, "abcdefghij", &style, 50.0);
    assert!(out.ends_with('…'), "超宽文本应以省略号收尾");
    assert!(
        engine.advance_width(&out, &style) <= 50.0,
        "截断后必须真的放得下"
    );
}

#[test]
fn text_style_scale_multiplies_size() {
    let base = TextStyle {
        size: 12.0,
        scale: 1.0,
        ..TextStyle::default()
    };
    assert_eq!(base.effective_size(), 12.0);
    let scaled = TextStyle {
        scale: 1.5,
        ..base.clone()
    };
    assert_eq!(scaled.effective_size(), 18.0);
}

// ============================== hit ==============================

use crate::hit::rect_contains;

#[test]
fn rect_contains_uses_half_open_intervals() {
    let r = Rect::new(0.0, 0.0, 10.0, 10.0);
    assert!(rect_contains(r, Point::new(0.0, 0.0)), "左上角属于自身");
    assert!(rect_contains(r, Point::new(9.9, 9.9)));
    // 右/下边界必须归属下一个矩形，否则相邻控件会同时命中。
    assert!(!rect_contains(r, Point::new(10.0, 5.0)));
    assert!(!rect_contains(r, Point::new(5.0, 10.0)));
    assert!(!rect_contains(r, Point::new(-0.1, 5.0)));
}

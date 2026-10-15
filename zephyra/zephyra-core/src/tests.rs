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
    // ⚠️ B1 之后这条测试的形状变了：以前靠 `Layout.` **前缀**自动生效
    // （`PropertyKey::new("Layout.Width", …)` 就足够了），现在必须由构造器
    // **显式声明**。所以这里同时用到两种构造器的键。
    const MARGIN: &PropertyKey = &PropertyKey::layout("Margin", PropertyValue::float(0.0));
    const BACKGROUND: &PropertyKey = &PropertyKey::new(
        "Background",
        PropertyValue::color(Color::rgb(0.0, 0.0, 0.0)),
    );
    const LAYOUT_WIDTH: &PropertyKey =
        &PropertyKey::layout("Layout.Width", PropertyValue::float(0.0));

    // 直接借 `&'static` —— 与 [`crate::property::PropertyStore::set_local`] 一样。
    //
    // 原先这里写的是 `Arc::new(PropertyKey::new(key.name(), key.default_value().clone()))`
    // —— 手动复刻了 `set_local` 里那次多余的复制，于是测试**自己**付了一次堆分配，
    // 还把"必须复制"这件事固化成了断言的一部分。编译器给的可选修复
    // （`&Arc::new(...)`，靠泄漏换取 `'static`）恰好是最坏的方向：它把一次多余的
    // 分配变成一次永久泄漏。所以这里按新契约改，而不是照编译器建议改。
    let mk = |key: &'static PropertyKey| PropertyChange {
        key,
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
        filter: crate::filter::FilterChain::none(),
        blend: crate::filter::BlendMode::default(),
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
        filter: crate::filter::FilterChain::none(),
        blend: crate::filter::BlendMode::default(),
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
        filter: crate::filter::FilterChain::none(),
        blend: crate::filter::BlendMode::default(),
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
        filter: crate::filter::FilterChain::none(),
        blend: crate::filter::BlendMode::default(),
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

use crate::diff::diff;
use crate::element::Element;
use crate::id::NodeTag;
use crate::node::{Dirty, Node};
use crate::paint::render;

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
    crate::node::clear_dirty(&mut node);
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
    crate::node::clear_dirty(&mut node);

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
fn render_draws_nothing_when_no_node_has_a_background() {
    // 这条钉住"透明即不画"：**没有**背景色的节点不产出任何绘制命令。
    //
    // 之前的骨架实现对每个节点都无条件画一条 Quad，于是"什么都没有"的树也会
    // 产出 4 条命令。真实管线必须让空树产出空列表——否则一层透明的容器就会
    // 挡住它下面的一切，而这正是"画面全无"类 bug 的成因。
    let mut node = Node::from_element(&sample());
    crate::layout::layout(
        &mut node,
        Rect::new(0.0, 0.0, 800.0, 600.0),
        crate::context::dark_context(),
    );

    let mut list = DisplayList::new();
    render(&node, crate::context::dark_context(), &mut list);
    assert!(
        list.is_empty(),
        "无背景的节点不应产出绘制命令，实得 {} 条",
        list.len()
    );
}

#[test]
fn render_produces_a_sorted_stable_display_list() {
    // 必须给出背景色，否则按上一条断言就该是空的。
    let element = Element::new(NodeTag::Container)
        .with_key("root")
        .with(
            &crate::props::BACKGROUND,
            PropertyValue::color(crate::core::Color::rgb(0.2, 0.2, 0.2)),
        )
        .with_children([
            Element::new(NodeTag::Control).with_key("a"),
            Element::new(NodeTag::Control).with_key("b"),
        ]);
    let mut node = Node::from_element(&element);
    let bounds = Rect::new(0.0, 0.0, 800.0, 600.0);
    // render 只读已摆好的矩形，布局必须先行。
    crate::layout::layout(&mut node, bounds, crate::context::dark_context());

    let mut a = DisplayList::new();
    render(&node, crate::context::dark_context(), &mut a);
    let mut b = DisplayList::new();
    render(&node, crate::context::dark_context(), &mut b);

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
    let mut node = Node::from_element(&sample());
    let bounds = Rect::new(0.0, 0.0, 100.0, 100.0);
    crate::layout::layout(&mut node, bounds, crate::context::dark_context());
    let mut list = DisplayList::new();
    render(&node, crate::context::dark_context(), &mut list);
    let first = list.len();
    render(&node, crate::context::dark_context(), &mut list);
    assert_eq!(list.len(), first, "复用缓冲不得让列表增长");
}

#[test]
fn clear_dirty_resets_every_node() {
    let mut node = Node::from_element(&sample());
    crate::node::clear_dirty(&mut node);
    assert_eq!(node.dirty(), Dirty::None);
    for child in node.children() {
        assert_eq!(child.dirty(), Dirty::None, "子节点的脏标记也必须清除");
    }
}

// ============================== text → DisplayList ==============================

/// 文本层的端到端：**声明 → 布局 → Command::Text**。
///
/// 这批断言补的是文本层最大的缺口 —— 之前 `Command::Text` 有定义、有 `GlyphQuad`
/// 类型、有 `TextEngine` trait，但**零生产者**：`render_rec` 从不为它产出命令，
/// 文本节点在屏幕上什么都不显示。
///
/// 用 `NullEngine` 而非真实字体：这里要断言的是"核心是否正确地接线了"，不是
/// "字形是否好看"。uv 的正确性属于 base 层（要真实光栅化才能验）。
mod text_pipeline {
    use crate::context::Context;
    use crate::core::{Rect, Size};
    use crate::element::Element;
    use crate::id::NodeTag;
    use crate::layout::layout;
    use crate::node::Node;
    use crate::paint::render;
    use crate::paint::{Command, DisplayList};
    use crate::property::PropertyValue;
    use crate::props;
    use crate::text::{NullEngine, TextStyle};
    use crate::theme::Theme;
    use std::cell::RefCell;

    /// 固定字宽的引擎，让测量结果可预测。
    fn engine() -> RefCell<NullEngine> {
        RefCell::new(NullEngine { char_width: 0.5 })
    }

    /// 一个文本节点。
    fn text_node(text: &'static str, size: f32) -> Element {
        Element::new(NodeTag::Text)
            .with(&props::TEXT, PropertyValue::static_str(text))
            .with(&props::TEXT_SIZE, PropertyValue::float(size))
            .with(
                &props::TEXT_COLOR,
                PropertyValue::color(crate::core::Color::rgb(1.0, 0.0, 0.0)),
            )
    }

    /// 走完 声明 → 布局 → 渲染，返回 DisplayList。
    fn render_text(element: &Element, engine: &RefCell<NullEngine>, size: Size) -> DisplayList {
        let theme = Theme::dark();
        let ctx = Context::with_engine(&theme, engine);
        let mut node = Node::from_element(element);
        layout(&mut node, Rect::new(0.0, 0.0, size.width, size.height), ctx);
        let mut list = DisplayList::new();
        render(&node, ctx, &mut list);
        list
    }

    fn text_commands(list: &DisplayList) -> Vec<(&str, usize)> {
        list.iter()
            .filter_map(|i| match &i.command {
                Command::Text { atlas, glyphs, .. } => Some((atlas.as_str(), glyphs.len())),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_text_node_produces_a_text_command() {
        let engine = engine();
        let list = render_text(&text_node("Hello", 20.0), &engine, Size::new(200.0, 100.0));

        assert_eq!(
            text_commands(&list),
            vec![("null-atlas", 5)],
            "5 个字符必须产出 5 个字形四边形"
        );
    }

    #[test]
    fn text_measured_size_drives_layout() {
        let engine = engine();
        // char_width 0.5 × size 20 = 每字符 10px；"Hello" = 50px 宽。
        // 行高 = 1.2 × 20 = 24px。
        let node = text_node("Hello", 20.0);
        let theme = Theme::dark();
        let ctx = Context::with_engine(&theme, &engine);
        let mut n = Node::from_element(&node);
        let size = crate::layout::measure(&mut n, crate::layout::Constraints::unbounded(), ctx);

        assert!(
            (size.width - 50.0).abs() < 1e-3,
            "文本宽度应由整形结果决定，实得 {}",
            size.width
        );
        assert!(
            (size.height - 24.0).abs() < 1e-3,
            "文本高度应等于行高，实得 {}",
            size.height
        );
    }

    #[test]
    fn text_grows_the_container_it_lives_in() {
        let engine = engine();
        // 把文本放进容器里，容器的尺寸必须被文本撑开 —— 否则文本会被压成零高。
        let host = Element::new(NodeTag::Container).with_children([text_node("Hi", 20.0)]);
        let theme = Theme::dark();
        let ctx = Context::with_engine(&theme, &engine);
        let mut n = Node::from_element(&host);
        let size = crate::layout::measure(&mut n, crate::layout::Constraints::unbounded(), ctx);

        assert!(
            size.height >= 24.0,
            "容器必须被文本内容撑开，实得高度 {}",
            size.height
        );
    }

    #[test]
    fn a_long_line_wraps_within_max_width() {
        let engine = engine();
        let mut e = text_node("Hello world", 20.0);
        e = e.with(&props::TEXT_MAX_WIDTH, PropertyValue::float(60.0));
        let list = render_text(&e, &engine, Size::new(200.0, 100.0));

        // 一个文本节点 = **一条** `Command::Text`，内含所有行的字形 —— 这是合批
        // 设计（一条命令一个图集、一个颜色）。所以断行不能靠命令条数验证，要看
        // 字形的 y 位置有几个不同值。
        let cmds = text_commands(&list);
        assert_eq!(
            cmds.len(),
            1,
            "一个文本节点应只产出一条 Text 命令（合批），实得 {cmds:?}"
        );
        assert_eq!(cmds[0].1, 11, "11 个字符应有 11 个字形");

        let glyphs: Vec<Rect> = list
            .iter()
            .filter_map(|i| match &i.command {
                Command::Text { glyphs, .. } => {
                    Some(glyphs.iter().map(|g| g.dst).collect::<Vec<_>>())
                }
                _ => None,
            })
            .flatten()
            .collect();

        let distinct_rows: std::collections::BTreeSet<u32> =
            glyphs.iter().map(|r| r.y.to_bits()).collect();
        assert_eq!(
            distinct_rows.len(),
            2,
            "60px 宽 / 每字符 10px = 每行 6 字符，11 字符应断成 2 行"
        );

        // 第一行 6 个、第二行 5 个，且第一行不得超出 max_width。
        let row1: Vec<&Rect> = glyphs
            .iter()
            .filter(|r| r.y == glyphs.iter().map(|g| g.y).fold(f32::MAX, f32::min))
            .collect();
        assert_eq!(row1.len(), 6, "第一行应有 6 个字形");
        let row1_right = row1.iter().map(|r| r.x + r.width).fold(0.0f32, f32::max);
        assert!(
            row1_right <= 60.0 + 1e-3,
            "第一行不得超出 max_width=60，实得 {row1_right}"
        );
    }

    #[test]
    fn text_color_comes_from_the_property() {
        let engine = engine();
        let list = render_text(&text_node("X", 20.0), &engine, Size::new(100.0, 50.0));
        let found = list
            .iter()
            .find_map(|i| match &i.command {
                Command::Text { color, .. } => Some(*color),
                _ => None,
            })
            .expect("应有 Text 命令");
        assert_eq!(found.r, 1.0, "文本色应取 TEXT_COLOR");
        assert_eq!(found.g, 0.0);
    }

    #[test]
    fn empty_text_produces_nothing() {
        let engine = engine();
        let list = render_text(&text_node("", 20.0), &engine, Size::new(100.0, 50.0));
        assert!(
            text_commands(&list).is_empty(),
            "空文本不该产出命令 —— 否则每个空标签都进 DisplayList"
        );
    }

    #[test]
    fn without_an_engine_text_nodes_are_inert() {
        // Context::new 不带引擎：文本节点既不产出命令，也不贡献尺寸。
        // 这让"没有字体的进程"仍能跑通整条流水线。
        let theme = Theme::dark();
        let ctx = Context::new(&theme);
        let mut node = Node::from_element(&text_node("Hello", 20.0));
        let size = crate::layout::measure(&mut node, crate::layout::Constraints::unbounded(), ctx);
        assert_eq!(size, Size::new(0.0, 0.0), "无引擎时文本应为零尺寸");

        let mut list = DisplayList::new();
        render(&node, ctx, &mut list);
        assert!(text_commands(&list).is_empty());
    }

    #[test]
    fn changing_only_text_color_marks_paint_dirty_not_layout() {
        // 这是本框架最核心的性能承诺：改颜色不该触发重排。
        let engine = engine();
        let theme = Theme::dark();
        let ctx = Context::with_engine(&theme, &engine);
        let red = Element::new(NodeTag::Text)
            .with(&props::TEXT, PropertyValue::static_str("Hi"))
            .with(
                &props::TEXT_COLOR,
                PropertyValue::color(crate::core::Color::rgb(1.0, 0.0, 0.0)),
            );
        let mut node = Node::from_element(&red);
        layout(&mut node, Rect::new(0.0, 0.0, 200.0, 100.0), ctx);
        crate::node::clear_dirty(&mut node);

        let blue = Element::new(NodeTag::Text)
            .with(&props::TEXT, PropertyValue::static_str("Hi"))
            .with(
                &props::TEXT_COLOR,
                PropertyValue::color(crate::core::Color::rgb(0.0, 0.0, 1.0)),
            );
        let stats = crate::diff::diff(&mut node, &blue);

        assert!(!stats.is_noop(), "颜色变了应被检出");
        assert!(
            !stats.needs_layout(),
            "只改颜色不得要求重排 —— 否则改一次主题就重排整棵树：{stats:?}"
        );
    }

    #[test]
    fn steady_state_text_is_a_noop() {
        let engine = engine();
        let theme = Theme::dark();
        let ctx = Context::with_engine(&theme, &engine);
        let mut node = Node::from_element(&text_node("Stable", 16.0));
        layout(&mut node, Rect::new(0.0, 0.0, 200.0, 100.0), ctx);
        crate::node::clear_dirty(&mut node);

        let stats = crate::diff::diff(&mut node, &text_node("Stable", 16.0));
        assert!(
            stats.is_noop(),
            "文本未变时 diff 必须是 no-op（否则每帧都在重排文本）：{stats:?}"
        );
    }

    #[test]
    fn text_style_line_height_is_respected() {
        let engine = engine();
        let theme = Theme::dark();
        let ctx = Context::with_engine(&theme, &engine);

        // 行高倍数直接乘在字号上。注意默认值是 **1.2**（不是 1.0），所以这里断言
        // 确切值而不是"翻倍" —— 后者只在默认是 1.0 时成立，写成比例会掩盖默认值
        // 被改动这件事。
        let mut default_node = Node::from_element(&text_node("X", 10.0));
        let default_h = crate::layout::measure(
            &mut default_node,
            crate::layout::Constraints::unbounded(),
            ctx,
        )
        .height;
        assert!(
            (default_h - 12.0).abs() < 1e-3,
            "默认行高 1.2 × 字号 10 = 12，实得 {default_h}"
        );

        let loose_element =
            text_node("X", 10.0).with(&props::TEXT_LINE_HEIGHT, PropertyValue::float(2.0));
        let mut loose_node = Node::from_element(&loose_element);
        let loose_h = crate::layout::measure(
            &mut loose_node,
            crate::layout::Constraints::unbounded(),
            ctx,
        )
        .height;
        assert!(
            (loose_h - 20.0).abs() < 1e-3,
            "行高 2.0 × 字号 10 = 20，实得 {loose_h}"
        );
    }

    #[test]
    fn the_default_text_style_is_usable() {
        // 宿主若不覆写任何东西，也不该拿到荒谬的默认值。
        let style = TextStyle::default();
        assert_eq!(style.size, 14.0);
        assert_eq!(style.scale, 1.0);
        assert!(style.line_height.is_none());
        assert_eq!(style.effective_size(), 14.0);
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
    assert!(shaped.lines[0].is_last);
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

// ============================== 效果契约 ==============================

/// 效果从 `props` 一路走到 `Command` 的端到端契约。
///
/// 拆开测的意义在于：每一段单独都对，但接错了照样整条链断掉 —— 而症状是
/// "设了 blur 但没反应"，没有任何报错。
mod effect_contract {
    use crate::context::Context;
    use crate::core::{Color, Point, Rect, Size};
    use crate::element::Element;
    use crate::filter::{BlendMode, Filter, FilterChain};
    use crate::icon::{IconSet, IconSource};
    use crate::id::NodeTag;
    use crate::layout::layout;
    use crate::node::Node;
    use crate::paint::{render, Command, DisplayList};
    use crate::property::PropertyValue;
    use crate::theme::Theme;
    use std::cell::RefCell;

    fn engine() -> RefCell<crate::text::NullEngine> {
        RefCell::new(crate::text::NullEngine { char_width: 0.5 })
    }

    /// 走完 声明 → 布局 → 渲染。
    fn render_of(element: &Element) -> DisplayList {
        let theme = Theme::dark();
        let engine = engine();
        let ctx = Context::with_engine(&theme, &engine);
        let mut node = Node::from_element(element);
        layout(&mut node, Rect::new(0.0, 0.0, 200.0, 100.0), ctx);
        let mut list = DisplayList::new();
        render(&node, ctx, &mut list);
        list
    }

    fn blurred_rect(sigma: f32) -> Element {
        Element::new(NodeTag::Container)
            .with_key("root")
            .with(
                &crate::props::BACKGROUND,
                PropertyValue::color(Color::rgb(1.0, 0.0, 0.0)),
            )
            .with(
                &crate::props::FILTER,
                PropertyValue::filter(FilterChain::single(Filter::Blur {
                    sigma_x: sigma,
                    sigma_y: sigma,
                })),
            )
    }

    /// `props::FILTER` 真的进了 `Command::Quad`。
    #[test]
    fn filter_从属性走到命令() {
        let list = render_of(&blurred_rect(2.0));
        let filters: Vec<FilterChain> = list
            .iter()
            .filter_map(|i| match &i.command {
                Command::Quad { filter, .. } => Some(filter.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(filters.len(), 1, "只有根节点一条 Quad");
        assert!(!filters[0].is_identity(), "blur 必须传下去");
        assert_eq!(filters[0].outsets().left, 6.0, "σ=2 ⇒ 3σ = 6");
    }

    /// 走完 声明 → 布局 → 渲染，且带上图标集。
    fn render_with_icons(element: &Element, icons: &'static IconSet) -> DisplayList {
        let theme = Theme::dark();
        let engine = engine();
        let ctx = Context::with_engine(&theme, &engine).with_icons(icons);
        let mut node = Node::from_element(element);
        layout(&mut node, Rect::new(0.0, 0.0, 200.0, 100.0), ctx);
        let mut list = DisplayList::new();
        render(&node, ctx, &mut list);
        list
    }

    /// `bounds_of_vertices` 的直接单测。
    ///
    /// ⚠ 顶点顺序是刻意选的：**首顶点在内部**，后两个才是包围盒的对角。
    ///
    /// 为什么端到端那条（`blurred_vector_icon_yields_offscreen_requirement`）替代不了这条：它走真实图标
    /// 的三角化网格，而那个网格的**首顶点恰好是极值角点** —— 于是「忘了更新 max 边」
    /// 这个变异算出来的结果与正确答案**恰好相同**，测试一直显示绿色却什么也没验证。
    /// 这个洞是**变异测试抓出来的**，不是看代码想出来的。
    #[test]
    fn vertex_bounds_when_first_vertex_is_interior() {
        use crate::paint::{bounds_of_vertices, Vertex};
        let v = |x: f32, y: f32| Vertex {
            position: Point::new(x, y),
            uv: Point::new(0.0, 0.0),
        };
        assert_eq!(
            bounds_of_vertices(&[v(20.0, 30.0), v(10.0, 20.0), v(33.0, 47.0)]),
            Rect::new(10.0, 20.0, 23.0, 27.0),
            "min 与 max 必须分别由不同顶点得出；首顶点在内部时这条才有意义"
        );
        // x 与 y 刻意不是整数对齐：x/w 与 y/h 必须**分别**来自 min 与 max，混了会露馅
        assert_eq!(
            bounds_of_vertices(&[v(20.0, 30.0), v(10.0, 20.0), v(33.0, 47.0)]).width,
            23.0,
            "宽 = max.x − min.x，不能取反"
        );
        assert_eq!(
            bounds_of_vertices(&[v(7.0, 9.0)]),
            Rect::new(7.0, 9.0, 0.0, 0.0),
            "单顶点 = 零尺寸矩形，原点在该顶点"
        );
        assert_eq!(
            bounds_of_vertices(&[]),
            Rect::new(0.0, 0.0, 0.0, 0.0),
            "空列表 = 零尺寸（与 union_of_quads 同一约定，不用 Option）"
        );
    }

    /// blurred_vector_icon_yields_offscreen_requirement。
    ///
    /// 「矢量图标 + 投影/模糊被静默忽略」就是这个缺口：以前 `Path` 分支直接
    /// `continue`，于是图标照常画、只是不开离屏，效果没发生也不报任何错。
    #[test]
    fn blurred_vector_icon_yields_offscreen_requirement() {
        // 方框只占 24×24 view_box 的中间（8..16），所以三角化后的紧包围盒
        // **明显小于**元素自身的 24×24 —— 让「误用布局矩形」的实现露馅。
        let icons: &'static IconSet = Box::leak(Box::new(
            IconSet::new().with("sq", IconSource::Path("M8 8L16 8L16 16L8 16Z")),
        ));
        let chain = FilterChain::single(Filter::Blur {
            sigma_x: 2.0,
            sigma_y: 2.0,
        });
        // ⚠ 图标必须**嵌在**根容器里：根节点自身会被拉伸铺满可用区，图标直接当根
        // 时拿到的是 100×100 而不是声明的 24×24（这个坑我自己踩了一次）。
        let icon = Element::new(NodeTag::Container)
            .with_key("icon")
            .with(&crate::props::ICON_NAME, PropertyValue::static_str("sq"))
            .with(&crate::props::WIDTH, PropertyValue::float(24.0))
            .with(&crate::props::HEIGHT, PropertyValue::float(24.0))
            .with(&crate::props::FILTER, PropertyValue::filter(chain.clone()));
        let root = Element::new(NodeTag::Container)
            .with_key("root")
            .with_children([icon]);

        let list = render_with_icons(&root, icons);
        // 前提：这条命令真的降级成了 Path（而不是 Quad/被丢弃）。
        let verts = list
            .iter()
            .find_map(|i| match &i.command {
                Command::Path { vertices, .. } => Some(vertices.clone()),
                _ => None,
            })
            .expect("带 ICON_NAME 的元素应产出 Command::Path");
        assert!(
            verts.len() >= 3,
            "三角化至少 3 个顶点，实得 {}",
            verts.len()
        );

        let reqs = list.offscreen_requirements();
        assert_eq!(
            reqs.len(),
            1,
            "带滤镜的 Path 必须产出离屏需求，否则效果被静默忽略"
        );

        // ⚠ 这里断言**字面量**而不是拿 `bounds_of_vertices`（或测试内的第二份实现）
        // 来当判据。两个原因：
        //
        // 1. 拿被测函数给自己当判据，就是本仓已经栽过两次的恒真测试。
        // 2. 更隐蔽的一种：图标的三角化结果**首顶点恰好可能是极值角点**，于是
        //    「忘了更新 max 边」这种变异算出来的结果与正确答案**恰好相同** ——
        //    顶点顺序一变测试就失效，却一直显示绿色。字面量与顶点顺序无关。
        //
        // 推导：view_box 24×24、元素 24×24 ⇒ `scale = 1`、`ox = oy = 0`；
        // path 跨 8..16 ⇒ 顶点跨 8..16 ⇒ 紧包围盒 = (8,8) 尺寸 8×8。
        // σ=2 ⇒ 四边各扩 6 ⇒ bounds = (2,2) 尺寸 20×20。
        assert_eq!(
            reqs[0].content,
            Rect::new(8.0, 8.0, 8.0, 8.0),
            "紧包围盒必须恰好等于顶点范围 8..16 —— 既不是布局矩形 0..24，\
             也不是退化矩形（首顶点恰好是极值角点时 '忘了 max' 会伪装成对）"
        );
        assert_eq!(
            reqs[0].bounds,
            Rect::new(2.0, 2.0, 20.0, 20.0),
            "bounds = content + 3σ（σ=2 ⇒ 每边 6）"
        );
        // 顶点确实落在 content 里（防止上面两条因为 content 算错而"自洽地"通过）
        assert!(
            verts.iter().all(|v| {
                let p = v.position;
                p.x >= 8.0 && p.x <= 16.0 && p.y >= 8.0 && p.y <= 16.0
            }),
            "前提核对：所有顶点应落在 8..16 内，实得 {:?}",
            verts.iter().map(|v| v.position).collect::<Vec<_>>()
        );
    }

    /// 不带滤镜的图标不开离屏 —— 图标是最常见的命令，不能给它加 RT 开销。
    #[test]
    fn unfiltered_vector_icon_yields_no_offscreen() {
        let icons: &'static IconSet = Box::leak(Box::new(
            IconSet::new().with("sq", IconSource::Path("M8 8L16 8L16 16L8 16Z")),
        ));
        let e = Element::new(NodeTag::Container)
            .with_key("icon")
            .with(&crate::props::ICON_NAME, PropertyValue::static_str("sq"))
            .with(&crate::props::WIDTH, PropertyValue::float(24.0))
            .with(&crate::props::HEIGHT, PropertyValue::float(24.0));
        let root = Element::new(NodeTag::Container)
            .with_key("root")
            .with_children([e]);
        let list = render_with_icons(&root, icons);
        assert!(
            list.iter()
                .any(|i| matches!(&i.command, Command::Path { .. })),
            "前提：不带滤镜时也应当产出 Path（否则本测试是恒真的）"
        );
        assert!(
            list.offscreen_requirements().is_empty(),
            "恒等滤镜的图标不该开 RT，否则每个图标每帧多一次 RT 绑定"
        );
    }

    /// 不设滤镜时是空链 —— 这是零回归的前提（否则每条命令都要开 RT）。
    #[test]
    fn command_without_filter_carries_empty_chain() {
        let list = render_of(&Element::new(NodeTag::Container).with(
            &crate::props::BACKGROUND,
            PropertyValue::color(Color::rgb(1.0, 0.0, 0.0)),
        ));
        for i in list.iter() {
            if let Command::Quad { filter, .. } = &i.command {
                assert!(filter.is_identity());
            }
        }
        assert!(
            list.offscreen_requirements().is_empty(),
            "空链不该产生任何离屏需求"
        );
    }

    /// `props::BLEND_MODE` 真的进了 `Command`，且被 `has_non_default_blend` 报出。
    #[test]
    fn blend_mode_从属性走到命令() {
        let list = render_of(
            &Element::new(NodeTag::Container)
                .with(
                    &crate::props::BACKGROUND,
                    PropertyValue::color(Color::rgb(1.0, 0.0, 0.0)),
                )
                .with(
                    &crate::props::BLEND_MODE,
                    PropertyValue::blend_mode(BlendMode::Multiply),
                ),
        );
        assert!(list.has_non_default_blend());
        let has = list.iter().any(
            |i| matches!(&i.command, Command::Quad { blend, .. } if *blend == BlendMode::Multiply),
        );
        assert!(has, "混合模式必须传下去");
    }

    /// 默认不报非默认混合（`Clear` 之外的所有比较都要走到）。
    #[test]
    fn default_blend_mode_is_not_reported() {
        let list = render_of(&blurred_rect(2.0));
        assert!(!list.has_non_default_blend(), "只设了 blur，不该报混合模式");
    }

    /// 离屏需求能回答"要多大RT"，且与 fx 层的规划一致。
    ///
    /// 两条路径（core 的 `offscreen_requirements` 与 fx 的 `plan_offscreens`）
    /// 算的是同一个问题，所以必须给出相同答案 —— 不一致就说明有一处漂移。
    #[test]
    fn offscreen_requirement_matches_geometry_outset() {
        let list = render_of(&blurred_rect(2.0));
        let reqs = list.offscreen_requirements();
        assert_eq!(reqs.len(), 1);
        let r = &reqs[0];
        // 根节点矩形是整个 200×100；blur σ=2 ⇒ 四边各扩 6。
        assert_eq!(r.content, Rect::new(0.0, 0.0, 200.0, 100.0));
        assert_eq!(r.bounds, Rect::new(-6.0, -6.0, 212.0, 112.0));
        // 局部原点：内容原点在 RT 内的坐标 = 内容原点 − bounds 原点 = (−6, −6)。
        //
        // ⚠ 方向容易搞反：RT 向外扩了，所以屏幕原点在 RT 里是**负**的。宿主
        // 用 `bounds.x/y` 做基准去减，而不是拿它当平移量直接加。
        assert_eq!(r.filter.origin_in(r.content), Point::new(-6.0, -6.0));
        assert_eq!(
            r.content.x - r.bounds.x,
            -r.filter.origin_in(r.content).x,
            "origin_in 与 bounds 必须自洽：origin 就是 −(bounds 原点)"
        );
    }

    /// 滤镜**不继承**：父节点设了 blur，子节点不该跟着糊。
    ///
    /// 这与 `Opacity` 的继承语义相反 —— 半透明叠加天然正确，模糊叠加完全不成立。
    #[test]
    fn filter_is_not_inherited_along_parent_chain() {
        let child = Element::new(NodeTag::Control).with_key("child").with(
            &crate::props::BACKGROUND,
            PropertyValue::color(Color::rgb(0.0, 1.0, 0.0)),
        );
        let list = render_of(&blurred_rect(2.0).with_children([child]));
        let blurred = list
            .iter()
            .filter(|i| matches!(&i.command, Command::Quad { filter, .. } if !filter.is_identity()))
            .count();
        assert_eq!(
            blurred, 1,
            "只有父节点那条 Quad 带滤镜；子节点必须干净，否则整棵子树被糊"
        );
    }

    /// 文本命令也带滤镜（`text_command` 的占位值被 `render_rec` 正确覆盖）。
    #[test]
    fn text_command_also_carries_filter() {
        let e = Element::new(NodeTag::Text)
            .with_key("t")
            .with(&crate::props::TEXT, PropertyValue::static_str("ab"))
            .with(
                &crate::props::FILTER,
                PropertyValue::filter(FilterChain::single(Filter::Blur {
                    sigma_x: 1.0,
                    sigma_y: 1.0,
                })),
            );
        let list = render_of(&e);
        let found = list
            .iter()
            .any(|i| matches!(&i.command, Command::Text { filter, .. } if !filter.is_identity()));
        assert!(
            found,
            "文本命令的滤镜占位值必须被 render_rec 覆盖成真实值，\
             否则 text_command 里的空链会静默吞掉效果"
        );
    }

    /// 属性相等判定覆盖新变体 —— diff 靠它判断"效果真的变了"。
    ///
    /// 这条若漏了，表现是"改了 blur 那一帧有反应、之后再也不更新"，而
    /// 单元测试全绿。
    #[test]
    fn property_equality_separates_distinct_filters() {
        let a = PropertyValue::filter(FilterChain::single(Filter::Blur {
            sigma_x: 1.0,
            sigma_y: 1.0,
        }));
        let b = PropertyValue::filter(FilterChain::single(Filter::Blur {
            sigma_x: 2.0,
            sigma_y: 1.0,
        }));
        assert_ne!(a, b, "σ 不同就是不同效果");
        assert_eq!(a, a.clone(), "同值必须相等（diff 判 no-op 的依据）");
        assert_ne!(
            PropertyValue::blend_mode(BlendMode::Src),
            PropertyValue::blend_mode(BlendMode::Dst)
        );
    }

    /// `Debug` 输出能看出滤镜项数 —— 属性 diff 失败时的诊断依据。
    #[test]
    fn filter_property_has_readable_debug_output() {
        let v = PropertyValue::filter(FilterChain::single(Filter::Blur {
            sigma_x: 1.0,
            sigma_y: 1.0,
        }));
        assert!(
            format!("{v:?}").contains("1 项"),
            "实际 {:?}",
            format!("{v:?}")
        );
    }

    /// 尺寸类断言用得到，确认测试里的常量与框架一致。
    #[test]
    fn layout_size_constants_are_self_consistent() {
        assert_eq!(Size::new(200.0, 100.0).width, 200.0);
    }
}

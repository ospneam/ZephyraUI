//! 属性驱动的布局与绘制——即"声明 → 布局 → DisplayList"这条主链路的测试。
//!
//! 这批测试补的是骨架阶段留下的最大缺口：当时 `paint::render` 是一个桩，对每个
//! 节点无条件画同一条硬编码 Quad，于是"属性系统 / 主题 / 绘制"三个模块各自存在
//! 却从未接在一起。这里第一次让它们串起来跑，并断言串起来之后的可见行为。

use crate::core::{Color, Rect, UvRect};
use crate::layout::{AlignItems, Direction, MainAxisAlign};
use crate::paint::{Command, Layer};
use crate::property::PropertyValue;
use crate::props;
use crate::theme::{roles, Theme};
use crate::context::Context;
use crate::diff::diff;
use crate::element::Element;
use crate::id::NodeTag;
use crate::layout::layout;
use crate::node::Node;
use crate::paint::render;

/// 这批测试大多只关心几何与绘制，用深色主题即可；需要对比主题的用例自建 `Context`。
fn ctx() -> Context<'static> {
    crate::context::dark_context()
}

// ============================== 辅助 ==============================

fn quad(color: Color) -> Element {
    Element::new(NodeTag::Container).with(&props::BACKGROUND, PropertyValue::color(color))
}

fn sized(w: f32, h: f32) -> impl Fn(Element) -> Element {
    move |e| {
        e.with(&props::WIDTH, PropertyValue::float(w))
            .with(&props::HEIGHT, PropertyValue::float(h))
    }
}

fn laid_out(element: &Element, bounds: Rect) -> Node {
    let mut node = Node::from_element(element);
    layout(&mut node, bounds, ctx());
    node
}

/// 把被测元素包进一个铺满 `bounds` 的容器，返回**被测的那个子节点**。
///
/// # 为什么需要这个辅助
///
/// 根节点**总是**铺满传入的 `bounds`——那是"填满窗口"的正确语义。于是任何"这个节点
/// 有多大"的断言，若直接拿根节点去量，得到的必然是 bounds 而不是它自己声明的尺寸。
/// 这不是 bug，是层级语义：根由宿主给定大小，尺寸声明只对**子项**有意义。
///
/// 所以凡是断言某个节点自身尺寸的用例，都必须把它放进容器里当子项。
fn as_child(element: Element, bounds: Rect) -> Node {
    let root = Element::new(NodeTag::Container)
        .with(&props::DIRECTION, PropertyValue::direction(Direction::Horizontal))
        .with_children([element]);
    let mut wrapper = Node::from_element(&root);
    layout(&mut wrapper, bounds, ctx());
    wrapper.take_only_child()
}

fn commands(node: &Node) -> Vec<Command> {
    let mut list = crate::paint::DisplayList::new();
    render(node, ctx(), &mut list);
    list.iter().map(|i| i.command.clone()).collect()
}

fn quad_rects(node: &Node) -> Vec<Rect> {
    commands(node)
        .into_iter()
        .filter_map(|c| match c {
            Command::Quad { rect, .. } => Some(rect),
            _ => None,
        })
        .collect()
}

/// 图像节点的 `src`。`None` = 整张纹理。
fn image_src(node: &Node) -> Vec<Option<UvRect>> {
    commands(node)
        .into_iter()
        .filter_map(|c| match c {
            Command::Image { src, .. } => Some(src),
            _ => None,
        })
        .collect()
}

/// 带一张纹理的图像节点。`src = None` 表示"用默认（整张）"。
fn image(src: Option<UvRect>) -> Element {
    let mut e = Element::new(NodeTag::Image)
        .with(&props::IMAGE_TEXTURE, PropertyValue::static_str("atlas.png"));
    if let Some(uv) = src {
        e = e.with(&props::IMAGE_SRC, PropertyValue::uv_rect(uv));
    }
    e
}

// ============================== 布局 ==============================

#[test]
fn horizontal_stack_places_children_left_to_right_with_spacing() {
    let e = Element::new(NodeTag::Container)
        .with(&props::DIRECTION, PropertyValue::direction(Direction::Horizontal))
        .with(&props::SPACING, PropertyValue::float(10.0))
        .with_children([sized(40.0, 20.0)(Element::new(NodeTag::Control)), sized(40.0, 20.0)(Element::new(NodeTag::Control))]);
    let node = laid_out(&e, Rect::new(0.0, 0.0, 500.0, 100.0));

    let rects: Vec<Rect> = node.children().iter().map(|c| c.rect()).collect();
    assert_eq!(rects.len(), 2);
    assert_eq!((rects[0].x, rects[0].width), (0.0, 40.0));
    assert_eq!(
        (rects[1].x, rects[1].width),
        (50.0, 40.0),
        "第二个子项应让开一个 10px 间距"
    );
}

#[test]
fn vertical_stack_stacks_downward() {
    let e = Element::new(NodeTag::Container)
        .with(&props::DIRECTION, PropertyValue::direction(Direction::Vertical))
        .with(&props::SPACING, PropertyValue::float(4.0))
        .with_children([sized(30.0, 10.0)(Element::new(NodeTag::Control)), sized(30.0, 10.0)(Element::new(NodeTag::Control))]);
    let node = laid_out(&e, Rect::new(0.0, 0.0, 100.0, 400.0));

    let rects: Vec<Rect> = node.children().iter().map(|c| c.rect()).collect();
    assert_eq!((rects[0].y, rects[0].height), (0.0, 10.0));
    assert_eq!((rects[1].y, rects[1].height), (14.0, 10.0));
}

#[test]
fn explicit_size_beats_content_size() {
    // 内容只有 10px 宽，但显式宽度 120 必须胜出——否则布局器只能靠父容器猜。
    // 注意用 `as_child`：根节点恒等于 bounds，尺寸声明只对子项生效。
    let e = sized(120.0, 30.0)(quad(Color::rgb(1.0, 0.0, 0.0)));
    let node = as_child(e, Rect::new(0.0, 0.0, 500.0, 500.0));
    assert_eq!(node.rect().width, 120.0);
    assert_eq!(node.rect().height, 30.0);
}

#[test]
fn the_root_always_fills_the_bounds_it_is_given() {
    // 与上一条互为对照：根节点**不**遵守自己的 Width/Height。
    // 这是刻意的——窗口多大根就多大，否则根收缩后背景只画一小块，
    // 窗口其余部分会露出上一个 frame 的残影。
    let node = laid_out(
        &sized(120.0, 30.0)(quad(Color::rgb(1.0, 0.0, 0.0))),
        Rect::new(0.0, 0.0, 500.0, 400.0),
    );
    assert_eq!(node.rect(), Rect::new(0.0, 0.0, 500.0, 400.0));
}

#[test]
fn content_never_exceeds_the_constraint_maximum() {
    // 子项要 1000px 宽，但只给了 200 ——必须被裁到 200，否则一个长列表会把窗口顶出去。
    let e = Element::new(NodeTag::Container)
        .with(&props::DIRECTION, PropertyValue::direction(Direction::Horizontal))
        .with_children([sized(1000.0, 10.0)(Element::new(NodeTag::Control))]);
    let node = laid_out(&e, Rect::new(0.0, 0.0, 200.0, 100.0));
    assert!(
        node.rect().width <= 200.0,
        "内容溢出应被约束裁剪，实得 {}",
        node.rect().width
    );
}

#[test]
fn padding_insets_the_children() {
    let e = Element::new(NodeTag::Container)
        .with(&props::PADDING, PropertyValue::float(8.0))
        .with_children([sized(20.0, 20.0)(Element::new(NodeTag::Control))]);
    let node = laid_out(&e, Rect::new(100.0, 50.0, 300.0, 200.0));
    let child = node.children()[0].rect();
    assert_eq!(child.x, 108.0, "左内边距应把子项推过去");
    assert_eq!(child.y, 58.0);
}

#[test]
fn cross_align_stretch_makes_children_fill_the_cross_axis() {
    // 工具栏的"组容器撑满高度"就是这个。
    let e = Element::new(NodeTag::Container)
        .with(&props::DIRECTION, PropertyValue::direction(Direction::Horizontal))
        .with(&props::CROSS_ALIGN, PropertyValue::cross_align(AlignItems::Stretch))
        .with_children([sized(40.0, 10.0)(Element::new(NodeTag::Control))]);
    let node = laid_out(&e, Rect::new(0.0, 0.0, 300.0, 56.0));
    assert_eq!(
        node.children()[0].rect().height,
        56.0,
        "Stretch 应让子项撑满交叉轴"
    );
}

#[test]
fn cross_align_center_centers_on_the_cross_axis() {
    let e = Element::new(NodeTag::Container)
        .with(&props::DIRECTION, PropertyValue::direction(Direction::Horizontal))
        .with(&props::CROSS_ALIGN, PropertyValue::cross_align(AlignItems::Center))
        .with_children([sized(40.0, 10.0)(Element::new(NodeTag::Control))]);
    let node = laid_out(&e, Rect::new(0.0, 0.0, 300.0, 50.0));
    assert_eq!(node.children()[0].rect().y, 20.0, "(50-10)/2 = 20");
}

#[test]
fn main_align_space_between_pushes_the_ends_apart() {
    let e = Element::new(NodeTag::Container)
        .with(&props::DIRECTION, PropertyValue::direction(Direction::Horizontal))
        .with(&props::MAIN_ALIGN, PropertyValue::main_align(MainAxisAlign::SpaceBetween))
        .with_children([sized(40.0, 10.0)(Element::new(NodeTag::Control)), sized(40.0, 10.0)(Element::new(NodeTag::Control))]);
    let node = laid_out(&e, Rect::new(0.0, 0.0, 200.0, 20.0));
    let rects: Vec<Rect> = node.children().iter().map(|c| c.rect()).collect();
    assert_eq!(rects[0].x, 0.0);
    assert_eq!(rects[1].x, 160.0, "末项应贴住右边缘");
}

#[test]
fn collapsed_children_take_no_space_at_all() {
    // 与"宽度为 0"不同：折叠的子项连间距都不占。工具栏在图片模式下少一个按钮时，
    // 间距必须跟着消失，而不是留下一个 10px 的空洞。
    let e = Element::new(NodeTag::Container)
        .with(&props::DIRECTION, PropertyValue::direction(Direction::Horizontal))
        .with(&props::SPACING, PropertyValue::float(10.0))
        .with_children([
            sized(40.0, 20.0)(Element::new(NodeTag::Control)),
            sized(40.0, 20.0)(
                Element::new(NodeTag::Control).with(&props::COLLAPSED, PropertyValue::bool(true))
            ),
            sized(40.0, 20.0)(Element::new(NodeTag::Control)),
        ]);
    let node = laid_out(&e, Rect::new(0.0, 0.0, 500.0, 100.0));

    // 参与布局的只有第 0 与第 2 项：第二个起点 = 40（首项）+ 10（间距）= 50。
    // 折叠项既不占位也不占间距，所以第 2 项不会因为中间那个被推到 100。
    assert_eq!(
        node.children()[2].rect().x,
        50.0,
        "折叠项应完全消失，连它的间距一起消失"
    );
    assert_eq!(
        node.children()[1].rect(),
        Rect::new(0.0, 0.0, 0.0, 0.0),
        "折叠项自身不应被摆出任何矩形"
    );
    assert_eq!(
        commands(&node).len(),
        0,
        "无背景的节点不产出命令——折叠项也不该有任何痕迹"
    );
}

#[test]
fn a_zero_width_sibling_still_occupies_its_spacing() {
    // 与上一条严格对照：折叠（不占位）与"宽度为 0"（仍占一个间距位）是两件事。
    // 混淆二者会让"隐藏一项"在界面上留下一个空洞。
    let e = Element::new(NodeTag::Container)
        .with(&props::DIRECTION, PropertyValue::direction(Direction::Horizontal))
        .with(&props::SPACING, PropertyValue::float(10.0))
        .with_children([
            sized(40.0, 20.0)(Element::new(NodeTag::Control)),
            sized(0.0, 0.0)(Element::new(NodeTag::Control)),
            sized(40.0, 20.0)(Element::new(NodeTag::Control)),
        ]);
    let node = laid_out(&e, Rect::new(0.0, 0.0, 500.0, 100.0));
    // 间距是"每项之后都加一次"，所以起点依次是 0、50、60：
    // 零宽项自己落在 50（= 40 + 10），它**后面**还要再加一个 10，第 3 项因此在 60。
    // 折叠那条的对应值是 50 与（无第 3 项），差别正来自"零宽仍占位"。
    assert_eq!(node.children()[1].rect().width, 0.0);
    assert_eq!(node.children()[1].rect().x, 50.0);
    assert_eq!(
        node.children()[2].rect().x,
        60.0,
        "零宽项仍占一个间距位"
    );
}

// ============================== 绘制 ==============================

#[test]
fn transparent_nodes_produce_no_commands() {
    let node = laid_out(&Element::new(NodeTag::Container), Rect::new(0.0, 0.0, 10.0, 10.0));
    assert!(
        commands(&node).is_empty(),
        "全透明的树应产出空 DisplayList——否则一层空容器会挡住下面的一切"
    );
}

#[test]
fn background_becomes_a_quad_at_the_node_rect() {
    let e = sized(60.0, 20.0)(quad(Color::rgb(0.2, 0.4, 0.6)));
    let node = as_child(e, Rect::new(0.0, 0.0, 500.0, 500.0));
    let cmds = quad_rects(&node);
    assert_eq!(cmds.len(), 1);
    assert_eq!(cmds[0], Rect::new(0.0, 0.0, 60.0, 20.0));
}

#[test]
fn border_rides_along_with_the_fill_in_one_command() {
    // 描边复用 Quad 的 stroke 字段，于是"圆角矩形 = 一个 draw call"才成立。
    let e = sized(40.0, 40.0)(quad(Color::rgb(0.1, 0.1, 0.1)))
        .with(&props::BORDER_COLOR, PropertyValue::color(Color::rgb(1.0, 1.0, 1.0)))
        .with(&props::BORDER_WIDTH, PropertyValue::float(2.0));
    let node = laid_out(&e, Rect::new(0.0, 0.0, 200.0, 200.0));
    let cmds = commands(&node);
    assert_eq!(cmds.len(), 1, "填充与描边必须合并成一条命令");
    match &cmds[0] {
        Command::Quad { stroke, .. } => {
            let (c, w) = stroke.expect("设了描边色与宽度就应产出描边");
            assert!((w - 2.0).abs() < 1e-6);
            assert!((c.r - 1.0).abs() < 1e-6);
        }
        other => panic!("期望 Quad，实得 {other:?}"),
    }
}

#[test]
fn corner_radius_is_taken_from_properties() {
    let e = sized(40.0, 40.0)(quad(Color::rgb(0.1, 0.1, 0.1)))
        .with(&props::CORNER_RADIUS, PropertyValue::float(12.0));
    let node = laid_out(&e, Rect::new(0.0, 0.0, 200.0, 200.0));
    match &commands(&node)[0] {
        Command::Quad { radius, .. } => assert!((radius.radius - 12.0).abs() < 1e-6),
        other => panic!("期望 Quad，实得 {other:?}"),
    }
}

#[test]
fn image_becomes_an_image_command_centred_in_its_node() {
    let e = sized(80.0, 40.0)(
        Element::new(NodeTag::Image)
            .with(
                &props::IMAGE_TEXTURE,
                PropertyValue::static_str("atlas/pen"),
            )
            .with(&props::IMAGE_SIZE, PropertyValue::float(18.0)),
    );
    let node = as_child(e, Rect::new(0.0, 0.0, 200.0, 200.0));
    let imgs: Vec<Rect> = commands(&node)
        .into_iter()
        .filter_map(|c| match c {
            Command::Image { dst, .. } => Some(dst),
            _ => None,
        })
        .collect();
    assert_eq!(imgs.len(), 1);
    // 18px 图标在 80x40 的节点里居中：(80-18)/2 = 31, (40-18)/2 = 11
    assert!((imgs[0].x - 31.0).abs() < 1e-4);
    assert!((imgs[0].y - 11.0).abs() < 1e-4);
    assert!((imgs[0].width - 18.0).abs() < 1e-4);
}

#[test]
fn image_fills_the_node_when_no_size_is_given() {
    let e = sized(50.0, 25.0)(
        Element::new(NodeTag::Image).with(
            &props::IMAGE_TEXTURE,
            PropertyValue::static_str("atlas/pen"),
        ),
    );
    let node = as_child(e, Rect::new(0.0, 0.0, 200.0, 200.0));
    match &commands(&node)[0] {
        Command::Image { dst, .. } => assert_eq!(*dst, Rect::new(0.0, 0.0, 50.0, 25.0)),
        other => panic!("期望 Image，实得 {other:?}"),
    }
}

#[test]
fn children_render_after_their_parents_background() {
    // 顺序即 z 序：父背景必须在子内容之前，否则子内容被自己的父盖住。
    let e = Element::new(NodeTag::Container)
        .with(&props::BACKGROUND, PropertyValue::color(Color::rgb(0.2, 0.2, 0.2)))
        .with_children([quad(Color::rgb(0.9, 0.9, 0.9))]);
    let node = laid_out(&e, Rect::new(0.0, 0.0, 100.0, 100.0));

    let mut list = crate::paint::DisplayList::new();
    render(&node, ctx(), &mut list);
    list.sort();
    let fills: Vec<Color> = list
        .iter()
        .filter_map(|i| match &i.command {
            Command::Quad { fill, .. } => Some(*fill),
            _ => None,
        })
        .collect();
    assert_eq!(fills.len(), 2);
    assert!((fills[0].r - 0.2).abs() < 1e-6, "第一条应是父背景");
    assert!((fills[1].r - 0.9).abs() < 1e-6, "第二条才是子内容");
}

#[test]
fn layer_property_selects_the_compositing_layer() {
    let e = quad(Color::rgb(0.2, 0.2, 0.2))
        .with(&props::LAYER, PropertyValue::layer(Layer::Popup));
    let node = laid_out(&e, Rect::new(0.0, 0.0, 50.0, 50.0));
    let mut list = crate::paint::DisplayList::new();
    render(&node, ctx(), &mut list);
    assert!(list.has_layer(Layer::Popup));
    assert!(!list.has_layer(Layer::Content));
}

// ============================== 主题 ==============================

#[test]
fn resource_keys_resolve_to_concrete_theme_values() {
    // "切主题"链路的最小闭环：属性存键 → 按当前主题解析 → 驱动绘制。
    let theme = Theme::dark();
    let ctx = Context::new(&theme);
    let e = sized(10.0, 10.0)(
        Element::new(NodeTag::Container)
            .with(&props::BACKGROUND, PropertyValue::Resource(roles::SURFACE.into())),
    );
    let node = Node::from_element(&e);

    let resolved = node.background(ctx);
    assert!(
        (resolved.r - theme.surface.r).abs() < 1e-6
            && (resolved.g - theme.surface.g).abs() < 1e-6
            && (resolved.b - theme.surface.b).abs() < 1e-6,
        "解析结果应等于主题的 surface，实得 {resolved:?}"
    );
}

#[test]
fn resolving_a_resource_key_leaves_the_key_in_place() {
    // 这条钉住"读取时解析"这个设计。早期实现是把 Resource 就地替换成字面值，
    // 于是**第一次解析之后键就没了**——第二次换主题时什么都不发生，切主题成了
    // 视觉空操作，而代码读起来完全正常。
    let theme = Theme::dark();
    let ctx = Context::new(&theme);
    let e = Element::new(NodeTag::Container)
        .with(&props::BACKGROUND, PropertyValue::Resource(roles::SURFACE.into()));
    let node = Node::from_element(&e);

    let _ = node.background(ctx);
    assert!(
        node.props()
            .local_value(crate::property::PropertyStore::id_of(&props::BACKGROUND))
            .map(|v| v.is_resource())
            .unwrap_or(false),
        "解析不得销毁资源键，否则同一个键无法在第二个主题下再次解析"
    );
}

#[test]
fn switching_theme_changes_the_resolved_color() {
    let dark_theme = Theme::dark();
    let light_theme = Theme::light();
    let e = Element::new(NodeTag::Container)
        .with(&props::BACKGROUND, PropertyValue::Resource(roles::SURFACE.into()));
    let node = Node::from_element(&e);

    // 同一棵树、同一份属性，只换上下文里的主题引用。
    let dark = node.background(Context::new(&dark_theme));
    let light = node.background(Context::new(&light_theme));

    assert_ne!(
        dark, light,
        "换主题后同一个资源键必须解析出不同颜色，否则切主题在视觉上是空操作"
    );
}

#[test]
fn a_literal_value_ignores_the_theme_entirely() {
    // 字面量不该被主题影响——否则"局部覆盖"这个能力不存在。
    let dark_theme = Theme::dark();
    let light_theme = Theme::light();
    let e = Element::new(NodeTag::Container)
        .with(&props::BACKGROUND, PropertyValue::color(Color::rgb(0.3, 0.6, 0.9)));
    let node = Node::from_element(&e);
    assert_eq!(
        node.background(Context::new(&dark_theme)),
        node.background(Context::new(&light_theme)),
    );
}

#[test]
fn unknown_resource_key_resolves_to_nothing_rather_than_black() {
    // 拼错角色名时必须"解析不出"，而不是静默变成黑色——黑是合法颜色，回退成它
    // 会把一个拼写错误变成看不出成因的视觉偏差。
    let theme = Theme::dark();
    let node = Node::from_element(&Element::new(NodeTag::Container).with(
        &props::BACKGROUND,
        PropertyValue::Resource("no_such_role".into()),
    ));
    assert!(
        node.background(Context::new(&theme)).a == 0.0,
        "无法解析的资源键应读作透明（不画背景），而不是黑色"
    );
    assert!(
        node.value(&props::BACKGROUND, Context::new(&theme)).is_none(),
        "无法解析的资源键应显式报告为 None，让调用方能发现它"
    );
}

#[test]
fn spacing_resources_resolve_to_dip_values() {
    // 间距也是资源，验证 resource_value 不只认颜色。
    let theme = Theme::dark();
    let ctx = Context::new(&theme);
    let node = Node::from_element(
        &Element::new(NodeTag::Container)
            .with(&props::SPACING, PropertyValue::Resource("spacing.small".into())),
    );
    assert!((node.stack(ctx).spacing - crate::core::Spacing::Small.dip()).abs() < 1e-6);
}

// ============================== 与 diff 的衔接 ==============================

#[test]
fn changing_a_paint_property_does_not_force_relayout() {
    // 改背景色只该重绘。若它触发重排，改个颜色就会让整棵子树重新布局——
    // 这是 UI 卡顿最常见的单一原因。
    let mut node = Node::from_element(&quad(Color::rgb(0.2, 0.2, 0.2)));
    crate::node::clear_dirty(&mut node);
    layout(&mut node, Rect::new(0.0, 0.0, 100.0, 100.0), ctx());
    crate::node::clear_dirty(&mut node);

    let changed = Element::new(NodeTag::Container)
        .with(&props::BACKGROUND, PropertyValue::color(Color::rgb(0.9, 0.0, 0.0)));
    let stats = diff(&mut node, &changed);
    assert!(stats.dirtied > 0, "颜色变了应被检出");
    assert!(
        !stats.needs_layout(),
        "颜色变化不应要求重排，实得 needs_layout={}",
        stats.needs_layout
    );
}

#[test]
fn changing_spacing_forces_relayout() {
    let before = Element::new(NodeTag::Container)
        .with(&props::DIRECTION, PropertyValue::direction(Direction::Horizontal))
        .with(&props::SPACING, PropertyValue::float(2.0))
        .with_children([sized(20.0, 20.0)(Element::new(NodeTag::Control)), sized(20.0, 20.0)(Element::new(NodeTag::Control))]);
    let mut node = Node::from_element(&before);
    crate::node::clear_dirty(&mut node);
    layout(&mut node, Rect::new(0.0, 0.0, 200.0, 100.0), ctx());
    crate::node::clear_dirty(&mut node);

    let after = Element::new(NodeTag::Container)
        .with(&props::DIRECTION, PropertyValue::direction(Direction::Horizontal))
        .with(&props::SPACING, PropertyValue::float(20.0))
        .with_children([sized(20.0, 20.0)(Element::new(NodeTag::Control)), sized(20.0, 20.0)(Element::new(NodeTag::Control))]);
    let stats = diff(&mut node, &after);
    assert!(
        stats.needs_layout(),
        "间距变了必须要求重排，否则子项位置不会更新"
    );
}

#[test]
fn layout_is_stable_across_repeated_calls() {
    // 同一输入两次布局必须产出完全相同的矩形——否则"静止时零重建"无从谈起。
    let e = Element::new(NodeTag::Container)
        .with(&props::DIRECTION, PropertyValue::direction(Direction::Horizontal))
        .with(&props::SPACING, PropertyValue::float(7.0))
        .with_children([sized(33.0, 11.0)(Element::new(NodeTag::Control)), sized(17.0, 29.0)(Element::new(NodeTag::Control))]);

    let mut a = Node::from_element(&e);
    layout(&mut a, Rect::new(0.0, 0.0, 300.0, 100.0), ctx());
    let mut b = Node::from_element(&e);
    layout(&mut b, Rect::new(0.0, 0.0, 300.0, 100.0), ctx());

    let rects = |n: &Node| n.children().iter().map(|c| c.rect()).collect::<Vec<_>>();
    assert_eq!(rects(&a), rects(&b));
    assert_eq!(commands(&a), commands(&b));
}

// ============================== 图像 src 裁剪 ==============================

#[test]
fn an_image_without_src_uses_the_whole_texture() {
    // `src: None` 是"没裁剪"的语义，而不是"零尺寸"——后者在采样器里表现为完全不显示。
    let node = laid_out(&image(None), Rect::new(0.0, 0.0, 64.0, 64.0));
    assert_eq!(
        image_src(&node),
        vec![None],
        "默认应不裁剪，宿主据此可以跳过 uv 计算"
    );
}

#[test]
fn an_explicit_full_rect_also_reads_as_uncropped() {
    // 显式写整张 uv 与不写必须落到同一个结果——否则"设与不设"会画出不同画面。
    let node = laid_out(
        &image(Some(UvRect::FULL)),
        Rect::new(0.0, 0.0, 64.0, 64.0),
    );
    assert_eq!(image_src(&node), vec![None]);
}

#[test]
fn a_sub_rect_reaches_the_command() {
    // 图集右半边：核心只搬运这四个数字，不认识它对应哪张纹理。
    let half = UvRect::new(0.5, 0.0, 0.5, 1.0);
    let node = laid_out(&image(Some(half)), Rect::new(0.0, 0.0, 64.0, 64.0));
    assert_eq!(image_src(&node), vec![Some(half)]);
}

#[test]
fn an_out_of_range_src_is_dropped_rather_than_forwarded() {
    // 越界 uv 的症状在采样器里才显现（取到相邻图块的像素），那时已离病因很远。
    // 所以核心在**产生**命令时就退化成"整张"，宁可少裁一点也不产出错误采样。
    for bad in [
        UvRect::new(0.0, 0.0, 1.5, 1.0),  // 宽出界
        UvRect::new(0.0, 0.0, 1.0, 2.0),  // 高出界
        UvRect::new(0.8, 0.0, 0.5, 1.0),  // 右边越过 1.0
        UvRect::new(-0.5, 0.0, 1.0, 1.0), // 左边为负
    ] {
        let node = laid_out(&image(Some(bad)), Rect::new(0.0, 0.0, 64.0, 64.0));
        assert_eq!(
            image_src(&node),
            vec![None],
            "越界 src {bad:?} 应退化成整张，而不是被原样转发"
        );
    }
}

#[test]
fn a_zero_area_src_is_dropped() {
    // 零尺寸 uv 没有可见像素。它的正确表现是"不裁剪"还是"不画"有歧义 ——
    // 选前者，因为 `src` 为 `None` 时节点矩形与背景仍会被画出，语义连续。
    let node = laid_out(
        &image(Some(UvRect::new(0.5, 0.5, 0.0, 0.0))),
        Rect::new(0.0, 0.0, 64.0, 64.0),
    );
    assert_eq!(image_src(&node), vec![None]);
}

#[test]
fn src_does_not_affect_layout() {
    // 裁剪只改变"取纹理的哪一块"，不改变节点多大。若它触发重排，改一个 uv 会让
    // 整棵列表重排——正是这套属性系统要避免的那类卡顿。
    let mut node = Node::from_element(&image(None));
    crate::node::clear_dirty(&mut node);
    layout(&mut node, Rect::new(0.0, 0.0, 64.0, 64.0), ctx());
    crate::node::clear_dirty(&mut node);

    let cropped = image(Some(UvRect::new(0.5, 0.0, 0.5, 1.0)));
    let stats = diff(&mut node, &cropped);
    assert!(stats.dirtied > 0, "src 变了应被检出为重绘");
    assert!(
        !stats.needs_layout(),
        "改 src 不应要求重排，实得 needs_layout={}",
        stats.needs_layout
    );
}

// ============================== 嵌套 ==============================

#[test]
fn nested_containers_each_get_their_own_rect() {
    // 工具栏的真实形状：根 Row 里三个组容器，组里各有按钮。
    let button = || {
        sized(40.0, 40.0)(
            Element::new(NodeTag::Control)
                .with(&props::BACKGROUND, PropertyValue::color(Color::rgb(0.3, 0.3, 0.3))),
        )
    };
    let group = |children: Vec<Element>| {
        Element::new(NodeTag::Container)
            .with(&props::DIRECTION, PropertyValue::direction(Direction::Horizontal))
            .with(&props::SPACING, PropertyValue::float(2.0))
            .with(&props::PADDING, PropertyValue::float(10.0))
            .with(&props::BACKGROUND, PropertyValue::color(Color::rgb(0.1, 0.1, 0.1)))
            .with_children(children)
    };
    let e = Element::new(NodeTag::Container)
        .with(&props::DIRECTION, PropertyValue::direction(Direction::Horizontal))
        .with(&props::SPACING, PropertyValue::float(8.0))
        .with_children([group(vec![button(), button()]), group(vec![button()])]);

    let node = laid_out(&e, Rect::new(0.0, 0.0, 800.0, 600.0));
    let groups: Vec<Rect> = node.children().iter().map(|c| c.rect()).collect();
    assert_eq!(groups.len(), 2);
    // 组0：两按钮 40+2+40 = 82，加 padding 10*2 = 102
    assert!((groups[0].width - 102.0).abs() < 1e-3);
    // 组1：一个按钮 40 + padding 20 = 60；间距 8 → 起点 110
    assert!((groups[1].x - 110.0).abs() < 1e-3);
    assert!((groups[1].width - 60.0).abs() < 1e-3);
    // 组0 的首个按钮应落在 padding 内
    assert!((node.children()[0].children()[0].rect().x - 10.0).abs() < 1e-3);

    // 每个组容器与每个按钮都应产出背景
    assert_eq!(commands(&node).len(), 5, "2 个组 + 3 个按钮");
}
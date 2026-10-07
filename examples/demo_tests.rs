//! hover / press 的端到端断言：**不需要窗口、不需要 GPU**。
//!
//! 这一批盯的是「输入 → 状态 → 声明 → diff → DisplayList」这条链。链路止于
//! `DisplayList` —— 那正是核心与后端之间的契约，也是断言的最强位置：往下是
//! D3D11 像素，往上是输入设备，中间这段全在核心里。
//!
//! 每条断言都对应一个「曾经会静默出错」的具体故障：
//!
//! - hover 不改变颜色 → 用户看不出鼠标在哪；
//! - 移出后不清 hover → 高亮粘在旧按钮上；
//! - press 视觉不跟随 capture → 按下后拖出控件，界面还停在按下态。

use zephyra::core::{Rect, UvRect};
use zephyra::input::{
    InputRouter, PointerEvent, PointerPhase, PointerSource, RoutedEvent, RoutedEvents,
};
use zephyra::layout::{AlignItems, Direction};
use zephyra::paint::{Command, DisplayList};
use zephyra::property::PropertyValue;
use zephyra::props;
use zephyra::theme::{roles, Theme};
use zephyra::context::Context;
use zephyra::diff::diff;
use zephyra::element::Element;
use zephyra::id::{ElementKey, NodeTag};
use zephyra::layout::layout;
use zephyra::node::{clear_dirty, Node};
use zephyra::paint::render;

/// 降级成命令列表（只取命令，不取排序键）。
fn commands(node: &Node) -> Vec<Command> {
    let mut list = DisplayList::new();
    render(node, Context::new(&Theme::dark()), &mut list);
    list.iter().map(|i| i.command.clone()).collect()
}

const BTN: f32 = 44.0;
const SPACING_DIP: f32 = 12.0;
const PAD_DIP: f32 = 8.0;
const W: f32 = 400.0;
const H: f32 = 200.0;
const BUTTONS: [&str; 3] = ["menu", "pen", "eraser"];

/// 一个工具栏按钮。外观与 `examples/demo.rs` 的同名函数一致。
fn button(name: &'static str, hovered: Option<&ElementKey>, pressed: Option<&ElementKey>) -> Element {
    let key = ElementKey::from(name);
    let is_hover = hovered == Some(&key);
    let is_pressed = pressed == Some(&key);
    Element::new(NodeTag::Control)
        .with_key(name)
        .with(&props::INTERACTIVE, PropertyValue::bool(true))
        .with(&props::WIDTH, PropertyValue::float(BTN))
        .with(&props::HEIGHT, PropertyValue::float(BTN))
        .with(&props::CORNER_RADIUS, PropertyValue::float(6.0))
        .with(
            &props::BACKGROUND,
            PropertyValue::Resource(props::control_fill_role(false, false, is_hover).into()),
        )
        .with(
            &props::BORDER_COLOR,
            PropertyValue::Resource(
                if is_pressed {
                    roles::FOCUS_RING
                } else {
                    roles::STROKE
                }
                .into(),
            ),
        )
        .with(
            &props::BORDER_WIDTH,
            PropertyValue::float(if is_pressed { 2.0 } else { 1.0 }),
        )
}

fn view(hovered: Option<&ElementKey>, pressed: Option<&ElementKey>) -> Element {
    Element::new(NodeTag::Container)
        .with_key("toolbar")
        .with(
            &props::BACKGROUND,
            PropertyValue::Resource(roles::SURFACE_ALT.into()),
        )
        .with(&props::PADDING, PropertyValue::float(PAD_DIP))
        .with(&props::SPACING, PropertyValue::float(SPACING_DIP))
        .with(&props::DIRECTION, PropertyValue::direction(Direction::Horizontal))
        .with(&props::CROSS_ALIGN, PropertyValue::cross_align(AlignItems::Center))
        .with_children(
            BUTTONS
                .iter()
                .map(|n| button(n, hovered, pressed))
                .collect::<Vec<_>>(),
        )
}

/// 驱动一整条链路的测试夹具。
struct Harness {
    node: Node,
    router: InputRouter,
    theme: Theme,
}

impl Harness {
    fn new() -> Self {
        let theme = Theme::dark();
        let ctx = Context::new(&theme);
        let mut node = Node::from_element(&view(None, None));
        layout(&mut node, Rect::new(0.0, 0.0, W, H), ctx);
        clear_dirty(&mut node);
        Self {
            node,
            router: InputRouter::new(),
            theme,
        }
    }

    /// 走一个指针事件。
    fn pointer(
        &mut self,
        phase: PointerPhase,
        at: (f32, f32),
    ) -> (Vec<RoutedEvent>, Option<ElementKey>, Option<ElementKey>) {
        let mut out = RoutedEvents::new();
        self.router.on_pointer(
            &self.node,
            PointerEvent {
                id: 1,
                x: at.0,
                y: at.1,
                phase,
                source: PointerSource::Mouse,
            },
            Context::new(&self.theme),
            &mut out,
        );
        (
            out.iter().cloned().collect(),
            self.router.hovered().cloned(),
            self.router.pressed().cloned(),
        )
    }

    /// 走一帧：把当前 hover/press 灌进视图 → diff →（必要时）重排 → 产出命令。
    fn frame(&mut self, hovered: Option<ElementKey>, pressed: Option<ElementKey>) -> Vec<Command> {
        let ctx = Context::new(&self.theme);
        let stats = diff(&mut self.node, &view(hovered.as_ref(), pressed.as_ref()));
        if stats.needs_layout() {
            layout(&mut self.node, Rect::new(0.0, 0.0, W, H), ctx);
        }
        let mut list = DisplayList::new();
        render(&self.node, ctx, &mut list);
        clear_dirty(&mut self.node);
        list.iter().map(|i| i.command.clone()).collect()
    }
}

/// 第 `i` 个按钮的中心点（DIP）。
///
/// 按布局算法算出来，不从结果反推：
///
/// - x：`padding(8) + i * (按钮宽 44 + spacing 12) + 宽/2`（主轴 Start 对齐）；
/// - y：**不是** 30。工具栏是根节点，`layout` 会让它铺满整个 `bounds`，所以内部
///   高度是 200-16=184，而 `cross_align: Center` 把 44 高的按钮放在正中 → y=100。
///   按"工具栏贴合内容"来算会点在按钮上方的空白里，于是 `target()` 返回 `None`。
fn button_center(i: usize) -> (f32, f32) {
    (
        PAD_DIP + i as f32 * (BTN + SPACING_DIP) + BTN / 2.0,
        H / 2.0,
    )
}

/// 一个远离所有按钮、但仍在根矩形内的空白点。
///
/// 必须在根矩形内 —— 否则命中测试会在根节点就返回 false，连"清空 hover"这条路
/// 都测不到（root 不命中与 root 命中但无目标，走的是两条不同的分支）。
fn blank() -> (f32, f32) {
    (W - 12.0, PAD_DIP + 4.0)
}

fn name_of(k: Option<&ElementKey>) -> Option<&str> {
    k.and_then(|k| k.as_name())
}

#[test]
fn no_input_means_diff_is_a_noop() {
    let mut h = Harness::new();
    let stats = diff(&mut h.node, &view(None, None));
    assert!(
        stats.is_noop(),
        "元素树不变时 diff 必须是 no-op —— 这是「不动的时候真的不动」：{stats:?}"
    );
}

#[test]
fn hovering_a_button_changes_the_display_list() {
    let mut h = Harness::new();
    let before = h.frame(None, None);

    let (evs, hovered, pressed) = h.pointer(PointerPhase::Move, button_center(0));
    assert!(
        evs.is_empty(),
        "纯移动不产生路由事件 —— hover 用 revision 表达，不是事件流"
    );
    assert_eq!(name_of(hovered.as_ref()), Some("menu"));

    let after = h.frame(hovered, pressed);
    assert_ne!(
        before, after,
        "hover 必须改变 DisplayList —— 否则用户看不出鼠标在哪"
    );
}

#[test]
fn moving_off_all_controls_clears_hover() {
    let mut h = Harness::new();
    h.pointer(PointerPhase::Move, button_center(0));
    let (_, hovered, _) = h.pointer(PointerPhase::Move, blank());
    assert!(
        hovered.is_none(),
        "移到空白处必须清掉 hover，否则高亮粘在旧按钮上"
    );
}

#[test]
fn moving_between_buttons_hovers_exactly_one() {
    let mut h = Harness::new();
    h.pointer(PointerPhase::Move, button_center(0));
    let (_, hovered, _) = h.pointer(PointerPhase::Move, button_center(1));
    assert_eq!(
        name_of(hovered.as_ref()),
        Some("pen"),
        "在按钮间移动时只有当前按钮应高亮"
    );
}

#[test]
fn pressing_captures_the_node_and_keeps_it_after_dragging_off() {
    let mut h = Harness::new();
    let (evs, _, pressed) = h.pointer(PointerPhase::Down, button_center(0));
    assert_eq!(evs.len(), 1, "按下应产生一条路由事件");
    assert!(matches!(&evs[0], RoutedEvent::Press(k) if k.as_name() == Some("menu")));
    assert_eq!(name_of(pressed.as_ref()), Some("menu"));

    // 拖出控件外 —— capture 语义要求 pressed 不变
    let (evs, hovered, pressed) = h.pointer(PointerPhase::Move, blank());
    assert!(evs.is_empty());
    assert!(hovered.is_none(), "拖动时 hover 不该乱跳");
    assert_eq!(
        name_of(pressed.as_ref()),
        Some("menu"),
        "capture 语义：移出控件后 pressed 仍归原节点，否则松手会误触发别的按钮"
    );
}

#[test]
fn press_visual_is_independent_of_hover() {
    let mut h = Harness::new();
    h.pointer(PointerPhase::Down, button_center(0));
    let (_, hovered, pressed) = h.pointer(PointerPhase::Move, blank());
    assert!(hovered.is_none(), "先确认 hover 已清");

    let on_button = h.frame(Some(ElementKey::from("menu")), pressed.clone());
    let off_button = h.frame(None, pressed.clone());
    assert_ne!(
        on_button, off_button,
        "pressed 必须独立于 hover 影响外观，否则按下后拖出控件时按下态消失"
    );
}

#[test]
fn release_on_same_node_produces_click() {
    let mut h = Harness::new();
    h.pointer(PointerPhase::Down, button_center(0));
    let (evs, _, pressed) = h.pointer(PointerPhase::Up, button_center(0));
    assert!(pressed.is_none(), "抬起后 capture 必须清除");

    assert_eq!(evs.len(), 2, "同节点按下+抬起应产生 Release + Click");
    match &evs[0] {
        RoutedEvent::Release { pressed, released } => {
            assert_eq!(pressed.as_name(), Some("menu"));
            assert_eq!(released.as_ref().and_then(|k| k.as_name()), Some("menu"));
        }
        other => panic!("第一条应是 Release，实得 {other:?}"),
    }
    assert!(
        matches!(&evs[1], RoutedEvent::Click(k) if k.as_name() == Some("menu")),
        "第二条应是 Click(menu)，实得 {:?}",
        evs[1]
    );
}

#[test]
fn dragging_off_before_release_does_not_click() {
    let mut h = Harness::new();
    h.pointer(PointerPhase::Down, button_center(0));
    h.pointer(PointerPhase::Move, blank());
    let (evs, _, _) = h.pointer(PointerPhase::Up, blank());
    assert!(
        !evs.iter().any(|e| matches!(e, RoutedEvent::Click(_))),
        "拖出后松手不得 click —— 那会误触发用户没点的东西：{evs:?}"
    );
}

#[test]
fn theme_switch_changes_colors_without_touching_the_tree() {
    let mut h = Harness::new();
    let dark = h.frame(None, None);

    // 换主题：树完全不动，只是换一个 &Theme 引用
    h.theme = Theme::light();
    let light = h.frame(None, None);

    assert_ne!(
        dark, light,
        "同一棵元素树在 Light/Dark 下必须产出不同的 DisplayList"
    );

    // 关键：换主题不产生任何布局需求 —— 颜色不影响几何
    let stats = diff(&mut h.node, &view(None, None));
    assert!(
        stats.is_noop(),
        "切主题不应让元素树变样：{stats:?}"
    );
}

/// 一个图像节点，`src` 可选。尺寸固定，好断言目标矩形没被 uv 影响。
fn image_node(src: Option<UvRect>) -> Element {
    let mut e = Element::new(NodeTag::Image)
        .with(&props::IMAGE_TEXTURE, PropertyValue::static_str("icon.png"))
        .with(&props::WIDTH, PropertyValue::float(32.0))
        .with(&props::HEIGHT, PropertyValue::float(32.0));
    if let Some(uv) = src {
        e = e.with(&props::IMAGE_SRC, PropertyValue::uv_rect(uv));
    }
    e
}

#[test]
fn cropping_an_image_changes_only_the_command_not_the_geometry() {
    // 端到端走一遍：属性 → diff → 布局 → 命令。uv 是纹理坐标，与节点矩形无关，
    // 所以布局必须命中缓存、矩形必须逐字段相同。
    let mut root = Element::new(NodeTag::Container)
        .with(&props::DIRECTION, PropertyValue::direction(Direction::Horizontal))
        .with_children([image_node(None)]);

    let theme = Theme::dark();
    let ctx = Context::new(&theme);
    let mut node = Node::from_element(&root);
    let bounds = Rect::new(0.0, 0.0, 200.0, 100.0);
    layout(&mut node, bounds, ctx);
    clear_dirty(&mut node);
    let before = commands(&node);
    let rect_before = node.children()[0].rect();

    // 换成图集右半边。
    root = Element::new(NodeTag::Container)
        .with(&props::DIRECTION, PropertyValue::direction(Direction::Horizontal))
        .with_children([image_node(Some(UvRect::new(0.5, 0.0, 0.5, 1.0)))]);
    let stats = diff(&mut node, &root);
    assert!(
        stats.dirtied > 0,
        "换 uv 应被检出为重绘，实得 {stats:?}"
    );
    assert!(
        !stats.needs_layout(),
        "换 uv 不应要求重排，否则改一个纹理坐标会让整表重排"
    );

    layout(&mut node, bounds, ctx);
    let after = commands(&node);

    // 命令变了（uv 不同）。
    assert_ne!(before, after, "换 uv 必须改变 DisplayList，否则裁剪没生效");
    // 但几何没变。
    assert_eq!(
        node.children()[0].rect(),
        rect_before,
        "uv 是纹理坐标，不该影响节点在屏幕上的矩形"
    );

    let src_of = |cmds: &[Command]| -> Vec<Option<UvRect>> {
        cmds
            .iter()
            .filter_map(|c| match c {
                Command::Image { src, .. } => Some(*src),
                _ => None,
            })
            .collect()
    };
    assert_eq!(src_of(&before), vec![None], "默认不裁剪");
    assert_eq!(
        src_of(&after),
        vec![Some(UvRect::new(0.5, 0.0, 0.5, 1.0))],
        "裁剪区域应原样到达命令"
    );
}

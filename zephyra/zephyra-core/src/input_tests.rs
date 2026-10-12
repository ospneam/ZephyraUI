//! 命中测试 + 输入路由的行为断言。
//!
//! 这批测试补的是骨架阶段留下的洞：原 `hit.rs` 从不把任何节点标成 `interactive`
//! （`HitNode.interactive` 恒为 `false`），也就是说命中测试永远答不出"点到了哪个
//! 控件"。更根本的是它**没人调用**——而 hover/click 正是保留式 UI 的地基。
//!
//! 每条断言都对应一个"曾经会静默出错"的具体故障。

use crate::core::{Color, Point, Rect};
use crate::hit::{hit_test, HitResult, HitTestRequest, MAX_DEPTH};
use crate::input::pointer::{PointerEvent, PointerPhase, PointerSource};
use crate::input::router::{InputRouter, RoutedEvent, RoutedEvents};
use crate::layout::Direction;
use crate::property::PropertyValue;
use crate::props;
use crate::context::Context;
use crate::element::Element;
use crate::id::NodeTag;
use crate::layout::layout;
use crate::node::Node;

// ============================== 辅助 ==============================

fn ctx() -> Context<'static> {
    crate::context::dark_context()
}

/// 一个可交互的矩形节点。
fn hitbox(id: &'static str, w: f32, h: f32) -> Element {
    Element::new(NodeTag::Control)
        .with_key(id)
        .with(&props::INTERACTIVE, PropertyValue::bool(true))
        .with(&props::WIDTH, PropertyValue::float(w))
        .with(&props::HEIGHT, PropertyValue::float(h))
        .with(&props::BACKGROUND, PropertyValue::color(Color::rgb(0.2, 0.2, 0.2)))
}

/// 一行两个按钮：a 在左（0..40），b 在右（40..80）。
fn two_buttons() -> Element {
    Element::new(NodeTag::Container)
        .with_key("root")
        .with(&props::DIRECTION, PropertyValue::direction(Direction::Horizontal))
        .with(&props::SPACING, PropertyValue::float(0.0))
        .with(&props::PADDING, PropertyValue::float(0.0))
        .with_children([hitbox("a", 40.0, 40.0), hitbox("b", 40.0, 40.0)])
}

fn tree(element: &Element) -> Node {
    let mut node = Node::from_element(element);
    layout(&mut node, Rect::new(0.0, 0.0, 200.0, 200.0), ctx());
    node
}

fn ev(id: u32, x: f32, y: f32, phase: PointerPhase) -> PointerEvent {
    PointerEvent {
        id,
        x,
        y,
        phase,
        source: PointerSource::Mouse,
    }
}

fn route(router: &mut InputRouter, node: &Node, e: PointerEvent) -> Vec<RoutedEvent> {
    let mut out = RoutedEvents::new();
    router.on_pointer(node, e, ctx(), &mut out);
    out.iter().cloned().collect()
}

fn hit_at(node: &Node, x: f32, y: f32) -> HitResult {
    let mut out = HitResult::default();
    hit_test(
        node,
        HitTestRequest {
            point: Point::new(x, y),
            ignore_above: None,
        },
        ctx(),
        &mut out,
    );
    out
}

// ============================== 命中测试 ==============================

#[test]
fn an_interactive_node_is_actually_reported_as_interactive() {
    // 这条钉的是那个恒为 false 的 bug：`interactive` 永远假 ⇒ target() 永远 None
    // ⇒ 所有控件都收不到点击，而且没有任何报错。
    let node = tree(&two_buttons());
    let result = hit_at(&node, 10.0, 10.0);
    let target = result
        .target()
        .expect("落在可交互按钮上时必须有 target");
    assert_eq!(target.id, crate::id::ElementKey::Name("a".into()));
    assert!(target.interactive);
}

#[test]
fn pressing_and_releasing_both_bump_the_revision() {
    // 宿主靠 `revision` 变化决定「这一帧要不要重绘」，而按下态与 click 之后
    // 的状态变化**都是画出来的**。漏掉 `revision++` 时症状极其安静：
    //
    // - Down 不 bump ⇒ 按下时没有按下样式，鼠标一动才出现；
    // - Up 不 bump ⇒ **第一次点击「没反应」**，把鼠标移开再移回来才生效。
    //
    // 第二条最难查：画面没有任何异常（该亮的亮、该画的画），只是「少了一帧」。
    // 判据刻意不让 hover 同时变化（鼠标不动）—— 那正是点按钮时的常态。
    let node = tree(&two_buttons());
    let mut router = InputRouter::new();

    // 先悬停上去，让 hover 稳定下来。
    route(&mut router, &node, ev(0, 10.0, 10.0, PointerPhase::Move));
    let before_down = router.revision();

    route(&mut router, &node, ev(0, 10.0, 10.0, PointerPhase::Down));
    assert_ne!(
        router.revision(),
        before_down,
        "按下必须 bump revision（否则按下态没有样式，且宿主不重绘）"
    );

    let before_up = router.revision();
    route(&mut router, &node, ev(0, 10.0, 10.0, PointerPhase::Up));
    assert_ne!(
        router.revision(),
        before_up,
        "抬起必须 bump revision —— click 之后应用改了状态，不 bump 就整帧不重绘"
    );
}

#[test]
fn a_container_is_on_the_path_but_is_not_the_target() {
    // 事件冒泡需要容器的矩形，但处理目标必须是按钮本身。
    let node = tree(&two_buttons());
    let result = hit_at(&node, 10.0, 10.0);
    assert!(result.path.iter().any(|n| n.id == crate::id::ElementKey::Name("root".into())));
    assert_eq!(
        result.target().map(|n| &n.id),
        Some(&crate::id::ElementKey::Name("a".into()))
    );
}

#[test]
fn sibling_rects_never_overlap_under_the_current_layout() {
    // 这条是给**将来**埋的雷：目前 `Stack` 布局按顺序摆放兄弟，所以兄弟矩形永不相交，
    // 命中测试不存在"多个候选"的歧义。
    //
    // 一旦加入绝对定位或浮层，兄弟就会重叠，那时 `descend` 里"反向遍历 = 后加入的
    // 在上层"这条约定才开始承重。这条断言会在那时失败——那正是它存在的意义：
    // 提醒实现者"重叠下的 z 序现在有人依赖了"。
    let node = tree(&two_buttons());
    let rects: Vec<Rect> = node.children().iter().map(|c| c.rect()).collect();
    assert!(
        rects[0].right() <= rects[1].x || rects[1].right() <= rects[0].x,
        "Stack 布局下兄弟不应重叠，实得 {rects:?}"
    );
}

#[test]
fn a_point_in_the_root_but_on_no_control_has_no_target() {
    // 复用 `out` 的关键风险：上一次命中的路径残留下来，被下一次误读为命中了控件。
    //
    // 注意 (150,150) 仍在根矩形内，所以 `hit_any()` 为真——"命中了容器"与
    // "命中了控件"是两回事，混为一谈就会把点击吞掉。
    let node = tree(&two_buttons());
    let mut out = HitResult::default();
    assert!(hit_test(
        &node,
        HitTestRequest { point: Point::new(10.0, 10.0), ignore_above: None },
        ctx(),
        &mut out
    ));
    assert!(out.target().is_some());

    assert!(hit_test(
        &node,
        HitTestRequest { point: Point::new(150.0, 150.0), ignore_above: None },
        ctx(),
        &mut out
    ));
    assert!(
        out.target().is_none(),
        "落在容器空白处不应命中任何控件，实得 {:?}",
        out.path
    );
}

#[test]
fn a_point_outside_the_root_leaves_the_buffer_empty() {
    let node = tree(&two_buttons());
    let mut out = HitResult::default();
    assert!(!hit_test(
        &node,
        HitTestRequest { point: Point::new(500.0, 500.0), ignore_above: None },
        ctx(),
        &mut out
    ));
    assert!(
        out.path.is_empty(),
        "完全未命中时路径必须被清空（供跨帧复用），实得 {:?}",
        out.path
    );
}

#[test]
fn collapsed_nodes_are_not_hittable() {
    // 折叠 = 不占位不绘制。若仍可命中，会出现"看不见但点得到"的幽灵控件。
    let e = two_buttons();
    let mut node = tree(&e);
    node.children_mut()[1].set_prop(&props::COLLAPSED, PropertyValue::bool(true));
    let result = hit_at(&node, 60.0, 10.0);
    assert!(
        result.target().is_none(),
        "折叠的按钮不该被命中"
    );
}

#[test]
fn a_deep_tree_truncates_but_still_answers_correctly() {
    // 超过 MAX_DEPTH 时路径被截断，但"命中了谁"必须照常给出——
    // 截断只该影响冒泡链，不该影响答案。
    let mut element = hitbox("deep", 100.0, 100.0);
    for _ in 0..(MAX_DEPTH + 5) {
        element = Element::new(NodeTag::Container)
            .with_key("l")
            .with(&props::DIRECTION, PropertyValue::direction(Direction::Vertical))
            .with(&props::PADDING, PropertyValue::float(0.0))
            .with_children([element]);
    }
    let mut node = Node::from_element(&element);
    layout(&mut node, Rect::new(0.0, 0.0, 200.0, 200.0), ctx());
    let result = hit_at(&node, 50.0, 50.0);
    assert!(result.truncated, "超深应标记为截断");
    assert!(result.hit_any());
}

// ============================== 路由：悬停 ==============================

#[test]
fn moving_onto_a_button_changes_the_revision() {
    let node = tree(&two_buttons());
    let mut router = InputRouter::new();
    let before = router.revision();

    route(&mut router, &node, ev(1, 10.0, 10.0, PointerPhase::Move));
    assert_eq!(
        router.hovered().and_then(|k| k.as_name()),
        Some("a"),
        "指针应悬停在 a 上"
    );
    assert_ne!(router.revision(), before, "悬停变化必须递增 revision");
}

#[test]
fn moving_between_buttons_keeps_the_same_revision_for_one_event() {
    // 一次 Move 内部完成了 a→b 的切换；revision 只该 +1（一个状态变更），
    // 而事件流里不该出现两条 hover 事件——宿主比较 revision 就够了。
    let node = tree(&two_buttons());
    let mut router = InputRouter::new();
    route(&mut router, &node, ev(1, 10.0, 10.0, PointerPhase::Move));
    let before = router.revision();
    route(&mut router, &node, ev(1, 60.0, 10.0, PointerPhase::Move));
    assert_eq!(router.hovered().and_then(|k| k.as_name()), Some("b"));
    assert_eq!(router.revision(), before + 1, "一次移动只应产生一次状态变更");
}

#[test]
fn hovering_blank_space_clears_the_hover() {
    // 指针移出所有按钮后，高亮必须消失——否则按钮会"粘"在亮起状态。
    let node = tree(&two_buttons());
    let mut router = InputRouter::new();
    route(&mut router, &node, ev(1, 10.0, 10.0, PointerPhase::Move));
    route(&mut router, &node, ev(1, 150.0, 150.0, PointerPhase::Move));
    assert!(router.hovered().is_none());
}

#[test]
fn the_same_position_does_not_bump_the_revision() {
    // 静止时指针可能仍送来 Move。若每次都 +1，宿主就会永远重绘。
    let node = tree(&two_buttons());
    let mut router = InputRouter::new();
    route(&mut router, &node, ev(1, 10.0, 10.0, PointerPhase::Move));
    let before = router.revision();
    for _ in 0..5 {
        route(&mut router, &node, ev(1, 10.0, 10.0, PointerPhase::Move));
    }
    assert_eq!(
        router.revision(),
        before,
        "位置未变就不该要求重绘"
    );
}

// ============================== 路由：按下/抬起/点击 ==============================

#[test]
fn press_and_release_on_the_same_button_produce_a_click() {
    let node = tree(&two_buttons());
    let mut router = InputRouter::new();
    let mut events = route(&mut router, &node, ev(1, 10.0, 10.0, PointerPhase::Down));
    assert!(matches!(events.first(), Some(RoutedEvent::Press(_))));
    events = route(&mut router, &node, ev(1, 10.0, 10.0, PointerPhase::Up));
    assert!(
        events.iter().any(|e| matches!(e, RoutedEvent::Click(k) if k.as_name() == Some("a"))),
        "同点按下抬起应产生 click，实得 {events:?}"
    );
}

#[test]
fn dragging_off_before_release_does_not_click() {
    // 这条是最容易出真 bug 的一条：按下 a、拖出去、在空白处松手。
    // 若按"抬起时命中谁"来判定，就会误触发——而用户根本没点它。
    let node = tree(&two_buttons());
    let mut router = InputRouter::new();
    route(&mut router, &node, ev(1, 10.0, 10.0, PointerPhase::Down));
    let events = route(&mut router, &node, ev(1, 150.0, 150.0, PointerPhase::Up));
    assert!(
        !events.iter().any(|e| matches!(e, RoutedEvent::Click(_))),
        "拖出后松手不得触发 click，实得 {events:?}"
    );
}

#[test]
fn dragging_from_a_onto_b_does_not_click_b() {
    // 捕获语义的核心断言：按下时是谁，就归谁。
    let node = tree(&two_buttons());
    let mut router = InputRouter::new();
    route(&mut router, &node, ev(1, 10.0, 10.0, PointerPhase::Down));
    let events = route(&mut router, &node, ev(1, 60.0, 10.0, PointerPhase::Up));
    assert!(
        !events.iter().any(|e| matches!(e, RoutedEvent::Click(_))),
        "a 按下、b 抬起不得触发 click，实得 {events:?}"
    );
    match events.iter().find(|e| matches!(e, RoutedEvent::Release { .. })) {
        Some(RoutedEvent::Release { pressed, released }) => {
            assert_eq!(pressed.as_name(), Some("a"), "press 应归按下者");
            assert_eq!(
                released.as_ref().and_then(|k| k.as_name()),
                Some("b"),
                "released 应如实报告抬起时命中的节点"
            );
        }
        other => panic!("应有 Release 事件，实得 {other:?}"),
    }
}

#[test]
fn hover_is_frozen_while_dragging() {
    // 拖动过程中悬停若跟随指针，按钮会在手指经过时逐个闪亮。
    let node = tree(&two_buttons());
    let mut router = InputRouter::new();
    route(&mut router, &node, ev(1, 10.0, 10.0, PointerPhase::Move));
    let hovered = router.hovered().cloned();
    route(&mut router, &node, ev(1, 10.0, 10.0, PointerPhase::Down));
    route(&mut router, &node, ev(1, 60.0, 10.0, PointerPhase::Move));
    assert_eq!(
        router.hovered(),
        hovered.as_ref(),
        "按下期间悬停不应跟随"
    );
    // 松手后悬停立刻追上当前位置
    route(&mut router, &node, ev(1, 60.0, 10.0, PointerPhase::Up));
    assert_eq!(router.hovered().and_then(|k| k.as_name()), Some("b"));
}

#[test]
fn pressing_on_blank_space_presses_nothing() {
    let node = tree(&two_buttons());
    let mut router = InputRouter::new();
    let events = route(&mut router, &node, ev(1, 150.0, 150.0, PointerPhase::Down));
    assert!(events.is_empty(), "空白处按下不应产出事件");
    assert!(router.pressed().is_none());
    let events = route(&mut router, &node, ev(1, 150.0, 150.0, PointerPhase::Up));
    assert!(!events.iter().any(|e| matches!(e, RoutedEvent::Click(_))));
}

#[test]
fn reset_clears_the_press_capture() {
    // 失焦复位必须同时清掉按下状态。残留的后果很具体：失焦后第一次点击会被判为
    // "抬起"，从而误触发一个用户以为已经取消了的操作。
    let node = tree(&two_buttons());
    let mut router = InputRouter::new();
    route(&mut router, &node, ev(1, 10.0, 10.0, PointerPhase::Down));
    assert!(router.pressed().is_some());
    router.reset();
    assert!(router.pressed().is_none());
    let events = route(&mut router, &node, ev(1, 10.0, 10.0, PointerPhase::Up));
    assert!(
        events.is_empty(),
        "复位后第一次抬起不得被当作 click，实得 {events:?}"
    );
}

// ============================== 手势 ==============================

#[test]
fn a_two_finger_pinch_is_reported_alongside_hover_changes() {
    // 手势与 hover 是正交通道：捏合时界面高亮也该更新到当前手指位置。
    let node = tree(&two_buttons());
    let mut router = InputRouter::new();
    route(
        &mut router,
        &node,
        PointerEvent { id: 1, x: 10.0, y: 10.0, phase: PointerPhase::Down, source: PointerSource::Touch },
    );
    let events = route(
        &mut router,
        &node,
        PointerEvent { id: 2, x: 60.0, y: 10.0, phase: PointerPhase::Down, source: PointerSource::Touch },
    );
    assert!(
        events.iter().any(|e| matches!(e, RoutedEvent::Gesture(_))),
        "第二指按下应激活捏合并产出手势，实得 {events:?}"
    );
    assert_eq!(router.pointer_count(), 2);
}
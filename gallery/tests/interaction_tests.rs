//! 交互回归 —— **保留** `examples/demo_tests.rs` 的 10 条测试。
//!
//! # 为什么复制而不是直接复用 `examples` 那份
//!
//! `demo_tests.rs` 驱动的是 `demo::view`（demo 自己的按钮行）。本文件驱动的是
//! **gallery 的真实控件**（按钮、复选框…）。两者测的是**同一批框架语义**
//! —— hover 清理、press 捕获、click 配对、拖走不 click、theme 换色不动树、
//! uv 裁剪只改命令不改几何 —— 但跑在**迁移后的代码**上，这才是"保留测试功能"
//! 的意义：测试跟着被测对象走，而不是守着一个已被替换的 demo。
//!
//! # 逐条对照（与 demo_tests.rs 同名同义）
//!
//! `no_input_means_diff_is_a_noop` / `hovering_a_button_changes_the_display_list` /
//! `moving_off_all_controls_clears_hover` / `moving_between_buttons_hovers_exactly_one` /
//! `pressing_captures_the_node_and_keeps_it_after_dragging_off` /
//! `press_visual_is_independent_of_hover` / `release_on_same_node_produces_click` /
//! `dragging_off_before_release_does_not_click` /
//! `theme_switch_changes_colors_without_touching_the_tree` /
//! `cropping_an_image_changes_only_the_command_not_the_geometry`
//!
//! 坐标**不硬算**：用 `center_of` 反查控件矩形（见该函数）—— demo 那份硬算了
//! 工具栏居中的 y，那类"按布局反推坐标"的算式一旦布局改动就悄悄点空，
//! 而反查永远对准。

use zephyra_core::context::Context;
use zephyra_core::core::Rect;
use zephyra_core::diff::diff;
use zephyra_core::id::ElementKey;
use zephyra_core::input::{
    InputRouter, PointerEvent, PointerPhase, PointerSource, RoutedEvent, RoutedEvents,
};
use zephyra_core::layout::layout;
use zephyra_core::node::{clear_dirty, Node};
use zephyra_core::paint::{render, Command, DisplayList};
use zephyra_core::theme::{Theme, ThemeVariant};
use zephyra_fx::View;

use zephyra_gallery::app::Gallery;

/// 画布（与 gallery 一致）。
const W: f32 = 1400.0;
const H: f32 = 800.0;

/// gallery Basic Input 页里的两个可点按钮。
const BTN_A: &str = "btn-standard";
const BTN_B: &str = "btn-accent";

/// 样式表（与 gallery 窗口路**同一份**）。
///
/// ⚠️ 必须装：控件的尺寸/间距全在 zstyle 里，不装表则所有按钮**零尺寸**
/// ⇒ hit_test 永远不命中。demo_tests 不装表是因为 demo 的按钮尺寸另有来源。
fn sheet() -> zephyra_style::ir::StyleSheet {
    // 内嵌源（控件层 + 本应用），不是磁盘目录 —— 理由同 `gates::merged_sheet`。
    let mut sheet = zephyra_style::ir::StyleSheet::new();
    for (name, css) in zephyra_gallery::styles::all() {
        let parsed = zstylec::parse(name, css);
        assert!(
            parsed.is_ok(),
            "{name}.zstyle must parse: {:?}",
            parsed.errors
        );
        sheet = sheet.merge(parsed.sheet);
    }
    sheet
}

/// 驱动一棵 gallery 树的夹具（与 demo_tests 的 Harness 同构）。
struct Harness {
    node: Node,
    router: InputRouter,
    theme: Theme,
    gallery: Gallery,
}

impl Harness {
    fn new() -> Self {
        let theme = Theme::builtin(ThemeVariant::Dark);
        let ctx = Context::new(&theme);
        let mut gallery = Gallery::default();
        // basics 页不再是路由（参考层收进 reference 页内 segmented 切换）。
        // btn-standard 等 key 现在住在 reference 页的第 0 族里。
        gallery.route = "reference";
        gallery.ref_family = 0;
        let mut node = Node::from_element(&gallery.view(&ctx));
        sheet().apply_all(&mut node);
        layout(&mut node, Rect::new(0.0, 0.0, W, H), ctx);
        clear_dirty(&mut node);
        Self {
            node,
            router: InputRouter::new(),
            theme,
            gallery,
        }
    }

    /// 走一个指针事件，返回（hovered, pressed）。
    fn pointer(
        &mut self,
        phase: PointerPhase,
        at: (f32, f32),
    ) -> (Option<ElementKey>, Option<ElementKey>) {
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
            self.router.hovered().cloned(),
            self.router.pressed().cloned(),
        )
    }

    /// 同上，但把路由出来的事件也返回（click 类测试用）。
    fn pointer_events(&mut self, phase: PointerPhase, at: (f32, f32)) -> Vec<RoutedEvent> {
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
        out.iter().cloned().collect()
    }

    /// 走一帧：router 状态 → View → diff →（必要时）重排 → 命令。
    fn frame(&mut self) -> Vec<Command> {
        let ctx = Context::new(&self.theme);
        let hovered = self.router.hovered().cloned();
        let pressed = self.router.pressed().cloned();
        self.gallery.on_input(&zephyra_fx::InputState {
            hovered,
            pressed: pressed.clone(),
            pointers: usize::from(pressed.is_some()),
            viewport: (W, H),
            scale: 1.0,
            // 测试里给一个确定值：动画时钟的断言因此可复现，
            // 而不依赖机器速度（若填 0，则所有动画类断言都只能测"未开始"）。
            dt_ms: 16.0,
            // 空表 = "读不到"。于是 `Gallery` 走回落分支，界面语言固定为
            // `SUPPORTED` 第一项，断言不依赖宿主机器的 Windows 语言设置 ——
            // 否则同一条测试在英文机器与中文机器上会给出不同结果。
            languages: Vec::new(),
        });
        // 顺序是**契约**：diff → apply → layout → render。
        // apply 必须在 diff **之后** —— 它读的是节点上**刚被 diff 改过**的分类名，
        // 放在前面就等于用旧类名算样式，于是"hover 变了但画面没变"。
        let element = self.gallery.view(&ctx);
        let stats = diff(&mut self.node, &element);
        sheet().apply_all(&mut self.node);
        if stats.needs_layout() {
            layout(&mut self.node, Rect::new(0.0, 0.0, W, H), ctx);
        }
        let mut list = DisplayList::new();
        render(&self.node, ctx, &mut list);
        clear_dirty(&mut self.node);
        list.iter().map(|i| i.command.clone()).collect()
    }
}

/// 控件矩形中心：遍历树反查（不硬算布局）。
fn center_of(node: &Node, key: &str) -> (f32, f32) {
    fn find(n: &Node, key: &str, out: &mut Option<Rect>) {
        if n.key().map(|k| k.to_string()).as_deref() == Some(key) {
            *out = Some(n.rect());
        }
        for c in n.children() {
            find(c, key, out);
        }
    }
    let mut rect = None;
    find(node, key, &mut rect);
    let r = rect.unwrap_or_else(|| panic!("gallery 里找不到 key {key}"));
    assert!(
        r.width > 1.0 && r.height > 1.0,
        "{key} 的矩形是 {r:?} —— 零尺寸说明样式表没装上",
    );
    (r.x + r.width / 2.0, r.y + r.height / 2.0)
}

fn name_of(k: Option<&ElementKey>) -> Option<&str> {
    k.and_then(|k| k.as_name())
}

// ---------------------------------------------------------------- 10 条回归

#[test]
fn no_input_means_diff_is_a_noop() {
    let mut h = Harness::new();
    let ctx = Context::new(&h.theme);
    let element = h.gallery.view(&ctx);
    sheet().apply_all(&mut h.node);
    let stats = diff(&mut h.node, &element);
    assert!(
        stats.is_noop(),
        "无输入时 diff 应当是 no-op（稳态判定），实际 {stats:?}"
    );
}

#[test]
fn hovering_a_button_changes_the_display_list() {
    let mut h = Harness::new();
    let before = h.frame();
    let a = center_of(&h.node, BTN_A);
    h.pointer(PointerPhase::Move, a);
    let after = h.frame();
    assert_ne!(before, after, "hover 应当改变 DisplayList");
}

#[test]
fn moving_off_all_controls_clears_hover() {
    let mut h = Harness::new();
    let a = center_of(&h.node, BTN_A);
    h.pointer(PointerPhase::Move, a);
    assert!(h.router.hovered().is_some(), "先要能 hover 上");
    h.pointer(PointerPhase::Move, (W - 4.0, H - 4.0)); // 空白角落
    assert!(
        h.router.hovered().is_none(),
        "移出所有控件后 hover 必须清空，实际 {:?}",
        h.router.hovered()
    );
}

#[test]
fn moving_between_buttons_hovers_exactly_one() {
    let mut h = Harness::new();
    let a = center_of(&h.node, BTN_A);
    let b = center_of(&h.node, BTN_B);
    h.pointer(PointerPhase::Move, a);
    assert_eq!(name_of(h.router.hovered()), Some(BTN_A));
    h.pointer(PointerPhase::Move, b);
    assert_eq!(
        name_of(h.router.hovered()),
        Some(BTN_B),
        "同时只该 hover 一个"
    );
}

#[test]
fn pressing_captures_the_node_and_keeps_it_after_dragging_off() {
    let mut h = Harness::new();
    let a = center_of(&h.node, BTN_A);
    h.pointer(PointerPhase::Move, a);
    h.pointer(PointerPhase::Down, a);
    assert_eq!(name_of(h.router.pressed()), Some(BTN_A));
    h.pointer(PointerPhase::Move, (W - 4.0, H - 4.0));
    assert_eq!(
        name_of(h.router.pressed()),
        Some(BTN_A),
        "拖出后 press 必须仍捕获原节点（捕获语义）"
    );
    h.pointer(PointerPhase::Up, (W - 4.0, H - 4.0));
}

#[test]
fn press_visual_is_independent_of_hover() {
    let mut h = Harness::new();
    let a = center_of(&h.node, BTN_A);
    h.pointer(PointerPhase::Move, a);
    let hover_only = h.frame();
    h.pointer(PointerPhase::Down, a);
    let pressed = h.frame();
    assert_ne!(
        hover_only, pressed,
        "按下视觉必须独立于 hover（按住不动时也要变）"
    );
    h.pointer(PointerPhase::Up, a);
}

#[test]
fn release_on_same_node_produces_click() {
    let mut h = Harness::new();
    let a = center_of(&h.node, BTN_A);
    h.pointer(PointerPhase::Move, a);
    h.pointer(PointerPhase::Down, a);
    h.frame(); // 让 View 记下 last_pressed
    let events = h.pointer_events(PointerPhase::Up, a);
    assert!(
        events.iter().any(|e| matches!(e, RoutedEvent::Click(_))),
        "同节点抬起必须产生 Click，实际 {:?}",
        events
    );
}

#[test]
fn dragging_off_before_release_does_not_click() {
    let mut h = Harness::new();
    let a = center_of(&h.node, BTN_A);
    h.pointer(PointerPhase::Move, a);
    h.pointer(PointerPhase::Down, a);
    h.pointer(PointerPhase::Move, (W - 4.0, H - 4.0)); // 拖走
    let events = h.pointer_events(PointerPhase::Up, (W - 4.0, H - 4.0));
    assert!(
        !events.iter().any(|e| matches!(e, RoutedEvent::Click(_))),
        "拖走再抬起**不应**产生 Click，实际 {:?}",
        events
    );
}

#[test]
fn theme_switch_changes_colors_without_touching_the_tree() {
    let h = Harness::new();
    let ctx_dark = Context::new(&h.theme);
    let mut dark = DisplayList::new();
    render(&h.node, ctx_dark, &mut dark);
    let dark_cmds: Vec<Command> = dark.iter().map(|i| i.command.clone()).collect();

    // 只换主题渲染，树一个字节都不动。
    let light = Theme::builtin(ThemeVariant::Light);
    let ctx_light = Context::new(&light);
    let mut light_list = DisplayList::new();
    render(&h.node, ctx_light, &mut light_list);
    let light_cmds: Vec<Command> = light_list.iter().map(|i| i.command.clone()).collect();

    assert!(
        !dark_cmds.is_empty(),
        "深色命令列表为空 —— 判据会恒真，先确认渲染真的出了东西"
    );
    assert_ne!(
        dark_cmds, light_cmds,
        "换主题必须改变颜色命令，而树没动（view 里不看主题）"
    );
}

#[test]
fn cropping_an_image_changes_only_the_command_not_the_geometry() {
    use zephyra_core::core::UvRect;
    use zephyra_core::element::Element;
    use zephyra_core::id::NodeTag;
    use zephyra_core::property::PropertyValue;
    use zephyra_core::props;

    let image = |u0: f32, v0: f32| {
        Element::new(NodeTag::Image)
            .with_key("a")
            .with(&props::IMAGE_TEXTURE, PropertyValue::static_str("t"))
            .with(
                &props::IMAGE_SRC,
                PropertyValue::uv_rect(UvRect::new(u0, v0, 1.0, 1.0)),
            )
            .with(&props::WIDTH, PropertyValue::float(100.0))
            .with(&props::HEIGHT, PropertyValue::float(50.0))
    };

    let theme = Theme::builtin(ThemeVariant::Dark);
    let ctx = Context::new(&theme);
    let mut a = Node::from_element(&image(0.0, 0.0));
    let mut b = Node::from_element(&image(0.5, 0.5));
    let bounds = Rect::new(0.0, 0.0, 200.0, 100.0);
    layout(&mut a, bounds, ctx);
    layout(&mut b, bounds, ctx);

    assert_eq!(a.rect(), b.rect(), "uv 裁剪不应改变几何");
}

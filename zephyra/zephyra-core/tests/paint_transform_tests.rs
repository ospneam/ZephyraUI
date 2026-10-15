//! 容器不透明度与绘制时偏移 —— core 的绘制语义。
//!
//! # 这一组测的是什么
//!
//! 官方动效表里每个动作作用的都是**不透明度**与**位置**
//! （`docs/animation.md` §1）。本组钉住这两件事在 core 侧的语义：
//!
//! 1. **不透明度落到每一种绘制**：背景、描边、文字、图像、图标；
//! 2. **opacity_multiplies_along_parent_chain**（不是"取最近一个"）；
//! 3. **偏移落到四类几何**：Quad / Image / Path / Text；
//! 4. **偏移不影响布局盒与命中测试** —— 否则控件在动画期间点不中；
//! 5. **静止时零开销**：位移为零 ⇒ 顶点数组原样返回（同一块内存）。
//!
//! # 为什么 1、2 之前是**红**的
//!
//! `props::OPACITY` 与 zstylec 的 `opacity` 映射**早就存在**，但 `paint.rs` 只把它
//! 用在 `image_tint` 上。于是"给容器设半透明"在画面上只对图像生效，背景 / 描边 /
//! 文字全都不透明 —— 而那与"没有这个属性"看起来一模一样。
//!
//! 更隐蔽的是 2：`property::resolve(store, key, None)` 在**生产代码里从来没收到过
//! 非 `None` 的 `inherited`**（`style.rs::value` 一律传 `None`），所以 `inheritable`
//! 标记没有任何效果，`node.opacity()` 取的是本节点值。只测 1 会漏掉 2。

use std::cell::RefCell;
use std::sync::Arc;

use zephyra_core::context::Context;
use zephyra_core::core::{Point, Rect};
use zephyra_core::element::Element;
use zephyra_core::frame::FrameCache;
use zephyra_core::hit::{HitResult, HitTestRequest};
use zephyra_core::id::NodeTag;
use zephyra_core::paint::{Command, DisplayList};
use zephyra_core::property::PropertyValue;
use zephyra_core::props;
use zephyra_core::text::NullEngine;
use zephyra_core::theme::{Theme, ThemeVariant};

const W: f32 = 400.0;
const H: f32 = 300.0;

/// 文字替身。
///
/// # 为什么夹具持**具体类型**而不是 `RefCell<dyn TextRender>`
///
/// `Context::with_engine` 收 `&RefCell<dyn TextRender>`，但 `RefCell<dyn T>` 本身
/// 是 unsized 的（`RefCell` 有固定大小，装不下胖指针）⇒ 直接写那个类型编不过。
/// 于是这里返回 `RefCell<NullEngine>`，在**参数位**上做一次 unsize 转换 ——
/// 与 core 自带测试（`tests.rs::engine`）同一手法。
fn null_engine() -> RefCell<NullEngine> {
    RefCell::new(NullEngine { char_width: 8.0 })
}

/// 一个**某个测试独占**的图标集（`slot` 区分）。
///
/// # 为什么不能用 [`zephyra_core::icon::builtin`]
///
/// `builtin()` 是进程级 `OnceLock` 单例，而它的几何缓存是**可变的共享状态**。
/// cargo test 默认多线程跑，同一个二进制里任何别的用例渲染图标都会往那份缓存里
/// 插入条目 —— 而 `cache_mesh` 的 `put` 对**同键**是**覆盖**，于是先跑的那个测试
/// 手里的 `Arc` 会被后跑的换掉，`ptr_eq` 断言**单跑绿、全量跑红**，且红得毫无道理。
///
/// # 为什么 `slot` 而不是直接给一个共享的
///
/// 同一个 `OnceLock` 共享两次（给两个测试各调一次）**同样有那个问题** ——
/// 两个线程可能同时 miss、各自 `put`，后写的赢。所以每条需要它的测试一个槽。
///
/// 泄漏一次是刻意的：`Context::with_icons` 要 `&'static IconSet`，而这是
/// **整个文件一生一次**的分配（不是每帧），与 `Box::leak` 在 `element.rs`
/// 里被禁用的场景不是一回事。
fn private_icons(slot: usize) -> &'static zephyra_core::icon::IconSet {
    const SLOTS: usize = 2;
    static SETS: [std::sync::OnceLock<&'static zephyra_core::icon::IconSet>; SLOTS] =
        [std::sync::OnceLock::new(), std::sync::OnceLock::new()];
    assert!(slot < SLOTS, "private_icons 的槽位用完了（{slot}）");
    SETS[slot].get_or_init(|| Box::leak(Box::new(zephyra_core::icon::IconSet::with_builtin())))
}

/// 一个带背景与描边、固定尺寸的方块。
fn box_el(key: &'static str) -> Element {
    Element::new(NodeTag::Container)
        .with_key(key)
        .with(&props::WIDTH, PropertyValue::float(100.0))
        .with(&props::HEIGHT, PropertyValue::float(100.0))
        .with(
            &props::BACKGROUND,
            PropertyValue::color(zephyra_core::core::Color::rgba(1.0, 0.0, 0.0, 1.0)),
        )
        .with(
            &props::BORDER_COLOR,
            PropertyValue::color(zephyra_core::core::Color::rgba(0.0, 1.0, 0.0, 1.0)),
        )
        .with(&props::BORDER_WIDTH, PropertyValue::float(2.0))
}

/// 建树 → 重排 → 绘制，返回命令列表。
///
/// 走与 `gates.rs` / `interaction_tests.rs` 同一套四步（diff → apply → layout →
/// render）；这里没有样式表，所以 `apply` 那步是空操作。
fn build(element: &Element) -> (zephyra_core::node::Node, Vec<Command>) {
    let theme = Theme::builtin(ThemeVariant::Dark);
    let ctx = Context::new(&theme);
    let mut node = zephyra_core::node::Node::from_element(element);
    zephyra_core::layout::layout(&mut node, Rect::new(0.0, 0.0, W, H), ctx);
    let mut list = DisplayList::new();
    zephyra_core::paint::render(&node, ctx, &mut list);
    (node, list.iter().map(|i| i.command.clone()).collect())
}

/// 第一条 Quad 的填充色。
#[track_caller]
fn first_quad_fill(list: &[Command]) -> zephyra_core::core::Color {
    list.iter()
        .find_map(|c| match c {
            Command::Quad { fill, .. } => Some(*fill),
            _ => None,
        })
        .expect("列表里应当有 Quad")
}

/// 第一条 Quad 的描边色。
#[track_caller]
fn first_quad_stroke(list: &[Command]) -> Option<zephyra_core::core::Color> {
    list.iter().find_map(|c| match c {
        Command::Quad {
            stroke: Some((color, _)),
            ..
        } => Some(*color),
        _ => None,
    })
}

// ---------------------------------------------------------------- 1. 落到每一种绘制

/// 容器 opacity 减半 ⇒ **背景**的 alpha 也减半。
///
/// 这条在修好之前是红的：`fill` 当时直接取 `node.background(ctx)`，没乘因子。
#[test]
fn container_opacity_applies_to_background() {
    let mut inner = box_el("inner");
    inner = inner.with(&props::OPACITY, PropertyValue::float(0.5));

    let (_, list) = build(&inner);
    assert_eq!(
        first_quad_fill(&list).a,
        0.5,
        "背景 alpha 应为 0.5（原本是 1.0）"
    );
}

/// 同一条，但查**描边**。
///
/// 描边与背景是同一个 `Command::Quad` 的两个字段，只测背景会漏掉描边没乘因子。
#[test]
fn container_opacity_applies_to_stroke() {
    let mut inner = box_el("inner");
    inner = inner.with(&props::OPACITY, PropertyValue::float(0.5));

    let (_, list) = build(&inner);
    assert_eq!(
        first_quad_stroke(&list).expect("有描边").a,
        0.5,
        "描边 alpha 应为 0.5"
    );
}

/// 同一条，但查**文字**。
///
/// 文字色必须**在造命令时**乘进 `Command::Text.color`：那条命令只有一个颜色字段，
/// 没有任何"每字形不透明度"的机制，所以事后回填做不到（`paint.rs::text_command`
/// 的文档说明了这一点）。
#[test]
fn container_opacity_applies_to_text() {
    let theme = Theme::builtin(ThemeVariant::Dark);
    let engine = null_engine();
    let ctx = Context::with_engine(&theme, &engine);

    let el = Element::new(NodeTag::Text)
        .with_key("t")
        .with(&props::TEXT, PropertyValue::static_str("abc"))
        .with(
            &props::TEXT_COLOR,
            PropertyValue::color(zephyra_core::core::Color::rgba(1.0, 1.0, 1.0, 1.0)),
        )
        .with(&props::OPACITY, PropertyValue::float(0.25));

    let mut node = zephyra_core::node::Node::from_element(&el);
    zephyra_core::layout::layout(&mut node, Rect::new(0.0, 0.0, W, H), ctx);
    let mut list = DisplayList::new();
    zephyra_core::paint::render(&node, ctx, &mut list);
    let color = {
        let c = list
            .iter()
            .find_map(|i| match &i.command {
                Command::Text { color, .. } => Some(*color),
                _ => None,
            })
            .expect("应当产出 Text 命令");
        c
    };
    assert_eq!(color.a, 0.25, "文字 alpha 应为 0.25");
    // 顺带确认 RGB 没被改 —— 因子只碰 alpha。
    assert_eq!((color.r, color.g, color.b), (1.0, 1.0, 1.0));
}

// ---------------------------------------------------------------- 2. 沿父子链相乘

/// 两层各 0.5 ⇒ 子树里的方块是 **0.25**，不是 0.5。
///
/// 这条在修好之前也是红的，且**比第 1 组更隐蔽**：`inheritable` 标记存在、
/// `node.opacity()` 存在，但 `property::resolve` 没人传 `inherited`，于是取的是
/// "最近一个定义" —— 单层看着对，嵌套就错，而单层正是人会手测的那一层。
#[test]
fn opacity_multiplies_along_parent_chain() {
    let child = box_el("child").with(&props::OPACITY, PropertyValue::float(0.5));
    let root = Element::new(NodeTag::Container)
        .with_key("root")
        .with(&props::WIDTH, PropertyValue::float(200.0))
        .with(&props::HEIGHT, PropertyValue::float(200.0))
        .with(&props::OPACITY, PropertyValue::float(0.5))
        .with_children([child]);

    let (_, list) = build(&root);
    let fill = first_quad_fill(&list);
    assert_eq!(
        fill.a, 0.25,
        "两层各 0.5 ⇒ 0.25；得到 0.5 说明取的是「最近一个定义」而不是乘积"
    );
}

/// 祖先的半透明要影响**后代**，而后代自己的背景不变。
///
/// 这条与上一条方向相反：上一条盯"因子怎么算"，这条盯"因子作用到谁"。
/// 少一条就会出现"容器半透明但里面的卡片不透明" —— 而那正是半透明的用处所在
/// （毛玻璃 / 压暗遮罩）。
#[test]
fn ancestor_opacity_affects_descendant_draw() {
    let child = box_el("child").with(
        &props::BACKGROUND,
        PropertyValue::color(zephyra_core::core::Color::rgba(1.0, 1.0, 1.0, 1.0)),
    );
    let root = Element::new(NodeTag::Container)
        .with_key("root")
        .with(&props::WIDTH, PropertyValue::float(200.0))
        .with(&props::HEIGHT, PropertyValue::float(200.0))
        // 祖先自己**没有背景**，所以唯一那条 Quad 就是子节点的。
        .with(&props::OPACITY, PropertyValue::float(0.5))
        .with_children([child]);

    let (_, list) = build(&root);
    assert_eq!(first_quad_fill(&list).a, 0.5, "后代的白色背景应被祖先折半");
}

/// opacity 为 0 ⇒ **完全不产出** Quad（而不是产出 alpha=0 的）。
///
/// 这条是上一组的对偶：alpha=0 的 Quad 仍要进 `DisplayList`、仍要占一次 draw call。
/// 动画结束时省掉它是实打实的，而判据必须是"列表里没有"而不是"alpha 等于 0"。
#[test]
fn fully_transparent_emits_no_draw_commands() {
    let inner = box_el("inner").with(&props::OPACITY, PropertyValue::float(0.0));
    let (_, list) = build(&inner);
    assert!(
        !list.iter().any(|c| matches!(c, Command::Quad { .. })),
        "alpha=0 时不应产出 Quad（画一个全透明的矩形没有任何意义）"
    );
}

// ---------------------------------------------------------------- 3. 偏移落到四类几何

/// 位移只改绘制，**不改布局矩形**。
///
/// 这是"绘制时偏移"的**全部**意义。改了布局矩形会连带改命中测试 ——
/// 症状是"动画期间按钮点不中"，而那种缺陷往往在上线后才被发现。
#[test]
fn offset_does_not_change_layout_rect() {
    let plain = box_el("b");
    let (_, a) = build(&plain);
    let shifted = box_el("b").with(&props::TRANSLATE_Y, PropertyValue::float(20.0));
    let (_, b) = build(&shifted);

    let ra = a
        .iter()
        .find_map(|c| match c {
            Command::Quad { rect, .. } => Some(*rect),
            _ => None,
        })
        .unwrap();
    let rb = b
        .iter()
        .find_map(|c| match c {
            Command::Quad { rect, .. } => Some(*rect),
            _ => None,
        })
        .unwrap();
    assert_eq!(rb.y - ra.y, 20.0, "绘制矩形应下移 20");
}

/// 布局阶段的矩形**真的没变**（与上一条配对的另一半）。
///
/// 只看 `DisplayList` 不够：万一某处把偏移写回了 `Node::rect()`，两条都会"看起来对"。
#[test]
fn offset_does_not_write_back_to_layout_box() {
    let plain = box_el("b");
    let (na, _) = build(&plain);
    let shifted = box_el("b").with(&props::TRANSLATE_Y, PropertyValue::float(20.0));
    let (nb, _) = build(&shifted);
    assert_eq!(
        na.rect(),
        nb.rect(),
        "布局盒必须逐字段相同：偏移是绘制时的，不是布局的"
    );
}

/// 命中测试按**布局盒**判，因此动画期间仍然可点。
///
/// 用 hit 模块的真实入口测（`interaction_tests.rs` 走的是宿主路由，这里直接问 hit）。
#[test]
fn offset_does_not_affect_hit_testing() {
    let plain = box_el("b");
    let shifted = box_el("b").with(&props::TRANSLATE_Y, PropertyValue::float(20.0));
    let theme = Theme::builtin(ThemeVariant::Dark);
    let ctx = Context::new(&theme);

    let mut na = zephyra_core::node::Node::from_element(&plain);
    zephyra_core::layout::layout(&mut na, Rect::new(0.0, 0.0, W, H), ctx);
    let mut nb = zephyra_core::node::Node::from_element(&shifted);
    zephyra_core::layout::layout(&mut nb, Rect::new(0.0, 0.0, W, H), ctx);

    // 未平移时两者都命中原点；这证明"位移没有渗进命中测试"。
    let req = HitTestRequest {
        point: Point::new(10.0, 10.0),
        ignore_above: None,
    };
    let mut ra = HitResult::default();
    let mut rb = HitResult::default();
    assert!(
        zephyra_core::hit::hit_test(&na, req, ctx, &mut ra),
        "未平移的树应当命中"
    );
    assert!(
        zephyra_core::hit::hit_test(&nb, req, ctx, &mut rb),
        "带绘制位移的树也应当命中 —— 命中测试按布局盒判，与绘制无关"
    );
    assert_eq!(ra.path.len(), rb.path.len(), "两条命中路径长度应相同");
}

/// 子节点跟着父节点一起位移（累加，不是取最近一个）。
#[test]
fn translation_accumulates_along_parent_chain() {
    let child = box_el("child");
    let root = Element::new(NodeTag::Container)
        .with_key("root")
        .with(&props::WIDTH, PropertyValue::float(200.0))
        .with(&props::HEIGHT, PropertyValue::float(200.0))
        .with(&props::TRANSLATE_Y, PropertyValue::float(10.0))
        // `Element` 没有 `Clone`（每帧重建的树，克隆它等于把"复用"这条纪律
        // 破掉），所以同一个子树造两遍。
        .with_children([
            child,
            box_el("child2").with(&props::TRANSLATE_Y, PropertyValue::float(5.0)),
        ]);

    let (_, list) = build(&root);
    let ys: Vec<f32> = list
        .iter()
        .filter_map(|c| match c {
            Command::Quad { rect, .. } => Some(rect.y),
            _ => None,
        })
        .collect();
    assert!(ys.len() >= 2, "应当至少两条 Quad，实际 {}", ys.len());
    // 根容器自己没背景 ⇒ 第一条是子节点的。
    assert_eq!(ys[0], 10.0, "只带父位移的子节点");
    assert_eq!(ys[1], 15.0, "父 10 + 自己 5");
}

// ---------------------------------------------------------------- 4. 静止时零开销

/// 位移为零 ⇒ 顶点数组是**同一块内存**（`Arc::ptr_eq`）。
///
/// 这条不是洁癖：Gallery 每帧 19 个图标，若静止时也要为每个图标分配一次
/// `Vec<Vertex>` + `Arc`，那是每帧 19 次无谓分配。判据用 `ptr_eq` 而不是
/// "数值相等" —— 后者对任何实现都成立，等于没测。
#[test]
fn zero_translation_does_not_reallocate_vertices() {
    let theme = Theme::builtin(ThemeVariant::Dark);
    let ctx = Context::new(&theme).with_icons(private_icons(0));
    let el = Element::new(NodeTag::Container)
        .with_key("icon")
        .with(&props::WIDTH, PropertyValue::float(16.0))
        .with(&props::HEIGHT, PropertyValue::float(16.0))
        .with(&props::ICON_NAME, PropertyValue::static_str("home"));
    let mut node = zephyra_core::node::Node::from_element(&el);
    zephyra_core::layout::layout(&mut node, Rect::new(0.0, 0.0, W, H), ctx);

    let render = || {
        let mut list = DisplayList::new();
        zephyra_core::paint::render(&node, ctx, &mut list);
        let v = list
            .iter()
            .find_map(|i| match &i.command {
                Command::Path { vertices, .. } => Some(vertices.clone()),
                _ => None,
            })
            .expect("应当产出 Path 命令");
        v
    };

    let a = render();
    let b = render();

    assert!(
        Arc::ptr_eq(&a, &b),
        "零位移时两次渲染必须共用同一块顶点内存"
    );
}

/// 位移非零 ⇒ 顶点确实动了，且 `uv` 没动。
///
/// 一起测是因为 `uv` 动起来**画面上仍然"只是图标平移了"** —— 只是会花屏。
/// 只测位置会漏掉"平移把图集坐标也平了"这种错误。
#[test]
fn translation_moves_position_but_not_uv() {
    let theme = Theme::builtin(ThemeVariant::Dark);
    let ctx = Context::new(&theme).with_icons(private_icons(1));
    let el = Element::new(NodeTag::Container)
        .with_key("icon")
        .with(&props::WIDTH, PropertyValue::float(16.0))
        .with(&props::HEIGHT, PropertyValue::float(16.0))
        .with(&props::ICON_NAME, PropertyValue::static_str("home"));
    let mut node = zephyra_core::node::Node::from_element(&el);
    zephyra_core::layout::layout(&mut node, Rect::new(0.0, 0.0, W, H), ctx);

    let snapshot = |translate: f32| {
        let mut n = zephyra_core::node::Node::from_element(
            &Element::new(NodeTag::Container)
                .with_key("icon")
                .with(&props::WIDTH, PropertyValue::float(16.0))
                .with(&props::HEIGHT, PropertyValue::float(16.0))
                .with(&props::ICON_NAME, PropertyValue::static_str("home"))
                .with(&props::TRANSLATE_X, PropertyValue::float(translate)),
        );
        zephyra_core::layout::layout(&mut n, Rect::new(0.0, 0.0, W, H), ctx);
        let mut list = DisplayList::new();
        zephyra_core::paint::render(&n, ctx, &mut list);
        let v = list
            .iter()
            .find_map(|i| match &i.command {
                Command::Path { vertices, .. } => Some(vertices.clone()),
                _ => None,
            })
            .expect("应当产出 Path 命令");
        v
    };

    let a = snapshot(0.0);
    let b = snapshot(7.0);
    assert_eq!(a.len(), b.len(), "平移不该改变顶点数");
    for (va, vb) in a.iter().zip(b.iter()) {
        assert_eq!((vb.position.x - va.position.x), 7.0, "X 应平移 7");
        assert_eq!(vb.position.y, va.position.y, "Y 不该动");
        assert_eq!(
            (vb.uv.x, vb.uv.y),
            (va.uv.x, va.uv.y),
            "uv 是图集坐标，平移屏幕位置不该动它"
        );
    }
}

// ---------------------------------------------------------------- 5. 稳态不被破坏

/// 这些属性每帧可能变，但它们**不重排**：稳态必须仍然 `Unchanged`。
///
/// 这条接的是 `tree_needs_layout` 那笔账 —— 若 `TRANSLATE_*` 被误声明成布局属性，
/// 动画会让每一帧都重排，而症状（帧率下降）远不如"控件位置跳"那么容易定位。
#[test]
fn offset_property_does_not_affect_steady_state_decision() {
    let theme = Theme::builtin(ThemeVariant::Dark);
    let ctx = Context::new(&theme);
    let mut cache = FrameCache::new();

    let el = box_el("b");
    cache.frame(&el, ctx, (W, H));

    // 同一帧跑两遍：第二遍必须 Unchanged。
    let second = cache.frame(&el, ctx, (W, H));
    assert!(
        matches!(second, zephyra_core::frame::FrameOutcome::Unchanged),
        "稳态必须 Unchanged"
    );
}

/// 改偏移要 `Updated`（否则动画不会重画）。
///
/// 与上一条配对：既要"静止时不重排"，也要"变了会重画"。
#[test]
fn offset_change_repaints() {
    let theme = Theme::builtin(ThemeVariant::Dark);
    let ctx = Context::new(&theme);
    let mut cache = FrameCache::new();

    let a = box_el("b");
    cache.frame(&a, ctx, (W, H));
    let moved_el = box_el("b").with(&props::TRANSLATE_Y, PropertyValue::float(4.0));
    let out = cache.frame(&moved_el, ctx, (W, H));
    assert!(
        matches!(out, zephyra_core::frame::FrameOutcome::Updated { .. }),
        "位移变了必须重画"
    );
}

/// 引擎替身在偏移后仍能给出字形，且**最终屏幕位置**跟着容器平移。
///
/// # 断言的是「局部 + offset」而不是字形自身
///
/// 字形四边形按**元素原点**缓存（见 `Command::Text::glyphs`），平移体现在
/// `offset` 上。断言 `glyphs[0].dst.x` 变了，等于要求缓存失效重算 —— 那正好是
/// 本次要消掉的开销。所以这里断言**合起来**的结果：`(dst + offset).x` 的差。
#[test]
fn glyphs_translate_with_container() {
    let theme = Theme::builtin(ThemeVariant::Dark);
    let engine = null_engine();
    let ctx = Context::with_engine(&theme, &engine);

    // 返回 (局部首字形 x, offset.x)。
    let origin_x = |tx: f32| {
        let el = Element::new(NodeTag::Text)
            .with_key("t")
            .with(&props::TEXT, PropertyValue::static_str("ab"))
            .with(&props::TRANSLATE_X, PropertyValue::float(tx));
        let mut node = zephyra_core::node::Node::from_element(&el);
        zephyra_core::layout::layout(&mut node, Rect::new(0.0, 0.0, W, H), ctx);
        let mut list = DisplayList::new();
        zephyra_core::paint::render(&node, ctx, &mut list);
        let (dx, off) = list
            .iter()
            .find_map(|i| match &i.command {
                Command::Text { glyphs, offset, .. } => Some((glyphs[0].dst.x, offset.x)),
                _ => None,
            })
            .expect("应当产出字形");
        (dx, off)
    };

    let (da, oa) = origin_x(0.0);
    let (db, ob) = origin_x(12.0);
    assert_eq!(
        (db + ob) - (da + oa),
        12.0,
        "字形的最终屏幕位置应随容器平移 12"
    );
}

/// 跨帧**复用同一份字形四边形**（`Arc::ptr_eq`），只有 `offset` 变。
///
/// 这条是本轮优化的主判据：`Vec<GlyphQuad>` 每帧 239 次 × 1024 B = 136 KB，
/// 占 resize 帧分配量的 40%。改完之后，字形四边形必须**一个字节都不重算**。
///
/// # 为什么改 `TRANSLATE_X` 而不是改视口宽
///
/// 视口宽变了确实会重排，但**裸 `Text` 节点的摆放矩形不变**（它在根容器里
/// 从 x=0 起排），于是 `offset` 也不会变 —— 那就测不到「局部量复用 + 只有
/// offset 变」这件事。`TRANSLATE_X` 只重绘不重排，正好把「几何变了但排版输入
/// 一个字节没变」这个组合单独拎出来。
#[test]
fn glyph_quads_are_not_recomputed_on_translation() {
    let theme = Theme::builtin(ThemeVariant::Dark);
    let engine = null_engine();
    let ctx = Context::with_engine(&theme, &engine);

    let mut cache = zephyra_core::frame::FrameCache::new();
    let grab = |cache: &mut zephyra_core::frame::FrameCache, tx: f32| {
        let el = Element::new(NodeTag::Text)
            .with_key("t")
            .with(&props::TEXT, PropertyValue::static_str("ab"))
            .with(&props::TRANSLATE_X, PropertyValue::float(tx));
        cache.frame(&el, ctx, (W, H));
        let mut list = DisplayList::new();
        // 直接重放缓存里的保留树，绕过 `frame` 的「要不要重画」判定 ——
        // 那条判定会让第二次调用返回 `Unchanged` 而不回放，字形就取不到了。
        zephyra_core::paint::render(cache.node().expect("frame 后应有保留树"), ctx, &mut list);
        // 必须先把结果绑到局部变量再返回：直接把 `list.iter()…` 放在块尾，
        // 那个借用 `list` 的临时 `Iterator` 会活到 `list` 之后才析构
        // （tail expression 的临时析构顺序）⇒ E0597。
        let found = list
            .iter()
            .find_map(|i| match &i.command {
                Command::Text { glyphs, offset, .. } => {
                    Some((std::sync::Arc::clone(glyphs), *offset))
                }
                _ => None,
            })
            .expect("应当产出字形");
        found
    };

    let (a, off_a) = grab(&mut cache, 0.0);
    let (b, off_b) = grab(&mut cache, 12.0);
    assert!(!a.is_empty(), "NullEngine 也应给出字形");
    assert!(
        std::sync::Arc::ptr_eq(&a, &b),
        "跨帧必须复用同一份字形四边形 —— 否则 136 KB/帧 的分配还在"
    );
    assert_eq!(
        off_b.x - off_a.x,
        12.0,
        "复用的前提下，只有 origin（offset）该变"
    );
}

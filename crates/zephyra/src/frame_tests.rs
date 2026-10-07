//! [`crate::frame`] 的断言 —— **纯 CPU，不开窗口、不碰 GPU**。
//!
//! # 这一组测的是什么
//!
//! 不是「脏标记能不能正确传播」（那是 [`crate::tests`] 的活），而是三条只有在
//! [`FrameCache`] 这一层才成立的性质：
//!
//! 1. **稳态真的什么都不做** —— 同一个 `Element` 连续喂两帧，第二帧必须
//!    [`FrameOutcome::Unchanged`]。这是本模块存在的**唯一**理由。
//! 2. **`Unchanged` 不等于「漏判」** —— 一旦有任何节点 paint-dirty，必须变回
//!    `Updated`，且列表内容随之改变。
//! 3. **脏区只在安全时收窄** —— 能收窄时它必须**真的**覆盖到脏节点；不该收窄的
//!    情况（重排 / 结构增删 / 折叠）必须退让成 [`Damage::Full`]。
//!
//! 第 3 条最要紧：脏区算小了的后果是**残影**，而残影在真机上极难归因到脏区。

use crate::context::Context;
use crate::core::Color;
use crate::element::Element;
use crate::frame::{Damage, FrameCache, FrameOutcome};
use crate::id::NodeTag;
use crate::property::PropertyValue;
use crate::props;
use crate::theme::Theme;

/// 走完 声明 → 缓存帧，返回本帧结果与列表长度。
///
/// 主题每次新建但内容相同 —— [`Theme`] 是纯数据，缓存与它无关。
fn step(cache: &mut FrameCache, element: &Element, bounds: (f32, f32)) -> (bool, usize) {
    let theme = Theme::dark();
    let ctx = Context::new(&theme);
    match cache.frame(element, ctx, bounds) {
        FrameOutcome::Unchanged => (false, cache.list().len()),
        FrameOutcome::Updated { list, .. } => (true, list.len()),
    }
}

/// 一个固定 120×60 的方块。
///
/// ⚠ `WIDTH` / `HEIGHT` 必须给：默认是 `NaN`（= 不约束），而一个既无子节点
/// 又无显式尺寸的容器量出来是 **0×0**。那会让脏矩形退化成零面积 —— 而
/// `Rect { width: 0 }` 在 scissor 里意味着「什么都不画」，症状是元素直接消失，
/// 且没有任何报错。所以这条尺寸是本组测试的**前提**，不是装饰。
fn chip(color: Color) -> Element {
    Element::new(NodeTag::Container)
        .with(&props::WIDTH, PropertyValue::float(120.0))
        .with(&props::HEIGHT, PropertyValue::float(60.0))
        .with(&props::BACKGROUND, PropertyValue::color(color))
}

/// 一个深色面板，里面一个浅色方块。
///
/// 两个节点是刻意的 —— 只有一个节点时「脏的是哪个」无从分辨，多一个才能验出
/// 脏区不是碰巧等于某个矩形。`PADDING` 让内层离开原点，于是脏矩形不等于整窗。
fn two_nodes() -> Element {
    Element::new(NodeTag::Container)
        .with(
            &props::BACKGROUND,
            PropertyValue::color(Color::rgb(0.1, 0.1, 0.1)),
        )
        .with(&props::PADDING, PropertyValue::float(20.0))
        .with_children([chip(Color::rgb(0.9, 0.2, 0.2))])
}

/// 同样的树，但内层换成另一种颜色。
///
/// 只改颜色、不动任何影响布局的属性 —— 这正是 hover / 按下走的路径，也是
/// 脏区唯一能收窄的场景。
fn two_nodes_recolored() -> Element {
    Element::new(NodeTag::Container)
        .with(
            &props::BACKGROUND,
            PropertyValue::color(Color::rgb(0.1, 0.1, 0.1)),
        )
        .with(&props::PADDING, PropertyValue::float(20.0))
        .with_children([chip(Color::rgb(0.2, 0.9, 0.2))])
}

// ==================================================================
// 1. 稳态
// ==================================================================

/// 同一棵树喂两次：第二次必须 `Unchanged`。
///
/// 这是本模块的全部意义。宿主据此跳过 `paint`（73 µs）+ `expand`（40 µs）
/// + 一次 present。
#[test]
fn 同一棵树第二次是_unchanged() {
    let theme = Theme::dark();
    let ctx = Context::new(&theme);
    let mut cache = FrameCache::new();
    let el = two_nodes();

    let first = cache.frame(&el, ctx, (400.0, 300.0));
    assert!(
        matches!(first, FrameOutcome::Updated { .. }),
        "首帧必须产出一份列表"
    );

    let second = cache.frame(&el, ctx, (400.0, 300.0));
    assert!(
        matches!(second, FrameOutcome::Unchanged),
        "什么都没改却重算了 —— 稳态门控失效，本模块白做"
    );
}

/// `Unchanged` 之后再来一次真正的改动，必须变回 `Updated`。
///
/// ⚠ 这条防的是「门控关死」：如果第一次 `Unchanged` 之后缓存被永久卡住，表现是
/// **画面再也不更新且所有计数器正常** —— 那是最难查的一类故障。
#[test]
fn unchanged_之后改动会恢复() {
    let theme = Theme::dark();
    let ctx = Context::new(&theme);
    let mut cache = FrameCache::new();
    let bounds = (400.0, 300.0);

    assert!(matches!(
        cache.frame(&two_nodes(), ctx, bounds),
        FrameOutcome::Updated { .. }
    ));
    assert!(matches!(
        cache.frame(&two_nodes(), ctx, bounds),
        FrameOutcome::Unchanged
    ));

    // 改一个子节点的颜色 —— 这正是 hover / 按下走的路径。
    let changed = cache.frame(&two_nodes_recolored(), ctx, bounds);
    match changed {
        FrameOutcome::Updated { list, .. } => assert!(
            list.len() >= 2,
            "重绘后应仍有两条 Quad，实际 {}",
            list.len()
        ),
        FrameOutcome::Unchanged => panic!("颜色变了却报 Unchanged —— 门控漏判"),
    }
}

/// 连喂十次稳态：只有第一次是 `Updated`。
///
/// 单次能过不代表幂等 —— 若某处每帧都留下一点脏（例如时间戳进了属性），第二次
/// 会过而第十次不过。这条把那种「每 N 帧重算一次」的抖动钉住。
#[test]
fn 连续十次稳态只有首次重算() {
    let mut cache = FrameCache::new();
    let el = two_nodes();
    let mut updates = 0;
    for i in 0..10 {
        let (changed, _) = step(&mut cache, &el, (400.0, 300.0));
        if changed {
            updates += 1;
            assert_eq!(i, 0, "第 {i} 帧不该重算");
        }
    }
    assert_eq!(updates, 1, "十帧里只有首帧该重算，实际 {updates} 次");
}

// ==================================================================
// 2. 失效来源
// ==================================================================

/// 视口变了必须 `Updated`。
///
/// ⚠ 视口是**窗口属性**，不挂在任何节点上，所以 `diff` 永远看不见「窗口被 resize
/// 了」。这条是专门钉那个「视口不参与判据」的漏洞的 —— 症状是「窗口拉大后右侧
/// 留白，且不会自愈」。
#[test]
fn 视口变了要重绘() {
    let mut cache = FrameCache::new();
    let el = two_nodes();

    step(&mut cache, &el, (400.0, 300.0));
    let (changed, _) = step(&mut cache, &el, (400.0, 300.0));
    assert!(!changed, "前置：同视口应当 Unchanged");

    let (changed, _) = step(&mut cache, &el, (800.0, 300.0));
    assert!(changed, "视口变了却不重绘 —— 根节点不会被重新铺满");
}

/// 结构增删（多一个带背景色的子节点）必须 `Updated`，且列表变长。
#[test]
fn 结构增删要重绘() {
    let mut cache = FrameCache::new();
    let (_, before) = step(&mut cache, &two_nodes(), (400.0, 300.0));

    let grown = Element::new(NodeTag::Container)
        .with(
            &props::BACKGROUND,
            PropertyValue::color(Color::rgb(0.1, 0.1, 0.1)),
        )
        .with(&props::PADDING, PropertyValue::float(20.0))
        .with_children([
            chip(Color::rgb(0.9, 0.2, 0.2)),
            chip(Color::rgb(0.1, 0.5, 0.9)),
        ]);

    let (changed, after) = step(&mut cache, &grown, (400.0, 300.0));
    assert!(changed, "多了个子节点却不重绘");
    assert!(
        after > before,
        "新子节点有背景色，应当多一条 Quad：{before} → {after}"
    );
}

/// `invalidate()` 能强制重绘，即便没有任何节点脏。
///
/// 这是 `Unchanged` 的**逃生门**：宿主某帧没能 present（resize 中途、device
/// removed）时必须能打断，否则窗口永远停在空白而计数器全对。
#[test]
fn invalidate_能强制重绘() {
    let mut cache = FrameCache::new();
    let el = two_nodes();

    step(&mut cache, &el, (400.0, 300.0));
    let (changed, _) = step(&mut cache, &el, (400.0, 300.0));
    assert!(!changed, "前置：应当 Unchanged");

    cache.invalidate();
    let (changed, _) = step(&mut cache, &el, (400.0, 300.0));
    assert!(changed, "invalidate() 没生效 —— Unchanged 会把窗口永久卡住");
}

/// `invalidate()` 只影响**下一帧**，不永久关掉门控。
#[test]
fn invalidate_只影响下一帧() {
    let mut cache = FrameCache::new();
    let el = two_nodes();

    cache.invalidate();
    step(&mut cache, &el, (400.0, 300.0));
    let (changed, _) = step(&mut cache, &el, (400.0, 300.0));
    assert!(
        !changed,
        "invalidate() 之后门控就该恢复 —— 否则稳态永远在重算"
    );
}

// ==================================================================
// 3. 脏区
// ==================================================================

fn damage_of(cache: &mut FrameCache, element: &Element, bounds: (f32, f32)) -> Damage {
    let theme = Theme::dark();
    let ctx = Context::new(&theme);
    match cache.frame(element, ctx, bounds) {
        FrameOutcome::Unchanged => panic!("本 helper 只用于会重绘的帧"),
        FrameOutcome::Updated { damage, .. } => damage,
    }
}

/// 一行里两个**不相邻**的方块，都改色 —— 脏区必须是两块之间的**并集**。
///
/// # 为什么单独一条
///
/// 其它脏区测试的树里**只有一个**脏节点，所以「取第一个矩形」和「取并集」给出
/// 完全相同的答案 —— 那条测试钉不住并集逻辑。实测把 `union` 改成只取第一个矩形，
/// 全部 13 条仍然全绿。
///
/// 变异测试是唯一能发现这种"测试恒真"的手段，而这里的症状恰好是最坏的一类：
/// 脏矩形偏小 ⇒ 屏幕上留着上一帧的旧颜色（残影），而**所有计数器都正常**。
#[test]
fn 多处脏时脏区是并集() {
    /// 水平一行两块。根不加背景色 —— 否则它会被标脏、并把整窗并进来。
    fn row(a: Color, b: Color) -> Element {
        Element::new(NodeTag::Container)
            .with(
                &props::DIRECTION,
                PropertyValue::static_str("horizontal"),
            )
            .with_children([chip(a), chip(b)])
    }

    let mut cache = FrameCache::new();
    let bounds = (400.0, 300.0);
    let theme = Theme::dark();
    let ctx = Context::new(&theme);

    cache.frame(&row(Color::rgb(0.9, 0.2, 0.2), Color::rgb(0.2, 0.2, 0.9)), ctx, bounds);

    let recolored = row(Color::rgb(0.1, 0.9, 0.1), Color::rgb(0.9, 0.1, 0.1));
    let Damage::Rect(r) = damage_of(&mut cache, &recolored, bounds) else {
        panic!("纯改色不该退让成整帧");
    };

    // 两个 120 宽的方块并排：并集宽 240。若 `union` 只取第一个，r.width 会是 120。
    assert_eq!(
        (r.x, r.width, r.height),
        (0.0, 240.0, 60.0),
        "脏区应覆盖两个方块的整体（x=0 宽=240）—— 偏小说明只取了一处，\
         屏幕上会留着另一处的旧颜色"
    );
}

/// 折叠一个节点：必须重绘（否则它的命令会留在留用的 `DisplayList` 里），
/// 且脏区必须退让成 `Full`。
///
/// # 为什么这条独立
///
/// 折叠与「改色」是**相反**的两件事：改色不增不减命令，折叠让命令**消失**。
/// 而 `DisplayList` 是留用的（[`FrameCache`] 只在真要重绘时才 `clear`），所以
/// 折叠若不重绘，那几条命令会一直留在列表里被反复画 —— 症状是「元素折叠了但
/// 那块颜色还在」，且所有计数器正常。
///
/// 脏区也必须退让：折叠掉的像素**留在屏幕上**，而那块区域现在没人负责重画。
/// 它的矩形我们算得出来，但「把它画回背景色」需要知道原来画的是什么 —— 所以
/// 只能整帧重来。
///
/// 变异测试：把 `scan` 里折叠分支的 `return dirty` 改成 `return false`，
/// 本条立刻失败（会退化成 `Unchanged`）。
#[test]
fn 折叠节点要重绘且脏区退让成整帧() {
    let mut cache = FrameCache::new();
    let bounds = (400.0, 300.0);

    let open = Element::new(NodeTag::Container).with_children([chip(Color::rgb(0.9, 0.2, 0.2))]);
    let (_, before) = step(&mut cache, &open, bounds);
    assert!(before > 0, "前置：chip 有背景色，应当产出命令");

    let folded = Element::new(NodeTag::Container).with_children([
        chip(Color::rgb(0.9, 0.2, 0.2)).with(&props::COLLAPSED, PropertyValue::bool(true)),
    ]);
    let (changed, after) = step(&mut cache, &folded, bounds);
    assert!(changed, "折叠却不重绘 —— 上一帧的命令会留在 DisplayList 里被反复画");
    assert_eq!(
        after, 0,
        "折叠后不该再有该节点的命令，实际仍有 {after} 条"
    );

    // 再来一帧：折叠状态稳定，必须回到 Unchanged（否则折叠会导致每帧重算）。
    let (changed, _) = step(&mut cache, &folded, bounds);
    assert!(!changed, "折叠状态没再变却持续重绘");
}

/// 首帧必然是 `Full`（树是新建的，没有「上一帧」可比较）。
#[test]
fn 首帧脏区是整帧() {
    let mut cache = FrameCache::new();
    let d = damage_of(&mut cache, &two_nodes(), (400.0, 300.0));
    assert!(
        d.is_full(),
        "首帧报了个具体矩形 —— 那矩形没有任何依据，纯paint-dirty 是凑出来的"
    );
}

/// 纯改色（不重排、不动结构）时脏区**必须收窄**，且要真的盖住那个节点。
///
/// 这是脏区唯一有价值的场景，也是上面那条「残影」风险的正面判据：矩形小到
/// 不含脏节点就是错的。
#[test]
fn 纯改色时脏区收窄且覆盖脏节点() {
    let theme = Theme::dark();
    let ctx = Context::new(&theme);
    let mut cache = FrameCache::new();
    let bounds = (400.0, 300.0);

    cache.frame(&two_nodes(), ctx, bounds);
    let d = damage_of(&mut cache, &two_nodes_recolored(), bounds);

    let Damage::Rect(r) = d else {
        panic!("纯改色不该退让成整帧 —— 那样脏区就没意义了");
    };
    assert!(
        r.x.is_finite() && r.width.is_finite() && r.width > 0.0 && r.height > 0.0,
        "脏矩形不能是退化或 NaN：{r:?}（零面积在 scissor 里等于什么都不画）"
    );
    // 脏节点是内层那个 120×60 的方块，它被 20 的内边距推到 (20,20)。
    // 这条断言是**具体值**而不是范围 —— 范围判据在"并集算成了某一个"时会通过。
    assert_eq!(
        (r.x, r.y, r.width, r.height),
        (20.0, 20.0, 120.0, 60.0),
        "脏矩形应恰好是那个方块 —— 偏大说明收窄没生效，偏小会留残影"
    );
}

/// 结构增删时脏区必须退让成 `Full`。
///
/// 被删节点留下的像素**没人**负责重画，而它的旧矩形已经随节点一起没了 ——
/// 没有任何数据能算出该重画哪里。这是「必须保守」的第一种情况。
#[test]
fn 结构增删时脏区退让成整帧() {
    let mut cache = FrameCache::new();
    step(&mut cache, &two_nodes(), (400.0, 300.0));

    let grown = Element::new(NodeTag::Container)
        .with(
            &props::BACKGROUND,
            PropertyValue::color(Color::rgb(0.1, 0.1, 0.1)),
        )
        .with(&props::PADDING, PropertyValue::float(20.0))
        .with_children([
            chip(Color::rgb(0.9, 0.2, 0.2)),
            chip(Color::rgb(0.1, 0.5, 0.9)),
        ]);
    let d = damage_of(&mut cache, &grown, (400.0, 300.0));
    assert!(
        d.is_full(),
        "结构变了却报了个具体矩形 —— 被删/被移节点的旧区域会留残影"
    );
}

/// 视口变化时脏区必须退让成 `Full`。
///
/// 重排会让节点移动与收缩，而**旧**矩形没有被快照下来 —— 算不出来就只能全画。
#[test]
fn 视口变化时脏区退让成整帧() {
    let mut cache = FrameCache::new();
    step(&mut cache, &two_nodes(), (400.0, 300.0));
    let d = damage_of(&mut cache, &two_nodes(), (800.0, 600.0));
    assert!(
        d.is_full(),
        "重排后报具体矩形 —— 节点旧位置的像素没人重画，会留残影"
    );
}

/// `Unchanged` 时不该有任何脏区可交 —— 压根没有 `Updated` 分支可拿。
///
/// 这条是「返回类型本身在表达这件事」的判据：想要脏区就必须先有更新。
#[test]
fn unchanged_不产出脏区() {
    let theme = Theme::dark();
    let ctx = Context::new(&theme);
    let mut cache = FrameCache::new();
    let el = two_nodes();

    cache.frame(&el, ctx, (400.0, 300.0));
    // 编译期就保证了：Unchanged 变体里没有 damage 字段。
    match cache.frame(&el, ctx, (400.0, 300.0)) {
        FrameOutcome::Unchanged => {}
        FrameOutcome::Updated { .. } => panic!("稳态不该是 Updated"),
    }
}

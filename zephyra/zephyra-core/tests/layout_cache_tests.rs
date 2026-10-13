//! 布局缓存的**可观测**判据：一次 layout 趟里有多少节点真的重测了。
//!
//! # 为什么用「文字整形次数」当计时器
//!
//! 本仓反复强调「性能判断必须来自计时，估算 + 论证不算」。而布局里最贵的
//! 一步就是 `TextEngine::shape`（整段文字重新排版 + 生成字形位置）。
//! `Node::measure_cache` 是 `pub(crate)`，测试拿不到；但 shape 调用次数
//! **恰好等价于**"有多少节点真的重测了"—— 因为每个带文本的节点在 measure
//! 里恰好调一次 shape。
//!
//! 官方依据：`microsoft-ui-xaml\dxaml\xcp\core\core\elements\StackPanel.cpp:24-30`
//!
//! ```cpp
//! XSIZEF childConstraint = (m_orientation == DirectUI::Orientation::Vertical)
//!     ? XSIZEF{ availableSize.width - combinedThickness.width, XFLOAT_INF }
//!     : XSIZEF{ XFLOAT_INF, availableSize.height - combinedThickness.height };
//! ```
//!
//! 堆叠轴给无穷的直接后果：**只改窗口高度时，子项的约束一个字都没变**
//! ⇒ `PreviousConstraint` 精确命中 ⇒ 子树整段跳过测量。这是官方 resize
//! 便宜的原因，也是本组要复现的东西。

use std::cell::{Cell, RefCell};

use zephyra_core::context::Context;
use zephyra_core::core::Rect;
use zephyra_core::element::Element;
use zephyra_core::id::NodeTag;
use zephyra_core::layout::{self, Constraints};
use zephyra_core::node::Node;
use zephyra_core::property::PropertyValue;
use zephyra_core::props;
use zephyra_core::paint::{GlyphQuad, TextureRef};
use zephyra_core::text::{NullEngine, ShapedText, TextEngine, TextRender, TextStyle};
use zephyra_core::theme::{Theme, ThemeVariant};
use zephyra_core::core::Point;

/// 会数次数的替身。
struct CountingEngine {
    inner: NullEngine,
    shapes: Cell<usize>,
    /// 每次 `shape` 的入参宽度（用来验证缓存命中时**真的没调用**）。
    _unused: RefCell<Vec<f32>>,
}

impl CountingEngine {
    fn new() -> Self {
        Self {
            inner: NullEngine { char_width: 8.0 },
            shapes: Cell::new(0),
            _unused: RefCell::new(Vec::new()),
        }
    }
}

impl TextEngine for CountingEngine {
    fn shape(&mut self, text: &str, style: &TextStyle, max_width: Option<f32>) -> ShapedText {
        self.shapes.set(self.shapes.get() + 1);
        self.inner.shape(text, style, max_width)
    }
    fn advance(&mut self, ch: char, style: &TextStyle) -> f32 {
        self.inner.advance(ch, style)
    }
    fn line_height(&mut self, style: &TextStyle) -> f32 {
        self.inner.line_height(style)
    }
}

impl TextRender for CountingEngine {
    fn glyph_quads(
        &mut self,
        shaped: &ShapedText,
        origin: Point,
        style: &TextStyle,
    ) -> Option<(TextureRef, Vec<GlyphQuad>)> {
        self.inner.glyph_quads(shaped, origin, style)
    }
}

/// 一个带文字的纵向堆叠：`n` 个固定宽高、可 stretch 的方块，各带一行字。
fn vertical_stack(n: usize) -> Element {
    let children: Vec<Element> = (0..n)
        .map(|i| {
            Element::new(NodeTag::Container)
                .with_key_by_index(i)
                .with(&props::HEIGHT, PropertyValue::float(40.0))
                .with(&props::PADDING, PropertyValue::float(4.0))
                .with(
                    &props::TEXT,
                    PropertyValue::static_str("item text here"),
                )
        })
        .collect();
    Element::new(NodeTag::Container)
        .with_key("root")
        .style_hint("vertical")
        .with_children(children)
}

/// 一棵建好的树 + 它的上下文。
///
/// ⚠ 必须是**同一棵树**跑多趟：`measure_cache` 挂在 `Node` 上，每次
/// `Node::from_element` 新建一棵就等于每趟都是首趟，缓存永远不命中 ——
/// 那测出来的是"建树成本"，不是"重排成本"。
struct Fixture {
    engine: RefCell<CountingEngine>,
    theme: Theme,
    node: Node,
}

impl Fixture {
    fn new(n: usize) -> Self {
        let engine = RefCell::new(CountingEngine::new());
        let theme = Theme::builtin(ThemeVariant::Dark);
        let node = Node::from_element(&vertical_stack(n));
        Self { engine, theme, node }
    }

    /// 跑一趟 layout，返回**这一趟新增的**整形次数。
    ///
    /// 取增量而不是累计值：累计值让"第二趟是 0 次"这种判据必须先知道首趟是
    /// 多少，两处真相容易对不上。
    fn layout(&mut self, size: (f32, f32)) -> usize {
        let ctx = Context::with_engine(&self.theme, &self.engine);
        let before = self.engine.borrow().shapes.get();
        crate::layout::layout(
            &mut self.node,
            Rect::new(0.0, 0.0, size.0, size.1),
            ctx,
        );
        zephyra_core::node::clear_dirty(&mut self.node);
        self.engine.borrow().shapes.get() - before
    }
}

/// 子项数量。
const N: usize = 6;

/// 改**高度**时，子树应当整体跳过测量（官方 StackPanel 的无穷约束）。
///
/// 这条在修好之前是红的：子项拿到的是父的有限约束，改高度 ⇒ 每个子项的
/// `measure_cache` 键都变 ⇒ 整段子树重排。
#[test]
fn 只改高度不重测子树() {
    let mut f = Fixture::new(N);
    let first = f.layout((800.0, 600.0));
    assert!(first > 0, "首趟必然要测量（实际 {first}）");

    let second = f.layout((800.0, 700.0));
    assert_eq!(
        second, 0,
        "只改高度时子项约束一个字都没变（堆叠轴是无穷），应当 0 次整形；实际 {second}"
    );
}

/// 改**宽度**时必须重测（宽度是交叉轴，官方也给有限值）。
///
/// 与上一条配对。只有上一条会让人误以为"约束根本不参与"，而那在宽度变化时
/// 会立刻给出错误宽度 —— 那是最容易发现的症状。
#[test]
fn 改宽度会重测子树() {
    let mut f = Fixture::new(N);
    let first = f.layout((800.0, 600.0));
    let third = f.layout((900.0, 600.0));
    assert_eq!(
        third, first,
        "改宽度必须与首趟同样多地重测（首趟 {first}、本次 {third}）"
    );
}

/// 稳态（尺寸不变）必须完全不重测。
///
/// 这条在**旧代码**下也应当是绿的（缓存键相同就命中），但它是上面两条的
/// 前提：若稳态都在重测，"只改高度不重测"就毫无意义。
#[test]
fn 稳态完全不重测() {
    let mut f = Fixture::new(N);
    f.layout((800.0, 600.0));
    let again = f.layout((800.0, 600.0));
    assert_eq!(again, 0, "尺寸不变时应当 0 次整形");
}

/// 高度**来回**改也不该重测（不只��"第一次改"有效）。
///
/// 守住"缓存键真的与高度无关"这件事。只测一次的话，一个"第一次看见高度变化
/// 就把子树标脏"的实现也能过。
#[test]
fn 高度来回改都不重测() {
    let mut f = Fixture::new(N);
    f.layout((800.0, 600.0));
    for h in [700.0, 500.0, 900.0, 600.0] {
        let n = f.layout((800.0, h));
        assert_eq!(n, 0, "高度 {h} 不该引起任何重测，实际 {n}");
    }
}

/// 子项在**交叉轴**上仍然拿到有限约束（`stretch` 依赖它）。
///
/// 若堆叠轴给无穷时**交叉轴也**被放成无穷，`stretch` 就无从谈起 ——
/// 子项会退回内容宽度，页面横向排版全散。这条守住"只放开一根轴"。
#[test]
fn 交叉轴仍是有限约束() {
    let mut f = Fixture::new(2);
    f.layout((800.0, 600.0));
    let child = f.node.children().first().expect("有子项");
    assert_eq!(
        child.rect().width, 800.0,
        "纵向堆叠 + stretch ⇒ 子项应撑满交叉轴（父的宽度）"
    );
}

/// 无界约束下不得产出 `f32::INFINITY` 的尺寸。
///
/// 官方在 `uielement.cpp:4276-4295` 对此**硬失败**：
/// `if (IsInfiniteF(desiredSize.width) || ...) hr = E_UNEXPECTED;`
/// 我们的做法是让 `pick_axis` 在上限无界时跳过夹逼，所以这一条守的是
/// "别在别的地方漏出无穷"。
#[test]
fn 无界约束下尺寸是有限值() {
    let engine = RefCell::new(CountingEngine::new());
    let theme = Theme::builtin(ThemeVariant::Dark);
    let ctx = Context::with_engine(&theme, &engine);
    let mut node = Node::from_element(&vertical_stack(3));
    // 整个窗口无界（模拟"在滚动容器里测内容"）。
    let size = crate::layout::measure(&mut node, Constraints::unbounded(), ctx);
    assert!(
        size.width.is_finite() && size.height.is_finite(),
        "无界约束下测出来的尺寸必须是有限的，实际 {size:?}"
    );
    assert!(size.width > 0.0 && size.height > 0.0);
}
// ---------------------------------------------------------------- 小工具

trait TestExt {
    fn with_key_by_index(self, i: usize) -> Self;
    fn style_hint(self, dir: &'static str) -> Self;
}

impl TestExt for Element {
    /// 稳定的 key（`idx-N`），否则 `Node::from_element` 复用子树时行为不可测。
    fn with_key_by_index(self, i: usize) -> Self {
        // 用静态表而不是 `format!`：每帧重建的树里做一次分配是不可接受的，
        // 而这里要构造 6 个节点，静态表够用。
        const KEYS: [&str; 6] = ["i0", "i1", "i2", "i3", "i4", "i5"];
        self.with_key(KEYS[i])
    }
    fn style_hint(self, _dir: &'static str) -> Self {
        // 纵向堆叠是 `Stack::default()` 之外的语义，所以这里直接下发方向。
        self.with(&props::DIRECTION, PropertyValue::direction(zephyra_core::layout::Direction::Vertical))
    }
}
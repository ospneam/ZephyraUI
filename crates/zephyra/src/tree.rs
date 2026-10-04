//! 元素树：声明 → 保留 → diff。
//!
//! # 两棵树
//!
//! - [`Element`]：**声明**。每次「重建」产生一棵，轻量、可丢弃，描述「我想要什么」。
//! - [`Node`]：**保留**。树上每个元素对应的持久状态（属性表、事件处理器、
//!   纹理缓存键、布局结果、脏标记）。
//!
//! 每帧不是「重建 Node」，而是「拿新 Element 与旧 Node 对账」（diff）。这一步是
//! 保留式相对即时式的全部收益所在：状态与纹理得以跨帧留存，只有真正变化的子树
//! 被重排/重绘。
//!
//! # 按类型与位置配对
//!
//! diff 的第一步是把新旧子节点配对。规则：
//!
//! 1. 同位置同类型 → 就地更新；
//! 2. 同位置不同类型 → 视为替换（丢弃旧 Node，建新的）；
//! 3. 类型不匹配时向前/向后找同类型的旧节点 → 移动（保状态）；
//! 4. 都找不到 → 插入。
//!
//! 规则 3 是「列表里插一项不该让后面全部重建状态」的关键。带 key 的子节点直接
//! 按 key 配对，跳过扫描。
//!
//! # 脏标记的粒度
//!
//! [`Node::dirty`] 至少要区分四个等级，因为它们触发的工作量差一个数量级：
//!
//! | 等级 | 触发 | 代价 |
//! |---|---|---|
//! | [`Dirty::None`] | — | 零 |
//! | [`Dirty::Paint`] | 颜色变了 | 重生成该子树的 DisplayList 段 |
//! | [`Dirty::Layout`] | 尺寸变了 | 重排该子树 |
//! | [`Dirty::Structure`] | 子节点增删 | 重建该子树 + 重排 |
//!
//! 把 `Background` 变更当成 `Layout` 处理，是 UI 卡顿最常见的单一原因。

use std::sync::Arc;

use crate::core::Size;
use crate::layout::{Constraints, MeasureResult};
use crate::paint::DisplayList;
use crate::property::{PropertyKey, PropertyStore, PropertyValue};

/// 脏标记等级。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum Dirty {
    /// 干净。
    #[default]
    None,
    /// 只需重绘。
    Paint,
    /// 需重排。
    Layout,
    /// 结构变了（子节点增删）。
    Structure,
}

impl Dirty {
    /// 取两者中较重的一个。
    ///
    /// 脏标记沿树向上传播时用这个合并——只要子树里有任何一处结构变化，父节点就
    /// 必须重排。
    pub fn merged(self, other: Dirty) -> Dirty {
        if other > self {
            other
        } else {
            self
        }
    }

    /// 是否需要重排。
    pub fn needs_layout(self) -> bool {
        self >= Dirty::Layout
    }
}

/// 元素类型标签。
///
/// diff 用它判断「能否就地复用」。用枚举而非 `TypeId` 是为了让「宿主定义了自定义
/// 元素类型」这件事在类型层面就是非法的——自定义元素应当是一个实现了
/// [`ElementKind`] 的新类型，而不是在字符串里登记。
pub trait ElementKind {
    /// 该元素对应的 Node 类型标签。
    fn node_tag(&self) -> NodeTag;
}

/// 节点标签（用于 diff 配对）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeTag {
    /// 文本节点。
    Text,
    /// 图像节点。
    Image,
    /// 布局容器。
    Container,
    /// 控件（具体类型由控件层定义）。
    Control,
}

/// 声明式元素。
///
/// 刻意**不**提供通用构造器 + `Vec<Element>` 字段：那等于把「什么都能塞」的
/// 袋子暴露给所有代码，`ElementKind` 也就无从约束。真实 API 由本 crate 的
/// 构造函数或宏提供，字段保持私有。
pub struct Element {
    tag: NodeTag,
    key: Option<ElementKey>,
    props: PropertyStore,
    children: Vec<Element>,
}

impl Element {
    /// 创建一个元素（供本 crate 的构造函数与控件层使用）。
    pub fn new(tag: NodeTag) -> Self {
        Self {
            tag,
            key: None,
            props: PropertyStore::new(),
            children: Vec::new(),
        }
    }

    /// 设置属性。
    pub fn with(mut self, key: &PropertyKey, value: PropertyValue) -> Self {
        self.props.set_local(key, value);
        self
    }

    /// 设置子节点。
    pub fn with_children(mut self, children: impl IntoIterator<Item = Element>) -> Self {
        self.children = children.into_iter().collect();
        self
    }

    /// 设置稳定 key（让 diff 按身份而非位置配对）。
    pub fn with_key(mut self, key: impl Into<ElementKey>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// 类型标签。
    pub fn tag(&self) -> NodeTag {
        self.tag
    }

    /// 稳定 key。
    pub fn key(&self) -> Option<&ElementKey> {
        self.key.as_ref()
    }

    /// 属性表。
    pub fn props(&self) -> &PropertyStore {
        &self.props
    }

    /// 子节点。
    pub fn children(&self) -> &[Element] {
        &self.children
    }

    /// 是否为「无子树」的叶子元素（diff 时可跳过结构比对）。
    pub fn is_leaf(&self) -> bool {
        self.children.is_empty()
    }
}

/// 元素身份。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ElementKey {
    /// 整数 id。
    Id(u64),
    /// 字符串 id。
    Name(Arc<str>),
}

impl From<&'static str> for ElementKey {
    fn from(s: &'static str) -> Self {
        ElementKey::Name(Arc::from(s))
    }
}

impl From<u64> for ElementKey {
    fn from(v: u64) -> Self {
        ElementKey::Id(v)
    }
}

/// 保留节点：树上每个元素对应的持久状态。
pub struct Node {
    tag: NodeTag,
    key: Option<ElementKey>,
    props: PropertyStore,
    children: Vec<Node>,
    /// 测量结果缓存（`Dirty >= Layout` 时失效）。
    measure_cache: Option<(Constraints, MeasureResult)>,
    /// 测量结果缓存所用的约束。
    arranged_rect: crate::core::Rect,
    dirty: Dirty,
}

impl Node {
    /// 由元素构建节点（首次进入树时）。
    pub fn from_element(element: &Element) -> Node {
        Node {
            tag: element.tag,
            key: element.key.clone(),
            props: element.props.clone(),
            children: element.children.iter().map(Node::from_element).collect(),
            measure_cache: None,
            arranged_rect: crate::core::Rect::default(),
            // 新节点必然需要一次完整布局。
            dirty: Dirty::Structure,
        }
    }

    /// 类型标签。
    pub fn tag(&self) -> NodeTag {
        self.tag
    }

    /// 稳定 key。
    pub fn key(&self) -> Option<&ElementKey> {
        self.key.as_ref()
    }

    /// 属性表。
    pub fn props(&self) -> &PropertyStore {
        &self.props
    }

    /// 可变属性表。
    ///
    /// 单独暴露而不是只留 [`Node::set_prop`]，是因为主题重解析必须**原地**改写
    /// 已存在的条目：那一轮里需要按"当前是否还是资源键"来筛选，逐个走
    /// `set_prop` 会多算一次脏标记，而资源重解析本身并不该把节点标脏——它只是把
    /// 同一个键的字面值换掉，布局不受影响。
    pub fn props_mut(&mut self) -> &mut PropertyStore {
        &mut self.props
    }

    /// 子节点。
    pub fn children(&self) -> &[Node] {
        &self.children
    }

    /// 脏标记。
    pub fn dirty(&self) -> Dirty {
        self.dirty
    }

    /// 把该子树标脏（向下传播）。
    pub fn mark_dirty(&mut self, level: Dirty) {
        self.dirty = self.dirty.merged(level);
        if level.needs_layout() {
            // 布局变化会让测量缓存失效。
            self.measure_cache = None;
        }
        if level >= Dirty::Structure {
            for child in &mut self.children {
                child.mark_dirty(level);
            }
        }
    }

    /// 本节点已摆放的矩形（相对父节点）。
    pub fn rect(&self) -> crate::core::Rect {
        self.arranged_rect
    }

    /// 设置属性并按影响范围标脏。
    ///
    /// 属性自己声明是否影响布局（见 [`crate::property::PropertyChange::affects_layout`]），
    /// 调用方不必知道每个属性的语义。
    pub fn set_prop(&mut self, key: &PropertyKey, value: PropertyValue) {
        if let Some(change) = self.props.set_local(key, value) {
            self.dirty = self.dirty.merged(if change.affects_layout() {
                Dirty::Layout
            } else {
                Dirty::Paint
            });
            if self.dirty.needs_layout() {
                self.measure_cache = None;
            }
        }
    }
}

/// diff 结果摘要（供测试与诊断）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DiffStats {
    /// 就地复用的节点数。
    pub reused: usize,
    /// 新建的节点数。
    pub inserted: usize,
    /// 丢弃的节点数。
    pub removed: usize,
    /// 移动的节点数。
    pub moved: usize,
    /// 本次 diff **新标脏**的节点数（不含此前就脏的）。
    pub dirtied: usize,
    /// 走到 `Dirty::Layout` 或更高的节点数。
    pub needs_layout: usize,
}

impl Node {
    /// 拆下唯一子节点并返回它。
    ///
    /// 给"只想检查某个已布局节点本身"的调用方用：布局必须从根开始（根决定子项矩形），
    /// 但断言关心的往往只是其中一个子项。
    ///
    /// # 为什么不叫 `only_child`
    ///
    /// 因为它在**子项数量不为 1 时直接 panic**。这不是"取第一个"那种宽容语义——
    /// 那样会让一个写错的测试断言悄悄读到错误的节点，而这里的错误一定来自测试自身。
    pub fn take_only_child(&mut self) -> Node {
        assert_eq!(
            self.children.len(),
            1,
            "take_only_child 要求恰好一个子节点，实得 {}",
            self.children.len()
        );
        self.children.remove(0)
    }
}

impl DiffStats {
    /// 本次 diff 是否**什么都没改变**。
    ///
    /// 稳态（UI 静止）应当为 `true`——这是「不动的时候真的不动」的判据。
    ///
    /// 注意 [`DiffStats::reused`] **不**参与判定：把每个节点都就地复用恰恰是
    /// 期望的稳态行为，而不是「什么都没做」。
    pub fn is_noop(&self) -> bool {
        self.inserted == 0 && self.removed == 0 && self.moved == 0 && self.dirtied == 0
    }

    /// 本次 diff 是否要求重排。
    pub fn needs_layout(&self) -> bool {
        self.needs_layout > 0
    }
}

/// 清除脏标记（重排 + 渲染完成后由宿主调用）。
///
/// **必须有这一步**，否则脏标记只增不减，`needs_layout` 会永久停在 true，
/// 「本帧要不要重排」这个判断就失去意义。真实宿主在 layout+render 走完后调用它。
pub fn clear_dirty(node: &mut Node) {
    node.dirty = Dirty::None;
    node.measure_cache = None;
    for child in &mut node.children {
        clear_dirty(child);
    }
}

/// 以新元素对账旧节点树。
///
/// 返回 diff 统计。`node` 被就地更新；`element` 借用后即失效（声明树本来就是
/// 可丢弃的）。
///
/// 统计里的 `needs_layout` 只计**本次 diff 造成**的重排需求，不计节点此前就
/// 带着的脏标记——否则一个尚未布局的新节点会让每次 diff 都报"需要重排"，
/// 稳态 no-op 的判据就废了。
///
/// 当前实现只做**统计**——真正的就地复用需要先有控件层的元素类型与属性系统
/// 落地。骨架阶段把统计做对，才能在控件加入时验证「稳态零重建」这一核心承诺。
pub fn diff(node: &mut Node, element: &Element) -> DiffStats {
    let mut stats = DiffStats::default();

    if node.tag != element.tag {
        *node = Node::from_element(element);
        stats.inserted = 1;
        stats.needs_layout = 1;
        return stats;
    }
    stats.reused = 1;
    let before = node.dirty;

    // 属性对账：只有值真的变了才标脏，且按属性语义区分重排与重绘。
    //
    // 区分不是优化而是正确性：`Spacing` 变了却只标 Paint，子项位置就不会更新，
    // 而画面上"看起来"只少了一个格子——这类 bug 极难从截图看出来。
    for (id, name, value) in element.props.iter_local() {
        if node.props.set_local_by_id(id, value.clone()) {
            let level = if crate::property::name_affects_layout(name) {
                Dirty::Layout
            } else {
                Dirty::Paint
            };
            node.mark_dirty(level);
        }
    }

    // 子节点对账。
    let old_len = node.children.len();
    let new_len = element.children.len();
    let common = old_len.min(new_len);
    for i in 0..common {
        let child = &mut node.children[i];
        let child_before = child.dirty;
        let child_stats = diff(child, &element.children[i]);
        stats.reused += child_stats.reused;
        stats.inserted += child_stats.inserted;
        stats.removed += child_stats.removed;
        stats.moved += child_stats.moved;
        stats.dirtied += child_stats.dirtied;
        stats.needs_layout += child_stats.needs_layout;
        if child.dirty() != child_before {
            stats.dirtied += 1;
            if child.dirty().needs_layout() {
                stats.needs_layout += 1;
            }
        }
    }
    for i in common..new_len {
        node.children.push(Node::from_element(&element.children[i]));
        stats.inserted += 1;
        stats.needs_layout += 1;
    }
    if new_len < old_len {
        stats.removed += old_len - new_len;
        node.children.truncate(new_len);
        node.dirty = node.dirty.merged(Dirty::Structure);
    }
    // 本节点自身的属性/结构变化也要计入。
    if node.dirty != before {
        stats.dirtied += 1;
        if node.dirty.needs_layout() {
            stats.needs_layout += 1;
        }
    }
    stats
}

/// 布局整棵子树：测量 + 按最终矩形摆放每个子节点。
///
/// 这是 [`render`] 的前置步骤——两者必须成对调用，因为 `render` 只读已摆好的
/// 矩形，不再自己算几何。**几何只有一处算法**（[`crate::layout`]），命中测试与
/// 绘制共用同一批矩形，所以不会出现"看得见但点不到"。
pub fn layout(node: &mut Node, bounds: crate::core::Rect, ctx: Context<'_>) {
    // 用**上界**而非固定约束测量：固定约束的 min == max，会把每个节点都撑成
    // 窗口大小，于是所有 `Width` / `Height` 全部失效（子树内容再大也不缩）。
    // 根节点最终仍由 `arrange` 铺满 `bounds` —— 那是"填满窗口"的正确语义，
    // 但测量阶段必须允许内容小于窗口。
    measure(
        node,
        Constraints {
            min: Size::new(0.0, 0.0),
            max: Size::new(bounds.width, bounds.height),
        },
        ctx,
    );
    arrange(node, bounds, ctx);
}

/// 测量并在必要时重排节点树，返回整棵树的内容尺寸。
///
/// 测量结果按「约束 → 结果」缓存：约束不变则直接命中，这是布局里最重要的一处
/// 短路（静止时每帧都会调它）。
pub fn measure(node: &mut Node, constraints: Constraints, ctx: Context<'_>) -> Size {
    if let Some((cached_constraints, result)) = node.measure_cache {
        if cached_constraints == constraints {
            return result.size;
        }
    }
    let padding = node.padding(ctx);
    let child_constraints = constraints.deflate(padding);

    // 折叠的子节点不参与测量，也不占间距。
    let visible: Vec<usize> = (0..node.children.len())
        .filter(|i| !node.children[*i].is_collapsed(ctx))
        .collect();

    let mut sizes = Vec::with_capacity(visible.len());
    for &i in &visible {
        sizes.push(measure(&mut node.children[i], child_constraints, ctx));
    }

    let stack = node.stack(ctx);
    let content = stack.content_size(&sizes);
    let (own_w, own_h) = node.explicit_size(ctx);
    let result = MeasureResult {
        size: Size::new(
            pick_axis(own_w, content.width, Axis::Horizontal, constraints, padding),
            pick_axis(own_h, content.height, Axis::Vertical, constraints, padding),
        ),
    };
    node.measure_cache = Some((constraints, result));
    node.dirty = node.dirty.merged(Dirty::Layout);
    result.size
}

/// 单轴。约束与内边距都必须按轴取，不能共用。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Axis {
    Horizontal,
    Vertical,
}

impl Axis {
    /// 该轴上的约束区间。
    fn bounds(self, c: Constraints) -> (f32, f32) {
        match self {
            Axis::Horizontal => (c.min.width, c.max.width),
            Axis::Vertical => (c.min.height, c.max.height),
        }
    }

    /// 该轴上的内边距之和。
    fn padding(self, p: crate::core::Thickness) -> f32 {
        match self {
            Axis::Horizontal => p.left + p.right,
            Axis::Vertical => p.top + p.bottom,
        }
    }
}

/// 单轴上「显式尺寸」与「内容尺寸」如何取值。
///
/// 显式尺寸优先；否则内容尺寸加内边距。三步夹逼各有理由：
///
/// - **不低于下限**：父容器明确要的空间必须给足（`Fill` 语义）。
/// - **不超过上限**：内容溢出时应当被裁剪/滚动，而不是把父容器撑大。没有这一步，
///   一个长列表会把窗口顶出去。
/// - **显式尺寸也要夹逼**：父给的区间是硬边界，`Width` 只是"想要"，不是"命令"。
///
/// 上限为无界时跳过中间那步——`natural.min(INFINITY)` 虽然也等于 `natural`，但
/// 写成显式分支能让"无界"这个意图留在代码里，而不是依赖 `f32::INFINITY` 的语义。
fn pick_axis(
    explicit: Option<f32>,
    content: f32,
    axis: Axis,
    constraints: Constraints,
    padding: crate::core::Thickness,
) -> f32 {
    let (min, max) = axis.bounds(constraints);
    let natural = match explicit {
        Some(v) => v,
        None => content + axis.padding(padding),
    };
    let clamped = natural.max(min);
    if max.is_finite() {
        clamped.min(max)
    } else {
        clamped
    }
}

/// 布局与绘制所需的只读上下文。
///
/// # 为什么要传上下文而不是把主题存在节点上
///
/// 主题是**应用状态**，不是节点状态。存在节点上意味着切主题要遍历全树改字段（正是
/// 本框架要避免的）；存进上下文后，切主题只是换一个 `&Theme`——树完全不动。
///
/// 也因此属性里的资源键**保持为键**，直到读取那一刻才解析（见
/// [`crate::theme::resolve_resource`]）。
#[derive(Debug, Clone, Copy)]
pub struct Context<'a> {
    /// 当前主题。
    pub theme: &'a crate::theme::Theme,
}

impl<'a> Context<'a> {
    /// 用给定主题构造。
    pub fn new(theme: &'a crate::theme::Theme) -> Self {
        Self { theme }
    }
}

    /// 进程级共享的内置主题。
///
/// `Context::dark()` 之类的便捷函数需要返回 `&'static Theme`，而主题是纯数据
/// （一堆 `Color`），构造一次就该全程复用。这里用 `OnceLock` 惰性建好并**只借不借出**
/// 所有权，于是既不会泄漏内存，也不会有人拿到 `&mut` 把它改坏。
static DARK_THEME: std::sync::OnceLock<crate::theme::Theme> = std::sync::OnceLock::new();
static LIGHT_THEME: std::sync::OnceLock<crate::theme::Theme> = std::sync::OnceLock::new();

/// 深色主题的上下文。
///
/// 共享同一个 `Theme` 实例：色板不可变，多份副本只会让人误以为改一份能影响另一份。
pub fn dark_context() -> Context<'static> {
    Context {
        theme: DARK_THEME.get_or_init(crate::theme::Theme::dark),
    }
}

/// 浅色主题的上下文。
pub fn light_context() -> Context<'static> {
    Context {
        theme: LIGHT_THEME.get_or_init(crate::theme::Theme::light),
    }
}

/// 按最终矩形摆放整棵子树。
///
/// 顺序无关紧要：绘制顺序由 [`render`] 的 `order` 决定，这里只算几何。
pub fn arrange(node: &mut Node, bounds: crate::core::Rect, ctx: Context<'_>) {
    node.arranged_rect = bounds;
    let padding = node.padding(ctx);
    let inner = bounds.inset(padding);
    let stack = node.stack(ctx);
    let cross_align = node.cross_align(ctx);

    let visible: Vec<usize> = (0..node.children.len())
        .filter(|i| !node.children[*i].is_collapsed(ctx))
        .collect();

    // 用**测量缓存**里的尺寸，而不是重算——measure 刚跑过，缓存必然命中。
    let sizes: Vec<Size> = visible
        .iter()
        .map(|&i| {
            node.children[i]
                .measure_cache
                .map(|(_, r)| r.size)
                .unwrap_or(Size::new(0.0, 0.0))
        })
        .collect();

    let origin = crate::core::Point::new(inner.x, inner.y);
    let rects = stack.arrange(origin, Size::new(inner.width, inner.height), &sizes, cross_align);

    for (slot, &i) in visible.iter().enumerate() {
        arrange(&mut node.children[i], rects[slot], ctx);
    }
}

/// 布局 + 降级：把一棵已布局的节点树变成 [`DisplayList`]。
///
/// 这是核心对外的**唯一**产出。宿主拿到它之后就不再关心树的存在。
///
/// 调用前必须先 [`layout`]——本函数只读已摆好的矩形。
pub fn render(node: &Node, ctx: Context<'_>, out: &mut DisplayList) {
    out.clear();
    let mut order = 0u32;
    render_rec(node, ctx, &mut order, out);
}

fn render_rec(node: &Node, ctx: Context<'_>, order: &mut u32, out: &mut DisplayList) {
    use crate::paint::{Command, SortKey};

    if node.is_collapsed(ctx) {
        return;
    }
    let rect = node.rect();
    let opacity = node.opacity(ctx);
    let layer = node.layer(ctx);

    // 自身外观：背景 + 描边合成到一条 Quad（描边复用同一条命令的 stroke 字段，
    // 于是圆角矩形永远是一个 draw call —— 这正是 paint.rs 选 Quad 图元的原因）。
    let fill = node.background(ctx);
    let stroke_color = node.border_color(ctx);
    let stroke_width = node.border_width(ctx);
    let radius = node.corner_radius(ctx);
    let needs_quad = fill.a > 0.0 || (stroke_color.a > 0.0 && stroke_width > 0.0);
    if needs_quad {
        out.push(
            SortKey {
                layer,
                z: 0,
                order: next_order(order),
            },
            Command::Quad {
                rect,
                radius: crate::core::CornerRadius { radius },
                fill,
                stroke: (stroke_color.a > 0.0 && stroke_width > 0.0)
                    .then_some((stroke_color, stroke_width)),
            },
        );
    }

    // 图像内容
    if let Some(texture) = node.image_texture(ctx) {
        let size = node.image_size(ctx);
        let dst = if size > 0.0 {
            // 指定边长：在节点矩形内居中。
            let dx = rect.x + (rect.width - size) * 0.5;
            let dy = rect.y + (rect.height - size) * 0.5;
            crate::core::Rect::new(dx, dy, size, size)
        } else {
            rect
        };
        out.push(
            SortKey {
                layer,
                z: 0,
                order: next_order(order),
            },
            Command::Image {
                texture,
                src: None,
                dst,
                tint: node.image_tint(ctx).with_alpha(opacity),
            },
        );
    }

    // 子节点
    for child in node.children() {
        render_rec(child, ctx, order, out);
    }
}

fn next_order(order: &mut u32) -> u32 {
    let o = *order;
    *order += 1;
    o
}

impl Node {
    /// 是否被折叠（不占位、不绘制）。
    pub fn is_collapsed(&self, ctx: Context<'_>) -> bool {
        self.flag(&crate::props::COLLAPSED, ctx)
    }

    /// 内边距（四边统一值）。
    pub fn padding(&self, ctx: Context<'_>) -> crate::core::Thickness {
        crate::core::Thickness::uniform(self.number(&crate::props::PADDING, ctx))
    }

    /// 显式尺寸（`None` = 由内容决定）。
    fn explicit_size(&self, ctx: Context<'_>) -> (Option<f32>, Option<f32>) {
        (
            finite(self.number_or(&crate::props::WIDTH, f32::NAN, ctx)),
            finite(self.number_or(&crate::props::HEIGHT, f32::NAN, ctx)),
        )
    }

    /// 图像纹理（`None` = 无图像内容）。
    ///
    /// 接受 `StaticStr`（编译期常量）与 `Str`（运行期字符串）两种来源——属性定义
    /// 的默认值只能是前者，但真实纹理由宿主在运行时填入。
    pub fn image_texture(&self, ctx: Context<'_>) -> Option<crate::paint::TextureRef> {
        match self.value(&crate::props::IMAGE_TEXTURE, ctx).as_deref()? {
            crate::property::PropertyValue::StaticStr(s) if !s.is_empty() => {
                Some(crate::paint::TextureRef(std::borrow::Cow::Borrowed(s)))
            }
            crate::property::PropertyValue::Str(s) if !s.is_empty() => Some(
                crate::paint::TextureRef(std::borrow::Cow::Owned(s.as_ref().to_owned())),
            ),
            _ => None,
        }
    }

    /// 堆叠描述。
    pub fn stack(&self, ctx: Context<'_>) -> crate::layout::Stack {
        crate::layout::Stack {
            direction: match self.value(&crate::props::DIRECTION, ctx).as_deref() {
                Some(crate::property::PropertyValue::Direction(d)) => *d,
                _ => crate::layout::Direction::Horizontal,
            },
            spacing: self.number(&crate::props::SPACING, ctx),
            main_align: match self.value(&crate::props::MAIN_ALIGN, ctx).as_deref() {
                Some(crate::property::PropertyValue::MainAlign(a)) => *a,
                _ => crate::layout::MainAxisAlign::Start,
            },
        }
    }

    /// 交叉轴对齐。
    pub fn cross_align(&self, ctx: Context<'_>) -> crate::layout::AlignItems {
        match self.value(&crate::props::CROSS_ALIGN, ctx).as_deref() {
            Some(crate::property::PropertyValue::CrossAlign(a)) => *a,
            _ => crate::layout::AlignItems::Start,
        }
    }

    /// 背景色。
    pub fn background(&self, ctx: Context<'_>) -> crate::core::Color {
        self.color(&crate::props::BACKGROUND, ctx)
            .unwrap_or(crate::core::Color::rgba(0.0, 0.0, 0.0, 0.0))
    }

    /// 圆角半径。
    pub fn corner_radius(&self, ctx: Context<'_>) -> f32 {
        self.number(&crate::props::CORNER_RADIUS, ctx)
    }

    /// 描边色。
    pub fn border_color(&self, ctx: Context<'_>) -> crate::core::Color {
        self.color(&crate::props::BORDER_COLOR, ctx)
            .unwrap_or(crate::core::Color::rgba(0.0, 0.0, 0.0, 0.0))
    }

    /// 描边宽度。
    pub fn border_width(&self, ctx: Context<'_>) -> f32 {
        self.number(&crate::props::BORDER_WIDTH, ctx)
    }

    /// 整体不透明度（继承）。
    pub fn opacity(&self, ctx: Context<'_>) -> f32 {
        self.number_or(&crate::props::OPACITY, 1.0, ctx)
    }

    /// 合成层。
    pub fn layer(&self, ctx: Context<'_>) -> crate::paint::Layer {
        match self.value(&crate::props::LAYER, ctx).as_deref() {
            Some(crate::property::PropertyValue::Layer(l)) => *l,
            _ => crate::paint::Layer::Content,
        }
    }

    /// 图像边长（`-1` = 撑满节点矩形）。
    pub fn image_size(&self, ctx: Context<'_>) -> f32 {
        self.number(&crate::props::IMAGE_SIZE, ctx)
    }

    /// 图像着色。
    pub fn image_tint(&self, ctx: Context<'_>) -> crate::core::Color {
        self.color(&crate::props::IMAGE_TINT, ctx)
            .unwrap_or(crate::core::Color::rgb(1.0, 1.0, 1.0))
    }

    /// 是否可交互。
    pub fn is_interactive(&self, ctx: Context<'_>) -> bool {
        self.flag(&crate::props::INTERACTIVE, ctx)
    }

    /// 是否处于选中态。
    pub fn is_selected(&self, ctx: Context<'_>) -> bool {
        self.flag(&crate::props::SELECTED, ctx)
    }

    /// 是否禁用。
    pub fn is_disabled(&self, ctx: Context<'_>) -> bool {
        self.flag(&crate::props::DISABLED, ctx)
    }

    /// 读一个颜色属性，解析其中的资源键。
    fn color(&self, key: &crate::property::PropertyKey, ctx: Context<'_>) -> Option<crate::core::Color> {
        self.value(key, ctx).and_then(|v| v.get::<crate::core::Color>())
    }

    /// 读一个属性值（本地→样式→继承→默认），并按当前主题解析其中的资源键。
    ///
    /// 返回 `Cow`：非资源值直接借用（零分配），资源键才现场解析。资源键解析失败
    /// （角色名拼错）时返回 `None` ——调用方据此决定降级，而不是拿到一个黑色。
    pub fn value<'a>(
        &'a self,
        key: &'a crate::property::PropertyKey,
        ctx: Context<'_>,
    ) -> Option<std::borrow::Cow<'a, crate::property::PropertyValue>> {
        let (raw, _source) = crate::property::resolve(&self.props, key, None);
        match crate::theme::resolve_resource(ctx.theme, raw) {
            Some(resolved) => Some(resolved),
            None if raw.is_resource() => None,
            None => Some(std::borrow::Cow::Borrowed(raw)),
        }
    }

    /// 读一个数值属性，缺省 0。
    fn number(&self, key: &crate::property::PropertyKey, ctx: Context<'_>) -> f32 {
        self.number_or(key, 0.0, ctx)
    }

    /// 读一个数值属性，带缺省。
    ///
    /// 过滤非有限值：属性默认值用 `NaN` 表示"未设置"，若直接透传会让
    /// `NaN` 混进几何计算并污染整条布局链（下游全变 `NaN`，且不会报错）。
    fn number_or(
        &self,
        key: &crate::property::PropertyKey,
        fallback: f32,
        ctx: Context<'_>,
    ) -> f32 {
        self.value(key, ctx)
            .and_then(|v| v.get::<f32>())
            .filter(|v| v.is_finite())
            .unwrap_or(fallback)
    }

    /// 读一个布尔属性。
    fn flag(&self, key: &crate::property::PropertyKey, ctx: Context<'_>) -> bool {
        self.value(key, ctx)
            .and_then(|v| v.get::<bool>())
            .unwrap_or(false)
    }
}

/// `NaN` 表示"未设置"，转成 `None`。
fn finite(v: f32) -> Option<f32> {
    (v.is_finite() && v >= 0.0).then_some(v)
}


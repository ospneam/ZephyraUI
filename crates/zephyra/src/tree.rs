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

    // 属性对账：只有值真的变了才标脏。这是「UI 静止时零重建」的第一道闸门。
    for (id, value) in element.props.iter_local() {
        if node.props.set_local_by_id(id, value.clone()) {
            node.mark_dirty(Dirty::Paint);
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

/// 布局 + 降级：把一棵已布局的节点树变成 [`DisplayList`]。
///
/// 这是核心对外的**唯一**产出。宿主拿到它之后就不再关心树的存在。
pub fn render(node: &Node, bounds: crate::core::Rect, out: &mut DisplayList) {
    out.clear();
    let mut order = 0u32;
    render_rec(node, bounds, crate::paint::Layer::Content, &mut order, out);
}

fn render_rec(
    node: &Node,
    bounds: crate::core::Rect,
    layer: crate::paint::Layer,
    order: &mut u32,
    out: &mut DisplayList,
) {
    let key = crate::paint::SortKey {
        layer,
        z: 0,
        order: {
            let o = *order;
            *order += 1;
            o
        },
    };
    match node.tag() {
        NodeTag::Container | NodeTag::Control | NodeTag::Text | NodeTag::Image => {
            // 控件/容器：先画自己的背景（若有），再递归子节点。
            out.push(
                key,
                crate::paint::Command::Quad {
                    rect: bounds,
                    radius: crate::core::CornerRadius { radius: 4.0 },
                    fill: crate::theme::Theme::dark().surface,
                    stroke: None,
                },
            );
        }
    }
    for child in node.children() {
        // 子节点暂按整块父区域处理；布局落地后改为各自的 arranged_rect。
        render_rec(child, bounds, layer, order, out);
    }
}

/// 测量并在必要时重排节点树，返回整棵树的内容尺寸。
///
/// 测量结果按「约束 → 结果」缓存：约束不变则直接命中，这是布局里最重要的一处
/// 短路（静止时每帧都会调它）。
pub fn measure(node: &mut Node, constraints: Constraints) -> Size {
    if let Some((cached_constraints, result)) = node.measure_cache {
        if cached_constraints == constraints {
            return result.size;
        }
    }
    let mut content = Size::new(0.0, 0.0);
    let child_constraints = constraints.deflate(node.padding());
    for child in &mut node.children {
        let size = measure(child, child_constraints);
        content.width = content.width.max(size.width);
        content.height = content.height.max(size.height);
    }
    let own = node.measure_impl(constraints);
    let result = MeasureResult {
        size: Size::new(
            own.size.width.max(content.width),
            own.size.height.max(content.height),
        ),
    };
    node.measure_cache = Some((constraints, result));
    node.dirty = node.dirty.merged(Dirty::Layout);
    result.size
}

impl Node {
    /// 本节点自身（不含子节点）想要的尺寸。
    ///
    /// 骨架阶段的默认实现：没有子节点时取约束下限（不主动撑开），有子节点时
    /// 由 [`measure`] 取子节点最大值。控件层落地后由各控件覆写。
    fn measure_impl(&mut self, constraints: Constraints) -> MeasureResult {
        let _ = self;
        MeasureResult {
            size: constraints.min,
        }
    }

    /// 本节点的内边距（供 [`measure`] 收缩可用空间）。
    fn padding(&self) -> crate::core::Thickness {
        /// 内边距四边统一值（骨架阶段不区分方向）。
        const PADDING: &crate::property::PropertyKey = &crate::property::PropertyKey::inheritable(
            "Padding",
            crate::property::PropertyValue::float(0.0),
        );
        let (value, _) = crate::property::resolve(&self.props, PADDING, None);
        crate::core::Thickness::uniform(value.get::<f32>().unwrap_or(0.0))
    }
}

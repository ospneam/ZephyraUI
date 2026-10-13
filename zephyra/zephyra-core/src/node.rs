//! 保留：一棵 [`Node`] 树持有跨帧存活的状态。
//!
//! # 脏标记的粒度
//!
//! [`Dirty`] 至少要分四级，因为它们触发的工作量差一个数量级：
//!
//! | 等级 | 触发 | 代价 |
//! |---|---|---|
//! | [`Dirty::None`] | — | 零 |
//! | [`Dirty::Paint`] | 颜色变了 | 重生成该子树的 DisplayList 段 |
//! | [`Dirty::Layout`] | 尺寸变了 | 重排该子树 |
//! | [`Dirty::Structure`] | 子节点增删 | 重建该子树 + 重排 |
//!
//! 把「背景色变了」当成 `Layout` 处理，是 UI 卡顿最常见的单一原因。

use crate::core::Rect;
use crate::id::{ElementKey, NodeTag};
use crate::layout::{Constraints, MeasureResult};
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

/// 保留节点：树上每个元素对应的持久状态。
pub struct Node {
    tag: NodeTag,
    key: Option<ElementKey>,
    /// 样式分类名（与 [`crate::element::Element::style`] 对应，每帧由 diff 对账）。
    pub(crate) styles: Vec<&'static str>,
    props: PropertyStore,
    children: Vec<Node>,
    /// 测量结果缓存（`Dirty >= Layout` 时失效）。
    ///
    /// `pub(crate)`：写入方是 [`crate::layout`] 的测量趟，读取方是排列趟（它复用
    /// 刚测出的尺寸而不重算）。对外只暴露 [`Node::cached_size`]，因为外部拿到
    /// `MeasureResult` 也用不上——那会诱使调用方绕过 `pick_axis` 直接改矩形。
    pub(crate) measure_cache: Option<(Constraints, MeasureResult)>,
    /// 本节点已摆放的矩形（相对父节点）。`pub(crate)`：只由排列趟写入。
    pub(crate) arranged_rect: Rect,
    /// 当前脏标记。`pub(crate)`：只由 [`crate::diff`] 直接合并（见
    /// [`Node::merge_dirty`]）。
    pub(crate) dirty: Dirty,
}

impl Node {
    /// 由元素构建节点（首次进入树时）。
    pub fn from_element(element: &crate::element::Element) -> Node {
        Node {
            tag: element.tag(),
            key: element.key().cloned(),
            styles: element.style_names().to_vec(),
            props: element.props().clone(),
            children: element.children().iter().map(Node::from_element).collect(),
            measure_cache: None,
            arranged_rect: Rect::default(),
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

    /// 样式分类名。
    pub fn style_names(&self) -> &[&'static str] {
        &self.styles
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

    /// 可变子节点。
    ///
    /// 给宿主在布局后调整已存在的子树（改样式、折叠某项）。注意这绕过 diff——
    /// 直接改动不会留下脏标记，所以调用方需要自己 [`Node::mark_dirty`]。
    pub fn children_mut(&mut self) -> &mut Vec<Node> {
        &mut self.children
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
    pub fn rect(&self) -> Rect {
        self.arranged_rect
    }

    /// 设置属性并按影响范围标脏。
    ///
    /// 属性自己声明是否影响布局（见 [`crate::property::PropertyChange::affects_layout`]），
    /// 调用方不必知道每个属性的语义。
    pub fn set_prop(&mut self, key: &'static PropertyKey, value: PropertyValue) {
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

    /// 设置**样式表**给的属性值并按影响范围标脏。
    ///
    /// 写在 `Styled` 层（低于 `Local`），所以应用代码的 `.with(...)` 仍然覆盖它。
    ///
    /// ⚠️ **为什么必须走这个方法而不能 `props_mut()` 直接改**：那样不产生脏标记，
    /// 于是「改了样式但画面不动」——而样式表正是每帧都在改的那一层。这个洞 B2 才补：
    /// 之前 `set_styled` 虽返回 `PropertyChange`，但 `Node` 上没有对应入口，
    /// 唯一的路是 `props_mut()`，而它**绕过了标脏**。
    ///
    /// 与 [`Self::set_prop`] 的差别只有一层：写 `Styled` 而不是 `Local`。
    /// 标脏逻辑**必须完全一致**——两边走偏的症状是
    /// 「同一种属性，源码里改会重排、样式表里改不会」。
    ///
    /// ⚠️ **「值没变就不标脏」这条在这个路径上尤其重要**：`apply` 每帧都会把同一条
    /// 规则再写一遍。若写同值也算变化，静止界面会永远在重绘——这是
    /// 「样式表导致 UI 永远不静止」这类 bug 的直接成因。
    pub fn set_styled_prop(&mut self, key: &'static PropertyKey, value: PropertyValue) {
        if self.props.set_styled_if_changed(key, value) {
            self.dirty = self.dirty.merged(if key.affects_layout() {
                Dirty::Layout
            } else {
                Dirty::Paint
            });
            if self.dirty.needs_layout() {
                self.measure_cache = None;
            }
        }
    }

    /// 拆掉**本轮没人再声明**的样式值，按影响范围标脏。
    ///
    /// [`Self::set_styled_prop`] 是「写」，本方法是「拆」—— 两者缺一不可，
    /// 少了拆的那一半，状态类消失后旧值会永久赖在节点上（症状：点一下取消勾选，
    /// 底色还是选中色；见 [`crate::property::PropertyStore::retain_styled`]）。
    ///
    /// 返回被拆下的键，供测试与诊断用。**只有真的拆下了东西才标脏** ——
    /// `apply` 每帧都跑一遍，写同值不算变化（见 [`Self::set_styled_prop`]），
    /// 拆不存在的东西也必须同样安静，否则静止界面会永远在重绘。
    pub fn retain_styled_props(&mut self, keep: impl Fn(&'static PropertyKey) -> bool) -> Vec<&'static PropertyKey> {
        let removed = self.props.retain_styled(keep);
        for key in &removed {
            self.dirty = self.dirty.merged(if key.affects_layout() {
                Dirty::Layout
            } else {
                Dirty::Paint
            });
        }
        if !removed.is_empty() && self.dirty.needs_layout() {
            self.measure_cache = None;
        }
        removed
    }

    /// 拆掉**本帧声明里没有**的本地属性，返回被拆下的键。
    ///
    /// 声明树是**真值**：`.with(...)` 这一帧没写的东西，就不该留在节点上。
    /// 与 [`Self::retain_styled_props`] 同源（只写不清 → 残值静默留存），区别是
    /// 这一层来自源码而不是样式表。
    ///
    /// 症状举例：控件写 `if closable { } else { .with(COLLAPSED, true) }`，
    /// 宿主把 `closable` 翻回 true 之后 `COLLAPSED` 仍赖着 —— 元素永远折叠着。
    pub fn retain_local_props(
        &mut self,
        keep: impl Fn(&'static PropertyKey) -> bool,
    ) -> Vec<&'static PropertyKey> {
        let removed = self.props.retain_local(keep);
        for key in &removed {
            self.dirty = self.dirty.merged(if key.affects_layout() {
                Dirty::Layout
            } else {
                Dirty::Paint
            });
        }
        if !removed.is_empty() && self.dirty.needs_layout() {
            self.measure_cache = None;
        }
        removed
    }

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

    /// 合并一个脏标记，**不**向子节点传播。返回是否真的变脏了。
    ///
    /// 专供 [`crate::diff`] 用。它已经**递归地对每个子节点各调一次**
    /// [`Node::mark_dirty`]，所以这里必须只改自己 —— 否则会把这次变化错误地盖到
    /// 那些其实没变的兄弟节点上（症状是"改一行文字，整棵列表都重排"）。
    pub(crate) fn merge_dirty(&mut self, level: Dirty) -> bool {
        let before = self.dirty;
        self.dirty = self.dirty.merged(level);
        self.dirty != before
    }

    /// 测量缓存里的尺寸（`None` = 没测过或已失效）。
    ///
    /// 排列趟用它复用刚测出的结果，而不是重跑一遍测量。
/// 测量趟存下来的**排版结果**（`None` = 没测过 / 本节点无文本 / 缓存已失效）。
    ///
    /// 绘制趟用它避免每帧重排一次文字 —— 见
    /// [`crate::paint::text_command`] 里那段实测数字。
    ///
    /// 借出引用而不是克隆：绘制趟只读，而 `ShapedText` 里是 `Vec<Line>`，
    /// 克隆一次是一整棵树的分配。
    pub(crate) fn cached_shaped(&self) -> Option<&crate::text::ShapedText> {
        self.measure_cache.as_ref().and_then(|(_, r)| r.shaped.as_ref())
    }

    /// 测量缓存里的尺寸（`None` = 没测过或已失效）。
    pub(crate) fn cached_size(&self) -> Option<crate::core::Size> {
        self.measure_cache.as_ref().map(|(_, r)| r.size)
    }

    /// 写入测量缓存。
    pub(crate) fn cache_measure(&mut self, constraints: Constraints, result: MeasureResult) {
        self.measure_cache = Some((constraints, result));
    }

    /// 写入已摆放的矩形。只应由 [`crate::layout`] 的排列趟调用。
    pub(crate) fn set_rect(&mut self, rect: Rect) {
        self.arranged_rect = rect;
    }
}

/// 清除脏标记（重排 + 渲染完成后由宿主调用）。
///
/// **必须有这一步**，否则脏标记只增不减，`needs_layout` 会永久停在 true，
/// 「本帧要不要重排」这个判断就失去意义。真实宿主在 layout+render 走完后调用它。
///
/// # ⚠ 这里**故意不清** `measure_cache`
///
/// 本函数早先会递归清掉测量缓存，那是**错的**，而且错得很贵：
///
/// - 官方把缓存放在 `CLayoutStorage::m_previousAvailableSize`
///   （`uielement.cpp:4223-4226`），它是**跨帧存活**的 —— 静止时每一帧
///   `Measure()` 都会拿它做一次精确比较然后整个子树跳过测量；
/// - 我们每帧调一次 `clear_dirty`，于是缓存每帧被清 ⇒ **每一帧都重测整棵树**、
///   重新整形所有文字。那是布局里最贵的一步，而它本来一次都不该发生。
///
/// 缓存的失效时机只有一处：**某个布局相关属性被改**。那已经由
/// [`Node::mark_dirty`] / [`Node::set_prop`] / [`Node::set_styled_prop`] /
/// `retain_*` 各自负责（它们都在 `needs_layout()` 时清缓存）。
/// 换句话说，"脏标记清空"与"测量结果过期"是两件事 ——
/// 前者每帧都发生，后者只在属性变化时发生。
///
/// 门禁：`zephyra-core/tests/layout_cache_tests.rs::稳态完全不重测`
/// （改之前它是红的，每帧 6 次整形）。
pub fn clear_dirty(node: &mut Node) {
    node.dirty = Dirty::None;
    for child in node.children_mut() {
        clear_dirty(child);
    }
}
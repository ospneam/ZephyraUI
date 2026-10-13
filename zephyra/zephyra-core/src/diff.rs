//! 对账：拿新 [`crate::element::Element`] 树与旧 [`Node`] 树配对。
//!
//! # 按身份（key）配对，其次按位置
//!
//! diff 的第一步是把新旧子节点配对。规则：
//!
//! 1. **有 key 的按 key 配对** —— key 就是组件实例的身份（见
//!    [`crate::element::Element::with_key`] 的约定）；
//! 2. 无 key 的按位置配对；
//! 3. 同位置（或同 key）但**类型不同** → 视为替换（丢弃旧 Node，建新的）；
//! 4. 配对上的节点逐层对账属性；没被认领的旧节点一律丢弃。
//!
//! 规则 1 是「列表里插一项不该让后面全部重建状态」的关键。
//!
//! ⚠️ **key 不同 = 身份不同 = 不能复用**。纯按位置配对隐含一条约定
//! 「同一位置永远是同一类节点」，而换页就违反它：`page` 的子节点从 `home`
//! 变成 `basic-input`，两者都是 `Container`，于是旧页面被就地改造成新页面，
//! 改不掉的旧节点**留在树上成为幽灵**（带着自己的样式与矩形继续绘制）。
//! 症状是「点了导航之后界面花掉」，且对 diff 而言每一步都合法 —— 无报错、无告警。

use crate::element::Element;
use crate::node::{Dirty, Node};
use crate::property::PropertyKey;

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

/// 以新元素对账旧节点树。
///
/// 返回 diff 统计。`node` 被就地更新；`element` 借用后即失效（声明树本来就是
/// 可丢弃的）。
///
/// 统计里的 `needs_layout` 只计**本次 diff 造成**的重排需求，不计节点此前就
/// 带着的脏标记——否则一个尚未布局的新节点会让每次 diff 都报"需要重排"，
/// 稳态 no-op 的判据就废了。
pub fn diff(node: &mut Node, element: &Element) -> DiffStats {
    let mut stats = DiffStats::default();

    if node.tag() != element.tag() {
        *node = Node::from_element(element);
        stats.inserted = 1;
        stats.needs_layout = 1;
        return stats;
    }
    stats.reused = 1;
    let before = node.dirty();

    // 属性对账：只有值真的变了才标脏，且按属性语义区分重排与重绘。
    //
    // 区分不是优化而是正确性：`Spacing` 变了却只标 Paint，子项位置就不会更新，
    // 而画面上"看起来"只少了一个格子——这类 bug 极难从截图看出来。
    let mut local_keys: Vec<&'static PropertyKey> = Vec::new();
    for (key, value) in element.props().iter_local() {
        local_keys.push(key);
        if node.props_mut().set_local_if_changed(key, value.clone()) {
            let level = if key.affects_layout() {
                Dirty::Layout
            } else {
                Dirty::Paint
            };
            node.mark_dirty(level);
        }
    }
    // 拆掉「上一帧声明过、这一帧不再声明」的本地属性。
    //
    // ⚠️ 声明树是**真值**：`.with(...)` 这一帧没写的东西，就不该留在节点上。
    // 只写不拆的症状很具体：`if closable { … } else { with(COLLAPSED) }` 这种
    // 写法一旦从 true 翻回 false，`COLLAPSED=true` 会**永远**赖着（元素被折叠、
    // 不占位、不绘制），而声明树里明明已经没有了 —— 于是「关掉之后再打开，
    // 它不见了」。
    // 键的身份是**地址**（`PropertyKey` 没有 `PartialEq`，见 property.rs），
    // 所以比对一律用 `ptr::eq` —— 与本文件其余处一致。
    for key in node.retain_local_props(|k| local_keys.iter().any(|x| std::ptr::eq(*x, k))) {
        node.mark_dirty(if key.affects_layout() {
            Dirty::Layout
        } else {
            Dirty::Paint
        });
    }

    // 样式分类名对账。**必须整表替换而不是追加**：声明树每帧重建，所以
    // 「这帧有、上帧没有」是常态。只追加不删除的话，去掉一个分类名后它
    // **永远留在节点上**——症状是「样式表改了不生效」，而这正是本轨道
    // 立规之前最担心的一类 bug。
    let want: Vec<&'static str> = element.style_names().to_vec();
    if node.style_names() != want.as_slice() {
        node.styles = want;
        // 分类名变不变都可能改变外观，但不影响布局（外观在 Styled 层）。
        // 标 Paint：下一帧 apply 会把新分类的样式写进来，然后按值的
        // `affects_layout` 决定要不要升级成 Layout。
        node.mark_dirty(Dirty::Paint);
    }

    // 子节点对账。
    //
    // **身份优先：有 key 的按 key 配对，无 key 的按位置。**
    //
    // # 为什么不能只按位置
    //
    // 按位置配对隐含一条约定：「同一位置永远是同一类节点」。**换页就违反它** ——
    // `page` 的子节点从 `home` 变成 `basic-input`，两者 tag 都是 `Container`，
    // 于是位置配对把**旧页面的整棵树**就地改造成新页面：能改的属性改掉了，
    // 改不掉的（旧节点更多时多出来的那些）**留在树上成为幽灵节点**，
    // 带着自己的样式值、自己的矩形继续绘制。
    //
    // 症状是「点了导航之后界面花掉」，而且**不报错、不告警**：对 diff 来说
    // 每一步都是合法的 —— 它确实按声明树把每一处都"对上了"。
    //
    // # 判据
    //
    // key 不同 = 身份不同 = 不能复用，整棵替换。这是保留式布局唯一站得住的判据：
    // 组件实例的身份由 key 给出（同 key 同身份），于是「复用」永远是安全的。
    let mut pool: Vec<Option<Node>> = node.children_mut().drain(..).map(Some).collect();
    let mut out: Vec<Node> = Vec::with_capacity(element.children().len());
    let mut moved = 0usize;
    // 本层新插入 / 替换掉的子节点数（供下面"子树变了 ⇒ 本节点也要重排"用）。
    let mut inserted_here = 0usize;
    // 递归收集到的"后代需要重排"计数。自底向上归并，用来把失效沿父链传上去。
    let mut child_needs_layout = 0usize;
    for (i, el) in element.children().iter().enumerate() {
        // 先找同 key 的旧节点；同位置优先（身份与位置一致是绝大多数情况），
        // 否则全表扫 —— 后者让「列表里插一项」只动被插处，而不必整段替换。
        let slot = match el.key() {
            Some(k) => {
                let same_pos = pool
                    .get(i)
                    .and_then(|c| c.as_ref())
                    .filter(|c| c.key() == Some(k))
                    .map(|_| i);
                same_pos.or_else(|| {
                    pool.iter()
                        .position(|c| c.as_ref().and_then(|n| n.key()) == Some(k))
                })
            }
            // 无 key：按位置拿第一个还没被认领的（保持旧行为）。
            None => pool.iter().position(|c| c.is_some()),
        };
        match slot.and_then(|j| pool[j].take()) {
            Some(mut child) if child.tag() == el.tag() => {
                if slot != Some(i) {
                    moved += 1;
                }
                let child_before = child.dirty();
                let child_stats = diff(&mut child, el);
                stats.reused += child_stats.reused;
                stats.inserted += child_stats.inserted;
                stats.removed += child_stats.removed;
                stats.moved += child_stats.moved;
                stats.dirtied += child_stats.dirtied;
                stats.needs_layout += child_stats.needs_layout;
                child_needs_layout += child_stats.needs_layout;
                if child.dirty() != child_before {
                    stats.dirtied += 1;
                    if child.dirty().needs_layout() {
                        stats.needs_layout += 1;
                    }
                }
                out.push(child);
            }
            Some(_old) => {
                // 同 key 但 tag 变了（或没有 key 而 tag 变了）：旧节点整个丢弃。
                out.push(Node::from_element(el));
                stats.inserted += 1;
                stats.removed += 1;
                stats.needs_layout += 1;
                inserted_here += 1;
            }
            None => {
                out.push(Node::from_element(el));
                stats.inserted += 1;
                stats.needs_layout += 1;
                inserted_here += 1;
            }
        }
    }
    let dropped = pool.iter().filter(|c| c.is_some()).count();
    if dropped > 0 {
        stats.removed += dropped;
        inserted_here += dropped;
    }
    node.children_mut().append(&mut out);
    if dropped > 0 || moved > 0 {
        stats.moved += moved;
        // `merge_dirty` 而不是 `mark_dirty`：子节点已各自递归对账过了，这里只
        // 标自己。用 `mark_dirty` 会把 Structure 传播到所有剩余子节点。
        node.merge_dirty(Dirty::Structure);
    }
    // 子树变了 ⇒ **本节点也必须重排**（并且丢掉自己的测量缓存）。
    //
    // ⚠ 早先这里不标，于是「插入一个新子节点」之后父节点的 `measure_cache`
    // 仍然有效 ⇒ 下一趟 `measure(父)` 命中缓存、**根本不下探**，新子节点
    // 因此从来没有被测量过 ⇒ `arrange` 里 `cached_size()` 返回 `None`
    // ⇒ `rects[slot]` 越界 panic。
    //
    // 它一直是被另一个缺陷**盖住**的：`clear_dirty` 每帧清空测量缓存，
    // 父节点必然重新下探。清掉那行之后这条就暴露了。
    //
    // 官方由集合自己负责（`UIElementCollection.cpp:451` 的
    // `pOwner->InvalidateMeasure()`，Insert / RemoveAt / 重排 同理），
    // 并用 `LF_ON_MEASURE_DIRTY_PATH` 把"后代脏"沿父链传播。
    // 这里是自底向上的一次归并，效果等价且不需要额外的位域。
    if inserted_here > 0 || child_needs_layout > 0 {
        node.mark_dirty(Dirty::Layout);
    }
    // 本节点自身的属性/结构变化也要计入。
    if node.dirty() != before {
        stats.dirtied += 1;
        if node.dirty().needs_layout() {
            stats.needs_layout += 1;
        }
    }
    stats
}

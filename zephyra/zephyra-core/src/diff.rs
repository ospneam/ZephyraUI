//! 对账：拿新 [`crate::element::Element`] 树与旧 [`Node`] 树配对。
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

use crate::element::Element;
use crate::node::{Dirty, Node};

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
    for (key, value) in element.props().iter_local() {
        if node.props_mut().set_local_if_changed(key, value.clone()) {
            let level = if key.affects_layout() {
                Dirty::Layout
            } else {
                Dirty::Paint
            };
            node.mark_dirty(level);
        }
    }

    // 子节点对账。
    let old_len = node.children().len();
    let new_len = element.children().len();
    let common = old_len.min(new_len);
    for i in 0..common {
        let child = &mut node.children_mut()[i];
        let child_before = child.dirty();
        let child_stats = diff(child, &element.children()[i]);
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
        node.children_mut()
            .push(Node::from_element(&element.children()[i]));
        stats.inserted += 1;
        stats.needs_layout += 1;
    }
    if new_len < old_len {
        stats.removed += old_len - new_len;
        node.children_mut().truncate(new_len);
        // `merge_dirty` 而不是 `mark_dirty`：子节点已各自递归对账过了，这里只
        // 标自己。用 `mark_dirty` 会把 Structure 传播到所有剩余子节点。
        node.merge_dirty(Dirty::Structure);
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

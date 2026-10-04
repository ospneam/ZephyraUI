//! 命中测试：把指针/触摸位置解析成「落在哪个控件上」。
//!
//! # 与布局共用几何，而不是各算一套
//!
//! 命中测试必须用**布局算出的矩形**，不能自己再推一遍——两套几何一旦漂移，
//! 就会出现"看得见但点不到"这类极难查的 bug。所以本模块只负责**遍历与优先级**，
//! 矩形一律取自 [`crate::tree::Node::rect`]。
//!
//! # 优先级：后来的在上面
//!
//! 命中测试要返回**最上层**命中的那个，而不是第一个。遍历顺序与
//! [`crate::paint`] 的合成顺序一致（`Layer::Popup` 在 `Content` 之后），因此
//! 反向遍历即可得到「后加入的优先」。
//!
//! 早期返回（first hit）是错的：浮层盖住按钮时，点击必须落到浮层，而不是被它
//! 下面的按钮吃掉。

use crate::core::{Point, Rect};
use crate::paint::Layer;

/// 命中测试请求。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HitTestRequest {
    /// 测试点（窗口坐标）。
    pub point: Point,
    /// 穿透：忽略该层及以上的所有内容（用于让点击穿过浮层落到下层）。
    pub ignore_above: Option<Layer>,
}

/// 命中结果。
#[derive(Debug, Clone, PartialEq)]
pub struct HitResult {
    /// 命中的节点路径（从根到命中者）。
    pub path: Vec<HitNode>,
}

/// 路径上的一个节点。
#[derive(Debug, Clone, PartialEq)]
pub struct HitNode {
    /// 该节点在树中的稳定 key（没有则用路径下标）。
    pub id: crate::tree::ElementKey,
    /// 该节点的矩形（窗口坐标）。
    pub rect: Rect,
    /// 该节点是否可交互（`false` 的节点只作为路径的一部分返回）。
    pub interactive: bool,
}

/// 矩形是否包含点。
///
/// 用 `>=` 与 `<` 而不是 `<=`：相邻矩形（如按钮与其边框）必须**恰好**归属一侧，
/// 否则边界点会同时命中两者。
pub fn rect_contains(rect: Rect, point: Point) -> bool {
    point.x >= rect.x && point.x < rect.right() && point.y >= rect.y && point.y < rect.bottom()
}

/// 遍历树收集命中路径（`node` 的矩形由调用方提供，因为核心不持有绝对坐标）。
///
/// `rect_of` 让调用方把「节点 → 窗口坐标矩形」这一步交给宿主——宿主持有变换栈
/// （滚动偏移、缩放），核心不该猜。
///
/// 返回的路径包含从根到最上层命中者的**全部**节点（含不可交互的容器），因为
/// 事件冒泡需要它们；只有 `interactive` 的节点才是真正的处理目标。
pub fn hit_test(
    node: &crate::tree::Node,
    request: HitTestRequest,
    rect_of: &mut dyn FnMut(&crate::tree::Node) -> Option<Rect>,
    out: &mut HitResult,
) -> bool {
    out.path.clear();
    hit_rec(node, request, rect_of, &mut Vec::new(), out)
}

fn hit_rec(
    node: &crate::tree::Node,
    request: HitTestRequest,
    rect_of: &mut dyn FnMut(&crate::tree::Node) -> Option<Rect>,
    path: &mut Vec<HitNode>,
    out: &mut HitResult,
) -> bool {
    let Some(rect) = rect_of(node) else {
        return false;
    };
    if !rect_contains(rect, request.point) {
        return false;
    }
    let id = node
        .key()
        .cloned()
        .unwrap_or_else(|| crate::tree::ElementKey::Id(path.len() as u64));
    path.push(HitNode {
        id,
        rect,
        interactive: false,
    });

    // 反向遍历子节点：后声明的在上层，先命中它。
    let mut child_path: Vec<HitNode> = Vec::new();
    for child in node.children().iter().rev() {
        if hit_rec(child, request, rect_of, &mut child_path, out) {
            path.extend(child_path);
            out.path = path.clone();
            return true;
        }
    }

    out.path = path.clone();
    true
}

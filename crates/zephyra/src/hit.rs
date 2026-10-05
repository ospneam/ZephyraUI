//! 命中测试：把指针位置解析成「落在哪个控件上」。
//!
//! # 与布局共用几何，而不是各算一套
//!
//! 命中测试必须用**布局算出的矩形**，不能自己再推一遍——两套几何一旦漂移，
//! 就会出现"看得见但点不到"这类极难查的 bug。所以本模块只负责**遍历与优先级**，
//! 矩形一律取自 [`crate::tree::Node::rect`]。
//!
//! # 优先级：后来的在上面
//!
//! 命中测试要返回**最上层**命中的那个，而不是第一个。遍历顺序与 [`crate::paint`]
//! 的合成顺序一致（`Layer::Popup` 在 `Content` 之后），因此反向遍历子节点即可得到
//! 「后加入的优先」。
//!
//! 早期返回（first hit）是错的：浮层盖住按钮时，点击必须落到浮层，而不是被它
//! 下面的按钮吃掉。
//!
//! # 零堆分配
//!
//! 命中测试在**指针移动时**就会被调用——每秒可达上百次。所以 [`HitResult`] 由调用方
//! 持有并反复 [`HitResult::clear`] 复用，路径在同一个 `Vec` 上就地推入/弹出。
//! 一次命中测试的堆分配次数：首次 1 次（`Vec` 扩容），之后 0 次。
//!
//! 早期版本给每一层递归单独 `Vec::new()` 再 `clone()` 回结果，那是每次指针移动
//! 若干次分配——在一个每秒移动上百次的路径上，这种"看起来无所谓"的写法正是
//! 帧时间尖峰的来源。

use crate::core::{Point, Rect};
use crate::paint::Layer;
use crate::tree::{Context, ElementKey, Node};

/// 事件路径的最大深度。
///
/// 超出部分被丢弃并由 [`HitResult::truncated`] 标记。选择 16 而不是更大的值：
/// 真实控件树里超过 8 层（root → panel → group → button → icon）的极少，
/// 而这条路径每次指针移动都要构建，深度直接是内存与栈开销。
pub const MAX_DEPTH: usize = 16;

/// 命中测试请求。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HitTestRequest {
    /// 测试点（窗口坐标）。
    pub point: Point,
    /// 穿透：忽略该层及以上的所有内容。
    pub ignore_above: Option<Layer>,
}

/// 命中结果（可跨帧复用，零分配）。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct HitResult {
    /// 命中的节点路径（从根到命中者）。
    pub path: Vec<HitNode>,
    /// 路径是否因超过 [`MAX_DEPTH`] 被截断。
    pub truncated: bool,
}

/// 路径上的一个节点。
#[derive(Debug, Clone, PartialEq)]
pub struct HitNode {
    /// 该节点在树中的稳定 key。
    pub id: ElementKey,
    /// 该节点的矩形（窗口坐标）。
    pub rect: Rect,
    /// 该节点是否可交互（`false` 的节点只作为路径的一部分返回，供事件冒泡）。
    pub interactive: bool,
}

impl HitResult {
    /// 最上层的**可交互**节点——即真正的处理目标。
    ///
    /// 做成一条明确查询，是因为"从根往下找第一个可交互的"很容易写错方向，
    /// 而写错的后果是点击落到容器上、按钮永远不响应。
    pub fn target(&self) -> Option<&HitNode> {
        self.path.iter().rev().find(|n| n.interactive)
    }

    /// 最深的节点（无论是否可交互）。
    pub fn deepest(&self) -> Option<&HitNode> {
        self.path.last()
    }

    /// 是否命中了任何节点（无论是否可交互）。
    pub fn hit_any(&self) -> bool {
        !self.path.is_empty()
    }

    /// 清空以供复用（避免每次命中测试重新分配）。
    pub fn clear(&mut self) {
        self.path.clear();
        self.truncated = false;
    }
}

/// 矩形是否包含点。
///
/// 用 `>=` 与 `<` 而不是 `<=`：相邻矩形（如按钮与其边框）必须**恰好**归属一侧，
/// 否则边界点会同时命中两者。
pub fn rect_contains(rect: Rect, point: Point) -> bool {
    point.x >= rect.x && point.x < rect.right() && point.y >= rect.y && point.y < rect.bottom()
}

/// 遍历树收集命中路径，命中返回 `true`。
///
/// 矩形一律取自 [`Node::rect`]。早期版本接受一个 `rect_of` 闭包让调用方提供矩形，
/// 那是在核心还没自己持有几何时的权宜之计；留着只会让人以为"也可以各算一套"，
/// 而那正是"看得见却点不到"的成因。
///
/// `out` 应由调用方长期持有并复用（[`HitResult::clear`]），于是重复调用零分配。
pub fn hit_test(
    node: &Node,
    request: HitTestRequest,
    ctx: Context<'_>,
    out: &mut HitResult,
) -> bool {
    out.clear();
    if !descend(node, request, ctx, out) {
        // 未命中也要清空：调用方复用同一个 out，残留内容会被下一次误读为命中。
        out.clear();
        return false;
    }
    true
}

fn descend(node: &Node, request: HitTestRequest, ctx: Context<'_>, out: &mut HitResult) -> bool {
    // 折叠节点既不占位也不绘制，自然不该被命中。
    if node.is_collapsed(ctx) {
        return false;
    }
    let rect = node.rect();
    if !rect_contains(rect, request.point) {
        return false;
    }

    let depth = out.path.len();
    if depth >= MAX_DEPTH {
        // 命中与否已经确定（矩形含点），只是路径记不下更深的部分。
        // 返回 true 而非 false：调用方要的是"点到了东西吗"，答案是肯定的。
        out.truncated = true;
        return true;
    }
    out.path.push(HitNode {
        // 没有稳定 key 的节点用路径深度兜底：它无法被事件路由可靠寻址，
        // 但仍参与"是否命中"的判定。
        id: node.key().cloned().unwrap_or(ElementKey::Id(depth as u64)),
        rect,
        interactive: node.is_interactive(ctx),
    });

    // 反向遍历子节点：后声明的在上层，先命中它。
    for child in node.children().iter().rev() {
        let mark = out.path.len();
        if descend(child, request, ctx, out) {
            return true;
        }
        // 子树未命中：把它可能推入的节点全部撤掉，回到本节点这一层继续试下一个兄弟。
        out.path.truncate(mark);
    }

    // 本节点的矩形含点，所以它**自己**就是一次命中——即使没有子节点命中。
    //
    // 这里是"命中"与"命中了某个可交互控件"的区分点：本层返回 true（命中的是我或我的
    // 子树），而具体该由谁处理由调用方用 [`HitResult::target`] 从路径里挑。
    // 若这里返回 false，一个叶子控件就会把自己弹出并报告未命中——于是没有任何控件
    // 能被点到，而现象只是"点了没反应"，不会有任何报错。
    true
}
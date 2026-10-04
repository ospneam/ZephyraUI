//! 布局：Measure / Arrange 协议。
//!
//! # 两趟而不是一趟
//!
//! WinUI 与 CSS 布局都分「测量」与「排列」两趟，原因是父容器必须在知道每个子项
//! 的**意图尺寸**之后才能决定如何分配空间（父给的约束通常是整块可用空间，
//! 而不是每个子项各自的目标尺寸）。
//!
//! - [`measure`]：给定约束 → 返回内容想要的尺寸（`Size`，可能小于约束）。
//! - [`arrange`]：给定最终矩形 → 摆放自身与子项。
//!
//! # 为什么是 trait 而不是 `dyn`
//!
//! 布局是热路径，且每个节点的布局类型是静态已知的。静态分发让 trait object 的
//! 一次间接调用与一次 `dyn` 胖指针解引用消失。代价是无法在运行时混排不同布局
//! 容器——而这本来也不需要（一个容器的子项布局由它自己决定）。

use crate::core::{Rect, Size, Thickness};

/// 布局约束。
///
/// 用「允许的尺寸区间」而不是「固定尺寸」表达，因为三种情形都要覆盖：
/// 固定（容器给了确切空间）、最小（`Fill` 要的是下限）、无界（滚动内容测量）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Constraints {
    /// 允许的最小尺寸。
    pub min: Size,
    /// 允许的最大尺寸；`f32::INFINITY` 表示无界。
    pub max: Size,
}

impl Constraints {
    /// 固定尺寸约束。
    pub fn fixed(size: Size) -> Self {
        Self {
            min: size,
            max: size,
        }
    }

    /// 无界约束（滚动内容测量用）。
    pub fn unbounded() -> Self {
        Self {
            min: Size::new(0.0, 0.0),
            max: Size::new(f32::INFINITY, f32::INFINITY),
        }
    }

    /// 在给定轴上是否无界。
    pub fn is_unbounded_width(&self) -> bool {
        self.max.width.is_infinite()
    }

    /// 在给定轴上是否无界。
    pub fn is_unbounded_height(&self) -> bool {
        self.max.height.is_infinite()
    }

    /// 收紧约束（把当前区间与新的上下界求交）。
    pub fn tighten(&self, min: Size, max: Size) -> Constraints {
        Constraints {
            min: Size::new(
                self.min.width.max(min.width),
                self.min.height.max(min.height),
            ),
            max: Size::new(
                self.max.width.min(max.width),
                self.max.height.min(max.height),
            ),
        }
    }

    /// 按内边距收缩可用空间。
    pub fn deflate(&self, padding: Thickness) -> Constraints {
        Constraints {
            min: Size::new(
                (self.min.width - padding.left - padding.right).max(0.0),
                (self.min.height - padding.top - padding.bottom).max(0.0),
            ),
            max: Size::new(
                (self.max.width - padding.left - padding.right).max(0.0),
                (self.max.height - padding.top - padding.bottom).max(0.0),
            ),
        }
    }
}

/// 测量结果。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeasureResult {
    /// 内容想要的尺寸。
    pub size: Size,
}

/// 可测量、可排列的节点。
///
/// 实现者持有子树并按上述两趟协议布局自身与子项。
pub trait LayoutNode {
    /// 在给定约束下测量自身与子树，返回内容想要的尺寸。
    ///
    /// **实现者不应在此处固定自身位置**——那属于 [`LayoutNode::arrange`]。
    fn measure(&mut self, constraints: Constraints) -> MeasureResult;

    /// 在最终矩形内摆放自身与子项。
    fn arrange(&mut self, rect: Rect);
}

/// 水平对齐。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Align {
    /// 拉伸填满。
    Stretch,
    /// 起始边对齐。
    #[default]
    Start,
    /// 居中。
    Center,
    /// 末端边对齐。
    End,
}

/// 垂直对齐。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AlignItems {
    /// 拉伸填满。
    Stretch,
    /// 起始边对齐。
    #[default]
    Start,
    /// 居中。
    Center,
    /// 末端边对齐。
    End,
}

/// 在给定空间内按对齐方式摆放一个已测量的盒子，返回它的最终矩形。
///
/// 抽成自由函数是因为栈布局、横/纵布局、以及各种居中容器全都用同一套算法，
/// 各自实现一遍必然出现细微不一致。
pub fn align_box(content: Size, available: Size, horizontal: Align, vertical: AlignItems) -> Rect {
    let (x, width) = match horizontal {
        Align::Stretch => (0.0, available.width),
        Align::Start => (0.0, content.width),
        Align::Center => ((available.width - content.width) * 0.5, content.width),
        Align::End => (available.width - content.width, content.width),
    };
    let (y, height) = match vertical {
        AlignItems::Stretch => (0.0, available.height),
        AlignItems::Start => (0.0, content.height),
        AlignItems::Center => ((available.height - content.height) * 0.5, content.height),
        AlignItems::End => (available.height - content.height, content.height),
    };
    Rect::new(x, y, width, height)
}

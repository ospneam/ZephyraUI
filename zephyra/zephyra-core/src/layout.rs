//! 布局：Measure / Arrange 协议 + 遍历节点树的两趟驱动。
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
//! # 几何只有一处算法
//!
//! 命中测试（[`crate::hit`]）与绘制（[`crate::paint::render`]）读的是**同一批**
//! 已摆好的矩形，所以不会出现"看得见但点不到"。这要求两趟都写在本模块而不是
//! 各写一份。
//!
//! # 为什么 `Stack` 是静态分发而不是 `dyn`
//!
//! 布局是热路径，且每个节点的布局类型是静态已知的。静态分发让 trait object 的
//! 一次间接调用与一次胖指针解引用消失。代价是无法在运行时混排不同布局容器
//! ——而这本来也不需要（一个容器的子项布局由它自己决定）。

use crate::context::Context;
use crate::core::{Point, Rect, Size, Thickness};
use crate::node::{Dirty, Node};

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
/// 测量结果。
///
/// `shaped` 是**排版结果本身**，不是尺寸。带着它是为了让绘制趟不必重排一次文字。
#[derive(Debug, Clone, PartialEq)]
pub struct MeasureResult {
    /// 内容想要的尺寸。
    pub size: Size,
    /// 这次测量产出的排版结果（`None` = 本节点没有文本）。
    ///
    /// # 为什么把 `ShapedText` 挂在测量结果上
    ///
    /// 官方把排版结果放在元素的布局存储里（`CLayoutStorage` 里
    /// `m_desiredSize` 旁边就是它），绘制趟直接取。原因是**排版是整帧最贵的一步**：
    /// `shape()` 要按样式算出每一行的基线、每个字形的位置与步进。
    ///
    /// 本仓曾经把排版结果**扔掉**（`text_size()` 只取 `shaped.size` 就返回），
    /// 于是绘制趟 `paint::text_command` 每帧对每个文本节点重新 `shape()` 一次。
    /// 实测（`--bench-resize` 的临时探针，324 帧）：`render` 92.9 ms、
    /// `layout` 18.4 ms —— **render 是 layout 的 5 倍**，而它里面绝大部分
    /// 就是重复排版。
    ///
    /// # 为什么绘制趟仍要重算字形矩形
    ///
    /// `shape()` 与位置无关，所以复用 `ShapedText` 本身是安全的；但字形的
    /// 屏幕矩形**必须**按当前 origin 重算 —— 节点可能被绘制偏移
    /// （`props::TRANSLATE_X/Y`）移动过，而排版结果里没有屏幕坐标。
    ///
    /// # 失效时机
    ///
    /// 它挂在 `measure_cache` 里，而缓存的失效时机是"某个布局相关属性被改"
    /// （见 [`crate::node::clear_dirty`] 的文档）。`text-size` / `text-line-height` /
    /// `text-max-width` / `TEXT` 都是布局键 ⇒ 改它们会让缓存作废 ⇒ 重新排版。
    pub shaped: Option<crate::text::ShapedText>,
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

/// 堆叠方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// 水平排列（对应 WinUI `StackPanel` 的 Orientation=Horizontal）。
    Horizontal,
    /// 垂直排列。
    Vertical,
}

/// 主轴对齐。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MainAxisAlign {
    /// 从起始边排布。
    #[default]
    Start,
    /// 居中。
    Center,
    /// 末端对齐。
    End,
    /// 首尾对齐（剩余空间均摊到间隔里）。
    SpaceBetween,
}

/// 一维堆叠布局（WinUI `StackPanel` / CSS flex）。
///
/// 子项在交叉轴上如何摆由**排列时**的 `cross_align` 决定，而不是这里声明——
/// 这正是 `align-items` 与逐项 `align-self` 的区别。工具栏里"图标按钮贴顶、
/// 组容器撑满高度"就依赖这个区分。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stack {
    /// 排列方向。
    pub direction: Direction,
    /// 主轴上相邻子项之间的固定间距。
    pub spacing: f32,
    /// 整组在主轴上的摆放。
    pub main_align: MainAxisAlign,
}

impl Default for Stack {
    fn default() -> Self {
        Self {
            direction: Direction::Horizontal,
            spacing: 0.0,
            main_align: MainAxisAlign::Start,
        }
    }
}

impl Stack {
    /// 主轴尺寸 = 各子项之和 + 间距 × (n−1)。
    ///
    /// 空容器返回 0 而不是 `spacing`：否则调用方无从区分"没有子项"与
    /// "有一个不可见子项"。
    pub fn main_size(&self, sizes: &[Size]) -> f32 {
        if sizes.is_empty() {
            return 0.0;
        }
        let total: f32 = sizes
            .iter()
            .map(|s| match self.direction {
                Direction::Horizontal => s.width,
                Direction::Vertical => s.height,
            })
            .sum();
        total + self.spacing * (sizes.len() as f32 - 1.0)
    }

    /// 内容总尺寸：主轴取累加，交叉轴取最大值。
    pub fn content_size(&self, sizes: &[Size]) -> Size {
        let main = self.main_size(sizes);
        let cross = sizes
            .iter()
            .map(|s| match self.direction {
                Direction::Horizontal => s.height,
                Direction::Vertical => s.width,
            })
            .fold(0.0f32, f32::max);
        match self.direction {
            Direction::Horizontal => Size::new(main, cross),
            Direction::Vertical => Size::new(cross, main),
        }
    }

    /// 主轴起点与间距（`main_offsets` 的规则本体）。
    ///
    /// # 为什么要抽出来
    ///
    /// `main_offsets`（切片版）与 `each_rect`（流式版）需要**同一套**规则。
    /// 写两遍的话，`SpaceBetween` 的单项特例这类边角必然会漂移 —— 而漂移的表现
    /// 是"两个版本在某个对齐方式下排出来不一样"，极难归因。
    fn main_offset_rules(&self, n: usize, content: f32, available_main: f32) -> (f32, f32) {
        match self.main_align {
            MainAxisAlign::Start => (0.0, self.spacing),
            MainAxisAlign::Center => ((available_main - content) * 0.5, self.spacing),
            MainAxisAlign::End => (available_main - content, self.spacing),
            // 单项时 SpaceBetween 不应留下任何空隙，否则"唯一一项"会被推到末尾。
            MainAxisAlign::SpaceBetween if n == 1 => (0.0, self.spacing),
            MainAxisAlign::SpaceBetween => {
                let slack = (available_main - content).max(0.0);
                (0.0, self.spacing + slack / (n as f32 - 1.0))
            }
        }
    }

    /// 该 `Size` 在堆叠轴上的长度。
    fn main_of(&self, size: Size) -> f32 {
        match self.direction {
            Direction::Horizontal => size.width,
            Direction::Vertical => size.height,
        }
    }

    /// 内容总尺寸，**不收集中间 `Vec`**（流式版）。
    ///
    /// # 为什么不直接用 [`Self::content_size`]
    ///
    /// `content_size` 要一个 `&[Size]`，而调用方得先把所有子项的尺寸收集进一个
    /// `Vec` —— 那是**每节点每趟一次堆分配**。resize 实测每帧 1709 次分配
    /// （`--bench-resize --alloc-probe`，约合每节点 2.8 次），而这一趟不需要
    /// "看第二眼"：主轴求和、交叉轴取最大都能在一次遍历里折出来。
    ///
    /// `content_size` 保留（`&[Size]` 更适合已经拿着切片的调用方，也被测试用着），
    /// 布局趟走这个流式版。
    pub fn content_size_of<I: Iterator<Item = Size>>(&self, sizes: I) -> Size {
        let mut main = 0.0f32;
        let mut cross = 0.0f32;
        let mut n = 0usize;
        for s in sizes {
            main += self.main_of(s);
            cross = cross.max(match self.direction {
                Direction::Horizontal => s.height,
                Direction::Vertical => s.width,
            });
            n += 1;
        }
        // 与 `main_size` 同一条规则：空容器返回 0 而不是 spacing。
        if n == 0 {
            return Size::new(0.0, 0.0);
        }
        main += self.spacing * (n as f32 - 1.0);
        match self.direction {
            Direction::Horizontal => Size::new(main, cross),
            Direction::Vertical => Size::new(cross, main),
        }
    }

    /// 逐个把子项矩形交出去，**不收集中间 `Vec`**（流式版）。
    ///
    /// # 为什么需要 `I: Clone`
    ///
    /// 主轴起点（`Center` / `End` / `SpaceBetween`）依赖**总长**，而总长要第一趟
    /// 才拿得到 —— 所以要遍历两次尺寸。`Clone` 的迭代器几乎总是
    /// `slice::Iter` 或 `Cloned<…>`，克隆零成本。
    ///
    /// 换来的好处：排列趟每节点零分配。改之前是每节点一个 `Vec<Size>` 加一个

    /// 各子项在主轴上的起点（相对本容器起点）。
    ///
    /// 规则本体在 [Self::main_offset_rules] —— 切片版与流式版共用同一套，
    /// 免得 SpaceBetween 的单项特例在两处漂移。
    pub fn main_offsets(&self, sizes: &[Size], available_main: f32) -> Vec<f32> {
        let n = sizes.len();
        if n == 0 {
            return Vec::new();
        }
        let (start, gap) = self.main_offset_rules(n, self.main_size(sizes), available_main);
        let mut offsets = Vec::with_capacity(n);
        let mut cursor = start;
        for s in sizes {
            offsets.push(cursor);
            cursor += self.main_of(*s) + gap;
        }
        offsets
    }

    /// 给**子项**的约束：堆叠轴无穷，交叉轴保持原值。
    ///
    /// # 为什么只放开一根轴
    ///
    /// 交叉轴必须保持有限，否则 `cross-align: stretch` 无从谈起 ——
    /// 子项会退回"内容宽度"，横向排版全散。官方也是只放开一根
    /// （`StackPanel.cpp:24-30`）。
    ///
    /// `min` 与 `max` **都要**改：只把 `max` 设成无穷而留着 `min`，
    /// 子项会拿到"至少要 N 这么宽、上不封顶"这种自相矛盾的约束，
    /// 而 [`pick_axis`] 的下限夹逼会把结果顶到那个 `min` 上。
    ///
    /// # 为什么这一行就是 resize 便宜的**全部**原因
    ///
    /// 测量缓存按**约束精确匹配**（见 [`cached_for`]）。子项在堆叠轴上拿的是
    /// 无穷，于是**只改窗口高度时，子项的约束一个字都没变** ⇒ 缓存命中 ⇒ 整段子树
    /// 跳过测量，也就不再重新整形那几百行文字。
    /// 门禁 `layout_cache_tests::height_only_change_does_not_remeasure_subtree` 钉的就是这一条。
    pub fn child_constraints(&self, deflated: Constraints) -> Constraints {
        match self.direction {
            Direction::Horizontal => Constraints {
                min: Size::new(0.0, deflated.min.height),
                max: Size::new(f32::INFINITY, deflated.max.height),
            },
            Direction::Vertical => Constraints {
                min: Size::new(deflated.min.width, 0.0),
                max: Size::new(deflated.max.width, f32::INFINITY),
            },
        }
    }

    /// 把主轴偏移落成每个子项的矩形。
    /// 把主轴偏移落成每个子项的矩形。
    ///
    /// `cross_align` 决定子项在交叉轴上的位置与伸展；[`AlignItems::Stretch`] 时
    /// 子项撑满交叉轴（工具栏的"组容器撑满高度"靠它）。
    pub fn arrange(
        &self,
        origin: Point,
        available: Size,
        sizes: &[Size],
        cross_align: AlignItems,
    ) -> Vec<Rect> {
        let available_main = match self.direction {
            Direction::Horizontal => available.width,
            Direction::Vertical => available.height,
        };
        let available_cross = match self.direction {
            Direction::Horizontal => available.height,
            Direction::Vertical => available.width,
        };
        let offsets = self.main_offsets(sizes, available_main);
        sizes
            .iter()
            .zip(offsets)
            .map(|(size, offset_main)| {
                let (cross_pos, cross_extent) = match (self.direction, cross_align) {
                    (Direction::Horizontal, AlignItems::Stretch) => (origin.y, available_cross),
                    (Direction::Horizontal, AlignItems::Start) => (origin.y, size.height),
                    (Direction::Horizontal, AlignItems::Center) => (
                        origin.y + (available_cross - size.height) * 0.5,
                        size.height,
                    ),
                    (Direction::Horizontal, AlignItems::End) => {
                        (origin.y + available_cross - size.height, size.height)
                    }
                    (Direction::Vertical, AlignItems::Stretch) => (origin.x, available_cross),
                    (Direction::Vertical, AlignItems::Start) => (origin.x, size.width),
                    (Direction::Vertical, AlignItems::Center) => {
                        (origin.x + (available_cross - size.width) * 0.5, size.width)
                    }
                    (Direction::Vertical, AlignItems::End) => {
                        (origin.x + available_cross - size.width, size.width)
                    }
                };
                match self.direction {
                    Direction::Horizontal => {
                        Rect::new(origin.x + offset_main, cross_pos, size.width, cross_extent)
                    }
                    Direction::Vertical => {
                        Rect::new(cross_pos, origin.y + offset_main, cross_extent, size.height)
                    }
                }
            })
            .collect()
    }
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

// ==================================================================
// 以下是遍历 `Node` 树的两趟驱动。
// ==================================================================

/// 布局整棵子树：测量 + 按最终矩形摆放每个子节点。
///
/// 这是 [`crate::paint::render`] 的前置步骤——两者必须成对调用，因为 `render` 只读
/// 已摆好的矩形，不再自己算几何。
pub fn layout(node: &mut Node, bounds: Rect, ctx: Context<'_>) {
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
    if let Some(size) = cached_for(node, constraints) {
        return size;
    }
    let padding = node.padding(ctx);
    // 堆叠轴给子项**无穷**约束；交叉轴保持有限（stretch 依赖它）。
    //
    // 官方依据（microsoft-ui-xaml 的 dxaml/xcp/core/core/elements/StackPanel.cpp:24-30）：
    //
    //     XSIZEF childConstraint = (m_orientation == DirectUI::Orientation::Vertical)
    //         ? XSIZEF{ availableSize.width - combinedThickness.width, XFLOAT_INF }
    //         : XSIZEF{ XFLOAT_INF, availableSize.height - combinedThickness.height };
    //
    // 而这个子项约束的缓存是**跨帧**存活的（见 cached_for 与
    // crate::node::clear_dirty 的文档）—— 两条合起来才是 resize 便宜的
    // 全部原因：只改高度时约束一个字没变，于是整段子树跳过测量。
    let stack = node.stack(ctx);
    let child_constraints = stack.child_constraints(constraints.deflate(padding));

    // 折叠的子节点不参与测量，也不占间距。

    // 一边测一边折叠，**不收集中间 Vec**（改之前每节点一次堆分配）。
    let mut main_sum = 0.0f32;
    let mut cross_max = 0.0f32;
    let mut n = 0usize;
    for i in 0..node.children().len() {
        if node.children()[i].is_collapsed(ctx) {
            continue;
        }
        let s = measure(&mut node.children_mut()[i], child_constraints, ctx);
        main_sum += stack.main_of(s);
        cross_max = cross_max.max(match stack.direction {
            Direction::Horizontal => s.height,
            Direction::Vertical => s.width,
        });
        n += 1;
    }
    let mut content = if n == 0 {
        Size::new(0.0, 0.0)
    } else {
        let main = main_sum + stack.spacing * (n as f32 - 1.0);
        match stack.direction {
            Direction::Horizontal => Size::new(main, cross_max),
            Direction::Vertical => Size::new(cross_max, main),
        }
    };

    // 文本节点自己贡献内容尺寸。
    //
    // 必须在 `content_size` **之后**做这件事，而不是把文本当成一个子项：文本是
    // 叶节点，没有自己的 `Node`，所以子项列表里根本没有它。若不单独补，内容尺寸
    // 恒为 0，文本行会被压成零高 —— 而"看不见"是这类 bug 最常见的表现形式。
    //
    // ⚠ 排版结果**要留下**（`shaped` 稍后进 `MeasureResult`）：绘制趟从测量缓存里
    // 取它，于是每帧只排版一次。丢掉它就是每帧重排全部文字 —— 实测那让
    // `render` 比 `layout` 还贵 5 倍。
    let shaped = node.shape(ctx);
    if let Some(shaped) = &shaped {
        content = shaped.size;
    }
    let (own_w, own_h) = explicit_size(node, ctx);
    let result = MeasureResult {
        size: Size::new(
            pick_axis(own_w, content.width, Axis::Horizontal, constraints, padding),
            pick_axis(own_h, content.height, Axis::Vertical, constraints, padding),
        ),
        shaped,
    };
    let size = result.size;
    node.cache_measure(constraints, result);
    node.merge_dirty(Dirty::Layout);
    size
}

/// 按最终矩形摆放整棵子树。
///
/// 顺序无关紧要：绘制顺序由 [`crate::paint::render`] 的 `order` 决定，这里只算几何。
///
/// # 零分配（改之前每节点三次）
///
/// 原写法是「收集可见子项 `Vec<usize>` → 收集尺寸 `Vec<Size>` → 收集矩形
/// `Vec<Rect>`」三步。而这三条**没有一条需要真的收集**：
///
/// - 主轴起点（`Center` / `End` / `SpaceBetween`）只依赖**总长**，一趟折叠就有；
/// - 矩形算出来立刻递归下去用，中间不留存。
///
/// resize 实测每帧 1709 次分配、约合每节点 2.8 次（`--bench-resize --alloc-probe`）。
///
/// # 为什么不用 [`Stack::each_rect`] 那样的流式回调
///
/// 递归要的是 `&mut Node`，而尺寸要从同一个节点的**子节点**里读 ——
/// 闭包迭代器会同时借走 `&Node`，与随后的 `&mut` 冲突。所以两趟都写成显式循环：
/// 第一趟只读（折叠总长），第二趟可变（算矩形 + 递归）。
///
/// # 缺尺寸时占零而不是消失
///
/// 尺寸取自测量缓存。缓存缺失时填 `0` **占位**而不是 `filter_map` 掉 ——
/// 掉一个会让后面所有下标整体前移，那是 `paint` 侧一个真实踩过的坑
/// （见 `docs/task-resize.md` §完成情况）。
pub fn arrange(node: &mut Node, bounds: Rect, ctx: Context<'_>) {
    node.set_rect(bounds);
    let padding = node.padding(ctx);
    let inner = bounds.inset(padding);
    let stack = node.stack(ctx);
    let cross_align = node.cross_align(ctx);
    let available = Size::new(inner.width, inner.height);
    let origin = Point::new(inner.x, inner.y);

    // ---- 第一趟：折叠主轴总长与项数（只读）----
    let mut main_sum = 0.0f32;
    let mut count = 0usize;
    for c in node.children().iter() {
        if c.is_collapsed(ctx) {
            continue;
        }
        main_sum += stack.main_of(cached_size_or_zero(c));
        count += 1;
    }
    if count == 0 {
        return;
    }
    let content_main = main_sum + stack.spacing * (count as f32 - 1.0);
    let (start, gap) = stack.main_offset_rules(count, content_main, stack.main_of(available));
    let available_cross = match stack.direction {
        Direction::Horizontal => available.height,
        Direction::Vertical => available.width,
    };

    // ---- 第二趟：算矩形并递归（可变）----
    let mut cursor = start;
    for child in node.children_mut().iter_mut() {
        if child.is_collapsed(ctx) {
            continue;
        }
        let size = cached_size_or_zero(child);
        let (cross_pos, cross_extent) = match (stack.direction, cross_align) {
            (Direction::Horizontal, AlignItems::Stretch) => (origin.y, available_cross),
            (Direction::Horizontal, AlignItems::Start) => (origin.y, size.height),
            (Direction::Horizontal, AlignItems::Center) => (
                origin.y + (available_cross - size.height) * 0.5,
                size.height,
            ),
            (Direction::Horizontal, AlignItems::End) => {
                (origin.y + available_cross - size.height, size.height)
            }
            (Direction::Vertical, AlignItems::Stretch) => (origin.x, available_cross),
            (Direction::Vertical, AlignItems::Start) => (origin.x, size.width),
            (Direction::Vertical, AlignItems::Center) => {
                (origin.x + (available_cross - size.width) * 0.5, size.width)
            }
            (Direction::Vertical, AlignItems::End) => {
                (origin.x + available_cross - size.width, size.width)
            }
        };
        let rect = match stack.direction {
            Direction::Horizontal => {
                Rect::new(origin.x + cursor, cross_pos, size.width, cross_extent)
            }
            Direction::Vertical => {
                Rect::new(cross_pos, origin.y + cursor, cross_extent, size.height)
            }
        };
        arrange(child, rect, ctx);
        cursor += stack.main_of(size) + gap;
    }
}

/// 测量缓存里的尺寸；缺失时返回**零尺寸占位**。
///
/// 见 [`arrange`] 的说明：占位而不是跳过 —— 跳过会让下标错位。
#[inline]
fn cached_size_or_zero(node: &Node) -> Size {
    node.cached_size().unwrap_or(Size::new(0.0, 0.0))
}

/// 缓存命中且约束相同则返回尺寸。
///
/// 单独抽出来是因为要同时看两样东西：缓存存在，且**约束相等**。只判存在的话，
/// 父容器尺寸一变就会拿旧结果继续用 —— 那正是 `Constraints` 必须参与相等比较的
/// 原因。
fn cached_for(node: &Node, constraints: Constraints) -> Option<Size> {
    match node.measure_cache.as_ref() {
        Some((cached, result)) if cached == &constraints => Some(result.size),
        _ => None,
    }
}

/// 显式尺寸（`None` = 由内容决定）。
fn explicit_size(node: &Node, ctx: Context<'_>) -> (Option<f32>, Option<f32>) {
    (
        finite(node.number_or(&crate::props::WIDTH, f32::NAN, ctx)),
        finite(node.number_or(&crate::props::HEIGHT, f32::NAN, ctx)),
    )
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
    fn padding(self, p: Thickness) -> f32 {
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
    padding: Thickness,
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

/// `NaN` 或负数表示"未设置"，转成 `None`。
fn finite(v: f32) -> Option<f32> {
    (v.is_finite() && v >= 0.0).then_some(v)
}

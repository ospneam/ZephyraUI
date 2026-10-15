//! 帧缓存：把「这一帧画什么」的决策**完整收进核心里**。
//!
//! # 这份文件存在的理由
//!
//! 在它出现之前，散落着同一件事的三块状态住在**宿主**里：
//!
//! | 原来在 | 内容 |
//! |---|---|
//! | `zephyra-winit/src/lib.rs` 的 `node: Option<Node>` | 保留树 |
//! | 同上的 `list: DisplayList` | 本帧要画的命令 |
//! | 同上的 `needs_layout: bool` / `dirty: bool` | 要不要重排 / 要不要重绘 |
//!
//! 而它们的**数据**全在核心里（[`Node`] / [`DisplayList`] / [`Dirty`] /
//! [`DiffStats`]），宿主只是拿着 core 的零件自己编排。这违反
//! `zephyra/zephyra-base/README.md` 的第二条契约 ——「不决定画什么」：宿主读
//! `Node::dirty()` 来决定跳过 `paint::render`，那正是在决定这一帧画什么。
//!
//! 现在这件事只有一个地方做：[`FrameCache::frame`]。宿主问一个问题
//! （「这一帧的 DisplayList 是什么」），拿到 [`FrameOutcome`]，然后只回答
//! 另一个问题（「我要不要把它推上屏」）。
//!
//! # core 与 base 的分工
//!
//! | | 谁 |
//! |---|---|
//! | 这一帧的 `DisplayList` 是什么 ——**含「与上次逐字节相同」** | **core**（本模块） |
//! | 哪块区域脏了（纯几何） | **core** |
//! | scissor 怎么设、局部 clear 怎么做 | base（D3D API） |
//! | 要不要 present（flip model / vsync） | base |
//! | 事件触发（resize / 指针 / 按键） | base（Win32 词汇） |
//!
//! # ⚠️ 为什么跨帧局部重绘不可行（所以 [`Damage`] 只用于帧内 scissor）
//!
//! 两个宿主都用 flip model（`DXGI_SWAP_EFFECT_FLIP_DISCARD`，DComp 路径是
//! `FLIP_SEQUENTIAL`），而 flip model 下 `Present` **之后**后缓冲内容未定义 ——
//! 任何要 present 的帧都必须重画整个目标。所以 [`Damage`] **不是**给宿主用来
//! 「跳过脏区外的像素」的，那在当前模型下根本不成立。它只有一个用途：
//! **帧内**限制光栅化范围（scissor），省脏区外的填充率。
//!
//! Skia 同样没有跨帧的 damage 跟踪（`SkSurface::damage` 全仓零命中）—— 不是
//! 偷懒，是这个交换链模型的限制。
//!
//! # 真正省下来的东西
//!
//! [`crate::paint::render`] 与 `quad::expand` 在本模块出现之前是**每帧无条件**跑的
//! （`out.clear()` + 全树递归）。实测（demo，45 节点）四阶段合计 162 µs：
//!
//! | 阶段 | 耗时 | 干净帧能否省 |
//! |---|---|---|
//! | view | 41 µs | ✗ —— 必须跑才知道有没有变（应用代码） |
//! | diff | 8 µs | ✗ —— 同上 |
//! | paint | 73 µs | **✓** |
//! | expand | 40 µs | **✓** |
//!
//! 省掉后 162 → 49 µs，外加一次 present。触发场景很常见：鼠标在空白画布上移动 ——
//! 宿主照旧置脏、走完 diff，但 `hovered` 前后都是 `None`，`diff` 零产出，于是
//! [`FrameOutcome::Unchanged`]，宿主连 present 都不用做。
//!
//! 剩下的 49 µs 是地板：除非应用自己复用声明树（那是架构改动），否则
//! `view()` 必须跑才能知道有没有变。

use crate::context::Context;
use crate::core::Rect;
use crate::diff::{diff, DiffStats};
use crate::element::Element;
use crate::node::{clear_dirty, Dirty, Node};
use crate::paint::DisplayList;

/// 本帧需要重画的范围。
///
/// 消费方只有 base，且**只能**用于帧内 scissor —— 见模块文档里 flip model 那段。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Damage {
    /// 整帧。无法收窄时的保守答案。
    Full,
    /// 一个矩形（绝对坐标、DIP）。
    Rect(Rect),
}

impl Damage {
    /// 是否覆盖整帧。
    ///
    /// 宿主据此决定要不要调 scissor —— `Full` 时设全屏与不设等价，而设了要多一次
    /// API 调用与一次状态变化。
    pub fn is_full(&self) -> bool {
        matches!(self, Damage::Full)
    }
}

/// 一帧的结果。
///
/// # 为什么 [`Unchanged`] 是独立变体而不是「列表为空」
///
/// 因为宿主要据此**整帧跳过**，包括不 present。返回空列表会诱导宿主去
/// 「渲染一个空帧」—— 而那在 flip model 下会把未定义的后缓冲推上屏。
#[derive(Debug)]
pub enum FrameOutcome<'a> {
    /// 与上次交给宿主的 `DisplayList` **逐字节相同** —— 宿主可以什么都不做。
    Unchanged,
    /// 列表已更新（可能是整树重建，也可能是若干子树重算）。
    Updated {
        /// 本帧要画的命令。
        list: &'a DisplayList,
        /// 对账统计。
        stats: DiffStats,
        /// 本帧脏区，供宿主做帧内 scissor。
        damage: Damage,
    },
}

/// 跨帧存活的保留树 + `DisplayList`。
///
/// # 为什么 `DisplayList` 也归它管
///
/// 因为「这一帧画什么」与「上一帧画了什么」是同一个问题的两面。分开放两个地方，
/// 就必然出现「列表是新的、树是旧的」这种组合，而那种组合没法判断能不能跳过。
///
/// 稳态零分配：[`DisplayList::clear`] 保留容量，所以 [`Self::frame`] 在稳态下
/// 不新增分配。
// `Node` 不派生 `Debug`（它持 `PropertyStore`，而那里面有用户自定义值，
// 打印出来没有可读性）。要打印整个缓存用 `node().map(|n| n.tag())`。
#[derive(Default)]
pub struct FrameCache {
    /// 保留树。`None` = 还没建过树（首帧）。
    node: Option<Node>,
    /// 本帧的绘制命令。
    list: DisplayList,
    /// 上次布局用的视口（DIP）。
    ///
    /// 尺寸变了就必须重排 —— 视口不属于任何节点的属性，所以 diff **察觉不到**
    /// 窗口尺寸变化。这是它必须被记下来的唯一原因。
    viewport: (f32, f32),
    /// 强制下一帧重渲（见 [`Self::invalidate`]）。
    forced: bool,
    /// 每帧在 diff 之后、布局之前被调用的样式注入点。
    ///
    /// ⚠️ 用 `Box<dyn Styler>` 而不是泛型参数：宿主**持有**这个 cache，
    /// 而它可能要在运行期换样式表（热重载）。泛型会把「换样式表」变成
    /// 「换掉整个 cache 的类型」，那做不到。
    styler: Option<Box<dyn crate::styler::Styler>>,
    /// diff 的跨帧复用缓冲。见 [`crate::diff::Scratch`] 的说明。
    ///
    /// 没有它时，子节点对账每个父节点每帧要开两个 `Vec`，且每个子节点被搬 3 次。
    diff_scratch: crate::diff::Scratch,
}

impl FrameCache {
    /// 空缓存。首帧 [`Self::frame`] 会建树。
    pub fn new() -> Self {
        Self::default()
    }

    /// 保留树，供命中测试用（[`crate::hit`] / [`crate::input`] 都按节点树走）。
    ///
    /// 首帧之前是 `None` —— 命中测试那时无几何可测，与今天宿主侧
    /// `self.node.as_ref()` 的判据一致。
    pub fn node(&self) -> Option<&Node> {
        self.node.as_ref()
    }

    /// 可变地取保留树，给「布局后直接调整已存在的子树」用
    /// （见 [`crate::node::Node::children_mut`]）。
    ///
    /// # ⚠ 改了之后必须自己标脏
    ///
    /// 绕过 `diff` 的改动**不留痕**，所以调用方有责任
    /// [`crate::node::Node::mark_dirty`]。这不是本模块的要求，而是保留式模型的
    /// 固有性质：框架不可能猜到外部改动过。
    ///
    /// 这也正是 [`Self::frame`] 的跳过判据**不用** [`DiffStats::is_noop`] 的原因
    /// —— 那条判据看不见这里的改动。
    ///
    /// 首帧之前是 `None`（还没有树可改）。
    pub fn node_mut(&mut self) -> Option<&mut Node> {
        self.node.as_mut()
    }

    /// 上一帧的 `DisplayList`（不含本帧）。
    ///
    /// 主要给测试与诊断用；宿主该读 [`FrameOutcome`] 里那份。
    pub fn list(&self) -> &DisplayList {
        &self.list
    }

    /// 强制下一次 [`Self::frame`] 重渲，即便没有任何节点被标脏。
    ///
    /// # 为什么需要它
    ///
    /// [`FrameOutcome::Unchanged`] 的含义是「列表与上次交给宿主的逐字节相同」，
    /// 宿主据此跳过绘制**与 present**。但有一帧可能没被成功推上屏 —— 窗口在
    /// 两次 present 之间被 resize、device removed、或者宿主主动决定重来一次。
    /// 那时若不打断，下一帧 core 仍会说 `Unchanged`，宿主就永远不画了 ——
    /// 窗口停在空白，而**任何计数器都正常**。
    ///
    /// 这是纯 CPU 的一次赋值，不涉及 GPU，所以任何时候都能安全调用。
    pub fn invalidate(&mut self) {
        self.forced = true;
    }

    /// 装一个样式注入点（每帧 diff 之后、布局之前调用）。
    ///
    /// # 为什么必须立刻 invalidate
    ///
    /// 新装上的 styler 还没跑过，所以树上**没有**它那份样式。若这一帧正好是
    /// 稳态（`Unchanged`），宿主会整帧跳过，那份样式就要等到下一次别的事件
    /// 才出现 —— 症状是「改了样式表，顿一下才生效」，而那个「顿一下」
    /// 取决于用户接下来点什么，完全不可预测。
    pub fn set_styler(&mut self, styler: Box<dyn crate::styler::Styler>) {
        self.styler = Some(styler);
        self.invalidate();
    }

    /// 推进一帧：声明 → 对账 → （必要时）布局 → 降级。
    ///
    /// `viewport` 是窗口的 DIP 尺寸。根节点由布局**铺满**它，所以它同时也是
    /// 「窗口尺寸变了没有」的判据。
    ///
    /// 返回 [`FrameOutcome::Unchanged`] 时，本帧**不做**任何绘制数据变更 ——
    /// 上一帧的 [`Self::list`] 仍然有效，且与宿主已经推上屏的那份一致。
    pub fn frame(
        &mut self,
        element: &Element,
        ctx: Context<'_>,
        viewport: (f32, f32),
    ) -> FrameOutcome<'_> {
        let first = self.node.is_none();

        // ---- 1. 声明 → 保留 ----
        //
        // 首帧走 `Node::from_element` + 一次 `diff`，与今天宿主侧的写法逐字相同。
        let stats = match &mut self.node {
            None => {
                let mut n = Node::from_element(element);
                let s = diff(&mut n, element);
                self.node = Some(n);
                s
            }
            Some(node) => crate::diff::diff_with(node, element, &mut self.diff_scratch),
        };

        // ---- 1.5 应用样式 ----
        //
        // ⚠️ **必须在 diff 之后、布局之前**，这是硬契约：
        // diff 在 tag 变化时整个替换节点，替换会把 `Styled` 层一起丢掉 ——
        // 排在 diff 之前等于白写；排在布局之后，改尺寸的样式要等下一帧。
        if let (Some(styler), Some(node)) = (self.styler.as_mut(), self.node.as_mut()) {
            styler.apply(node);
        }

        // ---- 2. 要不要重排 ----
        //
        // 四个来源：首帧、视口变了、**样式刚写了布局键**、`diff` 说要重排。
        //
        // ⚠ 第三项是本节新增的，也正是「为什么只有 resize 才重排」的答案：
        //
        // `apply` 排在 **diff 之后**（硬契约，见 `zephyra-style/src/apply.rs`
        // 模块文档），所以 `stats.needs_layout()` 算的是 apply **之前**的树 ——
        // 它对「hover/press 改了 padding / width / collapsed」一无所知。
        //
        // 于是这类几何变化走的是：`apply` 写出新值 ⇒ `set_styled_prop` 标
        // `Dirty::Layout` ⇒ 第 3 步的 `scan` 看到有脏 ⇒ **重绘**，但**没重排**。
        // 症状是「hover 改了尺寸，而布局盒还是旧的」—— 字形按新 padding 画、
        // 命中测试按旧矩形判，两者都对不上。鼠标移动只翻分类名，所以这条路径
        // 以前只能靠 resize 或整树改动来触发。
        //
        // 判据改成直接读树上的脏标记：谁标了 `Dirty::Layout` 就要重排，
        // 不问它是 diff 标的还是 apply 标的。
        let resized = !first && self.viewport != viewport;
        let mut dirty_rects: Vec<Rect> = Vec::new();
        let (need_layout, some_dirty) = match self.node.as_ref() {
            // 首帧 / 视口变了 ⇒ 一定重排，也就不必再遍历。
            Some(_) if first || resized => (true, true),
            Some(node) => {
                let layout_dirty = tree_needs_layout(node);
                (
                    layout_dirty || stats.needs_layout(),
                    scan(node, ctx, &mut dirty_rects) || layout_dirty,
                )
            }
            // 树建不起来（`element` 与 `Node::from_element` 抛不出东西，纯防御）。
            None => (true, true),
        };
        if need_layout {
            if let Some(node) = self.node.as_mut() {
                crate::layout::layout(
                    node,
                    Rect::new(0.0, 0.0, viewport.0, viewport.1),
                    ctx,
                );
            }
        }

        // ---- 3. 能不能整帧跳过 ----
        //
        // ⚠ 判据**不是** `stats.is_noop()`。那条判据漏掉一条路径：
        // `Node::children_mut()` 明确允许宿主直接改子树、绕过 diff，那种改动
        // 不留痕，除非调用方自己 `mark_dirty`。
        //
        // 所以真实判据是「树里有没有节点的 dirty 不是 None」。`Dirty` 派生自
        // `Ord` 且 `None` 是最小值，所以 `!= None` 等价于 `>= Dirty::Paint`。
        let forced = std::mem::take(&mut self.forced);

        if !forced && !some_dirty {
            // 稳态：树没变、列表还是上次那份。宿主可整帧跳过，连 present 都省。
            return FrameOutcome::Unchanged;
        }

        // ---- 4. 脏区 ----
        //
        // 收窄只在**确定没东西被移走**时才是对的：脏矩形之外的像素必须已经被
        // 上一帧画对，否则就会留下残影。所以四种情况一律退让成 `Full`：
        //
        // | 情况 | 为什么不能收窄 |
        // |---|---|
        // | 重排过 | 节点会移动/收缩，**旧**位置必须重画而我们没有旧矩形快照 |
        // | 结构增删/移动 | 被删节点留下的像素没人负责重画 |
        // | 有脏节点被折叠 | 折叠让它**不再产出命令**，上一帧的命令要抹掉 |
        //
        // 「重排 → Full」省掉的是整个「布局前给每节点快照绝对矩形」的机制。
        // 代价是窗口内任何重排都退化成整帧 —— 而重排本来就意味着大面积变化，
        // 那时脏矩形也帮不上多少。真正受益的是纯交互帧（hover / 按下 / 改色），
        // 它们不重排。
        let structural = stats.inserted > 0 || stats.removed > 0 || stats.moved > 0;
        let damage = if need_layout || structural || dirty_rects.is_empty() {
            Damage::Full
        } else {
            Damage::Rect(union(&dirty_rects))
        };

        // ---- 5. 降级 + 清脏 ----
        //
        // 顺序不能反：脏标记是 `damage` 的**输入**，清掉了就再也算不出矩形。
        if let Some(node) = self.node.as_mut() {
            crate::paint::render(node, ctx, &mut self.list);
            clear_dirty(node);
        }
        self.viewport = viewport;

        FrameOutcome::Updated {
            list: &self.list,
            stats,
            damage,
        }
    }
}

/// 扫一遍树，收集脏节点的矩形。
///
/// 返回「是否需要重绘」。**被折叠的子树不递归** —— `paint::render_rec` 遇到
/// 折叠节点会提前返回，它的子树不产出任何命令，所以子树里的脏标记与绘制无关。
///
/// ⚠ 折叠节点**自己**若被标脏，只贡献 `true`、不贡献矩形。理由是它一旦不再产出
/// 命令，宿主的 scissor 就不该圈住它那块区域 —— 圈了会被 clear 掉却没有命令去填，
/// 表现为一个颜色正确的空洞。
///
/// # 这条分支是防御性的，没有测试覆盖
///
/// 走不到的原因：`COLLAPSED` 在 `property::name_affects_layout` 的名单里，所以
/// 「一个节点从展开变成折叠」必然经由 `diff` 标成 `Dirty::Layout` ⇒
/// `need_layout` 为真 ⇒ [`Self::frame`] 直接短路，**根本不会调用 `scan`**。
///
/// 要抵达它只能靠 [`Self::node_mut`] + 手动 `mark_dirty(Paint)` 且不改属性，
/// 而那样构造出来的场景（一个自始至终折叠的节点被标脏）重绘与否在
/// `DisplayList` 上**看不出差别** —— 它从来没有过命令。
///
/// 实测：把这里的 `return dirty` 改成 `return false`，15 条测试仍然全绿。
/// 与其放一条恒真的测试，不如把这件事写清楚 —— 后来者会以为这里被验过了。
/// 树上**任一**节点是否标了 `Dirty::Layout` 及以上。
///
/// # 为什么需要它 —— 「为什么只有 resize 才更新布局」的答案
///
/// `FrameCache::frame` 的重排判据原先只有三项：首帧、视口变了、`diff` 说要重排。
/// 而 `apply` 排在 diff **之后**，它写出的布局键（`padding` / `width` /
/// `collapsed`…）会经 [`Node::set_styled_prop`] 标上 `Dirty::Layout`，
/// 但那个判据**不读树上的脏标记**，于是只重绘、不重排。
///
/// 鼠标移动只翻分类名 ⇒ hover 改了 padding 而布局盒还是旧的 ⇒ 字形按新 padding
/// 画、命中测试按旧矩形判，两套几何。改成直接读树之后，任何来源标了
/// `Dirty::Layout` 都会重排 —— 包括 hover / press 驱动的几何变化。
///
/// 不提前返回：与 [`scan`] 同理，早退会漏掉兄弟子树里的布局脏节点。
fn tree_needs_layout(node: &Node) -> bool {
    if node.dirty().needs_layout() {
        return true;
    }
    node.children().iter().any(tree_needs_layout)
}

fn scan(node: &Node, ctx: Context<'_>, out: &mut Vec<Rect>) -> bool {
    let dirty = node.dirty() != Dirty::None;
    if node.is_collapsed(ctx) {
        return dirty;
    }
    if dirty {
        // `Node::rect()` 是**绝对**坐标（`layout::arrange` 从根的 (0,0,w,h) 往下传
        // 绝对矩形），所以不需要向上累加偏移。
        out.push(node.rect());
    }
    // ⚠ 不能在这里提前返回：早退会漏掉后面兄弟子树里的脏节点，于是脏矩形偏小 ——
    // 症状是那一块留着上一帧的旧颜色（残影），而所有计数器都正常。
    //
    // （这条是有测试的：把并集换成"只取第一个矩形"，`多处脏时脏区是并集` 立刻失败。）
    let mut any = dirty;
    for child in node.children() {
        any |= scan(child, ctx, out);
    }
    any
}

/// 矩形并集。
fn union(rects: &[Rect]) -> Rect {
    let mut it = rects.iter();
    let first = it.next().expect("union 的入参非空（调用点已判）");
    let (mut x0, mut y0) = (first.x, first.y);
    let (mut x1, mut y1) = (first.right(), first.bottom());
    for r in it {
        x0 = x0.min(r.x);
        y0 = y0.min(r.y);
        x1 = x1.max(r.right());
        y1 = y1.max(r.bottom());
    }
    Rect::new(x0, y0, x1 - x0, y1 - y0)
}

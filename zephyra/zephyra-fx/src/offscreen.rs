//! 离屏目标规划：把"哪些命令需要离屏、要多大离屏纹理"算出来。
//!
//! # 职责边界
//!
//! 本模块**只算尺寸**，不碰任何 GPU 对象、不调任何 D3D API、不分配纹理。
//! 判据很简单：**能不能在没有 GPU 的机器上写单测并断言输出**。
//!
//! 这条边界带来两个实际好处：
//!
//! 1. CPU 参考光栅器（`cpu_raster`）能复用同一份尺寸计算，于是无 GPU 环境
//!    也能验证"离屏区域算得对不对"。
//! 2. 离屏尺寸是**效果正确性**的前提：算小了，模糊/阴影在边界处被硬切，
//!    症状是"效果的边缘有一道看不见的直边"——那种 bug 在真机上极难定位，
//!    而在这里是一行assertion。
//!
//! # 为什么不需要配对的层标记
//!
//! [`zephyra_core::paint::DisplayList`] 构造时即按 `(layer, z, order)` **稳定排序**。
//! `PushLayer` / `PopLayer` 这类配对标记靠"在有序流里前后夹住内容"生效，可排序
//! 会把中间内容插到别的 `layer` 上去，配对当场失效且不报错。
//!
//! 所以本模块走**按内容几何**的路子：每条命令自己算自己的离屏区域。这与 Skia
//! 一致 —— 它也从不录save/restore，而是靠 `SkCanvas::aboutToDraw` →
//! `AutoLayerForImageFilter`（`SkCanvasPriv.h:129`）在回放时按需开离屏，尺寸
//! 来自 `paint.computeFastBounds(drawBounds)`（`SkCanvasPriv.cpp:308`）。
//!
//! 与我们不同的地方是：Skia 那个 API 在 GPU 侧，尺寸算在 device 里；我们把它
//! 放在纯 CPU 层，所以能单测。

use zephyra_core::core::Rect;
use zephyra_core::filter::FilterChain;
use zephyra_core::paint::{Command, DisplayList, SortKey, command_content_rect, command_filter};

/// 一帧的回放计划：普通段与离屏段按 `DisplayList` 顺序**交替**。
///
/// # 为什么放在这里而不是两个后端各写一份
///
/// 这是"合成顺序必须等于绘制顺序"这条规则的**唯一**实现。两边各切一次的话，
/// 早晚有一边把离屏段排到了末尾 —— 而症状只是"某个元素的阴影被卡片背景盖住了"，
/// 不报任何错。
#[derive(Debug, Clone, PartialEq)]
pub enum Segment {
    /// 一段**不含**滤镜命令的普通图元（`DisplayList` 下标区间，半开）。
    ///
    /// 空区间也会产出（见 [`plan_segments`]），因为那样"段数 = 滤镜命令数 + 1"
    /// 是一条可以直接断言的不变量。
    Plain(std::ops::Range<usize>),
    /// 一条要走离屏 pass 的命令（按 `DisplayList` 下标定位）。
    Offscreen {
        /// 在 `DisplayList` 里的下标。
        index: usize,
        /// 已经算好的计划（避免二次解析）。
        pass: OffscreenPass,
        /// 合成之后要不要把原命令再照常画一遍（投影为 `true`）。
        redraw: bool,
        /// 原命令的排序键 —— 造单条列表时要它，否则离屏内容的 `SortKey` 会是
        /// 编造的默认值，而 `sort()` 的稳定性依赖真实键。
        key: SortKey,
    },
}

/// 把一帧切成 [`Segment`] 序列。
///
/// # 为什么空段也要产出
///
/// 不产出空段的话，"整帧只有一条滤镜命令"会得到 `vec![Offscreen]`，段数与滤镜
/// 命令数的关系变成"有时 +1 有时不 +"—— 而那条关系正是测试里最方便断言的不变量。
/// 产出空段换来的是 `segments.len() == 带滤镜命令数 + 1` 恒成立。
pub fn plan_segments(list: &DisplayList) -> Vec<Segment> {
    let mut out: Vec<Segment> = Vec::new();
    let mut start = 0usize;
    for (i, item) in list.iter().enumerate() {
        let Some(pass) = plan_pass(&item.command) else {
            continue;
        };
        out.push(Segment::Plain(start..i));
        out.push(Segment::Offscreen {
            index: i,
            redraw: pass.redraw_original,
            pass,
            key: item.key,
        });
        start = i + 1;
    }
    out.push(Segment::Plain(start..list.len()));
    out
}

/// 一张离屏纹理的规划。
///
/// `atlas` 那类共享资源不在此处——本模块只管**效果**离屏，纹理复用策略属于
/// 资源管理（对应 [`zephyra_core::text::atlas`] 里的淘汰策略那一层）。
#[derive(Debug, Clone, PartialEq)]
pub struct OffscreenPlan {
    /// 要开离屏的命令在 [`DisplayList`] 中的下标。
    pub command_index: usize,
    /// RT 要覆盖的区域（已含滤镜 outset）。
    pub bounds: Rect,
    /// 内容在 RT 内的局部原点。
    ///
    /// 几何坐标仍是屏幕坐标，所以宿主必须把 `bounds.x/y` 减掉才能定位 ——
    /// 这就是 [`zephyra_core::filter::FilterChain::origin_in`] 的用处。
    pub origin: zephyra_core::core::Point,
    /// 内容区未外扩的原始矩形。
    pub content: Rect,
    /// 该离屏承载的滤镜链。
    pub filter: FilterChain,
}

/// 规划整帧的离屏目标，按 DisplayList 顺序返回。
///
/// # 复用：相同矩形会被合并成一条
///
/// 两条命令若离屏矩形**完全相同**（常见于同一个卡片上的背景 + 文本都带同一
/// 个阴影），合并成一张 RT。判据是矩形相等而不是"重叠" —— 重叠合并需要
/// 面积包含判断，而两张 RT 恰好重叠时复用错误的后果是画面错位，收益
/// （省一张小 RT）远不抵风险。
pub fn plan_offscreens(list: &DisplayList) -> Vec<OffscreenPlan> {
    let mut out: Vec<OffscreenPlan> = Vec::new();
    for (i, item) in list.iter().enumerate() {
        let Some((filter, content)) = split_filter_and_content(&item.command) else {
            continue;
        };
        if filter.is_identity() {
            continue;
        }
        let bounds = filter.bounds_for(content);
        let origin = filter.origin_in(content);
        // 与已有的一张完全相同就复用 —— 典型场景是同一张卡片的背景与文本
        // 共用一个投影。
        if let Some(existing) = out.iter_mut().find(|p| p.bounds == bounds) {
            if existing.filter == *filter {
                continue;
            }
        }
        out.push(OffscreenPlan {
            command_index: i,
            bounds,
            origin,
            content,
            filter: filter.clone(),
        });
    }
    out
}

/// 取出命令的滤镜与内容矩形；`None` = 该命令不参与效果离屏。
///
/// 转发到 `paint` 里的那一份实现 —— 那里同时被
/// [`zephyra_core::paint::DisplayList::offscreen_requirements`] 用着。
/// 两处各写一遍的话，漏掉某个命令变体的后果是"那个变体的滤镜区域算小一圈"，
/// 症状是模糊在边界被硬切，**且不报任何错**。
fn split_filter_and_content(cmd: &Command) -> Option<(&FilterChain, Rect)> {
    Some((command_filter(cmd)?, command_content_rect(cmd)?))
}

/// 本帧有多少条命令的混合模式不是默认值。
///
/// 混合模式**不**产生离屏需求 —— 它改的是混合方程，不是"要多少像素"。
/// 绝大多数模式能在现有 target 上换混合状态完成（Skia 的 `GrBlendMode` 就是
/// 这么用的）。但它需要 PSO 变体，所以宿主要知道有没有。
pub fn count_non_default_blends(list: &DisplayList) -> usize {
    list.iter()
        .filter(|i| {
            let b = match &i.command {
                Command::Quad { blend, .. }
                | Command::Path { blend, .. }
                | Command::Image { blend, .. }
                | Command::Text { blend, .. } => *blend,
                Command::PushClip(_) | Command::PopClip => {
                    zephyra_core::filter::BlendMode::default()
                }
            };
            b != zephyra_core::filter::BlendMode::default()
        })
        .count()
}

// ═══════════════════════════════════════════════════════════════════════════
//  E4.4 / E4.5 / E4.6：离屏 pass 的**执行**计划
// ═══════════════════════════════════════════════════════════════════════════
//
//  上面 [`OffscreenPlan`] 回答"要开几张 RT、每张多大"，本节回答"每张 RT 上要做
//  哪几趟、每趟参数是什么、结果贴回哪里"。两者分开的理由与模块顶部那条边界
//  相同：这里仍然**一个 GPU 对象都不碰**，所以能被无设备环境单测断言。
//
//  分开的实际收益是：模糊趟的方向与顺序一旦在 D3D11 与 D3D12 的 `renderer.rs`
//  里各写一遍，早晚有一边顺序反了，而症状只是"某个元素的阴影糊的方向不对"，
//  不报任何错。现在两个后端只剩"照着计划设状态、画一个铺满四边形"。
//
//  ── 一趟离屏渲染长什么样 ──
//
//    ┌───────────── 目标（屏幕）───────────────┐
//    │  …普通图元… │ …普通图元… │ …普通图元…  │
//    └────────────────────────────────────────┘
//                    ↑ 遇到带滤镜的命令（分段回放）
//    1. 建 / 复用 RT_A（尺寸 = bounds 向上取整），清成全 0
//    2. 把该命令的几何按 `translate` 平移进 RT_A，逐段回放
//    3. 每一趟模糊：铺满四边形 → PSBlurH / PSBlurV，A⇄B **乒乓**
//    4. 铺满四边形 → PSComposite，把结果贴回目标的 `bounds` 处
//    5. 若 `redraw_original`（投影），再把原命令照常画一遍 —— 影子在下、原图在上
//
//  ── 乒乓只需要两张 RT ──
//
//  链长任意（E4.6）：每一趟都"读一张写另一张"，写的那张就是下一趟的源。所以池子
//  里固定两张就够，与链长无关。

/// 一次离屏渲染的**执行**计划。
///
/// 全部字段都是值，没有引用 —— 计划要跨后端、跨测试地存活。
#[derive(Debug, Clone, PartialEq)]
pub struct OffscreenPass {
    /// 离屏纹理尺寸（像素，整数）。
    ///
    /// 取 `bounds` 的**向上取整**：`bounds` 是像素空间的浮点矩形，而 RT 只能是
    /// 整数像素。向下取整会把最后不足一个像素的那条边切掉，症状是模糊在
    /// 右/下边缘少一列/一行 —— 只在 σ 大到肉眼可见时才显形。
    pub rt_size: (u32, u32),
    /// 离屏纹理左上角在**屏幕坐标**里的位置（= `bounds` 的左上角）。
    pub origin: (f32, f32),
    /// 离屏纹理覆盖的屏幕矩形（= `bounds`）。
    pub bounds: Rect,
    /// 画进离屏 RT 之前，几何要平移的量（像素）= `-origin + offset`。
    ///
    /// `offset` 来自 `DropShadow`：影子是"剪影模糊后**整体**平移"，所以平移必须
    /// 在**模糊之前**做。模糊之后再挪，会把已经算好的模糊边缘一起挪出 RT ——
    /// 症状是影子的某一侧凭空少一圈模糊。
    pub translate: (f32, f32),
    /// 依次执行的模糊趟，按顺序应用。
    pub blurs: Vec<BlurStep>,
    /// 合成阶段的染色色；`None` = 原样透传。
    ///
    /// 只有投影会给出 Some（"剪影 → 模糊 → 染色"三步里的第三步）。同一条链里
    /// 出现多次投影时**后者覆盖前者**：链语义是从左到右依次执行，而染色是就地
    /// 替换色相，后一次的结果覆盖前一次 —— 与「就地替换」而不是「两次合成」一致。
    pub tint: Option<zephyra_core::core::Color>,
    /// 原命令是否要在离屏合成**之后**照常再画一遍。
    ///
    /// 只有投影为 `true`：影子是**额外**画上去的一层，原图本身仍要画在它上面。
    /// 纯模糊相反 —— 离屏结果**就是**这条命令的最终形态；再画一次原图等于
    /// "模糊 + 清晰"叠在一起，视觉上完全看不出模糊。
    pub redraw_original: bool,
}

/// 一次模糊趟。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlurStep {
    /// `true` = 水平（H），`false` = 垂直（V）。
    ///
    /// # 为什么拆成两个方向而不是一个"任意方向"
    ///
    /// 拆开之后每趟对应**一个** PS 入口（`PSBlurH` / `PSBlurV`），方向不必走
    /// 常量缓冲也不必挤进 `Vertex` —— 而 `Vertex` 的每个字段都已被正常图元占满。
    /// 若合成一个入口，方向就得进 cbuffer（D3D12 还要动根签名）。
    pub horizontal: bool,
    /// 标准差（像素），**不是**半径。着色器内部自己乘 3 得到半径。
    ///
    /// 存 σ 而不是半径是为了与 [`zephyra_core::filter::Filter`] 的字段同名同义
    /// —— 中间任何一步做了重复的 `×SIGMA_TO_RADIUS` 换算，误差会直接体现在
    /// 模糊宽度上。
    pub sigma: f32,
}

/// 为一条命令算出执行计划；`None` = 它不需要离屏。
///
/// 「不需要」的三种情况：
///
/// 1. 滤镜链恒等（空链，或每一项都恒等）—— 见
///    [`FilterChain::is_identity`]。为恒等链开离屏会让每帧多两次全 RT 模糊。
/// 2. 命令没有内容矩形（`PushClip` / `PopClip`）。
/// 3. 内容矩形为空或非有限。模糊一张空图得到的是全透明 RT，合成上去什么也
///    不改变，只是白白花掉两趟全 RT 模糊；非有限值则更糟 —— 它会一路穿过
///    `ceil` 变成天文数字的尺寸，症状是"程序卡死"而不是"某个元素少画了"。
pub fn plan_pass(cmd: &Command) -> Option<OffscreenPass> {
    let filter = command_filter(cmd)?;
    if filter.is_identity() {
        return None;
    }
    let content = command_content_rect(cmd)?;
    if !(content.width > 0.0 && content.height > 0.0) {
        return None;
    }
    if !(content.x.is_finite()
        && content.y.is_finite()
        && content.width.is_finite()
        && content.height.is_finite())
    {
        return None;
    }
    Some(pass_for_chain(filter, content))
}

/// 按内容矩形与滤镜链算计划。[`plan_pass`] 的纯计算内核，测试可直接调。
pub fn pass_for_chain(filter: &FilterChain, content: Rect) -> OffscreenPass {
    let bounds = filter.bounds_for(content);
    let origin = (bounds.x, bounds.y);

    let mut blurs: Vec<BlurStep> = Vec::new();
    let mut tint = None;
    let mut offset = zephyra_core::core::Point::new(0.0, 0.0);
    let mut redraw_original = false;

    // `filters[0]` 最先执行 —— 见 `FilterChain` 的顺序语义。
    for f in filter.filters() {
        match f {
            zephyra_core::filter::Filter::Blur { sigma_x, sigma_y } => {
                push_blur(&mut blurs, true, *sigma_x);
                push_blur(&mut blurs, false, *sigma_y);
            }
            zephyra_core::filter::Filter::DropShadow {
                color,
                offset: o,
                sigma,
            } => {
                // `color` 进 `tint`（染色），`offset` 进 `translate`（整体平移）。
                // ⚠ 两者都**必须**在模糊趟**之前**定下来：染色发生在合成阶段，
                // 平移发生在画内容阶段 —— 顺序反了的话，影子会带着 blur 半径一起
                // 挪出去（某一侧凭空少一圈模糊）。
                push_blur(&mut blurs, true, *sigma);
                push_blur(&mut blurs, false, *sigma);
                offset.x += o.x;
                offset.y += o.y;
                tint = Some(*color);
                redraw_original = true;
            }
        }
    }

    // 向上取整，且至少 1×1：视口为 0 时 D3D11 **不报错**，只是什么都不画。
    let rt_size = (
        (bounds.width.ceil() as u32).max(1),
        (bounds.height.ceil() as u32).max(1),
    );

    OffscreenPass {
        rt_size,
        origin,
        bounds,
        translate: (-origin.0 + offset.x, -origin.1 + offset.y),
        blurs,
        tint,
        redraw_original,
    }
}

/// σ ≤ 0 的趟是恒等，不入列表。
///
/// # 为什么不留给着色器判
///
/// 着色器里 `BlurRadius(σ) = 0` 时循环只跑 k=0、权重 1，结果确实等于原值 ——
/// 但那是**两次全 RT 的 draw call + 一次完整的纹理往返**。`Filter::Blur{0,0}`
/// 在语义上是恒等，链里出现它多半是"σ 由上层算出来恰好是 0"的正常情况，
/// 不该为它付这个钱。
fn push_blur(out: &mut Vec<BlurStep>, horizontal: bool, sigma: f32) {
    if sigma.is_finite() && sigma > 0.0 {
        out.push(BlurStep { horizontal, sigma });
    }
}

/// 模糊 / 合成 pass 用的**铺满四边形**（`kind == kind::BLUR`）。
///
/// # 顶点字段怎么复用
///
/// 着色器对 `kind == 4` 的四边形有一套**与正常图元完全不同**的字段约定
/// （见 `zephyra.hlsl` 里 `PSBlurH` 上方那张表）：
///
/// | 字段       | 这里填                     |
/// |------------|----------------------------|
/// | `local`    | 该顶点在**源 RT** 里的像素坐标 |
/// | `half_size`| **源 RT** 的尺寸（四顶点同值） |
/// | `corner`   | `(方向, σ)` —— `.y` 是 σ     |
/// | `uv`       | 像素坐标 ÷ 源 RT 尺寸        |
/// | `color`    | 染色色（仅合成 pass 用）     |
///
/// `local` 原点在左上角、y 向下，与纹理 uv 一致 —— 所以 `uv = local / src_size`
/// 天然正确，不需要再翻一次 y。（再翻一次是这类代码最常见的错，而症状只是
/// "模糊方向上下颠倒"。）
///
/// # 几何铺满**目标**，采样**源**
///
/// 两者尺寸可以不同（源可能是上一趟乒乓过来的那张 RT）。`local` 必须跟着
/// **源**走、`position` 必须跟着**目标**走 —— 混了就是"采样坐标整体缩放"，
/// 症状是模糊变成一团糊在错误位置的东西。
pub fn pass_quad(
    dst_size: (f32, f32),
    src_size: (f32, f32),
    corner: [f32; 2],
    color: [f32; 4],
) -> [crate::quad::Vertex; 4] {
    let w = dst_size.0.max(1e-6);
    let h = dst_size.1.max(1e-6);
    let sw = src_size.0.max(1e-6);
    let sh = src_size.1.max(1e-6);
    let nx = |x: f32| x / w * 2.0 - 1.0;
    let ny = |y: f32| 1.0 - y / h * 2.0;
    let x0 = nx(0.0);
    let x1 = nx(dst_size.0);
    let y0 = ny(0.0);
    let y1 = ny(dst_size.1);
    // `half_size` 在 HLSL 里是 `nointerpolation`：四个顶点必须填**同一个**值，
    // 否则插值出的"源 RT 尺寸"随位置漂移，模糊的归一化坐标随之漂移 —— 症状
    // 是画面一侧正常、另一侧糊成斜纹。
    let half = [sw, sh];
    let v = |position: [f32; 2], local: [f32; 2]| crate::quad::Vertex {
        position,
        local,
        uv: [local[0] / sw, local[1] / sh],
        color,
        corner,
        half_size: half,
        kind: crate::quad::kind::BLUR,
    };
    [
        v([x0, y0], [0.0, 0.0]),
        v([x1, y0], [sw, 0.0]),
        v([x1, y1], [sw, sh]),
        v([x0, y1], [0.0, sh]),
    ]
}

/// 一次模糊趟的铺满四边形。
pub fn blur_quad(step: &BlurStep, dst_size: (f32, f32), src_size: (f32, f32)) -> [crate::quad::Vertex; 4] {
    pass_quad(
        dst_size,
        src_size,
        [f32::from(step.horizontal), step.sigma],
        [0.0, 0.0, 0.0, 1.0],
    )
}

/// 合成 pass 的铺满四边形：采样源 RT 并贴到**屏幕上的 `dst`**。
///
/// `tint` 走 `color`（着色器按 `corner.y > 0` 判是否染色），所以 `corner.y`
/// 这里就是"染色浓度"的存在标记 —— 传 0 或 `tint.a` 都对，但**不能传 NaN**，
/// 否则 `if (tintAmount <= 0.0)` 对 NaN 为 false，于是走进染色分支算出一个
/// 全 NaN 的像素。
pub fn composite_quad(
    dst_screen: Rect,
    dst_size: (f32, f32),
    src_size: (f32, f32),
    tint: Option<zephyra_core::core::Color>,
) -> [crate::quad::Vertex; 4] {
    // `pass_quad` 铺满**目标**，而这里的源是 RT（尺寸通常与目标不同）。先把
    // 四边形的几何挪到屏幕矩形，再把采样尺寸换成源的 —— 两个尺寸必须分别
    // 处理，见 [`pass_quad`] 的文档。
    let mut v = pass_quad(
        (dst_screen.width, dst_screen.height),
        src_size,
        [0.0, tint.map_or(0.0, |c| c.a)],
        match tint {
            // 顶点色在 `Vertex::quad` 里会被预乘；这里**故意**传非预乘值，
            // 因为 `PSComposite` 只用 `i.color.rgb` 当色相、并用 `t.a * i.color.a`
            // 自己算覆盖率（见那个函数里"染色不能整体乘 alpha"的注释）。
            Some(c) => [c.r, c.g, c.b, c.a],
            None => [0.0, 0.0, 0.0, 1.0],
        },
    );
    // 几何：按屏幕矩形重算 NDC（目标尺寸 = `dst_size`），因为合成四边形只覆盖
    // 离屏纹理对应的那块屏幕区域，而不是铺满整个目标。
    let w = dst_size.0.max(1e-6);
    let h = dst_size.1.max(1e-6);
    let (x0, x1) = (
        dst_screen.x / w * 2.0 - 1.0,
        (dst_screen.x + dst_screen.width) / w * 2.0 - 1.0,
    );
    let (y0, y1) = (
        1.0 - dst_screen.y / h * 2.0,
        1.0 - (dst_screen.y + dst_screen.height) / h * 2.0,
    );
    v[0].position = [x0, y0];
    v[1].position = [x1, y0];
    v[2].position = [x1, y1];
    v[3].position = [x0, y1];
    v
}

/// 把只含 `cmd` 的新列表准备好，让调用方 `expand` 出局部几何。
///
/// # 为什么要临时造一个单条列表，而不是给 `expand` 加"只处理第 N 条"的参数
///
/// `expand` 内部按命令类型分派，加筛选参数就要在**每一个**分派点重复判断，
/// 漏一处就是"某种命令偷偷被画两遍"或"漏画"。造一个单条列表则完全复用既有
/// 路径，筛选只发生一次。
///
/// 代价是一次 `Command` 克隆，而最重的字段 `Arc<[Vertex]>` 克隆只是引用计数 +1。
pub fn single_command_list(
    cmd: &Command,
    key: zephyra_core::paint::SortKey,
) -> DisplayList {
    let mut out = DisplayList::new();
    out.push(key, cmd.clone());
    out
}

impl OffscreenPass {
    /// 离屏纹理尺寸转成 `f32` 对。
    ///
    /// # 为什么专门给一个方法而不是调用处写 `(w as f32, h as f32)`
    ///
    /// 因为 `blur_quad` 的第三个参数是**源**尺寸，而 `pass_quad` 里 `local` 与
    /// `half_size` 都由它推导 —— 两处都写一遍 `as f32` 的话，某一处忘了写就会
    /// 让归一化坐标差一个数量级，症状是"模糊变成斜纹"。
    pub fn rt_size_as_f32(&self) -> (f32, f32) {
        (self.rt_size.0 as f32, self.rt_size.1 as f32)
    }
}
#[cfg(test)]
#[path = "tests/offscreen_tests.rs"]
mod tests;
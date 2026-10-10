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
use zephyra_core::paint::{Command, DisplayList, command_content_rect, command_filter};

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
mod tests {
    use super::*;
    use zephyra_core::core::{Color, CornerRadius, Point};
    use zephyra_core::filter::{BlendMode, Filter};
    use zephyra_core::paint::{Layer, SortKey};

    // ── E4.4 / E4.5 / E4.6：执行计划 ──

    fn blur_chain(sx: f32, sy: f32) -> FilterChain {
        FilterChain::single(Filter::Blur {
            sigma_x: sx,
            sigma_y: sy,
        })
    }

    /// 水平趟必须在垂直趟**之前**，且 σ 互不串位。
    ///
    /// # 变异目标
    ///
    /// 把 `push_blur` 的两行顺序对调（或把 `sigma_x`/`sigma_y` 写反）时本条
    /// 必须失败。否则症状只是"某元素的模糊宽窄方向反了"，在 demo 上很难看出。
    #[test]
    fn blur_becomes_two_steps_horizontal_first() {
        let p = pass_for_chain(&blur_chain(2.0, 3.0), Rect::new(10.0, 20.0, 30.0, 40.0));
        assert_eq!(
            p.blurs,
            vec![
                BlurStep {
                    horizontal: true,
                    sigma: 2.0
                },
                BlurStep {
                    horizontal: false,
                    sigma: 3.0
                },
            ],
            "σx / σy 是分别给的，顺序或串位都等于换了滤镜"
        );
    }

    #[test]
    fn zero_sigma_steps_are_dropped() {
        let p = pass_for_chain(&blur_chain(0.0, 0.0), Rect::new(0.0, 0.0, 10.0, 10.0));
        assert!(
            p.blurs.is_empty(),
            "σ=0 的趟是恒等，不该付两次全 RT 的 draw"
        );
        // 而恒等链整体压根不该开离屏。
        let id = quad(
            blur_chain(0.0, 0.0),
            BlendMode::SrcOver,
            Rect::new(0.0, 0.0, 10.0, 10.0),
        );
        assert!(plan_pass(&id).is_none());
    }

    /// 投影 = 染色 + 偏移 + 额外重画原图。
    ///
    /// `translate` 是这条测试的判据核心：平移必须在**模糊之前**，否则影子某一侧
    /// 会凭空少一圈模糊（而不是整体位置偏一点）。
    #[test]
    fn drop_shadow_tints_redraws_and_shifts_inside_the_offscreen() {
        let c = Rect::new(100.0, 100.0, 40.0, 40.0);
        let chain = FilterChain::single(Filter::DropShadow {
            color: Color::rgba(0.0, 0.0, 0.0, 0.5),
            offset: Point::new(6.0, 8.0),
            sigma: 2.0,
        });
        let p = pass_for_chain(&chain, c);
        assert!(p.redraw_original, "影子是额外一层，原图仍要画在它上面");
        assert_eq!(p.tint, Some(Color::rgba(0.0, 0.0, 0.0, 0.5)));
        assert_eq!(p.blurs.len(), 2);
        // outset.left = max(3σ - offset.x, 0) = max(6-6, 0) = 0 ⇒ origin.x = 100。
        assert_eq!(p.origin, (100.0, 100.0), "origin 决定平移量，算错等于整体错位");
        assert_eq!(
            p.translate,
            (-94.0, -92.0),
            "translate = -origin + offset；漏掉 offset 会让影子画在原位"
        );
    }

    /// 链里三道效果 ⇒ 六趟，且顺序与 σ 序列**逐项**对上。
    ///
    /// # 变异目标
    ///
    /// 任何对 `blurs` 排序、去重、或截断到前两项的实现都会在这里露馅。
    #[test]
    fn a_three_effect_chain_is_six_steps_in_chain_order() {
        let chain = FilterChain::new([
            Filter::Blur {
                sigma_x: 1.0,
                sigma_y: 1.0,
            },
            Filter::Blur {
                sigma_x: 2.0,
                sigma_y: 2.0,
            },
            Filter::DropShadow {
                color: Color::rgba(0.0, 0.0, 0.0, 1.0),
                offset: Point::new(0.0, 0.0),
                sigma: 1.0,
            },
        ]);
        let p = pass_for_chain(&chain, Rect::new(0.0, 0.0, 8.0, 8.0));
        assert_eq!(p.blurs.len(), 6, "三道效果各两趟");
        assert_eq!(
            p.blurs.iter().map(|b| b.sigma).collect::<Vec<_>>(),
            vec![1.0, 1.0, 2.0, 2.0, 1.0, 1.0],
            "趟的 σ 必须按 filters[0] 起的顺序，不能被排序或去重"
        );
        // 三道里最后一道是投影，所以要重画原图。
        assert!(p.redraw_original);
    }

    /// RT 尺寸向上取整，且永不塌成 0。
    ///
    /// # 变异目标
    ///
    /// 把 `.ceil()` 改成 `.floor()`（或 `.round()`）时本条必须失败 —— 症状是模糊
    /// 在右/下边缘少掉不足一个像素的一条，而那只在 σ 大到肉眼可见时才显形。
    #[test]
    fn rt_size_rounds_up_and_never_collapses_to_zero() {
        // σ=1 ⇒ outset = 3。内容 0.2×0.2 ⇒ bounds 6.2×6.2 ⇒ 7×7（不是 6×6）。
        let frac = pass_for_chain(&blur_chain(1.0, 1.0), Rect::new(0.0, 0.0, 0.2, 0.2));
        assert_eq!(frac.rt_size, (7, 7), "bounds 有小数时必须进位，否则切掉一条边");
        // 整数边界不该被多进一位（否则白白多分配一整行/一整列显存）。
        let exact = pass_for_chain(&blur_chain(1.0, 1.0), Rect::new(0.0, 0.0, 10.0, 20.0));
        assert_eq!(exact.rt_size, (16, 26), "bounds 已是整数时不该多进");
        let tiny = pass_for_chain(&blur_chain(1.0, 1.0), Rect::new(0.0, 0.0, 0.001, 0.001));
        assert!(
            tiny.rt_size.0 >= 1 && tiny.rt_size.1 >= 1,
            "0×0 的视口什么都不画且不报错"
        );
    }

    /// 非有限矩形必须被挡住 —— 否则它会穿过 `ceil` 变成天文数字的纹理尺寸。
    ///
    /// # 变异目标
    ///
    /// 删掉 `plan_pass` 里的 `is_finite` 检查时本条必须失败。
    #[test]
    fn a_non_finite_rect_yields_no_plan() {
        for bad in [
            Rect::new(f32::NAN, 0.0, 10.0, 10.0),
            Rect::new(0.0, 0.0, f32::INFINITY, 10.0),
        ] {
            let cmd = quad(blur_chain(2.0, 2.0), BlendMode::SrcOver, bad);
            assert!(plan_pass(&cmd).is_none(), "{bad:?} 必须在这里就被挡住");
        }
    }

    /// 空内容矩形不���离屏 —— 模糊一张空图得到的是全透明，纯浪费两趟全 RT。
    #[test]
    fn an_empty_content_rect_yields_no_plan() {
        let cmd = quad(
            blur_chain(2.0, 2.0),
            BlendMode::SrcOver,
            Rect::new(5.0, 5.0, 0.0, 10.0),
        );
        assert!(plan_pass(&cmd).is_none());
    }

    /// pass 四边形的**采样坐标按源尺寸**归一化，几何铺满目标。
    #[test]
    fn pass_quad_samples_the_source_and_covers_the_destination() {
        let v = blur_quad(
            &BlurStep {
                horizontal: true,
                sigma: 2.0,
            },
            (100.0, 100.0), // 目标
            (40.0, 60.0),    // 源
        );
        assert_eq!(
            (v[0].position, v[2].position),
            ([-1.0, 1.0], [1.0, -1.0]),
            "几何必须铺满目标"
        );
        assert_eq!(v[2].local, [40.0, 60.0], "local 必须是源 RT 里的像素坐标");
        assert!(
            (v[2].uv[0] - 1.0).abs() < 1e-6 && (v[2].uv[1] - 1.0).abs() < 1e-6,
            "右下角的 uv 必须是 (1,1) —— 归一化按**源**尺寸"
        );
        for vert in &v {
            assert_eq!(
                vert.half_size,
                [40.0, 60.0],
                "`half_size` 在 HLSL 里是 nointerpolation：四顶点不一致就插出斜纹"
            );
            assert_eq!(vert.kind, crate::quad::kind::BLUR);
        }
        assert!((v[0].corner[1] - 2.0).abs() < 1e-6, "σ 走 corner.y");
    }

    /// 合成四边形落在**屏幕矩形**上，且染色标志与颜色都对。
    #[test]
    fn composite_quad_lands_on_the_screen_rect() {
        let dst = Rect::new(10.0, 20.0, 40.0, 30.0);
        let v = composite_quad(dst, (200.0, 200.0), (40.0, 30.0), None);
        // NDC 角点：(10,20) → 左上，(50,50) → 右下。
        assert!((v[0].position[0] - (10.0 / 200.0 * 2.0 - 1.0)).abs() < 1e-5);
        assert!((v[0].position[1] - (1.0 - 20.0 / 200.0 * 2.0)).abs() < 1e-5);
        assert!((v[2].position[0] - (50.0 / 200.0 * 2.0 - 1.0)).abs() < 1e-5);
        assert!((v[2].position[1] - (1.0 - 50.0 / 200.0 * 2.0)).abs() < 1e-5);
        assert_eq!(v[0].corner[1], 0.0, "无染色时标志必须是 0（NaN 会走进染色分支）");
        // 采样尺寸仍是源 RT，与目标无关。
        assert_eq!(v[2].half_size, [40.0, 30.0]);

        let tinted = composite_quad(dst, (200.0, 200.0), (40.0, 30.0), Some(Color::rgba(0.0, 0.0, 0.0, 0.5)));
        assert_eq!(tinted[0].corner[1], 0.5, "染色浓度即 tint.a");
        assert_eq!(
            tinted[0].color,
            [0.0, 0.0, 0.0, 0.5],
            "顶点色必须**非预乘** —— PSComposite 自己用 `t.a * i.color.a` 算覆盖率"
        );
    }

    fn quad(filter: FilterChain, blend: BlendMode, rect: Rect) -> Command {
        Command::Quad {
            rect,
            radius: CornerRadius { radius: 0.0 },
            fill: Color::rgb(1.0, 1.0, 1.0),
            stroke: None,
            filter,
            blend,
        }
    }

    fn blur(s: f32) -> FilterChain {
        FilterChain::single(Filter::Blur {
            sigma_x: s,
            sigma_y: s,
        })
    }

    fn path(filter: FilterChain, blend: BlendMode, pts: &[(f32, f32)]) -> Command {
        Command::Path {
            vertices: std::sync::Arc::from(
                pts.iter()
                    .map(|&(x, y)| zephyra_core::paint::Vertex {
                        position: Point::new(x, y),
                        uv: Point::new(0.0, 0.0),
                    })
                    .collect::<Vec<_>>(),
            ),
            fill: Color::rgb(1.0, 1.0, 1.0),
            filter,
            blend,
        }
    }

    fn list_of(cmds: Vec<Command>) -> DisplayList {
        let mut l = DisplayList::new();
        for (i, c) in cmds.into_iter().enumerate() {
            l.push(
                SortKey {
                    layer: Layer::Content,
                    z: 0,
                    order: i as u32,
                },
                c,
            );
        }
        l.sort();
        l
    }

    /// 无滤镜时不产生任何离屏需求 —— 这是最常见的路径，必须零开销。
    #[test]
    fn 无滤镜不产生离屏需求() {
        let l = list_of(vec![
            quad(FilterChain::none(), BlendMode::default(), Rect::new(0.0, 0.0, 10.0, 10.0)),
            quad(blur(0.0), BlendMode::default(), Rect::new(0.0, 0.0, 10.0, 10.0)),
        ]);
        assert!(
            plan_offscreens(&l).is_empty(),
            "空链与 σ=0 的blur 都恒等，不该开RT"
        );
    }

    /// blur 的 RT 尺寸 = 内容 + 两侧 3σ。
    #[test]
    fn blur_离屏矩形外扩三倍标准差() {
        let l = list_of(vec![quad(
            blur(2.0),
            BlendMode::default(),
            Rect::new(100.0, 50.0, 40.0, 20.0),
        )]);
        let plans = plan_offscreens(&l);
        assert_eq!(plans.len(), 1);
        assert_eq!(
            plans[0].bounds,
            Rect::new(94.0, 44.0, 52.0, 32.0),
            "上下左右各扩 3 × 2 = 6"
        );
        assert_eq!(plans[0].content, Rect::new(100.0, 50.0, 40.0, 20.0));
    }

    /// 局部原点必须等于「bounds 左上角」，否则几何会被画偏。
    ///
    /// 这是最关键的一条：几何坐标是屏幕坐标，宿主靠 `origin` 才知道要把
    /// 内容往哪儿挪。origin 算错的症状是「效果整体偏移了 3σ」，极像模糊
    /// 参数不对，会把人引到完全错误的方向。
    ///
    /// ⚠ 方向容易搞反：RT 向**外**扩，所以屏幕原点落在 RT 内的**负**坐标上。
    /// 宿主该做的是 `screen − origin`，而不是 `screen + origin`。
    #[test]
    fn 局部原点就是离屏矩形左上角() {
        let l = list_of(vec![quad(
            blur(4.0),
            BlendMode::default(),
            Rect::new(37.0, 11.0, 20.0, 20.0),
        )]);
        let p = &plan_offscreens(&l)[0];
        assert_eq!((p.origin.x, p.origin.y), (p.bounds.x, p.bounds.y));
        assert_eq!((p.origin.x, p.origin.y), (25.0, -1.0), "12 = 3 × 4");
        // 内容左上角在 RT 内的局部坐标 = content − origin，正好是外扩量。
        assert_eq!(p.content.x - p.origin.x, 12.0);
        assert_eq!(p.content.y - p.origin.y, 12.0);
    }

    /// 相同矩形 + 相同滤镜 → 复用一张 RT。
    ///
    /// 典型场景是同一张卡片的背景与文本共用一个投影：两块内容矩形不同，
    /// 但**离屏矩形**可能相同（内容都在同一张卡片的投影范围内）。
    #[test]
    fn 相同离屏矩形会被合并() {
        // 两个不同矩形，但外扩后恰好同尺寸 → 不会被合并（矩形不等）。
        let a = quad(blur(2.0), BlendMode::default(), Rect::new(0.0, 0.0, 10.0, 10.0));
        let b = quad(blur(2.0), BlendMode::default(), Rect::new(0.0, 0.0, 10.0, 10.0));
        let l = list_of(vec![a, b]);
        assert_eq!(
            plan_offscreens(&l).len(),
            1,
            "同矩形同滤镜应合并成一张 RT"
        );
        // 不同矩形则各开一张。
        let l2 = list_of(vec![
            quad(blur(2.0), BlendMode::default(), Rect::new(0.0, 0.0, 10.0, 10.0)),
            quad(blur(2.0), BlendMode::default(), Rect::new(20.0, 0.0, 10.0, 10.0)),
        ]);
        assert_eq!(plan_offscreens(&l2).len(), 2);
    }

    /// 投影的不对称外扩被完整反映到 RT 尺寸上。
    #[test]
    fn 投影离屏矩形包含偏移() {
        let f = FilterChain::single(Filter::DropShadow {
            color: Color::rgba(0.0, 0.0, 0.0, 0.4),
            offset: Point::new(4.0, 6.0),
            sigma: 2.0,
        });
        let l = list_of(vec![quad(f, BlendMode::default(), Rect::new(0.0, 0.0, 100.0, 50.0))]);
        let p = &plan_offscreens(&l)[0];
        assert_eq!(p.bounds, Rect::new(-2.0, 0.0, 112.0, 62.0));
    }

    /// 混合模式不产生离屏需求 —— 它改的是混合方程，不是像素数量。
    #[test]
    fn 混合模式不产生离屏需求() {
        let l = list_of(vec![quad(
            FilterChain::none(),
            BlendMode::Multiply,
            Rect::new(0.0, 0.0, 10.0, 10.0),
        )]);
        assert!(plan_offscreens(&l).is_empty());
        assert_eq!(count_non_default_blends(&l), 1);
    }

    /// 文本命令的离屏范围是**所有字形的并集**，不是第一个字形。
    ///
    /// 逐字形模糊会把每个字糊成独立一块，视觉上完全不是"这段文字被模糊了"。
    #[test]
    fn 文本离屏范围是全部字形的并集() {
        let g = |x: f32| zephyra_core::paint::GlyphQuad {
            dst: Rect::new(x, 10.0, 8.0, 12.0),
            uv: zephyra_core::core::UvRect::FULL,
            color_glyph: false,
        };
        let cmd = Command::Text {
            atlas: zephyra_core::paint::TextureRef::new("a"),
            glyphs: vec![g(0.0), g(100.0), g(50.0)],
            color: Color::rgb(0.0, 0.0, 0.0),
            filter: blur(1.0),
            blend: BlendMode::default(),
        };
        let l = list_of(vec![cmd]);
        let p = &plan_offscreens(&l)[0];
        // 并集是 x∈[0,108]、y∈[10,22]；再各外扩 3。
        assert_eq!(
            p.content,
            Rect::new(0.0, 10.0, 108.0, 12.0),
            "取并集：最右字形到 108，最宽 ≠ 单字形宽"
        );
        assert_eq!(p.bounds, Rect::new(-3.0, 7.0, 114.0, 18.0));
    }

    /// 空字形的文本不产生离屏需求（内容矩形是零尺寸，RT 无意义）。
    #[test]
    fn 空文本不产生离屏需求() {
        let cmd = Command::Text {
            atlas: zephyra_core::paint::TextureRef::new("a"),
            glyphs: vec![],
            color: Color::rgb(0.0, 0.0, 0.0),
            filter: blur(2.0),
            blend: BlendMode::default(),
        };
        let l = list_of(vec![cmd]);
        let plans = plan_offscreens(&l);
        assert_eq!(plans.len(), 1, "命令本身带滤镜，所以仍有一条");
        assert_eq!(
            plans[0].content,
            Rect::new(0.0, 0.0, 0.0, 0.0),
            "内容是零尺寸 —— 宿主据此可以跳过开 RT"
        );
    }

    /// 排序不拆散效果 —— 离屏需求按 DisplayList 顺序产出，索引可用。
    ///
    /// 这条是对"配对命令会被排序拆散"那个论断的正面反证：改成按内容
    /// 几何后，跨 layer 的命令也不会错位。
    #[test]
    fn 离屏需求的下标能定位回原命令() {
        let a = quad(FilterChain::none(), BlendMode::default(), Rect::new(0.0, 0.0, 5.0, 5.0));
        let b = quad(blur(1.0), BlendMode::default(), Rect::new(10.0, 10.0, 20.0, 20.0));
        let l = list_of(vec![a, b]);
        let plans = plan_offscreens(&l);
        assert_eq!(plans.len(), 1);
        let target = l.iter().nth(plans[0].command_index).unwrap();
        assert!(
            matches!(&target.command, Command::Quad { filter, .. } if !filter.is_identity()),
            "第{} 条应是带滤镜的命令，实际是 {:?}",
            plans[0].command_index,
            target.command
        );
    }

    /// 带滤镜的 Path 产出离屏需求，区域 = **顶点的紧包围盒** + outset。
    ///
    /// 这是「矢量图标 + 投影被静默忽略」那个缺口的回归测试：以前这里直接返回
    /// `None`，于是 `quad::expand` 照常画图标、只是不开离屏，效果没发生也不报错。
    #[test]
    fn blurred_path_yields_offscreen_with_tight_bounds() {
        // ⚠ 顶点顺序是刻意选的：**首顶点在内部**，后两个才是包围盒的对角。
        // 于是「拿首顶点当 min/max」「只遍历一部分」「忘了 min」这几类实现都会
        // 算出不同的数 —— 这个测试对它们敏感。
        let cmd = path(
            blur(2.0),
            BlendMode::default(),
            &[(20.0, 30.0), (10.0, 20.0), (33.0, 47.0)],
        );
        let plans = plan_offscreens(&list_of(vec![cmd]));
        assert_eq!(plans.len(), 1, "带滤镜的 Path 必须开离屏，否则效果被静默忽略");
        // 紧包围盒 = (10,20)–(33,47)。刻意不是任何"整数对齐"的矩形，
        // 这样 x/w 与 y/h 必须**分别**由 min 与 max 得出，混了就会露馅。
        assert_eq!(
            plans[0].content,
            Rect::new(10.0, 20.0, 23.0, 27.0),
            "content 必须是顶点紧包围盒，而不是布局矩形"
        );
        // σ=2 ⇒ 四边各扩 3σ=6。
        assert_eq!(
            plans[0].bounds,
            Rect::new(4.0, 14.0, 35.0, 39.0),
            "bounds = content + outset：23+12 / 27+12"
        );
    }

    /// 无滤镜的 Path 不开离屏 —— 这是图标最常见的路径，必须零开销。
    #[test]
    fn unfiltered_path_yields_no_offscreen() {
        let l = list_of(vec![
            path(
                FilterChain::none(),
                BlendMode::default(),
                &[(0.0, 0.0), (10.0, 10.0)],
            ),
            path(blur(0.0), BlendMode::default(), &[(0.0, 0.0), (10.0, 10.0)]),
        ]);
        assert!(
            plan_offscreens(&l).is_empty(),
            "恒等链与 σ=0 的 Path 都不该开 RT，否则每个图标每帧多一次 RT 绑定"
        );
    }

    /// 空顶点列表给出**零尺寸**而不是崩掉。
    ///
    /// 这里刻意不断言"不产生需求"：空 Text 走的是同一条路（`union_of_quads`
    /// 同样返回零矩形），要改就得两层一起改、且连带 Text/Quad/Image 的语义。
    /// 真正的零尺寸护栏属于**执行侧**（E4 绑 RT 时），届时对所有命令类型统一处理，
    /// 而不是在这里给 Path 单独开一个特例。
    #[test]
    fn empty_path_gives_zero_sized_bounds() {
        let plans = plan_offscreens(&list_of(vec![path(
            blur(2.0),
            BlendMode::default(),
            &[],
        )]));
        assert_eq!(plans.len(), 1);
        assert_eq!(
            plans[0].content,
            Rect::new(0.0, 0.0, 0.0, 0.0),
            "空顶点 = 零尺寸内容，调用方自然开不出 RT"
        );
    }
}
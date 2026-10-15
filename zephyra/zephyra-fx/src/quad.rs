//! `DisplayList` → 顶点流：与 GPU 后端无关的那一半。
//!
//! # 为什么单独拆出来
//!
//! 这层做的事是"把绘制命令翻译成四边形顶点"：算裁剪交、把圆角/描边拆成独立的图元、
//! 按纹理把图像命令分批。它**不碰任何 GPU 对象**，所以：
//!
//! - 可以在没有窗口、没有 GPU 的机器上单测（`examples` 里的
//!   `demo_interaction` 就是靠这条链路做端到端断言的）；
//! - 换后端（D3D11 → Vulkan → 软件光栅）时**一字不用改**，因为它只输出
//!   [`Vertex`] 流，后端只负责把流喂进 GPU。
//!
//! 换句话说：这个模块是"画什么"与"怎么画"之间那道缝，换渲染后端不该碰到它。
//!
//! # 批处理策略
//!
//! 全部命令合批进**单个** render pass、单个 pipeline、单个顶点缓冲：
//!
//! - 所有绘制都是「带圆角的四边形」—— 圆角与描边在片元着色器内以 SDF 求出，
//!   不做 CPU 三角化（这正是核心选 `Quad` 图元而非路径的原因）；
//! - 圆角矩形、图像、文本在 GPU 上都是同一批四边形，因此按图元类型排序即可合批，
//!   不必按颜色分批 —— 颜色进了顶点缓冲。
//!
//! 代价是 `Command::Path`（任意多边形）逃不掉 CPU 三角化 —— SDF 需要解析距离场，
//! 而任意路径没有。矢量图标走的就是这条路：`zephyra_core::geom` 把它展平成三角形后，
//! 本模块逐三角形做裁剪并写入顶点流。抗锯齿只能靠 MSAA（没有距离场可解析）。
//!
//! 它顶点数不是 4 的倍数，所以索引规则与四边形那段不同，单独成段、单独一次 draw。
//! [`RenderReport::skipped`] 只在**真正失败**时非零 —— 例如全部顶点退化。
//!
//! # 坐标
//!
//! 核心用左上角原点、DIP 为单位；NDC 是左下角原点、单位 NDC。转换在
//! [`Vertex::quad`] 里做一次：
//!
//! ```text
//! ndc_x = rect.x / width  * 2 - 1
//! ndc_y = 1 - rect.y / height * 2      ← y 翻转
//! ```
//!
//! **像素与 DIP 的区别必须在这一层处理**：顶点携带的是 DIP，渲染器用目标像素
//! 尺寸把 DIP 映射到 NDC，于是同一个 `DisplayList` 在 100% 与 200% 缩放下
//! 尺寸相同、内容清晰 —— 因为栅格化发生在物理像素上。

use zephyra_core::core::{Color, Point, Rect, UvRect};
use std::rc::Rc;
use zephyra_core::paint::{Command, DisplayList};

/// 每顶点 52 字节：位置(2) + 局部(2) + uv(2) + 颜色(4) + 圆角描边(2) + 类型(1) 个 f32。
///
/// 圆角与描边必须走顶点而不是常量缓冲：同一批里每个四边形的圆角半径和描边宽度
/// 都不同。走常量就得每换一个矩形重设一次并重录命令，破坏合批。
///
/// 52 是算出来的（13 个 f32），不是凑的 —— 但仍要显式断言，因为 `Vertex` 是
/// `Pod`，布局一旦与着色器输入描述符对不上，症状是**画面整体错位**而不是报错。
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    /// NDC 位置。
    pub position: [f32; 2],
    /// 图元内局部坐标（DIP 空间的像素坐标，供 SDF 用）。
    pub local: [f32; 2],
    /// 纹理 uv（`kind >= 1` 时有效）。
    pub uv: [f32; 2],
    /// 线性 RGBA。
    pub color: [f32; 4],
    /// x = 圆角半径（DIP），y = 描边宽度（DIP）。
    pub corner: [f32; 2],
    /// 四边形的**半宽半高**（与 `local` 同单位）。
    ///
    /// # 它必须是「扩后四边形」的一半，而不是「矩形」的一半
    ///
    /// 片元里算的是 `p = local - half_size`，而 `local` 的原点在**扩后**四边形的
    /// 左上角（见 [`Vertex::quad`]）。要让矩形真实边界落在 `d == 0` 上，
    /// `half_size` 必须与扩后几何对齐——否则 `d == 0` 会偏移，描边环带也会长错位置。
    ///
    /// # 为什么必须有这个通道 —— 之前没有它，SDF 是坏的
    ///
    /// 着色器原先这样算：
    ///
    /// ```hlsl
    /// float2 half_size = i.local * 0.5;
    /// float2 center    = i.local - half_size;
    /// float d = sd_round_rect(center, half_size, i.corner.x);
    /// ```
    ///
    /// `half_size` 是从**逐片元**的 `local` 推的，所以它逐片元变化。而 `local` 恰好
    /// 满足 `center == half_size`，于是 `sd_round_rect` 里
    /// `q = abs(center) - half_size + r = r` —— **`d` 与位置无关**，退化成常数：
    ///
    /// | 四边形 | `d` | 结果 |
    /// |---|---|---|
    /// | 方角（r=0），任意位置 | 0 | alpha = 0.5，**整块均匀半透明，没有形状** |
    /// | 圆角 12px，任意位置 | 4.97 | alpha = 0，**完全不可见** |
    ///
    /// demo 的卡片与按钮都用 `CORNER_RADIUS = 12`，所以它们在 GPU 上整块消失。
    ///
    /// `half_size` 是**整个四边形的常量**属性。加一个顶点通道传它，四个顶点填**同一个
    /// 值** —— 线性插值两端相等，中间就恒等于它。这不是取巧，而是「常量属性的常量
    /// 插值」本来就是这么定义的。
    pub half_size: [f32; 2],
    /// 图元类型：0 = SDF 圆角矩形，1 = 字形图集采样，2 = 图像采样。
    ///
    /// 三类共用一个 pipeline 与顶点布局，靠这个逐顶点的标量在片元里分支。
    /// 分支是"每片元一次"而不是"每 draw call 一次" —— 后者才该用多 pipeline。
    /// 这里的代价可以忽略：图元都是小四边形，且同一批内 `kind` 通常相同，
    /// 分支预测器几乎总命中。
    pub kind: f32,
}

/// 抗锯齿过渡带的半宽（DIP），与 `zephyra.hlsl` 的 `float px = 0.75` 必须相等。
///
/// # 为什么四边形要按它向外扩
///
/// SDF 的过渡带跨 `d ∈ [-px, +px]`。而填充与描边的**真实边界都在 `d == 0`**，
/// 若四边形恰好等于矩形，则片元只能落在矩形**内**，即恒有 `d ≤ 0` ——
/// `d ∈ (0, px]` 那半边过渡带被光栅化器裁掉，**所有边缘都是硬边**。
///
/// 扩出去之后四边形边界落在 `d == ±px`，`smoothstep` 的两端才都在可见范围内。
/// Skia 的 `SkRRect` 靠 analytic AA 的 bounds outset 达成同一件事。
///
/// ⚠ 改这里必须同步改 `zephyra.hlsl` 的 `px`，否则过渡带宽度与几何余量脱节。
pub const AA_BAND: f32 = 0.75;

/// 图元类型 `kind` 的取值。
///
/// # 与着色器阈值的配对关系
///
/// `shaders/zephyra.hlsl` 的 `PSMain` 用两个阈值把图元分成三类：
///
/// ```hlsl
/// if (i.kind > 0.5 && i.kind < 1.5) { /* 字形 */ }
/// if (i.kind > 1.5)                 { /* 图像 */ }
/// /* 否则                            圆角矩形 */
/// ```
///
/// 所以三个值必须**有序且相邻间距 < 1**。若 `GLYPH` 取了 2.0，它就落进图像分支
/// —— 症状是**所有文字变成图像采样**（取到图集里别的字形），表现为文字乱码，
/// 而不是"某一个字符不对"。
///
/// 集中成常量而不是散落字面量：改阈值时只有一处要看。两侧的配对由
/// `shader::tests::the_kind_values_land_in_disjoint_shader_branches` 钉住。
pub mod kind {
    /// SDF 圆角矩形（纯色 / 描边）。
    pub const RECT: f32 = 0.0;
    /// 字形图集采样。
    pub const GLYPH: f32 = 1.0;
    /// 图像采样（每批绑定一种纹理）。
    pub const IMAGE: f32 = 2.0;
    /// 已 CPU 三角化的任意多边形（纯色，不采样纹理）。
    ///
    /// # 为什么不复用 `IMAGE` 再加一个"无纹理"标志
    ///
    /// 图像分支会 `g_tex.Sample(...)`，而一次 draw call 只能绑一张 SRV。
    /// 让多边形走图像分支，就得为它单独开一批并绑一张占位纹理 —— 为了一个
    /// "直接返回颜色"的分支付出一整次 draw call 与一次 SRV 绑定。
    ///
    /// 它也**不能**复用 `RECT`：那个分支算的是 `sd_round_rect`，而多边形的
    /// 三角形没有矩形这个概念，`half_size` 会被插值成无意义的值，算出来的
    /// alpha 是随位置漂移的噪声 —— 症状是"多边形半透明且边缘发花"。
    ///
    /// 所以它是独立的 `kind`。代价是着色器多一个分支（`> 2.5`），成本为零：
    /// 同一批内 `kind` 恒定，编译器可以把它提升成 uniform 分支。
    pub const PATH: f32 = 3.0;
    /// 离屏 pass 的**铺满四边形**（模糊 / 合成用），见 [`crate::offscreen`]。
    ///
    /// # 为什么不复用前四个
    ///
    /// 前四个值喂给 `PSMain` 的分支，而模糊 pass 喂给 `PSBlurH` / `PSBlurV` /
    /// `PSComposite` —— 那是**三个不同的像素着色器**，各自只有一种图元，根本没有
    /// 分支可言。`kind` 在这里只起一个作用：让调用方一眼看出这批顶点不是正常
    /// 图元，以及在调试打印里区分开。
    ///
    /// 值取 4.0 是因为它必须**大于** [`kind_thresholds::PATH_MIN`]，否则一旦有人
    /// 让模糊四边形也走 `PSMain`，它会落进多边形分支"直接返回顶点色"——
    /// 症状是模糊结果变成一块纯色，且没有任何错误。
    pub const BLUR: f32 = 4.0;
}

/// `zephyra.hlsl` 里 `PSMain` 用来把图元分成三类的那三个阈值。
///
/// # 为什么放在共享层而不是各后端的 `shader.rs`
///
/// 这三个数是 [`kind`] 与 **HLSL 源码**之间的契约。`zephyra.hlsl` 在 D3D11 与
/// D3D12 两边是**逐字节相同**的一份文件，所以契约也只有一份 —— 把它复制进两个
/// `shader.rs` 就等于造出两处可以各自漂移的地方。
///
/// # 配对关系
///
/// ```hlsl
/// if (i.kind > 0.5 && i.kind < 1.5) { /* 字形 */ }
/// if (i.kind > 1.5)                 { /* 图像 */ }
/// /* 否则                            圆角矩形 */
/// ```
///
/// 三个 `kind` 值（0.0 / 1.0 / 2.0）必须**有序且相邻间距 < 1**。若 `GLYPH` 取了
/// 2.0，它就落进图像分支 —— 症状是**所有文字变成图像采样**（取到图集里别的字形），
/// 表现为整块错位或乱码，而不是"某一个字符不对"。
pub mod kind_thresholds {
    /// 字形分支的下界（`kind > 0.5`）。
    pub const GLYPH_MIN: f32 = 0.5;
    /// 字形分支的上界（`kind < 1.5`）。
    pub const GLYPH_MAX: f32 = 1.5;
    /// 图像分支的下界（`kind > 1.5`）。
    pub const IMAGE_MIN: f32 = 1.5;
    /// 多边形分支的下界（`kind > 2.5`）。
    ///
    /// ⚠ 这个阈值必须**大于** [`Self::IMAGE_MIN`]，否则多边形会落进图像分支去
    /// 采样纹理 —— 症状是多边形变成"绑在当前批次那张纹理上的采样结果"，
    /// 换个批次颜色就变，看起来像随机噪声而不是形状错误。
    pub const PATH_MIN: f32 = 2.5;
}

/// 一个四边形的索引（两个三角形）。
///
/// 全局共享这一份**模板**，但它不是可以直接交给 GPU 的索引缓冲 —— 每个四边形
/// 要先加上自己的顶点基址，见 [`expand_indices`]。
pub const INDICES: [u16; 6] = [0, 1, 2, 0, 2, 3];

/// 把 `quads` 个四边形展开成可直接提交给 GPU 的索引序列。
///
/// # 为什么必须展开
///
/// 顶点缓冲里 `quads` 个四边形**连续排列**（每个占 4 个顶点），而所有四边形
/// 共用同一个索引模板 `[0,1,2,0,2,3]` + 各自基址。D3D11 的
/// `DrawIndexedInstanced` 只有一个 `BaseVertexLocation`，所以：
///
/// - 用**实例化**画 N 个四边形 → 每个实例都读 `[0,1,2,0,2,3]` + 同一基址
///   → 画出来是 N 份**同一个**四边形；
/// - 想一次画完 N 个四边形 → 索引缓冲里必须有 N 份已加基址的索引。
///
/// # 我踩的坑
///
/// 第一版用了实例化：`DrawIndexedInstanced(6, 135, 0, 0, 0)`，参数看起来完全
/// 合理（135 个图元、6 个索引），但它只画出**第一个**四边形、重复 135 次。
/// 而画面上只有一个全屏背景色，于是\"背景画对了、内容全没了\"这个症状看起来
/// 像"后面的四边形没画"，实际上前面的也一样 —— 135 份重叠的同一个背景。
///
/// 这类错误没有任何 API 会报错：`indexcount`、`instancecount`、`BaseVertexLocation`
/// 全是合法值。唯一的判据是看像素。
///
/// 展开的成本：每个四边形 6 个 `u16`，139 个图元 = 1668 字节，一帧一次
/// `memcpy` 量级。相比顶点流（28KB）可以忽略。
pub fn expand_indices(quads: usize) -> Vec<u16> {
    let mut out = Vec::with_capacity(quads * 6);
    for q in 0..quads {
        let base = (q * 4) as u16;
        out.extend(INDICES.iter().map(|i| i + base));
    }
    out
}

/// 一帧的完整索引流：`quads` 个四边形 + `path_tris` 个多边形三角形。
///
/// # 为什么路径段要单独生成而不是混进四边形段
///
/// 四边形的索引是 `[0,1,2,0,2,3]` + 顶点基址 × 4；多边形三角形是**连续**的
/// `base, base+1, base+2`。两者规则不同，混在一个循环里会算错。
///
/// 两段在索引流里是**首尾相接**的：四边形段在前（索引 `0 .. quads*6`），
/// 路径段在后（索引 `quads*6 .. quads*6 + tris*3`），各自一次 `DrawIndexed`
/// 用 `StartIndexLocation` 定位。返回 `(索引流, 路径段的起始索引)`。
///
/// # 为什么路径段仍然是展开的
///
/// 可以用 `Draw( tris*3, 0 )` 配合"顶点缓冲按绘制偏移"省略索引 —— 但
/// `DrawIndexed` 的 `StartIndexLocation` 与 `BaseVertexLocation` 在
/// [`crate::base`] 那两个后端里都已接好，混用两条路径反而多一处要维护的约定。
/// 索引段的总长是 `quads*6 + tris*3` 个 `u16`，一帧几百字节，可以忽略。
pub fn expand_indices_with_paths(quads: usize, path_tris: usize) -> (Vec<u16>, u32) {
    let mut out = Vec::with_capacity(quads * 6 + path_tris * 3);
    let start = expand_indices_with_paths_into(quads, path_tris, &mut out);
    (out, start)
}

/// [`expand_indices_with_paths`] 的复用版：写进调用方的缓冲，返回路径段起点。
///
/// 与 [`expand_indices_with_paths`] **共用同一份实现** —— 索引规则一旦有两份代码，
/// 漂移的症状是"某些图元画成乱码几何"，而那与"性能优化改坏了渲染"几乎无从区分。
///
/// 目标缓冲会被**清空后重填**（不是追加）—— 调用点想要"追加"时自己接一次即可，
/// 这里不留歧义。
pub fn expand_indices_with_paths_into(
    quads: usize,
    path_tris: usize,
    out: &mut Vec<u16>,
) -> u32 {
    out.clear();
    // 一次性 `reserve` 到位，免得循环里反复判断容量（那条分支在稳态下永远不成立，
    // 但每次迭代都要判）。
    out.reserve(quads * 6 + path_tris * 3);
    for q in 0..quads {
        let base = (q * 4) as u16;
        out.extend(INDICES.iter().map(|i| i + base));
    }
    let path_start = out.len() as u32;
    // 路径段的顶点基址是 `quads * 4` —— 顶点流里 `solid` + `images` 在前。
    let base = (quads * 4) as u16;
    for t in 0..path_tris {
        let b = base + (t * 3) as u16;
        out.push(b);
        out.push(b + 1);
        out.push(b + 2);
    }
    path_start
}

impl Vertex {
    /// SDF 圆角矩形的四个顶点。
    ///
    /// `local` 携带的是**像素空间**的局部坐标 —— SDF 需要知道片元离边角多远，
    /// 而这只有在知道目标物理尺寸时才算得出来。
    pub fn from_rect(rect: Rect, px: (f32, f32), color: [f32; 4], corner: [f32; 2]) -> [Vertex; 4] {
        Self::quad(
            rect,
            px,
            color,
            corner,
            [0.0; 2],
            [0.0, 1.0],
            kind::RECT,
        )
    }

    /// 纹理采样的四个顶点（字形 / 图像）。`uv` 是纹理内的归一化坐标。
    ///
    /// 不加圆角：位图自带抗锯齿的边缘，再套一层圆角 SDF 会把 'o' / 'e' 的边角
    /// 啃掉。图像若要圆角，应当在**上传前**把纹理裁成圆角形状。
    pub fn from_uv_rect(
        rect: Rect,
        px: (f32, f32),
        color: [f32; 4],
        uv: UvRect,
        kind: f32,
    ) -> [Vertex; 4] {
        Self::quad(
            rect,
            px,
            color,
            [0.0, 0.0],
            [uv.x, uv.y],
            [uv.right(), uv.bottom()],
            kind,
        )
    }

    /// 带圆角的图像四边形（`kind == IMAGE`）。
    ///
    /// # 与 [`Self::from_uv_rect`] 的唯一差别：半径 > 0 时外扩 + **uv 外插**
    ///
    /// 位图自带硬边，所以圆角必须由 SDF 决定，于是四边形要像 SDF 矩形那样按
    /// [`AA_BAND`] 向外扩 —— 否则 `d ∈ (0, px]` 那半边过渡带被光栅化器裁掉，
    /// 圆角会是硬边。
    ///
    /// 但**不能**沿用 [`Self::quad`] 那种「把 uv 铺在扩后四边形上」的做法：
    /// 那会让原矩形落在 quad 内缩 `pad` 处，等于**裁掉并放大**一圈图像
    /// （32px 的图裁 4.7%，肉眼可见）。
    ///
    /// 所以这里把 uv **外插**：让线性插值在**未外扩**的 `rect.x / rect.right()`
    /// 处恰好等于 `uv0 / uv1`，多出去的那 `pad` 由采样器的 `Clamp` 兜住。
    ///
    /// # 半径为 0 时**完全退化成** [`Self::from_uv_rect`]
    ///
    /// 不外扩、uv 不动、`corner.x = 0` ⇒ 片元里那个 `corner.x > 0` 的分支不进，
    /// 于是遮罩不施加。
    ///
    /// ⚠️ 这一条不是优化而是**正确性**：即使不外扩，SDF 的 `d == 0` 也恰好落在
    /// 矩形边界，`1 - smoothstep(-px, px, 0) == 0.5` —— 也就是说无条件施加遮罩
    /// 会把最外一圈像素压到 50% alpha，症状是**图像四周发暗一圈**。
    pub fn from_rounded_uv_rect(
        rect: Rect,
        px: (f32, f32),
        color: [f32; 4],
        uv: UvRect,
        radius: f32,
    ) -> [Vertex; 4] {
        // ⚠️ 判据是 `!(radius > 0.0)` 而不是 `radius <= 0.0`，因为**负数截距
        // （partial order）**下两者对 `NaN` 的处理相反：
        //
        // | 写法 | `NaN` 走哪边 |
        // |---|---|
        // | `!(radius > 0.0)` | **走直角**（`NaN > 0` 为 false）|
        // | `radius >= 0.0` | **走圆角**（比较为 false ⇒ 不进入）|
        //
        // 而 `NaN` 半径必须退化成直角 —— 走圆角路会让 `half_size` 与 uv 外插拿到
        // `NaN`，整条几何变 `NaN`、整帧静默消失。变异测试（M5）钉住这一条：
        // 去掉这个 `!` 之后 4 条测试失败。
        #[allow(clippy::neg_cmp_op_on_partial_ord)]
        if !(radius > 0.0) {
            return Self::from_uv_rect(rect, px, color, uv, kind::IMAGE);
        }
        let pad = AA_BAND;
        // uv 外插系数：外扩 `pad` 像素对应多少 uv。
        //
        // 除数用 `rect` 的**原始**尺寸（未外扩）—— 因为我们要的是「原矩形边界处
        // uv 恰好等于 uv0/uv1」。用退化尺寸（0）做除法会得到 ±inf，所以先挡掉。
        let (sx, sy) = (
            if rect.width > 1e-6 {
                pad / rect.width
            } else {
                0.0
            },
            if rect.height > 1e-6 {
                pad / rect.height
            } else {
                0.0
            },
        );
        let (u0, u1, v0, v1) = (uv.x, uv.right(), uv.y, uv.bottom());
        // 采样器是 `Clamp`，所以外插出去的一点点会被夹回边缘，不会取到邻块。
        //
        // ⚠️ `UvRect::new` 的后两个参数是**宽高**而不是 right/bottom —— 传错的话
        // uv 区间会整体偏移，而症状是「图像内容错位一点」，极难归因到构造参数。
        let du = (u1 - u0) * sx;
        let dv = (v1 - v0) * sy;
        let uv_out = UvRect::new(u0 - du, v0 - dv, (u1 - u0) + 2.0 * du, (v1 - v0) + 2.0 * dv);
        Self::quad(
            rect,
            px,
            color,
            [radius, 0.0],
            [uv_out.x, uv_out.y],
            [uv_out.right(), uv_out.bottom()],
            kind::IMAGE,
        )
    }

    /// 通用四边形展开。
    pub fn quad(
        rect: Rect,
        px: (f32, f32),
        color: [f32; 4],
        corner: [f32; 2],
        uv0: [f32; 2],
        uv1: [f32; 2],
        kind: f32,
    ) -> [Vertex; 4] {
        let (w, h) = (px.0.max(1e-6), px.1.max(1e-6));
        // **预乘 alpha**：`color` 进到这里是非预乘的（`Color::rgba` 原样存值），
        // 而整条管线在**预乘空间**里合成。
        //
        // 为什么在这里做，而不是让混合状态去适配非预乘：混合系数表
        // （[`zephyra_core::filter::BlendMode::as_coeff`]）给的是预乘空间的系数
        // （Skia 同样如此）。顶点里若放非预乘颜色，SrcOver 就得用
        // `SRC_ALPHA/INV_SRC_ALPHA`；而一旦要支持 Multiply 这类模式，混合因子就
        // 不再有"先乘一次 alpha"的统一形式 —— 每个模式都得单独推一遍。
        //
        // 顺带把 E4.4/E4.5 的路铺好：模糊 / 投影的离屏合成**必须**在预乘空间里做，
        // 否则半透明区域模糊后边缘发暗（预乘 alpha 的经典坑）。
        //
        // `a == 1` 时是恒等，所以绝大多数不透明图元的顶点一个字节都没变。
        let ca = color[3];
        let color = [color[0] * ca, color[1] * ca, color[2] * ca, ca];
        // `sdf` 类别需要 AA 余量；字形与图像自带边缘，不扩（扩了会把 uv 拉出图块）。
        //
        // 字形位图的抗锯齿是**烘进 alpha 里**的（`ab_glyph` 栅格化时就算好了），
        // 再叠一层 SDF 过渡带会把 'o'/'e' 的边角啃掉；图像同理。所以只有 SDF 分支扩。
        //
        // ⚠️ **带圆角的图像是唯一的例外**：位图的边缘是硬的，圆角改由 SDF 决定，
        // 所以它必须扩 —— 否则过渡带的上半边（`d ∈ (0, px]`）被裁掉，圆角变硬边。
        //
        // 而「扩了会不会裁掉图像内容」在调用方 [`Self::from_rounded_uv_rect`]
        // 里已经用 **uv 外插**解决了，所以这里扩是安全的。
        //
        // 半径为 0 时**不扩**：无条件扩会让直角图像白裁一圈，且片元里那个
        // `corner.x > 0` 的分支虽然不进、外扩却已经改变了几何。
        let pad = match kind {
            k if k == kind::RECT => AA_BAND,
            k if k == kind::IMAGE && corner[0] > 0.0 => AA_BAND,
            _ => 0.0,
        };
        // 几何向外扩 `pad`，于是片元能落到 `d ∈ (0, pad]` —— 过渡带的上半边。
        let gx = rect.x - pad;
        let gy = rect.y - pad;
        let gw = rect.width + pad * 2.0;
        let gh = rect.height + pad * 2.0;
        let nx = |x: f32| x / w * 2.0 - 1.0;
        let ny = |y: f32| 1.0 - y / h * 2.0;
        let (x0, x1) = (nx(gx), nx(gx + gw));
        let (y0, y1) = (ny(gy), ny(gy + gh));
        // `local` 跟随几何：原点在**扩后**四边形的左上角，范围 `[0, gw] × [0, gh]`。
        //
        // 于是矩形真实区域在 local 里是 `[pad, pad + rect.width]`，
        // 它的中心是 `pad + rect尺寸/2` —— 而片元里算的是 `p = local - half_size`，
        // 所以 `half_size` 必须等于**真实半宽 + pad**（见下）。
        let half_size = [gw * 0.5, gh * 0.5];
        // 圆角半径是**真实**几何量（不含 pad）—— 它进的是 `sd_round_rect` 的 `r`，
        // 与四边形扩不扩无关。加 pad 会让圆角在视觉上大一号。
        let v = |position: [f32; 2], local: [f32; 2], uv: [f32; 2]| Vertex {
            position,
            local,
            uv,
            color,
            corner,
            half_size,
            kind,
        };
        [
            v([x0, y0], [0.0, 0.0], uv0),
            v([x1, y0], [gw, 0.0], [uv1[0], uv0[1]]),
            v([x1, y1], [gw, gh], uv1),
            v([x0, y1], [0.0, gh], [uv0[0], uv1[1]]),
        ]
    }

    /// 一个已 CPU 三角化的多边形顶点（[`kind::PATH`]）。
    ///
    /// # 为什么这类图元必须单独成段
    ///
    /// 其余图元的顶点数都是 4 的倍数，索引可以按 [`INDICES`] + 顶点基址机械展开
    /// （见 [`expand_indices`]）。多边形的顶点数是**三角形数 × 3**，与 4 无关 ——
    /// 混进 `solid` 段会让整个索引展开错位（症状：多边形附近的所有图元一起
    /// 变成乱码几何）。
    ///
    /// `local` / `uv` / `half_size` 全部填零：多边形分支在片元里**一个都不读**。
    /// 填零而不是留未初始化 —— 顶点缓冲是 `Pod`，未初始化会让 MSAA 边缘采到
    /// 垃圾字节，正好落在需要精确覆盖率的路径边缘上。
    ///
    /// # 抗锯齿
    ///
    /// 不扩几何、不算距离场：三角化后的多边形没有解析距离，边缘 AA 靠 **MSAA**。
    /// 这与 [`crate::geom`] 的设计约定一致。
    pub fn path(position: [f32; 2], color: [f32; 4]) -> Vertex {
        // 与 [`Self::quad`] 同一个理由：顶点色在**预乘空间**里，见那里的说明。
        let ca = color[3];
        let color = [color[0] * ca, color[1] * ca, color[2] * ca, ca];
        Vertex {
            position,
            local: [0.0; 2],
            uv: [0.0; 2],
            color,
            corner: [0.0; 2],
            half_size: [0.0; 2],
            kind: kind::PATH,
        }
    }
}

/// 一次回放的统计。用于回答「这一帧重算了什么」。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RenderReport {
    /// 实际提交的四边形数（描边环带单独计）。
    pub quads: usize,
    /// 没能画出来的命令数（几何退化、顶点全被裁掉等）。
    ///
    /// ⚠ 稳态下应为 0。要区分「没人用效果」与「效果被静默忽略」，看的是
    /// [`Self::unimplemented_filters`] 与 [`Self::unimplemented_blends`] —— 那两个
    /// 恒为正是当前的常态，而本字段恒为 0 才是正常帧的样子。
    pub skipped: usize,
    /// 因带非恒等 `filter` 而**未执行**的命令数。
    ///
    /// 恒大于 0 说明有人设了效果但 GPU 侧还没接离屏pass —— 画面上表现为
    /// "效果静默消失"（没有任何报错），所以这个计数是唯一的诊断手段。
    ///
    /// 那条命令**本身照常画**（本层只负责展开几何，效果是后端的事），
    /// 但离屏 pass 要等 GPU 侧实现后才存在 —— 届时这里应恒为 0。
    pub unimplemented_filters: usize,
    /// 因带非默认 `blend` 而**未执行**的命令数。
    ///
    /// 与 [`Self::unimplemented_filters`] 同理：混合模式要由后端换混合状态，
    /// 几何展开层看不到它。
    pub unimplemented_blends: usize,
    ///
    /// # 它数的是"**还差多少**"，不是"有几个非默认模式"
    ///
    /// 能用系数对表达的 15 个混合模式（`Clear`..`Screen`）可以直接映射成 D3D 的混合
    /// 因子，不需要片元着色器配合；再往后的 14 个才需要 PS 变体。判据是
    /// [`as_coeff`](zephyra_core::filter::BlendMode::as_coeff)，见
    /// [`blend_is_unimplemented`]。
    /// 本次写入的顶点字节数。
    ///
    /// ⚠ **`expand` 不填这个字段** —— 它由渲染器在顶点流算出来之后写入（见
    /// `Renderer::render`）。所以从 [`expand`] 直接拿到的 `RenderReport` 里它是 0，
    /// 那**不代表**顶点流是空的，只代表这一层还没算。
    ///
    /// 早先这里写的是「本次写入的顶点字节数」，读起来像是 `expand` 会填 —— 于是
    /// `examples/demo.rs --dump-cpu` 打出「顶点字节 0」，看着像"一个字形都没进去"，
    /// 而同一次调用里 `quads = 135`。两个数字摆在一起还互相矛盾。
    ///
    /// 要判断顶点流是否真的非空，用 [`Self::quads`] 或 `QuadBatch::solid.len()`。
    pub vertex_bytes: u64,
    /// 顶点缓冲是否发生了扩容 —— 稳态下应为 `false`。
    pub reallocated: bool,
    /// 本帧真正发生的几何上传次数（顶点 + 索引）。
    ///
    /// 稳态的「只改颜色」帧应当是 0 —— 几何逐字节没变就不重传。
    pub index_uploads: usize,
    /// 几何逐字节没变、因此跳过上传。
    pub index_upload_skipped: bool,
    /// 图像命令分成的批次数（每批一次 draw call，因绑定不同纹理无法合并）。
    pub image_batches: usize,
    /// 因纹理未注册而跳过的图像命令数。
    ///
    /// 恒大于 0 基本都是 bug：注册名与 `props::IMAGE_TEXTURE` 的值拼错。
    pub missing_textures: usize,
    /// 本次提交的多边形三角形数（来自 `Command::Path`）。
    ///
    /// 与 [`Self::quads`] 分开计，因为两者的**抗锯齿来源不同**：四边形靠SDF 的
    /// 解析过渡带（几何自带距离场），多边形没有距离场、只能靠 MSAA。混在一个
    /// 计数里就看不出"图标边缘发毛"是 MSAA 没开还是几何少切了三角形。
    pub path_triangles: usize,
    /// 因走离屏 pass（E4.4）而**执行成功**的滤镜命令数。
    ///
    /// # 为什么单独计而不看 `unimplemented_filters`
    ///
    /// `unimplemented_filters` 的定义是"有多少条命令的滤镜**没**做"（取反计数）。
    /// 它无法区分"没人用滤镜"与"滤镜做了"，所以一旦 E4.4 落地，光看它会以为
    /// 后端一直没接离屏。这个字段给出正面的计数。
    ///
    /// 判据：`unimplemented_filters == 0 && offscreen_passes == 用滤镜的命令数`。
    pub offscreen_passes: usize,
}

impl RenderReport {
    /// 把另一段（分段回放里的一次展开 / 一次离屏 pass）的统计并进来。
    ///
    /// # 为什么求和而不是覆盖
    ///
    /// 分段回放把一帧拆成若干次 `expand`，每次都带回一份 `RenderReport`。只取
    /// 最后一份的话，前面各段的 `quads` / `path_triangles` / `missing_textures`
    /// 全丢 —— 于是「这一帧画了多少四边形」会随**有没有滤镜命令**而变，而这个变化
    /// 与几何量毫无关系，症状是同一份 `DisplayList` 在加一个阴影后报出更少的
    /// 图元。
    ///
    /// `reallocated` 取**或**：任一段扩容过就该报 true。
    pub fn absorb(&mut self, other: RenderReport) {
        self.quads += other.quads;
        self.skipped += other.skipped;
        self.unimplemented_filters += other.unimplemented_filters;
        self.unimplemented_blends += other.unimplemented_blends;
        self.offscreen_passes += other.offscreen_passes;
        self.reallocated |= other.reallocated;
        // `vertex_bytes` 取**最大值**而不是求和：所有段共用同一个顶点缓冲，而它
        // 记的是"这次写了多少字节"，不是"这一帧总共写了几次"。
        //
        // 求和会得到一个比实际缓冲还大的数 —— 而它的唯一用途是与
        // `vertex_capacity` 比大小，于是会出现"报告说写了 3 倍于缓冲容量的字节，
        // 却从没触发过扩容"这种自相矛盾的状态。
        self.vertex_bytes = self.vertex_bytes.max(other.vertex_bytes);
        self.image_batches += other.image_batches;
        self.missing_textures += other.missing_textures;
        self.path_triangles += other.path_triangles;
    }
}

/// 一帧展开出来的全部几何。
///
/// 分四段而**不是**四个 draw call 参数组：矩形与字形可合批成一次 draw，图像必须
/// 按纹理切批，而"按纹理切批"这件事只有拿到顶点之后才算得出来。多边形因为顶点数
/// 不是 4 的倍数只能单独成段（见 [`Vertex::path`]）。
/// 一段连续顶点，共享同一张纹理与同一种混合模式。
///
/// # 为什么需要它
///
/// [`QuadBatch`] 的三段只说明**顶点在流里的位置**，没说「哪一段来自哪些命令」。而
/// 两样东西必须按段切换，base 拿到合并后的流就无从下手：
///
/// 1. **纹理** —— 一次 draw 只能绑一张。
/// 2. **混合模式** —— 混合方程是**渲染管线状态**。29 个混合模式里绝大多数
///    （Multiply / Screen / Overlay…）必须按段换混合状态或 PS 变体。
///
/// [`QuadBatch::batches`] 只覆盖了第 1 条：它按**纹理**切，但两段相邻的图像若混合
/// 模式不同，仍会被合进同一批 —— base 于是给两段套同一个混合方程。
///
/// # 不变量：runs 恰好划分整条顶点流
///
/// 按 `first_vertex` 升序拼接时**不留缝、不重叠、不少不漏**，且顺序与
/// [`QuadBatch::flatten`] 的输出一致。拼接相等是本类型的全部意义 —— 漏掉一段的
/// 症状是「某个图元不出现」，两段重叠的症状是「某个图元重复画」（半透明时更糟：
/// 同一处叠两层，颜色变深且没有任何报错）。所以它有专门的测试守着。
///
/// # 为什么只合并**相邻**的同混合模式段
///
/// 重排命令可以把同混合模式的凑到一起，少几次 draw —— 但**不能**：半透明图形互相
/// 叠加时重排会改变可见结果。所以只认「本来就连着」，draw 次数是命令顺序的上界，
/// 不是最小值。这是刻意的取舍。
#[derive(Debug, Clone, PartialEq)]
pub struct DrawRun {
    /// 在 [`QuadBatch::flatten`] 合并流里的起点。
    pub first_vertex: u32,
    /// 顶点数。
    pub vertex_count: u32,
    /// 要绑定的纹理名。`None` = 不换纹理绑定。
    ///
    /// 字形在 `solid` 段里但**也要采样**（字形图集）—— 它记 `None`，因为图集是
    /// 渲染器全局绑的那一张，不参与逐批绑定；只有 [`Command::Image`](zephyra_core::paint::Command::Image)
    /// 的纹理是逐段绑的。
    pub texture: Option<Rc<str>>,
    /// 该段的混合模式。
    pub blend: zephyra_core::filter::BlendMode,
}

impl DrawRun {
    /// 这一段的结束下标（开区间）。
    pub fn end(&self) -> u32 {
        self.first_vertex + self.vertex_count
    }
}

/// 记录一段「同纹理 + 同混合」的连续顶点；能接上一段就接，否则新开一段。
///
/// 只合并**相邻**段（`last.end() == first`），因为不重排命令 —— 见 [`DrawRun`]。
///
/// # ⚠ 那条相邻性检查在当前结构下**不是**承重逻辑
///
/// 三份段内记录各自只增长（`solid` / `pending` / `paths` 都是只 append），所以连续两次
/// `push_run` 的偏移天然相等 —— 删掉这条检查没有任何测试会失败（变异测试实测
/// SURVIVED）。它保留下来是当**护栏**：万一哪天 `runs` 因为漏 clear 而带着上一帧的
/// 条目进来，相邻性检查会阻止它把新段接到一个坐标不相干的旧段后面。
///
/// 真正的合并规则是「同纹理 + 同混合」那两条；相邻性只是别把不接的东西硬接上。
fn push_run(
    runs: &mut Vec<DrawRun>,
    first: u32,
    count: u32,
    texture: Option<Rc<str>>,
    blend: zephyra_core::filter::BlendMode,
) {
    if count == 0 {
        return;
    }
    if let Some(last) = runs.last_mut() {
        if last.end() == first
            && last.blend == blend
            && last.texture.as_deref() == texture.as_deref()
        {
            last.vertex_count += count;
            return;
        }
    }
    runs.push(DrawRun {
        first_vertex: first,
        vertex_count: count,
        texture,
        blend,
    });
}

#[derive(Debug, Default)]
pub struct QuadBatch {
    /// 矩形 + 字形（可一次 draw 画完）。
    pub solid: Vec<Vertex>,
    /// 图像顶点，按 [`QuadBatch::batches`] 的顺序拼接在 `solid` 之后。
    pub images: Vec<Vertex>,
    /// 已三角化的多边形顶点（每 3 个一个三角形），拼接在最后。
    pub paths: Vec<Vertex>,
    /// 图像分批：`(纹理名, 该批顶点数)`。
    pub batches: Vec<(Rc<str>, u32)>,
    /// 逐段元数据：哪段用哪张纹理、哪种混合模式。位置是**合并流**里的下标。
    ///
    /// 与 [`QuadBatch::batches`] 的区别：后者只按纹理切图元段，前者多切了混合模式
    /// —— 两段相邻的图像若混合模式不同，`batches` 是一批而 `runs` 是两段。
    pub runs: Vec<DrawRun>,
    /// 统计。
    pub report: RenderReport,
}

impl QuadBatch {
    /// 全部顶点（`solid` → `images` → `paths`），以及后两段各自的起点。
    ///
    /// 顺序不能变 —— 图像批次用 `image_first` 偏移定位自己的段，路径用
    /// `path_first`。
    ///
    /// ⚠ 这**新分配**一个 Vec 并把三段拷进去。稳态下它是一次可测量的浪费
    /// （demo 里 540 顶点 × 60 字节 = 32 KB），所以稳态路径请用
    /// [`Self::flatten_into`] 配 [`Scratch`]。
    pub fn flatten(&self) -> (Vec<Vertex>, u32, u32) {
        let mut all = Vec::with_capacity(self.solid.len() + self.images.len() + self.paths.len());
        let (image_first, path_first) = self.append_to(&mut all);
        (all, image_first, path_first)
    }

    /// [`Self::flatten`] 的复用版：写进调用方给的缓冲，返回两段起点。
    ///
    /// # 为什么要有这个
    ///
    /// `flatten` 每帧新建一个 `Vec<Vertex>` 并把三段全拷一遍。demo 一帧 540 顶点
    /// × 60 字节 = 32 KB —— 分配加拷贝，实测是 `expand` 阶段的全部开销。
    ///
    /// 更要紧的是**分配次数**：每帧新 Vec 从 0 增长到 540，要经历 4→8→…→512
    /// 共 8 次扩容，也就是 8 次分配 + 8 次 memcpy。分配探针量到的正是这个 ——
    /// `expand` 那几笔"≤256 字节 × 13 次"就是扩容链。
    ///
    /// 复用缓冲把这两样一起消掉：容量只涨不缩（稳态下 `clear()` 后不再分配），
    /// 拷贝也只是一次连续写入。
    ///
    /// # 调用顺序不能反
    ///
    /// 典型用法是先 `clear` 再 `append_to`。`append_to` 自己**不清**目标 ——
    /// 让"要不要接着往已有内容后面写"由调用方决定，那比在这里默默清掉更可控。
    pub fn flatten_into(&self, out: &mut Vec<Vertex>) -> (u32, u32) {
        out.clear();
        self.append_to(out)
    }

    /// 把三段按顺序追加到 `out` 末尾，返回 `(images 起点, paths 起点)`。
    ///
    /// 不清 `out` —— 见 [`Self::flatten_into`] 的说明。
    fn append_to(&self, out: &mut Vec<Vertex>) -> (u32, u32) {
        out.reserve(self.solid.len() + self.images.len() + self.paths.len());
        out.extend_from_slice(&self.solid);
        let image_first = out.len() as u32;
        out.extend_from_slice(&self.images);
        let path_first = out.len() as u32;
        out.extend_from_slice(&self.paths);
        (image_first, path_first)
    }
}

/// 分段记录的三份（`solid` / `images` / `paths`），`first_vertex` 是**段内偏移**。
///
/// 末尾由 [`merge_runs`] 加上各自基址拼成合并流坐标。
#[derive(Debug, Default)]
struct RunTally {
    solid: Vec<DrawRun>,
    images: Vec<DrawRun>,
    paths: Vec<DrawRun>,
}

/// 把三份段内偏移的 run 拼成合并流坐标（`solid` → `images` → `paths`）。
fn merge_runs(tally: &RunTally, solid_len: u32, images_len: u32, out: &mut Vec<DrawRun>) {
    out.clear();
    out.reserve(tally.solid.len() + tally.images.len() + tally.paths.len());
    let mut add = |runs: &[DrawRun], base: u32| {
        for r in runs {
            out.push(DrawRun {
                first_vertex: r.first_vertex + base,
                vertex_count: r.vertex_count,
                texture: r.texture.clone(),
                blend: r.blend,
            });
        }
    };
    add(&tally.solid, 0);
    add(&tally.images, solid_len);
    add(&tally.paths, solid_len + images_len);
}

/// 跨帧复用的展开缓冲。
///
/// # 为什么需要它
///
/// [`expand`] 每帧要开六个 `Vec`（三段顶点 + 图像批次 + 图像队列 + 合并流），它们
/// 每帧从 0 长到目标大小，于是每帧都要付一串"扩容 = 一次分配 + 一次 memcpy"。
///
/// 实测（`demo --bench --alloc-probe`，每帧 302 次分配 / 172 KB）：其中约 69% 是这些
/// Vec 的扩容链，单是三笔大的就是 61440 + 30720 + 15360 字节 —— 精确对应 1024 / 512
/// / 256 个 `Vertex`（每个 60 字节）的扩容序列。
///
/// 让调用方持有本结构并传进 [`expand_into`]，容量就只涨不缩，稳态下这些分配**全部
/// 消失**。
///
/// # 为什么不塞进 `QuadBatch` 自己
///
/// 试过，两个原因都不成立不了：
///
/// 1. `QuadBatch` 是**返回值** —— 调用方每帧拿到一个新的它，里面的 Vec 也就不可能
///    跨帧存活。`mem::take` 能把旧 Vec 取出来还给调用方，但那要求调用方自己再存一份，
///    于是"谁拥有缓冲"又变成调用点的隐式约定。
/// 2. `expand` 是纯函数（`DisplayList` → `QuadBatch`），单测里大量这样直接调用。
///    把它改成需要额外状态才能调，会让每个测试都得多造一个 `Scratch`。
///
/// 所以复用是**可选的一层**：纯函数 [`expand`] 保留原样，稳态路径走 [`expand_into`]。
///
/// # 容量只涨不缩是刻意的
///
/// 一旦某个特别复杂的界面把容量撑大，之后就一直占着。取舍很明确：稳态零分配影响
/// **每一帧**，多占的几百 KB 只影响启动时那几毫秒。
#[derive(Debug, Default)]
pub struct Scratch {
    /// 一帧的全部顶点（`solid` → `images` → `paths` 合并后的流）。
    ///
    /// 三段仍然分开存放（[`Scratch::solid`] 等），因为图像批次要按纹理定位自己的
    /// 段、路径段基址与它们不同 —— 合并只发生在最后交给 GPU 之前。
    pub solid: Vec<Vertex>,
    pub images: Vec<Vertex>,
    pub paths: Vec<Vertex>,
    /// 图像分批：`(纹理名, 该批顶点数)`。
    pub batches: Vec<(Rc<str>, u32)>,
    /// 合并后的顶点流（交给 GPU 的那一份）。
    pub vertices: Vec<Vertex>,
    /// 索引流。
    pub indices: Vec<u16>,
    /// 图像命令的临时队列（分批前）。
    ///
    /// ⚠ 它存 `(String, …)`，所以复用 Vec **不等于**复用那些 `String` 的堆内存。
    /// 那部分（每帧两三个几十字节的小分配）留在原地，不为了它把句柄改成
    /// `Rc<str>` —— 那是契约改动，收益与风险不成比例。
    image_quads: Vec<(Rc<str>, Rect, UvRect, Color, f32, zephyra_core::filter::BlendMode)>,
    /// 分段记录，三份分开是因为它们在**遍历过程中交错**产生（`solid` 与 `paths`
    /// 来自同一次命令遍历，`images` 来自其后的分批循环），而合并流要求
    /// `solid` → `images` → `paths` 的顺序。分开记，末尾统一加基址即可。
    run_tally: RunTally,
    /// 合并后的 [`DrawRun`] 列表（`run_tally` 的三份按顺序加基址后的结果）。
    runs: Vec<DrawRun>,
    /// 当前图像批的顶点。换纹理时清空重填。
    ///
    /// 它必须在复用范围内 —— 否则 `out.images = pending` 那一步会把从 `scratch`
    /// 借来的 `images` 缓冲丢掉，末尾再把 `pending`（一个**新建**的 Vec）还回去，
    /// 于是 `images` 段每帧退回"从 0 增长"，实测顶点段逐帧**翻倍**。
    pending: Vec<Vertex>,
    /// 本帧两段的起点（`images` / `paths` 在 `vertices` 里的偏移）。
    pub image_first: u32,
    pub path_first: u32,
    /// 纹理名的**驻留器**。
    ///
    /// # 为什么需要它
    ///
    /// 图像批次这条路上，每帧要为**每个图像四边形**克隆一次纹理名（外加每批两次）。
    /// `String` 的克隆就是一次堆分配，于是 demo（141 个四边形 / 约 60 个图像）实测
    /// 每帧多出 **185 次 ≤128 字节的分配** —— 正是 alloc-probe 直方图里
    /// 56 + 63 + 66 那三桶。
    ///
    /// 换成 `Rc<str>` 后，克隆只是引用计数加一，**不分配**。驻留器让「查已有名字」
    /// 退化成对几个字符串的线性比较（demo 只有一种纹理名，几乎立刻命中），代价
    /// 远小于一次 `malloc`。
    ///
    /// 用 `Rc` 而不是 `&str`：后者的生命周期挂在 `DisplayList` 上，而 `Scratch`
    /// 跨帧存活、存在渲染器里，那个借用根本存不进去 —— 这正是当初写成 `String`
    /// 的原因（见本字段旧注释），但它挡掉的是**一次分配**而不是别的。
    names: Vec<Rc<str>>,
    /// 路径段在 `indices` 里的起点 —— 渲染器填 `DrawIndexed` 的
    /// `StartIndexLocation` 靠它。路径没画出来时它是 0（合法：路径段本来就在最后）。
    pub path_index_start: u32,
}

impl Scratch {
    /// 空缓冲。

    /// 本帧的 [`DrawRun`] 列表 —— 渲染器逐段回放的**目录**。
    ///
    /// 段的下标指向 [`Scratch::vertices`]（合并后的顶点缓冲）。
    pub fn runs(&self) -> &[DrawRun] {
        &self.runs
    }

    /// 把纹理名驻留成 [`Rc<str>`]：命中已有名字就只加引用计数，不分配。
    ///
    /// 线性扫描而不是哈希表：不同纹理名的个数是个位数（demo 只有 1 种），哈希表
    /// 本身要一次分配 + 一次哈希，而比较几个短字符串在 CPU 上几乎免费。
    fn intern(&mut self, name: &str) -> Rc<str> {
        for n in &self.names {
            if &**n == name {
                return n.clone();
            }
        }
        let rc: Rc<str> = Rc::from(name);
        self.names.push(rc.clone());
        rc
    }    pub fn new() -> Self {
        Self::default()
    }

    /// 清空全部段但**保留容量**。[`expand_into`] 自己会调。
    ///
    /// # ⚠ 每一段都必须在这里清到
    ///
    /// 漏掉任何一段的症状是**帧数越多、缓冲越大**：`append_to` 是往 `vertices`
    /// **末尾追加**的，所以不清它就等于每帧都把上一帧的几何再接一遍。第 100 帧的
    /// `vertices` 会有 100 帧的量，而渲染器照着它画 —— 于是画面上出现 N 层重叠，
    /// 而且**不报任何错**。
    ///
    /// 这条很难靠肉眼发现，因为单帧（`--dump-cpu`）是对的，只有连续跑才炸；
    /// 帧时间也会跟着线性涨（顶点从 540 涨到 54000），但那看起来像"正常变慢"。
    ///
    /// 所以下面有 `reused_scratch_does_not_grow_across_frames` 这条测试守着它。
    pub fn reset(&mut self) {
        self.solid.clear();
        self.images.clear();
        self.paths.clear();
        self.batches.clear();
        self.vertices.clear();
        self.indices.clear();
        self.image_quads.clear();
        self.pending.clear();
        // run 相关三处同样必须清：段记录不清会让 `runs` 每帧多出上一帧的条目，
        // 症状与顶点段不清时一样（合成器按 `runs` 逐段画，于是每帧多画一层）。
        //
        // ⚠ 这几行**无法被测试单独钉住** —— `expand_translated_into` 在借出缓冲后
        // 自己也 clear 一遍，所以删掉这里的 clear 不会有任何测试失败（变异实测
        // SURVIVED）。留着是因为 `reset()` 的契约是"清空全部段"，少清一处就让它
        // 变成一句谎话，而谎话的代价由下一个读文档的人付。
        self.run_tally.solid.clear();
        self.run_tally.images.clear();
        self.run_tally.paths.clear();
        self.runs.clear();
    }

    /// 各缓冲的容量，供诊断"稳态是否还在长"。
    ///
    /// 判据是**跨帧不变**：稳态下这几个数在若干帧之后不再增长。
    pub fn capacities(&self) -> ScratchCaps {
        ScratchCaps {
            solid: self.solid.capacity(),
            images: self.images.capacity(),
            paths: self.paths.capacity(),
            batches: self.batches.capacity(),
            vertices: self.vertices.capacity(),
            indices: self.indices.capacity(),
            runs: self.runs.capacity(),
            runs_solid: self.run_tally.solid.capacity(),
            runs_images: self.run_tally.images.capacity(),
            runs_paths: self.run_tally.paths.capacity(),
        }
    }
}

/// [`Scratch::capacities`] 的返回值。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ScratchCaps {
    pub solid: usize,
    pub images: usize,
    pub paths: usize,
    pub batches: usize,
    pub vertices: usize,
    pub indices: usize,
    /// 合并后的 [`DrawRun`] 缓冲。
    pub runs: usize,
    /// 分段记录的三份（段内偏移）。
    pub runs_solid: usize,
    pub runs_images: usize,
    pub runs_paths: usize,
}

/// 把一份 `DisplayList` 展开成顶点流。
///
/// `is_registered` 回答"这个纹理名有没有注册" —— 传函数而不是传表，让本模块不必
/// 认识任何 GPU 类型（也就因此能在无 GPU 的环境下测试）。
///
/// 未注册的图像命令会被跳过且**不**打断当前批次 —— 一个拼错的名字不该把前后两段
/// 同纹理图像拆成两次 draw。但它们会计入
/// [`RenderReport::missing_textures`]：不静默，因为症状是"图标不显示"，
/// 没有其他线索指向拼错。
pub fn expand(
    list: &DisplayList,
    size: (u32, u32),
    is_registered: impl Fn(&str) -> bool,
) -> QuadBatch {
    expand_translated(list, size, is_registered, Point::ZERO)
}

/// [`expand`] 加上**平移**：几何按 `translate` 整体搬移后再换算成 NDC。
///
/// # 为什么平移必须发生在 `expand` **内部**
///
/// [`Vertex::position`] 是**已经算好的 NDC**，而 NDC 是「像素坐标 ÷ 目标尺寸」的
/// 产物。于是要画进一张**紧致**的离屏 RT（尺寸 = `plan_offscreens` 的 `bounds`，
/// 原点在 `bounds.x/y`），就得让「屏幕坐标 − bounds 原点」这一步发生**在换算之前**。
/// 换算之后再平移做不到：那时已经除过一次目标尺寸了。
///
/// 所以调用方要成对传：`size` = RT 的像素尺寸，`translate` = `−bounds` 原点。
/// 两个都传错就会画偏，但**不会**看起来像"完全没画"—— 那是更难查的一类症状。
///
/// # `translate` 为零时逐字节等于 [`expand`]
///
/// 零平移走的是同一条代码路径（没有平移分支），所以这不只是"等价"，是同一份几何。
/// 有测试钉住这条。
pub fn expand_translated(
    list: &DisplayList,
    size: (u32, u32),
    is_registered: impl Fn(&str) -> bool,
    translate: Point,
) -> QuadBatch {
    let mut scratch = Scratch::new();
    let report = expand_translated_into(list, size, &is_registered, translate, &mut scratch);
    QuadBatch {
        solid: scratch.solid,
        images: scratch.images,
        paths: scratch.paths,
        batches: scratch.batches,
        runs: scratch.runs,
        report,
    }
}

/// [`expand`] 的复用版：所有缓冲由调用方跨帧持有。
///
/// # 与 [`expand`] 的差别**只有**"缓冲从哪来"
///
/// 遍历逻辑只有这一份，两个入口共用 —— 所以不会出现"开了优化之后某些图元画错了"
/// 那类漂移。这不是风格偏好：两份遍历迟早会在某个边缘 case 上分叉，而分叉的症状
/// 表现为"某类图元少画/错位"，极难往性能优化上联想。
///
/// 纯函数 [`expand`] 就是本函数的一层包装（自己造缓冲）。
///
/// # 返回值与缓冲的关系
///
/// 返回 [`RenderReport`]（纯统计）而**不是** `QuadBatch`：几何留在
/// [`Scratch`] 里。稳态路径要的正是"顶点流 + 索引"两份数据本身，报告只是附带产物。
/// [`expand`] 因此自己再包一层，把 `Scratch` 拷成 `QuadBatch` 按值返回 —— 测试与
/// 一次性调用仍拿到完整的批次结构。
pub fn expand_into(
    list: &DisplayList,
    size: (u32, u32),
    is_registered: &dyn Fn(&str) -> bool,
    scratch: &mut Scratch,
) -> RenderReport {
    expand_translated_into(list, size, is_registered, Point::ZERO, scratch)
}

/// [`expand_into`] 加上平移 —— 见 [`expand_translated`]。
///
/// 平移作用在**像素坐标**上（换算 NDC 之前），因此四条图元路径全部受影响：
/// 矩形、字形、图像、以及多边形的 `to_ndc`。裁剪矩形同样要平移，否则拿未平移的
/// 屏幕坐标去裁已平移的几何，交集算出来是空的。
pub fn expand_translated_into(
    list: &DisplayList,
    size: (u32, u32),
    is_registered: &dyn Fn(&str) -> bool,
    translate: Point,
    scratch: &mut Scratch,
) -> RenderReport {
    expand_range_translated_into(list, size, is_registered, translate, None, scratch)
}

/// [`expand_translated_into`] 加上**下标区间**筛选 —— E4.4 的分段回放用它。
///
/// # 为什么筛选放在**下标区间**而不是"传一个临时 DisplayList"
///
/// 临时列表要克隆每条 `Command`。`Command::Path` 的顶点是 `Arc<[Vertex]>`、
/// `Command::Text` 的字形是 `Arc<[GlyphQuad]>`，克隆只是引用计数 +1 —— 但
/// `Quad` / `Image` 里还有 `FilterChain`（`Vec<Filter>`），那是**真分配**。
/// 一帧几十条命令就是几十次小分配，而这一层存在的全部意义就是稳态零分配。
///
/// 区间筛选只在**这一个迭代点**生效，命令分派那段一个字都不用动 —— 而把
/// 筛选塞进每个分派分支的话，漏一处就是"某种命令被画两遍"或"漏画"。
///
/// # 区间是**半开**的 `[start, end)`
///
/// 与 Rust 切片一致，也与 `str::len()` 那类习惯一致。传 `end = len` 是合法的
/// （末段总是这样），传 `end > len` 只会让后面的下标取不到，不会 panic。
pub fn expand_range_translated_into(
    list: &DisplayList,
    size: (u32, u32),
    is_registered: &dyn Fn(&str) -> bool,
    translate: Point,
    range: Option<std::ops::Range<usize>>,
    scratch: &mut Scratch,
) -> RenderReport {
    let px = (size.0 as f32, size.1 as f32);
    let shift = |r: Rect| shift_rect(r, translate);
    // 从 `scratch` **借**缓冲：把上一帧的成果取到本地 `out`，末尾再还回去。
    //
    // 为什么是 `take` 而不是让 `out` 直接引用 `scratch` 的字段：`out` 还要在末尾
    // **连同内容一起**被搬进 `scratch`（`out` 是本层的局部结构，而 `scratch` 是
    // 调用方的跨帧缓冲）。若直接引用，末尾就还不了。
    //
    // 这么写的代价是每帧几�� `Vec` 的指针搬移（纳秒级），换来的是函数体一行都不用为
    // 复用而改动 —— 而"函数体不动"正是这份复用的关键：改过的遍历逻辑与原来不一致，
    // 那比不复用还糟。
    //
    // ⚠⚠ `take` 之后**必须立刻 `clear()`**，否则内容会被累加。
    //
    // `mem::take` 的语义是"把 `scratch` 换成 `Default::default()`（空 Vec，容量 0），
    // 并把旧值交出来" —— 交出来的那个 Vec **带着上一帧的内容**。而末尾又要把它
    // 填满后还回去，于是下一帧 `take` 拿到的就是一个**非空**的 Vec：不清的话
    // 每帧都把上一帧的几何叠在上面。
    //
    // 症状特别隐蔽：`RenderReport::quads` 一直是对的（它数 `solid.len()/4`，
    // 而 `solid` 确实有那些顶点），画面也只是"多画一层重叠"，不报任何错。
    // 只有逐帧比对顶点量才看得见 —— 那就是 `reused_scratch_does_not_grow_across_frames` 那条测试。
    //
    // 正确写法是"借容量、清内容"，两步都不能少。
    let mut out = QuadBatch {
        solid: std::mem::take(&mut scratch.solid),
        images: std::mem::take(&mut scratch.images),
        paths: std::mem::take(&mut scratch.paths),
        batches: std::mem::take(&mut scratch.batches),
        runs: std::mem::take(&mut scratch.runs),
        report: RenderReport::default(),
    };
    out.solid.clear();
    out.images.clear();
    out.paths.clear();
    out.batches.clear();
    out.runs.clear();
    // `vertices` 与 `indices` 不走 `take`（末尾还要继续用它们），所以显式清。
    scratch.vertices.clear();
    scratch.indices.clear();
    // 同样借出下面两个：`pending` 会成为 `out.images`（末尾要还回去），
    // `image_quads` 是本趟的临时队列（用完即弃，但 Vec 本身可复用）。
    let mut image_quads = std::mem::take(&mut scratch.image_quads);
    // 分段记录借出来，末尾连同内容一起还回去。
    let mut tally = std::mem::take(&mut scratch.run_tally);
    tally.solid.clear();
    tally.images.clear();
    tally.paths.clear();
    // 当前图像批的顶点。换纹理时把它整体搬进 `batches`。
    let mut pending = std::mem::take(&mut scratch.pending);
    image_quads.clear();
    let mut clip: Option<Rect> = None;

    for (i, item) in list.iter().enumerate() {
        // 分段回放（E4.4）：只展开落在本段下标区间里的命令。见
        // [`expand_segment_translated_into`]。
        if let Some(r) = &range {
            if !r.contains(&i) {
                continue;
            }
        }
        match &item.command {
            Command::Quad {
                rect,
                radius,
                fill,
                stroke,
                filter,
                blend,
            } => {
                // 效果/混合模式由 GPU 后端执行，本层只展开几何。但要**记一笔**：
                // 效果静默消失时没有任何视觉线索，这个计数是唯一诊断手段。
                //
                // ⚠ 命令**照常画**（不 skip）—— 否则"加了效果"会表现成"元素凭空
                // 消失"，比"效果没生效"更难排查。
                if !filter.is_identity() {
                    out.report.unimplemented_filters += 1;
                }
                if blend_is_unimplemented(*blend) {
                    out.report.unimplemented_blends += 1;
                }
                let (stroke_color, stroke_w) = match stroke {
                    Some((c, w)) if *w > 0.0 && c.a > 0.0 => (*c, *w),
                    _ => (Color::rgba(0.0, 0.0, 0.0, 0.0), 0.0),
                };
                if fill.a <= 0.0 && stroke_w <= 0.0 {
                    continue;
                }
                // 平移必须发生在**裁剪之前**：`clip` 在 [`Command::PushClip`] 时已经平移到目标
                // 空间，而 `rect` 还是屏幕坐标 —— 两者不在同一空间里，求交算出来是空的
                //（症状是该图元静默消失，而不是"位置略偏"，后者反而好查）。
                let rect = shift(*rect);
                let clipped = intersect(rect, clip.unwrap_or(rect));
                if clipped.width <= 0.0 || clipped.height <= 0.0 {
                    continue;
                }
                // 裁剪后**收缩半个 AA 带**。
                //
                // [`Vertex::quad`] 会把 SDF 四边形向外扩 [`AA_BAND`]，若不收缩，
                // 裁剪线处会多画出本该被裁掉的过渡带 —— 症状是「圆角矩形贴到滚动
                // 容器边缘时，边上有一圈没被裁干净的发白边」。
                //
                // 收缩半个带宽而不是整个：过渡带跨 `d ∈ [-px, px]` 共 2px，
                // 缩一半后仍有1px 在裁剪线内淡出，肉眼才是渐隐而不是硬接缝。
                let rect = shrink(clipped, AA_BAND * 0.5);
                if rect.width <= 0.0 || rect.height <= 0.0 {
                    continue;
                }
                // 描边单独画一遍环带，而不是把描边色混进填充 ——
                // 混进去会让描边与填充重叠处被填充盖住。
                let solid_before = out.solid.len();
                if fill.a > 0.0 {
                    out.solid.extend_from_slice(&Vertex::from_rect(
                        rect,
                        px,
                        [fill.r, fill.g, fill.b, fill.a],
                        [radius.radius, 0.0],
                    ));
                    out.report.quads += 1;
                }
                if stroke_w > 0.0 {
                    out.solid.extend_from_slice(&Vertex::from_rect(
                        rect,
                        px,
                        [
                            stroke_color.r,
                            stroke_color.g,
                            stroke_color.b,
                            stroke_color.a,
                        ],
                        [radius.radius, stroke_w],
                    ));
                    out.report.quads += 1;
                }
                push_run(
                    &mut tally.solid,
                    solid_before as u32,
                    (out.solid.len() - solid_before) as u32,
                    None,
                    *blend,
                );
            }
            Command::Image {
                dst,
                src,
                tint,
                texture,
                radius,
                filter,
                blend,
            } => {
                if !filter.is_identity() {
                    out.report.unimplemented_filters += 1;
                }
                if blend_is_unimplemented(*blend) {
                    out.report.unimplemented_blends += 1;
                }
                let rect = intersect(shift(*dst), clip.unwrap_or(shift(*dst)));
                if rect.width <= 0.0 || rect.height <= 0.0 || tint.a <= 0.0 {
                    continue;
                }
                // `src` 先钳到 0..1：越界 uv 的症状出现在采样器里（取到相邻
                // 图块的内容），那时已经离病因很远。
                let uv = src.map(|r| r.clamped()).unwrap_or(UvRect::FULL);
                image_quads.push((
                    scratch.intern(texture.as_str()),
                    rect,
                    uv,
                    *tint,
                    radius.radius,
                    *blend,
                ));
            }
            Command::Text {
                glyphs,
                offset,
                color,
                filter,
                blend,
                ..
            } => {
                if !filter.is_identity() {
                    out.report.unimplemented_filters += 1;
                }
                if blend_is_unimplemented(*blend) {
                    out.report.unimplemented_blends += 1;
                }
                if color.a <= 0.0 || glyphs.is_empty() {
                    continue;
                }
                let rgba = [color.r, color.g, color.b, color.a];
                let solid_before = out.solid.len();
                // ⚠ 字形的 `dst` 是**局部坐标**（原点 = 元素原点，见
                // `Command::Text::glyphs`），所以这里要把「元素原点」和
                // 「整帧的 translate」**都**加上 —— 少加哪一个都是
                // 「文字整体偏移了一段，且不报任何错」。
                //
                // 两者相加在**展开期**而不是命令构造期，正是为了让四边形能跨帧
                // 复用（构造期平移就等于每帧重算一遍，实测 136 KB/帧）。
                let glyph_shift = |r: Rect| {
                    shift_rect(
                        r,
                        Point::new(translate.x + offset.x, translate.y + offset.y),
                    )
                };
                for g in glyphs.iter() {
                    // 零尺寸字形（空格）不产出几何，但已经推进了后续字形的
                    // offset —— 跳过它即正确的"什么都不画"。
                    if g.dst.width <= 0.0 || g.dst.height <= 0.0 {
                        continue;
                    }
                    out.solid.extend_from_slice(&Vertex::from_uv_rect(
                        glyph_shift(g.dst),
                        px,
                        rgba,
                        g.uv,
                        kind::GLYPH,
                    ));
                    out.report.quads += 1;
                }
                // 整段文本记成一段：字形之间没有纹理/混合模式的变化，拆开只会让
                // draw 次数从 1 变成「字形个数」。
                push_run(
                    &mut tally.solid,
                    solid_before as u32,
                    (out.solid.len() - solid_before) as u32,
                    None,
                    *blend,
                );
            }
            Command::Path {
                vertices,
                fill,
                filter,
                blend,
            } => {
                // 效果/混合模式与其它图元同样只记一笔，命令照常画。
                if !filter.is_identity() {
                    out.report.unimplemented_filters += 1;
                }
                if blend_is_unimplemented(*blend) {
                    out.report.unimplemented_blends += 1;
                }
                if fill.a <= 0.0 || vertices.is_empty() {
                    continue;
                }
                // 逐三角形做裁剪：三角形与裁剪矩形求交得到 0~2 个多边形，
                // 再各自扇形三角化。
                //
                // 为什么不用"顶点变换 + 丢弃"：那样只能丢掉**完全**在裁剪区外的
                // 三角形，跨界的那一个会整块消失。症状是"滚动容器边缘的图标被
                // 削掉一整块"，而且削掉多少取决于三角形怎么切 —— 看起来像随机
                // 缺一块，不是像裁剪。
                let clip_rect = clip;
                let before = out.paths.len();
                let (w, h) = (px.0.max(1e-6), px.1.max(1e-6));
                let to_ndc = |p: zephyra_core::core::Point| {
                    let x = p.x + translate.x;
                    let y = p.y + translate.y;
                    [x / w * 2.0 - 1.0, 1.0 - y / h * 2.0]
                };
                let rgba = [fill.r, fill.g, fill.b, fill.a];
                match clip_rect {
                    // ---- 无裁剪：快路径 ----
                    //
                    // `geom_clip(_, None)` 原样返回三角形，`fan_triangulate` 对
                    // 三点多边形只做一遍退化检查 —— 所以整条链是**恒等变换**，
                    // 而它原本的成本是每个三角形两次堆分配（`tri.to_vec()` +
                    // `Vec::with_capacity`）。
                    //
                    // 实测（`demo --bench`，8 个图标 / 519 个三角形）：Path 分支
                    // 240 µs，占 `expand`（266 µs）的 90%。那是本层最贵的部分，
                    // 而它算出来的东西**每帧完全相同**。
                    //
                    // ⚠ 退化检查（面积 ≈ 0 的三角形）必须留着 —— 它不是优化，
                    // 是与原实现**行为等价**的一部分：漏掉它会让零面积三角形
                    // 进入顶点流，症状是索引数与几何数不一致（画出来看不见，
                    // 但 `path_triangles` 计数会偏）。
                    None => {
                        for tri in vertices.chunks_exact(3) {
                            let t = [
                                tri[0].position,
                                tri[1].position,
                                tri[2].position,
                            ];
                            if zephyra_core::geom::signed_area2(&t).abs() < 1e-12 {
                                continue;
                            }
                            for p in t {
                                out.paths.push(Vertex::path(to_ndc(p), rgba));
                            }
                        }
                    }
                    // ---- 有裁剪：Sutherland–Hodgman + 扇形三角化 ----
                    Some(clip_rect) => {
                        for tri in vertices.chunks_exact(3) {
                            let poly = crate::geom_clip::geom_clip(
                                &[
                                    tri[0].position,
                                    tri[1].position,
                                    tri[2].position,
                                ],
                                Some(clip_rect),
                            );
                            for fan in crate::geom_clip::fan_triangulate(&poly) {
                                for &p in &fan {
                                    out.paths.push(Vertex::path(to_ndc(p), rgba));
                                }
                            }
                        }
                    }
                }
                let added = (out.paths.len() - before) / 3;
                if added > 0 {
                    out.report.path_triangles += added;
                }
                push_run(
                    &mut tally.paths,
                    before as u32,
                    (out.paths.len() - before) as u32,
                    None,
                    *blend,
                );
            }
            Command::PushClip(r) => {
                clip = Some(match clip {
                    Some(c) => intersect(c, shift(*r)),
                    None => shift(*r),
                });
            }
            Command::PopClip => {
                clip = None;
            }
        }
    }

    // ---- 图像：按纹理分批 ----
    //
    // 一次 draw call 只能绑定一张纹理，所以每种纹理一批。批次之间必须保持
    // **原始命令顺序** —— 图像可能半透明互相叠加，重排会改变可见结果。
    // 这里用"仅合并连续同纹理"的分组，而不是把后面同纹理的拉回来合并。
    //
    // 每批记下它要绑的**纹理名**（而不是绑定对象）—— 绑定要活到提交之后，
    // 而纹理表是只读借用，提前取出会造成不必要的借用交叉。
    let mut open: Option<Rc<str>> = None;
    // 当前图像批的混合模式：纹理相同**且**混合模式相同才算同一批。
    //
    // ⚠ 只按纹理合批是不够的：相邻两段图像若混合模式不同而纹理相同，合进一批后
    // base 只能给它们套**同一个**混合方程 —— 症状是「有一段的混合效果没生效」，
    // 而顶点数、缺失纹理数这些统计全都正常。
    let mut open_blend = zephyra_core::filter::BlendMode::default();
    // 当前批在 `images` 段里的起点。
    //
    // ⚠⚠ 这里**不能**在换批时 `pending.clear()`：那是本层一个长期存在的 bug ——
    // `pending` 是 `out.images` 的**全部**内容（合并流里的整个 images 段），而
    // `batches` 报的是「每批多少顶点」、渲染器据此从 `image_first` 起逐批推进。
    // 清掉就等于只留下最后一批的顶点：`batches` 声称 12 个、`images` 只有 4 个，
    // 渲染器读到别的图元的位置上 —— 症状是图像错位或消失，而 `batches` 的统计
    // **完全正常**，所以极难往这里联想。
    //
    // 之所以一直没被发现：demo 里所有图像共用一张纹理 ⇒ 永远只有一批 ⇒
    // `clear()` 是空操作。有测试专门钉住多批的情形。
    let mut batch_start = 0u32;
    for (name, rect, uv, tint, radius, blend) in &image_quads {
        // 未注册的纹理整条跳过，且**不**打断当前批。
        if !is_registered(name) {
            out.report.missing_textures += 1;
            continue;
        }
        if open.as_deref() != Some(&**name) || open_blend != *blend {
            if let Some(prev) = open.take() {
                out.batches
                    .push((prev, (pending.len() as u32) - batch_start));
            }
            batch_start = pending.len() as u32;
            open = Some(name.clone());
            open_blend = *blend;
        }
        let first = pending.len() as u32;
        pending.extend_from_slice(&Vertex::from_rounded_uv_rect(
            *rect,
            px,
            [tint.r, tint.g, tint.b, tint.a],
            *uv,
            *radius,
        ));
        push_run(
            &mut tally.images,
            first,
            (pending.len() as u32) - first,
            Some(name.clone()),
            *blend,
        );
        out.report.quads += 1;
    }
    if let Some(prev) = open.take() {
        out.batches
            .push((prev, (pending.len() as u32) - batch_start));
    }
    // `out.images` 直接吃掉 `pending` 的内容。
    //
    // ⚠ 这里必须**移动**而不是逐帧新建一个 `out.images`：末尾要把缓冲还回
    // `scratch`，所以 `pending` 的所有权此刻就转移给 `out.images` 了 ——
    // 末尾那一行 `scratch.images = out.images` 正是把它还回去。
    //
    // 换个写法（保留 `pending`、只把内容拷进 `out.images`）就等于每帧多一次
    // 拷贝，而复用彻底失效。这是本函数里唯一一处"所有权搬家"，也是最容易
    // 在后续维护中写错的地方。
    out.images = pending;
    out.report.image_batches = out.batches.len();

    // ---- 合并三段 + 展开索引，全部写进 `scratch` ----
    //
    // 顺序不能变：图像批次用 `image_first` 偏移定位自己的段，路径段基址是
    // `quads * 4`（见 `expand_indices_with_paths`）。任何一处调换都会让四边形的
    // 顶点基址全部错位 —— 症状是"路径出现后，它前面所有图元一起变成乱码几何"。
    let report = out.report;
    let (image_first, path_first) = out.append_to(&mut scratch.vertices);
    // runs 的基址 = 合并流里各段的起点，与上面 `append_to` 的顺序**必须**一致。
    // `scratch.vertices` 是给渲染器用的合并流，`out.runs` 是给它导航的目录 ——
    // 两者对不上就是"按 runs 画会画错段"，症状是图元错位而非报错。
    merge_runs(
        &tally,
        out.solid.len() as u32,
        out.images.len() as u32,
        &mut out.runs,
    );
    let total_quads = (out.solid.len() + out.images.len()) / 4;
    let path_tris = out.paths.len() / 3;
    let path_index_start =
        expand_indices_with_paths_into(total_quads, path_tris, &mut scratch.indices);
    scratch.image_first = image_first;
    scratch.path_first = path_first;
    // 路径段的索引起点**必须**留给渲染器 —— 它要用它填 `DrawIndexed` 的
    // `StartIndexLocation`。在这里丢掉它就等于"路径那一批画不出东西"，而症状是
    // 「图标不出现」而不是报错。
    scratch.path_index_start = path_index_start;

    // 把四段还回去（带内容），下一帧就接着用。
    scratch.solid = out.solid;
    // ⚠ `images` 拿的是 `pending`（本帧临时缓冲），所以还回去的**是**它。
    // `pending` 此刻已被移空（见上面 `out.images = pending`）—— 下一帧从
    // `scratch.pending` 借到一个空但有容量的 Vec，正是我们要的。
    scratch.images = out.images;
    scratch.paths = out.paths;
    scratch.batches = out.batches;
    scratch.image_quads = image_quads;
    scratch.run_tally = tally;
    scratch.runs = out.runs;

    report
}

/// 把矩形按 `t` 平移（宽高不变）。
///
/// 平移只搬动原点、不改尺寸，所以 `Vertex::local` / `half_size` 这些**局部**量
/// 一概不用动 —— 它们是由 `rect` 的**尺寸**推出来的，与位置无关。
pub fn shift_rect(r: Rect, t: Point) -> Rect {
    if t.x == 0.0 && t.y == 0.0 {
        return r;
    }
    Rect::new(r.x + t.x, r.y + t.y, r.width, r.height)
}

/// 该混合模式是否**尚未**能在 GPU 上表达。
///
/// 判据是 [`as_coeff`](zephyra_core::filter::BlendMode::as_coeff)：能用系数对表达的
/// 那 15 个（`Clear`..`Screen`）可以直接映射成 `D3D11_BLEND_*` / `D3D12_BLEND_*`，
/// 不需要片元着色器配合。再往后的 14 个（11 个可分离 + 4 个非可分离）需要 PS 变体
/// 或 `SetLum` / `SetSat`。
///
/// # 为什么判据从"非默认"改成这个
///
/// 原来是 `blend != default`，于是 [`RenderReport::unimplemented_blends`] 的含义是
/// "本帧有几个非默认混合模式" —— 那是**用法**的统计，不是**进度**的统计。而它的
/// 文档写着"某个非零值表示'效果被静默忽略'"，两种含义对不上。
///
/// 改成"还不能表达"之后它就成了 E4.1 的判据：那 15 个做完，这个数对它们归零。
fn blend_is_unimplemented(b: zephyra_core::filter::BlendMode) -> bool {
    b != zephyra_core::filter::BlendMode::default() && b.as_coeff().is_none()
}

/// 两个矩形求交（裁剪用）。
pub fn intersect(a: Rect, b: Rect) -> Rect {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    let right = a.right().min(b.right());
    let bottom = a.bottom().min(b.bottom());
    Rect::new(x, y, (right - x).max(0.0), (bottom - y).max(0.0))
}

/// 把矩形四边各向内收`inset`（DIP）。宽或高被收到 ≤ 0 时返回空矩形。
///
/// 与 [`intersect`] 一样返回**空矩形**而不是负尺寸 —— 负尺寸会让
/// `Vertex::quad` 里的 `nx`/`ny` 产生反向 NDC，三角形翻转后仍然成像。
pub fn shrink(r: Rect, inset: f32) -> Rect {
    Rect::new(
        r.x + inset,
        r.y + inset,
        (r.width - inset * 2.0).max(0.0),
        (r.height - inset * 2.0).max(0.0),
    )
}

#[cfg(test)]
#[path = "tests/quad_tests.rs"]
mod tests;

/// 顶点色必须是**预乘**的：rgb 已经乘过自己的 alpha。
    ///
    /// # 为什么这条必须有测试
    ///
    /// `a == 1` 时预乘是恒等 —— 而 demo 与绝大多数测试都用不透明颜色，于是
    /// "预乘了" 与 "没预乘" 在全部现有断言下**完全一样**。真正暴露问题的是
    /// 半透明：GPU 上会双重预乘，整片偏暗，而几何、alpha、计数器全部正常。
    ///
    /// 也就是说：这条不测，E4.1 的混合因子表就是建在一个没人验证过的不变量上。
    #[test]
    fn vertex_color_is_premultiplied() {
        let q = Vertex::from_rect(
            Rect::new(0.0, 0.0, 10.0, 10.0),
            (100.0, 100.0),
            [0.4, 0.2, 0.8, 0.5],
            [0.0, 0.0],
        );
        for v in &q {
            assert!(
                (v.color[0] - 0.2).abs() < 1e-6
                    && (v.color[1] - 0.1).abs() < 1e-6
                    && (v.color[2] - 0.4).abs() < 1e-6,
                "rgb 必须已乘 alpha（0.4/0.2/0.8 × 0.5 = 0.2/0.1/0.4），实得 {:?}",
                v.color
            );
            assert!(
                (v.color[3] - 0.5).abs() < 1e-6,
                "alpha 本身不能被乘：{:?}",
                v.color
            );
        }
        // 不透明时必须是恒等，否则这一层会把所有不透明图元也改掉。
        let o = Vertex::from_rect(
            Rect::new(0.0, 0.0, 10.0, 10.0),
            (100.0, 100.0),
            [0.4, 0.2, 0.8, 1.0],
            [0.0, 0.0],
        );
        assert!((o[0].color[0] - 0.4).abs() < 1e-6, "a=1 时 rgb 不变");
    }

    /// [`Vertex::path`] 也必须预乘 —— 它不走 [`Vertex::quad`]，是**另一份**实现。
    #[test]
    fn path_vertex_color_is_premultiplied() {
        let v = Vertex::path([0.0, 0.0], [0.4, 0.2, 0.8, 0.5]);
        assert!((v.color[0] - 0.2).abs() < 1e-6, "实得 {:?}", v.color);
        assert!((v.color[1] - 0.1).abs() < 1e-6);
        assert!((v.color[2] - 0.4).abs() < 1e-6);
        assert!((v.color[3] - 0.5).abs() < 1e-6);
    }


/// `kind` 落在着色器划定的**哪个分支** —— 也就是"这块像素会被画成什么"。
///
/// 三类共用一条指令流，靠这个标量在片元里分支，所以"落进哪个区间"就是
/// "画成什么"。配对关系的完整说明见 [`kind`]。
#[cfg(test)]
#[path = "tests/quad_kind_branch_tests.rs"]
mod kind_branch_tests;

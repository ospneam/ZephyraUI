//! 效果契约：滤镜（filter）与混合模式（blend）——**纯描述，零依赖，不碰任何系统 API**。
//!
//! # 为什么效果在这里，而不在GPU 层
//!
//! Skia 把效果切在 [`include/effects`](https://skia.googlesource.com/skia/+/refs/heads/main/include/effects/)
//! ——与 `include/core` **并列**的独立层。构建依赖是单向的：`include/effects` 依赖
//! `//src/core`，反向零命中。所以它的切法是：
//!
//! > 效果的**描述**是纯数据（只用 `SkScalar` / `SkPath` 这类 core 词汇表），
//! > 效果的**执行**永远在 GPU 侧（`src/gpu/ganesh/ops/`）。
//!
//! 这条边界正是 [`crate::paint::DisplayList`] 契约成立的前提：效果一旦降级成
//! DisplayList 里的纯字段，换一个 base 实现（DXGI ↔ DirectComposition，或
//! Windows ↔ Wayland）产出的 DisplayList 必然逐字节相同 —— 因为效果从没
//! 参与过"画什么"的决策，它只是"画的东西要多经过一道什么处理"的声明。
//!
//! # 为什么**不**用 `PushLayer` / `PopLayer` 配对命令
//!
//! 这是本模块最容易被写错的地方，而DisplayList 的结构直接否掉了那条路：
//!
//! `DisplayList` 构造时即按 `(layer, z, order)` **稳定排序**（见
//! [`crate::paint`]）。配对命令靠"在有序流里前后夹住内容"生效，可排序会把
//! 中间内容插到别的 `layer` 上去 —— `PushLayer(Content)` 包住的命令可能被排到
//! `Popup` 之后，配对当场失效，且**没有任何报错**。
//!
//! Skia 也没有这个问题，因为它压根不用配对命令：滤镜挂在 `SkPaint` 上，离屏由
//! `SkCanvas::aboutToDraw` → `AutoLayerForImageFilter`（`SkCanvasPriv.h:129`）
//! 在**回放时按需自动开** —— 有 imageFilter 才 `addImageFilterLayer`
//! (`SkCanvasPriv.cpp:234`)。命令流里没有 save/restore 标记。
//!
//! 所以这里沿用同一思路：[`crate::paint::Command`] 各变体**自己**带
//! `filter`与 `blend` 字段，回放方看到非恒等的 filter 才去开离屏。
//!
//! # 离屏边界由谁算
//!
//! 由**滤镜自己**给出，而不是录进命令流。Skia 走
//! `paint.computeFastBounds(drawBounds)`（`SkCanvasPriv.cpp:308`），本质是滤镜
//! 贡献一份 outset。本模块用 [`Filter::outsets`] 做同一件事。
//!
//! ⚠ 那条 outset 与四边形 SDF 的抗锯齿余量（`zephyra_fx::quad::AA_BAND = 0.75`）
//! 是**两套独立依据**，不要混为一谈：
//!
//! | | 谁贡献 | 值 | 挡什么 |
//! |---|---|---|---|
//! | `AA_BAND` | 着色器 SDF | 固定 0.75px | 边缘一个像素的抗锯齿过渡带 |
//! | [`Filter::outsets`] | 滤镜半径 | `3σ`（blur）/ `offset + 3σ`（shadow） | 模糊/阴影溢出原矩形的那部分 |

use crate::core::{Color, Point, Rect};

/// 高斯模糊的标准差 → 实际半径的倍数。
///
/// 3σ 覆盖 99.7% 的高斯能量，所以 blur 的离屏矩形按`3σ` 外扩后，
/// 边界外残留的可见强度低于 0.3%，肉眼与像素断言都不可见。
///
/// 与 Skia 一致：`SkImageFilters::Blur` 内部走 `SigmaToRadius(sigma, scale)`，
/// CPU 侧系数为 3（GPU 侧因线性空间放大而略有差异）。
pub const SIGMA_TO_RADIUS: f32 = 3.0;

/// 一个图像滤镜。
///
/// 只列 UI 真正需要的两种。Skia 有二十多种（displacement / morphology /
/// turbulence / lighting …），但它们要么需要额外资源，要么对UI 没有可感知的
/// 收益 —— 缺了可以后加，加这个枚举不影响任何契约。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Filter {
    /// 各向异性高斯模糊。`sigma` 是**标准差**（不是半径），单位像素。
    ///
    /// 两个 σ 分开给，是因为 UI 里水平方向的模糊（文字选中态的边缘柔化）
    /// 与垂直方向的模糊需求常常不同。
    Blur { sigma_x: f32, sigma_y: f32 },
    /// 外投影。
    ///
    /// 阴影画在**原图之外**，所以离屏矩形必须按 `offset + 3σ` 外扩；若
    /// `offset` 为负（向左上投影），矩形向左上扩。
    DropShadow {
        color: Color,
        offset: Point,
        sigma: f32,
    },
    /// 折射（"液态玻璃"）：把元素**背后**的画面按边缘法线推开再显示。
    ///
    /// # 与 [`Filter::Blur`] 的根本区别
    ///
    /// `Blur` 模糊的是元素**自己**的离屏内容（RT_A 里只有它一条命令）；
    /// 折射模糊/位移的是元素**背后**已经画在屏幕上的东西 —— 也就是
    /// `g_dst` 那张目标副本。两者读的是**不同的纹理**，所以它们的离屏
    /// 内容不能互换。这也是本变体存在的全部理由：没有"读背景"这个动作，
    /// 液态玻璃做不出来。
    ///
    /// # 参数
    ///
    /// - `strength`：折射强度，**像素**。边缘处背景被推开的最大距离。
    /// - `dispersion`：色散量（0..1 常用）。>0 时 R / B 通道相对 G 通道
    ///   沿法线方向偏移不同的量，于是边缘出现彩色描边 —— 真实玻璃的色散。
    ///   0 = 单通道位移，无色散。
    ///
    /// # 为什么 `redraw_original` 必须是 `true`
    ///
    /// 合成阶段输出的是**被扰动的背景**，不是这个元素本身。元素自己的内容
    /// （描边、文字、图标）要在那之后再照常画一遍 —— 与 `DropShadow`
    /// 同一条理由：影子/折射都是**额外**的一层。
    Refraction {
        /// 玻璃**厚度**（像素）。
        ///
        /// # 语义
        ///
        /// 它不是「位移多少像素」，而是**光学厚度** —— 位移由它与
        /// `bezel`（同一个枚举的字段）一起经斯涅尔定律算出来（见
        /// `zephyra.hlsl` 的 `PSRefraction`）。于是同一个 `thickness` 在
        /// 厚玻璃（倒角窄）上弯得多、在薄玻璃（倒角宽）上弯得少，这与
        /// 「位移 = 强度 × 系数」的线性手感是两回事。
        strength: f32,
        /// 倒角带**宽度**（像素）：折射只发生在这条带子里。
        ///
        /// # 为什么必须有它
        ///
        /// 没有倒角带的概念时，位移只能按「到边缘的距离」单调衰减，于是元素
        /// **整块**都在被弯、只是越靠里越弱 —— 看起来像「把画面整体揉了一下」，
        /// 而不是玻璃。真实的玻璃只在边缘那一圈弯（光在那里斜着进出），
        /// 中间是直的。
        bezel: f32,
        /// 色散量（0..1 常用，`0` = R / B 通道相对 G 通道沿法线方向偏移不同的量，
        /// 于是边缘出现彩色描边 —— 真实玻璃的色散。`0` = 单通道位移，无色散）。
        dispersion: f32,
        /// **折射率**（IOR）。1.0 = 没有折射；普通冕牌玻璃约 1.5。
        ///
        /// # 它为什么不是常量
        ///
        /// 它是整条位移公式里唯一让「同一块玻璃看起来厚还是薄」的量：
        /// IOR 越大，光线偏折越小，玻璃越「硬」。真实取值在 1.0–2.2 之间，
        /// 超出这个区间就不再像玻璃，所以给了上界。
        ior: f32,
    },
}

impl Filter {
    /// 本滤镜要求离屏矩形向外扩多少（每边，单位像素）。
    ///
    /// 多个滤镜串联时取**逐边最大值**，不是求和 —— 见 [`FilterChain::outsets`]。
    ///
    /// # 为什么是纯函数
    ///
    /// 它是"要多大离屏纹理"这个问题的唯一来源，而那个问题必须在**不碰 GPU**
    /// 的前提下能回答：这样无 GPU 环境（CPU 参考光栅器、单测）也能验证离屏
    /// 尺寸算得对不对。它是纯几何计算，不含任何系统 API。
    pub fn outsets(&self) -> Outset {
        match self {
            // blur 的强度处处向外，不分方向 ⇒ 四边相同。
            //
            // ⚠ 半径必须 clamp 到 0：负 σ 或 NaN 会让 outset 变成负数，于是
            // 离屏矩形**比**原矩形还小，模糊在边界处被硬生生切掉，且没有任何
            // 报错。`f32::max` 对 NaN 无效（NaN 的一切比较都是 false，直接返回
            // 另一个参数），所以先用 `is_finite` 挡掉。
            Filter::Blur { sigma_x, sigma_y } => {
                let rx = blur_radius(*sigma_x);
                let ry = blur_radius(*sigma_y);
                Outset {
                    left: rx,
                    top: ry,
                    right: rx,
                    bottom: ry,
                }
            }
            Filter::DropShadow { offset, sigma, .. } => {
                let r = blur_radius(*sigma);
                // 影子 = 原矩形被模糊（**四边**各扩 r）后整体平移 offset。
                //
                // ⚠ 四边都要留blur 半径，只有 offset 那一侧才额外多出 |offset|：
                // 向右下投影时，右边扩 `r + offset.x`，而左边只剩 `r - offset.x`
                //（模糊仍会漏出 |offset| 那么多，只是恰好被 offset 抵消掉了）。
                // 写成"只有投影方向扩"会让左边凭空多扩 r —— 症状是离屏矩形被
                // 整体平移，元素左上角出现一小段未渲染的空白，且不报错。
                Outset {
                    left: (r - offset.x).max(0.0),
                    top: (r - offset.y).max(0.0),
                    right: (r + offset.x).max(0.0),
                    bottom: (r + offset.y).max(0.0),
                }
            }
            // 折射的采样会**往元素外**多读 `strength` 那么多像素（边缘处背景被
            // 推开的方向是朝外的），所以离屏矩形必须留出这一圈，否则边缘的
            // 位移在 RT 边界被硬切 —— 症状是玻璃边缘出现一道平直的硬边，
            // 而那看起来像"边框画歪了"而不是"离屏算小了"。
            Filter::Refraction { strength, .. } => {
                let s = if param_active(*strength) {
                    *strength
                } else {
                    0.0
                };
                Outset {
                    left: s,
                    top: s,
                    right: s,
                    bottom: s,
                }
            }
        }
    }

    /// 该滤镜是否为恒等（不改变任何像素）。
    ///
    /// σ 为 0 的 blur 是恒等；`DropShadow` 恒不为恒等（哪怕全透明也有影子形状，
    /// 且 alpha=0 时渲不渲染都可能 —— 保守地判为非恒等，交给离屏处理）。
    pub fn is_identity(&self) -> bool {
        match self {
            Filter::Blur { sigma_x, sigma_y } => {
                blur_radius(*sigma_x) <= 0.0 && blur_radius(*sigma_y) <= 0.0
            }
            Filter::DropShadow { .. } => false,
            // 两个参数**分别**判：strength=0 但 dispersion>0 时仍然有效果
            //（R/B 相对 G 偏移），所以不能"strength 为 0 就整条链恒等"。
            //
            // ⚠ 这里必须是 `!A && !B`，**不能**写成 `!(A && B)` —— 那是
            //    「两个都不生效才算恒等」的**反面**（`||` 的语义），
            //    会让「只有色散」被误判成恒等。
            Filter::Refraction {
                strength,
                bezel,
                dispersion,
                ior,
            } => {
                // ⚠ `ior` **不参与**这条判据。
                //
                // 它是 1.0 就等于「没有折射」，而那正是 `strength = 0` 的情形；
                // 把两者一起判进来会写出 `strength=0 但 ior=1.5 ⇒ 非恒等`，
                // 于是「没写厚度却写了折射率」的样式表会让整条滤镜命令**既不恒等
                // 也不生效** —— 离屏照开、像素照弯成 0，白付一遍离屏开销。
                let _ = (bezel, ior);
                !param_active(*strength) && !param_active(*dispersion)
            }
        }
    }
}

/// 折射的某个参数是否**真的在起作用**。
///
/// 抽出成函数而不是内联两个比较：`is_identity` 与 `outsets` 都要用同一条
/// 判据（`strength=0` 在两处必须得出同一个结论），而内联写两遍就有漂移的机会。
fn param_active(v: f32) -> bool {
    v.is_finite() && v > 0.0
}

/// blur 的 σ → 半径，挡掉非有限值。
fn blur_radius(sigma: f32) -> f32 {
    if !sigma.is_finite() || sigma <= 0.0 {
        0.0
    } else {
        sigma * SIGMA_TO_RADIUS
    }
}

/// 离屏矩形每边要外扩的量。
///
/// 分四边而不是一个标量，是因为 [`Filter::DropShadow`] 的外扩**不对称**：
/// 向右下投影只扩右下，向左上投影只扩左上。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Outset {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl Outset {
    /// 四边取逐边最大值（并集）。
    ///
    /// 串联滤镜时**不能求和** —— 那会让 blur+blur 的离屏矩形大一倍，而实际
    /// 只有最后一个滤镜的输出边缘在起作用。取最大值是保守且正确的上界。
    pub fn union(self, other: Outset) -> Outset {
        Outset {
            left: self.left.max(other.left),
            top: self.top.max(other.top),
            right: self.right.max(other.right),
            bottom: self.bottom.max(other.bottom),
        }
    }

    /// 是否为零（无需外扩）。
    pub fn is_zero(&self) -> bool {
        self.left <= 0.0 && self.top <= 0.0 && self.right <= 0.0 && self.bottom <= 0.0
    }
}

/// 有序滤镜链。
///
/// # 顺序语义
///
/// `chain[0]` 是**最先**执行的 —— 也就是最贴近源图的那一道，与 Skia 的
/// `SkImageFilters::Blur(..., input = SkImageFilters::Offset(...))` 里
/// `input` 继续嵌套的方向一致。
///
/// # 为什么不用 `Option<Box<FilterChain>>` 而存 `Vec`
///
/// 递归结构更贴近 Skia 的嵌套定义，但带来两个实际代价：
///
/// 1. `Command` 要么深（递归嵌套）要么boxed（每次分配）——而 `Command` 是
///    每帧每节点都产出的热路径数据。
/// 2. 无法在单测里对**扁平**的链断言顺序。
///
/// `Vec` 让 `Command` 保持 `Clone + PartialEq` 且拷贝是一次小分配，且顺序
/// 可以直接断言。两级已足够表达 UI 需求（"先投影再模糊"这种三段以上链
/// 在 Skia 里也是用 `SkImageFilters::Shader` 组合的，不走链）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FilterChain {
    filters: Vec<Filter>,
}

impl FilterChain {
    /// 空链（恒等）。
    pub const fn none() -> Self {
        Self {
            filters: Vec::new(),
        }
    }

    /// 单个滤镜。
    pub fn single(f: Filter) -> Self {
        Self { filters: vec![f] }
    }

    /// 按顺序构造（`filters[0]` 最先执行）。
    pub fn new(filters: impl IntoIterator<Item = Filter>) -> Self {
        Self {
            filters: filters.into_iter().collect(),
        }
    }

    /// 追加一道滤镜，返回新链（`self` 不变）。
    pub fn then(mut self, f: Filter) -> Self {
        self.filters.push(f);
        self
    }

    /// 链中的滤镜，按执行顺序。
    pub fn filters(&self) -> &[Filter] {
        &self.filters
    }

    /// 链是否恒等（空链，或每一项都恒等）。
    ///
    /// 回放方**必须**先判这个再决定要不要开离屏 —— 为恒等链开离屏会让每帧
    /// 多一次全屏 RT 绑定，是纯浪费（Skia 的
    /// `AutoLayerForImageFilter` 就只在 `getImageFilter()` 非空时才进）。
    pub fn is_identity(&self) -> bool {
        self.filters.iter().all(Filter::is_identity)
    }

    /// 整条链要求的外扩（逐边最大值）。
    pub fn outsets(&self) -> Outset {
        self.filters
            .iter()
            .fold(Outset::default(), |acc, f| acc.union(f.outsets()))
    }

    /// 给定内容矩形，算出离屏纹理要覆盖的区域。
    ///
    /// 这是 [`crate::paint::DisplayList`] 之外的**纯计算**：宿主拿它去决定
    /// 离屏纹理尺寸，核心不碰GPU 对象。
    ///
    /// # 与 `quad::AA_BAND` 的关系
    ///
    /// 两者是**叠加**的。外扩之后SDF 的过渡带仍然在四边形内（`AA_BAND`
    /// 已经先把几何扩出去了），所以这里**只**加滤镜自己的 outset，不重复
    /// 加 0.75。
    pub fn bounds_for(&self, content: Rect) -> Rect {
        let o = self.outsets();
        Rect::new(
            content.x - o.left,
            content.y - o.top,
            content.width + o.left + o.right,
            content.height + o.top + o.bottom,
        )
    }

    /// 内容矩形映射回离屏纹理内的局部坐标原点。
    ///
    /// 宿主拿它在离屏 RT 上定位几何 —— 因为 [`Self::bounds_for`] 把左上角
    /// 移到了 `content.x - left`，而几何的坐标仍是屏幕坐标。
    pub fn origin_in(&self, content: Rect) -> Point {
        let o = self.outsets();
        Point::new(content.x - o.left, content.y - o.top)
    }
}

/// 混合模式。
///
/// 与 Skia 的 `SkBlendMode` **逐值对齐**（含顺序），见本模块底部的对照测试。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub enum BlendMode {
    /// `r = 0`。清空目标。
    Clear,
    /// `r = s`。只画源。
    Src,
    /// `r = d`。只保留目标（丢弃源）。
    Dst,
    /// `r = s + (1-sa)·d`。**默认**。
    #[default]
    SrcOver,
    /// `r = d + (1-da)·s`。
    DstOver,
    /// `r = s·da`。
    SrcIn,
    /// `r = d·sa`。
    DstIn,
    /// `r = s·(1-da)`。
    SrcOut,
    /// `r = d·(1-sa)`。
    DstOut,
    /// `r = s·da + d·(1-sa)`。
    SrcATop,
    /// `r = d·sa + s·(1-da)`。
    DstATop,
    /// `r = s·(1-da) + d·(1-sa)`。
    Xor,
    /// `r = min(s + d, 1)`。
    Plus,
    /// `r = s·d`。
    Modulate,
    /// `r = s + d - s·d`。
    Screen,
    // ---- 以下为可分离（separable）模式：逐通道独立计算 ----
    /// 视目标而定multiply 或 screen。
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
    Multiply,
    // ---- 以下为非可分离（non-separable）模式：需整幅颜色参与 ----
    /// 取源的色相，目标的饱和度与明度。
    Hue,
    /// 取源的饱和度，目标的色相与明度。
    Saturation,
    /// 取源的色相与饱和度，目标的明度。
    Color,
    /// 取源的明度，目标的色相与饱和度。
    Luminosity,
}

/// 混合方程的系数。
///
/// 对应 Skia 的 `SkBlendModeCoeff`。方程是
/// `result = dst · dst_coeff + src · src_coeff`，其中系数可以是常数、源/目标的
/// 某个分量、或它们的补。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Coeff {
    /// `0`
    Zero,
    /// `1`
    One,
    /// `sc`
    SrcColor,
    /// `1 - sc`
    OneMinusSrcColor,
    /// `dc`
    DstColor,
    /// `1 - dc`
    OneMinusDstColor,
    /// `sa`
    SrcAlpha,
    /// `1 - sa`
    OneMinusSrcAlpha,
    /// `da`
    DstAlpha,
    /// `1 - da`
    OneMinusDstAlpha,
}

impl BlendMode {
    /// 最后一个能用系数对表达的混合模式。
    ///
    /// 与 Skia 的 `kLastCoeffMode = kScreen` 对齐。超过它的模式**不可分离**
    /// （或需要整幅颜色参与），无法拆成 `dst·Cd + src·Cs` 的形式 ——
    /// 这不是实现偷懒，是数学上做不到。
    pub const LAST_COEFF_MODE: BlendMode = BlendMode::Screen;

    /// 最后一个可分离模式（逐通道独立运算）。
    ///
    /// 与 Skia 的 `kLastSeparableMode = kMultiply` 对齐。
    pub const LAST_SEPARABLE_MODE: BlendMode = BlendMode::Multiply;

    /// 模式总数（与 Skia 的 `kSkBlendModeCount` 一致：29）。
    pub const COUNT: usize = 29;

    /// 本模式是否可分离（逐通道独立）。
    pub fn is_separable(&self) -> bool {
        *self <= BlendMode::LAST_SEPARABLE_MODE
    }

    /// 本模式是否只能用系数对表达（Porter-Duff那批）。
    pub fn is_coeff(&self) -> bool {
        *self <= BlendMode::LAST_COEFF_MODE
    }

    /// 分解为 `(src_coeff, dst_coeff)`。
    ///
    /// 返回 `None` 表示该模式不可用系数表达（见 [`Self::is_coeff`]）。
    /// 逐项对齐 Skia `SkBlendMode_AsCoeff` 的 `kCoeffs` 表
    /// （`src/core/SkBlendMode.cpp:68-84`）。
    pub fn as_coeff(&self) -> Option<(Coeff, Coeff)> {
        use BlendMode::*;
        use Coeff::*;
        Some(match self {
            Clear => (Zero, Zero),
            Src => (One, Zero),
            Dst => (Zero, One),
            SrcOver => (One, OneMinusSrcAlpha),
            DstOver => (OneMinusDstAlpha, One),
            SrcIn => (DstAlpha, Zero),
            DstIn => (Zero, SrcAlpha),
            SrcOut => (OneMinusDstAlpha, Zero),
            DstOut => (Zero, OneMinusSrcAlpha),
            SrcATop => (DstAlpha, OneMinusSrcAlpha),
            DstATop => (OneMinusDstAlpha, SrcAlpha),
            Xor => (OneMinusDstAlpha, OneMinusSrcAlpha),
            Plus => (One, One),
            Modulate => (Zero, SrcColor),
            Screen => (One, OneMinusSrcColor),
            _ => return None,
        })
    }

    /// 该模式是否需要覆盖已写入的目标像素。
    ///
    /// `Src` / `SrcOver` 之外的所有模式都要读 `dst`，所以**必须**在绘制前
    /// 保证目标区已有内容 —— 这正是"先画背景再画浮层"不能颠倒的原因。
    ///
    /// 只有一个例外值得注意：[`BlendMode::Clear`] 不读 `dst`，可以画在
    /// 未初始化的 RT 上（而不必先清）。
    pub fn needs_destination(&self) -> bool {
        !matches!(self, BlendMode::Clear | BlendMode::Src)
    }
}

#[cfg(test)]
#[path = "tests/filter_tests.rs"]
mod tests;

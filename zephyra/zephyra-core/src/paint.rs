//! 绘制契约：[`DisplayList`] ——核心与 GPU 后端之间**唯一**的接口。
//!
//! # 为什么这是整个框架最关键的契约
//!
//! 布局、控件、主题最终全部降到一个**排序后的绘制命令列表**。后端只消费它。
//! 这意味着：
//!
//! - 换宿主（宿主 → winit+D3D11）时，**核心一行都不用改**——因为核心从不认识
//!   宿主；
//! - 宿主可以自由选择批次策略：全程一个 draw call，或按图层切若干 pass；
//! - 「这一帧重算了什么」可以直接从 DisplayList 上读出来，回答 GPU 占用率问题；
//! - 单元测试不需要 GPU：断言 DisplayList 的内容即可验证整个降级过程。
//!
//! # 排序
//!
//! 命令按 `(layer, z_index, order)` 稳定排序。`layer` 是粗粒度的合成层
//! （背景 / 内容 / 装饰 / 浮层），后端据此切 render pass；`z_index` 是层内顺序；
//! `order` 是同 z 内的稳定次序（**稳定**排序保证同样输入产出同样列表，
//! 否则每次 diff 出来的顺序都不同，纹理缓存命中率会随机抖动）。
//!
//! # 为什么不是「绘制回调」
//!
//! 另一种常见设计是给后端一个闭包列表（`Vec<Box<dyn Fn(&mut Renderer)>>`）。
//! 那样无法跨线程、无法缓存、无法测试——DisplayList 用数据换来了这三样。
//!
//! 本模块同时提供 [`render`]：把一棵**已布局**的节点树降级成本模块的数据结构。
//! 它是核心对外的唯一产出 —— 宿主拿到 [`DisplayList`] 之后就不再关心树的存在。
//! 几何必须由 [`crate::layout`] 先算好，本函数只读已摆好的矩形。

use std::borrow::Cow;

use crate::context::Context;
use crate::core::{Color, CornerRadius, Point, Rect};
use crate::node::Node;

/// 合成层。粗粒度，决定后端如何切 render pass。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Layer {
    /// 窗口底色。
    Background = 0,
    /// 内容（卡片、文本、图像）。
    Content = 1,
    /// 装饰（边框、分隔线、焦点环）。
    Overlay = 2,
    /// 浮层（弹出菜单、提示、对话框）。
    Popup = 3,
}

impl Layer {
    /// 该层是否需要独立离屏纹理。
    ///
    /// 浮层需要——它必须能盖住下层内容并带自己的阴影/模糊，若直接画进同一
    /// target 就无法在绘制下层之前保留其像素。
    pub fn needs_offscreen(self) -> bool {
        matches!(self, Layer::Popup)
    }
}

/// 一条绘制命令。
///
/// # 为什么效果挂在命令上，而不是 `PushLayer` / `PopLayer` 配对标记
///
/// 本列表**构造时即按 `(layer, z, order)` 稳定排序**（见 [`DisplayList::sort`]）。
/// 配对命令靠"在有序流里前后夹住内容"生效，可排序会把中间内容插到别的
/// `layer` 上去 —— `PushLayer(Content)` 包住的命令可能被排到 `Popup` 之后，
/// 配对当场失效，**且不报任何错**。
///
/// （本模块确实有 [`Command::PushClip`] / [`Command::PopClip`] 这对配对标记，
/// 但它们**零生产者**——只有定义处与 fx 的消费处。真在 `render_rec` 里发出来，
/// 会撞上同一个问题。）
///
/// Skia 也没这个问题，因为它压根不用配对命令：滤镜挂在 `SkPaint` 上，离屏由
/// `SkCanvas::aboutToDraw` → `AutoLayerForImageFilter`（`SkCanvasPriv.h:129`）
/// 在**回放时按需自动开** —— 有 imageFilter 才`addImageFilterLayer`
/// (`SkCanvasPriv.cpp:234`)。命令流里没有 save/restore 标记。
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// 填充一个圆角矩形（着色器内以 SDF 求出圆角与描边）。
    ///
    /// 刻意**不**提供「任意路径填充」作为主要手段：路径填充要 CPU 三角化，而
    /// UI 里绝大多数图形就是圆角矩形 + 描边——把它做成一个 draw call 级别的
    /// 图元能省掉整条三角化管线。需要任意路径时用 [`Command::Path`]。
    Quad {
        rect: Rect,
        radius: CornerRadius,
        /// 填充色。
        fill: Color,
        /// 描边；`None` 表示不描边。
        stroke: Option<(Color, f32)>,
        /// 滤镜链；空链 = 恒等（回放方据此跳过开离屏）。
        ///
        /// 见本类型文档里"为什么效果挂在命令上"。
        filter: crate::filter::FilterChain,
        /// 混合模式（与已画内容合成）。
        blend: crate::filter::BlendMode,
    },
    /// 填充一段路径（图标、矢量图形等非常规形状）。
    Path {
        /// 已三角化的顶点；由宿主的路径栅格化器或 [`crate::text`] 之外的来源提供。
        ///
        /// # 为什么是 `Arc<[Vertex]>` 而不是 `Vec<Vertex>`
        ///
        /// 几何来自 [`crate::icon::IconSet`] 的三角化缓存，而那条缓存里存的就是
        /// `Arc<TriMesh>` —— 每帧把它克隆成 `Vec` 是**纯浪费**：demo 的 8 个图标
        /// 519 个顶点，每帧拷一遍，拷完当帧丢掉。
        ///
        /// 消费者（`crate::geom_clip` 与各后端）只用 `&[Vertex]` 上的方法
        /// （`len` / `is_empty` / `iter` / `chunks_exact`），而 `Arc<[T]>` 的
        /// `Deref` 让这些调用**一字不用改**。
        ///
        /// 共享所有权在这里是**正当**的（不是 `ElementKey` 那种没人需要的
        /// refcount）：命令要活到展开趟结束，而几何缓存在帧之后仍然有效。
        vertices: std::sync::Arc<[Vertex]>,
        fill: Color,
        filter: crate::filter::FilterChain,
        blend: crate::filter::BlendMode,
    },
    /// 绘制一张纹理图像。
    ///
    /// `texture` 是宿主定义的句柄（宿主 的 `Handle` 或自建的 D3D11 纹理 id）。
    /// 核心**不**认识它——这正是宿主边界所在。
    Image {
        texture: TextureRef,
        /// 纹理内的源区域（`None` = 整张）。
        ///
        /// 用 [`crate::core::UvRect`] 而不是 [`Rect`]：后者是 DIP 屏幕矩形，
        /// 两者坐标系不同，混用会让图形上下颠倒且无任何其他症状。
        src: Option<crate::core::UvRect>,
        dst: Rect,
        /// 圆角半径（DIP）。`0` = 直角。
        ///
        /// # 与 [`Command::Quad::radius`] 同机制，但**多一处 uv 外插**
        ///
        /// 位图本身**不带**抗锯齿（它的边缘是硬的），所以一旦要圆角，边缘就变成
        /// 由 SDF 决定 —— 于是四边形必须像 [`Command::Quad`] 那样按 `AA_BAND`
        /// 向外扩，否则 `d ∈ (0, px]` 那半边过渡带被光栅化器裁掉，**圆角会是硬边**。
        ///
        /// 而外扩若沿用「把 uv 铺在扩后四边形上」的做法，会**裁掉并放大**一圈图像
        /// （32px 的图裁 4.7%），所以 fx 侧改成把 uv **外插**，由采样器的 Clamp 兜住。
        ///
        /// 半径为 0 时**不**外扩、**不**施加遮罩 —— 否则最外一圈像素会被压到
        /// 50% alpha，表现为图像四周发暗一圈。
        radius: CornerRadius,
        tint: Color,
        filter: crate::filter::FilterChain,
        blend: crate::filter::BlendMode,
    },
    /// 绘制一段已排版的文本。
    ///
    /// 文本形状（glyph run 的位置与轮廓）在绘制前由宿主按 [`crate::text`] 的
    /// 测量结果算好，核心不参与字形整形。
    Text {
        /// 字形纹理句柄（宿主定义）。
        atlas: TextureRef,
        /// 已定位的字形四边形，坐标**相对 `offset`**。
        ///
        /// # 为什么是 `Arc<[GlyphQuad]>` 而不是 `Vec`
        ///
        /// 这条命令每帧都会被重建（`DisplayList` 不跨帧存活），而**字形位置本身
        /// 几乎从不变** —— 变的是 `offset`（元素原点随布局/resize 移动）。
        /// 若四边形里烘焙了绝对坐标，那么 resize 的每一帧都要重新算一遍 239 个
        /// 字形、每帧分配 239 个 `Vec`。
        ///
        /// 实测（`docs/dev-log/task-resize.md` §批 C · C1b）：`Vec<GlyphQuad>` 每帧
        /// **136 KB**（239 × 1024 B），占 resize 帧分配量的 **40%**。
        ///
        /// 改成 `Arc<[GlyphQuad]>` + 独立的 `offset` 后，缓存挂在节点上，
        /// 命令每帧只克隆一个 `Arc`（引用计数 +1，**零分配**）。
        ///
        /// 官方对应物：`CLayoutStorage` 存的是**相对元素原点**的布局结果，
        /// 绘制趟叠加变换（`uielement.cpp` 的 `Render`/`Arrange` 都不重排文字）。
        glyphs: std::sync::Arc<[GlyphQuad]>,
        /// `glyphs` 的局部原点要落到屏幕上的哪个点。
        ///
        /// 坐标系是**像素**，与 `Quad::rect` / `Image::dst` 一致。
        offset: Point,
        color: Color,
        filter: crate::filter::FilterChain,
        blend: crate::filter::BlendMode,
    },
    /// 裁剪后续命令到该矩形。
    ///
    /// 作用于**其后**的命令直到下一个 `PopClip`——这是有序列表而非栈的代价：
    /// 用显式配对换取「回放时无需维护状态」。
    ///
    /// ⚠ 见本类型文档：排序会拆散配对，所以这个命令**目前零生产者**。
    PushClip(Rect),
    /// 结束裁剪。见 [`Command::PushClip`] 的排序警告。
    PopClip,
}

/// 宿主定义的纹理句柄。
///
/// 核心刻意不定义纹理类型——`宿主::widget::image::Handle` 与自建的 D3D11 纹理
/// id 毫无共性。核心只搬运它，由宿主解释。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TextureRef(pub Cow<'static, str>);

impl TextureRef {
    /// 由宿主提供的稳定字符串 id 构造。
    ///
    /// 接受 `&'static str`（零分配）与 `String` / `Arc<str>`（运行期构造）——
    /// 资产句柄这类东西绝大多数是编译期已知的常量，没必要为它们分配。
    pub fn new(id: impl Into<Cow<'static, str>>) -> Self {
        Self(id.into())
    }

    /// id 字符串。
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// 三角化后的顶点（位置 + 纹理坐标）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vertex {
    pub position: Point,
    pub uv: Point,
}

/// 一个已定位的字形四边形。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlyphQuad {
    /// 纹理内的字形区域（atlas 的归一化 uv）。
    ///
    /// 用 [`crate::core::UvRect`] 而非 [`Rect`]：图集 uv 与屏幕矩形坐标系不同，
    /// 用同一个类型表达两者，颠倒的错误不会有任何其他症状。
    pub uv: crate::core::UvRect,
    /// 屏幕位置与尺寸。
    pub dst: Rect,
    /// 该字形是否来自彩色字体（如 emoji）——决定采样时是否忽略 `color`。
    pub color_glyph: bool,
}

/// 一条命令的排序键。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SortKey {
    /// 合成层。
    pub layer: Layer,
    /// 层内 z 序。
    pub z: i32,
    /// 同 z 内的稳定次序。
    pub order: u32,
}

/// 带排序键的命令。
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    /// 排序键。
    pub key: SortKey,
    /// 绘制命令。
    pub command: Command,
}

/// 排序后的绘制命令列表 —— 核心与后端之间的契约。
///
/// 构造时即完成稳定排序，因此后端可以顺序回放而无需再比较。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DisplayList {
    items: Vec<Item>,
}

impl DisplayList {
    /// 空列表。
    pub fn new() -> Self {
        Self::default()
    }

    /// 追加一条命令。
    pub fn push(&mut self, key: SortKey, command: Command) {
        self.items.push(Item { key, command });
    }

    /// 按 `(layer, z, order)` **稳定**排序。
    ///
    /// 稳定性是缓存命中率的前提：同样输入必须产出逐字节相同的列表，否则
    /// 宿主侧的批次缓存每次都失效。
    pub fn sort(&mut self) {
        self.items.sort_by_key(|i| i.key);
    }

    /// 命令条数。
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// 当前已分配容量（用于验证稳态零分配）。
    pub fn capacity_hint(&self) -> usize {
        self.items.capacity()
    }

    /// 顺序迭代。
    pub fn iter(&self) -> impl Iterator<Item = &Item> {
        self.items.iter()
    }

    /// 某合成层是否出现。
    pub fn has_layer(&self, layer: Layer) -> bool {
        self.items.iter().any(|i| i.key.layer == layer)
    }

    /// 按层切分，供后端决定 render pass 边界。
    pub fn layers(&self) -> Vec<Layer> {
        let mut out: Vec<Layer> = self.items.iter().map(|i| i.key.layer).collect();
        out.sort_unstable();
        out.dedup();
        out
    }

    /// 本列表里所有需要离屏的命令及其离屏矩形。
    ///
    /// 这是"要多大离屏纹理"这个问题的**唯一**答案来源，且它是**纯计算**：
    /// 不碰任何 GPU 对象、不查任何系统 API。所以 CPU 参考光栅器与单元测试
    /// 都能验证离屏尺寸算得对不对——而离屏尺寸算错的后果是"效果在边界处被
    /// 切掉"，那种 bug 极难在真机上看出来。
    ///
    /// # 为什么不需要配对的层标记
    ///
    /// 因为排序会拆散配对（见 [`Command`] 文档）。这里改成**按内容几何**回答：
    /// 每条带非恒等 `filter` 的命令自己声明"我要多大离屏目标"，宿主据此按需
    /// 绑定 RT。相邻命令若离屏矩形相同/重叠，宿主可以复用同一张——复用策略
    /// 属于资源管理，不属于契约。
    ///
    /// 混合模式**不**产生离屏需求：它改的是混合方程，而不是"要多少像素"。
    /// 绝大多数模式可以直接在现有 target 上换混合状态完成（Skia 的
    /// `GrBlendMode` 就是这么用的）。
    pub fn offscreen_requirements(&self) -> Vec<OffscreenRequirement> {
        let mut out = Vec::new();
        for item in &self.items {
            let (Some(content), Some(filter)) = (
                command_content_rect(&item.command),
                command_filter(&item.command),
            ) else {
                continue;
            };
            if filter.is_identity() {
                continue;
            }
            out.push(OffscreenRequirement {
                bounds: filter.bounds_for(content),
                content,
                filter: filter.clone(),
                layer: item.key.layer,
            });
        }
        out
    }

    /// 本列表里是否出现过非默认混合模式。
    ///
    /// 宿主据此决定要不要为混合状态建 PSO 变体。这是纯查询，无副作用。
    pub fn has_non_default_blend(&self) -> bool {
        self.items.iter().any(|i| match &i.command {
            Command::Quad { blend, .. }
            | Command::Path { blend, .. }
            | Command::Image { blend, .. }
            | Command::Text { blend, .. } => *blend != crate::filter::BlendMode::default(),
            Command::PushClip(_) | Command::PopClip => false,
        })
    }

    /// 清空（复用分配，稳态零分配的关键）。
    pub fn clear(&mut self) {
        // 保留容量：每帧重建 DisplayList 时复用同一块缓冲。
        self.items.clear();
    }
}

/// 一条命令的离屏需求：宿主据此分配 RT 并把内容画进去。
#[derive(Debug, Clone, PartialEq)]
pub struct OffscreenRequirement {
    /// 离屏纹理要覆盖的区域（已含滤镜外扩）。
    pub bounds: Rect,
    /// 未外扩的原始内容区域。
    ///
    /// 几何坐标仍是屏幕坐标，所以宿主要用
    /// [`crate::filter::FilterChain::origin_in`] 算出局部原点才知道往哪儿挪。
    pub content: Rect,
    /// 要执行的滤镜链。
    pub filter: crate::filter::FilterChain,
    /// 该命令所在的合成层 —— 决定 RT 的生命周期跨度。
    pub layer: Layer,
}

/// 一条命令的**滤镜链**；`None` = 这条命令不参与离屏（剪裁标记）。
pub fn command_filter(cmd: &Command) -> Option<&crate::filter::FilterChain> {
    match cmd {
        Command::Quad { filter, .. }
        | Command::Path { filter, .. }
        | Command::Image { filter, .. }
        | Command::Text { filter, .. } => Some(filter),
        Command::PushClip(_) | Command::PopClip => None,
    }
}

/// 一条命令要模糊 / 投影的**原始内容矩形**（未外扩）。
///
/// # 为什么单独抽出来
///
/// [`DisplayList::offscreen_requirements`]（纯查询，给资源规划用）与渲染器的
/// 「分段回放」（要真的建 RT 并画一遍）都需要这个矩形。两处各写一遍的话，
/// 迟早有一处漏掉某个命令变体 —— 症状是那个变体的滤镜区域算小了一圈，
/// 模糊在边界处被硬生生切掉，**且不报任何错**。
pub fn command_content_rect(cmd: &Command) -> Option<Rect> {
    match cmd {
        Command::Quad { rect, .. } => Some(*rect),
        Command::Image { dst, .. } => Some(*dst),
        // 文本的范围是所有字形四边形的并集 —— 一条命令里有 N 个字形四边形，
        // 整体模糊才正确（逐字形模糊会把每个字都糊成独立一块，视觉上完全不是
        // "这段文字被模糊了"）。
        Command::Text { glyphs, offset, .. } => Some(union_of_quads(glyphs, *offset)),
        // Path 没有包围盒**字段**，但顶点里就带着位置 —— 遍历一次即得紧包围盒。
        //
        // 用**紧**包围盒而不是节点的布局矩形：三角化的结果不会铺满布局矩形
        // （图标的空白边角、外接圆与图形的差集），按布局矩形算会让每张 RT
        // 白白大一圈；按紧包围盒算，包围盒相同的两条 Path 还能复用同一张 RT。
        Command::Path { vertices, .. } => Some(bounds_of_vertices(vertices)),
        Command::PushClip(_) | Command::PopClip => None,
    }
}

/// 一条命令的**形状圆角半径**（像素）—— 折射算 SDF 用。
///
/// # 为什么不能靠内容矩形反推
///
/// 内容矩形只是包围盒。圆角矩形的 SDF 要的是**半径**：同一个包围盒，
/// 方角与 40px 圆角是两个完全不同的形状，而折射的边缘高光位置完全由半径
/// 决定 —— 半径错了，高光落在错误的地方，画面仍然"有折射效果"，
/// 于是极难归因。
///
/// # 非矩形命令返回 0
///
/// `Path`（图标 / 矢量图形）与 `Text`（一排字形四边形）没有单一的圆角半径：
/// 前者的形状来自三角化后的多边形，后者是多个独立四边形。给它们一个假的
/// 半径只会在合成阶段算出一个**看似合理但毫无意义**的 SDF。
/// 返回 0（方角）是诚实的退化 —— 折射对它们仍然工作，只是不带边缘高光。
pub fn command_corner_radius(cmd: &Command) -> f32 {
    match cmd {
        // 四角相同（`CornerRadius` 只有一个 `radius`），所以取它即可。
        Command::Quad { radius, .. } => radius.radius,
        // 图像可以裁成圆角，所以它的半径也是真的。
        Command::Image { radius, .. } => radius.radius,
        Command::Path { .. } | Command::Text { .. } => 0.0,
        Command::PushClip(_) | Command::PopClip => 0.0,
    }
}

/// 一条命令的内容矩形是否非空（可以渲出像素）。
///
/// # 为什么要单独判
///
/// 空矩形（例如 `Text` 命令一个字形都没有、或 `Path` 顶点为空）经
/// [`crate::filter::FilterChain::bounds_for`] 外扩后仍可能是非空的 —— 模糊
/// 一个空区域，得到的是一张**全透明**的 RT。把全透明的东西往目标上合成
/// 一次没有任何效果，只是白白花掉两趟全 RT 的模糊。所以宿主应当在这里就跳过。
pub fn content_rect_is_drawable(cmd: &Command) -> bool {
    match command_content_rect(cmd) {
        Some(r) => r.width > 0.0 && r.height > 0.0 && r.width.is_finite() && r.height.is_finite(),
        None => false,
    }
}

/// 一批字形四边形的并集，平移 `offset`。
///
/// 空列表返回零尺寸矩形（不是 `None`）——调用方拿到后自然画不出东西，
/// 而 `Option` 会逼每个调用点都写一遍 `if let`。
///
/// # `offset` 为什么是参数而不是烘焙进每个 `dst`
///
/// 因为四边形要**跨帧复用**（见 [`Command::Text::glyphs`]）：烘焙了绝对坐标就
/// 没法复用，每帧都得重算 + 重分配。
///
/// `pub` 是为了让 fx 的 `plan_offscreens` **调用同一份实现**而不是抄一份：
/// 两条路径必须给出相同答案（见 `zephyra-fx::offscreen` 的模块文档），
/// 抄一份就等于把「必须相同」变成「碰巧相同」。
pub fn union_of_quads(glyphs: &[GlyphQuad], offset: Point) -> Rect {
    let mut it = glyphs.iter();
    let Some(first) = it.next() else {
        return Rect::new(0.0, 0.0, 0.0, 0.0);
    };
    // ⚠ 偏移**只**在这里加一次。四个边界各自加同一个 `offset` 等价于把
    // 整个矩形平移 —— 与逐字形平移再取并集**完全相同**（平移对 min/max 可交换）。
    // 写成一次而不是四次，是为了让"漏掉某一处平移"这件事在代码上就不可能发生。
    let (mut x0, mut y0, mut x1, mut y1) = (
        first.dst.x + offset.x,
        first.dst.y + offset.y,
        first.dst.right() + offset.x,
        first.dst.bottom() + offset.y,
    );
    for g in it {
        x0 = x0.min(g.dst.x + offset.x);
        y0 = y0.min(g.dst.y + offset.y);
        x1 = x1.max(g.dst.right() + offset.x);
        y1 = y1.max(g.dst.bottom() + offset.y);
    }
    Rect::new(x0, y0, x1 - x0, y1 - y0)
}

/// 一批顶点的紧包围盒。
///
/// 与 [`union_of_quads`] 同一约定：空列表返回零尺寸矩形而非 `None`。
///
/// # 为什么遍历而不是在 `Command::Path` 上缓存一个包围盒字段
///
/// 缓存的代价是**每条命令**都多 8 字节，而包围盒只在带滤镜时才被读到 —— 图标
/// 里绝大多数命令 `filter` 是恒等链，这条字段对它们永远是死重量。遍历的代价
/// 则是有条件的（`filter.is_identity()` 时根本不进），且 CPU 侧本来就要多遍
/// 遍历同一批顶点（三角化、展开、光栅化）。
///
/// # NaN
///
/// `f32::min`/`max` 忽略 NaN（返回非 NaN 的那个），所以**个别**坏顶点会被静默
/// 跳过，不会污染结果；全部顶点都是 NaN 时返回 NaN 矩形 —— 与"一个 NaN 的
/// `Command::Quad::rect`"同样的下场。有意不特判：那需要一个"哪些顶点算数"的
/// 策略，而这套策略在别处（Quad/Image/Text 的矩形）并不存在，特判只会让
/// Path 成为唯一一条路径不同的命令。
pub fn bounds_of_vertices(vertices: &[Vertex]) -> Rect {
    let Some(first) = vertices.first() else {
        return Rect::new(0.0, 0.0, 0.0, 0.0);
    };
    let (mut x0, mut y0) = (first.position.x, first.position.y);
    let (mut x1, mut y1) = (x0, y0);
    for v in &vertices[1..] {
        x0 = x0.min(v.position.x);
        y0 = y0.min(v.position.y);
        x1 = x1.max(v.position.x);
        y1 = y1.max(v.position.y);
    }
    Rect::new(x0, y0, x1 - x0, y1 - y0)
}

/// 布局 + 降级：把一棵已布局的节点树变成 [`DisplayList`]。
///
/// 调用前必须先 [`crate::layout::layout`] —— 本函数只读已摆好的矩形，不算几何。
/// 几何只有一处算法（[`crate::layout`]），命中测试与绘制共用同一批矩形，所以不会
/// 出现"看得见但点不到"。
pub fn render(node: &Node, ctx: Context<'_>, out: &mut DisplayList) {
    out.clear();
    let mut order = 0u32;
    render_rec(node, ctx, &mut order, out, PaintState::ROOT);
}

/// 绘制趟沿树传递的状态。
///
/// # 为什么不靠属性继承
///
/// [`crate::props::OPACITY`] 声明成 `inheritable`，而
/// [`crate::property::resolve`] 的 `inherited` 参数在**生产代码里没有任何地方
/// 传过非 `None` 的值**（`style.rs::value` 一律传 `None`）—— 也就是说
/// `node.opacity(ctx)` 取的是**本节点**的值，不是祖先链的乘积。
///
/// 去接属性继承会牵动 `text_color` 等一整批 `inheritable` 属性，那是独立一轮。
/// 绘制趟自己累加是**自洽且正确**的做法：半透明天然沿父子链相乘、位移天然相加，
/// 而这里只需要这两种。
///
/// ⚠️ 这也意味着 `opacity()` 作为公开 API 的语义是"**本节点**的不透明度"，
/// 与 [`crate::property::PropertyKey::inheritable`] 的名字不一致。记账见
/// `docs/dev-log/task-anim-wire.md` §0。
#[derive(Debug, Clone, Copy, PartialEq)]
struct PaintState {
    /// 祖先链的不透明度之积。
    opacity: f32,
    /// 祖先链的位移之和（绘制时偏移，**不影响布局与命中测试**）。
    translate: Point,
}

impl PaintState {
    /// 根状态：什么都不乘、什么都不移。
    const ROOT: PaintState = PaintState {
        opacity: 1.0,
        translate: Point::ZERO,
    };

    /// 推进到本节点：把**本节点**的不透明度与位移并入。
    ///
    /// ⚠ 返回的是"走到本节点时"的累积值，所以它既用于**本节点自身**的命令，
    /// 也直接传给子节点。若只在传子节点时用它，本节点自己的位移/透明度就丢了 ——
    /// 症状是"容器背景不动、里面的东西动了"（反过来）。
    ///
    /// `opacity` 夹到 `[0, 1]`：`Opacity` 的默认值是 1.0，但样式表可以写
    /// `opacity: 2`（CSS 不拦），不夹会让下游拿到大于 1 的因子，
    /// 而那永远不是"更不透明"。
    fn descend(&self, node: &Node, ctx: Context<'_>) -> Self {
        let own = node.opacity(ctx);
        let own = if own.is_finite() { own } else { 1.0 };
        let opacity = self.opacity * own;
        let (tx, ty) = (
            self.translate.x + node.translate_x(ctx),
            self.translate.y + node.translate_y(ctx),
        );
        PaintState {
            opacity: if opacity.is_finite() {
                opacity.clamp(0.0, 1.0)
            } else {
                1.0
            },
            translate: if tx.is_finite() && ty.is_finite() {
                Point::new(tx, ty)
            } else {
                // 非有限的位移（样式表里写了 `translate-x: 1e40` 之类）会让整棵
                // 子树的顶点变成 NaN，而 NaN 在光栅化里表现为"整块不画"——
                // 症状离病因很远，所以这里当场丢弃而不是画出去。
                self.translate
            },
        }
    }
}

fn render_rec(
    node: &Node,
    ctx: Context<'_>,
    order: &mut u32,
    out: &mut DisplayList,
    state: PaintState,
) {
    if node.is_collapsed(ctx) {
        return;
    }
    // 先进"本节点"再画：累积的不透明度与位移既属于本节点，也往下传。
    let state = state.descend(node, ctx);
    let offset = state.translate;
    let rect = moved(node.rect(), offset);
    let opacity = state.opacity;
    let layer = node.layer(ctx);
    // 效果与混合模式取自节点自身，不沿父链继承（见 props.rs 的理由）。
    let filter = node.filter(ctx);
    let blend = node.blend_mode(ctx);

    // 自身外观：背景 + 描边合成到一条 Quad（描边复用同一条命令的 stroke 字段，
    // 于是圆角矩形永远是一个 draw call —— 这正是本模块选 Quad 图元的原因）。
    //
    // ⚠ `fill` / `stroke_color` 都过 `with_alpha(opacity)`：**在加上这个之前，
    // `opacity` 只作用于 `image_tint`**，所以"给容器设半透明"在画面上只对图像
    // 生效，而背景、描边、文字全都不透明 —— 那与"没有这个属性"看起来一模一样，
    // 只会让人以为它坏了。
    let fill = node.background(ctx).with_alpha(opacity);
    let stroke_color = node.border_color(ctx).with_alpha(opacity);
    let stroke_width = node.border_width(ctx);
    let radius = node.corner_radius(ctx);
    let has_stroke = stroke_color.a > 0.0 && stroke_width > 0.0;
    if fill.a > 0.0 || has_stroke {
        out.push(
            SortKey {
                layer,
                z: 0,
                order: next_order(order),
            },
            Command::Quad {
                rect,
                radius: CornerRadius { radius },
                fill,
                stroke: has_stroke.then_some((stroke_color, stroke_width)),
                filter: filter.clone(),
                blend,
            },
        );
    }

    // 图像内容
    if let Some(texture) = node.image_texture(ctx) {
        let size = node.image_size(ctx);
        let dst = if size > 0.0 {
            // 指定边长：在节点矩形内居中。
            let dx = rect.x + (rect.width - size) * 0.5;
            let dy = rect.y + (rect.height - size) * 0.5;
            Rect::new(dx, dy, size, size)
        } else {
            rect
        };
        out.push(
            SortKey {
                layer,
                z: 0,
                order: next_order(order),
            },
            Command::Image {
                texture,
                src: node.image_src(ctx),
                dst,
                radius: CornerRadius {
                    radius: node.corner_radius(ctx),
                },
                tint: node.image_tint(ctx).with_alpha(opacity),
                filter: filter.clone(),
                blend,
            },
        );
    }

    // 矢量图标内容（CPU 三角化 → `Command::Path`）。
    //
    // ⚠ **图像优先于图标**：两者都设了时只画图像，不画图标。
    //
    // 这是本函数里唯一一处"两个属性指向同一个视觉位置"的歧义，判据必须写死，
    // 否则症状是"图标压在图像上"或"图像不出现"——两者都看不出是选择规则的问题。
    //
    // 为什么图像胜出：它已经有位图缓存（不需要每帧三角化），且**位图自带抗锯齿**；
    // 而路径图元要走 MSAA 才能有干净的边缘。在两者都给定时，先用便宜且已就绪的那个
    // 是唯一不会退化画面的选择。
    //
    // 换句话说：**同时设两者是调用方的 bug**，而这个 bug 的最佳表现方式是
    // "图像正常显示"（少画一个图标），而不是崩溃或什么都不画。
    if node.image_texture(ctx).is_none() {
        if let Some(name) = node.icon_name(ctx) {
            if !name.is_empty() {
                // 这里传的是**未平移**的 `node.rect()`，不是上面那个 `rect` ——
                // 理由见 [`icon_command`] 里"几何缓存查找"那一段的警告。
                if let Some(cmd) = icon_command(
                    &name,
                    node.rect(),
                    node,
                    ctx,
                    &filter,
                    blend,
                    opacity,
                    offset,
                ) {
                    out.push(
                        SortKey {
                            layer,
                            z: 0,
                            order: next_order(order),
                        },
                        cmd,
                    );
                }
            }
        }
    }

    // 文本内容
    //
    // 排在背景之后、子节点之前：文本属于节点自身的"外观"，与背景同一层；而子节点
    // 是叠在它上面的内容。若把文本排到最后，压在文本节点之上的子元素会被文字盖住。
    if let Some(mut cmd) = text_command(node, ctx, offset, opacity) {
        // `text_command` 造出命令后才补效果 —— 那时它已不再关心 style，只管排版。
        match &mut cmd {
            Command::Text {
                filter: f,
                blend: b,
                ..
            } => {
                *f = filter.clone();
                *b = blend;
            }
            _ => unreachable!("text_command 只产出 Command::Text"),
        }
        out.push(
            SortKey {
                layer,
                z: 0,
                order: next_order(order),
            },
            cmd,
        );
    }

    // 子节点。
    //
    // ⚠ 传**已经推进过本节点**的 `state` —— 这是"容器不透明度影响整棵子树"
    // 与"祖先位移带着后代一起动"的唯一实现点。
    let child_state = state;
    for child in node.children() {
        render_rec(child, ctx, order, out, child_state);
    }
}

/// 矩形加一个绘制时偏移。
///
/// 没有位移时**原样返回**（不动 `x`/`y` 的浮点位）—— 那让"静止时零开销"是
/// 结构上的事实，而不是靠调用方判 `is_zero()`。
#[inline]
fn moved(r: Rect, offset: Point) -> Rect {
    if offset.x == 0.0 && offset.y == 0.0 {
        r
    } else {
        Rect::new(r.x + offset.x, r.y + offset.y, r.width, r.height)
    }
}

/// 文本绘制命令（`None` = 不产出）。
///
/// 抽成自由函数而不是 [`Node`] 的方法：它需要 `&Node`（只读），而其余样式 getter
/// 都是 `&self` 的公开 API。把它留在 `impl Node` 上会让"公开词汇表"里混进一个
/// 只有绘制趟需要的东西。
///
/// # `offset` 为什么直接进 `origin` 而不是事后平移字形
///
/// [`crate::text::TextRender::glyph_quads`] 是 `(shaped, origin, style)` 的纯函数、
/// **没有缓存**（见 `text/mod.rs`），所以把平移量折进 `origin` 是零成本的。
/// 事后平移字形反而要重新分配一个 `Vec<GlyphQuad>` —— 那是"为了省一个加法而付一次分配"。
///
/// # `opacity` 为什么在这里就乘进文字色
///
/// 文字色是 `Color`（含 alpha），而 `Command::Text` 的字形颜色**只有一个** `color`
/// 字段 —— 没有任何"每字形不透明度"的机制。要让半透明容器里的文字也半透明，
/// 只能在**造命令时**就把因子乘进颜色。放在 [`render_rec`] 里回填是做不到的。
/// # 字形四边形缓存：为什么**按元素原点**存，而不是按屏幕位置
///
/// 元素原点 `origin` 每个 resize 帧都在变（那正是本轮优化要快的那一趟），
/// 而字形的**局部**位置一个像素都不会变。把原点折进 `glyph_quads` 的参数里，
/// 就等于把「每帧都要重算」变成了「每帧都要重算」——正是要消掉的那笔开销。
///
/// 官方对应物：`CLayoutStorage` 存的是相对元素原点的结果，变换在绘制趟叠加。
///
/// 代价是 [`Command::Text`] 多一个 `offset` 字段，且**三处**必须同步用它：
/// `text_command`（写入）、`command_content_rect`（滤镜范围）、`zephyra-fx::quad`
/// （顶点展开）。少改一处 ⇒ 那条路径上的几何整体错位，且不报任何错。
fn text_command(node: &Node, ctx: Context<'_>, offset: Point, opacity: f32) -> Option<Command> {
    let text = node.text()?;
    let engine = ctx.engine?;
    let style = node.text_style(ctx);
    let max_width = node.text_max_width(ctx);
    let base = node.rect();
    let origin = Point::new(base.x + offset.x, base.y + offset.y);
    let mut engine = engine.borrow_mut();
    // ⚠ 优先用**测量趟存下来的排版结果**。
    //
    // 这一句是本轮 resize 优化里最大的一笔：排版（`shape()`）要按样式算出每一行
    // 的基线与每个字形的位置，是整帧最贵的一步；早先它每帧对每个文本节点重跑
    // 一次。临时探针实测（324 帧 resize）：`render` 92.9 ms、`layout` 18.4 ms ——
    // render 是 layout 的 5 倍，而那差额基本就是重复排版。
    //
    // 官方对应物：`CLayoutStorage` 里存着排版结果，绘制趟直接取
    // （`uielement.cpp` 的 `Measure` / `Arrange` 都不重排文字）。
    //
    // ⚠ 缓存未命中时**必须**排一次，而不是返回 `None`：直接返回会让"渲染趟先于
    // 测量趟跑"的调用方（以及任何直接调 `render` 的测试）突然看不见文字。
    let owned;
    let shaped = match node.cached_shaped() {
        Some(s) => s,
        // 缓存未命中时排一次并**就地借用**：`owned` 必须活到 `glyph_quads` 之后。
        //
        // ⚠ 不能在这里返回 `None`：那会让"渲染趟先于测量趟跑"的调用方
        // （以及任何直接调 `render` 的测试）突然看不见文字 —— 而那是个
        // 只在特定顺序下出现的、极难归因的缺陷。
        None => {
            owned = engine.shape(text, &style, max_width);
            &owned
        }
    };
    // uv 与图集句柄由 `TextRender::glyph_quads` 给出 —— core 有字形图集
    // （`GlyphAtlas`，纯 CPU 数据），所以能算出 uv；但 core 不碰 GPU 纹理句柄，
    // 于是只用那个不透明的 `TextureRef`。
    //
    // ⚠ 缓存命中时**只**克隆 `Arc`；`atlas` 必须重新取一次，因为它可能随图集
    // 增长而变（`GlyphAtlas` 满了会重建纹理）。克隆 `Arc` 是引用计数 +1，
    // 零分配 —— 这就是这一段存在的全部意义。
    let revision = engine.atlas_revision();
    // ⚠ 判据是**排版输入指纹**（文本 / 样式 / `max_width`）+ 图集修订号。
    //
    // 不要改成「测量趟这帧有没有重算」：resize 时约束一变 `measure` 就会重跑，
    // 产出一个**内容相同**的 `MeasureResult`（只有 `size` 被 `pick_axis` 夹了一下）。
    // 用那种判据，命中率恒为 0 —— 这正是早先那版的错误，136 KB/帧 一分没省。
    let cached = {
        let slot = node.glyph_quads.borrow();
        match slot.as_ref() {
            Some(c)
                if c.revision == revision
                    && c.text.as_ref() == text
                    && c.style == style
                    && c.max_width == max_width =>
            {
                Some((c.atlas.clone(), std::sync::Arc::clone(&c.glyphs)))
            }
            _ => None,
        }
    };
    let (atlas, glyphs) = match cached {
        Some(hit) => hit,
        None => {
            let (atlas, v) = engine.glyph_quads(shaped, Point::new(0.0, 0.0), &style)?;
            let g: std::sync::Arc<[crate::paint::GlyphQuad]> = v.into();
            // 借用守卫在**写回之前**就必须放掉：`glyph_quads` 已经是 `RefCell`，
            // 在还持着读守卫时 `borrow_mut()` 会 panic。上面的读借用写在
            // `let cached = { ... }` 的**独立作用域**里，正是为此。
            *node.glyph_quads.borrow_mut() = Some(Box::new(crate::node::GlyphQuadCache {
                text: text.into(),
                style: style.clone(),
                max_width,
                revision,
                atlas: atlas.clone(),
                glyphs: std::sync::Arc::clone(&g),
            }));
            (atlas, g)
        }
    };
    Some(Command::Text {
        atlas,
        glyphs,
        offset: origin,
        color: node
            .color(&crate::props::TEXT_COLOR, ctx)
            .unwrap_or(Color::rgb(1.0, 1.0, 1.0))
            .with_alpha(opacity),
        // 效果由 [`render_rec`] 回填 —— 它已经算好了 `filter`/`blend`，而本函数
        // 只该关心排版。给空链占位是为了让类型完整，值不参与渲染。
        filter: crate::filter::FilterChain::none(),
        blend: crate::filter::BlendMode::default(),
    })
}

fn next_order(order: &mut u32) -> u32 {
    let o = *order;
    *order += 1;
    o
}

/// 曲线展平的容差（DIP）。
///
/// # 为什么是 0.25 而不是"更精细"的值
///
/// 容差决定"曲线被折线逼近到多准"，代价是**三角形个数** —— 而三角化是**每帧**做的
/// （[`Command::Path`] 的顶点进了 `DisplayList`，下一帧重建时重新算）。
///
/// 对 24×24 的图标，0.25 DIP 意味着圆弧上大约每 1.5° 一个顶点：一个典型图标
/// 30~60 个三角形。足够光滑（比一个像素细），而量不到性能代价。
///
/// 再往下收（0.05）会让三角形数翻几倍，收益却看不见 —— 屏幕上仍然是一个 16~24 像素
/// 宽的图标，肉眼分辨不出 0.05 DIP 的曲率。
const FLATTEN_TOLERANCE: f32 = 0.25;
#[allow(clippy::too_many_arguments)]
// 参数是画一次所需的全部输入，彼此没有可成组打包的语义 —— 凑一个 struct 只是把
// 8 个参数搬进另一个名字，读的人反而要多跳一层。见 AGENTS.md「代码自解释优先」。
/// 图标 → [`Command::Path`]。
///
/// # 这条链是几何层唯一的生产入口
///
/// ```text
/// props::ICON_NAME  →  IconSet::get  →  flatten（曲线 → 折线）
///                    →  tessellate（多孔三角化）  →  Command::Path
/// ```
///
/// 核心只做中间两步 —— **"名字 → 数据"是宿主的事**（就像纹理句柄一样），
/// 所以这里从 [`IconSet`] 取，而 [`crate::icon::assets::REGISTRY`] 只是内置的
/// 一份默认表。宿主登记自己的图标走同一条路。
///
/// # 三角化缓存（在 [`IconSet`] 上，不在这里）
///
/// `flatten` + `triangulate` 的结果只取决于（图标, 目标矩形），与时间无关。
/// 缓存在 [`IconSet`] 上而不是本函数里，因为缓存要跨帧存活，而本函数每帧被调、
/// 手上什么都没有。
///
/// # ⚠ 这里曾经因为"估错了成本"而不做缓存，而那次估算的教训值得留着
///
/// 原文写的是：
///
/// > 而实测每帧成本只有几十微秒（`demo --bench` 的 `paint` 阶段中位数 78 µs，
/// > 其中图标占比很小），远低于缓存的复杂度与失效风险。所以**现在不做**。
///
/// 两处都错：
///
/// 1. **"图标占比很小"是估的，不是量的。** 那个 `paint` 阶段的中位数之所以只有
///    78 µs，是因为**当时 demo 里还没有矢量图标**（只有 8 条路径，是后来
///    接通的）。接通之后实测：`paint` 中位 **603 µs**，其中三角化
///    **50 µs/个 × 8 个 = 400 µs**，占 66%、占整帧 39%。
/// 2. **"几十微秒"恰好落在"值不值得"的边界上**，而边界附近的估算错误最贵 ——
///    它是唯一一种看起来有依据的猜测。
///
/// 修掉之后：`paint` 603 → **107 µs**，整帧 1021 → **504 µs**。
///
/// 教训与本 crate 里另外两处同源（`glyph_quads` 的重复栅格化、`PropertyChange`
/// 的多余 `Arc::new`）：**性能判断必须来自计时，"估算 + 论证"不算**。
/// 论证只会让猜测看起来更可信。
///
/// # 返回 `None` 的两种情况
///
/// - 名字未登记 → [`Icon::EMPTY`]，无路径段 → 不产出命令；
/// - 三角化失败（自相交等）→ 不产出命令。
///
/// 两者都**不** panic：一个拼错的图标名不该让整个界面崩掉，而"某个图标不显示"
/// 是可见、可对比的。
fn icon_command(
    name: &str,
    rect: Rect,
    node: &Node,
    ctx: Context<'_>,
    filter: &crate::filter::FilterChain,
    blend: crate::filter::BlendMode,
    opacity: f32,
    offset: Point,
) -> Option<Command> {
    let set = ctx.icon_set()?;
    let icon = set.get(name);
    if icon.is_empty() {
        return None;
    }
    // ⚠ 命中缓存就直接返回 —— 这一步在稳态下把每个图标的 50 µs 三角化降到
    // 一次哈希查找。判据是**整个映射的输入**：图标数据 + 目标矩形（见
    // `IconSet::cached_mesh`）。
    let (vx, vy, vw, vh) = icon.view_box;
    if vw <= 0.0 || vh <= 0.0 {
        return None;
    }

    // ---- 几何缓存查找 ----
    //
    // 必须放在**展平之前**：展平 + 三角化是整条链里唯一贵的部分（实测 50 µs/个），
    // 而它只由（图标，目标矩形）决定 —— 见 [`crate::icon::IconSet`] 的
    // `GeometryCache` 文档。
    //
    // ⚠ 这里用的必须是**未平移**的 `rect`。若拿平移后的矩形当键，入场动画的
    // 每一帧位移都不同 ⇒ 缓存次次落空 ⇒ demo 的 19 个图标 × 50 µs = **每帧 1 ms**，
    // 而动画只有 333 ms —— 也就是整个动画期间都在为"省一次顶点加法"付三角化的钱。
    // 平移挪到产出之后做（见 `shift_vertices`），静止时它连分配都没有。
    if let Some((vertices, _triangles)) = set.cached_mesh(icon, rect) {
        if vertices.is_empty() {
            return None;
        }
        // 只加 refcount，不拷贝顶点 —— 这正是"命中缓存"该有的成本。
        //
        // ⚠ 早于本轮，`Command::Path` 的 `vertices` 是 `Vec<Vertex>`，所以这里
        // 无论缓存命中与否都要 `mesh.vertices.clone()`：demo 的 8 个图标
        // 519 个顶点每帧拷一遍，拷完当帧丢掉。改成 `Arc<[Vertex]>` 之后
        // 缓存命中那条路变成零分配 —— 而那正是缓存存在的意义。
        return Some(Command::Path {
            vertices: shift_vertices(&vertices, offset),
            fill: node.icon_color(ctx).with_alpha(opacity),
            filter: filter.clone(),
            blend,
        });
    }

    // viewBox → 节点矩形：等比缩放并居中。
    //
    // 等比是必须的（非等比会把圆形图标压成椭圆），居中是因为节点矩形与图标
    // 宽高比不一致时不该把图形拉偏 —— 那看起来是"图标歪了"。
    let scale = (rect.width / vw).min(rect.height / vh);
    let w = vw * scale;
    let h = vh * scale;
    let ox = rect.x + (rect.width - w) * 0.5 - vx * scale;
    let oy = rect.y + (rect.height - h) * 0.5 - vy * scale;
    let map = |p: Point| Point::new(ox + p.x * scale, oy + p.y * scale);

    // 展平：`Vec<Contour>`（见 [`crate::geom::FlatPath`]），每条轮廓是闭合的。
    let flat = crate::geom::flatten(&icon.segments, FLATTEN_TOLERANCE);
    if flat.is_empty() {
        return None;
    }
    // viewBox → 节点矩形的映射，展平之后立刻套上（三角化只关心最终坐标）。
    let contours: Vec<crate::geom::Contour> = flat
        .into_iter()
        .map(|c| crate::geom::Contour::new(c.points().iter().copied().map(map).collect()))
        .filter(|c| c.is_fillable())
        .collect();
    if contours.is_empty() {
        return None;
    }

    let rule = match icon.paint.rule {
        crate::icon::FillRule::NonZero => crate::geom::FillRule::NonZero,
        crate::icon::FillRule::EvenOdd => crate::geom::FillRule::EvenOdd,
    };
    let mesh = crate::geom::triangulate(&contours, rule).ok()?;
    if mesh.vertices.is_empty() {
        return None;
    }
    let vertices = set.cache_mesh(icon, rect, mesh);

    Some(Command::Path {
        vertices: shift_vertices(&vertices, offset),
        fill: node.icon_color(ctx).with_alpha(opacity),
        filter: filter.clone(),
        blend,
    })
}

/// 把整条路径的顶点平移。
///
/// # 静止时**不分配**
///
/// 零位移直接返回原引用（`Arc` 克隆只是 refcount +1）。这不是优化洁癖：
/// 入场动画之外的所有绘制都走这条路，而 Gallery 每帧有 19 个图标 ——
/// 每帧 19 次 `Vec` + 19 次 `Arc` 分配换 0 像素位移，是笔亏本买卖。
///
/// ⚠ `uv` **不动**：图集的归一化坐标与屏幕位置无关，平移屏幕位置不该动它。
#[inline]
fn shift_vertices(vertices: &std::sync::Arc<[Vertex]>, offset: Point) -> std::sync::Arc<[Vertex]> {
    if offset.x == 0.0 && offset.y == 0.0 {
        return vertices.clone();
    }
    std::sync::Arc::from(
        vertices
            .iter()
            .map(|v| Vertex {
                position: Point::new(v.position.x + offset.x, v.position.y + offset.y),
                uv: v.uv,
            })
            .collect::<Vec<Vertex>>(),
    )
}

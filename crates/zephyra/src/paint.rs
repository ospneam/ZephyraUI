//! 绘制契约：[`DisplayList`] ——核心与 GPU 后端之间**唯一**的接口。
//!
//! # 为什么这是整个框架最关键的契约
//!
//! 布局、控件、主题最终全部降到一个**排序后的绘制命令列表**。后端只消费它。
//! 这意味着：
//!
//! - 换宿主（iced → winit+wgpu）时，**核心一行都不用改**——因为核心从不认识
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

use std::sync::Arc;

use crate::core::{Color, CornerRadius, Point, Rect};

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
    },
    /// 填充一段路径（图标、矢量图形等非常规形状）。
    Path {
        /// 已三角化的顶点；由宿主的路径栅格化器或 [`text`] 之外的来源提供。
        vertices: Vec<Vertex>,
        fill: Color,
    },
    /// 绘制一张纹理图像。
    ///
    /// `texture` 是宿主定义的句柄（iced 的 `Handle` 或自建的 wgpu 纹理 id）。
    /// 核心**不**认识它——这正是宿主边界所在。
    Image {
        texture: TextureRef,
        src: Option<Rect>,
        dst: Rect,
        tint: Color,
    },
    /// 绘制一段已排版的文本。
    ///
    /// 文本形状（glyph run 的位置与轮廓）在绘制前由宿主按 [`crate::text`] 的
    /// 测量结果算好，核心不参与字形整形。
    Text {
        /// 字形纹理句柄（宿主定义）。
        atlas: TextureRef,
        /// 已定位的字形四边形。
        glyphs: Vec<GlyphQuad>,
        color: Color,
    },
    /// 裁剪后续命令到该矩形。
    ///
    /// 作用于**其后**的命令直到下一个 `PopClip`——这是有序列表而非栈的代价：
    /// 用显式配对换取「回放时无需维护状态」。
    PushClip(Rect),
    /// 结束裁剪。
    PopClip,
}

/// 宿主定义的纹理句柄。
///
/// 核心刻意不定义纹理类型——`iced::widget::image::Handle` 与自建的 wgpu 纹理
/// id 毫无共性。核心只搬运它，由宿主解释。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TextureRef(pub Arc<str>);

impl TextureRef {
    /// 由宿主提供的稳定字符串 id 构造。
    pub fn new(id: impl Into<Arc<str>>) -> Self {
        Self(id.into())
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
    /// 纹理内的字形区域（atlas uv 空间的矩形）。
    pub uv: Rect,
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

    /// 清空（复用分配，稳态零分配的关键）。
    pub fn clear(&mut self) {
        // 保留容量：每帧重建 DisplayList 时复用同一块缓冲。
        self.items.clear();
    }
}

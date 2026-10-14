//! 文本层：测量、排版、字形图集 —— 全部在这里，与宿主无关。
//!
//! # 分层
//!
//! 文本拆成两半，切分线是**字体文件从哪来**：
//!
//! ```text
//! core（零依赖，本层）                base（宿主）
//! ───────────────────                ─────────────────
//! GlyphSource  trait（唯一要宿主实现的）  该 trait 的实现
//! TextStack    排版 + 断行 + 图集 + 降级   └ ab_glyph 解析字体字节
//! GlyphAtlas   字形位图打包（shelf）     D3D11 纹理上传
//! ```
//!
//! core 侧**不**知道字体格式、不知道图集是 GPU 纹理、不知道纹理句柄长什么样。
//! 它只知道"给我一个能把字符变成位图的来源"。于是换一个字体栈（ab_glyph →
//! cosmic-text → CoreText）只需实现 [`GlyphSource`]，本文件一行不改。
//!
//! 宿主那一侧因此只剩两件事：把字体字节解析成 [`GlyphSource`]，以及把
//! [`GlyphAtlas::pixels`] 上传成纹理。
//!
//! # 为什么图集在 core 而不是 base
//!
//! 字形图集常被误认为"GPU 资源所以该在 base"。但它**不是** —— 它是一块 CPU 侧的
//! `Vec<u8>` 加一张"哪个字形在哪个位置"的表；GPU 只在最后一步 upload 一次。
//!
//! 把它放 base 的代价是：打包逻辑（shelf packer、边界检查、uv 计算）只能在 base
//! 里测试，而 base 的测试需要 GPU 或字体文件。放 core 则这些都能在纯 CPU 上断言 ——
//! 本文件末尾的断言正是这么写的。
//!
//! # 为什么 `Command::Text` 在 core 里产出
//!
//! [`crate::paint::Command::Text`] 要求 `Vec<GlyphQuad>`（带 uv）与图集句柄。
//! 宿主只能**消费** `DisplayList`，不能往里注入命令 —— 所以这个转换必须在 core
//! 降级时发生。core 有图集，于是能给出 uv；core 不碰纹理句柄，于是只用那个不透明
//! 的 [`crate::paint::TextureRef`]。
//!
//! # 已知限制
//!
//! **没有 GSUB/GPOS 整形。** 本层不做连字、双向文本与复杂脚本 —— 那些需要完整的
//! OpenType 整形栈。阿拉伯语、印地语会显示成逐字孤立字形。要支持就得换一个更强的
//! [`GlyphSource`]（且 [`GlyphSource::advance`] 需要返回整段而非单字形，以承载
//! 字素簇）。
//!
//! 行内断行按**字符边界**切，不做连字符与字素簇感知的断词。对 UI 标签够用。

use std::borrow::Cow;

use crate::core::{Point, Rect, Size, UvRect};
use crate::paint::{GlyphQuad, TextureRef};

pub mod atlas;
pub mod stack;

/// `GlyphAtlas` 的**坐标契约**测试。
///
/// 单独成文件而不是塞进 `atlas.rs` 的 `mod tests`，是因为这些测试用的是
/// `GlyphSource` 的**假实现** —— 它们验的是「core 与字形来源之间的约定」，
/// 而不是图集自己的内部实现。放在 core 里意味着**任何**实现了 `GlyphSource`
/// 的后端都受这组测试约束，不必各自重写一遍。
///
/// 这个契约被违反过一次，且症状极端（图集全空 → 文字完全不出现），详见文件头。
#[cfg(test)]
mod atlas_tests;

pub use atlas::{
    AtlasGlyph, AtlasRaster, AtlasStats, EntryInfo, EvictPlan, EvictionPolicy, GlyphAtlas,
    GlyphKey, LruEviction,
};
pub use stack::TextStack;

/// 字形来源：把字符变成位图与步进。
///
/// **这是宿主唯一需要为文本实现的东西。** 其余全部（断行、图集打包、uv 计算、
/// `Command::Text` 产出）都由 [`TextStack`] 在 core 里完成。
///
/// 实现由宿主提供（`ab_glyph` / `cosmic-text` / `swash` …）。全部方法按
/// **新字形**调用而非按帧调用 —— 已缓存的字形不再走 `rasterize`，所以性能约束
/// 宽松得多。
pub trait GlyphSource {
    /// 字符 → 字形 id。字体缺该字符时返回一个**可渲染的**替代 id（通常是 U+FFFD
    /// 的豆腐框）；实在没有就返回 0。
    ///
    /// 不要返回"失败"——缺字是个常态（emoji、CJK 扩展、符号），让调用方据此决定
    /// 画不画，会导致同一段文本在有无字体的机器上排版不同。
    fn glyph_id(&mut self, ch: char) -> u32;

    /// 单个字形的步进宽度（DIP）。
    fn advance(&mut self, glyph_id: u32, px_size: f32) -> f32;

    /// 栅格化一个字形到 8 位覆盖率位图。
    ///
    /// 返回 `None` = 空白字形（空格、零宽字符）—— 它们没有位图，但**有**步进，
    /// 所以排版仍要为它们让出空间。
    ///
    /// 实现提示：直接用字体库自己的回调（`ab_glyph` 的 `OutlinedGlyph::draw`）
    /// 写进返回值的 `coverage` 缓冲即可，不需要额外中间层。
    fn rasterize(&mut self, glyph_id: u32, px_size: u32) -> Option<AtlasRaster>;

    /// 该来源在图集里的字体 id。同一份字节的多次加载必须返回**相同**的 id ——
    /// 否则同一个字形会在图集里存多份，白白吃掉容量。
    fn font_id(&self) -> u32;
}

/// 字形来源与度量。
pub trait TextEngine {
    /// 一段**已整形**文本的排版结果。
    fn shape(&mut self, text: &str, style: &TextStyle, max_width: Option<f32>) -> ShapedText;

    /// 单个字符的步进宽度（用于光标定位等不需要完整整形的场景）。
    fn advance(&mut self, ch: char, style: &TextStyle) -> f32;

    /// 行高（两行基线之间的距离）。
    fn line_height(&mut self, style: &TextStyle) -> f32;
}

/// 文本样式。
#[derive(Debug, Clone, PartialEq)]
pub struct TextStyle {
    /// 字号（DIP）。
    pub size: f32,
    /// 字重（400 = Regular，600 = Semibold）。
    pub weight: u16,
    /// 字体族名；`None` 用宿主默认。
    pub family: Option<String>,
    /// 行高倍数；`None` 用字体自带值。
    pub line_height: Option<f32>,
    /// 字号缩放系数（Windows 的「文本大小」设置会改它）。
    pub scale: f32,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            size: 14.0,
            weight: 400,
            family: None,
            line_height: None,
            scale: 1.0,
        }
    }
}

impl TextStyle {
    /// 有效字号（应用缩放后的 DIP）。
    pub fn effective_size(&self) -> f32 {
        self.size * self.scale
    }
}

/// 一次排版的结果。
///
/// 只含**度量与位置**，不含字形轮廓 —— 轮廓由宿主在绘制时从自己的字形图集取。
/// 这样 core 不必知道字体的存储格式（`swash` 的 outline 与 `ab_glyph` 的位图
/// 轮廓形状完全不同）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ShapedText {
    /// 整体尺寸。
    pub size: Size,
    /// 排出的行。
    pub lines: Vec<Line>,
    /// **本次排版用的字号（像素）**。
    ///
    /// # 为什么必须显式带上，而不能由 `lines` 反推
    ///
    /// 图集按 `(font_id, glyph_id, px_size)` 缓存，所以绘制时必须知道**同一个**字号 ——
    /// 拿错字号会取到另一套位图。
    ///
    /// 它曾经是**反推**的（`stack.rs` 里的 `infer_px_size`：基线 ÷ 1.2，或最大步进
    /// ÷ 0.5）。那个启发式假设「最宽的字形是拉丁窄字符，步进 ≈ 0.5em」，于是
    ///
    /// - 含 **CJK** 的文本：全角步进 ≈ 1em → 反推出 **两倍**字号。16px 的中文被按
    ///   32px 栅格化，而步进仍按 16px 算 —— **字形互相叠压**，一行字挤成一团。
    /// - 只有 `line_height` 自定义（不是默认 1.2）时同样错。
    ///
    /// 那是个**纯拉丁**启发式，而 demo 的文本全是中文 —— 也就是说这条路径一直在产出
    /// 错误结果，且症状（字挤在一起）与「字体不含 CJK」「GPU 没画」都长得像。
    ///
    /// 排版时字号就在手上（`style.effective_size()`），直接存下来即可，没有任何理由
    /// 去猜。
    pub px_size: f32,
}

/// 一行。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Line {
    /// 该行基线位置（相对文本块左上角）。
    pub baseline: Point,
    /// 该行宽度。
    pub width: f32,
    /// 该行的字形位置（相对基线）。
    pub glyphs: Vec<PlacedGlyph>,
    /// 是否是因换行而断开的最后一行（用于省略号处理）。
    pub is_last: bool,
}

/// 一个已定位的字形。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlacedGlyph {
    /// 字形 id。
    pub glyph_id: u32,
    /// 相对基线的位置。
    pub offset: Point,
    /// 步进宽度。
    pub advance: f32,
    /// 是否来自彩色字体（emoji）——决定绘制时是否忽略主题文本色。
    pub color_glyph: bool,
}

/// 截断：把超宽文本裁到 `max_width` 并加省略号。
///
/// 这是 UI 里最常被重写的一段逻辑（表格单元格、按钮标签、文件名列都要），所以
/// 放在核心而不是让每个控件各写一遍。
pub fn ellipsize(
    engine: &mut dyn TextEngine,
    text: &str,
    style: &TextStyle,
    max_width: f32,
) -> String {
    if max_width <= 0.0 {
        return String::new();
    }
    if engine.advance_width(text, style) <= max_width {
        return text.to_string();
    }
    // 逐字退，直到加上省略号后能放下。
    let dot = engine.advance_width("…", style);
    let mut kept: String = text.chars().collect();
    while !kept.is_empty() && engine.advance_width(&kept, style) + dot > max_width {
        kept.pop();
    }
    kept.push('…');
    kept
}

/// [`TextEngine`] 的便利扩展。
pub trait TextEngineExt: TextEngine {
    /// 整段文本的步进宽度（不换行）。
    fn advance_width(&mut self, text: &str, style: &TextStyle) -> f32 {
        self.shape(text, style, None).size.width
    }

    /// 测量一段文本在给定宽度下的排版结果。
    fn measure_block(&mut self, text: &str, style: &TextStyle, max_width: f32) -> ShapedText {
        self.shape(text, style, Some(max_width))
    }
}

impl<T: TextEngine + ?Sized> TextEngineExt for T {}

/// 能把排版结果降级成绘制命令的引擎。
///
/// 与 [`TextEngine`] 分开有两个理由：
///
/// 1. **它可以有默认实现** —— [`NullEngine`] 那样没有字体的实现只需什么都不返回，
///    不必为"如何画字形"写一个空方法体。
/// 2. **两个能力可以被分别要求** —— 例如只做布局预计算的宿主只需要 [`TextEngine`]，
///    不该被迫实现绘制。
pub trait TextRender: TextEngine {
    /// 图集内容的修订号。图集**内容**一变就加一。
    ///
    /// # 它是给缓存用的，不是给渲染用的
    ///
    /// 绘制趟缓存了字形四边形，其中含 uv。uv 指向图集里的具体位置，而图集满了
    /// 会 LRU 淘汰并**整体重排** —— 于是「已缓存字形数不变」**推不出**
    /// 「uv 仍然有效」。修订号是唯一可靠的判据。
    ///
    /// 宿主那边早就靠它发现图集变化并强制重绘（`host.rs` 的
    /// `atlas_revision_uploaded`），所以这不是新概念，只是把它补进 trait ——
    /// 绘制趟拿到的是 `&dyn TextRender`，拿不到具体类型。
    ///
    /// # 默认 0
    ///
    /// 替身（无真实图集）永远返回 0：它们的 uv 恒为 `UvRect::FULL`，不存在
    /// 「重排导致 uv 失效」这回事。给个恒定值比逼每个替身编一个计数器好。
    fn atlas_revision(&self) -> u64 {
        0
    }

    /// 把排版结果转成绘制命令所需的字形四边形与图集句柄。
    ///
    /// 返回 `None` = 画不出来（无图集、图集已满、无字体）。**不得**用空 `Vec` 表示
    /// "这段没有字形" —— 那与"整段不可见"是两种不同的故障，调用方需要能区分以便
    /// 给出降级提示。
    fn glyph_quads(
        &mut self,
        shaped: &ShapedText,
        origin: Point,
        style: &TextStyle,
    ) -> Option<(TextureRef, Vec<GlyphQuad>)>;
}

/// 一个空实现，用于测试与「本节点无文本」的情形。
///
/// **定宽**（不是零宽）：核心的测试用它让布局结果可预测，于是能断言"某段文本占多
/// 宽"。零宽版本（找不到字体时的降级）在 [`TextStack::disabled`]。
#[derive(Debug, Default)]
pub struct NullEngine {
    /// 固定每字符宽度；让测量结果可预测，便于断言。
    pub char_width: f32,
}

impl TextEngine for NullEngine {
    fn shape(&mut self, text: &str, style: &TextStyle, max_width: Option<f32>) -> ShapedText {
        let cw = self.char_width * style.effective_size();
        let count = text.chars().count();
        let line_height = style.line_height.unwrap_or(1.2) * style.effective_size();

        // 无界时每行就是整段文本，不该走 `count.max(1)` 那条路 —— 那会把空文本
        // 变成"一行零宽"，而空文本必须能表示成零行。
        let per_line = match max_width {
            Some(max) if cw > 0.0 && max > 0.0 => ((max / cw).floor() as usize).max(1),
            _ => count,
        };
        let line_count = if per_line == 0 { 0 } else { count.div_ceil(per_line) };

        let chars: Vec<char> = text.chars().collect();
        let mut lines = Vec::with_capacity(line_count);
        for i in 0..line_count {
            let chunk: Vec<char> = chars
                .iter()
                .skip(i * per_line)
                .take(per_line)
                .copied()
                .collect();
            let mut x = 0.0f32;
            let glyphs: Vec<PlacedGlyph> = chunk
                .iter()
                .map(|c| {
                    let g = PlacedGlyph {
                        glyph_id: *c as u32,
                        offset: Point::new(x, 0.0),
                        advance: cw,
                        color_glyph: false,
                    };
                    x += cw;
                    g
                })
                .collect();
            lines.push(Line {
                baseline: Point::new(0.0, (i as f32 + 1.0) * line_height),
                width: chunk.len() as f32 * cw,
                glyphs,
                is_last: i + 1 == line_count,
            });
        }
        ShapedText {
            // 宽度取**最长的一行**，不是"每行容量"。多行文本块的实际宽度由最宽的
            // 那行决定 —— 若用 `per_line * cw`，一个 100 字符的段落被断成 10 行时
            // 会报告 10 字符宽，容器随之算出错误的尺寸。
            size: Size::new(
                lines.iter().map(|l| l.width).fold(0.0f32, f32::max),
                line_count as f32 * line_height,
            ),
            lines,
            // 替身没有图集，但字号照样要带 —— 契约上 `px_size` 必须等于排版时用的
            // 那个值，不能因为「反正用不上」就填 0。
            px_size: style.effective_size(),
        }
    }

    fn advance(&mut self, _ch: char, style: &TextStyle) -> f32 {
        self.char_width * style.effective_size()
    }

    fn line_height(&mut self, style: &TextStyle) -> f32 {
        style.line_height.unwrap_or(1.2) * style.effective_size()
    }
}

/// 由排版结果推导文本块的绘制矩形。
pub fn text_rect(shaped: &ShapedText, origin: Point) -> Rect {
    Rect::new(origin.x, origin.y, shaped.size.width, shaped.size.height)
}

impl TextRender for NullEngine {
    /// 用**单位方块**为每个字形产出占位四边形。
    ///
    /// # 为什么 `NullEngine` 要产出可绘制的四边形
    ///
    /// 它没有真实图集，所以画出来是"每个字一个像素"。但这些四边形有**正确的锚点
    /// 与基线关系** —— 而那恰好是最容易算错的部分（y 翻转方向、基线相对文本块的
    /// 位置、跨行推进）。
    ///
    /// 于是核心里那条契约（"文本节点会产出 `Command::Text`，且字形落在正确位置"）
    /// 能在纯 CPU 上断言，不必等 GPU。若 `NullEngine` 返回 `None` 或零四边形，那些
    /// 断言就只能改用 `TextStack` + 测试字形来源 —— 而**核心保证**
    /// （属性 → 命令 → 坐标）就变成间接验证，间接验证最常见的失效形式就是
    /// "测的是替身，不是真身"。
    ///
    /// # 单位方块而不是真实尺寸
    ///
    /// 用 `char_width × size` 当宽度会让相邻四边形互相重叠（`'m'` 的步进远宽于它的
    /// 墨迹），而用行高当高度会让多行文本的矩形互相压住。单位方块让"位置对不对"
    /// 与"尺寸准不准"解耦：前者由替身断言，后者由有真实图集的实现断言。
    ///
    /// `uv` 指向 (0,0) 那个像素是有意的 —— 它是"我是替身，没有真实图集"的显式
    /// 标记。真实实现给各自的 uv。
    fn glyph_quads(
        &mut self,
        shaped: &ShapedText,
        origin: Point,
        _style: &TextStyle,
    ) -> Option<(TextureRef, Vec<GlyphQuad>)> {
        let cw = self.char_width;
        let quads = quads_from_shaped(shaped, origin, |_, _, _| {
            Some((
                UvRect::FULL,
                Point::new(0.0, 0.0),
                Size::new(cw, 1.0),
            ))
        });
        Some((TextureRef::new(NULL_ATLAS), quads))
    }
}

/// 图集句柄名。`TextStack` 用它产出 [`crate::paint::Command::Text`]，宿主按这个名字
/// 绑定纹理。
///
/// 单一图集、全进程共享 —— 换字体栈意味着换纹理，所以这不是"每个 TextStack 一张"。
pub const ATLAS_NAME: &str = "glyph-atlas";

/// `NullEngine` 的图集句柄。恒定值而非每次新建：宿主侧的栅格化缓存以图集数据的
/// **地址**为键，每帧换一句柄会让缓存每次都落空。
const NULL_ATLAS: &str = "null-atlas";

impl NullEngine {
    /// 供测试断言用：图集句柄名。
    pub const ATLAS: &'static str = NULL_ATLAS;
}

/// 把 `ShapedText` 降级成 `Command::Text` 所需的字形四边形。
///
/// 独立成自由函数（而不是 [`TextStack`] 的方法）是为了让**没有字体的实现**也能产出
/// 命令 —— `NullEngine` 需要断言"文本节点会产出 `Command::Text`"这条契约，而它没有
/// 图集可用；此时 `uv_of` 可以恒返回 `None`（于是产出零四边形但仍有命令），或像
/// 测试那样用固定 uv。
///
/// `uv_of` 的第二个参数是 `(行号, 列号)` —— 同一字形在不同位置可能是不同 uv 吗？
/// 不是（同一字形的位图只有一块）。这个参数存在的唯一理由是让调用方能用它做
/// 稳定排序或调试输出。
pub fn quads_from_shaped(
    shaped: &ShapedText,
    origin: Point,
    uv_of: impl Fn(u32, usize, usize) -> Option<(UvRect, Point, Size)>,
) -> Vec<GlyphQuad> {
    let mut quads = Vec::new();
    for (row, line) in shaped.lines.iter().enumerate() {
        for (col, g) in line.glyphs.iter().enumerate() {
            let Some((uv, offset, size)) = uv_of(g.glyph_id, row, col) else {
                continue;
            };
            // 尺寸全由 `size` 决定 —— **不**从步进推导。
            //
            // 早期版本在没有图集条目时用「步进宽 × 行高」凑一个矩形。那是错的：
            // 步进是**字形之间的间距**，不是字形的宽度。`'m'` 的步进远大于它的墨迹
            // 宽度，于是占位矩形互相重叠；而空白字形（步进非零、墨迹为零）会得到一
            // 个非零矩形，在屏幕上表现为"空格也在画东西"。
            //
            // 所以 `size` 为零就跳过 —— 那是"这个字形没有可见像素"的唯一诚实表达。
            if size.width <= 0.0 || size.height <= 0.0 {
                continue;
            }
            let baseline_x = origin.x + g.offset.x;
            let baseline_y = origin.y + line.baseline.y;
            quads.push(GlyphQuad {
                uv,
                dst: Rect::new(
                    baseline_x + offset.x,
                    // 字体的 y 向上为正，核心的 y 向下为正 → 减。
                    baseline_y - offset.y - size.height,
                    size.width,
                    size.height,
                ),
                color_glyph: g.color_glyph,
            });
        }
    }
    quads
}

/// `String` → `PropertyValue::Str` 的便捷转换。
///
/// 文本几乎总是运行期构造的（来自应用状态），所以这个转换比手写
/// `PropertyValue::Str(s.into())` 常见得多。
pub fn text_value(s: impl Into<Cow<'static, str>>) -> crate::property::PropertyValue {
    crate::property::PropertyValue::Str(s.into().into())
}

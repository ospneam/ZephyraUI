//! 文本：测量与排版的抽象。
//!
//! # 为什么是 trait 而不是直接依赖 cosmic-text
//!
//! 字体整形（shaping）是 UI 里最不可能自己重做的部分：需要处理 OpenType
//! 的 GSUB/GPOS、连字、双向文本、各家平台字体差异。而**测量与排版的接口**
//! （一段文本有多宽、怎么断行、每个字形落在哪）却非常稳定。
//!
//! 所以核心只声明接口，宿主提供实现：
//!
//! - `zephyra-iced` → 转发给 iced 的文本栈（省掉一整套字形缓存实现）
//! - `zephyra-winit` → cosmic-text 或 swash + 自建字形图集
//!
//! 核心**不缓存字形**，因为字形图集的生存期由 GPU 资源管理决定，属于宿主。
//! 核心只缓存「测量结果」，那部分是纯 CPU 且与后端无关。

use crate::core::{Point, Rect, Size};

/// 字形来源与度量。
///
/// 实现由宿主提供。全部方法可能被高频调用，故要求实现内部零分配或复用缓冲。
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
/// 只含**度量与位置**，不含字形轮廓——轮廓由宿主在绘制时从自己的字形图集取。
/// 这样核心不必知道字体的存储格式（`swash` 的 outline 与 `ab_glyph` 的位图
/// 轮廓形状完全不同）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ShapedText {
    /// 整体尺寸。
    pub size: Size,
    /// 排出的行。
    pub lines: Vec<Line>,
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
    /// 字体内的字形 id。
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

/// 一个空实现，用于测试与「本节点无文本」的情形。
#[derive(Debug, Default)]
pub struct NullEngine {
    /// 固定每字符宽度；让测量结果可预测，便于断言。
    pub char_width: f32,
}

impl TextEngine for NullEngine {
    fn shape(&mut self, text: &str, style: &TextStyle, max_width: Option<f32>) -> ShapedText {
        let width = text.chars().count() as f32 * self.char_width * style.effective_size();
        let (width, line_count) = match max_width {
            Some(max) if self.char_width * style.effective_size() > 0.0 && width > max => {
                let per_line =
                    ((max / (self.char_width * style.effective_size())).floor() as usize).max(1);
                let count = text.chars().count();
                let lines = count.div_ceil(per_line);
                (
                    (per_line.min(count)) as f32 * self.char_width * style.effective_size(),
                    lines,
                )
            }
            _ => (width, 1),
        };
        let line_height = style.line_height.unwrap_or(1.2) * style.effective_size();
        let mut lines = Vec::with_capacity(line_count);
        for i in 0..line_count {
            lines.push(Line {
                baseline: Point::new(0.0, (i as f32 + 1.0) * line_height),
                width,
                glyphs: Vec::new(),
                is_last: i + 1 == line_count,
            });
        }
        ShapedText {
            size: Size::new(width, line_count as f32 * line_height),
            lines,
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

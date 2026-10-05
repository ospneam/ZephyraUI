//! 图标：SVG path data 解析 + 图标注册表。
//!
//! # 为什么在核心里
//!
//! 图标是 UI 的**基础词汇**：工具栏、菜单、树、列表到处都要用，而且解析逻辑
//! （SVG 的 `A` 命令要按附录 F.6 从端点参数化转中心参数化再展成三次贝塞尔；数字字面
//! 有十进制与科学计数两种写法；隐式命令可重复）本身与宿主、与应用都无关。
//!
//! 留在应用侧的结果是每个应用都要重写一遍解析器，而重写就意味着重错——尤其是弧
//! 展开，写错时图标形状会**略微**不对，肉眼很难判断是解析错了还是设计稿就这样。
//!
//! # 分层
//!
//! - [`path`]：SVG `d` 属性 → [`Segment`] 序列（零依赖）
//! - [`svg`]：SVG 文本 → [`Icon`]（零依赖，只提取图标级子集）
//! - [`IconSet`]：名字 → 图标数据的注册表（零依赖，进程内解析一次）
//! - [`assets`]：随框架分发的通用字形（`include_str!`，运行时零 IO）
//!
//! **栅格化不在这里。** tiny-skia 与宿主纹理句柄都是渲染栈的东西，放在核心会破坏
//! "核心零依赖"这条不变量——而那正是迁移缝能被验证的前提。栅格化在 `zephyra-iced`。
//!
//! # 不做完整 XML 解析
//!
//! 图标级 SVG 只需要 `viewBox` / `fill` / `fill-rule` 与若干 `<path d>`。完整 XML
//! 解析器要处理命名空间、实体、CDATA、校验，而图标的实际形态高度一致——
//! 「实现全部但用不到的部分」比「实现用得到的部分」更容易出 bug。

pub mod assets;
pub mod path;
mod registry;
pub mod svg;

pub use path::{parse_path_data, ParseError};
pub use registry::{IconSet, IconSource};
pub use svg::{parse_svg, SvgError};

/// 一条已展平的路径指令。曲线只保留三次/二次贝塞尔两种形式，弧已离线展开。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Segment {
    /// 开启新子路径
    MoveTo(f32, f32),
    /// 直线到绝对坐标
    LineTo(f32, f32),
    /// 三次贝塞尔：c1x c1y c2x c2y x y（均为绝对坐标）
    CubicTo(f32, f32, f32, f32, f32, f32),
    /// 二次贝塞尔：cx cy x y（均为绝对坐标）
    QuadraticTo(f32, f32, f32, f32),
    /// 闭合当前子路径（当前点回到子路径起点）
    Close,
}

/// 填充规则（对应 SVG `fill-rule`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FillRule {
    NonZero,
    EvenOdd,
}

/// 填充模式（对应 SVG `fill`）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PaintMode {
    /// `currentColor` 或未指定：渲染端给定颜色。
    ///
    /// 图标必须走这一档而非把颜色烤进纹理：工具栏按钮会随选中/hover 变色，
    /// 而图标是静态内容（不重新栅格化）——颜色只能在绘制时给。
    CurrentColor,
    /// 具体颜色（RGBA，0..1）
    Solid([f32; 4]),
    /// `fill="none"`：该元素不参与填充
    None,
}

/// 填充样式：模式 + 规则。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Paint {
    /// 填充模式
    pub mode: PaintMode,
    /// 填充规则
    pub rule: FillRule,
}

impl Paint {
    /// `currentColor` + 非零环绕。
    pub const CURRENT_COLOR: Paint = Paint {
        mode: PaintMode::CurrentColor,
        rule: FillRule::NonZero,
    };
}

impl Default for Paint {
    fn default() -> Self {
        Self::CURRENT_COLOR
    }
}

/// 解析后的图标：viewBox + 填充样式 + 展平路径段。
#[derive(Debug, Clone, PartialEq)]
pub struct Icon {
    /// (min_x, min_y, width, height)
    pub view_box: (f32, f32, f32, f32),
    /// 填充样式
    pub paint: Paint,
    /// 展平后的路径段
    pub segments: Vec<Segment>,
}

impl Icon {
    /// 空图标（viewBox 24×24，无路径段），可作 const 兜底值。
    ///
    /// 未知图标名回落到它，而不是 panic 或缺资源：一个拼错的名字不该让整个
    /// 界面崩掉，且"某个图标不显示"是可见、可对比的；崩掉不是。
    pub const EMPTY: Icon = Icon {
        view_box: (0.0, 0.0, 24.0, 24.0),
        paint: Paint::CURRENT_COLOR,
        segments: Vec::new(),
    };

    /// 是否有可填充的内容。
    ///
    /// `fill="none"` 的图标与解析失败都落在这里；渲染端据此跳过而不是
    /// 画出一块空白纹理占位。
    pub fn is_empty(&self) -> bool {
        self.segments.is_empty() || self.paint.mode == PaintMode::None
    }
}
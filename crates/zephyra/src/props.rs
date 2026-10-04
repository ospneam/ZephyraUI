//! 标准属性键。
//!
//! 单独成模块而不是散落在 `tree` / `paint` 里，因为**三方都要用**：
//! 布局读尺寸与间距、绘制读外观、控件层写默认值。集中在这里意味着改一个键的
//! 语义只需要看一个文件，而"这个键到底影响重排还是只影响重绘"（见
//! [`crate::property::PropertyChange::affects_layout`]）也能一并对照。
//!
//! 全部是 `const`：属性定义是编译期常量，全树共享同一份身份，diff 靠指针相等配对。

use crate::core::{Color, CornerRadius};
use crate::layout::{AlignItems, Direction, MainAxisAlign};
use crate::property::{PropertyKey, PropertyValue};

// ============================== 布局 ==============================

/// 内边距（四边统一值）。
///
/// 四边统一是刻意的简化：UI 里绝大多数控件的 padding 是对称的。真需要非对称时
/// 把它扩成 `Thickness` 变体，而不是现在就为它把键拆成四个。
pub static PADDING: PropertyKey =
    PropertyKey::inheritable("Padding", PropertyValue::float(0.0));

/// 显式宽度（DIP）。`None` 表示由布局决定。
pub static WIDTH: PropertyKey = PropertyKey::new("Width", PropertyValue::float(f32::NAN));

/// 显式高度（DIP）。
pub static HEIGHT: PropertyKey = PropertyKey::new("Height", PropertyValue::float(f32::NAN));

/// 堆叠方向（仅对容器生效）。
pub static DIRECTION: PropertyKey = PropertyKey::new(
    "Direction",
    PropertyValue::direction(Direction::Horizontal),
);

/// 主轴上相邻子项的间距。
pub static SPACING: PropertyKey =
    PropertyKey::new("Layout.Spacing", PropertyValue::float(0.0));

/// 交叉轴对齐（由容器统一施加给子项）。
pub static CROSS_ALIGN: PropertyKey = PropertyKey::new(
    "CrossAlign",
    PropertyValue::cross_align(AlignItems::Start),
);

/// 主轴对齐（由容器施加给整组子项）。
pub static MAIN_ALIGN: PropertyKey = PropertyKey::new(
    "MainAlign",
    PropertyValue::main_align(MainAxisAlign::Start),
);

/// 子节点参与布局（`false` 时不占位、不绘制）。
///
/// 对应 WinUI 的 `Visibility=Collapsed`。与"宽度为 0"不同：零宽仍占一个间距位，
/// 而折叠完全不占——工具栏在图片模式下少一个按钮时，间距必须跟着消失。
pub static COLLAPSED: PropertyKey = PropertyKey::new("Collapsed", PropertyValue::bool(false));

// ============================== 外观 ==============================

/// 背景填充色。`Color(0,0,0,0)` 表示不画背景。
pub static BACKGROUND: PropertyKey =
    PropertyKey::new("Background", PropertyValue::color(Color::rgba(0.0, 0.0, 0.0, 0.0)));

/// 圆角半径（DIP）。
pub static CORNER_RADIUS: PropertyKey =
    PropertyKey::new("CornerRadius", PropertyValue::float(0.0));

/// 描边颜色。alpha 为 0 表示不描边。
pub static BORDER_COLOR: PropertyKey =
    PropertyKey::new("BorderColor", PropertyValue::color(Color::rgba(0.0, 0.0, 0.0, 0.0)));

/// 描边宽度（DIP）。
pub static BORDER_WIDTH: PropertyKey =
    PropertyKey::new("BorderWidth", PropertyValue::float(0.0));

/// 整体不透明度。
pub static OPACITY: PropertyKey = PropertyKey::inheritable("Opacity", PropertyValue::float(1.0));

/// 合成层（决定后端如何切 render pass）。
pub static LAYER: PropertyKey = PropertyKey::new(
    "Layer",
    PropertyValue::layer(crate::paint::Layer::Content),
);

// ============================== 内容 ==============================

/// 图像纹理句柄（宿主定义）。
pub static IMAGE_TEXTURE: PropertyKey = PropertyKey::new(
    "Image.Texture",
    PropertyValue::static_str(""),
);

/// 图像绘制矩形（DIP 边长，正方形）。`-1` 表示"撑满节点矩形"。
pub static IMAGE_SIZE: PropertyKey =
    PropertyKey::new("Image.Size", PropertyValue::float(-1.0));

/// 图像着色（乘算）。
pub static IMAGE_TINT: PropertyKey = PropertyKey::new(
    "Image.Tint",
    PropertyValue::color(Color::rgb(1.0, 1.0, 1.0)),
);

// ============================== 交互 ==============================

/// 是否可交互。`false` 的节点只作为事件路径的一部分返回，不接收指针。
pub static INTERACTIVE: PropertyKey = PropertyKey::new("Interactive", PropertyValue::bool(false));

/// 是否处于选中态（控件层据此换用 `control_fill_pressed` 之类的资源）。
///
/// 之所以是**属性**而不是控件内部字段：选中态要能被样式与资源解析影响，而
/// `ThemeResource` 只能作用于属性。
pub static SELECTED: PropertyKey = PropertyKey::new("Selected", PropertyValue::bool(false));

/// 是否禁用（禁用态的文字与填充取 `content_disabled` / 更低的填充层）。
pub static DISABLED: PropertyKey = PropertyKey::new("Disabled", PropertyValue::bool(false));

/// 把 `SELECTED` / `DISABLED` 翻译成该用的填充色资源键。
///
/// 这就是控件"可重样式"的最小实现：控件不自己写死选中色，而是问"当前状态下我
/// 该用哪个语义角色"。于是换主题只改 `Theme`，控件代码一行不动。
pub fn control_fill_role(selected: bool, disabled: bool, hovered: bool) -> &'static str {
    if disabled {
        return crate::theme::roles::SURFACE_ALT;
    }
    if selected {
        return crate::theme::roles::CONTROL_FILL_PRESSED;
    }
    if hovered {
        return crate::theme::roles::CONTROL_FILL_HOVER;
    }
    crate::theme::roles::CONTROL_FILL
}

/// 便捷构造：带圆角与背景的填充节点描述。
pub fn fill_props(background: PropertyValue, radius: f32) -> Vec<(&'static PropertyKey, PropertyValue)> {
    vec![
        (&BACKGROUND, background),
        (&CORNER_RADIUS, PropertyValue::float(radius)),
    ]
}

/// 便捷构造：描边。
pub fn border_props(color: PropertyValue, width: f32) -> Vec<(&'static PropertyKey, PropertyValue)> {
    vec![
        (&BORDER_COLOR, color),
        (&BORDER_WIDTH, PropertyValue::float(width)),
    ]
}

/// 便捷构造：固定尺寸。
pub fn size_props(width: f32, height: f32) -> Vec<(&'static PropertyKey, PropertyValue)> {
    vec![
        (&WIDTH, PropertyValue::float(width)),
        (&HEIGHT, PropertyValue::float(height)),
    ]
}

/// 未使用的占位，避免 `CornerRadius` 导入被判定为 dead code。
///
/// `CornerRadius` 目前只作为文档语义出现（绘制命令用的是裸 `f32` 半径），
/// 但它属于本模块的公开词汇表，保留导入以便后续 `CORNER_RADIUS` 改用该类型。
#[allow(dead_code)]
type RadiusDoc = CornerRadius;
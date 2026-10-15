//! 标准属性键。
//!
//! 单独成模块而不是散落在 `tree` / `paint` 里，因为**三方都要用**：
//! 布局读尺寸与间距、绘制读外观、控件层写默认值。集中在这里意味着改一个键的
//! 语义只需要看一个文件，而"这个键到底影响重排还是只影响重绘"（见
//! [`crate::property::PropertyChange::affects_layout`]）也能一并对照。
//!
//! 全部是 `const`：属性定义是编译期常量，全树共享同一份身份，diff 靠指针相等配对。

use crate::core::{Color, CornerRadius, UvRect};
use crate::layout::{AlignItems, Direction, MainAxisAlign};
use crate::property::{PropertyKey, PropertyValue};

// ============================== 布局 ==============================

/// 内边距（四边统一值）。
///
/// 四边统一是刻意的简化：UI 里绝大多数控件的 padding 是对称的。真需要非对称时
/// 把它扩成 `Thickness` 变体，而不是现在就为它把键拆成四个。
pub static PADDING: PropertyKey =
    PropertyKey::inheritable_layout("Padding", PropertyValue::float(0.0));

/// 显式宽度（DIP）。`None` 表示由布局决定。
pub static WIDTH: PropertyKey = PropertyKey::layout("Width", PropertyValue::float(f32::NAN));

/// 显式高度（DIP）。
pub static HEIGHT: PropertyKey = PropertyKey::layout("Height", PropertyValue::float(f32::NAN));

/// 堆叠方向（仅对容器生效）。
pub static DIRECTION: PropertyKey =
    PropertyKey::layout("Direction", PropertyValue::direction(Direction::Horizontal));

/// 主轴上相邻子项的间距。
pub static SPACING: PropertyKey = PropertyKey::layout("Spacing", PropertyValue::float(0.0));

/// 交叉轴对齐（由容器统一施加给子项）。
pub static CROSS_ALIGN: PropertyKey =
    PropertyKey::layout("CrossAlign", PropertyValue::cross_align(AlignItems::Start));

/// 主轴对齐（由容器施加给整组子项）。
pub static MAIN_ALIGN: PropertyKey =
    PropertyKey::layout("MainAlign", PropertyValue::main_align(MainAxisAlign::Start));

/// 子节点参与布局（`false` 时不占位、不绘制）。
///
/// 对应 WinUI 的 `Visibility=Collapsed`。与"宽度为 0"不同：零宽仍占一个间距位，
/// 而折叠完全不占——工具栏在图片模式下少一个按钮时，间距必须跟着消失。
pub static COLLAPSED: PropertyKey = PropertyKey::layout("Collapsed", PropertyValue::bool(false));

// ============================== 外观 ==============================

/// 背景填充色。`Color(0,0,0,0)` 表示不画背景。
pub static BACKGROUND: PropertyKey = PropertyKey::new(
    "Background",
    PropertyValue::color(Color::rgba(0.0, 0.0, 0.0, 0.0)),
);

/// 圆角半径（DIP）。
pub static CORNER_RADIUS: PropertyKey = PropertyKey::new("CornerRadius", PropertyValue::float(0.0));

/// 描边颜色。alpha 为 0 表示不描边。
pub static BORDER_COLOR: PropertyKey = PropertyKey::new(
    "BorderColor",
    PropertyValue::color(Color::rgba(0.0, 0.0, 0.0, 0.0)),
);

/// 描边宽度（DIP）。
pub static BORDER_WIDTH: PropertyKey = PropertyKey::new("BorderWidth", PropertyValue::float(0.0));

/// 整体不透明度。
pub static OPACITY: PropertyKey = PropertyKey::inheritable("Opacity", PropertyValue::float(1.0));

/// 绘制时 X 偏移（DIP）。
///
/// # 为什么**不是**布局属性
///
/// `affects_layout() == false` 是这个属性的**全部意义**：位移只改画面，
/// 不改布局盒、不改命中测试。声明成布局属性会让入场动画每帧触发一次全树重排
/// （实测 demo 45 节点 ≈ 一趟 StackPanel 遍历），而动画要的恰恰是"不重排"。
///
/// 于是动画期间控件仍然可点、命中测试仍然按动画前的位置走 —— 这与浏览器上
/// `transform` 的行为一致（`transform` 不参与布局，`will-change` 存在的理由正是
/// 避免每帧重排）。
pub static TRANSLATE_X: PropertyKey = PropertyKey::new("TranslateX", PropertyValue::float(0.0));

/// 绘制时 Y 偏移（DIP）。见 [`TRANSLATE_X`]。
pub static TRANSLATE_Y: PropertyKey = PropertyKey::new("TranslateY", PropertyValue::float(0.0));

/// 合成层（决定后端如何切 render pass）。
pub static LAYER: PropertyKey =
    PropertyKey::new("Layer", PropertyValue::layer(crate::paint::Layer::Content));

// ============================== 效果 ==============================

/// 滤镜链。
///
/// 不可继承：滤镜是**节点自身的**外观属性，父节点设了模糊不该让整棵子树跟着
/// 糊掉（那正是"可继承"会带来灾难的地方 —— `Opacity` 可继承是因为半透明叠加
/// 天然正确，模糊叠加则完全不成立）。
pub static FILTER: PropertyKey = PropertyKey::new(
    "Filter",
    PropertyValue::filter(crate::filter::FilterChain::none()),
);

/// 混合模式（默认 [`crate::filter::BlendMode::SrcOver`]，即普通 alpha 合成）。
///
/// 同样不可继承：混合模式描述的是"这一项与它**下面**那层的合成方式"，而"下面"
/// 随父链变化，继承下来语义就乱了。
pub static BLEND_MODE: PropertyKey = PropertyKey::new(
    "BlendMode",
    PropertyValue::blend_mode(crate::filter::BlendMode::SrcOver),
);

// ============================== 内容 ==============================

/// 图像纹理句柄（宿主定义）。
pub static IMAGE_TEXTURE: PropertyKey =
    PropertyKey::new("Image.Texture", PropertyValue::static_str(""));

/// 图像绘制矩形（DIP 边长，正方形）。`-1` 表示"撑满节点矩形"。
pub static IMAGE_SIZE: PropertyKey = PropertyKey::new("Image.Size", PropertyValue::float(-1.0));

/// 图像着色（乘算）。
pub static IMAGE_TINT: PropertyKey = PropertyKey::new(
    "Image.Tint",
    PropertyValue::color(Color::rgb(1.0, 1.0, 1.0)),
);

/// 图像源裁剪（归一化 uv，`0,0,1,1` = 整张）。
///
/// 九宫格（`Slice`）、雪碧图图集、SVG viewBox 都靠它取一个子区域。归一化而非
/// 像素，因为宿主上传的纹理尺寸不由核心决定 —— 核心若存像素值，同一张图换个
/// 分辨率就要重新设一遍。
pub static IMAGE_SRC: PropertyKey =
    PropertyKey::new("Image.Src", PropertyValue::uv_rect(UvRect::FULL));

// ============================== 矢量图标 ==============================

/// 内置图标名（[`crate::icon::assets::REGISTRY`] 里的键）。
///
/// # 为什么图标名是**资源键**而不是内联路径数据
///
/// 两条理由：
///
/// 1. **契约层不认识 SVG。** 核心的 `icon` 模块只**解析** path data（`parse_path_data`），
///    而"怎么拿到那段数据"是宿主的事 —— 就像纹理句柄是宿主定义的一样。名字让核心
///    只持一把 `&'static str`，宿主与内置资源表都能满足。
/// 2. **同一个图标可以有多份几何。** 内置表里 `camera` 是一条 path data，但应用
///    自己的图标库也是「名字 → 数据」。用名字作键，两边共用同一条属性。
///
/// # 与 [`IMAGE_TEXTURE`] 的关系
///
/// 两者是**互斥**的图元选择，不是叠加：
///
/// - 图标名 → 走 [`crate::paint::Command::Path`]（CPU 三角化 + MSAA 边缘）；
/// - 纹理句柄 → 走 [`crate::paint::Command::Image`]（采样，位图自带 AA）。
///
/// 同时设两者时**图像胜出**（它不需要三角化，且已有位图缓存）。理由见
/// `crate::paint::render_rec` 里那段的说明 —— 那是一条真会被写出来的歧义，
/// 所以判据写在这里而不留在实现里。
pub static ICON_NAME: PropertyKey = PropertyKey::new("Icon.Name", PropertyValue::static_str(""));

/// 图标填充色。
///
/// 不继承：图标的颜色由**该图标所在的控件**决定（按钮前景色），而继承链上的
/// 文本色未必是同一个语义（同一行里按钮与标签常常不同色）。
pub static ICON_COLOR: PropertyKey = PropertyKey::new(
    "Icon.Color",
    PropertyValue::color(Color::rgb(1.0, 1.0, 1.0)),
);

// ============================== 文本 ==============================

/// 文本内容。
///
/// 值是字符串本身（`StaticStr` 或 `Str`），**不是**资源键 —— 文本不是设计令牌，
/// 不该进主题。颜色与字号才是。
pub static TEXT: PropertyKey = PropertyKey::new("Text", PropertyValue::static_str(""));

/// 字号（DIP）。`NaN` 表示"用宿主默认"。
pub static TEXT_SIZE: PropertyKey =
    PropertyKey::layout("Text.Size", PropertyValue::float(f32::NAN));

/// 文本颜色（继承）。
pub static TEXT_COLOR: PropertyKey = PropertyKey::inheritable(
    "Text.Color",
    PropertyValue::color(Color::rgb(1.0, 1.0, 1.0)),
);

/// 文本最大宽度（DIP），用于断行。`NaN` 表示不限宽（不换行）。
pub static TEXT_MAX_WIDTH: PropertyKey =
    PropertyKey::layout("Text.MaxWidth", PropertyValue::float(f32::NAN));

/// 文本行高倍数（继承）。`NaN` 表示用字体自带值。
pub static TEXT_LINE_HEIGHT: PropertyKey =
    PropertyKey::inheritable_layout("Text.LineHeight", PropertyValue::float(f32::NAN));

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
pub fn fill_props(
    background: PropertyValue,
    radius: f32,
) -> Vec<(&'static PropertyKey, PropertyValue)> {
    vec![
        (&BACKGROUND, background),
        (&CORNER_RADIUS, PropertyValue::float(radius)),
    ]
}

/// 便捷构造：描边。
pub fn border_props(
    color: PropertyValue,
    width: f32,
) -> Vec<(&'static PropertyKey, PropertyValue)> {
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

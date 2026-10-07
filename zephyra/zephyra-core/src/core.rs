//! 几何与排版基元。
//!
//! 这些类型被所有其他模块使用，所以它们必须先定死——一旦上层开始依赖某个形状，
//! 再改就是全仓破坏。
//!
//! ## 命名取自 WinUI
//!
//! 借用成熟系统的命名是为了降低认知成本：`Thickness`（四边内边距/外边距）、
//! `CornerRadius`、`Spacing`。但**不**照搬 XAML 的字符串解析式语法——Rust 的
//! 类型系统已经覆盖了它。

/// 长度基元。
///
/// 刻意**不**用 `f32` 直接表达尺寸：布局里混用裸 `f32` 与「像素 / 密度无关像素」
/// 是 UI bug 的常见来源（高 DPI 下边框细一半）。需要明确单位时用
/// [`Length::Px`] 与 [`Length::Dip`]。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Length {
    /// 填充可用空间。
    #[default]
    Fill,
    /// 按内容计算。
    Shrink,
    /// 绝对像素。
    Px(f32),
    /// 密度无关像素（高 DPI 下由宿主乘以缩放因子）。
    Dip(f32),
    /// 占父级的比例（`0.0..=1.0`）。
    Fraction(f32),
}

/// 四边厚度。
///
/// 用一个类型而不是四个 `f32`，是为了让「水平边距」与「垂直边距」在类型层面
/// 可交换——这正是 WinUI 引入 `Thickness` 统一 `Padding`/`Margin` 的原因。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Thickness {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl Thickness {
    /// 四边相同。
    pub const fn uniform(v: f32) -> Self {
        Self {
            left: v,
            top: v,
            right: v,
            bottom: v,
        }
    }

    /// 仅水平方向。
    pub const fn horizontal(v: f32) -> Self {
        Self::uniform(v)
    }

    /// 仅垂直方向。
    pub const fn vertical(v: f32) -> Self {
        Self::uniform(v)
    }
}

/// 圆角半径（四角相同）。
///
/// 矩形/边框的圆角由宿主在 quad shader 内以 SDF 求出，因此本类型只是**描述**，
/// 不参与几何细分——这是它与「圆角矩形路径」的本质区别。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CornerRadius {
    pub radius: f32,
}

/// 间距标度。
///
/// 4pt 栅格。刻度值来自 WinUI 的 `Spacing` 资源键，取值 0/2/4/8/12/16/20/24/28/32。
///
/// 存在的理由很直接：应用里散落着 `padding([6, 10])`、`padding([10, 16])` 这类
/// 各自发明的数值，没有任何一个是设计过的。令牌化之后「控件内边距」是一个
/// **语义名**，而不是一个要靠记忆选的小数字。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Spacing {
    None,
    XXSmall,
    XSmall,
    Small,
    Medium,
    Large,
    XLarge,
    XXLarge,
}

impl Spacing {
    /// 该刻度的 DIP 值。
    pub fn dip(self) -> f32 {
        match self {
            Spacing::None => 0.0,
            Spacing::XXSmall => 2.0,
            Spacing::XSmall => 4.0,
            Spacing::Small => 8.0,
            Spacing::Medium => 12.0,
            Spacing::Large => 16.0,
            Spacing::XLarge => 20.0,
            Spacing::XXLarge => 24.0,
        }
    }
}

/// 线性 RGBA 颜色。
///
/// 内部一律**线性**空间：直接拿 sRGB 数值当线性用会让两层半透明叠加后的亮度
/// 明显偏暗（这是 sRGB 出现的原因）。宿主在写入非 sRGB 表面时负责编码回去。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    /// 构造不透明颜色。
    pub const fn rgb(r: f32, g: f32, b: f32) -> Self {
        Self { r, g, b, a: 1.0 }
    }

    /// 构造带透明度的颜色。
    pub const fn rgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    /// 与 `other` 按 `t ∈ 0..=1` 线性插值（`t=0` 取 `self`）。
    pub fn lerp(self, other: Color, t: f32) -> Color {
        let t = t.clamp(0.0, 1.0);
        Color {
            r: self.r + (other.r - self.r) * t,
            g: self.g + (other.g - self.g) * t,
            b: self.b + (other.b - self.b) * t,
            a: self.a + (other.a - self.a) * t,
        }
    }

    /// 在**线性空间**叠加（源在上、alpha 合成）。
    ///
    /// 不能用 `lerp` 代替：`lerp` 是颜色的直觉插值，而 alpha 合成遵循
    /// `out = src + dst·(1 − src.a)`。半透明层叠时两者结果明显不同。
    pub fn over(self, dst: Color) -> Color {
        let out_a = self.a + dst.a * (1.0 - self.a);
        if out_a <= f32::EPSILON {
            return Color::rgba(0.0, 0.0, 0.0, 0.0);
        }
        let inv = 1.0 / out_a;
        Color {
            r: (self.r * self.a + dst.r * dst.a * (1.0 - self.a)) * inv,
            g: (self.g * self.a + dst.g * dst.a * (1.0 - self.a)) * inv,
            b: (self.b * self.a + dst.b * dst.a * (1.0 - self.a)) * inv,
            a: out_a,
        }
    }

    /// 以 `factor` 调整 alpha（`factor > 1` 会被截断到 1）。
    pub fn with_alpha(self, factor: f32) -> Color {
        Color {
            a: (self.a * factor).min(1.0),
            ..self
        }
    }
}

/// 矩形（左上角 + 尺寸）。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    /// 构造矩形。
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// 右边界。
    pub fn right(&self) -> f32 {
        self.x + self.width
    }

    /// 下边界。
    pub fn bottom(&self) -> f32 {
        self.y + self.height
    }

    /// 与 `other` 求交集；不相交返回 `None`（而不是空矩形——空矩形在后续
    /// 布局里会变成难以追踪的零尺寸）。
    pub fn intersect(&self, other: &Rect) -> Option<Rect> {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        (right > x && bottom > y).then(|| Rect::new(x, y, right - x, bottom - y))
    }

    /// 按 `inset`（四边内缩）收缩。
    pub fn inset(&self, inset: Thickness) -> Rect {
        Rect::new(
            self.x + inset.left,
            self.y + inset.top,
            (self.width - inset.left - inset.right).max(0.0),
            (self.height - inset.top - inset.bottom).max(0.0),
        )
    }
}

/// 纹理内的归一化矩形（uv）。
///
/// 刻意与 [`Rect`] 分成两个类型：两者字段完全相同，但**单位与坐标系不同** ——
/// [`Rect`] 是 DIP 的屏幕矩形，原点是左上且 y 向下；`UvRect` 是 0..1 的纹理
/// 比例，原点在左上但 y 方向取决于纹理上传时有没有翻转。
///
/// 合成一个类型等于把这两个坐标系混在一起，而混起来的代价是"忘了翻转"这种
/// 静默错误：图形上下颠倒，除此之外没有任何异常。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UvRect {
    /// 左边界（0..1）。
    pub x: f32,
    /// 上边界（0..1）。**y 向下**，与 [`Rect`] 一致。
    pub y: f32,
    /// 宽度（0..1）。
    pub width: f32,
    /// 高度（0..1）。
    pub height: f32,
}

impl UvRect {
    /// 整张纹理。
    pub const FULL: UvRect = UvRect {
        x: 0.0,
        y: 0.0,
        width: 1.0,
        height: 1.0,
    };

    /// 构造 uv 矩形。
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// 右边界（0..1）。
    pub fn right(&self) -> f32 {
        self.x + self.width
    }

    /// 下边界（0..1）。
    pub fn bottom(&self) -> f32 {
        self.y + self.height
    }

    /// 四边都在 `0..1` 内且面积为正。
    ///
    /// 这三个条件缺一不可：只判边界会让零尺寸 uv 通过（采样器处表现为完全不显示），
    /// 而零尺寸与越界的症状都出现在**采样之后**，那时已经离病因很远。
    pub fn is_in_range(&self) -> bool {
        self.x >= 0.0
            && self.y >= 0.0
            && self.right() <= 1.0
            && self.bottom() <= 1.0
            && self.width > 0.0
            && self.height > 0.0
    }

    /// 是否为整张纹理（可省略 `Command::Image` 的 `src`）。
    pub fn is_full(&self) -> bool {
        *self == Self::FULL
    }

    /// 钳到 `0..1`，并夹正零/负尺寸。
    ///
    /// 越界 uv 的危害在采样器处才显现（取到相邻图块 / 整个不显示），而那时已经
    /// 离病因很远。所以在**这里**钳一次，症状留在产生它的地方。
    pub fn clamped(&self) -> UvRect {
        let x = self.x.clamp(0.0, 1.0);
        let y = self.y.clamp(0.0, 1.0);
        UvRect {
            x,
            y,
            width: self.width.clamp(0.0, 1.0 - x),
            height: self.height.clamp(0.0, 1.0 - y),
        }
    }
}

impl Default for UvRect {
    fn default() -> Self {
        Self::FULL
    }
}

/// 点。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    /// 构造点。
    /// 零点。
    ///
    /// 「不位移」在离屏渲染里是**常用的默认值**（展开一帧到屏幕时平移量就是零），
    /// 所以给它具名常量而不是到处写 `Point::new(0.0, 0.0)` —— 后者读起来像
    /// 「平移到屏幕左上角」，而 `ZERO` 读起来像「不平移」。
    pub const ZERO: Point = Point { x: 0.0, y: 0.0 };

    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// 尺寸。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Size {
    pub width: f32,
    pub height: f32,
}

impl Size {
    /// 构造尺寸。
    pub const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }
}

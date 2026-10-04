//! 设计令牌与主题。
//!
//! # 解决的是什么问题
//!
//! 没有主题系统时，一个中等规模应用的典型状态是：散落着上百处硬编码颜色、
//! 二十来处各自发明的间距、几十个重复实现的按钮 hover 态。改一次品牌色要动几十个
//! 文件；加深色模式根本不存在（只能整体反色，文字对比度崩掉）。
//!
//! 令牌化之后：
//!
//! - 颜色只有一处真相（[`Theme`]），且**每个令牌有语义名**而非色值名——
//!   `surface` 而不是 `gray_800`；
//! - Light/Dark 是两套 [`Theme`] 值的字段，切换即换整套；
//! - 控件从令牌取值，所以换主题不需要碰任何控件代码。
//!
//! # 语义分层
//!
//! 令牌分三层，因为它们的**稳定性**不同：
//!
//! 1. **基础层**（`palette`）：原始色板，如 `neutral_100`。只有主题定义该用哪一
//!    个基础色时才引用它。
//! 2. **语义层**（`surface` / `content` / `accent` / `stroke`）：角色名。
//!    控件**只**引用这一层——这是换主题不碰控件代码的前提。
//! 3. **控件层**：`button.background.rest` 这类具体槽位。为将来的可重样式控件
//!    预留；控件实现落地后再启用。
//!
//! # 为什么切主题不用遍历控件树
//!
//! [`property::PropertyValue::resource`] 让属性存「资源键」而不是字面值，
//! 主题切换时只需重解析这些键并重算受影响的子树（由 dirty 标记驱动），而不是
//! 无条件重建整棵树。

use crate::core::{Color, Spacing};

/// 主题变体。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ThemeVariant {
    /// 浅色。
    Light,
    /// 深色。
    Dark,
}

impl ThemeVariant {
    /// 相反的变体。
    pub fn toggled(self) -> ThemeVariant {
        match self {
            ThemeVariant::Light => ThemeVariant::Dark,
            ThemeVariant::Dark => ThemeVariant::Light,
        }
    }
}

/// 基础色板。
///
/// 刻意保持窄（冷/暖中性 + 一个强调色相），而不是完整的 20 色调色板：色板越宽，
/// 「该用哪个」的决策成本越高，最后就会退回到硬编码。这里的每个色阶都按
/// WCAG 相对亮度分级，深浅两套主题各自挑用。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    /// 中性色 0（最浅）。
    pub neutral_0: Color,
    /// 中性色 100。
    pub neutral_100: Color,
    /// 中性色 200。
    pub neutral_200: Color,
    /// 中性色 300。
    pub neutral_300: Color,
    /// 中性色 400（适合描边）。
    pub neutral_400: Color,
    /// 中性色 500（次要文本）。
    pub neutral_500: Color,
    /// 中性色 600（正文）。
    pub neutral_600: Color,
    /// 中性色 700。
    pub neutral_700: Color,
    /// 中性色 800（浅色主题的 surface）。
    pub neutral_800: Color,
    /// 中性色 900。
    pub neutral_900: Color,
    /// 强调色浅档（选中/激活背景）。
    pub accent_light: Color,
    /// 强调色主档。
    pub accent: Color,
    /// 强调色深档（强调色上的文本）。
    pub accent_dark: Color,
}

/// 一整套主题令牌。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Theme {
    /// 变体标识。
    pub variant: ThemeVariant,
    /// 基础色板。
    pub palette: Palette,
    /// 语义层：页面/窗口底色。
    pub surface: Color,
    /// 语义层：次级底色（卡片、输入框内）。
    pub surface_alt: Color,
    /// 语义层：主要文本。
    pub content: Color,
    /// 语义层：次要文本。
    pub content_secondary: Color,
    /// 语义层：禁用文本。
    pub content_disabled: Color,
    /// 语义层：描边/分隔线。
    pub stroke: Color,
    /// 语义层：控件静止态填充。
    pub control_fill: Color,
    /// 语义层：控件悬停态填充。
    pub control_fill_hover: Color,
    /// 语义层：控件按下态填充。
    pub control_fill_pressed: Color,
    /// 语义层：强调填充。
    pub accent_fill: Color,
    /// 语义层：强调填充上的文本。
    pub on_accent_fill: Color,
    /// 语义层：焦点环。
    pub focus_ring: Color,
    /// 全局间距刻度（控件外边距等默认间距来源）。
    pub spacing: Spacing,
}

impl Theme {
    /// 按变体取内置主题。
    pub fn builtin(variant: ThemeVariant) -> Theme {
        match variant {
            ThemeVariant::Light => Theme::light(),
            ThemeVariant::Dark => Theme::dark(),
        }
    }

    /// 内置深色主题。
    ///
    /// 色值取自应用现有的硬编码深色（`#1a1a1a` 底、`#e5e5e5` 文本、`#2a2a2a`
    /// 控件填充）——即迁移前的实际观感，所以切换到本框架后视觉不回退。
    pub fn dark() -> Theme {
        let palette = Palette {
            neutral_0: Color::rgb(0.090, 0.098, 0.106),
            neutral_100: Color::rgb(0.137, 0.149, 0.161),
            neutral_200: Color::rgb(0.184, 0.196, 0.212),
            neutral_300: Color::rgb(0.239, 0.255, 0.275),
            neutral_400: Color::rgb(0.400, 0.427, 0.455),
            neutral_500: Color::rgb(0.541, 0.573, 0.604),
            neutral_600: Color::rgb(0.898, 0.898, 0.898),
            neutral_700: Color::rgb(0.694, 0.694, 0.694),
            neutral_800: Color::rgb(0.102, 0.110, 0.118),
            neutral_900: Color::rgb(0.063, 0.071, 0.078),
            accent_light: Color::rgb(0.153, 0.310, 0.639),
            accent: Color::rgb(0.231, 0.510, 0.965),
            accent_dark: Color::rgb(0.882, 0.941, 1.0),
        };
        Theme {
            variant: ThemeVariant::Dark,
            palette,
            surface: palette.neutral_900,
            surface_alt: palette.neutral_800,
            content: palette.neutral_600,
            content_secondary: palette.neutral_500,
            content_disabled: palette.neutral_400,
            stroke: palette.neutral_300,
            control_fill: palette.neutral_100,
            control_fill_hover: palette.neutral_200,
            control_fill_pressed: palette.neutral_300,
            accent_fill: palette.accent,
            on_accent_fill: palette.accent_dark,
            focus_ring: palette.accent,
            spacing: Spacing::Medium,
        }
    }

    /// 内置浅色主题。
    pub fn light() -> Theme {
        let palette = Palette {
            neutral_0: Color::rgb(1.0, 1.0, 1.0),
            neutral_100: Color::rgb(0.973, 0.976, 0.980),
            neutral_200: Color::rgb(0.933, 0.941, 0.949),
            neutral_300: Color::rgb(0.863, 0.882, 0.902),
            neutral_400: Color::rgb(0.541, 0.573, 0.604),
            neutral_500: Color::rgb(0.400, 0.427, 0.455),
            neutral_600: Color::rgb(0.118, 0.133, 0.149),
            neutral_700: Color::rgb(0.290, 0.310, 0.333),
            neutral_800: Color::rgb(0.976, 0.980, 0.984),
            neutral_900: Color::rgb(1.0, 1.0, 1.0),
            accent_light: Color::rgb(0.878, 0.925, 1.0),
            accent: Color::rgb(0.157, 0.388, 0.921),
            accent_dark: Color::rgb(1.0, 1.0, 1.0),
        };
        Theme {
            variant: ThemeVariant::Light,
            palette,
            surface: palette.neutral_0,
            surface_alt: palette.neutral_100,
            content: palette.neutral_600,
            content_secondary: palette.neutral_500,
            content_disabled: palette.neutral_400,
            stroke: palette.neutral_300,
            control_fill: palette.neutral_100,
            control_fill_hover: palette.neutral_200,
            control_fill_pressed: palette.neutral_300,
            accent_fill: palette.accent,
            on_accent_fill: palette.accent_dark,
            focus_ring: palette.accent,
            spacing: Spacing::Medium,
        }
    }

    /// 按语义角色查颜色。
    ///
    /// 这是控件与样式访问主题的**唯一**入口——它们不直接读字段，从而允许将来
    /// 加派生令牌（如 `surface_elevated`）而不改任何调用点。
    pub fn role(&self, name: &str) -> Option<Color> {
        Some(match name {
            "surface" => self.surface,
            "surface_alt" => self.surface_alt,
            "content" => self.content,
            "content_secondary" => self.content_secondary,
            "content_disabled" => self.content_disabled,
            "stroke" => self.stroke,
            "control_fill" => self.control_fill,
            "control_fill_hover" => self.control_fill_hover,
            "control_fill_pressed" => self.control_fill_pressed,
            "accent_fill" => self.accent_fill,
            "on_accent_fill" => self.on_accent_fill,
            "focus_ring" => self.focus_ring,
            _ => return None,
        })
    }
}

/// 语义角色名常量。
///
/// 写成常量而非裸字符串，是为了让「拼错角色名」变成编译错误——裸字符串只会在
/// 运行时安静地返回 `None`，然后控件用上一次的旧颜色继续画。
pub mod roles {
    pub const SURFACE: &str = "surface";
    pub const SURFACE_ALT: &str = "surface_alt";
    pub const CONTENT: &str = "content";
    pub const CONTENT_SECONDARY: &str = "content_secondary";
    pub const CONTENT_DISABLED: &str = "content_disabled";
    pub const STROKE: &str = "stroke";
    pub const CONTROL_FILL: &str = "control_fill";
    pub const CONTROL_FILL_HOVER: &str = "control_fill_hover";
    pub const CONTROL_FILL_PRESSED: &str = "control_fill_pressed";
    pub const ACCENT_FILL: &str = "accent_fill";
    pub const ON_ACCENT_FILL: &str = "on_accent_fill";
    pub const FOCUS_RING: &str = "focus_ring";
}

/// 运行时主题状态：当前主题 + 切换。
#[derive(Debug, Clone)]
pub struct ThemeContext {
    current: Theme,
}

impl ThemeContext {
    /// 以某个变体初始化。
    pub fn new(variant: ThemeVariant) -> Self {
        Self {
            current: Theme::builtin(variant),
        }
    }

    /// 当前主题。
    pub fn theme(&self) -> &Theme {
        &self.current
    }

    /// 切换主题变体，返回是否发生变化。
    pub fn toggle(&mut self) -> bool {
        let next = self.current.variant.toggled();
        if next == self.current.variant {
            return false;
        }
        self.current = Theme::builtin(next);
        true
    }
}

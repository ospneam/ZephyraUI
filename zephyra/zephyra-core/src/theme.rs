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
//! [`crate::property::PropertyValue::Resource`] 让属性存「资源键」而不是字面值，
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
    /// 语义层：焦点环（外圈，强调色那一圈）。
    pub focus_ring: Color,
    /// 语义层：焦点环内圈（RinUI 的 focusBorderInner，深/浅主题各一极）。
    pub focus_inner: Color,
    /// 语义层：卡片底（RinUI cardColor，半透明叠加在 surface 上）。
    pub card: Color,
    /// 语义层：卡片次级底。
    pub card_secondary: Color,
    /// 语义层：卡片描边。
    pub card_border: Color,
    /// 语义层：控件次级填充（hover 档）。
    pub control_secondary: Color,
    /// 语义层：控件三级填充（pressed 档）。
    pub control_tertiary: Color,
    /// 语义层：控件强填充（滑条轨道一类的实心底）。
    pub control_strong: Color,
    /// 语义层：控件描边。
    pub control_border: Color,
    /// 语义层：控件强描边（未选中复选框的框）。
    pub control_border_strong: Color,
    /// 语义层：分隔线。
    pub divider: Color,
    /// 语义层：浮层底（RinUI layerColor，内容卡片那一层）。
    pub layer: Color,
    /// 语义层：成功严重级色（InfoBadge / ProgressBar）。
    pub success: Color,
    /// 语义层：注意严重级色。
    pub caution: Color,
    /// 语义层：严重错误色。
    pub critical: Color,
    /// 语义层：成功严重级的**背景**档（InfoBar / Toast 卡底）。
    pub success_bg: Color,
    /// 语义层：注意严重级的背景档。
    pub caution_bg: Color,
    /// 语义层：严重错误的背景档。
    pub critical_bg: Color,
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
            // 深色主题：内圈是近黑——focus 环压在浅色卡片上时才看得出（RinUI 的
            // focusBorderInner）。卡片与各级填充的 alpha 全部取自 RinUI dark.qml。
            focus_inner: Color::rgba(0.0, 0.0, 0.0, 0.70),
            card: Color::rgba(1.0, 1.0, 1.0, 0.0512),
            card_secondary: Color::rgba(1.0, 1.0, 1.0, 0.0326),
            card_border: Color::rgba(0.0, 0.0, 0.0, 0.10),
            control_secondary: Color::rgba(1.0, 1.0, 1.0, 0.0837),
            control_tertiary: Color::rgba(1.0, 1.0, 1.0, 0.0419),
            control_strong: Color::rgba(1.0, 1.0, 1.0, 0.5442),
            control_border: Color::rgba(0.0, 0.0, 0.0, 0.09),
            control_border_strong: Color::rgba(1.0, 1.0, 1.0, 0.6047),
            divider: Color::rgba(1.0, 1.0, 1.0, 0.0837),
            layer: Color::rgba(0.227, 0.227, 0.227, 0.30),
            success: Color::rgb(0x6c as f32 / 255.0, 0xcb as f32 / 255.0, 0x5f as f32 / 255.0),
            caution: Color::rgb(0xfc as f32 / 255.0, 0xe1 as f32 / 255.0, 0.00 as f32 / 255.0),
            critical: Color::rgb(0xff as f32 / 255.0, 0x99 as f32 / 255.0, 0xa4 as f32 / 255.0),
            // 严重级**背景**档（InfoBar / Toast 的卡片底，RinUI 的
            // system*BackgroundColor）。深色主题是低饱和暗色调。
            success_bg: Color::rgb(0x39 as f32 / 255.0, 0x3d as f32 / 255.0, 0x1b as f32 / 255.0),
            caution_bg: Color::rgb(0x43 as f32 / 255.0, 0x35 as f32 / 255.0, 0x19 as f32 / 255.0),
            critical_bg: Color::rgb(0x44 as f32 / 255.0, 0x27 as f32 / 255.0, 0x26 as f32 / 255.0),
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
            // 浅色主题的 focus 内圈反过来是近白（RinUI light 的 focusBorderInner）。
            // 卡片在浅底上是「半透明白」，所以卡片的观感是比 surface 更亮的一点。
            focus_inner: Color::rgb(1.0, 1.0, 1.0),
            card: Color::rgba(1.0, 1.0, 1.0, 0.7),
            card_secondary: Color::rgba(0.976, 0.976, 0.976, 0.5),
            card_border: Color::rgba(0.0, 0.0, 0.0, 0.0578),
            control_secondary: Color::rgba(0.976, 0.976, 0.976, 0.5),
            control_tertiary: Color::rgba(0.976, 0.976, 0.976, 0.3),
            control_strong: Color::rgba(0.0, 0.0, 0.0, 0.4458),
            control_border: Color::rgba(0.0, 0.0, 0.0, 0.06),
            control_border_strong: Color::rgba(0.0, 0.0, 0.0, 0.6063),
            divider: Color::rgba(0.0, 0.0, 0.0, 0.0803),
            layer: Color::rgba(1.0, 1.0, 1.0, 0.5),
            success: Color::rgb(0x0f as f32 / 255.0, 0x7b as f32 / 255.0, 0x0f as f32 / 255.0),
            caution: Color::rgb(0x9d as f32 / 255.0, 0x5d as f32 / 255.0, 0x00 as f32 / 255.0),
            critical: Color::rgb(0xc4 as f32 / 255.0, 0x2b as f32 / 255.0, 0x1c as f32 / 255.0),
            // 浅色主题的严重级背景是低饱和亮色调（RinUI light 的
            // system*BackgroundColor）。
            success_bg: Color::rgb(0xdf as f32 / 255.0, 0xf6 as f32 / 255.0, 0xdd as f32 / 255.0),
            caution_bg: Color::rgb(0xff as f32 / 255.0, 0xf4 as f32 / 255.0, 0xce as f32 / 255.0),
            critical_bg: Color::rgb(0xfd as f32 / 255.0, 0xe7 as f32 / 255.0, 0xe9 as f32 / 255.0),
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
            // 半透明变体：让系统背景材质（Mica/Acrylic）从底下透出来。
            //
            // 材质画在窗口**背后**（`DWMWA_SYSTEMBACKDROP_TYPE`），所以它只在
            // "应用没把那块涂满不透明色"时可见。这个令牌就是那个"没涂满"的口子。
            //
            // alpha 取 0.85 而不是更低：Mica 本身已带一层 tint，再压太狠会让
            // 标题栏文字失去对比。0.85 下深色主题标题栏仍清晰，浅色主题也不会
            // 泛白到看不清。
            "surface_alt_translucent" => self.surface_alt.with_alpha(0.85),
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
            "focus_inner" => self.focus_inner,
            "card" => self.card,
            "card_secondary" => self.card_secondary,
            "card_border" => self.card_border,
            "control_secondary" => self.control_secondary,
            "control_tertiary" => self.control_tertiary,
            "control_strong" => self.control_strong,
            "control_border" => self.control_border,
            "control_border_strong" => self.control_border_strong,
            "divider" => self.divider,
            "layer" => self.layer,
            "success" => self.success,
            "caution" => self.caution,
            "critical" => self.critical,
            "success_bg" => self.success_bg,
            "caution_bg" => self.caution_bg,
            "critical_bg" => self.critical_bg,
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
    /// 半透明次级底色（让系统背景材质透出，见 `Theme::role` 同名项）。
    pub const SURFACE_ALT_TRANSLUCENT: &str = "surface_alt_translucent";
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
    pub const FOCUS_INNER: &str = "focus_inner";
    pub const CARD: &str = "card";
    pub const CARD_SECONDARY: &str = "card_secondary";
    pub const CARD_BORDER: &str = "card_border";
    pub const CONTROL_SECONDARY: &str = "control_secondary";
    pub const CONTROL_TERTIARY: &str = "control_tertiary";
    pub const CONTROL_STRONG: &str = "control_strong";
    pub const CONTROL_BORDER: &str = "control_border";
    pub const CONTROL_BORDER_STRONG: &str = "control_border_strong";
    pub const DIVIDER: &str = "divider";
    pub const LAYER: &str = "layer";
    pub const SUCCESS: &str = "success";
    pub const CAUTION: &str = "caution";
    pub const CRITICAL: &str = "critical";
    pub const SUCCESS_BG: &str = "success_bg";
    pub const CAUTION_BG: &str = "caution_bg";
    pub const CRITICAL_BG: &str = "critical_bg";
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

/// 按资源键取值，无该键时返回 `None`。
///
/// 这是**唯一**的资源查找入口：颜色走 [`Theme::role`]，间距走
/// [`Spacing::dip`]，其余一律"无此键"。
pub fn resource_value(theme: &Theme, key: &str) -> Option<crate::property::PropertyValue> {
    use crate::property::PropertyValue;
    if let Some(color) = theme.role(key) {
        return Some(PropertyValue::color(color));
    }
    let spacing = match key {
        "spacing.none" => Some(Spacing::None),
        "spacing.xxsmall" => Some(Spacing::XXSmall),
        "spacing.xsmall" => Some(Spacing::XSmall),
        "spacing.small" => Some(Spacing::Small),
        "spacing.medium" => Some(Spacing::Medium),
        "spacing.large" => Some(Spacing::Large),
        "spacing.xlarge" => Some(Spacing::XLarge),
        "spacing.xxlarge" => Some(Spacing::XXLarge),
        _ => None,
    }?;
    Some(PropertyValue::float(spacing.dip()))
}

/// 解析一个属性值里的资源键，返回**拥有者**。
///
/// # 为什么是返回拥有者而不是写回属性表
///
/// 早先的实现是"遍历属性表，把 `Resource(k)` 就地替换成解析出的字面值"。那个做法
/// 有一个致命后果：**第一次解析之后键就没了**。于是第二次换主题时
/// `is_resource()` 全部为假，什么都不发生——切主题在视觉上成了空操作，而代码读起来
/// 完全正常。
///
/// 保留键、读取时解析，才是"同一棵树换个主题就换配色"能成立的前提。代价是每次读属性
/// 多一次字符串哈希（[`Theme::role`]）；换来的是切主题只需换一个 `&Theme` 引用，
/// 不碰任何节点状态。
///
/// 解析失败**返回 `None` 而不是回退成黑色**：黑是一个合法的颜色，静默回退会让"角色名
/// 拼错"变成一个看不出成因的视觉偏差。调用方据此显式决定降级策略。
pub fn resolve_resource<'v>(
    theme: &Theme,
    value: &'v crate::property::PropertyValue,
) -> Option<std::borrow::Cow<'v, crate::property::PropertyValue>> {
    use std::borrow::Cow;
    let key = value.resource_key()?;
    resource_value(theme, key).map(Cow::Owned)
}

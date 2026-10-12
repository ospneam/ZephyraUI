//! `@令牌` 名单 —— 样式表里能引用的全部主题角色。
//!
//! # 为什么单独一份常量表
//!
//! `resolve_resource` 在角色名拼错时返回 `None`，属性**静默**保持原值 ——
//! 这正是「样式全没了」类故障最安静的成因（demo 那次排查的结论）。
//! 名字写成常量后，门禁可以逐个比对 `Theme::role()`；拼错的令牌在
//! `cargo test` 阶段就红，而不是在用户的屏幕上安静地退回默认值。
//!
//! 与 `zephyra_core::theme::roles` 是同一份名单的两个消费者：
//! 那份给 Rust 代码用，这份给样式表用。**新增角色的唯一入口是 core 的
//! `theme.rs`** —— 这里只做镜像，防止两份漂移。

#![allow(dead_code)]

use zephyra_core::theme::roles;

// ---- 语义层（core 原有） ----
pub const SURFACE: &str = roles::SURFACE;
pub const SURFACE_ALT: &str = roles::SURFACE_ALT;
pub const SURFACE_ALT_TRANSLUCENT: &str = roles::SURFACE_ALT_TRANSLUCENT;
pub const CONTENT: &str = roles::CONTENT;
pub const CONTENT_SECONDARY: &str = roles::CONTENT_SECONDARY;
pub const CONTENT_DISABLED: &str = roles::CONTENT_DISABLED;
pub const STROKE: &str = roles::STROKE;

// ---- 控件填充层 ----
pub const CONTROL_FILL: &str = roles::CONTROL_FILL;
pub const CONTROL_FILL_HOVER: &str = roles::CONTROL_FILL_HOVER;
pub const CONTROL_FILL_PRESSED: &str = roles::CONTROL_FILL_PRESSED;
pub const CONTROL_SECONDARY: &str = roles::CONTROL_SECONDARY;
pub const CONTROL_TERTIARY: &str = roles::CONTROL_TERTIARY;
pub const CONTROL_STRONG: &str = roles::CONTROL_STRONG;

// ---- 描边层 ----
pub const CONTROL_BORDER: &str = roles::CONTROL_BORDER;
pub const CONTROL_BORDER_STRONG: &str = roles::CONTROL_BORDER_STRONG;
pub const DIVIDER: &str = roles::DIVIDER;

// ---- 强调与焦点 ----
pub const ACCENT_FILL: &str = roles::ACCENT_FILL;
pub const ON_ACCENT_FILL: &str = roles::ON_ACCENT_FILL;
pub const FOCUS_RING: &str = roles::FOCUS_RING;
pub const FOCUS_INNER: &str = roles::FOCUS_INNER;

// ---- 卡片与浮层 ----
pub const CARD: &str = roles::CARD;
pub const CARD_SECONDARY: &str = roles::CARD_SECONDARY;
pub const CARD_BORDER: &str = roles::CARD_BORDER;
pub const LAYER: &str = roles::LAYER;

// ---- 严重级（InfoBadge / ProgressBar 三态） ----
pub const SUCCESS: &str = roles::SUCCESS;
pub const CAUTION: &str = roles::CAUTION;
pub const CRITICAL: &str = roles::CRITICAL;
pub const SUCCESS_BG: &str = roles::SUCCESS_BG;
pub const CAUTION_BG: &str = roles::CAUTION_BG;
pub const CRITICAL_BG: &str = roles::CRITICAL_BG;

/// 全部令牌。门禁遍历它逐个对 `Theme::role()`。
pub const ALL: &[&str] = &[
    SURFACE,
    SURFACE_ALT,
    CONTENT,
    CONTENT_SECONDARY,
    CONTENT_DISABLED,
    STROKE,
    CONTROL_FILL,
    CONTROL_FILL_HOVER,
    CONTROL_FILL_PRESSED,
    CONTROL_SECONDARY,
    CONTROL_TERTIARY,
    CONTROL_STRONG,
    CONTROL_BORDER,
    CONTROL_BORDER_STRONG,
    DIVIDER,
    ACCENT_FILL,
    ON_ACCENT_FILL,
    FOCUS_RING,
    FOCUS_INNER,
    CARD,
    CARD_SECONDARY,
    CARD_BORDER,
    LAYER,
    SUCCESS,
    CAUTION,
    CRITICAL,
    SUCCESS_BG,
    CAUTION_BG,
    CRITICAL_BG,
];

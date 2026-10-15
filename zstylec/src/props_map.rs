//! CSS 属性名 → `&'static PropertyKey` 的映射表。
//!
//! # 为什么 CSS 名字和 Rust 里的不一样
//!
//! CSS 惯例是 kebab-case（`corner-radius`），而 `zephyra_core::props` 用的是
//! PascalCase + 点（`CornerRadius`、`Text.Size`）。
//!
//! **用 kebab-case**，不是把 Rust 名直接搬过来。理由：「CSS 的写法 + 我们的语义」
//! 这个边界值这个成本——前端经验直接能用，而映射表本身是集中管理，
//! 加新属性只加一行。
//!
//! # 拼错怎么办
//!
//! 一律**报错**，并给出最接近的几个（见 [`super::errors`]）。绝不静默忽略——
//! 静默忽略的样式表比没有样式表更糟：设计者以为生效了。

use zephyra_core::property::PropertyKey;
use zephyra_core::props;

/// 一条映射：CSS 名 + 对应的属性键 + 「改它要不要重排」的中文说明。
///
/// # 为什么要带说明
///
/// 因为**编译期就能算出**这条声明会不会触发重排，而这对写样式表的人是真信息：
/// 写了 `text-size` 意味着改它要重排，比 `background` 贵。编译时给出来，
/// 写的时候就知道。
pub(crate) struct Prop {
    /// CSS 里的写法（kebab-case）。
    pub css: &'static str,
    pub key: &'static PropertyKey,
}

/// 全部受支持的 CSS 属性名。
///
/// ⚠️ 这张表是**唯一**的入口。加新属性 = 这里加一行，别处不用动。
/// ⚠️ 反过来，`props.rs` 里加了属性但这里没有 → 那个属性**样式表设不了**，
/// 而这不会有任何提示。所以 B2 里那条「props 与映射表对齐」的测试会红。
pub(crate) const PROPS: &[Prop] = &[
    // ---- 尺寸 / 间距（影响布局）----
    Prop {
        css: "width",
        key: &props::WIDTH,
    },
    Prop {
        css: "height",
        key: &props::HEIGHT,
    },
    Prop {
        css: "padding",
        key: &props::PADDING,
    },
    Prop {
        css: "spacing",
        key: &props::SPACING,
    },
    Prop {
        css: "collapsed",
        key: &props::COLLAPSED,
    },
    // ---- 弹性容器（影响布局）----
    Prop {
        css: "direction",
        key: &props::DIRECTION,
    },
    Prop {
        css: "cross-align",
        key: &props::CROSS_ALIGN,
    },
    Prop {
        css: "main-align",
        key: &props::MAIN_ALIGN,
    },
    // ---- 文本（影响布局）----
    Prop {
        css: "text-size",
        key: &props::TEXT_SIZE,
    },
    Prop {
        css: "text-line-height",
        key: &props::TEXT_LINE_HEIGHT,
    },
    Prop {
        css: "text-max-width",
        key: &props::TEXT_MAX_WIDTH,
    },
    // ---- 只重绘 ----
    Prop {
        css: "background",
        key: &props::BACKGROUND,
    },
    Prop {
        css: "corner-radius",
        key: &props::CORNER_RADIUS,
    },
    Prop {
        css: "border-color",
        key: &props::BORDER_COLOR,
    },
    Prop {
        css: "border-width",
        key: &props::BORDER_WIDTH,
    },
    Prop {
        css: "opacity",
        key: &props::OPACITY,
    },
    // 绘制时偏移。**只重绘**：声明成 `PropertyKey::new`（不是 `::layout`），
    // 所以每帧改它不会触发重排 —— 那正是"绘制时插值偏移"的定义。
    Prop {
        css: "translate-x",
        key: &props::TRANSLATE_X,
    },
    Prop {
        css: "translate-y",
        key: &props::TRANSLATE_Y,
    },
    Prop {
        css: "layer",
        key: &props::LAYER,
    },
    Prop {
        css: "filter",
        key: &props::FILTER,
    },
    Prop {
        css: "blend-mode",
        key: &props::BLEND_MODE,
    },
    // ---- 图像 / 图标 ----
    Prop {
        css: "image-texture",
        key: &props::IMAGE_TEXTURE,
    },
    Prop {
        css: "image-size",
        key: &props::IMAGE_SIZE,
    },
    Prop {
        css: "image-tint",
        key: &props::IMAGE_TINT,
    },
    Prop {
        css: "image-src",
        key: &props::IMAGE_SRC,
    },
    Prop {
        css: "icon-name",
        key: &props::ICON_NAME,
    },
    Prop {
        css: "icon-color",
        key: &props::ICON_COLOR,
    },
    // ---- 文本内容 / 颜色 ----
    Prop {
        css: "text",
        key: &props::TEXT,
    },
    Prop {
        css: "text-color",
        key: &props::TEXT_COLOR,
    },
    // ---- 状态（行为，不是外观）----
    // ⚠️ 这三个虽然**不改外观**，但允许出现在样式表里：`:disabled` 一类伪类
    // 需要它们作为「选择依据」。立规禁的是「源码里写样式」，不是「样式表里设状态」。
    Prop {
        css: "interactive",
        key: &props::INTERACTIVE,
    },
    Prop {
        css: "selected",
        key: &props::SELECTED,
    },
    Prop {
        css: "disabled",
        key: &props::DISABLED,
    },
];

/// 按 CSS 名查属性键。
pub(crate) fn lookup(css_name: &str) -> Option<&'static PropertyKey> {
    PROPS.iter().find(|p| p.css == css_name).map(|p| p.key)
}

/// 全部 CSS 名（报错时列最接近的用）。
pub(crate) fn all_css_names() -> impl Iterator<Item = &'static str> {
    PROPS.iter().map(|p| p.css)
}

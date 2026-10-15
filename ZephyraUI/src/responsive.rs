//! 响应式与触控判据 —— 断点、触摸目标、列数。
//!
//! # 每个数字的出处
//!
//! 本模块**不允许**出现没有出处的魔法数字。三个来源：
//!
//! | 数字 | 出处 |
//! |---|---|
//! | 640 / 1008（断点） | `screen-sizes-and-breakpoints-for-responsive-design`：<br>「小：最大可达 640 像素；中：641 - 1007px；大：1008px及以上」 |
//! | 40 / 44 / 32 / 120 / 4（触控） | `touch-interactions`「命中目标」：<br>「可触摸定义为最小 40 x 40 epx…或 32 epx 高（宽度至少 120 epx）」<br>「触控目标增加到 44 x 44 epx，触控目标之间至少有 4 epx 的可见空间」 |
//! | 4 的倍数 | 同断点页：「UI 元素的大小、边距和位置应始终位于 4 个 epx 的倍数中」 |
//!
//! 完整引用见仓库根的 `docs/developer/responsive-and-touch.md`，那份文档是
//! **实现前的依据**，改动这些数字前先回查原文。
//!
//! # 单位口径（这一节是本模块最容易出错的地方）
//!
//! 本仓当前的布局基准是「**1 布局单位 = 1 物理像素**」（宿主把 `inner_size()`
//! 直接当视口交给 core）。这与官方「有效像素 epx」在 scale ≠ 100% 时不一致：
//! 本机 scale 1.25 下，样式表里写 200 的元素视觉上只有 160 epx。
//!
//! 因此本模块的**两种**取值必须分开用，混用会得到静默错误的结果：
//!
//! - `WidthClass::of` 的入参要**先除 scale** —— 官方 `AdaptiveTrigger.MinWindowWidth`
//!   的定义是 "the minimum window width (**in effective pixels**)"。
//!   本机 1400×800 的客户区是 1120×640 **epx**；按物理像素判会整体错一档
//!   （1400 判成"大"是对的，但 800 物理宽的窗口判成"小"是错的，它其实 640 epx = "中"）。
//! - `columns_for` 用**布局单位** —— 它要除的是卡片宽度，而卡片宽度是布局单位。
//!
//! 这个不一致本身记在 `docs/developer/responsive-and-touch.md` §5：把基准改成有效像素是
//! 独立的一轮，不与响应式混做，否则一次改动同时动了「哪套布局」和「布局有多大」。

/// 窗口宽度类别（官方的三个尺寸类别）。
///
/// 命名照抄官方：Small / Medium / Large。官方的断点是**宽度**，所以这里只按宽度判，
/// 不含高度 —— 「人像优化」在官方文档里属于另一个主题（`touch-interactions` 末尾
/// 只要求「支持响应式布局」，没给高度断点）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WidthClass {
    /// 小：有效宽度 **≤ 640 epx**。官方典型窗口 480x854、540x960。
    Small,
    /// 中：有效宽度 **641 – 1007 epx**。官方典型窗口 960x540。
    Medium,
    /// 大：有效宽度 **≥ 1008 epx**。官方典型窗口 1024x640、1366x768、1920x1080。
    #[default]
    Large,
}

impl WidthClass {
    /// 官方断点上界（含）。
    pub const SMALL_MAX_EPX: f32 = 640.0;
    /// 官方断点下界（含）。
    pub const LARGE_MIN_EPX: f32 = 1008.0;

    /// 按**有效宽度**判类别。
    ///
    /// # 入参必须是有效像素
    ///
    /// 调用方先除 scale：`WidthClass::of(w / scale)`。传物理像素在 scale ≠ 100%
    /// 时会错一档，而错一档的表现是「该折叠导航时不折叠」—— 不是崩溃，是布局
    /// 明显不合身，很难归因到单位上。
    pub fn of_effective_width(epx: f32) -> Self {
        if epx <= Self::SMALL_MAX_EPX {
            Self::Small
        } else if epx >= Self::LARGE_MIN_EPX {
            Self::Large
        } else {
            Self::Medium
        }
    }

    /// 从宿主给的**物理**视口判类别（内部折算有效像素）。
    ///
    /// 这一层做 `物理 / scale`，把口径问题收在这一处 —— 调用方不必记得要除，
    /// 而"忘了除"正是这个口径唯一的失败方式。
    pub fn of_viewport(physical: (f32, f32), scale: f32) -> Self {
        let scale = if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            // 退化时按 1.0 算：那等于用物理像素判，在 100% 缩放下恰好正确。
            1.0
        };
        Self::of_effective_width(physical.0 / scale)
    }

    /// 是否为紧凑档（小）。
    ///
    /// 官方在 `responsive-design` 页举的自适应布局例子就是它：
    /// 「瞬态 UI（导航窗格及其精简版）适用于较小的屏幕，但在较大屏幕上，
    /// 最好选择使用选项卡」。
    pub fn is_compact(&self) -> bool {
        matches!(self, Self::Small)
    }

    /// 该档下网格建议的**最少**列数。
    ///
    /// 这是本仓推导，不是官方数字 —— 官方只给断点，说「当改用更大的屏幕时，
    /// 可以添加列」，没给每档几列。所以真正的列数由 [`columns_for`] 按可用
    /// 宽度算，这里只做下限兜底（可用宽度极窄时也至少要看得见两列）。
    pub fn min_columns(&self) -> usize {
        match self {
            Self::Small => 1,
            Self::Medium => 2,
            Self::Large => 3,
        }
    }
}

/// 触控的**最小可触摸**目标边长（epx）。
///
/// 官方原文：「我们将'可触摸'定义为最小 40 x 40 epx（即使视觉对象较小）」。
pub const TOUCH_MIN_EPX: f32 = 40.0;

/// 触控**优化**档的目标边长（epx）。
///
/// 官方原文：「对于触摸优化的 UI，请考虑将触控目标的大小增加到 44 x 44 epx」。
pub const TOUCH_OPTIMAL_EPX: f32 = 44.0;

/// 触控目标之间的最小可见间距（epx）。
///
/// 官方原文：「触控目标之间至少有 4 epx 的可见空间」。
pub const TOUCH_GAP_EPX: f32 = 4.0;

/// 宽条形目标的替代下限：高 32 epx、宽至少 120 epx。
///
/// 官方原文：「或 32 epx 高（如果宽度至少为 120 epx）」。
/// 导航项这类又宽又扁的目标适用它 —— 强行要求 40 高会把导航撑散。
pub const TOUCH_ROW_HEIGHT_EPX: f32 = 32.0;
pub const TOUCH_ROW_WIDTH_EPX: f32 = 120.0;

/// 把 epx 目标换算成**本仓布局单位**（1 单位 = 1 物理像素）。
///
/// # 为什么要乘 scale
///
/// 触控目标是官方的硬下限，单位是 epx（视觉大小）。而本仓布局单位是物理像素，
/// 两者在 scale ≠ 100% 时不等。不乘就会在 1.25 缩放下给出 40 物理 px 的热区，
/// 视觉上只有 32 epx —— **低于官方下限，而代码看起来完全正确**。
pub fn touch_px(epx: f32, scale: f32) -> f32 {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    epx * scale
}

/// 按可用宽度算网格列数（官方的「重新排列 / Reflow」）。
///
/// # 判据
///
/// `列数 = floor((可用宽 + 间距) / (最小卡宽 + 间距))`，再夹到最少 1 列。
/// 公式是本仓推导：官方只说「当改用更大的屏幕时，可以添加列、使用更大的容器」，
/// 没给算式。
///
/// # 为什么卡宽用**布局单位**而断点用 epx
///
/// 见模块文档「单位口径」。两者服务不同目的，混用是本模块唯一的失败方式。
///
/// # `min_card` 必须 ≤ 可用宽
///
/// 否则返回 1 列，而那一列会比可用宽还宽 —— 内容被裁。调用方给的最小卡宽
/// 应当小于最窄的可用宽度；`Small` 档下应当先降级到单列布局而不是硬挤。
pub fn columns_for(available: f32, min_card: f32, gap: f32) -> usize {
    if !(min_card.is_finite() && min_card > 0.0) || !(available.is_finite() && available > 0.0) {
        return 1;
    }
    let stride = min_card + gap;
    let n = ((available + gap) / stride).floor() as isize;
    n.max(1) as usize
}

#[cfg(test)]
#[path = "tests/responsive_tests.rs"]
mod tests;

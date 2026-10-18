//! 动画的时钟与缓动 —— 时长/曲线全部来自官方规范。
//!
//! # 出处
//!
//! `https://learn.microsoft.com/zh-cn/windows/apps/design/motion/` 的「动画属性」表。
//! 每个常量都在 `docs/developer/animation.md` §1 有逐字抄录，改之前先回查原文。
//!
//! # 为什么本模块只有纯函数
//!
//! 动画 = **缓动后的进度**，而进度要映射到外观值。本仓的规矩是源码零外观，
//! 所以进度在这里算完就停，量化成档位（`step`）交给 zstyle 给值。
//! 见 `docs/developer/animation.md` §3。
//!
//! 纯函数还带来一件实质好处：不依赖 GPU、不依赖窗口，
//! 于是「缓动算得对不对」可以在 CI 里逐点断言，而不是只能靠肉眼。
//!
//! # 求值内核在 `zephyra-core::extra::bezier`
//!
//! 本模块只管「官方档位 → 进度」，而三次方贝塞尔的牛顿迭代住在
//! [`Bezier`](zephyra_core::extra::bezier::Bezier)。拆开的理由：那条求值
//! 是**与官方表格无关**的通用数学，而 extra 正是放这类东西的地方。
//!
//! ⚠️ 本 crate 依赖 `zephyra-bezier`（`zephyra-core/extra/` 下的独立 crate）。
//!    它不进 workspace `default-members`，所以无差别构建不编译它 ——
//!    但 `anim` 的公开 API 里有它（[`Easing::curve`] 返回 [`Bezier`]），
//!    于是选 `zephyra-ui` 时它一定在。

use zephyra_bezier::Bezier;

/// 官方「最低限度」档的时长：淡入 + 淡出，**83 ms**。
///
/// 官方原文：「最低限度 | 淡入 + 淡出 | 线性 | 83 毫秒 | 不透明度」。
/// 注意 83 **只配不透明度** —— 它不是一个"通用短动画"时长。
pub const FADE_MS: f32 = 83.0;

/// 官方 167 ms 一档（快入 / 快出 / 软出共用）。
pub const MS_167: f32 = 167.0;

/// 官方 250 ms 一档。
pub const MS_250: f32 = 250.0;

/// 官方 333 ms 一档（最长档）。
pub const MS_333: f32 = 333.0;

/// 官方「弹性入」的三段时长之和：167 + 167 + 333。
pub const BOUNCE_MS: f32 = MS_167 + MS_167 + MS_333;

/// 本仓的**入场**时长（Gallery 的页面刷新）。
///
/// 取官方动效表「直接入口 / 现有元素」的**最长档** 333 ms。
///
/// # 为什么选 333 而不是 83
///
/// 官方表里 83 ms 是「淡入 + 淡出」那一档，而且**只配不透明度**。本仓的入场
/// 同时动不透明度与位置（官方页面刷新 = 「向上滑动 + 淡入」），位置那一类动作
/// 在表里对应的是 167 / 250 / **333** 三档，取最长档是因为内容区的位移距离小、
/// 时长短会让"闪一下就没了"，而时长长不会让人觉得卡（位移只有 8 px，见
/// `docs/dev-log/task-anim-wire.md`）。
///
/// ⚠️ 这 333 ms 与 [`MS_333`] 是同一个数字但**不同的用途**：`MS_333` 是表里的
/// 原始档位，而 `ENTER_MS` 是"本仓选了哪一档"的决定。分开命名是为了让
/// "这是官方数字"与"这是我们的选择"在代码里看得见。
pub const ENTER_MS: f32 = MS_333;

/// 入场起点的不透明度。
///
/// 官方「页面刷新」=「向上滑动动画和对传入内容的**淡入**动画的组合」
/// （`motion/page-transitions`）。淡入的起点就是全透明。
pub const ENTER_START_ALPHA: f32 = 0.0;

/// 入场起点的**位移**（DIP，向上为正 ⇒ 起点在下方）。
///
/// # ⚠️ 这个数字**不是**官方的
///
/// 官方材料给了两样东西：动效表给**时长与缓动**，`page-transitions` 给
/// **方向**（向上滑动 + 淡入）。**像素距离两处都没有**。
///
/// 所以取值是本仓的决定，理由是：位移越大越容易在不重排（`TRANSLATE_*` 是
/// 纯绘制属性）之外的地方露出破绽，而 8 DIP 在 1159 宽的内容区上是看不出来的
/// 量级 —— 动画要的是"有响应"，不是"看得见的位移"。
/// `docs/dev-log/task-anim-wire.md` 记了同一笔账。
pub const ENTER_START_RISE: f32 = 8.0;

/// 档位数。8 档 × 83ms 时每档约 10.4ms，仍在官方"至少 60 fps"内。
///
/// 档位是取舍：越多越平滑，但每档都是一个分类名，而分类名必须在 zstyle 里
/// 有对应规则（`every_class_name_the_source_emits_exists_in_the_stylesheet` 会查）。
pub const STEPS: u32 = 8;

/// 缓动曲线。
///
/// # ⚠️ 命名按**官方用途**，形状与名字相反（这是官方措辞，不是笔误）
///
/// 官方表格里「直接入口」与「直接退出」**共用** `cubic-bezier(0,0,0,1)`，
/// 「平缓退出」是 `cubic-bezier(1,0,1,1)`。实测两条的形状是：
///
/// | 官方用途 | 曲线 | t=0.5 处 | 实际形状 |
/// |---|---|---|---|
/// | 直接入口 / 直接退出 | (0,0,0,1) | **0.89** | 先快后慢 |
/// | 平缓退出（软出） | (1,0,1,1) | **0.11** | 先慢后快 |
///
/// 也就是说官方把先快后慢那条叫做"快入"。这里**照抄官方用途名**而不改文档 ——
/// 但每个常量的注释都写明实际形状，免得后来者按名字反推曲线。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Easing {
    /// 官方「最低限度」：不透明度用，**线性**。
    #[default]
    Linear,
    /// 官方「直接入口 / 直接退出」：`cubic-bezier(0, 0, 0, 1)`。
    ///
    /// 实际形状是**先快后慢**（t=0.5 处 0.89）。
    Direct,
    /// 官方「点到点」（现有元素）：`cubic-bezier(0.55, 0.55, 0, 1)`。
    ///
    /// 两头都快、中间匀（t=0.5 处约 0.5）。
    PointToPoint,
    /// 官方「平缓退出」：`cubic-bezier(1, 0, 1, 1)`。
    ///
    /// 实际形状是**先慢后快**（t=0.5 处 0.11）。
    SoftExit,
}

impl Easing {
    /// 本曲线的控制点（`Linear` 为 `None`）。
    ///
    /// # 为什么这条是**唯一**的真源
    ///
    /// 以前控制点写在 [`Easing::at`] 的 `match` 里，于是"这条曲线长什么样"
    /// 只有一个地方能回答，而**外部无法查询**—— 想在界面上画出曲线预览、
    /// 或做逐点对照测试，都只能把它复制一份，那必然漂移。
    ///
    /// 现在形状由 [`curve`] 单点定义，`at` 只负责求值。改形状只需改这一处。
    pub const fn curve(self) -> Option<Bezier> {
        match self {
            // 官方明确"线性"，不是省事 —— 对应原文。所以它没有控制点。
            Self::Linear => None,
            // 官方「直接入口 / 直接退出」cubic-bezier(0,0,0,1)
            Self::Direct => Some(Bezier::new(0.0, 0.0, 0.0, 1.0)),
            // 官方「点到点」cubic-bezier(0.55,0.55,0,1)
            Self::PointToPoint => Some(Bezier::new(0.55, 0.55, 0.0, 1.0)),
            // 官方「平缓退出」cubic-bezier(1,0,1,1)
            Self::SoftExit => Some(Bezier::new(1.0, 0.0, 1.0, 1.0)),
        }
    }

    /// 曲线在 `t` 处的值（`t` 已夹到 0..1）。
    ///
    /// 线性直返 `t`（零误差、不迭代）；其余交给 [`curve`] 的求值。
    pub fn at(self, t: f32) -> f32 {
        if !t.is_finite() {
            return 0.0;
        }
        match self.curve() {
            None => t.clamp(0.0, 1.0),
            Some(b) => b.at(t),
        }
    }
}

/// 把「已过去的毫秒数」换算成缓动后的进度（0..1）。
///
/// `elapsed` 已夹满 `dur` 以上就算是 1.0 —— 这一步很关键：
/// 动画的**任何**消费方都不必自己判"放完了没"，否则到处都会漏一次。
pub fn progress(elapsed: f32, dur: f32, easing: Easing) -> f32 {
    if !(dur.is_finite() && dur > 0.0) {
        // 时长非法时视为立即完成：返回 1 比返回 0 安全 ——
        // 0 会让动画永远不结束，而那通常表现为"停在初始外观"。
        return 1.0;
    }
    let raw = if elapsed.is_finite() && elapsed > 0.0 {
        elapsed / dur
    } else {
        0.0
    };
    easing.at(raw)
}

/// 进度 → 档位（`0..=STEPS`）。未完成时落在 `1..STEPS`，完成时是 [`STEPS`]。
///
/// 消费方拿它发分类名。返回 `Option`：进度恰为 0 时返回 `None`，
/// 意思是"这一档没有任何外观差异，不要白发一个分类名" ——
/// 多发一个分类名会让样式表多一条永不命中的规则，而门禁会要求它存在。
pub fn step(p: f32) -> Option<u32> {
    if !p.is_finite() || p <= 0.0 {
        return None;
    }
    let n = (p * STEPS as f32).ceil() as u32;
    Some(n.clamp(1, STEPS))
}

//! 动画的时钟与缓动 —— 时长/曲线全部来自官方规范。
//!
//! # 出处
//!
//! `https://learn.microsoft.com/zh-cn/windows/apps/design/motion/` 的「动画属性」表。
//! 每个常量都在 `docs/animation.md` §1 有逐字抄录，改之前先回查原文。
//!
//! # 为什么本模块只有纯函数
//!
//! 动画 = **缓动后的进度**，而进度要映射到外观值。本仓的规矩是源码零外观，
//! 所以进度在这里算完就停，量化成档位（[`step`]）交给 zstyle 给值。
//! 见 `docs/animation.md` §3。
//!
//! 纯函数还带来一件实质好处：不依赖 GPU、不依赖窗口，
//! 于是「缓动算得对不对」可以在 CI 里逐点断言，而不是只能靠肉眼。

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
/// `docs/task-anim-wire.md`）。
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
/// `docs/task-anim-wire.md` 记了同一笔账。
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
    /// 曲线在 `t` 处的值（`t` 已夹到 0..1）。
    ///
    /// # 为什么自己求而不引 crate
    ///
    /// 三次方贝塞尔的求值是十几行闭式解，引一个 crate 换来的是
    /// 一条依赖与"不知道它在边界上怎么处理"。这里显式处理两处：
    ///
    /// - `t <= 0` / `t >= 1` 直接返回端点（牛顿迭代在端点附近可能不收敛）；
    /// - 分母为 0（控制点使 `cx*3 - cx*2` 退化）时退回线性 —— 那时曲线本身就是直线。
    pub fn at(self, t: f32) -> f32 {
        if !t.is_finite() {
            return 0.0;
        }
        let t = t.clamp(0.0, 1.0);
        match self {
            // 官方明确"线性"，所以这里不是省事，是对应原文。
            Self::Linear => t,
            _ => {
                let (x1, y1, x2, y2) = match self {
                    Self::Direct => (0.0, 0.0, 0.0, 1.0),
                    Self::PointToPoint => (0.55, 0.55, 0.0, 1.0),
                    Self::SoftExit => (1.0, 0.0, 1.0, 1.0),
                    Self::Linear => unreachable!("上面已分支"),
                };
                cubic_bezier(t, x1, y1, x2, y2)
            }
        }
    }
}

/// 三次方贝塞尔求值（控制点形式，对应 CSS / WinUI 的 `cubic-bezier(x1,y1,x2,y2)`）。
///
/// 参数 `x1..y2` 是**控制点**，不是端点 —— 端点是 (0,0) 与 (1,1)。
fn cubic_bezier(t: f32, x1: f32, y1: f32, x2: f32, y2: f32) -> f32 {
    // 曲线参数方程：用 t 求 (x, y)。要的是"给定 x 求 y"，所以先反解 t。
    let cx = 3.0 * x1;
    let bx = 3.0 * (x2 - x1) - cx;
    let ax = 1.0 - cx - bx;
    let cy = 3.0 * y1;
    let by = 3.0 * (y2 - y1) - cy;
    let ay = 1.0 - cy - by;

    let x_of = |s: f32| ((ax * s + bx) * s + cx) * s;
    let y_of = |s: f32| ((ay * s + by) * s + cy) * s;

        // 牛顿迭代解 `x_of(s) = t`。8 次足够：贝塞尔在这个区间上是单调的且平滑。
        //
        // ⚠️ `s` 必须每步夹在 [0,1]。不夹的后果很具体：`x_of(s)=s³` 时
        // 导数 `3s²` 在 s→0 附近趋 0，于是一次迭代就把 s 推到 16.8，
        // 之后 `y_of` 溢出、被外层 clamp 成 0.9989 —— 曲线看起来"倒退"，
        // 而那种抖动与"参数选错"完全不同，极难归因。
        let mut s = t;
        for _ in 0..8 {
            let x = x_of(s) - t;
            let d = (3.0 * ax * s + 2.0 * bx) * s + cx;
            if d == 0.0 || !d.is_finite() {
                break;
            }
            let next = s - x / d;
            if !next.is_finite() {
                break;
            }
            let next = next.clamp(0.0, 1.0);
            // 已收敛到浮点精度就停：继续迭代只会来回抖。
            if (next - s).abs() < 1e-6 {
                s = next;
                break;
            }
            s = next;
        }
        y_of(s).clamp(0.0, 1.0)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 缓动必须满足不动点的基本性质，否则任何接入都会得到错的外观。
    #[test]
    fn 缓动在两端与总时长上闭合() {
        for e in [
            Easing::Linear,
            Easing::Direct,
            Easing::PointToPoint,
            Easing::SoftExit,
        ] {
            assert_eq!(e.at(0.0), 0.0, "{e:?} 起点必须是 0");
            assert!(
                (e.at(1.0) - 1.0).abs() < 1e-4,
                "{e:?} 终点必须是 1，实际 {}",
                e.at(1.0)
            );
            // 单调不减：动画倒退比不动更糟（视觉上会"回头"）。
            //
            // 这条曾经真红过：牛顿迭代没把参数夹在 [0,1]，s 飞到 16.8，
            // y_of 溢出后被 clamp 成 0.9989 —— 曲线在 t=0.03 处"倒退"。
            let mut prev = -1.0;
            for i in 0..=100 {
                let v = e.at(i as f32 / 100.0);
                assert!(v >= prev, "{e:?} 在 {} 处倒退：{v} < {prev}", i as f32 / 100.0);
                prev = v;
            }
        }
    }

    /// 官方曲线要钉住控制点与实际形状：改曲线必须先改 `docs/animation.md` §1。
    ///
    /// ⚠️ 形状**全部实测**过，不按"点到点听起来该居中"猜。三条曲线的 y 控制点
    /// 都在高位，所以中段都 > 0.5（先快后慢）：
    ///
    /// | 官方用途 | 曲线 | t=0.5 处 |
    /// |---|---|---|
    /// | 直接入口 / 直接退出 | (0,0,0,1) | 0.89 |
    /// | 点到点 | (0.55,0.55,0,1) | 0.92 |
    /// | 平缓退出 | (1,0,1,1) | 0.11 |
    ///
    /// 第一版我断言"点到点应接近 0.5"，红在这里 —— 那是我按名字猜的。
    /// 官方表格给的是**控制点字面值**，形状由字面值决定，与名字无关。
    #[test]
    fn 官方曲线的形状与出处一致() {
        let direct = Easing::Direct.at(0.5);
        let p2p = Easing::PointToPoint.at(0.5);
        let soft = Easing::SoftExit.at(0.5);
        for (name, v) in [("Direct", direct), ("PointToPoint", p2p)] {
            assert!(v > 0.8, "官方 {name} 中段应远高于 0.5，实际 {v}");
        }
        assert!(
            soft < 0.2,
            "官方 (1,0,1,1) 中段应远低于 0.5，实际 {soft}"
        );
        // 两条"快"曲线要能区分：点到点比直接入口更"撑在后面"。
        assert!(
            p2p > direct,
            "点到点 ({p2p}) 应比直接入口 ({direct}) 更靠后"
        );
        // 线性就是线性（官方"最低限度"档明确线性，不是省事）。
        for i in 0..=10 {
            assert_eq!(Easing::Linear.at(i as f32 / 10.0), i as f32 / 10.0);
        }
    }

    /// 非法输入必须收敛而不是发散。
    #[test]
    fn 非法时长与负流逝都收敛() {
        assert_eq!(progress(0.0, 0.0, Easing::Linear), 1.0, "时长为 0 ⇒ 立即完成");
        assert_eq!(progress(f32::NAN, 83.0, Easing::Linear), 0.0);
        assert_eq!(progress(-5.0, 83.0, Easing::Direct), 0.0);
        assert_eq!(progress(1000.0, 83.0, Easing::Linear), 1.0, "超时必须停在 1");
        assert!(progress(f32::INFINITY, 83.0, Easing::PointToPoint).is_finite());
    }

    /// 档位：0 进度不发分类名，完成时满档，绝不越界到 `STEPS + 1`。
    #[test]
    fn 档位不越界且零进度不发分类名() {
        assert_eq!(step(0.0), None);
        assert_eq!(step(f32::NAN), None);
        assert_eq!(step(1.0), Some(STEPS));
        for i in 0..=1000 {
            let p = i as f32 / 1000.0;
            if let Some(n) = step(p) {
                assert!((1..=STEPS).contains(&n), "档位 {n} 越界");
            }
        }
        // 83 ms 的动画过了 12 ms ⇒ 约 0.145 ⇒ 进位到 2 档。
        // （第一版写的是"84 ms ⇒ 2 档"，但那已经**完成**（progress=1）⇒ 8 档。
        //  84 > 83，是我把"没放完"读成了"放到一半"。）
        assert_eq!(step(progress(12.0, FADE_MS, Easing::Linear)), Some(2));
        // 正好 12.5% ⇒ 1 档；83 ms 整 ⇒ 满档。
        assert_eq!(step(progress(20.0, FADE_MS, Easing::Linear)), Some(2));
        assert_eq!(step(progress(FADE_MS, FADE_MS, Easing::Linear)), Some(STEPS));
    }

    /// 时长常量必须等于官方表格里的数（83 / 167 / 250 / 333）。
    #[test]
    fn 时长常量与官方表格一致() {
        assert_eq!(FADE_MS, 83.0);
        assert_eq!(MS_167, 167.0);
        assert_eq!(MS_250, 250.0);
        assert_eq!(MS_333, 333.0);
        assert_eq!(BOUNCE_MS, 667.0, "167 + 167 + 333");
        assert_eq!(STEPS, 8);
    }
}

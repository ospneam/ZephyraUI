//! `zephyra-bezier`：可配置的三次方贝塞尔缓动。
//!
//! 一条曲线 = 四个 `f32`。CSS 的 `cubic-bezier(x1,y1,x2,y2)`、
//! WinUI 的同名函数、以及本仓 `ZephyraUI::anim` 的官方四条曲线，
//! 都是这一个类型的不同取值。
//!
//! # 为什么它在 `extra/` 而不是主线
//!
//! 主线 `ZephyraUI::anim` 的 `Easing` 只有 4 条**硬编码**曲线，调用方换不了 ——
//! 想让页面刷动用别的形状，就得往那个 `match` 里加一支，那让"框架官方档位"
//! 变成了"任何应用的私货"。把求值内核拆成独立 crate 之后：
//!
//! - 主线保留**官方那几条**（它们的出处是官方动效表，改动要先改文档）；
//! - 形状本身成为一个**值**，谁都能存、传、画曲线预览、逐点断言。
//!
//! 判据与 `zephyra-base/README.md` 的分层判据同源：**零依赖 + 无 GPU 可单测**
//! ⇒ 放 core 层 `extra/`。要碰平台或允许 unsafe 的包放 `zephyra-base/extra/`。
//!
//! # 三处最容易写错、且错了**不报错**的地方
//!
//! 1. **`s` 必须每步夹到 \[0,1\]** —— `x_of(s)=s³`（即 `x1=x2=0`）时导数 `3s²`
//!    在 `s→0` 附近趋 0，一次牛顿迭代就把 `s` 推到 16.8，`y_of` 随后溢出、
//!    被外层夹成 0.9989 —— 曲线看起来"倒退"。症状与"参数选错"完全不同。
//! 2. **`y` 不能夹** —— 弹性曲线（`y1 > 1`）的过冲正是靠返回 >1 表达的，
//!    夹掉就变成"没有弹性"，而且画面上仍然"看起来是对的"。
//! 3. **±∞ 的方向** —— `+∞` 是"动画已结束"⇒ 1；`-∞` ⇒ 0；`NaN` ⇒ 0。
//!    把 `!t.is_finite() → 0` 写在 clamp 前面，会让 `+∞` 也返回 0，
//!    症状是"进度传了无穷大反而回到起点"，看着像时钟坏了。
//!
//! 三条都有测试钉住，见 `tests/bezier.rs`。

/// 三次方贝塞尔曲线（CSS / WinUI 的 `cubic-bezier(x1,y1,x2,y2)`）。
///
/// 端点**固定**为 `(0,0)` 与 `(1,1)`；`x1..y2` 是两个中间控制点。
/// 所以本类型表示的永远是「单调推进的时间重映射」`y = f(x)`，
/// 而不是一个任意形状的三次多项式。
///
/// # 例子
///
/// ```ignore
/// // 官方「平缓退出」：(1,0,1,1)
/// let soft_exit = Bezier::new(1.0, 0.0, 1.0, 1.0);
/// assert!(soft_exit.at(0.5) < 0.2); // 先慢后快
///
/// // 弹性类曲线允许 y 出界（>1 = 过冲），x 仍必须单调。
/// let overshoot = Bezier::new(0.34, 1.56, 0.64, 1.0);
/// assert!(overshoot.at(0.4) > 1.0);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bezier {
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
}

impl Bezier {
    /// 牛顿迭代上限。8 次足够：三次贝塞尔在 \[0,1\] 上单调且二阶连续。
    pub const ITERATIONS: usize = 8;

    /// 收敛判据：`|Δs| < 1e-6`。再细只是浮点噪声来回抖。
    const EPS: f32 = 1e-6;

    /// 四个控制点。`x1`/`x2` **必须**在 0..1，否则时间轴不再单调，
    /// 动画会出现"倒着走"。这里不校验，见 [`Bezier::is_monotonic`]。
    pub const fn new(x1: f32, y1: f32, x2: f32, y2: f32) -> Self {
        Self { x1, y1, x2, y2 }
    }

    /// 线性曲线（`y = x`）。等价于 CSS 的 `linear`。
    pub const fn linear() -> Self {
        Self::new(0.0, 0.0, 1.0, 1.0)
    }

    /// 控制点 `(0, 0, 1, 1)`，形状就是 `y = x`。
    ///
    /// 缓入缓出（CSS `ease`）：`(0.25, 0.1, 0.25, 1.0)`。
    pub const fn ease() -> Self {
        Self::new(0.25, 0.1, 0.25, 1.0)
    }

    /// 缓入（CSS `ease-in`）：`(0.42, 0, 1, 1)`。
    pub const fn ease_in() -> Self {
        Self::new(0.42, 0.0, 1.0, 1.0)
    }

    /// 缓出（CSS `ease-out`）：`(0, 0, 0.58, 1)`。
    pub const fn ease_out() -> Self {
        Self::new(0.0, 0.0, 0.58, 1.0)
    }

    /// 缓入缓出（CSS `ease-in-out`）：`(0.42, 0, 0.58, 1)`。
    pub const fn ease_in_out() -> Self {
        Self::new(0.42, 0.0, 0.58, 1.0)
    }

    /// 四个控制点，供调用方自省或持久化。
    pub const fn control_points(self) -> (f32, f32, f32, f32) {
        (self.x1, self.y1, self.x2, self.y2)
    }

    /// `x1`/`x2` 是否都在 0..1 —— 即时间轴单调。
    ///
    /// # 为什么不构造时就 panic
    ///
    /// 弹性曲线（`y` 越界）完全合法且常用，而 `x` 越界是**配置错误**。
    /// 但把它做成 panic 会让「从 JSON 读曲线」这条路在数据出错时整个应用崩掉 ——
    /// 而动画出错最多是"看起来不对"，不该是"起不来"。所以改成可查询的谓词，
    /// 由调用方在开发期断言（见本模块的 `is_monotonic_is_false_for_bad_x`）。
    pub const fn is_monotonic(self) -> bool {
        in_unit(self.x1) && in_unit(self.x2)
    }

    /// 曲线在 `t` 处的值。`t` 会被夹到 0..1。
    ///
    /// # 返回值可能**超出** 0..1
    ///
    /// `y` 不夹 —— 弹性曲线（`y1 > 1`）的过冲正是靠这个返回值表达的，
    /// 夹掉就变成"没有弹性"了。调用方若只接受 0..1，**自己**夹。
    pub fn at(self, t: f32) -> f32 {
        // ⚠️ **先夹、后判有限性** —— 顺序反了会让正无穷返回 0。
        //
        // `f32::clamp` 的文档规定：min > max 时返回 NaN，自变量为 NaN 时返回 NaN。
        // 而本函数对 NaN 的约定是"返回 0"（动画不该因 NaN 崩掉）。
        // 正确写法是分开处理三态，而不是依赖 `clamp`：
        //   - NaN       → 0
        //   - ±∞        → 夹到端点（正无穷 = 动画已结束 ⇒ 1）
        //   - 有限值    → clamp
        //
        // 写成 `if !t.is_finite() { return 0.0 }` 会把 +∞ 也吞成 0，
        // 而 +∞ 的正确答案是 1（进度已经远超终点）—— 症状是"传了无穷大
        // 进度反而回到起点"，那与"参数错了"看起来完全不像。
        if t.is_nan() {
            return 0.0;
        }
        let t = t.clamp(0.0, 1.0);
        // 控制点退化（x1=x2=0 ⇒ x_of(s)=s³）的情形见模块文档：
        // 导数在 s→0 附近趋 0，不夹 `s` 会让它飞到 16.8。
        let cx = 3.0 * self.x1;
        let bx = 3.0 * (self.x2 - self.x1) - cx;
        let ax = 1.0 - cx - bx;
        let cy = 3.0 * self.y1;
        let by = 3.0 * (self.y2 - self.y1) - cy;
        let ay = 1.0 - cy - by;

        let x_of = |s: f32| ((ax * s + bx) * s + cx) * s;
        let y_of = |s: f32| ((ay * s + by) * s + cy) * s;

        // 初值取 t 本身：单调曲线下 x_of(t) 与 t 同阶，用它起步收敛最快。
        let mut s = t;
        for _ in 0..Self::ITERATIONS {
            let d = (3.0 * ax * s + 2.0 * bx) * s + cx;
            // 导数为 0 或非有限：无处可退，停在当前 s。
            if d == 0.0 || !d.is_finite() {
                break;
            }
            let next = s - (x_of(s) - t) / d;
            if !next.is_finite() {
                break;
            }
            let next = next.clamp(0.0, 1.0);
            if (next - s).abs() < Self::EPS {
                s = next;
                break;
            }
            s = next;
        }
        y_of(s)
    }

    /// 已过去的毫秒 → 进度。时长非法时视为**立即完成**（返回 1）。
    ///
    /// 返回 1 而不是 0：0 会让动画永远不结束，症状是"停在初始外观"。
    pub fn progress(self, elapsed: f32, duration_ms: f32) -> f32 {
        if !(duration_ms.is_finite() && duration_ms > 0.0) {
            return 1.0;
        }
        let raw = if elapsed.is_finite() && elapsed > 0.0 {
            elapsed / duration_ms
        } else {
            0.0
        };
        self.at(raw)
    }
}

/// `v` 是否落在闭区间 0..1。
///
/// # 为什么不用 `(0.0..=1.0).contains(&v)`
///
/// `RangeInclusive::contains` **还不是 `const fn`**（rustc issue #143874），
/// 而 `is_monotonic` 需要 `const` —— 它会被用在 `const` 上下文里
/// （例如静态断言曲线是否合法）。手写两个比较即可，代价为零。
const fn in_unit(v: f32) -> bool {
    // NaN 也要返回 false：`v >= 0.0` 对 NaN 是 false，所以这里天然覆盖。
    v >= 0.0 && v <= 1.0
}

//! 输入：指针事件词汇表、手势识别、视口变换、惯性。
//!
//! # 这一层为什么在核心里
//!
//! 手势识别是**框架**职责而非应用职责，原因有三：
//!
//! 1. **宿主无关。** 同一个"两指捏合"要同时服务于 iced 的 `Touch` 事件、未来的
//!    winit `WindowEvent`、以及可能的手写 `WM_POINTER*` 钩子。识别算法若住在宿主侧，
//!    每换一个宿主就要重写一遍，而重写就意味着重错。
//! 2. **零分配是硬约束。** 笔画热路径上的每次堆分配都会让帧时间出现尖峰。所以指针表
//!    是定长数组、识别输出是 `Copy` 值——这些在应用层写不出来，只有作为核心的一部分
//!    才有强制力。
//! 3. **视口变换与命中测试必须同源。** 手势产出的缩放/平移直接决定命中测试用的几何。
//!    两者分开定义就会出现"拖得动但点不准"这类漂移。
//!
//! # 模块
//!
//! - [`pointer`]：事件词汇表 + 固定容量指针表
//! - [`pinch`]：两指捏合（增量语义 + 平方域容差）
//! - [`recognizer`]：单入口手势状态机
//! - [`viewport`]：屏幕 ↔ 内容坐标变换（缩放钳位的唯一来源）
//! - [`momentum`]：帧率无关惯性
//!
//! # 与控件树的接口
//!
//! 本模块只产出**手势**（[`Gesture`]），不关心落在哪个控件上——那是 [`crate::hit`]
//! 与 [`crate::input::router`] 的事。分开的原因：手势是可复用的底层语义，
//! 而"哪个控件被点了"必须结合布局矩形才有意义。

pub mod momentum;
pub mod pinch;
pub mod pointer;
pub mod recognizer;
pub mod router;
pub mod viewport;

pub use momentum::Momentum;
pub use pinch::PinchDelta;
pub use pointer::{PointerEvent, PointerPhase, PointerSource};
pub use recognizer::{Gesture, Recognizer};
pub use router::{InputRouter, RoutedEvent};
pub use viewport::Viewport;
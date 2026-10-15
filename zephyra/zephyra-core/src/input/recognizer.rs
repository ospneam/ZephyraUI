//! 统一手势状态机：指针表 + 两指捏合，单入口 [`Recognizer::on_pointer`]。
//!
//! 每个 OS 指针事件恰好一次 O(MAX_POINTERS) 调用，产出 [`Gesture`]（`Copy`，
//! 零分配）。消费方无需再自行维护指针表或串联多个识别器。

use super::pinch::{PinchDelta, PinchState};
use super::pointer::{PointerEvent, PointerPhase, PointerTable};

/// 单个指针事件的识别输出（`Copy`）。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Gesture {
    /// 捏合增量：Down 激活时为 1.0 锚点增量（消费方仅取 center）；
    /// Move 为增量比；Up 恒为 `None`。
    pub pinch: Option<PinchDelta>,
    /// 本次事件是否刚激活捏合（仅 Down 可能）。
    pub pinch_started: bool,
    /// 事件处理后的捏合状态。
    pub is_pinching: bool,
    /// 事件处理后的活跃指针数。
    pub pointers: u8,
}

/// 手势识别器：固定指针表 + 两指捏合（V2 增量语义）。
#[derive(Clone)]
pub struct Recognizer {
    table: PointerTable,
    pinch: PinchState,
}

impl Default for Recognizer {
    fn default() -> Self {
        Self::new()
    }
}

impl Recognizer {
    pub fn new() -> Self {
        Self {
            table: PointerTable::new(),
            pinch: PinchState::new(),
        }
    }

    /// 当前是否处于捏合中。
    #[inline]
    pub fn is_pinching(&self) -> bool {
        self.pinch.is_pinching()
    }

    /// 当前活跃指针数。
    #[inline]
    pub fn pointer_count(&self) -> usize {
        self.table.len()
    }

    /// 单一入口：喂入一个指针事件，返回识别结果。
    ///
    /// - Down：登记指针；活跃 ≥2 时激活捏合并产出 1.0 锚点增量；
    /// - Move：仅更新已登记指针（未知 ID 忽略，与旧语义一致）；
    /// - Up：注销指针；捏合指抬起时结束，剩余 ≥2 指则无缝 regrab。
    #[inline]
    pub fn on_pointer(&mut self, ev: PointerEvent) -> Gesture {
        match ev.phase {
            PointerPhase::Down => {
                self.table.down(ev.id, ev.x, ev.y);
                let pinch = self.pinch.on_down(&self.table);
                Gesture {
                    pinch,
                    pinch_started: pinch.is_some(),
                    is_pinching: self.pinch.is_pinching(),
                    pointers: self.table.len() as u8,
                }
            }
            PointerPhase::Move => {
                let tracked = self.table.move_pointer(ev.id, ev.x, ev.y);
                let pinch = if tracked {
                    self.pinch.on_move(&self.table, ev.id)
                } else {
                    None
                };
                Gesture {
                    pinch,
                    pinch_started: false,
                    is_pinching: self.pinch.is_pinching(),
                    pointers: self.table.len() as u8,
                }
            }
            PointerPhase::Up => {
                let had = self.table.up(ev.id);
                if had {
                    self.pinch.on_up(&self.table, ev.id);
                }
                Gesture {
                    pinch: None,
                    pinch_started: false,
                    is_pinching: self.pinch.is_pinching(),
                    pointers: self.table.len() as u8,
                }
            }
        }
    }

    /// 全量复位（窗口失焦等异常恢复路径用）。
    pub fn reset(&mut self) {
        self.table = PointerTable::new();
        self.pinch = PinchState::new();
    }
}

#[cfg(test)]
#[path = "tests/recognizer_tests.rs"]
mod tests;

// 本文件由 recognizer.rs 的 #[cfg(test)] mod tests 抽出 —— 测试代码与生产代码分离。
// 模块路径不变（src 里的声明用 #[path] 指过来），所以 use super::* 与私有可见性都照旧。

use super::*;
use crate::input::pointer::PointerSource;

fn ev(id: u32, x: f32, y: f32, phase: PointerPhase) -> PointerEvent {
    PointerEvent {
        id,
        x,
        y,
        phase,
        source: PointerSource::Touch,
    }
}

#[test]
fn single_finger_never_pinches() {
    let mut r = Recognizer::new();
    let g = r.on_pointer(ev(1, 100.0, 100.0, PointerPhase::Down));
    assert_eq!(g.pointers, 1);
    assert!(!g.pinch_started);
    assert!(g.pinch.is_none());
    let g = r.on_pointer(ev(1, 200.0, 100.0, PointerPhase::Move));
    assert!(g.pinch.is_none());
    assert!(!g.is_pinching);
}

#[test]
fn pinch_starts_on_second_down_with_anchor() {
    let mut r = Recognizer::new();
    r.on_pointer(ev(1, 100.0, 100.0, PointerPhase::Down));
    let g = r.on_pointer(ev(2, 200.0, 100.0, PointerPhase::Down));
    assert!(g.pinch_started);
    assert!(g.is_pinching);
    assert_eq!(g.pointers, 2);
    let d = g.pinch.expect("激活时应携带 1.0 锚点增量");
    assert_eq!(d.scale, 1.0);
    assert_eq!(d.center_x, 150.0);
    assert_eq!(d.center_y, 100.0);
}

#[test]
fn within_tolerance_no_delta() {
    let mut r = Recognizer::new();
    r.on_pointer(ev(1, 100.0, 100.0, PointerPhase::Down));
    r.on_pointer(ev(2, 200.0, 100.0, PointerPhase::Down));
    // 双指对称移动 2px（< 8px 容差，指距 100→96，中点不动）→ 不产出增量
    let g = r.on_pointer(ev(1, 102.0, 100.0, PointerPhase::Move));
    assert!(g.pinch.is_none(), "容差内不得产出增量");
    let g = r.on_pointer(ev(2, 198.0, 100.0, PointerPhase::Move));
    assert!(g.pinch.is_none(), "容差内不得产出增量");
    // 批次门控通过但仍在容差内
    assert!(g.is_pinching);
}

#[test]
fn incremental_ratio_with_batch_gate() {
    let mut r = Recognizer::new();
    r.on_pointer(ev(1, 100.0, 100.0, PointerPhase::Down));
    r.on_pointer(ev(2, 200.0, 100.0, PointerPhase::Down));
    // 指 1 大幅移动（超出容差）但指 2 未动 → 批次门控拦截
    let g = r.on_pointer(ev(1, 80.0, 100.0, PointerPhase::Move));
    assert!(g.pinch.is_none(), "单指先动不得产出增量");
    // 指 2 再动 → 两指均已动 → 产出增量（参考距离为激活帧 120）
    let g = r.on_pointer(ev(2, 205.0, 100.0, PointerPhase::Move));
    let d = g.pinch.expect("两指均动后应产出增量");
    assert!(
        (d.scale - 125.0 / 120.0).abs() < 1e-5,
        "实际 scale={}",
        d.scale
    );
    assert!((d.center_x - 142.5).abs() < 1e-5);
}

#[test]
fn up_ends_pinch_and_regrabs_with_remaining() {
    let mut r = Recognizer::new();
    r.on_pointer(ev(1, 100.0, 100.0, PointerPhase::Down));
    r.on_pointer(ev(2, 200.0, 100.0, PointerPhase::Down));
    r.on_pointer(ev(3, 300.0, 300.0, PointerPhase::Down)); // 已捏合中，忽略
    assert!(r.is_pinching());
    // 捏合指 1 抬起 → 结束并用剩余 {2,3} 无缝 regrab
    let g = r.on_pointer(ev(1, 100.0, 100.0, PointerPhase::Up));
    assert_eq!(g.pointers, 2);
    assert!(g.is_pinching, "剩余 ≥2 指应 regrab");
    assert!(g.pinch.is_none(), "Up 事件不产出增量");
    // 再抬指 2 → 仅剩 1 指 → 捏合结束
    let g = r.on_pointer(ev(2, 200.0, 100.0, PointerPhase::Up));
    assert_eq!(g.pointers, 1);
    assert!(!g.is_pinching);
}

#[test]
fn unknown_pointer_up_is_ignored() {
    let mut r = Recognizer::new();
    r.on_pointer(ev(1, 100.0, 100.0, PointerPhase::Down));
    r.on_pointer(ev(2, 200.0, 100.0, PointerPhase::Down));
    let g = r.on_pointer(ev(99, 0.0, 0.0, PointerPhase::Up));
    assert_eq!(g.pointers, 2);
    assert!(g.is_pinching, "未知 ID 的 Up 不得影响捏合");
}

#[test]
fn pinch_fends_when_finger_disappears() {
    let mut r = Recognizer::new();
    r.on_pointer(ev(1, 100.0, 100.0, PointerPhase::Down));
    r.on_pointer(ev(2, 200.0, 100.0, PointerPhase::Down));
    // 指针表只剩 1 指时（系统吞掉 Up 的兜底路径）
    let mut r2 = r.clone();
    r2.table = crate::input::pointer::PointerTable::new();
    r2.table.down(1, 100.0, 100.0);
    let g = r2.on_pointer(ev(1, 100.0, 100.0, PointerPhase::Move));
    assert!(!g.is_pinching, "活跃指针不足应结束捏合");
}

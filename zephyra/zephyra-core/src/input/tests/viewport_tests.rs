// 本文件由 viewport.rs 的 #[cfg(test)] mod tests 抽出 —— 测试代码与生产代码分离。
// 模块路径不变（src 里的声明用 #[path] 指过来），所以 use super::* 与私有可见性都照旧。

use super::*;

#[test]
fn screen_canvas_roundtrip() {
    let t = Viewport::new(30.0, -20.0, 2.5);
    let (cx, cy) = t.screen_to_canvas(105.0, 55.0);
    assert!((cx - 30.0).abs() < 1e-4);
    assert!((cy - 30.0).abs() < 1e-4);
    let (sx, sy) = t.canvas_to_screen(cx, cy);
    assert!((sx - 105.0).abs() < 1e-3);
    assert!((sy - 55.0).abs() < 1e-3);
}

#[test]
fn apply_pinch_keeps_anchor_under_center() {
    let mut t = Viewport::new(0.0, 0.0, 1.0);
    let center = (200.0, 150.0);
    let (ax, ay) = t.pinch_anchor(center.0, center.1);
    t.apply_pinch(center.0, center.1, 2.0, ax, ay);
    // 锚点画布点缩放后仍应位于同一屏幕点
    let (sx, sy) = t.canvas_to_screen(ax, ay);
    assert!((sx - center.0).abs() < 1e-3);
    assert!((sy - center.1).abs() < 1e-3);
    assert!((t.scale - 2.0).abs() < 1e-6);
}

#[test]
fn apply_pinch_clamps_scale() {
    let mut t = Viewport::new(0.0, 0.0, 9.5);
    t.apply_pinch(100.0, 100.0, 2.0, 0.0, 0.0);
    assert_eq!(t.scale, Viewport::MAX_SCALE);
    t.apply_pinch(100.0, 100.0, 0.01, 0.0, 0.0);
    assert_eq!(t.scale, Viewport::MIN_SCALE);
}

#[test]
fn pinch_anchor_guards_zero_scale() {
    let t = Viewport::new(10.0, 10.0, 0.0);
    let (ax, ay) = t.pinch_anchor(20.0, 20.0);
    assert!(
        ax.is_finite() && ay.is_finite(),
        "零 scale 不得产生 inf/nan"
    );
}

// 本文件由 momentum.rs 的 #[cfg(test)] mod tests 抽出 —— 测试代码与生产代码分离。
// 模块路径不变（src 里的声明用 #[path] 指过来），所以 use super::* 与私有可见性都照旧。

use super::*;

#[test]
fn inactive_tick_is_none() {
    let mut m = Momentum::new();
    assert!(m.tick(1.0 / 60.0).is_none());
    assert!(!m.is_active());
}

#[test]
fn decays_and_settles() {
    let mut m = Momentum::new();
    m.start(10.0, 0.0);
    let mut total = 0.0f32;
    let mut frames = 0;
    for _ in 0..2000 {
        match m.tick(1.0 / 60.0) {
            Some((dx, _)) => {
                total += dx;
                frames += 1;
            }
            None => break,
        }
    }
    assert!(!m.is_active(), "惯性应收敛停止");
    assert!(frames > 5, "应有实际滚动帧");
    assert!(total > 10.0, "总位移应超过初速（摩擦渐进），实际 {total}");
    assert!(total < 300.0, "总位移应有界，实际 {total}");
}

#[test]
fn frame_rate_independent_total_distance() {
    // 同一初速度：60Hz 与 120Hz 的累计位移应接近（离散积分误差 < 10%）
    let run = |steps: usize, dt: f32| {
        let mut m = Momentum::new();
        m.start(20.0, 0.0);
        let mut total = 0.0f32;
        for _ in 0..steps {
            match m.tick(dt) {
                Some((dx, _)) => total += dx,
                None => break,
            }
        }
        total
    };
    let t60 = run(600, 1.0 / 60.0);
    let t120 = run(1200, 1.0 / 120.0);
    let rel = (t60 - t120).abs() / t60;
    assert!(rel < 0.10, "帧率无关性破坏：60Hz {t60} vs 120Hz {t120}");
}

#[test]
fn stop_is_immediate() {
    let mut m = Momentum::new();
    m.start(50.0, 50.0);
    assert!(m.tick(1.0 / 60.0).is_some());
    m.stop();
    assert!(!m.is_active());
    assert!(m.tick(1.0 / 60.0).is_none());
}

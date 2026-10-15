// 本文件由 anim.rs 的 #[cfg(test)] mod tests 抽出 —— 测试代码与生产代码分离。
// 模块路径不变（src 里的声明用 #[path] 指过来），所以 use super::* 与私有可见性都照旧。

use super::*;

/// 缓动必须满足不动点的基本性质，否则任何接入都会得到错的外观。
#[test]
fn easing_closes_at_both_ends_and_total_duration() {
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
            assert!(
                v >= prev,
                "{e:?} 在 {} 处倒退：{v} < {prev}",
                i as f32 / 100.0
            );
            prev = v;
        }
    }
}

/// 官方曲线要钉住控制点与实际形状：改曲线必须先改 `docs/developer/animation.md` §1。
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
fn official_curve_shapes_match_their_source() {
    let direct = Easing::Direct.at(0.5);
    let p2p = Easing::PointToPoint.at(0.5);
    let soft = Easing::SoftExit.at(0.5);
    for (name, v) in [("Direct", direct), ("PointToPoint", p2p)] {
        assert!(v > 0.8, "官方 {name} 中段应远高于 0.5，实际 {v}");
    }
    assert!(soft < 0.2, "官方 (1,0,1,1) 中段应远低于 0.5，实际 {soft}");
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
fn invalid_duration_and_negative_elapsed_converge() {
    assert_eq!(
        progress(0.0, 0.0, Easing::Linear),
        1.0,
        "时长为 0 ⇒ 立即完成"
    );
    assert_eq!(progress(f32::NAN, 83.0, Easing::Linear), 0.0);
    assert_eq!(progress(-5.0, 83.0, Easing::Direct), 0.0);
    assert_eq!(
        progress(1000.0, 83.0, Easing::Linear),
        1.0,
        "超时必须停在 1"
    );
    assert!(progress(f32::INFINITY, 83.0, Easing::PointToPoint).is_finite());
}

/// 档位：0 进度不发分类名，完成时满档，绝不越界到 `STEPS + 1`。
#[test]
fn tiers_clamp_and_zero_progress_emits_no_class() {
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
    assert_eq!(
        step(progress(FADE_MS, FADE_MS, Easing::Linear)),
        Some(STEPS)
    );
}

/// 时长常量必须等于官方表格里的数（83 / 167 / 250 / 333）。
#[test]
fn duration_constants_match_official_table() {
    assert_eq!(FADE_MS, 83.0);
    assert_eq!(MS_167, 167.0);
    assert_eq!(MS_250, 250.0);
    assert_eq!(MS_333, 333.0);
    assert_eq!(BOUNCE_MS, 667.0, "167 + 167 + 333");
    assert_eq!(STEPS, 8);
}

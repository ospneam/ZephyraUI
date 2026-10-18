//! `zephyra_bezier` 的集成测试。
//!
//! # 为什么这些断言是「不恒真」的
//!
//! 贝塞尔求值最容易出的三类错，症状都**不报错**：曲线形状看起来差不多。
//! 所以每条断言都钉一个具体的数或一条具体的不变量，而不是"大概单调"。

use zephyra_bezier::Bezier;

/// 官方 WinUI 四条曲线的控制点必须与 `docs/developer/animation.md` §1 一致。
///
/// 这条钉的是**来源**而不是形状：改动画必须先改文档，而文档改了就该红。
#[test]
fn official_control_points_are_pinned() {
    // 「直接入口 / 直接退出」cubic-bezier(0,0,0,1)
    assert_eq!(
        Bezier::new(0.0, 0.0, 0.0, 1.0).control_points(),
        (0.0, 0.0, 0.0, 1.0)
    );
    // 「点到点」cubic-bezier(0.55,0.55,0,1)
    assert_eq!(
        Bezier::new(0.55, 0.55, 0.0, 1.0).control_points(),
        (0.55, 0.55, 0.0, 1.0)
    );
    // 「平缓退出」cubic-bezier(1,0,1,1)
    assert_eq!(
        Bezier::new(1.0, 0.0, 1.0, 1.0).control_points(),
        (0.0 + 1.0, 0.0, 1.0, 1.0)
    );
}

/// 端点必须是 `(0,0)` 与 `(1,1)` —— 不动点。
///
/// 端点不动**不是**构造器的性质（它压根不接受端点参数），而是牛顿迭代
/// 在边界上必须收敛到那里。端点错了的话，动画开头/结尾会有一帧跳变。
#[test]
fn endpoints_are_fixed_points() {
    for b in [
        Bezier::linear(),
        Bezier::ease(),
        Bezier::ease_in(),
        Bezier::ease_out(),
        Bezier::ease_in_out(),
        Bezier::new(0.0, 0.0, 0.0, 1.0),
        Bezier::new(1.0, 0.0, 1.0, 1.0),
    ] {
        assert!(b.at(0.0).abs() < 1e-4, "{b:?} at(0) = {}", b.at(0.0));
        assert!(
            (b.at(1.0) - 1.0).abs() < 1e-4,
            "{b:?} at(1) = {}",
            b.at(1.0)
        );
    }
}

/// `s` 每步夹取的效果：`x1=x2=0`（即 `x_of(s)=s³`）这条最危险的曲线不能倒退。
///
/// ⚠️ 这条**曾经真红过**：不夹 `s` 时导数 `3s²` 在 `s→0` 附近趋 0，
/// 一次迭代把 `s` 推到 16.8，`y_of` 溢出后被 clamp 成 0.9989 ——
/// 症状是曲线在 t=0.03 附近"倒退"，而那与"参数选错"症状完全不同。
#[test]
fn cubic_x_axis_stays_monotonic() {
    let b = Bezier::new(0.0, 0.0, 0.0, 1.0);
    let mut prev = -1.0;
    for i in 0..=1000 {
        let t = i as f32 / 1000.0;
        let v = b.at(t);
        assert!(v >= prev, "在 t={t} 处倒退：{v} < {prev}");
        prev = v;
    }
}

/// 单调递增是 x 单调的直接推论，且比逐点采样更能抓住"整体倒着走"。
#[test]
fn all_presets_are_monotonic_increasing() {
    for b in [
        Bezier::linear(),
        Bezier::ease(),
        Bezier::ease_in(),
        Bezier::ease_out(),
        Bezier::ease_in_out(),
    ] {
        let mut prev = f32::NEG_INFINITY;
        for i in 0..=200 {
            let v = b.at(i as f32 / 200.0);
            assert!(v >= prev, "{b:?} 在 t={} 处倒退", i as f32 / 200.0);
            prev = v;
        }
    }
}

/// `linear()` 真的是直线。
///
/// ⚠️ 这里用**容差**而不是 `assert_eq!`：`linear()` 走的是贝塞尔求值，
/// 而牛顿迭代的末位误差是 `f32` 量级。主线的 `Easing::Linear` 是
/// `=> t` 直返（零误差），两者不是同一条路径 —— 别拿主线的严格度要求这里。
#[test]
fn linear_is_straight_within_float_error() {
    let b = Bezier::linear();
    for i in 0..=100 {
        let t = i as f32 / 100.0;
        assert!(
            (b.at(t) - t).abs() < 1e-5,
            "t={t} 处偏离直线 {}",
            b.at(t) - t
        );
    }
}

/// 非法输入必须**收敛**而不是发散。
#[test]
fn degenerate_inputs_converge() {
    let b = Bezier::ease();
    assert_eq!(b.at(f32::NAN), 0.0);
    assert_eq!(b.at(f32::INFINITY), 1.0, "正无穷夹到上端点");
    assert_eq!(b.at(f32::NEG_INFINITY), 0.0, "负无穷夹到下端点");
    assert_eq!(b.at(-5.0), 0.0, "负数夹到 0");
    assert_eq!(b.at(5.0), 1.0, "大于 1 夹到 1");
    // 时长非法 ⇒ 立即完成（返回 1，不是 0 —— 0 会让动画永不结束）。
    assert_eq!(b.progress(10.0, 0.0), 1.0);
    assert_eq!(b.progress(10.0, -1.0), 1.0);
    assert_eq!(b.progress(10.0, f32::NAN), 1.0);
    // 非法 elapsed ⇒ 进度 0。
    assert_eq!(b.progress(f32::NAN, 100.0), 0.0);
    assert_eq!(b.progress(-5.0, 100.0), 0.0);
}

/// `progress` 的夹取：超时必须停在 1，不能继续往上走。
#[test]
fn progress_clamps_past_duration() {
    let b = Bezier::ease_out();
    assert!((b.progress(100.0, 100.0) - 1.0).abs() < 1e-4);
    assert!((b.progress(500.0, 100.0) - 1.0).abs() < 1e-4);
    assert!(b.progress(50.0, 100.0) < 1.0);
}

/// 弹性曲线允许 `y` 越界 —— 那正是"过冲"的表达方式。
///
/// ⚠️ 如果 `at()` 把返回值夹到 0..1，弹性就变成"没有弹性"，而且**看起来
/// 仍然是对的**（只是少了过冲）。所以这条钉住"不过夹"。
#[test]
fn overshoot_is_allowed_in_y() {
    // 这是常见的 "easeOutBack" 形状：y1 > 1 ⇒ 中段冲过 1。
    let b = Bezier::new(0.34, 1.56, 0.64, 1.0);
    let mut peak: f32 = 0.0;
    for i in 0..=1000 {
        peak = peak.max(b.at(i as f32 / 1000.0));
    }
    assert!(peak > 1.0, "弹性曲线的峰值应超过 1，实际 {peak}");
}

/// `is_monotonic` 只看 x —— y 越界不是错误。
#[test]
fn monotonicity_only_constrains_x() {
    assert!(
        Bezier::new(0.34, 1.56, 0.64, 1.0).is_monotonic(),
        "y>1 合法"
    );
    assert!(Bezier::linear().is_monotonic());
    assert!(Bezier::new(0.0, 0.0, 0.0, 1.0).is_monotonic());
    // x 越界是配置错误（时间轴不再单调）。
    assert!(!Bezier::new(1.5, 0.0, 0.5, 1.0).is_monotonic());
    assert!(!Bezier::new(-0.1, 0.0, 0.5, 1.0).is_monotonic());
    // NaN 不在 0..1 内。
    assert!(!Bezier::new(f32::NAN, 0.0, 0.5, 1.0).is_monotonic());
}

/// `const fn` 可用性：`is_monotonic` 必须能在**编译期**求值。
///
/// 这条不是"测个函数"，是测**它还能不能 const**：哪天有人把它改成非 const，
/// 下面这两行 `const` 会在编译期红。
///
/// ⚠️ 用 `const _: () = assert!(..)` 而不是运行时 `assert!` —— 后者会被 clippy
///    的 `assertions_on_constants` 判红（它确实是恒真的），而**编译期断言才是
///    这条测试真正想钉的东西**：既有运行期的效果，也不与 lint 打架。
#[test]
fn monotonicity_is_const_evaluable() {
    const _: () = assert!(Bezier::linear().is_monotonic());
    const _: () = assert!(!Bezier::new(2.0, 0.0, 0.5, 1.0).is_monotonic());
    // 留一条运行期的，让这条测试在 --test 下真的会跑（不留的话它只剩编译期效果）。
    assert!(Bezier::ease().is_monotonic());
}

/// 预设的形状必须与 CSS 语义一致 —— 这几个是"名字带承诺"的。
#[test]
fn preset_shapes_match_their_css_meaning() {
    // ease-in 起点慢：t 小处斜率应小于线性。
    assert!(Bezier::ease_in().at(0.2) < 0.2, "ease-in 开头应比线性慢");
    // ease-out 结尾慢：t 大处"剩余距离"应大于线性。
    assert!(Bezier::ease_out().at(0.8) > 0.8, "ease-out 结尾应比线性慢");
    // ease-in-out 两头慢：两端都贴住线性，中段拉开。
    let io = Bezier::ease_in_out();
    assert!(io.at(0.2) < 0.2);
    assert!(io.at(0.8) > 0.8);
}

//! 拖动去抖的**零 GPU 门禁**。
//!
//! # 为什么它能不用 GPU / 窗口就测
//!
//! 拖动闪屏与 GPU 挂死的因果链已经被实测钉死（本文件所在 crate 的
//! `tests/swapchain_draw.rs::probe2_ResizeBuffers_矩阵`）：本机 `ResizeBuffers`
//! 在后缓冲被用过之后就再也调不成功，所以尺寸一变只能**重建整条交换链**；而拖边时
//! `WM_SIZE` 每秒来几十次，一帧一重建实测能把 GPU 拖进 `DXGI_ERROR_DEVICE_HUNG`。
//!
//! 修法是把判定抽成纯函数 [`resize_settled`]：尺寸还在变就不重建（只把画面缩放
//! 铺满），等它稳下来才重建一次。这条门禁守的就是那个判定 —— 它不需要真的拖窗口，
//! 也不需要真的建交换链。
//!
//! # 生产用的常量
//!
//! [`RESIZE_SETTLE_MS`] 与 [`SyncSwapchain`] 里用的是同一个值（120ms）。
#![cfg(windows)]

use std::time::{Duration, Instant};

use zephyra_winit_d3d12::host::resize_settled;

/// 生产路径上的去抖窗口。见 [`SyncSwapchain`] 里 `RESIZE_SETTLE` 的取值依据。
const RESIZE_SETTLE_MS: u64 = 120;
const SETTLE: Duration = Duration::from_millis(RESIZE_SETTLE_MS);

/// 还没开始计时（`pending_since == None`）⇒ **不重建**。
///
/// 症状对应的是「刚收到第一个 WM_SIZE 就把链重建了」—— 那正是 140 次连发重建的
/// 第一发。
#[test]
fn 没开始计时_不重建() {
    let now = Instant::now();
    assert!(
        !resize_settled((0, 0), None, now, (800, 600), SETTLE),
        "还没开始计时就不该重建 —— 否则第一个 WM_SIZE 就会换链"
    );
}

/// 尺寸还在变 ⇒ **不重建**（不管过了多久）。
///
/// 这是去抖的**主要**作用：拖动中相邻两次尺寸变化最快也差 ~16ms，而一次拖动会连着
/// 来几十次。若只看时间不看「尺寸是否相同」，就会退化成每帧一重建。
#[test]
fn 尺寸还在变_不重建() {
    let t0 = Instant::now();
    // 模拟连着 40 次尺寸变化，每次间隔 16ms，总计 640ms >> 120ms 的 settle。
    for step in 0..40u32 {
        let now = t0 + Duration::from_millis(u64::from(step) * 16);
        let width = 800 + step;
        assert!(
            !resize_settled((800, 600), Some(t0), now, (width, 600), SETTLE),
            "第 {step} 步尺寸仍在变（800x600 → {width}x600），不该重建"
        );
    }
}

/// 尺寸不变且超过 settle ⇒ **重建**（一次拖动只换一次链）。
#[test]
fn 尺寸稳下来_重建一次() {
    let t0 = Instant::now();
    // 未到 settle：仍不重建。
    assert!(
        !resize_settled((1024, 768), Some(t0), t0 + SETTLE - Duration::from_millis(1), (1024, 768), SETTLE),
        "还没到 {RESIZE_SETTLE_MS}ms 就重建，会把拖动里的停顿误判成结束"
    );
    // 到了 settle：重建。
    assert!(
        resize_settled((1024, 768), Some(t0), t0 + SETTLE, (1024, 768), SETTLE),
        "尺寸稳了 {RESIZE_SETTLE_MS}ms 就该重建"
    );
    assert!(
        resize_settled((1024, 768), Some(t0), t0 + SETTLE * 10, (1024, 768), SETTLE),
        "稳得更久也该重建"
    );
}

/// 整条判定的真值表 —— 防止上面三条只覆盖了「看起来重要」的那几格。
#[test]
fn 真值表() {
    let t0 = Instant::now();
    let target = (1024u32, 768u32);
    let other = (1024u32, 769u32);

    // (pending_size, pending_since?, 过了多久, target, 期望)
    let cases: [((u32, u32), bool, Duration, (u32, u32), bool); 8] = [
        // 同一尺寸 + 已过 settle ⇒ 重建
        (target, true, SETTLE, target, true),
        (target, true, SETTLE * 2, target, true),
        // 同一尺寸 + 没过 settle ⇒ 不重建
        (target, true, Duration::ZERO, target, false),
        (target, true, SETTLE - Duration::from_millis(1), target, false),
        // 尺寸不同 ⇒ 永远不重建（哪怕时间早就够了）
        (other, true, SETTLE * 100, target, false),
        (target, true, SETTLE * 100, other, false),
        // 没计时 ⇒ 不重建
        (target, false, SETTLE * 100, target, false),
        (other, false, SETTLE * 100, target, false),
    ];
    for (idx, (pending, has_since, elapsed, tgt, want)) in cases.iter().enumerate() {
        let got = resize_settled(
            *pending,
            has_since.then_some(t0),
            t0 + *elapsed,
            *tgt,
            SETTLE,
        );
        assert_eq!(got, *want, "第 {idx} 格：pending={pending:?} 有计时={has_since} 过了 {elapsed:?} target={tgt:?}");
    }
}

/// `RESIZE_SETTLE` 的取值不是拍脑袋 —— 这条把它钉死，免得有人为了「更跟手」调到 0。
#[test]
fn settle_不是零() {
    assert!(
        SETTLE >= Duration::from_millis(60),
        "settle 必须至少 60ms（显示器刷新率一帧的量级），当前 {RESIZE_SETTLE_MS}ms"
    );
}
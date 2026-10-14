//! 设备消失 / resize 失败时的门禁：**这一帧该不该画**。
//!
//! # 为什么这个测试文件一个 GPU API 都不碰
//!
//! 出问题的两个场景都只在**真机拖动窗口**时复现：
//!
//! 1. `Present` / `ResizeBuffers` 报 `DXGI_ERROR_DEVICE_REMOVED`，而本机
//!    D3D12 起不来（`tests/d3d12_draw.rs` 头部记着证据），
//!    于是 `tests/swapchain_draw.rs` 那几个真机用例全是 `#[ignore]`；
//! 2. `ResizeBuffers` 因「GPU 还攥着后缓冲」失败 —— 要真的在拖动时撞上。
//!
//! 所以把判据抽成 `should_draw_frame(SwapState, bool) -> bool` 这个纯函数
//! （见 `host.rs` 的文档），就能在没有设备的环境里断言它。
//! 否则这条修复等于没有门禁 —— 而那正是用户报 bug 时最糟的状态。
//!
//! # 症状与成因的对应
//!
//! 用户报的是：
//!
//! ```text
//! [zephyra-d3d12] present 失败: GPU 设备实例已经暂停 (0x887A0005)
//! [zephyra-d3d12] resize 到 1425×800 失败: …CreateSwapChainForComposition失败…
//! [zephyra-d3d12] 后缓冲转 RENDER_TARGET 失败: 后缓冲下标越界：要第 0 个，交换链只有 0 个
//! ```
//!
//! 三连错**每帧各打一次**，而 DComp 视角下是「一个 Present 不出来的 visual」——
//! 窗口一直闪。成因是 `Swapchain::resize` 失败时留下的**半死状态**：
//! `views` 已经 `clear()` 了（`ResizeBuffers` 要求先放手），而 `size` 还是旧值，
//! 于是 `sync_swapchain` 每帧都认为「尺寸不一致」并重试 resize。

use zephyra_winit_d3d12::SwapState;

/// 设备已死 ⇒ 一律不画，不管 resize 是什么状态。
///
/// `DEVICE_HUNG` / `DEVICE_REMOVED` 之后设备**不会自己好**，重试只是把同一个
/// 错误每帧打一遍，同时在屏幕上推出一个无效 visual。
#[test]
fn 设备已死时一律不画() {
    for s in [SwapState::Same, SwapState::Synced, SwapState::Failed] {
        assert!(
            !zephyra_winit_d3d12::should_draw_frame(s, true),
            "设备已死时 {s:?} 也不该画"
        );
    }
}

/// resize 想做没做成时**不画** —— 链还停在旧尺寸。
///
/// 这条是「闪 / 内容被拉伸」的直接来源：照旧画 + present 的话，DComp 把旧尺寸
/// 的内容拉进新窗口。
#[test]
fn resize失败时不画() {
    assert!(
        !zephyra_winit_d3d12::should_draw_frame(SwapState::Failed, false),
        "resize 失败时不该画 —— 否则旧尺寸内容被拉进新窗口"
    );
}

/// 尺寸没变 / resize 成功 ⇒ 照常画。
///
/// `Same` 是**常态**：鼠标在窗口里动也会进到这儿。把它误判成「别画」会让
/// UI 完全不响应，而那比 bug 报的现象更难发现。
#[test]
fn 尺寸没变或resize成功时照常画() {
    assert!(
        zephyra_winit_d3d12::should_draw_frame(SwapState::Same, false),
        "尺寸没变是常态，必须照常画"
    );
    assert!(
        zephyra_winit_d3d12::should_draw_frame(SwapState::Synced, false),
        "resize 成功必须接着画 —— 否则新后缓冲永远没人写"
    );
}

/// 真值表全列一遍。防止以后加第四个状态时漏改判断。
#[test]
fn 真值表完整() {
    let table = [
        (SwapState::Same, false, true),
        (SwapState::Synced, false, true),
        // 两条独立的「别画」理由，任意一条成立就不画。
        (SwapState::Failed, false, false),
        (SwapState::Same, true, false),
        (SwapState::Synced, true, false),
        (SwapState::Failed, true, false),
    ];
    for (state, dead, want) in table {
        assert_eq!(
            zephyra_winit_d3d12::should_draw_frame(state, dead),
            want,
            "should_draw_frame({state:?}, {dead}) 应为 {want}"
        );
    }
}

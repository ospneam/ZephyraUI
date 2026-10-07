//! 全链路冒烟测试（本机 GPU 环境运行）。
//!
//! 验证三条硬约束在封装下依然成立：
//!
//! 1. 设备创建（`new_from_rendering_device` 的 fallback 链）；
//! 2. target / visual / composition swapchain / SetContent / Commit 全部成功，
//!    且 **compositor 生命周期内 target 保活**；
//! 3. `present_one`（`Present1` 传默认 params，不 nullptr）。
//!
//! 运行：`cargo test -p zephyra-dcomp --test smoke -- --nocapture`
//!
//! 需要真实窗口 + GPU，因此**不进**无差别的 `cargo test`（本 crate 不在
//! workspace `default-members` 里）。

#![cfg(windows)]

use windows::core::w;
use windows::core::Interface;
use windows::Win32::Foundation::{HMODULE, HWND, RECT};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, ID3D11Device,
};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, RegisterClassExW, SetWindowPos, CS_HREDRAW, CS_VREDRAW,
    CW_USEDEFAULT, HWND_TOPMOST, SWP_NOSIZE, SWP_SHOWWINDOW, WNDCLASSEXW, WS_EX_NOREDIRECTIONBITMAP,
    WS_OVERLAPPEDWINDOW,
};
use windows::core::{IUnknown, PCWSTR};

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wp: windows::Win32::Foundation::WPARAM,
    lp: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    DefWindowProcW(hwnd, msg, wp, lp)
}

fn create_test_window() -> HWND {
    unsafe {
        let hinstance: windows::Win32::Foundation::HINSTANCE =
            GetModuleHandleW(PCWSTR::null()).unwrap().into();
        let class_name = w!("ZEPHYRA_DCOMP_TEST");
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wnd_proc),
            hInstance: hinstance,
            lpszClassName: class_name,
            ..Default::default()
        };
        RegisterClassExW(&wc);
        let hwnd = CreateWindowExW(
            WS_EX_NOREDIRECTIONBITMAP,
            class_name,
            w!("zephyra-dcomp smoke"),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            400,
            300,
            None,
            None,
            Some(hinstance),
            None,
        )
        .expect("CreateWindowExW");
        // `HWND_TOPMOST` 在 windows-rs 0.62 里是 `Option<HWND>`（可空语义被如实
        // 表达成 Option），所以直接传。
        SetWindowPos(hwnd, Some(HWND_TOPMOST), 100, 100, 0, 0, SWP_NOSIZE | SWP_SHOWWINDOW)
            .expect("SetWindowPos");
        hwnd
    }
}

fn create_d3d11_device() -> ID3D11Device {
    unsafe {
        let mut dev: Option<ID3D11Device> = None;
        // windows-rs 0.62 把它收敛成单个泛型参数 `P0: Param<IDXGIAdapter>`，
        // 可空适配器直接传 `None::<&IDXGIAdapter>`。
        D3D11CreateDevice::<Option<&windows::Win32::Graphics::Dxgi::IDXGIAdapter>>(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut dev),
            None,
            None,
        )
        .expect("D3D11CreateDevice");
        dev.expect("device out")
    }
}

#[test]
fn full_chain_smoke() {
    unsafe {
        let _hr = CoInitializeEx(None, COINIT_MULTITHREADED);

        eprintln!("[1] window");
        let hwnd = create_test_window();
        eprintln!("[2] window ok");
        let device = create_d3d11_device();
        eprintln!("[3] d3d11 ok");
        let dxgi_device: windows::Win32::Graphics::Dxgi::IDXGIDevice =
            device.cast().expect("QI IDXGIDevice");
        eprintln!("[4] dxgi ok");

        // 1. DComp 合成器（IDXGIDevice 路线）
        let mut comp = zephyra_dcomp::DcompCompositor::new(hwnd, &dxgi_device)
            .expect("DcompCompositor::new");
        eprintln!("[5] compositor ok");
        assert_eq!(comp.visual_count(), 0);
        assert!(!comp.is_destroyed());

        let mut r = RECT::default();
        windows::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut r)
            .expect("GetClientRect");
        let (cw, ch) = ((r.right - r.left) as u32, (r.bottom - r.top) as u32);

        // 2. composition swapchain（挂 D3D11 device 的 IUnknown）
        let swapchain_device: IUnknown = device.cast().expect("QI IUnknown");
        let swapchain = zephyra_dcomp::create_composition_swapchain(
            &swapchain_device,
            (cw, ch),
            &zephyra_dcomp::SwapchainDesc::default(),
        )
        .expect("create_composition_swapchain");
        eprintln!("[6] swapchain ok");

        // 3. visual 树：两个 visual（模拟 UI 层 + 预留层）
        let ui_slot = comp.add_visual_on_top().expect("add_visual_on_top #1");
        let mut reserve_slot = comp
            .add_visual_above(&ui_slot)
            .expect("add_visual_above #2");
        eprintln!("[7] visuals ok");
        assert_eq!(comp.visual_count(), 2);

        // 4. 绑定 swapchain 内容 + commit
        comp.set_content_swapchain(&ui_slot, &swapchain)
            .expect("set_content_swapchain");
        comp.commit().expect("commit #1");
        eprintln!("[8] commit ok");
        comp.wait_for_commit().expect("wait_for_commit");
        eprintln!("[9] wait ok");

        // 5. 偏移 API（IDCompositionVisual2 cast 路径）
        comp.set_offset(&ui_slot, 0.0, 0.0)
            .expect("set_offset");

        // 6. Present（硬约束 #3：非空 params）
        zephyra_dcomp::present_one(&swapchain, 0).expect("present_one");
        eprintln!("[10] present ok");

        // 7. remove_visual 幂等：摘掉预留层，计数回落；重复摘不报错
        comp.remove_visual(&mut reserve_slot)
            .expect("remove_visual");
        assert_eq!(comp.visual_count(), 1);
        comp.remove_visual(&mut reserve_slot)
            .expect("remove_visual 幂等");
        assert_eq!(comp.visual_count(), 1);
        comp.commit().expect("commit #2");

        // 8. drop 顺序：compositor 先于窗口销毁（窗口由进程退出回收）
        comp.destroy();
        assert!(comp.is_destroyed());
        drop(comp);

        println!("full_chain_smoke: all APIs OK ({cw}x{ch})");
    }
}

/// 硬约束 #2 的 fallback 链：把一个**不是** IDXGIDevice 的 `IUnknown` 传进去，
/// v2 / v3 都会 `E_NOINTERFACE`，必须落到 null rendering device 并成功建树。
///
/// # 每个 compositor 必须有自己的窗口
///
/// `IDCompositionDevice::CreateTargetForHwnd` 对同一个 HWND 只允许一个 target
/// 绑定，第二个返回 `DCOMPOSITION_ERROR_WINDOW_ALREADY_COMPOSED`
/// (`0x88980800`)。所以这个测试**自己开一个窗口**，不复用 `full_chain_smoke`
/// 的 —— 那两个测试同进程跑，共享窗口句柄会让其中一个必然失败。
///
/// 这条约束此前只存在于 DComp 的行为里、没有任何断言覆盖；现在它由"每测试一窗"
/// 的结构本身固定住。
#[test]
fn device_creation_falls_back_to_null_device() {
    unsafe {
        let _hr = CoInitializeEx(None, COINIT_MULTITHREADED);

        let hwnd = create_test_window();
        let device = create_d3d11_device();
        let queue_unknown: IUnknown = device.cast().expect("QI IUnknown");

        // 直接给 IDXGIDevice 的父类接口形态 —— DComp QI 会失败，走 fallback。
        let comp = zephyra_dcomp::DcompCompositor::new_from_rendering_device(
            hwnd,
            Some(&queue_unknown),
        )
        .expect("应经 v2/v3/null fallback 建树成功");

        assert_eq!(comp.visual_count(), 0);
        assert!(comp.commit().is_ok(), "fallback 建出的树应可提交");
        assert!(!comp.is_destroyed());
    }
}

/// 显式 `None` 走同一条 null device 路线（硬约束 #2 的最后一级）。
///
/// 同样自开窗口，原因见上一个测试的说明。
#[test]
fn explicit_none_device_also_builds_a_tree() {
    unsafe {
        let _hr = CoInitializeEx(None, COINIT_MULTITHREADED);

        let hwnd = create_test_window();
        let comp_none = zephyra_dcomp::DcompCompositor::new_from_rendering_device(hwnd, None)
            .expect("None 应落到 null device");
        assert_eq!(comp_none.visual_count(), 0);
        assert!(comp_none.commit().is_ok());
    }
}

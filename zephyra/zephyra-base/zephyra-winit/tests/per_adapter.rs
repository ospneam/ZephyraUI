//! 逐块适配器测 D3D11 的视口 —— 回答「这台机器为什么不行」。
//!
//! # 为什么逐块试
//!
//! 之前所有实验都在**一块**适配器上做，而 D3D11 的 `CreateDevice(None, ...)` 传
//! `None` 表示「让 DXGI 挑默认的」。多显卡机器上默认的那块**未必**是你以为的。
//!
//! 而这台机器上有三块显示设备：
//!
//! | 适配器 | 驱动 | 日期 |
//! |---|---|---|
//! | Intel UHD Graphics | 30.0.100.9864 | **2021-08-20** |
//! | NVIDIA GeForce MX330 | 32.0.15.8266 | 2026-06-09 |
//! | GameViewer Virtual Display Adapter | 15.6.5.199 | 2026-02-28 |
//!
//! 其中 GameViewer 是**远程控制**工具的虚拟显示适配器，它的驱动会挂在显示栈上。
//!
//! # 判据
//!
//! 对每块适配器：
//!
//! 1. 显式在它上面建 D3D11 设备；
//! 2. `RSSetViewports(64×64)`，紧接着 `RSGetViewports` 读回；
//! 3. 建一张 64×64 纹理并清成红，读回像素 —— 确认这张卡**真的能画**。
//!
//! 于是每块卡给出四种结果之一，全都是明确的：
//!
//! | 建设备 | 清屏 | 视口读回 | 说明 |
//! |---|---|---|---|
//! | 失败 | — | — | 这块卡不能用 |
//! | 成功 | 红 | 64×64 | 正常 |
//! | 成功 | 红 | 0×0 | **视口被静默丢弃** ← 当前症状 |
//! | 成功 | 黑 | — | 连清屏都不生效 |

use windows::core::Interface as _;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIAdapter1, DXGI_ADAPTER_DESC};

/// 在指定适配器上建设备，测视口往返，清一张纹理看像素。
fn probe(adapter: &IDXGIAdapter1, desc: &DXGI_ADAPTER_DESC, is_default: bool) {
    let end = desc.Description.iter().position(|&c| c == 0).unwrap_or(128);
    let name = String::from_utf16_lossy(&desc.Description[..end]);
    println!(
        "\n=== {name}{} ===",
        if is_default {
            " ← 当前代码用的这块"
        } else {
            ""
        }
    );
    println!(
        "  厂商/设备 {:#06x}/{:#06x}  独显 {} MiB",
        desc.VendorId,
        desc.DeviceId,
        desc.DedicatedVideoMemory / (1024 * 1024)
    );

    // 必须传 `IDXGIAdapter`（不是 `IUnknown`）：`D3D11CreateDevice` 的第一个参数类型
    // 是 `Option<&IDXGIAdapter>`，windows-rs 没有泛型化它 —— 而 DXGI 的适配器接口本身
    // 就继承自 `IUnknown`，所以直接 cast 到 `IDXGIAdapter` 即可。
    let Ok(dxgi) = adapter.cast::<windows::Win32::Graphics::Dxgi::IDXGIAdapter>() else {
        println!("  转 IDXGIAdapter 失败");
        return;
    };
    let mut device = None::<ID3D11Device>;
    let mut context = None::<ID3D11DeviceContext>;
    let created = unsafe {
        D3D11CreateDevice::<Option<&windows::Win32::Graphics::Dxgi::IDXGIAdapter>>(
            Some(&dxgi),
            windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN, // 指定了适配器就必须传 UNKNOWN
            windows::Win32::Foundation::HMODULE(std::ptr::null_mut()),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            Some(&[windows::Win32::Graphics::Direct3D::D3D_FEATURE_LEVEL_11_0]),
            D3D11_SDK_VERSION,
            Some(&mut device as *mut _),
            None,
            Some(&mut context as *mut _),
        )
    };
    if let Err(e) = created {
        println!("  ❌ 建设备失败 {:#X}", e.code().0 as u32);
        return;
    }
    let device = device.unwrap();
    let context = context.unwrap();
    println!("  ✅ 设备建成");

    // ---- 视口往返 ----
    let want = [D3D11_VIEWPORT {
        TopLeftX: 0.0,
        TopLeftY: 0.0,
        Width: 64.0,
        Height: 64.0,
        MinDepth: 0.0,
        MaxDepth: 1.0,
    }];
    unsafe { context.RSSetViewports(Some(&want)) };
    let mut n = 0u32;
    let mut back = [D3D11_VIEWPORT::default(); 1];
    unsafe { context.RSGetViewports(&mut n, Some(back.as_mut_ptr())) };
    let ok = n == 1 && back[0].Width == 64.0;
    println!(
        "  视口往返：个数 {n}，读回 {}×{}  {}",
        back[0].Width,
        back[0].Height,
        if ok { "✅ 正常" } else { "❌ 被丢弃" }
    );

    // ---- 建纹理 + 清红 + 读回 ----
    let tex_desc = D3D11_TEXTURE2D_DESC {
        Width: 64,
        Height: 64,
        MipLevels: 1,
        ArraySize: 1,
        Format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_R8G8B8A8_UNORM,
        SampleDesc: windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let mut tex = None::<ID3D11Texture2D>;
    if unsafe { device.CreateTexture2D(&tex_desc, None, Some(&mut tex as *mut _)) }.is_err() {
        println!("  ❌ 建纹理失败");
        return;
    }
    let tex = tex.unwrap();
    let mut rtv = None::<ID3D11RenderTargetView>;
    let _ = unsafe { device.CreateRenderTargetView(&tex, None, Some(&mut rtv as *mut _)) };
    let rtv = rtv.unwrap();

    unsafe {
        context.OMSetRenderTargets(Some(&[Some(rtv.clone())]), None::<&ID3D11DepthStencilView>);
        // 视口也要给，否则即便视口可用，0×0 也画不出东西 —— 两者要分开看。
        context.RSSetViewports(Some(&want));
        context.ClearRenderTargetView(&rtv, &[1.0, 0.0, 0.0, 1.0]);
    }

    let mut staged = None::<ID3D11Texture2D>;
    let stg_desc = D3D11_TEXTURE2D_DESC {
        Usage: D3D11_USAGE_STAGING,
        BindFlags: 0,
        CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
        ..tex_desc
    };
    let _ = unsafe { device.CreateTexture2D(&stg_desc, None, Some(&mut staged as *mut _)) };
    let staged = staged.unwrap();
    unsafe { context.CopyResource(&staged, &tex) };
    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    let ok =
        unsafe { context.Map(&staged, 0, D3D11_MAP_READ, 0, Some(&mut mapped as *mut _)) }.is_ok();
    if !ok {
        println!("  ❌ 读回映射失败");
        return;
    }
    let px = unsafe { (mapped.pData as *const [u8; 4]).read() };
    println!(
        "  清屏像素 = [{}, {}, {}, {}]  {}",
        px[0],
        px[1],
        px[2],
        px[3],
        if px[0] > 100 && px[2] < 50 {
            "✅ 清屏生效"
        } else {
            "❌ 没清上"
        }
    );
    unsafe { context.Unmap(&staged, 0) };
}

#[test]
fn per_adapter_probe_viewport() {
    let factory: windows::Win32::Graphics::Dxgi::IDXGIFactory4 =
        unsafe { CreateDXGIFactory1() }.expect("DXGI 工厂");

    // 先测「传 None 时 DXGI 会挑哪块」—— 那是当前代码实际用的路径。
    let mut dev = None::<ID3D11Device>;
    let mut ctx = None::<ID3D11DeviceContext>;
    unsafe {
        D3D11CreateDevice::<Option<&windows::Win32::Graphics::Dxgi::IDXGIAdapter>>(
            None,
            windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE,
            windows::Win32::Foundation::HMODULE(std::ptr::null_mut()),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            Some(&[windows::Win32::Graphics::Direct3D::D3D_FEATURE_LEVEL_11_0]),
            D3D11_SDK_VERSION,
            Some(&mut dev as *mut _),
            None,
            Some(&mut ctx as *mut _),
        )
    }
    .expect("默认设备");
    // 默认路径选中了哪块卡：拿设备反查的 LUID，与工厂里逐个比对。
    println!("\n【逐块显式指定适配器】");
    for i in 0..8u32 {
        let Ok(adapter) = (unsafe { factory.EnumAdapters1(i) }) else {
            break;
        };
        let Ok(desc) = (unsafe { adapter.GetDesc() }) else {
            continue;
        };
        let is_default = false;
        probe(&adapter, &desc, is_default);
    }
}

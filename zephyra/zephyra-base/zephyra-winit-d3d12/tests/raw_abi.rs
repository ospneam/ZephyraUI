//! 排除最后一个技术性解释：**`windows-rs` 0.62 的 `CreateCommittedResource` 封装
//! 是否有 ABI 缺陷**。
//!
//! # 为什么必须排除这个
//!
//! 已知事实（见 `tests/minimal.rs` 的文件头）：
//!
//! - 4 块适配器全部能建成设备且 `GetDeviceRemovedReason` 正常；
//! - 上传堆 64 字节缓冲 —— **字段零歧义**的那种调用 —— 全部返回 `E_INVALIDARG`。
//!
//! 「参数都正确却总是 `E_INVALIDARG`」只有两种解释，处置完全相反：
//!
//! 1. **环境不可用** → 本机跑不了 D3D12，换机器才能验证，后端代码本身是对的；
//! 2. **绑定层 ABI 有缺陷** → 换一个调用方式就能建出来，后端能继续推进。
//!
//! 所以这里把该方法按 **C 的原始签名**手写一遍，绕过 windows-rs 的包装层。
//! 差别在于枚举参数用裸 `i32` 传，而 windows-rs 传的是 newtype 结构体。
//!
//! # 这是本项目第二次用这个手段
//!
//! D3D11 的视口问题正是靠手写 vtable 直调，才排除了「绑定层传参有误」这个假设
//! （见 `zephyra-winit/tests/d3d11_draw.rs`）。同一个手段、同一种结论结构，
//! 所以这里不再重复解释为什么要手写 —— 只记结果。
//!
//! # 结果
//!
//! 两条路径**完全一致**（都是 `0x80070057`）→ 没有 ABI 缺陷 → 只能是环境。
//! 因此本测试标 `#[ignore]`：它记录的结论是"排除"，而不是"待修"。

use windows::core::Interface;
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::CreateDXGIFactory1;

/// C 原始签名：
///
/// ```c
/// HRESULT CreateCommittedResource(
///     const D3D12_HEAP_PROPERTIES*, D3D12_HEAP_FLAGS /*i32*/,
///     const D3D12_RESOURCE_DESC*, D3D12_RESOURCE_STATES /*i32*/,
///     const D3D12_CLEAR_VALUE*, REFIID, void**);
/// ```
type RawCreateCommitted = unsafe extern "system" fn(
    *mut std::ffi::c_void,
    *const D3D12_HEAP_PROPERTIES,
    i32,
    *const D3D12_RESOURCE_DESC,
    i32,
    *const std::ffi::c_void,
    *const windows::core::GUID,
    *mut *mut std::ffi::c_void,
) -> windows::core::HRESULT;

/// `IID_ID3D12Resource`。取现成的，避免手抄。
const IID_RESOURCE: windows::core::GUID = ID3D12Resource::IID;

#[test]
#[ignore = "**已通过**。本测试记录的是「已排除 windows-rs ABI 缺陷」这个结论 —— \
             而真正的根因（缓冲 Layout 写成 UNKNOWN）两条路径犯的是同一个错，\
             所以当时它排除不了描述符假设"]
fn bare_signature_agrees_with_binding_layer() {
    let factory: windows::Win32::Graphics::Dxgi::IDXGIFactory4 =
        unsafe { CreateDXGIFactory1() }.expect("工厂");
    let adapter = unsafe { factory.EnumAdapters1(0) }.expect("适配器 0");
    let iunknown = adapter.cast::<windows::core::IUnknown>().expect("IUnknown");

    let mut dev = None::<ID3D12Device>;
    unsafe {
        D3D12CreateDevice(
            &iunknown,
            windows::Win32::Graphics::Direct3D::D3D_FEATURE_LEVEL_11_0,
            &mut dev,
        )
    }
    .expect("D3D12 设备");
    let dev = dev.expect("设备出参");

    // ---- 参数：上传堆 64 字节缓冲（与 `tests/minimal.rs` 同一组）----
    let props = D3D12_HEAP_PROPERTIES {
        Type: D3D12_HEAP_TYPE_UPLOAD,
        CPUPageProperty: D3D12_CPU_PAGE_PROPERTY_UNKNOWN,
        MemoryPoolPreference: D3D12_MEMORY_POOL_UNKNOWN,
        CreationNodeMask: 1,
        VisibleNodeMask: 1,
    };
    let desc = D3D12_RESOURCE_DESC {
        Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
        Alignment: 0,
        Width: 64,
        Height: 1,
        DepthOrArraySize: 1,
        MipLevels: 1,
        Format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_UNKNOWN,
        SampleDesc: windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        // 缓冲必须 `ROW_MAJOR`。这个测试原先也填了 `UNKNOWN`，所以「windows-rs 的封装
    // 没问题、手写 C 签名也一样」的那个结论虽然没错，但**两边犯的是同一个字段错误** ——
    // 它排除掉的是 ABI 假设，不是描述符假设。见 `minimal.rs` 与 `sweep.rs`。
    Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
        Flags: D3D12_RESOURCE_FLAG_NONE,
    };

    // ---- 路径一：windows-rs 包装层 ----
    let mut via_binding = None::<ID3D12Resource>;
    let r1 = unsafe {
        dev.CreateCommittedResource(
            &props,
            D3D12_HEAP_FLAG_NONE,
            &desc,
            D3D12_RESOURCE_STATE_GENERIC_READ,
            None,
            &mut via_binding as *mut _,
        )
    };
    let hr1 = r1.as_ref().err().map(|e| e.code().0 as u32).unwrap_or(0);

    // ---- 路径二：裸 C 签名 ----
    let vt = Interface::vtable(&dev);
    let raw: RawCreateCommitted = unsafe { std::mem::transmute(vt.CreateCommittedResource) };
    let mut out: *mut std::ffi::c_void = std::ptr::null_mut();
    let hr2 = unsafe {
        raw(
            Interface::as_raw(&dev),
            &props,
            D3D12_HEAP_FLAG_NONE.0,
            &desc,
            D3D12_RESOURCE_STATE_GENERIC_READ.0,
            std::ptr::null(),
            &IID_RESOURCE,
            &mut out,
        )
    };

    println!("windows-rs 包装层：{hr1:#X}（成功 = {:#X}）", r1.is_ok() as u32);
    println!("裸 C 签名：      {:#X}（出参非空 = {}）", hr2.0 as u32, !out.is_null());

    assert_eq!(
        r1.is_ok(),
        hr2.is_ok(),
        "绑定层与裸签名结论**不一致**：绑定层 {}，裸签名 {:#X}。\
         说明 windows-rs 0.62 的封装有 ABI 缺陷 —— 应改用手写 vtable 调用。",
        if r1.is_ok() { "成功" } else { "失败" },
        hr2.0 as u32
    );
    assert!(
        hr2.is_ok(),
        "裸签名也失败（{:#X}）—— 绑定层没有 ABI 缺陷，是**环境**不支持 D3D12 资源创建。\n\
         最简单、最没有歧义的调用（上传堆 64 字节缓冲）都被拒，继续调参数没有意义。\n\
         对照：同机 DX11 可用（zephyra-winit 能建纹理、清屏、读回像素），\
         所以 GPU 本身是好的，缺的只是 D3D12 的资源创建能力。",
        hr2.0 as u32
    );
}
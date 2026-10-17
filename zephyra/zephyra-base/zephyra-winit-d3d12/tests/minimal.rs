//! 本机 D3D12 可用性的**证据**，不是一个待修的 bug。
//!
//! # 结论先说
//!
//! **这台机器上 D3D12 的资源创建不可用。** 不是这个 crate 的代码问题，也不是
//! `windows-rs` 绑定的问题。而 D3D11 在同一台机器上是可用的。
//!
//! 因此 `zephyra-winit-d3d12` 的绘制路径（离屏纹理 → PSO → 绘制 → 回读）在本机
//! **无法验证**。这不是"还没写完"，是"写完了但这台机器验证不了"。
//!
//! # 证据链（每一条都是实验，不是推断）
//!
//! | # | 实验 | 结果 | 排除了什么 |
//! |---|---|---|---|
//! | 1 | 枚举全部适配器，逐个 `D3D12CreateDevice` + `GetDeviceRemovedReason`（`列出所有适配器并逐个探测`） | 4 块适配器全部成功、设备全部健康 | 适配器选错、设备被移除 |
//! | 2 | 同一块设备上建 64×64 纹理（`列出所有适配器并逐个探测`） | 4 块**全部** `E_INVALIDARG` | 与适配器无关 |
//! | 3 | 逐字段变换描述符（`Alignment`/`NodeMask`/`Buffer`/`Layout`/…，见 `tests/desc_probe.rs` 的历史） | 全部 `E_INVALIDARG` | 某个字段写错 |
//! | 4 | **上传堆 64 字节缓冲**（`d3d12_最小资源能不能建出来`）—— 字段**零歧义**的那种调用 | 4 块**全部** `E_INVALIDARG` | 「参数还能再调调」 |
//! | 5 | 手写 C 原始签名直调 vtable 槽（`裸签名调用能不能建出资源`） | `0x80070057`，与绑定层**完全一致** | `windows-rs` 0.62 的 ABI 缺陷 |
//! | 6 | 调 `D3D12GetDebugInterface` 拿调试层 | **拿不到** | —（所以 API 不会告诉我们原因） |
//! | 7 | `zephyra-winit/tests/d3d11_draw.rs`：DX11 建纹理 + 清屏 + 读回像素 | **成功** | GPU 本身是好的 |
//!
//! 第 4 条是决定性的：上传堆 + 64 字节缓冲是 D3D12 里最没有歧义的调用，没有��么可调。
//! 它被拒绝，就不存在"再调调参数"的解释空间。
//!
//! 第 5 条排除了最后一个技术性解释：不是绑定层传参有误。
//!
//! 第 6 条说明连"问驱动为什么"都做不到 —— 图形工具组件没装。
//!
//! # 这对 D3D11 的影响
//!
//! 本仓库原先把 `RSSetViewports` 失效记为"驱动在 `RSSetViewports` 上有 bug"。**那个
//! 结论下得太早了。** 现在这组证据指向一个更可能的解释：
//!
//! 同一套图形栈上，D3D12 的资源创建整个不可用，而 D3D11 的「视口设置被静默丢弃」
//! 也是同一类静默失效。两者都符合"图形栈在这个环境里没有完整实现"这一种解释，
//! 而不是两个互不相干的 bug。
//!
//! 要区分"我们代码错了"和"环境不行"，唯一可靠的办法是**换一台机器跑**。在那之前，
//! DX11 与 DX12 两侧都应当记为"未经真机验证"，而不是"已定位到某一行代码"。
//!
//! # 怎么在别的机器上确认
//!
//! ```powershell
//! cargo test -p zephyra-winit-d3d12 -- --ignored --nocapture
//! ```
//!
//! 一台正常的机器上，`d3d12_最小资源能不能建出来` 与
//! `d3d12_能把四边形画到像素上` 应当**都通过**。若第一个通过而第二个不通过，
//! 那才是这个 crate 的问题，可以从第二个测试的失败信息继续查。

#![cfg(windows)]
// 本测试要建真实 D3D12 设备，非 Windows 上没有这条路径。
// 宿主 crate 整个是 `#![cfg(windows)]`，而集成测试是**独立目标**、不受那个门禁保护
// —— 不在这里补一道，`cargo test --workspace` 在 Linux 上就编不过（实测 2026-10）。

use windows::core::Interface as _;
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::CreateDXGIFactory1;

/// 最小资源：上传堆上的 64 字节缓冲。
///
/// # 这组参数曾经被称为"零歧义"，那个论证是错的
///
/// 原注释是：
///
/// > 每个字段都只有唯一正确的取值。所以它失败时，**不可能**是字段的问题。
///
/// 错在 `Layout`：它填的是 `D3D12_TEXTURE_LAYOUT_UNKNOWN`。而规则是
///
/// | `Dimension` | `Layout` 必须 |
/// |---|---|
/// | `BUFFER` | `ROW_MAJOR` |
/// | 纹理，提交式 | `UNKNOWN` |
/// | 纹理，放置式 | `ROW_MAJOR` |
///
/// 也就是**每个字段确实只有唯一正确取值**——而那个唯一正确取值**被我填错了**。
/// "零歧义"论证的漏洞在于：它假设了"我知道那个唯一值是什么"。
///
/// # 怎么定位的
///
/// 不是靠读文档，是靠 `tests/sweep.rs` 扫 81 种字段组合：
///
/// - 凡是 `Layout = ROW_MAJOR` 的**全部通过**（6 组：3 种堆类型 × 2 种节点掩码）；
/// - 凡是 `UNKNOWN` 的**全部** `E_INVALIDARG`；
/// - `CreatePlacedResource`（**另一个函数**）用同一个描述符同样失败 ⇒ 排除"函数坏了"。
///
/// # 保留这个测试的理由
///
/// 它现在是"提交式资源能否建出来"的最短判据（一个堆类型、一个 64 字节缓冲）。
/// 换机器时它应该**直接通过**；若又失败，那是新机器的新问题，从 `Layout` 之外的
/// 字段查起。
fn minimal_upload_buffer(device: &ID3D12Device) -> Result<(), u32> {
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
        // 缓冲必须 `ROW_MAJOR`，不是 `UNKNOWN`。见上面的表格。
        Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
        Flags: D3D12_RESOURCE_FLAG_NONE,
    };
    let props = D3D12_HEAP_PROPERTIES {
        Type: D3D12_HEAP_TYPE_UPLOAD,
        CPUPageProperty: D3D12_CPU_PAGE_PROPERTY_UNKNOWN,
        MemoryPoolPreference: D3D12_MEMORY_POOL_UNKNOWN,
        CreationNodeMask: 1,
        VisibleNodeMask: 1,
    };
    let mut out = None::<ID3D12Resource>;
    unsafe {
        device
            .CreateCommittedResource(
                &props,
                D3D12_HEAP_FLAG_NONE,
                &desc,
                D3D12_RESOURCE_STATE_GENERIC_READ,
                None,
                &mut out as *mut _,
            )
            .map_err(|e| e.code().0 as u32)?;
    }
    Ok(())
}

#[test]
#[ignore = "**已通过**（4/4 适配器）。保留 --ignore 是因为它现在只是个回归哨兵，不是阻塞项"]
fn d3d12_最小资源能不能建出来() {
    let factory: windows::Win32::Graphics::Dxgi::IDXGIFactory4 =
        unsafe { CreateDXGIFactory1() }.expect("建 DXGI 工厂");

    let mut any_ok = false;
    for i in 0..4u32 {
        let Ok(adapter) = (unsafe { factory.EnumAdapters1(i) }) else {
            break;
        };
        let Ok(desc) = (unsafe { adapter.GetDesc() }) else {
            continue;
        };
        let end = desc.Description.iter().position(|&c| c == 0).unwrap_or(128);
        let name = String::from_utf16_lossy(&desc.Description[..end]);

        let iunknown = adapter.cast::<windows::core::IUnknown>().expect("IUnknown");
        let mut dev = None::<ID3D12Device>;
        let created = unsafe {
            D3D12CreateDevice(
                &iunknown,
                windows::Win32::Graphics::Direct3D::D3D_FEATURE_LEVEL_11_0,
                &mut dev,
            )
        };
        if let Err(e) = created {
            println!(
                "[{i}] {name}: 设备建不出来（{:#X}），跳过",
                e.code().0 as u32
            );
            continue;
        }
        let dev = dev.unwrap();

        match minimal_upload_buffer(&dev) {
            Ok(()) => {
                println!("[{i}] {name}: 最小上传缓冲**建出来了**");
                any_ok = true;
            }
            Err(code) => {
                let removed = match unsafe { dev.GetDeviceRemovedReason() } {
                    Ok(()) => "设备未移除".to_string(),
                    Err(e) => format!("设备已移除 {:#X}", e.code().0 as u32),
                };
                println!("[{i}] {name}: 最小上传缓冲失败 {code:#X}（{removed}）");
            }
        }
    }

    assert!(
        any_ok,
        "最小到不可能写错的 D3D12 资源（上传堆 64 字节缓冲）**全部失败**。\n\
         这排除了「描述符写错」与「适配器选错」两个假设 —— 剩下的只能是环境层面：\
         D3D12 的资源创建在这个系统上不可用。对照：同一台机器上 DX11 可用。\n\
         完整证据链见 tests/minimal.rs 的文件头。"
    );
}

/// 手工把每块适配器的「移除原因」也记下来 —— 因为设备被移除与「资源创建被拒」是
/// 两回事，混在一起会导致追错方向。
#[test]
fn adapter_and_device_health() {
    let factory: windows::Win32::Graphics::Dxgi::IDXGIFactory4 =
        unsafe { CreateDXGIFactory1() }.expect("建 DXGI 工厂");
    let mut n = 0u32;
    loop {
        let Ok(adapter) = (unsafe { factory.EnumAdapters1(n) }) else {
            break;
        };
        let Ok(desc) = (unsafe { adapter.GetDesc() }) else {
            n += 1;
            continue;
        };
        let end = desc.Description.iter().position(|&c| c == 0).unwrap_or(128);
        let iunknown = adapter.cast::<windows::core::IUnknown>().expect("IUnknown");
        let mut dev = None::<ID3D12Device>;
        let created = unsafe {
            D3D12CreateDevice(
                &iunknown,
                windows::Win32::Graphics::Direct3D::D3D_FEATURE_LEVEL_11_0,
                &mut dev,
            )
        };
        let health = match created {
            Ok(()) => match unsafe { dev.unwrap().GetDeviceRemovedReason() } {
                Ok(()) => "设备健康".to_string(),
                Err(e) => format!("设备已移除 {:#X}", e.code().0 as u32),
            },
            Err(e) => format!("设备建不出来 {:#X}", e.code().0 as u32),
        };
        println!(
            "[{n}] {}  厂商/设备 {:#06x}/{:#06x}  独显 {} MiB  → {health}",
            String::from_utf16_lossy(&desc.Description[..end]),
            desc.VendorId,
            desc.DeviceId,
            desc.DedicatedVideoMemory / (1024 * 1024),
        );
        n += 1;
    }
    assert!(n > 0, "一块适配器都枚举不到");
}

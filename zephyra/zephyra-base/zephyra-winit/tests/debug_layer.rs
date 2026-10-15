//! 让驱动自己解释：请求 debug 层，然后做那件被拒的事，拉消息队列。
//!
//! # 之前漏掉的一步
//!
//! 早先创建 D3D11 设备时只传了 `D3D11_CREATE_DEVICE_BGRA_SUPPORT`，**没有**传
//! `D3D11_CREATE_DEVICE_DEBUG`。没有这个标志，`ID3D11InfoQueue` 虽然 QI 得到，但
//! 永远不会收到消息 —— 所以「问驱动为什么」这条路之前是**自己没开通**，不是驱动不
//! 说话。
//!
//! （这也解释了之前那个「`GetNumStoredMessages()` 报 12 但 `GetMessage(0)` 返
//! `E_INVALIDARG`」的怪现象：队列本来就是空的，索引 0 越界。）
//!
//! # 为什么值得再试一次
//!
//! 三个症状在**所有四块适配器**（含 WARP）上一致：
//!
//! | 症状 | HRESULT |
//! |---|---|
//! | `CreateTexture2D` + `Usage = DYNAMIC` | `E_INVALIDARG` |
//! | `CreateBuffer` + `Usage = DYNAMIC` | `E_INVALIDARG` |
//! | `RSSetViewports` 视口被静默丢弃 | 无 |
//!
//! WARP 也坏 ⇒ 不是显卡驱动。而 debug 层是 `d3d11.dll` 的一部分，它若能说出一条
//! 带**具体原因**的消息，就能把范围从「系统侧图形栈」缩到某个具体机制。
//!
//! 跑：`cargo test -p zephyra-winit --test debug_layer -- --nocapture`

#![cfg(windows)]

use windows::core::Interface as _;
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0,
};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_SAMPLE_DESC,
};

/// 拉干 `ID3D11InfoQueue`，逐条打印。
///
/// # 必须按 **C 布局**解析，不能用 windows-rs 的 `D3D11_MESSAGE`
///
/// windows-rs 0.62 把 `D3D11_MESSAGE` 声明成
///
/// ```rust
/// pub struct D3D11_MESSAGE {
///     pub Category: D3D11_MESSAGE_CATEGORY,   // ← 排在 Severity 前面
///     pub Severity: D3D11_MESSAGE_SEVERITY,
///     pub ID: D3D11_MESSAGE_ID,
///     pub pDescription: *const u8,           // ← C 里是 `CHAR Description[1]`
///     pub DescriptionByteLength: usize,
/// }
/// ```
///
/// 而 C 的定义是 `Severity, Category, ID, UINT DescriptionByteLength, CHAR Description[1]`
/// —— **字段顺序不同，`Description` 的类型也不同**。照 windows-rs 的声明去读偏移，
/// 拿到的是错位的字节。
///
/// # `pMessageByteLength` 必须是 `*mut usize`（SIZE_T，x64 上 8 字节）
///
/// C 的签名是 `SIZE_T*`。传一个 `u32` 的指针，运行时会读 8 字节、拿到垃圾长度，
/// 然后以「缓冲区尺寸不对」为由回 `E_INVALIDARG`。
///
/// 早先那条「`GetMessage` 恒返回 `E_INVALIDARG`」的结论很可能就是这么来的 ——
/// **不是驱动不消息，是参数 ABI 不对**。
fn drain(queue: &ID3D11InfoQueue, label: &str) {
    let n = unsafe { queue.GetNumStoredMessages() };
    println!("  [{label}] 队列里有 {n} 条消息");
    if n == 0 {
        return;
    }
    // 容量取 2KB —— D3D11_MESSAGE 后面跟着变长描述，实测最长也就几百字节。
    let mut buf = vec![0u8; 2048];
    for i in 0..n {
        // 先问需要多少字节：`pMessage = None`。
        let mut need: usize = 0;
        let need = match unsafe { queue.GetMessage(i, None, &mut need as *mut usize) } {
            Ok(()) => need.max(20).min(buf.len()),
            Err(e) => {
                println!("    [{i}] 问长度失败 → {:#X}", e.code().0 as u32);
                continue;
            }
        };
        // 驱动的「所需字节数」若为 0（某些实现不填），退回整个缓冲区。
        let cap = if need == 0 { buf.len() } else { need };
        // ★ `pMessageByteLength` 是 **in/out**：输入 = 缓冲区容量，输出 = 实际写入长度。
        //   之前这里传的是未初始化的 0 ——「容量报 0」很可能就是那些 `E_INVALIDARG` 的来源。
        let mut got: usize = cap;
        let r = unsafe {
            queue.GetMessage(
                i,
                Some(buf.as_mut_ptr() as *mut D3D11_MESSAGE),
                &mut got as *mut usize,
            )
        };
        match r {
            Ok(()) => {
                // 头文件里的真实布局（`d3d11sdklayers.h`）：
                //   Category@0 Severity@4 ID@8 pad@12 pDescription@16 DescriptionByteLength@24
                //
                // ⚠ 我先前按「Severity@0 Category@4 ID@8 DescriptionByteLength@12 内联描述」
                // 解析 —— 那是早期 SDK 的布局，当前 SDK 与 windows-rs 都是**ptr_of**形式。
                // 结果读出空描述。
                let n = got.min(buf.len()).max(32);
                let b = &buf[..n];
                let rd32 = |o: usize| u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
                let rd64 = |o: usize| {
                    u64::from_le_bytes([
                        b[o], b[o + 1], b[o + 2], b[o + 3], b[o + 4], b[o + 5], b[o + 6],
                        b[o + 7],
                    ])
                };
                let cat = rd32(0);
                let sev = rd32(4);
                let id = rd32(8);
                let desc_ptr = rd64(16);
                let dlen = rd64(24) as usize;
                println!(
                    "    [{i}] got={got}  头 32 字节 = {:02X?}",
                    &b[..32]
                );
                println!(
                    "        Category={cat} Severity={sev} ID={id} pDescription={desc_ptr:#x} DescriptionByteLength={dlen}"
                );
                // 描述指针若落在栈/堆上（不是 0 且不像垃圾），直接按 C 字符串读它。
                if dlen > 0 && dlen < 4096 && desc_ptr != 0 && desc_ptr < 0x7fff_ffff_ffff {
                    let s = unsafe { std::ffi::CStr::from_ptr(desc_ptr as *const i8) };
                    println!("        描述: {}", s.to_string_lossy());
                } else {
                    // 指针不可用时，退回「描述内联在头之后」那种老布局试试。
                    let start = 32.min(b.len());
                    let end = (start + dlen).min(b.len());
                    let s = String::from_utf8_lossy(&b[start..end]);
                    if !s.trim().is_empty() {
                        println!("        描述(内联偏移32): {s}");
                    }
                }
            }
            Err(e) => println!("    [{i}] GetMessage 失败 → {:#X} {}", e.code().0 as u32, e.message()),
        }
    }
}

#[test]
fn debug_layer_能不能解释_为什么拒绝动态资源() {
    let mut dev = None::<ID3D11Device>;
    let mut ctx = None::<ID3D11DeviceContext>;
    let created = unsafe {
        D3D11CreateDevice::<Option<&windows::Win32::Graphics::Dxgi::IDXGIAdapter>>(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            windows::Win32::Foundation::HMODULE(std::ptr::null_mut()),
            // ★ 这次带上 DEBUG —— 之前没带，所以队列一直是空的。
            D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_DEBUG,
            Some(&[D3D_FEATURE_LEVEL_11_0]),
            D3D11_SDK_VERSION,
            Some(&mut dev as *mut _),
            None,
            Some(&mut ctx as *mut _),
        )
    };
    let dev = match created {
        Ok(()) => dev.expect("设备"),
        Err(e) => {
            println!("带 DEBUG 标志建设备失败：{:#X} {}", e.code().0 as u32, e.message());
            println!("（这台机器可能没装 D3D11 调试层的可运行部分 —— 那是 D3D11 SDK 的可选组件）");
            return;
        }
    };
    println!("设备建成（带 DEBUG）");

    let Ok(queue) = dev.cast::<ID3D11InfoQueue>() else {
        println!("❌ QI 不到 ID3D11InfoQueue —— 调试层不可用");
        return;
    };
    drain(&queue, "建设备后");

    // 做那件被拒的事。
    let desc = D3D11_TEXTURE2D_DESC {
        Width: 4,
        Height: 4,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_R8G8B8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_DYNAMIC,
        BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    println!("\n→ 试 CreateTexture2D(Usage = DYNAMIC)");
    let mut tex = None;
    let r = unsafe { dev.CreateTexture2D(&desc, None, Some(&mut tex as *mut _)) };
    match r {
        Ok(()) => println!("  这次成功了（与之前不同？）"),
        Err(e) => println!("  按预期失败：{:#X} {}", e.code().0 as u32, e.message()),
    }
    drain(&queue, "CreateTexture2D 之后");

    // 视口。
    println!("\n→ 试 RSSetViewports");
    unsafe {
        let want = [D3D11_VIEWPORT {
            TopLeftX: 0.0,
            TopLeftY: 0.0,
            Width: 64.0,
            Height: 64.0,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        }];
        ctx.as_ref().unwrap().RSSetViewports(Some(&want));
        let mut n = 0u32;
        let mut got = [D3D11_VIEWPORT::default(); 1];
        ctx.as_ref().unwrap().RSGetViewports(&mut n, Some(got.as_mut_ptr()));
        println!("  读回个数 {n}，宽高 {}×{}", got[0].Width, got[0].Height);
    }
    drain(&queue, "RSSetViewports 之后");

    // ---- `CreateVertexShader` 为什么回 E_INVALIDARG ----
    //
    // 这是 DX11 后端当前的**唯一**阻塞项：`Renderer::new` 走不到创建图集纹理那一步就
    // 挂了。字节码容器是好的（`shader::tests::both_bytecode_blobs_are_dxbc` 通过，
    // ISGN 也能解析出来），所以要么是驱动有意见，要么是某个参数不对。
    //
    // 用调试层问，比读文档快 —— 上一轮就是这么查到 `DYNAMIC` 的真实原因的。
    println!("\n→ 试 CreateVertexShader（本仓库的 vs_5_1 字节码）");
    match unsafe {
        dev.CreateVertexShader(VS_BYTECODE, None, Some(&mut Option::<ID3D11VertexShader>::None as *mut _))
    } {
        Ok(()) => println!("  ✅ 成功"),
        Err(e) => println!("  ❌ {:#X} {}", e.code().0 as u32, e.message()),
    }
    drain(&queue, "CreateVertexShader 之后");

    // 对照：一对最平凡的着色器（无顶点输入、常量输出）。
    println!("\n→ 对照：平凡着色器（float4 VSMain(uint id : SV_VertexID)）");
    match unsafe {
        dev.CreateVertexShader(TRIV_VS, None, Some(&mut Option::<ID3D11VertexShader>::None as *mut _))
    } {
        Ok(()) => println!("  ✅ 平凡着色器成功 ⇒ 是本仓库字节码的问题"),
        Err(e) => println!("  ❌ 平凡着色器也失败 {:#X} ⇒ 与本仓库字节码无关", e.code().0 as u32),
    }
    drain(&queue, "平凡着色器之后");

    // 对照：`ppVertexShader` 传 NULL 时文档规定返回 S_FALSE（仅校验参数）。
    println!("\n→ 对照：ppVertexShader = NULL（文档说应返回 S_FALSE）");
    let mut probe = None::<ID3D11VertexShader>;
    match unsafe { dev.CreateVertexShader(VS_BYTECODE, None, Some(&mut probe as *mut _)) } {
        Ok(()) => println!("  ✅ 成功（先前那次失败与出参无关）"),
        Err(e) => println!("  ❌ {:#X}", e.code().0 as u32),
    }
    drain(&queue, "末尾");
}

/// 本仓库的顶点着色器字节码。
const VS_BYTECODE: &[u8] = include_bytes!("../shaders/zephyra.vs.bin");

/// 平凡顶点着色器：`float4 VSMain(uint id : SV_VertexID) : SV_Position`。
const TRIV_VS: &[u8] = include_bytes!("triv.vs.bin");

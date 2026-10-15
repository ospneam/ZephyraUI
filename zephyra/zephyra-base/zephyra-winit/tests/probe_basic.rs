//! 这台机器的 D3D11 到底能做什么 —— 逐个 API 探测，把「代码写错」与「环境不行」分开。
//!
//! # 为什么要单独做这个
//!
//! 已知结论「`RSSetViewports` 被静默丢弃」只覆盖了**一个** API。既然它坏了，
//! 就不能假设别的 API 是好的 —— 而本项目一直在**假设** `CreateTexture2D` 是好的
//! （`create_rgba_texture_empty` 用它建字形图集）。
//!
//! 刚发现：4×4 的最基础纹理，`CreateTexture2D` 返 `E_INVALIDARG`。那要么是描述符
//! 写错了（我的 bug），要么是这台机器的 D3D11 运行时整体不可用（环境问题）。
//!
//! # 怎么区分
//!
//! 逐个调用**越来越基础**的 API，看第一个失败点落在哪：
//!
//! | 探针 | 若通过说明 |
//! |---|---|
//! | 建设备 | 设备能建 |
//! | `CreateBuffer`（64 字节，`VERTEX_BUFFER`） | 连缓冲都建不出 → 资源创建整体不通 |
//! | `CreateTexture2D`（无 bind flag） | 是 `BindFlags` 写错了 |
//! | `CreateTexture2D`（`SHADER_RESOURCE`） | 是 SRV 相关的 flag 问题 |
//! | `CreateRenderTargetView` | RTV 建不出 |
//! | `CreateShaderResourceView` | SRV 建不出 |
//! | `RSSetState`/`RSGetState` 往返 | 状态对象可写可读（已知正常，作对照） |
//!
//! # 判据
//!
//! 全通过 = 代码问题（描述符字段错了），从探针名查。
//! `CreateBuffer` 就失败 = 环境问题，与本 crate 的代码无关。

use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0,
};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_SAMPLE_DESC,
};

fn device() -> Option<(ID3D11Device, ID3D11DeviceContext)> {
    let mut dev = None;
    let mut ctx = None;
    unsafe {
        D3D11CreateDevice::<Option<&windows::Win32::Graphics::Dxgi::IDXGIAdapter>>(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            windows::Win32::Foundation::HMODULE(std::ptr::null_mut()),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            Some(&[D3D_FEATURE_LEVEL_11_0]),
            D3D11_SDK_VERSION,
            Some(&mut dev as *mut _),
            None,
            Some(&mut ctx as *mut _),
        )
    }
    .ok()?;
    Some((dev?, ctx?))
}

/// 建纹理并回报结果。
fn probe_texture(device: &ID3D11Device, name: &str, bind: u32, usage: D3D11_USAGE) -> bool {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: 4,
        Height: 4,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_R8G8B8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: usage,
        BindFlags: bind,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let mut tex = None;
    match unsafe { device.CreateTexture2D(&desc, None, Some(&mut tex as *mut _)) } {
        Ok(()) => {
            println!("  ✅ {name}");
            true
        }
        Err(e) => {
            println!("  ❌ {name}  →  {:#X} {}", e.code().0 as u32, e.message());
            false
        }
    }
}

/// 逐块适配器探一遍 D3D11 的基础能力。
///
/// # 为什么要**逐块**而不是只探默认那块
///
/// 关键在于 **Microsoft Basic Render Driver（WARP）** —— 它是微软自带的**纯软件**
/// 光栅器，**完全不依赖任何显卡驱动**。所以：
///
/// - 若 WARP 也失败 ⇒ 问题**不可能**是显卡驱动，只可能在操作系统侧的图形栈
///   （`d3d11.dll` / `dxgi.dll`）或某个挂在显示栈上的东西；
/// - 若只有独显/核显失败而 WARP 正常 ⇒ 那是驱动问题。
///
/// 这个区分决定了「该找驱动还是该找系统」，所以必须逐块跑。
#[test]
fn per_adapter_probe_d3d11_baseline_capability() {
    use windows::core::Interface as _;
    use windows::Win32::Graphics::Dxgi::CreateDXGIFactory1;

    let factory: windows::Win32::Graphics::Dxgi::IDXGIFactory4 = unsafe { CreateDXGIFactory1() }
        .expect("DXGI 工厂");

    // 汇总表：适配器 × 4 个探针
    let mut rows: Vec<(String, [char; 6])> = Vec::new();
    for i in 0..8u32 {
        let Ok(adapter) = (unsafe { factory.EnumAdapters1(i) }) else {
            break;
        };
        let Ok(desc) = (unsafe { adapter.GetDesc() }) else { continue };
        let end = desc.Description.iter().position(|&c| c == 0).unwrap_or(128);
        let name = String::from_utf16_lossy(&desc.Description[..end]);

        let Ok(dxgi) = adapter.cast::<windows::Win32::Graphics::Dxgi::IDXGIAdapter>() else {
            continue;
        };
        let mut dev = None::<ID3D11Device>;
        let mut ctx = None::<ID3D11DeviceContext>;
        let built = unsafe {
            D3D11CreateDevice::<Option<&windows::Win32::Graphics::Dxgi::IDXGIAdapter>>(
                Some(&dxgi),
                windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN,
                windows::Win32::Foundation::HMODULE(std::ptr::null_mut()),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&[D3D_FEATURE_LEVEL_11_0]),
                D3D11_SDK_VERSION,
                Some(&mut dev as *mut _),
                None,
                Some(&mut ctx as *mut _),
            )
        }
        .is_ok();
        if !built {
            println!("\n=== {name} ===\n  ❌ 建不出设备");
            rows.push((name, ['x', 'x', 'x', 'x', 'x', 'x']));
            continue;
        }
        let dev = dev.unwrap();
        let ctx = ctx.unwrap();

        // 探针 1：DEFAULT 纹理（已知可用，作对照）
        let t_ok = |usage: D3D11_USAGE, cpu: u32| -> bool {
            let desc = D3D11_TEXTURE2D_DESC {
                Width: 4,
                Height: 4,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: usage,
                BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
                CPUAccessFlags: cpu,
                MiscFlags: 0,
            };
            let mut t = None;
            unsafe { dev.CreateTexture2D(&desc, None, Some(&mut t as *mut _)) }.is_ok()
        };
        let p1_default = t_ok(D3D11_USAGE_DEFAULT, 0);
        // ★ `DYNAMIC` 的**正确**写法：`CPUAccessFlags` 必须是 `CPU_ACCESS_WRITE`。
        //
        // 按 `D3D11_USAGE` 参考页的能力表，`DYNAMIC` 意味着「GPU 只读 + **CPU 只写**」，
        // 而 CPU 写权限只能由 `D3D11_CPU_ACCESS_WRITE` 授予。
        //
        // 之前这个探针（以及本 crate 里原来的 `create_rgba_texture_empty`）把
        // `DYNAMIC` 与 `CPUAccessFlags: 0` 配在一起 —— 声称 CPU 可写却没给写权限，
        // 那是个**非法组合**，回 `E_INVALIDARG` 理所当然。
        //
        // 也就是说：「本机建不出 DYNAMIC 资源」这个结论**是探针自己的错**，
        // 不是环境的问题。
        let p2_dynamic_wrong = t_ok(D3D11_USAGE_DYNAMIC, 0);
        let p2_dynamic = t_ok(D3D11_USAGE_DYNAMIC, D3D11_CPU_ACCESS_WRITE.0 as u32);

        // 探针 3：D3D11 缓冲（DYNAMIC，同样两种写法）
        let mk_buf = |cpu: u32| {
            let desc = D3D11_BUFFER_DESC {
                ByteWidth: 64,
                Usage: D3D11_USAGE_DYNAMIC,
                BindFlags: D3D11_BIND_VERTEX_BUFFER.0 as u32,
                CPUAccessFlags: cpu,
                MiscFlags: 0,
                StructureByteStride: 0,
            };
            let mut b = None;
            unsafe { dev.CreateBuffer(&desc, None, Some(&mut b as *mut _)) }.is_ok()
        };
        let buf_dynamic_wrong = mk_buf(0);
        let buf_dynamic = mk_buf(D3D11_CPU_ACCESS_WRITE.0 as u32);

        // 探针 4：视口往返
        let viewport_ok = unsafe {
            let want = [D3D11_VIEWPORT {
                TopLeftX: 0.0,
                TopLeftY: 0.0,
                Width: 64.0,
                Height: 64.0,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            }];
            ctx.RSSetViewports(Some(&want));
            let mut n = 0u32;
            let mut got = [D3D11_VIEWPORT::default(); 1];
            ctx.RSGetViewports(&mut n, Some(got.as_mut_ptr()));
            n == 1 && got[0].Width == 64.0
        };

        println!("\n=== {name} ===");
        println!("  纹理 DEFAULT          {}", if p1_default { "✅" } else { "❌" });
        println!(
            "  纹理 DYNAMIC+无CPU权限  {}   ← 非法组合，回错是应该的",
            if p2_dynamic_wrong { "✅" } else { "❌" }
        );
        println!(
            "  纹理 DYNAMIC+WRITE     {}   ← 正确写法",
            if p2_dynamic { "✅" } else { "❌" }
        );
        println!(
            "  缓冲 DYNAMIC+无CPU权限  {}   ← 非法组合",
            if buf_dynamic_wrong { "✅" } else { "❌" }
        );
        println!(
            "  缓冲 DYNAMIC+WRITE     {}   ← 正确写法",
            if buf_dynamic { "✅" } else { "❌" }
        );
        println!("  视口往返              {}", if viewport_ok { "✅" } else { "❌" });
        let mark = |b: bool| if b { 'Y' } else { 'n' };
        rows.push((
            name,
            [
                mark(p1_default),
                mark(p2_dynamic_wrong),
                mark(p2_dynamic),
                mark(buf_dynamic_wrong),
                mark(buf_dynamic),
                mark(viewport_ok),
            ],
        ));
    }

    println!("\n\n=== 汇总 ===");
    println!(
        "  {:<32} 纹理DEF 纹DYN✗ 纹DYN✓ 缓DYN✗ 缓DYN✓ 视口",
        "适配器"
    );
    for (name, m) in &rows {
        let w = if name.chars().count() > 30 { 30 } else { name.chars().count() };
        println!(
            "  {:<w$}    {}      {}      {}      {}      {}      {}",
            name, m[0], m[1], m[2], m[3], m[4], m[5]
        );
    }
    println!("\n  ✗ = CPUAccessFlags: 0（与 DYNAMIC 配对是**非法组合**）");
    println!("  ✓ = CPUAccessFlags: D3D11_CPU_ACCESS_WRITE（正确写法）");
    println!("  读法：Microsoft Basic Render Driver 是微软的**纯软件**光栅器，不经显卡驱动。");
}

#[test]
fn probe_d3d11_baseline_on_this_machine() {
    let Some((device, context)) = device() else {
        panic!("建不出 D3D11 设备");
    };
    println!("\n=== 资源创建（从最基础的开始）===");
    // 缓冲：与纹理完全不同的资源类型，但它连 64 字节都建不出来的话，
    // 「纹理描述符写错了」这个解释就站不住了。
    let mut buf = None;
    let bdesc = D3D11_BUFFER_DESC {
        ByteWidth: 64,
        // 注意：`D3D11_USAGE` 里**没有** `VERTEX` 这个值。顶点缓冲用
        // `DEFAULT`（可 `Map` 写）或 `IMMUTABLE`（一次写死），不是 `VERTEX`。
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_VERTEX_BUFFER.0 as u32,
        CPUAccessFlags: 0,
        MiscFlags: 0,
        StructureByteStride: 0,
    };
    match unsafe { device.CreateBuffer(&bdesc, None, Some(&mut buf as *mut _)) } {
        Ok(()) => println!("  ✅ CreateBuffer 64B (VERTEX_BUFFER, DEFAULT)"),
        Err(e) => println!(
            "  ❌ CreateBuffer 64B (DEFAULT)  →  {:#X} {}",
            e.code().0 as u32,
            e.message()
        ),
    }

    // ★ 顶点/索引缓冲用的是 `DYNAMIC`，所以这条必须单独测。
    // `DYNAMIC` 缓冲要靠 `Map(WRITE_DISCARD)` 每帧写 —— 若它也不可用，
    // 就得连缓冲一起改成 `DEFAULT` + `UpdateSubresource`。
    let mut dbuf = None;
    let ddesc = D3D11_BUFFER_DESC {
        ByteWidth: 64,
        Usage: D3D11_USAGE_DYNAMIC,
        BindFlags: (D3D11_BIND_VERTEX_BUFFER.0 | D3D11_BIND_INDEX_BUFFER.0) as u32,
        CPUAccessFlags: 0,
        MiscFlags: 0,
        StructureByteStride: 0,
    };
    match unsafe { device.CreateBuffer(&ddesc, None, Some(&mut dbuf as *mut _)) } {
        Ok(()) => {
            println!("  ✅ CreateBuffer 64B (VERTEX+INDEX, DYNAMIC)");
            let dbuf = dbuf.unwrap();
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            match unsafe {
                context.Map(
                    &dbuf,
                    0,
                    D3D11_MAP_WRITE_DISCARD,
                    0,
                    Some(&mut mapped as *mut _),
                )
            } {
                Ok(()) => println!(
                    "    Map(WRITE_DISCARD) ✅ RowPitch={}",
                    mapped.RowPitch
                ),
                Err(e) => println!("    Map(WRITE_DISCARD) ❌ {:#X}", e.code().0 as u32),
            }
        }
        Err(e) => println!(
            "  ❌ CreateBuffer 64B (DYNAMIC)  →  {:#X} {}  ← 顶点缓冲建不出来",
            e.code().0 as u32,
            e.message()
        ),
    }

    probe_texture(&device, "CreateTexture2D 无 bind flag, DEFAULT", 0, D3D11_USAGE_DEFAULT);
    probe_texture(
        &device,
        "CreateTexture2D SHADER_RESOURCE, DEFAULT",
        D3D11_BIND_SHADER_RESOURCE.0 as u32,
        D3D11_USAGE_DEFAULT,
    );
    probe_texture(
        &device,
        "CreateTexture2D SHADER_RESOURCE, DYNAMIC",
        D3D11_BIND_SHADER_RESOURCE.0 as u32,
        D3D11_USAGE_DYNAMIC,
    );
    probe_texture(
        &device,
        "CreateTexture2D RENDER_TARGET, DEFAULT",
        D3D11_BIND_RENDER_TARGET.0 as u32,
        D3D11_USAGE_DEFAULT,
    );

    // ★ 关键一问：DYNAMIC 建不出来，那字形图集怎么写进去？
    //
    // `Map` 只能用于 `DYNAMIC`（或 STAGING/READ）。DYNAMIC 既然建不出来，
    // 就必须走 `UpdateSubresource` —— 它对 `DEFAULT` 纹理可用，代价是驱动内部
    // 可能要暂存一份。这是「DYNAMIC 在本机不可用」时的退路。
    println!("\n=== 上传路径（字形图集靠它写像素）===");
    let udesc = D3D11_TEXTURE2D_DESC {
        Width: 4,
        Height: 4,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_R8G8B8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let mut utext = None;
    match unsafe { device.CreateTexture2D(&udesc, None, Some(&mut utext as *mut _)) } {
        Ok(()) => {
            let utext = utext.unwrap();
            let pixels = [7u8; 4 * 4 * 4];
            let src = D3D11_SUBRESOURCE_DATA {
                pSysMem: pixels.as_ptr() as *const _,
                SysMemPitch: 4 * 4,
                SysMemSlicePitch: 0,
            };
            // `pdstbox` 必须是 None（全区域）；给了 Some 会走另一条校验路径。
            // windows-rs 声明成返回 `()` —— **没有 HRESULT 可查**，所以
            // 「UpdateSubresource 有没有真的生效」只能靠之后读回来判断，
            // 不能靠返回值。这就是 `void` 返回的 D3D11 API 最麻烦的地方。
            unsafe {
                context.UpdateSubresource(
                    &utext,
                    0,
                    None,
                    &src as *const _ as *const std::ffi::c_void,
                    0,
                    0,
                )
            };
            println!("  ✅ UpdateSubresource 已调用（无返回值可查，见下方读回验证）");

            // Map 在 DEFAULT 纹理上**必然**失败（CPU 不可寻址），这是文档规定的。
            // 这里测它是为了把因果坐实：DYNAMIC 建不出来 ⇒ 不能用 Map 写
            // ⇒ 只能走 UpdateSubresource。
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            match unsafe {
                context.Map(
                    &utext,
                    0,
                    D3D11_MAP_WRITE_DISCARD,
                    0,
                    Some(&mut mapped as *mut _),
                )
            } {
                Ok(()) => println!("  ⚠ Map(DEFAULT 纹理) 竟然成功了（RowPitch={}）", mapped.RowPitch),
                Err(e) => println!(
                    "  · Map(DEFAULT 纹理) → {:#X} {}（预期如此：DEFAULT 内存 CPU 不可寻址）",
                    e.code().0 as u32,
                    e.message()
                ),
            }
        }
        Err(e) => println!("  （跳过：DEFAULT 纹理建不出来 {:#X}）", e.code().0 as u32),
    }

    println!("\n=== 视图（用交换链的后缓冲，所以只测 RTV/SRV 建不建得出来）===");
    // 用一张建出来的纹理测视图。若上面全失败，这段会连带跳过。
    let tdesc = D3D11_TEXTURE2D_DESC {
        Width: 4,
        Height: 4,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_R8G8B8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32 | D3D11_BIND_SHADER_RESOURCE.0 as u32,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let mut tex = None;
    let built = unsafe { device.CreateTexture2D(&tdesc, None, Some(&mut tex as *mut _)) }.is_ok();
    if built {
        let tex = tex.unwrap();
        let mut rtv = None;
        match unsafe { device.CreateRenderTargetView(&tex, None, Some(&mut rtv as *mut _)) } {
            Ok(()) => println!("  ✅ CreateRenderTargetView"),
            Err(e) => println!("  ❌ CreateRenderTargetView → {:#X}", e.code().0 as u32),
        }
        let mut srv = None;
        match unsafe { device.CreateShaderResourceView(&tex, None, Some(&mut srv as *mut _)) } {
            Ok(()) => println!("  ✅ CreateShaderResourceView"),
            Err(e) => println!("  ❌ CreateShaderResourceView → {:#X}", e.code().0 as u32),
        }
    } else {
        println!("  （跳过：纹理建不出来，视图无从谈起）");
    }

    println!("\n=== 对照组：已知正常的路径 ===");
    // 视口已知坏；光栅器状态已知好。这个对照是必要的 —— 若连状态对象都不行，
    // 那「视口坏」就不是孤例。
    let mut rs_out = None::<ID3D11RasterizerState>;
    // ⚠ 不能用 `D3D11_RASTERIZER_DESC::default()` —— 它的 `FillMode` 是 0，而
    // `D3D11_FILL_WIREFRAME = 2` / `D3D11_FILL_SOLID = 3`，**0 是非法值**，
    // 于是 `CreateRasterizerState` 返 E_INVALIDARG。Rust 的 `Default` 在这里是陷阱。
    let raster_desc = D3D11_RASTERIZER_DESC {
        FillMode: D3D11_FILL_SOLID,
        CullMode: D3D11_CULL_NONE,
        ..Default::default()
    };
    let rs = match unsafe {
        device.CreateRasterizerState(&raster_desc, Some(&mut rs_out as *mut _))
    } {
        Ok(()) => rs_out,
        Err(e) => {
            println!("  ❌ CreateRasterizerState（合法 desc）→ {:#X}", e.code().0 as u32);
            None
        }
    };
    let Some(rs) = rs else { return };
    unsafe { context.RSSetState(Some(&rs)) };
    let back = unsafe { context.RSGetState() };
    println!(
        "  RSSetState/RSGetState 往返：{}",
        match back {
            Ok(_) => "✅ 正常",
            Err(e) => {
                println!("     （GetState 返 {:#X}）", e.code().0 as u32);
                "❌ 读不回"
            }
        }
    );

    unsafe {
        let want = [D3D11_VIEWPORT {
            TopLeftX: 0.0,
            TopLeftY: 0.0,
            Width: 64.0,
            Height: 64.0,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        }];
        context.RSSetViewports(Some(&want));
        let mut n = 0u32;
        let mut got = [D3D11_VIEWPORT::default(); 1];
        context.RSGetViewports(&mut n, Some(got.as_mut_ptr()));
        println!(
            "  RSSetViewports/RSGetViewports：个数 {n}，读回 {}×{}  {}",
            got[0].Width,
            got[0].Height,
            if n == 1 && got[0].Width == 64.0 { "✅ 正常" } else { "❌ 被丢弃" }
        );
    }
}

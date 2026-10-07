//! `UpdateSubresource` 扫缓冲大小 × 绑定标志 —— 找出崩溃阈值。
//!
//! # 起因
//!
//! 一次真实的 `Renderer::render` 里，两次 `UpdateSubresource` 走**完全相同**的代码：
//!
//! | 调用 | 缓冲字节数 | `BindFlags` | 源长度 | 结果 |
//! |---|---|---|---|---|
//! | 索引缓冲 | 3072 | `BIND_INDEX_BUFFER` | 3072（补齐） | ✅ |
//! | 顶点缓冲 | 65536 | `BIND_VERTEX_BUFFER` | 65536（补齐） | ❌ AV |
//!
//! 源指针合法、`D3D11_SUBRESOURCE_DATA` 合法、崩溃点由逐步打印锁定在
//! `UpdateSubresource` **内部**（调用返回前）。已排除：源越界读（补齐后仍崩）、
//! 「资源已绑定到管线」导致 contention（挪到 `IASetVertexBuffers` 之前仍崩）。
//!
//! 剩下两个变量：**大小**与**绑定标志**。这个测试把它们分开扫。
//!
//! # 怎么看结果
//!
//! 每次调用前打印一行 `N = <字节数> <标志>`，崩溃时最后一行就是触发崩溃的尺寸。
//! 因为 `stderr` 无缓冲，即使进程被 AV 打死，已打印的行仍会留在输出里 ——
//! 这正是能定位到崩溃点的原因。
#![cfg(windows)]

use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_FLAG, D3D11_BIND_INDEX_BUFFER, D3D11_BIND_VERTEX_BUFFER, D3D11_BUFFER_DESC,
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CPU_ACCESS_WRITE, D3D11_SDK_VERSION,
    D3D11_SUBRESOURCE_DATA, D3D11_USAGE, D3D11_USAGE_DEFAULT, D3D11_USAGE_DYNAMIC, ID3D11Buffer,
    ID3D11Device, ID3D11DeviceContext,
};
use windows::Win32::Graphics::Dxgi::IDXGIAdapter;

/// 从环境变量 `ZEPHYRA_PROBE_BYTES` 取字节数；缺省 65536。
fn probe_bytes() -> u32 {
    std::env::var("ZEPHYRA_PROBE_BYTES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(65536)
}

/// 从环境变量 `ZEPHYRA_PROBE_USAGE` 取 `default` / `dynamic`。
fn probe_usage() -> D3D11_USAGE {
    match std::env::var("ZEPHYRA_PROBE_USAGE").as_deref() {
        Ok("dynamic") => D3D11_USAGE_DYNAMIC,
        _ => D3D11_USAGE_DEFAULT,
    }
}

/// 从环境变量 `ZEPHYRA_PROBE_BIND` 取 `vb` / `ib` / `none`。
fn probe_bind() -> D3D11_BIND_FLAG {
    match std::env::var("ZEPHYRA_PROBE_BIND").as_deref() {
        Ok("vb") => D3D11_BIND_VERTEX_BUFFER,
        Ok("ib") => D3D11_BIND_INDEX_BUFFER,
        _ => D3D11_BIND_FLAG(0),
    }
}

/// 建设备 —— 与 `surface::Gpu::new` **同一条路**（`D3D_DRIVER_TYPE_HARDWARE` +
/// `BGRA_SUPPORT` + FL 11_0），只是只要 device + context，不建交换链。
///
/// 走默认适配器（第一个参数给 `None`）而不是 `EnumAdapters` 挑的 —— 与生产代码
/// 保持一致，否则「探针能过但生产崩」就成了第三种可能。
fn device_and_context() -> Option<(ID3D11Device, ID3D11DeviceContext)> {
    let mut device = None::<ID3D11Device>;
    let mut context = None::<ID3D11DeviceContext>;
    unsafe {
        windows::Win32::Graphics::Direct3D11::D3D11CreateDevice::<Option<&IDXGIAdapter>>(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE(std::ptr::null_mut()),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            Some(&[D3D_FEATURE_LEVEL_11_0]),
            D3D11_SDK_VERSION,
            Some(&mut device as *mut _),
            None,
            Some(&mut context as *mut _),
        )
    }
    .ok()?;
    Some((device?, context?))
}

/// 单次探测：建一个 `bytes` 大小的缓冲，用同样大小的源调 `UpdateSubresource`。
#[test]
#[ignore = "本测试按设计会在某些档位上崩进程（见模块文档）—— 结论已记录在文件里，\
             默认测试不该被它带着一起挂掉"]
fn update_subresource_单次探测() {
    let Some((device, ctx)) = device_and_context() else {
        eprintln!("跳过：建不出 D3D11 设备");
        return;
    };

    let bytes = probe_bytes();
    let usage = probe_usage();
    let bind = probe_bind();

    // `DYNAMIC` 时 CPUAccessFlags **必须**是 WRITE，否则 `CreateBuffer` 直接回
    // `E_INVALIDARG`（驱动的原话："A D3D11_USAGE_DYNAMIC Resource may only have the
    // D3D11_CPU_ACCESS_WRITE CPUAccessFlags set."）。
    //
    // 但注意：文档说 `UpdateSubresource` 的目标**不能**是 `DYNAMIC` 资源 ——
    // 「A resource cannot be used as a destination if: the resource is created with
    // immutable or dynamic usage.」所以 `dynamic` 这一档本该**失败**（安静地）。
    // 列在这里是为了确认它是安静失败，而不是崩。
    let cpu_access = if usage.0 == D3D11_USAGE_DYNAMIC.0 {
        D3D11_CPU_ACCESS_WRITE.0 as u32
    } else {
        0
    };

    let desc = D3D11_BUFFER_DESC {
        ByteWidth: bytes,
        Usage: usage,
        BindFlags: bind.0 as u32,
        CPUAccessFlags: cpu_access,
        MiscFlags: 0,
        StructureByteStride: 0,
    };
    let mut buf: Option<ID3D11Buffer> = None;
    unsafe { device.CreateBuffer(&desc, None, Some(&mut buf as *mut _)) }
        .expect("CreateBuffer 应当成功");
    let buf = buf.expect("CreateBuffer 成功时必给缓冲");

    // 源**恰好**与缓冲等长 —— 排除越界读这一变量。
    let mut src = vec![0xABu8; bytes as usize];
    let data = D3D11_SUBRESOURCE_DATA {
        pSysMem: src.as_mut_ptr().cast(),
        SysMemPitch: 0,
        SysMemSlicePitch: 0,
    };
    // 写入一个可辨认的模式：源若被读，就该在缓冲里看到它。
    for (i, b) in src.iter_mut().enumerate() {
        *b = (i % 251) as u8;
    }
    let first = src[0];

    eprintln!(
        "  → UpdateSubresource: bytes={bytes} usage={} bind={} 源首字节={first}",
        if usage.0 == D3D11_USAGE_DYNAMIC.0 { "DYNAMIC" } else { "DEFAULT" },
        bind.0,
    );
    unsafe {
        ctx.UpdateSubresource(
            &buf,
            0,
            None,
            &data as *const _ as *const std::ffi::c_void,
            0,
            0,
        );
    }
    eprintln!("  ✅ 返回了（没崩）");

    // ---- 对照：`Map(WRITE_DISCARD)` 能不能写进同一块缓冲 ----
    //
    // 只有 `DYNAMIC` 资源 CPU 可寻址，所以 `DEFAULT` 那一档必然被 `Map` 拒 ——
    // 那正是「`Map` 需要 `DYNAMIC`」这条约束的来源，也是当初驱动报的原话。
    let mut mapped =
        windows::Win32::Graphics::Direct3D11::D3D11_MAPPED_SUBRESOURCE::default();
    eprintln!("  → Map(WRITE_DISCARD) 对照");
    match unsafe {
        ctx.Map(
            &buf,
            0,
            windows::Win32::Graphics::Direct3D11::D3D11_MAP_WRITE_DISCARD,
            0,
            Some(&mut mapped as *mut _),
        )
    } {
        Ok(()) => {
            eprintln!("    ✅ Map 成功 RowPitch={}", mapped.RowPitch);
            let pitch = mapped.RowPitch as usize;
            let w = bytes as usize;
            if w > pitch {
                eprintln!("    ⚠ 缓冲装不下（{w} > RowPitch {pitch}）");
            } else {
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        src.as_ptr(),
                        mapped.pData.cast::<u8>(),
                        w.min(pitch),
                    );
                }
            }
            unsafe { ctx.Unmap(&buf, 0) };
            eprintln!("    ✅ Unmap 完成");
        }
        Err(e) => eprintln!(
            "    ❌ Map 失败 {:#X} {}",
            e.code().0 as u32,
            e.message()
        ),
    }
}

/// 从小到大扫一遍尺寸，每档 `Map(WRITE_DISCARD)` 一次。
///
/// # ⚠ 这个测试**不能**用来验证 `UpdateSubresource`
///
/// `UpdateSubresource` 在本机对 `DEFAULT` 缓冲**随机**崩。同一条命令、同一档
/// （4096 字节、`BIND_VERTEX_BUFFER`）连跑 10 次，崩了 3 次。
///
/// 随机性让这条故障格外难查：**任何单次实验都可能得出相反的结论**。我一度扫出
/// 「阈值是 8192，4096 正常」—— 那是运气；换个时间跑 4096 就崩。
///
/// 所以：
///
/// - 它不能用「跑通了」当通过判据 —— 单次通过说明不了任何事；
/// - 它也不能用来扫尺寸 —— 进程随时会死，看不到后面的档。
///
/// 逐档结论只能靠**外部**驱动，一次一档、多次重复：
///
/// ```text
/// $env:ZEPHYRA_PROBE_BYTES = "4096"; $env:ZEPHYRA_PROBE_USAGE = "default"
/// cargo test -p zephyra-winit --test update_probe 单次 -- --nocapture
/// ```
///
/// 而 `DYNAMIC` + `Map(WRITE_DISCARD)` 在 2^12..2^21 **全过**、各跑 8 次零崩溃 ——
/// 这就是渲染器的缓冲现在走这条路的原因，也是本测试能钉住的东西。
#[test]
fn update_subresource_扫尺寸() {
    let Some((device, ctx)) = device_and_context() else {
        eprintln!("跳过：建不出 D3D11 设备");
        return;
    };
    // 这里固定用 `DYNAMIC` —— 只有它能 `Map`，才有一条不会随机崩的路可扫。
    //
    // `BindFlags` 固定给 `BIND_VERTEX_BUFFER`：零（无绑标志）的缓冲在本机
    // `CreateBuffer` 直接回 `E_INVALIDARG` —— 它没有任何合法用途，D3D11 拒绝。
    let usage = D3D11_USAGE_DYNAMIC;
    let cpu_access = D3D11_CPU_ACCESS_WRITE.0 as u32;
    let bind = D3D11_BIND_VERTEX_BUFFER;
    let _ = probe_bind;

    let mut map_ok: Vec<u32> = Vec::new();

    for k in 12..21u32 {
        let bytes = 1u32 << k;
        let desc = D3D11_BUFFER_DESC {
            ByteWidth: bytes,
            Usage: usage,
            BindFlags: bind.0 as u32,
            CPUAccessFlags: cpu_access,
            MiscFlags: 0,
            StructureByteStride: 0,
        };
        let mut buf: Option<ID3D11Buffer> = None;
        unsafe { device.CreateBuffer(&desc, None, Some(&mut buf as *mut _)) }
            .unwrap_or_else(|e| panic!("建 {bytes} 字节的 DYNAMIC 缓冲失败: {e:?}"));
        let buf = buf.expect("必给缓冲");

        let mut src = vec![0u8; bytes as usize];
        for (i, b) in src.iter_mut().enumerate() {
            *b = (i % 251) as u8;
        }
        // 先打印，崩了才知道崩在哪一档。
        eprintln!("  → bytes={bytes} (2^{k}) usage=DYNAMIC bind={}", bind.0);

        // 只走 `Map`。`UpdateSubresource` 那条路在本机会**随机**打死进程，
        // 逐档结论得靠 `update_subresource_单次探测` 配合外部环境变量跑。
        let mut mapped =
            windows::Win32::Graphics::Direct3D11::D3D11_MAPPED_SUBRESOURCE::default();
        match unsafe {
            ctx.Map(
                &buf,
                0,
                windows::Win32::Graphics::Direct3D11::D3D11_MAP_WRITE_DISCARD,
                0,
                Some(&mut mapped as *mut _),
            )
        } {
            Ok(()) => {
                // `RowPitch` 对**缓冲**恒等于 `ByteWidth` —— 缓冲只有一行。
                // 若哪天不等了，说明这里不能当紧密排列用，连续拷就会写歪。
                assert_eq!(
                    mapped.RowPitch, bytes,
                    "缓冲的 RowPitch 应当等于 ByteWidth（只有一行，无行间填充）"
                );
                unsafe {
                    std::ptr::copy_nonoverlapping(src.as_ptr(), mapped.pData.cast::<u8>(), src.len());
                }
                map_ok.push(bytes);
                unsafe { ctx.Unmap(&buf, 0) };
                eprintln!("    ✅ Map(WRITE_DISCARD) OK RowPitch={}", mapped.RowPitch);
            }
            Err(e) => eprintln!("    ❌ Map {:#X}", e.code().0 as u32),
        }
    }
    eprintln!("  Map(WRITE_DISCARD) 通过的尺寸: {map_ok:?}");
    assert_eq!(
        map_ok.len(),
        9,
        "2^12..2^21 共 9 档。DYNAMIC + Map(WRITE_DISCARD) 必须全过 —— \
         渲染器每帧往 64KB 顶点缓冲和 3KB 索引缓冲写数据，靠的就是这条路"
    );
}
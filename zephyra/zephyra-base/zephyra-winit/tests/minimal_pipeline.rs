//! **最小管线**：只用自带字节码画一个全屏三角形，不碰顶点缓冲、不碰输入布局。
//!
//! # 它守住哪一层
//!
//! 渲染器出问题时，故障域可以切成两半：与**顶点数据**有关的那一半（顶点缓冲、
//! 输入布局、语义、偏移），与**顶点数据无关**的那一半（设备、交换链或离屏目标、
//! 视口、清屏、光栅化、回读）。这一格只测后面那一半 —— 因为它连顶点缓冲都不用，
//! 位置直接由 `SV_VertexID` 在着色器里造出来。
//!
//! 两者正交，所以它能回答一个别处回答不了的问题：某个改动弄坏了画面之后，
//! 「连最小管线都不出图」意味着问题在最底层，而不必去翻顶点布局。
//!
//! # 这层曾经被错误地判定为坏的
//!
//! 视图端口曾被记成「在本机设不进去」，依据是 `RSGetViewports` 读回个数 0。
//! 那是**读回代码**的错：`pNumViewports` 是 `[in, out]`，输入值表示数组长度，
//! 而当时传的是 0 —— 「你要 0 个」就得到 0 个。视口一直是好的。
//!
//! 教训留在 [`视图读回`] 的注释里：观测代码自身出错时，它制造的现象与它要排除的
//! 故障**完全同形**。所以这一格对视口做的是**断言**，不是打印。
#![cfg(windows)]

use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};

/// 离屏渲染目标的边长。
const SIZE: u32 = 64;

/// 最低覆盖比例。低于它就说明三角形没铺满视口。
const MIN_COVERAGE: f64 = 0.99;

/// 全屏三角形顶点着色器：**不声明任何输入**，位置由 `SV_VertexID` 造出来。
///
/// 编译命令（`shaders/minimal.hlsl`）：
///
/// ```text
/// fxc /T vs_4_0 /E VSMain shaders/minimal.hlsl /Fo shaders/minimal.vs.bin
/// fxc /T ps_4_0 /E PSMain shaders/minimal.hlsl /Fo shaders/minimal.ps.bin
/// ```
///
/// 刻意用 SM 4.0（比本项目那套的 5.0 低两档）：这样一旦它坏了，可以顺带排除
/// 「5.0 的某个特性有问题」，而不用再去猜版本差异。
const MINIMAL_VS: &[u8] = include_bytes!("../shaders/minimal.vs.bin");

/// 纯红像素着色器，无输入。
const MINIMAL_PS: &[u8] = include_bytes!("../shaders/minimal.ps.bin");

/// 清成纯蓝，于是「画出来的红色像素」与「没画到的蓝色像素」不会混淆。
///
/// ⚠ 两个颜色都**不透明**，因为 `D3D11_USAGE_DEFAULT` 的 RT 上混合状态是未定义的，
/// 一旦依赖 alpha 就等于给这个测试加了一个与被测层无关的变量。
const CLEAR_BG: [u8; 4] = [0, 0, 255, 255];

/// 建 `SIZE × SIZE` 的纹理。
unsafe fn make_texture(
    device: &ID3D11Device,
    usage: D3D11_USAGE,
    bind: u32,
    cpu_access: u32,
) -> ID3D11Texture2D {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: SIZE,
        Height: SIZE,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: usage,
        BindFlags: bind,
        CPUAccessFlags: cpu_access,
        MiscFlags: 0,
    };
    let mut tex = None::<ID3D11Texture2D>;
    device.CreateTexture2D(&desc, None, Some(&mut tex as *mut _)).expect("建纹理");
    tex.unwrap()
}

/// 把视口**读回来**，并返回 `(个数, 宽, 高)`。
///
/// # 输入值必须填 16
///
/// `RSGetViewports(UINT *pNumViewports, D3D11_VIEWPORT *pViewports)` 的第一个参数
/// 是 `[in, out]`：**输入**表示 `pViewports` 数组能容纳多少个，**输出**才是实际绑定的
/// 个数。官方原文：
///
/// > on input, specifies the number of viewports in the *pViewports* array;
/// > on output, the variable contains the actual number of viewports that are
/// > bound to the rasterizer stage.
///
/// 传 0 得到 0 是**正确行为**，不是驱动丢弃了状态。同一个坑 `RSGetScissorRects`
/// 也踩（Windows 8 之前还有 16 个槽的上限）。
///
/// 数组给 16 个而不是 1 个：万一有人把 `pNumViewports` 写成「实际个数」而不是
/// 「容量」，多给几个槽能让越界写表现为读回脏数据而不是内存破坏 —— 后者会变成
/// 一个与本测试无关的崩溃。
unsafe fn 视图读回(context: &ID3D11DeviceContext) -> (u32, f32, f32) {
    let mut count = 16u32;
    let mut buf = [D3D11_VIEWPORT::default(); 16];
    context.RSGetViewports(&mut count, Some(buf.as_mut_ptr()));
    (count, buf[0].Width, buf[0].Height)
}

/// 把渲染目标复制到 STAGING 并读回**紧密排列**的字节。
///
/// STAGING 的 `RowPitch` 通常**大于** `width × 4`，所以必须逐行裁掉行尾填充 ——
/// 直接 `copy` 整个 `pitch × height` 字节会把填充也算进来，整幅图错位。
unsafe fn read_back(gpu: &zephyra_winit::surface::Gpu, tex: &ID3D11Texture2D) -> Vec<u8> {
    let stg = make_texture(
        &gpu.device,
        D3D11_USAGE_STAGING,
        0,
        D3D11_CPU_ACCESS_READ.0 as u32,
    );
    gpu.context.CopyResource(&stg, tex);
    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    gpu.context
        .Map(&stg, 0, D3D11_MAP_READ, 0, Some(&mut mapped as *mut _))
        .expect("STAGING 纹理必须能用 Map(READ) 读");
    let pitch = mapped.RowPitch as usize;
    let mut raw = vec![0u8; pitch * SIZE as usize];
    std::ptr::copy_nonoverlapping(mapped.pData as *const u8, raw.as_mut_ptr(), raw.len());
    gpu.context.Unmap(&stg, 0);
    raw.chunks(pitch)
        .flat_map(|row| row[..SIZE as usize * 4].to_vec())
        .collect()
}

/// 不碰顶点缓冲的全屏三角形能铺满视口。
///
/// # 断言而不是打印
///
/// 这一版原本只 `println!` 覆盖率与几个像素值，靠人眼看输出。那样「回归了」不会
/// 让 `cargo test` 变红 —— 而本项目里这类故障的特征恰恰是**没有任何错误**。
/// 所以覆盖率、视口读回、设备状态三样都必须是断言。
#[test]
fn 最小管线能铺满视口() {
    let Ok(gpu) = zephyra_winit::surface::Gpu::new() else {
        panic!("建不出 D3D11 设备 —— 最小管线都建不出设备，后面的测试没有意义");
    };

    unsafe {
        let tex = make_texture(
            &gpu.device,
            D3D11_USAGE_DEFAULT,
            D3D11_BIND_RENDER_TARGET.0 as u32,
            0,
        );
        let mut rtv = None::<ID3D11RenderTargetView>;
        gpu.device
            .CreateRenderTargetView(&tex, None, Some(&mut rtv as *mut _))
            .expect("建 RTV");
        let rtv = rtv.unwrap();

        let mut vs = None::<ID3D11VertexShader>;
        gpu.device
            .CreateVertexShader(MINIMAL_VS, None, Some(&mut vs as *mut _))
            .expect("最小 VS");
        let vs = vs.unwrap();
        let mut ps = None::<ID3D11PixelShader>;
        gpu.device
            .CreatePixelShader(MINIMAL_PS, None, Some(&mut ps as *mut _))
            .expect("最小 PS");
        let ps = ps.unwrap();

        gpu.context
            .OMSetRenderTargets(Some(&[Some(rtv.clone())]), None);
        gpu.context.ClearRenderTargetView(&rtv, &[0.0, 0.0, 1.0, 1.0]);
        gpu.context.RSSetViewports(Some(&[D3D11_VIEWPORT {
            TopLeftX: 0.0,
            TopLeftY: 0.0,
            Width: SIZE as f32,
            Height: SIZE as f32,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        }]));
        gpu.context.VSSetShader(Some(&vs), None);
        gpu.context.PSSetShader(Some(&ps), None);
        // ⚠ 拓扑**必须**设 —— 漏掉它时 `Draw` 拿 `UNDEFINED` 当参数**不报错**，
        // 只是什么都不画。官方《Getting Started with the Input-Assembler Stage》
        // 把这一步列为绑完输入缓冲后的硬性要求。这正是渲染器曾经的 bug。
        gpu.context
            .IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
        gpu.context.Draw(3, 0);

        let (count, width, height) = 视图读回(&gpu.context);
        assert_eq!(
            (count, width, height),
            (1, SIZE as f32, SIZE as f32),
            "视口没绑上。注意读回的输入值必须是数组容量，不是实际个数"
        );
        assert_eq!(
            gpu.device.GetDeviceRemovedReason(),
            Ok(()),
            "设备已被移除"
        );

        let pixels = read_back(&gpu, &tex);
        let total = (SIZE * SIZE) as usize;
        let drawn = pixels
            .chunks_exact(4)
            .filter(|p| p != &CLEAR_BG)
            .count();
        assert!(
            drawn as f64 >= total as f64 * MIN_COVERAGE,
            "全屏三角形只覆盖了 {drawn}/{total} 个像素 —— \
             设备 / 视口 / RTV / 清屏 / 光栅化 / 回读这一层有问题，\
             与顶点数据无关（这一格压根没用顶点缓冲）"
        );
    }
}

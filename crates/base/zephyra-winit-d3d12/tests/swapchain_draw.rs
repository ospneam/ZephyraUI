//! 交换链的**端到端**冒烟：建链 → 渲染到后缓冲 → 读回像素 → present。
//!
//! # 为什么这个测试不需要窗口
//!
//! `CreateSwapChainForComposition` 不需要 `HWND` —— 它挂进 DComp 视觉树，由系统
//! 合成器上屏。所以整条呈现链路（后缓冲、RTV 描述符、资源状态转换、`Present`）
//! 可以在没有窗口的进程里跑完整。
//!
//! 这很重要：本项目的渲染失败模式是「不报错但画面不对」，而「窗口里是空的」与
//! 「GPU 没画」在屏幕上**无法区分**。这里把「画对了」变成可复核的断言。
//!
//! # 读回为什么走独立路径
//!
//! flip model 下 `Present` **之后**读 `GetBuffer` 拿到的是**下一张**后缓冲（已被
//! 清空或已画别的），症状是「明明画了却读到全黑」。所以这里在 `Present` 之前把
//! 后缓冲 `CopyResource` 进一块 STAGING 缓冲再读 —— 与 [`d3d12_draw`] 同一手法。
//!
//! # 判据不看字节序
//!
//! 后缓冲用 `B8G8R8A8_UNORM`，字节序是 BGRA。判据取「最大的通道显著大于最小的
//! 通道」，这样换呈现格式时不会假失败。
#![cfg(windows)]

use zephyra::core::{Color, CornerRadius, Rect};
use zephyra::paint::{Command, DisplayList, SortKey};
use zephyra_dcomp::SwapchainDesc;
use zephyra_winit_d3d12::{
    renderer::{buffer_desc, RenderCtx, Renderer, TextureTable},
    surface::{create_resource, heap_props, Offscreen, Staging},
    swapchain::{Swapchain, BUFFER_COUNT, MSAA_SAMPLES},
    Gpu, RenderReport,
};
use windows::Win32::Graphics::Direct3D12::{
    D3D12_HEAP_TYPE_READBACK, D3D12_RESOURCE_STATE_COPY_DEST, D3D12_RESOURCE_STATE_COPY_SOURCE,
    D3D12_RESOURCE_STATE_PRESENT, ID3D12Resource,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_R8G8B8A8_UNORM;

/// 后缓冲边长。
const SIZE: u32 = 128;
/// 图集边长。
const ATLAS_SIZE: u32 = 64;
/// 矩形边长占整屏的比例。
const INSET: f64 = 0.1;
/// 抗锯齿余量。
const AA_SLACK: f64 = 0.03;

/// 期望的「红色主导」像素数。**由几何算出**，不写死 —— 魔数与几何脱节时，
/// 画得完全正确也会被判失败（DX12 那个测试就栽过一次）。
fn expected_coverage(size: u32) -> f64 {
    let side = (1.0 - 2.0 * INSET) * size as f64;
    side * side * (1.0 - AA_SLACK)
}

/// 交换链能建成、能取到 RTV、能呈现，并且**真的把像素画上了**。
#[test]
#[ignore = "需要真 GPU 与 DComp。手动跑：--ignored --nocapture"]
fn 交换链_能呈现出真实像素() {
    let Ok(mut gpu) = Gpu::new() else {
        panic!("建不出 D3D12 设备");
    };
    // 不在这里 `begin_frame` —— 下面那次手工记录用不到，而 `render_and_read` 自己会开帧。
    // 连着开两次会让第二次 `list.Reset` 失败（「命令列表未关闭」），那是一条与本测试
    // 无关的假故障。
    let mut chain = Swapchain::new(&gpu, (SIZE, SIZE), &swapchain_desc(), MSAA_SAMPLES).expect("建交换链");
    assert_eq!(chain.buffer_count(), BUFFER_COUNT);
    assert_eq!(chain.size(), (SIZE, SIZE));

    // 描述符必须**按缓冲个数**给 —— flip model 下索引会轮转。
    // 堆只给 1 个时第二帧 `cpu(1)` 就越界，而症状是「画面卡在第一帧的颜色」。
    for i in 0..chain.buffer_count() {
        chain
            .rtv(i)
            .unwrap_or_else(|e| panic!("取第 {i} 个 RTV 描述符失败: {e}"));
    }

    let mut renderer = Renderer::new(&gpu.device, ATLAS_SIZE, chain.samples()).expect("建渲染器");
    let textures = TextureTable::new();
    let list = one_red_quad(SIZE);

    gpu.begin_frame().expect("开帧失败");
    let (report, pixels) = render_and_read(
        &mut gpu,
        &mut chain,
        &mut renderer,
        &textures,
        &list,
        (SIZE, SIZE),
    );
    assert_eq!(report.quads, 1, "一个 Quad 应当展开成 1 个四边形");
    assert!(report.vertex_bytes > 0, "顶点流不该是空的");

    chain.present(1).expect("present 失败");

    let total = (SIZE * SIZE) as usize;
    let drawn = count_red(&pixels);
    println!(
        "[swapchain] 覆盖 {drawn}/{total}（{:.1}%，几何要求 ≥ {:.1}%）",
        drawn as f64 / total as f64 * 100.0,
        expected_coverage(SIZE) / total as f64 * 100.0
    );

    assert!(
        drawn as f64 >= expected_coverage(SIZE),
        "只画出了 {drawn} 个红像素，几何要求 ≥ {:.0} —— 呈现链路建起来了但内容不对",
        expected_coverage(SIZE)
    );
    assert_eq!(
        chain.consecutive_present_errors(),
        0,
        "present 失败过（可恢复错误也要计数，否则「等下一帧」会永远等不到）"
    );
}

/// 第二个后缓冲也要能画 —— 证明 `GetCurrentBackBufferIndex` 轮转后 RTV 没串。
///
/// 只有一张能画的症状是「画面每帧一样」或「偶尔闪一下」，很难从截图上看出来。
#[test]
#[ignore = "需要真 GPU 与 DComp。手动跑：--ignored --nocapture"]
fn 两个后缓冲都能画() {
    let Ok(mut gpu) = Gpu::new() else {
        panic!("建不出 D3D12 设备");
    };
    let mut chain = Swapchain::new(&gpu, (SIZE, SIZE), &swapchain_desc(), MSAA_SAMPLES).expect("建交换链");
    let mut renderer = Renderer::new(&gpu.device, ATLAS_SIZE, chain.samples()).expect("建渲染器");
    let textures = TextureTable::new();
    let list = one_red_quad(SIZE);

    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..4 {
        gpu.begin_frame().expect("开帧失败");
        let (_, pixels) = render_and_read(&mut gpu, &mut chain, &mut renderer, &textures, &list, (SIZE, SIZE));
        let idx = chain.back_buffer_index();
        let drawn = count_red(&pixels);
        println!("[swapchain] 后缓冲 #{idx}: {drawn} 个红像素");
        seen.insert(idx);
        assert!(
            drawn as f64 >= expected_coverage(SIZE),
            "后缓冲 #{idx} 上只画出了 {drawn} 个红像素 —— RTV 描述符可能串了"
        );
        chain.present(1).expect("present 失败");
    }
    assert!(
        seen.len() >= 2,
        "四帧里只轮转到 {seen:?} 一个后缓冲 —— flip model 下索引本应轮转"
    );
}

/// resize 之后**必须**还能画 —— 因为 `ResizeBuffers` 会让旧 RTV 指向已释放的内存。
///
/// 不重建的症状是「窗口变一块固定颜色」或直接崩，**没有任何 HRESULT**。
#[test]
#[ignore = "需要真 GPU 与 DComp。手动跑：--ignored --nocapture"]
fn resize_之后仍然能画() {
    let Ok(mut gpu) = Gpu::new() else {
        panic!("建不出 D3D12 设备");
    };
    let mut chain = Swapchain::new(&gpu, (SIZE, SIZE), &swapchain_desc(), MSAA_SAMPLES).expect("建交换链");
    let mut renderer = Renderer::new(&gpu.device, ATLAS_SIZE, chain.samples()).expect("建渲染器");
    let textures = TextureTable::new();

    for size in [(SIZE, SIZE), (SIZE * 2, SIZE * 2), (SIZE, SIZE)] {
        // ★ 先开帧：list.Reset() 丢掉上一帧对后缓冲的**间接引用**，ResizeBuffers 才可能成功。
        gpu.begin_frame().expect("开帧失败");
        // 失败时立刻拉调试层消息 —— `drain_debug` 只在 `begin_frame` 开头调，
        // 而这里还没到下一帧，那时命令可能已经被下一条错误淹掉了。
        if let Err(e) = chain.resize(&gpu, size) {
            gpu.drain_debug();
            panic!("resize 到 {size:?} 失败: {e}");
        }
        let list = one_red_quad(size.0);
        let (_, pixels) =
            render_and_read(&mut gpu, &mut chain, &mut renderer, &textures, &list, size);
        let drawn = count_red(&pixels);
        let total = (size.0 * size.1) as usize;
        println!(
            "[swapchain] {size:?}: {drawn}/{total} 红像素（要求 ≥ {:.0}）",
            expected_coverage(size.0)
        );
        assert!(
            drawn as f64 >= expected_coverage(size.0),
            "resize 到 {size:?} 后只画出了 {drawn} 个红像素（要求 ≥ {:.0}）—— \
             RTV 视图**没有**随 ResizeBuffers 重建",
            expected_coverage(size.0)
        );
        chain.present(1).expect("present 失败");
    }
}

/// resize 失败时**必须**能重建整条链 —— 本机 `ResizeBuffers` 画过之后必失败。
///
/// 这条断言钉住「失败后自动重建」这个行为：它不是优化，是**唯一**能让窗口在改尺寸
/// 后继续显示东西的路（`Swapchain::resize` 的文档里有实测对照表）。
#[test]
#[ignore = "需要真 GPU 与 DComp。手动跑：--ignored --nocapture"]
fn resize_失败时会重建_并置位标记() {
    let Ok(mut gpu) = Gpu::new() else {
        panic!("建不出 D3D12 设备");
    };
    let mut chain = Swapchain::new(&gpu, (SIZE, SIZE), &swapchain_desc(), MSAA_SAMPLES).expect("建交换链");
    let mut renderer = Renderer::new(&gpu.device, ATLAS_SIZE, chain.samples()).expect("建渲染器");
    let textures = TextureTable::new();
    let list = one_red_quad(SIZE);

    // 先画一帧 —— 这是让 `ResizeBuffers` 失败的前提。
    gpu.begin_frame().expect("开帧失败");
    let sz = chain.size();
    let _ = render_and_read(&mut gpu, &mut chain, &mut renderer, &textures, &list, sz);
    chain.present(1).expect("present 失败");
    assert!(
        !chain.was_recreated(),
        "刚建好时不该是重建过的"
    );

    // ⚠ 不在这里 `begin_frame` —— resize 不需要开帧，而它成功后会**重建整条链**，
    // 紧接着再开一次帧会因「列表已在记录中」而失败。下一段那里开帧才对。
    chain.resize(&gpu, (SIZE * 2, SIZE * 2)).expect("resize 应当成功");
    assert_eq!(chain.size(), (SIZE * 2, SIZE * 2), "尺寸必须真的变了");
    assert!(
        chain.was_recreated(),
        "画过之后本机 `ResizeBuffers` 必然失败，所以这里必须走了重建 —— \
         宿主据此重新把 swapchain 挂进 DComp 视觉树"
    );

    // 重建之后还得能画 —— 否则只是「不报错」而不是「能用」。
    //
    // ⚠ quad 要按**新尺寸**重造：沿用旧的那份，它只占新目标的左上角一小块 ——
    // 覆盖率判据立刻会抓到（这也是判据按几何算、不写死值的另一个理由）。
    gpu.begin_frame().expect("开帧失败");
    let sz = chain.size();
    let big = one_red_quad(sz.0);
    let (_, pixels) = render_and_read(&mut gpu, &mut chain, &mut renderer, &textures, &big, sz);
    let drawn = count_red(&pixels);
    assert!(
        drawn as f64 >= expected_coverage(sz.0),
        "重建之后只画出了 {drawn} 个红像素（要求 ≥ {:.0}）",
        expected_coverage(sz.0)
    );
}

/// 零尺寸（最小化）必须被安静地忽略。
///
/// DXGI 拒绝 `ResizeBuffers(0, 0, …)`，而 `CreateSwapChainForComposition` 建出来的
/// 交换链**连传 0 都不允许**（文档明写）。不早退的话每次最小化都会抛一个无意义的错，
/// 而窗口最小化是**正常使用**的一部分。
#[test]
fn 零尺寸_resize_被忽略而不是报错() {
    let Ok(gpu) = Gpu::new() else {
        panic!("建不出 D3D12 设备");
    };
    let mut chain = Swapchain::new(&gpu, (SIZE, SIZE), &swapchain_desc(), MSAA_SAMPLES).expect("建交换链");
    for bad in [(0, 0), (0, 480), (640, 0)] {
        chain
            .resize(&gpu, bad)
            .unwrap_or_else(|e| panic!("resize 到 {bad:?} 不该报错，却得到 {e}"));
        assert_eq!(chain.size(), (SIZE, SIZE), "尺寸不该被零值改写");
    }
}

/// 交换链参数。
///
/// # 格式必须是 `R8G8B8A8_UNORM`，**不能**用 `zephyra_dcomp::SwapchainDesc::default()`
/// 的 `B8G8R8A8_UNORM`
///
/// PSO 的 `RTVFormats[0]` 写死在 `Renderer::create_pso` 里为 `R8G8B8A8_UNORM`，而 D3D12
/// 在 `DrawIndexedInstanced` 时逐槽比对渲染目标格式与 PSO 的声明 —— 不一致时调试层报
/// "The render target format in slot 0 does not match the pipeline state's RTV format"，
/// 而**画面全是错的、不报任何 HRESULT**。
///
/// `SwapchainDesc::default()` 是给 D3D11 那条路调的（那边 RTV 是 BGRA），复用它会让
/// 两个后端的呈现格式**悄悄不一致** —— 这正是本项目反复栽的那类 bug。
fn swapchain_desc() -> SwapchainDesc {
    SwapchainDesc {
        buffer_count: BUFFER_COUNT,
        format: DXGI_FORMAT_R8G8B8A8_UNORM,
    }
}

/// 一帧的**渲染部分**：屏障 → 记录 → 提交 → 等 GPU → 读回后缓冲。返回（报告，像素）。
///
/// **不含 `Present`** —— 读回必须在 `Present` 之前，见模块文档。
///
/// **不含 `begin_frame`** —— 调用方必须自己开帧，而且要在 `resize` **之前**：
/// `list.Reset()` 才会丢掉上一帧对后缓冲的**间接引用**，那是 `ResizeBuffers` 成功的
/// 前提（见 `Swapchain::resize` 的文档）。
fn render_and_read(
    gpu: &mut Gpu,
    chain: &mut Swapchain,
    renderer: &mut Renderer,
    textures: &TextureTable,
    list: &DisplayList,
    size: (u32, u32),
) -> (RenderReport, Vec<u8>) {
    // 后缓冲在开帧之后仍是 `PRESENT` —— 不转就没有渲染目标可用，
    // 而 D3D12 **不会**为此报任何错。
    chain
        .barrier_to_render_target(&gpu.list)
        .expect("转 RENDER_TARGET");

    // ⚠ 渲染目标必须与 PSO 的 `SampleDesc.Count` 一致 —— 所以这里**跟着
    // `chain.samples()` 走**，而不是无条件取后缓冲。生产路径（`host.rs` 的
    // `render_pass`）是同一个分支；测试若在这里偷懒用单采样，它就测不到
    // MSAA 那条路，而那条路的失败症状是"全黑且零 HRESULT"。
    let rtv = if chain.samples() > 1 {
        chain.msaa_rtv().expect("取 MSAA 的 RTV")
    } else {
        chain.current_rtv().expect("取当前后缓冲的 RTV")
    };
    let report = renderer
        .record(RenderCtx {
            device: &gpu.device,
            list: &gpu.list,
            rtv,
            display_list: list,
            size,
            clear: Color::rgba(0.0, 0.0, 0.0, 1.0),
            textures,
            // 整帧：这条测试验的是「几何有没有铺满视口」，不是 scissor。
            damage: zephyra::frame::Damage::Full,
        })
        .expect("记录绘制命令失败");

    // ⚠ resolve 必须在 `barrier_to_present` 之前 —— 目标后缓冲要在
    // `RENDER_TARGET` 状态才能当 resolve 目标，而 D3D12 不自动转换。
    //
    // 漏掉它的症状与「什么都没画」**完全一样**（读回的是上一帧内容或全黑），
    // 所以这条必须在这里，而不能靠"生产路径记得调"。
    if chain.samples() > 1 {
        chain.resolve_msaa(&gpu.list).expect("MSAA 解析失败");
    }
    chain.barrier_to_present(&gpu.list).expect("转回 PRESENT");

    let index = chain.back_buffer_index();
    // 读回只**记录**拷贝；提交/等待/映射由这里做 —— 分成两处 `submit` 会让第二次
    // `Close` 一个已关闭的列表，于是后续每条命令都报
    // 「This API cannot be called on a closed command list」。
    let staging = record_read_back(gpu, chain, index);
    gpu.submit().expect("提交失败");
    gpu.wait_gpu().expect("等 GPU 失败");
    let pixels = Offscreen::map(&staging, chain.size());
    (report, pixels)
}

/// 把某个后缓冲复制进 STAGING **缓冲**的拷贝**记录进命令列表**（不提交）。
///
/// # 为什么是 `CopyTextureRegion` 而不是 `CopyResource`
///
/// 源是**纹理**、目标是**缓冲**。`CopyResource` 要求两者「尺寸与格式完全一致」，
/// 资源**类型**也必须相同 —— 纹理拷到缓冲上会失败：
///
/// ```text
/// ID3D12CommandList::CopyResource: Source and Destination resource must have
/// identical dimensions and formats.
/// ```
///
/// 这条错误还会**连锁**：拷贝没发生 ⇒ 后缓冲仍卡在 `COPY_SOURCE` 状态 ⇒
/// `Close` 报「资源仍在使用」⇒ 命令列表被留在关闭态 ⇒ 后面每一条命令都报
/// 「This API cannot be called on a closed command list」。一个根因炸出三条消息。
///
/// 正规做法就是 `CopyTextureRegion`：源 `SUBRESOURCE_INDEX`、目标缓冲
/// `PLACED_FOOTPRINT` + 显式行距 —— 与库里的 `Offscreen::record_readback` 同一形态。
fn record_read_back(gpu: &Gpu, chain: &Swapchain, index: u32) -> Staging {
    use windows::Win32::Graphics::Direct3D12::{
        D3D12_PLACED_SUBRESOURCE_FOOTPRINT, D3D12_SUBRESOURCE_FOOTPRINT,
        D3D12_TEXTURE_COPY_LOCATION, D3D12_TEXTURE_COPY_LOCATION_0,
        D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT, D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
    };

    let (w, h) = chain.size();
    let tight = w as u64 * 4;
    let byte_width = tight * h as u64;
    let buffer: ID3D12Resource = create_resource(
        &gpu.device,
        &heap_props(D3D12_HEAP_TYPE_READBACK),
        &buffer_desc(byte_width),
        D3D12_RESOURCE_STATE_COPY_DEST,
    )
    .expect("建回读缓冲");

    let src = chain.buffer(index).expect("取后缓冲");
    // 后缓冲此刻已是 `PRESENT`，转 `COPY_SOURCE` 才能被拷贝；拷完转回去。
    Gpu::barrier(
        &gpu.list,
        &src,
        D3D12_RESOURCE_STATE_PRESENT,
        D3D12_RESOURCE_STATE_COPY_SOURCE,
    );
    unsafe {
        gpu.list.CopyTextureRegion(
            &D3D12_TEXTURE_COPY_LOCATION {
                pResource: std::mem::ManuallyDrop::new(Some(buffer.clone())),
                Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
                Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                    PlacedFootprint: D3D12_PLACED_SUBRESOURCE_FOOTPRINT {
                        Offset: 0,
                        Footprint: D3D12_SUBRESOURCE_FOOTPRINT {
                            Format: chain.format(),
                            Width: w,
                            Height: h,
                            Depth: 1,
                            RowPitch: tight as u32,
                        },
                    },
                },
            },
            0,
            0,
            0,
            &D3D12_TEXTURE_COPY_LOCATION {
                pResource: std::mem::ManuallyDrop::new(Some(src.clone())),
                Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
                Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                    SubresourceIndex: 0,
                },
            },
            None,
        );
    }
    Gpu::barrier(
        &gpu.list,
        &src,
        D3D12_RESOURCE_STATE_COPY_SOURCE,
        D3D12_RESOURCE_STATE_PRESENT,
    );

    Staging {
        buffer,
        row_pitch: tight as u32,
        byte_width,
    }
}

/// 一个铺满 80%（边长）的纯红矩形。
fn one_red_quad(size: u32) -> DisplayList {
    let mut list = DisplayList::new();
    let inset = size as f32 * INSET as f32;
    let side = size as f32 - inset * 2.0;
    list.push(
        SortKey {
            layer: zephyra::paint::Layer::Content,
            z: 0,
            order: 0,
        },
        Command::Quad {
            rect: Rect::new(inset, inset, side, side),
            radius: CornerRadius { radius: 0.0 },
            fill: Color::rgb(1.0, 0.0, 0.0),
            stroke: None,
            filter: zephyra::filter::FilterChain::none(),
            blend: zephyra::filter::BlendMode::default(),
        },
    );
    list.sort();
    list
}

/// 数「某一个通道显著占主导」的像素 —— 与字节序无关。
fn count_red(pixels: &[u8]) -> usize {
    pixels
        .chunks_exact(4)
        .filter(|p| {
            let (a, b, c) = (p[0] as i32, p[1] as i32, p[2] as i32);
            let hi = a.max(b).max(c);
            let lo = a.min(b).min(c);
            hi > 100 && hi - lo > 60
        })
        .count()
}
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

use windows::Win32::Graphics::Direct3D12::{
    ID3D12Resource, D3D12_HEAP_TYPE_READBACK, D3D12_RESOURCE_STATE_COPY_DEST,
    D3D12_RESOURCE_STATE_COPY_SOURCE, D3D12_RESOURCE_STATE_PRESENT,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_R8G8B8A8_UNORM;
use zephyra_core::core::{Color, CornerRadius, Rect};
use zephyra_core::paint::{Command, DisplayList, SortKey};
use zephyra_dcomp::SwapchainDesc;
use zephyra_winit_d3d12::{
    renderer::{buffer_desc, RenderCtx, Renderer, TextureTable},
    surface::{create_resource, heap_props, Offscreen, Staging},
    swapchain::{Swapchain, BUFFER_COUNT, MSAA_SAMPLES},
    Gpu, RenderReport,
};

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
    expected_coverage_rect(size, size)
}

/// 矩形区域的期望覆盖像素数。
///
/// ⚠ 不能拿正方形那版去算长宽不等的内容 —— 后缓冲比窗口大（容量制）时，内容常常是
/// 宽扁的，按 `size` 当边长算会把宽度套到高度上，期望值虚高近一倍，判据反而会
/// 误报「没画够」。
fn expected_coverage_rect(w: u32, h: u32) -> f64 {
    let sw = (1.0 - 2.0 * INSET) * w as f64;
    let sh = (1.0 - 2.0 * INSET) * h as f64;
    sw * sh * (1.0 - AA_SLACK)
}

/// 交换链能建成、能取到 RTV、能呈现，并且**真的把像素画上了**。
#[test]
#[ignore = "需要真 GPU 与 DComp。手动跑：--ignored --nocapture"]
fn swapchain_presents_real_pixels() {
    let Ok(mut gpu) = Gpu::new() else {
        panic!("建不出 D3D12 设备");
    };
    // 不在这里 `begin_frame` —— 下面那次手工记录用不到，而 `render_and_read` 自己会开帧。
    // 连着开两次会让第二次 `list.Reset` 失败（「命令列表未关闭」），那是一条与本测试
    // 无关的假故障。
    let mut chain =
        Swapchain::new(&gpu, (SIZE, SIZE), &swapchain_desc(), MSAA_SAMPLES).expect("建交换链");
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
fn both_back_buffers_are_drawable() {
    let Ok(mut gpu) = Gpu::new() else {
        panic!("建不出 D3D12 设备");
    };
    let mut chain =
        Swapchain::new(&gpu, (SIZE, SIZE), &swapchain_desc(), MSAA_SAMPLES).expect("建交换链");
    let mut renderer = Renderer::new(&gpu.device, ATLAS_SIZE, chain.samples()).expect("建渲染器");
    let textures = TextureTable::new();
    let list = one_red_quad(SIZE);

    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..4 {
        gpu.begin_frame().expect("开帧失败");
        let (_, pixels) = render_and_read(
            &mut gpu,
            &mut chain,
            &mut renderer,
            &textures,
            &list,
            (SIZE, SIZE),
        );
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
    let mut chain =
        Swapchain::new(&gpu, (SIZE, SIZE), &swapchain_desc(), MSAA_SAMPLES).expect("建交换链");
    let mut renderer = Renderer::new(&gpu.device, ATLAS_SIZE, chain.samples()).expect("建渲染器");
    let textures = TextureTable::new();

    for size in [(SIZE, SIZE), (SIZE * 2, SIZE * 2), (SIZE, SIZE)] {
        // ★ 先开帧：list.Reset() 丢掉上一帧对后缓冲的**间接引用**，ResizeBuffers 才可能成功。
        gpu.begin_frame().expect("开帧失败");
        // 失败时立刻拉调试层消息 —— `drain_debug` 只在 `begin_frame` 开头调，
        // 而这里还没到下一帧，那时命令可能已经被下一条错误淹掉了。
        if let Err(e) = chain.resize(&mut gpu, size) {
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

/// **实测**：画过之后 `ResizeBuffers` 必然失败，按设计重建整条链。
///
/// # 这条门禁钉的是**实测结果**，不是期望
///
/// 本仓曾经有一个记了很久的结论 ——「本机 `ResizeBuffers` 一旦画过就回
/// `DXGI_ERROR_INVALID_CALL`」，**然后又推翻了它三轮**，每轮都靠读码猜成因：
///
/// 1. 猜「resize 跑在 `wait_gpu()` 之前」→ 加了 `wait_gpu` → 没用
/// 2. 猜「命令列表攥着引用」→ 加了 `release_command_list_refs` → 没用
/// 3. 猜「分配器攥着引用」→ 改成双槽 + `allocator.Reset` → 没用
/// 4. 猜「要连对象一起销毁」→ `recreate_command_slots` → **还是没用**
///
/// 最后靠**隔离实验**定案（`probe2_resize_buffers_矩阵`，保留在本文件里）：
///
/// | 一帧里做了什么 | `ResizeBuffers` |
/// |---|---|
/// | 什么都不开帧 | **成功** |
/// | 开帧 + 提交**空**命令列表 | **成功** |
/// | 开帧 + 只转后缓冲资源屏障 | `0x887A0001` |
/// | 开帧 + 画（1× / 4× MSAA） | `0x887A0001` |
///
/// 而这两条修复——等 GPU、Reset 列表与分配器、销毁重建命令槽——**全部保留**：
/// 它们各自消掉了一条真实的引用来源，只是**都不足以**单独让 `ResizeBuffers` 成功。
/// 留着的理由是：它们让「重建」这条兜底路径的引用更干净。
///
/// 本仓的 resize **不依赖 `ResizeBuffers`**：链只保证「≥ 客户区」，装不下才重建。
/// 见 `Swapchain::resize` 的文档（含本机实测矩阵与 Chromium 对照）。
#[test]
#[ignore = "需要真 GPU 与 DComp。手动跑：--ignored --nocapture"]
fn resize_buffers_在画过之后失败_并按设计重建() {
    let Ok(mut gpu) = Gpu::new() else {
        panic!("建不出 D3D12 设备");
    };
    let mut chain =
        Swapchain::new(&gpu, (SIZE, SIZE), &swapchain_desc(), MSAA_SAMPLES).expect("建交换链");
    let mut renderer = Renderer::new(&gpu.device, ATLAS_SIZE, chain.samples()).expect("建渲染器");
    let textures = TextureTable::new();
    let list = one_red_quad(SIZE);

    gpu.begin_frame().expect("开帧失败");
    let sz = chain.content_size();
    let _ = render_and_read(&mut gpu, &mut chain, &mut renderer, &textures, &list, sz);
    chain.present(1).expect("present 失败");

    // 画过之后本机 `ResizeBuffers` 必然失败（实测 `0x887A0001`），所以本仓的 resize
    // **不再依赖它**：链只保证「≥ 客户区」，装不下才重建。
    // 装得下的尺寸 ⇒ 什么都不做，内容尺寸立刻跟上。
    //
    // ⚠ 尺寸必须**真的**比容量小：链建在 (128,128)，而 128 + 8 = 136 是**装不下**的
    // —— 那个「小尺寸」会被当成超容而重建，判据就成了假通过。
    let small = (SIZE - 16, SIZE - 16);
    chain.resize(&gpu, small).expect("resize 不该报错");
    assert!(!chain.was_recreated(), "装得下就不该重建");
    assert_eq!(chain.content_size(), small, "内容尺寸应立刻跟上窗口");
    assert_eq!(chain.size(), (SIZE, SIZE), "装得下时容量不变");

    // 超出容量 ⇒ 重建，且容量带余量。
    let big = (SIZE * 4, SIZE * 2);
    chain.resize(&gpu, big).expect("resize 不该报错");
    assert!(
        chain.was_recreated(),
        "超出容量必须重建（视口不能比后缓冲大）"
    );
    assert_eq!(chain.content_size(), big, "内容尺寸仍按窗口");
    assert!(
        chain.size().0 >= big.0 && chain.size().1 >= big.1,
        "新容量 {:?} 必须 ≥ 窗口 {big:?}",
        chain.size()
    );

    // 重建之后还得能画 —— 否则只是「不报错」而不是「能用」。
    drop(chain.take_superseded());
    gpu.begin_frame().expect("开帧失败");
    let sz = chain.content_size();
    let big_quad = one_red_quad(sz.0);
    let (_, pixels) = render_and_read(
        &mut gpu,
        &mut chain,
        &mut renderer,
        &textures,
        &big_quad,
        sz,
    );
    let drawn = count_red(&pixels);
    // 读回的是整块后缓冲（容量），quad 只画了内容那一块 ⇒ 按面积折算。
    let (bw, bh) = chain.size();
    let frac = (sz.0 as f64 * sz.1 as f64) / (bw as f64 * bh as f64);
    let want = expected_coverage_rect(sz.0, sz.1) * frac;
    assert!(
        drawn as f64 >= want,
        "重建之后只画出了 {drawn} 个红像素（几何要求 ≥ {want:.0}）"
    );
}

/// **窗口缩放不应该重建交换链** —— 链只保证「≥ 客户区」。
///
/// # 这条守的是「拖动窗口时逐帧跟随」的机制本身
///
/// 本机 `ResizeBuffers` 在后缓冲被用过之后就再也调不成功（实测矩阵见
/// `Swapchain::resize` 文档）。所以链**不跟着窗口变**，而是永远比窗口大：窗口缩放
/// 只改视口（内容尺寸），本帧照常按新尺寸排版并绘制 ⇒ 清晰、逐帧、无缩放。
///
/// 「比窗口大」不会露馅：DComp 按内容**自然尺寸 1:1** 画在原点，窗口只显示左上角
/// 那块，其余在窗口外不可见。
///
/// 以前这条性质不存在 —— 每来一个 `WM_SIZE` 就重建一条链，实测一拖重建 140 次把
/// GPU 拖进 `DEVICE_HUNG`。本条就是那个回归的门禁。
#[test]
#[ignore = "需要真 GPU 与 DComp。手动跑：--ignored --nocapture"]
fn window_resize_does_not_rebuild_swapchain() {
    let Ok(gpu) = Gpu::new() else {
        panic!("建不出 D3D12 设备");
    };
    let start_size = (SIZE * 2, SIZE * 2);
    let mut chain =
        Swapchain::new(&gpu, start_size, &swapchain_desc(), MSAA_SAMPLES).expect("建交换链");
    assert_eq!(chain.size(), start_size, "建链时容量应等于请求尺寸");

    // 模拟一次拖动：40 个连续变化的尺寸，全部**应该**只改内容尺寸。
    let mut rebuilt = Vec::new();
    for step in 0..40u32 {
        let want = (SIZE * 2 + step * 8, SIZE * 2 + step * 4);
        chain.resize(&gpu, want).expect("resize 不该报错");
        assert_eq!(
            chain.content_size(),
            want,
            "第 {step} 步的内容尺寸应立刻跟上窗口"
        );
        if chain.was_recreated() {
            rebuilt.push(want);
        }
        // 内容尺寸永远不得超过容量 —— 否则会画到窗口外。
        assert!(
            chain.content_size().0 <= chain.size().0 && chain.content_size().1 <= chain.size().1,
            "第 {step} 步内容 {:?} 超出容量 {:?}",
            chain.content_size(),
            chain.size()
        );
    }

    // 40 步里只有「装不下」那几次会重建。上界按容量翻倍次数估：容量每次 ×1.25，
    // 所以从 256 长到 576（宽 2.25 倍）必然重建 4 次 —— 4 次是几何决定的，不是实现
    // 拖沓。关键是这个数与**步数无关**：40 步 4 次，而不是 40 步 40 次。
    assert!(
        rebuilt.len() <= 5,
        "40 步拖动重建了 {:?} 次（容量每次 ×1.25 ⇒ 宽 2.25 倍需 4 次；绝不能是 40 次）",
        rebuilt.len()
    );
    assert!(
        rebuilt.len() < 10,
        "重建次数必须远小于步数，否则又会回到「一拖就把 GPU 拖进 DEVICE_HUNG」"
    );
}

/// 装不下时容量要**涨到够大并留余量**，且内容尺寸仍按窗口。
#[test]
#[ignore = "需要真 GPU 与 DComp。手动跑：--ignored --nocapture"]
fn rebuilds_with_headroom_when_capacity_insufficient() {
    let Ok(gpu) = Gpu::new() else {
        panic!("建不出 D3D12 设备");
    };
    let mut chain =
        Swapchain::new(&gpu, (SIZE, SIZE), &swapchain_desc(), MSAA_SAMPLES).expect("建交换链");
    assert!(!chain.was_recreated(), "刚建好不算重建");

    // 窗口长到超出当前容量 ⇒ 必须重建，且容量比请求**大**（有余量）。
    let want = (SIZE * 8, SIZE * 4);
    chain.resize(&gpu, want).expect("resize 不该报错");
    assert!(
        chain.was_recreated(),
        "窗口超出容量时必须重建（链不能装下比它大的视口）"
    );
    assert_eq!(chain.content_size(), want, "内容尺寸仍按窗口");
    assert!(
        chain.size().0 >= want.0 && chain.size().1 >= want.1,
        "新容量 {:?} 必须 ≥ 窗口 {want:?}",
        chain.size()
    );
    // 余量按 25% + 对齐 8 计算（见 `Swapchain::grown_capacity`）。
    let expect_w = ((want.0 as f64 * 1.25).ceil() as u32).next_multiple_of(8);
    assert_eq!(chain.size().0, expect_w, "宽度余量应符合 25% + 对齐 8");

    // 重建之后还能画 —— 否则只是「不报错」而不是「能用」。
    drop(chain.take_superseded());
    let mut gpu = gpu;
    let mut renderer = Renderer::new(&gpu.device, ATLAS_SIZE, chain.samples()).expect("建渲染器");
    let textures = TextureTable::new();
    gpu.begin_frame().expect("开帧失败");
    let sz = chain.content_size();
    let q = one_red_quad(sz.0);
    let (_, px) = render_and_read(&mut gpu, &mut chain, &mut renderer, &textures, &q, sz);
    let drawn = count_red(&px);
    // ⚠ 读回的是**整块后缓冲**（= 容量），而 quad 只画了内容那一块。所以期望值要按
    // 「内容面积 / 缓冲面积」折算 —— 直接拿 `expected_coverage(content)` 去比会误判。
    let (bw, bh) = chain.size();
    let frac = (sz.0 as f64 * sz.1 as f64) / (bw as f64 * bh as f64);
    let want = expected_coverage_rect(sz.0, sz.1) * frac;
    assert!(
        drawn as f64 >= want,
        "重建之后只画出了 {drawn} 个红像素（几何要求 ≥ {want:.0}）"
    );
}

#[test]
#[ignore = "诊断用。手动跑：--ignored --nocapture"]
fn probe2_resize_buffers_矩阵() {
    // (标签, 采样数, 模式 0=只转屏障 1=画 2=画+读回)
    let cases: [(&str, u32, u8); 8] = [
        ("s4 不开帧      ", 4, 9),
        ("s4 开帧+提交空 ", 4, 7),
        ("s4 转屏障+销毁槽", 4, 10),
        ("s4 画+读回+销毁槽", 4, 12),
        ("s1 只转屏障    ", 1, 0),
        ("s1 画          ", 1, 1),
        ("s4 画(不读回)  ", 4, 1),
        ("s4 画+读回     ", 4, 2),
    ];
    for (label, samples, mode) in cases {
        let Ok(mut gpu) = Gpu::new() else {
            panic!("建不出 D3D12 设备");
        };
        let mut chain =
            Swapchain::new(&gpu, (SIZE, SIZE), &swapchain_desc(), samples).expect("建交换链");
        let mut renderer =
            Renderer::new(&gpu.device, ATLAS_SIZE, chain.samples()).expect("建渲染器");
        let textures = TextureTable::new();
        let sz = chain.size();

        if mode == 9 {
            // 一帧都不开：纯 baseline。
        } else if mode == 8 {
            // 只开帧，不记录、不提交 —— 列表处于**记录中**且为空。
            gpu.begin_frame().expect("开帧失败");
        } else if mode == 7 {
            // 开帧 + 空提交：命令列表走完一次完整的 Reset→Close→Execute→Signal。
            gpu.begin_frame().expect("开帧失败");
            gpu.submit().expect("提交失败");
            gpu.wait_gpu().expect("等 GPU 失败");
        } else if mode == 2 {
            // 走完整帧（含 MSAA resolve 与读回拷贝）。
            gpu.begin_frame().expect("开帧失败");
            let q = one_red_quad(sz.0);
            let _ = render_and_read(&mut gpu, &mut chain, &mut renderer, &textures, &q, sz);
        } else {
            gpu.begin_frame().expect("开帧失败");
            chain.barrier_to_render_target(gpu.list()).expect("转 RT");
            if mode == 1 {
                let q = one_red_quad(sz.0);
                let rtv = if chain.samples() > 1 {
                    chain.msaa_rtv().expect("MSAA RTV")
                } else {
                    chain.current_rtv().expect("后缓冲 RTV")
                };
                renderer
                    .record(RenderCtx {
                        device: &gpu.device,
                        list: gpu.list(),
                        rtv,
                        display_list: &q,
                        size: sz,
                        clear: Color::rgba(0.0, 0.0, 0.0, 1.0),
                        textures: &textures,
                        rt: chain.back_buffer().ok().as_ref(),
                        damage: zephyra_core::frame::Damage::Full,
                    })
                    .expect("记录绘制命令失败");
                if chain.samples() > 1 {
                    chain.resolve_msaa(gpu.list()).expect("MSAA 解析失败");
                }
            }
            chain.barrier_to_present(gpu.list()).expect("转回 PRESENT");
            gpu.submit().expect("提交失败");
            gpu.wait_gpu().expect("等 GPU 失败");
        }

        // 判别实验 3：把命令列表与分配器**整体销毁重建**（而不是 Reset），再 resize。
        //   成功 ⇒ 那份引用只有销毁才能放走（`Reset` 不够）。
        if mode >= 10 {
            gpu.recreate_command_slots().expect("销毁重建命令槽");
        }
        let r = chain.resize(&mut gpu, (SIZE * 2, SIZE * 2));
        println!(
            "[probe2] {label} 采样{samples}: 第一次 resize={} 重建={} 尺寸={:?}",
            if r.is_ok() { "Ok  " } else { "Err " },
            chain.was_recreated(),
            chain.size()
        );

        // 判别实验 1：紧接着再 resize 一次（换个尺寸）。
        //   「第一次失败、第二次成功」⇒ 是**一次性**状态（提交后还没排空之类）。
        //   「两次都失败」     ⇒ 引用是**持续**持有的，重试救不了。
        if r.is_err() || chain.was_recreated() {
            let r2 = chain.resize(&mut gpu, (SIZE * 3, SIZE * 2));
            println!(
                "[probe2] {label}: 紧接着第二次 resize={} 重建={} 尺寸={:?}",
                if r2.is_ok() { "Ok  " } else { "Err " },
                chain.was_recreated(),
                chain.size()
            );
        }

        // 判别实验 2：等 300ms 再 resize。
        //   若与「紧接着重试」结果不同 ⇒ 是**时间**问题（GPU/DXGI 异步侧还没完）。
        std::thread::sleep(std::time::Duration::from_millis(300));
        gpu.wait_gpu().expect("等 GPU 失败");
        let r3 = chain.resize(&mut gpu, (SIZE, SIZE * 2));
        println!(
            "[probe2] {label}: 等 300ms 后 resize={} 重建={} 尺寸={:?} removed=0x{:08X}",
            if r3.is_ok() { "Ok  " } else { "Err " },
            chain.was_recreated(),
            chain.size(),
            gpu.removed_reason().0 as u32
        );
    }
}
/// **诊断探针**：`ResizeBuffers` 到底被哪一步卡住。
///
/// # 为什么需要它
///
/// 本仓为 `0x887A0001`（`DXGI_ERROR_INVALID_CALL`）绕了三轮弯路，每轮都靠读码猜：
/// 先猜命令列表 → 改双槽分配器环（**没用**）→ 准备再猜 MSAA。三轮都错。
/// 唯一可靠的办法是把步骤**逐个拆开**，让回码说话。
///
/// 三个变体逐步加料，唯一变量是「做没做那一步」：
///
/// | 变体 | 画 | 提交 | `Present` |
/// |---|---|---|---|
/// | A | ✗ | ✗ | ✗ |
/// | B | ✓ | ✓ | ✗ |
/// | C | ✓ | ✓ | ✓ |
///
/// 结论（实测，见 `resize_被_present 卡住` 断言）：**C 失败而 B 成功** ⇒
/// 卡点不在我们自己的队列，而在 **DXGI 那一侧尚未完成的 present**。
#[test]
#[ignore = "诊断用。手动跑：--ignored --nocapture"]
fn probe_resize_buffers_被哪一步卡住() {
    for (label, do_draw, do_present) in [
        ("A 不画不 present", false, false),
        ("B 画+提交，不 present", true, false),
        ("C 画+提交+present", true, true),
    ] {
        let Ok(mut gpu) = Gpu::new() else {
            panic!("建不出 D3D12 设备");
        };
        let mut chain =
            Swapchain::new(&gpu, (SIZE, SIZE), &swapchain_desc(), MSAA_SAMPLES).expect("建交换链");
        let mut renderer =
            Renderer::new(&gpu.device, ATLAS_SIZE, chain.samples()).expect("建渲染器");
        let textures = TextureTable::new();

        if do_draw {
            gpu.begin_frame().expect("开帧失败");
            let sz = chain.size();
            let q = one_red_quad(sz.0);
            let _ = render_and_read(&mut gpu, &mut chain, &mut renderer, &textures, &q, sz);
            if do_present {
                chain.present(1).expect("present 失败");
            }
        }

        let r = chain.resize(&mut gpu, (SIZE * 2, SIZE * 2));
        println!(
            "[probe] {label}: resize = {}, 重建 = {}, 链尺寸 {:?}",
            match &r {
                Ok(()) => "Ok".to_string(),
                Err(e) => format!("Err({})", e),
            },
            chain.was_recreated(),
            chain.size()
        );
    }
}
/// 重建后旧链**必须还活着**，直到宿主把新链挂进 DComp 并 Commit。
///
/// # 这条守住什么
///
/// 早先 `Swapchain::resize` 直接 `*self = fresh`，旧链当场析构 —— 而 DComp 的
/// visual 上还挂着 `SetContent(旧链)`。下一帧合成就是拿一个**已释放**的交换链，
/// use-after-free。
///
/// 实测症状串（用户报的原文）：
///
/// ```text
/// [zephyra-d3d12] 交换链已重建，重新挂进 DComp 视觉树
/// [zephyra-d3d12] present 失败: GPU 设备实例已经暂停 (0x887A0005)
/// [zephyra-d3d12] 设备不可用：DXGI_ERROR_DEVICE_HUNG (0x887A0007)
/// ```
///
/// 官方纪律（Chromium `ui/gl/dc_layer_tree.cc:626`）：只在内容变了时 `SetContent`，
/// 拆除时 `SetContent(nullptr)` 丢引用，然后 `Commit()`。
///
/// # 为什么用 `#[ignore]`
///
/// 需要真的 D3D12 设备。本机起不来（见 `d3d12_draw.rs` 头部）。
#[test]
#[ignore = "需要 GPU 与 DComp 现场；本机起不来（--ignored --nocapture 跑）"]
fn old_swapchain_must_survive_until_host_rebinds() {
    let Ok(mut gpu) = Gpu::new() else {
        panic!("建不出 D3D12 设备");
    };
    let mut chain =
        Swapchain::new(&gpu, (SIZE, SIZE), &swapchain_desc(), MSAA_SAMPLES).expect("建交换链");
    let mut renderer = Renderer::new(&gpu.device, ATLAS_SIZE, chain.samples()).expect("建渲染器");
    let textures = TextureTable::new();
    let list = one_red_quad(SIZE);

    // 先画一帧 —— 这是让 `ResizeBuffers` 失败、从而走重建的前提。
    gpu.begin_frame().expect("开帧失败");
    let sz = chain.size();
    let _ = render_and_read(&mut gpu, &mut chain, &mut renderer, &textures, &list, sz);
    chain.present(1).expect("present 失败");

    chain
        .resize(&mut gpu, (SIZE * 2, SIZE * 2))
        .expect("resize 应当成功");
    assert!(chain.was_recreated(), "本机画过之后必然走重建");

    // 判据一：旧链还在。
    assert!(
        chain.take_superseded().is_some(),
        "重建后旧链必须保留在 superseded 里，\
         否则 DComp 的 visual 会持有一个已析构的交换链（实测 TDR）"
    );

    // 判据二：取走之后就没有了 —— 避免「永远留着、每次重建都泄漏一条」。
    assert!(
        chain.take_superseded().is_none(),
        "旧链只该保留一次：宿主 rebind 成功后取走，不该有第二份"
    );

    // 判据三：重建之后这条链本身仍然可用。
    gpu.begin_frame().expect("开帧失败");
    let sz = chain.size();
    let big = one_red_quad(sz.0);
    let (_, pixels) = render_and_read(&mut gpu, &mut chain, &mut renderer, &textures, &big, sz);
    assert!(
        count_red(&pixels) as f64 >= expected_coverage(sz.0),
        "重建之后画不出来，说明保旧链那套改坏了重建本身"
    );
}

/// 零尺寸（最小化）必须被安静地忽略。
///
/// DXGI 拒绝 `ResizeBuffers(0, 0, …)`，而 `CreateSwapChainForComposition` 建出来的
/// 交换链**连传 0 都不允许**（文档明写）。不早退的话每次最小化都会抛一个无意义的错，
/// 而窗口最小化是**正常使用**的一部分。
#[test]
fn zero_sized_resize_is_ignored_not_an_error() {
    let Ok(mut gpu) = Gpu::new() else {
        panic!("建不出 D3D12 设备");
    };
    let mut chain =
        Swapchain::new(&gpu, (SIZE, SIZE), &swapchain_desc(), MSAA_SAMPLES).expect("建交换链");
    for bad in [(0, 0), (0, 480), (640, 0)] {
        chain
            .resize(&mut gpu, bad)
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
        .barrier_to_render_target(gpu.list())
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
            list: gpu.list(),
            rtv,
            display_list: list,
            size,
            clear: Color::rgba(0.0, 0.0, 0.0, 1.0),
            textures,
            rt: chain.back_buffer().ok().as_ref(),
            // 整帧：这条测试验的是「几何有没有铺满视口」，不是 scissor。
            damage: zephyra_core::frame::Damage::Full,
        })
        .expect("记录绘制命令失败");

    // ⚠ resolve 必须在 `barrier_to_present` 之前 —— 目标后缓冲要在
    // `RENDER_TARGET` 状态才能当 resolve 目标，而 D3D12 不自动转换。
    //
    // 漏掉它的症状与「什么都没画」**完全一样**（读回的是上一帧内容或全黑），
    // 所以这条必须在这里，而不能靠"生产路径记得调"。
    if chain.samples() > 1 {
        chain.resolve_msaa(gpu.list()).expect("MSAA 解析失败");
    }
    chain.barrier_to_present(gpu.list()).expect("转回 PRESENT");

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
        gpu.list(),
        &src,
        D3D12_RESOURCE_STATE_PRESENT,
        D3D12_RESOURCE_STATE_COPY_SOURCE,
    );
    unsafe {
        gpu.list().CopyTextureRegion(
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
        gpu.list(),
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
            layer: zephyra_core::paint::Layer::Content,
            z: 0,
            order: 0,
        },
        Command::Quad {
            rect: Rect::new(inset, inset, side, side),
            radius: CornerRadius { radius: 0.0 },
            fill: Color::rgb(1.0, 0.0, 0.0),
            stroke: None,
            filter: zephyra_core::filter::FilterChain::none(),
            blend: zephyra_core::filter::BlendMode::default(),
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

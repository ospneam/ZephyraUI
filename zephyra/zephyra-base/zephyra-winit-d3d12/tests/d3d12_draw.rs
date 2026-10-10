//! D3D12 最小复现：**能不能画出像素**。
//!
//! # 这个测试要回答的唯一问题
//!
//! 「D3D12 后端能不能把四边形真的画到像素上。」
//!
//! 不回答别的。不测性能、不测交换链、不测文字排版。理由是本项目的历史教训：
//! D3D11 侧的排查之所以耗掉几十轮，正是因为把「画得出东西」与「窗口/呈现正常」
//! 缠在一起测 —— 画面空时，分不清是绘制没生效还是呈现没生效。
//!
//! 所以这里用**离屏纹理**做目标，完全不碰窗口系统。
//!
//! # ⚠ 当前状态：本机无法验证
//!
//! 本机 D3D12 的**资源创建整体不可用** —— 连「上传堆 64 字节缓冲」这种零歧义的
//! 调用都被 `E_INVALIDARG` 拒掉，4 块适配器全中，且手写 C 签名直调 vtable 得到**完全
//! 相同**的结果（排除 `windows-rs` ABI 缺陷）。完整证据链见 `tests/minimal.rs`。
//!
//! 对照：同机 DX11 可用。所以 GPU 本身是好的，缺的只是 D3D12 的资源创建能力。
//!
//! 因此本测试标 `#[ignore]`。在能用的机器上跑：
//!
//! ```powershell
//! cargo test -p zephyra-winit-d3d12 -- --ignored --nocapture
//! ```
//!
//! 若 `tests/minimal.rs` 里的 `d3d12_最小资源能不能建出来` 通过、而本测试不通过，
//! 那才是这个 crate 的问题，可以从本测试的失败信息继续查。
//!
//! # 判据：清黑、数非黑
//!
//! 这个措辞是刻意的。D3D11 那边曾经写过一版「清屏成绿色、数绿色像素数覆盖率」的
//! 判据 —— 那是**恒真式**：清屏本身就让绿色像素等于全屏，测试永远通过。基于它得出的
//! 「问题在 `Renderer::render`」的结论完全错，浪费了几十轮。
//!
//! 清成黑色、数「不是黑的」像素，才真的在问「有没有东西被画上去」。
//!
//! # 每一步都要能独立失败
//!
//! 判据里刻意留了一串**逐级收紧**的断言，从最外层到最内层：
//!
//! 1. 设备能建起来，且**未被移除**
//! 2. 离屏纹理能建起来（验证堆/描述符/格式这层）
//! 3. 渲染器与 PSO 能建起来（验证根签名/字节码/输入布局这层）
//! 4. 命令列表能记录并提交（验证「记录 ≠ 执行」这层）
//! 5. **回读到的像素不是全黑**（这才是真的"画出来了"）
//!
//! 前四步任何一步失败都直接 `panic!` 并说清是哪一步 —— 这样失败时第一眼就能知道
//! 问题在哪一层，而不必去猜「第 0 个像素是不是真的没画」。

use zephyra_core::core::Color;
use zephyra_core::paint::{Command, DisplayList};

use zephyra_winit_d3d12::{Gpu, Offscreen, Renderer, TextureTable};

/// 离屏目标边长。256 足够看清形状，又小到回读快。
const SIZE: u32 = 256;

/// 圆角矩形**按面积**能占整屏的比例。
///
/// ⚠ 这个阈值必须与几何**算出来**，不能凭「取个 0.9 吧」。
///
/// 矩形边长是屏幕的 90%（两侧各 inset 5%），所以面积是 `0.9² = 81%`，不是 90%。
/// 原先写 `MIN_COVERAGE = 0.9` 且拿它乘**整屏**像素数，于是阈值比几何实际能给的
/// 多出 11% —— 画得完全正确也过不了（实测 80.1% 就被断言拦下）。
///
/// 与其用一个和几何脱节的魔数，不如让判据自己从 [`INSET`] 算出来：这样改几何时
/// 判据自动跟着变，而不会出现「改了几何忘了改阈值」。
const INSET: f64 = 0.05;
/// 抗锯齿会在边缘吃掉少量像素，所以放宽一点点。
const AA_SLACK: f64 = 0.02;

/// 期望的非背景像素数（按矩形几何算，再减去抗锯齿的余量）。
///
/// ⚠ `side * side` **已经是像素数**了 —— 别再乘 `SIZE * SIZE`。第一版这么写过，
/// 于是期望值变成 34 亿，永远不可能达到。
fn expected_coverage() -> f64 {
    let side = (1.0 - 2.0 * INSET) * SIZE as f64;
    side * side * (1.0 - AA_SLACK)
}

/// 图集边长。图集内容在这个测试里不重要（矩形分支不采样它），但纹理必须存在。
const ATLAS_SIZE: u32 = 64;

/// D3D12 端到端：建管线 → 回放一个四边形 → 把渲染目标读回像素。
///
/// # 从「整体不可用」到通过，中间隔着六个我自己的 bug
///
/// 本测试原先被标成 `#[ignore]`，理由是「`CreateGraphicsPipelineState` 在本机整体
/// 不可用 —— 平凡着色器 + 空根签名这个最小 PSO 也回 `E_INVALIDARG`」。那个结论是错的。
///
/// 真正的阻塞项**每一个**都是我自己写的非法参数，而 `E_INVALIDARG` 不带任何说明，
/// 于是每一个都曾被记成「环境问题」：
///
/// | # | 现象 | 根因 |
/// |---|---|---|
/// | 1 | `CreateCommittedResource` 全部 `E_INVALIDARG` | 缓冲的 `Layout` 写成了 `UNKNOWN`（必须 `ROW_MAJOR`） |
/// | 2 | 离屏渲染目标建不出来 | `texture_desc` 写死 `Flags = NONE`（缺 `ALLOW_RENDER_TARGET`） |
/// | 3 | `CreateGraphicsPipelineState` `E_INVALIDARG` | `RTVFormats` 8 个槽全填了 `R8G8B8A8_UNORM`；第 1..8 槽必须 `UNKNOWN` |
/// | 4 | 回读建不出来 | 在 `READBACK` 堆上建了**纹理**（该堆只能放缓冲） |
/// | 5 | `CopyTextureRegion` 报 `Type` 错 | 目标是缓冲却用了 `SUBRESOURCE_INDEX`（该配 `PLACED_FOOTPRINT`） |
/// | 6 | `Map` 之后 `STATUS_ACCESS_VIOLATION` | 把 `D3D12_RANGE::End` 当成行距，而本机并未改写它 |
///
/// 而「问不到原因」本身也是我的 bug：调试层在**建设备之后**才
/// `EnableDebugLayer`（文档要求之前），且消息队列从**全局** `ID3D12Debug` 上取
/// （它只在挂在**设备**上的队列里）；`drain` 又只给 32 字节缓冲，而带描述的消息要
/// 一百多字节，于是每条消息都被 `continue` 静默跳过。
///
/// **教训**：扫描能告诉你「哪些组合能过」，给不出「为什么」。正确顺序是**先让调试层
/// 能说话，再问驱动**。
#[test]
fn d3d12_能把四边形画到像素上() {
    // ---- 1. 设备 ----
    let Ok(mut gpu) = Gpu::new() else {
        panic!("建不出 D3D12 设备 —— 第 1 步就断了，后面的断言都无从谈起");
    };
    // ★ 必须开一帧才能记录命令。
    //
    // `Gpu::new` 按 D3D12 的规矩把命令列表**关掉**再交出来（`CreateCommandList` 交付时
    // 处于记录中状态，不关就没法在帧循环里复用）。所以正确顺序是
    // `new` → `begin_frame` → 记录 → `submit`。
    //
    // 少了 `begin_frame` 时，列表一直是关闭状态，`submit` 对它再 `Close` 一次 →
    // `E_INVALIDARG`（"Close 命令列表: 参数错误"）。那条消息完全不提生命周期。
    gpu.begin_frame().expect("开帧失败");
    // 设备是否已挂 —— **先看这个**。D3D12 里「某个 API 返回了别的错误」经常只是后果：
    // 设备一旦挂掉，之后所有 API 都会返回看起来无关的错误（`E_INVALIDARG` 是最常见的
    // 一种），追它们等于追影子。
    assert_eq!(
        gpu.removed_reason().0,
        0,
        "设备刚建好就被移除了（code {:#X}）：{:#X} = DXGI_ERROR_DEVICE_HUNG",
        gpu.removed_reason().0,
        0x887A0005u32
    );

    // ---- 2. 离屏目标 ----
    let target = Offscreen::new(&gpu.device, (SIZE, SIZE))
        .unwrap_or_else(|e| {
            // 调试层的消息是**排队**的，必须主动拉 —— 不拉就永远看不到。
            gpu.drain_debug();
            panic!("建不出离屏目标 —— 第 2 步（堆/描述符/RTV）断了：{e}")
        });

    // ---- 3. 渲染器 + PSO ----
    let mut renderer = // 采样数 1：本测试的渲染目标是**单采样**离屏纹理（`Offscreen::new`），
    // PSO 必须与它一致。走 MSAA 的那条路由 `swapchain_draw.rs` 覆盖。
    Renderer::new(&gpu.device, ATLAS_SIZE, 1).unwrap_or_else(|e| {
            // 调试层的消息是**排队**的，必须主动拉 —— 不拉就永远看不到。
            gpu.drain_debug();
            panic!(
                "建不出渲染器 —— 第 3 步（根签名/字节码/输入布局/PSO）断了：{e}\n\
                 设备移除原因 = {:#X}",
                gpu.removed_reason().0
            )
        });

    // ---- 画什么：一个铺满 90% 的不透明红色圆角矩形 ----
    //
    // 中心点会单独断言"精确等于纯红"：覆盖率能靠"边缘画了点东西"蒙混，中心点的精确值
    // 不能。
    let mut list = DisplayList::new();
    // 几何与判据共用 [`INSET`] —— 改一处就够，不会出现「改了形状忘了改阈值」。
    let inset = SIZE as f32 * INSET as f32;
    let side = SIZE as f32 - inset * 2.0;
    list.push(
        zephyra_core::paint::SortKey {
            layer: zephyra_core::paint::Layer::Content,
            z: 0,
            order: 0,
        },
        Command::Quad {
            rect: zephyra_core::core::Rect {
                x: inset,
                y: inset,
                width: side,
                height: side,
            },
            radius: zephyra_core::core::CornerRadius { radius: 24.0 },
            fill: Color::rgba(1.0, 0.0, 0.0, 1.0),
            stroke: None,
            filter: zephyra_core::filter::FilterChain::none(),
            blend: zephyra_core::filter::BlendMode::default(),
        },
    );

    // ---- 4. 记录 + 提交 ----
    //
    // D3D12 的命令是**记录**进列表、提交后才交给 GPU。所以「记录成功」与「GPU 真的
    // 执行了」是两件事，必须分成两步、各自可查。
    //
    // ⚠ 这里**不再**调 `begin_frame` —— 上面第 1 步已经调过一次。
    // `list.Reset` 的前置条件是列表处于「closed」，而它成功后把列表留在
    // 「recording」状态 —— 于是连着调第二次必然 `E_FAIL`
    // （"Reset fails because the command list was not closed"）。
    let textures = TextureTable::new();

    let report = renderer
        .record(zephyra_winit_d3d12::RenderCtx {
            device: &gpu.device,
            list: &gpu.list,
            rtv: target.rtv,
            display_list: &list,
            size: (SIZE, SIZE),
            // 清成**黑**。判据是「数非黑像素」，所以清屏色必须是黑的 —— 见文件头
            // 关于恒真式判据的说明。
            clear: Color::rgba(0.0, 0.0, 0.0, 1.0),
            textures: &textures,
            rt: None,
            damage: zephyra_core::frame::Damage::Full,
        })
        .expect("记录绘制命令失败");

    assert_eq!(
        report.quads, 1,
        "DisplayList 里有一个 Quad，展开后应当正好 1 个四边形 —— \
         实际 {} 个，说明 `quad::expand` 或 DisplayList 的语义变了",
        report.quads
    );
    assert!(
        report.vertex_bytes > 0,
        "一个四边形应当产生 4 个顶点（{} 字节），实际 0 字节",
        report.vertex_bytes
    );

    // 回读缓冲：回读要求资源处于可拷贝状态，所以把拷贝命令也记录进去。
    let staging = target.record_readback(&gpu.device, &gpu.list).unwrap_or_else(|e| {
        // 消息是**排队**的，不主动拉就永远看不到 —— 这是本项目在 D3D11 与 D3D12
        // 上各踩一次的坑。
        gpu.drain_debug();
        panic!("记录回读拷贝失败：{e:?}\n设备移除原因 = {:#X}", gpu.removed_reason().0)
    });

    gpu.submit().expect("提交命令列表失败");
    gpu.wait_gpu().expect("等 GPU 失败");

    // ---- 5. 像素判据 ----
    let pixels = Offscreen::map(&staging, (SIZE, SIZE));
    assert_eq!(
        pixels.len(),
        (SIZE * SIZE * 4) as usize,
        "回读到 {} 字节，应为 {} 字节 —— 回读缓冲的映射或行距处理有问题",
        pixels.len(),
        SIZE * SIZE * 4
    );

    let drawn = non_background(&pixels);
    let total = (SIZE * SIZE) as f64;
    let expect = expected_coverage();
    let hist = color_hist(&pixels);

    println!(
        "[d3d12] 覆盖 {drawn}/{total}（{:.1}%，几何要求 ≥ {:.1}%），颜色分布 {hist:?}",
        drawn as f64 / total * 100.0,
        expect / total * 100.0
    );

    assert!(
        drawn as f64 >= expect,
        "只画出了 {drawn} 个非黑像素，几何要求 ≥ {expect:.0}（整屏 {total:.0} 的 {:.1}%）—— \
         D3D12 下仍然是「不报错但什么都不画」。颜色分布：{hist:?}\n\
         \n\
         先查这三处（都导致「零面积 / 全被裁掉」且**零错误**）：\n\
          1. `RSSetViewports` —— D3D12 视口是命令列表方法，若没设或设成 0×0，零面积；\n\
          2. `RSSetScissorRects` —— D3D12 **没有** ScissorEnable 开关，裁剪永远生效，\
             默认矩形为空 → 全部图元被裁掉；\n\
          3. `SampleMask` —— 必须是全 1，否则所有图元被采样掩码丢弃。",
        expect / total * 100.0
    );

    // 中心点必须**精确**是纯红。这条比覆盖率更强：覆盖率能靠"别处画了点东西"蒙混，
    // 中心点的精确值不能。
    let cx = (SIZE / 2) as usize;
    let c = &pixels[(cx * SIZE as usize + cx) * 4..][..4];
    assert_eq!(
        c,
        &[255, 0, 0, 255],
        "中心像素是 {c:?}，应为纯红 [255,0,0,255] —— 覆盖率可能靠边缘蒙混，\
         中心点的精确值才说明整个矩形真的填上了"
    );
}

/// 数「不是纯黑」的像素数。
///
/// 与 D3D11 版同名函数逐字一致 —— 两个后端共用同一份 `quad.rs`，判据也该共用同一个
/// 定义。改这里就要改那边。
fn non_background(pixels: &[u8]) -> usize {
    pixels
        .chunks_exact(4)
        .filter(|p| !(p[0] == 0 && p[1] == 0 && p[2] == 0))
        .count()
}

/// 各颜色的像素直方图。失败时打出来，能一眼看出"实际画成了什么"。
fn color_hist(pixels: &[u8]) -> std::collections::BTreeMap<[u8; 4], usize> {
    let mut m: std::collections::BTreeMap<[u8; 4], usize> = Default::default();
    for p in pixels.chunks_exact(4) {
        *m.entry([p[0], p[1], p[2], p[3]]).or_default() += 1;
    }
    m
}
/// D3D12 的精简夹具：渲染一帧并回读像素。
///
/// # 为什么单独开一个而不是往那个巨型测试里塞
///
/// `d3d12_draw.rs` 原本只有一个测试，自己管帧生命周期 / MSAA 目标 / PSO / 覆盖率
/// 阈值，是为"一次端到端冒烟"设计的。为一条断言复制那整套不成比例，但**没有**
/// 像素级夹具就等于 D3D12 侧完全无法验证 —— 混合因子、clear 预乘这类东西错了
/// 只有像素说得清。
///
/// 所以抽出一个只做"渲染 + 回读"的最小函数，其余断言各自独立。
fn render_and_readback(
    list: &DisplayList,
    clear: Color,
) -> (Vec<u8>, zephyra_winit_d3d12::RenderReport) {
    let mut gpu = Gpu::new().expect("应当建得出 D3D12 设备");
    gpu.begin_frame().expect("应当能开帧");
    let target = Offscreen::new(&gpu.device, (SIZE, SIZE)).expect("应当建得出离屏目标");
    // 采样数必须与单采样离屏目标一致。
    let mut renderer = Renderer::new(&gpu.device, ATLAS_SIZE, 1).expect("应当建得出渲染器");
    let textures = TextureTable::new();

    let report = renderer
        .record(zephyra_winit_d3d12::RenderCtx {
            device: &gpu.device,
            list: &gpu.list,
            rtv: target.rtv,
            display_list: list,
            size: (SIZE, SIZE),
            clear,
            textures: &textures,
            // 可分离 / 非可分离混合模式要读 `dst`，把目标资源给进去。
            rt: Some(&target.texture),
            damage: zephyra_core::frame::Damage::Full,
        })
        .unwrap_or_else(|e| {
            // 失败时把调试层已存的消息全部倒出来 —— 否则只拿到一个
            // `E_INVALIDARG`，而**到底是哪个字段非法**只在消息队列里。
            gpu.drain_debug();
            panic!("记录绘制命令失败: {e}");
        });

    // 回读要求资源可拷贝，所以拷贝命令也**记录**进同一份列表。
    let staging = target
        .record_readback(&gpu.device, &gpu.list)
        .expect("记录回读拷贝失败");
    gpu.submit().expect("提交失败");
    gpu.wait_gpu().expect("等 GPU 失败");
    // ⚠ **成功路径也必须抽干调试层队列**。
    //
    // D3D12 的验证层把问题**排队**报出来，而绝大多数错误既不返回失败码、也不让画面
    // 明显出错 —— 只在 `$env:ZEPHYRA_D3D12_DEBUG=1` 时才进队列。只在错误路径上抽，
    // 等于「命令列表合法」这个结论永远得不到证据：之前正是因为这里不抽，「整张图变成
    // 一个死值」查了很久，而真正的 `OMSetRenderTargets` 与状态屏障冲突就明明白白躺在
    // 队列里。
    gpu.drain_debug();
    (Offscreen::map(&staging, (SIZE, SIZE)), report)
}

/// D3D12 的离屏目标是 **`R8G8B8A8_UNORM`** —— 与 D3D11 的 `B8G8R8A8` **顺序相反**。
///
/// 两个后端各写一遍断言时这里最容易搞错：按 D3D11 的直觉去读 D3D12，红蓝会对调，
/// 于是"期望 128 的通道"读到的是另一个通道，断言看起来在管颜色、其实管错了。
fn at_rgba(px: &[u8], x: u32, y: u32) -> [u8; 4] {
    let i = (y as usize * SIZE as usize + x as usize) * 4;
    [px[i], px[i + 1], px[i + 2], px[i + 3]]
}

/// 半透明 clear 色必须**预乘**后写进目标 —— D3D12 侧的独立验证。
///
/// D3D11 那边已经有 `clear_color_is_premultiplied`，但两个后端的 clear 是**各写一份**
/// 的代码（`ClearRenderTargetView` 的签名与调用点都不同），所以必须各测一次。
///
/// 变异测试确认过：改掉 D3D12 的预乘而保留 D3D11 的，本测试会失败。
#[test]
fn clear_color_is_premultiplied_d3d12() {
    // 空列表：什么都不画，只剩 clear。
    let (px, _report) = render_and_readback(&DisplayList::new(), Color::rgba(1.0, 1.0, 1.0, 0.5));
    for (x, y) in [(0u32, 0u32), (SIZE / 2, SIZE / 2), (SIZE - 1, SIZE - 1)] {
        let p = at_rgba(&px, x, y);
        assert!(
            (p[0] as i32 - 128).abs() <= 2 && (p[1] as i32 - 128).abs() <= 2,
            "clear(1,1,1,0.5) 预乘后 rgb 约 128（未预乘会是 255），实得 {p:?} @({x},{y})"
        );
        assert!(
            (p[3] as i32 - 128).abs() <= 2,
            "alpha 应仍是 128，实得 {p:?} @({x},{y})"
        );
    }
}

/// 半透明图元叠不透明背景，结果必须是**预乘 over** 的结果 —— 钉住因子必须是 `ONE`。
///
/// 与 D3D11 的同名测试逐字对应（含判据推导）。两个后端的混合状态各建一份
/// （`CreateBlendState` vs PSO 的 `BlendState` 字段），所以也必须各测一次。
#[test]
fn translucent_quad_composites_in_premultiplied_space_d3d12() {
    let s = SIZE as f32;
    let mut list = DisplayList::new();
    list.push(
        zephyra_core::paint::SortKey {
            layer: zephyra_core::paint::Layer::Background,
            z: 0,
            order: 0,
        },
        Command::Quad {
            rect: zephyra_core::core::Rect::new(0.0, 0.0, s, s),
            radius: zephyra_core::core::CornerRadius { radius: 0.0 },
            fill: Color::rgb(0.0, 0.0, 1.0),
            stroke: None,
            filter: zephyra_core::filter::FilterChain::none(),
            blend: zephyra_core::filter::BlendMode::default(),
        },
    );
    list.push(
        zephyra_core::paint::SortKey {
            layer: zephyra_core::paint::Layer::Content,
            z: 0,
            order: 1,
        },
        Command::Quad {
            rect: zephyra_core::core::Rect::new(0.0, 0.0, s, s),
            radius: zephyra_core::core::CornerRadius { radius: 0.0 },
            fill: Color::rgba(1.0, 1.0, 1.0, 0.5),
            stroke: None,
            filter: zephyra_core::filter::FilterChain::none(),
            blend: zephyra_core::filter::BlendMode::default(),
        },
    );
    let (px, report) = render_and_readback(&list, Color::rgb(0.0, 0.0, 0.0));
    assert_eq!(report.quads, 2, "两个四边形都应当被展开");

    for (x, y) in [(8u32, 8u32), (SIZE / 2, SIZE / 2), (SIZE - 9, SIZE - 9)] {
        let p = at_rgba(&px, x, y);
        // r/g = 0.5 纯白压在纯蓝上 ⇒ 128；b = 0.5 + 0.5·1 = 1.0 ⇒ 255；a ⇒ 255
        assert!(
            (p[0] as i32 - 128).abs() <= 3 && (p[1] as i32 - 128).abs() <= 3,
            "r/g 应约 128（双重预乘会是 64），实得 {p:?} @({x},{y})"
        );
        assert!(
            (p[2] as i32 - 255).abs() <= 2,
            "b 应约 255（0.5 + 0.5·1），实得 {p:?} @({x},{y})"
        );
        assert!(
            (p[3] as i32 - 255).abs() <= 2,
            "a 应为 255（不透明底 + 半透明顶），实得 {p:?}"
        );
    }
}

/// D3D12 的混合状态**烤在 PSO 里**，所以"换混合模式"必然等于"换 PSO"。
///
/// # 为什么这条必须存在
///
/// D3D12 的 PSO 按 `(PS 变体, 混合模式, 采样数)` 三元组建并缓存。缓存的键漏了
/// `mode`（或漏了 `samples`）时，**不报错** —— 只是第二个模式复用了第一个的
/// PSO，画出来颜色不对。像素断言是唯一能拦住这类"键写错"的地方。
///
/// 背景中灰 + 源纯红，两个都走固定功能系数对（预乘空间）：
///
/// ```text
///   Screen   = 1−(1−b)(1−s) = (1, 0.5, 0.5) → 255,128,128
///   Modulate = Cs·Cd        = (0.5, 0, 0)   → 128,0,0
/// ```
/// 两者与默认 `SrcOver`（255,0,0）都不同，也互不相同。
///
/// ⚠ 别拿 `DstIn` 当判据：预乘空间里 `DstIn = Cd·as`，源不透明时它就是 `Cd`
/// 本身 —— 结果等于"第二个四边形根本没画"，看起来像 PSO 没换，其实是算对了。
#[test]
fn blend_modes_switch_pso_on_d3d12() {
    use zephyra_core::filter::BlendMode;
    let cases: [(BlendMode, [i32; 3]); 2] = [
        (BlendMode::Screen, [255, 128, 128]),
        (BlendMode::Modulate, [128, 0, 0]),
    ];
    for (mode, want) in cases {
        let s = SIZE as f32;
        let mut list = DisplayList::new();
        list.push(
            zephyra_core::paint::SortKey {
                layer: zephyra_core::paint::Layer::Background,
                z: 0,
                order: 0,
            },
            Command::Quad {
                rect: zephyra_core::core::Rect::new(0.0, 0.0, s, s),
                radius: zephyra_core::core::CornerRadius { radius: 0.0 },
                fill: Color::rgb(0.5, 0.5, 0.5),
                stroke: None,
                filter: zephyra_core::filter::FilterChain::none(),
                blend: BlendMode::default(),
            },
        );
        list.push(
            zephyra_core::paint::SortKey {
                layer: zephyra_core::paint::Layer::Content,
                z: 0,
                order: 1,
            },
            Command::Quad {
                rect: zephyra_core::core::Rect::new(0.0, 0.0, s, s),
                radius: zephyra_core::core::CornerRadius { radius: 0.0 },
                fill: Color::rgb(1.0, 0.0, 0.0),
                stroke: None,
                filter: zephyra_core::filter::FilterChain::none(),
                blend: mode,
            },
        );
        let (px, _) = render_and_readback(&list, Color::rgb(0.0, 0.0, 0.0));
        let p = at_rgba(&px, SIZE / 2, SIZE / 2);
        let got = [p[0] as i32, p[1] as i32, p[2] as i32];
        let ok = (0..3).all(|i| (got[i] - want[i]).abs() <= 4);
        assert!(
            ok,
            "{mode:?}（D3D12）应得 {:?}，实得 {got:?}（255,0,0 = PSO 没换成该模式的）",
            want
        );
    }
}

/// 同一个 `DisplayList` 里模式逐段不同 ⇒ `g_dst` 必须在**每段之前**刷新。
///
/// 若副本只在帧首拷一次，段3 读到的 `dst` 是帧首的纯灰，绿通道会变成 128 ——
/// 与 D3D11 那边 `per_run_ps_switching_uses_the_updated_destination` 的判据同源。
#[test]
fn per_run_dst_refresh_on_d3d12() {
    use zephyra_core::filter::BlendMode;
    let s = SIZE as f32;
    let mut list = DisplayList::new();
    let mut push = |order: u32,
                    fill: Color,
                    blend: BlendMode,
                    layer: zephyra_core::paint::Layer| {
        list.push(
            zephyra_core::paint::SortKey { layer, z: 0, order },
            Command::Quad {
                rect: zephyra_core::core::Rect::new(0.0, 0.0, s, s),
                radius: zephyra_core::core::CornerRadius { radius: 0.0 },
                fill,
                stroke: None,
                filter: zephyra_core::filter::FilterChain::none(),
                blend,
            },
        );
    };
    push(0, Color::rgb(0.5, 0.5, 0.5), BlendMode::default(), zephyra_core::paint::Layer::Background);
    push(1, Color::rgb(1.0, 0.0, 0.0), BlendMode::Darken, zephyra_core::paint::Layer::Content);
    push(2, Color::rgb(0.0, 0.0, 1.0), BlendMode::Screen, zephyra_core::paint::Layer::Overlay);
    list.sort();

    let (px, _) = render_and_readback(&list, Color::rgb(0.0, 0.0, 0.0));
    let p = at_rgba(&px, SIZE / 2, SIZE / 2);
    assert!(
        (p[0] as i32 - 128).abs() <= 4 && p[1] <= 4 && p[2] >= 252,
        "两段非默认模式应依次生效（期望 128,0,255），实得 {p:?}。\
         绿若为 128 = `g_dst` 只在帧首拷了一次"
    );
}

/// 可分离 / 非可分离那 14 个模式在 D3D12 上也必须**真的**生效。
///
/// # 判据为什么与 D3D11 相同却要再写一遍
///
/// 两个后端从**同一份** `.hlsl` 与**同一份** `DisplayList` 出发，但
/// `g_dst` 的拷贝、`t1` 的绑定、PSO 的选择是各写各的。任何一边漂移（漏绑
/// `t1`、PSO 用了 `SrcOver`、副本格式与 RT 不一致）都只会在这边暴露 ——
/// 共享代码的正确性不能替代两边各自的像素证据。
///
/// 背景中灰 + 源纯红，`Darken` 在预乘空间下的期望：
///
/// ```text
///   B = min(Cd, Cs) = (0.5, 0, 0) → 128,0,0
///   退化 SrcOver     → 255,0,0
/// ```
#[test]
fn separable_blends_are_applied_on_d3d12() {
    use zephyra_core::filter::BlendMode;
    let s = SIZE as f32;
    let mut list = DisplayList::new();
    list.push(
        zephyra_core::paint::SortKey {
            layer: zephyra_core::paint::Layer::Background,
            z: 0,
            order: 0,
        },
        Command::Quad {
            rect: zephyra_core::core::Rect::new(0.0, 0.0, s, s),
            radius: zephyra_core::core::CornerRadius { radius: 0.0 },
            fill: Color::rgb(0.5, 0.5, 0.5),
            stroke: None,
            filter: zephyra_core::filter::FilterChain::none(),
            blend: BlendMode::default(),
        },
    );
    list.push(
        zephyra_core::paint::SortKey {
            layer: zephyra_core::paint::Layer::Content,
            z: 0,
            order: 1,
        },
        Command::Quad {
            rect: zephyra_core::core::Rect::new(0.0, 0.0, s, s),
            radius: zephyra_core::core::CornerRadius { radius: 0.0 },
            fill: Color::rgb(1.0, 0.0, 0.0),
            stroke: None,
            filter: zephyra_core::filter::FilterChain::none(),
            blend: BlendMode::Darken,
        },
    );
    let (px, _) = render_and_readback(&list, Color::rgb(0.0, 0.0, 0.0));
    let p = at_rgba(&px, SIZE / 2, SIZE / 2);
    assert!(
        (p[0] as i32 - 128).abs() <= 4 && p[1] <= 4 && p[2] <= 4,
        "Darken 在 D3D12 上应生效得 (128,0,0)，实得 {p:?}。\
         若是 (255,0,0) = 退回了 SrcOver —— 检查 `RenderCtx::rt` 是否给了、\
         `refresh_dst_copy` 是否成功、PSO 是否配了 `Src`"
    );
}
// ---------------------------------------------------------------------------
//  E4.4 / E4.5 / E4.6：离屏 pass 的**像素**判据（D3D12）
// ---------------------------------------------------------------------------
//
//  与 D3D11 那边的同名测试**逐条对应**：判据、构图、甚至「为什么这条不是真空的」
//  都一样，因为两个后端从**同一份** `.hlsl` 与**同一份** `DisplayList` 出发。
//
//  重复写一遍不是为了保险：`.hlsl` 共用，但 `g_dst` 的拷贝、`t1` 的绑定、PSO 的
//  选择、**资源状态的推进**全是各写各的。任何一边漂移都只会在这边暴露 ——
//  而本文件里那条 `per_run_dst_refresh_on_d3d12` 就是共享代码正确性替代不了
//  两边各自像素证据的例子。

/// 只含一条命令的 `DisplayList`（离屏测试用）。
fn one(
    filter: zephyra_core::filter::FilterChain,
    fill: Color,
    rect: zephyra_core::core::Rect,
) -> DisplayList {
    let mut l = DisplayList::new();
    l.push(
        zephyra_core::paint::SortKey {
            layer: zephyra_core::paint::Layer::Background,
            z: 0,
            order: 0,
        },
        Command::Quad {
            rect,
            radius: zephyra_core::core::CornerRadius { radius: 0.0 },
            fill,
            stroke: None,
            filter,
            blend: zephyra_core::filter::BlendMode::default(),
        },
    );
    l
}

/// 探针图案：左半边被白块盖住，右半边是 clear 色。
fn bar() -> zephyra_core::core::Rect {
    zephyra_core::core::Rect::new(0.0, 8.0, 32.0, (SIZE - 16) as f32)
}

/// 右边界**正好落在** `SIZE / 2` 的白块。
///
/// 与 D3D11 那边的 `split_rect` 同名同形 —— 不是抄，是两边共用同一个度量方式：
/// `band_half_width` 采样的是 `SIZE/2 + 1 .. SIZE/2 + 11`，只有当内容的右边界落在
/// `SIZE/2` 时那段才落在过渡带里。拿 `bar()`（右边界在 x=32）去喂它，采样点全在
/// 空白上，量出来恒为 0 —— 判据不是抓到了 bug，而是压根没在看模糊。
fn split_rect() -> zephyra_core::core::Rect {
    zephyra_core::core::Rect::new(0.0, 8.0, SIZE as f32 / 2.0, SIZE as f32 - 16.0)
}

/// 一行里有多少种不同的灰度。
///
/// 硬边图案被模糊之后，过渡带里必然出现既不是 0 也不是满值的灰度；而**没做**模糊
/// 时全图只有两个值。所以这是「模糊真的跑了」这条判据的最短形式。
fn distinct_gray_levels(px: &[u8], x0: u32, y: u32, w: u32) -> usize {
    let mut levels = [false; 256];
    for x in x0..x0 + w {
        let p = at_rgba(px, x, y);
        let l = ((p[0] as u32 * 54 + p[1] as u32 * 183 + p[2] as u32 * 19) >> 8) as usize;
        levels[l] = true;
    }
    levels.iter().filter(|b| **b).count()
}

/// blur **真的**执行了：硬边被卷成过渡带。
///
/// # 判据为什么是"灰度种类数"
///
/// 源是硬边块：模糊没做 ⇒ 只有 0 与满值两种；模糊做了（哪怕 σ 很小）⇒ 过渡带里
/// 有一串中间值。三种情况互不相交（σ=0 那档也是两种），所以这一个数字就全分开了。
///
/// # 这一条在 D3D12 上格外重要
///
/// 模糊趟要在**两张离屏 RT 之间乒乓**，每趟都要改资源状态。而 D3D12 禁止「把仍绑在
/// RTV 上的资源转出 `RENDER_TARGET`」—— 违反了不会崩，只会让整张图变成一个死值。
/// 症状与"PS 绑错了""SRV 描述符指错了"完全同形。
#[test]
fn blur_produces_a_transition_band_on_d3d12() {
    let chain = zephyra_core::filter::FilterChain::single(zephyra_core::filter::Filter::Blur {
        sigma_x: 3.0,
        sigma_y: 0.0,
    });
    let (px, _) = render_and_readback(
        &one(chain, Color::rgb(1.0, 1.0, 1.0), bar()),
        Color::rgb(0.0, 0.0, 0.0),
    );
    let kinds = distinct_gray_levels(&px, 20, 32, 24);
    assert!(
        kinds >= 5,
        "D3D12 上模糊应把硬边卷成过渡带（≥5 种灰度），实得 {kinds} 种 —— \
         两种说明模糊没做；一种说明整张图被冲成了死值"
    );
}

/// 只模糊一个方向时，**另一个**方向必须保持硬边。
///
/// # 这条在抓什么
///
/// `PSBlurH` 与 `PSBlurV` 是两个独立入口，方向靠"绑哪个 PS"决定。若两者接反，
/// "有过渡带"那条判据照样通过 —— 所以必须有一条**只区分方向**的测试。
#[test]
fn horizontal_blur_leaves_vertical_edges_hard_on_d3d12() {
    let chain = zephyra_core::filter::FilterChain::single(zephyra_core::filter::Filter::Blur {
        sigma_x: 3.0,
        sigma_y: 0.0,
    });
    let (px, _) = render_and_readback(
        &one(chain, Color::rgb(1.0, 1.0, 1.0), bar()),
        Color::rgb(0.0, 0.0, 0.0),
    );
    let row_kinds = distinct_gray_levels(&px, 20, 32, 24);
    let col_kinds = {
        let mut levels = [false; 256];
        for y in 12..52 {
            let p = at_rgba(&px, 32, y);
            let l = ((p[0] as u32 * 54 + p[1] as u32 * 183 + p[2] as u32 * 19) >> 8) as usize;
            levels[l] = true;
        }
        levels.iter().filter(|b| **b).count()
    };
    assert!(
        row_kinds >= 5,
        "水平模糊应在行内产生过渡带，实得 {row_kinds} 种"
    );
    assert!(
        col_kinds <= 2,
        "只开 σx 时纵向必须保持硬边，实得 {col_kinds} 种 —— σy 显然也被应用了"
    );
}

/// 合成只落在 `bounds` 之内。
///
/// 底色是**纯蓝**：预乘的透明合成 SrcOver 是"什么都不变"，所以"离屏矩形之外被误画"
/// 在不透明底上**看不出来** —— 判据必须换一个可分辨的方向。
#[test]
fn composite_lands_only_on_the_bounds_on_d3d12() {
    let mut l = DisplayList::new();
    l.push(
        zephyra_core::paint::SortKey {
            layer: zephyra_core::paint::Layer::Background,
            z: 0,
            order: 0,
        },
        Command::Quad {
            rect: zephyra_core::core::Rect::new(0.0, 0.0, SIZE as f32, SIZE as f32),
            radius: zephyra_core::core::CornerRadius { radius: 0.0 },
            fill: Color::rgb(0.0, 0.0, 1.0),
            stroke: None,
            filter: zephyra_core::filter::FilterChain::none(),
            blend: zephyra_core::filter::BlendMode::default(),
        },
    );
    l.push(
        zephyra_core::paint::SortKey {
            layer: zephyra_core::paint::Layer::Content,
            z: 0,
            order: 1,
        },
        Command::Quad {
            rect: bar(),
            radius: zephyra_core::core::CornerRadius { radius: 0.0 },
            fill: Color::rgb(1.0, 1.0, 1.0),
            stroke: None,
            filter: zephyra_core::filter::FilterChain::single(
                zephyra_core::filter::Filter::Blur {
                    sigma_x: 2.0,
                    sigma_y: 2.0,
                },
            ),
            blend: zephyra_core::filter::BlendMode::default(),
        },
    );
    l.sort();
    let (px, _) = render_and_readback(&l, Color::rgb(0.0, 0.0, 0.0));
    // bounds 右边界 = 内容右(32) + outset(6) = 38 ⇒ x ∈ [38, 64) 全是底色。
    let outside = at_rgba(&px, 55, 32);
    assert!(
        outside[0] <= 2 && outside[1] <= 2 && outside[2] >= 253,
        "bounds 之外(x=55)应当仍是底色蓝，实得 {outside:?} —— 合成四边形铺过了界"
    );
    // bounds 之内取**远离图案边缘**的点：内容右边界在 x=32，σ=2 的过渡带跨
    // x ∈ [26, 38]，取 x=32 正好落在带中间（那是正确行为而不是 bug）。
    let inside = at_rgba(&px, 16, 32);
    assert!(
        inside[0] >= 253 && inside[1] >= 253 && inside[2] >= 253,
        "bounds 之内(x=16)应当是图案白，实得 {inside:?} —— 合成没落地"
    );
}

/// 合成**真的与目标混合**（`SrcOver` 而非 `Src`）。
///
/// # 两条约束，缺一个判据就真空
///
/// 1. **必须半透明**：预乘 `SrcOver` = `src + dst·(1−srcA)`，`srcA = 1` 时目标项
///    整项消失，`Src` 与 `SrcOver` 完全等价。25% alpha 的白叠蓝底：`SrcOver` 蓝通道
///    255，`Src` 只有 64。
/// 2. **滤镜链必须非恒等**：`Filter::Blur{0,0}` 是恒等，`plan_pass` 直接返回 `None`
///    —— 压根不开离屏，这条命令走普通路径，合成用什么混合状态都无所谓。实测踩过：
///    σ=0 的版本让这条判据形同虚设。
#[test]
fn composite_blends_over_the_destination_on_d3d12() {
    let mut l = DisplayList::new();
    l.push(
        zephyra_core::paint::SortKey {
            layer: zephyra_core::paint::Layer::Background,
            z: 0,
            order: 0,
        },
        Command::Quad {
            rect: zephyra_core::core::Rect::new(0.0, 0.0, SIZE as f32, SIZE as f32),
            radius: zephyra_core::core::CornerRadius { radius: 0.0 },
            fill: Color::rgb(0.0, 0.0, 1.0),
            stroke: None,
            filter: zephyra_core::filter::FilterChain::none(),
            blend: zephyra_core::filter::BlendMode::default(),
        },
    );
    l.push(
        zephyra_core::paint::SortKey {
            layer: zephyra_core::paint::Layer::Content,
            z: 0,
            order: 1,
        },
        Command::Quad {
            rect: zephyra_core::core::Rect::new(8.0, 8.0, 24.0, 24.0),
            radius: zephyra_core::core::CornerRadius { radius: 0.0 },
            fill: Color::rgba(1.0, 1.0, 1.0, 0.25),
            stroke: None,
            filter: zephyra_core::filter::FilterChain::single(
                zephyra_core::filter::Filter::Blur {
                    sigma_x: 0.0,
                    sigma_y: 2.0,
                },
            ),
            blend: zephyra_core::filter::BlendMode::default(),
        },
    );
    l.sort();
    let (px, _) = render_and_readback(&l, Color::rgb(0.0, 0.0, 0.0));
    let p = at_rgba(&px, 20, 20);
    assert!(
        p[2] >= 200,
        "蓝通道应 ≥200（底色透过来），实得 {p:?} —— 合成用了 `Src`"
    );
}

/// DropShadow：影子**按 offset 平移**、**染色**，且原图仍在其上。
///
/// # 为什么必须铺一层纯蓝底
///
/// 纯黑 clear 下"影子画在偏移位"与"影子画在原位、而原图盖住了它"都表现为"某处是黑的"，
/// 两者无法区分。蓝底让影子（黑）在蓝上分得开。
#[test]
fn drop_shadow_works_on_d3d12() {
    let mut l = DisplayList::new();
    l.push(
        zephyra_core::paint::SortKey {
            layer: zephyra_core::paint::Layer::Background,
            z: 0,
            order: 0,
        },
        Command::Quad {
            rect: zephyra_core::core::Rect::new(0.0, 0.0, SIZE as f32, SIZE as f32),
            radius: zephyra_core::core::CornerRadius { radius: 0.0 },
            fill: Color::rgb(0.0, 0.0, 1.0),
            stroke: None,
            filter: zephyra_core::filter::FilterChain::none(),
            blend: zephyra_core::filter::BlendMode::default(),
        },
    );
    l.push(
        zephyra_core::paint::SortKey {
            layer: zephyra_core::paint::Layer::Content,
            z: 0,
            order: 1,
        },
        Command::Quad {
            rect: zephyra_core::core::Rect::new(28.0, 28.0, 8.0, 8.0),
            radius: zephyra_core::core::CornerRadius { radius: 0.0 },
            fill: Color::rgb(1.0, 1.0, 1.0),
            stroke: None,
            filter: zephyra_core::filter::FilterChain::single(
                zephyra_core::filter::Filter::DropShadow {
                    color: Color::rgba(0.0, 0.0, 0.0, 1.0),
                    offset: zephyra_core::core::Point::new(12.0, 0.0),
                    sigma: 0.0,
                },
            ),
            blend: zephyra_core::filter::BlendMode::default(),
        },
    );
    l.sort();
    let (px, _) = render_and_readback(&l, Color::rgb(0.0, 0.0, 0.0));
    // 内容 x ∈ [28,36)，offset 12 ⇒ 影子 x ∈ [40,48)。x=42 只落在偏移后的影子里。
    let shadow = at_rgba(&px, 42, 32);
    assert!(
        shadow[2] <= 40,
        "x=42 只落在偏移后的影子里，应当是黑的，实得 {shadow:?} —— \
         offset 没被加进 translate"
    );
    let original = at_rgba(&px, 32, 32);
    assert!(
        original[0] >= 253 && original[2] >= 253,
        "x=32 是原图，应当是白（重画在影子之上），实得 {original:?}"
    );
}

/// 三级链（E4.6）比单趟糊得更开。
///
/// # 判据为什么选过渡带的**半宽**
///
/// 两趟 σ=2 的高斯 ≈ 一趟 σ=2.83（方差相加），三趟 ≈ σ=3.46。σ 越大过渡带越宽，
/// 所以半宽随链长单调增长。判据用**保守的**单调性而不是精确宽度 —— 精确宽度依赖
/// 高斯离散化，不该写死在像素断言里。
///
/// 变异目标：`blurs.iter().take(1)` 那样的实现必须让本条失败。
#[test]
fn a_longer_chain_blurs_more_on_d3d12() {
    fn band_half_width(px: &[u8]) -> usize {
        let mid = SIZE / 2;
        let mut n = 0usize;
        for d in 1..12u32 {
            let p = at_rgba(px, mid + d, 32);
            let l = (p[0] as i32 + p[1] as i32 + p[2] as i32) / 3;
            if l > 4 && l < 251 {
                n += 1;
            }
        }
        n
    }
    let one_pass = one(
        zephyra_core::filter::FilterChain::single(zephyra_core::filter::Filter::Blur {
            sigma_x: 2.0,
            sigma_y: 0.0,
        }),
        Color::rgb(1.0, 1.0, 1.0),
        split_rect(),
    );
    let three = one(
        zephyra_core::filter::FilterChain::new([
            zephyra_core::filter::Filter::Blur {
                sigma_x: 2.0,
                sigma_y: 0.0,
            },
            zephyra_core::filter::Filter::Blur {
                sigma_x: 2.0,
                sigma_y: 0.0,
            },
            zephyra_core::filter::Filter::Blur {
                sigma_x: 2.0,
                sigma_y: 0.0,
            },
        ]),
        Color::rgb(1.0, 1.0, 1.0),
        split_rect(),
    );
    let w1 = band_half_width(&render_and_readback(&one_pass, Color::rgb(0.0, 0.0, 0.0)).0);
    let w3 = band_half_width(&render_and_readback(&three, Color::rgb(0.0, 0.0, 0.0)).0);
    assert!(w1 >= 2, "单趟模糊本身就得有过渡带，实得半宽 {w1}");
    assert!(
        w3 > w1,
        "三级链应比单趟糊得更开：单趟 {w1}，三级 {w3} —— 链只跑了第一趟？"
    );
}

/// 滤镜真的被执行了（计数层面）。
///
/// 这是**唯一**能抓住"离屏整条链建不出来（分配失败 / 设备拒绝某种格式）"的判据
/// —— 那种情况下元素**不出现**，而像素断言看到的是"少了一个元素"，与"这个元素本来
/// 就不该出现"无法区分。
#[test]
fn offscreen_pass_reports_zero_unimplemented_on_d3d12() {
    use zephyra_core::filter::Filter;
    let Ok(mut gpu) = Gpu::new() else {
        panic!("建不出 D3D12 设备");
    };
    gpu.begin_frame().expect("应当能开帧");
    let target = Offscreen::new(&gpu.device, (SIZE, SIZE)).expect("应当建得出离屏目标");
    let mut renderer = Renderer::new(&gpu.device, ATLAS_SIZE, 1).expect("应当建得出渲染器");
    let textures = TextureTable::new();
    let list = one(
        zephyra_core::filter::FilterChain::single(Filter::Blur {
            sigma_x: 2.0,
            sigma_y: 2.0,
        }),
        Color::rgb(1.0, 1.0, 1.0),
        bar(),
    );
    let report = renderer
        .record(zephyra_winit_d3d12::RenderCtx {
            device: &gpu.device,
            list: &gpu.list,
            rtv: target.rtv,
            display_list: &list,
            size: (SIZE, SIZE),
            clear: Color::rgb(0.0, 0.0, 0.0),
            textures: &textures,
            rt: Some(&target.texture),
            damage: zephyra_core::frame::Damage::Full,
        })
        .expect("记录绘制命令失败");
    assert_eq!(
        report.unimplemented_filters, 0,
        "滤镜已由离屏 pass 执行（passes={}）",
        report.offscreen_passes
    );
    assert_eq!(report.offscreen_passes, 1, "恰好一条命令走了离屏");
    gpu.submit().expect("提交失败");
    gpu.wait_gpu().expect("等 GPU 失败");
}

/// 分段回放的**顺序**：滤镜元素压在下层图元之上。
///
/// # 为什么这条非写不可
///
/// 分段回放的全部意义就是保持绘制顺序。若实现成"先跑完所有离屏、再画普通段"，离屏
/// 结果会落到所有普通图元**之下** —— 于是这里的上层红块会盖不住离屏白条，画面全红，
/// 而所有计数都正常。两种顺序都只对一半，所以只能靠像素区分。
#[test]
fn offscreen_result_is_ordered_above_lower_primitives_on_d3d12() {
    let mut l = DisplayList::new();
    // order 1：离屏模糊的白条（"上层"）。
    l.push(
        zephyra_core::paint::SortKey {
            layer: zephyra_core::paint::Layer::Content,
            z: 0,
            order: 1,
        },
        Command::Quad {
            rect: zephyra_core::core::Rect::new(8.0, 8.0, (SIZE - 16) as f32, (SIZE - 16) as f32),
            radius: zephyra_core::core::CornerRadius { radius: 0.0 },
            fill: Color::rgb(1.0, 1.0, 1.0),
            stroke: None,
            filter: zephyra_core::filter::FilterChain::single(
                zephyra_core::filter::Filter::Blur {
                    sigma_x: 1.0,
                    sigma_y: 1.0,
                },
            ),
            blend: zephyra_core::filter::BlendMode::default(),
        },
    );
    // order 2：全屏纯红（"更上层"）。
    l.push(
        zephyra_core::paint::SortKey {
            layer: zephyra_core::paint::Layer::Popup,
            z: 0,
            order: 2,
        },
        Command::Quad {
            rect: zephyra_core::core::Rect::new(0.0, 0.0, SIZE as f32, SIZE as f32),
            radius: zephyra_core::core::CornerRadius { radius: 0.0 },
            fill: Color::rgb(1.0, 0.0, 0.0),
            stroke: None,
            filter: zephyra_core::filter::FilterChain::none(),
            blend: zephyra_core::filter::BlendMode::default(),
        },
    );
    l.sort();
    let (px, _) = render_and_readback(&l, Color::rgb(0.0, 0.0, 0.0));
    let p = at_rgba(&px, SIZE / 2, SIZE / 2);
    assert!(
        p[0] >= 253 && p[1] <= 2,
        "更上层(Popup)的红应当盖住离屏白条，实得 {p:?} —— 离屏结果被画到了末尾"
    );
}
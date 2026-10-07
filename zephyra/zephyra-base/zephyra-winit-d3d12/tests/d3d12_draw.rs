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
//! D3D11 渲染管线的**端到端**冒烟测试：真的建设备、真的回放 `DisplayList`、
//! 真的把渲染目标读回来，然后断言"像素确实变了"。
//!
//! # 为什么必须有这个测试
//!
//! D3D11 的失败模式是**不报错但什么都不画**：没有 HRESULT、没有异常、日志一切正常。
//! 于是"进程跑起来了"与"画面是对的"完全是两件事，而只有后者能靠断言守住。
//!
//! 我在把渲染器从 wgpu 迁到 D3D11 时踩过：顶点数据、索引数据、输入布局签名
//! **逐项读回验证全部正确**，画面却只有清屏色。
//!
//! # 断言怎么写才不是恒真式（这里有个我犯过的错）
//!
//! 第一版我清成绿色，然后数**绿色**像素当覆盖率。那样清屏本身就满足断言，
//! 测试**永远通过**，等于什么都没测 —— 于是我据此得出"裸 D3D11 绘制是好的，
//! 问题在渲染器"，那个结论完全是错的。
//!
//! 现在统一：清成**黑色**，数**非黑**像素。这样"覆盖了 N 个像素"只能是画出来的结果，
//! 而"没画"必然是 0。
#![cfg(windows)]

use zephyra_core::frame::Damage;
use zephyra_core::core::{Color, CornerRadius, Rect};
use zephyra_core::paint::{Command, DisplayList, SortKey};

use zephyra_winit::renderer::{RenderCtx, Renderer};
use zephyra_winit::surface::Gpu;
use zephyra_winit::texture::TextureTable;

/// 离屏渲染目标的边长。
const SIZE: u32 = 64;

/// 覆盖窗口内多少比例才算画出来了。
///
/// 取一半而不是全部：SDF 的抗锯齿边缘允许有少量像素不落在矩形内，
/// 而"只差几个像素"和"一个像素都没画"必须能被区分开。
const MIN_COVERAGE: f64 = 0.5;

/// 清屏色：纯黑。
///
/// 它是**判据的一部分** —— [`非背景像素`] 数的就是"不是黑色"的像素。
const CLEAR: Color = Color::rgba(0.0, 0.0, 0.0, 1.0);

/// 渲染器能把两个四边形真的画到像素上。
///
/// # 为什么绕开窗口与交换链
///
/// 交换链会引入 flip model、后缓冲计数、`Present` 时机、DComp 等一堆**与本测试
/// 无关**的失败原因。离屏纹理是到己建的、不经过 DXGI 的渲染目标，于是出问题时
/// 故障域只剩渲染器本身。
#[test]
fn 矩形会真的落到像素上() {
    let Ok(gpu) = Gpu::new() else {
        eprintln!("跳过：建不出 D3D11 设备");
        return;
    };
    let (rtv, tex) = offscreen_target(&gpu);

    let mut renderer = Renderer::new(&gpu.device).expect("渲染器应当能建出来");
    let textures = TextureTable::new();
    let list = two_quads();

    let report = renderer.render(RenderCtx {
        context: &gpu.context,
        device: &gpu.device,
        rtv: &rtv,
        list: &list,
        size: (SIZE, SIZE),
        clear: CLEAR,
        textures: &textures,
        // 这份测试只验「几何是否铺满视口」，所以用整帧。
        //
        // ⚠ 别改成某个子矩形来「顺便」验 scissor —— 那样这条判据就变成两件事的
        // 合取：几何不对**或** scissor 不对都会失败，而两者的排查方向完全不同。
        damage: zephyra_core::frame::Damage::Full,
    });

    assert_eq!(report.quads, 2, "两份命令应当产生两个四边形");

    let pixels = read_back(&gpu, &tex);
    let drawn = non_background(&pixels);
    let total = (SIZE * SIZE) as f64;
    eprintln!("[t] 渲染报告 {report:?}");
    eprintln!("[t] 覆盖 {drawn}/{total} 像素，颜色分布 {:?}", color_hist(&pixels));

    // ---- 视口读回（**输入 count 给 16**）----
    //
    // ⚠ `RSGetViewports` 的 `pNumViewports` 是 `[in, out]`：输入值表示
    // `pViewports` 数组的长度。原代码传的是 **0**，而 `pViewports` 也确实只有
    // 1 个元素 —— 于是「你要 0 个」就是「给你 0 个」，读回恒为 0，与驱动状态无关。
    // 官方原文：
    //   "on input, specifies the number of viewports ... in the *pViewports* array;
    //    on output, the variable contains the actual number of viewports that are
    //    bound to the rasterizer stage."
    unsafe {
        let mut n = 16u32;
        let mut buf = [D3D11_VIEWPORT::default(); 16];
        gpu.context.RSGetViewports(&mut n, Some(buf.as_mut_ptr()));
        eprintln!("[t] 视口读回：个数={n} 宽={} 高={}", buf[0].Width, buf[0].Height);
    }
    eprintln!(
        "[t] 设备移除原因 = {:?}",
        unsafe { gpu.device.GetDeviceRemovedReason() }
    );
    // ---- 那几个像素**在哪** ----
    //
    // 此前一直假设它们在原点，而那只是「退化三角形」的推测。位置本身就是证据：
    // 原点 ⇒ 顶点属性读成 0；其它地方 ⇒ 视口/坐标变换的问题。
    {
        let mut at = Vec::new();
        for (i, p) in pixels.chunks_exact(4).enumerate() {
            if p[0] != 0 || p[1] != 0 || p[2] != 0 {
                at.push((i % SIZE as usize, i / SIZE as usize, [p[0], p[1], p[2], p[3]]));
            }
        }
        eprintln!("[t] 非黑像素共 {} 个，位置 = {:?}", at.len(), &at[..at.len().min(8)]);
    }

    assert!(
        drawn as f64 >= total * MIN_COVERAGE,
        "只覆盖了 {drawn}/{total} 个像素 —— 渲染基本没画上东西。\
         D3D11 不画任何东西时不会报错，所以这一条断言是唯一的防线。"
    );
}

/// 裸 D3D11 绘制：自己建缓冲、复用 [`zephyra_winit::shader`] 的着色器与输入布局，
/// 画一个铺满视口的三角形，然后读回像素。
///
/// # 它的作用是当"对照组"
///
/// 它和 [`矩形会真的落到像素上`] 共用同一份着色器、同一份输入布局、同一份顶点数据、
/// 同一种缓冲（`DYNAMIC` + `Map`），唯一差别是**不经过 `Renderer::render`**。
///
/// 于是"能不能画"被从一堆变量里单独拎了出来：
///
/// - 两边都画不出 → 问题在着色器 / 输入布局 / 顶点布局，与渲染器无关；
/// - 只有渲染器画不出 → 问题在 [`Renderer::render`] 的管线状态设置里。
///
/// # ⚠ 这一格自己以前也漏了拓扑，于是它是「坏掉的对照组」
///
/// 两边都只画出 2/4096 个像素时，据此判定「问题在两者共有的那一半」—— 而共有
/// 的那一半里就包括**都漏掉的** `IASetPrimitiveTopology`。拿一个自己也是坏的
/// 东西当参照物，两边一起错，结论看着自洽却毫无信息。
#[test]
fn 裸_d3d11_能画出满屏三角形() {
    let Ok(gpu) = Gpu::new() else {
        eprintln!("跳过：建不出 D3D11 设备");
        return;
    };
    let (rtv, tex) = offscreen_target(&gpu);
    let shaders = zephyra_winit::shader::Shaders::new(&gpu.device).expect("着色器应当能建出来");

    // 用**渲染器完全相同**的数据（两个四边形 + `expand_indices` 展开的索引），
    // 这样"数据"这一项不再是变量。
    let list = two_quads();
    let (verts, indices) = renderer_data(&list);

    let vb = dynamic_buffer_sized(&gpu.device, D3D11_BIND_VERTEX_BUFFER, 64 * 1024);
    let ib = dynamic_buffer_sized(&gpu.device, D3D11_BIND_INDEX_BUFFER, 256 * 6 * 2);

    // ---- 顶点数据诊断 ----
    //
    // 两个退化像素（alpha=128 的抗锯齿边）说明光栅器收到的是**NDC 原点上的一个点**
    // —— 也就是顶点位置全 0。所以要先分清「数据没写进去」与「写进去了但绑错了」。
    eprintln!(
        "[v] 顶点数 = {}，索引数 = {}",
        verts.len() / 60,
        indices.len() / 2
    );
    {
        let v: &[zephyra_winit::quad::Vertex] = bytemuck::cast_slice(&verts);
        for (i, vert) in v.iter().take(4).enumerate() {
            eprintln!(
                "[v]   #{i} position={:?} local={:?} kind={} color={:?}",
                vert.position, vert.local, vert.kind, vert.color
            );
        }
        eprintln!("[v] 索引 = {:?}", &indices[..12.min(indices.len())]);
    }

    unsafe {
        gpu.context
            .OMSetRenderTargets(Some(&[Some(rtv.clone())]), None::<&ID3D11DepthStencilView>);
        gpu.context.RSSetViewports(Some(&[D3D11_VIEWPORT {
            TopLeftX: 0.0,
            TopLeftY: 0.0,
            Width: SIZE as f32,
            Height: SIZE as f32,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        }]));
        // 设完**立刻**读回：区分"设的时候就没生效"与"设了之后被别的东西重置"。
        {
            // 具名数组，而不是内联临时值 —— 用来验证"临时值被提前 drop"这条假设。
            let named = [D3D11_VIEWPORT {
                TopLeftX: 0.0,
                TopLeftY: 0.0,
                Width: SIZE as f32,
                Height: SIZE as f32,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            }];
            gpu.context.RSSetViewports(Some(&named[..]));
            // 输入 count 必须给**数组的真实长度**。传 0 时运行时按「你要 0 个」
            // 处理，于是无论状态如何都读回 0 —— 那正是本文件原先「视口设不进去」
            // 这个结论的来源。
            let mut c = 16u32;
            let mut buf = [D3D11_VIEWPORT::default(); 16];
            gpu.context.RSGetViewports(&mut c, Some(buf.as_mut_ptr()));
            eprintln!("[t] 具名数组：个数={c} 宽={} 高={}", buf[0].Width, buf[0].Height);
        }
        gpu.context
            .ClearRenderTargetView(&rtv, &[CLEAR.r, CLEAR.g, CLEAR.b, CLEAR.a]);
        gpu.context.VSSetShader(Some(&shaders.vs), None);
        gpu.context.PSSetShader(Some(&shaders.ps), None);
        gpu.context.IASetInputLayout(&shaders.input_layout);
        let stride = std::mem::size_of::<zephyra_winit::quad::Vertex>() as u32;
        let off = 0u32;
        gpu.context.IASetVertexBuffers(
            0,
            1,
            Some([Some(vb.clone())].as_ptr()),
            Some(&stride),
            Some(&off),
        );
        gpu.context
            .IASetIndexBuffer(&ib, DXGI_FORMAT_R16_UINT, 0);
        // ⚠⚠ 这一行是**真凶**，而且本测试自己以前也漏了它，所以它一直是「坏掉的
        // 对照组」—— 拿它当参照物去判断渲染器，结论必然是错的。
        //
        // 不设拓扑时上下文用的是 `D3D11_PRIMITIVE_TOPOLOGY_UNDEFINED`，`DrawIndexed`
        // 拿它当参数**不报错**：`RenderReport` 正常、设备未移除、画面上却只剩两个
        // 退化像素。官方把这一步列为绑完输入缓冲后的硬性要求，见 `renderer.rs`
        // 里 `IASetPrimitiveTopology` 处的引用。
        gpu.context.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
        map_write(&gpu.context, &vb, &verts);
        map_write(&gpu.context, &ib, &indices);
        // ---- 顶点缓冲绑定读回 ----
        //
        // ⚠ 曾经的注释说「布局与签名不匹配」也可能导致属性读成全 0，并据此把它当
        // 候选原因之一。那是**错的**：`CreateInputLayout` 拿着色器签名校验
        // （官方原文："this allows the API to validate the input-layout-object
        // elements against the shader-input signature to make sure that there is
        // an exact match of types and semantics"），对不上会直接失败，而
        // `Shaders::new` 已经抛过错了。布局既然建出来就是对的。
        //
        // C API 的 `IAGetVertexBuffers(StartSlot, NumBuffers, ppBuffers, pStrides,
        // pOffsets)` **没有** 个数出参 —— 读几个由 `NumBuffers` 决定。
        {
            let mut got: [Option<ID3D11Buffer>; 1] = [None];
            let mut strides = [0u32; 1];
            let mut offsets = [0u32; 1];
            gpu.context.IAGetVertexBuffers(
                0,
                1,
                Some(got.as_mut_ptr()),
                Some(strides.as_mut_ptr()),
                Some(offsets.as_mut_ptr()),
            );
            eprintln!(
                "[vb] IAGetVertexBuffers → 缓冲 {}、stride {}、offset {}",
                if got[0].is_some() { "有" } else { "空" },
                strides[0],
                offsets[0]
            );
        }
        // 12 个索引 = 两个四边形 × 两个三角形。
        gpu.context.DrawIndexed(12, 0, 0);
    }

    let pixels = read_back(&gpu, &tex);
    let drawn = non_background(&pixels);
    let total = (SIZE * SIZE) as f64;

    assert!(
        drawn as f64 >= total * MIN_COVERAGE,
        "裸 D3D11 绘制只覆盖了 {drawn}/{total} 个像素 —— 不经过渲染器也画不出来，\
         那问题在顶点数据 / 输入布局 / 拓扑这三者之一"
    );
}

/// 两个四边形：整屏红 + 左上半屏蓝。
///
/// 刻意用两种颜色，且都**不等于**清屏色 —— 只用一种颜色的话，"清屏色恰好等于命令色"
/// 会让测试假通过。
fn two_quads() -> DisplayList {
    let mut list = DisplayList::new();
    let s = SIZE as f32;
    list.push(
        SortKey {
            layer: zephyra_core::paint::Layer::Content,
            z: 0,
            order: 0,
        },
        Command::Quad {
            rect: Rect::new(0.0, 0.0, s, s),
            radius: CornerRadius { radius: 0.0 },
            fill: Color::rgb(1.0, 0.0, 0.0),
            stroke: None,
            filter: zephyra_core::filter::FilterChain::none(),
            blend: zephyra_core::filter::BlendMode::default(),
        },
    );
    list.push(
        SortKey {
            layer: zephyra_core::paint::Layer::Content,
            z: 0,
            order: 1,
        },
        Command::Quad {
            rect: Rect::new(0.0, 0.0, s / 2.0, s / 2.0),
            radius: CornerRadius { radius: 0.0 },
            fill: Color::rgb(0.0, 0.0, 1.0),
            stroke: None,
            filter: zephyra_core::filter::FilterChain::none(),
            blend: zephyra_core::filter::BlendMode::default(),
        },
    );
    list.sort();
    list
}

/// 取出渲染器会喂给 GPU 的那份数据（顶点流 + 展开后的索引）。
fn renderer_data(list: &DisplayList) -> (Vec<u8>, Vec<u8>) {
    let batch = zephyra_winit::quad::expand(list, (SIZE, SIZE), |_| false);
    let (all, _, _) = batch.flatten();
    let idx = zephyra_winit::quad::expand_indices(all.len() / 4);
    (
        bytemuck::cast_slice(&all).to_vec(),
        bytemuck::cast_slice(&idx).to_vec(),
    )
}

/// 帧内 scissor 只影响脏区内部，脏区外一个像素都不许动。
///
/// # 这是 S2（脏区）唯一的**像素级**判据
///
/// 其余测试都走 `Damage::Full`，所以它们证明不了 scissor 真的生效 —— 而
/// scissor 失效是**静默**的：画面照样出得来，只是省不下填充率，没有任何一个
/// 计数器会变。所以必须有一条测试直接量「脏区外有没有被碰过」。
///
/// # 判据怎么设计才不会恒真
///
/// 关键是**脏区外必须有一个已知的、不是清屏色的初始值**。若离屏纹理是新建的，
/// 它的内容未定义，于是「脏区外没被碰过」与「脏区外恰好也是黑的」无法区分。
///
/// 所以这里先把整张纹理**明确地**清成红色，再用 `Damage::Full` 跑一次蓝方块，
/// 然后用 `Damage::Rect`（只覆盖左半边）再跑一次。此时断言：
///
/// - 左半边 = 蓝（新画的）
/// - 右半边 = 红（**没被碰过** —— 若 scissor 失效会被清成黑色再画上蓝）
///
/// 这条也顺带钉住了「clear 受 scissor 约束」：若 renderer 把 scissor 设在了
/// `ClearRenderTargetView` **之后**，左半边会是「清成黑 + 画蓝」，而右半边仍是红 ——
/// 看起来照样通过，但真实宿主里那块就永远清不干净。所以下面额外断言左半边的
/// **边缘**不是黑色。
#[test]
fn 帧内_scissor_只影响脏区() {
    let Ok(gpu) = Gpu::new() else {
        eprintln!("跳过：建不出 D3D11 设备");
        return;
    };
    let (rtv, tex) = offscreen_target(&gpu);
    let mut renderer = Renderer::new(&gpu.device).expect("渲染器应当能建出来");
    let textures = TextureTable::new();

    // ---- 1. 清成红色并读回当基线；渲染时的**清屏色也用同一个红** ----
    //
    // ⚠ 为什么清屏色必须等于基线色：D3D11 的 `ClearRenderTargetView` **不受 scissor
    // 约束**（D3D12 才受），所以清屏永远刷满整张纹理。若基线与清屏色不同，脏区外
    // 必然「变了」（被清成清屏色），而那条变化与 scissor 无关 —— 判据就废了。
    //
    // 取同一个红之后，「脏区外一个字节都没变」就精确等价于「脏区外什么都没画」。
    // 第一版就是没意识到这点：基线红、清屏黑，于是脏区外 2048/2048 全变，
    // 白查了半天 scissor。
    //
    // 判据也刻意**不判具体颜色**，只判「变没变」：离屏格式是 `B8G8R8A8` 而 `Color`
    // 是 RGBA，通道顺序相反；按直觉写判据会把红判成蓝（第一版的第二个坑）。
    // 与基线逐字节比就不依赖任何通道约定。
    unsafe { gpu.context.ClearRenderTargetView(&rtv, &[1.0, 0.0, 0.0, 1.0]) };
    let baseline = read_back(&gpu, &tex);
    assert!(
        baseline.iter().any(|&b| b != 0),
        "基线全是 0 —— 那块离屏纹理的内容不可读，本测试的判据整个失效"
    );
    const RED_CLEAR: Color = Color::rgba(1.0, 0.0, 0.0, 1.0);

    // ---- 2. 铺满视口的方块，用左半边做脏区 ----
    let mut list = DisplayList::new();
    let s = SIZE as f32;
    list.push(
        SortKey {
            layer: zephyra_core::paint::Layer::Content,
            z: 0,
            order: 0,
        },
        Command::Quad {
            rect: Rect::new(0.0, 0.0, s, s),
            radius: CornerRadius { radius: 0.0 },
            fill: Color::rgb(0.0, 0.0, 1.0),
            stroke: None,
            filter: zephyra_core::filter::FilterChain::none(),
            blend: zephyra_core::filter::BlendMode::default(),
        },
    );

    let mid = SIZE as usize / 2;
    let half = Damage::Rect(Rect::new(0.0, 0.0, s / 2.0, s));
    let report = renderer.render(RenderCtx {
        context: &gpu.context,
        device: &gpu.device,
        rtv: &rtv,
        list: &list,
        size: (SIZE, SIZE),
        clear: RED_CLEAR,
        textures: &textures,
        damage: half,
    });
    assert!(report.quads > 0, "一个四边形都没提交 —— 判据失效，先查别的");

    let after = read_back(&gpu, &tex);

    // ---- 3. 脏区内该变了，脏区外该一个字节都没变 ----
    let (mut changed_in, mut same_in) = (0usize, 0usize);
    let (mut changed_out, mut same_out) = (0usize, 0usize);
    for y in 0..SIZE as usize {
        for x in 0..SIZE as usize {
            let i = (y * SIZE as usize + x) * 4;
            let differs = after[i..i + 4] != baseline[i..i + 4];
            if x < mid {
                if differs {
                    changed_in += 1;
                } else {
                    same_in += 1;
                }
            } else if differs {
                changed_out += 1;
            } else {
                same_out += 1;
            }
        }
    }
    let in_total = SIZE as usize * mid;
    let out_total = SIZE as usize * (SIZE as usize - mid);
    eprintln!(
        "[t] 脏区内变化 {changed_in}/{in_total}（未变 {same_in}）、\
         脏区外变化 {changed_out}/{out_total}（未变 {same_out}）"
    );

    assert!(
        changed_in as f64 >= in_total as f64 * 0.95,
        "脏区内只有 {changed_in}/{in_total} 个像素变了 —— scissor 把该画的也裁掉了"
    );
    assert_eq!(
        changed_out, 0,
        "脏区外有 {changed_out}/{out_total} 个像素变了 —— scissor 没生效。\
         这类失败是静默的：画面照样出得来，只是省不下填充率，没有计数器会变。"
    );
}

/// 图像圆角在 GPU 上真的切掉了四角。
///
/// # 这是着色器改动的**唯一**判据
///
/// `zephyra.hlsl` 的 IMAGE 分支加了 SDF 遮罩，而那段 HLSL 只在 `.bin` 里 ——
/// 单测、`--dump-cpu` 全都碰不到它。所以必须有这条像素级断言。
///
/// # 判据只看「形状」，不判颜色
///
/// 离屏格式是 `B8G8R8A8` 而 `Color` 是 RGBA，通道顺序相反。所以这里把
/// **纹理**做成纯白 + **alpha 通道**当判据：白图在 alpha 上是 1，底色在 alpha 上是
/// 0/1…——不，还是会被颜色顺序坑到。
///
/// 干脆用**两层底色**：先把 RT 清成红，再画。矩形内被纹理（纯白）覆盖 ⇒ 变白；
/// 四角被遮罩压掉 ⇒ 保持红。所以判据是「变白了没有」，与通道顺序无关：
/// 无论白是 `[255,255,255]` 还是别的什么，它**不等于**红。
///
/// # 为什么需要一个真的注册进纹理表的纹理
///
/// 图像命令在 `quad::expand` 里会按纹理表过滤（未注册 ⇒ 整条跳过并计入
/// `missing_textures`）。不注册的话画的是空气，而 `report.quads` 仍然 > 0 ——
/// 又是一个「计数器正常但画面是空的」陷阱。
#[test]
fn 图像圆角_四角被切掉() {
    let Ok(gpu) = Gpu::new() else {
        eprintln!("跳过：建不出 D3D11 设备");
        return;
    };
    let (rtv, tex) = offscreen_target(&gpu);
    let mut renderer = Renderer::new(&gpu.device).expect("渲染器应当能建出来");

    // ---- 1. 一张纯白纹理 ----
    let ttex = white_texture(&gpu, 16, 16);
    let tsrv = zephyra_winit::texture::create_srv(&gpu.device, &ttex).expect("应当能建 SRV");
    let mut textures = TextureTable::new();
    textures.insert("img", tsrv, ttex);

    // ---- 2. 底色铺红 ----
    unsafe { gpu.context.ClearRenderTargetView(&rtv, &[1.0, 0.0, 0.0, 1.0]) };
    let baseline = read_back(&gpu, &tex);
    assert!(baseline.iter().any(|&b| b != 0), "基线全 0，判据失效");

    // ---- 3. 一张铺满、圆角 16 的图像 ----
    let s = SIZE as f32;
    let mut list = DisplayList::new();
    list.push(
        SortKey {
            layer: zephyra_core::paint::Layer::Content,
            z: 0,
            order: 0,
        },
        Command::Image {
            texture: zephyra_core::paint::TextureRef::new("img"),
            src: None,
            dst: Rect::new(4.0, 4.0, s - 8.0, s - 8.0),
            radius: CornerRadius { radius: 16.0 },
            tint: Color::rgb(1.0, 1.0, 1.0),
            filter: zephyra_core::filter::FilterChain::none(),
            blend: zephyra_core::filter::BlendMode::default(),
        },
    );
    let report = renderer.render(RenderCtx {
        context: &gpu.context,
        device: &gpu.device,
        rtv: &rtv,
        list: &list,
        size: (SIZE, SIZE),
        clear: Color::rgb(1.0, 0.0, 0.0),
        textures: &textures,
        damage: zephyra_core::frame::Damage::Full,
    });
    assert_eq!(
        report.missing_textures, 0,
        "纹理没注册上 —— 画的是空气，这条测试的判据整个失效"
    );
    assert_eq!(report.quads, 1, "应当正好一条图像四边形");

    let px = read_back(&gpu, &tex);
    let at = |x: u32, y: u32| &px[(y as usize * SIZE as usize + x as usize) * 4..][..4];

    { let mut out=String::new(); for y in 0..16u32 { for x in 0..16u32 { let p=at(x*4,y*4); out.push_str(if p[0]>200&&p[1]>200&&p[2]>200 {"W"} else if p[0]>150&&p[1]<80 {"R"} else {"."}); } out.push('\n'); } eprintln!("[t] 16x16 (W=白 R=红 .=其它):\n{out}"); }
    // 中心：被纯白纹理覆盖 ⇒ 三个通道都接近 255。
    let c = at(SIZE / 2, SIZE / 2);
    assert!(
        c[0] > 200 && c[1] > 200 && c[2] > 200,
        "中心应是白的，实际 {c:?} —— 图像根本没画出来"
    );

    // 四角：圆角 16 而矩形 4..60，所以紧贴矩形角的像素在形状之外 ⇒ 仍是红底。
    // 只断言「不是白的」，与通道顺序无关。
    { let mut out=String::new(); for y in 0..16u32 { for x in 0..16u32 { let p=at(x*4,y*4); out.push(if p[0]>200&&p[1]>200&&p[2]>200 {'W'} else if p[0]>150 {'R'} else {'.'}); } out.push('\n'); } eprintln!("[t] 16x16 缩略图:\n{out}"); }
    let mut white_corners = 0;
    for (x, y) in [(5u32, 5u32), (58, 5), (5, 58), (58, 58)] {
        let p = at(x, y);
        if p[0] > 200 && p[1] > 200 && p[2] > 200 {
            white_corners += 1;
            eprintln!("[t] ({x},{y}) 是白的 {p:?} —— 圆角没切掉");
        }
    }
    assert_eq!(
        white_corners, 0,
        "四个角里有 {white_corners} 个是白的 —— IMAGE 分支的 SDF 遮罩没生效"
    );
}

/// 直角图像的边缘**不能被压暗** —— 遮罩不能无条件施加。
///
/// ⚠️ 这是同一个陷阱的两面：无条件施加时 `1 - smoothstep(-px, px, d)` 在靠近边界处
/// < 1，最外一圈于是变暗。症状是「所有直角图像的边缘发暗一圈」，
/// 而 `quads`、`missing_textures`、顶点字节数全部正常。
///
/// # ⚠️ 判据必须读**颜色**，不能读 alpha
///
/// 第一版断言的是边缘像素的 alpha，结果**变异测试抓不到**这个 bug。原因是：
/// 遮罩把 src 的 alpha 压到 0.93，而清屏色是**不透明**的黑，于是
/// `SrcAlpha / InvSrcAlpha` 混合后 `a = 0.93 + 0.07×1 = 1.0` —— alpha 又饱和回 255，
/// 断言照样通过。
///
/// 而「变暗」这件事发生在**颜色**上（白 × 0.93 = 236），所以判据必须落在 RGB 上。
/// 这条与本仓反复记录的「判据要落在真正变化的那个量上」是同一件事。
#[test]
fn 直角图像的边缘不被压暗() {
    let Ok(gpu) = Gpu::new() else {
        eprintln!("跳过：建不出 D3D11 设备");
        return;
    };
    let (rtv, tex) = offscreen_target(&gpu);
    let mut renderer = Renderer::new(&gpu.device).expect("渲染器应当能建出来");

    let ttex = white_texture(&gpu, 16, 16);
    let tsrv = zephyra_winit::texture::create_srv(&gpu.device, &ttex).expect("应当能建 SRV");
    let mut textures = TextureTable::new();
    textures.insert("img", tsrv, ttex);

    let s = SIZE as f32;
    let mut list = DisplayList::new();
    list.push(
        SortKey {
            layer: zephyra_core::paint::Layer::Content,
            z: 0,
            order: 0,
        },
        Command::Image {
            texture: zephyra_core::paint::TextureRef::new("img"),
            src: None,
            dst: Rect::new(0.0, 0.0, s, s),
            radius: CornerRadius { radius: 0.0 },
            tint: Color::rgb(1.0, 1.0, 1.0),
            filter: zephyra_core::filter::FilterChain::none(),
            blend: zephyra_core::filter::BlendMode::default(),
        },
    );
    renderer.render(RenderCtx {
        context: &gpu.context,
        device: &gpu.device,
        rtv: &rtv,
        list: &list,
        size: (SIZE, SIZE),
        clear: Color::rgb(0.0, 0.0, 0.0),
        textures: &textures,
        damage: zephyra_core::frame::Damage::Full,
    });

    let px = read_back(&gpu, &tex);
    let at = |x: u32, y: u32| &px[(y as usize * SIZE as usize + x as usize) * 4..][..4];

    // 铺满全屏 ⇒ 四条边的最外一圈像素都必须**仍然是纯白**。
    // 判据落在 RGB 上而不是 alpha —— 见本函数的文档。
    let mut dim = Vec::new();
    for y in 0..SIZE {
        for x in 0..SIZE {
            if x == 0 || y == 0 || x == SIZE - 1 || y == SIZE - 1 {
                let p = at(x, y);
                if p[0] < 250 || p[1] < 250 || p[2] < 250 {
                    dim.push((x, y, [p[0], p[1], p[2]]));
                }
            }
        }
    }
    assert!(
        dim.is_empty(),
        "边缘有 {} 个像素不是纯白，前几个：{:?} —— \
         遮罩被无条件施加了（直角图像四周会发暗一圈）",
        dim.len(),
        &dim[..dim.len().min(6)]
    );
}

/// 半透明图元叠在不透明背景上，结果必须是**预乘 over** 的结果。
///
/// # 这条钉住的是混合因子的 `ONE`
///
/// 顶点色已在 `quad::Vertex::quad` 里乘过自己的 alpha，所以 `SrcOver` 的颜色因子
/// 必须是 `ONE`。若沿用 `SRC_ALPHA`，D3D 会**再乘一次** alpha：
///
/// ```text
///   正确（ONE）        : 0.5·1.0 + 0.0·0.5 = 0.50   ⇒ 128
///   双重预乘（SRC_ALPHA）: 0.5·1.0·0.5 = 0.25        ⇒  64
/// ```
///
/// 而 SrcOver 在**非预乘**管线下的结果恰好也是 0.50 —— 也就是说这条测试同时排除
/// 了两种错法，只接受"顶点预乘 + 因子 ONE"这一种组合。
#[test]
fn translucent_quad_composites_in_premultiplied_space() {
    let Ok(gpu) = Gpu::new() else {
        eprintln!("跳过：建不出 D3D11 设备");
        return;
    };
    let (rtv, tex) = offscreen_target(&gpu);
    let mut renderer = Renderer::new(&gpu.device).expect("渲染器应当能建出来");
    let textures = TextureTable::new();

    let s = SIZE as f32;
    let mut list = DisplayList::new();
    // 不透明蓝底
    list.push(
        SortKey {
            layer: zephyra_core::paint::Layer::Background,
            z: 0,
            order: 0,
        },
        Command::Quad {
            rect: Rect::new(0.0, 0.0, s, s),
            radius: CornerRadius { radius: 0.0 },
            fill: Color::rgb(0.0, 0.0, 1.0),
            stroke: None,
            filter: zephyra_core::filter::FilterChain::none(),
            blend: zephyra_core::filter::BlendMode::default(),
        },
    );
    // 50% 白覆盖全屏
    list.push(
        SortKey {
            layer: zephyra_core::paint::Layer::Content,
            z: 0,
            order: 1,
        },
        Command::Quad {
            rect: Rect::new(0.0, 0.0, s, s),
            radius: CornerRadius { radius: 0.0 },
            fill: Color::rgba(1.0, 1.0, 1.0, 0.5),
            stroke: None,
            filter: zephyra_core::filter::FilterChain::none(),
            blend: zephyra_core::filter::BlendMode::default(),
        },
    );
    renderer.render(RenderCtx {
        context: &gpu.context,
        device: &gpu.device,
        rtv: &rtv,
        list: &list,
        size: (SIZE, SIZE),
        clear: Color::rgb(0.0, 0.0, 0.0),
        textures: &textures,
        damage: zephyra_core::frame::Damage::Full,
    });

    let px = read_back(&gpu, &tex);
    let at = |x: u32, y: u32| -> [u8; 4] {
    // ⚠ 离屏 RT 是 `B8G8R8A8`，而 `Color` 是 RGBA —— **通道顺序是反的**。
    // 按 RGB 去读会把红蓝对调，于是「期望 128 的那个通道」实际读的是另一个通道，
    // 断言看起来在管颜色、其实管错了通道。

        let i = (y as usize * SIZE as usize + x as usize) * 4;
        [px[i + 2], px[i + 1], px[i], px[i + 3]] // 重排成 (r, g, b, a)
    };
    for (x, y) in [(8u32, 8u32), (SIZE / 2, SIZE / 2), (SIZE - 9, SIZE - 9)] {
        let p = at(x, y);
        // r/g = 0.5 纯白压在纯蓝上 ⇒ 128；b = 0.5 + 0.5 = 1.0 ⇒ 255；a 不透明 ⇒ 255
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
            "a 应为 255（不透明底 + 半透明顶 ⇒ 0.5 + 0.5），实得 {p:?}"
        );
    }
}

/// 着色器资产必须与源码同步。
///
/// # 为什么这条值得存在
///
/// GPU 加载的是**预编译的 `.bin`**，不是 `.hlsl`。所以「改了 `.hlsl` 但忘了重跑
/// `fxc`」这件事**没有任何测试会失败** —— 画面照旧跑的是旧着色器，而所有断言都
/// 针对那份旧代码的结果。症状是「我明明改了着色器，怎么没效果」，能查的方向有
/// 一大堆（绑定、状态、输入布局），而真正的病因是**一个没执行的命令**。
///
/// 这不是假设：本次做预乘转换时，我用变异测试去改 `.hlsl`，结果 **SURVIVED** ——
/// 正是这个原因。
///
/// 两条断言：
/// 1. 两份 `.hlsl`（D3D11 与 D3D12）必须**逐字节相同**。两边各存一份，改了其中
///    一份忘了另一份，同样是无声失败（两个后端画出不同的东西）。
/// 2. `.bin` 的修改时间**不得早于** `.hlsl`。用 `>=` 而不是 `>`：全新 clone 里两者
///    同一时刻落地，`>` 会误报。
#[test]
fn shader_assets_are_in_sync() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("zephyra-winit");
    let hlsl_a = root.join("shaders").join("zephyra.hlsl");
    let hlsl_b = root
        .parent()
        .unwrap()
        .join("zephyra-winit-d3d12")
        .join("shaders")
        .join("zephyra.hlsl");

    let a = std::fs::read(&hlsl_a).expect("读得到 D3D11 的 hlsl");
    let b = std::fs::read(&hlsl_b).expect("读得到 D3D12 的 hlsl");
    assert_eq!(
        a.len(),
        b.len(),
        "两份 zephyra.hlsl 长度不同 —— 有一份被改过而另一份没跟上"
    );
    for (i, (x, y)) in a.iter().zip(b.iter()).enumerate() {
        assert_eq!(x, y, "两份 zephyra.hlsl 在第 {i} 个字节处分叉（第 4 行的 Viewstage 引用之类）");
    }

    for bin_name in ["zephyra.ps.bin", "zephyra.vs.bin"] {
        let bin = root.join("shaders").join(bin_name);
        let src_time = std::fs::metadata(&hlsl_a)
            .expect("拿得到 hlsl 的修改时间")
            .modified()
            .expect("modified() 可用");
        let bin_time = std::fs::metadata(&bin)
            .unwrap_or_else(|e| panic!("读得到 {bin_name}：{e}"))
            .modified()
            .expect("modified() 可用");
        assert!(
            bin_time >= src_time,
            "{bin_name} 比 zephyra.hlsl 旧 —— 改了着色器但忘了重新编译。\
             命令：fxc /nologo /T ps_5_0 /E PSMain /Fo zephyra.ps.bin zephyra.hlsl（VS 用 vs_5_0 / VSMain）"
        );
    }
}


/// 纯色背景 + 带指定混合模式的纯色前景。
///
/// ⚠ 只能挑**系数可表达**的那 15 个（`Clear`..`Screen`）。`Multiply` / `Overlay` 这类
/// 属于「可分离」那批，需要片元着色器变体（E4.2），现在会**故意**退回 `SrcOver` ——
/// 拿它写测试会得到"混合没生效"的结论，而那其实是预期行为。
fn blend_probe(mode: zephyra_core::filter::BlendMode) -> DisplayList {
    let s = SIZE as f32;
    let mut list = DisplayList::new();
    list.push(
        SortKey {
            layer: zephyra_core::paint::Layer::Background,
            z: 0,
            order: 0,
        },
        Command::Quad {
            rect: Rect::new(0.0, 0.0, s, s),
            radius: CornerRadius { radius: 0.0 },
            // 中灰：与红色相乘得"红的一半"，与相加得 (255,128,128)，都和 SrcOver 差得开。
            fill: Color::rgb(0.5, 0.5, 0.5),
            stroke: None,
            filter: zephyra_core::filter::FilterChain::none(),
            blend: zephyra_core::filter::BlendMode::default(),
        },
    );
    list.push(
        SortKey {
            layer: zephyra_core::paint::Layer::Content,
            z: 0,
            order: 1,
        },
        Command::Quad {
            rect: Rect::new(0.0, 0.0, s, s),
            radius: CornerRadius { radius: 0.0 },
            fill: Color::rgb(1.0, 0.0, 0.0),
            stroke: None,
            filter: zephyra_core::filter::FilterChain::none(),
            blend: mode,
        },
    );
    list.sort();
    list
}

/// 把一条列表渲染到离屏并回读。
fn render_probe(list: &DisplayList) -> Vec<u8> {
    render_probe_with(list, Color::rgb(0.0, 0.0, 0.0))
}

/// 同上，但 clear 色可指定。
///
/// ⚠ clear 默认是**不透明**黑（alpha=1），所以画布的 dst alpha 恒为 1 ——
/// 那样任何 s·ad 形式的 alpha 判据都会退化成 s，两种映射分不开。要测
/// alpha 因子就得让 clear 的 alpha 为 0。
fn render_probe_with(list: &DisplayList, clear: Color) -> Vec<u8> {
    let Ok(gpu) = Gpu::new() else {
        panic!("建不出 D3D11 设备");
    };
    let (rtv, tex) = offscreen_target(&gpu);
    let mut renderer = Renderer::new(&gpu.device).expect("渲染器应当能建出来");
    let textures = TextureTable::new();
    renderer.render(RenderCtx {
        context: &gpu.context,
        device: &gpu.device,
        rtv: &rtv,
        list,
        size: (SIZE, SIZE),
        clear,
        textures: &textures,
        damage: zephyra_core::frame::Damage::Full,
    });
    read_back(&gpu, &tex)
}

/// 离屏 RT 是 **BGRA**，`Color` 是 RGBA —— 重排成 `(r, g, b, a)` 再断言。
fn probe_at(px: &[u8], x: u32, y: u32) -> [u8; 4] {
    let i = (y as usize * SIZE as usize + x as usize) * 4;
    [px[i + 2], px[i + 1], px[i], px[i + 3]]
}

/// 三个采样点：左上、中心、右下（避开抗锯齿带）。
const PROBE_POINTS: [(u32, u32); 3] = [(8, 8), (32, 32), (55, 55)];

/// `Modulate (0, Cs)` 必须真的逐通道相乘。
///
/// # 为什么这条能区分它与"没生效"
///
/// 背景 50% 灰、源纯红：
///
/// ```text
///   Modulate  1.0·0.5 = 0.5 ⇒ 128      要的行为
///   SrcOver   纯红覆盖 ⇒ 255           效果没生效 / 被静默忽略
/// ```
///
/// 两者差得很开，所以这条断言**不可能**在"混合状态没生效"时也通过 —— 而后者正是
/// E4.1 之前的状态。
#[test]
fn modulate_blend_is_applied_on_gpu() {
    let px = render_probe(&blend_probe(zephyra_core::filter::BlendMode::Modulate));
    for (x, y) in PROBE_POINTS {
        let p = probe_at(&px, x, y);
        assert!(
            (p[0] as i32 - 128).abs() <= 3,
            "Modulate 下红通道应是 0.5（128）；SrcOver 会是 255 —— 实得 {p:?} @({x},{y})"
        );
        assert!(
            p[1] <= 3 && p[2] <= 3,
            "绿蓝应被乘成 0，实得 {p:?} @({x},{y})"
        );
    }
}

/// `Plus (1, 1)` 必须真的相加（并被钳到 1）。
///
/// 选它是因为结果**只可能**来自加法：`(255, 128, 128)`。SrcOver 会给纯红 `(255,0,0)`，
/// Modulate 给 `(128,0,0)` —— 三者两两不同，所以这条断言对"用错系数"也敏感。
#[test]
fn plus_blend_is_applied_on_gpu() {
    let px = render_probe(&blend_probe(zephyra_core::filter::BlendMode::Plus));
    for (x, y) in PROBE_POINTS {
        let p = probe_at(&px, x, y);
        assert!(
            p[0] >= 252 && (p[1] as i32 - 128).abs() <= 3 && (p[2] as i32 - 128).abs() <= 3,
            "Plus 应得 (255,128,128)：红溢出、绿蓝相加。实得 {p:?} @({x},{y})"
        );
    }
}

/// `Clear` 必须把目标清成全零（含 alpha）—— 它是唯一**不读** `dst` 的模式。
#[test]
fn clear_blend_zeroes_the_target() {
    let px = render_probe(&blend_probe(zephyra_core::filter::BlendMode::Clear));
    for (x, y) in PROBE_POINTS {
        assert_eq!(
            probe_at(&px, x, y),
            [0, 0, 0, 0],
            "Clear 之后应是全 0（含 alpha）@({x},{y})"
        );
    }
}

/// `Dst (0, 1)` 只保留目标 —— 前景被完全丢弃。
///
/// 与 `Src` 合起来钉住两端：`Src` 得纯源色、`Dst` 得纯背景色。两条一起就说明
/// 系数表的两端没被写反（写反的话这两条会**互换**，各自单独都能通过一半）。
#[test]
fn src_and_dst_blends_are_distinct() {
    let src = render_probe(&blend_probe(zephyra_core::filter::BlendMode::Src));
    let dst = render_probe(&blend_probe(zephyra_core::filter::BlendMode::Dst));
    let (sp, dp) = (probe_at(&src, 32, 32), probe_at(&dst, 32, 32));
    assert!(
        sp[0] >= 252 && sp[1] <= 3 && sp[2] <= 3,
        "Src 应得纯红 (255,0,0)，实得 {sp:?}"
    );
    assert!(
        (dp[0] as i32 - 128).abs() <= 3 && (dp[1] as i32 - 128).abs() <= 3,
        "Dst 应得背景灰 (128,128,128)，实得 {dp:?}"
    );
}

/// 相邻几段混合模式不同 ⇒ 逐段换状态，每一段的效果都要生效。
///
/// # 这条钉住的是 E3 那条接缝
///
/// 以前绘制循环把三段写死、整帧套一个混合状态，"两段不同模式"这个情形根本表达不了。
/// 现在按 `runs` 走，所以这一条同时验证：GPU 结果正确、`runs` 的切分在真实回放里
/// 真的被用上、以及**顺序**没被打乱（重排会改变半透明叠加的可见结果，所以本实现
/// 只合并相邻段、绝不重排）。
#[test]
fn adjacent_runs_with_different_blends_all_apply_in_order() {
    let s = SIZE as f32;
    let mut list = DisplayList::new();
    let mut push = |order: u32,
                    fill: Color,
                    blend: zephyra_core::filter::BlendMode,
                    layer: zephyra_core::paint::Layer| {
        list.push(
            SortKey {
                layer,
                z: 0,
                order,
            },
            Command::Quad {
                rect: Rect::new(0.0, 0.0, s, s),
                radius: CornerRadius { radius: 0.0 },
                fill,
                stroke: None,
                filter: zephyra_core::filter::FilterChain::none(),
                blend,
            },
        );
    };
    // 灰底 → Modulate 红 ⇒ (0.5,0,0) → Plus 绿 ⇒ (0.5,1,0) ⇒ (128,255,0)
    push(0, Color::rgb(0.5, 0.5, 0.5), zephyra_core::filter::BlendMode::default(), zephyra_core::paint::Layer::Background);
    push(1, Color::rgb(1.0, 0.0, 0.0), zephyra_core::filter::BlendMode::Modulate, zephyra_core::paint::Layer::Content);
    push(2, Color::rgb(0.0, 1.0, 0.0), zephyra_core::filter::BlendMode::Plus, zephyra_core::paint::Layer::Overlay);
    list.sort();

    let px = render_probe(&list);
    let p = probe_at(&px, 32, 32);
    assert!(
        (p[0] as i32 - 128).abs() <= 3 && p[1] >= 252 && p[2] <= 3,
        "三段应当依次生效、顺序不变，最终是 (128,255,0)。\
         若中间那两段被整帧的 SrcOver 吃掉，会得到 (0,255,0) 或 (255,0,0)。实得 {p:?}"
    );
}

/// 尚不能表达的混合模式必须**退回 SrcOver**，且**照常画**。
///
/// # 为什么这条重要
///
/// 退化方式选"退回 SrcOver"而不是"跳过这一段"：跳过会让元素凭空消失，而"效果没
/// 生效"至少有迹可循（画出来颜色不对）。这是 fx 层早就定下的约定
/// （`RenderReport::unimplemented_*` 那段注释），这里把它钉住。
#[test]
fn unsupported_blend_falls_back_to_src_over_and_still_draws() {
    // `Multiply` 属于「可分离」那批（E4.2 才做），现在必然退回。
    let px = render_probe(&blend_probe(zephyra_core::filter::BlendMode::Multiply));
    let p = probe_at(&px, 32, 32);
    assert!(
        p[0] >= 252 && p[1] <= 3 && p[2] <= 3,
        "Multiply 尚未实现，应退回 SrcOver 得纯红 (255,0,0) —— \
         元素必须**画出来**，不能被跳过。实得 {p:?}"
    );
}

/// `Modulate` 的 **alpha 通道**必须走 alpha 对偶（`Cs → as`），不是颜色系数。
///
/// # 为什么这条单独存在
///
/// `Modulate (0, Cs)` 是 15 个系数模式里**唯一**用到 `SrcColor` 的，所以只有它的
/// alpha 因子会经过 [`d3d11_alpha_factor`] 那个映射。而 RGB 断言**看不到**这个映射
/// 写没写错 —— 两边算出来的颜色一模一样。
///
/// 判据要**半透明背景**才分得开：
///
/// ```text
///   背景 alpha = 0.5，源 alpha = 1
///   正确  a = 0·as + as·ad = 0.5·1 = 0.5 ⇒ 128
///   错(ONE) a = 0·as + 1·ad = 1·0.5     = 1.0 ⇒ 255
/// ```
///
/// 背景与源都不透明时两者都是 1，所以那条 RGB 测试怎么写都抓不到。
#[test]
fn modulate_alpha_uses_the_alpha_analogue_not_the_color_factor() {
    let s = SIZE as f32;
    let mut list = DisplayList::new();
    // 半透明灰底：压在黑 clear 上 ⇒ 合成后 alpha = 0.5
    list.push(
        SortKey {
            layer: zephyra_core::paint::Layer::Background,
            z: 0,
            order: 0,
        },
        Command::Quad {
            rect: Rect::new(0.0, 0.0, s, s),
            radius: CornerRadius { radius: 0.0 },
            fill: Color::rgba(0.5, 0.5, 0.5, 0.5),
            stroke: None,
            filter: zephyra_core::filter::FilterChain::none(),
            blend: zephyra_core::filter::BlendMode::default(),
        },
    );
    // 不透明红，Modulate
    list.push(
        SortKey {
            layer: zephyra_core::paint::Layer::Content,
            z: 0,
            order: 1,
        },
        Command::Quad {
            rect: Rect::new(0.0, 0.0, s, s),
            radius: CornerRadius { radius: 0.0 },
            // 源必须**半透明**：as=1 时 s·ad 与 d 数值相同，两种映射分不开。
            fill: Color::rgba(1.0, 0.0, 0.0, 0.5),
            stroke: None,
            filter: zephyra_core::filter::FilterChain::none(),
            blend: zephyra_core::filter::BlendMode::Modulate,
        },
    );
    list.sort();

    // clear 与源都必须**半透明**：`as == 1` 时 `as·ad` 与 `ad` 数值相同，
    // 两种 alpha 映射分不开；`ad == 1`（不透明 clear）同理。
    let px = render_probe_with(&list, Color::rgba(0.0, 0.0, 0.0, 0.0));
    let p = probe_at(&px, 32, 32);
    assert!(
        (p[3] as i32 - 64).abs() <= 4,
        "Modulate 的 alpha 应是 as·ad = 0.5·0.5 = 0.25（64）；\
         若 alpha 通道把 SrcColor 当成 ONE，就会得到 ad = 0.5（128）。实得 {p:?}"
    );
    // 颜色侧顺带复核。期望 32：半透明灰在**预乘**空间里是 0.25，压在透明 clear 上
    // 之后 dst 仍是 0.25；半透明红预乘后是 0.5，相乘得 0.125。
    assert!(
        (p[0] as i32 - 32).abs() <= 4 && p[1] <= 4 && p[2] <= 4,
        "颜色侧应是 (32,0,0)（源 0.5 红 × 底 0.25 灰），实得 {p:?}"
    );
}

/// 多边形（`Command::Path`）必须真的画出来。
///
/// # 这条钉住的是索引基址换算
///
/// `runs` 记的是**顶点**下标，而多边形的索引区在 `path_index_start` **之后**，
/// 且顶点↔索引是 1:1（不像四边形要 `×6/÷4`）。这一支只被图标用，而图标在
/// 单测里从不进 GPU 测试 —— 于是"漏加 `path_index_start`"这种错误**没有任何测试
/// 会失败**：顶点照样上传、统计照样正常，只是多边形不出现。
#[test]
fn path_geometry_is_rendered() {
    use std::sync::Arc;
    let s = SIZE as f32;
    // 一个铺满大半屏的三角形，顶点按逆时针给（`signed_area2` 非零即可）。
    let verts: Arc<[zephyra_core::paint::Vertex]> = Arc::from(vec![
        zephyra_core::paint::Vertex {
            position: zephyra_core::core::Point {
                x: s * 0.1,
                y: s * 0.1,
            },
            uv: zephyra_core::core::Point { x: 0.0, y: 0.0 },
        },
        zephyra_core::paint::Vertex {
            position: zephyra_core::core::Point {
                x: s * 0.9,
                y: s * 0.1,
            },
            uv: zephyra_core::core::Point { x: 0.0, y: 0.0 },
        },
        zephyra_core::paint::Vertex {
            position: zephyra_core::core::Point {
                x: s * 0.1,
                y: s * 0.9,
            },
            uv: zephyra_core::core::Point { x: 0.0, y: 0.0 },
        },
    ]);
    let mut list = DisplayList::new();
    // ⚠ 必须先有一个四边形：多边形的索引起点是 path_index_start，而它等于前面
    // 四边形的索引总数。列表里只有多边形时它是 0，于是
    // 「漏加 path_index_start」这个错误**数值上等价于正确写法**，测不出来。
    list.push(
        SortKey {
            layer: zephyra_core::paint::Layer::Background,
            z: 0,
            order: 0,
        },
        Command::Quad {
            rect: Rect::new(0.0, 0.0, s, s),
            radius: CornerRadius { radius: 0.0 },
            // 必须**不透明**：fill.a == 0 的四边形会被 ill.a <= 0.0 直接跳过，
            // 于是索引流里仍然没有四边形，path_index_start 还是 0。
            fill: Color::rgb(0.0, 0.0, 1.0),
            stroke: None,
            filter: zephyra_core::filter::FilterChain::none(),
            blend: zephyra_core::filter::BlendMode::default(),
        },
    );
    list.push(
        SortKey {
            layer: zephyra_core::paint::Layer::Content,
            z: 0,
            order: 1,
        },
        Command::Path {
            vertices: verts,
            fill: Color::rgb(0.0, 1.0, 0.0),
            filter: zephyra_core::filter::FilterChain::none(),
            blend: zephyra_core::filter::BlendMode::default(),
        },
    );
    list.sort();

    let px = render_probe_with(&list, Color::rgba(0.0, 0.0, 0.0, 0.0));
    // 三角形内部（约 1/3 处）必须是绿的；外部必须仍是透明。
    let inside = probe_at(&px, 22, 22);
    assert!(
        inside[1] >= 200 && inside[0] <= 40 && inside[2] <= 40,
        "三角形内部应是纯绿，实得 {inside:?}"
    );
    let outside = probe_at(&px, 58, 58);
    assert_eq!(
        outside,
        [0, 0, 255, 255],
        "三角形外应是不透明蓝底（索引区间算错时这里会被三角形盖住）"
    );
}

/// 半透明 clear 色必须**预乘**后写进目标。
    ///
    /// # 为什么这条能抓不到回归，而 SrcOver 抓不到
    ///
    /// 预乘与非预乘两种管线在 `SrcOver` 下**结果完全相同**（非预乘那一版靠
    /// D3D 的 `SRC_ALPHA` 因子补上那一次乘法），所以任何画一个矩形的像素测试都
    /// 分辨不出这次转换。clear 是唯一一处**没有混合方程兜底**的地方 ——
    /// 它直接写进 RT，于是预乘与否立刻可见。
    ///
    /// 这也解释了为什么本次转换后 23 个 GPU 测试与 demo 基线**全部不变**：变化
    /// 只在"非 SrcOver 的混合模式"与"半透明 clear"上可见，而前者要等 E4.1。
    #[test]
    fn clear_color_is_premultiplied() {
        let Ok(gpu) = Gpu::new() else {
            eprintln!("跳过：建不出 D3D11 设备");
            return;
        };
        let (rtv, tex) = offscreen_target(&gpu);
        let mut renderer = Renderer::new(&gpu.device).expect("渲染器应当能建出来");
        let textures = TextureTable::new();

        // 一条空列表：什么都不画，只剩 clear。
        let list = DisplayList::new();
        renderer.render(RenderCtx {
            context: &gpu.context,
            device: &gpu.device,
            rtv: &rtv,
            list: &list,
            size: (SIZE, SIZE),
            clear: Color::rgba(1.0, 1.0, 1.0, 0.5),
            textures: &textures,
            damage: zephyra_core::frame::Damage::Full,
        });

        let px = read_back(&gpu, &tex);
        let at = |x: u32, y: u32| -> [u8; 4] {
            let i = (y as usize * SIZE as usize + x as usize) * 4;
            [px[i], px[i + 1], px[i + 2], px[i + 3]]
        };
        for (x, y) in [(0u32, 0u32), (SIZE / 2, SIZE / 2), (SIZE - 1, SIZE - 1)] {
            let p = at(x, y);
            assert!(
                (p[0] as i32 - 128).abs() <= 2 && (p[1] as i32 - 128).abs() <= 2,
                "clear(1,1,1,0.5) 预乘后 rgb 约为 128（未预乘会是 255），实得 {p:?} @({x},{y})"
            );
            assert!(
                (p[3] as i32 - 128).abs() <= 2,
                "alpha 应仍是 128，实得 {p:?} @({x},{y})"
            );
        }
    }

/// 建一张**内容确定**的纯白纹理，供图像圆角那两条测试用。
///
/// # 为什么不用 [`zephyra_winit::texture::create_rgba_texture`]
///
/// 那条路走 `DEFAULT` + `UpdateSubresource`，而**本机上它不可靠** —— 第一次写这
/// 条测试时，中心像素在三次运行里分别是 `[7,9,239]`、`[78,9,186]`、`[4,6,229]`：
/// alpha 随机，于是 `clip(a - 0.0039)` 把绝大多数片元剔掉，画面整片是清屏色。
///
/// 而「纹理是垃圾」与「SDF 遮罩没生效」的症状**完全一样**（都是图像看不见），
/// 排查方向会被带到完全错误的地方 —— 与本仓反复记录的「失败模式不报错」同类。
///
/// `IMMUTABLE` + 初始数据把驱动从「要正确处理一次拷贝」里摘出来，是本机上唯一
/// 确定的建纹理方式。代价是内容不可再改 —— 测试里正需要这种不可变性。
fn white_texture(gpu: &Gpu, w: u32, h: u32) -> ID3D11Texture2D {
    use windows::Win32::Graphics::Direct3D11::*;
    use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_SAMPLE_DESC};
    let pixels = vec![255u8; (w * h * 4) as usize];
    let desc = D3D11_TEXTURE2D_DESC {
        Width: w,
        Height: h,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_R8G8B8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_IMMUTABLE,
        BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let data = D3D11_SUBRESOURCE_DATA {
        pSysMem: pixels.as_ptr() as *const _,
        SysMemPitch: w * 4,
        SysMemSlicePitch: 0,
    };
    let mut tex = None;
    unsafe {
        gpu.device
            .CreateTexture2D(&desc, Some(&data), Some(&mut tex as *mut _))
    }
    .expect("应当能建 IMMUTABLE 纹理");
    tex.expect("CreateTexture2D 成功时必给纹理")
}

/// 建一张离屏渲染目标。
fn offscreen_target(
    gpu: &Gpu,
) -> (ID3D11RenderTargetView, ID3D11Texture2D) {
    use windows::Win32::Graphics::Direct3D11::*;
    use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
    let desc = D3D11_TEXTURE2D_DESC {
        Width: SIZE,
        Height: SIZE,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let mut tex = None;
    unsafe { gpu.device.CreateTexture2D(&desc, None, Some(&mut tex as *mut _)) }
        .expect("应当能建离屏纹理");
    let tex = tex.expect("CreateTexture2D 成功时必给纹理");
    let mut rtv = None;
    unsafe { gpu.device.CreateRenderTargetView(&tex, None, Some(&mut rtv as *mut _)) }
        .expect("应当能给离屏纹理建 RTV");
    (rtv.expect("CreateRenderTargetView 成功时必给视图"), tex)
}

/// 把渲染目标复制到 STAGING 纹理并读回**紧密排列**的 BGRA（自顶向下）。
fn read_back(gpu: &Gpu, tex: &ID3D11Texture2D) -> Vec<u8> {
    use windows::Win32::Graphics::Direct3D11::*;
    use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
    let desc = D3D11_TEXTURE2D_DESC {
        Width: SIZE,
        Height: SIZE,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_STAGING,
        BindFlags: 0,
        CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
        MiscFlags: 0,
    };
    let mut stg = None;
    unsafe { gpu.device.CreateTexture2D(&desc, None, Some(&mut stg as *mut _)) }
        .expect("应当能建 STAGING 纹理");
    let stg = stg.expect("CreateTexture2D 成功时必给纹理");
    unsafe {
        gpu.context.CopyResource(&stg, tex);
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        gpu.context
            .Map(&stg, 0, D3D11_MAP_READ, 0, Some(&mut mapped as *mut _))
            .expect("STAGING 纹理应当能 Map(READ)");
        let pitch = mapped.RowPitch as usize;
        let mut out = vec![0u8; pitch * SIZE as usize];
        std::ptr::copy_nonoverlapping(mapped.pData as *const u8, out.as_mut_ptr(), out.len());
        gpu.context.Unmap(&stg, 0);
        // 去掉行尾填充 —— 驱动的 `RowPitch` 可能大于 `SIZE * 4`。
        out.chunks(pitch)
            .flat_map(|row| row[..SIZE as usize * 4].to_vec())
            .collect()
    }
}

/// 数"不是黑色"的像素 —— 这才是覆盖率。
///
/// 判据必须是"**不是清屏色**"而不是某个具体颜色：图元颜色有 SDF 抗锯齿，
/// 边缘像素是过渡值，只有"等于清屏色"才是确定的"没被画过"。
fn non_background(pixels: &[u8]) -> usize {
    pixels
        .chunks_exact(4)
        .filter(|p| !(p[0] == 0 && p[1] == 0 && p[2] == 0))
        .count()
}

/// 颜色分布 —— 失败时用它一眼看出"画成了什么"。
fn color_hist(pixels: &[u8]) -> std::collections::BTreeMap<[u8; 4], usize> {
    let mut m: std::collections::BTreeMap<[u8; 4], usize> = Default::default();
    for p in pixels.chunks_exact(4) {
        *m.entry([p[0], p[1], p[2], p[3]]).or_default() += 1;
    }
    m
}

/// 建 `DYNAMIC` + `CPU_ACCESS_WRITE` 缓冲，**只给容量**（内容稍后 `Map` 写）。
///
/// 容量故意开得比数据大，与渲染器完全一致 —— 否则测的就不是渲染器实际在跑的那条路。
fn dynamic_buffer_sized(device: &ID3D11Device, bind: D3D11_BIND_FLAG, capacity: u32) -> ID3D11Buffer {
    use windows::Win32::Graphics::Direct3D11::*;
    let desc = D3D11_BUFFER_DESC {
        ByteWidth: capacity,
        Usage: D3D11_USAGE_DYNAMIC,
        BindFlags: bind.0 as u32,
        CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
        MiscFlags: 0,
        StructureByteStride: 0,
    };
    let mut out = None;
    unsafe { device.CreateBuffer(&desc, None, Some(&mut out as *mut _)) }
        .expect("应当能建动态缓冲");
    out.expect("CreateBuffer 成功时必给缓冲")
}

/// 用 `Map(WRITE_DISCARD)` 写入 —— 与渲染器同一条路。
fn map_write(context: &ID3D11DeviceContext, buffer: &ID3D11Buffer, bytes: &[u8]) {
    use windows::Win32::Graphics::Direct3D11::*;
    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    unsafe {
        context
            .Map(buffer, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut mapped as *mut _))
            .expect("动态缓冲应当能 Map");
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), mapped.pData.cast::<u8>(), bytes.len());
        context.Unmap(buffer, 0);
    }
}

use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_FLAG, D3D11_BIND_INDEX_BUFFER, D3D11_BIND_VERTEX_BUFFER, D3D11_VIEWPORT,
    ID3D11Buffer, ID3D11DepthStencilView, ID3D11Device, ID3D11DeviceContext,
    ID3D11RenderTargetView, ID3D11Texture2D,
};
// ⚠ 拓扑常量住在 `Direct3D`（D3D9 起共用的枚举），**不在** `Direct3D11` 下。
use windows::Win32::Graphics::Direct3D::D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_R16_UINT;

// ---------------------------------------------------------------------------
// 视口在本机**读回恒为 0×0** —— 原因未确定
// ---------------------------------------------------------------------------
//
// # 现象
//
// `RSSetViewports` 调用后立刻 `RSGetViewports`，读回来恒为「个数 0、宽高全 0」。
// D3D11 的默认视口正是 0×0，于是所有三角形都是零面积 → 一个像素都不画，**且不报
// 任何错**。
//
// 裸 D3D11 那个对照组里唯一被画出的 2 个像素，就是两个退化三角形各自落在原点的
// 那一个边角（`[255,0,0,128]` 与 `[0,0,255,128]`，alpha 都是 128 —— 是抗锯齿边缘，
// 不是图元内部的像素）。
//
// 补充：`RSSetViewports` 之后调试层消息队列**没有任何新增** —— 驱动对丢弃视口
// 一声不吭。（调试层本身原先是坏的，两处都是我的 bug：`pMessageByteLength` 传了
// 未初始化的 0，以及按早期 SDK 的 `D3D11_MESSAGE` 布局解析。修好后它能说话了，
// 也正是它说出了下面表里那两条。见 `tests/debug_layer.rs`。）
//
// # 已排除的可能（每一条都做过实验）
//
// | 假设 | 实验 | 结果 |
// |---|---|---|
// | 临时数组被提前 drop | 换成具名数组 | 仍为 0 |
// | `windows-rs` 0.62 的绑定传参有误 | 手写 vtable 直调该槽 | 仍为 0 |
// | vtable 签名不对 | 核对 `fn(*mut c_void, u32, *const D3D11_VIEWPORT)` | 与 C API 一致 |
// | 上下文状态机制整体失效 | `RSSetState`/`RSGetState` 往返 | **正常**（读到我们设的值） |
// | 调用时机不对 | 裸设 / 设两个 / 绑 RT 之后再设 | 三种全是 0 |
//
// 最后两行是关键：`RSSetState` 在**同一个上下文**上往返正常，而视口独立失效。
// 所以既不是绑定层、不是上下文、也不是「上下文状态机制」整体不可用 —— 只剩视口这一项，
// 且它在任何调用时机下都不生效。
//
// # 一条必须记下的假信号
//
// 排查过程中我临时插入过一段探查代码（多调一次 `OMSetRenderTargets`，并设了一个
// `ScissorEnable=1` 的光栅化状态）。**只要那段代码在，对照测试就"通过"了** ——
// 覆盖 4096/4096。
//
// 那是副作用，不是理解：把探查代码删掉，立刻回到 0/4096。所以任何"修好了"的结论
// 都必须在**干净状态**下复现一遍，否则测的其实是自己刚加的那段代码。
//
// # 与本项目症状的对应
//
// - `ClearRenderTargetView` 正常 —— 它不看视口；
// - 顶点/索引缓冲逐项读回全部正确 —— 数据确实是对的；
// - 每个四边形恰好留 1 个半透明像素 —— 退化三角形的边缘；
// - 换成只用 `SV_VertexID`、画覆盖全视口的三角形 → **0 像素**
//   （此时顶点数据与输入布局已被完全排除）。
//
// # ⚠ 三条曾经归给「环境」的症状，其实是本 crate 自己的 bug
//
// 这一段是本文件最该留下的东西。同一个模式连犯三次：**把自己写的非法参数造成的
// 失败，当成了环境的静默失效。**
//
// | 曾经的结论 | 实际 | 驱动 / 文档怎么说 |
// |---|---|---|
// | D3D11 `DYNAMIC` 资源建不出来 | ❌ `DYNAMIC` + `CPUAccessFlags: 0` 是**非法组合** | "A D3D11_USAGE_DYNAMIC Resource may only have the D3D11_CPU_ACCESS_WRITE CPUAccessFlags set." |
// | 调试层不可用，连原因都问不到 | ❌ `pMessageByteLength` 传了未初始化的 **0**；`D3D11_MESSAGE` 按早期 SDK 布局解析 | 当前 SDK 与 windows-rs 都是 `Category@0 Severity@4 ID@8 pDescription@16 DescriptionByteLength@24` |
// | `CreateVertexShader` 拒我的字节码 | ⚠ 一半：SM 5.1 确实只到 5_0，但**是我把两个 crate 一起改成了 5.1** | "Shader must be vs_4_0, vs_4_1, or vs_5_0. Shader version provided: UNRECOGNIZED" |
// | （本轮新发现）`UpdateSubresource` 不可用 | ✅ 现象真实：本机对 `DEFAULT` 缓冲**随机**崩 —— 4096 字节连跑 10 次崩 3 次。这是**另一条独立故障**，不是视口那条 | 见 `tests/update_probe.rs` |
//
// 教训不是「要更小心」，而是：**扫描能告诉你「哪些组合能过」，给不出「为什么」。**
// 于是我顺手把结论归给了环境 —— 而驱动从一开始就在消息队列里写着答案。
// 正确顺序是：**先让调试层能说话，再问驱动。**
//
// # 视口这一条仍然未确定
//
// 上面的更正把「视口 = 同一个病」这个假设消掉了 —— 驱动对丢弃视口不发任何消息，
// 而 `UpdateSubresource` 那条是崩溃（信号很强）、不是静默丢弃。两者没有共同证据。
//
// 另一个 crate 的 zephyra-winit-d3d12 在本机做了一组独立实验，发现**同一套图形栈上
// D3D12 的资源创建整体不可用**：
//
// - 4 块适配器（Intel 核显 x2、NVIDIA MX330、Microsoft Basic Render Driver）全部
//   D3D12CreateDevice 成功、GetDeviceRemovedReason 正常；
// - 但 CreateCommittedResource 全部 E_INVALIDARG —— **包括「上传堆 64 字节缓冲」这种
//   每个字段都只有唯一正确取值的零歧义调用**；
// - 手写 C 原始签名直调 vtable，得到**完全相同**的结果 → 排除 windows-rs ABI 缺陷。
//
// 完整证据链见 zephyra-winit-d3d12/tests/minimal.rs。
//
// （顺带更正：D3D12 那边原本记的「`CreateCommittedResource` 全部 E_INVALIDARG 是
// 环境问题」也是**同一个模式**的第四次 —— 真正的根因是缓冲的
// `D3D12_TEXTURE_LAYOUT` 写成了 `UNKNOWN`，而文档明写 Buffers
// 「Layout must be D3D12_TEXTURE_LAYOUT_ROW_MAJOR」。修掉之后 4/4 适配器都能建。
// 所以「图形栈不完整」这个统一解释现在只剩视口一个候选，且没有任何支持它的直接证据。）
//
// # 怎么区分「我们错了」与「环境不行」
//
// 只有一条路，换一台机器跑。判据现成 ——
// zephyra-winit-d3d12/tests/minimal.rs 的 `d3d12_最小资源能不能建出来`，
// 在一台正常机器上应当通过。
// ---------------------------------------------------------------------------

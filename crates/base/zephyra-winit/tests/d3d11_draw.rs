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

use zephyra::frame::Damage;
use zephyra::core::{Color, CornerRadius, Rect};
use zephyra::paint::{Command, DisplayList, SortKey};

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
/// 无关**的失败原因。离屏纹理是���己建的、不经过 DXGI 的渲染目标，于是出问题时
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
        damage: zephyra::frame::Damage::Full,
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
            layer: zephyra::paint::Layer::Content,
            z: 0,
            order: 0,
        },
        Command::Quad {
            rect: Rect::new(0.0, 0.0, s, s),
            radius: CornerRadius { radius: 0.0 },
            fill: Color::rgb(1.0, 0.0, 0.0),
            stroke: None,
            filter: zephyra::filter::FilterChain::none(),
            blend: zephyra::filter::BlendMode::default(),
        },
    );
    list.push(
        SortKey {
            layer: zephyra::paint::Layer::Content,
            z: 0,
            order: 1,
        },
        Command::Quad {
            rect: Rect::new(0.0, 0.0, s / 2.0, s / 2.0),
            radius: CornerRadius { radius: 0.0 },
            fill: Color::rgb(0.0, 0.0, 1.0),
            stroke: None,
            filter: zephyra::filter::FilterChain::none(),
            blend: zephyra::filter::BlendMode::default(),
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
            layer: zephyra::paint::Layer::Content,
            z: 0,
            order: 0,
        },
        Command::Quad {
            rect: Rect::new(0.0, 0.0, s, s),
            radius: CornerRadius { radius: 0.0 },
            fill: Color::rgb(0.0, 0.0, 1.0),
            stroke: None,
            filter: zephyra::filter::FilterChain::none(),
            blend: zephyra::filter::BlendMode::default(),
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
            layer: zephyra::paint::Layer::Content,
            z: 0,
            order: 0,
        },
        Command::Image {
            texture: zephyra::paint::TextureRef::new("img"),
            src: None,
            dst: Rect::new(4.0, 4.0, s - 8.0, s - 8.0),
            radius: CornerRadius { radius: 16.0 },
            tint: Color::rgb(1.0, 1.0, 1.0),
            filter: zephyra::filter::FilterChain::none(),
            blend: zephyra::filter::BlendMode::default(),
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
        damage: zephyra::frame::Damage::Full,
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
            layer: zephyra::paint::Layer::Content,
            z: 0,
            order: 0,
        },
        Command::Image {
            texture: zephyra::paint::TextureRef::new("img"),
            src: None,
            dst: Rect::new(0.0, 0.0, s, s),
            radius: CornerRadius { radius: 0.0 },
            tint: Color::rgb(1.0, 1.0, 1.0),
            filter: zephyra::filter::FilterChain::none(),
            blend: zephyra::filter::BlendMode::default(),
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
        damage: zephyra::frame::Damage::Full,
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

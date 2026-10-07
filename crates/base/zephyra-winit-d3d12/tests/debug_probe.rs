//! D3D12 调试层到底有没有挂上、有没有消息 —— 不猜。
//!
//! # 为什么需要它
//!
//! `CreateGraphicsPipelineState` 在本机回 `E_INVALIDARG`，而这条错误**不带任何说明**。
//! 调试层是唯一的问法 —— 而我此前**两次**判「调试层不可用」，两次都是自己的 bug：
//!
//! - D3D11 那次：`pMessageByteLength` 传了未初始化的 0，且按早期 SDK 的
//!   `D3D11_MESSAGE` 布局解析；
//! - D3D12 这次的**上一版**：`EnableDebugLayer` 在**建设备之后**才调（文档要求之前），
//!   而且消息队列从**全局** `ID3D12Debug` 上 cast —— 而它只在挂在**设备**上的队列里。
//!
//! 所以这个测试只回答三件事，逐条打印，不做推断：
//!
//! 1. `D3D12GetDebugInterface` 拿不拿得到 `ID3D12Debug`；
//! 2. 设备上 QI 拿不拿得到 `ID3D12InfoQueue`；
//! 3. 造一次必然失败的 PSO 之后，队列里躺着几条消息。
//!
//! 三条都「否」时才能说「调试层不可用」；只要有一条是「是」而消息是 0，那才轮到怀疑
//! 「这个 API 在本机不产生调试消息」。
#![cfg(windows)]

use windows::core::Interface;
use windows::Win32::Graphics::Direct3D::ID3DBlob;
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_FORMAT_UNKNOWN, DXGI_SAMPLE_DESC,
};

/// 平凡顶点着色器：`float4 VSMain(uint id : SV_VertexID) : SV_Position`。
const TRIV_VS: &[u8] = include_bytes!("triv.vs.bin");
/// 平凡像素着色器：`float4 PSMain() : SV_TARGET { return float4(1,1,1,1); }`。
const TRIV_PS: &[u8] = include_bytes!("triv.ps.bin");

#[test]
#[ignore = "诊断用：要真设备 + 调试层。跑法见文件末尾"]
fn 调试层_挂上了吗_有消息吗() {
    // ---- 1. 调试层 ----
    //
    // 必须在建设备**之前**开 —— 文档明写。
    let mut layer = None::<ID3D12Debug>;
    let got_layer = unsafe { D3D12GetDebugInterface(&mut layer) }.is_ok() && layer.is_some();
    println!("  1. D3D12GetDebugInterface → {}", yn(got_layer));
    if got_layer {
        unsafe { layer.as_ref().unwrap().EnableDebugLayer() };
        println!("     EnableDebugLayer 已调（在建设备之前）");
    }
    if !got_layer {
        println!("     ⇒ 这台机器没有调试层。下面全部不成立。");
        return;
    }

    // ---- 2. 设备 ----
    let mut device = None::<ID3D12Device>;
    let made = unsafe {
        D3D12CreateDevice(
            None,
            windows::Win32::Graphics::Direct3D::D3D_FEATURE_LEVEL_11_0,
            &mut device as *mut _,
        )
    };
    println!(
        "  2. D3D12CreateDevice → {}{}",
        yn(made.is_ok()),
        if device.is_some() {
            "（设备已建）"
        } else {
            ""
        }
    );
    let Some(device) = device else { return };

    // ---- 3. 挂在**设备**上的消息队列 ----
    //
    // 这一步是本测试的核心：原先的代码从全局 `ID3D12Debug` 上 cast，拿到的是**空**队列。
    let queue: Option<ID3D12InfoQueue> = device.cast::<ID3D12InfoQueue>().ok();
    println!(
        "  3. 设备 → ID3D12InfoQueue → {}",
        yn(queue.is_some())
    );
    let Some(queue) = queue else {
        println!("     ⇒ 队列拿不到，问不到原因。");
        return;
    };
    let n0: u64 = unsafe { queue.GetNumStoredMessages() };
    println!("     刚建完时队列里有 {n0} 条");

    // ---- 7. 命令列表 / 分配器的真实生命周期 ----
//
// 症状：`begin_frame` 里 `list.Reset` 回 `E_FAIL`，调试层说
// 「Reset fails because the command list was not closed」—— 而 `Gpu::new` 明明
// `Close` 过。同样的代码上一轮却成功，所以是非确定性的。
//
// 与其推断，不如把每一步的 HRESULT 打出来。
let alloc: ID3D12CommandAllocator =
        match unsafe { device.CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT) } {
            Ok(a) => {
                println!("  7. CreateCommandAllocator → ✅");
                a
            }
            Err(e) => {
                println!("  7. CreateCommandAllocator → ❌ {:#X}", e.code().0 as u32);
                return;
            }
        };

    let mk = |tag: &str| {
        let r: windows::core::Result<ID3D12GraphicsCommandList> = unsafe {
            device.CreateCommandList(0, D3D12_COMMAND_LIST_TYPE_DIRECT, &alloc, None)
        };
        println!("     {tag}CreateCommandList → {}", yn(r.is_ok()));
        r
    };
    let Ok(list) = mk("") else { return };

    let step = |tag: &str, r: windows::core::Result<()>| {
        println!(
            "     {tag} → {}{}",
            yn(r.is_ok()),
            match &r {
                Err(e) => format!(" {:#X}", e.code().0 as u32),
                Ok(()) => String::new(),
            }
        );
    };

    step("Close（刚创建）", unsafe { list.Close() });
    step(
        "list.Reset（第一次）",
        unsafe { list.Reset::<&_, Option<&ID3D12PipelineState>>(&alloc, None) },
    );
    step("alloc.Reset", unsafe { alloc.Reset() });
    // 第二轮：模拟 `submit` 之后再 `begin_frame`。
    step("Close（第二次）", unsafe { list.Close() });
    step(
        "list.Reset（第二次）",
        unsafe { list.Reset::<&_, Option<&ID3D12PipelineState>>(&alloc, None) },
    );
    step("alloc.Reset（第二次）", unsafe { alloc.Reset() });

    // ---- 4. 空根签名 ----
    let mut blob = None::<ID3DBlob>;
    let ok = unsafe {
        D3D12SerializeRootSignature(
            &D3D12_ROOT_SIGNATURE_DESC::default(),
            D3D_ROOT_SIGNATURE_VERSION_1_0,
            &mut blob as *mut _,
            None,
        )
    };
    println!("  4. 序列化空根签名 …… {}", yn(ok.is_ok()));
    let Some(blob) = blob else { return };
    let sig = unsafe {
        std::slice::from_raw_parts(blob.GetBufferPointer().cast::<u8>(), blob.GetBufferSize())
    }
    .to_vec();
    let root = unsafe { device.CreateRootSignature(0, &sig) };
    println!("  4. CreateRootSignature → {}", yn(root.is_ok()));
    let Ok(root) = root else { return };

    // ---- 5. 建 PSO ----
    let pso = None::<ID3D12PipelineState>;
    let hr: windows::core::Result<ID3D12PipelineState> =
        unsafe { device.CreateGraphicsPipelineState(&pso_desc(&root)) };
    println!(
        "  5. CreateGraphicsPipelineState → {}{}",
        yn(hr.is_ok()),
        if pso.is_some() { " ✅" } else { "" }
    );

    // ---- 6. 队列里有什么 ----
    let n = unsafe { queue.GetNumStoredMessages() };
    println!("  6. 队列里有 {n} 条消息");

    // ---- 6a. 先问需要多少字节（`pMessage = NULL`）----
    //
    // D3D11 那边正是这一步救回了整条链路（原先传未初始化的 0）。
    let mut need: usize = 0;
    let ask = unsafe { queue.GetMessage(0, None, &mut need as *mut usize) };
    println!(
        "     6a. 问长度 → {}，需要 {need} 字节（结构体 {} 字节）",
        if ask.is_ok() { "ok" } else { "err" },
        std::mem::size_of::<D3D12_MESSAGE>()
    );

    for i in 0..n {
        // 缓冲区**给足**：描述文本可能紧跟在结构体后面一起写出，所以按最大消息长度
        // 分配，而不是恰好 `size_of::<D3D12_MESSAGE>()`。
        let mut buf = vec![0u8; 4096];
        let mut len = buf.len();
        match unsafe { queue.GetMessage(i, Some(buf.as_mut_ptr().cast()), &mut len) } {
            Ok(()) => {}
            Err(e) => {
                println!("     [{i}] GetMessage 失败 {:#X}，改用 size_of 试试", e.code().0 as u32);
                let mut len2 = std::mem::size_of::<D3D12_MESSAGE>();
                let mut small = D3D12_MESSAGE::default();
                if let Err(e2) = unsafe {
                    queue.GetMessage(i, Some(&mut small as *mut _), &mut len2)
                } {
                    println!("        仍然失败 {:#X}", e2.code().0 as u32);
                    continue;
                }
                dump(&small);
                continue;
            }
        }
        let msg: &D3D12_MESSAGE = unsafe { &*buf.as_ptr().cast::<D3D12_MESSAGE>() };
        dump(msg);
    }
    unsafe { queue.ClearStoredMessages() };
}

/// 打印一条消息。`pDescription` 是**指向别处**的指针，长度由 `DescriptionByteLength` 给。
fn dump(msg: &D3D12_MESSAGE) {
    let text = if msg.pDescription.is_null() {
        String::from("(没有描述)")
    } else {
        String::from_utf8_lossy(unsafe {
            std::slice::from_raw_parts(msg.pDescription, msg.DescriptionByteLength)
        })
        .into_owned()
    };
    println!(
        "       Category={:?} Severity={:?} ID={:?} len={}",
        msg.Category, msg.Severity, msg.ID, msg.DescriptionByteLength
    );
    println!("       {text}");
}

fn yn(b: bool) -> &'static str {
    if b {
        "✅"
    } else {
        "❌"
    }
}

/// 一个尽可能小的 PSO 描述符。
///
/// 字段值逐条对着 `CreateGraphicsPipelineState` 文档的运行时校验列过：
///
/// - `NodeMask = 0`：单 GPU 必须为 0（`D3D12CreateDevice` 的 `NodeMask` 同样是 0）。
/// - `BlendEnable = FALSE` ⇒ `SrcBlend`/`DestBlend` 必须都是 `ZERO` —— 已满足。
/// - `DSVFormat = UNKNOWN` ⇒ `DepthStencilState.DepthEnable` 必须为 `FALSE` ——
///   `D3D12_DEPTH_STENCIL_DESC::default()` 的 `BOOL` 字段都是 0，满足。
/// - `InputLayout.NumElements = 0` 且着色器无输入 —— 平凡着色器只用 `SV_VertexID`。
fn pso_desc(root: &ID3D12RootSignature) -> D3D12_GRAPHICS_PIPELINE_STATE_DESC {
    // `pRootSignature` 是 `ManuallyDrop<Option<ID3D12RootSignature>>` —— windows-rs 要
// 我们手动保证它在描述符存活期间不释放。`ManuallyDrop` 就是那个保证。
let keep = std::mem::ManuallyDrop::new(Some(root.clone()));
    D3D12_GRAPHICS_PIPELINE_STATE_DESC {
        pRootSignature: keep.clone(),
        VS: D3D12_SHADER_BYTECODE {
            pShaderBytecode: TRIV_VS.as_ptr().cast(),
            BytecodeLength: TRIV_VS.len(),
        },
        PS: D3D12_SHADER_BYTECODE {
            pShaderBytecode: TRIV_PS.as_ptr().cast(),
            BytecodeLength: TRIV_PS.len(),
        },
        DS: D3D12_SHADER_BYTECODE::default(),
        HS: D3D12_SHADER_BYTECODE::default(),
        GS: D3D12_SHADER_BYTECODE::default(),
        StreamOutput: D3D12_STREAM_OUTPUT_DESC::default(),
        BlendState: D3D12_BLEND_DESC {
            AlphaToCoverageEnable: false.into(),
            IndependentBlendEnable: false.into(),
            RenderTarget: [rts(0); 8],
        },
        SampleMask: u32::MAX,
        RasterizerState: D3D12_RASTERIZER_DESC {
            FillMode: D3D12_FILL_MODE_SOLID,
            CullMode: D3D12_CULL_MODE_NONE,
            FrontCounterClockwise: false.into(),
            DepthBias: 0,
            DepthBiasClamp: 0.0,
            SlopeScaledDepthBias: 0.0,
            DepthClipEnable: true.into(),
            MultisampleEnable: false.into(),
            AntialiasedLineEnable: false.into(),
            ForcedSampleCount: 0,
            ConservativeRaster: D3D12_CONSERVATIVE_RASTERIZATION_MODE_OFF,
        },
        DepthStencilState: D3D12_DEPTH_STENCIL_DESC::default(),
        InputLayout: D3D12_INPUT_LAYOUT_DESC {
            pInputElementDescs: std::ptr::null(),
            NumElements: 0,
        },
        IBStripCutValue: D3D12_INDEX_BUFFER_STRIP_CUT_VALUE_DISABLED,
        PrimitiveTopologyType: D3D12_PRIMITIVE_TOPOLOGY_TYPE_TRIANGLE,
        NumRenderTargets: 1,
        // 第 1..8 槽必须 `UNKNOWN` —— 这正是本探针最初要验的那一条。
        RTVFormats: {
            let mut f = [DXGI_FORMAT_UNKNOWN; 8];
            f[0] = DXGI_FORMAT_R8G8B8A8_UNORM;
            f
        },
        DSVFormat: DXGI_FORMAT_UNKNOWN,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        NodeMask: 0,
        CachedPSO: D3D12_CACHED_PIPELINE_STATE::default(),
        Flags: D3D12_PIPELINE_STATE_FLAG_NONE,
    }
}

/// 第 `i` 个渲染目标的状态。`BlendEnable = FALSE` 时 `SrcBlend`/`DestBlend` 必须为 `ZERO`。
fn rts(_i: usize) -> D3D12_RENDER_TARGET_BLEND_DESC {
    D3D12_RENDER_TARGET_BLEND_DESC {
        BlendEnable: false.into(),
        LogicOpEnable: false.into(),
        SrcBlend: D3D12_BLEND_ZERO,
        DestBlend: D3D12_BLEND_ZERO,
        BlendOp: D3D12_BLEND_OP_ADD,
        SrcBlendAlpha: D3D12_BLEND_ZERO,
        DestBlendAlpha: D3D12_BLEND_ZERO,
        BlendOpAlpha: D3D12_BLEND_OP_ADD,
        LogicOp: D3D12_LOGIC_OP_NOOP,
        RenderTargetWriteMask: D3D12_COLOR_WRITE_ENABLE_ALL.0 as u8,
    }
}
//! D3D12 资源创建到底卡在哪 —— 扫字段，不猜。
//!
//! # 背景
//!
//! 之前的结论是「`CreateCommittedResource` 在 4 块适配器上全部返回 `E_INVALIDARG`，
//! 包括「上传堆 64 字节缓冲 + `GENERIC_READ`」这种每个字段都只有唯一正确取值的调用」。
//! 从那以后就把它记成了「环境问题」。
//!
//! **但 D3D11 那边刚给出过一个教训**：我以为「本机 DYNAMIC 纹理建不出来是环境问题」，
//! 结果 `DYNAMIC` 是**我自己选的** —— 换成 `DEFAULT` 就正常。所以「字段零歧义」这个
//! 前提本身需要被检验。
//!
//! 另外有一条独立线索：Skia 的 Ganesh D3D 后端**一次都不调
//! `CreateCommittedResource`** —— 它走 D3D12MemAlloc 的 suballocation +
//! `CreatePlacedResource`。那么「提交式不行、放置式行」是完全可能的。
//!
//! # 这个探针做什么
//!
//! 按「越来越基础」和「逐字段变体」两条轴试，每一步都报 HRESULT：
//!
//! | 步骤 | 试什么 | 若通过说明 |
//! |---|---|---|
//! | 1 | 枚举适配器 + 建设备 + **建命令队列** | 基础链路通 |
//! | 2 | `CreateHeap`（UPLOAD） | **堆本身**建得出来 → 内存分配路径可用 |
//! | 3 | `CreatePlacedResource`（用 2 的堆） | 放置式行 → 问题在 committed 特有逻辑 |
//! | 4 | `CreateCommittedResource`，扫 `Layout` | `ROW_MAJOR` vs `UNKNOWN` |
//! | 5 | 扫 `MemoryPoolPreference` | `UNKNOWN` vs `ONE` |
//! | 6 | 扫 `CPUPageProperty` | `UNKNOWN` vs `NOT_CPU_CACHED` |
//! | 7 | 扫节点掩码 | `1/1` vs `0/0` |
//! | 8 | `CreateCommittedResource` 在 DEFAULT 堆上建纹理 | 与堆类型无关？ |
//! | 9 | 每次失败后查 `GetDeviceRemovedReason` | 区分「参数错」与「设备已移除」 |
//!
//! # 判据
////!
//! 某一步通过 ⇒ 前面的假设被推翻，从该步开始的信息查。全失败且
//! `GetDeviceRemovedReason` 一直是 `S_OK` ⇒ 真的每个组合都不被接受，那时才轮到环境。

#![cfg(windows)]

use windows::Win32::Graphics::Direct3D::ID3DBlob;
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_FORMAT_UNKNOWN, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIAdapter1, DXGI_ADAPTER_DESC, DXGI_ADAPTER_DESC1,
};

fn hres(r: windows::core::Result<()>) -> Result<(), u32> {
    r.map_err(|e| e.code().0 as u32)
}

/// 上传堆上的 64 字节缓冲描述符（起点，各字段随后逐个变体）。
fn buffer_desc(layout: D3D12_TEXTURE_LAYOUT) -> D3D12_RESOURCE_DESC {
    D3D12_RESOURCE_DESC {
        Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
        Alignment: 0,
        Width: 64,
        Height: 1,
        DepthOrArraySize: 1,
        MipLevels: 1,
        Format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_UNKNOWN,
        SampleDesc: windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Layout: layout,
        Flags: D3D12_RESOURCE_FLAG_NONE,
    }
}

fn heap_props(
    kind: D3D12_HEAP_TYPE,
    cpu: D3D12_CPU_PAGE_PROPERTY,
    pool: D3D12_MEMORY_POOL,
    create_mask: u32,
    visible_mask: u32,
) -> D3D12_HEAP_PROPERTIES {
    D3D12_HEAP_PROPERTIES {
        Type: kind,
        CPUPageProperty: cpu,
        MemoryPoolPreference: pool,
        CreationNodeMask: create_mask,
        VisibleNodeMask: visible_mask,
    }
}

struct Dev {
    device: ID3D12Device,
    queue: ID3D12CommandQueue,
}

fn make_device(adapter: &IDXGIAdapter1) -> Result<Dev, u32> {
    use windows::core::Interface;
    let dxgi = adapter.cast::<windows::Win32::Graphics::Dxgi::IDXGIAdapter>().map_err(|e| e.code().0 as u32)?;
    let mut dev = None::<ID3D12Device>;
    unsafe {
        windows::Win32::Graphics::Direct3D12::D3D12CreateDevice(
            &dxgi,
            windows::Win32::Graphics::Direct3D::D3D_FEATURE_LEVEL_11_0,
            &mut dev as *mut _,
        )
    }
    .map_err(|e| e.code().0 as u32)?;
    let dev = dev.ok_or(0xFFFF_FFFFu32)?;
    // `CreateCommandQueue` 在 windows-rs 0.62 里是**返回**接口而不是写出参。
    let queue: ID3D12CommandQueue =
        unsafe { dev.CreateCommandQueue(&D3D12_COMMAND_QUEUE_DESC::default()) }
            .map_err(|e| e.code().0 as u32)?;
    Ok(Dev { device: dev, queue })
}

/// `CreateHeap` / `CreatePlacedResource` 的出参在 0.62 里是 `*mut Option<T>`（不是 `Option<*mut _>`）。
fn heap_of(device: &ID3D12Device, hdesc: &D3D12_HEAP_DESC) -> Result<ID3D12Heap, u32> {
    let mut out = None::<ID3D12Heap>;
    unsafe { device.CreateHeap(hdesc, &mut out as *mut _) }.map_err(|e| e.code().0 as u32)?;
    out.ok_or(0xFFFF_FFFFu32)
}

fn removed_reason(device: &ID3D12Device) -> String {
    match unsafe { device.GetDeviceRemovedReason() } {
        Ok(()) => "S_OK（设备还活着）".to_string(),
        Err(e) => match e.code().0 as u32 {
            0x887A0004 => "DXGI_ERROR_DEVICE_HUNG".to_string(),
            0x887A0005 => "DXGI_ERROR_DEVICE_REMOVED".to_string(),
            0x887A0006 => "DXGI_ERROR_DEVICE_RESET".to_string(),
            0x887A0007 => "DXGI_ERROR_DRIVER_INTERNAL_ERROR".to_string(),
            0x887A000A => "DXGI_ERROR_DEVICE_HUNG（内核态）".to_string(),
            0x887A0009 => "DXGI_ERROR_INVALID_STATE".to_string(),
            other => format!("{other:#X}"),
        },
    }
}

/// 试一次 `CreateCommittedResource`。返回 `Ok(())` / `Err(hr)`。
fn try_committed(
    device: &ID3D12Device,
    props: &D3D12_HEAP_PROPERTIES,
    desc: &D3D12_RESOURCE_DESC,
    initial: D3D12_RESOURCE_STATES,
) -> Result<(), u32> {
    let mut out = None::<ID3D12Resource>;
    unsafe {
        device
            .CreateCommittedResource(props, D3D12_HEAP_FLAG_NONE, desc, initial, None, &mut out as *mut _)
            .map_err(|e| e.code().0 as u32)?;
    }
    Ok(())
}

fn probe(adapter: &IDXGIAdapter1, desc1: &DXGI_ADAPTER_DESC1) {
    let end = desc1.Description.iter().position(|&c| c == 0).unwrap_or(128);
    let name = String::from_utf16_lossy(&desc1.Description[..end]);
    println!("\n╔══ {name}");

    let dev = match make_device(adapter) {
        Ok(d) => {
            println!("║  1. 设备 + 命令队列 …… ✅");
            d
        }
        Err(hr) => {
            println!("║  1. 设备 + 命令队列 …… ❌ {hr:#X}");
            return;
        }
    };
    let _ = &dev.queue;

    // ---- 2. CreateHeap ----
    let hp = heap_props(
        D3D12_HEAP_TYPE_UPLOAD,
        D3D12_CPU_PAGE_PROPERTY_UNKNOWN,
        D3D12_MEMORY_POOL_UNKNOWN,
        1,
        1,
    );
    let hdesc = D3D12_HEAP_DESC {
        SizeInBytes: 65536,
        Properties: hp,
        Alignment: 0,
        Flags: D3D12_HEAP_FLAG_NONE,
    };
    let heap = match heap_of(&dev.device, &hdesc) {
        Ok(h) => {
            println!("║  2. CreateHeap(UPLOAD, 64KB) …… ✅");
            h
        }
        Err(hr) => {
            println!("║  2. CreateHeap(UPLOAD, 64KB) …… ❌ {hr:#X}   ← 堆本身都不行");
            println!("║     设备状态: {}", removed_reason(&dev.device));
            println!("╚══");
            return;
        }
    };

    // ---- 3. CreatePlacedResource ----
    let pdesc = buffer_desc(D3D12_TEXTURE_LAYOUT_UNKNOWN);
    let mut placed = None::<ID3D12Resource>;
    match unsafe {
        dev.device.CreatePlacedResource(
            &heap,
            0,
            &pdesc,
            D3D12_RESOURCE_STATE_GENERIC_READ,
            None,
            &mut placed as *mut _,
        )
    } {
        Ok(()) => println!("║  3. CreatePlacedResource(UPLOAD) …… ✅  ★ 放置式可用！"),
        Err(e) => println!(
            "║  3. CreatePlacedResource(UPLOAD) …… ❌ {:#X} {}",
            e.code().0 as u32,
            e.message()
        ),
    }

    // ---- 4~7. 扫 committed 的各个字段 ----
    println!("║  4. CreateCommittedResource 字段扫描：");
    // ⚠ 枚举名都按 windows-rs 0.62 的实际拼写，不是 D3D12 SDK 文档里的：
//   内存池是 `L0`/`L1`（不是 `ONE`）；`CPU_PAGE_PROPERTY` 只有
//   `UNKNOWN`/`NOT_AVAILABLE`/`WRITE_BACK`/`WRITE_COMBINE`（**没有** `NOT_CPU_CACHED`，
//   那是 Vulkan 的叫法）；纹理布局没有 `OPTIMIZED`（那是 `D3D12_TILE_LAYOUT` 的词）。
// 名字猜错会编译不过 —— 这本身是好事：拼错的常量进不了二进制。
let layouts = [
        ("Layout=UNKNOWN", D3D12_TEXTURE_LAYOUT_UNKNOWN),
        ("Layout=ROW_MAJOR", D3D12_TEXTURE_LAYOUT_ROW_MAJOR),
        ("Layout=64KB_UNDEF_SWIZZLE", D3D12_TEXTURE_LAYOUT_64KB_UNDEFINED_SWIZZLE),
    ];
    let pools = [
        ("Pool=UNKNOWN", D3D12_MEMORY_POOL_UNKNOWN),
        ("Pool=L0", D3D12_MEMORY_POOL_L0),
        ("Pool=L1", D3D12_MEMORY_POOL_L1),
    ];
    let cpus = [
        ("CPUPage=UNKNOWN", D3D12_CPU_PAGE_PROPERTY_UNKNOWN),
        ("CPUPage=WRITE_BACK", D3D12_CPU_PAGE_PROPERTY_WRITE_BACK),
        ("CPUPage=NOT_AVAILABLE", D3D12_CPU_PAGE_PROPERTY_NOT_AVAILABLE),
    ];
    let masks = [("Mask=1/1", 1u32, 1u32), ("Mask=0/0", 0u32, 0u32)];
    let heap_kinds = [
        ("UPLOAD", D3D12_HEAP_TYPE_UPLOAD),
        ("READBACK", D3D12_HEAP_TYPE_READBACK),
        ("DEFAULT", D3D12_HEAP_TYPE_DEFAULT),
    ];

    let mut any_ok = false;
    for (lname, layout) in layouts {
        let d = buffer_desc(layout);
        for (pname, pool) in pools {
            for (cname, cpu) in cpus {
                for (mname, cm, vm) in masks {
                    // READBACK/DEFAULT 堆不能用 GENERIC_READ
                    for (hname, hk) in heap_kinds {
                        let state = match hk {
                            D3D12_HEAP_TYPE_READBACK => D3D12_RESOURCE_STATE_COPY_DEST,
                            D3D12_HEAP_TYPE_DEFAULT => D3D12_RESOURCE_STATE_COMMON,
                            _ => D3D12_RESOURCE_STATE_GENERIC_READ,
                        };
                        let props = heap_props(hk, cpu, pool, cm, vm);
                        match try_committed(&dev.device, &props, &d, state) {
                            Ok(()) => {
                                println!(
                                    "║     ✅ {lname} {hname} {pname} {cname} {mname}  ← 通了"
                                );
                                any_ok = true;
                            }
                            // `E_INVALIDARG` 是预期内的绝大多数结果 —— 逐条打印会把
                            // 81 行全刷成噪声。只报**别的**错误码，那才是意外。
                            Err(0x8007_0057) => {}
                            Err(hr) => println!("║     · {lname} {hname} {pname} {cname} {mname} → {hr:#X}（非 E_INVALIDARG）"),
                        }
                    }
                }
            }
        }
    }
    if !any_ok {
        println!("║     （以上全部 E_INVALIDARG，共 {} 种组合）", layouts.len()*pools.len()*cpus.len()*masks.len()*heap_kinds.len());
    }

    // ---- 8. DEFAULT 堆上的纹理 ----
    let tdesc = D3D12_RESOURCE_DESC {
        Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
        Alignment: 0,
        Width: 64,
        Height: 64,
        DepthOrArraySize: 1,
        MipLevels: 1,
        Format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_R8G8B8A8_UNORM,
        SampleDesc: windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Layout: D3D12_TEXTURE_LAYOUT_UNKNOWN,
        Flags: D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET,
    };
    for (tname, flags) in [
        ("RT", D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET),
        ("NONE", D3D12_RESOURCE_FLAG_NONE),
    ] {
        let mut d = tdesc;
        d.Flags = flags;
        let props = heap_props(
            D3D12_HEAP_TYPE_DEFAULT,
            D3D12_CPU_PAGE_PROPERTY_UNKNOWN,
            D3D12_MEMORY_POOL_UNKNOWN,
            1,
            1,
        );
        match try_committed(&dev.device, &props, &d, D3D12_RESOURCE_STATE_RENDER_TARGET) {
            Ok(()) => println!("║  8. 纹理(DEFAULT 堆, Flags={tname}) …… ✅"),
            Err(hr) => println!("║  8. 纹理(DEFAULT 堆, Flags={tname}) …… ❌ {hr:#X}"),
        }
    }

    println!("║  9. 设备状态: {}", removed_reason(&dev.device));
    probe_pso(&dev.device, &heap);
    println!("╚══");
}

/// D3D12 里**能建的最小的 PSO**：平凡着色器 + 空根签名 + 无输入布局。
///
/// 它失败就说明 `CreateGraphicsPipelineState` 在这台机器上整体不可用 —— 与本仓库
/// 的描述符、字节码、输入布局都无关。
///
/// 这不是猜的：把生产 PSO 的每一项逐个去掉（输入布局、alpha 混合、深度模板）之后
/// 它**照样**失败，所以问题落到了「PSO 这条路径本身」。
fn probe_pso(device: &ID3D12Device, _heap: &ID3D12Heap) {
    // 空根签名。
    let rsdesc = D3D12_ROOT_SIGNATURE_DESC {
        NumParameters: 0,
        pParameters: std::ptr::null(),
        NumStaticSamplers: 0,
        pStaticSamplers: std::ptr::null(),
        Flags: D3D12_ROOT_SIGNATURE_FLAG_NONE,
    };
    let mut blob = None::<ID3DBlob>;
    let mut err = None::<ID3DBlob>;
    if unsafe {
        D3D12SerializeRootSignature(&rsdesc, D3D_ROOT_SIGNATURE_VERSION_1_0, &mut blob, Some(&mut err))
    }
    .is_err()
    {
        println!("║ 10. 序列化空根签名 …… ❌");
        return;
    }
    let Some(blob) = blob else {
        println!("║ 10. 序列化空根签名 …… ❌ 没给 blob");
        return;
    };
    let bytes = unsafe {
        std::slice::from_raw_parts(blob.GetBufferPointer().cast::<u8>(), blob.GetBufferSize())
    }
    .to_vec();
    let root: ID3D12RootSignature = match unsafe { device.CreateRootSignature(0, &bytes) } {
        Ok(r) => r,
        Err(e) => {
            println!("║ 10. CreateRootSignature …… ❌ {:#X}", e.code().0 as u32);
            return;
        }
    };

    // 平凡着色器：`float4 VSMain(uint id : SV_VertexID) : SV_Position` + 常量像素。
    let vs = D3D12_SHADER_BYTECODE {
        pShaderBytecode: TRIV_VS.as_ptr() as *const _,
        BytecodeLength: TRIV_VS.len(),
    };
    let ps = D3D12_SHADER_BYTECODE {
        pShaderBytecode: TRIV_PS.as_ptr() as *const _,
        BytecodeLength: TRIV_PS.len(),
    };
    let desc = D3D12_GRAPHICS_PIPELINE_STATE_DESC {
        pRootSignature: std::mem::ManuallyDrop::new(Some(root.clone())),
        VS: vs,
        PS: ps,
        DS: D3D12_SHADER_BYTECODE::default(),
        HS: D3D12_SHADER_BYTECODE::default(),
        GS: D3D12_SHADER_BYTECODE::default(),
        StreamOutput: D3D12_STREAM_OUTPUT_DESC::default(),
        BlendState: D3D12_BLEND_DESC {
            AlphaToCoverageEnable: windows::core::BOOL(0),
            IndependentBlendEnable: windows::core::BOOL(0),
            RenderTarget: [D3D12_RENDER_TARGET_BLEND_DESC {
                BlendEnable: windows::core::BOOL(0),
                LogicOpEnable: windows::core::BOOL(0),
                SrcBlend: D3D12_BLEND_ZERO,
                DestBlend: D3D12_BLEND_ZERO,
                BlendOp: D3D12_BLEND_OP_ADD,
                SrcBlendAlpha: D3D12_BLEND_ZERO,
                DestBlendAlpha: D3D12_BLEND_ZERO,
                BlendOpAlpha: D3D12_BLEND_OP_ADD,
                LogicOp: D3D12_LOGIC_OP_NOOP,
                RenderTargetWriteMask: D3D12_COLOR_WRITE_ENABLE_ALL.0 as u8,
            }; 8],
        },
        SampleMask: u32::MAX,
        RasterizerState: D3D12_RASTERIZER_DESC {
            FillMode: D3D12_FILL_MODE_SOLID,
            CullMode: D3D12_CULL_MODE_NONE,
            FrontCounterClockwise: windows::core::BOOL(0),
            DepthBias: 0,
            DepthBiasClamp: 0.0,
            SlopeScaledDepthBias: 0.0,
            DepthClipEnable: windows::core::BOOL(1),
            MultisampleEnable: windows::core::BOOL(0),
            AntialiasedLineEnable: windows::core::BOOL(0),
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
        RTVFormats: [DXGI_FORMAT_R8G8B8A8_UNORM; 8],
        DSVFormat: DXGI_FORMAT_UNKNOWN,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        NodeMask: 1,
        CachedPSO: D3D12_CACHED_PIPELINE_STATE::default(),
        Flags: D3D12_PIPELINE_STATE_FLAG_NONE,
    };
    match unsafe { device.CreateGraphicsPipelineState(&desc) } as windows::core::Result<ID3D12PipelineState>
    {
        Ok(_) => println!("║ 10. 最小 PSO（平凡着色器 + 空根签名）…… ✅"),
        Err(e) => println!(
            "║ 10. 最小 PSO（平凡着色器 + 空根签名）…… ❌ {:#X} {}   ← PSO 这条路径整体不通",
            e.code().0 as u32,
            e.message()
        ),
    }
}

/// `float4 VSMain(uint id : SV_VertexID) : SV_Position` 的字节码。
const TRIV_VS: &[u8] = include_bytes!("triv.vs.bin");
/// `float4 PSMain() : SV_TARGET { return float4(1,1,1,1); }` 的字节码。
const TRIV_PS: &[u8] = include_bytes!("triv.ps.bin");

#[test]
#[ignore = "诊断用：本机 D3D12 的真实边界。看完结果后决定是否去掉 --ignored"]
fn d3d12_资源创建_逐字段扫描() {
    let factory: windows::Win32::Graphics::Dxgi::IDXGIFactory4 = unsafe { CreateDXGIFactory1() }
        .expect("DXGI 工厂");
    for i in 0..8u32 {
        let Ok(adapter) = (unsafe { factory.EnumAdapters1(i) }) else {
            break;
        };
        let Ok(desc1) = (unsafe { adapter.GetDesc1() }) else { continue };
        probe(&adapter, &desc1);
    }
}

#[test]
fn placeholder_keeps_file_compiled() {
    let _ = DXGI_ADAPTER_DESC::default();
    let _ = hres(Ok(()));
}

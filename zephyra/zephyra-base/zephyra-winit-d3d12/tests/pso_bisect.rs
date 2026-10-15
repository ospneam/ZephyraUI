//! D3D12 的 PSO 建不出来 —— 逐字段二分，不猜。
//!
//! # 背景
//!
//! `CreateGraphicsPipelineState` 返回 `E_INVALIDARG`，且设备没被移除。前面已经修掉了
//! 三个真 bug（缓冲的 `Layout`、纹理缺 `ALLOW_RENDER_TARGET`、`NodeMask: 0`），
//! 外加一个悬垂的 `SemanticName` 和一个一律 `R32G32_FLOAT` 的格式 —— 全都不是它。
//!
//! D3D12 的 PSO 校验失败**只回 `E_INVALIDARG`，从不说是哪个字段**。所以唯一可靠
//! 的办法是从最小可用描述开始，一次加一项，看哪一项加上去就坏。
//!
//! # 二分的顺序
//!
//! | 步骤 | 加了什么 | 若这一步坏，说明 |
//! |---|---|---|
//! | 0 | 只有 VS + PS，无输入布局、无混合、无深度 | 连最小 PSO 都不行 → 字节码或设备 |
//! | 1 | + 顶点输入布局 | 输入布局与签名对不上 |
//! | 2 | + 混合（alpha blend） | 混合描述不合法 |
//! | 3 | + 深度模板描述 | 深度模板与 `DSVFormat` 不一致 |
//! | 4 | + 静态采样器与根签名的完整组合 | 根签名与着色器寄存器对不上 |
//! | 5 | 全部（即生产用的描述） | 收尾比对 |
//!
//! # 判据
//!
//! 每一步都打印 HRESULT。**第一个坏掉的步骤**就是问题所在；它之前全通过，说明那些
//! 部分没问题。

#![cfg(windows)]

use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::Common::*;

use zephyra_winit_d3d12::{Gpu, Shaders};

fn vs() -> D3D12_SHADER_BYTECODE {
    D3D12_SHADER_BYTECODE {
        pShaderBytecode: zephyra_winit_d3d12::shader::VS_BYTECODE.as_ptr() as *const _,
        BytecodeLength: zephyra_winit_d3d12::shader::VS_BYTECODE.len(),
    }
}

fn ps() -> D3D12_SHADER_BYTECODE {
    D3D12_SHADER_BYTECODE {
        pShaderBytecode: zephyra_winit_d3d12::shader::PS_VARIANTS[0].1.as_ptr() as *const _,
        BytecodeLength: zephyra_winit_d3d12::shader::PS_VARIANTS[0].1.len(),
    }
}

// ---- 平凡着色器：用来把「字节码」与「根签名」分开 ----
//
// 源码（`fxc /T vs_5_1 /T ps_5_1` 现场编译，字节数与本仓库其余字节码同量级）：
//
// ```hlsl
// float4 VSMain(uint id : SV_VertexID) : SV_Position { … }  // 不读任何顶点输入
// float4 PSMain() : SV_TARGET { return float4(1,1,1,1); }      // 不读任何资源
// ```

/// `float4 VSMain(uint id : SV_VertexID) : SV_Position`
const TRIV_VS: &[u8] = include_bytes!("triv.vs.bin");
/// `float4 PSMain() : SV_TARGET { return float4(1,1,1,1); }`
const TRIV_PS: &[u8] = include_bytes!("triv.ps.bin");

fn triv_vs() -> D3D12_SHADER_BYTECODE {
    D3D12_SHADER_BYTECODE {
        pShaderBytecode: TRIV_VS.as_ptr() as *const _,
        BytecodeLength: TRIV_VS.len(),
    }
}

fn triv_ps() -> D3D12_SHADER_BYTECODE {
    D3D12_SHADER_BYTECODE {
        pShaderBytecode: TRIV_PS.as_ptr() as *const _,
        BytecodeLength: TRIV_PS.len(),
    }
}

/// 空根签名（零参数、零静态采样器）。
fn empty_root_signature(device: &ID3D12Device) -> ID3D12RootSignature {
    let rsdesc = D3D12_ROOT_SIGNATURE_DESC {
        NumParameters: 0,
        pParameters: std::ptr::null(),
        NumStaticSamplers: 0,
        pStaticSamplers: std::ptr::null(),
        Flags: D3D12_ROOT_SIGNATURE_FLAG_NONE,
    };
    let mut blob = None::<ID3DBlob>;
    let mut err = None::<ID3DBlob>;
    unsafe {
        D3D12SerializeRootSignature(
            &rsdesc,
            D3D_ROOT_SIGNATURE_VERSION_1_0,
            &mut blob,
            Some(&mut err),
        )
    }
    .expect("序列化空根签名");
    let blob = blob.expect("blob");
    let bytes = unsafe {
        std::slice::from_raw_parts(blob.GetBufferPointer().cast::<u8>(), blob.GetBufferSize())
    }
    .to_vec();
    unsafe { device.CreateRootSignature(0, &bytes) }.expect("空根签名")
}

/// 逐步加料并报告。
fn step(
    device: &ID3D12Device,
    _root: &ID3D12RootSignature,
    name: &str,
    desc: &D3D12_GRAPHICS_PIPELINE_STATE_DESC,
) -> bool {
    let r: windows::core::Result<ID3D12PipelineState> =
        unsafe { device.CreateGraphicsPipelineState(desc) };
    match r {
        Ok(pso) => {
            println!("  ✅ {name}");
            let _ = pso;
            true
        }
        Err(e) => {
            println!("  ❌ {name}  →  {:#X} {}", e.code().0 as u32, e.message());
            false
        }
    }
}

#[test]
#[ignore = "诊断用：二分 PSO 描述符。结论已记录在 src/lib.rs —— 本机最小 PSO 也失败"]
fn d3d12_管线状态描述符逐字段二分() {
    let gpu = Gpu::new().expect("建设备");
    println!("设备移除原因 = {:#X}", gpu.removed_reason().0);

    // 根签名：与生产代码一致（t0 描述符表 + s0 静态采样器）。
    let ranges = [D3D12_DESCRIPTOR_RANGE {
        RangeType: D3D12_DESCRIPTOR_RANGE_TYPE_SRV,
        NumDescriptors: 1,
        BaseShaderRegister: 0,
        RegisterSpace: 0,
        OffsetInDescriptorsFromTableStart: 0,
    }];
    let params = [D3D12_ROOT_PARAMETER {
        ParameterType: D3D12_ROOT_PARAMETER_TYPE_DESCRIPTOR_TABLE,
        Anonymous: D3D12_ROOT_PARAMETER_0 {
            DescriptorTable: D3D12_ROOT_DESCRIPTOR_TABLE {
                NumDescriptorRanges: 1,
                pDescriptorRanges: ranges.as_ptr(),
            },
        },
        ShaderVisibility: D3D12_SHADER_VISIBILITY_PIXEL,
    }];
    let sampler = D3D12_STATIC_SAMPLER_DESC {
        Filter: D3D12_FILTER_MIN_MAG_MIP_LINEAR,
        AddressU: D3D12_TEXTURE_ADDRESS_MODE_CLAMP,
        AddressV: D3D12_TEXTURE_ADDRESS_MODE_CLAMP,
        AddressW: D3D12_TEXTURE_ADDRESS_MODE_CLAMP,
        MipLODBias: 0.0,
        MaxAnisotropy: 1,
        ComparisonFunc: D3D12_COMPARISON_FUNC_NEVER,
        BorderColor: D3D12_STATIC_BORDER_COLOR_TRANSPARENT_BLACK,
        MinLOD: 0.0,
        MaxLOD: f32::MAX,
        ShaderRegister: 0,
        RegisterSpace: 0,
        ShaderVisibility: D3D12_SHADER_VISIBILITY_PIXEL,
    };
    let rsdesc = D3D12_ROOT_SIGNATURE_DESC {
        NumParameters: 1,
        pParameters: params.as_ptr(),
        NumStaticSamplers: 1,
        pStaticSamplers: &sampler,
        Flags: D3D12_ROOT_SIGNATURE_FLAG_ALLOW_INPUT_ASSEMBLER_INPUT_LAYOUT,
    };
    let mut blob = None::<ID3DBlob>;
    let mut err = None::<ID3DBlob>;
    unsafe {
        D3D12SerializeRootSignature(
            &rsdesc,
            D3D_ROOT_SIGNATURE_VERSION_1_0,
            &mut blob,
            Some(&mut err),
        )
    }
    .expect("序列化根签名");
    let blob = blob.expect("blob");
    let bytes = unsafe {
        std::slice::from_raw_parts(blob.GetBufferPointer().cast::<u8>(), blob.GetBufferSize())
    }
    .to_vec();
    let root: ID3D12RootSignature =
        unsafe { gpu.device.CreateRootSignature(0, &bytes) }.expect("根签名");
    println!("根签名建成");

    let shaders = Shaders::new().expect("Shaders");
    let elements = shaders.elements.descs();
    println!("输入元素 {} 项", elements.len());

    // ---- 公共部分 ----
    let base = D3D12_GRAPHICS_PIPELINE_STATE_DESC {
        pRootSignature: std::mem::ManuallyDrop::new(Some(root.clone())),
        VS: vs(),
        PS: ps(),
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
        // 显式给一份「合法」的深度模板，而不是 `default()`。
        //
        // windows-rs 0.62 把 C 里那两组 `StencilOp FrontFace;` / `StencilOp BackFace;`
        // 建模成**嵌套**的 `D3D12_DEPTH_STENCILOP_DESC`，不是 `D3D12_STENCIL_FACE_*`
        // 那个标量 —— D3D12 头文件里也没有 `D3D12_STENCIL_FACE_FRONT` 这个常量。
        DepthStencilState: D3D12_DEPTH_STENCIL_DESC {
            DepthEnable: windows::core::BOOL(0),
            DepthWriteMask: D3D12_DEPTH_WRITE_MASK_ALL,
            DepthFunc: D3D12_COMPARISON_FUNC_ALWAYS,
            StencilEnable: windows::core::BOOL(0),
            StencilReadMask: 0xFF,
            StencilWriteMask: 0xFF,
            FrontFace: D3D12_DEPTH_STENCILOP_DESC {
                StencilFailOp: D3D12_STENCIL_OP_KEEP,
                StencilDepthFailOp: D3D12_STENCIL_OP_KEEP,
                StencilPassOp: D3D12_STENCIL_OP_KEEP,
                StencilFunc: D3D12_COMPARISON_FUNC_ALWAYS,
            },
            BackFace: D3D12_DEPTH_STENCILOP_DESC {
                StencilFailOp: D3D12_STENCIL_OP_KEEP,
                StencilDepthFailOp: D3D12_STENCIL_OP_KEEP,
                StencilPassOp: D3D12_STENCIL_OP_KEEP,
                StencilFunc: D3D12_COMPARISON_FUNC_ALWAYS,
            },
        },
        InputLayout: D3D12_INPUT_LAYOUT_DESC {
            pInputElementDescs: std::ptr::null(),
            NumElements: 0,
        },
        IBStripCutValue: D3D12_INDEX_BUFFER_STRIP_CUT_VALUE_DISABLED,
        PrimitiveTopologyType: D3D12_PRIMITIVE_TOPOLOGY_TYPE_TRIANGLE,
        NumRenderTargets: 1,
        RTVFormats: [DXGI_FORMAT_R8G8B8A8_UNORM; 8],
        DSVFormat: DXGI_FORMAT_UNKNOWN,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        NodeMask: 1,
        CachedPSO: D3D12_CACHED_PIPELINE_STATE::default(),
        Flags: D3D12_PIPELINE_STATE_FLAG_NONE,
    };

    println!("\n=== 先把「字节码」与「根签名」分开 ===");
    // 平凡着色器 + 空根签名：这是 D3D12 里能建的最小的 PSO。它过不了就是设备/驱动
    // 层面的问题，跟本仓库的代码无关。
    let empty_root = empty_root_signature(&gpu.device);
    let trivial = D3D12_GRAPHICS_PIPELINE_STATE_DESC {
        pRootSignature: std::mem::ManuallyDrop::new(Some(empty_root.clone())),
        VS: triv_vs(),
        PS: triv_ps(),
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
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        NodeMask: 1,
        CachedPSO: D3D12_CACHED_PIPELINE_STATE::default(),
        Flags: D3D12_PIPELINE_STATE_FLAG_NONE,
    };
    let ok_trivial = step(&gpu.device, &empty_root, "平凡着色器 + 空根签名", &trivial);
    // 平凡着色器 + 生产根签名：若这个过、上面那个也过，说明根签名与着色器**互相**
    // 不匹配才是问题（PSO 会校验「着色器实际用到的寄存器 ⊆ 根签名声明的」）。
    let mut t2 = D3D12_GRAPHICS_PIPELINE_STATE_DESC {
        pRootSignature: std::mem::ManuallyDrop::new(Some(root.clone())),
        ..trivial
    };
    t2.pRootSignature = std::mem::ManuallyDrop::new(Some(root.clone()));
    let ok_trivial_prod_root = step(&gpu.device, &root, "平凡着色器 + 生产根签名", &t2);
    // 生产着色器 + 空根签名：这一步会**必然**失败（生产 PS 读了 t0/s0，而空根签名
    // 没声明它们），所以它只是确认校验确实在跑。
    let mut t3 = D3D12_GRAPHICS_PIPELINE_STATE_DESC {
        VS: vs(),
        PS: ps(),
        InputLayout: D3D12_INPUT_LAYOUT_DESC {
            pInputElementDescs: elements.as_ptr(),
            NumElements: elements.len() as u32,
        },
        ..trivial
    };
    t3.pRootSignature = std::mem::ManuallyDrop::new(Some(empty_root.clone()));
    let _ = step(
        &gpu.device,
        &empty_root,
        "生产着色器 + 空根签名（预期失败）",
        &t3,
    );

    println!("\n=== 生产组合逐项加料 ===");
    let mut d = base;
    d.InputLayout = D3D12_INPUT_LAYOUT_DESC {
        pInputElementDescs: elements.as_ptr(),
        NumElements: elements.len() as u32,
    };
    let ok1 = step(&gpu.device, &root, "1. 输入布局 + 最小混合/深度", &d);

    d.BlendState.RenderTarget[0] = D3D12_RENDER_TARGET_BLEND_DESC {
        BlendEnable: windows::core::BOOL(1),
        LogicOpEnable: windows::core::BOOL(0),
        SrcBlend: D3D12_BLEND_SRC_ALPHA,
        DestBlend: D3D12_BLEND_INV_SRC_ALPHA,
        BlendOp: D3D12_BLEND_OP_ADD,
        SrcBlendAlpha: D3D12_BLEND_ONE,
        DestBlendAlpha: D3D12_BLEND_INV_SRC_ALPHA,
        BlendOpAlpha: D3D12_BLEND_OP_ADD,
        LogicOp: D3D12_LOGIC_OP_NOOP,
        RenderTargetWriteMask: D3D12_COLOR_WRITE_ENABLE_ALL.0 as u8,
    };
    let ok2 = step(&gpu.device, &root, "2. + alpha 混合", &d);

    d.DepthStencilState = D3D12_DEPTH_STENCIL_DESC::default();
    let ok3 = step(
        &gpu.device,
        &root,
        "3. + DepthStencil::default()（= 生产用的）",
        &d,
    );

    println!("\n=== 结论 ===");
    println!(
        "  平凡着色器 + 空根签名     : {}",
        if ok_trivial {
            "✅"
        } else {
            "❌ ← 设备/驱动层面"
        }
    );
    println!(
        "  平凡着色器 + 生产根签名   : {}",
        if ok_trivial_prod_root {
            "✅"
        } else {
            "❌ ← 根签名本身"
        }
    );
    println!(
        "  生产着色器 + 输入布局     : {}",
        if ok1 {
            "✅"
        } else {
            "❌ ← 字节码/输入布局"
        }
    );
    println!(
        "  + alpha 混合             : {}",
        if ok2 { "✅" } else { "❌ ← 混合描述" }
    );
    println!(
        "  + DepthStencil::default(): {}",
        if ok3 { "✅" } else { "❌ ← 深度模板" }
    );
}

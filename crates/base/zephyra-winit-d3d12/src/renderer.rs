//! D3D12 渲染器：根签名、PSO、描述符堆、顶点/索引缓冲与绘制命令的记录。
//!
//! # 与 D3D11 版（`zephyra-winit::renderer`）的对应关系
//!
//! | D3D11 | D3D12 |
//! |---|---|
//! | `CreateVertexShader` / `CreatePixelShader` | 无独立对象，字节码直接进 PSO |
//! | `CreateInputLayout` | 无独立对象，作为 `InputLayout` 字段随 PSO 创建 |
//! | 立即上下文 `VSSetShader` 等 | 命令列表上的 `SetPipelineState` / `SetGraphicsRootSignature` |
//! | `OMSetRenderTargets(视图对象)` | `OMSetRenderTargets(堆里的第 n 个描述符)` |
//! | 驱动隐式做资源状态转换 | **必须手工发 `ResourceBarrier`** |
//! | `Map` / `Unmap` 逐帧写顶点 | UPLOAD 堆缓冲常驻映射，写完就完 |
//! | 无根签名概念 | 根签名决定 `t0`/`s0` 怎么找描述符 |
//!
//! # 记录 vs 提交
//!
//! D3D11 的 `render()` 里所有调用**立即生效**；D3D12 的同名调用只是**往命令列表里
//! 记一条命令**，要 `Gpu::submit` 之后才交给 GPU。因此本文件的 [`Renderer::record`]
//! **只记录**，不提交 —— 提交与等待交给宿主，这样「记录 / 提交 / 等待」三步在测试
//! 里都看得见，而不是被一个函数悄悄合在一起。

#![cfg(windows)]

use windows::Win32::Foundation::{E_FAIL, E_INVALIDARG, RECT};
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_R16_UINT;

use crate::quad::{self, RenderReport, Vertex};
use crate::shader::{self, ShaderError, Shaders};
use crate::surface::{create_resource, heap_props, DescriptorHeap, SurfaceError};

/// 一张已上传的图像纹理。
///
/// # 暂存缓冲为什么跟着活这么久
///
/// `CopyTextureRegion` 是**异步**的：命令记录完就返回，GPU 可能还没读那块内存。
/// 暂存缓冲一释放就把内存还给驱动，于是 GPU 读到的是别人的数据 —— 而这类故障在
/// 画面上表现为「偶发的花屏 / 少一块」，几乎不可能归因。
///
/// 稳妥的做法是等 GPU 读完再释放（宿主那边 `submit_one_off` 已经在等）。但那要求
/// 渲染器知道「什么时候可以等」。所以这里干脆**一直留着**：图像纹理是启动期一次性
/// 建、之后只读的资源，暂存缓冲的常驻代价（`w*h*4` 字节，图标是 4KB）与正确性不成
/// 比例。真要省，用 `Vec<Option<Staging>>` 在下一次上传时替换即可。
struct UploadedTexture {
    /// 纹理本体。**只为了持有所有权** —— 绘制时着色器读的是描述符堆里那个 SRV，
    /// 而 SRV 只是堆里的一个槽位，不延长纹理的寿命。
    _resource: ID3D12Resource,
    _staging: UploadBuffer,
    /// 它在 `srv_heap` 里的槽位。槽位用尽时据此说清「一共要几张」。
    _slot: u32,
}

/// 渲染器：持有 D3D12 管线状态与常驻资源。
///
/// # 设备不归本结构所有
///
/// `ID3D12Device` 由 [`crate::surface::Gpu`] 持有并按引用传入 —— 它是整个进程唯一的
/// 一份。若渲染器自己存一份，就多出一条与 `Gpu` 并行的引用路径，「谁在用设备」会变得
/// 无法回答（同 `zephyra-winit::renderer` 的取舍）。
pub struct Renderer {
    root_signature: ID3D12RootSignature,
    pso: ID3D12PipelineState,
    /// shader 可见的 CBV/SRV 堆。着色器的 `t0` 绑到其中第 0 号。
    srv_heap: DescriptorHeap,
    /// 图集的 SRV 在 `srv_heap` 里的位置（恒为 0）。
    atlas_srv_slot: u32,
    /// 上传堆 + 默认堆的逐帧几何缓冲。
    vertex: StreamBuffer,
    /// 索引缓冲（**逐帧重写**，内容由 [`quad::expand_indices`] 生成）。
    index: StreamBuffer,
    /// 顶点缓冲已分配字节数，用于验证稳态不增长。
    vertex_capacity: u64,
    /// 跨帧复用的展开缓冲（顶点流 + 索引 + 分批）。
    ///
    /// 稳态下 `quad::expand_into` 一帧**零分配** —— 纯函数版每帧要新开六七个
    /// `Vec` 并从 0 长到目标大小（实测 demo 每帧约 170 KB 的分配，69% 出在这里）。
    ///
    /// 本渲染器已经是 `&mut self`，复用不额外增加借用成本；`record` 的借用边界
    /// 要读一下就明白：先把 `scratch` 填满，再把各字段**分别**借成局部变量，
    /// 因为同时持有 `&self.scratch` 与 `&mut self.xxx` 会撞上借用检查器。
    scratch: quad::Scratch,
    /// 图集边长（像素）。
    atlas_size: u32,
    /// 图集纹理（所有权）。
    ///
    /// D3D12 **没有**单独的采样视图对象（那是 D3D11 的概念）：SRV 是描述符堆里的
    /// 一个槽位，所以这里只有纹理本体，视图由 `srv_heap` 的槽位隐含表示。
    atlas: ID3D12Resource,
    /// 图集上传的暂存缓冲（上传堆）。
    ///
    /// 必须活得比 `record_upload_atlas` 记录的那次 `CopyTextureRegion` 久 —— 释放了
    /// 就等于让 GPU 去读一段已经归还给驱动的内存。所以挂在渲染器上，由下一次上传替换。
    atlas_staging: Option<UploadBuffer>,
    /// 设备。
    device: ID3D12Device,
    /// 已上传的图像纹理（所有权）。**必须活得比画它的那些命令久**。
    textures: Vec<UploadedTexture>,
    /// 下一个可用的 SRV 槽位。0 号恒给图集（见 [`Renderer::new`]）。
    next_texture_slot: u32,
}

/// 上传堆上一块常驻映射的暂存缓冲。
///
/// # 上传堆的资源**只能**处于 `GENERIC_READ`，不能做状态转换
///
/// 这是 D3D12 的一条硬规则（上传堆的可写性以牺牲状态灵活性为代价）。所以上传堆缓冲
/// **不能**直接当顶点缓冲用 —— 那需要 `VERTEX_AND_CONSTANT_BUFFER` 状态，而这条转换
/// 在上传堆资源上是非法的。
///
/// 因此走 [`StreamBuffer`] 的「上传 → 拷贝 → 默认堆」三步。
struct UploadBuffer {
    resource: ID3D12Resource,
    /// 已映射的 CPU 地址（`resource` 拥有底层内存，这里只是借一个指针）。
    mapped: *mut u8,
}

// 上传缓冲的所有权在 `resource` 上；`mapped` 是它的别名，Rust 不知道这层关系。
// 这里显式声明：该缓冲的 CPU 指针只在单线程记录命令时使用。
unsafe impl Send for UploadBuffer {}
unsafe impl Sync for UploadBuffer {}

/// 逐帧重写的几何缓冲：上传堆暂存 + 默认堆常驻。
///
/// # 每帧固定四步一循环
///
/// ```text
/// CPU 写 staging（常驻映射，无需屏障）
/// staging: GENERIC_READ → COPY_SRC ─┐
/// resident: VERTEX/INDEX → COPY_DEST ├→ CopyBufferRegion
/// resident: COPY_DEST → VERTEX/INDEX ┘
/// staging: COPY_SRC → GENERIC_READ（为下一帧的 CPU 写让路）
/// ```
///
/// # 为什么不直接把上传堆缓冲绑给 draw
///
/// 上传堆资源**不允许**状态转换，而索引缓冲被 draw 读取时要求处于 `INDEX_BUFFER`。
/// 看似"上传堆内存本来就 CPU 可读、直接绑就行"，但零售驱动上会拿到未定义行为 ——
/// 而这正是本项目最讨厌的那类故障（不报错、画面乱）。所以走 `CopyBufferRegion`：
/// 多一次拷贝，换来"每一步状态转换都是文档规定的"，调试层能查、零售驱动不会崩。
///
/// 每帧多出的那次拷贝对 UI 来说可以忽略（几百 KB 量级），而正确性不能。
struct StreamBuffer {
    /// 默认堆上、绘制时处于 `VERTEX_AND_CONSTANT_BUFFER` / `INDEX_BUFFER` 的缓冲。
    resident: ID3D12Resource,
    /// 上传堆上的暂存，常驻映射。
    staging: UploadBuffer,
    /// 常驻缓冲的字节数，也是暂存的字节数。
    size: u64,
    /// 绘制时它必须处于哪个状态（决定屏障的两端）。
    draw_state: D3D12_RESOURCE_STATES,
}

/// 一次 `record` 的输入。
///
/// 打包成结构体而不是六个位置参数：位置参数多到第 5 个之后就没法靠记忆区分了，而
/// 传错 `size` 与 `clear` 的后果（画面拉伸 / 底色错）都不带任何编译期信号。
pub struct RenderCtx<'a> {
    /// 设备（建资源时用；顶点缓冲扩容走这条路）。
    pub device: &'a ID3D12Device,
    /// 记录用的命令列表（必须处于**记录中**状态，即 `Gpu::begin_frame` 之后）。
    pub list: &'a ID3D12GraphicsCommandList,
    /// 目标渲染目标的 CPU 描述符句柄（离屏目标的 `rtv` 或交换链后缓冲的句柄）。
    pub rtv: D3D12_CPU_DESCRIPTOR_HANDLE,
    /// 要回放的命令列表。
    pub display_list: &'a zephyra::paint::DisplayList,
    /// 目标像素尺寸。
    pub size: (u32, u32),
    /// 清屏颜色（线性 RGBA）。
    pub clear: zephyra::core::Color,
    /// 图像纹理表（名字 → SRV 描述符位置）。
    pub textures: &'a TextureTable,
    /// 本帧脏区，用于**帧内** scissor。
    ///
    /// # ⚠ 它不能用来「跳过脏区外的回放」
    ///
    /// 本后端是 flip model（`FLIP_DISCARD`，DComp 路径 `FLIP_SEQUENTIAL`），
    /// `Present` **之后**后缓冲内容是**未定义**的。所以任何要 present 的帧都必须
    /// 重画整个目标 —— 顶点流永远整份提交，只有**光栅化范围**能被裁掉。
    ///
    /// ⚠ 而且 D3D12 的 scissor 与视口**必须同时**设（见下面第 6 步）：只设视口忘了
    /// 裁剪，或反过来，都得到「什么都不画且零错误」。
    pub damage: zephyra::frame::Damage,
}

/// 图像纹理表：名字 → `srv_heap` 里的描述符位置。
///
/// # 与 D3D11 版的差别
///
/// D3D11 版的表直接存 `ID3D11ShaderResourceView` 对象；D3D12 里「SRV」不是对象，而是
/// **描述符堆里的一个槽位**。所以这里存的是 `u32` 位置，`record` 时按位置取 GPU 句柄。
pub struct TextureTable {
    entries: std::collections::HashMap<String, u32>,
}

impl TextureTable {
    /// 新建一张表。
    ///
    /// # 为什么槽位只增不减
    ///
    /// `srv_heap` 的槽位是**描述符堆里的地址**，不是引用计数。忘掉一个名字并不
    /// 意味着那个地址可以被安全地交给别人 —— 驱动的描述符缓存可能还持有指向它的
    /// GPU 可见状态。要回收就得显式走「销毁纹理 + 把槽位标为自由」的流程，
    /// 那需要跟踪哪些槽位是活的（[`Renderer`] 里的 `next_texture_slot` 只增）。
    ///
    /// 对 UI 来说这没关系：图像纹理的种类数是有界的（小图标集），而槽位数在
    /// [`Renderer::new`] 里按 `ATLAS_SLOTS` 固定。**纹理种类超过槽位时报错**，
    /// 而不是静默覆盖别人的槽位。
    pub fn new() -> Self {
        Self {
            entries: std::collections::HashMap::new(),
        }
    }

    /// 登记一个纹理名 → 描述符位置。
    pub fn insert(&mut self, name: String, slot: u32) {
        self.entries.insert(name, slot);
    }

    /// 这个名字有没有登记。
    pub fn contains(&self, name: &str) -> bool {
        self.entries.contains_key(name)
    }

    /// 取描述符位置。
    pub fn slot(&self, name: &str) -> Option<u32> {
        self.entries.get(name).copied()
    }

    /// 忘掉一个名字。返回它原来占的槽位，好让调用方决定是否回收。
    ///
    /// **不**改描述符堆 —— 见 [`Self::insert`] 的说明：槽位只能单调增长。
    pub fn remove(&mut self, name: &str) -> Option<u32> {
        self.entries.remove(name)
    }

    /// 登记了几张纹理。
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 一张都没登记。
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for TextureTable {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderer {
    /// 建根签名、PSO、描述符堆、顶点/索引缓冲与图集纹理。
    ///
    /// 着色器编译失败**必须**冒泡：它唯一的症状是「窗口一片空白 / 全透明」，而那与
    /// 「没有绘制任何东西」看起来一模一样。
    /// `samples` 是 MSAA 采样数（1 = 关）。**必须**与绑定的渲染目标一致 ——
    /// 传 [`crate::swapchain::Swapchain::samples`] 的返回值，别自己写常量。
    pub fn new(
        device: &ID3D12Device,
        atlas_size: u32,
        samples: u32,
    ) -> Result<Self, ShaderError> {
        let shaders = Shaders::new()?;

        // ---- 描述符堆 ----
        //
        // 1 个给图集（`t0` 恒绑它），剩下留给图像纹理。图集固定占 0 号，这样即使一个
        // 图像都没注册，`t0` 也绑着有效对象 —— 着色器在矩形分支不采样它，但「绑着
        // 一个无效 SRV」在某些驱动上会让后续采样返回未定义值。
        const ATLAS_SLOTS: u32 = 8;
        let srv_heap = DescriptorHeap::new_srv(device, ATLAS_SLOTS, true)
            .map_err(|e| ShaderError::Create("建 SRV 描述符堆", e))?;

        // ---- 根签名 ----
        //
        // 一个描述符表（`t0`，一个 SRV）+ 一个静态采样器（`s0`）。静态采样器直接写在
        // 根签名里，就不需要单独的采样器堆与 `CreateSampler` —— 对我们这种「采样器永不
        // 改变」的场景是纯粹的简化。
        let root_signature = Self::create_root_signature(device)
            .map_err(|e| ShaderError::Create("建根签名", e))?;

        // ---- PSO ----
        let pso = Self::create_pso(device, &root_signature, shaders.elements.descs(), samples)
            .map_err(|e| ShaderError::Create("建 PSO", e))?;

        // ---- 顶点 / 索引缓冲 ----
        //
        // 起始 64KB 与 4KB。稳态下按需增长到 2 的幂，之后不再动（见 `record`）。
        let vertex = StreamBuffer::new(device, 64 * 1024, D3D12_RESOURCE_STATE_VERTEX_AND_CONSTANT_BUFFER)
            .map_err(|e| ShaderError::Create("建顶点缓冲", e))?;
        let index = StreamBuffer::new(device, 4 * 1024, D3D12_RESOURCE_STATE_INDEX_BUFFER)
            .map_err(|e| ShaderError::Create("建索引缓冲", e))?;
        let vertex_capacity = vertex.size;

        // ---- 图集纹理 ----
        //
        // R8 单通道（字形位图只需覆盖率），与 D3D11 版一致。
        let atlas = create_atlas(device, atlas_size).map_err(|e| ShaderError::Create("建图集纹理", e))?;
        unsafe {
            device.CreateShaderResourceView(
                &atlas,
                None,
                srv_heap.cpu(0).map_err(|e| ShaderError::Create("取图集 SRV 槽", e))?,
            );
        }

        Ok(Self {
            root_signature,
            pso,
            srv_heap,
            atlas_srv_slot: 0,
            vertex,
            index,
            vertex_capacity,
            scratch: quad::Scratch::new(),
            atlas_size,
            atlas,
            atlas_staging: None,
            device: device.clone(),
            textures: Vec::new(),
            next_texture_slot: 1,
        })
    }

    /// 建根签名：一个 `t0` 描述符表 + 一个 `s0` 静态采样器。
    fn create_root_signature(device: &ID3D12Device) -> Result<ID3D12RootSignature, SurfaceError> {
        let ranges = [D3D12_DESCRIPTOR_RANGE {
            RangeType: D3D12_DESCRIPTOR_RANGE_TYPE_SRV,
            NumDescriptors: 1,
            BaseShaderRegister: 0, // 与 `Texture2D g_tex : register(t0)` 对应
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
        // 静态采样器：与 `SamplerState g_smp : register(s0)` 对应。
        //
        // 过滤取线性、寻址取夹取（clamp）。夹取而非环绕：字形图集的 UV 在 [0,1] 内，
        // 但边缘像素的采样器权重会溢出到邻格 —— 环绕会从图集另一侧取到字，开取会把
        // 图集外的黑色渲进来。两者都会在图集边界上看到「鬼影」。
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
            MaxLOD: f32::MAX, // D3D12_LOD_CLAMP_NONE
            ShaderRegister: 0, // s0
            RegisterSpace: 0,
            ShaderVisibility: D3D12_SHADER_VISIBILITY_PIXEL,
        };
        let desc = D3D12_ROOT_SIGNATURE_DESC {
            NumParameters: 1,
            pParameters: params.as_ptr(),
            NumStaticSamplers: 1,
            pStaticSamplers: &sampler,
            // `ALLOW_INPUT_ASSEMBLER_INPUT_LAYOUT`：根签名只声明了片元可见的
            // `t0`，而 PSO 里**有**顶点输入布局。没有这个标志时，D3D12 允许「有输入
            // 布局但根签名没为它留位」，但版本差异多；加上它最省心，且与 D3D11 的行为
            // 一致（D3D11 没有这一层校验）。
            Flags: D3D12_ROOT_SIGNATURE_FLAG_ALLOW_INPUT_ASSEMBLER_INPUT_LAYOUT,
        };

        let mut blob = None::<windows::Win32::Graphics::Direct3D::ID3DBlob>;
        let mut err_blob = None::<windows::Win32::Graphics::Direct3D::ID3DBlob>;
        unsafe {
            D3D12SerializeRootSignature(&desc, D3D_ROOT_SIGNATURE_VERSION_1_0, &mut blob, Some(&mut err_blob))
                .map_err(|e| {
                    // 把驱动的错误 blob 转成可读消息：根签名序列化失败时，HRESULT
                    // 常是 `E_INVALIDARG`，真正的原因在 blob 里。
                    SurfaceError::new(
                        "D3D12SerializeRootSignature",
                        windows::core::Error::new(
                            e.code(),
                            blob_message(&err_blob, "根签名描述本身就不合法"),
                        ),
                    )
                })?;
        }
        let blob = blob.ok_or_else(|| {
            SurfaceError::new(
                "根签名序列化成功却没给 blob",
                windows::core::Error::new(windows::Win32::Foundation::E_FAIL, "no blob"),
            )
        })?;
        // `CreateRootSignature` 收的是**字节**，而 `D3D12SerializeRootSignature` 给的是
        // 一个 `ID3DBlob`。所以要把 blob 的内容拷出来 —— 注意必须拷，因为 blob 在函数
        // 返回时就被释放了。
        let ptr = unsafe { blob.GetBufferPointer() };
        let len = unsafe { blob.GetBufferSize() };
        if ptr.is_null() || len == 0 {
            return Err(SurfaceError::new(
                "根签名 blob 是空的",
                windows::core::Error::new(windows::Win32::Foundation::E_FAIL, "empty blob"),
            ));
        }
        let bytes = unsafe { std::slice::from_raw_parts(ptr.cast::<u8>(), len) }.to_vec();
        unsafe {
            device
                .CreateRootSignature(0, &bytes)
                .map_err(|e| SurfaceError::new("CreateRootSignature", e))
        }
    }

    /// 建图形 PSO：着色器字节码 + 输入布局 + 混合 + 光栅化。
    fn create_pso(
        device: &ID3D12Device,
        root_signature: &ID3D12RootSignature,
        elements: &[D3D12_INPUT_ELEMENT_DESC],
        samples: u32,
    ) -> Result<ID3D12PipelineState, SurfaceError> {
        // 混合状态与 D3D11 版逐项一致：`Src/InvSrc = ALPHA/INV_SRC_ALPHA`，
        // alpha 用 `ONE/INV_SRC_ALPHA`（预乘 alpha 的正确组合，交换链用
        // `DXGI_ALPHA_MODE_PREMULTIPLIED`）。alpha 也写成 `ALPHA/INV_SRC_ALPHA` 会让
        // 半透明区域的 alpha 变成 `a*a`，表现为整屏发暗。
        let rt_blend = D3D12_RENDER_TARGET_BLEND_DESC {
            BlendEnable: windows::core::BOOL(1),
            LogicOpEnable: windows::core::BOOL(0),
            SrcBlend: D3D12_BLEND_SRC_ALPHA,
            DestBlend: D3D12_BLEND_INV_SRC_ALPHA,
            BlendOp: D3D12_BLEND_OP_ADD,
            SrcBlendAlpha: D3D12_BLEND_ONE,
            DestBlendAlpha: D3D12_BLEND_INV_SRC_ALPHA,
            BlendOpAlpha: D3D12_BLEND_OP_ADD,
            LogicOp: D3D12_LOGIC_OP_NOOP,
            // windows-rs 0.62 把写掩码声明成了**裸 `u8`**（不是 `D3D12_COLOR_WRITE_ENABLE`），
            // 与 D3D11 那边的怪癖同源。
            RenderTargetWriteMask: D3D12_COLOR_WRITE_ENABLE_ALL.0 as u8,
        };
        let blend = D3D12_BLEND_DESC {
            AlphaToCoverageEnable: windows::core::BOOL(0),
            IndependentBlendEnable: windows::core::BOOL(0),
            RenderTarget: [rt_blend; 8],
        };

        // 光栅化：关背面剔除 —— 圆角 SDF 在边缘产生的三角形可能退化，剔除会让圆角
        // 出现缺口（表现为四边形的角被啃掉一块）。
        let raster = D3D12_RASTERIZER_DESC {
            FillMode: D3D12_FILL_MODE_SOLID,
            CullMode: D3D12_CULL_MODE_NONE,
            FrontCounterClockwise: windows::core::BOOL(0),
            DepthBias: 0,
            DepthBiasClamp: 0.0,
            SlopeScaledDepthBias: 0.0,
            DepthClipEnable: windows::core::BOOL(1),
            // ⚠ MSAA。**必须**与实际绑定的渲染目标采样数一致 —— D3D12 在
            // `OMSetRenderTargets` / `DrawIndexedInstanced` 处逐项比对，不一致时
            // 报「render target format does not match」而画面全错。
            //
            // `samples` 来自宿主传进来的 [`crate::swapchain::Swapchain::samples`]，
            // 所以两边必然同源。写成常量 1 而实际绑单采样是本条最危险的写法：
            // D3D12 会**解除渲染目标绑定**，症状是「窗口全黑且零 HRESULT」。
            MultisampleEnable: windows::core::BOOL((samples > 1) as i32),
            AntialiasedLineEnable: windows::core::BOOL(0),
            ForcedSampleCount: 0,
            ConservativeRaster: D3D12_CONSERVATIVE_RASTERIZATION_MODE_OFF,
        };

        let desc = D3D12_GRAPHICS_PIPELINE_STATE_DESC {
            // `ManuallyDrop`：PSO 描述里的 COM 指针**不**拥有所指对象 —— 根签名的所有权在
            // `Renderer::root_signature` 上。所以这里必须手动包一层，不能用 `From`。
            pRootSignature: std::mem::ManuallyDrop::new(Some(root_signature.clone())),
            VS: D3D12_SHADER_BYTECODE {
                pShaderBytecode: shader::VS_BYTECODE.as_ptr() as *const _,
                BytecodeLength: shader::VS_BYTECODE.len(),
            },
            PS: D3D12_SHADER_BYTECODE {
                pShaderBytecode: shader::PS_BYTECODE.as_ptr() as *const _,
                BytecodeLength: shader::PS_BYTECODE.len(),
            },
            DS: D3D12_SHADER_BYTECODE::default(),
            HS: D3D12_SHADER_BYTECODE::default(),
            GS: D3D12_SHADER_BYTECODE::default(),
            StreamOutput: D3D12_STREAM_OUTPUT_DESC::default(),
            BlendState: blend,
            SampleMask: u32::MAX, // 全采样掩码（D3D12 必填，非 0 即「什么都不画」）
            RasterizerState: raster,
            // 不用深度缓冲：UI 全部靠 painter's algorithm 排序，D3D12 没有深度缓冲
            // 就没有「没清深度导致残留」这类问题。
            DepthStencilState: D3D12_DEPTH_STENCIL_DESC::default(),
            InputLayout: D3D12_INPUT_LAYOUT_DESC {
                pInputElementDescs: elements.as_ptr(),
                NumElements: elements.len() as u32,
            },
            IBStripCutValue: D3D12_INDEX_BUFFER_STRIP_CUT_VALUE_DISABLED,
            PrimitiveTopologyType: D3D12_PRIMITIVE_TOPOLOGY_TYPE_TRIANGLE,
            NumRenderTargets: 1,
            // ⚠ 只有第 0 槽是真实格式，第 1..8 槽**必须**是 `UNKNOWN`。
            //
            // 调试层的原话：
            //     "ID3D12Device::CreateGraphicsPipelineState: RTVFormats[] must be
            //      DXGI_FORMAT_UNKNOWN for all indices >= NumRenderTargets."
            //
            // 之前 8 个槽全填了 `R8G8B8A8_UNORM`，于是本机每一个 PSO 都被
            // `E_INVALIDARG` 拒掉 —— 而**没有任何提示指向这个字段**。这是同一个模式
            // 的第六次：把自己写的非法组合当成了环境的静默失效。证据链与复现命令见
            // `tests/debug_probe.rs`。
            RTVFormats: {
                let mut f = [windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_UNKNOWN; 8];
                f[0] = windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_R8G8B8A8_UNORM;
                f
            },
            DSVFormat: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_UNKNOWN,
            // ⚠ 必须与 `MultisampleEnable` 和实际 RTV 的 `SampleDesc` **三者一致**。
            //
            // D3D12 允许 PSO 的 Count 大于 RTV 的（那样只是不做 MSAA），但**不允许
            // 小于** —— 后者在 `CreateGraphicsPipelineState` 阶段就可能被拒，或在
            // 绘制时逐槽报「format does not match」。
            SampleDesc: windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC {
                Count: samples,
                Quality: 0,
            },
            // 单 GPU 就是 0（驱动会转成 1）。理由见 [`PSO_NODE_MASK`]。
            NodeMask: PSO_NODE_MASK,
            CachedPSO: D3D12_CACHED_PIPELINE_STATE::default(),
            Flags: D3D12_PIPELINE_STATE_FLAG_NONE,
        };
        unsafe {
            device
                .CreateGraphicsPipelineState(&desc)
                .map_err(|e| SurfaceError::new("CreateGraphicsPipelineState", e))
        }
    }

    /// 图集边长（像素）。
    pub fn atlas_size(&self) -> u32 {
        self.atlas_size
    }

    /// 顶点缓冲容量（字节）。稳态下跨帧不变。
    pub fn vertex_capacity(&self) -> u64 {
        self.vertex_capacity
    }

    /// 把字形图集位图上传到 GPU 纹理（R8，`atlas_size × atlas_size`）。
    ///
    /// # 为什么纹理上传要走「上传缓冲 + placed footprint」而不是直接映射纹理
    ///
    /// 只有 `UPLOAD` 堆的资源能被 CPU 写，而纹理**不能**建在上传堆里 —— 上传堆只支持
    /// 缓冲。所以纹理上传的固定形态是：
    ///
    /// ```text
    /// CPU → (upload 缓冲, 按 footprint 摆放)  → CopyTextureRegion  → 图集纹理
    /// ```
    ///
    /// # 为什么必须用驱动给的行距
    ///
    /// `GetCopyableFootprints` 返回的 `RowPitch` 是驱动挑的（1024 宽的 R8 纹理常常是
    /// 1024 而恰好等于紧密行距，但换个分辨率就不等了）。按紧密行距连续写进去，内容会
    /// **斜着**排在缓冲里 —— 症状是字形整体倾斜、且倾斜量随行号累积，极难定位。
    ///
    /// # 调用时机
    ///
    /// **记录**命令（不提交）。调用方负责 `Gpu::submit` + `Gpu::wait_gpu`，这样
    /// 「记录 / 提交 / 等待」三步在测试里都看得见。
    ///
    /// # 为什么整张上传而不是只传新增区域
    ///
    /// 图集会被**整体重排**（LRU 淘汰之后按高度重排），那时几乎每个字形的 uv 都变了，
    /// 增量上传无从下手。代价是每次变化多一次 4MB 上传；稳态下图集不再变化，这个上传
    /// 也就不再发生 —— 宿主用 [`zephyra::text::TextStack::atlas_revision`] 判断，
    /// **不能**用字形数（淘汰可能让数量不变而内容全变）。
    pub fn record_upload_atlas(
        &mut self,
        list: &ID3D12GraphicsCommandList,
        pixels: &[u8],
    ) -> Result<(), SurfaceError> {
        let side = self.atlas_size;
        // ⚠ 行宽是 `side * 4`，不是 `side`。
        //
        // 图集是 RGBA8（覆盖率写在四个通道里），所以一行 `side` 个像素占 `side*4`
        // 字节。这里原先按 `side` 算 —— 于是 `CopyTextureRegion` 源缓冲区只有实际需要的
        // 1/4，症状是「上传越界」或「图集只填了左上角一条」。
        let bytes_per_px = 4usize;
        let tight = side as usize * bytes_per_px;
        let height = side as usize;

        // 图集纹理只当 SRV 采样，不需要任何 bind flag（D3D12 里 shader 可见的资源
        // 默认就是 `FLAG_NONE`；`ALLOW_UNORDERED_ACCESS` 才会互斥）。
        let tex_desc = crate::surface::texture_desc((side, side), D3D12_RESOURCE_FLAG_NONE);

        // 行距与总量都由驱动挑，且**必须**按它摆放 / 按它定暂存大小。
        let mut footprint = D3D12_PLACED_SUBRESOURCE_FOOTPRINT::default();
        let mut num_rows = 0u32;
        let mut row_size = 0u64;
        let mut total = 0u64;
        unsafe {
            self.device.GetCopyableFootprints(
                &tex_desc,
                0,
                1,
                0,
                Some(&mut footprint),
                Some(&mut num_rows),
                Some(&mut row_size),
                Some(&mut total),
            );
        }
        let row_pitch = footprint.Footprint.RowPitch as usize;
        if row_pitch < tight {
            return Err(SurfaceError::new(
                "图集上传：驱动给的行距装不下一行像素",
                windows::core::Error::new(
                    E_FAIL,
                    format!("行距 {row_pitch} < 一行 {tight} 字节"),
                ),
            ));
        }

        // ⚠ 暂存是**上传堆**缓冲，**直接**当拷贝源 —— 中间不经过 DEFAULT 堆的副本。
        //
        // 原先这里建的是 `StreamBuffer`（上传堆 + DEFAULT 堆两份），像素写进了上传堆的
        // 映射内存，`CopyTextureRegion` 却从 `resident`（DEFAULT 堆那份、**从未被写过**）
        // 拷。于是每次上传都「成功」，而图集永远是 `CreateCommittedResource` 时的内容
        // （零）—— 采样覆盖率恒为 0，文字整批消失且**没有任何报错**。
        //
        // 上传堆资源恒为 `GENERIC_READ`，而 `GENERIC_READ` 正是拷贝源要的状态，所以
        // 一条屏障都不需要。原先那条 `GENERIC_READ → COPY_SOURCE` 反而是**非法**的
        // （上传堆资源不允许状态转换）。
        let staging = UploadBuffer::new(&self.device, total.max((tight * height) as u64))?;

        // 按行距逐行写进映射内存。
        //
        // `n` 是**字节**数，不是像素数 —— 所以还要除以 `bytes_per_px` 才得到
        // "源数据里有几行"。这里早先写成 `side * height`（像素数），于是每个像素
        // 只写了 1/4 个字节，图集变成一条条斜纹。
        let have_bytes = (tight * height).min(pixels.len());
        let rows_usable = have_bytes / tight;
        unsafe {
            let dst = staging.mapped;
            for row in 0..rows_usable.min(height) {
                std::ptr::copy_nonoverlapping(
                    pixels.as_ptr().add(row * tight),
                    dst.add(row * row_pitch),
                    tight,
                );
            }

            gpu_barrier(
                list,
                &self.atlas,
                D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE,
                D3D12_RESOURCE_STATE_COPY_DEST,
            );
            list.CopyTextureRegion(
                &D3D12_TEXTURE_COPY_LOCATION {
                    pResource: std::mem::ManuallyDrop::new(Some(self.atlas.clone())),
                    Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
                    Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 { SubresourceIndex: 0 },
                },
                0,
                0,
                0,
                &D3D12_TEXTURE_COPY_LOCATION {
                    pResource: std::mem::ManuallyDrop::new(Some(staging.resource.clone())),
                    Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
                    Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 { PlacedFootprint: footprint },
                },
                None,
            );
            gpu_barrier(
                list,
                &self.atlas,
                D3D12_RESOURCE_STATE_COPY_DEST,
                D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE,
            );
        }
        // 暂存缓冲必须活得比这次拷贝久。把它挂到渲染器上，由下一帧开头替换。
        self.atlas_staging = Some(staging);
        Ok(())
    }

    /// 上传一张图像纹理，返回它在 `srv_heap` 里的槽位。
    ///
    /// **只记录，不提交** —— 与 [`Self::record`] / [`Self::record_upload_atlas`] 同一个
    /// 约定。调用方负责提交并等 GPU 读完（见 [`crate::surface::Gpu::submit_one_off`]）。
    ///
    /// # 三步，与图集上传同构
    ///
    /// 1. 纹理建在**默认堆**，初始状态 `COPY_DEST`；
    /// 2. 上传堆暂存按驱动给的 `RowPitch` 逐行填，屏障转 `COPY_SOURCE` 后
    ///    `CopyTextureRegion`；
    /// 3. 转回 `PIXEL_SHADER_RESOURCE`，并 `CreateShaderResourceView` 写进堆槽。
    ///
    /// # 为什么第 2 步不能用 `WriteToSubresource`
    ///
    /// 那个 API 也能一步写完，但它是 **D3D11 风格的同步拷贝**：内部隐式等待
    /// 「GPU 不再引用该资源」，也就是一个隐式的 fence。对启动期一次性上传这无所谓，
    /// 可它把「一次隐式全局等待」藏在一个看起来像普通 memcpy 的调用里 ——
    /// 与本项目「每一步都能被调试层查到」的取舍相反。
    ///
    /// # 尺寸不做对齐
    ///
    /// 图标是 32×32、任意小图也是。`GetCopyableFootprints` 的行距必须按它填，
    /// 但纹理本身不必补齐到 2 的幂 —— 非幂尺寸在 `CreateShaderResourceView` 里
    /// 只需注意 `MipLevels = 1`。
    pub fn upload_texture(
        &mut self,
        list: &ID3D12GraphicsCommandList,
        pixels: &[u8],
        w: u32,
        h: u32,
    ) -> Result<u32, SurfaceError> {
        if w == 0 || h == 0 {
            return Err(SurfaceError::new(
                "上传纹理",
                windows::core::Error::new(E_INVALIDARG, "宽高不能为 0"),
            ));
        }
        let tight = w as usize * 4;
        let need = tight * h as usize;
        if pixels.len() < need {
            return Err(SurfaceError::new(
                "上传纹理",
                windows::core::Error::new(
                    E_INVALIDARG,
                    format!("像素 {need} 字节，实收 {}", pixels.len()),
                ),
            ));
        }
        let slot = self.next_texture_slot;
        if slot >= self.srv_heap.count() {
            // 静默覆盖别人的槽位 = 「图标随机变成另一张图」，几乎无法归因。直接报错。
            return Err(SurfaceError::new(
                "上传纹理",
                windows::core::Error::new(
                    E_FAIL,
                    format!(
                        "SRV 堆只有 {} 个槽位（0 号给图集），已用尽",
                        self.srv_heap.count()
                    ),
                ),
            ));
        }
        self.next_texture_slot = slot + 1;

        let tex_desc = crate::surface::texture_desc((w, h), D3D12_RESOURCE_FLAG_NONE);
        let texture = create_resource(
            &self.device,
            &heap_props(D3D12_HEAP_TYPE_DEFAULT),
            &tex_desc,
            D3D12_RESOURCE_STATE_COPY_DEST,
        )?;

        // ⚠ 行距与总量都必须用驱动给的，不能用 `tight * h`。
        //
        // 拷贝源是 `PLACED_FOOTPRINT` 时，D3D12 按 `GetCopyableFootprints` 返回的
        // `RowPitch` 读 —— 那是按 256 字节对齐过的。32×32 的图每行 128 字节，
        // 而驱动要 256，于是：
        //
        // - 按 `tight` 摆数据 → 每隔一行横向偏 128 字节（图像内容斜着错位）；
        // - 暂存缓冲按 `tight * h`（4096）建 → 驱动要 8064，`CopyTextureRegion`
        //   越界。
        //
        // 这两条**不报 HRESULT**，只有调试层会说 —— 而 `E_INVALIDARG` 那条只在
        // 内存访问真正越出去时才触发。所以先算 footprint、**再**按 `total` 定暂存
        // 大小，而不是反过来。
        let mut footprint = D3D12_PLACED_SUBRESOURCE_FOOTPRINT::default();
        let mut num_rows = 0u32;
        let mut row_size = 0u64;
        let mut total = 0u64;
        unsafe {
            self.device.GetCopyableFootprints(
                &tex_desc,
                0,
                1,
                0,
                Some(&mut footprint),
                Some(&mut num_rows),
                Some(&mut row_size),
                Some(&mut total),
            );
        }
        let row_pitch = footprint.Footprint.RowPitch as usize;
        if row_pitch < tight {
            return Err(SurfaceError::new(
                "上传纹理：驱动给的行距装不下一行像素",
                windows::core::Error::new(
                    E_FAIL,
                    format!("行距 {row_pitch} < 一行 {tight} 字节"),
                ),
            ));
        }
        let staging = UploadBuffer::new(&self.device, total.max(need as u64))?;

        unsafe {
            for row in 0..h as usize {
                std::ptr::copy_nonoverlapping(
                    pixels.as_ptr().add(row * tight),
                    staging.mapped.add(row * row_pitch),
                    tight,
                );
            }
            // ⚠ 这里**不发** `GENERIC_READ → COPY_SOURCE` 屏障：上传堆资源只能处于
            // `GENERIC_READ`，状态转换本身是非法的。`GENERIC_READ` 正是拷贝源要的
            // 状态，直接拷即可。
            list.CopyTextureRegion(
                &D3D12_TEXTURE_COPY_LOCATION {
                    pResource: std::mem::ManuallyDrop::new(Some(texture.clone())),
                    Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
                    Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 { SubresourceIndex: 0 },
                },
                0,
                0,
                0,
                &D3D12_TEXTURE_COPY_LOCATION {
                    pResource: std::mem::ManuallyDrop::new(Some(staging.resource.clone())),
                    Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
                    Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 { PlacedFootprint: footprint },
                },
                None,
            );
            // 拷贝**之后**才能转回 PS 资源状态。转早了 GPU 会在拷贝中途重新排布它 ——
            // 而那次重排与拷贝的先后关系没有任何东西记录，症状是随机花屏。
            gpu_barrier(
                list,
                &texture,
                D3D12_RESOURCE_STATE_COPY_DEST,
                D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE,
            );
        }

        // 描述符**必须在记录之外**建：它是 CPU 侧立刻生效的状态，不是命令。
        //
        // 顺序上也无所谓先后（描述符内容对已记录的命令立刻可见），但要放在
        // `next_texture_slot` 递增之后 —— 万一上面某步已经失败了，就不该占掉槽位。
        //
        // 视图描述传 `None`：D3D12 会从**资源**推出格式与尺寸，覆盖全部 mip
        // （`texture_desc` 只建了 1 层）。手写一遍 `D3D12_SHADER_RESOURCE_VIEW_DESC`
        // 只会多一处「和资源格式对不上就花屏」的失同步点 —— 而格式一致性正是
        // 前面几个 bug 的同一类成因。
        unsafe {
            self.device
                .CreateShaderResourceView(&texture, None, self.srv_heap.cpu(slot)?);
        }

        self.textures.push(UploadedTexture {
            _resource: texture,
            _staging: staging,
            _slot: slot,
        });
        Ok(slot)
    }

    /// 回放一份 `DisplayList` 并**记录**绘制命令（不提交）。
    ///
    /// `ctx.size` 是目标的**像素**尺寸 —— 顶点在此从 DIP 转到 NDC。
    pub fn record(&mut self, ctx: RenderCtx<'_>) -> Result<RenderReport, SurfaceError> {
        let RenderCtx {
            device,
            list,
            rtv,
            display_list,
            size,
            clear,
            textures,
            damage,
        } = ctx;

        // ---- 1. 展开成顶点流 ----
        //
        // 走复用路径（[`quad::expand_into`]）：`self.scratch` 跨帧持有全部中间缓冲，
        // 稳态下这一段**零分配**。
        //
        // 为什么不直接用纯函数 `quad::expand`：那会每帧新开六七个 `Vec` 并从 0 长到
        // 目标大小（demo 实测每帧约 170 KB 的分配，其中约 69% 出在这里）。
        // 而本方法已经是 `&mut self`，复用不额外增加借用成本。
        let textures_ref = &textures;
        let mut report = quad::expand_into(display_list, size, &|n| textures_ref.contains(n), &mut self.scratch);
        let scratch = &self.scratch;
        let image_first = scratch.image_first;
        let path_index_start = scratch.path_index_start;
        let all = &scratch.vertices;
        let indices = &scratch.indices;
        let path_tris = scratch.paths.len() / 3;
        report.vertex_bytes = (all.len() * std::mem::size_of::<Vertex>()) as u64;
        let batches: &[(String, u32)] = &scratch.batches;

        // ---- 2. 顶点缓冲按需增长，但不缩 ----
        //
        // 稳态零分配的前提。扩容会重建资源，所以顶点已经写好的内容会丢 —— 所以扩容
        // 发生在**写入**之前。
        let needed = report.vertex_bytes.max(64);
        if needed > self.vertex_capacity {
            let grown = needed.next_power_of_two();
            match StreamBuffer::new(device, grown, D3D12_RESOURCE_STATE_VERTEX_AND_CONSTANT_BUFFER) {
                Ok(b) => {
                    self.vertex = b;
                    self.vertex_capacity = grown;
                    report.reallocated = true;
                }
                Err(_) => {
                    // 建失败就退回旧缓冲：本帧可能画不全，但不该整帧崩掉。
                }
            }
        }

        // ---- 3. 索引缓冲 ----
        //
        // 索引要逐四边形加顶点基址（见 `quad::expand_indices`），而四边形数逐帧可变，
        // 所以索引每帧都不同、必须逐帧重写。
        //
        // ⚠ 四边形数必须**只数 solid + images** —— 用 `vertices.len() / 4` 会把路径
        // 顶点也除进去，而路径顶点数是 3 的倍数，整数截断会让四边形段的基址全部错位
        // （症状：路径一出现，它前面所有图元一起变乱码）。
        if !indices.is_empty() {
            let need = (indices.len() * 2) as u64;
            if need > self.index.size {
                let grown = need.next_power_of_two();
                if let Ok(b) = StreamBuffer::new(device, grown, D3D12_RESOURCE_STATE_INDEX_BUFFER) {
                    self.index = b;
                }
            }
        }

        // ---- 4. 顶点/索引上传 ----
        //
        // 两者的屏障由 [`StreamBuffer::record_upload`] 自己管（详见那里的四步循环）。
        // 顺序有讲究：几何数据必须**先**到位，后面才是绑定与绘制。
        self.vertex.record_upload(list, bytemuck::cast_slice(&all));
        self.index.record_upload(list, bytemuck::cast_slice(&indices));

        // ---- 5. 绑定管线状态 ----
        unsafe {
            list.SetGraphicsRootSignature(&self.root_signature);
            list.SetPipelineState(&self.pso);
            list.SetDescriptorHeaps(&[Some(self.srv_heap.heap.clone())]);
            // 根签名里 `t0` 绑到 `srv_heap` 第 `atlas_srv_slot` 号。
            list.SetGraphicsRootDescriptorTable(0, self.srv_heap.gpu(self.atlas_srv_slot)?);
        }

        // ---- 6. 视口 + 裁剪（必须成对）----
        //
        // D3D12 **没有** `ScissorEnable` 开关：裁剪永远生效。所以只设视口、忘了裁剪，
        // 就得到「默认裁剪矩形是空的 → 什么都不画且零错误」——与 D3D11 视口坑同一类。
        let viewport = [D3D12_VIEWPORT {
            TopLeftX: 0.0,
            TopLeftY: 0.0,
            Width: size.0 as f32,
            Height: size.1 as f32,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        }];
        // ⚠️ scissor 必须在 `ClearRenderTargetView` **之前**设：clear 也受 scissor
        // 约束，顺序反了会得到「清屏是全屏、绘制是脏区」—— 而脏区外的像素来自上一帧，
        // 那正是 flip model 下未定义的内容。
        //
        // DIP → 像素的比例**不能假设为 1**（高 DPI 下同一个 `DisplayList` 会被放大，
        // 按 1 算会让脏区只有实际需要的 1/k，症状是「改色的那一小块没变干净」）。
        let scissor = [match damage {
            zephyra::frame::Damage::Full => RECT {
                left: 0,
                top: 0,
                right: size.0 as i32,
                bottom: size.1 as i32,
            },
            zephyra::frame::Damage::Rect(r) => {
                let sx = size.0 as f32 / viewport[0].Width.max(1e-6);
                let sy = size.1 as f32 / viewport[0].Height.max(1e-6);
                let l = (r.x * sx).floor().max(0.0) as i32;
                let t = (r.y * sy).floor().max(0.0) as i32;
                let rt = ((r.x + r.width) * sx).ceil() as i32;
                let b = ((r.y + r.height) * sy).ceil() as i32;
                // ⚠️ 退化矩形回退到全屏，而不是照设：D3D12 的 scissor 是半开区间
                // [left, right)，right <= left 时**什么都不画**且零错误 ——
                // 症状是元素凭空消失。
                if rt > l && b > t {
                    RECT {
                        left: l,
                        top: t,
                        right: rt,
                        bottom: b,
                    }
                } else {
                    RECT {
                        left: 0,
                        top: 0,
                        right: size.0 as i32,
                        bottom: size.1 as i32,
                    }
                }
            }
        }];
        unsafe {
            list.RSSetViewports(&viewport);
            list.RSSetScissorRects(&scissor);
        }

        // ---- 7. 绑定渲染目标并清屏 ----
        //
        // 清屏色。`Color` 存的是**线性** 0..1，而 `ClearRenderTargetView` 要的也是线性
        // —— 这里不做任何编码转换。
        unsafe {
            list.OMSetRenderTargets(1, Some(&rtv), false, None);
            list.ClearRenderTargetView(rtv, &[clear.r, clear.g, clear.b, clear.a], None);
        }

        // ---- 8. 绑定顶点/索引缓冲 ----
        let stride = std::mem::size_of::<Vertex>() as u32;
        unsafe {
            list.IASetPrimitiveTopology(windows::Win32::Graphics::Direct3D::D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            // `IASetVertexBuffers` 要 `Option<&[..]>` 而不是 `&[..]` —— windows-rs 0.62 用
            // `Option` 来表达「传空指针」。传 `None` 等于解绑全部顶点缓冲：不报错，
            // 只是整帧什么都不画。
            list.IASetVertexBuffers(
                0,
                Some(&[D3D12_VERTEX_BUFFER_VIEW {
                    BufferLocation: self.vertex.gpu_addr(),
                    SizeInBytes: self.vertex_capacity as u32,
                    StrideInBytes: stride,
                }]),
            );
            // 索引缓冲**必须**在 draw 之前绑定 —— 漏掉它不报错，只是整帧什么都不画。
            // 症状是"窗口只剩清屏色，而 RenderReport 里 quads 一切正常"。
            list.IASetIndexBuffer(Some(&D3D12_INDEX_BUFFER_VIEW {
                BufferLocation: self.index.gpu_addr(),
                SizeInBytes: self.index.size as u32,
                Format: DXGI_FORMAT_R16_UINT,
            }));
        }

        // ---- 9. 矩形 + 字形：一次 draw ----
        //
        // 它们不采样纹理（`kind == 0` 那条分支根本不看 `g_tex`），但片元着色器仍然声明
        // 了 `g_tex`，所以 `t0` 必须绑着**有效**对象 —— 已由第 5 步绑图集兜住。
        let solid_quads = (scratch.solid.len() / 4) as u32;
        if solid_quads > 0 {
            // `DrawIndexedInstanced` 而非 `DrawIndexed`：D3D12 只有带实例参数的版本
            // （`ID3D12GraphicsCommandList` 没有 `DrawIndexed`）。这里 `instancecount`
            // 恒为 1 —— 实例化在这里没有意义，索引已逐四边形展开。
            unsafe {
                list.DrawIndexedInstanced(solid_quads * 6, 1, 0, 0, 0);
            }
        }

        // ---- 10. 图像：每种纹理一批 ----
        //
        // 一批一次 draw，因为一次 draw 只能绑一张 SRV。批内是 `DisplayList` 的连续
        // 切片（顺序由 `quad::expand` 保证），所以把不连续的同纹理合批会改变可见结果
        // —— 半透明图像最明显。
        //
        // `cursor` 是**四边形**计数（`batch.batches` 里记的是顶点数），换算成索引偏移
        // 要乘 6。混用这两个单位是本文件最容易搞错的地方：错的时候 `startindex` 仍是
        // 个合法值，只是不报错地画错位置。
        let mut cursor_quads = image_first / 4;
        for (name, n) in batches {
            if *n == 0 {
                continue;
            }
            // 顶点收集阶段已过滤掉未注册纹理，所以这里必然查得到。
            let Some(slot) = textures.slot(name) else {
                continue;
            };
            let quads = *n / 4;
            unsafe {
                list.SetGraphicsRootDescriptorTable(0, self.srv_heap.gpu(slot)?);
                list.DrawIndexedInstanced(quads * 6, 1, cursor_quads * 6, 0, 0);
            }
            cursor_quads += quads;
        }

        // ---- 11. 多边形：一次 draw ----
        //
        // 独立成段的原因：顶点数不是 4 的倍数、索引规则也不同
        // （见 `quad::expand_indices_with_paths`）。
        //
        // ⚠ 同样必须把 `t0` 绑成有效对象（哪怕这一分支不读纹理）：`g_tex` 在 PS 里
        // 被声明了，`t0` 悬空会让某些驱动解除引用并崩溃。这里重绑图集兜住。
        if path_tris > 0 {
            unsafe {
                list.SetGraphicsRootDescriptorTable(0, self.srv_heap.gpu(self.atlas_srv_slot)?);
                list.DrawIndexedInstanced(
                    path_tris as u32 * 3,
                    1,
                    path_index_start,
                    0,
                    0,
                );
            }
        }

        // 图像批可能把 `t0` 留在最后一张图上。下一帧的 `record` 会重绑图集，所以不刻意
        // 恢复 —— **但**这只在「命令列表的完整生命周期就是 `record` → `submit`」时成立。
        // 若宿主日后复用同一条命令列表做别的绘制，这里就得补一次重绑。

        Ok(report)
    }
}

/// 上传缓冲的实现。
impl UploadBuffer {
    /// 在上传堆上建一块 `size` 字节的缓冲并**立即映射**。
    ///
    /// 上传堆保证 CPU 可写，所以映射传 `None`（整块）即可，不需要写类型参数 ——
    /// D3D12 的读写方向由**堆类型**决定，不像 D3D11 的 `Map` 还要再传一个
    /// `D3D11_MAP_WRITE_DISCARD`。
    fn new(device: &ID3D12Device, size: u64) -> Result<Self, SurfaceError> {
        let resource = create_resource(
            device,
            &heap_props(D3D12_HEAP_TYPE_UPLOAD),
            &buffer_desc(size),
            D3D12_RESOURCE_STATE_GENERIC_READ,
        )?;
        // 上传堆没有 barrier；`Map` 直接写。注意 `Map` 的出参形式与 D3D11 完全不同：
        // 它返回 `Result<()>`，指针走出参。
        let mut ptr = std::ptr::null_mut();
        unsafe { resource.Map(0, None, Some(&mut ptr)) }
            .map_err(|e| SurfaceError::new("Map 上传暂存缓冲", e))?;
        Ok(Self {
            resource,
            mapped: ptr.cast::<u8>(),
        })
    }
}

impl StreamBuffer {
    /// 建一块 `size` 字节的流缓冲。
    ///
    /// `draw_state` 是绘制时它必须处于的状态（顶点缓冲给
    /// `VERTEX_AND_CONSTANT_BUFFER`，索引缓冲给 `INDEX_BUFFER`）。
    fn new(
        device: &ID3D12Device,
        size: u64,
        draw_state: D3D12_RESOURCE_STATES,
    ) -> Result<Self, SurfaceError> {
        let resident = create_resource(
            device,
            &heap_props(D3D12_HEAP_TYPE_DEFAULT),
            &buffer_desc(size),
            draw_state,
        )?;
        let staging = UploadBuffer::new(device, size)?;
        Ok(Self {
            resident,
            staging,
            size,
            draw_state,
        })
    }

    /// 记录「把新内容搬进常驻缓冲」所需的屏障、拷贝与状态还原。
    ///
    /// # `bytes` 必须不长于 `size`
    ///
    /// `CopyBufferRegion` 的尺寸由**两个缓冲的较小者**决定，所以给一个超长的切片不会
    /// 报错 —— 它只会**安静地只拷一部分**。所以这里自己夹一次并夹成整块（宁可少拷，
    /// 不要半拷出撕裂的帧）。
    fn record_upload(&self, list: &ID3D12GraphicsCommandList, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let n = (bytes.len() as u64).min(self.size) as usize;
        // CPU 写入常驻映射的上传内存：不需要屏障（上传堆资源恒为 GENERIC_READ，
        // 且 CPU 写映射内存不受资源状态约束）。
        unsafe { self.staging.mapped.copy_from_nonoverlapping(bytes.as_ptr(), n) };

        unsafe {
            gpu_barrier(
                list,
                &self.staging.resource,
                D3D12_RESOURCE_STATE_GENERIC_READ,
                D3D12_RESOURCE_STATE_COPY_SOURCE,
            );
            gpu_barrier(
                list,
                &self.resident,
                self.draw_state,
                D3D12_RESOURCE_STATE_COPY_DEST,
            );
            list.CopyBufferRegion(&self.resident, 0, &self.staging.resource, 0, n as u64);
            gpu_barrier(
                list,
                &self.resident,
                D3D12_RESOURCE_STATE_COPY_DEST,
                self.draw_state,
            );
            gpu_barrier(
                list,
                &self.staging.resource,
                D3D12_RESOURCE_STATE_COPY_SOURCE,
                D3D12_RESOURCE_STATE_GENERIC_READ,
            );
        }
    }

    /// GPU 虚拟地址（绑给 `IASet*Buffer` 时用）。
    fn gpu_addr(&self) -> u64 {
        unsafe { self.resident.GetGPUVirtualAddress() }
    }
}

/// PSO 的适配器节点掩码。**单 GPU 就是 0。**
///
/// # 官方文档原文
///
/// `D3D12_GRAPHICS_PIPELINE_STATE_DESC::NodeMask`：
///
/// > For single GPU operation, **set this to zero**. If there are multiple GPU nodes,
/// > set bits to identify the nodes (the device's physical adapters) for which the
/// > graphics pipeline state is to apply.
///
/// `Multi-adapter systems` 页面进一步说明：
///
/// > If your application passes **0** for this bit mask, then the Direct3D 12 driver
/// > **converts that to the bit mask 1** (indicating that the object is associated with
/// > node 0).
///
/// 而 `D3D12_GRAPHICS_PIPELINE_STATE_DESC` 明确列在那一页的 **Multiple nodes** 清单里
/// —— **不**在 "Single nodes" 清单里。
///
/// # 我曾经把它改成 1，那是错的
///
/// 「单节点 API 必须恰好设一位」那条规则只适用于 `CreateCommandQueue` /
/// `CreateCommandList` / `CreateDescriptorHeap` / `CreateQueryHeap` —— **PSO 不在其列**。
///
/// 当时凭推理认定「0 意味着不属于任何节点」，改了之后
/// `CreateGraphicsPipelineState` 依然失败，**本该据此撤回这个改动却没有**。写在这里
/// 是为了留下「改完没验证好就等于没改」的记录。
///
/// # 顺带说明另两处节点掩码为什么是 1
///
/// 堆属性的 `CreationNodeMask` / `VisibleNodeMask` 不在「驱动会把 0 转成 1」那份
/// 清单里 —— 那个是 `tests/sweep.rs` 扫出来的：写 0 建不出堆。
pub const PSO_NODE_MASK: u32 = 0;


/// `size` 字节的缓冲描述。
///
/// # `Layout` 必须是 `ROW_MAJOR` —— 缓冲没有 `UNKNOWN` 这个选项
///
/// 这里曾经写着 `D3D12_TEXTURE_LAYOUT_UNKNOWN`，并配了一段注释说「提交式资源的布局
/// 必须留 UNKNOWN，写成 ROW_MAJOR 会拿到没有提示的 `E_INVALIDARG`」。
///
/// **那句话对纹理成立，对缓冲正好反了**，于是它一度成了本 crate 最大的阻塞项
/// （**已解**，诊断过程留在下面当记录）：
///
/// - `CreateCommittedResource` 对 4 块适配器**全部**返回 `E_INVALIDARG`；
/// - `CreatePlacedResource`（**另一个函数**）用同一个描述符同样失败 —— 这一条就
///   排除了"函数坏了"，指向字段；
/// - `tests/sweep.rs` 扫 81 种字段组合：凡是 `Layout = ROW_MAJOR` 的**全部通过**
///   （3 种堆类型 × 2 种节点掩码共 6 组），凡是 `UNKNOWN` 的**全部失败**。
///
/// 规则本身是：
///
/// | `Dimension` | `Layout` 必须 |
/// |---|---|
/// | `BUFFER` | `ROW_MAJOR` |
/// | 纹理，**提交式** | `UNKNOWN` |
/// | 纹理，**放置式** | `ROW_MAJOR` |
///
/// 两种写反了都是 `E_INVALIDARG`，且都不提示是哪个字段。
///
/// 官方依据（`D3D12_RESOURCE_DESC` 参考页的 Buffers 一节）：
///
/// > SampleDesc.Count must be 1 and Quality must be 0.
/// > **Layout must be D3D12_TEXTURE_LAYOUT_ROW_MAJOR**, as buffer memory layouts are
/// > understood by applications and row-major texture data is commonly marshaled
/// > through buffers.
///
/// <https://learn.microsoft.com/en-us/windows/win32/api/d3d12/ns-d3d12-d3d12_resource_desc>
pub fn buffer_desc(size: u64) -> D3D12_RESOURCE_DESC {
    D3D12_RESOURCE_DESC {
        Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
        // `Alignment` 对**提交式**资源必须是 0 —— 只有放置式才由调用方指定。
        Alignment: 0,
        Width: size,
        Height: 1,
        DepthOrArraySize: 1,
        MipLevels: 1,
        Format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_UNKNOWN,
        SampleDesc: windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
        // 缓冲的 `Flags` 只能是 `NONE`。`D3D12_RESOURCE_FLAG_BUFFER` 是个**陷阱** ——
        // 它在 DX11 里存在，在 D3D12 里没有，传了会 `E_INVALIDARG`。
        Flags: D3D12_RESOURCE_FLAG_NONE,
    }
}

/// 建图集纹理。
///
/// # 格式必须是 `RGBA8`，不是 `R8`
///
/// [`zephyra::text::GlyphAtlas`] 把覆盖率写进**四个通道**（`atlas.rs` 里
/// `pixels[base..base+4]` 全赋同一个值），而 `pixels()` 返回 `ATLAS_SIZE² × 4` 字节。
///
/// 这里原先写的是 `DXGI_FORMAT_R8_UNORM` —— 那是**两处**错：
///
/// - **格式错**：着色器 `g_tex.Sample(...).a` 从 **alpha** 取覆盖率，而 R8 没有 alpha；
/// - **尺寸错**：上传缓冲按 `side * side` 分配，实际数据是 `side * side * 4` ——
///   `CopyTextureRegion` 要求拷贝区域与缓冲区匹配，多出来那 3 倍会越界。
///
/// 症状是「字完全不出现」，或者崩在上传那一步 —— 而这两者与「GPU 画不出东西」
/// 在现象上无法区分。DX11 侧用 RGBA8，**两边必须一致**。
fn create_atlas(device: &ID3D12Device, size: u32) -> Result<ID3D12Resource, SurfaceError> {
    if size == 0 {
        return Err(SurfaceError::new(
            "图集边长为零",
            windows::core::Error::new(windows::Win32::Foundation::E_FAIL, "atlas size 0"),
        ));
    }
    // 格式必须是 RGBA8，不能是 R8 —— 见上面那段文档。
    create_resource(
        device,
        &heap_props(D3D12_HEAP_TYPE_DEFAULT),
        // `texture_desc` 已经是 `R8G8B8A8_UNORM`，正好对上 `GlyphAtlas` 的四个通道。
        // `FLAG_NONE`：图集只被采样，不当渲染目标。
        &crate::surface::texture_desc((size, size), D3D12_RESOURCE_FLAG_NONE),
        D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE,
    )
}
/// 屏障的本地别名，让 `record` 里的调用短一点。
fn gpu_barrier(
    list: &ID3D12GraphicsCommandList,
    res: &ID3D12Resource,
    before: D3D12_RESOURCE_STATES,
    after: D3D12_RESOURCE_STATES,
) {
    crate::surface::Gpu::barrier(list, res, before, after);
}

/// 把驱动的错误 blob 转成可读消息。
fn blob_message(blob: &Option<windows::Win32::Graphics::Direct3D::ID3DBlob>, fallback: &'static str) -> String {
    let Some(b) = blob else {
        return fallback.to_string();
    };
    // blob 是**不保证**以 NUL 结尾的字节串，取到第一个 0 为止。
    let ptr = unsafe { b.GetBufferPointer() }.cast::<u8>();
    let len = unsafe { b.GetBufferSize() };
    if ptr.is_null() || len == 0 {
        return fallback.to_string();
    }
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
    let end = bytes.iter().position(|&c| c == 0).unwrap_or(len);
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 缓冲的 `Layout` 必须是 `ROW_MAJOR`。
    ///
    /// 这条测试存在的理由：`Layout` 曾被写成 `UNKNOWN`，并配了一段言之凿凿的注释说
    /// 「提交式资源的布局必须留 UNKNOWN」。那句话对**纹理**成立、对**缓冲**正好相反。
    ///
    /// 后果是它一度成了本 crate 最大的阻塞项（**已解**）：`CreateCommittedResource`
    /// 对 4 块适配器**全部**返回 `E_INVALIDARG`，4 块适配器里还包含一块 2026 年的
    /// NVIDIA 独显 —— 那个现象当时被记成了「环境问题」。`tests/sweep.rs` 扫 81 种
    /// 字段组合把它查了出来（凡是 `ROW_MAJOR` 的全过，`UNKNOWN` 的全败）。
    ///
    /// 写不成 `const` 断言，因为 `D3D12_TEXTURE_LAYOUT` 不是结构体常量友好类型。
    #[test]
    fn 缓冲的_布局_必须是行主序而不是未知() {
        let d = buffer_desc(64);
        assert_eq!(
            d.Layout,
            D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
            "D3D12_RESOURCE_DIMENSION_BUFFER 的 Layout 必须是 ROW_MAJOR —— \
             填 UNKNOWN 会让 CreateCommittedResource 回 E_INVALIDARG，\
             且错误消息不说是哪个字段"
        );
        // 顺带把另外两条「写反就静默失效」的规则一起钉住。
        assert_eq!(
            d.Alignment, 0,
            "提交式资源的 Alignment 必须为 0（只有放置式才由调用方指定）"
        );
        assert_eq!(
            d.Flags,
            D3D12_RESOURCE_FLAG_NONE,
            "缓冲的 Flags 只能是 NONE —— D3D12 里没有 D3D11 的 D3D11_RESOURCE_MISC_D3D11_RESOURCE_BINDING \
             那种写法，传了同样回 E_INVALIDARG"
        );
        assert_eq!(d.SampleDesc.Count, 1, "Count 为 0 的 SampleDesc 非法");
    }

    /// 渲染目标纹理必须带 `ALLOW_RENDER_TARGET`。
    ///
    /// 能不能当 RTV 在**创建时**就定死了。这条 bug 的表现形式很怪：
    /// `CreateCommittedResource` 传 `Flags = NONE` + 初始状态
    /// `D3D12_RESOURCE_STATE_RENDER_TARGET` 会回 `E_INVALIDARG`，而错误消息只说
    /// 「参数错误」—— 不提 flag，也不提它跟状态有关。
    ///
    /// 官方依据（`D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET`）：
    ///
    /// > Allows a render target view to be created for the resource; and also **enables
    /// > the resource to transition into the state of
    /// > D3D12_RESOURCE_STATE_RENDER_TARGET**.
    ///
    /// <https://learn.microsoft.com/en-us/windows/win32/api/d3d12/ne-d3d12-d3d12_resource_flags>
    #[test]
    fn 渲染目标纹理必须带_允许渲染目标标志() {
        let rt = crate::surface::texture_desc((64, 64), D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET);
        assert_eq!(
            rt.Flags,
            D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET,
            "离屏渲染目标必须带 ALLOW_RENDER_TARGET"
        );        assert_eq!(
            rt.Layout, D3D12_TEXTURE_LAYOUT_UNKNOWN,
            "提交式**纹理**的 Layout 必须留 UNKNOWN（缓冲才要 ROW_MAJOR）"
        );
    }

    /// PSO 的 `NodeMask` 在单 GPU 下就是 0。
    ///
    /// 这条测试是**反向**钉住文档结论的：官方文档明写
    /// 「For single GPU operation, set this to zero」，且 `multi-engine` 页面说驱动会把
    /// 0 转成掩码 1。
    ///
    /// 它存在是因为本仓库**曾经**把这个值改成 1，理由是「单节点 API 必须恰好设一位」
    /// —— 而那条规则只适用于命令队列/命令列表/描述符堆/查询堆，**PSO 不在其列**。
    /// 改完没有验证是否真的解决了问题（没有），于是这个基于推理的改动就留了下来。
    /// 官方文档把这件事说清楚了，所以改回去，并用这条测试防止再次凭推理改动它。
    #[test]
    fn 管线状态的_节点掩码_单_gpu_下就是零() {
        // 断言的是**生产代码用的那个常量**（`create_pso` 里引用的就是它）。
        assert_eq!(
            PSO_NODE_MASK, 0,
            "文档原文：单 GPU 时 PSO 的 NodeMask 就是 0，驱动会转成掩码 1。\
             「必须设一位」那条规则只适用于单节点 API 清单（命令队列/命令列表/\
             描述符堆/查询堆），PSO 不在其中。"
        );
    }
}

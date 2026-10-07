//! `DisplayList` → D3D11 像素。
//!
//! 这是 base 层的核心职责 —— 核心决定**画什么**，本模块决定**怎么画**。
//! 输入是一份排序后的绘制命令列表，输出是一帧。
//!
//! # 为什么这里没有命令列表、堆与屏障
//!
//! D3D11 用**立即上下文**：`DrawIndexedInstanced` 之后立刻被驱动记录并提交，不需要
//! `ID3D11GraphicsCommandList`、不需要描述符堆、不需要资源屏障、不需要 fence 等
//! GPU。相比 D3D12 少掉的概念比 wgpu-hal 那一层还多。
//!
//! 代价是拿不到"GPU 还没做完"的信号，所以 resize 前无法显式等 GPU 排空
//! （见 [`crate::surface::Surface::resize`] 的说明）。
//!
//! # 每帧的状态设定顺序
//!
//! D3D11 的管线状态是**上下文状态**而非管线对象状态，所以每帧都要重设一遍：
//! 输入布局 → 顶点着色器 → 像素着色器 → 采样器 → SRV → 顶点缓冲 → 混合 → 光栅化。
//! 少设一项不会报错，只会静默沿用上一次的值 —— 那类 bug 极难查，所以这里
//! 集中在 [`Renderer::render`] 里按固定顺序做完，不散落。

#![cfg(windows)]

use windows::core::Interface;
use windows::Win32::Foundation::{E_FAIL, RECT};
// ⚠ `D3D_PRIMITIVE_TOPOLOGY` 及其常量住在 `Direct3D`（D3D9 起共用的那套枚举），
// **不在** `Direct3D11` 下，`IASetPrimitiveTopology` 的签名也收这个类型。所以
// 「从 `Direct3D11::*` 里 import 拓扑常量」必然 `E0432`；测试文件能直接写那个名字，
// 是因为它们 `use ...Direct3D::*` 把两边都引进来了。
use windows::Win32::Graphics::Direct3D::D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_R16_UINT;

use windows::Win32::Graphics::Direct3D11::{
    // 0.62 把 C 的 `D3D11_FILL_MODE_SOLID` 这类名字**去掉了尾部 `_MODE`** ——
    // 枚举类型是 `D3D11_FILL_MODE`，常量却是 `D3D11_FILL_SOLID`。同理
    // `D3D11_CULL_NONE` / `D3D11_COMPARISON_NEVER` / `D3D11_BLEND_ONE`。
    // 写记忆里的名字会得到"no associated item"错误。
    D3D11_BIND_VERTEX_BUFFER, D3D11_BIND_FLAG, D3D11_BLEND, D3D11_BLEND_DESC,
    D3D11_BLEND_DEST_ALPHA, D3D11_BLEND_DEST_COLOR, D3D11_BLEND_INV_DEST_ALPHA, D3D11_BLEND_INV_DEST_COLOR,
    D3D11_BLEND_INV_SRC_ALPHA, D3D11_BLEND_INV_SRC_COLOR, D3D11_BLEND_ONE,
    D3D11_BLEND_SRC_ALPHA, D3D11_BLEND_SRC_COLOR, D3D11_BLEND_ZERO,
    D3D11_BLEND_OP_ADD, D3D11_BUFFER_DESC,
    D3D11_COLOR_WRITE_ENABLE_ALL,
    D3D11_COMPARISON_NEVER, D3D11_CULL_NONE, D3D11_FILL_SOLID,
    D3D11_FILTER_MIN_MAG_MIP_LINEAR,
    D3D11_BIND_INDEX_BUFFER, D3D11_RASTERIZER_DESC, D3D11_RENDER_TARGET_BLEND_DESC,
    D3D11_SAMPLER_DESC,
    D3D11_CPU_ACCESS_WRITE, D3D11_TEXTURE_ADDRESS_CLAMP, D3D11_USAGE_DEFAULT,
    D3D11_USAGE_DYNAMIC, D3D11_VIEWPORT,
    ID3D11BlendState, ID3D11Buffer, ID3D11DepthStencilView, ID3D11Device, ID3D11DeviceContext,

    ID3D11RasterizerState, ID3D11RenderTargetView, ID3D11SamplerState, ID3D11ShaderResourceView,
    ID3D11Texture2D,
};

use zephyra_core::core::Color;
use zephyra_core::filter::{BlendMode, Coeff};
use zephyra_core::frame::Damage;
use zephyra_core::paint::DisplayList;

use crate::quad::{self, RenderReport, Vertex};
use crate::shader::{ShaderError, Shaders};
use crate::texture::TextureTable;

/// 顶点缓冲初始容量（字节）。
///
/// 取 64KB 是量出来的：demo 一帧约 150 个四边形 × 4 顶点 × 52 B ≈ 31KB，
/// 所以 64KB 足够稳态不扩容，而又小到不值得更省。
const INITIAL_VERTEX_CAPACITY: u64 = 64 * 1024;

/// 建一个 D3D11 状态对象，并把"建不出来"统一转成 [`CompileError`]。
///
/// 三个调用点（混合 / 光栅化 / 采样器）都是"建 → map_err 带名字 → 必有值"。
/// 抽成宏而不是泛型函数：三个 `Create*State` 的签名各不相同（描述符类型与出参
/// 类型都不同），泛化只能手写 vtable 调用，那比重复三行更危险。
macro_rules! create_state {
    ($what:literal, $device:expr, $create:ident, $desc:expr) => {{
        let mut out = None;
        unsafe { $device.$create($desc, Some(&mut out as *mut _)) }
            .map_err(|e| ShaderError::Create($what, e))?;
        out.expect("Create*State 成功时必给状态对象")
    }};
}

/// 建一个 `DEFAULT` 缓冲（GPU 可读写，CPU 不可寻址）。
///
/// `bind_flags` 决定它是顶点缓冲还是索引缓冲 —— 两者除这个标志位与字节数以外
/// 完全一样，所以共用一个函数。
fn create_dynamic_buffer(
    device: &ID3D11Device,
    capacity: u64,
    bind_flags: D3D11_BIND_FLAG,
) -> windows::core::Result<ID3D11Buffer> {
    // `ByteWidth` 是 `u32` 而 `capacity` 是 `u64`。截断前必须查：**静默截断**会让
    // 缓冲比声称的小 —— 症状是绘制在某个顶点处开始花屏。
    let byte_width = u32::try_from(capacity)
        .map_err(|_| windows::core::Error::new(E_FAIL, "缓冲超过 D3D11 的 4GB 上限"))?;
    let desc = D3D11_BUFFER_DESC {
        ByteWidth: byte_width,
        // ⚠ 用 `DYNAMIC`，**不是** `DEFAULT`。这不是偏好，是被本机逼出来的：
        //
        // `UpdateSubresource` 在本机对 `DEFAULT` 缓冲会**随机**
        // `STATUS_ACCESS_VIOLATION`。**随机**是这件事最难缠的地方：同一条命令、
        // 同一档（4096 字节）连跑 10 次崩了 3 次，于是任何单次实验都可能得出
        // 相反的结论。我一度扫出「阈值是 8192」，那只是运气。
        //
        // 顶点缓冲是 64 KB，于是每帧都可能撞在这条上崩。而**报不出任何错**：
        // `UpdateSubresource` 返回 `void`，没有 HRESULT 可查 —— 崩溃就是唯一信号。
        //
        // `DYNAMIC` + `Map(WRITE_DISCARD)` 在 2^12..2^21 全过、各跑 8 次零崩溃，
        // 所以走它。证据见 `tests/update_probe.rs`。
        //
        // ⚠ 顺带更正一条已撤销的说法：这段注释原先写「本机 `CreateBuffer` 对
        // `DYNAMIC` 返 `E_INVALIDARG`」—— 那是**我的错**：把 `DYNAMIC` 与
        // `CPUAccessFlags: 0` 配成了非法组合。正确写法在四块适配器上都通过，
        // 见 `tests/probe_basic.rs`。
        //
        // 0.62 的 `Usage` 字段类型是 `D3D11_USAGE` 本身，不是 `u32`。
        Usage: D3D11_USAGE_DYNAMIC,
        BindFlags: bind_flags.0 as u32,
        // `DYNAMIC` 意味着「GPU 只读 + CPU 只写」，写权限**只能**由这个标志授予。
        // 配成 0 是**非法组合**，`CreateBuffer` 会回 `E_INVALIDARG` —— 驱动的原话：
        // "A D3D11_USAGE_DYNAMIC Resource may only have the D3D11_CPU_ACCESS_WRITE
        // CPUAccessFlags set."
        CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
        MiscFlags: 0,
        StructureByteStride: 0,
    };
    let mut buf = None;
    unsafe { device.CreateBuffer(&desc, None, Some(&mut buf as *mut _)) }?;
    Ok(buf.expect("CreateBuffer 成功时必给缓冲"))
}

/// 建一个可动态写的**顶点**缓冲。
///
/// 只是 [`create_dynamic_buffer`] 传了 `D3D11_BIND_VERTEX_BUFFER` 的薄包装 ——
/// 调用点写清楚自己要顶点缓冲，比让它自己推断可读性好。
fn create_vertex_buffer(
    device: &ID3D11Device,
    capacity: u64,
) -> windows::core::Result<ID3D11Buffer> {
    create_dynamic_buffer(device, capacity, D3D11_BIND_VERTEX_BUFFER)
}

/// `Coeff` → D3D11 的**颜色**混合因子。
///
/// **穷尽**匹配：漏一个变体就是编译错误，而手抄因子表的版本漏一条只是某个模式
/// 静默画错。逐项与 Skia `SkBlendMode_AsCoeff` 的 `kCoeffs` 对齐。
fn d3d11_factor(c: Coeff) -> D3D11_BLEND {
    match c {
        Coeff::Zero => D3D11_BLEND_ZERO,
        Coeff::One => D3D11_BLEND_ONE,
        Coeff::SrcColor => D3D11_BLEND_SRC_COLOR,
        Coeff::OneMinusSrcColor => D3D11_BLEND_INV_SRC_COLOR,
        Coeff::DstColor => D3D11_BLEND_DEST_COLOR,
        Coeff::OneMinusDstColor => D3D11_BLEND_INV_DEST_COLOR,
        Coeff::SrcAlpha => D3D11_BLEND_SRC_ALPHA,
        Coeff::OneMinusSrcAlpha => D3D11_BLEND_INV_SRC_ALPHA,
        Coeff::DstAlpha => D3D11_BLEND_DEST_ALPHA,
        Coeff::OneMinusDstAlpha => D3D11_BLEND_INV_DEST_ALPHA,
    }
}

/// `Coeff` → D3D11 的 **alpha 通道**混合因子。
///
/// # ⚠ 不能直接复用 [`d3d11_factor`] —— D3D11 **不允许**在 alpha 通道上用颜色因子
///
/// `CreateBlendState` 对 `SrcBlendAlpha` / `DestBlendAlpha` 只接受
/// `ZERO` / `ONE` / `SRC_ALPHA` / `INV_SRC_ALPHA` / `DEST_ALPHA` / `INV_DEST_ALPHA`。
/// 填 `SRC_COLOR` 会得到 `E_INVALIDARG`（0x80070057）。
///
/// 这个坑是实测撞出来的：`Modulate (0, Cs)` 与 `Screen (1, 1−Cs)` 是 15 个系数
/// 模式里**唯二**用到颜色因子的，于是恰好是它们建不出来。症状很难定位 ——
/// `Renderer::new` 整个失败，报错只说"参数错误"，不会告诉你是哪个字段。
///
/// # 正确的映射：颜色系数换成它的 alpha 对偶
///
/// `Cs → as`、`1−Cs → 1−as`、`Cd → ad`、`1−Cd → 1−ad`，其余原样。
/// 这不是近似，而是 Porter-Duff 代数在预乘空间下的正确结果。逐个核对 15 个模式：
///
/// ```text
///   Clear    (0,0)        → a = 0                  ✓
///   Src      (1,0)        → a = as                 ✓
///   Dst      (0,1)        → a = ad                 ✓
///   SrcOver  (1,1−as)     → a = as+(1−as)·ad        ✓
///   DstOver  (1−ad,1)     → a = (1−ad)·as+ad        ✓
///   SrcIn    (ad,0)       → a = as·ad               ✓（预乘）
///   DstIn    (0,as)       → a = ad·as               ✓
///   SrcOut   (1−ad,0)     → a = as·(1−ad)           ✓
///   DstOut   (0,1−as)     → a = ad·(1−as)           ✓
///   SrcATop  (ad,1−as)    → a = as·ad+(1−as)·ad     ✓
///   DstATop  (1−ad,as)    → a = (1−ad)·as+ad·as     ✓
///   Xor      (1−ad,1−as)  → a = as·(1−ad)+ad·(1−as) ✓
///   Plus     (1,1)        → a = as+ad               ✓
///   Modulate (0,Cs)       → a = ad·as               ✓
///   Screen   (1,1−Cs)     → a = as+ad·(1−as)        ✓
/// ```
fn d3d11_alpha_factor(c: Coeff) -> D3D11_BLEND {
    match c {
        Coeff::Zero => D3D11_BLEND_ZERO,
        Coeff::One => D3D11_BLEND_ONE,
        // 颜色系数 → alpha 对偶。见本函数的文档。
        Coeff::SrcColor => D3D11_BLEND_SRC_ALPHA,
        Coeff::OneMinusSrcColor => D3D11_BLEND_INV_SRC_ALPHA,
        Coeff::DstColor => D3D11_BLEND_DEST_ALPHA,
        Coeff::OneMinusDstColor => D3D11_BLEND_INV_DEST_ALPHA,
        Coeff::SrcAlpha => D3D11_BLEND_SRC_ALPHA,
        Coeff::OneMinusSrcAlpha => D3D11_BLEND_INV_SRC_ALPHA,
        Coeff::DstAlpha => D3D11_BLEND_DEST_ALPHA,
        Coeff::OneMinusDstAlpha => D3D11_BLEND_INV_DEST_ALPHA,
    }
}

/// 按**声明顺序**取第 `i` 个混合模式。
///
/// 下标就是 `BlendMode as usize`，所以这是零成本的身份映射 —— 存在的意义只是
/// 能在不写出 29 个分支的前提下遍历整张表。
///
/// ⚠ 它依赖枚举的声明顺序，而 `LAST_COEFF_MODE` / `LAST_SEPARABLE_MODE` 也是用
/// `<=` 定义的，所以顺序本身就是契约：`zephyra-fx` 里的
/// `unimplemented_blends_counts_only_modes_without_coeffs` 逐项对照 29 个名字，
/// 顺序被改就会失败。
fn mode_by_index(i: usize) -> BlendMode {
    /// 与 `filter.rs` 的 `pub enum BlendMode` **逐项同名同序**。
    const ALL: [BlendMode; BlendMode::COUNT] = [
        BlendMode::Clear,
        BlendMode::Src,
        BlendMode::Dst,
        BlendMode::SrcOver,
        BlendMode::DstOver,
        BlendMode::SrcIn,
        BlendMode::DstIn,
        BlendMode::SrcOut,
        BlendMode::DstOut,
        BlendMode::SrcATop,
        BlendMode::DstATop,
        BlendMode::Xor,
        BlendMode::Plus,
        BlendMode::Modulate,
        BlendMode::Screen,
        BlendMode::Overlay,
        BlendMode::Darken,
        BlendMode::Lighten,
        BlendMode::ColorDodge,
        BlendMode::ColorBurn,
        BlendMode::HardLight,
        BlendMode::SoftLight,
        BlendMode::Difference,
        BlendMode::Exclusion,
        BlendMode::Multiply,
        BlendMode::Hue,
        BlendMode::Saturation,
        BlendMode::Color,
        BlendMode::Luminosity,
    ];
    ALL[i]
}

/// 渲染器：持有 D3D11 管线状态与常驻资源，回放 `DisplayList`。
///
/// 设备与上下文**不归**本结构所有，而是由 [`crate::HostWindow`] 持有并按引用传入
/// —— 它们是整个窗口共享的唯一一份，而渲染器只是它的一个"用户"。若渲染器自己存
/// 一份 `ID3D11Device`，就多出一条与窗口那条并行的引用路径，"谁在用上下文"会
/// 变得无法回答。
pub struct Renderer {
    shaders: Shaders,
    blend: ID3D11BlendState,
    /// 每个混合模式一个 `ID3D11BlendState`，下标即 `BlendMode as usize`。
    ///
    /// `None` = 该模式还不能用系数表达，退回 [`Self::blend`]（`SrcOver`）并计入
    /// [`RenderReport::unimplemented_blends`][^r]。能用系数表达的 15 个
    /// （`Clear`..`Screen`）都有实体。
    ///
    /// # 为什么由 `as_coeff()` **推导**而不是手抄 29 条因子表
    ///
    /// 手抄的那份要靠人肉对齐 Skia 的 `kCoeffs`，抄错一条就是某个混合模式静默
    /// 画错 —— 而且**没有任何测试会失败**（几何、alpha、计数器全都正常）。推导则
    /// 只需要一个 `Coeff → D3D11_BLEND` 的枚举映射，那是有编译器检查的穷尽匹配。
    ///
    /// [^r]: `zephyra_fx::quad::RenderReport`
    blends: Vec<Option<ID3D11BlendState>>,
    /// 目标像素的单采样副本（PS 混合模式要读 dst）。见 [DstCopy]。
    dst_copy: std::cell::RefCell<Option<DstCopy>>,
    rasterizer: ID3D11RasterizerState,
    sampler: ID3D11SamplerState,
    vertex_buffer: ID3D11Buffer,
    /// 索引缓冲（**逐帧重写**，内容由 [`quad::expand_indices`] 生成）。
    ///
    /// # 为什么不是 `IMMUTABLE` 的一次性缓冲
    ///
    /// 索引必须逐四边形加上顶点基址（见 [`quad::expand_indices`]），而四边形数
    /// 每帧可变 —— 所以索引内容每帧都不同，只能动态重写。
    ///
    /// 曾试过 `IMMUTABLE` + `DrawIndexedInstanced(6, quads, 0, 0, 0)`：参数全
    /// 合法，但每个实例都读同一份 `[0,1,2,0,2,3]` + 同一基址，于是 139 个图元
    /// 全画成第一个（且 139 份重叠）。**不报错**，画面上只剩那个全屏背景色。
    ///
    /// 每帧必须 `IASetIndexBuffer` —— 漏掉它 `DrawIndexed` 不报错、只是什么都不画
    /// （另一类实测踩过的静默失败）。
    index_buffer: ID3D11Buffer,
    /// 索引缓冲已分配字节数。
    index_capacity: u64,
    /// 顶点缓冲已分配字节数，用于验证稳态不增长。
    vertex_capacity: u64,
    /// 字形图集纹理（所有权）。
    atlas_texture: ID3D11Texture2D,
    /// 字形图集的采样视图。
    atlas_srv: ID3D11ShaderResourceView,
    /// 图集边长（像素），也是 `bytes_per_row / 4`。
    atlas_size: u32,
}

/// 一次回放的输入。
///
/// 打包成结构体而不是八个位置参数：位置参数多到第 5 个之后就没法靠记忆区分了，
/// 而传错 `size` 与 `clear` 的后果（画面拉伸 / 底色错）都不带任何编译期信号。
pub struct RenderCtx<'a> {
    /// 立即上下文（D3D11 的绘制全部经它）。
    pub context: &'a ID3D11DeviceContext,
    /// 设备（只在需要新建资源时用；顶点缓冲扩容走这条路）。
    pub device: &'a ID3D11Device,
    /// 目标渲染目标视图（通常是 swapchain 当前后缓冲）。
    pub rtv: &'a ID3D11RenderTargetView,
    /// 要回放的命令列表。
    pub list: &'a DisplayList,
    /// 目标像素尺寸。`render()` 据此设视口。
    ///
    /// # ⚠ 这里曾经记着一整条「视口在本机设不进去」的调查，**已撤销**
    ///
    /// 当时的观察是「`RSSetViewports` 之后立刻 `RSGetViewports`，读回恒为个数 0」，
    /// 并据此推断「视口被驱动丢弃、于是三角形零面积」，还顺带怀疑整个图形栈不完整。
    ///
    /// **错因是读回代码，不是驱动**：`RSGetViewports` 的 `pNumViewports` 是
    /// `[in, out]`，**输入值表示 `pViewports` 数组的长度**。当时传 0、数组也是 0 长，
    /// 于是「你要 0 个」就得到 0 个 —— 与驱动状态无关。视口一直正常（1 个、64×64）。
    ///
    /// 而真正让画面只剩两个像素的是**没设拓扑**（见 `render()` 里
    /// `IASetPrimitiveTopology` 处的注释）。当时为了排除它，排查先后转向了顶点属性、
    /// 语义索引、抗锯齿 —— 三样全都正常。真凶只有一行，而它没有任何报错。
    ///
    /// 这条撤销留在这里，是因为它的**教训**比结论本身重要：
    ///
    /// - 「现象真实」不等于「现象的原因真实」。读回得到 `Ok` 也能是错的。
    /// - 观测代码自身出错时，它制造的现象与它要排除的故障**完全同形**。
    ///   「视口失效」与「读回传 0」在像素上的表现一模一样。
    /// - 被拿来做对照组的那个测试，自己也漏了拓扑，于是两边一起错、结论看着自洽
    ///   却毫无信息量。
    ///
    /// 另有一条**独立**故障，别与上面混为一谈：`UpdateSubresource` 在本机对
    /// `DEFAULT` 缓冲会**随机** `STATUS_ACCESS_VIOLATION`（4096 字节连跑 10 次崩
    /// 3 次）。`Map(WRITE_DISCARD)` + `DYNAMIC` 则 2^12..2^21 全过，所以缓冲走后者。
    /// 证据见 `tests/update_probe.rs`。
    ///
    /// 把它单独列出是因为**崩溃与静默丢弃是两种信号**：崩溃的因果关系很容易顺着
    /// 崩点一路读下去，而静默丢弃看起来什么都正常 —— 我就在这里顺手把原因归给了
    /// 环境。
    ///
    /// # 怎么区分「我们错了」与「环境不行」
    ///
    /// 只有一条路：**换一台机器跑**。判据是现成的 ——
    /// `zephyra-winit-d3d12/tests/minimal.rs` 里的
    /// `d3d12_最小资源能不能建出来`。一台正常机器上它应当通过。
    pub size: (u32, u32),
    /// 清屏颜色（线性 RGBA）。
    pub clear: Color,
    /// 图像纹理表。
    pub textures: &'a TextureTable,
    /// 本帧脏区，用于**帧内** scissor。
    ///
    /// # ⚠ 它不能用来「跳过脏区外的回放」
    ///
    /// 本项目两个后端都是 flip model（`DXGI_SWAP_EFFECT_FLIP_DISCARD`），
    /// `Present` **之后**后缓冲内容是**未定义**的。所以任何要 present 的帧都
    /// 必须重画整个目标 —— 顶点流永远整份提交，只有**光栅化范围**能被裁掉。
    ///
    /// 症状若搞反：只画脏区就 present ⇒ 屏幕上大片是未定义内容（通常是黑或
    /// 上一次的帧），而 `RenderReport` 里四边形数完全正常。
    ///
    /// `Damage::Full` 时不设 scissor（D3D11 默认就是全屏，设了要多一次状态变化）。
    pub damage: Damage,
}


/// 目标像素的**单采样**副本。
///
/// # 为什么需要它
///
/// 可分离 / 非可分离混合模式必须读 `dst`，而 D3D11 **不能采样当前绑定的 RTV** ——
/// 着色器读它得到的是未定义值（实测是黑），症状是「混合效果完全没发生」而不报错。
///
/// # 为什么是单采样
///
/// 交换链那条路是 MSAA（present 前 `resolve_msaa`）。MS 纹理**不能**直接
/// `CopyResource` 到非 MS 纹理，只能 `ResolveSubresource` —— 那会对样本求平均，
/// 于是这一段的边缘比别处略软。
///
/// 这是**近似**，不是静默的错：软化只发生在"用了 PS 混合模式的那一段"，
/// 其余段仍走 MSAA 硬件。而且没有任何办法做得更好：要在 MSAA 目标上正确地读
/// 逐样本的 `dst`，就得先 resolve，而 resolve 本身就是求平均。
struct DstCopy {
    tex: ID3D11Texture2D,
    srv: ID3D11ShaderResourceView,
    size: (u32, u32),
}
impl Renderer {
    /// 建管线状态、编译着色器、建图集纹理与顶点缓冲。
    ///
    /// 着色器编译失败**必须**冒泡：它唯一的症状是"窗口一片空白 / 全透明"，
    /// 而那与"没有绘制任何东西"看起来一模一样。
    pub fn new(device: &ID3D11Device) -> Result<Self, ShaderError> {
        let shaders = Shaders::new(device)?;

        // MSAA 采样数。与 [`crate::surface::Surface`] 用**同一个**查询函数 ——
        // 两者是独立构造的（这里拿不到 surface），只有同源查询才能保证 PSO 与
        // 渲染目标一致。设备退化时两边都退回 1。
        let samples = crate::surface::msaa_samples(device);

        // ---- 混合状态 ----
        //
        // **预乘 alpha**：顶点色在 `quad::Vertex::quad` 里已乘过自己的 alpha，
        // 所以 `SrcOver` 的颜色因子是 `ONE`（不能再乘一次 `SRC_ALPHA`，否则
        // 半透明区域会**双重预乘**，表现为整体发暗）。
        //
        // alpha 通道 `ONE/INV_SRC_ALPHA`：`a = as + (1-as)·ad`，这正是
        // swapchain 用 `DXGI_ALPHA_MODE_PREMULTIPLIED` 时该有的形状。
        //
        // 0.62 的 `D3D11_BLEND_DESC` 是**每个渲染目标一个** `RenderTarget[8]` 数组
        // （C 里的 `RenderTarget[1]` 展开），且没有 `LogicOpEnable` 字段 ——
        // `LogicOpEnable` 在每个 RT 的描述里。
        let rt_blend = D3D11_RENDER_TARGET_BLEND_DESC {
            BlendEnable: windows::core::BOOL(1),
            SrcBlend: D3D11_BLEND_ONE,
            DestBlend: D3D11_BLEND_INV_SRC_ALPHA,
            BlendOp: D3D11_BLEND_OP_ADD,
            SrcBlendAlpha: D3D11_BLEND_ONE,
            DestBlendAlpha: D3D11_BLEND_INV_SRC_ALPHA,
            BlendOpAlpha: D3D11_BLEND_OP_ADD,
            // 字段是裸 `u8`，不是 `D3D11_COLOR_WRITE_ENABLE`。
            RenderTargetWriteMask: D3D11_COLOR_WRITE_ENABLE_ALL.0 as u8,
        };
        let blend_desc = D3D11_BLEND_DESC {
            AlphaToCoverageEnable: windows::core::BOOL(0),
            IndependentBlendEnable: windows::core::BOOL(0),
            RenderTarget: [rt_blend; 8],
        };
        let blend = create_state!("BlendState", device, CreateBlendState, &blend_desc);

        // ---- 每个混合模式一个混合状态 ----
        //
        // 由 `BlendMode::as_coeff()` **推导**，不手抄因子表（理由见 `blends` 字段）。
        //
        // ⚠ `SrcOver` 由系数生成的结果与上面手写的那份**逐字段相同**
        // （`(One, OneMinusSrcAlpha)` ⇒ `ONE/INV_SRC_ALPHA`，alpha 通道同理），
        // 所以默认路径的像素一个都没变 —— 这条一致性是本表正确性的旁证。
        let mut blends: Vec<Option<ID3D11BlendState>> = Vec::with_capacity(BlendMode::COUNT);
        for i in 0..BlendMode::COUNT {
            let Some((src, dst)) = mode_by_index(i).as_coeff() else {
                // 不能用系数表达 ⇒ 留空，回放时退回 `SrcOver`。
                blends.push(None);
                continue;
            };
            let desc = D3D11_BLEND_DESC {
                AlphaToCoverageEnable: windows::core::BOOL(0),
                IndependentBlendEnable: windows::core::BOOL(0),
                RenderTarget: [D3D11_RENDER_TARGET_BLEND_DESC {
                    BlendEnable: windows::core::BOOL(1),
                    SrcBlend: d3d11_factor(src),
                    DestBlend: d3d11_factor(dst),
                    BlendOp: D3D11_BLEND_OP_ADD,
                    // alpha 通道用**同一对系数的 alpha 对偶**，不是同一对系数本身
                    // —— D3D11 不允许 alpha 通道用颜色因子，见
                    // [`d3d11_alpha_factor`]。
                    SrcBlendAlpha: d3d11_alpha_factor(src),
                    DestBlendAlpha: d3d11_alpha_factor(dst),
                    BlendOpAlpha: D3D11_BLEND_OP_ADD,
                    RenderTargetWriteMask: D3D11_COLOR_WRITE_ENABLE_ALL.0 as u8,
                }; 8],
            };
            // 15 个系数模式按 D3D11 规范都**必须**建得出来；建不出来就是本函数
            // 映射写错了，那是该炸的时候 —— 静默退化成 `SrcOver` 只会让某个模式
            // 悄悄画错。
            blends.push(Some(create_state!(
                "BlendState",
                device,
                CreateBlendState,
                &desc
            )));
        }

        // ---- 光栅化状态 ----
        //
        // 关掉背面剔除：圆角 SDF 在边缘处产生的三角形可能退化，剔除会让圆角出现
        // 缺口 —— 表现为四边形的角被啃掉一块。
        let raster_desc = D3D11_RASTERIZER_DESC {
            FillMode: D3D11_FILL_SOLID,
            CullMode: D3D11_CULL_NONE,
            FrontCounterClockwise: windows::core::BOOL(0),
            DepthBias: 0,
            DepthBiasClamp: 0.0,
            SlopeScaledDepthBias: 0.0,
            DepthClipEnable: windows::core::BOOL(1),
            // ⚠️ 这里**必须开**才能用帧内 scissor，且**不能**改成「有脏区时才开」——
            // `ScissorEnable` 是在建状态对象时定下的，运行时改不了；要改就得建第二个
            // 光栅化状态对象并在两者间切。
            //
            // 代价：scissor 启用后 `SV_Position` 会被裁到 scissor 矩形内，而
            // MSAA 覆盖率不受 scissor 影响 —— 所以边缘行为与全屏时一致。
            ScissorEnable: windows::core::BOOL(1),
            // ⚠ MSAA。**必须**与实际绑定的渲染目标采样数一致，否则 D3D11 会
            // **解除渲染目标绑定** —— 症状是「一帧什么都不画且零 HRESULT 错误」。
            //
            // 这里的值来自 `samples` 参数（`Renderer::new` 用
            // `surface::msaa_samples(device)` 求得），而绑定目标那边用的是
            // `Surface::msaa_samples` —— **同一个函数**、同一个设备，所以两边
            // 必然一致。这是刻意的：`Renderer::new` 与 `Surface::new` 互不相识，
            // 若各自硬编码常量，设备退化时（MSAA→1）就会一边开一边不开。
            MultisampleEnable: windows::core::BOOL((samples > 1) as i32),
            AntialiasedLineEnable: windows::core::BOOL(0),
        };
        let rasterizer = create_state!("RasterizerState", device, CreateRasterizerState, &raster_desc);

        // ---- 采样器 ----
        //
        // 线性过滤：字形位图在高 DPI 下会被缩小采样，双线性才不会有锯齿。
        // 地址模式 CLAMP：字形图集边缘的字形 uv 可能刚好贴到 1.0，REPEAT 会把相邻
        // 图块的内容绕回来 —— 表现为最右/最下一列字形带上别人的笔画。
        let sampler_desc = D3D11_SAMPLER_DESC {
            // 线性过滤：字形位图在高 DPI 下会被缩小采样，双线性才不会有锯齿。
            Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
            // CLAMP：字形图集边缘的字形 uv 可能刚好贴到 1.0，REPEAT 会把相邻
            // 图块的内容绕回来 —— 表现为最右/最下一列字形带上别人的笔画。
            AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
            AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
            AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
            // 不做深度比较（本渲染器无深度缓冲），但这个字段必须给出**某个**值；
            // 留 0（对应 `D3D11_COMPARISON_NEVER` 的枚举值 1 之外的非法值）
            // 会让 `CreateSamplerState` 失败。
            ComparisonFunc: D3D11_COMPARISON_NEVER,
            BorderColor: [0.0; 4],
            MinLOD: 0.0,
            MaxLOD: f32::MAX,
            MipLODBias: 0.0,
            MaxAnisotropy: 1,
        };
        let sampler = create_state!("SamplerState", device, CreateSamplerState, &sampler_desc);

        // ---- 图集纹理 ----
        //
        // 用 `DEFAULT` + `UpdateSubresource`，而不是 `DYNAMIC` + `Map`。
        //
        // ⚠ 曾经在注释里写「`DYNAMIC` 在本机建不出来」—— **那是错的**：调用方把
        // `DYNAMIC` 与 `CPUAccessFlags: 0` 配成了非法组合（`DYNAMIC` 意味着 CPU 可写，
        // 而写权限只能由 `D3D11_CPU_ACCESS_WRITE` 授予）。本机四块适配器在
        // `DYNAMIC + CPU_ACCESS_WRITE` 下全部建得出来，见 `tests/probe_basic.rs`。
        //
        // 现在仍选 `DEFAULT`，理由是 `UpdateSubresource` 让**目标行距由驱动处理**，
        // 而 `Map` 要求自己按驱动的 `RowPitch` 跳行 —— 忘了就是「内容斜着进纹理」
        // 这一整类难查的 bug。详见 `texture::write_rows` 的对照表。
        let atlas_size = zephyra_core::text::atlas::ATLAS_SIZE;
        let atlas_texture =
            crate::texture::create_rgba_texture_empty(device, atlas_size, atlas_size, D3D11_USAGE_DEFAULT)
                .map_err(|e| ShaderError::Create("建字形图集", e))?;
        let atlas_srv = crate::texture::create_srv(device, &atlas_texture)
            .map_err(|e| ShaderError::Create("建图集采样视图", e))?;

        // ---- 顶点缓冲 ----
        let vertex_buffer = create_vertex_buffer(device, INITIAL_VERTEX_CAPACITY)
            .map_err(|e| ShaderError::Create("建顶点缓冲", e))?;

        // ---- 索引缓冲 ----
        //
        // `DYNAMIC` + `CPU_ACCESS_WRITE`：索引内容逐帧不同（每个四边形要加自己的
        // 顶点基址），所以必须每帧重写。理由见字段文档。
        //
        // 初始容量按 256 个四边形给 —— demo 约 139，留一点余量，稳态不扩容。
        let index_capacity = 256u64 * 6 * 2;
        let index_buffer = create_dynamic_buffer(
            device,
            index_capacity,
            D3D11_BIND_INDEX_BUFFER,
        )
        .map_err(|e| ShaderError::Create("建索引缓冲", e))?;

        Ok(Self {
            shaders,
            blends,
            dst_copy: std::cell::RefCell::new(None),
            blend,
            rasterizer,
            sampler,
            vertex_buffer,
            index_buffer,
            index_capacity,
            vertex_capacity: INITIAL_VERTEX_CAPACITY,
            atlas_texture,
            atlas_srv,
            atlas_size,
        })
    }

    /// 图集边长（像素）。
    pub fn atlas_size(&self) -> u32 {
        self.atlas_size
    }

    /// 顶点缓冲容量（字节）。稳态下跨帧不变。
    pub fn vertex_capacity(&self) -> u64 {
        self.vertex_capacity
    }

    /// 把字形图集的位图上传到 GPU 纹理。
    ///
    /// 图集本体是 core 的 [`zephyra_core::text::GlyphAtlas`]（纯 CPU 数据）；本方法只做
    /// 那一次"CPU 像素 → GPU 纹理"的转换。
    ///
    /// 用 `UpdateSubresource` 而不是 `Map(WRITE_DISCARD)`。
    ///
    /// ⚠ 原文写「`Map` 需要纹理是 `DYNAMIC` 建的，而本机 `CreateTexture2D` 对
    /// `Usage = DYNAMIC` 直接返 `E_INVALIDARG`」—— **前半句对，后半句错**：本机
    /// 建不出的是**非法组合**（`DYNAMIC` + `CPUAccessFlags: 0`），正确写法在四块
    /// 适配器上都通过。见 `tests/probe_basic.rs` 与 `texture::create_rgba_texture_empty`。
    ///
    /// 选 `UpdateSubresource` 的真实理由：目标行距由驱动处理，不必自己按 `RowPitch`
    /// 跳行。代价是驱动内部可能再拷一份 4MB。
    pub fn upload_atlas(
        &self,
        context: &ID3D11DeviceContext,
        atlas: &zephyra_core::text::GlyphAtlas,
    ) -> windows::core::Result<()> {
        self.upload_atlas_pixels(context, atlas.pixels())
    }

    /// 上传图集位图（RGBA8，`ATLAS_SIZE × ATLAS_SIZE`）。
    ///
    /// 整张上传而不是只传变化区域 —— 图集会被**整体重排**（LRU 淘汰之后按高度重排），
    /// 那时几乎每个字形的 uv 都变了，增量上传无从下手。代价是每次变化多一次 4MB
    /// 上传；稳态下图集不再变化，这个上传也就不再发生（宿主用
    /// [`zephyra_core::text::TextStack::atlas_revision`] 判断，**不能**用字形数）。
    pub fn upload_atlas_pixels(
        &self,
        context: &ID3D11DeviceContext,
        pixels: &[u8],
    ) -> windows::core::Result<()> {
        let width_bytes = (self.atlas_size as usize) * 4;
        let need = width_bytes * (self.atlas_size as usize);
        if pixels.len() < need {
            return Err(windows::core::Error::new(
                windows::Win32::Foundation::E_FAIL,
                format!("图集像素不足：需要 {need} 字节，实得 {}", pixels.len()),
            ));
        }
        // 源是**紧密排列**的（`SysMemPitch = width * 4`），目标那侧的填充由驱动处理。
        //
        // 原来这里是 `Map(WRITE_DISCARD)` + 按 `RowPitch` 跳行 —— 那样写必须自己
        // 处理目标行距，漏了会让图集内容斜着进纹理（症状是字形整体倾斜、倾斜量随行号
        // 累积）。改用 `UpdateSubresource` 后驱动接管这件事，这类整类 bug 直接消失。
        let data = windows::Win32::Graphics::Direct3D11::D3D11_SUBRESOURCE_DATA {
            pSysMem: pixels.as_ptr() as *const _,
            SysMemPitch: width_bytes as u32,
            SysMemSlicePitch: 0,
        };
        unsafe {
            context.UpdateSubresource(
                &self.atlas_texture,
                0,
                None, // 整个资源
                &data as *const _ as *const std::ffi::c_void,
                0,
                0,
            )
        };
        Ok(())
    }

    /// 回放一份 `DisplayList`。
    ///
    /// `ctx.size` 是目标的**像素**尺寸 —— 顶点在此从 DIP 转到 NDC。
    ///
    /// 未注册的纹理命令会被跳过并计入 [`RenderReport::missing_textures`] —— 不静默：
    /// 那是宿主的 bug（注册名与属性值拼错），症状是"图标不显示"，没有其他线索。
    pub fn render(&mut self, ctx: RenderCtx<'_>) -> RenderReport {
        let RenderCtx {
            context,
            device,
            rtv,
            list,
            size,
            clear,
            textures,
            damage,
        } = ctx;


        // ---- 1. 展开成顶点流 ----
        let batch = quad::expand(list, size, |name| textures.contains(name));
        let (all, _image_first, path_first) = batch.flatten();
        let runs = batch.runs;
        let mut report = batch.report;
        report.vertex_bytes = (all.len() * std::mem::size_of::<Vertex>()) as u64;

        // ---- 2. 顶点缓冲按需增长，但不缩 ----
        // 稳态零分配的前提。
        let needed = report.vertex_bytes.max(64);
        if needed > self.vertex_capacity {
            let grown = needed.next_power_of_two();
            if let Ok(b) = create_vertex_buffer(device, grown) {
                self.vertex_buffer = b;
                self.vertex_capacity = grown;
                report.reallocated = true;
            }
            // 建失败就退回旧缓冲：本帧可能画不全，但不该整帧崩掉。
            // 旧缓冲装不下时会截断绘制（D3D11 静默丢弃越界的 draw）——
            // 症状是"多出来的图元不见了"，比崩溃容易查。
        }

        // ---- 2b. 索引缓冲 ----
        //
        // 必须在 draw 之前写好：索引要逐四边形加顶点基址（见 `quad::expand_indices`），
        // 而四边形数逐帧可变。
        //
        // 索引流分两段：前段是四边形（`solid` + `images`，每图元 6 个索引），
        // 后段是多边形三角形（每三角形 3 个连续索引）。路径段的顶点基址是
        // `quads * 4`，也就是 `quad::expand_indices_with_paths` 算的那个值。
        //
        // ⚠ 四边形数必须**只数 solid + images**。写成 `all.len() / 4` 会把路径
        // 的顶点也除进去 —— 路径顶点数是 3 的倍数不是 4 的倍数，于是这个除法
        // 本身就没有意义（整数截断），结果是四边形段的顶点基址全部错位。
        // 症状：路径一旦出现，它前面所有图元一起变成乱码几何。
        let total_quads = (batch.solid.len() + batch.images.len()) / 4;
        let path_tris = batch.paths.len() / 3;
        let (indices, path_index_start) = quad::expand_indices_with_paths(total_quads, path_tris);
        if !indices.is_empty() {
            let need = (indices.len() * 2) as u64;
            if need > self.index_capacity {
                let grown = need.next_power_of_two();
                if let Ok(b) =
                    create_dynamic_buffer(device, grown, D3D11_BIND_INDEX_BUFFER)
                {
                    self.index_buffer = b;
                    self.index_capacity = grown;
                    report.reallocated = true;
                }
            }
            self.write_buffer(context, &self.index_buffer, bytemuck::cast_slice(&indices), self.index_capacity);
        }

        // ---- 3. 写顶点 ----
        //
        // 顺序：**先写数据，再绑状态，最后 draw**。
        //
        // ⚠ 这里曾经把「写顶点」放在设状态**之后**，并写下一条注释说「必须先写、
        // 否则会 `STATUS_ACCESS_VIOLATION`」。那条注释是**从相关性里读出来的因果**，
        // 实测被推翻：把这一段挪到绑定之前，崩溃照旧。真正的原因是
        // `UpdateSubresource` 在本机对 ≥ 8192 字节的缓冲必崩，与是否绑定无关。
        //
        // 现在的顺序仍然是正确的 —— 只是理由不同：绑定之后再写同一个缓冲，
        // 驱动要等 GPU 用完旧内容才能真正替换（`WRITE_DISCARD` 让它另给一块，
        // 所以这里没有硬等待）。「先写后绑」少一次这样的往返。
        if !all.is_empty() {
            self.write_vertices(context, bytemuck::cast_slice(&all));
        }

        // ---- 4. 设定每帧上下文状态 ----
        //
        // 顺序固定：渲染目标 → 视口 → 清屏 → 混合/光栅化 → 着色器 → 输入布局 →
        // 顶点缓冲 → 采样器。少设一项不会报错，只会静默沿用上一次的值。
        unsafe {
            // `OMSetRenderTargets` 的失败症状极具误导性，因为它与 `Clear` 的
            // 行为**不对称**：
            //
            // - `ClearRenderTargetView(rtv, ...)` 把 RTV 当**参数**收，完全不看
            //   绑定，所以绑定没成功时清屏照样出现在画面上；
            // - `Draw*` 依赖绑定，绑不上就一个像素都不画，且**不报错**。
            //
            // 而 windows-rs 0.62 把 `OMSetRenderTargets` 的返回值声明成 `()`
            // （vtable 槽也是 `-> ()`），HRESULT 拿不到 —— 我试过把它 transmute
            // 成返回 `HRESULT` 再调，读回来的却是每次都不同的垃圾值，说明该槽
            // 实际上不返回 HRESULT。所以这条路只能作废，改看设备状态。
            context.OMSetRenderTargets(
                Some(&[Some(rtv.clone())]),
                None::<&ID3D11DepthStencilView>,
            );
            // ⚠ 这一句**生效**，曾经那句「本机不生效」的结论是**读回代码的错**：
            // `RSGetViewports` 的 `pNumViewports` 是 `[in, out]`，**输入值表示
            // `pViewports` 数组的长度**，而当时传的是 0、数组也确实是 0 长 ——
            // 于是「你要 0 个」就得到 0 个，与驱动状态无关。视口读回一直是 1 个、
            // 64×64。D3D11 的默认视口确实是 0×0，所以真出问题时症状仍是无声画不出，
            // 但那是另一回事，见 [`RenderCtx::size`]。
            let viewport = [D3D11_VIEWPORT {
                TopLeftX: 0.0,
                TopLeftY: 0.0,
                Width: size.0 as f32,
                Height: size.1 as f32,
                MinDepth: 0.0,
                MaxDepth: 1.0,
            }];
            context.RSSetViewports(Some(&viewport));

            // ---- 帧内 scissor ----
            //
            // ⚠️⚠️ **D3D11 的 `ClearRenderTargetView` 不受 scissor 约束** ——
            // 这是 D3D11 与 D3D12 的一处真实差异（D3D12 命令列表上的 clear **会**被
            // scissor 裁掉）。所以本后端里清屏永远是全屏的，scissor 只限制**绘制**。
            //
            // 实测踩过：先按「clear 也被裁剪」的假设写了断言（脏区外应保持上一帧内容），
            // 结果脏区外 2048/2048 全变 —— 因为整张纹理被清成了清屏色。
            //
            // 这不影响正确性（flip model 下本来就得重画整帧），但它**限定了收益**：
            // D3D11 侧脏区省的是**片元着色**的填充率，不是 clear 的。要连 clear 一起省
            // 就得改成 D3D12 那样的命令列表语义，或干脆别清（DComp 路径上靠保留缓冲）。
            //
            // DIP → 像素的比例**不能假设为 1**。`RenderCtx::damage` 是 DIP，而
            // scissor 用像素；当前宿主恰好把像素尺寸当 DIP 传进来，于是比例是 1，
            // 但那是巧合 —— 高 DPI 下同一个 `DisplayList` 会被放大，而这里若按 1 算，
            // 脏区就只有实际需要的 1/k，症状是「脏区外的图元也还在算」。
            let scissor = match damage {
                // ⚠️ `Full` 也**必须显式设一个矩形**，不能靠 `RSSetScissorRects(None)`
                // 来「关掉裁剪」。
                //
                // 实测：本仓的 `d3d11_draw` 测试在改成 `None` 之后覆盖从 4096/4096
                // 掉到 **1/4096** —— 而 D3D11 不画任何东西时不报错，
                // `GetDeviceRemovedReason` 仍是 `Ok(())`，`RenderReport` 里 `quads: 2`
                // 也一切正常。
                //
                // 也就是说 `ScissorEnable = TRUE` 且**没有**有效矩形时，光栅化结果与
                // 「拓扑没设」完全同形（各剩 1 个退化像素）。这类静默失败只能靠像素
                // 断言挡住，所以那条测试不许为了「顺便验 scissor」而改判据。
                Damage::Full => RECT {
                    left: 0,
                    top: 0,
                    right: size.0 as i32,
                    bottom: size.1 as i32,
                },
                Damage::Rect(r) => {
                    let sx = size.0 as f32 / viewport[0].Width.max(1e-6);
                    let sy = size.1 as f32 / viewport[0].Height.max(1e-6);
                    // ⚠️ 退化矩形必须回退到「全屏」而不是照设：D3D11 的 scissor 是
                    // 半开区间 [left, right)，right <= left 时**什么都不画** ——
                    // 症状是元素凭空消失，且零错误。
                    let l = (r.x * sx).floor().max(0.0) as i32;
                    let t = (r.y * sy).floor().max(0.0) as i32;
                    let rt = ((r.x + r.width) * sx).ceil() as i32;
                    let b = ((r.y + r.height) * sy).ceil() as i32;
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
            };
            context.RSSetScissorRects(Some(&[scissor]));

            // 清屏色。`Color` 存的是**线性** 0..1，而 `ClearRenderTargetView` 要的
            // 也是线性 —— 这里不做任何编码转换。
            //
            // ⚠ 但必须**预乘**：目标纹理从此与顶点色在同一个预乘空间里。漏掉这一步
            // 的话，一个半透明的 clear 色会让整帧偏亮，且只有非 1.0 的 clear 色
            // 才暴露 —— 而默认 clear 是全 1.0 或全 0.0，于是永远看不出来。
            //
            // （诊断时把读回的像素按 sRGB 编码再显示，见 [`crate::write_bmp`]。那
            // 是**看图工具的约定**问题，与这里无关。）
            let ca = clear.a;
            context.ClearRenderTargetView(
                rtv,
                &[clear.r * ca, clear.g * ca, clear.b * ca, ca],
            );
            context.OMSetBlendState(&self.blend, None, 0xFFFF_FFFF);
            context.RSSetState(&self.rasterizer);
            // ⚠ `IASetInputLayout` **必须**在这里绑。
            //
            // D3D11 没有 PSO 对象 —— 输入布局是上下文状态的一部分。漏绑时
            // `CreateInputLayout` 与 `VSSetShader` 都不会报错，而顶点着色器读不到
            // 任何顶点属性，于是三角形退化。
            //
            // ⚠⚠ 这里曾经写着两条**臆测**，都把排查带偏了整整两轮，留在下面当反例：
            //
            // 1. 「漏绑的症状是屏幕上恰好 2 个半透明像素」。**没有验证过**。真正被
            //    实测钉死的是另一回事 —— 那个「2 个像素」的元凶是**拓扑未设**
            //    （见下方 `IASetPrimitiveTopology`）。
            // 2. 「`CreateInputLayout` 不校验与着色器签名的匹配」。官方原文相反 ——
            //    「this allows the API to validate the input-layout-object elements
            //    against the shader-input signature to make sure that there is an
            //    exact match of types and semantics」。布局若对不上，`Shaders::new`
            //    会直接失败；它既然成功了，布局就是对的。
            //
            // 把没验证过的结论写成排除依据，是纯粹的负资产。
            context.IASetInputLayout(&self.shaders.input_layout);
            context.VSSetShader(&self.shaders.vs, None);
            context.PSSetShader(&self.shaders.ps[0], None);
            context.PSSetSamplers(0, Some(&[Some(self.sampler.clone())]));
            let stride = std::mem::size_of::<Vertex>() as u32;
            let offset = 0u32;
            context.IASetVertexBuffers(
                0,
                1,
                Some([Some(self.vertex_buffer.clone())].as_ptr()),
                Some(&stride),
                Some(&offset),
            );
            // 索引缓冲**必须**在 draw 之前绑定 —— 漏掉它时 D3D11 不报错，只是整帧
            // 什么都不画（实测踩过：窗口只剩清屏色，而 `RenderReport` 里
            // `quads=143` 一切正常）。那是最难查的一类故障，因为没有错误可查。
            context.IASetIndexBuffer(&self.index_buffer, DXGI_FORMAT_R16_UINT, 0);
            // ⚠⚠ 拓扑**必须**设，否则整批几何不进光栅化 —— 这就是「两个四边形
            // 只剩两个像素」的真凶，找了整整两轮。
            //
            // 上下文创建后拓扑是 `D3D11_PRIMITIVE_TOPOLOGY_UNDEFINED`（0），而
            // `DrawIndexed` 拿它当参数时**不报错**：没有 HRESULT、没有异常、
            // `GetDeviceRemovedReason` 仍是 `Ok(())`，`RenderReport` 里
            // `quads=2` 也一切正常 —— 唯一可观察的后果就是画面上几乎什么都没有。
            //
            // 官方《Getting Started with the Input-Assembler Stage》把这一步写成
            // 硬性要求：
            //   "After the input buffers have been bound, the IA stage **must** be
            //    told how to assemble the vertices into primitives. This is done by
            //    specifying the primitive type by calling
            //    ID3D11DeviceContext::IASetPrimitiveTopology".
            //
            // 残留的「两个像素」是退化几何的残迹（顶点全在同一处时光栅化仍会命中
            // 极少片元），所以症状**看起来**像「画了一点点」，把排查带到了顶点属性、
            // 语义索引、抗锯齿上 —— 而那三样全都是好的。判据见
            // `tests/d3d11_draw.rs`：同一份着色器、同一份顶点数据，
            // 唯一多了 `IASetPrimitiveTopology(TRIANGLELIST)`，覆盖从 2/4096 变成
            // 4096/4096。
            context.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
        }

        // `DrawIndexed` 而**不是** `DrawIndexedInstanced`：索引里已经逐四边形加好了
// ---- 4. 逐段回放：一段一次 draw ----
        //
        // 为什么按 [`quad::DrawRun`] 走而不是把 solid / images / paths 三段写死：
        // 混合模式是**渲染管线状态**，一次 draw 只能有一种。两段相邻的图元若混合
        // 模式不同，就必须分成两次 draw —— 而三段写死的写法表达不了这件事，
        // 只能给整帧套一个状态（症状是"有一个的效果没生效"，而所有计数都正常）。
        //
        // `runs` 的不变量是「恰好划分整条顶点流」，所以循环覆盖全部几何，既不漏
        // 也不重叠。见 `quad::DrawRun` 的文档。
        for run in &runs {
            if run.vertex_count == 0 {
                continue;
            }
            // ⚠ 混合模式**逐段**决定两样东西：PS 变体与混合状态。这两样必须**成对**决定 ——
            // 只换其一的后果是"模式看起来没生效"，而且没有任何错误。
            //
            // | 模式            | PS 变体        | 混合状态        |
            // |-----------------|----------------|-----------------|
            // | 系数可表达 15 个 | `PSMain`        | 该模式的系数对     |
            // | 可分离/非可分离  | 各自的合成变体  | **`Src`**（ONE/ZERO）|
            //
            // 第二行为什么是 `Src`：那些 PS 已经把 `dst` 算进去、写出了**最终**颜色，
            // 固定功能方程只能"原样接受"。用 `SrcOver` 会再叠一次 ⇒ 整体过曝。
            let mode = run.blend as usize;
            let ps_variant = crate::shader::ps_for_blend_mode(mode);
            let (ps_idx, blend, dst_srv) = match ps_variant {
                // 系数可表达：PS 只算源色，混合交给固定功能方程。
                // `None` = 后面不必再绑 `g_dst`（`PSMain` 不读它）。
                None => (
                    0usize,
                    self.blends[mode].clone().unwrap_or_else(|| self.blend.clone()),
                    None,
                ),
                // PS 合成模式：需要 `dst` 的副本，绑到 **t1**（`g_dst` 的槽位）。
                Some(k) => match self.refresh_dst_copy(device, context, rtv, size) {
                    Some(srv) => {
                        // `blends[1]` 是 `BlendMode::Src` = `(One, Zero)`，正好是
                        // "原样接受 PS 的输出"。
                        (
                            k,
                            self.blends[1].clone().unwrap_or_else(|| self.blend.clone()),
                            Some(srv),
                        )
                    }
                    // 建不出目标副本 ⇒ 退回 `SrcOver` 而不是整帧崩。
                    None => (0usize, self.blend.clone(), None),
                },
            };
            // 绑 `g_dst`。
            //
            // ⚠ 必须绑在 **每段** 之前而不是帧首一次：可分离 / 非可分离模式要读的
            // `dst` 已经含**前面各段**的结果，只在帧首拷一次的话第二段起全都基于
            // 帧首的目标算。`per_run_ps_switching_uses_the_updated_destination`
            // 钉住这条。
            //
            // `None` 时也要显式解绑 t1：上一段用 PS 合成模式、这一段不用时，
            // 残留的 SRV 会让绑定点错位（本段读 t0 就变成读上一段的目标）。
            unsafe {
                // `Some(&[…])` 里放**拥有所有权**的视图（D3D11 会 AddRef），
                // 不是引用 —— 传引用会拿到 `Option<&ID3D11ShaderResourceView>`。
                let t1: Option<ID3D11ShaderResourceView> = dst_srv.clone();
                context.PSSetShaderResources(1, Some(&[t1]));
            }
            unsafe {
                context.PSSetShader(&self.shaders.ps[ps_idx], None);
            }
            // 纹理绑定。只有 `Command::Image` 逐段换绑定；矩形 / 字形 / 多边形记
            // `None`，绑图集兜住（片元着色器仍然声明了 `g_tex`，缺 SRV 时某些驱动
            // 会解除引用并崩溃 —— 这是本文件另一处实测踩过的静默失败）。
            let srv = match &run.texture {
                Some(name) => match textures.srv(name) {
                    Some(s) => s,
                    // 顶点收集阶段已过滤掉未注册纹理，所以这里本不该发生。
                    None => continue,
                },
                None => self.atlas_srv.clone(),
            };
            // 索引区间。
            //
            // ⚠ 两种单位在这里交汇：`runs` 记的是**顶点**下标，而 `DrawIndexed` 要的是
            // **索引**下标。四边形 4 顶点 ↔ 6 索引，所以要 `× 6 / ÷ 4`；多边形则是
            // 1:1，且它的索引区在 `path_index_start` 之后。混用这两个单位时
            // `StartIndexLocation` 仍是个合法值 —— 只是不报错地画错位置。
            let (index_start, index_count) = if run.first_vertex >= path_first {
                (
                    path_index_start + (run.first_vertex - path_first),
                    run.vertex_count,
                )
            } else {
                (
                    (run.first_vertex / 4) * 6,
                    (run.vertex_count / 4) * 6,
                )
            };
            unsafe {
                context.OMSetBlendState(&blend, None, 0xFFFF_FFFF);
                context.PSSetShaderResources(0, Some(&[Some(srv)]));
                // `DrawIndexed` 而**不是** `DrawIndexedInstanced`：索引里已经逐四边形
                // 加好了顶点基址（见 `quad::expand_indices`）。
                context.DrawIndexed(index_count, index_start, 0);
            }
        }
        report
    }



    /// 把当前 RTT 的内容取到一张可采样纹理，返回它的 SRV。
    ///
    /// 复用同一张纹理（按尺寸），所以稳态零分配。尺寸变了就重建 —— 那说明窗口
    /// 改了大小，本就该重新分配。
    ///
    /// 建不出来（`None`）时调用方退回 `SrcOver`：宁可混合模式不生效，也不要整帧崩。
    fn refresh_dst_copy(
        &self,
        device: &ID3D11Device,
        context: &ID3D11DeviceContext,
        rtv: &ID3D11RenderTargetView,
        size: (u32, u32),
    ) -> Option<ID3D11ShaderResourceView> {
        use windows::Win32::Graphics::Direct3D11::{
            D3D11_BIND_SHADER_RESOURCE, D3D11_RESOURCE_MISC_SHARED, D3D11_TEXTURE2D_DESC,
            D3D11_USAGE_DEFAULT,
        };
        use windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC;

        if size.0 == 0 || size.1 == 0 {
            return None;
        }
        // 目标的格式与采样数从**纹理**读，而不是从 RTV —— `ID3D11RenderTargetView`
        // 在 windows 0.62 上没有 `GetDesc`。
        //
        // 格式必须一致，否则 `CopyResource` 拒绝（源与目标格式不同），而 SRV 的
        // 格式还必须与纹理一致，否则采样结果整体错位。
        let src = unsafe { rtv.GetResource() }.ok()?;
        let src_tex: ID3D11Texture2D = src.cast().ok()?;
        // ⚠ `ID3D11Texture2D::GetDesc` 在 windows 0.62 上是**出参**形式
        // （`GetDesc(&mut desc)`），不是返回 `D3D11_TEXTURE2D_DESC`。
        // 按后者写会编译成"调用了一个需要 1 个参数的方法"——错误信息完全不指向它。
        let mut tex_desc = D3D11_TEXTURE2D_DESC::default();
        unsafe { src_tex.GetDesc(&mut tex_desc) };

        let mut slot = self.dst_copy.borrow_mut();
        if slot.as_ref().is_none_or(|d| d.size != size) {
            let desc = D3D11_TEXTURE2D_DESC {
                Width: size.0,
                Height: size.1,
                MipLevels: 1,
                ArraySize: 1,
                Format: tex_desc.Format,
                // **必须**单采样：MS 纹理不能当 `CopyDestination`。
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
                CPUAccessFlags: 0,
                MiscFlags: D3D11_RESOURCE_MISC_SHARED.0 as u32,
            };
            let mut tex = None;
            unsafe { device.CreateTexture2D(&desc, None, Some(&mut tex as *mut _)) }.ok()?;
            let tex = tex?;
            // 格式必须跟着纹理走 —— 见 `create_srv_with_format` 的文档：拿
            // `R8G8B8A8` 去描述 `B8G8R8A8` 的纹理会让 `CreateShaderResourceView`
            // 直接失败，而这个失败一路被 `.ok()` 吞掉，最终表现为"效果没生效"。
            let srv = crate::texture::create_srv_with_format(device, &tex, tex_desc.Format).ok()?;
            *slot = Some(DstCopy { tex, srv, size });
        }
        let d = slot.as_ref()?;

        // 单采样目标 ⇒ 直接拷贝。
        //
        // MSAA 目标 ⇒ 必须 `ResolveSubresource`：`CopyResource` 在 MS→非 MS 上是
        // **非法**的（D3D11 给 `E_INVALIDARG`；更糟的是某些驱动只警告就继续，于是
        // 画面停在上一帧的内容上 —— 症状是"混合效果看起来生效了但慢一拍"）。
        unsafe {
            if tex_desc.SampleDesc.Count > 1 {
                context.ResolveSubresource(&d.tex, 0, &src_tex, 0, tex_desc.Format);
            } else {
                context.CopyResource(&d.tex, &src_tex);
            }
        }
        Some(d.srv.clone())
    }

    /// 把顶点写进缓冲并解除映射。
    fn write_vertices(&self, context: &ID3D11DeviceContext, bytes: &[u8]) {
        self.write_buffer(context, &self.vertex_buffer, bytes, self.vertex_capacity);
    }

    /// 往缓冲里写入数据（顶点或索引）。
    ///
    /// # 为什么不写成泛型
    ///
    /// 两个调用点的差别只有"写哪个缓冲"，其余完全相同。泛型化要么把 `ID3D11Buffer`
    /// 塞进类型参数、要么加生命周期标注 —— 那比重复这几行更绕。
    ///
    /// # 用 `Map(WRITE_DISCARD)` 而不是 `UpdateSubresource`
    ///
    /// 这不是「哪个更快」的选择 —— 在本机上**只有一条不崩**。
    ///
    /// `UpdateSubresource` 对 `DEFAULT` 缓冲会**随机** `STATUS_ACCESS_VIOLATION`：
    /// 同一档（4096 字节）连跑 10 次崩 3 次。顶点缓冲 64 KB，每帧都可能中招。
    /// 而 `UpdateSubresource` 返回 `void`，**没有任何错误码**能提前发现 ——
    /// 崩溃就是唯一信号。
    ///
    /// 随机性还意味着：**不能用「跑通了」当通过判据**。我一度扫出「阈值是 8192、
    /// 4096 正常」，那只是运气。证据见 `tests/update_probe.rs`。
    ///
    /// 文档也**明确禁止**把 `DYNAMIC` 资源当 `UpdateSubresource` 的目标：
    /// 「A resource cannot be used as a destination if: the resource is created with
    /// immutable or **dynamic** usage.」（本机对 `DYNAMIC` 不崩，但那是未定义行为，
    /// 不能依赖。）
    ///
    /// <https://learn.microsoft.com/en-us/windows/win32/api/d3d11/nf-d3d11-id3d11devicecontext-updatesubresource>
    ///
    /// # `RowPitch` 对**缓冲**不是问题
    ///
    /// 「忘了按 `RowPitch` 跳行、内容斜着串行」那**一整类** bug 只发生在**纹理**上：
    /// 纹理有多行，行与行之间有填充。**缓冲只有一行**，`RowPitch` 恒等于 `ByteWidth`，
    /// 源与目标都是紧密排列的 —— 一次连续的 `copy_nonoverlapping` 就对了。
    ///
    /// 纹理那条路仍然走 `UpdateSubresource`（纹理不能用 `DYNAMIC` 写），
    /// 见 [`crate::texture::write_rows`]。
    ///
    /// # 容量由调用方保证
    ///
    /// `vertex_capacity` / `index_capacity` 都在扩容之后才写，所以这里不必再夹长度。
    /// 夹了反而把「写少了」变成静默成功 —— 那比崩更难查。
    ///
    /// # `WRITE_DISCARD` 的含义
    ///
    /// 让驱动直接给出**新**的存储，旧的（可能还在被 GPU 读）另作安排。这正是
    /// 「每帧重写」想要的语义，也是不必自己同步/双缓冲的原因。
    ///
    /// # ⚠ 先前这里走过一条错路，结论也一起记在这里
    ///
    /// 原先的注释写「`Map` 需要缓冲是 `DYNAMIC` 建的，而**本机 `CreateBuffer` 对
    /// `Usage = DYNAMIC` 直接返 `E_INVALIDARG`**」—— 后半句**是错的**：本机建不出的是
    /// **非法组合** `DYNAMIC` + `CPUAccessFlags: 0`；正确写法在四块适配器上都通过。
    /// 我据此把缓冲改成 `DEFAULT` + `UpdateSubresource`，然后撞上了本文档开头那条
    /// `ByteWidth >= 8192` 必崩的坑。两个错误都是同一个模式：**把自己写的非法参数
    /// 当成了环境的静默失效**。证据见 `tests/probe_basic.rs` 与
    /// `tests/update_probe.rs`。
    fn write_buffer(
        &self,
        context: &ID3D11DeviceContext,
        buffer: &ID3D11Buffer,
        bytes: &[u8],
        capacity: u64,
    ) {
        if bytes.is_empty() {
            return;
        }
        // 缓冲只有一行 ⇒ `RowPitch == ByteWidth`，源与目标都紧密排列 ⇒ 连续拷。
        // 见本函数的文档：跳行那套只对纹理有意义。
        debug_assert!(
            bytes.len() as u64 <= capacity,
            "写入量超过缓冲容量"
        );
        let mut mapped = windows::Win32::Graphics::Direct3D11::D3D11_MAPPED_SUBRESOURCE::default();
        unsafe {
            context
                .Map(
                    buffer,
                    0,
                    windows::Win32::Graphics::Direct3D11::D3D11_MAP_WRITE_DISCARD,
                    0,
                    Some(&mut mapped as *mut _),
                )
                .expect("DYNAMIC 缓冲应当能 Map(WRITE_DISCARD)");
            std::ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                mapped.pData.cast::<u8>(),
                bytes.len(),
            );
            context.Unmap(buffer, 0);
        }
    }
}


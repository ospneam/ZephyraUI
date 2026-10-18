//! 预编译的着色器字节码。
//!
//! # 为什么**不在运行期**编译
//!
//! 最早的版本在运行期调 `d3dcompiler_47.dll` 的 `D3DCompile`。那踩了两个坑，
//! 第二个是硬伤：
//!
//! 1. `#[link(name = "d3dcompiler_47")]` 会让链接器去找**导入库**
//!    `d3dcompiler_47.lib`，它只在装了 Windows SDK 的机器上存在 —— 实测
//!    `LNK1181: 无法打开输入文件`。改成 `LoadLibraryExW` 动态装载后这条消失；
//! 2. `D3DCompile` 的 ABI 极易写错，而写错的表现是**运行时越界读**而不是编译错误。
//!    `windows` crate 恰好**没有** `D3DCompile` 的绑定（实测 0.62.2 全 crate
//!    搜不到该符号），所以这 10 个参数全靠手写。实测把 `SrcDataSize` 声明成
//!    `u32`（头文件是 `SIZE_T`）会在**调用瞬间** AV —— x64 上参数各占一个 64 位
//!    槽，宽度不对就让 DLL 按 8 字节读长度，拿到垃圾值后越界读。
//!
//! 与其和 ABI 搏斗，不如**让编译器在构建期跑完**。DXBC 是稳定且有文档的容器
//! 格式，D3D11 直接接受它，于是：
//!
//! - 运行期零编译、零 `d3dcompiler_47.dll` 依赖；
//! - 启动更快（省掉解析 HLSL 的那几十毫秒）；
//! - 没有手写 FFI，也就没有 ABI 宽度这一类只在运行时炸的错误。
//!
//! # 重新生成字节码
//!
//! ```text
//! fxc /T vs_5_0 /E VSMain shaders\zephyra.hlsl /Fo shaders\zephyra.vs.bin
//! fxc /T ps_5_0 /E PSMain shaders\zephyra.hlsl /Fo shaders\zephyra.ps.bin
//! ```
//!
//! `fxc.exe` 随 Windows SDK 装（`C:\Program Files (x86)\Windows Kits\10\bin\<ver>\x64`）。
//! 它是**构建期**工具而不是运行期依赖 —— 用 SDK 的机器才需要重新生成字节码，
//! 而普通用户拿到的只是已经编好的 `.bin`。
//!
//! # DXBC 容器格式
//!
//! 只需读两个字段，都在固定偏移上（`DXBC` 魔数后紧跟一个 header）：
//!
//! ```text
//! offset 0  : u32  'DXBC' = 0x43425844
//! offset 4  : u8[16] 校验和（本渲染器不校验）
//! offset 20 : u32  版本（1）
//! offset 24 : u32  总长度（含头）
//! offset 28 : u32  chunk 数量
//! offset 32 : u32  偏移表 —— 每个 chunk 一项
//! ```
//!
//! 我们要取输入签名（`ISGN` chunk）。`CreateInputLayout` 需要它，所以不能只
//! 拿字节码就算完 —— 那正是"把顶点布局描述符与着色器对齐"这一步的输入。

#![cfg(windows)]

use windows::Win32::Graphics::Direct3D11::{
    ID3D11Device, ID3D11InputLayout, ID3D11PixelShader, ID3D11VertexShader,
    D3D11_INPUT_CLASSIFICATION, D3D11_INPUT_ELEMENT_DESC, D3D11_INPUT_PER_VERTEX_DATA,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT, DXGI_FORMAT_R32G32B32A32_FLOAT, DXGI_FORMAT_R32G32_FLOAT, DXGI_FORMAT_R32_FLOAT,
};

use crate::quad::Vertex;

/// 顶点着色器字节码（`fxc /T vs_5_0`）。
pub const VS_BYTECODE: &[u8] = include_bytes!("../shaders/zephyra.vs.bin");

/// 像素着色器变体表：`(HLSL 入口名, 字节码)`。
///
/// # 为什么是**离线**变体而不是"一个 PS + 一个模式常量"
///
/// 模式常量要有地方放：常量缓冲（要新建 buffer + `SetConstantBuffers`，D3D12 那边
/// 还要动根签名）或顶点数据（把混合模式塞进与颜色无关的通道，污染 `quad::Vertex`
/// 的顶点契约）。离线变体两者都不需要，运行时只是多一张 `ID3D11PixelShader`。
///
/// 代价只是 `.bin` 变多 —— 而 `.bin` 本来就是离线成本。
///
/// # 顺序**必须**与 `filter.rs` 的 `pub enum BlendMode` 一致
///
/// `PS_FOR_BLEND_MODE` 按下标取表，所以 `BlendMode as usize` 直接就是索引。
/// `the_ps_variant_table_matches_the_blend_mode_enum` 逐项核对 29 个名字。
///
/// `PSMain` 是默认路径（系数可表达模式走固定功能混合方程，PS 只算源色）；
/// 紧随其后的 14 个是可分离 / 非可分离混合模式的合成变体（E4.2 / E4.3）。
/// 最后 3 个是效果执行用的：`PSBlurH` / `PSBlurV` / `PSComposite`。
pub const PS_VARIANTS: [(&str, &[u8]); 19] = [
    ("PSMain", include_bytes!("../shaders/zephyra.PSMain.ps.bin")),
    (
        "PSOverlay",
        include_bytes!("../shaders/zephyra.PSOverlay.ps.bin"),
    ),
    (
        "PSDarken",
        include_bytes!("../shaders/zephyra.PSDarken.ps.bin"),
    ),
    (
        "PSLighten",
        include_bytes!("../shaders/zephyra.PSLighten.ps.bin"),
    ),
    (
        "PSColorDodge",
        include_bytes!("../shaders/zephyra.PSColorDodge.ps.bin"),
    ),
    (
        "PSColorBurn",
        include_bytes!("../shaders/zephyra.PSColorBurn.ps.bin"),
    ),
    (
        "PSHardLight",
        include_bytes!("../shaders/zephyra.PSHardLight.ps.bin"),
    ),
    (
        "PSSoftLight",
        include_bytes!("../shaders/zephyra.PSSoftLight.ps.bin"),
    ),
    (
        "PSDifference",
        include_bytes!("../shaders/zephyra.PSDifference.ps.bin"),
    ),
    (
        "PSExclusion",
        include_bytes!("../shaders/zephyra.PSExclusion.ps.bin"),
    ),
    (
        "PSMultiply",
        include_bytes!("../shaders/zephyra.PSMultiply.ps.bin"),
    ),
    ("PSHue", include_bytes!("../shaders/zephyra.PSHue.ps.bin")),
    (
        "PSSaturation",
        include_bytes!("../shaders/zephyra.PSSaturation.ps.bin"),
    ),
    (
        "PSColor",
        include_bytes!("../shaders/zephyra.PSColor.ps.bin"),
    ),
    (
        "PSLuminosity",
        include_bytes!("../shaders/zephyra.PSLuminosity.ps.bin"),
    ),
    (
        "PSBlurH",
        include_bytes!("../shaders/zephyra.PSBlurH.ps.bin"),
    ),
    (
        "PSBlurV",
        include_bytes!("../shaders/zephyra.PSBlurV.ps.bin"),
    ),
    (
        "PSComposite",
        include_bytes!("../shaders/zephyra.PSComposite.ps.bin"),
    ),
    (
        "PSRefraction",
        include_bytes!("../shaders/zephyra.PSRefraction.ps.bin"),
    ),
];

/// 第 `i` 个混合模式该用哪个 PS 变体。
///
/// `None` = 用默认的 `PSMain`（系数可表达模式，混合交给固定功能方程）。
/// `Some(k)` = 用 `PS_VARIANTS[k]`。
///
/// 顺序与 `BlendMode` 的声明顺序一一对应：0..=14 是系数可表达（→ `None`），
/// 15..=24 可分离，25..=28 非可分离。
pub const fn ps_for_blend_mode(i: usize) -> Option<usize> {
    match i {
        0..=14 => None,
        15 => Some(1),  // Overlay
        16 => Some(2),  // Darken
        17 => Some(3),  // Lighten
        18 => Some(4),  // ColorDodge
        19 => Some(5),  // ColorBurn
        20 => Some(6),  // HardLight
        21 => Some(7),  // SoftLight
        22 => Some(8),  // Difference
        23 => Some(9),  // Exclusion
        24 => Some(10), // Multiply
        25 => Some(11), // Hue
        26 => Some(12), // Saturation
        27 => Some(13), // Color
        28 => Some(14), // Luminosity
        _ => None,
    }
}

/// 模糊水平 / 垂直两个 pass 的 PS 变体下标。
pub const PS_BLUR_H: usize = 15;
pub const PS_BLUR_V: usize = 16;
/// 离屏结果合成回目标（含投影染色）。
pub const PS_COMPOSITE: usize = 17;
/// 折射（液态玻璃）合成 —— 读 `g_dst`（**目标副本**）而不是 `g_tex`（离屏 RT）。
///
/// ⚠ 与 [`PS_COMPOSITE`] **互斥**：一条 pass 只走其中之一。
pub const PS_REFRACTION: usize = 18;

/// 顶点布局表：`(HLSL 语义名, 语义索引, DXGI 格式, 字节偏移)`。
///
/// 这些偏移与 `quad::Vertex` 的字段偏移一一对应 ——
/// `quad.rs` 里的 `vertex_field_offsets_match_the_input_layout_description`
/// 钉住那一半，这里是另一半。**两处必须一起改。**
/// 顶点布局表：`(HLSL 语义名, 语义索引, DXGI 格式, 字节偏移)`。
///
/// 这些偏移与 `quad::Vertex` 的字段偏移一一对应 ——
/// `quad.rs` 里的 `vertex_field_offsets_match_the_input_layout_description`
/// 钉住那一半，这里是另一半。**两处必须一起改。**
///
/// # 为什么位置用的是 `TEXCOORD4` 而不是 `POSITION`
///
/// 从 SM4 起 `POSITION` 在**顶点着色器输入**上是保留语义：`fxc` 把它编成系统值
/// （ISGN 里 `SystemValueType = 3` = `D3D_SV_POSITION`），D3D11 便认为它由流水线
/// 自己提供，**不从顶点缓冲取**。
///
/// 症状安静到极点：顶点缓冲、索引缓冲、输入签名逐项读回验证全部正确，draw 也正常
/// 提交，但每个顶点的位置都是 `(0,0)` —— 所有三角形退化到 NDC 原点，每个四边形
/// 只剩 1 个像素。`CreateInputLayout` 成功，日志零错误。
///
/// 代价只是多占一个 `TEXCOORD` 索引。位置是顶点缓冲里偏移 0 的那两个 f32，
/// 用哪个语义名都只是一段字符串。
pub const VERTEX_LAYOUT: [(&str, u32, DXGI_FORMAT, u32); 7] = [
    ("TEXCOORD", 0, DXGI_FORMAT_R32G32_FLOAT, 0), // position
    ("TEXCOORD", 2, DXGI_FORMAT_R32G32_FLOAT, 8), // local
    ("TEXCOORD", 4, DXGI_FORMAT_R32G32_FLOAT, 16), // uv
    ("COLOR", 0, DXGI_FORMAT_R32G32B32A32_FLOAT, 24), // color
    ("TEXCOORD", 6, DXGI_FORMAT_R32G32_FLOAT, 40), // corner
    ("TEXCOORD", 8, DXGI_FORMAT_R32G32_FLOAT, 48), // half_size
    ("TEXCOORD", 10, DXGI_FORMAT_R32_FLOAT, 56),  // kind
];

/// 编译期错误。
#[derive(Debug)]
pub enum ShaderError {
    /// 字节码不是 DXBC 容器（`fxc` 没跑，或 `.bin` 被截断/覆盖）。
    NotDxbc,
    /// 找不到输入签名 chunk —— `CreateInputLayout` 没法建顶点布局。
    NoInputSignature,
    /// 字节码越界或自相矛盾。
    Malformed(&'static str),
    /// `Create*` 调用失败。
    Create(&'static str, windows::core::Error),
}

impl std::fmt::Display for ShaderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShaderError::NotDxbc => write!(
                f,
                "着色器字节码不是 DXBC 容器。请重新生成：\
                 fxc /T vs_5_0 /E VSMain shaders/zephyra.hlsl /Fo shaders/zephyra.vs.bin"
            ),
            ShaderError::NoInputSignature => {
                write!(f, "DXBC 里没有 ISGN（输入签名）chunk，无法建顶点布局")
            }
            ShaderError::Malformed(why) => write!(f, "DXBC 字节码损坏: {why}"),
            ShaderError::Create(what, e) => write!(f, "{what} 失败: {e}"),
        }
    }
}

impl std::error::Error for ShaderError {}

/// 编译期就绪的着色器。
pub struct Shaders {
    pub vs: ID3D11VertexShader,
    /// 全部 PS 变体，下标与 [`PS_VARIANTS`] 一致。
    ///
    /// 逐段选 PS（而不是全局一个）是因为混合模式**逐段**不同 —— 见
    /// [`crate::renderer::Renderer`] 里按 `runs` 回放的那段。
    pub ps: Vec<ID3D11PixelShader>,
    pub input_layout: ID3D11InputLayout,
}

impl Shaders {
    /// 从预编译字节码创建。
    ///
    /// 失败必须冒泡：着色器缺失或字节码损坏的唯一症状是"窗口全透明 / 全空白"，
    /// 而那与"没有绘制任何东西"看起来一模一样。
    pub fn new(device: &ID3D11Device) -> Result<Self, ShaderError> {
        let mut vs = None;
        unsafe { device.CreateVertexShader(VS_BYTECODE, None, Some(&mut vs as *mut _)) }
            .map_err(|e| ShaderError::Create("CreateVertexShader", e))?;
        let mut ps = Vec::with_capacity(PS_VARIANTS.len());
        for (i, (name, bytes)) in PS_VARIANTS.iter().enumerate() {
            let mut p = None;
            unsafe { device.CreatePixelShader(bytes, None, Some(&mut p as *mut _)) }
                .map_err(|e| ShaderError::Create(name, e))?;
            ps.push(p.unwrap_or_else(|| {
                panic!("CreatePixelShader({name}) 成功时必给着色器（变体 #{i}）")
            }));
        }
        let vs = vs.expect("CreateVertexShader 成功时必给着色器");

        let holders: Vec<InputElement> = VERTEX_LAYOUT
            .iter()
            .map(|&(n, i, f, off)| InputElement::new(n, i, f, off))
            .collect();
        let descs: Vec<D3D11_INPUT_ELEMENT_DESC> = holders.iter().map(|h| h.as_desc()).collect();

        // 最后一个元素的偏移 + 宽度必须不超过 stride。越界不报错，症状是 `kind`
        // 被读成垃圾值 → 图元类型随机错乱（图标变纯色块 / 字变方块）。
        //
        // 宽度由 `VERTEX_LAYOUT` 末行自己说明 —— `DXGI_FORMAT` 在 0.62 **没有**
        // `channel_count()`，不能从格式反推。
        const LAST_ELEMENT_BYTES: u32 = 4; // DXGI_FORMAT_R32_FLOAT = 1 通道 f32
        debug_assert_eq!(
            VERTEX_LAYOUT[VERTEX_LAYOUT.len() - 1].2,
            DXGI_FORMAT_R32_FLOAT,
            "最后一个元素变了就要同步改 LAST_ELEMENT_BYTES"
        );
        let last = holders.last().expect("VERTEX_LAYOUT 非空");
        let covered = last.aligned_byte_offset + LAST_ELEMENT_BYTES;
        let stride = std::mem::size_of::<Vertex>() as u32;
        if covered > stride {
            return Err(ShaderError::Malformed("顶点布局越界"));
        }

        // `CreateInputLayout` 要的是**完整字节码** + 一个标记为
        // `D3D11_CP_UNUSED`-风格的偏移 —— 不是裸的 ISGN。
        //
        // 两个签名都试过，都失败（E_INVALIDARG 0x80070057）：
        //
        // 1. 传整个 DXBC 容器；
        // 2. 传裸 ISGN chunk 数据（本文件上一版的做法）。
        //
        // 正确用法：第二个参数是**顶点着色器的完整字节码**，第三个参数传
        // `D3D11_INPUT_LAYOUT_DESC_HINT{ pInputLayoutDescs: <描述符>, ... }`
        // —— 实际上 `CreateInputLayout(pDesc, pVertexShaderDeclaration, pInputLayout)` 的
        // 第 2 个参数文档写的是「顶点着色器的**声明**」，而 DXBC 字节码里恰好包含
        // 那份声明（ISGN chunk）。
        //
        // 本机（fxc 10.1）实测：传**完整 VS 字节码**成功，传裸 ISGN 失败。
        // 差别在于 D3D 需要从字节码的 chunk 表里定位并校验，而裸 ISGN 缺少
        // `DXBC` 头 —— 校验直接判为非法。
        let mut input_layout = None;
        unsafe { device.CreateInputLayout(&descs, VS_BYTECODE, Some(&mut input_layout as *mut _)) }
            .map_err(|e| {
                ShaderError::Create(
                    "CreateInputLayout（多半是 VERTEX_LAYOUT 的语义名/格式与 HLSL 的 VSIn 对不上）",
                    e,
                )
            })?;

        Ok(Self {
            vs,
            ps,
            input_layout: input_layout.expect("CreateInputLayout 成功时必给布局"),
        })
    }
}

/// HLSL 语义名在 `D3D11_INPUT_ELEMENT_DESC` 里是 `PCSTR`（**窄**字符串，NUL 结尾）。
/// 这层 holder 拥有那些字节，让描述符借用 —— 因为描述符里存的是裸指针。
struct InputElement {
    name: [u8; 12],
    semantic_index: u32,
    format: DXGI_FORMAT,
    input_slot: u32,
    aligned_byte_offset: u32,
    /// 0.62 显式建模了这个字段（0.58 时代靠结构体布局隐式补齐）。漏填它会让
    /// `InstanceDataStepRate` 落到错误的偏移 —— 症状是顶点被当成实例数据处理。
    input_slot_class: D3D11_INPUT_CLASSIFICATION,
    instance_data_step_rate: u32,
}

impl InputElement {
    fn new(name: &str, semantic_index: u32, format: DXGI_FORMAT, offset: u32) -> Self {
        let mut buf = [0u8; 12];
        let bytes = name.as_bytes();
        // 12 字节够装 "TEXCOORD"(8) + NUL。放不下说明表里语义名写错 —— 截断会让
        // D3D 读到相邻字节，症状是"绑定不到语义"且原因不明。
        assert!(bytes.len() < buf.len(), "HLSL 语义名 {name:?} 超出 11 字节");
        buf[..bytes.len()].copy_from_slice(bytes);
        Self {
            name: buf,
            semantic_index,
            format,
            input_slot: 0,
            aligned_byte_offset: offset,
            input_slot_class: D3D11_INPUT_PER_VERTEX_DATA,
            instance_data_step_rate: 0,
        }
    }

    fn as_desc(&self) -> D3D11_INPUT_ELEMENT_DESC {
        D3D11_INPUT_ELEMENT_DESC {
            SemanticName: windows::core::PCSTR(self.name.as_ptr()),
            SemanticIndex: self.semantic_index,
            Format: self.format,
            InputSlot: self.input_slot,
            AlignedByteOffset: self.aligned_byte_offset,
            InputSlotClass: self.input_slot_class,
            InstanceDataStepRate: self.instance_data_step_rate,
        }
    }
}

/// DXBC chunk 的四字节标签（输入签名）。
///
/// 只在**测试**里用 —— 运行期不解析字节码，直接把整份 VS 字节码交给
/// `CreateInputLayout`（那才是 D3D 要的东西，见 [`Shaders::new`]）。
#[cfg(test)]
const FOURCC_ISGN: [u8; 4] = *b"ISGN";

/// 从 DXBC 容器里取出 `ISGN`（输入签名）chunk。
///
/// # 只用于测试
///
/// 运行期**不需要**这个函数：`CreateInputLayout` 要的是完整字节码，我们直接
/// 传 [`VS_BYTECODE`]。解析 `ISGN` 只为了一件事 —— 断言"着色器声明的输入"与
/// "`VERTEX_LAYOUT` 描述给 D3D 的输入"逐项一致。
///
/// # 容器布局（已逐字节验证于本机 fxc 10.1 的输出）
///
/// ```text
/// 0  u32   'DXBC'
/// 4  [16]  校验和
/// 20 u32   版本 = 1
/// 24 u32   总长度
/// 28 u32   chunk 数
/// 32 u32[chunk 数]  每个 chunk 的绝对偏移
/// ```
///
/// 每个 chunk 是「4 字节标签 + 4 字节长度 + 数据」。所有整数都是**小端**。
///
/// 写这个函数的过程本身是个教训：它先按"元素数组直接开头"解析，读出第 5 个
/// 语义名是 `"TION"` —— 那是 `"POSITION"` 的尾巴，一眼就看出落点错了 8 字节。
/// 若不是那个可读的破绽，症状会是 `CreateInputLayout` 回 `E_INVALIDARG`，
/// 而那条消息只说「参数错误」。
#[cfg(test)]
fn input_signature(container: &[u8]) -> Result<Vec<u8>, ShaderError> {
    const HEADER_LEN: usize = 32;
    if container.len() < HEADER_LEN || &container[0..4] != b"DXBC" {
        return Err(ShaderError::NotDxbc);
    }
    let rd_u32 = |off: usize| -> u32 {
        u32::from_le_bytes([
            container[off],
            container[off + 1],
            container[off + 2],
            container[off + 3],
        ])
    };
    let total = rd_u32(24) as usize;
    if total > container.len() {
        return Err(ShaderError::Malformed("头里的总长度超过实际字节数"));
    }
    let count = rd_u32(28) as usize;
    let table_end = HEADER_LEN
        .checked_add(
            count
                .checked_mul(4)
                .ok_or(ShaderError::Malformed("chunk 数溢出"))?,
        )
        .ok_or(ShaderError::Malformed("偏移表长度溢出"))?;
    if table_end > container.len() {
        return Err(ShaderError::Malformed("chunk 偏移表越界"));
    }
    for i in 0..count {
        let off = rd_u32(HEADER_LEN + i * 4) as usize;
        // 每个 chunk 是「4 字节标签 + 4 字节长度 + 数据」。
        if off + 8 > total {
            return Err(ShaderError::Malformed("chunk 偏移越界"));
        }
        if container[off..off + 4] == FOURCC_ISGN {
            let len = u32::from_le_bytes([
                container[off + 4],
                container[off + 5],
                container[off + 6],
                container[off + 7],
            ]) as usize;
            let start = off + 8;
            if start + len > container.len() {
                return Err(ShaderError::Malformed("ISGN chunk 长度越界"));
            }
            return Ok(container[start..start + len].to_vec());
        }
    }
    Err(ShaderError::NoInputSignature)
}

/// 一个输入签名的语义名、索引与分量数（供测试比对 `VERTEX_LAYOUT`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureEntry {
    /// 语义名（`POSITION` / `TEXCOORD` / `COLOR`）。
    pub name: String,
    /// 语义索引（同名语义出现多次时区分，如 `TEXCOORD0` 与 `TEXCOORD1`）。
    pub index: u32,
    /// 分量数（从 mask 字节数出来：`float2` → 2）。
    pub component_count: u32,
    /// `D3D_SYSTEM_VALUE` 枚举值（`POSITION` → 3 即 `D3D_SV_POSITION`）。
    ///
    /// 与分量数**不是**同一个字段 —— 我最初读错过，见 [`parse_input_signature`]。
    pub system_value: u32,
}

/// 解析 `ISGN`，列出语义名与索引。
///
/// 存在的理由：上面那个 8 字节头的 bug 只能靠"与 `VERTEX_LAYOUT` 逐一比对"发现。
/// 有了这个函数，测试就能断言"着色器里声明的输入"与"我们描述给 D3D 的输入"
/// 恰好一致 —— 而两者不一致时 `CreateInputLayout` 只回一个 `E_INVALIDARG`。
pub fn parse_input_signature(isgn: &[u8]) -> Result<Vec<SignatureEntry>, ShaderError> {
    if isgn.len() < 8 {
        return Err(ShaderError::Malformed("ISGN 太短"));
    }
    let rd = |d: &[u8], off: usize| -> u32 {
        u32::from_le_bytes([d[off], d[off + 1], d[off + 2], d[off + 3]])
    };
    let count = rd(isgn, 0) as usize;
    // 第二个字段是**元素记录区的起始偏移**，不是字符串池的偏移（我最初读错过：
    // 它的值是 8，而 "POSITION" 在 0x98 —— 差 0x90）。
    //
    // 字符串池的起点**不在头里**，它是 `记录区末尾`，也正是名字偏移所指向的
    // 基准（实测名字偏移 0x98 == 8 + 6×24）。所以池边界由记录区推出来，
    // 而不是读出来。
    let records = rd(isgn, 4) as usize;
    if records < 8 || records > isgn.len() {
        return Err(ShaderError::Malformed("ISGN 的记录区偏移越界"));
    }
    // 每条记录 **24** 字节：
    //   name_offset(4) + semantic_index(4) + system_value(4)
    // + component_count(4) + register(4) + mask(1) + never_writes(1) + pad(2)
    //
    // 24 是**唯一**与实测自洽的候选：本着色器的名字偏移是 0x98 = 152，而
    // `8 + 6×24 = 152` 恰好等于它 —— 记录区正好结束在字符串池开始处。
    // 两个错值都会让记录区越过池，末尾记录读到的将是名字字节：
    //   - 28 → 8 + 6×28 = 176 > 152
    //   - 32 → 8 + 6×32 = 200 > 152
    const RECORD_LEN: usize = 24;
    let records_end = records
        .checked_add(
            count
                .checked_mul(RECORD_LEN)
                .ok_or(ShaderError::Malformed("元素数溢出"))?,
        )
        .ok_or(ShaderError::Malformed("记录区末尾溢出"))?;
    if records_end > isgn.len() {
        return Err(ShaderError::Malformed("ISGN 元素记录区越界"));
    }
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let r = records + i * RECORD_LEN;
        let name_off = rd(isgn, r) as usize;
        let index = rd(isgn, r + 4);
        // +12 是 `SystemValueType`，**不是**分量数。
        //
        // 我最初把它读成 component_count，于是 `POSITION`（float2）读出 3 ——
        // 而 3 其实是 `D3D_SV_POSITION` 这个枚举值。两者数值相近到不会报错，
        // 只会在"和 DXGI 格式的通道数比对"时变成一个假的失败。
        //
        // 分量数要从 **mask** 字节（+20）数 1 的个数：`float2` → 0x03，
        // `float4` → 0x0F，`float` → 0x01。
        let system_value = rd(isgn, r + 12);
        let mask = isgn[r + 20];
        let component_count = mask.count_ones();
        // 名字是 NUL 结尾，偏移相对 ISGN 数据起点（不是相对记录或池）。
        //
        // 顺带校验它落在池内 —— 名字偏移本该 `>= records_end`（池在记录区之后），
        // 不满足说明 `RECORD_LEN` 取错了，而那会让这里读出字符串中间的碎片
        // （最典型的表现是读出 `"TION"` —— `"POSITION"` 的尾巴）。
        if name_off < records_end || name_off >= isgn.len() {
            return Err(ShaderError::Malformed(
                "ISGN 语义名偏移落在记录区之外（RECORD_LEN 可能取错）",
            ));
        }
        if mask == 0 {
            return Err(ShaderError::Malformed("ISGN 元素的 mask 为 0"));
        }
        let tail = &isgn[name_off..];
        let end = tail.iter().position(|&c| c == 0).unwrap_or(tail.len());
        out.push(SignatureEntry {
            name: String::from_utf8_lossy(&tail[..end]).into_owned(),
            index,
            component_count,
            system_value,
        });
    }
    Ok(out)
}

#[cfg(test)]
#[path = "tests/shader_tests.rs"]
mod tests;

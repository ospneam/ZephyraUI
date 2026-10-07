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
    D3D11_INPUT_CLASSIFICATION, D3D11_INPUT_ELEMENT_DESC, D3D11_INPUT_PER_VERTEX_DATA,
    ID3D11Device, ID3D11InputLayout, ID3D11PixelShader, ID3D11VertexShader,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT, DXGI_FORMAT_R32_FLOAT, DXGI_FORMAT_R32G32B32A32_FLOAT, DXGI_FORMAT_R32G32_FLOAT,
};

use crate::quad::Vertex;

/// 顶点着色器字节码（`fxc /T vs_5_0`）。
pub const VS_BYTECODE: &[u8] = include_bytes!("../shaders/zephyra.vs.bin");

/// 像素着色器字节码（`fxc /T ps_5_0`）。
pub const PS_BYTECODE: &[u8] = include_bytes!("../shaders/zephyra.ps.bin");

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
    ("TEXCOORD", 0, DXGI_FORMAT_R32G32_FLOAT, 0),          // position
    ("TEXCOORD", 2, DXGI_FORMAT_R32G32_FLOAT, 8),          // local
    ("TEXCOORD", 4, DXGI_FORMAT_R32G32_FLOAT, 16),         // uv
    ("COLOR", 0, DXGI_FORMAT_R32G32B32A32_FLOAT, 24),      // color
    ("TEXCOORD", 6, DXGI_FORMAT_R32G32_FLOAT, 40),         // corner
    ("TEXCOORD", 8, DXGI_FORMAT_R32G32_FLOAT, 48),         // half_size
    ("TEXCOORD", 10, DXGI_FORMAT_R32_FLOAT, 56),          // kind
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
    pub ps: ID3D11PixelShader,
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
        let mut ps = None;
        unsafe { device.CreatePixelShader(PS_BYTECODE, None, Some(&mut ps as *mut _)) }
            .map_err(|e| ShaderError::Create("CreatePixelShader", e))?;
        let vs = vs.expect("CreateVertexShader 成功时必给着色器");
        let ps = ps.expect("CreatePixelShader 成功时必给着色器");

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
        unsafe {
            device.CreateInputLayout(&descs, VS_BYTECODE, Some(&mut input_layout as *mut _))
        }
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
        .checked_add(count.checked_mul(4).ok_or(ShaderError::Malformed("chunk 数溢出"))?)
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
        .checked_add(count.checked_mul(RECORD_LEN).ok_or(ShaderError::Malformed("元素数溢出"))?)
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
mod tests {
    use super::*;

    /// 字节码必须是真 DXBC —— 否则运行期才炸，且症状是"窗口空白"。
    ///
    /// 这条断言钉住"`.bin` 文件被提交进来了"这件事：漏提交时 `include_bytes!`
    /// 在编译期就失败，而格式写错只会在运行期变成一片空白。
    #[test]
    fn both_bytecode_blobs_are_dxbc() {
        for (name, b) in [("vs", VS_BYTECODE), ("ps", PS_BYTECODE)] {
            assert!(b.len() >= 32, "{name} 太短（{} B），大概没生成", b.len());
            assert_eq!(&b[0..4], b"DXBC", "{name} 魔数不对");
            let total =
                u32::from_le_bytes([b[24], b[25], b[26], b[27]]) as usize;
            assert_eq!(
                total,
                b.len(),
                "{name} 头里的长度与实际字节数不符（重新生成字节码）"
            );
        }
    }

    /// 顶点字节码必须含输入签名 —— 没有它就建不出顶点布局。
    #[test]
    fn the_vertex_bytecode_carries_an_input_signature() {
        let sig = input_signature(VS_BYTECODE).expect("VS 必须有 ISGN chunk");
        let entries = parse_input_signature(&sig).expect("ISGN 必须能解析");
        assert!(!entries.is_empty(), "ISGN 元素列表不该为空");
        assert_eq!(
            entries.len(),
            VERTEX_LAYOUT.len(),
            "ISGN 元素数必须与 VERTEX_LAYOUT 一致，否则语义名对不上"
        );
    }

    /// **着色器声明的输入**必须与**我们描述给 D3D 的输入**逐一对应。
    ///
    /// 这条断言存在的原因：两者不一致时 `CreateInputLayout` 返回
    /// `E_INVALIDARG`（0x80070057），而那条消息只说「参数错误」——
    /// 不会告诉你是哪个语义名、是第几个、还是解析 ISGN 时落点错了 8 字节。
    ///
    /// 我就踩过后者：ISGN 那个 8 字节头被漏掉，解析出的第 5 个语义名成了
    /// `"TION"`（`"POSITION"` 的尾巴）。靠肉眼看 hex 才发现。
    #[test]
    fn the_shader_signature_matches_our_vertex_layout() {
        let sig = input_signature(VS_BYTECODE).expect("VS 必须有 ISGN chunk");
        let entries = parse_input_signature(&sig).expect("ISGN 必须能解析");
        let ours: Vec<(String, u32, u32)> = VERTEX_LAYOUT
            .iter()
            .map(|&(n, i, fmt, _)| {
                // 组件数 = 格式的通道数。DXGI_FORMAT 的通道数没有辅助方法，
                // 所以在这里显式映射 —— 这张表就是「格式 ↔ float 数量」。
                let comps = match fmt {
                    DXGI_FORMAT_R32_FLOAT => 1,
                    DXGI_FORMAT_R32G32_FLOAT => 2,
                    DXGI_FORMAT_R32G32B32A32_FLOAT => 4,
                    other => panic!("VERTEX_LAYOUT 里出现了未预期的格式 {other:?}"),
                };
                (n.to_string(), i, comps)
            })
            .collect();

        for (k, (name, index, comps)) in ours.iter().enumerate() {
            let theirs = &entries[k];
            assert_eq!(
                (&theirs.name, theirs.index, theirs.component_count),
                (name, *index, *comps),
                "第 {k} 个输入不一致 —— \
                 这会让 CreateInputLayout 回 E_INVALIDARG（参数错误），\
                 而那条消息不会说是哪个语义名对不上"
            );
        }
    }

    /// 语义索引区间必须**互不重叠**。
    ///
    /// # 为什么这条不能省
    ///
    /// 上面那条 `the_shader_signature_matches_our_vertex_layout` 只保证「我们的表
    /// 与着色器一致」。但**两边同时错**时它照样通过 —— 而这正是本项目真踩过的：
    /// 旧的 D3D12 后端用 `position:TEXCOORD4 / half_size:TEXCOORD5`，而 HLSL 里
    /// `float2 x : TEXCOORDn` 占**两格**语义索引（分量 0 在 n、分量 1 在 n+1），
    /// 于是 4、5 与 5、6 重叠：`half_size.x` 读到的是别的属性的分量。
    ///
    /// 症状是「所有圆角矩形变成均匀的半透明色块」（SDF 恒为常数），且不报任何错 ——
    /// 那与"半边 alpha 公式写错"完全同形，于是排查方向会一路跑到像素函数上去。
    ///
    /// 修好之后两边的表与 HLSL 都是 `0/2/4/6/8/10`，但**没有任何一条测试在守这个
    /// 约束** —— 于是下一次有人"顺手把索引改紧凑一点"就会静默地把它打回去。
    /// 这条断言就是那道护栏。
    ///
    /// D3D12 那边有一条同名的（`shader.rs::语义索引区间互不重叠`）。两个后端是**同类
    /// 缺陷**，只给一个加防护等于把另一个留在裸奔状态。
    #[test]
    fn semantic_index_ranges_do_not_overlap() {
        fn slots(fmt: DXGI_FORMAT) -> u32 {
            match fmt {
                DXGI_FORMAT_R32_FLOAT => 1,
                DXGI_FORMAT_R32G32_FLOAT => 2,
                DXGI_FORMAT_R32G32B32A32_FLOAT => 4,
                other => panic!("VERTEX_LAYOUT 里出现了未预期的格式 {other:?}"),
            }
        }
        // ⚠ 只在**同一语义名内部**比区间：`TEXCOORDn` 与 `COLORn` 是两套独立的
        // 索引空间，从不互相干扰。混在一起比会报出一堆假重叠（本表里
        // `COLOR 0..4` 与 `TEXCOORD 0..2` 区间相交，但那本来就合法），
        // 而为了让测试变绿去放宽判据更糟 —— 那等于把唯一的护栏改成摆设。
        let mut used: Vec<(&str, usize, usize, usize)> = Vec::new();
        for (idx, (name, index, fmt, _)) in VERTEX_LAYOUT.iter().enumerate() {
            let lo = *index as usize;
            let hi = lo + slots(*fmt) as usize;
            for (uname, ulo, uhi, uidx) in &used {
                if *uname != *name {
                    continue;
                }
                assert!(
                    hi <= *ulo || lo >= *uhi,
                    "同一语义名 {name} 内索引区间重叠：第 {idx} 项 [{lo}..{hi}] 与 \
                     第 {uidx} 项 [{ulo}..{uhi}] 相交 —— \
                     HLSL 里 float2 占**两格**索引，相邻属性必须至少隔 2。\
                     症状是片元读到别的属性的分量（SDF 恒为常数 → 圆角矩形变成均匀半透明块），\
                     且**不报任何错**"
                );
            }
            used.push((name, lo, hi, idx));
        }
        assert_eq!(used.len(), VERTEX_LAYOUT.len(), "内部错误：没覆盖全部输入元素");
    }

    /// 语义名不许有截断。
    ///
    /// 截断的症状极隐蔽：`"POSITION"` 被读成 `"POSITIO"` 时名字对不上，D3D 报
    /// `E_INVALIDARG`；而被读成 `"TION"`（前缀被吃掉）时更糟 —— 名字里居然还
    /// 是个合法标识符，肉眼扫一遍 hex 很容易滑过去。
    #[test]
    fn every_signature_name_is_fully_read() {
        let sig = input_signature(VS_BYTECODE).expect("VS 必须有 ISGN chunk");
        for e in parse_input_signature(&sig).expect("ISGN 必须能解析") {
            assert!(
                ["POSITION", "TEXCOORD", "COLOR"].contains(&e.name.as_str()),
                "读出意外的名字 {:?} —— 多半是解析落点错了",
                e.name
            );
        }
    }

    /// 非 DXBC 输入必须被明确拒绝，而不是被当成"有签名的字节码"。
    #[test]
    fn a_non_dxbc_blob_is_rejected() {
        assert!(matches!(
            input_signature(b"not a shader at all, really"),
            Err(ShaderError::NotDxbc)
        ));
        assert!(matches!(input_signature(&[]), Err(ShaderError::NotDxbc)));
    }

    /// 顶点布局的最后一个元素不得越过 stride。
    #[test]
    fn the_vertex_layout_fits_inside_one_vertex() {
        let last = VERTEX_LAYOUT[VERTEX_LAYOUT.len() - 1];
        assert_eq!(
            last.2,
            DXGI_FORMAT_R32_FLOAT,
            "最后一个元素变了就要同步改 renderer.rs 里的 LAST_ELEMENT_BYTES"
        );
        let covered = last.3 + 4;
        let stride = std::mem::size_of::<Vertex>() as u32;
        assert!(covered <= stride, "越界：{covered} > {stride}");
    }

    /// `kind` 的数值必须与 `shader.hlsl` 里 PSMain 的阈值配对。
    ///
    /// 改了 `quad::kind` 的数值而不改 HLSL 的 `1.5` 阈值，症状是"图标变成纯色块"
    /// 或"字变成方块" —— 没有任何编译期信号。
    /// `kind` 的取值必须落在着色器划定的三个互不重叠的区间里。
    ///
    /// HLSL 的 PSMain：`kind > 0.5 && kind < 1.5` → 字形；`kind > 1.5` → 图像；
    /// 否则 → 圆角矩形。三类共用一条指令流，所以"落进哪个区间"就是"画成什么"。
    ///
    /// 若 `GLYPH` 取了 2.0 它会落进图像分支，症状是**所有文字变成图像采样**
    /// —— 表现为文字乱码或整块错位，而不是"某一个字符不对"。
    ///
    /// 检查是常量折叠的，所以 clippy 会报 `assertions_on_constants`。那条警告在
    /// 这里同样是误报：我们要的正是「这些常量一旦被改坏，编译就该吵」。同样的
    /// 检查在 [`crate::quad::kind_branch_tests`] 里有第二份 —— 那边验证的是
    /// `quad` 侧的定义，这边验证的是**从 HLSL 阈值推出来的**区间，两边独立。
    #[test]
    #[allow(
        clippy::assertions_on_constants,
        reason = "防的正是常量被改坏；编译期求值正是我们要的"
    )]
    fn the_kind_values_land_in_disjoint_shader_branches() {
        use crate::quad::kind_thresholds::{GLYPH_MAX, GLYPH_MIN, IMAGE_MIN};
        use crate::quad::kind::{GLYPH, IMAGE, RECT};

        assert!(
            RECT < GLYPH_MIN,
            "RECT={RECT} 必须落在 SDF 分支（kind <= {GLYPH_MIN}）"
        );
        assert!(
            GLYPH > GLYPH_MIN && GLYPH < GLYPH_MAX,
            "GLYPH={GLYPH} 必须落在 ({GLYPH_MIN}, {GLYPH_MAX})"
        );
        assert!(
            IMAGE > IMAGE_MIN,
            "IMAGE={IMAGE} 必须落在图像分支（kind > {IMAGE_MIN}）"
        );
        // 有序：`nointerpolation` 保证没有插值中间值，但顺序错乱仍会让某个值
        // 落进它**不属于**的区间（上面三条正是逐条排除这种情形）。
        assert!(RECT < GLYPH && GLYPH < IMAGE, "三者必须有序");
    }
}

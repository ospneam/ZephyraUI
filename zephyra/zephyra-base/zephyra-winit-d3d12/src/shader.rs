//! 预编译的着色器字节码与管线状态（PSO）构件。
//!
//! # 与 D3D11 版（`zephyra-winit::shader`）的差别
//!
//! 两者做的事相同：把 `fxc` 编好的字节码交给驱动、建顶点布局描述。
//! 但有两处实质差别，踩过才知道：
//!
//! 1. **D3D12 没有 `CreateInputLayout`。** D3D12 的输入布局只是
//!    `D3D12_GRAPHICS_INPUT_LAYOUT_DESC` 里的一张**描述表**，随 PSO 一起
//!    `CreateGraphicsPipelineState` 生效 —— 没有单独的对象、也没有单独的 HRESULT。
//!    这消灭了 D3D11 那个「布局与签名对不上 → `E_INVALIDARG`」的失败点。
//!
//! 2. **`POSITION` 在 SM4+ 的顶点着色器输入上是保留名。** `fxc` 把它编成系统值
//!    语义（ISGN 里 `SystemValueType = 3` = `SV_POSITION`），于是**顶点缓冲不会被
//!    喂给它** —— 顶点位置恒为 `(0,0)`，所有三角形退化到原点、每个四边形只剩
//!    1 个像素，而**不报任何错**。
//!
//!    本项目在 D3D11 上踩过这个坑（见 `zephyra-winit` 里的记录）。所以下面的
//!    [`VERTEX_LAYOUT`] 用 `TEXCOORD4` 而不是 `POSITION` 承载位置，DX11 侧也已
//!    改成一致 —— **两边必须一起改**。

#![cfg(windows)]

use windows::core::PCSTR;
use windows::Win32::Graphics::Direct3D12::{D3D12_INPUT_ELEMENT_DESC, D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT, DXGI_FORMAT_R32G32B32A32_FLOAT, DXGI_FORMAT_R32G32_FLOAT, DXGI_FORMAT_R32_FLOAT,
};

use crate::quad::Vertex;

/// 顶点着色器字节码（`fxc /T vs_5_1`）。
pub const VS_BYTECODE: &[u8] = include_bytes!("../shaders/zephyra.vs.bin");

/// 像素着色器**变体表**，与 `zephyra-winit`（D3D11 那边）**逐项一致**。
///
/// 两边各写一份是刻意的：`.bin` 是**同一批**离线产物（`shader_assets_are_in_sync`
/// 保证两份 `.hlsl` 与全部 `.bin` 字节一致），但"哪个模式用哪个变体"是**后端自己的
/// 决定**，放在一起反而会让两边互相牵制。
///
/// 顺序与 `BlendMode` 的声明顺序对应，见 [`ps_for_blend_mode`]。
pub const PS_VARIANTS: [(&str, &[u8]); 18] = [
    ("PSMain", include_bytes!("../shaders/zephyra.PSMain.ps.bin")),
    ("PSOverlay", include_bytes!("../shaders/zephyra.PSOverlay.ps.bin")),
    ("PSDarken", include_bytes!("../shaders/zephyra.PSDarken.ps.bin")),
    ("PSLighten", include_bytes!("../shaders/zephyra.PSLighten.ps.bin")),
    ("PSColorDodge", include_bytes!("../shaders/zephyra.PSColorDodge.ps.bin")),
    ("PSColorBurn", include_bytes!("../shaders/zephyra.PSColorBurn.ps.bin")),
    ("PSHardLight", include_bytes!("../shaders/zephyra.PSHardLight.ps.bin")),
    ("PSSoftLight", include_bytes!("../shaders/zephyra.PSSoftLight.ps.bin")),
    ("PSDifference", include_bytes!("../shaders/zephyra.PSDifference.ps.bin")),
    ("PSExclusion", include_bytes!("../shaders/zephyra.PSExclusion.ps.bin")),
    ("PSMultiply", include_bytes!("../shaders/zephyra.PSMultiply.ps.bin")),
    ("PSHue", include_bytes!("../shaders/zephyra.PSHue.ps.bin")),
    ("PSSaturation", include_bytes!("../shaders/zephyra.PSSaturation.ps.bin")),
    ("PSColor", include_bytes!("../shaders/zephyra.PSColor.ps.bin")),
    ("PSLuminosity", include_bytes!("../shaders/zephyra.PSLuminosity.ps.bin")),
    ("PSBlurH", include_bytes!("../shaders/zephyra.PSBlurH.ps.bin")),
    ("PSBlurV", include_bytes!("../shaders/zephyra.PSBlurV.ps.bin")),
    ("PSComposite", include_bytes!("../shaders/zephyra.PSComposite.ps.bin")),
];

/// 第 `i` 个混合模式该用哪个 PS 变体；`None` = 默认 `PSMain` + 固定功能混合。
///
/// 与 D3D11 那边**逐项一致** —— 两边的行为必须一样，否则同一个 `DisplayList`
/// 在两个后端上长得不一样。
pub const fn ps_for_blend_mode(i: usize) -> Option<usize> {
    match i {
        0..=14 => None,
        15 => Some(1),
        16 => Some(2),
        17 => Some(3),
        18 => Some(4),
        19 => Some(5),
        20 => Some(6),
        21 => Some(7),
        22 => Some(8),
        23 => Some(9),
        24 => Some(10),
        25 => Some(11),
        26 => Some(12),
        27 => Some(13),
        28 => Some(14),
        _ => None,
    }
}

/// 模糊水平 / 垂直两个 pass 的 PS 变体下标。
pub const PS_BLUR_H: usize = 15;
pub const PS_BLUR_V: usize = 16;
/// 离屏结果合成回目标（含投影染色）。
pub const PS_COMPOSITE: usize = 17;

/// D3D12 那边不再有单一 `PS_BYTECODE`：见 [`PS_VARIANTS`]。
///
/// 两份 `.hlsl` 用 `ps_5_0` / `vs_5_0` 各编一份，D3D12 直接吃 SM5.0 字节码
/// （D3D12 不要求 5.1）。真正的约束是「两边 `.bin` 字节一致」。
///
/// 顶点布局表：`(HLSL 语义名, 语义索引, DXGI 格式, 字节偏移)`。
///
/// # 为什么格式也在这张表里
///
/// 早先这里只有三列（没有格式），格式在 [`input_elements`] 里被写死成
/// `R32G32_FLOAT`。而顶点里的属性宽度是**混的**：`color` 是 `float4`、`kind` 是
/// `float`、其余是 `float2`。一律声明成 `R32G32_FLOAT` 会让
/// `CreateGraphicsPipelineState` 回 `E_INVALIDARG` —— 它把描述的格式与着色器输入
/// 签名逐项对比，对不上就拒绝，且**不告诉你是第几个**。
///
/// 现在这张表与 D3D11 那边**形状相同**（`shader.rs::VERTEX_LAYOUT`），两个后端的
/// 顶点布局只有一处可写。代价是 DX12 的偏移测试要从 `.2` 改成 `.3`。
///
/// # 偏移必须与 `Vertex` 的字段偏移逐项对应
///
/// `quad.rs` 里的测试 `vertex_field_offsets_match_the_input_layout_description`
/// 钉住那一半；这里是另一半。**两处必须一起改。**
///
/// # 语义索引必须与 `zephyra.hlsl` 的 `VSIn` **逐项一致**
///
/// ⚠⚠ 这里曾经是 `TEXCOORD4 / 0 / 1 / COLOR0 / 2 / 5 / 3`，而 D3D11 那边早已
/// 修成 `0 / 2 / 4 / 6 / 8 / 10`。这份漂移没有任何测试能拦住（两边各自的
/// "签名与布局匹配"测试都通过——D3D12 的 PSO 校验按**名字+索引**比对，
/// 而两边 HLSL 各自的索引与各自的布局表是自洽的）。
///
/// 但那份索引本身是**错的**：HLSL 里 `float2 x : TEXCOORDn` 占用
/// `TEXCOORDn` 与 `TEXCOORD(n+1)` **两格**，于是 `position:TEXCOORD4`（占 4,5）
/// 与 `half_size:TEXCOORD5`（占 5,6）重叠、`local:TEXCOORD0`（占 0,1）与
/// `uv:TEXCOORD1`（占 1,2）重叠、`corner:TEXCOORD2`（占 2,3）与
/// `kind:TEXCOORD3`（占 3）重叠。
///
/// DX12 的 PSO 创建按区间绑定，只有不冲突的部分生效。症状与 DX11 上踩过的
/// 那次**完全一致**：几何正确、而每个属性的第一个 float 读到 0，于是 SDF 的
/// `local.x` 恒 0，画面变成一条竖线而不是方块。
///
/// 现在两份 HLSL 是逐字节相同的一份文件（由 `the_two_backends_ship_the_same_hlsl_source`
/// 钉住），本表与它严格对应。
pub const VERTEX_LAYOUT: [(&str, u32, DXGI_FORMAT, u32); 7] = [
    // position — 不用 POSITION，理由见模块文档
    ("TEXCOORD", 0, DXGI_FORMAT_R32G32_FLOAT, 0),
    ("TEXCOORD", 2, DXGI_FORMAT_R32G32_FLOAT, 8),     // local
    ("TEXCOORD", 4, DXGI_FORMAT_R32G32_FLOAT, 16),    // uv
    ("COLOR", 0, DXGI_FORMAT_R32G32B32A32_FLOAT, 24), // color
    ("TEXCOORD", 6, DXGI_FORMAT_R32G32_FLOAT, 40),    // corner
    // half_size — 四边形常量，理由见 quad.rs::Vertex::half_size
    ("TEXCOORD", 8, DXGI_FORMAT_R32G32_FLOAT, 48),
    ("TEXCOORD", 10, DXGI_FORMAT_R32_FLOAT, 56), // kind
];

/// 一个输入元素的**持有者**：语义名的字节 + 索引 + 偏移 + 格式。
///
/// # `name` 必须是 `Box` 而不是 `[u8; 12]`
///
/// `D3D12_INPUT_ELEMENT_DESC::SemanticName` 是**裸指针**，它必须指向一块一直有效的
/// 内存，直到 `CreateGraphicsPipelineState` 返回为止。
///
/// 早先这里是 `name: [u8; 12]`，而 [`input_elements`] 这么写：
///
/// ```text
/// .map(|&(n, i, off)| InputElement::new(n, i, off).as_desc(0))
/// ```
///
/// `InputElement::new(...)` 是**临时值**，`as_desc(&self)` 存下 `self.name.as_ptr()`
/// 之后闭包就返回了 —— 那块栈内存随即失效，**7 个语义名指针全是悬空的**。
///
/// ⚠ 曾把本 crate 卡住的那条 `CreateGraphicsPipelineState` → `E_INVALIDARG` 归因到
/// 这里。**那是错的归因**：真正的根因是 `RTVFormats` 的第 1..8 槽填了
/// `R8G8B8A8_UNORM`（必须 `UNKNOWN`），这句话是调试层说的。
///
/// 悬垂指针本身仍然是真 bug（症状是随机的 `E_INVALIDARG` 或崩溃），但它**不是**
/// 当时那条的原因。两条都被归进了「环境问题」，而它们都是本 crate 自己写的。
///
/// `Box` 让地址在堆上，与本结构体被移动到哪、与 `Vec` 是否扩容都无关。
struct InputElement {
    name: Box<[u8; 12]>,
    semantic_index: u32,
    aligned_byte_offset: u32,
    format: DXGI_FORMAT,
}

impl InputElement {
    fn new(name: &str, semantic_index: u32, aligned_byte_offset: u32, format: DXGI_FORMAT) -> Self {
        let mut buf = [0u8; 12];
        let bytes = name.as_bytes();
        // 12 字节够装 "TEXCOORD"(8) + NUL。放不下说明表里语义名写错 —— 截断会让
        // D3D 读到相邻字节，症状是「绑定不到语义」且原因不明。
        assert!(
            bytes.len() < buf.len(),
            "HLSL 语义名 {name:?} 超出 11 字节"
        );
        buf[..bytes.len()].copy_from_slice(bytes);
        Self {
            name: Box::new(buf),
            semantic_index,
            aligned_byte_offset,
            format,
        }
    }

    fn as_desc(&self, input_slot: u32) -> D3D12_INPUT_ELEMENT_DESC {
        D3D12_INPUT_ELEMENT_DESC {
            SemanticName: PCSTR(self.name.as_ptr()),
            SemanticIndex: self.semantic_index,
            AlignedByteOffset: self.aligned_byte_offset,
            InputSlot: input_slot,
            Format: self.format,
            // 每顶点一次（不是每实例）。用错它会让"每实例"的数据被重复读 ——
            // 症状是所有图元长得一模一样，且不报错。
            InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,
            InstanceDataStepRate: 0,
        }
    }
}

/// PSO 需要的顶点布局：**持有者**与**描述符**必须在同一个结构体里。
///
/// # 为什么不能分成两个返回值
///
/// `descs` 里的 `SemanticName` 指向 `owners` 内部的堆分配。分成
/// `(Vec<InputElement>, Vec<D3D12_INPUT_ELEMENT_DESC>)` 两个返回值，编译器无法表达
/// 「后者依赖前者活着」，于是调用方可以把两个 Vec 随意分开存 —— 而这正是原来那个
/// 悬垂指针 bug 的形态。放进同一个结构体，正确用法的正确性就是类型检查的事。
///
/// 字段声明顺序不影响正确性（`Box` 的地址与结构体位置无关），但保持 `owners` 在前
/// 便于阅读。
pub struct InputLayout {
    /// 语义名字节的**唯一所有者**。构造完之后代码不再读它，但**必须留在结构体里** ——
    /// 删掉这个字段就会让 `descs` 里的裸指针悬垂，而症状只是
    /// `CreateGraphicsPipelineState` 回 `E_INVALIDARG`。
    ///
    /// `names_are_owned` 在测试里读它，那正是这个字段存在意义的可执行版本。
    #[allow(dead_code)]
    owners: Vec<InputElement>,
    descs: Vec<D3D12_INPUT_ELEMENT_DESC>,
}

impl InputLayout {
    /// 交给 `D3D12_INPUT_LAYOUT_DESC` 的那一份。
    pub fn descs(&self) -> &[D3D12_INPUT_ELEMENT_DESC] {
        &self.descs
    }

    /// 每个 `SemanticName` 指针都必须落在某个 [`InputElement::name`] 内部。
    ///
    /// 这条不是风格检查 —— 它是**悬垂指针**那个 bug 的直接判据。`SemanticName` 是裸
    /// 指针，D3D12 只在 `CreateGraphicsPipelineState` 那一刻读它。早先的实现把
    /// `InputElement` 建成 `map` 闭包里的临时值，7 个指针全部指向已经释放的栈内存，
    /// 而症状只是 `E_INVALIDARG`。
    #[cfg(test)]
    fn names_are_owned(&self) -> Result<(), usize> {
        for (i, d) in self.descs.iter().enumerate() {
            let p = d.SemanticName.0 as usize;
            let ok = self
                .owners
                .iter()
                .any(|o| {
                    let base = o.name.as_ptr() as usize;
                    p >= base && p < base + o.name.len()
                });
            if !ok {
                return Err(i);
            }
        }
        Ok(())
    }
}

/// 建 PSO 需要的顶点元素描述。
///
/// 全部元素都在 slot 0 —— 与 `IASetVertexBuffers(0, 1, ...)` 对应。用单一 slot 而不是
/// 一个元素一个槽，是因为所有属性都来自同一个交错顶点。
///
/// # 格式**必须**逐元素给，不能一律 `R32G32_FLOAT`
///
/// 顶点里的属性宽度是混的：`color` 是 `float4`、`kind` 是 `float`，其余是 `float2`。
/// 一律声明成 `R32G32_FLOAT` 会让 `CreateGraphicsPipelineState` 回 `E_INVALIDARG` ——
/// 它把「描述的格式」与「着色器输入签名里的格式」逐项对比，对不上就拒绝，而且
/// 不告诉你是第几个。
///
/// 格式来自 [`VERTEX_LAYOUT`] 的第三列，与 D3D11 那边同一张表 —— 这样两个后端的
/// 顶点布局只有**一处**定义可写。
pub fn input_elements() -> InputLayout {
    let owners: Vec<InputElement> = VERTEX_LAYOUT
        .iter()
        .map(|&(n, i, _fmt, off)| InputElement::new(n, i, off, _fmt))
        .collect();
    let descs = owners.iter().map(|o| o.as_desc(0)).collect();
    InputLayout { owners, descs }
}

/// 校验顶点布局不越过 stride。
///
/// 越界在 D3D12 里是**静默的**（不像 D3D11 那样回 `E_INVALIDARG`），所以必须在这里
/// 挡住。
///
/// # 末元素宽度是**问出来的**，不是写死的
///
/// 这里曾经写着 `const LAST_ELEMENT_BYTES: u32 = 8; // float2` —— 假设最后一项是
/// `float2`。而最后一项其实是 `kind`，类型是 `f32`，宽度 4。用 8 去算会得出
/// 「覆盖到 56 字节、超过 stride 52」的假阳性。
///
/// 写成 `size_of::<f32>()` 只是把同一个错误挪了个位置：一旦 `Vertex` 末尾再加一个字段，
/// 这里的常数又会悄悄过期。**问编译器**（`offset_of` + `size_of`）才没有这个余地。
///
/// 这类错误之所以值得记一笔：它**不是**失败得太晚，而是**失败得太早** —— 一个永远报
/// 错的校验函数会让人去怀疑 `Vertex` 的定义，而真正的错在一个 `const` 里。
pub fn validate_layout() -> Result<(), ShaderError> {
    // 末元素 = `kind`（f32）。`offset_of` 保证我们问的是**真实**布局。
    let covered = (std::mem::offset_of!(Vertex, kind) + std::mem::size_of::<f32>()) as u32;
    let stride = std::mem::size_of::<Vertex>() as u32;
    if covered > stride {
        return Err(ShaderError::LayoutOutOfRange {
            covered,
            stride,
        });
    }
    Ok(())
}

/// 着色器与管线相关的错误。
#[derive(Debug)]
pub enum ShaderError {
    /// 顶点布局的最后一个元素越过了 stride。
    LayoutOutOfRange { covered: u32, stride: u32 },
    /// `Create*` 调用失败。
    ///
    /// 带的是 [`crate::surface::SurfaceError`] 而不是裸 `windows::core::Error` ——
    /// 因为 D3D12 的失败点分散在「哪个对象」与「哪一步」上，只有两者一起才能指到
    /// 调用点。裸 HRESULT 只会给出 `E_INVALIDARG` 这种没有指向性的消息。
    Create(&'static str, crate::surface::SurfaceError),
}

impl std::fmt::Display for ShaderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShaderError::LayoutOutOfRange { covered, stride } => write!(
                f,
                "顶点布局越界：末元素覆盖到 {covered} 字节，超过 stride {stride}"
            ),
            ShaderError::Create(what, e) => write!(f, "{what} 失败: {e}"),
        }
    }
}

impl std::error::Error for ShaderError {}

/// 编译期就绪的着色器构件。
///
/// 只持有字节码与描述表；真正创建 PSO 的地方在 `renderer`，因为它还需要根签名。
pub struct Shaders {
    pub elements: InputLayout,
}

impl Shaders {
    /// 准备着色器构件。
    pub fn new() -> Result<Self, ShaderError> {
        validate_layout()?;
        Ok(Self {
            elements: input_elements(),
        })
    }
}

#[cfg(test)]
#[path = "tests/shader_tests.rs"]
mod tests;

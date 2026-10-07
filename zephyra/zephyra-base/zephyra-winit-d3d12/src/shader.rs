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

/// 像素着色器字节码（`fxc /T ps_5_1`）。
///
/// # 为什么是 `ps_5_1` 而不是 `ps_5_0`
///
/// D3D12 的目标特性集是着色器模型 **5.1**（Skia 的 D3D12 后端也用 `vs_5_1`）。5.1 比
/// 5.0 多出的东西（本项目用不上：`SampleShading`、`ViewInstancing`、`Int64` 运算）
/// 无关紧要，要紧的是 5.1 与 D3D12 的根签名/描述符模型是配套的。
///
/// D3D11 接受 5.1 字节码（`FeatureLevel 11_0` 起就支持），所以 `zephyra-winit` 那边
/// 也用 5.1 —— **两个后端共用同一份字节码**，于是顶点布局也只有一处定义。
pub const PS_BYTECODE: &[u8] = include_bytes!("../shaders/zephyra.ps.bin");

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
mod tests {
    use super::*;
    use crate::quad::kind;

    /// 两个字节码都必须是真 DXBC。
    ///
    /// 这条断言钉住「`.bin` 被提交进来了」——漏提交时 `include_bytes!` 在编译期就失败。
    #[test]
    fn both_bytecode_blobs_are_dxbc() {
        for (name, b) in [("vs", VS_BYTECODE), ("ps", PS_BYTECODE)] {
            assert!(b.len() >= 32, "{name} 太短（{} B），大概没生成", b.len());
            assert_eq!(&b[0..4], b"DXBC", "{name} 魔数不对");
            let total = u32::from_le_bytes([b[24], b[25], b[26], b[27]]) as usize;
            assert_eq!(total, b.len(), "{name} 头里的长度与实际字节数不符");
        }
    }

    /// 布局不得越过 stride。
    #[test]
    fn layout_fits_inside_one_vertex() {
        validate_layout().expect("顶点布局不该越界");
    }

    /// 位置的语义必须是 `TEXCOORD4` 而不是 `POSITION`。
    ///
    /// 这条断言是**防呆**：位置语义改回 `POSITION` 时，症状是每个四边形只剩
    /// 1 个像素、零错误 —— 极难定位。这里失败会立刻指出原因。
    /// 输入元素的语义名指针必须全部落在 `InputLayout` 自己持有的那块内存里。
    ///
    /// 这条钉住那个**悬垂指针** bug：`SemanticName` 是裸指针，早先的实现让它指向
    /// `map` 闭包里的临时 `InputElement`，闭包一返回就失效。症状只是
    /// `CreateGraphicsPipelineState` 回 `E_INVALIDARG`，与「描述符字段填错」无法区分。
    #[test]
    fn 语义名指针_必须落在自己持有的内存里() {
        let layout = input_elements();
        if let Err(i) = layout.names_are_owned() {
            panic!(
                "第 {i} 个输入元素的 SemanticName 不指向 InputLayout 内部的任何一块 —— \
                 它是悬垂指针。检查 InputElement::new 的结果有没有被当成临时值丢掉。"
            );
        }
    }

    /// 每个输入元素的格式必须与它在 `VERTEX_LAYOUT` 里声明的一致，且通道数正确。
    ///
    /// 一律写 `R32G32_FLOAT` 会让 `CreateGraphicsPipelineState` 回 `E_INVALIDARG`
    /// （它逐项比对描述格式与着色器输入签名，且不告诉你是第几个）。`kind` 是 `float`、
    /// `color` 是 `float4`，写错必然被拒。
    #[test]
    fn 输入元素格式_必须与表里声明的一致() {
        let layout = input_elements();
        for (i, d) in layout.descs().iter().enumerate() {
            let (name, index, fmt, _) = VERTEX_LAYOUT[i];
            // 指针不比地址，只比「它解出来的字符串」。
            let p = d.SemanticName.0;
            let len = (0..12).find(|&k| unsafe { *p.add(k) } == 0).unwrap_or(0);
            let actual_name = unsafe { std::slice::from_raw_parts(p, len) };
            assert_eq!(
                (actual_name, d.SemanticIndex, d.Format),
                (name.as_bytes(), index, fmt),
                "第 {i} 项（{name}）的语义名/索引/格式与表不符 —— \
                 格式写错会让 PSO 静默地建不出来"
            );
        }
        // 关键的两项单独点名，因为它们正是被写错过的两个。
        let color = layout.descs()[3];
        assert_eq!(
            color.Format,
            DXGI_FORMAT_R32G32B32A32_FLOAT,
            "color 是 float4，不能声明成 float2"
        );
        let kind = layout.descs()[6];
        assert_eq!(
            kind.Format,
            DXGI_FORMAT_R32_FLOAT,
            "kind 是 float，不能声明成 float2"
        );
    }

    /// 语义索引区间必须**互不重叠** —— 本文件最该有的一条断言。
    ///
    /// # 为什么
    ///
    /// HLSL 里 `float2 x : TEXCOORDn` 占用语义索引 **n 与 n+1 两格**（分量 0 在 n、
    /// 分量 1 在 n+1），`float x : TEXCOORDn` 只占 n。所以两个属性重叠的判据是
    /// 「前者的 [n, n+分量数) 与后者的区间相交」。
    ///
    /// 本项目真的踩过：旧 D3D12 的 HLSL 写的是 `position:TEXCOORD4` 与
    /// `half_size:TEXCOORD5` —— 占 4,5 与 5,6，**重叠**。于是 `half_size.x` 读到的是
    /// `position.y`（或 0，取决于驱动），片元里 `p = local - half_size` 于是算错。
    /// 症状是「所有圆角矩形变成均匀半透明色块」—— 完全不像语义索引问题。
    ///
    /// # ⚠ 索引空间必须**按语义名分开**判，不能混在一起
    ///
    /// `TEXCOORD0` 与 `COLOR0` 是**两个独立的索引空间** —— 它们从不互相干扰，所以
    /// 拿一张表把所有项的索引区间放在一起比，会报出一堆**假重叠**（`COLOR 0..4`
    /// 与 `TEXCOORD 0..2` 区间相交，可它们本来就不共享空间）。
    ///
    /// 这是本条断言第一版就犯的错：它 naively 地把所有语义名混编，于是要么在正确
    /// 的布局上失败，要么为了让测试变绿而放宽判据 —— 后者更坏，因为它把唯一的
    /// 护栏改成了摆设。所以判据按 `(语义名, 索引)` 分组，**只在同一语义名内部**比。
    ///
    /// # 为什么旧断言没拦住这个缺陷
    ///
    /// `position_does_not_use_the_reserved_position_semantic` 只查"第 0 项不是
    /// POSITION"。那是**另一个**问题（`POSITION` 是 SM4+ 的系统值语义，顶点缓冲不
    /// 会喂给它），与"索引区间重叠"无关 —— 两条独立约束，却曾只有一条有断言。
    ///
    /// 那类"断言看起来相关、实际查的是别的东西"的缺口比没有断言更危险：它让人
    /// 以为这一层有防护。
    #[test]
    fn 语义索引区间互不重叠() {
        // 各格式占几格语义索引：`float2` 两格，`float4` 四格，`float` 一格。
        fn slots(fmt: DXGI_FORMAT) -> u32 {
            match fmt {
                DXGI_FORMAT_R32_FLOAT => 1,
                DXGI_FORMAT_R32G32_FLOAT => 2,
                DXGI_FORMAT_R32G32B32A32_FLOAT => 4,
                other => panic!("测试没覆盖这个格式 {other:?} —— 补上对应的格数再谈重叠"),
            }
        }
        // 只在同一语义名内部比较区间：`TEXCOORDn` 与 `COLORn` 是两套独立索引空间。
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
        assert_eq!(
            used.len(),
            VERTEX_LAYOUT.len(),
            "内部错误：区间表没覆盖全部输入元素"
        );
    }

    #[test]
    fn position_does_not_use_the_reserved_position_semantic() {
        let (name, index, _, _) = VERTEX_LAYOUT[0];
        // ⚠ 这里原来断言索引是 `4`，那是**旧布局**的值。它坏在一个真实缺陷上：
        // 旧 D3D12 的 HLSL 用 `position:TEXCOORD4 / half_size:TEXCOORD5`，而
        // `float2` 会占**两格**语义索引 —— 4、5 与 5、6 重叠，于是 `half_size`
        // 的第一个 float 读到 0。症状是 SDF 恒等于常数、圆角矩形整块均匀半透明。
        //
        // 修复是把两份 HLSL 统一成逐字节相同，并把语义索引改成互不重叠的
        // `0/2/4/6/8/10`（见 `VERTEX_LAYOUT` 的完整说明）。
        //
        // 而这条断言当时**没能发现那个缺陷**：它只查"第 0 项不是 POSITION"，
        // 对语义索引是否**互相重叠**一无所知。真正能拦住它的是
        // `语义索引区间互不重叠`。所以这里只保留"名字不是 POSITION"这半条 ——
        // 另半条由那条测试负责。
        assert_eq!(
            name, "TEXCOORD",
            "顶点位置不能用 POSITION —— SM4+ 里它是保留的系统值语义，\
             顶点缓冲不会被喂给它，位置会恒为 (0,0)"
        );
        assert_eq!(
            index, 0,
            "第 0 项的语义索引应为 0 —— 索引由上面 `语义索引区间互不重叠` 一起约束，\
             硬编一个具体数字只会在改布局时变成一处假绿"
        );
    }

    /// `kind` 的数值必须与着色器阈值配对。
    ///
    /// # 为什么用 `const` 块而不是普通 `assert!`
    ///
    /// 两边都是常量，所以编译器能在编译期判完 —— 普通 `assert!` 也会被 lint 挑出来
    /// （`assertions_on_constants`）。放进 `const` 断言里表达的是同一件事，但说的是
    /// **本意**：这不是"运行时检查"，这是"两处定义必须一致"这条**不变量**。
    ///
    /// 换阈值时若只改了 `zephyra.hlsl` 或只改了 `quad::kind`，编译就断 ——
    /// 而不是等到画面上「图标变成纯色块」才发现。
    #[test]
    fn kind_values_land_in_disjoint_shader_branches() {
        use crate::quad::kind_thresholds::{GLYPH_MAX, GLYPH_MIN, IMAGE_MIN};
        const {
            assert!(kind::RECT < GLYPH_MIN, "kind::RECT 落进了字形分支");
            assert!(
                kind::GLYPH > GLYPH_MIN && kind::GLYPH < GLYPH_MAX,
                "kind::GLYPH 落进了非字形分支"
            );
            assert!(kind::IMAGE > IMAGE_MIN, "kind::IMAGE 落进了字形分支");
        }
    }

    /// 顶点布局的偏移必须与 `Vertex` 的**真实字段偏移**逐项对上。
    ///
    /// # 为什么必须用 `offset_of` 而不是"累加字段大小"
    ///
    /// 累加只有在「字段顺序与声明顺序一致」时才成立。一旦有人在 `Vertex` 中间插了
    /// 字段（或改了 `repr`），累加出来的数会**与真实布局不符**，而断言**照样通过** ——
    /// 它拿自己的假设去验自己的假设。这是恒真式的一种变种。
    ///
    /// `offset_of` 直接问编译器"这个字段真的在第几字节"，验的是**事实**。
    ///
    /// 没有这条，`kind` 被读成垃圾值的症状（图标变纯色块 / 字变方块）不会有任何
    /// 编译期信号。
    #[test]
    fn vertex_field_offsets_match_the_layout() {
        use std::mem::offset_of;

        let actual: [(&str, usize); 7] = [
            ("position", offset_of!(Vertex, position)),
            ("local", offset_of!(Vertex, local)),
            ("uv", offset_of!(Vertex, uv)),
            ("color", offset_of!(Vertex, color)),
            ("corner", offset_of!(Vertex, corner)),
            ("half_size", offset_of!(Vertex, half_size)),
            ("kind", offset_of!(Vertex, kind)),
        ];

        for (i, (name, off)) in actual.iter().enumerate() {
            assert_eq!(
                VERTEX_LAYOUT[i].3 as usize, *off,
                "顶点布局第 {i} 项（{name}）应位于偏移 {off} 字节，实际声明为 {} —— \
                 两者不一致时着色器会读到错位的值，且**没有任何错误**",
                VERTEX_LAYOUT[i].3
            );
        }

        let last_end = offset_of!(Vertex, kind) + std::mem::size_of::<f32>();
        assert!(
            last_end <= std::mem::size_of::<Vertex>(),
            "最后一个元素覆盖到 {last_end} 字节，超过 stride {}",
            std::mem::size_of::<Vertex>()
        );
    }

    /// 相邻字段之间**不得**有空洞。
    ///
    /// # 为什么这条是上一条的**前提**而不是细节
    ///
    /// `offset_of` 读的是真实偏移，所以即使布局被重排，它也不会自己发现「布局表按声明
    /// 顺序写，而实际不是」这件事 —— 那种情况下上一条会失败，但失败信息说的是
    /// 「某一项对不上」，看不出根因是重排。
    ///
    /// 这里直接把「`a` 紧跟在 `b` 后面」钉住：字段宽度全是 4 字节的 `float2`/`float4`，
    /// 所以相邻偏移差**必须**等于前者的宽度。多出来的空洞说明有人插了字段或改了类型。
    #[test]
    fn vertex_has_no_padding_between_fields() {
        use std::mem::{offset_of, size_of};
        // (前字段偏移, 后字段偏移, 前字段名, 前字段宽度)
        let gaps: [(usize, usize, &str, usize); 6] = [
            (
                offset_of!(Vertex, position),
                offset_of!(Vertex, local),
                "position",
                size_of::<[f32; 2]>(),
            ),
            (
                offset_of!(Vertex, local),
                offset_of!(Vertex, uv),
                "local",
                size_of::<[f32; 2]>(),
            ),
            (
                offset_of!(Vertex, uv),
                offset_of!(Vertex, color),
                "uv",
                size_of::<[f32; 2]>(),
            ),
            (
                offset_of!(Vertex, color),
                offset_of!(Vertex, corner),
                "color",
                size_of::<[f32; 4]>(),
            ),
            (
                offset_of!(Vertex, corner),
                offset_of!(Vertex, half_size),
                "corner",
                size_of::<[f32; 2]>(),
            ),
            (
                offset_of!(Vertex, half_size),
                offset_of!(Vertex, kind),
                "half_size",
                size_of::<[f32; 2]>(),
            ),
        ];
        for (a, b, name, width) in gaps {
            assert_eq!(
                b - a,
                width,
                "{name} 与下一个字段之间夹了 {} 字节，期望正好 {width} —— \
                 多出来的空洞说明布局被重排（不是 repr(C)）或字段类型改了",
                b - a
            );
        }
    }
}

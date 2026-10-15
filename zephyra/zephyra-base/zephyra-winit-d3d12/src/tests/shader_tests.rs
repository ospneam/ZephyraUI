// 本文件由 shader.rs 的 #[cfg(test)] mod tests 抽出 —— 测试代码与生产代码分离。
// 模块路径不变（src 里的声明用 #[path] 指过来），所以 use super::* 与私有可见性都照旧。

use super::*;
use crate::quad::kind;

/// 全部字节码都必须是真 DXBC。
///
/// 这条断言钉住「`.bin` 被提交进来了」——漏提交时 `include_bytes!` 在编译期就失败。
///
/// # 为什么遍历**全部**变体而不是只看一个
///
/// `.bin` 是离线产物，不参与编译期检查：漏提交 18 个变体里的任何一个，编译都
/// 照样成功，症状只会在运行时表现为"某个模式画不出来"。逐个查魔数与长度
/// 是唯一能把这种漏网在测试里拦下来的地方。
#[test]
fn all_bytecode_blobs_are_dxbc() {
    for (name, b) in PS_VARIANTS {
        assert!(b.len() >= 32, "{name} 太短（{} B），大概没生成", b.len());
        assert_eq!(&b[0..4], b"DXBC", "{name} 魔数不对");
        let total = u32::from_le_bytes([b[24], b[25], b[26], b[27]]) as usize;
        assert_eq!(total, b.len(), "{name} 头里的长度与实际字节数不符");
    }
    let b = VS_BYTECODE;
    assert!(b.len() >= 32, "vs 太短（{} B），大概没生成", b.len());
    assert_eq!(&b[0..4], b"DXBC", "vs 魔数不对");
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
fn semantic_name_pointer_lies_in_own_memory() {
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
fn input_element_layout_matches_table() {
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
        color.Format, DXGI_FORMAT_R32G32B32A32_FLOAT,
        "color 是 float4，不能声明成 float2"
    );
    let kind = layout.descs()[6];
    assert_eq!(
        kind.Format, DXGI_FORMAT_R32_FLOAT,
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
fn semantic_index_ranges_do_not_overlap() {
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
    // `semantic_index_ranges_do_not_overlap`。所以这里只保留"名字不是 POSITION"这半条 ——
    // 另半条由那条测试负责。
    assert_eq!(
        name, "TEXCOORD",
        "顶点位置不能用 POSITION —— SM4+ 里它是保留的系统值语义，\
             顶点缓冲不会被喂给它，位置会恒为 (0,0)"
    );
    assert_eq!(
        index, 0,
        "第 0 项的语义索引应为 0 —— 索引由上面 `semantic_index_ranges_do_not_overlap` 一起约束，\
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

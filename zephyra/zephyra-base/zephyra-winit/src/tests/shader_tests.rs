// 本文件由 shader.rs 的 #[cfg(test)] mod tests 抽出 —— 测试代码与生产代码分离。
// 模块路径不变（src 里的声明用 #[path] 指过来），所以 use super::* 与私有可见性都照旧。

    use super::*;

    /// 字节码必须是真 DXBC —— 否则运行期才炸，且症状是"窗口空白"。
    ///
    /// 这条断言钉住"`.bin` 文件被提交进来了"这件事：漏提交时 `include_bytes!`
    /// 在编译期就失败，而格式写错只会在运行期变成一片空白。
    #[test]
    fn both_bytecode_blobs_are_dxbc() {
        for (name, b) in [("vs", VS_BYTECODE), ("ps", PS_VARIANTS[0].1)] {
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

// 本文件由 renderer.rs 的 #[cfg(test)] mod tests 抽出 —— 测试代码与生产代码分离。
// 模块路径不变（src 里的声明用 #[path] 指过来），所以 use super::* 与私有可见性都照旧。

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
    fn buffer_layout_must_be_row_major_not_unknown() {
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
    fn render_target_texture_must_allow_render_target() {
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
    fn pipeline_state_node_mask_is_zero_for_single_gpu() {
        // 断言的是**生产代码用的那个常量**（`create_pso` 里引用的就是它）。
        assert_eq!(
            PSO_NODE_MASK, 0,
            "文档原文：单 GPU 时 PSO 的 NodeMask 就是 0，驱动会转成掩码 1。\
             「必须设一位」那条规则只适用于单节点 API 清单（命令队列/命令列表/\
             描述符堆/查询堆），PSO 不在其中。"
        );
    }

    /// 回归：`upload_end` 必须**含对齐**那一项。
    ///
    /// # 这个 bug 的形状（值得原样保留在测试里）
    ///
    /// 索引流缓冲 `next_power_of_two` 之后是 8192；主路径（`upload_geometry`）
    /// 把 `head` 推到 8000；随后离屏 pass（`draw_pass_quad`）要传一枚四边形的
    /// 索引 —— `expand_indices_with_paths(1, 0)` 是 6 个 `u16` = **12 字节**。
    ///
    /// 旧实现里 `reserve` 判 `head + extra <= size`：`8012 <= 8192` **放行**；
    /// 而 `record_upload` 把起点对齐到 **8192** 再放 12 字节 → 越界，整帧被丢。
    ///
    /// 判据直接写成那个形状：`head = 8000` 时，12 字节**必须放不下**。
    /// 若哪天有人把对齐项从 `upload_end` 里去掉，这条立刻红。
    #[test]
    fn upload_end_offset_accounts_for_256_byte_alignment() {
        // 对齐把起点从 8000 推到 8192，于是 12 字节放不下。
        assert_eq!(
            upload_end(8000, 12),
            8192 + 12,
            "起点应当被推到 8192（8000 的下一个 256 倍数）"
        );
        assert!(
            upload_end(8000, 12) > 8192,
            "回归：旧实现算成 8012 <= 8192 于是放行，随后 record_upload 越界"
        );
        // 对齐前就在边界上时，判据与实际一致（不该多要一整个对齐块）。
        assert_eq!(upload_end(8192, 12), 8204);
        assert_eq!(upload_end(0, 12), 12);
    }

    /// `upload_end` 对 `head` 单调不减 —— `reserve` 靠它判断"再放一段会不会超"，
    /// 若出现非单调就会出现"更大的 head 反而放得下"的荒谬情形。
    #[test]
    fn upload_end_offset_is_monotonic_in_head() {
        let mut prev = 0u64;
        for head in (0..40_000u64).step_by(37) {
            let cur = upload_end(head, 64);
            assert!(
                cur >= prev,
                "head={head}: upload_end={cur} 小于上一个 {prev}"
            );
            prev = cur;
        }
    }

    /// `head` 大到接近 `u64::MAX` 时，`reserve` 必须**永远判为要扩容**。
    ///
    /// # 为什么这是饱和的正确判据（而不是「end > head」）
    ///
    /// 先排除一个**看起来**该守、实际守不住的属性：`off < head` 在这种输入下
    /// 确实可能出现（`u64::MAX` 被掩掉低位后反而变小）。但它是**不可达**的 ——
    /// 此时 `want` 也必然是一个天文数字，`reserve` 必然扩容、`head` 归零，
    /// `record_upload` 永远不会拿着这个 `head` 跑。
    ///
    /// 真正要防的是**回绕**：若 `head + ALIGN - 1` 溢出后回绕成小数，
    /// `want` 会突然变小，于是 `want <= size` 成立、扩容被跳过，
    /// `record_upload` 从老远的老位置写 —— **覆盖同帧已录制的其它段**。
    /// 那不是看得见的报错，是静默的内存损坏。
    ///
    /// 所以判据取真实后果：荒谬 `head` 的 `want` 必须大到不可能被当成「放得下」。
    #[test]
    fn upload_end_offset_rejects_absurd_head() {
        // 1GB 远超任何暂存区（初始 64KB / 4KB，扩容上限也就几 MB）。
        const IMPOSSIBLE: u64 = 1 << 30;
        for head in [
            u64::MAX - 8,
            u64::MAX - 256,
            u64::MAX - 255,
            u64::MAX - 1,
            u64::MAX,
        ] {
            let want = upload_end(head, 64);
            assert!(
                want > IMPOSSIBLE,
                "head={head} 算出 want={want}，小到会被当成「放得下」—— 溢出回绕了"
            );
            // 且不能 panic（本测试本身跑过就说明这一步）。
        }
    }

    /// 现实域内 `upload_end` 必须**精确等于**「对齐后的起点 + 字节数」。
    ///
    /// 上一条管「不可达的荒谬输入」，这一条管「真会跑到的输入」：
    /// 缓冲容量最大不过几十 MB，`head` 不可能离开。[`UPLOAD_ALIGN`] 的对齐项
    /// 在这个域里必须精确，不能有半点漂移。
    #[test]
    fn upload_end_offset_is_exact_on_realistic_inputs() {
        let align = |v: u64| (v + UPLOAD_ALIGN - 1) & !(UPLOAD_ALIGN - 1);
        for &head in &[0u64, 1, 255, 256, 257, 7919, 8000, 8180, 8192, 8193, 65_536] {
            for &bytes in &[0u64, 1, 12, 64, 300, 4096] {
                assert_eq!(
                    upload_end(head, bytes),
                    align(head) + bytes,
                    "head={head} bytes={bytes}"
                );
            }
        }
    }

    /// 起点必须**真的对齐**，且 `head` 未对齐时起点必须严格后移。
    ///
    /// 这一条是 bug 的另一半：对齐项存在的意义就是把起点推后。
    /// 若哪天有人把 `& !(ALIGN-1)` 去掉，「12 字节放不进 8192 边界上」的回归
    /// 会以另一种形式回来。
    #[test]
    fn upload_start_offset_is_aligned() {
        for head in 0..3000u64 {
            let (off, _) = upload_span(head, 0);
            assert_eq!(off % UPLOAD_ALIGN, 0, "head={head} 的起点 {off} 没对齐");
            assert!(off >= head, "head={head} 的起点 {off} 跑到 head 前面了");
        }
    }

    /// `end - off` 必须**正好**等于请求的字节数。
    ///
    /// 原实现里 `off` 与 `end` 是两条算式（`off` 单独算、`end = off + n`）。
    /// 一旦有人改了其中一条而漏了另一条，`end - off != n` 会立刻在这里现形 ——
    /// 而它的实际后果是 memcpy 与游标推进量不一致，也就是"拷了 N 字节却只推进 M"，
    /// 下一段会重叠覆盖上一段。那类损坏**没有报错**，只是画面随机错。
    #[test]
    fn upload_span_length_is_conserved() {
        for head in 0..4000u64 {
            for bytes in [0u64, 1, 12, 64, 300, 4096, u64::MAX / 2, u64::MAX] {
                let (off, end) = upload_span(head, bytes);
                assert_eq!(
                    end - off,
                    bytes.min(u64::MAX - off),
                    "head={head} bytes={bytes}：区间长度不等于请求长度"
                );
            }
        }
    }

// ZephyraUI 的单一渲染着色器（SDF 圆角矩形 + 字形图集 + 图像采样）。
//
// 与 quad.rs 是严格对偶的两半：那里的 Vertex 字段顺序 / 偏移 / kind 阈值，
// 与这里的 VSIn 语义与 PSMain 分支必须逐项对应。
Texture2D<float4> g_tex : register(t0);
SamplerState      g_smp : register(s0);

struct VSIn {
    // 顶点位置刻意**不用** `POSITION`。
    //
    // 从 SM4 起 `POSITION` 在顶点着色器**输入**上是保留名：`fxc` 把它编成系统值
    // 语义（ISGN 里 `SystemValueType = 3` = `D3D_SV_POSITION`），于是 D3D11 认为这个
    // 输入由流水线自己提供，**不会**从顶点缓冲里取。
    //
    // 症状安静到极点：`CreateInputLayout` 成功、日志零错误，而每个顶点的位置都是
    // (0,0)，所有三角形退化到 NDC 原点，每个四边形只剩 1 个像素。
    //
    // ⚠⚠ 语义索引**必须互不重叠** —— 这是本文件最容易踩的一类错，且**与上面那条
    // 症状长得完全不一样**，所以更容易被误诊。
    //
    // HLSL 里 `float2 x : TEXCOORDn` 会**占掉 TEXCOORDn 与 TEXCOORD(n+1) 两格**。
    // 下面这份分配曾经写成这样 —— 逐项看索引都不同，但**按「float2 占两格」一算就
    // 大量重叠**（❌ 已修，留在这里当反例）：
    //
    // ```text
    // position  : TEXCOORD4 → 占 4,5     half_size : TEXCOORD5 → 占 5,6  ← 重叠
    // local     : TEXCOORD0 → 占 0,1     uv        : TEXCOORD1 → 占 1,2  ← 重叠
    // corner    : TEXCOORD2 → 占 2,3     kind      : TEXCOORD3 → 占 3    ← 重叠
    // ```
    //
    // `fxc` **不报错**，ISGN 里也如实记着每项自己的索引，所以
    // `the_shader_signature_matches_our_vertex_layout` 照样通过。但 D3D11 的
    // `CreateInputLayout` 是按**区间**绑定的，只有不冲突的那部分生效。实测症状
    // （`tests/minimal_draw.rs::逐个属性探测`）：
    //
    // - 几何**完全正确** —— 三角形铺满视口 4096/4096；
    // - 而每个属性的**第一个 float 都读到 0**，第二个分量正常；
    // - 于是 SDF 的 `local.x` 恒 0 → 每行只有一个 x 位置满足 `d ≈ 0` → 画面上是
    //   一条约 300 像素的线，**而不是空白**。
    //
    // 「不是空白」是最误导的地方：它看起来像「画了一点点」，于是排查被带到视口、
    // 数据、抗锯齿上去 —— 而这三样全都正常。
    //
    // ✅ **现行分配**（下方声明）：按「float2 占两格、float 占一格」**递增分配**，
    // 任何两项都不重叠。
    // ⚠ 改这里必须同步改 `shader.rs` 的 `VERTEX_LAYOUT`，并用 `fxc` 重新生成
    // `.bin` —— `include_bytes!` 不会因为源文件变化而重新编译。
    float2 position  : TEXCOORD0;   // 占 0,1
    float2 local     : TEXCOORD2;   // 占 2,3
    float2 uv        : TEXCOORD4;   // 占 4,5
    float4 color     : COLOR0;
    float2 corner    : TEXCOORD6;   // 占 6,7  x=圆角半径 y=描边宽度
    // 四边形的半宽半高。四个顶点填**同一个**值，插值后仍是它 —— 这是把一个
    // 「整个四边形的常量」送进片元着色器的标准做法。
    //
    // ⚠ 不能在片元里用 `local * 0.5` 之类推出来：那会让 `center` 恒等于 `half_size`，
    // 于是 SDF 退化成与位置无关的常数（方角块均匀半透明、圆角块直接消失）。
    // 完整推导见 `crates/base/zephyra-winit/src/quad.rs` 的 `Vertex::half_size`。
    float2 half_size : TEXCOORD8;   // 占 8,9
    float  kind      : TEXCOORD10;  // 占 10
};

struct VSOut {
    float4 clip   : SV_Position;
    float2 local  : TEXCOORD0;
    float2 uv     : TEXCOORD1;
    float4 color  : COLOR0;
    float2 corner : TEXCOORD2;
    // 整个四边形要么全是一类要么全不是：逐片元插值会在四边形内部产生中间值，
    // 导致部分像素走错分支（"图标变成纯色块" / "字变成方块"）。
    nointerpolation float kind : TEXCOORD3;
    // `half_size` 在片元里必须**精确**，不能是插值出来的近似值 —— 它要参与
    // `abs(p) - half_size` 这种相减。所以标成 `nointerpolation`。
    //
    // 实际上四个顶点填的是同一个值，插值结果本来就精确；标 `nointerpolation` 是为了
    // 让这件事在代码里显式，且避免未来有人改动顶点值时悄悄失去这个性质。
    nointerpolation float2 half_size : TEXCOORD4;
};

VSOut VSMain(VSIn i) {
    VSOut o;
    o.clip      = float4(i.position, 0.0, 1.0);
    o.local     = i.local;
    o.uv        = i.uv;
    o.color     = i.color;
    o.corner    = i.corner;
    o.kind      = i.kind;
    o.half_size = i.half_size;
    return o;
}

// 圆角矩形的有符号距离：矩形内为负，外部为正。
float sd_round_rect(float2 p, float2 half_size, float radius) {
    float r = min(radius, min(half_size.x, half_size.y));
    float2 q = abs(p) - half_size + float2(r, r);
    return length(max(q, float2(0.0, 0.0))) + min(max(q.x, q.y), 0.0) - r;
}

float4 PSMain(VSOut i) : SV_TARGET {
    // kind == 3：已 CPU 三角化的多边形。**不采样纹理**，直接返回顶点色。
    //
    // # 必须排在图像分支之前
    //
    // 图像分支的判据是 `kind > 1.5`，而 3 > 1.5 —— 若多边形分支排在其后，
    // 每个三角形都会去`g_tex.Sample()` 当前批次绑着的那张纹理。症状是多边形
    // 变成"图集/图像的内容"而非路径形状，且**换批次就换颜色**，看起来像随机
    // 噪声而不是错误 —— 这类症状极难回溯到`kind` 配错上。
    //
    // 阈值用 2.5 而不是 3.0：三个`kind` 靠"相邻且间距 < 1"区分，阈值取相邻
    // 两类的中点。写 3.0 的话 `> 3.0` 会把恰好等于 3.0 的多边形漏进图像分支。
    //
    // 抗锯齿：这里没有 `smoothstep`。三角化后的多边形没有解析距离场，
    // 覆盖率只能来自 MSAA 的多点采样 —— 见 `crates/zephyra/src/geom/mod.rs`
    // 的「抗锯齿不在这里」。
    if (i.kind > 2.5) {
        clip(i.color.a - 0.0039);
        return i.color;
    }

    // kind == 1：字形。采样图集，用 alpha 当覆盖率。
    if (i.kind > 0.5 && i.kind < 1.5) {
        float4 texel = g_tex.Sample(g_smp, i.uv);
        // 图集是 R8 展开成 RGBA，覆盖率在 alpha。字体位图自带抗锯齿，这里**不再**
        // 叠 SDF 的边缘过渡，否则字会被啃一圈。
        float a = texel.a * i.color.a;
        clip(a - 0.0039);
        // 预乘空间：`i.color.rgb` 在顶点展开时已乘过 `i.color.a`（见 `quad.rs`），
        // 这里再乘一次覆盖率 `texel.a`。漏掉这一步 rgb 与 a 不同步 —— 半透明笔画
        // 会显得比它该有的更亮（叠加时目标被压暗得更少）。
        return float4(i.color.rgb * texel.a, a);
    }

    // kind == 2：图像。采样当前绑定的纹理（每批一种）。
    if (i.kind > 1.5) {
        float4 texel = g_tex.Sample(g_smp, i.uv);
        float a = texel.a * i.color.a;
        // tint 是**乘算**：图标要染色时用它，而不是改纹理内容。
        //
        // ⚠️ 圆角遮罩**必须**以 `corner.x > 0` 为条件，不能无条件施加。
        //
        // 圆角矩形的 SDF 在 `d == 0` 处正是矩形边界，于是
        // `1 - smoothstep(-px, px, 0) == 0.5` —— 无条件施加会把最外一圈像素压到
        // 50% alpha。症状是**图像四周发暗一圈**，而所有计数器正常。
        //
        // 直角图像四边形**没有**外扩（`quad.rs` 的 `pad` 对 `corner.x == 0` 取 0），
        // 所以那半边过渡带本来就不可见 —— 也就是说即使施加了也不会「更抗锯齿」，
        // 只会让边缘变暗。这条分支不是优化，是正确性。
        //
        // 带圆角时 `quad.rs` 已按 `AA_BAND` 外扩，并用 **uv 外插**保证图像内容
        // 不被裁掉；采样器的 Clamp 兜住外插出去的那一点点。
        if (i.corner.x > 0.0) {
            float2 half_size = i.half_size;
            float2 p = i.local - half_size;
            float d = sd_round_rect(p, half_size, i.corner.x);
            float px_img = 0.75;
            a *= 1.0 - smoothstep(-px_img, px_img, d);
        }
        clip(a - 0.0039);
        // 预乘空间：采样到的图像纹理是**直通 alpha**（R8G8B8A8），所以 rgb 要先乘自己的
        // alpha；`i.color.rgb` 则已在顶点展开时乘过 tint 的 alpha（见 `quad.rs`）。
        return float4(texel.rgb * texel.a * i.color.rgb, a);
    }

    // kind == 0：SDF 圆角矩形。
    //
    // `p` 是「片元相对四边形中心的偏移」，`half_size` 是四边形的半宽半高 ——
    // **两者来自不同的顶点通道**，且都是常量/线性插值的正确量。
    float2 half_size = i.half_size;
    float2 p = i.local - half_size;
    float d = sd_round_rect(p, half_size, i.corner.x);

    // 1 像素过渡带：抗锯齿。带宽不随缩放变化，所以高 DPI 下边缘不会变毛糙。
    float px = 0.75;
    float alpha = 1.0 - smoothstep(-px, px, d);
    clip(alpha - 0.0039);

    if (i.corner.y > 0.0) {
        // 描边：距边界 stroke 宽度以内的环带。
        //
        // ⚠ 这里**不能**写成 `1.0 - smoothstep(...)`。`d < 0` 在内部、`d == 0` 在边界，
        // 环带对应 `d ∈ [-width, 0]`；而 `smoothstep(-width-px, -width+px, d)` 在
        // `d == 0`（边界）处取 1、在 `d == -width`（环带内侧）处取 0 —— 正好是我们
        // 要的。再取一次 `1.0 -` 会把整条环带**翻过来**：越靠里越实、越靠边越透明，
        // 描边看起来变成一块实心填充。
        float inner = -i.corner.y;
        float sa = smoothstep(inner - px, inner + px, d);

        // 外侧**必须**再乘一道淡出：`d > 0` 在环带之外，而 smoothstep 在那里饱和为 1。
        // 四边形按 px（AA_BAND）向外扩过，所以缺这一道会让描边外侧多出一圈实色 ——
        // 症状是「描边比设定值粗一圈，且那一圈没有抗锯齿」。
        //
        // 注意它**只**乘在描边分支上：填充分支的 `alpha` 本身已经含外侧淡出，
        // 再乘一次会让填充边缘被淡两次（更瘦）。
        float outerFade = 1.0 - smoothstep(-px, px, d);
        sa *= outerFade;

        clip(sa - 0.0039);
        return float4(i.color.rgb * sa, i.color.a * sa);
    }
    return float4(i.color.rgb * alpha, i.color.a * alpha);
}
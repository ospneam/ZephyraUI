// ZephyraUI 的单一渲染着色器（SDF 圆角矩形 + 字形图集 + 图像采样）。
//
// 与 quad.rs 是严格对偶的两半：那里的 Vertex 字段顺序 / 偏移 / kind 阈值，
// 与这里的 VSIn 语义与 PSMain 分支必须逐项对应。
Texture2D<float4> g_tex : register(t0);
SamplerState      g_smp : register(s0);

// **目标像素的副本**（t1）。可分离 / 非可分离混合模式必须读 `dst`，而 D3D11
// **不能采样当前绑定的 RTV** —— 所以宿主先把渲染目标 `CopyResource` 到另一张纹理，
// 再把它当 SRV 绑到这里。见 `renderer.rs` 的 `dst_copy`。
//
// 系数可表达的那 15 个模式（`Clear`..`Screen`）**不读**这里：它们由固定功能混合
// 方程完成，PS 只负责算出源色。见 `PSMain` 与 `MAKE_BLEND_PS` 的分工。
Texture2D<float4> g_dst : register(t1);

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
    // 原始 NDC 位置透传到片元。
    //
    // 用途只有一个：采样 `g_dst` 时需要**屏幕 uv**。`SV_Position` 给的是像素坐标，
    // 要换算成 uv 就得知道目标尺寸 —— 而那需要一个常量缓冲（本管线没有）。
    // NDC 已经在手里，`×0.5+0.5` 就是 uv（y 与 `to_ndc` 相反，见下）。
    //
    // ⚠ 只加在 **VSOut** 上，不动 `VSIn`：`VSIn` 的语义索引被 `CreateInputLayout`
    // 按区间绑定，加一项就要同步改 `shader.rs` 的 `VERTEX_LAYOUT`，且索引必须重排。
    // `VSOut` 的语义只是名字，`fxc` 会自己分配寄存器。
    float2 ndc : TEXCOORD5;
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
    o.ndc       = i.position;
    return o;
}

/// 目标像素的屏幕 uv（与 `quad::expand_into` 里 `to_ndc` 的 y 方向相反）。
///
/// NDC 的 y 向上、纹理 uv 的 y 向下，所以是 `0.5 - y*0.5` 而不是 `y*0.5 + 0.5`。
/// 写反的症状是"混合效果整体上下颠倒"，且只在 `dst` 与源不同时才看得出来。
float2 DstUv(VSOut i) {
    return float2(i.ndc.x * 0.5 + 0.5, 0.5 - i.ndc.y * 0.5);
}

// 圆角矩形的有符号距离：矩形内为负，外部为正。
float sd_round_rect(float2 p, float2 half_size, float radius) {
    float r = min(radius, min(half_size.x, half_size.y));
    float2 q = abs(p) - half_size + float2(r, r);
    return length(max(q, float2(0.0, 0.0))) + min(max(q.x, q.y), 0.0) - r;
}

/// 算出这一片元的**预乘**源色（不含任何混合）。
///
/// 抽出来是因为可分离 / 非可分离混合要与 `dst` 合成，而「算出源色」这部分
/// 与混合模式**完全无关**。共用一份才不会出现「某个混合模式下的圆角矩形与
/// 默认路径长得不一样」这类漂移。
float4 ShadeSource(VSOut i) {
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

/// 默认路径：源色交给**固定功能混合方程**（`SrcOver`，或 `runs` 段指定的系数对）。
///
/// 系数可表达的那 15 个模式都用这一个 PS —— 混合因子在 `ID3D11BlendState` /
/// PSO 的 `BlendState` 里。见 `renderer.rs` 的 `blends` 表。
float4 PSMain(VSOut i) : SV_TARGET {
    return ShadeSource(i);
}
// ═══════════════════════════════════════════════════════════════════════════
//  E4.2 / E4.3：可分离与���可分离混合模式
// ═══════════════════════════════════════════════════════════════════════════
//
// # 为什么必须用 PS 变体而不是统一一个 PS
//
// 系数可表达的那 15 个（`Clear`..`Screen`）是 `r = Cs·ks + Cd·kd` 的形式，D3D 的
// 固定功能混合方程**原样就能算**。往后的 14 个不是：
//
// - **可分离** 10 个（`Overlay`..`Multiply`）：`B = f(Cb, Cs)` 逐通道，但 `f` 不是
//   线性组合（`Overlay` 里有 `1−2·Cb` 这种分支），固定功能表达不了。
// - **非可分离** 4 个（`Hue` / `Saturation` / `Color` / `Luminosity`）：结果依赖
//   **对方的色相 / 饱和度 / 明度**，逐通道算不出来。
//
// # 为什么是"每个模式一个入口"而不是"一个入口 + 一个模式常量"
//
// 模式常量要有地方放：常量缓冲（要新建 `ID3D11Buffer` + `SetConstantBuffers`，
// D3D12 还要根签名里加项）或顶点数据（会把混���模式塞进与颜色无关的通道，
// 污染 `quad.rs` 的顶点契约）。而**离线编译**的变体零运行时开销、零新管线状态，
// 代价只是 `.bin` 变多 —— 那本来就是离线成本。
//
// 所以用宏生成 14 个入口：编译期展开，运行时按 `runs` 段的 `blend` 选 PS。
//
// # 合成公式（预乘空间）
//
// ```text
//   as = s.a                       源的 alpha
//   Cs = s.rgb / as                反预乘
//   Cd = d.rgb / d.a               反预乘
//   B  = blend(Cs, Cd)             混合函数的**非预乘**结果
//   out.rgb = (1−as)·Cd + as·B     ← 已经是预乘
//   out.a   = as + ad·(1−as)
// ```
//
// `as == 0` 时直接返回全 0：`(1−as)·Cd` 会把目标**擦掉**，而"什么都没画"绝不该
// 改变目标像素。
//
// # 与系数模式的等价性
//
// `SrcOver` 在这个公式下的结果是 `(1−as)·Cd + as·Cs = Cs + (1−as)·Cd`，与固定功能
// 的 `ONE/INV_SRC_ALPHA` **完全一致**。所以 15 个系数模式不需要走这里 —— 少一条
// 路径就少一处能漂移的地方。

float Lum(float3 c) { return dot(c, float3(0.3, 0.59, 0.11)); }

float3 ClipColor(float3 c) {
    // 非可分离模式的标准前置：把色域外的负分量拉到 0（保住明度关系）。
    float l = Lum(c);
    float n = min(c.r, min(c.g, c.b));
    float x = max(c.r, max(c.g, c.b));
    if (n < 0.0) { c = l + (c - l) * l / max(l - n, 1e-6); }
    if (x > 1.0) { c = l + (c - l) * (1.0 - l) / max(x - l, 1e-6); }
    return c;
}

float3 SetLum(float3 c, float l) { return ClipColor(c + (l - Lum(c))); }

// ⚠ 返回**标量**，不是 float3。
//
// 写成 `float3 Sat(float3)` 时 `fxc` 会给 X3206「implicit truncation of vector type」
// ——而它**能编译**、结果也对（标量被 splat 成 float3(x,x,x)），于是这个警告看起来
// 可以忽略。实际上后面 `SetSat(c, Sat(other))` 把它当标量用，签名在说谎。
float Sat(float3 c) { return max(c.r, max(c.g, c.b)) - min(c.r, min(c.g, c.b)); }

float3 SetSat(float3 c, float s) {
    // 找出各通道在 min/max 之间的相对位置，按新的饱和度重排。
    float mn = min(c.r, min(c.g, c.b));
    float mx = max(c.r, max(c.g, c.b));
    float d = mx - mn;
    if (d <= 0.0) { return float3(0, 0, 0); }
    return (c - mn) / d * s;
}

// ---- 可分离：逐通道 ----
float3 BOverlay(float3 b, float3 s) {
    return lerp(2.0 * b * s, 1.0 - 2.0 * (1.0 - b) * (1.0 - s), step(0.5, b));
}
float3 BDarken(float3 b, float3 s) { return min(b, s); }
float3 BLighten(float3 b, float3 s) { return max(b, s); }
float3 BColorDodge(float3 b, float3 s) {
    float3 z = step(0.0, b);
    float3 m = min(1.0, (1.0 - b) / max(s, 1e-6));
    // `lerp(float3(0.0), m, z)` 写成 `m * z`：`fxc` 拒绝把单标量 splat 当
    // `lerp` 的第一个实参（X3014），而这里的 else 分支恰好是 0，等价。
    return m * z;
}
float3 BColorBurn(float3 b, float3 s) {
    float3 z = step(b, 1.0);
    float3 m = min(1.0, 1.0 - (1.0 - b) / max(1.0 - s, 1e-6));
    return m * z + (1.0 - z);   // 同上：else 分支是 1。
}
float3 BHardLight(float3 b, float3 s) { return BOverlay(s, b); }
float3 BSoftLight(float3 b, float3 s) {
    float3 d = ((16.0 * b - 12.0) * b + 4.0) * b * step(b, 0.25);
    return sqrt(max(b, 0.0)) * step(0.5, b) * s + d * (1.0 - s);
}
float3 BDifference(float3 b, float3 s) { return abs(b - s); }
float3 BExclusion(float3 b, float3 s) { return b + s - 2.0 * b * s; }
float3 BMultiply(float3 b, float3 s) { return b * s; }

// ---- 非可分离：需要 ClipColor / SetLum / SetSat ----
float3 BHue(float3 b, float3 s) { return SetLum(SetSat(s, Sat(b)), Lum(b)); }
float3 BSaturation(float3 b, float3 s) { return SetLum(SetSat(b, Sat(s)), Lum(b)); }
float3 BColorMode(float3 b, float3 s) { return SetLum(s, Lum(b)); }
float3 BLuminosity(float3 b, float3 s) { return SetLum(b, Lum(s)); }

/// 生成一个「PS 变体合成」入口。
///
/// `expr` 求的是**非预乘**的 `B`，参数 `b` = 目标色、`s` = 源色。
#define MAKE_BLEND_PS(entry, expr)                                 \
    float4 entry(VSOut i) : SV_TARGET {                            \
        float4 s = ShadeSource(i);                                 \
        if (s.a <= 0.0) { return float4(0.0, 0.0, 0.0, 0.0); }     \
        float4 d = g_dst.Sample(g_smp, DstUv(i));                  \
        float3 Cs = s.rgb / s.a;                                   \
        float3 Cd = d.a > 0.0 ? d.rgb / d.a : float3(0.0, 0.0, 0.0); \
        float3 B = expr;                                           \
        float a = s.a;                                             \
        return float4((1.0 - a) * Cd + a * B, a + d.a * (1.0 - a)); \
    }

// 可分离 10 个。顺序与 `filter.rs` 的枚举**一致** —— `mode_by_index` 依赖它。
MAKE_BLEND_PS(PSOverlay,     BOverlay(Cd, Cs))
MAKE_BLEND_PS(PSDarken,      BDarken(Cd, Cs))
MAKE_BLEND_PS(PSLighten,     BLighten(Cd, Cs))
MAKE_BLEND_PS(PSColorDodge,  BColorDodge(Cd, Cs))
MAKE_BLEND_PS(PSColorBurn,   BColorBurn(Cd, Cs))
MAKE_BLEND_PS(PSHardLight,   BHardLight(Cd, Cs))
MAKE_BLEND_PS(PSSoftLight,   BSoftLight(Cd, Cs))
MAKE_BLEND_PS(PSDifference,  BDifference(Cd, Cs))
MAKE_BLEND_PS(PSExclusion,   BExclusion(Cd, Cs))
MAKE_BLEND_PS(PSMultiply,    BMultiply(Cd, Cs))

// 非可分离 4 个
MAKE_BLEND_PS(PSHue,         BHue(Cd, Cs))
MAKE_BLEND_PS(PSSaturation,  BSaturation(Cd, Cs))
MAKE_BLEND_PS(PSColor,       BColorMode(Cd, Cs))
MAKE_BLEND_PS(PSLuminosity,  BLuminosity(Cd, Cs))
// ═══════════════════════════════════════════════════════════════════════════
//  E4.4 / E4.5：模糊与投影
// ═══════════════════════════════════════════════════════════════════════════
//
// # 顶点数据怎么带参数
//
// 模糊 pass 画的是一张**铺满离屏 RT 的四边形**，它的顶点字段含义与正常图元不同，
// 于是用 `kind` 分流：
//
// | 字段      | 正常图元          | 模糊 pass                    |
// |-----------|-------------------|------------------------------|
// | `kind`    | 0..3（图元类别）  | 4（BLUR）                    |
// | `local`   | 相对四边形中心    | 该片元在源 RT 里的**像素坐标** |
// | `half_size`| 四边形半宽半高    | 源 RT 的**尺寸**（nointerp）  |
// | `corner`  | (圆角, 描边)      | (方向, σ) —— 方向 0=H 1=V     |
// | `uv`      | 纹理 uv           | 片元中心（0..1）              |
//
// 之所以能把参数塞进这些字段而不是用常量缓冲：模糊 pass 每个 RT 只有一组参数，
// 放进顶点数据就省掉一个 `ID3D11Buffer` + `SetConstantBuffers`（D3D12 那边还要动
// 根签名）。代价是 `local` 的语义被复用了 —— 用 `kind` 分流，不会串。
//
// # 为什么要 H / V 两趟
//
// 高斯核在二维上是**可分离**的：先沿 x 模糊再沿 y 模糊，与一次性做二维卷积等价，
// 但代价从 O(r²) 降到 O(2r)。这是 E4.4 能跑得动的原因。
//
// 方向不进常量而是拆成两个入口 —— 与混合模式同样的理由：离线变体零运行时开销。

/// 高斯权重。σ ≤ 0 时退化成「不模糊」，避免除零。
float GaussWeight(float i, float sigma) {
    if (sigma <= 0.0) { return i == 0.0 ? 1.0 : 0.0; }
    return exp(-(i * i) / (2.0 * sigma * sigma));
}

/// 半径 = ⌈3σ⌉，截到 32 —— 32 像素的 σ 已经超出「看得清边缘」的范畴，再多是浪费。
float BlurRadius(float sigma) {
    return min(ceil(3.0 * sigma), 32.0);
}

float4 PSBlurH(VSOut i) : SV_TARGET {
    float sigma = i.corner.y;
    float r = BlurRadius(sigma);
    float2 size = i.half_size;
    // ⚠ 累加器是 **float4**，不是 float3 —— 四通道**一起**模糊，与 Chrome / Skia 一致。
    //
    // 之前这里是 `float3`，alpha 从中心那一抽样直接取。那样做的后果不是"偏暗"，
    // 而是**投影有硬边**：影子颜色糊开了，覆盖率却还是原图的硬边，于是半透明阴影
    // 周围出现一圈 alpha 不连续的暗框。
    //
    // 为什么"只糊 rgb"会被想成更安全：不糊 alpha 时 rgb 与 a **本来就不同步**，这才是
    // 暗边的来源。预乘空间里正确的做法是对 `(rgb, a)` 一起做同一个卷积 —— 出来仍然
    // 满足 `rgb ≤ a`，因为卷积是线性的、且输入逐像素都满足这个不等式。
    float4 sum = float4(0.0, 0.0, 0.0, 0.0);
    float wsum = 0.0;
    // `[0, r]` 的整数上界。写成动态上界的循环在 SM5 里没问题，但**必须**给上限，
    // 否则恶意/异常的 σ 会让它跑几百万次。
    [loop]
    for (int k = 0; k <= 32; ++k) {
        if (float(k) > r) { break; }
        float2 uv = (i.local + float2(float(k), 0.0)) / size;
        float w = GaussWeight(float(k), sigma);
        sum += g_tex.SampleLevel(g_smp, uv, 0) * w;
        wsum += w;
        // 对称项：k = 0 只算一次（否则权重翻倍）。
        if (k > 0) {
            float2 uv2 = (i.local - float2(float(k), 0.0)) / size;
            sum += g_tex.SampleLevel(g_smp, uv2, 0) * w;
            wsum += w;
        }
    }
    // `wsum > 0` 恒成立（k=0 的权重是 1），但仍挡一道：σ 异常时宁可输出全 0
    // 也不要 NaN —— NaN 会顺着预乘链把整帧污染掉。
    if (wsum <= 0.0) { return float4(0.0, 0.0, 0.0, 0.0); }
    // 归一化后的 alpha 就是「被模糊的覆盖率」，直接送进后面的合成。
    return sum / wsum;
}

float4 PSBlurV(VSOut i) : SV_TARGET {
    float sigma = i.corner.y;
    float r = BlurRadius(sigma);
    float2 size = i.half_size;
    float4 sum = float4(0.0, 0.0, 0.0, 0.0);
    float wsum = 0.0;
    [loop]
    for (int k = 0; k <= 32; ++k) {
        if (float(k) > r) { break; }
        float2 uv = (i.local + float2(0.0, float(k))) / size;
        float w = GaussWeight(float(k), sigma);
        sum += g_tex.SampleLevel(g_smp, uv, 0) * w;
        wsum += w;
        if (k > 0) {
            float2 uv2 = (i.local - float2(0.0, float(k))) / size;
            sum += g_tex.SampleLevel(g_smp, uv2, 0) * w;
            wsum += w;
        }
    }
    if (wsum <= 0.0) { return float4(0.0, 0.0, 0.0, 0.0); }
    return sum / wsum;
}

/// 把离屏 RT 的结果**合成**回当前目标。
///
/// `corner.y > 0` 时把 rgb 换成一个**常量色**（投影的染色），alpha 保留 —— 那就是
/// DropShadow 的「剪影 → 模糊 → 染色」三步里的第三步。`corner.y == 0` 时原样透传，
/// 那就是普通模糊的回放。
float4 PSComposite(VSOut i) : SV_TARGET {
    float4 t = g_tex.SampleLevel(g_smp, i.uv, 0);
    float tintAmount = i.corner.y;
    if (tintAmount <= 0.0) {
        // 直通。已经是预乘的，别再乘一次 alpha。
        return t;
    }
    float3 tint = i.color.rgb;   // 顶点里带的是**非预乘**的目标颜色
    // 染色不能整体乘 alpha —— 那会让半透明阴影变成"颜色越淡越看不见"。
    // 正确做法：保留覆盖率，用目标色替换色相。`tint.a` 是阴影浓度。
    float a = t.a * i.color.a;
    return float4(tint * a, a);
}
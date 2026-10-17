// ZephyraUI 的单一渲染着色器（**WGSL**）—— Linux/Vulkan 侧的等价物。
//
// 与 `zephyra/zephyra-base/zephyra-winit/shaders/zephyra.hlsl` 是同一份着色器的
// 两个方言。两个文件必须**逐项对应**，对应关系有四条：
//
// 1. 顶点字段顺序 / 偏移 / `kind` 阈值 ↔ `zephyra_fx::quad::Vertex`；
// 2. `kind` 的四个取值（0 矩形 / 1 字形 / 2 图像 / 3 多边形 / 4 模糊 pass）；
// 3. `PSMain` 的三个分支阈值（0.5 / 1.5 / 2.5）与 SDF 的 `px = 0.75`；
// 4. 模糊的半径公式 `min(ceil(3σ), 32)` 与四通道一起卷积。
//
// # 为什么是「抄一份」而不是「编译一份」
//
// D3D 与 Vulkan 的着色器语言不同（HLSL vs WGSL），没有共同的中间表示能同时喂给
// 两条 API。所以这里选择**两份手写 + 逐条对应 + 用像素对照测试钉住**，而不是
// 引入 naga/fxc 之类的交叉编译链：
//
// - 交叉编译要多一个工具链依赖（且 fxc 只在 Windows 上有，Linux 上根本编不出
//   HLSL 那份），换来的是「同一份源」这一**已经由对照测试提供**的性质；
// - 漂移的症状是「某个混合模式算错」或「边缘宽度不对」—— 两者都由
//   `tests/backend_parity.rs`（Windows 侧）与本 crate 的 `blend_coefficients` /
//   `kind_thresholds` 纯逻辑测试钉住，不靠"两份文件一样"这句承诺。
//
// # 与 HLSL 的三处语言差异（都记在这里，免得下次重新踩）
//
// - `clip(x)` → `if (x <= 0.0) { discard; }`（WGSL 没有 clip）；
// - `lerp` → `mix`；
// - 模糊里必须用 `textureSampleLevel`（显式 LOD）而不是 `textureSample`：
//   循环 + `break` 是**非一致控制流**，而 `textureSample` 要求一致控制流。

struct VsIn {
    @location(0) position: vec2f,
    @location(1) local: vec2f,
    @location(2) uv: vec2f,
    @location(3) color: vec4f,
    /// x = 圆角半径（DIP），y = 描边宽度（DIP）；模糊 pass 里是 (方向, σ)。
    @location(4) corner: vec2f,
    /// 四边形半宽半高；模糊 pass 里是源 RT 的尺寸。
    @location(5) half_size: vec2f,
    /// 0 = SDF 矩形，1 = 字形，2 = 图像，3 = 多边形，4 = 模糊 pass。
    @location(6) kind: f32,
};

struct VsOut {
    @builtin(position) clip: vec4f,
    @location(0) local: vec2f,
    @location(1) uv: vec2f,
    @location(2) color: vec4f,
    @location(3) corner: vec2f,
    /// `@interpolate(flat)` 对应 HLSL 的 `nointerpolation`。
    ///
    /// 整个四边形要么全是一类要么全不是：逐片元插值会在四边形内部产生中间值，
    /// 导致部分像素走错分支（"图标变成纯色块" / "字变成方块"）。
    @location(4) @interpolate(flat) kind: f32,
    /// `half_size` 在片元里必须**精确**（它参与 `abs(p) - half_size` 的相减），
    /// 所以同样 flat。四个顶点填的是同一个值，插值本来也精确；标出来是为了让
    /// 这件事在代码里显式。
    @location(5) @interpolate(flat) half_size: vec2f,
    /// 原始 NDC 位置。用途只有一个：采样 `g_dst` 时需要**屏幕 uv**。
    @location(6) ndc: vec2f,
};

// group 0：主通道只需要「一张纹理 + 一个采样器」。模糊 / 合成 pass 也用同一个
// 布局，所以它们与主通道共享同一批 bind group。
@group(0) @binding(0) var g_tex: texture_2d<f32>;
@group(0) @binding(1) var g_smp: sampler;
// group 1：可分离 / 非可分离混合模式要读**目标**。单独一个 group，于是只用
// group 0 的那些 pass 不必绑一张多余的纹理。
@group(1) @binding(0) var g_dst: texture_2d<f32>;

@vertex
fn vs_main(i: VsIn) -> VsOut {
    var o: VsOut;
    o.clip = vec4f(i.position, 0.0, 1.0);
    o.local = i.local;
    o.uv = i.uv;
    o.color = i.color;
    o.corner = i.corner;
    o.kind = i.kind;
    o.half_size = i.half_size;
    o.ndc = i.position;
    return o;
}

/// 目标像素的屏幕 uv（与 `quad::expand` 里 `to_ndc` 的 y 方向相反）。
///
/// NDC 的 y 向上、纹理 uv 的 y 向下，所以是 `0.5 - y*0.5`。
/// 写反的症状是"混合效果整体上下颠倒"，且只在 `dst` 与源不同时才看得出来。
fn dst_uv(i: VsOut) -> vec2f {
    return vec2f(i.ndc.x * 0.5 + 0.5, 0.5 - i.ndc.y * 0.5);
}

/// 圆角矩形的有符号距离：矩形内为负，外部为正。
fn sd_round_rect(p: vec2f, half_size: vec2f, radius: f32) -> f32 {
    let r = min(radius, min(half_size.x, half_size.y));
    let q = abs(p) - half_size + vec2f(r, r);
    return length(max(q, vec2f(0.0, 0.0))) + min(max(q.x, q.y), 0.0) - r;
}

/// 算出这一片元的**预乘**源色（不含任何混合）。
///
/// 抽出来是因为「算出源色」这部分与混合模式**完全无关**，共用一份才不会出现
/// 「某个混合模式下的圆角矩形与默认路径长得不一样」这类漂移。
fn shade_source(i: VsOut) -> vec4f {
    // kind == 3：已 CPU 三角化的多边形。**不采样纹理**，直接返回顶点色。
    //
    // ⚠ 必须排在图像分支之前 —— 图像分支的判据是 `kind > 1.5`，而 3 > 1.5，
    // 于是每个三角形都会去采当前批次绑着的那张纹理。症状是多边形变成"别的
    // 内容"且换批次就换颜色，看起来像随机噪声而不是错误。
    //
    // 阈值用 2.5（相邻两类的中点）：写 3.0 会把恰好等于 3.0 的多边形漏进图像分支。
    //
    // 抗锯齿：这里**没有**过渡带。三角化后的多边形没有解析距离场，覆盖率只能
    // 来自 MSAA 的多点采样。
    if (i.kind > 2.5) {
        if (i.color.a - 0.0039 <= 0.0) { discard; }
        return i.color;
    }

    // kind == 1：字形。采样图集，用 alpha 当覆盖率。
    if (i.kind > 0.5 && i.kind < 1.5) {
        let texel = textureSample(g_tex, g_smp, i.uv);
        // 图集是 R8 展开成 RGBA，覆盖率在 alpha。字体位图自带抗锯齿，这里**不再**
        // 叠 SDF 的边缘过渡，否则字会被啃一圈。
        let a = texel.a * i.color.a;
        if (a - 0.0039 <= 0.0) { discard; }
        // 预乘空间：`i.color.rgb` 在顶点展开时已乘过 `i.color.a`，这里再乘一次
        // 覆盖率。漏掉这步 rgb 与 a 不同步 —— 半透明笔画会比它该有的更亮。
        return vec4f(i.color.rgb * texel.a, a);
    }

    // kind == 2：图像。采样当前批绑定的纹理（每批一种）。
    if (i.kind > 1.5) {
        let texel = textureSample(g_tex, g_smp, i.uv);
        var a = texel.a * i.color.a;
        // ⚠ 圆角遮罩**必须**以 `corner.x > 0` 为条件，不能无条件施加。
        //
        // 圆角矩形的 SDF 在 `d == 0` 处正是矩形边界，于是
        // `1 - smoothstep(-px, px, 0) == 0.5` —— 无条件施加会把最外一圈像素压到
        // 50% alpha。症状是**图像四周发暗一圈**，而所有计数器正常。
        if (i.corner.x > 0.0) {
            let half_size = i.half_size;
            let p = i.local - half_size;
            let d = sd_round_rect(p, half_size, i.corner.x);
            let px_img = 0.75;
            a = a * (1.0 - smoothstep(-px_img, px_img, d));
        }
        if (a - 0.0039 <= 0.0) { discard; }
        return vec4f(texel.rgb * texel.a * i.color.rgb, a);
    }

    // kind == 0：SDF 圆角矩形。
    let half_size = i.half_size;
    let p = i.local - half_size;
    let d = sd_round_rect(p, half_size, i.corner.x);

    // 1 像素过渡带：抗锯齿。带宽不随缩放变化，所以高 DPI 下边缘不会变毛糙。
    let px = 0.75;
    let alpha = 1.0 - smoothstep(-px, px, d);
    if (alpha - 0.0039 <= 0.0) { discard; }

    if (i.corner.y > 0.0) {
        // 描边：距边界 stroke 宽度以内的环带。
        //
        // ⚠ 这里**不能**再取一次 `1.0 -`：`d < 0` 在内部、`d == 0` 在边界，环带
        // 对应 `d ∈ [-width, 0]`；而 `smoothstep(inner-px, inner+px, d)` 在
        // `d == 0` 取 1、在环带内侧取 0 —— 正是要的。再取一次会把整条环带翻过来：
        // 越靠里越实、越靠边越透明，描边看起来变成一块实心填充。
        let inner = -i.corner.y;
        var sa = smoothstep(inner - px, inner + px, d);

        // 外侧**必须**再乘一道淡出：`d > 0` 在环带之外，而 smoothstep 在那里
        // 饱和为 1。四边形按 px（`AA_BAND`）向外扩过，缺这一道会让描边外侧多出
        // 一圈实色 —— 症状是"描边比设定值粗一圈，且那一圈没有抗锯齿"。
        let outer_fade = 1.0 - smoothstep(-px, px, d);
        sa = sa * outer_fade;

        if (sa - 0.0039 <= 0.0) { discard; }
        return vec4f(i.color.rgb * sa, i.color.a * sa);
    }
    return vec4f(i.color.rgb * alpha, i.color.a * alpha);
}

/// 默认路径：源色交给**固定功能混合方程**（`SrcOver`，或 `runs` 段指定的系数对）。
@fragment
fn fs_main(i: VsOut) -> @location(0) vec4f {
    return shade_source(i);
}

// ═══════════════════════════════════════════════════════════════════════════
//  可分离 / 非可分离混合模式
// ═══════════════════════════════════════════════════════════════════════════
//
// 为什么必须用**片元着色器变体**而不是统一一个方程：系数可表达的那 15 个
// （`Clear`..`Screen`）是 `r = Cs·ks + Cd·kd` 的形式，固定功能混合**原样**就能算；
// 往后的 14 个不是 —— 可分离的 10 个里 `f` 不是线性组合（`Overlay` 有
// `1−2·Cb` 这种分支），非可分离的 4 个依赖对方的色相 / 饱和度 / 明度。
//
// ⚠ HLSL 那边用宏一次生成 14 个入口；WGSL 没有预处理器，所以这里是 14 个
// 三行的入口 + 共享的 `blend_with_dst`。顺序与 `filter.rs` 的枚举**一致** ——
// 宿主按 `runs[].blend` 的下标挑入口，两边错位会让**某个混合模式画成别的模式**
// 且没有任何报错。
//
// 合成公式（预乘空间）：
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
// `as == 0` 时直接返回全 0：`(1−as)·Cd` 会把目标**擦掉**，而"什么都没画"
// 绝不该改变目标像素。
//
// # 与系数模式的等价性
//
// `SrcOver` 在这个公式下的结果是 `(1−as)·Cd + as·Cs = Cs + (1−as)·Cd`，与固定功能的
// `ONE/INV_SRC_ALPHA` **完全一致**。所以 15 个系数模式不需要走这里 —— 少一条
// 路径就少一处能漂移的地方。

fn lum(c: vec3f) -> f32 {
    return dot(c, vec3f(0.3, 0.59, 0.11));
}

/// 非可分离模式的标准前置：把色域外的负分量拉到 0（保住明度关系）。
fn clip_color(c_in: vec3f) -> vec3f {
    var c = c_in;
    let l = lum(c);
    let n = min(c.r, min(c.g, c.b));
    let x = max(c.r, max(c.g, c.b));
    if (n < 0.0) { c = l + (c - l) * l / max(l - n, 1e-6); }
    if (x > 1.0) { c = l + (c - l) * (1.0 - l) / max(x - l, 1e-6); }
    return c;
}

fn set_lum(c: vec3f, l: f32) -> vec3f {
    return clip_color(c + vec3f(l - lum(c)));
}

/// ⚠ 返回**标量**，不是 vec3f。
fn sat(c: vec3f) -> f32 {
    return max(c.r, max(c.g, c.b)) - min(c.r, min(c.g, c.b));
}

fn set_sat(c: vec3f, s: f32) -> vec3f {
    let mn = min(c.r, min(c.g, c.b));
    let mx = max(c.r, max(c.g, c.b));
    let d = mx - mn;
    if (d <= 0.0) { return vec3f(0.0, 0.0, 0.0); }
    return (c - vec3f(mn)) / d * s;
}

// ---- 可分离：逐通道 ----
fn b_overlay(b: vec3f, s: vec3f) -> vec3f {
    return mix(2.0 * b * s, 1.0 - 2.0 * (1.0 - b) * (1.0 - s), step(vec3f(0.5), b));
}
fn b_darken(b: vec3f, s: vec3f) -> vec3f { return min(b, s); }
fn b_lighten(b: vec3f, s: vec3f) -> vec3f { return max(b, s); }
fn b_color_dodge(b: vec3f, s: vec3f) -> vec3f {
    let z = step(vec3f(0.0), b);
    let m = min(vec3f(1.0), (1.0 - b) / max(s, vec3f(1e-6)));
    return m * z;
}
fn b_color_burn(b: vec3f, s: vec3f) -> vec3f {
    let z = step(b, vec3f(1.0));
    let m = min(vec3f(1.0), 1.0 - (1.0 - b) / max(1.0 - s, vec3f(1e-6)));
    return m * z + (1.0 - z);
}
fn b_hard_light(b: vec3f, s: vec3f) -> vec3f { return b_overlay(s, b); }
fn b_soft_light(b: vec3f, s: vec3f) -> vec3f {
    let d = ((16.0 * b - 12.0) * b + 4.0) * b * step(b, vec3f(0.25));
    return sqrt(max(b, vec3f(0.0))) * step(vec3f(0.5), b) * s + d * (1.0 - s);
}
fn b_difference(b: vec3f, s: vec3f) -> vec3f { return abs(b - s); }
fn b_exclusion(b: vec3f, s: vec3f) -> vec3f { return b + s - 2.0 * b * s; }
fn b_multiply(b: vec3f, s: vec3f) -> vec3f { return b * s; }

// ---- 非可分离：需要 clip_color / set_lum / set_sat ----
fn b_hue(b: vec3f, s: vec3f) -> vec3f { return set_lum(set_sat(s, sat(b)), lum(b)); }
fn b_saturation(b: vec3f, s: vec3f) -> vec3f { return set_lum(set_sat(b, sat(s)), lum(b)); }
fn b_color_mode(b: vec3f, s: vec3f) -> vec3f { return set_lum(s, lum(b)); }
fn b_luminosity(b: vec3f, s: vec3f) -> vec3f { return set_lum(b, lum(s)); }

/// 「PS 变体合成」的公共部分：`mode` 选哪一档混合函数，参数 `b` = 目标色、
/// `s` = 源色。
///
/// # 为什么用 mode 标量而不是传函数指针
///
/// WGSL **不允许**把函数指针当参数（naga 直接解析失败，实测 2026-10：
/// `expected template args end, found "->"`）。HLSL 那边用宏一次生成 14 个入口，
/// WGSL 没有预处理器 —— 于是「共享公式 + 一个判据」是唯一既能共用实现、又不引宏
/// 的写法。代价是这 14 条管线在片元里多一次分支，而它们本来就是罕见路径
/// （普通图元走 `fs_main` + 固定功能方程，不进这里）。
///
/// ⚠ `mode` 的编号必须与 `BLEND_PS_NAMES` 的下标一致（宿主按下标取入口）。
fn blend_with_dst(i: VsOut, mode: u32) -> vec4f {
    let s = shade_source(i);
    if (s.a <= 0.0) { return vec4f(0.0, 0.0, 0.0, 0.0); }
    let d = textureSample(g_dst, g_smp, dst_uv(i));
    let cs = s.rgb / s.a;
    var cd = vec3f(0.0, 0.0, 0.0);
    if (d.a > 0.0) { cd = d.rgb / d.a; }
    var b = vec3f(0.0, 0.0, 0.0);
    if (mode == 0u)      { b = b_overlay(cd, cs); }
    else if (mode == 1u) { b = b_darken(cd, cs); }
    else if (mode == 2u) { b = b_lighten(cd, cs); }
    else if (mode == 3u) { b = b_color_dodge(cd, cs); }
    else if (mode == 4u) { b = b_color_burn(cd, cs); }
    else if (mode == 5u) { b = b_hard_light(cd, cs); }
    else if (mode == 6u) { b = b_soft_light(cd, cs); }
    else if (mode == 7u) { b = b_difference(cd, cs); }
    else if (mode == 8u) { b = b_exclusion(cd, cs); }
    else if (mode == 9u) { b = b_multiply(cd, cs); }
    else if (mode == 10u) { b = b_hue(cd, cs); }
    else if (mode == 11u) { b = b_saturation(cd, cs); }
    else if (mode == 12u) { b = b_color_mode(cd, cs); }
    else                 { b = b_luminosity(cd, cs); }
    let a = s.a;
    return vec4f((1.0 - a) * cd + a * b, a + d.a * (1.0 - a));
}

// ---- 14 个入口。顺序与 `zephyra_core::filter::BlendMode` 的枚举声明顺序一致。 ----
@fragment
fn fs_overlay(i: VsOut) -> @location(0) vec4f {
    return blend_with_dst(i, 0u);
}
@fragment
fn fs_darken(i: VsOut) -> @location(0) vec4f {
    return blend_with_dst(i, 1u);
}
@fragment
fn fs_lighten(i: VsOut) -> @location(0) vec4f {
    return blend_with_dst(i, 2u);
}
@fragment
fn fs_color_dodge(i: VsOut) -> @location(0) vec4f {
    return blend_with_dst(i, 3u);
}
@fragment
fn fs_color_burn(i: VsOut) -> @location(0) vec4f {
    return blend_with_dst(i, 4u);
}
@fragment
fn fs_hard_light(i: VsOut) -> @location(0) vec4f {
    return blend_with_dst(i, 5u);
}
@fragment
fn fs_soft_light(i: VsOut) -> @location(0) vec4f {
    return blend_with_dst(i, 6u);
}
@fragment
fn fs_difference(i: VsOut) -> @location(0) vec4f {
    return blend_with_dst(i, 7u);
}
@fragment
fn fs_exclusion(i: VsOut) -> @location(0) vec4f {
    return blend_with_dst(i, 8u);
}
@fragment
fn fs_multiply(i: VsOut) -> @location(0) vec4f {
    return blend_with_dst(i, 9u);
}
@fragment
fn fs_hue(i: VsOut) -> @location(0) vec4f {
    return blend_with_dst(i, 10u);
}
@fragment
fn fs_saturation(i: VsOut) -> @location(0) vec4f {
    return blend_with_dst(i, 11u);
}
@fragment
fn fs_color(i: VsOut) -> @location(0) vec4f {
    return blend_with_dst(i, 12u);
}
@fragment
fn fs_luminosity(i: VsOut) -> @location(0) vec4f {
    return blend_with_dst(i, 13u);
}

// ═══════════════════════════════════════════════════════════════════════════
//  模糊与投影
// ═══════════════════════════════════════════════════════════════════════════
//
// 模糊 pass 画的是一张**铺满离屏 RT 的四边形**，顶点字段的含义与正常图元不同，
// 于是用 `kind = 4` 分流（表见 HLSL 那份的同名小节）：
//
// | 字段       | 正常图元         | 模糊 pass                   |
// |------------|------------------|-----------------------------|
// | `kind`     | 0..3（图元类别） | 4                           |
// | `local`    | 相对四边形中心   | 该片元在源 RT 里的**像素坐标** |
// | `half_size`| 四边形半宽半高   | 源 RT 的**尺寸**（flat）     |
// | `corner`   | (圆角, 描边)     | (方向, σ) —— 0 = H，1 = V   |
// | `uv`       | 纹理 uv          | 片元中心（0..1）             |
//
// 之所以能把参数塞进顶点而不是常量缓冲：每个 RT 只有一组参数，省掉一个 uniform
// buffer 与一次绑定。代价是 `local` 的语义被复用了 —— 用 `kind` 分流，不会串。
//
// # 为什么要 H / V 两趟
//
// 高斯核在二维上**可分离**：先沿 x 再沿 y，与一次性做二维卷积等价，代价从
// O(r²) 降到 O(2r)。
//
// # 累加器是 **vec4f**，不是 vec3f
//
// 四通道**一起**模糊，与 Chrome / Skia 一致。只糊 rgb 会让投影出现硬边：影子颜色
// 糊开了而覆盖率还是原图的硬边，于是半透明阴影周围出现一圈 alpha 不连续的暗框。
// 预乘空间里正确的做法是对 `(rgb, a)` 做同一个卷积 —— 出来仍满足 `rgb ≤ a`，
// 因为卷积是线性且输入逐像素都满足这个不等式。

fn gauss_weight(i: f32, sigma: f32) -> f32 {
    // σ ≤ 0 时退化成「不模糊」，避免除零。
    if (sigma <= 0.0) { return select(0.0, 1.0, i == 0.0); }
    return exp(-(i * i) / (2.0 * sigma * sigma));
}

/// 半径 = ⌈3σ⌉，截到 32 —— 32 像素的 σ 已超出「看得清边缘」的范畴，再多是浪费。
fn blur_radius(sigma: f32) -> f32 {
    return min(ceil(3.0 * sigma), 32.0);
}

fn blur_common(i: VsOut, horizontal: bool) -> vec4f {
    let sigma = i.corner.y;
    let r = blur_radius(sigma);
    let size = i.half_size;
    var sum = vec4f(0.0, 0.0, 0.0, 0.0);
    var wsum = 0.0;
    // `[0, r]` 的整数上界。必须给循环上限，否则异常大的 σ 会让它跑几百万次。
    for (var k: i32 = 0; k <= 32; k = k + 1) {
        if (f32(k) > r) { break; }
        let fk = f32(k);
        var step_v = vec2f(fk, 0.0);
        if (!horizontal) { step_v = vec2f(0.0, fk); }
        let w = gauss_weight(fk, sigma);
        sum = sum + textureSampleLevel(g_tex, g_smp, (i.local + step_v) / size, 0.0) * w;
        wsum = wsum + w;
        // 对称项：k = 0 只算一次（否则权重翻倍）。
        if (k > 0) {
            sum = sum + textureSampleLevel(g_tex, g_smp, (i.local - step_v) / size, 0.0) * w;
            wsum = wsum + w;
        }
    }
    // `wsum > 0` 恒成立（k=0 的权重是 1），但仍挡一道：σ 异常时宁可输出全 0
    // 也不要 NaN —— NaN 会顺着预乘链把整帧污染掉。
    if (wsum <= 0.0) { return vec4f(0.0, 0.0, 0.0, 0.0); }
    return sum / wsum;
}

@fragment
fn fs_blur_h(i: VsOut) -> @location(0) vec4f {
    return blur_common(i, true);
}

@fragment
fn fs_blur_v(i: VsOut) -> @location(0) vec4f {
    return blur_common(i, false);
}

/// 把离屏 RT 的结果**合成**回当前目标。
///
/// `corner.y > 0` 时把 rgb 换成一个**常量色**（投影的染色），alpha 保留 —— 那就是
/// DropShadow 的「剪影 → 模糊 → 染色」三步里的第三步。`corner.y == 0` 时原样透传，
/// 那就是普通模糊的回放。
@fragment
fn fs_composite(i: VsOut) -> @location(0) vec4f {
    let t = textureSampleLevel(g_tex, g_smp, i.uv, 0.0);
    let tint_amount = i.corner.y;
    if (tint_amount <= 0.0) {
        // 直通。已经是预乘的，别再乘一次 alpha。
        return t;
    }
    // 顶点里带的是**非预乘**的目标颜色。
    // ⚠ 染色不能整体乘 alpha —— 那会让半透明阴影变成"颜色越淡越看不见"。
    // 正确做法：保留覆盖率，用目标色替换色相。`color.a` 是阴影浓度。
    let a = t.a * i.color.a;
    return vec4f(i.color.rgb * a, a);
}
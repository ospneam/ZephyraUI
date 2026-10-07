//! CPU 参考光栅器：把 `quad::expand` 的顶点流**在 CPU 上画出来**，写成一张图。
//!
//! # 它回答什么问题
//!
//! 「顶点流本身对不对」与「GPU 有没有把它画出来」是**两件事**。本模块回答前者：
//!
//! ```text
//! DisplayList ──quad::expand──▶ 顶点流 ──本模块──▶ 一张 RGBA 图（能打开看）
//!                                  │
//!                                  └──GPU──▶ 屏幕（本机阻塞，见 tests/d3d11_draw.rs）
//! ```
//!
//! 两者吃的是**同一个**顶点流。所以：
//!
//! - 这张图对、屏幕空白 ⇒ 顶点流没问题，嫌疑落在 GPU 环节（纹理格式 / 采样器 /
//!   混合 / 视口）。本项目现在正处在这个状态。
//! - 这张图也空 ⇒ 不用再查 GPU，`expand` 或它上游就已经错了。
//!
//! 本项目反复出现「不报错但什么都不画」，每次都只能靠猜。有了它就能先排除一大半 ——
//! 这是本仓库里**唯一**能在 GPU 不可用时给出像素级判据的东西。
//!
//! # 范围
//!
//! 只实现 `kind` 的矩形与字形两个分支：
//!
//! - `RECT`：照 `zephyra.hlsl` 的 `PSMain` 逐行重演 SDF 圆角 + 描边；
//! - `GLYPH`：按 uv 采样字形图集，取 alpha 当覆盖率；
//! - `IMAGE`：**不画**。它绑的是图像纹理而非字形图集，去采样字形图集会画出完全
//!   不相干的图案 —— 那比留空更难查。
//!
//! # 它**不是**像素级金标准
//!
//! 覆盖率是逐像素求值（GPU 是多点采样），字形用最近邻（GPU 那边的采样器由宿主决定）。
//! 所以边缘锐度会与实际画面有可见差别。
//!
//! 它能告诉你：**形状对不对、位置对不对、有没有内容**。
//! 它不能告诉你：**抗锯齿边缘和 GPU 完全一致**。

#![cfg(windows)]

use crate::quad::{self, kind, Vertex};

/// 一张 RGBA8 位图（**线性**颜色，未做 sRGB 编码）。
///
/// 为什么是线性：与着色器输出一致。sRGB 编码只发生在**写文件给人看**的那一侧
/// （见 [`crate::write_bmp`] 的 `encode_srgb`）。混进渲染会让「像素值对不对」
/// 这个判据变得不可用。
pub struct CpuCanvas {
    pub width: u32,
    pub height: u32,
    /// RGBA8，每像素 4 字节。
    pub pixels: Vec<u8>,
}

impl CpuCanvas {
    /// 铺一块纯色。
    pub fn new(width: u32, height: u32, clear: [f32; 4]) -> Self {
        let px = to_bytes(clear);
        let mut pixels = vec![0u8; (width * height) as usize * 4];
        for p in pixels.chunks_exact_mut(4) {
            p.copy_from_slice(&px);
        }
        Self {
            width,
            height,
            pixels,
        }
    }

    /// 覆盖一个像素。
    ///
    /// 混合组合与 D3D11 侧一致：`Src/InvSrc = ALPHA/INV_SRC_ALPHA`，
    /// alpha 通道 `ONE/INV_SRC_ALPHA`。抄错这条会让半透明区域整体偏暗 ——
    /// 那时看起来像「颜色不对」，而根因在参考实现自己身上，是最难查的一类。
    fn blend(&mut self, x: i32, y: i32, color: [f32; 4], coverage: f32) {
        if coverage <= 0.0
            || x < 0
            || y < 0
            || x >= self.width as i32
            || y >= self.height as i32
        {
            return;
        }
        let a = (color[3] * coverage).clamp(0.0, 1.0);
        if a <= 0.0 {
            return;
        }
        let inv = 1.0 - a;
        let i = (y as u32 * self.width + x as u32) as usize * 4;
        let dst_a = self.pixels[i + 3] as f32 / 255.0;
        for (ch, dst) in self.pixels[i..i + 3].iter_mut().enumerate() {
            let s = color[ch] * 255.0;
            let d = *dst as f32;
            *dst = (s * a + d * inv).round().clamp(0.0, 255.0) as u8;
        }
        self.pixels[i + 3] = ((a + dst_a * inv) * 255.0).round().clamp(0.0, 255.0) as u8;
    }

    /// 画一个四边形（`chunk` 是 4 个顶点，顺序与 `quad::quad` 一致）。
    ///
    /// # 为什么**逐像素**判覆盖率，而不是两个三角形
    ///
    /// 顶点顺序是 `(左上, 右上, 右下, 左下)`，而 `quad::INDICES` 是 `(0,1,2, 0,2,3)`
    /// —— 这两条对角线把矩形切成两个**互不重叠**的三角形。用重心坐标逐三角形光栅化
    /// 是对的，但两次采样之间的插值会被**重新归一化**，于是对角线两侧的 uv 会略有
    /// 偏差：在字形这种 7×9 像素的小块上，那点偏差足以让笔画错位一整像素。
    ///
    /// 所以这里改成**逐像素求覆盖率**：对包围盒里的每个像素，用它在矩形内的
    /// 归一化位置直接算 uv。只在**矩形内**才画 —— 这与「两个三角形并集恰好等于
    /// 矩形」等价，但没有插值偏差。
    fn quad(&mut self, chunk: &[Vertex], atlas: Option<&AtlasBitmap>) {
        let Some(a) = chunk.first() else { return };
        let to_px = |ndc: [f32; 2]| {
            (
                (ndc[0] * 0.5 + 0.5) * self.width as f32,
                (0.5 - ndc[1] * 0.5) * self.height as f32,
            )
        };
        // 四个角在屏幕上的位置：`a` 左上、`b` 右上、`c` 右下、`d` 左下。
        let corners = [
            to_px(chunk[0].position),
            to_px(chunk[1].position),
            to_px(chunk[2].position),
            to_px(chunk[3].position),
        ];
        let min_x = corners.iter().map(|p| p.0).fold(f32::MAX, f32::min);
        let max_x = corners.iter().map(|p| p.0).fold(f32::MIN, f32::max);
        let min_y = corners.iter().map(|p| p.1).fold(f32::MAX, f32::min);
        let max_y = corners.iter().map(|p| p.1).fold(f32::MIN, f32::max);

        let w = max_x - min_x;
        let h = max_y - min_y;
        if w <= 0.0 || h <= 0.0 {
            return;
        }

        let x0 = min_x.floor().max(0.0) as i32;
        let x1 = max_x.ceil().min(self.width as f32) as i32;
        let y0 = min_y.floor().max(0.0) as i32;
        let y1 = max_y.ceil().min(self.height as f32) as i32;

        for py in y0..y1 {
            for px in x0..x1 {
                // 像素中心在矩形内的归一化位置 (u, v)，v 向下为正。
                let u = (px as f32 + 0.5 - min_x) / w;
                let v = (py as f32 + 0.5 - min_y) / h;
                // 只画矩形内。留一点点余量以免边界像素被漏掉。
                if !(-0.001..=1.001).contains(&u) || !(-0.001..=1.001).contains(&v) {
                    continue;
                }
                // 顶点属性按同样的归一化位置取。
                // uv 在四边形内线性插值：左上是 uv0，右下是 uv1。
                let uv = [
                    a.uv[0] + (chunk[2].uv[0] - a.uv[0]) * u,
                    a.uv[1] + (chunk[2].uv[1] - a.uv[1]) * v,
                ];
                // `local` 用**与硬件相同的方式**插值：取左上角顶点与右下角顶点，
                // 按同一个 (u, v) 线性混合。这样这里的 `d` 与 GPU 上算出的 `d` 逐位可比，
                // 而不是"差不多"。
                //
                // 刻意**不**做归一化：`local`、`half_size`、`corner` 三者同单位
                // （DIP 像素），相减就是真实的带符号距离。
                let local = [
                    a.local[0] + (chunk[2].local[0] - a.local[0]) * u,
                    a.local[1] + (chunk[2].local[1] - a.local[1]) * v,
                ];
                // `half_size` 标了 `nointerpolation`，四个顶点填同一个值，直接取第一个。
                let half_size = a.half_size;
                // `color` / `kind` 是逐四边形恒定的，取第一个顶点即可。
                let color = a.color;
                let coverage = match a.kind {
                    k if (k - kind::GLYPH).abs() < 0.01 => {
                        atlas.map(|at| at.sample_alpha(uv)).unwrap_or(0.0)
                    }
                    // 图像：**只画形状，不采样纹理。**
                    //
                    // ⚠️ 这里过去恒返回 `0.0`，于是**整条图像链在无 GPU 环境零验证** ——
                    // 那是本仓审计里一直没修的一条。现在按「形状 + tint」近似绘制：
                    // 半径为 0 时整块不透明，半径 > 0 时套 SDF 遮罩（与 HLSL 的 IMAGE
                    // 分支**同一套坐标、同一段公式**，所以两边的形状逐位可比）。
                    //
                    // 纹理采样仍然是零 —— CPU 侧没有纹理表（见文件头），而补齐它要引入
                    // 一条与「形状对不对」正交的依赖。先把形状验上。
                    k if (k - kind::IMAGE).abs() < 0.01 => {
                        // ⚠️ 半径为 0 时**不加**遮罩，必须与 GPU 的 `corner.x > 0`
                        // 分支一致：SDF 的 `d == 0` 恰好落在矩形边界，
                        // `1 - smoothstep(-px, px, 0) == 0.5` —— 无条件施加会把
                        // 最外一圈像素压到 50%，两边就会分叉。
                        if a.corner[0] > 0.0 {
                            sdf_coverage(local, half_size, a.corner)
                        } else {
                            1.0
                        }
                    }
                    _ => sdf_coverage(local, half_size, a.corner),
                };
                self.blend(px, py, color, coverage);
            }
        }
    }
}

/// 字形图集（CPU 位图）。
pub struct AtlasBitmap {
    /// RGBA8，`size × size`。
    pub pixels: Vec<u8>,
    pub size: u32,
}

impl AtlasBitmap {
    /// 按 uv 取 **alpha** 分量当覆盖率。
    ///
    /// 用最近邻而不是双线性：字形图集是二值化的 alpha 位图，双线性会把它糊掉
    /// （小字号下表现为笔画粘连）。
    pub fn sample_alpha(&self, uv: [f32; 2]) -> f32 {
        if uv[0] < 0.0 || uv[1] < 0.0 || uv[0] >= 1.0 || uv[1] >= 1.0 {
            return 0.0;
        }
        let s = self.size as usize;
        let x = (uv[0] * s as f32) as usize;
        let y = (uv[1] * s as f32) as usize;
        if x >= s || y >= s {
            return 0.0;
        }
        self.pixels[(y * s + x) * 4 + 3] as f32 / 255.0
    }

    /// 非透明像素数。
    pub fn inked(&self) -> usize {
        self.pixels.chunks_exact(4).filter(|p| p[3] > 0).count()
    }
}

/// `zephyra.hlsl` 里 `PSMain` 的矩形分支：`sd_round_rect` + 抗锯齿 + 描边。
///
/// **逐行照抄 HLSL，且刻意不加任何"修正"。** 改 HLSL 时这里也该改 —— 两边不一致
/// 会让「参考图」与「实际画面」在圆角处对不上，而那种差异极难归因。
///
/// # 这里曾经是"故意不照抄"的
///
/// 之前 HLSL 写的是：
///
/// ```hlsl
/// float2 half_size = i.local * 0.5;      // local 是像素坐标 0..width
/// float2 center    = i.local - half_size;
/// float d = sd_round_rect(center, half_size, i.corner.x);
/// ```
///
/// `half_size` 是从**逐片元**的 `local` 推的，于是它随片元变化，且恰好
/// `center == half_size` —— `sd_round_rect` 里 `q = abs(center) - half_size + r = r`，
/// `d` 退化成与位置**无关**的常数 `r(√2−1)`：
///
/// | 四边形 | `d` | GPU 上的结果 |
/// |---|---|---|
/// | 方角（r=0） | 0 | 整块均匀 alpha = 0.5，**没有形状** |
/// | 圆角 12px | 4.97 | alpha = 0，**完全不可见** |
///
/// 当时 CPU 侧用**归一化坐标**（`half_size` 恒为 0.5）绕开了它，等于把两个实现的
/// 差异藏起来。现在 `Vertex::half_size` 把真实的半宽半高送进来了，两边终于可以
/// 用同一套坐标 —— 归一化那套也一并扔掉，因为它会让抗锯齿带宽随四边形尺寸变化。
fn sdf_coverage(local: [f32; 2], half_size: [f32; 2], corner: [f32; 2]) -> f32 {
    // `p` = 片元相对四边形中心的偏移。`local` 与 `half_size` 同单位（顶点里就是
    // 什么单位这里就是什么单位），所以两者相减不需要任何换算。
    let p = [local[0] - half_size[0], local[1] - half_size[1]];
    let r = corner[0].min(half_size[0].min(half_size[1]));
    let q = [
        p[0].abs() - half_size[0] + r,
        p[1].abs() - half_size[1] + r,
    ];
    let (mx, my) = (q[0].max(0.0), q[1].max(0.0));
    let d = (mx * mx + my * my).sqrt() + q[0].max(q[1]).min(0.0) - r;

    // 过渡带半宽。与 `quad::AA_BAND` 是同一个量 —— 写成两处独立的 `0.75`
    // 的话，改了一处会让本参考实现与 GPU 上的实际过渡带宽度分叉，
    // 而症状是「圆角处的过渡带两边不等宽」，极难归因。
    let aa: f32 = crate::quad::AA_BAND;
    let alpha = 1.0 - smoothstep(-aa, aa, d);
    if alpha <= 0.0039 {
        return 0.0;
    }
    if corner[1] > 0.0 {
        // 描边：只画「离边界不超过 `corner[1]`」的环带，即 `d ∈ [-width, 0]`。
        //
        // 所以是**正**的 `smoothstep`：d=0（贴着边）→ 1，d=-width（环带内侧）→ 0。
        //
        // ⚠ HLSL 曾经写成 `1.0 - smoothstep(...)` —— 那个 `1.0 -` 把整条环带翻了个个儿：
        // 结果变成「越靠里越实、越靠边越透明」，于是描边看起来是一块**实心**填充。
        let inner = -corner[1];
        let sa = smoothstep(inner - aa, inner + aa, d);
        // 外侧必须**乘**一道淡出：`d > 0` 在环带之外，而 `smoothstep` 在那里饱和为 1。
        // 四边形已按 AA_BAND 外扩，所以不乘这一道，描边外侧会多出一圈实色 ——
        // 症状是「描边比边框设定值粗一圈，且那一圈没有抗锯齿」。
        let outer_fade = 1.0 - smoothstep(-aa, aa, d);
        let out = sa * outer_fade;
        return if out <= 0.0039 { 0.0 } else { out };
    }
    alpha
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn to_bytes(c: [f32; 4]) -> [u8; 4] {
    [
        (c[0] * 255.0).round().clamp(0.0, 255.0) as u8,
        (c[1] * 255.0).round().clamp(0.0, 255.0) as u8,
        (c[2] * 255.0).round().clamp(0.0, 255.0) as u8,
        (c[3] * 255.0).round().clamp(0.0, 255.0) as u8,
    ]
}

/// 把一批顶点画到画布上。
///
/// `atlas` 是字形图集；`None` 表示没有图集 —— 此时字形**不画**，矩形照画。
/// 这么分开是有意的：没有图集时把字画成色块是最容易误导人的失败方式。
pub fn draw_batch(canvas: &mut CpuCanvas, batch: &quad::QuadBatch, atlas: Option<&AtlasBitmap>) {
    for chunk in batch.solid.chunks_exact(4) {
        canvas.quad(chunk, atlas);
    }
    // 图像**也画** —— 但只画**形状**，不采样纹理（见 `kind::IMAGE` 分支）。
    //
    // ⚠️ 过去这里是空注释，于是 `batch.images` 整段从未被光栅化 ——
    // 「图像链在无 GPU 环境零验证」的老缺口就是这么来的：不是没人写测试，
    // 是这条路径根本不会被执行。
    //
    // 画形状（而不是跳过）的理由：圆角、uv 钳制、混合这些都落在图像这条链上，
    // 而它们出问题时的症状是「图像边缘发暗 / 内容错位」，在真机上极难归因。
    for chunk in batch.images.chunks_exact(4) {
        canvas.quad(chunk, atlas);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 把 `batches` 里**第一张图像**的 `uv` 换成整张纹理。
    ///
    /// 图像**不采样纹理**（见 `kind::IMAGE` 分支的注释），所以 uv 对像素没有影响 ——
    /// 但把它规整成 `FULL` 让构造读起来不误导：像是「一张铺满的图」而不是
    /// 「一张 uv 乱写的图」。
    fn image_batch(dst: zephyra_core::core::Rect, radius: f32) -> quad::QuadBatch {
        let mut l = zephyra_core::paint::DisplayList::new();
        l.push(
            zephyra_core::paint::SortKey {
                layer: zephyra_core::paint::Layer::Content,
                z: 0,
                order: 0,
            },
            zephyra_core::paint::Command::Image {
                texture: zephyra_core::paint::TextureRef::new("t"),
                src: Some(zephyra_core::core::UvRect::FULL),
                dst,
                radius: zephyra_core::core::CornerRadius { radius },
                tint: zephyra_core::core::Color::rgb(1.0, 1.0, 1.0),
                filter: Default::default(),
                blend: Default::default(),
            },
        );
        l.sort();
        quad::expand(&l, (100, 100), |_| true)
    }

    /// 圆角图像的**四角必须是空的**。
    ///
    /// # 为什么这条能钉住 CPU 侧的遮罩
    ///
    /// 本参考光栅器**不采样纹理**，所以图像的可见形状**全部**来自这个遮罩。
    /// 遮罩若没施加（或只在半径 > 0 时才施加的判据写反），画出来就是一块
    /// **方**块 —— 而「方块」在「有没有画出来」这条判据下与圆角**无法区分**。
    /// 所以判据必须是「四角空、中心实」这种**只看形状**的断言。
    #[test]
    fn 圆角图像的四角被切掉() {
        let dst = zephyra_core::core::Rect::new(10.0, 10.0, 80.0, 80.0);
        let mut canvas = CpuCanvas::new(100, 100, [0.0; 4]);
        draw_batch(&mut canvas, &image_batch(dst, 20.0), None);

        let at = |x: u32, y: u32| canvas.pixels[(y as usize * 100 + x as usize) * 4 + 3];
        // 四个**紧贴矩形角**的像素：圆角 20 下它们在形状之外。
        for (x, y) in [(11u32, 11u32), (88, 11), (11, 88), (88, 88)] {
            assert_eq!(
                at(x, y),
                0,
                "({x},{y}) 是圆角矩形的角，应该被切掉却是实心的 —— 遮罩没生效"
            );
        }
        // 中心必须是实的 —— 否则这条会因「什么都没画」而恒真。
        assert!(at(50, 50) > 200, "中心必须画上，否则上面四条断言是恒真的");
        // 矩形中边上、贴着上沿的点：不在角上，应���是实的。
        assert!(at(50, 12) > 200, "上边中点应��是实的");
    }

    /// 半径为 0 时**不施加**遮罩 —— 整块实心。
    ///
    /// ⚠️ 这条与 GPU 的 `corner.x > 0` 分支是**同一个陷阱**：
    /// SDF 的 `d == 0` 恰好落在矩形边界，`1 - smoothstep(-px, px, 0) == 0.5` ——
    /// 无条件施加会让最外一圈像素压到 50% alpha，两边就分叉了。
    /// 症状是「所有直角图像的边缘发暗一圈」，且没有任何计数器会变。
    #[test]
    fn 直角图像不施加遮罩() {
        let dst = zephyra_core::core::Rect::new(10.0, 10.0, 80.0, 80.0);
        let mut canvas = CpuCanvas::new(100, 100, [0.0; 4]);
        draw_batch(&mut canvas, &image_batch(dst, 0.0), None);
        // 紧贴四边的像素必须是**不透明**的，而不是 50%。
        for (x, y) in [(10u32, 10u32), (10, 89), (89, 10), (89, 89)] {
            let a = canvas.pixels[(y as usize * 100 + x as usize) * 4 + 3];
            assert!(
                a > 240,
                "({x},{y}) 的 alpha 只有 {a} —— 遮罩被无条件施加了（应 ≈255）"
            );
        }
    }

    /// 中心满、边界半、界外空 —— 覆盖率必须是 `d` 的单调函数。
    ///
    /// `d` 的定义就是「到边界的带符号距离」，所以覆盖率只能由它决定。
    #[test]
    fn sdf_覆盖率随到边界的距离单调下降() {
        let half = [10.0, 10.0]; // 20×20 的四边形
        // 边界在 local.x = 20，从内往外每 0.25 走一步。
        let samples: [([f32; 2], f32); 5] = [
            ([18.0, 10.0], 1.0),   // 深内部
            ([19.25, 10.0], 1.0), // 过渡带内侧
            ([19.75, 10.0], 0.741),
            ([20.0, 10.0], 0.5), // 抗锯齿中点
            ([20.75, 10.0], 0.0), // 界外
        ];
        let mut prev = 2.0f32;
        for (p, expect) in samples {
            let cov = sdf_coverage(p, half, [0.0, 0.0]);
            println!("  local={p:?}  cov={cov:.4}（期望约 {expect}）");
            assert!(
                cov <= prev + 1e-6,
                "覆盖率应当随靠近边界而单调不增，但 {p:?} 处是 {cov}，上一处是 {prev}"
            );
            assert!(
                (cov - expect).abs() < 0.02,
                "local={p:?} 处覆盖率应约 {expect}，实际 {cov}"
            );
            prev = cov;
        }
    }

    /// **`half_size` 必须是四边形的常量属性**，不是从逐片元坐标推出来的。
    ///
    /// 这条测试是那个 bug 的判据，也是修好它的判据。
    ///
    /// 早先的 HLSL 写 `half_size = i.local * 0.5`，于是它逐片元变化、且恰好
    /// `center == half_size`，`d` 退化成常数 `r(√2−1)`。后果是**同一个「到边界的距离」在
    /// 不同尺寸的四边形上给出不同的覆盖率** —— 因为当时 `d` 根本不等于那个距离。
    ///
    /// 所以这里换两种尺寸、停在同样的距边界 0.25 像素处，覆盖率必须逐位相同。
    #[test]
    fn sdf_覆盖率只取决于到边界的距离_与四边形尺寸无关() {
        // 20×20，边界在 x=20，往里 0.25 → x=19.75
        let small = sdf_coverage([19.75, 10.0], [10.0, 10.0], [0.0, 0.0]);
        // 100×100，边界在 x=100，往里 0.25 → x=99.75
        let big = sdf_coverage([99.75, 50.0], [50.0, 50.0], [0.0, 0.0]);
        assert!(
            (small - big).abs() < 1e-5,
            "同样距边界 0.25 像素，覆盖率必须一致，实际 {small} vs {big} —— \
             若不等，说明 half_size 又是从 local 推的（那个 bug）"
        );
    }

    /// 中心必须**满**。旧实现里中心的 `d` 恒为 0 → alpha 恒为 0.5，整块半透明。
    #[test]
    fn sdf_中心是满的_且_圆角半径_必须真正切掉角() {
        for (name, half) in [("20×20", [10.0f32, 10.0]), ("100×100", [50.0, 50.0])] {
            let center = sdf_coverage(half, half, [0.0, 0.0]);
            assert!(
                center > 0.99,
                "{name} 的中心覆盖率应为 1，实际 {center} —— \
                 若约 0.5，说明 half_size 又是从 local 推的（那个 bug）"
            );
        }

        // 角上：20×20、半径 8。取 local=(0.5,0.5)，即离左上和上边各 0.5 像素。
        let half = [10.0, 10.0];
        let p = [0.5, 0.5];
        let sharp = sdf_coverage(p, half, [0.0, 0.0]);
        let rounded = sdf_coverage(p, half, [8.0, 0.0]);
        println!("  方角 {sharp:.4} → 圆角8 {rounded:.4}");
        assert!(
            sharp > 0.9,
            "方角时 (0.5,0.5) 几乎贴着边界，覆盖率应很高，实际 {sharp}"
        );
        assert!(
            rounded < 0.1,
            "半径 8 的圆角应当把这个角切掉（离角最近的边界只有约 2.6 像素），实际 {rounded}"
        );
    }

    /// 描边只画**环带**，中心应当是空的。
    ///
    /// 这条测试钉住 `1.0 - smoothstep` 那个多余的反相。
    #[test]
    fn sdf_描边只画环带() {
        let half = [10.0, 10.0];
        let stroke = [0.0, 2.0]; // 无圆角，描边 2 像素
        assert!(
            sdf_coverage([10.0, 10.0], half, stroke) < 0.05,
            "描边模式下中心应当透明（环带只贴边）"
        );
        assert!(
            sdf_coverage([19.5, 10.0], half, stroke) > 0.5,
            "描边模式下贴近边界处应当不透明"
        );
        // 比描边宽度更深的内侧应当空 —— 环带只有那么宽。
        assert!(
            sdf_coverage([17.0, 10.0], half, stroke) < 0.05,
            "描边宽度 2 像素，所以 d = -3（x=17）处应当已经在环带之外"
        );
    }

    /// 四边形必须留出 AA 余量，过渡带的两端才都在可见范围内。
    ///
    /// # 这条测试原来是「钉住缺陷」，现已反转为「钉住修复」
    ///
    /// 之前 `Vertex::quad` 让四边形恰好等于矩形，于是 `local - half_size` 每个轴都
    /// 落在 `[-half, +half]` 内，`abs(p) - half ≤ 0` ⇒ **`d ≤ 0` 恒成立**。填充的
    /// 过渡带在 `d ∈ (0, 0.75]`、描边的 `sa` 在 `d > 0` 时饱和为 1 —— 那半边
    /// **永远不会被采样**，所以所有矩形边缘都是硬边。
    ///
    /// 现在四边形按 [`crate::quad::AA_BAND`] 向外扩，`half_size` 跟着扩，于是片元
    /// 能落到 `d ∈ (0, AA_BAND]`，`smoothstep` 的两端都可见。
    #[test]
    fn 四边形留了_抗锯齿_余量_所以过渡带的上半边能被采样() {
        let rect = zephyra_core::core::Rect::new(10.0, 20.0, 64.0, 32.0);
        let vs = crate::quad::Vertex::from_rect(rect, (10.0, 20.0), [1.0; 4], [0.0, 0.0]);
        let band = crate::quad::AA_BAND;
        let half = vs[0].half_size;

        // `half_size` 与**扩后**几何对齐 —— 否则 `d == 0` 会偏移，描边环带长错位置。
        assert_eq!(
            half,
            [rect.width * 0.5 + band, rect.height * 0.5 + band],
            "half_size 必须等于扩后四边形的一半"
        );
        // `local` 的原点在**扩后**四边形的左上角，所以是 0；
        // 真实矩形对应 local ∈ [AA_BAND, AA_BAND + 尺寸] 这个更窄的区间。
        assert!(
            vs[0].local[0].abs() < 1e-5 && vs[0].local[1].abs() < 1e-5,
            "左上角 local 应为 0（扩后四边形的原点），实际 {:?}",
            vs[0].local
        );

        // 关键判据：四边形外沿（`d ≈ +AA_BAND`，在边界内侧一点点）必须**仍然是半透明**。
        // 这正是「原本该被裁掉的那半边」—— 它现在可被采样，所以读到 0.5 而不是 1。
        // 若四边形又缩回矩形大小，这里会读到 1.0（AA 又丢了）。
        let on_outer_band = [2.0 * half[0], half[1]];
        let cov = sdf_coverage(on_outer_band, half, [0.0, 0.0]);
        assert!(
            (cov - 0.5).abs() < 0.05,
            "四边形外沿（d ≈ +{band}）覆盖率应约 0.5，实际 {cov} —— \
             若为 1.0 说明四边形又缩回去了（抗锯齿又丢了）"
        );

        // 描边同理：外侧过渡带现在也可见。四边形外沿 `d = +AA_BAND` 恰好是
        // `smoothstep(-AA, AA, d)` 的**中点**，所以读到 0.5 —— 半透明，不是实色。
        //
        // 关键对比：修之前这里是 1.0（环带外侧饱和），于是描边多出一圈没有
        // 抗锯齿的实色，比设定值粗一圈。
        let stroke = [0.0, 2.0];
        let stroke_outer = sdf_coverage(on_outer_band, half, stroke);
        assert!(
            (stroke_outer - 0.5).abs() < 0.05,
            "描边模式下四边形外沿应是过渡带中点（约 0.5），实际 {stroke_outer} —— \
             若为 1.0 说明描边外侧没有淡出（描边会粗一圈且无抗锯齿）"
        );
        // 更外侧一点必须明显淡出，不能再有饱和的实色。
        //
        // ⚠ 取点要用**扩后半宽**做基准：片元里 `p = local - half_size`，而
        // `half_size` 已含 pad，所以「四边形右沿」在 `local.x == 2*half.x`。
        //
        // 这里读到的是**渐变过程中的一个值**而不是 0：该点 `d ≈ +0.375`，
        // 正落在 `smoothstep(-0.75, 0.75, ·)` 的上升段。判据是「远低于中点
        // 0.5」—— 修之前这里读到 1.0（饱和实色）。
        let beyond = [2.0 * half[0] + band * 0.5, half[1]];
        let beyond_cov = sdf_coverage(beyond, half, stroke);
        assert!(
            beyond_cov < 0.25,
            "四边形之外应明显淡出（读到 {beyond_cov}），\
             否则描边外侧是一圈硬边实色"
        );

        // 反过来，矩形**真实**边界（`d == 0`，即 local.x = half.x + band）
        // 上，填充必须是满的 —— AA 不该啃掉形状本身。
        let real_edge = [half[0] + band, half[1]];
        assert!(
            sdf_coverage(real_edge, half, [0.0, 0.0]) > 0.99,
            "矩形真实边界（d==0）填充应满，实际 {} —— AA 余量把形状啃小了",
            sdf_coverage(real_edge, half, [0.0, 0.0])
        );
    }

    #[test]
    fn 空白画布就是清屏色() {
        let c = CpuCanvas::new(4, 4, [1.0, 0.5, 0.0, 1.0]);
        assert_eq!(&c.pixels[0..4], &[255, 128, 0, 255]);
    }

    #[test]
    fn 图集采样取_alpha_通道() {
        let mut px = vec![0u8; 4 * 4 * 4];
        px[3] = 200; // (0,0) 的 alpha
        let a = AtlasBitmap {
            pixels: px,
            size: 4,
        };
        assert!((a.sample_alpha([0.0, 0.0]) - 200.0 / 255.0).abs() < 0.01);
        // 越界的 uv 必须返回 0 而不是回绕（回绕会采到图集另一侧的字）。
        assert_eq!(a.sample_alpha([1.0, 0.0]), 0.0);
        assert_eq!(a.sample_alpha([1.5, 0.5]), 0.0);
    }
}

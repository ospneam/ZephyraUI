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
        if coverage <= 0.0 || x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
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

    /// 画一个三角形（`chunk` 是 3 个顶点，`kind::PATH`）。
    ///
    /// # 为什么这里也需要一个光栅器
    ///
    /// `Command::Path` 是**唯一**不走 SDF 的图元：它在 `quad::expand` 里已经被
    /// 展平成三角形（多边形没有解析距离，边缘 AA 靠 MSAA），所以 CPU 侧只能
    /// 按三角形逐个填。
    ///
    /// ⚠️ 本方法曾经**不存在** —— `draw_batch` 只遍历 `solid` 与 `images` 两段，
    /// 于是 `paths` 整段从未被光栅化。症状极其安静：`report.path_triangles`
    /// 照常增长（几何统计看着很健康），图里却一个图标都没有。而 `--dump-cpu`
    /// 正是本仓**唯一**的像素级自验手段 —— 也就是说「图标在参考图里看不见」
    /// 这件事，靠人眼看图是发现不了的（没人会怀疑"图上少东西"是自己看错）。
    ///
    /// 这与历史上 `images` 那段是**同一个洞**（那里的注释还留着）。教训一致：
    /// 新增图元段时，必须同时问「谁消费它」，而不只是「谁生产它」。
    ///
    /// 覆盖率用 4×4 子采样近似 MSAA：GPU 上是多点采样，这里要在**没有**
    /// 多点采样的前提下给出可看的边缘（硬边会让参考图看起来像渲染坏了）。
    fn triangle(&mut self, chunk: &[Vertex]) {
        if chunk.len() < 3 {
            return;
        }
        let to_px = |ndc: [f32; 2]| {
            (
                (ndc[0] * 0.5 + 0.5) * self.width as f32,
                (0.5 - ndc[1] * 0.5) * self.height as f32,
            )
        };
        let p = [
            to_px(chunk[0].position),
            to_px(chunk[1].position),
            to_px(chunk[2].position),
        ];
        // 有向面积：0 = 退化（三点共线），`expand` 已经滤过一次，这里是兜底。
        let area = (p[1].0 - p[0].0) * (p[2].1 - p[0].1) - (p[2].0 - p[0].0) * (p[1].1 - p[0].1);
        if area.abs() < 1e-9 {
            return;
        }
        let min_x = p.iter().map(|q| q.0).fold(f32::MAX, f32::min);
        let max_x = p.iter().map(|q| q.0).fold(f32::MIN, f32::max);
        let min_y = p.iter().map(|q| q.1).fold(f32::MAX, f32::min);
        let max_y = p.iter().map(|q| q.1).fold(f32::MIN, f32::max);
        let x0 = min_x.floor().max(0.0) as i32;
        let x1 = max_x.ceil().min(self.width as f32) as i32;
        let y0 = min_y.floor().max(0.0) as i32;
        let y1 = max_y.ceil().min(self.height as f32) as i32;

        // 边函数：三点同号即在三角形内（与绕向无关，所以 CW / CCW 都对）。
        let edge = |a: (f32, f32), b: (f32, f32), s: (f32, f32)| {
            (b.0 - a.0) * (s.1 - a.1) - (b.1 - a.1) * (s.0 - a.0)
        };
        const SUBS: usize = 4;
        let color = chunk[0].color;
        for py in y0..y1 {
            for px in x0..x1 {
                let mut hits = 0u32;
                for sy in 0..SUBS {
                    for sx in 0..SUBS {
                        let s = (
                            px as f32 + (sx as f32 + 0.5) / SUBS as f32,
                            py as f32 + (sy as f32 + 0.5) / SUBS as f32,
                        );
                        let e = [
                            edge(p[1], p[2], s),
                            edge(p[2], p[0], s),
                            edge(p[0], p[1], s),
                        ];
                        if (e[0] >= 0.0 && e[1] >= 0.0 && e[2] >= 0.0)
                            || (e[0] <= 0.0 && e[1] <= 0.0 && e[2] <= 0.0)
                        {
                            hits += 1;
                        }
                    }
                }
                if hits > 0 {
                    // 顶点色已是预乘（见 `quad::Vertex::path`），`blend` 正是按预乘合成。
                    self.blend(px, py, color, hits as f32 / (SUBS * SUBS) as f32);
                }
            }
        }
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
    let q = [p[0].abs() - half_size[0] + r, p[1].abs() - half_size[1] + r];
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
    // 多边形（`Command::Path`，矢量图标全走这里）**也画**。
    //
    // ⚠️ 过去这里是空的，于是 `paths` 整段从未被光栅化 —— 与上面 `images`
    // 那条完全同型的洞：几何统计照常增长（`path_triangles` 有数），图里却
    // 一个图标都没有。自验图是本仓唯一的像素级判据，它少画一类图元，
    // 等于把一整类缺陷永久地藏了起来。
    for chunk in batch.paths.chunks_exact(3) {
        canvas.triangle(chunk);
    }
}

#[cfg(test)]
#[path = "tests/cpu_raster_tests.rs"]
mod tests;

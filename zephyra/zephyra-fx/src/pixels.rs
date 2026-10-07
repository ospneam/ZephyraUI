//! 一批像素的描述符 + 把它写成 BMP。
//!
//! # 为什么这两个东西在后端无关的层
//!
//! 它们看起来像 D3D 的东西（[`PixelRead`] 的字段直接对应 `D3D11_MAPPED_SUBRESOURCE`
//! 的 `RowPitch`），但**没有任何一个字段是 D3D 特有的**：
//!
//! - CPU 参考光栅器产出的也是「一批 BGRA，自顶向下，可能带行距填充」；
//! - DX12 的 `Map` 读回也是同一个形状。
//!
//! 之前它们住在 `zephyra-winit::surface`（DX11 后端）里，于是 `cpu_only.rs` 为了写
//! 一张调试图，得反过来依赖 DX11 的 `surface` —— 一条从 CPU 侧指向图形后端的边。
//! 那条边会让 DX12 后端**用不了** CPU 诊断能力。

/// 一批 BGRA 像素（**自顶向下**，可能带行距填充）。
///
/// # 为什么不能假设「行距 = 宽度 × 4」
///
/// 驱动的 `RowPitch` 是**按 64 字节对齐**向上取整的，所以只有当 `宽度 × 4` 恰好
/// 是 64 的倍数时两者才相等（64 宽时 `64*4 = 256`，恰好相等；128 宽时通常就不是
/// 了）。按 `width*4` 当行距读，整张图会**斜着**串行 —— 症状是一道道对角色带。
///
/// # 像素方向
///
/// BGRA、自**顶**向下（GPU 的方向）。写 BMP 时必须逐行翻转，而不能只交换 RGB/BGR
/// —— 只换通道会得到上下颠倒的图。
#[derive(Debug, Clone)]
pub struct PixelRead {
    /// 像素数据，长度 = `row_pitch * height`。
    pub data: Vec<u8>,
    /// 每行字节数（`>= width * 4`）。
    pub row_pitch: u32,
    /// 像素的真实宽度（从资源描述读出）。
    pub width: u32,
    /// 像素的真实高度（从资源描述读出）。
    pub height: u32,
    /// 后端**声明**的宽度 —— 只用于**对比**。
    ///
    /// 与 `width` 不一致说明交换链的尺寸没跟上窗口：症状是右侧或底部一整条
    /// 纯黑边（清屏与视口都按声明尺寸走，多出来的那几条从没被清过），而
    /// **没有任何错误**。实测踩过：声明 800×600、实际 600×600，黑边宽 200。
    pub declared_width: u32,
    /// 后端**声明**的高度 —— 只用于对比。
    pub declared_height: u32,
}

impl PixelRead {
    /// 声明尺寸与真实尺寸是否一致。
    ///
    /// 不一致时**画面一定缺一条边**，而这在别处看不出来：没有 HRESULT 错误、
    /// `RenderReport` 正常、清屏也"成功"了。所以它值得单独成一个方法，
    /// 让调用方能明确判断"这一帧的边是真缺还是我读错了"。
    pub fn size_matches(&self) -> bool {
        (self.width, self.height) == (self.declared_width, self.declared_height)
    }

    /// 按行距取某一行的字节（裁到 `width * 4`）。
    pub fn row(&self, y: u32) -> &[u8] {
        let start = y as usize * self.row_pitch as usize;
        let n = (self.width as usize * 4).min(self.row_pitch as usize);
        &self.data[start..start + n]
    }

    /// 去掉行距填充后的紧凑像素（仍自顶向下）。
    pub fn to_tight(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity((self.width * self.height * 4) as usize);
        for y in 0..self.height {
            out.extend_from_slice(self.row(y));
        }
        out
    }

    /// 不同颜色的像素个数（`step` 为采样步长）。
    ///
    /// 存在的理由：判断"画没画出东西"最直接的办法不是看坐标，而是看**有多少种
    /// 颜色**。全屏纯色（无论黑白）意味着什么都没画上。
    pub fn distinct_colors(&self, step: usize) -> usize {
        let mut seen = std::collections::HashSet::new();
        for y in (0..self.height).step_by(step.max(1)) {
            let row = self.row(y);
            for x in (0..self.width as usize).step_by(step.max(1)) {
                let i = x * 4;
                if i + 4 <= row.len() {
                    seen.insert([row[i], row[i + 1], row[i + 2], row[i + 3]]);
                }
            }
        }
        seen.len()
    }
}

/// 把 [`PixelRead`] 写成 24 位 BMP。
///
/// # 参数 `encode_srgb` —— 诊断时通常要 `true`
///
/// 后缓冲是**非** sRGB 的（我们没建 `DXGI_FORMAT_*_SRGB`，清屏与混合都在线性值
/// 上做）。而 BMP/PNG 阅读器把 8 位通道**当作 sRGB** 直接显示，于是线性 0.06 的
/// 深色背景会按 0.06 的 sRGB 显示 —— **远比实际更暗**。
///
/// 本项目深色主题的 `surface` 是 `rgb(0.063, 0.071, 0.078)`（线性），编码成 sRGB
/// 后约 `0x2A` —— 深灰，在看图工具里一眼可辨。传 `false` 可核对原始数值。
///
/// # 两个必须做对的细节
///
/// 1. **行对齐**：每行按 4 字节对齐（`dst_stride = (w*3 + 3) & !3`），不足处补 0。
///    漏掉补齐会让 BMP 阅读器把后面的行全部错位。
/// 2. **上下翻转**：像素自**顶**向下，BMP 自**底**向上。写 `for y in (0..h).rev()`
///    即可。只交换 BGR 不翻转会得到上下颠倒的图 —— 那种错误很容易在"看起来有
///    东西"的时候被忽略过去。
pub fn write_bmp(path: &str, px: &PixelRead, encode_srgb: bool) -> std::io::Result<()> {
    let (w, h) = (px.width, px.height);
    // 源行距与目标行距是**两个不同的量**，混用会静默丢像素：
    //
    // - `src_stride` = 驱动的 `RowPitch`，**每像素 4 字节**（BGRA）；
    // - `dst_stride` = BMP 的行字节数，**每像素 3 字节**（BGR）再按 4 对齐。
    //
    // 我第一版把 `w*3` 当成了源行长，于是每行只读到 `w*3/4` 个像素，剩下的
    // 用 0 补齐 —— 症状是画面右侧一整条纯黑边，宽度恰好 `w/4`。本机实测
    // 800 宽时黑边正好 200 像素，与 `w - w*3/4` 完全吻合。
    //
    // 而那个错误极难自己发现：黑边看起来"像是有意留的边框"，而且日志里一切正常。
    // 只有把 `w/4` 这个数与 `w` 对上才看得出来。
    let src_stride = px.row_pitch as usize;
    let src_row_bytes = w as usize * 4;
    let dst_stride = (w as usize * 3 + 3) & !3;
    let pixel_bytes = dst_stride * h as usize;

    let mut out = Vec::with_capacity(54 + pixel_bytes);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&((54 + pixel_bytes) as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&54u32.to_le_bytes());
    // BITMAPINFOHEADER
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&(h as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&24u16.to_le_bytes()); // 24bpp BGR
    out.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
    out.extend_from_slice(&(pixel_bytes as u32).to_le_bytes());
    out.extend_from_slice(&[0u8; 16]);

    for y in (0..h as usize).rev() {
        let row = px.data.get(y * src_stride..y * src_stride + src_row_bytes);
        let Some(row) = row else { continue };
        // 按**像素**步进（4 字节），不是按 3 字节 —— 见上面两个 stride 的说明。
        let mut line = Vec::with_capacity(dst_stride);
        for p in row.chunks_exact(4) {
            let [b, g, r, _a] = [p[0], p[1], p[2], p[3]];
            let [r, g, b] = if encode_srgb {
                // 线性 → sRGB。近似用 `x^(1/2.2)`：精确的 sRGB 传输函数在低端有
                // 分段，诊断用途下这个近似足够（误差 < 1/255）。
                [enc(r), enc(g), enc(b)]
            } else {
                [r, g, b]
            };
            line.extend_from_slice(&[b, g, r]); // BGR
        }
        // 只在**行尾**补对齐字节（不足 4 字节那一处），不是补像素。
        line.resize(dst_stride, 0);
        out.extend_from_slice(&line);
    }
    std::fs::write(path, out)
}

/// 线性 [0,1] → sRGB [0,255]。
fn enc(linear: u8) -> u8 {
    let l = linear as f32 / 255.0;
    (l.powf(1.0 / 2.2) * 255.0).round().clamp(0.0, 255.0) as u8
}

/// 线性 `0..1` → sRGB 8 位通道。
///
/// 存在的理由：判断"这块区域有没有被清过"需要知道清屏色**在图上**长什么样，
/// 而不是它的线性数值 —— 深色主题的线性 0.06 显示出来是 `0x2A`，拿线性值去
/// 比对会以为"没清"。
pub fn srgb_bytes(c: zephyra_core::core::Color) -> [u8; 3] {
    let f = |v: f32| -> u8 { (v.powf(1.0 / 2.2) * 255.0).round().clamp(0.0, 255.0) as u8 };
    [f(c.r), f(c.g), f(c.b)]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 行距大于 `width * 4` 时必须按行距读，否则整张图斜着串行。
    ///
    /// 这条测试钉住的是**症状**：宽度 3、高 2、`row_pitch = 16`（64 对齐后的常见值）
    /// 的一批像素。按 `width*4 = 12` 读会把第二行读成"第一行的后半 + 第二行的头"。
    #[test]
    fn 按行距读_不是按宽度乘四() {
        // 每行 16 字节：前 12 字节 = 3 个像素（各 4 字节，用该行行号标记颜色，
        // alpha 全 255），后 4 字节是行距填充。
        const PITCH: usize = 16;
        const W: u32 = 3;
        const H: u32 = 3;
        let mk = |row: u8| -> Vec<u8> {
            let mut v = Vec::with_capacity(PITCH);
            for _ in 0..(W as usize) {
                v.extend_from_slice(&[row, row, row, 255]);
            }
            v.resize(PITCH, 0); // 行距填充必须真的存在，否则测不出「按 width*4 读」的后果
            v
        };
        let mut data = vec![0u8; PITCH * H as usize];
        for (y, mark) in [10u8, 20, 30].iter().enumerate() {
            data[y * PITCH..(y + 1) * PITCH].copy_from_slice(&mk(*mark));
        }
        let px = PixelRead {
            data,
            row_pitch: PITCH as u32,
            width: W,
            height: H,
            declared_width: W,
            declared_height: H,
        };
        for (y, want) in [(0u32, 10u8), (1, 20), (2, 30)] {
            let row = px.row(y);
            assert_eq!(row.len(), 12, "第 {y} 行应裁到 width*4 = 12 字节");
            for x in 0..W as usize {
                assert_eq!(
                    row[x * 4],
                    want,
                    "第 {y} 行第 {x} 个像素应是 {want} —— 不等说明按 width*4 当行距读了"
                );
            }
        }
        assert_eq!(px.to_tight().len(), (W * H * 4) as usize, "紧凑化后应是 w*h*4 字节");
    }

    /// 声明尺寸与真实尺寸不一致必须能查出来 —— 那意味着画面缺一条边且**不报错**。
    #[test]
    fn 尺寸不匹配_必须查得出来() {
        let px = PixelRead {
            data: vec![0; 600 * 600 * 4],
            row_pitch: 600 * 4,
            width: 600,
            height: 600,
            declared_width: 800,
            declared_height: 600,
        };
        assert!(
            !px.size_matches(),
            "声明 800×600 而实际 600×600 —— 必须判为不匹配（症状是右侧 200 像素黑边）"
        );
    }
}

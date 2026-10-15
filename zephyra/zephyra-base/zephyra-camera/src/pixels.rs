//! RGB 回退路径的 CPU 处理（RGB→RGBA + 最近邻降采样）。
//!
//! NV12 直传路径不经过此模块的逐帧处理：原始平面零拷贝切片后直接上传，
//! YUV→RGB 由渲染管线着色器完成（见 render/camera.wgsl）。

const DOWNSAMPLE_WIDTH: u32 = 854;
const DOWNSAMPLE_HEIGHT: u32 = 480;

/// NV12 帧的两个平面（零拷贝切片，共享同一底层分配）。
#[derive(Debug, Clone)]
pub struct Nv12Planes {
    /// Y 平面：w*h 字节，每像素 1 字节亮度
    pub y: bytes::Bytes,
    /// UV 平面：(w/2)*(h/2) 个交错 (U,V) 对 = w*h/2 字节
    pub uv: bytes::Bytes,
    pub width: u32,
    pub height: u32,
}

impl Nv12Planes {
    /// 把连续 NV12 缓冲切分为 Y/UV 两平面（Bytes::slice 零拷贝）。
    pub fn split(data: bytes::Bytes, w: u32, h: u32) -> Option<Nv12Planes> {
        let y_size = (w as usize) * (h as usize);
        let uv_size = y_size / 2;
        if w == 0 || h == 0 || data.len() < y_size + uv_size {
            return None;
        }
        Some(Nv12Planes {
            y: data.slice(0..y_size),
            uv: data.slice(y_size..y_size + uv_size),
            width: w,
            height: h,
        })
    }

    /// 【仅拍照时调用】单次 NV12→RGBA 转换（BT.601 limited range，与着色器一致）。
    /// 每次拍照一次 O(w*h)，不在实时路径上。
    pub fn to_rgba(&self, dst: &mut Vec<u8>) {
        let w = self.width as usize;
        let h = self.height as usize;
        dst.clear();
        dst.resize(w * h * 4, 0);
        // BT.601 limited range: Y ∈ [16,235], UV ∈ [16,240]
        // 逐像素写入预分配的切片（4 字节一组），替代旧实现的 4 次 push：
        // 同样的算术，省掉每像素 4 次长度检查与容量增长分支。
        let uv_w = w / 2;
        for (i, px) in dst.chunks_exact_mut(4).enumerate() {
            let row = i / w;
            let col = i - row * w;
            let y_val = self.y[i] as i32;
            let uv_index = (row / 2) * uv_w + (col / 2);
            let u_val = self.uv[uv_index * 2] as i32 - 128;
            let v_val = self.uv[uv_index * 2 + 1] as i32 - 128;
            let yy = (y_val - 16) * 298; // ≈255/219*256
                                         // 整数近似（右移 8 位），与浮点公式偏差 ≤1
            px[0] = ((yy + 409 * v_val) >> 8).clamp(0, 255) as u8;
            px[1] = ((yy - 100 * u_val - 208 * v_val) >> 8).clamp(0, 255) as u8;
            px[2] = ((yy + 516 * u_val) >> 8).clamp(0, 255) as u8;
            px[3] = 255;
        }
    }
}

pub struct CpuFrameProcessor {
    /// 直接写入并交出的输出缓冲（原地转换，见 [`Self::process`]）。
    output: Vec<u8>,
    last_w: u32,
    last_h: u32,
    index_size: (u32, u32, u32, u32),
    sx: Vec<u32>,
    sy: Vec<u32>,
}

impl CpuFrameProcessor {
    /// 无参构造等价于 `Default`。
    ///
    /// 刻意**不**只提供 `Default`：调用点写 `CpuFrameProcessor::new()` 更直白，
    /// 而 clippy 会要求两者等价——所以这里让 `Default` 委托给 `new()`，把
    /// 初始状态的**唯一定义**留在 `new` 里。反过来（`new` 调 `Default`）会让
    /// 默认值藏在 `#[derive(Default)]` 里，而那里的字段初值容易被误改。
    pub fn new() -> Self {
        Self {
            output: Vec::new(),
            last_w: 0,
            last_h: 0,
            index_size: (0, 0, 0, 0),
            sx: Vec::new(),
            sy: Vec::new(),
        }
    }

    fn downsample_size(src_w: u32, src_h: u32) -> (u32, u32) {
        let rgba_size = (src_w * src_h * 4) as usize;
        if rgba_size < 2 * 1024 * 1024 {
            return (src_w, src_h);
        }
        let aspect = src_w as f32 / src_h as f32;
        let dst_w = DOWNSAMPLE_WIDTH;
        let mut dst_h = DOWNSAMPLE_HEIGHT;
        if (aspect - 16.0 / 9.0).abs() > 0.01 {
            dst_h = (dst_w as f32 / aspect) as u32;
        }
        (dst_w.max(1), dst_h.max(1))
    }

    fn record_size(&mut self, dst_w: u32, dst_h: u32) {
        self.last_w = dst_w;
        self.last_h = dst_h;
    }

    pub fn last_width(&self) -> u32 {
        self.last_w
    }
    pub fn last_height(&self) -> u32 {
        self.last_h
    }

    /// RGB → RGBA + 最近邻降采样（全 CPU，无 GPU 往返）。
    ///
    /// 原地写入 `output` 并将其所有权交出（调用方包进 `Arc`）——不做
    /// `buffer → output` 的中间拷贝。降采样索引表（`sx`/`sy`）在尺寸不变时
    /// 复用。
    pub fn process(&mut self, rgb: &[u8], src_w: u32, src_h: u32) -> Vec<u8> {
        let (dst_w, dst_h) = Self::downsample_size(src_w, src_h);
        let index_size = (src_w, src_h, dst_w, dst_h);
        self.record_size(dst_w, dst_h);
        if dst_w != src_w && dst_h != src_h && self.index_size != index_size {
            self.sy = (0..dst_h)
                .map(|dy| (dy as u64 * src_h as u64 / dst_h as u64) as u32)
                .collect();
            self.sx = (0..dst_w)
                .map(|dx| (dx as u64 * src_w as u64 / dst_w as u64) as u32)
                .collect();
            self.index_size = index_size;
        }
        // 原地转换：直接把 RGBA 写进即将交出的 `output`，省掉
        // `buffer → output` 的整帧 memcpy（旧实现每帧多拷 1.6MB）。
        // `output` 被 take 走后为新空 Vec，下一帧 resize 重新分配——
        // 这是必要的：返回值的所有权要交给调用方包进 Arc。
        let needed = (dst_w * dst_h * 4) as usize;
        self.output.clear();
        self.output.resize(needed, 0);
        let buf = &mut self.output;
        let sx = &self.sx;
        let sy = &self.sy;

        if dst_w == src_w && dst_h == src_h {
            for (i, px) in buf.chunks_exact_mut(4).enumerate() {
                let s = i * 3;
                px[0] = rgb[s];
                px[1] = rgb[s + 1];
                px[2] = rgb[s + 2];
                px[3] = 255;
            }
        } else {
            for (dy, row) in buf.chunks_exact_mut((dst_w * 4) as usize).enumerate() {
                let src_y = sy[dy] as usize;
                let base = src_y * src_w as usize;
                for (dx, px) in row.chunks_exact_mut(4).enumerate() {
                    let s = (base + sx[dx] as usize) * 3;
                    px[0] = rgb[s];
                    px[1] = rgb[s + 1];
                    px[2] = rgb[s + 2];
                    px[3] = 255;
                }
            }
        }

        std::mem::take(&mut self.output)
    }
}

impl Default for CpuFrameProcessor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "tests/pixels_tests.rs"]
mod tests;

//! 屏幕坐标 ↔ 内容坐标变换（f32 全程，零分配，`Copy` 值）。
//!
//! 变换模型与渲染侧（viewink `ViewTransform` / `Globals`）一致：
//!
//! ```text
//! canvas = (screen - offset) / scale
//! screen = canvas * scale + offset
//! ```

/// 视口变换：屏幕坐标 ↔ 内容坐标（12 字节 `Copy`）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    pub offset_x: f32,
    pub offset_y: f32,
    pub scale: f32,
}

impl Viewport {
    /// 缩放下限（滚轮 / 捏合共用）。
    pub const MIN_SCALE: f32 = 0.1;
    /// 缩放上限（滚轮 / 捏合共用）。
    pub const MAX_SCALE: f32 = 10.0;

    #[inline]
    pub const fn new(offset_x: f32, offset_y: f32, scale: f32) -> Self {
        Self {
            offset_x,
            offset_y,
            scale,
        }
    }

    /// 屏幕 → 内容。
    #[inline]
    pub fn screen_to_canvas(&self, x: f32, y: f32) -> (f32, f32) {
        (
            (x - self.offset_x) / self.scale,
            (y - self.offset_y) / self.scale,
        )
    }

    /// 内容 → 屏幕。
    #[inline]
    pub fn canvas_to_screen(&self, x: f32, y: f32) -> (f32, f32) {
        (x * self.scale + self.offset_x, y * self.scale + self.offset_y)
    }

    /// 捏合锚点换算：屏幕中点 → 内容锚点。
    ///
    /// 除法换倒数（1 次除法 + 2 次乘法）；scale 以 0.001 下限防退化，
    /// 与旧行为一致。
    #[inline]
    pub fn pinch_anchor(&self, center_x: f32, center_y: f32) -> (f32, f32) {
        let inv = 1.0 / self.scale.max(0.001);
        (
            (center_x - self.offset_x) * inv,
            (center_y - self.offset_y) * inv,
        )
    }

    /// 应用捏合增量：以屏幕点 `(center_x, center_y)` 为视觉锚点缩放，
    /// 保持内容锚点 `(anchor_cx, anchor_cy)` 始终位于该屏幕点之下。
    ///
    /// 缩放结果钳位到 [`Self::MIN_SCALE`]..=[`Self::MAX_SCALE`]。
    #[inline]
    pub fn apply_pinch(
        &mut self,
        center_x: f32,
        center_y: f32,
        scale_ratio: f32,
        anchor_cx: f32,
        anchor_cy: f32,
    ) {
        let new_scale = (self.scale * scale_ratio).clamp(Self::MIN_SCALE, Self::MAX_SCALE);
        self.offset_x = center_x - anchor_cx * new_scale;
        self.offset_y = center_y - anchor_cy * new_scale;
        self.scale = new_scale;
    }
}

#[cfg(test)]
#[path = "tests/viewport_tests.rs"]
mod tests;

//! 已注册纹理的**名字集合**。
//!
//! # 为什么共享层只存名字，不存 GPU 资源
//!
//! 原先这里叫 `TextureTable`，里面装的是 `ID3D11ShaderResourceView`。但真正**与
//! 后端无关**的只有一件事：「这个名字注册过没有」。
//!
//! 拆开的好处是 [`crate::cpu_only::CpuHost`] 也能有纹理表了。它以前做不到 —— 它
//! 不碰 GPU，却被迫持有一个装 D3D11 COM 对象的表，于是：
//!
//! - 这个离线宿主住在 D3D11 后端 crate 里（`zephyra_fx` 之前是
//!   `zephyra-winit::cpu_only`）；
//! - D3D12 后端**用不了**它，因为它拿不到 D3D11 的纹理表。
//!
//! 而 CPU 侧对纹理的全部需求就是 `contains()` —— 用它判断某张图该画还是该跳过
//! （见 [`crate::quad::expand`] 的 `missing_textures` 计数）。
//!
//! # 与后端侧的关系
//!
//! 各后端自己的纹理表（D3D11 的 `zephyra_winit::texture::TextureTable`）持有一个
//! [`TextureNames`] **加上** GPU 资源对象。要新增纹理时两处一起改 —— 那一步
//! 本来就只能在后端里做，因为只有后端能建 GPU 资源。

use std::collections::HashSet;

/// 已注册的纹理名字。
#[derive(Debug, Clone, Default)]
pub struct TextureNames {
    names: HashSet<String>,
}

impl TextureNames {
    /// 空表。
    pub fn new() -> Self {
        Self::default()
    }

    /// 这个名字注册过吗。
    ///
    /// [`crate::quad::expand`] 用它决定图像命令能否画出来 —— 没注册就跳过并计入
    /// [`crate::RenderReport::missing_textures`]，**不静默**：症状是「图标不显示」，
    /// 而那与「纹理加载失败」看起来一模一样。
    pub fn contains(&self, name: &str) -> bool {
        self.names.contains(name)
    }

    /// 注册一个名字。重复注册返回 `false`（值已存在）。
    pub fn insert(&mut self, name: impl Into<String>) -> bool {
        self.names.insert(name.into())
    }

    /// 注销一个名字。原先存在返回 `true`。
    pub fn remove(&mut self, name: &str) -> bool {
        self.names.remove(name)
    }

    /// 已注册的名字个数。
    pub fn len(&self) -> usize {
        self.names.len()
    }

    /// 一个都没注册？
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    /// 迭代全部名字。
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.names.iter().map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 注册过才_contains() {
        let mut n = TextureNames::new();
        assert!(n.is_empty());
        assert!(!n.contains("logo.png"), "未注册的不能算存在");
        assert!(n.insert("logo.png"));
        assert!(n.contains("logo.png"));
        assert_eq!(n.len(), 1);
        // 重复插入返回 false，且不改变计数。
        assert!(!n.insert("logo.png"));
        assert_eq!(n.len(), 1, "重复插入不应重复计数");
    }

    #[test]
    fn 注销后_contains_变假() {
        let mut n = TextureNames::new();
        n.insert("a.png");
        assert!(n.remove("a.png"));
        assert!(!n.contains("a.png"));
        assert!(!n.remove("a.png"), "注销不存在的应返回 false");
    }
}

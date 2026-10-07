//! 声明：一棵 [`Element`] 树描述「我想要什么」。
//!
//! 每次「重建」产生一棵，轻量、可丢弃。它与 [`crate::node::Node`]（保留状态）的
//! 配对规则写在 [`crate::diff`]。
//!
//! # 为什么刻意不给「万能构造器」
//!
//! 刻意**不**提供通用构造器 + `Vec<Element>` 字段：那等于把「什么都能塞」的
//! 袋子暴露给所有代码，[`crate::id::ElementKind`] 也就无从约束。真实 API 由本
//! crate 的构造函数或宏提供，字段保持私有。

use crate::id::{ElementKey, NodeTag};
use crate::property::{PropertyKey, PropertyStore, PropertyValue};

/// 声明式元素。
pub struct Element {
    tag: NodeTag,
    key: Option<ElementKey>,
    props: PropertyStore,
    children: Vec<Element>,
}

impl Element {
    /// 创建一个元素（供本 crate 的构造函数与控件层使用）。
    pub fn new(tag: NodeTag) -> Self {
        Self {
            tag,
            key: None,
            props: PropertyStore::new(),
            children: Vec::new(),
        }
    }

    /// 设置属性。
    ///
    /// `key` 必须是 `&'static` —— 属性键的身份就是它的地址（见
    /// [`crate::property::PropertyKey`]），而 [`PropertyStore::set_local`] 把这份
    /// 身份原样带进 [`crate::property::PropertyChange`]。收 `&'static` 让这条约束
    /// 由类型保证，而不是靠"调用方记得用 static 定义"。
    ///
    /// ⚠ `set_local` 的返回值**这里直接丢掉**。它只为 [`crate::node::Node::set_prop`]
    /// 存在（那条路径要靠它决定标 `Layout` 还是 `Paint` 脏）。声明树不需要 —— 声明树
    /// 本来就是"每帧重建、无状态"的。
    pub fn with(mut self, key: &'static PropertyKey, value: PropertyValue) -> Self {
        self.props.set_local(key, value);
        self
    }

    /// 设置子节点。
    pub fn with_children(mut self, children: impl IntoIterator<Item = Element>) -> Self {
        self.children = children.into_iter().collect();
        self
    }

    /// 设置稳定 key（让 diff 按身份而非位置配对）。
    pub fn with_key(mut self, key: impl Into<ElementKey>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// 类型标签。
    pub fn tag(&self) -> NodeTag {
        self.tag
    }

    /// 稳定 key。
    pub fn key(&self) -> Option<&ElementKey> {
        self.key.as_ref()
    }

    /// 属性表。
    pub fn props(&self) -> &PropertyStore {
        &self.props
    }

    /// 子节点。
    pub fn children(&self) -> &[Element] {
        &self.children
    }

    /// 是否为「无子树」的叶子元素（diff 时可跳过结构比对）。
    pub fn is_leaf(&self) -> bool {
        self.children.is_empty()
    }
}

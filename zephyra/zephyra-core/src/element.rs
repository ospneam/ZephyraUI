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
    /// 样式分类名。**只**给样式表的选择器用，与 [`Self::key`] 无关。
    ///
    /// # 为什么单独一份，不复用 `key`
    ///
    /// `key` 是**事件路由**的身份（哪个按钮被点了），`style` 是**外观**的分类
    /// （这一组元素长什么样）。两者用途不同、变更时机也不同：
    /// key 一辈子不变，style 会随状态切（`danger` / `compact`）。
    ///
    /// # 为什么必须是字面量，不能带判断
    ///
    /// `.style(if x { "a" })` = 源码在挑外观，那是立规要禁的。
    /// 判断写在外面、只把结论传进来：
    ///
    /// ```rust,ignore
    /// let name = if low { "danger" } else { "normal" };   // 判断在这里
    /// Button::new().style(name)                             // 这里只有一个词
    /// ```
    styles: Vec<&'static str>,
    props: PropertyStore,
    children: Vec<Element>,
}

impl Element {
    /// 创建一个元素（供本 crate 的构造函数与控件层使用）。
    pub fn new(tag: NodeTag) -> Self {
        Self {
            tag,
            key: None,
            styles: Vec::new(),
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

    /// 加一个样式分类名（对应样式表里的 `.名字`）。
    ///
    /// 这是**源码里唯一能碰外观的地方**——但它只给一个词，不给任何值。
    /// 「这个名字长什么样」全部在样式表里。
    pub fn style(mut self, name: &'static str) -> Self {
        self.styles.push(name);
        self
    }

    /// 一次加多个样式分类名。
    ///
    /// 语义同 [`Self::style`] 连调多次，只是省几次调用。
    pub fn styles(mut self, names: impl IntoIterator<Item = &'static str>) -> Self {
        self.styles.extend(names);
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

    /// 样式分类名。
    pub fn style_names(&self) -> &[&'static str] {
        &self.styles
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

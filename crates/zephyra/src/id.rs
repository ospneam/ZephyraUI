//! 身份：元素按什么配对。
//!
//! [`crate::diff`] 要回答的唯一问题是「新的这个是不是旧的同一个」。答案只靠两个
//! 东西：[`NodeTag`]（类型能否就地复用）与 [`ElementKey`]（同一类型里的哪一个）。
//!
//! 它们住在自己的模块里，而不是并进 [`crate::element`] 或 [`crate::node`] ——
//! 因为 `Element`（声明）与 `Node`（保留）**双方**都要用这两个概念，而它们不属于
//! 其中任何一方。把它们塞进其中一个，剩下那个就得反向依赖，那不叫分层。

/// 节点标签（用于 diff 配对）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeTag {
    /// 文本节点。
    Text,
    /// 图像节点。
    Image,
    /// 布局容器。
    Container,
    /// 控件（具体类型由控件层定义）。
    Control,
}

/// 元素类型标签。
///
/// diff 用它判断「能否就地复用」。用枚举而非 `TypeId` 是为了让「宿主定义了自定义
/// 元素类型」这件事在类型层面就是非法的——自定义元素应当是一个实现了
/// [`ElementKind`] 的新类型，而不是在字符串里登记。
pub trait ElementKind {
    /// 该元素对应的 Node 类型标签。
    fn node_tag(&self) -> NodeTag;
}

/// 元素身份。
///
/// # 为什么 `Name` 是 `&'static str` 而不是 `Arc<str>`
///
/// `Arc<str>` 在这里**只**提供共享所有权，而没有任何消费者需要它：
///
/// - 所有来源都是 `&'static str`（`with_key(impl Into<ElementKey>)` 的实际入参
///   全是字面量或 `'static` 常量）—— 字符串本来就活到进程结束，refcount 增删
///   纯属白做；
/// - 消费者只比内容（`PartialEq` / `Hash`），不持有、不跨帧保存；
/// - 而声明树**每帧重建**：demo 的 45 个节点 = 每帧 45 次 `Arc::from` 堆分配，
///   分配完立刻随树丢掉。这是"算出来只是为了扔掉"的又一个实例。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ElementKey {
    /// 整数 id。
    Id(u64),
    /// 静态字符串 id。
    Name(&'static str),
}

impl From<&'static str> for ElementKey {
    fn from(s: &'static str) -> Self {
        ElementKey::Name(s)
    }
}

impl From<u64> for ElementKey {
    fn from(v: u64) -> Self {
        ElementKey::Id(v)
    }
}

impl ElementKey {
    /// 名字形式的 key（`Id` 形式返回 `None`）。
    ///
    /// 供测试与调试断言用。宿主做事件路由时应当比较整个 key（`PartialEq`）而不是名字——
    /// 字符串比较在每次输入事件上都是不必要的开销。
    pub fn as_name(&self) -> Option<&'static str> {
        match self {
            ElementKey::Name(n) => Some(n),
            ElementKey::Id(_) => None,
        }
    }
}

impl std::fmt::Display for ElementKey {
    /// 人读形式。**只用于日志与断言**，不要拿它做身份比较 ——
    /// 身份靠 `PartialEq`（`Id` 是整数，不是字符串）。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ElementKey::Id(v) => write!(f, "#{v}"),
            ElementKey::Name(n) => write!(f, "{n}"),
        }
    }
}

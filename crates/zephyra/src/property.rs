//! 属性系统：属性的定义、存储、继承、变更通知与样式化。
//!
//! # 为什么这是地基里最先要定的一块
//!
//! 控件、主题、样式三者共同依赖它：
//!
//! - 控件需要「属性变了要重排/重绘」的通知；
//! - 主题需要「属性可以按资源键查值」（`ThemeResource`）；
//! - 样式需要「属性可以按选择器覆盖」（`{ThemeResource AccentBrush}`）。
//!
//! 如果三者各自实现一遍，就得到三套互不相通的失效传播——而 UI bug 里最难查的
//! 恰恰是「改了颜色但那一块没更新」。
//!
//! # 对应 WinUI 的 DependencyProperty
//!
//! 借鉴三点：
//!
//! 1. **属性有身份，不只是值。** 同一个 `PropertyKey` 在整棵树上指向同一份
//!    语义，因此可以按 key 建索引（样式匹配、变更广播都靠它）。
//! 2. **继承链。** 未在本地设置的属性沿父链向上查找，根上再回落到主题默认值。
//! 3. **变更优先级。** 本地值 > 样式/模板值 > 继承值 > 主题默认值。这个顺序
//!    让「父节点设一次影响子树，子节点可局部覆盖」成为默认行为。
//!
//! # 类型擦除
//!
//! [`PropertyValue`] 擦除具体类型。理由是属性表要在每节点上开一个 `HashMap`，
//! 而泛型会把这个 map 变成 `HashMap<K, HashMap<String, Box<dyn Any>>>`——两次
//! 哈希加一次 `downcast`。「按名称而非按类型查属性」也是样式系统的实际需求。

use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;

/// 属性键：属性的全局身份。
///
/// 一个 [`PropertyKey`] 对应全树唯一的一份语义（类型 + 默认值 + 是否可继承 +
/// 变更回调）。用 `&'static str` 作名字是为了能写调试输出；相等性靠地址，
/// 所以定义处必须是 `static`/`const` 提升。
pub struct PropertyKey {
    name: &'static str,
    default: PropertyValue,
    inherits: bool,
}

impl PropertyKey {
    /// 定义一个不可继承的属性。
    pub const fn new(name: &'static str, default: PropertyValue) -> Self {
        Self {
            name,
            default,
            inherits: false,
        }
    }

    /// 定义一个**可继承**的属性（未在本地设置时沿父链查找）。
    pub const fn inheritable(name: &'static str, default: PropertyValue) -> Self {
        Self {
            name,
            default,
            inherits: true,
        }
    }

    /// 属性名（调试与序列化用）。
    pub const fn name(&self) -> &'static str {
        self.name
    }

    /// 是否可继承。
    pub const fn inherits(&self) -> bool {
        self.inherits
    }

    /// 主题默认值。
    pub fn default_value(&self) -> &PropertyValue {
        &self.default
    }
}

impl std::fmt::Debug for PropertyKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PropertyKey({})", self.name)
    }
}

/// 类型擦除的属性值。
///
/// # 为什么是枚举而不是 `Box<dyn Any>`
///
/// 属性**定义**必须是 `const`——它们是编译期常量，每个节点/控件/样式表引用同
/// 一个定义。而 `Box<dyn Any>` 需要堆分配，无法在 `const` 上下文构造。
///
/// 所以常见类型走枚举变体（可 `const` 构造、零分配），只有真正需要自建类型时才
/// 落到 `Custom` 变体。这也顺带让相等性可以手写而不必塞一个函数指针。
#[derive(Clone)]
pub enum PropertyValue {
    /// `f32`。
    Float(f32),
    /// `i32`。
    Int(i32),
    /// `bool`。
    Bool(bool),
    /// UTF-8 字符串。
    Str(Arc<str>),
    /// 颜色。
    Color(crate::core::Color),
    /// **资源键引用**：存键而非字面值，主题切换时只需重解析这些键。
    ///
    /// 这是 `ThemeResource` 能做到「切主题不遍历控件树」的关键。
    Resource(Arc<str>),
    /// 任意自定义类型。
    Custom {
        inner: Arc<dyn Any + Send + Sync>,
        /// 类型擦除后的相等判断（存 fn 指针，避免每次构造都分配闭包）。
        eq: fn(&dyn Any, &dyn Any) -> bool,
    },
}

impl PropertyValue {
    /// 构造 `f32` 属性值（`const` 可用）。
    pub const fn float(v: f32) -> Self {
        PropertyValue::Float(v)
    }

    /// 构造 `i32` 属性值（`const` 可用）。
    pub const fn int(v: i32) -> Self {
        PropertyValue::Int(v)
    }

    /// 构造 `bool` 属性值（`const` 可用）。
    pub const fn bool(v: bool) -> Self {
        PropertyValue::Bool(v)
    }

    /// 构造颜色属性值（`const` 可用）。
    pub const fn color(v: crate::core::Color) -> Self {
        PropertyValue::Color(v)
    }

    /// 构造任意自定义属性值。
    pub fn custom<T: Any + Send + Sync + Clone + PartialEq + 'static>(value: T) -> Self {
        PropertyValue::Custom {
            inner: Arc::new(value),
            eq: |a, b| a.downcast_ref::<T>() == b.downcast_ref::<T>(),
        }
    }

    /// 是否是资源键引用。
    pub fn is_resource(&self) -> bool {
        matches!(self, PropertyValue::Resource(_))
    }

    /// 资源键。
    pub fn resource_key(&self) -> Option<&str> {
        match self {
            PropertyValue::Resource(k) => Some(k),
            _ => None,
        }
    }

    /// 向下转型为具体类型。
    ///
    /// 返回 `None` 有两种原因：类型不符，或该值是尚未解析的资源键 / 尚未解析成
    /// 具体类型的其他变体。前者是真错，后者是正常中间态。
    pub fn get<T: Any + Clone + 'static>(&self) -> Option<T> {
        let any: &dyn Any = match self {
            PropertyValue::Float(v) => v,
            PropertyValue::Int(v) => v,
            PropertyValue::Bool(v) => v,
            PropertyValue::Str(v) => v,
            PropertyValue::Color(v) => v,
            PropertyValue::Custom { inner, .. } => &**inner,
            PropertyValue::Resource(_) => return None,
        };
        any.downcast_ref::<T>().cloned()
    }
}

/// 相等性逐变体比较；`Custom` 用其携带的类型擦除比较函数。
///
/// diff 靠它判断属性是否真的变了——这是「UI 静止时零重建」的基础。
impl PartialEq for PropertyValue {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (PropertyValue::Float(a), PropertyValue::Float(b)) => a == b,
            (PropertyValue::Int(a), PropertyValue::Int(b)) => a == b,
            (PropertyValue::Bool(a), PropertyValue::Bool(b)) => a == b,
            (PropertyValue::Str(a), PropertyValue::Str(b)) => a == b,
            (PropertyValue::Color(a), PropertyValue::Color(b)) => a == b,
            (PropertyValue::Resource(a), PropertyValue::Resource(b)) => a == b,
            (
                PropertyValue::Custom { inner: ai, eq },
                PropertyValue::Custom { inner: bi, eq: eqi },
            ) => {
                ai.type_id() == bi.type_id()
                    && eq(&**ai, &**bi)
                    && std::ptr::eq(*eq as *const (), *eqi as *const ())
            }
            _ => false,
        }
    }
}

impl std::fmt::Debug for PropertyValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PropertyValue::Float(v) => write!(f, "{v}"),
            PropertyValue::Int(v) => write!(f, "{v}"),
            PropertyValue::Bool(v) => write!(f, "{v}"),
            PropertyValue::Str(v) => write!(f, "{v:?}"),
            PropertyValue::Color(v) => write!(f, "{v:?}"),
            PropertyValue::Resource(k) => write!(f, "Resource({k})"),
            PropertyValue::Custom { .. } => write!(f, "Custom"),
        }
    }
}

/// 属性解析后的来源。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueSource {
    /// 本地显式设置（`element.prop(x)`）。
    Local,
    /// 由样式/模板设置。
    Styled,
    /// 从祖先继承。
    Inherited,
    /// 主题默认值。
    Default,
}

/// 一次属性变更。
#[derive(Debug, Clone)]
pub struct PropertyChange {
    /// 哪个属性变了。
    pub key: Arc<PropertyKey>,
    /// 旧值。
    pub old: Option<PropertyValue>,
    /// 新值。
    pub new: Option<PropertyValue>,
}

impl PropertyChange {
    /// 变更是否影响布局（而非仅绘制）。
    ///
    /// 属性系统据此决定重排还是只重绘——把 `Background` 与 `Margin` 同等对待
    /// 会让改个颜色触发整棵子树重排，是 UI 卡顿最常见的来源。
    pub fn affects_layout(&self) -> bool {
        self.key.name().starts_with("Layout.")
            || matches!(
                self.key.name(),
                "Margin" | "Padding" | "Width" | "Height" | "Size"
            )
    }
}

/// 单个节点上的属性表。
///
/// 键为属性**定义**的地址，值只存「非默认」的那些——默认回落由
/// [`resolve`] 沿继承链完成，因此绝大多数节点的这张表是空的。
#[derive(Debug, Default, Clone)]
pub struct PropertyStore {
    local: HashMap<usize, PropertyValue>,
    styled: HashMap<usize, PropertyValue>,
}

impl PropertyStore {
    /// 空属性表。
    pub fn new() -> Self {
        Self::default()
    }

    /// 设置本地值（最高优先级）。
    pub fn set_local(&mut self, key: &PropertyKey, value: PropertyValue) -> Option<PropertyChange> {
        let old = self.local.insert(key as *const _ as usize, value.clone());
        Some(PropertyChange {
            key: Arc::new(PropertyKey::new(key.name, key.default.clone())),
            old,
            new: Some(value),
        })
    }

    /// 设置样式值（低于本地值）。
    pub fn set_styled(
        &mut self,
        key: &PropertyKey,
        value: PropertyValue,
    ) -> Option<PropertyChange> {
        let old = self.styled.insert(key as *const _ as usize, value.clone());
        Some(PropertyChange {
            key: Arc::new(PropertyKey::new(key.name, key.default.clone())),
            old,
            new: Some(value),
        })
    }

    /// 清除本地值（回落到样式/继承/默认）。
    pub fn clear_local(&mut self, key: &PropertyKey) {
        self.local.remove(&(key as *const _ as usize));
    }

    /// 本地是否显式设置了该属性。
    pub fn is_locally_set(&self, key: &PropertyKey) -> bool {
        self.local.contains_key(&(key as *const _ as usize))
    }

    /// 内部 id（`PropertyKey` 的地址）。diff 对账时用它避免借用键定义。
    pub fn id_of(key: &PropertyKey) -> usize {
        key as *const _ as usize
    }

    /// 本地设置的全部条目，供 diff 对账。
    pub fn iter_local(&self) -> impl Iterator<Item = (usize, &PropertyValue)> {
        self.local.iter().map(|(k, v)| (*k, v))
    }

    /// 按内部 id 写入本地值，返回**值是否真的变了**。
    ///
    /// diff 用返回值决定要不要标脏——「值没变就别标脏」是 UI 静止时零重建的
    /// 第一道闸门。
    pub fn set_local_by_id(&mut self, id: usize, value: PropertyValue) -> bool {
        match self.local.get_mut(&id) {
            Some(existing) if *existing == value => false,
            Some(existing) => {
                *existing = value;
                true
            }
            None => {
                self.local.insert(id, value);
                true
            }
        }
    }

    /// 本节点能提供的值（不含继承与默认回落）。
    pub fn lookup(&self, key: &PropertyKey) -> Option<(&PropertyValue, ValueSource)> {
        let id = Self::id_of(key);
        if let Some(v) = self.local.get(&id) {
            return Some((v, ValueSource::Local));
        }
        if let Some(v) = self.styled.get(&id) {
            return Some((v, ValueSource::Styled));
        }
        None
    }
}

/// 按优先级解析属性值。
///
/// 顺序即 WinUI 的变更优先级：**本地 > 样式 > 继承 > 主题默认值**。宿主在
/// `tree` 模块沿父链收集继承值后调用本函数。
///
/// 三个入参共享同一生命周期：属性键通常是 `&'static`，因此实践中总是满足。
pub fn resolve<'a>(
    store: &'a PropertyStore,
    key: &'a PropertyKey,
    inherited: Option<&'a PropertyValue>,
) -> (&'a PropertyValue, ValueSource) {
    if let Some((v, src)) = store.lookup(key) {
        return (v, src);
    }
    if key.inherits() {
        if let Some(v) = inherited {
            return (v, ValueSource::Inherited);
        }
    }
    (key.default_value(), ValueSource::Default)
}

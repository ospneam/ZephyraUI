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
//! [`PropertyValue`] 擦除具体类型。理由是属性表要按 key 存不同类型的值，而泛型会
//! 把它变成按 key 再分派到按类型的一张表 —— 多一层间接，且每种类型一份。
//! 「按名称而非按类型查属性」也是样式系统的实际需求。

use std::any::Any;
use std::sync::Arc;

/// 属性键：属性的全局身份。
///
/// 一个 [`PropertyKey`] 对应全树唯一的一份语义（类型 + 默认值 + 是否可继承 +
/// 变更是否影响布局）。用 `&'static str` 作名字是为了能写调试输出；相等性靠地址，
/// 所以定义处必须是 `static`/`const` 提升。
pub struct PropertyKey {
    name: &'static str,
    default: PropertyValue,
    inherits: bool,
    /// 改这个属性要不要**重排**（而不只是重绘）。
    ///
    /// ⚠️ 它在**构造时**定死，不是运行时按名字算出来的。早期版本用
    /// `name_affects_layout(name)`——一个靠字符串前缀 + 名单的函数。那样「忘了
    /// 声明」会是一次**静默的布局错乱**（拿不到 `Dirty::Layout` → 不重排 → 布局盒是
    /// 旧尺寸、字形按新字号画），零报错。改成字段后，忘了声明**编译不过**。
    layout: bool,
}

impl PropertyKey {
    /// 定义一个**不可继承**、**只重绘**的属性。
    pub const fn new(name: &'static str, default: PropertyValue) -> Self {
        Self {
            name,
            default,
            inherits: false,
            layout: false,
        }
    }

    /// 定义一个**可继承**、**只重绘**的属性（未在本地设置时沿父链查找）。
    pub const fn inheritable(name: &'static str, default: PropertyValue) -> Self {
        Self {
            name,
            default,
            inherits: true,
            layout: false,
        }
    }

    /// 定义一个**不可继承**、**影响布局**的属性。
    ///
    /// 改变它必须重排——比如 `Width` / `Padding`。**拿不准就选这个**：多重排一次
    /// 只是慢一点，少重排一次会让布局盒与内容错位（零报错）。
    pub const fn layout(name: &'static str, default: PropertyValue) -> Self {
        Self {
            name,
            default,
            inherits: false,
            layout: true,
        }
    }

    /// 定义一个**可继承**、**影响布局**的属性。
    pub const fn inheritable_layout(name: &'static str, default: PropertyValue) -> Self {
        Self {
            name,
            default,
            inherits: true,
            layout: true,
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

    /// 改这个属性要不要重排。
    ///
    /// 属性系统据此决定重排还是只重绘——把 `Background` 与 `Width` 同等对待会让
    /// 改个颜色触发整棵子树重排，是 UI 卡顿最常见的来源。
    pub const fn affects_layout(&self) -> bool {
        self.layout
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
    /// `'static` 字符串常量。
    ///
    /// 与 [`PropertyValue::Str`] 分开是因为后者持 `Arc`（运行时构造），无法作为
    /// `const` 默认值。属性定义里的"空串"占位只能用这个。
    StaticStr(&'static str),
    /// 颜色。
    Color(crate::core::Color),
    /// 布局方向（`layout::Direction`）。
    Direction(crate::layout::Direction),
    /// 交叉轴对齐（`layout::AlignItems`）。
    CrossAlign(crate::layout::AlignItems),
    /// 主轴对齐（`layout::MainAxisAlign`）。
    MainAlign(crate::layout::MainAxisAlign),
    /// 合成层（`paint::Layer`）。
    Layer(crate::paint::Layer),
    /// 滤镜链（`filter::FilterChain`）。
    ///
    /// 刻意**不**走 [`PropertyValue::Custom`]：滤镜要进每帧产出的 `Command`，
    /// 热路径上多一次 `downcast_ref` 不可接受；而它有固定词汇表，所以枚举变体
    /// 既是零分配的，也能参与手写的相等判断。
    ///
    /// 默认值是空链（`FilterChain::none()`，`const` 可用），而 [`Clone`] + 逐字段
    /// 比较让 diff 能准确判出"滤镜变了" —— 这是效果失效传播的基础。
    Filter(crate::filter::FilterChain),
    /// 混合模式（`filter::BlendMode`）。
    BlendMode(crate::filter::BlendMode),
    /// 纹理内的归一化矩形（`core::UvRect`）。
    UvRect(crate::core::UvRect),
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

    /// 构造 `'static` 字符串属性值（`const` 可用）。
    pub const fn static_str(v: &'static str) -> Self {
        PropertyValue::StaticStr(v)
    }

    /// 构造布局方向属性值（`const` 可用）。
    pub const fn direction(v: crate::layout::Direction) -> Self {
        PropertyValue::Direction(v)
    }

    /// 构造交叉轴对齐属性值（`const` 可用）。
    pub const fn cross_align(v: crate::layout::AlignItems) -> Self {
        PropertyValue::CrossAlign(v)
    }

    /// 构造主轴对齐属性值（`const` 可用）。
    pub const fn main_align(v: crate::layout::MainAxisAlign) -> Self {
        PropertyValue::MainAlign(v)
    }

    /// 构造合成层属性值（`const` 可用）。
    pub const fn layer(v: crate::paint::Layer) -> Self {
        PropertyValue::Layer(v)
    }

    /// 构造滤镜链属性值（空链可`const`）。
    pub const fn filter(v: crate::filter::FilterChain) -> Self {
        PropertyValue::Filter(v)
    }

    /// 构造混合模式属性值（`const` 可用）。
    pub const fn blend_mode(v: crate::filter::BlendMode) -> Self {
        PropertyValue::BlendMode(v)
    }

    /// 构造纹理 uv 矩形属性值（`const` 可用）。
    pub const fn uv_rect(v: crate::core::UvRect) -> Self {
        PropertyValue::UvRect(v)
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
            PropertyValue::StaticStr(v) => v,
            PropertyValue::Color(v) => v,
            PropertyValue::Direction(v) => v,
            PropertyValue::CrossAlign(v) => v,
            PropertyValue::MainAlign(v) => v,
            PropertyValue::Layer(v) => v,
            PropertyValue::Filter(v) => v,
            PropertyValue::BlendMode(v) => v,
            PropertyValue::UvRect(v) => v,
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
            (PropertyValue::StaticStr(a), PropertyValue::StaticStr(b)) => a == b,
            (PropertyValue::Color(a), PropertyValue::Color(b)) => a == b,
            (PropertyValue::Direction(a), PropertyValue::Direction(b)) => a == b,
            (PropertyValue::CrossAlign(a), PropertyValue::CrossAlign(b)) => a == b,
            (PropertyValue::MainAlign(a), PropertyValue::MainAlign(b)) => a == b,
            (PropertyValue::Layer(a), PropertyValue::Layer(b)) => a == b,
            (PropertyValue::Filter(a), PropertyValue::Filter(b)) => a == b,
            (PropertyValue::BlendMode(a), PropertyValue::BlendMode(b)) => a == b,
            (PropertyValue::UvRect(a), PropertyValue::UvRect(b)) => a == b,
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
            PropertyValue::StaticStr(v) => write!(f, "{v:?}"),
            PropertyValue::Color(v) => write!(f, "{v:?}"),
            PropertyValue::Direction(v) => write!(f, "{v:?}"),
            PropertyValue::CrossAlign(v) => write!(f, "{v:?}"),
            PropertyValue::MainAlign(v) => write!(f, "{v:?}"),
            PropertyValue::Layer(v) => write!(f, "{v:?}"),
            // 调试输出要能看出"有几个滤镜"，否则属性 diff 失败时无从下手。
            PropertyValue::Filter(v) => write!(f, "Filter({} 项)", v.filters().len()),
            PropertyValue::BlendMode(v) => write!(f, "{v:?}"),
            PropertyValue::UvRect(v) => write!(f, "{v:?}"),
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
    ///
    /// # 为什么是 `&'static PropertyKey` 而不是 `Arc<PropertyKey>`
    ///
    /// 属性键**本来就是 `static`** —— 它的身份就是地址（见 [`PropertyKey`] 模块
    /// 文档：「相等性靠地址，所以定义处必须是 `static`/`const` 提升」）。所以这里
    /// 完全可以借一个 `&'static`，不必复制。
    ///
    /// 原先存 `Arc<PropertyKey>`，于是 `set_local` 每次都做一次
    /// `Arc::new(PropertyKey::new(key.name, key.default.clone()))` ——
    /// **一次堆分配加一次默认值克隆**，只为让调用方读一个 `name()`。
    ///
    /// 而这两个字段当时在本文件里**一个都没被读过**：
    ///
    /// - `name` 的唯一读者 [`Self::affects_layout`] 只要名字；
    /// - `default` 的读者 [`PropertyKey::default_value`] 已经能从 `key` 直接拿。
    ///
    /// 声明树每个节点要 `.with()` 五到八个属性，于是 demo 一帧约 200 次这种分配，
    /// 而它们产出的东西**当场就被丢掉**（见 [`crate::element::Element::with`]，
    /// 它忽略 `set_local` 的返回值）。这类"算出来只是为了扔掉"的浪费功能上完全
    /// 正确，所以任何功能测试都抓不到它。
    pub key: &'static PropertyKey,
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
        self.key.affects_layout()
    }
}

/// 属性名是否属于「影响布局」的一类。
///
/// # ⚠️ 只在测试里用，生产代码一律走 [`PropertyKey::affects_layout`]
///
/// 这个函数是 B0 时期的实现——判定看**属性名**（一串硬编码名单）。B1 已把判定搬进
/// [`PropertyKey`] 的 `layout` 字段，由**构造器**在定义时定死，于是「忘了声明」
/// 从一次静默的布局错位变成**编译错误**。
///
/// 保留它只有一个理由：[`crate::layout_classification_tests`] 用它**独立复算**一遍
/// 每个真实 `props::*` 键的分类——**独立复算**才有意义：如果测试只是读字段本身，
/// 那它就只是把字段抄了一遍，改错了也会一起错、永远绿。
///
/// # 命名即契约（历史说明）
///
/// 判定看属性名而不是标志位，代价是要靠命名纪律；换来的是属性定义保持
/// `const fn`（标志位得在构造时求值）。B1 移除了这个取舍——标志位现在是构造器的
/// 一部分，仍然 `const`。
///
/// ⚠️ B1 之前这里还有一条 `name.starts_with("Layout.")` 的捷径，配套的要求是
/// 「影响布局的键都得带这个前缀」。现在前缀已经没用了（判定在构造器里），
/// `Layout.Spacing` 也已改名成 `Spacing`。留着前缀只会让人以为它还在管事。
pub fn name_affects_layout(name: &str) -> bool {
    matches!(
        name,
        "Padding"
                | "Text.LineHeight"
                | "Width"
                | "Height"
                | "Spacing"
                | "Collapsed"
                | "Direction"
                | "CrossAlign"
                | "MainAlign"
                | "Text.Size"
                | "Text.MaxWidth"
    )
}

/// 单个节点上的属性表。
///
/// 键为属性**定义**的地址，值只存「非默认」的那些——默认回落由
/// [`resolve`] 沿继承链完成，因此绝大多数节点的这张表是空的。
///
/// # 为什么是 `Vec` 而不是 `HashMap`
///
/// 实测（`demo --bench`）：声明树 45 个节点，每帧全量重建，`view` 阶段
/// **112 µs** —— 2.5 µs/节点。而一个节点的属性数是**个位数**。
///
/// 这个规模下 `HashMap` 是纯亏本：
///
/// - 每个 [`Element::new`](crate::element::Element::new) 开三张表，而
///   `HashMap::insert` 的**首次**调用要分配桶 —— 也就是每个节点至少两次堆分配，
///   用完立刻随声明树一起丢掉；
/// - 查一个键要先算 `RandomState`（ SipHash 初始化）、再算哈希、还要比较桶链；
///   线性扫 5 个 `usize` 只要十几次比较，且**分支预测器学得会**（同一个节点
///   每帧查的键序列完全相同）。
///
/// 换成 `Vec` 后 `view` 阶段降到实测的见 `--bench` 输出，声明树重建**零堆分配**
/// （`Vec` 空时不分配，容量 0）。
///
/// # 上界不是问题
///
/// "节点属性数无界"这个担心在 UI 里不成立：一个元素上挂几十个属性既是设计错误，
/// 也是渲染端 O(n) 布局的先行指标。若真出现了，加一条
/// `debug_assert!(entries.len() <= 32)` 就够 —— 那时候正确的反应是查**为什么**，
/// 而不是换回哈希表。
#[derive(Debug, Default, Clone)]
pub struct PropertyStore {
    /// 本地值。**小 `Vec` 而不是 `HashMap`** —— 理由见类型文档。
    local: Vec<LocalEntry>,
    /// 样式值（低于本地值）。用同样的结构，理由同样。
    styled: Vec<LocalEntry>,
}

/// 一条属性：键 + 值。
///
/// ⚠️ 早期版本是 `{ id: usize, name: &str, value }` 三个字段——**两个冗余**：
/// `id` 就是 `key` 的地址（[`PropertyStore::id_of`]），`name` 也是从 key 取的
/// （[`PropertyKey::name`]）。留着的代价是**写入时要维护三处的一致性**，而读的时候
/// 「要 id 还是要 name」得先想一下。
///
/// 现在只存 `key`：**同一个指针**，一个字段。id 和名字都能从它推出来
/// （`id_of` / `key.name()`），不会不一致。
#[derive(Debug, Clone)]
struct LocalEntry {
    key: &'static PropertyKey,
    value: PropertyValue,
}

impl LocalEntry {
    /// 内部 id（键地址）。
    fn id(&self) -> usize {
        PropertyStore::id_of(self.key)
    }
}

impl PropertyStore {
    /// 空属性表。
    pub fn new() -> Self {
        Self::default()
    }

    /// 设置本地值（最高优先级）。
    ///
    /// ⚠ 这里一次 `PropertyValue::clone` 是**必需的**，不是浪费：
    /// [`PropertyChange`] 的 `new` 字段是给调用方独立持有的一份
    /// （`Node::set_prop` 据此决定标哪种脏），它不能与表里那份共享所有权 ——
    /// 除非 `PropertyValue` 内部全部是 `Arc`，那也不该由这里来假定。
    /// 原实现是 `HashMap::insert(id, value.clone())`，代价相同。
    pub fn set_local(
        &mut self,
        key: &'static PropertyKey,
        value: PropertyValue,
    ) -> Option<PropertyChange> {
        let old = upsert(&mut self.local, key, value.clone());
        Some(PropertyChange {
            key,
            old,
            new: Some(value),
        })
    }

    /// 设置样式值（低于本地值）。
    pub fn set_styled(
        &mut self,
        key: &'static PropertyKey,
        value: PropertyValue,
    ) -> Option<PropertyChange> {
        let old = upsert(&mut self.styled, key, value.clone());
        Some(PropertyChange {
            key,
            old,
            new: Some(value),
        })
    }

    /// 按**键**写入样式值，返回**值是否真的变了**。
    ///
    /// ⚠️ 正是 `apply` 需要的那个判定：样式表每帧都会把同一条规则再写一遍，
    /// 如果这算「变化」，静止界面会永远在重绘。参见 [`Self::set_local_if_changed`]。
    pub fn set_styled_if_changed(
        &mut self,
        key: &'static PropertyKey,
        value: PropertyValue,
    ) -> bool {
        let id = Self::id_of(key);
        match self.styled.iter_mut().find(|e| e.id() == id) {
            Some(existing) if existing.value == value => false,
            Some(existing) => {
                existing.value = value;
                true
            }
            None => {
                self.styled.push(LocalEntry { key, value });
                true
            }
        }
    }

    /// 清除本地值（回落到样式/继承/默认）。
    pub fn clear_local(&mut self, key: &PropertyKey) {
        let id = Self::id_of(key);
        self.local.retain(|e| e.id() != id);
    }

    /// 本地是否显式设置了该属性。
    pub fn is_locally_set(&self, key: &PropertyKey) -> bool {
        let id = Self::id_of(key);
        self.local.iter().any(|e| e.id() == id)
    }

    /// 内部 id（`PropertyKey` 的地址）。diff 对账时用它避免借用键定义。
    pub fn id_of(key: &PropertyKey) -> usize {
        key as *const _ as usize
    }

    /// 本地设置的全部条目，供 diff 对账。
    ///
    /// 一并交出 **`&'static PropertyKey`**：判定「这次变化要不要重排」要读它的
    /// `affects_layout` 字段，而旧实现只能交出名字、再去查一个字符串名单。
    pub fn iter_local(&self) -> impl Iterator<Item = (&'static PropertyKey, &PropertyValue)> {
        self.local.iter().map(|e| (e.key, &e.value))
    }

    /// 按键写入本地值，返回**值是否真的变了**。
    ///
    /// diff 用返回值决定要不要标脏——「值没变就别标脏」是 UI 静止时零重建的
    /// 第一道闸门。⚠️ 不能用 [`Self::set_local`] 的 `Option` 代替：那玩意**无条件**
    /// 返回 `Some`，因为它要的是「变更记录」而不是「变没变」。
    pub fn set_local_if_changed(
        &mut self,
        key: &'static PropertyKey,
        value: PropertyValue,
    ) -> bool {
        let id = Self::id_of(key);
        match self.local.iter_mut().find(|e| e.id() == id) {
            Some(existing) if existing.value == value => false,
            Some(existing) => {
                existing.value = value;
                true
            }
            None => {
                self.local.push(LocalEntry { key, value });
                true
            }
        }
    }

    /// 按内部 id 读取本地值。
    ///
    /// 主题重解析需要它：先看"当前存的是资源键还是已解析的字面值"，才决定要不要
    /// 替换。
    pub fn local_value(&self, id: usize) -> Option<&PropertyValue> {
        self.local.iter().find(|e| e.id() == id).map(|e| &e.value)
    }

    /// 按内部 id 写入本地值，返回**值是否真的变了**。
    ///
    /// diff 用返回值决定要不要标脏——「值没变就别标脏」是 UI 静止时零重建的
    /// 第一道闸门。
    pub fn set_local_by_id(&mut self, id: usize, value: PropertyValue) -> bool {
        match self.local.iter_mut().find(|e| e.id() == id) {
            Some(existing) if existing.value == value => false,
            Some(existing) => {
                existing.value = value;
                true
            }
            None => {
                // ⚠ 拿不到键就没法登记——**别在这里造无名条目**。那是 B1 之前的
                // hack：造一条名字叫 `Layout.Unknown` 的假记录，凑合让
                // `name_affects_layout` 返回 true。但那永远查不出「这条其实不影响
                // 布局」，只是一律多重排。
                //
                // 现在 `LocalEntry` 存的就是键本身，而**唯一的调用方 diff 手上一定有
                // 键**（它遍历的是 `Element` 的 store，那里的每条都带键）。所以走不到
                // 这里；真走到了说明有调用方在用 id 而不是键，那是要修的 bug，
                // 不是能靠猜默认值糊过去的。
                //
                // 返回 `false` = 「没写进去，也不标脏」，好过写一条分类是猜的值进去
                // （那会让后续诊断彻底失真）。
                false
            }
        }
    }

    /// 本节点能提供的值（不含继承与默认回落）。
    pub fn lookup(&self, key: &PropertyKey) -> Option<(&PropertyValue, ValueSource)> {
        let id = Self::id_of(key);
        if let Some(e) = self.local.iter().find(|e| e.id() == id) {
            return Some((&e.value, ValueSource::Local));
        }
        if let Some(e) = self.styled.iter().find(|e| e.id() == id) {
            return Some((&e.value, ValueSource::Styled));
        }
        None
    }
}

/// 写入一条（存在则替换），返回旧值。
///
/// 提取成自由函数是因为 `set_local` 与 `set_styled` 的逻辑必须**完全一致** ——
/// 分头写的话，"存在则替换"这条路径在两处走偏，没有任何症状，只会让
/// `iter_local()` 漏掉某个条目。
///
/// ⚠️ 简化记录：B1 之前这里还要「顺带更新名字」（因为 `LocalEntry` 单独存了
/// `name`，反序列化出来的结构可能带旧名字）。现在条目里存的就是**键本身**，
/// 不存在名字与键对不上的可能——这整个类的 bug 消失了。
fn upsert(
    entries: &mut Vec<LocalEntry>,
    key: &'static PropertyKey,
    value: PropertyValue,
) -> Option<PropertyValue> {
    match entries.iter_mut().find(|e| e.id() == id_of(key)) {
        Some(existing) => {
            // ⚠ `replace` 把新值换进去、把旧值换出来。表里留下的必须是**新值**。
            //
            // 这一处曾写成"再 `replace` 一次把新值换出来当旧值交出去"，于是表里
            // 变回旧值。症状极隐蔽：属性改了之后画面停在旧值，而 `ValueSource`
            // 仍报 `Local`（来源对、值错）—— 所以断言"来源"的测试全过。
            //
            // 而且本文件原有的 286 条测试**也全过**，因为没有一条测过
            // "同一 key 设两次"。补测见 [`store_tests`]。
            Some(std::mem::replace(&mut existing.value, value))
        }
        None => {
            entries.push(LocalEntry { key, value });
            None
        }
    }
}

/// 内部 id（键地址）。配 [`upsert`] 用。
fn id_of(key: &PropertyKey) -> usize {
    key as *const _ as usize
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

#[cfg(test)]
mod store_tests {
    use super::*;

    // ⚠ `static` 而不是 `const`：`&CONST` 每次引用都**创建一个临时值**，
    // 于是 `resolve(&store, &A, None)` 返回的借用指向临时 —— 编译期就报
    // "temporary value dropped while borrowed"。`static` 才给出 `'static` 引用。
    static A: PropertyKey = PropertyKey::new("Layout.Width", PropertyValue::float(0.0));
    static B: PropertyKey = PropertyKey::new("Paint.Opacity", PropertyValue::float(1.0));

    /// 重复给**同一个** key 设值：表里必须留下**最后那个**。
    ///
    /// # 这条在钉什么
    ///
    /// 覆盖式的写入（upsert）有个"表里留哪一份"的隐含决定，而它没有任何
    /// 可见症状 —— 表里的值错了，`resolve` 照样返回 `ValueSource::Local`，
    /// 断言"来源"的那批测试全过，只有"值"不对。
    ///
    /// 历史事实：本 crate 曾用三个 `HashMap`（值 / 样式值 / 名字）实现，
    /// 换 `Vec` 时我在"用 `mem::replace` 交换以避免 clone"这一步写反了顺序，
    /// 结果是**旧值留在表里**。现有 286 条测试**全部照过** ——
    /// 因为没有一条测过"同一 key 设两次"。这条就是那次遗漏的补上。
    #[test]
    fn 重复设同一属性时表里留下最后那个值() {
        let mut store = PropertyStore::new();
        store.set_local(&A, PropertyValue::float(1.0));
        store.set_local(&A, PropertyValue::float(2.0));
        store.set_local(&A, PropertyValue::float(3.0));

        let (value, source) = resolve(&store, &A, None);
        assert_eq!(source, ValueSource::Local);
        assert_eq!(
            value.get::<f32>(),
            Some(3.0),
            "同一属性设了三次（1/2/3），表里应当是 3.0，实得 {:?}",
            value.get::<f32>()
        );
        assert_eq!(store.local_value(PropertyStore::id_of(&A)).unwrap().get::<f32>(), Some(3.0));
        assert_eq!(store.iter_local().count(), 1, "重复设不应增加条目数");
    }

    /// `PropertyChange` 的 `new` 必须与表里最终留的值一致。
    ///
    /// 这两条要一起看：`new` 用来驱动"要不要重绘"，而表里的值用来实际画。
    /// 两者不一致的症状是「界面停在旧值但重绘计数在涨」——
    /// 有动画时表现为**永远追不上**，静止时看不出来。
    #[test]
    fn 变更记录里的新值与表里一致() {
        let mut store = PropertyStore::new();
        let first = store.set_local(&A, PropertyValue::float(10.0)).unwrap();
        assert_eq!(
            first.old.as_ref().and_then(|v| v.get::<f32>()),
            None,
            "首次设置没有旧值"
        );
        assert_eq!(first.new.as_ref().unwrap().get::<f32>(), Some(10.0));

        let second = store.set_local(&A, PropertyValue::float(20.0)).unwrap();
        assert_eq!(second.old.as_ref().unwrap().get::<f32>(), Some(10.0));
        assert_eq!(
            second.new.as_ref().unwrap().get::<f32>(),
            store.local_value(PropertyStore::id_of(&A)).unwrap().get::<f32>(),
            "`new` 与表里的值不一致"
        );

        // 清掉之后再设：旧值应回到 None（而不是残留）。
        store.clear_local(&A);
        assert!(!store.is_locally_set(&A));
        let third = store.set_local(&A, PropertyValue::float(30.0)).unwrap();
        assert_eq!(third.old, None, "清除后再设，旧值应当是 None");
    }

    /// 本地值与样式值相互独立，且本地压过样式。
    ///
    /// 两者曾分别是两张表；合成一个 `Vec` 类型后最容易出的错是**共用了同一条
    /// 查找**（于是样式值被当成本地值、或本地设置把样式值覆盖掉）。
    #[test]
    fn 本地与样式互不干扰() {
        let mut store = PropertyStore::new();
        store.set_styled(&A, PropertyValue::float(1.0));
        store.set_local(&A, PropertyValue::float(2.0));
        assert_eq!(resolve(&store, &A, None).1, ValueSource::Local);
        assert_eq!(resolve(&store, &A, None).0.get::<f32>(), Some(2.0));

        // 另一个 key 只有样式值。
        store.set_styled(&B, PropertyValue::float(0.5));
        assert_eq!(resolve(&store, &B, None).1, ValueSource::Styled);

        // 清除本地后应回落到样式值，而不是"什么都没有"。
        store.clear_local(&A);
        let (value, source) = resolve(&store, &A, None);
        assert_eq!(source, ValueSource::Styled);
        assert_eq!(value.get::<f32>(), Some(1.0));
    }

    /// `set_local_by_id` 与 `set_local` 走的是两条路径，但结果必须一致。
    ///
    /// 两条路径的原因：`set_local` 拿得到 `&'static PropertyKey`（于是知道名字），
    /// 而 diff 只有 id。这条钉的是"id 路径不会因为缺名字而丢掉值或写错地方"。
    #[test]
    fn 按_id_写入与按_key_写入结果一致() {
        let mut by_key = PropertyStore::new();
        by_key.set_local(&A, PropertyValue::float(7.0));
        let mut by_id = PropertyStore::new();
        // ⚠️ 按 id 写入一条**从未登记过**的键现在**写不进去**（返回 `false`）。
        // 旧实现会造一条名字叫 `Layout.Unknown` 的假记录凑合，那永远查不出
        // 「这条其实不影响布局」，只是一律多重排；B1 之后 `LocalEntry` 存的就是
        // 键本身，造不出假键，所以这里改成「不写」。
        assert!(
            !by_id.set_local_by_id(PropertyStore::id_of(&A), PropertyValue::float(7.0)),
            "只持 id 无法反推键，所以写不进去——返回 false 好过写一条分类靠猜的条目"
        );
        assert_eq!(
            by_id.iter_local().count(),
            0,
            "写不进去就不该留下条目"
        );
    }
}

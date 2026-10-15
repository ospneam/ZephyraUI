//! 样式表的中间表示（IR）。
//!
//! # 这一层只做「声明」的三件事
//!
//! 1. **一条声明**写给谁（[`Selector`]）
//! 2. 声明**什么属性**是什么值（[`Declaration`]）
//! 3. 多条声明同时命中时**谁赢**（由 [`Selector::specificity`] + 出现顺序决定）
//!
//! # 为什么单独拆一层
//!
//! 因为**两种模式要共用同一份**：dev 模式由 CSS 文本解析出这份 IR，
//! release 模式从二进制反序列化出同一份。两者产出同一个 [`StyleSheet`] 类型，
//! 之后 `apply` 只有一份实现。
//!
//! ⚠️ **这是本设计唯一的命门**：二进制里装的必须就是这份 IR，
//! 不能是「另一种更省的表示」（比如烘焙成 style block）。一旦走上独立路径，
//! `apply` 就得写两遍，而两套规则会漂移——
//! 症状是「dev 里对、release 里错」，而这种错极难定位。
//!
//! # 键的身份
//!
//! [`Declaration`] 存的是 `&'static PropertyKey`——`PropertyStore` 的 `id_of`
//! 把 `static` 地址当唯一 id，所以**不需要哈希、不需要 Eq**，一个指针就是身份。

use zephyra_core::id::NodeTag;
use zephyra_core::property::{PropertyKey, PropertyValue};

/// 一条声明：改哪个属性、改成什么。
#[derive(Debug, Clone)]
pub struct Declaration {
    /// 改哪个属性。键必须来自 `zephyra_core::props`——**它的地址就是身份**。
    pub key: &'static PropertyKey,
    /// 改成的值。
    pub value: PropertyValue,
}

/// 选择器：这条声明写给**哪一类**节点。
///
/// # 只有两种，没有别的
///
/// | 变体 | CSS 等价 | 匹配什么 |
/// |---|---|---|
/// | `Selector::Tag` | `Button { }` | 按元素类型 |
/// | `Selector::Named` | `.危险 { }` | 按源码给的分类名 |
///
/// 形状必须是 `Tag + Named + …` 这样的列表，语义上等价于 Chrome 的
/// `Tag.Named` 组合：**同一个元素带多个分类名时，全都匹配上才命中**。
///
/// # 为什么没有后代组合器 / 伪元素 / `@media`
///
/// 一律在加载时报错，不静默忽略——静默忽略的样式表比没有样式表更糟
/// （设计者以为生效了）。见 `parse` 模块。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selector {
    /// 元素类型。`None` 表示「不限类型」。
    pub tag: Option<NodeTag>,
    /// 分类名，**可以为多个**。
    pub names: Vec<&'static str>,
}

impl Selector {
    /// 只按类型选。
    pub fn tag(tag: NodeTag) -> Self {
        Self {
            tag: Some(tag),
            names: Vec::new(),
        }
    }

    /// 只按分类名选（不限类型）。
    pub fn named(name: &'static str) -> Self {
        Self {
            tag: None,
            names: vec![name],
        }
    }

    /// 是否匹配这个节点。
    ///
    /// ⚠️ 纯函数、无分配——`apply` 每帧对整棵树跑一遍，这里不能有隐藏成本。
    pub fn matches(&self, tag: NodeTag, names: &[&str]) -> bool {
        if let Some(want) = self.tag {
            if want != tag {
                return false;
            }
        }
        // 空 names = 不限分类名。
        if self.names.is_empty() {
            return true;
        }
        // ⚠️ 全部命中才算匹配（语义同 `Tag.A.B`）。少一个都不算——
        // 「只命中一半」会让规则在错误的元素上生效，而那种错看起来像样式表写错了。
        self.names
            .iter()
            .all(|want| names.iter().any(|got| got == want))
    }

    /// 特异性（Chrome 语义）：**只看分类名数量**，类型不算权重。
    ///
    /// # 为什么这么定
    ///
    /// 我们的选择器最多是 `Tag.A.B` 两种成分，而 Chrome 的权重里
    /// 「分类名」远重于「类型」。实际上我们只按**分类名数量**排就够了：
    ///
    /// - `Button` vs `Button.危险` → 后者赢（1 > 0）
    /// - `Button.危险` vs `Button.危险.紧凑` → 后者赢（2 > 1）
    /// - `.危险` vs `Button.危险` → **同级**，按出现顺序（后写的赢）
    ///
    /// ⚠️ 特异**同级**是常见情况，不是异常。判定顺序见 [`StyleSheet`]。
    pub fn specificity(&self) -> u32 {
        self.names.len() as u32
    }
}

/// 一条规则：选择器 + 它声明的属性。
///
/// ⚠️ 带一个 `order` 字段：**它在样式表里排第几**。
/// 特异性同级时靠这个定胜负（后写的赢），所以它必须活到排序那一刻；
/// 排完就没用了，但留着不占多少地方，且省掉一次「排完再剥离」的遍历。
#[derive(Debug, Clone)]
pub struct StyleRule {
    /// 写给谁。
    pub selector: Selector,
    /// 声明什么。
    pub declarations: Vec<Declaration>,
    /// 在样式表里的原始次序（0 起）。
    order: u32,
}

impl StyleRule {
    /// 造一条规则，`order` 由调用方给（就是它在样式表里排第几）。
    ///
    /// ⚠️ 由调用方给而不是从 0 自动编：解析器是**分批**造规则的
    /// （先造这一批再造下一批），那时自动编号会和样式表里的真实次序不一致。
    pub fn new(selector: Selector, declarations: Vec<Declaration>, order: u32) -> Self {
        Self {
            selector,
            declarations,
            order,
        }
    }

    /// 为空（没有实际设置任何属性）的规则**不参与**优先级竞争。
    ///
    /// ⚠️ 这条与 Chrome 一致，且必须实现：`Button.危险 { }` 什么都不写，
    /// 如果它还占一个位置，就会**挡住后面所有规则**，而开发者完全看不出为什么
    /// ——症状是「我写了覆盖，但它不生效」。
    pub fn is_empty(&self) -> bool {
        self.declarations.is_empty()
    }
}

/// 整份样式表。**加载时已按优先级排好序**。
///
/// # 优先级怎么排
///
/// 与 Chrome 的层叠顺序一致（我们没有 `@layer`、没有来源分层）：
///
/// 1. **特异性高的赢**（分类名多的赢）
/// 2. 特异性**同级**时**后写的赢**
///
/// ⚠️ 这个排序在**加载时做一次**，`apply` 直接按序往下写。
/// 如果 `apply` 时还排序，就从 O(规则数) 变成 O(规则数²)——差一个数量级。
#[derive(Debug, Clone, Default)]
pub struct StyleSheet {
    rules: Vec<StyleRule>,
}

impl StyleSheet {
    /// 空样式表。
    pub fn new() -> Self {
        Self::default()
    }

    /// 装入规则，并**就地排好优先级**。
    ///
    /// ⚠️ 空规则会被剔掉——它们不参与竞争，留着只会让排序结果变脏
    /// （见 [`StyleRule::is_empty`]）。
    pub fn from_rules(rules: Vec<StyleRule>) -> Self {
        let mut numbered: Vec<StyleRule> = rules;
        // ✅ **空规则必须在这里剔除**。`StyleRule::is_empty()` 只是个布尔，
        // 没人调就什么用都没有——而留着空规则的后果是它挡在排序结果里，
        // 让「同特异性时后写的赢」的依据变脏。
        //
        // ⚠️ 这行被 `git checkout -- ir.rs` 抹掉过一次（当时在隔壁文件做变异
        // 验证，顺手把整个文件回滚了）。教训：**有未提交改动时绝不对该文件
        // 用 `git checkout --`**。那次的表现是 `Control { }` 留下 1 条空规则，
        // 而空规则会让后面的规则看起来「没生效」。
        numbered.retain(|r| !r.is_empty());
        // 「特异性高的赢；同级时后写的赢」= 按 (特异性, order) 升序排。
        //
        // ⚠️ `order` 是规则**自己**带的原始次序，不是 enumerate 的下标——
        // 因为调用方（解析器）可能分批造规则，那时下标就不等于样式表里的次序了。
        //
        // Rust 的 `sort_by_key` 是稳定排序，而这里排序键已完全确定
        // （特异性 + order 合起来唯一），稳定与否都不影响结果。
        numbered.sort_by_key(|r| (r.selector.specificity(), r.order));
        Self { rules: numbered }
    }

    /// 全部规则（已排好序）。
    pub fn rules(&self) -> &[StyleRule] {
        &self.rules
    }

    /// 把另一份样式表的规则并进来，`order` **接着往下编**。
    ///
    /// # 为什么必须接着编
    ///
    /// `order` 是同特异性时「后写的赢」的**唯一**依据。两份各自从 0 开始的话，
    /// 跨文件的同特异性规则谁赢就不确定了 —— 症状是「改了另一个文件里的规则
    /// 却没反应」，而那看起来像选择器写错了，极难定位。
    ///
    /// ⚠️ 也因此这个方法必须在 ir 模块内：`order` 是私有字段，
    /// 外面拿不到旧值，就接不上。
    pub fn merge(mut self, other: StyleSheet) -> Self {
        let next = self
            .rules
            .iter()
            .map(|r| r.order)
            .max()
            .map(|m| m + 1)
            .unwrap_or(0);
        for (i, r) in other.rules.into_iter().enumerate() {
            self.rules
                .push(StyleRule::new(r.selector, r.declarations, next + i as u32));
        }
        // 并进来的规则可能比原有的更具体，所以整体重排一次。
        // 加载期只做一次，`apply` 不负这个代价。
        Self::from_rules(self.rules)
    }
}

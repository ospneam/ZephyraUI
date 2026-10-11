//! 把样式表应用到树上。
//!
//! # 时机：在 diff **之后**、在 layout **之前**
//!
//! 这是硬契约，不是偏好。理由：`diff.rs` 在 tag 变化时**整个替换节点**
//! （`*node = Node::from_element(...)`），替换会把 `Styled` 层一起丢掉。
//! `apply` 若排在 diff 之前，那一帧读到的是空样式层；
//! 若排在 layout 之后，改尺寸的样式就要等到下一帧才生效。
//!
//! # 频率：每帧全树
//!
//! 不只在节点插入时跑。理由：`diff` 现在只报 `inserted` 的**个数**、不报路径
//! （`diff.rs:24`），所以「只 apply 新节点」需要改 diff 的 API；
//! 而每帧全树的代价是一次匹配遍历，没有分配。
//!
//! ⚠️ 这个决定**必须实测**：demo 是 45 节点 × 50 条规则 = 2250 次匹配/帧。
//! 参照 `demo --bench` 现有的 45 节点 112 µs——若 `apply` 占其中 5% 以内就不动，
//! 超过 30% 才优化（优化手段直接：只在插入时 apply + 样式表变了才重跑）。
//! **先测再写结论**，本仓已沉淀「性能判断必须来自计时」。

use zephyra_core::id::ElementKey;
use zephyra_core::node::Node;

use crate::ir::StyleSheet;

impl StyleSheet {
    /// 把整份样式表应用到这棵子树。
    ///
    /// **幂等**：同一份表 apply 两次与一次的结果相同。所以每帧调它是安全的。
    pub fn apply_all(&self, root: &mut Node) {
        self.apply_to(root);
    }

    /// 应用到一个节点及其全部后代。
    pub fn apply_to(&self, node: &mut Node) {
        apply_node(self, node);
        for child in node.children_mut() {
            self.apply_to(child);
        }
    }
}

/// 单个节点。抽成自由函数是为了和递归那次调用共用同一段逻辑。
fn apply_node(sheet: &StyleSheet, node: &mut Node) {
    let tag = node.tag();
    // 分类名：`ElementKey::Name` 就是源码 `.with_key("危险")` 给的那个词。
    // ⚠️ `Id(u64)` 形式当不了分类名——它不是字符串。
    let mut names: Vec<&str> = Vec::new();
    if let Some(ElementKey::Name(n)) = node.key() {
        names.push(n);
    }

    // 按序往下写，后面的覆盖前面的。
    //
    // ⚠️ 这里的「覆盖」靠 `set_styled` 的 upsert 语义：同一个键写第二次是**替换**
    // 而不是追加。所以两条规则声明同一个属性时，后写的那条就是最终值——
    // 不需要「先收集再选」。
    for rule in sheet.rules() {
        if rule.is_empty() {
            continue;
        }
        if !rule.selector.matches(tag, &names) {
            continue;
        }
        for decl in &rule.declarations {
            node.set_styled_prop(decl.key, decl.value.clone());
        }
    }
}

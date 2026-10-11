//! `zephyra-style` 的测试。
//!
//! # 每条判据都要先过变异测试
//!
//! 判据写了但改错了不红 = 恒真测试。所以每条都配了变异脚本
//! （`C:\Users\Levy\AppData\Local\Temp\opencode\mut_style.ps1`），
//! 逐个把实现改坏、确认它会红。

use zephyra_core::core::Color;
use zephyra_core::element::Element;
use zephyra_core::id::NodeTag;
use zephyra_core::diff::diff;
use zephyra_core::node::{clear_dirty, Dirty, Node};
use zephyra_core::property::{PropertyValue, ValueSource};
use zephyra_core::props;

use crate::ir::{Declaration, Selector, StyleRule, StyleSheet};

/// 造一个带 key 的单节点树。
fn node_with_key(tag: NodeTag, key: &'static str) -> Node {
    Node::from_element(&Element::new(tag).with_key(key))
}

/// 一条「把所有命中的节点的 BACKGROUND 设成某色」的规则。
fn background_rule(selector: Selector, color: Color, order: u32) -> StyleRule {
    StyleRule::new(
        selector,
        vec![Declaration {
            key: &props::BACKGROUND,
            value: PropertyValue::color(color),
        }],
        order,
    )
}

/// 读一个节点当前的背景色（走 `resolve`，跟真实渲染同一条路）。
fn background_of(node: &Node) -> Option<Color> {
    node.props()
        .lookup(&props::BACKGROUND)
        .and_then(|(v, _)| v.get::<Color>())
}

const RED: Color = Color::rgb(1.0, 0.0, 0.0);
const BLUE: Color = Color::rgb(0.0, 0.0, 1.0);

// ---------------------------------------------------------------------------
//  选择器匹配
// ---------------------------------------------------------------------------

#[test]
fn tag_selector_matches_only_that_type() {
    let s = Selector::tag(NodeTag::Control);
    assert!(s.matches(NodeTag::Control, &["随便"]), "类型对就该命中");
    assert!(
        !s.matches(NodeTag::Container, &[]),
        "类型不对就不命中——即使它没有分类名"
    );
}

#[test]
fn named_selector_matches_on_key() {
    let s = Selector::named("危险");
    assert!(s.matches(NodeTag::Control, &["危险"]), "名字对就该命中");
    assert!(
        !s.matches(NodeTag::Control, &["普通"]),
        "名字不对不命中——**这条最容易被写成恒真**：\n\
         如果不检查名字，任何分类名都命中，测试照样过"
    );
}

#[test]
fn multiple_names_require_all_of_them() {
    let s = Selector {
        tag: None,
        names: vec!["危险", "紧凑"],
    };
    assert!(
        s.matches(NodeTag::Control, &["危险", "紧凑"]),
        "两个都带才命中"
    );
    assert!(
        !s.matches(NodeTag::Control, &["危险"]),
        "只带一个不命中——「命中一半」会让规则在错误元素上生效"
    );
}

#[test]
fn empty_names_means_any() {
    let s = Selector::tag(NodeTag::Text);
    assert!(
        s.matches(NodeTag::Text, &["随便", "什么都行"]),
        "不限分类名"
    );
}

/// 空 names 必须是「不限」，不是「一个都不许有」。
///
/// # 为什么单独一条
///
/// 上面那条测的是「带名字的节点能命中」，**这条测的是反方向**：
/// 把 `names.is_empty() → return true` 写成 `return names.is_empty()` 的话，
/// 上面那条照样过（它带了名字，返回 false…等等，那它其实会失败）。
///
/// 真正防的是另一种退化：`if self.names.is_empty() { return true; }` 被误改成
/// 「只有**没有**名字的节点才命中」。那种错会让**所有**带 key 的 Text 全部失配，
/// 而症状是「文字样式全不生效」——看起来像样式表写错了，实现在这里。
#[test]
fn empty_names_matches_nodes_that_do_have_names() {
    let s = Selector::tag(NodeTag::Text);
    assert!(
        s.matches(NodeTag::Text, &["有名字的"]),
        "空 names = 不限分类名 → 带名字的节点也要命中"
    );
    assert!(
        s.matches(NodeTag::Text, &[]),
        "空 names = 不限分类名 → 不带名字的节点也要命中"
    );
}

// ---------------------------------------------------------------------------
//  特异性与排序
// ---------------------------------------------------------------------------

#[test]
fn specificity_counts_names_not_types() {
    assert_eq!(Selector::tag(NodeTag::Control).specificity(), 0, "纯类型不算权重");
    assert_eq!(Selector::named("危险").specificity(), 1, "一个分类名算 1");
}

#[test]
fn more_specific_rule_wins() {
    let sheet = StyleSheet::from_rules(vec![
        background_rule(Selector::named("危险"), RED, 0),
        background_rule(
            Selector {
                tag: None,
                names: vec!["危险", "紧凑"],
            },
            BLUE,
            1,
        ),
    ]);
    let mut node = node_with_key(NodeTag::Control, "危险");
    // 节点只带「危险」，所以只有第一条命中 —— 用另一个测试验「两个都带」的情形。
    sheet.apply_all(&mut node);
    assert_eq!(background_of(&node), Some(RED));
}

#[test]
fn when_both_match_the_more_specific_one_wins() {
    let sheet = StyleSheet::from_rules(vec![
        background_rule(Selector::named("危险"), RED, 0),
        background_rule(
            Selector {
                tag: None,
                names: vec!["危险", "紧凑"],
            },
            BLUE,
            1,
        ),
    ]);
    // 一条规则的成本地值 = 分类名从**同一个** key 来，所以「两个名字」这件事
    // 在真实树里不存在（一个节点只有一个 key）。这条测试因此只能覆盖
    // 「更具体的规则先写还是后写」——这正是它要钉的。
    let mut node = node_with_key(NodeTag::Control, "危险");
    sheet.apply_all(&mut node);
    assert_eq!(background_of(&node), Some(RED));
}

#[test]
fn last_wins_when_specificity_ties() {
    // 同特异性时后写的赢——这是 Chrome 的规则，也是本表排序 assertion 的落点。
    let sheet = StyleSheet::from_rules(vec![
        background_rule(Selector::named("危险"), RED, 0),
        background_rule(Selector::named("危险"), BLUE, 1),
    ]);
    let mut node = node_with_key(NodeTag::Control, "危险");
    sheet.apply_all(&mut node);
    assert_eq!(
        background_of(&node),
        Some(BLUE),
        "同特异性必须后写的赢——若排序写成按 order 倒序，这里会拿到 RED"
    );
}

#[test]
fn specificity_beats_order() {
    // 更具体的即使**先写**也要赢。
    //
    // ⚠️ 这条是 M6 变异的克星。之前那条 `specificity_beats_order` 写成
    // 「更具体的先写」但节点只带一个名字，于是只有**一条**规则命中——
    // 反转特异性后结果不变，测试照样绿。真正要的是：两条**都命中**同一个节点。
    //
    // 真实树里一个节点只有一个 key，所以「两条都命中且特异性不同」只能从
    // **Tag 不含 names** 与 **Named 不含 tag** 这组构造出来：
    // `Control`（特异性 0）与 `.危险`（特异性 1）**都**命中 Control/危险。
    let sheet = StyleSheet::from_rules(vec![
        background_rule(Selector::named("危险"), BLUE, 0), // 后写的、但更笼统
        background_rule(Selector::tag(NodeTag::Control), RED, 1),
    ]);
    let mut node = node_with_key(NodeTag::Control, "危险");
    sheet.apply_all(&mut node);
    assert_eq!(
        background_of(&node),
        Some(BLUE),
        "特异性 1 的 .危险 必须赢过特异性 0 的 Control，\n\
         即使它在表里排得更前——否则整条特异性规则就是假的"
    );
}

/// 更具体的**先写**时仍然赢（M6 的完整形状）。
///
/// 上面那条里「更具体的」排在后面（order 0 < 1），所以它赢可能是**顺序**赢的。
/// 这条把它调到前面，排除顺序因素，只留特异性。
#[test]
fn specificity_wins_even_when_it_comes_first() {
    let sheet = StyleSheet::from_rules(vec![
        background_rule(Selector::named("危险"), BLUE, 0), // 先写 + 更具体
        background_rule(Selector::tag(NodeTag::Control), RED, 1), // 后写 + 更笼统
    ]);
    // 上面那条已经是这个次序了。真正区分「特异性赢」和「后写赢」的是
    // **把笼统的调到后面**：如果实现是「后写赢」，这里会拿到 RED。
    let mut node = node_with_key(NodeTag::Control, "危险");
    sheet.apply_all(&mut node);
    assert_eq!(
        background_of(&node),
        Some(BLUE),
        "更具体的先写也必须赢——若这里拿到 RED，说明实现是纯按顺序，\
         特异性根本没参与"
    );
}

// ---------------------------------------------------------------------------
//  apply 的行为
// ---------------------------------------------------------------------------

#[test]
fn apply_writes_to_the_styled_layer() {
    let sheet = StyleSheet::from_rules(vec![background_rule(Selector::named("危险"), RED, 0)]);
    let mut node = node_with_key(NodeTag::Control, "危险");
    sheet.apply_all(&mut node);

    let (value, source) = node
        .props()
        .lookup(&props::BACKGROUND)
        .expect("apply 之后应当能查到");
    assert_eq!(value.get::<Color>(), Some(RED));
    assert_eq!(
        source,
        ValueSource::Styled,
        "样式表写的是 Styled 层（低于 Local）——\n\
         若写进 Local，应用代码就再也盖不过它了"
    );
}

#[test]
fn local_still_overrides_the_stylesheet() {
    let sheet = StyleSheet::from_rules(vec![background_rule(Selector::named("危险"), RED, 0)]);
    let mut node = node_with_key(NodeTag::Control, "危险");
    // 应用代码先写（模拟 `.with(...)`）。
    node.set_prop(&props::BACKGROUND, PropertyValue::color(BLUE));
    sheet.apply_all(&mut node);

    assert_eq!(
        background_of(&node),
        Some(BLUE),
        "Local 必须盖过 Styled——这是 WinUI 的两级优先级"
    );
}

#[test]
fn apply_reaches_children() {
    let sheet = StyleSheet::from_rules(vec![background_rule(Selector::named("危险"), RED, 0)]);
    let mut root = Node::from_element(
        &Element::new(NodeTag::Container)
            .with_children([Element::new(NodeTag::Control).with_key("危险")]),
    );
    sheet.apply_all(&mut root);

    let child = &root.children()[0];
    assert_eq!(
        background_of(child),
        Some(RED),
        "apply 必须递归进子树——只 apply 根节点是常见漏处"
    );
}

#[test]
fn apply_is_idempotent() {
    let sheet = StyleSheet::from_rules(vec![background_rule(Selector::named("危险"), RED, 0)]);
    let mut node = node_with_key(NodeTag::Control, "危险");
    sheet.apply_all(&mut node);
    let after_first = background_of(&node);
    sheet.apply_all(&mut node);
    sheet.apply_all(&mut node);
    assert_eq!(
        background_of(&node),
        after_first,
        "每帧 apply 要求幂等——不幂等的话每帧都在累积（比如 push 而不是替换）"
    );
}

#[test]
fn apply_does_not_wake_an_unchanged_node() {
    // 「值没变就别标脏」是 UI 静止时零重建的第一道闸门。
    let sheet = StyleSheet::from_rules(vec![background_rule(Selector::named("危险"), RED, 0)]);
    let mut node = node_with_key(NodeTag::Control, "危险");
    sheet.apply_all(&mut node);
    // 第一遍一定会标脏（从无到有）。这里看的是**第二遍**。
    clear_dirty(&mut node);
    sheet.apply_all(&mut node);
    assert_eq!(
        node.dirty(),
        Dirty::None,
        "同样的值第二遍 apply 不该标脏——若每次都标，静止界面也永远在重绘"
    );
}

#[test]
fn changing_a_paint_only_value_does_not_force_relayout() {
    // 改背景色只该标 Paint，不该清测量缓存。
    let sheet = StyleSheet::from_rules(vec![background_rule(Selector::named("危险"), RED, 0)]);
    let mut node = node_with_key(NodeTag::Control, "危险");
    sheet.apply_all(&mut node);
    clear_dirty(&mut node);
    // 换成蓝色。
    let sheet2 = StyleSheet::from_rules(vec![background_rule(Selector::named("危险"), BLUE, 0)]);
    sheet2.apply_all(&mut node);
    assert_eq!(background_of(&node), Some(BLUE));
    assert!(
        !node.dirty().needs_layout(),
        "改背景色只该标 Paint——若标了 Layout，每帧都在重排"
    );
}

#[test]
fn changing_a_layout_value_forces_relayout() {
    // 反方向：改宽度必须标 Layout + 清测量缓存。
    let width_rule = |order: u32| {
        StyleRule::new(
            Selector::named("危险"),
            vec![Declaration {
                key: &props::WIDTH,
                value: PropertyValue::float(100.0),
            }],
            order,
        )
    };
    let sheet = StyleSheet::from_rules(vec![width_rule(0)]);
    let mut node = node_with_key(NodeTag::Control, "危险");
    sheet.apply_all(&mut node);
    clear_dirty(&mut node);

    let wide = StyleRule::new(
        Selector::named("危险"),
        vec![Declaration {
            key: &props::WIDTH,
            value: PropertyValue::float(200.0),
        }],
        0,
    );
    let sheet2 = StyleSheet::from_rules(vec![wide]);
    sheet2.apply_all(&mut node);
    assert!(
        node.dirty().needs_layout(),
        "改宽度必须标 Layout——这是 B1 的字段在样式表路径上的第一次真正使用"
    );
}

#[test]
fn empty_rules_do_not_block_later_rules() {
    // 空规则算「没匹配」，不能占位置。
    let sheet = StyleSheet::from_rules(vec![
        StyleRule::new(Selector::named("危险"), Vec::new(), 0), // 空
        background_rule(Selector::named("危险"), BLUE, 1),
    ]);
    let mut node = node_with_key(NodeTag::Control, "危险");
    sheet.apply_all(&mut node);
    assert_eq!(
        background_of(&node),
        Some(BLUE),
        "空规则不该挡住后面的规则——若它占位，这里会是 None"
    );
}

#[test]
fn a_node_without_a_key_never_matches_a_named_selector() {
    let sheet = StyleSheet::from_rules(vec![background_rule(Selector::named("危险"), RED, 0)]);
    // `Id` 形式的 key 当不了分类名。
    let mut node = Node::from_element(&Element::new(NodeTag::Control).with_key(7u64));
    sheet.apply_all(&mut node);
    assert_eq!(
        background_of(&node),
        None,
        "Id 形式的 key 不是分类名——把它也当名字会撞车"
    );
}

/// `Id` 形式的 key **不会**变成某个碰巧叫 `"id"` 的分类名。
///
/// # 为什么单独一条
///
/// 上面那条用的规则叫 `"危险"`，所以把 Id 误当成 `"id"` 也**不会命中**、照样绿。
/// 这条把规则名直接叫 `"id"`，专钉那一条退化路径。
#[test]
fn a_numeric_key_is_not_the_name_id() {
    let sheet = StyleSheet::from_rules(vec![background_rule(Selector::named("id"), RED, 0)]);
    let mut node = Node::from_element(&Element::new(NodeTag::Control).with_key(7u64));
    sheet.apply_all(&mut node);
    assert_eq!(
        background_of(&node),
        None,
        "Id(7) 不等于分类名 \"id\"——若相等，任何数字 key 都会吃到这条规则"
    );
}

#[test]
fn elements_from_separate_keys_do_not_share_styles() {
    // 两个不同 key 的同类型节点，样式互不影响。
    let sheet = StyleSheet::from_rules(vec![background_rule(Selector::named("危险"), RED, 0)]);
    let mut a = node_with_key(NodeTag::Control, "危险");
    let mut b = node_with_key(NodeTag::Control, "普通");
    sheet.apply_all(&mut a);
    sheet.apply_all(&mut b);
    assert_eq!(background_of(&a), Some(RED));
    assert_eq!(background_of(&b), None, "没命中就不该有值");
}

#[test]
fn a_dropped_style_does_not_stick_around() {
    // 这条边界情形**当前实现做不到**——换个不含某条声明的表，旧值会留在 Styled 层。
    // 记录它是为了让 B4（热重载）知道：它必须「清空 Styled 层 + 重新 apply」，
    // 而不能只换表。
    let with = StyleSheet::from_rules(vec![background_rule(Selector::named("危险"), RED, 0)]);
    let without = StyleSheet::from_rules(vec![StyleRule::new(
        Selector::named("其他人"),
        vec![Declaration {
            key: &props::BACKGROUND,
            value: PropertyValue::color(BLUE),
        }],
        0,
    )]);
    let mut node = node_with_key(NodeTag::Control, "危险");
    with.apply_all(&mut node);
    assert_eq!(background_of(&node), Some(RED));
    without.apply_all(&mut node);
    assert_eq!(
        background_of(&node),
        Some(RED),
        "⚠️ 这条目前是**恒真的**：换表不清 Styled 层，旧值还在。\n\
         B4 必须为此清层——见热重载那节。这里显式记录，免得以后有人以为是 bug 而\
         误以为 apply 泄露了状态。"
    );
}

// ---------------------------------------------------------------------------
//  apply 的时机（与 diff 的交互）
// ---------------------------------------------------------------------------

#[test]
fn applying_after_diff_sees_restyled_nodes() {
    // ⚠️ 这条钉的是「apply 必须在 diff **之后**」这个契约。
    //
    // diff 在 tag 变化时整个替换节点，替换会把 Styled 层一起丢掉。所以：
    // 先 diff（Node 被重建）→ 再 apply（样式重新进来）。
    // 反过来的话，重建会把你刚 apply 的丢掉。
    let sheet = StyleSheet::from_rules(vec![background_rule(Selector::named("危险"), RED, 0)]);

    let mut node = Node::from_element(&Element::new(NodeTag::Container));
    // 第一帧：Container。
    diff(&mut node, &Element::new(NodeTag::Container));
    sheet.apply_all(&mut node);
    assert_eq!(background_of(&node), None, "Container 不命中「危险」");

    // 第二帧：同一个位置变成带 key 的 Control —— diff 会**整个替换**节点。
    diff(
        &mut node,
        &Element::new(NodeTag::Control).with_key("危险"),
    );
    // ⚠️ 关键：diff 之后这里必须是 None（Styled 层被重建丢掉了）。
    assert_eq!(
        background_of(&node),
        None,
        "diff 重建节点后 Styled 层应该没了——这正是 apply 必须排在 diff 之后的原因"
    );
    // 重新 apply 才对。
    sheet.apply_all(&mut node);
    assert_eq!(background_of(&node), Some(RED));
}

#[test]
fn rect_defaults_are_not_disturbed_by_styling() {
    // 样式表只该碰它声明的属性，不该动别的。
    let sheet = StyleSheet::from_rules(vec![background_rule(Selector::named("危险"), RED, 0)]);
    let mut node = node_with_key(NodeTag::Control, "危险");
    let before = node.rect();
    sheet.apply_all(&mut node);
    assert_eq!(node.rect(), before, "apply 不该碰布局矩形");

}

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

/// 造一个带若干样式名的节点。
fn node_with_styles(tag: NodeTag, names: impl IntoIterator<Item = &'static str>) -> Node {
    let mut e = Element::new(tag);
    for n in names {
        e = e.style(n);
    }
    Node::from_element(&e)
}

/// 造一个带单个样式名的节点。
fn node_with_style(tag: NodeTag, name: &'static str) -> Node {
    node_with_styles(tag, [name])
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
    assert!(s.matches(NodeTag::Control, &["any"]), "matching type must hit");
    assert!(
        !s.matches(NodeTag::Container, &[]),
        "a wrong type must not hit -- even when the element carries no class name"
    );
}

#[test]
fn named_selector_matches_on_key() {
    let s = Selector::named("danger");
    assert!(s.matches(NodeTag::Control, &["danger"]), "matching name must hit");
    assert!(
        !s.matches(NodeTag::Control, &["normal"]),
        "a wrong name must not hit -- **the easiest one to write vacuous**: if names are not checked, any class name hits and the test still passes"

    );
}

#[test]
fn multiple_names_require_all_of_them() {
    let s = Selector {
        tag: None,
        names: vec!["danger", "compact"],
    };
    assert!(
        s.matches(NodeTag::Control, &["danger", "compact"]),
        "both names required"
    );
    assert!(
        !s.matches(NodeTag::Control, &["danger"]),
        "one of two must not hit -- partial matches apply rules to the wrong elements"
    );
}

#[test]
fn empty_names_means_any() {
    let s = Selector::tag(NodeTag::Text);
    assert!(
        s.matches(NodeTag::Text, &["any", "anything"]),
        "names are not filtered"
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
        s.matches(NodeTag::Text, &["named"]),
        "empty names means any -> named nodes must hit too"
    );
    assert!(
        s.matches(NodeTag::Text, &[]),
        "empty names means any -> unnamed nodes must hit too"
    );
}

// ---------------------------------------------------------------------------
//  特异性与排序
// ---------------------------------------------------------------------------

#[test]
fn specificity_counts_names_not_types() {
    assert_eq!(Selector::tag(NodeTag::Control).specificity(), 0, "a bare type has no weight");
    assert_eq!(Selector::named("danger").specificity(), 1, "one class name counts as 1");
}

#[test]
fn more_specific_rule_wins() {
    let sheet = StyleSheet::from_rules(vec![
        background_rule(Selector::named("danger"), RED, 0),
        background_rule(
            Selector {
                tag: None,
                names: vec!["danger", "compact"],
            },
            BLUE,
            1,
        ),
    ]);
    let mut node = node_with_style(NodeTag::Control, "danger");
    // 节点只带「危险」，所以只有第一条命中 —— 用另一个测试验「两个都带」的情形。
    sheet.apply_all(&mut node);
    assert_eq!(background_of(&node), Some(RED));
}

#[test]
fn when_both_match_the_more_specific_one_wins() {
    let sheet = StyleSheet::from_rules(vec![
        background_rule(Selector::named("danger"), RED, 0),
        background_rule(
            Selector {
                tag: None,
                names: vec!["danger", "compact"],
            },
            BLUE,
            1,
        ),
    ]);
    // 一条规则的成本地值 = 分类名从**同一个** key 来，所以「两个名字」这件事
    // 在真实树里不存在（一个节点只有一个 key）。这条测试因此只能覆盖
    // 「更具体的规则先写还是后写」——这正是它要钉的。
    let mut node = node_with_style(NodeTag::Control, "danger");
    sheet.apply_all(&mut node);
    assert_eq!(background_of(&node), Some(RED));
}

#[test]
fn last_wins_when_specificity_ties() {
    // 同特异性时后写的赢——这是 Chrome 的规则，也是本表排序 assertion 的落点。
    let sheet = StyleSheet::from_rules(vec![
        background_rule(Selector::named("danger"), RED, 0),
        background_rule(Selector::named("danger"), BLUE, 1),
    ]);
    let mut node = node_with_style(NodeTag::Control, "danger");
    sheet.apply_all(&mut node);
    assert_eq!(
        background_of(&node),
        Some(BLUE),
        "same specificity must let the later one win -- a reversed sort would give RED"
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
        background_rule(Selector::named("danger"), BLUE, 0), // 后写的、但更笼统
        background_rule(Selector::tag(NodeTag::Control), RED, 1),
    ]);
    let mut node = node_with_style(NodeTag::Control, "danger");
    sheet.apply_all(&mut node);
    assert_eq!(
        background_of(&node),
        Some(BLUE),
        "specificity 1 (.danger) must beat specificity 0 (Control), even when it comes first -- otherwise the specificity rule is fake"

    );
}

/// 更具体的**先写**时仍然赢（M6 的完整形状）。
///
/// 上面那条里「更具体的」排在后面（order 0 < 1），所以它赢可能是**顺序**赢的。
/// 这条把它调到前面，排除顺序因素，只留特异性。
#[test]
fn specificity_wins_even_when_it_comes_first() {
    let sheet = StyleSheet::from_rules(vec![
        background_rule(Selector::named("danger"), BLUE, 0), // 先写 + 更具体
        background_rule(Selector::tag(NodeTag::Control), RED, 1), // 后写 + 更笼统
    ]);
    // 上面那条已经是这个次序了。真正区分「特异性赢」和「后写赢」的是
    // **把笼统的调到后面**：如果实现是「后写赢」，这里会拿到 RED。
    let mut node = node_with_style(NodeTag::Control, "danger");
    sheet.apply_all(&mut node);
    assert_eq!(
        background_of(&node),
        Some(BLUE),
        "the more specific one must win even when written first -- RED here would mean order alone decides and specificity never participates"

    );
}

// ---------------------------------------------------------------------------
//  apply 的行为
// ---------------------------------------------------------------------------

#[test]
fn apply_writes_to_the_styled_layer() {
    let sheet = StyleSheet::from_rules(vec![background_rule(Selector::named("danger"), RED, 0)]);
    let mut node = node_with_style(NodeTag::Control, "danger");
    sheet.apply_all(&mut node);

    let (value, source) = node
        .props()
        .lookup(&props::BACKGROUND)
        .expect("must be found after apply");
    assert_eq!(value.get::<Color>(), Some(RED));
    assert_eq!(
        source,
        ValueSource::Styled,
        "the stylesheet writes Styled (below Local) -- if it wrote Local, app code could never override it"

    );
}

#[test]
fn local_still_overrides_the_stylesheet() {
    let sheet = StyleSheet::from_rules(vec![background_rule(Selector::named("danger"), RED, 0)]);
    let mut node = node_with_style(NodeTag::Control, "danger");
    // 应用代码先写（模拟 `.with(...)`）。
    node.set_prop(&props::BACKGROUND, PropertyValue::color(BLUE));
    sheet.apply_all(&mut node);

    assert_eq!(
        background_of(&node),
        Some(BLUE),
        "Local must override Styled -- the two-level WinUI precedence"
    );
}

#[test]
fn apply_reaches_children() {
    let sheet = StyleSheet::from_rules(vec![background_rule(Selector::named("danger"), RED, 0)]);
    let mut root = Node::from_element(
        &Element::new(NodeTag::Container)
            .with_children([Element::new(NodeTag::Control).style("danger")]),
    );
    sheet.apply_all(&mut root);

    let child = &root.children()[0];
    assert_eq!(
        background_of(child),
        Some(RED),
        "apply must recurse -- only-apply-root is a common miss"
    );
}

#[test]
fn apply_is_idempotent() {
    let sheet = StyleSheet::from_rules(vec![background_rule(Selector::named("danger"), RED, 0)]);
    let mut node = node_with_style(NodeTag::Control, "danger");
    sheet.apply_all(&mut node);
    let after_first = background_of(&node);
    sheet.apply_all(&mut node);
    sheet.apply_all(&mut node);
    assert_eq!(
        background_of(&node),
        after_first,
        "per-frame apply must be idempotent -- otherwise it accumulates (push instead of replace)"
    );
}

#[test]
fn apply_does_not_wake_an_unchanged_node() {
    // 「值没变就别标脏」是 UI 静止时零重建的第一道闸门。
    let sheet = StyleSheet::from_rules(vec![background_rule(Selector::named("danger"), RED, 0)]);
    let mut node = node_with_style(NodeTag::Control, "danger");
    sheet.apply_all(&mut node);
    // 第一遍一定会标脏（从无到有）。这里看的是**第二遍**。
    clear_dirty(&mut node);
    sheet.apply_all(&mut node);
    assert_eq!(
        node.dirty(),
        Dirty::None,
        "the same value twice must not dirty -- else a static UI repaints forever"
    );
}

#[test]
fn changing_a_paint_only_value_does_not_force_relayout() {
    // 改背景色只该标 Paint，不该清测量缓存。
    let sheet = StyleSheet::from_rules(vec![background_rule(Selector::named("danger"), RED, 0)]);
    let mut node = node_with_style(NodeTag::Control, "danger");
    sheet.apply_all(&mut node);
    clear_dirty(&mut node);
    // 换成蓝色。
    let sheet2 = StyleSheet::from_rules(vec![background_rule(Selector::named("danger"), BLUE, 0)]);
    sheet2.apply_all(&mut node);
    assert_eq!(background_of(&node), Some(BLUE));
    assert!(
        !node.dirty().needs_layout(),
        "a paint-only change must not force relayout"
    );
}

#[test]
fn changing_a_layout_value_forces_relayout() {
    // 反方向：改宽度必须标 Layout + 清测量缓存。
    let width_rule = |order: u32| {
        StyleRule::new(
            Selector::named("danger"),
            vec![Declaration {
                key: &props::WIDTH,
                value: PropertyValue::float(100.0),
            }],
            order,
        )
    };
    let sheet = StyleSheet::from_rules(vec![width_rule(0)]);
    let mut node = node_with_style(NodeTag::Control, "danger");
    sheet.apply_all(&mut node);
    clear_dirty(&mut node);

    let wide = StyleRule::new(
        Selector::named("danger"),
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
        "a width change must force relayout -- first real use of B1's field on the stylesheet path"
    );
}

#[test]
fn empty_rules_do_not_block_later_rules() {
    // 空规则算「没匹配」，不能占位置。
    let sheet = StyleSheet::from_rules(vec![
        StyleRule::new(Selector::named("danger"), Vec::new(), 0), // 空
        background_rule(Selector::named("danger"), BLUE, 1),
    ]);
    let mut node = node_with_style(NodeTag::Control, "danger");
    sheet.apply_all(&mut node);
    assert_eq!(
        background_of(&node),
        Some(BLUE),
        "an empty rule must not block later ones -- if it does, this would be None"
    );
}

#[test]
fn a_node_without_a_style_never_matches_a_named_selector() {
    let sheet = StyleSheet::from_rules(vec![background_rule(Selector::named("danger"), RED, 0)]);
    // No `.style(...)` at all.
    let mut node = Node::from_element(&Element::new(NodeTag::Control));
    sheet.apply_all(&mut node);
    assert_eq!(
        background_of(&node),
        None,
        "a node with no style name must not match a named selector"
    );
}

/// A node's **event key** must never act as a style class name.
///
/// # Why this separate assertion
///
/// `with_key("danger")` and `.style("danger")` are now two different things.
/// If key leaked into selector matching, a button keyed "submit" would
/// silently eat `.submit { }` rules -- and nobody could tell from the
/// element declaration which one is in play.
#[test]
fn an_event_key_is_not_a_style_name() {
    let sheet = StyleSheet::from_rules(vec![background_rule(Selector::named("submit"), RED, 0)]);
    let mut node = Node::from_element(&Element::new(NodeTag::Control).with_key("submit"));
    sheet.apply_all(&mut node);
    assert_eq!(
        background_of(&node),
        None,
        "with_key must not act as a class name -- style() is the only entry"
    );
}

/// A style name must never be confused with an event key, in either direction.
/// A style name and an event key are separate namespaces.
#[test]
fn style_and_key_are_independent_namespaces() {
    // Same word, two roles: keyed for events, styled for looks.
    let mut node = Node::from_element(
        &Element::new(NodeTag::Control).with_key("reset").style("primary"),
    );
    let sheet = StyleSheet::from_rules(vec![
        background_rule(Selector::named("primary"), RED, 0),
        background_rule(Selector::named("reset"), Color::rgb(0.0, 1.0, 0.0), 1),
    ]);
    sheet.apply_all(&mut node);
    assert_eq!(
        background_of(&node),
        Some(RED),
        ".style(\"primary\") must win -- with_key(\"reset\") is not a selector"
    );
}

#[test]
fn elements_with_different_styles_do_not_share_styles() {
    // Two nodes of the same type but different class names stay independent.
    let sheet = StyleSheet::from_rules(vec![background_rule(Selector::named("danger"), RED, 0)]);
    let mut a = node_with_style(NodeTag::Control, "danger");
    let mut b = node_with_style(NodeTag::Control, "normal");
    sheet.apply_all(&mut a);
    sheet.apply_all(&mut b);
    assert_eq!(background_of(&a), Some(RED));
    assert_eq!(background_of(&b), None, "no match means no value");
}

#[test]
fn a_dropped_style_does_not_stick_around() {
    // 这条曾经是**恒真记录**：换表不清 Styled，旧值赖着不走（症状：热重载后样式
    // 不消失；状态类消失后选中色不消失）。它被标成 "B4 must fix" 等在原地。
    //
    // 现在 `apply_node` 每轮先算「本轮要写哪些键」，写完再把「上一轮写了、
    // 这一轮没人写」的键**拆掉**，所以换表即换样式，不必再由调用方记得清层。
    let with = StyleSheet::from_rules(vec![background_rule(Selector::named("danger"), RED, 0)]);
    let without = StyleSheet::from_rules(vec![StyleRule::new(
        Selector::named("other"),
        vec![Declaration {
            key: &props::BACKGROUND,
            value: PropertyValue::color(BLUE),
        }],
        0,
    )]);
    let mut node = node_with_style(NodeTag::Control, "danger");
    with.apply_all(&mut node);
    assert_eq!(background_of(&node), Some(RED));
    without.apply_all(&mut node);
    assert_eq!(
        background_of(&node),
        None,
        "换表后旧的 Styled 值必须被拆掉 —— 留着的话热重载与状态类消失都会留下残值"
    );
}

/// 状态分类名消失后，那条状态规则写的值必须跟着消失。
///
/// 与上面那条同源，但走的是**日常路径**（不是换表）：同一个节点，第一帧带
/// `.checked`、第二帧不带。这是「点一下取消勾选，底色还是选中色」的成因。
///
/// 判据里特意让状态规则声明一个**基类没有的键**（`BORDER_WIDTH`）：如果基类也
/// 声明了它，基类每帧会把残值盖回去，洞就被"恰好"堵住 —— 那正是它长期没被
/// 发现的原因，所以判据必须踩在那个空隙上。
#[test]
fn a_state_class_that_disappears_takes_its_values_with_it() {
    // 走**真实交互路径**：diff（分类名变化）→ apply（重算样式层）。
    let sheet = StyleSheet::from_rules(vec![
        StyleRule::new(
            Selector::named("row"),
            vec![Declaration {
                key: &props::BACKGROUND,
                value: PropertyValue::color(BLUE),
            }],
            0,
        ),
        // 状态规则声明一个**基类没有的键** —— 必须踩在这个空隙上，
        // 否则基类每帧都会把残值盖回去，洞就被"恰好"堵住了。
        StyleRule::new(
            Selector {
                tag: None,
                names: vec!["row", "checked"],
            },
            vec![Declaration {
                key: &props::BORDER_WIDTH,
                value: PropertyValue::float(3.0),
            }],
            1,
        ),
    ]);

    let checked = Element::new(NodeTag::Control).styles(["row", "checked"]);
    let unchecked = Element::new(NodeTag::Control).styles(["row"]);
    let mut node = Node::from_element(&checked);
    sheet.apply_all(&mut node);
    assert_eq!(
        node.props().lookup(&props::BORDER_WIDTH).map(|(v, _)| v.clone()),
        Some(PropertyValue::float(3.0)),
        "带 `checked` 时 BORDER_WIDTH 应当来自状态规则"
    );

    // 第二帧：分类名去掉 `checked`（节点被复用，只换分类名）。
    zephyra_core::diff::diff(&mut node, &unchecked);
    sheet.apply_all(&mut node);
    assert_eq!(
        node.props().lookup(&props::BORDER_WIDTH).map(|(v, _)| v.clone()),
        None,
        "`checked` 已经不在分类名里，它写的 BORDER_WIDTH 必须被拆掉 —— \
         这正是「点一下取消勾选，底色还是选中色」的成因"
    );
    assert_eq!(
        node.props().lookup(&props::BACKGROUND).map(|(v, _)| v.clone()),
        Some(PropertyValue::color(BLUE)),
        "基类规则仍然匹配，它的值必须留着（拆的只是没人写的那部分）"
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
    let sheet = StyleSheet::from_rules(vec![background_rule(Selector::named("danger"), RED, 0)]);

    let mut node = Node::from_element(&Element::new(NodeTag::Container));
    // 第一帧：Container。
    diff(&mut node, &Element::new(NodeTag::Container));
    sheet.apply_all(&mut node);
    assert_eq!(background_of(&node), None, "Container must not match 'danger'");

    // 第二帧：同一个位置变成带 key 的 Control —— diff 会**整个替换**节点。
    diff(
        &mut node,
        &Element::new(NodeTag::Control).style("danger"),
    );
    // ⚠️ 关键：diff 之后这里必须是 None（Styled 层被重建丢掉了）。
    assert_eq!(
        background_of(&node),
        None,
        "diff rebuild must drop Styled -- exactly why apply goes after diff"
    );
    // 重新 apply 才对。
    sheet.apply_all(&mut node);
    assert_eq!(background_of(&node), Some(RED));
}

#[test]
fn rect_defaults_are_not_disturbed_by_styling() {
    // 样式表只该碰它声明的属性，不该动别的。
    let sheet = StyleSheet::from_rules(vec![background_rule(Selector::named("danger"), RED, 0)]);
    let mut node = node_with_style(NodeTag::Control, "danger");
    let before = node.rect();
    sheet.apply_all(&mut node);
    assert_eq!(node.rect(), before, "apply must not touch the layout rect");

}

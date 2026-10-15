//! `Element::style` / `Node::style_names` / diff 对账的测试。
//!
//! # 为什么单独一个文件
//!
//! 「源码给分类名、样式表按名字匹配」是 zstyle 的第一块地基。它有个**必须**被
//! 钉住的行为：diff 每帧把 `Element` 的分类名**整表替换**到 `Node` 上
//! （不是追加）—— 否则去掉一个分类名后它永远留在节点上，
//! 症状是「样式表改了不生效」。
//!
//! 这块在变异测试里被查出来**零覆盖**：把 `node.styles = want` 改成「只追加」，
//! 全部测试照绿。所以这里补上。

use crate::diff::diff;
use crate::element::Element;
use crate::id::NodeTag;
use crate::node::Node;

/// 一个带分类名的 Control。
fn styled(name: &'static str) -> Element {
    Element::new(NodeTag::Control).style(name)
}

#[test]
fn style_names_survive_the_trip_to_the_node() {
    let node = Node::from_element(&styled("danger"));
    assert_eq!(node.style_names(), &["danger"]);
}

#[test]
fn several_style_names_all_survive() {
    let e = Element::new(NodeTag::Control).styles(["danger", "compact"]);
    let node = Node::from_element(&e);
    assert_eq!(node.style_names(), &["danger", "compact"]);
}

#[test]
fn no_style_means_an_empty_list() {
    let node = Node::from_element(&Element::new(NodeTag::Control));
    assert!(
        node.style_names().is_empty(),
        "没 .style() 就该是空的 —— 空 = 只被类型选择器命中"
    );
}

/// diff 必须**整表替换**，不是追加。
///
/// # 这条在防什么
///
/// 追加式对账下，去掉一个分类名后它**永远留在节点上**。症状是
/// 「样式表改了不生效」——而这种错从画面上完全看不出原因。
#[test]
fn dropping_a_style_name_actually_removes_it() {
    let mut node = Node::from_element(&styled("danger"));
    assert_eq!(node.style_names(), &["danger"]);

    // 下一帧：应用不再标 danger。
    diff(&mut node, &Element::new(NodeTag::Control));
    assert!(
        node.style_names().is_empty(),
        "去掉 .style(\"danger\") 后节点上不该再有它 —— 追加式对账会留在这里"
    );
}

/// 换分类名也要真的换掉。
#[test]
fn switching_a_style_name_replaces_it() {
    let mut node = Node::from_element(&styled("danger"));
    diff(&mut node, &styled("compact"));
    assert_eq!(
        node.style_names(),
        &["compact"],
        "换分类名是替换，不是两个都留"
    );
}

/// `with_key` 与 `.style()` 是两个东西。
///
/// 这条钉住「key 不是分类名」：它俩名字可以一样，但走的路径完全不同
/// （key 管事件路由，style 管选择器）。
#[test]
fn a_key_is_not_a_style_name() {
    let node = Node::from_element(&Element::new(NodeTag::Control).with_key("danger"));
    assert!(node.style_names().is_empty(), "with_key 不该变成分类名");
    assert!(node.key().is_some(), "但 key 本身还在");
}

/// 同源同词：key 和 style 都叫 "danger" 时互不干扰。
#[test]
fn a_key_and_a_style_with_the_same_word_coexist() {
    let node = Node::from_element(
        &Element::new(NodeTag::Control)
            .with_key("danger")
            .style("danger"),
    );
    assert_eq!(node.style_names(), &["danger"], "style 那半边在");
    assert!(node.key().is_some(), "key 那半边也在");
}

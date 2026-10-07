//! 地基验收示例：把一棵节点树降为 [`zephyra::paint::DisplayList`]。
//!
//! 这是 ZephyraUI 骨架阶段的**验收标准**——不是演示，而是契约的可执行陈述：
//!
//! 1. 核心**零依赖**（`cargo tree -p zephyra` 为空）；
//! 2. 一棵声明树能落地成节点树；
//! 3. 节点树能降级为 `DisplayList`，且 `DisplayList` 已按 `(layer, z, order)`
//!    **稳定**排序；
//! 4. 稳态（元素树不变时）diff 是 no-op——「不动的时候真的不动」；
//! 5. 属性相等可判定，因此颜色不变就不标脏。
//!
//! 用 `cargo run -p zephyra-foundation` 运行。

use zephyra::core::Rect;
use zephyra::paint::{Command, DisplayList, Layer, SortKey};
use zephyra::property::{PropertyKey, PropertyValue};
use zephyra::props;
use zephyra::theme::{roles, Theme, ThemeVariant};
use zephyra::context::Context;
use zephyra::diff::{diff, DiffStats};
use zephyra::element::Element;
use zephyra::id::NodeTag;
use zephyra::node::{Dirty, Node};
use zephyra::paint::render;

/// 内容前景色（语义名 → 主题角色）。
const CONTENT: &PropertyKey = &PropertyKey::new("Content", PropertyValue::float(0.0));

/// 可继承的强调色槽位。
const ACCENT_FILL: &PropertyKey =
    &PropertyKey::inheritable("AccentFill", PropertyValue::float(0.0));

/// 声明一棵界面：标题 + 两个按钮。
fn ui_element() -> Element {
    Element::new(NodeTag::Container)
        .with_key("root")
        // 必须给根一个背景色。真实管线里"没有背景"意味着**不画任何东西**，所以一棵
        // 全透明的树会产出空的 DisplayList——`tests::render_draws_nothing_when_no_node_has_a_background`
        // 钉的就是这条。
        .with(&props::BACKGROUND, PropertyValue::color(Theme::dark().surface))
        .with_children([
            Element::new(NodeTag::Text).with_key("title"),
            Element::new(NodeTag::Control).with_key("ok").with(
                ACCENT_FILL,
                PropertyValue::Resource(roles::ACCENT_FILL.into()),
            ),
            Element::new(NodeTag::Control).with_key("cancel"),
        ])
}

/// 断言：DisplayList 已稳定排序，且每个 `(layer, z, order)` 组合唯一。
fn assert_sorted_strictly(list: &DisplayList) {
    let mut prev: Option<SortKey> = None;
    for item in list.iter() {
        if let Some(p) = prev {
            assert!(
                p < item.key,
                "DisplayList 未严格有序: {p:?} 然后 {:?}",
                item.key
            );
        }
        prev = Some(item.key);
    }
}

fn main() {
    // ---- 1. 零依赖 ----
    // `cargo tree -p zephyra` 应无输出。这条断言无法在运行时自证，
    // 故由 CI 检查；此处仅在文档中标注。

    // ---- 2. 声明 → 节点 ----
    let element = ui_element();
    assert!(!element.is_leaf());
    assert_eq!(element.children().len(), 3);
    let mut node = Node::from_element(&element);
    assert_eq!(node.children().len(), 3);
    // 新节点必然需要一次完整布局。
    assert_eq!(node.dirty(), Dirty::Structure);

    // ---- 3. 布局 → DisplayList ----
    //
    // 分两步是刻意的：`render` 只读已摆好的矩形，几何计算全在 `layout` 里。
    // 这样命中测试与绘制共用同一批矩形，不会出现"看得见但点不到"。
    let bounds = Rect::new(0.0, 0.0, 800.0, 600.0);
    let theme = Theme::dark();
    let ctx = Context::new(&theme);
    zephyra::layout::layout(&mut node, bounds, ctx);

    let mut list = DisplayList::new();
    render(&node, ctx, &mut list);
    assert!(!list.is_empty(), "DisplayList 不应为空");
    assert_sorted_strictly(&list);
    assert!(list.has_layer(Layer::Content));
    // 顺序稳定：同样输入产出同样列表（纹理缓存命中率的前提）。
    let mut again = DisplayList::new();
    render(&node, ctx, &mut again);
    assert_eq!(list, again, "同样输入必须产出逐字节相同的 DisplayList");

    // 圆角矩形是 draw-call 级别的图元，不需要 CPU 三角化。
    assert!(
        list.iter()
            .any(|i| matches!(i.command, Command::Quad { .. })),
        "容器背景应降级为 Quad"
    );

    // ---- 4. 稳态 diff 是 no-op ----
    //
    // 先走一遍「布局 + 渲染」并清脏——真实宿主每帧末尾都做这一步。不清的话脏
    // 标记只增不减，本帧要不要重排的判断就失去意义。
    zephyra::node::clear_dirty(&mut node);
    let stats: DiffStats = diff(&mut node, &ui_element());
    assert!(
        stats.is_noop(),
        "元素树不变时 diff 应无任何变化，实得 {stats:?}"
    );

    // ---- 5. 属性变化才标脏 ----
    let changed = Element::new(NodeTag::Container)
        .with_key("root")
        .with(
            CONTENT,
            PropertyValue::color(zephyra::theme::Theme::dark().content),
        )
        .with_children([
            Element::new(NodeTag::Text).with_key("title"),
            Element::new(NodeTag::Control).with_key("ok").with(
                ACCENT_FILL,
                PropertyValue::Resource(roles::ACCENT_FILL.into()),
            ),
            Element::new(NodeTag::Control).with_key("cancel"),
        ]);
    let stats = diff(&mut node, &changed);
    assert!(!stats.is_noop(), "属性新增应被 diff 检出");
    assert!(
        node.props().is_locally_set(CONTENT),
        "新增属性应写入节点属性表"
    );

    // 属性值相等可判定 —— 这是上一条断言（稳态 no-op）成立的前提。
    assert_eq!(
        PropertyValue::float(1.0),
        PropertyValue::float(1.0),
        "同值属性必须判等"
    );
    assert_ne!(PropertyValue::float(1.0), PropertyValue::float(2.0));

    // ---- 6. 主题：深浅两套都有可查的语义角色 ----
    for variant in [ThemeVariant::Light, ThemeVariant::Dark] {
        let theme = Theme::builtin(variant);
        for role in [
            roles::SURFACE,
            roles::CONTENT,
            roles::STROKE,
            roles::ACCENT_FILL,
            roles::FOCUS_RING,
        ] {
            assert!(
                theme.role(role).is_some(),
                "{variant:?} 主题缺少语义角色 {role}"
            );
        }
    }

    // 深浅两套的 surface 必须不同 —— 否则「切主题」在视觉上没有任何效果，
    // 而这正是最容易被悄悄做坏的地方。
    let light = Theme::builtin(ThemeVariant::Light);
    let dark = Theme::builtin(ThemeVariant::Dark);
    assert_ne!(light.surface, dark.surface);

    println!("ZephyraUI 地基验收通过：");
    println!("  DisplayList 命令数     {}", list.len());
    println!("  合成层                 {:?}", list.layers());
    println!("  根节点子项数           {}", node.children().len());
    println!("  稳态 diff              no-op ✓");
    println!("  语义角色               Light/Dark 均可查 ✓");
}

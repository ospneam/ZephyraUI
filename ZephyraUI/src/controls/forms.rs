//! BasicInput 第二批：RoundButton / Hyperlink / DropDownButton / ComboBox /
//! SpinBox / ColorPicker。
//!
//! 度量出处（RinUI 侧逐条可查）：
//! - RoundButton：`BasicInput/RoundButton.qml` —— `radius: height / 2`，
//!   `controlQuaternaryColor` 底（= 我们的 `control_tertiary`）。
//! - Hyperlink：`BasicInput/Hyperlink.qml` —— flat + highlighted（accent 字）。
//! - DropDownButton：`BasicInput/DropDownButton.qml` —— Button + 后缀
//!   chevron-down 图标（spacing 8，`suffixIconName`）。
//! - ComboBox：`BasicInput/ComboBox.qml` —— 框 + 32×24 扁平 chevron
//!   （icon size 14，textSecondaryColor）。
//! - SpinBox：`Text/SpinBox.qml` —— 底 field + 右两枚 32×24 flat
//!   chevron 按钮（icon size 16），left/right padding 12。
//! - ColorPicker：`BasicInput/ColorPicker.qml` —— 默认色板 16 格
//!   （每格默认 24×24、radius 3、选中描边 2px）。
//!
//! # 交互降级的记账（core 能力边界的诚实裁剪）
//!
//! - ComboBox 的**展开浮层**不做（core 无 overlay 层）—— 点击循环切换选项，
//!   由宿主状态驱动（key 路由一样工作）。
//! - SpinBox 的**键入**不做（core 无输入法）—— 值由宿主给，上下按钮是
//!   真交互。
//! - ColorPicker 的**取色交互**（HSV 面板 / 滑块）不做 —— 迁的是默认色板
//!   网格与选中态。色板本身是**样式表**里的 16 个类（`sw-0`..`sw-15`），
//!   应用可在自己的样式表里改写它们 —— 源码里一个颜色字面量都没有。

use zephyra_core::element::Element;
use zephyra_core::id::NodeTag;
use zephyra_core::property::PropertyValue;
use zephyra_core::props;

use super::{state, with_classes};

/// Hyperlink 的 URL：**数据**（内容），不是外观。
///
/// 宿主自定义属性键的手法与 demo 的 `NOTE` 相同：控件不写死去向，只把话
/// 带到；点下去之后去哪，是应用按 key 路由时自己决定的事。
pub static HYPERLINK_URL: zephyra_core::property::PropertyKey =
    zephyra_core::property::PropertyKey::new("Hyperlink.Url", PropertyValue::static_str(""));

/// ColorPicker 默认色板的分类名（16 格）。
///
/// 颜色在 `ui.zstyle` 的 `Container.sw-*` 规则里 —— 源码发的是**词**不是值，
/// 与"variant"同一机制。应用想换色板，在自己的样式表里改写这 16 个类即可。
pub const PALETTE: [&str; 16] = [
    "sw-0", "sw-1", "sw-2", "sw-3", "sw-4", "sw-5", "sw-6", "sw-7", "sw-8", "sw-9", "sw-10",
    "sw-11", "sw-12", "sw-13", "sw-14", "sw-15",
];

/// 圆角按钮（RinUI RoundButton）：正圆 / 全圆角。
///
/// 与 `pill_button` 的差别只有圆角词：`round` 的 radius 由样式表给 99。
/// `icon` 为 `None` 时退化成文字药丸。
pub fn round_button(
    key: &'static str,
    label: &'static str,
    icon: Option<&'static str>,
    checked: bool,
    hovered: bool,
    pressed: bool,
) -> Element {
    let mut classes: Vec<&'static str> = vec!["round"];
    if checked {
        classes.push("checked");
    }
    classes.extend(state(hovered, pressed));
    let mut children = Vec::new();
    if let Some(icon) = icon {
        children.push(with_classes(
            Element::new(NodeTag::Container)
                .with_key("icon")
                .with(&props::ICON_NAME, PropertyValue::static_str(icon)),
            &classes,
        ));
    }
    children.push(with_classes(
        Element::new(NodeTag::Text)
            .with_key("label")
            .with(&props::TEXT, PropertyValue::static_str(label)),
        &classes,
    ));
    let children: Vec<Element> = children
        .into_iter()
        .map(|c| match c.tag() {
            NodeTag::Text => c.style("round-label"),
            _ => c.style("round-icon"),
        })
        .collect();
    with_classes(
        Element::new(NodeTag::Control)
            .with_key(key)
            .with(&props::INTERACTIVE, PropertyValue::bool(true))
            .style("round"),
        &classes,
    )
    .with_children(children)
}

/// 超链接（RinUI Hyperlink）：accent 字 + 下划线。
///
/// 下划线是常驻子节点（1px 直棒）：hover 时颜色加深由样式表说，源码不发值。
/// `url` 是数据（[`HYPERLINK_URL`]），不是样式。
pub fn hyperlink(
    key: &'static str,
    text: &'static str,
    url: &'static str,
    hovered: bool,
) -> Element {
    let classes = state(hovered, false);
    with_classes(
        Element::new(NodeTag::Control)
            .with_key(key)
            .with(&props::INTERACTIVE, PropertyValue::bool(true))
            .with(&HYPERLINK_URL, PropertyValue::static_str(url))
            .style("hyperlink")
            .with_children([
                with_classes(
                    Element::new(NodeTag::Text)
                        .with_key("label")
                        .with(&props::TEXT, PropertyValue::static_str(text)),
                    &classes,
                )
                .style("hyperlink-label"),
                with_classes(
                    Element::new(NodeTag::Container)
                        .with_key("underline")
                        .style("link-underline"),
                    &classes,
                ),
            ]),
        &classes,
    )
}

/// 下拉按钮（RinUI DropDownButton）：按钮 + 后缀 chevron-down。
///
/// `open` 是分类名（不是数值）：展开态的长相（背景 / chevron 色）由样式表说。
pub fn drop_down_button(
    key: &'static str,
    label: &'static str,
    icon: Option<&'static str>,
    open: bool,
    hovered: bool,
    pressed: bool,
) -> Element {
    let mut classes = vec!["drop-down"];
    if open {
        classes.push("open");
    }
    classes.extend(state(hovered, pressed));
    let mut children = Vec::new();
    if let Some(icon) = icon {
        children.push(
            Element::new(NodeTag::Container)
                .with_key("icon")
                .with(&props::ICON_NAME, PropertyValue::static_str(icon))
                .style("dd-icon"),
        );
    }
    children.push(
        Element::new(NodeTag::Text)
            .with_key("label")
            .with(&props::TEXT, PropertyValue::static_str(label))
            .style("dd-label"),
    );
    children.push(
        Element::new(NodeTag::Container)
            .with_key("chevron")
            .with(&props::ICON_NAME, PropertyValue::static_str("chevron-down"))
            .style("dd-chevron"),
    );
    with_classes(
        Element::new(NodeTag::Control)
            .with_key(key)
            .with(&props::INTERACTIVE, PropertyValue::bool(true))
            .style("drop-down"),
        &classes,
    )
    .with_children(children)
}

/// 下拉框（RinUI ComboBox）：框 + 当前值 + chevron。
///
/// 展开浮层**不做**（core 无 overlay 层）：点击由宿主推进选中项（key 路由
/// 不变，交互语义从"展开选"降级为"循环切"，已在 MD 记账）。
///
/// `value` 由**宿主**把下标映射成文字后给出（`&'static`）：控件不持有选项表，
/// 也就不会为了显示当前值去 `format!` —— `view()` 每帧跑一次，那会是每帧泄漏。
/// 下拉选项本身是**内容**，样式表碰不到。
pub fn combo_box(key: &'static str, value: &'static str, hovered: bool, pressed: bool) -> Element {
    let el = with_classes(
        Element::new(NodeTag::Control)
            .with_key(key)
            .with(&props::INTERACTIVE, PropertyValue::bool(true))
            .style("combo-box"),
        &state(hovered, pressed),
    );
    el.with_children([
        Element::new(NodeTag::Text)
            .with_key("value")
            .with(&props::TEXT, PropertyValue::static_str(value))
            .style("combo-value"),
        Element::new(NodeTag::Container)
            .with_key("chevron")
            .with(&props::ICON_NAME, PropertyValue::static_str("chevron-down"))
            .style("combo-chevron"),
    ])
}
#[allow(clippy::too_many_arguments)]
// 参数是画一次所需的全部输入，彼此没有可成组打包的语义 —— 凑一个 struct 只是把
// 8 个参数搬进另一个名字，读的人反而要多跳一层。见 AGENTS.md「代码自解释优先」。
/// 数字框（RinUI SpinBox）：值 + 右两枚 32×24 扁平上/下按钮。
///
/// `value` 是内容（当前数值的文字形式）。上/下按钮各自带 hover/pressed
/// 状态类 —— 它们是真交互（宿主按 key 增减）。
pub fn spin_box(
    key: &'static str,
    value: &'static str,
    hovered: bool,
    pressed: bool,
    up_hovered: bool,
    up_pressed: bool,
    down_hovered: bool,
    down_pressed: bool,
) -> Element {
    let step = |key: &'static str, icon: &'static str, h: bool, p: bool| {
        with_classes(
            Element::new(NodeTag::Control)
                .with_key(key)
                .with(&props::INTERACTIVE, PropertyValue::bool(true))
                .style("spin-step"),
            &state(h, p),
        )
        .with_children([Element::new(NodeTag::Container)
            .with_key("icon")
            .with(&props::ICON_NAME, PropertyValue::static_str(icon))
            .style("spin-step-icon")])
    };
    with_classes(
        Element::new(NodeTag::Control)
            .with_key(key)
            .with(&props::INTERACTIVE, PropertyValue::bool(true))
            .style("spin-box"),
        &state(hovered, pressed),
    )
    .with_children([
        Element::new(NodeTag::Text)
            .with_key("value")
            .with(&props::TEXT, PropertyValue::static_str(value))
            .style("spin-value"),
        step("spin:up", "chevron-up", up_hovered, up_pressed),
        step("spin:down", "chevron-down", down_hovered, down_pressed),
    ])
}

/// 色板（RinUI ColorPicker 的默认网格部分）。
///
/// 16 格，每格 24×24、radius 3；选中格 2px 强调描边（`.selected`）。
/// 格子的**颜色**在样式表的 `sw-0..15` 里（见 [`PALETTE`]）—— 换色板改写
/// 那 16 个类即可，源码零颜色字面量。
pub fn color_picker(key: &'static str, selected: usize) -> Element {
    // 4×4 网格：core 没有 Grid，按行手排（与 gallery 的 clip_grid 同手法）。
    let rows: Vec<Element> = PALETTE
        .chunks(4)
        .enumerate()
        .map(|(r, chunk)| {
            let cells: Vec<Element> = chunk
                .iter()
                .enumerate()
                .map(|(c, sw)| {
                    let index = r * 4 + c;
                    let mut cell = Element::new(NodeTag::Container)
                        .with_key(gen_swatch_key(index))
                        .style("swatch")
                        .style(sw);
                    if index == selected {
                        cell = cell.style("selected");
                    }
                    cell
                })
                .collect();
            Element::new(NodeTag::Container)
                .with_key(row_key(r))
                .style("swatch-row")
                .with_children(cells)
        })
        .collect();
    Element::new(NodeTag::Container)
        .with_key(key)
        .style("color-picker")
        .with_children(rows)
}

/// 色板行的 key（纯身份，不参与路由）。
fn row_key(r: usize) -> &'static str {
    match r {
        0 => "sw-row:0",
        1 => "sw-row:1",
        2 => "sw-row:2",
        _ => "sw-row:3",
    }
}

/// 色格的 key（纯身份，不参与路由）。
fn gen_swatch_key(i: usize) -> &'static str {
    const KEYS: [&str; 16] = [
        "swatch:0",
        "swatch:1",
        "swatch:2",
        "swatch:3",
        "swatch:4",
        "swatch:5",
        "swatch:6",
        "swatch:7",
        "swatch:8",
        "swatch:9",
        "swatch:10",
        "swatch:11",
        "swatch:12",
        "swatch:13",
        "swatch:14",
        "swatch:15",
    ];
    KEYS[i.min(15)]
}

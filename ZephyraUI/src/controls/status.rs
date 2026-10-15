//! StatusAndInfo 第二批：ProgressRing / InfoBar / Toast / ToolTip /
//! BusyIndicator / PageIndicator。
//!
//! 度量出处（RinUI）：
//! - ProgressRing：`StatusAndInfo/ProgressRing.qml` —— 56×56、strokeWidth 6、
//!   running/paused/error 三态换色（accent / caution / critical）。
//! - InfoBar：`StatusAndInfo/InfoBar.qml` —— Frame 卡（cardColor 系底，
//!   但按 severity 换 `system*BackgroundColor`）+ 左 icon + 标题/正文 +
//!   右侧关闭钮。
//! - Toast：`StatusAndInfo/Toast.qml` —— 与 InfoBar 同骨架，用**实心**
//!   严重级底 + 白字。
//! - ToolTip：`StatusAndInfo/ToolTip.qml` —— acrylic 底（= 我们的 layer 档）、
//!   cardBorder 边、radius 5、12px 正文。
//! - BusyIndicator：继承 ProgressRing 的 indeterminate（三档静态演示）。
//! - PageIndicator：`Scrolling/PageIndicator.qml` —— 圆点行，当前项是宽条。
//!
//! # 两个诚实的取舍
//!
//! 1. **ProgressRing 没有弧**：core 布局没有绝对定位，12 个点围不成圆。
//!    迁的是"圆环 + 中央数值"这个形态（圆环 = 粗边透明底容器），弧长进度
//!    降级为环色 + 数值。要真弧线，得等 core 长出绝对定位或弧图元。
//! 2. **动画全部没有**：框架没有时间轴，BusyIndicator 的三档由调用方按帧给
//!    `frame` 参数（0..2）—— 那是宿主的事，控件只负责"这一档长什么样"。

use zephyra_core::element::Element;
use zephyra_core::id::NodeTag;
use zephyra_core::property::PropertyValue;
use zephyra_core::props;

use super::{state, with_classes};

/// 进度环（RinUI ProgressRing 的圆环 + 数值形态）。
///
/// `state` ∈ `running` / `paused` / `error`（换环色）；`value` 是中央数值的
/// 文字（内容）。弧进度不可用（见模块文档第 1 条）。
pub fn progress_ring(
    key: &'static str,
    value: &'static str,
    state: &'static str,
    show_value: bool,
) -> Element {
    // 圆环 = 56×56 正圆 + 6px 粗边 + 透明底（core 的 border 就是描边环）。
    let mut ring = Element::new(NodeTag::Container)
        .with_key("ring")
        .style("ring-track")
        .style(state);
    if show_value {
        ring = ring.with_children([Element::new(NodeTag::Text)
            .with_key("value")
            .with(&props::TEXT, PropertyValue::static_str(value))
            .style("ring-value")]);
    }
    Element::new(NodeTag::Container)
        .with_key(key)
        .style("progress-ring")
        .with_children([ring])
}

/// 信息条（RinUI InfoBar）：severity 背景卡 + 图标 + 标题/正文 + 关闭钮。
///
/// `severity` ∈ `info` / `success` / `caution` / `error`（背景令牌
/// `*_bg`；info 用 `surface_alt`）。`closable` 控制关闭钮的**折叠**
/// （COLLAPSED：不占位不绘制），不增删结构。
pub fn info_bar(
    key: &'static str,
    title: &'static str,
    text: &'static str,
    severity: &'static str,
    closable: bool,
) -> Element {
    let mut close = Element::new(NodeTag::Control)
        .with_key("bar:close")
        .with(&props::INTERACTIVE, PropertyValue::bool(true))
        .style("bar-close");
    if !closable {
        close = close.with(&props::COLLAPSED, PropertyValue::bool(true));
    }
    close = close.with_children([Element::new(NodeTag::Container)
        .with_key("icon")
        .with(&props::ICON_NAME, PropertyValue::static_str("close"))
        .style("bar-close-icon")]);

    Element::new(NodeTag::Container)
        .with_key(key)
        .style("info-bar")
        .style(severity)
        .with_children([
            Element::new(NodeTag::Container)
                .with_key("bar-icon")
                .with(
                    &props::ICON_NAME,
                    PropertyValue::static_str(icon_for(severity)),
                )
                .style("bar-icon")
                .style(severity),
            Element::new(NodeTag::Container)
                .with_key("texts")
                .style("bar-texts")
                .with_children([
                    Element::new(NodeTag::Text)
                        .with_key("title")
                        .with(&props::TEXT, PropertyValue::static_str(title))
                        .style("bar-title"),
                    Element::new(NodeTag::Text)
                        .with_key("text")
                        .with(&props::TEXT, PropertyValue::static_str(text))
                        .style("bar-text"),
                ]),
            close,
        ])
}

/// 提示浮条的卡片形态（RinUI Toast）：实心严重级底 + 白字。
///
/// 白字是**分类名**下发给两个文本子节点（选择器没有后代组合器，
/// 子的颜色只能由自己的类说）。
pub fn toast(
    key: &'static str,
    title: &'static str,
    text: &'static str,
    severity: &'static str,
) -> Element {
    let title_el = with_classes(
        Element::new(NodeTag::Text)
            .with_key("title")
            .with(&props::TEXT, PropertyValue::static_str(title)),
        &[severity],
    )
    .style("bar-title");
    let text_el = with_classes(
        Element::new(NodeTag::Text)
            .with_key("text")
            .with(&props::TEXT, PropertyValue::static_str(text)),
        &[severity],
    )
    .style("bar-text");
    Element::new(NodeTag::Container)
        .with_key(key)
        .style("toast")
        .style(severity)
        .with_children([
            Element::new(NodeTag::Container)
                .with_key("toast-icon")
                .with(
                    &props::ICON_NAME,
                    PropertyValue::static_str(icon_for(severity)),
                )
                .style("bar-icon")
                .style(severity),
            Element::new(NodeTag::Container)
                .with_key("texts")
                .style("bar-texts")
                .with_children([title_el, text_el]),
        ])
}

/// 工具提示（RinUI ToolTip）： acrylic 卡 + 12px 正文。
pub fn tool_tip(key: &'static str, text: &'static str) -> Element {
    Element::new(NodeTag::Container)
        .with_key(key)
        .style("tool-tip")
        .with_children([Element::new(NodeTag::Text)
            .with_key("text")
            .with(&props::TEXT, PropertyValue::static_str(text))
            .style("tip-text")])
}

/// 忙碌指示（RinUI BusyIndicator）：三点，`frame` ∈ 0..2 决定哪颗大。
///
/// 框架没有时间轴，所以"动"由宿主按帧给参数 —— 控件只画这一帧。
pub fn busy_indicator(key: &'static str, frame: usize) -> Element {
    let sizes = ["dot-lg", "dot-md", "dot-sm"];
    let dots: Vec<Element> = (0..3)
        .map(|i| {
            // 三颗点按 frame 轮换大小档：0→大中小，1→小大中，2→中小大。
            let order = [(i + 3 - frame % 3) % 3];
            let size_class = sizes[order[0]];
            Element::new(NodeTag::Container)
                .with_key(dot_key(i))
                .style("busy-dot")
                .style(size_class)
        })
        .collect();
    Element::new(NodeTag::Container)
        .with_key(key)
        .style("busy")
        .with_children(dots)
}

/// 页面指示器（RinUI PageIndicator）：圆点行，当前项是宽条。
pub fn page_indicator(key: &'static str, count: usize, current: usize) -> Element {
    let dots: Vec<Element> = (0..count)
        .map(|i| {
            let mut d = Element::new(NodeTag::Container)
                .with_key(pip_key(i))
                .style("pip");
            if i == current {
                d = d.style("selected");
            }
            d
        })
        .collect();
    Element::new(NodeTag::Container)
        .with_key(key)
        .style("page-indicator")
        .with_children(dots)
}

// ---------------------------------------------------------------- 内部

fn icon_for(severity: &str) -> &'static str {
    match severity {
        "success" => "check",
        "caution" => "bell",
        "error" => "close",
        _ => "bell",
    }
}

fn dot_key(i: usize) -> &'static str {
    ["busy:0", "busy:1", "busy:2"][i.min(2)]
}

fn pip_key(i: usize) -> &'static str {
    ["pip:0", "pip:1", "pip:2", "pip:3", "pip:4"][i.min(4)]
}

/// 让「纯展示件」也能挂交互态（调用方不用时传 false/false）。
#[allow(dead_code)]
fn idle(el: Element) -> Element {
    with_classes(el, &state(false, false))
}

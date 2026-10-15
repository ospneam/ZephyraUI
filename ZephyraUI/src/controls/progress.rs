//! 滑条与进度条 —— 唯一允许「源码写几何」的两个控件。
//!
//! # 为什么填充宽度是内容而不是版式
//!
//! RinUI 的滑条填充宽 = `visualPosition × 轨道宽`，进度条填充宽 =
//! `value × 轨道宽`。这个宽度是**值**的可视化，像文字一样属于内容；
//! 而"轨道多高、什么颜色、圆角多大"才是版式，那些仍在样式表里。
//!
//! 本 crate 的规矩因此留了一个**有形状的例外**：只有 key 以 `fill:` 开头的
//! 节点允许在源码里写 `WIDTH`，其余一概不行（见 `tests/gates.rs` 的
//! `only_fill_nodes_carry_local_width`）。Blacklist 会烂，形状不会。
//!
//! # 轨道宽度只有一个真相
//!
//! 填充宽 = 值 × `TRACK`，而轨道自身的宽度在样式表里。两处各写一个数就会
//! 漂（症状：拖到 100% 还差一截，且没有任何报错）。所以轨道宽度在
//! **这里**定义一次，样式表里的值由门禁 `track_width_matches_stylesheet`
//! 逐字比对 —— 不一致就让 `cargo test` 红。

use zephyra_core::element::Element;
use zephyra_core::id::NodeTag;
use zephyra_core::property::PropertyValue;
use zephyra_core::props;

use super::state;

/// 滑条轨道宽度（DIP）。样式表 `Control.slider-track` 的 width 必须与它相等。
pub const SLIDER_TRACK: f32 = 220.0;

/// 进度条轨道宽度（DIP）。样式表 `Control.progress` 的 width 必须与它相等。
pub const PROGRESS_TRACK: f32 = 240.0;

/// 滑条（RinUI Slider）：轨道 4px 药丸 + 强调填充 + 20px 手柄。
///
/// 手柄不做绝对居中（core 没有负边距），而是紧跟填充末端 —— flex 布局天然
/// 把它推到值的位置上，零坐标计算。
pub fn slider(
    key: &'static str,
    value01: f32,
    label: Option<&'static str>,
    hovered: bool,
    pressed: bool,
) -> Element {
    let v = value01.clamp(0.0, 1.0);
    let mut el = Element::new(NodeTag::Control)
        .with_key(key)
        .with(&props::INTERACTIVE, PropertyValue::bool(true))
        .style("slider");
    for s in state(hovered, pressed) {
        el = el.style(s);
    }
    let fill = Element::new(NodeTag::Container)
        .with_key("fill:slider")
        .style("slider-fill")
        .with(&props::WIDTH, PropertyValue::float(v * SLIDER_TRACK));
    let track = Element::new(NodeTag::Container)
        .with_key("track")
        .style("slider-track")
        .with_children([
            fill,
            Element::new(NodeTag::Container)
                .with_key("thumb")
                .style("slider-thumb"),
        ]);
    let mut children = vec![track];
    if let Some(text) = label {
        children.push(
            Element::new(NodeTag::Text)
                .with_key("value")
                .with(&props::TEXT, PropertyValue::static_str(text))
                .style("slider-value"),
        );
    }
    el.with_children(children)
}

/// 进度条（RinUI ProgressBar）：3px 药丸轨道 + 强调填充。
///
/// `state` 是运行态分类名（`running` / `paused` / `error`）—— RinUI 那三种
/// 状态的颜色（主色 / caution / critical）全在样式表里。
pub fn progress_bar(key: &'static str, value01: f32, state: &'static str) -> Element {
    let v = value01.clamp(0.0, 1.0);
    let fill = Element::new(NodeTag::Container)
        .with_key("fill:progress")
        .style("progress-fill")
        .style(state)
        .with(&props::WIDTH, PropertyValue::float(v * PROGRESS_TRACK));
    Element::new(NodeTag::Control)
        .with_key(key)
        .with(&props::INTERACTIVE, PropertyValue::bool(true))
        .style("progress")
        .with_children([fill])
}

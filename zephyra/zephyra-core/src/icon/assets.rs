//! 随框架分发的通用 SVG 字形（编译期内嵌，运行时零 IO）。
//!
//! 用 `include_str!` 暴露成 `&'static str`，让调用方**不**自己写跨目录的
//! `include_str!` 路径——那种路径会在目录层级调整时静默失效，且每个消费方都要
//! 重复一遍。
//!
//! 这里放的是**任何** UI 都要用的通用字形（菜单、画笔、撤销、图片…）。应用专属
//! 的图标不进这里：框架不该知道某个应用有个"拍照"按钮，那属于
//! [`crate::icon::IconSet::with`]。
//!
//! 新增图标：把 `.svg` 放进 `assets/icons/`，加一条 `include_str!` 常量，
//! 再登记进 [`REGISTRY`]。
//!
//! [`REGISTRY`]: crate::icon::assets::REGISTRY

use super::IconSource;

/// 相机。
pub const CAMERA: &str = include_str!("assets/icons/camera.svg");
/// 日历。
pub const CALENDAR: &str = include_str!("assets/icons/calendar.svg");
/// 向下 V 形（下拉 / 展开指示）。
pub const CHEVRON_DOWN: &str = include_str!("assets/icons/chevron-down.svg");
/// 向上 V 形（收起 / 数值加）。
pub const CHEVRON_UP: &str = include_str!("assets/icons/chevron-up.svg");
/// 时钟。
pub const CLOCK: &str = include_str!("assets/icons/clock.svg");
/// 关闭（资源文件名为 `dismiss`，`assets` 中无 `close.svg`）。
pub const DISMISS: &str = include_str!("assets/icons/dismiss.svg");
/// 橡皮擦。
pub const ERASER: &str = include_str!("assets/icons/eraser.svg");
/// 齿轮。
pub const GEAR: &str = include_str!("assets/icons/gear.svg");
/// 房子（主页）。
pub const HOME: &str = include_str!("assets/icons/home.svg");
/// 图片。
pub const IMAGE: &str = include_str!("assets/icons/image.svg");
/// 补光灯（关）。
pub const LIGHT_OFF: &str = include_str!("assets/icons/light-off.svg");
/// 补光灯（开）。
pub const LIGHT: &str = include_str!("assets/icons/light.svg");
/// 菜单。
pub const MENU: &str = include_str!("assets/icons/menu.svg");
/// 最小化。
pub const MINIMIZE: &str = include_str!("assets/icons/minimize.svg");
/// 移动。
pub const MOVE: &str = include_str!("assets/icons/move.svg");
/// 画笔。
pub const PEN: &str = include_str!("assets/icons/pen.svg");
/// 设置。
pub const SETTINGS: &str = include_str!("assets/icons/settings.svg");
/// 撤销。
pub const UNDO: &str = include_str!("assets/icons/undo.svg");
/// 对勾。
pub const CHECK: &str = include_str!("assets/icons/check.svg");
/// 放大镜。
pub const SEARCH: &str = include_str!("assets/icons/search.svg");
/// 四格宫。
pub const GRID: &str = include_str!("assets/icons/grid.svg");
/// 五角星。
pub const STAR: &str = include_str!("assets/icons/star.svg");
/// 文档页。
pub const DOC: &str = include_str!("assets/icons/doc.svg");
/// 铃铛。
pub const BELL: &str = include_str!("assets/icons/bell.svg");
/// 三条滑条（调节）。
pub const SLIDERS: &str = include_str!("assets/icons/sliders.svg");

/// 名字 → 图标源。
///
/// [`IconSet::with_builtin`](super::IconSet::with_builtin) 读这张表。
///
/// **名字与文件名不一定相同**（`close` → `dismiss.svg`），因为名字是**调用方
/// 的词汇**而不是文件系统的词汇。把"关掉"叫 `close` 而不叫 `dismiss`，
/// 读代码的人才不用去查资源目录才能知道这个图标是什么。
pub const REGISTRY: &[(&str, IconSource)] = &[
    ("camera", IconSource::Svg(CAMERA)),
    ("calendar", IconSource::Svg(CALENDAR)),
    ("check", IconSource::Svg(CHECK)),
    ("chevron-down", IconSource::Svg(CHEVRON_DOWN)),
    ("chevron-up", IconSource::Svg(CHEVRON_UP)),
    ("clock", IconSource::Svg(CLOCK)),
    ("close", IconSource::Svg(DISMISS)),
    ("doc", IconSource::Svg(DOC)),
    ("eraser", IconSource::Svg(ERASER)),
    ("gear", IconSource::Svg(GEAR)),
    ("grid", IconSource::Svg(GRID)),
    ("bell", IconSource::Svg(BELL)),
    ("home", IconSource::Svg(HOME)),
    ("image", IconSource::Svg(IMAGE)),
    ("light", IconSource::Svg(LIGHT)),
    ("light-off", IconSource::Svg(LIGHT_OFF)),
    ("menu", IconSource::Svg(MENU)),
    ("minimize", IconSource::Svg(MINIMIZE)),
    ("move", IconSource::Svg(MOVE)),
    ("pen", IconSource::Svg(PEN)),
    ("search", IconSource::Svg(SEARCH)),
    ("settings", IconSource::Svg(SETTINGS)),
    ("sliders", IconSource::Svg(SLIDERS)),
    ("star", IconSource::Svg(STAR)),
    ("undo", IconSource::Svg(UNDO)),
];

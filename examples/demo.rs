//! 宿主示例：一个真开窗口的 ZephyraUI 应用。
//!
//! 演示**保留式声明**的核心收益：状态与纹理跨帧留存，只有真正变化的子树被重排 /
//! 重绘。窗口里看到的每一个像素都由 [`zephyra_core::paint::DisplayList`] 描述 ——
//! 本示例**不做任何绘制决策**，它只负责把元素树交给宿主。
//!
//! # 画面里有什么
//!
//! - **工具栏**：8 个可交互按钮。hover / press 态由核心里�� `InputRouter` 驱动，
//!   它处理了捕获、穿透与按下-抬起配对，示例不需要自己写事件分发。
//! - **内容区**：一个强调色面板，内含若干"文字条"色块。
//! - **状态栏**：焦点环样式的圆点 + 一条提示条。
//!
//! 按 `T` 切 Light/Dark。**树完全不动** —— 属性里存的资源键保持为键，只是换了一个
//! `&Theme` 存在的理由，也是本框架最想证明的一件事。
//!
//! # 文字画不出来（已知缺口）
//!
//! [`zephyra_core::paint::Command::Text`] 目前**零生产者**：核心的 `props.rs` 还没有任何
//! 文本属性键，`render_rec` 遇到 `NodeTag::Text` 不产出命令。所以示例用色块示意
//! 文字的位置与宽度 —— 那些色块的尺寸就是真实文字块的尺寸，一旦文本层接进
//! `render_rec`，换掉颜色即可，布局一行不用改。
//!
//! 这不是示例偷懒，是核心的真实边界。见 `zephyra/zephyra-base/README.md` 的「还缺什么」。
//!
//! # 运行
//!
//! ```bash
//! cargo run -p zephyra-foundation --bin demo
//! ZEPHYRA_TRACE=1 cargo run -p zephyra-foundation --bin demo   # 每帧统计
//! ```

use zephyra_core::core::{Spacing, UvRect};
use zephyra_core::layout::{AlignItems, Direction};
use zephyra_core::property::{PropertyKey, PropertyValue};
use zephyra_core::props;
use zephyra_core::theme::roles;
use zephyra_core::context::Context;
use zephyra_core::element::Element;
use zephyra_core::id::{ElementKey, NodeTag};

/// 分配计数探针（诊断用，不是产品代码 —— 见该文件头的警告）。
mod alloc_probe;

/// 宿主后端。
///
/// 由**特性开关**决定编哪个后端进来，而不是运行时切换：同一个二进制里只有一个后端被
/// 编译，两个都编进来会让「画面一空时是哪个后端的问题」变得无法回答。
///
/// - `d3d12`（默认）：DX12 后端。外壳 + 呈现链路已逐档验证（见
///   `zephyra-winit-d3d12/tests/swapchain_draw.rs`）。
/// - `d3d11`：D3D11 后端。本机视口设不进去，窗口是空的 —— 见
///   `zephyra-winit/tests/d3d11_draw.rs`。
/// - 无：只跑 `--dump-cpu`（不开窗口），适合无显示器的环境。
#[cfg(feature = "d3d12")]
use zephyra_winit_d3d12::{HostWindow, InputState, View, Z_INDEX};
#[cfg(all(feature = "d3d11", not(feature = "d3d12")))]
use zephyra_winit::{HostWindow, InputState, View, Z_INDEX};

/// 工具栏按钮的语义名。同时是元素 key —— 所以必须是 `&'static str`。
const BUTTONS: [&str; 8] = [
    "menu", "pen", "eraser", "undo", "image", "camera", "gear", "settings",
];

/// 内容区里的文本行数。
const LINES: usize = 4;

/// 每一行的内容。真实的 UTF-8 文本 —— 顺带验证非 ASCII 路径（`TextEngine`
/// 按 `char` 而非字节处理，中文与标点不该算错字数）。
const LINE_TEXTS: [&str; LINES] = [
    "ZephyraUI 核心",
    "DisplayList 是唯一契约",
    "换主题不需要遍历控件树",
    "Steady state = zero rebuild",
];

/// 演示用的自定义属性键。
///
/// 刻意**不**改核心的 `props.rs` —— 宿主可以定义自己的属性键而不动核心，这正是
/// 「宿主迁移不必搬核心」在日常开发里的体现。
static NOTE: PropertyKey = PropertyKey::new("Demo.Note", PropertyValue::static_str(""));

/// 视图：把状态变成一棵声明树。
#[derive(Debug, Default)]
struct Demo {
    /// 当前 hover 的按钮 key。
    hovered: Option<ElementKey>,
    /// 按下时被捕获的按钮 key。
    pressed: Option<ElementKey>,
}

impl View for Demo {
    fn on_input(&mut self, state: &InputState) {
        // 直接存 key 而不存「hover 了第几个按钮」——元素身份就是 key，
        // 跨帧稳定，diff 才知道该重绘哪个节点。
        self.hovered = state.hovered.clone();
        self.pressed = state.pressed.clone();
    }

    fn view(&self, _ctx: &Context<'_>) -> Element {
        Element::new(NodeTag::Container)
            .with_key("root")
            // 根必须有背景：真实管线里"没有背景"意味着**什么都不画**，全透明的树
            // 会产出空的 DisplayList。
            .with(
                &props::BACKGROUND,
                PropertyValue::Resource(roles::SURFACE.into()),
            )
            .with(
                &props::DIRECTION,
                PropertyValue::direction(Direction::Vertical),
            )
            .with_children([
            toolbar(self),
            icons(),
            content(),
            src_crops(),
            effects(),
            status_bar(self),
        ])
            .with(&NOTE, PropertyValue::static_str("demo"))
            .with(&Z_INDEX, PropertyValue::int(0))
    }
}

/// 工具栏：一排可交互按钮。
fn toolbar(demo: &Demo) -> Element {
    Element::new(NodeTag::Container)
        .with_key("toolbar")
        .with(
            &props::BACKGROUND,
            PropertyValue::Resource(roles::SURFACE_ALT.into()),
        )
        .with(&props::PADDING, PropertyValue::float(Spacing::Small.dip()))
        .with(&props::SPACING, PropertyValue::float(Spacing::Medium.dip()))
        .with(
            &props::DIRECTION,
            PropertyValue::direction(Direction::Horizontal),
        )
        .with(
            &props::CROSS_ALIGN,
            PropertyValue::cross_align(AlignItems::Center),
        )
        .with_children(BUTTONS.map(|name| button(name, demo)))
}

/// 一个工具栏按钮。
///
/// 颜色全部取**语义角色**，没有一处硬编码 —— 于是换主题不碰控件代码。
/// `props::control_fill_role` 是本框架「可重样式」的最小实现：控件不写死状态色，
/// 只问「当前状态下我该用哪个角色」。
fn button(name: &'static str, demo: &Demo) -> Element {
    let key = ElementKey::from(name);
    let hovered = demo.hovered.as_ref() == Some(&key);
    let pressed = demo.pressed.as_ref() == Some(&key);

    Element::new(NodeTag::Control)
        .with_key(name)
        .with(&props::INTERACTIVE, PropertyValue::bool(true))
        // NOTE: 状态机制的全部形状。源码只说「我现在是什么状态」，
        // 样式表只说「这个状态长什么样」，两边不见面。
        //
        // 没有分类名 = 没有状态，于是只被 `Control { }` 命中；
        // 有 "pressed" = 被 `Control.pressed { }` 命中，而那条的特异性
        // 更高，自动盖过默认值 —— 不引进任何新概念。
        //
        // ⚠️ 早先这里写死着 `BORDER_COLOR` / `BORDER_WIDTH` 两个值
        // （`if pressed { 2.0 } else { 1.0 }`）。那是源码里最后一处样式泄漏：
        // 换主题改不了它，改样式表也改不了它。
        .styles(pressed.then_some("pressed"))
        // NOTE: width / height / corner-radius / background / direction /
        // main-align / cross-align / spacing all moved to controls.zstyle.

        .with_children([
            // 图标位：真矢量图标，走 core 的「图标名 → 展平 → 三角化 → Path」。
            //
            // 之前这里是「用图标名字当标签画出真实文字」—— 那是在 `Command::Path`
            // **零生产者**时的权宜之计：核心只能产出 Quad/Image/Text 三种图元，
            // 图标只能退化成文字。现在 `props::ICON_NAME` 接通了几何层，
            // 这一位换回真正的图形。
            //
            // 注意 hover / press 态**同时**改颜色与文字颜色 —— 图标颜色走
            // `ICON_COLOR`，它不继承（见该属性的文档），所以必须显式给。
            Element::new(NodeTag::Container)
                .with_key("glyph")
                .with(&props::ICON_NAME, PropertyValue::static_str(icon_name(name)))
                .with(&props::ICON_COLOR, PropertyValue::Resource(icon_role(hovered, pressed).into()))
                .with(&props::WIDTH, PropertyValue::float(20.0))
                .with(&props::HEIGHT, PropertyValue::float(20.0)),
            Element::new(NodeTag::Text)
                .with_key("label")
                .with(&props::TEXT, PropertyValue::static_str(name))
                .with(
                    &props::TEXT_SIZE,
                    PropertyValue::float(if name.len() > 6 { 9.0 } else { 11.0 }),
                )
                .with(
                    &props::TEXT_COLOR,
                    PropertyValue::Resource(
                        if hovered || pressed {
                            roles::CONTENT
                        } else {
                            roles::CONTENT_SECONDARY
                        }
                        .into(),
                    ),
                ),
        ])
}

/// 按钮的图标名（内置图标集里的键）。
///
/// 工具栏那一排是「功能名」，而内置图标集里是「图标名」—— 两者语义不同，
/// 所以这里显式映射而不是直接用 `name`。映射不到时返回**原名**：
/// 图标集查不到会回落到空图标（不产出命令），症状是"少一个图标"而不是崩掉。
fn icon_name(button: &'static str) -> &'static str {
    match button {
        "menu" => "menu",
        "pen" => "pen",
        "eraser" => "eraser",
        "undo" => "undo",
        "image" => "image",
        "camera" => "camera",
        "gear" => "gear",
        "settings" => "settings",
        other => other,
    }
}

/// 图标颜色随交互状态变化。
fn icon_role(hovered: bool, pressed: bool) -> &'static str {
    if hovered || pressed {
        roles::CONTENT
    } else {
        roles::CONTENT_SECONDARY
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn demo_with(pressed: bool, name: &'static str) -> Demo {
        Demo {
            hovered: None,
            pressed: pressed.then(|| ElementKey::from(name)),
        }
    }

    /// **源码里不许有样式** —— 这条就是那把锁。
    ///
    /// 迁移前 `button()` 里写着：
    /// ```ignore
    /// .with(&props::BORDER_COLOR, Resource(if pressed { FOCUS_RING } else { STROKE }.into()))
    /// .with(&props::BORDER_WIDTH, float(if pressed { 2.0 } else { 1.0 }))
    /// ```
    /// 那是**最后一个**让「改样式表改不动按钮」的洞：换主题改不了它，
    /// 改 `controls.zstyle` 也改不了它。
    ///
    /// 现在这两个属性只能来自样式表。这条测试不看样式表（那是
    /// `controls_zstyle.rs` 的职责），只看源码这边**有没有偷偷写回去**。
    #[test]
    fn button_source_writes_no_appearance_at_all() {
        for pressed in [false, true] {
            let el = button("menu", &demo_with(pressed, "menu"));
            for key in [&props::BORDER_COLOR, &props::BORDER_WIDTH] {
                assert_eq!(
                    el.props().lookup(key),
                    None,
                    "{}(pressed={pressed}) must not write {} -- appearance comes from the stylesheet",
                    key.name(),
                    key.name()
                );
            }
        }
    }

    /// 状态靠**分类名**表达，而不是靠给属性塞值。
    ///
    /// 这是整个状态机制的形状：源码只说「我现在是什么状态」，
    /// 「这个状态长什么样」由样式表的 `Control.pressed { }` 说。
    #[test]
    fn the_state_reaches_the_element_as_a_class_name() {
        let pressed = button("menu", &demo_with(true, "menu"));
        assert_eq!(
            pressed.style_names(),
            &["pressed"],
            "pressing must add the pressed class"
        );

        let idle = button("menu", &demo_with(false, "menu"));
        assert!(
            idle.style_names().is_empty(),
            "no state must mean no class -- otherwise every button looks pressed"
        );
    }

    /// 按下的**是当前这个按钮**，不是所有按钮。
    ///
    /// `Demo::pressed` 存的是 key，而 `button()` 拿自己的 key 去比。
    /// 这条钉住「按钮认得住自己」：写成常量 true 的话，会出现
    /// 「按一个、全亮」——那种错从截图上很难定位。
    #[test]
    fn only_the_pressed_button_gets_the_class() {
        let demo = demo_with(true, "gear");
        assert_eq!(button("gear", &demo).style_names(), &["pressed"]);
        assert!(
            button("menu", &demo).style_names().is_empty(),
            "another button must stay idle"
        );
    }
}

/// 图像内容区：一组真实纹理的图标。
///
/// 纹理在启动阶段（`Bootstrap` 的 `resumed`）里注册一次，之后每帧只引用句柄。核心全程只看到
/// [`zephyra_core::paint::TextureRef`] 这个字符串 —— 它不认识 GPU 纹理。
fn icons() -> Element {
    Element::new(NodeTag::Container)
        .with_key("icons")
        .with(
            &props::DIRECTION,
            PropertyValue::direction(Direction::Horizontal),
        )
        .with(&props::SPACING, PropertyValue::float(Spacing::Large.dip()))
        .with(
            &props::CROSS_ALIGN,
            PropertyValue::cross_align(AlignItems::Center),
        )
        .with_children(
            ICON_NAMES
                .iter()
                .map(|n| {
                    Element::new(NodeTag::Image)
                        .with_key(*n)
                        // 只填句柄 —— 核心不知道它对应哪张 GPU 纹理。
                        .with(&props::IMAGE_TEXTURE, PropertyValue::static_str(n))
                        .with(&props::WIDTH, PropertyValue::float(56.0))
                        .with(&props::HEIGHT, PropertyValue::float(56.0))
                        .with(&props::CORNER_RADIUS, PropertyValue::float(8.0))
                })
                .collect::<Vec<_>>(),
        )
}

/// 演示用的纹理名。注册时用同一个名字，于是核心侧的属性值与宿主的注册名对上。
const ICON_NAMES: [&str; 4] = ["demo:checker", "demo:gradient", "demo:ring", "demo:stripes"];

/// 纹理内再画一个"取子区域"的样例：把同一个棋盘图集按不同 uv 裁剪并排显示。
///
/// 这一条专门验证 `src` 裁剪真的取到了纹理的对应区域 —— 棋盘的 8px 方块边界清晰，
/// 裁剪偏移 1/8 就肉眼可辨，而整张/翻转这两种错误则完全看不出来。
fn src_crops() -> Element {
    Element::new(NodeTag::Container)
        .with_key("src-crops")
        .with(
            &props::DIRECTION,
            PropertyValue::direction(Direction::Horizontal),
        )
        .with(&props::SPACING, PropertyValue::float(Spacing::Small.dip()))
        .with(
            &props::CROSS_ALIGN,
            PropertyValue::cross_align(AlignItems::Center),
        )
        .with_children(
            SRC_CROPS
                .iter()
                .enumerate()
                .map(|(i, uv)| {
                    Element::new(NodeTag::Image)
                        // key 必须是 `&'static str`：`ElementKey::Name` 持
                        // `Arc<str>`，而运行期 `format!` 出来的 String 不满足。
                        // 下标在这里就够了 —— 顺序即身份。
                        .with_key(SRC_CROP_KEYS[i])
                        // 只填 uv：核心不认识它对应哪张纹理，也不知道纹理多大。
                        .with(
                            &props::IMAGE_SRC,
                            PropertyValue::uv_rect(UvRect::new(uv.0, uv.1, uv.2, uv.3)),
                        )
                        .with(
                            &props::IMAGE_TEXTURE,
                            PropertyValue::static_str("demo:checker"),
                        )
                        .with(&props::WIDTH, PropertyValue::float(40.0))
                        .with(&props::HEIGHT, PropertyValue::float(40.0))
                })
                .collect::<Vec<_>>(),
        )
}

/// 演示用的裁剪区域（`x, y, w, h`，归一化）。
///
/// 依次是：整张、右半、左下四分之一、以及一个**越界**区域。
/// 最后一项是故意的 —— 核心应把它退化成整张，画面看起来与第一项相同，而不是
/// 采样到边界外的像素。
const SRC_CROPS: [(f32, f32, f32, f32); 4] = [
    (0.0, 0.0, 1.0, 1.0),
    (0.5, 0.0, 0.5, 1.0),
    (0.5, 0.5, 0.5, 0.5),
    (0.9, 0.0, 0.5, 1.0),
];

/// 与 [`SRC_CROPS`] 一一对应的 key。长度不同则是编译错误，不会静默错位。
const SRC_CROP_KEYS: [&str; 4] = ["crop:full", "crop:right", "crop:br", "crop:overflow"];

/// 生成一张演示纹理：按名字给出不同的图案，用来确认采样与 tint 正确。
///
/// 这些是**程序生成**的图案，不是图片文件 —— 免得仓库里塞二进制资源，也顺带证明
/// 纹理上传这条路对任意像素都成立。
fn icon_pixels(name: &str) -> (Vec<u8>, u32, u32) {
    const S: u32 = 32;
    let mut px = vec![0u8; (S * S * 4) as usize];
    for y in 0..S {
        for x in 0..S {
            let i = ((y * S + x) * 4) as usize;
            let (r, g, b, a) = match name {
                // 棋盘：明显的 8px 方块，用来核对 uv 方向与缩放。
                "demo:checker" => {
                    if ((x / 8) + (y / 8)) % 2 == 0 {
                        (230, 230, 235, 255)
                    } else {
                        (40, 44, 52, 255)
                    }
                }
                // 斜向渐变：用来核对 uv 是"左上原点"而不是翻转过的。
                "demo:gradient" => {
                    let t = (x + y) as f32 / (2.0 * S as f32);
                    (
                        (40.0 + 180.0 * t) as u8,
                        (90.0 + 120.0 * t) as u8,
                        (200.0 - 120.0 * t) as u8,
                        255,
                    )
                }
                // 圆环：透明中心，用来核对 alpha 与 discard。
                "demo:ring" => {
                    let dx = x as f32 - S as f32 / 2.0;
                    let dy = y as f32 - S as f32 / 2.0;
                    let d = (dx * dx + dy * dy).sqrt();
                    if d < 11.0 {
                        (255, 170, 60, 255)
                    } else {
                        (0, 0, 0, 0)
                    }
                }
                // 横条：alpha 递减，用来核对半透明混合。
                _ => {
                    let a = (x * 255 / S) as u8;
                    let band = if (y / 8) % 2 == 0 { 255 } else { 120 };
                    (band, band / 2, 255, a)
                }
            };
            px[i] = r;
            px[i + 1] = g;
            px[i + 2] = b;
            px[i + 3] = a;
        }
    }
    (px, S, S)
}

/// 内容区：一个强调色面板，内含"文字条"。
///
/// 面板本身取 `accent_fill` 资源键 —— 按 `T` 切主题时它会跟着变，而这一行代码
/// 不需要改动。这就是「切主题不遍历控件树」。
fn content() -> Element {
    Element::new(NodeTag::Container)
        .with_key("content")
        .with(
            &props::BACKGROUND,
            PropertyValue::Resource(roles::SURFACE.into()),
        )
        .with(
            &props::PADDING,
            PropertyValue::float(Spacing::Large.dip()),
        )
        .with(
            &props::DIRECTION,
            PropertyValue::direction(Direction::Vertical),
        )
        .with(&props::SPACING, PropertyValue::float(Spacing::Small.dip()))
        .with_children([
            Element::new(NodeTag::Container)
                .with_key("hero")
                .with(
                    &props::BACKGROUND,
                    PropertyValue::Resource(roles::ACCENT_FILL.into()),
                )
                .with(&props::CORNER_RADIUS, PropertyValue::float(12.0))
                .with(
                    &props::PADDING,
                    PropertyValue::float(Spacing::Large.dip()),
                )
                .with(
                    &props::DIRECTION,
                    PropertyValue::direction(Direction::Vertical),
                )
                .with(&props::SPACING, PropertyValue::float(Spacing::Small.dip()))
                .with_children(
                    (0..LINES)
                        .map(|i| {
                            // 真实文本：核心按 Text 属性排版，宿主提供引擎。
                            //
                            // 之前这里是"用色块示意文字位置"—— 那是 `Command::Text`
                            // 零生产者时的权宜之计。现在文本层通了，字号、行高、宽度
                            // 全是真算出来的。
                            Element::new(NodeTag::Text)
                                .with_key(line_key(i))
                                .with(&props::TEXT, PropertyValue::static_str(line_text(i)))
                                .with(&props::TEXT_SIZE, PropertyValue::float(16.0))
                                .with(
                                    &props::TEXT_COLOR,
                                    PropertyValue::Resource(roles::ON_ACCENT_FILL.into()),
                                )
                        })
                        .collect::<Vec<_>>(),
                ),
        ])
}

/// 一个效果样本：深色底板 + 半透明色块，色块带自己的滤镜 / 混合模式。
///
/// # 三层而不是两层
///
/// ```text
/// 外层（竖排：底板 + 说明文字）
/// └ 底板  深色、不透明、带滤镜/混合模式吗？不带 —— 它只是「目标」
///   └ 样本  半透明色块，滤镜与混合模式挂在这里
/// ```
///
/// 底板与样本**必须**是两个节点。把同一个节点先涂成深色再涂成半透明色，
/// 后一次会**覆盖**前一次（属性是覆盖语义，不是叠加），于是「底板」消失、
/// 样本直接混在面板底色上 —— 混合模式照样能跑，但两个模式之间的对比度掉一大截，
/// 肉眼就不容易分辨了。
///
/// # 为什么样本要**半透明**
///
/// 混合模式是「源」与「目标」的函数，而不透明色块叠在底板上时，除 `Src` 之外的
/// 每个模式算出来的结果都等于色块本身 —— 29 个模式看起来一模一样。
/// 50% alpha 才让 `Multiply`（底板透过来）与 `Screen`（两者都往白推）分得开。
fn effect_swatch(
    key: &'static str,
    caption: &'static str,
    fill: zephyra_core::core::Color,
    filter: zephyra_core::filter::FilterChain,
    blend: zephyra_core::filter::BlendMode,
) -> Element {
    use zephyra_core::core::Color;

    Element::new(NodeTag::Container)
        .with_key(key)
        .with(&props::DIRECTION, PropertyValue::direction(Direction::Vertical))
        .with(&props::SPACING, PropertyValue::float(6.0))
        .with_children([
            // 底板：混合模式的「目标」。深色底 + 浅色描边，让两种模式的差别最大。
            Element::new(NodeTag::Container)
                .with_key("backdrop")
                .with(
                    &props::BACKGROUND,
                    PropertyValue::color(Color::rgb(0.10, 0.20, 0.42)),
                )
                .with(&props::CORNER_RADIUS, PropertyValue::float(8.0))
                .with(&props::WIDTH, PropertyValue::float(96.0))
                .with(&props::HEIGHT, PropertyValue::float(64.0))
                .with(&props::PADDING, PropertyValue::float(12.0))
                .with_children([Element::new(NodeTag::Container)
                    .with_key("swatch")
                    .with(
                        &props::BACKGROUND,
                        PropertyValue::color(Color::rgba(fill.r, fill.g, fill.b, 0.55)),
                    )
                    .with(&props::CORNER_RADIUS, PropertyValue::float(6.0))
                    // ⚠ 样本必须给**显式**尺寸。只挂背景与圆角的节点没有内在尺寸，
                    // 而容器布局不会把「父节点扣掉内边距后的剩余空间」默认分给孩子
                    // —— 于是它塌成 0×0，滤镜与混合模式都挂在一条什么都不画的
                    // 命令上。症状是「离屏规划里有这个 RT、但画面上什么都没有」，
                    // 很像滤镜没实现。
                    .with(&props::WIDTH, PropertyValue::float(72.0))
                    .with(&props::HEIGHT, PropertyValue::float(40.0))
                    .with(&props::FILTER, PropertyValue::filter(filter))
                    .with(&props::BLEND_MODE, PropertyValue::blend_mode(blend))]),
            Element::new(NodeTag::Text)
                .with_key("caption")
                .with(&props::TEXT, PropertyValue::static_str(caption))
                .with(&props::TEXT_SIZE, PropertyValue::float(12.0)),
        ])
}

/// E4 的效果展台：模糊 / 投影 / 多级链，外加两个非默认混合模式。
///
/// # 这一块在演示什么
///
/// E4 的三件事各自都能被肉眼分辨，但**必须有对照**才不会误判：
///
/// - 模糊 —— 看边缘有没有过渡带。没做模糊时硬边只有 0 与满值两种灰度；
/// - 投影 —— 看**偏移方向**上有没有第二个更暗的形状，且原图仍压在它上面；
/// - 多级链 —— 单趟模糊与三趟模糊的过渡带**宽度不同**，只看「糊了」区分不了。
///
/// 最后两个样本不带滤镜、只带混合模式，是为了让「混合模式生效」这件事与
/// 「离屏 pass 生效」互不干扰 —— 否则一个模式下错，会被另一个的偏移掩盖。
fn effects() -> Element {
    use zephyra_core::core::{Color, Point};
    use zephyra_core::filter::{BlendMode, Filter, FilterChain};

    let accent = Color::rgb(0.35, 0.78, 1.0);
    let warm = Color::rgb(1.0, 0.62, 0.28);

    // 三趟同参数的高斯 ≈ 一趟 σ=2.83，所以过渡带明显更宽 —— 这就是判据。
    let triple = FilterChain::new([
        Filter::Blur { sigma_x: 2.0, sigma_y: 2.0 },
        Filter::Blur { sigma_x: 2.0, sigma_y: 2.0 },
        Filter::Blur { sigma_x: 2.0, sigma_y: 2.0 },
    ]);

    Element::new(NodeTag::Container)
        .with_key("effects")
        .with(
            &props::BACKGROUND,
            PropertyValue::Resource(roles::SURFACE_ALT.into()),
        )
        .with(&props::PADDING, PropertyValue::float(Spacing::Medium.dip()))
        .with(
            &props::DIRECTION,
            PropertyValue::direction(Direction::Vertical),
        )
        .with(&props::SPACING, PropertyValue::float(Spacing::Small.dip()))
        .with_children([
            Element::new(NodeTag::Container)
                .with_key("effects-row")
                .with(
                    &props::DIRECTION,
                    PropertyValue::direction(Direction::Horizontal),
                )
                .with(&props::SPACING, PropertyValue::float(Spacing::Medium.dip()))
                .with_children([
                    effect_swatch(
                        "fx-none",
                        "对照",
                        accent,
                        FilterChain::none(),
                        BlendMode::SrcOver,
                    ),
                    effect_swatch(
                        "fx-blur",
                        "模糊 σ3",
                        accent,
                        FilterChain::single(Filter::Blur { sigma_x: 3.0, sigma_y: 3.0 }),
                        BlendMode::SrcOver,
                    ),
                    effect_swatch(
                        "fx-shadow",
                        "投影 σ3",
                        accent,
                        FilterChain::single(Filter::DropShadow {
                            color: Color::rgba(0.0, 0.0, 0.0, 0.85),
                            offset: Point::new(6.0, 6.0),
                            sigma: 3.0,
                        }),
                        BlendMode::SrcOver,
                    ),
                    effect_swatch("fx-triple", "三级模糊", accent, triple, BlendMode::SrcOver),
                    effect_swatch(
                        "fx-multiply",
                        "Multiply",
                        warm,
                        FilterChain::none(),
                        BlendMode::Multiply,
                    ),
                    effect_swatch(
                        "fx-screen",
                        "Screen",
                        warm,
                        FilterChain::none(),
                        BlendMode::Screen,
                    ),
                ]),
        ])
}

/// 状态栏。右侧那块的宽度反映 hover 目标 —— 没有文字层的替代指示。
fn status_bar(demo: &Demo) -> Element {
    Element::new(NodeTag::Container)
        .with_key("status")
        .with(
            &props::BACKGROUND,
            PropertyValue::Resource(roles::SURFACE_ALT.into()),
        )
        .with(&props::PADDING, PropertyValue::float(Spacing::Small.dip()))
        .with(
            &props::DIRECTION,
            PropertyValue::direction(Direction::Horizontal),
        )
        .with(&props::SPACING, PropertyValue::float(Spacing::Medium.dip()))
        .with(&props::CROSS_ALIGN, PropertyValue::cross_align(AlignItems::Center))
        .with_children([
            // 焦点环样式的圆点：切主题时它跟着 `FOCUS_RING` 变。
            Element::new(NodeTag::Container)
                .with_key("focus")
                .with(
                    &props::BACKGROUND,
                    PropertyValue::Resource(roles::FOCUS_RING.into()),
                )
                .with(&props::CORNER_RADIUS, PropertyValue::float(8.0))
                .with(&props::WIDTH, PropertyValue::float(10.0))
                .with(&props::HEIGHT, PropertyValue::float(10.0)),
            // hover 目标：真实文字。
            //
            // 之前这里是"用色块宽度编码目标序号"—— 那是 `Command::Text` 零生产者
            // 时的替代方案。文本层通了之后就不需要这类绕路了。
            Element::new(NodeTag::Text)
                .with_key("hovered")
                .with(&props::TEXT, PropertyValue::static_str(hover_caption(demo)))
                .with(&props::TEXT_SIZE, PropertyValue::float(12.0))
                .with(
                    &props::TEXT_COLOR,
                    PropertyValue::Resource(
                        if demo.hovered.is_some() {
                            roles::CONTENT
                        } else {
                            roles::CONTENT_DISABLED
                        }
                        .into(),
                    ),
                ),
        ])
}

/// hover 目标的名字；无目标时给一句提示。
fn hover_caption(demo: &Demo) -> &'static str {
    match demo.hovered.as_ref().and_then(|k| k.as_name()) {
        Some(name) => BUTTONS
            .iter()
            .find(|b| **b == name)
            .copied()
            .unwrap_or("?"),
        None => "hover a button",
    }
}

/// 第 `i` 行的 key。必须 `&'static str` —— `ElementKey` 只接受编译期常量或 `u64`。
fn line_key(i: usize) -> &'static str {
    match i {
        0 => "line0",
        1 => "line1",
        2 => "line2",
        _ => "line3",
    }
}

/// 第 `i` 行的文本。
fn line_text(i: usize) -> &'static str {
    LINE_TEXTS.get(i).copied().unwrap_or("")
}

/// 随 demo 分发的字体（MiSans Regular，含 CJK 与拉丁）。
///
/// # 为什么自带字体而不是用系统的
///
/// `zephyra-fx` 的候选表里首选 `segoeui.ttf` —— 它**没有汉字**，于是中文一律走
/// U+FFFD 变豆腐块；含 CJK 的 `msyh.ttc` 排第二，但命中首选就返回，永远轮不到。
/// 见 `zephyra_fx::text::FONT_CANDIDATES`。
///
/// 不用「把雅黑挪到表首」那种改法，是因为那会让拉丁字母的观感随机器而变 —— 同一段
/// 代码在两台机器上是两个样子。自带字体则处处一致。
///
/// 代价是 7.7 MB 的二进制进仓。这是权衡的结果，不是疏忽。
const DEMO_FONT: &[u8] = include_bytes!("fonts/MiSans-Regular.ttf");

/// 让 [`zephyra_fx`] 用 demo 自带的字体。
///
/// # 为什么用 `include_bytes!` 而不是运行时读 `examples/fonts/`
///
/// 运行时读路径的话，字体文件必须在**运行目录**下 —— 而 `cargo run` 的运行目录是
/// 仓库根、`cargo test` 又是 crate 目录、两处都不同，那条路会脆得没法用。
///
/// `include_bytes!` 在编译期就把字节嵌进二进制，路径从哪跑都一样。代价是改字体文件
/// 必须重新编译（不像 `include_str!` 那样有增量失效的麻烦）。
fn use_demo_font() {
    // 写进临时文件再让 `from_font_file` 读 —— 那条路需要的是**路径**，而我们手里只有
    // 字节。放在 `std::env::temp_dir()` 下并按内容哈希命名，于是同一个字体在多次
    // 运行之间复用同一个文件，且换了字体内容就是新文件。
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in DEMO_FONT {
        h ^= *b as u64;
        h = h.wrapping_mul(0x1000_0000_01b3);
    }
    let path = std::env::temp_dir().join(format!("zephyra-demo-font-{h:016x}.ttf"));
    // 内容哈希在名字里，同名即同内容 —— 存在就说明已经写好了，不用重写 7.7 MB。
    if !path.exists() {
        if let Err(e) = std::fs::write(&path, DEMO_FONT) {
            eprintln!("[demo] 落地字体失败（{e}），退回系统字体 —— 中文会是豆腐块");
            return;
        }
    }
    std::env::set_var(
        zephyra_fx::FONT_OVERRIDE_ENV,
        path.to_string_lossy().as_ref(),
    );
}

fn main() {
    use_demo_font();

    // `--dump-cpu`：**不开窗口**，把第一帧在 CPU 上画成 BMP 后退出。
    //
    // # 为什么需要这个开关
    //
    // 本机上 GPU 侧是坏的（D3D11 的 `RSSetViewports` 设不进去，见
    // `zephyra-winit/tests/d3d11_draw.rs`），所以窗口里永远是空的。而"空"无法区分
    // 「顶点流错了」与「GPU 没画」—— 这正是过去几十轮反复卡住的地方。
    //
    // `--dump-cpu` 走**同一个** `DisplayList`、**同一个** `quad::expand`，只是把光栅化
    // 从 GPU 挪到 CPU（`zephyra_fx::cpu_raster`）。图能打开看，就证明顶点流与
    // 文本链路都对，嫌疑当场缩小到 GPU 环节。
    if std::env::args().any(|a| a == "--dump-cpu") {
        dump_cpu();
        return;
    }

    // `--bench`：不开窗口，跑 N 帧并报各阶段耗时分布。
    //
    // # 为什么需要它
    //
    // `--dump-cpu` 只跑**一帧** —— 而一帧里绝大部分工作是"首次"：字体解析、图集
    // 打包、树首次构建。它量的是冷启动，不是稳态。真正的帧时间要看第 2 帧之后。
    //
    // 稳态才是瓶颈所在：静止的 UI 每帧仍要走完 `view → diff → layout → paint →
    // expand`，其中任何一段的浪费都会在 60fps 下变成可感知的掉帧。
    //
    // 输出**中位数**而不是平均值：单帧偶尔会被 OS 抢占或页错误污染，平均值会把
    // 那几毫秒摊薄到看不出来。中位数只反映典型帧。
    if std::env::args().any(|a| a == "--bench") {
        bench_cpu();
        return;
    }

    // `HostWindow::new_async` 是 async（D3D11 的 adapter/device 请求如此），而 `main`
    // 是同步的 —— （已移除） 把它挤过去。它是个约 200 行的 block_on，引 tokio 只为跑
    // 两个 future 不划算。
    //
    // winit 0.30 有个硬要求：窗口只能在事件循环的 `resumed` 回调里创建，而
    // `ActiveEventLoop` 只能通过 `EventLoop::run_app` 拿到 —— 于是「建窗」与
    // 「跑事件循环」必须由同一个 `EventLoop` 串起来，不能像普通库那样先建后跑。
    //
    // 所以这里不用 `（已移除）`（它需要 sync 上下文，而建窗必须在 loop 回调里）。
    // 正确形态是一个自举的应用类型：在 `resumed` 里用 （已移除） 跑完 async 初始化，
    // 然后把建好的宿主存进自己。
    let event_loop =
        winit::event_loop::EventLoop::new().expect("EventLoop::new");
    let mut app = Bootstrap {
        inner: None,
        view: Some(Demo::default()),
    };
    if let Err(e) = event_loop.run_app(&mut app) {
        eprintln!("事件循环退出：{e}");
    }
}

/// 把演示画面在 CPU 上画成 BMP。
///
/// # 为什么绕开窗口
///
/// 窗口需要 GPU，而本机 GPU 侧是坏的。但**排版、图集打包、顶点展开、文本布局**全都不
/// 需要 GPU —— 它们本来就是 CPU 工作。绕开窗口不是"简化版"，而是**同一份数据**走
/// 另一条路，所以它的结论对 GPU 侧同样有效。
///
/// # 为什么还要走 `View` → `HostWindow` 而不是直接调 core
///
/// 因为要验证的是**整条链**：demo 的元素树 → 布局 → `DisplayList` → `quad::expand`。
/// 绕过 `HostWindow` 就等于跳过了布局，而布局错位是文字"看起来不对"最常见的原因。
fn dump_cpu() {
    let (w, h) = (1000u32, 640u32);
    let mut host = zephyra_fx::cpu_only::CpuHost::new(Demo::default(), (w, h));

    let frame = host.frame();
    let r = frame.render;
    // ⚠ 不打印 `r.vertex_bytes`：`expand` 不填它（由渲染器填），所以它恒为 0。
    // 打印出来会让人以为"顶点流是空的"，而同一次调用里 quads = 135。
    // 顶点流是否非空看下面 `batch.solid.len()`。
    let batch = host.batch();
    let glyph_vertices = batch
        .solid
        .chunks_exact(4)
        .filter(|c| (c[0].kind - zephyra_fx::quad::kind::GLYPH).abs() < 0.01)
        .count();
    println!(
        "[dump] DisplayList → 顶点：四边形 {} 个、顶点 {} 个（其中字形 {} 个）、跳过 {} 个、缺失纹理 {} 个",
        r.quads,
        batch.solid.len(),
        glyph_vertices,
        r.skipped,
        r.missing_textures
    );
    // 效果与混合模式的执行在GPU 后端，几何层看不到它们。所以这两个计数是
    // "效果被静默忽略"的**唯一**线索。
    //
    // ⚠ 本 demo 不带任何效果，所以它们是 0 —— **这不是"应该恒为 0"**：
    // 一旦有人给元素加了 blur / 非默认 blend，它们立刻变成非零，而那正是
    // "效果没发生"的信号（命令照常画，所以症状不是元素消失）。
    if r.unimplemented_filters > 0 || r.unimplemented_blends > 0 {
        println!(
            "[dump] ⚠ 效果未执行：滤镜 {} 条、混合模式 {} 条 —— GPU 侧离屏/混合状态尚未实现",
            r.unimplemented_filters, r.unimplemented_blends
        );
    }
    let plans = zephyra_fx::offscreen::plan_offscreens(host.list());
    if !plans.is_empty() {
        println!(
            "[dump] 离屏规划：{} 张 RT",
            plans.len()
        );
        for p in &plans {
            println!(
                "[dump]   RT {:>4.0}×{:<4.0}  内容 {:?}  滤镜 {} 项",
                p.bounds.width,
                p.bounds.height,
                p.content,
                p.filter.filters().len()
            );
        }
    }
    // 图标落地诊断：几何链跑通与否只能看 `DisplayList` 里有没有 `Command::Path`。
    //
    // ⚠ 这一条曾经**完全没有**观测手段：demo 里图标位画的是文字，而文字走
    // `Command::Text` —— 于是"图标接通了"与"图标被当成文字画了"在所有既有指标上
    // 都长得一模一样（四边形数、顶点数、内容像素全对）。只有一个不查渲染管线、
    // 只查契约的判据能区分它们。
    let mut path_cmds = 0usize;
    let mut path_tris = 0usize;
    for item in host.list().iter() {
        if let zephyra_core::paint::Command::Path { vertices, .. } = &item.command {
            path_cmds += 1;
            path_tris += vertices.len() / 3;
        }
    }
    println!(
        "[dump] 矢量图标：Command::Path {path_cmds} 条、三角形 {path_tris} 个\
         {}",
        if path_tris == 0 {
            "  ← ⚠ 一个都没有：图标名没查到，或展平/三角化返回空"
        } else {
            ""
        }
    );
    let a = host.atlas_stats();
    println!(
        "[dump] 字形图集：已缓存 {} 个字形，用量 {:.1}%（{}/{} 像素）",
        a.glyphs,
        a.usage() * 100.0,
        a.used_px,
        a.capacity_px
    );

    let clear = zephyra_core::core::Color::rgba(0.0, 0.0, 0.0, 1.0);
    let canvas = host.render_cpu(clear);

    // 「有多少像素与**底色**不同」—— 这是"有没有东西被画上去"最直接的判据。
    //
    // ⚠ 基准色必须是**画完之后的实际底色**，不是清屏色。
    //
    // 踩了两次：
    //
    // 1. 基准用「非黑」、清屏纯黑 → 640000/640000 全"有内容"。因为 demo 的根容器
    //    铺了整屏 `SURFACE` 深灰（sRGB 约 53,56,59），每个角落都非黑。
    // 2. 基准改成「与清屏色精确不同」、清屏深灰 → **同样** 640000/640000。因为底色
    //    不是清屏色，而是根容器画上去的 `SURFACE`。
    //
    // 两次都是**恒真式**：它们给出"CPU 侧完全正常"的结论，而图其实只有底色和几行
    // 文字。共同点是拿"一个并不等于底色的东西"当基准。
    //
    // 正确做法：**先渲染一遍，取出现最多的那个颜色作为底色** —— 它必然是底色，
    // 因为底色覆盖面积最大。这个基准无法被"画错了"骗过去：真出错时底色还在，
    // 而内容色会变。
    let mut counts: std::collections::HashMap<[u8; 3], usize> = Default::default();
    for p in canvas.pixels.chunks_exact(4) {
        *counts.entry([p[0], p[1], p[2]]).or_default() += 1;
    }
    let Some((&bg, _)) = counts.iter().max_by_key(|(_, n)| **n) else {
        eprintln!("[dump] 画布是空的");
        return;
    };
    let lit = canvas
        .pixels
        .chunks_exact(4)
        .filter(|p| [p[0], p[1], p[2]] != bg)
        .count();
    let total = (w * h) as usize;
    let distinct = counts.len();
    println!(
        "[dump] CPU 光栅：底色 {:?}，内容像素 {lit}/{total}（{:.1}%），不同颜色 {distinct} 种",
        bg,
        lit as f64 / total as f64 * 100.0
    );
    if lit == 0 {
        println!("[dump] ⚠ 整幅图只有底色 —— 顶点流或图集有问题，**别去查 GPU**");
    }

    let path = std::path::Path::new("demo-cpu.bmp");
    match host.dump_cpu_bmp(&path.display().to_string(), clear) {
        Ok(()) => {
            // 自动打开 —— 用户要的是"看到效果"，让他自己去翻目录找文件是白费一轮。
            //
            // 用 `rundll32 url.dll,FileProtocolHandler` 而不是 `start`：后者在
            // `cmd /c start` 里需要额外转义，含空格或 `&` 的路径会静默失败。
            let abs = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
            println!("[dump] 已写出 {}", abs.display());
            let _ = std::process::Command::new("rundll32")
                .arg("url.dll,FileProtocolHandler")
                .arg(&abs)
                .spawn();
        }
        Err(e) => eprintln!("[dump] 写 {} 失败：{e}", path.display()),
    }
}

/// 跑 N 帧，报各阶段耗时的中位数与最大值。
///
/// # 为什么是「同一棵树重复跑」
///
/// 这一帧测的是**稳态**：保留树已建好、图集已打包、字形位图已在缓存里。于是每一段
/// 都在测它自己的稳态成本，而不是"第一次做"的一次性开销。
///
/// 这也顺带暴露一个关键事实：如果某一帧的耗时与前��帧几乎相同，那说明**稳态下每帧
/// 都在重复做同一件事** —— 那是本函数存在的全部理由（`paint.rs` 的排版与字形栅格化
/// 正是这样，每帧重做）。
fn bench_cpu() {
    /// 参与统计的帧数。
    const FRAMES: usize = 120;
    /// 丢弃的开头帧数（字体解析、图集首次打包、树首次构建都在这几帧里）。
    const WARMUP: usize = 12;

    let (w, h) = (1000u32, 640u32);
    let mut host = zephyra_fx::cpu_only::CpuHost::new(Demo::default(), (w, h));

    // 探针只数分配，不改耗时结论 —— 它的原子操作会把绝对耗时抬高。
    let probe = std::env::args().any(|a| a == "--alloc-probe");
    let mut before = alloc_probe::Counts::now();
    // 直方图只在采样窗口里累加：`reset` 放在 `WARMUP` 之后的头一帧之前，
    // 这样它量的是**稳态**帧的分配构成，不掺首次构建的那些。
    let mut histogram_ready = false;

    let mut samples: Vec<zephyra_fx::cpu_only::StageTimes> = Vec::with_capacity(FRAMES);
    let mut alloc_samples: Vec<(u64, u64)> = Vec::new();
    let mut last = None;
    for i in 0..FRAMES {
        if probe {
            if i == WARMUP {
                alloc_probe::reset_histogram();
                histogram_ready = true;
            }
            before = alloc_probe::Counts::now();
        }
        let (frame, t) = host.frame_timed();
        if probe && i >= WARMUP {
            let d = alloc_probe::Counts::now().since(&before);
            alloc_samples.push((d.allocs, d.bytes));
        }
        last = Some(frame);
        if i >= WARMUP {
            samples.push(t);
        }
    }

    if probe && !alloc_samples.is_empty() {
        let mut counts: Vec<u64> = alloc_samples.iter().map(|(a, _)| *a).collect();
        let mut sizes: Vec<u64> = alloc_samples.iter().map(|(_, b)| *b).collect();
        counts.sort_unstable();
        sizes.sort_unstable();
        println!(
            "[bench] 每帧分配：中位 {} 次 / {} 字节，最大 {} 次 / {} 字节（全局最大单次 {}）",
            counts[counts.len() / 2],
            sizes[sizes.len() / 2],
            counts[counts.len() - 1],
            sizes[sizes.len() - 1],
            alloc_probe::Counts::now().max_one,
        );
        if histogram_ready {
            // 只印构成里权重最大的几桶：全印出来会把"18 次 × 32 字节"这种噪声
            // 与"3 次 × 30KB"这种真问题混在一起，看不出哪个该修。
            let mut h = alloc_probe::histogram();
            h.sort_by_key(|&(_, _, bytes)| std::cmp::Reverse(bytes));
            let per_frame = (FRAMES - WARMUP) as f64;
            println!("[bench] 分配大小分布（按每帧字节降序，次数已除以采样帧数）：");
            for (hi, n, bytes) in h.iter().take(10) {
                println!(
                    "[bench]   ≤ {hi:>6} 字节：每帧 {:>7.1} 次  {:>9.1} 字节",
                    *n as f64 / per_frame,
                    *bytes as f64 / per_frame,
                );
            }
        }
    }

    // 中位数而不是平均值：单帧会被 OS 抢占 / 页错误污染，而平均值会把那几毫秒
    // 摊薄到看不出来。中位数只反映**典型**帧 —— 那才是决定手感的东西。
    fn median(xs: &mut [u32]) -> u32 {
        xs.sort_unstable();
        xs[xs.len() / 2]
    }
    fn report(name: &str, mut xs: Vec<u32>, total: u64) {
        xs.sort_unstable();
        let med = median(&mut xs.clone());
        let max = *xs.last().unwrap_or(&0);
        let pct = if total == 0 {
            0.0
        } else {
            med as f64 / total as f64 * 100.0
        };
        println!(
            "[bench] {name:<8} 中位 {med:>6} µs  最大 {max:>6} µs  占比 {pct:>5.1}%"
        );
    }

    let frame = last.expect("FRAMES > 0");
    println!(
        "[bench] {} 帧（丢弃开头 {} 帧），{}×{}；四边形 {}、顶点 {}",
        FRAMES,
        WARMUP,
        w,
        h,
        frame.render.quads,
        host.batch().solid.len(),
    );
    // 各阶段的占比之和是判据：它应当约等于 100%。明显小于 100% 说明**有阶段没被
    // 计时**（计时点之间漏了代码），那时"占比"这列数字会误导 —— 所以直接印出来。
    let sum: u64 = samples
        .iter()
        .map(|s| s.view_us as u64 + s.diff_us as u64 + s.layout_us as u64 + s.paint_us as u64 + s.expand_us as u64)
        .sum();
    let sum_total: u64 = samples.iter().map(|s| s.total_us as u64).sum();
    let accounted = if sum_total == 0 {
        0.0
    } else {
        sum as f64 / sum_total as f64 * 100.0
    };
    report(
        "合计",
        samples.iter().map(|s| s.total_us).collect(),
        sum_total,
    );
    report(
        "view",
        samples.iter().map(|s| s.view_us).collect(),
        sum,
    );
    report(
        "diff",
        samples.iter().map(|s| s.diff_us).collect(),
        sum,
    );
    report(
        "layout",
        samples.iter().map(|s| s.layout_us).collect(),
        sum,
    );
    report(
        "paint",
        samples.iter().map(|s| s.paint_us).collect(),
        sum,
    );
    report(
        "expand",
        samples.iter().map(|s| s.expand_us).collect(),
        sum,
    );
    println!(
        "[bench] 各阶段之和 / 合计 = {accounted:.1}%（应当 ≈100%；明显小于 100 说明有代码没被计时）"
    );
}

/// Load the `.zstyle` that sits next to this source file, install it as the
/// frame's styler, and report problems in full.
///
/// Errors are printed and then **skipped** -- the app keeps running on
/// `props.rs` defaults. A broken look-and-feel file must not stop the program:
/// that would turn "one wrong property name" into "this app will not start".
fn load_stylesheet<V: View>(app: &mut HostWindow<V>) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("controls.zstyle");
    let css = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[demo] 读不到 {}：{e}（按默认值继续）", path.display());
            return;
        }
    };

    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("controls.zstyle");
    let result = zstylec::parse(name, &css);

    if !result.is_ok() {
        for e in &result.errors {
            eprintln!("{e}\n");
        }
        eprintln!(
            "[demo] 样式表 {} 个错误，仍按默认值继续（不是退出）",
            result.errors.len()
        );
        return;
    }

    let rules = result.sheet.rules().len();
    app.set_styler(Box::new(zephyra_style::SheetStyler(std::sync::Arc::new(
        result.sheet,
    ))));
    eprintln!("[demo] 已加载 {name}：{rules} 条规则");
}

/// 自举宿主：第一次 `resumed` 时建窗（async），之后把事件转交给 [`HostWindow`]。
///
/// `V` 必须能整份移出（`new_async` 拿走所有权），所以这里存 `Option<V>`。
struct Bootstrap<V: View> {
    inner: Option<HostWindow<V>>,
    view: Option<V>,
}

impl<V: View + 'static> winit::application::ApplicationHandler for Bootstrap<V> {
    fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        if self.inner.is_some() {
            return;
        }
        let view = self.view.take().expect("resumed 只应发生一次建窗");
        // `new_lenient` 内部已 block_on 掉 D3D11 的 future，所以这里是同步的 ——
        // 而这正是它存在的理由：winit 0.30 要求窗口只能在 `resumed` 里创建，而
        // `resumed` 是同步回调。
        let mut app = HostWindow::new_lenient(event_loop, view);

        // 注册演示纹理。核心只看到 `TextureRef`（一个字符串），宿主在这里把字符串
        // 接到真实的 GPU 纹理上 —— 这就是 core 与 base 的全部接缝。
        //
        // 失败**逐条报出来**而不是静默跳过：SRV 槽位是有限的（DX12 侧 8 个，1 号
        // 恒给图集），用尽时若只跳过，症状是「图标少了几个」—— 而那个现象与
        // 「元素树里少写了几个图标」无法区分。
        for name in ICON_NAMES {
            let (pixels, w, h) = icon_pixels(name);
            if let Err(e) = app.register_texture(name, &pixels, w, h) {
                eprintln!("[demo] 纹理 {name} 注册失败：{e}");
            }
        }
        eprintln!(
            "[demo] 已注册 {}/{} 张纹理（32×32 RGBA）",
            app.textures().len(),
            ICON_NAMES.len()
        );

        // Load the stylesheet -- the first proof of "no styles in source".
        // Failure only warns: without it the UI still works off `props.rs`
        // defaults. Letting a look-and-feel file stop the app is backwards.
        load_stylesheet(&mut app);

        // ⚠ 必须在这里补第一帧。
        //
        // `HostWindow` 自己**也**实现了 `ApplicationHandler`，但注册给 winit 的是
        // `Bootstrap`（它要先建窗、再注册纹理，才轮到宿主）。所以 `HostWindow` 的
        // `resumed` / `about_to_wait` **永远不会被 winit 调用** —— 它们只是被这里
        // 转发过去的方法。
        //
        // 漏掉这一帧的后果是：交换链建好了、DComp 提交了，但**从未 present 过**。
        // 屏幕上是一个还没被写过的 composition swapchain，透出窗口的白色背景 ——
        // 而它的唯一症状就是「窗口全白」，与「画了但颜色不对」看起来不一样，
        // 于是很容易被当成渲染问题去查。
        app.sync_input();
        app.frame();
        app.present();

        self.inner = Some(app);
    }

    /// 转发给宿主。
    ///
    /// winit 在「事件都处理完了、下一次 `WaitMessage` 之前」调它。那才是「按需重绘」
    /// 的时机 —— 事件回调里每来一个 `CursorMoved` 就重画一帧的话，一次鼠标划过会
    /// 排上百帧。
    fn about_to_wait(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        if let Some(app) = self.inner.as_mut() {
            app.about_to_wait(event_loop);
        }
    }

    fn window_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        id: winit::window::WindowId,
        event: winit::event::WindowEvent,
    ) {
        match self.inner.as_mut() {
            Some(app) => app.window_event(event_loop, id, event),
            // 建窗之前不该有事件；收到说明 winit 行为变了。
            None => {
                let _ = event;
            }
        }
    }
}

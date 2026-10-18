//! 液态玻璃：折射（`Filter::Refraction`）+ 既有滤镜。
//!
//! # 这一页要展示的是「透过玻璃看到后面是什么」
//!
//! 所以玻璃块**必须压在有内容的地方**。放一块纯色底板上的折射等于什么都不显示
//! —— 折射采样的是**背景**，背景没东西就看不出任何位移。
//!
//! 底下铺的那几行字不是装饰：它们是**折射的输入**。删掉它们，这一页
//! 就变成"三个灰色方块"，而那看起来像折射没实现。
//!
//! # 后端支持
//!
//! 折射在 **D3D11 / D3D12** 上实现；Vulkan 的滤镜管线尚未接通（那边
//! `fs_blur_h` / `fs_composite` 还没被调用），所以在 Linux 上这一页
//! 看起来与"没设滤镜"相同。`--dump-cpu` 那条路也不画滤镜。

use zephyra_core::element::Element;
use zephyra_core::id::NodeTag;
use zephyra_ui::controls;

use crate::app::pages;
use crate::app::Gallery;

/// 玻璃**后面**铺的内容 —— **折射的输入**，不是装饰。
const BACKDROP_TEXT: &[&str] = &[
    "The quick brown fox jumps over the lazy dog.",
    "Refraction samples what is already on screen.",
    "Remove these lines and the glass looks broken.",
];

/// 三块玻璃的强度档（`(key, 标题, 强度分类名)`）。
///
/// # 为什么只有分类名没有数字
///
/// 折射的**真实参数**（强度、色散）全在样式表的 `.glass-weak` /
/// `.glass-medium` / `.glass-strong` 里，写成 `filter: refraction(...)`。
/// 这里传数字不产生任何效果 —— 那正是「外观不进源码」这条纪律：控件只发
/// **类别**，不给**值**。
const PANES: &[(&str, &str, &str)] = &[
    ("glass:subtle", "Subtle", "weak"),
    ("glass:medium", "Medium", "medium"),
    ("glass:strong", "Strong", "strong"),
];

pub fn page(_g: &Gallery) -> Element {
    pages::page_root(
        "effects",
        "page-effects",
        vec![
            pages::section(
                "sec-glass",
                "Liquid glass",
                vec![
                    controls::caption("glass-note", "The backdrop behind each pane is what bends."),
                    stage(),
                ],
            ),
            pages::section(
                "sec-filters",
                "Filters",
                vec![pages::demo_many(
                    "Progress",
                    vec![
                        controls::progress_bar("fx-soft", 0.35, "running"),
                        controls::progress_bar("fx-hard", 0.8, "error"),
                    ],
                )],
            ),
        ],
    )
}

/// 折射的**舞台**：一块深色底板 + 压在它上面的三块玻璃。
///
/// # 为什么玻璃块要靠 `translate-y` 压上去，而不是当兄弟节点摆
///
/// 折射采样的是「**已经画在屏幕上**的东西」，所以玻璃块必须与它要折射的图案
/// **在几何上重叠**。而 core 没有浮层、没有负 margin、没有绝对定位（属性表里
/// 只有 `translate-x/y`，见 `zstylec/src/props_map.rs`）—— 于是重叠只有一条路：
/// 负 `translate-y`。
///
/// ⚠ 这是**有意为之的脆弱**：`.glass-panes` 的位移量必须与 `.glass-stage` 的
/// padding + 文字高度对得上，否则玻璃块会滑出底板 —— 症状是「折射不生效」，
/// 而真正的原因是**玻璃块下面没有内容可折射**。改文字行数时记得一起改样式表。
///
/// # 为什么底板要挑这种深蓝而不是深灰
///
/// 折射位移在**色差大**的地方才看得见。深灰底上的深色玻璃，位移 20 像素与
/// 位移 0 像素画出来几乎一样 —— 于是「折射没生效」与「折射生效但看不出来」
/// 没法区分，而后者会被当成前者去查着色器。深蓝 + 白字把这个问题挡在门外。
fn stage() -> Element {
    pages::page_root("glass-stage", "glass-stage", vec![backdrop(), panes()])
}

/// 玻璃**背后**的内容。
///
/// ⚠ 这些行不是装饰，它们是折射要采样的东西。删掉之后玻璃块与纯色底板背景
/// 之间没有任何差别，于是「折射没生效」看起来像 bug，而真正的原因是
/// **没有可供折射的内容**。
fn backdrop() -> Element {
    pages::page_root(
        "glass-backdrop",
        "glass-backdrop",
        BACKDROP_TEXT
            .iter()
            .enumerate()
            .map(|(i, l)| controls::text_label(backdrop_key(i), l, "glass-bg-text"))
            .collect(),
    )
}

/// 三块玻璃并排。
fn panes() -> Element {
    pages::page_root(
        "glass-panes",
        "glass-panes",
        PANES
            .iter()
            .map(|(key, title, strength)| {
                pages::page_root(
                    "cell",
                    "glass-cell",
                    vec![
                        controls::text_label(pane_title_key(key), title, "glass-title"),
                        Element::new(NodeTag::Container)
                            .with_key(*key)
                            .style("glass-pane")
                            .style(strength),
                    ],
                )
            })
            .collect(),
    )
}

/// 背景文字的 key（不能 `format!`，理由见 `pages::basics::spin_text`）。
const fn backdrop_key(i: usize) -> &'static str {
    match i {
        0 => "gb:0",
        1 => "gb:1",
        _ => "gb:2",
    }
}

/// 玻璃块标题的 key。
///
/// ⚠ 不是 `const fn`：`&str` 上的 `match` 在常量上下文里还不允许（E0658）。
/// 它每帧只调三次，运行期比较的开销可忽略 —— 而省下的是一次 `format!`，
/// 那才是每帧一次泄漏（见 `pages::basics::spin_text`）。
fn pane_title_key(key: &str) -> &'static str {
    match key {
        "glass:subtle" => "gt:subtle",
        "glass:medium" => "gt:medium",
        _ => "gt:strong",
    }
}

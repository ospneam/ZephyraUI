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
//! # 参数是**运行期可调**的，不是三档静态值
//!
//! 玻璃块上的 `filter` 由 `gallery::app::glass::Glass` 逐帧算出（见其
//! `filter` 方法），面板上的 `[−] [+]` 直接改它。所以这一页同时是**说明**与**实验台**：
//! 把厚度调到 0 会看到玻璃变成纯透镜板，把 IOR 调到 1.00 会看到位移完全消失
//! —— 后者是最直接的「公式被改坏了吗」诊断。
//!
//! # 后端支持
//!
//! 折射在 **D3D11 / D3D12** 上实现；Vulkan 的滤镜管线尚未接通（那边
//! `fs_blur_h` / `fs_composite` 还没被调用），所以在 Linux 上这一页
//! 看起来与"没设滤镜"相同。`--dump-cpu` 那条路也不画滤镜。

use zephyra_core::element::Element;
use zephyra_core::id::NodeTag;
use zephyra_core::property::PropertyValue;
use zephyra_core::props;
use zephyra_ui::controls;

use crate::app::glass::Param;
use crate::app::pages;
use crate::app::Gallery;

/// 玻璃**后面**铺的内容 —— **折射的输入**，不是装饰。
const BACKDROP_TEXT: &[&str] = &[
    "The quick brown fox jumps over the lazy dog.",
    "Refraction samples what is already on screen.",
    "Remove these lines and the glass looks broken.",
];

/// 三块玻璃的 key 与标题。
///
/// ⚠ **不带强度分类名**。旧版这里是 `.weak` / `.medium` / `.strong`，参数由
/// 样式表的 `filter: refraction(...)` 给 —— 那等于把参数钉死成三档。
/// 现在参数由 [`Gallery::glass`] 逐帧算，三块玻璃**共用同一组**，于是拖一次
/// 就能同时看���变化（分成三档反而看不出「改的到底是哪一项」）。
const PANES: &[(&str, &str)] = &[
    ("glass:a", "Thin"),
    ("glass:b", "Normal"),
    ("glass:c", "Thick"),
];

pub fn page(g: &Gallery) -> Element {
    pages::page_root(
        "effects",
        "page-effects",
        vec![
            pages::section(
                "sec-params",
                "Refraction parameters",
                vec![params(g), stage(g)],
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

/// 参数面板：四组 `[−] 名称 值 [+]` 横排。
///
/// # 为什么一行放四组而不是四行
///
/// 内容卡（`shell::CONTENT_H` 756）减去页头与标题只剩 400 出头，而舞台本身
/// 就占 255。参数面板只能占一行 —— 而一行四组在 1139 宽下绰绰有余（每组约
/// 240px），竖排则要 160px，直接把舞台挤出客户区。
///
/// # 为什么用 `[−] [+] 而不是滑条
///
/// core 还没把滑条做成**输入设备**（`Gallery::click` 里它仍属「展示型」，
/// 见 `docs/dev-log/README.md`）。用滑条做这个面板会得到一个**看得见摸不动**
/// 的控件 —— 而那正是本页最不该出现的东西：整个页面就是讲「参数可调」的。
fn params(g: &Gallery) -> Element {
    let (t_text, b_text, d_text, i_text) = g.glass.texts();
    let values = [t_text, b_text, d_text, i_text];

    pages::page_root(
        "glass-params",
        "glass-params",
        Param::ALL
            .iter()
            .enumerate()
            .map(|(i, p)| stepper(g, *p, values[i]))
            .collect(),
    )
}

/// 一组步进器。
fn stepper(g: &Gallery, param: Param, value: &'static str) -> Element {
    pages::page_root(
        "cell",
        "glass-param",
        vec![
            controls::text_label(param.label(), param.label(), "glass-param-label"),
            controls::round_button(
                dec_key(param),
                "-",
                None,
                false,
                g.hovers(dec_key(param)),
                g.holds(dec_key(param)),
            ),
            controls::text_label("v", value, "glass-param-value"),
            controls::round_button(
                inc_key(param),
                "+",
                None,
                false,
                g.hovers(inc_key(param)),
                g.holds(inc_key(param)),
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
fn stage(g: &Gallery) -> Element {
    pages::page_root("glass-stage", "glass-stage", vec![backdrop(), panes(g)])
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
///
/// # `filter` 写在这里、而不是样式表 —— 见模块文档
///
/// 值来自 [`Gallery::glass`]，每帧重算。它是**逐帧**的（`view()` 每帧重建整棵
/// 树），但查表是零成本的：四次数组下标，没有分配、没有浮点运算。
fn panes(g: &Gallery) -> Element {
    // ⚠ 每块玻璃**各自重建**链，而不是建一次后复用：`FilterChain` 不是 `Copy`，
    // 而 `map` 的闭包是 `FnMut` —— 捕获一个要按值取用的变量会编译不过。
    // 重建的代价是四次数组下标，零分配。
    pages::page_root(
        "glass-panes",
        "glass-panes",
        PANES
            .iter()
            .map(|(key, title)| {
                pages::page_root(
                    "cell",
                    "glass-cell",
                    vec![
                        controls::text_label(pane_title_key(key), title, "glass-title"),
                        Element::new(NodeTag::Container)
                            .with_key(*key)
                            .style("glass-pane")
                            .with(
                                &props::FILTER,
                                PropertyValue::filter(zephyra_core::filter::FilterChain::single(
                                    g.glass.filter(),
                                )),
                            ),
                    ],
                )
            })
            .collect(),
    )
}

/// 减号按钮的 key（**不能 `format!`**，见模块文档的每帧泄漏那条）。
const fn dec_key(p: Param) -> &'static str {
    match p {
        Param::Thickness => "glass-thickness:dec",
        Param::Bezel => "glass-bezel:dec",
        Param::Dispersion => "glass-dispersion:dec",
        Param::Ior => "glass-ior:dec",
    }
}

/// 加号按钮的 key。
const fn inc_key(p: Param) -> &'static str {
    match p {
        Param::Thickness => "glass-thickness:inc",
        Param::Bezel => "glass-bezel:inc",
        Param::Dispersion => "glass-dispersion:inc",
        Param::Ior => "glass-ior:inc",
    }
}

/// 背景文字的 key。
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
/// 它每帧只调三次，运行期比较的开销可忽略 —— 而省下的是一次 `format!`。
fn pane_title_key(key: &str) -> &'static str {
    match key {
        "glass:a" => "gt:a",
        "glass:b" => "gt:b",
        _ => "gt:c",
    }
}

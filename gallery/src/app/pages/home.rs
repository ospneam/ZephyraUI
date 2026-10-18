//! 首页：**活样例** + 分类入口。
//!
//! # 这一页要回答的问题
//!
//! 「这个库现在有什么、长什么样」。所以它不放**说明文字**（那是文档的事），
//! 而是把最常用的控件真的摆出来能点 —— 想看细节的走分类页。
//!
//! # 为什么不做成「卡片网格」
//!
//! 旧首页是「横幅 + 三张大卡 + 最近添加的十张卡」，其中十张卡点进去是控件页。
//! 现在控件页已经合并进分类页，那十张卡就没有去处了 —— 而它们每次都要在
//! 导航树之外再维护一份顺序与描述（`catalog.rs` 的 340 行大半是这个）。
//!
//! 换成「能用的控件 + 指向分类页的小条目」之后：页面自己就是内容，
//! 不再需要一张「指向别处的卡片的清单」。

use zephyra_core::element::Element;

use super::Gallery;
use crate::app::pages;
use zephyra_ui::controls;

/// 首页根容器。
pub fn page(g: &Gallery) -> Element {
    pages::page_root(
        "home",
        "page-home",
        vec![
            hero(),
            live_samples(g),
            pages::section("sec-cats", "Browse by category", category_links(g)),
        ],
    )
}

/// 顶部横幅：一句话 + 一个主行动。
///
/// 高度刻意压到 180 —— 旧版 320，占掉一屏的三分之一，而那三分之一里
/// 只有三行字。
fn hero() -> Element {
    pages::page_root(
        "hero",
        "hero",
        vec![
            controls::text_label("hero-title", "ZephyraUI", "hero-title"),
            controls::text_label(
                "hero-sub",
                "A retained-mode widget layer for Rust — class names in, stylesheet out.",
                "hero-sub",
            ),
        ],
    )
}

/// 活样例区：常用的、能立刻摸到的控件。
fn live_samples(g: &Gallery) -> Element {
    pages::section(
        "sec-live",
        "Try them",
        vec![
            pages::demo_many(
                "Buttons",
                vec![
                    controls::button(
                        "btn-primary",
                        "Primary",
                        None,
                        "accent",
                        g.hovers("btn-primary"),
                        g.holds("btn-primary"),
                    ),
                    controls::button(
                        "btn-secondary",
                        "Secondary",
                        None,
                        "standard",
                        g.hovers("btn-secondary"),
                        g.holds("btn-secondary"),
                    ),
                    controls::button(
                        "btn-subtle",
                        "Subtle",
                        None,
                        "subtle",
                        g.hovers("btn-subtle"),
                        g.holds("btn-subtle"),
                    ),
                ],
            ),
            pages::demo_many(
                "Selection",
                vec![
                    controls::toggle_switch(
                        "live-switch",
                        g.switch_on,
                        g.hovers("live-switch"),
                        g.holds("live-switch"),
                    ),
                    controls::check_box(
                        "live-check",
                        "Agree",
                        g.checked[0],
                        g.hovers("live-check"),
                        g.holds("live-check"),
                    ),
                ],
            ),
            pages::demo(
                "Slider",
                controls::slider(
                    "live-slider",
                    g.volume,
                    None,
                    g.hovers("live-slider"),
                    g.holds("live-slider"),
                ),
            ),
        ],
    )
}

/// 分类入口：每类一个小条目，点了进那一页。
///
/// # 为什么是 3×3 而不是一列 9 个
///
/// core **没有**折行语义（`docs/developer/README.md` 的帧流水线一节记着这条），
/// 所以「分类入口」这个容器放不下自动换行。一列 9 个 34 高的按钮 = 9×40 = 360
/// 像素 —— 而内容卡（`shell::CONTENT_H` 756）扣掉页头与首页横幅之后只剩 400 出头，
/// 于是最后三行**掉到窗口外面**：既点不到（命中测试找不到），画面上还看不见。
///
/// 这不是「多写点内容」，是一条硬约束：**页面必须一屏装得下**。门禁
/// `hit_test_agrees_with_the_painted_rects` 对全部路由跑一遍，任何掉出客户区的
/// 可交互节点都会被点名。
///
/// 3×3 是排出来的而不是算出来的：9 个入口 ÷ 3 列 = 3 行，而 3 行 × 40 = 120
/// 正好把首页的剩余高度吃掉一小块。加到第 10 个分类时这一页就得重新排 ——
/// 那时该做的是拆成两页，不是把格子缩到更小。
fn category_links(g: &Gallery) -> Vec<Element> {
    const PER_ROW: usize = 3;

    let cats: Vec<(&'static str, &'static str, &'static str)> = super::PAGES
        .iter()
        .filter(|p| p.id != "home")
        .map(|p| (p.id, p.title, p.icon))
        .collect();

    cats.chunks(PER_ROW)
        .enumerate()
        .map(|(r, chunk)| {
            pages::page_root(
                category_row_key(r),
                "category-row",
                chunk
                    .iter()
                    .map(|(id, title, icon)| {
                        controls::button(
                            id,
                            title,
                            Some(icon),
                            "category",
                            g.hovers(id),
                            g.holds(id),
                        )
                    })
                    .collect(),
            )
        })
        .collect()
}

/// 分类入口的行 key。不能用 `format!`（每帧一次泄漏，见 `pages::basics::spin_text`）。
const fn category_row_key(r: usize) -> &'static str {
    match r {
        0 => "catrow:0",
        1 => "catrow:1",
        2 => "catrow:2",
        _ => "catrow:3",
    }
}

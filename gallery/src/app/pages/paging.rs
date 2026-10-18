//! 分页：SwipeView（卡片轮播 + 圆点翻页）。
//!
//! # 为什么单独一族，而不是留在 Collections 里
//!
//! 参考层每族必须**一屏装得下**——core 既无滚动也不裁剪溢出，装不下的部分
//! 就是被吃掉、点不到（`hit_test_agrees_with_the_painted_rects` 用真实字体量）。
//! Collections 曾经有 List / Table / Swipe / Avatar 四节，实测在真实字体下
//! 页高 838px、Avatar 整节掉出客户区，连翻页圆点都点不到（y=858）。
//!
//! SwipeView 的「取数据的形状」其实更接近导航（翻到第几页），
//! 把它单列一族比硬塞回 Collections 更清楚，也让两边都留出余量。

use zephyra_core::element::Element;

use super::Gallery;
use crate::app::pages;
use zephyra_ui::controls;

/// SwipeView 的页面（`(标题, 说明)`；只显示当前那一张）。
const SWIPE: &[(&str, &str)] = &[
    ("First card", "Click a dot below to change pages."),
    (
        "Second card",
        "The core has no drag gesture; paging is by key.",
    ),
    ("Third card", "State lives in the app, not in the control."),
];

pub fn page(g: &Gallery) -> Element {
    pages::page_root(
        "paging",
        "page-paging",
        vec![
            pages::section(
                "sec-swipe",
                "Swipe",
                vec![
                    // 当前页走 `Gallery::swipe_page`：底部圆点（`sw:N`）**可点**
                    // 跳页（core 没有拖拽手势，翻页只能由点击表达），
                    // `click` 的 `sw:` 分支回写。
                    controls::swipe_view("swipe", SWIPE, g.swipe_page),
                    // ⚠ 这里不再另摆一个 `page_indicator`：`swipe_view` 自带
                    // 圆点（同一组 `sw:N` key），再摆一个就是同页两份翻页控件。
                ],
            ),
            pages::section(
                "sec-pages",
                "Page indicator",
                // 独立的进度点（`pips`）：**展示**件 —— 它的语义是「一共多少页」，
                // 不接收点击（可点的是上面 SwipeView 的圆点）。白名单注明。
                vec![pages::demo(
                    "Position",
                    controls::page_indicator("pips", SWIPE.len(), g.swipe_page),
                )],
            ),
        ],
    )
}

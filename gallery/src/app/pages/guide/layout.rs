//! 布局与响应式：core 的三个真实边界。
//!
//! # 这一章教什么
//!
//! core 的三个边界，每条都配一个**看得见**的例子。开发者先知道边界在哪，
//! 才不会把「内容太多」当成 bug 去查样式表。
//!
//! # 为什么这三条边界值得单独一页
//!
//! 它们都不会报错：
//! - **无滚动** → 内容超出的唯一表现是下沿被吃掉
//! - **无折行** → 「横排几列」是排出来的，不是算出来的
//! - **无 flex-grow** → 垂直方向的铺满要靠定高占位
//!
//! 三条合起来的症状是「画面上东西不见了」，而排查方向有十几种。把它们写在
//! 一页上，比让开发者踩一遍划算。

use zephyra_core::element::Element;
use zephyra_ui::controls;

use crate::app::pages;
use crate::app::Gallery;

pub fn page(g: &Gallery) -> Element {
    pages::page_root(
        "layout",
        "page-layout",
        vec![
            controls::text_label(
                "layout-blurb",
                "Three core boundaries: no scroll, no wrap, no flex-grow. Each is invisible until you hit it.",
                "text-body",
            ),
            // ---- 边界一：无滚动 ----
            //
            // 内容区高 756。这一列卡片的总高故意贴近上限 —— 开发者拖窗口
            // 变小时会看到最后一张卡片的下沿「被切掉」，而不是出现滚动条。
            // 那是 core 的当前边界，不是 bug。
            pages::section(
                "layout-noscroll",
                "No scroll — overflow is clipped",
                vec![
                    pages::demo("Card 1", controls::card("lc-1", "card", vec![
                        controls::text_label("lc-1-t", "First", "text-body-strong"),
                        controls::text_label("lc-1-d", "Fits inside the card", "text-caption"),
                    ])),
                    pages::demo("Card 2", controls::card("lc-2", "card", vec![
                        controls::text_label("lc-2-t", "Second", "text-body-strong"),
                        controls::text_label("lc-2-d", "Also fits", "text-caption"),
                    ])),
                    pages::demo("Card 3", controls::card("lc-3", "card", vec![
                        controls::text_label("lc-3-t", "Third", "text-body-strong"),
                        controls::text_label("lc-3-d", "Watch me when the window shrinks", "text-caption"),
                    ])),
                ],
            ),
            // ---- 边界二：断点三档 ----
            //
            // 宽度 640 / 1000 / 1400 是官方三档的边界。窄档下导航栏收成图标列
            // （`nav-pane.compact`），内容区拿到全宽。开发者拖窗口宽度就能看到切换。
            pages::section(
                "layout-breakpoints",
                "Breakpoints: small / medium / large",
                vec![pages::demo(
                    "Current breakpoint",
                    controls::text_label(
                        "layout-bp",
                        match g.width_class() {
                            zephyra_core::responsive::WidthClass::Small => "Small — nav collapsed to icons",
                            zephyra_core::responsive::WidthClass::Medium => "Medium — nav shows labels",
                            zephyra_core::responsive::WidthClass::Large => "Large — full layout",
                        },
                        "text-body-strong",
                    ),
                )],
            ),
            // ---- 边界三：触控目标 ----
            //
            // 官方触控目标是 44 epx。小于这个尺寸的按钮在触控设备上点不准 ——
            // 而鼠标路径不受影响。`Gallery::touch_target` 给出官方换算，
            // 门禁 `touch_target_converts_to_official_ephemeral_px` 钉住它。
            // 两个按钮都发 `act:` key —— 触发反馈与尺寸演示互不干扰。
            pages::section(
                "layout-touch",
                "Touch targets need 44 epx",
                vec![
                    pages::demo("44 epx (correct)", controls::button(
                        "act:touch-ok",
                        "Big enough",
                        None,
                        "standard",
                        g.hovers("act:touch-ok"),
                        g.holds("act:touch-ok"),
                    )),
                    pages::demo("28 epx (too small)", controls::round_button(
                        "act:touch-bad",
                        "Tiny",
                        None,
                        false,
                        g.hovers("act:touch-bad"),
                        g.holds("act:touch-bad"),
                    )),
                ],
            ),
        ],
    )
}

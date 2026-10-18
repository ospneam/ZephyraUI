//! 快速开始：一个按钮 + 一个计数，跑通整条链。
//!
//! # 这一章教什么
//!
//! 开发者第一次打开会看到什么：**点一下，界面变了**。而背后整条链已经全跑通：
//!
//! ```text
//! 点击 ─▶ Gallery::click      应用状态（这里是 count）
//!          └▶ Gallery::view   每帧重建整棵树
//!              └▶ shell::window   外壳：标题栏 + 导航 + 内容卡
//!                  └▶ 本章内容   按钮 + 计数
//!                      └▶ controls::*   控件层
//!                          └▶ core          diff → layout → paint
//! ```
//!
//! 开发者要抄的就是这个形状：**状态住在 Gallery，页面只读 `&Gallery`，
//! 交互发一个带 key 的控件、由 `click()` 统一处理**。页面本身永远不
//! 直接改状态 —— 于是「哪些 key 会改状态」只有一处真相。

use zephyra_core::element::Element;
use zephyra_ui::controls;

use crate::app::pages;
use crate::app::Gallery;

pub fn page(g: &Gallery) -> Element {
    pages::page_root(
        "guide",
        "page-guide",
        vec![
            // 页首一两句说明：开发者要的是「这页在讲什么」，不是一篇文档。
            controls::text_label(
                "guide-blurb",
                "Click the button. The count goes up. That is the whole pipeline.",
                "text-body",
            ),
            pages::demo(
                "Counter",
                controls::button(
                    "guide-inc",
                    "Click me",
                    None,
                    "accent",
                    g.hovers("guide-inc"),
                    g.holds("guide-inc"),
                ),
            ),
            Element::new(zephyra_core::id::NodeTag::Text)
                .with_key("guide-count")
                .with(
                    &zephyra_core::props::TEXT,
                    zephyra_core::property::PropertyValue::static_str(match g.guide_count {
                        0 => "0",
                        1 => "1",
                        2 => "2",
                        3 => "3",
                        4 => "4",
                        5 => "5",
                        _ => "5+",
                    }),
                )
                .style("text-title"),
        ],
    )
}

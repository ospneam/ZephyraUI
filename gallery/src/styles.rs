//! 合并控件层样式表与本应用自己的表。
//!
//! # 加载顺序即优先级
//!
//! [`zstylec`] 对**同特异性**的规则是「后写的赢」，所以这张表的顺序不是
//! 随意的 —— 它是优先级本身。分三段：
//!
//! ```text
//!   1. 控件层（15 个）      —— Button / Card / Dialog 的长相
//!   2. 应用外壳 shell.zstyle —— 标题栏 / 导航 / 内容卡
//!   3. 页面 page-*.zstyle   —— 每页自己的容器间距
//! ```
//!
//! # 为什么控件层在前而不是在后
//!
//! 因为**本应用覆盖控件外观是合法需求**：某个页面想让 Card 更矮、或者把
//! Button 改成胶囊，都应该只写一行 `Container.card { height: 64px; }` 而
//! 不用去改控件层。把控件层放在前面就是这个覆盖能力的来源。
//!
//! 反过来（控件层在后）的话，应用层任何同名规则都会被静默吃掉 —— 症状是
//! 「我写了样式但没反应」，而那是最难查的一类样式 bug。
//!
//! # 一条硬约束
//!
//! 控件层与本应用**不得出现同名选择器**，除非本应用确实想覆盖。
//! 这个约束由门禁 `tests/gates.rs::stylesheet_selectors_do_not_collide`
//! 盯着 —— 不是因为会编译失败（zstylec 不报同名），而是因为同名时
//! 「谁赢」完全靠文件顺序，读代码看不出来。

/// 本应用自己的样式表，按加载顺序。
///
/// 只有两份：`shell`（外壳）与 `pages`（全部页面编排）。
///
/// # 为什么从 15 份合成 2 份
///
/// 旧结构每个页面一个文件（14 个 `page-*.zstyle`），于是「新增一个页面」要
/// 做三件事：建 `.zstyle`、改 `styles.rs` 的常量表、改宿主的加载列表 ——
/// 而后两处漏了不会报错，只是**那个页面完全没有样式**（退回默认外观）。
///
/// 现在页面编排的分类名高度重复（10 个 `.page-*` 都只是「竖排 + 间距」），
/// 合成一份之后它们相邻，改间距一眼看全；真正按页不同的东西（`.hero`、
/// `.glass-*`）在自己的规则块里，也不会被别页的同名规则盖掉。
const APP_SHEETS: &[(&str, &str)] = &[
    ("shell", include_str!("../styles/shell.zstyle")),
    ("pages", include_str!("../styles/pages.zstyle")),
];

/// 全部样式表（控件层 + 本应用），按**加载顺序**返回 `(名字, 源码)`。
///
/// 这就是宿主要喂给 `zstylec` 的全部输入 —— 一份列表，没有别的入口。
pub fn all() -> Vec<(&'static str, &'static str)> {
    let mut v = Vec::with_capacity(zephyra_ui::STYLESHEETS.len() + APP_SHEETS.len());
    v.extend_from_slice(zephyra_ui::STYLESHEETS);
    v.extend_from_slice(APP_SHEETS);
    v
}

/// 只取本应用自己的表（给「想单独验一遍应用层样式」的门禁用）。
pub fn app_only() -> &'static [(&'static str, &'static str)] {
    APP_SHEETS
}

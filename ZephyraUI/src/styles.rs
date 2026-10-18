//! 控件层的样式表 —— **内嵌**在二进制里。
//!
//! # 为什么是 `include_str!` 而不是运行时读文件
//!
//! 旧版本的做法是：gallery 在启动时 `read_dir` 自己的 crate 目录，把找到的
//! `.zstyle` 全读进来。那条路对**应用**没问题，对**库**有三个坏处：
//!
//! 1. **发布二进制时必须把 `.zstyle` 一并拷过去**，漏了就静默失效 ——
//!    样式表不在，控件全退回默认外观，而**没有任何报错**。
//! 2. 库的使用者得知道「样式文件放在哪」，这是本该由库自己保证的事。
//! 3. 没法在无文件系统的环境（wasm、单文件分发）里工作。
//!
//! `include_str!` 把这三件事一并解决：文件在**编译期**读进来，所以运行时
//! 不需要它们存在，也不存在「漏拷」这个失败模式。
//!
//! # 加载顺序即优先级
//!
//! [`STYLESHEETS`] 按下面这个顺序返回，而 `zstylec` 对**同特异性**的规则
//! 是「后写的赢」。所以这个顺序是有意义的：
//!
//! - `base` 最先 —— 排版档位与分隔线，被所有控件继承；
//! - 其余按控件族；
//! - 每个控件族内部，源码侧的书写次序就是优先级（变体在前、状态在后）。
//!
//! ⚠ 别把 `base` 往后挪：`Container.glyph` / `Text.text-body` 这类基础规则
//!     被几十个控件继承，挪到后面会让所有控件的图标与字号一起变。

/// 全部控件样式表，按**加载顺序**返回 `(名字, 源码)`。
///
/// 顺序即优先级，见模块文档。
pub const STYLESHEETS: &[(&str, &str)] = &[
    ("base", include_str!("../controls-base.zstyle")),
    ("badge", include_str!("../controls-badge.zstyle")),
    ("button", include_str!("../controls-button.zstyle")),
    ("card", include_str!("../controls-card.zstyle")),
    (
        "collections",
        include_str!("../controls-collections.zstyle"),
    ),
    ("dialogs", include_str!("../controls-dialogs.zstyle")),
    ("forms", include_str!("../controls-forms.zstyle")),
    ("menu", include_str!("../controls-menu.zstyle")),
    ("nav", include_str!("../controls-nav.zstyle")),
    ("navigation", include_str!("../controls-navigation.zstyle")),
    ("segmented", include_str!("../controls-segmented.zstyle")),
    ("status", include_str!("../controls-status.zstyle")),
    ("text", include_str!("../controls-text.zstyle")),
    ("text-input", include_str!("../controls-text-input.zstyle")),
    ("toggle", include_str!("../controls-toggle.zstyle")),
];

/// 按名字取一份样式表（给「只想加载其中一份」的场景用）。
///
/// `None` = 没有这个名字 —— 注意 `base` 之类的**基础**表漏了会让所有控件
/// 退回默认外观，而那不报错，所以调用方不该吞掉这个 `None`。
pub fn stylesheet(name: &str) -> Option<&'static str> {
    STYLESHEETS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, src)| *src)
}

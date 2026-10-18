#![forbid(unsafe_code)]
//! ZephyraUI —— 控件层。
//!
//! # 这个 crate 的边界
//!
//! 一句话：**产出 `Element`，不决定画在哪、也不决定画什么内容。**
//!
//! | 在这里 | 不在这里 |
//! |---|---|
//! | Button / CheckBox / Card / Dialog… 的**结构** | 路由、页面、应用状态 |
//! | 控件的**分类名**（`.button` / `.checked` / `.hovered`） | 文案（"确定" vs "OK"） |
//! | 控件自己的 `.zstyle`（[`styles`]） | 应用外壳与页面布局的 `.zstyle` |
//! | 排布所需的断点 / 触控判据（转发 `zephyra_core::responsive`） | 任何渲染后端 |
//!
//! # 三条不能碰的规矩
//!
//! 1. **外观不进源码。** 控件只发 `NodeTag` / `ElementKey` / 分类名 / 内容，
//!    颜色与尺寸全在 `.zstyle`。门禁 `tests/gates.rs` 按**整棵树**判。
//!
//! 2. **本 crate 不依赖渲染后端。** 控件库的用户不该被迫引入 D3D12。
//!    后端由应用（`gallery/`）用 feature 选。
//!
//! 3. **字符串全是 `&'static str`。** 元素树可以每帧重建（见
//!    `zephyra_core::frame::FrameCache`），任何运行期拼出来的字符串都会
//!    变成泄漏。所以控件函数不接受 `String`，也不 `format!`。

pub mod controls;
pub mod styles;
pub mod tokens;

pub use controls::*;
pub use styles::{stylesheet, STYLESHEETS};
pub use tokens as token_names;

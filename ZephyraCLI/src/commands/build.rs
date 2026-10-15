//! `build` 与 `run` 共用同一套「转交 cargo」逻辑（见 `run.rs`）。
//!
//! 拆成两个文件只是为了命令表里能各自指向一个入口；逻辑不重复 —— 重复一份
//! 「怎么拼 cargo 参数」正是那种「改了 run 忘了改 build」的经典漂移。

pub use super::run::build;

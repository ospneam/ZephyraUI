//! 薄壳，**只为了**占住 `cargo-zephyra` 这个可执行文件名 ——
//! `cargo zephyra dev` 靠「`cargo-` + 子命令名」的约定去 PATH 上找它。
//!
//! 逻辑同样在 `zephyra_cli::main_entry`；那里会剥掉 cargo 传来的 `zephyra`，
//! 所以 `cargo zephyra dev` 与 `zephyra dev` 走到完全相同的位置。

fn main() -> std::process::ExitCode {
    zephyra_cli::main_entry()
}

//! 构建期：把 `shaders/zephyra.wgsl` 编译成 SPIR-V。
//!
//! # 为什么在构建期编译
//!
//! Windows 侧用 fxc 在构建期把 HLSL 编成 DXBC（`zephyra-winit/shaders/zephyra.hlsl`）。
//! 这里同构：naga 在构建期把 WGSL 编成 SPIR-V。两者的好处一样 —— 着色器错误在
//! `cargo build` 时就暴露，而不是等到运行时才崩。
//!
//! # 为什么用 naga 而不用 glslang
//!
//! - glslang 是原生库，需要 apt 安装（本仓的 WSL 前置明确不装）；
//! - naga 是纯 Rust，走 cargo，与 wgpu 30 用的是同一个（wgpu 的着色器验证就靠它）；
//! - 输入是**同一份 WGSL** —— 与 wgpu 版逐项对应，由像素对照测试钉住。
//!
//! # 产物
//!
//! 输出到 `OUT_DIR/zephyra.spv`，运行时由 `include_bytes!` 读入。
//!
//! # 失败怎么办
//!
//! naga 的 WGSL 前端**不是**完整的 WGSL 实现（它是为 wgpu 服务的）。本文件的
//! WGSL 是 wgpu 实际加载过的那份，所以 naga 一定能编过 —— 这不是假设，是
//! 「wgpu 30 与 naga 30 用同一个前端」的直接推论。真编不过时 `cargo build` 失败，
//! 这是好事：比运行时崩好查。

use std::path::PathBuf;

fn main() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("shaders")
        .join("zephyra.wgsl");
    let wgsl = std::fs::read_to_string(&src).unwrap_or_else(|e| {
        panic!("读着色器源 {} 失败: {e}", src.display());
    });
    println!("cargo:rerun-if-changed={}", src.display());

    let module = naga::front::wgsl::parse_str(&wgsl).unwrap_or_else(|e| {
        panic!("WGSL 解析失败: {e}");
    });
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap_or_else(|e| panic!("WGSL 校验失败: {e:?}"));

    let spv =
        naga::back::spv::write_vec(&module, &info, &naga::back::spv::Options::default(), None)
            .unwrap_or_else(|e| panic!("SPIR-V 生成失败: {e}"));

    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR not set"))
        .join("zephyra.spv");
    let bytes: &[u8] =
        unsafe { std::slice::from_raw_parts(spv.as_ptr() as *const u8, spv.len() * 4) };
    std::fs::write(&out, bytes).unwrap_or_else(|e| {
        panic!("写 {} 失败: {e}", out.display());
    });
    println!("cargo:warning=SPIR-V 编译完成: {} bytes", spv.len() * 4);
}

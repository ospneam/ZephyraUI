// 本文件由 version.rs 的 #[cfg(test)] mod tests 抽出 —— 测试代码与生产代码分离。
// 模块路径不变（src 里的声明用 #[path] 指过来），所以 use super::* 与私有可见性都照旧。

use super::*;

#[test]
fn version_looks_like_real_windows() {
    // 这条测试是 `RtlGetVersion` 那段 `extern` 的**唯一**判据：
    // 如果签名/结构体布局写错，拿到的 major 会是 0 或乱七八糟的数。
    let (major, minor, build) = os_version();
    assert!(
        major >= 10 && minor <= 99 && build >= 10_000,
        "读到不像 Windows 的版本号：{major}.{minor}.{build}（若 manifest 没生效，\
             GetVersionExW 会给 6.2 —— 那正是本文件要避开的情形）"
    );
}

#[test]
fn build_缓存与直接读一致() {
    // 缓存若写死一个值，这条会红 —— 那正是"版本探测写错了"的症状。
    assert_eq!(build_number(), os_version().2);
}

#[test]
fn repeated_reads_are_stable() {
    let a = os_version();
    assert_eq!(a, os_version());
    assert_eq!(a, os_version());
}

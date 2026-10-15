// 本文件由 quad.rs 的 #[cfg(test)] mod kind_branch_tests 抽出 —— 测试代码与生产代码分离。
// 模块路径不变（src 里的声明用 #[path] 指过来），所以 use super::* 与私有可见性都照旧。

use super::kind::*;
use crate::quad::kind_thresholds::{GLYPH_MAX, GLYPH_MIN, IMAGE_MIN, PATH_MIN};

/// 每个值必须落在它**预期**的那个分支里。
///
/// 这是唯一需要检查的：`nointerpolation`（HLSL 侧 `VSOut.kind`）保证片元
/// 拿到的永远是某个顶点的原值，不存在插值中间值 —— 所以"相邻间距"不是
/// 这里该关心的量，把间距当成检查项是**把插值的顾虑错记在这里**。
///
/// 检查本身是常量折叠的（`const f32` 与字面量比较），所以它防的不是运行期
/// 的偏差，而是**日后有人改动这些常量**。clippy 因此报
/// `assertions_on_constants` —— 那条警告在这里是误报：我们要的正是
/// 「这个常量关系一旦被改坏，编译就该吵」。
#[test]
#[allow(
    clippy::assertions_on_constants,
    reason = "防的正是常量被改坏；编译期求值正是我们要的"
)]
fn each_value_lands_in_its_intended_branch() {
    assert!(RECT < GLYPH_MIN, "RECT 应在 SDF 分支");
    assert!(GLYPH > GLYPH_MIN && GLYPH < GLYPH_MAX, "GLYPH 应在字形分支");
    assert!(IMAGE > IMAGE_MIN, "IMAGE 应在图像分支");
    assert!(RECT < GLYPH && GLYPH < IMAGE, "三者必须有序");
}

/// 同上，但把 `PATH` 也算进去。
///
/// ⚠ `PATH` 的判据与前三类**结构不同**：它在 HLSL 里排在最前面，判据是
/// `kind > PATH_MIN`（无上界）。所以要额外钉住 `PATH > PATH_MIN`，
/// 否则它会落进图像分支去采样纹理。
#[test]
#[allow(
    clippy::assertions_on_constants,
    reason = "防的正是常量被改坏；编译期求值正是我们要的"
)]
fn path_lands_in_its_own_branch_and_not_in_the_image_branch() {
    assert!(
        PATH > PATH_MIN,
        "PATH={PATH} 必须大于 PATH_MIN={PATH_MIN}，否则落进图像分支"
    );
    assert!(
        PATH > IMAGE,
        "PATH 必须排在 IMAGE 之后取值（着色器里先判 PATH），两者不能相等"
    );
}

/// 每个分支阈值都严格落在它分界的那两个 `kind` 值之间。
///
/// ⚠ `PATH_MIN` 单独钉：它在着色器里**不参与**分割（PATH 是最后一个分支，
/// 判据无上界），但那个字面量 `2.5` 仍然必须与 `IMAGE` 和 `PATH` 对齐 ——
/// 写 `3.0` 会让 `> 3.0` 把恰好等于 3.0 的多边形漏进图像分支。
///
/// ⚠ `IMAGE_MIN` 只钉"在 IMAGE 与 PATH 之间"，**不**钉它与 GLYPH 的关系：
/// 字形分支是**双侧**判据（`> 0.5 && < 1.5`），1.5 同时是它的上界和图像
/// 分支的下界。把它当成"只分 IMAGE / PATH"的阈值去要求它大于 GLYPH，
/// 是把双侧区间的上界当成了单侧阈值。
#[test]
#[allow(clippy::assertions_on_constants, reason = "同上")]
fn path_threshold_sits_between_image_and_path() {
    assert!(
        IMAGE < PATH_MIN && PATH_MIN < PATH,
        "PATH_MIN={PATH_MIN} 必须落在 IMAGE({IMAGE}) 与 PATH({PATH}) 之间"
    );
    assert!(
        PATH > PATH_MIN,
        "PATH={PATH} 必须大于 PATH_MIN，否则落进图像分支去采样纹理"
    );
}

/// 字形分支是**双侧**判据：下界与上界都必须夹住 `GLYPH` 且排除两侧的邻居。
#[test]
#[allow(clippy::assertions_on_constants, reason = "同上")]
fn the_two_sided_glyph_branch_excludes_its_neighbours() {
    assert!(RECT < GLYPH_MIN, "RECT({RECT}) 必须被字形分支排除");
    assert!(
        GLYPH_MIN < GLYPH && GLYPH < GLYPH_MAX,
        "GLYPH 必须落在双侧区间内"
    );
    assert!(
        IMAGE > GLYPH_MAX,
        "IMAGE({IMAGE}) 必须被字形分支排除（GLYPH_MAX 同时是图像分支下界）"
    );
}

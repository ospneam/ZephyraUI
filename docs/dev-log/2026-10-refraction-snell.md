# 折射：换成斯涅尔定律，并第一次在真 GPU 上证明它生效

日期：2026-10

## 背景

`Filter::Refraction` 的整条链路（core → fx → HLSL → D3D11/D3D12）在上一轮就写完了，
但**从来没在真 GPU 上验证过** —— 只有 `--dump-cpu`，而那条路不画滤镜。

而折射恰好是本仓**最不可能靠肉眼验收**的能力（理由见下）。

## 现象

开始验证时，第一条 GPU 诊断就打出一个刺眼的数：

```
[gallery] 首帧#0 redrawn=true quads=379 offscreen=3 refraction=3
           unimplemented_filters=1 unimplemented_blends=0 skipped=0
```

`refraction=3` 是好的（三块玻璃各走一趟 `PSRefraction`）。
但 `unimplemented_filters=1` —— 按 `RenderReport` 自己的文档，判据是
`unimplemented_filters == 0`。也就是说：**每次用折射，都有一条滤镜「没接上」**。

## 排查

### 一、`unimplemented_filters` 恒为 1

这是**会计口径**错，不是折射没跑。账是这样走的：

| 步 | `unimplemented_filters` |
|---|---|
| `expand` 展开这条带滤镜的命令 | +1 |
| 离屏 pass 内部 `saturating_sub(1)` | −1（归零）|
| `absorb(inner)` | +0 |
| 外层 `saturating_sub(1)` | −1（已在 0，饱和，保持 0）|
| **`redraw` 原图那趟 `draw_range` 又展开同一条命令** | **+1 ← 元凶** |
| 该值被减掉 | 从来没有 |

重画原图那一趟**该减不減**：滤镜已经在离屏 pass 里执行过了，重画画的只是元素本体。

**排除的假设**：

| 假设 | 为什么不成立 |
|---|---|
| 有第 4 条滤镜命令（`refs` 页里还有别的） | 不是。`--dump-cpu` 逐条列出来只有 3 条，且与 `refraction=3` 一致 |
| `refresh_dst_copy` 失败降级成普通合成 | 不是。那样 `refraction_passes` 不会 +1（它只在 `dst_srv.is_some()` 时加） |
| `Plain` 段也含滤镜命令 | 不是。在 `Plain` 段打了探针，`unimplemented_filters` 为 0 |
| 内层 report 里带计数，`absorb` 求和后翻倍 | 不是。三次内层 report 全是 `unimpl=0` |

第一次修的时候把减法写在 `absorb` **之前** —— 而 `absorb` 立刻又加了回去，净效果为零，
症状与没写一样。**这个顺序错误不会自己暴露**，所以注释里写明了。

### 二、玻璃铺满了整幅画面

修好计数后去写像素门禁，发现无论 IOR 取多少、位移多大，画面**逐像素不变**。

用探针着色器（把 `local` / `d` / `mask` / `dG.a` 编码进 RGB 再读回）定位到：
`position` 铺的是**离屏范围**（整块 RT），而 `local` 按**元素矩形**铺。

四边形越出元素的那一圈里，`local` 仍落在 `[0, ew]` 内，SDF 于是判它「在元素内部」，
`mask` 恰好是 1 —— 于是**整幅画面都成了玻璃**。

**而那正是折射本该有的样子，所以没人会当它是坏的。**
真正露出病状的地方只有一处：玻璃外的画面被撕裂。

**排除的假设**：

| 假设 | 为什么不成立 |
|---|---|
| `i.local` 单位错了（是 uv 不是像素） | 不是。探针显示 x=128 处 `local.x/128 = 128`，单位正确 |
| `g_dst` 是空的 | 不是。探针显示玻璃内 `dG.a = 255` |
| `mask` 恒为 0 | 不是。探针显示玻璃中心 `mask = 255` |
| 着色器没跑到（`.bin` 没重编） | 不是。`shader_assets_are_in_sync` 绿，且探针输出确实变了 |

修法是让 `position` 与 `local` 覆盖**同一块区域**：把 `pass.bounds` 换成元素的内容矩形。
函数签名也跟着简化（不再需要 `element_rect_in_rt`）。

### 三、判据本身错了三次

这部分不比前两个轻 —— 写门禁时连着三次判据不成立，每次都是**探针选错**，不是代码错。

| 版本 | 背景 | 现象 | 为什么是探针的错 |
|---|---|---|---|
| 1 | 左右黑白硬边 | diff = 0 | 折射的位移在深度上是 `t → 0` 时最大、到倒角带中段衰减到**零点几个像素**。纯黑往左挪 1 像素，采样点仍在纯黑区，画面逐像素相同 |
| 2 | 8px 黑白条纹 + 要求「IOR=1 时逐像素相同」 | 4560 个像素差 | 条纹对**亚像素**的 uv 误差也敏感（`DstUv` 浮点误差约 0.03 像素，8px 周期下仍可能跨过条边界）。于是即使位移恒为 0 也红 |
| 3 | 纯色背景验「位移为 0」+ 条纹验「位移生效」 | 过了 | 分工：位移为 0 用**对 uv 误差不敏感**的纯色；位移生效用**对位移敏感**的条纹 |

色散那条又错了第四次：沿用黑白条纹，判「红蓝分离像素数」，得 0 个 ——
**黑白条纹的 R = G = B**，三个通道采到哪根条上都是同一个灰度，
「把 R 和 B 分开采样」在纯灰度图上**原理上不可见**。

改成红蓝相间的背景，并把判据从「红蓝像素的绝对数量」改成「有色散 vs 无色散的两图之差」。
后者不受位移量级跨三个数量级的影响：紧贴边缘那圈位移溢出被 `saturate` 夹住、
带中段位移约 1 像素，两段都让「绝对数量」落在噪声量级（实测 0 个），但只要颜色**变了**
两图之差就数得到。

## 结论 + 门禁

折射的位移公式换成斯涅尔定律（`zephyra.hlsl` 的 `PSRefraction`）：

```
bezel = min(color.g, radius, half_size)        // 倒角带宽度
t     = saturate(-d / bezel)                    // 只在倒角带内 0..1
h     = (1 - (1-t)^4)^0.25                      // 高度剖面，t→0 处斜率发散
dh    = (h(t+dt) - h) / dt                      // 有限差分
slope = atan(dh * thickness / bezel)            // 面倾角
θr    = asin(sin(slope) / ior)                  // 斯涅尔
disp  = h * thickness * (tan(slope) - tan(θr))  // 位移
```

与旧版 `n * (strength * mask)` 相比有两处本质差别：

1. **位移是「新旧入射角之差」**，所以在深度上**先增后减**（近边缘几乎不弯、倒角带中段最弯、
   到带内侧回 0），而旧版是一条单调曲线；
2. **IOR = 1 时位移恰好为 0** —— 这给了判据一条物理依据极强的反向断言。

参数从 2 个（强度、色散）扩到 4 个（厚度、倒角带宽、色散、折射率），`color.g/.b`
是空着的通道，不加顶点字段。

| 门禁 | 钉住什么 |
|---|---|
| `refraction_resamples_the_backdrop_on_d3d12` | 四幅图两两对照：IOR=1 ⇒ 与「没有玻璃」几乎逐像素相同（容差 0.5%，浮点噪声实测 0.14%）；IOR=1.5 ⇒ 内部显著重新采样；厚度翻倍 ⇒ 受影响像素更多 |
| `refraction_dispersion_changes_the_sample_positions_on_d3d12` | 色散把 R / B 的采样点拉开（红蓝交替背景 + 两图之差） |
| `zero_dispersion_samples_red_and_blue_at_the_same_place` | 同一组参数渲染两次逐像素相同（渲染确定性） |
| `refraction_quad_geometry_covers_the_element_exactly` | 折射四边形的 `position` / `local` / `half_size` 覆盖**同一块区域**，且那块区域是元素 |
| `RenderReport::refraction_passes` | 正面计数：只在真正走 `PSRefraction` 时 +1，拿不到目标副本而降级时**不**加 |

## 顺带修正的两处

- **`vs_5_1` 是错的**。`zephyra-winit-d3d12/src/shader.rs` 的文档写着「`fxc /T vs_5_1`」，
  而实际只有 `*_5_0` 能逐字节复现已提交的 `.bin`。用 `vs_5_1` 编出来的 `vs.bin`
  **大小完全相同**（1152 字节）但 `CreateVertexShader` 返回 `E_INVALIDARG` ——
  整个 D3D11 GPU 套件（31 条）一起红，而画面路根本还没走到。判据是可复核的：
  `git show HEAD:…hlsl` + 两种 profile 各编一遍，比 SHA256。
- **`d3d12_draw` 的 17 条 `#[ignore]` 已经过期**。那条注释写着「本机 D3D12 资源创建
  整体不可用」，实测 16 条全部通过（本机有 NVIDIA MX330）。本轮据此把折射门禁也标上
  `#[ignore]`，与同套件其余各条一致 —— 至于**要不要**整体摘掉 `#[ignore]`，
  是一次独立的决定（`AGENTS.md` 明确说别为了让计数好看去删），本轮不动。

## 验收

- `cargo fmt --all -- --check` 干净
- `cargo clippy` 我涉及的 7 个 crate —— 0 警告
  （`--workspace` 仍有 `zephyra-vulkan` 的既有告警，本轮未触碰该 crate）
- `cargo test --workspace --no-fail-fast` —— **800 passed / 1 failed / 25 ignored**
  唯一失败仍是 `zephyra-winit` 的
  `backend_parity::every_blend_mode_looks_the_same_on_both_backends`，本轮之前就红
- `cargo test -p zephyra-winit-d3d12 -- --include-ignored` —— 19 passed / 0 failed
- `--dump-cpu --route effects` —— 三档折射链解析正确：
  `Refraction { strength: 14, bezel: 18, dispersion: 0.08, ior: 1.45 }` 等
- gallery 窗口路（DirectComposition，D3D12）—— `refraction=3`、`unimplemented_filters=0`

## 未做

- **D3D11 的折射像素门禁**：D3D11 的 `d3d11_draw` 已有同构夹具，本轮只把算法与
  接线同步过去，**没有**加等价的像素门禁 —— D3D11 与 D3D12 用的是同一份 HLSL
  与同一套顶点数据，所以两条真机门禁的内容会是重复的。
- **Vulkan**：滤镜管线仍未接通（那边 `fs_blur_h` / `fs_composite` 没被调用），
  所以 Linux 上这一页与「没设滤镜」无异。
- **glass 页面上的高光 / 内阴影**：liquid-glass 的 `webgl.html` 还有镜面高光、
  内阴影、内边缘亮线与玻璃外投影四层。本轮只移植了**位移**（也就是「算法」本身），
  那四层属于外观装饰，且玻璃外投影需要四边形画到元素矩形之外 —— 而那正是
  上面第二个 bug 的成因，两者要一起改才安全。
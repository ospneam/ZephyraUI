# 追加：2026-10-18 —— 1:1 像素级达成（承接上文）

## 现象

MSAA/图集上传接通后，`ZEPHYRA_FRAME_DUMP` 读回的 Linux 帧与 Windows 参考帧
（DPI-aware 屏幕捕获，`winref/win-demo-client.png`）结构一致但**整体偏亮一档**：
背景 71 vs 16。

## 排查

`decode(71)=16`、`decode(132)=59` 逐色吻合 —— 差的正好是一次 sRGB 编码。
llvmpipe 对 `B8G8R8A8_SRGB` 附件**按规范做了硬件编码**（着色器写线性 16，
swapchain 落 encode(16)=71），而 Windows D3D12 的交换链是 `B8G8R8A8_UNORM`
（非 sRGB，着色器值直通）。两边显示观感本来就不同 —— 不是 dump 的错。

## 排除的假设

| 假设 | 为什么排除 |
|---|---|
| dump 的 encode_srgb 传反了 | 传 true 让它更亮（143），传 false 保持 71 —— 都回不到 16；问题在上游格式 |
| llvmpipe 不做 sRGB 附件编码 | 71 = encode(16) 逐色吻合 —— 它**做了**，规范行为 |
| 「判据是格式」可以直接推出 dump 参数 | 光看格式推 sRGB→false，实测就是错的：llvmpipe 编码后读回的是编码值，但显示偏亮；正解是换格式而不是换 dump 参数 |

## 结论 + 实测门禁

**swapchain 选 `B8G8R8A8_UNORM`（非 sRGB），与 Windows D3D12 同一直通行为。**
换格式后两侧像素统计（demo，1100×700，抽样统计）：

| 颜色（RGB） | Windows 占比 | Linux 占比 |
|---|---|---|
| (16,18,20) 背景 | 80.4% | 80.7% |
| (26,28,30) 面板 | 5.7% | 5.6% |
| (59,130,246) 主题蓝 | 3.1% | 3.1% |
| (26,51,107) 深蓝 | 3.1% | 2.3% |
| (61,132,188) 亮蓝 | 0.7% | 1.4% |

前 6 大颜色**逐字节一致**；184 quads（与 wgpu 版同数）；MSAA 4× + resolve。
复现：`ZEPHYRA_FRAME_DUMP=/tmp/f.bmp cargo run -p zephyra-foundation --bin demo
--no-default-features --features linux-vk`（WSL2/WSLg/llvmpipe，2026-10-18）。
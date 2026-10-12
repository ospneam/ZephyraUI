# DXGI flip model —— 为什么「只重画脏区」需要前提

> 出处：`https://learn.microsoft.com/en-us/windows/win32/direct3ddxgi/dxgi-flip-model`
> （2018-05 原始文档，2025-04 更新，2026-10 查）。

## 1. flip model 与 bitblt 的差别（文档原文要点）

- **bitblt**：`Present1` 时运行时把后缓冲**拷贝**进 DWM 的重定向表面。
- **flip**：**所有后缓冲与 DWM 共享**，DWM 直接从这些后缓冲合成，无需额外拷贝。

由此得到一条对本仓至关重要的推论（文档原文）：

> If you pass `NULL` to `pFullscreenDesc` … the runtime creates one extra back buffer
> and **rotates whichever handle belongs to the buffer that becomes the front buffer
> at presentation time**.

**后缓冲是轮转的**：呈现后系统会换一个后缓冲给你画。因此「后缓冲里还留着上一帧」
这一前提**在 flip model 下不成立** —— 你下一帧拿到的可能是完全另一块内存。

## 2. 直接后果：damage 必须退化为整帧（除有持久 RT 的情况）

`zephyra-core` 的 `Damage::Rect` 只在「脏区之外仍是上一帧画面」时才能用。
按上节，flip model 的后缓冲逐帧轮转 ⇒ 该前提不成立。

两条合法出路（都在本仓采纳）：

1. **渲染到自有持久纹理**，只把脏区画上去，帧末把整张纹理 present 出去。
   本仓 D3D12 后端在 **MSAA 开启**时正是这个形态（`chain.msaa_rtv()` 是自有纹理，
   帧末整体 resolve 进后缓冲）—— 所以 MSAA 开时按脏区绘制成立。
2. **否则强制整帧**：D3D11 后端与 D3D12 的 MSAA-off 路径都直接用 `Damage::Full`。

这一条是**上一轮修掉「点一下界面全花」那个 bug 的文档依据**：那条 bug 的根因是
「整屏 clear + 只画脏矩形」，而这里说明了为什么必须先保住上一帧画面。

## 3. 文档给出的其他硬性约束（对本仓的核对结论）

| 约束（文档原文） | 本仓现状 | 结论 |
|---|---|---|
| `BufferCount` 取 2..16，否则有等待 DWM 释放的惩罚 | `BUFFER_COUNT = 2`（+1 给 MSAA 槽） | ✅ 满足 |
| `Format` 须为 `R16G16B16A16_FLOAT` / `B8G8R8A8_UNORM` / `R8G8B8A8_UNORM` | 后缓冲 `R8G8B8A8_UNORM`，与 PSO 的 `RTVFormats[0]` 一致 | ✅ 满足 |
| **MSAA 在 `DXGI_SWAP_CHAIN_DESC1` 上不受支持**（`SampleDesc.Count` 必须为 1） | 交换链本身非 MSAA，MSAA 落在**自有中间纹理**上 | ✅ 满足，且是第 2 节能成立的原因 |
| 每个 `HWND` 只挂一条 flip 交换链 | 一窗一链 | ✅ |
| 不要让 GDI `ScrollWindow` 之类与 flip 链共存 | 本仓未用 | ✅ |
| Win7 及更早用 `FLIP_SEQUENTIAL` 会创建设备失败 | 仅 Win10/11 | ✅ |

## 4. present 统计（本轮**不**做，留口）

文档给了 `GetFrameStatistics` / `GetLastPresentCount` 与
`DXGI_ERROR_FRAME_SQUINT_DISJOINT`（首次调用、模式切换时）的完整用法。
本轮不实现（不在需求内），但记录在此：将来做「帧率统计」时不必重新查，
且注意 **`PresentCount == 0` 表示呈现被丢弃**。
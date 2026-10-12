# DXGI 交换效果 —— 为什么本仓两个后端都「按脏区绘制」只在特定条件下成立

> 出处：`https://learn.microsoft.com/en-us/windows/win32/api/dxgi/ne-dxgi-dxgi_swap_effect`
> （2026-10 查）
> 配套：`docs/win/05-flip-model.md`（flip model 的缓冲区轮转）

## 1. 文档原文（D3D12 一节，节选）

> `DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL`
> Value: 3. Flags to present a rendered image to the screen using the flip
> presentation model. **The DXGI discards the contents of the back buffer after
> calling Present.**
> This flag is valid only for Direct3D 12 and Direct3D 11.2 apps.

以及 bitblt 一节：

> `DXGI_SWAP_EFFECT_DISCARD` … **Direct3D 12: This enumeration value is never
> supported.** D3D12 apps must use `DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL` or
> `DXGI_SWAP_EFFECT_FLIP_DISCARD`.

## 2. 这条文档解释了本仓上一轮修掉的那个 bug

我们的两处后端都用了 composition swapchain（DComp 路径恒为
`FLIP_SEQUENTIAL`）。文档明写 Present 之后**后缓冲内容被丢弃**。

于是这一行代码是错的：

```rust
// ✗ 曾经这么写
ClearRenderTargetView(rtv, ..., None);          // 整屏清成底色
RSSetScissorRects(&[damage_rect]);               // 只画脏区
```

结果是：**整屏变成底色 + 只有脏区那一块是对的**。首帧正常（`Damage::Full`），
所以缺陷只在交互之后出现 —— 这就是它难以定位的原因。修法与依据见
`MIGRATION.md` §7.4。

## 3. 结论表（本仓现状）

| 后端 | 渲染目标 | Present 后脏区外是否还有上一帧 | 做法 |
|---|---|---|---|
| D3D12 + MSAA **开** | 自有 MSAA 纹理（`chain.msaa_rtv()`） | **有**（自有纹理不被系统丢弃），且帧末整体 resolve 进后缓冲 | 按脏区 clear + 按脏区画 ✓ |
| D3D12 MSAA **关** | flip model 后缓冲 | **无**（被丢弃） | **强制整帧** |
| D3D11（`zephyra-winit`） | flip model 后缓冲 | **无** | **强制整帧** |

「强制整帧」的代价写在代码注释里（`render_pass` 的 `backdrop` / MSAA 判定处）：
交互帧整帧重绘。**取舍是画面对，而不是画面快。**

## 4. `sample` 参数的约束（文档确证）

同一页的 `DXGI_SWAP_CHAIN_DESC1::SampleDesc` 说明里：

> **MSAA is not supported with the flip model.** Therefore, `SampleDesc.Count`
> must be 1 and `Quality` must be 0.

本仓的落地（`zephyra-dcomp/src/lib.rs::base_swapchain_desc`）：
交换链自身 `SampleDesc { Count: 1, Quality: 0 }`，
MSAA 落在**自有的中间纹理**上（`Swapchain::msaa`），帧末 `ResolveSubresource`
把整张纹理解析进后缓冲。

这条正是 §3 表格第一行能成立的前提：**脏区外的内容在自有的 MSAA 纹理里，
不在后缓冲里**，而 resolve 是整张的。
# 抄 Chromium 的合成器：`E:\chromium`

本轮修「拖窗口时一直闪」的第三条依据，全部来自真实源码。

最贴近本仓宿主形态的那份文件是
`components/viz/service/display_embedder/software_output_device_win_swapchain.cc`
（`CreateSwapChainForComposition` + `FLIP_SEQUENTIAL` + 2 缓冲 + DComp 视觉树）。

---

## 1. 已抄

### 1.1 「窗口尺寸与交换链不一致」= 不可恢复，不要跛着继续

`software_output_device_win_swapchain.cc:96`：

```cpp
    if (FAILED(hr)) {
      // If ResizeBuffers fails, the swapchain and window sizes will be out of
      // sync, causing unexpected behavior such as permanent gutters or
      // clipping. Therefore, terminate the GPU process to refresh state.
      RestartGpuProcessForContextLoss(...);
```

他们有专门的**死亡测试**（`software_output_device_win_swapchain_unittest.cc:278`）：

```cpp
// Ensure the GPU process is terminated if ResizeBuffers fails.
TEST_F(..., ResizeBuffersFailure) {
  ...
  EXPECT_DEATH(device_->Resize(gfx::Size(1180, 620), 1.f), "");
```

**本仓的对应物**：没有第二个进程可重启，所以改成「判死 + 停下 + 只报一次」
（commit `9963ba1`）。这是**有意的分歧**，理由是单进程宿主；不是漏抄。

### 1.2 「不要 present 一张尺寸不匹配的帧」

`components/viz/service/display/display.cc:997`：

```cpp
  bool size_matches = surface_size == current_surface_size_;
  ...
  bool should_draw = have_copy_requests || (have_damage && size_matches);
  bool should_swap = !disable_swap_until_resize_ && should_draw && size_matches;
```

`disable_swap_until_resize_` 默认 `true`（`display.h:357`），只在
`Display::Resize` 里清零（`display.cc:461`）。

**本仓的对应物**：`SwapState` 三态 + `should_draw_frame()` 纯函数。
他们的 `size_matches` 就是我们 `Failed` 那一支；他们的
`disable_swap_until_resize_` 对应「resize 没成功前不 present」。

### 1.3 最小化 ⇒ 不画

`ui/views/win/hwnd_message_handler.cc:2968`：

```cpp
void HWNDMessageHandler::OnSize(UINT param, const gfx::Size& size) {
  ...
  delegate_->HandleWindowMinimizedOrRestored(param != SIZE_MINIMIZED);
```

一路走到 `components/viz/service/display/display_scheduler.cc:602`：

```cpp
bool DisplayScheduler::ShouldDraw() const {
  return needs_draw_ && !output_surface_lost_ && visible_ &&
         !damage_tracker_->root_frame_missing();
}
```

**本仓的对应物**：`client_size()` 为 0 时返回 `SwapState::Failed`。
顺带修掉一个真缺陷 —— `Swapchain::resize` 对 `(0,0)` 早退 `Ok(())`，
链停在旧尺寸，而我们照样把 0×0 视口画进旧后缓冲并 present。

### 1.4 `DXGI_STATUS_OCCLUDED` 不是错误

`ui/gl/swap_chain_presenter.cc:1285`：

```cpp
      // Ignore DXGI_STATUS_OCCLUDED since that's not an error but only
      // indicates that the window is occluded and we can stop rendering.
```

本仓 `Swapchain::present` 早就这么做了（`0x087A_0001` 提前 `Ok(())`）。

### 1.5 `GetDeviceRemovedReason` 才是判据

`ui/gl/dcomp_presenter.cc:213`：

```cpp
    const HRESULT device_removed_reason =
        gl::GetDirectCompositionD3D11Device()->GetDeviceRemovedReason();
    const bool not_device_removed = SUCCEEDED(device_removed_reason);
    if (not_device_removed && result.error().hr != PRESENTATION_ERROR_LOST) {
      ... base::debug::DumpWithoutCrashing();
    } else {
      // Ignore device removed cases as they don't usually indicate a problem
      // originating from viz.
    }
```

`Gpu::removed_reason()` 本仓**早就写了**（`surface.rs:328`，注释里还记着
「`CreateCommittedResource` 的 `E_INVALIDARG` 只是 `DEVICE_HUNG` 投的影子」），
只是**没接到 `present` / `resize` 的失败路径上** —— 用户报的那个 0x887A0005
就是这么一路漏成三连错的。本轮接上了。

---

## 2. 最重要的一条：**Chromium 从不用 D3D12 建 DComp 交换链**

全仓 `CreateSwapChainForComposition` 只有三个调用点，**每一个传的都是 D3D11 设备**：

| 位置 | 传入的设备 |
|---|---|
| `software_output_device_win_swapchain.cc:169` | `d3d11_device` |
| `d3d_image_backing_factory.cc:550` | `d3d11_device_` |
| `dxgi_swap_chain_image_backing.cc:92` | `d3d11_device` |

Chromium 的 **D3D12** 那条路根本不用 DXGI swapchain，而是走 composition texture：

`ui/gl/dc_layer_tree.cc:144`：

```cpp
  d3d12_command_queue_ = GetDirectCompositionD3D12CommandQueue();
  if (d3d12_command_queue_) {
    Microsoft::WRL::ComPtr<EXPERIMENTAL_IDCompositionDevice6> dcomp_device6;
    HRESULT hr = dcomp_device_.As(&dcomp_device6);
    dcomp_device_6_ = std::move(dcomp_device6);
  }
```

`ui/gl/dc_layer_tree.cc:892`：

```cpp
    // First call `PresentCompositionTextures` to specify the queue that the
    // next `Commit` call uses to actually present the composition textures.
    IUnknown* command_queue_unk = dc_layer_tree_->d3d12_command_queue_.Get();
    HRESULT hr = dc_layer_tree_->dcomp_device_6_->PresentCompositionTextures(
        &command_queue_unk, 1);
```

而且它在 `ui/gl/direct_composition_support.cc:1249` 明确把 D3D12 挡在 feature 后面：

```cpp
  if (SUCCEEDED(d3d11_device.As(&d3d11on12_device)) &&
      !base::FeatureList::IsEnabled(features::kDCompOnD3D12)) {
    // IDCompositionTexture is not implemented on an 11on12 ... It is however
    // implemented on D3D12, and if the feature is enabled,
    // IDCompositionDevice6::PresentCompositionTextures will be used for D3D12
    // backed composition textures.
    return false;
  }
```

**对我们的意义**：本仓 `zephyra-winit-d3d12` 走的是「D3D12 设备 + `CreateSwapChainForComposition`
+ DComp `Present1`」—— 正是 Chromium **不**走的那条。修好 use-after-free 之后如果还 TDR，
下一步就是这个架构选择，而不是继续调参数。

两个方向（都不是小改动，都要单独评估）：
- **换 D3D11 后端跑 DComp 窗口**（与 Chromium 的 swapchain 路径完全一致）。本仓已有
  `zephyra-winit` 的 D3D11 渲染层，但没有 `HostWindow`。
- **实现 composition texture 路径**（`IDCompositionDevice6` + `IDCompositionTexture`
  + `PresentCompositionTextures`）。

---

## 3. 没抄，以及为什么

| 项 | 出处 | 不抄的理由 |
|---|---|---|
| `Present1` 脏矩形局部呈现 | `software_output_device_win_swapchain.cc:256` | Chromium 自己说硬 GPU 路径是全帧 Present，脏矩形只在**软件 + DComp** 路径用。我们正是硬 GPU 路径 |
| 交换链换内容时 `Commit` | `dc_layer_tree.cc:626` | 本仓已有（`rebind_visual`），Their extra lesson: free DComp resources 后必须 `Commit`，否则 DWM 崩了窗口会黑到下次 Commit —— **待办** |
| `child_window_win.cc` 吞掉 `SetWindowPos` 失败 | `:195` | 我们没有子窗口（合成链直接挂主窗） |

---

## 3. 待办（有证据、没动手）

### 3.1 换视觉内容时 `Commit` + 旧内容活到 Commit 之后 ✅ 本轮做了

`dc_layer_tree.cc:626`：

```cpp
  if (dcomp_visual_content_changed) {
    hr = content_visual_->SetContent(dcomp_visual_content_.Get());
    CHECK_EQ(hr, S_OK);            // "SetContent(nullptr)" is used on teardown to drop refs
  }
```

**这条直接命中了本轮 TDR 的真凶**（见 §4）。

`ui/gl/swap_chain_presenter.cc:1155`：

```cpp
  if (first_present_) {
    first_present_ = false;
    UINT flags = DXGI_PRESENT_USE_DURATION;
    // DirectComposition can display black for a swap chain between the first
    // and second time it's presented to - maybe the first Present can get lost
    // somehow and it shows the wrong buffer. In that case copy the buffers so
    // all have the correct contents, which seems to help. ...
    for (size_t i = 0; i < swap_chain_buffer_count_ - 1; ++i) {
      hr = swap_chain_->Present(0, flags);
      ...
      context->CopyResource(dest_texture.Get(), src_texture.Get());
    }
    // Additionally wait for the GPU to finish ... or else there still may be
    // a black flicker when presenting expensive content (e.g. 4k video).
    ... dxgi_device2->EnqueueSetEvent(event.handle()); event.Wait();
```

**为什么这条最相关**：拖动期间我们的 `Swapchain::resize` 失败一次就
`recreate()` 整条链，也就是**拖动中会新建很多条交换链**，每一条都是
DComp 眼里的「新链 + 首次 present」⇒ 每次都可能黑一帧。这与我已修的
「半死状态 / 尺寸不匹配」是**两套不同的闪**。

对应实现：recreate 后的第一帧把所有后缓冲塞满同样内容 + 等 GPU
（`Gpu::wait_gpu()` 已有，见 `surface.rs:409`）。

### 3.2 设备丢失的阶梯

Chromium 有一整套（`gpu_channel_manager.cc:1050`、`gpu_service_impl.cc:897`、
`gpu_data_manager_impl_private.cc:482`）：

```cpp
  fallback_modes_.push_back(gpu::GpuMode::DISPLAY_COMPOSITOR);   // 纯软件合成
  if (SoftwareGLAllowed()) {
    fallback_modes_.push_back(gpu::GpuMode::SOFTWARE_GL);        // SwiftShader
  }
```

本仓现在只到「判死 + 停下」，**没有自动重建设备**。要做的话阶梯应当是：
`DEVICE_RESET` → 下一帧重试 → 连续 N 次 → 重建 D3D12 设备与全部依赖它的
资源（`Renderer` / `TextureTable` / 图集）→ 仍失败才停下。
`Gpu::wait_gpu` 与 `FrameCache::invalidate` 都现成。

### 3.3 BeginFrame 只在需要时观察 VSync

`components/viz/service/frame_sinks/external_begin_frame_source_win.cc:109`：

```cpp
void ExternalBeginFrameSourceWin::OnNeedsBeginFrames(bool needs_begin_frames) {
  if (observing_vsync_ == needs_begin_frames) return;
  ...
  if (needs_begin_frames) gl::VSyncThreadWin::GetInstance()->AddObserver(this);
  else                   gl::VSyncThreadWin::GetInstance()->RemoveObserver(this);
}
```

本仓 `about_to_wait` 已经有 `dirty` 门（静止时一帧不跑），等价目的。
---

## 4. 本轮 TDR 的真凶：`recreate` 时的 use-after-free

用户报的日志串：

```text
[zephyra-d3d12] 交换链已重建，重新挂进 DComp 视觉树
[zephyra-d3d12] present 失败: GPU 设备实例已经暂停 (0x887A0005)
[zephyra-d3d12] 设备不可用：DXGI_ERROR_DEVICE_HUNG (0x887A0007)
```

`GetDeviceRemovedReason` 给的是 **`DEVICE_HUNG`**（真 TDR），不是 `DEVICE_REMOVED`。

### 顺序错在哪

`Swapchain::resize` 的重建分支**当场**析构旧链：

```rust
*self = fresh;          // ← 旧 IDXGISwapChain1 在这里 Release
self.was_recreated = true;
```

而重新挂 DComp 是在**之后**（宿主 `sync_swapchain` 看到 `was_recreated` 才调
`rebind_visual`）。也就是说存在一个窗口期：

> DComp 的 visual 上还挂着 `SetContent(旧链)`，而旧链已经被 `Release` 了。

下一帧合成 / `Present` 拿到的是**已释放**的交换链 ⇒ use-after-free ⇒ GPU hang。

### 修法

顺序改成：**建新链 → `SetContent(新)` → `Commit` → 才放旧链**。

- `Swapchain::superseded: Option<IDXGISwapChain1>` 保住旧链；
- `take_superseded()` 由宿主在 `rebind_visual()` **成功后**才调；
- `rebind_visual()` 改成返回 `bool`：它就是**释放旧链的许可**。
  `SetContent` 或 `Commit` 任一失败都返回 `false`，此时**绝不能**放旧链 ——
  DComp 还在用它。

### 为什么现在才暴露

`live_resize` 之前，一次拖动只产生一个 `Resized` 事件 ⇒ 一次 resize ⇒
最多一次重建。改成 `WM_SIZE` 里逐帧跟随之后，拖动中**重建次数暴增**，这个
use-after-free 从「一年碰不到一次」变成「拖一下就中」。

也就是说：这条不是我新写的 bug，但我的改动把它从冷门路径挪到了热路径。

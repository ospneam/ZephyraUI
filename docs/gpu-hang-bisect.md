# GPU 挂掉（`DEVICE_HUNG`）二分手册

> 现象：拖动窗口 / 正常绘制时 `Present` 报
> `DXGI_ERROR_DEVICE_REMOVED (0x887A0005)`，
> 而 `GetDeviceRemovedReason()` 给 **`DXGI_ERROR_DEVICE_HUNG (0x887A0007)`** —— 真 TDR。

---

## 0. 为什么需要这份手册

调试层在本机**两条路都用不了**：

| 开启方式 | 结果 |
|---|---|
| 事后 `ID3D12Debug::EnableDebugLayer()` | 设备**刚建好**就 `DEVICE_HUNG`（本仓 `debug_enabled` 原注释记着的老现象） |
| 事前 `D3D12CreateDevice` + `D3D12_CREATE_DEVICE_DEBUG` | 应用**静默退出**（访问违例；`backend_parity` 两条用例同样崩） |

所以「哪个 API 出的问题」这条证据拿不到，只能靠**逐个关掉子系统**来二分。
下面每个开关都能单独关掉一个子系统，组合起来覆盖了本仓已知的所有
`DEVICE_HUNG` 高发区。

## 1. 开关一览

| 环境变量 | 关掉什么 | 首要嫌疑点 |
|---|---|---|
| `ZEPHYRA_MSAA=0` | 多重采样（采样数 4 → 1） | `Swapchain::resolve_msaa` 的相对顺序；MSAA 纹理在 `resize` 里被**整体重建** |
| `ZEPHYRA_NO_FILTERS=1` | **整条离屏路径**（不建 RT、不写描述符、不发跨状态屏障） | 本仓记过的两处 `DEVICE_HUNG` 都在这里；离屏 RT 按尺寸重建 vs 状态追踪跨帧 |
| `ZEPHYRA_D3D12_DEBUG=1` | —— | **别用**，见 §0 |

启动日志会**自己打出生效配置**（MSAA 那行 + 「离屏滤镜：整条关掉」），
所以每一份日志都自带它跑的是哪套组合。

## 2. 按顺序跑

```powershell
cd E:\ZephyraUI\target\debug

# 基线：确认还能复现
$env:ZEPHYRA_MSAA="0"; $env:ZEPHYRA_NO_FILTERS=""; .\gallery.exe
# 关掉 MSAA
$env:ZEPHYRA_MSAA="0"; .\gallery.exe
# 关掉离屏滤镜
$env:ZEPHYRA_MSAA=""; $env:ZEPHYRA_NO_FILTERS="1"; .\gallery.exe
# 两个都关（若上一个还挂，说明问题在两者之外）
$env:ZEPHYRA_MSAA="0"; $env:ZEPHYRA_NO_FILTERS="1"; .\gallery.exe
```

每次**拖窗口**触发一次，然后记下：挂 / 不挂。

清掉：`$env:ZEPHYRA_MSAA=""; $env:ZEPHYRA_NO_FILTERS=""`。

## 3. 怎么读结果

| 现象 | 结论 | 下一步 |
|---|---|---|
| 关 MSAA 后不挂 | 嫌疑落在 resolve / MSAA 纹理重建 | 查 `Swapchain::resolve_msaa` 与 `barrier_to_present` 的相对顺序，以及 `rebuild_msaa` 之后纹理状态是否被当作 `RENDER_TARGET` |
| 关滤镜后不挂 | 嫌疑落在离屏 RT 的状态追踪 | 查 `offscreen_pair` 改尺寸重建时 `RtState` 与实际状态的对应 |
| 两个都关还挂 | 问题在**两者之外** | 最可能是 DComp + `CreateSwapChainForComposition` 这条组合本身（见 §4） |
| 关任一个都不挂但都会挂 | 两个嫌疑叠加 | 分别修 |

## 4. 如果「两个都关」还挂

那就基本可以指向架构本身，而不是某一行代码。

Chromium 全仓 `CreateSwapChainForComposition` 只有三个调用点，
**每一个传的都是 D3D11 设备**：

| 位置 | 传入的设备 |
|---|---|
| `components/viz/service/display_embedder/software_output_device_win_swapchain.cc:169` | `d3d11_device` |
| `gpu/command_buffer/service/shared_image/d3d_image_backing_factory.cc:550` | `d3d11_device_` |
| `gpu/command_buffer/service/shared_image/dxgi_swap_chain_image_backing.cc:92` | `d3d11_device` |

Chromium 的 **D3D12** 那条路**不用 DXGI 交换链**，走 composition texture：

- `ui/gl/dc_layer_tree.cc:144` —— 只有存在 D3D12 命令队列时才 QI 出
  `EXPERIMENTAL_IDCompositionDevice6`
- `ui/gl/dc_layer_tree.cc:892` —— `PresentCompositionTextures(&command_queue, 1)`
  之后才 `Commit()`
- `ui/gl/direct_composition_support.cc:1249` —— D3D12 被挡在 feature 标志后面

本仓走的是「D3D12 设备 + `CreateSwapChainForComposition` + DComp `Present1`」，
正是 Chromium **不**走的那条。届时要做的选择：

1. **换 D3D11 后端跑 DComp 窗口**（与 Chromium 一致）。本仓有 `zephyra-winit`
   的 D3D11 渲染层，但**没有** `HostWindow` —— 要补一个宿主。
2. **实现 composition texture 路径**（`IDCompositionDevice6` +
   `IDCompositionTexture` + `PresentCompositionTextures`）。

两条都不是小改动，需要单独评估。

## 5. 已排除的（别再回去查）

- **半死状态**：`Swapchain::resize` 失败后 `views` 被清空而 `size` 还是旧值
  ⇒ 每帧重试 + `rtv` 下标越界。已修（`9963ba1`），错误不再刷屏。
- **首次 present 黑闪**：Chromium `swap_chain_presenter.cc:1155` 的那条，
  已抄（`f33558b`）。
- **旧交换链被 DComp 引用着就 Release**：`recreate` 的 use-after-free，
  已修（`d880df2`），顺序改成「建新链 → `SetContent` → `Commit` → 才放旧链」。

## 6. 一条已经写进代码的经验

**getter 之外还要看顺序。** 本仓三轮都把 `DEVICE_HUNG` 归到了某个 API 的
参数或顺序上，每次读码都能自洽——但都错。区别在于：真正定位它需要的是
「哪条命令、哪个资源、哪个状态」，而那个信息**只**存在于调试层里。
调试层不可用时，正确做法是**把子系统关掉做二分**，不是继续读码猜。

所以 `report_device_dead_once` 里现在明确写了：drain 之后若**一条 debug 消息
都没有**，就说明调试层没挂上，此时该做的是二分，不是继续猜。
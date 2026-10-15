# Windows 窗口属性（DWM）

> 出处：`https://learn.microsoft.com/en-us/windows/win32/api/dwmapi/ne-dwmapi-dwmwindowattribute`
> 与 `.../ne-dwmapi-dwm_window_corner_preference`（2026-10 查）。
> 本文只写**文档确证**的事实；文档没写的（典型如 Mica 的纯 Win32 路线）记成
> 「未确证」，不许在代码里凭印象实现。

## 1. `DWMWINDOWATTRIBUTE` 取值（本轮唯一需要的几个）

| 名称 | 值 | 起始版本（文档原文） | 取值类型 |
|---|---|---|---|
| `DWMWA_USE_IMMERSIVE_DARK_MODE` | 20 | Windows 11 Build 22000 | `BOOL` |
| `DWMWA_WINDOW_CORNER_PREFERENCE` | 33 | Windows 11 Build 22000 | `DWM_WINDOW_CORNER_PREFERENCE` |
| `DWMWA_BORDER_COLOR` | 34 | （文档未在本页标注版本） | `COLORREF` |
| `DWMWA_CAPTION_COLOR` | 35 | （同上） | `COLORREF` |
| `DWMWA_TEXT_COLOR` | 36 | （同上） | `COLORREF` |
| `DWMWA_SYSTEMBACKDROP_TYPE` | 38 | **Windows 11 Build 22621** | `DWM_SYSTEMBACKDROP_TYPE` |
| `DWMWA_REDIRECTIONBITMAP_ALPHA` | 39 | Windows 11 Build 26100 | `BOOL` |

枚举定义里 `DWMWA_NCRENDERING_ENABLED` 起是隐式递增，显式赋值的只有 20 与 33，
故 34/35/36/37/38/39 由顺序推出（与上表一致）。

## 2. `DWM_WINDOW_CORNER_PREFERENCE`（dwmapi.h）

| 名称 | 值 | 文档语义 |
|---|---|---|
| `DWMWCP_DEFAULT` | 0 | 由系统决定（本版本为不圆角） |
| `DWMWCP_DONOTROUND` | 1 | 永不圆角 |
| `DWMWCP_ROUND` | 2 | 圆角（较大） |
| `DWMWCP_ROUNDSMALL` | 3 | 小圆角 |

**起始版本**：Windows Build 22000（页面 `req.target-min-winverclnt`）。

## 3. 与 Rin-UI 的 Python 实现的两处出入

Rin-UI `RinUI/core/theme.py` 是我们要照抄的参照物，但它的实现里有两处**与文档
不符**，直接照抄会把已知错误带进来：

1. **它把 `DWMWA_MICA_EFFECT = 1029` 当作正式属性。** 官方 `DWMWINDOWATTRIBUTE`
   枚举里**没有**这一项（Rin-UI 自己的注释也承认它是 undocumented）。本仓
   **不使用**未文档化的属性 —— 迁移版一律走已文档化的 `DWMWA_SYSTEMBACKDROP_TYPE`。
2. **它用 `DwmSetWindowAttribute` 传 `ctypes.c_int` 给 `DWMWA_USE_IMMERSIVE_DARK_MODE`。**
   文档说该属性取值类型是 `BOOL`。`BOOL` 是 4 字节 int，传 `c_int` 在 ABI 上
   等价，但**类型声明必须按文档写 `BOOL`**，否则将来 `BOOL` 语义变化时无从察觉。
3. **版本门槛写错**：Rin-UI 的 `is_win11_22h2()` 才启用 backdrop，而文档明确的
   门槛是 Build 22621（22H2）—— 与它一致；但 `is_win11()`（22000）就启用
   dark mode 也与文档一致（22000）。这条它没错，记录在此以免后来人误改。

## 4. Mica / Acrylic：本仓走不了官方路线（**记账，不假装能做**）

官方文档 `apply-mica-win32` 的前置条件明确写着：

> To apply Mica to a Win32 app for Windows 11, you need to use the **Windows App SDK**.

路线是 `MicaController` + `DispatcherQueue` + `Compositor` + `CreateWindowTarget`
—— 依赖 WinRT 与 Windows App SDK 运行时。本仓是**纯 Rust + winit + DComp + D3D12**，
不引入 WinAppSDK（那会引入一整套 COM/WinRT 初始化依赖）。

结论：**Mica/Acrylic 在本轮不做**，且不是因为"没查到 API"，而是因为**官方路线需要
本仓刻意不引入的运行时**。若将来要做，接入点是 `docs/portability.md` 里记的
`Platform::backdrop()` 能力位 —— 上层已按"平台不给就降级为实色"写好。

## 5. 对本仓的影响（→ 代码位置）

- 新增 crate `zephyra-platform-win`：封装 `DwmSetWindowAttribute`，
  **只暴露文档确证的能力**（dark mode / corner preference / caption & border color）。
- 所有调用**必须**先过版本判定（`RtlGetVersion` 实测 build 号 ≥ 22000），
  低版本静默返回 `Unsupported`，不报错、不崩。
- 宿主在窗口创建后调一次，并在 trace 里打出「请求了什么 / 实际是否生效」。
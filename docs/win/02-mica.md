# Mica / Acrylic —— DWM 属性路线（本仓走的）

> 出处：
> - `https://learn.microsoft.com/en-us/windows/win32/api/dwmapi/ne-dwmapi-dwmwindowattribute`（属性 38）
> - `https://learn.microsoft.com/en-us/windows/win32/api/dwmapi/ne-dwmapi-dwm_systembackdrop_type`（取值枚举）
> - `https://learn.microsoft.com/en-us/windows/win32/api/dwmapi/nf-dwmapi-dwmsetwindowattribute`
> （2026-10 查）

## 0. 先更正一则此前的错误结论

上一轮我写「**Mica 做不了**，官方路线要求 Windows App SDK」—— **那条结论是错的**，
现在更正，并说明它为什么错。

官方文档里**确实**有一条 WinAppSDK 路线（`apply-mica-win32`，用
`MicaController` + `Compositor` + `DispatcherQueue` + `CreateWindowTarget`），
但那**不是唯一路线**。同一份文档的 `DWMWINDOWATTRIBUTE` 页面写着：

> `DWMWA_SYSTEMBACKDROP_TYPE` — Use with `DwmGetWindowAttribute` or
> `DwmSetWindowAttribute`. Retrieves or specifies the **system-drawn backdrop
> material** of a window, including behind the non-client area. The `pvAttribute`
> parameter points to a value of type `DWM_SYSTEMBACKDROP_TYPE`.
> **This value is supported starting with Windows 11 Build 22621.**

即：**纯 Win32 + `DwmSetWindowAttribute` 就能开背景材质**，不需要 WinRT、
不需要 Windows App SDK 运行时。

我为什么会判错：我读了「How to use Mica in Win32 Apps」那篇（它以 WinAppSDK
为前提），没顺着 `DWMWINDOWATTRIBUTE` 枚举查到底。**教训记在这里**：
「某 API 需要某个运行时」这句话必须区分**官方示例**与**API 契约** ——
示例选了一条路，不等于别的路不通。

## 1. 取值（`DWM_SYSTEMBACKDROP_TYPE`，dwmapi.h）

| 名称 | 值 | 文档语义 |
|---|---|---|
| `DWMSBT_AUTO` | 0 | 交给 DWM。只在**默认标题栏后面**应用，且可能**什么都不画**（内部启发式） |
| `DWMSBT_NONE` | 1 | 不画任何背景材质 |
| `DWMSBT_MAINWINDOW` | 2 | 长生命周期窗口，**整窗**范围。Win11 上 = **Mica**（默认变体） |
| `DWMSBT_TRANSIENTWINDOW` | 3 | 临时窗口，**整窗**范围。Win11 上 = **Acrylic**（最亮变体） |
| `DWMSBT_TABBEDWINDOW` | 4 | 带标签页标题栏的窗口。Win11 上 = **Mica Alt** |

文档措辞里两处值得抄下来（它们决定了本仓怎么做）：

- 「**behind the entire window bounds**」—— 非 `AUTO` 的取值作用于**整窗**，
  不只是标题栏。
- 「The material effect **might change with future Windows releases**」——
  这不是 Mica 的别名，是**当前版本上的对应关系**。所以代码里表达的是
  「我想要 Mica 这一类材质」，不是「我要那个像素效果」。

**最低版本**：Windows 11 Build 22621（22H2）。这与属性 38 本身的门槛相同。

## 2. windows crate 绑定核对

`windows` 0.62 **有完整绑定**（与 `RtlGetVersion` 那次不同）：

```rust
pub const DWMWA_SYSTEMBACKDROP_TYPE: DWMWINDOWATTRIBUTE = DWMWINDOWATTRIBUTE(38i32);
pub const DWMSBT_AUTO:           DWM_SYSTEMBACKDROP_TYPE = DWM_SYSTEMBACKDROP_TYPE(0);
pub const DWMSBT_NONE:           DWM_SYSTEMBACKDROP_TYPE = DWM_SYSTEMBACKDROP_TYPE(1);
pub const DWMSBT_MAINWINDOW:     DWM_SYSTEMBACKDROP_TYPE = DWM_SYSTEMBACKDROP_TYPE(2);
pub const DWMSBT_TRANSIENTWINDOW:DWM_SYSTEMBACKDROP_TYPE = DWM_SYSTEMBACKDROP_TYPE(3);
pub const DWMSBT_TABBEDWINDOW:   DWM_SYSTEMBACKDROP_TYPE = DWM_SYSTEMBACKDROP_TYPE(4);
```

取值与文档表**逐一吻合**（0/1/2/3/4）。所以本仓用强类型常量，不自己写裸数字 ——
理由同 `DwmAttr`：写错数字会**静默失效**（DWM 返回 `E_INVALIDARG`，
上层只看到「没生效」）。

## 3. 前提条件（比上一轮多的一条）

材质是**系统画**在窗口**背后**的。所以它只在「本应用没有把那里涂满不透明色」
时可见。本仓的应用是**自绘整窗**的（`Container.window { background: @surface }`），
所以要做两件事，二者缺一不可：

1. **打开材质**（`DWMWA_SYSTEMBACKDROP_TYPE`）；
2. **让根背景半透明**（否则不透明色盖住材质，等于没开）。

第 2 点属于**应用/样式层**，不属于平台层：平台层只回答「能不能开」。
本仓的做法：`gallery.zstyle` 里 `Container.window` 的背景在启用材质时
改成 `@surface_translucent`（半透明实色），材质关时回到 `@surface`。

⚠️ 这条耦合要写在明处：**平台层能力位为 true，不等于画面上一定能看到材质** ——
还取决于应用有没有给根留透明。后者由 `does_the_backdrop_reach_the_screen` 门禁在
**像素层面**核对（见 §4）。

## 4. 验证口径（不能只看 `ApplyOutcome`）

「调用返回成功」**不能**证明材质可见。真正的判据是**像素**：窗口左上角
（标题栏区域）应当显示出与桌面壁纸/主题相关的颜色，而不是应用自己的 `@surface`。

所以验收有两条：
- `ApplyOutcome.backdrop == true`（API 层：调用被接受）；
- 截图取标题栏区域像素 ≠ `@surface`（画面层：材质真的透出来了）。

只有第一条时，结论只能写「调用成功」，**不能**写「Mica 生效」。

## 5. 与 Rin-UI 的关系

Rin-UI 的 `RinConfig["backdrop_effect"]` 与 `ACCENT_STATES = {"acrylic": 3,
"mica": 2, "tabbed": 4, "none": 0}` —— 数值与文档**一致**（注意它的 `none: 0`
其实对应 `DWMSBT_AUTO` 而非 `DWMSBT_NONE=1`，属命名与取值错配，本仓按文档
把 `None` 映射到 `DWMSBT_NONE`）。

它另外还有一条 Win10 路线 `SetWindowCompositionAttribute(WCA_ACCENT_POLICY=19)`。
那是 Win10 的 Acrylic，与 Win11 材质**不是同一个东西**；本仓**不做**（只面向
Win11 及以上，`docs/win/01` §1 的版本门槛已把 Win10 排除在外）。
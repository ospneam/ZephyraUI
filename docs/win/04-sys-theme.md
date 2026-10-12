# 系统深浅色的权威来源与变更通知

> 结论先行：**这一页是「查了、发现没查��到官方文档」的记账页**，
> 不是实现页。批 3 因此**只做只读查询**，不做变更通知。

## 1. 查了什么

| 查法 | 结果 |
|---|---|
| 文档站全文检索 `RtlGetVersion` | 命中 `wdm.h`（驱动）与 `Ntddk.h`（DevNotes）两处；`windows` crate **没有绑定** |
| 文档站全文检索 `GetVersionExW` | 命中 `sysinfoapi` 页，**明确写了 manifest 兼容性 lie** |
| `windows` crate 0.62 源码 grep | `Win32` 与 `Wdk` 两个命名空间**都没有** `RtlGetVersion` |

`GetVersionExW` 文档原文（`nf-sysinfoapi-getversionexw`）：

> The value returned by the GetVersionEx function now **depends on how the application
> is manifested**. Applications not manifested for Windows 8.1 or Windows 10 will
> **return the Windows 8 OS version value (6.2)**.

`RtlGetVersion` 文档原文（`win32/devnotes/rtlgetversion`）：

> `NTSTATUS RtlGetVersion(_Out_ PRTL_OSVERSIONINFOW lpVersionInformation);`
> DLL: `Ntdll.dll`；Header: `Ntddk.h`；Minimum supported client: Windows 2000。

所以本仓的 `version.rs` 走 `RtlGetVersion` 自己声明 extern（有单测钉住：
`major >= 10 && build >= 10000`，写成 6.2 的 `GetVersionExW` 会红）。

## 2. 深浅色：查不到官方文档

深浅色偏好的**权威来源**是注册表：

```
HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize
    AppsUseLightTheme   (REG_DWORD, 0 = 深色)
    SystemUsesLightTheme (REG_DWORD, 0 = 深色)
```

但这两个值**没有官方文档页** —— 它们是 Windows 的**实现细节**，不是契约。
Rin-UI 用 `darkdetect`（Python 库）读同一处。

按本仓「写 Windows 优化先查文档」的规矩，处理方式是：

| 事项 | 处理 |
|---|---|
| 只读查询 | **允许**，但代码里必须注释"未确证"（`zephyra-platform-win/src/lib.rs` 的 `sys_theme`） |
| 读不到 / 键不存在 | 返回 `None`，**不猜**；上层据此「不跟随系统主题」，而不是跟随成浅色 |
| **变更通知**（`WM_SETTINGCHANGE` / `UISettings` / `RegNotifyChangeKeyValue`） | **本轮不做** |

## 3. 为什么变更通知本轮不做

三条理由，第三条是决定性的：

1. 需要常驻消息钩子或 `RegNotifyChangeKeyValue` 回调 —— 都是**长生命周期资源**，
   而宿主当前**没有"后台线程 + 生命周期管理"这层结构**（见 base 层第二条契约）；
2. 通知到达 → 改主题 → 重绘，需要一个**跨帧的状态通道**。现有机制是
   `Arc<AtomicU8>` 的"主题意图共享格"，它够用，但**谁来写**需要一条新的事件源；
3. **收益不足以支撑复杂度**：用户在系统设置里切主题的频率极低，而
   「切回来时主题不跟随」是个**可见但不损坏**的问题；相比之下本轮要修的
   交互渲染缺陷（见 `MIGRATION.md` §7）严重得多。

**做它的时机**：当有人抱怨「切系统主题应用不跟随」时，届时先查
`WM_SETTINGCHANGE` 与 `UISettings` 的文档，再按本仓的分离要求放进
`zephyra-platform-win`（不混进渲染后端）。

## 4. Linux 上对应的权威来源（留给适配）

`docs/portability.md` 会记：XDG portal 的 `org.freedesktop.appearance`
`color-scheme` 属性（值 `prefer-dark` / `prefer-light` / `no-preference`）。
那是**有规范文档的**（freedesktop portal spec），比 Windows 那两个注册表值
靠谱 —— 这也是 Linux 适配时不需要重新调研深浅色的原因。
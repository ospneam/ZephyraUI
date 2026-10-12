# 可移植性 —— 哪些是平台能力，Linux 上各自怎么接

> 这份文档是**接口契约**，不是 TODO 清单。每一项都写清了：
> 当前状态、Linux 上对应的权威来源、以及「上层不��变」这件事如何被保证。

## 1. 抽象层的形状

```
zephyra-core / zephyra-fx      零 Win32 代码（这一层**任何平台都完全相同**）
        │
zephyra-ui / zephyra-style     零 Win32 代码；只问 zephyra-platform 的能力位
        │
zephyra-platform               trait Platform + Capabilities + stub
        ├── zephyra-platform-win     Windows 实现（DWM / 版本 / 注册表）
        └── （将来）zephyra-platform-linux
```

`zephyra-platform` 是 `#![forbid(unsafe_code)]` 且**没有 `cfg(windows)`**——
它本身就是「平台无关」的证据。Windows 代码只在实现 crate 里。

## 2. 能力位与 Linux 对应（`Platform::capabilities`）

| 能力位 | Windows 上 | Linux 上怎么办 | 权威来源 |
|---|---|---|---|
| `dark_mode` | `DWMWA_USE_IMMERSIVE_DARK_MODE`（22000+） | **不适用**（无标题栏 DWM 属性）→ `false` | — |
| `corner_preference` | `DWMWA_WINDOW_CORNER_PREFERENCE` | Wayland 下由合成器决定（通常圆角）；X11 下是 WM 私有约定 → `false` | wlr-protocols / XDG |
| `caption_color` / `border_color` | `DWMWA_CAPTION_COLOR` / `_BORDER_COLOR` | 同上 → `false` | — |
| `backdrop` | `DWMWA_SYSTEMBACKDROP_TYPE`（22621+） | **X11 有真实对应**：`_COMPOSITE_UNSUPPORTED` + `blur` 走 compositor 的 `_KDE_NET_WM_BLUR_BEHIND`；Wayland 无标准 → `false` | KDE 约定 / 合成器文档 |
| `system_theme_query` | 注册表 `AppsUseLightTheme`（**未文档化**） | **有规范**：`org.freedesktop.appearance` 的 `color-scheme`（`prefer-dark` / `prefer-light` / `no-preference`） | freedesktop portal spec |

**关键**：Linux 的深浅色来源**有正式规范**，比 Windows 那两个注册表值可靠。
所以 Linux 适配时**不需要重新调研**，只需实现 `system_theme()`。

## 3. 上层为什么不用改代码

- 主题跟随：`Gallery` 只读 `Platform::system_theme()`；读不到（`None`）⇒
  **不跟随**（而不是跟随成浅色）。这一点已经由
  `the_app_always_asks_for_the_mica_look` 与
  `系统主题查询结果必须与注册表一致或读不到` 两条门禁钉住。
- 材质：`BackdropKind` 枚举在通用层；上层无条件发 `.mica` 分类名，
  平台层回答「能不能」。平台不支持 ⇒ 材质不生效，**界面仍然可读**
  （半透明标题栏叠在不透明 clear 上）。降级路径已在
  `docs/win/02-mica.md` §3 记账。

## 4. 尚未抽象出去的部分（**诚实边界**）

| 部分 | 现状 | Linux 影响 |
|---|---|---|
| 渲染后端 | `zephyra-winit-d3d12`（D3D12）+ `zephyra-winit`（D3D11）都是 Windows-only（`#![cfg(windows)]`） | **最大的一块**：需 Vulkan/GL 后端，或 wgpu |
| `zephyra-dcomp` | DirectComposition | Wayland 走 wl_surface，X11 走 XCB |
| 字体 | `zephyra-fx` 的字形来源（ab_glyph） | 已是纯 Rust，**可跨平台** |
| core / fx / style / ui | 零平台代码 | **直接可用** |

结论：本轮完成的抽象让 **core + UI 控件层**成为平台无关的部分（这是绝大部分逻辑），
剩下的是**渲染后端**——那本来就是 base 层的职责，也是 Linux 适配的主要工作量。

## 5. 验收口径

- `cargo check -p zephyra-ui --no-default-features` **通过**（关掉 d3d12 后应用层仍可编译）
- `cargo check -p zephyra-platform --no-default-features` 通过
- 这两条只证明「不依赖 Windows 编译」，**不**证明「Linux 上能跑起来」——后者需要
  真正的 Vulkan/GL 后端。两者不要混淆。
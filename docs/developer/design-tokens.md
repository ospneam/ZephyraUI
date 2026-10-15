# 设计令牌规范

## 为什么需要这份文件

Viewstage 在引入主题之前的状态（可由 `git show 892f6a2:src/` 复核）：

| 指标 | 数量 |
| --- | --- |
| 硬编码颜色字面量（`Color::from_rgb8` 等） | **111 处**，分布在 7 个文件 |
| `.style()` 闭包（各自重写 hover/active/disabled 状态机） | **75 处** |
| 各自发明的间距值（`padding([6,10])`、`padding([10,16])`…） | 数十处 |
| 主题设置 | 名义存在，实际是假的 —— 只决定哪个按钮高亮，所有 UI 颜色都是硬编码深色 |

后果：改一次品牌色要动几十个文件；深色模式根本不存在。

本文件规定令牌体系，目标是让上面四个数字都归零。

## 三层结构

令牌按**稳定性**分三层。控件只允许引用第 2 层与第 3 层。

```
第 1 层  palette      原始色板（neutral_0..900, accent）
          ↑ 只有主题定义该用哪一档时才引用
第 2 层  语义角色      surface / content / stroke / accent_fill …
          ↑ 控件只引用这一层 ← 这是换主题不碰控件代码的前提
第 3 层  控件槽位      button.background.rest / … （为可重样式控件预留）
```

**为什么控件不能引用第 1 层**：一旦控件写死 `neutral_800`，深色主题想把它换成
`neutral_900` 就必须改控件代码。语义层存在的全部意义就是阻断这一层依赖。

## 语义角色清单

角色名是 `&'static str` 常量（`theme::roles`），不是裸字符串 —— 拼错时编译不过。

| 角色 | 用途 | 缺它会怎样 |
| --- | --- | --- |
| `surface` | 页面/窗口底色 | 无 |
| `surface_alt` | 卡片、输入框内底色 | 卡片与页面同色，层级消失 |
| `content` | 主要文本 | 无 |
| `content_secondary` | 次要文本、标签 | 无 |
| `content_disabled` | 禁用文本 | 禁用态与正常态无法区分 |
| `stroke` | 描边、分隔线 | 无 |
| `control_fill` | 控件静止态 | 无 |
| `control_fill_hover` | 控件悬停态 | 指针位置无反馈 |
| `control_fill_pressed` | 控件按下态 | 按下无反馈 |
| `accent_fill` | 强调填充（主按钮、选中态） | 无 |
| `on_accent_fill` | 强调色之上的文本 | 主按钮文字与背景对比度不可控 |
| `focus_ring` | 焦点环 | 键盘用户看不到焦点位置 |

**每个角色都必须在 Light 与 Dark 下都有值**，且两套的值必须**真的不同**——
这一点由 `tests.rs::light_and_dark_differ_visibly` 钉住。若浅深两套的
`surface` 相同，「切主题」在视觉上就是空操作，而这类退化不会报错。

## 间距

`Spacing` 刻度（WinUI 的 `Spacing` 资源键）：

| 刻度 | DIP |
| --- | --- |
| `None` | 0 |
| `XXSmall` | 2 |
| `XSmall` | 4 |
| `Small` | 8 |
| `Medium` | 12 |
| `Large` | 16 |
| `XLarge` | 20 |
| `XXLarge` | 24 |

4pt 栅格。单调性由 `spacing_scale_is_monotonic` 断言。

控件的内边距应通过 `Padding` 属性取 `Spacing`，而不是写数字。控件之间用
`Margin` + `Spacing::Small`（8）或 `Spacing::XSmall`（4）。

**尚未规定**：控件级具体间距（按钮内边距是 `Small` 还是 `Medium`）。这属于
第 3 层，等控件实现时定。

## 圆角与阴影

圆角用 `CornerRadius`，在 quad shader 内以 SDF 求出（因此不参与几何细分）。
阴影同理，应作为一条独立的 `Quad` 命令（模糊矩形）而不是烘进填充色。

刻度**尚未规定**。当前 Viewstage 实际使用的值（迁移时要复现）：

| 用途 | 半径 |
| --- | --- |
| 按钮 | 6 |
| 菜单项 | 8 |
| 工具栏分组 / 弹窗 | 12 |

## 颜色空间

一律**线性**。`Color::over`（alpha 合成）与 `Color::lerp`（插值）在语义上不同，
不可互换：

- `lerp` 是颜色的直觉插值，用于状态过渡（如 hover 色渐变）；
- `over` 是 alpha 合成，用于半透明层叠。

在**不透明**背景上 `over` 恰好退化为 `lerp`（因为 `out.a == 1`）——这是恒等式
而非巧合，`tests.rs::composite_over_opaque_backdrop_is_exactly_lerp` 钉住了它。
在**半透明**背景上两者结果明显不同，此时必须用 `over`。

直接拿 sRGB 数值当线性用会让两层半透明叠加后的亮度明显偏暗——这正是 sRGB 曲线
存在的原因。宿主在写入非 sRGB 表面时负责编码回 sRGB。

## 从 Viewstage 迁移的映射

迁移 `src/views/*.rs` 时按此表把现有硬编码值换成语义角色：

| 现有字面量 | 语义角色 |
| --- | --- |
| `Color::from_rgb8(26, 26, 26)`（底色、容器） | `surface` |
| `Color::from_rgb8(42, 42, 42)`（按钮 hover/active） | `control_fill_hover` / `control_fill_pressed` |
| `Color::from_rgb8(229, 229, 229)`（文本） | `content` |
| `Color::from_rgba8(255,255,255,25/255)`（描边） | `stroke` |
| `Color::from_rgb8(59, 130, 246)`（选中态边框） | `accent_fill` |

当前内置深色主题的色值直接取自 Viewstage 的实际观感（`#1a1a1a` 底、`#e5e5e5`
文本、`#2a2a2a` 控件填充），因此接入 ZephyraUI 后**视觉不回退**。

## 检查清单

新增控件或页面时：

- [ ] 没有裸 `Color` 字面量（除主题定义内部）
- [ ] 没有裸间距数字（除令牌定义内部）
- [ ] hover / pressed / disabled 三态都从语义角色取，而不是各写各的
- [ ] Light 与 Dark 下都检查过（`ThemeContext::toggle` 切换即可）
- [ ] 半透明层叠用了 `over` 而不是 `lerp`

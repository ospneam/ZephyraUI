# ZephyraUI —— 控件层

一句话：**控件只发结构与分类名，外观全在 `.zstyle`。**

它是一个**库**，不是应用。它不知道颜色、不写死尺寸、不决定一个控件出现在哪一页。
应用（`gallery/`）把控件摆进页面，并决定那一页长什么样。

## 依赖边界

```
zephyra-core   零依赖，禁 unsafe
zephyra-style  动画与贝塞尔缓动
     ↑
ZephyraUI     本 crate：控件 + 控件层样式表（零后端）
     ↑
gallery        应用：页面编排 + 路由 + i18n + 宿主接线
```

**不依赖任何渲染后端。** 不出现 `D3D` / `Vulkan` / `wgpu`，也不做 `#[cfg(windows)]`
分支挑后端 —— 后端由宿主 crate 的特性开关选，一个二进制只会有一个。

## 三条硬约定

### 1. 源码里没有外观

控件函数只写结构（`NodeTag`）、`key` 与**分类名**。不写
`WIDTH` / `HEIGHT` / `BACKGROUND` / `CORNER_RADIUS` / `PADDING` / `SPACING` …

**唯一例外**是 `fill:*` 前缀的 key（进度条填充、滑条 thumb）：它们的宽度是**内容**，
不是外观。见 `tests/gates.rs::only_fill_nodes_carry_local_width`。

门禁按**整棵树 × 全部页面 × 全部交互态**判，比你想的更容易红。

### 2. 文案归应用

控件**不发**可见文字，除了调用方**显式传进来**的那些。控件层里不许出现
「日一二三四五六」这种写死的中文表头（`date_picker` 的星期头是参数，就是这条的产物）。

控件也不做 i18n：`zephyra_core::i18n` 不在这里。

### 3. 分类名必须存在

拼错分类名时**什么都不发生** —— 不报错、不崩、只是退回默认外观。
而「退回默认值」看起来就像「样式表没生效」，是本层最隐蔽的失败模式。

所以每个 `pub fn` 里的 `style("…")` 都得在 `controls-*.zstyle` 里有对应规则，
门禁逐个查。

## 控件清单

按族分，见 `src/controls/`：

| 文件 | 控件 |
|---|---|
| `button.rs` | Button / ToolButton / PillButton / RoundButton / Hyperlink |
| `card.rs` | Card / SettingCard / ClipCard / Expander |
| `collections.rs` | ListView / TableView / SwipeView / Avatar |
| `dialogs.rs` | Dialog / AcrylicCard / DatePicker / TimePicker / RoundCornerOverlay |
| `forms.rs` | TextField / TextArea / ComboBox / SpinBox / Slider / ColorPicker / AutoSuggestBox |
| `menu.rs` | MenuBar / Menu / MenuItem / DropDownButton |
| `nav.rs` | NavItem |
| `navigation.rs` | NavSubItem / SelectorBar |
| `segmented.rs` | Segmented |
| `status.rs` | Badge / DotBadge / InfoBar / Toast / ProgressBar / ProgressRing / BusyIndicator / PageIndicator |
| `toggle.rs` | CheckBox / RadioButton / ToggleSwitch |
| `text.rs` | 文本层级（`text_label` / `caption`） |

## 样式表

15 份 `controls-*.zstyle`，由 `src/styles.rs` 用 `include_str!` 内嵌。

**内嵌而不是运行时读磁盘**：让「改了表忘了重编」变成**编译错误**而不是运行时的
静默失效。门禁吃的是**同一份** `styles::all()`，所以不存在「图对、门禁绿」。

### zstylec 的三条硬限制

属性表只有 **32 项**，且：

| 限制 | 后果 |
|---|---|
| 没有 `min-height` / `margin-*` | 留白只能靠 `height` + `padding`，或 `translate-y` |
| `padding` 只吃单值 | `padding: 10px 16px` 解析失败 |
| 不支持 `rgba()` | 用 `#rrggbbaa` 八位十六进制 |

## 测试

```bash
cargo test -p zephyra-ui
```

24 条门禁，按**整棵树**判（遍历全部页面 × 全部交互态），不按函数判 ——
逐函数查只能查到「我记得去查的那个」。
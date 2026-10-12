# ZephyraUI — core 之上的 UI 控件层 + RinUI 风格 Gallery

> 本目录是 RinUI 示例应用（gallery）的 Zephyra 重写版。**不是移植，是照着它的
> 功能 / 样式 / 设计用 Zephyra 的方式重写一遍** —— 数据逐条取自
> `E:\Rin-UI\examples\assets\ItemData.qml`（44 条），页面结构照
> `examples/pages/**`，度量照原版 QML。
>
> **施工计划与逐项完成情况见 [`REWRITE.md`](./REWRITE.md)（12 批）。**
> **Windows 相关依据见 [`docs/win/`](</../docs/win/>)，可移植性见
> [`docs/portability.md`](</../docs/portability.md>)。**

---

## 0. 为什么有这一层

`zephyra-core` 的 `lib.rs` 自己写着：

> 控件库（Button / ToggleSwitch / Slider…）与模板系统属于**上层**，建立在
> [`property`] 与 [`theme`] 之上，尚未实现。

本目录就是那一层。约束全部沿用仓库已确立的规矩：

| 规矩 | 出处 |
|---|---|
| 源码里**不许有样式**：控件只发「分类名」，不发颜色/尺寸/圆角 | `examples/demo.rs` 的 6 条门禁 |
| 外观来自 `.zstyle`，经 `@令牌` 走主题，换主题不碰控件代码 | `controls.zstyle` / `demo.zstyle` |
| 状态也是分类名（`.hovered` / `.pressed`），不是属性值 | commit `0f7a840` |
| 每帧 `diff → apply → layout`；`apply` 在 core `frame.rs` 里 | `zephyra-style/src/apply.rs` 的硬契约 |
| 零告警、门禁先行的施工方式 | 全仓约定 |

---

## 1. 范围（这次做什么 / 不做什么）

**做**：一个可运行的 RinUI-gallery 式应用（`bin/gallery`），外加它背后的控件层
（`src/controls`）。页面结构对齐 Rin-UI 的 `examples/gallery.qml`：左侧导航栏 +
标题栏 + 内容卡片，多页可切。

**不做**（如实记账，都是 core 能力边界的诚实裁剪，不是偷懒）：

- **滚动**：core 布局是单趟 flex（`Direction` 只有横/竖），没有 Flickable/ScrollView。
  页面按固定画布 1400×800 设计，一页放得下。
- **网格布局**：core 没有 Grid。RinUI Home 的「Recently added」网格用
  **定行长的横排**手排（每行 N 张，行长由页面常量决定）。
- **文本输入**：core 只有 `props::TEXT`（展示），没有输入法与光标。标题栏的
  搜索框是**视觉件**（placeholder + 边框），不真的收字符。
- **动画**：保留式架构 + 无时间轴的样式层，这一版不做过渡动画（RinUI 的
  `Behavior on …` 全部缺席，状态切换是即时的）。
- **浮层/弹出**（Flyout / Menu / ToolTip / Dialog）：需要 core 的 overlay 层，未实现。

---

## 2. 目录结构

```
E:\ZephyraUI\ZephyraUI\
├── README.md              本文件（设计 + 施工计划 + 分批状态）
├── Cargo.toml             package `zephyra-ui`（lib + bin `gallery`）
├── ui.zstyle              控件层外观（按钮/开关/滑条/徽章/卡片…）
├── gallery.zstyle         应用层版式（外壳/导航/各页面）
├── src\
│   ├── lib.rs             模块汇总 + 预导入
│   ├── tokens.rs          @令牌名常量（与 core theme 新角色一一对应）
│   ├── controls\
│   │   ├── mod.rs
│   │   ├── button.rs      Button(standard/accent/subtle) · ToolButton · PillButton
│   │   ├── toggle.rs      ToggleButton · CheckBox · RadioButton
│   │   ├── slider.rs      Slider
│   │   ├── progress.rs    ProgressBar · ProgressRing
│   │   ├── badge.rs       InfoBadge
│   │   ├── card.rs        Card(Frame) · SettingCard
│   │   ├── segmented.rs   Segmented
│   │   ├── nav.rs         NavigationItem
│   │   └── text.rs        TextLabel / Caption / 排版档位
│   └── app\
│       ├── mod.rs         Gallery（View 实现：路由/主题/交互状态）
│       ├── shell.rs       窗口外壳：标题栏 + 导航栏 + 内容卡片
│       └── pages.rs       Home / AllSamples / BasicInput / Collections / …
├── bin\gallery.rs         可运行宿主（winit + D3D12）+ `--dump-cpu` 无窗口自验
└── tests\gates.rs         树级门禁（见 §7）
```

坐标与尺寸全部**抄 RinUI**（`themes/dark.qml`、`appearance`）：

| 项 | RinUI 值 |
|---|---|
| 按钮圆角 | 5 |
| 小圆角（卡片/帧） | 3 |
| 窗口标题栏高 | 48 |
| 窗口圆角 | 7 |
| 标题栏按钮 | 46×48 |
| ToolButton 图标 | 20 |
| CheckBox 指示器 | 20×20 |
| Slider 手柄 | 20，轨道高 4 |
| ProgressBar 高 | 3 |
| InfoBadge 高 | 16（dot 4×4） |
| navItem 高 | 37（间距 2） |
| 按钮 padding | 6/5/7（水平 6） |
| 控件间距刻度 | 4 / 8 / 12 / 16 |

---

## 3. 令牌层：给 core theme 补 Fluent 角色

`zephyra_core::theme::Theme::role()` 只认 12 个角色（surface/…/focus_ring）。
RinUI 的卡片、次级填充、严重级色没有对应物。**加角色，不改语义**：
在 core `theme.rs` 里追加字段 + `role()` 分支 + dark/light 两套值。
`@令牌` 解析只走 `resource_value` 一条路（`resolve_resource` 保留键的设计不动）。

新增角色（名 → RinUI 对应 → dark 值 → light 值）：

| 令牌 | RinUI | dark | light |
|---|---|---|---|
| `card` | cardColor | #ffffff @5.12% | #ffffff @60%（按 light.qml） |
| `card_secondary` | cardSecondaryColor | #ffffff @3.26% | … |
| `card_border` | cardBorderColor | #000000 @10% | … |
| `control_secondary` | controlSecondaryColor | #ffffff @8.37% | … |
| `control_tertiary` | controlTertiaryColor | #ffffff @4.19% | … |
| `control_strong` | controlStrongColor | #ffffff @54.42% | … |
| `control_border` | controlBorderColor | #000000 @9% | … |
| `control_border_strong` | controlBorderStrongColor | #ffffff @60.47% | … |
| `divider` | dividerBorderColor | #ffffff @8.37% | … |
| `focus_inner` | focusBorderInner | #000000 @70% | … |
| `layer` | layerColor | #3A3A3A @30% | … |
| `success` / `caution` / `critical` | system*Color | #6ccb5f / #fce100 / #ff99a4 | … |

`src/tokens.rs` 是这些名字的 `&str` 常量（防拼错），并有一条门禁：
**样式表里出现的每个 `@令牌` 都必须能在 `Theme::role()` 里查到**——
拼错令牌时 `resolve_resource` 返回 `None`、属性静默保持原值，
正是「样式全没了」类故障最安静的成因。

---

## 4. 控件层的 API 形状

每个控件是一个**函数**（不是 struct）：语义参数进，`Element` 出。例：

```rust
pub fn button(key: &'static str, label: &str, hovered: bool, pressed: bool) -> Element
```

- 事件身份 = `ElementKey`（路由用），外观分类 = `style()`（选择器用）——两条线不混。
- 状态（hovered/pressed）由控件自己从 `Option<&ElementKey>` 比对得出，
  转成 `.hovered` / `.pressed` 分类名。**与 `examples/demo.rs::button` 同一形状。**
- 控件不写任何尺寸/颜色：`width/height/corner-radius/background/…` 全在
  `ui.zstyle` 的 `Control.*` / `Container.*` 规则里。控件只保证
  `key` 稳定、分类名存在、`props::INTERACTIVE` 置位（可交互才有 hover/press）。

点击（导航/开关/主题切换）不用新 host 能力：`View` 自己从
`InputState.pressed` 的「上一帧有、这一帧无且 key 相同」还原出 click
（这正是 `RoutedEvent::Click` 的配对语义，放在 View 侧不丢信息）。

图标：core 内置 12 个矢量图标（menu/pen/eraser/undo/image/camera/gear/settings/
light/minimize/dismiss/move）。RinUI 的 `ic_fluent_*`  glyph 字体没有对应物，
一律映射到内置图标集（`src/controls/icon.rs` 承担映射）。

---

## 5. 应用层（RinUI gallery 的重写）

窗口 1400×800，深色 `#202020` 底，内容卡 `layer` 色 + 7px 圆角。

```
┌──────────────────────────────────────────────────────┐
│ titlebar: [search]            [—][▢][✕]  46×48 caps │
├──────────┬───────────────────────────────────────────┤
│ nav      │  content card（layer 底，radius 7）        │
│ 240px    │   FluentPage：标题 + InfoBadge + 工具行     │
│ (48 折)  │   + 控件展台                                │
└──────────┴───────────────────────────────────────────┘
```

导航项（对齐 gallery.qml 的 navigationItems，裁剪到本层能表现的程度）：

`Home`（顶）、`All samples`、`Basic Input`、`Collections`、
`Status & Info`、`Text`、`Settings`（底）。

页面：

- **Home**：banner（渐变横幅 + 两行白字标题）、3 张链接卡（acrylic 卡片行）、
  PillButton 分段（Recent / Favorite）、「Recently added」控件卡网格（定行长）。
- **All samples**：全部控件缩略网格（定行长 5）。
- **Basic Input**：Button 四种变体、ToolButton 八个、CheckBox/RadioButton、
  ToggleSwitch、Slider、ComboBox 视觉件。
- **Collections**：Card / SettingCard / Expander 视觉件。
- **Status & Info**：ProgressBar（三态）、ProgressRing、InfoBadge（4 严重级）。
- **Text**：排版档位样张（title / bodyStrong / body / caption）。
- **Settings**：SettingCard 列表（ToggleSwitch + Slider + 主题切换）。
  主题切换是**真的切换**（dark/light → `Theme::builtin`），验证
  「换主题只换令牌值，不碰控件代码」。

---

## 6. 运行时与自验

`bin/gallery.rs` 三条路（照 `examples/demo.rs` 的成熟形态）：

1. **窗口路**（默认）：winit + `zephyra-winit-d3d12::HostWindow`，Bootstrap 里
   注册纹理 → `load_stylesheet` → 首帧。启动向 stderr 打三行：
   ```
   [gallery] ui.zstyle: N rules
   [gallery] gallery.zstyle: M rules
   [gallery] loaded N+M rules in total
   ```
   少一行/总数不对 = 样式表没加载上（与 demo 同一诊断，勿改语义）。
2. **`--dump-cpu`**：`zephyra_fx::cpu_only::CpuHost` 画第一帧，写 BMP。
   **不开窗口、不碰 GPU**，同一份 `DisplayList` + `quad::expand`。
   这是本任务的**像素级自验**手段：BMP 直接读图核对。
3. **`--theme light|dark`**：起始主题。

> `CpuHost` 原本没有 `set_styler`，样式表在 dump 路径上会被跳过——
> 那会让「CPU 图」与「窗口画面」悄悄不同（dump 全默认值、窗口有样式），
> 自验就失去意义。所以给 `zephyra-fx::cpu_only::CpuHost` 补一个
> `set_styler`（转发 `FrameCache::set_styler`），仅此一个加法。

---

## 7. 门禁（tests/gates.rs，全部按整棵树判）

1. `the_whole_tree_writes_no_appearance_in_source`
   —— 遍历全部页面 × 全部交互态，任何外观键出现在 Local 层就红。
   （判据 `iter_local()`，与 demo 同。）
2. `every_class_name_the_source_emits_exists_in_the_stylesheet`
   —— 源码发的分类名，样式表里必须有规则（拼错 = 静默失效，最隐蔽）。
3. `every_element_is_reachable_by_the_stylesheet`
   —— 反方向：每个元素要么有分类名、要么其类型有裸规则。
4. `every_token_the_stylesheet_uses_exists_in_theme`
   —— `@令牌` 拼错时 `resolve_resource` 返回 None、静默退回旧值，
   这是「样式全没了」最安静的成因；按张量表逐个对 `Theme::role()`。
5. `every_page_builds_a_real_tree`
   —— 每个路由目标页都能造出 ≥20 节点的树（防恒真）。
6. `stylesheet_parses_clean`
   —— 两份 `.zstyle` 零解析错误（错误即整份被跳过）。

---

## 8. 动工前必须先修的坑（否则新应用照样中招）

排查 `examples/demo` 时实测到（本机，`cargo run` 窗口路）：

```
[demo] controls.zstyle: 2 rules / demo.zstyle: 28 rules / loaded 30 rules in total
[zephyra-d3d12] 记录绘制命令失败: 上传暂存区不够：本段 240 字节，
   从 131072 起放不下（暂存区 131072 字节）。
```

三行样式表**正常**（不是样式没加载上）。真正故障在 D3D12 后端：

`upload_geometry`（主路径）把顶点缓冲的写游标 `head` 推到刚满；随后
`draw_pass_quad`（模糊/投影的离屏 pass，demo 恰好有 3 个）再往**同一条**
`StreamBuffer` 传 240 字节。它的容量判据是 `bytes > capacity`（没算已花掉的
`head`），于是 `record_upload` 在 `off=131072, size=131072` 处越界，
`?` 一路上抛 → **整帧 record 失败** → 画面丢弃。

新 gallery 的圆角卡片/投影只会更重，必修：

- `StreamBuffer` 加 `reserve(extra)`：按 **`head + extra`** 判增长，
  增长到 `(head + extra).next_power_of_two()`，两个调用点
  （`upload_geometry` / `draw_pass_quad`）都改用它。
- 旧缓冲不就地丢：加 `retired: Vec<StreamBuffer>` 留到帧尾，
  `record()` 帧首清空（`begin_frame` 已 `wait_gpu`，GPU 空闲才安全）。
  否则「记录中的拷贝」指向被替换掉资源的风险无法排除。
- 顺带修 `frame.rs:277` 的 `unused variable: node` 告警（本仓零告警标准）。

---

## 9. 分批施工顺序（每批完成即更新本节状态）

- [x] **批 0**：本 MD（设计定稿 + 施工计划）。
- [x] **批 1**：core theme 加 14 个 Fluent 角色；`CpuHost::set_styler`；
      D3D12 暂存区修复（§8）；`frame.rs` 告警。
- [x] **批 2**：crate 骨架 + `tokens.rs` + `controls/*` + `ui.zstyle`。
- [x] **批 3**：`app/*`（shell + pages）+ `gallery.zstyle` + `bin/gallery`。
- [x] **批 4**：`tests/gates.rs` 八条门禁 + `cargo test` 全绿。
- [x] **批 5**：`--dump-cpu` 出图自验 + 修掉两个**全框架性**文字 bug + 全量测试。

## 10. 施工中实测抓到并修掉的两个文字 bug（全框架性，非 gallery 独有）

两处都在 `zephyra-fx`/`zephyra-core` 的文本链上，`examples/demo` 同样中招
（只是从没做过像素级核对）。两条回归测试钉死，`cargo test` 全绿。

1. **字号被整体缩成 0.75 倍**
   `PxScale::from(em)` 在 ab_glyph 里的语义是**行高**（hhea 的
   `ascent − descent`），不是 em。Segoe UI 的 hhea 行高 = 1.33 em，于是所有
   文字一律渲成 0.75 倍，而布局（步进/行高）仍按 em 算 —— 字变小、行距松、
   不同字号对不齐。实测 34px 的 'Z' 位图只有 15×18（GDI 基准 19×24）。
   修法：`AbGlyphSource::px_scale` 做 em → 行高换算（`em × hhea/upem`），
   两个调用点（advance / rasterize）都走它。
   回归：`px_scale_maps_em_to_font_line_height` +
   `capital_ink_is_cap_height_not_three_quarters`。

2. **同一行字的基线互相错开、descender 消失**
   `GlyphQuad::dst.y` 写成 `baseline − offset.y − height`，把 offset 当成
   y-up 正值；而 ab_glyph 的 `px_bounds` 是 **y-down**（基线之上为负）。
   后果不是偏移一两个像素，而是**每个字形的顶部贴到基线**上 —— 34px 行里
   'e' 顶 71 / 'h' 顶 72、底差一个字高。修法：`dst.y = baseline + offset.y`。
   回归：core 侧 `glyphs_on_one_line_share_a_baseline`（VaryingOffset 替身，
   FixedWidth 的 offset 恒 0 测不出这条）+ fx 侧
   `raster_offset_is_screen_space_y_down`。

## 11. 顺带修掉的三个静默缺陷

- `write_bmp` 曾按死 BGRA 解，而 CPU 画布是 RGBA —— dump 的图 R/B 互换。
  深色 UI 几乎全灰所以肉眼看不出来，直到一面饱和色横幅（#16305c 变棕色）
  才暴露。修法：`PixelRead::order`（Rgba/Bgra）进类型，两边调用方显式给。
- D3D12 暂存区：同帧两条上传路径，第二条的容量判据没算已花掉的 `head`
  （§8）。离屏 pass 一多就整帧 record 失败。
- `zephyra-style` 的门禁 `style_and_key_are_independent_namespaces`
  忘了挂 `#[test]` —— 一条写着但从不运行的门禁（恒真）。

## 12. 控件迁移（RinUI → ZephyraUI）已完成

`MIGRATION.md` 是这一段的施工计划与**最终对照表**：RinUI 78 个 QML 里落地 45 个
控件函数，3 个明确不做（Flyout/Popup 浮层、媒体播放 —— 浮层是**框架级**能力），
其余 34 个带逐条降级记账（无浮层 / 无滚动 / 无输入法 / 无绝对定位 / 无时间轴）。
要点：

- **可复用**：控件函数只收语义参数与 `&'static` 切片，只发分类名与 key，
  外观全在 `ui.zstyle`、颜色全走 `@令牌`。应用改主题/换色板不动控件代码。
- **每帧不泄漏**：`view()` 每帧重建元素树，所以 key 与文案**不得**用
  `format!` + `Box::leak` 生成 —— 迁移中改掉了三处（详见 `MIGRATION.md` §6）。
- **逐路由像素自验**：`--dump-cpu --route <id>` 可单独渲任一页
  （默认只有 home，其余页面的样式错误要等窗口路才暴露）。

## 13. 验收状态

- `cargo test --workspace`：**662 用例全绿**；`cargo check --workspace --all-targets`
  **零告警**（含 `examples/`）。
- **70 条路由**（10 页面 + Settings + 14 分组索引 + 44 控件展台 + testbed），
  `--dump-cpu --route` **逐一出图，md5 全部互不相同** ——
  这一条不是形式：它抓到过「`--route` 是白名单 match，未知 id 静默回 home，
  22 张图全是同一张」的缺陷（`REWRITE.md` 批 8）。
  `--list-routes` 可列出全部合法 id；写错 id 现在**退出码 2**，不再静默出错图。
- 门禁**十八条**全绿。除了原有 12 条，本轮为**交互**与**数据一致性**新增 6 条：
  - `interaction_leaves_no_stale_appearance` —— 保留式树 vs 同状态冷启动树，
    逐节点逐属性比对（单帧图永远看不出残留）。
  - `hit_test_agrees_with_the_painted_rects` —— 命中偏了画面完全正常。
  - `catalog_matches_the_rinui_original` —— 44 条与原版逐条一致（脚本提取）。
  - `catalog_search_filters_and_empty_query_returns_all` —— 含"搜不到必须零命中"。
  - `every_catalog_entry_has_a_non_empty_showcase_page` —— 挡"空壳"展台页。
  - `the_mica_titlebar_token_is_actually_translucent` —— 半透明令牌真的半透明。
- **窗口路真机验收**：真实鼠标点击 + 截图逐状态核对（切页 / 复选框往返 / 开关 /
  Slider / 圆角按钮），并与 `click` 路由日志一一对照。
- **Windows 优化**：暗色标题栏 + 大圆角 + **Mica 背景材质**均已真机生效
  （trace `dark=true corner=true backdrop=true`，且标题栏像素与材质 tint 可区分 ——
  API 返回成功**不能**证明材质可见，见 `docs/win/02-mica.md` §4）。
- 原 `examples/demo.rs` 的 **7 项测试能力一条不少**地并入：
  testbed 页（矢量图标 / uv 裁剪 / 滤镜混合 / 状态行）+ `--bench` +
  `--alloc-probe` + **10 条交互回归**（`tests/interaction_tests.rs`）。
- 交互期另修六个缺陷（换页旧树残留 / 样式层残留 / 本地层残留 / clear 与 scissor
  错配 / `revision` 不 bump / 窗口尺寸语义），逐条记在 `MIGRATION.md` §7。
- 单帧期另修七个（含最重的「CPU 参考光栅器从不画矢量图标」，它让 `--dump-cpu`
  对图标完全失明），逐条记在 `MIGRATION.md` §6。

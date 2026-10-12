# RinUI 基础组件全量迁移 —— 施工计划

> 参照 `E:\Rin-UI`（Fluent Design-like，Qt Quick / PySide6）的组件体系，
> 把它的**基础组件**迁进 `ZephyraUI/`（core 之上的 UI 控件层）。
> 目标：**可复用于所有使用 ZephyraUI 的应用** —— 控件不认识任何具体应用，
> 只认 `@令牌`、分类名与事件 key。

## 0. 复用性的三条硬约束（沿用既有规矩，新增组件一律遵守）

1. **源码零外观**：控件函数只写 tag / key / INTERACTIVE / 内容属性 / 分类名。
   颜色、圆角、间距、边框、字号全部在 `ui.zstyle`，且颜色只走 `@令牌`。
2. **状态零数值**：hover / pressed / checked / selected 是分类名，特异性层叠；
   禁止 `if pressed { 2.0 } else { 1.0 }` 这类源码挑值。
3. **数据几何有形状**：唯一例外是"值驱动的几何"（滑条填充、进度填充、
   缩略图尺寸），必须放在 key 为 `fill:*` / `data:*` 的节点上，由门禁
   `only_fill_nodes_carry_local_width` 钉死形态。

新组件落地必须同时满足：
- `controls/<name>.rs` 一个函数族，签名只收语义参数；
- `ui.zstyle` 有对应规则，且只引 `tokens.rs` 里的令牌；
- `tests/gates.rs` 自动覆盖（全树遍历，不按函数判）；
- gallery 有展台（`app/pages.rs` 加一格），`--dump-cpu` 出图可验。

## 1. 组件清单（RinUI → ZephyraUI）

来源：`RinUI/components/**` 共 78 个 QML。按"基础组件"裁剪 + 按 core 能力
分级。**能做**= core 现有能力可完整表达；** Demo 级**= 视觉与静态可迁、
交互部分降级（缺浮层/滚动/文本输入时如实记账）；**不做**= 与 core 架构
冲突（需要 overlay / 动画 / 富文本），留待 core 长出能力。

### 1.1 BasicInput（批 A）

| RinUI | 计划 | 说明 |
|---|---|---|
| Button | ✅ 已迁 | standard / accent / subtle |
| ToolButton | ✅ 已迁 | 40×40 |
| PillButton | ✅ 已迁 | 圆角 99 |
| RoundButton | 批 A | 圆形按钮 = PillButton 的 radius 99 + 正圆尺寸 |
| Hyperlink | 批 A | 文字 + 下划线 + accent 色 |
| DropDownButton | 批 A | 按钮 + chevron-down 后缀图标 |
| CheckBox / RadioButton | ✅ 已迁 | |
| ToggleSwitch | ✅ 已迁 | |
| Slider | ✅ 已迁 | 值驱动填充宽 |
| ComboBox | 批 A | 选中项 + chevron；**展开浮层不做**（core 无 overlay），点击循环切换选项 |
| SpinBox | 批 A | TextField + 上/下按钮 |
| ColorPicker / DropDownColorPicker | 批 A（Demo 级） | 色板网格（16 格），选中态描边；取色交互不做 |

### 1.2 Text（批 B）

| RinUI | 计划 | 说明 |
|---|---|---|
| Text | ✅ 已迁 | text_label 五档 |
| TextField | 批 B | 输入框视觉（底/边/placeholder/后缀图标）；**真实键入不做**（core 无输入法），值由宿主状态给 |
| TextArea | 批 B | 多行版（TEXT_MAX_WIDTH 断行） |
| ScrollableTextArea | 批 B | TextArea + 右侧滚动条视觉件（不滚） |
| AutoSuggestBox | 批 B | TextField + 建议列表视觉件（列表是普通子节点，不浮层） |
| TextInputMenu | 批 B | 右键菜单视觉件（附加在 TextField 旁的浮层**不做**，改为选中文本时才显示的 inline 条） |

### 1.3 StatusAndInfo（批 C）

| RinUI | 计划 | 说明 |
|---|---|---|
| InfoBadge | ✅ 已迁 | |
| ProgressBar | ✅ 已迁 | running/paused/error |
| ProgressRing | 批 C | 环形 = 24 段小竖条围一圈？core 无圆环几何 → 用**点阵环**（12 个圆点按角度排）表达不确定态 |
| InfoBar | 批 C | 四严重级横幅卡（图标 + 标题 + 正文 + 关闭钮） |
| ToolTip | 批 C | Demo 级：常驻展示一件（core 无 hover 延迟/浮层） |
| Toast | 批 C | 底部滑入卡（静态） |
| BusyIndicator | 批 C | 三点跳动（静态三档） |
| PageIndicator | 批 C | 圆点行 + 当前项实心 |
| ToolSeparator / MenuSeparator | 批 C | 竖/横分隔线 |

### 1.4 MenusAndToolbars（批 D）

| RinUI | 计划 | 说明 |
|---|---|---|
| MenuBar | 批 D | 顶栏 + 若干 MenuItem |
| Menu / MenuItem / MenuItemGroup | 批 D | Menu 作为**普通容器**（弗整体），展开/收起用 Expander 的折叠机制 |
| TextInputMenu | 见 1.2 | |

### 1.5 Navigation（批 E）

| RinUI | 计划 | 说明 |
|---|---|---|
| NavigationItem / NavigationView | ✅ 已迁 | |
| NavigationSubItem | 批 E | 缩进一档的子项 |
| Segmented / SegmentedItem | ✅ 已迁 | |
| SelectorBar / SelectorBarItem | 批 E | 底部药丸 tab（Segmented 变体） |
| ErrorPage | 批 E | 图标 + 标题 + 描述 + 重试按钮 |

### 1.6 ListAndCollections（批 F）

| RinUI | 计划 | 说明 |
|---|---|---|
| Frame / Card | ✅ 已迁 | |
| SettingCard | ✅ 已迁 | |
| SettingExpander / SettingItem | 批 F | 卡头 + 折叠体 / 纯展示行 |
| ListView / ListViewDelegate | 批 F | 纵向栈 + 行委托（选中/hover） |
| TableView / HeaderView / Delegate | 批 F | 表头行 + 数据行（定列宽） |
| SwipeView | 批 F | 页面切换视觉件（受控 currentIndex） |
| Avatar | 批 F | 圆图 + 首字母兜底（内置 12 图标无人物像 → 用 initials） |
| Clip | ✅ 已迁（clip_card） | |

### 1.7 Layout / Styles / Dialogs / DateAndTime / Media（批 G）

| RinUI | 计划 | 说明 |
|---|---|---|
| Expander | ✅ 已迁 | |
| RoundedCornerOverlay | 批 G | 四角遮罩视觉件（纯容器 + 圆角，注释说明与 core 圆角的差别） |
| AcrylicBrush | 批 G | Demo 级：半透明卡片（无真实 acrylic 模糊材质） |
| Dialog / DialogButtonBox | 批 G | Demo 级：居中卡 + 三按钮行（无模态遮罩/浮层） |
| Flyout / Popup | **不做** | core 无 overlay 层 |
| Calendar / DatePicker / TimePicker / CalendarDatePicker / PickerView | 批 G（Demo 级） | 月历网格（7×5 静态当月）+ 时间列选择视觉件 |
| AnimatedVisualPlayer / MediaPlayerElement | **不做** | 媒体播放，超出基础组件 |

### 1.8 统一小件（随时插队）

ToolTip 的 info 图标、BusyIndicator 的点、各种 chevron / separator ——
都属于上面各批，不单列。

## 2. 每批的固定动作（Definition of Done）

1. `controls/<x>.rs`：函数 + 文档（度量来源注释到 RinUI 具体 qml 行）。
2. `ui.zstyle`：规则 + 注释（只引 `@令牌`；字面量仅限"样张色"且注明理由）。
3. `tokens.rs`：如缺令牌，**先在 core theme 加角色**（加法），再镜像。
4. `app/pages.rs`：gallery 加/扩一个展台 section（用真实控件，不造假）。
5. `tests/gates.rs`：不用改 —— 全树遍历自动覆盖新分类名 / 可达性 /
   源码无外观 / 令牌存在。**新组件如果gate 红，就是组件写错了。**
6. 需要新 token 或交互时补专项测试（如 `track_width_matches_stylesheet`）。
7. `--dump-cpu` 出图人眼 + 数字核对（不换基线、不挤行、不缺件）。

## 3. 施工顺序与状态

- [x] 批 0：当前任务收尾（UI 层 + gallery + 两个文字 bug 修复 + 485 测试全绿）
- [x] 批 A：BasicInput 剩余（RoundButton / Hyperlink / DropDownButton /
      ComboBox / SpinBox / ColorPicker）
- [x] 批 B：Text 族（TextField / TextArea / ScrollableTextArea /
      AutoSuggestBox / TextInputMenu）
- [x] 批 C：StatusAndInfo 剩余（ProgressRing / InfoBar / ToolTip / Toast /
      BusyIndicator / PageIndicator / Separator）
- [x] 批 D：MenusAndToolbars（MenuBar / Menu / MenuItem / MenuItemGroup）
- [x] 批 E：Navigation 剩余（NavigationSubItem / SelectorBar / ErrorPage）
- [x] 批 F：ListAndCollections 剩余（ListView / TableView / SwipeView /
      Avatar / SettingExpander / SettingItem）
- [x] 批 G：Layout / Styles / Dialogs / DateAndTime（RoundedCornerOverlay /
      AcrylicBrush / Dialog / DatePicker / TimePicker）
- [x] 批 H：对照表补全（§5）+ 全量回归 621 用例零告警 + 逐路由 dump 验收

## 4. 度量来源（RinUI 侧，逐条可查）

- 圆角/边宽/标题栏/窗口：`RinUI/themes/dark.qml` 的 `appearance`
- 控件尺寸：各自 QML 的 `implicitWidth/Height` 与 `indicator`/`handle` 块
- 色板：`themes/dark.qml` 的 `colors`（已映射到 `tokens.rs`）
- 排版：`RinUI/utils/Typography.qml` 的九档枚举

## 5. 落地对照表（RinUI → ZephyraUI，逐条给函数与降级记账）

`✅` = 视觉与交互都迁到；`◐` = 迁了骨架、某一部分按 core 能力降级（后面写清降什么）。

| RinUI 组件 | ZephyraUI 函数 | 档 | 降级 / 备注 |
|---|---|---|---|
| Button | `button` | ✅ | standard / accent / subtle |
| ToolButton | `tool_button` | ✅ | 40×40 |
| PillButton | `pill_button` | ✅ | radius 99 |
| RoundButton | `round_button` | ✅ | 正圆 + 图标/文字 |
| Hyperlink | `hyperlink` | ◐ | URL 走数据键 `HYPERLINK_URL`，打开由宿主路由（core 无外链） |
| DropDownButton | `drop_down_button` | ✅ | chevron 后缀 + `open` 分类名 |
| CheckBox | `check_box` | ✅ | |
| RadioButton | `radio_button` | ✅ | |
| ToggleSwitch | `toggle_switch` | ✅ | |
| Slider | `slider` | ◐ | 轨道 + `fill:*` 填充宽度是值；**拖拽不做**，值由宿主给 |
| ComboBox | `combo_box` | ◐ | 无浮层：点击循环切换选项；值由宿主映射成文案 |
| SpinBox | `spin_box` | ◐ | 上下按钮是真交互；**键入不做**（core 无输入法） |
| ColorPicker | `color_picker` | ◐ | 只迁默认 16 色板 + 选中描边；HSV 面板 / 滑块不做 |
| Text | `text_label` | ✅ | 五档排版 |
| TextField | `text_field` | ◐ | 底/边/placeholder/清空钮/focused 指示条全有；**光标与键入不做** |
| TextArea | `text_area` | ◐ | 多行视觉；滚动不做 |
| ScrollableTextArea | `scrollable_text_area` | ◐ | 滚动条视觉 + thumb 比例（`fill:*` 高度）；不真的滚 |
| AutoSuggestBox | `auto_suggest_box` | ◐ | 输入框 + 建议列表（内联，非浮层） |
| TextInputMenu | `text_input_menu` | ◐ | 内联菜单卡；右键浮层不做 |
| InfoBadge | `info_badge` / `dot_badge` | ✅ | 四严重级 × 实心/描边/圆点 |
| ProgressBar | `progress_bar` | ✅ | running / paused / error |
| ProgressRing | `progress_ring` | ◐ | **弧做不出来**（core 无绝对定位）：迁成"圆环 + 中央数值"，环色随三态 |
| InfoBar | `info_bar` | ✅ | 四严重级底（新增 `*_bg` 三个令牌）+ 关闭钮（折叠，不增删结构） |
| Toast | `toast` | ✅ | 实心严重级底 + 白字 |
| ToolTip | `tool_tip` | ◐ | 常驻展示；hover 延迟与浮层不做 |
| BusyIndicator | `busy_indicator` | ◐ | 三点大小档轮换，帧号由宿主给（框架无时间轴） |
| PageIndicator | `page_indicator` | ✅ | 当前项宽条 |
| ToolSeparator / MenuSeparator | `separator` / `menu-separator` 类 | ✅ | |
| MenuBar | `menu_bar` | ◐ | 栏本身；展开的菜单由宿主挂在旁边（无浮层） |
| Menu / MenuItem / MenuItemGroup | `menu` / `menu_item` | ◐ | 互斥选中退化成 `checked` 分类名；子菜单箭头只有图标 |
| NavigationItem / NavigationView | `nav_item` + `app/shell.rs` | ✅ | |
| NavigationSubItem | `nav_sub_item` | ✅ | 缩进用常驻空档容器（core 无 margin） |
| Segmented / SegmentedItem | `segmented` | ✅ | |
| SelectorBar / SelectorBarItem | `selector_bar` | ✅ | 无外壳的 Tab 行 |
| ErrorPage | `error_page` | ✅ | |
| Frame / Card | `card` | ✅ | |
| SettingCard | `setting_card` | ✅ | |
| SettingItem | `setting_item` | ✅ | 左图标 + 标题/描述 |
| SettingExpander | `setting_expander` | ◐ | 卡头 + `COLLAPSED` 折叠体 |
| ListView / ListViewDelegate | `list_view` | ◐ | 纵排行 + 选中态；滚动与键盘导航不做 |
| TableView / HeaderView / Delegate | `table_view` | ◐ | 定列宽表头 + 数据行；排序/列宽拖拽不做 |
| SwipeView | `swipe_view` | ◐ | 当前页 + 圆点；滑动手势不做 |
| Avatar | `avatar` | ◐ | 正圆 + 首字母（内置图标集无人物像） |
| Clip | `clip_card` | ✅ | |
| Expander | `expander` | ✅ | |
| RoundedCornerOverlay | `rounded_corner_overlay` | ◐ | 迁成"圆角边框容器"，不是真遮罩（core 无 clip 到圆角） |
| AcrylicBrush | `acrylic_card` | ◐ | 半透明卡；真实亚克力（背景模糊 + 噪点）不做 |
| Dialog / DialogButtonBox | `dialog` | ◐ | 居中卡 + 按钮行；模态遮罩与焦点圈闭不做 |
| Calendar / CalendarDatePicker | — | ◐ | 由 `date_picker` 承担月历网格；翻月不做 |
| DatePicker | `date_picker` | ◐ | 静态当月网格；滚动选值不做，点选按 key 路由 |
| TimePicker / PickerView | `time_picker` | ◐ | 可见窗口由宿主给（`(key, 文字)` 静态切片）+ 选中高亮 |
| Flyout / Popup | — | ✗ | core 无 overlay 层：浮层是**框架级**能力，不是控件级 |
| AnimatedVisualPlayer / MediaPlayerElement | — | ✗ | 媒体播放，超出"基础组件"范畴 |

统计：**45 个控件函数**（另 4 个内部件 `state` / `with_classes` / `glyph` /
`hovered_key`+`pressed_key`）；RinUI 78 个 QML 里，**3 个明确不做**（浮层、媒体），
其余全部落地，其中 34 个带降级记账。

## 6. 迁移过程中额外抓到的七个缺陷（都不是控件问题，是真 bug）

1. **三处每帧内存泄漏**：`view()` 每帧重建元素树，而 `clip_grid` / `combo_box` /
   `time_picker` 用 `format!` + `Box::leak` 生成 key 与文案 —— 泄漏随帧数无界增长。
   修法：全部改成静态表（clip 卡 key、`HOME_CLIP_KEYS` 等）或把映射上移给宿主
   （`combo_box(key, value)`、`time_picker(..., 可见窗口切片)`）。
   这条值得单列，因为它只在窗口路才致命，`--dump-cpu` 只渲一帧，永远看不出来。
2. **CPU 参考光栅器从不画矢量图标**（本轮最重的一个）。
   `cpu_raster::draw_batch` 只遍历 `solid` 与 `images` 两段，`paths` 整段从未被
   光栅化 —— 与历史上 `images` 那段**完全同型**的洞（那里注释还留着）。症状安静
   到无法自证：`report.path_triangles` 照常增长（几何统计看着很健康），图里却一个
   图标都没有；而 GPU 路正常，所以真机上一切正常。
   修法：补 `CpuCanvas::triangle`（NDC→像素、4×4 子采样近似 MSAA）并在
   `draw_batch` 里遍历 `paths`。回归：`多边形被光栅化到画布上`
   （判据分两半：三角形内部有不透明像素、三角形外仍是清屏色 —— 只判前者的话
   「无条件糊满画布」也能过）。
   **它同时让本文件这条自验链第一次真的看见图标**：在此之前，我核对过十几次
   渲染图，每次都「图标不见了」，却每次都当成自己看错。
3. **图标名按文件名写而不是按登记名写**：`REGISTRY` 刻意用控件词汇
   （关掉叫 `close`，资源文件却叫 `dismiss.svg`），应用按 `dismiss` 去用 ⇒
   `IconSet::get` 回落到 `Icon::EMPTY` ⇒ 空白图标位。
   修法：应用侧改用 `close`；门禁侧补 `every_icon_name_the_tree_emits_is_registered`
   （**引用方**的门禁 —— core 侧只能抓「登记了但空」，抓不了「该登记却没登记」）。
   这条门禁在写完当天就抓到了我自己上面那个错（我一度把 `dismiss` 补进 REGISTRY，
   与词汇约定冲突）。
4. **陈旧例外清单**：`backend_parity.rs` 的 `KNOWN_DIVERGENT` 里还挂着 `SoftLight`，
   而两个后端早已逐像素一致。门禁 `the_known_divergence_list_is_not_stale` 抓到了
   它 —— 这条门禁的设计（清单不许养过期条目）在这次**真的生效了一次**。
5. **重复属性**：`d3d11_draw.rs` 里两个测试各带了两个 `#[test]`（新插入的文档块
   把属性与函数隔开、又在下方补了一个），rustc 一直在报 `duplicated attribute`。
6. **会自己骗人的时间戳门禁**：`shader_assets_are_in_sync` 用严格 `>=` 比
   `.bin` 与 `.hlsl` 的 mtime，而批量落盘会把两者刷到同一瞬间、相差几毫秒 ——
   「我什么都没改」也能红。加 2 秒宽限（远小于真忘编译的间隔，意图不松）。
7. **空图标能通过「所有内置图标都能三角化」**：那条在 `icon.is_empty()` 时是
   `continue`。补 `所有内置图标都不是空的`（一张内置表里的图标不许有空的）。

## 7. 「点击之后渲染有问题」——交互期才暴露的六个缺陷

> §6 那些是**单帧**能看出来的。这一节全部只在**交互之后**暴露，而它们在单帧
> dump 上**完全正常** —— 这是自验手段的系统性盲区，值得单列。

### 7.1 `diff` 按位置配对 ⇒ 换页时旧页面整棵留在树上

`diff` 的子节点对账只按**下标**配对，隐含假设「同一位置永远是同一类节点」。
换页就违反它：`page` 的子节点从 `home` 变成 `basic-input`，两者 tag 都是
`Container`，于是**旧页面的整棵树被就地改造成新页面** —— 改得掉的属性改掉了，
改不掉的（多出来的那些）**留在树上继续绘制**，带着自己的样式与矩形。

症状：点了导航之后界面花掉；**不报错、不告警**（对 diff 来说每一步都合法）。

修法：**按 key（身份）配对**，key 不同即整棵替换；顺带把「列表里插一项」从
「后面全部重建」变回「只动被插处」。

新增门禁 `interaction_leaves_no_stale_appearance`：跑一串交互（换页 / 勾选 /
开关 / 折叠 / 分段），每一步把**保留式树**与**同一状态的冷启动树**逐节点逐属性
比对。判据不能是「与上一帧比」—— 残留本来就是上一帧的值；必须是与「无历史的
正确答案」比。

### 7.2 样式层只写不清 ⇒ 状态类消失后旧值永久残留

`apply` 每帧只**写**匹配到的声明。分类名这帧不匹配时，没有任何声明会提到那些键，
于是上一帧的值原样留着。症状：取消勾选后底色还是选中色。

它长期没被发现，是因为**多数**样式表里每条状态规则都和基类规则声明同一批键，
基类每帧把残值盖回去，洞被"恰好"堵住。

修法：`apply_node` 先算「本轮要写哪些键」，写完再把「上一轮写了、这一轮没人写」
的键**拆掉**（`retain_styled_props`）。顺带把 `zephyra-style` 里那条早写着
"WARNING: currently **vacuous**… B4 must fix this" 的恒真记录变成真测试。

### 7.3 本地层只写不清 ⇒ 「本帧不设的属性」赖着不走

同一形状、另一层：声明树里没有的属性，保留节点上不该留着。症状很具体：控件写
`if closable { } else { .with(COLLAPSED, true) }`，宿主翻回 `closable` 之后元素
**永远折叠着**。修法：`diff` 写完本地属性后拆掉本轮没声明的。

### 7.4 clear 的区域与 scissor 无关（两个后端都写错了）

代码里断言「D3D12 的 clear 受 scissor 裁掉、D3D11 不受」—— **两边都错**：clear 的
区域只由 `pRects` 决定（`NULL` ⇒ 整张）。

于是每帧**整屏清成底色 + 只画脏矩形** ⇒ **任何交互帧（hover / 按下 / 勾选）都把
没脏的部分抹成一片底色**。首帧完全正常（`Damage::Full`），所以"渲染有问题"只在
交互之后出现，与布局、内容、像素都无关。实测现场：点一下复选框，界面几乎全黑、
只剩那一小块。

修法要分两种情况，因为「只重画脏区」有个前提：**脏区之外的上一帧画面还在渲染
目标里**。

- D3D12 + MSAA 开：成立 —— 渲染目标是自有的多重采样纹理，逐帧存活、帧末整体
  resolve。按脏区清 + 按脏区画。
- D3D12 MSAA 关 / D3D11：不成立 —— 渲染目标就是 flip model 的后缓冲，
  `Present` 之后内容未定义。**强制整帧**，代价写在明处（画面慢 vs 画面错）。

### 7.5 `InputRouter` 的 Down/Up 不 bump `revision` ⇒ 第一次点击「没反应」

宿主靠 `revision` 变化决定「这一帧要不要重绘」，而 click 之后应用改的状态
**都是画出来的**。Down/Up 时 `pressed` 明明变了却没 `revision++`；若此时 hover
恰好也没变（点按钮时鼠标不动 —— 最常见），宿主判定「什么都没变」⇒ 不重绘。

症状：**第一次点击「没反应」，把鼠标移开再移回来才生效**。画面没有任何异常，
只是「少了一帧」—— 最难查的一类。

修法：Down 设 `pressed`、Up 清 `pressed` 时都 `revision += 1`。回归：
`pressing_and_releasing_both_bump_the_revision`（判据刻意不让 hover 同时变化）。

### 7.6 窗口尺寸语义错 ⇒ 高 DPI 下画布与窗口对不上

宿主不给尺寸时用系统默认（本机 640×480），固定画布的应用被**裁掉一大块**；
补上尺寸时又用了**逻辑**像素，本机 125% 下 `LogicalSize(1400,800)` 变成物理
1750×1000，于是内容只占窗口左上角、右/下留白。

两种症状都像"排版坏了"，而布局与绘制都没错。修法：`new_lenient_sized` 收
**物理像素**（整条链是「像素对像素」的），并把 `inner_size` 与 `scale_factor`
打进启动日志。

### 7.7 顺带三处

- `list-view` / `menu` / `context-menu` / `suggest-list` 的行没 stretch ⇒
  选中与悬停底色只盖住文字那一小段。加 `cross-align: stretch`。
- 新增门禁 `hit_test_agrees_with_the_painted_rects`：每个可交互节点的矩形中心，
  `HitResult::target()` 必须是它自己。命中与绘制的偏移**用截图核不出来**
  （画面完全正确），而症状是「点 A 执行 B」。
- 宿主新增 trace：启动打 `inner_size` + `scale_factor`，指针事件打
  「坐标 → 处理目标」。本轮有三轮排查卡在"猜"上，其中两轮结论是「应用是对的，
  是我的测量脚本错了」（PowerShell 非 DPI-aware ⇒ 坐标差 1.25 倍；窗口没前台
  焦点 ⇒ 点击不送达）。**宿主能自己说出坐标系，是这类问题的前提。**

## 8. 一条方法论教训（比上面任何一条 bug 都贵）

**验图时"图上少东西"最容易被当成自己看错。** 本轮我对着 dump 图反复核对了十几次，
每一次都看到导航栏没有图标，每一次都归因为"大概是设计如此"或"自己眼花" ——
直到某个探针顺手打出 `icon dismiss: is_empty`，才顺着查出整段 `paths` 从未被光栅化。

**交互路径的缺陷比单帧缺陷更难被现有自验看到**，因为单帧图永远是对的。所以这轮把
验证手段也当产物来修：新增「保留树 vs 冷启动树」与「命中 vs 绘制」两条门禁，
以及宿主自报坐标系的 trace。

另有两条自查：

- 出图批处理必须**核对产物互不相同**（比字节数即可），否则就是陈旧二进制 ——
  本轮就因此把十一张"逐路由图"当成了真实结果，全是同一张旧图；
- "图上少了某类东西"当**待查故障**，而不是观察结论 —— 参考图少画一类图元，
  是那条链路自己的缺陷，不是观者的。

## 9. 上传暂存区的对齐预算：一次"调用方必须预测容量"的缺陷

### 9.1 现象与归因

窗口路在**带滤镜的页**上一次性报三条：

```
[zephyra-d3d12] 记录绘制命令失败: 上传暂存区不够：本段要 12 字节，从 8192 起放不下（暂存区 8192 字节）
```

三个数字都对得上，因此归因不需要猜：

- **12 字节**是 `draw_pass_quad` 传的一枚四边形索引 —— `expand_indices_with_paths(1, 0)`
  = 6 个 `u16`。主路径上传的是整帧索引，不会是 12。
- **8192**是索引流缓冲 `next_power_of_two` 之后的容量（初值 4KB，主路径一帧就用满）。
- **起点正好顶在末尾**是"只差那点对齐"的形状，不是"缓冲真的太小"。
- **三条**对应 testbed 页 6 个样张里恰好 3 个带滤镜（`fx-blur` / `fx-shadow` /
  `fx-triple`），每个样张一次离屏 pass。不是"每帧失败一次"，是"一帧里三个滤镜"

### 9.2 根因：两处独立算布局，必然漂移

`StreamBuffer::reserve`（决定要不要扩容）与 `record_upload`（真正记录拷贝）
各自算了一遍段落位置：

- `reserve` 判的是 `head + extra <= size`；
- `record_upload` 实际用的起点是 `align_up_256(head)`。

`head = 8000`、缓冲 8192、`extra = 12` 时：前者算 `8012 <= 8192` **放行**；
后者把起点推到 **8192** 再放 12 字节 → 越界。差的就是对齐最多 255 字节那一项。

修法不是"把对齐补进 `reserve`"就完事 —— 那只是**第二次**让两边一致，
下次再来第三种上传路径还会漂。所以把两个调用点收敛到**一个**函数
（`upload_span`），并把不变式写成纯数学测试（本机 D3D12 资源创建不可用，
`tests/d3d12_draw.rs` 因此整体 `#[ignore]`，真设备回归在这里跑不起来）。
判据刻意写成那个 bug 的形状：`upload_end(8000, 12) > 8192` —— 哪天对齐项被去掉，
这条立刻红。

### 9.3 纯数学测试当场抓到的第二个缺陷

 saturation 原来只加在**最后一步**（`off + bytes`），而 `head + ALIGN - 1` 自己就会溢出。
我在测试里断言"荒谬 head 的 want 必须大到不可能被当成放得下"，才发现它：

- 回绕让 `off` 变成**很小的数**；
- 于是 `want <= size` 成立、扩容被跳过；
- `record_upload` 从老远的老位置写，**覆盖同帧已录制的其它段**。

那不是看得见的报错，是静默的内存损坏。修成两处都 `saturating_add`。

测试还排除了一个**看起来该守、实际守不住**的属性："end > head"。
`head` 接近 `u64::MAX` 时 `off` 会略小于 `head`，但它不可达 —— 那种 `head` 下 `want`
必然是天文学数字，`reserve` 必然扩容、`head` 归零，`record_upload` 根本拿不到它。
断言一个不可达输入上的性质，只会把测试写成撒谎。改成断言真实后果。

### 9.4 一条方法论：这个 bug 的"形状"值得逐位对上

当时完全可以从报错数字反推出根因：**12 字节的段落在哪、8192 从哪来、
为什么是三条**。三者都对上之后再动手，一次就修在点上。

反过来，如果只看"暂存区不够"就把初值从 4KB 改大 —— 那会让报错消失，
但报错消失不等于缺陷消失：对齐那一项仍然算错，下一次主路径顶点数正好落在
某个边界上时会以"更大的初值也放不下"的形式回来。**报错消失不是修好的证据，
根因对上是。**

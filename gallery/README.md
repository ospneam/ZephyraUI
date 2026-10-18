# gallery —— 应用层

把 `ZephyraUI` 的控件摆成一屏能看的东西。它是**应用**，不是库。

```bash
# CPU 参考光栅（不开窗、不碰 GPU，无显示器也能跑）
cargo run -p zephyra-gallery --bin gallery -- --dump-cpu --route home

# 真窗口（D3D12 / D3D11 / Vulkan 三选一，由 feature 选）
cargo run -p zephyra-gallery --bin gallery
```

## 11 条路由

| 路由 | 内容 |
|---|---|
| `home` | 概览：常用控件活样例 + 3×3 分类入口 |
| `basics` | Button / TextField / TextArea / ComboBox / SpinBox / Slider / ColorPicker |
| `toggles` | CheckBox / RadioButton / ToggleSwitch / Segmented |
| `layout` | Card / Expander / SettingItem / SettingExpander |
| `collections` | ListView / TableView / SwipeView / Avatar |
| `menus` | MenuBar / Menu / ToolButton / DropDownButton |
| `navigation` | NavItem / NavSubItem / SelectorBar |
| `status` | Badge / InfoBar / Toast / ProgressBar / ProgressRing / Busy / PageIndicator |
| `dialogs` | DatePicker / TimePicker / ToolTip / AcrylicCard |
| `effects` | 液态玻璃（`filter: refraction(...)`）+ 滤镜 |
| `settings` | 主题 / 通知 / 音量 / 语言 |

**为什么是 11 条而不是 70 条**：旧结构是 Rin-UI 的翻版（10 页 + 14 分组 + 44 控件页），
能查到每一个控件，代价是导航树大到没人用 —— 而且那让 `catalog.rs`（340 行）与
`showcase.rs`（453 行）成为两份几乎重复的模板代码。现在**按控件的族分页**。

代价：**控件级的深度文档没有了**。那件事的正统载体是控件层自己的 doc 注释，
gallery 的职责是「让人看见它长什么样、摸一摸」，不是当手册。

## 命令行

| 标志 | 作用 |
|---|---|
| `--dump-cpu` | CPU 参考光栅出 BMP（`gallery-cpu.bmp`），不碰 GPU |
| `--route <id>` | 指定起始路由（`--list-routes` 列出全集） |
| `--list-routes` | 打印 11 条路由 |
| `--lang <tag>` | 覆盖语言协商结果 |
| `--enter-ms <n>` | 把入场动画时钟钉在第 n 毫秒（出图用） |
| `--theme light\|dark` | 覆盖主题 |
| `--bench` / `--bench-resize` | 稳态 / resize 路径的 CPU 计时 |
| `--alloc-probe` | 装上分配计数用的 `GlobalAlloc` |

## 三个必须知道的边界

### 一、页面必须一屏装得下

core 既没有滚动也不裁剪溢出，属性表里也没有 `min-height` / `margin-*`。
所以「内容太多」唯一的表现就是**下沿被吃掉**：节点在树里、可交互、样式齐全，
只是命中测试按定义不会命中它们 —— 画面上就是「点不到」，且不报任何错。

门禁 `hit_test_agrees_with_the_painted_rects` 对**全部路由**跑一遍并单独报
「矩形掉出客户区」。core 没有折行语义，所以「横排几列」是**排出来的**，
不是算出来的。

### 二、菜单与对话框是内联的，不是浮层

`Layer::Popup` 与 `Command::PushClip` 有类型但没有生产调用方。所以：

- `menus` 页展开的菜单是页面里的一块**内容**，不是盖在上面的浮层；
- `dialogs` 页的对话框同理，页面里还留了说明文字点破这件事。

**别把它做成「点一下弹出」** —— 那会做出一个「点击后页面内容被替换掉」的东西，
而它看起来像弹窗、行为不是，是最难查的一类 bug。

### 三、只有这些控件真的接了状态

`click()` 里接了状态的：路由、语言、主题、分段、复选框、单选、开关、折叠、
ComboBox、SpinBox、RoundButton、DropDown、菜单栏与菜单条目。

**滑条、表格、Tooltip、进度条是展示型** —— core 还没把它们做成输入设备。
那不是漏接，别给它们编造一个假动作。

## 样式表

两份，都由 `src/styles.rs` 用 `include_str!` 内嵌：

| 表 | 内容 |
|---|---|
| `shell.zstyle` | 外壳：标题栏 / 导航栏 / 内容卡 |
| `pages.zstyle` | 全部页面的编排 |

从 15 份合成 2 份的原因：旧结构每个页面一个文件，于是「新增一个页面」要做三件事
（建 `.zstyle`、改常量表、改加载列表），而后两处漏了不会报错，只是**那个页面完全
没有样式**。现在页面编排的分类名高度重复（10 个 `.page-*` 都只是「竖排 + 间距」），
合成一份之后相邻，改间距一眼看全。

## 测试

```bash
cargo test -p zephyra-gallery
```

45 条：25 条树级门禁 + 7 条动画接线 + 3 条 hover/布局 + 10 条交互回归。

门禁全部按**整棵树 × 全部页面 × 全部交互态**判。逐函数查只能查到「我记得去查的那个」。

排查过程见 [`docs/dev-log/2026-10-gallery-rewrite.md`](../docs/dev-log/2026-10-gallery-rewrite.md)。
# gallery —— 开发者示例项目

把 ZephyraUI 用**做一个应用**的方式摆出来：指南层教「怎么构建」，参考层给「控件长什么样」。

```bash
# 真窗口（D3D12 / D3D11 / Vulkan 三选一，由 feature 选）
cargo run -p zephyra-gallery --bin gallery

# CPU 参考光栅（不开窗、不碰 GPU，无显示器也能跑）
cargo run -p zephyra-gallery --bin gallery -- --dump-cpu --route guide
```

## 两层结构

### 指南（6 章，学习顺序）

| 路由 | 教什么 |
|---|---|
| `guide` | 快速开始：一个按钮 + 计数，跑通「点击 → 状态 → 树 → 渲染」整条链 |
| `state` | 状态管理：哪些控件真的接了状态、哪些是展示型（core 的当前边界） |
| `styling` | 样式表：外观不进源码、`@令牌` → `Theme::role()`、`affects_layout` |
| `layout` | 布局与响应式：无滚动、断点三档、触控 44 —— 三个**不报错**的边界 |
| `theme` | 主题与国际化：暗/亮意图、Mica、三语协商 |
| `testing` | 门禁与出图：三层自验，每层防哪类静默失败 |

首次打开落在 `guide`。每章 = 一两句说明 + 一个**可交互的完整示例**。

### 控件参考（附录，1 页 9 族）

`reference` 页内用 segmented 切九族：Basic Input / Selection / Layout /
Collections / Menus / Navigation / Status / Dialogs / Liquid Glass。

**为什么不是九条路由**：导航栏高 756，两层平铺（6 章 + 9 族 + Settings）装不下，
而 core 的二级导航 `nav_sub_item` 不折叠、九个子项常驻照样撑爆 —— 超出的项
唯一的表现是「下沿被吃掉」（画不出来也点不到，且不报错）。所以参考层收进
一页，segmented 切族；每族内容独立一屏，装得下。

### `settings`（置底）

主题 / 通知 / 音量 / 语言。不是指南的一部分：它是**成品页**的形态参照。

## 三个必须知道的边界

### 一、页面必须一屏装得下

core 既没有滚动也不裁剪溢出，属性表里也没有 `min-height` / `margin-*`。
「内容太多」唯一的表现是**下沿被吃掉**：节点在树里、可交互、样式齐全，
只是命中测试按定义不会命中它们 —— 画面上就是「点不到」，且不报任何错。
门禁 `hit_test_agrees_with_the_painted_rects` 对**全部路由**逐节点量。

### 二、菜单与对话框是内联的，不是浮层

`Layer::Popup` 与 `Command::PushClip` 有类型但没有生产调用方。`reference`
页的 menus 族里展开的菜单是页面里的一块**内容**。别把它做成「点一下弹出」
—— 那会做出一个「点击后页面内容被替换掉」的东西，看起来像弹窗、行为不是。

### 三、只有这些控件真的接了状态

`Gallery::click` 里接了状态：路由、语言、主题、分段、复选框、单选、开关、
折叠、ComboBox、SpinBox、RoundButton、DropDown、菜单栏与菜单条目、参考页
族切换、玻璃参数步进、快速开始计数。

**滑条、表格、Tooltip、进度条是展示型** —— core 还没把它们做成输入设备。
那不是漏接，别给它们编造一个假动作（`state` 章把这条边界画给开发者看）。

## 样式表

两份，由 `src/styles.rs` 用 `include_str!` 内嵌：

| 表 | 内容 |
|---|---|
| `shell.zstyle` | 外壳：标题栏 / 导航栏 / 内容卡 |
| `pages.zstyle` | 全部页面的编排（指南六章 + 参考九族） |

从 15 份合成 2 份的原因见 `git log a227257`：旧结构每页一个文件，「新增页面」
要做三件事且后两处漏了不报错。

## 测试

```bash
cargo test -p zephyra-gallery
```

51 条：31 树级门禁 + 7 动画接线 + 3 hover/布局 + 10 交互回归。

门禁全部按**整棵树 × 全部路由 × 全部交互态**判（`tests/gates.rs`）。
其中 `every_stylesheet_rule_matches_something` 是反查方向：样式表里每条规则
必须至少在一个（路由 × 交互态 × 断点 × 入场档）组合里命中一个元素 ——
「分类名存在」「元素有分类名」两条都拦不住**死规则**。

## 命令行

| 标志 | 作用 |
|---|---|
| `--dump-cpu` | CPU 参考光栅出 BMP（`gallery-cpu.bmp`），不碰 GPU |
| `--route <id>` | 指定起始路由（未知路由**直接退出**，不静默回首页） |
| `--list-routes` | 打印 8 条路由 |
| `--lang <tag>` | 覆盖语言协商结果（与 `theme` 章 / `testing` 章的语言切换同状态） |
| `--enter-ms <n>` | 把入场动画时钟钉在第 n 毫秒（出图用） |
| `--theme light\|dark` | 覆盖主题 |
| `--bench` / `--bench-resize` | 稳态 / resize 路径的 CPU 计时 |
| `--alloc-probe` | 装上分配计数用的 `GlobalAlloc` |

排查过程见 [`docs/dev-log/2026-10-gallery-example-rewrite.md`](../docs/dev-log/2026-10-gallery-example-rewrite.md)。

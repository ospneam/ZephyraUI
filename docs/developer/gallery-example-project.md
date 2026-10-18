# gallery 重写：从「控件展柜」到「开发者示例项目」

状态：**已实施**（commit 待填）。最终形态与本文的差异：参考层不做二级导航
（`nav_sub_item` 不折叠，九个子项常驻反而更占——44px vs 40px），而是
`reference` **页内 segmented 切九族**。排查过程见
[`docs/dev-log/2026-10-gallery-example-rewrite.md`](../dev-log/2026-10-gallery-example-rewrite.md)。

## 一、定位变更

| | 现在 | 重写后 |
|---|---|---|
| 读者 | 要查「某控件长什么样」的人 | 要写 ZephyraUI 应用的**开发者** |
| 组织轴 | 控件族（覆盖面优先） | 开发者任务（构建一个应用要回答的问题） |
| 导航 | 11 条路由平铺 | 两层：**指南**（主线）+ **控件参考**（附录） |
| 每页内容 | 一族控件的罗列 | 一个可交互的完整示例 + 简短说明 |

现在导航 10 项 + Settings。重写后 **8 项**：6 章指南 + 「控件参考」+ Settings 置底。

**为什么不把两层平铺成 16 项**：导航栏高 756，项高 40 + 间距 6 = 46，16 项 + 两个
分组标题 = 838 > 756。core 既无滚动也不裁剪溢出，超出的项唯一的表现就是
**下沿被吃掉**（画不出来也点不到，且不报错）。所以参考层收进**一页内分节**，
不作为一级导航项——它仍然是「附录」，只是入口只有一处。

## 二、导航与路由

```text
指南（Guide）
  ├─ guide      快速开始      一个最小骨架跑通：树 → 样式 → 状态 → 路由
  ├─ state      状态管理      哪些控件真的接了状态、click() 统一入口在哪
  ├─ styling    样式表        分类名怎么发、令牌怎么引、affects_layout 是干什么的
  ├─ layout     布局与响应式   无滚动意味着什么、断点三档、触控 44
  ├─ theme      主题与国际化  暗/亮、Mica、三语切换
  └─ testing    门禁与出图    --dump-cpu 出图、--list-routes、树级门禁怎么接
附录（Reference）
  └─ reference  控件参考      一页内分 9 节，每节一族控件（复用现有页面内容）
系统
  └─ settings   主题 / 通知 / 音量 / 语言   （置底，现状保留）
```

「控件参考」页**内部分节**：Basic Input / Selection / Layout / Collections /
Menus / Navigation / Status / Dialogs / Liquid Glass。每节直接复用现有
`pages/*.rs` 的控件罗列，但**不各自占一条路由**——它们由 `reference` 一节
承载，节标题即族名。

这样指南层的 6 章是**学习顺序**（首次打开落在 `guide`），参考层是
**查阅入口**（需要时点进去）。

## 三、指南层六章各教什么

每章 = 页首一两句说明 + 一个可交互的完整示例。说明文字要短：core 没有滚动，
正文才是这一页的主体。

### 1. `guide` — 快速开始

一个按钮 + 一个计数。教**主干**：

```text
 输入 ─▶ Gallery::on_input   推进时钟 / 还原 click
          └▶ Gallery::view   ─▶ shell::window  标题栏 + 导航 + 内容卡
                                  └▶ pages::page  按路由分发
                                      └▶ controls::*  控件层
```

页面上就一个按钮，点它计数 +1。开发者第一次打开就能看到「点一下，界面变了」，
而背后整条链已经全跑通。说明文字点出这条链，并指向 `--dump-cpu`（不开窗也能看）。

### 2. `state` — 状态管理

教**「哪些控件真的接了状态」**——这是开发者最容易踩的坑。

页面上放三组：
- **真的接了**：CheckBox / ToggleSwitch / Segmented / ComboBox —— 点一下界面变
- **展示型**：滑条 / 进度条 / Tooltip —— 点了没反应（core 还没做成输入设备）

说明文字明确：那不是漏接，是 core 的当前边界，记账在 `docs/dev-log/README.md`。
交互仍走 `Gallery::click()` 单一入口，页面只读 `&Gallery`。

### 3. `styling` — 样式表

教「外观不进源码」这条纪律，用**对照**讲：

- 左：一个按钮，**颜色**走样式表（`@令牌` → `Theme::role()`）
- 右：同一个按钮，**间距**写死在源码里（被门禁拦）

再引 `affects_layout`：声明「这个属性影响布局」的门禁，漏声明会让「只改颜色」
也触发全树重排。现场改样式表 → 重加载，颜色变而布局不变。

### 4. `layout` — 布局与响应式

教 core 的三个真实边界，每条配一个**看得见**的例子：

- **无滚动**：内容超出唯一的表现是下沿被吃掉（放一列刚好溢出的卡片）
- **断点**：窗口宽度 640 / 1000 / 1400 三档，导航栏在窄档收成图标列
- **触控**：按钮小于 44 时点击区不够，`Gallery::touch_target` 给出官方换算

### 5. `theme` — 主题与国际化

暗/亮切换（`Arc<AtomicU8>` 主题意图 + Bootstrap 对账）、Mica 半透明标题栏、
三语切换（中/英/日，走 `Gallery::tr` 与 `Preferences` 协商）。这一章基本是现有
`settings` 页的内容，但按「主题与语言」这条任务线重新组织，并补上
「为什么意图走共享格而不是直接引用」的说明。

### 6. `testing` — 门禁与出图

教「这个 repo 怎么自验」：

- `--dump-cpu` 出 BMP，不开窗不碰 GPU
- `--list-routes` / `--route` 逐页出图（未知路由直接退出，不静默回首页）
- 树级门禁：整棵树 × 全部页面 × 全部交互态，不是逐函数
- `cargo clippy -p zstylec -- check <file.zstyle>` 也能验样式表

页面上放**可直接抄的命令**，每条配一句「它防的是什么」。

## 四、门禁体系映射

30 条旧门禁，按「与组织方式是否耦合」分三类。**推翻的是遍历基础与路由耦合的
那部分，防坑的断言一条不丢**——它们防的坑与新旧结构无关。

### 保留（只改遍历的路由表）

| 旧门禁 | 防的坑 |
|---|---|
| `the_whole_tree_writes_no_appearance_in_source` | 外观写进源码 |
| `every_class_name_the_source_emits_exists_in_the_stylesheet` | 分类名拼错 → 静默失效 |
| `every_element_is_reachable_by_the_stylesheet` | 少写 `.style(..)` |
| `every_token_the_stylesheet_uses_exists_in_theme` | 令牌拼错 → 静默保持原值 |
| `every_icon_name_the_tree_emits_is_registered` | 图标名拼错 → 静默消失 |
| `every_stylesheet_rule_matches_something` | 死规则（表→树反查） |
| `every_page_builds_a_real_tree` | 页面造出空树 |
| `hit_test_agrees_with_the_painted_rects` | 内容溢出客户区 → 点不到 |
| `interaction_leaves_no_stale_appearance` | 交互后残留旧外观 |
| `route_table_and_page_table_agree` | 路由表与页表 id 集合不同 |
| `every_route_renders_its_own_page` | 点进去却是首页 |
| `every_page_key_is_a_legal_route` | 页 id 不在路由表里 |
| `every_page_carries_its_own_controls` | 某页没有控件 |
| `every_route_id_is_translated_in_every_supported_language` | 某语言缺译文 |
| `segment_keys_are_all_wired` | 分段 key 与 click 分支对不上 |
| `breakpoint_categories_match_doc_and_are_monotonic` | 断点倒挂 |
| `column_count_tracks_width_and_baseline_is_five` | 列数不随宽度变 |
| `content_width_follows_breakpoint` | 内容宽与断点脱钩 |
| `touch_target_converts_to_official_ephemeral_px` | 触控尺寸换算错 |
| `only_fill_nodes_carry_local_width` | 源码写死宽度 |
| `shell_metrics_match_stylesheet` | 外壳尺寸两处真相 |
| `root_container_writes_no_fixed_size` | 根容器写死尺寸 |
| `root_container_still_fills_client_area_at_baseline` | 根容器不铺满 |
| `shell_layout_geometry` | 外壳几何漂了 |
| `the_mica_titlebar_token_is_actually_translucent` | Mica 令牌不半透明 |
| `the_app_always_asks_for_the_mica_look` | 没请求 Mica 外观 |
| `glass_params_are_not_hard_coded` | 折射参数写死（effects 页仍在） |
| `glass_param_tables_line_up` | 参数表与文本表错位 |
| `glass_param_keys_all_wired` | 参数按钮没接线 |
| `glass_param_steppers_are_clamped_without_side_effects` | 步进无副作用 no-op |

### 改写

| 旧门禁 | 新形态 |
|---|---|
| `route_table_and_page_table_agree` | 变成「导航表（8 项）与页表 id 集合一致」，且**指南层在前** |
| `every_route_renders_its_own_page` | 不变，但参考层是**一页**，路由数从 11 降到 8 |
| `column_count_tracks_width_and_baseline_is_five` | 重写后网格列数可能更没消费者；**断言保留**（理由见旧注释：等 core 长出 wrap 语义时直接接上） |

### 新增（指南层特有）

| 新门禁 | 钉住什么 |
|---|---|
| `guide_chapters_are_in_learning_order` | 6 章按学习顺序排列（快速开始必须第一） |
| `guide_pages_carry_interactive_examples` | 每章至少一个**可交互**控件（只有说明文字的章是失败的） |
| `reference_page_has_one_section_per_control_family` | 参考页 9 节齐全，一节一族 |
| `every_documented_command_is_real` | testing 页列的每条 CLI 命令都能真跑（防「文档与实现漂移」） |

## 五、分阶段

1. **骨架**：路由表 8 项、`guide` 六章模块建好（先空壳）、`reference` 页聚合、
   导航改 8 项。项目能编译能跑，`--list-routes` 列出 8 条。
2. **指南层逐章填充**：每章内容 + 门禁。每填一章跑一次门禁。
3. **参考层**：现有 9 个页面内容搬进 `reference` 的分节，删掉各自的路由。
4. **门禁重写**：按映射表改，跑全量。
5. **README 重写**：从「控件展柜」改成「示例项目」的口吻。
6. **dev-log**：四段式记录。

每阶段结束都 commit。

## 六、会被删掉的东西

- `catalog.rs` / `showcase.rs` 已在上轮删除，不再回来。
- 旧 `home` 页（3×3 分类入口网格）被 `guide` 章取代。
- `pages/*.rs` 里 9 个页面的**路由身份**消失（降级为参考层的分节），内容保留。
- `PAGES` 里 9 个条目消失。

## 七、风险

- **参考层一页装得下吗**：9 节控件罗列在一页里，几乎肯定**装不下**（每节 3-4
 行 × 46 = 150+，9 节 = 1350 >> 756）。**这是最大的风险**，可能的解法：
  - (a) 参考页只放**族入口列表**，点一个族才展开该族控件（但 core 没有页内
    导航跳转，展开 = 整页内容替换，可行但像「弹窗」）
  - (b) 参考页按**两列**铺 9 节，每节压缩到 2 行
  - (c) 参考层仍是**多条路由**，但导航栏里收成一个「控件参考」折叠项
    （`nav_sub_item` 支持二级）
  倾向 (c)：导航栏「控件参考」项下挂 9 个子项（二级导航），既不挤占一级
  导航，又保住按族查阅。**这条要在骨架阶段实测二级导航的几何**。

- **指南层 6 章会不会太像教程而不像 gallery**：可接受。「示例项目」本来就
  该有教程味道，且每章仍以可交互示例为主体。

- **门禁重写的回归风险**：30 条门禁一次改完容易顾此失彼。分阶段 + 每阶段全量
  跑。

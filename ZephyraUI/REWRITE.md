# Rin-UI Demo 全量重写 —— 任务清单

> 目标：把 `E:\Rin-UI\examples\`（Qt Quick 的 RinUI Gallery）**逐项照抄**成一个
> ZephyraUI 应用：**功能、样式、设计**都对齐原版，同时**保留** zephyra 现有 demo
> 里的全部**测试功能**（探针/bench/回归展台）。
>
> 两条硬约束：
>
> 1. **Windows 优化与通用代码分离** —— Windows 专属能力放独立 crate，
>    通用路径走 trait/抽象，非 Windows 有 stub 实现，为后续 Linux 适配留口。
> 2. **写 Windows 优化前先查文档** —— 每一条 Win32/D3D/GPU 优化都要在
>    `docs/` 下留「查了什么、出处、结论」，不许凭印象写 API。

---

## 0. 现状盘点（已完成，作为规划的依据）

| 参照物 | 位置 | 关键事实 |
|---|---|---|
| Rin-UI gallery | `E:\Rin-UI\examples\` | `gallery.qml` 1400×800 + 标题栏搜索框 + NavigationView；`pages/` 18 个分类页 + `pages/controls/` 42 个控件页；`ItemData.qml` 登记 44 个控件（title/type/desc/page/added） |
| Rin-UI 组件库 | `E:\Rin-UI\RinUI\components\` | 78 个 QML，按 BasicInput / Text / StatusAndInfo / MenusAndToolbars / Navigation / ListAndCollections / Layout / Styles / DialogsAndFlyouts / DateAndTime / Media / Scrolling 分组 |
| Rin-UI 主题 | `RinUI\themes\{dark,light}.qml` | `colors` + `appearance`（圆角/边宽/动画时长），已映射进 ZephyraUI `tokens.rs` |
| Rin-UI 排版 | `RinUI\utils\Typography.qml` | 九档枚举 |
| Rin-UI 窗口层 | `RinUI\windows\` | `FluentWindow` / `TitleBar` / `NavigationView` / `FluentPage` / `CtrlBtn` / `WindowManager` |
| **Rin-UI 的 Windows 专属实现** | `RinUI\core\theme.py` | `DwmSetWindowAttribute`：`DWMWA_USE_IMMERSIVE_DARK_MODE(20)` / `DWMWA_SYSTEMBACKDROP_TYPE(38)` / `DWMWA_MICA_EFFECT(1029)` / `DWMWA_CAPTION_COLOR(35)` / `DWMWA_BORDER_COLOR(34)` / `DWMWA_WINDOW_CORNER_PREFERENCE(33)`；Win10 走 `SetWindowCompositionAttribute(WCA_ACCENT_POLICY=19)`；`is_win10()/is_win11()/is_win11_22h2()` 分版本判定 |
| 现有 zephyra demo | `E:\ZephyraUI\examples\demo.rs` | **必须保留的测试功能**：① 矢量图标行 ② uv 裁剪展台 ③ 滤镜/混合模式 29 种展台 ④ hover/press 状态行 ⑤ `--bench` 分阶段计时 ⑥ `--alloc-probe` 全局分配器探针 ⑦ `demo_tests.rs` 10 条交互回归 |
| 现有 UI 层 | `E:\ZephyraUI\ZephyraUI\` | 45 个控件函数 + `ui.zstyle` 341 规则 + 12 条门禁 + gallery（10 路由） |

---

## 1. 架构决策（先定，再动手）

### D1 Windows 优化与通用代码**物理分离**

```
zephyra/zephyra-base/
  zephyra-platform/          ← 新增：**平台能力抽象层**（不含任何 Windows 代码）
    src/lib.rs              Platform trait + Capabilities + BackdropEffect + 能力探测
    src/stub.rs             非 Windows 的空实现（Linux 适配时只改这一个文件）
  zephyra-platform-win/      ← 新增：**Windows 实现**（DWM/版本探测/主题同步）
    src/lib.rs              #[cfg(windows)] DwmSetWindowAttribute 等
    src/version.rs          is_win10 / is_win11 / is_win11_22h2（用 RtlGetVersion，不是 manifest）
    src/theme_sync.rs       系统深浅色监听（对应 Rin-UI 的 ThemeListener）
  zephyra-winit-d3d12/       ← 已存在，Windows 专属，但**只管 GPU 渲染**
```

规则：
- `zephyra-core` / `zephyra-fx` **一行 Win32 代码都不加**；
- 应用只依赖 `zephyra-platform` 的 trait；`win` 与 `stub` 由 feature 选择；
- Windows 上才有的效果（ Mica/Acrylic/圆角/暗色标题栏）在**通用层是"能力位"**，
  平台不支持时上层自动降级 —— 这就是 Linux 适配要接的地方。

### D2 Demo 应用形态

在 `ZephyraUI/` 下新增 `app2/`（gallery 的**重写版**，与现有 `app/` 并存直到验收通过）：
- 路由表由 `ItemData.qml` 的 44 条**移植**过来（title / type / desc / page / added）；
- 分类页 18 个（照抄 Rin-UI 的 `pages/*.qml` 结构）；
- 控件页 42 个（照抄 `pages/controls/*.qml`）—— 能迁的用 ZephyraUI 控件重建，
  依赖 core 能力的（Flyout/Popup/媒体）**照原版标"Demo 级降级"并写明**；
- 搜索框按 title 过滤后跳页（照抄 `gallery.qml` 的 `onAccepted`）；
- **测试功能**作为独立分区/页面挂在 gallery 里（图标行 / uv 裁剪 / 滤镜混合 / 状态行），
  加上 `--bench` / `--alloc-probe` 两个 CLI。

### D3 三层验收口径（沿用现有）

1. **门禁**（`tests/gates.rs`）—— 现有 12 条自动覆盖新页面；
2. **`--dump-cpu --route`** 逐路由出图（十一个路由 → 会涨到 ~60 个）；
3. **窗口路真机点击**（PowerShell DPI-aware 脚本 + 截图 + trace）。

---

## 2. 任务清单（批 → 勾）

### 批 0：规划与文档基座 ✅ 本文件
- [x] 盘点 Rin-UI demo / 组件 / 主题 / Windows 专属实现
- [x] 盘点现有 demo 的测试功能（7 项，全部要保留）
- [x] 定架构（D1 平台层分离 / D2 应用形态 / D3 验收口径）

### 批 1：平台抽象层（Windows 优化的**容器**） ✅
- [x] 1.1 `zephyra-platform`：trait + capability 位 + stub 实现（`#![forbid(unsafe_code)]`）
- [x] 1.2 查文档 → `docs/win/01-window-attributes.md`
      （DWM 属性 20/33/34/35/36/38 的**确切值与版本门槛**；与 Rin-UI 的两处出入已记）
- [x] 1.3 `zephyra-platform-win`：`DwmSetWindowAttribute` 封装 + `RtlGetVersion`
- [x] 1.4 单测：16 条（6 + 10）全绿
- [x] 门禁：`cargo test -p zephyra-platform -p zephyra-platform-win`

> **查文档的三处硬收获**（都不该凭印象写）：
> 1. `RtlGetVersion` 在 `windows` crate 里**根本没有绑定**（`Win32` 与 `Wdk` 都没有），
>    必须自己声明 extern —— 而 `GetVersionExW` 因 manifest lie 会给 6.2，
>    导致所有 `build >= 22000` 门槛静默失效；
> 2. `DwmSetWindowAttribute` 的属性参数是**强类型 newtype**，不是 `u32` ——
>    写错数字会编译不过，而不是静默失效；
> 3. Mica 的官方路线**要求 Windows App SDK**，本仓不引入 ⇒ 明确不做并记账。

### 批 2：Windows 窗口优化（第一批，只做查过文档的） ✅
- [x] 2.1 查文档 → `docs/win/01-window-attributes.md`（暗色模式门槛 Build 22000）
- [x] 2.2 查文档 → `docs/win/01-window-attributes.md`（圆角四档 `DWMWCP_*`）
- [x] 2.3 宿主接线：`HostWindow::apply_window_chrome`（句柄转换 + 降级收敛）
- [x] 2.4 trace：`[trace] 窗口外观 {请求} -> {逐项结果}`
- [x] 门禁：**真机验证通过** —— trace `dark=true corner=true`，截图确认暗色标题栏 + 圆角
- [x] 应用侧零 Win32 代码（`gallery.rs` 只调 `apply_window_chrome`）

### 批 3：主题同步（系统深浅色 → 应用主题） ✅（只读，不做通知）
- [x] 3.1 查文档 → `docs/win/04-sys-theme.md`
      （结论：**注册表那两个值没有官方文档**，属实现细节；本轮只读不通知）
- [x] 3.2 Windows 实现：只读查询 `AppsUseLightTheme`，读不到返回 `None`（**不猜**）
- [x] 3.3 stub 实现：恒为 `None`（不跟随系统）
- [x] 门禁：查询结果与注册表一致或读不到；能力位与实际行为一致；热路径无副作用
- [ ] （不做）变更通知 —— 理由与时机记在 `docs/win/04-sys-theme.md` §3

### 批 2b：Mica / Acrylic 背景材质 ✅（**更正上一轮的错误结论**）
- [x] 更正：上一轮写「Mica 做不了，官方路线要求 WinAppSDK」——**错了**。
      那篇 `apply-mica-win32` 讲的是 WinAppSDK 路线；同一份文档的
      `DWMWINDOWATTRIBUTE` 页面明写 `DWMWA_SYSTEMBACKDROP_TYPE`(=38) 可用于
      `DwmSetWindowAttribute`，Build 22621+。**纯 Win32 即可。**
- [x] 查文档 → `docs/win/02-mica.md`（`DWM_SYSTEMBACKDROP_TYPE` 五个取值、
      版本门槛、"might change with future Windows releases" 的措辞含义）
- [x] 实现：`BackdropKind::{None,Mica,MicaAlt,Acrylic}` → `DWMSBT_*`；
      门槛 22621（比 dark mode 高一档）
- [x] **可见性前提**：材质画在窗口背后，必须 clear 半透明 + 根背景留透明。
      宿主 clear 在 `backdrop_active` 时改 `with_alpha(0.0)`；
      应用发 `.mica` 分类名 → `Container.titlebar.mica` 半透明（内容区保持不透明）
- [x] `BackdropKind::None` 映射 `DWMSBT_NONE`（**不能**用 `DWMSBT_AUTO` ——
      文档说 AUTO 由 DWM 决定、可能仍画材质 ⇒「关掉 Mica」关不掉）
- [x] 门禁 2 条：`the_mica_titlebar_token_is_actually_translucent`（半透明令牌
      真的半透明 + RGB 与实色一致）、`the_app_always_asks_for_the_mica_look`
      （应用无条件表达意图，平台层回答能力）
- [x] **真机像素验收**：trace `backdrop=true`，标题栏像素 25,27,29 vs 材质
      tint 处 53,57,60 —— 材质真的透出来了（API 返回成功**不能**证明可见，
      见 `docs/win/02` §4）

### 批 4：Demo 数据层（ItemData 移植） ✅
- [x] 4.1 `ItemData.qml` 的 44 条 → Rust 常量表（title / type / desc / added / page）
      （**脚本逐块解析** QML 后生成 `catalog.rs`，不手抄；44 块零缺字段）
- [x] 4.2 分类映射（14 个 type → 14 个分组 id）+ **原版导航顺序**
- [x] 4.3 门禁 3 条：
      - `catalog_matches_the_rinui_original`（条数 + **逐组计数** + page 唯一）
      - `catalog_search_filters_and_empty_query_returns_all`（含"搜不到必须零命中"
        的负向判据 —— 否则"永远返回全部"的实现能过）
      - `catalog_group_icons_are_registered`（图标按**登记名**，且**不新增图标**）

> 门禁当场抓到两处真错：分组顺序被我写成字母序（原版虽近字母但 **Design**
> 排在末尾）、4 个图标名（`chevron-right`/`layout`/`palette`/`dialog`/`play`/
> `scroll`/`design`/`window`）**不在内置表**。两处都是"界面上看起来还行、
> 点开是空白"的类型，靠肉眼核对很难发现。

### 批 5：外壳重写（FluentWindow + NavigationView + TitleBar） ◐
- [x] 5.1 标题栏：站名 + 搜索框 + 窗口按钮（min/max/close）—— 既有结构已与 Rin-UI 对齐
- [x] 5.2 NavigationView：10 个页面路由 + Settings 置底（保持 —— 加 14 分组会让
      导航超出 752px 高度，而 core 无 flex-grow、无折叠；记为能力边界）
- [x] 5.3 FluentPage：横向 padding 42 / 间距 18（既有几何门禁仍绿）
- [x] 5.4 **`All samples` 改用真 catalog 数据**（44 条，非手抄常量）——
      5 列网格、等高卡片、文字换行、图标按分组
- [ ] 5.5 14 个分组索引页（`group:<id>` 路由）—— 未做，见下方记录

> **5.4 踩到的三处真缺陷**（都靠门禁/出图抓到，值得留档）：
> 1. **把用户可见文案当分类名发出去了** —— `clip_card` 里我写了 `.styles(title)`，
>    于是 "Button" 这些标题被样式表查找，找不到 = 静默失效。门禁
>    `every_class_name_...` 当场抓到。文案必须走 `TEXT` 属性。
> 2. **卡片描述横向溢出糊成一片** —— `Text` 不换行也不裁剪，卡片 200px 宽
>    而描述 40~60 字符。修法是 `text-max-width`（core **已有** `TEXT_MAX_WIDTH`，
>    我一开始漏了）。**单行截断（省略号）core 仍不支持** —— 记能力边界。
> 3. **卡片竖排成一列** —— 44 张卡挤在内容区，实际是每张 200px + 换行后更高，
>    加上我给 clip 网格用了 `Container.row`（`cross-align: center`）导致不等高。
>    修法：clip 网格专用 `clip-grid-row`（`stretch` + spacing 16）。

### 批 6：Home 页重写（照抄） ✅
- [x] 6.1 Banner 高 350（照抄 `Home.qml` 的 `contentHeader height: 350`）；
      渐隐遮罩仍不做（core 无 ShaderEffect/渐变 —— 记账）
- [x] 6.2 三张链接卡（标题改回**原版英文原文**：Getting Started /
      Documentation / RinUI on GitHub）+ Recent/Favorite 两个 PillButton
- [x] 6.3 Recently added 改为 **catalog 的 `added:true` 真实数据**（取前 10 条）
- [x] 6.4 度量对齐：页间距 18（`FluentPage spacing: 18`）、链接卡 300 宽
- [x] 门禁 + dump 出图

> **6.3 的降级理由值得记**：catalog 有 12 条 `added`，5 列排 3 行，而 Home 从
> banner（350）到视口底只放得下两行半 —— 第 3 行掉出视口。
> **溢出不会报错，只是"看不见"**；是 `hit_test_agrees_with_the_painted_rects`
> 门禁发现元素中心"谁都没命中"才暴露的。
> Rin-UI 那边是滚动页，core 无滚动 ⇒ 只能少放几条并写明，而不是假装能滚。
> 这也顺带说明那条门禁的价值：它抓的是**肉眼极难发现**的溢出。

### 批 7：分类页与索引 ✅
- [x] 7.1 All samples：顶部 **14 个分组 chip**（两行横排，点进分组索引页）
      + 下方 44 条 catalog 真实卡片网格
- [x] 7.2 14 个分组索引页（`group:<id>` 路由），列出该组全部控件卡
- [x] 7.3 44 个控件卡点击 → 落到**所属分组**索引页（逐控件独立页是批 8）
- [x] 7.4 路由解析改为**三表查表**（10 页面 + 14 分组 + 44 控件卡），
      不用 `match` 列举 —— 新增 id 不会漏进 `_` 分支静默回 Home
- [x] 门禁：`every_catalog_entry_is_reachable_through_route_resolution`
      （44 条全可达 + 14 分组都有索引路由）
- [x] 门禁：`every_page_builds_a_real_tree`（既有，阈值覆盖新页）

> **路由解析用查表而非 `match`**：枚举式里新增路由会漏进 `_` 分支，
> 表现为「点那张卡没反应」——界面完全正常，只是静默失效。
> 这与"图标名按登记名"是同一类错误：**写错不报错，只是那一条不生效**。
> 分组 chip 放All 页而非侧栏：core 无滚动/折叠，侧栏放不下 14 项（752px 不够），
> 放All 页是诚实降级，记在 §能力边界。

### 批 8：控件页 × 44（照抄 `pages/controls/*.qml`） ✅
- [x] 8.1 新模块 `app/showcase.rs`：**每条 catalog 条目一个展台页**（44 页），
      与 10 个大页面分开（前者跟控件库演进，后者跟布局）
- [x] 8.2 38 个用已有控件重建（真实控件实例 + 原版 desc 英文原文）
- [x] 8.3 6 个**显式降级页**（Flyout / Popup / CalendarView / CalendarDatePicker /
      TitleBar / Iconography）：页面上**明写能力边界**，且带一个真按钮证明
      页面是活的 —— 降级 ≠ 空壳
- [x] 8.4 门禁 `every_catalog_entry_has_a_non_empty_showcase_page`
      （节点数下界 + **至少一个带 TEXT/ICON 的叶子**）
- [x] 8.5 **`--route` 改为走 `resolve_route`，未知路由直接报错退出**
      （原来 10 个字面量 match → 其余静默回 home）

> **批 8 抓到一个我自己写下的教训当场复发**：逐路由出图 22 张，
> **md5 全同** —— 因为 `--route` 是白名单 `match`，`--route Button` 静默回 home。
> 而"跑过了 22 个路由"看起来完全正常。
> 这正是 MIGRATION §8 写的「必须核对产物互不相同」—— 规则写得对，
> 但我只用在**上一批**，没用在**这一批**。修法：路由解析走唯一真源
> （`resolve_route`），未知路由**退出码 2**，并加 `--list-routes` 让拼错立刻可见。
> 教训升级为：**每批出图都要核对 md5**，不是"知道要核对"。

### 批 9：测试功能并入（**保留原 demo 的 7 项**） ✅
- [x] 9.1 矢量图标行 → `testbed` 页（8 个内置图标，验证「图标名 → 三角化 → 绘制」）
- [x] 9.2 uv 裁剪展台 → `testbed` 页（full / right / br / **overflow** 四种 src）
- [x] 9.3 滤镜 + 混合模式展台 → `testbed` 页（6 个样张，含 Multiply/Screen 对比）
- [x] 9.4 hover / press 状态行 → `testbed` 页常驻
- [x] 9.5 `--bench`（分阶段计时）→ 逐字照抄 `examples/demo.rs::bench_cpu`
- [x] 9.6 `--alloc-probe` → `alloc_probe.rs` 复制进 gallery（自带全局分配器）
- [x] 9.7 `demo_tests.rs` 的 **10 条**交互回归 → `tests/interaction_tests.rs`（10/10 绿）
- [x] 9.8 真机验证：`--bench` 120 帧（阶段占比 100.0%）、`--alloc-probe`
      每帧 1091 次分配 / 133KB + 直方图

> **9.7 的两处设计差异，都值得记**：
> 1. **坐标不硬算**，用 `center_of` 反查控件矩形。demo 那份硬算了"工具栏居中
>    的 y"，布局一改就悄悄点空 —— 而 `hit_test` 反查永远对准。
> 2. **`frame()` 的顺序是契约**：diff → **apply** → layout → render。我第一版把
>    apply 写在 diff **前面**，于是"hover 变了但画面没变"，被 `hovering_a_button_
>    changes_the_display_list` 当场抓到。这条契约在 `apply.rs` 顶部有注释，
>    但**我写夹具时没读** —— 复现了框架自己的一个坑。
> 3. 迁移后 `NAV_SPACER` 从 289 → 248（导航多了一项 `testbed`，Settings 被推出
>    视口，y=837 > 800）。几何门禁 `shell_layout_geometry` 当场报出 —— 那条门禁
>    本来是为"改几何常量"写的，这次它拦下的是"加了页面没改常量"。

### 批 10：Windows 优化第二批（GPU / 呈现） ✅（只补文档，代码已满足）
- [x] 10.1 `docs/win/05-flip-model.md` —— flip model 的**缓冲区轮转**（已查）
- [x] 10.2 `docs/win/06-swap-effects.md` —— `DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL`
      明写「Present 之后后缓冲内容被丢弃」+ **MSAA 不受 flip model 支持**
      （`SampleDesc.Count` 必须为 1）⇒ 本仓 MSAA 必须落在自有中间纹理上
- [ ] 10.3 DComp target / DPI 切换文档 —— **未做**（本轮未改动该路径，
      无从查起；将来动 resize/DPI 时必须先查）
- [x] 10.4 结论写入两后端代码注释（`render_pass` / `acquire_and_render`）

### 批 11：Linux 适配预留（**只留口，不实现**） ✅
- [x] 11.1 `zephyra-platform::stub` 覆盖全部 trait 方法
- [x] 11.2 `docs/portability.md`：逐条列出能力位与 Linux 对应（含**有规范**的
      `org.freedesktop.appearance`，比 Windows 的注册表值可靠）
- [x] 11.3 `cargo check -p zephyra-ui --no-default-features` 通过

### 批 12：收尾 ✅
- [x] 12.1 全量测试 + 零告警（见下方验收数字）
- [x] 12.2 逐路由出图，**核对 md5 互不相同**（22 张全不同）
- [x] 12.3 窗口路真机：暗色标题栏 + 圆角 + Mica 像素级验证
- [x] 12.4 README + MIGRATION 更新

---

## 3. Windows 文档清单（**写代码前必须先查**）

| # | 主题 | 要查清的问题 |
|---|---|---|
| 01 | DWM 窗口属性 | 20/33/34/35/36/38 的确切语义、支持版本、取值 |
| 02 | 暗色模式标题栏 | 1809 的 `20` 与 Win11 的差异；`SetWindowCompositionAttribute` 何时必须 |
| 03 | 圆角 | `DWMWCP_*` 四档与 Win11 vs Win10 的行为差异 |
| 04 | 系统主题 | 深浅色的权威来源与变更通知途径 |
| 05 | flip model | Present 之后内容失效；何时必须整帧重绘 |
| 06 | DirectComposition | target/visual 的建立顺序、resize 与 DPI 切换 |
| 07 | MSAA | 多重采样 RT 与 resolve 的合法性 |

每份文档格式固定：**问题 → 出处（MS 文档名 + 链接）→ 结论 → 对本仓的影响**。

---

## 4. 完成口径

- 12 批全勾；
- `cargo test --workspace` 全绿、`cargo check --workspace --all-targets` 零告警；
- 门禁覆盖新外壳与全部页面；
- 原 demo 的 7 项测试功能一条不少地并入；
- Windows 优化每一条都有文档出处，且**不与通用代码混写**；
- `--no-default-features` 能编过（Linux 留口成立）。

---

## 5. 风险与对策

| 风险 | 表现 | 对策 |
|---|---|---|
| Rin-UI 依赖 core 没有的能力 | 42 个控件页做不出来 | 照原版做 Demo 级降级并在页面写明；不假装能做 |
| 60 个页面导致 dump 验收量过大 | 出图核不过来 | 批处理核对产物互不相同 + 按批验收，每批 5~8 页 |
| Win32 API 凭印象写 | 运行时不生效或崩溃 | 批 1/2/10 各有「先查文档」的强制步骤，写进 `docs/win/` |
| 新旧 demo 并存期互相干扰 | 测试读错树 | 新 demo 独立 crate，`tests/` 分开；验收后删旧 demo 的应用层（保留其测试功能） |
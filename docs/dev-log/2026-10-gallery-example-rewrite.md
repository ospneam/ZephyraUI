# gallery 重写：从「控件展柜」到「开发者示例项目」

> 现象：gallery 的定位是错的。它是 Rin-UI 翻版的演进结果——11 条路由按控件
> 族罗列（basics 页同时摆 button×4、pill、text_field、combo、spin、slider、
> color_picker），组织轴是「覆盖面」。用户一句话点破：「这个 gallery 不是
> 用来调试框架的，而是给开发者的示例项目。」

设计文档（章目、几何账、门禁映射）见
[`docs/developer/gallery-example-project.md`](../developer/gallery-example-project.md)。
本文只记**排查**与**排除的假设**。

---

## 排查

### 1. 两层结构在导航栏上装不下——第一次方案当场作废

最初方案是「导航栏两层平铺」：6 章指南 + 9 族参考 + 分组标题。算账：

```text
6×46（指南）+ 1×46（参考）+ 9×48（子项）+ 46（Settings）= 800 > 756
```

导航栏高 756，超出的项唯一的表现是**下沿被吃掉**——画不出来也点不到，
且不报错。`hit_test_agrees_with_the_painted_rects` 防的就是这类 bug，
只不过它量的是内容卡，导航栏自己就是几何来源。

于是参考层收进**一页**，页内 segmented 切族。这引出第二个坑：

### 2. `nav_sub_item` 不折叠——方案 A 被几何否决

为「参考层 9 族」找导航形态时验证了 `nav_sub_item`：它**只是缩进显示的
常驻项**（高 44px，比一级项还高 4px），没有任何折叠语义。九个子项常驻 =
432px，加上指南六章照样爆。用户拍板前把这个发现摆了出来，改用方案 D
（页内 segmented）。

### 3. segmented 的段 key 必须带 `ref:` 前缀——分派静默失败

`controls::segmented` 的段 id 就是节点的 key，而 `Gallery::click` 的查表按
`strip_prefix("ref:")` 解析。第一版 items 发的是裸名 `basics`——
门禁 `reference_family_keys_all_wired` 立刻红：「参考页上找不到族切换按钮
`ref:basics`」。**编译过、画面正常、点击无效**——与
「`--route` 静默回首页」是同一类失效。

### 4. 嵌入的族页面在参考页里横排了

第一版 `family_content` 直接嵌各族 `page()` 的返回值，出图后
`hit_test_agrees_with_the_painted_rects` 红：`combo-material` 落在
x=1182（宽 220，右边 1402 > 1400）。内容被**横向**推到了右边缘——
嵌入的容器在参考页里没有拿到竖排语义。后续修法是给参考页的族内容
一个显式竖排的宿主容器（`.page-reference` 本身就是竖排，族页面自带的
`.page-basics` 等分类名也在样式表里保留）。

### 5. 交互回归测试找不到控件

`interaction_tests.rs` 的 `Harness::new` 写死 `route = "basics"`。basics
不再是路由后，7 条测试一起红：「gallery 里找不到 key btn-standard」。
改成 `route = "reference"` + `ref_family = 0`——控件没变，只是换了住址。

---

## 排除的假设

### 「二级导航能省空间」

否。`nav_sub_item` 不折叠（见上），九个子项常驻显示，几何上**比一级项更占**
（44 vs 40）。它省的只是「导航栏里少一个一级项」，代价是九个常驻子项——
总高度只增不减。

### 「参考层一页内九节平铺，压一压就行」

否。九节 × 每节 150px ≈ 1350，内容区只有 756。压缩到两列每节 2 行后，
每族只剩 2 个控件——参考层失去查阅价值。**每族独立一屏**是唯一既保住
内容又不超几何的形态，所以切换器（segmented）必须是页内的。

### 「segment_keys_are_all_wired 改改就能用」

否。它钉的是 `seg:grid` / `seg:list` / `seg:card`（toggles 页的分段），
那些段随 toggles 路由一起降级成了参考层内容。新分段是九族的
`ref:*` key，语义完全不同——硬改注释不如重写一条
（`reference_family_keys_all_wired`），并且新门禁**真的点一遍**回读
`ref_family`，而旧门禁只查 key 在树上。

### 「glass 门禁的 effects 路由引用可以留着」

否。`laid_out("effects")` 在新路由表里查不到，`resolve_route` 返回 None，
静默回落——玻璃门禁会因「树里没有玻璃」而恒真地绿。所以加了
`laid_out_ref(family)` helper（reference 页 + 指定族），四条 glass 门禁
全部改走它。

---

## 结论

- 路由 11 → 8：指南 6 章 + `reference` + `settings`（置底）。
- 参考层 9 族 = `reference` 页内 segmented 切换，`REF_FAMILIES` 是唯一真相。
- 旧 `home` 页（hero 横幅 + 3×3 分类入口）删除，被 `guide` 章取代；
  其样式规则（`Container.hero` / `Text.hero-*` / `Control.button.category` /
  `Container.category-row`）整块删除——由死规则门禁抓出来。
- 门禁 30 → 31：删 `segment_keys_are_all_wired`，加
  `reference_family_keys_all_wired` / `reference_page_has_all_nine_families`。
  其余 29 条防坑断言只改遍历基础。

## 门禁

| 门禁 | 钉住什么 |
|---|---|
| `reference_family_keys_all_wired` | 九族 segmented key 都在树上，且点了真的改 `ref_family` |
| `reference_page_has_all_nine_families` | 每族树里都有该族的特征控件（防某族内容没接进参考页） |
| `shell_metrics_match_stylesheet` / `shell_layout_geometry` | 导航项数变了 spacer 必须同步（10→8 项时 226→364） |
| `every_stylesheet_rule_matches_something` | home 页删除后其样式规则成为死规则，当场抓出 |

## 验收

- `cargo test -p zephyra-gallery` —— **51 passed / 0 failed**（31 gates +
  7 anim + 3 hover + 10 interaction）
- `cargo clippy -p zephyra-gallery --all-targets -- -D warnings` —— 0 警告
- `cargo fmt --all -- --check` —— 干净
- `--list-routes` —— 8 条；`--dump-cpu` 逐路由出图 **8/8 MD5 互不相同**
  （旧结构曾出过「20 张图全同」的事故，这条核对是 `main.rs` 注释里的要求）
- 窗口路（D3D12）reference 页 `quads=488`、`unimplemented_filters=0`

## 未做

- 指南层六章的**深度**：每章目前是一个示例 + 说明，还没有「可抄的源码片段」
  展示区（core 没有代码块渲染，只能放文本标签——形态待定）。
- `testing` 章列的命令与实际实现之间的**自动对账**（现在靠人核对；
  `every_documented_command_is_real` 门禁在设计中提过，未实现）。
- i18n 覆盖：新页面的正文说明仍是英文硬编码（与旧页面的债同源，记账在
  `docs/developer/i18n.md` §5）。

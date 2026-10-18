# gallery 重写：门禁全绿而界面是错的

日期：2026-10

## 现象

重写 gallery 的信息架构（70 路由 → 11 路由）之后，25 条树级门禁**全部通过**，
但截图上 `settings` 那一页的**正文是首页**：

- 标题写着「Settings」，
- 侧栏正确高亮 Settings，
- 正文却是首页的横幅 + 活样例 + 分类入口。

同时另外两处「看得见但不对」：

1. 首页最后三行分类入口点不到（掉到窗口外面去了）。
2. 液态玻璃页的三块玻璃**整块消失**，只剩三个标题。

三处都不报错、不崩、无告警。

## 排查

### 一、`settings` 渲染成首页

`pages::page()` 的兜底是 `home::page(g)`：

```rust
match PAGES.iter().find(|p| p.id == g.route) {
    Some(p) => (p.build)(g),
    None => home::page(g),
}
```

第一版把 Settings 排除在 `PAGES` 之外（理由是「它由 shell 置底，不在 `ROUTES` 里」）。
于是 `settings` 查不到构造器 → 静默回落首页。

**排除的假设**：

| 假设 | 为什么不成立 |
|---|---|
| `resolve_route` 漏了 settings，所以路由没切过去 | 不是。侧栏高亮跟着 `route` 走，`route` 是对的；错的只是**页面分发** |
| `page-settings.zstyle` 没被加载，样式全丢 | 不是。正文带的是 `.hero` / `.category` 这些**首页**的分类名，不是「没样式」 |
| 路由字符串两处不一致 | 不是。`click()` 与 `view()` 读的是同一个 `Gallery::route` |

**为什么 25 条门禁一条都没抓到**：

- `every_page_builds_a_real_tree` —— 首页的树足够大，**过**。它量的是「非空」，不是「对的」。
- `hit_test_agrees_with_the_painted_rects` —— 首页的每个节点都在客户端内且可命中，**过**。
- `every_class_name_the_source_emits_exists_in_the_stylesheet` —— 首页的分类名全都有规则，**过**。
- `interaction_leaves_no_stale_appearance` —— 暖树与冷启动树一致，**过**。

它们共同的盲区是：**没有任何一条问「这一页是不是它自己那一页」**。

**结论 + 门禁**：`PAGES` 收进 Settings（它是「能构造的页面」，与「导航项」不是一回事）；
新增门禁 `every_route_renders_its_own_page` —— 逐路由找「key == 路由 id **且** 带
`.page-<id>`」的节点，找不到就红。

> 判据为什么不能只比 `key == 路由 id`：侧栏的导航项与页根**同 key**（「home」
> 既是路由 id 也是导航项 key），深度优先先撞上的一定是导航项。第一版就是这么
> 写的，结果这条门禁在**正确的**代码上报红。

### 二、首页分类入口掉出客户区

第一版把 9 个分类入口放进一个竖排容器（core 的容器**默认横排**，但 `.section-body`
给了 `direction: vertical`）。core **没有折行语义**，于是 9 × 40 = 360 像素
垂直堆叠，而内容卡扣掉页头与横幅后只剩 400 出头 —— 最后三行落在 800px 之外。

core 既不滚动也不裁剪溢出，所以「一屏装不下」唯一的表现就是**下沿被吃掉**：
节点在树里、可交互、样式齐全，只是命中测试按定义不会命中它们。

**排除的假设**：

| 假设 | 为什么不成立 |
|---|---|
| 分类按钮的 key 拼错了 | 不是。门禁 `every_class_name_…` 与命中门禁都指出它们**存在**且**不可命中** |
| 内容卡高度写错了 | 不是。`shell_layout_geometry` 量出来是 756，与常量一致 |
| 这是旧结构就有的问题 | 不是。旧首页的卡片网格是 5 列，横向不溢出 |

**结论 + 门禁**：改成 3×3 排布；把 `hit_test_agrees_with_the_painted_rects`
从「只跑 home」改成「跑全部路由」，并**先单独报「矩形掉出客户区」**。

### 三、玻璃块整块消失

`Container.glass-pane` 没有子节点，而 core 的容器**不给宽度就按内容测量** ——
空容器量出来是 0 宽，于是这块玻璃在画面上根本不存在。

**排除的假设**：

| 假设 | 为什么不成立 |
|---|---|
| `filter: refraction(...)` 语法没被 zstylec 认 | 不是。整张表能解析（否则 `--dump-cpu` 会报「skipping it」）；`filter` 在 32 个属性里 |
| 玻璃半透明底色与深色底板太接近 | 部分成立，但那只会让块**变淡**，不会让它**消失**。采样像素证明块的位置是纯底板色 |

**结论**：`Container.glass-pane` 写死宽度。

## 顺手记账的三条框架约束

这一轮踩到的，都不是 bug 而是**既有边界**，写在这里免得下一个人再踩：

| 约束 | 后果 |
|---|---|
| `zstylec` 的属性表只有 32 项，**没有** `min-height` / `margin-*` | 想给容器留白只能靠 `height` + `padding`，或 `translate-y` |
| `padding` **只吃单值** | `padding: 10px 16px` 解析失败；横向内边距只能靠控件自己的 `width` |
| `rgba(r,g,b,a)` **不支持** | 用 `#rrggbbaa` 八位十六进制 |

## 门禁

| 门禁 | 钉住什么 |
|---|---|
| `every_route_renders_its_own_page` | 每个路由渲染的就是它自己那一页（防 `pages::page()` 兜底） |
| `hit_test_agrees_with_the_painted_rects` | 全部路由的每个可交互节点：矩形在客户区内，且中心命中自己或自己子树里的节点 |
| `every_class_name_the_source_emits_exists_in_the_stylesheet` | 分类名漂移（本轮它抓到了 `.glass-grid` → `.glass-panes` 的一次改名漏改） |
| `route_table_and_page_table_agree` | `ROUTES + settings` 与 `PAGES` 的 id 集合与顺序一致 |
| `every_page_carries_its_own_controls` | 每一页除了外壳（13 个可交互节点）之外还有自己的控件 |
| `every_route_id_is_translated_in_every_supported_language` | 三门语言的路由文案都齐（漏翻只回落到 `route.unknown` 这个合法字符串，不崩） |
| `segment_keys_are_all_wired` | 分段控件的段 id 与 `click()` 的分支一一对应 |
| `shell_layout_geometry` | 置底的 Settings 落在导航栏底部附近（改路由数量时 `NAV_SPACER` 必须重算） |

## 验收

- `cargo fmt --all -- --check` —— 干净
- `cargo clippy -p zephyra-gallery -p zephyra-ui --all-targets -- -D warnings` —— 0 警告
- `cargo test -p zephyra-gallery` —— 45 passed / 0 failed（25 门禁 + 7 动画 + 3 hover + 10 交互）
- `cargo test --workspace --no-fail-fast` —— 799 passed / 1 failed / 22 ignored
  （唯一失败是 `zephyra-winit` 的 `backend_parity::every_blend_mode_looks_the_same_on_both_backends`，
  本轮之前就红，与本次无关）
- `cargo doc --no-deps --workspace` —— 34 条警告，**全部来自 `zephyra-core/src/geom/tessellate.rs`**
  的既有链接；`zephyra-gallery` / `zephyra-ui` 零警告
- `--dump-cpu` 11 条路由全通，四边形 239–428、缺失纹理 0、跳出 0
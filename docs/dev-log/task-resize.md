# 任务清单：resize 深度优化

> 参照 `E:\microsoft-ui-xaml` 的布局引擎源码重做 resize 路径。
> 官方依据在每条下面逐条给出（文件:行号）。

---

## 0. 先查官方，再动手

子代理把 `dxaml\xcp\core\` 摸了一遍，与本仓直接相关的五条：

| 官方做法 | 位置 | 本仓当时 |
|---|---|---|
| 堆叠轴给子项**无穷**约束 | `core/core/elements/StackPanel.cpp:24-30` | 给的是父的有限约束 |
| 测量缓存按**约束精确匹配**且**跨帧存活** | `core/core/elements/uielement.cpp:4223-4226` | 缓存存在，但被 `clear_dirty` 每帧清掉 |
| measure / arrange 是两趟、两套脏标记 | `inc/uielement.h:1368-1402` | 一个 `Dirty` 序数，合在一起 |
| arrange 在 final rect 未变时短路 | `core/core/elements/uielement.cpp:4542-4546` | 无短路 |
| 集合变化由集合自己 invalidate 父 | `components/Collection/UIElementCollection.cpp:451` | diff 不让父失效 |

官方注释里还有一条值得抄的（`StackPanel.cpp:434-437`）：Auto 列要给无穷，
而 Grid 的 `*` 行要给视口 —— 「无穷」在官方是有语义的，不是"忘了给"。

官方自己也踩过同类坑：`core/text/TextBlock/TextBlock.cpp:526-528` 的 TODO 说
`PageNode/BlockNode` 的 measure 旁路判等没有算 padding，会绕过测量。
**连他们都会在缓存键上漏东西** —— 这条记在 `docs/task-resize.md` §5 作为提醒。

---

## 批 A：三处优化

- [x] A1 测量缓存跨帧存活（`clear_dirty` 不再清它）
- [x] A2 堆叠轴给子项无穷约束（`Stack::child_constraints`）
- [x] A3 排版结果进测量缓存（`MeasureResult::shaped`），绘制趟复用
- [x] A4 门禁：稳态 0 次整形 / 只改高度 0 次整形 / 改宽度必须重测
- [x] A5 `--bench-resize` 且**三个方向分开报**

## 批 B：两个真缺陷

- [x] B1 diff 插入/替换子节点时让父失效（原先会 `rects[slot]` 越界 panic）
- [x] B2 `StageTimes` 的 diff/layout/paint 恒为 0 —— 换成诚实的 `cache_us`

## 批 C：C1 已做，其余记账

- [x] C1 resize 期间的分配：两趟都改成**不收集中间 `Vec`**。
      `measure` 一边测一边折叠主轴总和与交叉轴最大；`arrange` 第一趟只读折叠
      总长、第二趟可变算矩形并立即递归。`visible_children` 整个函数删掉 ——
      折叠判断内联进循环，顺带消掉"每节点一个 `Vec<usize>`"。
      规则本体抽成 `Stack::main_offset_rules` / `main_of`，切片版与流式版共用，
      免得 `SpaceBetween` 的单项特例在两处漂移。
- [ ] C1b **`glyph_quads` 的 `Vec<GlyphQuad>`**：实测仍是最大的一笔，
      **239 次 × 1024 B = 136 KB/帧**（每帧字节数的 40%），一个文本节点一次。
      修法与官方一致：把字形矩形按**元素原点**缓存，绘制偏移时再平移 ——
      这也正是 `translate` 属性该有的形状。记账在 §下一步。
- [ ] C2 measure / arrange 拆成两套脏标记（官方的 `LF_MEASURE_*` 与
      `LF_ARRANGE_*` 分开）。本仓一个 `Dirty` 序数，所以"只影响排列"的属性
      也会触发全树重测。要改 `PropertyKey` 增加一个标志位，影响面较大。
- [ ] C3 arrange 在 final rect 未变时短路（官方 `uielement.cpp:4542`）。
- [ ] C4 `IsSameSize` 的无穷比较。官方对 `inf == inf` 特判
      （`uielement.cpp:15236-15241`），本仓 `Constraints` 的 `PartialEq`
      是派生的浮点比较，`f32::INFINITY == f32::INFINITY` 恰好为真，所以
      结论相同但没写下来。补一条注释即可，不改行为。

---

## 下一步

1. **C1b 字形矩形缓存**（最大的一笔字节开销，239 × 1024 B/帧）。
2. **C2 measure/arrange 脏标记分离**（结构性，影响面最大）。
3. C3 / C4 收尾。

---

## 完成情况

批 A、B 全部完成，742 条全绿、零告警，渲染产物逐项未变。

### 实测（`--bench-resize`，release，324 帧）

临时探针（已删）测得三个 run 合计：

| | layout | render |
|---|---|---|
| 修复前 | 18363 µs | 92937 µs |
| 修复后 | 15392 µs | 55636 µs |

render 是 layout 的 5 倍 → 3.6 倍，降 40%。

只改高度中位 513 → **440** µs；只改宽度 649 → **438** µs；宽高都改 → **423** µs。
整帧中位 679 / 612 / 623 → **625 / 625 / 605** µs。

分配探针（第二轮，布局两趟不再收集中间 Vec）：每帧 **1709 → 1451 次**、
343 → 336 KB。剩下的最大一笔是 **239 次 × 1024 B = 136 KB/帧** ——
每个文本节点一次，来自 `glyph_quads` 的 `Vec<GlyphQuad>`（记账见 §批 C）。

### 两轮改动的合计

| | 改前 | 改后 |
|---|---|---|
| 只改高度（layout+diff+paint 中位） | 513 µs | 440 µs |
| 只改宽度 | 649 µs | 438 µs |
| render（324 帧合计） | 92937 µs | 55636 µs |
| 每帧分配 | 1709 | 1451 |

布局两趟改成"一边算一边用"之后，`visible_children` 这个函数整个删掉了 ——
折叠判断内联进循环，顺带消掉每节点一个 `Vec<usize>`。规则本体抽成
`Stack::main_offset_rules` / `main_of`，切片版（`main_offsets`，测试在用）
与布局趟共用，免得 `SpaceBetween` 的单项特例在两处漂移。

### 过程中被自己的门禁抓到的一个缺陷

`layout_cache_tests::稳态完全不重测` 在修复前是红的（每帧 6 次整形）。
它揭出的是**缓存活不过一帧**，而那又揭出 diff 不让父失效 —— 后者一直是被前者
**盖住**的 panic。两个缺陷叠在一起，只修一个必然还有另一个。

### 一个方法论上的收获

最初以为贵的在 measure，于是先修测量缓存。改完再测，**只改高度仍然 504 µs，
一分没降**。插桩才发现 render 是 layout 的 5 倍，而它里面绝大部分是重复排版。

「性能判断必须来自计时，估算 + 论证不算」这条规矩救了这一轮 ——
按直觉继续在 measure 上花时间，真正的瓶颈就会留在原地。
插桩用的是 `AtomicU64` 而不是 `static mut`，因为本 crate
`#![forbid(unsafe_code)]`；那条禁令在编译期就把"为了加计数器而引入未定义行为"
堵死了，于是只能用真正安全的写法。
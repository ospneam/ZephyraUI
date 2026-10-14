# 字形四边形缓存：按元素原点缓存，绘制期平移

`docs/task-resize.md` §批 C · C1b。

---

## 1. 开销来自哪

`text_command` 每帧对每个文本节点调一次 `TextRender::glyph_quads`，它返回一个
**新建的 `Vec<GlyphQuad>`** —— 绝对坐标，原点折进了 `origin` 参数里。

于是：`origin` 每帧都在变（resize 那一趟正是要改摆放矩形），字形的位置**其实一个
像素都没变**，却跟着重算 + 重新分配。

`docs/task-resize.md` 的既有探针记录是：每帧 **239 次 × 1024 B = 136 KB**，
占 resize 帧分配量的 **40%**。

## 2. 官方对应物

`CLayoutStorage` 存的是**相对元素原点**的布局结果，绘制趟叠加变换
（`uielement.cpp` 的 `Measure` / `Arrange` 都不重排文字）。

本仓照的就是这个形状：字形四边形按元素原点缓存，绘制期平移。

## 3. 做法

`Command::Text.glyphs` 从 `Vec<GlyphQuad>` 变成 `Arc<[GlyphQuad]>`，并新增
`offset: Point` 字段携带元素原点：

- `text_command` 用 `glyph_quads(shaped, (0,0), style)` 算出**局部**四边形，
  挂到节点的 `RefCell<Option<Box<GlyphQuadCache>>>` 上；
- 命中时只克隆 `Arc`（引用计数 +1，**零分配**）；
- 矩形随 `offset` 在 `quad::expand` 展开时与整帧 `translate` **一起**加。

**三处必须同步**（少改一处 = 那条路径整体错位，且不报错）：

| 位置 | 职责 |
|---|---|
| `paint.rs::text_command` | 写入缓存 + 填 `offset` |
| `paint.rs::command_content_rect` | 滤镜范围要含 `offset`（`union_of_quads(glyphs, offset)`） |
| `zephyra-fx/quad.rs::expand` | 顶点展开时加 `translate + offset` |

## 4. 失效判据：排版输入**指纹**，不是脏标记

缓存只在两件事上失效，都在使用点就地判定：

1. 排版输入指纹变了（`text` / `TextStyle` / `max_width`）；
2. 图集修订号变了（LRU 淘汰会整体重排 uv）。

`TextRender` 因此补了 `atlas_revision() -> u64`（默认 0，给无图集的替身）。

### 两条被否掉的判据（都真的实现过）

| 判据 | 为什么不行 |
|---|---|
| `mark_dirty` 时清缓存 | 布局脏 ⇒ 字形局部坐标**一个字节都不变**。resize 每帧全树清一遍，命中率恒为 0 |
| 「测量趟这帧有没有重算」 | resize 时约束一变 `measure` 就重跑，产出**内容相同**的 `MeasureResult`（只有 `size` 被 `pick_axis` 夹了一下）。命中率同样恒为 0 |

第二条是先用 `measure_epoch` 计数器实现的，被门禁抓到（`Arc::ptr_eq` 不为真），
才改成指纹版。代码 `node.rs::mark_dirty` 里留了墓碑注释。

## 5. 为什么缓存要装箱

`GlyphQuadCache` 有一百多字节。内联进 `Node` 会把它撑大同等大小：

**实测（`demo --bench-resize-cpu --alloc-probe`，与 `1f76d8e` 同一次运行的对照）：**

| 缓存形态 | 分配次数/帧 | 分配字节/帧 |
|---|---|---|
| 基线 `1f76d8e` | 612 | 98 920 |
| 内联 `GlyphQuadCache` | 576 | 106 904 |
| **装箱 `Box<GlyphQuadCache>`** | **576** | **91 224** |

内联版分配**次数**降了 36（字形 `Vec` 确实没了），但**字节涨了 8%** ——
`Node` 变大让建树时每一次 `Vec<Node>` 重分配都变大。装箱后每个桶都变小，
与基线对照是 **−5.9% 次数 / −7.8% 字节**。

代价：每次 miss 多一次堆分配。miss 极少（实测 100 帧 resize 共 19 次）。

### 静态 bench 上的代价（诚实记账）

同一个缓存在**静态** bench（`--bench`）上是 **+4% 字节**
（53 312 → 55 616），次数不变。

原因是那条路径里文本节点不是每帧 paint-dirty —— `text_command` 整个 120 帧
只跑 **19 次**（对照：resize 帧是 1900 次调用、1881 次命中），缓存无从发挥，
只剩 `Node` 长大的代价。直方图是「同样次数、每次略大」，与 `Node` 长大
8 字节吻合。

## 6. 门禁

`paint_transform_tests.rs::位移时字形四边形不重算` —— 跨帧断言 `Arc::ptr_eq`
为真 + `offset` 恰好移动预期距离。改回 `Vec` 会立刻红。

## 7. 探针方法论（留给下次）

写「前 N 次打印」时把「初值 N」和「条件 `< N`」写在**两处**是错的 ——
改漏一处就一条都不打，而表面上看像「缓存没进到那条路径」。
这轮为此多跑了好几轮。正确写法是单句自限：

```rust
if PROBE_VERBOSE.fetch_add(1, Ordering::Relaxed) < 60 { eprintln!(..) }
```

另外：`PROBE_MISS.fetch_add` 放在 `match` **之后**（无条件和命中路径共用的尾巴）
会把「总调用数」当成「miss 数」报出来 —— 这轮一度以为 50% 命中。
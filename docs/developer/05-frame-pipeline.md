# 帧流水线

一帧里发生什么，以及每段花多少时间。

## 五个阶段

```text
view()  ──▶  Element 树
   │
diff   ──▶  Node 树（保留式）+ DiffStats
   │
layout ──▶  measure / arrange
   │
paint  ──▶  DisplayList
   │
expand ──▶  顶点 / 索引（GPU 侧）
   │
present
```

**这个划分是本仓性能结论的坐标系。** 说「优化了一下」而不说是哪一段，等于没说。

---

## 实测（demo，45 节点，1400×800）

| 阶段 | 耗时 | 干净帧能否省 |
|---|---|---|
| view | 41 µs | ✗ —— 必须跑才知道有没有变（应用代码） |
| diff | 8 µs | ✗ —— 同上 |
| paint | 73 µs | **✓** |
| expand | 40 µs | **✓** |
| **合计** | **162 µs** | 省后 **49 µs** + 一次 present |

省掉后剩下的 49 µs 是**地板**：除非应用自己复用声明树（那是架构改动），否则 `view()`
必须跑才能知道有没有变。

---

## 触发场景

很常见：**鼠标在空白画布上移动**。

宿主照旧置脏、走完 diff，但 `hovered` 前后都是 `None`，`diff` 零产出，于是结果是
`Unchanged`——宿主连 present 都不用做。

---

## 每帧分配

这是另一条独立指标。实测（同一场景）：

| | 优化前 | 优化后 | 降幅 |
|---|---|---|---|
| 每帧分配次数 | 324 | **201** | −38% |
| 每帧分配字节 | 55,624 | **23,072** | −59% |

做了哪几件事：

1. **分类名先比后建**（`diff.rs`）—— 键已经匹配时不再构造新字符串，−65 次。
2. **子节点对账缓冲跨帧复用**（`diff::Scratch` 栈）—— 对账要临时缓冲，让它活过一帧，
   −42 次 / −16 字节。
3. **每帧上传去重**（`renderer.rs`）—— 几何逐字节没变就不重传。`RenderReport` 里有
   `index_uploads` / `index_upload_skipped` 两个计数可观测。
4. **删掉 staging 上两条屏障**—— `GENERIC_READ ↔ COPY_SOURCE` 互斥，两条既非法又没用，
   每帧 −4 条屏障。

> 有两条改法**实测无收益**，没有保留：`apply.rs` 把每节点两个 `Vec` 提到树外（那个
> benchmark 不调 `set_styler`，测不到）、`quad.rs` 的 `Rc<str>` 驻留器（那个场景
> `Command::Image` 数为 0）。**写进注释的每个数字都应该有一次实测背书**，不然它就是
> 装饰。

---

## 怎么自己量

```text
demo --bench                 # 跑 N 帧，报两段耗时
demo --bench --alloc-probe   # 另报每帧分配次数与字节
```

`--alloc-probe` 是**纯 CPU 基准，不走 D3D12**——所以它测不出 GPU 侧改动。GPU 那部分
只能报屏障条数（`RenderReport` 里的计数），别拿 CPU 基准的数去论证 GPU 结论。

---

## 一个必须记住的 GPU 约束

**顶点/索引只有一份 resident 缓冲**，所以 `begin_frame` 每帧要 `wait_gpu()` 全等一次。

这是目前唯一一处未解决的 GPU 性能问题。正解是给 `StreamBuffer` 加环形多缓冲（按帧
轮转，像命令分配器那两槽那样），让第 N+2 帧覆写第 N 帧位置时 GPU 早已读完。

它会动 GPU 资源生命周期语义，要连带验证 resize / recreate 路径——所以在真做过完整
验证之前不写进这个文档。

---

## 相关

- 架构边界 → [总览](01-overview.md)
- 交换链与 flip model → [win/05-flip-model.md](win/05-flip-model.md)
# 核心概念

五件套：`Element` · `property` · `Node` · `diff` · `DisplayList`。

---

## 1. `Element` —— 声明树（可丢弃）

应用每帧**新建**一棵，用完就扔。它描述「我想要什么」，不含任何状态。

```rust
Element::new(NodeTag::Container)
    .with_key("root")        // 身份：diff 靠它认节点
    .style("root")          // 指向样式表里的规则
    .with_children([ ... ])
    .with(&props::TEXT_SIZE, PropertyValue::len(24.0))
```

两种节点身份，**不要混**：

| | 是什么 | 用在哪 |
|---|---|---|
| `NodeTag` | 节点**种类**：`Text` / `Image` / `Container` / `Control` | 样式表里按类型选规则 |
| `ElementKey` | 节点**身份**：`Name(&'static str)` / 动态 id | diff 认「哪个节点变了」 |

`ElementKey` 是 diff 的全部依据。**没给 `with_key` 的节点无法跨帧正确对账** —— 每个
帧它都会被当成一个新节点。

---

## 2. `property` —— 属性系统

三件事：**定义**、**存储**、**按优先级求值**。

```rust
pub static TEXT_SIZE: PropertyKey = PropertyKey::new("Text.Size", PropertyValue::len(16.0));
```

`PropertyKey` 的身份是**地址**（它没有 `PartialEq`，全仓一律 `ptr::eq` 比对）。
所以属性键必须是 `static` 单例，不要每次现造一个。

**变更优先级**（高的赢）：

```text
内联 .with()  >  样式表规则（按 specificity）  >  继承  >  PropertyKey 的默认值
```

---

## 3. `Node` —— 保留树（带状态）

`Element` 丢掉之后，活下来的东西在这里。它有**脏标记**：布局脏、绘制脏。

```rust
node.dirty()          // 要不要重排
node.needs_paint()    // 要不要重绘
```

宿主**不允许**靠读 `dirty()` 来决定跳过 `paint::render` —— 那就是在决定这一帧画什么，
越过了核心与后端的边界。这条边界被破过，代价见
[`../dev-log/2026-10-gpu-device-hung.md`](../dev-log/2026-10-gpu-device-hung.md)。

---

## 4. `diff` —— 对账

拿这一帧的 `Element` 和上一帧留下的 `Node` 逐节点比，产出 `DiffStats`。

关键性质：**只有节点真的变了才有产出**。鼠标在空白画布上移动时 `hovered` 前后都是
`None`，diff 零产出，于是这一帧被判定为 `Unchanged`。

`diff` 本身**不决定**跳不跳过渲染，它只报告。判定在
[`frame.rs`](../../zephyra/zephyra-core/src/frame.rs) 的 `FrameCache` 里做。

---

## 5. `DisplayList` —— 唯一契约

一帧要画的命令序列。核心产出，宿主消费，**中间没有第三种东西**。

命令种类对应几何：

| 命令 | 画什么 |
|---|---|
| 矩形 / 圆角矩形 | 带圆角的实心块 |
| 纹理四边形 | 字形位图、图像 |
| 三角形 | 图标与路径（CPU 三角化后） |
| 滤镜 / 混合模式 | 效果链 |

---

## 帧缓存：为什么它存在

在它出现之前，同一件事的三块状态住在**宿主**里（树的引用、要画的列表、两个 dirty
布尔）。而它们的**数据**全在核心里——宿主拿着 core 的零件自己编排。

现在只有一个地方回答「这一帧画什么」：`FrameCache::frame`。宿主只回答另一个问题：
「我要不要把它推上屏」。

省下来的时间（demo，45 节点，1400×800）：

| 阶段 | 耗时 | 干净帧能否省 |
|---|---|---|
| view | 41 µs | ✗ —— 必须跑才知道有没有变（应用代码） |
| diff | 8 µs | ✗ —— 同上 |
| paint | 73 µs | **✓** |
| expand | 40 µs | **✓** |

162 → 49 µs，外加一次 present。剩下的 49 µs 是地板：除非应用自己复用声明树（那是架构
改动），否则 `view()` 必须跑才能知道有没有变。

---

## 为什么跨帧局部重绘不可行

两个宿主都用 flip model（`DXGI_SWAP_EFFECT_FLIP_DISCARD`，DComp 路径是
`FLIP_SEQUENTIAL`），而 flip model 下 `Present` **之后**后缓冲内容未定义——任何要
present 的帧都必须重画整个目标。

所以 `Damage` **不是**给宿主用来「跳过脏区外的像素」的，那在当前模型下根本不成立。
它只有一个用途：**帧内**限制光栅化范围（scissor），省脏区外的填充率。

Skia 同样没有跨帧的 damage 跟踪（`SkSurface::damage` 全仓零命中）——不是偷懒，是这个
交换链模型的限制。

---

## 下一步

- 写外观 → [样式表](04-stylesheets.md)
- 关心性能 → [帧流水线](05-frame-pipeline.md)
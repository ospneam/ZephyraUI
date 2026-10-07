# ZephyraUI 架构

## 一句话

核心负责**决定画什么**，宿主负责**怎么画**，两者之间只有一个契约：`DisplayList`。

## 为什么是这个切分

UI 框架的历史上有两种切法，各自失败在不同的地方：

| 切法 | 问题 |
| --- | --- |
| 核心与宿主**耦合**（例如核心直接依赖某个窗口框架的类型） | 换宿主 = 重写核心。框架变成某一个应用的附属品，无法独立发布。 |
| 「什么都在核」（核心含窗口/事件/GPU） | 无法测试：`cargo test` 需要真实窗口与 GPU，CI 上跑不起来；也无法在无显示环境跑单测。 |

本框架把切分线画在 `DisplayList` 上：

- 核心 → `DisplayList`：**纯数据**，可序列化、可断言、可比较。
- `DisplayList` → 像素：**宿主**。

于是核心的全部测试都不需要 GPU（本仓 304 条核心断言全部在纯 CPU 上跑，
效果器层另有 64 条，同样不需要 GPU）。

## 数据流

```text
  Element 树   ──diff──▶  Node 树  ──measure/arrange──▶  带几何的 Node 树
   (声明)         (保留)      (状态)                          │
                                                          ▼
                                                   DisplayList
                                            (排序后的绘制命令)
                                                          │
                            ┌─────────────────────────────┴──────────────┐
                            ▼                                            ▼
                    zephyra-winit                              zephyra-winit-d3d12
                      （D3D11）                                    （D3D12）
                            └────────────────┬───────────────────┘
                                             ▼
                                   DisplayList → 像素
```

## 模块与它们各自挡住的坑

| 模块 | 职责 | 它挡住的问题 |
| --- | --- | --- |
| `core` | 几何/排版基元、`Length`、`Color` | 裸 `f32` 混用导致高 DPI 下边框细一半；sRGB 数值当线性用导致叠加发暗 |
| `property` | 属性定义/存储/继承/变更通知 | 控件、主题、样式各写一套失效传播 → 「改了颜色那一块没更新」 |
| `theme` | 设计令牌 + Light/Dark | 上百处硬编码颜色；深色模式不存在 |
| `id` | `NodeTag` / `ElementKey` / `ElementKind` | 身份与类型混在声明树里 → 「谁是谁」说不清 |
| `element` | 声明树（可丢弃） | 声明与状态共用一个类型 → 每帧重建状态 |
| `node` | 保留树 + 脏标记 | 把 `Background` 变更当 `Layout` 处理 → UI 卡顿的头号原因 |
| `diff` | Element→Node 对账 + `DiffStats` | 说不清「这一帧重算了什么」 |
| `context` | 只读上下文（主题 + 文本引擎） | 主题存在节点上 → 切主题要遍历全树 |
| `style` | 属性表 + 上下文 → 具体值 | 各处复制读取逻辑 → 资源键解析不一致 |
| `layout` | Measure/Arrange 协议**与两趟驱动** | 布局与绘制融合 → 无法缓存、无法复用纹理 |
| `paint` | `DisplayList` 契约**与降级驱动** | 核心与宿主纠缠 |
| `text` | 测量/排版抽象 | 字形整形是最不可能自己重做的部分 |
| `hit` | 命中测试 | 与布局各算一套几何 → 「看得见但点不到」 |
| `input` | 指针词汇表、手势、视口、输入路由 | 每个宿主重写一遍手势识别 → 重写就意味着重错 |
| `icon` | SVG path 解析 + 图标注册表 | 每个应用重写弧展开 → 图标形状「略微」不对，肉眼难辨 |

### 层内数据流

同一层内依赖单向，前一个模块的产出是后一个的输入：

```text
Element ──diff──▶ Node ──style──▶ 具体值 ──layout──▶ 带几何的 Node ──paint──▶ DisplayList
(声明)           (保留)                          (测量/排列)                (降级)
                  ▲
                  └── id：身份被 Element 与 Node 共用，所以独立成模块
```

`Context` 横切在 `style` / `layout` / `paint` 之间：它是这些阶段唯一的外部输入，
不参与任何一个阶段的数据变换。

### 三层结构

本框架分三层，依赖方向单向，上层不知道下层存在：

```text
控件层（未实现）        Button / Slider / ToggleSwitch …
   ↓ 构造 Element、设属性
通用层 crates/zephyra   property / theme / tree / layout / paint / filter / hit / input / icon
   ↓ DisplayList（纯数据契约，可序列化可断言）
效果器层 crates/zephyra-fx   顶点展开 / 字形图集 / CPU 光栅 / 资源复用策略
   ↓ 顶点流 + 中立的资源需求
base crates/base        D3D11 / D3D12 / DComp / 摄像头
   ↓
操作系统                 Win32 / DirectComposition / Wayland / X11 / v4l2 …
```

上表「模块与它们各自挡住的坑」描述的是通用层内部。下面这条是效果器层的边界判断。

### 效果器层为什么与 base 并列

判据：**能不能在没有 GPU、没有窗口的环境里写单测**。

- 顶点展开、字形图集打包、CPU 参考光栅、离屏边界计算 —— 全都能纯 CPU 断言，
  所以进效果器层；
- 建纹理、设混合状态、走 DXGI 提交 —— 必须与系统打交道，所以留在 base。

这与 Skia 把 `src/effects` 放在与 `src/core` **并列**处是同一个判断
（`include/effects` 依赖 `//src/core`，而 `include/core` 零引用 effects）。

效果**描述**是纯数据（可进 core 与效果器层），效果**执行**需要 GPU（留 base）。
Skia 从不把执行放core —— `Blur` 在 core 只是描述，真正跑 pass 的在
`src/gpu/ganesh/ops/`。

### base 层：凡是要问操作系统才能回答的事

`crates/base/` 回答窗口怎么开、事件怎么收、GPU 设备从哪来、最后怎么上屏。
它**不**回答「画什么」。

三条契约（详见 [`crates/base/README.md`](../crates/base/README.md)）：

1. **消费 `DisplayList`**，而不是产出它 —— 核心从不 import 任何宿主类型；
2. **不得决定画什么** —— 换 base 实现后同一棵元素树必须产出逐字节相同的
   `DisplayList`；
3. **不得引回控件层** —— 只依赖通用层，或者什么都不依赖。

| crate | 职责 | 平台 | unsafe |
| --- | --- | --- | --- |
| `zephyra-winit` | 窗口 + 事件循环 + D3D11 设备与呈现 | Windows | 解除forbid（FFI） |
| `zephyra-winit-d3d12` | 窗口 + D3D12 设备与呈现（与 D3D11 **互不依赖**） | Windows | 需要 |
| `zephyra-dcomp` | DirectComposition 呈现后端 | Windows | 需要 |
| `zephyra-camera` | 摄像头采集 + 像素格式（默认 feature 为空） | 全平台 | forbid |

`zephyra-winit-d3d12` 与 `zephyra-camera` / `zephyra-dcomp` 在 workspace
`members` 里但排除在 `default-members` 之外 —— 前者因为同一个二进制只应有一个
后端，后两者因为设备代码与 `unsafe` 不该被无差别构建编译。

`zephyra-dcomp` 需要 `unsafe` 与 `windows` 依赖，与通用层的零依赖 /
`forbid(unsafe_code)` 两条约束都冲突。它也**不依赖 `zephyra`**：
DirectComposition 与 DXGI 是 Win32 词汇，与 UI 无关。由宿主 crate 同时依赖两者接线。

## 保留式 vs 即时式

选了保留式（GPUI / rgui 风格），不选即时式（egui 风格）。

理由是本框架的目标应用**在 GPU 占用率上是第一约束**：

- 即时模式每帧从代码重建 UI，布局与绘制融合，没有「只有这一小块变脏」的概念；
- 保留式让你描述一棵树，框架 diff 后生成 DisplayList，可以明确回答「这一帧重算了什么」。

代价是要自建布局、绘制与命中测试 —— 这就是 `layout` / `paint` / `hit` 三个模块存在的原因。

## 当前状态

骨架阶段。模块边界与契约已定并有测试覆盖（`cargo test -p zephyra` 共 304 条
断言、`-p zephyra-fx` 另有 64 条，都不需要 GPU）；控件库与模板系统**未实现**。

`examples/foundation` 是验收标准而非演示 —— 它断言的是契约：

1. 核心零依赖；
2. 声明树能落地成节点树；
3. 节点树能降级为已稳定排序的 `DisplayList`；
4. 稳态 diff 是 no-op；
5. 属性相等可判定（这是第 4 条成立的前提）；
6. 主题的语义角色在 Light/Dark 下均可查。

## 已决定不做的事

诚实列出，避免后来者重提：

- **节点级失效**（只重算脏子树的 `DisplayList` 分片，而不是整树重渲）。
  「这一帧画什么」已经收进 [`zephyra::frame::FrameCache`]，稳态整帧跳过也做完了
  （省 `paint` 73 µs + `expand` 40 µs + 一次 present，162 → 49 µs）。剩下的这一档
  收益约 40%，但代价有三条：

  1. 要改 `DisplayList` 的语义 —— 从「整树重渲」变成「拼接陈旧片段」，而那份
     逐字节相同的契约会变脆；
  2. **省不了 `expand`** —— 合批跨节点（`batches` 合并连续同纹理图像），按节点切
     缓存就会打散合批，draw call 反而变多；
  3. 要额外缓存展开后的几何才能真省，于是失效逻辑要乘两层。

- **GPU 显存预算 / 主动回收**（对标 Skia `GrResourceCache::setResourceCacheLimit`）。

## 尚未决定的事

诚实列出，避免后来者以为这些已经想清楚了：

- **跨帧脏区 / 局部重绘**：在当前 flip model 下**不可行**（`Present` 之后后缓冲未定义），
  所以只能做帧内 scissor。要真正做到「只重画脏区」需要**保留式后备缓冲**
  （渲染到自己的一张位图，再整体上屏），那是一次架构决定。详见
  [`migration-seam.md`](migration-seam.md) 第 3 节。
- **事件系统**：命令式绑定（`on_click(Arc<dyn Fn>)`）还是消息式（`on_click(EventId)` + 应用侧路由）。
  后者更易测试与序列化，但多一层间接。
- **虚拟化列表**：`layout` 协议是否需要内建「只测量可见项」的支持。当前未考虑。
- **动画**：脏标记只有 `Paint`/`Layout`/`Structure` 四级，没有「插值中」这一等。属性动画可能需要第五级，
  或者动画完全在宿主侧做。
- **无障碍**：WinUI 的 Automation 与 axum 同等重要。当前完全没有。
- **多窗口 / 多显示器**：`DisplayList` 是按窗口的，但跨窗口共享资源（纹理图集）的策略未考虑。

## 相关

- [`migration-seam.md`](migration-seam.md) —— 宿主可替换点清单（换一个 base 实现要改哪里）
- [`design-tokens.md`](design-tokens.md) —— 令牌规范

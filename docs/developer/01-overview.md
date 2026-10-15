# 总览

## 一句话

一个保留式、GPU 优先的 UI 框架。**核心负责决定画什么，宿主负责怎么画。**

## 数据流

```text
  Element 树   ──diff──▶  Node 树  ──measure/arrange──▶  DisplayList  ──▶ 像素
   (声明)         (保留)      (状态)                     (唯一契约)      (宿主)
```

五步各自回答一个问题：

| 阶段 | 回答 | 谁负责 |
|---|---|---|
| `Element` 树 | 应用**声明**了什么 | 应用代码，每帧新建，可整个丢弃 |
| `diff` | 上一帧的 `Node` 树和这一��有什么不同 | `zephyra-core`，逐节点对账 |
| `Node` 树 | 当前**状态**是什么（保留式） | `zephyra-core`，带脏标记 |
| `measure` / `arrange` | 每个节点**有多大、放在哪** | `zephyra-core`，纯几何 |
| `DisplayList` | 这一帧**要画什么命令** | `zephyra-core` 产出，宿主消费 |

`DisplayList` 是核心与后端之间**唯一**的契约。宿主拿到它，只回答另一个问题：
「我要不要把它推上屏」。

## 为什么分成这样

宿主**不允许**做三件事，详见 [`../zephyra/zephyra-base/README.md`](../../zephyra/zephyra-base/README.md)：

1. 不决定画什么 —— 画面内容全部来自 `DisplayList`
2. 不持有 UI 状态 —— 树和列表都在核心里
3. 不引回控件层 —— 免得 base 依赖 core 之外的东西

第2 条是最容易被破坏的。它一旦被破坏，「这一帧画什么」就会散落到多个地方，
而那正是本仓历史上 GPU 问题反复出现的根源（见
[`../dev-log/2026-10-gpu-device-hung.md`](../dev-log/2026-10-gpu-device-hung.md)）。

## 仓库结构

```text
zephyra/
  zephyra-core/      通用层：零依赖，#![forbid(unsafe_code)]
    property.rs      属性系统：定义、存储、继承、变更优先级
    props.rs         标准属性键（三方共用：布局/绘制/控件层）
    theme.rs         设计令牌 + Light/Dark
    element.rs       声明树（可丢弃）
    node.rs          保留树 + 脏标记
    diff.rs          Element→Node 对账 + DiffStats
    context.rs       只读上下文（主题 + 文本引擎）
    layout.rs        Measure / Arrange 协议
    paint.rs         DisplayList —— 核心与后端的唯一契约
    filter.rs        效果描述：Filter / FilterChain / BlendMode
    geom/            路径几何：展平 / 笔画展开 / 多孔耳切三角化
    hit.rs           命中测试
    id.rs            NodeTag / ElementKey —— diff 的身份判定词汇
    frame.rs         帧缓存：跨帧存树与列表，回答「这一帧画什么」
    input/           指针事件词汇表、手势识别、视口变换、输入路由
    icon/            SVG path 解析 + 图标注册表
    text/            文本测量与排版抽象（trait）+ 字形图集
  zephyra-fx/        效果器层：顶点展开 / 字形图集打包 / CPU 光栅 / 离屏规划
                     （零系统 API，与 base 并列而非其子目录）
  zephyra-base/      系统适配层：凡是要问操作系统才能回答的事
    zephyra-winit/     winit + D3D11 宿主
    zephyra-winit-d3d12/  winit + D3D12 后端（独立 crate）
    zephyra-camera/    摄像头采集与像素格式（可选子包）
    zephyra-dcomp/     DirectComposition 呈现后端（可选子包，Windows-only + unsafe）
    zephyra-platform/    平台无关的 Win32 抽象
    zephyra-platform-win/  Win32 API（unsafe + COM）
zstylec/             .zstyle 编译器（解析 + 检查）
ZephyraCLI/          命令行：new / check / dev / run / build / info / props
examples/            验收基线 + 窗口化 demo
ZephyraUI/           控件层 + RinUI 风格 gallery
```

### 三层的关系

- **core**：零依赖、`forbid(unsafe_code)`。不认识操作系统。
- **fx**：效果器。零系统 API，但会算——顶点展开、图集打包、CPU 光栅。
- **base**：凡是要问操作系统才能回答的事才在这里。它不认识控件。

## 下一步

- 想跑起来 → [快速开始](02-getting-started.md)
- 想理解为什么这么分层 → [核心概念](03-core-concepts.md)
- 想写样式 → [样式表](04-stylesheets.md)
# ZephyraUI

一个保留/声明式、GPU 优先的 UI 框架。核心负责决定画什么，宿主负责怎么画。

```text
  Element 树   ──diff──▶  Node 树  ──measure/arrange──▶  DisplayList  ──▶ 像素
   (声明)         (保留)      (状态)                     (唯一契约)      (宿主)
```

## 仓库结构

```text
zephyra/
  zephyra/         通用层：零依赖，#![forbid(unsafe_code)]
    core.rs        几何与排版基元（Rect / Thickness / Color / Spacing / Length）
    property.rs    属性系统：定义、存储、继承、变更优先级
    props.rs       标准属性键（三方共用：布局/绘制/控件层）
    theme.rs       设计令牌 + Light/Dark
    element.rs     声明树（可丢弃）
    node.rs        保留树 + 脏标记
    diff.rs        Element→Node 对账 + DiffStats
    context.rs     只读上下文（主题 + 文本引擎）
    style.rs       属性表 + 上下文 → 具体值
    layout.rs      Measure / Arrange 协议
    paint.rs       DisplayList —— 核心与后端的唯一契约
    filter.rs      效果描述：Filter / FilterChain / BlendMode
    geom/          路径几何：展平 / 笔画展开 / 多孔耳切三角化（图标走 CPU 三角化）
    hit.rs         命中测试
    id.rs          NodeTag / ElementKey —— diff 的身份判定词汇
    frame.rs       帧缓存：跨帧存树与列表，回答「这一帧画什么」
    input/         指针事件词汇表、手势识别、视口变换、输入路由
    icon/          SVG path 解析 + 图标注册表 + 通用字形
    text/          文本测量与排版抽象（trait，不绑定实现）+ 字形图集
  zephyra-fx/      效果器层：顶点展开 / 字形图集打包 / CPU 光栅 / 离屏规划
                   （零系统 API，与 base 并列而非其子目录）
  base/            系统适配层：凡是要问操作系统才能回答的事
    README.md      这一层的三条契约（消费 DisplayList / 不决定画什么 / 不引回控件层）
    zephyra-winit/   winit + D3D11 宿主：窗口 / 事件循环 / DisplayList 回放
    zephyra-winit-d3d12/  winit + D3D12 后端（独立 crate，与 D3D11 互不依赖）
    zephyra-camera/  摄像头采集与像素格式（可选子包，默认 feature 为空）
    zephyra-dcomp/   DirectComposition 呈现后端（可选子包，Windows-only + unsafe）
docs/
  architecture.md      数据流、模块职责、尚未决定的事
  migration-seam.md    宿主可替换点清单（换一个 base 实现要改哪里）
  design-tokens.md     令牌规范
examples/
  foundation.rs        地基验收标准（可执行）
  demo.rs              窗口示例应用（winit + D3D11，真出像素）
```

## 现状

**可运行的控件库 + Gallery 展台。** `ZephyraUI/` 是照抄 RinUI Gallery 的演示应用：
44 个控件卡、70 条路由、`ui.zstyle` + `gallery.zstyle` 两份外观表。

设计参考：WinUI 3 / Fluent 的分层与令牌体系，GPUI / rgui 的保留式架构。
UI 编程模型采用**保留式声明**（代码描述元素树，不用 XAML，不用即时模式）。

### 本轮新增的三件事

| 能力 | 入口 | 文档 |
|---|---|---|
| **布局跟随输入更新**（此前只有 resize 才重排） | `zephyra-core/src/frame.rs` 的 `tree_needs_layout` | 见下 |
| **响应式 / 触控判据** | `ZephyraUI/src/responsive.rs` | [`docs/responsive-and-touch.md`](docs/responsive-and-touch.md) |
| **动画地基**（缓动 / 时钟 / 档位） | `ZephyraUI/src/anim.rs` | [`docs/animation.md`](docs/animation.md) |
| **i18n**（BCP 47 协商 + 回退链 + 三语文案表） | `ZephyraUI/src/i18n.rs` | [`docs/i18n.md`](docs/i18n.md) |

### 「为什么只有 resize 以后布局才更新」

`FrameCache::frame` 的重排判据原先只有三项：首帧、视口变了、`diff` 说要重排。
而样式应用（`apply`）排在 `diff` **之后**（这是硬契约，见
`zephyra-style/src/apply.rs` 的模块文档），所以样式写出的布局键 ——
hover / press 改 `padding` 就是最典型的一种 —— 会经 `Node::set_styled_prop`
标上 `Dirty::Layout`，却**没人读它**：只重绘、不重排。

症状不是崩溃，是两套几何：字形按新 padding 画，命中测试按旧矩形判。
鼠标移动只翻分类名，于是这条路径**只能**靠 resize 或整树改动触发。

修法是把判据换成直接读树上的脏标记（谁标了 `Dirty::Layout` 就要重排，
不问它是 diff 标的还是 apply 标的）。门禁 `ZephyraUI/tests/hover_layout_tests.rs`
的三条断言在修好之前是红的。

## 快速开始

```bash
# 纯 CPU：契约断言，不需要 GPU、不需要窗口
cargo run -p zephyra-foundation                           # 地基验收
cargo test -p zephyra-core                                     # 304 条契约断言
cargo test -p zephyra-fx                                  # 64 条，纯 CPU
cargo test -p zephyra-foundation --test demo_interaction  # hover/press 端到端
cargo tree -p zephyra-core --depth 1# 应只输出 zephyra-core 自身

# 真开窗口：需要 GPU
cargo run -p zephyra-foundation --bin demo                # 窗口示例
ZEPHYRA_TRACE=1 cargo run -p zephyra-foundation --bin demo   # 每帧统计
ZEPHYRA_DCOMP=1 cargo run -p zephyra-foundation --bin demo   # 走DirectComposition 呈现
#   交互：鼠标 hover / 按下按钮；按 T 切 Light/Dark；Esc 退出

# 可选子包：默认不参与无差别构建，需要显式选中
cargo test -p zephyra-dcomp --lib                         # 纯逻辑断言，不需要窗口
cargo test -p zephyra-dcomp --test smoke                 # 需要真实窗口 + GPU
```

倒数第四条是本框架**唯一不可让步的架构约束**：通用层一旦 import 了宿主类型，
宿主迁移时就得连它一起搬。若你看到它开始输出依赖，说明这条被破坏了。

`zephyra/zephyra-base/` 下的 `zephyra-camera` 与 `zephyra-dcomp` 在 workspace `members` 里
但**不在** `default-members` 里，所以无差别的 `cargo build` / `cargo test` 既不
编译设备代码、也不编译 unsafe 代码 —— 「选择编译」落在 Cargo 层面，比在源码里
`#[cfg]` 一大块更细，因为连依赖解析都省掉了。

## 三层结构

```text
控件层 ZephyraUI/src/controls + ZephyraUI/src/app   Button / Slider / ToggleSwitch …
   ↓ 构造 Element、设属性
通用层 zephyra/zephyra-core property / theme / element / node / diff / layout / paint / filter / geom / hit / input / icon / frame
   ↓ DisplayList（纯数据契约，可序列化可断言）
效果器层 zephyra/zephyra-fx 顶点展开 / 字形图集 / CPU 光栅 / 离屏规划
   ↓ 顶点流 + 中立的资源需求
base zephyra/zephyra-base       D3D11 / D3D12 / DComp / 摄像头 / 平台抽象
   ↓
操作系统                Win32 / DirectComposition / Wayland / X11 / v4l2 …
```

依赖方向单向：**base 消费 `DisplayList`，通用层与效果器层都不知道 base 存在。**
判据很硬—— 换一个 base 实现（D3D11 ↔ D3D12，DXGI 传统路径 ↔ DirectComposition，或
Windows ↔ Wayland），同一棵元素树必须产出逐字节相同的 `DisplayList`。详见
[`zephyra/zephyra-base/README.md`](zephyra/zephyra-base/README.md)。

## 与 Viewstage 的关系

Viewstage（`Viewstage-Rust`）是本框架的第一个消费者。它的渲染层已从应用中抽出：

| crate | 职责 |
| --- | --- |
| `viewink` | 笔迹算法 + GPU 批注管线 |
| `viewgfx` | 画布几何编排 + 逐笔画网格缓存 |
| `viewframe` | 帧驱动、vsync 节拍、直通合成通道 |
| `viewcomp` | DirectComposition 合成（target/visual 树 + composition swapchain） |
| `viewcamera` | 摄像头采集 + 渲染通道 |
| `viewicon` | SVG 解析 + 图标栅格化 |

这些 crate 构成了 ZephyraUI 将来的**渲染底座**。其中图标栅格化与纹理缓存最终
也应并入本仓；现暂留 Viewstage，因为它与该仓的应用层还耦合在一起。

`viewcomp` 是唯一例外：它已迁入本仓成为 `zephyra/zephyra-base/zephyra-dcomp`（DirectComposition
是 Win32 词汇，与 UI 无关，因此它属于 base 层而非某个具体宿主）。两边目前并存同一份
实现，本仓侧验证稳定后会让 Viewstage 切换依赖 ——
见 [`docs/migration-seam.md`](docs/migration-seam.md)。

## License

MIT OR Apache-2.0

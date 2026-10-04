# ZephyraUI

一个保留/声明式、GPU 优先的 UI 框架。核心负责决定画什么，宿主负责怎么画。

```text
  Element 树   ──diff──▶  Node 树  ──measure/arrange──▶  DisplayList  ──▶ 像素
   (声明)         (保留)      (状态)                     (唯一契约)      (宿主)
```

## 仓库结构

```text
crates/
  zephyra/         核心：零依赖，#![forbid(unsafe_code)]
    core.rs        几何与排版基元（Rect / Thickness / Color / Spacing / Length）
    property.rs    属性系统：定义、存储、继承、变更优先级
    theme.rs       设计令牌 + Light/Dark
    tree.rs        Element ↔ Node 的 diff 与脏标记
    layout.rs      Measure / Arrange 协议
    paint.rs       DisplayList —— 核心与后端的唯一契约
    text.rs        文本测量与排版抽象（trait，不绑定实现）
    hit.rs         命中测试
  zephyra-winit/   winit + wgpu 宿主适配层（目标形态，占位）
docs/
  architecture.md      数据流、模块职责、尚未决定的事
  migration-seam.md    iced 宿主 → winit+wgpu 宿主的替换点清单
  design-tokens.md     令牌规范
examples/
  foundation.rs        地基验收标准（可执行）
```

## 现状

**骨架阶段。** 模块边界与契约已定，控件库与模板系统未实现。

设计参考：WinUI 3 / Fluent 的分层与令牌体系，GPUI / rgui 的保留式架构。
UI 编程模型采用**保留式声明**（代码描述元素树，不用 XAML，不用即时模式）。

## 快速开始

```bash
cargo run -p zephyra-foundation   # 跑一遍地基验收
cargo test -p zephyra             # 45 条契约断言，不需要 GPU
cargo tree -p zephyra --depth 1   # 应只输出 zephyra 自身 —— 零依赖
```

最后一条是本框架**唯一不可让步的架构约束**：核心一旦 import 了宿主类型，
宿主迁移时就得连核心一起搬。若你看到它开始输出依赖，说明这条被破坏了。

## 与 Viewstage 的关系

Viewstage（`Viewstage-Rust`）是本框架的第一个消费者。它的渲染层已从应用中抽出：

| crate | 职责 |
| --- | --- |
| `viewink` | 笔迹算法 + GPU 批注管线 |
| `viewgfx` | 画布几何编排 + 逐笔画网格缓存 |
| `viewframe` | 帧驱动、vsync 节拍、直通合成通道 |
| `viewcamera` | 摄像头采集 + 渲染通道 |
| `viewicon` | SVG 解析 + 图标栅格化 |

这些 crate 构成了 ZephyraUI 将来的**渲染底座**。其中图标栅格化与纹理缓存最终
也应并入本仓；现暂留 Viewstage，因为它依赖该仓的 iced 版本。

## License

MIT OR Apache-2.0

# 迁移缝：iced 宿主 → winit+wgpu 宿主

## 目标

Viewstage 的渲染层最终收敛到 **winit + wgpu**。`iced-fork` 是过渡期。
本文件列出**所有必须改动的地方**——目的就是让这个清单可穷举，而不是"到时候再看"。

## 判据：核心是否需要改

> 如果某个改动需要修改 `crates/zephyra/`，说明迁移缝漏了。

当前状态下这条判据成立：核心零依赖，`cargo tree -p zephyra` 无输出。

```
$ cargo tree -p zephyra --depth 1
zephyra v0.1.0 (D:\code\ZephyraUI\crates\zephyra)
```

建议把它做成 CI 检查（`cargo tree -p zephyra --depth 1 | wc -l` 应为 1）。

## 两个宿主的位置

| 宿主 | 位置 | 状态 |
| --- | --- | --- |
| `zephyra-iced` | `Viewstage-dev/crates/zephyra-iced` | 待实现（过渡期） |
| `zephyra-winit` | 本仓 `crates/zephyra-winit` | 占位（最终目标） |

### 为什么 `zephyra-iced` 暂不在本仓

它需要 iced，而 Viewstage 的 `[patch.crates-io]` 指向 `Viewstage-dev/iced-fork`。
若把它放进本仓，单独 `cargo build` 会拉到**上游未打补丁的** iced，编译失败或行为不一致。

要在本仓独立构建它，就得在本仓再写一份 `[patch.crates-io]` 指向
`Viewstage-dev/iced-fork` —— 仓间路径耦合，比现在更难维护。

**等 iced-fork 迁入本仓后**，把它加回 workspace members 并移除 Viewstage 侧副本。
这是本文件里唯一一个"需要两次动作"的迁移点，已在 `Cargo.toml` 注释中标注。

## 替换点清单

### 1. 窗口与事件循环

| 能力 | iced 宿主 | winit+wgpu 宿主 |
| --- | --- | --- |
| 窗口创建 | `iced::window::Settings` | `winit::WindowBuilder` |
| 事件循环 | `iced_winit::run` | `winit::EventLoop::run` |
| 指针/触摸/滚轮 | `Event::Mouse` / `Event::Touch` / `Event::MouseWheel` | `WindowEvent::CursorMoved` / `Touch` / `MouseWheel` |
| 键盘 | `Event::Keyboard` | `WindowEvent::KeyboardInput` |
| 缩放因子 | `iced::window::scale_factor` | `Window::scale_factor()` |
| 关闭事件 | `iced::window::close_events()` | `WindowEvent::CloseRequested` |

**注意**：Viewstage 当前的触控链路依赖 iced 的 `Event::Touch`（winit 把
`WM_POINTER*` 转成 `WindowEvent::Touch`，iced 再转成 `Event::Touch`）。迁到 winit
时直接消费 `WindowEvent::Touch` 即可，但**不能**假设触控会被提升成鼠标事件 ——
Viewstage 的代码里对此有明确记录（窗口已 `RegisterTouchWindow`，Windows 不会做
这种提升）。

### 2. 渲染

| 能力 | iced 宿主 | winit+wgpu 宿主 |
| --- | --- | --- |
| 表面与设备 | `iced_wgpu` 内部管理 | `wgpu::Instance` / `Adapter` / `Device` / `Surface` |
| `DisplayList` 回放 | `iced::widget::Scene`（`Quad` / `Image` / `Text`） | 自有 renderer |
| 圆角矩形 | iced quad shader | 自有 SDF shader |
| 图像纹理 | `iced::widget::image::Handle` | 自建纹理图集 |
| 字形图集 | iced 的 glyph atlas | 自建（cosmic-text / swash） |
| 合成顺序 | `Layer`（Viewstage 自有直通通道） | `Layer` 映射到 render pass |
| 抗锯齿 | 全局 `antialiasing: false` + 自有 SDF | 同 |

**已有可复用的经验**：Viewstage 的 `iced-fork/iced_wgpu/src/direct.rs` 已经实现了
"直通合成通道"——在 iced 的 pass 里插入自己的 `DirectChannel`，绕过 Scene 直接画进
frame。这个思路（**按 Layer 切 render pass**）应当原样搬到 `zephyra-winit`。

### 3. 呈现节拍

Viewstage 的 `viewframe` crate 已经解决了一个真实问题：`Program::draw` 的调用次数
是 **layout 次数**而非呈现次数（实测 74/s vs 37.7/s），所以刷新率估计必须挂在
"成功 `present` 之后"。

winit+wgpu 宿主下这个坑**依然存在**——`render()` 同样会在每帧被调用。做法是一样的：
在 `queue.submit` + `present` 成功之后回调一次，把呈现间隔喂给节拍估计器。

`viewframe::pacing` 的逻辑与宿主无关，迁移时应原样保留（它的 16 条断言就是标定
的回归网）。

### 4. 字体

| | iced 宿主 | winit+wgpu 宿主 |
| --- | --- | --- |
| 字体枚举 | 系统字体 / `iced_graphics` 的 font manager | `fontdb` |
| 整形 | iced 内部 | cosmic-text 或 swash |
| 字形缓存 | iced 的 glyph atlas | 自建 |
| 测量接口 | `canvas::Program` 的 `text::layout` | `zephyra::text::TextEngine` 实现 |

**这是迁移中工作量最大的一块**，因为字形图集的生存期管理（LRU、atlas 增长、
异步上传）必须自建。核心侧的 `zephyra::text` 刻意只声明接口，就是为了把这块
完全推给宿主。

### 5. 图像解码

Viewstage 当前用 `image` crate 解码，`iced` 的 `image-without-codecs` feature
只提供纹理上传。迁到 wgpu 后这部分**不变**（解码与上传本来就与框架无关）。

## 迁移顺序建议

1. 先让 `zephyra-iced` 跑通一个最小窗口 + 一个按钮 + 一段文本；
2. 补 `zephyra-winit` 到同等能力；
3. 把 Viewstage 的工具栏（`src/views/toolbar.rs`，189 行）作为第一个真实页面迁移 ——
   它足够小（7 个图标按钮 + 2 个容器），但包含图标、hover/active 态、状态联动；
4. 迁完工具栏后，**对比两宿主的 DisplayList**（`zephyra::paint::DisplayList` 是
   可比较的数据，直接 `assert_eq!` 即可）。这是验证迁移正确性的最强手段。
5. 之后按页面逐个迁。

## 不需要改的部分（已确认）

- `zephyra::core` / `property` / `theme` / `tree` / `layout` / `paint` / `hit`
  —— 零依赖，与宿主无关。
- `zephyra::text` 的 **trait 定义**（实现会换，签名不变）。
- Viewstage 的领域引擎：`viewink`（笔迹）、`viewcamera`（采集）、`viewgfx`（画布编排）。
  它们只依赖 wgpu，不依赖 iced 的 widget 层。

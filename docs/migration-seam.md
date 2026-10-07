# 迁移缝：宿主可替换点清单

## 这份文件的定位

本框架只有**一个**宿主实现：`crates/base/zephyra-winit`（D3D11）与其姊妹后端
`crates/base/zephyra-winit-d3d12`。但 base 层的存在意义就是"凡是要问操作系统才能
回答的事都可以换实现"，所以这份文件的作用是：

> 把**换一个 base 实现时必须改动的地方**全部列出来，让清单可穷举，而不是"到时候再看"。

判据贯穿全篇：

> **如果某个改动需要修改 `crates/zephyra/` 或 `crates/zephyra-fx/`，说明迁移缝漏了。**

当前状态下这条判据成立：核心零依赖、效果器层零系统 API。

```bash
$ cargo tree -p zephyra --depth 1
zephyra v0.1.0
```

建议把它做成 CI 检查（`cargo tree -p zephyra --depth 1 | wc -l` 应为 1）。

## 已被移除的宿主：iced

本框架**曾经**有一个 iced 宿主作为过渡期方案，那条路线已经放弃：

- `zephyra-iced` 已从 workspace 移除，`crates/zephyra-iced/` 空占位目录已删除；
- `Cargo.lock` 中不再有 `iced` / `iced_winit` / `iced_graphics` 任何条目；
- 渲染不走 `wgpu`，而是直接用 **D3D11 / D3D12 + DirectComposition**。

放弃的理由不是"iced 不好"，而是**多一个宿主就多一份必须同步的替换点**。既然最终要
落到 D3D，过渡宿主带来的迁移清单是纯负债：它列出的每一项在落地时都要重写一遍，而
清单本身还会因为过时（描述一条不走的路线）误导后来者。

`crates/zephyra/src/input/mod.rs` 与 `icon/mod.rs` 里仍提到 iced，那两处是**不变量
说明**——"同一个两指捏合要同时服务于不同宿主的事件"、"栅格化不放在核心里"——用的是
具体例子而非依赖，保留它们是为了让约束可读。

## 当前的后端矩阵

两个后端是**两个独立 crate**，互不依赖，各自支持两条呈现路径：

| crate | GPU | 状态 |
| --- | --- | --- |
| `zephyra-winit` | D3D11 | default-members，无差别构建就编它 |
| `zephyra-winit-d3d12` | D3D12 | members 但不在 default-members |
| `zephyra-dcomp` | 两者共用 | 可选子包，DComp 呈现 |

| 路径 | 何时 | 后缓冲来源 | 上屏 |
| --- | --- | --- | --- |
| 普通 | 默认 | `CreateSwapChainForHwnd` | `Present` |
| DComp | `ZEPHYRA_DCOMP` != `"0"` | `CreateSwapChainForComposition` | `Present1` + DComp `Commit` |

## 替换点清单

### 1. 窗口与事件循环

| 能力 | winit 宿主 | 替换成别的窗口系统 |
| --- | --- | --- |
| 窗口创建 | `winit::WindowBuilder`（经 `HostWindow::new_lenient`） | 平台自己的窗口创建 |
| 事件循环 | `winit::EventLoop` + `ApplicationHandler` | 平台消息循环 |
| 指针 | `WindowEvent::CursorMoved` / `MouseInput` / `MouseWheel` | 平台事件 |
| 触摸 | `WindowEvent::Touch` | 平台触摸事件 |
| 键盘 | `WindowEvent::KeyboardInput` | 平台键盘事件 |
| 缩放因子 | `Window::scale_factor()` | 平台 DPI |
| 关闭 | `WindowEvent::CloseRequested` | 平台关闭事件 |

**不可省略的一条**：winit 把 `WM_POINTER*` 转成 `WindowEvent::Touch`。**不能**假设触控
会被提升成鼠标事件——窗口已 `RegisterTouchWindow`，Windows 不做这种提升。所以事件
路由（`zephyra::input`）必须在 touch 与 pointer 两条路径上都接，缺一条就等于没有触控。

`ApplicationHandler` 的类型是两个后端**共用**的，所以 `winit` 的版本号（0.30）在两个
crate 里必须一致——否则应用层代码没法同时对上两个后端。

### 2. GPU 设备与渲染

| 能力 | D3D11 | D3D12 | 备注 |
| --- | --- | --- | --- |
| 设备 | `ID3D11Device` | `ID3D12Device` + 队列 | D3D12 额外要描述符堆与根签名 |
| 命令提交 | 上下文立即提交 | 命令列表 + 资源屏障 | D3D12 每个概念在 D3D11 里都没有对应物 |
| PSO | `ID3D11PixelShader` 等 | `ID3D12PipelineState` | |
| 顶点缓冲 | 动态 | 默认堆 + 上传堆 | |

**为什么分 crate 而不合并**：D3D11 与 D3D12 不是同一套 API 的两个版本。塞进一个
crate 意味着两种写法永远在同一批文件里互相干扰，`#[cfg]` 分支会淹没真正的逻辑，
而两个后端各自的"静默不画"类 bug 会长得一模一样、难以区分。同一个二进制里只应有
一个后端被编译进来，所以两者都在 `members` 但都不在 `default-members`。

### 3. 呈现节拍

真实存在的坑：`render()` 的调用次数是 **layout 次数**而非呈现次数。所以刷新率估计
必须挂在"成功 `present` 之后"，而不是挂在 render 回调里。

做法在两个后端里都一样：`Present` / `Present1` 成功之后回调一次，把呈现间隔喂给节拍
估计器。任何新后端都要重复这个教训——它不会以崩溃的形式出现，只会表现为刷新率
读数是 layout 频率的整数倍。

#### 3.1 ⚠️ `Unchanged` 时**连 present 都不能做**

这是新增后端最容易漏的一条，而且**两个现有后端各自踩过一次**。

`zephyra::frame::FrameCache` 会返回 `FrameOutcome::Unchanged`（`DisplayList` 与上次
交给宿主的逐字节相同）。宿主据此跳过绘制 —— 而**必须连 present 一起跳过**：

> 两个后端都是 flip model（`FLIP_DISCARD` / `FLIP_SEQUENTIAL`），`Present` **之后**
> 后缓冲内容是**未定义**的。不画就 present 会把未定义内容推上屏（通常是黑，或
> 上一帧的残影），而 `RenderReport` 里各项计数全对、没有任何 HRESULT 错误。

所以宿主侧的判据是 `Frame::redrawn`，不是「有没有跑 `frame()`」。

#### 3.2 `Damage` 只能用于**帧内** scissor

`FrameOutcome::Updated { damage, .. }` 里的 `Damage` **不是**给宿主用来「跳过脏区外
的像素」的——由 3.1 那条约束，那根本不成立。它唯一的用途是**帧内**限制光栅化范围。

新后端要注意两处平台差异：

| | scissor 与 clear 的关系 |
|---|---|
| **D3D11** | `ClearRenderTargetView` **不受** scissor 约束 ⇒ 清屏永远全屏，脏区只省绘制 |
| **D3D12** | 命令列表上的 clear **会**被 scissor 裁掉 ⇒ 脏区连 clear 一起省 |

另有一条两边都踩过的：退化矩形（`right <= left`）会让 scissor **什么都不画**且零错误。
所以算出来的矩形若退化，必须回退到全屏。像素级判据见
`crates/base/zephyra-winit/tests/d3d11_draw.rs::帧内_scissor_只影响脏区`。

### 4. 效果（filter / blend）

**这是当前最大的未完成项。** 契约已在核心侧完整，执行层还没接：

| 环节 | 位置 | 状态 |
| --- | --- | --- |
| `Filter` / `FilterChain` / 29 个 `BlendMode` | `zephyra::filter` | 完成 |
| `Command::{Quad,Image,Text}` 的 `filter` / `blend` 字段 | `zephyra::paint` | 完成 |
| 离屏边界规划（blur 按 3σ、shadow 按四边半径 + offset） | `zephyra_fx::offscreen::plan_offscreens` | 完成 |
| 离屏 RT 创建 / 采样 / 回合成 | base | **未实现** |
| blend state 与 PSO 变体 | base | **未实现** |

所以 `RenderReport` 有两个计数：`unimplemented_filters` / `unimplemented_blends`
——**它们恒为正是当前的常态**，用来区分"没人用效果"与"效果被静默忽略"。命令**照常
画**（不 skip），所以症状是"设了 blur 没反应"，而不是"元素消失"。

接执行层时**不要**改成把离屏边界录进 `DisplayList`。核心只描述效果，边界由 fx 规划，
这样同一棵元素树在"有 GPU"与"无 GPU"两种环境下产出逐字节相同的 `DisplayList`——
这正是 base 层契约 2 的要求。

### 5. 字体

| 能力 | 当前 | 替换成别的 |
| --- | --- | --- |
| 字体枚举 / 解析 | `ab_glyph`（`zephyra-fx` 的 text 栈） | `fontdb` |
| 整形 | **无**（ab_glyph 不含 GSUB/GPOS） | cosmic-text / swash |
| 字形图集 | `zephyra::text::GlyphAtlas`（core，纯 CPU） | 位置表可换，契约不变 |
| 淘汰策略 | `EvictionPolicy` trait，默认 `LruEviction` | 宿主可整体替换 |

`TextEngine` 是 trait，换整形引擎时核心侧一行不用改。

图集满时的淘汰是可替换策略而不是硬编码 LRU——两个值得换掉默认的真实动机：
按可见性淘汰（弹窗打开时被遮挡的背景文字没必要留着）；**不清屏重排**（默认策略
每次淘汰都让所有幸存者的 uv 变化，宿主因此重传整张 4MB 图集，逐帧换字串的场景会
每秒重传一次）。

### 6. 图像解码

解码与上传本来就与框架无关：核心不管，fx 只持纹理**名字**（`TextureNames`），
`zephyra-winit` 的 `TextureTable` 持真实资源。

⚠️ **目前仓里没有解码器。** `HostWindow::update_texture(name, pixels, w, h)` 只收
**裸 RGBA 字节**，PNG / JPEG 的解码完全在应用侧。所以「换一个解码器」这件事目前
还没有落点 —— 等真接上时，它同样不触及任何契约（核心不管、fx 只见名字、base 只见
字节与尺寸），但别把 `image` crate 当成既成事实写进文档。

注意一次 draw call 只能绑定一张纹理视图，所以图像**无法**与圆角矩形/字形合批。这是
D3D11/D3D12 的模型决定的，不是可以绕过的选择。

### 7. 非 Windows 呈现路径

`zephyra-winit` 在非 Windows 上直接 `compile_error!` 并说明原因，而不是静默退化成
"能编译但不出画面"。Linux 侧要么另写一个 Vulkan 后端，要么走 Wayland 的 subsurface
——**尚未决定**。这是当前唯一一个"完全没有第二种实现"的维度。

## 不需要改的部分（已确认）

- `zephyra` 全部模块（`core` / `property` / `props` / `theme` / `id` / `element` /
  `node` / `diff` / `context` / `style` / `filter` / `layout` / `paint` / `hit` /
  `input` / `icon`）—— 零依赖，`forbid(unsafe_code)`，与宿主无关。
- `zephyra-fx` 全部模块 —— 一个系统 API 都不碰，`forbid(unsafe_code)`。
- `zephyra::text` 的 **trait 定义**（实现会换，签名不变）。
- `zephyra-dcomp` 与宿主的关系：它只管上屏，不碰 `DisplayList` 的内容，因此换呈现
  路径不影响核心。它也**不依赖 `zephyra`**——DComp 与 DXGI 是 Win32 词汇，由宿主
  crate 同时依赖两者接线。

## 与 Viewstage 的关系

Viewstage 是本框架的第一个消费者，它的渲染层已经从应用中抽出。`viewcomp` 是唯一
已经迁入本仓的（成为 `crates/base/zephyra-dcomp`），两边目前并存同一份实现，本仓侧
验证稳定后让 Viewstage 切换依赖。其余 crate（`viewink` / `viewgfx` / `viewframe` /
`viewcamera` / `viewicon`）仍留在 Viewstage 仓。

对比两边的 `DisplayList` 是验证接入正确性最强的手段——`zephyra::paint::DisplayList`
是可比较的数据，直接 `assert_eq!` 即可。

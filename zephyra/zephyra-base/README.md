# base —— 系统适配层

这一层回答**只有问操作系统才能回答的问题**：窗口怎么开、事件怎么收、GPU 设备从哪
来、最后怎么上屏。

它**不**回答"画什么"。那是通用层（`../zephyra`）的事。

## 三层结构

```text
控件层（未实现）              Button / Slider / ToggleSwitch …
   ↓ 构造 Element、设属性
通用层 zephyra/zephyra-core        property / theme / tree / layout / paint / filter / hit / input / icon
   ↓ DisplayList（纯数据契约，可序列化可断言）
效果器层 zephyra/zephyra-fx   顶点展开 / 字形图集 / CPU 光栅 / 资源复用策略
   ↓ 顶点流 + 中立的资源需求
base（本目录）               D3D11 / D3D12 / DComp / 摄像头
   ↓
操作系统                      Win32 / DirectComposition / Wayland / X11 / v4l2 …
```

依赖方向单向，上层不知道下层存在。

## 效果器层为什么不在本目录里

`zephyra/zephyra-fx` **一个系统 API 都不碰**（无 `ID3D11Device`、无窗口句柄，
`unsafe_code = "forbid"`），所以它与 base **并列**而不是 base 的子目录。

判据是：**能不能在无 GPU 的机器上写单测**。顶点展开、字形图集打包、CPU 光栅、
离屏边界计算全都能单测，所以该进通用层与效果器层；而建纹理、设混合状态、走
DXGI 提交这些必须与系统打交道，所以留在本层。

这与 Skia 把 `src/effects` 放在与 `src/core` **并列**处是同一个判断。

## 三条契约

**1. 消费 `DisplayList`,而不是产出它。**

核心与这一层之间只有一个契约：`zephyra::paint::DisplayList`。它是一份排序后的
绘制命令列表 —— 纯数据、可比较、可序列化。base 负责回放它。

这意味着核心从不 import 任何宿主类型。`cargo tree -p zephyra-core --depth 1` 的输出
应当只有 `zephyra-core` 自己一行；一旦它开始输出依赖，说明这条被破坏了。

**2. 不得决定"画什么"。**

布局、命中、绘制顺序都在核心里。base 不重排、不裁剪、不挑选子集。

判断标准：换一个 base 实现（DXGI 传统路径 ↔ DirectComposition，或 Windows ↔
Wayland），同一棵元素树必须产出逐字节相同的 `DisplayList`。凡是会在替换 base 后
改变 `DisplayList` 内容的逻辑，都放错了层。

> ⚠️ **这条曾经被破坏过，现已修复。** `HostWindow` 里长期存着
> `node: Option<Node>` / `list: DisplayList` / `needs_layout: bool` 三块状态 ——
> 而它们的**数据**全在 core，于是宿主要自己编排「要不要重排 / 要不要重绘」。
> 那正是本条禁止的：base 在读 `Node::dirty()` 来决定这一帧画什么。
>
> 现在「这一帧画什么」只有一个地方做：[`zephyra::frame::FrameCache`]（core），
> 它连「与上次逐字节相同」这个答案一起给出。base 只回答另一个问题：要不要把它
> 推上屏。宿主侧剩下的唯一状态是 `dirty` —— 而它的含义是「**有没有待处理的
> 事件**」，不是「内容变了」（判断后者必须先跑应用的 `view()`）。
>
> 判据的反向用法同样成立：**改动只落在 base，就说明职责放错层了**。

**3. 不得引回控件层。**

base 不依赖 `zephyra-controls`（将来若有）。它只依赖核心，或者什么都不依赖。

值得注意的例子：`zephyra-dcomp` **不依赖 `zephyra-core`** —— DirectComposition 与 DXGI
是 Win32 词汇，与 UI 无关；由宿主 crate 同时依赖两者接线。

## 成员

| crate | 职责 | 平台 | unsafe |
| --- | --- | --- | --- |
| `zephyra-winit` | 窗口 + 事件循环 + D3D11 设备 + `DisplayList` 回放 | 全平台 | 本 crate 内无 unsafe 块 |
| `zephyra-winit-d3d12` | 窗口 + D3D12 后端（与 D3D11 **互不依赖**） | Windows | 需要 |
| `zephyra-dcomp` | DirectComposition 呈现：device/target/visual 树 + composition swapchain | Windows | 需要 |
| `zephyra-camera` | 摄像头采集 + 像素格式处理（默认 feature 为空） | 全平台 | forbid |

后两者是**可选子包**：在 workspace `members` 里但**不在** `default-members` 里，
所以无差别的 `cargo build` / `cargo test` 既不编译设备代码、也不编译 unsafe 代码。

```bash
cargo test -p zephyra-dcomp --lib           # 纯逻辑断言，不需要窗口
cargo test -p zephyra-dcomp --test smoke   # 需要真实窗口 + GPU
```

## 渲染策略（zephyra-winit）

四类图元（圆角矩形 / 字形 / 图像 / 多边形路径）合批进**单个** render pass、单个
pipeline、共用一个顶点缓冲 —— 靠顶点里的 `kind` 标量在片元着色器内分支，而不是多
pipeline。分支是逐片元一次而非逐 draw call 一次；图元都是小四边形且同批内 `kind`
通常相同，分支预测器几乎总命中。

圆角与描边在片元着色器内以 SDF 求出，因此**不需要 CPU 三角化** —— 这正是核心把
圆角矩形设计成 draw-call 级图元（`Command::Quad`）而非路径的回报。抗锯齿带宽固定
0.75 像素、不随缩放变化，所以高 DPI 下边缘不会变毛糙。描边单独画一遍环带而不是
混进填充色 —— 混进去会让描边与填充重叠处被填充盖住。

多边形路径（`Command::Path`，矢量图标走的就是它）是唯一的例外：SDF 需要解析距离场，
而任意路径没有，所以它**必须** CPU 三角化（`zephyra::geom` 的笔画展开 + 耳切），
抗锯齿只能靠 MSAA。它顶点数不是 4 的倍数，索引规则也不同，因此单独成段、单独一次
draw —— 见 `quad.rs` 的 `Command::Path` 分支。

## 两个后端 × 两条呈现路径

`zephyra-winit`（D3D11）与 `zephyra-winit-d3d12`（D3D12）是**两个独立 crate**，
互不依赖，各自都支持两条呈现路径，由环境变量 `ZEPHYRA_DCOMP` 切换：

| 路径 | 何时 | 后缓冲来源 | 上屏 |
|---|---|---|---|
| 普通 | 默认 | `CreateSwapChainForHwnd` | `Present` |
| DComp | `ZEPHYRA_DCOMP` 非 `"0"` | `CreateSwapChainForComposition` | `Present1` + DComp `Commit` |

两个后端**共用** `zephyra-dcomp` 这个 crate，于是 swapchain 参数与 DComp 挂树逻辑
天然一致 —— 少一份"两个后端悄悄不一致"的余地。

`Present1` 必须收非空 `DXGI_PRESENT_PARAMETERS`，所以统一走
`zephyra_dcomp::present_one`，不要自己调 `SwapChain1::Present1`。

### 为什么 D3D11 与 D3D12 要分crate 而不合并

D3D11 与 D3D12 是**两套 API**，不是同一套 API 的两个版本：描述符堆、根签名、
命令列表、PSO、显式资源屏障 —— D3D12 的每个概念在 D3D11 里都没有对应物。

塞进一个 crate 意味着两种写法永远在一个文件里互相干扰：`#[cfg]` 分支会淹没真正
的逻辑，而两个后端各自的"静默不画"类 bug 会长得一模一样、难以区分。同一个二进制
里只应有一个后端被编译进来，所以两者都在 `members` 但都不在 `default-members`。

### DComp 的三条硬约束

1. 交换链必须先于 DComp 合成器创建，且后者的释放要**晚于**前者（保活字段）。
2. DComp 设备走 `IDXGIDevice` 路线 —— **D3D11 device 能 QI 出它**，所以本仓选了
   D3D11 作为默认宿主。
3. `Present1` 的参数不能省，必须走 `zephyra_dcomp::present_one`。

### `windows` 绑定的版本是硬约束

`zephyra-dcomp` 与两个宿主 crate 都用 **`windows 0.62`**。两个大版本的 COM 类型
互相不认识（各自的 `ID3D12Resource` 是不同类型，`cast()` 不存在），版本错配唯一的
"解法"是跨 crate 指针强转 —— 那是把配置错误伪装成一次 unsafe。

## 字形图集满了怎么办：可替换的淘汰策略

图集是一块固定大小的 CPU 画布（默认 1024²）。装不下新字形时，它按**淘汰策略**腾地方
并重排整张图集 —— 这不是硬编码的 LRU，而是一个 trait：

```text
core：trait EvictionPolicy   ← 纯契约，零依赖，可单测
        evict(条目快照, 待放入尺寸, 图集边长) -> Option<EvictPlan>
  ↓
core：LruEviction（默认）    ← 丢最久未用的，重排按高度降序
宿主：可整体替换              ← TextStack::set_atlas_policy(...)
```

两种值得换掉默认的真实动机：

- **按可见性淘汰**：弹窗打开时，正在被遮挡的背景文字没必要留在图集里；
- **不清屏重排**：默认策略每次淘汰都清空画布再逐字形重画，于是**所有**幸存者的 uv
  都变了 —— 宿主会因此重传整张 4MB 图集。逐帧换字串的场景（每秒跳一次的时钟）会
  因此每秒重传一次。一个"只丢不排"的策略能避开重传，代价是空间利用率下降。

契约里有一处刻意设计：`EvictPlan::survivors` 携带的就是**重排顺序**本身。落盘与
"放得下吗"的干跑共用这一个顺序源，所以两处排序漂移（判得下却放不下，而画布已清空）
在类型层面就不可能发生。图集还会在清空前独立复验一次 —— 策略是外部代码，可能算错，
那时宁可放弃这次淘汰，也不能拿已清空的画布去赌。

字形图集本身住在 **core**（`zephyra::text::GlyphAtlas`），因为它是一块 `Vec<u8>` 加
一张位置表，GPU 只在最后 upload 一次。宿主那侧只做两件事：把字体字节解析成
`GlyphSource`，把 `atlas.pixels()` 上传成纹理。

图像无法与前两类合并：**一次 draw call 只能绑定一张纹理视图**。这是 D3D11/D3D12
的模型决定的，不是可以绕过的选择。

分批**只合并连续的同纹理命令**。把后面同纹理的拉到前面合并会改变绘制顺序，而绘制
顺序就是 z序 —— 图像常是半透明的，重排直接改变可见结果（一层浅色罩子被画到深色
之下，看起来像"没生效"）。代价是同一纹理可能分成多批，换来顺序绝对正确。

纹理所有权在宿主（它要能 resize 与释放），纹理表只持**名字**
（`zephyra_fx::TextureNames`）。CPU 侧对纹理的全部需求就是 `contains()` ——
用它决定某张图该画还是该跳过（见 `quad::expand` 的 `missing_textures` 计数）。

## uv 裁剪的两道防线

`Command::Image` 的 `src` 是 `core::UvRect`（归一化），不是 `core::Rect`。两者字段
相同但坐标系不同，用同一个类型表达它们，颠倒的错误不会有任何其他症状。

越界 uv 的症状出现在**采样之后**（取到相邻图块的像素），那时已离病因很远。所以
钳了两次，各管一段：

- **核心**（`style::Node::image_src`）：明显越界或零尺寸的 uv **退化成整张**，不
  转发也不钳正。宁可裁剪没生效（画面仍正确），也不要花屏。
- **效果器层**（`quad::expand` → `UvRect::clamped`）：部分越界（宽度算出 0.9 而起点
  0.3）仍能通过核心那一关，所以渲染器再钳一次。这里是最后一道防线。

## 这一层还缺什么

诚实列出，避免后来者以为"base 已经做完了"：

- ~~**无脏区 / 局部重绘**~~ —— **「静止时整帧跳过」已做**（`dirty` 事件门控），
  「稳态零重算」也已做（`zephyra::frame::FrameCache`）。剩下的只有节点级失效，见下。

  ⚠️ **跨帧局部重绘在当前交换链模型下不可行**，所以这一项不能按「跳过脏区外的
  像素」来设计。两个后端都用 flip model（`DXGI_SWAP_EFFECT_FLIP_DISCARD`，
  DComp 路径是 `FLIP_SEQUENTIAL`），而 flip model 下 `Present` 之后后缓冲内容
  **未定义** —— 任何要 present 的帧都必须重画整个目标，scissor 只能限制帧内。

  这不是实现偷懒：Skia 同样没有这个能力（`SkSurface::damage` 全仓零命中）。

  **已经接上的部分**：

  | 能力 | 位置 | 省什么 |
  |---|---|---|
  | 稳态零重算（`FrameOutcome::Unchanged` ⇒ 不画、不 present） | `zephyra::frame`（core） | `paint` 73 µs + `expand` 40 µs + 一次 present |
  | 帧内 scissor（`Damage::Rect`） | 本层两个渲染器 | 脏区外的**片元着色** |

  ⚠️ **D3D11 的 `ClearRenderTargetView` 不受 scissor 约束**（D3D12 命令列表上的
  clear **会**被裁剪）。所以本后端的清屏永远是全屏的，脏区只省绘制不省 clear。

  ⚠️ 节点级失效（只重算脏子树的 `DisplayList` 分片）**明确不做**：要改
  `DisplayList` 的语义（从「整树重渲」变成「拼接陈旧片段」），而合批跨节点
  （`batches` 合并连续同纹理图像），按节点切缓存会打散合批、draw call 变多，
  且**省不了 `expand`**。收益约 40%，代价是上面两条。
- **无 GPU 资源预算**：没有显存上限、没有 `purgeUnlockedResources` 那套主动回收。
  主机侧的纹理与缓冲都没有预算概念，量一多只能靠各自的容量上限兜着。
  （⚠ 字形图集**不在**此列：它满时会按策略淘汰并重排，见下。）
- **输入设备枚举**：`input` 模块产出的是手势词汇表，真正的设备枚举在宿主。
- ~~**非 Windows 呈现路径**~~ —— **已做**（2026-10）：`zephyra-winit-vk`
  （winit + wgpu，Vulkan 优先、GL 亦可）。它是**独立 crate**，与两个 Windows 后端
  互不依赖，理由逐条相同。`zephyra-winit` / `zephyra-winit-d3d12` /
  `zephyra-dcomp` / `zephyra-platform-win` 仍是 `#![cfg(windows)]`（在别的平台上
  是空 crate，而不是 `compile_error!`）。

  ⚠ 剩下的缺口是 **14 档 PS 变体混合模式**（`Overlay`..`Luminosity`）。WGSL 里的
  入口已经写好，但**没接线**：wgpu 下「正在渲染的附件」不能被采样（MSAA 更是根本
  不支持采样），接通需要「resolve 到中间纹理 → 拷贝 → 第二个通道」。现状是照常画、
  退回 `SrcOver`，并计入 `RenderReport::unimplemented_blends`。
- **无 DComp 等价物**：Linux 的合成由合成器 / wgpu surface 负责，本层不再单独
  提供一条呈现路径（Windows 那边的 `zephyra-dcomp` 是为了让 flip model 与
  DirectComposition 两条路并存而拆出来的）。
- **效果（blur / 投影 / 混合）的 GPU 执行**：三个后端都实现了
  「内容 → 模糊（乒乓）→ 合成」与 PSO 混合变体。`RenderReport` 保留
  `unimplemented_filters` / `unimplemented_blends` 两个计数器，它们的用途是
  **区分"没人用效果"与"效果被静默忽略"** —— 命令**照常画**，不 skip，所以症状
  是"设了没反应"而不是"图元消失"。`zephyra-winit-vk` 上 `unimplemented_filters`
  实测为 0（demo 的 3 条带滤镜命令全部走离屏），`unimplemented_blends` 为 2
  （demo 里那两档 PS 变体模式，见上面「14 档」那条）。
  命令**照常画**（不 skip），所以症状是"设了 blur 没反应"，而不是"元素消失"。
  `demo --dump-cpu` 会打印这两个计数与 RT 规划。

//! ZephyraUI 的 D3D12 渲染后端。
//!
//! # 与 `zephyra-winit`（D3D11）的关系
//!
//! **两个完全独立的 crate**，互不依赖。共同依赖的只有 `zephyra-core`（核心）与
//! `zephyra-fx`（后端无关的渲染层）。
//!
//! 这么分而不是用一个 crate 加 `#[cfg]`，是因为 D3D11 与 D3D12 的失败模式**长得
//! 几乎一样**：两者都是"不报错但什么都不画"。混在一个 crate 里时，一旦画面为空，
//! 你分不清是后端 A 的静默失效还是后端 B 的 —— 而这两种要查的完全不同。
//!
//! # ✅ 状态：离屏渲染与呈现链路都已真机验证
//!
//! 这里曾记着「本机 D3D12 的资源创建整体不可用」与「`CreateGraphicsPipelineState`
//! 整体不可用」。**两个结论都是错的** —— 而它们每一个都由**我自己写的非法参数**造成，
//! 当时一律被记成了「环境问题」。
//!
//! | # | 症状（全是一条裸 `E_INVALIDARG` 或崩溃） | 根因 |
//! |---|---|---|
//! | 1 | `CreateCommittedResource` 全适配器失败 | 缓冲 `Layout` 写成 `UNKNOWN`（须 `ROW_MAJOR`） |
//! | 2 | 离屏渲染目标建不出来 | `texture_desc` 写死 `Flags = NONE`（缺 `ALLOW_RENDER_TARGET`） |
//! | 3 | `CreateGraphicsPipelineState` 失败 | `RTVFormats` 8 个槽全填 `R8G8B8A8_UNORM`，第 1..8 槽须 `UNKNOWN` |
//! | 4 | 回读建不出来 | 在 `READBACK` 堆上建了**纹理**（该堆只能放缓冲） |
//! | 5 | `CopyTextureRegion` 报 `Type` 错 | 目标是缓冲却用 `SUBRESOURCE_INDEX`（该配 `PLACED_FOOTPRINT`） |
//! | 6 | `Map` 之后 AV | 把 `D3D12_RANGE::End` 当行距，本机并未改写它 |
//!
//! 而「问不到原因」本身也是三个我的 bug：`EnableDebugLayer` 在**建设备之后**才调
//! （文档要求之前）、消息队列从**全局** `ID3D12Debug` 取（它只在挂在**设备**上的队列里）、
//! `drain` 只给 32 字节缓冲而带描述的消息要 153 字节 —— 于是每条消息都被静默跳过。
//! 与 `zephyra-winit`（D3D11）在那里踩的是**同一个坑**。
//!
//! **教训**：扫描能告诉你「哪些组合能过」，给不出「为什么」。正确顺序是**先让调试层
//! 能说话，再问驱动**。修好之后第一句话就是答案：
//! `RTVFormats[] must be DXGI_FORMAT_UNKNOWN for all indices >= NumRenderTargets.`
//!
//! 另有一处**改错了又改回来**的：PSO 的 `NodeMask` 曾被从 0 改成 1，理由是「单节点
//! API 必须恰好设一位」。那是**推理**而非依据 —— 那条规则只适用于命令队列 / 命令
//! 列表 / 描述符堆 / 查询堆，PSO 不在清单里。官方文档明写单 GPU 时 PSO 的
//! `NodeMask` 就是 0，且驱动会把 0 转成掩码 1。改完没验证问题是否解决（并没有），
//! 却一直没撤回。详见 `renderer::PSO_NODE_MASK`。
//!
//! # 模块
//!
//! | 模块 | 职责 | 平台相关 |
//! |---|---|---|
//! | [`zephyra_fx::quad`] | `DisplayList` → 平台无关顶点流 | **无** |
//! | [`zephyra_fx::text`] | ab_glyph 字形来源、字形图集打包 | **无** |
//! | [`zephyra_fx::cpu_raster`] | CPU 参考光栅器（无 GPU 也能出画面） | **无** |
//! | [`zephyra_fx::texture`] | 已注册纹理的**名字**表 | **无** |
//! | [`shader`] | 预编译字节码、顶点布局、阈值配对 | 极少 |
//! | [`renderer`] | 根签名、PSO、描述符堆、流缓冲、绘制命令的**记录** | 有 |
//! | [`surface`] | 设备、描述符堆、离屏目标、栅栏、调试层 | 有 |
//! | [`swapchain`] | 交换链、后缓冲 RTV、资源屏障、呈现、resize | 有 |
//!
//! # 之前这里有一份 `quad.rs` 的**逐字节副本**，现已抽走
//!
//! 原来的理由是「两个后端各有独立编译边界，复制比抽象便宜」。**那个论证是错的**，
//! 代价不是"改一处忘一处"，而是更直接的两条：
//!
//! 1. DX12 后端**根本没有字体、没有纹理、也没法做 CPU 侧诊断** ——
//!    `text` / `texture` / `cpu_raster` / `cpu_only` 全都只存在于 D3D11 那个 crate
//!    里，因为它们被放在了「D3D11 后端」里，尽管一个 GPU 类型都不碰。
//! 2. 于是「顶点流对不对」这个问题在 DX12 上**无法回答** —— 而这恰恰是本项目
//!    反复需要回答的问题。
//!
//! 现在共享部分住在 `zephyra-fx`，本 crate 与 `zephyra-winit` **互不依赖**，
//! 只共同依赖它。契约一致性不再靠「记得同步两份」，而是**只有一份可不同步**。
//!
//! # 记录 vs 提交
//!
//! D3D11 的 `render()` 里所有调用**立即生效**；D3D12 的同名调用只是**往命令列表里
//! 记一条命令**，要 [`surface::Gpu::submit`] 之后才交给 GPU。所以
//! [`renderer::Renderer::record`] **只记录**，不提交 —— 提交与等待交给宿主，这样
//! 「记录 / 提交 / 等待」三步在测试里都看得见，而不是被一个函数悄悄合在一起。
//!
//! # 换机器后第一件事
//!
//! ```powershell
//! cargo test -p zephyra-winit-d3d12 -- --ignored --nocapture
//! ```
//!
//! 判定标准（顺序不能颠倒）：
//!
//! 1. `d3d12_最小资源能不能建出来` 通过 → 环境没问题，继续；
//! 2. `d3d12_能把四边形画到像素上` 通过 → 后端真的能画出东西，收工；
//! 3. 第 1 个过、第 2 个不过 → **这才是本 crate 的问题**，从第 2 个测试的失败信息查。
//!
//! 只有第 3 种情况才值得继续读后端代码。反过来（前两个都不过）说明是环境问题，
//! 再读代码是浪费时间。
#![cfg(windows)]

pub mod host;
pub mod renderer;
pub mod shader;
pub mod surface;
pub mod swapchain;

// 后端无关的那一半在 `zephyra-fx`，不再在本 crate 里留副本。
// 按原路径再导出，是为了让既有调用点（`zephyra_winit_d3d12::Vertex`）不用改。
pub use zephyra_fx::{cpu_only, cpu_raster, pixels, quad, text};
pub use zephyra_fx::{PixelRead, RenderReport, TextureNames};
pub use zephyra_fx::quad::Vertex;
pub use zephyra_fx::layout_tree;

pub use renderer::{RenderCtx, Renderer, TextureTable};
pub use shader::{ShaderError, Shaders};
pub use surface::{DebugInfo, DescriptorHeap, Gpu, Offscreen, Staging, SurfaceError};
// 设备消失 / resize 失败的「这一帧该不该画」判据。见 `host::should_draw_frame`
// 与 `tests/device_lost_gate.rs`：它是纯函数，所以能在没有 GPU 的机器上门禁。
pub use host::{should_draw_frame, SwapState};
pub use swapchain::Swapchain;
pub use host::{run_with, HostWindow};
pub use zephyra_fx::{Frame, InputState, View, Z_INDEX};
pub use winit;

pub use zephyra_fx;

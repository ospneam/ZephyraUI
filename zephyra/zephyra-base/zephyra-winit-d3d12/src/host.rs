//! 窗口宿主：winit 事件循环 + DComp 视觉树 + D3D12 帧循环。
//!
//! # 与 `zephyra-winit`（D3D11）的关系
//!
//! 两个 crate **互不依赖**，只共同依赖 `zephyra-core`（核心）与 `zephyra-fx`（后端无关
//! 的渲染层）。所以本文件与 D3D11 那份 `HostWindow` 是**两份实现**，不是一份代码两处
//! 条件编译 —— 理由见 `lib.rs`：D3D11 与 D3D12 的失败模式长得几乎一样，混在一个 crate
//! 里时画面一空就分不清是哪个后端的问题。
//!
//! # 字段顺序**不是**随意的（析构约束）
//!
//! Rust 按**声明顺序**析构字段（先用一段小程序实测过：先声明的先 `drop`）。而 DComp
//! 有两条硬约束：
//!
//! 1. `DcompCompositor` 持有 `target` / `root` / `visual`，它必须**比视觉内容**
//!    （也就是交换链）**活得久** —— 否则 visual 还指着已释放的 swapchain，窗口全白；
//! 2. 它又必须**比窗口活得久** —— target 绑在 `HWND` 上，窗口先死就没有「拆除绑定」
//!    这回事了，只是关窗瞬间闪一下黑。
//!
//! 于是顺序必须是「交换链先死 → 合成器 → …… → 窗口最后死」，也就是**反过来声明**：
//! [`Swapchain`] 在最前，[`Window`] 在最后。
//!
//! ⚠ 另一个 crate 曾在注释里把这个结论写反（说「surface 必须声明在 window 之后」），
//! 而那与实测的析构顺序相反。判据不是注释，是上面那两条约束 + 一次实测。

use std::sync::Arc;

use winit::event::WindowEvent as WEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::event::ElementState;
use winit::window::{Window, WindowId};
use windows::Win32::Foundation::HWND;

use windows::core::Interface;

use zephyra_core::context::Context;
use zephyra_core::diff::DiffStats;
use zephyra_core::frame::{Damage, FrameCache, FrameOutcome};
use zephyra_core::id::ElementKey;
use zephyra_core::input::{InputRouter, PointerEvent, PointerPhase, PointerSource, RoutedEvents};
use zephyra_core::node::Node;
use zephyra_core::theme::Theme;

use zephyra_fx::{InputState, TextEngine, View};

use zephyra_dcomp::{DcompCompositor, SwapchainDesc, VisualSlot};

use crate::renderer::{RenderCtx, Renderer, TextureTable};
use crate::surface::Gpu;
use crate::swapchain::Swapchain;
use crate::{Frame, RenderReport};

/// 窗口 + GPU + 保留树的组合体。
///
/// 保留树（[`Node`]）跨帧存活 —— 这正是保留式相对即时式的收益：状态与纹理得以复用，
/// 只有真正变化的子树被重排/重绘。
///
/// # 字段顺序是**硬约束**，见模块文档
pub struct HostWindow<V: View> {
    /// 交换链（视觉的内容）。**最先析构**。
    chain: Swapchain,
    /// DComp 合成器（持有 target / root / visual）。比 `chain` 活得久，比 `window` 短。
    compositor: Option<DcompCompositor>,
    /// 挂着交换链的那个 visual。单独存一份是为了「交换链重建后重新挂上去」。
    visual: Option<VisualSlot>,
    /// D3D12 设备、命令队列、命令列表、栅栏（全进程唯一一份）。
    gpu: Gpu,
    renderer: Renderer,
    view: V,
    theme: &'static Theme,
    /// 帧缓存 —— 保留树 + `DisplayList` + 上一帧视口，全在核心里。
    ///
    /// 理由与 `zephyra-winit` 那份**逐字相同**：这三样原来分散在本结构体的
    /// `node` / `list` / `needs_layout` 里，而数据全在 core，于是本 crate 要自己
    /// 编排「要不要重排 / 要不要重绘」—— 那正是 base 层第二条契约禁止的。
    cache: FrameCache,
    router: InputRouter,
    pointer: Option<u32>,
    pointer_pos: (f32, f32),
    /// 有没有**待处理的事件**（resize / 指针 / 按键 / 图集变化）。
    ///
    /// ⚠ 这**不是**「内容变了」。内容变没变由 core 判定（`FrameOutcome`）。
    dirty: bool,
    /// 文本引擎。用 `RefCell` 是为了让 core 的 `Context` 保持 `Copy` —— 布局递归要求
    /// 这一点。
    engine: std::cell::RefCell<TextEngine>,
    /// 已上传的图集字形数，用来避免每帧重传 4MB。
    atlas_revision_uploaded: u64,
    /// 帧计数。只给 `trace` 用。
    frames: usize,
    /// 图像纹理表：`Command::Image` 的句柄 → SRV 描述符位置。
    textures: TextureTable,
    /// 窗口。**最后析构**，见模块文档。
    window: Arc<Window>,
}

impl<V: View> HostWindow<V> {
    /// 建窗口与 GPU 资源，**找不到系统字体时不 panic**。
    ///
    /// 字体缺失不该让整个应用起不来：文本不显示，其余部分照常。这与「崩溃」之间的
    /// 差别很大 —— 后者让「这台机器没装字体」变成「这个程序不能跑」。
    pub fn new_lenient(event_loop: &ActiveEventLoop, view: V) -> Self {
        let attrs = Window::default_attributes().with_title("ZephyraUI (D3D12)");
        let window = Arc::new(
            event_loop
                .create_window(attrs)
                .expect("create_window（D3D12 后端）"),
        );
        let size = window.inner_size();

        // 文本引擎：core 的 `TextStack` + ab_glyph 字形来源。找不到字体时退回禁用模式。
        let (engine, note) = zephyra_fx::from_system_font();
        eprintln!("[zephyra-d3d12] {note}");

        let gpu = Gpu::new().expect("创建 D3D12 设备：需要可用的 GPU");

        // 后缓冲格式必须与 PSO 的 `RTVFormats[0]` 一致（`R8G8B8A8`）。用
        // `SwapchainDesc::default()` 的 `B8G8R8A8` 会让 D3D12 在 `DrawIndexedInstanced`
        // 时逐槽比对并报「render target format does not match」，而画面全错、
        // 不报任何 HRESULT。
        let desc = SwapchainDesc {
            buffer_count: 2,
            format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_R8G8B8A8_UNORM,
        };
        let chain = Swapchain::new(&gpu, (size.width, size.height), &desc, crate::swapchain::MSAA_SAMPLES)
            .expect("创建 composition swapchain");

        // composition swapchain **必须**挂在 visual 上才会被合成，否则窗口是黑的。
        // 所以这里失败要如实说出来，不能只打一行 warn 就继续跑。
        let (compositor, visual) = match hwnd_of(window.as_ref()) {
            Some(h) => match attach_visual(HWND(h as *mut _), &gpu, &chain) {
                Ok((c, v)) => {
                    eprintln!("[zephyra-d3d12] 呈现路径: DirectComposition");
                    (Some(c), Some(v))
                }
                Err(e) => {
                    eprintln!("[zephyra-d3d12] ⚠ DComp 建树失败（{e}）");
                    eprintln!(
                        "[zephyra-d3d12] ⚠ composition swapchain 没有 visual 就不会被合成，\
                         窗口会是黑的（这一条不可能靠参数调好）"
                    );
                    (None, None)
                }
            },
            None => {
                eprintln!("[zephyra-d3d12] ⚠ 取不到 HWND，无法建立 DComp target，窗口会是黑的");
                (None, None)
            }
        };

        let atlas_size = zephyra_core::text::atlas::ATLAS_SIZE;
        // ⚠ 采样数必须**取自交换链**，不能在这里另写一个常量 —— PSO 建好就固定了，
        // 而它必须与实际绑定的渲染目标一致（见 `Swapchain::samples` 的文档）。
        let renderer = Renderer::new(&gpu.device, atlas_size, chain.samples())
            .expect("建根签名 / PSO / 描述符堆");
        eprintln!(
            "[zephyra-d3d12] MSAA: {}×",
            if chain.samples() > 1 { "开" } else { "关（无多重采样目标）" }
        );

        let theme: &'static Theme = Box::leak(Box::new(Theme::dark()));
        Self {
            chain,
            compositor,
            visual,
            gpu,
            renderer,
            view,
            theme,
            cache: FrameCache::new(),
            router: InputRouter::new(),
            pointer: None,
            pointer_pos: (0.0, 0.0),
            dirty: true,
            engine: std::cell::RefCell::new(engine),
            atlas_revision_uploaded: 0,
            frames: 0,
            textures: TextureTable::new(),
            window,
        }
    }

    /// 图像纹理表（只读）。
    pub fn textures(&self) -> &TextureTable {
        &self.textures
    }

    /// 图像纹理表（可变）。宿主在这里注册 / 更新纹理。
    pub fn textures_mut(&mut self) -> &mut TextureTable {
        &mut self.textures
    }

    /// 注册一张图像纹理，返回写进 `props::IMAGE_TEXTURE` 的槽位句柄。
    ///
    /// D3D12 里「SRV」不是对象而是**描述符堆里的槽位**，所以这里返回 `u32` 而不是
    /// 一个 `IShaderResourceView` 指针。
    ///
    /// # 为什么走一条**独立**的提交
    ///
    /// 上传要发屏障 + `CopyTextureRegion`，那是命令列表上的一条命令。而注册通常发生在
    /// 建窗那一帧的中途（此刻帧还没 `begin_frame`）。所以这里用
    /// [`Gpu::submit_one_off`] 单独开一条命令列表、提交、**并等 GPU 做完** ——
    /// 等这一手是必须的：纹理与暂存缓冲随即就要被下一帧的 draw 读到，
    /// 而「下一帧」不会自动等这一条命令。
    pub fn register_texture(
        &mut self,
        name: &str,
        pixels: &[u8],
        w: u32,
        h: u32,
    ) -> Result<u32, String> {
        let mut slot = None;
        if let Err(e) = self.gpu.submit_one_off(|list| {
            slot = Some(self.renderer.upload_texture(list, pixels, w, h));
        }) {
            return Err(format!("提交纹理上传命令失败: {e}"));
        }
        let slot = slot.expect("submit_one_off 一定跑一遍回调");
        let slot = slot.map_err(|e| format!("上传纹理 {name}: {e}"))?;
        self.textures.insert(name.to_string(), slot);
        Ok(slot)
    }

    /// 窗口（供宿主设置标题等）。
    pub fn window(&self) -> &Arc<Window> {
        &self.window
    }

    /// 客户区像素尺寸。
    ///
    /// 最小化时 w/h 可能是 0，而布局拿 0 去算会让所有元素的尺寸塌成 0、恢复后第一帧
    /// 全错。所以这里**夹到至少 1** —— 那不是「修正尺寸」，是「不让布局看到 0」。
    pub fn client_size(&self) -> (u32, u32) {
        let s = self.window.inner_size();
        (s.width.max(1), s.height.max(1))
    }

    /// 当前 hover 的节点 key。
    pub fn hovered(&self) -> Option<&ElementKey> {
        self.router.hovered()
    }

    /// 拉取调试层消息（`ZEPHYRA_D3D12_DEBUG=1` 时才有内容）。
    pub fn drain_debug(&self) {
        self.gpu.drain_debug();
    }

    /// 跑一帧：布局 → `DisplayList` → 顶点展开 → 记录 → 提交。
    ///
    /// **不**包含 `present` —— 那由 [`Self::present`] 单独做，因为 resize 必须在
    /// 绘制之前处理（见 `sync_swapchain`），而呈现必须在绘制之后。
    pub fn frame(&mut self) -> Frame {
        // ⚠ resize 必须在**布局与绘制之前**，见 [`Self::sync_swapchain`] 的说明。
        // 顺序反了会留下一个「从未被写过的新后缓冲」，而且它可能永远等不到下一帧。
        self.sync_swapchain();
        self.drain_debug();

        let theme = self.theme;
        // 作用域要短：`ctx` 借着 `self.engine`，而下面还要可变借用 `self`。
        //
        // 在作用域**内**就把 `FrameOutcome` 拆成 `Copy` 标量：`Updated` 借着一份
        // `&DisplayList`，带出作用域会让借用活到函数末尾，与下面的
        // `&mut self` 冲突。列表留到 `render_pass` 里用 `self.cache.list()` 现取。
        let (stats, damage, redrawn) = {
            // 挂上**内置**图标集 —— 于是矢量图标在这条路径上也走完整的
            // 「图标名 → 展平 → 三角化 → `Command::Path`」链条。
            //
            // 用内置表而不是让宿主登记自己的：内置表是编译期常量（`assets/*.svg`
            // 经 `include_str!`），零运行时准备成本，且 `OnceLock` 只解析一次。
            // 应用自己的图标库应当自己建一个 `IconSet` 再挂进来 —— 那是宿主的事，
            // 与纹理句柄同一层。
            let ctx = Context::with_engine(theme, &self.engine)
                .with_icons(zephyra_core::icon::builtin());
            let element = self.view.view(&ctx);
            let size = self.client_size();

            // ⚠️ 图集是「内容变了」的一个**独立**来源，而 core 的判据只看节点脏标记，
            // 看不见纹理侧的变化。所以图集一改就强制重绘。
            //
            // 真实触发路径：图集满时的**淘汰重排**（`make_room`）发生在 layout 的
            // measure 阶段，那时可能一个节点都没 paint-dirty ⇒ core 返回
            // `Unchanged` ⇒ 本帧不回放 ⇒ GPU 抱着**重排前**的纹理。症状是
            // 「文字忽然整片消失，或每个字跑到别的位置上」，且**只在图集填满后出现**。
            //
            // 判据是 `atlas_revision()` 而不是「已缓存字形数」：加了 LRU 淘汰之后
            // 「数量不变 ⇒ 内容不变」不成立 —— 淘汰一个再放进一个，数量不变而纹理已
            // 整体重排。
            let revision_now = self.engine.borrow().atlas_revision();
            if revision_now != self.atlas_revision_uploaded {
                self.cache.invalidate();
            }

            // 「这一帧画什么」由核心里唯一的那份判据决定，见
            // `zephyra_core::frame::FrameCache`。
            match self.cache.frame(&element, ctx, (size.0 as f32, size.1 as f32)) {
                FrameOutcome::Unchanged => (DiffStats::default(), Damage::Full, false),
                FrameOutcome::Updated { stats, damage, .. } => (stats, damage, true),
            }
        };

        // ⚠ `Unchanged` 时**连 present 都不能做**：本后端是 flip model
        // （`FLIP_DISCARD`，DComp 路径 `FLIP_SEQUENTIAL`），`Present` 之后后缓冲内容
        // 未定义。不画就 present 会把未定义内容推上屏。
        let report = if redrawn {
            self.render_pass(damage)
        } else {
            RenderReport::default()
        };

        if trace_enabled() {
            self.frames += 1;
            // 首帧与之后每 60 帧：稳态下这些数恒定，一变就说明布局或图集在漂。
            if self.frames == 1 || self.frames.is_multiple_of(60) {
                // `kind` 直方图：片元着色器**只**按 `kind` 分支，所以「矩形出来了、
                // 字形没出来」这件事一定能在这一行里看到（是 `GLYPH` 一个都没有，
                // 还是有一堆但采样不出来）。这两者的排查方向完全不同。
                let mut kinds = [0usize; 3];
                for c in self.list_kind_histogram() {
                    kinds[c] += 1;
                }
                eprintln!(
                    "[trace] 第 {} 帧：四边形 {}（跳过 {}）、kind 矩形/字形/图像 {}/{}/{}、\
                     图像批 {}、图集字形 {}、顶点 {} 字节、窗口 {:?}、交换链 {:?}",
                    self.frames,
                    report.quads,
                    report.skipped,
                    kinds[0],
                    kinds[1],
                    kinds[2],
                    report.image_batches,
                    self.engine.borrow().atlas_stats().glyphs,
                    report.vertex_bytes,
                    self.client_size(),
                    self.chain.size(),
                );
            }
        }

        // 3. 清事件标志。脏标记由 `FrameCache` 内部清（它知道什么时候该清）。
        self.dirty = false;

        Frame {
            diff: stats,
            render: report,
            hovered: self.router.hovered().map(|k| k.to_string()),
            damage,
            redrawn,
        }
    }

    /// 一帧的 GPU 部分。
    ///
    /// # 图集上传与绘制**必须同帧**
    ///
    /// 图集纹理是每帧重建的（`record_upload_atlas` 内部建暂存 + 拷贝 + 销毁），所以
    /// 「先上传、等一帧、再画」会在中间那帧读到**已销毁**的纹理资源。
    ///
    /// 因此这里把两件事记进**同一条命令列表**：要么都成，要么都不成。
    fn render_pass(&mut self, damage: Damage) -> RenderReport {
        if let Err(e) = self.gpu.begin_frame() {
            eprintln!("[zephyra-d3d12] 开帧失败: {e}");
            self.drain_debug();
            return RenderReport::default();
        }

        // 图集：只有**位图内容**变了才传。判据是 `atlas_revision()`，不是字形数 ——
        // 淘汰一个再进一个时字形数不变而纹理已整体重排，用数量当判据会让 GPU 一直
        // 抱着旧图集（画面上字全跑到别的位置）。每帧传 4MB 又会让稳态多几毫秒。
        let revision_now = self.engine.borrow().atlas_revision();
        if revision_now != self.atlas_revision_uploaded {
            if let Err(e) = self
                .renderer
                .record_upload_atlas(&self.gpu.list, self.engine.borrow().atlas().pixels())
            {
                eprintln!("[zephyra-d3d12] 图集上传失败（{e}），本帧文字可能不显示");
            }
            self.atlas_revision_uploaded = revision_now;
        }

        // 后缓冲在每帧开头都是 `PRESENT`。不转就没有渲染目标 —— 且**不报任何错**。
        //
        // MSAA 开启时它**仍然要转**：它是 resolve 的目标，而 D3D12 不为 resolve
        // 自动转换状态。所以这一段与 MSAA 无关，始终执行。
        if let Err(e) = self.chain.barrier_to_render_target(&self.gpu.list) {
            eprintln!("[zephyra-d3d12] 后缓冲转 RENDER_TARGET 失败: {e}");
            return RenderReport::default();
        }
        // ⚠ MSAA 开启时渲染目标是**多重采样纹理**，不是后缓冲。
        //
        // PSO 的 `SampleDesc.Count` 与 RTV 的必须一致，所以这一步不能"看情况"：
        // `samples` 决定用哪一个，MSAA 关闭时 `msaa_rtv()` 明确报错而不是回退成
        // 后缓冲 —— 回退会让"采样数不一致"这个真错误变成"画面全黑且零 HRESULT"。
        let rtv = if self.chain.samples() > 1 {
            match self.chain.msaa_rtv() {
                Ok(h) => h,
                Err(e) => {
                    eprintln!("[zephyra-d3d12] 取不到 MSAA 的 RTV: {e}");
                    return RenderReport::default();
                }
            }
        } else {
            match self.chain.current_rtv() {
                Ok(h) => h,
                Err(e) => {
                    eprintln!("[zephyra-d3d12] 取不到后缓冲的 RTV: {e}");
                    return RenderReport::default();
                }
            }
        };
        let clear = self.theme.surface;
        // 可分离 / 非可分离混合模式要读 `dst`，必须把后缓冲当**资源**传进去。
        // 取失败时传 `None`：那 14 个模式会退回 `SrcOver`，画面仍然正确只是少了效果。
        let rt = self.chain.back_buffer().ok();
        let report = match self.renderer.record(RenderCtx {
            device: &self.gpu.device,
            list: &self.gpu.list,
            rtv,
            display_list: self.cache.list(),
            damage,
            size: self.chain.size(),
            clear,
            textures: &self.textures,
            rt: rt.as_ref(),
        }) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("[zephyra-d3d12] 记录绘制命令失败: {e}");
                RenderReport::default()
            }
        };
        // ⚠ MSAA 解析必须在 `barrier_to_present` **之前**、且必须画完之后。
        //
        // 画进 MSAA 纹理的内容不会出现在后缓冲上 —— 漏掉这步的症状是「画面卡在
        // 旧内容」或「全黑」，且零 HRESULT 错误。顺序也不能反：先转回 PRESENT
        // 再 resolve 是非法的（resolve 目标必须在 RENDER_TARGET）。
        if self.chain.samples() > 1 {
            if let Err(e) = self.chain.resolve_msaa(&self.gpu.list) {
                eprintln!("[zephyra-d3d12] MSAA 解析失败: {e}");
            }
        }
        if let Err(e) = self.chain.barrier_to_present(&self.gpu.list) {
            eprintln!("[zephyra-d3d12] 后缓冲转回 PRESENT 失败: {e}");
        }
        if let Err(e) = self.gpu.submit() {
            eprintln!("[zephyra-d3d12] 提交失败: {e}");
        }
        report
    }

    /// 呈现一次。
    ///
    /// **不含 resize** —— 见 `sync_swapchain`：resize 必须发生在**绘制之前**，
    /// 否则新建的后缓冲要到下一次 `frame()` 才有内容，而那一次可能永远不会来。
    pub fn present(&mut self) {
        if let Err(e) = self.chain.present(1) {
            eprintln!("[zephyra-d3d12] present 失败: {e}");
            self.drain_debug();
        }
    }

    /// 把交换链对齐到窗口尺寸。**返回尺寸是否变了**（变了就要重排）。
    ///
    /// # 为什么在**每帧开头**做，而不是画完再 resize
    ///
    /// 「先画、再 resize」有一个致命后果：本机 `ResizeBuffers` 一旦画过就回
    /// `DXGI_ERROR_INVALID_CALL`（见 [`Swapchain::resize`] 的实测对照表），于是
    /// [`Swapchain::resize`] 改为**重建整条交换链**。重建后的后缓冲是**从未被写过的**
    /// —— 而画它的那一帧已经过去了。若此后没有新的脏（窗口不缩放不移动），就再没有
    /// `frame()`，于是**永久**停在那一块未初始化的缓冲上：屏幕上是一块纯白（或纯黑）
    /// 矩形，外面是窗口背景。
    ///
    /// 先 resize 再画，那一帧就直接画进新缓冲，没有中间态。
    ///
    /// # 重建后必须重新挂 DComp 视觉树
    ///
    /// visual 上挂的是**旧的** swapchain 对象。新的那个没人引用，合成器不会去取它 ——
    /// 于是「提交了、也没报错，就是不显示」。这与「没提交」在屏幕上完全一样。
    fn sync_swapchain(&mut self) -> bool {
        let (w, h) = self.client_size();
        if (w, h) == self.chain.size() {
            return false;
        }
        if let Err(e) = self.chain.resize(&self.gpu, (w, h)) {
            eprintln!("[zephyra-d3d12] resize 到 {w}×{h} 失败: {e}");
            return false;
        }
        if self.chain.was_recreated() {
            eprintln!("[zephyra-d3d12] 交换链已重建，重新挂进 DComp 视觉树");
            self.rebind_visual();
        }
        true
    }

    /// 把（可能已被重建的）交换链重新挂到 visual 上并提交。
    fn rebind_visual(&mut self) {
        let (Some(comp), Some(vis)) = (self.compositor.as_ref(), self.visual.as_ref()) else {
            return;
        };
        if let Err(e) = comp.set_content_swapchain(vis, self.chain.as_swap_chain1()) {
            eprintln!("[zephyra-d3d12] 重新挂 visual 失败: {e}");
            return;
        }
        if let Err(e) = comp.commit() {
            eprintln!("[zephyra-d3d12] DComp commit 失败: {e}");
        }
    }

    /// 把上一帧的顶点流按 `kind` 归类（0 矩形 / 1 字形 / 2 图像），逐个返回。
///
/// 只给 `trace` 用 —— 它要重跑一次 `quad::expand`，不该进稳态路径。
fn list_kind_histogram(&self) -> Vec<usize> {
    use zephyra_fx::quad::{self, kind};
    let size = self.chain.size();
    let batch = quad::expand(self.cache.list(), size, |n| self.textures.contains(n));
    batch
        .solid
        .iter()
        .chain(batch.images.iter())
        .step_by(4)
        .map(|v| {
            if (v.kind - kind::GLYPH).abs() < 0.01 {
                1
            } else if v.kind > 1.5 {
                2
            } else {
                0
            }
        })
        .collect()
}

/// 把新输入状态推给视图（每帧开头一次）。
    pub fn sync_input(&mut self) {
        let state = InputState {
            hovered: self.router.hovered().cloned(),
            pressed: self.router.pressed().cloned(),
            pointers: self.router.pointer_count(),
        };
        self.view.on_input(&state);
    }

    /// 处理指针移动。返回是否产生了路由事件。
    pub fn on_cursor_moved(&mut self, x: f32, y: f32) -> bool {
        self.pointer_pos = (x, y);
        let theme = self.theme;
        self.route(|router, node, out| {
            router.on_pointer(
                node,
                PointerEvent {
                    id: 0,
                    x,
                    y,
                    phase: PointerPhase::Move,
                    source: PointerSource::Mouse,
                },
                Context::new(theme),
                out,
            );
        })
    }

    /// 处理一次按下。返回是否产生了路由事件。
    pub fn on_pointer_down(&mut self) -> bool {
        self.pointer = Some(0);
        let (x, y) = self.pointer_pos;
        let theme = self.theme;
        self.route(|router, node, out| {
            router.on_pointer(
                node,
                PointerEvent {
                    id: 0,
                    x,
                    y,
                    phase: PointerPhase::Down,
                    source: PointerSource::Mouse,
                },
                Context::new(theme),
                out,
            );
        })
    }

    /// 处理一次抬起。返回是否产生了路由事件。
    pub fn on_pointer_up(&mut self) -> bool {
        self.pointer = None;
        let (x, y) = self.pointer_pos;
        let theme = self.theme;
        self.route(|router, node, out| {
            router.on_pointer(
                node,
                PointerEvent {
                    id: 0,
                    x,
                    y,
                    phase: PointerPhase::Up,
                    source: PointerSource::Mouse,
                },
                Context::new(theme),
                out,
            );
        })
    }

    /// 统一的「跑一次路由器 + 置脏」包装。
    fn route<F>(&mut self, f: F) -> bool
    where
        F: FnOnce(&mut InputRouter, &Node, &mut RoutedEvents),
    {
        let Some(node) = self.cache.node() else {
            return false;
        };
        let before = self.router.revision();
        let mut out = RoutedEvents::new();
        f(&mut self.router, node, &mut out);
        let changed = self.router.revision() != before;
        if changed {
            self.dirty = true;
        }
        changed
    }
}

/// 每帧把关键计数打到 stderr（`ZEPHYRA_D3D12_TRACE=1` 时才开）。
///
/// # 为什么需要它
///
/// 「窗口全白」和「文字不见了」都是**只有画面能证明**的现象，而在没有窗口的测试里
/// 两者都看不见 —— `d3d12_draw` 与 `glyph_draw` 在离屏上都过，窗口却仍是错的。
/// 所以宿主必须能**自己说**出「这一帧展开出多少四边形、图集里有多少字形、窗口与
/// 交换链各是多大」，否则每次都得靠「截屏 → 肉眼 → 猜」。
fn trace_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("ZEPHYRA_D3D12_TRACE").map(|v| v != "0").unwrap_or(false))
}

/// 建 DComp target / visual 并把交换链挂上去。
fn attach_visual(
    hwnd: HWND,
    gpu: &Gpu,
    chain: &Swapchain,
) -> Result<(DcompCompositor, VisualSlot), String> {
    // DComp 设备要从渲染设备建；本机 D3D12 command queue 路线 `E_NOINTERFACE`，
    // 所以走带 fallback 的那个构造（v2 → v3 → null）。
    let unknown: windows::core::IUnknown = gpu
        .queue
        .cast()
        .map_err(|e| format!("command queue QI 成 IUnknown: {e}"))?;
    let mut comp = DcompCompositor::new_from_rendering_device(hwnd, Some(&unknown))
        .map_err(|e| format!("DCompositionCreateDevice: {e}"))?;
    let visual = comp
        .add_visual_on_top()
        .map_err(|e| format!("AddVisual: {e}"))?;
    comp.set_content_swapchain(&visual, chain.as_swap_chain1())
        .map_err(|e| format!("SetContent(swapchain): {e}"))?;
    comp.commit().map_err(|e| format!("commit: {e}"))?;
    Ok((comp, visual))
}

/// 从 winit 窗口取裸 HWND。失败返回 `None`（非 Win32 平台）。
fn hwnd_of<W: raw_window_handle::HasWindowHandle>(window: &W) -> Option<isize> {
    use raw_window_handle::RawWindowHandle;
    let handle = window.window_handle().ok()?;
    match RawWindowHandle::from(handle) {
        RawWindowHandle::Win32(h) => Some(h.hwnd.get()),
        _ => None,
    }
}

impl<V: View> winit::application::ApplicationHandler for HostWindow<V> {
    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {
        // 窗口在 `new_lenient` 里就建好了；这里补第一帧。
        self.sync_input();
        self.frame();
        self.present();
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _id: WindowId,
        event: WEvent,
    ) {
        match event {
            WEvent::CloseRequested => {
                event_loop.exit();
                return;
            }
            WEvent::Resized(_) => {
                // 只置脏，**不**在这里 resize：同一帧里「提交 + resize + 呈现」会打架。
                // 下一帧由 `frame()` 开头的 `sync_swapchain` 处理。
                self.dirty = true;
            }
            WEvent::ScaleFactorChanged { .. } | WEvent::RedrawRequested => {
                self.dirty = true;
            }
            WEvent::CursorMoved { position, .. } => {
                self.on_cursor_moved(position.x as f32, position.y as f32);
            }
            WEvent::MouseInput { state, .. } => match state {
                ElementState::Pressed => {
                    self.on_pointer_down();
                }
                ElementState::Released => {
                    self.on_pointer_up();
                }
            },
            WEvent::KeyboardInput { .. } => {
                self.dirty = true;
            }
            _ => {}
        }
        self.sync_input();
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        // 只在「有待处理的事件」时跑一帧。静止时一帧都不跑 —— 这既符合保留式 UI 的
        // 卖点，也让 demo 在静止时几乎没有 GPU 开销。
        //
        // ⚠ 是否**真的**要画由 core 判定（`Frame::redrawn`）：事件可能什么都没改
        // （鼠标在空白处移动 ⇒ `hovered` 前后都是 `None`），此时 core 返回
        // `Unchanged`，本帧不画。
        //
        // ⚠ 那时**连 present 都不能做**。本后端是 flip model（`FLIP_DISCARD`，
        // DComp 路径 `FLIP_SEQUENTIAL`），`Present` 之后后缓冲内容未定义 ——
        // 不画就 present 会把未定义内容推上屏（通常是黑，或上一次的帧）。
        //
        // ⚠ 这里**只**在画完之后 `present`，事件回调里不 present：那会在第一帧之前就
        // 呈现一个从未被写过的交换链（屏幕上一块纯白），也会在拖拽缩放时把
        // 「还没重建完」的状态推上屏。
        if self.dirty {
            let f = self.frame();
            if f.redrawn {
                self.present();
            }
        }
    }
}

/// 用给定视图跑一个 D3D12 窗口应用。
///
/// 与 `zephyra-winit::run_with` 同形：调用方若自举（`ApplicationHandler`），用它自己
/// 的那一套；否则用它。
pub fn run_with<V: View + 'static>(app: HostWindow<V>) -> Result<(), winit::error::EventLoopError> {
    let mut app = app;
    let el: EventLoop<()> = EventLoop::new()?;
    el.run_app(&mut app)
}

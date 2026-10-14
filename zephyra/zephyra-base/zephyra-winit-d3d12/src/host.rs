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
    /// resize 的状态（本帧）。`frame()` 据它决定要不要画。
    swap_state: SwapState,
    /// 连续 resize 失败的次数。只给 trace 用：它是「为什么拖动时不动了」的判据。
    resize_failures: u32,
    /// 客户区尺寸**最后一次变化**的时刻 + 当时的尺寸。拖动去抖用。
    ///
    /// # 为什么要有它
    ///
    /// 本机 `ResizeBuffers` 在后缓冲被用过之后就再也调不成功（实测矩阵见
    /// [`Swapchain::resize`] 文档），所以尺寸一变就只能**重建整条交换链**。而拖边时
    /// `WM_SIZE` 每秒来几十次 —— 每次都重建，实测一口气重建 140 次，GPU 直接进
    /// `DEVICE_HUNG`，屏幕上就是「一直闪」。
    ///
    /// 所以这里**不再跟着每一次 `WM_SIZE` 重建**：尺寸还在变的时候只把已有画面
    /// 缩放铺满窗口（见 [`Self::stretch_visual_to_client`]），等它**稳下来**才真的
    /// 重建一次。于是一次拖动 = 1 次重建，而不是 140 次。
    /// 「设备已不可用」是否已经报过一次。见 [`Self::report_device_dead_once`]。
    reported_device_dead: bool,
    /// 文本引擎。用 `RefCell` 是为了让 core 的 `Context` 保持 `Copy` —— 布局递归要求
    /// 这一点。
    engine: std::cell::RefCell<TextEngine>,
    /// 已上传的图集字形数，用来避免每帧重传 4MB。
    atlas_revision_uploaded: u64,
    /// 窗口背后**真的**挂着系统背景材质（Mica/Acrylic）。
    ///
    /// ⚠️ 记的是「实际生效」而不是「应用请求了」——平台不支持时后者为真、
    /// 前者为假，而两者对 clear 色（透明 vs 不透明）与根背景的处理**不同**。
    backdrop_active: bool,
    /// 帧计数。只给 `trace` 用。
    /// 系统语言偏好（首帧读一次后缓存）。
    ///
    /// 每帧问一次是一次 Win32 调用，而它是**启动时的外部事实**，之后不变
    /// （用户切换系统语言要重启进程才该重新读）。与版本号、系统主题同一手法。
    system_languages: Vec<String>,
    frames: usize,
    /// 上一次 `sync_input` 的时刻，用来算**真实帧间隔**（动画时钟的唯一来源）。
    ///
    /// `None` = 还没有上一帧，于是 dt 为 0。首帧没有"距上一帧多久"可言，
    /// 填 0 而不是填一个猜的值：猜的值会让首帧动画从中间开始。
    last_frame_at: Option<std::time::Instant>,
    /// 上一帧布局用的视口（有效像素）。**只给 trace 用**。
    ///
    /// 存在的理由与 `FrameCache::viewport` 不同：那条是 core 的**判据**，决定要不要
    /// 重排；这一份纯粹是「resize 之后到底发生了什么」的观测点。
    /// 两者的值应当一致 —— 不一致本身就是缺陷（见 `docs/task-responsive.md` B2）。
    traced_viewport: Option<(f32, f32)>,
    /// 图像纹理表：`Command::Image` 的句柄 → SRV 描述符位置。
    textures: TextureTable,
    /// 窗口。**最后析构**，见模块文档。
    window: Arc<Window>,
}

/// 未指定尺寸时的默认客户区尺寸。
///
/// winit 的默认值在本机是 640×480 —— 对按视口自适应的应用够用，对**固定画布**
/// 的应用（`width: 1400px` 那类）会裁掉一大块。要固定画布就用
/// [`HostWindow::new_lenient_sized`]。
pub const DEFAULT_WINDOW_SIZE: (u32, u32) = (1100, 700);

/// [`HostWindow::sync_swapchain`] 的结果：这一帧交换链与客户区的关系。
///
/// # 为什么要三个值而不是原来的 `bool`
///
/// 原来的 `bool` 只有一个含义「尺寸变了没」，而宿主实际需要分辨的是**三件事**，
/// 其中两件的处置完全相反：
///
/// | 值 | 含义 | 宿主该做什么 |
/// |---|---|---|
/// | [`Self::Same`] | 尺寸本来就没变 | **照常画**（鼠标在窗口里动也走这条） |
/// | [`Self::Synced`] | 刚 resize 完，链与客户区一致 | 照常画 |
/// | [`Self::Failed`] | resize 想做没做成，链还停在旧尺寸 | **本帧空过** |
///
/// `Failed` 被原来的 `bool` 和 `Same` 混为同一个 `false`，于是宿主照旧画 + present ——
/// DComp 把**旧尺寸**的内容拉进**新**窗口，那就是「窗口一直闪 / 内容被拉伸」。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwapState {    /// 尺寸没变，什么都没做。
    Same,
    /// resize 成功，链已与客户区一致。
    Synced,
    /// resize 失败但设备还活着（GPU 还攥着后缓冲那类）。
    ///
    /// 此时**不能**画：链还停在旧尺寸，画了就是把旧内容拉进新窗口。
    Failed,
}

/// 这一帧到底该不该画。**纯函数** —— 于是能在没有 GPU 的机器上门禁。
///
/// # 为什么必须是纯函数
///
/// 这个判断是「设备消失 / resize 失败」两条规则的唯一汇合点：
///
/// 1. 设备已死 ⇒ 不画（否则每帧三连错 + present 一个无效 visual = 闪）；
/// 2. resize 想做没做成 ⇒ 不画（否则把旧尺寸内容拉进新窗口 = 闪 / 拉伸）。
///
/// 两条都只在**真机拖动**时复现，而本机 D3D12 起不来（`tests/d3d12_draw.rs`
/// 头部记着证据，`tests/swapchain_draw.rs` 的真机用例因此全是 `#[ignore]`）。
/// 抽成 `(SwapState, bool) -> bool` 就能在没有设备的环境里断言它 ——
/// 否则这条修复等于没有门禁。
///
/// `Same` 与 `Synced` 都返回 `true`：前者是「尺寸没变」的**常态**（鼠标在窗口里
/// 动也走这条），误把它当成「别画」会让 UI 完全不响应，而那比原来的 bug
/// 更难发现。
pub fn should_draw_frame(state: SwapState, device_dead: bool) -> bool {
    if device_dead {
        return false;
    }
    !matches!(state, SwapState::Failed)
}

impl<V: View> HostWindow<V> {
    /// 建窗口与 GPU 资源，**找不到系统字体时不 panic**。
    ///
    /// 字体缺失不该让整个应用起不来：文本不显示，其余部分照常。这与「崩溃」之间的
    /// 差别很大 —— 后者让「这台机器没装字体」变成「这个程序不能跑」。
    pub fn new_lenient(event_loop: &ActiveEventLoop, view: V) -> Self {
        Self::new_lenient_sized(event_loop, view, DEFAULT_WINDOW_SIZE)
    }

    /// [`Self::new_lenient`] + **指定初始客户区尺寸**（**物理像素**）。
    ///
    /// # 为什么必须有这条
    ///
    /// `winit` 在不给尺寸时用系统默认值（本机是 **640×480**），而按固定画布
    /// 布局的应用（`Container.window { width: 1400px }` 那类）会在这个窗口里
    /// 被**裁掉一大块** —— 而且裁得毫无提示：布局是对的、绘制是对的，
    /// 只是有一多半在窗口外，看起来像「排版坏了」。
    ///
    /// # 为什么是**物理**像素而不是逻辑像素
    ///
    /// 核心里布局的单位是**像素**，而且宿主把交换链的像素尺寸原样当作视口传进去
    /// （见 [`Self::client_size`]）—— 即整条链是「像素对像素」的。若这里收逻辑
    /// 尺寸，高 DPI 下窗口会被放大（实测 125%：逻辑 1400×800 → 物理 1750×1000），
    /// 于是固定画布的内容只占窗口左上角，右/下留出空白：布局没错、绘制没错，
    /// 只是"窗口比界面大了一圈"。
    ///
    /// 症状与「布局算错」难以区分，所以画布尺寸必须由**应用**按像素给。
    /// `new_lenient` 保留默认尺寸，是为了不改变既有调用点（按视口自适应的应用
    /// 不受影响）。
    pub fn new_lenient_sized(event_loop: &ActiveEventLoop, view: V, size: (u32, u32)) -> Self {
        let attrs = Window::default_attributes()
            .with_title("ZephyraUI (D3D12)")
            .with_inner_size(winit::dpi::PhysicalSize::new(size.0, size.1));
        let window = Arc::new(
            event_loop
                .create_window(attrs)
                .expect("create_window（D3D12 后端）"),
        );
        let size = window.inner_size();
        // 宿主必须能**自己说出**坐标系：「点哪儿不好使」这类问题的头号嫌疑就是
        // 事件坐标（物理）与布局坐标（DIP）不在一个空间里，而画面永远是"对的"，
        // 只有命中会偏 —— 不把这三个数打出来就只能靠猜。
        eprintln!(
            "[zephyra-d3d12] 窗口: inner_size(物理) {:?} scale {}",
            window.inner_size(),
            window.scale_factor()
        );

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
        let chain = Swapchain::new(&gpu, (size.width, size.height), &desc, crate::swapchain::msaa_samples())
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
        // 二分开关必须**自己打出来**：跑完一轮二分，回头看日志时如果分不清
        // 「关掉 MSAA 之后不挂了」和「那次根本没设环境变量」，这轮二分就白做。
        // 这一行让每份日志自带生效配置。
        if crate::renderer::filters_disabled() {
            eprintln!("[zephyra-d3d12] 离屏滤镜：整条关掉（ZEPHYRA_NO_FILTERS）");
        }

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
            reported_device_dead: false,
            swap_state: SwapState::Same,
            resize_failures: 0,

            engine: std::cell::RefCell::new(engine),
            atlas_revision_uploaded: 0,
            backdrop_active: false,
            frames: 0,
            system_languages: zephyra_platform_win::current().system_languages(),
            last_frame_at: None,
            traced_viewport: None,
            textures: TextureTable::new(),
            window,
        }
    }

    /// Install a styler (called every frame after diff, before layout).
    ///
    /// Forwards to `FrameCache::set_styler`. Kept out of `new`'s signature on
    /// purpose: a stylesheet is a run-time thing (and dev hot-reloads it),
    /// while the window must exist at startup. Folding it into the constructor
    /// would make "start the window" depend on "the stylesheet parsed" -- one
    /// wrong property name and the app will not launch.
    pub fn set_styler(&mut self, styler: Box<dyn zephyra_core::styler::Styler>) {
        self.cache.set_styler(styler);
    }

    /// 切换主题（Light / Dark）。
    ///
    /// # 为什么必须显式作废缓存
    ///
    /// 资源键是**读取时**解析的（`resolve_resource` 不写回属性表），所以换主题
    /// 不需要动树——但 `FrameCache` 可能正处在稳态（`Unchanged`），那一帧会整帧
    /// 跳过，新主题要等下一次别的事件才上屏。这里立刻 `invalidate`，让下一帧
    /// 必然重画重解析。与 `set_styler` 同一个理由。
    ///
    /// `Theme` 是 `Copy` + 这里 `Box::leak` 成 `&'static`：与构造时同一手法
    /// （进程内切主题次数有界，泄漏的是两份 200 字节的结构）。
    pub fn set_theme(&mut self, variant: zephyra_core::theme::ThemeVariant) {
        let theme: &'static Theme = Box::leak(Box::new(Theme::builtin(variant)));
        self.theme = theme;
        self.cache.invalidate();
    }

    /// 当前主题变体（应用据此判断「想切的」与「在用的」是否一致）。
    pub fn theme_variant(&self) -> zephyra_core::theme::ThemeVariant {
        self.theme.variant
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

    /// 客户区**有效像素**尺寸 = 物理 / scale。
    ///
    /// # 为什么单独一个方法，而不是把 `client_size` 改掉
    ///
    /// 两者用途不同，混在一起会一次改两件事：
    /// - `client_size()`（物理）是**布局视口**，改它会让全部视觉尺寸一起变
    ///   （1.25 缩放下整体小 25%）—— 那是独立的一轮，见
    ///   `docs/responsive-and-touch.md` §5 的记账；
    /// - 本方法（有效）只服务**断点判定**。官方口径写得很清楚：
    ///   `AdaptiveTrigger.MinWindowWidth` = "the minimum window width
    ///   (**in effective pixels**)"。本机 scale 1.25 下，1400 物理 = 1120 有效，
    ///   而 800 物理 = 640 有效，正好压在「小/中」的边界上 ——
    ///   **按物理判会整体错一档**。
    pub fn effective_size(&self) -> (f32, f32) {
        let (w, h) = self.client_size();
        let scale = self.window.scale_factor();
        // scale 为 0 / 非正 / NaN 时退回 1.0：除零会得到 inf，而 inf 会让
        // 每个断点判定都走向同一支，那种"静默退化"比"明显错了"更难发现。
        let scale = if scale.is_finite() && scale > 0.0 {
            scale as f32
        } else {
            1.0
        };
        (w as f32 / scale, h as f32 / scale)
    }

    /// 当前 hover 的节点 key。
    pub fn hovered(&self) -> Option<&ElementKey> {
        self.router.hovered()
    }

    /// 按应用请求设置窗口外观（暗色模式 / 圆角 / 标题栏与边框颜色）。
    ///
    /// # 为什么要宿主转发一层，而不是让应用直接调 `zephyra-platform-win`
    ///
    /// 三个理由，都不是"省一行代码"：
    ///
    /// 1. **句柄转换是本 crate 的事**：应用拿到的是 `&Window`，`HWND` 只有
    ///    拿到 `window.hwnd()` 的那一层才该碰；
    /// 2. **能力降级要在这里收敛**：平台不支持时不该让应用拿到一个"我以为
    ///    设上了"的结果 —— [`ApplyOutcome`] 逐项如实返回；
    /// 3. **可移植性**：应用调的是宿主方法，Linux 上宿主内部换成别的实现，
    ///    应用代码一个字不改。
    ///
    /// 返回值即 [`ApplyOutcome`]，逐项说明"哪些真的生效了"。
    ///
    /// 取 `&mut self`：它会记下材质是否生效（影响每帧的 clear 色）。
    pub fn apply_window_chrome(
        &mut self,
        chrome: zephyra_platform::WindowChrome,
    ) -> zephyra_platform::ApplyOutcome {
        // 复用本文件已有的句柄提取（`raw_window_handle` 路径），不重复造轮子。
        // 拿不到（理论上只在非 Windows 上）⇒ 0 号句柄 ⇒ 平台层静默跳过。
        let raw = hwnd_of(self.window.as_ref()).unwrap_or(0) as usize;
        let outcome =
            zephyra_platform_win::current()
                .apply_window_chrome(zephyra_platform::WindowHandle::from_raw(raw), chrome);
        // 记下「材质是否真的生效」：它决定 clear 用不用透明色（见 `render_pass`）。
        // 请求 Mica 但平台不支持 ⇒ 仍是实色 clear，不能当成已生效。
        self.backdrop_active = outcome.backdrop && chrome.backdrop != zephyra_platform::BackdropKind::None;
        if trace_enabled() {
            // 「界面全对、就是标题栏没变暗」这类问题只有这一行能查。
            eprintln!("[trace] 窗口外观 {chrome:?} -> {outcome}");
        }
        outcome
    }

    /// 拉取调试层消息（`ZEPHYRA_D3D12_DEBUG=1` 时才有内容）。
    pub fn drain_debug(&self) {
        self.gpu.drain_debug();
    }

    /// 一帧什么都没做时返回的东西。
    ///
    /// `redrawn: false` 是关键：`about_to_wait` 见到它就**不 present** —— 而设备
    /// 坏着或链还没 resize 成功时，present 正是最该省掉的那一步。
    fn empty_frame(&self) -> Frame {
        Frame {
            diff: zephyra_core::diff::DiffStats::default(),
            render: zephyra_fx::quad::RenderReport::default(),
            hovered: None,
            damage: zephyra_core::frame::Damage::Full,
            redrawn: false,
        }
    }

    /// 这一帧到底该不该画。**纯函数** —— 于是能在没有 GPU 的机器上门禁。
    ///
    /// # 为什么必须是纯函数
    /// 跑一帧：布局 → `DisplayList` → 顶点展开 → 记录 → 提交。
    ///
    /// **不**包含 `present` —— 那由 [`Self::present`] 单独做，因为 resize 必须在
    /// 绘制之前处理（见 `sync_swapchain`），而呈现必须在绘制之后。
    pub fn frame(&mut self) -> Frame {
        // ⚠ resize 必须在**布局与绘制之前**，见 [`Self::sync_swapchain`] 的说明。
        // 顺序反了会留下一个「从未被写过的新后缓冲」，而且它可能永远等不到下一帧。
        //
        // ⚠ 设备已死时**整个函数直接返回**：不进布局、不画、不 present。
        //
        // 早先是照样往下走，于是每帧产出三连错（resize 重试 / 后缓冲下标越界 /
        // present 失败），而 DComp 视角下是「一个 Present 不出来的 visual」——
        // 症状就是「窗口一直闪 + 错误刷屏」。
        self.swap_state = self.sync_swapchain();
        let dead = self.chain.device_dead();
        if !should_draw_frame(self.swap_state, dead) {
            // ⚠ 只有**设备真的死了**才报「设备不可用」。resize 暂时失败时设备还活着，
            // 那条路径唯一该响的是 `sync_swapchain` 里的 trace 行（它每秒可能来几十
            // 次，不该按错误报）。
            if dead {
                self.report_device_dead_once();
            }
            return self.empty_frame();
        }
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
            // 视口一变就打一行，不按帧号节流：resize 类缺陷的判据全在这一行里。
            //
            // `redrawn == false` 且视口刚变过 ⇒ **core 认为不用重画**（问题在
            // 判据，不在绘制）；`redrawn == true` 而屏幕没变 ⇒ **画了但没显示**
            // （问题在交换链 / DComp）。这两类症状看起来一模一样（都是"窗口变大了
            // 但内容没跟着变"），而排查方向完全相反 —— 所以必须分开打。
            let vp = {
                let s = self.client_size();
                (s.0 as f32, s.1 as f32)
            };
            if self.traced_viewport != Some(vp) {
                if let Some(old) = self.traced_viewport {
                    eprintln!(
                        "[trace] 第 {} 帧视口 {old:?} → {vp:?}：redrawn={redrawn} damage={damage:?}",
                        self.frames,
                    );
                }
                self.traced_viewport = Some(vp);
            }
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
        // # 「只重画脏区」的前提：脏区之外的上一帧画面**还在渲染目标里**
        //
        // flip model（`CreateSwapChainForComposition` 恒为 FLIP_SEQUENTIAL）的
        // 后缓冲在 `Present` 之后**内容未定义** —— 它是会被丢弃的临时目标，
        // 不是"上一帧那张图"。
        //
        // 分两种情况：
        //
        // - **MSAA 开**：渲染目标是**自有的**多重采样纹理（`chain.msaa_rtv()`），
        //   逐帧存活、不被系统丢弃；帧末整体 resolve 进后缓冲。所以脏区之外的
        //   上一帧内容确实还在 —— 只清脏区、只画脏区是对的。
        // - **MSAA 关**：渲染目标**就是**那个会被丢弃的后缓冲 ⇒ 脏区之外是
        //   未定义内容。此时按脏区绘制等于把未定义像素推上屏（实测：整屏变底色，
        //   只有脏区那一小块是对的）。所以这里**强制整帧**。
        //
        // 代价是诚实的：MSAA 关时每个交互帧都整帧重绘。取舍写在明处 ——
        // 画面错与画面慢之间，这个仓选画面。
        let damage = if self.chain.samples() > 1 {
            damage
        } else {
            Damage::Full
        };
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
                .record_upload_atlas(self.gpu.list(), self.engine.borrow().atlas().pixels())
            {
                eprintln!("[zephyra-d3d12] 图集上传失败（{e}），本帧文字可能不显示");
            }
            self.atlas_revision_uploaded = revision_now;
        }

        // 后缓冲在每帧开头都是 `PRESENT`。不转就没有渲染目标 —— 且**不报任何错**。
        //
        // MSAA 开启时它**仍然要转**：它是 resolve 的目标，而 D3D12 不为 resolve
        // 自动转换状态。所以这一段与 MSAA 无关，始终执行。
        if let Err(e) = self.chain.barrier_to_render_target(self.gpu.list()) {
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
        // 清屏色：默认是主题底色（不透明）。
        //
        // **启用系统背景材质时改成全透明**：材质（Mica/Acrylic）是 DWM 画在
        // 窗口**背后**的，只有我们把后缓冲清成 alpha=0、且应用根背景半透明，
        // 材质才透得出来。清成不透明底色 = 材质被自己的 clear 挡住，等于没开。
        //
        // 判据是「材质是否真的生效」（`ApplyOutcome.backdrop`），而不是
        // 「应用是否请求了」——请求了但平台不支持时，仍��不透明色。
        let clear = if self.backdrop_active {
            self.theme.surface.with_alpha(0.0)
        } else {
            self.theme.surface
        };
        // 可分离 / 非可分离混合模式要读 `dst`，必须把后缓冲当**资源**传进去。
        // 取失败时传 `None`：那 14 个模式会退回 `SrcOver`，画面仍然正确只是少了效果。
        let rt = self.chain.back_buffer().ok();
        let report = match self.renderer.record(RenderCtx {
            device: &self.gpu.device,
            list: self.gpu.list(),
            rtv,
            display_list: self.cache.list(),
            damage,
            size: self.client_size(),
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
            if let Err(e) = self.chain.resolve_msaa(self.gpu.list()) {
                eprintln!("[zephyra-d3d12] MSAA 解析失败: {e}");
            }
        }
        if let Err(e) = self.chain.barrier_to_present(self.gpu.list()) {
            eprintln!("[zephyra-d3d12] 后缓冲转回 PRESENT 失败: {e}");
        }
        if let Err(e) = self.gpu.submit() {
            eprintln!("[zephyra-d3d12] 提交失败: {e}");
        }
        report
    }

    /// 让拖动窗口边时**逐帧跟随**（而不是拖完才更新一次）。
    ///
    /// # 问题出在哪（不是本 crate 的 bug）
    ///
    /// 拖边时 Windows 在 `DefWindowProc(WM_NCLBUTTONDOWN)` 里跑一个**模态循环**。
    /// 该循环**会**把一串 `WM_SIZE` 派发到窗口过程，但 winit 的主循环此刻被
    /// 阻塞在 `DefWindowProc` 里面 —— winit 自己的 `WndProc` 收到 `WM_SIZE` 后只做
    /// `userdata.send_event(Resized(..))`，**入队**，而主循环取不出来
    /// （winit-0.30.13 `platform_impl/windows/event_loop.rs:1397-1418`）。
    /// 于是 [`Self::about_to_wait`] 那种「脏了就画」直到拖动结束才跑第一次。
    ///
    /// # 修法（照 XAML 的形状，不照直觉的形状）
    ///
    /// 直觉方案是「`WM_ENTERSIZEMOVE` 起个 16 ms 定时器，`WM_TIMER` 里重绘」。
    /// **不这么做**，两条理由都来自 `E:\microsoft-ui-xaml`：
    ///
    /// - XAML 侧**零** resize 定时器，所以那条路没有官方先例；
    /// - XAML 对「渲染期间意外泵消息」是 fail-fast 的
    ///   （`dxaml/xcp/core/dll/xcpcore.cpp:6338-6345`，注释原文就是防止渲染循环里
    ///   不小心泵消息导致又一次 paint 进来）。定时器方案恰恰是在拖动期间反复进
    ///   渲染，等于主动制造它最怕的重入。
    ///
    /// XAML 真正**同步**处理尺寸的地方是窗口过程里的链条：`WM_SIZE` →
    /// `CWindowChrome::UpdateContainerSize` → `UpdateBridgeWindowSizePosition` →
    /// `::SetWindowPos`（`CWindowChrome.cpp:338-343, 388`）。
    /// **本方法就是那个形状**：子类化窗口过程，在 `WM_SIZE` 里同步把这一帧做完。
    ///
    /// # 为什么不需要另起线程
    ///
    /// D3D12 命令列表与交换链都是**创建线程所有**的，所以渲染必须在 UI 线程。
    /// 而模态循环期间 UI 线程**正是空闲的**（它只是卡在 `DefWindowProc` 里），
    /// 换句话说这恰好是唯一能安全画的时候。
    ///
    /// # SAFETY（这里有一处 `unsafe`，不藏）
    ///
    /// 回调必须是 `'static`，拿不到 `&mut Self`。用裸指针的前提有三条：
    ///
    /// 1. **地址稳定**。宿主必须在 `Box`/栈上原地存活，且装钩子之后**不许**再挪。
    ///    调用方（`Bootstrap::resumed` 里的 `Option<HostWindow>` 内联存储）满足。
    /// 2. **不并发**。回调只在模态循环派发 `WM_SIZE` 时触发，而那时 winit 的主循环
    ///    被阻塞着，不存在活的 `&mut HostWindow`。平台层的 `IN_HOOK` 再挡一层重入。
    /// 3. **不重入**。回调里不泵消息、不调 `SetWindowPos`、不调 `DefWindowProc`。
    ///
    /// 这三条写在 `zephyra-platform-win/src/live_resize.rs` 的模块文档里也有，
    /// 两处必须一致 —— 改一处就要改另一处。
    pub fn enable_live_resize(&mut self) -> Result<(), zephyra_platform_win::LiveResizeError>
    where
        V: 'static,
    {
        let Some(hwnd) = hwnd_of(self.window.as_ref()) else {
            // 非 Win32 平台没有 HWND 可子类化。不算错：这个后端本来就是 Windows 专属。
            return Ok(());
        };
        let me = HostPtr(self as *mut Self);
        zephyra_platform_win::set_render_hook(Arc::new(move || {
            // ⚠ 这一行不是废代码。Rust 2021 的 `move` 闭包**按字段**捕获：
            // 下面要用 `me.0`，编译器就会只把那个裸指针字段搬进闭包 ——
            // 于是 `HostPtr` 上那两条 `unsafe impl Send/Sync` **根本没被用上**，
            // 整个包装白写，编译直接报 E0277。
            // 把整个 `me` 重新绑一次，捕获的就变成 `me` 本身。
            let me = &me;
            // SAFETY：见上面「SAFETY」三条。这三段是同一个不变量的三个侧面。
            let host = unsafe { &mut *me.0 };
            host.dirty = false;
            let f = host.frame();
            if f.redrawn {
                host.present();
            }
            // 返回 `true` = 这一帧确实画了。没画也无所谓 —— 调用方只看
            // 「有没有跳过」，那由 `IN_HOOK` 决定，不是这里。
            true
        }));
        zephyra_platform_win::install_live_resize(hwnd)
    }

    /// 呈现一次。
    ///
    /// **不含 resize** —— 见 `sync_swapchain`：resize 必须发生在**绘制之前**，
    /// 否则新建的后缓冲要到下一次 `frame()` 才有内容，而那一次可能永远不会来。
    pub fn present(&mut self) {
        // 设备已死时**一个 API 都不调**。每次拖动/恢复都重试 present，
        // 在 DComp 看来就是反复推一个无效 visual ⇒ 闪。
        if self.chain.device_dead() {
            self.report_device_dead_once();
            return;
        }
        // 新交换链第一次 present 之前把 GPU 排空。
        //
        // 官方依据见 `Swapchain::take_first_present_flush`（Chromium
        // `ui/gl/swap_chain_presenter.cc:1155`）：不这么做，DComp 会在新链
        // 第 1~2 次 present 之间显示黑帧。拖动期间我们每次 `recreate()` 都会
        // 走到这儿 —— 也就是说这正是「拖动时闪」的另一半成因。
        if self.chain.take_first_present_flush() {
            // 等不到也不阻断 present：这是消闪，不是正确性前提。真在这儿死等，
            // 症状会变成「UI 静止不变」—— 比黑一闪难查得多。
            let _ = self.gpu.wait_gpu();
        }
        if let Err(e) = self.chain.present(1) {
            eprintln!("[zephyra-d3d12] present 失败: {e}");
            self.drain_debug();
        }
    }

    /// 把「设备已不可用」只打一次，并给出官方要求的后续动作。
    ///
    /// # 为什么必须只打一次
    ///
    /// 早先是每帧三条错误（resize / 后缓冲越界 / present），拖动期间一秒几十条，
    /// 把真正有用的信息全埋掉 —— 用户看到的就是一片刷屏。而原因码要
    /// `GetDeviceRemovedReason()` 才拿得到，那才是「该采取什么措施」的答案。
    fn report_device_dead_once(&mut self) {
        if self.reported_device_dead {
            return;
        }
        self.reported_device_dead = true;
        let code = self.chain.device_dead_reason();
        let (name, advice) = match code {
            // 官方四个常量，按「该怎么办」分组而不是按名字排。
            Some(0x887A_0007) => (
                "DXGI_ERROR_DEVICE_HUNG",
                "GPU 挂了（TDR）。必须重建 D3D12 设备与全部依赖它的资源；\
                 本宿主现在停下绘制，等下一次设备重建的入口",
            ),
            Some(0x887A_0006) => (
                "DXGI_ERROR_DEVICE_RESET",
                "设备正在重建，可恢复 —— 但本宿主目前不自动重试，\
                 恢复后需要重建设备再继续",
            ),
            Some(0x887A_0005) => (
                "DXGI_ERROR_DEVICE_REMOVED",
                "设备被移除（通常是驱动更新/显卡复位）。GetDeviceRemovedReason \
                 给出了通用码，真实原因见上面两者之一",
            ),
            // 未知原因码：不能沉默 —— 静默的黑屏比响一声的黑屏难查得多。
            _other => ("未知", "GetDeviceRemovedReason 的返回值不在预期集合里"),
        };
        if let Some(c) = code {
            eprintln!(
                "[zephyra-d3d12] 设备不可用：{name} (0x{c:08X})\n\
                 [zephyra-d3d12] 后续动作：{advice}\n\
                 [zephyra-d3d12] 本帧起停止绘制与呈现（不再重试，也不再打印）"
            );
        } else {
            // 走到这里说明 `device_dead` 是真的但原因码丢了 —— 不该发生，
            // 但也不能沉默：静默的黑屏比响一声的黑屏难查得多。
            eprintln!("[zephyra-d3d12] 设备被标记为不可用，但拿不到原因码");
        }
        self.drain_debug();
        // ⚠ 上面那几行 `[d3d12 debug] ID=... severity=...: <描述>` 就是**唯一的
        // 一手证据**，请把它们连同本条日志一起贴回来 —— 它们会直接点名是哪个 API
        // 在哪个资源上出的问题（典型：`ID=527` = 屏障的 Before 状态与实际不符）。
        //
        // 如果**一条都没有**，说明调试层没挂上（`ZEPHYRA_D3D12_DEBUG=0` 把它关了，
        // 或者这台机器的 `d3d12.dll` 不支持事后开启）。那就先去掉这个环境变量再跑。
        //
        // ⚠ 为什么这里要专门提一句：`begin_frame` 之前每帧都会 drain 一次并
        // `ClearStoredMessages`，所以**出问题那一帧**的消息是在下一帧才打出来的。
        // 早先设备一判死就整体返回，把那一帧的消息连同上下文一起丢了 ——
        // 于是每次都只能拿到「GPU 挂了」这个结论，拿不到原因。
    }

    /// 把交换链对齐到窗口尺寸。**返回尺寸是否变了**（变了就要重排）。
    ///
    /// # 为什么在**每帧开头**做，而不是画完再 resize
    ///
    /// 「先画、再 resize」有两个后果：
    ///
    /// 1. 重建后的后缓冲是**从未被写过的** —— 而画它的那一帧已经过去了。若此后没有
    ///    新的脏（窗口不缩放不移动），就再没有 `frame()`，于是**永久**停在那一块
    ///    未初始化的缓冲上：屏幕上是一块纯白（或纯黑）矩形，外面是窗口背景。
    /// 2. 更要紧：`resize` 要**等 GPU 跑完上一帧**（`Swapchain::resize` 里的
    ///    `gpu.wait_gpu()`，那是 `ResizeBuffers` 的硬要求）。放在画完之后就等于
    ///    刚提交完就立刻同步等栅栏，每帧多一次 GPU 往返。
    ///
    /// 先 resize 再画，那一帧就直接画进新缓冲，没有中间态。
    ///
    /// # 重建后必须重新挂 DComp 视觉树
    ///
    /// visual 上挂的是**旧的** swapchain 对象。新的那个没人引用，合成器不会去取它 ——
    /// 于是「提交了、也没报错，就是不显示」。这与「没提交」在屏幕上完全一样。
    /// 把交换链对齐到窗口尺寸。
    ///
    /// # 返回值的三个含义（宿主据此决定这一帧还画不画）
    ///
    /// - [`SwapState::Synced`]：链已与客户区一致，正常画。
    /// - [`SwapState::Same`]：尺寸本来就没变（鼠标在窗口里动也走这条）。
    /// - [`SwapState::Failed`]：**resize 想做没做成，而设备还活着**。
    ///
    /// # 为什么 `Failed` 必须让宿主跳过本帧
    ///
    /// resize 失败时链还停在**旧尺寸**。此时若照旧"画 + present"，DComp 会把这块
    /// 旧内容拉进**新**窗口 —— 那就是「窗口一直闪 / 内容被拉伸」的直接来源。
    ///
    /// 跳过本帧而不是硬 present，另一头是「不 present 就不闪」：flip model 下
    /// DComp 会继续显示上一次成功 present 的内容，等下一次成功 resize 再更新。
    fn sync_swapchain(&mut self) -> SwapState {
        let (w, h) = self.client_size();

        // ⚠ 零尺寸 = 窗口被最小化。此时**什么都不做、本帧跳过**。
        //
        // 官方对应做法是把「不可见」当成**不画**的理由：Chromium 的最小化一路走到
        // `DisplayScheduler::ShouldDraw()` 返回 false
        // （`components/viz/service/display/display_scheduler.cc`）。
        // 这里复用 `Failed` 这条「本帧空过」的路径，不新增状态。
        if w == 0 || h == 0 {
            return SwapState::Failed;
        }

        // 内容尺寸没变 ⇒ 链也不用动，直接画。
        if (w, h) == self.chain.content_size() {
            return SwapState::Same;
        }

        // 链**不跟着窗口变**：它只保证「≥ 客户区」。绝大多数尺寸变化在这里什么都不做 ——
        // 只是把视口改成新的客户区，本帧照常按新尺寸排版并绘制，于是**拖动时逐帧跟随**。
        //
        // 只有窗口大到装不下时才重建整条链，且一次给 25% 余量。详见
        // [`Swapchain::resize`] 的文档（含为什么本机不能用 `ResizeBuffers`）。
        let before = self.chain.size();
        if let Err(e) = self.chain.resize(&self.gpu, (w, h)) {
            self.resize_failures = self.resize_failures.saturating_add(1);
            eprintln!("[zephyra-d3d12] 调整交换链容量到 {w}×{h} 失败（第 {} 次）：{e}", self.resize_failures);
            return SwapState::Failed;
        }
        if self.chain.was_recreated() {
            // 自述型日志：容量从多少涨到多少。没有它就分不清「窗口还在拖」与
            // 「同一次调整反复重建」。
            eprintln!(
                "[zephyra-d3d12] 后缓冲容量 {before:?} → {:?}（客户区 {w}×{h}），重建交换链",
                self.chain.size()
            );
            if self.rebind_visual() {
                // 必须**在 rebind 成功之后**才放旧链 —— rebind 是
                // SetContent(新链) + Commit()；在那之前 DComp 的 visual 上挂的还是
                // 旧链，早一步释放就是把 use-after-free 送给 DComp。
                //
                // 官方纪律见 Chromium ui/gl/dc_layer_tree.cc:626（只在内容变了时
                // SetContent，拆除时 SetContent(nullptr)，然后 Commit）。
                drop(self.chain.take_superseded());
            } else {
                eprintln!("[zephyra-d3d12] 重新挂载未生效，旧交换链继续保留（DComp 仍在用它）");
            }
        }
        if trace_enabled() {
            eprintln!(
                "[trace] 内容尺寸 → {w}×{h}；后缓冲容量 {:?}；重建 {}",
                self.chain.size(),
                self.chain.was_recreated(),
            );
        }
        SwapState::Synced
    }
    /// 把（可能已被重建的）交换链重新挂到 visual 上并提交。
    ///
    /// # 返回 `true` = DComp 已经在用新链了，调用方**可以**释放旧链
    ///
    /// 这个返回值不是「成没成功」这么简单，它是**释放旧链的许可**。
    /// 返回 `false` 时 visual 上挂的还是旧链 —— 那时把它释放就是把
    /// use-after-free 送给 DComp。调用方必须留住旧链（见
    /// [`Swapchain::take_superseded`]）。
    ///
    /// 两步都必须成功才算：`SetContent` 只是**记下**新内容，`Commit()` 才让它
    /// 生效（官方：DComp 的所有改动都要 Commit 才落地，见
    /// `ui/gl/dc_layer_tree.cc:626`）。Commit 失败 ⇒ 视觉树仍是旧内容。
    fn rebind_visual(&mut self) -> bool {
        let (Some(comp), Some(vis)) = (self.compositor.as_ref(), self.visual.as_ref()) else {
            // 没有合成器/visual（窗口已销毁、或平台层说不支持）—— 无可挂，
            // 但也**没有别人在用旧链**，所以旧链可以安全释放。
            return true;
        };
        if let Err(e) = comp.set_content_swapchain(vis, self.chain.as_swap_chain1()) {
            eprintln!("[zephyra-d3d12] 重新挂 visual 失败: {e}");
            return false;
        }

        if let Err(e) = comp.commit() {
            // ⚠ 这一步失败也算「DComp 还没切过去」—— 见方法文档。
            eprintln!("[zephyra-d3d12] DComp commit 失败: {e}");
            return false;
        }
        true
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
        // 视口、缩放比与帧间隔一起推给视图：断点判定要**有效像素**（除 scale），
        // 列数计算要**布局宽**（本仓即物理像素），动画时钟要**真实 dt**。
        let size = self.client_size();
        let scale = self.window.scale_factor();
        // dt 实测而不是按刷新率拍：官方动画时长全是毫秒（83/167/250/333），
        // 只有真实时间下才成立。掉一帧动画会变慢 —— 但那比"每帧固定推进"好：
        // 后者在任何非标刷新率上都不准。
        let now = std::time::Instant::now();
        let dt_ms = self
            .last_frame_at
            .map(|t| now.duration_since(t).as_secs_f32() * 1000.0)
            .unwrap_or(0.0);
        self.last_frame_at = Some(now);
        let state = InputState {
            hovered: self.router.hovered().cloned(),
            pressed: self.router.pressed().cloned(),
            pointers: self.router.pointer_count(),
            viewport: (size.0 as f32, size.1 as f32),
            scale: scale as f32,
            dt_ms,
            // 系统语言偏好：每帧问一次很贵（一次 Win32 调用），而它是**启动时的
            // 外部事实**，之后不会变（用户切换系统语言要重启进程才该重新读）。
            // 所以只问一次并缓存 —— 与版本号、系统主题同一手法。
            languages: self.system_languages.clone(),
        };
        self.view.on_input(&state);
    }

    /// 处理指针移动。返回是否产生了路由事件。
    pub fn on_cursor_moved(&mut self, x: f32, y: f32) -> bool {
        self.pointer_pos = (x, y);
        if trace_enabled() {
            eprintln!("[trace] CursorMoved ({x:.1}, {y:.1})");
        }
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
        if trace_enabled() {
            // 「点哪儿不好使」的唯一一手证据：坐标 → 处理目标。
            // 画面永远是对的，命中偏了也只有这一行能看出来。
            eprintln!(
                "[trace] 指针 ({:.0}, {:.0}) → {:?}",
                self.pointer_pos.0, self.pointer_pos.1, self.router.hovered()
            );
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

/// 装进 `'static` 回调里的宿主指针。
///
/// # 为什么需要这个包装
///
/// 裸指针既不是 `Send` 也不是 `Sync`，而 [`HostWindow::enable_live_resize`] 的
/// 回调类型是 `Arc<dyn Fn() -> bool + Send + Sync>`（窗口过程可能被任意线程的
/// 消息派发调用，哪怕本进程只有 UI 线程）。所以必须显式声明一次。
///
/// # SAFETY
///
/// 真正的不变量在 [`HostWindow::enable_live_resize`] 的文档里写了三条
/// （地址稳定 / 不并发 / 不重入）。这里只是把那三条**转成类型系统要求的断言**。
///
/// 这里**不能**用 `Arc<Mutex<..>>` 之类替代：那要求把宿主的可变状态整体搬进
/// `Mutex`，而 `frame()` 要连着 `&mut self` 跑排版 + 录制 + 提交，锁的范围会被
/// 撑到整个帧 —— 而帧里会 `eprintln!`、会访问 `RefCell`、会回调 `View::view`，
/// 加一把可能被别人拿到的锁只会换来死锁而不是安全。
struct HostPtr<V: View>(*mut HostWindow<V>);

// SAFETY：见上。三条不变量由 `enable_live_resize` 的文档与平台层模块文档共同保证；
// 那两处任何一条被改，这条都要重新论证。
unsafe impl<V: View> Send for HostPtr<V> {}
// SAFETY：同上（回调可被并发调用的面由平台层的 `IN_HOOK` 原子位挡住）。
unsafe impl<V: View> Sync for HostPtr<V> {}

/// 从 winit 窗口取裸 HWND。失败返回 `None`（非 Win32 平台）。
fn hwnd_of<W: raw_window_handle::HasWindowHandle>(window: &W) -> Option<isize> {
    use raw_window_handle::RawWindowHandle;
    let handle = window.window_handle().ok()?;
    match RawWindowHandle::from(handle) {
        RawWindowHandle::Win32(h) => Some(h.hwnd.get()),
        _ => None,
    }
}

impl<V: View + 'static> winit::application::ApplicationHandler for HostWindow<V> {
    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {
        // 窗口在 `new_lenient` 里就建好了；这里补第一帧。
        self.sync_input();
        self.frame();
        self.present();
        // 拖边时逐帧跟随。**必须放在第一帧之后**：钩子里会 `frame()`，
        // 而那时 `cache` 还是空的 —— 空树上跑一趟布局既是白费，也更容易
        // 把「首帧就该建好的缓存」留到一个不对劲的时刻再建。
        if let Err(e) = self.enable_live_resize() {
            eprintln!("[zephyra-d3d12] 装不上实时 resize 子类，拖边时会退回「拖完才刷新」: {e}");
        }
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

#![deny(rustdoc::broken_intra_doc_links)]
//! # zephyra-winit
//!
//! ZephyraUI 的 winit + D3D11 宿主适配层，位于系统适配层 `zephyra/zephyra-base/`
//! （见该目录的 README）。
//!
//! ## 职责
//!
//! 1. 用 winit 建窗口、收事件，把指针/触摸/键盘/滚轮翻译成核心的输入类型；
//! 2. 用 D3D11 建设备与交换链，把 `zephyra_core::paint::DisplayList` 回放成一帧。
//!
//! ## 不做什么
//!
//! - 不做布局、不做命中、不做文本整形之外的任何决策 —— 那些在核心里；
//! - 不缓存字形图集的生存期语义（那是纹理资源的管理）。
//!
//! 判据很硬：换一个 base 实现（DXGI 传统路径 ↔ DirectComposition，或
//! D3D11 ↔ 别的后端），同一棵元素树必须产出逐字节相同的 `DisplayList`。
//! 凡是在替换本 crate 后会改变 `DisplayList` 内容的逻辑，都放错了层。
//!
//! ## 模块划分（每个只有一件事）
//!
//! | 模块 | 职责 | 依赖 GPU？ |
//! |---|---|---|
//! | [`shader`] | HLSL 源码 + `D3DCompile` FFI | 是（要设备） |
//! | [`renderer`] | 管线状态、顶点缓冲、每帧 draw | 是 |
//! | [`surface`] | 交换链 + 可选 DComp 树 + present | 是 |
//! | （`zephyra-fx`） | `DisplayList` → 顶点流、字形图集、纹理表、CPU 光栅器 | 否 |
//!
//! `DisplayList` → 顶点流那一整段（原先是本 crate 的 `quad`/`texture`/`text`/
//! `cpu_raster`/`cpu_only`）**已经搬到 `zephyra-fx`**。它们一个 GPU 类型都不碰，
//! 而放在这里会让 D3D12 后端既没有字体也没有 CPU 诊断能力。
//!
//! ## 当前状态
//!
//! [`Renderer`] 能把 `DisplayList` 回放成像素：圆角矩形与描边走片元 SDF，
//! 文本与图像走纹理采样，`Command::Path`（矢量图标）走 CPU 三角化后的三角形流、
//! 抗锯齿靠 MSAA。

#![cfg(windows)]
#![allow(unsafe_code)]

use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};
use zephyra_core::context::Context;
use zephyra_core::core::Color;
use zephyra_core::diff::DiffStats;
use zephyra_core::element::Element;
use zephyra_core::frame::{Damage, FrameCache, FrameOutcome};
use zephyra_core::id::NodeTag;
use zephyra_core::input::{PointerEvent, PointerPhase, PointerSource, RoutedEvents};
use zephyra_core::property::PropertyValue;
use zephyra_core::props;
use zephyra_core::theme::Theme;

pub mod renderer;
pub mod shader;
pub mod surface;
pub mod texture;

// ── 来自 `zephyra-fx` 的再导出 ─────────────────────────────────────────────
//
// 这些类型**没有后端依赖**，却曾经住在这个 D3D11 后端 crate 里，于是 DX12 后端
// 一个也用不到。下面按原路径再导出，是为了让既有调用点
// （`zephyra_winit::cpu_only::CpuHost`、`zephyra_winit::quad::kind::GLYPH`）
// 不用改 —— 搬 crate 是结构调整，不该顺手改调用点的名字。
pub use zephyra_fx::layout_tree;
pub use zephyra_fx::{cpu_only, cpu_raster, pixels, quad, text};
pub use zephyra_fx::{
    from_font_file, from_system_font, Frame, InputState, PixelRead, RenderReport, TextEngine,
    TextureNames, View, FONT_OVERRIDE_ENV,
};
pub use zephyra_fx::{srgb_bytes, write_bmp};

pub use renderer::{RenderCtx, Renderer};
pub use surface::{dcomp_enabled, Gpu, Surface, SurfaceError};
pub use texture::TextureTable;

pub use winit;
pub use zephyra_core;
pub use zephyra_fx;

/// 标量属性的本地定义（宿主自定义控件用）。
///
/// 定义搬到了 [`zephyra_fx::Z_INDEX`] —— 本 crate 与 `zephyra-winit-d3d12` 是两个
/// 独立 crate，而属性键靠**名字**派生 id，所以两份定义**恰好**能互换；一旦只改一处，
/// 另一处就静默变成另一个键，症状是「z-index 不生效」且没有任何编译或运行错误。
pub use zephyra_fx::Z_INDEX;

/// 窗口 + GPU + 保留树的组合体。
///
/// 保留树（`Node`）跨帧存活 —— 这正是保留式相对即时式的收益：状态与纹理
/// 得以复用，只有真正变化的子树被重排/重绘。
///
/// # 字段顺序不是随意的
///
/// `surface` 持有 DComp 树（含 target 的 COM 引用），必须比窗口活得久
/// （DComp 硬约束 #1）。Rust 按**声明顺序**释放字段，所以 `surface` 必须声明在
/// `window` **之后** —— 否则窗口先死、DComp 树后死，绑定被拆除时窗口已不可见，
/// 现象是关窗瞬间闪一下黑。
pub struct HostWindow<V: View> {
    /// 上一次 `sync_input` 的时刻 —— 动画时钟的唯一来源（`InputState::dt_ms`）。
    ///
    /// 放在最前面与释放顺序无关（`Instant` 无 Drop 语义），只为让"帧间隔从哪来"
    /// 一眼可寻。`None` = 还没有上一帧，于是 dt 为 0 —— 首帧没有
    /// "距上一帧多久"可言，填 0 而不是猜一个值，否则首帧动画从中间开始。
    last_frame_at: Option<std::time::Instant>,
    window: std::sync::Arc<Window>,
    /// D3D11 设备与上下文（唯一一份）。
    gpu: Gpu,
    /// 交换链 + 可选 DComp 树。
    surface: Surface,
    renderer: Renderer,
    view: V,
    theme: &'static Theme,
    /// 帧缓存 —— 保留树 + `DisplayList` + 上一帧视口，全在核心里。
    ///
    /// # 为什么这三样都不在本结构体里
    ///
    /// 它们原来分散在 `node` / `list` / `needs_layout` 三个字段里，而**数据**全在
    /// core（`Node` / `DisplayList` / `Dirty` / `DiffStats`）。本 crate 于是要自己
    /// 编排「要不要重排 / 要不要重绘」—— 那正是 `zephyra/zephyra-base/README.md` 第二条
    /// 契约禁止的（base 不决定画什么）。
    ///
    /// 现在这一帧画什么只有 core 一个地方决定（[`FrameCache::frame`]），本 crate
    /// 只回答另一个问题：要不要把它推上屏。
    cache: FrameCache,
    router: zephyra_core::input::InputRouter,
    pointer: Option<u32>,
    pointer_pos: (f32, f32),
    /// 有没有**待处理的事件**（resize / ptr_of / 按键 / 切主题）。
    ///
    /// ⚠ 这**不是**「内容变了」。内容变没变由 core 判定（`FrameOutcome`）。
    /// 本字段只表示「值不值得跑一帧」—— 因为判断后者必须先跑 `view()`，而那是
    /// 应用代码。
    dirty: bool,
    /// 文本引擎。
    ///
    /// 泛型参数固定为 [`text::AbGlyphSource`]（本 crate 的唯一字体后端）。用
    /// `RefCell` 是为了让 core 的 `Context` 保持 `Copy` —— 布局递归要求这一点。
    engine: std::cell::RefCell<TextEngine>,
    /// 已上传的图集字形数，用来避免每帧重传 4MB。
    atlas_revision_uploaded: u64,
    /// 图像纹理表：`Command::Image` 的句柄 → 采样视图。
    textures: TextureTable,
}

impl<V: View> HostWindow<V> {
    /// 建窗口与 GPU 资源，**找不到系统字体时不 panic**。
    ///
    /// 字体缺失不该让整个应用起不来：文本会不显示，其余部分照常工作。这与"崩溃"
    /// 之间的差别很大 —— 后者让"这台机器没装字体"变成"这个程序不能跑"。
    ///
    /// 同步构造（可在 winit 的 `resumed` 回调里直接调用）：D3D11 的设备与交换链
    /// 都是同步 API，不像 wgpu 的设备请求那样需要 `block_on` 两个 future
    /// —— 所以本 crate **不再依赖 pollster**。
    pub fn new_lenient(event_loop: &ActiveEventLoop, view: V) -> Self {
        let attrs = Window::default_attributes().with_title("ZephyraUI");
        let window = std::sync::Arc::new(event_loop.create_window(attrs).expect("create_window"));
        let size = window.inner_size();

        // 文本引擎：core 的 `TextStack` + 本 crate 的 ab_glyph 字形来源。
        // 找不到字体时退回禁用模式 —— 文本不显示，其余部分照常。
        let (engine, note) = text::from_system_font();
        eprintln!("[zephyra] {note}");

        let gpu = Gpu::new().expect("创建 D3D11 设备：需要可用的 GPU");

        // DComp 走不走由 `ZEPHYRA_DCOMP` 决定。它只影响**要不要建 DComp 树**，
        // 两条路的调用方式完全一致（见 `surface` 模块文档），所以这里没有回退分支：
        // 启用后 DComp 建树失败就是失败，不静默退回 —— 那会让"我开了 DComp 却没生效"
        // 无从排查。
        let use_dcomp = dcomp_enabled();
        let hwnd = surface::hwnd_of(window.as_ref())
            .map(|h| windows::Win32::Foundation::HWND(h as *mut _))
            .ok_or_else(|| panic!("DComp 需要 Win32 HWND；非 Windows 后端尚未实现渲染器"))
            .expect("取 HWND");

        let surface = match Surface::new(hwnd, &gpu, (size.width, size.height), use_dcomp) {
            Ok(s) => s,
            Err(e) => {
                if use_dcomp {
                    // 建树失败不能静默：少了 DComp 特性窗口依然完全可用，为此启动失败
                    // 是本末倒置；但**必须让用户看得见**，否则无从排查。
                    eprintln!("[zephyra] DComp 不可用（{e}），退回普通 DXGI 交换链");
                    Surface::new(hwnd, &gpu, (size.width, size.height), false).expect("普通交换链")
                } else {
                    panic!("创建交换链失败: {e}");
                }
            }
        };
        eprintln!(
            "[zephyra] 呈现路径: {}",
            if use_dcomp {
                "DirectComposition"
            } else {
                "DXGI swapchain"
            }
        );

        let renderer = Renderer::new(&gpu.device).expect("编译着色器 / 建管线");
        // 首帧就要上传图集（字体字形此刻还没打包，但纹理得先有内容）。
        renderer.upload_atlas(&gpu.context, engine.atlas()).ok();

        let theme: &'static Theme = Box::leak(Box::new(Theme::dark()));

        Self {
            last_frame_at: None,
            window,
            gpu,
            surface,
            renderer,
            view,
            theme,
            cache: FrameCache::new(),
            router: zephyra_core::input::InputRouter::new(),
            pointer: None,
            pointer_pos: (0.0, 0.0),
            dirty: true,
            engine: std::cell::RefCell::new(engine),
            atlas_revision_uploaded: 0,
            textures: TextureTable::new(),
        }
    }

    /// 装一个样式注入点（每帧 diff 之后、布局之前调用）。
    ///
    /// 转发给 [`FrameCache::set_styler`]。分离在这里而不是塞进 `new` 的签名：
    /// 样式表是**运行期**才加载的东西（dev 模式还要能热重载），
    /// 而窗口是启动时就得建好的。把它塞进构造器会让「起窗口」依赖
    /// 「读样式表成功」——那样一个拼错的属性名就让程序起不来。
    pub fn set_styler(&mut self, styler: Box<dyn zephyra_core::styler::Styler>) {
        self.cache.set_styler(styler);
    }

    /// 图像纹理表（只读）。宿主用 [`Self::textures_mut`] 改。
    pub fn textures(&self) -> &TextureTable {
        &self.textures
    }

    /// 图像纹理表（可变）。宿主在这里注册 / 更新纹理。
    pub fn textures_mut(&mut self) -> &mut TextureTable {
        &mut self.textures
    }

    /// 注册一张图像纹理，返回写进 `props::IMAGE_TEXTURE` 的句柄。
    ///
    /// 像素是 RGBA8、行优先。注册后**不要**销毁纹理 —— 表里持有它的采样视图，
    /// 而视图引用纹理的内存。要释放纹理时先 [`Self::forget_texture`]。
    ///
    /// 同名重复注册会**替换**纹理，且**不需要**任何额外的清理调用：下一帧自然用
    /// 新的采样视图（D3D11 每次 draw 直接绑 SRV，没有 wgpu 那种需要手动失效的
    /// bind group 缓存）。
    pub fn register_texture(
        &mut self,
        name: &str,
        pixels: &[u8],
        width: u32,
        height: u32,
    ) -> zephyra_core::paint::TextureRef {
        let tex = texture::create_rgba_texture(
            &self.gpu.device,
            &self.gpu.context,
            pixels,
            width,
            height,
        )
        .expect("创建纹理");
        let srv = texture::create_srv(&self.gpu.device, &tex).expect("创建采样视图");
        self.textures.insert(name, srv, tex);
        zephyra_core::paint::TextureRef::new(name.to_string())
    }

    /// 原地更新一张已注册纹理的像素（尺寸不变）。供视频帧、动态图标用。
    ///
    /// 返回 `false` = 纹理未注册或像素长度不足 —— 此时什么都没写入，调用方必须
    /// 自己重建纹理。采样视图不变，所以没有任何需要失效的缓存。
    pub fn update_texture(&self, name: &str, pixels: &[u8], width: u32, height: u32) -> bool {
        self.textures
            .update(&self.gpu.context, name, pixels, width, height)
    }

    /// 移除一张图像纹理。**必须在销毁纹理之前调用。**
    pub fn forget_texture(&mut self, name: &str) {
        self.textures.remove(name);
    }

    /// 当前主题。
    pub fn theme(&self) -> &Theme {
        self.theme
    }

    /// 窗口物理像素尺寸。
    pub fn size(&self) -> (u32, u32) {
        self.surface.size()
    }

    /// 窗口缩放比。断点判定要它把物理视口折成有效像素（见 `InputState::viewport`）。
    pub fn scale_factor(&self) -> f32 {
        self.window.scale_factor() as f32
    }

    /// 切主题：换 `Theme` 引用并强制重绘。
    ///
    /// 树**不动** —— 属性里存的资源键保持为键，切主题只是换了一个 `&Theme`。
    pub fn set_theme(&mut self, theme: Theme) {
        self.theme = Box::leak(Box::new(theme));
        self.dirty = true;
    }

    /// 当前悬停的可交互节点 key（`Display` 形式，供示例比对）。
    pub fn hovered(&self) -> Option<String> {
        self.router.hovered().map(|k| k.to_string())
    }

    /// 推进一帧：声明 → 核心帧缓存 → （必要时）回放。
    ///
    /// # 分工
    ///
    /// 「这一帧画什么」由 [`FrameCache::frame`] 判定 —— 包括「与上次完全相同，
    /// 什么都不用做」。本函数只在 core 说 [`FrameOutcome::Updated`] 时才回放，
    /// 所以稳定帧省掉的是 `paint`（73 µs）+ `expand`（40 µs）+ 一次 present。
    ///
    /// ⚠ **图集上传不受这个门控保护**，它另有判据 —— 见下面那条警告。
    pub fn frame(&mut self) -> Frame {
        let theme = self.theme;
        let size = self.size();
        // 作用域要短：`ctx` 借着 `self.engine`，而下面要可变借用 `self` 去回放。
        // 把树的构建、图集上传全部放进这个作用域，ctx 在离开时释放。
        let (stats, damage, redrawn) = {
            // 挂上内置图标集：矢量图标走完整的「图标名 → 展平 → 三角化 →
            // `Command::Path`」链条。理由与 `zephyra-winit-d3d12` 那份一致。
            let ctx =
                Context::with_engine(theme, &self.engine).with_icons(zephyra_core::icon::builtin());

            // 0. 把路由器持有的输入状态推给视图 —— hover 必然晚于上一帧的树（命中
            //    测试要读已布局的矩形），所以只能从下一帧开始生效。
            let input = InputState {
                hovered: None,
                pressed: self.router.pressed().cloned(),
                pointers: self.router.pointer_count(),
                // 与 D3D12 那份同源：视口走 `on_input` 推给视图，断点判定要用。
                // （这个后端本身不参与响应式，但接口必须一致，否则每加一个后端
                //   就要为「忘了这个字段」付一次排查成本。）
                viewport: (size.0 as f32, size.1 as f32),
                scale: self.scale_factor(),
                // D3D11 后端同样要有帧间隔：动画时钟的口径只在 InputState 一处，
                // 这里不填就让"接动画"变成逐个后端补字段。
                dt_ms: {
                    let now = std::time::Instant::now();
                    let dt = self
                        .last_frame_at
                        .map(|t| now.duration_since(t).as_secs_f32() * 1000.0)
                        .unwrap_or(0.0);
                    self.last_frame_at = Some(now);
                    dt
                },
                // 这个后端不接 `zephyra-platform`（它只管 D3D11 画面），所以语言
                // 偏好给**空表**而不是猜一个。空表在 `Gallery` 里的含义是
                // "读不到" ⇒ 回落 `SUPPORTED` 第一项，不会把界面钉死。
                //
                // ⚠️ 不能省掉这个字段：漏填是编译错误，而"填成英语"是行为变更 ——
                // 后者要等到有人在这台机器上用中文系统跑起来才会被发现。
                languages: Vec::new(),
            };
            self.view.on_input(&input);

            let element = self.view.view(&ctx);

            // 1. 声明 → 保留 → （必要时）布局 → 降级，全部在核心里完成。
            let outcome = self
                .cache
                .frame(&element, ctx, (size.0 as f32, size.1 as f32));

            // ⚠️ 图集上传的判据是 `atlas_revision()`，**不是**「本帧有没有重绘」。
            //
            // 这两件事不等价，而把它们混为一谈会出一个极难查的 bug：字形图集满时的
            // **淘汰重排**（`zephyra_core::text::atlas` 的 `make_room`）发生在 layout 的
            // measure 阶段，那时 `FrameCache` 可能判定「无节点 paint-dirty」而返回
            // `Unchanged` —— 于是图集这次没上传，而 GPU 还抱着**重排前**的纹理。
            //
            // 症状是「文字忽然整片消失，或每个字都跑到别的位置上」，且**只在图集
            // 填满之后出现**，日志里一切正常。所以这里无条件检查，与 `outcome` 无关。
            //
            // 判据不能换成「已缓存字形数」：图集加了 LRU 淘汰之后，「数量不变 ⇒ 内容
            // 不变」这条**不成立** —— 淘汰一个再放进一个，数量不变而纹理已整体重排。
            let revision_now = self.engine.borrow().atlas_revision();
            if revision_now != self.atlas_revision_uploaded {
                if self
                    .renderer
                    .upload_atlas(&self.gpu.context, self.engine.borrow().atlas())
                    .is_err()
                {
                    eprintln!("[zephyra] 图集上传失败，本帧文字可能不显示");
                }
                self.atlas_revision_uploaded = revision_now;
            }
            // ⚠️ 在作用域**内**就把 `FrameOutcome` 拆成 `Copy` 的标量。
            //
            // `FrameOutcome::Updated` 借着一份 `&DisplayList`，而下面
            // `acquire_and_render` 要 `&mut self` —— 直接把 `outcome` 带出作用域会
            // 让借用活到函数末尾，编译期就撞上（`cannot borrow *self as mutable more
            // than once`）。这里提前解构，列表留到回放时用
            // `self.cache.list()` 现取。
            match outcome {
                FrameOutcome::Unchanged => (DiffStats::default(), Damage::Full, false),
                FrameOutcome::Updated { stats, damage, .. } => (stats, damage, true),
            }
        };

        // 2. DisplayList → 像素。ctx 已出作用域，`self` 可变借用恢复。
        //
        // ⚠ `Unchanged` 时**连 present 都不能做**：本项目两个后端都是 flip model，
        // `Present` 之后后缓冲内容未定义。不画就 present 会把未定义的内容推上屏。
        // 什么都不做才是对的 —— 合成器继续显示上一帧。
        let report = if redrawn {
            self.acquire_and_render(damage)
        } else {
            RenderReport::default()
        };

        // 3. 清事件标志。脏标记由 `FrameCache` 内部清（它知道什么时候该清）。
        self.dirty = false;

        Frame {
            diff: stats,
            render: report,
            hovered: self.hovered(),
            damage,
            redrawn,
        }
    }

    /// 取本帧渲染目标并回放 `DisplayList`。
    ///
    /// `damage` 只用于**帧内** scissor —— 它不能用来跳过回放，原因见
    /// `Renderer::render` 里关于 flip model 的说明。
    fn acquire_and_render(&mut self, _damage: Damage) -> RenderReport {
        // # 「只重画脏区」的前提：脏区之外的上一帧画面**还在渲染目标里**
        //
        // flip model 的后缓冲在 `Present` 之后**内容未定义** —— 它是会被丢弃的
        // 临时目标，不是「上一帧那张图」。本后端的渲染目标**就是**那个后缓冲
        // （没有 MSAA 中间纹理），所以按脏区绘制等于把未定义像素留在屏幕上：
        // 症状是「点一下之后界面花成一整片底色，只有那一小块是对的」。
        //
        // 因此这里强制整帧，**忽略**调用方给的 `damage`。代价写在明处：交互帧
        // 整帧重绘；换来的是画面永远对。（D3D12 后端有 MSAA 中间纹理，
        // 那条路上按脏区绘制才成立，见它的 `render_pass`。）
        self.acquire_and_render_inner(true, Damage::Full)
    }

    fn acquire_and_render_inner(&mut self, allow_dump: bool, damage: Damage) -> RenderReport {
        // 拆开借用：`Renderer::render` 要 `&mut self.renderer`，而取 RTV 要
        // `&self.surface`。整体借用 `self` 会在第一个就撞上借用检查器。
        let rtv = match self.surface.current_rtv() {
            Ok(v) => v.clone(),
            Err(e) => {
                eprintln!("[zephyra] 取不到后缓冲: {e}");
                return RenderReport::default();
            }
        };
        let clear = self.theme.surface;
        let report = self.renderer.render(RenderCtx {
            context: &self.gpu.context,
            device: &self.gpu.device,
            rtv: &rtv,
            list: self.cache.list(),
            damage,
            size: self.surface.size(),
            clear,
            textures: &self.textures,
        });

        // 诊断截帧必须在 `present` **之前**，且由 `read_pixels` 用
        // `current_rtv()` 反查资源 —— 两个条件缺一不可。
        //
        // 实测三种组合（Intel UHD + flip model，只探到 1 个可用后缓冲）：
        //
        // | 何时读 | 按什么取 | 结果 |
        // |---|---|---|
        // | present **前** | `current_rtv()` 反查 | **完整一帧** ✅ |
        // | present **前** | `GetCurrentBackBufferIndex` | 撕裂：左半有内容、右半全黑 |
        // | present **后** | 新索引 | 全黑 |
        //
        // 两次误判的教训是同一句：**D3D11 的失败模式是"不报错但内容不对"，所以
        // 唯一可靠的判据是看像素**。当时日志里一切正常（`quads=143`、零 HRESULT）。
        if allow_dump {
            self.maybe_dump_frame();
        }

        // ⚠ MSAA 解析必须在 present **之前**。
        //
        // 画进 MSAA 纹理的内容不会出现在后缓冲上 —— 后缓冲拿到的是上一次
        // `ResolveSubresource` 的结果。漏掉这步的症状是「画面卡在旧内容」
        // 或「全黑」，且**零 HRESULT 错误**。
        //
        // 位置也在 `maybe_dump_frame` **之后**：截图走 `read_pixels`，而它内部
        // 已经自己解析了一次（见 `Surface::read_pixels`）。两次解析幂等（后者读
        // 同一份已解析内容），但让截图与屏幕**同源**比省一次 GPU 操作重要。
        if let Err(e) = self.surface.resolve_msaa() {
            eprintln!("[zephyra] MSAA 解析失败: {e}");
            return RenderReport::default();
        }

        // 提交失败不能静默：丢掉一帧还能继续，丢掉整帧的呈现会让窗口停住更新，
        // 而那与"UI 卡住了"是同一种症状 —— 区别只有日志里有。
        if let Err(e) = self.surface.present(1) {
            eprintln!("[zephyra] present 失败: {e}");
            return RenderReport::default();
        }
        report
    }

    /// 处理一次指针移动：命中测试 → 路由 → 是否需要重绘。
    pub fn on_cursor_moved(&mut self, x: f32, y: f32) -> bool {
        let id = *self.pointer.get_or_insert(0);
        self.pointer_pos = (x, y);
        let Some(node) = self.cache.node() else {
            return false;
        };
        let theme = self.theme;
        let before = self.router.revision();
        let mut out = RoutedEvents::new();
        self.router.on_pointer(
            node,
            PointerEvent {
                id,
                x,
                y,
                phase: PointerPhase::Move,
                source: PointerSource::Mouse,
            },
            Context::new(theme),
            &mut out,
        );
        let changed = self.router.revision() != before;
        if changed {
            self.dirty = true;
        }
        changed
    }

    /// 处理一次按下：返回是否产生了路由事件。
    pub fn on_pointer_down(&mut self) -> bool {
        let id = *self.pointer.get_or_insert(0);
        let Some(node) = self.cache.node() else {
            return false;
        };
        let (x, y) = self.pointer_pos;
        let theme = self.theme;
        let mut out = RoutedEvents::new();
        self.router.on_pointer(
            node,
            PointerEvent {
                id,
                x,
                y,
                phase: PointerPhase::Down,
                source: PointerSource::Mouse,
            },
            Context::new(theme),
            &mut out,
        );
        let any = !out.is_empty();
        if any {
            self.dirty = true;
        }
        any
    }

    /// 诊断：把某一帧的后缓冲写成 BMP（`ZEPHYRA_FRAME_DUMP=<路径>` 时启用）。
    ///
    /// **必须在 `present` 之前调用。** flip model 下 `Present` 一执行，DXGI 就
    /// 可以回收/换掉那张后缓冲；之后再 `CopyResource` 拿到的是**另一张**（通常
    /// 是刚被清空的、或还没被画的）—— 读出来全零，而画面其实是对的。
    ///
    /// 这就是为什么它挂在 [`Self::acquire_and_render`] 内部（渲染完、提交前）
    /// 而不是 `window_event` 里。我第一版挂在 `window_event` 里，读回的
    /// STAGING 纹理**全零**，而那正是这个原因 —— 差点误判成"渲染器什么都没画"。
    ///
    /// # 为什么需要它
    ///
    /// 本项目的渲染失败模式**不是崩溃，而是"不崩但画面不对"**：着色器没绑上、
    /// 顶点写歪了、混合状态错了 —— 症状全都是"一片纯色"或"全黑"，而
    /// `RenderReport` 里 `quads=143` 一切正常。
    ///
    /// "进程没崩"与"画对了"是两件事，而后者只能靠看像素证明。这个开关把后缓冲
    /// 落到文件上，于是"画对了"变成可复核的。
    ///
    /// 写到 `ZEPHYRA_FRAME_DUMP` 给出的路径；`-` 表示只统计颜色数不写文件。
    fn maybe_dump_frame(&self) {
        // 调试层消息**每帧都要拉** —— 与是否 dump 像素无关。
        //
        // 早先把这一句放在 `read_pixels` 成功分支之后，于是单独打开
        // ZEPHYRA_D3D_DEBUG=1 时**一条消息都没有**：而 D3D11 的问题从不以 HRESULT
        // 形式出现，只会进消息队列。所以「开了调试层却没输出」看起来像「没有消息」，
        // 实际上是「根本没去拉」。
        self.surface.dump_debug_messages();

        static ONCE: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
        let path = ONCE.get_or_init(|| std::env::var("ZEPHYRA_FRAME_DUMP").ok());
        let Some(path) = path.as_deref() else {
            return;
        };
        // 前几帧每次都报告（颜色数是关键判据），之后只写文件不报告。
        //
        // 帧号存在环境变量里：不同问题要看的帧不同（首帧看初始化，稳态看管线）。
        //
        // 注意：**稳态可能只有 1~2 帧**。`frame()` 只在 `dirty` 时被调用，而
        // 一帧跑完 `dirty` 立刻归零 —— 所以静止的 demo 总共只渲染那么几帧。
        // 指望"第 30 帧"是等不到的（我第一版就写了这个，结果永远不触发）。
        static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        const REPORT_UPTO: u32 = 4;
        if n > REPORT_UPTO {
            return;
        }
        match self.surface.read_pixels() {
            Ok(px) => {
                // 不同颜色数是"画没画出东西"最直接的判据：全屏纯色（无论黑白）
                // 都意味着什么都没画上。
                let colors = px.distinct_colors(2);
                // 顺便报几个采样点的原始字节值 —— 出问题时"全黑"与"应该是深灰"
                // 的区别就在这几十个字节里，不必去看图。
                let samples: Vec<String> = [2usize, 40, 300]
                    .iter()
                    .filter_map(|y| {
                        let row = px.row(*y as u32);
                        let i = (px.width as usize / 2) * 4;
                        row.get(i..i + 4)
                            .map(|p| format!("y={y}: BGRA=({},{},{},{})", p[0], p[1], p[2], p[3]))
                    })
                    .collect();
                if path != "-" && n == 0 {
                    if let Err(e) = write_bmp(path, &px, true) {
                        eprintln!("[zephyra] 写 {path} 失败: {e}");
                        return;
                    }
                }
                // 尺寸不一致必须**显式**说出来：症状是画面右侧/底部一整条黑边，
                // 而其它一切（HRESULT、RenderReport）都正常。不说的话，看图的人
                // 只会以为"边框就这么宽"。
                // 尺寸不一致必须**显式**说出来：症状是画面右侧/底部一整条黑边，
                // 而其它一切（HRESULT、RenderReport）都正常。
                let size_note = if px.size_matches() {
                    String::new()
                } else {
                    format!(
                        "；⚠ surface {}x{} 而后缓冲 {}x{}",
                        px.declared_width, px.declared_height, px.width, px.height,
                    )
                };
                // 窗口客户区尺寸也打出来：它是\"交换链该有多大\"的唯一权威，
                // 而 surface 里的 size 是我们自己记的 —— 两者不一致就说明
                // 某条 resize 路径漏掉了。
                let window_size = self.window.inner_size();
                // 清屏色在图上的样子（线性 0.063 经 sRGB 编码约 0x2A=42/43/45）
                // 用来定位\"哪些区域真的被清过\"。
                let clear_ref = srgb_bytes(self.theme.surface);
                let clear_str = format!("{},{},{}", clear_ref[0], clear_ref[1], clear_ref[2]);
                eprintln!(
                    "[zephyra] 第 {n} 帧 后缓冲 {}x{} 行距 {} surface {}x{} 窗口 {}x{} \
                     不同颜色 {colors} 种{size_note}；清屏≈{clear_str}；{}",
                    px.width,
                    px.height,
                    px.row_pitch,
                    px.declared_width,
                    px.declared_height,
                    window_size.width,
                    window_size.height,
                    samples.join("  "),
                );
            }
            Err(e) => eprintln!("[zephyra] 读回像素失败: {e}"),
        }
        // `ZEPHYRA_D3D_DEBUG=1`（建了带调试层的设备）时把驱动自己的话打出来。
        //
        // 放在这里而不是只在出错时打：D3D11 的问题**从不以 HRESULT 形式出现**，
        // 逐帧打才看得出"哪一帧开始不再报"。
        self.surface.dump_debug_messages();
    }

    /// 让呈现路径按当前 `surface` 尺寸重新配置。
    ///
    /// 只有"宿主在**没有**收到 `Resized` 的情况下改了尺寸"才需要它 —— 例如宿主
    /// 自己接管了尺寸来源。走 winit 事件循环时 `Resized` / `ScaleFactorChanged`
    /// 已经直接调 `surface.resize`，所以本方法通常用不上。
    ///
    /// 之所以留着而不是删掉：它的存在让"尺寸变更只有一个入口"这件事显式化 ——
    /// `Surface::resize` 幂等（尺寸相同就提前返回），多一个调用点不会出错。
    pub fn reconfigure_present(&mut self) {
        let (w, h) = self.surface.size();
        self.surface.resize(w, h);
    }
}

impl<V: View> ApplicationHandler for HostWindow<V> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.dirty = true;
        event_loop.set_control_flow(ControlFlow::Poll);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                let (w, h) = (size.width.max(1), size.height.max(1));
                self.surface.resize(w, h);
                self.dirty = true;
            }
            WindowEvent::RedrawRequested => {
                if self.dirty {
                    let f = self.frame();
                    if std::env::var("ZEPHYRA_TRACE").as_deref() == Ok("1") {
                        eprintln!(
                            "[zephyra] quads={} skipped={} img_batches={} missing_tex={} \
                             dirty={} needs_layout={} hovered={:?}",
                            f.render.quads,
                            f.render.skipped,
                            f.render.image_batches,
                            f.render.missing_textures,
                            f.diff.dirtied,
                            f.diff.needs_layout,
                            f.hovered
                        );
                    }
                }
                self.window.request_redraw();
            }
            WindowEvent::CursorMoved { position, .. } => {
                if self.on_cursor_moved(position.x as f32, position.y as f32) {
                    self.dirty = true;
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                if self.on_pointer_down() {
                    self.dirty = true;
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            } => {
                // 抬起必须成对处理：否则按下捕获会残留，下一次点击会被判为抬起。
                let id = self.pointer.unwrap_or(0);
                if let Some(node) = self.cache.node() {
                    let (x, y) = self.pointer_pos;
                    let theme = self.theme;
                    let mut out = RoutedEvents::new();
                    self.router.on_pointer(
                        node,
                        PointerEvent {
                            id,
                            x,
                            y,
                            phase: PointerPhase::Up,
                            source: PointerSource::Mouse,
                        },
                        Context::new(theme),
                        &mut out,
                    );
                    if !out.is_empty() {
                        self.dirty = true;
                    }
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state == ElementState::Pressed {
                    match event.logical_key {
                        Key::Named(NamedKey::Escape) => event_loop.exit(),
                        Key::Character(c) if c.as_str() == "t" => {
                            // 演示「切主题不碰树」：只换一个 &Theme，树完全不动。
                            let next =
                                if self.theme.variant == zephyra_core::theme::ThemeVariant::Dark {
                                    Theme::light()
                                } else {
                                    Theme::dark()
                                };
                            self.set_theme(next);
                        }
                        _ => {}
                    }
                }
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                let s = self.window.inner_size();
                self.surface.resize(s.width.max(1), s.height.max(1));
                self.dirty = true;
            }
            _ => {}
        }
    }
}

/// 用一个已构造好的窗口宿主跑事件循环。
pub fn run_with<V: View>(app: HostWindow<V>) -> Result<(), winit::error::EventLoopError> {
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = app;
    event_loop.run_app(&mut app)
}

/// 用**已有**的 `EventLoop` 跑窗口宿主。
///
/// winit 0.30 要求 `run_app` 收到的 `EventLoop` 与建窗时那个是同一个，所以
/// [`HostWindow::new_lenient`] 的调用方必须自己持有它 —— 这时用本函数，而不是会
/// 新建 `EventLoop` 的 [`run_with`]。
pub fn run_with_on<V: View>(
    event_loop: EventLoop<()>,
    app: HostWindow<V>,
) -> Result<(), winit::error::EventLoopError> {
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = app;
    event_loop.run_app(&mut app)
}

/// 便捷构造：一个带背景、圆角与尺寸的元素。
pub fn panel(key: &'static str, bg: Color, radius: f32) -> Element {
    Element::new(NodeTag::Container)
        .with_key(key)
        .with(&props::BACKGROUND, PropertyValue::color(bg))
        .with(&props::CORNER_RADIUS, PropertyValue::float(radius))
}

// `srgb_bytes` 已搬到 `zephyra_fx::pixels`（它与后端无关）。上面的
// `pub use zephyra_fx::srgb_bytes` 让调用点不用改。

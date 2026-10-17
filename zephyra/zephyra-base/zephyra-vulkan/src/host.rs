//! 窗口 + GPU + 保留树的组合体。对应 Windows 那边 `host.rs` 的角色。
//!
//! 本文件以 `zephyra-winit-vk/src/host.rs`（wgpu 版）为模板移植，只改 GPU 相关调用。
//! wgpu 版经过实测验证，接口形状直接复用。
//!
//! # 字段顺序是**硬约束**
//!
//! 窗口**最后**声明，因而**最先**被借用、最晚被析构。winit 的窗口句柄被 surface
//! 与 HostWindow 同时持有，而 surface 的生命周期不能比它所依附的窗口长。

use std::sync::Arc;

use ash::vk;
use winit::application::ApplicationHandler;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

use zephyra_core::context::Context;
use zephyra_core::frame::{Damage, FrameCache, FrameOutcome};
use zephyra_core::input::{
    InputRouter, PointerEvent, PointerPhase, PointerSource, RoutedEvents,
};
use zephyra_core::node::Node;
use zephyra_core::theme::{Theme, ThemeVariant};

use zephyra_fx::{Frame, InputState, TextEngine, View};

use crate::renderer::{RenderCtx, Renderer};
use crate::surface::Gpu;

/// 窗口 + GPU + 保留树的组合体。
pub struct HostWindow<V: View> {
    /// 设备、队列与 surface。全进程唯一一份。
    gpu: Gpu,
    renderer: Renderer,
    view: V,
    theme: &'static Theme,
    /// 帧缓存 —— 保留树 + `DisplayList` + 上一帧视口，全在核心里。
    cache: FrameCache,
    router: InputRouter,
    pointer: Option<u32>,
    pointer_pos: (f32, f32),
    dirty: bool,
    engine: std::cell::RefCell<TextEngine>,
    atlas_revision_uploaded: u64,
    backdrop_active: bool,
    system_languages: Vec<String>,
    frames: usize,
    last_frame_at: Option<std::time::Instant>,
    /// 每帧的命令缓冲 + 围栏（复用）。
    cmd: vk::CommandBuffer,
    fence: vk::Fence,
    /// 交换链图的帧缓冲（每个图一个）。resize 后重建。
    framebuffers: Vec<vk::Framebuffer>,
    /// 窗口。**最后析构**。
    window: Arc<Window>,
}

/// 未指定尺寸时的默认客户区尺寸。
pub const DEFAULT_CLIENT_SIZE: (u32, u32) = (1100, 700);

impl<V: View> HostWindow<V> {
    /// 建一个宿主（默认尺寸）。
    pub fn new_lenient(event_loop: &ActiveEventLoop, view: V) -> Self {
        Self::new_lenient_sized(event_loop, view, DEFAULT_CLIENT_SIZE)
    }

    /// 建一个指定客户区尺寸的宿主。
    pub fn new_lenient_sized(event_loop: &ActiveEventLoop, view: V, size: (u32, u32)) -> Self {
        let window = match event_loop.create_window(
            Window::default_attributes()
                .with_title("ZephyraUI")
                .with_inner_size(winit::dpi::LogicalSize::new(size.0, size.1)),
        ) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                eprintln!("[zephyra] 建窗失败：{e}");
                panic!("建窗失败：winit 拿不到窗口");
            }
        };

        let gpu = match Gpu::new(&window) {
            Ok(g) => g,
            Err(e) => {
                eprintln!(
                    "[zephyra] 建立 GPU 失败：{e}\n\
                     [zephyra] 这台机器上可能没有可用的 Vulkan 适配器。"
                );
                panic!("建立 GPU 失败");
            }
        };

        let (engine, note) = zephyra_fx::text::from_system_font();
        eprintln!("[zephyra] {note}");

        // ⚠ 先用单采样跑通；MSAA 需要 renderpass 加 resolve 附件，后续补。
        let samples = 1u32;
        let renderer = match Renderer::new(&gpu, 1024, samples) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("[zephyra] 建渲染管线失败：{e}");
                panic!("建渲染管线失败");
            }
        };

        let platform = zephyra_platform_linux::current();
        let system_languages = platform.system_languages();

        let cmd = unsafe {
            gpu.device.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::builder()
                    .command_pool(gpu.command_pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(1)
                    .build(),
            )
        }
        .expect("command buffer")[0];
        let fence = unsafe {
            gpu.device
                .create_fence(&vk::FenceCreateInfo::default(), None)
        }
        .expect("fence");

        Self {
            gpu,
            renderer,
            view,
            theme: Box::leak(Box::new(Theme::dark())),
            cache: FrameCache::new(),
            router: InputRouter::new(),
            pointer: None,
            pointer_pos: (0.0, 0.0),
            dirty: true,
            engine: std::cell::RefCell::new(engine),
            atlas_revision_uploaded: u64::MAX,
            backdrop_active: false,
            system_languages,
            frames: 0,
            last_frame_at: None,
            cmd,
            fence,
            framebuffers: Vec::new(),
            window,
        }
    }

    /// 装一个样式注入点。
    pub fn set_styler(&mut self, styler: Box<dyn zephyra_core::styler::Styler>) {
        self.cache.set_styler(styler);
    }

    /// 切主题（Light / Dark）。
    pub fn set_theme(&mut self, variant: ThemeVariant) {
        self.theme = Box::leak(Box::new(Theme::builtin(variant)));
        self.cache.invalidate();
    }

    /// 当前主题。
    pub fn theme_variant(&self) -> ThemeVariant {
        self.theme.variant
    }

    /// 图像纹理表。
    pub fn textures(&self) -> &crate::renderer::TextureTable {
        &self.renderer.textures()
    }

    /// 注册 / 重传一张图像纹理。
    pub fn register_texture(
        &mut self,
        name: &str,
        pixels: &[u8],
        width: u32,
        height: u32,
    ) -> Result<(), crate::surface::SurfaceError> {
        self.renderer
            .upload_texture(&self.gpu, name, pixels, (width, height))
    }

    /// 窗口。
    pub fn window(&self) -> &Arc<Window> {
        &self.window
    }

    /// 客户区尺寸（**物理**像素）。
    pub fn client_size(&self) -> (u32, u32) {
        let s = self.window.inner_size();
        (s.width.max(1), s.height.max(1))
    }

    /// 布局用的尺寸（有效像素）。
    pub fn effective_size(&self) -> (f32, f32) {
        let scale = self.window.scale_factor() as f32;
        let (w, h) = self.client_size();
        (w as f32 / scale, h as f32 / scale)
    }

    /// 当前 hover 的节点。
    pub fn hovered(&self) -> Option<&zephyra_core::id::ElementKey> {
        self.router.hovered()
    }

    /// 按应用请求设置窗口外观。
    pub fn apply_window_chrome(
        &mut self,
        chrome: zephyra_platform::WindowChrome,
    ) -> zephyra_platform::ApplyOutcome {
        let outcome = zephyra_platform_linux::current()
            .apply_window_chrome(window_handle_of(&self.window), chrome);
        self.backdrop_active =
            outcome.backdrop && chrome.backdrop != zephyra_platform::BackdropKind::None;
        outcome
    }

    /// 一帧什么都没做时返回的东西。
    fn empty_frame(&self) -> Frame {
        Frame {
            diff: zephyra_core::diff::DiffStats::default(),
            render: zephyra_fx::quad::RenderReport::default(),
            hovered: None,
            damage: Damage::Full,
            redrawn: false,
        }
    }

    /// 跑一帧：布局 → `DisplayList` → 顶点展开 → 提交。
    pub fn frame(&mut self) -> Frame {
        let size = self.client_size();
        let revision_now = self.engine.borrow().atlas_revision();
        if revision_now != self.atlas_revision_uploaded {
            self.cache.invalidate();
        }
        self.atlas_revision_uploaded = revision_now;

        let theme = self.theme;
        let (diff, damage, redrawn) = {
            let ctx = Context::with_engine(theme, &self.engine)
                .with_icons(zephyra_core::icon::builtin());
            let element = self.view.view(&ctx);
            let outcome = self
                .cache
                .frame(&element, ctx, (size.0 as f32, size.1 as f32));
            match outcome {
                FrameOutcome::Unchanged => {
                    (zephyra_core::diff::DiffStats::default(), Damage::Full, false)
                }
                FrameOutcome::Updated { stats, damage, .. } => (stats, damage, true),
            }
        };
        if !redrawn {
            return Frame {
                diff,
                render: zephyra_fx::quad::RenderReport::default(),
                hovered: self.router.hovered().map(|k| k.to_string()),
                damage,
                redrawn: false,
            };
        }

        let (atlas_pixels, atlas_size) = {
            let engine = self.engine.borrow();
            let atlas = engine.atlas();
            (atlas.pixels().to_vec(), atlas.size())
        };
        let ctx = RenderCtx {
            display_list: self.cache.list(),
            size,
            damage,
            atlas_pixels: &atlas_pixels,
            atlas_size,
            atlas_revision: revision_now,
            filters_enabled: true,
            default_blend: zephyra_core::filter::BlendMode::SrcOver,
        };
        let render = match self.renderer.prepare(&self.gpu, &ctx) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("[zephyra] 渲染失败：{e}");
                return self.empty_frame();
            }
        };
        self.frames += 1;
        if trace_enabled() {
            eprintln!(
                "[trace] frame {} 客户区 {:?} 顶点 {} 字节 四边形 {} 未实现混合 {}",
                self.frames,
                self.client_size(),
                render.vertex_bytes,
                render.quads,
                render.unimplemented_blends,
            );
        }
        Frame {
            diff,
            render,
            hovered: self.router.hovered().map(|k| k.to_string()),
            damage,
            redrawn: true,
        }
    }

    /// 上屏：开渲染通道 → 画 → 提交 → present。
    pub fn present(&mut self) {
        // 帧缓冲可能还没建（首次或 resize 后）。
        if self.framebuffers.is_empty() || self.framebuffers.len() != self.gpu.images.len() {
            self.rebuild_framebuffers();
        }
        let (idx, wait_sem) = match self.gpu.acquire() {
            Ok(v) => v,
            Err(_) => return,
        };
        let signal_sem = unsafe {
            self.gpu
                .device
                .create_semaphore(&vk::SemaphoreCreateInfo::default(), None)
        }
        .expect("semaphore");
        let fb = match self.framebuffers.get(idx as usize) {
            Some(fb) => *fb,
            None => return,
        };

        // 开渲染通道（用最近一次 frame() 录好的命令）。
        unsafe {
            self.gpu
                .device
                .begin_command_buffer(
                    self.cmd,
                    &vk::CommandBufferBeginInfo::builder()
                        .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT)
                        .build(),
                )
                .ok();
            self.gpu.device.cmd_begin_render_pass(
                self.cmd,
                &vk::RenderPassBeginInfo::builder()
                    .render_pass(self.renderer.msaa_render_pass)
                    .framebuffer(fb)
                    .render_area(vk::Rect2D {
                        offset: vk::Offset2D { x: 0, y: 0 },
                        extent: vk::Extent2D {
                            width: self.gpu.extent.0,
                            height: self.gpu.extent.1,
                        },
                    })
                    .clear_values(&[vk::ClearValue {
                        color: vk::ClearColorValue {
                            float32: [0.0, 0.0, 0.0, 0.0],
                        },
                    }])
                    .build(),
                vk::SubpassContents::INLINE,
            );
            self.gpu.device.cmd_set_viewport(
                self.cmd,
                0,
                &[vk::Viewport {
                    x: 0.0,
                    y: 0.0,
                    width: self.gpu.extent.0 as f32,
                    height: self.gpu.extent.1 as f32,
                    min_depth: 0.0,
                    max_depth: 1.0,
                }],
            );
            self.gpu.device.cmd_set_scissor(
                self.cmd,
                0,
                &[vk::Rect2D {
                    offset: vk::Offset2D { x: 0, y: 0 },
                    extent: vk::Extent2D {
                        width: self.gpu.extent.0,
                        height: self.gpu.extent.1,
                    },
                }],
            );
            // ⚠ 绘制命令必须**在渲染通道内**录制 —— 这是本次重构的核心：
            // frame() 只做顶点展开与上传（prepare），真正的绘制在这里录。
            self.renderer
                .record_draws(self.cmd, zephyra_core::filter::BlendMode::SrcOver);
            self.gpu.device.cmd_end_render_pass(self.cmd);
            self.gpu.device.end_command_buffer(self.cmd).ok();
        }
        let _ = self
            .gpu
            .submit_and_present(self.cmd, wait_sem, signal_sem, idx, self.fence);
        // 等 GPU 完成（简化）。
        unsafe {
            self.gpu.device.device_wait_idle().ok();
        }
    }

    /// 重建帧缓冲（resize 后或首次 present 时）。
    fn rebuild_framebuffers(&mut self) {
        // 销毁旧的。
        for fb in &self.framebuffers {
            unsafe {
                self.gpu.device.destroy_framebuffer(*fb, None);
            }
        }
        self.framebuffers.clear();
        let (w, h) = self.gpu.extent;
        for &img in &self.gpu.images {
            let view = unsafe {
                self.gpu.device.create_image_view(
                    &vk::ImageViewCreateInfo::builder()
                        .image(img)
                        .view_type(vk::ImageViewType::TYPE_2D)
                        .format(self.gpu.format)
                        .subresource_range(vk::ImageSubresourceRange {
                            aspect_mask: vk::ImageAspectFlags::COLOR,
                            base_mip_level: 0,
                            level_count: 1,
                            base_array_layer: 0,
                            layer_count: 1,
                        })
                        .build(),
                    None,
                )
            }
            .expect("image view");
            let fb = unsafe {
                self.gpu.device.create_framebuffer(
                    &vk::FramebufferCreateInfo::builder()
                        .render_pass(self.renderer.msaa_render_pass)
                        .attachments(&[view])
                        .width(w)
                        .height(h)
                        .layers(1)
                        .build(),
                    None,
                )
            }
            .expect("framebuffer");
            self.framebuffers.push(fb);
        }
    }

    /// 把当前输入状态推给视图。
    pub fn sync_input(&mut self) {
        let size = self.client_size();
        let scale = self.window.scale_factor();
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
            languages: self.system_languages.clone(),
        };
        self.view.on_input(&state);
    }

    /// 处理指针移动。返回是否产生了路由事件。
    pub fn on_cursor_moved(&mut self, x: f32, y: f32) -> bool {
        self.pointer_pos = (x, y);
        let theme = self.theme;
        let pos = PointerEvent {
            id: 0,
            x,
            y,
            phase: PointerPhase::Move,
            source: PointerSource::Mouse,
        };
        self.route(|router, node, out| {
            router.on_pointer(node, pos, Context::new(theme), out);
        })
    }

    /// 处理一次按下。
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

    /// 处理一次抬起。
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

    /// 窗口是否背后真的挂着系统背景材质。
    pub fn backdrop_active(&self) -> bool {
        self.backdrop_active
    }
}

impl<V: View> ApplicationHandler for HostWindow<V> {
    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {
        // 帧缓冲在首次 present 时建。
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if self.dirty {
            self.dirty = false;
            self.sync_input();
            let frame = self.frame();
            if frame.redrawn {
                self.present();
            }
        }
        self.window.request_redraw();
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        id: WindowId,
        event: winit::event::WindowEvent,
    ) {
        use winit::event::{ElementState, MouseButton, WindowEvent};
        match event {
            WindowEvent::CloseRequested if id == self.window.id() => event_loop.exit(),
            WindowEvent::Resized(size) => {
                self.gpu.resize(&self.window);
                self.dirty = true;
                self.framebuffers.clear();
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                self.cache.invalidate();
                self.dirty = true;
            }
            WindowEvent::CursorMoved { position, .. } => {
                if self.on_cursor_moved(position.x as f32, position.y as f32) {
                    self.dirty = true;
                }
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => match state {
                ElementState::Pressed => {
                    if self.on_pointer_down() {
                        self.dirty = true;
                    }
                }
                ElementState::Released => {
                    if self.on_pointer_up() {
                        self.dirty = true;
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
}

fn trace_enabled() -> bool {
    matches!(std::env::var("ZEPHYRA_TRACE").as_deref(), Ok("1") | Ok("true"))
}

fn window_handle_of(window: &Window) -> zephyra_platform::WindowHandle {
    use raw_window_handle::HasWindowHandle;
    let raw = window.window_handle().expect("window handle").as_raw();
    // RawWindowHandle -> usize（X11/Wayland 句柄转换）
    let usize_handle = match raw {
        raw_window_handle::RawWindowHandle::Xlib(h) => h.window as usize,
        raw_window_handle::RawWindowHandle::Xcb(h) => h.window.get() as usize,
        raw_window_handle::RawWindowHandle::Wayland(h) => h.surface.as_ptr() as usize,
        _ => 0,
    };
    zephyra_platform::WindowHandle::from_raw(usize_handle)
}
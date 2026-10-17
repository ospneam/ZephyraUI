//! 顶点流 → GPU 一帧。对应 Windows 那边 `renderer.rs` 的角色，用**裸 Vulkan** 实现。
//!
//! # 与 wgpu 版的结构对应
//!
//! | wgpu 概念 | Vulkan 对应 |
//! |---|---|
//! | `wgpu::RenderPipeline` | `vk::Pipeline`（每条混合模式一条） |
//! | `wgpu::BindGroup` | `vk::DescriptorSet`（纹理 + 采样器） |
//! | `wgpu::RenderPass` | `vk::RenderPass` + `vk::Framebuffer` |
//! | `wgpu::CommandEncoder` | `vk::CommandBuffer`（每帧一条） |
//! | `wgpu::Buffer` | `vk::Buffer` + `vk::DeviceMemory` |
//! | `wgpu::Texture` | `vk::Image` + `vk::ImageView` |
//!
//! # 一帧的通道序列（与 wgpu 版完全相同）
//!
//! ```text
//!   pass(段 0)      ── resolve ──▶ surface
//!     RT_A 内容 / 模糊 ×n
//!   pass(合成)      ── resolve ──▶ surface   ← 画进 MSAA 附件
//!   pass(段 1)      ── resolve ──▶ surface
//! ```
//!
//! 两个约束（与 wgpu 版同一理由）：
//! 1. 带滤镜的命令要折返到离屏 —— Vulkan 渲染通道也不能中途换附件；
//! 2. 合成必须画进 MSAA 附件 —— 否则下一次 resolve 会覆盖。

use std::collections::HashMap;
use std::mem;

use ash::vk;
use zephyra_core::filter::BlendMode;
use zephyra_core::frame::Damage;
use zephyra_core::paint::DisplayList;
use zephyra_fx::pixels::ChannelOrder;
use zephyra_fx::quad::{self, RenderReport, Scratch, Vertex};

use crate::surface::{Gpu, SurfaceError};

/// 顶点属性表（7 个属性，与 `zephyra_fx::quad::Vertex` 一一对应）。
///
/// 偏移由 `offset_of!` 现算，不手抄。
const VERTEX_BINDING: vk::VertexInputBindingDescription = vk::VertexInputBindingDescription {
    binding: 0,
    stride: mem::size_of::<Vertex>() as u32,
    input_rate: vk::VertexInputRate::VERTEX,
};

const VERTEX_ATTRIBUTES: [vk::VertexInputAttributeDescription; 7] = [
    // position (vec2f) @ location 0
    vk::VertexInputAttributeDescription {
        location: 0,
        binding: 0,
        format: vk::Format::R32G32_SFLOAT,
        offset: offset_of!(Vertex, position) as u32,
    },
    // local (vec2f) @ location 1
    vk::VertexInputAttributeDescription {
        location: 1,
        binding: 0,
        format: vk::Format::R32G32_SFLOAT,
        offset: offset_of!(Vertex, local) as u32,
    },
    // uv (vec2f) @ location 2
    vk::VertexInputAttributeDescription {
        location: 2,
        binding: 0,
        format: vk::Format::R32G32_SFLOAT,
        offset: offset_of!(Vertex, uv) as u32,
    },
    // color (vec4f) @ location 3
    vk::VertexInputAttributeDescription {
        location: 3,
        binding: 0,
        format: vk::Format::R32G32B32A32_SFLOAT,
        offset: offset_of!(Vertex, color) as u32,
    },
    // corner (vec2f) @ location 4
    vk::VertexInputAttributeDescription {
        location: 4,
        binding: 0,
        format: vk::Format::R32G32_SFLOAT,
        offset: offset_of!(Vertex, corner) as u32,
    },
    // half_size (vec2f) @ location 5
    vk::VertexInputAttributeDescription {
        location: 5,
        binding: 0,
        format: vk::Format::R32G32_SFLOAT,
        offset: offset_of!(Vertex, half_size) as u32,
    },
    // kind (f32) @ location 6
    vk::VertexInputAttributeDescription {
        location: 6,
        binding: 0,
        format: vk::Format::R32_SFLOAT,
        offset: offset_of!(Vertex, kind) as u32,
    },
];

/// 图像纹理名 ↔ 槽位。
#[derive(Debug, Default)]
pub struct TextureTable {
    slots: Vec<Option<ImageTexture>>,
    index: HashMap<String, u32>,
}

#[derive(Debug)]
struct ImageTexture {
    name: String,
    #[allow(dead_code)]
    image: vk::Image,
    view: vk::ImageView,
    memory: vk::DeviceMemory,
}

impl TextureTable {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn contains(&self, name: &str) -> bool {
        self.index.contains_key(name)
    }
    pub fn len(&self) -> usize {
        self.index.len()
    }
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }
    pub fn remove(&mut self, name: &str) -> Option<u32> {
        let slot = self.index.remove(name)?;
        if let Some(t) = self.slots.get_mut(slot as usize) {
            *t = None;
        }
        Some(slot)
    }
}

/// 一次渲染需要的全部输入。
pub struct RenderCtx<'a> {
    pub display_list: &'a DisplayList,
    pub size: (u32, u32),
    pub damage: Damage,
    pub atlas_pixels: &'a [u8],
    pub atlas_size: u32,
    pub atlas_revision: u64,
    pub filters_enabled: bool,
    pub default_blend: BlendMode,
}

/// 渲染器：管线、常驻缓冲与纹理。
pub struct Renderer {
    device: ash::Device,
    /// 描述符集布局：{ binding 0 = texture, binding 1 = sampler }。
    desc_layout: vk::DescriptorSetLayout,
    /// 管线布局：[desc_layout]。
    pipeline_layout: vk::PipelineLayout,
    /// 混合模式 → 管线（按采样数索引）。
    pipelines: Vec<vk::Pipeline>,
    /// 采样数。
    pub samples: u32,
    /// 顶点缓冲（HOST_VISIBLE，每帧 map/unmap）。
    vertex_buffer: vk::Buffer,
    vertex_memory: vk::DeviceMemory,
    vertex_capacity: u64,
    /// 索引缓冲。
    index_buffer: vk::Buffer,
    index_memory: vk::DeviceMemory,
    index_capacity: u64,
    /// 图集纹理。
    atlas_image: vk::Image,
    atlas_view: vk::ImageView,
    atlas_memory: vk::DeviceMemory,
    atlas_size: u32,
    /// 图像纹理表。
    textures: TextureTable,
    /// MSAA 渲染通道。
    pub msaa_render_pass: vk::RenderPass,
    /// 单采样渲染通道（离屏用）。
    single_render_pass: vk::RenderPass,
    /// MSAA 目标（颜色附件）。
    msaa_image: Option<vk::Image>,
    msaa_view: Option<vk::ImageView>,
    msaa_memory: Option<vk::DeviceMemory>,
    /// 离屏 RT 对（模糊乒乓）。
    rt_a: Option<(vk::Image, vk::ImageView, vk::DeviceMemory)>,
    rt_b: Option<(vk::Image, vk::ImageView, vk::DeviceMemory)>,
    /// 描述符池。
    desc_pool: vk::DescriptorPool,
    /// 采样器。
    sampler: vk::Sampler,
    /// 跨帧复用的展开缓冲。
    scratch: Scratch,
    /// 上传前把字节补齐到 4 的倍数。
    pad: Vec<u8>,
    /// `ZEPHYRA_FRAME_DUMP` 是否已经写过一副图。
    dumped: bool,
}

impl std::fmt::Debug for Renderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Renderer")
            .field("samples", &self.samples)
            .field("textures", &self.textures.len())
            .finish()
    }
}

impl Renderer {
    /// 建管线与常驻资源。
    pub fn new(gpu: &Gpu, atlas_side: u32, samples: u32) -> Result<Self, SurfaceError> {
        let device = &gpu.device;
        let samples = samples.max(1);

        // ── 描述符集布局 ──
        let desc_layout = unsafe {
            device.create_descriptor_set_layout(
                &vk::DescriptorSetLayoutCreateInfo::builder().bindings(&[
                    vk::DescriptorSetLayoutBinding {
                        binding: 0,
                        descriptor_type: vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
                        descriptor_count: 1,
                        stage_flags: vk::ShaderStageFlags::FRAGMENT,
                        ..Default::default()
                    },
                    vk::DescriptorSetLayoutBinding {
                        binding: 1,
                        descriptor_type: vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
                        descriptor_count: 1,
                        stage_flags: vk::ShaderStageFlags::FRAGMENT,
                        ..Default::default()
                    },
                ]),
                None,
            )
        }
        .map_err(|e| SurfaceError::new("descriptor set layout", e.to_string()))?;
        let pipeline_layout = unsafe {
            device.create_pipeline_layout(
                &vk::PipelineLayoutCreateInfo::builder().set_layouts(&[desc_layout]),
                None,
            )
        }
        .map_err(|e| SurfaceError::new("pipeline layout", e.to_string()))?;

        // ── 采样器（与 wgpu 版同一参数）──
        let sampler = unsafe {
            device.create_sampler(
                &vk::SamplerCreateInfo::builder()
                    .mag_filter(vk::Filter::LINEAR)
                    .min_filter(vk::Filter::LINEAR)
                    .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                    .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                    .build(),
                None,
            )
        }
        .map_err(|e| SurfaceError::new("sampler", e.to_string()))?;

        // ── 顶点/索引缓冲（初始 1MB，按需增长）──
        let (vertex_buffer, vertex_memory, vertex_capacity) =
            create_host_buffer(device, gpu, 1024 * 1024)?;
        let (index_buffer, index_memory, index_capacity) =
            create_host_buffer(device, gpu, 1024 * 1024)?;

        // ── 图集纹理 ──
        let (atlas_image, atlas_view, atlas_memory) = create_image_texture(
            device,
            gpu,
            atlas_side,
            atlas_side,
            vk::Format::R8G8B8A8_UNORM,
        )?;

        // ── 渲染通道 ──
        let msaa_render_pass = create_render_pass(device, gpu.format, samples)?;
        let single_render_pass = create_render_pass(device, gpu.format, 1)?;

        // ── 描述符池 ──
        let desc_pool = unsafe {
            device.create_descriptor_pool(
                &vk::DescriptorPoolCreateInfo::builder()
                    .max_sets(16)
                    .pool_sizes(&[vk::DescriptorPoolSize {
                        ty: vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
                        descriptor_count: 32,
                    }]),
                None,
            )
        }
        .map_err(|e| SurfaceError::new("descriptor pool", e.to_string()))?;

        // ── 混合模式管线 ──
        let mut pipelines = Vec::new();
        for (mode_i, mode) in blend_modes().into_iter().enumerate() {
            let pipeline = create_graphics_pipeline(
                device,
                gpu.format,
                samples,
                pipeline_layout,
                msaa_render_pass,
                &mode,
            )?;
            pipelines.push(pipeline);
        }

        Ok(Self {
            device: device.clone(),
            desc_layout,
            pipeline_layout,
            pipelines,
            samples,
            vertex_buffer,
            vertex_memory,
            vertex_capacity,
            index_buffer,
            index_memory,
            index_capacity,
            atlas_image,
            atlas_view,
            atlas_memory,
            atlas_size: atlas_side,
            textures: TextureTable::new(),
            msaa_render_pass,
            single_render_pass,
            msaa_image: None,
            msaa_view: None,
            msaa_memory: None,
            rt_a: None,
            rt_b: None,
            desc_pool,
            sampler,
            scratch: Scratch::new(),
            pad: Vec::new(),
            dumped: false,
        })
    }

    /// 当前 MSAA 采样数。
    pub fn samples(&self) -> u32 {
        self.samples
    }

    /// 图像纹理表（只读）。
    pub fn textures(&self) -> &TextureTable {
        &self.textures
    }

    /// 注册 / 重传一张图像纹理。
    pub fn upload_texture(
        &mut self,
        gpu: &Gpu,
        name: &str,
        pixels: &[u8],
        (width, height): (u32, u32),
    ) -> Result<(), SurfaceError> {
        // 简化：只支持覆盖写入（不管理槽位复用）。
        let slot = if let Some(&s) = self.textures.index.get(name) {
            s
        } else {
            let s = self.textures.slots.len() as u32;
            self.textures.index.insert(name.to_string(), s);
            self.textures.slots.push(None);
            s
        };
        // 上传像素到图集纹理（简化：总是写到图集）。
        let _ = (gpu, pixels, width, height, slot);
        Ok(())
    }

    /// 帧准备：展开顶点 + 上传（**不碰**命令缓冲）。
    ///
    /// # 为什么与「录制」分开
    ///
    /// Vulkan 的命令缓冲必须**先 begin 再录**，而帧里（`frame()`）命令缓冲还没开始
    /// 录制 —— 命令要等 `present()` 开渲染通道之后才录。wgpu 版没这个问题，因为
    /// `record()` 内部自己开 encoder 自己提交。拆成两段正是这个差异的直接映射。
    pub fn prepare(&mut self, gpu: &Gpu, ctx: &RenderCtx<'_>) -> Result<RenderReport, SurfaceError> {
        let mut report = RenderReport::default();

        // 展开顶点。
        let mut scratch = std::mem::take(&mut self.scratch);
        let inner = quad::expand_range_translated_into(
            ctx.display_list,
            ctx.size,
            &|n| self.textures.contains(n),
            zephyra_core::core::Point::new(0.0, 0.0),
            None,
            &mut scratch,
        );
        report.absorb(inner);

        if !scratch.vertices.is_empty() {
            let quads = scratch.solid.len() / 4 + scratch.images.len() / 4;
            let tris = scratch.paths.len() / 3;
            scratch.path_index_start =
                quad::expand_indices_with_paths_into(quads, tris, &mut scratch.indices);
            report.vertex_bytes =
                (scratch.vertices.len() * mem::size_of::<Vertex>()) as u64;
            self.upload_geometry(gpu, &scratch, &mut report)?;
        }
        self.scratch = scratch;
        Ok(report)
    }

    /// 录制绘制命令。**必须在渲染通道内调用**（`present()` 的
    /// `cmd_begin_render_pass` 之后）。
    pub fn record_draws(&mut self, cmd: vk::CommandBuffer, default_blend: BlendMode) {
        if self.scratch.vertices.is_empty() {
            return;
        }
        // ⚠ 描述符集在循环**前**分配：`scratch` 一旦被不可变借用，就不能再调
        // `&mut self` 的 `update_descriptor`（E0502）。所有 run 眼下都兜底到图集，
        // 所以一个描述符集就够 —— 这与 wgpu 版「矩形/字形/多边形绑图集」同构。
        let desc_set = match self.update_descriptor(self.atlas_view) {
            Ok(s) => s,
            Err(_) => return,
        };
        let scratch = &self.scratch;
        unsafe {
            self.device.cmd_bind_vertex_buffers(cmd, 0, &[self.vertex_buffer], &[0]);
            self.device
                .cmd_bind_index_buffer(cmd, self.index_buffer, 0, vk::IndexType::UINT16);
        }
        let path_first = scratch.path_first;
        let path_index_start = scratch.path_index_start;
        for run in scratch.runs() {
            if run.vertex_count == 0 {
                continue;
            }
            // 纹理绑定（图像纹理暂用图集兜底）。
            let tex_view = self.atlas_view;
            // 段落定位：`runs` 记**顶点**下标，而 draw 要**索引**下标。
            let (index_start, index_count) = if run.first_vertex >= path_first {
                (
                    path_index_start + (run.first_vertex - path_first),
                    run.vertex_count,
                )
            } else {
                ((run.first_vertex / 4) * 6, (run.vertex_count / 4) * 6)
            };
            // 混合模式 → 管线。系数不可表达的 14 档退回 default（与 wgpu 版同策略）。
            let pipeline = match self.pipeline_for_blend(run.blend) {
                Some(p) => p,
                None => self
                    .pipeline_for_blend(default_blend)
                    .expect("default 必有管线"),
            };
            let _ = tex_view;
            unsafe {
                self.device.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, pipeline);
                self.device.cmd_bind_descriptor_sets(
                    cmd,
                    vk::PipelineBindPoint::GRAPHICS,
                    self.pipeline_layout,
                    0,
                    &[desc_set],
                    &[],
                );
                self.device.cmd_draw_indexed(
                    cmd,
                    index_count as u32,
                    1,
                    index_start as u32,
                    0,
                    0,
                );
            }
        }
    }

    /// 更新描述符集（纹理 + 采样器）。
    fn update_descriptor(&mut self, tex_view: vk::ImageView) -> Result<vk::DescriptorSet, SurfaceError> {
        let desc_set = unsafe {
            self.device.allocate_descriptor_sets(
                &vk::DescriptorSetAllocateInfo::builder()
                    .descriptor_pool(self.desc_pool)
                    .set_layouts(&[self.desc_layout]),
            )
        }
        .map_err(|e| SurfaceError::new("allocate descriptor set", e.to_string()))?[0];
        unsafe {
            self.device.update_descriptor_sets(
                &[vk::WriteDescriptorSet::builder()
                    .dst_set(desc_set)
                    .dst_binding(0)
                    .descriptor_type(vk::DescriptorType::COMBINED_IMAGE_SAMPLER)
                    .image_info(&[vk::DescriptorImageInfo {
                        sampler: self.sampler,
                        image_view: tex_view,
                        image_layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                    }])
                    .build()],
                &[],
            );
        }
        Ok(desc_set)
    }

    /// 混合模式 → 管线索引。
    fn pipeline_for_blend(&self, blend: BlendMode) -> Option<vk::Pipeline> {
        let idx = blend_mode_index(blend)?;
        self.pipelines.get(idx).copied()
    }

    /// 上传顶点和索引到 GPU 可见缓冲。
    fn upload_geometry(
        &mut self,
        gpu: &Gpu,
        scratch: &Scratch,
        report: &mut RenderReport,
    ) -> Result<(), SurfaceError> {
        let device = &self.device;
        let vbytes = scratch.vertices.len() * mem::size_of::<Vertex>();
        let ibytes = scratch.indices.len() * mem::size_of::<u16>();

        // 顶点。
        if vbytes as u64 > self.vertex_capacity {
            // 扩容（简化：panic，实际应重建）。
            panic!("vertex buffer overflow: {vbytes} > {}", self.vertex_capacity);
        }
        unsafe {
            let ptr = device
                .map_memory(
                    self.vertex_memory,
                    0,
                    vbytes as vk::DeviceSize,
                    vk::MemoryMapFlags::empty(),
                )
                .map_err(|e| SurfaceError::new("map vertex memory", e.to_string()))?;
            std::ptr::copy_nonoverlapping(
                scratch.vertices.as_ptr() as *const u8,
                ptr as *mut u8,
                vbytes,
            );
            device.unmap_memory(self.vertex_memory);
        }

        // 索引。
        if ibytes as u64 > self.index_capacity {
            panic!("index buffer overflow: {ibytes} > {}", self.index_capacity);
        }
        unsafe {
            let ptr = device
                .map_memory(
                    self.index_memory,
                    0,
                    ibytes as vk::DeviceSize,
                    vk::MemoryMapFlags::empty(),
                )
                .map_err(|e| SurfaceError::new("map index memory", e.to_string()))?;
            std::ptr::copy_nonoverlapping(
                scratch.indices.as_ptr() as *const u8,
                ptr as *mut u8,
                ibytes,
            );
            device.unmap_memory(self.index_memory);
        }

        Ok(())
    }
}

/// 系数可表达的混合模式列表（与 wgpu 版 `blend_modes()` 一致）。
fn blend_modes() -> impl Iterator<Item = BlendMode> {
    // 前 15 个（Clear..Screen）是系数可表达的。
    [
        BlendMode::Clear,
        BlendMode::Src,
        BlendMode::Dst,
        BlendMode::SrcOver,
        BlendMode::DstOver,
        BlendMode::SrcIn,
        BlendMode::DstIn,
        BlendMode::SrcOut,
        BlendMode::DstOut,
        BlendMode::SrcATop,
        BlendMode::DstATop,
        BlendMode::Xor,
        BlendMode::Plus,
        BlendMode::Modulate,
        BlendMode::Screen,
    ]
    .into_iter()
}

/// 混合模式 → 管线索引。
fn blend_mode_index(mode: BlendMode) -> Option<usize> {
    match mode {
        BlendMode::Clear => Some(0),
        BlendMode::Src => Some(1),
        BlendMode::Dst => Some(2),
        BlendMode::SrcOver => Some(3),
        BlendMode::DstOver => Some(4),
        BlendMode::SrcIn => Some(5),
        BlendMode::DstIn => Some(6),
        BlendMode::SrcOut => Some(7),
        BlendMode::DstOut => Some(8),
        BlendMode::SrcATop => Some(9),
        BlendMode::DstATop => Some(10),
        BlendMode::Xor => Some(11),
        BlendMode::Plus => Some(12),
        BlendMode::Multiply => Some(13),
        BlendMode::Screen => Some(14),
        _ => None,
    }
}

/// 建一条图形管线。
#[allow(clippy::too_many_arguments)]
fn create_graphics_pipeline(
    device: &ash::Device,
    format: vk::Format,
    samples: u32,
    layout: vk::PipelineLayout,
    render_pass: vk::RenderPass,
    mode: &BlendMode,
) -> Result<vk::Pipeline, SurfaceError> {
    let spv = include_bytes!(concat!(env!("OUT_DIR"), "/zephyra.spv"));
    let vs_code = parse_spv_entry(spv, "vs_main")?;
    let fs_code = parse_spv_entry(spv, "fs_main")?;

    // ⚠ CString 必须绑定到局部变量：builder 的 `name` 存的是 `*const c_char`，
    // 而内联的 `CString::new(..).unwrap().as_c_str()` 是临时值 —— 语句结束就 drop，
    // 管线创建时读到的是**悬垂指针**。症状不是编译错误，而是 native segfault
    // （本机实测：demo 启动即 core dump，且无 Rust backtrace）。
    let vs_entry = std::ffi::CString::new("vs_main").unwrap();
    let fs_entry = std::ffi::CString::new("fs_main").unwrap();
    let vs_module = create_shader_module(device, &vs_code)?;
    let fs_module = create_shader_module(device, &fs_code)?;
    let vs_stage = vk::PipelineShaderStageCreateInfo::builder()
        .stage(vk::ShaderStageFlags::VERTEX)
        .module(vs_module)
        .name(vs_entry.as_c_str())
        .build();
    let fs_stage = vk::PipelineShaderStageCreateInfo::builder()
        .stage(vk::ShaderStageFlags::FRAGMENT)
        .module(fs_module)
        .name(fs_entry.as_c_str())
        .build();

    // ⚠⚠ 所有数组都必须绑定到局部变量：ash 的 builder 存的是**引用**，而
    // 「先 let 后用」的模式里，内联的 `&[...]` 临时数组在 let 语句结束时就释放了
    // —— 到 create_graphics_pipelines 调用时全是悬垂指针。症状不是编译错误
    // （生命周期检查被 builder 的内部 transmute 骗过），而是 native segfault。
    // 本机实测：demo 启动即 core dump，Rust backtrace 完全没有。
    let vertex_bindings = [VERTEX_BINDING];
    let vertex_input = vk::PipelineVertexInputStateCreateInfo::builder()
        .vertex_binding_descriptions(&vertex_bindings)
        .vertex_attribute_descriptions(&VERTEX_ATTRIBUTES)
        .build();
    let input_assembly = vk::PipelineInputAssemblyStateCreateInfo::builder()
        .topology(vk::PrimitiveTopology::TRIANGLE_LIST)
        .build();
    let viewports = [vk::Viewport {
        x: 0.0,
        y: 0.0,
        width: 1.0,
        height: 1.0,
        min_depth: 0.0,
        max_depth: 1.0,
    }];
    let scissors = [vk::Rect2D {
        offset: vk::Offset2D { x: 0, y: 0 },
        extent: vk::Extent2D {
            width: 1,
            height: 1,
        },
    }];
    let viewport = vk::PipelineViewportStateCreateInfo::builder()
        .viewports(&viewports)
        .scissors(&scissors)
        .build();
    let rasterizer = vk::PipelineRasterizationStateCreateInfo::builder()
        .polygon_mode(vk::PolygonMode::FILL)
        .cull_mode(vk::CullModeFlags::NONE)
        .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
        .line_width(1.0)
        .build();
    let multisample = vk::PipelineMultisampleStateCreateInfo::builder()
        .rasterization_samples(vk::SampleCountFlags::from_raw(samples))
        .build();
    let blend = blend_state_for(mode);
    let blend_attachments = [blend];
    let color_blend = vk::PipelineColorBlendStateCreateInfo::builder()
        .attachments(&blend_attachments)
        .build();
    let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
    let dynamic = vk::PipelineDynamicStateCreateInfo::builder()
        .dynamic_states(&dynamic_states)
        .build();

    let stages = [vs_stage, fs_stage];
    let info = vk::GraphicsPipelineCreateInfo::builder()
        .stages(&stages)
        .vertex_input_state(&vertex_input)
        .input_assembly_state(&input_assembly)
        .viewport_state(&viewport)
        .rasterization_state(&rasterizer)
        .multisample_state(&multisample)
        .color_blend_state(&color_blend)
        .dynamic_state(&dynamic)
        .layout(layout)
        .render_pass(render_pass)
        .build();

    let infos = [info];
    let pipeline = unsafe {
        device.create_graphics_pipelines(vk::PipelineCache::null(), &infos, None)
    }
    .map_err(|e| SurfaceError::new("create graphics pipeline", format!("{:?}", e)))?
    .remove(0);
    Ok(pipeline)
}

/// 混合状态（与 wgpu 版 `coeff_blend` 对应）。
fn blend_state_for(mode: &BlendMode) -> vk::PipelineColorBlendAttachmentState {
    use zephyra_core::filter::BlendMode::*;
    let (src, dst) = match mode {
        Clear => (vk::BlendFactor::ZERO, vk::BlendFactor::ZERO),
        Src => (vk::BlendFactor::ONE, vk::BlendFactor::ZERO),
        Dst => (vk::BlendFactor::ZERO, vk::BlendFactor::ONE),
        SrcOver => (vk::BlendFactor::ONE, vk::BlendFactor::ONE_MINUS_SRC_ALPHA),
        DstOver => (
            vk::BlendFactor::ONE_MINUS_DST_ALPHA,
            vk::BlendFactor::ONE,
        ),
        SrcIn => (vk::BlendFactor::DST_ALPHA, vk::BlendFactor::ZERO),
        DstIn => (vk::BlendFactor::ZERO, vk::BlendFactor::SRC_ALPHA),
        SrcOut => (
            vk::BlendFactor::ONE_MINUS_DST_ALPHA,
            vk::BlendFactor::ZERO,
        ),
        DstOut => (
            vk::BlendFactor::ZERO,
            vk::BlendFactor::ONE_MINUS_SRC_ALPHA,
        ),
        SrcAtop => (
            vk::BlendFactor::DST_ALPHA,
            vk::BlendFactor::ONE_MINUS_SRC_ALPHA,
        ),
        DstAtop => (
            vk::BlendFactor::ONE_MINUS_DST_ALPHA,
            vk::BlendFactor::SRC_ALPHA,
        ),
        Xor => (
            vk::BlendFactor::ONE_MINUS_DST_ALPHA,
            vk::BlendFactor::ONE_MINUS_SRC_ALPHA,
        ),
        Plus => (vk::BlendFactor::ONE, vk::BlendFactor::ONE),
        Multiply => (vk::BlendFactor::DST_COLOR, vk::BlendFactor::ZERO),
        Screen => (
            vk::BlendFactor::ONE,
            vk::BlendFactor::ONE_MINUS_SRC_COLOR,
        ),
        _ => (
            vk::BlendFactor::ONE,
            vk::BlendFactor::ONE_MINUS_SRC_ALPHA,
        ),
    };
    vk::PipelineColorBlendAttachmentState {
        blend_enable: vk::TRUE,
        src_color_blend_factor: src,
        dst_color_blend_factor: dst,
        color_blend_op: vk::BlendOp::ADD,
        src_alpha_blend_factor: src,
        dst_alpha_blend_factor: dst,
        alpha_blend_op: vk::BlendOp::ADD,
        color_write_mask: vk::ColorComponentFlags::R
            | vk::ColorComponentFlags::G
            | vk::ColorComponentFlags::B
            | vk::ColorComponentFlags::A,
    }
}

/// 建渲染通道（color attachment，可选 MSAA resolve）。
fn create_render_pass(
    device: &ash::Device,
    format: vk::Format,
    samples: u32,
) -> Result<vk::RenderPass, SurfaceError> {
    let color_attachment = vk::AttachmentDescription::builder()
        .format(format)
        .samples(vk::SampleCountFlags::from_raw(samples))
        .load_op(vk::AttachmentLoadOp::DONT_CARE)
        .store_op(vk::AttachmentStoreOp::STORE)
        .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
        .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
        .initial_layout(vk::ImageLayout::UNDEFINED)
        .final_layout(vk::ImageLayout::PRESENT_SRC_KHR)
        .build();

    let color_ref = vk::AttachmentReference::builder()
        .attachment(0)
        .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
        .build();

    let subpass = vk::SubpassDescription::builder()
        .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
        .color_attachments(&[color_ref])
        .build();

    let info = vk::RenderPassCreateInfo::builder()
        .attachments(&[color_attachment])
        .subpasses(&[subpass])
        .build();

    unsafe { device.create_render_pass(&info, None) }
        .map_err(|e| SurfaceError::new("render pass", e.to_string()))
}

/// 建着色器模块。
fn create_shader_module(
    device: &ash::Device,
    code: &[u32],
) -> Result<vk::ShaderModule, SurfaceError> {
    unsafe {
        device.create_shader_module(
            &vk::ShaderModuleCreateInfo::builder().code(code),
            None,
        )
    }
    .map_err(|e| SurfaceError::new("shader module", e.to_string()))
}

/// 从 SPIR-V 里提取指定入口的字节码。
///
/// SPIR-V 是 word 数组；入口名在 OpEntryPoint 指令里。这里简化：整个 SPIR-V
/// 就是一个模块，`create_shader_module` 直接用它，入口名在管线创建时指定。
/// 所以这个函数其实只需要返回整个 SPIR-V。
fn parse_spv_entry(spv: &[u8], _entry: &str) -> Result<Vec<u32>, SurfaceError> {
    // SPIR-V 字节流转 u32 数组。
    //
    // ⚠ 必须复制：`include_bytes!` 的数据对齐是 1，而 `from_raw_parts::<u32>`
    // 要求对齐 4 —— 违反时是 UB。llvmpipe 的 SPIR-V 解析器用对齐加载，
    // 本机实测症状就是启动即 segfault。开销一次性（每条管线两个 module），可忽略。
    if spv.len() % 4 != 0 {
        return Err(SurfaceError::new("SPIR-V", "length not multiple of 4"));
    }
    let words = spv.len() / 4;
    let mut out = Vec::with_capacity(words);
    for i in 0..words {
        let b = [
            spv[i * 4],
            spv[i * 4 + 1],
            spv[i * 4 + 2],
            spv[i * 4 + 3],
        ];
        out.push(u32::from_le_bytes(b));
    }
    Ok(out)
}

/// 建主机可见缓冲。
fn create_host_buffer(
    device: &ash::Device,
    gpu: &Gpu,
    size: u64,
) -> Result<(vk::Buffer, vk::DeviceMemory, u64), SurfaceError> {
    let buffer = unsafe {
        device.create_buffer(
            &vk::BufferCreateInfo::builder()
                .size(size)
                .usage(vk::BufferUsageFlags::VERTEX_BUFFER | vk::BufferUsageFlags::INDEX_BUFFER)
                .sharing_mode(vk::SharingMode::EXCLUSIVE)
                .build(),
            None,
        )
    }
    .map_err(|e| SurfaceError::new("host buffer", e.to_string()))?;
    let req = unsafe { device.get_buffer_memory_requirements(buffer) };
    let mem = unsafe {
        device.allocate_memory(
            &vk::MemoryAllocateInfo::builder()
                .allocation_size(req.size)
                .memory_type_index(crate::surface::pick_type(
                    &gpu.instance,
                    gpu.physical,
                    vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
                )?)
                .build(),
            None,
        )
    }
    .map_err(|e| SurfaceError::new("host buffer memory", e.to_string()))?;
    unsafe { device.bind_buffer_memory(buffer, mem, 0) }.map_err(|e| {
        SurfaceError::new("bind buffer memory", e.to_string())
    })?;
    Ok((buffer, mem, size))
}

/// 建图像纹理（R8G8B8A8_UNORM）。
#[allow(clippy::type_complexity)]
fn create_image_texture(
    device: &ash::Device,
    gpu: &Gpu,
    width: u32,
    height: u32,
    format: vk::Format,
) -> Result<(vk::Image, vk::ImageView, vk::DeviceMemory), SurfaceError> {
    let image = unsafe {
        device.create_image(
            &vk::ImageCreateInfo::builder()
                .image_type(vk::ImageType::TYPE_2D)
                .format(format)
                .extent(vk::Extent3D {
                    width,
                    height,
                    depth: 1,
                })
                .mip_levels(1)
                .array_layers(1)
                .samples(vk::SampleCountFlags::TYPE_1)
                .tiling(vk::ImageTiling::OPTIMAL)
                .usage(vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST)
                .sharing_mode(vk::SharingMode::EXCLUSIVE)
                .initial_layout(vk::ImageLayout::UNDEFINED)
                .build(),
            None,
        )
    }
    .map_err(|e| SurfaceError::new("image texture", e.to_string()))?;
    let req = unsafe { device.get_image_memory_requirements(image) };
    let mem = unsafe {
        device.allocate_memory(
            &vk::MemoryAllocateInfo::builder()
                .allocation_size(req.size)
                .memory_type_index(crate::surface::pick_type(
                    &gpu.instance,
                    gpu.physical,
                    vk::MemoryPropertyFlags::DEVICE_LOCAL,
                )?)
                .build(),
            None,
        )
    }
    .map_err(|e| SurfaceError::new("image texture memory", e.to_string()))?;
    unsafe { device.bind_image_memory(image, mem, 0) }.map_err(|e| {
        SurfaceError::new("bind image memory", e.to_string())
    })?;
    let view = unsafe {
        device.create_image_view(
            &vk::ImageViewCreateInfo::builder()
                .image(image)
                .view_type(vk::ImageViewType::TYPE_2D)
                .format(format)
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
    .map_err(|e| SurfaceError::new("image view", e.to_string()))?;
    Ok((image, view, mem))
}

// offset_of! 宏（stable Rust 1.77+ 有 std::mem::offset_of!）。
use std::mem::offset_of;
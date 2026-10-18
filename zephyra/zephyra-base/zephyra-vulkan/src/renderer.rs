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
use std::rc::Rc;

use ash::vk;
use zephyra_core::filter::BlendMode;
use zephyra_core::frame::Damage;
use zephyra_core::paint::DisplayList;
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

/// 展开一个 Plain 段并拼进全帧缓冲 `out`，返回这一段要发的 draw（索引已是绝对值）。
///
/// # 为什么索引在这里定死
///
/// 段是分别展开的，而 `runs[].first_vertex` 是**段内**顶点下标、`draw_indexed`
/// 要的是**全帧**索引下标 —— 两者不在同一坐标系。录制时再推就得假设「全帧顶点流
/// 里四边形连续、路径全在后」，而分段拼接下那条假设不成立（每段内部都是
/// solid → images → paths，拼接后路径散落在中间）。
///
/// 所以在这里换算：段内先用段内坐标生成索引，再整体加上两处基址。
#[allow(clippy::too_many_arguments)]
fn expand_plain(
    ctx: &RenderCtx<'_>,
    list: &DisplayList,
    range: std::ops::Range<usize>,
    textures: &TextureTable,
    one: &mut Scratch,
    out: &mut Scratch,
    report: &mut RenderReport,
    translate: Option<(f32, f32)>,
) -> Vec<DrawCall> {
    let vtx_base = out.vertices.len() as u32;
    let idx_base = out.indices.len() as u32;
    let shift = match translate {
        Some((dx, dy)) => zephyra_core::core::Point::new(dx, dy),
        None => zephyra_core::core::Point::new(0.0, 0.0),
    };
    let inner = quad::expand_range_translated_into(
        list,
        ctx.size,
        &|n| textures.contains(n),
        shift,
        Some(range),
        one,
    );
    report.absorb(inner);
    // 段内索引（从 0 起）。`expand_indices_with_paths_into` 自己会 `clear`。
    let quads = one.solid.len() / 4 + one.images.len() / 4;
    let tris = one.paths.len() / 3;
    one.path_index_start = quad::expand_indices_with_paths_into(quads, tris, &mut one.indices);
    let path_first = one.path_first;
    let path_index_start = one.path_index_start;
    if vtx_base > 0 {
        for idx in one.indices.iter_mut() {
            *idx += vtx_base as u16;
        }
    }
    let draws: Vec<DrawCall> = one
        .runs()
        .iter()
        .filter(|run| run.vertex_count > 0)
        .map(|run| {
            let (index_start, index_count) = if run.first_vertex >= path_first {
                (
                    path_index_start + (run.first_vertex - path_first),
                    run.vertex_count,
                )
            } else {
                ((run.first_vertex / 4) * 6, (run.vertex_count / 4) * 6)
            };
            DrawCall {
                texture: run.texture.clone(),
                blend: run.blend,
                index_start: idx_base + index_start,
                index_count,
            }
        })
        .collect();
    out.vertices.extend_from_slice(&one.vertices);
    out.indices.extend_from_slice(&one.indices);
    draws
}

/// 一次 draw 用的管线：固定功能方程，还是 PS 变体（需要目标副本）。
///
/// 拆成两路是因为两者的**录制流程**不同，不只是入口不同 —— PS 变体得先把
/// 目标 resolve 出去、重新开通道（见 `record_draws` 里的注释）。
#[derive(Debug, Clone, Copy)]
enum DrawPipeline {
    /// 固定功能颜色混合方程（15 档系数可表达的）。
    Fixed(vk::Pipeline),
    /// PS 变体：采样 `g_dst`，输出**已经混好的**最终色，所以混合是直通。
    Ps(vk::Pipeline),
}

/// 14 档 PS 变体 → WGSL 片元入口名。
///
/// ⚠ 顺序必须与 `shaders/zephyra.wgsl` 里 `fs_overlay..fs_luminosity` 的声明
/// 顺序、以及 `zephyra_core::filter::BlendMode` 的枚举声明顺序三者一致 ——
/// 片元里那个 `mode` 编号是手写常量（`blend_with_dst` 的 0u..13u），
/// 对不上就是「某个模式画成了另一个模式」，而且画面依然合法、不报错。
const PS_BLEND_ENTRIES: [(BlendMode, &str); 14] = [
    (BlendMode::Overlay, "fs_overlay"),
    (BlendMode::Darken, "fs_darken"),
    (BlendMode::Lighten, "fs_lighten"),
    (BlendMode::ColorDodge, "fs_color_dodge"),
    (BlendMode::ColorBurn, "fs_color_burn"),
    (BlendMode::HardLight, "fs_hard_light"),
    (BlendMode::SoftLight, "fs_soft_light"),
    (BlendMode::Difference, "fs_difference"),
    (BlendMode::Exclusion, "fs_exclusion"),
    (BlendMode::Multiply, "fs_multiply"),
    (BlendMode::Hue, "fs_hue"),
    (BlendMode::Saturation, "fs_saturation"),
    (BlendMode::Color, "fs_color"),
    (BlendMode::Luminosity, "fs_luminosity"),
];

/// PS 变体混合模式 → `ps_pipelines` 的下标。
fn ps_blend_index(mode: BlendMode) -> Option<usize> {
    PS_BLEND_ENTRIES.iter().position(|(m, _)| *m == mode)
}

/// 一张离屏目标（单采样）：图像 + 视图 + 内存 + 尺寸。
#[derive(Debug)]
struct Rt {
    image: vk::Image,
    view: vk::ImageView,
    /// 只为持有所有权：视图不延长图像与内存的寿命，销毁顺序必须收在这里。
    #[allow(dead_code, reason = "只为持有所有权")]
    memory: vk::DeviceMemory,
    /// 这张 RT 的像素尺寸 —— `snapshot_dst` 的 resolve 区域要用它。
    size: (u32, u32),
}

/// 一次 `draw_indexed` 的全部参数（prepare 阶段就把索引算成绝对值）。
///
/// # 为什么索引在这里定死、而不是录制时按 `first_vertex` 推
///
/// 段是分别展开的，而 `quad::expand_*` 每次调用都会**清空并重建** `Scratch` 的
/// `vertices`/`indices`/`runs`（见 `expand_range_translated_into` 的 `take` +
/// `clear` 那段）。所以「一段的 runs」和「全帧的索引流」不在同一个坐标系里：
/// `first_vertex` 是**段内**偏移，而 draw 要的是**全帧**索引偏移。
///
/// 录制时再推，就必须假设「全帧顶点流里四边形连续、路径在后」—— 那条假设在
/// 分段拼接下不成立（每段都是 solid → images → paths，拼接后路径散落在中间）。
/// 于是把它挪到 prepare：拼接时基址已知，直接算成绝对值。
#[derive(Debug)]
struct DrawCall {
    /// 要绑的纹理名；`None` = 绑图集。
    texture: Option<Rc<str>>,
    blend: BlendMode,
    index_start: u32,
    index_count: u32,
}

/// 一段的绘制计划（prepare 阶段算好，record_frame 阶段回放）。
#[derive(Debug)]
enum SegmentPlan {
    /// 普通段：这一段要发的 draw（索引已是全帧绝对值）。
    Plain { draws: Vec<DrawCall> },
    /// 离屏段：计划 + 两趟顶点偏移（模糊趟们 / 合成趟）。
    Offscreen {
        pass: zephyra_fx::offscreen::OffscreenPass,
        /// 这条命令**自身**的几何（已按 `pass.translate` 平移），要画进 RT。
        ///
        /// ⚠ RT 里装的是「被模糊的东西」，**不是**背后的屏幕。把整屏 resolve 进
        /// RT 去模糊看着像那么回事，实际糊的是背景，而命令本身从没进过 RT ——
        /// 对 `redraw_original == false` 的纯模糊，结果就是这条命令**整条消失**。
        content: Vec<DrawCall>,
        /// 每趟模糊：(顶点偏移, 索引偏移)。
        blurs: Vec<(u32, u32)>,
        /// 合成趟：(顶点偏移, 索引偏移)。
        composite: Option<(u32, u32)>,
    },
}

/// 图像纹理名 ↔ 槽位。
#[derive(Debug, Default)]
pub struct TextureTable {
    slots: Vec<Option<ImageTexture>>,
    index: HashMap<String, u32>,
}

#[derive(Debug)]
struct ImageTexture {
    /// 只为调试输出（`Renderer::Debug` 打纹理名）。
    #[allow(dead_code, reason = "只为 Debug")]
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
    /// 名字 → 纹理视图（record_draws 逐 run 绑定时用）。
    fn view_of(&self, name: &str) -> Option<vk::ImageView> {
        let slot = *self.index.get(name)?;
        self.slots.get(slot as usize)?.as_ref().map(|t| t.view)
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
    /// 描述符集布局（set 1：目标副本）。只在 PS 变体管线上被静态使用。
    desc_layout_dst: vk::DescriptorSetLayout,
    /// 14 档 PS 变体管线（与 [`PS_BLEND_ENTRIES`] 同序）。
    ps_pipelines: Vec<vk::Pipeline>,
    /// 目标副本：PS 变体要采样「当前目标」，而目标正在被当附件写 ——
    /// 所以先把 MSAA resolve 到这张 RT，再拿它当 `g_dst`。
    dst_rt: Option<Rt>,
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
    atlas_size: u32,
    /// 只为持有所有权：图集视图不延长图像与内存的寿命，销毁顺序必须在这里。
    #[allow(dead_code, reason = "只为持有所有权")]
    atlas_memory: vk::DeviceMemory,
    /// 图集已上传到的 revision（变了才重传 —— 与 wgpu 版 sync_atlas 同构）。
    atlas_revision_uploaded: u64,
    /// MSAA 目标当前尺寸（None = 未建）。
    msaa_size: Option<(u32, u32)>,
    /// 图像纹理表。
    textures: TextureTable,
    /// MSAA 渲染通道（`loadOp` 恒 LOAD，见 `create_render_pass`）。
    pub msaa_render_pass: vk::RenderPass,
    /// 单采样渲染通道（离屏模糊趟用）。
    offscreen_render_pass: vk::RenderPass,
    /// MSAA 目标（颜色附件）。
    msaa_image: Option<vk::Image>,
    msaa_view: Option<vk::ImageView>,
    msaa_memory: Option<vk::DeviceMemory>,
    /// 离屏 RT 对（模糊乒乓）。
    rt_a: Option<Rt>,
    rt_b: Option<Rt>,
    /// rt_a / rt_b 的帧缓冲（对单采样 renderpass）。
    rt_framebuffers: Option<(vk::Framebuffer, vk::Framebuffer)>,
    /// 离屏 RT 已分配到的尺寸（None = 未建 / 需重建）。
    rt_size: Option<(u32, u32)>,
    /// 当前这段正在用的 RT 尺寸（视口/裁剪按它设）。
    rt_extent: (u32, u32),
    /// 模糊趟管线（水平 / 垂直）：单采样、画进 RT。
    blur_h: vk::Pipeline,
    blur_v: vk::Pipeline,
    /// 合成趟管线：画回 MSAA 附件。
    composite: vk::Pipeline,
    /// 描述符池。
    desc_pool: vk::DescriptorPool,
    /// 采样器。
    sampler: vk::Sampler,
    /// 跨帧复用的展开缓冲。
    scratch: Scratch,
    /// 一帧的段计划（prepare 算好，record_frame 回放）。
    segments: Vec<SegmentPlan>,
    /// 单段展开用的暂存（跨段复用 —— 见 `prepare` 里那条「每次调用都清空」的警告）。
    segment_scratch: Scratch,
    /// 当前帧的绘制尺寸（物理像素）—— 视口/裁剪由它算。
    size: (u32, u32),
    /// 上传前把字节补齐到 4 的倍数。
    #[allow(dead_code, reason = "离屏 pass 用；接线前先建好")]
    pad: Vec<u8>,
    /// `ZEPHYRA_FRAME_DUMP` 是否已经写过一副图。
    #[allow(dead_code, reason = "离屏 pass 用；接线前先建好")]
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
                // ⚠ 必须与 WGSL 声明逐条一致：binding 0 = `texture_2d<f32>`（采样图像），
                // binding 1 = `sampler`（采样器）。原来写成两个 COMBINED_IMAGE_SAMPLER
                // —— 与管线布局不匹配，症状是「画面全空且零错误」。
                &vk::DescriptorSetLayoutCreateInfo::builder().bindings(&[
                    vk::DescriptorSetLayoutBinding {
                        binding: 0,
                        descriptor_type: vk::DescriptorType::SAMPLED_IMAGE,
                        descriptor_count: 1,
                        stage_flags: vk::ShaderStageFlags::FRAGMENT,
                        ..Default::default()
                    },
                    vk::DescriptorSetLayoutBinding {
                        binding: 1,
                        descriptor_type: vk::DescriptorType::SAMPLER,
                        descriptor_count: 1,
                        stage_flags: vk::ShaderStageFlags::FRAGMENT,
                        ..Default::default()
                    },
                ]),
                None,
            )
        }
        .map_err(|e| SurfaceError::new("descriptor set layout", e.to_string()))?;
        // set 1 = 目标副本（WGSL 的 `@group(1) @binding(0) var g_dst`）。
        let desc_layout_dst = unsafe {
            device.create_descriptor_set_layout(
                &vk::DescriptorSetLayoutCreateInfo::builder().bindings(&[
                    vk::DescriptorSetLayoutBinding {
                        binding: 0,
                        descriptor_type: vk::DescriptorType::SAMPLED_IMAGE,
                        descriptor_count: 1,
                        stage_flags: vk::ShaderStageFlags::FRAGMENT,
                        ..Default::default()
                    },
                ]),
                None,
            )
        }
        .map_err(|e| SurfaceError::new("dst descriptor set layout", e.to_string()))?;
        let pipeline_layout = unsafe {
            device.create_pipeline_layout(
                &vk::PipelineLayoutCreateInfo::builder()
                    .set_layouts(&[desc_layout, desc_layout_dst]),
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
            // 图集用非 sRGB：alpha 通道是**覆盖率**不是颜色 —— 传输函数会破坏它
            //（与 wgpu 版 Rgba8Unorm 同一条理由）。
            vk::Format::R8G8B8A8_UNORM,
            vk::SampleCountFlags::TYPE_1,
            vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST,
        )?;

        // ── 渲染通道 ──
        // 只有**一条**主通道：`loadOp` 恒 LOAD（清空走显式 cmd_clear_*），见
        // create_render_pass 的文档 —— 两条 loadOp 不同的通道会让管线各建一套。
        let msaa_render_pass = create_render_pass(
            device,
            gpu.format,
            samples,
            vk::ImageLayout::PRESENT_SRC_KHR,
        )?;
        // 离屏通道：终点布局必须是 COLOR_ATTACHMENT_OPTIMAL —— PRESENT_SRC 只对
        // 交换链图合法，拿它转 RT 的布局是非法转换。
        let offscreen_render_pass = create_render_pass(
            device,
            gpu.format,
            1,
            vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
        )?;

        // ── 描述符池 ──
        let desc_pool = unsafe {
            device.create_descriptor_pool(
                &vk::DescriptorPoolCreateInfo::builder()
                    // ⚠ 1024 而不是 256：每个 draw 现在要占**两套**描述符集
                    // （set 0 + set 1），一帧几百个 draw 时 256 就不够 ——
                    // 而「不够」的失败形状是 `allocate_descriptor_sets` 返回错误、
                    // 那一个 draw 被 `continue` 跳过，画面少一块且零报错。
                    .max_sets(1024)
                    .pool_sizes(&[
                        vk::DescriptorPoolSize {
                            ty: vk::DescriptorType::SAMPLED_IMAGE,
                            descriptor_count: 1024,
                        },
                        vk::DescriptorPoolSize {
                            ty: vk::DescriptorType::SAMPLER,
                            descriptor_count: 1024,
                        },
                    ]),
                None,
            )
        }
        .map_err(|e| SurfaceError::new("descriptor pool", e.to_string()))?;

        // ── 模糊 / 合成管线（离屏 pass 用）──
        // blur：单采样 renderpass（画进 RT，采样另一张 RT）。
        let blur_h = create_entry_pipeline(
            device,
            1,
            pipeline_layout,
            offscreen_render_pass,
            "fs_blur_h",
            src_blend(),
        )?;
        let blur_v = create_entry_pipeline(
            device,
            1,
            pipeline_layout,
            offscreen_render_pass,
            "fs_blur_v",
            src_blend(),
        )?;
        // composite：画回 MSAA（采样数与主通道一致），SrcOver 混合。
        let composite = create_entry_pipeline(
            device,
            samples,
            pipeline_layout,
            msaa_render_pass,
            "fs_composite",
            src_over_blend(),
        )?;

        // ── 14 档 PS 变体管线 ──
        // 直通混合（`src_blend`）：片元输出的已经是「与目标混完之后」的最终色，
        // 再让固定功能方程混一次就等于混了两遍。
        let mut ps_pipelines = Vec::new();
        for (_mode, entry) in PS_BLEND_ENTRIES {
            ps_pipelines.push(create_entry_pipeline(
                device,
                samples,
                pipeline_layout,
                msaa_render_pass,
                entry,
                src_blend(),
            )?);
        }

        // ── 混合模式管线 ──
        let mut pipelines = Vec::new();
        for mode in blend_modes() {
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
            desc_layout_dst,
            pipeline_layout,
            pipelines,
            ps_pipelines,
            dst_rt: None,
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
            atlas_revision_uploaded: u64::MAX,
            msaa_size: None,
            textures: TextureTable::new(),
            msaa_render_pass,
            offscreen_render_pass,
            msaa_image: None,
            msaa_view: None,
            msaa_memory: None,
            rt_a: None,
            rt_b: None,
            rt_framebuffers: None,
            rt_size: None,
            rt_extent: (1, 1),
            blur_h,
            blur_v,
            composite,
            desc_pool,
            sampler,
            scratch: Scratch::new(),
            segments: Vec::new(),
            segment_scratch: Scratch::new(),
            size: (1, 1),
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
        // 图像纹理用 sRGB 格式（彩色），与 wgpu 版 Rgba8UnormSrgb 对应。
        let (image, view, memory) = create_image_texture(
            &self.device,
            gpu,
            width,
            height,
            vk::Format::R8G8B8A8_SRGB,
            vk::SampleCountFlags::TYPE_1,
            vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST,
        )?;
        if !pixels.is_empty() {
            upload_image(gpu, image, pixels, (width, height))?;
        }
        let slot = if let Some(&s) = self.textures.index.get(name) {
            s
        } else {
            let s = self.textures.slots.len() as u32;
            self.textures.index.insert(name.to_string(), s);
            self.textures.slots.push(None);
            s
        };
        // 覆盖写入时销毁旧句柄 —— 不销毁就是显存泄漏，而 demo 会反复注册同名纹理。
        if let Some(old) = self
            .textures
            .slots
            .get_mut(slot as usize)
            .and_then(|s| s.take())
        {
            unsafe {
                self.device.destroy_image_view(old.view, None);
                self.device.destroy_image(old.image, None);
                self.device.free_memory(old.memory, None);
            }
        }
        self.textures.slots[slot as usize] = Some(ImageTexture {
            name: name.to_string(),
            image,
            view,
            memory,
        });
        Ok(())
    }

    /// 离屏 RT 对的帧缓冲 `(fb_a, fb_b)`；单采样时 None。
    ///
    /// `size` 是**本段**的 `pass.rt_size`（= bounds 向上取整），不是屏幕尺寸：
    /// RT 只覆盖滤镜区域，而 `blur_quad` 的 `local / half_size` 必须与 RT 的真实
    /// 像素尺寸一致 —— 拿屏幕尺寸去算，σ 会整整差一个数量级（模糊几乎看不见）。
    ///
    /// 不同段尺寸不同就重建：两个尺寸交替的帧会反复重建（本仓 demo 正好是
    /// 91×59 / 85×53 交替）。那点开销换「不用在段之间搬来搬去」更划算。
    pub fn offscreen_framebuffers(
        &mut self,
        gpu: &Gpu,
        size: (u32, u32),
    ) -> Option<(vk::Framebuffer, vk::Framebuffer)> {
        if self.samples <= 1 {
            return None;
        }
        let size = (size.0.max(1), size.1.max(1));
        self.rt_extent = size;
        if self.rt_size != Some(size) {
            self.destroy_offscreen();
            let rt_a = create_rt(&self.device, gpu, size)?;
            let rt_b = create_rt(&self.device, gpu, size)?;
            let fb_a =
                create_rt_framebuffer(&self.device, self.offscreen_render_pass, rt_a.view, size);
            let fb_b =
                create_rt_framebuffer(&self.device, self.offscreen_render_pass, rt_b.view, size);
            self.rt_a = Some(rt_a);
            self.rt_b = Some(rt_b);
            self.rt_framebuffers = Some((fb_a, fb_b));
            self.rt_size = Some(size);
        }
        self.rt_framebuffers
    }

    /// 销毁离屏 RT 与帧缓冲。
    fn destroy_offscreen(&mut self) {
        if let Some(rt) = self.dst_rt.take() {
            unsafe {
                self.device.destroy_image_view(rt.view, None);
                self.device.destroy_image(rt.image, None);
                self.device.free_memory(rt.memory, None);
            }
        }
        if let Some((a, b)) = self.rt_framebuffers.take() {
            unsafe {
                self.device.destroy_framebuffer(a, None);
                self.device.destroy_framebuffer(b, None);
            }
        }
        for rt in [self.rt_a.take(), self.rt_b.take()].into_iter().flatten() {
            unsafe {
                self.device.destroy_image_view(rt.view, None);
                self.device.destroy_image(rt.image, None);
                self.device.free_memory(rt.memory, None);
            }
        }
        self.rt_size = None;
    }

    /// MSAA 目标视图（`samples <= 1` 时 None）。尺寸变化时重建。
    pub fn ensure_msaa(&mut self, gpu: &Gpu, size: (u32, u32)) -> Option<vk::ImageView> {
        if self.samples <= 1 {
            return None;
        }
        if self.msaa_size != Some(size) {
            if let (Some(i), Some(v), Some(m)) = (self.msaa_image, self.msaa_view, self.msaa_memory)
            {
                unsafe {
                    self.device.destroy_image_view(v, None);
                    self.device.destroy_image(i, None);
                    self.device.free_memory(m, None);
                }
            }
            let (image, view, memory) = create_image_texture(
                &self.device,
                gpu,
                size.0,
                size.1,
                gpu.format,
                vk::SampleCountFlags::from_raw(self.samples),
                // MSAA 只当颜色附件 + resolve 源。`TRANSFER_SRC` 留着是因为
                // resolve 在 spec 里归为 transfer 操作、要求**源**带这个位 ——
                // 去掉它不会报错，只是 resolve 静默产出全 0（`cmd_resolve_image`
                // 现在没有调用点，但接线时这条坑很容易再踩一次）。
                //
                // ⚠ 方向别搞反：源是 `TRANSFER_SRC`、目标才是 `TRANSFER_DST`。
                vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC,
            )
            .expect("MSAA target");
            self.msaa_image = Some(image);
            self.msaa_view = Some(view);
            self.msaa_memory = Some(memory);
            self.msaa_size = Some(size);
        }
        self.msaa_view
    }

    /// 帧准备：展开顶点 + 上传（**不碰**命令缓冲）。
    ///
    /// # 为什么与「录制」分开
    ///
    /// Vulkan 的命令缓冲必须**先 begin 再录**，而帧里（`frame()`）命令缓冲还没开始
    /// 录制 —— 命令要等 `present()` 开渲染通道之后才录。wgpu 版没这个问题，因为
    /// `record()` 内部自己开 encoder 自己提交。拆成两段正是这个差异的直接映射。
    pub fn prepare(
        &mut self,
        gpu: &Gpu,
        ctx: &RenderCtx<'_>,
    ) -> Result<RenderReport, SurfaceError> {
        let mut report = RenderReport::default();

        // 图集：内容变了才重传（判据 revision）。长度核对防「图集尺寸 ≠ 建好的纹理」
        // —— 不匹配时跳过（打一行），比把越界拷进纹理好查。
        if ctx.atlas_revision != self.atlas_revision_uploaded {
            if !ctx.atlas_pixels.is_empty()
                && ctx.atlas_size == self.atlas_size
                && ctx.atlas_pixels.len() == (self.atlas_size * self.atlas_size * 4) as usize
            {
                upload_image(
                    gpu,
                    self.atlas_image,
                    ctx.atlas_pixels,
                    (self.atlas_size, self.atlas_size),
                )?;
            }
            self.atlas_revision_uploaded = ctx.atlas_revision;
        }

        // ── 段计划 ──
        //
        // 带滤镜的命令要**折返**到离屏去（否则它会盖在所有东西上面），所以一帧
        // 不是「一次全展开」，而是若干 Plain 段与 Offscreen 段交替。
        //
        // ⚠⚠ 每一段都**单独**展开，再由本层拼进全帧缓冲 —— 因为
        // `quad::expand_range_translated_into` 每次调用都把 `Scratch` 的
        // `vertices`/`indices`/`runs` **清空重建**（它 `take` 走旧 Vec 再 `clear`）。
        // 拿同一个 scratch 反复调，得到的只是**最后一段**的几何：症状是
        // 「帧里只剩离屏段附近的东西，其余全没了」，而 `RenderReport` 的计数
        // （逐段 `absorb`）仍然是对的 —— 计数对、画面错，最难自查的一种。
        self.segments.clear();
        let mut scratch = std::mem::take(&mut self.scratch);
        scratch.vertices.clear();
        scratch.indices.clear();
        // 单段展开的暂存，跨段复用（稳态零分配）。
        let mut one = std::mem::take(&mut self.segment_scratch);
        for seg in zephyra_fx::offscreen::plan_segments(ctx.display_list) {
            match seg {
                zephyra_fx::offscreen::Segment::Plain(range) => {
                    let draws = expand_plain(
                        ctx,
                        ctx.display_list,
                        range,
                        &self.textures,
                        &mut one,
                        &mut scratch,
                        &mut report,
                        None,
                    );
                    if !draws.is_empty() {
                        self.segments.push(SegmentPlan::Plain { draws });
                    }
                }
                zephyra_fx::offscreen::Segment::Offscreen {
                    index,
                    pass,
                    redraw,
                    key,
                } => {
                    // ⚠ `DisplayList` 没有 `get(index)`；段下标就是命令下标，
                    // 按同一顺序 iter().nth() 取回的就是同一条。
                    let Some(item) = ctx.display_list.iter().nth(index) else {
                        continue;
                    };
                    let single = zephyra_fx::offscreen::single_command_list(&item.command, key);
                    // ① 画进 RT 的那份：带 `pass.translate`（影子要在模糊**之前**平移）。
                    let content = expand_plain(
                        ctx,
                        &single,
                        0..1,
                        &self.textures,
                        &mut one,
                        &mut scratch,
                        &mut report,
                        Some(pass.translate),
                    );
                    self.segments.push(SegmentPlan::Offscreen {
                        pass,
                        content,
                        blurs: Vec::new(),
                        composite: None,
                    });
                    if redraw {
                        // 合成之后再照常画一遍原命令（投影类效果）：不折返、不平移，
                        // 直接画回屏幕坐标。
                        let single = zephyra_fx::offscreen::single_command_list(&item.command, key);
                        let draws = expand_plain(
                            ctx,
                            &single,
                            0..1,
                            &self.textures,
                            &mut one,
                            &mut scratch,
                            &mut report,
                            None,
                        );
                        if !draws.is_empty() {
                            self.segments.push(SegmentPlan::Plain { draws });
                        }
                    }
                }
            }
        }
        self.segment_scratch = one;

        report.vertex_bytes = (scratch.vertices.len() * mem::size_of::<Vertex>()) as u64;
        self.size = (ctx.size.0.max(1), ctx.size.1.max(1));

        // 离屏趟的四边形追加到**末尾**：它们用独立的顶点偏移绑定，与主几何互不干涉。
        self.build_offscreen_geometry(ctx.size, &mut scratch);

        self.upload_geometry(gpu, &scratch, &mut report)?;
        self.scratch = scratch;
        Ok(report)
    }

    /// 为每个离屏段生成模糊/合成四边形（追加到 vertices/indices 末尾）。
    ///
    /// `size` 是**屏幕**尺寸（合成趟的几何要按它算 NDC）；模糊趟与合成的**采样**
    /// 尺寸则用该段自己的 `pass.rt_size`（= bounds 向上取整）—— RT 只那么大，
    /// 而 `local / half_size` 必须与 RT 的真实像素尺寸一致，否则模糊半径会按
    /// 屏幕尺寸缩放，σ 直接缩水一个数量级。
    fn build_offscreen_geometry(&mut self, size: (u32, u32), scratch: &mut Scratch) {
        for i in 0..self.segments.len() {
            // 先只读地取到要用的数据（克隆 blur 列表与 bounds/tint），之后才可变地
            // 写回偏移 —— 一次解构里同时拿 `&pass` 与 `&mut blurs` 编译不过。
            let (blur_steps, bounds, tint, rt_f) = match &self.segments[i] {
                SegmentPlan::Offscreen { pass, .. } => (
                    pass.blurs.clone(),
                    pass.bounds,
                    pass.tint,
                    (pass.rt_size.0 as f32, pass.rt_size.1 as f32),
                ),
                SegmentPlan::Plain { .. } => continue,
            };
            for step in &blur_steps {
                let vtx_off = scratch.vertices.len() as u32;
                let idx_off = scratch.indices.len() as u32;
                scratch
                    .vertices
                    .extend_from_slice(&zephyra_fx::offscreen::blur_quad(step, rt_f, rt_f));
                scratch.indices.extend_from_slice(&[0u16, 1, 2, 0, 2, 3]);
                let SegmentPlan::Offscreen { blurs, .. } = &mut self.segments[i] else {
                    unreachable!("刚才是 Offscreen 分支")
                };
                blurs.push((vtx_off, idx_off));
            }
            let vtx_off = scratch.vertices.len() as u32;
            let idx_off = scratch.indices.len() as u32;
            let screen_f = (size.0 as f32, size.1 as f32);
            let composite = zephyra_fx::offscreen::composite_quad(bounds, screen_f, rt_f, tint);
            scratch.vertices.extend_from_slice(&composite);
            scratch.indices.extend_from_slice(&[0u16, 1, 2, 0, 2, 3]);
            let SegmentPlan::Offscreen { composite: c, .. } = &mut self.segments[i] else {
                unreachable!("刚才是 Offscreen 分支")
            };
            *c = Some((vtx_off, idx_off));
        }
    }

    /// 录制**一整帧**的所有通道（普通段与离屏段交替）。
    ///
    /// 调用方只需 `begin_command_buffer` → 本方法 → `end_command_buffer`；
    /// 所有 `begin_render_pass` 与布局屏障都在这里。
    ///
    /// # 单采样为什么不拆段
    ///
    /// 单采样时主通道**直接画进交换链图**，它的 `loadOp` 只能是 `CLEAR`
    /// （交换链图不能跨通道保留内容 —— 那正是多趟合成必须经 MSAA 中转的原因）。
    /// 于是多趟就等于「每趟擦一次」，只剩最后一趟的内容。所以 `samples <= 1`
    /// 时这里把全部 run 一趟画完、忽略段结构：离屏链本来也用不了（没有 MSAA
    /// 做 resolve 起点），段边界对画面没有意义。
    pub fn record_frame(
        &mut self,
        gpu: &Gpu,
        cmd: vk::CommandBuffer,
        framebuffer: vk::Framebuffer,
        default_blend: BlendMode,
    ) -> u32 {
        let mut unimplemented = 0u32;
        // 描述符池每帧重置一次（**不是每段一次**）。
        //
        // ⚠ 这里曾经写成「每段重置」，后果是隐蔽的崩溃：描述符集是 per-draw
        // 现场分配的，第 2 段的 reset 把第 1 段已绑定、GPU 还要读的描述符集
        // 释放掉了 —— 命令本身全部录完才提交，所以驱动在 `wait_for_fences`
        // 里读到已释放内存才 segfault，画面、帧计数、返回值一切正常。
        // 「重置必须在所有段录完、命令提交之后才做有效」——所以放帧首。
        unsafe {
            self.device
                .reset_descriptor_pool(self.desc_pool, vk::DescriptorPoolResetFlags::empty())
                .ok();
        }
        if self.samples <= 1 {
            // 单采样：整帧**一趟**画完，段边界只影响「哪些 draw 属于哪一段」，
            // 而不拆通道。离屏段的内容因此丢失（离屏链需要 MSAA 做 resolve 起点），
            // 与「滤镜不可用」的既有语义一致。
            //
            // clear=false：这条通道的 loadOp 就是 CLEAR，`clear_values` 会生效，
            // 再叠一次 cmd_clear_attachments 是白清一遍交换链图。
            self.begin_main_pass(cmd, framebuffer, false);
            let segments = std::mem::take(&mut self.segments);
            for seg in &segments {
                if let SegmentPlan::Plain { draws } = seg {
                    // 单采样直接画进交换链图，没有 MSAA 也就没有「目标副本」，
                    // PS 变体在此退化为默认混合（由 record_draws 内部计数）。
                    self.record_draws(
                        gpu,
                        cmd,
                        framebuffer,
                        false,
                        draws,
                        default_blend,
                        &mut unimplemented,
                    );
                }
            }
            self.segments = segments;
            unsafe {
                self.device.cmd_end_render_pass(cmd);
            }
            return unimplemented;
        }
        // ── 帧首：把 MSAA 转到 COLOR_ATTACHMENT_OPTIMAL ──
        //
        // 主通道的附件 0 声明 `initial_layout = COLOR_ATTACHMENT_OPTIMAL`，而
        // Vulkan 要求**声明的 initial_layout 必须与实际布局一致**。刚建的图实际是
        // UNDEFINED，与声明不符 = 未定义行为（llvmpipe 在 wait_for_fences 里
        // core dump，没有任何报错）。
        //
        // 每帧都发这条屏障（而不是只在首帧发）有两个理由：
        //   1. resize 会重建 MSAA，「首帧」是相对重建而言的，跨帧记状态容易漏；
        //   2. `old_layout = UNDEFINED` 意味着「旧内容不必等、不必保留」，所以
        //      丢弃上一帧的 MSAA 内容是安全的 —— 而首趟本来就会 clear 它。
        // 这条屏障同时也是「本帧首次写 MSAA」的可见性/顺序保证。
        if let Some(msaa) = self.msaa_image {
            self.image_barrier(
                cmd,
                msaa,
                vk::ImageLayout::UNDEFINED,
                vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
                vk::AccessFlags::empty(),
                vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
                vk::PipelineStageFlags::TOP_OF_PIPE,
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
            );
        }
        // 主通道是否已开过：首趟清，之后一律续画（`loadOp` 恒 LOAD，清空是
        // `begin_main_pass(.., true)` 里的 `cmd_clear_attachments`）。
        let mut opened = false;
        let segments = std::mem::take(&mut self.segments);
        for seg in &segments {
            match seg {
                SegmentPlan::Plain { draws } => {
                    self.begin_main_pass(cmd, framebuffer, !opened);
                    self.record_draws(
                        gpu,
                        cmd,
                        framebuffer,
                        true,
                        draws,
                        default_blend,
                        &mut unimplemented,
                    );
                    unsafe {
                        self.device.cmd_end_render_pass(cmd);
                    }
                    opened = true;
                }
                SegmentPlan::Offscreen {
                    pass,
                    content,
                    blurs,
                    composite,
                } => {
                    // 每段按**自己的** rt_size 准备 RT（尺寸变了会重建）。
                    let Some((fb_a, fb_b)) = self.offscreen_framebuffers(gpu, pass.rt_size) else {
                        // 没有离屏 RT（单采样 / 建不出来）：折返段退化为不画，
                        // 与「滤镜不可用」的既有语义一致。
                        continue;
                    };
                    if !opened {
                        // 只为把 MSAA 清成确定值（合成趟是 loadOp=LOAD，不清会把
                        // 上一帧的残留当本帧底图）。
                        self.begin_main_pass(cmd, framebuffer, true);
                        unsafe {
                            self.device.cmd_end_render_pass(cmd);
                        }
                        opened = true;
                    }
                    self.record_offscreen(
                        gpu,
                        cmd,
                        framebuffer,
                        fb_a,
                        fb_b,
                        pass,
                        content,
                        blurs,
                        *composite,
                        default_blend,
                        &mut unimplemented,
                    );
                }
            }
        }
        self.segments = segments;
        unimplemented
    }

    /// 开主通道（MSAA → surface）并设视口/裁剪。
    ///
    /// `clear` 为 true 时在通道**内**清一遍 MSAA（`cmd_clear_attachments`）——
    /// 通道恒 `loadOp=LOAD`，「首趟要清」必须显式做，见 `create_render_pass`。
    fn begin_main_pass(&self, cmd: vk::CommandBuffer, framebuffer: vk::Framebuffer, clear: bool) {
        // ⚠ loadOp 是 LOAD，所以这里传的这组 clear **不会**被驱动执行 ——
        // 它只是 `cmd_begin_render_pass` 的合法入参（长度要够）。真正的清空在
        // 下面的 `cmd_clear_attachments`。传 CLEAR 值却指望它生效，是本文件里
        // 最容易复发的一处「看起来做了其实没做」。
        let clears = [vk::ClearValue {
            color: vk::ClearColorValue {
                float32: [0.0, 0.0, 0.0, 0.0],
            },
        }];
        let viewports = [self.viewport()];
        let scissors = [self.full_rect()];
        unsafe {
            self.device.cmd_begin_render_pass(
                cmd,
                &vk::RenderPassBeginInfo::builder()
                    .render_pass(self.msaa_render_pass)
                    .framebuffer(framebuffer)
                    .render_area(self.full_rect())
                    .clear_values(&clears)
                    .build(),
                vk::SubpassContents::INLINE,
            );
            self.device.cmd_set_viewport(cmd, 0, &viewports);
            self.device.cmd_set_scissor(cmd, 0, &scissors);
            if clear {
                // 只清 attachment 0（MSAA）。attachment 1 是 resolve 目标、每趟末
                // 都被整块重写，清它是白费。
                //
                // `clear_value` 是 `ClearValue` 联合体（不是裸 ClearColorValue）。
                let clear_attachments = [vk::ClearAttachment {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    color_attachment: 0,
                    clear_value: vk::ClearValue {
                        color: vk::ClearColorValue {
                            float32: [0.0, 0.0, 0.0, 0.0],
                        },
                    },
                }];
                // rects 必须是**非空**的：传空切片等于一个矩形都不清，而那不报错
                // —— 症状是「清空指令看着在录，画面却带着上一帧的残留」。
                let rects = [vk::ClearRect {
                    rect: self.full_rect(),
                    base_array_layer: 0,
                    layer_count: 1,
                }];
                self.device
                    .cmd_clear_attachments(cmd, &clear_attachments, &rects);
            }
        }
    }

    /// 开一趟离屏通道（RT 目标），视口/裁剪按 RT 尺寸。
    ///
    /// 这条通道的 `loadOp` 是 CLEAR、终点布局是 COLOR_ATTACHMENT_OPTIMAL：
    /// 每趟都整块重画，载入旧内容毫无意义（而且 RT 的旧布局是
    /// SHADER_READ_ONLY_OPTIMAL，声明 UNDEFINED 才是合法的那个）。
    fn begin_rt_pass(&self, cmd: vk::CommandBuffer, framebuffer: vk::Framebuffer) {
        let clears = [vk::ClearValue {
            color: vk::ClearColorValue {
                float32: [0.0, 0.0, 0.0, 0.0],
            },
        }];
        let viewports = [vk::Viewport {
            x: 0.0,
            y: 0.0,
            width: self.rt_extent.0 as f32,
            height: self.rt_extent.1 as f32,
            min_depth: 0.0,
            max_depth: 1.0,
        }];
        let scissors = [vk::Rect2D {
            offset: vk::Offset2D { x: 0, y: 0 },
            extent: vk::Extent2D {
                width: self.rt_extent.0,
                height: self.rt_extent.1,
            },
        }];
        unsafe {
            self.device.cmd_begin_render_pass(
                cmd,
                &vk::RenderPassBeginInfo::builder()
                    .render_pass(self.offscreen_render_pass)
                    .framebuffer(framebuffer)
                    .render_area(scissors[0])
                    // ⚠ loadOp=CLEAR 时 clear_values 不能是空切片 —— 那是 spec 违规，
                    // llvmpipe 在 submit 之后才 core dump（实测：无 backtrace）。
                    .clear_values(&clears)
                    .build(),
                vk::SubpassContents::INLINE,
            );
            self.device.cmd_set_viewport(cmd, 0, &viewports);
            self.device.cmd_set_scissor(cmd, 0, &scissors);
        }
    }

    /// 一个离屏段：内容进 RT → 模糊乒乓 → 合成回 MSAA。
    #[allow(
        clippy::too_many_arguments,
        reason = "三个 framebuffer（主/a/b）+ 计划 + 顶点偏移 + 内容绘制，合成结构体更绕"
    )]
    fn record_offscreen(
        &mut self,
        gpu: &Gpu,
        cmd: vk::CommandBuffer,
        framebuffer: vk::Framebuffer,
        fb_a: vk::Framebuffer,
        fb_b: vk::Framebuffer,
        pass: &zephyra_fx::offscreen::OffscreenPass,
        content: &[DrawCall],
        blurs: &[(u32, u32)],
        composite: Option<(u32, u32)>,
        default_blend: BlendMode,
        unimplemented: &mut u32,
    ) {
        // ⚠ 必须把 view **拷成局部值**：直接持有 `&self.rt_a` 会让后面
        // `&mut self` 的 `update_descriptor` 借不出来（句柄都是 Copy 的复制品）。
        let (Some(rt_a_view), Some(rt_b_view), Some(rt_b_img), Some(rt_a_img)) = (
            self.rt_a.as_ref().map(|r| r.view),
            self.rt_b.as_ref().map(|r| r.view),
            self.rt_b.as_ref().map(|r| r.image),
            self.rt_a.as_ref().map(|r| r.image),
        ) else {
            // RT 建不出来（显存不够）：整段跳过，症状与「滤镜不可用」一致。
            return;
        };
        let _ = rt_a_img;
        // ── 1) 把这条命令自身画进 rt_a ──
        //
        // ⚠ 不是「把整屏 resolve 进 RT」。RT 装的是**被模糊的那个东西**：这条命令
        // 自己（已按 `pass.translate` 平移过 —— 影子的位移必须在模糊之前发生）。
        // 把背后的屏幕 resolve 进去看着像那么回事，实际糊的是背景，而命令本身
        // 从没进过 RT —— 对 `redraw_original == false` 的纯模糊，结果是这条命令
        // **整条消失**（Linux 少画两张深蓝卡片、Windows 画出来了，就是这么来的）。
        self.begin_rt_pass(cmd, fb_a);
        let mut rt_unimplemented = 0u32;
        self.record_draws(
            gpu,
            cmd,
            fb_a,
            false,
            content,
            default_blend,
            &mut rt_unimplemented,
        );
        unsafe {
            self.device.cmd_end_render_pass(cmd);
        }
        *unimplemented += rt_unimplemented;

        // ── 2) 模糊趟：rt_a ⇄ rt_b 乒乓 ──
        // ── 2) 模糊趟：rt_a ⇄ rt_b 乒乓 ──
        // 末趟的输出就是合成要采样的源 —— 在循环里记下来，不要在循环后猜
        // 「最后一趟是水平还是垂直」（猜错的后果是采到**上一趟**的结果，
        // 症状是模糊两遍/少一遍，而帧计数器全对）。
        let mut last_view = rt_a_view;
        for (i, (vtx_off, idx_off)) in blurs.iter().enumerate() {
            let horizontal = pass.blurs.get(i).map(|s| s.horizontal).unwrap_or(true);
            let (src_view, dst_image, fb, pipeline) = if horizontal {
                (rt_a_view, rt_b_img, fb_b, self.blur_h)
            } else {
                (rt_b_view, rt_a_img, fb_a, self.blur_v)
            };
            // ⚠ 模糊趟的通道 `loadOp` 是 CLEAR（整块覆盖），所以 clear_values **必须**
            // 至少有一个颜色项 —— 传空切片是 spec 违规，llvmpipe 在 submit 时
            // 直接 core dump（实测：submit 返回后才崩，无 backtrace）。
            self.begin_rt_pass(cmd, fb);
            if let Ok(desc) = self.update_descriptor(src_view) {
                let sets = [desc];
                let vertex_offsets = [*vtx_off as u64 * mem::size_of::<Vertex>() as u64];
                unsafe {
                    self.device
                        .cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, pipeline);
                    self.device.cmd_bind_descriptor_sets(
                        cmd,
                        vk::PipelineBindPoint::GRAPHICS,
                        self.pipeline_layout,
                        0,
                        &sets,
                        &[],
                    );
                    self.device.cmd_bind_vertex_buffers(
                        cmd,
                        0,
                        &[self.vertex_buffer],
                        &vertex_offsets,
                    );
                    self.device.cmd_bind_index_buffer(
                        cmd,
                        self.index_buffer,
                        0,
                        vk::IndexType::UINT16,
                    );
                    self.device.cmd_draw_indexed(cmd, 6, 1, *idx_off, 0, 0);
                }
            }
            unsafe {
                self.device.cmd_end_render_pass(cmd);
            }
            self.image_barrier(
                cmd,
                dst_image,
                vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
                vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
                vk::AccessFlags::SHADER_READ,
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                vk::PipelineStageFlags::FRAGMENT_SHADER,
            );
            last_view = if horizontal { rt_b_view } else { rt_a_view };
        }

        // ── 3) 合成：采样末趟输出，画进 MSAA（Load）──
        let Some((vtx_off, idx_off)) = composite else {
            return;
        };
        self.begin_main_pass(cmd, framebuffer, false);
        if let Ok(desc) = self.update_descriptor(last_view) {
            let sets = [desc];
            let vertex_offsets = [vtx_off as u64 * mem::size_of::<Vertex>() as u64];
            unsafe {
                self.device
                    .cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, self.composite);
                self.device.cmd_bind_descriptor_sets(
                    cmd,
                    vk::PipelineBindPoint::GRAPHICS,
                    self.pipeline_layout,
                    0,
                    &sets,
                    &[],
                );
                self.device
                    .cmd_bind_vertex_buffers(cmd, 0, &[self.vertex_buffer], &vertex_offsets);
                self.device
                    .cmd_bind_index_buffer(cmd, self.index_buffer, 0, vk::IndexType::UINT16);
                self.device.cmd_draw_indexed(cmd, 6, 1, idx_off, 0, 0);
            }
        }
        unsafe {
            self.device.cmd_end_render_pass(cmd);
        }
    }

    /// 布局屏障（图像级）。
    #[allow(clippy::too_many_arguments)]
    fn image_barrier(
        &self,
        cmd: vk::CommandBuffer,
        image: vk::Image,
        old: vk::ImageLayout,
        new: vk::ImageLayout,
        src_access: vk::AccessFlags,
        dst_access: vk::AccessFlags,
        src_stage: vk::PipelineStageFlags,
        dst_stage: vk::PipelineStageFlags,
    ) {
        let bar = vk::ImageMemoryBarrier::builder()
            .src_access_mask(src_access)
            .dst_access_mask(dst_access)
            .old_layout(old)
            .new_layout(new)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(image)
            .subresource_range(self.color_range())
            .build();
        let bars = [bar];
        unsafe {
            self.device.cmd_pipeline_barrier(
                cmd,
                src_stage,
                dst_stage,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &bars,
            );
        }
    }

    /// 颜色子资源范围（单 mip 单层 —— 本 crate 不用 mipmap / 数组层）。
    fn color_range(&self) -> vk::ImageSubresourceRange {
        vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        }
    }

    /// 当前视口（动态状态，尺寸变化时由每帧重设）。
    fn viewport(&self) -> vk::Viewport {
        vk::Viewport {
            x: 0.0,
            y: 0.0,
            width: self.size.0 as f32,
            height: self.size.1 as f32,
            min_depth: 0.0,
            max_depth: 1.0,
        }
    }

    /// 整屏裁剪矩形。
    fn full_rect(&self) -> vk::Rect2D {
        vk::Rect2D {
            offset: vk::Offset2D { x: 0, y: 0 },
            extent: vk::Extent2D {
                width: self.size.0,
                height: self.size.1,
            },
        }
    }

    /// 录制一个 Plain 段的全部 draw（**必须在主通道内**）。
    ///
    /// 索引偏移在 `prepare` 阶段就算成了全帧绝对值（见 [`expand_plain`]），所以
    /// 这里不再做任何坐标换算 ——「录制时才知道索引从哪来」正是上一版把画面弄坏的
    /// 原因（见 `expand_plain` 的文档）。
    /// 录制一批 draw。`in_main_pass` 为真时表示当前正在主通道里 ——
    /// PS 变体要在这里结束/重开通道来腾出「目标可采样」的窗口。
    #[allow(
        clippy::too_many_arguments,
        reason = "gpu/cmd/framebuffer/in_main_pass 是录制环境，draws 与混合是内容"
    )]
    fn record_draws(
        &mut self,
        gpu: &Gpu,
        cmd: vk::CommandBuffer,
        framebuffer: vk::Framebuffer,
        in_main_pass: bool,
        draws: &[DrawCall],
        default_blend: BlendMode,
        unimplemented: &mut u32,
    ) {
        if draws.is_empty() {
            return;
        }
        // 先只读地解析出 (视图, 管线, 索引起点, 数量)：`&self.textures` /
        // `&self.pipelines` 的借用期间不能调 `&mut self` 的 `update_descriptor`
        // （E0502），所以分两遍。
        let mut resolved: Vec<(vk::ImageView, DrawPipeline, u32, u32)> = Vec::new();
        for d in draws {
            let tex_view = match &d.texture {
                Some(name) => self.textures.view_of(name).unwrap_or(self.atlas_view),
                None => self.atlas_view,
            };
            let pipeline = match self.pipeline_for_blend(d.blend) {
                Some(p) => p,
                None => {
                    *unimplemented += 1;
                    self.pipeline_for_blend(default_blend)
                        .expect("default 必有管线")
                }
            };
            resolved.push((tex_view, pipeline, d.index_start, d.index_count));
        }
        let vertex_offsets = [0u64];
        unsafe {
            self.device
                .cmd_bind_vertex_buffers(cmd, 0, &[self.vertex_buffer], &vertex_offsets);
            self.device
                .cmd_bind_index_buffer(cmd, self.index_buffer, 0, vk::IndexType::UINT16);
        }
        for (tex_view, pipeline, index_start, index_count) in resolved {
            let is_ps = matches!(pipeline, DrawPipeline::Ps(_));
            // PS 变体在 RT 通道里没有「目标副本」可言（那张 RT 本身就是目标），
            // 退回默认混合并计数 —— 与其它「不可用」的语义一致。
            let dst = if is_ps {
                // RT 通道里没有「目标副本」可言（那张 RT 本身就是目标），
                // 退回并计数 —— 与其它「不可用」的既有语义一致。
                if !in_main_pass {
                    *unimplemented += 1;
                    continue;
                }
                self.ensure_dst_rt(gpu, self.size);
                self.dst_rt.as_ref().map(|rt| (rt.image, rt.view, rt.size))
            } else {
                None
            };
            if let Some((img, _view, sz)) = dst {
                // 目标此刻是附件 —— 必须先结束通道、resolve 出副本、再重开。
                unsafe {
                    self.device.cmd_end_render_pass(cmd);
                }
                let rt = Rt {
                    image: img,
                    view: vk::ImageView::null(),
                    memory: vk::DeviceMemory::null(),
                    size: sz,
                };
                self.snapshot_dst(cmd, &rt);
                self.begin_main_pass(cmd, framebuffer, false);
            }
            let desc_set = match self.update_descriptor(tex_view) {
                Ok(s) => s,
                Err(_) => continue,
            };
            // set 1 始终绑：管线布局有两套 set，`fs_main` 虽然静态不用 set 1，
            // 但绑上它比赌「驱动只校验用到的」便宜得多。PS 变体缺这一绑就是
            // 「画面少一块且零报错」，所以宁可多绑一次。
            let dst_view = self.dst_rt.as_ref().map_or(self.atlas_view, |rt| rt.view);
            let dst_set = match self.update_dst_descriptor(dst_view) {
                Ok(s) => s,
                Err(_) => continue,
            };
            let sets = [desc_set, dst_set];
            let handle = match pipeline {
                DrawPipeline::Fixed(p) | DrawPipeline::Ps(p) => p,
            };
            unsafe {
                self.device
                    .cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, handle);
                self.device.cmd_bind_descriptor_sets(
                    cmd,
                    vk::PipelineBindPoint::GRAPHICS,
                    self.pipeline_layout,
                    0,
                    &sets,
                    &[],
                );
                self.device
                    .cmd_draw_indexed(cmd, index_count, 1, index_start, 0, 0);
            }
        }
    }

    /// 更新描述符集（纹理 + 采样器）。
    fn update_descriptor(
        &mut self,
        tex_view: vk::ImageView,
    ) -> Result<vk::DescriptorSet, SurfaceError> {
        let desc_set = unsafe {
            self.device.allocate_descriptor_sets(
                &vk::DescriptorSetAllocateInfo::builder()
                    .descriptor_pool(self.desc_pool)
                    .set_layouts(&[self.desc_layout]),
            )
        }
        .map_err(|e| SurfaceError::new("allocate descriptor set", e.to_string()))?[0];
        // 两个 write 对应两个 binding（图像 / 采样器分开声明）。数组绑局部变量 ——
        // 悬垂指针的教训在 dev-log 里，不重复。
        let image_infos = [vk::DescriptorImageInfo {
            sampler: vk::Sampler::null(),
            image_view: tex_view,
            image_layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
        }];
        let sampler_infos = [vk::DescriptorImageInfo {
            sampler: self.sampler,
            image_view: vk::ImageView::null(),
            image_layout: vk::ImageLayout::UNDEFINED,
        }];
        let writes = [
            vk::WriteDescriptorSet::builder()
                .dst_set(desc_set)
                .dst_binding(0)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .image_info(&image_infos)
                .build(),
            vk::WriteDescriptorSet::builder()
                .dst_set(desc_set)
                .dst_binding(1)
                .descriptor_type(vk::DescriptorType::SAMPLER)
                .image_info(&sampler_infos)
                .build(),
        ];
        unsafe {
            self.device.update_descriptor_sets(&writes, &[]);
        }
        Ok(desc_set)
    }

    /// 混合模式 → 管线索引。
    fn pipeline_for_blend(&self, blend: BlendMode) -> Option<DrawPipeline> {
        // ⚠ 这里**不能**写 `blend_mode_index(blend)?`：`?` 会把「不在系数表里」
        // 当成「这个模式完全不可用」直接从函数返回，于是 14 档 PS 变体永远走不到
        // 下面那次 `ps_blend_index` —— 症状恰好是「PS 变体全部退回默认混合」，
        // 而画面里看不出任何异常。
        if let Some(p) = blend_mode_index(blend).and_then(|i| self.pipelines.get(i).copied()) {
            return Some(DrawPipeline::Fixed(p));
        }
        let p = ps_blend_index(blend).and_then(|i| self.ps_pipelines.get(i).copied())?;
        Some(DrawPipeline::Ps(p))
    }

    /// 目标副本 RT（PS 变体的 `g_dst`），按 MSAA 尺寸惰性建。
    fn ensure_dst_rt(&mut self, gpu: &Gpu, size: (u32, u32)) {
        let size = (size.0.max(1), size.1.max(1));
        if self.dst_rt.as_ref().map(|r| r.size) != Some(size) {
            if let Some(old) = self.dst_rt.take() {
                unsafe {
                    self.device.destroy_image_view(old.view, None);
                    self.device.destroy_image(old.image, None);
                    self.device.free_memory(old.memory, None);
                }
            }
            let Ok((image, view, memory)) = create_image_texture(
                &self.device,
                gpu,
                size.0,
                size.1,
                gpu.format,
                vk::SampleCountFlags::TYPE_1,
                // ⚠ 三个位缺一即崩：
                //   COLOR_ATTACHMENT —— resolve 的目标必须带这个（resolve 走
                //     「附件解析」这条路径，不是拷贝）；
                //   TRANSFER_DST    —— spec 明确要求 resolve 的目标带它；
                //   SAMPLED         —— 这张 RT 要绑进描述符当 `g_dst` 采样。
                // 少 `SAMPLED` 时 llvmpipe 不报错、直接在采样时 core dump
                // （与 `create_rt` 同一个坑，本轮踩了两次）。
                vk::ImageUsageFlags::COLOR_ATTACHMENT
                    | vk::ImageUsageFlags::TRANSFER_DST
                    | vk::ImageUsageFlags::SAMPLED,
            ) else {
                return;
            };
            self.dst_rt = Some(Rt {
                image,
                view,
                memory,
                size,
            });
        }
    }

    /// 把 MSAA resolve 到目标副本 RT，并把它转成 SHADER_READ_ONLY。
    ///
    /// ⚠ `old_layout` 填 `UNDEFINED`：那张 RT 上一次被采样后停在
    /// `SHADER_READ_ONLY_OPTIMAL`，而我们只把它当「只读快照」用、内容每次都被
    /// resolve 整体覆盖 —— 于是不必去追踪它此刻究竟在哪个布局。
    fn snapshot_dst(&self, cmd: vk::CommandBuffer, dst: &Rt) {
        let Some(msaa) = self.msaa_image else {
            return;
        };
        self.image_barrier(
            cmd,
            dst.image,
            vk::ImageLayout::UNDEFINED,
            vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
            vk::AccessFlags::empty(),
            vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
            vk::PipelineStageFlags::TOP_OF_PIPE,
            vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
        );
        let regions = [vk::ImageResolve::builder()
            .src_subresource(
                vk::ImageSubresourceLayers::builder()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .mip_level(0)
                    .base_array_layer(0)
                    .layer_count(1)
                    .build(),
            )
            .src_offset(vk::Offset3D { x: 0, y: 0, z: 0 })
            .dst_subresource(
                vk::ImageSubresourceLayers::builder()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .mip_level(0)
                    .base_array_layer(0)
                    .layer_count(1)
                    .build(),
            )
            .dst_offset(vk::Offset3D { x: 0, y: 0, z: 0 })
            .extent(vk::Extent3D {
                width: dst.size.0,
                height: dst.size.1,
                depth: 1,
            })
            .build()];
        unsafe {
            self.device.cmd_resolve_image(
                cmd,
                msaa,
                vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
                dst.image,
                vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
                &regions,
            );
        }
        self.image_barrier(
            cmd,
            dst.image,
            vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
            vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
            vk::AccessFlags::SHADER_READ,
            vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
            vk::PipelineStageFlags::FRAGMENT_SHADER,
        );
    }

    /// 分配并写入 set 1（目标副本）。
    fn update_dst_descriptor(
        &mut self,
        view: vk::ImageView,
    ) -> Result<vk::DescriptorSet, SurfaceError> {
        let set = unsafe {
            self.device.allocate_descriptor_sets(
                &vk::DescriptorSetAllocateInfo::builder()
                    .descriptor_pool(self.desc_pool)
                    .set_layouts(&[self.desc_layout_dst]),
            )
        }
        .map_err(|e| SurfaceError::new("allocate dst descriptor set", e.to_string()))?[0];
        let infos = [vk::DescriptorImageInfo {
            sampler: vk::Sampler::null(),
            image_view: view,
            image_layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
        }];
        let writes = [vk::WriteDescriptorSet::builder()
            .dst_set(set)
            .dst_binding(0)
            .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
            .image_info(&infos)
            .build()];
        unsafe {
            self.device.update_descriptor_sets(&writes, &[]);
        }
        Ok(set)
    }

    /// 上传顶点和索引到 GPU 可见缓冲。
    fn upload_geometry(
        &mut self,
        _gpu: &Gpu,
        scratch: &Scratch,
        _report: &mut RenderReport,
    ) -> Result<(), SurfaceError> {
        let device = &self.device;
        let vbytes = scratch.vertices.len() * mem::size_of::<Vertex>();
        let ibytes = scratch.indices.len() * mem::size_of::<u16>();

        // 顶点。
        if vbytes as u64 > self.vertex_capacity {
            // 扩容（简化：panic，实际应重建）。
            panic!(
                "vertex buffer overflow: {vbytes} > {}",
                self.vertex_capacity
            );
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
        BlendMode::Modulate => Some(13),
        BlendMode::Screen => Some(14),
        _ => None,
    }
}

/// 建一条图形管线。
#[allow(clippy::too_many_arguments)]
fn create_graphics_pipeline(
    device: &ash::Device,
    _format: vk::Format,
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
    let pipeline =
        unsafe { device.create_graphics_pipelines(vk::PipelineCache::null(), &infos, None) }
            .map_err(|e| SurfaceError::new("create graphics pipeline", format!("{:?}", e)))?
            .remove(0);
    Ok(pipeline)
}

/// 直通混合（`Src`）：用于模糊趟 —— 它写的是**采样结果本身**，
/// 不能与目标做混合（模糊趟的目标 RT 是「整块覆盖」）。
fn src_blend() -> vk::PipelineColorBlendAttachmentState {
    vk::PipelineColorBlendAttachmentState {
        blend_enable: vk::TRUE,
        src_color_blend_factor: vk::BlendFactor::ONE,
        dst_color_blend_factor: vk::BlendFactor::ZERO,
        color_blend_op: vk::BlendOp::ADD,
        src_alpha_blend_factor: vk::BlendFactor::ONE,
        dst_alpha_blend_factor: vk::BlendFactor::ZERO,
        alpha_blend_op: vk::BlendOp::ADD,
        color_write_mask: vk::ColorComponentFlags::R
            | vk::ColorComponentFlags::G
            | vk::ColorComponentFlags::B
            | vk::ColorComponentFlags::A,
    }
}

/// 标准覆盖（`SrcOver`）：用于合成趟 —— 它叠在**已有内容**上。
fn src_over_blend() -> vk::PipelineColorBlendAttachmentState {
    vk::PipelineColorBlendAttachmentState {
        blend_enable: vk::TRUE,
        src_color_blend_factor: vk::BlendFactor::ONE,
        dst_color_blend_factor: vk::BlendFactor::ONE_MINUS_SRC_ALPHA,
        color_blend_op: vk::BlendOp::ADD,
        src_alpha_blend_factor: vk::BlendFactor::ONE,
        dst_alpha_blend_factor: vk::BlendFactor::ONE_MINUS_SRC_ALPHA,
        alpha_blend_op: vk::BlendOp::ADD,
        color_write_mask: vk::ColorComponentFlags::R
            | vk::ColorComponentFlags::G
            | vk::ColorComponentFlags::B
            | vk::ColorComponentFlags::A,
    }
}

/// 按**片元入口名**建一条管线（模糊 h / 模糊 v / 合成）。
///
/// 与 [`create_graphics_pipeline`] 的差别只有两处：入口名可指定、混合状态
/// 可指定 —— 其余（顶点布局、混合模式表）完全共用，所以走同一条创建路径。
#[allow(clippy::too_many_arguments)]
fn create_entry_pipeline(
    device: &ash::Device,
    samples: u32,
    layout: vk::PipelineLayout,
    render_pass: vk::RenderPass,
    fs_entry: &str,
    blend: vk::PipelineColorBlendAttachmentState,
) -> Result<vk::Pipeline, SurfaceError> {
    let spv = include_bytes!(concat!(env!("OUT_DIR"), "/zephyra.spv"));
    let code = parse_spv_entry(spv, fs_entry)?;
    let module = create_shader_module(device, &code)?;
    let vs_entry = std::ffi::CString::new("vs_main").unwrap();
    let fs_name = std::ffi::CString::new(fs_entry).unwrap();
    let vs_stage = vk::PipelineShaderStageCreateInfo::builder()
        .stage(vk::ShaderStageFlags::VERTEX)
        .module(module)
        .name(vs_entry.as_c_str())
        .build();
    let fs_stage = vk::PipelineShaderStageCreateInfo::builder()
        .stage(vk::ShaderStageFlags::FRAGMENT)
        .module(module)
        .name(fs_name.as_c_str())
        .build();

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
    let pipeline =
        unsafe { device.create_graphics_pipelines(vk::PipelineCache::null(), &infos, None) }
            .map_err(|e| SurfaceError::new("create entry pipeline", format!("{:?}", e)))?
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
        DstOver => (vk::BlendFactor::ONE_MINUS_DST_ALPHA, vk::BlendFactor::ONE),
        SrcIn => (vk::BlendFactor::DST_ALPHA, vk::BlendFactor::ZERO),
        DstIn => (vk::BlendFactor::ZERO, vk::BlendFactor::SRC_ALPHA),
        SrcOut => (vk::BlendFactor::ONE_MINUS_DST_ALPHA, vk::BlendFactor::ZERO),
        DstOut => (vk::BlendFactor::ZERO, vk::BlendFactor::ONE_MINUS_SRC_ALPHA),
        // ⚠ 变体名是 `SrcATop` / `DstATop` / `Modulate`（大写 AT、`Modulate`
        // 不是 `Multiply`）。写成 `SrcAtop`/`Multiply` 编译期就报「无此变体」，
        // 但 `cargo clippy --fix` 会把它当「不可达 arm」改成通配 `_` —— 症状是
        // 这三个模式静默退化成默认 SrcOver，画面错而一切正常。
        SrcATop => (
            vk::BlendFactor::DST_ALPHA,
            vk::BlendFactor::ONE_MINUS_SRC_ALPHA,
        ),
        DstATop => (
            vk::BlendFactor::ONE_MINUS_DST_ALPHA,
            vk::BlendFactor::SRC_ALPHA,
        ),
        Xor => (
            vk::BlendFactor::ONE_MINUS_DST_ALPHA,
            vk::BlendFactor::ONE_MINUS_SRC_ALPHA,
        ),
        Plus => (vk::BlendFactor::ONE, vk::BlendFactor::ONE),
        // `r = s·d`：源色直接乘目标色。
        Modulate => (vk::BlendFactor::DST_COLOR, vk::BlendFactor::ZERO),
        Screen => (vk::BlendFactor::ONE, vk::BlendFactor::ONE_MINUS_SRC_COLOR),
        // 其余（14 档 PS 变体 + 任何未来变体）：默认 SrcOver，调用方另作计数。
        _ => (vk::BlendFactor::ONE, vk::BlendFactor::ONE_MINUS_SRC_ALPHA),
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
///
/// # `final_layout` 为什么是形参
///
/// MSAA 主通道的终点是 `PRESENT_SRC_KHR`（交换链图），离屏模糊趟的终点是
/// `COLOR_ATTACHMENT_OPTIMAL`（RT）—— 两者不能共用一条通道：`PRESENT_SRC`
/// 只对**带 present usage 的交换链图**合法，拿它去转一张 RT 的布局是非法转换
/// （llvmpipe 上未必报错，换硬件或开验证层就会）。所以终点布局按用途传。
fn create_render_pass(
    device: &ash::Device,
    format: vk::Format,
    samples: u32,
    final_layout: vk::ImageLayout,
) -> Result<vk::RenderPass, SurfaceError> {
    // MSAA 时是「目标 + resolve」两附件：resolve 由硬件在通道结束时自动完成，
    // 等价 wgpu 版「画进 MSAA、resolve 进 surface」—— 也就是没有额外 blit。
    // 单采样时一附件、直接画进交换链图。
    let mut attachments = Vec::new();
    let mut color_refs = Vec::new();
    let mut resolve_refs = Vec::new();
    if samples > 1 {
        attachments.push(
            vk::AttachmentDescription::builder()
                .format(format)
                .samples(vk::SampleCountFlags::from_raw(samples))
                // ⚠ LOAD：离屏段让一帧分成多趟主通道，后一趟必须能续画前面的内容。
                .load_op(vk::AttachmentLoadOp::LOAD)
                // ⚠ STORE：同理，DONT_CARE 会把本帧后续段要用的内容丢掉。
                .store_op(vk::AttachmentStoreOp::STORE)
                .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
                .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
                // 首趟的清空由 begin_main_pass 的 cmd_clear_attachments 做，所以
                // 进 COLOR_ATTACHMENT 即可（UNDEFINED+LOAD 也合法，白载一遍垃圾）。
                .initial_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .final_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .build(),
        );
        attachments.push(
            vk::AttachmentDescription::builder()
                .format(format)
                .samples(vk::SampleCountFlags::TYPE_1)
                // DONT_CARE + UNDEFINED 是对的：每趟末都把 MSAA **全部**内容
                // resolve 进它（整块覆盖），从不需要载入上一趟的任何东西。
                .load_op(vk::AttachmentLoadOp::DONT_CARE)
                .store_op(vk::AttachmentStoreOp::STORE)
                .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
                .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
                .initial_layout(vk::ImageLayout::UNDEFINED)
                .final_layout(final_layout)
                .build(),
        );
        color_refs.push(
            vk::AttachmentReference::builder()
                .attachment(0)
                .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .build(),
        );
        resolve_refs.push(
            vk::AttachmentReference::builder()
                .attachment(1)
                .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .build(),
        );
    } else {
        attachments.push(
            vk::AttachmentDescription::builder()
                .format(format)
                .samples(vk::SampleCountFlags::TYPE_1)
                .load_op(vk::AttachmentLoadOp::CLEAR)
                .store_op(vk::AttachmentStoreOp::STORE)
                .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
                .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
                .initial_layout(vk::ImageLayout::UNDEFINED)
                .final_layout(final_layout)
                .build(),
        );
        color_refs.push(
            vk::AttachmentReference::builder()
                .attachment(0)
                .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .build(),
        );
    }

    let subpass = vk::SubpassDescription::builder()
        .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
        .color_attachments(&color_refs)
        .resolve_attachments(if samples > 1 { &resolve_refs } else { &[] })
        .build();

    let info = vk::RenderPassCreateInfo::builder()
        .attachments(&attachments)
        .subpasses(std::slice::from_ref(&subpass))
        .build();

    unsafe { device.create_render_pass(&info, None) }
        .map_err(|e| SurfaceError::new("render pass", e.to_string()))
}

/// 建着色器模块。
fn create_shader_module(
    device: &ash::Device,
    code: &[u32],
) -> Result<vk::ShaderModule, SurfaceError> {
    unsafe { device.create_shader_module(&vk::ShaderModuleCreateInfo::builder().code(code), None) }
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
    if !spv.len().is_multiple_of(4) {
        return Err(SurfaceError::new("SPIR-V", "length not multiple of 4"));
    }
    let words = spv.len() / 4;
    let mut out = Vec::with_capacity(words);
    for i in 0..words {
        let b = [spv[i * 4], spv[i * 4 + 1], spv[i * 4 + 2], spv[i * 4 + 3]];
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
    unsafe { device.bind_buffer_memory(buffer, mem, 0) }
        .map_err(|e| SurfaceError::new("bind buffer memory", e.to_string()))?;
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
    samples: vk::SampleCountFlags,
    usage: vk::ImageUsageFlags,
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
                .samples(samples)
                .tiling(vk::ImageTiling::OPTIMAL)
                .usage(usage)
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
    unsafe { device.bind_image_memory(image, mem, 0) }
        .map_err(|e| SurfaceError::new("bind image memory", e.to_string()))?;
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

/// 建一张单采样离屏目标（既是颜色附件、又要被采样）。
///
/// # ⚠ `SAMPLED` 少一个就是崩
///
/// - `COLOR_ATTACHMENT`：模糊趟与「内容」趟画进它；
/// - `SAMPLED`：模糊趟把「另一张 RT」绑进描述符采样，合成趟采样末趟输出。
///
/// 实测：只给 `COLOR_ATTACHMENT` 时 llvmpipe **不报错、直接 segfault**
/// （frame 2 core dump，无 backtrace）。所以这里的 usage 位不是「按需精简」，
/// 而是「少一个就是崩」—— 本函数没有按调用点分档的余地。
/// （`TRANSFER_DST` 是为将来可能的 resolve 预留的；当前没有调用点用它。）
fn create_rt(device: &ash::Device, gpu: &Gpu, size: (u32, u32)) -> Option<Rt> {
    let (image, view, memory) = create_image_texture(
        device,
        gpu,
        size.0.max(1),
        size.1.max(1),
        gpu.format,
        vk::SampleCountFlags::TYPE_1,
        vk::ImageUsageFlags::COLOR_ATTACHMENT
            | vk::ImageUsageFlags::SAMPLED
            | vk::ImageUsageFlags::TRANSFER_DST,
    )
    .ok()?;
    Some(Rt {
        image,
        view,
        memory,
        size,
    })
}

/// 给单采样 RT 建一个帧缓冲（对单采样 renderpass）。
fn create_rt_framebuffer(
    device: &ash::Device,
    render_pass: vk::RenderPass,
    view: vk::ImageView,
    size: (u32, u32),
) -> vk::Framebuffer {
    let attachments = [view];
    unsafe {
        device
            .create_framebuffer(
                &vk::FramebufferCreateInfo::builder()
                    .render_pass(render_pass)
                    .attachments(&attachments)
                    .width(size.0.max(1))
                    .height(size.1.max(1))
                    .layers(1)
                    .build(),
                None,
            )
            .expect("rt framebuffer")
    }
}

/// 经暂存缓冲把像素上传到一张图像，并把布局转到 SHADER_READ_ONLY。
///
/// 一次性命令 + 等 fence —— 图集/纹理只在 revision 变化时重传（帧循环之外），
/// 同步等待的开销是可接受的；把它们挪进帧命令缓冲只会让时序复杂化。
fn upload_image(
    gpu: &Gpu,
    image: vk::Image,
    pixels: &[u8],
    (w, h): (u32, u32),
) -> Result<(), SurfaceError> {
    let device = &gpu.device;
    let byte_len = pixels.len() as vk::DeviceSize;
    let buffer = unsafe {
        device.create_buffer(
            &vk::BufferCreateInfo::builder()
                .size(byte_len)
                .usage(vk::BufferUsageFlags::TRANSFER_SRC)
                .sharing_mode(vk::SharingMode::EXCLUSIVE)
                .build(),
            None,
        )
    }
    .map_err(|e| SurfaceError::new("staging buffer", e.to_string()))?;
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
    .map_err(|e| SurfaceError::new("staging memory", e.to_string()))?;
    unsafe { device.bind_buffer_memory(buffer, mem, 0) }
        .map_err(|e| SurfaceError::new("bind staging", e.to_string()))?;
    unsafe {
        let ptr = device
            .map_memory(mem, 0, byte_len, vk::MemoryMapFlags::empty())
            .map_err(|e| SurfaceError::new("map staging", e.to_string()))?;
        std::ptr::copy_nonoverlapping(pixels.as_ptr(), ptr as *mut u8, pixels.len());
        device.unmap_memory(mem);
    }
    let cmd = unsafe {
        device.allocate_command_buffers(
            &vk::CommandBufferAllocateInfo::builder()
                .command_pool(gpu.command_pool)
                .level(vk::CommandBufferLevel::PRIMARY)
                .command_buffer_count(1)
                .build(),
        )
    }
    .map_err(|e| SurfaceError::new("upload cmd", e.to_string()))?[0];
    let range = vk::ImageSubresourceRange {
        aspect_mask: vk::ImageAspectFlags::COLOR,
        base_mip_level: 0,
        level_count: 1,
        base_array_layer: 0,
        layer_count: 1,
    };
    let to_dst = vk::ImageMemoryBarrier::builder()
        .src_access_mask(vk::AccessFlags::empty())
        .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE)
        .old_layout(vk::ImageLayout::UNDEFINED)
        .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .image(image)
        .subresource_range(range)
        .build();
    let to_read = vk::ImageMemoryBarrier::builder()
        .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
        .dst_access_mask(vk::AccessFlags::SHADER_READ)
        .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
        .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .image(image)
        .subresource_range(range)
        .build();
    let regions = [vk::BufferImageCopy::builder()
        .image_subresource(
            vk::ImageSubresourceLayers::builder()
                .aspect_mask(vk::ImageAspectFlags::COLOR)
                .base_array_layer(0)
                .layer_count(1)
                .build(),
        )
        .image_extent(vk::Extent3D {
            width: w,
            height: h,
            depth: 1,
        })
        .build()];
    unsafe {
        device
            .begin_command_buffer(
                cmd,
                &vk::CommandBufferBeginInfo::builder()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT)
                    .build(),
            )
            .map_err(|e| SurfaceError::new("begin upload cmd", e.to_string()))?;
        let dsts = [to_dst];
        device.cmd_pipeline_barrier(
            cmd,
            vk::PipelineStageFlags::TOP_OF_PIPE,
            vk::PipelineStageFlags::TRANSFER,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &dsts,
        );
        device.cmd_copy_buffer_to_image(
            cmd,
            buffer,
            image,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            &regions,
        );
        let reads = [to_read];
        device.cmd_pipeline_barrier(
            cmd,
            vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::FRAGMENT_SHADER,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &reads,
        );
        device
            .end_command_buffer(cmd)
            .map_err(|e| SurfaceError::new("end upload cmd", e.to_string()))?;
        let fence = device
            .create_fence(&vk::FenceCreateInfo::default(), None)
            .map_err(|e| SurfaceError::new("upload fence", e.to_string()))?;
        let cmds = [cmd];
        device
            .queue_submit(
                gpu.queue,
                &[vk::SubmitInfo::builder().command_buffers(&cmds).build()],
                fence,
            )
            .map_err(|e| SurfaceError::new("submit upload", e.to_string()))?;
        device
            .wait_for_fences(&[fence], true, u64::MAX)
            .map_err(|e| SurfaceError::new("wait upload", e.to_string()))?;
        device.destroy_fence(fence, None);
        device.free_command_buffers(gpu.command_pool, &cmds);
        device.destroy_buffer(buffer, None);
        device.free_memory(mem, None);
    }
    Ok(())
}

// offset_of! 宏（stable Rust 1.77+ 有 std::mem::offset_of!）。
use std::mem::offset_of;

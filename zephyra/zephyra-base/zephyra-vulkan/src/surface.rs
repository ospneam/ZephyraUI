//! Instance / Device / Queue / Surface 的生命周期 —— 对应 Windows 那边
//! `zephyra-winit-d3d12::surface` 的角色，但用**裸 Vulkan(ash)** 实现。
//!
//! # 与 wgpu 版的差异
//!
//! wgpu 版（`zephyra-winit-vk::surface`）把设备+队列+swapchain 合在一个 `Gpu`
//! 结构里，因为 wgpu 的 `Device`/`Queue` 是不可分割的一对。裸 Vulkan 这边也是
//! 一个 `Gpu` 结构 —— 但内部是 `ash::Instance` + `ash::Device` + `vk::Queue`
//! + `VkSurfaceKHR` + `VkSwapchainKHR`。
//!
//! # 为什么不用 Xlib surface
//!
//! 实测（2026-10，WSL2 Ubuntu 24.04 + WSLg）：
//!   - Wayland surface 可用（winit 默认走 Wayland，`libxkbcommon.so.0` 系统自带）；
//!   - X11 surface 需要 `libxkbcommon-x11.so.0`，系统没有、装它要 sudo（无免密 sudo）。
//! 所以本 crate 走 **Wayland surface** —— 这是 WSLg 的原生合成路径，少一层
//! XWayland 转译，对「输出 1:1」这件事少一处缩放/色彩分叉点。
//!
//! # Wayland 特有的两个坑
//!
//! 1. `current_extent` 在 Wayland 上报 `u32::MAX`（"尺寸未定义，由 swapchain 定"）——
//!    规范允许，extent 必须由应用给（用 `window.inner_size()`）。
//! 2. surface 指针来自 winit 自己的连接 —— surface 必须挂在**创建窗口的那个连接**上。

use std::fmt;

use ash::vk;
use winit::window::Window;

use zephyra_fx::pixels::{ChannelOrder, PixelRead};

/// 渲染失败。**不带**平台错误码回码 —— 回码属于 `docs/dev-log/`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfaceError {
    what: &'static str,
    detail: String,
}

impl SurfaceError {
    pub fn new(what: &'static str, detail: impl Into<String>) -> Self {
        Self {
            what,
            detail: detail.into(),
        }
    }
}

impl fmt::Display for SurfaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.what, self.detail)
    }
}

impl std::error::Error for SurfaceError {}

/// 设备 + 队列 + surface + swapchain。全进程唯一一份。
pub struct Gpu {
    // ⚠⚠ **Entry 必须随 Gpu 活着**：ash::Entry 持有 libloading::Library（dlopen 的
    // libvulkan 句柄），drop 时 dlclose —— 之后 Instance/Device 函数表里的所有
    // 指针都指向已卸载的代码。症状不是「第一个调用就崩」而是**随机崩**：
    // Linux 的延迟卸载让前面的调用碰巧成功，直到某一次踩到被回收的内存。
    // 本机实测：demo 在「建顶点缓冲的 vkAllocateMemory」处 segfault，而它之前的
    // 五个调用全部成功 —— 与「库已卸载但页还在」的行为完全吻合。
    //
    // （探针不崩的原因：entry 在 main 里，活到进程结束。）
    #[allow(dead_code, reason = "只为持有 libvulkan 的 dlopen 句柄")]
    entry: ash::Entry,
    pub(crate) instance: ash::Instance,
    pub(crate) device: ash::Device,
    pub(crate) queue: vk::Queue,
    pub(crate) physical: vk::PhysicalDevice,
    pub(crate) queue_family: u32,
    pub(crate) khr_surface: ash::extensions::khr::Surface,
    pub(crate) khr_wayland: ash::extensions::khr::WaylandSurface,
    pub(crate) khr_swapchain: ash::extensions::khr::Swapchain,
    pub(crate) surface: vk::SurfaceKHR,
    pub(crate) swapchain: vk::SwapchainKHR,
    pub(crate) images: Vec<vk::Image>,
    pub(crate) format: vk::Format,
    pub(crate) extent: (u32, u32),
    pub(crate) present_mode: vk::PresentModeKHR,
    pub(crate) msaa_samples: vk::SampleCountFlags,
    pub(crate) device_dead: bool,
    /// 一次性命令池（读回等）。与 Gpu 同寿命。
    pub(crate) command_pool: vk::CommandPool,
}

impl fmt::Debug for Gpu {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Gpu")
            .field("format", &self.format)
            .field("extent", &self.extent)
            .field("dead", &self.device_dead)
            .finish()
    }
}

impl Gpu {
    /// 建设备、队列与 surface。
    pub fn new(window: &Window) -> Result<Self, SurfaceError> {
        // ── instance ──
        let entry = unsafe { ash::Entry::load() }
            .map_err(|e| SurfaceError::new("dlopen libvulkan", e.to_string()))?;
        let app_name = std::ffi::CString::new("zephyra-vulkan").unwrap();
        let ext_names = [
            std::ffi::CString::new("VK_KHR_surface").unwrap(),
            std::ffi::CString::new("VK_KHR_wayland_surface").unwrap(),
        ];
        let instance = unsafe {
            entry.create_instance(
                &vk::InstanceCreateInfo::builder()
                    .application_info(
                        &vk::ApplicationInfo::builder()
                            .application_name(app_name.as_c_str())
                            .application_version(vk::make_api_version(0, 1, 0, 0))
                            .api_version(vk::API_VERSION_1_1)
                            .build(),
                    )
                    .enabled_extension_names(&[
                        ext_names[0].as_ptr(),
                        ext_names[1].as_ptr(),
                    ])
                    .build(),
                None,
            )
        }
        .map_err(|e| SurfaceError::new("vkCreateInstance", e.to_string()))?;

        // ── Wayland surface ──
        let khr_surface = ash::extensions::khr::Surface::new(&entry, &instance);
        let khr_wayland = ash::extensions::khr::WaylandSurface::new(&entry, &instance);
        let (dpy, wl_surface) = wayland_handles(window)
            .map_err(|e| SurfaceError::new("wayland handles", e))?;
        let surface = unsafe {
            khr_wayland.create_wayland_surface(
                &vk::WaylandSurfaceCreateInfoKHR::builder()
                    .display(dpy)
                    .surface(wl_surface)
                    .build(),
                None,
            )
        }
        .map_err(|e| SurfaceError::new("vkCreateWaylandSurfaceKHR", e.to_string()))?;

        // ── 物理设备 + 队列 ──
        let physical = unsafe { instance.enumerate_physical_devices() }
            .map_err(|e| SurfaceError::new("enumerate_physical_devices", e.to_string()))?
            .into_iter()
            .find(|p| {
                unsafe { instance.get_physical_device_queue_family_properties(*p) }
                    .iter()
                    .any(|f| f.queue_flags.contains(vk::QueueFlags::GRAPHICS) && f.queue_count > 0)
            })
            .ok_or_else(|| SurfaceError::new("no physical device", "no graphics queue"))?;
        let queue_family = unsafe {
            instance.get_physical_device_queue_family_properties(physical)
        }
        .iter()
        .position(|f| f.queue_flags.contains(vk::QueueFlags::GRAPHICS) && f.queue_count > 0)
        .unwrap() as u32;
        let swapchain_name = std::ffi::CString::new("VK_KHR_swapchain").unwrap();
        let device = unsafe {
            instance.create_device(
                physical,
                &vk::DeviceCreateInfo::builder()
                    .queue_create_infos(&[vk::DeviceQueueCreateInfo::builder()
                        .queue_family_index(queue_family)
                        .queue_priorities(&[1.0])
                        .build()])
                    .enabled_extension_names(&[swapchain_name.as_ptr()])
                    .build(),
                None,
            )
        }
        .map_err(|e| SurfaceError::new("vkCreateDevice", e.to_string()))?;
        let queue = unsafe { device.get_device_queue(queue_family, 0) };

        // ── swapchain ──
        // MSAA 由环境变量 ZEPHYRA_MSAA 控制（与两个 Windows 后端同一约定）。
        let msaa_samples = if std::env::var("ZEPHYRA_MSAA").as_deref() == Ok("0") {
            vk::SampleCountFlags::TYPE_1
        } else {
            vk::SampleCountFlags::TYPE_4
        };
        let caps = unsafe { khr_surface.get_physical_device_surface_capabilities(physical, surface) }
            .map_err(|e| SurfaceError::new("surface capabilities", e.to_string()))?;
        let (w, h) = if caps.current_extent.width == u32::MAX {
            let s = window.inner_size();
            (s.width, s.height)
        } else {
            (caps.current_extent.width, caps.current_extent.height)
        };
        if w == 0 || h == 0 {
            return Err(SurfaceError::new("surface extent", format!("{w}x{h}")));
        }
        let formats = unsafe { khr_surface.get_physical_device_surface_formats(physical, surface) }
            .map_err(|e| SurfaceError::new("surface formats", e.to_string()))?;
        let format = formats
            .iter()
            .map(|f| f.format)
            .find(|f| *f == vk::Format::B8G8R8A8_SRGB)
            .or_else(|| {
                formats
                    .iter()
                    .map(|f| f.format)
                    .find(|f| *f == vk::Format::R8G8B8A8_UNORM)
            })
            .ok_or_else(|| SurfaceError::new("surface format", "no supported format"))?;
        if !caps.supported_usage_flags.contains(vk::ImageUsageFlags::TRANSFER_SRC) {
            return Err(SurfaceError::new(
                "swapchain usage",
                "TRANSFER_SRC not supported (needed for frame dump)",
            ));
        }
        let alpha = if caps
            .supported_composite_alpha
            .contains(vk::CompositeAlphaFlagsKHR::PRE_MULTIPLIED)
        {
            vk::CompositeAlphaFlagsKHR::PRE_MULTIPLIED
        } else if caps
            .supported_composite_alpha
            .contains(vk::CompositeAlphaFlagsKHR::OPAQUE)
        {
            vk::CompositeAlphaFlagsKHR::OPAQUE
        } else {
            vk::CompositeAlphaFlagsKHR::INHERIT
        };
        let khr_swapchain = ash::extensions::khr::Swapchain::new(&instance, &device);
        let swapchain = unsafe {
            khr_swapchain.create_swapchain(
                &vk::SwapchainCreateInfoKHR::builder()
                    .surface(surface)
                    .min_image_count(caps.min_image_count.max(2))
                    .image_format(format)
                    .image_color_space(vk::ColorSpaceKHR::SRGB_NONLINEAR)
                    .image_extent(vk::Extent2D { width: w, height: h })
                    .image_array_layers(1)
                    .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC)
                    .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
                    .pre_transform(caps.current_transform)
                    .composite_alpha(alpha)
                    .present_mode(vk::PresentModeKHR::FIFO)
                    .build(),
                None,
            )
        }
        .map_err(|e| SurfaceError::new("vkCreateSwapchainKHR", e.to_string()))?;
        let images = unsafe { khr_swapchain.get_swapchain_images(swapchain) }
            .map_err(|e| SurfaceError::new("get_swapchain_images", e.to_string()))?;

        // 命令池（读回等一次性命令用）。
        let command_pool = unsafe {
            device.create_command_pool(
                &vk::CommandPoolCreateInfo::builder()
                    .queue_family_index(queue_family)
                    .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER)
                    .build(),
                None,
            )
        }
        .map_err(|e| SurfaceError::new("command pool", e.to_string()))?;

        // 存一份 mem_props 给离屏纹理/回读用。
        let _ = unsafe { instance.get_physical_device_memory_properties(physical) };

        Ok(Self {
            entry,
            instance,
            device,
            queue,
            physical,
            queue_family,
            khr_surface,
            khr_wayland,
            khr_swapchain,
            surface,
            swapchain,
            images,
            format,
            extent: (w, h),
            present_mode: vk::PresentModeKHR::FIFO,
            msaa_samples,
            device_dead: false,
            command_pool,
        })
    }

    /// 重新创建 swapchain（窗口 resize 后调用）。
    pub fn resize(&mut self, window: &Window) -> Result<(), SurfaceError> {
        // 销毁旧 swapchain。
        unsafe {
            self.khr_swapchain.destroy_swapchain(self.swapchain, None);
        }
        let (w, h) = {
            let s = window.inner_size();
            (s.width, s.height)
        };
        if w == 0 || h == 0 {
            return Ok(());
        }
        let caps = unsafe {
            self.khr_surface
                .get_physical_device_surface_capabilities(self.physical, self.surface)
        }
        .map_err(|e| SurfaceError::new("surface capabilities", e.to_string()))?;
        let (w, h) = if caps.current_extent.width == u32::MAX {
            (w, h)
        } else {
            (caps.current_extent.width, caps.current_extent.height)
        };
        self.extent = (w, h);
        self.swapchain = unsafe {
            self.khr_swapchain.create_swapchain(
                &vk::SwapchainCreateInfoKHR::builder()
                    .surface(self.surface)
                    .min_image_count(caps.min_image_count.max(2))
                    .image_format(self.format)
                    .image_color_space(vk::ColorSpaceKHR::SRGB_NONLINEAR)
                    .image_extent(vk::Extent2D { width: w, height: h })
                    .image_array_layers(1)
                    .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC)
                    .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
                    .pre_transform(caps.current_transform)
                    .composite_alpha(caps.supported_composite_alpha)
                    .present_mode(self.present_mode)
                    .build(),
                None,
            )
        }
        .map_err(|e| SurfaceError::new("vkCreateSwapchainKHR", e.to_string()))?;
        self.images = unsafe { self.khr_swapchain.get_swapchain_images(self.swapchain) }
            .map_err(|e| SurfaceError::new("get_swapchain_images", e.to_string()))?;
        Ok(())
    }

    /// 取得下一张交换链图并等待 acquire fence。
    pub fn acquire(&self) -> Result<(u32, vk::Semaphore), SurfaceError> {
        let sem = unsafe {
            self.device
                .create_semaphore(&vk::SemaphoreCreateInfo::default(), None)
        }
        .map_err(|e| SurfaceError::new("create semaphore", e.to_string()))?;
        let (idx, _) = unsafe {
            self.khr_swapchain
                .acquire_next_image(self.swapchain, u64::MAX, sem, vk::Fence::null())
        }
        .map_err(|e| SurfaceError::new("acquire_next_image", e.to_string()))?;
        Ok((idx, sem))
    }

    /// 提交命令缓冲并 present。
    pub fn submit_and_present(
        &self,
        cmd: vk::CommandBuffer,
        wait_sem: vk::Semaphore,
        signal_sem: vk::Semaphore,
        image_idx: u32,
        fence: vk::Fence,
    ) -> Result<(), SurfaceError> {
        unsafe {
            self.device.reset_fences(&[fence]).ok();
            self.device
                .queue_submit(
                    self.queue,
                    &[vk::SubmitInfo::builder()
                        .wait_semaphores(&[wait_sem])
                        .wait_dst_stage_mask(&[vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT])
                        .command_buffers(&[cmd])
                        .signal_semaphores(&[signal_sem])
                        .build()],
                    fence,
                )
                .ok();
            self.device.wait_for_fences(&[fence], true, u64::MAX).ok();
            self.khr_swapchain
                .queue_present(
                    self.queue,
                    &vk::PresentInfoKHR::builder()
                        .wait_semaphores(&[signal_sem])
                        .swapchains(&[self.swapchain])
                        .image_indices(&[image_idx])
                        .build(),
                )
                .ok();
        }
        Ok(())
    }

    /// 读回一张交换链图的像素（用于 `ZEPHYRA_FRAME_DUMP`）。
    ///
    /// 与 D3D12 那份「MSAA 再 resolve 一次」同构：这里读的是**交换链图**，
    /// 而它只在 `PRESENT_SRC` 布局下可读 —— 所以先转到 `TRANSFER_SRC` 再拷。
    pub fn read_back(&self, image: vk::Image, w: u32, h: u32) -> Result<Vec<u8>, SurfaceError> {
        let byte_len = (w * h * 4) as vk::DeviceSize;
        let buffer = unsafe {
            self.device.create_buffer(
                &vk::BufferCreateInfo::builder()
                    .size(byte_len)
                    .usage(vk::BufferUsageFlags::TRANSFER_DST)
                    .sharing_mode(vk::SharingMode::EXCLUSIVE)
                    .build(),
                None,
            )
        }
        .map_err(|e| SurfaceError::new("readback buffer", e.to_string()))?;
        let breq = unsafe { self.device.get_buffer_memory_requirements(buffer) };
        let bmem = unsafe {
            self.device.allocate_memory(
                &vk::MemoryAllocateInfo::builder()
                    .allocation_size(breq.size)
                    .memory_type_index(pick_type(
                        &self.instance,
                        self.physical,
                        vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
                    )?)
                    .build(),
                None,
            )
        }
        .map_err(|e| SurfaceError::new("readback memory", e.to_string()))?;
        unsafe { self.device.bind_buffer_memory(buffer, bmem, 0) }.ok();

        let cmd = unsafe {
            self.device.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::builder()
                    .command_pool(self.command_pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(1)
                    .build(),
            )
        }
        .map_err(|e| SurfaceError::new("readback cmd", e.to_string()))?[0];

        unsafe {
            self.device
                .begin_command_buffer(
                    cmd,
                    &vk::CommandBufferBeginInfo::builder()
                        .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT)
                        .build(),
                )
                .ok();
            // swapchain 图当前在 PRESENT_SRC；转到 TRANSFER_SRC 才能拷。
            let bar = vk::ImageMemoryBarrier::builder()
                .src_access_mask(vk::AccessFlags::MEMORY_READ)
                .dst_access_mask(vk::AccessFlags::TRANSFER_READ)
                .old_layout(vk::ImageLayout::PRESENT_SRC_KHR)
                .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(image)
                .subresource_range(vk::ImageSubresourceRange {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    base_mip_level: 0,
                    level_count: 1,
                    base_array_layer: 0,
                    layer_count: 1,
                })
                .build();
            self.device.cmd_pipeline_barrier(
                cmd,
                vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                vk::PipelineStageFlags::TRANSFER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[bar],
            );
            self.device.cmd_copy_image_to_buffer(
                cmd,
                image,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                buffer,
                &[vk::BufferImageCopy::builder()
                    .image_subresource(
                        vk::ImageSubresourceLayers::builder()
                            .aspect_mask(vk::ImageAspectFlags::COLOR)
                            .base_array_layer(0)
                            .layer_count(1)
                            .build(),
                    )
                    .image_extent(vk::Extent3D { width: w, height: h, depth: 1 })
                    .build()],
            );
            self.device.end_command_buffer(cmd).ok();
        }
        let fence = unsafe {
            self.device
                .create_fence(&vk::FenceCreateInfo::default(), None)
        }
        .map_err(|e| SurfaceError::new("readback fence", e.to_string()))?;
        unsafe {
            self.device
                .queue_submit(
                    self.queue,
                    &[vk::SubmitInfo::builder().command_buffers(&[cmd]).build()],
                    fence,
                )
                .ok();
            self.device.wait_for_fences(&[fence], true, u64::MAX).ok();
        }
        let ptr = unsafe { self.device.map_memory(bmem, 0, byte_len, vk::MemoryMapFlags::empty()) }
            .map_err(|e| SurfaceError::new("map_memory", e.to_string()))?;
        let data = unsafe { std::slice::from_raw_parts(ptr as *const u8, byte_len as usize) }.to_vec();
        // 清理（读回是一次性的，每次都建/销）。
        unsafe {
            self.device.unmap_memory(bmem);
            self.device.destroy_fence(fence, None);
            self.device.free_command_buffers(self.command_pool, &[cmd]);
            self.device.destroy_buffer(buffer, None);
            self.device.free_memory(bmem, None);
        }
        Ok(data)
    }

    /// 命令池（读回等一次性命令用）。
    pub(crate) fn command_pool(&self) -> vk::CommandPool {
        self.command_pool
    }
}

fn wayland_handles(window: &Window) -> Result<(*mut std::ffi::c_void, *mut std::ffi::c_void), String> {
    use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
    let dpy = match window.display_handle().map_err(|e| e.to_string())?.as_raw() {
        RawDisplayHandle::Wayland(h) => h.display.as_ptr(),
        _ => return Err("winit did not give a Wayland display handle".into()),
    };
    let surface = match window.window_handle().map_err(|e| e.to_string())?.as_raw() {
        RawWindowHandle::Wayland(h) => h.surface.as_ptr(),
        _ => return Err("winit did not give a Wayland surface handle".into()),
    };
    Ok((dpy, surface))
}

pub(crate) fn pick_type(
    instance: &ash::Instance,
    physical: vk::PhysicalDevice,
    want: vk::MemoryPropertyFlags,
) -> Result<u32, SurfaceError> {
    let props = unsafe { instance.get_physical_device_memory_properties(physical) };
    (0..props.memory_type_count)
        .find(|i| props.memory_types[*i as usize].property_flags.contains(want))
        .ok_or_else(|| SurfaceError::new("memory type", format!("no type with {want:?}")))
}
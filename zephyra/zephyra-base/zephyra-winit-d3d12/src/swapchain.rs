//! D3D12 交换链：后缓冲、渲染目标视图、呈现、resize。
//!
//! # 为什么用 composition swapchain 而不是 `CreateSwapChainForHwnd`
//!
//! `zephyra-dcomp` 的实测结论已经写在那里：`CreateSwapChainForComposition` 的
//! `pDevice` 接受 **command queue 的 `IUnknown`**，而 `D3D12_COMMAND_QUEUE` 正是
//! 本 crate 手里有的东西。走这条路与 D3D11 后端（`zephyra-winit`）**用同一个
//! `zephyra-dcomp` crate**，两个后端的呈现参数因此天然一致。
//!
//! # 关键约束：后缓冲的 RTV 视图在 `ResizeBuffers` 后**全部失效**
//!
//! `ResizeBuffers` 会让交换链内部重建后缓冲，之前建的 `ID3D12Resource` 与
//! `CreateRenderTargetView` 得到的描述符都指向**已释放的内存**。继续用旧 RTV 的
//! 症状是「窗口变成一块固定颜色」或直接崩 —— 而且没有任何 HRESULT。
//!
//! 所以 [`Swapchain::resize`] 里必须重建全部后缓冲引用与 RTV；而 RTV **描述符堆**
//! 不受影响（它不属于交换链），所以堆只建一次。
//!
//! # 为什么 `Swapchain` 不持有窗口
//!
//! composition swapchain 不需要 `HWND` —— 它挂进 DComp 视觉树，由系统合成器上屏。
//! 窗口与 DComp 树由宿主持有，且**必须比交换链活得久**（`zephyra-dcomp` 硬约束 #1）。
//! 本结构因此可以在没有窗口的测试里建起来并跑完整条呈现链路 —— 那正是
//! `tests/swapchain_draw.rs` 做的事。

use windows::core::Interface;
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_R8G8B8A8_UNORM,
};
use windows::Win32::Graphics::Dxgi::{IDXGISwapChain1, IDXGISwapChain3};

use zephyra_dcomp::{create_composition_swapchain, present_one, SwapchainDesc};

use crate::surface::{DescriptorHeap, Gpu, SurfaceError};

/// 问设备到底坏没坏。返回 `Some(reason)` = 坏，`None` = 还活着。
///
/// # 为什么不直接用调用点的 HRESULT
///
/// `Present` / `ResizeBuffers` 回的 `DXGI_ERROR_DEVICE_REMOVED`（0x887A0005）是
/// **通用**码。同一个码既可能是「GPU 真的 TDR 了」（必须重建整个设备），也可能
/// 只是「窗口刚最小化、队列暂时打满」（下一帧就好）。官方指定要再调
/// `GetDeviceRemovedReason()` 才能分辨 —— 用错那一侧的代价都很大：
///
/// - 把可恢复的抖动判成死 ⇒ 窗口**永久黑屏**；
/// - 把真的 TDR 判成抖动 ⇒ 每帧重试一个已经不能用的设备 = 用户看到的
///   「一直闪 + 错误刷屏」。
///
/// 这里**只**把明确的设备类错误判成死，其余一律返回 `None`（活着）。
fn device_removed_reason(device: &ID3D12Device) -> Option<u32> {
    // 与 [`Gpu::removed_reason`] 是同一个问题、同一份实现。之所以不直接调那个方法：
    // `present()` 只拿到 `&mut self`，而 `Gpu` 由宿主持有、不在 `Swapchain` 里
    // （存整个 `Gpu` 会让 `Swapchain` 持有它自己的创建者，drop 顺序立刻变成一道
    // 要论证的题）。所以这里只留一份 device 句柄。
    //
    // `Ok(())` = 设备没被移除。`Err(code)` = 移除原因。
    match unsafe { device.GetDeviceRemovedReason() } {
        Ok(()) => None,
        Err(e) => Some(e.code().0 as u32),
    }
}

/// 后缓冲个数。flip model 下至少 2。
pub const BUFFER_COUNT: u32 = 2;

/// MSAA 采样数。
///
/// # 为什么是 4 而不是 8
///
/// 代价在**填充率**（每像素跑 N 遍片元着色器），而 UI 的帧时间几乎全在填充率上。
/// 4× 是「图标边缘不再有肉眼可见台阶」的最低档；8× 在一个 20 像素宽的齿轮上看不出
/// 比 4× 更圆，却要付双倍像素着色开销。
///
/// # 为什么是常量
///
/// 采样数必须在**建 PSO 之前**定下来（它写死在 PSO 的 `SampleDesc` 与光栅化状态的
/// `MultisampleEnable` 里），而 PSO 由 [`crate::renderer::Renderer`] 独立于交换链
/// 构造。若改成"查设备能力再定"，就得把结果从交换链一路传到 PSO 构造 ——
///
/// 而更根本的原因是 D3D12 **没有** `CheckMultisampleQualityLevels`（那是 D3D11 的
/// API）。D3D12 里 `DXGI_SAMPLE_DESC.Quality = 0` 对所有 feature level 都是合法的，
/// `Count` 不支持时失败发生在 `CreateCommittedResource`/PSO 建造时。所以这里选一个
/// **所有 D3D12 设备都支持**的值（4× 对 `R8G8B8A8` 是 feature level 11_0 的底线），
/// 而不是冒"也许某设备不支持"的风险换一个理论上更好的 8×。
pub const MSAA_SAMPLES: u32 = 4;


/// **运行期**的 MSAA 采样数：ZEPHYRA_MSAA=0 关掉多重采样，否则用 MSAA_SAMPLES。
///
/// # 为什么要能关（这是给 GPU 挂掉时做二分用的开关，不是产品选项）
///
/// 本机反复出现 `Present` 报 `DEVICE_REMOVED` 而 `GetDeviceRemovedReason` 说
/// `DEVICE_HUNG`（真 TDR），而**调试层在本机两条路都不可用**：事后
/// `EnableDebugLayer()` 会让设备刚建好就挂；改成建设备时带
/// `D3D12_CREATE_DEVICE_DEBUG` 后应用直接访问违例退出。于是拿不到「哪个 API」
/// 的证据，只能靠**关掉子系统**来二分。
///
/// MSAA 的解析（`Swapchain::resolve_msaa`）与 `barrier_to_present` 的相对顺序是
/// 首要嫌疑之一：resolve 目标必须在 `RENDER_TARGET`，而 MSAA 纹理在
/// [`Swapchain::resize`] 里会被**整体重建**。
///
/// # 为什么不用命令行 flag 而是环境变量
///
/// 后端不解析命令行（那属于应用层）。环境变量在 `Gpu`/宿主初始化时就能读到，
/// 不需要把参数一路传到渲染层。
pub fn msaa_samples() -> u32 {
    match std::env::var("ZEPHYRA_MSAA").as_deref().map(str::trim) {
        Ok("0") | Ok("off") | Ok("false") => 1,
        _ => MSAA_SAMPLES,
    }
}

/// 一张多重采样颜色目标：纹理本体 + 它的渲染目标视图。
///
/// # 为什么 RTV 描述符要与后缓冲**分开**数
///
/// [`Swapchain::heap`] 按 `buffer_count` 个槽建（后缓冲用）。MSAA 目标只要 1 个槽，
/// 所以堆要多给 1 个 —— 见 [`Swapchain::new`]。把 MSAA 的视图塞进后缓冲的槽位
/// 会覆盖某张后缓冲的 RTV，而那张后缓冲随时可能被 resolve 目标选中。
struct MsaaTarget {
    tex: ID3D12Resource,
}

/// 按采样数建多重采样颜色目标。`samples <= 1` 返回 `None`。
fn build_msaa_target(
    device: &ID3D12Device,
    size: (u32, u32),
    samples: u32,
    rtv_slot: D3D12_CPU_DESCRIPTOR_HANDLE,
) -> Result<Option<MsaaTarget>, SurfaceError> {
    if samples <= 1 {
        return Ok(None);
    }
    // ⚠ `ALLOW_RENDER_TARGET` 不是可选项。
    //
    // 实测（本机，`E_INVALIDARG` 0x80070057）：缺了它 `CreateCommittedResource`
    // 直接失败 —— 而失败点在**建纹理**这一步，与采样数无关（1×/2×/4×/8× 全挂），
    // 所以症状看起来像"设备不支持 MSAA"，而真因是 flag。
    //
    // 为什么必须是它：D3D12 允许**不**设这个 flag 的纹理被当渲染目标用，但那样
    // `CreateCommittedResource` 的 clear value 必须给到格式同组（见
    // [`crate::surface::create_resource`] 里的那段）。而 `create_resource` 只在
    // **有这个 flag 时**才给 clear value —— 于是"无 flag"的路径恰好也是
    // "无 clear value"的路径，直接踩 `E_INVALIDARG`。
    //
    // 换句话说：本文件**不能**用 `create_resource` 建 MSAA 纹理，除非先给 flag；
    // 而给了 flag 之后 `create_resource` 就完全够用了（它会顺带补上同格式的
    // clear value）。所以这里直接走 `create_resource` 而不是自己调 API。
    let mut desc = crate::surface::texture_desc(
        (size.0.max(1), size.1.max(1)),
        D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET,
    );
    // ⚠ 格式必须与后缓冲**逐位相同** —— `ResolveSubresource` 要求源与目标格式一致，
    // 而 PSO 的 `RTVFormats[0]` 也必须是它。三者不一致时 D3D12 会在 resolve 处报
    // `E_INVALIDARG`，画面则停在旧内容上。
    desc.Format = DXGI_FORMAT_R8G8B8A8_UNORM;
    desc.SampleDesc = windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC {
        Count: samples,
        Quality: 0,
    };

    // 初始状态给 `RENDER_TARGET`：每帧都是「渲染进它 → resolve 走」，所以状态
    // 转换只发生一次。给 `COMMON` 的话每帧都要在 `PRESENT`↔`RENDER_TARGET`
    // 之间转两次，而那两次转换在 4× 纹理上不便宜。
    let tex = crate::surface::create_resource(
        device,
        &crate::surface::heap_props(D3D12_HEAP_TYPE_DEFAULT),
        &desc,
        D3D12_RESOURCE_STATE_RENDER_TARGET,
    )?;
    unsafe { device.CreateRenderTargetView(&tex, None, rtv_slot) };
    Ok(Some(MsaaTarget { tex }))
}

/// 一个 D3D12 交换链，含已建好的 RTV 描述符。
///
/// # 字段顺序不是随意的
///
/// `rtvs` 里的 `D3D12_CPU_DESCRIPTOR_HANDLE` **不是**引用计数对象，但 `buffers` 里的
/// 后缓冲资源**必须**在本结构存活期间有效 —— 交换链自己持有后缓冲，这里存的是额外
/// 的强引用，好让 `resize` 之后重建视图时不必重新 `GetBuffer`。
pub struct Swapchain {
    /// `IDXGISwapChain3` 用于 [`Self::back_buffer_index`]。
    chain3: IDXGISwapChain3,
    /// `IDXGISwapChain1` 用于挂进 DComp 视觉树与 `Present`。
    ///
    /// 两个接口各存一份 COM 引用，而不是靠 `ptr::from_ref` 在类型间强转 ——
    /// COM 接口的内存布局都是「vtable 指针打头」，所以那种转置**恰好**能用，
    /// 但它是把「能跑」冒充成「正确」，而且 clippy 已经在报它。
    chain1: IDXGISwapChain1,
    /// RTV 描述符堆。`ResizeBuffers` **不影响**它 —— 它属于设备而非交换链。
    ///
    /// 容量是 `buffer_count + 1` —— 多出来的 1 个给 [`Self::msaa`]。用同一个堆
    /// 而不是另建一个：RTV 堆很小，而"两张堆各自 `cpu(i)`"多出一条能走错的路径
    /// （错槽位的症状是渲染目标内容不对而零报错）。
    heap: DescriptorHeap,
    /// 多重采样颜色目标（`samples > 1` 时存在）。
    ///
    /// # 为什么必须有它
    ///
    /// **交换链后缓冲永远是单采样** —— DXGI 不提供让后缓冲多重采样的 API，
    /// `DXGI_SWAP_CHAIN_DESC1` 里也没有采样数字段。正路是自建一张 `samples` 倍纹理、
    /// 渲染进它、每帧末尾 `ResolveSubresource` 解析回后缓冲。
    ///
    /// **两处必须同时改**，只改一处必黑屏：
    ///
    /// - 只建纹理而 PSO 的 `SampleDesc.Count` 还是 1 → D3D12 在
    ///   `OMSetRenderTargets` / `DrawIndexedInstanced` 处拒绝（`render target
    ///   format does not match`）；
    /// - PSO 开了而没建纹理 → 绑单采样 RTV，同样被拒，且画面全错。
    ///
    /// 它必须由本结构持有：尺寸跟着交换链变（[`Self::resize`]），而无人持有时
    /// 它会在建完的下一帧就析构。
    msaa: Option<MsaaTarget>,
    /// 采样数（1 = 无 MSAA）。存下来是为了 resize 时**按同一个值**重建。
    samples: u32,
    /// 每个后缓冲的渲染目标视图。索引即 `GetCurrentBackBufferIndex`。
    views: Vec<ID3D12Resource>,
    /// 后缓冲的**容量**（= 交换链实际尺寸）。
    ///
    /// ⚠ 它**不是**窗口尺寸。窗口尺寸见 [`Self::content`]。
    ///
    /// # 为什么容量与内容要分开
    ///
    /// 本机 `ResizeBuffers` 在后缓冲被用过之后就再也调不成功（实测矩阵见本文件
    /// `resize` 文档）。既然改不了链的尺寸，就让**链一直比窗口大**：窗口缩放时
    /// 什么都不用做 —— 改视口、重新排版、重新画即可，于是**拖动窗口能逐帧跟随**。
    ///
    /// 「比窗口大」不会露馅：DComp 把内容按**自然尺寸 1:1** 画在原点，窗口只显示
    /// 左上角那一块，其余部分在窗口外不可见。没有任何缩放。
    ///
    /// 代价是显存：容量按 25% 余量增长（见 [`Self::resize`]），两缓冲 R8G8B8A8。
    size: (u32, u32),
    /// 本帧要渲染/呈现的**内容尺寸**（= 客户区物理尺寸）。
    ///
    /// 它是视口尺寸：渲染、clear、MSAA 解析都按它来。它**永远** ≤ [`Self::size`]。
    content: (u32, u32),
    buffer_count: u32,
    /// 设备本体的一份句柄。只为在 `present` / `resize` 失败时能问
    /// [`device_removed_reason`] —— 它与构造时那个 `gpu.device` 是同一个对象，
    /// 但 `present()` 只拿到 `&mut self`，拿不到 `&Gpu`。
    ///
    /// 不把整个 [`Gpu`] 存进来：那会让 `Swapchain` 持有它自己的创建者，drop
    /// 顺序立刻变成一道要论证的题。
    gpu_device: ID3D12Device,
    /// 本链是**新建**的（`recreate` 刚跑过）⇒ 下一次 `present` 前要先等 GPU。
    ///
    /// 见 [`Swapchain::present`] 里那段 Chromium 依据。拖动期间这里会被反复置位
    /// —— 那正是这个标志存在的理由。
    first_present_flush: bool,    /// 连续两次 `Present` 之后仍未收到新尺寸的次数。
    ///
    /// `Present` 失败时最常见的两个码是 `DXGI_ERROR_DEVICE_REMOVED`（设备没了）与
    /// `DXGI_ERROR_DEVICE_RESET`（设备重建中，**可恢复**）。后者应当重试而不是
    /// 让整个应用退出 —— 一个窗口最小化再恢复就会走这条路。
    consecutive_present_errors: u32,
    /// 上一次 [`Self::resize`] 是否走了「重建整条链」这条路。见该方法文档。
    was_recreated: bool,
    /// 被**新链替换下来**的旧链。必须活到「DComp 重新 `SetContent`」之后才释放。
    ///
    /// # 为什么不能当场析构
    ///
    /// DComp 的 visual 上挂着 SetContent(旧链)。直接 *self = fresh 会当场
    /// 析构旧链，那一瞬 DComp 仍持着一个**已释放**的交换链，下一帧合成就是
    /// use-after-free。
    ///
    /// 实测症状正是这个顺序：日志打「交换链已重建」→ 紧接着 Present 报
    /// 「GPU 设备实例已经暂停」→ GetDeviceRemovedReason 给
    /// DXGI_ERROR_DEVICE_HUNG。
    ///
    /// 官方纪律（Chromium ui/gl/dc_layer_tree.cc:626）：只在内容**变了**时
    /// SetContent，拆除时 SetContent(nullptr) 显式丢掉引用，然后 Commit()。
    /// 本仓的顺序改为：先建新链 → SetContent(新) → Commit → 才放旧链。
    superseded: Option<IDXGISwapChain1>,

    ///
    /// # 为什么必须有这个标志
    ///
    /// `resize` 失败时留下的是一个**半死**状态：`views` 已经 `clear()` 过了
    /// （`ResizeBuffers` 要求先放手），而 `size` 还是旧值。这个组合单独一帧就
    /// 足以产出三连错：
    ///
    /// 1. `sync_swapchain` 发现 `client_size != chain.size()` ⇒ **每帧重试** resize；
    /// 2. `rtv()` / `buffer()` 拿到空 `views` ⇒ 「下标越界：要第 0 个，交换链只有 0 个」；
    /// 3. `present()` 在一个已失效的链上 ⇒ 「GPU 设备实例已经暂停」。
    ///
    /// 而这三条**每帧各打一次**，同时在 DComp 视角下是「一个 Present 不出来的
    /// visual」⇒ 窗口一直闪。
    ///
    /// 加了标志之后：宿主见到它就直接跳过绘制与呈现，错误只打一次。
    device_dead: Option<u32>,
}

impl Swapchain {
    /// 后缓冲容量相对内容尺寸的**余量**（0.25 = 长宽各多 25%）。
    ///
    /// 它的作用是把「重建次数」从「每帧一次」压到「随窗口增长对数级」：
    /// 800 → 1000 → 1250 → 1563 → 1954 → 2443，从 800px 拖到 2000px 只重建 4 次。
    ///
    /// 取 25% 而不是更大，是因为显存按 `w × h` **线性**涨：余量 50% 要多占 2.25 倍
    /// 显存，却只把重建次数从 ~4 降到 ~3。
    const CAPACITY_HEADROOM: f64 = 0.25;

    /// 建一个 composition swapchain。
    ///
    /// `gpu.queue` 提供 `IUnknown`（`CreateSwapChainForComposition` 要的就是它）。
    ///
    /// `samples` 是 MSAA 采样数（1 = 关）。**必须**与 PSO 的 `SampleDesc.Count`
    /// 一致 —— 宿主在建 PSO 前用 [`Self::samples`] 取同一个值。
    pub fn new(
        gpu: &Gpu,
        size: (u32, u32),
        desc: &SwapchainDesc,
        samples: u32,
    ) -> Result<Self, SurfaceError> {
        let unknown: windows::core::IUnknown = gpu
            .queue
            .cast()
            .map_err(|e| SurfaceError::new("command queue QI 成 IUnknown", e))?;
        let chain1 = create_composition_swapchain(&unknown, size, desc)
            .map_err(|e| SurfaceError::new("CreateSwapChainForComposition", e))?;
        let chain3: IDXGISwapChain3 = chain1
            .cast()
            .map_err(|e| SurfaceError::new("swapchain QI 成 IDXGISwapChain3", e))?;

        let buffer_count = desc.buffer_count.max(BUFFER_COUNT);
        let mut me = Self {
            chain3,
            chain1,            // ⚠ 容量 +1：后缓冲用 `0..buffer_count`，MSAA 用第 `buffer_count` 号。
            heap: DescriptorHeap::new_rtv(&gpu.device, buffer_count + 1)?,
            msaa: None,
            samples,
            views: Vec::new(),
            size,
            buffer_count,
            gpu_device: gpu.device.clone(),
            first_present_flush: false,
            superseded: None,
            consecutive_present_errors: 0,
            content: size,
            was_recreated: false,
            device_dead: None,
        };
        me.rebuild_views(&gpu.device)?;
        me.rebuild_msaa(&gpu.device)?;
        // 刚建出来的链对 DComp 也是「新链」，同样要走一次「等 GPU 再 present」。
        me.first_present_flush = true;
        Ok(me)
    }

    /// 本结构生效的采样数（1 = 无 MSAA）。
    ///
    /// 宿主在建 PSO **之前**调它，把同一个值交给
    /// [`crate::renderer::Renderer::new`] —— 那是"PSO 与渲染目标必须一致"这条
    /// 硬约束的唯一对齐点（顺序不能反：PSO 建好就固定了）。
    /// 当前可绘制的后缓冲资源（与 [`Self::rtv`] 同一资源）。
    ///
    /// # 为什么要把它暴露出去
    ///
    /// D3D12 的 `ID3D12RenderTargetView` **拿不到**背后那个资源（D3D11 的
    /// `GetResource` 在 D3D12 上没有对应物）。而可分离 / 非可分离混合模式要读
    /// `dst`，宿主得先把后缓冲拷进一张可采样纹理 —— 那就需要资源本体。
    pub fn back_buffer(&self) -> Result<ID3D12Resource, SurfaceError> {
        self.buffer(self.back_buffer_index())
    }

    pub fn samples(&self) -> u32 {
        self.samples
    }

    /// MSAA 渲染目标视图。`samples <= 1` 时返回 `Err`（该画到后缓冲上）。
    pub fn msaa_rtv(&self) -> Result<D3D12_CPU_DESCRIPTOR_HANDLE, SurfaceError> {
        if self.msaa.is_none() {
            return Err(SurfaceError::new(
                "MSAA 未启用却来取它的 RTV",
                windows::core::Error::new(
                    windows::Win32::Foundation::E_POINTER,
                    format!("当前采样数是 {}，没有多重采样目标", self.samples),
                ),
            ));
        }
        self.heap.cpu(self.buffer_count)
    }

    /// 把多重采样纹理解析回后缓冲。**必须在 present 之前调。**
    ///
    /// # 目标必须是**本帧绘制的那张**后缓冲
    ///
    /// 索引取 [`Self::back_buffer_index`] —— 与 [`Self::rtv`] 同一个来源。
    /// 两者若不一致（本项目踩过：flip model 下 `GetCurrentBackBufferIndex`
    /// 报的数与实际可建的视图数不同），就会解析进一张不是本帧目标的缓冲，
    /// 症状是撕裂且零 HRESULT。
    ///
    /// # 屏障由调用方负责
    ///
    /// 目标后缓冲必须**已经在 `RENDER_TARGET`** —— D3D12 不为 resolve 自动转换。
    /// 本项目的帧顺序是「`barrier_to_render_target` → 绘制 → resolve →
    /// `barrier_to_present`」，天然满足。
    ///
    /// 源（MSAA 纹理）不需要转换：它从建起来就一直是 `RENDER_TARGET`。
    pub fn resolve_msaa(&self, list: &ID3D12GraphicsCommandList) -> Result<(), SurfaceError> {
        let Some(m) = &self.msaa else {
            return Ok(());
        };
        let dst = self.buffer(self.back_buffer_index())?;
        unsafe {
            list.ResolveSubresource(
                &dst,
                0,
                &m.tex,
                0,
                self.format(),
            );
        }
        Ok(())
    }

    /// 按当前尺寸重建 MSAA 纹理与它的 RTV。
    ///
    /// 在 [`Self::new`] 与 [`Self::resize`] 之后各调一次 —— 尺寸变了必须重建，
    /// 而它不属于交换链，`ResizeBuffers` 不会碰它。
    fn rebuild_msaa(&mut self, device: &ID3D12Device) -> Result<(), SurfaceError> {
        let slot = self.heap.cpu(self.buffer_count)?;
        self.msaa = build_msaa_target(device, self.size, self.samples, slot)?;
        Ok(())
    }

    /// 当前应渲染的后缓冲下标。
    pub fn back_buffer_index(&self) -> u32 {
        // flip model 下这个值在 `0..BufferCount` 内；`ResizeBuffers` 之后仍有效。
        unsafe { self.chain3.GetCurrentBackBufferIndex() }
    }

    /// 第 `i` 个后缓冲的 RTV 描述符。
    ///
    /// 返回**复制**出来的句柄而不是引用 —— 句柄只是 `(ptr, 步长)` 算出来的整数，
    /// 没有任何生命周期管理；真正要活着的是 [`DescriptorHeap`]，它在本结构里。
    pub fn rtv(&self, i: u32) -> Result<D3D12_CPU_DESCRIPTOR_HANDLE, SurfaceError> {
        self.heap.cpu(i)
    }

    /// 本帧要渲染的后缓冲对应的 RTV。
    pub fn current_rtv(&self) -> Result<D3D12_CPU_DESCRIPTOR_HANDLE, SurfaceError> {
        self.rtv(self.back_buffer_index())
    }

    /// 后缓冲个数。
    pub fn buffer_count(&self) -> u32 {
        self.buffer_count
    }

    /// 客户区像素尺寸。
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// 把某个后缓冲从 `PRESENT` 转到 `RENDER_TARGET`。
    ///
    /// flip model 下后缓冲常态就在 `PRESENT`，**不转就画不上去** —— 而 D3D12
    /// 不为此报任何错。所以这个转换必须由调用方在每帧开头显式做。
    pub fn barrier_to_render_target(&self, list: &ID3D12GraphicsCommandList) -> Result<(), SurfaceError> {
        let res = self.buffer(self.back_buffer_index())?;
        Gpu::barrier(
            list,
            &res,
            D3D12_RESOURCE_STATE_PRESENT,
            D3D12_RESOURCE_STATE_RENDER_TARGET,
        );
        Ok(())
    }

    /// 把后缓冲转回 `PRESENT`（上屏前必做）。
    pub fn barrier_to_present(&self, list: &ID3D12GraphicsCommandList) -> Result<(), SurfaceError> {
        let res = self.buffer(self.back_buffer_index())?;
        Gpu::barrier(
            list,
            &res,
            D3D12_RESOURCE_STATE_RENDER_TARGET,
            D3D12_RESOURCE_STATE_PRESENT,
        );
        Ok(())
    }

    /// 第 `i` 个后缓冲的资源。
    pub fn buffer(&self, i: u32) -> Result<ID3D12Resource, SurfaceError> {
        self.views
            .get(i as usize)
            .cloned()
            .ok_or_else(|| SurfaceError::new(
                "后缓冲下标越界",
                windows::core::Error::new(
                    windows::Win32::Foundation::E_POINTER,
                    format!(
                        "要第 {i} 个，交换链只有 {} 个",
                        self.views.len()
                    ),
                ),
            ))
    }

    /// 交换链本体（挂进 DComp 视觉树时用）。
    pub fn as_swap_chain1(&self) -> &IDXGISwapChain1 {
        &self.chain1
    }

    /// 上屏。
    ///
    /// `sync_interval = 1` 对应垂直同步。`DXGI_ERROR_DEVICE_RESET` 是**可恢复**的
    ///（设备正在重建，窗口最小化/恢复就走这条路），不当致命错误 —— 见
    /// [`Self::consecutive_present_errors`]。
    ///
    /// # 失败后必须再问 `GetDeviceRemovedReason`
    ///
    /// `Present` 回的 `DXGI_ERROR_DEVICE_REMOVED`（0x887A0005）是**通用**码，
    /// 不告诉你原因 —— 用户看到的那句「GPU 设备实例已经暂停」就是它。官方指定
    /// 要再调 `GetDeviceRemovedReason()` 才知道该怎么办：
    ///
    /// - `S_OK` ⇒ 设备其实**没坏**，这次 present 是别的原因失败的（队列满、
    ///   窗口刚最小化等）。**不能**据此判死 —— 那会把一次可恢复的抖动变成永久黑屏。
    /// - `DXGI_ERROR_DEVICE_RESET`（0x887A0006）⇒ 设备重建中，可恢复，**重试**。
    /// - `DXGI_ERROR_DEVICE_HUNG`（0x887A0007）⇒ 真的 TDR 了，得重建整个设备。
    ///
    /// 本仓 `renderer.rs` 记过一次 `DEVICE_HUNG`：采样一块仍是 `RENDER_TARGET`
    /// 状态的纹理，**错报在帧尾**而不在那一步 —— 也就是这里的 `Present` 是唯一
    /// 能看到它的地方。
    pub fn present(&mut self, sync_interval: u32) -> Result<(), SurfaceError> {
        // 走 `zephyra_dcomp::present_one` 而不是自己调 `Present1`：
        // `pPresentParameters` 必须**非空**，传 `nullptr` 直接 AV（硬约束 #3）。
        // ⚠ 新交换链的**第一次** present 之前，先把 GPU 排空。
        //
        // 官方依据（Chromium `ui/gl/swap_chain_presenter.cc:1155`）：
        // DirectComposition 会在「新交换链第 1~2 次 present 之间」显示黑屏，
        // 并且要等 GPU 做完，否则呈现昂贵内容时仍然会黑闪。
        //
        // 本仓对应场景：拖动期间 `Swapchain::resize` 失败一次就 `recreate()`
        // 整条链，也就是**拖动中会新建很多条交换链**；每一条在 DComp 眼里都是
        // 「新链 + 首次 present」⇒ 每次都可能黑一帧。这与 commit `9963ba1`
        // 修的「半死状态 / 尺寸不匹配」是**两套不同的闪**。
        //
        // 这里只抄「等 GPU」那一半。Chromium 还做了把每个后缓冲 `CopyResource`
        // 塞满同样内容 —— 那是他们的**软件路径**（present 时不重画每个缓冲）；
        // 我们每帧都画当前后缓冲，两个缓冲在两个帧内都会被写到，那半不必要。
        // ⚠ 新交换链的**第一次** present，调用方**应当先等 GPU**
        //（用 `take_first_present_flush()` 问它，宿主据此调 `Gpu::wait_gpu`）。
        // 说明见 [`Self::take_first_present_flush`]。
        let r = unsafe { present_one(&self.chain1, sync_interval) };
        match r {
            Ok(()) => {
                self.consecutive_present_errors = 0;

                Ok(())
            }
            Err(e) => {
                // `DXGI_STATUS_OCCLUDED`（0x087A0001）：窗口被完全遮挡。不算错 ——
                // 不重绘是对的，所以它不该累加「连续错误」计数。
                if e.code().0 as u32 == 0x087A_0001 {
                    return Ok(());
                }
                self.consecutive_present_errors = self.consecutive_present_errors.saturating_add(1);
                // 只有设备**真的**被判定不可用时才置死标志，见方法文档。
                if let Some(code) = device_removed_reason(&self.gpu_device) {
                    self.device_dead = Some(code);
                }
                Err(SurfaceError::new("Present", e))
            }
        }
    }

    /// 本链是不是「刚建出来、还没首次 present」。`true` 时调用方**应当先等 GPU**。
    ///
    /// # 官方依据
    ///
    /// Chromium `ui/gl/swap_chain_presenter.cc:1155`：DirectComposition 会在
    /// 「新交换链第 1~2 次 present 之间」显示黑屏，并且要等 GPU 做完，
    /// 否则呈现昂贵内容时仍然会黑闪。
    ///
    /// 本仓对应场景：拖动期间 `Swapchain::resize` 失败一次就 `recreate()` 整条
    /// 链，也就是**拖动中会新建很多条交换链**；每一条在 DComp 眼里都是
    /// 「新链 + 首次 present」⇒ 每次都可能黑一帧。这与 commit `9963ba1` 修的
    /// 「半死状态 / 尺寸不匹配」是**两套不同的闪**。
    ///
    /// # 为什么不在这里面等 GPU
    ///
    /// 等 GPU 要的是栅栏与命令队列，而它们在宿主的 `Gpu` 里。在本结构里再存一份
    /// `Gpu` 会让 `Swapchain` 持有它自己的创建者，drop 顺序立刻变成一道要论证的题。
    /// 所以只把**标志**交出去，等的事由宿主做（`Gpu::wait_gpu`，`surface.rs`）。
    ///
    /// Chromium 还做了把每个后缓冲 `CopyResource` 塞满同样内容 —— 那是他们的
    /// **软件路径**（present 时不重画每个缓冲）；我们每帧都画当前后缓冲，
    /// 两个缓冲在两个帧内都会被写到，那半不必要。
    pub fn take_first_present_flush(&mut self) -> bool {
        // ool 没有 .take()（那是 Option 的方法）；用 mem::take 取出来并复位。
        std::mem::take(&mut self.first_present_flush)
    }

    /// 连续 `Present` 失败的次数。
    ///
    /// 宿主据此决定是「等下一帧」还是「重建整个 GPU 栈」—— 单次失败（例如
    /// `DEVICE_RESET`）完全正常，把它当致命错误会让窗口在最小化恢复后直接退出。
    pub fn consecutive_present_errors(&self) -> u32 {
        self.consecutive_present_errors
    }
    /// 设备是否已不可用（`GetDeviceRemovedReason` 的结论）。
    pub fn device_dead(&self) -> bool {
        self.device_dead.is_some()
    }


    /// 设备不可用的原因码（`DXGI_ERROR_DEVICE_HUNG` 等）。`None` = 设备还活着。
    pub fn device_dead_reason(&self) -> Option<u32> {
        self.device_dead
    }

    /// 把**内容尺寸**设为 `(w, h)`，必要时把后缓冲**容量**涨到够用。
    ///
    /// # 这就是「拖动窗口时逐帧跟随」的全部机制
    ///
    /// 本机 `ResizeBuffers` 在后缓冲被用过之后就再也调不成功 —— 实测矩阵（可复跑，
    /// 见 `tests/swapchain_draw.rs::probe2_resize_buffers_矩阵`）：
    ///
    /// | 一帧里做了什么 | `ResizeBuffers` |
    /// |---|---|
    /// | 什么都不开帧 | **成功** |
    /// | 开帧 + 提交**空**命令列表 | **成功** |
    /// | 开帧 + 只转后缓冲资源屏障 | `0x887A0001` |
    /// | 开帧 + 画（1× / 4× MSAA） | `0x887A0001` |
    ///
    /// 被实测排除的四种成因（各自的修复仍保留，只是不足以成功）：等 GPU、
    /// `list.Reset` + `allocator.Reset`、双槽轮转、整体销毁重建命令对象。
    ///
    /// 所以本仓**不靠 `ResizeBuffers`**，而是让链一直**比窗口大**：
    ///
    /// - 窗口缩放 → 只改 [`Self::content`]（视口）→ 每帧照常按新尺寸排版并绘制 ⇒
    ///   **逐帧、清晰、无缩放**；
    /// - 窗口大到装不下时才**重建**整条链，且一次给 25% 余量
    ///   （[`CAPACITY_HEADROOM`]）⇒ 重建频率随窗口增长对数递减，而不是每帧一次。
    ///
    /// 「链比窗口大」不会露出破绽：DComp 按内容**自然尺寸 1:1** 画在原点，窗口只显示
    /// 左上角那块，其余在窗口外不可见。
    ///
    /// # 对照 Chromium
    ///
    /// Chromium 的同名路径（`components/viz/service/display_embedder/
    /// software_output_device_win_swapchain.cc:81 ResizeDelegated`）每次 resize 都调
    /// `ResizeBuffers`，失败就 `RestartGpuProcessForContextLoss` —— 它靠**独立 GPU
    /// 进程**兜底，而且那条路用的是 **`ID3D11Device`**（同文件 :115），不是 D3D12。
    /// 我们没有第二个进程，所以只能从结构上让 resize 不需要动链。
    ///
    /// # 零尺寸
    ///
    /// `(0, 0)` 是最小化状态，直接忽略（DXGI 拒绝 `ResizeBuffers(0,0,…)`，而这里
    /// 根本不调它，但内容尺寸为 0 时视口无意义）。
    pub fn resize(
        &mut self,
        gpu: &Gpu,
        size: (u32, u32),
    ) -> Result<(), SurfaceError> {
        self.was_recreated = false;
        if size.0 == 0 || size.1 == 0 {
            return Ok(());
        }
        self.content = size;
        if size.0 <= self.size.0 && size.1 <= self.size.1 {
            return Ok(());
        }

        // 装不下 ⇒ 重建，并一次给足余量，避免「每涨一点就重建一次」。
        let (w, h) = Self::grown_capacity(size, Self::CAPACITY_HEADROOM);
        // 等 GPU：旧链可能还有命令在飞，而重建会把它整个换掉。
        gpu.wait_gpu()?;
        let count = self.buffer_count;
        let fresh = Self::recreate(gpu, (w, h), count, self.samples)?;
        // `fresh` 的 content 从构造值起，按请求尺寸修正（容量 >= 内容）。
        let old_chain = std::mem::replace(&mut self.chain1, fresh.chain1.clone());
        let content = self.content;
        let samples = self.samples;
        *self = fresh;
        self.content = content;
        self.samples = samples;
        self.superseded = Some(old_chain);
        self.was_recreated = true;
        // 新链第一次 present 可能被 DComp 显示成黑帧，见 present 的说明。
        self.first_present_flush = true;
        Ok(())
    }

    /// 容量不足时**增长到**多大。
    ///
    /// 25% 余量的来由：它让重建次数随窗口尺寸**对数**增长 —— 从 800px 拖到 2000px
    /// 只会重建 3~4 次（800 → 1000 → 1250 → 1563 → 1954 → 2443），而逐帧重建是
    /// 1200 次。取 25% 而不是更大的余量，是因为显存按 `w × h` 线性涨，50% 余量要多
    /// 占 2.25 倍显存却只把重建次数从 ~4 降到 ~3。
    pub fn grown_capacity(need: (u32, u32), headroom: f64) -> (u32, u32) {
        let f = |v: u32| -> u32 {
            let want = (v as f64 * (1.0 + headroom)).ceil() as u32;
            // flip model 对尺寸没有 2 的幂要求（Chromium 那边自己还要特判奇数，
            // 见 swap_chain_presenter.cc:755），但对齐到 8 能省掉跨步幅的带宽浪费。
            want.next_multiple_of(8)
        };
        (f(need.0), f(need.1))
    }

    /// 内容尺寸（视口）。见 [`Self::content`]。
    pub fn content_size(&self) -> (u32, u32) {
        self.content
    }
    /// 丢弃旧交换链并重新建一条同样参数的。
    fn recreate(
        gpu: &Gpu,
        size: (u32, u32),
        buffer_count: u32,
        samples: u32,
    ) -> Result<Self, SurfaceError> {
        Self::new(
            gpu,
            size,
            &SwapchainDesc {
                buffer_count,
                format: DXGI_FORMAT_R8G8B8A8_UNORM,
            },
            samples,
        )
    }

    /// 本次 [`Self::resize`] 是否走了「重建整条链」这条路。
    ///
    /// 宿主据此决定要不要把 [`Self::as_swap_chain1`] 重新挂进 DComp 视觉树。
    pub fn was_recreated(&self) -> bool {
        self.was_recreated
    }

    /// 取走被替换下来的旧链（取到就释放）。
    ///
    /// 调用方必须在**已经把新链 `SetContent` 上并 `Commit` 过**之后才调这个 ——
    /// 早一步释放就是让 DComp 持有一个已释放的交换链，实测会 TDR。
    pub fn take_superseded(&mut self) -> Option<IDXGISwapChain1> {
        self.superseded.take()
    }

    /// 后缓冲格式。
    /// 后缓冲格式。
    ///
    /// 用 `GetDesc1`（`DXGI_SWAP_CHAIN_DESC1`）而不是 `GetDesc`
    /// （`DXGI_SWAP_CHAIN_DESC`，后者的格式藏在 `BufferDesc` 里）—— 混用会在
    /// `Format` 字段上直接编译失败，而那正是当初手写时的错。
    pub fn format(&self) -> DXGI_FORMAT {
        unsafe { self.chain3.GetDesc1() }
            .map(|d| d.Format)
            .unwrap_or(DXGI_FORMAT_B8G8R8A8_UNORM)
    }

    /// 重新 `GetBuffer` 全部后缓冲并重建 RTV。
    ///
    /// 走 [`SurfaceError`] 而不是裸 `windows::core::Error`：描述符越界是本 crate 自己的
    /// 错误（`E_POINTER`），抹平成 DXGI 的码会丢掉「是第几个越界」这条线索。
    fn rebuild_views(&mut self, device: &ID3D12Device) -> Result<(), SurfaceError> {
        let mut fresh = Vec::with_capacity(self.buffer_count as usize);
        for i in 0..self.buffer_count {
            // `GetBuffer` 的出参类型由调用方指定 —— D3D12 里它就是 `ID3D12Resource`。
            let res: ID3D12Resource = unsafe { self.chain3.GetBuffer(i) }
                .map_err(|e| SurfaceError::new(format!("GetBuffer({i})"), e))?;
            // 描述符堆在 resize 时不变，所以这里可以就地覆写同一个槽位。
            let slot = self.heap.cpu(i)?;
            unsafe { device.CreateRenderTargetView(&res, None, slot) };
            fresh.push(res);
        }
        self.views = fresh;
        Ok(())
    }
}

/// 描述符必须**按缓冲个数**给，而不是固定 1 个。
///
/// flip model 下 `GetCurrentBackBufferIndex` 会在 0..BufferCount 之间轮转，
/// 堆只给 1 个描述符时第二帧 `cpu(1)` 就越界 —— 而越界的症状是「画面卡在第
/// 一帧的颜色」或崩，没有任何提示指向描述符个数。
#[cfg(test)]
#[path = "tests/swapchain_tests.rs"]
mod tests;
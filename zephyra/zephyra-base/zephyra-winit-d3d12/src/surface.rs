//! D3D12 的设备、描述符堆、离屏目标与呈现。
//!
//! 与 D3D11 版的**根本差别**在这里：D3D12 的资源必须**手工**在状态之间搬移
//! （`ResourceBarrier`），D3D11 则由驱动在背后做隐式转换。
//!
//! # 资源状态机
//!
//! 每个资源在任一时刻处于一种状态，且**每次跨状态都要发屏障**。
//!
//! ```text
//!  后缓冲： PRESENT ⇄ RENDER_TARGET       每帧一圈
//!  纹理回读：RENDER_TARGET → COPY_SRC → (拷贝) → PRESENT
//!  顶点/索引：UPLOAD 堆常驻映射，不需要屏障（见 `renderer`）
//! ```
//!
//! 用错顺序的后果与 D3D11 的静默失效不同 —— D3D12 会在调试层或 GPU 挂起时**明确
//! 报错**。这正是选它作为首选后端的一个理由：本项目在 D3D11 上遇到的「什么都不画且
//! 零错误」，在 D3D12 上会变成一条可读的错误。
//!
//! # 但有一件事 D3D12 仍然静默
//!
//! 光栅化状态里有一个坑与 D3D11 视口一模一样：
//!
//! D3D11 有 `D3D11_RASTERIZER_DESC::ScissorEnable` 这个开关；**D3D12 没有**——
//! 裁剪矩形**永远生效**。忘记调 `RSSetScissorRects` 时，默认矩形是**空的**，所有
//! 图元被全部裁掉、一个像素都不画，且**不报任何错**。
//!
//! 这与本项目在 D3D11 上踩的视口坑是同一类错误（「范围为零 → 零面积 → 静默不画」），
//! 所以 [`crate::renderer`] 里视口与 `RSSetScissorRects` 是**成对**设置的，缺一不可。

#![cfg(windows)]

use windows::core::Interface as _;
use windows::Win32::Foundation::{E_POINTER, E_FAIL, RECT};
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, DXGI_ERROR_NOT_FOUND, IDXGIAdapter1};

/// 设备 + 提交队列 + 命令分配器 + 命令列表 + 栅栏。
///
/// # 为什么设备里挂着一个调试层
///
/// D3D12 的 `E_INVALIDARG` **不带任何说明**：「哪个字段错了」这件事只有调试层知道。
/// 本项目在 `Layout` 上就吃过这个亏 —— 提交式资源必须写 `D3D12_TEXTURE_LAYOUT_UNKNOWN`，
/// 写成 `ROW_MAJOR` 就得到一个纯 `E_INVALIDARG`，没有任何提示指向 `Layout`。
///
/// 所以这里默认就开调试层（`Win32_Graphics_Direct3D12` 里 `D3D12GetDebugInterface` 是
/// `link!` 的软链接，不引入编译期依赖），并把消息打到 stderr。
///
/// 开销：调试层会让 API 调用慢 2~10 倍，且会占内存。所以生产环境应当能关掉 ——
/// 用环境变量 `ZEPHYRA_D3D12_DEBUG=0` 关。
pub struct Gpu {
    pub device: ID3D12Device,
    pub queue: ID3D12CommandQueue,
    pub allocator: ID3D12CommandAllocator,
    pub list: ID3D12GraphicsCommandList,
    fence: ID3D12Fence,
    fence_event: windows::Win32::Foundation::HANDLE,
    /// 每提交一次加一。
    fence_value: u64,
    /// 调试层（`None` = 关掉了）。
    debug: Option<DebugInfo>,
}

/// `D3D12_CREATE_DEVICE_FLAGS` —— `windows` 0.62 **没有**导出这个类型。
///
/// 值按官方头文件 `d3d12.h` 抄，不自己拍：
///
/// ```c
/// typedef enum D3D12_CREATE_DEVICE_FLAGS {
///   D3D12_CREATE_DEVICE_DEBUG = 0x1,
/// } D3D12_CREATE_DEVICE_FLAGS;
/// ```
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct D3D12CreateDeviceFlags(u32);

/// `D3D12_CREATE_DEVICE_DEBUG`（见上面 `D3D12CreateDeviceFlags` 的出处）。
const D3D12_CREATE_DEVICE_DEBUG: D3D12CreateDeviceFlags = D3D12CreateDeviceFlags(0x1);

extern "system" {
    /// `D3D12CreateDevice` 的**五参数**原形。
    ///
    /// `windows` crate 的绑定把 `riid` 由泛型固定、且没有 `Flags` 参数，所以拿不到
    /// `D3D12_CREATE_DEVICE_DEBUG`。见 [`Gpu::create_device_with_flag`] 的说明。
    #[link_name = "D3D12CreateDevice"]
    fn D3D12CreateDevice_raw(
        padapter: *mut std::ffi::c_void,
        minimumfeaturelevel: windows::Win32::Graphics::Direct3D::D3D_FEATURE_LEVEL,
        riid: *const windows::core::GUID,
        flags: D3D12CreateDeviceFlags,
        ppdevice: *mut *mut std::ffi::c_void,
    ) -> windows::core::HRESULT;
}

/// 开/关调试层。
///
/// # 默认**关**，要显式 `ZEPHYRA_D3D12_DEBUG=1` 才开
///
/// 早先这里是默认开的。实测在本机上，只要开了它，`Gpu::new()` 返回后设备立刻
/// `DXGI_ERROR_DEVICE_HUNG`（`GetDeviceRemovedReason` 报 `0x887A0007`
/// `DRIVER_INTERNAL_ERROR`）—— **设备刚建好就挂了，一个字节都还没画**。
///
/// 原因是本实现走的是「先建设备，再 `ID3D12Debug::EnableDebugLayer()`」这条
/// 事后开启的路（见 [`DebugInfo::attach`]）。微软推荐的是**事前**在
/// `D3D12CreateDevice` 的 `Flags` 里带 `D3D12_CREATE_DEVICE_DEBUG`；事后开启在
/// 部分驱动上不稳。
///
/// 而且调试层在这里本来也没用：它要么能拿到消息队列（那确实有用），要么
/// `D3D12GetDebugInterface` 直接失败（退化成只有 layer 没有队列）。既然默认开启
/// 会在本机把整个后端搞挂，而收益又依赖环境，就不该默认开。
///
/// 需要查驱动为什么报错时：`$env:ZEPHYRA_D3D12_DEBUG=1`。
pub fn debug_enabled() -> bool {
    // ⚠ 默认**关**。这不是省事 —— 是被逼的：
    //
    // 把调试层开成默认，会让 zephyra-winit/tests/backend_parity.rs 直接
    // STATUS_ACCESS_VIOLATION 崩掉（实测，stash 前后对照确认）。那条用例要同时
    // 建 D3D11 与 D3D12 两个后端，本机在这条组合上开调试层不稳。
    //
    // 而本机恰恰只有开了调试层才问得到「哪个 API、哪个资源、哪个状态」—— 于是形成
    // 一个死结：不开没证据，开了崩。死结的解法不是二选一，而是**把证据自动送到**
    // 人眼前：见 HostWindow::report_device_dead_once（设备一判死就 drain 调试
    // 队列，并在没有消息时提示怎么开）。
    //
    // 查 GPU 为什么挂时：$env:ZEPHYRA_D3D12_DEBUG=1
    //
    // # 开启方式已改成微软推荐的「事前」路径
    //
    // 早先是 ID3D12Debug::EnableDebugLayer()（**事后**开启），本注释原来记着
    // 「事后开启会让设备刚建好就 HUNG」。现在调试层改由
    // Gpu::create_device_with_flag 在 D3D12CreateDevice 的 Flags 里带
    // D3D12_CREATE_DEVICE_DEBUG 开启（**事前**，微软推荐路径）。事后开启那条路
    // 已经**不再使用**。事前开启是否也躲开了那个 HUNG，需要有 GPU 的机器确认。
    matches!(
        std::env::var("ZEPHYRA_D3D12_DEBUG").as_deref(),
        Ok("1") | Ok("true") | Ok("on")
    )
}

/// 调试层 + 挂在**设备**上的消息队列。
///
/// # 队列为什么挂在设备上而不是 `ID3D12Debug` 上
///
/// `ID3D12Debug` 在 windows-rs 0.62 里**只有** `EnableDebugLayer` 一个方法；消息队列
/// 是 `ID3D12InfoQueue`。但**只有挂在设备上的那个队列**会收到资源创建失败之类的消息
/// —— 只查 `ID3D12Debug` 的队列会一条都收不到，而这个项目遇到的第一个 D3D12 错误正是
/// `CreateCommittedResource` 的 `E_INVALIDARG`。
///
/// # 为什么必须主动拉
///
/// 消息是**排队**的。所以「没看到错误」完全不能推出「没有错误」。这一点与
/// `ID3D11InfoQueue` 相同，也与 D3D11 那次踩过的坑相同。
pub struct DebugInfo {
    _layer: Option<ID3D12Debug>,
    queue: ID3D12InfoQueue,
}

impl DebugInfo {
    /// **在建设备之前**打开调试层。
    ///
    /// # ⚠ 这里原先有两处错，都是「找不到原因」的直接原因
    ///
    /// 1. **顺序反了。** 原先是在设备建好之后才 `EnableDebugLayer()`。微软的文档明写
    ///    要**先**开再建设备：
    ///    「To enable the debug layer, get the `ID3D12Debug` interface using
    ///    `D3D12GetDebugInterface` and call `EnableDebugLayer()`. Note that you should
    ///    call this **before** creating the device.」
    ///    事后开启时有些驱动不把消息记进队列。
    ///
    ///    （`D3D12CreateDevice` 本身**没有** `Flags` 参数 —— 调试层不是靠建设备时传标志
    ///    开的，那条路对 D3D12 不存在。）
    ///
    /// 2. **取错了队列。** 原先是把 `ID3D12Debug`（**全局**接口）`cast` 成
    ///    `ID3D12InfoQueue`。而本文件自己的注释就写着「只有挂在**设备**上的那个队列会
    ///    收到资源创建失败之类的消息」—— 注释说的是对的，代码做的正好相反，于是
    ///    **一条消息都收不到**。
    ///
    /// 两条合起来的效果就是：`CreateGraphicsPipelineState` 回 `E_INVALIDARG`，而调试层
    /// 「确认可用」却什么都不说。上一轮我在 D3D11 上因为同一个原因（队列取错 + 长度参数
    /// 传 0）误判了两次「调试层不可用」，这里是同一个坑的 D3D12 版本。
    ///
    /// 返回 `None` 表示这台机器没有调试层 —— 那时**不**是错误，只是问不到原因。
    pub fn enable_layer() -> Option<ID3D12Debug> {
        let mut layer = None::<ID3D12Debug>;
        unsafe {
            D3D12GetDebugInterface(&mut layer).ok()?;
            layer.as_ref()?.EnableDebugLayer();
        }
        layer
    }

    /// 从**设备**上拿消息队列。
    ///
    /// `ID3D12InfoQueue` 是挂在设备上的调试对象，要对 `ID3D12Device` 做 QI 才拿得到。
    /// 拿不到就退化成「有调试层、没有队列」，而不是整体失败 —— 那会让一台没装调试层的
    /// 机器完全跑不起来。
    pub fn attach(layer: Option<ID3D12Debug>, device: &ID3D12Device) -> Option<Self> {
        // QI 拿队列。`cast` 是 `Interface` 的方法，windows-rs 0.62 里是**安全**的。
        let queue = device.cast::<ID3D12InfoQueue>().ok()?;
        Some(Self {
            _layer: layer,
            queue,
        })
    }

    /// 把待处理消息全部打到 stderr，然后清空队列。
    ///
    /// 清空是为了不让下一帧的消息与这一帧的混在一起 —— 混在一起的日志比看不到更难查。
    pub fn drain(&self) -> Vec<u32> {
        let mut severe: Vec<u32> = Vec::new();
        unsafe {
            let count = self.queue.GetNumStoredMessages();
            for i in 0..count {
                // ⚠ 缓冲区必须**给足**，不能只给 `size_of::<D3D12_MESSAGE>()`。
                //
                // `D3D12_MESSAGE` 是 32 字节，但一条带描述的消息实际要一百多字节 ——
                // 描述**紧跟在结构体后面**一起写出。给 32 字节时 `GetMessage` 回
                // `E_INVALIDARG`，而这里原本是 `continue` 静默跳过，于是调试层
                // 「能挂上、能用、一条也读不出来」。
                //
                // 这与 D3D11 那次是**同一个坑**：`pMessageByteLength` 是 in/out，
                // 且缓冲区容量给小了。两次都让我判成「调试层不可用」。
                //
                // 先空指针问一次要多少（更稳），失败就退回一个宽松的固定值。
                let mut need: usize = 0;
                if self.queue.GetMessage(i, None, &mut need as *mut usize).is_err() {
                    need = 0;
                }
                let mut buf = vec![0u8; need.max(std::mem::size_of::<D3D12_MESSAGE>() * 8)];
                let mut len = buf.len();
                if self
                    .queue
                    .GetMessage(i, Some(buf.as_mut_ptr().cast()), &mut len)
                    .is_err()
                {
                    eprintln!("[d3d12 debug] 第 {i} 条消息读不出来（缓冲 {len} 字节）");
                    continue;
                }
                let msg: &D3D12_MESSAGE = &*buf.as_ptr().cast::<D3D12_MESSAGE>();
                // `pDescription` 是**不保证**以 NUL 结尾的字节串，长度由
                // `DescriptionByteLength` 给出 —— 不能指望 C 字符串约定。
                let text = if msg.pDescription.is_null() {
                    String::new()
                } else {
                    String::from_utf8_lossy(std::slice::from_raw_parts(
                        msg.pDescription,
                        msg.DescriptionByteLength,
                    ))
                    .into_owned()
                };
                // ERROR 及以上（2 = ERROR / 3 = CORRUPTION）把**消息 ID**收进返回值。
                //
                // drain 原来只打印，于是「命令列表合法」这个结论永远得不到证据 ——
                // 验证层是**排队**报的，而跨帧资源状态错位（527）在像素上只表现为「第二帧
                // 模糊偏亮一点」。返回 ID 而不是计数，是为了让调用方能**按消息**豁免：
                // 820（ClearRenderTargetView 的清屏色与创建时不同）只是性能提示，
                // 而 527（屏障的 Before 状态与实际不符）是实打实的 bug。
                if msg.Severity.0 >= D3D12_MESSAGE_SEVERITY_ERROR.0 {
                    severe.push(msg.ID.0 as u32);
                }
                eprintln!(
                    "[d3d12 debug] ID={:?} severity={:?}: {text}",
                    msg.ID, msg.Severity
                );
            }
            self.queue.ClearStoredMessages();
        }
        severe
    }
}

impl Gpu {
    /// 建设备，可选带 `D3D12_CREATE_DEVICE_DEBUG`。
    ///
    /// # 默认路径**逐字保持原样**，只为 debug 路径才手搓 ABI
    ///
    /// `windows` crate 的 `D3D12CreateDevice` 绑定把 `riid` 由泛型固定、且没有
    /// `Flags` 参数，所以拿不到调试层 flag，只能自己链一次五参原形。
    ///
    /// 但**默认（不带调试层）那条路必须走 windows 的绑定**：先试过「两条路都用
    /// 手搓的原形」，结果是 `hr = S_FALSE(1)` 且不给设备 —— 调用约定/符号解析
    /// 与 windows 自己的 `link!` 声明对不上，于是 `backend_parity` 两条用例全崩。
    ///
    /// 现在只有 `debug = true` 才走 raw 调用；`false` 时与改动前**逐字节相同**。
    /// 理由：拿不到 GPU 证据时，默认路径的**可验证性**比多一条能力重要。
    ///
    /// # 为什么调试层改成「事前」开启
    ///
    /// 早先这里是 `ID3D12Debug::EnableDebugLayer()`（**事后**开启），本仓注释记着
    /// 「事后开启会让设备刚建好就 `DXGI_ERROR_DEVICE_HUNG`」。微软推荐的是在
    /// `D3D12CreateDevice` 的 Flags 里带 `D3D12_CREATE_DEVICE_DEBUG`。
    fn create_device_with_flag(
        adapter: &windows::Win32::Graphics::Dxgi::IDXGIAdapter1,
        debug: bool,
    ) -> Result<ID3D12Device, SurfaceError> {
        let mut device = None::<ID3D12Device>;
        if !debug {
            // SAFETY：`adapter` 是活着的适配器；`device` 是合法的输出槽。
            unsafe {
                D3D12CreateDevice(
                    adapter,
                    windows::Win32::Graphics::Direct3D::D3D_FEATURE_LEVEL_11_0,
                    &mut device,
                )
            }
            .map_err(|e| SurfaceError::new("D3D12CreateDevice", e))?;
            return Ok(device.expect("D3D12CreateDevice 成功时必给设备"));
        }
        let flags = D3D12_CREATE_DEVICE_DEBUG;
        // SAFETY：`d3d12.dll` 本进程已加载；指针参数都指向合法局部变量。
        let hr = unsafe {
            D3D12CreateDevice_raw(
                windows::core::Interface::as_raw(adapter),
                windows::Win32::Graphics::Direct3D::D3D_FEATURE_LEVEL_11_0,
                &<ID3D12Device as windows::core::Interface>::IID,
                flags,
                &mut device as *mut Option<ID3D12Device> as *mut *mut std::ffi::c_void,
            )
        };
        if hr.is_err() || device.is_none() {
            return Err(SurfaceError::new(
                "D3D12CreateDevice（带 DEBUG flag）",
                windows::core::Error::from_hresult(hr),
            ));
        }
        Ok(device.expect("上面已判过 is_none"))
    }

    /// 拿到调试层对象（**不**在这里开启它）。
    ///
    /// 调试层已经由 [`Self::create_device_with_flag`] 的 `Flags` 事前开启了；
    /// 这里只是 QI 出那个 `ID3D12Debug` 实例，好让 [`DebugInfo`] 持有它不被释放
    /// —— 层的生命周期由设备的 Debug 标志决定，不由这个引用决定。
    fn enable_debug_layer() -> Option<ID3D12Debug> {
        if !debug_enabled() {
            return None;
        }
        let mut layer = None::<ID3D12Debug>;
        // SAFETY：只查询接口，不调任何会改设备状态的方法。
        unsafe {
            D3D12GetDebugInterface(&mut layer).ok()?;
        }
        layer
    }

    /// 用默认硬件适配器建 D3D12 设备。
    ///
    /// # 为什么显式枚举适配器而不是传 `None`
    ///
    /// 传 `None` 会让 D3D12 在所有适配器上选一个，但**多适配器机器上未必是你要的
    /// 那块**（插着独显却选到核显）。显式枚举 0 号在单卡机器上等价，在多卡机器上
    /// 至少是确定行为。
    pub fn new() -> Result<Self, SurfaceError> {
        let factory: windows::Win32::Graphics::Dxgi::IDXGIFactory4 =
            unsafe { CreateDXGIFactory1() }.map_err(SurfaceError::factory)?;
        let adapter = pick_adapter(&factory).map_err(|_| {
            // 「没有更多适配器」返回 DXGI_ERROR_NOT_FOUND —— 这台机器一个适配器都没有。
            // 用它自己的消息比笼统的「建设备失败」有用得多。
            SurfaceError::new(
                "枚举不到任何适配器（没装显卡驱动？）",
                windows::core::Error::new(DXGI_ERROR_NOT_FOUND, "枚举不到适配器"),
            )
        })?;

        let device = match Self::create_device_with_flag(&adapter, debug_enabled()) {
            Ok(d) => d,
            Err(e) => {
                // 调试层建不起来不能算应用失败 —— 退回不带 flag 的普通路径。
                // 唯一代价是这一帧之后没有 D3D 侧证据。
                eprintln!(
                    "[zephyra-d3d12] 带 D3D12_CREATE_DEVICE_DEBUG 建设备失败（{e}），\
                     退回普通路径；之后 GPU 报错将没有调试层消息可查"
                );
                Self::create_device_with_flag(&adapter, false)?
            }
        };

        let queue: ID3D12CommandQueue =
            unsafe { device.CreateCommandQueue(&D3D12_COMMAND_QUEUE_DESC::default()) }
                .map_err(|e| SurfaceError::new("CreateCommandQueue", e))?;

        let allocator: ID3D12CommandAllocator =
            unsafe { device.CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT) }
                .map_err(|e| SurfaceError::new("CreateCommandAllocator", e))?;

        let list: ID3D12GraphicsCommandList =
            unsafe { device.CreateCommandList(0, D3D12_COMMAND_LIST_TYPE_DIRECT, &allocator, None) }
                .map_err(|e| SurfaceError::new("CreateCommandList", e))?;
        // `CreateCommandList` 交付时处于**记录中**状态，必须先关掉才能在帧循环里复用。
        unsafe { list.Close() }
            .map_err(|e| SurfaceError::new("Close 初始命令列表", e))?;

        let fence: ID3D12Fence = unsafe {
            device.CreateFence(0, D3D12_FENCE_FLAG_NONE)
        }
        .map_err(|e| SurfaceError::new("CreateFence", e))?;

        let fence_event = unsafe {
            windows::Win32::System::Threading::CreateEventW(None, false, false, None)
        }
        .map_err(|e| SurfaceError::new("CreateEvent（栅栏事件）", e))?;

        // ⚠ 必须**与原行为逐字一致**：调试层关时整个 `DebugInfo` 是 `None`，而不是
        // 「layer 为 None 但队列挂着」。
        //
        // 后者会让 `drain_debug()` 在没有调试层时照样去 QI `ID3D12InfoQueue` 并
        // 每帧 `ClearStoredMessages` —— 实测那会让
        // `zephyra-winit/tests/backend_parity.rs` 直接 `STATUS_ACCESS_VIOLATION`。
        let debug = if debug_enabled() {
            // 队列取自**设备**，不是全局 layer。见 `attach` 的说明。
            DebugInfo::attach(Self::enable_debug_layer(), &device)
        } else {
            None
        };

        Ok(Self {
            device,
            queue,
            allocator,
            list,
            fence,
            fence_event,
            fence_value: 0,
            debug,
        })
    }

    /// 调试层（`None` = 关掉了）。
    pub fn debug(&self) -> Option<&DebugInfo> {
        self.debug.as_ref()
    }

    /// 把调试层的消息打到 stderr。
    ///
    /// `begin_frame` 里已经调了，所以日常帧循环不用管；只在排查时手动调。
    pub fn drain_debug(&self) -> Vec<u32> {
        if let Some(d) = &self.debug {
            return d.drain();
        }
        Vec::new()
    }

    /// 设备是否已被移除（挂起 / 驱动重置 / 驱动升级）。
    ///
    /// # 为什么值得单独提供
    ///
    /// D3D12 里「某个 API 返回了别的错误」经常只是**后果**，真正的原因是这个值。
    /// 之前追一个 `CreateCommittedResource` 的 `E_INVALIDARG` 时，
    /// `GetDeviceRemovedReason` 已经返回 `DXGI_ERROR_DEVICE_HUNG` —— 也就是说设备早
    /// 就挂了，后面所有的 `E_INVALIDARG` 都是它投的影子。**先看这个，再看别的。**
    pub fn removed_reason(&self) -> windows::core::HRESULT {
        // windows-rs 把它包成 `Result<(), Error>`：`Ok` 表示设备正常，`Err` 的 code 就是
        // 移除原因（`DXGI_ERROR_DEVICE_HUNG` / `_REMOVED` / `_RESET` / `DXGI_ERROR_DRIVER_INTERNAL_ERROR`）。
        match unsafe { self.device.GetDeviceRemovedReason() } {
            Ok(()) => windows::core::HRESULT(0),
            Err(e) => e.code(),
        }
    }

    /// 开一个新帧：等上一帧做完，再重置分配器与命令列表。
    ///
    /// # 为什么必须先等 GPU
    ///
    /// 分配器被 `Reset` 时若 GPU 还在从里面读命令，D3D12 会让 GPU **挂起**而不是报错
    /// —— 症状是「跑着跑着整个窗口卡死」，debug 层之外什么都看不到。
    pub fn begin_frame(&self) -> Result<(), SurfaceError> {
        self.drain_debug();
        self.wait_gpu()?;
        // ⚠ 只调 `list.Reset`，**不**调 `ID3D12CommandAllocator::Reset` —— 后者在本机
        // 恒定回 `E_FAIL`，无论开不开调试层。
        //
        // 失败时的官方说法是「there is an **actively recording** command list
        // referencing the command allocator」，而此刻**没有人在记录**：列表已被上一帧
        // 的 `submit` 关过。于是「按文档的顺序（先分配器后列表）走」这条路在本机
        // **根本走不通** —— 这是本轮实测出来的（`tests/swapchain_draw.rs` 的
        // `resize_失败点_是建链还是帧流程` 有对照）。
        //
        // 于是代价落在别处：分配器一直攥着那些命令用过的资源，所以画过之后
        // `ResizeBuffers` 会回 `DXGI_ERROR_INVALID_CALL`。官方对「还有引用时
        // resize」的原文：
        //
        //     "Indirect references are held by views to a resource, binding a view
        //      of the resource to a device context, a command list that used the
        //      resource, …"
        //
        // 我先怀疑命令列表，每帧 `begin_frame` 都 `list.Reset` —— **没用**，因为
        // `list.Reset()` 并不重置分配器（文档没这么说，是我当时假设的）。
        //
        // 正路是**重建整个交换链**而不是 resize —— 见 `Swapchain::resize`。
        //
        // https://learn.microsoft.com/en-us/windows/win32/api/d3d12/nf-d3d12-id3d12commandallocator-reset
        // https://learn.microsoft.com/en-us/windows/win32/api/d3d12/nf-d3d12-id3d12graphicscommandlist-reset
        //
        // 失败时立刻拉调试层消息：`drain_debug` 只在开头调过一次，漏了就得等下一帧
        // —— 那时进程可能已经死了。
        if let Err(e) = unsafe { self.list.Reset(&self.allocator, None) } {
            self.drain_debug();
            return Err(SurfaceError::new("Reset 命令列表", e));
        }
        Ok(())
    }

    /// 提交已记录的命令列表，并递增信号值。
    pub fn submit(&mut self) -> Result<(), SurfaceError> {
        // 失败时立刻拉调试层消息 —— `drain_debug` 只在 `begin_frame` 开头调过，
        // 而本帧记录命令期间产生的任何错误都要等到下一帧才看得见，那时可能已经崩了。
        if let Err(e) = unsafe { self.list.Close() } {
            self.drain_debug();
            return Err(SurfaceError::new("Close 命令列表", e));
        }
        self.fence_value += 1;
        let value = self.fence_value;
        // `ExecuteCommandLists` 收的是基类 `ID3D12CommandList`。`cast()` 走 QueryInterface，
        // 拿到的是**同一个对象**（D3D12 的接口继承不做包装），所以这里没有额外开销。
        let base: ID3D12CommandList = self.list.cast().map_err(|e| {
            SurfaceError::new("把命令列表转成 ID3D12CommandList", e)
        })?;
        unsafe { self.queue.ExecuteCommandLists(&[Some(base)]) };
        unsafe { self.queue.Signal(&self.fence, value) }
            .map_err(|e| SurfaceError::new("Signal 栅栏", e))?;
        Ok(())
    }

    /// 等队列排空。
    ///
    /// 用 `SetEventOnFenceCompletion` 而不是等栅栏本身：**栅栏不是 Win32 句柄**，
    /// 不能直接 `WaitForSingleObject`。所以要额外建一个事件（`Gpu::new` 里建的），
    /// 让 GPU 到达目标值时去置位它。
    ///
    /// 死等而不是超时：GPU 真挂起时让进程卡住，好过返回一个**看起来正常**的空结果
    /// ——后者会把「GPU 卡死」误诊成「渲染逻辑没画东西」。
    pub fn wait_gpu(&self) -> Result<(), SurfaceError> {
        let target = self.fence_value;
        if target == 0 {
            return Ok(()); // 还没提交过任何东西
        }
        if unsafe { self.fence.GetCompletedValue() } >= target {
            return Ok(());
        }
        // 事件置位失败时**不能**当作「已完成」继续：那会让调用方覆写 GPU 还在读的
        // 命令缓冲 —— 症状是随机崩溃，而不是一条可查的错误。所以这里冒泡。
        unsafe { self.fence.SetEventOnCompletion(target, self.fence_event) }
            .map_err(|e| SurfaceError::new("SetEventOnCompletion", e))?;
        unsafe {
            windows::Win32::System::Threading::WaitForSingleObject(
                self.fence_event,
                windows::Win32::System::Threading::INFINITE,
            )
        };
        Ok(())
    }

    /// 开一条**独立的**命令列表，记录一小段命令，立刻提交并等 GPU 做完。
    ///
    /// # 什么时候需要它
    ///
    /// 凡是「不发生在帧的 `begin_frame` 与 `submit` 之间」的 GPU 操作：注册图像纹理
    /// （建窗那一帧的中途）、换 shader、调试用的单次拷贝。把它们塞进帧循环里会要求
    /// 调用方知道宿主的帧状态 —— 而那正是宿主该藏起来的东西。
    ///
    /// # 为什么等 GPU
    ///
    /// 因为这条命令的操作数（暂存缓冲）通常由调用方的栈或局部变量持有，回去就失效。
    /// `begin_frame` 开头也有 `wait_gpu`，但那是**下一帧**才发生的事 ——
    /// 中间这一段时间里 GPU 可能正在读已经归还的内存。
    pub fn submit_one_off<F>(&mut self, record: F) -> Result<(), SurfaceError>
    where
        F: FnOnce(&ID3D12GraphicsCommandList),
    {
        self.wait_gpu()?;
        unsafe { self.list.Reset(&self.allocator, None) }
            .map_err(|e| SurfaceError::new("一次性命令：Reset 命令列表", e))?;
        record(&self.list);
        // ⚠ 这里**不能**自己 `Close` —— `submit` 内部已经关了一次。
        //
        // 对一条已关闭的列表再 `Close`，D3D12 不返回错误：`Close` 的返回码是
        // `HRESULT`，但驱动只把它当警告，实际是通过调试层的
        // `ID3D12_MESSAGE_ID(547) "This API cannot be called on a closed command list"`
        // 报出来。不开调试层时它完全静默 —— 于是症状是「一次性命令没生效」，
        // 而所有 HRESULT 都是成功的。
        self.submit()?;
        self.wait_gpu()
    }

/// 发一条资源状态转换屏障。
    ///
    /// # 为什么它是自由函数而不是 `Gpu` 的方法
    ///
    /// 屏障**必须**在命令列表上记录，不能直接提交 —— 它是一条命令，不是一次操作。
    /// 所以命令列表是必需参数。做成自由函数是为了让「屏障该在哪一步发」在调用点上一眼
    /// 可见，而不是被藏进某个 `gpu.transition(...)` 里。
    pub fn barrier(
        list: &ID3D12GraphicsCommandList,
        resource: &ID3D12Resource,
        before: D3D12_RESOURCE_STATES,
        after: D3D12_RESOURCE_STATES,
    ) {
        unsafe {
            list.ResourceBarrier(&[D3D12_RESOURCE_BARRIER {
                Type: D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
                Flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
                Anonymous: D3D12_RESOURCE_BARRIER_0 {
                    Transition: std::mem::ManuallyDrop::new(
                        D3D12_RESOURCE_TRANSITION_BARRIER {
                            pResource: std::mem::ManuallyDrop::new(Some(resource.clone())),
                            Subresource: D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
                            StateBefore: before,
                            StateAfter: after,
                        },
                    ),
                },
            }]);
        }
    }

    /// 后缓冲 `PRESENT → RENDER_TARGET`。
    pub fn to_render_target(list: &ID3D12GraphicsCommandList, back: &ID3D12Resource) {
        Self::barrier(
            list,
            back,
            D3D12_RESOURCE_STATE_PRESENT,
            D3D12_RESOURCE_STATE_RENDER_TARGET,
        );
    }

    /// 后缓冲 `RENDER_TARGET → PRESENT`，交回交换链。
    pub fn to_present(list: &ID3D12GraphicsCommandList, back: &ID3D12Resource) {
        Self::barrier(
            list,
            back,
            D3D12_RESOURCE_STATE_RENDER_TARGET,
            D3D12_RESOURCE_STATE_PRESENT,
        );
    }
}

impl Drop for Gpu {
    fn drop(&mut self) {
        let _ = unsafe { windows::Win32::Foundation::CloseHandle(self.fence_event) };
    }
}

/// 一段描述符堆，外加「第 n 个描述符的 CPU/GPU 句柄」。
///
/// # 为什么自己算而不用迭代器
///
/// 步长由驱动的 `GetDescriptorHandleSize` 决定。本项目的描述符数量在初始化时已知、
/// 之后不变，算一次存起来更省心，也让「第几个」在调用点上一眼可读。
pub struct DescriptorHeap {
    pub heap: ID3D12DescriptorHeap,
    cpu_start: D3D12_CPU_DESCRIPTOR_HANDLE,
    /// GPU 句柄起点。`None` = 该堆**不是** shader visible —— 那种堆**不能**问
    /// GPU 句柄。
    ///
    /// # 为什么 RTV 堆没有 GPU 句柄
    ///
    /// RTV 堆永远不需要 shader visible（着色器看不见渲染目标），所以它的
    /// `Flags` 必然是 `NONE`。而 `ID3D12DescriptorHeap::GetGPUDescriptorHandleForHeapStart`
    /// 对没有 `DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE` 的堆是**非法调用**，调试层会报：
    ///
    /// ```text
    /// GetGPUDescriptorHandleForHeapStart is invalid to call on a descriptor heap
    /// that does not have DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE set. ... If the heap
    /// is not supposed to be shader visible, then GetCPUDescriptorHandleForHeapStart
    /// would be the appropriate method to call.
    /// ```
    ///
    /// 原先无论什么堆都取一次 GPU 句柄，于是每建一个 RTV 堆就多一条消息 —— 而
    /// 「调试层一直有消息」这件事会让人以为后端有问题。
    gpu_start: Option<D3D12_GPU_DESCRIPTOR_HANDLE>,
    stride: u32,
    count: u32,
}

impl DescriptorHeap {
    /// 建 RTV 堆。
    pub fn new_rtv(device: &ID3D12Device, count: u32) -> Result<Self, SurfaceError> {
        Self::new(device, D3D12_DESCRIPTOR_HEAP_TYPE_RTV, count, false)
    }

    /// 建 CBV/SRV/UAV 堆。
    ///
    /// `shader_visible = true` 才能被 `SetGraphicsRootDescriptorTable` 用。
    pub fn new_srv(device: &ID3D12Device, count: u32, shader_visible: bool) -> Result<Self, SurfaceError> {
        Self::new(device, D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV, count, shader_visible)
    }

    fn new(
        device: &ID3D12Device,
        kind: D3D12_DESCRIPTOR_HEAP_TYPE,
        count: u32,
        shader_visible: bool,
    ) -> Result<Self, SurfaceError> {
        let flags = if shader_visible {
            D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE
        } else {
            D3D12_DESCRIPTOR_HEAP_FLAG_NONE
        };
        let heap: ID3D12DescriptorHeap = unsafe {
            device.CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                Type: kind,
                NumDescriptors: count,
                Flags: flags,
                NodeMask: 0,
            })
        }
        .map_err(|e| SurfaceError::new("CreateDescriptorHeap", e))?;

        Ok(Self {
            cpu_start: unsafe { heap.GetCPUDescriptorHandleForHeapStart() },
            // 只在 shader visible 时才问 GPU 句柄 —— 见 `gpu_start` 的说明。
            gpu_start: shader_visible
                .then(|| unsafe { heap.GetGPUDescriptorHandleForHeapStart() }),
            // 名字叫 `GetDescriptorHandleIncrementSize` 而不是 C API 的
            // `GetDescriptorHandleSize` —— windows-rs 0.62 的命名，含义相同。
            stride: unsafe { device.GetDescriptorHandleIncrementSize(kind) },
            heap,
            count,
        })
    }

    /// 第 `i` 个描述符的 CPU 句柄（建视图时用）。
    ///
    /// 越界不夹：返回一个指向堆尾之后的句柄，D3D12 会当场报错，但错误里不会有「第几
    /// 个越界」这种能指到调用点的信息。这里自己挡一次。
    pub fn cpu(&self, i: u32) -> Result<D3D12_CPU_DESCRIPTOR_HANDLE, SurfaceError> {
        self.check(i)?;
        Ok(D3D12_CPU_DESCRIPTOR_HANDLE {
            ptr: self.cpu_start.ptr + i as usize * self.stride as usize,
        })
    }

    /// 第 `i` 个描述符的 GPU 句柄（绑给着色器时用）。
    ///
    /// 堆**不是** shader visible 时直接报错而不是返回垃圾 —— 那条路径只对
    /// `new_srv(device, n, true)` 建的堆成立。
    pub fn gpu(&self, i: u32) -> Result<D3D12_GPU_DESCRIPTOR_HANDLE, SurfaceError> {
        self.check(i)?;
        let start = self.gpu_start.ok_or_else(|| {
            SurfaceError::new(
                "这个描述符堆不是 shader visible，没有 GPU 句柄",
                windows::core::Error::new(
                    E_POINTER,
                    "RTV 堆永远不是 shader visible；GPU 句柄只对 CBV/SRV/UAV + SHADER_VISIBLE 有意义",
                ),
            )
        })?;
        Ok(D3D12_GPU_DESCRIPTOR_HANDLE {
            ptr: start.ptr + i as u64 * self.stride as u64,
        })
    }

    fn check(&self, i: u32) -> Result<(), SurfaceError> {
        if i >= self.count {
            return Err(SurfaceError::new(
                "描述符索引越界",
                windows::core::Error::new(
                    E_POINTER,
                    format!("要第 {i} 个，堆里只有 {} 个", self.count),
                ),
            ));
        }
        Ok(())
    }

    /// 描述符总数。
    pub fn count(&self) -> u32 {
        self.count
    }
}

/// 一块离屏渲染目标：纹理 + 它在 RTV 堆里的 CPU 句柄。
///
/// 存在的理由不是「测试方便」，而是**它是唯一能把「画得出东西」与「交换链/窗口是否
/// 正常」分���验证的手段**。D3D11 侧的排查之所以耗掉那么多轮，就是因为这两件事一直
/// 缠在一起：画面空时，分不清是绘制没生效还是呈现没生效。
pub struct Offscreen {
    pub texture: ID3D12Resource,
    pub rtv: D3D12_CPU_DESCRIPTOR_HANDLE,
    pub size: (u32, u32),
    /// 本目标占用的 RTV 堆（自己建时才有；交换链那条路径是借用外部堆）。
    _heap: Option<DescriptorHeap>,
}

impl Offscreen {
    /// 建 `size` 大小的 RGBA8 离屏目标，自带一个 RTV 堆。
    ///
    /// 用 `R8G8B8A8_UNORM` 而不是 sRGB：着色器输出是**线性**颜色，而回读要逐字节
    /// 比较 —— sRGB 会在写入与读回之间插一段传递函数，让「像素值对不对」这个判据
    /// 变得不可用。
    pub fn new(device: &ID3D12Device, size: (u32, u32)) -> Result<Self, SurfaceError> {
        let heap = DescriptorHeap::new_rtv(device, 1)?;
        let mut me = Self::in_heap(device, size, &heap, 0)?;
        me._heap = Some(heap);
        Ok(me)
    }

    /// 用**外部**RTV 堆里的第 `slot` 个描述符建目标。
    ///
    /// 交换链的后缓冲需要按缓冲个数从同一个堆里取位置，所以堆不能由这里创建。
    pub fn in_heap(
        device: &ID3D12Device,
        size: (u32, u32),
        heap: &DescriptorHeap,
        slot: u32,
    ) -> Result<Self, SurfaceError> {
        if size.0 == 0 || size.1 == 0 {
            return Err(SurfaceError::new(
                "离屏目标尺寸为零",
                windows::core::Error::new(E_POINTER, "zero-sized offscreen target"),
            ));
        }
        let texture = create_resource(
            device,
            &heap_props(D3D12_HEAP_TYPE_DEFAULT),
            // `ALLOW_RENDER_TARGET` 是**必须**的：能不能当 RTV 在创建时就定死了，
            // 少了它连初始状态 `RENDER_TARGET` 都不合法（`E_INVALIDARG`）。
            &texture_desc(size, D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET),
            D3D12_RESOURCE_STATE_RENDER_TARGET,
        )?;
        let rtv = heap.cpu(slot)?;
        unsafe { device.CreateRenderTargetView(&texture, None, rtv) };
        Ok(Self {
            texture,
            rtv,
            size,
            _heap: None,
        })
    }

    /// 整屏裁剪矩形。
    ///
    /// D3D12 的裁剪**永远生效**（没有 `ScissorEnable` 开关），所以这个矩形不给，
    /// 等于「什么都不画且不报错」。
    pub fn scissor(&self) -> RECT {
        RECT {
            left: 0,
            top: 0,
            right: self.size.0 as i32,
            bottom: self.size.1 as i32,
        }
    }

    /// 把内容拷进一张回读纹理并返回像素（`width * height * 4` 字节 RGBA）。
    ///
    /// **记录**拷贝命令，不提交 —— 提交与等待由调用方做（`Gpu::submit` +
    /// `Gpu::wait_gpu`），这样「记录 / 提交 / 等待」三步在测试里都看得见。
    pub fn record_readback(
        &self,
        device: &ID3D12Device,
        list: &ID3D12GraphicsCommandList,
    ) -> Result<Staging, SurfaceError> {
        // ★ 回读目标必须是**缓冲**，不能是纹理。
        //
        // 调试层的原话：
        //     "A texture resource cannot be created on a D3D12_HEAP_TYPE_UPLOAD or
        //      D3D12_HEAP_TYPE_READBACK heap. Investigate CopyTextureRegion to copy
        //      texture data in CPU accessible buffers, ..."
        //
        // 原先这里建的是一张 `R8G8B8A8_UNORM` **纹理**，堆类型 READBACK ——
        // 非法组合，于是 `CreateCommittedResource` 回 `E_INVALIDARG`，而错误本身
        // 不提「纹理不能建在回读堆上」。这是同一个模式的第七次。
        //
        // 正确形态就是那句话里的 `CopyTextureRegion`：源是纹理、目标是**字节缓冲**，
        // 目标侧用 `PlacedFootprint` 说明行距。
        let tight = self.size.0 as u64 * 4;
        let byte_width = tight * self.size.1 as u64;
        let buffer: ID3D12Resource = create_resource(
            device,
            &heap_props(D3D12_HEAP_TYPE_READBACK),
            &crate::renderer::buffer_desc(byte_width),
            D3D12_RESOURCE_STATE_COPY_DEST,
        )?;
        let staging = Staging {
            buffer,
            row_pitch: tight as u32,
            byte_width,
        };

        let (w, h) = self.size;
        unsafe {
            Self::barrier(
                list,
                &self.texture,
                D3D12_RESOURCE_STATE_RENDER_TARGET,
                D3D12_RESOURCE_STATE_COPY_SOURCE,
            );
            list.CopyTextureRegion(
                &D3D12_TEXTURE_COPY_LOCATION {
                    pResource: std::mem::ManuallyDrop::new(Some(staging.buffer.clone())),
                    // ⚠ `Type` 必须与 union 那一半**配套**：
                    // 目标是缓冲 ⇒ `PLACED_FOOTPRINT` + `PlacedFootprint`；
                    // 源是纹理 ⇒ `SUBRESOURCE_INDEX` + `SubresourceIndex`。
                    //
                    // 配错时调试层的原话：
                    //     "D3D12_TEXTURE_COPY_LOCATION::Type can not be
                    //      D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX and used with a
                    //      resource that is D3D12_RESOURCE_DIMENSION_BUFFER.
                    //      D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX can only be used
                    //      with textures."
                    Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
                    Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                        // 目标是缓冲 ⇒ 必须给 `PlacedFootprint`（驱动靠它知道行距）。
                        //
                        // ⚠ `D3D12_TEXTURE_COPY_LOCATION_0` 是 **union**，所以
                        // `SubresourceIndex` 与 `PlacedFootprint` **只能填一个** ——
                        // 两个都写是编译错误。这是 windows-rs 照抄 C 的匿名联合。
                        PlacedFootprint: D3D12_PLACED_SUBRESOURCE_FOOTPRINT {
                            Offset: 0,
                            Footprint: D3D12_SUBRESOURCE_FOOTPRINT {
                                Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                                Width: w,
                                Height: h,
                                Depth: 1,
                                RowPitch: tight as u32,
                            },
                        },
                    },
                },
                0,
                0,
                0,
                &D3D12_TEXTURE_COPY_LOCATION {
                    pResource: std::mem::ManuallyDrop::new(Some(self.texture.clone())),
                    Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
                    Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 {
                        // 源是**纹理** ⇒ 用 `SubresourceIndex`（同一个 union 的另一半）。
                        SubresourceIndex: 0,
                    },
                },
                None,
            );
            Self::barrier(
                list,
                &self.texture,
                D3D12_RESOURCE_STATE_COPY_SOURCE,
                // ⚠ 回到 **`RENDER_TARGET`**，不是 `PRESENT`。
                //
                // `PRESENT` 是**交换链后缓冲**的收尾状态，而这里搬的是离屏目标 ——
                // 下一帧还要继续往它上面画。留在 `PRESENT` 的话，第二帧的
                // `ClearRenderTargetView` 就发生在一张 `PRESENT` 纹理上，验证层报
                // 「is invalid for use as a render target」，而画面在单帧测试里完全正常。
                //
                // ⚠ 这个错误**只在渲第二帧时**出现，所以任何「渲一帧就回读」的夹具都
                // 看不见它 —— 而真实宿主每帧都渲。
                D3D12_RESOURCE_STATE_RENDER_TARGET,
            );
        }
        Ok(staging)
    }

    /// 封装上面的屏障步骤，供 `record_readback` 与测试复用。
    fn barrier(
        list: &ID3D12GraphicsCommandList,
        res: &ID3D12Resource,
        before: D3D12_RESOURCE_STATES,
        after: D3D12_RESOURCE_STATES,
    ) {
        Gpu::barrier(list, res, before, after);
    }

    /// 把回读缓冲映射出来，按行距逐行拷成紧密行距。
    ///
    /// # 行距来自我们自己写下的 footprint，不是从驱动那里问来的
    ///
    /// [`Staging::row_pitch`] 里存的是 `CopyTextureRegion` 时指定的
    /// `D3D12_SUBRESOURCE_FOOTPRINT::RowPitch`，而那次拷贝就是按它落的行，所以回读时
    /// 必然是同一个值。
    ///
    /// 原先这里读 `Map` 之后被「改写」过的 `D3D12_RANGE::End`，本机两次实测都不成立
    /// （它仍是总字节数），于是越界读 → AV。理由见该字段的说明。
    ///
    /// # 越界防护
    ///
    /// `row_pitch < tight` 时**宁可不拷**：那样拿到的是错位的字节，比空结果更难查。
    /// 同时逐行偏移都夹在映射区之内，不做「按总字节数跳行」那种越界算术。
    pub fn map(staging: &Staging, size: (u32, u32)) -> Vec<u8> {
        let tight = size.0 as usize * 4;
        let height = size.1 as usize;
        let total = tight * height;

        let range = D3D12_RANGE {
            Begin: 0,
            End: total,
        };
        let mut ptr = std::ptr::null_mut();
        if unsafe { staging.buffer.Map(0, Some(&range), Some(&mut ptr)) }.is_err() || ptr.is_null() {
            return Vec::new();
        }
        let row_pitch = staging.row_pitch as usize;
        let src = ptr.cast::<u8>();
        let mut out = vec![0u8; total];
        // 越界防护：**按缓冲的真实字节数**判断最后一行的末尾还在里面。
        //
        // 本项目在这里崩过一次 —— 把 `D3D12_RANGE::End`（本机并未改写，仍是总字节数）
        // 当成行距，于是 `src.add(row * row_pitch)` 远远越过缓冲尾部。
        //
        // 「行距不合理时宁可不拷」也是同一个取舍：错位的字节比空结果更难查。
        let last_row_end = row_pitch
            .checked_mul(height.saturating_sub(1))
            .and_then(|o| o.checked_add(tight))
            .unwrap_or(usize::MAX);
        if row_pitch >= tight && last_row_end <= staging.byte_width as usize {
            for row in 0..height {
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        src.add(row * row_pitch),
                        out.as_mut_ptr().add(row * tight),
                        tight,
                    );
                }
            }
        }
        unsafe { staging.buffer.Unmap(0, None) };
        out
    }
}

/// 一次性的回读用缓冲。
///
/// # 为什么是**缓冲**而不是纹理
///
/// `D3D12_HEAP_TYPE_READBACK` / `_UPLOAD` 堆**只能**放缓冲，放纹理是非法组合
/// （`CreateCommittedResource` 回 `E_INVALIDARG`）。像素从纹理搬进 CPU 可读内存的
/// 正规做法是 `CopyTextureRegion`：源是纹理、目标是缓冲，目标侧带 `PlacedFootprint`。
pub struct Staging {
    pub buffer: ID3D12Resource,
    /// 拷贝时**我们**要求的行距（字节）。
    ///
    /// # 为什么不能从 `Map` 的 `D3D12_RANGE::End` 读
    ///
    /// 原先这里读 `Map` 之后被改写过的 `range.End`，当它是行距。本机两次实测都不成立：
    ///
    /// - 它根本没被改写，仍是我们传进去的**总字节数**；
    /// - 于是 `row_pitch` 远大于紧密行距，`copy_nonoverlapping(src.add(row * row_pitch), …)`
    ///   在第二行就越界读 → `STATUS_ACCESS_VIOLATION`。
    ///
    /// 改读这个字段之后，读写都严格落在缓冲内：行距**是**我们在
    /// `D3D12_PLACED_SUBRESOURCE_FOOTPRINT::Footprint.RowPitch` 里写下的那个值，
    /// 驱动的 `CopyTextureRegion` 正是按它落的行。
    pub row_pitch: u32,
    /// 缓冲的**实际**字节数（`ByteWidth`）。越界防护按它判，不按别的推算。
    pub byte_width: u64,
}

/// `width × height` 的 RGBA8 纹理描述。
///
/// # `flags` **必须由调用方给**，不能写死
///
/// 一张纹理能不能当渲染目标，是在**创建时**由 `Flags` 决定的
/// （`D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET`），不是事后能补的。本函数原先把
/// `Flags` 写死成 `NONE`，于是离屏渲染目标建不出来 —— 而且失败点是**初始状态**：
/// `D3D12_RESOURCE_STATE_RENDER_TARGET` 配 `Flags = NONE` 的资源是
/// `E_INVALIDARG`，错误消息只说「参数错误」，不会说是哪个字段、也不知道跟状态有关。
///
/// 顺带一提：这也解释了为什么「建纹理」和「用纹理当 RTV」必须一起看 —— 少了 flag，
/// `CreateCommittedResource` 未必立刻报错，可能要等到状态转换或建视图时才炸。
///
/// # `Layout` 必须是 `UNKNOWN` —— 但**只对纹理**
///
/// 这是 `CreateCommittedResource` 的一条硬要求，且失败方式是 `E_INVALIDARG` —— 没有
/// 任何提示说"是 Layout 写错了"。`ROW_MAJOR` 只对**显式放置**（placed）纹理有意义，
/// 而提交式纹理的布局由驱动决定，描述里必须留 `UNKNOWN`。
/// `Alignment` 同理必须是 0。
///
/// # ⚠ 缓冲的规则**正好相反**：`Layout` 必须写 `ROW_MAJOR`
///
/// 本仓库一度把这句「必须留 UNKNOWN」当成通用规则套到了缓冲上，于是
/// `CreateCommittedResource` 对所有适配器全部 `E_INVALIDARG`，排查了很久。
/// 完整规则见 `renderer::buffer_desc` 的表格，`tests/sweep.rs` 是那个把它查出来的探针。
pub fn texture_desc(size: (u32, u32), flags: D3D12_RESOURCE_FLAGS) -> D3D12_RESOURCE_DESC {
    D3D12_RESOURCE_DESC {
        Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
        Alignment: 0,
        Width: size.0 as u64,
        Height: size.1,
        DepthOrArraySize: 1,
        MipLevels: 1,
        Format: DXGI_FORMAT_R8G8B8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Layout: D3D12_TEXTURE_LAYOUT_UNKNOWN,
        Flags: flags,
    }
}

/// 标准堆属性。
///
/// `MemoryPoolPreference` 留 `UNKNOWN` 而不是选 `ONE`：D3D12 会选适配器最合适的那档，
/// 显式指定反而可能落到核显的系统内存上。
pub fn heap_props(kind: D3D12_HEAP_TYPE) -> D3D12_HEAP_PROPERTIES {
    D3D12_HEAP_PROPERTIES {
        Type: kind,
        CPUPageProperty: D3D12_CPU_PAGE_PROPERTY_UNKNOWN,
        MemoryPoolPreference: D3D12_MEMORY_POOL_UNKNOWN,
        CreationNodeMask: 1,
        VisibleNodeMask: 1,
    }
}

/// 建一个已提交资源。
///
/// # 出参为什么是 `*mut Option<T>`
///
/// D3D12 把结果塞进出参，而 `windows-rs` 0.62 把它泛型化成「由调用方指定接口类型」。
/// 调用点必须写 `&mut out as *mut _`，`out` 的类型就是想要的具体接口。
pub fn create_resource(
    device: &ID3D12Device,
    props: &D3D12_HEAP_PROPERTIES,
    desc: &D3D12_RESOURCE_DESC,
    initial: D3D12_RESOURCE_STATES,
) -> Result<ID3D12Resource, SurfaceError> {
    // 缓冲**只能**建在 `COMMON`。传 `VERTEX_AND_CONSTANT_BUFFER` 之类不会报错 ——
    // 调试层会警告「Ignoring InitialState … Buffers are effectively created in state
    // COMMON」，而资源状态与你以为的不一致会让后面的屏障推理出错。
    let initial = if desc.Dimension == D3D12_RESOURCE_DIMENSION_BUFFER {
        D3D12_RESOURCE_STATE_COMMON
    } else {
        initial
    };

    // 能当渲染目标的纹理给一个优化清屏值。
    //
    // 缺了它，调试层会说「ClearRenderTargetView: The application did not pass any
    // clear value to resource creation. The clear operation is typically slower as a
    // result」。这只是性能提示（驱动少一次快速清零），但调试层**每帧都有一条消息**
    // 会把真正的问题淹掉 —— 而「调试层安静」本身是本项目现在最要紧的可查性前提。
    //
    // ⚠ `Format` **必须与资源的格式同组**，否则调试层报：
    //     "D3D12_CLEAR_VALUE::Format must belong to the same format cast group as
    //      D3D12_RESOURCE_DESC::Format. D3D12_CLEAR_VALUE::Format = UNKNOWN,
    //      D3D12_RESOURCE_DESC::Format = R8G8B8A8_UNORM."
    // 而 `D3D12_CLEAR_VALUE::default()` 的 `Format` 恰好是 `UNKNOWN` —— 所以不能
    // 直接用默认值。
    //
    // 颜色全零即透明黑：本项目所有清屏都是覆盖式的，值选什么都不影响正确性。
    let clear: Option<D3D12_CLEAR_VALUE> = if desc.Flags.0 & D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET.0 != 0 {
        Some(D3D12_CLEAR_VALUE {
            Format: desc.Format,
            Anonymous: D3D12_CLEAR_VALUE_0 {
                Color: [0.0, 0.0, 0.0, 0.0],
            },
        })
    } else {
        None
    };

    let mut out = None::<ID3D12Resource>;
    unsafe {
        device.CreateCommittedResource(
            props,
            D3D12_HEAP_FLAG_NONE,
            desc,
            initial,
            clear.as_ref().map(|c| c as *const _),
            &mut out as *mut _,
        )
    }
    .map_err(|e| SurfaceError::new("CreateCommittedResource", e))?;
    out.ok_or_else(|| {
        SurfaceError::new(
            "CreateCommittedResource 成功却没给资源",
            windows::core::Error::new(E_FAIL, "no resource returned"),
        )
    })
}

/// 挑一个能用的适配器。
///
/// # 为什么不能直接拿 `EnumAdapters1(0)`
///
/// DXGI 的枚举顺序**不是**「独显优先」，而是驱动注册顺序。装了虚拟显示适配器
/// （远程桌面 / 投屏 / 串流软件都会注册一个）之后，0 号往往正是它。
///
/// 本项目在这台机器上实测：0 号是 `GameViewer Virtual Display Adapter`（软件虚拟
/// 显示设备，没有真正的 GPU 后端），**任何用到离屏 RT 的帧都会把设备搞挂** ——
/// `GetDeviceRemovedReason()` 返回 `DXGI_ERROR_DEVICE_HUNG`。而同一份代码在 1 号
/// （NVIDIA）上跑得完全正常。
///
/// 症状极具欺骗性：像素**完全正确**，`submit` 与 `wait_gpu` 都返回成功，只是设备在
/// 帧尾被摘掉；所以「单看一帧」永远看不出问题，只有**下一帧新建设备**时报
/// `D3D12CreateDevice` 失败才暴露。
///
/// # 规则
///
/// 1. `$env:ZEPHYRA_ADAPTER = <序号>` 直接指定（排查用，不做下面的筛选）；
/// 2. 否则按「**有独显显存** > **非软件** > 第一个」的顺序取第一个满足的：
///    - 跳过 [`DXGI_ADAPTER_FLAG_SOFTWARE`]（那就是 WARP）；
///    - 跳过 `DedicatedVideoMemory == 0` 的 —— 虚拟显示适配器（投屏 / 串流 / 远程桌面
///      都会注册一个）报的正是 0，而集显在部分驱动上也报 0，所以这条**只是排序偏好**，
///      不是硬性排除；
/// 3. 一个都没满足就退回第一个非软件适配器，再不行就退回 0 号（让报错来自
///    `D3D12CreateDevice`，而不是一句「找不到适配器」）。
///
/// 本机实测（`GameViewer Virtual Display Adapter` + NVIDIA MX330 + Intel UHD）：
/// 2 号会挂、1 号正常、0 号（虚拟显示适配器）也挂。
fn pick_adapter(
    factory: &windows::Win32::Graphics::Dxgi::IDXGIFactory4,
) -> Result<IDXGIAdapter1, windows::core::Error> {
    if let Some(idx) = std::env::var("ZEPHYRA_ADAPTER")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
    {
        return unsafe { factory.EnumAdapters1(idx) };
    }
    let mut first = None::<IDXGIAdapter1>;
    let mut first_hw = None::<IDXGIAdapter1>;
    // 取**独显显存最大**的那一个，而不是第一个显存 > 0 的 —— 虚拟显示适配器有时也会
    // 报一个非零但很小的数值（它可能借用系统内存），按顺序挑就会挑中它。
    let mut best_vram = 0usize;
    let mut best = None::<IDXGIAdapter1>;
    for i in 0.. {
        let Ok(adapter) = (unsafe { factory.EnumAdapters1(i) }) else {
            break;
        };
        if first.is_none() {
            first = Some(adapter.clone());
        }
        // 拿不到描述就当它不是软件适配器 —— 保守地继续用，总比直接放弃好。
        let (is_software, vram) = match unsafe { adapter.GetDesc1() } {
            Ok(d) => (
                d.Flags
                    & windows::Win32::Graphics::Dxgi::DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32
                    != 0,
                d.DedicatedVideoMemory,
            ),
            Err(_) => (false, 0),
        };
        if is_software {
            continue;
        }
        if first_hw.is_none() {
            first_hw = Some(adapter.clone());
        }
        if vram > best_vram {
            best_vram = vram;
            best = Some(adapter);
        }
    }
    best.or(first_hw)
        .or(first)
        .ok_or_else(|| windows::core::Error::new(DXGI_ERROR_NOT_FOUND, "枚举不到适配器"))
}

/// 建交换链 / 访问设备资源时的错误。
#[derive(Debug)]
pub struct SurfaceError {
    /// 失败发生在哪一步。
    ///
    /// `String` 而非 `&'static str`：D3D12 很多 API 失败时**必须**带上参数才能定位
    /// （`GetBuffer(3)`、`第 5 个描述符越界`）。固定生命周期字符串会逼着调用点把
    /// 索引丢掉，只留一句「GetBuffer 失败」—— 而那正是本项目查了六轮的那类问题。
    what: String,
    source: windows::core::Error,
}

impl SurfaceError {
    pub fn new(what: impl Into<String>, source: windows::core::Error) -> Self {
        Self {
            what: what.into(),
            source,
        }
    }

    pub fn factory(source: windows::core::Error) -> Self {
        Self::new("CreateDXGIFactory1", source)
    }

    /// 一个**自己造成的**非法用法错误（与 `windows::core::Error` 无关）。
    ///
    /// # 为什么不复用 `Self::new`
    ///
    /// `Self::new` 的 `source` 是 `windows::core::Error` —— 它只装得下「某个 API 返回了
    /// 某个 HRESULT」。而「上传暂存区不够」「描述符槽位越界」这类错误**没有任何 API
    /// 失败**，硬塞一个假 HRESULT 会让日志里出现一条与真实原因无关的 `E_FAIL`，
    /// 排查时反而要多跳一步确认它是不是真的 API 失败。
    pub fn invalid(what: impl Into<String>) -> Self {
        Self {
            what: what.into(),
            source: windows::core::Error::new(
                windows::Win32::Foundation::E_FAIL,
                "调用方用错了 API，不是 D3D12 调用失败",
            ),
        }
    }
}

impl std::fmt::Display for SurfaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}失败: {}", self.what, self.source)
    }
}

impl std::error::Error for SurfaceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}
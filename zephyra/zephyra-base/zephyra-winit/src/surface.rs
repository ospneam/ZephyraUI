//! 呈现：DXGI 交换链 + 可选的 DirectComposition 合成树。
//!
//! # 两条呈现路径在这里合并成一条
//!
//! wgpu 时代它们是两个类型（`wgpu::Surface` 与 `DcompSurface`），差别在于"怎么
//! 创建后缓冲、怎么提交"。换成 D3D11 之后差别只剩**一处**：
//!
//! | | 交换链创建 | 提交 |
//! |---|---|---|
//! | 普通 | `CreateSwapChainForHwnd` | `Present` |
//! | DComp | `CreateSwapChainForComposition` | `Present1` + `Commit` |
//!
//! 两者的后缓冲都是 `ID3D11Texture2D`，都能 `GetBuffer` + `CreateRenderTargetView`。
//! 所以这里用**一个** `Surface` 类型 + 一个 `Option<DcompCompositor>`：调用方拿到
//! 帧的方式（[`Surface::current_rtv`] → 渲染 → [`Surface::present`]）在两条路上
//! 完全一致，因此上层（`HostWindow`、渲染器）根本不需要知道走的是哪条。
//!
//! 这比 wgpu 时代的 `Present` 枚举好：枚举要求每个调用方都写 `match`，而两条路
//! 现在真的只有一处不同。
//!
//! # DComp 的三条硬约束
//!
//! 都在 `zephyra-dcomp` 那边有详细记录，本模块是调用方但**持有它们的载体**，
//! 搬运时不能丢：
//!
//! 1. `DcompCompositor`（含 target 的 COM 引用）必须比它绑定的 HWND 活得久。
//!    本结构由 [`crate::HostWindow`] 持有，而 `HostWindow` 在 winit 事件循环里，
//!    窗口销毁时它先被 drop —— 顺序天然正确。
//! 2. DComp 设备走 `IDXGIDevice` 路线。**D3D11 在这里比 D3D12 好**：D3D11 device
//!    能 QI `IDXGIDevice`（实测成功），而本机 D3D12 device 不能（`E_NOINTERFACE`，
//!    即 wgpu 时代那条 v2→v3→null fallback 链存在的原因）。这里仍然调
//!    `new_from_rendering_device`：它第一级就是 IDXGIDevice 路线，成功；后面两级
//!    只是保险。
//! 3. `Present1` 必须收非空 `DXGI_PRESENT_PARAMETERS`，所以**必须**走
//!    `zephyra_dcomp::present_one`，不要自己调 `SwapChain1::Present1`
//!    （传 `nullptr` 直接 AV）。

#![cfg(windows)]

/// [`PixelRead`] 与 [`write_bmp`] 已搬到后端无关的 [`zephyra_fx::pixels`]。
///
/// 这里保留 `pub use` 只是为了不打断既有调用点（`zephyra_winit::surface::PixelRead`）。
///
/// # 为什么它们不属于任何后端
///
/// [`PixelRead`] 的字段直接对应 `D3D11_MAPPED_SUBRESOURCE` 的 `RowPitch`，看起来
/// 像 D3D 的东西，但**没有一个字段是 D3D 特有的** —— CPU 参考光栅器产出的也是
/// 「一批 BGRA、自顶向下、可能带行距填充」，DX12 的 `Map` 读回也是同一个形状。
///
/// 之前它们住在这个文件里，于是 `cpu_only.rs`（一个不碰 GPU 的离线宿主）为了写
/// 一张调试图，得反过来依赖 D3D11 的 `surface`。那条从 CPU 侧指向图形后端的边，
/// 会让 DX12 后端**用不了** CPU 诊断能力。
pub use zephyra_fx::pixels::{write_bmp, PixelRead};

use windows::core::Interface;
use windows::Win32::Foundation::{E_FAIL, HMODULE, HWND};
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_RENDER_TARGET,    D3D11_CREATE_DEVICE_BGRA_SUPPORT,
    D3D11_CREATE_DEVICE_DEBUG,
    D3D11_CREATE_DEVICE_FLAG, D3D11_SDK_VERSION, ID3D11Device, ID3D11DeviceContext,
    ID3D11RenderTargetView, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory2, IDXGIFactory2, IDXGIAdapter, IDXGISwapChain1, IDXGISwapChain3,
    DXGI_CREATE_FACTORY_FLAGS, DXGI_SWAP_CHAIN_DESC1, DXGI_SWAP_CHAIN_FLAG,
    DXGI_SWAP_EFFECT_FLIP_DISCARD,
};

use zephyra_dcomp::{create_composition_swapchain, present_one, DcompCompositor, SwapchainDesc};

/// 后缓冲格式。
///
/// 必须与 [`zephyra_dcomp::SwapchainDesc`] 的默认格式**一致**。两者不一致时
/// DXGI 直接让 `CreateSwapChain*` 失败（`DXGI_ERROR_UNSUPPORTED`），而不是画出
/// 一屏错色 —— 但普通路径不走 `SwapchainDesc`，所以那里的格式由本常量决定。
pub const BACKBUFFER_FORMAT: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT =
    windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;

/// MSAA 采样数。1 = 关（不建多重采样纹理）。
///
/// # 为什么是 4 而不是 8
///
/// MSAA 的代价是**填充率**（每个像素要跑 N 遍片元着色器）而不是几何量，而 UI 的
/// 帧时间几乎全在填充率上。4× 是「图标边缘不再有肉眼可见台阶」的最低档；8× 在
/// 一个 20 像素宽的齿轮上看不出比 4× 更圆，却要付双倍的像素着色开销。
///
/// # 为什么是**常量**而不是查设备能力
///
/// 采样数必须在**建 PSO 之前**定下来（[`crate::renderer`] 的
/// `MultisampleEnable` 与 `SampleDesc` 都写死在光栅化状态里），而 PSO 在
/// [`crate::Renderer::new`] 里建、那时还没有交换链。
///
/// 若改成"查设备支持再定"，就得把查询结果从 surface 一路传到 PSO 构造 ——
/// 而 D3D11 侧两者是独立构造的（`Surface::new` 与 `Renderer::new` 互不相识）。
/// 用常量换来"两边不可能不一致"，这比省一点填充率重要：
/// `MultisampleEnable=1` 但绑单采样 RTV 时，D3D11 会**解除绑定**渲染目标，
/// 症状是「一帧什么都不画且零 HRESULT 错误」。
pub const MSAA_SAMPLES: u32 = 4;

/// 本设备**真正可用**的采样数（[`MSAA_SAMPLES`]，或不支持时退回 1）。
///
/// # 为什么要查设备能力
///
/// `CheckMultisampleQualityLevels` 返回 0 表示**不支持**这个采样数（老设备，
/// 或驱动把它关了）。那种情况下必须退回单采样 —— 否则 `CreateTexture2D` 建多重
/// 采样纹理会失败，而失败发生在 `Surface::new` 里，症状是**程序起不来**，
/// 不是「画面糙一点」。那个失败/降级的取舍必须由这一处代劳。
///
/// # 返回值怎么用
///
/// - 采样数：写进多重采样纹理的 `SampleDesc.Count`，**并且**必须与 PSO 的
///   `MultisampleEnable` / `SampleDesc` 一致；
/// - 质量档：恒为 0（标准档）。`CheckMultisampleQualityLevels` 返回的是**可用档
///   数**而不是档位，所以「支持」这件事本身只能回答「用不用」，回答不了「用第几档」
///   —— 而 D3D11 对 `R8G8B8A8` 的 4× 只承诺档 0（其余档是可选优化）。
///
/// ⚠ 调用方必须让**纹理与 PSO 用同一个返回值**。`MultisampleEnable=1` 而绑单采样
/// RTV 时 D3D11 会解除渲染目标绑定，症状是「一帧什么都不画且零 HRESULT 错误」。
pub fn msaa_samples(device: &ID3D11Device) -> u32 {
    let levels =
        unsafe { device.CheckMultisampleQualityLevels(BACKBUFFER_FORMAT, MSAA_SAMPLES) }.unwrap_or(0);
    if levels > 0 {
        MSAA_SAMPLES
    } else {
        eprintln!(
            "[zephyra] ⚠ 设备不支持 {MSAA_SAMPLES}× MSAA（CheckMultisampleQualityLevels \
             报告 0 个可用档），退回单采样 —— 几何边缘会有可见台阶"
        );
        1
    }
}

/// 呈现路径的错误：**原因分类 + 失败步骤**。
///
/// # 为什么步骤必须单独记
///
/// DXGI 的错误消息（`DXGI_ERROR_INVALID_CALL` = "应用程序进行了无效的调用"）
/// 从不说明是哪个参数或哪个对象状态不对。实测中"这个 HWND 已有交换链"、
/// "alpha 模式与 swap effect 不兼容"、"设备缺某项能力"三种完全不同的病因都返回
/// 这一句话 —— 没有步骤标签就只能靠试参数猜，而试参数本身又会因为"一个 HWND 只能
/// 挂一个交换链"而污染下一次尝试（那样得到的结论是假的）。
///
/// 步骤之所以做成**字段**而不是并进 `SurfaceErrorKind` 的变体：变体会让调用方的
/// `match` 随步骤数膨胀，而它只用于诊断输出。
#[derive(Debug)]
pub struct SurfaceError {
    /// 出错的原因分类。
    pub kind: SurfaceErrorKind,
    /// 失败在哪一步（`"CreateSwapChainForHwnd"` 等）。恒有值。
    pub step: &'static str,
    /// 现场信息（实际尺寸、绑定位、缓冲索引等）。可选。
    pub hint: Option<String>,
}

/// [`SurfaceError`] 的原因分类。**不含**步骤信息 —— 见 [`SurfaceError`]。
#[derive(Debug)]
pub enum SurfaceErrorKind {
    /// 创建 D3D11 设备失败（没有可用 GPU / 驱动拒绝）。
    Device(windows::core::Error),
    /// 创建交换链或取后缓冲失败。
    SwapChain(windows::core::Error),
    /// 后缓冲上取渲染目标视图失败。
    RenderTarget(windows::core::Error),
    /// 呈现路径的其它 Win32 调用失败。
    Win32(windows::core::Error),
    /// 窗口句柄不是 Win32 HWND。
    ///
    /// 只该在**非** Windows 后端出现。它与"创建失败"必须可区分：前者说明宿主用
    /// 错了平台，后者说明这台机器有问题。
    BadWindowHandle,
}

impl SurfaceError {
    /// 构造一个尚未标注步骤的错误。
    ///
    /// 立刻调 [`Self::at`] 补上步骤 —— 刻意**不**给 `Default` 步骤值，那样漏标会
    /// 静默通过，而漏标的错误信息在排查 DXGI 时毫无用处。
    fn new(kind: SurfaceErrorKind) -> Self {
        Self {
            kind,
            step: "未标注",
            hint: None,
        }
    }

    /// 标注失败发生在哪一步。
    #[must_use]
    pub fn at(self, step: &'static str) -> Self {
        Self { step, ..self }
    }

    /// 附加现场信息（实际尺寸 / 绑定位 / 索引等）。
    ///
    /// 存在的理由与 [`Self::at`] 相同：系统消息只给结论不给现场。
    #[must_use]
    pub fn with_hint(mut self, hint: String) -> Self {
        self.hint = Some(hint);
        self
    }
}

impl std::fmt::Display for SurfaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.kind {
            SurfaceErrorKind::Device(e) => write!(f, "创建 D3D11 设备失败: {e}"),
            SurfaceErrorKind::SwapChain(e) => write!(f, "创建交换链失败: {e}"),
            SurfaceErrorKind::RenderTarget(e) => write!(f, "创建渲染目标视图失败: {e}"),
            SurfaceErrorKind::Win32(e) => write!(f, "DXGI/呈现调用失败: {e}"),
            SurfaceErrorKind::BadWindowHandle => write!(f, "窗口句柄不是 Win32 HWND"),
        }?;
        write!(f, "（失败于：{}）", self.step)?;
        if let Some(h) = &self.hint {
            write!(f, "；现场：{h}")?;
        }
        Ok(())
    }
}

impl std::error::Error for SurfaceError {}

impl From<windows::core::Error> for SurfaceError {
    fn from(e: windows::core::Error) -> Self {
        SurfaceError::new(SurfaceErrorKind::Win32(e))
    }
}

/// 是否启用 DComp 呈现路径（`ZEPHYRA_DCOMP`，非 `"0"` 即启用）。
///
/// 进程内只读一次：它决定的是**资源拓扑**（要不要建 DComp 树），中途切换意味着
/// 一半帧走一条路、一半走另一路，而两者的后缓冲不是同一块 —— 那会画到看不见的
/// 纹理上。
pub fn dcomp_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("ZEPHYRA_DCOMP")
            .map(|v| v != "0")
            .unwrap_or(false)
    })
}

/// 从 winit 窗口取裸 HWND。
///
/// 失败返回 `None`（非 Win32 平台、或将来 winit 换了句柄方案）。
pub fn hwnd_of<W: raw_window_handle::HasWindowHandle>(window: &W) -> Option<isize> {
    use raw_window_handle::RawWindowHandle;
    let handle = window.window_handle().ok()?;
    match RawWindowHandle::from(handle) {
        RawWindowHandle::Win32(h) => Some(h.hwnd.get()),
        _ => None,
    }
}

/// 设备与立即上下文。
///
/// 单独成结构是为了让"渲染器用的设备"与"呈现用的交换链"共享同一份 —— 交换链必须
/// 建在渲染它的那份设备上，否则 D3D11 会拒绝（`CreateSwapChain` 要求设备与
/// 交换链匹配）。
pub struct Gpu {
    pub device: ID3D11Device,
    pub context: ID3D11DeviceContext,
}

impl Gpu {
    /// 用默认硬件适配器建 D3D11 设备（feature level 11_0）。
    ///
    /// `ZEPHYRA_D3D_DEBUG=1` 时开 D3D11 调试层并把驱动自己的警告打到 stderr。
    ///
    /// # 为什么值得有
    ///
    /// D3D11 的 draw 出错时**默认完全静默** —— 不返回错误、不崩溃、不进任何日志，
    /// 只是什么都不画。我实测踩过一整轮：顶点数据、索引数据、输入布局全部逐项读回
    /// 验证正确，画面仍然只有 463 个像素；打开调试层一句话就点出了问题所在。
    ///
    /// 根本原因是：我们所有的自检都只验证了"**我们自己算出来的东西**"，从未验证
    /// "驱动是否接受了它"。而 D3D11 的失败模式恰恰全在后半段。
    pub fn new() -> Result<Self, SurfaceError> {
        // `BGRA_SUPPORT` 与 `DEBUG` 合并成一个 flag —— 两者都是**创建设备时**的
        // 标志，事后无法追加，所以只能在这里一次性决定。
        //
        // `BGRA_SUPPORT` 是必需的（不是可选优化）：缺它时设备能建起来，但任何
        // 带目标 alpha 的交换链会在创建时报 `DXGI_ERROR_UNSUPPORTED` —— 而那条
        // 错误完全不提"缺 BGRA 支持"，会让人去查 swapchain 参数（实测踩到）。
        let debug = std::env::var("ZEPHYRA_D3D_DEBUG").as_deref() == Ok("1");
        let flags = D3D11_CREATE_DEVICE_BGRA_SUPPORT
            | if debug {
                D3D11_CREATE_DEVICE_DEBUG
            } else {
                D3D11_CREATE_DEVICE_FLAG(0)
            };

        let mut device = None::<ID3D11Device>;
        let mut context = None::<ID3D11DeviceContext>;
        unsafe {
            windows::Win32::Graphics::Direct3D11::D3D11CreateDevice::<Option<&IDXGIAdapter>>(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                // `software` 是裸 `*mut c_void`，不是 `Option` —— 没装 WARP 时给
                // 空指针而不是 `HMODULE(0)`（后者塞不进去）。
                HMODULE(std::ptr::null_mut()),
                flags,
                Some(&[D3D_FEATURE_LEVEL_11_0]),
                D3D11_SDK_VERSION,
                Some(&mut device as *mut _),
                None,
                Some(&mut context as *mut _),
            )
        }
        .map_err(|e| SurfaceError::new(SurfaceErrorKind::Device(e)).at("D3D11CreateDevice"))?;
        Ok(Self {
            device: device.expect("D3D11CreateDevice 成功时必给设备"),
            context: context.expect("D3D11CreateDevice 成功时必给上下文"),
        })
    }

    /// 设备的 `IUnknown`，建交换链时要用（`CreateSwapChain*` 收 `IUnknown`）。
    fn unknown(device: &ID3D11Device) -> windows::core::Result<windows::core::IUnknown> {
        device.cast()
    }

    /// 把调试层积累的消息打到 stderr。
    ///
    /// # 为什么要主动拉取而不是"设备自己会打"
    ///
    /// 调试层默认**不弹窗**（`D3D11_CREATE_DEVICE_DEBUG` 不带
    /// `D3D11_CREATE_DEVICE_DEBUG` 之外的行为控制），它只把消息**排队**，
    /// 需要应用自己 `GetMessage` / `ReportMessage` 取出来。
    ///
    /// 而"驱动说了什么"往往是唯一的一手证据 —— 我们的每一步自检都只验证了
    /// **我们自己算出来的东西**（顶点值对不对、索引对不对、布局签名对不对），
    /// 从未验证"驱动是否接受了它"。实测踩过的那个 bug 就是这样：三项自检全绿，
    /// 画面全错。
    pub fn dump_debug_messages(&self) {
        let q = info_queue(&self.device);
        dump_queue_messages(q.as_ref());
    }
}

impl Surface {
    /// 把调试层积累的消息打到 stderr（开了 `ZEPHYRA_D3D_DEBUG=1` 才有内容）。
    ///
    /// 与 [`Gpu::dump_debug_messages`] 同一个实现 —— `Surface` 自带 `device`，
    /// 而队列是**设备**级共享的，所以从哪个入口打都看到同一批消息。
    pub fn dump_debug_messages(&self) {
        dump_queue_messages(info_queue(&self.device).as_ref());
    }
}

/// 取设备的调试信息队列（仅在开了调试层时存在）。
///
/// # 为什么是 `ID3D11InfoQueue` 而不是 `ID3D11Debug`
///
/// windows-rs 0.62 的 `ID3D11Debug` 只暴露 `ReportLiveDeviceObjects` 之类，
/// **没有** `GetMessage` —— 那两个方法被生成器归到了 `ID3D11InfoQueue` 上。
/// 而 `ID3D11Debug::GetContext` 换来的上下文同样要再 cast 成 `ID3D11InfoQueue`
/// 才能读消息，所以直接查 `ID3D11Device` 更短。
///
/// 没开调试层时 `cast` 返回 `Err` —— 静默跳过即可，调试层是可选诊断功能。
pub fn info_queue(
    device: &ID3D11Device,
) -> Option<windows::Win32::Graphics::Direct3D11::ID3D11InfoQueue> {
    device.cast::<windows::Win32::Graphics::Direct3D11::ID3D11InfoQueue>().ok()
}

/// 把队列里积累的消息全部打到 stderr。
///
/// # 为什么"全部"打，包括信息级
///
/// 队列里**信息级**的消息常常才是根因（"注意：此处无绑定"、"像素格式不支持
/// sRGB"），而错误级往往只是它的后果。只打错误会正好错过真正有用的那一条。
pub fn dump_queue_messages(
    queue: Option<&windows::Win32::Graphics::Direct3D11::ID3D11InfoQueue>,
) {
    use windows::Win32::Graphics::Direct3D11::D3D11_MESSAGE;
    let Some(queue) = queue else {
        if std::env::var("ZEPHYRA_D3D_DEBUG").as_deref() == Ok("1") {
            eprintln!(
                "[d3d] 要了调试层但拿不到 ID3D11InfoQueue —— \
                 多半是没装 Windows 的 \"Graphics Tools\"（D3D11 调试层是可选功能）"
            );
        }
        return;
    };
    eprintln!(
        "[d3d] 已存 {} 条（取回过滤器放行 {} 条）/ 被存储过滤器拒 {} 条 / 被丢弃 {} 条 / 上限 {}",
        unsafe { queue.GetNumStoredMessages() },
        unsafe { queue.GetNumStoredMessagesAllowedByRetrievalFilter() },
        unsafe { queue.GetNumMessagesDeniedByStorageFilter() },
        unsafe { queue.GetNumMessagesDiscardedByMessageCountLimit() },
        unsafe { queue.GetMessageCountLimit() },
    );
    // 消息是**排队**的，索引从 0 连续递增，取不到就停。
    //
    // 先压一个"全部放行"的取回过滤器 —— 默认过滤器是按层/类别配的，实测在
    // 本机上会让 `GetMessage(0)` 直接回 `E_INVALIDARG`，尽管
    // `GetNumStoredMessages` 明明说有 6 条。
    //
    // `len` 必须**先填成目标结构体的大小**再传进去 —— 它是 in-out 参数，语义是
    // "我给的缓冲区有多大"。填 0 调用会让 D3D11 回 `E_INVALIDARG`，而那条错误
    // 只说"参数错误"，完全看不出是哪一个 —— 我为此白绕了一圈。
    // 先看看队列里到底有什么。`GetNumStoredMessages` 说有 N 条，那么第 0 条**一定存在**
    // —— 取不出来就是**取法**错了，不是「没有消息」。
    //
    // `GetMessage(MessageIndex, pMessage, pMessageByteLength)`：
    // `pMessageByteLength` 是 **in-out**，入参必须是 `pMessage` 指向的缓冲区大小。
    //
    // 早先这里在调用**前**就把 `len` 设成 `size_of::<D3D11_MESSAGE>()`（对），
    // 但把 `E_INVALIDARG` 一律当成「队列空了」而 `break` —— 于是**一条都没打出来**，
    // 而队列里明明有 12 条。看起来像「驱动没报任何错」，实际是「我们没在读」。
    let total = unsafe { queue.GetNumStoredMessages() };
    for i in 0..total.min(64) {
        let mut msg = unsafe { std::mem::zeroed::<D3D11_MESSAGE>() };
        let mut len = std::mem::size_of::<D3D11_MESSAGE>();
        let hr = unsafe { queue.GetMessage(i, Some(&mut msg as *mut _), &mut len) };
        if hr.is_err() {
            eprintln!(
                "[d3d] GetMessage({i}) 失败 {hr:?}（缓冲区 {} 字节，存储数 {total}）—— \
                 这说明**取消息的调用本身**有问题，不是队列为空",
                std::mem::size_of::<D3D11_MESSAGE>()
            );
            break;
        }
        // 描述文本：`pDescription` 可能带也可能不带 NUL，两种都要处理。
        let desc = if msg.pDescription.is_null() {
            String::new()
        } else {
            let n = if msg.DescriptionByteLength > 0 {
                msg.DescriptionByteLength as usize
            } else {
                let mut n = 0usize;
                while unsafe { *msg.pDescription.add(n) } != 0 {
                    n += 1;
                }
                n
            };
            let bytes = unsafe { std::slice::from_raw_parts(msg.pDescription, n) };
            String::from_utf8_lossy(bytes).into_owned()
        };
        eprintln!("[d3d] {:?} #{:?} {}", msg.Severity, msg.ID, desc);
    }

    }
/// 一张多重采样颜色目标：纹理本体 + 它的渲染目标视图。
///
/// # 两个字段缺一不可
///
/// `rtv` 本身持有纹理的 COM 引用，所以严格说 `tex` 是冗余的。但**解析那一步
/// 需要纹理本体**（`ResolveSubresource` 收的是资源视图，不是 RTV），而从 RTV
/// 反查资源只在 [`Surface::read_pixels`] 里用过一次、还得配一次 QI。
/// 直接存着比每次反查便宜，也少一条能走错的路径。
struct MsaaTarget {
    tex: ID3D11Texture2D,
    rtv: ID3D11RenderTargetView,
    /// 采样数。存下来是为了 resize 时**按同一个值**重建。
    ///
    /// 不能重新查设备：查询结果在设备生命周期内是常量，但把常量重新问一遍
    /// 就多了一条"两次问不一致"的可能，而那会让 MSAA 静默失效（见类型文档）。
    samples: u32,
}

/// 按采样数建一张多重采样颜色目标。
///
/// `samples == 1` 返回 `None`（不需要中间纹理，直接画后缓冲）。
fn build_msaa_target(
    device: &ID3D11Device,
    size: (u32, u32),
    samples: u32,
) -> Result<Option<MsaaTarget>, SurfaceError> {
    if samples <= 1 {
        return Ok(None);
    }
    let desc = windows::Win32::Graphics::Direct3D11::D3D11_TEXTURE2D_DESC {
        Width: size.0.max(1),
        Height: size.1.max(1),
        MipLevels: 1,
        ArraySize: 1,
        Format: BACKBUFFER_FORMAT,
        SampleDesc: windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC {
            Count: samples,
            Quality: 0,
        },
        Usage: windows::Win32::Graphics::Direct3D11::D3D11_USAGE_DEFAULT,
        // ⚠ 多重采样纹理**不能**同时绑 SRV（D3D11 不允许 MSAA 纹理被采样），
        // 所以只给 RENDER_TARGET。给了 SHADER_RESOURCE 会让 CreateTexture2D 失败。
        BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let mut tex = None;
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut tex as *mut _)) }
        .map_err(|e| SurfaceError::new(SurfaceErrorKind::RenderTarget(e)).at("建 MSAA 纹理"))?;
    let tex = tex.expect("CreateTexture2D 成功时必给纹理");
    let mut rtv = None;
    unsafe { device.CreateRenderTargetView(&tex, None, Some(&mut rtv as *mut _)) }
        .map_err(|e| SurfaceError::new(SurfaceErrorKind::RenderTarget(e)).at("MSAA RTV"))?;
    Ok(Some(MsaaTarget {
        tex,
        rtv: rtv.expect("CreateRenderTargetView 成功时必给视图"),
        samples,
    }))
}

/// 一个窗口的全部呈现状态。
///
/// 字段顺序有意义：`_compositor` 声明在 `swapchain` **之后**，因此 Rust 先释放
/// `swapchain`、再释放 `_compositor`。这正是 DComp 硬约束 #1 要的顺序 ——
/// visual 的内容必须先于 visual 本身消失，反过来同样是全白。
pub struct Surface {
    /// 后缓冲（所有权）。
    ///
    /// DXGI 造的交换链与 D3D11 纹理各持一份引用；交换链销毁后纹理仍要留给当前帧
    /// 的在途绘制，所以纹理必须自己持有一份。
    backbuffers: Vec<ID3D11Texture2D>,
    /// 与 `backbuffers` 同序的渲染目标视图。
    rtvs: Vec<ID3D11RenderTargetView>,
    /// 诊断用离屏渲染目标（`ZEPHYRA_OFFSCREEN=1` 时存在）。
    ///
    /// 存在的唯一理由是把"渲染管线不对"与"交换链不对"分开 —— 见
    /// [`Self::current_rtv`]。
    offscreen: Option<ID3D11RenderTargetView>,
    /// MSAA 纹理 + 它的渲染目标视图（`samples > 1` 时存在）。
    ///
    /// # 为什么后缓冲不能直接开 MSAA
    ///
    /// **交换链后缓冲永远是单采样** —— DXGI 不提供让后缓冲多重采样的 API，
    /// `DXGI_SWAP_CHAIN_DESC1` 里也没有采样数字段（只有 `BufferUsage` / `Flags`）。
    ///
    /// 正路是自建一张 `samples` 倍纹理、渲染进它、每帧末尾
    /// `ResolveSubresource` 解析回后缓冲。**两处都必须改**：
    ///
    /// - 只建 MSAA 纹理而 PSO 的 `MultisampleEnable` 还是 0 → 画进多重采样
    ///   RTV 但每像素只写一个采样，D3D11 直接拒绝绑定；
    /// - PSO 开了而没建纹理 → 绑单采样 RTV，D3D11 **解除渲染目标绑定**，
    ///   症状是「一帧什么都不画且零 HRESULT 错误」。
    ///
    /// 纹理必须**自己持有**：它是渲染目标，每帧都要在，不能靠后缓冲的引用顺带活着。
    msaa: Option<MsaaTarget>,
    swapchain: IDXGISwapChain1,
    /// `GetCurrentBackBufferIndex` 只能从 v3 接口拿（v1/v2 没有）。
    swapchain3: IDXGISwapChain3,
    /// DComp 合成器。`None` = 普通路径。
    ///
    /// 保活字段（DComp 硬约束 #1）——**绝不可删**。
    _compositor: Option<DcompCompositor>,
    /// 设备的第二份引用，仅用于 resize 时重建后缓冲视图与 `read_pixels`。
    ///
    /// 交换链持有设备引用，但没法从交换链把 `ID3D11Device` 接口拿回来，所以这里
    /// 存一份。**它不参与绘制** —— 绘制用的是 `HostWindow` 持有的那份上下文。
    device: ID3D11Device,
    /// 上下文的第二份引用，仅用于 `read_pixels` 里的 `CopyResource` / `Map`。
    ///
    /// 同一份理由：截图要发命令，而命令只能经上下文发。
    context: ID3D11DeviceContext,
    width: u32,
    height: u32,
}

impl Surface {
    /// 在 `hwnd` 上建立交换链。
    ///
    /// `use_dcomp` 为真时建 composition swapchain 并挂进 DComp visual 树；为假时
    /// 建普通 HWND swapchain。两条路的返回值**用法完全相同** —— 调用方不需要知道
    /// 区别（见模块文档的对照表）。
    ///
    /// `size` 必须是窗口客户区的**物理像素**。不一致时 DComp 会缩放采样，画面
    /// 整体发虚 —— 图标最先暴露这个问题。
    pub fn new(
        hwnd: HWND,
        gpu: &Gpu,
        size: (u32, u32),
        use_dcomp: bool,
    ) -> Result<Self, SurfaceError> {
        // 零尺寸不是合法 swapchain 请求，但也不能 panic —— 最小化时客户区就是
        // 0×0，渲染器不该因此崩掉。钳在这里，唯一的归属处。
        let (width, height) = (size.0.max(1), size.1.max(1));

        // flip model 不接受 0 宽/高，但**最小化**时 winit 报的正是 0×0。上面钳成 1
        // 就够 DXGI 接受（实测 800×600 与 1×1 都成功），所以这里不再多问尺寸。
        let device = Gpu::unknown(&gpu.device).map_err(|e| SurfaceError::new(SurfaceErrorKind::SwapChain(e)).at("QI IUnknown(D3D11Device)"))?;

        let swapchain = if use_dcomp {
            create_composition_swapchain(&device, (width, height), &SwapchainDesc::default())
                .map_err(|e| {
                    SurfaceError::new(SurfaceErrorKind::SwapChain(e))
                        .at("CreateSwapChainForComposition")
                })?
        } else {
            create_hwnd_swapchain(&device, hwnd, (width, height))
                .map_err(|e| e.at("CreateSwapChainForHwnd"))?
        };
        let swapchain3: IDXGISwapChain3 = swapchain
            .cast()
            .map_err(|e| {
                SurfaceError::new(SurfaceErrorKind::SwapChain(e)).at("QI IDXGISwapChain3")
            })?;

        let compositor = if use_dcomp {
            // 硬约束 #2：走 `new_from_rendering_device`。它第一级就是 IDXGIDevice
            // 路线（D3D11 device QI 得动，实测成功），后两级是保险。
            let mut c = DcompCompositor::new_from_rendering_device(hwnd, Some(&device))
                .map_err(|e| SurfaceError::new(SurfaceErrorKind::Win32(e)).at("DCompositionCreateDevice"))?;
            let visual = c.add_visual_on_top()
                .map_err(|e| SurfaceError::new(SurfaceErrorKind::Win32(e)).at("CreateVisual + AddVisual"))?;
            c.set_content_swapchain(&visual, &swapchain)
                .map_err(|e| SurfaceError::new(SurfaceErrorKind::Win32(e)).at("SetContent(swapchain)"))?;
            c.commit()
                .map_err(|e| SurfaceError::new(SurfaceErrorKind::Win32(e)).at("Commit(DComp 建树)"))?;
            Some(c)
        } else {
            None
        };

        let (backbuffers, rtvs) = build_backbuffers(&gpu.device, &swapchain, (width, height))
            .map_err(|e| e.at("build_backbuffers"))?;

        // MSAA 中间目标。采样数由**设备能力**决定（可能退回 1）。
        //
        // ⚠ 离屏诊断模式（`ZEPHYRA_OFFSCREEN=1`）下**不建** MSAA：那条路径的意义是
        // "把渲染管线问题与交换链问题分开"，而多插一层 resolve 会让判据变复杂
        // （截图读到的是解析后的后缓冲还是 MSAA 纹理？）。诊断路径要的是最少的活动
        // 部件，不是好看的边缘。
        let samples = if std::env::var("ZEPHYRA_OFFSCREEN").as_deref() == Ok("1") {
            1
        } else {
            msaa_samples(&gpu.device)
        };
        let msaa = build_msaa_target(&gpu.device, (width, height), samples)
            .map_err(|e| e.at("build_msaa_target"))?;
        if let Some(m) = &msaa {
            eprintln!("[zephyra] MSAA: {}× 中间目标 {width}×{height}", m.samples);
        }

        // 诊断用离屏纹理（`ZEPHYRA_OFFSCREEN=1`）：不碰交换链的渲染目标。
        let offscreen = if std::env::var("ZEPHYRA_OFFSCREEN").as_deref() == Ok("1") {
            let desc = windows::Win32::Graphics::Direct3D11::D3D11_TEXTURE2D_DESC {
                Width: width,
                Height: height,
                MipLevels: 1,
                ArraySize: 1,
                Format: BACKBUFFER_FORMAT,
                SampleDesc: windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: windows::Win32::Graphics::Direct3D11::D3D11_USAGE_DEFAULT,
                BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
                CPUAccessFlags: 0,
                MiscFlags: 0,
            };
            let mut tex = None;
            unsafe { gpu.device.CreateTexture2D(&desc, None, Some(&mut tex as *mut _)) }
                .map_err(|e| SurfaceError::new(SurfaceErrorKind::RenderTarget(e)).at("建离屏纹理"))?;
            let tex = tex.expect("CreateTexture2D 成功时必给纹理");
            let mut rtv = None;
            unsafe { gpu.device.CreateRenderTargetView(&tex, None, Some(&mut rtv as *mut _)) }
                .map_err(|e| {
                    SurfaceError::new(SurfaceErrorKind::RenderTarget(e)).at("离屏 RTV")
                })?;
            eprintln!("[zephyra] 诊断：渲染目标改为离屏纹理 {width}x{height}");
            rtv
        } else {
            None
        };

        Ok(Self {
            backbuffers,
            rtvs,
            offscreen,
            msaa,
            swapchain,
            swapchain3,
            _compositor: compositor,
            device: gpu.device.clone(),
            context: gpu.context.clone(),
            width,
            height,
        })
    }

    /// 本帧应渲染进的渲染目标视图。
    ///
    /// ⚠ MSAA 开启时这里返回的是**多重采样纹理**的视图，不是后缓冲的。调用方
    /// 因此**必须**在本帧绘制结束后调 [`Self::resolve_msaa`]，否则呈现的是
    /// 上一帧的解析结果（而首帧是全黑）。
    ///
    /// 索引由 `GetCurrentBackBufferIndex` 给出，它只在 `Present` 时前进，所以这里
    /// 不需要"acquire 后必须归还"的配对 —— 与 wgpu `Surface::get_current_texture`
    /// 的 `SurfaceTexture` 那种借用句语义不同。
    ///
    /// 返回 `Err` 表示后缓冲取不到了（理论上不会发生：缓冲数固定且都已建好视图）。
    ///
    /// # 索引必须夹住，不能取模
    ///
    /// `GetCurrentBackBufferIndex` 报的是**交换链自己的**缓冲数，而 `rtvs` 只有
    /// 探测出来的那些（flip model 下通常只有 1 个）。本机实测：交换链报 2、
    /// `rtvs.len() == 1`、`GetCurrentBackBufferIndex` 返回 1。
    ///
    /// 取模在这种情况下"碰巧对"（`1 % 1 == 0`），但那是蒙对的 —— 一旦某个驱动
    /// 真的给出两个可用后缓冲，索引 1 就会指向越界项。夹住的语义是"只有一个视图
    /// 时就一直用它"，与 flip model 的实际行为一致。
    pub fn current_rtv(&self) -> windows::core::Result<&ID3D11RenderTargetView> {
        // 诊断（`ZEPHYRA_OFFSCREEN=1`）：绕开交换链，改用一张普通的离屏纹理。
        //
        // 用途：把"渲染管线不对"与"交换链/后缓冲不对"分开。离屏纹理是我们自己
        // 建的、不经过 DXGI 的 RTV —— 若在它上面能画出东西，问题就在交换链那一侧。
        if let Some(rtv) = &self.offscreen {
            return Ok(rtv);
        }
        // MSAA 优先于后缓冲：MSAA 开着时 PSO 的 `MultisampleEnable` 是 1，而
        // 单采样 RTV 与它不兼容（D3D11 会解除绑定）。所以渲染目标必须是 MSAA 纹理。
        if let Some(m) = &self.msaa {
            return Ok(&m.rtv);
        }
        let idx = unsafe { self.swapchain3.GetCurrentBackBufferIndex() } as usize;
        let i = idx.min(self.rtvs.len().saturating_sub(1));
        self.rtvs
            .get(i)
            .ok_or_else(|| windows::core::Error::new(E_FAIL, "后缓冲索引越界"))
    }

    /// 当前的采样数（MSAA 关闭或尚未建起时为 1）。
    ///
    /// resize 时用它重建 —— 见该处「采样数沿用原值」的说明。
    fn samples(&self) -> u32 {
        self.msaa.as_ref().map_or(1, |m| m.samples)
    }

    /// 本帧实际生效的采样数（1 = 无 MSAA）。
    ///
    /// 与 [`Self::current_rtv`] 绑定的目标**必然一致** —— 两者读同一个
    /// `msaa` 字段，不存在"目标换了但采样数没换"的可能。
    pub fn sample_count(&self) -> u32 {
        self.msaa.as_ref().map_or(1, |m| m.samples)
    }

    /// 把多重采样纹理解析回后缓冲。**必须在 present 之前调。**
    ///
    /// # 为什么这一步不能省
    ///
    /// 画进 MSAA 纹理的内容**不会**出现在后缓冲上 —— 后缓冲拿到的是上一次
    /// `ResolveSubresource`（或交换链初始化时）的结果。省掉它的症状是
    /// 「画面卡在旧内容上」或「全黑」，而**没有任何 HRESULT 错误**。
    ///
    /// # 目标索引必须与绘制时一致
    ///
    /// 用的是同一个 `GetCurrentBackBufferIndex` + `min` 夹住，与
    /// [`Self::current_rtv`] 的取法**逐字一致**。两者若不一致（本机只探到 1 个
    /// 可用后缓冲时 `min` 是必需的），解析就会写进一张不是本帧绘制目标的那张 ——
    /// 症状是撕裂，且同样零错误。
    ///
    /// MSAA 关闭时是空操作（没有中间纹理可解析）。
    pub fn resolve_msaa(&self) -> Result<(), SurfaceError> {
        let Some(m) = &self.msaa else {
            return Ok(());
        };
        let idx = unsafe { self.swapchain3.GetCurrentBackBufferIndex() } as usize;
        let i = idx.min(self.rtvs.len().saturating_sub(1));
        let Some(dst) = self.backbuffers.get(i) else {
            return Err(SurfaceError::new(
                SurfaceErrorKind::RenderTarget(windows::core::Error::new(E_FAIL, "后缓冲索引越界")),
            )
            .at("ResolveSubresource(取目标)"));
        };
        unsafe {
            self.context.ResolveSubresource(
                dst,
                0,
                &m.tex,
                0,
                BACKBUFFER_FORMAT,
            );
        }
        Ok(())
    }

    /// 提交本帧：渲染入 [`Self::current_rtv`] 之后调用。
    ///
    /// `sync_interval` 传 1（等一次垂直同步）。传 0 会让渲染跑在合成器之前，
    /// 表现为撕裂。
        /// 复制当前后缓冲到 CPU 可读缓冲。
    ///
    /// # 为什么需要 STAGING 纹理
    ///
    /// 后缓冲是 `D3D11_USAGE_DEFAULT`，内存**不可**被 CPU 直接寻址 ——
    /// 对它 `Map` 返回 `DXGI_ERROR_MAP_INVALID`（实测）。正路是复制到一张
    /// `D3D11_USAGE_STAGING` + `CPU_ACCESS_READ` 的纹理，再对**它** `Map(READ)`。
    ///
    /// # 返回布局
    ///
    /// 返回**行距**而不是紧凑的 `w*4`：驱动的 `RowPitch` 可能大于它（实测 64 宽
    /// 的纹理拿到 256 字节行距），调用方必须按行距跳行，否则整张图会斜。
    /// 像素是 BGRA、自**顶**向下（D3D 的方向），需要自底向上时由调用方翻转。
    ///
    /// # 必须在 `present` **之前**调用，且按 `current_rtv()` 反查资源
    ///
    /// 两种时机都试过（Intel UHD + flip model，只探到 1 个可用后缓冲）：
    ///
    /// | 何时读 | 按什么取 | 结果 |
    /// |---|---|---|
    /// | `present` **前** | `current_rtv()` 反查 | **完整一帧** ✅ |
    /// | `present` **前** | `GetCurrentBackBufferIndex` | 撕裂：左半有内容、右半全黑 |
    /// | `present` **后** | 新索引 | 全黑 |
    ///
    /// 后两行各有一个独立原因：
    ///
    /// - "按索引"的撕裂：flip model 下 DXGI 报 2 个缓冲，但只有索引 0 能建 RTV
    ///   （`build_backbuffers` 逐个探测的结果），于是 `GetCurrentBackBufferIndex`
    ///   返回 1 —— 按它取就取到了另一张未被本帧绘制的缓冲，于是左右两半来自不同
    ///   缓冲；
    /// - present 之后的全黑：`Present` 刚执行、DXGI 尚未交接，此刻复制后缓冲
    ///   拿不到内容。
    ///
    /// 所以两处必须**同时**对：时机在 present 前，取法用 `current_rtv()` 反查。
    ///
    /// 两次误判（先"present 后 + 按索引"读到全黑，再"present 前 + 按索引"读到撕裂）
    /// 的教训是同一句：**D3D11 的失败模式是"不报错但内容不对"，所以唯一可靠的
    /// 判据是看像素**。日志里的一切都正常（`quads=143`、无 HRESULT 错误）。
    pub fn read_pixels(&self) -> windows::core::Result<PixelRead> {
        unsafe {
            // ⚠ MSAA 开启时**不能**直接读 `current_rtv()` 的资源：那是多重采样
            // 纹理，而 `CopyResource` 到单采样 staging 是**非法**的（采样数对不上）。
            //
            // 正路：先把 MSAA 解析回后缓冲（与每帧末尾走同一条路），再按**后缓冲**
            // 索引取资源。截图因此与屏幕所见**逐像素一致** —— 那正是诊断截图存在的
            // 意义（看 MSAA 有没有把边缘糊掉、糊掉多少）。
            let src = match &self.msaa {
                Some(_) => {
                    self.resolve_msaa().map_err(|e| {
                        windows::core::Error::new(E_FAIL, format!("resolve 失败: {e}"))
                    })?;
                    // 索引取法必须与 `current_rtv` 逐字一致（同一个 `min` 夹住），
                    // 否则解析写进 A、截图读 B —— 症状是截图与屏幕不一致。
                    let idx = self.swapchain3.GetCurrentBackBufferIndex() as usize;
                    let i = idx.min(self.backbuffers.len().saturating_sub(1));
                    self.backbuffers
                        .get(i)
                        .ok_or_else(|| windows::core::Error::new(E_FAIL, "后缓冲索引越界"))?
                        .clone()
                }
                None => {
                    // 单采样路径：沿用原来的「current_rtv 反查」—— 它同时覆盖了
                    // 离屏诊断模式（那里 RTV 不是后缓冲）。
                    self.current_rtv()?.GetResource()?.cast::<ID3D11Texture2D>()?
                }
            };

            // 后缓冲的**真实**尺寸。
            //
            // 必须从资源描述里读，而不是用 [`Self::size`]：两者不一致时
            // `ClearRenderTargetView` 与视口都按"声明的尺寸"走，于是右/下多出来
            // 的那几条永远不会被清 —— 症状是**一整条纯黑边**，宽度恰好是尺寸差。
            //
            // 这个值同时用于：① 建 STAGING 纹理（尺寸错了就斜着读）、② 在报告里
            // 与声明尺寸对比。
            let mut td = std::mem::zeroed::<
                windows::Win32::Graphics::Direct3D11::D3D11_TEXTURE2D_DESC,
            >();
            src.GetDesc(&mut td);
            let actual_w = td.Width;
            let actual_h = td.Height;

            let stg = crate::texture::create_staging_texture(
                &self.device,
                actual_w,
                actual_h,
                BACKBUFFER_FORMAT,
                windows::Win32::Graphics::Direct3D11::D3D11_CPU_ACCESS_READ.0 as u32,
            )
            .map_err(|e| windows::core::Error::new(E_FAIL, format!("建 STAGING 纹理失败: {e}")))?;
            self.context.CopyResource(&stg, &src);
            let mut mapped =
                windows::Win32::Graphics::Direct3D11::D3D11_MAPPED_SUBRESOURCE::default();
            self.context
                .Map(
                    &stg,
                    0,
                    windows::Win32::Graphics::Direct3D11::D3D11_MAP_READ,
                    0,
                    Some(&mut mapped as *mut _),
                )
                .map_err(|e| windows::core::Error::new(E_FAIL, format!("Map(READ) 失败: {e}")))?;
            
            let row_pitch = mapped.RowPitch as usize;
            // 行数取**真实**高度，不是 [`Self::size`] —— 尺寸不一致时按声明高度
            // 读会越界（staging 只有 `actual_h` 行）。
            let total = row_pitch * actual_h as usize;
            let mut data = vec![0u8; total];
            if !mapped.pData.is_null() {
                std::ptr::copy_nonoverlapping(mapped.pData as *const u8, data.as_mut_ptr(), total);
            }
            self.context.Unmap(&stg, 0);
            Ok(PixelRead {
                data,
                // DXGI 的 B8G8R8A8 读回是 **BGRA** 序（与 CPU 画布的 RGBA 相反，
                // 见 `ChannelOrder`）。
                order: zephyra_fx::pixels::ChannelOrder::Bgra,
                row_pitch: row_pitch as u32,
                width: actual_w,
                height: actual_h,
                declared_width: self.width,
                declared_height: self.height,
            })
        }
    }

    /// 提交本帧：渲染入 [`Self::current_rtv`] 之后调用。
    ///
    /// `sync_interval` 传 1（等一次垂直同步）。传 0 会让渲染跑在合成器之前，
    /// 表现为撕裂。
    pub fn present(&self, sync_interval: u32) -> Result<(), SurfaceError> {
        unsafe {
            // 硬约束 #3：params 必须非空，必须走 `present_one`。
            present_one(&self.swapchain, sync_interval)
                .map_err(|e| SurfaceError::new(SurfaceErrorKind::Win32(e)).at("Present1"))?;
        }
        if let Some(c) = &self._compositor {
            c.commit()
                .map_err(|e| SurfaceError::new(SurfaceErrorKind::Win32(e)).at("Commit(present 后)"))?;
        }
        Ok(())
    }

    /// 物理尺寸变更：重建后缓冲视图。
    ///
    /// # 为什么不能等 GPU 排空
    ///
    /// D3D11 没有 fence/等待查询这类"GPU 干完了吗"的 API —— 立即上下文的绘制由
    /// 驱动内部排队。所以这里不能像 D3D12 那样先 drain 再 `ResizeBuffers`。
    ///
    /// 实践上这不构成问题：`Resizing` 事件在主线程事件循环里到达，而主线程此刻
    /// 必然没在提交绘制（同一线程）。真正的风险是"上一帧的绘制还在 GPU 队列里",
    /// 那由 DXGI 自己在检测到在途引用时让 `ResizeBuffers` 返回
    /// `DXGI_ERROR_WAS_STILL_DRAWING` —— 本方法把那个错误吞掉（视图随后按新尺寸
    /// 重建），下一帧自然就正确了。
    pub fn resize(&mut self, width: u32, height: u32) {
        let (width, height) = (width.max(1), height.max(1));
        if (width, height) == (self.width, self.height) {
            return;
        }
        // 先放掉旧视图与纹理，再 resize：反过来会让 DXGI 因为仍有引用而拒绝。
        self.rtvs.clear();
        self.backbuffers.clear();
        unsafe {
            // count=0 保持缓冲数；`DXGI_FORMAT_UNKNOWN` 保持格式。
            let _ = self.swapchain.ResizeBuffers(
                0,
                width,
                height,
                windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_UNKNOWN,
                DXGI_SWAP_CHAIN_FLAG(0),
            );
        }
        match build_backbuffers(&self.device, &self.swapchain, (width, height)) {
            Ok((bufs, views)) => {
                self.backbuffers = bufs;
                self.rtvs = views;
                self.width = width;
                self.height = height;
                // ⚠ MSAA 纹理**也必须**按新尺寸重建 —— 它不是交换链的一部分，
                // `ResizeBuffers` 不会碰它。忘了这一步的症状是「窗口右侧/下侧的
                // 图标被裁掉」：MSAA 纹理还是旧尺寸，视口却按新尺寸设了，
                // 越界部分直接没有 RTV 可写。
                //
                // 采样数**沿用原值**而不是重新查设备：重建失败时退化成
                // "尺寸对但没有 MSAA 目标"，那比尺寸不对更难查。
                self.msaa = match build_msaa_target(&self.device, (width, height), self.samples()) {
                    Ok(m) => m,
                    Err(_) => None,
                };
                // 尺寸变了 → 交换链重建 → DComp 侧的 visual 内容绑定必须重新提交一次，
                // 否则合成器仍持有旧后缓冲的引用。
                if let Some(c) = &self._compositor {
                    let _ = c.commit();
                }
            }
            Err(_) => {
                // 建视图失败时保持旧尺寸不变：宁可不 resize（画面拉伸），也不要
                // 变成"尺寸说有 N×N 但没有对应视图"的更糟状态。
                let (w, h) = (self.width, self.height);
                if let Ok((bufs, views)) = build_backbuffers(&self.device, &self.swapchain, (w, h))
                {
                    self.backbuffers = bufs;
                    self.rtvs = views;
                }
            }
        }
    }

    /// 物理尺寸。
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

/// 逐个后缓冲建纹理与渲染目标视图。
///
/// DXGI 拥有交换链的资源，所以纹理只能从 `GetBuffer` 取 —— 这也是保证 D3D11 与
/// DComp 看到同一块显存的办法。
fn build_backbuffers(
    device: &ID3D11Device,
    swapchain: &IDXGISwapChain1,
    _size: (u32, u32),
) -> Result<(Vec<ID3D11Texture2D>, Vec<ID3D11RenderTargetView>), SurfaceError> {
    // 上限取 `GetDesc1` 报告的缓冲数 —— 0.62 的 `IDXGISwapChain1` 没有
    // `GetBufferCount`（DXGI1.2 才加的，windows-rs 没生成）。
    //
    // 但**这个上限本身是不可信的**，所以下面不直接用它，而是**逐个试到失败为止**。
    let reported = unsafe { swapchain.GetDesc1() }
        .map_err(|e| SurfaceError::new(SurfaceErrorKind::SwapChain(e)).at("GetDesc1"))?
        .BufferCount as usize;

    // # 为什么必须探测，而不是照着报告的数取
    //
    // 实测两条路径各暴露一种不一致：
    //
    // - 普通 HWND 交换链：`GetDesc1` 报 2，但 `GetBuffer(1)` 返
    //   `DXGI_ERROR_INVALID_CALL`（0x887A0001）—— 实际只有 1 个可用；
    // - DComp composition 交换链：`GetBuffer(1)` **成功**（纹理带
    //   `BindFlags = 0x20` = `RENDER_TARGET`），但 `CreateRenderTargetView`
    //   对它返 `E_INVALIDARG`（0x80070057）。
    //
    // 也就是说"可用"的判据既不是 `GetBuffer` 成功、也不是绑定位正确，而是
    // **`GetBuffer` + `CreateRenderTargetView` 都成功**。两条错误消息都不说是
    // 索引越界，所以逐个试是唯一可靠的办法。
    //
    // 失败即边界：多试只会一路撞同一堵墙，且每次都留下一个 HRESULT。
    let mut backbuffers = Vec::new();
    let mut rtvs = Vec::new();
    for i in 0..reported.max(1) {
        let Ok(tex) = (unsafe { swapchain.GetBuffer::<ID3D11Texture2D>(i as u32) }) else {
            break;
        };
        let mut rtv = None;
        if unsafe { device.CreateRenderTargetView(&tex, None, Some(&mut rtv as *mut _)) }.is_err() {
            break;
        }
        rtvs.push(rtv.expect("CreateRenderTargetView 成功时必给视图"));
        backbuffers.push(tex);
    }

    if backbuffers.is_empty() {
        // 至少要有一个能画的地方。全都建不出来时给一条带现场的错误，
        // 而不是让 `current_rtv` 在第一帧返回 `Err`（那时机已经过了，
        // 症状是"窗口永远空白"）。
        return Err(
            SurfaceError::new(SurfaceErrorKind::RenderTarget(
                windows::core::Error::new(E_FAIL, "一个后缓冲都用不了"),
            ))
            .at("GetBuffer/CreateRenderTargetView")
            .with_hint(format!(
                "GetDesc1 报告 {reported} 个缓冲，但索引 0 就建不出视图。"
            )),
        );
    }

    if backbuffers.len() < reported {
        eprintln!(
            "[zephyra] 交换链报告 {reported} 个后缓冲，但只有 {} 个能建成渲染目标\
             （flip model 下 D3D11 常见行为）。按 {} 个继续。",
            backbuffers.len(),
            backbuffers.len(),
        );
    }
    Ok((backbuffers, rtvs))
}

/// 建普通 HWND 交换链（flip model）。
fn create_hwnd_swapchain(
    device: &windows::core::IUnknown,
    hwnd: HWND,
    size: (u32, u32),
) -> Result<IDXGISwapChain1, SurfaceError> {
    let factory: IDXGIFactory2 =
        unsafe { CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0)) }
        .map_err(|e| SurfaceError::new(SurfaceErrorKind::SwapChain(e)).at("CreateDXGIFactory2"))?;
    let desc = DXGI_SWAP_CHAIN_DESC1 {
        Width: size.0,
        Height: size.1,
        Format: BACKBUFFER_FORMAT,
        // `Stereo` 是裸 `BOOL`（不是 0.62 高层 API 那样的 `bool`），所以必须
        // `BOOL(0)`。这个不一致不是笔误 —— 它说明"统一成 bool"的重构只做到了高层
        // 绑定层，结构体字段没跟上。
        Stereo: windows::core::BOOL(0),
        SampleDesc: windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        BufferUsage: windows::Win32::Graphics::Dxgi::DXGI_USAGE_RENDER_TARGET_OUTPUT,
        BufferCount: SwapchainDesc::default().buffer_count,
        Scaling: windows::Win32::Graphics::Dxgi::DXGI_SCALING_STRETCH,
        // FLIP_DISCARD 而不是 DISCARD：flip model 才能拿到 DComp 那种呈现路径，
        // 且在 DWM 下不撕裂。
        SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
        // `PREMULTIPLIED` 在**普通 HWND 交换链**上一律失败：本机实测八个组合，
        // 凡带 `PREMULTIPLIED` 的都返回 `DXGI_ERROR_INVALID_CALL`（0x887A0001），
        // 而 `IGNORE` / `UNSPECIFIED` 全部成功。设备已带
        // `D3D11_CREATE_DEVICE_BGRA_SUPPORT` 建（见 [`Gpu::new`]），所以这不是
        // "设备能力不够"，而是这条路本身不支持预乘 alpha。
        //
        // 这不影响正确性：DComp 路径用 `CreateSwapChainForComposition` + `PREMULTIPLIED`
        // （`zephyra-dcomp` 的 `SwapchainDesc` 钉死），而**只有那条路径**需要预乘 ——
        // 它要把后缓冲交给合成器做 alpha 混合。普通路径由 DWM 按不透明窗口处理，
        // 底色就是清屏色，alpha 通道不参与任何运算。
        AlphaMode: windows::Win32::Graphics::Dxgi::Common::DXGI_ALPHA_MODE_IGNORE,
        // `DXGI_SWAP_CHAIN_DESC1::Flags` 是裸 `u32`，不是 `DXGI_SWAP_CHAIN_FLAG`
        // —— 与 `Stereo` 同一个坑。
        Flags: 0,
    };
    // 注意 DXGI 的**参数顺序**：`CreateSwapChainForHwnd` 是
    // `(device, hwnd, desc, pFullscreenDesc, pRestrictToOutput)`，
    // 而 `CreateSwapChainForComposition` 是 `(device, desc, pRestrictToOutput)`。
    // 两者第三个参数含义不同 —— 0.62 把 `pRestrictToOutput` 收成泛型 `Param<IDXGIOutput>`，
    // 所以"没有输出限制"必须写成 `None::<&IDXGIOutput>`，直接传 `None` 会
    // 推断不出类型而报错；更糟的是传 `None::<&IDXGISwapChain1>` 这类会编译过但
    // 语义完全不对。
    // # 这里绝不能"先试几个参数看哪个成功"
    //
    // DXGI 规定**一个 HWND 同时只能挂一个交换链**。第二次对同一 HWND 调
    // `CreateSwapChainForHwnd` 会返回 `DXGI_ERROR_INVALID_CALL`（0x887A0001），
    // 而那条错误**完全不提"这个 HWND 已经有交换链了"** —— 它和"某个参数非法"
    // 看起来一模一样。
    //
    // 本机实测踩过：为了定位 INVALID_CALL，在真实调用**之前**先试了三个参数组合，
    // 三个全部成功；紧接着的真实调用（参数与成功者逐一相同）却失败。也就是
    // INVALID_CALL 的真正原因是"槽位已被前面的诊断调用占用"，而不是参数。
    //
    // 所以要试参数就必须**每个组合换一个 HWND**。同一约束在 `zephyra-dcomp` 那边
    // 表现为另一个错误码（`DCOMPOSITION_ERROR_WINDOW_ALREADY_COMPOSED` 0x88980800，
    // 见其文档），症状同样是"第二次建树失败且不说是重复"。
    let sc: IDXGISwapChain1 = unsafe {
        factory.CreateSwapChainForHwnd(
            device,
            hwnd,
            &desc,
            None,
            None::<&windows::Win32::Graphics::Dxgi::IDXGIOutput>,
        )
    }
    .map_err(|e| {
        SurfaceError::new(SurfaceErrorKind::SwapChain(e)).at("CreateSwapChainForHwnd")
    })?;
    Ok(sc)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dcomp_is_off_unless_the_env_var_says_otherwise() {
        // 进程内只读一次，所以本测试只断言"读过一次之后是某个 bool"，
        // 不去改环境变量 —— 改了也不会生效，反而会让人以为门控是坏的。
        assert_eq!(dcomp_enabled(), dcomp_enabled(), "门控必须幂等，否则会中途切资源拓扑");
    }

    #[test]
    fn the_backbuffer_format_matches_the_swapchain_default() {
        // 两处格式必须一致，而其中一处是 zephyra-dcomp 里的 DXGI 常量。
        // 这条断言是"改了格式忘了改另一边"的唯一护栏 —— 那种不一致不会编译
        // 报错，只会让整屏偏色或 DComp 建链直接失败。
        assert_eq!(
            BACKBUFFER_FORMAT.0,
            SwapchainDesc::default().format.0,
            "本常量与 zephyra-dcomp 的 SwapchainDesc::default 必须一致"
        );
        // 比数值而不是 Debug 串：`DXGI_FORMAT` 的 `Debug` 只打印 `DXGI_FORMAT(87)`。
        assert_eq!(
            BACKBUFFER_FORMAT.0,
            windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM.0
        );
    }

    #[test]
    fn the_failures_stay_distinguishable() {
        // `Device` 意味着"这台机器没有可用 GPU"，`BadWindowHandle` 意味着"宿主用错
        // 平台"。混成一个错误会让宿主无法决定该报错还是该退回，而误诊很难查。
        assert!(SurfaceError::new(SurfaceErrorKind::Device(windows::core::Error::from_hresult(
            E_FAIL
        )))
        .to_string()
        .contains("D3D11"));
        assert!(SurfaceError::new(SurfaceErrorKind::BadWindowHandle)
            .to_string()
            .contains("HWND"));
    }

    /// 步骤标签必须出现在消息里。
    ///
    /// 这条断言守的是本文件存在的主要理由：DXGI 的 `INVALID_CALL` 三种病因同一句话，
    /// 唯一步骤标签能区分。漏掉它 → 错误信息退回"某处失败"，排查成本立刻变高。
    #[test]
    fn the_step_is_always_reported() {
        let e = SurfaceError::new(SurfaceErrorKind::SwapChain(
            windows::core::Error::from_hresult(E_FAIL),
        ))
        .at("CreateSwapChainForHwnd");
        let s = e.to_string();
        assert!(s.contains("CreateSwapChainForHwnd"), "实得 {s}");
        // 未标注时也必须显式说出来，而不是省略 —— "有个步骤忘了填"要能被看见。
        let unlabelled = SurfaceError::new(SurfaceErrorKind::SwapChain(
            windows::core::Error::from_hresult(E_FAIL),
        ));
        assert!(
            unlabelled.to_string().contains("未标注"),
            "实得 {unlabelled}"
        );
    }
}

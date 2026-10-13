//! GPU **后端无关**的那一层：顶点展开、字形图集、纹理表、CPU 参考光栅器。
//!
//! # 为什么要有这个 crate
//!
//! 之前这些代码住在 `zephyra-winit` 里，而那个 crate 实际上是 **D3D11 后端**。
//! 于是「DX11 和 DX12 分 crate、不互相依赖」这个决定带来两个坏结果：
//!
//! 1. `quad.rs` 被**逐字节复制**成两份（726 行）。任何一处改动都必须记得改两遍，
//!    否则两个后端的顶点布局会静默漂移 —— 而漂移的症状是「画面整体错位」，
//!    不报任何错。
//! 2. `cpu_raster` / `text` / `texture` **只存在于 DX11 那边**，于是 DX12 后端
//!    既没有字体、也没有纹理、也没法做 CPU 侧诊断。
//!
//! 两者的根因是同一个：**这些代码本来就没有后端依赖**，只是被放错了位置。
//!
//! # 层次
//!
//! ```text
//!   zephyra            纯逻辑，零依赖（DisplayList / 布局 / diff / 输入）
//!        │
//!        ▼
//!   zephyra-fx   ← 本 crate：把 DisplayList 变成顶点流/字形图集，与谁渲染无关
//!        │
//!        ├──► base/zephyra-winit      D3D11 后端
//!        ├──► base/zephyra-winit-d3d12  D3D12 后端
//!        └──► base/zephyra-opengl     （将来的）OpenGL 后端
//! ```
//!
//! 三个后端**互不依赖**，只依赖本 crate。这正是 Skia 的做法：抽象层放在
//! Ganesh 层，各后端分别实现它的接口，而不是让后端之间互相引用、或者把
//! 后端无关的代码复制三份。
//!
//! # 为什么它在 `zephyra/` 下与 `zephyra-base/` 并列，而不是 `zephyra-base/` 的子目录
//!
//! 这一层**一个系统 API 都不碰**：没有 `ID3D11Device`、没有 `ID3D12Device`、
//! 没有 `IDXGIFactory`、没有窗口句柄，`unsafe_code = "forbid"`。凡是碰驱动的
//! 东西都在某个后端 crate 里。
//!
//! 所以物理位置跟着职责走 —— 它与 Skia 把 `src/effects` 放在与 `src/core`
//! **并列**处是同一个判断：效果（及其顶点展开、光栅化）的**描述**是纯数据，
//! 能在无 GPU 的机器上单测；而只有效果的**执行**才需要系统 API。
//!
//! # 本 crate 里没有的东西
//!
//! **任何 D3D 类型。** 没有 `ID3D11Device`、没有 `ID3D12Device`、没有
//! `IDXGIFactory`。凡是碰驱动的东西都该在某个后端 crate 里。

#![cfg_attr(windows, allow(dead_code))]

pub mod cpu_only;
pub mod geom_clip;
pub mod cpu_raster;
pub mod offscreen;
pub mod pixels;
pub mod quad;
pub mod text;
pub mod texture;

pub use pixels::{srgb_bytes, write_bmp, PixelRead};
pub use quad::RenderReport;
pub use text::{from_font_file, from_system_font, AbGlyphSource, FONT_OVERRIDE_ENV};
pub use texture::TextureNames;

/// 标量属性的本地定义（宿主自定义控件用）。
///
/// 刻意放这里而不是加进核心的 `props.rs`：z-index 是**渲染层**的词汇，而核心的
/// 属性表不该知道后端怎么用它。真要提升为标准键时再迁移。
///
/// 它原先只在 `zephyra-winit`（D3D11）里，而 D3D12 那个 crate 是**另一个独立 crate**
/// —— 于是同一个属性键有两份定义。属性键靠**名字**相等（`PropertyKey::new("ZIndex", …)`
/// 内部的 id 由名字派生），所以两份定义**恰好**能互换；一旦有人只改一处，另一处就静默
/// 变成另一个键，而症状是「z-index 不生效」且没有任何编译或运行错误。
///
/// 放这里就没有这份风险：两个后端共用一个定义。
pub static Z_INDEX: zephyra_core::property::PropertyKey =
    zephyra_core::property::PropertyKey::new("ZIndex", zephyra_core::property::PropertyValue::int(0));

/// 应用的文本引擎 —— 本 crate 唯一支持的字体来源。
///
/// 之所以是类型别名而不是 `impl Trait`：它出现在各后端 `HostWindow` 与
/// [`cpu_only::CpuHost`] 的字段类型里，必须能被外部指名。
pub type TextEngine = zephyra_core::text::TextStack<AbGlyphSource>;

/// 重新导出布局函数。
///
/// `cpu_only` 与各后端的帧循环走的是**同一段**布局代码路径。重复实现一遍的话，
/// 两边迟早会漂移 —— 而漂移之后「CPU 参考图」就不再是「实际画面」的参考了。
pub use zephyra_core::layout::layout as layout_tree;

use zephyra_core::context::Context;
use zephyra_core::diff::DiffStats;
use zephyra_core::element::Element;
use zephyra_core::frame::Damage;
use zephyra_core::id::ElementKey;

/// 当前输入状态 —— 由 [`zephyra_core::input::InputRouter`] 产出。
///
/// 捕获、穿透、按下抬起配对都已经在核心里处理完了，宿主与应用都不必自己写
/// 事件分发。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct InputState {
    /// 当前悬停的可交互节点。
    pub hovered: Option<ElementKey>,
    /// 按下时被捕获的节点（指针移出该节点后仍归它所有，直到抬起）。
    pub pressed: Option<ElementKey>,
    /// 当前活跃指针数。
    pub pointers: usize,
    /// 客户区视口，**物理像素**，与宿主的 `inner_size()` 同源。
    /// scale 是当时的缩放比（`window.scale_factor()`）。
    ///
    /// # 为什么两个都要给
    ///
    /// 视图做**两种**不同单位的计算，缺一个就只能猜：
    ///
    /// - **断点**按有效像素 —— 官方 `AdaptiveTrigger.MinWindowWidth` 的定义是
    ///   "the minimum window width (**in effective pixels**)"，所以要 `viewport.0 / scale`；
    /// - **列数**按布局宽 —— 它除的是卡片宽度，而卡片宽度是布局单位（本仓当前
    ///   1 单位 = 1 物理像素），所以要 `viewport.0`。
    ///
    /// 只给其中一个，另一个就得靠约定推，而"忘了除 scale"是这个口径唯一的失败方式，
    /// 它的症状是「该折叠导航时没折叠」—— 不是报错，是布局明显不合身。
    ///
    /// 放在 [`Self`] 上而不是新增一个 trait 方法：`on_input` 已经是「宿主每帧开头
    /// 把状态推给视图」的既有通道，再加一个方法只是多一条要记得调的路径。
    pub viewport: (f32, f32),
    /// 视口的缩放比。见 [`Self::viewport`]。
    pub scale: f32,
    /// 距上一帧的**毫秒**数（宿主用 `Instant` 实测，不是按刷新率拍的）。
    ///
    /// # 为什么动画时钟走这里
    ///
    /// 动画 = 缓动后的进度，而进度必须由**真实经过的时间**推进。
    /// 若按"每帧推进一个固定量"，那么掉一帧动画就变慢、快机器上就变快 ——
    /// 而官方规范的时长全是**毫秒**（83 / 167 / 250 / 333），只在真实时间下成立。
    ///
    /// 首帧没有"上一帧"，宿主填 0。填 `None` 也可以，但那会让动画在第一帧
    /// 与余下帧之间多一条分支，而本条已经能表达"没经过时间"。
    pub dt_ms: f32,
    /// 系统的语言偏好（有序，最偏好的在前）—— 对应浏览器的 `navigator.languages`。
    ///
    /// 宿主经 [`zephyra_platform::Platform::system_languages`] 读，读不到给空表。
    /// **空表不是"英语"**，是"读不到"：上层据此回落默认语言，
    /// 而不是把界面钉死在某一门上。
    ///
    /// 放在这里而不是另开一个 trait 方法：它与视口 / 缩放比同属
    /// 「宿主每帧开头把外部事实推给视图」那一批，多一条路径只会更容易漏。
    pub languages: Vec<String>,
}

/// 一帧里应用要做的事：宿主把「当前状态」变成一棵 `Element`。
///
/// # 两段式：先同步状态，再造树
///
/// [`View::on_input`] 与 [`View::view`] 分开是有意的：命中测试需要**已布局**的
/// 矩形，所以 hover 只能在布局之后才知道 —— 它必然晚于上一帧的 `view()`。因此
/// 宿主在每帧开头先把新状态推给视图，视图再据此造下一帧的树。
///
/// 这就是保留式 UI 的节奏：输入改变状态，状态改变声明，声明经 diff 落到保留树。
pub trait View {
    /// 宿主在每帧开头把输入状态推过来。
    fn on_input(&mut self, state: &InputState);

    /// 造出这一帧的声明树。
    ///
    /// `ctx` 提供当前主题 —— 核心不持有主题，主题是应用状态。
    fn view(&self, ctx: &Context<'_>) -> Element;
}

/// 一帧的输出：diff 统计 + 渲染统计。
///
/// **不碰 GPU。** 它把「布局 + 排版 + `DisplayList` → 顶点流」这一整段单独跑完，
/// 所以在没有窗口、没有设备、没有交换链的环境里也能得到。
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    /// 声明 → 保留的对账结果。
    pub diff: DiffStats,
    /// DisplayList → 顶点流 的回放结果。
    pub render: RenderReport,
    /// 命中的最上层可交互节点（`None` = 点在空白处）。
    pub hovered: Option<String>,
    /// 本帧的脏区，供宿主做**帧内** scissor。
    ///
    /// ⚠ 不能用它跨帧跳过回放 —— flip model 下 `Present` 之后后缓冲内容未定义，
    /// 任何要 present 的帧都必须重画整个目标。详见
    /// [`zephyra_core::frame`](zephyra_core::frame) 模块文档。
    pub damage: Damage,
    /// 本帧是否真的画了并上屏了。
    ///
    /// `false` = core 判定 `DisplayList` 与上次逐字节相同，宿主整帧跳过
    /// （含 present）。所以 `render` 各项此时全是 0 —— 那不是「什么都没画」，
    /// 而是「这一帧不需要画」。
    ///
    /// 要判断「有没有东西可见」不能看 `render`，得看这个字段。
    pub redrawn: bool,
}

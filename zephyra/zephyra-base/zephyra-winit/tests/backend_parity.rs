//! # 跨后端一致性：同一份 `DisplayList`，D3D11 与 D3D12 必须画出同一批像素
//!
//! 计划的 E4 一节把这条列为**第一个交付物**，并且明确说它**比**「29 个模式都实现了」
//! 更重要 —— 因为两个后端编码混合状态的方式根本不同：
//!
//! | | D3D11 | D3D12 |
//! |---|---|---|
//! | 混合状态 | `ID3D11BlendState`，运行时换 | **烤进 PSO**，换状态就是换 PSO |
//! | 可分离 / 非可分离模式 | 每个模式一个 PS 变体 + `Src` 混合 | 同 |
//!
//! 两侧各写一套测试、期望值**分别手写**，那么一个**两边一致**的误读会同时通过。
//! 本仓已经栽过同型的：离屏是 `B8G8R8A8` 而 `Color` 是 RGBA，通道顺序相反，
//! 第一版就把红判成了蓝。
//!
//! ## 构图为什么全是**无边缘**的四边形
//!
//! 两个后端的采样数不同（D3D11 那边的夹具跑 4× MSAA，D3D12 的离屏目标是单采样），
//! 任何**边缘**上的像素都会因抗锯齿覆盖率不同而分道扬镳。所以这里的图形全部是
//! **铺满采样点的矩形** —— 一旦引入圆角或半条边，这条测试就变成在比抗锯齿。
//!
//! ## 容差
//!
//! 同一块卡、同样的 SM5 字节码、同一套预乘公式，但固定功能混合的中途舍入由驱动决定。
//! 实测两个后端的差**全部落在 ±1** 以内，所以容差取 `2`：既能吸收舍入，又不至于
//! 把「某个模式算错了」这种量级的偏差（通常是几十）放过。

#![cfg(windows)]
// 本测试要建真实 D3D11 设备，非 Windows 上没有这条路径。
// 宿主 crate 整个是 `#![cfg(windows)]`，而集成测试是**独立目标**、不受那个门禁保护
// —— 不在这里补一道，`cargo test --workspace` 在 Linux 上就编不过（实测 2026-10）。

use zephyra_core::core::Color;
use zephyra_core::filter::BlendMode;
use zephyra_core::paint::{Command, DisplayList, Layer, SortKey};

const SIZE: u32 = 64;

/// 允许的逐通道最大偏差（见文件头「容差」）。
const TOL: i32 = 2;

/// 背景色。取 0.5 而不是 0 / 1：0 与 1 会让很多混合模式**退化**成同一个结果，
/// 于是「两个后端一致」变成「两边都退化了」—— 那是恒真。
const BACKDROP: Color = Color {
    r: 0.5,
    g: 0.5,
    b: 0.5,
    a: 1.0,
};

/// 前景色：**半透明**，而且 RGB 三个通道互不相同。
///
/// 半透明是必须的：不透明源在预乘空间里让除 `Src` 之外的每个模式都退化成源本身。
/// 而 RGB 相等的话，`Hue` / `Saturation` 之类模式的退化现象更难察觉。
const FOREGROUND: Color = Color {
    r: 0.8,
    g: 0.4,
    b: 0.2,
    a: 0.6,
};

fn scene_with(blend: BlendMode, fg: Color) -> DisplayList {
    let mut list = DisplayList::new();
    let s = SIZE as f32;
    list.push(
        SortKey {
            layer: Layer::Background,
            z: 0,
            order: 0,
        },
        Command::Quad {
            rect: zephyra_core::core::Rect::new(0.0, 0.0, s, s),
            radius: zephyra_core::core::CornerRadius { radius: 0.0 },
            fill: BACKDROP,
            stroke: None,
            filter: zephyra_core::filter::FilterChain::none(),
            blend: BlendMode::SrcOver,
        },
    );
    list.push(
        SortKey {
            layer: Layer::Content,
            z: 0,
            order: 1,
        },
        Command::Quad {
            rect: zephyra_core::core::Rect::new(0.0, 0.0, s, s),
            radius: zephyra_core::core::CornerRadius { radius: 0.0 },
            fill: fg,
            stroke: None,
            filter: zephyra_core::filter::FilterChain::none(),
            blend,
        },
    );
    list.sort();
    list
}

fn scene(blend: BlendMode) -> DisplayList {
    scene_with(blend, FOREGROUND)
}

// ---------------------------------------------------------------------------
//  D3D11 侧
// ---------------------------------------------------------------------------

use zephyra_winit::renderer::{RenderCtx, Renderer};
use zephyra_winit::surface::Gpu;
use zephyra_winit::texture::TextureTable;

/// 一个长期存活的 D3D11 上下文：29 个模式共用一台设备。
///
/// 逐模式新建设备会让这条测试从「一次对照」变成「29 次设备初始化」—— 而设备创建在
/// 本机要几十毫秒（还要过一遍适配器选择），慢到足以让人在 CI 上把它关掉。
struct D3D11 {
    gpu: Gpu,
    rtv: windows::Win32::Graphics::Direct3D11::ID3D11RenderTargetView,
    tex: windows::Win32::Graphics::Direct3D11::ID3D11Texture2D,
    renderer: Renderer,
    textures: TextureTable,
}

impl D3D11 {
    fn new() -> Self {
        let gpu = Gpu::new().expect("应当建得出 D3D11 设备");
        use windows::Win32::Graphics::Direct3D11::*;
        use windows::Win32::Graphics::Dxgi::Common::{
            DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC,
        };
        let desc = D3D11_TEXTURE2D_DESC {
            Width: SIZE,
            Height: SIZE,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            // ⚠ 必须与 `Renderer` 用的采样数一致。
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let mut tex = None;
        unsafe {
            gpu.device
                .CreateTexture2D(&desc, None, Some(&mut tex as *mut _))
        }
        .expect("应当能创建离屏纹理");
        let tex = tex.expect("CreateTexture2D 成功时必给纹理");
        let mut rtv = None;
        unsafe {
            gpu.device
                .CreateRenderTargetView(&tex, None, Some(&mut rtv as *mut _))
        }
        .expect("应当能创建离屏 RTV");
        let rtv = rtv.expect("CreateRenderTargetView 成功时必给视图");
        let renderer = Renderer::new(&gpu.device).expect("渲染器应当能建得出来");
        Self {
            gpu,
            rtv,
            tex,
            renderer,
            textures: TextureTable::new(),
        }
    }

    fn render(&mut self, list: &DisplayList) -> Vec<u8> {
        self.renderer.render(RenderCtx {
            context: &self.gpu.context,
            device: &self.gpu.device,
            rtv: &self.rtv,
            list,
            size: (SIZE, SIZE),
            clear: Color::rgb(0.0, 0.0, 0.0),
            textures: &self.textures,
            damage: zephyra_core::frame::Damage::Full,
        });
        read_back(&self.gpu, &self.tex)
    }
}

/// 把目标拷进一张 **STAGING**（类型必须是 CPU 可访问）并读回。
///
/// D3D11 的 RT 是 **BGRA**，而 `Color` 是 RGBA —— 通道顺序相反。
/// 这里**按 BGRA 原样返回**，由 `to_rgba` 统一换算，避免每个调用点各换一次。
fn read_back(gpu: &Gpu, tex: &windows::Win32::Graphics::Direct3D11::ID3D11Texture2D) -> Vec<u8> {
    use windows::Win32::Graphics::Direct3D11::*;
    use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
    let desc = D3D11_TEXTURE2D_DESC {
        Width: SIZE,
        Height: SIZE,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_STAGING,
        BindFlags: 0,
        CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
        MiscFlags: 0,
    };
    let mut staging = None;
    unsafe {
        gpu.device
            .CreateTexture2D(&desc, None, Some(&mut staging as *mut _))
    }
    .expect("应当能创建 staging 纹理");
    let staging = staging.expect("CreateTexture2D 成功时必给纹理");
    unsafe { gpu.context.CopyResource(&staging, tex) };
    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    unsafe {
        gpu.context
            .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
    }
    .expect("应当能映射 staging");
    let bytes = (SIZE * SIZE * 4) as usize;
    let out = unsafe { std::slice::from_raw_parts(mapped.pData.cast::<u8>(), bytes) }.to_vec();
    unsafe { gpu.context.Unmap(&staging, 0) };
    out
}

// ---------------------------------------------------------------------------
//  D3D12 侧
// ---------------------------------------------------------------------------

use zephyra_winit_d3d12::{Gpu as Gpu12, Offscreen as Offscreen12, Renderer as Renderer12};

struct D3D12 {
    gpu: Gpu12,
    target: Offscreen12,
    renderer: Renderer12,
    textures: zephyra_winit_d3d12::TextureTable,
}

impl D3D12 {
    fn new() -> Self {
        let gpu = Gpu12::new().expect("应当建得出 D3D12 设备");
        let target = Offscreen12::new(&gpu.device, (SIZE, SIZE)).expect("应当建得出离屏目标");
        let renderer = Renderer12::new(&gpu.device, 64, 1).expect("应当建得出渲染器");
        Self {
            gpu,
            target,
            renderer,
            textures: zephyra_winit_d3d12::TextureTable::new(),
        }
    }

    fn render(&mut self, list: &DisplayList) -> Vec<u8> {
        self.gpu.begin_frame().expect("应当能开帧");
        self.renderer
            .record(zephyra_winit_d3d12::RenderCtx {
                device: &self.gpu.device,
                list: self.gpu.list(),
                rtv: self.target.rtv,
                display_list: list,
                size: (SIZE, SIZE),
                clear: Color::rgb(0.0, 0.0, 0.0),
                textures: &self.textures,
                rt: Some(&self.target.texture),
                damage: zephyra_core::frame::Damage::Full,
            })
            .expect("记录绘制命令失败");
        let staging = self
            .target
            .record_readback(&self.gpu.device, self.gpu.list())
            .expect("记录回读拷贝失败");
        self.gpu.submit().expect("提交失败");
        self.gpu.wait_gpu().expect("等 GPU 失败");
        Offscreen12::map(&staging, (SIZE, SIZE))
    }
}

// ---------------------------------------------------------------------------
//  对照
// ---------------------------------------------------------------------------

/// D3D11 的 **BGRA → RGBA**。只对 D3D11 用。
///
/// ⚠ D3D12 的 `R8G8B8A8_UNORM` **本来就是 RGBA**，对它再做一次同样的重排等于
/// 自己把红蓝换掉 —— 而这种错误两边都只差 R/B，断言看上去像「某个模式算错了」，
/// 实际上是这个夹具错了。所以两个后端各有各的换算函数，不共用一个。
fn bgra_to_rgba(bgra: &[u8]) -> Vec<[u8; 4]> {
    bgra.chunks_exact(4)
        .map(|p| [p[2], p[1], p[0], p[3]])
        .collect()
}

/// D3D12 的 `R8G8B8A8_UNORM` → RGBA，**不换通道**。
fn rgba_as_is(px: &[u8]) -> Vec<[u8; 4]> {
    px.chunks_exact(4)
        .map(|p| [p[0], p[1], p[2], p[3]])
        .collect()
}

/// **全部 29 个模式**逐个对照，不抽样。
///
/// 抽样只能证明抽样的那几个，而漏掉一个模式的后果正是「那个模式静默失效」。
fn all_modes() -> Vec<BlendMode> {
    // 按 `filter.rs` 里枚举的**声明顺序** —— 顺序本身就是契约的一部分。
    vec![
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
        BlendMode::Overlay,
        BlendMode::Darken,
        BlendMode::Lighten,
        BlendMode::ColorDodge,
        BlendMode::ColorBurn,
        BlendMode::HardLight,
        BlendMode::SoftLight,
        BlendMode::Difference,
        BlendMode::Exclusion,
        BlendMode::Multiply,
        BlendMode::Hue,
        BlendMode::Saturation,
        BlendMode::Color,
        BlendMode::Luminosity,
    ]
}

// 诊断：换前景色，看 D3D12 的 SoftLight 输出跟不跟着变。
// 跟着变 → 真的跑了 PSSoftLight；不变 → 那个 PS 根本没被用上。
/// **已知分歧**：这些模式两个后端画出来的不一样，先记在这里。
///
/// # 为什么不直接删掉这条判据
///
/// 删掉就等于「SoftLight 在两个后端上不一样」这件事**没有任何测试知道**。
/// 留一个显式的例外清单，好处是：
///
/// 1. 清单之外的**任何**新分歧仍然会让本条失败；
/// 2. 这个例外是**有名字、有证据**的，不是「把容差调大到能过」；
/// 3. 修好之后从清单里删掉即可，不需要重写判据。
///
/// # SoftLight 的症状（2026-10-10 记录）
///
/// ```text
/// 前景 (0.8,0.4,0.2,α=0.6)   D3D11 = [138, 95, 73]   D3D12 = [51, 51, 51]
/// 前景 (0.3,0.9,0.5,α=0.4)   D3D11 = [ 98,142,113]   D3D12 = [76, 76, 76]
/// 前景 (0.5,0.5,0.5,α=1.0)   D3D11 = [ 90, 90, 90]   D3D12 = [ 0,  0,  0]
/// ```
///
/// **D3D12 那一列三个通道恒等**，而混合模式在定义上是**逐通道**的函数 ——
/// 同一个 `BlendMode::SoftLight` 在别的通道组合上不该退化成标量。所以问题出在
/// 「某个逐通道输入退化成了标量」，而不是 SoftLight 的公式本身（两个后端共用**同一份**
/// `.hlsl` 与**同一份** `.bin`，公式错会**两边一起错**）。
///
/// 已排除：混合模式下标到 PS 变体的映射（`21 → PSSoftLight`）、
/// PSO 缓存键 `(ps_idx, mode, samples)`、通道顺序（`R8G8B8A8` vs `B8G8R8A8`
/// 已各自换算，见 `rgba_as_is` / `bgra_to_rgba`）、源色是否到达着色器
/// （换前景色 D3D12 的输出**跟着变**）。
///
/// 尚未排除：`g_dst` 在该 PS 下被采样成标量、顶点色在 `ShadeSource` 里被广播、
/// 以及 `BSoftLight` 里 `step` / `sqrt` 在某个后端被降成标量运算。
///
/// # 2026-10-12：SoftLight 已收敛，例外清单清空
///
/// 上面记的那个症状（D3D12 侧三通道恒等）不再出现 —— 两个后端现在逐像素一致。
/// 本仓的规矩是**清单不许养着过期条目**：留着等于「SoftLight 仍然有分歧」这个
/// 已经不成立的说法还在替测试挡路，而 `the_known_divergence_list_is_not_stale`
/// 就是专门盯这件事的门禁（它一响，就是该删条目了）。
///
/// 清单保留（哪怕是空的）：它是**结构**而不是备注 —— 下一个真分歧会被显式登记
/// 在这里、带着证据和名字，而不是靠调大容差蒙过去。
const KNOWN_DIVERGENT: &[&str] = &[];

fn known(mode: BlendMode) -> bool {
    KNOWN_DIVERGENT.contains(&format!("{mode:?}").as_str())
}

#[test]
fn every_blend_mode_looks_the_same_on_both_backends() {
    let mut d11 = D3D11::new();
    let mut d12 = D3D12::new();

    let mut diverged = Vec::new();
    for mode in all_modes() {
        if known(mode) {
            continue;
        }
        let list = scene(mode);
        let a = bgra_to_rgba(&d11.render(&list));
        let b = rgba_as_is(&d12.render(&list));
        assert_eq!(a.len(), b.len(), "{mode:?} 两侧像素数不同");

        // 只看中心那一块：图形铺满全屏，任意采样点都该一致；取中心是为了出错时
        // 坐标好读。
        let (x, y) = ((SIZE / 2) as usize, (SIZE / 2) as usize);
        let (pa, pb) = (a[y * SIZE as usize + x], b[y * SIZE as usize + x]);
        let worst = (0..4)
            .map(|i| (pa[i] as i32 - pb[i] as i32).abs())
            .max()
            .unwrap_or(0);
        if worst > TOL {
            diverged.push(format!("{mode:?}(差 {worst}：D3D11={pa:?} D3D12={pb:?})"));
        }
    }
    assert!(
        diverged.is_empty(),
        "这些混合模式在两个后端上画出来的不一样：\n  {}\n\
         ⚠ 已知例外见本文件顶部的 `KNOWN_DIVERGENT` —— 如果不在那份清单里，\
         说明又出现了一个新分歧，那才是要立刻查的。",
        diverged.join("\n  ")
    );
}

/// 已知分歧的**反向**门禁：清单里的模式必须**仍然**是分歧的。
///
/// 为什么要这条：例外清单最大的风险是「修好了但忘了删」，于是判据从
/// 「抓 SoftLight」退化成「什么都不抓」——而这正是本仓栽过两次的恒真测试。
/// 所以反向钉一次：真的修好之后本条会失败，提醒把清单改掉。
#[test]
fn the_known_divergence_list_is_not_stale() {
    let mut d11 = D3D11::new();
    let mut d12 = D3D12::new();
    for name in KNOWN_DIVERGENT {
        let mode = all_modes()
            .into_iter()
            .find(|m| format!("{m:?}") == *name)
            .unwrap_or_else(|| panic!("KNOWN_DIVERGENT 里的 {name} 不是一个合法模式"));
        let list = scene(mode);
        let a = bgra_to_rgba(&d11.render(&list));
        let b = rgba_as_is(&d12.render(&list));
        let (x, y) = ((SIZE / 2) as usize, (SIZE / 2) as usize);
        let (pa, pb) = (a[y * SIZE as usize + x], b[y * SIZE as usize + x]);
        let worst = (0..4)
            .map(|i| (pa[i] as i32 - pb[i] as i32).abs())
            .max()
            .unwrap_or(0);
        assert!(
            worst > TOL,
            "{name} 已经不再分歧了（差 {worst}）—— 修好了？\
             请把它从 `KNOWN_DIVERGENT` 里删掉，否则上面那条就少抓一个模式"
        );
    }
}

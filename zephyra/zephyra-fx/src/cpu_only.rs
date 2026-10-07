//! 离线路径：布局 + 排版 + `DisplayList`，**完全不碰 GPU**。
//!
//! # 为什么需要它
//!
//! 本机上 GPU 侧是坏的（D3D11 的 `RSSetViewports` 设不进去，见
//! `tests/d3d11_draw.rs`）。于是正常路径的每一帧只能验到「`DisplayList` 已产出」
//! 为止，中间的 [`crate::quad::expand`]、字形图集打包、顶点流**全部没有证据**。
//!
//! 更糟的是「空画面」这件事**无法自证**。本项目已经反复栽在这里：看到窗口是空的，
//! 就怀疑 GPU，然后一轮轮调 GPU 参数；而真相可能在 `quad::expand` 里，也可能在一张
//! 全是 0 的图集里 —— 两者在屏幕上长得一模一样。
//!
//! 这个模块的价值就在于此：它把「画面空」的**排查顺序**固定下来。
//!
//! ```text
//! 1. DisplayList 有内容吗？        → CpuHost::list()
//! 2. expand 产出顶点了吗？        → CpuHost::frame().render
//! 3. 顶点流画出来对吗？            → CpuHost::render_cpu()
//!    ↓ 以上都过了
//! 4. 那才轮到 GPU
//! ```
//!
//! # 不需要窗口
//!
//! 它没有 `Window`、没有 `Gpu`、没有 `Surface`。尺寸是构造时传入的 —— 因为「布局需要
//! 什么尺寸」这件事与「有没有窗口」无关。这让它能在 CI、无显示器的机器、以及
//! GPU 坏掉时跑。
//!
//! # 它**不**证明 GPU 能画
//!
//! 本机当前的状态恰恰是「CPU 图正确、GPU 图空白」。所以本模块给出的结论是
//! 「顶点流是对的」，而不是「渲染是对的」。

use zephyra_core::context::Context;
use zephyra_core::frame::{Damage, FrameCache, FrameOutcome};
use zephyra_core::input::InputRouter;
use zephyra_core::paint::DisplayList;
use zephyra_core::diff::DiffStats;
use zephyra_core::theme::Theme;

use crate::cpu_raster::{self, CpuCanvas};
use crate::quad;
use crate::texture::TextureNames;
use crate::text;
use crate::{Frame, View};

/// 文本引擎 —— 与 [`crate::TextEngine`] 是**同一个类型**。
///
/// 复用而不另立别名，是因为两条路径必须跑同一套排版与栅格化代码；一旦分成两个类型，
/// 它们迟早会各自演化，而「CPU 参考图对、GPU 图不对」就不再是有效证据。
pub type CpuTextEngine = crate::TextEngine;

/// 一帧各阶段的耗时（微秒）。
///
/// # 为什么要按阶段拆，而不是只报总时间
///
/// 总时间只能回答"慢"，回答不了"慢在哪"。而这条管线每一段的责任方不同：
///
/// | 阶段 | 归属 | 典型瓶颈 |
/// |---|---|---|
/// | `view` | 宿主 | 重建声明树 |
/// | `diff` | core | 逐属性对账 |
/// | `layout` | core | 递归摆矩形 |
/// | `paint` | core | **排版 + 字形栅格化**（见 [`Self::paint`] 的警告）|
/// | `expand` | fx | `DisplayList` → 顶点 |
///
/// 拆开之后"哪一段吞掉了帧时间"是**读出来的**，不用靠猜。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct StageTimes {
    /// `View::view()`：构造声明树。
    pub view_us: u32,
    /// `diff()`：声明树与保留树对账。
    pub diff_us: u32,
    /// `layout_tree()`：递归算矩形。
    pub layout_us: u32,
    /// `paint::render()`：树 → `DisplayList`（含文本排版与字形栅格化）。
    pub paint_us: u32,
    /// `quad::expand()`：`DisplayList` → 顶点流。
    pub expand_us: u32,
    /// 上面五段之和。
    pub total_us: u32,
}

impl StageTimes {
    /// 五段累加。
    pub fn sum(&self) -> u32 {
        self.view_us + self.diff_us + self.layout_us + self.paint_us + self.expand_us
    }
}

pub struct CpuHost<V: View> {
    view: V,
    theme: &'static Theme,
    /// 保留树跨帧存活，与窗口版同构。
    /// 帧缓存 —— 与窗口宿主走**同一条**判据，避免两条路各判各的。
    cache: FrameCache,
    /// 布局用的像素尺寸。独立于窗口 —— 离线路径没有窗口可问。
    size: (u32, u32),
    engine: std::cell::RefCell<CpuTextEngine>,
    textures: TextureNames,
    /// 输入路由器。
    ///
    /// 离线路径不处理鼠标事件，但保留它是为了 [`Self::hovered_key`] 有真实来源 ——
    /// 那让「hover 目标」在两条路径上是同一个概念，而不是两套。
    router: InputRouter,
    /// 跨帧复用的展开缓冲（顶点流 + 索引）。
    ///
    /// 稳态下它让 `quad::expand` 一帧**零分配** —— 展开要开六七个 `Vec`，每帧重建
    /// 就等于每帧付一串"从 0 增长"的分配与拷贝（实测 demo 一帧 302 次 / 172 KB，
    /// 其中约 69% 出在这里）。
    scratch: quad::Scratch,
}

impl<V: View> CpuHost<V> {
    /// 建一个离线宿主。
    ///
    /// 找不到系统字体时**不** panic：字体缺失应当让文本不显示，而不是让程序起不来
    /// —— 与各后端宿主的 `HostWindow::new_lenient` 同一个取舍。
    pub fn new(view: V, size: (u32, u32)) -> Self {
        let (engine, note) = text::from_system_font();
        eprintln!("[zephyra:cpu] {note}");
        Self {
            view,
            // `Theme` 是静态引用：`Box::leak` 让它活到进程结束。离线路径只活一帧，
            // 但保持与窗口版**同一个**取法，免得两条路径的主题悄悄不同。
            theme: Box::leak(Box::new(Theme::dark())),
            cache: FrameCache::new(),
            size,
            engine: std::cell::RefCell::new(engine),
            textures: TextureNames::new(),
            router: InputRouter::new(),
            scratch: quad::Scratch::new(),
        }
    }

    /// 布局用的尺寸。
    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// 改尺寸（等价于窗口 resize）。会触发下一帧重排。
    ///
    /// 不用显式标脏：`FrameCache` 每次 [`Self::frame_timed`] 都比对视口，尺寸变了
    /// 自然走重排 + 整帧重绘。少一个状态位，少一处能忘的地方。
    pub fn resize(&mut self, size: (u32, u32)) {
        self.size = size;
    }

    /// 当前 hover 目标的 key。
    pub fn hovered_key(&self) -> Option<String> {
        self.router.hovered().map(|k| k.to_string())
    }

    /// 图集统计（字形数、用量）。用来回答"字到底有没有被栅格化进去"。
    pub fn atlas_stats(&self) -> zephyra_core::text::AtlasStats {
        self.engine.borrow().atlas_stats()
    }

    /// 跑一帧的前半段：元素树 → 布局 → `DisplayList` → 顶点展开。
    ///
    /// 返回的 [`Frame::render`] 里带 `quads` / `skipped` / `missing_textures` ——
    /// 这是「顶点流里到底有什么」的第一手证据。
    pub fn frame(&mut self) -> Frame {
        self.frame_timed().0
    }

    /// 与 [`Self::frame`] 相同，额外给出各阶段耗时。
    ///
    /// 计时用 [`std::time::Instant`]。**它不进稳态路径** —— `frame()` 不带计时，
    /// 所以真实宿主（两个 `HostWindow`）的每帧开销里没有这一项。
    ///
    /// 为什么不把计时留在 `frame()` 里而做成两个方法：那会让每个 `Instant::now()`
    /// （一次 `QueryPerformanceCounter`，约 20~30ns）留在稳态路径上，更重要的是
    /// 会让"被测量的那条路径"与"真实跑的那条路径"变成两段代码 —— 而本项目反复栽
    /// 过的坑正是"观测代码自己出错，症状与要排除的故障同形"。
    pub fn frame_timed(&mut self) -> (Frame, StageTimes) {
        let theme = self.theme;
        let mut t = StageTimes::default();
        // 作用域要短：`ctx` 借着 `self.engine`，而下面还要可变借用 `self`。
        let t0 = std::time::Instant::now();
        let (diff, damage, redrawn) = {
            // 挂上**内置**图标集 —— 于是 `--dump-cpu` 与窗口路径走同一条几何链，
            // 图标在 CPU 参考图里也可见。
            //
            // 用 `with_engine(...)` 再 `.with_icons(...)` 而不是直接 `with_all`：
            // 布局与命中测试不需要图标，只有渲染趟要；而 `Context` 是 `Copy`，
            // 挂一次到处都能用。
            let ctx = Context::with_engine(theme, &self.engine)
                .with_icons(zephyra_core::icon::builtin());
            let element = self.view.view(&ctx);
            let size = self.size;
            t.view_us = t0.elapsed().as_micros() as u32;

            // ⚠ 逐阶段计时现在**盖不住** diff / layout / paint 三段了 ——
            // 它们搬进了 `zephyra_core::frame::FrameCache`，本模块拿不到内部分段耗时。
            //
            // 所以这三项恒为 0，而 `view_us` / `expand_us` 仍是真值。
            //
            // 这不是测量退化而是**诚实**：把一个数写成"看起来还在测"比不测更糟。
            // 要恢复分段计时就得让 `FrameCache` 暴露回调，那会把它从"纯计算"变成
            // "带观测钩子" —— 而它的全部价值就在于能在无 GPU 环境里断言。
            // 真要分段，直接改 `frame.rs` 加临时探针（注意探针本身的开销）。
            let outcome = self.cache.frame(&element, ctx, (size.0 as f32, size.1 as f32));

            match outcome {
                FrameOutcome::Unchanged => (DiffStats::default(), Damage::Full, false),
                FrameOutcome::Updated {
                    list: _,
                    stats,
                    damage,
                } => (stats, damage, true),
            }
        };

        let t4 = std::time::Instant::now();
        // `Unchanged` 时仍然展开：CPU 参考光栅器是**离屏单帧**工具（`--dump-cpu`
        // 只跑一帧），它的产物要能从上一帧的列表复现。门控省的是 GPU 那条路，
        // 不是这里。
        let render =
            quad::expand_into(self.cache.list(), self.size, &|n| self.textures.contains(n), &mut self.scratch);
        t.expand_us = t4.elapsed().as_micros() as u32;
        t.total_us = t.sum();

        (
            Frame {
                diff,
                render,
                // 离线路径不经路由器产生 hover，所以恒为 `None`。这是**事实**而不是占位
                // —— 写成 `self.router.hovered()` 会让人以为离线路径也支持 hover。
                hovered: None,
                damage,
                redrawn,
            },
            t,
        )
    }

    /// 上一帧的 `DisplayList`（供直接检查命令内容）。
    pub fn list(&self) -> &DisplayList {
        self.cache.list()
    }

    /// 上一帧的顶点流（`quad::expand` 的完整结果，含 `solid` 与图像批次）。
    ///
    /// 单独暴露是为了让调用方能**数顶点** —— `RenderReport::vertex_bytes` 在
    /// `expand` 这一层恒为 0（它由渲染器填），所以判断「顶点流非空」得看
    /// `batch.solid.len()`。第一版 `--dump-cpu` 打印 `vertex_bytes = 0` 就以为
    /// 什么都没进去，而同一次调用里 `quads = 135`。
    pub fn batch(&self) -> quad::QuadBatch {
        // 这里**不复用** [`Self::scratch`]：本方法签名是 `&self`，而复用要 `&mut`。
        // 它是给诊断用的旁路（数顶点、跑 CPU 光栅），不在稳态路径上 ——
        // 真要它也复用，得把签名改成 `&mut self`，而那会让只想读一下的调用点
        // 都被迫拿可变借用。
        quad::expand(self.cache.list(), self.size, |n| self.textures.contains(n))
    }

    /// 跨帧复用的展开缓冲的容量，供诊断"稳态是否还在长"。
    pub fn scratch_capacities(&self) -> quad::ScratchCaps {
        self.scratch.capacities()
    }

    /// 把上一帧的顶点流在 CPU 上画出来。
    ///
    /// 与 GPU 吃的是**同一个** `quad::expand` 输出 —— 所以图对就证明顶点流对。
    /// 图像（`kind == IMAGE`）不画：它绑的是图像纹理而非字形图集，见
    /// [`crate::cpu_raster`] 的说明。
    pub fn render_cpu(&self, clear: zephyra_core::core::Color) -> CpuCanvas {
        let batch = self.batch();
        let engine = self.engine.borrow();
        let atlas = engine.atlas();
        let bitmap = cpu_raster::AtlasBitmap {
            pixels: atlas.pixels().to_vec(),
            size: atlas.size(),
        };
        let mut canvas =
            CpuCanvas::new(self.size.0, self.size.1, [clear.r, clear.g, clear.b, clear.a]);
        cpu_raster::draw_batch(&mut canvas, &batch, Some(&bitmap));
        canvas
    }

    /// 图集像素（RGBA8，尺寸见 [`Self::atlas_size`]）。
    ///
    /// GPU 后端上传的**就是这一份**。暴露它是为了让「同一份图集 + 同一份
    /// `DisplayList`」能被两条渲染路径消费 —— 那才是 CPU 参考图能当**判据**的前提：
    /// 参考与实测若各自有自己的图集，它们在「图集打包错了」这件事上会一起错，
    /// 对比结果永远是绿的。
    pub fn atlas_pixels(&self) -> Vec<u8> {
        self.engine.borrow().atlas().pixels().to_vec()
    }

    /// 图集边长（像素，边长而非宽高）。
    pub fn atlas_size(&self) -> u32 {
        self.engine.borrow().atlas().size()
    }

    /// 写一张 BMP。
    ///
    /// 用 sRGB 编码写：这是**给人看**的那一步，编码必须发生在这里而不是渲染里
    /// —— 混进渲染会让「像素值对不对」这个判据变得不可用。
    pub fn dump_cpu_bmp(&self, path: &str, clear: zephyra_core::core::Color) -> std::io::Result<()> {
        let canvas = self.render_cpu(clear);
        crate::write_bmp(
            path,
            &crate::pixels::PixelRead {
                width: canvas.width,
                height: canvas.height,
                row_pitch: canvas.width * 4,
                declared_width: canvas.width,
                declared_height: canvas.height,
                data: canvas.pixels.clone(),
            },
            true,
        )
    }
}

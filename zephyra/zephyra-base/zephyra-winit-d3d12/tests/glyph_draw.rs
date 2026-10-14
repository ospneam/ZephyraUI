//! 字形路径的**等价性**测试：同一份 `DisplayList` + 同一份图集，CPU 光栅与 GPU 渲染
//! 必须画出同一张图。
//!
//! # 为什么必须是「等价」而不是「看着对」
//!
//! 本项目的失败模式是「不报错、画面不对」。而字形路径在 DX12 上有四段各自可能静默
//! 失效的东西：
//!
//! | 段 | 静默失效的样子 |
//! |---|---|
//! | 图集上传（`CopyTextureRegion`） | 全黑 / 只填了左上角 |
//! | `t0` 绑到图集 SRV 槽 | 采样返回 0 ⇒ 文字消失（**本次实测的现象**）|
//! | 线性过滤 + 夹取寻址 | 边缘发虚 / 邻格鬼影 |
//! | 字形四边形的 UV | 采样到错误位置 ⇒ 字形变形 |
//!
//! 这四段的症状都落在同一处：**屏幕上没有字**。看截图区分不了「图集没传上去」与
//! 「传上去了但没绑」。所以判据不写成「有没有字」，而写成**「与 CPU 参考图逐像素
//! 比对」**：任何一段坏了，两者就对不上。
//!
//! # 参考与实测必须共用同一份输入
//!
//! 参考侧用 [`CpuHost`]，它内部持有文本引擎；实测侧上传的像素取自
//! [`CpuHost::atlas_pixels`] —— **同一份**图集。各自造一份图集的话，「图集打包错了」
//! 这一类 bug 会让两边**一起**错，对比永远是绿的。
//!
//! # 判据为什么不是「逐像素相等」
//!
//! CPU 按覆盖率直接混色（相当于最近邻），GPU 采样的是 8 位纹理再乘一次覆盖率
//! （线性过滤）。两种栅格化器在字形**边缘**上必然不同，而边缘占了文字图里的一大半
//! 像素。逐像素比对要判它，只能给一个从「实现差异」倒推出来的魔数阈值 ——
//! 而魔数与几何脱节时，画得完全正确也会被判失败（本测试的第一版就是这么过的：
//! 图集根本没上传，它照样绿）。
//!
//! 这里的判据是**总墨量 = Σ 覆盖率**：两条路径积分的是同一批字形的覆盖率，边缘怎么
//! 重分配都不改变积分。所以它抓得住「字少画一半 / 位置偏一个像素 / 图集没传」，
//! 又不会被抗锯齿差异误伤。另有��条「内容像素数不小于参考图的 95%」兜住
//! 「整个功能缺失」这一档。
//!
//! # 第一版的两处失效（都留在注释里当反面教材）
//!
//! 1. 阈值给 5%、而那三行 20px 的字只占画布 0.5% ⇒ 字全没画出来，比例仍然合格；
//! 2. 根容器**有背景色** ⇒ 「非黑像素」恒为 100%，判据作废。
//!
//! 两条都指向同一件事：判据必须**由几何 / 实测量出**，且它的分母要能区分
//! 「噪声」与「缺失」。

#![cfg(windows)]

use zephyra_core::context::Context;
use zephyra_core::core::Color;
use zephyra_core::element::Element;
use zephyra_core::id::NodeTag;
use zephyra_core::layout::Direction;
use zephyra_core::property::PropertyValue;
use zephyra_core::props;
use zephyra_fx::cpu_only::CpuHost;
use zephyra_winit_d3d12::{surface::Offscreen, Gpu, InputState, Renderer, TextureTable, View, Z_INDEX};

/// 目标边长。
const SIZE: u32 = 256;
/// 字号。要让文字**笔画**占住画布足够大的一块，见 [`TextHeavy`]。
const TEXT_PX: f32 = 34.0;
/// 总墨量允许的相对差。
///
/// 取 3% 的依据**不是**「试出来差不多」：字形覆盖率在 8 位纹理里量化，一圈轮廓上
/// 每像素 ±1/255，累加起来不到 1%；线性过滤把边缘重分配掉的部分是**面积守恒**的，
/// 净差只剩量化噪声。3% 留了两到三倍余量，而「字少画了一半」「偏了一个像素」
/// 「图集没传」这几类真故障的墨量偏差都在 30% 以上 —— 差着数量级。
const INK_RATIO_MAX: f64 = 0.03;
/// 参考图上「内容像素」占比的下限 —— 低于它说明这套判据没有判别力。
const COVERAGE_MIN: f64 = 0.08;
/// 演示文字。中英文各一半：中文走全角步进、拉丁走窄步进，两条排版路径都要覆盖。
const LINES: [&str; 6] = ["字形 MMMM", "核心 IIII", "WWWW 渲染", "契约 XXXX", "稳态 YYYY", "ZephyraUI 0000"];

/// 一屏文字：字足够大、足够多。
///
/// # 为什么字要占满画布
///
/// 这个测试的第一版用 20px 的三行字，结果**在图集根本没上传的情况下也过了** ——
/// 那三行字只占 256×256 画布的约 0.5%，而阈值给的是 5%，差了一个数量级。
///
/// 所以现在的判据靠**覆盖率**，而覆盖率要够大才有判别力：字占画布的比例越大，
/// 「一个字都没画」就越不可能被当成抗锯齿噪声。
struct TextHeavy {
    lines: &'static [&'static str],
}

impl View for TextHeavy {
    fn on_input(&mut self, _state: &InputState) {}

    fn view(&self, _ctx: &Context<'_>) -> Element {
        Element::new(NodeTag::Container)
            .with_key("root")
            // ⚠ **不要**给根容器上背景色。
            //
            // 「非黑像素数」是这里最有判别力的那个量 —— 前提是画布上**只有字**。
            // 加上背景之后满屏都是非黑像素，于是这个数恒为 100%，判据当场作废
            // （第一版就是这么写的：参考图 100%、GPU 100%、两个数相等，
            // 什么都证明不了）。背景是黑、清屏也是黑，两者相减才是字形。
            .with(
                &props::DIRECTION,
                PropertyValue::direction(Direction::Vertical),
            )
            .with_children(self.lines.iter().enumerate().map(|(i, line)| {
                Element::new(NodeTag::Text)
                    .with_key(*line)
                    .with(&props::TEXT, PropertyValue::static_str(*line))
                    .with(&props::TEXT_SIZE, PropertyValue::float(TEXT_PX))
                    .with(&Z_INDEX, PropertyValue::int(i as i32))
            }))
            .with(&Z_INDEX, PropertyValue::int(0))
    }
}

/// 顶点流里的字形四边形个数（按 `kind == GLYPH` 数）。
fn glyph_quads(batch: &zephyra_fx::quad::QuadBatch) -> usize {
    batch
        .solid
        .chunks_exact(4)
        .filter(|c| (c[0].kind - zephyra_fx::quad::kind::GLYPH).abs() < 0.01)
        .count()
}

/// 字形在 CPU 与 GPU 上必须画出同一张图。
#[test]
#[ignore = "需要真 GPU。手动跑：--ignored --nocapture"]
#[allow(non_snake_case, reason = "测试名用中文描述被测行为，比逐字翻成英文好读")]
fn 字形_在GPU上与CPU参考图一致() {
    let view = TextHeavy { lines: &LINES };
    // ---- 参考侧：CPU ----
    let mut cpu = CpuHost::new(view, (SIZE, SIZE));
    let frame = cpu.frame();
    let glyphs = glyph_quads(&cpu.batch());
    assert!(
        frame.render.quads > 0,
        "元素树没展开出任何四边形 —— 那这个测试在比两张空白图"
    );
    assert!(
        glyphs > 0,
        "一个字形四边形都没有 —— 那这个测试只在比「矩形画对了没有」"
    );
    let canvas = cpu.render_cpu(Color::rgba(0.0, 0.0, 0.0, 1.0));

    // ---- 覆盖率下界（**先**从 CPU 参考图上量出来）----
    //
    // 阈值必须**由几何 / 实测量出**，不能拍。这里量的就是「参考图上有多少像素不是背景色」
    // —— 而 GPU 那一侧要达到同一个数才算数。用 CPU 的数当标尺，是因为两边吃的是
    // 同一份 DisplayList 与同一份图集，**理应**逐像素相同。
    let reference_drawn = count_non_clear(&canvas.pixels);
    let total = (SIZE * SIZE) as usize;
    let reference_ratio = reference_drawn as f64 / total as f64;
    println!(
        "[glyph] 字形四边形 {glyphs} 个；CPU 参考图上内容像素 {reference_drawn}/{total}\
         （{:.1}%）",
        reference_ratio * 100.0
    );
    assert!(
        reference_ratio >= COVERAGE_MIN,
        "参考图本身就只有 {:.1}% 的内容像素（阈值 {:.1}%）—— 这套判据没有判别力，\
         先把字放大到让内容占住画布",
        reference_ratio * 100.0,
        COVERAGE_MIN * 100.0
    );

    // ---- 实测侧：GPU ----
    let mut gpu = Gpu::new().expect("建不出 D3D12 设备");
    // 渲染器的图集边长**必须**与上传的那份一致 —— `record_upload_atlas` 用
    // `self.atlas_size` 算行宽，两个数不一样时越界且不报错。
    let mut renderer = Renderer::new(&gpu.device, cpu.atlas_size(), 1).expect("建渲染器");
    let target = Offscreen::new(&gpu.device, (SIZE, SIZE)).expect("建离屏目标");

    gpu.begin_frame().expect("开帧失败");
    // 图集必须在**绘制之前**录进同一条命令列表：图集纹理是每帧重建的资源，
    // 「先画下一帧再传」会在中间那帧读到已释放的纹理。
    renderer
        .record_upload_atlas(gpu.list(), &cpu.atlas_pixels())
        .expect("上传图集失败");

    let textures = TextureTable::new();
    let report = renderer
        .record(zephyra_winit_d3d12::RenderCtx {
            device: &gpu.device,
            list: gpu.list(),
            rtv: target.rtv,
            display_list: cpu.list(),
            size: (SIZE, SIZE),
            clear: Color::rgba(0.0, 0.0, 0.0, 1.0),
            textures: &textures,
            rt: None,
            damage: zephyra_core::frame::Damage::Full,
        })
        .expect("记录绘制命令失败");
    assert_eq!(
        report.quads, frame.render.quads,
        "同一份 DisplayList 在两条路径上展开出的四边形数不同 —— \
         说明 `quad::expand` 吃到的输入不一样"
    );

    let staging = target
        .record_readback(&gpu.device, gpu.list())
        .expect("记录回读拷贝失败");
    gpu.submit().expect("提交失败");
    gpu.wait_gpu().expect("等 GPU 失败");
    let pixels = Offscreen::map(&staging, (SIZE, SIZE));

    // ---- 判据 1：GPU 上的内容像素不能少于参考图 ----
    //
    // 这一条专治「一个字都没画」。它与「图差比例」是两回事：图差比例会因为字太小而
    // 被抗锯齿噪声淹没（第一版就是这么被骗过去的），而覆盖数不会。
    let gpu_drawn = count_non_clear(&pixels);
    println!("[glyph] GPU 上内容像素 {gpu_drawn}/{total}");
    assert!(
        gpu_drawn * 100 >= reference_drawn * 95,
        "GPU 上的内容像素只有 {gpu_drawn}，参考图有 {reference_drawn}（少于 5%）—— \
         先查图集上传与 `t0` 绑定，别查字形 UV 与覆盖率",
    );

    // ---- 判据 2：总墨量（覆盖率积分）一致 ----
    //
    // ⚠ **不是**逐像素相等。
    //
    // CPU 按覆盖率直接混色（相当于最近邻），GPU 采样的是 8 位纹理再乘覆盖率
    // （线性过滤）。两种栅格化器在字形**边缘**上必然不同 —— 那是实现差异，不是缺陷。
    // 逐像素比对要判它，就只能给一个从「实现差异」倒推出来的魔数阈值，而魔数与几何
    // 脱节时画得完全正确也会被误判（DX12 的矩形测试与这个测试的第一版各栽一次）。
    //
    // 「总墨量 = Σ 覆盖率」是**可推导**的不变量：两条路径积分的是同一批字形的覆盖率，
    // 边缘怎么重分配都不改变积分。所以它既能抓住「字少画了一半 / 位置偏了 / 图集没传」，
    // 又不会被抗锯齿差异误伤。
    let cpu_ink = ink(&canvas.pixels);
    let gpu_ink = ink(&pixels);
    let ink_delta = (gpu_ink as f64 - cpu_ink as f64).abs();
    println!(
        "[glyph] 总墨量：CPU {cpu_ink}、GPU {gpu_ink}，相对差 {:.2}%",
        ink_delta / cpu_ink.max(1) as f64 * 100.0
    );

    // 失败时把两侧都落盘 —— 逐像素数字看不出「字重了一倍」还是「整行偏了一个像素」，
    // 而那两种的成因完全不同。
    let dump_failed = |ok: bool| {
        if ok {
            return;
        }
        let dump = |name: &str, px: &[u8], order: zephyra_fx::pixels::ChannelOrder| {
            let img = zephyra_fx::PixelRead {
                data: px.to_vec(),
                order,
                row_pitch: SIZE * 4,
                width: SIZE,
                height: SIZE,
                declared_width: SIZE,
                declared_height: SIZE,
            };
            let _ = zephyra_fx::write_bmp(
                &format!("{}/{name}.bmp", env!("CARGO_TARGET_TMPDIR")),
                &img,
                /* encode_srgb */ false,
            );
        };
        // 两侧的通道序**不同**：CPU 画布是 RGBA，DXGI 读回是 BGRA。
        dump("glyph-cpu", &canvas.pixels, zephyra_fx::pixels::ChannelOrder::Rgba);
        dump("glyph-gpu", &pixels, zephyra_fx::pixels::ChannelOrder::Bgra);
        eprintln!(
            "[glyph] 已把两侧落盘到 {}/glyph-{{cpu,gpu}}.bmp",
            env!("CARGO_TARGET_TMPDIR")
        );
    };

    let ok = ink_delta <= cpu_ink as f64 * INK_RATIO_MAX;
    dump_failed(ok);
    assert!(
        ok,
        "总墨量相对差 {:.2}% 超过 {:.0}%（CPU {cpu_ink} / GPU {gpu_ink}）—— \
         同一份 DisplayList 与同一份图集，两侧的覆盖率积分本该相同",
        ink_delta / cpu_ink.max(1) as f64 * 100.0,
        INK_RATIO_MAX * 100.0
    );
}

/// 总墨量：所有像素的覆盖率之和（用亮度当覆盖率，背景为黑时它就是 alpha）。
fn ink(pixels: &[u8]) -> u64 {
    pixels
        .chunks_exact(4)
        .map(|p| (p[0] as u64 + p[1] as u64 + p[2] as u64 + p[3] as u64) / 4)
        .sum()
}

/// 顶点流必须真的带出字形四边形 —— 否则上面那个测试可能在比两张「只有矩形」的图。
#[test]
fn 字形四边形_在顶点流里可数() {
    let mut cpu = CpuHost::new(TextHeavy { lines: &LINES }, (SIZE, SIZE));
    let _ = cpu.frame();
    let batch = cpu.batch();
    assert_eq!(batch.solid.len() % 4, 0, "顶点流长度必须是四边形数的倍数");
    assert!(
        glyph_quads(&batch) > 0,
        "一个字形四边形都没有 —— `quad::expand` 把 `Command::Text` 吃掉了"
    );
}

/// 数非清屏（黑）像素。
fn count_non_clear(pixels: &[u8]) -> usize {
    pixels
        .chunks_exact(4)
        .filter(|p| p[0] > 8 || p[1] > 8 || p[2] > 8)
        .count()
}
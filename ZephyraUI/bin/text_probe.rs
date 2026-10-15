//! 文字链探针：把「一排文本 → 排版 → 图集 → 顶点」中间量全打出来。
//!
//! 用它的场合：图里"字看起来不对"（大小写乱、字形串味、间距不对）时。
//! 那种症状没有 HRESULT、没有计数异常，只有把每字符的 uv / advance /
//! measured size 打出来才能定位错在哪一段。
//!
//! 本探针还做一件事：**把每个字形对应的图集槽位打成 ASCII-art**。
//! 槽里印出来的形状必须就是那个字符 —— 串味 / 错位在 ASCII 里一眼可见，
//! 而在成品图里只是"字不对"。

use zephyra_core::context::Context;
use zephyra_core::element::Element;
use zephyra_core::id::NodeTag;
use zephyra_core::property::PropertyValue;
use zephyra_core::props;
use zephyra_fx::View;

/// 探针视图：三行不同字号的文本（同一行内字号一致，便于逐字符对号）。
struct Probe;

/// 三行文本与字号。 ASCII 对号按这个顺序来。
const LINES: [(&str, f32); 3] = [
    ("Hamburgefonstiv 0123", 14.0),
    ("The quick brown fox", 12.0),
    ("ZephyraUI Gallery", 34.0),
];

impl View for Probe {
    fn on_input(&mut self, _state: &zephyra_fx::InputState) {}
    fn view(&self, _ctx: &Context<'_>) -> Element {
        // 三行**竖排**（direction: vertical）：默认方向是 horizontal，
        // 不显式声明会把三行排成一列横队，逐字符对号就乱了。
        Element::new(NodeTag::Container)
            .with_key("root")
            .with(
                &props::DIRECTION,
                PropertyValue::direction(zephyra_core::layout::Direction::Vertical),
            )
            .with(
                &props::BACKGROUND,
                PropertyValue::color(zephyra_core::core::Color::rgb(0.05, 0.05, 0.06)),
            )
            .with_children(LINES.iter().map(|(s, size)| text(s, *size)))
    }
}

fn text(s: &'static str, size: f32) -> Element {
    Element::new(NodeTag::Text)
        .with_key(s)
        .with(&props::TEXT, PropertyValue::static_str(s))
        .with(&props::TEXT_SIZE, PropertyValue::float(size))
        .with(
            &props::TEXT_COLOR,
            PropertyValue::color(zephyra_core::core::Color::rgb(1.0, 1.0, 1.0)),
        )
}

fn main() {
    let (w, h) = (900u32, 300u32);
    let mut host = zephyra_fx::cpu_only::CpuHost::new(Probe, (w, h));
    let frame = host.frame();

    let batch = host.batch();
    let glyphs: Vec<_> = batch
        .solid
        .chunks_exact(4)
        .filter(|c| (c[0].kind - zephyra_fx::quad::kind::GLYPH).abs() < 0.01)
        .collect();
    println!("[probe] 字形 quad {} 个", glyphs.len());

    // 顶点序 = 排版序：三行竖排，所以先第 0 行、再三行各自连续。
    // 逐字符对号：把每行的字符序列与顶点序对齐。
    let mut cursor = 0usize;
    for (line, _size) in LINES.iter() {
        let chars: Vec<char> = line.chars().filter(|c| *c != ' ').collect();
        println!("[probe] 行 {line:?}（{} 个非空格字符）", chars.len());
        for ch in &chars {
            let Some(g) = glyphs.get(cursor) else { break };
            cursor += 1;
            let v0 = &g[0];
            let v2 = &g[2];
            // v0.uv = 槽位左上，v2.uv = 槽位右下（见 quad() 的顶点排布）。
            let (u0, v0u) = (v0.uv[0], v0.uv[1]);
            let (u1, v1u) = (v2.uv[0], v2.uv[1]);
            println!(
                "[probe]   '{ch}' pos_px=({:.1},{:.1}) size=({:.1},{:.1}) slot=({:.3},{:.3})-({:.3},{:.3}) adv={:.1}px",
                v0.position[0] * 450.0 + 450.0,
                (0.5 - v0.position[1] * 0.5) * 300.0,
                v0.half_size[0] * 2.0,
                v0.half_size[1] * 2.0,
                u0,
                v0u,
                u1,
                v1u,
                adv(glyphs.get(cursor), v0)
            );
            // 槽位 ASCII：把图集里这块区域打出来。形状必须就是 ch。
            ascii_slot(&host, u0, v0u, u1, v1u, *ch);
            // 渲染区 vs 槽位：逐像素比。不一致率 > 25% 记一笔 —— 那是
            // "这个字符画错了"的硬判据，不靠肉眼。
            let mismatch = compare_rendered_vs_slot(
                &host,
                v0.position[0] * 450.0 + 450.0,
                (0.5 - v0.position[1] * 0.5) * 300.0,
                u0,
                v0u,
                u1,
                v1u,
            );
            if mismatch > 0.25 {
                println!(
                    "[probe]   ⚠ '{ch}' 渲染区与槽位不一致率 {:.0}%（>25% 即画错）",
                    mismatch * 100.0
                );
            }
        }
    }

    let a = host.atlas_stats();
    println!(
        "[probe] 图集：{} 字形，用量 {:.2}%",
        a.glyphs,
        a.usage() * 100.0
    );

    let clear = zephyra_core::core::Color::rgba(0.0, 0.0, 0.0, 1.0);
    let path = "text-probe.bmp";
    host.dump_cpu_bmp(path, clear).unwrap();
    println!("[probe] 已写出 {path}");
    let _ = frame;
}

/// 到下一个字形的水平距离（NDC → 像素：canvas 宽 900）。
fn adv(next: Option<&&[zephyra_fx::quad::Vertex]>, cur: &zephyra_fx::quad::Vertex) -> f32 {
    match next {
        Some(n) => (n[0].position[0] - cur.position[0]) * 450.0,
        None => 0.0,
    }
}

/// 把图集里 [u0,u1]×[v0,v1] 这块打成 ASCII。上采样一倍便于辨认。
fn ascii_slot(
    host: &zephyra_fx::cpu_only::CpuHost<Probe>,
    u0: f32,
    v0: f32,
    u1: f32,
    v1: f32,
    ch: char,
) {
    let atlas = host.atlas_pixels();
    let side = host.atlas_size() as usize;
    let (x0, x1) = ((u0 * side as f32) as usize, (u1 * side as f32) as usize);
    let (y0, y1) = ((v0 * side as f32) as usize, (v1 * side as f32) as usize);
    if x1 <= x0 || y1 <= y0 || x1 > side || y1 > side {
        println!("[probe]     槽位越界 ({x0},{y0})-({x1},{y1})，side={side}");
        return;
    }
    let mut out = String::new();
    for y in (y0..y1).step_by(1) {
        let mut row = String::new();
        for x in x0..x1 {
            let a = atlas[(y * side + x) * 4 + 3];
            row.push(if a > 128 {
                '#'
            } else if a > 32 {
                '.'
            } else {
                ' '
            });
        }
        out.push_str(&format!("\n[probe]     |{row}|"));
    }
    println!("[probe]     槽位形状（期望 '{ch}'）：{out}");
}

/// 渲染区 vs 图集槽位：返回不一致率（0..1）。
///
/// 槽位的 alpha 通道就是覆盖率；渲染区里的字是白色乘覆盖率叠在深底上，
/// 所以 `r/255` 近似覆盖率。两者逐像素比，>0.25 记「画错」。
/// 这是不靠肉眼的判据 —— 肉眼在缩小图里分不出 'Z' 与 'z'（本次就被骗过）。
fn compare_rendered_vs_slot(
    host: &zephyra_fx::cpu_only::CpuHost<Probe>,
    x: f32,
    y: f32,
    u0: f32,
    v0: f32,
    u1: f32,
    v1: f32,
) -> f32 {
    let canvas = host.render_cpu(zephyra_core::core::Color::rgba(0.0, 0.0, 0.0, 1.0));
    let atlas = host.atlas_pixels();
    let side = host.atlas_size() as usize;
    let cw = canvas.width as usize;
    let (ax0, ax1) = ((u0 * side as f32) as usize, (u1 * side as f32) as usize);
    let (ay0, ay1) = ((v0 * side as f32) as usize, (v1 * side as f32) as usize);
    if ax1 <= ax0 || ay1 <= ay0 {
        return 1.0;
    }
    let (px0, px1) = (x as usize, (x + (ax1 - ax0) as f32).ceil() as usize);
    let (py0, py1) = (y as usize, (y + (ay1 - ay0) as f32).ceil() as usize);
    let mut diff = 0usize;
    let mut total = 0usize;
    for (dy, py) in (py0..py1).enumerate() {
        for (dx, px) in (px0..px1).enumerate() {
            let ay = ay0 + dy;
            let ax = ax0 + dx;
            if ay >= side || ax >= side {
                continue;
            }
            let want = atlas[(ay * side + ax) * 4 + 3] as f32 / 255.0;
            let got = canvas.pixels[(py * cw + px) * 4] as f32 / 255.0;
            total += 1;
            if (want - got).abs() > 0.2 {
                diff += 1;
            }
        }
    }
    if total == 0 {
        return 1.0;
    }
    diff as f32 / total as f32
}

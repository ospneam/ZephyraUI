//! Gallery 宿主：三条路（照 `examples/demo.rs` 的成熟形态）。
//!
//! 1. **窗口路**（默认）：winit + `zephyra-winit-d3d12::HostWindow`。
//! 2. **`--dump-cpu`**：`zephyra_fx::cpu_only::CpuHost` 画第一帧写 BMP，
//!    不开窗口、不碰 GPU —— 这是**像素级自验**手段（同一份样式表、同一条
//!    `DisplayList → quad::expand` 链）。
//! 3. **`--theme dark|light`**：起始主题意图。
//!
//! # 主题意图怎么到宿主
//!
//! View 住在 `HostWindow` 里面，拿不到直接引用。所以意图走一个
//! `Arc<AtomicU8>`：View 点切换时翻它，`Bootstrap` 每帧 `about_to_wait`
//! 对账一次，不一致就 `set_theme`。于是「想切的」与「在用的」始终可分开
//! 观测 —— 不会出现"点了没反应也不知道为什么"。

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;

use zephyra_core::theme::ThemeVariant;
use zephyra_ui::app::{resolve_route, Gallery, SUPPORTED};

#[cfg(feature = "d3d12")]
use zephyra_core::styler::Styler;
use zephyra_style::ir::StyleSheet;
use zephyra_style::SheetStyler;

/// 主题意图：0 = dark，1 = light。View 写、Bootstrap 读。
fn intent_cell(dark: bool) -> Arc<AtomicU8> {
    Arc::new(AtomicU8::new(if dark { 0 } else { 1 }))
}

/// 全局分配器探针（**注册为全局分配器**）。
///
/// ⚠️ #[global_allocator] 全进程只能有一个，所以本模块一旦进这个二进制，
///    --alloc-probe 就总可用 —— 与 examples 的 demo 同一形态。
#[path = "../src/alloc_probe.rs"]
mod alloc_probe;


fn main() {
    // 与 demo 同一套字体手法：自带 MiSans（含 CJK），编译期内嵌，
    // 落地到临时文件后经环境变量让 zephyra-fx 选中它。
    use_demo_font();

    let args: Vec<String> = std::env::args().collect();
    let want_light = args
        .windows(2)
        .any(|w| w[0] == "--theme" && w[1] == "light");

    if args.iter().any(|a| a == "--list-routes") {
        // 列出全部合法路由：拼错 id 时**立刻可见**，而不是静默回 home。
        let routes = zephyra_ui::app::list_routes();
        eprintln!("[gallery] {} 条路由：", routes.len());
        for r in &routes {
            eprintln!("  {r}");
        }
        return;
    }

    if args.iter().any(|a| a == "--bench") {
        bench_cpu(args.iter().any(|a| a == "--alloc-probe"), want_light);
        return;
    }

    if args.iter().any(|a| a == "--dump-cpu") {
        dump_cpu(want_light);
        return;
    }

    #[cfg(feature = "d3d12")]
    {
        let event_loop = winit::event_loop::EventLoop::new().expect("EventLoop::new");
        let theme = intent_cell(!want_light);
        let view = Gallery::with_theme_intent(theme.clone(), !want_light);
        let mut app = Bootstrap {
            inner: None,
            view: Some(view),
            theme,
        };
        if let Err(e) = event_loop.run_app(&mut app) {
            eprintln!("事件循环退出：{e}");
        }
    }
    #[cfg(not(feature = "d3d12"))]
    {
        eprintln!("[gallery] 需要 d3d12 特性才能开窗口；或用 --dump-cpu");
    }
}

/// 读入 crate 目录下全部 `.zstyle`，解析、按文件名排序合并。
///
/// 每个文件的规则数单独汇报 —— 三行 stderr 是「样式表加载了没有」的
/// 判据（与 demo 同一诊断语义，勿改）：
///
/// ```text
/// [gallery] ui.zstyle: N rules
/// [gallery] gallery.zstyle: M rules
/// [gallery] loaded N+M rules in total
/// ```
fn build_sheet() -> StyleSheet {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut paths: Vec<std::path::PathBuf> = match std::fs::read_dir(dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("zstyle"))
            .collect(),
        Err(e) => {
            eprintln!("[gallery] cannot list {}: {e}", dir.display());
            return StyleSheet::new();
        }
    };
    // 排序是必须的：目录枚举顺序不确定，而同特异性跨文件时「后写的赢」。
    paths.sort();

    let mut sheet = StyleSheet::new();
    for path in &paths {
        let css = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[gallery] cannot read {}: {e}", path.display());
                continue;
            }
        };
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("style.zstyle")
            .to_string();
        let result = zstylec::parse(&name, &css);
        if !result.is_ok() {
            for e in &result.errors {
                eprintln!("{e}\n");
            }
            eprintln!(
                "[gallery] {name} has {} error(s) -- skipping it, the rest still load",
                result.errors.len()
            );
            continue;
        }
        eprintln!("[gallery] {name}: {} rules", result.sheet.rules().len());
        sheet = sheet.merge(result.sheet);
    }
    eprintln!("[gallery] loaded {} rules in total", sheet.rules().len());
    sheet
}

/// 把整份样式表装成宿主的样式注入点。
fn install(sheet: StyleSheet) -> Box<dyn Styler> {
    Box::new(SheetStyler(std::sync::Arc::new(sheet)))
}

/// 分阶段计时 + 分配探针 —— 逐字照抄 examples/demo.rs::bench_cpu。
///
/// bench 的价值全在**可比**：两处独立实现的时间数字不能对比。改动时两处同步。
fn bench_cpu(probe: bool, light: bool) {
    /// 参与统计的帧数。
    const FRAMES: usize = 120;
    /// 丢弃的开头帧数（字体解析、图集首次打包、树首次构建都在这几帧里）。
    const WARMUP: usize = 12;

    let (w, h) = (1400u32, 800u32);
    let mut host = zephyra_fx::cpu_only::CpuHost::new(Gallery::default(), (w, h));
    // 装样式表：否则量的是无规则树，不是本应用。
    host.set_styler(install(build_sheet()));
    if light {
        host.set_theme(ThemeVariant::Light);
    }

    // 探针只数分配，不改耗时结论 —— 它的原子操作会把绝对耗时抬高。
    let mut before = alloc_probe::Counts::now();
    // 直方图只在采样窗口里累加：`reset` 放在 `WARMUP` 之后的头一帧之前，
    // 这样它量的是**稳态**帧的分配构成，不掺首次构建的那些。
    let mut histogram_ready = false;

    let mut samples: Vec<zephyra_fx::cpu_only::StageTimes> = Vec::with_capacity(FRAMES);
    let mut alloc_samples: Vec<(u64, u64)> = Vec::new();
    let mut last = None;
    for i in 0..FRAMES {
        if probe {
            if i == WARMUP {
                alloc_probe::reset_histogram();
                histogram_ready = true;
            }
            before = alloc_probe::Counts::now();
        }
        let (frame, t) = host.frame_timed();
        if probe && i >= WARMUP {
            let d = alloc_probe::Counts::now().since(&before);
            alloc_samples.push((d.allocs, d.bytes));
        }
        last = Some(frame);
        if i >= WARMUP {
            samples.push(t);
        }
    }

    if probe && !alloc_samples.is_empty() {
        let mut counts: Vec<u64> = alloc_samples.iter().map(|(a, _)| *a).collect();
        let mut sizes: Vec<u64> = alloc_samples.iter().map(|(_, b)| *b).collect();
        counts.sort_unstable();
        sizes.sort_unstable();
        println!(
            "[bench] 每帧分配：中位 {} 次 / {} 字节，最大 {} 次 / {} 字节（全局最大单次 {}）",
            counts[counts.len() / 2],
            sizes[sizes.len() / 2],
            counts[counts.len() - 1],
            sizes[sizes.len() - 1],
            alloc_probe::Counts::now().max_one,
        );
        if histogram_ready {
            // 只印构成里权重最大的几桶：全印出来会把"18 次 × 32 字节"这种噪声
            // 与"3 次 × 30KB"这种真问题混在一起，看不出哪个该修。
            let mut h = alloc_probe::histogram();
            h.sort_by_key(|&(_, _, bytes)| std::cmp::Reverse(bytes));
            let per_frame = (FRAMES - WARMUP) as f64;
            println!("[bench] 分配大小分布（按每帧字节降序，次数已除以采样帧数）：");
            for (hi, n, bytes) in h.iter().take(10) {
                println!(
                    "[bench]   ≤ {hi:>6} 字节：每帧 {:>7.1} 次  {:>9.1} 字节",
                    *n as f64 / per_frame,
                    *bytes as f64 / per_frame,
                );
            }
        }
    }

    // 中位数而不是平均值：单帧会被 OS 抢占 / 页错误污染，而平均值会把那几毫秒
    // 摊薄到看不出来。中位数只反映**典型**帧 —— 那才是决定手感的东西。
    fn median(xs: &mut [u32]) -> u32 {
        xs.sort_unstable();
        xs[xs.len() / 2]
    }
    fn report(name: &str, mut xs: Vec<u32>, total: u64) {
        xs.sort_unstable();
        let med = median(&mut xs.clone());
        let max = *xs.last().unwrap_or(&0);
        let pct = if total == 0 {
            0.0
        } else {
            med as f64 / total as f64 * 100.0
        };
        println!(
            "[bench] {name:<8} 中位 {med:>6} µs  最大 {max:>6} µs  占比 {pct:>5.1}%"
        );
    }

    let frame = last.expect("FRAMES > 0");
    println!(
        "[bench] {} 帧（丢弃开头 {} 帧），{}×{}；四边形 {}、顶点 {}",
        FRAMES,
        WARMUP,
        w,
        h,
        frame.render.quads,
        host.batch().solid.len(),
    );
    // 各阶段的占比之和是判据：它应当约等于 100%。明显小于 100% 说明**有阶段没被
    // 计时**（计时点之间漏了代码），那时"占比"这列数字会误导 —— 所以直接印出来。
    let sum: u64 = samples
        .iter()
        .map(|s| s.view_us as u64 + s.diff_us as u64 + s.layout_us as u64 + s.paint_us as u64 + s.expand_us as u64)
        .sum();
    let sum_total: u64 = samples.iter().map(|s| s.total_us as u64).sum();
    let accounted = if sum_total == 0 {
        0.0
    } else {
        sum as f64 / sum_total as f64 * 100.0
    };
    report(
        "合计",
        samples.iter().map(|s| s.total_us).collect(),
        sum_total,
    );
    report(
        "view",
        samples.iter().map(|s| s.view_us).collect(),
        sum,
    );
    report(
        "diff",
        samples.iter().map(|s| s.diff_us).collect(),
        sum,
    );
    report(
        "layout",
        samples.iter().map(|s| s.layout_us).collect(),
        sum,
    );
    report(
        "paint",
        samples.iter().map(|s| s.paint_us).collect(),
        sum,
    );
    report(
        "expand",
        samples.iter().map(|s| s.expand_us).collect(),
        sum,
    );
    println!(
        "[bench] 各阶段之和 / 合计 = {accounted:.1}%（应当 ≈100%；明显小于 100 说明有代码没被计时）"
    );
}
/// 无窗口像素自验：CPU 上画第一帧，写 BMP + 打统计。
///
/// 为什么要看统计而不是只看图：图只能看"像不像"，统计能看"链路通不通" ——
/// 四边形数、字形数、被跳过的纹理数，每一项都对应一段可能静默失败的代码。
/// 取 `--flag value` 形式的值；没有该 flag 时 `None`。
///
/// # 为什么不用 `skip_while`（`--route` 现在就是这么取的）
///
/// `skip_while(|a| a != "--route").nth(1)` 在**没有**该 flag 时会返回
/// 最后一个参数 —— 于是 `--lang` 不存在时拿到的是别的 flag 的值，
/// 而那正好是一个合法的语言标签时会静默生效。这个助手用 `position` 定位，
/// 找不到就是 `None`，没有这种歧义。
fn flag_value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    let i = args.iter().position(|a| a == flag)?;
    args.get(i + 1).map(|s| s.as_str())
}

fn dump_cpu(light: bool) {
    let args: Vec<String> = std::env::args().collect();
    // `--route <id>`：dump 指定页（默认 home）。逐页像素自验用 ——
    // 否则只有第一页能被看到，后面几页的样式错误要等窗口路才暴露。
    let route = flag_value(&args, "--route")
        .map(|s| s.to_string())
        .unwrap_or_else(|| "home".to_string());

    // 路由解析走 `resolve_route`（应用层唯一真源），**不做白名单 match**。
    //
    // ⚠️ 之前这里是 10 个字面量的 `match`，其余一律 `_ => "home"`。于是
    // `--route Button` / `--route Flyout` 全部静默回到 home 页，出图 20 张
    // md5 全同 —— 而"逐路由出图"看起来是跑过了。是我自己写下的教训
    // （MIGRATION §8：必须核对产物互不相同）当场复发。
    //
    // 未知路由现在**明确报错并退出**，不再静默回 home：出图工具静默给错图，
    // 比直接失败恶劣得多。
    let Some(resolved) = resolve_route(&route) else {
        eprintln!("[gallery] 未知路由 {route:?}；用 --list-routes 看全部合法值");
        std::process::exit(2);
    };
    let mut view = Gallery::default();
    view.route = resolved;
// --lang <tag>：强制语言，供**像素自验**用。
    //
    // 为什么需要它：dump 路不读系统语言（Gallery::default 的 prefs 是空的，
    // 于是走回落分支），所以中日文只能这样才看得到。而中日文能不能真正出图，
    // 正是 i18n 唯一无法用纯 CPU 断言覆盖的部分 —— 字形在宿主的字形图集里
    // （见 docs/i18n.md §7）。给一个开关，比让下一个人以为"中日文没接"要好。
    if let Some(tag) = flag_value(&args, "--lang") {
        match SUPPORTED.iter().position(|s| *s == tag) {
            Some(idx) => view.lang = idx,
            None => {
                // 未知语言**报错退出**，不静默回落：`--lang zh_Hant`（下划线）
                // 这类拼错会给出英文界面，而那看起来像"功能没接上"。
                eprintln!(
                    "[gallery] 未知语言 {tag:?}；可用：{}",
                    SUPPORTED.join(" ")
                );
                std::process::exit(2);
            }
        }
    }
    let (w, h) = (1400u32, 800u32);
    let mut host = zephyra_fx::cpu_only::CpuHost::new(view, (w, h));
    if light {
        host.set_theme(ThemeVariant::Light);
    }
    // 样式表必须装上：否则 dump 图是「属性默认值」的样子，与窗口画面不同 ——
    // 那会让这条自验从参考变成误导（两条路吃同一份表才互为参考）。
    host.set_styler(install(build_sheet()));
    println!("[dump] 路由：{route}");

    let frame = host.frame();
    let r = frame.render;
    let batch = host.batch();
    let glyph_vertices = batch
        .solid
        .chunks_exact(4)
        .filter(|c| (c[0].kind - zephyra_fx::quad::kind::GLYPH).abs() < 0.01)
        .count();
    println!(
        "[dump] DisplayList → 顶点：四边形 {} 个、顶点 {} 个（其中字形 {} 个）、跳过 {} 个、缺失纹理 {} 个",
        r.quads,
        batch.solid.len(),
        glyph_vertices,
        r.skipped,
        r.missing_textures
    );
    let mut path_cmds = 0usize;
    let mut path_tris = 0usize;
    for item in host.list().iter() {
        if let zephyra_core::paint::Command::Path { vertices, .. } = &item.command {
            path_cmds += 1;
            path_tris += vertices.len() / 3;
        }
    }
    println!(
        "[dump] 矢量图标：Command::Path {path_cmds} 条、三角形 {path_tris} 个{}",
        if path_tris == 0 {
            "  ← ⚠ 一个都没有：图标名没查到，或展平/三角化返回空"
        } else {
            ""
        }
    );
    let a = host.atlas_stats();
    println!(
        "[dump] 字形图集：已缓存 {} 个字形，用量 {:.1}%",
        a.glyphs,
        a.usage() * 100.0
    );

    let clear = zephyra_core::core::Color::rgba(0.0, 0.0, 0.0, 1.0);
    let canvas = host.render_cpu(clear);
    // 底色 = 出现最多的颜色（必然是根容器铺上去的 surface），与 demo 同法。
    let mut counts: std::collections::HashMap<[u8; 3], usize> = Default::default();
    for p in canvas.pixels.chunks_exact(4) {
        *counts.entry([p[0], p[1], p[2]]).or_default() += 1;
    }
    let Some((&bg, _)) = counts.iter().max_by_key(|(_, n)| **n) else {
        eprintln!("[dump] 画布是空的");
        return;
    };
    let lit = canvas
        .pixels
        .chunks_exact(4)
        .filter(|p| [p[0], p[1], p[2]] != bg)
        .count();
    let total = (w * h) as usize;
    println!(
        "[dump] CPU 光栅：底色 {:?}，内容像素 {lit}/{total}（{:.1}%），不同颜色 {} 种",
        bg,
        lit as f64 / total as f64 * 100.0,
        counts.len()
    );

    let path = std::path::Path::new("gallery-cpu.bmp");
    match host.dump_cpu_bmp(&path.display().to_string(), clear) {
        Ok(()) => {
            let abs = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
            println!("[dump] 已写出 {}", abs.display());
        }
        Err(e) => eprintln!("[dump] 写 {} 失败：{e}", path.display()),
    }
}

/// demo 自带的 MiSans 字体（含 CJK）。不放副本，直接内嵌仓库里那一份 ——
/// 7.7MB 进两次二进制没有意义。
const DEMO_FONT: &[u8] = include_bytes!("../../examples/fonts/MiSans-Regular.ttf");

/// 让 `zephyra_fx` 选中自带字体（与 `examples/demo.rs` 同手法）。
fn use_demo_font() {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in DEMO_FONT {
        h ^= *b as u64;
        h = h.wrapping_mul(0x1000_0000_01b3);
    }
    let path = std::env::temp_dir().join(format!("zephyra-demo-font-{h:016x}.ttf"));
    if !path.exists() {
        if let Err(e) = std::fs::write(&path, DEMO_FONT) {
            eprintln!("[gallery] 落地字体失败（{e}），退回系统字体 —— 中文会是豆腐块");
            return;
        }
    }
    std::env::set_var(
        zephyra_fx::FONT_OVERRIDE_ENV,
        path.to_string_lossy().as_ref(),
    );
}

/// 自举宿主：第一次 `resumed` 时建窗、装样式表、补第一帧。
#[cfg(feature = "d3d12")]
struct Bootstrap {
    inner: Option<zephyra_winit_d3d12::HostWindow<Gallery>>,
    view: Option<Gallery>,
    /// 主题意图共享格（View 写、这里读）。
    theme: Arc<AtomicU8>,
}

#[cfg(feature = "d3d12")]
impl winit::application::ApplicationHandler for Bootstrap {
    fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        if self.inner.is_some() {
            return;
        }
        let view = self.view.take().expect("resumed 只应发生一次建窗");
        // ⚠️ 必须按**画布**尺寸建窗：`gallery.zstyle` 的 `Container.window` 是
        // `width: 1400px / height: 800px`（core 没有 flex-grow，铺满靠定值），
        // 而宿主默认给 640×480 —— 窗口比画布小的时候，超出部分只是**被裁掉**：
        // 布局对、绘制对、屏幕上一片内容不见了，看起来像排版坏了。
        let (w, h) = (zephyra_ui::app::shell::CANVAS_W as u32, zephyra_ui::app::shell::CANVAS_H as u32);
        let mut app =
            zephyra_winit_d3d12::HostWindow::new_lenient_sized(event_loop, view, (w, h));

        // 样式表是第一优先级证明物：「源码里没有样式」要能看到它接上了。
        app.set_styler(install(build_sheet()));

        // 窗口外观：应用只说「我要暗色 + 大圆角 + Mica」，能不能做到由平台层回答。
        //
        // ⚠️ 这里**没有一行 Win32 代码** —— 暗色/圆角/Mica 走 DWM，判定走
        // `zephyra-platform-win`，句柄转换在宿主里。Linux 上这段原样编译，
        // 只是平台层返回「不支持」，界面自动退回实色。
        //
        // 颜色留空（`None`）＝交给系统：本仓自绘标题栏，系统配色只在
        // 「深色模式」这一位上参与，其余由 `gallery.zstyle` 管。
        //
        // Mica：材质画在**窗口背后**，而我们自绘整窗（`Container.window` 有
        // 不透明背景）——所以必须同时让根背景半透明（`gallery.zstyle` 的
        // `Container.window.backdrop-on` 分类名，由 Gallery 的状态决定）才透得出
        // 材质。两者缺一不可，见 `docs/win/02-mica.md` §3。
        let _ = app.apply_window_chrome(zephyra_platform::WindowChrome {
            dark: true,
            corner: zephyra_platform::CornerPreference::Round,
            caption_color: None,
            border_color: None,
            backdrop: zephyra_platform::BackdropKind::Mica,
        });

        // 首帧必须在这里补：交换链建好了、DComp 提交了，但从未 present 过的话，
        // 屏幕上是一个还没被写过的白色合成交换链 —— 唯一症状是「窗口全白」，
        // 极易被当成渲染问题查。（同 examples/demo.rs 的注释。）
        app.sync_input();
        app.frame();
        app.present();

        self.inner = Some(app);
    }

    fn about_to_wait(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        if let Some(app) = self.inner.as_mut() {
            // 主题意图对账：View 点了切换，这里才真的换。
            let want_light = self.theme.load(Ordering::Relaxed) == 1;
            let is_dark = app.theme_variant() == ThemeVariant::Dark;
            if want_light == is_dark {
                app.set_theme(if want_light {
                    ThemeVariant::Light
                } else {
                    ThemeVariant::Dark
                });
            }
            app.about_to_wait(event_loop);
        }
    }

    fn window_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        id: winit::window::WindowId,
        event: winit::event::WindowEvent,
    ) {
        match self.inner.as_mut() {
            Some(app) => app.window_event(event_loop, id, event),
            None => {
                let _ = event;
            }
        }
    }
}
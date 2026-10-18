//! 入场动画的接线 —— 从时钟到分类名。
//!
//! # 这一组测的是什么
//!
//! core 侧的不透明度与偏移由 `zephyra-core/tests/paint_transform_tests.rs`
//! 负责。本文件测**另一半**：应用层是否真的在推进时钟、真的在换档、
//! 真的把外观**完全**交给样式表。
//!
//! 1. 时钟按 `dt_ms` 推进，**不**按帧数（一帧 1 ms 会在 144 Hz 上快 2.4 倍）；
//! 2. 换路由 ⇒ 归零 ⇒ 重放入场；
//! 3. 档位从"没有分类名"一路走到 `enter-8`，放完之后**回到没有分类名**；
//! 4. 8 条规则都在样式表里，且**最后一档等于无动画时的外观**
//!    —— 否则动画结束后画面会停在半透明。
//!
//! # 为什么第 4 条要看"最后一档等于不透明"
//!
//! `.enter-8` 是 `opacity: 0.91` 而不是 1.0（见 `shell.zstyle` 的注释：
//! 8 档量化下每档 1/8，最后一档落在 91%）。这本身是个取舍：如果 `.enter-8`
//! 写成 1.0，那么从 `enter-7`(0.78) 到 `enter-8`(1.0) 会有一次可见跳变；
//! 写成 0.91 再由"放完之后不发分类名"补上最后一跳，跳变只有 9%，
//! 看不出来。**代价是必须在动画结束时撤掉分类名** —— 那就是本文件第 3 条。

use zephyra_fx::{InputState, View};

use zephyra_ui::anim;
use zephyra_ui::app::Gallery;

/// 一帧的间隔（ms）。取 16.0 而不是真实帧时间：断言要可复现。
const DT: f32 = 16.0;

/// 推进 `n` 帧，返回每帧的入场分类名。
///
/// 用**空输入**推进：不按任何键、指针不在任何控件上。这样这条测试只测动画，
/// 不掺交互状态（`interaction_tests.rs` 负责那部分）。
fn run_frames(g: &mut Gallery, n: u32) -> Vec<Option<&'static str>> {
    let mut out = Vec::new();
    for _ in 0..n {
        g.on_input(&InputState {
            hovered: None,
            pressed: None,
            pointers: 0,
            viewport: (1400.0, 800.0),
            scale: 1.0,
            dt_ms: DT,
            // 空表 = "读不到"，于是语言固定为 `SUPPORTED` 第一项，
            // 断言与宿主机器的 Windows 语言设置无关。
            languages: Vec::new(),
        });
        out.push(g.enter_class());
    }
    out
}

/// 按给定的**每帧间隔**跑 `n` 帧。
///
/// 与 [`run_frames`] 分开是因为"按固定 16 ms"和"按固定 8.3 ms"要分别读两遍
/// —— 合并成一个带参数的函数会让 [`run_frames`] 的调用点多出一个不必需的实参。
fn run_frames_dt(g: &mut Gallery, dt: f32, n: u32) {
    for _ in 0..n {
        g.on_input(&InputState {
            hovered: None,
            pressed: None,
            pointers: 0,
            viewport: (1400.0, 800.0),
            scale: 1.0,
            dt_ms: dt,
            languages: Vec::new(),
        });
    }
}

/// 时钟按**毫秒**推进，不是按帧。
///
/// 判据是"跑够 333 ms 之后放完"：若实现按"每帧 +1 档"写死，那么
/// `333/16 ≈ 20` 帧就会放完，而按真实时间的实现要 21 帧。差一帧不多，
/// 所以这里用更狠的判据：**同样的总时长，分成两倍多的帧数跑，放完的时刻不变**。
#[test]
fn clock_advances_by_milliseconds_not_frames() {
    let mut g = Gallery::default();
    g.route = "settings";
    // 先跑一帧让"路由变了"被吃掉。
    run_frames(&mut g, 1);
    g.enter_ms = 0.0;

    // 333 ms 分成 40 帧跑（每帧 8.3 ms）。
    run_frames_dt(&mut g, anim::ENTER_MS / 40.0, 40);
    assert!(
        (g.enter_ms - anim::ENTER_MS).abs() < 0.5,
        "40 × 8.3ms 应恰好走完 333ms，实际 {}",
        g.enter_ms
    );
}

/// 换路由 ⇒ 时钟归零并重放入场。
#[test]
fn route_change_replays_entrance() {
    let mut g = Gallery::default();
    // 先在 home 上把动画跑完。
    run_frames(&mut g, 40);
    assert_eq!(g.enter_ms, anim::ENTER_MS, "前置条件：home 已放完");

    g.route = "settings";
    let classes = run_frames(&mut g, 3);
    assert!(
        classes.iter().any(|c| c.is_some()),
        "换路由后必须重新发档位分类名，实际 {classes:?}"
    );
}

/// 档位一路走到 `enter-8`，放完之后**回到 `None`**。
#[test]
fn no_class_name_after_tier_exhausted() {
    let mut g = Gallery::default();
    g.route = "settings";
    // 先吃掉"路由变了"那一帧。
    let first = run_frames(&mut g, 1);
    assert_eq!(
        first[0], None,
        "进度 0 时不该发分类名（`anim::step(0) == None`）"
    );

    // 一路跑到放完。
    let mut seen: Vec<Option<&'static str>> = Vec::new();
    let mut last = None;
    for _ in 0..60 {
        let c = g.enter_class();
        if c != last {
            seen.push(c);
            last = c;
        }
        g.enter_ms = (g.enter_ms + DT).min(anim::ENTER_MS);
    }
    assert_eq!(
        last, None,
        "放完之后必须撤掉分类名（否则页面停在 `.enter-8` 的 0.91 不透明度上）：{seen:?}"
    );
    // 中间应当至少出现过 `enter-1` 与 `enter-8`。
    assert!(seen.contains(&Some("enter-1")), "应出现 enter-1：{seen:?}");
    assert!(seen.contains(&Some("enter-8")), "应出现 enter-8：{seen:?}");
}

/// 8 档一个都不能少，且档位单调不减。
///
/// 少一条的后果是"最高档永不命中，页面永远半透明" —— 那是个看起来像
/// "动画卡住了"的缺陷。
#[test]
fn all_eight_tiers_in_sheet_and_monotonic() {
    let mut g = Gallery::default();
    g.route = "settings";
    let _ = run_frames(&mut g, 1);

    let mut last_alpha = -1.0f32;
    let mut seen: Vec<&str> = Vec::new();
    let mut t = 0.0f32;
    while t < anim::ENTER_MS {
        g.enter_ms = t;
        // 只在**换档**时判单调：每毫秒采一次，同一档会被看到十几次，
        // 逐次判就会拿 `enter-1` 跟 `enter-1` 比。
        if let Some(c) = g.enter_class() {
            if seen.last().copied() != Some(c) {
                let alpha = opacity_of(c);
                assert!(
                    alpha > last_alpha,
                    "{c} 的不透明度 {alpha} 没有比上一档 {last_alpha} 大 —— 动画在倒退"
                );
                last_alpha = alpha;
                seen.push(c);
            }
        }
        t += 1.0;
    }
    assert_eq!(
        seen.len(),
        anim::STEPS as usize,
        "应当出现 {} 个档位，实际 {seen:?}",
        anim::STEPS
    );
    assert_eq!(seen.first(), Some(&"enter-1"));
    assert_eq!(seen.last(), Some(&"enter-8"));
}

/// 静止（放完之后）持续跑帧不会一直重画。
///
/// 这条接的是 `frame.rs::tree_needs_layout` 那笔账：入场动画结束时若还留着
/// 一个分类名，每帧 `apply` 都会重写 `opacity`/`translate-y`（值相同、
/// 所以不标脏）—— 但如果实现里出现"每帧 +dt 不夹上限"那种写法，
/// 就会变成永久动画。这条钉住"放完即静止"。
#[test]
fn clock_stops_after_layout() {
    let mut g = Gallery::default();
    g.route = "settings";
    run_frames(&mut g, 60);
    assert_eq!(g.enter_ms, anim::ENTER_MS, "时钟必须夹在时长上");
    let before = g.enter_class();
    let frames = run_frames(&mut g, 10);
    assert!(
        frames.iter().all(|c| *c == before),
        "放完之后不该再有档位变化：{frames:?}"
    );
}

/// `dt_ms` 为 0（首帧）时不能抛异常，也不能发分类名。
///
/// 宿主首帧填 0（`InputState::dt_ms` 的文档说明了），而首帧恰好也是
/// "刚建树"那一帧 —— 这里出问题的症状是整个界面起不来。
#[test]
fn first_frame_zero_interval_is_safe_and_emits_no_class() {
    let mut g = Gallery::default();
    g.route = "settings";
    g.on_input(&InputState {
        hovered: None,
        pressed: None,
        pointers: 0,
        viewport: (1400.0, 800.0),
        scale: 1.0,
        dt_ms: 0.0,
        languages: Vec::new(),
    });
    assert_eq!(g.enter_ms, 0.0);
    assert_eq!(g.enter_class(), None);
}

/// `dt_ms` 极大（断点调试 / 挂起后恢复）时也必须夹住。
///
/// 不夹的话 `enter_ms` 一步跳到 10 万，然后 `f32` 精度丢到小数点后第 7 位 ——
/// 症状是"动画结束之后又重放了一次"，而且极难归因。
#[test]
fn huge_interval_is_clamped() {
    let mut g = Gallery::default();
    g.route = "settings";
    // 第一帧吃掉"路由变了"（归零，不累加本帧的 dt）—— 这是刻意的：
    // 点导航那一帧的时间不该算进**新**页面的入场动画里。
    g.on_input(&InputState {
        hovered: None,
        pressed: None,
        pointers: 0,
        viewport: (1400.0, 800.0),
        scale: 1.0,
        dt_ms: 100_000.0,
        languages: Vec::new(),
    });
    assert_eq!(
        g.enter_ms, 0.0,
        "换路由那一帧只归零，不把这一帧的 dt 算进新动画"
    );

    // 第二帧才是"动画已在跑"，这时极大间隔必须被夹住。
    g.on_input(&InputState {
        hovered: None,
        pressed: None,
        pointers: 0,
        viewport: (1400.0, 800.0),
        scale: 1.0,
        dt_ms: 100_000.0,
        languages: Vec::new(),
    });
    assert_eq!(g.enter_ms, anim::ENTER_MS);
    assert_eq!(g.enter_class(), None, "放完即无分类名");
}

// ---------------------------------------------------------------- 帮手

/// 从样式表读某个档位的 `opacity`。
///
/// # 为什么真的去读 zstyle 而不是把表抄一份到测试里
///
/// 抄一份就变成**两处真相**：改了样式表里的 0.91 而测试里还写 0.91，
/// 测试照样绿，而画面已经错了。这里读真表。
fn opacity_of(class: &str) -> f32 {
    use std::path::Path;
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let css = std::fs::read_to_string(dir.join("shell.zstyle")).expect("读 shell.zstyle");
    let needle = format!("Container.page.{class} {{");
    let start = css
        .find(&needle)
        .unwrap_or_else(|| panic!("样式表里没有 {needle} —— 档位与 `anim::STEPS` 不同数？"));
    let body = &css[start + needle.len()..];
    let end = body.find('}').expect("规则未闭合");
    let decl = body[..end]
        .split(';')
        .find_map(|d| {
            let d = d.trim();
            d.strip_prefix("opacity:").map(|v| v.trim())
        })
        .unwrap_or_else(|| panic!("{class} 规则里没有 opacity"));
    // 单行写法 `opacity: 0.13; translate-y: 7px;` 在这里按 `;` 切开，
    // 所以拿到的就是纯数值那一段。`parse` 失败说明样式表里写了非数字 ——
    // 那正是这条门禁要抓的（漏写单位、写成 `0.13px`、或者写成了颜色）。
    decl.parse::<f32>()
        .unwrap_or_else(|e| panic!("opacity 解析失败：{e}（读到 {decl:?}）"))
}

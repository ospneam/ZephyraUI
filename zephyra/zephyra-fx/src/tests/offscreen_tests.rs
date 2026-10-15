// 本文件由 offscreen.rs 的 #[cfg(test)] mod tests 抽出 —— 测试代码与生产代码分离。
// 模块路径不变（src 里的声明用 #[path] 指过来），所以 use super::* 与私有可见性都照旧。

use super::*;
use zephyra_core::core::{Color, CornerRadius, Point};
use zephyra_core::filter::{BlendMode, Filter};
use zephyra_core::paint::{Layer, SortKey};

// ── E4.4 / E4.5 / E4.6：执行计划 ──

fn blur_chain(sx: f32, sy: f32) -> FilterChain {
    FilterChain::single(Filter::Blur {
        sigma_x: sx,
        sigma_y: sy,
    })
}

/// 水平趟必须在垂直趟**之前**，且 σ 互不串位。
///
/// # 变异目标
///
/// 把 `push_blur` 的两行顺序对调（或把 `sigma_x`/`sigma_y` 写反）时本条
/// 必须失败。否则症状只是"某元素的模糊宽窄方向反了"，在 demo 上很难看出。
#[test]
fn blur_becomes_two_steps_horizontal_first() {
    let p = pass_for_chain(&blur_chain(2.0, 3.0), Rect::new(10.0, 20.0, 30.0, 40.0));
    assert_eq!(
        p.blurs,
        vec![
            BlurStep {
                horizontal: true,
                sigma: 2.0
            },
            BlurStep {
                horizontal: false,
                sigma: 3.0
            },
        ],
        "σx / σy 是分别给的，顺序或串位都等于换了滤镜"
    );
}

#[test]
fn zero_sigma_steps_are_dropped() {
    let p = pass_for_chain(&blur_chain(0.0, 0.0), Rect::new(0.0, 0.0, 10.0, 10.0));
    assert!(
        p.blurs.is_empty(),
        "σ=0 的趟是恒等，不该付两次全 RT 的 draw"
    );
    // 而恒等链整体压根不该开离屏。
    let id = quad(
        blur_chain(0.0, 0.0),
        BlendMode::SrcOver,
        Rect::new(0.0, 0.0, 10.0, 10.0),
    );
    assert!(plan_pass(&id).is_none());
}

/// 投影 = 染色 + 偏移 + 额外重画原图。
///
/// `translate` 是这条测试的判据核心：平移必须在**模糊之前**，否则影子某一侧
/// 会凭空少一圈模糊（而不是整体位置偏一点）。
#[test]
fn drop_shadow_tints_redraws_and_shifts_inside_the_offscreen() {
    let c = Rect::new(100.0, 100.0, 40.0, 40.0);
    let chain = FilterChain::single(Filter::DropShadow {
        color: Color::rgba(0.0, 0.0, 0.0, 0.5),
        offset: Point::new(6.0, 8.0),
        sigma: 2.0,
    });
    let p = pass_for_chain(&chain, c);
    assert!(p.redraw_original, "影子是额外一层，原图仍要画在它上面");
    assert_eq!(p.tint, Some(Color::rgba(0.0, 0.0, 0.0, 0.5)));
    assert_eq!(p.blurs.len(), 2);
    // outset.left = max(3σ - offset.x, 0) = max(6-6, 0) = 0 ⇒ origin.x = 100。
    assert_eq!(
        p.origin,
        (100.0, 100.0),
        "origin 决定平移量，算错等于整体错位"
    );
    assert_eq!(
        p.translate,
        (-94.0, -92.0),
        "translate = -origin + offset；漏掉 offset 会让影子画在原位"
    );
}

/// 链里三道效果 ⇒ 六趟，且顺序与 σ 序列**逐项**对上。
///
/// # 变异目标
///
/// 任何对 `blurs` 排序、去重、或截断到前两项的实现都会在这里露馅。
#[test]
fn a_three_effect_chain_is_six_steps_in_chain_order() {
    let chain = FilterChain::new([
        Filter::Blur {
            sigma_x: 1.0,
            sigma_y: 1.0,
        },
        Filter::Blur {
            sigma_x: 2.0,
            sigma_y: 2.0,
        },
        Filter::DropShadow {
            color: Color::rgba(0.0, 0.0, 0.0, 1.0),
            offset: Point::new(0.0, 0.0),
            sigma: 1.0,
        },
    ]);
    let p = pass_for_chain(&chain, Rect::new(0.0, 0.0, 8.0, 8.0));
    assert_eq!(p.blurs.len(), 6, "三道效果各两趟");
    assert_eq!(
        p.blurs.iter().map(|b| b.sigma).collect::<Vec<_>>(),
        vec![1.0, 1.0, 2.0, 2.0, 1.0, 1.0],
        "趟的 σ 必须按 filters[0] 起的顺序，不能被排序或去重"
    );
    // 三道里最后一道是投影，所以要重画原图。
    assert!(p.redraw_original);
}

/// RT 尺寸向上取整，且永不塌成 0。
///
/// # 变异目标
///
/// 把 `.ceil()` 改成 `.floor()`（或 `.round()`）时本条必须失败 —— 症状是模糊
/// 在右/下边缘少掉不足一个像素的一条，而那只在 σ 大到肉眼可见时才显形。
#[test]
fn rt_size_rounds_up_and_never_collapses_to_zero() {
    // σ=1 ⇒ outset = 3。内容 0.2×0.2 ⇒ bounds 6.2×6.2 ⇒ 7×7（不是 6×6）。
    let frac = pass_for_chain(&blur_chain(1.0, 1.0), Rect::new(0.0, 0.0, 0.2, 0.2));
    assert_eq!(
        frac.rt_size,
        (7, 7),
        "bounds 有小数时必须进位，否则切掉一条边"
    );
    // 整数边界不该被多进一位（否则白白多分配一整行/一整列显存）。
    let exact = pass_for_chain(&blur_chain(1.0, 1.0), Rect::new(0.0, 0.0, 10.0, 20.0));
    assert_eq!(exact.rt_size, (16, 26), "bounds 已是整数时不该多进");
    let tiny = pass_for_chain(&blur_chain(1.0, 1.0), Rect::new(0.0, 0.0, 0.001, 0.001));
    assert!(
        tiny.rt_size.0 >= 1 && tiny.rt_size.1 >= 1,
        "0×0 的视口什么都不画且不报错"
    );
}

/// 非有限矩形必须被挡住 —— 否则它会穿过 `ceil` 变成天文数字的纹理尺寸。
///
/// # 变异目标
///
/// 删掉 `plan_pass` 里的 `is_finite` 检查时本条必须失败。
#[test]
fn a_non_finite_rect_yields_no_plan() {
    for bad in [
        Rect::new(f32::NAN, 0.0, 10.0, 10.0),
        Rect::new(0.0, 0.0, f32::INFINITY, 10.0),
    ] {
        let cmd = quad(blur_chain(2.0, 2.0), BlendMode::SrcOver, bad);
        assert!(plan_pass(&cmd).is_none(), "{bad:?} 必须在这里就被挡住");
    }
}

/// 空内容矩形不���离屏 —— 模糊一张空图得到的是全透明，纯浪费两趟全 RT。
#[test]
fn an_empty_content_rect_yields_no_plan() {
    let cmd = quad(
        blur_chain(2.0, 2.0),
        BlendMode::SrcOver,
        Rect::new(5.0, 5.0, 0.0, 10.0),
    );
    assert!(plan_pass(&cmd).is_none());
}

/// pass 四边形的**采样坐标按源尺寸**归一化，几何铺满目标。
#[test]
fn pass_quad_samples_the_source_and_covers_the_destination() {
    let v = blur_quad(
        &BlurStep {
            horizontal: true,
            sigma: 2.0,
        },
        (100.0, 100.0), // 目标
        (40.0, 60.0),   // 源
    );
    assert_eq!(
        (v[0].position, v[2].position),
        ([-1.0, 1.0], [1.0, -1.0]),
        "几何必须铺满目标"
    );
    assert_eq!(v[2].local, [40.0, 60.0], "local 必须是源 RT 里的像素坐标");
    assert!(
        (v[2].uv[0] - 1.0).abs() < 1e-6 && (v[2].uv[1] - 1.0).abs() < 1e-6,
        "右下角的 uv 必须是 (1,1) —— 归一化按**源**尺寸"
    );
    for vert in &v {
        assert_eq!(
            vert.half_size,
            [40.0, 60.0],
            "`half_size` 在 HLSL 里是 nointerpolation：四顶点不一致就插出斜纹"
        );
        assert_eq!(vert.kind, crate::quad::kind::BLUR);
    }
    assert!((v[0].corner[1] - 2.0).abs() < 1e-6, "σ 走 corner.y");
}

/// 合成四边形落在**屏幕矩形**上，且染色标志与颜色都对。
#[test]
fn composite_quad_lands_on_the_screen_rect() {
    let dst = Rect::new(10.0, 20.0, 40.0, 30.0);
    let v = composite_quad(dst, (200.0, 200.0), (40.0, 30.0), None);
    // NDC 角点：(10,20) → 左上，(50,50) → 右下。
    assert!((v[0].position[0] - (10.0 / 200.0 * 2.0 - 1.0)).abs() < 1e-5);
    assert!((v[0].position[1] - (1.0 - 20.0 / 200.0 * 2.0)).abs() < 1e-5);
    assert!((v[2].position[0] - (50.0 / 200.0 * 2.0 - 1.0)).abs() < 1e-5);
    assert!((v[2].position[1] - (1.0 - 50.0 / 200.0 * 2.0)).abs() < 1e-5);
    assert_eq!(
        v[0].corner[1], 0.0,
        "无染色时标志必须是 0（NaN 会走进染色分支）"
    );
    // 采样尺寸仍是源 RT，与目标无关。
    assert_eq!(v[2].half_size, [40.0, 30.0]);

    let tinted = composite_quad(
        dst,
        (200.0, 200.0),
        (40.0, 30.0),
        Some(Color::rgba(0.0, 0.0, 0.0, 0.5)),
    );
    assert_eq!(tinted[0].corner[1], 0.5, "染色浓度即 tint.a");
    assert_eq!(
        tinted[0].color,
        [0.0, 0.0, 0.0, 0.5],
        "顶点色必须**非预乘** —— PSComposite 自己用 `t.a * i.color.a` 算覆盖率"
    );
}

fn quad(filter: FilterChain, blend: BlendMode, rect: Rect) -> Command {
    Command::Quad {
        rect,
        radius: CornerRadius { radius: 0.0 },
        fill: Color::rgb(1.0, 1.0, 1.0),
        stroke: None,
        filter,
        blend,
    }
}

fn blur(s: f32) -> FilterChain {
    FilterChain::single(Filter::Blur {
        sigma_x: s,
        sigma_y: s,
    })
}

fn path(filter: FilterChain, blend: BlendMode, pts: &[(f32, f32)]) -> Command {
    Command::Path {
        vertices: std::sync::Arc::from(
            pts.iter()
                .map(|&(x, y)| zephyra_core::paint::Vertex {
                    position: Point::new(x, y),
                    uv: Point::new(0.0, 0.0),
                })
                .collect::<Vec<_>>(),
        ),
        fill: Color::rgb(1.0, 1.0, 1.0),
        filter,
        blend,
    }
}

fn list_of(cmds: Vec<Command>) -> DisplayList {
    let mut l = DisplayList::new();
    for (i, c) in cmds.into_iter().enumerate() {
        l.push(
            SortKey {
                layer: Layer::Content,
                z: 0,
                order: i as u32,
            },
            c,
        );
    }
    l.sort();
    l
}

/// 无滤镜时不产生任何离屏需求 —— 这是最常见的路径，必须零开销。
#[test]
fn no_filter_creates_no_offscreen_requirement() {
    let l = list_of(vec![
        quad(
            FilterChain::none(),
            BlendMode::default(),
            Rect::new(0.0, 0.0, 10.0, 10.0),
        ),
        quad(
            blur(0.0),
            BlendMode::default(),
            Rect::new(0.0, 0.0, 10.0, 10.0),
        ),
    ]);
    assert!(
        plan_offscreens(&l).is_empty(),
        "空链与 σ=0 的blur 都恒等，不该开RT"
    );
}

/// blur 的 RT 尺寸 = 内容 + 两侧 3σ。
#[test]
fn blur_离屏矩形外扩三倍标准差() {
    let l = list_of(vec![quad(
        blur(2.0),
        BlendMode::default(),
        Rect::new(100.0, 50.0, 40.0, 20.0),
    )]);
    let plans = plan_offscreens(&l);
    assert_eq!(plans.len(), 1);
    assert_eq!(
        plans[0].bounds,
        Rect::new(94.0, 44.0, 52.0, 32.0),
        "上下左右各扩 3 × 2 = 6"
    );
    assert_eq!(plans[0].content, Rect::new(100.0, 50.0, 40.0, 20.0));
}

/// 局部原点必须等于「bounds 左上角」，否则几何会被画偏。
///
/// 这是最关键的一条：几何坐标是屏幕坐标，宿主靠 `origin` 才知道要把
/// 内容往哪儿挪。origin 算错的症状是「效果整体偏移了 3σ」，极像模糊
/// 参数不对，会把人引到完全错误的方向。
///
/// ⚠ 方向容易搞反：RT 向**外**扩，所以屏幕原点落在 RT 内的**负**坐标上。
/// 宿主该做的是 `screen − origin`，而不是 `screen + origin`。
#[test]
fn local_origin_is_offscreen_rect_top_left() {
    let l = list_of(vec![quad(
        blur(4.0),
        BlendMode::default(),
        Rect::new(37.0, 11.0, 20.0, 20.0),
    )]);
    let p = &plan_offscreens(&l)[0];
    assert_eq!((p.origin.x, p.origin.y), (p.bounds.x, p.bounds.y));
    assert_eq!((p.origin.x, p.origin.y), (25.0, -1.0), "12 = 3 × 4");
    // 内容左上角在 RT 内的局部坐标 = content − origin，正好是外扩量。
    assert_eq!(p.content.x - p.origin.x, 12.0);
    assert_eq!(p.content.y - p.origin.y, 12.0);
}

/// 相同矩形 + 相同滤镜 → 复用一张 RT。
///
/// 典型场景是同一张卡片的背景与文本共用一个投影：两块内容矩形不同，
/// 但**离屏矩形**可能相同（内容都在同一张卡片的投影范围内）。
#[test]
fn identical_offscreen_rects_are_merged() {
    // 两个不同矩形，但外扩后恰好同尺寸 → 不会被合并（矩形不等）。
    let a = quad(
        blur(2.0),
        BlendMode::default(),
        Rect::new(0.0, 0.0, 10.0, 10.0),
    );
    let b = quad(
        blur(2.0),
        BlendMode::default(),
        Rect::new(0.0, 0.0, 10.0, 10.0),
    );
    let l = list_of(vec![a, b]);
    assert_eq!(plan_offscreens(&l).len(), 1, "同矩形同滤镜应合并成一张 RT");
    // 不同矩形则各开一张。
    let l2 = list_of(vec![
        quad(
            blur(2.0),
            BlendMode::default(),
            Rect::new(0.0, 0.0, 10.0, 10.0),
        ),
        quad(
            blur(2.0),
            BlendMode::default(),
            Rect::new(20.0, 0.0, 10.0, 10.0),
        ),
    ]);
    assert_eq!(plan_offscreens(&l2).len(), 2);
}

/// 投影的不对称外扩被完整反映到 RT 尺寸上。
#[test]
fn shadow_offscreen_rect_includes_offset() {
    let f = FilterChain::single(Filter::DropShadow {
        color: Color::rgba(0.0, 0.0, 0.0, 0.4),
        offset: Point::new(4.0, 6.0),
        sigma: 2.0,
    });
    let l = list_of(vec![quad(
        f,
        BlendMode::default(),
        Rect::new(0.0, 0.0, 100.0, 50.0),
    )]);
    let p = &plan_offscreens(&l)[0];
    assert_eq!(p.bounds, Rect::new(-2.0, 0.0, 112.0, 62.0));
}

/// blend_mode_creates_no_offscreen_requirement —— 它改的是混合方程，不是像素数量。
#[test]
fn blend_mode_creates_no_offscreen_requirement() {
    let l = list_of(vec![quad(
        FilterChain::none(),
        BlendMode::Multiply,
        Rect::new(0.0, 0.0, 10.0, 10.0),
    )]);
    assert!(plan_offscreens(&l).is_empty());
    assert_eq!(count_non_default_blends(&l), 1);
}

/// 文本命令的离屏范围是**所有字形的并集**，不是第一个字形。
///
/// 逐字形模糊会把每个字糊成独立一块，视觉上完全不是"这段文字被模糊了"。
#[test]
fn text_offscreen_bounds_is_union_of_all_glyphs() {
    let g = |x: f32| zephyra_core::paint::GlyphQuad {
        dst: Rect::new(x, 10.0, 8.0, 12.0),
        uv: zephyra_core::core::UvRect::FULL,
        color_glyph: false,
    };
    let cmd = Command::Text {
        atlas: zephyra_core::paint::TextureRef::new("a"),
        glyphs: vec![g(0.0), g(100.0), g(50.0)].into(),
        offset: Point::new(0.0, 0.0),
        color: Color::rgb(0.0, 0.0, 0.0),
        filter: blur(1.0),
        blend: BlendMode::default(),
    };
    let l = list_of(vec![cmd]);
    let p = &plan_offscreens(&l)[0];
    // 并集是 x∈[0,108]、y∈[10,22]；再各外扩 3。
    assert_eq!(
        p.content,
        Rect::new(0.0, 10.0, 108.0, 12.0),
        "取并集：最右字形到 108，最宽 ≠ 单字形宽"
    );
    assert_eq!(p.bounds, Rect::new(-3.0, 7.0, 114.0, 18.0));
}

/// 空字形的文本不产生离屏需求（内容矩形是零尺寸，RT 无意义）。
#[test]
fn empty_text_creates_no_offscreen_requirement() {
    let cmd = Command::Text {
        atlas: zephyra_core::paint::TextureRef::new("a"),
        glyphs: Vec::new().into(),
        offset: Point::new(0.0, 0.0),
        color: Color::rgb(0.0, 0.0, 0.0),
        filter: blur(2.0),
        blend: BlendMode::default(),
    };
    let l = list_of(vec![cmd]);
    let plans = plan_offscreens(&l);
    assert_eq!(plans.len(), 1, "命令本身带滤镜，所以仍有一条");
    assert_eq!(
        plans[0].content,
        Rect::new(0.0, 0.0, 0.0, 0.0),
        "内容是零尺寸 —— 宿主据此可以跳过开 RT"
    );
}

/// 排序不拆散效果 —— 离屏需求按 DisplayList 顺序产出，索引可用。
///
/// 这条是对"配对命令会被排序拆散"那个论断的正面反证：改成按内容
/// 几何后，跨 layer 的命令也不会错位。
#[test]
fn offscreen_requirement_index_maps_back_to_source_command() {
    let a = quad(
        FilterChain::none(),
        BlendMode::default(),
        Rect::new(0.0, 0.0, 5.0, 5.0),
    );
    let b = quad(
        blur(1.0),
        BlendMode::default(),
        Rect::new(10.0, 10.0, 20.0, 20.0),
    );
    let l = list_of(vec![a, b]);
    let plans = plan_offscreens(&l);
    assert_eq!(plans.len(), 1);
    let target = l.iter().nth(plans[0].command_index).unwrap();
    assert!(
        matches!(&target.command, Command::Quad { filter, .. } if !filter.is_identity()),
        "第{} 条应是带滤镜的命令，实际是 {:?}",
        plans[0].command_index,
        target.command
    );
}

/// 带滤镜的 Path 产出离屏需求，区域 = **顶点的紧包围盒** + outset。
///
/// 这是「矢量图标 + 投影被静默忽略」那个缺口的回归测试：以前这里直接返回
/// `None`，于是 `quad::expand` 照常画图标、只是不开离屏，效果没发生也不报错。
#[test]
fn blurred_path_yields_offscreen_with_tight_bounds() {
    // ⚠ 顶点顺序是刻意选的：**首顶点在内部**，后两个才是包围盒的对角。
    // 于是「拿首顶点当 min/max」「只遍历一部分」「忘了 min」这几类实现都会
    // 算出不同的数 —— 这个测试对它们敏感。
    let cmd = path(
        blur(2.0),
        BlendMode::default(),
        &[(20.0, 30.0), (10.0, 20.0), (33.0, 47.0)],
    );
    let plans = plan_offscreens(&list_of(vec![cmd]));
    assert_eq!(
        plans.len(),
        1,
        "带滤镜的 Path 必须开离屏，否则效果被静默忽略"
    );
    // 紧包围盒 = (10,20)–(33,47)。刻意不是任何"整数对齐"的矩形，
    // 这样 x/w 与 y/h 必须**分别**由 min 与 max 得出，混了就会露馅。
    assert_eq!(
        plans[0].content,
        Rect::new(10.0, 20.0, 23.0, 27.0),
        "content 必须是顶点紧包围盒，而不是布局矩形"
    );
    // σ=2 ⇒ 四边各扩 3σ=6。
    assert_eq!(
        plans[0].bounds,
        Rect::new(4.0, 14.0, 35.0, 39.0),
        "bounds = content + outset：23+12 / 27+12"
    );
}

/// 无滤镜的 Path 不开离屏 —— 这是图标最常见的路径，必须零开销。
#[test]
fn unfiltered_path_yields_no_offscreen() {
    let l = list_of(vec![
        path(
            FilterChain::none(),
            BlendMode::default(),
            &[(0.0, 0.0), (10.0, 10.0)],
        ),
        path(blur(0.0), BlendMode::default(), &[(0.0, 0.0), (10.0, 10.0)]),
    ]);
    assert!(
        plan_offscreens(&l).is_empty(),
        "恒等链与 σ=0 的 Path 都不该开 RT，否则每个图标每帧多一次 RT 绑定"
    );
}

/// 空顶点列表给出**零尺寸**而不是崩掉。
///
/// 这里刻意不断言"不产生需求"：空 Text 走的是同一条路（`union_of_quads`
/// 同样返回零矩形），要改就得两层一起改、且连带 Text/Quad/Image 的语义。
/// 真正的零尺寸护栏属于**执行侧**（E4 绑 RT 时），届时对所有命令类型统一处理，
/// 而不是在这里给 Path 单独开一个特例。
#[test]
fn empty_path_gives_zero_sized_bounds() {
    let plans = plan_offscreens(&list_of(vec![path(blur(2.0), BlendMode::default(), &[])]));
    assert_eq!(plans.len(), 1);
    assert_eq!(
        plans[0].content,
        Rect::new(0.0, 0.0, 0.0, 0.0),
        "空顶点 = 零尺寸内容，调用方自然开不出 RT"
    );
}

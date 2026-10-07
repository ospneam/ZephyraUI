//! 离屏目标规划：把"哪些命令需要离屏、要多大离屏纹理"算出来。
//!
//! # 职责边界
//!
//! 本模块**只算尺寸**，不碰任何 GPU 对象、不调任何 D3D API、不分配纹理。
//! 判据很简单：**能不能在没有 GPU 的机器上写单测并断言输出**。
//!
//! 这条边界带来两个实际好处：
//!
//! 1. CPU 参考光栅器（`cpu_raster`）能复用同一份尺寸计算，于是无 GPU 环境
//!    也能验证"离屏区域算得对不对"。
//! 2. 离屏尺寸是**效果正确性**的前提：算小了，模糊/阴影在边界处被硬切，
//!    症状是"效果的边缘有一道看不见的直边"——那种 bug 在真机上极难定位，
//!    而在这里是一行assertion。
//!
//! # 为什么不需要配对的层标记
//!
//! [`zephyra::paint::DisplayList`] 构造时即按 `(layer, z, order)` **稳定排序**。
//! `PushLayer` / `PopLayer` 这类配对标记靠"在有序流里前后夹住内容"生效，可排序
//! 会把中间内容插到别的 `layer` 上去，配对当场失效且不报错。
//!
//! 所以本模块走**按内容几何**的路子：每条命令自己算自己的离屏区域。这与 Skia
//! 一致 —— 它也从不录save/restore，而是靠 `SkCanvas::aboutToDraw` →
//! `AutoLayerForImageFilter`（`SkCanvasPriv.h:129`）在回放时按需开离屏，尺寸
//! 来自 `paint.computeFastBounds(drawBounds)`（`SkCanvasPriv.cpp:308`）。
//!
//! 与我们不同的地方是：Skia 那个 API 在 GPU 侧，尺寸算在 device 里；我们把它
//! 放在纯 CPU 层，所以能单测。

use zephyra::core::Rect;
use zephyra::filter::FilterChain;
use zephyra::paint::{Command, DisplayList};

/// 一张离屏纹理的规划。
///
/// `atlas` 那类共享资源不在此处——本模块只管**效果**离屏，纹理复用策略属于
/// 资源管理（对应 [`zephyra::text::atlas`] 里的淘汰策略那一层）。
#[derive(Debug, Clone, PartialEq)]
pub struct OffscreenPlan {
    /// 要开离屏的命令在 [`DisplayList`] 中的下标。
    pub command_index: usize,
    /// RT 要覆盖的区域（已含滤镜 outset）。
    pub bounds: Rect,
    /// 内容在 RT 内的局部原点。
    ///
    /// 几何坐标仍是屏幕坐标，所以宿主必须把 `bounds.x/y` 减掉才能定位 ——
    /// 这就是 [`zephyra::filter::FilterChain::origin_in`] 的用处。
    pub origin: zephyra::core::Point,
    /// 内容区未外扩的原始矩形。
    pub content: Rect,
    /// 该离屏承载的滤镜链。
    pub filter: FilterChain,
}

/// 规划整帧的离屏目标，按 DisplayList 顺序返回。
///
/// # 复用：相同矩形会被合并成一条
///
/// 两条命令若离屏矩形**完全相同**（常见于同一个卡片上的背景 + 文本都带同一
/// 个阴影），合并成一张 RT。判据是矩形相等而不是"重叠" —— 重叠合并需要
/// 面积包含判断，而两张 RT 恰好重叠时复用错误的后果是画面错位，收益
/// （省一张小 RT）远不抵风险。
pub fn plan_offscreens(list: &DisplayList) -> Vec<OffscreenPlan> {
    let mut out: Vec<OffscreenPlan> = Vec::new();
    for (i, item) in list.iter().enumerate() {
        let Some((filter, content)) = split_filter_and_content(&item.command) else {
            continue;
        };
        if filter.is_identity() {
            continue;
        }
        let bounds = filter.bounds_for(content);
        let origin = filter.origin_in(content);
        // 与已有的一张完全相同就复用 —— 典型场景是同一张卡片的背景与文本
        // 共用一个投影。
        if let Some(existing) = out.iter_mut().find(|p| p.bounds == bounds) {
            if existing.filter == *filter {
                continue;
            }
        }
        out.push(OffscreenPlan {
            command_index: i,
            bounds,
            origin,
            content,
            filter: filter.clone(),
        });
    }
    out
}

/// 取出命令的滤镜与内容矩形；`None` = 该命令不参与效果离屏。
fn split_filter_and_content(cmd: &Command) -> Option<(&FilterChain, Rect)> {
    match cmd {
        Command::Quad { rect, filter, .. } => Some((filter, *rect)),
        Command::Image { dst, filter, .. } => Some((filter, *dst)),
        Command::Text {
            glyphs, filter, ..
        } => Some((filter, union_of_glyphs(glyphs))),
        // ⚠️ **已知缺口**：Path 的顶点是三角化后的 mesh，本层没有它的包围盒，
        // 所以这里返回 `None` —— 后果是**带滤镜 / 阴影的图标会被静默忽略**
        // （`quad::expand` 照常画它，只是不开离屏，于是效果没发生、也不报错）。
        //
        // 这不是设计决定，是没接。补起来很小：包围盒 = 所有顶点的 min/max。
        // 在补上之前，「矢量图标 + 投影」这个组合不要指望生效。
        Command::Path { .. } | Command::PushClip(_) | Command::PopClip => None,
    }
}

/// 一批字形四边形的并集。
///
/// 空列表给零尺寸矩形而不是 `Option` —— 调用方拿到零矩形后自然开不出可用的 RT，
/// 而 `Option` 会逼每个调用点写一遍 `if let`。
fn union_of_glyphs(glyphs: &[zephyra::paint::GlyphQuad]) -> Rect {
    let Some(first) = glyphs.first() else {
        return Rect::new(0.0, 0.0, 0.0, 0.0);
    };
    let (mut x0, mut y0) = (first.dst.x, first.dst.y);
    let (mut x1, mut y1) = (first.dst.right(), first.dst.bottom());
    for g in &glyphs[1..] {
        x0 = x0.min(g.dst.x);
        y0 = y0.min(g.dst.y);
        x1 = x1.max(g.dst.right());
        y1 = y1.max(g.dst.bottom());
    }
    Rect::new(x0, y0, x1 - x0, y1 - y0)
}

/// 本帧有多少条命令的混合模式不是默认值。
///
/// 混合模式**不**产生离屏需求 —— 它改的是混合方程，不是"要多少像素"。
/// 绝大多数模式能在现有 target 上换混合状态完成（Skia 的 `GrBlendMode` 就是
/// 这么用的）。但它需要 PSO 变体，所以宿主要知道有没有。
pub fn count_non_default_blends(list: &DisplayList) -> usize {
    list.iter()
        .filter(|i| {
            let b = match &i.command {
                Command::Quad { blend, .. }
                | Command::Path { blend, .. }
                | Command::Image { blend, .. }
                | Command::Text { blend, .. } => *blend,
                Command::PushClip(_) | Command::PopClip => {
                    zephyra::filter::BlendMode::default()
                }
            };
            b != zephyra::filter::BlendMode::default()
        })
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use zephyra::core::{Color, CornerRadius, Point};
    use zephyra::filter::{BlendMode, Filter};
    use zephyra::paint::{Layer, SortKey};

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
    fn 无滤镜不产生离屏需求() {
        let l = list_of(vec![
            quad(FilterChain::none(), BlendMode::default(), Rect::new(0.0, 0.0, 10.0, 10.0)),
            quad(blur(0.0), BlendMode::default(), Rect::new(0.0, 0.0, 10.0, 10.0)),
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
    fn 局部原点就是离屏矩形左上角() {
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
    fn 相同离屏矩形会被合并() {
        // 两个不同矩形，但外扩后恰好同尺寸 → 不会被合并（矩形不等）。
        let a = quad(blur(2.0), BlendMode::default(), Rect::new(0.0, 0.0, 10.0, 10.0));
        let b = quad(blur(2.0), BlendMode::default(), Rect::new(0.0, 0.0, 10.0, 10.0));
        let l = list_of(vec![a, b]);
        assert_eq!(
            plan_offscreens(&l).len(),
            1,
            "同矩形同滤镜应合并成一张 RT"
        );
        // 不同矩形则各开一张。
        let l2 = list_of(vec![
            quad(blur(2.0), BlendMode::default(), Rect::new(0.0, 0.0, 10.0, 10.0)),
            quad(blur(2.0), BlendMode::default(), Rect::new(20.0, 0.0, 10.0, 10.0)),
        ]);
        assert_eq!(plan_offscreens(&l2).len(), 2);
    }

    /// 投影的不对称外扩被完整反映到 RT 尺寸上。
    #[test]
    fn 投影离屏矩形包含偏移() {
        let f = FilterChain::single(Filter::DropShadow {
            color: Color::rgba(0.0, 0.0, 0.0, 0.4),
            offset: Point::new(4.0, 6.0),
            sigma: 2.0,
        });
        let l = list_of(vec![quad(f, BlendMode::default(), Rect::new(0.0, 0.0, 100.0, 50.0))]);
        let p = &plan_offscreens(&l)[0];
        assert_eq!(p.bounds, Rect::new(-2.0, 0.0, 112.0, 62.0));
    }

    /// 混合模式不产生离屏需求 —— 它改的是混合方程，不是像素数量。
    #[test]
    fn 混合模式不产生离屏需求() {
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
    fn 文本离屏范围是全部字形的并集() {
        let g = |x: f32| zephyra::paint::GlyphQuad {
            dst: Rect::new(x, 10.0, 8.0, 12.0),
            uv: zephyra::core::UvRect::FULL,
            color_glyph: false,
        };
        let cmd = Command::Text {
            atlas: zephyra::paint::TextureRef::new("a"),
            glyphs: vec![g(0.0), g(100.0), g(50.0)],
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
    fn 空文本不产生离屏需求() {
        let cmd = Command::Text {
            atlas: zephyra::paint::TextureRef::new("a"),
            glyphs: vec![],
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
    fn 离屏需求的下标能定位回原命令() {
        let a = quad(FilterChain::none(), BlendMode::default(), Rect::new(0.0, 0.0, 5.0, 5.0));
        let b = quad(blur(1.0), BlendMode::default(), Rect::new(10.0, 10.0, 20.0, 20.0));
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
}
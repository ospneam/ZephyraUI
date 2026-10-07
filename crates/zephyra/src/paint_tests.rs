//! [`crate::paint::render`] 的降级产物断言 —— **只查契约，不查 GPU**。
//!
//! # 为什么单独一个文件而不是塞进 `tests.rs`
//!
//! 这一组测的是「图标名 → `Command::Path`」这一条链，它的中间环节（SVG 解析、曲线
//! 展平、多孔三角化）各有各的测试文件。`tests.rs` 里已有的是"布局 → 命令"那类，
//! 而图标链**额外**跨了三层，所以单独成文件让边界清楚。
//!
//! # 为什么只看 `DisplayList`
//!
//! 因为这是**唯一**不依赖渲染后端的判据。历史上这条链缺观测手段时，
//! "图标接通了"与"图标被当成文字画了"在所有像素指标上都长得一样 ——
//! 四边形数、顶点数、内容像素全对（见 `icon_command` 的文档）。

use crate::context::Context;
use crate::core::{Color, Rect};
use crate::element::Element;
use crate::id::NodeTag;
use crate::layout::layout;
use crate::node::Node;
use crate::paint::{render, Command, DisplayList};
use crate::property::PropertyValue;
use crate::theme::Theme;

/// 走完 声明 → 布局 → 降级，挂上**内置**图标集。
///
/// 与 `tests.rs` 里的同名辅助函数刻意同形 —— 唯一差别是这里
/// `.with_icons(builtin())`，而那正是图标链需要的唯一外部条件。
///
/// # 为什么根节点用 `bounds` 而不是 `WIDTH` / `HEIGHT` 定尺寸
///
/// [`layout`] 里写明了：根节点最终由 `arrange` **铺满 `bounds`**，那是"填满窗口"
/// 的正确语义。所以根上的 `WIDTH` / `HEIGHT` 不生效（它们是给子节点用的）。
///
/// 而图标链要测的恰恰是「节点矩形 → 几何」这条映射，矩形必须**真的**是预期值，
/// 否则测的是"图标铺满 200×100 对不对" —— 判据会静默地变成别的东西。
/// 所以尺寸从 `bounds` 传进来。
fn render_of(element: &Element, bounds: Rect) -> DisplayList {
    let theme = Theme::dark();
    let ctx = Context::new(&theme).with_icons(crate::icon::builtin());
    let mut node = Node::from_element(element);
    layout(&mut node, bounds, ctx);
    let mut list = DisplayList::new();
    render(&node, ctx, &mut list);
    list
}

/// 单条测试用的节点矩形：20×20，与内置图标的 24×24 viewBox 等比缩放。
const UNIT: Rect = Rect::new(0.0, 0.0, 20.0, 20.0);

/// 放大一档的节点矩形：48×48。
const BIG: Rect = Rect::new(0.0, 0.0, 48.0, 48.0);

/// 一个带图标的节点。**尺寸由 [`render_of`] 的 `bounds` 决定**（见那里的说明）。
fn icon_el(name: &'static str) -> Element {
    Element::new(NodeTag::Container)
        .with_key("root")
        .with(&crate::props::ICON_NAME, PropertyValue::static_str(name))
}

/// 所有 `Command::Path` 顶点的包围盒。
fn path_bounds(list: &DisplayList) -> Option<(f32, f32)> {
    let mut lo = (f32::MAX, f32::MAX);
    let mut hi = (f32::MIN, f32::MIN);
    let mut any = false;
    for item in list.iter() {
        let Command::Path { vertices, .. } = &item.command else {
            continue;
        };
        // `vertices` 是 `Arc<[Vertex]>`；`Arc` 的 `Deref` 到切片，但不为迭代器实现，
        // 所以要显式 `.iter()`。
        for v in vertices.iter() {
            any = true;
            lo.0 = lo.0.min(v.position.x);
            lo.1 = lo.1.min(v.position.y);
            hi.0 = hi.0.max(v.position.x);
            hi.1 = hi.1.max(v.position.y);
        }
    }
    // `then` 而不是 `then_some`：后者会**急切**求值参数，于是"没有任何 Path"
    // 时也算一次减法。代价可忽略，但 `then` 才是这里表达"有才算是"的写法。
    #[allow(clippy::unnecessary_lazy_evaluations, reason = "then_some 会急切求值")]
    any.then(|| (hi.0 - lo.0, hi.1 - lo.1))
}

/// 本列表里所有 `Command::Path` 的三角形总数。
fn path_triangles(list: &DisplayList) -> usize {
    list.iter()
        .filter_map(|i| match &i.command {
            Command::Path { vertices, .. } => Some(vertices.len() / 3),
            _ => None,
        })
        .sum()
}

/// 已登记的内置图标都能产出非空路径。
///
/// # 判据为什么是"三角形数 > 0"而不是具体数值
///
/// 具体数值会随展平容差与图标形状变化 —— 那让它变成一条必须跟着常量更新的断言，
/// 于是改容差时它先失败，而失败信息（"预期 47，实得 52"）对读的人毫无帮助。
///
/// "非空"才是这条链真正的契约：解析 → 展平 → 三角化 任何一环静默返回空，
/// 症状都是**图标整个不出现**，且没有任何报错（`RenderReport` 里只少几个三角形）。
/// 所以判据取存在性，把数值留给 `demo --bench` 观察。
#[test]
fn 每个内置图标都产出非空路径() {
    let names = crate::icon::assets::REGISTRY.iter().map(|(n, _)| *n);
    let mut checked = 0usize;
    for name in names {
        // 关掉"图像优先于图标"这条：内置图标不带纹理，不影响。
        let list = render_of(&icon_el(name), UNIT);
        let tris = path_triangles(&list);
        assert!(
            tris > 0,
            "内置图标 {name:?} 没有产出任何三角形 —— \
             症状是图标整个不出现，且**不报任何错**。链上任一环静默返回空都长这样：\
             图标集查不到 / SVG 解析失败 / 展平为空 / 三角化失败"
        );
        // 三角形必须是**整数个**（写入 Vertices 时按 3 个一组），而具体数量
        // 交给 `路径顶点数恒为三的倍数` 与 `demo --bench` 观察。
        checked += 1;
    }
    assert!(
        checked >= 10,
        "内置图标只有 {checked} 个 —— 资源表被误删了，这条测试就测不到东西了"
    );
}

/// 未登记的图标名**安全降级**：不产出命令，也不 panic。
///
/// # 为什么这条重要
///
/// 图标名是**字符串**，而字符串会拼错。崩溃在这里是最坏的处理方式 ——
/// 它把「某个图标不显示」升级成「整个程序起不来」。
///
/// 而"什么都不产出"是好的：症状是可见的（少一个图标），可对比、可定位。
#[test]
fn 未登记的图标名安全降级() {
    let list = render_of(&icon_el("this-icon-does-not-exist"), UNIT);
    assert_eq!(
        path_triangles(&list),
        0,
        "未登记的名字不该产出几何 —— `IconSet::get` 应当回落到 `Icon::EMPTY`"
    );
    assert!(
        list.is_empty(),
        "未登记的名字不该产出**任何**命令；实得 {} 条",
        list.len()
    );
}

/// 图标的几何落在节点矩形内。
///
/// # 这条在钉什么
///
/// `icon_command` 做 viewBox → 节点矩形的映射（等比缩放 + 居中）。映射算错时，
/// 三角形仍会存在、仍会被画 —— 只是画在**框外**或**贴错位置**。
///
/// 而那个症状与"图标不显示"完全不同（前者是图形出现在奇怪的地方），所以需要
/// 一条独立断言，而不是靠"非空"顺带覆盖。
#[test]
fn 图标几何落在节点矩形内() {
    let list = render_of(&icon_el("gear"), UNIT);
    for item in list.iter() {
        let Command::Path { vertices, .. } = &item.command else {
            continue;
        };
        // `vertices` 是 `Arc<[Vertex]>`；`Arc` 的 `Deref` 到切片，但不为迭代器实现，
        // 所以要显式 `.iter()`。
        for v in vertices.iter() {
            // 矩形 20×20 位于原点；留 0.5 的余量给展平的弦高误差。
            assert!(
                v.position.x >= -0.5 && v.position.x <= 20.5,
                "顶点 x={} 落在 20×20 的节点矩形之外 —— viewBox 映射算错了",
                v.position.x
            );
            assert!(
                v.position.y >= -0.5 && v.position.y <= 20.5,
                "顶点 y={} 落在 20×20 的节点矩形之外 —— viewBox 映射算错了",
                v.position.y
            );
        }
    }
}

/// 节点矩形放大时，**产出的几何也等比放大** —— 证明映射真的用上了 `rect`。
///
/// # 为什么需要这条
///
/// `icon_command` 有一个很容易写错的版本：把节点矩形算丢了，直接用 viewBox 的原始
/// 坐标画（`viewBox` 通常是 24×24，而节点可能是 20×20 或 48×48）。
///
/// 那种错误的症状是"图标大小固定、位置固定，与容器无关"—— 它**看起来完全正常**
/// （图标确实画出来了），只是尺寸不对。所以判据必须落在"随尺寸变化"上。
///
/// # 判据为什么是包围盒而**不是**三角形数量
///
/// 最初这条断言写的是"48×48 的三角形更多"，实测两者相等（都是 125 个）。
///
/// 那是**实现对的、断言错的**：[`crate::geom::flatten`] 在映射**之前**调用，容差
/// 0.25 是 **viewBox 单位**（见 `flatten::TOLERANCE` 的文档：它谈的是"24 DIP 的
/// 图标在 200% 缩放下是 48 物理像素"）—— 所以顶点数只取决于图标本身，与画到多大
/// 无关。这是有意的**分辨率无关**：同一个图标在 1× 和 4× 下共用一份展平结果。
///
/// 于是三角形数量对这条判据**完全没有分辨力**（两种正确实现给出同样的 125），
/// 而唯一能区分"用了 rect"与"丢了 rect"的量是**坐标范围**。
#[test]
fn 图标尺寸随节点变大而放大() {
    let small = path_bounds(&render_of(&icon_el("gear"), UNIT))
        .expect("20×20 的齿轮应当产出路径");
    let big = path_bounds(&render_of(&icon_el("gear"), BIG))
        .expect("48×48 的齿轮应当产出路径");

    // viewBox 是 24×24 等比映射到 20×20 与 48×48，宽高比一致 ⇒ 缩放因子
    // 恰好是 20/24 与 48/24，即 2.4 倍。留 1% 余量给浮点与展平的弦高误差。
    let ratio = big.0 / small.0;
    assert!(
        (ratio - 2.4).abs() < 0.05,
        "48×48 的齿轮几何应当是 20×20 的 2.4 倍，实得 {ratio:.3} \
         （20×20 宽 {:.2} / 48×48 宽 {:.2}）—— \
         若约等于 1.0，说明映射把节点矩形算丢了、直接用了 viewBox 的原始坐标",
        small.0,
        big.0
    );
    // 高宽同理，且两者应当一致（等比缩放，不许压扁）。
    assert!(
        (big.1 / small.1 - ratio).abs() < 0.05,
        "等比缩放下高宽倍率应当一致，实得 {:.3} vs {:.3}",
        big.1 / small.1,
        ratio
    );
}

/// 图像与图标都设时：**图像胜出**，不产出路径。
///
/// # 为什么这条规则要钉死
///
/// 两者都占用同一个视觉位置，不写判据的话实现者只能猜。而猜错的两边症状都不明显：
/// 「两个都画」是图标压在图像上，「都不画」是元素凭空消失。
///
/// 选图像胜出的理由写在 `paint::render_rec` 里（位图已有缓存、自带抗锯齿、路径要走 MSAA）。
/// 这里是那条规则的回归护栏。
/// `CORNER_RADIUS` 必须落到 `Command::Image` 的 `radius` 上。
///
/// # 为什么这条重要
///
/// 图像圆角是**新增能力**，而 `CORNER_RADIUS` 是早就存在的通用样式属性。
/// 如果 `render_rec` 的图像分支忘了读它，症状是「矩形有圆角、图像没有」——
/// 而两者在 demo 里并排出现时，肉眼会以为「图像不支持圆角」而不是「接线漏了」。
#[test]
fn 圆角半径传到图像命令() {
    let el = Element::new(NodeTag::Container)
        .with(
            &crate::props::IMAGE_TEXTURE,
            PropertyValue::static_str("t"),
        )
        .with(&crate::props::CORNER_RADIUS, PropertyValue::float(14.0));
    let list = render_of(&el, UNIT);
    let radii: Vec<f32> = list
        .iter()
        .filter_map(|i| match &i.command {
            Command::Image { radius, .. } => Some(radius.radius),
            _ => None,
        })
        .collect();
    assert_eq!(
        radii,
        vec![14.0],
        "圆角半径没进图像命令 —— 图像会画成直角且不报错"
    );
}

/// 图像的 `radius` 缺省是 0（直角），且**与矩形共用同一个属性**。
#[test]
fn 图像默认圆角是零() {
    let el =
        Element::new(NodeTag::Container).with(&crate::props::IMAGE_TEXTURE, PropertyValue::static_str("t"));
    let list = render_of(&el, UNIT);
    let radii: Vec<f32> = list
        .iter()
        .filter_map(|i| match &i.command {
            Command::Image { radius, .. } => Some(radius.radius),
            _ => None,
        })
        .collect();
    assert_eq!(radii, vec![0.0], "未设 CORNER_RADIUS 时图像应是直角");
}

#[test]
fn 图像与图标都设时图像胜出() {
    let el = Element::new(NodeTag::Container)
        .with_key("root")
        .with(&crate::props::ICON_NAME, PropertyValue::static_str("gear"))
        .with(
            &crate::props::IMAGE_TEXTURE,
            PropertyValue::static_str("some-texture"),
        )
;
    let list = render_of(&el, UNIT);
    assert_eq!(
        path_triangles(&list),
        0,
        "图像存在时不该再产出路径 —— 两条路径都画会让图标压在图像上"
    );
    assert!(
        list.iter()
            .any(|i| matches!(i.command, Command::Image { .. })),
        "应当仍有 Image 命令"
    );
}

/// 图标颜色取自 `ICON_COLOR`，默认是白色。
///
/// # 这条在钉什么
///
/// `ICON_COLOR` **刻意不继承**（见该属性的文档）—— 图标颜色由所在控件决定，而
/// 继承链上的文本色未必同义。那条设计决定需要一个断言，否则后人会"顺手补上继承"
/// （看起来更方便），然后按钮里的图标跟着外层文本色一起变。
#[test]
fn 图标颜色取自_icon_color_且默认不继承() {
    // 显式给色。
    let colored = Element::new(NodeTag::Container)
        .with_key("root")
        .with(&crate::props::ICON_NAME, PropertyValue::static_str("gear"))
        .with(&crate::props::ICON_COLOR, PropertyValue::color(Color::rgb(1.0, 0.0, 0.0)))
;
    let list = render_of(&colored, UNIT);
    let fill = list
        .iter()
        .find_map(|i| match &i.command {
            Command::Path { fill, .. } => Some(*fill),
            _ => None,
        })
        .expect("应当有 Path 命令");
    assert!(
        (fill.r - 1.0).abs() < 1e-6 && fill.g < 1e-6,
        "图标填充色应当是显式给的红色：{:?}",
        fill
    );

    // 不给色时默认白色（**不是**继承来的）。
    let default = render_of(&icon_el("gear"), UNIT);
    let fill = default
        .iter()
        .find_map(|i| match &i.command {
            Command::Path { fill, .. } => Some(*fill),
            _ => None,
        })
        .expect("应当有 Path 命令");
    assert!(
        fill.r > 0.99 && fill.g > 0.99 && fill.b > 0.99,
        "未给 ICON_COLOR 时默认白色：{:?}",
        fill
    );
}

/// 三角形顶点数恒为 3 的倍数，且每条 Path 命令的顶点数非零。
///
/// # 为什么"3 的倍数"是硬契约
///
/// fx 层的路径索引按 `path_tris = paths.len() / 3` 算（`renderer.rs` 里的
/// `DrawIndexed(path_tris * 3, ...)`）。若顶点数不是 3 的倍数，整数截断会让
/// 索引**少一截** —— 表现为路径缺最后一个三角形，且**不报任何错**。
///
/// 这条断言放在 core（契约层）而不是 fx：core 是产出方，由它保证"我给的顶点数
/// 一定是完整三角形的倍数"才是负责的做法。fx 那边只能验证消费方没错。
#[test]
fn 路径顶点数恒为三的倍数() {
    let list = render_of(&icon_el("camera"), UNIT);
    let mut paths = 0usize;
    for item in list.iter() {
        let Command::Path { vertices, .. } = &item.command else {
            continue;
        };
        paths += 1;
        assert!(
            !vertices.is_empty(),
            "Path 命令不该带空顶点（那是一条什么都不画的命令）"
        );
        assert_eq!(
            vertices.len() % 3,
            0,
            "路径顶点数 {} 不是 3 的倍数 —— fx 的 `paths.len()/3` 会截断，\
             症状是路径缺最后一个三角形且**不报任何错**",
            vertices.len()
        );
    }
    assert!(paths > 0, "camera 应当产出至少一条 Path 命令");
}

// ---- 几何缓存 ----
//
// 缓存引入了一类新故障：命中时返回**上一次**的几何。功能全对（图标还是画出来了），
// 只是位置/大小不对 —— 而那种症状与"viewBox 映射算错"完全同形，正是上面两条
// 测试要区分的那件事。所以这里专门钉"缓存不能串味"。

/// 缓存**不得**让不同尺寸的图标串味。
///
/// # 这条在钉什么
///
/// 缓存键是（图标数据地址，目标矩形位模式）。若键漏了矩形的某个分量（最可能的是
/// 只记宽高、漏掉 x/y），则同一个图标在**不同位置**画两次时会命中同一条目——
///
/// 症状是「第二个图标叠在第一个的位置上」，而四条已有测试全都通过：
/// 它们各自只画一个图标、一个矩形，恰好落在同一个键上。
#[test]
fn 缓存不得让不同位置的图标串味() {
    let a = path_bounds(&render_of(&icon_el("gear"), Rect::new(0.0, 0.0, 20.0, 20.0)))
        .expect("第一个齿轮");
    let b = path_bounds(&render_of(&icon_el("gear"), Rect::new(40.0, 0.0, 20.0, 20.0)))
        .expect("第二个齿轮");

    // 尺寸相同是前提 —— 这条只管"位置进了键没有"。
    assert!(
        (a.0 - b.0).abs() < 1e-4 && (a.1 - b.1).abs() < 1e-4,
        "同一图标画在两个位置，包围盒应相同：{a:?} vs {b:?}"
    );

    // 真正的判据：第二个的**顶点 x** 应当整体右移 40。
    let xs_of = |x0: f32| -> (f32, f32) {
        let list = render_of(&icon_el("gear"), Rect::new(x0, 0.0, 20.0, 20.0));
        let mut lo = f32::MAX;
        let mut hi = f32::MIN;
        for item in list.iter() {
            let Command::Path { vertices, .. } = &item.command else { continue };
            // `vertices` 是 `Arc<[Vertex]>`；`Arc` 的 `Deref` 到切片，但不为迭代器实现，
        // 所以要显式 `.iter()`。
        for v in vertices.iter() {
                lo = lo.min(v.position.x);
                hi = hi.max(v.position.x);
            }
        }
        (lo, hi)
    };
    let (lo0, hi0) = xs_of(0.0);
    let (lo1, hi1) = xs_of(40.0);
    assert!(
        (lo1 - lo0 - 40.0).abs() < 1e-3,
        "第二个齿轮应当整体右移 40：[{lo0:.2},{hi0:.2}] → [{lo1:.2},{hi1:.2}]—— \
         若差值约为 0，说明矩形的 x 没进缓存键"
    );
}

/// 缓存**不得**让不同尺寸的图标串味。
///
/// 与上一条同形、判据不同：键里若只有位置没有尺寸，则同一位置先画20×20 再画
/// 48×48，第二次会拿到 20×20 的几何。
#[test]
fn 缓存不得让不同尺寸的图标串味() {
    let small = path_bounds(&render_of(&icon_el("gear"), UNIT)).expect("20×20");
    let big = path_bounds(&render_of(&icon_el("gear"), BIG)).expect("48×48");

    assert!(
        big.0 > small.0 * 2.0,
        "同一位置先后画 20×20 与 48×48，第二个应当更大：{small:?} → {big:?}—— \
         若两者相同，说明矩形的尺寸没进缓存键"
    );
}

/// 缓存命中与首次计算必须产出**逐字节相同**的几何。
///
/// # 为什么这条不能靠"画出来一样"判
///
/// 两次调用走的是不同代码路径（一次哈希命中、一次完整展平 + 耳切）。若它们只在
/// 某个退化情形下不同（比如某个孔的绕向），症状是「图标某个角缺一块」——
/// 而那与"多孔三角化有缺陷"完全同形。
///
/// 所以判据取**逐顶点相等**：没有第二条路能蒙混过去。
#[test]
fn 缓存命中与首次计算逐字节相同() {
    let path_of = |bounds: Rect| -> Vec<[f32; 2]> {
        let list = render_of(&icon_el("gear"), bounds);
        let mut all = Vec::new();
        for item in list.iter() {
            let Command::Path { vertices, .. } = &item.command else { continue };
            // `vertices` 是 `Arc<[Vertex]>`；`Arc` 的 `Deref` 到切片，但不为迭代器实现，
        // 所以要显式 `.iter()`。
        for v in vertices.iter() {
                all.push([v.position.x, v.position.y]);
            }
        }
        all
    };

    // 第一次调fill 缓存，第二次必然命中。
    let first = path_of(UNIT);
    let second = path_of(UNIT);
    assert!(
        !first.is_empty(),
        "齿轮至少要产出一些顶点，否则这条断言比较的是两个空集"
    );
    assert_eq!(
        first.len(),
        second.len(),
        "命中缓存前后顶点数不同：{} vs {}—— \
         说明缓存的不是同一份几何",
        first.len(),
        second.len()
    );
    for (i, (a, b)) in first.iter().zip(second.iter()).enumerate() {
        assert_eq!(
            a, b,
            "第 {i} 个顶点不同：{a:?} vs {b:?}—— \
             缓存键没有覆盖住全部映射输入"
        );
    }
}

/// 不同图标**不得**共享缓存条目。
///
/// 键取的是图标数据的**地址**。若哪天改成按名字取，而两个名字映射到同一段
/// `include_str!`（框架里"别名的图标"很常见），它们会合法地共享 —— 那是对的。
/// 真正要防的是**取错了**：键算成常量 0，于是所有图标命中同一条。
#[test]
fn 不同图标不共享缓存条目() {
    let gear = path_triangles(&render_of(&icon_el("gear"), UNIT));
    let camera = path_triangles(&render_of(&icon_el("camera"), UNIT));

    assert!(gear > 0 && camera > 0, "两个内置图标都要有几何");
    assert_ne!(
        gear, camera,
        "齿轮与相机都画在 20×20 上却给出同样的顶点数（{gear}）—— \
         说明缓存键没有区分图标（多半是取成了常量）"
    );
}

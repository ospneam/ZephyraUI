//! 路径几何：把 [`Segment`]（SVG 路径段）变成三角形顶点。
//!
//! # 这一层解决什么
//!
//! [`crate::paint::Command::Path`] 承载矢量图标的 CPU 三角化结果。链条是：
//!
//! ```text
//!   Segment[]  ──flatten──▶  折线轮廓[]  ──stroke?──▶  轮廓[]  ──triangulate──▶  Vertex[]
//!   (icon)                    (本模块)                   (本模块)              (Command::Path)
//! ```
//!
//! 三段都在核心里，`paint` 降级时调用，fx 的 `quad::expand` 再把它逐三角形裁剪后
//! 写进顶点流（抗锯齿靠 MSAA，见下）。图标**不再**走纹理化的 `Command::Image`。
//!
//! # 为什么在核心里而不是效果器层
//!
//! 判据是 [`crate::icon::mod`] 那条：**能不能在无 GPU 的机器上写单测**。这里全部是
//! `f32` 算术与 `Vec`，没有系统 API、没有纹理句柄，于是每一步都能断言：
//!
//! - 展平的**弦高误差**有上界（不是"看起来够圆"）；
//! - 三角化后所有三角形的**面积之和**等于按填充规则算出的多边形面积；
//! - 描边宽度可以用"内外轮廓相距"直接量出来。
//!
//! 这些断言在 GPU 上一个字也验不了，而它们恰好是"图标形状略微不对"这类 bug 的
//! **唯一**检出手段——所以必须在核心里。
//!
//! # 抗锯齿不在这里
//!
//! 产出的是普通三角形，片元着色器不知道覆盖率，所以边缘靠 **MSAA**。这是刻意的
//! 分工：[`crate::paint::Command::Quad`] 的抗锯齿来自 SDF（几何自带距离场），
//! 而三角化后的多边形没有距离场；要在几何层做解析式 AA，就得让每个三角形携带
//! 边方程并保证相邻边严格共享，那是另一个数量级的复杂度与一整类接缝 bug。
//!
//! MSAA 是**后端**的采样数开关，正好落在该放它的那一层。

pub mod flatten;
pub mod stroke;
pub mod tessellate;

use crate::core::{Point, Rect};
use crate::icon::Segment;

pub use flatten::{flatten, FlatPath, TOLERANCE};
pub use stroke::{stroke_outline, LineCap, LineJoin, StrokeStyle};
pub use tessellate::{area_of, tessellate, triangulate, TriMesh, TriMeshError};

// `FillRule` 从 `icon` 借来用，但要**转发出去**：它是"同一组轮廓按哪种规则判定内外"
// 的约定，调用方（`paint::icon_command`、宿主、未来的 WindingRule 支持）需要指名它。
// 只在这里 `use` 而不 `pub use` 的话，它就成了本模块的私有实现细节 —— 而它明明是
// 公开契约的一部分（`icon::FillRule` 与 `geom::FillRule` 是**同一个类型**，不是两个
// 各自演化的枚举）。
pub use crate::icon::FillRule;

/// 路径几何的失败（面向调用方的简���错误类型）。
///
/// 只是一层薄包装：真正的错误信息在 [`TriMeshError`] 里，而它的名字在公开API 上
/// 会被反复引用，所以对外给一个不带私有细节的名字。
#[derive(Debug, Clone, PartialEq)]
pub struct TriangulateError(pub String);

impl core::fmt::Display for TriangulateError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// 一条闭合的折线轮廓（展平后的子路径）。
///
/// **不含重复的首尾点** —— 首尾相连这件事由 [`Contour::edges`] 隐含表达。
/// 这不是省事：首尾都放一遍点会让耳切法数错顶点数，而那种错误表现为"某一块
/// 多出一个三角形"，肉眼极难定位。
#[derive(Debug, Clone, PartialEq)]
pub struct Contour {
    points: Vec<Point>,
}

impl Contour {
    /// 由折线点构造。会自动去掉相邻重复点与末尾与开头重复的点。
    pub fn new(points: Vec<Point>) -> Self {
        let mut out: Vec<Point> = Vec::with_capacity(points.len());
        for p in points {
            // 相邻重复点：零长边会让后面 ear-clipping 的"点在三角形内"判定
            // 拿到退化输入，进而产生零面积三角形。
            if out.last().is_some_and(|q| same_point(*q, p)) {
                continue;
            }
            out.push(p);
        }
        while out.len() >= 2 && same_point(out[0], *out.last().unwrap()) {
            out.pop();
        }
        Self { points: out }
    }

    /// 折线点。
    pub fn points(&self) -> &[Point] {
        &self.points
    }

    /// 顶点数（已去重）。
    pub fn len(&self) -> usize {
        self.points.len()
    }

    /// 顶点是否为空。
    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    /// 是否至少有三个**互不共线**的顶点——退化的轮廓（点、线段）不可填充。
    pub fn is_fillable(&self) -> bool {
        if self.points.len() < 3 {
            return false;
        }
        // 三点共线的轮廓面积为 0，填充它没有意义，而耳切法会在它上面死循环。
        abs_area(self.points()) > AREA_EPS
    }

    /// 有向面积的两倍（**未除以 2**）。
    ///
    /// > 0 表示**逆时针**（标准 shoelace 约定，与 `y` 轴方向无关）。
    ///
    /// # 坐标系与"绕向"的坑
    ///
    /// 本框架与 SVG 一样 **y 向下**（[`crate::core::Rect`] 的 y 从上往下增）。
    /// 于是在这套坐标系里，"标准 shoelace 为正"看起来是**顺时针**。
    ///
    /// 两边都对，但混用会让符号判断整体反过来 —— 症状是"洞被填实了"或
    /// "耳切找不到耳朵"，而那恰好是最容易被误判成"填充规则实现错了"的现象。
    ///
    /// 所以本模块一律用 [`signed_area2`] 取符号，不在别处手写 shoelace。
    pub fn signed_area2(&self) -> f32 {
        signed_area2(&self.points)
    }

    /// 按指定绕向重排（仅在需要整体反向时用，例如统一外轮廓绕向）。
    pub fn reversed(&self) -> Self {
        let mut p = self.points.clone();
        p.reverse();
        Self { points: p }
    }
}

/// 按填充规则算多边形面积。
///
/// - [`FillRule::NonZero`]：外轮廓面积之和减去被它包住的孔（洞的绕向与外相反，
///   所以直接按符号累加即可）；
/// - [`FillRule::EvenOdd`]：按嵌套深度交替加减。
///
/// # 这是**符号面积**，不是"视觉面积"
///
/// 对一个 EvenOdd 的双层轮廓（外圈 + 中间反向洞 + 洞里反向的岛），视觉面积是
/// 外 - 洞 + 岛，而符号累加会得到别的数——所以它只作为"面积是否退化的快速判据"
/// 与测试辅助，**不**用来当渲染的正确性依据。
pub fn contour_area_with_rule(contours: &[Contour], rule: FillRule) -> f32 {
    match rule {
        FillRule::NonZero => contours
            .iter()
            .filter(|c| c.is_fillable())
            .map(|c| c.signed_area2() * 0.5)
            .sum(),
        FillRule::EvenOdd => {
            // 按嵌套深度奇偶加减。身份比较用**下标**而不是 `PartialEq`：两个形状
            // 完全相同的轮廓是不同实体，用值比较会把它们当成同一个。
            let usable: Vec<&Contour> = contours.iter().filter(|c| c.is_fillable()).collect();
            let mut total = 0.0;
            for (i, c) in usable.iter().enumerate() {
                let mut depth = 0usize;
                for (j, o) in usable.iter().enumerate() {
                    if i == j {
                        continue;
                    }
                    if abs_area(o.points()) > abs_area(c.points())
                        && c.points()
                            .iter()
                            .any(|p| inside(*p, std::slice::from_ref(*o), FillRule::EvenOdd))
                    {
                        depth += 1;
                    }
                }
                let sign = if depth % 2 == 0 { 1.0 } else { -1.0 };
                total += sign * abs_area(c.points());
            }
            total
        }
    }
}

/// 有向面积的两倍。空轮廓返回 0。
///
/// # 符号约定（用探针定标过，不是推断）
///
/// 标准 shoelace：`Σ (x_i · y_{i+1} − x_{i+1} · y_i)`，数学惯例下逆时针为正。
/// 在 **y 向下**的坐标系里，"视觉上的顺时针"对应这个式的**正**值
/// （实测：矩形 (0,0)(1,0)(1,1)(0,1) 得 `+2`）。
///
/// ⚠ 网上常见的 `(y_i − y_{i+1})(x_i + x_{i+1})` 变体在本坐标系下得 `−2`——
/// 它等价于 `−标准值`。两者的**符号完全相反**，混用会让耳切法的凸顶点判定
/// 整体反过来，症状是"切不出三角形"或"切出重叠的碎片"，而那极容易被误判成
/// 填充规则实现错了。所以全模块只认这一个函数，不在别处手写。
pub fn signed_area2(points: &[Point]) -> f32 {
    let n = points.len();
    if n < 3 {
        return 0.0;
    }
    let mut acc = 0.0f32;
    let mut prev = points[n - 1];
    for &p in points {
        acc += prev.x * p.y - p.x * prev.y;
        prev = p;
    }
    acc
}

/// 面积的绝对值（**已除以 2**，即真实面积）。
///
/// # 为什么不直接复用 [`signed_area2`]
///
/// `signed_area2` 返回的是两倍面积且带符号。拿它冒充"面积"会让每一个
/// `abs_area(...) == 144` 的断言实际在比 288 —— 症状是"覆盖率对得上、面积对不上"，
/// 而两份数据只差一个系数 2，极难定位。
///
/// 所以本模块的面积一律是**真实面积**（带符号的用 [`signed_area2`]），只有
/// 耳切法的凸顶点判定在用两倍值（那里判的是符号，系数无所谓）。
pub fn abs_area(points: &[Point]) -> f32 {
    signed_area2(points).abs() * 0.5
}

/// 面积绝对值低于此值视为退化。
///
/// 口径是**真实面积**（与 [`abs_area`] 一致，不是两倍面积），所以在 24 DIP 的
/// viewBox 里 `1e-9` 约等于边长 `6e-6`。
///
/// 不是零而是 `1e-9`：退化轮廓会让耳切法找不到任何耳从而**死循环**，而症状是
/// "整个进程卡住"而不是报错。宁可把极小的图标当作不可填充。
pub const AREA_EPS: f32 = 1e-9;

/// 两点是否相同（带容差）。
///
/// 容差用**相对**判据：绝对 `1e-6` 在 24 单位viewBox 下是 4e-8 比例，在
/// 4096 像素的大图上却是 2.5e-10 —— 后者会被浮点噪声轻易越过。
fn same_point(a: Point, b: Point) -> bool {
    let scale = a.x.abs().max(a.y.abs()).max(b.x.abs()).max(b.y.abs()).max(1.0);
    let eps = POINT_EPS * scale;
    (a.x - b.x).abs() <= eps && (a.y - b.y).abs() <= eps
}

/// [`same_point`] 的公开别名（供 [`flatten`] 去重时复用同一套判据）。
///
/// # 为什么要暴露
///
/// 展平时"跳过与前一点重合的点"和 [`Contour::new`] 的去重必须**用同一个容差**。
/// 两处各写一个`==` 或各定一个常量，症状是"某些图标有个多余的顶点"，
/// 而那会让耳切法在特定轮廓上切出重叠三角形。
pub fn same_point_public(a: Point, b: Point) -> bool {
    same_point(a, b)
}

/// 点的同一性容差（相对量纲）。
pub const POINT_EPS: f32 = 1e-6;

/// 把 [`Segment`] 序列里的坐标按 `transform` 变换。
///
/// # 为什么要先变换再展平，而不是展平后变换顶点
///
/// 展平的**弦高误差**是相对目标像素的。若先在 viewBox 空间展平再缩放，
/// 一个 24 单位viewBox 里误差 0.01 的折线放大到 2000 像素后误差变成 0.83 像素
/// —— 肉眼可见的多边形化。所以展平必须发生在**最终尺度**上。
pub fn transform_segments(
    segments: &[Segment],
    view_box: (f32, f32, f32, f32),
    dst: Rect,
) -> Vec<Segment> {
    let (vx, vy, vw, vh) = view_box;
    if vw <= 0.0 || vh <= 0.0 || dst.width <= 0.0 || dst.height <= 0.0 {
        return Vec::new();
    }
    // `meet` 语义：等比缩放后居中。图标不该被拉伸。
    let scale = (dst.width / vw).min(dst.height / vh);
    let tx = dst.x + (dst.width - vw * scale) * 0.5;
    let ty = dst.y + (dst.height - vh * scale) * 0.5;
    let map = |x: f32, y: f32| Point {
        x: (x - vx) * scale + tx,
        y: (y - vy) * scale + ty,
    };
    segments
        .iter()
        .map(|s| match *s {
            Segment::MoveTo(x, y) => {
                let p = map(x, y);
                Segment::MoveTo(p.x, p.y)
            }
            Segment::LineTo(x, y) => {
                let p = map(x, y);
                Segment::LineTo(p.x, p.y)
            }
            Segment::CubicTo(a, b, c, d, e, f) => {
                let (p0, p1, p2) = (map(a, b), map(c, d), map(e, f));
                Segment::CubicTo(p0.x, p0.y, p1.x, p1.y, p2.x, p2.y)
            }
            Segment::QuadraticTo(a, b, c, d) => {
                let (p0, p1) = (map(a, b), map(c, d));
                Segment::QuadraticTo(p0.x, p0.y, p1.x, p1.y)
            }
            Segment::Close => Segment::Close,
        })
        .collect()
}

/// 按填充规则判断某点是否在轮廓集合内部。
///
/// # 为什么要区分两种规则
///
/// - [`FillRule::NonZero`]：按**环绕方向累加**，非零即在内部。于是同向嵌套的两个
///   轮廓（齿轮的齿与轴）实心，而反向嵌套挖出洞。
/// - [`FillRule::EvenOdd`]：穿越**奇数次**即在内部。于是任意嵌套都交替实心/镂空，
///   与方向无关。
///
/// 图标里两种都会出现（齿轮是 NonZero 实心环，某些字体图标是 EvenOdd 挖洞），
/// 所以这条判据不是"高级功能"，而是"不实现就会画错一半图标"。
pub fn inside(point: Point, contours: &[Contour], rule: FillRule) -> bool {
    match rule {
        FillRule::NonZero => winding_number(point, contours) != 0,
        FillRule::EvenOdd => crossings(point, contours) % 2 == 1,
    }
}

/// 从 `point` 出发的射线（+x 方向）穿越轮廓边界的次数。
///
/// # 半开区间规则（`> y` 而不是 `>= y`）
///
/// 射线穿过**顶点**时会同时碰到两条边的端点。若两边都计数，穿越数会**多算 2**
/// （于是 EvenOdd 判定翻转）；若都不计，会**少算 2**。所以必须一边算一边不算：
/// `(yi > py) != (yj > py)` 恰好对共享端点的两条边给出相反结论。
///
/// 这是扫描线填充与点定位的标准做法，漏掉它的症状是"边界上的点归属随机"——
/// 在抗锯齿下表现为描边某一侧比另一侧粗。
pub fn crossings(point: Point, contours: &[Contour]) -> u32 {
    let mut n = 0u32;
    for c in contours {
        let p = c.points();
        let len = p.len();
        if len < 3 {
            continue;
        }
        let mut prev = p[len - 1];
        for &cur in p {
            if (prev.y > point.y) != (cur.y > point.y) {
                let t = (point.y - prev.y) / (cur.y - prev.y);
                let x = prev.x + t * (cur.x - prev.x);
                if x > point.x {
                    n += 1;
                }
            }
            prev = cur;
        }
    }
    n
}

/// 环绕数（signed crossing count）：向上穿越记 +1，向下记 −1。
pub fn winding_number(point: Point, contours: &[Contour]) -> i32 {
    let mut w = 0i32;
    for c in contours {
        let p = c.points();
        let len = p.len();
        if len < 3 {
            continue;
        }
        let mut prev = p[len - 1];
        for &cur in p {
            if (prev.y > point.y) != (cur.y > point.y) {
                let t = (point.y - prev.y) / (cur.y - prev.y);
                let x = prev.x + t * (cur.x - prev.x);
                if x > point.x {
                    w += if cur.y > prev.y { 1 } else { -1 };
                }
            }
            prev = cur;
        }
    }
    w
}

/// 两点的叉积（z 分量）。
pub fn cross(o: Point, a: Point, b: Point) -> f32 {
    (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x)
}

/// 点 `p` 是否在三角形 `abc` 内（含边界）。
///
/// # 为什么是"含边界"
///
/// 耳切法要判断"有没有别的顶点落在这个候选耳里"。若用**不含**边界，则共线顶点
/// 会被判为在外面，于是算法切出一个"其实还含有顶点"的耳——结果是多边形被切成
/// 了互相重叠的三角形，画面上是图标某一角多出一块颜色相近的碎片。
pub fn point_in_triangle(p: Point, a: Point, b: Point, c: Point) -> bool {
    let d1 = cross(a, b, p);
    let d2 = cross(b, c, p);
    let d3 = cross(c, a, p);
    let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(neg && pos)
}

/// 线段 `ab` 与线段 `cd` 是否**严格**相交（不含端点接触与共线重叠）。
///
/// 耳切法里用来判"折线是否自交"与"耳是否被别处顶点侵入"。严格相交判定是
/// 必需的：把"顶点正好落在别人的边上"也算自交，会让本该合法的轮廓被判为自交。
pub fn segments_properly_intersect(a: Point, b: Point, c: Point, d: Point) -> bool {
    let d1 = cross(a, b, c);
    let d2 = cross(a, b, d);
    let d3 = cross(c, d, a);
    let d4 = cross(c, d, b);
    if ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0))
        && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
    {
        return true;
    }
    false
}

/// 折线是否自交（任意两条非相邻边严格相交）。
///
/// 自交的轮廓不能靠耳切法三角化——耳切法假设多边形是**简单多边形**，
/// 而自交轮廓的"内部"本身就没有良定义。检出后**放弃**比切出错形状好：
/// 画面上少一个图标，而不是多一块看不懂的碎片。
pub fn is_self_intersecting(points: &[Point]) -> bool {
    let n = points.len();
    if n < 4 {
        return false;
    }
    for i in 0..n {
        let a1 = points[i];
        let a2 = points[(i + 1) % n];
        // 跳过相邻边（共享端点）
        for j in i + 1..n {
            if j == i || (j + 1) % n == i || (i + 1) % n == j {
                continue;
            }
            let b1 = points[j];
            let b2 = points[(j + 1) % n];
            if segments_properly_intersect(a1, a2, b1, b2) {
                return true;
            }
        }
    }
    false
}
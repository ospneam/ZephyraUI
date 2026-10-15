//! 三角化：把带孔多边形变成三角形。
//!
//! # 算法：earcut（mapbox，v3.0.1）的逐函数对译
//!
//! 参考实现是 JavaScript 的 [`earcut`](https://github.com/mapbox/earcut)，它本身
//! 是 Isamu Toshinari 的「耳切法」在带孔多边形上的经典实现。本文件是它的
//! Rust 对译，**逐函数对应**，不做结构压缩：
//!
//! | earcut | 本文件 |
//! |---|---|
//! | `linkedList` | [`Earcut::linked_list`] |
//! | `filterPoints` | [`Earcut::filter_points`] |
//! | `earcutLinked` | [`Earcut::earcut_linked`] |
//! | `isEar` / `isEarHashed` | [`Earcut::is_ear`] / [`Earcut::is_ear_hashed`] |
//! | `indexCurve` / `sortLinked` / `zOrder` | [`Earcut::index_curve`] / [`Earcut::sort_linked`] / [`z_order`] |
//! | `cureLocalIntersections` | [`Earcut::cure_local_intersections`] |
//! | `splitEarcut` / `isValidDiagonal` | [`Earcut::split_earcut`] / [`Earcut::is_valid_diagonal`] |
//! | `eliminateHoles` / `eliminateHole` | [`Earcut::eliminate_holes`] / [`Earcut::eliminate_hole`] |
//! | `findHoleBridge` | [`Earcut::find_hole_bridge`] |
//! | `splitPolygon` | [`Earcut::split_polygon`] |
//! | `insertNode` / `removeNode` | [`Earcut::insert_node`] / [`Earcut::remove_node`] |
//!
//! 唯一改动是把「ptr_of」换成「`Vec` 下标」。这是机械替换：`prev`/`next`/`prev_z`/
//! `next_z` 四个字段的语义与更新时机完全照原版，节点被 [`Earcut::remove_node`]
//! 摘掉后**留在 arena 里不复用**（与原版的 GC 行为一致），所以下标始终有效。
//!
//! # 为什么不用扫线分解（Skia `GrTriangulator` 的做法）
//!
//! 扫线（monotone decomposition）也是正确路线，Chrome/Skia 的 GPU 路径在用。
//! 本项目没走它，是因为它要求**逐顶点精确重建活跃边表与区带归属**
//! （`fLeftPoly`/`fRightPoly`/`fFirstEdgeAbove`/`fPartner` 互相咬合），一旦在移植
//! 时"顺手简化"其中任何一环，症状都是"连三角形都切不出来"且极难定位。
//! earcut 的状态是**单条链表**，全部状态就是这个链表本身，没有跨结构的隐式约束。
//!
//! # 三层兜底（与原版一致）
//!
//! 找不到耳时 earcut 依次尝试：① 清洗共线点重切 → ② 局部修复自交 → ③ 找合法
//! 对角线把多边形劈成两半各自切。三层都失败就放弃剩余部分（本文件记为
//! [`TriMeshError::Partial`]）。**任何一层都不重试**，所以不存在死循环。
//!
//! # 填充规则在哪一层生效
//!
//! earcut 本身只有"外轮廓 + 一组孔"这一个概念。非零 / 奇偶的区分在本文件里
//! 更上一层完成（[`triangulate`] 的分组阶段）：哪些轮廓是孔、哪些是独立外轮廓，
//! 由 [`super::winding_number`] / [`super::crossings`] 的语义决定。分好组之后
//! 每组内部是纯几何问题，交给 earcut。

use super::{abs_area, contour_area_with_rule, inside, Contour, TriangulateError};
use crate::core::Point;
use crate::icon::FillRule;
use crate::paint::Vertex;

/// 「无此节点」的下标哨兵。
const NONE: usize = usize::MAX;

/// 顶点总数超过此值时启用 z-order 加速（与 earcut 的 `80 * dim` 同值）。
///
/// z-order 把耳测试里"遍历所有顶点找侵入者"从 O(n) 降到期望 O(1)，代价是每帧
/// 一次 O(n log n) 排序。小多边形上排序反而更慢，所以设阈值。
const ZORDER_THRESHOLD: usize = 80;

/// 共线判据的相对容差。
///
/// 只用于 [`Earcut::filter_points`]。耳测试里的符号判据**必须**用精确值
/// （`>= 0` / `< 0` 是拓扑判断，容差会改变哪些角被当成凸角），唯独"这个顶点
/// 是否恰好落在前后的直线上"是纯几何问题，浮点噪声会让本该被清掉的顶点留下，
/// 进而让耳切在退化输入上找不到耳。
const COLLINEAR_EPS: f32 = 1e-6;

/// 三角化产物：顶点列表（每 3 个一个三角形）。
///
/// # 为什么直接给 `Vertex` 而不是 `Vec<[Point; 3]>`
///
/// 因为它立刻就是 [`crate::paint::Command::Path`] 的 `vertices` 字段。中间表示层
/// 在这里没有第二个消费者——多一层就多一处"两边约定漂移"的可能，而那类 bug 的
/// 症状是"画面整体错位"且不报错。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TriMesh {
    /// 三角形顶点，每 3 个一组。
    pub vertices: Vec<Vertex>,
    /// 三角形个数（= `vertices.len() / 3`）。
    pub triangles: usize,
}

impl TriMesh {
    fn push_tri(&mut self, a: Point, b: Point, c: Point) {
        let mk = |p: Point| Vertex {
            position: p,
            // 路径填充不用纹理，uv 恒为零。填零而不是留未初始化：顶点缓冲是
            // `Pod`，未初始化会让 MSAA 边缘采到垃圾。
            uv: Point { x: 0.0, y: 0.0 },
        };
        self.vertices.push(mk(a));
        self.vertices.push(mk(b));
        self.vertices.push(mk(c));
        self.triangles += 1;
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.triangles == 0
    }
}

/// 把带孔轮廓集合三角化。
///
/// # 参数
///
/// - `contours`：任意条闭合轮廓，外轮廓与孔混在一起 —— 谁是孔由 `rule` 与几何
///   关系决定，不由参数顺序或书写顺序决定。
/// - `rule`：[`FillRule::NonZero`] / [`FillRule::EvenOdd`]。二者只在"分组"这一步
///   有区别：分组之后每组是"一条外轮廓 + 若干孔"，交给 earcut 的部分完全共用。
///
/// # 返回值
///
/// 失败时返回已成功切出的部分与原因（[`TriMeshError::Partial`]），这样上层能把
/// "部分成功"画出来（少一块好过整块消失），同时又有诊断手段。
///
/// # 完整性判据
///
/// 每个"外轮廓 + 孔"分组切完后，用**解析面积**核对：`|Σ|三角形面积| − 解析面积|`
/// 超过 0.5% + 0.05 绝对量就记为 [`TriMeshError::Partial`]。这个判据抓的是
/// "三角形数量看着对但盖不全"——那种情况在自交或数值退化的输入上真的会发生，
/// 而只数三角形个数是抓不到的。
pub fn triangulate(contours: &[Contour], rule: FillRule) -> Result<TriMesh, TriMeshError> {
    let mut mesh = TriMesh::default();
    let usable: Vec<&Contour> = contours
        .iter()
        .filter(|c| c.is_fillable() && !super::is_self_intersecting(c.points()))
        .collect();
    if usable.is_empty() {
        return Err(TriMeshError::NoUsableContour);
    }

    // ── ① 分组：确定哪些轮廓是外轮廓、哪些是它的孔。
    //
    // 用**下标**做身份比较，而不是 `PartialEq`：两个形状完全相同的轮廓（比如齿轮的
    // 两段重复弧）在值比较下会被判成同一个，于是其中一个被误认成孔、最终少一块。
    //
    // 父级取"包含它且面积最小"的那条（最紧的那层），保证孔直接挂到它所属的那一层
    // 而不是挂到某个巨大的祖先上。
    let mut groups: Vec<Group> = Vec::new();
    let mut group_of: Vec<Option<usize>> = vec![None; usable.len()];

    for (ci, c) in usable.iter().enumerate() {
        let parent_idx = usable
            .iter()
            .enumerate()
            .filter(|(oi, o)| *oi != ci && encloses(o, c, rule))
            .min_by(|(_, a), (_, b)| {
                abs_area(a.points())
                    .partial_cmp(&abs_area(b.points()))
                    .unwrap_or(core::cmp::Ordering::Equal)
            })
            .map(|(i, _)| i);

        // 非零规则下"与父级同向"的嵌套**不是孔**：环绕数叠加（1+1=2≠0）仍然填充。
        // 这不是可以"统一按孔处理"的细节——按孔处理会把实心的齿轮轴挖空。
        //
        // 但它也不能直接丢掉：它内部**可能还有孔**（一个实心的环，环心里挖掉一块）。
        // 所以让它成为**独立的一组**，它自己的孔照常挂上来。填充结果与"并成一块"
        // 完全一致（并集），但孔的归属是对的。
        if rule == FillRule::NonZero {
            if let Some(pi) = parent_idx {
                if super::signed_area2(usable[pi].points()).signum()
                    == super::signed_area2(c.points()).signum()
                {
                    group_of[ci] = Some(groups.len());
                    groups.push(Group {
                        outer: c.points().to_vec(),
                        holes: Vec::new(),
                    });
                    continue;
                }
            }
        }

        match parent_idx.and_then(|pi| group_of[pi]) {
            Some(g) => groups[g].holes.push(c.points().to_vec()),
            None => {
                group_of[ci] = Some(groups.len());
                groups.push(Group {
                    outer: c.points().to_vec(),
                    holes: Vec::new(),
                });
            }
        }
    }

    // ── ② 每组独立交给 earcut。
    let mut degraded = false;
    for g in &groups {
        let expected = abs_area(&g.outer) - g.holes.iter().map(|h| abs_area(h)).sum::<f32>();
        let tris = earcut_triangulate(&g.outer, &g.holes);
        let actual: f32 = tris.iter().map(|t| tri_area(t)).sum();
        // 相对 0.5% + 绝对 0.05：前者抓"盖不全一大片"，后者抓"小图标差一点点"。
        // 定得太紧会把浮点累积误差误判成退化（症状：一个本该正常的图标消失）。
        let tol = expected.abs() * 0.005 + 0.05;
        if (actual - expected).abs() > tol {
            degraded = true;
        }
        for t in tris {
            mesh.push_tri(t[0], t[1], t[2]);
        }
    }

    if mesh.is_empty() {
        return Err(TriMeshError::NoUsableEar);
    }
    if degraded {
        return Err(TriMeshError::Partial { mesh });
    }
    Ok(mesh)
}

/// 一个"外轮廓 + 若干孔"。
struct Group {
    outer: Vec<Point>,
    holes: Vec<Vec<Point>>,
}

fn tri_area(t: &[Point; 3]) -> f32 {
    super::signed_area2(t).abs() * 0.5
}

/// 对译 earcut 的顶层入口。
///
/// 对应 `earcut(data, holeIndices, dim)`。原版把外轮廓与孔摊平成一个坐标数组
/// 加上孔的起始下标；本项目的输入天然就是 `Vec<Vec<Point>>`，省掉那层展平。
fn earcut_triangulate(outer: &[Point], holes: &[Vec<Point>]) -> Vec<[Point; 3]> {
    let mut ec = Earcut::default();
    ec.run(outer, holes)
}

/// 错误类型。
///
/// # 为什么 `Partial` 也要携带 mesh
///
/// 因为"部分成功"比"全失败"有用得多：画面上少一个图标，用户能看出来；
/// 而整个图标消失会被当成 bug 上报。同时它仍然是一个 `Err`，调用方**必须**
/// 显式决定怎么处理——沉默地吞掉错误正是这类几何代码最容易犯的错。
#[derive(Debug, Clone, PartialEq)]
pub enum TriMeshError {
    /// 所有轮廓都退化或自交。
    NoUsableContour,
    /// 找不到可切的耳（拓扑退化）。
    NoUsableEar,
    /// 部分轮廓失败，但其余已切出。
    Partial {
        /// 已成功切出的部分。
        mesh: TriMesh,
    },
}

impl TriMeshError {
    /// 已成功切出的网格（`None` 表示完全没有产出）。
    pub fn mesh(&self) -> Option<&TriMesh> {
        match self {
            TriMeshError::Partial { mesh } => Some(mesh),
            _ => None,
        }
    }
}

impl From<TriMeshError> for TriangulateError {
    fn from(e: TriMeshError) -> Self {
        TriangulateError(e.to_string())
    }
}

impl core::fmt::Display for TriMeshError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            TriMeshError::NoUsableContour => write!(f, "没有可用轮廓（全部退化或自交）"),
            TriMeshError::NoUsableEar => write!(f, "找不到可切的耳"),
            TriMeshError::Partial { mesh } => {
                write!(f, "部分轮廓退化（{} 个三角形可用）", mesh.triangles)
            }
        }
    }
}

/// `a` 是否包住 `b`（面积更大且 `b` 的某个顶点在 `a` 内）。
///
/// # 为什么用"任一顶点在内"而不是"全部顶点在内"
///
/// 退化输入里被包含的轮廓可能有一个顶点恰好压在父轮廓边上（描边的内缩环就会
/// 这样）。要求"全部在内"会把这种明显的孔判成外轮廓，于是孔不挖、面积多一块。
/// "任一在内"+面积更小这个组合在实践中更稳。
fn encloses(a: &Contour, b: &Contour, _rule: FillRule) -> bool {
    if abs_area(a.points()) <= abs_area(b.points()) {
        return false;
    }
    b.points()
        .iter()
        .any(|p| inside(*p, std::slice::from_ref(a), FillRule::NonZero))
}

// ============================================================================
// earcut 对译
// ============================================================================

/// earcut 的链表节点。
///
/// # 四个链表字段缺一不可
///
/// `prev`/`next` 是**多边形环**，`prev_z`/`next_z` 是 **z-order 环**。后者只在
/// 顶点数超过 [`ZORDER_THRESHOLD`] 时启用。earcut 原版把两者放在同一个对象里
/// 同步维护（`removeNode` 同时摘两条链），这里照做——把 z 序另开一个平行数组
/// 看着"更干净"，但 `removeNode` 就要在两处各写一遍摘链逻辑，两处不同步就是一个
/// 越界读。
#[derive(Debug, Clone, Copy)]
struct Node {
    /// 坐标。
    p: Point,
    /// 原始点序号。`split_polygon` 复制出的节点沿用同一个 `i`，于是
    /// "坐标相同"与"同一个顶点"在需要区分时能被 [`Earcut::intersects_polygon`]
    /// 这类判据分辨开。
    i: usize,
    prev: usize,
    next: usize,
    /// z-order 曲线值，0 表示"尚未计算"。
    z: u32,
    prev_z: usize,
    next_z: usize,
    /// 只有一个点的"孔"（steiner 点），不能被 [`Earcut::filter_points`] 清掉。
    steiner: bool,
}

/// 递归深度上限。
///
/// earcut 用 `pass`（0/1/2）保证终止，但 [`Earcut::split_earcut`] 会递归调用
/// `earcut_linked(pass=0)`，理论上可以无限深（每次劈开都产生新的 pass 0 调用）。
/// 真实输入不会这么 adversarial，但栈溢出是进程崩溃、没有诊断信息的那类失败，
/// 所以这里显式封顶：超限就放弃剩余部分（记为 `Partial`）。
const MAX_DEPTH: u32 = 64;

#[derive(Debug, Default)]
struct Earcut {
    /// 节点 arena。摘掉的节点保留在原地（不复用），下标因此永久有效。
    nodes: Vec<Node>,
    /// 原始点序号计数器（供 [`Node::i`] 用，只需"相等即同一"）。
    next_index: usize,
    /// 三角形输出。
    tris: Vec<[Point; 3]>,
    /// z-order 的 bbox 与缩放（`inv_size == 0` 表示未启用）。
    min_x: f32,
    min_y: f32,
    inv_size: f32,
}

impl Earcut {
    // ---------------------------------------------------------------- 入口

    /// 对应 `earcut()`。
    fn run(&mut self, outer: &[Point], holes: &[Vec<Point>]) -> Vec<[Point; 3]> {
        let Some(outer_node) = self.linked_list(outer, true) else {
            return Vec::new();
        };
        // 外轮廓退化成一个点或一条线段。
        if self.nodes[outer_node].next == self.nodes[outer_node].prev {
            return Vec::new();
        }

        let outer_node = if holes.is_empty() {
            outer_node
        } else {
            self.eliminate_holes(outer_node, holes)
        };

        // z-order 只在大多边形上启用。bbox 覆盖全部输入点（含孔）——比原版多算了
        // 孔，但只是把坐标压进 15 位整数时更准一点，不影响正确性。
        let total = outer.len() + holes.iter().map(|h| h.len()).sum::<usize>();
        self.inv_size = 0.0;
        if total > ZORDER_THRESHOLD {
            let (mut min_x, mut min_y) = (f32::INFINITY, f32::INFINITY);
            let (mut max_x, mut max_y) = (f32::NEG_INFINITY, f32::NEG_INFINITY);
            for p in outer.iter().chain(holes.iter().flatten()) {
                min_x = min_x.min(p.x);
                min_y = min_y.min(p.y);
                max_x = max_x.max(p.x);
                max_y = max_y.max(p.y);
            }
            self.min_x = min_x;
            self.min_y = min_y;
            let span = (max_x - min_x).max(max_y - min_y);
            self.inv_size = if span != 0.0 { 32767.0 / span } else { 0.0 };
        }

        self.earcut_linked(outer_node, 0, 0);
        core::mem::take(&mut self.tris)
    }

    // ---------------------------------------------------------------- 建链表

    /// 对应 `linkedList()`。
    ///
    /// `clockwise` 表示"期望的绕向"；实现按需正序或逆序插入，于是**输入的书写
    /// 方向不影响结果**（SVG 规范就是这样要求的）。
    fn linked_list(&mut self, pts: &[Point], clockwise: bool) -> Option<usize> {
        if pts.is_empty() {
            return None;
        }
        let mut last = NONE;
        if clockwise == (signed_area(pts) > 0.0) {
            for &p in pts {
                last = self.insert_node(p, last);
            }
        } else {
            for &p in pts.iter().rev() {
                last = self.insert_node(p, last);
            }
        }
        if last == NONE {
            return None;
        }
        // 首尾重合：摘掉一个（earcut 原样）。
        if self.equals(last, self.nodes[last].next) {
            self.remove_node(last);
            last = self.nodes[last].next;
        }
        // 全部点重合 → 摘完只剩一个自环节点，无从三角化。
        if self.nodes[last].next == last {
            return None;
        }
        Some(last)
    }

    /// 对应 `insertNode()`。
    fn insert_node(&mut self, p: Point, last: usize) -> usize {
        let i = self.next_index;
        self.next_index += 1;
        let idx = self.nodes.len();
        self.nodes.push(Node {
            p,
            i,
            prev: idx,
            next: idx,
            z: 0,
            prev_z: NONE,
            next_z: NONE,
            steiner: false,
        });
        if last != NONE {
            let ln = self.nodes[last].next;
            self.nodes[idx].next = ln;
            self.nodes[idx].prev = last;
            self.nodes[ln].prev = idx;
            self.nodes[last].next = idx;
        }
        idx
    }

    /// 对应 `removeNode()`。节点留在 arena 里，但两条链上都被摘掉。
    fn remove_node(&mut self, p: usize) {
        let (pv, nx) = (self.nodes[p].prev, self.nodes[p].next);
        self.nodes[nx].prev = pv;
        self.nodes[pv].next = nx;
        let (pz, nz) = (self.nodes[p].prev_z, self.nodes[p].next_z);
        if pz != NONE {
            self.nodes[pz].next_z = nz;
        }
        if nz != NONE {
            self.nodes[nz].prev_z = pz;
        }
    }

    // ---------------------------------------------------------------- 清洗

    /// 对应 `filterPoints()`：清掉重合点与共线点。
    fn filter_points(&mut self, start: usize, end_opt: usize) -> usize {
        if start == NONE {
            return NONE;
        }
        let mut end = if end_opt == NONE { start } else { end_opt };
        let mut p = start;
        loop {
            let mut again = false;
            let (pv, nx) = (self.nodes[p].prev, self.nodes[p].next);
            if !self.nodes[p].steiner
                && (self.equals(p, nx) || self.area_rel(pv, p, nx) == 0.0)
            {
                self.remove_node(p);
                end = self.nodes[p].prev;
                p = end;
                if p == self.nodes[p].next {
                    break;
                }
                again = true;
            } else {
                p = self.nodes[p].next;
            }
            if !(again || p != end) {
                break;
            }
        }
        end
    }

    // ---------------------------------------------------------------- 主循环

    /// 对应 `earcutLinked()`。
    ///
    /// `pass`：0 = 正常切；1 = 清洗后重切；2 = 局部修复自交后再切，此时若还失败
    /// 就走 [`Earcut::split_earcut`]。
    fn earcut_linked(&mut self, mut ear: usize, pass: u32, depth: u32) {
        if ear == NONE || depth > MAX_DEPTH {
            return;
        }
        if pass == 0 && self.inv_size != 0.0 {
            self.index_curve(ear);
        }

        let mut stop = ear;
        while self.nodes[ear].prev != self.nodes[ear].next {
            let prev = self.nodes[ear].prev;
            let next = self.nodes[ear].next;

            let is_ear = if self.inv_size != 0.0 {
                self.is_ear_hashed(ear)
            } else {
                self.is_ear(ear)
            };

            if is_ear {
                self.push_tri(prev, ear, next);
                self.remove_node(ear);
                // 跳过下一个顶点 → 少产生细长三角形（earcut 的经验法则）。
                ear = self.nodes[next].next;
                stop = ear;
                continue;
            }

            ear = next;

            if ear == stop {
                // 绕了一圈还找不到耳：按 pass 逐级升级。
                if pass == 0 {
                    let e = self.filter_points(ear, NONE);
                    self.earcut_linked(e, 1, depth + 1);
                } else if pass == 1 {
                    let cleaned = self.filter_points(ear, NONE);
                    let e = self.cure_local_intersections(cleaned);
                    self.earcut_linked(e, 2, depth + 1);
                } else if pass == 2 {
                    self.split_earcut(ear, depth + 1);
                }
                break;
            }
        }
    }

    fn push_tri(&mut self, a: usize, b: usize, c: usize) {
        self.tris.push([self.nodes[a].p, self.nodes[b].p, self.nodes[c].p]);
    }

    // ---------------------------------------------------------------- 耳测试

    /// 对应 `isEar()`。
    fn is_ear(&self, ear: usize) -> bool {
        let (a, b, c) = (self.nodes[ear].prev, ear, self.nodes[ear].next);
        // 凹角不能作耳。判据用**精确**面积：这是拓扑判断，容差会改变哪些角算凸。
        if self.area(a, b, c) >= 0.0 {
            return false;
        }

        let (pa, pb, pc) = (self.nodes[a].p, self.nodes[b].p, self.nodes[c].p);
        let (x0, y0, x1, y1) = bbox3(pa, pb, pc);

        let mut p = self.nodes[c].next;
        while p != a {
            let pp = self.nodes[p].p;
            if pp.x >= x0
                && pp.x <= x1
                && pp.y >= y0
                && pp.y <= y1
                && point_in_triangle_except_first(pa, pb, pc, pp)
                && self.area(self.nodes[p].prev, p, self.nodes[p].next) >= 0.0
            {
                return false;
            }
            p = self.nodes[p].next;
        }
        true
    }

    /// 对应 `isEarHashed()`：只在 z-order 环上找侵入者。
    fn is_ear_hashed(&self, ear: usize) -> bool {
        let (a, b, c) = (self.nodes[ear].prev, ear, self.nodes[ear].next);
        if self.area(a, b, c) >= 0.0 {
            return false;
        }

        let (pa, pb, pc) = (self.nodes[a].p, self.nodes[b].p, self.nodes[c].p);
        let (x0, y0, x1, y1) = bbox3(pa, pb, pc);
        let min_z = self.z_of(x0, y0);
        let max_z = self.z_of(x1, y1);

        // 双向扫：z 落在 [min_z, max_z] 区间的节点才可能是侵入者。
        let mut p = self.nodes[ear].prev_z;
        let mut n = self.nodes[ear].next_z;
        while p != NONE && self.nodes[p].z >= min_z && n != NONE && self.nodes[n].z <= max_z {
            if self.z_is_candidate(p, pa, pb, pc, a, c, x0, y0, x1, y1) {
                return false;
            }
            p = self.nodes[p].prev_z;
            if self.z_is_candidate(n, pa, pb, pc, a, c, x0, y0, x1, y1) {
                return false;
            }
            n = self.nodes[n].next_z;
        }
        while p != NONE && self.nodes[p].z >= min_z {
            if self.z_is_candidate(p, pa, pb, pc, a, c, x0, y0, x1, y1) {
                return false;
            }
            p = self.nodes[p].prev_z;
        }
        while n != NONE && self.nodes[n].z <= max_z {
            if self.z_is_candidate(n, pa, pb, pc, a, c, x0, y0, x1, y1) {
                return false;
            }
            n = self.nodes[n].next_z;
        }
        true
    }

    /// z-order 版耳测试里对单个候选节点的判定（bbox 已由调用方保证）。
    #[allow(clippy::too_many_arguments)]
    fn z_is_candidate(
        &self,
        p: usize,
        pa: Point,
        pb: Point,
        pc: Point,
        a: usize,
        c: usize,
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
    ) -> bool {
        let pp = self.nodes[p].p;
        pp.x >= x0
            && pp.x <= x1
            && pp.y >= y0
            && pp.y <= y1
            && p != a
            && p != c
            && point_in_triangle_except_first(pa, pb, pc, pp)
            && self.area(self.nodes[p].prev, p, self.nodes[p].next) >= 0.0
    }

    // ---------------------------------------------------------------- z-order

    fn z_of(&self, x: f32, y: f32) -> u32 {
        z_order(x, y, self.min_x, self.min_y, self.inv_size)
    }

    /// 对应 `indexCurve()`：把多边形环的节点另建一条按 z 排序的双向链。
    fn index_curve(&mut self, start: usize) {
        let mut p = start;
        loop {
            if self.nodes[p].z == 0 {
                let v = self.nodes[p].p;
                self.nodes[p].z = self.z_of(v.x, v.y);
            }
            self.nodes[p].prev_z = self.nodes[p].prev;
            self.nodes[p].next_z = self.nodes[p].next;
            p = self.nodes[p].next;
            if p == start {
                break;
            }
        }
        // 砍成**线性**链：start 没有前驱，它的前驱没有后继。
        //
        // # 这里必须是 start 的"前驱"而不是 start 自己
        //
        // z 序链如果留成环形（也就是仍等于多边形环），`is_ear_hashed` 里
        // `n = n.next_z` 会绕着多边形环一直走下去 —— z 值有升有降，条件
        // `z <= max_z` 总能再次成立，于是**死循环**。症状是某个点数超过
        // [`ZORDER_THRESHOLD`] 的图标（如 gear）把进程挂住。
        let pv = self.nodes[p].prev_z;
        if pv != NONE {
            self.nodes[pv].next_z = NONE;
        }
        self.nodes[p].prev_z = NONE;
        self.sort_linked(p);
    }

    /// 对应 `sortLinked()`：Simon Tatham 的链表归并排序。
    ///
    /// 为什么自己写而不用 `sort_unstable_by_key`：这里是**就地**改 `prev_z`/`next_z`
    /// 两个字段的重排，把节点抽到 `Vec` 里排再写回要多一次分配与一次间接，
    /// 而这个函数每个大多边形调一次、每次 O(n log n) 次内层迭代。
    fn sort_linked(&mut self, list: usize) -> usize {
        let mut in_size = 1usize;
        let mut list = list;
        loop {
            let mut p = list;
            let mut tail = NONE;
            list = NONE;
            let mut num_merges = 0usize;

            while p != NONE {
                num_merges += 1;
                let mut q = p;
                let mut p_size = 0usize;
                for _ in 0..in_size {
                    p_size += 1;
                    q = self.nodes[q].next_z;
                    if q == NONE {
                        break;
                    }
                }
                let mut q_size = in_size;

                while p_size > 0 || (q_size > 0 && q != NONE) {
                    let e;
                    if p_size != 0
                        && (q_size == 0
                            || q == NONE
                            || self.nodes[p].z <= self.nodes[q].z)
                    {
                        e = p;
                        p = self.nodes[p].next_z;
                        p_size -= 1;
                    } else {
                        e = q;
                        q = self.nodes[q].next_z;
                        q_size -= 1;
                    }
                    if tail != NONE {
                        self.nodes[tail].next_z = e;
                    } else {
                        list = e;
                    }
                    self.nodes[e].prev_z = tail;
                    tail = e;
                }
                p = q;
            }

            if tail != NONE {
                self.nodes[tail].next_z = NONE;
            }
            in_size *= 2;
            if num_merges <= 1 {
                break;
            }
        }
        list
    }

    // ---------------------------------------------------------------- 兜底一

    /// 对应 `cureLocalIntersections()`：局部切掉自交的"小耳朵"。
    fn cure_local_intersections(&mut self, start: usize) -> usize {
        if start == NONE {
            return NONE;
        }
        let mut p = start;
        loop {
            let pn = self.nodes[p].next;
            let (a, b) = (self.nodes[p].prev, self.nodes[pn].next);
            if !self.equals(a, b)
                && self.intersects(a, p, pn, b)
                && self.locally_inside(a, b)
                && self.locally_inside(b, a)
            {
                self.push_tri(a, p, b);
                self.remove_node(p);
                self.remove_node(pn);
                p = b;
            }
            p = self.nodes[p].next;
            if p == start {
                break;
            }
        }
        self.filter_points(p, NONE)
    }

    // ---------------------------------------------------------------- 兜底二

    /// 对应 `splitEarcut()`：找一条合法对角线，把多边形劈成两半分别切。
    fn split_earcut(&mut self, start: usize, depth: u32) {
        if depth > MAX_DEPTH {
            return;
        }
        let mut a = start;
        loop {
            let an = self.nodes[a].next;
            let mut b = self.nodes[an].next;
            while b != self.nodes[a].prev {
                if self.nodes[a].i != self.nodes[b].i && self.is_valid_diagonal(a, b) {
                    let c = self.split_polygon(a, b);
                    // 注意：`a.next` / `c.next` 必须在 `filter_points` **之前**读出，
                    // 因为那是传给它的 `end` 参数，会在调用中被改掉。
                    let an = self.nodes[a].next;
                    let a = self.filter_points(a, an);
                    let cn = self.nodes[c].next;
                    let c = self.filter_points(c, cn);
                    self.earcut_linked(a, 0, depth + 1);
                    self.earcut_linked(c, 0, depth + 1);
                    return;
                }
                b = self.nodes[b].next;
            }
            a = self.nodes[a].next;
            if a == start {
                break;
            }
        }
    }

    /// 对应 `isValidDiagonal()`。
    fn is_valid_diagonal(&self, a: usize, b: usize) -> bool {
        self.nodes[a].next != b
            && self.nodes[a].prev != b
            && !self.intersects_polygon(a, b)
            && ((self.locally_inside(a, b)
                && self.locally_inside(b, a)
                && self.middle_inside(a, b)
                // 两个 `area` 是"非零即真"的短路，不是"大于零"。
                && (self.area(self.nodes[a].prev, a, self.nodes[b].prev) != 0.0
                    || self.area(a, self.nodes[b].prev, b) != 0.0))
                || (self.equals(a, b)
                    && self.area(self.nodes[a].prev, a, self.nodes[a].next) > 0.0
                    && self.area(self.nodes[b].prev, b, self.nodes[b].next) > 0.0))
    }

    // ---------------------------------------------------------------- 消孔

    /// 对应 `eliminateHoles()`。
    fn eliminate_holes(&mut self, outer_node: usize, holes: &[Vec<Point>]) -> usize {
        let mut queue: Vec<usize> = Vec::with_capacity(holes.len());
        for h in holes {
            let Some(list) = self.linked_list(h, false) else {
                continue;
            };
            // 单点孔：标成 steiner，清洗时不能删。
            if self.nodes[list].next == list {
                self.nodes[list].steiner = true;
            }
            queue.push(self.get_leftmost(list));
        }
        // 从左到右消孔：靠左的孔先桥，后面的孔就不会被"已经改写过的环"挡住。
        queue.sort_by(|&a, &b| self.compare_xy_slope(a, b));

        let mut outer = outer_node;
        for h in queue {
            outer = self.eliminate_hole(h, outer);
        }
        outer
    }

    /// 对应 `compareXYSlope()`。
    ///
    /// 两个孔的最左点重合时，用"出边斜率"让它们**逆时针**排序，于是找到的桥总是
    /// 恰好落在两孔相接的那个公共顶点上。
    fn compare_xy_slope(&self, a: usize, b: usize) -> core::cmp::Ordering {
        let (pa, pb) = (self.nodes[a].p, self.nodes[b].p);
        let c = pa.x.total_cmp(&pb.x);
        if c != core::cmp::Ordering::Equal {
            return c;
        }
        let c = pa.y.total_cmp(&pb.y);
        if c != core::cmp::Ordering::Equal {
            return c;
        }
        let sa = self.slope(a);
        let sb = self.slope(b);
        sa.total_cmp(&sb)
    }

    /// 出边斜率。竖直边（`dx == 0`）给 `INFINITY`：JavaScript 里是 ±Infinity，
    /// 排序行为未定义，取一个确定值即可——它只在"两孔最左点完全重合"时被用到。
    fn slope(&self, m: usize) -> f32 {
        let p = self.nodes[m].p;
        let n = self.nodes[self.nodes[m].next].p;
        let dx = n.x - p.x;
        if dx == 0.0 {
            f32::INFINITY
        } else {
            (n.y - p.y) / dx
        }
    }

    /// 对应 `eliminateHole()`：给 `hole` 找一条桥接进 `outer`，然后清洗切口。
    fn eliminate_hole(&mut self, hole: usize, outer_node: usize) -> usize {
        let Some(bridge) = self.find_hole_bridge(hole, outer_node) else {
            return outer_node;
        };
        let bridge_reverse = self.split_polygon(bridge, hole);

        let bn = self.nodes[bridge_reverse].next;
        self.filter_points(bridge_reverse, bn);
        // `bridge.next` 要在上一句**之后**读：filter_points 会改动它。
        let bn = self.nodes[bridge].next;
        self.filter_points(bridge, bn)
    }

    /// 对应 `findHoleBridge()`：David Eberly 的孔-外轮廓桥接算法。
    ///
    /// 从孔的**最左点**向左发射水平射线，找被它穿过的第一条边，取该边上 x 较小
    /// 的端点作候选；再用"切线角最小"原则在孔的顶点里挑一个更好的连接点，
    /// 并要求该点对孔**局部可见**（否则桥会从孔内部穿出去）。
    fn find_hole_bridge(&self, hole: usize, outer_node: usize) -> Option<usize> {
        let h = self.nodes[hole].p;
        let (hx, hy) = (h.x, h.y);
        let mut qx = f32::NEG_INFINITY;
        let mut m = NONE;

        let mut p = outer_node;
        loop {
            if self.equals(hole, p) {
                return Some(p);
            }
            let pn = self.nodes[p].next;
            if self.equals(hole, pn) {
                return Some(pn);
            }
            let (pp, ppn) = (self.nodes[p].p, self.nodes[pn].p);
            if hy <= pp.y && hy >= ppn.y && ppn.y != pp.y {
                let x = pp.x + (hy - pp.y) * (ppn.x - pp.x) / (ppn.y - pp.y);
                if x <= hx && x > qx {
                    qx = x;
                    m = if pp.x < ppn.x { p } else { pn };
                    if x == hx {
                        // 孔正好贴在外轮廓上：取 x 较小的端点。
                        return Some(m);
                    }
                }
            }
            p = pn;
            if p == outer_node {
                break;
            }
        }

        if m == NONE {
            return None;
        }

        // 在 (孔最左点, 交点, 射线交点) 构成的三角形里找孔顶点，
        // 取切线角最小者 = "|hy - p.y| / (hx - p.x)" 最小。
        let stop = m;
        let mp = self.nodes[m].p;
        let (mx, my) = (mp.x, mp.y);
        let mut tan_min = f32::INFINITY;

        let mut p = m;
        loop {
            let pp = self.nodes[p].p;
            if hx >= pp.x
                && pp.x >= mx
                && hx != pp.x
                && point_in_triangle(
                    if hy < my { hx } else { qx },
                    hy,
                    mx,
                    my,
                    if hy < my { qx } else { hx },
                    hy,
                    pp.x,
                    pp.y,
                )
            {
                let tan = (hy - pp.y).abs() / (hx - pp.x);
                let better = if tan < tan_min {
                    true
                } else if tan == tan_min {
                    pp.x > mx
                        || (pp.x == mx && self.sector_contains_sector(m, p))
                } else {
                    false
                };
                if better && self.locally_inside(p, hole) {
                    m = p;
                    tan_min = tan;
                }
            }
            p = self.nodes[p].next;
            if p == stop {
                break;
            }
        }

        if m == NONE {
            None
        } else {
            Some(m)
        }
    }

    /// 对应 `sectorContainsSector()`。
    fn sector_contains_sector(&self, m: usize, p: usize) -> bool {
        self.area(self.nodes[m].prev, m, self.nodes[p].prev) < 0.0
            && self.area(self.nodes[p].next, m, self.nodes[m].next) < 0.0
    }

    /// 对应 `getLeftmost()`。
    fn get_leftmost(&self, start: usize) -> usize {
        let mut p = start;
        let mut leftmost = start;
        loop {
            let (a, b) = (self.nodes[p].p, self.nodes[leftmost].p);
            if a.x < b.x || (a.x == b.x && a.y < b.y) {
                leftmost = p;
            }
            p = self.nodes[p].next;
            if p == start {
                break;
            }
        }
        leftmost
    }

    /// 对应 `splitPolygon()`。
    ///
    /// 若 `a`/`b` 分属两条环 → 把孔并进外环（消孔）；若同属一条环 → 把多边形
    /// 劈成两条环（[`Earcut::split_earcut`]）。同一段代码服务两种用途，
    /// 这正是 earcut 结构紧凑的关键。
    fn split_polygon(&mut self, a: usize, b: usize) -> usize {
        let a2 = self.nodes.len();
        self.nodes.push(Node {
            p: self.nodes[a].p,
            i: self.nodes[a].i,
            prev: a2,
            next: a2,
            z: 0,
            prev_z: NONE,
            next_z: NONE,
            steiner: false,
        });
        self.next_index += 1;
        let b2 = self.nodes.len();
        self.nodes.push(Node {
            p: self.nodes[b].p,
            i: self.nodes[b].i,
            prev: b2,
            next: b2,
            z: 0,
            prev_z: NONE,
            next_z: NONE,
            steiner: false,
        });
        self.next_index += 1;

        let an = self.nodes[a].next;
        let bp = self.nodes[b].prev;

        self.nodes[a].next = b;
        self.nodes[b].prev = a;

        self.nodes[a2].next = an;
        self.nodes[an].prev = a2;

        self.nodes[b2].next = a2;
        self.nodes[a2].prev = b2;

        self.nodes[bp].next = b2;
        self.nodes[b2].prev = bp;

        b2
    }

    // ---------------------------------------------------------------- 几何判据

    /// 对应 `area(p, q, r)`：`(q-p) × (r-q)`。
    fn area(&self, p: usize, q: usize, r: usize) -> f32 {
        let (a, b, c) = (self.nodes[p].p, self.nodes[q].p, self.nodes[r].p);
        (b.y - a.y) * (c.x - b.x) - (b.x - a.x) * (c.y - b.y)
    }

    /// 带相对容差的 `area`，**只**用于共线判据。见 [`COLLINEAR_EPS`]。
    fn area_rel(&self, p: usize, q: usize, r: usize) -> f32 {
        let (a, b, c) = (self.nodes[p].p, self.nodes[q].p, self.nodes[r].p);
        let t1 = (b.y - a.y) * (c.x - b.x);
        let t2 = (b.x - a.x) * (c.y - b.y);
        let scale = t1.abs() + t2.abs();
        if (t1 - t2).abs() <= COLLINEAR_EPS * scale.max(1.0) {
            0.0
        } else {
            t1 - t2
        }
    }

    /// 对应 `equals()`：精确相等。
    ///
    /// 这里**故意不用** [`super::same_point_public`] 的相对容差：earcut 的多条判据
    /// 依赖"这两个节点是不是同一个顶点"，而用容差会把桥接复制的节点误判成原节点
    /// （症状：孔被重复消一次，面积多出一块）。
    fn equals(&self, a: usize, b: usize) -> bool {
        let (p, q) = (self.nodes[a].p, self.nodes[b].p);
        p.x == q.x && p.y == q.y
    }

    /// 对应 `intersectsPolygon()`。
    fn intersects_polygon(&self, a: usize, b: usize) -> bool {
        let mut p = a;
        loop {
            let pn = self.nodes[p].next;
            if self.nodes[p].i != self.nodes[a].i
                && self.nodes[pn].i != self.nodes[a].i
                && self.nodes[p].i != self.nodes[b].i
                && self.nodes[pn].i != self.nodes[b].i
                && self.intersects(p, pn, a, b)
            {
                return true;
            }
            p = pn;
            if p == a {
                break;
            }
        }
        false
    }

    /// 对应 `locallyInside()`：`b` 是否在 `a` 的内角扇形内。
    fn locally_inside(&self, a: usize, b: usize) -> bool {
        if self.area(self.nodes[a].prev, a, self.nodes[a].next) < 0.0 {
            self.area(a, b, self.nodes[a].next) >= 0.0
                && self.area(a, self.nodes[a].prev, b) >= 0.0
        } else {
            self.area(a, b, self.nodes[a].prev) < 0.0 || self.area(a, self.nodes[a].next, b) < 0.0
        }
    }

    /// 对应 `middleInside()`：线段 `a-b` 的中点是否在多边形内（射线法）。
    fn middle_inside(&self, a: usize, b: usize) -> bool {
        let (pa, pb) = (self.nodes[a].p, self.nodes[b].p);
        let px = (pa.x + pb.x) / 2.0;
        let py = (pa.y + pb.y) / 2.0;
        let mut inside = false;
        let mut p = a;
        loop {
            let pn = self.nodes[p].next;
            let (pp, ppn) = (self.nodes[p].p, self.nodes[pn].p);
            if (pp.y > py) != (ppn.y > py)
                && ppn.y != pp.y
                && px < (ppn.x - pp.x) * (py - pp.y) / (ppn.y - pp.y) + pp.x
            {
                inside = !inside;
            }
            p = pn;
            if p == a {
                break;
            }
        }
        inside
    }

    /// 对应 `intersects()`：两线段是否相交（含共线重叠与端点落在另一条上）。
    fn intersects(&self, p1: usize, q1: usize, p2: usize, q2: usize) -> bool {
        let o1 = sign(self.area(p1, q1, p2));
        let o2 = sign(self.area(p1, q1, q2));
        let o3 = sign(self.area(p2, q2, p1));
        let o4 = sign(self.area(p2, q2, q1));

        if o1 != o2 && o3 != o4 {
            return true;
        }
        if o1 == 0 && on_segment(self.nodes[p1].p, self.nodes[p2].p, self.nodes[q1].p) {
            return true;
        }
        if o2 == 0 && on_segment(self.nodes[p1].p, self.nodes[q2].p, self.nodes[q1].p) {
            return true;
        }
        if o3 == 0 && on_segment(self.nodes[p2].p, self.nodes[p1].p, self.nodes[q2].p) {
            return true;
        }
        if o4 == 0 && on_segment(self.nodes[p2].p, self.nodes[q1].p, self.nodes[q2].p) {
            return true;
        }
        false
    }
}

// ============================================================================
// 自由函数（earcut 里不依赖链表状态的纯几何判据）
// ============================================================================

/// 对应 `signedArea()`：鞋带公式的等价形式（省一次减法）。
fn signed_area(pts: &[Point]) -> f32 {
    let n = pts.len();
    if n < 3 {
        return 0.0;
    }
    let mut sum = 0.0;
    for i in 0..n {
        let j = (i + n - 1) % n;
        sum += (pts[j].x - pts[i].x) * (pts[i].y + pts[j].y);
    }
    sum
}

/// 对应 `pointInTriangle()`：点是否在凸三角形内（**含边界**）。
#[allow(clippy::too_many_arguments)]
fn point_in_triangle(
    ax: f32,
    ay: f32,
    bx: f32,
    by: f32,
    cx: f32,
    cy: f32,
    px: f32,
    py: f32,
) -> bool {
    (cx - px) * (ay - py) >= (ax - px) * (cy - py)
        && (ax - px) * (by - py) >= (bx - px) * (ay - py)
        && (bx - px) * (cy - py) >= (cx - px) * (by - py)
}

fn point_in_triangle_except_first(a: Point, b: Point, c: Point, p: Point) -> bool {
    if a.x == p.x && a.y == p.y {
        return false;
    }
    point_in_triangle(a.x, a.y, b.x, b.y, c.x, c.y, p.x, p.y)
}

/// 对应 `onSegment()`：共线三点里 `q` 是否落在 `pr` 上。
fn on_segment(p: Point, q: Point, r: Point) -> bool {
    q.x <= p.x.max(r.x)
        && q.x >= p.x.min(r.x)
        && q.y <= p.y.max(r.y)
        && q.y >= p.y.min(r.y)
}

fn sign(v: f32) -> i32 {
    if v > 0.0 {
        1
    } else if v < 0.0 {
        -1
    } else {
        0
    }
}

fn bbox3(a: Point, b: Point, c: Point) -> (f32, f32, f32, f32) {
    (
        a.x.min(b.x).min(c.x),
        a.y.min(b.y).min(c.y),
        a.x.max(b.x).max(c.x),
        a.y.max(b.y).max(c.y),
    )
}

/// 对应 `zOrder()`：把坐标压进 15 位整数后交织成一条单调曲线。
///
/// 作用是让"空间上靠近的点在 z 序上也靠近"，于是耳测试沿 z 序找侵入者时
/// 大部分节点能被 z 区间直接排除，期望复杂度从 O(n) 降到 O(1)。
fn z_order(x: f32, y: f32, min_x: f32, min_y: f32, inv_size: f32) -> u32 {
    #[inline]
    fn part(v: u32) -> u32 {
        let mut x = v;
        x = (x | (x << 8)) & 0x00FF_00FF;
        x = (x | (x << 4)) & 0x0F0F_0F0F;
        x = (x | (x << 2)) & 0x3333_3333;
        x = (x | (x << 1)) & 0x5555_5555;
        x
    }
    let xi = ((x - min_x) * inv_size) as i64;
    let yi = ((y - min_y) * inv_size) as i64;
    let xi = (xi.clamp(0, 0x7FFF)) as u32;
    let yi = (yi.clamp(0, 0x7FFF)) as u32;
    part(xi) | (part(yi) << 1)
}

/// 便捷入口：Segment → 顶点。
///
/// 这是本模块面向调用方的**主要**接口：SVG 路径进来，三角化顶点出去。
///
/// `tessellate` 是"填充"的入口；要描边请先 [`super::flatten`] 再对每条轮廓调
/// [`super::stroke_outline`]，最后合并 [`triangulate`] —— 描边的轮廓集与填充的
/// 轮廓集在三角化器眼里没有区别。
pub fn tessellate(
    segments: &[crate::icon::Segment],
    view_box: (f32, f32, f32, f32),
    dst: crate::core::Rect,
    rule: FillRule,
) -> Result<TriMesh, TriMeshError> {
    let segs = super::transform_segments(segments, view_box, dst);
    let contours = super::flatten(&segs, super::TOLERANCE);
    triangulate(&contours, rule)
}

/// 按填充规则算多边形面积（供测试断言"面积守恒"）。
pub fn area_of(contours: &[Contour], rule: FillRule) -> f32 {
    contour_area_with_rule(contours, rule)
}
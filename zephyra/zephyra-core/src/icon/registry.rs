//! 图标注册表：名字 → 已解析的 [`Icon`]。
//!
//! # 为什么要注册表而不是直接调 `parse_svg`
//!
//! 两个原因，都是硬约束：
//!
//! 1. **解析一次。** 图标是静态内容。若每次 `view()` 都调解析器，一个 8 按钮的
//!    工具栏每帧要跑 8 次全命令集 path 解析——这是纯浪费，且随图标复杂度线性增长。
//! 2. **身份必须稳定。** 栅格化缓存以图标数据的**地址**为键（非 `'static` 的图标
//!    可能在栈上，函数返回后地址被复用，另一个图标就会命中它的缓存条目）。
//!    `OnceLock` 保证了每个名字恰好对应一个 `'static` 地址。
//!
//! # 谁定义哪些图标
//!
//! [`assets`] 里是**框架自带**的通用字形（菜单、画笔、撤销…），任何应用都能用。
//! 应用专属的图标（相机、图片库、批注）由应用自己 [`IconSet::with`] 追加——
//! 框架不该知道 Viewstage 有个"拍照"按钮，反过来也不该让每个应用自带一份
//! `menu.svg`。

use std::collections::HashMap;
use std::sync::OnceLock;

use super::{parse_path_data, parse_svg, Icon};

/// 未知图标名的兜底，进程内唯一。
///
/// 必须是**同一个** `static` 而不是每次构造一个新的 `Icon::EMPTY`——缓存键取的是
/// 数据地址，而"所有未知名字共享同一个空图标"正是我们想要的：一个未知名字只占
/// 一个（永远不会被填上的）缓存条目。
static EMPTY: Icon = Icon::EMPTY;

/// 图标源。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconSource {
    /// 一份图标级 SVG 文本（通常来自 [`crate::icon::assets`] 的内嵌常量）。
    Svg(&'static str),
    /// 裸 path data（无 SVG 包装），`viewBox` 按 `0 0 24 24` 视为。
    ///
    /// 与 [`IconSource::Svg`] 走**同一个**解析器，所以只有少一层 XML 提取的差别，
    /// 形状语义完全一致。
    Path(&'static str),
}

impl IconSource {
    /// 解析成 [`Icon`]。
    fn parse(self) -> Result<Icon, String> {
        match self {
            IconSource::Svg(text) => parse_svg(text).map_err(|e| e.to_string()),
            IconSource::Path(d) => parse_path_data(d)
                .map(|segments| Icon {
                    view_box: (0.0, 0.0, 24.0, 24.0),
                    paint: super::Paint::CURRENT_COLOR,
                    segments,
                })
                .map_err(|e| e.to_string()),
        }
    }
}

/// 图标注册表。
///
/// 构造后 [`IconSet::get`] 在构造完成前只会返回 [`Icon::EMPTY`]；正常用法是先
/// [`IconSet::new`] + 若干 [`IconSet::with`]，再 [`IconSet::freeze`]。
///
/// # 必须放在 `static` 里
///
/// [`IconSet::get`] 返回 `&'static Icon`，所以 `self` 也必须是 `'static`——
/// 这是**刻意**的约束，不是签名上的省事。
///
/// 宿主侧的栅格化缓存以图标数据的**地址**为键（比字符串哈希便宜，且
/// `include_str!` 的地址天然稳定）。若图标能放在栈上或随 `IconSet` 一起析构，
/// 函数返回后地址就被下一个图标复用，于是 A 的缓存条目会被 B 命中——
/// 表现为"某个图标显示成另一个图标"，且不报任何错。
///
/// 把 `&'static self` 写进签名，这条不变量就从"文档里的叮嘱"变成了编译期事实。
pub struct IconSet {
    entries: Vec<(&'static str, IconSource)>,
    parsed: OnceLock<HashMap<&'static str, Icon>>,
    /// 三角化结果缓存。见 [`GeometryCache`]。
    geometry: GeometryCache,
}

/// 三角化结果的缓存键。
///
/// # 为什么键里是**地址**而不是图标名
///
/// 与 [`IconSet::get`] 的 `'static` 约束同源：地址比字符串哈希便宜，而且
/// `include_str!` 的地址天然稳定。同一段源码地址在进程内唯一 —— 于是"两个名字
/// 指向同一图标资源"会自动共享缓存条目，而那本来就是同一份工作。
#[derive(PartialEq, Eq, Hash)]
struct GeometryKey {
    /// 图标数据的地址（`'static`，见 [`IconSet`] 的类型文档）。
    icon: usize,
    /// 目标矩形的四个分量，各自的**位模式**。
    ///
    /// ⚠ 位模式而非数值：`Rect` 的分量参与映射，而 `20.0` 与
    /// `20.0000001` 映射出的顶点**逐位不同**。按数值近似比较会让后者命中前者
    /// 的缓存，于是图标差半个亚像素 —— 那在 1× 下看不见，在 2× 下看得见。
    rect: [u32; 4],
}

/// 三角化结果缓存。
///
/// # 为什么需要它
///
/// 展平（曲线 → 折线）与三角化（多孔耳切）的结果**只**取决于（图标，目标矩形），
/// 与时间无关 —— 而 [`crate::paint`] 每帧为每个图标节点重算一遍。
///
/// 实测（`demo --bench`，8 个图标节点）：**50 µs/个/帧**，占 `paint` 阶段
/// （603 µs）的 66%，整帧的 39%。
///
/// # 我当初的判断错在哪
///
/// [`crate::paint::icon_command`] 的文档里写着"实测每帧成本只有几十微秒……远低于
/// 缓存的复杂度与失效风险，所以现在不做"。那个"几十微秒"是**估**的，不是量的，
/// 而它恰好就在"值不值得"的边界上 —— 于是决定被做错了。
///
/// 教训与本 crate 反复出现的同一条：**性能判断必须来自计时，不能来自估算**。
/// 估算在边界附近出错时最贵，因为它是唯一一种看起来有依据的猜测。
///
/// # 为什么容量是 128
///
/// 键是「图标 × 目标矩形」的组合。稳态下活跃组合数 = 不同图标数 × 不同尺寸数，
/// 一个典型界面是十几个图标 × 两三种尺寸 = 几十条。128 留了余量，而上界必须有：
/// 一个让矩形连续变化的界面（拖拽缩放、动画）会无界增长，那比不缓存更糟。
struct GeometryCache {
    map: std::sync::Mutex<HashMap<GeometryKey, CachedMesh>>,
}

/// 缓存里的一条三角化结果。
///
/// # 为什么存 `Arc<[Vertex]>` 而不是 `Arc<TriMesh>`
///
/// 消费者（[`crate::paint::Command::Path`] 的 `vertices` 字段）只拿顶点，
/// 三角形**计数**它自己按 `len() / 3` 算。而 `Arc<TriMesh>` → `Arc<[Vertex]>`
/// 的转换要**重新分配并 memcpy 整个数组**（胖指针得重建）—— 于是"命中缓存"
/// 反而成了最贵的那条路，与缓存的目的正好相反。
///
/// `triangles` 保留为诊断字段：它不参与渲染，但让"缓存里的东西与刚算出来的
/// 不一致"这件事在测试里可断言（而不只是"画出来看起来一样"）。
#[derive(Clone)]
struct CachedMesh {
    vertices: std::sync::Arc<[crate::paint::Vertex]>,
    triangles: usize,
}

impl GeometryCache {
    const CAPACITY: usize = 128;

    fn new() -> Self {
        Self {
            map: std::sync::Mutex::new(HashMap::new()),
        }
    }

    fn get(&self, key: &GeometryKey) -> Option<CachedMesh> {
        self.map.lock().ok()?.get(key).cloned()
    }

    fn put(&self, key: GeometryKey, value: CachedMesh) {
        let Ok(mut map) = self.map.lock() else {
            return;
        };
        // 满了就整表清空：清空的最坏后果是"下一帧全部重算"，与没有缓存同价，
        // 而 LRU 要在**每次命中**时改表 —— 为一个可能永不发生的场景付每帧成本
        // 不划算（同 [`crate::text`] 里的排版缓存，理由一致）。
        if map.len() >= Self::CAPACITY {
            map.clear();
        }
        map.insert(key, value);
    }
}

impl Default for IconSet {
    fn default() -> Self {
        Self::new()
    }
}

impl IconSet {
    /// 三角化结果（带缓存）。
    ///
    /// [`crate::paint::icon_command`] 每帧为每个图标节点调它。同一个
    /// （图标，目标矩形）组合的结果**逐次相同**，所以第二次起走缓存。
    ///
    /// 返回 `None` 表示"这个组合没缓存过" —— 调用方算完用 [`Self::cache_mesh`]
    /// 存回。拆成两个方法而不是一个"给我或算它"，是因为算它需要展平容差与填充
    /// 规则，而那两项是调用方的参数选择，不该藏在一个"方便"方法里。
    pub(crate) fn cached_mesh(
        &'static self,
        icon: &'static Icon,
        rect: crate::core::Rect,
    ) -> Option<(std::sync::Arc<[crate::paint::Vertex]>, usize)> {
        self.geometry
            .get(&Self::geometry_key(icon, rect))
            .map(|m| (m.vertices, m.triangles))
    }

    /// 存一条三角化结果，返回可直接放进命令里的 `Arc`。
    pub(crate) fn cache_mesh(
        &'static self,
        icon: &'static Icon,
        rect: crate::core::Rect,
        mesh: crate::geom::TriMesh,
    ) -> std::sync::Arc<[crate::paint::Vertex]> {
        // `Vec` → `Arc<[T]>` 是**一次**分配加拷贝，之后每帧只加 refcount。
        // 反过来（`Arc<[T]>` → `TriMesh`）就得多一次分配。
        let vertices: std::sync::Arc<[crate::paint::Vertex]> = mesh.vertices.into();
        self.geometry.put(
            Self::geometry_key(icon, rect),
            CachedMesh {
                vertices: std::sync::Arc::clone(&vertices),
                triangles: mesh.triangles,
            },
        );
        vertices
    }

    /// 键的构造单独一处 —— 查找与存回必须用**同一个**键，两处分头写就是一条
    /// "缓存永不命中"的静默故障（症状：性能回去了，功能全对，没有任何报错）。
    fn geometry_key(icon: &'static Icon, rect: crate::core::Rect) -> GeometryKey {
        GeometryKey {
            icon: icon as *const Icon as usize,
            rect: [
                rect.x.to_bits(),
                rect.y.to_bits(),
                rect.width.to_bits(),
                rect.height.to_bits(),
            ],
        }
    }

    /// 空注册表。
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            parsed: OnceLock::new(),
            geometry: GeometryCache::new(),
        }
    }

    /// 以框架自带字形为基础的注册表。
    ///
    /// [`crate::icon::assets`] 的全部条目。应用在此之上追加自己的图标。
    pub fn with_builtin() -> Self {
        let mut set = Self::new();
        for (name, source) in super::assets::REGISTRY {
            set = set.with(name, *source);
        }
        set
    }

    /// 登记一个图标。
    ///
    /// 同一个名字重复登记时**后者覆盖前者**——而不是取第一个。那样更符合
    /// "追加覆盖"的直觉，也让应用可以覆盖框架自带的字形（比如换个更符合
    /// 品牌规范的 `menu`）而不必先删。
    pub fn with(mut self, name: &'static str, source: IconSource) -> Self {
        match self.entries.iter_mut().find(|(n, _)| *n == name) {
            Some(slot) => slot.1 = source,
            None => self.entries.push((name, source)),
        }
        self
    }

    /// 固化：此刻解析全部登记项。
    ///
    /// 可选——不调用的话 [`IconSet::get`] 会自己固化。但显式调用把"解析发生在哪里"
    /// 交给应用决定：放在启动路径上，解析耗时就落在启动里而不是第一帧。
    pub fn freeze(&self) {
        self.parsed.get_or_init(|| self.build());
    }

    /// 已登记的名字（顺序即登记顺序）。
    ///
    /// 给"我要哪些图标"的校验用：某个名字不在这里，就说明它没被登记，
    /// 渲染时会静默变成空白——那是最难查的一类问题。
    pub fn names(&self) -> Vec<&'static str> {
        self.entries.iter().map(|(n, _)| *n).collect()
    }

    /// 按名字取图标；未知名字回落到 [`Icon::EMPTY`]。
    pub fn get(&'static self, name: &str) -> &'static Icon {
        let map = self.parsed.get_or_init(|| self.build());
        map.get(name).unwrap_or(&EMPTY)
    }

    fn build(&self) -> HashMap<&'static str, Icon> {
        let mut map = HashMap::with_capacity(self.entries.len());
        for (name, source) in &self.entries {
            match source.parse() {
                Ok(icon) => {
                    map.insert(*name, icon);
                }
                Err(err) => {
                    // 解析失败**不静默**。debug 下打印，release 下也不吞掉——
                    // 因为后果是"某个图标不显示"，而没有任何其他症状会指向它。
                    //
                    // 这里选择 panic 而非跳过：注册表是编译期常量，解析失败意味着
                    // 某个写死的 SVG 资源坏了，那属于必须当场暴露的构建缺陷，
                    // 而不是可以靠"少一个图标"蒙混过去的运行时状况。
                    panic!("[zephyra_core::icon] 解析失败 name={name} err={err}");
                }
            }
        }
        map
    }
}
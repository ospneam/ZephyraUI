//! 字形图集：CPU 侧的位图仓库。
//!
//! # 为什么在 core 而不是 base
//!
//! 图集常被误认为"GPU 资源"。它不是 —— 它是一块 `Vec<u8>` 加一张"哪个字形在哪个
//! 位置"的表，**GPU 只在最后一步 upload 一次**。把它留在 core 有两个直接好处：
//!
//! - 打包逻辑（边界检查、uv 计算、shelf 换行）能在纯 CPU 上断言，不需要 GPU 也不
//!   需要字体文件 —— 本文件的 8 条断言就是这么写的；
//! - 换字体栈时图集逻辑不必跟着搬。
//!
//! # 打包策略：shelf packer
//!
//! 横排货架：字形从左到右排，放不下就换行。**新字形只往后追加** —— 已放置的
//! 字形位置不动，所以取用拿到的 uv 在下次淘汰之前都稳定。
//!
//! # 每个字形四周留一圈透明像素
//!
//! 采样器是**双线性**的（`D3D11_FILTER_MIN_MAG_MIP_LINEAR`）。一个字形占据图集
//! 里 `[x, x+w)` 这些像素，而片元在四边形最外沿采样时，采样点落在
//! `u = x / ATLAS_SIZE` 上 —— 换算回纹素坐标是 `x - 0.5`，正好在纹素 `x-1` 与 `x`
//! 之间，于是**各取一半**。
//!
//! 没有留白时，纹素 `x-1` 属于**左边那个字形**，那半个邻居的墨就会渗进来：
//! 字与字之间出现淡淡的竖条、圆角处出现鬼影。所以每个字形实际占
//! `(w + 2 × GLYPH_PADDING) × (h + 2 × GLYPH_PADDING)`，墨写在中间，四周留透明。
//!
//! 一圈（`GLYPH_PADDING = 1`）对双线性**恰好够**：越界的采样最多摸到紧邻的一格。
//! ⚠ 若将来给图集加 mipmap，这个值必须跟着变大 —— mip 层级越低采样足迹越宽，
//! 1 像素的留白挡不住。当前纹理是 `MipLevels: 1`（见各后端的建图集处）。
//!
//! # 满了就按 LRU 淘汰
//!
//! 图集是一块固定大小的画布，`ATLAS_SIZE²`。装满之后再来新字形时：
//!
//! 1. 把已有条目按**最近使用时间**排序；
//! 2. 从最久未用的开始丢，直到「剩下的 + 新字形」能放下；
//! 3. 清空画布，按**高度降序**把幸存者重新排一遍，再放新字形。
//!
//! 重排之所以可行，是因为每个条目**留着自己的覆盖率位图**。代价是这块冗余内存
//! 约等于"图集已用区域的面积"，对 1024² 最多 1MB —— 换来的是不必重新栅格化
//! （那要访问字体，而图集是 CPU 侧的纯数据仓库，不该依赖字体栈）。
//!
//! ⚠ **重排会让所有幸存的 uv 变化。** 所以在一次 `glyph_quads` 里必须先把所有
//! 字形都放进去、再统一读 uv（见 `TextRender::glyph_quads` 的两趟写法）。边放边读
//! 会让先读到的 uv 指向已经被别人占用的位置 —— 画面上是「几个字换成了别的字」，
//! 而不是崩溃。
//!
//! 淘汰是**按需**的：只有真的放不下才动，且一次只丢到刚好够。正常 UI 的可见字符集
//! 远小于1K²，所以稳态下这段代码不会被执行。
//!
//! # 淘汰策略可替换：[`EvictionPolicy`]
//!
//! 「丢谁、按什么顺序重新摆」是一个**决策**，与「位图像素怎么写」无关。所以它被抽成
//! trait：core 只定义契约（[`EvictionPolicy`] + [`EvictPlan`]），默认实现是现在这套
//! LRU（[`LruEviction`]），宿主可以整体替换。
//!
//! 替换动机不是"想写得不一样"，而是两种真实需求：
//!
//! - **按可见性淘汰**：弹窗打开时，正在被遮挡的背景文字没必要留在图集里；
//! - **不清屏重排**：现策略每次淘汰都`pixels.fill(0)` 再逐字形重画，于是**所有**
//!   幸存者的 uv 都变了。逐帧都换字串的场景（时钟每秒跳一次）会因此每秒重传一次
//!   整张 4MB 图集。一个"只丢不排"的策略能避开重传，代价是空间利用率下降。
//!
//! # 契约的关键设计：策略返回**最终重排顺序**，而不是只返回"丢哪些"
//!
//! 原先`fits`（干跑「放得下吗」）与 `rebuild`（真的重排）各自写了一遍排序，靠一行
//! 注释警告它们必须一致。漂移的后果极其恶劣：`fits` 判得下、`rebuild` 放不下，而
//! 那时画布**已经被清空** —— 图集变成一片空白，且没有任何报错。
//!
//! 现在 [`EvictPlan::survivors`] 携带的就是重排顺序本身，两处共用同一个顺序源。
//! 顺序一旦由策略给出，图集就不可能"用另一套顺序"去摆放 —— 漂移在类型层面被消灭，
//! 而不是靠注释提醒。
//!
//! 图集在落盘前会**独立复验**一次（[`GlyphAtlas::make_room`] 里的二次`fits`）。
//! 策略是外部代码，可能给出放不下的顺序；那时宁可放弃这次淘汰，也不能拿已清空的
//! 画布去赌一把。

use std::collections::HashMap;

use crate::core::{Point, Rect, Size};

/// 图集边长（像素）。取 2 的幂：GPU 上的寻址与采样更快，且便于分块压缩。
pub const ATLAS_SIZE: u32 = 1024;

/// 每个字形四周留出的透明像素数。
///
/// 存在的理由是双线性采样会摸到紧邻的一格 —— 完整推导见模块文档。
///
/// ⚠ 给图集加 mipmap 时必须一并调大。
pub const GLYPH_PADDING: u32 = 1;

/// 字形在缓存里的键：`(font_id, glyph_id, px_size)`。
///
/// 三者缺一不可：同一码位在不同字体、或同一字形在不同像素字号下都是**不同的位图**。
///
/// 是公开类型而不是私有别名，因为 [`EvictionPolicy`] 的实现必须能引用它 ——
/// 策略要决定丢哪些条目，就得有办法指名一个条目。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GlyphKey {
    /// 字体 id（见 [`crate::text::GlyphSource::font_id`]）。
    pub font_id: u32,
    /// 字形在该字体内的 id。
    pub glyph_id: u32,
    /// 像素字号。
    pub px_size: u32,
}

impl GlyphKey {
    /// 三元组构造。
    pub const fn new(font_id: u32, glyph_id: u32, px_size: u32) -> Self {
        Self {
            font_id,
            glyph_id,
            px_size,
        }
    }
}

impl From<(u32, u32, u32)> for GlyphKey {
    fn from(v: (u32, u32, u32)) -> Self {
        Self::new(v.0, v.1, v.2)
    }
}

/// 淘汰策略要看的**一个条目的只读快照**。
///
/// 刻意只暴露决策所需的字段，不给`&mut` 也不给像素：策略要回答的是"丢谁、怎么重排"，
/// 与"像素怎么写"无关。给它 `&mut [u8]` 只会诱使人写出越界写这类跑不掉的逻辑。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntryInfo {
    /// 条目的键。
    pub key: GlyphKey,
    /// 该条目的**占位**尺寸（含两侧留白）。空白字形为 `(0, 0)`。
    ///
    /// 用占位尺寸而不是墨的尺寸：策略要判断"腾出来的地方够不够"，而画布上占的
    /// 是含留白的那块。
    pub padded: (u32, u32),
    /// 最近一次被取用的序号（越大越新）。淘汰通常从最小的开始。
    pub last_used: u64,
}

/// 淘汰与重排的**决策结果**。
///
/// # 为什么 survivors 携带顺序
///
/// 它就是**重排时要照着放的顺序**。[`GlyphAtlas`] 的干跑与落盘都用这一个列表，
/// 所以"两处排序不一致"这件事在类型层面不可能发生 —— 详见模块文档。
///
/// 顺序的语义：先放哪个。默认实现按**高度降序**（高的先放能少浪费整行）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvictPlan {
    /// 幸存的条目，**已按重排顺序排好**。
    pub survivors: Vec<GlyphKey>,
    /// 被丢弃的条目数（诊断用，累加进 [`AtlasStats::evictions`]）。
    pub dropped: usize,
}

/// 字形图集满了之后"丢谁、按什么顺序重排"的决策。
///
/// # 边界：只决策，不碰像素
///
/// 实现**不得**触碰画布或条目位图 —— [`GlyphAtlas`] 会照 `plan` 执行落盘。理由是
/// 落盘那条路要处理留白、越界裁剪、`revision` 推进等一堆细节，让每个策略各写一遍
/// 必然漂移；而这些细节错了的后果是**画面出现随机噪点**，且只在触发淘汰后出现。
///
/// # 正确性要求
///
/// 返回的顺序**必须真的放得下**。图集在落盘前会独立复验一次，放不下就放弃这次
/// 淘汰 —— 所以违反这条不会损坏画布，但会白丢一批字形（它们仍在图集里，下一个
/// 字形会再触发一次淘汰）。
pub trait EvictionPolicy {
    /// 决策。`entries` 是当前全部条目（顺序**不保证**，实现不能依赖）。
    ///
    /// `need` 是待放入字形的**占位**尺寸（含留白）。`side` 是画布边长 —— 必须显式
    /// 传进来而不是读全局 [`ATLAS_SIZE`]：图集可以用 [`GlyphAtlas::with_size`] 缩小
    /// （测试正是靠这个把淘汰路径跑起来的），而策略若拿 1024 去算就会判错。
    ///
    /// 返回 `None` 表示淘汰到空都放不下 —— 那只有"单个字形比整张图集还大"或
    /// "图集小到离谱"两种情况。
    fn evict(&self, entries: &[EntryInfo], need: (u32, u32), side: u32) -> Option<EvictPlan>;
}

/// 默认策略：丢最久未用的，重排时高度降序。
///
/// # 为什么按 `last_used` 而不是别的
///
/// `last_used` 是**单调递增序号**而非挂钟时间：core 里不引入时钟，且测试要能**确定地**
/// 构造"谁更久没用过"—— 挂钟在测试里是不确定的。
///
/// # 为什么重排要按高度降序
///
/// shelf packer 按行切分。一行里放不下的会换到下一行，而换行时**这一行剩下的横向空间
/// 就永久浪费了**。所以先放高的，能让每一行都尽量填满。
///
/// # `dropped` 的算法：逐个丢，丢到刚好够
///
/// 不做"一次丢一半"那种粗粒度决策 —— 稳态下每次淘汰只丢 1 个，而丢得越少，
/// 下一帧需要重新栅格化的字形就越少。淘汰会让幸存者 uv 全部失效，那意味着宿主要
/// 重传整张图集，所以**丢得越少越好**。
#[derive(Debug, Clone, Copy, Default)]
pub struct LruEviction;

/// 一个条目是否**真的占画布**。
///
/// 空白字形（空格、零宽字符）有条目、有 `last_used`，但不占画布 —— 所以它们绝不能
/// 被算进"腾地方"的分母，否则会为了塞下一个字形而白丢一堆真实字形。
///
/// ⚠ 这一条很容易漏：`rebuild` 的重排里空白条目**不调用** `place`，而 `fits` 的干跑
/// 里也必须跳过它。两边不一致同样是"判得下、放不下"。
fn occupies_canvas(e: &EntryInfo) -> bool {
    e.padded.0 > 0 && e.padded.1 > 0
}

impl EvictionPolicy for LruEviction {
    fn evict(&self, entries: &[EntryInfo], need: (u32, u32), side: u32) -> Option<EvictPlan> {
        if need.0 == 0 || need.1 == 0 {
            // 空白字形不占画布，永远放得下 —— 不该触发任何淘汰。
            return Some(EvictPlan {
                survivors: Vec::new(),
                dropped: 0,
            });
        }

        // 最久未用的在前。淘汰从队首开始。
        //
        // ⚠ 必须**同时**排除空白字形：它们占着 `last_used`（会被取用，会刷新新鲜度），
        // 若留在排序里，一旦图集紧张就会先丢空格 —— 而丢空格腾不出任何空间。
        let mut order: Vec<&EntryInfo> = entries.iter().filter(|e| occupies_canvas(e)).collect();
        order.sort_by_key(|e| e.last_used);

        for drop_count in 0..=order.len() {
            let survivors = &order[drop_count..];
            if pack_fits(survivors, need, side) {
                return Some(EvictPlan {
                    survivors: repack_order(survivors),
                    dropped: drop_count,
                });
            }
        }
        None
    }
}

/// 重排顺序：高度降序，同高按宽度降序。
///
/// ⚠ 宽度也要降序而不是按键升序 —— 同高的字形先放宽的，能填满当前行；先放窄的会让
/// 宽的换到下一行，把当前行的余量浪费掉。
///
/// 稳定性的处理：完全同尺寸时用 `last_used` 兜底（而不是靠 `sort_by` 的稳定性），
/// 因为输入顺序不保证（来自 `HashMap` 迭代）—— 不加兜底的话，同一状态两次重排
/// 可能得到不同布局，于是宿主无法判断"内容是否真的变了"。
fn repack_order(entries: &[&EntryInfo]) -> Vec<GlyphKey> {
    let mut v: Vec<&EntryInfo> = entries.to_vec();
    v.sort_by(|a, b| {
        b.padded
            .1
            .cmp(&a.padded.1)
            .then(b.padded.0.cmp(&a.padded.0))
            .then(a.last_used.cmp(&b.last_used))
            .then(a.key.cmp(&b.key))
    });
    v.into_iter().map(|e| e.key).collect()
}

/// 干跑：`survivors` 按给定顺序放完，再放 `need`，能不能装下。
///
/// **纯函数**，不碰画布也不碰游标 —— 这是"先干跑再落盘"的前提：试错若直接改
/// `self`，失败回滚就要把画布和映射表都还原，而那是最容易写出"半还原"状态的地方。
///
/// `survivors` 的顺序**就是**将来落盘的顺序（由 [`repack_order`] 给出），
/// 所以这里判"放得下"与 [`GlyphAtlas::rebuild`] 实际摆放必然一致。
fn pack_fits(survivors: &[&EntryInfo], need: (u32, u32), side: u32) -> bool {
    let mut shelf = Shelf::new(side);
    for e in survivors {
        if shelf.place(e.padded.0, e.padded.1).is_none() {
            return false;
        }
    }
    shelf.place(need.0, need.1).is_some()
}

/// 一个缓存条目的内部状态。
///
/// 与公开的 [`AtlasGlyph`] 分开，是因为重排需要**位图本身**，而那是实现细节 ——
/// 把它塞进 `AtlasGlyph` 会让每次 `get` 都复制一份像素。
#[derive(Debug)]
struct Entry {
    /// 对外暴露的那一份：uv、尺寸、基线偏移。
    glyph: AtlasGlyph,
    /// 重排时用来重新落盘的覆盖率位图，行宽 = `width`。空白字形为空。
    coverage: Vec<u8>,
    width: u32,
    height: u32,
    /// 最近一次被取用的序号（越大越新）。淘汰从最小的开始。
    last_used: u64,
}

impl Entry {
    /// 给 [`EvictionPolicy`] 看的只读快照。
    fn info(&self, key: GlyphKey) -> EntryInfo {
        EntryInfo {
            key,
            padded: padded_size(self),
            last_used: self.last_used,
        }
    }
}

/// 一个条目的**占位尺寸**（含两侧留白）。空白字形返回 `(0, 0)`。
///
/// ⚠ 空白字形必须返回 `(0, 0)` 而不是 `(2, 2)`（`2 × GLYPH_PADDING`）—— 它们压根
/// 不占画布。这个值同时喂给策略的 `padded` 与落盘的 `place`，给错会让干跑与落盘
/// 对不上（又是"判得下、放不下"）。
fn padded_size(e: &Entry) -> (u32, u32) {
    if e.width == 0 || e.height == 0 {
        return (0, 0);
    }
    let p = 2 * GLYPH_PADDING;
    (e.width + p, e.height + p)
}

/// shelf packer 的游标状态。
///
/// 抽成一个类型，是为了让「逐个追加」与「整批重排」共用**同一份**换行规则 ——
/// 两份实现必然漂移，而漂移的后果是重排算出的空间与实际不符。
#[derive(Debug, Clone, Copy)]
struct Shelf {
    /// 画布边长。带上它而不用全局 [`ATLAS_SIZE`]，是为了让测试能用一张小图集
    /// 真的把淘汰路径跑起来 —— 1K² 要塞进上万个字形才满，那样的测试既慢又脆。
    side: u32,
    x: u32,
    y: u32,
    row_h: u32,
}

impl Shelf {
    fn new(side: u32) -> Self {
        Self {
            side,
            x: 0,
            y: 0,
            row_h: 0,
        }
    }

    /// 放一个 `w × h`，返回左上角。放不下返回 `None`，且**不改动状态**。
    ///
    /// 换行前先探测下一行够不够高：若先换行再发现新行也放不下，游标已经下移过了
    /// —— 那些行就永久空了出来，而后续字形仍会继续尝试、继续失败。
    fn place(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        if w > self.side || h > self.side {
            return None;
        }
        let mut next = *self;
        if next.x + w > next.side {
            let ny = next.y + next.row_h;
            if ny + h > next.side {
                return None;
            }
            next.y = ny;
            next.x = 0;
            next.row_h = 0;
        }
        let pos = (next.x, next.y);
        next.x += w;
        next.row_h = next.row_h.max(h);
        *self = next;
        Some(pos)
    }

    /// 剩余的**垂直**空间（像素）。
    ///
    /// 刻意不看当前行的剩余横向空间 —— shelf packer 一旦某行放不下就整行作废、
    /// 换到下一行，所以"本行还差多少 x"对能否继续放字形没有意义。
    fn remaining_height(&self) -> u32 {
        self.side.saturating_sub(self.y + self.row_h)
    }
}

/// 图集纹理格式的 RGBA 分量数。
///
/// 图集本质是单通道覆盖率（R8），但展开成 RGBA 以兼容所有后端 —— 有些采样器只读
/// rgb 通道（[`crate::paint::Renderer`] 的着色器就是从 alpha 取覆盖率的）。
const RGBA: usize = 4;

/// 一个已打包的字形：纹理内的位置与像素尺寸。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AtlasGlyph {
    /// uv 矩形（**像素**单位，渲染器按图集边长归一化）。
    pub uv: Rect,
    /// 该字形栅格化后的像素尺寸。
    pub size: Size,
    /// 字形相对基线原点的偏移。
    ///
    /// 字体约定 y **向上**为正，而核心的 DIP 坐标系 y 向下为正。转换发生在构造
    /// [`crate::paint::GlyphQuad::dst`] 时，这里原样保留以免丢信息。
    pub offset: Point,
}

/// 字形图集。
///
/// # 淘汰策略可替换
///
/// 默认用 [`LruEviction`]。要换策略用 [`Self::with_policy`]（构造时定）或
/// [`Self::set_policy`]（运行期换）—— 换策略**不需要**重建图集，已有位图与 uv 保留。
pub struct GlyphAtlas {
    /// 边长。默认 [`ATLAS_SIZE`]；测试可以给小图，好让淘汰路径真的跑起来。
    side: u32,
    /// `side × side` 的位图（以 RGBA 分量存储）。
    pixels: Vec<u8>,
    /// 已打包的字形，键为 `GlyphKey`。
    glyphs: HashMap<GlyphKey, Entry>,
    /// 货架游标。**追加**路径用它；重排会把它整个换掉。
    shelf: Shelf,
    /// 单调递增的取用序号，给淘汰策略用。
    ///
    /// 用序号而不是 `Instant`：一是 core 里不引入时钟，二是测试要能**确定地**
    /// 构造"谁更久没用过"，而挂钟时间在测试里是不确定的。
    tick: u64,
    /// 位图内容的版本号，**只在 `pixels` 真的变了**时自增。
    ///
    /// 宿主靠它决定「要不要把图集重新传给 GPU」。⚠ 不能用"字形数量"当判据：淘汰
    /// 之后数量可能**恰好不变**（丢一个、进一个），而纹理内容已经整体重排过了 ——
    /// 那样 GPU 会一直抱着旧图集，画面上是字全跑到别的位置上去了。
    revision: u64,
    /// 累计淘汰过的条目数（诊断用）。
    evictions: u64,
    /// 淘汰策略。缺省 [`LruEviction`]。
    policy: Box<dyn EvictionPolicy>,
}

impl Default for GlyphAtlas {
    fn default() -> Self {
        Self::new()
    }
}

/// 手写 [`Debug`] 而不用 derive：`policy` 是 `Box<dyn EvictionPolicy>`，那个 trait
/// 不是 `Debug` 的（刻意如此 —— 策略的实现不该被迫提供调试输出）。
impl std::fmt::Debug for GlyphAtlas {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GlyphAtlas")
            .field("side", &self.side)
            .field("glyphs", &self.glyphs.len())
            .field("revision", &self.revision)
            .field("evictions", &self.evictions)
            .finish_non_exhaustive()
    }
}

impl GlyphAtlas {
    /// 空图集（一次性分配 `ATLAS_SIZE² × 4` = 4MB）。
    ///
    /// 那 4MB 是**常驻**的，与用到的字形数无关。对一个最小应用来说这个开销不小，
    /// 但换来的是"图集地址稳定"—— 纹理对象不必随字形增多而重建。真要省内存可以
    /// 改成惰性分配（首次 `get_or_insert` 时才建 Vec），代价是多一个分支。
    pub fn new() -> Self {
        Self::with_size(ATLAS_SIZE)
    }

    /// 指定边长的空图集。
    ///
    /// 存在的意义几乎只有测试：淘汰路径只在**装不下**时才走，而 1K² 要放进上万个
    /// 字形才满 —— 用一张 64×64 的图集几行就能把 `make_room` / `rebuild` 跑到。
    /// 生产路径请用 [`Self::new`]：后端建纹理时读的是 [`ATLAS_SIZE`]，用别的边长
    /// 会让上传的尺寸与纹理对不上。
    pub fn with_size(side: u32) -> Self {
        Self::with_policy(side, LruEviction)
    }

    /// 指定边长**与淘汰策略**的图集。
    ///
    /// # `side` 必须同时传给策略
    ///
    /// 策略需要知道画布边长才能判断"腾出来的地方够不够"。所以边长是每次 `evict`
    /// 的一个参数，而不是策略自己的字段 —— 同一份策略实例可以服务于不同边长的
    /// 图集，也就不会出现"策略以为图集是 1024、实际是 64"这种错判。
    pub fn with_policy(side: u32, policy: impl EvictionPolicy + 'static) -> Self {
        Self {
            side,
            pixels: vec![0u8; (side * side) as usize * RGBA],
            glyphs: HashMap::new(),
            shelf: Shelf::new(side),
            tick: 0,
            revision: 0,
            evictions: 0,
            policy: Box::new(policy),
        }
    }

    /// 运行期换策略。已有的位图与 uv 全部保留。
    ///
    /// 刻意**不**在这里触发一次重排 —— 换策略不该让所有幸存者的 uv 失效
    /// （那会让宿主白重传一次整张图集）。新策略从**下一次**放不下时开始生效。
    pub fn set_policy(&mut self, policy: impl EvictionPolicy + 'static) {
        self.policy = Box::new(policy);
    }

    /// 取（或打包）一个字形。
    ///
    /// `px_size` 是像素字号 —— 同一个字形在不同字号下是**不同的位图**，所以它是键的
    /// 一部分。用 DIP 还是像素取决于宿主是否已乘上缩放因子；[`super::TextStack`] 传的
    /// 是像素（DIP × 缩放）。
    ///
    /// `raster` 由 [`crate::text::GlyphSource::rasterize`] 提供：`None` = 空白字形。
    ///
    /// # 返回值
    ///
    /// `None` = **连淘汰都腾不出空间**。只有「图集装不下这个字形本身」才会发生
    /// （单个字形比整张图还大，或图集被一个同样大的字形占着）。
    /// 调用方应当跳过该字形而不是放弃整段文本。
    ///
    /// # ⚠ 返回的 uv 可能被**后续**的 `get_or_insert` 弄失效
    ///
    /// 淘汰会重排整张图集，幸存的 uv 全部改变。所以不要「取一个、用它的 uv、再取
    /// 下一个」—— 要先把需要的字形都取一遍，再统一读 uv。`TextRender::glyph_quads`
    /// 就是这么写的。
    pub fn get_or_insert(
        &mut self,
        font_id: u32,
        glyph_id: u32,
        px_size: u32,
        raster: Option<AtlasRaster>,
    ) -> Option<AtlasGlyph> {
        let key = GlyphKey::new(font_id, glyph_id, px_size);
        self.tick += 1;
        let now = self.tick;

        if let Some(e) = self.glyphs.get_mut(&key) {
            e.last_used = now;
            return Some(e.glyph);
        }

        // 空白字形（空格、零宽字符）：没有位图，但**必须**占一个条目 —— 否则每次都
        // 要重跑一遍栅格化才发现"原来是空的"。
        //
        // 空白**不**占空间：不能让它推进货架游标，否则字形之间会出现一条永远填不满
        // 的缝，实际可用面积比 `usage()` 报告的少得多。也因此它不参与淘汰。
        let blank_offset = raster
            .as_ref()
            .map(|r| r.offset)
            .unwrap_or(Point::new(0.0, 0.0));
        let Some(r) = raster.filter(|r| r.width > 0 && r.height > 0) else {
            let entry = AtlasGlyph {
                uv: Rect::new(0.0, 0.0, 0.0, 0.0),
                size: Size::new(0.0, 0.0),
                offset: blank_offset,
            };
            self.glyphs.insert(
                key,
                Entry {
                    glyph: entry,
                    coverage: Vec::new(),
                    width: 0,
                    height: 0,
                    last_used: now,
                },
            );
            return Some(entry);
        };

        // 真正要占的地方：位图 + 四周一圈透明留白。
        let padded = (r.width + 2 * GLYPH_PADDING, r.height + 2 * GLYPH_PADDING);
        let (x, y) = match self.shelf.place(padded.0, padded.1) {
            Some(p) => p,
            None => {
                // 满了 —— 按 LRU 腾地方，然后重排。
                self.make_room(padded.0, padded.1)?;
                self.shelf
                    .place(padded.0, padded.1)
                    .expect("make_room 保证重排后放得下 —— 否则它应当返回 false")
            }
        };

        // 墨写在留白之内。
        let ink_x = x + GLYPH_PADDING;
        let ink_y = y + GLYPH_PADDING;
        self.write_bitmap(ink_x, ink_y, &r);
        self.revision += 1;

        let entry = AtlasGlyph {
            uv: Rect::new(ink_x as f32, ink_y as f32, r.width as f32, r.height as f32),
            size: Size::new(r.width as f32, r.height as f32),
            offset: r.offset,
        };
        self.glyphs.insert(
            key,
            Entry {
                glyph: entry,
                coverage: r.coverage,
                width: r.width,
                height: r.height,
                last_used: now,
            },
        );
        Some(entry)
    }

    /// 把覆盖率位图写进画布。
    ///
    /// ⚠ `coverage` 的坐标**已经**是 `(0,0)..(w,h)`，不要减 `offset`。
    ///
    /// `AtlasRaster::coverage` 是"字形自身位图"，第 0 行 0 列就是字形左上角 ——
    /// 这一点由 [`crate::text::GlyphSource::rasterize`] 的实现保证（ab_glyph 的
    /// `OutlinedGlyph::draw` 回调坐标本就相对 `px_bounds.min`）。
    ///
    /// 这里原先写的是 `ax = sx - bx; ay = sy - by;`，又减了一次。而
    /// `r.offset.y` 对大写字母来说就是 cap 高度（32px 的 `'H'` 约 22），于是
    /// `ay` 恒为负 → 越界判断把**每一个**像素都 `continue` 掉 → 图集全空 →
    /// 屏幕上"字完全不出现"。
    ///
    /// 位置换算只做一次：`图集内位置 = (x + sx, y + sy)`。
    fn write_bitmap(&mut self, x: u32, y: u32, r: &AtlasRaster) {
        self.write_coverage(x, y, r.width, r.height, &r.coverage);
    }

    /// 同上，但直接收尺寸与位图 —— 重排时手头只有这两样，不需要再拼一个 `AtlasRaster`。
    fn write_coverage(&mut self, x: u32, y: u32, width: u32, height: u32, coverage: &[u8]) {
        let src_w = (coverage.len() / height.max(1) as usize).min(width as usize);
        for sy in 0..height {
            for sx in 0..src_w {
                let v = coverage[sy as usize * src_w + sx];
                if v == 0 {
                    // 跳过透明像素：不写就不会破坏该位置上**已有**的字形。
                    // 早期版本无条件写，于是每个字形的透明区域都把邻居擦成了 0 ——
                    // 症状是"字之间互相啃掉一块"。
                    continue;
                }
                let dx = x + sx as u32;
                let dy = y + sy;
                if dx >= self.side || dy >= self.side {
                    continue;
                }
                let dst = (dy as usize * self.side as usize + dx as usize) * RGBA;
                if dst + RGBA > self.pixels.len() {
                    continue;
                }
                // 四个通道都写：着色器从 alpha 取覆盖率，但留一份灰度可让调试用的
                // 「把图集当图片看」直接显示出形状。
                self.pixels[dst] = v;
                self.pixels[dst + 1] = v;
                self.pixels[dst + 2] = v;
                self.pixels[dst + 3] = v;
            }
        }
    }

    /// 按策略腾地方并重排。
    ///
    /// 返回 `Some(())` 表示「已经腾出并重排好，现在画布必然是**空**的」；
    /// `None` 表示淘汰到空都放不下 —— 也就是这个字形比整张图集还大。
    ///
    /// # 落盘前的独立复验
    ///
    /// 策略是外部代码，它返回的顺序**可能真的放不下**（写错策略、或图集被改小）。
    /// 所以这里在清空画布**之前**先用 [`pack_fits`] 复验一遍：放不下就放弃这次淘汰，
    /// 返回 `None`让调用方跳过该字形。
    ///
    /// 代价是白丢一批本该淘汰的字形（它们仍在图集里），而收益是画布永不被清空 ——
    /// 后者的症状是"所有文字突然消失，且没有任何报错"。
    fn make_room(&mut self, need_w: u32, need_h: u32) -> Option<()> {
        let entries: Vec<EntryInfo> = self.glyphs.iter().map(|(k, e)| e.info(*k)).collect();
        let plan = self.policy.evict(&entries, (need_w, need_h), self.side)?;

        // 复验：用策略给的**同一个顺序**干跑一次。
        let mut shelf = Shelf::new(self.side);
        for key in &plan.survivors {
            let e = self
                .glyphs
                .get(key)
                .expect("策略返回的幸存者必须都还在图集里");
            let (pw, ph) = padded_size(e);
            shelf.place(pw, ph)?;
        }
        shelf.place(need_w, need_h)?;

        self.evictions += plan.dropped as u64;
        self.rebuild(&plan.survivors);
        Some(())
    }

    /// 清空画布，把 `keep` 里的条目**按给定顺序**重新放一遍。
    ///
    /// ⚠ 顺序由 [`EvictPlan::survivors`] 给出，本函数**不再自己排序** —— 干跑
    /// ([`Self::make_room`] 里的复验) 与落盘共用同一个顺序源，所以"两处不一致"
    /// 在类型层面就不可能发生。
    fn rebuild(&mut self, keep: &[GlyphKey]) {
        self.pixels.fill(0);
        self.revision += 1;
        self.shelf = Shelf::new(self.side);

        // 只保留幸存者。淘汰掉的那些条目连位图一起丢掉 —— 那正是内存回收的地方。
        let mut survivors: HashMap<GlyphKey, Entry> = HashMap::with_capacity(keep.len());
        for key in keep {
            let Some(mut e) = self.glyphs.remove(key) else {
                continue;
            };
            let (pw, ph) = padded_size(&e);
            if pw == 0 || ph == 0 {
                // 空白字形不占画布，**不调用** `place` —— 否则会在图里留出一个
                // 永远填不满的洞，实际可用面积比 `stats().used_px` 报告的少得多。
                survivors.insert(*key, e);
                continue;
            }
            let (x, y) = self
                .shelf
                .place(pw, ph)
                .expect("复验已经证明这批能全部放下");
            e.glyph.uv.x = (x + GLYPH_PADDING) as f32;
            e.glyph.uv.y = (y + GLYPH_PADDING) as f32;
            self.write_coverage(
                x + GLYPH_PADDING,
                y + GLYPH_PADDING,
                e.width,
                e.height,
                &e.coverage,
            );
            survivors.insert(*key, e);
        }

        // ⚠ 空白字形必须**无条件**回到表里，哪怕策略没提名它们。
        //
        // 淘汰策略只关心"谁占画布"，所以空白条目被排除在 `survivors` 之外 —— 但它们
        // 是有效缓存：下次取用同一个空格时若条目丢了，就得重跑一遍栅格化才发现
        // "原来是空的"，而空白字形的 `offset`（它决定光标往哪走）也就丢了。
        //
        // 这个 bug 的症状很隐蔽：图集重排之后，空格相关的排版度量悄悄变了。
        for (key, e) in self.glyphs.drain() {
            if padded_size(&e) == (0, 0) {
                survivors.insert(key, e);
            }
        }

        self.glyphs = survivors;
    }

    /// 已打包的字形（不含空白）。
    ///
    /// 只读 —— 不动 LRU 的 recency（本方法是 `&self`，动不了）。真正刷新鲜度的是
    /// [`Self::get_or_insert`]，所以「这一帧用到了谁」由第一趟决定，这一趟只负责读。
    pub fn get(&self, font_id: u32, glyph_id: u32, px_size: u32) -> Option<AtlasGlyph> {
        self.glyphs
            .get(&GlyphKey::new(font_id, glyph_id, px_size))
            .map(|e| e.glyph)
    }

    /// 刷新一个字形的 LRU 时间戳，不碰位图。
    ///
    /// # 为什么需要它，而不是"没命中才调 `get_or_insert`"
    ///
    /// 那样会**跳过已缓存字形的时间戳刷新**，于是长时间静止后再动一下的界面会发生
    /// 灾难性的连锁淘汰：所有字形的时间戳都停在很早以前，LLU 认为它们全都"最久
    /// 未用"，于是一插新字形就把整张图集重排一遍 —— 而重排会让**所有幸存者的 uv
    /// 全变**，触发一次 4MB 重传。
    ///
    /// 症状是"静止几秒后第一次打字，整屏文字闪一下并错位一会儿"。这与"字体坏了"
    /// 看起来一样，而根因是一次看起来无害的性能优化。
    ///
    /// # 与 `get_or_insert` 的关系
    ///
    /// `get_or_insert` 的命中分支只做两件事：刷时间戳、返回 `e.glyph`。本方法就是
    /// 那条路径的独立入口 —— 区别是**调用方不必先栅格化出位图**。
    ///
    /// # ⚠ 返回的 uv 同样可能被**后续**写入弄失效
    ///
    /// 与 [`Self::get_or_insert`] 同理：淘汰重排会改掉所有幸存的 uv。所以调用方
    /// 仍必须"先把需要的都取一遍、再统一用"（见 [`crate::text::TextRender`]）。
    pub fn touch(&mut self, font_id: u32, glyph_id: u32, px_size: u32) -> Option<AtlasGlyph> {
        let key = GlyphKey::new(font_id, glyph_id, px_size);
        self.tick += 1;
        let now = self.tick;
        let e = self.glyphs.get_mut(&key)?;
        e.last_used = now;
        Some(e.glyph)
    }

    /// 图集位图（RGBA 分量）。宿主把它上传成纹理。
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// 纹理边长。
    pub fn size(&self) -> u32 {
        self.side
    }

    /// 位图内容的版本号。**只在 `pixels` 真的变了**时自增。
    ///
    /// 宿主用它判断「要不要把图集重新传给 GPU」。⚠ 不能用"字形数量"代替：淘汰之后
    /// 数量可能恰好不变（丢一个、进一个），而纹理会整体重排 —— 那样 GPU 会一直抱着
    /// 旧图集，画面上是字全跑到别的位置去了。
    ///
    /// 空白字形不占画布也不改像素，所以插入它们**不**推进版本号 —— 免得每遇到一个
    /// 空格就白传一次 4MB。
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// 诊断信息。
    pub fn stats(&self) -> AtlasStats {
        AtlasStats {
            glyphs: self
                .glyphs
                .values()
                .filter(|e| e.glyph.size.width > 0.0)
                .count(),
            // 下一个**新行**的顶部。当前行部分占用的横向空间不计入 ——
            // `used_px` 的语义是"已提交的行高度之和"，与横向填充率无关。
            used_px: self.shelf.y + self.shelf.row_h,
            capacity_px: self.side,
            evictions: self.evictions,
        }
    }

    /// 剩余的**垂直**空间（像素）：还能开几个高度为 `h` 的新行。
    ///
    /// 与货架自身的口径一致 —— 转发过去，免得两处算法各自漂移。
    pub fn remaining_height(&self) -> u32 {
        self.shelf.remaining_height()
    }
}

/// 一次栅格化的结果：位置 + 尺寸 + 覆盖率位图。
///
/// # 为什么是 `Vec<u8>` 而不是回调
///
/// 早期版本用回调（`Box<dyn FnMut(u32,u32,u8)>`），试图省掉中间缓冲。实测下来那是
/// 错的：图集需要在**定位之后**才知道往哪里写，而回调式意味着要么先问一遍几何、
/// 定位后再重放一遍（于是回调必须可重复调用，实现负担转移到 `GlyphSource` 上），
/// 要么就得临时缓冲。
///
/// `Vec<u8>` 让这件事变直白：来源写进这块缓冲，图集把它搬到定位好的地方。代价是
/// 每个**新**字形一次分配 —— 而新字形是罕见事件（稳态下零分配），完全可接受。
#[derive(Debug, Clone, PartialEq)]
pub struct AtlasRaster {
    /// 位图相对基线原点的偏移。
    pub offset: Point,
    /// 位图宽（像素）。
    pub width: u32,
    /// 位图高（像素）。
    pub height: u32,
    /// 逐像素覆盖率（0..=255），长度 = `width × height`。
    pub coverage: Vec<u8>,
}

/// 图集使用情况。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AtlasStats {
    /// 已缓存的**可见**字形数（空白字形不计）。
    pub glyphs: usize,
    /// 已用高度（像素）。
    ///
    /// shelf packer 下"已用面积"不是这个数，但高度足以判断是否接近上限。
    pub used_px: u32,
    /// 总高度。
    pub capacity_px: u32,
    /// 累计被 LRU 淘汰掉的条目数。
    ///
    /// 稳态下**应当是 0**：正常 UI 的可见字符集远小于 1K²。它持续增长说明要么
    /// 字号种类太多，要么有人在每帧用不同的字号/字体刷图集 —— 那种情况该调大
    /// `ATLAS_SIZE` 而不是靠淘汰硬扛（淘汰会让下一帧重新栅格化，是明确的性能坑）。
    pub evictions: u64,
}

impl AtlasStats {
    /// 行高口径的使用率（0..1）。
    ///
    /// 注意这是**近似**：它按"已提交的行高度之和 / 总高"算，不反映当前行内的横向
    /// 填充率。所以它可能**高估**可用空间。需要精确判断"还能不能放下一个字形"
    /// 时用 [`GlyphAtlas::remaining_height`]。
    pub fn usage(&self) -> f32 {
        self.used_px as f32 / self.capacity_px as f32
    }
}

#[cfg(test)]
#[path = "tests/atlas_tests.rs"]
mod tests;

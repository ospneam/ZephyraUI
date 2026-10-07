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
    fn evict(
        &self,
        entries: &[EntryInfo],
        need: (u32, u32),
        side: u32,
    ) -> Option<EvictPlan>;
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
    fn evict(
        &self,
        entries: &[EntryInfo],
        need: (u32, u32),
        side: u32,
    ) -> Option<EvictPlan> {
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
        let padded = (
            r.width + 2 * GLYPH_PADDING,
            r.height + 2 * GLYPH_PADDING,
        );
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
            uv: Rect::new(
                ink_x as f32,
                ink_y as f32,
                r.width as f32,
                r.height as f32,
            ),
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
        let entries: Vec<EntryInfo> = self
            .glyphs
            .iter()
            .map(|(k, e)| e.info(*k))
            .collect();
        let plan = self.policy.evict(&entries, (need_w, need_h), self.side)?;

        // 复验：用策略给的**同一个顺序**干跑一次。
        let mut shelf = Shelf::new(self.side);
        for key in &plan.survivors {
            let e = self
                .glyphs
                .get(key)
                .expect("策略返回的幸存者必须都还在图集里");
            let (pw, ph) = padded_size(e);
            if shelf.place(pw, ph).is_none() {
                return None;
            }
        }
        if shelf.place(need_w, need_h).is_none() {
            return None;
        }

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
            glyphs: self.glyphs.values().filter(|e| e.glyph.size.width > 0.0).count(),
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
mod tests {
    use super::*;

    /// 构造一个全不透明的 `AtlasRaster`。
    fn raster(w: u32, h: u32, offset: Point) -> AtlasRaster {
        AtlasRaster {
            offset,
            width: w,
            height: h,
            coverage: vec![255u8; (w * h) as usize],
        }
    }

    /// 只有一个像素非零的 `AtlasRaster` —— 用于验证落盘位置。
    fn sparse_raster(w: u32, h: u32, offset: Point, value: u8) -> AtlasRaster {
        let mut coverage = vec![0u8; (w * h) as usize];
        coverage[0] = value;
        AtlasRaster {
            offset,
            width: w,
            height: h,
            coverage,
        }
    }

    #[test]
    fn shelf_packer_never_overflows_the_atlas() {
        let mut shelf = Shelf::new(ATLAS_SIZE);
        let mut placed: Vec<(u32, u32)> = Vec::new();
        for _ in 0..2000 {
            match shelf.place(32, 32) {
                Some(p) => placed.push(p),
                None => break,
            }
        }

        // 逐个检查，而不是靠"数量 × 边长"那种推算 —— 推算在换行逻辑正确时通过、
        // 有 off-by-one 时也通过，只有直接比对坐标才能抓住越界。
        for (x, y) in &placed {
            assert!(
                *x + 32 <= ATLAS_SIZE && *y + 32 <= ATLAS_SIZE,
                "字形被放在 ({x},{y})，越出 {ATLAS_SIZE}×{ATLAS_SIZE}"
            );
        }
        assert_eq!(
            placed.len(),
            (ATLAS_SIZE / 32) as usize * (ATLAS_SIZE / 32) as usize,
            "应恰好填满整张图集后才报满"
        );
        assert!(shelf.place(32, 32).is_none(), "图集满后必须拒绝");
    }

    #[test]
    fn a_glyph_larger_than_the_atlas_is_refused() {
        let mut shelf = Shelf::new(ATLAS_SIZE);
        assert!(
            shelf.place(ATLAS_SIZE + 1, 8).is_none(),
            "比图集还宽的字形必须被拒绝，否则会越界写"
        );
        assert!(
            shelf.place(8, ATLAS_SIZE + 1).is_none(),
            "比图集还高的字形必须被拒绝"
        );
        // 恰好等于边长是合法的。
        assert!(shelf.place(ATLAS_SIZE, ATLAS_SIZE).is_some());
    }

    #[test]
    fn allocate_wraps_to_a_new_row() {
        let mut shelf = Shelf::new(ATLAS_SIZE);
        let a = shelf.place(600, 40).unwrap();
        let b = shelf.place(600, 40).unwrap();
        // 600 + 600 > 1024 → 第二个必须换行
        assert_eq!(a.0, 0);
        assert_eq!(b.0, 0, "换行后 x 归零");
        assert!(b.1 >= 40, "换行后 y 下移至少一行高，实得 {}", b.1);
    }

    #[test]
    fn a_row_tracks_its_tallest_glyph() {
        let mut shelf = Shelf::new(ATLAS_SIZE);
        // 两个占满整行宽度的字形，高度不同：先矮后高。
        shelf.place(ATLAS_SIZE, 20).unwrap();
        shelf.place(ATLAS_SIZE, 60).unwrap();
        // 第一个已填满 x 方向，下一个必然换行。y 必须跳过 60（最高者）。
        let next = shelf.place(10, 10).unwrap();
        assert_eq!(next.0, 0, "换行后 x 归零");
        assert!(next.1 >= 60, "换行应让开最高字形，实得 y={}", next.1);
    }

    #[test]
    fn a_failed_wrap_does_not_burn_the_remaining_rows() {
        // 尾部空间不足时，一个更高的字形必须报"满"，而不是先换行把游标推下去 ——
        // 后者会让那些像素永久空出来，而诊断信息还报告有余量（撒谎比报错更难查）。
        //
        // `ATLAS_SIZE` 不是行高的整数倍（1024 / 30 = 34 余 4），所以填满后必然还剩
        // 一点。这正好顺带验证了"剩余不足一行时更高字形被拒"。
        const H: u32 = 30;
        let mut shelf = Shelf::new(ATLAS_SIZE);
        while shelf.remaining_height() >= H {
            shelf.place(ATLAS_SIZE, H).expect("整行总能放下");
        }
        let before = shelf.remaining_height();
        assert!(before < H, "前置条件：剩余应不足一行，实得 {before}");

        assert!(
            shelf.place(ATLAS_SIZE, 60).is_none(),
            "剩余高度不足时更高的字形应报满"
        );
        assert_eq!(shelf.remaining_height(), before, "失败的换行不得推进游标");
        assert!(
            shelf.place(ATLAS_SIZE, H).is_none(),
            "剩余不足一行时连一行也放不下了"
        );
        assert_eq!(
            shelf.remaining_height(),
            before,
            "连续失败也不得推进游标"
        );
    }

    #[test]
    fn the_last_row_is_filled_then_reported_full() {
        // 用**窄**字形逐行填满高度，把最后一行留成"正好一行"的空间，然后验证
        // 那一行能用、之后报满、且失败不推进游标。
        //
        // 宽度刻意不等于 1024，否则每次 `place` 都填满 x 方向、从而直接触发换行，
        // 测不到"同一行里连续放入多个字形"这条路径。
        const W: u32 = 100;
        const H: u32 = 30;
        let mut shelf = Shelf::new(ATLAS_SIZE);

        // 填到剩余高度不足两行。
        while shelf.remaining_height() >= 2 * H {
            // 每行放 8 个（800px），再补一个 224px 填满该行 → 强制换行。
            for _ in 0..8 {
                shelf.place(W, H).expect("100×30 总能放下");
            }
            shelf.place(224, H).expect("补满该行");
        }

        let remaining = shelf.remaining_height();
        assert!(
            remaining >= H,
            "前置条件：应至少还剩一行，实得 {remaining}"
        );
        assert!(
            remaining < 2 * H,
            "前置条件：剩余应不足两行，实得 {remaining}"
        );

        // 剩余空间至少够一行 → 应当成功。
        assert!(
            shelf.place(W, H).is_some(),
            "剩余 {remaining}px 应能容纳一行"
        );
        // 1024 不是 30 的整数倍，所以剩余高度未必恰好为 0 —— 但必然不足一行。
        assert!(
            shelf.remaining_height() < H,
            "用掉最后一行后剩余应不足一行，实得 {}",
            shelf.remaining_height()
        );
        let after = shelf.remaining_height();
        // 用**整行宽度**再试一次：shelf packer 只在放不下时才换行，而 100px 可能还
        // 塞得进当前行的横向余量（那样就不触发换行判定，也就测不到垂直空间耗尽）。
        assert!(
            shelf.place(ATLAS_SIZE, H).is_none(),
            "剩余不足一行时整行字形应被拒"
        );
        assert_eq!(shelf.remaining_height(), after, "失败不得推进游标");
    }

    #[test]
    fn remaining_height_ignores_unused_space_in_the_current_row() {
        // shelf packer 一旦某行放不下就整行换掉，所以"本行剩余 x"不该被算成可用。
        let mut shelf = Shelf::new(ATLAS_SIZE);
        shelf.place(10, 40).unwrap();
        // 当前行只用了 10/1024 的宽度，但下一行仍从 y=40 开始。
        assert_eq!(
            shelf.remaining_height(),
            ATLAS_SIZE - 40,
            "剩余高度只按行高算，与横向填充率无关"
        );
    }

    #[test]
    fn packing_the_same_glyph_twice_reuses_the_slot() {
        let mut atlas = GlyphAtlas::new();
        let first = atlas
            .get_or_insert(0, 42, 16, Some(raster(8, 8, Point::new(0.0, 0.0))))
            .unwrap();
        // 第二次传一个**不同**的位图：若图集真的复用了缓存，返回的条目应当与第一次
        // 完全一致（而不是被新位图覆盖）。
        let second = atlas
            .get_or_insert(0, 42, 16, Some(sparse_raster(8, 8, Point::new(0.0, 0.0), 7)))
            .unwrap();
        assert_eq!(first, second, "同字形同字号必须返回同一条目");
        // 且位图内容未被第二次的输入改写。
        let (x, y) = (first.uv.x as usize, first.uv.y as usize);
        let base = (y * ATLAS_SIZE as usize + x) * RGBA;
        assert_eq!(atlas.pixels()[base + 3], 255, "缓存命中时不得重写位图");
    }

    #[test]
    fn a_different_px_size_is_a_different_glyph() {
        let mut atlas = GlyphAtlas::new();
        let a = atlas
            .get_or_insert(0, 7, 12, Some(raster(8, 8, Point::new(0.0, 0.0))))
            .unwrap();
        let b = atlas
            .get_or_insert(0, 7, 24, Some(raster(16, 16, Point::new(0.0, 0.0))))
            .unwrap();
        assert_ne!(a.uv, b.uv, "不同字号应打包到不同位置");
        assert_eq!(b.size, Size::new(16.0, 16.0));
    }

    #[test]
    fn a_different_font_is_a_different_glyph() {
        let mut atlas = GlyphAtlas::new();
        let a = atlas
            .get_or_insert(0, 7, 16, Some(raster(8, 8, Point::new(0.0, 0.0))))
            .unwrap();
        let b = atlas
            .get_or_insert(1, 7, 16, Some(raster(8, 8, Point::new(0.0, 0.0))))
            .unwrap();
        assert_ne!(a.uv, b.uv, "不同字体不得共用同一块位图");
    }

    #[test]
    fn blank_glyphs_get_a_zero_sized_entry() {
        let mut atlas = GlyphAtlas::new();
        let g = atlas.get_or_insert(0, 3, 16, None).unwrap();
        assert_eq!(g.size, Size::new(0.0, 0.0), "空白字形尺寸应为 0");
        assert_eq!(g.uv, Rect::new(0.0, 0.0, 0.0, 0.0));
        // 且必须占条目，否则每次都要重跑栅格化。
        assert_eq!(
            atlas.get(0, 3, 16).map(|e| e.size),
            Some(Size::new(0.0, 0.0)),
            "空白字形也要缓存"
        );
        // 空白字形不计入"可见字形"统计。
        assert_eq!(atlas.stats().glyphs, 0);
    }

    #[test]
    fn packed_pixels_land_at_the_allocated_position() {
        // ⚠ 这个测试原先断言的是**错误**的语义，已改正。
        //
        // 原注释说「来源位图的像素 (sx, sy) 对应基线原点的 (offset.x + sx,
        // offset.y + sy)，即图集内的 (x + sx - offset.x, y + sy - offset.y)」。
        // 那正是那个 bug 的样子：它把 `offset` 从图集落点里减掉了。
        //
        // 正确契约（见本文件 `get_or_insert` 里写 `coverage` 的那段）：
        //
        // ```text
        // coverage(sx, sy)  →  图集 (x + sx, y + sy)
        // ```
        //
        // `offset` **不参与**落点计算 —— 它是给 `GlyphQuad::dst` 用的，
        // 描述的是"这个字形在基线原点哪儿"，与"它在图集里放在哪儿"是两件事。
        let mut atlas = GlyphAtlas::new();
        let entry = atlas
            .get_or_insert(
                0,
                5,
                16,
                // offset 故意给一个"典型的大写字母"的值：32px 的 'H' 的
                // bounds.min.y 约 22。旧实现会因此把整块位图跳过。
                Some(sparse_raster(4, 4, Point::new(3.0, 22.0), 200)),
            )
            .unwrap();
        let (x, y) = (entry.uv.x as usize, entry.uv.y as usize);
        let px = |ax: usize, ay: usize| {
            atlas.pixels()[((y + ay) * ATLAS_SIZE as usize + (x + ax)) * RGBA + 3] as i32
        };

        // coverage(0,0) 是唯一非零的像素 → 它落在分配位置的左上角。
        assert_eq!(
            px(0, 0),
            200,
            "coverage(0,0) 应落在分配位置的左上角，而不是被 offset 推开"
        );
        assert_eq!(px(1, 0), 0, "coverage(1,0) 是透明的，不该有值");
        assert_eq!(px(0, 1), 0, "coverage(0,1) 是透明的，不该有值");

        // 反过来：第二个字形**左侧**那一格仍属于第一个字形。
        //
        // 注意货架打包器把两个字形放在**同一行**的不同 x 上（y 都是 0），所以这里查的是
        // 左边而不是上边 —— 早先写成 `y - 1` 在第 0 行会下溢，那种"只在特定布局下才成立"
        // 的断言等于没有断言。
        let mut atlas_up = GlyphAtlas::new();
        let filler = AtlasRaster {
            offset: Point::new(0.0, 0.0),
            width: 1,
            height: 1,
            coverage: vec![7],
        };
        atlas_up.get_or_insert(0, 1, 16, Some(filler)).unwrap();
        let e2 = atlas_up
            .get_or_insert(
                0,
                2,
                16,
                Some(sparse_raster(4, 4, Point::new(3.0, 22.0), 200)),
            )
            .unwrap();
        let (x2, y2) = (e2.uv.x as usize, e2.uv.y as usize);
        assert!(x2 > 0, "第二个字形应当排在第一个右边 —— 否则这个断言没有意义");
        let left = atlas_up.pixels()[(y2 * ATLAS_SIZE as usize + (x2 - 1)) * RGBA + 3];
        assert_ne!(
            left, 200,
            "第二个字形把**第一个字形**所在的那一列覆盖了 —— 落点算错了方向"
        );
    }

    /// coverage 的**内部**位置必须原样保留，不能因为 `offset` 而错位。
    ///
    /// 上一条验的是左上角。这一条把非零像素放在位图内部，才能证明实现**完全**没有
    /// 位移 —— 只验角上的一个像素，会漏掉"只偏了一行/一列"的实现。
    #[test]
    fn interior_pixels_keep_their_position_inside_the_bitmap() {
        let mut atlas = GlyphAtlas::new();
        let mut coverage = vec![0u8; 16]; // 4×4
        coverage[5] = 200; // 行 1、列 1
        let e = atlas
            .get_or_insert(
                0,
                5,
                16,
                Some(AtlasRaster {
                    // offset 与上一条相同 —— 换成"典型大写字母"的值。
                    offset: Point::new(3.0, 22.0),
                    width: 4,
                    height: 4,
                    coverage,
                }),
            )
            .unwrap();
        let (x, y) = (e.uv.x as usize, e.uv.y as usize);
        assert_eq!(
            atlas.pixels()[((y + 1) * ATLAS_SIZE as usize + (x + 1)) * RGBA + 3],
            200,
            "coverage(1,1) 应原样落在 (x+1, y+1) —— 位图内部不得有位移"
        );
        // 其余全透明：不该有墨漏到别处。
        for (sy, sx) in [(0usize, 0usize), (0, 1), (1, 0), (2, 2), (3, 3)] {
            assert_eq!(
                atlas.pixels()[((y + sy) * ATLAS_SIZE as usize + (x + sx)) * RGBA + 3],
                0,
                "coverage({sx},{sy}) 是透明的，图集 ({}, {}) 上却有值",
                x + sx,
                y + sy
            );
        }
    }

    #[test]
    fn oversized_coverage_is_clipped_not_written_out_of_bounds() {
        // 来源实现若给出比声明尺寸更多的像素，越界写会破坏相邻字形 —— 那种损坏在
        // 屏幕上表现为随机噪点，极难定位。
        let mut atlas = GlyphAtlas::new();
        let mut coverage = vec![0u8; 4 * 4];
        for v in coverage.iter_mut() {
            *v = 255;
        }
        coverage.extend_from_slice(&[255u8; 32]); // 声称是 4×4 却给了 36 字节
        let entry = atlas
            .get_or_insert(
                0,
                9,
                16,
                Some(AtlasRaster {
                    offset: Point::new(0.0, 0.0),
                    width: 4,
                    height: 4,
                    coverage,
                }),
            )
            .unwrap();
        // 紧随其后打包一个字形：它的内容必须是干净的。
        let next = atlas
            .get_or_insert(0, 10, 16, Some(sparse_raster(4, 4, Point::new(0.0, 0.0), 99)))
            .unwrap();
        let (nx, ny) = (next.uv.x as usize, next.uv.y as usize);
        let base = (ny * ATLAS_SIZE as usize + nx) * RGBA;
        assert_eq!(
            atlas.pixels()[base + 3], 99,
            "越界的覆盖率不得写进后续字形的区域"
        );
        assert_eq!(entry.size, Size::new(4.0, 4.0));
    }

    #[test]
    fn stats_report_usage() {
        let mut atlas = GlyphAtlas::new();
        let before = atlas.stats();
        assert_eq!(before.usage(), 0.0);
        atlas
            .get_or_insert(0, 1, 16, Some(raster(64, 64, Point::new(0.0, 0.0))))
            .unwrap();
        let after = atlas.stats();
        assert!(after.usage() > 0.0, "放入字形后使用率应大于 0");
        assert!(after.usage() <= 1.0, "使用率不得超过 1");
        assert_eq!(after.evictions, 0, "没有放满时不该发生淘汰");
    }

    #[test]
    fn atlas_pixel_buffer_has_the_expected_size() {
        let atlas = GlyphAtlas::new();
        assert_eq!(
            atlas.pixels().len(),
            (ATLAS_SIZE * ATLAS_SIZE) as usize * RGBA
        );
    }

    /// 位图里每个像素的 alpha —— 断言落点时比 `atlas.pixels()[...]` 好读。
    fn alpha(atlas: &GlyphAtlas, x: u32, y: u32) -> u8 {
        atlas.pixels()[(y as usize * atlas.size() as usize + x as usize) * RGBA + 3]
    }

    /// 数画布上有墨的像素。
    fn inked(atlas: &GlyphAtlas) -> usize {
        atlas
            .pixels()
            .chunks_exact(4)
            .filter(|p| p[3] > 0)
            .count()
    }

    // ========================================================================
    // 留白：双线性采样会摸到紧邻的一格，所以墨的四周必须留透明
    // ========================================================================

    /// 两个相邻字形之间必须隔出 `2 × GLYPH_PADDING` 列**透明**像素。
    ///
    /// 没有留白时，片元在四边形最外沿采样会各取一半邻居的纹素 —— 画面上是字与字
    /// 之间的淡竖条、圆角处的鬼影。这个测试直接钉住那几格的值。
    #[test]
    fn 相邻字形之间留出透明的一圈() {
        let mut atlas = GlyphAtlas::new();
        let a = atlas
            .get_or_insert(0, 1, 16, Some(raster(4, 4, Point::new(0.0, 0.0))))
            .unwrap();
        let b = atlas
            .get_or_insert(0, 2, 16, Some(raster(4, 4, Point::new(0.0, 0.0))))
            .unwrap();

        let (ax, ay) = (a.uv.x as u32, a.uv.y as u32);
        let (bx, by) = (b.uv.x as u32, b.uv.y as u32);

        assert_eq!(
            bx,
            ax + 4 + 2 * GLYPH_PADDING,
            "第二个字的墨应当让开「本字右边一圈 + 邻字左边一圈」"
        );
        assert_eq!(by, ay, "同一行里两个字的 y 应当一致（这里就是验证它们在同行）");

        // 墨本身是满的。
        assert_eq!(alpha(&atlas, ax + 3, ay + 1), 255, "第一个字的右边界应当是墨");
        assert_eq!(alpha(&atlas, bx, by + 1), 255, "第二个字的左边界应当是墨");

        // 而两者之间那两列必须是空的。
        assert_eq!(
            alpha(&atlas, bx - 1, by + 1),
            0,
            "邻字左侧的一圈没留白 —— 双线性采样会把它啃进来"
        );
        assert_eq!(
            alpha(&atlas, bx - 2, by + 1),
            0,
            "本字右侧的一圈没留白 —— 这是最典型的渗色源"
        );

        // 上下也要留：四边形上下沿的采样同样会越界。
        let above = ay - GLYPH_PADDING;
        let below = ay + 4 + GLYPH_PADDING - 1;
        for x in ax..ax + 4 {
            assert_eq!(alpha(&atlas, x, above), 0, "字上方的一圈不是空的");
            assert_eq!(alpha(&atlas, x, below), 0, "字下方的一圈不是空的");
        }
        assert_eq!(alpha(&atlas, ax - 1, ay), 0, "字左侧的一圈不是空的");
    }

    /// 留白**不影响**对外的 uv 与尺寸 —— uv 指的仍然是墨。
    ///
    /// 若把留白算进 uv，`GlyphQuad::dst` 会跟着变大，字与字之间凭空多出缝隙，
    /// 且字会被拉伸一点点。
    #[test]
    fn 留白不改变对外的_uv_与尺寸() {
        let mut atlas = GlyphAtlas::new();
        let e = atlas
            .get_or_insert(0, 1, 16, Some(raster(4, 3, Point::new(0.0, 0.0))))
            .unwrap();
        assert_eq!(e.size, Size::new(4.0, 3.0), "size 必须是墨的尺寸");
        assert_eq!(e.uv.width, 4.0, "uv 宽度必须是墨的宽度");
        assert_eq!(e.uv.height, 3.0, "uv 高度必须是墨的高度");
        // uv 的左上是墨的左上，不是留白的左上。
        assert_eq!(e.uv.x, GLYPH_PADDING as f32);
        assert_eq!(e.uv.y, GLYPH_PADDING as f32);
    }

    // ========================================================================
    // LRU 淘汰
    // ========================================================================

    /// 64×64 的图集、8×8 的墨（占 10×10）恰好装 3×3 = 9 个。
    const SMALL: u32 = 64;

    /// 用满一张小图集（**不**触发淘汰），返回它。
    ///
    /// ⚠ 不能写「一直插到返回 `None`」—— 现在满了会淘汰，于是永远不会返回 `None`，
    /// 循环停不下来。判满要看**容量**，不是看返回值。
    fn fill_small() -> GlyphAtlas {
        let mut atlas = GlyphAtlas::with_size(SMALL);
        for n in 0..36 {
            atlas
                .get_or_insert(0, n, 16, Some(raster(8, 8, Point::new(0.0, 0.0))))
                .unwrap_or_else(|| panic!("第 {n} 个应当放得下 —— 容量不该小于 36"));
        }
        assert_eq!(atlas.stats().evictions, 0, "还没满就不该淘汰");
        assert_eq!(atlas.stats().glyphs, 36);
        // 「满了」的判据：连一个 10×10（墨 + 两圈留白）都放不下了。
        assert!(
            atlas.remaining_height() < 10,
            "36 个之后应当已经放不下第 37 个，实得剩余 {}",
            atlas.remaining_height()
        );
        atlas
    }

    /// 满了之后再来新字形：淘汰**最久未用**的那个，最近用过的留下。
    #[test]
    fn 放不下时淘汰最久未用的字形() {
        let mut atlas = fill_small();

        // 把第 0、1 个标记成"最近用过"（重新取用即刷新）。
        atlas
            .get_or_insert(0, 0, 16, Some(raster(8, 8, Point::new(0.0, 0.0))))
            .unwrap();
        atlas
            .get_or_insert(0, 1, 16, Some(raster(8, 8, Point::new(0.0, 0.0))))
            .unwrap();

        let fresh = atlas
            .get_or_insert(0, 999, 16, Some(raster(8, 8, Point::new(0.0, 0.0))))
            .expect("淘汰之后应当放得下");

        assert_eq!(atlas.stats().evictions, 1, "只该淘汰一个 —— 多了就是白丢");
        assert!(
            atlas.get(0, 2, 16).is_none(),
            "第 2 个是最久未用的，应当被淘汰"
        );
        assert!(atlas.get(0, 0, 16).is_some(), "刚用过的不该被淘汰");
        assert!(atlas.get(0, 1, 16).is_some(), "刚用过的不该被淘汰");
        assert!(
            atlas.get(0, 999, 16).is_some_and(|e| e.uv == fresh.uv),
            "新字形应当真的在图集里"
        );
    }

    /// 重排之后画布上**只剩**幸存者的墨。
    ///
    /// 这条是重排最容易写错的地方：清了映射表却没清画布，旧墨会从新字形的留白里
    /// 露出来 —— 画面上是随机噪点，且只在触发淘汰之后才出现。
    #[test]
    fn 重排之后画布上只剩幸存者的墨() {
        let mut atlas = fill_small();
        let before = inked(&atlas);
        assert_eq!(before, 36 * 64, "36 个 8×8 的块，墨量应当是 36×64");

        // 触发一次淘汰：36 → 35 个幸存 + 1 个新的 = 36 个。
        atlas
            .get_or_insert(0, 999, 16, Some(raster(8, 8, Point::new(0.0, 0.0))))
            .unwrap();
        assert_eq!(atlas.stats().evictions, 1);

        let survivors = 36 - 1 + 1;
        assert_eq!(
            inked(&atlas),
            survivors * 64,
            "墨量对不上：多于这个数说明旧墨没清干净，少于说明有幸存者没被重画"
        );
        assert!(
            atlas.get(0, 0, 16).is_none(),
            "这一格没有预热任何字形，最久未用的应当是第 0 个 —— \
             它不该还留在映射表里"
        );
    }

    /// 重排后每个幸存者的 uv 必须指向**它自己**的墨，且彼此不重叠。
    ///
    /// 让每个字形的覆盖率值等于它的编号，于是「uv 指着谁的位图」是可判定的事实，
    /// 而不是"那里有墨"这种弱条件 —— 重排把映射表换了一遍，最典型的错法就是把
    /// 残值留在了别处。
    #[test]
    fn 重排后幸存者的_uv_互不重叠且指向自己的位图() {
        let mut atlas = GlyphAtlas::with_size(SMALL);
        // 编号 1..=36，覆盖率值 = 编号（非零，且两两不同）。
        for n in 1..=36u32 {
            let mut coverage = vec![0u8; 64];
            coverage[0] = n as u8;
            coverage[1] = n as u8;
            atlas
                .get_or_insert(
                    0,
                    n,
                    16,
                    Some(AtlasRaster {
                        offset: Point::new(0.0, 0.0),
                        width: 8,
                        height: 8,
                        coverage,
                    }),
                )
                .unwrap_or_else(|| panic!("第 {n} 个应当放得下"));
        }
        assert_eq!(atlas.stats().evictions, 0, "前置条件：36 个刚好装满");

        // 第 37 个触发淘汰与重排。
        atlas
            .get_or_insert(0, 999, 16, Some(raster(8, 8, Point::new(0.0, 0.0))))
            .unwrap();
        assert_eq!(atlas.stats().evictions, 1);

        let mut rects = Vec::new();
        for n in 1..=999u32 {
            let Some(e) = atlas.get(0, n, 16) else { continue };
            let (x, y) = (e.uv.x as u32, e.uv.y as u32);
            if n != 999 {
                assert_eq!(
                    alpha(&atlas, x, y),
                    n as u8,
                    "条目 {n} 的 uv 处不是它自己的位图 —— 重排把墨放错了位置"
                );
            }
            rects.push((n, x, y, x + 8, y + 8));
        }
        assert!(rects.len() >= 36, "幸存者数量不对：{}", rects.len());

        for i in 0..rects.len() {
            for j in i + 1..rects.len() {
                let (_, ax0, ay0, ax1, ay1) = rects[i];
                let (_, bx0, by0, bx1, by1) = rects[j];
                let disjoint = ax1 <= bx0 || bx1 <= ax0 || ay1 <= by0 || by1 <= ay0;
                assert!(
                    disjoint,
                    "条目 {} 与 {} 的 uv 重叠：{:?} / {:?}",
                    rects[i].0, rects[j].0, rects[i], rects[j]
                );
            }
        }
    }

    /// 重排后不许有**残墨**留在任何条目之外。
    ///
    /// 这是重排最容易写错的地方：清了映射表却没清画布，旧墨会留在原地。画面上是
    /// 随机噪点，而且**只在触发过淘汰之后**才出现 —— 没跑满的测试永远看不到。
    #[test]
    fn 重排后没有残墨留在条目之外() {
        let mut atlas = fill_small();
        atlas
            .get_or_insert(0, 999, 16, Some(raster(8, 8, Point::new(0.0, 0.0))))
            .unwrap();

        let mut rects = Vec::new();
        for n in 0..=999u32 {
            if let Some(e) = atlas.get(0, n, 16) {
                let (x, y) = (e.uv.x as u32, e.uv.y as u32);
                rects.push((x, y, x + 8, y + 8));
            }
        }

        // 正向：每个有墨的像素都必须落在某个条目的墨矩形内。
        for y in 0..SMALL {
            for x in 0..SMALL {
                if alpha(&atlas, x, y) == 0 {
                    continue;
                }
                let inside = rects
                    .iter()
                    .any(|&(x0, y0, x1, y1)| x >= x0 && x < x1 && y >= y0 && y < y1);
                assert!(
                    inside,
                    "({x},{y}) 有墨，却不属于任何条目 —— 重排留下了残墨"
                );
            }
        }
        // 反向：墨量恰好等于各条目面积之和 ⇒ 每个幸存者都被完整重画了。
        assert_eq!(
            inked(&atlas),
            rects.len() * 64,
            "有条目没被重画（墨少于面积），或有墨落在条目之外"
        );
    }

    /// 真的放不下时才淘汰 —— 反复取用已缓存的字形不该触发任何回收。
    #[test]
    fn 缓存命中不触发淘汰() {
        let mut atlas = GlyphAtlas::with_size(SMALL);
        let e = atlas
            .get_or_insert(0, 1, 16, Some(raster(8, 8, Point::new(0.0, 0.0))))
            .unwrap();
        for _ in 0..100 {
            let again = atlas
                .get_or_insert(0, 1, 16, Some(raster(8, 8, Point::new(0.0, 0.0))))
                .unwrap();
            assert_eq!(again.uv, e.uv, "缓存命中却换了位置");
        }
        assert_eq!(atlas.stats().evictions, 0);
        assert_eq!(atlas.stats().glyphs, 1, "反复取用不该新增条目");
    }

    /// 单个字形比整张图集还大 ⇒ 淘汰也没用，必须返回 `None` 而不是死循环。
    #[test]
    fn 比图集还大的字形_淘汰也放不下就返回_none() {
        let mut atlas = GlyphAtlas::with_size(SMALL);
        atlas
            .get_or_insert(0, 1, 16, Some(raster(8, 8, Point::new(0.0, 0.0))))
            .unwrap();
        assert!(
            atlas
                .get_or_insert(0, 2, 16, Some(raster(SMALL, SMALL, Point::new(0.0, 0.0))))
                .is_none(),
            "半个图集大的块加留白后超出边长，必须被拒绝"
        );
        // 拒绝时不得把已有的那个也搭进去。
        assert!(atlas.get(0, 1, 16).is_some(), "失败不该顺手清空图集");
    }

    #[test]
    fn with_size_决定边长与缓冲大小() {
        let atlas = GlyphAtlas::with_size(32);
        assert_eq!(atlas.size(), 32);
        assert_eq!(atlas.pixels().len(), 32 * 32 * RGBA);
        assert_eq!(atlas.stats().capacity_px, 32);
    }

    /// 淘汰那一下：字形**数量可能不变**，但版本号必须前进。
    ///
    /// # 这条在钉什么
    ///
    /// 宿主靠版本号决定「要不要把图集重传给 GPU」。若用字形数量当判据（图集只增不减
    /// 时那样是对的），淘汰一个再进一个 —— 数量不变、内容整体重排 —— 宿主就会跳过
    /// 上传，GPU 抱着旧图集，画面上字全跑到别的位置去了。
    #[test]
    fn 淘汰时字形数可能不变但版本号必须前进() {
        let mut atlas = fill_small();
        let before_rev = atlas.revision();
        let before_n = atlas.stats().glyphs;

        atlas
            .get_or_insert(0, 999, 16, Some(raster(8, 8, Point::new(0.0, 0.0))))
            .unwrap();

        assert_eq!(atlas.stats().evictions, 1, "前置条件：这一下应当触发淘汰");
        assert_eq!(
            atlas.stats().glyphs,
            before_n,
            "前置条件：丢一个、进一个 ⇒ 数量**不变** —— 这正是判据不能用量子的理由"
        );
        assert!(
            atlas.revision() > before_rev,
            "数量没变但内容整体重排了，版本号必须前进，否则宿主不会重传"
        );
    }

    /// 空白字形不推进版本号 —— 否则每遇到一个空格就白传一次整张图集。
    #[test]
    fn 空白字形不推进版本号() {
        let mut atlas = GlyphAtlas::with_size(SMALL);
        let before = atlas.revision();
        atlas.get_or_insert(0, 1, 16, None).unwrap();
        atlas.get_or_insert(0, 2, 16, None).unwrap();
        assert_eq!(
            atlas.revision(),
            before,
            "空白字形不占画布，不该让宿主重传整张图集"
        );
        // 但有条目、且数量统计不计入可见字形。
        assert_eq!(atlas.stats().glyphs, 0);
        assert!(atlas.get(0, 1, 16).is_some());
    }

    // ========================================================================
    // 可替换的淘汰策略（[`EvictionPolicy`]）
    // ========================================================================

    /// 只记录调用、不决策的策略 —— 用来断言"图集确实问过策略"。
    ///
    /// 计数用 [`Rc`] 共享，因为整个策略被 move 进 `Box<dyn EvictionPolicy>` ——
    /// 外部若拿一个独立的 [`Cell`] 副本去读，读到的永远是 0（`Cell` 不共享）。
    /// 那条测试最初就是这么写的，于是断言"策略没被调用"，而调用其实发生了。
    struct Counting {
        calls: std::rc::Rc<std::cell::Cell<usize>>,
    }

    impl Counting {
        fn new() -> (Self, std::rc::Rc<std::cell::Cell<usize>>) {
            let calls = std::rc::Rc::new(std::cell::Cell::new(0));
            (Self { calls: calls.clone() }, calls)
        }
    }

    impl EvictionPolicy for Counting {
        fn evict(
            &self,
            entries: &[EntryInfo],
            need: (u32, u32),
            side: u32,
        ) -> Option<EvictPlan> {
            self.calls.set(self.calls.get() + 1);
            LAST_SIDE.with(|s| s.set(side));
            LAST_NEED.with(|n| n.set(need));
            LruEviction.evict(entries, need, side)
        }
    }

    thread_local! {
        static LAST_SIDE: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
        static LAST_NEED: std::cell::Cell<(u32, u32)> = const { std::cell::Cell::new((0, 0)) };
    }

    /// 图集必须把**自己的边长**与**待放入的占位尺寸**传给策略。
    ///
    /// `side` 传错的后果很具体：策略按 1024 算"放得下"，实际图集是 64 —— 于是它
    /// 交回一个必然放不下的顺序。那条复验会挡住，所以不会清空画布，但每一个新字形
    /// 都会白触发一次淘汰。
    #[test]
    fn 策略收到的是本图集的真实边长与占位尺寸() {
        let (counting, calls) = Counting::new();
        let mut atlas = GlyphAtlas::with_policy(SMALL, counting);
        // 先塞一个字形占位，再触发一次淘汰。
        atlas
            .get_or_insert(0, 1, 16, Some(raster(8, 8, Point::new(0.0, 0.0))))
            .unwrap();
        for n in 2..40u32 {
            atlas
                .get_or_insert(0, n, 16, Some(raster(8, 8, Point::new(0.0, 0.0))))
                .unwrap();
        }
        assert!(calls.get() >= 1, "图集满了就该问过策略");
        assert_eq!(
            LAST_SIDE.with(|s| s.get()),
            SMALL,
            "策略必须拿到本图集的真实边长，而不是全局 ATLAS_SIZE"
        );
        // 8×8 的墨 + 两侧各1 像素留白 = 10×10。
        assert_eq!(
            LAST_NEED.with(|n| n.get()),
            (10, 10),
            "need 必须是**占位**尺寸（含留白），不是墨的尺寸"
        );
    }

    /// 一个"什么都留不住、全丢"的策略：验证图集真的照策略的结果执行。
    struct DropEverything;

    impl EvictionPolicy for DropEverything {
        fn evict(
            &self,
            entries: &[EntryInfo],
            _need: (u32, u32),
            _side: u32,
        ) -> Option<EvictPlan> {
            Some(EvictPlan {
                survivors: Vec::new(),
                dropped: entries.iter().filter(|e| occupies_canvas(e)).count(),
            })
        }
    }

    /// 图集必须**照策略给的幸存者列表**执行，而不是自己再挑一遍。
    ///
    /// `DropEverything` 要求一个不留 —— 若图集忽略策略而自行 LRU，画布上就还会有
    /// 36 个块。这里断言的是"策略的话语权"，不是"淘汰能否工作"。
    #[test]
    fn 策略说全丢_图集就全丢() {
        let mut atlas = fill_small();
        assert_eq!(atlas.stats().glyphs, 36, "前置条件：图集已满");
        atlas.set_policy(DropEverything);

        atlas
            .get_or_insert(0, 999, 16, Some(raster(8, 8, Point::new(0.0, 0.0))))
            .unwrap();

        assert_eq!(
            atlas.stats().glyphs,
            1,
            "策略要求一个不留 ⇒ 只该剩下刚放进去的那个"
        );
        assert_eq!(atlas.stats().evictions, 36);
    }

    /// 只留指定键的策略 —— 用来验证**顺序**也被照搬。
    ///
    /// 用 `Rc` 包裹是为了让测试在把策略交给图集之后仍能读它记录的调用 ——
    /// `set_policy` 会把策略 move 进去。
    struct KeepJust {
        keep: Vec<GlyphKey>,
        observed: std::rc::Rc<std::cell::RefCell<Vec<GlyphKey>>>,
    }

    impl KeepJust {
        fn new(keep: Vec<GlyphKey>) -> (Self, std::rc::Rc<std::cell::RefCell<Vec<GlyphKey>>>) {
            let observed = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
            (
                Self {
                    keep,
                    observed: observed.clone(),
                },
                observed,
            )
        }
    }

    impl EvictionPolicy for KeepJust {
        fn evict(
            &self,
            entries: &[EntryInfo],
            need: (u32, u32),
            side: u32,
        ) -> Option<EvictPlan> {
            // ⚠ 干跑必须用**即将给出的那个顺序**（`self.keep`），不能自己重排 ——
            // 否则策略算"放得下"、图集照 `keep` 复验却放不下，那条测试就会变成
            // 在测"策略内部排序与图集复验不一致"，而那不是它要验的东西。
            let by_key: std::collections::HashMap<GlyphKey, &EntryInfo> =
                entries.iter().map(|e| (e.key, e)).collect();
            let probe: Vec<&EntryInfo> = self
                .keep
                .iter()
                .filter_map(|k| by_key.get(k).copied())
                .collect();
            if !pack_fits(&probe, need, side) {
                return None;
            }
            self.observed.borrow_mut().extend_from_slice(&self.keep);
            Some(EvictPlan {
                survivors: self.keep.clone(),
                dropped: entries
                    .iter()
                    .filter(|e| occupies_canvas(e) && !self.keep.contains(&e.key))
                    .count(),
            })
        }
    }

    /// 策略给的**重排顺序**被原样照搬。
    ///
    /// # 为什么策略只提名两个
    ///
    /// 为了让"策略顺序"与"默认的高度降序"必然不同，幸存者里必须有**高矮不同**的字形；
    /// 而为了让尺寸前提宽裕、不用去凑"恰好装满"的边长，又希望幸存者尽量少。两个
    /// 高矮不同的字形同时满足这两条 —— 这比用三个字形然后反复核算边长稳得多。
    ///
    /// 凑尺寸这件事本身很容易出错：32×32 下 `22 + 10 = 32` 恰好放得下，于是"以为会
    /// 触发淘汰"其实没触发；28×28 下换行后又不够放第四个，于是策略正确地返回 `None`
    /// 而测试误判成"图集没采纳策略"。两次都是前提算错，不是实现问题。
    #[test]
    fn 重排顺序由策略决定而不是图集自己排() {
        const TINY: u32 = 32;
        let mut atlas = GlyphAtlas::with_size(TINY);
        // 两个高矮不同的字形（含留白后 10×22 与 10×14），再加一个填充物把图集占满。
        atlas
            .get_or_insert(0, 2, 16, Some(raster(8, 12, Point::new(0.0, 0.0))))
            .unwrap();
        atlas
            .get_or_insert(0, 1, 16, Some(raster(8, 20, Point::new(0.0, 0.0))))
            .unwrap();
        // 用一串窄字形把第一行填满，逼得后来的字形必须换行。
        for n in 10..14u32 {
            atlas
                .get_or_insert(0, n, 16, Some(raster(8, 6, Point::new(0.0, 0.0))))
                .unwrap();
        }

        // 策略只保留两个，顺序是「矮的在前、高的在后」—— 与默认的高度降序相反。
        let order = vec![GlyphKey::new(0, 2, 16), GlyphKey::new(0, 1, 16)];
        let (policy, observed) = KeepJust::new(order.clone());
        atlas.set_policy(policy);

        atlas
            .get_or_insert(0, 99, 16, Some(raster(8, 8, Point::new(0.0, 0.0))))
            .expect("策略只要求保留两个小字形，必然放得下");

        assert_eq!(
            observed.borrow().as_slice(),
            order.as_slice(),
            "图集必须照策略给的顺序复验 —— 它就是将来的落盘顺序"
        );

        // 落盘真的照这个顺序：矮的（n=2，占位高 14）在最前，高的（n=1，占位高 22）在后。
        //
        // 判据用 **x** 而不是 y：两者的占位都是 10 宽，32 宽的行放得下三个，所以它们
        // 必然**同处一行** —— 此时先后只体现在 x 上。若图集自己按高度降序排，
        // n=1 会占 x=1，n=2 退到 x=11，与策略意图正好相反。
        let xy_of = |n: u32| {
            let e = atlas.get(0, n, 16).unwrap().uv;
            (e.x, e.y)
        };
        let (x1, _) = xy_of(1);
        let (x2, _) = xy_of(2);
        assert!(
            x2 < x1,
            "策略把 n=2 排在最前 ⇒ 它应当在左边(x 更小)。\
             实得 x(n=2)={x2}、x(n=1)={x1}。若 n=2 反而在右，说明图集自己按高度降序重排了"
        );
        // 策略没提名的那些（填充物）应当被丢。
        for n in 10..14u32 {
            assert!(
                atlas.get(0, n, 16).is_none(),
                "字形 {n} 不在策略的幸存者名单里，应当被丢"
            );
        }
    }

    /// 一个**故意给出放不下顺序**的策略：图集必须拒绝，且**不碰画布**。
    ///
    /// 这是复验存在的全部理由。策略是外部代码，它可能算错；而画布一旦被清空又没有
    /// 重排成功，症状是"所有文字消失且无任何报错" —— 最难查的一类。
    #[test]
    fn 策略给出放不下的顺序时_图集拒绝且不清空画布() {
        let mut atlas = fill_small();
        let before_ink = inked(&atlas);
        let before_rev = atlas.revision();
        assert_eq!(before_ink, 36 * 64, "前置条件：画布是满的");

        // 要求保留全部 36 个（放不下，因为还要塞第 37 个），但只报 dropped=1。
        let all: Vec<GlyphKey> = (0..36u32).map(|n| GlyphKey::new(0, n, 16)).collect();
        let (policy, _) = KeepJust::new(all);
        atlas.set_policy(policy);

        let result = atlas.get_or_insert(0, 999, 16, Some(raster(8, 8, Point::new(0.0, 0.0))));

        assert!(
            result.is_none(),
            "放不下就必须放弃 —— 返回条目等于把字形写到了越界位置"
        );
        assert_eq!(
            inked(&atlas),
            before_ink,
            "画布绝不能被清空：清空而不重排 ⇒ 屏幕上所有文字消失且无报错"
        );
        assert_eq!(
            atlas.revision(),
            before_rev,
            "没发生重排就不该推进版本号，否则宿主白重传一次 4MB"
        );
        assert_eq!(
            atlas.stats().glyphs,
            36,
            "一个条目都不该丢 —— 复验失败时不做任何改动"
        );
        // 原有的 36 个还都在，且 uv 未变。
        for n in 0..36u32 {
            assert!(
                atlas.get(0, n, 16).is_some(),
                "字形 {n} 不该被丢"
            );
        }
    }

    /// 空白字形绝不能被当成"可淘汰对象"。
    ///
    /// 空白字形（空格）有 `last_used`（会被取用、会刷新新鲜度），所以在按时间排序时
    /// 必然混在里面。若策略把它算进分母，图集紧张时会先丢空格 —— 而丢空格腾不出
    /// 任何空间，于是白白触发一连串淘汰。
    #[test]
    fn 空白字形不会被策略当成可淘汰对象() {
        let mut atlas = GlyphAtlas::with_size(SMALL);
        // 塞满可见字形。
        for n in 0..36u32 {
            atlas
                .get_or_insert(0, n, 16, Some(raster(8, 8, Point::new(0.0, 0.0))))
                .unwrap();
        }
        // 现在插一批空白字形。它们的 `last_used` 最大（最新鲜）。
        for n in 100..110u32 {
            atlas.get_or_insert(0, n, 16, None).unwrap();
        }

        // 触发淘汰：应当丢**可见**字形里最旧的一个，而不是空白。
        atlas
            .get_or_insert(0, 999, 16, Some(raster(8, 8, Point::new(0.0, 0.0))))
            .unwrap();
        assert_eq!(atlas.stats().evictions, 1, "只需要腾一个 10×10 的位置");
        assert!(
            atlas.get(0, 100, 16).is_some(),
            "空白字形不该被丢 —— 它腾不出任何空间"
        );
        assert!(
            atlas.get(0, 0, 16).is_none(),
            "最旧的**可见**字形才是该丢的那个"
        );
        // 全部空白条目都必须还在：它们是有效缓存（下次取用不必重跑栅格化），
        // 且 `offset` 决定光标推进。
        for n in 100..110u32 {
            assert!(
                atlas.get(0, n, 16).is_some(),
                "空白字形 {n} 在重排后消失了 —— 它不占画布，但条目必须无条件保留"
            );
        }
    }

    /// 空白条目在重排后必须**全部**幸存，且不占画布。
    ///
    /// 这条钉住的是一条容易被"优化"掉的逻辑：淘汰策略只提名占画布的条目，空白条目
    /// 不在名单里；若重排不把它们捞回来，它们会被静默丢弃。症状不是崩溃，而是
    /// 「同一个空格在重排前后表现不同」—— 极难定位。
    #[test]
    fn 重排后空白条目全部保留且不占画布() {
        let mut atlas = fill_small();
        let blank_keys: Vec<GlyphKey> = (200..205u32).map(|n| GlyphKey::new(0, n, 16)).collect();
        for k in &blank_keys {
            atlas.get_or_insert(k.font_id, k.glyph_id, k.px_size, None).unwrap();
        }
        let used_before = atlas.stats().used_px;

        // 触发淘汰 + 重排。
        atlas
            .get_or_insert(0, 999, 16, Some(raster(8, 8, Point::new(0.0, 0.0))))
            .unwrap();
        assert_eq!(atlas.stats().evictions, 1, "前置条件：应当只丢一个可见字形");

        for k in &blank_keys {
            let e = atlas
                .get(k.font_id, k.glyph_id, k.px_size)
                .unwrap_or_else(|| panic!("空白条目 {k:?} 在重排后不见了"));
            assert_eq!(e.size, Size::new(0.0, 0.0), "空白字形尺寸必须仍为 0");
            assert_eq!(e.uv, Rect::new(0.0, 0.0, 0.0, 0.0), "空白字形没有 uv");
        }
        // 且它们没有推进货架 —— 否则会占掉画布空间（`used_px` 会多出一行）。
        assert!(
            atlas.stats().used_px <= used_before,
            "空白字形不该占画布：重排后已用高度 {} > 之前 {}",
            atlas.stats().used_px,
            used_before
        );
        // 统计里不计入可见字形。
        assert_eq!(atlas.stats().glyphs, 36, "丢一个进一个 ⇒ 可见字形数不变");
    }

    /// `LruEviction` 自己就该把空白排除掉 —— 上面那条是端到端，这条钉住单元。
    #[test]
    fn 默认策略直接把空白条目排除在分母外() {
        let real = EntryInfo {
            key: GlyphKey::new(0, 1, 16),
            padded: (10, 10),
            last_used: 1,
        };
        let blank = EntryInfo {
            key: GlyphKey::new(0, 2, 16),
            padded: (0, 0),
            // 最"新鲜" —— 若策略不排除空白，它会被排在最后保留、占着幸存者名额。
            last_used: 999,
        };
        // 图集小到只放得下一个 10×10：必须丢掉那个真实字形才能腾出地方。
        let plan = LruEviction
            .evict(&[blank, real], (10, 10), 12)
            .expect("丢掉唯一的真实字形后应当放得下");
        assert_eq!(plan.dropped, 1, "只该丢那个真实字形 —— 丢空白腾不出空间");
        // 幸存者里**不该有真实字形**（它被丢了），空白字形也**不在**里面 ——
        // 空白条目由图集无条件保留（`rebuild` 里单独处理），策略不必也不该提名它。
        assert!(
            !plan.survivors.contains(&real.key),
            "最旧的真实字形该被丢：{:?}",
            plan.survivors
        );
        assert!(
            !plan.survivors.contains(&blank.key),
            "空白不占画布，策略不该把它算进重排名单：{:?}",
            plan.survivors
        );

        // need 为零（空白字形）时不该触发任何淘汰。
        let noop = LruEviction.evict(&[blank, real], (0, 0), 12).unwrap();
        assert_eq!(noop.dropped, 0, "空白字形永远放得下，不该淘汰");
        assert!(noop.survivors.is_empty());
    }
}

//! 全局分配器探针 —— **逐字取自** xamples/alloc_probe.rs。
//!
//! 复制而不是共享：它在 examples 里是 mod alloc_probe;，而 gallery 是另一个 crate。
//! 两处内容必须**逐字一致**（否则两边的分配统计不可比），所以改动时同步改两处。

//! 临时诊断探针：给 `--bench` 加全局分配计数。
//!
//! # 目的
//!
//! `--bench` 已经能量出"哪一阶段慢"，但答不了"那一阶段在分配什么"。本探针用
//! `#[global_allocator]` 包一层计数器，把每次 `alloc` 的次数与字节数记进两个
//! 原子量，于是能把"分配次数"当成和耗时一样可测量的指标。
//!
//! ⚠ **这是诊断代码，不是产品代码**：`System` 是全局的、计数是原子的，它给每个
//! 分配加约两次原子操作，会把耗时数字本身抬高。所以耗时结论必须以**不带探针**的
//! `--bench` 为准；探针只用来数"分配了几次、每次多大"。
//!
//! 用法：`cargo run --release --bin demo -- --bench --alloc-probe`

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};

/// 累计分配次数。
static ALLOCS: AtomicU64 = AtomicU64::new(0);
/// 累计分配字节。
static BYTES: AtomicU64 = AtomicU64::new(0);
/// 单次分配的最大字节。
static MAX_ONE: AtomicU64 = AtomicU64::new(0);

/// 按大小分桶的次数。桶按 2 的幂上界，`u = bucket + 16`。
///
/// 为什么按幂分桶而不是精确字节：`PropertyValue` 里混着 `f32`（无分配）、
/// `Arc<str>`（一次分配 + 引用计数）和 `Vec`（一次分配 + 后续扩容），尺寸跨越
/// 两个数量级。幂桶能把它们分成"小对象 / 中对象 / 大对象"三组，而每组几乎总是
/// 对应一种数据结构 —— 那正是要认出来的东西。
const BUCKETS: usize = 40;
/// 每桶的次数与字节数。
static HIST: [AtomicU64; BUCKETS * 2] = {
    #[allow(clippy::declare_interior_mutable_const)]
    const Z: AtomicU64 = AtomicU64::new(0);
    [Z; BUCKETS * 2]
};

#[inline]
fn bucket_of(n: usize) -> usize {
    // `usize::BITS - leading_zeros` 给出"需要几个二进制位"，即 `ceil(log2(n+1))`。
    let b = (usize::BITS - n.leading_zeros()) as usize;
    b.min(BUCKETS - 1)
}

#[inline]
fn record(n: usize) {
    let i = bucket_of(n) * 2;
    HIST[i].fetch_add(1, Ordering::Relaxed);
    HIST[i + 1].fetch_add(n as u64, Ordering::Relaxed);
}

/// 包一层的全局分配器。
struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(l.size() as u64, Ordering::Relaxed);
        MAX_ONE.fetch_max(l.size() as u64, Ordering::Relaxed);
        record(l.size());
        System.alloc(l)
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        System.dealloc(p, l)
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
        // `realloc` 可能缩容，`new < l.size()` 时不该计入"分配字节"的净值 ——
        // 但我们量的是"申请过多少"，所以记 `new`。
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(new as u64, Ordering::Relaxed);
        MAX_ONE.fetch_max(new as u64, Ordering::Relaxed);
        record(new);
        System.realloc(p, l, new)
    }
}

/// 把全局分配器换成计数的那个。
#[global_allocator]
static A: Counting = Counting;

/// 一次采样的计数。
#[derive(Debug, Default, Clone, Copy)]
pub struct Counts {
    pub allocs: u64,
    pub bytes: u64,
    pub max_one: u64,
}

impl Counts {
    /// 读当前值。
    pub fn now() -> Self {
        Self {
            allocs: ALLOCS.load(Ordering::Relaxed),
            bytes: BYTES.load(Ordering::Relaxed),
            max_one: MAX_ONE.load(Ordering::Relaxed),
        }
    }
    /// 两次采样之差（注意 `max_one` 取的是区间内的累计最大值，不能相减）。
    pub fn since(&self, prev: &Self) -> Self {
        Self {
            allocs: self.allocs - prev.allocs,
            bytes: self.bytes - prev.bytes,
            max_one: self.max_one,
        }
    }
}

/// 直方图的一行：`(上界字节, 次数, 总字节)`，只含**有过分配**的桶。
pub fn histogram() -> Vec<(usize, u64, u64)> {
    (0..BUCKETS)
        .filter_map(|b| {
            let n = HIST[b * 2].load(Ordering::Relaxed);
            if n == 0 {
                return None;
            }
            let bytes = HIST[b * 2 + 1].load(Ordering::Relaxed);
            // 桶 `b` 覆盖 `(2^(b-1), 2^b]`；`b=0` 是 0 或 1 字节。
            let hi = if b == 0 { 1 } else { 1usize << b };
            Some((hi, n, bytes))
        })
        .collect()
}

/// 直方图归零（下一段采样从零开始）。
pub fn reset_histogram() {
    for c in HIST.iter() {
        c.store(0, Ordering::Relaxed);
    }
}


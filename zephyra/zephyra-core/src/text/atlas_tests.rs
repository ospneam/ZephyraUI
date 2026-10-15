//! `GlyphAtlas` 的**坐标契约** —— core 侧，不依赖任何字体。
//!
//! # 这条契约是什么
//!
//! [`GlyphAtlas::get_or_insert`] 收到一个 [`AtlasRaster`]，把 `coverage` 原样搬进图集。
//! 它的前提是：
//!
//! > **`coverage` 的第 `(0,0)` 个像素就是字形左上角**，
//! > 行宽 = `width`，行数 = `height`。
//!
//! 也就是说 `coverage` 是"**字形自身**的位图"，与 `offset`（字形左上角相对基线原点的
//! 偏移）**无关**。
//!
//! # 为什么值得用一整个测试文件钉它
//!
//! 这个契约在实现里被违反过一次，而且症状极端：
//!
//! `atlas.rs` 原先写的是 `ax = sx - floor(offset.x); ay = sy - floor(offset.y);` ——
//! 它以为 `coverage` 是相对**基线原点**的坐标。而 `GlyphSource` 的实现（ab_glyph 的
//! `OutlinedGlyph::draw`）给的回调坐标**本来就已相对 `px_bounds.min`**。于是偏移被减了
//! 两次，而大写字母的 `floor(offset.y)` 就是 cap 高度（32px 的 `'H'` 约 22）——
//! `ay` 恒为负，被越界判断全部跳过，**图集里一个字形的墨都没有**。
//!
//! 屏幕上表现为「文字完全不出现」。而这与「GPU 画不出东西」在现象上**无法区分** ——
//! 本项目已经因此查了 D3D11 视口好几十轮。
//!
//! 所以这里用一个**假**字形来源把契约钉死：不依赖字体，任何机器上都能跑，
//! 而且失败信息直接指向「坐标空间搞错了」而不是「字没画出来」。
//!
//! # 为什么不用真字体
//!
//! 因为契约是 core 与 `GlyphSource` **之间**的约定，与具体字体无关。用真字体会让这个
//! 测试依赖系统字体（换台机器就可能跳过），而契约本身不需要字体就能验。
//!
//! # 契约的另一半：`uv` 指的是**墨**，四周另有一圈留白
//!
//! 本文件验的是"`coverage` 怎样落进图集"。而对**采样**这一侧另有一条：返回的
//! `AtlasGlyph::uv` 描述的是**墨**的矩形，图集在那个矩形外面还留了一圈透明像素
//! （`GLYPH_PADDING`）。
//!
//! 两者缺一不可：`uv` 若把留白算进去，`GlyphQuad::dst` 会跟着变大，字之间凭空多出
//! 缝隙；留白若没有，双线性采样会在四边形边缘各取一半邻居的纹素，字之间出现淡竖条。
//! 留白那半边由 `atlas.rs` 里的 `transparent_gap_between_adjacent_glyphs` 与本文件下面那条
//! 「尺寸一致」共同钉住。

// 这个文件在 **core 内部**，所以用 `crate::` 而不是 `zephyra_core::`。
// 写成 `zephyra_core::` 会在 core 里指向"依赖自己的那个 crate"，而 core 没有自依赖 ——
// 那种写法只可能在 core 外的测试里成立。
use crate::core::Point;
use crate::text::atlas::{AtlasRaster, GlyphAtlas, ATLAS_SIZE};
use crate::text::{GlyphSource, TextEngine, TextRender, TextStack, TextStyle};

/// 一个只产出一个固定位图的假字形来源。
///
/// 唯一目的是给 `get_or_insert` 一个**已知内容**的 `coverage`，好让断言能说出
/// 「图集里第几个像素该是多少」。
struct OneGlyph {
    font_id: u32,
    /// 真正会被返回的位图。
    raster: AtlasRaster,
}

impl GlyphSource for OneGlyph {
    fn glyph_id(&mut self, _ch: char) -> u32 {
        1
    }

    fn advance(&mut self, _glyph_id: u32, _px_size: f32) -> f32 {
        10.0
    }

    fn rasterize(&mut self, _glyph_id: u32, _px_size: u32) -> Option<AtlasRaster> {
        Some(self.raster.clone())
    }

    fn font_id(&self) -> u32 {
        self.font_id
    }
}

/// 一个 4×3 的位图，覆盖率沿对角线递增。
///
/// 用非平凡的数值（不是全 0 / 全 255）才能区分「写错位置」与「写错值」。
fn diag_bitmap() -> AtlasRaster {
    let (w, h) = (4u32, 3u32);
    let mut coverage = vec![0u8; (w * h) as usize];
    for y in 0..h {
        for x in 0..w {
            coverage[(y * w + x) as usize] = 40 + (y * w + x) as u8 * 10;
        }
    }
    AtlasRaster {
        // **故意给一个大 offset** —— 契约就是「offset 不影响 coverage 的落点」。
        // 若实现里减了 offset，这个测试就会失败（而真字体的大写字母正是这种情况）。
        offset: Point::new(3.0, 22.0),
        width: w,
        height: h,
        coverage,
    }
}

fn atlas_of(raster: AtlasRaster) -> GlyphAtlas {
    let mut atlas = GlyphAtlas::new();
    let mut src = OneGlyph {
        font_id: 7,
        raster,
    };
    let entry = atlas
        .get_or_insert(src.font_id(), 1, 16, src.rasterize(1, 16))
        .expect("图集不该在第一个字形上就满");
    assert!(entry.size.width > 0.0 && entry.size.height > 0.0);
    atlas
}

/// 取图集里某点的 alpha 分量。
fn alpha_at(atlas: &GlyphAtlas, x: u32, y: u32) -> u8 {
    atlas.pixels()[(y as usize * ATLAS_SIZE as usize + x as usize) * 4 + 3]
}

/// 图集里的每个像素必须与 `coverage` 的对应像素**逐字相同**。
#[test]
fn coverage_原样落进图集_不因_offset_而移位() {
    let raster = diag_bitmap();
    let atlas = atlas_of(raster.clone());

    let entry = atlas.get(7, 1, 16).expect("刚插入的字形应当查得到");

    // `AtlasGlyph::uv` 记录的是图集里的**像素**位置（不是 0..1）。
    let (ox, oy) = (entry.uv.x as u32, entry.uv.y as u32);

    for sy in 0..raster.height {
        for sx in 0..raster.width {
            let want = raster.coverage[(sy * raster.width + sx) as usize];
            let got = alpha_at(&atlas, ox + sx, oy + sy);
            assert_eq!(
                got, want,
                "coverage({sx},{sy}) 应落在图集 ({}, {})，但读到 {got}，期望 {want}",
                ox + sx,
                oy + sy
            );
        }
    }
}

/// 整块位图必须**整体**落在图集内，且 `AtlasGlyph::size` 与位图尺寸一致。
///
/// 「尺寸一致」这条不是废话：`AtlasGlyph::size` 会被拿去算 `GlyphQuad::dst`，
/// 而 `dst` 的尺寸要能与 `uv` 反推出的像素尺寸对上，否则采样会错位。
#[test]
fn bitmap_size_matches_atlas_entry_strictly() {
    let raster = diag_bitmap();
    let atlas = atlas_of(raster.clone());
    let entry = atlas.get(7, 1, 16).unwrap();

    assert_eq!(entry.size.width, raster.width as f32);
    assert_eq!(entry.size.height, raster.height as f32);
    assert_eq!(entry.uv.width, raster.width as f32);
    assert_eq!(entry.uv.height, raster.height as f32);

    // uv 必须在图集内。
    assert!(entry.uv.x + entry.uv.width <= ATLAS_SIZE as f32);
    assert!(entry.uv.y + entry.uv.height <= ATLAS_SIZE as f32);
}

/// 相邻字形**不得互相啃掉**。
///
/// 透明像素不写 —— 所以先插一个满图，再插一个只占左上半的位图，右侧应当保留满图的
/// 内容。早期版本无条件写 `coverage`（含 0），于是每个字形的透明区域都把邻居擦成 0。
#[test]
fn later_glyph_insert_does_not_erase_existing_ones() {
    let mut atlas = GlyphAtlas::new();

    // 先放一个铺满 64×64 的不透明块。
    let big = AtlasRaster {
        offset: Point::new(0.0, 0.0),
        width: 64,
        height: 64,
        coverage: vec![200; (64 * 64) as usize],
    };
    let big_entry = atlas
        .get_or_insert(1, 1, 16, Some(big))
        .expect("应当放得下");
    let (bx, by) = (big_entry.uv.x as u32, big_entry.uv.y as u32);

    // 再放一个 4×3 的位图；它会落在 (64, 0) 附近，右边是满图的范围。
    let small = diag_bitmap();
    let small_entry = atlas
        .get_or_insert(2, 1, 16, Some(small))
        .expect("应当放得下");

    // 满图**最后一行**仍在。
    let last_row_y = by + 63;
    let still_there = (bx..bx + 64)
        .all(|x| alpha_at(&atlas, x, last_row_y) == 200);
    assert!(
        still_there,
        "第二个字形插入后，第一个字形的最后一行被擦了 —— 透明像素不该覆盖已有内容"
    );

    // 小位图之外不该有小位图的墨。
    let (sx, sy) = (small_entry.uv.x as u32, small_entry.uv.y as u32);
    assert_eq!(alpha_at(&atlas, sx + 3, sy + 2), 40 + 11 * 10);
}

/// 空白字形**不占空间**，但**必须**占一个条目。
///
/// 「必须占条目」是为了不重复栅格化；「不占空间」是为了不让它把货架推进一格 ——
/// 那会在字形之间留下永远填不满的缝。
#[test]
fn blank_glyph_costs_no_space_but_has_an_entry() {
    let mut atlas = GlyphAtlas::new();
    let before = atlas.stats().usage();

    let blank = AtlasRaster {
        offset: Point::new(0.0, 0.0),
        width: 0,
        height: 0,
        coverage: Vec::new(),
    };
    let e = atlas
        .get_or_insert(9, 5, 16, Some(blank))
        .expect("空白字形不该被当成图集满");
    assert_eq!(e.size.width, 0.0);
    assert_eq!(e.size.height, 0.0);

    assert!(
        atlas.get(9, 5, 16).is_some(),
        "空白字形必须留一个条目 —— 否则每帧都要重跑栅格化才发现「原来是空的」"
    );
    assert_eq!(
        atlas.stats().usage(),
        before,
        "空白字形推进了货架游标 —— 它会在字形之间留下永远填不满的缝"
    );
}

/// 重复插入**不新增条目**（缓存命中）。
#[test]
fn reinserting_same_glyph_hits_cache() {
    let raster = diag_bitmap();
    let mut atlas = GlyphAtlas::new();
    let a = atlas.get_or_insert(7, 1, 16, Some(raster.clone())).unwrap();
    let n = atlas.stats().glyphs;
    let b = atlas.get_or_insert(7, 1, 16, Some(raster)).unwrap();
    assert_eq!(a.uv, b.uv, "同一键两次插入返回了不同的位置");
    assert_eq!(atlas.stats().glyphs, n, "第二次插入新增了条目 —— 缓存没命中");
}

/// 字号不同 ⇒ 不同位图。
///
/// 同一个字形在不同像素字号下是不同的图，混用会得到"缩放后字形模糊/错位"。
#[test]
fn different_font_sizes_are_distinct_entries() {
    let raster = diag_bitmap();
    let mut atlas = GlyphAtlas::new();
    atlas.get_or_insert(7, 1, 16, Some(raster.clone())).unwrap();
    atlas.get_or_insert(7, 1, 32, Some(raster)).unwrap();
    assert_eq!(
        atlas.stats().glyphs,
        2,
        "字号不同却被合并成同一条目 —— 同一个字形会以错误的位图被采样"
    );
}

/// `GlyphSource` 的契约在 core 侧也要能被检查：形状 + 降级。
///
/// 这一条不测字形，只测「core 会把 `PlacedGlyph` 送进 `GlyphSource::rasterize`」。
#[test]
fn shaping_to_rasterization_chain_runs_end_to_end() {
    let raster = diag_bitmap();
    let src = OneGlyph {
        font_id: 3,
        raster,
    };
    let mut stack = TextStack::new(src);
    let shaped = TextEngine::shape(
        &mut stack,
        "AB",
        &TextStyle {
            size: 16.0,
            ..Default::default()
        },
        None,
    );
    assert_eq!(shaped.lines.len(), 1);
    assert_eq!(shaped.lines[0].glyphs.len(), 2);

    let (_handle, quads) = TextRender::glyph_quads(
        &mut stack,
        &shaped,
        Point::new(0.0, 20.0),
        &TextStyle {
            size: 16.0,
            ..Default::default()
        },
    )
    .expect("有字形来源时不该返回 None");
    assert_eq!(quads.len(), 2, "\"AB\" 是两个非空白字形");
    for q in &quads {
        assert!(q.uv.width > 0.0 && q.uv.height > 0.0);
        assert!(q.uv.right() <= 1.0 && q.uv.bottom() <= 1.0);
    }
}
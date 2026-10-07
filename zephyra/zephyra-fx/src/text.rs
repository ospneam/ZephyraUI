//! 字形来源：把 `ab_glyph` 适配到 base 层为文字做的**唯一**一件事。
//!
//! # 为什么只做这一件事
//!
//! 文本排版的一切都在 core 的 [`zephyra_core::text`]：拆行、基线、逐字形四边形、图集打包
//! 与 uv。这里只回答一个问题：**怎么把字符变成位图**，也就是怎么把 `.ttf` / `.otf`
//! 在内存里接成 core 的 [`GlyphSource`]。
//!
//! ```text
//! core                              base（这里）
//! 排版 + 图集 + uv → Command::Text   AbGlyphSource：唯一实现
//! GlyphSource  ?───────────────────▶
//! TextStack    拆行 + 基线 + 打包
//! GlyphAtlas   CPU 位图            宿主 upload_atlas(pixels) → GPU 纹理
//! ```
//!
//! 把这层抽成独立 crate（`ab_glyph` 或 `cosmic-text` 或 CoreText）= 换一个实现，
//! core 一行不用改。
//!
//! # 与 Skia 的一个关键差异：不做子像素位置量化
//!
//! Skia 的 `SkPackedGlyphID`（`src/core/SkGlyph.h`）把「字形 id + 子像素 x + 子像素 y」
//! 打包进一个 u32，其中子像素位置**只有 2 bit** —— 即量化到 1/4 像素：
//!
//! ```c
//! static constexpr uint32_t kSubPixelPosLen = 2u;
//! x = (x - floorf(x)) + 1.0f;
//! sub_x = (int)(x * (1u << (kSubPixelPosLen + kSubPixelX))) & kSubPixelPosMask;
//! ```
//!
//! **这里刻意不做这件事**，因为两者的架构前提不同：
//!
//! | | Skia | 本项目 |
//! |---|---|---|
//! | 位图按什么位置栅格化 | 每个量化子像素位置各存一份 | 每个 (font, glyph, px) **一份** |
//! | 绘制时 | 画在整数位置 + 量化后的子像素偏移 | 顶点是浮点，**GPU 自己**做亚像素光栅化 |
//!
//! Skia 要量化，是因为它把位图缓存当作**最终产物** —— 位置必须进键。而本项目把位图只当作
//! 「覆盖率来源」，落点是浮点顶点，由 GPU 做亚像素采样。照搬量化会：
//!
//! - 把字形位置**吸附到 1/4 像素**，小字号下肉眼可见地抖；
//! - 换不到任何好处 —— 我们的键本来就没有子像素这一维。
//!
//! 所以「参考 Skia」在这里的结论是**不要**照抄这一项。
//!
//! # `AtlasRaster.coverage` 的坐标契约（踩过一次）
//!
//! 本模块返回的 `coverage` 的 `(0,0)` 就是**字形左上角**，行宽 = `width`。
//! 它**不是**相对基线原点的坐标 —— 基线偏移由 [`AtlasRaster::offset`] 单独给出，
//! 由 core 拿去算 `GlyphQuad::dst`。
//!
//! 违反这条的后果极端：core 的图集原本据此把 `offset` 又减了一次，而大写字母的
//! `floor(offset.y)` 就是 cap 高度（32px 的 'H' 约 22），于是每个像素都被越界判断
//! 跳过 —— **图集全空，屏幕上字完全不出现**。见 `zephyra/src/text/atlas.rs` 里的修正，
//! 以及 `zephyra/src/text/atlas_tests.rs` 里用假字形来源钉死的契约测试。
use ab_glyph::{Font, FontVec, GlyphId, PxScale, ScaleFont};
use zephyra_core::core::Point;
use zephyra_core::text::{AtlasRaster, GlyphSource, TextStack};

/// 基于 ab_glyph 的字形来源。
///
/// 只做三件事：查字形 id、算步进、栅格化。三者都是 `ab_glyph` 的直接映射 —— 这里
/// 没有任何"排版策略"，那全在 core 的 [`TextStack`] 里。
#[derive(Debug)]
pub struct AbGlyphSource {
    font: FontVec,
    font_id: u32,
}

impl AbGlyphSource {
    /// 从字体字节构造。
    ///
    /// `font_id` 是**调用方给的稳定整数**，而不是字体指针或名称的哈希：图集要按
    /// `(font_id, glyph_id, px_size)` 做键，若用哈希字符串，每帧每个字形都要算一次
    /// 字符串哈希。同一个 `font_id` 必须始终对应同一份字体字节 —— 否则同一个字形会
    /// 在图集里存多份。
    pub fn from_bytes(font_id: u32, bytes: &[u8]) -> Result<Self, String> {
        let font = FontVec::try_from_vec(bytes.to_vec())
            .map_err(|e| format!("字体解析失败（可能是 CFF/可变字体或文件已损坏）: {e}"))?;
        Ok(Self { font, font_id })
    }

    /// 字体里是否真的有这个字形。
    ///
    /// `ab_glyph` 的 `glyph_id` 对缺失字符返回 `.notdef`（id 0），**不是**错误。
    /// 而多数字体的 `.notdef` 是空轮廓 —— 缺字会静默消失。所以要显示豆腐框就得
    /// 显式替换，而 [`GlyphSource::glyph_id`] 的文档要求实现做这件事。
    fn has_glyph(&self, id: GlyphId) -> bool {
        id.0 != 0
    }
}

/// 候选系统字体路径。
///
/// 刻意**不**引入 `fontdb` 去枚举字体目录：那会遍历数百个文件（本机实测上千个），
/// 而这里只需要一个能显示字形的字体。真实应用应当用 fontdb 加用户偏好。
///
/// # ⚠ 首个候选不含 CJK —— 这是本表最容易被误读的地方
///
/// `segoeui.ttf` 只有拉丁与部分符号，**没有汉字**。而 [`from_system_font`] 命中第一个
/// 就返回，所以排第二的 `msyh.ttc`（含 CJK）**永远轮不到**，中文一律走 U+FFFD
/// 变豆腐块。
///
/// 顺序不是「按显示效果排的」，只是「挑一个存在的字体」—— 曾经没人在意拉丁字体的
/// 观感，于是中文坏掉也没人发现。要显示中文就设 [`FONT_OVERRIDE_ENV`]，
/// 或把一份含 CJK 的字体挪到表首（代价：拉丁观感变）。
const FONT_CANDIDATES: &[&str] = &[
    // Windows
    "C:\\Windows\\Fonts\\segoeui.ttf",
    "C:\\Windows\\Fonts\\msyh.ttc", // 微软雅黑 —— 含 CJK
    "C:\\Windows\\Fonts\\arial.ttf",
    "C:\\Windows\\Fonts\\tahoma.ttf",
    // Linux
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
    "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
    // macOS
    "/System/Library/Fonts/Helvetica.ttc",
    "/System/Library/Fonts/PingFang.ttc",
];

/// 环境变量名：指定字体文件的绝对或相对路径，**优先于**内置候选表。
///
/// # 为什么需要它
///
/// `FONT_CANDIDATES` 里第一个 `segoeui.ttf` **不含 CJK**，于是中文全变豆腐块
/// （`glyph_id` 退到 U+FFFD）。这不是 bug，是那份字体的客观事实 —— 候选表里
/// 排第二的 `msyh.ttc` 含 CJK，但它**永远轮不到**，因为 `from_system_font` 命中第一个
/// 就返回。
///
/// 换掉首选字体的两种办法：
///
/// - 把系统首选改成一份含 CJK 的字体 —— 代价是拉丁字母的观感随机器而变；
/// - 指定一份随程序分发的字体 —— 观感在任何机器上都一样。
///
/// demo 走后者（`examples/fonts/MiSans-Regular.ttf`，含 CJK 与拉丁）。
pub const FONT_OVERRIDE_ENV: &str = "ZEPHYRA_FONT";

/// 从指定路径造一个文本栈。
///
/// 与 [`from_system_font`] 的差别只有「字体从哪来」，其余（失败时退回禁用模式而不
/// panic）完全一致。
pub fn from_font_file(path: &str) -> (TextStack<AbGlyphSource>, String) {
    match std::fs::read(path) {
        Ok(bytes) if !bytes.is_empty() => match AbGlyphSource::from_bytes(0, &bytes) {
            Ok(src) => {
                let stack = TextStack::new(src);
                (
                    stack,
                    format!(
                        "字体 {path}（{} 字节，图集 {}×{}）",
                        bytes.len(),
                        zephyra_core::text::atlas::ATLAS_SIZE,
                        zephyra_core::text::atlas::ATLAS_SIZE
                    ),
                )
            }
            Err(e) => (
                TextStack::disabled(AbGlyphSource {
                    font: FontVec::try_from_vec(minimal_sfnt())
                        .expect("最小 sfnt 头应可解析"),
                    font_id: u32::MAX,
                }),
                format!("字体 {path} 解析失败（{e}），已退回无文本模式"),
            ),
        },
        Ok(_) => (
            TextStack::disabled(AbGlyphSource {
                font: FontVec::try_from_vec(minimal_sfnt()).expect("最小 sfnt 头应可解析"),
                font_id: u32::MAX,
            }),
            format!("字体 {path} 是空文件，已退回无文本模式"),
        ),
        Err(e) => (
            TextStack::disabled(AbGlyphSource {
                font: FontVec::try_from_vec(minimal_sfnt()).expect("最小 sfnt 头应可解析"),
                font_id: u32::MAX,
            }),
            format!("读不到字体 {path}（{e}），已退回无文本模式"),
        ),
    }
}

/// 挑一个可用的系统字体 —— 或用 [`FONT_OVERRIDE_ENV`] 指定的字体。
///
/// # 优先读环境变量
///
/// 设了 [`FONT_OVERRIDE_ENV`] 就用它，且**不**再尝试系统字体。这样「指定字体」这件事
/// 是宿主的事，不用改 `zephyra-fx` 的候选表。
pub fn from_system_font() -> (TextStack<AbGlyphSource>, String) {
    if let Ok(path) = std::env::var(FONT_OVERRIDE_ENV) {
        if !path.is_empty() {
            return from_font_file(&path);
        }
    }
    let mut tried = Vec::new();
    for (i, path) in FONT_CANDIDATES.iter().enumerate() {
        match std::fs::read(path) {
            Ok(bytes) if !bytes.is_empty() => {
                match AbGlyphSource::from_bytes(i as u32, &bytes) {
                    Ok(src) => {
                        let stack = TextStack::new(src);
                        return (
                            stack,
                            format!(
                                "字体 {}（{} 字节，图集 {}×{}）",
                                path,
                                bytes.len(),
                                zephyra_core::text::atlas::ATLAS_SIZE,
                                zephyra_core::text::atlas::ATLAS_SIZE
                            ),
                        );
                    }
                    Err(e) => tried.push(format!("{path}（{e}）")),
                }
            }
            Ok(_) => tried.push(format!("{path}（空文件）")),
            Err(_) => tried.push(path.to_string()),
        }
    }
    (
        TextStack::disabled(AbGlyphSource {
            // 占位字体：`font_id` 任意，因为 disabled 模式不产出任何字形。
            font: FontVec::try_from_vec(minimal_sfnt())
                .expect("最小 sfnt 头应可解析"),
            font_id: u32::MAX,
        }),
        format!("未找到可用系统字体（试过 {} 个路径），已退回无文本模式", tried.len()),
    )
}

/// 最小 sfnt 头：12 字节 sfnt + 16 字节表记录，表数为 0。
///
/// 只在"无字体模式"下使用 —— 那时任何查询都返回零尺寸/零字形，字体本身永不被查询。
/// 构造它是为了让 [`FontVec`] 能构造成功（它要求非空字节）。
fn minimal_sfnt() -> Vec<u8> {
    let mut v = vec![0u8; 12 + 16];
    v[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes()); // sfnt version 1.0
    // numTables = searchRange = entrySelector = rangeShift 全为 0
    v
}

impl GlyphSource for AbGlyphSource {
    fn glyph_id(&mut self, ch: char) -> u32 {
        let id = self.font.glyph_id(ch);
        if self.has_glyph(id) {
            return id.0 as u32;
        }
        // 缺字：换成 U+FFFD（REPLACEMENT CHARACTER）。多数字体的替换字符是个豆腐框
        // 或问号 —— 让用户看见"这里有个字不认识"，好过静默留白。
        let replacement = self.font.glyph_id('\u{FFFD}');
        if self.has_glyph(replacement) {
            replacement.0 as u32
        } else {
            // 连替换字符都没有：退回 .notdef（多半是空轮廓）。
            id.0 as u32
        }
    }

    fn advance(&mut self, glyph_id: u32, px_size: f32) -> f32 {
        self.font
            .as_scaled(PxScale::from(px_size.max(1.0)))
            .h_advance(GlyphId(glyph_id.min(u16::MAX as u32) as u16))
    }

    fn rasterize(&mut self, glyph_id: u32, px_size: u32) -> Option<AtlasRaster> {
        let px = px_size.max(1) as f32;
        let id = GlyphId(glyph_id.min(u16::MAX as u32) as u16);
        let scaled = self.font.as_scaled(PxScale::from(px));

        // 空白字形（空格、零宽字符）没有轮廓 —— 返回 `None`，core 会给它一个零尺寸
        // 图集条目并推进光标（步进仍由 `advance` 给出）。
        let outlined = scaled.outline_glyph(id.with_scale(px))?;

        let bounds = outlined.px_bounds();
        // ⚠ 尺寸必须用**截断**，不是 `ceil`。
        //
        // `ab_glyph` 的 `OutlinedGlyph::draw` 内部是这样开栅格器的（0.2.32，`outlined.rs`）：
        //
        // ```text
        // let (w, h) = (self.px_bounds.width() as usize,   // `as usize` = 截断
        //               self.px_bounds.height() as usize);
        // let offset = self.glyph.position - self.px_bounds.min;
        // Rasterizer::new(w, h)
        // ```
        //
        // 也就是说回调收到的坐标**已经在** `[0, w) × [0, h)` 里，`w`/`h` 是截断值。
        // 若这里按 `ceil` 分配缓冲区并按 `ceil` 报尺寸，多出来的那一行/一列
        // **永远是 0** —— 而图集会把它们当作这个字形的一部分写进去，于是屏幕上
        // 每个字形边缘都出现一条上一个字形的残留。
        let (w, h) = (bounds.width() as u32, bounds.height() as u32);
        if w == 0 || h == 0 {
            return None;
        }

        // ⚠ 回调坐标**已经是**相对 `px_bounds.min` 的，不要再减一次。
        //
        // 这个函数原先写的是 `let ax = x as i32 - min.0 as i32;` —— 减了第二次。
        // 后果是缓冲区左边与上边各被砍掉 `floor(min)` 个像素；32px 的 `'H'` 那类
        // `bounds.min.y ≈ 22` 的字形会**整块丢失**（`ay` 恒为负，全部被下面的越界
        // 判断 `continue` 掉）。字形进图集时再被减第三次（见 `zephyra_core::text::atlas`），
        // 于是图集里什么都没有 —— 屏幕上"字完全不出现"。
        let mut coverage = vec![0u8; (w * h) as usize];
        outlined.draw(|x, y, alpha| {
            let (ax, ay) = (x as usize, y as usize);
            if ax >= w as usize || ay >= h as usize {
                return;
            }
            let v = (alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
            coverage[ay * w as usize + ax] = v;
        });

        Some(AtlasRaster {
            // offset 是相对**基线原点**的偏移，y 向上为正（字体约定）。
            // core 在构造 `GlyphQuad::dst` 时做 y 翻转。
            //
            // 这里用**整数**的 `min`：位图是从 `floor(min)` 开始的，而 dst 矩形要与
            // 位图严格对齐。若这里报带小数的 `min`，字形会整体偏移 `frac(min)` 像素
            // —— 在小字号下那是一整个像素的错位，且随字号变化，看起来像"字没对齐"。
            offset: Point::new(bounds.min.x.floor(), bounds.min.y.floor()),
            width: w,
            height: h,
            coverage,
        })
    }

    fn font_id(&self) -> u32 {
        self.font_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zephyra_core::text::{TextEngine, TextRender, TextStyle};

    /// 系统字体可用时才跑这些断言 —— CI 上不该因为"没装某字体"而失败。
    fn system_stack() -> Option<TextStack<AbGlyphSource>> {
        match AbGlyphSource::from_bytes(0, std::fs::read(FONT_CANDIDATES[0]).ok()?.as_slice())
        {
            Ok(s) => Some(TextStack::new(s)),
            Err(_) => None,
        }
    }

    #[test]
    fn a_real_font_measures_and_renders() {
        let Some(mut s) = system_stack() else {
            eprintln!("跳过：找不到系统字体");
            return;
        };
        let style = TextStyle {
            size: 16.0,
            ..Default::default()
        };
        let shaped = s.shape("Hello", &style, None);
        assert!(
            shaped.size.width > 0.0,
            "真实字体必须测出非零宽度"
        );
        assert_eq!(shaped.lines.len(), 1);

        let (_atlas, quads) = s.glyph_quads(&shaped, Point::new(0.0, 0.0), &style).unwrap();
        assert!(!quads.is_empty(), "应产出字形四边形");
        for q in &quads {
            assert!(q.uv.x >= 0.0 && q.uv.y >= 0.0 && q.uv.x + q.uv.width <= 1.0);
            assert!(q.dst.width > 0.0 && q.dst.height > 0.0);
        }
    }

    #[test]
    fn space_advances_but_renders_nothing() {
        let Some(mut s) = system_stack() else {
            eprintln!("跳过：找不到系统字体");
            return;
        };
        let style = TextStyle::default();
        // 空格的步进应大于 0（它确实占空间）。
        assert!(
            s.advance(' ', &style) > 0.0,
            "空格必须推进光标，否则单词会粘在一起"
        );
        let shaped = s.shape("a a", &style, None);
        let (_atlas, quads) = s.glyph_quads(&shaped, Point::new(0.0, 0.0), &style).unwrap();
        // 3 个字符，但空格没有位图 → 只有 2 个可绘制四边形。
        assert_eq!(
            quads.len(),
            2,
            "空格有步进无位图，不应产出四边形"
        );
    }

    #[test]
    fn a_missing_glyph_becomes_the_replacement_character() {
        let Some(mut s) = system_stack() else {
            eprintln!("跳过：找不到系统字体");
            return;
        };
        use zephyra_core::text::GlyphSource as _;
        let style = TextStyle::default();
        // 私用区字符几乎肯定不在任何系统字体里。
        let missing = '\u{E000}';
        let id = s.source_mut().glyph_id(missing);
        let normal = s.source_mut().glyph_id('A');
        assert_ne!(
            id, normal,
            "缺失字符应映射到替换字符而不是复用正常字形"
        );
        // 关键是它**有宽度**（会被画成豆腐框），而不是静默消失。
        assert!(
            s.advance(missing, &style) > 0.0,
            "缺失字符必须有步进，否则布局会把它当不存在"
        );
    }

    #[test]
    fn the_same_text_packs_each_glyph_once() {
        let Some(mut s) = system_stack() else {
            eprintln!("跳过：找不到系统字体");
            return;
        };
        let style = TextStyle::default();
        let shaped = s.shape("repeat repeat", &style, None);
        let _ = s.glyph_quads(&shaped, Point::new(0.0, 0.0), &style);
        let first = s.atlas_stats().glyphs;
        let _ = s.glyph_quads(&shaped, Point::new(0.0, 0.0), &style);
        assert_eq!(
            s.atlas_stats().glyphs,
            first,
            "同一字形不得重复打包"
        );
    }

    #[test]
    fn different_font_sizes_use_different_bitmaps() {
        let Some(mut s) = system_stack() else {
            eprintln!("跳过：找不到系统字体");
            return;
        };
        let small = TextStyle {
            size: 12.0,
            ..Default::default()
        };
        let large = TextStyle {
            size: 36.0,
            ..Default::default()
        };
        let before = s.atlas_stats().glyphs;
        let a = s.shape("H", &small, None);
        let _ = s.glyph_quads(&a, Point::new(0.0, 0.0), &small);
        let b = s.shape("H", &large, None);
        let _ = s.glyph_quads(&b, Point::new(0.0, 0.0), &large);
        assert!(
            s.atlas_stats().glyphs > before + 1,
            "不同字号必须各打包一份位图"
        );
    }

    #[test]
    fn a_broken_font_is_rejected_rather_than_silently_ignored() {
        let r = AbGlyphSource::from_bytes(0, b"this is not a font");
        assert!(r.is_err(), "损坏的字体必须报错而不是产出空栈");
    }
}

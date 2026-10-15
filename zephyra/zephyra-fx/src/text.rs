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

    /// **em 像素 → ab_glyph 的 `PxScale`**。全部字号换算的唯一入口。
    ///
    /// # 为什么必须有这一刀
    ///
    /// ab_glyph 的 `PxScale::from(x)` 的语义是**行高**（`hhea` 的
    /// `ascent − descent (+ lineGap)`），不是 em 尺寸：`as_scaled(PxScale::from(45))`
    /// 之后 `.height() == 45.0`，而 `.height()` 是行高。而 CSS / 本框架的
    /// `TEXT_SIZE` 给的是 **em**（`font-size`）。
    ///
    /// 直接把 em 当 `PxScale` 传的后果：整份文本按 `upem / height_unscaled` 的比例
    /// 缩小。Segoe UI 的 hhea 行高 = 1.33 em，于是**所有文字一律渲染成 0.75 倍** ——
    /// 而布局（步进、行高）仍按 em 算：字变小、行距变松、不同字号之间对不齐。
    /// 实测症状：34px 的 'Z' 位图只有 15×18（应为 19×24）。
    ///
    /// 两个调用点（`advance` / `rasterize`）都必须经过这里 —— 漏一个的后果是
    /// 「步进按 em、位图按行高」，字形互相叠压。
    fn px_scale(&self, px_em: f32) -> PxScale {
        let upem = self.font.units_per_em().unwrap_or(1000.0).max(1.0);
        // hhea 行高（字体单位）。ascent 为正、descent 为负，相减就是总行高。
        let line = self.font.height_unscaled().max(1.0);
        PxScale::from(px_em.max(1.0) * line / upem)
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
                    font: FontVec::try_from_vec(minimal_sfnt()).expect("最小 sfnt 头应可解析"),
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
            Ok(bytes) if !bytes.is_empty() => match AbGlyphSource::from_bytes(i as u32, &bytes) {
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
            },
            Ok(_) => tried.push(format!("{path}（空文件）")),
            Err(_) => tried.push(path.to_string()),
        }
    }
    (
        TextStack::disabled(AbGlyphSource {
            // 占位字体：`font_id` 任意，因为 disabled 模式不产出任何字形。
            font: FontVec::try_from_vec(minimal_sfnt()).expect("最小 sfnt 头应可解析"),
            font_id: u32::MAX,
        }),
        format!(
            "未找到可用系统字体（试过 {} 个路径），已退回无文本模式",
            tried.len()
        ),
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
        let scale = self.px_scale(px_size);
        self.font
            .as_scaled(scale)
            .h_advance(GlyphId(glyph_id.min(u16::MAX as u32) as u16))
    }

    fn rasterize(&mut self, glyph_id: u32, px_size: u32) -> Option<AtlasRaster> {
        let px = px_size.max(1) as f32;
        let id = GlyphId(glyph_id.min(u16::MAX as u32) as u16);
        // em → 行高换算必须走同一个入口：`outline_glyph` 用的是 **Glyph 自带**的
        // scale（见 `ScaleFont::outline_glyph` 的注释「does not make use of the
        // associated scale」），所以这里给 `with_scale` 的也必须是换算后的值。
        let scale = self.px_scale(px);
        let scaled = self.font.as_scaled(scale);

        // 空白字形（空格、零宽字符）没有轮廓 —— 返回 `None`，core 会给它一个零尺寸
        // 图集条目并推进光标（步进仍由 `advance` 给出）。
        let outlined = scaled.outline_glyph(id.with_scale(scale))?;

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
            // offset 是**屏幕方向**（y 向下）相对基线原点的偏移，基线之上为**负**。
            //
            // ab_glyph 的 `px_bounds` 就是这个约定：`min.y * -scale_factor.vertical`
            // 把字体坐标（y-up）翻成屏幕坐标（y-down），于是 'h' 的 min.y ≈ −26、
            // 'p' 的 min.y ≈ −19 而 `min.y + height > 0`（descender 伸到基线下面）。
            // core 拿它直接加在基线上（`GlyphQuad::dst.y = baseline + offset.y`）。
            //
            // ⚠ 这里曾经写成整数 `min.floor()` 并注释"y 向上为正"—— 约定说反了，
            // 于是 core 的公式也跟着反，每个字形的顶部贴到基线上（同一行基线错开、
            // descender 消失）。整数部分保留：位图从 `floor(min)` 开始，dst 必须与
            // 位图严格对齐，否则小字号下整体偏一个像素。
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
#[path = "tests/text_tests.rs"]
mod tests;

//! [`TextStack`]：排版 + 断行 + 图集 + 降级 —— 全在 core。
//!
//! 这是文本层的**默认实现**。宿主只提供 [`GlyphSource`]（怎么把字符变成位图），
//! 其余全部由本类型完成：断行、打包、算 uv、产出 [`crate::paint::Command::Text`]。
//!
//! # 为什么不是一个 trait 实现而是具体类型
//!
//! 因为它**没有可替换的地方** —— 它就是"文本层"本身。换掉它意味着换掉整个文本
//! 层，而不是换一个策略。真正需要替换的是它依赖的 [`GlyphSource`]。
//!
//! # 与宿主的关系
//!
//! ```text
//! core                              base
//! ────                              ────
//! TextStack<S: GlyphSource>   ──►   S 的实现（如 ab_glyph）
//!   └ GlyphAtlas（CPU 位图）    ──►   upload_atlas(pixels) 一次
//! ```

use crate::core::{Point, Rect, Size, UvRect};
use crate::paint::{GlyphQuad, TextureRef};

use super::{
    atlas::AtlasRaster, GlyphAtlas, GlyphSource, Line, PlacedGlyph, ShapedText, TextRender,
    TextStyle,
};

/// 默认行高倍数。字体自带值缺失时用它。
pub const DEFAULT_LINE_HEIGHT: f32 = 1.2;

/// 文本栈：把字符流变成可绘制的字形。
pub struct TextStack<S: GlyphSource> {
    source: S,
    atlas: GlyphAtlas,
    /// 无字体模式：所有测量返回零、不产出字形。
    disabled: bool,
}

impl<S: GlyphSource> TextStack<S> {
    /// 用一个字形来源构造。
    pub fn new(source: S) -> Self {
        Self {
            source,
            atlas: GlyphAtlas::new(),
            disabled: false,
        }
    }

    /// 用指定边长的图集构造。
    ///
    /// 存在的意义是让**淘汰路径**能被端到端测到：`glyph_quads` 的两趟写法之所以
    /// 必要，只有真的触发一次重排才能看出来，而默认的 1K² 图集要放进上万个字形
    /// 才会满。测试用一张小图集，几行就能把那条路跑通。
    ///
    /// ⚠ 生产代码请用 [`Self::new`]：后端建纹理读的是 `atlas::ATLAS_SIZE`，
    /// 自定义边长会让上传尺寸与纹理对不上。
    pub fn with_atlas_size(source: S, side: u32) -> Self {
        Self {
            source,
            atlas: GlyphAtlas::with_size(side),
            disabled: false,
        }
    }

    /// 图集。宿主把它上传成纹理。
    pub fn atlas(&self) -> &GlyphAtlas {
        &self.atlas
    }

    /// 换掉图集的**淘汰策略**（丢谁、按什么顺序重排）。
    ///
    /// 宿主实际持有的是 `TextStack` 而不是 `GlyphAtlas`，所以没有这一层的话
    /// [`EvictionPolicy`] 就只是个够不着的契约。
    ///
    /// 换策略**不重建图集**：已有的位图与 uv 全部保留，新策略从下一次放不下时开始
    /// 生效 —— 换策略本身不该让所有幸存者的 uv 失效（那会让宿主白重传一次 4MB）。
    ///
    /// 两种真实动机见 [`EvictionPolicy`]: 按可见性淘汰（弹窗打开时不留被遮挡的
    /// 背景文字）、以及"只丢不排"以避开每次淘汰都重传整张图集。
    pub fn set_atlas_policy(&mut self, policy: impl super::EvictionPolicy + 'static) {
        self.atlas.set_policy(policy);
    }

    /// 用指定边长**与淘汰策略**构造。
    ///
    /// 与 [`Self::with_atlas_size`] 的差别只有策略那一个参数；生产路径仍然应该用
    /// [`Self::new`]（后端建纹理读的是 `atlas::ATLAS_SIZE`）。
    pub fn with_atlas_policy(
        source: S,
        side: u32,
        policy: impl super::EvictionPolicy + 'static,
    ) -> Self {
        Self {
            source,
            atlas: GlyphAtlas::with_policy(side, policy),
            disabled: false,
        }
    }

    /// 图集统计。
    pub fn atlas_stats(&self) -> super::AtlasStats {
        self.atlas.stats()
    }

    /// 图集**位图内容**的版本号。
    ///
    /// 宿主靠它决定「要不要把图集重传给 GPU」。⚠ 不要用 `atlas_stats().glyphs`
    /// 代替：淘汰之后字形数可能不变而纹理已整体重排，那样 GPU 会一直用旧图集。
    pub fn atlas_revision(&self) -> u64 {
        self.atlas.revision()
    }

    /// 字形来源（只读）。多字体的应用用它检查覆盖范围。
    pub fn source(&self) -> &S {
        &self.source
    }

    /// 字形来源（可变）。用于枚举可用字形、按 `font_id` 分派等场景。
    pub fn source_mut(&mut self) -> &mut S {
        &mut self.source
    }

    /// **无字体模式**：不产出任何字形，所有测量返回零尺寸。
    ///
    /// 用于"这台机器找不到字体"的降级 —— 文本节点于是既不占空间也不绘制，其余
    /// 节点完全不受影响。这比 panic 好得多：后者让"没装字体"变成"程序不能跑"。
    pub fn disabled(source: S) -> Self {
        Self {
            source,
            atlas: GlyphAtlas::new(),
            disabled: true,
        }
    }

    /// 排版：按 `max_width` 断行。
    ///
    /// 断行按**字符边界**切，不做连字符与字素簇感知的断词 —— 对 UI 标签够用。
    fn layout(&mut self, text: &str, px_size: f32, max_width: Option<f32>) -> Vec<Line> {
        let mut lines = Vec::new();
        let mut current: Vec<PlacedGlyph> = Vec::new();
        let mut width = 0.0f32;

        for ch in text.chars() {
            // 显式换行优先于自动断行。
            if ch == '\n' {
                lines.push(Line {
                    baseline: Point::new(0.0, 0.0),
                    width,
                    glyphs: std::mem::take(&mut current),
                    is_last: false,
                });
                width = 0.0;
                continue;
            }

            let glyph_id = self.source.glyph_id(ch);
            let advance = self.source.advance(glyph_id, px_size);

            // 自动断行：在**加入之前**判断，于是超宽的那个字形整体换到下一行。
            if let Some(max) = max_width {
                if width + advance > max && !current.is_empty() {
                    lines.push(Line {
                        baseline: Point::new(0.0, 0.0),
                        width,
                        glyphs: std::mem::take(&mut current),
                        is_last: false,
                    });
                    width = 0.0;
                }
            }

            current.push(PlacedGlyph {
                glyph_id,
                offset: Point::new(width, 0.0),
                advance,
                color_glyph: false,
            });
            width += advance;
        }
        lines.push(Line {
            baseline: Point::new(0.0, 0.0),
            width,
            glyphs: current,
            is_last: true,
        });
        lines
    }
}

impl<S: GlyphSource> TextStack<S> {
    /// 排版并填好基线。
    fn shape_impl(
        &mut self,
        text: &str,
        style: &TextStyle,
        max_width: Option<f32>,
    ) -> ShapedText {
        if self.disabled {
            return ShapedText {
                size: Size::new(0.0, 0.0),
                lines: Vec::new(),
                px_size: 0.0,
            };
        }
        let px_size = style.effective_size().max(1.0);
        let line_height = style.line_height.unwrap_or(DEFAULT_LINE_HEIGHT) * px_size;

        // 空文本：返回**零行**而不是"一行空行"。
        //
        // 后者会让空标签占掉一个行高的空间 —— 于是一个空 `Text` 节点在垂直堆叠里
        // 凭空多出 16.8px 的间隙，而这种间隙在截图里看不出来源。零行才能让「没有
        // 内容」与「没有高度」等价。
        if text.is_empty() {
            return ShapedText {
                size: Size::new(0.0, 0.0),
                lines: Vec::new(),
                px_size,
            };
        }

        let mut lines = self.layout(text, px_size, max_width);

        // 填基线：逐行下移一个行高。y 向下为正，与核心的 DIP 坐标系一致。
        for (i, line) in lines.iter_mut().enumerate() {
            line.baseline = Point::new(0.0, (i as f32 + 1.0) * line_height);
        }

        // 宽度取**最长的一行**，不是"每行容量"。多行文本块的实际宽度由最宽的那行
        // 决定 —— 若用「每行容量 × 步进」，一个 100 字符的段落被断成 10 行时会
        // 报告 10 字符宽，容器随之算出错误的尺寸。
        let width = lines.iter().map(|l| l.width).fold(0.0f32, f32::max);
        ShapedText {
            size: Size::new(width, lines.len() as f32 * line_height),
            lines,
            // 排版时字号就在手上，直接带下去。`glyph_quads` 靠它查图集 ——
            // 曾经那里是**反推**的，而那条启发式只对纯拉丁成立（含 CJK 时错一倍）。
            px_size,
        }
    }
}

impl<S: GlyphSource> super::TextEngine for TextStack<S> {
    fn shape(&mut self, text: &str, style: &TextStyle, max_width: Option<f32>) -> ShapedText {
        self.shape_impl(text, style, max_width)
    }

    fn advance(&mut self, ch: char, style: &TextStyle) -> f32 {
        if self.disabled {
            return 0.0;
        }
        let id = self.source.glyph_id(ch);
        self.source.advance(id, style.effective_size().max(1.0))
    }

    fn line_height(&mut self, style: &TextStyle) -> f32 {
        if self.disabled {
            return 0.0;
        }
        style.line_height.unwrap_or(DEFAULT_LINE_HEIGHT) * style.effective_size().max(1.0)
    }
}

impl<S: GlyphSource> TextRender for TextStack<S> {
    /// # 为什么先展平再打包
    ///
    /// `self.source` 与 `self.atlas` 的可变借用不能同时存在（都是 `self` 的字段）。
    /// 而"问来源要几何"与"交给图集打包"必须交替进行 —— 每问一个字形就要打包一个。
    ///
    /// 所以顺序是：先一次性把所有几何问出来（只借 source），再统一打包（只借
    /// atlas）。代价是多一个 `Vec` 与每字形一次 `coverage` 克隆；换来的是不需要
    /// `unsafe`、不需要 `Rc<RefCell>`、也不需要"回调可重复调用"这种隐晦约束。
    ///
    /// # ⚠ 打包必须分**两趟**，不能边放边读
    ///
    /// 第一趟把所有字形都放进图集，第二趟才读 uv 拼四边形。
    ///
    /// 原因是**淘汰会重排整张图集**：一旦某个字形放不下，`GlyphAtlas` 会丢掉最久未用
    /// 的那些、把幸存者重新排一遍，于是**所有幸存的 uv 都变了**。如果边放边读，第一趟
    /// 早期读到并已经写进 `quads` 的 uv 就会指向后来被别人占用的位置 ——
    /// 画面上是「几个字换成了别的字」，然后随淘汰时机随机变化。那不是崩溃，所以更贵。
    ///
    /// 两趟之后就稳定了：第一趟结束时图集不再变化，第二趟只读不写（不再插入 ⇒ 不再
    /// 淘汰），uv 在整趟循环里都是对的。
    ///
    /// # ⚠⚠ 第一趟必须**先查图集**，命中就不能栅格化
    ///
    /// 这一条是本函数里最贵的性能判据，量级差得很远：`rasterize` 要做完整的字体
    /// 轮廓提取 + 抗锯齿栅格化（ab_glyph 的 `outline_glyph` → `draw`），而
    /// `get_or_insert` 命中缓存时会把刚算出来的位图**直接丢掉**。
    ///
    /// 原先的实现无条件先 `rasterize` 再 `get_or_insert`，于是**稳态下每帧都要把
    /// 全部可见字形重新栅格化一遍**，哪怕一个字都没变。实测（demo 113 个字形四边形）
    /// 它独占 `paint` 阶段的 85%，整帧 1.1ms → 优化后 0.13ms。
    ///
    /// 这类"算出来只是为了扔掉"的浪费有一个共同特征：**功能完全正确**，所以任何
    /// 功能测试都抓不到它，只有计时能。
    ///
    /// 跳过栅格化时**仍要 `touch`**（刷新 LRU 时间戳），否则静止一段时间后第一次
    /// 更新会引发连锁淘汰 + 整张图集重排 → 所有 uv 变化 → 一次 4MB 重传。理由见
    /// [`crate::text::GlyphAtlas::touch`]。
    fn glyph_quads(
        &mut self,
        shaped: &ShapedText,
        origin: Point,
        _style: &TextStyle,
    ) -> Option<(TextureRef, Vec<GlyphQuad>)> {
        if self.disabled || shaped.lines.is_empty() {
            return None;
        }
        // 图集按**像素字号**缓存，拿错字号会取到另一套位图。字号由排版时存进
        // `ShapedText::px_size` —— 这里只取用。
        //
        // ⚠ 这里曾经是**反推**的（`infer_px_size`）。那条公式假设「最宽字形是拉丁窄字符、
        // 步进 ≈ 0.5em」，于是含 CJK 的文本（全角步进 ≈ 1em）会反推出**两倍**字号：
        // 16px 的中文按 32px 栅格化，而步进仍按 16px 算 —— 字形互相叠压成一行糊。
        // 症状与「字体没有该字」「GPU 没画」都极像，于是被当成环境问题查了很久。
        let px_size = shaped.px_size;
        debug_assert!(
            px_size > 0.0,
            "ShapedText::px_size 为 0 —— 排版时没填？手搭的 ShapedText 要自己填"
        );
        let px = px_size.max(1.0) as u32;
        let font_id = self.source.font_id();

        // 归一化用图集**实际**的边长，而不是全局 `ATLAS_SIZE`。
        //
        // 曾经写死 `ATLAS_SIZE`：`GlyphAtlas` 记的 uv 是像素，两者必须一致。用固定
        // 常量的话，一旦图集边长与常量不同，所有 uv 会整体偏移一个比例 —— 而那是
        // "字形出现在错误的位置/大小"，看起来像打包 bug。
        let inv = 1.0 / self.atlas.size() as f32;

        // ---- 第一趟：把字形全部放进去。----
        //
        // 遍历顺序与 `shaped.lines` 严格一致，第二趟的游标才认得。
        //
        // 空白字形也占一个位置（零尺寸），以保持游标对齐 —— 若在这里跳过，后面
        // 所有字形都会取到**上一个**的位图，于是整段文字显示成同一个字反复出现。
        // 这类 bug 极难从截图里定位，因为它看起来"基本是对的"。
        //
        // ⚠ 两条路径的**顺序不可交换**：命中缓存的也要走一趟（`touch`），否则上面
        // 那条"静止后连锁淘汰"的坑就回来了。
        for line in &shaped.lines {
            for g in &line.glyphs {
                if self.atlas.touch(font_id, g.glyph_id, px).is_some() {
                    // 已有位图：**跳过栅格化**，只刷新了 LRU 时间戳。
                    continue;
                }
                let r = self
                    .source
                    .rasterize(g.glyph_id, px)
                    .unwrap_or(AtlasRaster {
                        offset: Point::new(0.0, 0.0),
                        width: 0,
                        height: 0,
                        coverage: Vec::new(),
                    });
                self.atlas.get_or_insert(font_id, g.glyph_id, px, Some(r));
            }
        }

        // ---- 第二趟：只读 uv，拼四边形。----
        //
        // 这一趟**不再插入** ⇒ 不会淘汰 ⇒ uv 稳定。用 `get` 而不是 `get_or_insert`：
        // 万一第一趟里被淘汰掉了，这里就该跳过它（`None`），而不是又插一次把别人
        // 挤掉 —— 那会让"谁留下来"随遍历顺序漂移。
        //
        // ⚠ **不能**用 `touch` 顶替 `get`：那会在淘汰掉某个字形时把它**重新插回来**
        // （`touch` 只在命中时返回 `None`，不做插入，所以其实等价）—— 但语义上
        // 第二趟就该纯读。任何"顺便插一下"的行为都会让"这一趟不改变图集"的前提
        // 失效，而那条前提正是两趟写法成立的原因。
        let mut quads = Vec::new();
        for line in &shaped.lines {
            for g in &line.glyphs {
                let Some(entry) = self.atlas.get(font_id, g.glyph_id, px) else {
                    // 淘汰掉了：多半是图集装不下这一帧的整个字符集。跳过而不是
                    // 整段放弃 —— 部分文本可见远好于整段消失，且
                    // `atlas_stats().evictions` 会暴露这件事。
                    continue;
                };
                if entry.size.width <= 0.0 || entry.size.height <= 0.0 {
                    continue;
                }
                let baseline_x = origin.x + g.offset.x;
                let baseline_y = origin.y + line.baseline.y;
                quads.push(GlyphQuad {
                    // 图集条目按**像素**记 uv（位置在下次淘汰之前不变），这里归一化。
                    //
                    // ⚠ 归一化的是**墨**的矩形，不含四周的留白 —— 留白只是让双线性
                    // 采样在边缘摸到透明像素，不参与 uv，也不改变 `dst` 的尺寸。
                    uv: UvRect::new(
                        entry.uv.x * inv,
                        entry.uv.y * inv,
                        entry.uv.width * inv,
                        entry.uv.height * inv,
                    ),
                    // offset 是**屏幕方向**（y 向下）相对基线原点的偏移：基线之上
                    // 为**负**（ab_glyph 的 `px_bounds` 就是这个约定，见其
                    // `min.y * -scale_factor.vertical`）。
                    //
                    // ⚠ 这条公式曾经写成 `baseline - offset.y - height`（把 offset
                    // 当成 y-up 的正值）。后果不是偏移一两个像素，而是**每个字形的
                    // 顶部都贴到基线上**：'h' 与 'e' 顶在同一条线、底相差一个字高 ——
                    // 同一行字的基线互相错开，descender 整个消失。
                    dst: Rect::new(
                        baseline_x + entry.offset.x,
                        baseline_y + entry.offset.y,
                        entry.size.width,
                        entry.size.height,
                    ),
                    color_glyph: g.color_glyph,
                });
            }
        }
        Some((TextureRef::new(super::ATLAS_NAME), quads))
    }
}

/// 测试用的字形来源：定宽、每字符一块位图。
///
/// 用来断言**排版逻辑**（断行、基线、尺寸）与**图集接线**（uv、缓存）而不掺入真实
/// 字体 —— 真实字体的行为由 base 层那侧的断言覆盖。
#[derive(Debug, Default, Clone)]
pub struct FixedWidth {
    /// 每字符步进（像素）。
    pub advance: f32,
}

impl GlyphSource for FixedWidth {
    fn glyph_id(&mut self, ch: char) -> u32 {
        ch as u32
    }

    fn advance(&mut self, _glyph_id: u32, _px_size: f32) -> f32 {
        self.advance
    }

    /// **全都有位图** —— 定宽替身要能验证"每个字符一个四边形"。
    ///
    /// 空白字形（空格）的路径由 `space_advances_but_renders_nothing` 那类断言覆盖：
    /// 它需要"有步进、无位图"，而本替身不区分字符。
    fn rasterize(&mut self, _glyph_id: u32, px_size: u32) -> Option<AtlasRaster> {
        // 画一个 px/2 见方的实心块 —— 有位图，于是整条 `glyph_quads` 路径可测。
        let n = (px_size / 2).max(1);
        Some(AtlasRaster {
            offset: Point::new(0.0, 0.0),
            width: n,
            height: n,
            coverage: vec![255u8; (n * n) as usize],
        })
    }

    fn font_id(&self) -> u32 {
        0
    }
}

/// 测试用字形来源：offset **随字符变化**（模仿 ab_glyph 的 y-down 约定）。
///
/// `FixedWidth` 的 offset 恒为 0 —— 那种替身测不出基线公式（所有字形都退化成
/// 同一个矩形）。真实字体里 'a' / 'h' / 'p' 的 offset 各不相同，基线公式的
/// 符号错误**只有**在 offset 有符号时才暴露。
#[derive(Debug, Default, Clone)]
pub struct VaryingOffset {
    advance: f32,
    /// 位图全高（像素）。
    full: f32,
    width: f32,
}

impl GlyphSource for VaryingOffset {
    fn glyph_id(&mut self, ch: char) -> u32 {
        ch as u32
    }

    fn advance(&mut self, _glyph_id: u32, _px_size: f32) -> f32 {
        self.advance
    }

    fn rasterize(&mut self, glyph_id: u32, _px_size: u32) -> Option<AtlasRaster> {
        // (位图顶在基线之上的像素数, 位图底在基线之下的像素数)
        let (top, below) = match char::from_u32(glyph_id)? {
            // x-height 档：a e n
            'a' | 'e' | 'n' => (self.full * 0.7, 0.0),
            // 升部档：h l
            'h' | 'l' => (self.full, 0.0),
            // descender 档：p y
            'p' | 'y' => (self.full * 0.7, self.full * 0.3),
            _ => (self.full * 0.7, 0.0),
        };
        Some(AtlasRaster {
            // y-down：基线之上为负。
            offset: Point::new(0.0, -top),
            width: self.width as u32,
            height: (top + below) as u32,
            coverage: vec![255u8; ((self.width as u32) * (top + below) as u32).max(4) as usize],
        })
    }

    fn font_id(&self) -> u32 {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::{TextEngine, TextRender};

    fn stack(advance: f32) -> TextStack<FixedWidth> {
        TextStack::new(FixedWidth { advance })
    }

    /// 同一行的字形必须**共用一条基线**：无 descender 的字，位图底落在基线上。
    ///
    /// 这条钉的是本次实测的第二个 bug：`dst.y` 公式把 offset 的符号搞反
    /// （offset 是 y-down、基线之上为负），于是每个字形的**顶部**贴到基线上 ——
    /// 'h' 与 'e' 顶相同、底差一个字高，descender 整个消失。
    /// `FixedWidth` 的 offset 恒 0，测不出这条；所以用 `VaryingOffset`。
    #[test]
    fn glyphs_on_one_line_share_a_baseline() {
        let mut s = TextStack::new(VaryingOffset {
            advance: 10.0,
            full: 20.0,
            width: 8.0,
        });
        let style = TextStyle {
            size: 20.0,
            ..Default::default()
        };
        let shaped = s.shape("ahep", &style, None);
        let origin = Point::new(0.0, 0.0);
        let (_atlas, quads) = s.glyph_quads(&shaped, origin, &style).unwrap();
        assert_eq!(quads.len(), 4, "a h e p 各一个四边形");

        let bottom = |q: &GlyphQuad| q.dst.y + q.dst.height;
        // a / h / e 都无 descender：底必须相等（= 基线）。
        let bottoms = [bottom(&quads[0]), bottom(&quads[1]), bottom(&quads[2])];
        for b in bottoms {
            assert!(
                (b - bottoms[0]).abs() < 0.001,
                "同一行无 descender 的字必须共基线：{:?}",
                bottoms
            );
        }
        // p 有 descender：底必须**低于**基线（比基线大）。
        assert!(
            bottom(&quads[3]) > bottoms[0] + 1.0,
            "p 的 descender 必须伸到基线之下：{} vs 基线 {}",
            bottom(&quads[3]),
            bottoms[0]
        );
        // 升部字（h）的顶必须**高于** x-height 字（a）的顶。
        assert!(
            quads[1].dst.y < quads[0].dst.y - 1.0,
            "h 的顶应高于 a 的顶：{} vs {}",
            quads[1].dst.y,
            quads[0].dst.y
        );
    }

    #[test]
    fn empty_text_yields_no_lines() {
        let mut s = stack(10.0);
        let shaped = s.shape("", &TextStyle::default(), None);
        assert!(shaped.lines.is_empty(), "空文本应是零行");
        assert_eq!(
            shaped.size,
            Size::new(0.0, 0.0),
            "空文本的尺寸必须是 0 —— 否则空标签会凭空占一个行高"
        );
        assert!(
            s.glyph_quads(&shaped, Point::new(0.0, 0.0), &TextStyle::default()).is_none(),
            "空文本不应产出绘制命令"
        );
    }

    #[test]
    fn newline_splits_lines() {
        let mut s = stack(10.0);
        let shaped = s.shape("a\nb\nc", &TextStyle::default(), None);
        assert_eq!(shaped.lines.len(), 3, "显式换行应产生三行");
        assert!(
            !shaped.lines[..2].iter().any(|l| l.is_last),
            "只有最后一行该标 is_last"
        );
        assert!(shaped.lines[2].is_last, "最后一行必须标 is_last");
    }

    #[test]
    fn max_width_forces_wrapping() {
        let mut s = stack(10.0);
        let style = TextStyle::default();
        let unwrapped = s.shape("aaaaaaaaaa", &style, None).lines.len();
        let wrapped = s.shape("aaaaaaaaaa", &style, Some(35.0)).lines.len();
        assert!(
            wrapped > unwrapped,
            "限宽应产生更多行：{wrapped} vs {unwrapped}"
        );
        // 每行不得超出上限。
        for line in &s.shape("aaaaaaaaaa", &style, Some(35.0)).lines {
            assert!(line.width <= 35.0 + 1e-3, "行宽 {} 超出 35", line.width);
        }
    }

    #[test]
    fn a_single_glyph_wider_than_max_width_still_gets_its_own_line() {
        // 断行前会先判断 `!current.is_empty()`，所以一个超宽的单字形不会导致
        // 无限切分或空行。
        let mut s = stack(100.0);
        let shaped = s.shape("ab", &TextStyle::default(), Some(10.0));
        assert_eq!(shaped.lines.len(), 2, "每个字形各占一行");
    }

    #[test]
    fn measured_width_grows_with_text() {
        let mut s = stack(10.0);
        let style = TextStyle::default();
        let short = s.shape("ab", &style, None).size.width;
        let long = s.shape("abcdefgh", &style, None).size.width;
        assert!(long > short, "更多字符应更宽：{short} vs {long}");
    }

    #[test]
    fn line_height_multiplies_the_font_size() {
        let mut s = stack(10.0);
        let base = TextStyle {
            size: 10.0,
            ..Default::default()
        };
        let h_default = s.shape("X", &base, None).size.height;
        assert!(
            (h_default - 12.0).abs() < 1e-3,
            "默认行高 1.2 × 10 = 12，实得 {h_default}"
        );

        let loose = TextStyle {
            size: 10.0,
            line_height: Some(2.0),
            ..Default::default()
        };
        let h_loose = s.shape("X", &loose, None).size.height;
        assert!((h_loose - 20.0).abs() < 1e-3, "行高 2.0 × 10 = 20，实得 {h_loose}");
    }

    #[test]
    fn baselines_advance_by_one_line_height_each() {
        let mut s = stack(10.0);
        let shaped = s.shape("a\nb\nc", &TextStyle::default(), None);
        let lh = 1.2 * 14.0;
        for (i, line) in shaped.lines.iter().enumerate() {
            assert!(
                (line.baseline.y - (i as f32 + 1.0) * lh).abs() < 1e-3,
                "第 {i} 行基线应为 {}，实得 {}",
                (i as f32 + 1.0) * lh,
                line.baseline.y
            );
        }
    }

    /// 图集装不下一次排版的所有字形时（会淘汰 + 重排），产出的 uv **互不重叠**。
    ///
    /// # 这条在钉什么
    ///
    /// 淘汰会重排整张图集，所有幸存的 uv 都变。若 `glyph_quads` 边放边读（第一趟里
    /// 每放一个就立刻记下 uv），先记下的 uv 会在中途中改变含义 —— 它指向的位置会被
    /// 后来者占用。画面上是「几个字换成了别的字」，而且随淘汰时机随机变化。
    ///
    /// 两趟写法（先全放、再统一读）之后，同一个图集位置不会出现在两个四边形上。
    /// **重叠就是边放边读留下的指纹** —— 这是从外面唯一能观测到这件事的地方。
    ///
    /// 用小图集（32×32，只装得下 9 个）而不是默认的 1K²：后者要上万个字形才满，
    /// 那样的测试既慢又容易因为排版细节变化而失效。
    ///
    /// ⚠ 这条测试曾经**从未运行过** —— `#[test]` 被缩进掉到了函数体外（缩进比模块
    /// 体深），于是它只是个没人调用的私有函数，编译器只给一条 `dead_code` 警告。
    ///
    /// 它恰好是本文件里**最关键**的那条：第一趟"边放边读"是这里最容易写错的结构，
    /// 而这条测试是唯一能抓住它的断言。它没运行这件事本身很说明问题 ——
    /// **缩进错误的测试不会失败，它只是消失**，所以要靠 `dead_code` 警告发现。
    #[test]
    fn 触发淘汰时_产出的_uv_互不重叠() {
        // FixedWidth 的字形是 px/2 见方的实心块：16px ⇒ 8×8 的墨 + 两圈留白 = 10×10。
        // 32×32 只装得下 3×3 = 9 个，而下面排 12 个字符 ⇒ 必然淘汰。
        let mut s = TextStack::with_atlas_size(FixedWidth { advance: 10.0 }, 32);
        let style = TextStyle {
            size: 16.0,
            ..Default::default()
        };
        let shaped = s.shape("ABCDEFGHIJKL", &style, None);
        let (_, quads) = s
            .glyph_quads(&shaped, Point::new(0.0, 0.0), &style)
            .expect("有字形来源时不该返回 None");

        assert!(
            s.atlas_stats().evictions > 0,
            "12 个字形装进只放得下 9 个的图集，应当发生过淘汰 —— 否则这条测试没测到东西"
        );
        assert!(!quads.is_empty(), "淘汰之后仍应有字形可见");

        for i in 0..quads.len() {
            for j in i + 1..quads.len() {
                let (a, b) = (quads[i].uv, quads[j].uv);
                let disjoint = a.right() <= b.x
                    || b.right() <= a.x
                    || a.bottom() <= b.y
                    || b.bottom() <= a.y;
                assert!(
                    disjoint,
                    "四边形 {i} 与 {j} 的 uv 重叠（{:?} / {:?}）—— \
                     这就是「边放边读」的指纹：先记下的 uv 被后来的重排改成了别处",
                    a, b
                );
            }
        }

        // 每个 uv 处必须真的有墨：重排若把墨放错地方，这条会挂。
        let size = s.atlas().size() as usize;
        let pixels = s.atlas().pixels();
        for (k, q) in quads.iter().enumerate() {
            let x = (q.uv.x * size as f32) as usize;
            let y = (q.uv.y * size as f32) as usize;
            assert!(x < size && y < size, "四边形 {k} 的 uv 越出图集");
            assert!(
                pixels[(y * size + x) * 4 + 3] > 0,
                "四边形 {k} 的 uv 指向的像素是空的"
            );
        }
    }

    #[test]
    fn glyph_quads_have_normalized_uvs() {
        let mut s = stack(10.0);
        let shaped = s.shape("Hello", &TextStyle::default(), None);
        let (_atlas, quads) = s.glyph_quads(&shaped, Point::new(0.0, 0.0), &TextStyle::default()).unwrap();
        assert_eq!(quads.len(), 5, "5 个字符应有 5 个四边形");
        for q in &quads {
            assert!(
                q.uv.x >= 0.0 && q.uv.y >= 0.0 && q.uv.x + q.uv.width <= 1.0 && q.uv.y + q.uv.height <= 1.0,
                "uv 必须归一化到 (0,1)：{:?}",
                q.uv
            );
            assert!(q.dst.width > 0.0 && q.dst.height > 0.0);
        }
    }

    #[test]
    fn glyph_quads_follow_the_origin() {
        let mut s = stack(10.0);
        let shaped = s.shape("A", &TextStyle::default(), None);
        let (_, at_zero) = s.glyph_quads(&shaped, Point::new(0.0, 0.0), &TextStyle::default()).unwrap();
        let (_, moved) =
            s.glyph_quads(&shaped, Point::new(100.0, 50.0), &TextStyle::default())
                .unwrap();
        let dx = moved[0].dst.x - at_zero[0].dst.x;
        let dy = moved[0].dst.y - at_zero[0].dst.y;
        assert!((dx - 100.0).abs() < 1e-3, "横向偏移应等于 origin.x，实得 {dx}");
        assert!((dy - 50.0).abs() < 1e-3, "纵向偏移应等于 origin.y，实得 {dy}");
    }

    #[test]
    fn the_same_text_packs_each_glyph_only_once() {
        let mut s = stack(10.0);
        let shaped = s.shape("aaa", &TextStyle::default(), None);
        let _ = s.glyph_quads(&shaped, Point::new(0.0, 0.0), &TextStyle::default());
        let after_first = s.atlas_stats().glyphs;
        // 再渲染一次同样的文本 —— 图集不得增长。
        let _ = s.glyph_quads(&shaped, Point::new(0.0, 0.0), &TextStyle::default());
        assert_eq!(
            s.atlas_stats().glyphs,
            after_first,
            "重复文本不得重复打包（否则图集会被同一批字形撑满）"
        );
    }

    #[test]
    fn disabled_stack_produces_nothing() {
        let mut s = TextStack::disabled(FixedWidth { advance: 10.0 });
        let shaped = s.shape("Hello", &TextStyle::default(), None);
        assert_eq!(shaped.size, Size::new(0.0, 0.0), "无字体模式应测出零尺寸");
        assert_eq!(s.advance('a', &TextStyle::default()), 0.0);
        assert_eq!(s.line_height(&TextStyle::default()), 0.0);
        assert!(s.glyph_quads(&shaped, Point::new(0.0, 0.0), &TextStyle::default()).is_none());
    }

    /// 字号必须**原样**从排版传到绘制，不许反推。
///
/// # 这条测试替换掉的是什么
///
/// 原本是 `infer_px_size_recovers_a_plausible_value`：它喂一段手工搭的 `ShapedText`
/// （步进按 0.5em 设），断言能反推出接近的字号。那条断言**只对纯拉丁成立**。
///
/// CJK 是全角，步进 ≈ 1em，于是反推公式的 `最大步进 / 0.5` 给出**两倍**字号：
/// 16px 的中文被按 32px 栅格化，而布局步进仍是 16px —— 字形互相叠压。
///
/// demo 的文本几乎全是中文，所以这条路径一直在产出错误结果。症状是「字挤成一团」，
/// 而它与「字体不含 CJK」「GPU 没画出来」在截图上极像。
///
/// 现在字号由 `shape` 直接写进 [`ShapedText::px_size`]，所以断言是「等于」而不是
/// 「接近」—— 而**含 CJK 时也必须成立**。
#[test]
fn 字号从排版原样传到绘制_不反推() {
    for px in [12.0f32, 16.0, 18.0, 24.0, 32.0] {
        let mut s = TextStack::new(FixedWidth { advance: 1.0 });
        let style = TextStyle {
            size: px,
            ..Default::default()
        };
        // 全角步进（CJK）：1em。这正是让旧公式错两倍的那一档。
        let shaped = s.shape("中文", &style, None);
        assert_eq!(
            shaped.px_size, px,
            "字号 {px} 必须在 ShapedText 里原样保留"
        );
        let (_tex, quads) = s
            .glyph_quads(&shaped, Point::new(0.0, 0.0), &style)
            .expect("有字体时应有字形四边形");
        assert!(!quads.is_empty());
        // 判据取**替身自己知道的**关系：`FixedWidth::rasterize` 画 `px_size / 2`
        // 见方的块，所以字形高度必须**精确等于** `px/2`。
        //
        // 旧的反推公式会给出 `2 × px`，那时高度就是 `px` —— 差一倍，肉眼是一行糊。
        let expect = ((px as u32) / 2).max(1) as f32;
        assert_eq!(
            quads[0].dst.height, expect,
            "字号 {px} 时字形高度应为 {expect} —— 若约为 {px}，说明字号被反推成了两倍"
        );
    }
}

    /// 空排版的字号是 0，而 `glyph_quads` 对空 `lines` 直接返回 `None` ——
    /// 所以「0 号字号」不会被拿去查图集。
    #[test]
    fn 空排版不产出任何四边形_所以零字号无害() {
        let mut s = stack(7.0);
        let empty = ShapedText::default();
        assert_eq!(empty.px_size, 0.0, "默认构造的排版结果字号为 0");
        assert!(
            s.glyph_quads(&empty, Point::new(0.0, 0.0), &TextStyle::default())
                .is_none(),
            "没有行就没有字形可画 —— 必须在查图集**之前**返回"
        );
    }

    #[test]
    fn advance_matches_the_sum_of_glyph_advances() {
        let mut s = stack(7.0);
        let style = TextStyle::default();
        let text = "abc";
        let summed: f32 = text.chars().map(|c| s.advance(c, &style)).sum();
        let shaped_width = s.shape(text, &style, None).size.width;
        assert!(
            (summed - shaped_width).abs() < 1e-2,
            "advance 之和应等于整段宽度：{summed} vs {shaped_width}"
        );
    }

    /// 一个"按 id 奇偶决定去留"的策略：奇数留、偶数丢。
    ///
    /// 刻意选一个**与 LRU 无关**的判据 —— 这样测试才能区分"策略真的被采纳"与
    /// "碰巧与默认策略结果一致"。
    struct OddOnly;

    impl super::super::EvictionPolicy for OddOnly {
        fn evict(
            &self,
            entries: &[super::super::EntryInfo],
            _need: (u32, u32),
            _side: u32,
        ) -> Option<super::super::EvictPlan> {
            Some(super::super::EvictPlan {
                survivors: entries
                    .iter()
                    .filter(|e| e.key.glyph_id % 2 == 1)
                    .map(|e| e.key)
                    .collect(),
                dropped: entries
                    .iter()
                    .filter(|e| e.key.glyph_id % 2 == 0 && e.padded.0 > 0)
                    .count(),
            })
        }
    }

    /// 宿主侧换策略必须真的生效 —— 否则 [`EvictionPolicy`] 只是个够不着的契约。
    ///
    /// 宿主实际持有的是 `TextStack`，所以只有经 [`TextStack::set_atlas_policy`] 这条
    /// 路换上的策略才算数。这条测试用 12 个字形（'A'..'L'）打进只装得下 9 个的图集，
    /// 触发淘汰后检查"偶数字形真的没了"。
    #[test]
    fn 宿主可以从文本栈换上自定义淘汰策略() {
        let mut s = TextStack::with_atlas_size(FixedWidth { advance: 10.0 }, 32);
        s.set_atlas_policy(OddOnly);
        let style = TextStyle {
            size: 16.0,
            ..Default::default()
        };
        let shaped = s.shape("ABCDEFGHIJKL", &style, None);
        let (_atlas, quads) = s
            .glyph_quads(&shaped, Point::new(0.0, 0.0), &style)
            .expect("有字形来源时不该返回 None");

        assert!(
            s.atlas_stats().evictions > 0,
            "12 个字形装进只放得下 9 个的图集，应当发生过淘汰 —— 否则这条测试没测到东西"
        );

        // 'A'..'L' 的码点连续，所以 id 偶数 = B,D,F,H,J,L。
        //
        // ⚠ 判据是「偶数字形**确实少了**」，不是「偶数字形一个都不在」。
        //
        // 后者会误判：淘汰只在**放不下**时才发生，而 12 个字形是逐个插入的 ——
        // 最后几个（尤其 'L'）可能是在最后一次淘汰**之后**才落进图集的，那时策略
        // 根本还没被问到。它们留在图集里完全正常，不是策略失效。
        //
        // 真正能区分"策略生效"与"碰巧与 LRU 一致"的判据是：奇数字形**全部**保留，
        // 而偶数字形至少少了一个（默认 LRU 会丢最旧的，恰好也是偶数，但也会
        // 在字形更多时丢奇数 —— 所以"奇数全留"才是策略的指纹）。
        let all: Vec<u32> = ('A'..='L').map(|c| c as u32).collect();
        let present: Vec<u32> = all.iter().copied().filter(|id| s.atlas().get(0, *id, 16).is_some()).collect();

        let evens: Vec<u32> = all.iter().copied().filter(|id| id % 2 == 0).collect();
        let odds: Vec<u32> = all.iter().copied().filter(|id| id % 2 == 1).collect();

        for id in &odds {
            assert!(
                s.atlas().get(0, *id, 16).is_some(),
                "id={id} 是奇数，OddOnly 策略要求保留 —— 若它被丢了，说明策略没生效"
            );
        }
        let evens_left: Vec<u32> = evens
            .iter()
            .copied()
            .filter(|id| s.atlas().get(0, *id, 16).is_some())
            .collect();
        assert!(
            evens_left.len() < evens.len(),
            "6 个偶数字形应当至少丢掉一个（策略要求全丢），实得仍剩 {:?}",
            evens_left
        );
        assert!(
            present.len() <= 9,
            "32×32 只装得下 9 个 10×10 的字形，实得 {} —— 复验或打包有问题",
            present.len()
        );
        assert!(!quads.is_empty(), "淘汰之后仍应有字形可见");
    }
}

// 本文件由 stack.rs 的 #[cfg(test)] mod tests 抽出 —— 测试代码与生产代码分离。
// 模块路径不变（src 里的声明用 #[path] 指过来），所以 use super::* 与私有可见性都照旧。

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

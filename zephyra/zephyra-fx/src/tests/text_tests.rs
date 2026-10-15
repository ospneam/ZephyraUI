// 本文件由 text.rs 的 #[cfg(test)] mod tests 抽出 —— 测试代码与生产代码分离。
// 模块路径不变（src 里的声明用 #[path] 指过来），所以 use super::* 与私有可见性都照旧。

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

    /// `px_scale` 是把 em 翻成 ab_glyph 行高的唯一入口。契约：
    /// 换算后的 scale 装回字体，`.height()`（行高像素）必须等于
    /// `em × hhea行高 / upem`。
    ///
    /// 这条钉的是本次实测的 bug：ab_glyph 的 `PxScale::from(x)` 是**行高**语义，
    /// 直接拿 em 传进去（`.height() == em`），Segoe UI（hhea = 1.33 em）的全部
    /// 文字一律渲成 0.75 倍。此处对 Segoe 期望 15.96 而实测 12 —— 两种写法
    /// 只差这一个比例，症状是整库文字变小且行距松弛。
    #[test]
    fn px_scale_maps_em_to_font_line_height() {
        use ab_glyph::{Font, ScaleFont};
        let Ok(bytes) = std::fs::read(FONT_CANDIDATES[0]) else {
            eprintln!("跳过：找不到系统字体");
            return;
        };
        let src = match AbGlyphSource::from_bytes(0, &bytes) {
            Ok(s) => s,
            Err(_) => return,
        };
        let upem = src.font.units_per_em().unwrap_or(1000.0).max(1.0);
        let ratio = src.font.height_unscaled() / upem;
        for em in [12.0f32, 14.0, 16.0, 34.0, 48.0] {
            let scaled = src.font.as_scaled(src.px_scale(em));
            let expect = em * ratio;
            assert!(
                (scaled.height() - expect).abs() < 0.01,
                "px_scale({em}) 的行高应为 {expect:.2}（= em × {ratio:.3}），实测 {}。\
                 em 与行高混用时这里恰好等于 em —— 那是 0.75 倍文字的成因",
                scaled.height()
            );
        }
    }

    /// 大写字母的位图必须是**cap 高度**（≈0.7em），不是 0.75 倍缩放后的 0.53em。
    ///
    /// 实测过：34px 的 'Z' 位图曾只有 15×18（GDI 基准 19×24）。0.75 这个比例
    /// 对所有拉丁字体都成立，所以阈值取 0.55em / 0.9em 两侧夹。
    #[test]
    fn capital_ink_is_cap_height_not_three_quarters() {
        use zephyra_core::text::GlyphSource as _;
        let Ok(bytes) = std::fs::read(FONT_CANDIDATES[0]) else {
            eprintln!("跳过：找不到系统字体");
            return;
        };
        let mut src = match AbGlyphSource::from_bytes(0, &bytes) {
            Ok(s) => s,
            Err(_) => return,
        };
        let id = src.glyph_id('H');
        let r = src.rasterize(id, 34).expect("'H' 必须栅格化出位图");
        let ratio = r.height as f32 / 34.0;
        assert!(
            (0.55..0.90).contains(&ratio),
            "'H'@34px 位图高 {} = {ratio:.2}em；cap 高度应约 0.7em。\
             实测 0.5 出头说明字被按 upem/hhea 比例缩小了（em 与行高混用）",
            r.height
        );
    }

    /// offset 必须是**屏幕方向**（y 向下，基线之上为负）。
    ///
    /// 这条钉的是第二个 bug：offset 的约定说反后，core 的 `dst.y` 公式也跟着反，
    /// 每个字形的**顶部贴到基线**上 —— 同一行里 'h' 与 'e' 顶相同、底差一个字高，
    /// descender 整个不见。实测 34px 行 'e' 顶 71 / 'h' 顶 72（应相差约 7）。
    #[test]
    fn raster_offset_is_screen_space_y_down() {
        use zephyra_core::text::GlyphSource as _;
        let Ok(bytes) = std::fs::read(FONT_CANDIDATES[0]) else {
            eprintln!("跳过：找不到系统字体");
            return;
        };
        let mut src = match AbGlyphSource::from_bytes(0, &bytes) {
            Ok(s) => s,
            Err(_) => return,
        };
        // 'H'：无 descender，整个位图在基线**之上** → offset.y 负、bottom 恰在 0 附近。
        let h_id = src.glyph_id('H');
        let h = src.rasterize(h_id, 34).expect("'H'");
        assert!(
            h.offset.y < 0.0,
            "'H' 的 offset.y 应为负（位图在基线之上），实测 {}",
            h.offset.y
        );
        assert!(
            (h.offset.y + h.height as f32).abs() <= 1.5,
            "'H' 无 descender，位图底应落在基线上，实测 {}",
            h.offset.y + h.height as f32
        );
        // 'p'：有 descender → 位图底伸到基线**下面**（offset.y + height > 0）。
        let p_id = src.glyph_id('p');
        let p = src.rasterize(p_id, 34).expect("'p'");
        assert!(
            p.offset.y < 0.0,
            "'p' 的 offset.y 应为负（位图主体在基线之上），实测 {}",
            p.offset.y
        );
        assert!(
            p.offset.y + p.height as f32 > 1.0,
            "'p' 有 descender，位图底应在基线之下，实测 {}",
            p.offset.y + p.height as f32
        );
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

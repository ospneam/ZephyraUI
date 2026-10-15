// 本文件由 svg.rs 的 #[cfg(test)] mod tests 抽出 —— 测试代码与生产代码分离。
// 模块路径不变（src 里的声明用 #[path] 指过来），所以 use super::* 与私有可见性都照旧。

    use super::*;
    use super::Segment::{Close, LineTo, MoveTo};

    #[test]
    fn typical_current_color_icon() {
        // 复制自 assets 图标的典型结构：svg fill=none + path fill=currentColor
        let text = r#"<svg width="24" height="24" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg"><path d="M2 5.75C2 5.34 2.34 5 2.75 5h18.5c.41 0 .75.34.75.75S21.66 6.5 21.25 6.5H2.75C2.34 6.5 2 6.16 2 5.75Z" fill="currentColor"/></svg>"#;
        let icon = parse_svg(text).unwrap();
        assert_eq!(icon.view_box, (0.0, 0.0, 24.0, 24.0));
        assert_eq!(icon.paint.mode, PaintMode::CurrentColor);
        assert_eq!(icon.paint.rule, FillRule::NonZero);
        assert!(matches!(icon.segments[0], MoveTo(2.0, 5.75)));
    }

    #[test]
    fn evenodd_is_propagated() {
        let text = r#"<svg viewBox="0 0 24 24"><path fill-rule="evenodd" d="M0 0L24 0L0 24Z M0 0L0 24L24 0Z"/></svg>"#;
        let icon = parse_svg(text).unwrap();
        assert_eq!(icon.paint.rule, FillRule::EvenOdd);
    }

    #[test]
    fn none_paths_are_skipped() {
        let text = r##"<svg viewBox="0 0 24 24"><path d="M1 1L2 2" fill="none"/><path d="M0 0L5 5Z" fill="#212121"/></svg>"##;
        let icon = parse_svg(text).unwrap();
        assert_eq!(icon.segments, vec![MoveTo(0.0, 0.0), LineTo(5.0, 5.0), Close]);
        match icon.paint.mode {
            PaintMode::Solid(c) => {
                // 0x21 = 33
                let v = 33.0 / 255.0;
                assert!((c[0] - v).abs() < 1e-4 && (c[1] - v).abs() < 1e-4);
            }
            other => panic!("expected solid, got {other:?}"),
        }
    }

    #[test]
    fn multiple_paths_merge() {
        let text = r#"<svg viewBox="0 0 24 24"><path d="M0 0L1 1"/><path d="M2 2L3 3"/><path d="M4 4Z"/></svg>"#;
        let icon = parse_svg(text).unwrap();
        // 每条 path 各产生 MoveTo + 一段 = 6 段
        assert_eq!(icon.segments.len(), 6);
    }

    #[test]
    fn svg_level_fill_rule_fallback() {
        let text = r#"<svg viewBox="0 0 24 24" fill-rule="evenodd"><path d="M0 0L1 1"/></svg>"#;
        let icon = parse_svg(text).unwrap();
        assert_eq!(icon.paint.rule, FillRule::EvenOdd);
    }

    #[test]
    fn comments_and_ignored_tags() {
        let text = r#"<?xml version="1.0"?><!-- icon --><svg viewBox="0 0 24 24"><title>x</title><path d="M0 0L1 1"/></svg>"#;
        let icon = parse_svg(text).unwrap();
        assert_eq!(icon.segments.len(), 2);
    }

    #[test]
    fn errors() {
        assert_eq!(parse_svg("<path d='M0 0L1 1'/>"), Err(SvgError::NoSvgRoot));
        assert_eq!(parse_svg("<svg></svg>"), Err(SvgError::NoViewBox));
        assert_eq!(parse_svg("<svg><path d='M0 0'/></svg>"), Err(SvgError::NoViewBox));
        assert_eq!(
            parse_svg("<svg viewBox='0 0 24'><path d='M0 0'/></svg>"),
            Err(SvgError::BadViewBox)
        );
        assert_eq!(
            parse_svg("<svg viewBox='0 0 24 24'><rect/></svg>"),
            Err(SvgError::NoPath)
        );
    }

    #[test]
    fn gt_inside_quotes_is_not_tag_end() {
        // 属性值内含 '>' 时标签解析不提前截断
        let text = r#"<svg viewBox="0 0 24 24"><path data-x="a>b" d="M0 0L1 1"/></svg>"#;
        let icon = parse_svg(text).unwrap();
        assert_eq!(icon.segments.len(), 2);
    }

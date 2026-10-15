// 本文件由 path.rs 的 #[cfg(test)] mod tests 抽出 —— 测试代码与生产代码分离。
// 模块路径不变（src 里的声明用 #[path] 指过来），所以 use super::* 与私有可见性都照旧。

    use super::*;
    use crate::icon::Segment::{Close, CubicTo, LineTo, MoveTo, QuadraticTo};

    fn segs(d: &str) -> Vec<Segment> {
        parse_path_data(d).unwrap_or_else(|e| panic!("parse {d:?} failed: {e:?}"))
    }

    fn assert_close(a: f32, b: f32) {
        assert!((a - b).abs() < 1e-4, "{a} vs {b}");
    }

    #[test]
    fn absolute_and_relative_lines() {
        assert_eq!(segs("M 10 10 L 20 10 L 20 20 Z"), vec![
            MoveTo(10.0, 10.0),
            LineTo(20.0, 10.0),
            LineTo(20.0, 20.0),
            Close,
        ]);
        assert_eq!(segs("m 10 10 l 10 0 l 0 10 z"), vec![
            MoveTo(10.0, 10.0),
            LineTo(20.0, 10.0),
            LineTo(20.0, 20.0),
            Close,
        ]);
    }

    #[test]
    fn implicit_repeats() {
        // M 的隐式后续是 L
        assert_eq!(segs("M 0 0 10 10 20 20"), vec![
            MoveTo(0.0, 0.0),
            LineTo(10.0, 10.0),
            LineTo(20.0, 20.0),
        ]);
        // m 的隐式后续是 l
        assert_eq!(segs("m 0 0 10 10"), vec![
            MoveTo(0.0, 0.0),
            LineTo(10.0, 10.0),
        ]);
        // L 的隐式重复仍是 L
        assert_eq!(segs("M 0 0 L 1 1 2 2 3 3"), vec![
            MoveTo(0.0, 0.0),
            LineTo(1.0, 1.0),
            LineTo(2.0, 2.0),
            LineTo(3.0, 3.0),
        ]);
    }

    #[test]
    fn horizontal_vertical() {
        assert_eq!(segs("M 5 5 H 15 V 25"), vec![
            MoveTo(5.0, 5.0),
            LineTo(15.0, 5.0),
            LineTo(15.0, 25.0),
        ]);
        assert_eq!(segs("M 5 5 h 10 v 20"), vec![
            MoveTo(5.0, 5.0),
            LineTo(15.0, 5.0),
            LineTo(15.0, 25.0),
        ]);
    }

    #[test]
    fn smooth_cubic_reflection() {
        // S 的第一控制点 = 2·cur − prev_c2 = 2·(3,0) − (2,1) = (4,−1)
        assert_eq!(segs("M 0 0 C 1 1 2 1 3 0 S 5 -1 6 0"), vec![
            MoveTo(0.0, 0.0),
            CubicTo(1.0, 1.0, 2.0, 1.0, 3.0, 0.0),
            CubicTo(4.0, -1.0, 5.0, -1.0, 6.0, 0.0),
        ]);
    }

    #[test]
    fn smooth_quadratic_reflection() {
        // T 的控制点 = 2·(4,0) − (2,2) = (6,−2)
        assert_eq!(segs("M 0 0 Q 2 2 4 0 T 8 0"), vec![
            MoveTo(0.0, 0.0),
            QuadraticTo(2.0, 2.0, 4.0, 0.0),
            QuadraticTo(6.0, -2.0, 8.0, 0.0),
        ]);
    }

    #[test]
    fn number_formats() {
        // .5 / 5. / 指数 / 负号分隔
        assert_eq!(segs("M.5.5L1e1-1e-1"), vec![
            MoveTo(0.5, 0.5),
            LineTo(10.0, -0.1),
        ]);
        assert_eq!(segs("M 5. 5."), vec![MoveTo(5.0, 5.0)]);
    }

    #[test]
    fn quarter_circle_arc() {
        // 90° 圆弧：M 0 0 A 10 10 0 0 1 10 10，中心 (0,10)，kappa 展开单段
        // （out[0] 是 MoveTo，out[1] 才是弧段）
        let out = segs("M 0 0 A 10 10 0 0 1 10 10");
        assert_eq!(out.len(), 2);
        match out[1] {
            CubicTo(x1, y1, x2, y2, x, y) => {
                assert_close(x1, 5.5228);
                assert_close(y1, 0.0);
                assert_close(x2, 10.0);
                assert_close(y2, 4.4772);
                assert_close(x, 10.0);
                assert_close(y, 10.0);
            }
            other => panic!("expected cubic, got {other:?}"),
        }
    }

    #[test]
    fn semicircle_arc_two_segments() {
        // 半圆：180° → 2 段 cubic（加 MoveTo 共 3）
        let out = segs("M 0 0 A 5 5 0 1 1 10 0");
        assert_eq!(out.len(), 3);
        match out[2] {
            CubicTo(_, _, _, _, x, y) => {
                assert_close(x, 10.0);
                assert_close(y, 0.0);
            }
            other => panic!("expected cubic, got {other:?}"),
        }
    }

    #[test]
    fn rotated_arc_reaches_endpoint() {
        let out = segs("M 0 0 A 10 5 45 0 1 10 10");
        assert!(!out.is_empty());
        match *out.last().unwrap() {
            CubicTo(_, _, _, _, x, y) => {
                assert_close(x, 10.0);
                assert_close(y, 10.0);
            }
            other => panic!("expected cubic, got {other:?}"),
        }
    }

    #[test]
    fn small_radii_scale_up() {
        // 端点距 14.14 > 2r=2 → 半径按规范放大，仍产生合法弧且到达端点
        let out = segs("M 0 0 A 1 1 0 0 1 10 10");
        match *out.last().unwrap() {
            CubicTo(_, _, _, _, x, y) => {
                assert_close(x, 10.0);
                assert_close(y, 10.0);
            }
            other => panic!("expected cubic, got {other:?}"),
        }
    }

    #[test]
    fn degenerate_arcs() {
        // 端点重合 → 弧零段（仅剩 MoveTo）
        assert_eq!(segs("M 0 0 A 5 5 0 0 1 0 0"), vec![MoveTo(0.0, 0.0)]);
        // rx=0 → 直线
        assert_eq!(segs("M 0 0 A 0 5 0 0 1 10 10"), vec![
            MoveTo(0.0, 0.0),
            LineTo(10.0, 10.0),
        ]);
    }

    #[test]
    fn arc_flags_without_separator() {
        // large/sweep 标志可与后续数字连写；
        // 端点距 14.14 > 2r=10 → 半径放大到 7.07（端点距=直径 → 180° → 2 段 cubic）
        let out = segs("M 0 0 A 5 5 0 0110 10");
        assert_eq!(out.len(), 3);
        match out[2] {
            CubicTo(_, _, _, _, x, y) => {
                assert_close(x, 10.0);
                assert_close(y, 10.0);
            }
            other => panic!("expected cubic, got {other:?}"),
        }
    }

    #[test]
    fn close_resets_current_point() {
        // Z 后当前点回到子路径起点；新 M 重新定义子路径
        let out = segs("M 0 0 L 5 5 Z L 10 0");
        assert_eq!(out, vec![
            MoveTo(0.0, 0.0),
            LineTo(5.0, 5.0),
            Close,
            LineTo(10.0, 0.0),
        ]);
    }

    #[test]
    fn evenodd_style_path_with_many_arcs() {
        // 取自一枚 Bootstrap 风格图标（evenodd 描边、大量小弧）的 `d`。
        // 该图标本身已随工具栏精简移除，但这段 path 数据仍是最严苛的解析
        // 样本：相对弧（a 命令）、隐式重复命令、无分隔符负数、fill-rule 闭合。
        // 数据内联在此，解析器不依赖任何资产文件。
        let d = "M12 0a.75.75 0 0 1 .71.506L13.569 3H21a1.5 1.5 0 0 1 1.5 1.5v10.5a1.5 1.5 0 0 1-1.5 1.5h-2.775l1.985 5.756a.75.75 0 1 1-1.419.489L16.638 16.5H12.75v4.5a.75.75 0 0 1-1.5 0v-4.5H7.362l-2.153 6.245a.75.75 0 1 1-1.419-.489L5.775 16.5H3a1.5 1.5 0 0 1-1.5-1.5V4.5a1.5 1.5 0 0 1 1.5-1.5h7.431l.86-2.495A.75.75 0 0 1 12 0M3 4.5v10.5h18V4.5z";
        let out = segs(d);
        // 首 MoveTo(12,0)，尾 Close；段数充足
        assert!(matches!(out[0], MoveTo(12.0, 0.0)));
        assert!(matches!(*out.last().unwrap(), Close));
        assert!(out.len() > 30);
    }

    #[test]
    fn errors() {
        assert_eq!(parse_path_data(""), Ok(Vec::new()));
        assert_eq!(parse_path_data("   "), Ok(Vec::new()));
        assert_eq!(parse_path_data("M"), Err(ParseError::UnexpectedEof));
        assert_eq!(parse_path_data("M 1 1 Q"), Err(ParseError::UnexpectedEof));
        assert_eq!(parse_path_data("X 1 1"), Err(ParseError::UnknownCommand('X')));
        assert_eq!(parse_path_data("M 1 1 L 1 e"), Err(ParseError::BadNumber));
        assert_eq!(parse_path_data("M 1 1 A 5 5 0 2 0 5 5"), Err(ParseError::BadArcFlag));
        assert_eq!(parse_path_data("123"), Err(ParseError::BadNumber));
        // Z 后跟数字：非法
        assert!(parse_path_data("M 0 0 Z 3 4").is_err());
    }

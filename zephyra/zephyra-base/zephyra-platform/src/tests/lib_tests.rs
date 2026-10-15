// 本文件由 lib.rs 的 #[cfg(test)] mod tests 抽出 —— 测试代码与生产代码分离。
// 模块路径不变（src 里的声明用 #[path] 指过来），所以 use super::* 与私有可见性都照旧。

    use super::*;

    #[test]
    fn corner_values_map_one_to_one_onto_doc_enum() {
        // 数值必须与 DWM_WINDOW_CORNER_PREFERENCE 一致（docs/win/01）。
        assert_eq!(CornerPreference::Default.raw(), 0);
        assert_eq!(CornerPreference::DoNotRound.raw(), 1);
        assert_eq!(CornerPreference::Round.raw(), 2);
        assert_eq!(CornerPreference::RoundSmall.raw(), 3);
        for p in [
            CornerPreference::Default,
            CornerPreference::DoNotRound,
            CornerPreference::Round,
            CornerPreference::RoundSmall,
        ] {
            assert_eq!(CornerPreference::from_raw(p.raw()), p, "往返必须无损");
        }
    }

    #[test]
    fn undocumented_corner_value_falls_back_instead_of_erroring() {
        // 新版 Windows 加第五档时，我们不该把窗口弄坏。
        assert_eq!(CornerPreference::from_raw(99), CornerPreference::Default);
        assert_eq!(CornerPreference::from_raw(-1), CornerPreference::Default);
    }

    #[test]
    fn null_handle_is_invalid() {
        assert!(!WindowHandle::from_raw(0).is_valid());
        assert!(WindowHandle::from_raw(0x1234).is_valid());
    }

    #[test]
    fn handle_debug_output_has_no_addresses() {
        // 裸地址在日志里既没信息量又指向可能已释放的窗口。
        let s = format!("{:?}", WindowHandle::from_raw(0x7FFF_ABCD_1234));
        assert!(!s.contains("7FFF"), "调试输出不该含句柄值：{s}");
    }

    #[test]
    fn current_platform_defaults_are_self_consistent() {
        let p = current();
        let caps = p.capabilities();
        // 能力位与实际行为必须一致：声称支持就必须真的返回 true，
        // 否则上层会按"支持"走分支然后拿到 false，静默降级失败。
        let h = WindowHandle::from_raw(0);
        let out = p.apply_window_chrome(
            h,
            WindowChrome::minimal(true, CornerPreference::Round),
        );
        if caps.dark_mode {
            assert!(out.dark, "{} 声称支持暗色模式却没生效", p.name());
        } else {
            assert!(!out.dark, "{} 不声称支持却生效了", p.name());
        }
        if caps.corner_preference {
            assert!(out.corner);
        } else {
            assert!(!out.corner);
        }
    }

    #[test]
    fn null_window_handle_enables_nothing() {
        // 句柄无效时实现必须静默返回全 false，而不是碰 0 号地址。
        let out = current().apply_window_chrome(
            WindowHandle::from_raw(0),
            WindowChrome::minimal(true, CornerPreference::Round),
        );
        assert!(!out.dark && !out.corner && !out.caption_color && !out.border_color);
    }

// 本文件由 responsive.rs 的 #[cfg(test)] mod tests 抽出 —— 测试代码与生产代码分离。
// 模块路径不变（src 里的声明用 #[path] 指过来），所以 use super::* 与私有可见性都照旧。

    use super::*;

    /// 断点按官方的三个边界值判，一格都不偏。
    ///
    /// 官方原文：小 = 最大可达 640；中 = 641 - 1007；大 = 1008 及以上。
    /// 边界两侧都取，因为"≤ 640 算小"与"< 640 算小"只差这一个像素。
    #[test]
    fn 断点按官方的三个边界判() {
        assert_eq!(WidthClass::of_effective_width(640.0), WidthClass::Small);
        assert_eq!(WidthClass::of_effective_width(641.0), WidthClass::Medium);
        assert_eq!(WidthClass::of_effective_width(1007.0), WidthClass::Medium);
        assert_eq!(WidthClass::of_effective_width(1008.0), WidthClass::Large);
        // 极值不能 panic。
        assert_eq!(WidthClass::of_effective_width(0.0), WidthClass::Small);
        assert_eq!(
            WidthClass::of_effective_width(f32::INFINITY),
            WidthClass::Large
        );
    }

    /// 物理视口必须**先除 scale** 再判 —— 这一条是本模块的口径判据。
    ///
    /// 本机 scale 1.25：1400 物理 = 1120 epx（大），800 物理 = 640 epx（中）。
    /// 若不除，800 会被判成 Small，于是该展开的导航被折叠 —— 症状是布局不合身，
    /// 而不是报错。
    #[test]
    fn 断点判据用有效像素而不是物理像素() {
        assert_eq!(
            WidthClass::of_viewport((1400.0, 800.0), 1.25),
            WidthClass::Large,
            "1400 物理 @1.25 = 1120 epx，应为大"
        );
        assert_eq!(
            WidthClass::of_viewport((800.0, 600.0), 1.25),
            WidthClass::Small,
            "800 物理 @1.25 = 640 epx，而官方「小 = 最大可达 640」⇒ 640 属小"
        );
        // 反过来：同一个物理宽度，scale 不同必须能落到不同档。
        assert_eq!(
            WidthClass::of_viewport((900.0, 600.0), 1.0),
            WidthClass::Medium
        );
        assert_eq!(
            WidthClass::of_viewport((900.0, 600.0), 2.0),
            WidthClass::Small,
            "900 物理 @2.0 = 450 epx ⇒ 小。不除 scale 会判成中"
        );
    }

    /// scale 退化时不能 panic、也不能判出荒谬结果。
    #[test]
    fn scale_退化时退回_1_0() {
        for bad in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert_eq!(
                WidthClass::of_viewport((900.0, 600.0), bad),
                WidthClass::Medium,
                "scale={bad} 时应按 1.0 折算"
            );
        }
    }

    /// 列数：可用宽度每涨一个"卡宽 + 间距"才多一列，且不越界。
    #[test]
    fn 列数随可用宽度增长且至少一列() {
        // 200 卡宽 + 12 间距 = 212 步长。n 列占宽 = n×212 − 12。
        assert_eq!(columns_for(212.0, 200.0, 12.0), 1);
        assert_eq!(columns_for(411.0, 200.0, 12.0), 1, "第二列要 412 起");
        assert_eq!(columns_for(412.0, 200.0, 12.0), 2, "正好两列占宽");
        assert_eq!(columns_for(423.0, 200.0, 12.0), 2);
        // 极窄 / 非法入参都要退回 1 列，而不是 0 列或 panic。
        assert_eq!(columns_for(0.0, 200.0, 12.0), 1);
        assert_eq!(columns_for(50.0, 200.0, 12.0), 1);
        assert_eq!(columns_for(f32::NAN, 200.0, 12.0), 1);
        assert_eq!(columns_for(1136.0, 0.0, 12.0), 1);
        // 基准画布（客户区 1400，导航 240，内容区 1136）下要正好 5 列 ——
        // 这一条同时是「改了列数公式不许破坏基准外观」的保险。
        assert_eq!(columns_for(1136.0, 200.0, 12.0), 5);
    }

    /// 每档的最少列数是**下限**，不是目标值 —— 真正的列数由可用宽度决定。
    #[test]
    fn 最少列数单调不减() {
        assert!(WidthClass::Small.min_columns() <= WidthClass::Medium.min_columns());
        assert!(WidthClass::Medium.min_columns() <= WidthClass::Large.min_columns());
    }

    /// 触控目标换算：epx → 布局单位必须乘 scale。
    ///
    /// 这一条是官方硬下限的正确性判据。1.25 缩放下 40 epx = 50 物理 px；
    /// 忘了乘就会得到 40 物理 = 32 epx，**低于官方"可触摸"下限**，
    /// 而代码读起来完全正确。
    #[test]
    fn 触控目标按_scale_换算成布局单位() {
        assert_eq!(touch_px(TOUCH_MIN_EPX, 1.25), 50.0);
        assert_eq!(touch_px(TOUCH_OPTIMAL_EPX, 1.25), 55.0);
        assert_eq!(touch_px(TOUCH_MIN_EPX, 1.0), 40.0, "100% 下应恒等");
        assert_eq!(touch_px(TOUCH_MIN_EPX, 0.0), 40.0, "scale 退化时按 1.0");
    }

    /// 官方数字本身要钉住，防止被"顺手调参"改掉。
    ///
    /// 判据直接抄原文。改这些数字必须同时改 `docs/responsive-and-touch.md`
    /// 里的引用，否则这条会红。
    #[test]
    fn 官方数字必须与文档一致() {
        assert_eq!(WidthClass::SMALL_MAX_EPX, 640.0);
        assert_eq!(WidthClass::LARGE_MIN_EPX, 1008.0);
        assert_eq!(TOUCH_MIN_EPX, 40.0);
        assert_eq!(TOUCH_OPTIMAL_EPX, 44.0);
        assert_eq!(TOUCH_GAP_EPX, 4.0);
        assert_eq!(TOUCH_ROW_HEIGHT_EPX, 32.0);
        assert_eq!(TOUCH_ROW_WIDTH_EPX, 120.0);
        // 4 的倍数：官方要求尺寸/边距/位置落在 4 epx 的倍数上。
        for v in [TOUCH_MIN_EPX, TOUCH_OPTIMAL_EPX, TOUCH_GAP_EPX, TOUCH_ROW_HEIGHT_EPX, TOUCH_ROW_WIDTH_EPX] {
            assert_eq!(
                v % 4.0,
                0.0,
                "{v} 不是 4 的倍数 —— 官方要求基单位为 4（缩放 125% 等时才是整数像素）"
            );
        }
    }

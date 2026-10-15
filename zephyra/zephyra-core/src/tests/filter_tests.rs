// 本文件由 filter.rs 的 #[cfg(test)] mod tests 抽出 —— 测试代码与生产代码分离。
// 模块路径不变（src 里的声明用 #[path] 指过来），所以 use super::* 与私有可见性都照旧。

    use super::*;

    /// `BlendMode` 的 29 个变体与 Skia 的 `SkBlendMode` **逐值同序**。
    ///
    /// 这条测试的价值在于：一旦有人在本枚举中间插入变体，两侧的下标就全错位了，
    /// 而错位不会报错 —— 只会让 `kMultiply` 悄悄按 `kOverlay` 渲染。
    #[test]
    fn 混合模式与_skia_逐值同序() {
        // 按 Skia `SkBlendMode.h:38-74` 的声明顺序抄下来，不允许改序。
        let skia_order = [
            BlendMode::Clear,
            BlendMode::Src,
            BlendMode::Dst,
            BlendMode::SrcOver,
            BlendMode::DstOver,
            BlendMode::SrcIn,
            BlendMode::DstIn,
            BlendMode::SrcOut,
            BlendMode::DstOut,
            BlendMode::SrcATop,
            BlendMode::DstATop,
            BlendMode::Xor,
            BlendMode::Plus,
            BlendMode::Modulate,
            BlendMode::Screen,
            BlendMode::Overlay,
            BlendMode::Darken,
            BlendMode::Lighten,
            BlendMode::ColorDodge,
            BlendMode::ColorBurn,
            BlendMode::HardLight,
            BlendMode::SoftLight,
            BlendMode::Difference,
            BlendMode::Exclusion,
            BlendMode::Multiply,
            BlendMode::Hue,
            BlendMode::Saturation,
            BlendMode::Color,
            BlendMode::Luminosity,
        ];
        assert_eq!(
            skia_order.len(),
            BlendMode::COUNT,
            "Skia 的 kSkBlendModeCount 是 29；本枚举若增删了变体，这条测试该同步"
        );
        // `#[derive(Ord)]` 给出的是声明序下的序数，逐一断言它落在下标 i 上。
        for (i, m) in skia_order.iter().enumerate() {
            assert_eq!(
                *m as usize,
                i,
                "{m:?} 的序数是 {} 而非 {i} —— 有人改了枚举顺序",
                *m as usize
            );
        }
        assert_eq!(skia_order[BlendMode::COUNT - 1], BlendMode::Luminosity);
    }

    /// 系数表逐项对齐 Skia `SkBlendMode_AsCoeff`（`SkBlendMode.cpp:68-84`）。
    ///
    /// 这张表是 GPU 混合状态与 CPU 参考光栅器的**唯一共同基准**：两侧若不一致，
    /// 同一DisplayList 在 GPU 上与 CPU 上会画出不同的颜色，而测试全绿。
    #[test]
    fn 系数表与_skia_逐项对齐() {
        use BlendMode::*;
        use Coeff::*;
        // (src coeff, dst coeff)，抄自 kCoeffs。
        let expected = [
            (Clear, Zero, Zero),
            (Src, One, Zero),
            (Dst, Zero, One),
            (SrcOver, One, OneMinusSrcAlpha),
            (DstOver, OneMinusDstAlpha, One),
            (SrcIn, DstAlpha, Zero),
            (DstIn, Zero, SrcAlpha),
            (SrcOut, OneMinusDstAlpha, Zero),
            (DstOut, Zero, OneMinusSrcAlpha),
            (SrcATop, DstAlpha, OneMinusSrcAlpha),
            (DstATop, OneMinusDstAlpha, SrcAlpha),
            (Xor, OneMinusDstAlpha, OneMinusSrcAlpha),
            (Plus, One, One),
            (Modulate, Zero, SrcColor),
            (Screen, One, OneMinusSrcColor),
        ];
        for (mode, src, dst) in expected {
            assert_eq!(
                mode.as_coeff(),
                Some((src, dst)),
                "{mode:?} 的系数对不对"
            );
        }
    }

    /// 只有 Porter-Duff 那 15 个能用系数表达，且分类与 Skia 的两个 Last 常量一致。
    #[test]
    fn 只有前十五个能用系数表达() {
        for m in [
            BlendMode::Clear,
            BlendMode::Modulate,
            BlendMode::Screen,
            BlendMode::Multiply,
            BlendMode::Overlay,
            BlendMode::Hue,
            BlendMode::Luminosity,
        ] {
            assert_eq!(
                m.as_coeff().is_some(),
                m.is_coeff(),
                "{m:?} 的 is_coeff 与 as_coeff 自相矛盾"
            );
        }
        // 系数模式必然可分离，但反之不成立。
        for i in 0..BlendMode::COUNT {
            let m = enum_by_index(i);
            if m.is_coeff() {
                assert!(m.is_separable(), "{m:?} 是系数模式却不可分离");
            }
        }
        assert!(BlendMode::Overlay.is_separable());
        assert!(!BlendMode::Overlay.is_coeff());
        assert!(!BlendMode::Hue.is_separable());
    }

    /// `Overlay` 在系数表之外，且`Multiply` 是最后一个可分离模式。
    #[test]
    fn 分类边界与_skia_一致() {
        assert_eq!(BlendMode::LAST_COEFF_MODE, BlendMode::Screen);
        assert_eq!(BlendMode::LAST_SEPARABLE_MODE, BlendMode::Multiply);
        assert!(BlendMode::Screen.as_coeff().is_some());
        assert!(BlendMode::Overlay.as_coeff().is_none());
        assert!(BlendMode::Multiply.as_coeff().is_none());
    }

    /// blur 的外扩是 `3σ`，且各向异性。
    #[test]
    fn blur_外扩三倍标准差() {
        let o = Filter::Blur {
            sigma_x: 2.0,
            sigma_y: 5.0,
        }
        .outsets();
        assert_eq!(o.left, 6.0, "3 × 2");
        assert_eq!(o.top, 15.0, "3 × 5");
        assert_eq!(o.right, 6.0);
        assert_eq!(o.bottom, 15.0);
    }

    /// 非有限σ 必须退化成 0 外扩，而不是产生 NaN 矩形。
    ///
    /// 这条是真实防御：负σ 会让 outset 为负，离屏矩形**比**原矩形还小，
    /// 模糊在边界处被切掉；而 NaN 会让整个矩形坐标变 NaN，光栅化器直接丢弃
    /// 一切（症状是"元素凭空消失"）。
    #[test]
    fn 非有限或非正标准差退化成零() {
        for bad in [-1.0, 0.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let f = Filter::Blur {
                sigma_x: bad,
                sigma_y: 2.0,
            };
            assert_eq!(
                f.outsets().left,
                0.0,
                "σ={bad} 应当退化成 0 半径（左）"
            );
            assert_eq!(f.outsets().top, 6.0, "另一轴仍是 3 × 2");
            let b = FilterChain::single(f).bounds_for(Rect::new(10.0, 10.0, 100.0, 50.0));
            assert!(b.x.is_finite() && b.width.is_finite(), "矩形不能是 NaN");
            assert!(
                b.width >= 100.0,
                "外扩不能为负（σ={bad} 得到 width={}）",
                b.width
            );
        }
    }

    /// 投影的不对称外扩：右下投影只扩右下。
    #[test]
    fn 投影按方向不对称外扩() {
        let f = Filter::DropShadow {
            color: Color::rgba(0.0, 0.0, 0.0, 0.4),
            offset: Point::new(4.0, 6.0),
            sigma: 2.0,
        };
        let o = f.outsets();
        assert_eq!(
            (o.left, o.top, o.right, o.bottom),
            (2.0, 0.0, 10.0, 12.0),
            "右下投影 (4,6) + 半径 6：右侧 6+4=10、下侧 6+6=12，\
             而左侧被 offset 抵消成 6-4=2、上侧恰好抵消成 0"
        );
        let b = FilterChain::single(f).bounds_for(Rect::new(0.0, 0.0, 100.0, 50.0));
        assert_eq!((b.x, b.y, b.width, b.height), (-2.0, 0.0, 112.0, 62.0));
    }

    /// 向左上投影：左上扩得更多，右下只留 blur 半径。
    #[test]
    fn 左上投影往左上扩() {
        let f = Filter::DropShadow {
            color: Color::rgba(0.0, 0.0, 0.0, 0.4),
            offset: Point::new(-10.0, -4.0),
            sigma: 2.0,
        };
        let o = f.outsets();
        assert_eq!(
            (o.left, o.top, o.right, o.bottom),
            (16.0, 10.0, 0.0, 2.0),
            "左上投影 (-10,-4) + 半径 6：左侧 6+10=16、上侧 6+4=10，\
             右侧被 offset 抵消成 0、下侧剩 6-4=2"
        );
        let b = FilterChain::single(f).bounds_for(Rect::new(0.0, 0.0, 100.0, 50.0));
        assert_eq!((b.x, b.y, b.width, b.height), (-16.0, -10.0, 116.0, 62.0));
    }

    /// 串联滤镜取逐边最大值，**不是**求和。
    ///
    /// 求和会让 blur+blur 的离屏矩形大一倍 —— 而实际上只有最后一级的输出
    /// 边缘在起作用。
    #[test]
    fn 串联滤镜取逐边最大值而不是求和() {
        let chain = FilterChain::new([
            Filter::Blur {
                sigma_x: 2.0,
                sigma_y: 2.0,
            },
            Filter::Blur {
                sigma_x: 4.0,
                sigma_y: 1.0,
            },
        ]);
        let o = chain.outsets();
        assert_eq!((o.left, o.right), (12.0, 12.0), "取 max(6, 12)");
        assert_eq!((o.top, o.bottom), (6.0, 6.0), "取 max(6, 3)");
        assert_ne!(
            chain.outsets().left,
            6.0 + 12.0,
            "求和会给 18；这条断言钉住我们用的是 max"
        );
    }

    /// 空链与恒等滤镜都判为恒等，宿主据此跳过开离屏。
    #[test]
    fn 恒等链不开离屏() {
        assert!(FilterChain::none().is_identity());
        assert!(FilterChain::default().is_identity());
        assert!(
            FilterChain::single(Filter::Blur {
                sigma_x: 0.0,
                sigma_y: 0.0
            })
            .is_identity(),
            "σ=0 的 blur 不改变任何像素"
        );
        assert!(!FilterChain::single(Filter::Blur {
            sigma_x: 0.5,
            sigma_y: 0.0
        })
        .is_identity());
        assert!(!FilterChain::single(Filter::DropShadow {
            color: Color::rgba(0.0, 0.0, 0.0, 0.0),
            offset: Point::new(0.0, 0.0),
            sigma: 0.0,
        })
        .is_identity());
    }

    /// 链的顺序被保留：`then` 追加到末尾。
    #[test]
    fn 链顺序即执行顺序() {
        let a = Filter::Blur {
            sigma_x: 1.0,
            sigma_y: 1.0,
        };
        let b = Filter::DropShadow {
            color: Color::rgba(0.0, 0.0, 0.0, 0.5),
            offset: Point::new(1.0, 1.0),
            sigma: 2.0,
        };
        let chain = FilterChain::single(a).then(b);
        assert_eq!(chain.filters(), &[a, b], "先执行的在前");
        assert_eq!(
            FilterChain::new([b, a]).filters(),
            &[b, a],
            "构造顺序即顺序"
        );
    }

    /// 离屏原点：几何坐标仍是屏幕坐标，宿主需要知道往哪儿挪。
    #[test]
    fn 离屏原点是内容原点减去外扩() {
        let f = Filter::Blur {
            sigma_x: 2.0,
            sigma_y: 2.0,
        };
        let chain = FilterChain::single(f);
        let content = Rect::new(100.0, 50.0, 20.0, 20.0);
        let bounds = chain.bounds_for(content);
        let origin = chain.origin_in(content);
        assert_eq!((origin.x, origin.y), (94.0, 44.0));
        // 关键性质：内容左上角在离屏纹理内的局部坐标恒为外扩量。
        assert_eq!(content.x - bounds.x, 6.0);
        assert_eq!(content.y - bounds.y, 6.0);
    }

    /// 恒等链的离屏矩形就是内容本身 —— 尺寸决策不引入任何额外开销。
    #[test]
    fn 恒等链不改变矩形() {
        let r = Rect::new(3.0, 7.0, 40.0, 24.0);
        assert_eq!(FilterChain::none().bounds_for(r), r);
        assert!(FilterChain::none().outsets().is_zero());
    }

    /// `Clear` 不需要目标像素，其余模式都需要 —— 这决定"先清屏还是先画背景"。
    #[test]
    fn 读取目标的模式判定() {
        assert!(!BlendMode::Clear.needs_destination());
        assert!(!BlendMode::Src.needs_destination());
        assert!(BlendMode::SrcOver.needs_destination());
        assert!(BlendMode::Multiply.needs_destination());
    }

    /// `Default` 是 `SrcOver`，与现有渲染器的固定混合一致（零回归的前提）。
    #[test]
    fn 默认混合模式是_src_over() {
        assert_eq!(BlendMode::default(), BlendMode::SrcOver);
    }

    /// 按声明序取变体（供分类测试用）。
    fn enum_by_index(i: usize) -> BlendMode {
        // 与 `混合模式与_skia_逐值同序` 的数组同序。
        const ALL: [BlendMode; BlendMode::COUNT] = [
            BlendMode::Clear,
            BlendMode::Src,
            BlendMode::Dst,
            BlendMode::SrcOver,
            BlendMode::DstOver,
            BlendMode::SrcIn,
            BlendMode::DstIn,
            BlendMode::SrcOut,
            BlendMode::DstOut,
            BlendMode::SrcATop,
            BlendMode::DstATop,
            BlendMode::Xor,
            BlendMode::Plus,
            BlendMode::Modulate,
            BlendMode::Screen,
            BlendMode::Overlay,
            BlendMode::Darken,
            BlendMode::Lighten,
            BlendMode::ColorDodge,
            BlendMode::ColorBurn,
            BlendMode::HardLight,
            BlendMode::SoftLight,
            BlendMode::Difference,
            BlendMode::Exclusion,
            BlendMode::Multiply,
            BlendMode::Hue,
            BlendMode::Saturation,
            BlendMode::Color,
            BlendMode::Luminosity,
        ];
        ALL[i]
    }

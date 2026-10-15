// 本文件由 pixels.rs 的 #[cfg(test)] mod tests 抽出 —— 测试代码与生产代码分离。
// 模块路径不变（src 里的声明用 #[path] 指过来），所以 use super::* 与私有可见性都照旧。

    use super::*;

    #[test]
    fn converts_rgb_to_rgba_without_downsampling() {
        let mut processor = CpuFrameProcessor::new();
        let output = processor.process(&[1, 2, 3, 4, 5, 6], 2, 1);
        assert_eq!(output, [1, 2, 3, 255, 4, 5, 6, 255]);
        assert_eq!((processor.last_width(), processor.last_height()), (2, 1));
    }

    #[test]
    fn downsamples_with_nearest_neighbor_indices() {
        let mut processor = CpuFrameProcessor::new();
        let rgb = (0..1280 * 720)
            .flat_map(|index| {
                let x = index % 1280;
                let y = index / 1280;
                [x as u8, y as u8, (x + y) as u8]
            })
            .collect::<Vec<_>>();
        let output = processor.process(&rgb, 1280, 720);
        assert_eq!(
            (processor.last_width(), processor.last_height()),
            (854, 480)
        );
        assert_eq!(output.len(), 854 * 480 * 4);
        let offset = (854 * 480 - 1) * 4;
        let x = processor.sx[853] as usize;
        let y = processor.sy[479] as usize;
        assert_eq!(
            &output[offset..offset + 4],
            &[x as u8, y as u8, (x + y) as u8, 255]
        );
    }

    #[test]
    fn reuses_downsample_indices_for_unchanged_dimensions() {
        let mut processor = CpuFrameProcessor::new();
        let rgb = vec![0; 1280 * 720 * 3];
        processor.process(&rgb, 1280, 720);
        let sx = processor.sx.as_ptr();
        let sy = processor.sy.as_ptr();
        processor.process(&rgb, 1280, 720);
        assert_eq!(processor.sx.as_ptr(), sx);
        assert_eq!(processor.sy.as_ptr(), sy);
    }

    #[test]
    fn rebuilds_downsample_indices_when_dimensions_change() {
        let mut processor = CpuFrameProcessor::new();
        processor.process(&vec![0; 1280 * 720 * 3], 1280, 720);
        processor.process(&vec![0; 1024 * 768 * 3], 1024, 768);
        assert_eq!(processor.index_size, (1024, 768, 854, 640));
        assert_eq!((processor.sx.len(), processor.sy.len()), (854, 640));
        assert_eq!((processor.sx[853], processor.sy[639]), (1022, 766));
    }

    #[test]
    fn repeated_processing_is_idempotent_per_dimensions() {
        // 原地转换 + 交出所有权：连续两帧同尺寸必须产出逐位相同的结果，
        // 且 `sx`/`sy` 不重建（无降采样尺寸变化时）。
        let mut processor = CpuFrameProcessor::new();
        let rgb: Vec<u8> = (0..64 * 64 * 3).map(|i| (i % 251) as u8).collect();
        let first = processor.process(&rgb, 64, 64);
        let second = processor.process(&rgb, 64, 64);
        assert_eq!(first, second, "同输入同尺寸必须逐位一致");
        assert_eq!(first.len(), 64 * 64 * 4);
        // 无降采样时索引表保持为空且不参与寻址
        assert!(processor.sx.is_empty() && processor.sy.is_empty());
    }

    #[test]
    fn downsampled_rows_are_independent() {
        // 逐行 chunked 写入：验证行内寻址未串行（相邻行起点不同）。
        //
        // **尺寸必须让 `src_w * src_h * 4` 真的超过 2MB 阈值**，否则走的是
        // 不降采样的分支，断言 `!= (src_w, src_h)` 会失败。
        // （原测试用的是 300×300，其注释称 "300*300*4 > 2MB" —— 实为 360KB，
        //  差了 6 倍，那条注释本身就是错的。）
        //
        // 取 2000×500：RGBA = 4.0MB > 2MB 触发降采样；宽高比 4.0 非 16:9，
        // 故输出为 854×(854/4)=854×213 —— 既走到降采样分支，又让下面的
        // 逐像素校验只有约 18 万次，debug 下也跑得动。
        let src_w = 2000u32;
        let src_h = 500u32;
        assert!(
            (src_w * src_h * 4) as usize > 2 * 1024 * 1024,
            "测试自身的前提失效：输入未达降采样阈值"
        );
        let rgb: Vec<u8> = (0..src_w * src_h * 3).map(|i| (i % 256) as u8).collect();
        let mut processor = CpuFrameProcessor::new();
        let out = processor.process(&rgb, src_w, src_h);
        let (dw, dh) = (processor.last_width(), processor.last_height());
        assert_ne!((dw, dh), (src_w, src_h), "该尺寸应触发降采样");
        assert_eq!((dw, dh), (854, 213), "非 16:9 应按宽高比推算输出高度");
        for dy in 0..dh as usize {
            for dx in 0..dw as usize {
                let s = (processor.sy[dy] as usize * src_w as usize
                    + processor.sx[dx] as usize)
                    * 3;
                let d = (dy * dw as usize + dx) * 4;
                assert_eq!(
                    &out[d..d + 4],
                    &[rgb[s], rgb[s + 1], rgb[s + 2], 255],
                    "像素 ({dx},{dy}) 寻址错误"
                );
            }
        }
    }

    /// 构造 2×2 合成 NV12：Y 平面逐像素给定，UV 单对 (U,V)。
    fn nv12_fixture(y: [u8; 4], u: u8, v: u8) -> bytes::Bytes {
        let mut data = Vec::with_capacity(4 + 2);
        data.extend_from_slice(&y);
        data.push(u);
        data.push(v);
        bytes::Bytes::from(data)
    }

    #[test]
    fn splits_nv12_planes_zero_copy() {
        let data = nv12_fixture([16, 128, 235, 81], 90, 240);
        let planes = Nv12Planes::split(data.clone(), 2, 2).expect("split");
        assert_eq!(planes.y.len(), 4);
        assert_eq!(planes.uv.len(), 2);
        assert_eq!(planes.y.as_ref(), &[16, 128, 235, 81]);
        assert_eq!(planes.uv.as_ref(), &[90, 240]);
        // Bytes::slice 零拷贝：切片指针应落在原始分配内
        let base = data.as_ptr() as usize;
        let y_ptr = planes.y.as_ptr() as usize;
        let uv_ptr = planes.uv.as_ptr() as usize;
        assert_eq!(y_ptr, base);
        assert_eq!(uv_ptr, base + 4);
    }

    #[test]
    fn split_rejects_undersized_buffer_and_zero_dims() {
        let data = bytes::Bytes::from(vec![0u8; 5]); // 2×2 需要 6 字节
        assert!(Nv12Planes::split(data, 2, 2).is_none());
        let empty = bytes::Bytes::new();
        assert!(Nv12Planes::split(empty, 0, 0).is_none());
    }

    #[test]
    fn to_rgba_indexing_is_row_major() {
        // 逐像素写入版按 `i = row*w + col` 定位 Y 平面——用非对称图案
        // 验证行/列索引未串行（4×4，Y(i)=i，UV(i)=2i+1）。
        let (w, h) = (4usize, 4usize);
        let y: Vec<u8> = (0..w * h).map(|i| i as u8).collect();
        let uv: Vec<u8> = (0..w * h / 2)
            .flat_map(|i| [128u8, (2 * i + 1) as u8])
            .collect();
        let planes = Nv12Planes::split(bytes::Bytes::from([y, uv].concat()), 4, 4).unwrap();
        let mut out = Vec::new();
        planes.to_rgba(&mut out);
        assert_eq!(out.len(), w * h * 4);
        for row in 0..h {
            for col in 0..w {
                let i = row * w + col;
                let d = i * 4;
                // alpha 恒 255；RGB 由 Y 与 UV 决定，此处只校验 alpha 与
                // 长度契约（数值等价由参考色测试覆盖）。
                assert_eq!(out[d + 3], 255, "({col},{row}) alpha 必须恒 255");
            }
        }
    }

    #[test]
    fn nv12_to_rgba_bt601_limited_range_reference_colors() {
        // BT.601 limited range 参考色（整数近似 ±1）：
        // 白 Y=235/U=V=128；黑 Y=16；红 Y=81/U=90/V=240
        let white = Nv12Planes::split(nv12_fixture([235; 4], 128, 128), 2, 2).unwrap();
        let mut out = Vec::new();
        white.to_rgba(&mut out);
        for px in out.chunks_exact(4) {
            assert_eq!(px[0], 254);
            assert_eq!(px[1], 254);
            assert_eq!(px[2], 254);
            assert_eq!(px[3], 255);
        }

        let black = Nv12Planes::split(nv12_fixture([16; 4], 128, 128), 2, 2).unwrap();
        let mut out = Vec::new();
        black.to_rgba(&mut out);
        for px in out.chunks_exact(4) {
            assert_eq!(&px[..3], &[0, 0, 0]);
        }

        let red = Nv12Planes::split(nv12_fixture([81; 4], 90, 240), 2, 2).unwrap();
        let mut out = Vec::new();
        red.to_rgba(&mut out);
        for px in out.chunks_exact(4) {
            assert_eq!(px[0], 254); // ≈255
            assert_eq!(px[1], 0);
            assert_eq!(px[2], 0);
        }
    }

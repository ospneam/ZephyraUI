// 本文件由 pixels.rs 的 #[cfg(test)] mod tests 抽出 —— 测试代码与生产代码分离。
// 模块路径不变（src 里的声明用 #[path] 指过来），所以 use super::* 与私有可见性都照旧。

    use super::*;

    /// 行距大于 `width * 4` 时必须按行距读，否则整张图斜着串行。
    ///
    /// 这条测试钉住的是**症状**：宽度 3、高 2、`row_pitch = 16`（64 对齐后的常见值）
    /// 的一批像素。按 `width*4 = 12` 读会把第二行读成"第一行的后半 + 第二行的头"。
    #[test]
    fn read_uses_row_pitch_not_width_times_four() {
        // 每行 16 字节：前 12 字节 = 3 个像素（各 4 字节，用该行行号标记颜色，
        // alpha 全 255），后 4 字节是行距填充。
        const PITCH: usize = 16;
        const W: u32 = 3;
        const H: u32 = 3;
        let mk = |row: u8| -> Vec<u8> {
            let mut v = Vec::with_capacity(PITCH);
            for _ in 0..(W as usize) {
                v.extend_from_slice(&[row, row, row, 255]);
            }
            v.resize(PITCH, 0); // 行距填充必须真的存在，否则测不出「按 width*4 读」的后果
            v
        };
        let mut data = vec![0u8; PITCH * H as usize];
        for (y, mark) in [10u8, 20, 30].iter().enumerate() {
            data[y * PITCH..(y + 1) * PITCH].copy_from_slice(&mk(*mark));
        }
        let px = PixelRead {
            data,
            order: ChannelOrder::Bgra,
            row_pitch: PITCH as u32,
            width: W,
            height: H,
            declared_width: W,
            declared_height: H,
        };
        for (y, want) in [(0u32, 10u8), (1, 20), (2, 30)] {
            let row = px.row(y);
            assert_eq!(row.len(), 12, "第 {y} 行应裁到 width*4 = 12 字节");
            for x in 0..W as usize {
                assert_eq!(
                    row[x * 4],
                    want,
                    "第 {y} 行第 {x} 个像素应是 {want} —— 不等说明按 width*4 当行距读了"
                );
            }
        }
        assert_eq!(px.to_tight().len(), (W * H * 4) as usize, "紧凑化后应是 w*h*4 字节");
    }

    /// 声明尺寸与真实尺寸不一致必须能查出来 —— 那意味着画面缺一条边且**不报错**。
    #[test]
    fn size_mismatch_must_be_detected() {
        let px = PixelRead {
            data: vec![0; 600 * 600 * 4],
            order: ChannelOrder::Bgra,
            row_pitch: 600 * 4,
            width: 600,
            height: 600,
            declared_width: 800,
            declared_height: 600,
        };
        assert!(
            !px.size_matches(),
            "声明 800×600 而实际 600×600 —— 必须判为不匹配（症状是右侧 200 像素黑边）"
        );
    }

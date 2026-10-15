// 本文件由 pixels.rs 的 #[cfg(test)] mod channel_order_tests 抽出 —— 测试代码与生产代码分离。
// 模块路径不变（src 里的声明用 #[path] 指过来），所以 use super::* 与私有可见性都照旧。

use super::*;

/// RGBA 输入写出来的 BMP 必须是**原序**：红画素进 B 位、蓝画素进 R 位是全反的。
///
/// 这条钉的是本次实测的故障：`write_bmp` 曾按死 BGRA 解，CPU 画布（RGBA）
///  dump 出来的图 R/B 互换 —— 深色 UI 几乎全灰，肉眼看不出来，
/// 直到一面饱和色横幅（#16305c 变成棕色）才暴露。
#[test]
fn rgba_input_keeps_channels() {
    let px = PixelRead {
        data: vec![
            255, 0, 0, 255, // 纯红
            0, 0, 255, 255, // 纯蓝
            0, 255, 0, 255, // 纯绿
            255, 255, 255, 255,
        ],
        order: ChannelOrder::Rgba,
        row_pitch: 16,
        width: 4,
        height: 1,
        declared_width: 4,
        declared_height: 1,
    };
    let dir = std::env::temp_dir().join("zephyra-bmp-test");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("rgba.bmp");
    write_bmp(path.to_str().unwrap(), &px, false).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    // 文件头 54 字节；BMP 自底向上，单行时顺序不变；24bpp 下 4 像素 = 12 字节。
    let px_data = &bytes[54..];
    // 第一像素 BGR：红 → (0,0,255)
    assert_eq!(&px_data[0..3], &[0, 0, 255], "red must land in the B slot");
    // 第二像素：蓝 → (255,0,0)
    assert_eq!(&px_data[3..6], &[255, 0, 0], "blue must land in the R slot");
}

/// BGRA 输入（D3D 读回）必须走同一条规则，不能为了修 CPU 那条把 GPU 那条弄反。
#[test]
fn bgra_input_keeps_channels() {
    let px = PixelRead {
        // 同一个「纯红」在 BGRA 源里是 [0,0,255,255]。
        data: vec![0, 0, 255, 255],
        order: ChannelOrder::Bgra,
        row_pitch: 4,
        width: 1,
        height: 1,
        declared_width: 1,
        declared_height: 1,
    };
    let dir = std::env::temp_dir().join("zephyra-bmp-test");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("bgra.bmp");
    write_bmp(path.to_str().unwrap(), &px, false).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(&bytes[54..57], &[0, 0, 255], "red must land in the B slot");
}

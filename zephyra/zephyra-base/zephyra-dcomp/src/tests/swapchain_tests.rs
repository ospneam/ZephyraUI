// 本文件由 swapchain.rs 的 #[cfg(test)] mod tests 抽出 —— 测试代码与生产代码分离。
// 模块路径不变（src 里的声明用 #[path] 指过来），所以 use super::* 与私有可见性都照旧。

    use super::*;

    /// 零尺寸不是合法请求，但也不能让它 panic —— `width.max(1)` 的钳制放在
    /// 调用方（surface 构造）是有意的：这里断言"本函数不隐式改写尺寸"，
    /// 也就是钳制责任有唯一归属处，不会出现两处各钳一次导致 resize 判定漂移。
    #[test]
    fn zero_sized_request_is_passed_through_unchanged() {
        // 不建真实 swapchain（需要设备与工厂），只验证描述组装这一段。
        let d = base_swapchain_desc(0, 0, SwapchainDesc::default().format, 2);
        assert_eq!(d.Width, 0);
        assert_eq!(d.Height, 0);
    }

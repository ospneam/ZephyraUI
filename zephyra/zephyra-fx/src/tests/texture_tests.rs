// 本文件由 texture.rs 的 #[cfg(test)] mod tests 抽出 —— 测试代码与生产代码分离。
// 模块路径不变（src 里的声明用 #[path] 指过来），所以 use super::* 与私有可见性都照旧。

    use super::*;

    #[test]
    fn 注册过才_contains() {
        let mut n = TextureNames::new();
        assert!(n.is_empty());
        assert!(!n.contains("logo.png"), "未注册的不能算存在");
        assert!(n.insert("logo.png"));
        assert!(n.contains("logo.png"));
        assert_eq!(n.len(), 1);
        // 重复插入返回 false，且不改变计数。
        assert!(!n.insert("logo.png"));
        assert_eq!(n.len(), 1, "重复插入不应重复计数");
    }

    #[test]
    fn 注销后_contains_变假() {
        let mut n = TextureNames::new();
        n.insert("a.png");
        assert!(n.remove("a.png"));
        assert!(!n.contains("a.png"));
        assert!(!n.remove("a.png"), "注销不存在的应返回 false");
    }

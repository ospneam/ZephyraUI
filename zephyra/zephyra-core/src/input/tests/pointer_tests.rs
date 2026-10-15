// 本文件由 pointer.rs 的 #[cfg(test)] mod tests 抽出 —— 测试代码与生产代码分离。
// 模块路径不变（src 里的声明用 #[path] 指过来），所以 use super::* 与私有可见性都照旧。

use super::*;

#[test]
fn down_move_up_lifecycle() {
    let mut t = PointerTable::new();
    assert_eq!(t.len(), 0);
    t.down(1, 10.0, 20.0);
    t.down(2, 30.0, 40.0);
    assert_eq!(t.len(), 2);
    assert_eq!(t.position_of(1), Some((10.0, 20.0)));
    assert!(t.move_pointer(2, 31.0, 41.0));
    assert_eq!(t.position_of(2), Some((31.0, 41.0)));
    // 未知 ID 的 Move 被忽略
    assert!(!t.move_pointer(9, 0.0, 0.0));
    assert!(t.up(1));
    assert_eq!(t.len(), 1);
    // 重复 Up：已不存在
    assert!(!t.up(1));
}

#[test]
fn duplicate_down_overwrites_in_place() {
    let mut t = PointerTable::new();
    t.down(1, 1.0, 1.0);
    t.down(2, 2.0, 2.0);
    // 缺 Up 的重复 Down：原位覆盖，不新增
    t.down(1, 5.0, 5.0);
    assert_eq!(t.len(), 2);
    assert_eq!(t.position_of(1), Some((5.0, 5.0)));
}

#[test]
fn nth_is_slot_ordered_and_deterministic() {
    let mut t = PointerTable::new();
    t.down(7, 0.0, 0.0);
    t.down(3, 1.0, 1.0);
    assert_eq!(t.nth(0).map(|(id, ..)| id), Some(7));
    assert_eq!(t.nth(1).map(|(id, ..)| id), Some(3));
    assert!(t.nth(2).is_none());
}

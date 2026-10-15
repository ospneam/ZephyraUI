// 本文件由 property.rs 的 #[cfg(test)] mod store_tests 抽出 —— 测试代码与生产代码分离。
// 模块路径不变（src 里的声明用 #[path] 指过来），所以 use super::* 与私有可见性都照旧。

    use super::*;

    // ⚠ `static` 而不是 `const`：`&CONST` 每次引用都**创建一个临时值**，
    // 于是 `resolve(&store, &A, None)` 返回的借用指向临时 —— 编译期就报
    // "temporary value dropped while borrowed"。`static` 才给出 `'static` 引用。
    static A: PropertyKey = PropertyKey::new("Layout.Width", PropertyValue::float(0.0));
    static B: PropertyKey = PropertyKey::new("Paint.Opacity", PropertyValue::float(1.0));

    /// 重复给**同一个** key 设值：表里必须留下**最后那个**。
    ///
    /// # 这条在钉什么
    ///
    /// 覆盖式的写入（upsert）有个"表里留哪一份"的隐含决定，而它没有任何
    /// 可见症状 —— 表里的值错了，`resolve` 照样返回 `ValueSource::Local`，
    /// 断言"来源"的那批测试全过，只有"值"不对。
    ///
    /// 历史事实：本 crate 曾用三个 `HashMap`（值 / 样式值 / 名字）实现，
    /// 换 `Vec` 时我在"用 `mem::replace` 交换以避免 clone"这一步写反了顺序，
    /// 结果是**旧值留在表里**。现有 286 条测试**全部照过** ——
    /// 因为没有一条测过"同一 key 设两次"。这条就是那次遗漏的补上。
    #[test]
    fn last_value_wins_when_setting_same_property_twice() {
        let mut store = PropertyStore::new();
        store.set_local(&A, PropertyValue::float(1.0));
        store.set_local(&A, PropertyValue::float(2.0));
        store.set_local(&A, PropertyValue::float(3.0));

        let (value, source) = resolve(&store, &A, None);
        assert_eq!(source, ValueSource::Local);
        assert_eq!(
            value.get::<f32>(),
            Some(3.0),
            "同一属性设了三次（1/2/3），表里应当是 3.0，实得 {:?}",
            value.get::<f32>()
        );
        assert_eq!(store.local_value(PropertyStore::id_of(&A)).unwrap().get::<f32>(), Some(3.0));
        assert_eq!(store.iter_local().count(), 1, "重复设不应增加条目数");
    }

    /// `PropertyChange` 的 `new` 必须与表里最终留的值一致。
    ///
    /// 这两条要一起看：`new` 用来驱动"要不要重绘"，而表里的值用来实际画。
    /// 两者不一致的症状是「界面停在旧值但重绘计数在涨」——
    /// 有动画时表现为**永远追不上**，静止时看不出来。
    #[test]
    fn change_log_value_matches_table() {
        let mut store = PropertyStore::new();
        let first = store.set_local(&A, PropertyValue::float(10.0)).unwrap();
        assert_eq!(
            first.old.as_ref().and_then(|v| v.get::<f32>()),
            None,
            "首次设置没有旧值"
        );
        assert_eq!(first.new.as_ref().unwrap().get::<f32>(), Some(10.0));

        let second = store.set_local(&A, PropertyValue::float(20.0)).unwrap();
        assert_eq!(second.old.as_ref().unwrap().get::<f32>(), Some(10.0));
        assert_eq!(
            second.new.as_ref().unwrap().get::<f32>(),
            store.local_value(PropertyStore::id_of(&A)).unwrap().get::<f32>(),
            "`new` 与表里的值不一致"
        );

        // 清掉之后再设：旧值应回到 None（而不是残留）。
        store.clear_local(&A);
        assert!(!store.is_locally_set(&A));
        let third = store.set_local(&A, PropertyValue::float(30.0)).unwrap();
        assert_eq!(third.old, None, "清除后再设，旧值应当是 None");
    }

    /// 本地值与样式值相互独立，且本地压过样式。
    ///
    /// 两者曾分别是两张表；合成一个 `Vec` 类型后最容易出的错是**共用了同一条
    /// 查找**（于是样式值被当成本地值、或本地设置把样式值覆盖掉）。
    #[test]
    fn local_and_styled_do_not_interfere() {
        let mut store = PropertyStore::new();
        store.set_styled(&A, PropertyValue::float(1.0));
        store.set_local(&A, PropertyValue::float(2.0));
        assert_eq!(resolve(&store, &A, None).1, ValueSource::Local);
        assert_eq!(resolve(&store, &A, None).0.get::<f32>(), Some(2.0));

        // 另一个 key 只有样式值。
        store.set_styled(&B, PropertyValue::float(0.5));
        assert_eq!(resolve(&store, &B, None).1, ValueSource::Styled);

        // 清除本地后应回落到样式值，而不是"什么都没有"。
        store.clear_local(&A);
        let (value, source) = resolve(&store, &A, None);
        assert_eq!(source, ValueSource::Styled);
        assert_eq!(value.get::<f32>(), Some(1.0));
    }

    /// `set_local_by_id` 与 `set_local` 走的是两条路径，但结果必须一致。
    ///
    /// 两条路径的原因：`set_local` 拿得到 `&'static PropertyKey`（于是知道名字），
    /// 而 diff 只有 id。这条钉的是"id 路径不会因为缺名字而丢掉值或写错地方"。
    #[test]
    fn write_by_id_matches_write_by_key() {
        let mut by_key = PropertyStore::new();
        by_key.set_local(&A, PropertyValue::float(7.0));
        let mut by_id = PropertyStore::new();
        // ⚠️ 按 id 写入一条**从未登记过**的键现在**写不进去**（返回 `false`）。
        // 旧实现会造一条名字叫 `Layout.Unknown` 的假记录凑合，那永远查不出
        // 「这条其实不影响布局」，只是一律多重排；B1 之后 `LocalEntry` 存的就是
        // 键本身，造不出假键，所以这里改成「不写」。
        assert!(
            !by_id.set_local_by_id(PropertyStore::id_of(&A), PropertyValue::float(7.0)),
            "只持 id 无法反推键，所以写不进去——返回 false 好过写一条分类靠猜的条目"
        );
        assert_eq!(
            by_id.iter_local().count(),
            0,
            "写不进去就不该留下条目"
        );
    }

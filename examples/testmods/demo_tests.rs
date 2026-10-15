// 本文件由 demo.rs 的 #[cfg(test)] mod tests 抽出 —— 测试代码与生产代码分离。
// ⚠ 放在 examples/testmods/ 而不是 examples/tests/：后者是 cargo 的集成测试目录，
// 放在那里会被**另编译一次**成独立测试 crate，而本文件里的 use super::* 只在
// 「作为 demo 二进制的子模块」时成立 —— 那次编译必然报
use super::*;

fn demo_with(pressed: bool, name: &'static str) -> Demo {
    Demo {
        hovered: None,
        pressed: pressed.then(|| ElementKey::from(name)),
    }
}

/// **源码里不许有样式** —— 这条就是那把锁。
///
/// 迁移前 `button()` 里写着：
/// ```ignore
/// .with(&props::BORDER_COLOR, Resource(if pressed { FOCUS_RING } else { STROKE }.into()))
/// .with(&props::BORDER_WIDTH, float(if pressed { 2.0 } else { 1.0 }))
/// ```
/// 那是**最后一个**让「改样式表改不动按钮」的洞：换主题改不了它，
/// 改 `controls.zstyle` 也改不了它。
///
/// 现在这两个属性只能来自样式表。这条测试不看样式表（那是
/// `controls_zstyle.rs` 的职责），只看源码这边**有没有偷偷写回去**。
#[test]
fn button_source_writes_no_appearance_at_all() {
    for pressed in [false, true] {
        let el = button("menu", &demo_with(pressed, "menu"));
        for key in [&props::BORDER_COLOR, &props::BORDER_WIDTH] {
            assert_eq!(
                el.props().lookup(key),
                None,
                "{}(pressed={pressed}) must not write {} -- appearance comes from the stylesheet",
                key.name(),
                key.name()
            );
        }
    }
}

/// 状态靠**分类名**表达，而不是靠给属性塞值。
///
/// 这是整个状态机制的形状：源码只说「我现在是什么状态」，
/// 「这个状态长什么样」由样式表的 `Control.pressed { }` 说。
#[test]
fn the_state_reaches_the_element_as_a_class_name() {
    let pressed = button("menu", &demo_with(true, "menu"));
    assert_eq!(
        pressed.style_names(),
        &["pressed"],
        "pressing must add the pressed class"
    );

    let idle = button("menu", &demo_with(false, "menu"));
    assert!(
        idle.style_names().is_empty(),
        "no state must mean no class -- otherwise every button looks pressed"
    );
}

/// 按下的**是当前这个按钮**，不是所有按钮。
///
/// `Demo::pressed` 存的是 key，而 `button()` 拿自己的 key 去比。
/// 这条钉住「按钮认得住自己」：写成常量 true 的话，会出现
/// 「按一个、全亮」——那种错从截图上很难定位。
#[test]
fn only_the_pressed_button_gets_the_class() {
    let demo = demo_with(true, "gear");
    assert_eq!(button("gear", &demo).style_names(), &["pressed"]);
    assert!(
        button("menu", &demo).style_names().is_empty(),
        "another button must stay idle"
    );
}
/// 按下的**是当前这个按钮**，不是所有按钮。
// ---------------------------------------------------------------------
//  整棵树的门禁
// ---------------------------------------------------------------------
use zephyra_core::node::Node;

/// **源码里不许有样式** —— 全树版本，这是那把锁的本体。
///
/// # 为什么比「逐个函数查」强
///
/// 逐函数查只能查到「我记得去查的那个函数」。整棵树遍历不依赖记忆：
/// 以后谁在任何一个视图函数里写下 `.with(&props::BACKGROUND, ...)`，
/// 这条都会红，而不必有人想起要往清单里加一项。
///
/// # 判据是 `iter_local()`，不是 `lookup()`
///
/// `lookup()` 会一路回落到默认值，看不出这一层有没有被写过。
/// `iter_local()` 只列**这一帧源码真的写进 Local 层**的条目 ——
/// 那正是「源码里的样式」的定义。
///
/// ⚠️ `PropertyKey` 没有实现 `PartialEq`，所以按**ptr_of**比。
/// 这是安全的：`props.rs` 里全是 `&'static PropertyKey`，同一个键在全进程内
/// 只有一个实例，地址即身份。
///
/// ⚠️ 遍历结果必须先确认树不是空的：空树上这条恒真，那正是本项目
/// 反复吃过两次的亏（「全绿但恒真」）。
#[test]
fn the_whole_demo_tree_writes_no_appearance_in_source() {
    const APPEARANCE: &[&zephyra_core::property::PropertyKey] = &[
        &props::WIDTH,
        &props::HEIGHT,
        &props::CORNER_RADIUS,
        &props::PADDING,
        &props::SPACING,
        &props::DIRECTION,
        &props::CROSS_ALIGN,
        &props::MAIN_ALIGN,
        &props::BACKGROUND,
        &props::BORDER_COLOR,
        &props::BORDER_WIDTH,
        &props::TEXT_SIZE,
        &props::TEXT_COLOR,
        &props::TEXT_LINE_HEIGHT,
        &props::TEXT_MAX_WIDTH,
        &props::ICON_COLOR,
        &props::IMAGE_SIZE,
        &props::IMAGE_TINT,
        &props::OPACITY,
    ];

    let theme = zephyra_core::theme::Theme::dark();
    let ctx = zephyra_core::context::Context::new(&theme);
    // 四种状态组合各跑一遍：hover / pressed 会改变分类名，
    // 而「某些外观只在某个状态下才泄漏」恰恰是逐函数查最容易漏的形状。
    let mut seen_nodes = 0;
    for (hovered, pressed) in [(false, false), (true, false), (false, true), (true, true)] {
        let demo = Demo {
            hovered: hovered.then(|| ElementKey::from("menu")),
            pressed: pressed.then(|| ElementKey::from("menu")),
        };
        let node = Node::from_element(&demo.view(&ctx));
        let mut offenders: Vec<String> = Vec::new();
        walk(&node, &mut |n: &Node| {
            seen_nodes += 1;
            for (key, _) in n.props().iter_local() {
                if APPEARANCE.iter().any(|k| std::ptr::eq(*k, key)) {
                    offenders.push(format!(
                        "{} / {}",
                        n.key().map(|k| k.to_string()).unwrap_or_else(|| "?".into()),
                        key.name()
                    ));
                }
            }
        });
        assert!(
            offenders.is_empty(),
            "源码把这些外观写进了 Local 层（hovered={hovered}, pressed={pressed}）：\n  {}",
            offenders.join("\n  ")
        );
    }
    assert!(
        seen_nodes > 20,
        "树遍历得太少（{seen_nodes} 个节点）—— 这条测试可能恒真，先查 walk()"
    );
}

/// 一个换行。写在 CSS 拼接处，避免在字符串里嵌 \n 时被 PowerShell 转义弄坏。
fn char10() -> char {
    char::from(10)
}

/// 递归遍历：父先于子。
fn walk(node: &Node, f: &mut impl FnMut(&Node)) {
    f(node);
    for c in node.children() {
        walk(c, f);
    }
}

/// 源码发出的每个分类名，样式表里必须有对应规则。
///
/// # 这条在防什么
///
/// `.style("tollbar")` 拼错时**什么都不发生**：不报错、不崩、画面只是
/// 回到默认值。而「回到默认值」看起来就像「样式表没生效」，
/// 于是人去查样式表 —— 而样式表是对的。
///
/// 这是整条迁移路上最隐蔽的失败模式，所以单独钉一条。
#[test]
fn every_class_name_the_source_emits_exists_in_the_stylesheet() {
    let mut css = String::new();
    for name in ["controls.zstyle", "demo.zstyle"] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(name);
        css.push_str(&std::fs::read_to_string(&path).expect("stylesheet must exist"));
        css.push('\n');
    }
    let parsed = zstylec::parse("all.zstyle", &css);
    assert!(
        parsed.is_ok(),
        "stylesheets must parse: {:?}",
        parsed.errors
    );

    // 样式表里出现过的全部分类名。
    let known: Vec<&str> = parsed
        .sheet
        .rules()
        .iter()
        .flat_map(|r| r.selector.names.iter().copied())
        .collect();

    let theme = zephyra_core::theme::Theme::dark();
    let ctx = zephyra_core::context::Context::new(&theme);
    let node = Node::from_element(&Demo::default().view(&ctx));

    let mut missing: Vec<String> = Vec::new();
    let mut emitted = 0;
    walk(&node, &mut |n: &Node| {
        for name in n.style_names() {
            emitted += 1;
            if !known.contains(name) {
                missing.push(format!(
                    "{} : .{}",
                    n.key().map(|k| k.to_string()).unwrap_or_else(|| "?".into()),
                    name
                ));
            }
        }
    });
    assert!(
        emitted > 15,
        "只发出 {emitted} 个分类名 —— 树遍历可能没走完，这条测试有恒真的风险"
    );
    assert!(
        missing.is_empty(),
        "这些分类名在样式表里没有对应规则（拼错就是静默失效）：\n  {}",
        missing.join("\n  ")
    );
}
/// 每个元素都必须**能被样式表选中**：有分类名，或其类型有裸规则。
///
/// # 为什么单独一条
///
/// 「少了 `.style("status")`」这种删除是最安静的失败：树的 Local 层仍然
/// 干净（上一条依然绿），分类名也都存在（上一条也绿），但那个元素
/// **谁都选不中**，于是它的外观全部退回属性默认值 —— 没有背景、没有内边距。
/// 从代码审查上看看不出任何异常。
///
/// # 为什么不是「每个元素都要有分类名」
///
/// 工具按钮就不带分类名 —— 它们由 `controls.zstyle` 的裸类型选择器
/// `Control { }` 命中。写成「必须有分类名」会误杀那条正确用法。
/// 所以判据是「能被选中」，而选中的两条路（分类名、裸类型规则）都要算。
#[test]
fn every_element_is_reachable_by_the_stylesheet() {
    let mut css = String::new();
    for name in ["controls.zstyle", "demo.zstyle"] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(name);
        css.push_str(&std::fs::read_to_string(&path).expect("stylesheet must exist"));
        css.push(char10());
    }
    let parsed = zstylec::parse("all.zstyle", &css);
    assert!(
        parsed.is_ok(),
        "stylesheets must parse: {:?}",
        parsed.errors
    );

    // 有裸类型规则的类型 —— 这些类型的元素不带分类名也能被选中。
    let bare: Vec<NodeTag> = parsed
        .sheet
        .rules()
        .iter()
        .filter(|r| r.selector.names.is_empty())
        .filter_map(|r| r.selector.tag)
        .collect();

    let theme = zephyra_core::theme::Theme::dark();
    let ctx = zephyra_core::context::Context::new(&theme);
    let node = Node::from_element(&Demo::default().view(&ctx));

    let mut unreachable: Vec<String> = Vec::new();
    let mut total = 0;
    walk(&node, &mut |n: &Node| {
        total += 1;
        if n.style_names().is_empty() && !bare.contains(&n.tag()) {
            unreachable.push(format!(
                "{}",
                n.key().map(|k| k.to_string()).unwrap_or_else(|| "?".into())
            ));
        }
    });
    assert!(
        total > 20,
        "树遍历得太少（{total} 个节点）—— 这条测试可能恒真，先查 walk()"
    );
    assert!(
        unreachable.is_empty(),
        "这些元素既没分类名、其类型也没有裸规则，样式表选不中它们：\n  {}",
        unreachable.join("\n  ")
    );
}

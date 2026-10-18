//! 树级门禁。全部按**整棵树**判，不按函数判 —— 逐函数查只能查到
//! 「我记得去查的那个」。（与 `examples/demo.rs` 的 6 条同一理由。）
//!
//! # 恒真防御
//!
//! 每条涉及遍历的断言都先确认「树不是空的」（节点数下界），否则树造失败时
//! 断言会**恒真**地绿 —— 这个项目已经因为恒真式吃过两次亏。

use zephyra_core::context::Context;
use zephyra_core::core::Rect;
use zephyra_core::id::NodeTag;
use zephyra_core::node::Node;
use zephyra_core::theme::{Theme, ThemeVariant};
use zephyra_fx::View;
use zephyra_style::ir::StyleSheet;

use zephyra_gallery::app::pages::PAGES;
use zephyra_gallery::app::{Gallery, BASE_COLUMNS, ROUTES, SETTINGS_ROUTE};
use zephyra_ui::tokens::ALL as TOKENS;

// ---------------------------------------------------------------- 公共帮手

/// 合并全部样式表 —— **直接用 [`zephyra_gallery::styles::all`]**，
/// 与 `bin/main.rs::build_sheet` 同一份输入。
///
/// # 为什么不再 `read_dir`
///
/// 旧版本扫 crate 目录。控件层拆出去之后那条路有两个问题：一是它只能看见
/// gallery 自己的表，看不见控件层的；二是「图对、门禁绿」要求门禁吃**同一份**
/// 表，而现在那份表是 `include_str!` 内嵌的 —— 门禁再去磁盘读一遍就成了
/// **两次真相**：改了 `styles.rs` 忘了重编，门禁照样绿。
///
/// 走同一个函数就没有这个问题：内嵌的源是唯一定义处。
fn merged_sheet() -> StyleSheet {
    let mut sheet = StyleSheet::new();
    for (name, css) in zephyra_gallery::styles::all() {
        let parsed = zstylec::parse(name, css);
        assert!(
            parsed.is_ok(),
            "{name}.zstyle must parse: {:?}",
            parsed.errors
        );
        sheet = sheet.merge(parsed.sheet);
    }
    sheet
}

/// 造一棵**应用了样式表并布局过**的树。
fn laid_out(route: &'static str) -> Node {
    let theme = Theme::builtin(ThemeVariant::Dark);
    let ctx = Context::new(&theme);
    let mut g = Gallery::default();
    g.route = route;
    let element = g.view(&ctx);
    let mut root = Node::from_element(&element);
    merged_sheet().apply_all(&mut root);
    zephyra_core::layout::layout(&mut root, Rect::new(0.0, 0.0, 1400.0, 800.0), ctx);
    root
}

/// 覆盖「会改外观的状态」的门禁脚本表 —— `interaction_leaves_no_stale_appearance`
/// 与 `every_stylesheet_rule_matches_something` 共用。
///
/// # 为什么要提成两处共用
///
/// 后者要回答「样式表里每条规则都选得中东西吗」，而「有东西可被选中」依赖
/// **交互态**：`.pressed` 规则只有在按钮被按住时才选得中元素。若只跑默认状态的
/// 树，它会把所有 `.pressed` / `.checked` 规则误报成死规则 —— 于是唯一的选择
/// 就是「把死规则检查关掉」，而那正是这条检查要防的事。
///
/// ⚠ 往表里加脚本时同步问一句：新状态会不会带出新的 `.class` 规则？会的话
/// 不加进这里就会出现假阳性，然后有人会去**删规则**而不是加脚本 —— 那比没有
/// 这条门禁更坏。
type GateScripts = Vec<(&'static str, fn(&mut Gallery))>;

/// 见 [`GateScripts`]。
fn interaction_scripts() -> GateScripts {
    vec![
        ("切到 basics", |g| g.route = "basics"),
        ("勾上 alpha", |g| g.checked[0] = true),
        ("再勾 beta 与 gamma", |g| {
            g.checked[1] = true;
            g.checked[2] = true;
        }),
        ("全部取消", |g| g.checked = [false, false, false]),
        ("选中第三个单选项", |g| g.radio = 2),
        ("选中第一个单选项", |g| g.radio = 0),
        ("开主开关", |g| g.switch_on = true),
        ("关掉主开关", |g| g.switch_on = false),
        ("切到 list 分段", |g| g.seg = "seg:list"),
        ("切回 grid 分段", |g| g.seg = "seg:grid"),
        ("展开 expander", |g| g.expanded = true),
        ("收起 expander", |g| g.expanded = false),
        ("ComboBox 前进一项", |g| g.combo = (g.combo + 1) % 4),
        ("ComboBox 前进第二项", |g| g.combo = (g.combo + 1) % 4),
        ("SpinBox 加一", |g| g.spin += 1),
        ("SpinBox 减一", |g| g.spin -= 1),
        ("打开菜单", |g| g.menu_open = Some(1)),
        ("关掉菜单", |g| g.menu_open = None),
        ("展开下拉", |g| g.dd_open = true),
        ("收起下拉", |g| g.dd_open = false),
        ("切到 collections", |g| g.route = "collections"),
        ("切到 settings", |g| g.route = "settings"),
        ("切到 toggles", |g| g.route = "toggles"),
        ("切到 effects", |g| g.route = "effects"),
        ("切回 home", |g| g.route = "home"),
        ("折射参数加一档", |g| {
            g.glass.nudge(zephyra_gallery::app::glass::Param::Ior, 1);
        }),
        ("折射参数减一档", |g| {
            g.glass
                .nudge(zephyra_gallery::app::glass::Param::Dispersion, -1);
        }),
    ]
}

/// 递归遍历：父先于子。
fn walk(node: &Node, f: &mut impl FnMut(&Node)) {
    f(node);
    for c in node.children() {
        walk(c, f);
    }
}

/// 数一幌之：确认树不是空的。
fn count_nodes(root: &Node) -> usize {
    let mut n = 0;
    walk(root, &mut |_| n += 1);
    n
}

/// 按 key 找节点（深度优先，第一个命中）。
fn find_by_key<'a>(root: &'a Node, key: &str) -> Option<&'a Node> {
    if root.key().and_then(|k| k.as_name()) == Some(key) {
        return Some(root);
    }
    for c in root.children() {
        if let Some(n) = find_by_key(c, key) {
            return Some(n);
        }
    }
    None
}

/// 内嵌的 `shell.zstyle` 源（不是磁盘）。
///
/// # 为什么必须有这个 helper 而不是直接 include_str!
///
/// 那两条门禁（`shell_metrics_match_stylesheet` / `root_container_writes_no_fixed_size`）
/// 要在**源码文本**里按行找 `height:` / `width:`，而不是走 StyleSheet —— 它们查的是
/// 「这几行字还在不在」，那需要文本，不是 IR。而文本现在内嵌在
/// `zephyra_gallery::styles` 里，所以从那里取，与运行时用的是同一份。
fn shell_css() -> &'static str {
    zephyra_gallery::styles::app_only()
        .iter()
        .find(|(n, _)| *n == "shell")
        .map(|(_, src)| *src)
        .expect("shell.zstyle 应当在内嵌样式表里")
}

// ------------------------------------------------------- 门禁 1：源码无外观

/// 源码里不许有样式 —— 全树 × 全部页面 × 全部交互态。
///
/// 判据是 `iter_local()`（Local 层）而不是 `lookup()`：后者会一路回落到
/// 样式值，看不出这一层有没有被源码写过。与 demo 同款。
#[test]
fn the_whole_tree_writes_no_appearance_in_source() {
    const APPEARANCE: &[&zephyra_core::property::PropertyKey] = &[
        &zephyra_core::props::WIDTH,
        &zephyra_core::props::HEIGHT,
        &zephyra_core::props::CORNER_RADIUS,
        &zephyra_core::props::PADDING,
        &zephyra_core::props::SPACING,
        &zephyra_core::props::DIRECTION,
        &zephyra_core::props::CROSS_ALIGN,
        &zephyra_core::props::MAIN_ALIGN,
        &zephyra_core::props::BACKGROUND,
        &zephyra_core::props::BORDER_COLOR,
        &zephyra_core::props::BORDER_WIDTH,
        &zephyra_core::props::TEXT_SIZE,
        &zephyra_core::props::TEXT_COLOR,
        &zephyra_core::props::TEXT_LINE_HEIGHT,
        &zephyra_core::props::TEXT_MAX_WIDTH,
        &zephyra_core::props::ICON_COLOR,
        &zephyra_core::props::IMAGE_SIZE,
        &zephyra_core::props::IMAGE_TINT,
        &zephyra_core::props::OPACITY,
    ];

    let routes: Vec<&str> = ROUTES
        .iter()
        .map(|(id, _, _)| *id)
        .chain(std::iter::once(SETTINGS_ROUTE.0))
        .collect();
    let mut seen_nodes = 0;
    for route in routes {
        // hover / pressed 各造一次：某些外观只在某个状态下才泄漏。
        for (hovered, pressed) in [(false, false), (true, false), (false, true), (true, true)] {
            let mut g = Gallery::default();
            g.route = route;
            if hovered {
                g.hovered = Some(zephyra_core::id::ElementKey::from("home"));
            }
            if pressed {
                g.pressed = Some(zephyra_core::id::ElementKey::from("home"));
            }
            let theme = Theme::builtin(ThemeVariant::Dark);
            let ctx = Context::new(&theme);
            let element = g.view(&ctx);
            let mut root = Node::from_element(&element);
            merged_sheet().apply_all(&mut root);
            let mut offenders: Vec<String> = Vec::new();
            walk(&root, &mut |n: &Node| {
                seen_nodes += 1;
                for (key, _) in n.props().iter_local() {
                    if APPEARANCE.iter().any(|k| std::ptr::eq(*k, key)) {
                        // 唯一有形状的例外：`fill:*` 的填充尺寸是**内容**（见 lib.rs 第 3 条）。
                        // 横条（进度条/滑条/滚动 thumb）用 WIDTH 或 HEIGHT，都放行；
                        // 其余键（颜色/圆角/边…）一概不放行。
                        let is_fill = n
                            .key()
                            .and_then(|k| k.as_name())
                            .is_some_and(|k| k.starts_with("fill:"));
                        let is_fill_geometry = is_fill
                            && (std::ptr::eq(key, &zephyra_core::props::WIDTH)
                                || std::ptr::eq(key, &zephyra_core::props::HEIGHT));
                        if !is_fill_geometry {
                            offenders.push(format!(
                                "{} / {}",
                                n.key().map(|k| k.to_string()).unwrap_or_else(|| "?".into()),
                                key.name()
                            ));
                        }
                    }
                }
            });
            assert!(
                offenders.is_empty(),
                "源码把外观写进了 Local 层（route={route}, hovered={hovered}, pressed={pressed}）：\n  {}",
                offenders.join("\n  ")
            );
        }
    }
    assert!(
        seen_nodes > 200,
        "树遍历得太少（{seen_nodes} 个）—— 这条可能恒真，先查 walk()"
    );
}

// ------------------------------------------------------- 门禁 2：分类名存在

/// 源码发出的每个分类名，样式表里必须有规则。
///
/// 拼错分类名时**什么都不发生**（不报错、不崩、只是退回默认值），而「退回
/// 默认值」看起来就像「样式表没生效」——这是整条迁移路上最隐蔽的失败模式。
#[test]
fn every_class_name_the_source_emits_exists_in_the_stylesheet() {
    let sheet = merged_sheet();
    let known: Vec<&str> = sheet
        .rules()
        .iter()
        .flat_map(|r| r.selector.names.iter().copied())
        .collect();
    let routes: Vec<&str> = ROUTES
        .iter()
        .map(|(id, _, _)| *id)
        .chain(std::iter::once(SETTINGS_ROUTE.0))
        .collect();
    let mut emitted = 0;
    let mut missing: Vec<String> = Vec::new();
    for route in routes {
        let mut g = Gallery::default();
        g.route = route;
        let theme = Theme::builtin(ThemeVariant::Dark);
        let ctx = Context::new(&theme);
        let element = g.view(&ctx);
        let mut root = Node::from_element(&element);
        sheet.apply_all(&mut root);
        walk(&root, &mut |n: &Node| {
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
    }
    assert!(
        emitted > 100,
        "只发出 {emitted} 个分类名 —— 树遍历可能没走完，这条测试有恒真的风险"
    );
    assert!(
        missing.is_empty(),
        "这些分类名在样式表里没有对应规则（拼错就是静默失效）：\n  {}",
        missing.join("\n  ")
    );
}

// ------------------------------------------------------- 门禁 3：元素可被选中

/// 反方向：每个元素要么有分类名，要么其类型有裸规则。
///
/// 防的是「少写一个 `.style(..)`」这种最安静的删除：那一个元素谁都选不中，
/// 外观全部退回默认值，而源码侧全绿。
#[test]
fn every_element_is_reachable_by_the_stylesheet() {
    let sheet = merged_sheet();
    let bare: Vec<NodeTag> = sheet
        .rules()
        .iter()
        .filter(|r| r.selector.names.is_empty())
        .filter_map(|r| r.selector.tag)
        .collect();
    let routes: Vec<&str> = ROUTES
        .iter()
        .map(|(id, _, _)| *id)
        .chain(std::iter::once(SETTINGS_ROUTE.0))
        .collect();
    let mut total = 0;
    let mut unreachable: Vec<String> = Vec::new();
    for route in routes {
        let mut g = Gallery::default();
        g.route = route;
        let theme = Theme::builtin(ThemeVariant::Dark);
        let ctx = Context::new(&theme);
        let element = g.view(&ctx);
        let mut root = Node::from_element(&element);
        sheet.apply_all(&mut root);
        walk(&root, &mut |n: &Node| {
            total += 1;
            if n.style_names().is_empty() && !bare.contains(&n.tag()) {
                unreachable.push(format!(
                    "{} ({:?})",
                    n.key().map(|k| k.to_string()).unwrap_or_else(|| "?".into()),
                    n.tag()
                ));
            }
        });
    }
    assert!(total > 200, "树遍历得太少（{total} 个）");
    assert!(
        unreachable.is_empty(),
        "这些元素既没分类名、其类型也没有裸规则，样式表选不中它们：\n  {}",
        unreachable.join("\n  ")
    );
}

// ------------------------------------------------------- 门禁 4：令牌存在

/// 样式表里引用的每个 `@令牌` 都必须能在 `Theme::role()` 里查到。
///
/// 拼错令牌时 `resolve_resource` 返回 `None`、属性**静默**保持原值 ——
/// 这正是「样式全没了」类故障最安静的成因。按张量表逐个对，让它在
/// `cargo test` 阶段就红。
#[test]
fn every_token_the_stylesheet_uses_exists_in_theme() {
    let mut used: Vec<String> = Vec::new();
    // 遍历内嵌源（控件层 + 本应用），不是磁盘目录 —— 理由同 `merged_sheet`。
    for (name, css) in zephyra_gallery::styles::all() {
        // `@令牌` 只出现在值位上：抓所有 `@word`。
        for (i, _) in css.match_indices('@') {
            let rest = &css[i + 1..];
            let word: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
                .collect();
            if !word.is_empty() {
                used.push(format!("{name}.zstyle:{word}"));
            }
        }
    }
    assert!(
        !used.is_empty(),
        "一个 @令牌 都没用到 —— 这条测试有恒真的风险"
    );
    let theme = Theme::builtin(ThemeVariant::Dark);
    let light = Theme::builtin(ThemeVariant::Light);
    let mut missing: Vec<String> = Vec::new();
    for token in &used {
        let name = token.rsplit(':').next().unwrap();
        if theme.role(name).is_none() || light.role(name).is_none() {
            missing.push(token.clone());
        }
    }
    // 顺带钉住 tokens.rs 的名单与 core 的 role 表一致（防两份漂移）。
    for t in TOKENS {
        assert!(
            theme.role(t).is_some() && light.role(t).is_some(),
            "tokens.rs 的 {t} 在主题里查不到 —— 两份名单漂移了"
        );
    }
    assert!(
        missing.is_empty(),
        "这些 @令牌 在 Theme::role() 里不存在（拼错 = 静默退回默认值）：\n  {}",
        missing.join("\n  ")
    );
}

// ------------------------------------------------------- 门禁 5：页可达

/// 每个路由目标页都能造出一棵有规模的树。
#[test]
fn every_page_builds_a_real_tree() {
    let routes: Vec<&str> = ROUTES
        .iter()
        .map(|(id, _, _)| *id)
        .chain(std::iter::once(SETTINGS_ROUTE.0))
        .collect();
    for route in routes {
        let root = laid_out(route);
        let n = count_nodes(&root);
        assert!(
            n > 40,
            "页 {route} 只造出 {n} 个节点 —— 要么页面是空的，要么树遍历坏了"
        );
    }
}

// ------------------------------------------------------- 门禁 6：常量对齐

/// shell.rs 的外壳常量必须与 gallery.zstyle 里的值逐字相等。
///
/// 两处各写一个数的后果：窗口 1400 而卡片 1159 对不上时不会报错，
/// 只是右下缺一条 —— 那种"看起来像设计如此"的错最难查。
#[test]
fn shell_metrics_match_stylesheet() {
    let css = shell_css();
    let want = [
        (
            "Container.titlebar",
            "height",
            zephyra_gallery::app::shell::TITLEBAR_H,
        ),
        (
            "Container.nav-pane",
            "width",
            zephyra_gallery::app::shell::NAV_W,
        ),
        (
            "Container.nav-pane",
            "height",
            zephyra_gallery::app::shell::NAV_H,
        ),
        (
            "Container.nav-spacer",
            "height",
            zephyra_gallery::app::shell::NAV_SPACER,
        ),
        (
            "Container.content-card",
            "width",
            zephyra_gallery::app::shell::CONTENT_W,
        ),
    ];
    for (selector, prop, expect) in want {
        // 取该规则块里该属性的第一个值。
        let block = css
            .split(&format!("{selector} {{"))
            .nth(1)
            .unwrap_or_else(|| panic!("样式表里找不到 {selector} 规则"));
        let block = &block[..block.find('}').unwrap_or(block.len())];
        let line = block
            .lines()
            .map(|l| l.trim())
            .find(|l| l.starts_with(&format!("{prop}:")))
            .unwrap_or_else(|| panic!("{selector} 里找不到 {prop}"));
        let got: f32 = line
            .trim_start_matches(&format!("{prop}:"))
            .trim()
            .trim_end_matches("px;")
            .trim()
            .parse()
            .unwrap_or_else(|_| panic!("{selector} 的 {prop} 不是数字：{line}"));
        assert_eq!(
            got, expect,
            "{selector} 的 {prop}：样式表 {got} ≠ shell.rs 常量 {expect}"
        );
    }
}

/// 根容器**必须不写** `width` / `height`。
///
/// # 为什么这是动态布局的**前提**而不是细节
///
/// `arrange` 无条件把根矩形设成 `bounds`（= 窗口客户区），所以铺满窗口不需要写尺寸。
/// 而写上 `width: 1400px` 会让 `measure` 返回 1400——一切按内容算可用宽的分支
/// （列数、banner 高度、是否折叠导航）都会拿到这个"假窗口宽"。症状：
///
/// - 窗口拉到 1882 时，右侧留 386 单位**空白**（内容仍按 1400 排）；
/// - 窗口缩到 982 时，内容溢出被**裁**（同样按 1400 排）。
///
/// 两种表现看起来都像"渲染坏了"，而根因只是样式表里多了两行。
///
/// 判据是"写没写"，不是"写了多少"：数值对错还能调，**这两行存在**就让整个
/// 响应式失效，且没有任何报错。
#[test]
fn root_container_writes_no_fixed_size() {
    let css = shell_css();
    let start = css
        .split("Container.window {")
        .nth(1)
        .unwrap_or_else(|| panic!("样式表里找不到 Container.window"));
    let block = &start[..start.find('}').unwrap_or(start.len())];
    for prop in ["width", "height"] {
        assert!(
            !block
                .lines()
                .any(|l| l.trim().starts_with(&format!("{prop}:"))),
            "Container.window 写了 {prop} —— 根容器必须由 arrange 按 bounds 铺满，\
             写死尺寸会让整个响应式失效（窗口变化时内容仍按旧尺寸排）"
        );
    }
}

/// 基准画布（1400×800）下，根矩形必须仍铺满客户区。
///
/// # 为什么去掉固定尺寸之后还要这一条
///
/// 因为"去掉显式尺寸"有另一种可能的失败：如果 `arrange` 改成按测量结果铺，
/// 根矩形会缩到内容尺寸，于是根背景不再铺满窗口 —— 右侧和底部露出未绘制区。
/// 那条路**也不报错**，只是"窗口边缘多一圈别的东西"。
///
/// 所以这条门禁把两件事一起钉住：
/// 1. 根矩形 == 客户区（铺满）；
/// 2. 基准画布下与去掉尺寸之前**无差别**（回归保护）。
#[test]
fn root_container_still_fills_client_area_at_baseline() {
    let root = laid_out("home");
    let r = root.rect();
    let (cw, ch) = zephyra_gallery::app::BASE_VIEWPORT;
    assert_eq!(
        (r.width, r.height),
        (cw as f32, ch as f32),
        "根矩形必须铺满基准画布：{r:?}"
    );
}

// ------------------------------------------------------- 门禁 7：外壳几何

/// 真去**量布局后的矩形**：标题栏铺满画布宽、内容卡贴着导航右侧、
/// 置底的 Settings 落在画布底部附近。
///
/// core 没有 flex-grow，"铺满"靠的是固定宽高常量 —— 常量错了这里立刻红。
#[test]
fn shell_layout_geometry() {
    let root = laid_out("home");
    assert!(count_nodes(&root) > 40, "树是空的，几何门禁恒真");

    let titlebar = find_by_key(&root, "titlebar").expect("titlebar");
    let r = titlebar.rect();
    assert!(
        (r.width - 1400.0).abs() < 1.0,
        "标题栏应铺满画布宽（1400），实测 {:.1}",
        r.width
    );
    assert!(
        (r.height - 44.0).abs() < 1.0,
        "标题栏高应是 44，实测 {:.1}",
        r.height
    );

    let content = find_by_key(&root, "content").expect("content card");
    let rc = content.rect();
    assert!(
        (rc.x - 260.0).abs() < 1.0,
        "内容卡应紧贴导航右侧（x=260），实测 {:.1}",
        rc.x
    );
    assert!(
        (rc.width - 1139.0).abs() < 1.0,
        "内容卡宽应是 1139，实测 {:.1}",
        rc.width
    );
    assert!(
        (rc.height - 756.0).abs() < 1.0,
        "内容卡高应是 756，实测 {:.1}",
        rc.height
    );

    // 置底项：底部落在画布内 24px 以内。
    let settings = find_by_key(&root, "settings").expect("settings nav item");
    let rs = settings.rect();
    let bottom = rs.y + rs.height;
    assert!(
        (bottom - 788.0).abs() < 24.0,
        "Settings 应落在导航栏底部附近（约 788），实测 {bottom:.1}"
    );
}

// ------------------------------------------------------- 门禁 8：唯一例外

/// 源码里唯一允许写 `WIDTH` 的形态：key 以 `fill:` 开头（滑条/进度条填充）。
///
/// 黑名单会烂，形状不会 —— 这条钉的是**形状**，所以新增一处数据几何时
/// 必须显式命名，而不是悄悄加进一个清单里。
#[test]
fn only_fill_nodes_carry_local_width() {
    let routes: Vec<&str> = ROUTES
        .iter()
        .map(|(id, _, _)| *id)
        .chain(std::iter::once(SETTINGS_ROUTE.0))
        .collect();
    let mut bad: Vec<String> = Vec::new();
    let mut fills = 0;
    for route in routes {
        let root = laid_out(route);
        walk(&root, &mut |n: &Node| {
            let has_local_width = n
                .props()
                .iter_local()
                .any(|(k, _)| std::ptr::eq(k, &zephyra_core::props::WIDTH));
            if !has_local_width {
                return;
            }
            let is_fill = n
                .key()
                .and_then(|k| k.as_name())
                .is_some_and(|k| k.starts_with("fill:"));
            if is_fill {
                fills += 1;
            } else {
                bad.push(format!(
                    "{} ({route})",
                    n.key().map(|k| k.to_string()).unwrap_or_else(|| "?".into())
                ));
            }
        });
    }
    // basic-input 有 2 个滑条、status 有 3 个进度条、settings 有 1 个 ——
    // 少于 5 说明某页的填充没造出来，这条可能恒真。
    assert!(fills >= 5, "fill:* 只有 {fills} 个 —— 这条可能恒真");
    assert!(
        bad.is_empty(),
        "这些节点在源码里写了 WIDTH 但 key 不是 fill:*：\n  {}",
        bad.join("\n  ")
    );
}

// ------------------------------------------------------- 门禁 9：图标已登记

/// 源码里引用的每一个图标名，都必须在内置表里**登记过**。
///
/// # 为什么这条必须住在**应用**侧而不是 core 侧
///
/// core 的 `all_builtin_icons_are_non_empty`（新加）能抓「登记了但资源是空的」，抓不了
/// 「压根没登记」—— 没登记的名字根本不会出现在那张表里，而它在本仓造成过的故障
/// 是：`DISMISS` 常量与 `dismiss.svg` 资源都在，唯独 `REGISTRY` 少一行，于是
/// `IconSet::get` 回落到 `Icon::EMPTY`，界面上的图标位**是空的**。
///
/// 症状安静到无法自证：没有报错、没有告警、节点矩形还在（那格留白），
/// 只有「图上少东西」这件事 —— 而没人会怀疑自己看错。
///
/// 所以判据必须站在**引用方**：树里出现过的名字，逐个查表。
#[test]
fn every_icon_name_the_tree_emits_is_registered() {
    let known: std::collections::HashSet<&str> = zephyra_core::icon::assets::REGISTRY
        .iter()
        .map(|(n, _)| *n)
        .collect();
    // ⚠️ 查的是**登记名**，不是资源文件名：内置表刻意用「控件的词汇」
    // （关掉 = `close`，资源文件却叫 `dismiss.svg`，见 `assets::REGISTRY` 的文档）。
    // 按文件名去查会把「用词不同」误判成「没登记」，反过来也会把真故障放过去 ——
    // 本条在落地当天就抓到过一次：应用按文件名写了 `dismiss`，而登记名是
    // `close`，于是那个图标位一直是空的。
    let routes: Vec<&str> = ROUTES
        .iter()
        .map(|(id, _, _)| *id)
        .chain(std::iter::once(SETTINGS_ROUTE.0))
        .collect();
    let mut used: Vec<String> = Vec::new();
    let mut bad: Vec<String> = Vec::new();
    for route in routes {
        let root = laid_out(route);
        walk(&root, &mut |n: &Node| {
            for (key, value) in n.props().iter_local() {
                if !std::ptr::eq(key, &zephyra_core::props::ICON_NAME) {
                    continue;
                }
                let name = match value {
                    zephyra_core::property::PropertyValue::StaticStr(s) => *s,
                    zephyra_core::property::PropertyValue::Str(s) => s.as_ref(),
                    _ => continue,
                };
                used.push(name.to_string());
                if !known.contains(name) {
                    bad.push(format!(
                        "{} ({route}) -> {name}",
                        n.key().map(|k| k.to_string()).unwrap_or_else(|| "?".into())
                    ));
                }
            }
        });
    }
    // 十个路由里有 30+ 个图标位；少于 20 说明遍历没走完，这条可能恒真。
    assert!(
        used.len() >= 20,
        "只用到 {} 个图标名 —— 遍历可能没走完",
        used.len()
    );
    assert!(
        bad.is_empty(),
        "这些图标名不在内置表里（画面上会是空白图标位）：\n  {}",
        bad.join("\n  ")
    );
}

// -------------------------------------------------- 门禁 11：命中与绘制同一套几何

/// 命中测试选中的元素，必须**就是**那个像素上画着的东西。
///
/// # 为什么这条必须有
///
/// 命中与绘制读的是**同一棵树**（`HostWindow::route` 与 `render_pass` 都取
/// `cache.node()`），所以一旦它们对不上，症状就只有一种：**点击落在 A 上、
/// 却执行了 B 的动作** —— 界面上没有任何异常（该亮的亮了、该画的画了），
/// 只有"点哪儿不好使"。
///
/// 而这种偏移一旦存在，用截图核对是**核不出来的**：画面完全正确。
///
/// 判据取自实际像素坐标：`rect()` 是绘制用的绝对矩形，把点取在它的**中心**，
/// 处理目标必须是它自己。
///
/// 用 `HitResult::target()`（最深的**可交互**节点）而不是 `deepest()`（最深的
/// 任何节点）：命中链本来就包含图标、文本这些后代，它们不是处理目标 ——
/// `INTERACTIVE` 是**不可继承**属性（见 `props::INTERACTIVE`），只有控件本身#[test]
#[test]
fn hit_test_agrees_with_the_painted_rects() {
    // ⚠ 这一条必须跑**全部**路由，不是只跑 home。
    //
    // 旧版只测 home，于是「某一页内容比内容卡高、最后几行掉到窗口外面」这件事
    // 完全没被看见：那些节点确实存在于树里、也确实可交互，只是中心落在客户区
    // 之外，命中测试按定义不会命中它们 —— 画面上就是「点不到」，且不报任何错。
    //
    // core 既没有滚动也没有裁剪溢出（`docs/dev-log/README.md` 记着「core 目前
    // 无滚动」），所以「一屏装不下」唯一的表现就是**下沿被吃掉**。要留住就得
    // 逐页量。
    let theme = Theme::builtin(ThemeVariant::Dark);
    let ctx = Context::new(&theme);

    let mut wrong: Vec<String> = Vec::new();
    let mut checked = 0usize;

    for route in zephyra_gallery::app::list_routes() {
        let root = laid_out(route);
        let client = root.rect();

        let mut targets: Vec<(String, zephyra_core::core::Rect)> = Vec::new();
        walk(&root, &mut |n: &Node| {
            if n.props()
                .lookup(&zephyra_core::props::INTERACTIVE)
                .map(|(v, _)| matches!(v, zephyra_core::property::PropertyValue::Bool(true)))
                .unwrap_or(false)
            {
                targets.push((n.key().map(|k| k.to_string()).unwrap_or_default(), n.rect()));
            }
        });

        for (key, rect) in &targets {
            // 零尺寸的节点跳过：它们在树里可交互，但布局给不出面积（父容器把它
            // 压成 0 宽/0 高），于是既画不出来也点不到。判据是**面积**，不是
            // 「有没有 key」—— 留着会让这条门禁对每一个 `.close` 按钮都误报。
            if rect.width <= 0.0 || rect.height <= 0.0 {
                continue;
            }
            // 掉出客户区的先报出来 —— 那是「看得见但点不到」的一类。
            if rect.y + rect.height > client.height + 0.5
                || rect.x + rect.width > client.width + 0.5
            {
                wrong.push(format!(
                    "{route} / {key}：矩形 {:?} 掉出客户区 {:?} —— 画面上被吃掉，点不到",
                    rect, client
                ));
                continue;
            }
            // 1px 的框在中心可能恰好落在边界外，取中心即可（矩形高宽都 >= 1）。
            let point = zephyra_core::core::Point::new(
                rect.x + rect.width * 0.5,
                rect.y + rect.height * 0.5,
            );
            let mut hit = zephyra_core::hit::HitResult::default();
            checked += 1;
            if !zephyra_core::hit::hit_test(
                &root,
                zephyra_core::hit::HitTestRequest {
                    point,
                    ignore_above: None,
                },
                ctx,
                &mut hit,
            ) {
                wrong.push(format!("{route} / {key}：它的中心 {point:?} 谁都没命中"));
                continue;
            }
            let Some(target) = hit.target() else {
                wrong.push(format!(
                    "{route} / {key}：中心 {point:?} 命中了某个**不可交互**的节点 —— \\
                     说明它被压在别人下面（谁盖住了它？）"
                ));
                continue;
            };
            // 目标可以是自己，**也可以是自己的后代**。
            //
            // 组合控件就是这样：`SpinBox` 本体是 INTERACTIVE（这样整块都能点），
            // 而它的 `spin:up` / `spin:down` 两个按钮叠在上面。所以 SpinBox
            // 的矩形中心落在「减号」上时，处理目标是 `spin:down` —— 那是**正确**的，
            // 因为命中链取的是最深的可交互节点。
            //
            // 只允许「自己或后代」是有意的：祖先不是。如果处理目标是祖先，
            // 说明子控件被什么东西盖住了，而盖住它的东西不在自己的子树里。
            // 目标可以是自己，**也可以是自己的后代**。
            //
            // 组合控件就是这样：`SpinBox` 本体是 INTERACTIVE（这样整块都能点），
            // 而它的 `spin:up` / `spin:down` 两个按钮叠在上面。所以 SpinBox 的
            // 矩形中心落在「减号」上时，处理目标是 `spin:down` —— 那是**正确**的，
            // 因为命中链取的是最深的可交互节点。
            //
            // 所以判据不能是「id 相等」，而要问一句更朴素的话：**这个点上有没有
            // 属于它自己子树的可交互节点？** 没有，就说明它被子树外的东西盖住了
            // （盖住它的不是祖先，因为它在自己子树里没有命中）—— 那才是真的错。
            let mut covered_by_own_subtree = false;
            walk(&root, &mut |n: &Node| {
                if !covered_by_own_subtree
                    && n.key().and_then(|k| k.as_name()) == Some(key.as_str())
                {
                    covered_by_own_subtree = deepest_interactive_in(n, point).is_some();
                }
            });
            if !covered_by_own_subtree {
                wrong.push(format!(
                    "{route} / {key}：中心 {point:?} 的处理目标是 {}（矩形 {:?}）—— \
                     而它自己子树里没有任何可交互节点在那个点上",
                    target.id, target.rect
                ));
            }
        }
    }

    assert!(
        checked >= 150,
        "只量了 {checked} 个可交互节点 —— 遍历可能没走完，这条有恒真的风险"
    );
    assert!(
        wrong.is_empty(),
        "命中测试与绘制几何对不上（点 A 执行 B / 掉出客户区）：\n  {}",
        wrong.join("\n  ")
    );
}

// -------------------------------------------------- 门禁 10：交互不留残迹

/// 一条**交互序列**：保留式树跑过它之后，必须与「同一状态下冷启动的树」完全一致。
///
/// # 判据为什么是「与冷启动比」而不是「与上一帧比」
///
/// 交互 bug 的形状是**状态残留**：点一下之后，上一个状态的样式值赖在新树上。
/// 「和上一帧比」会把残留当成正确（上一帧确实就是那个值）。
/// 而「同一状态冷启动」是**无历史的正确答案** —— 保留式路径与它逐节点相等，
/// 就是没有残留；不等，就是有。
///
/// # 这条为什么必须有
///
/// `--dump-cpu` 只渲一帧（无输入），所以**任何交互路径的缺陷它都看不见**：
/// 单帧图永远是对的。缺陷只在「连着点几下」之后才出现，而那种缺陷恰好是
/// 最伤的一类 —— 用户看到的是「按钮点了没反应 / 点了以后别的按钮花了」。
#[test]
fn interaction_leaves_no_stale_appearance() {
    use zephyra_core::frame::FrameCache;
    use zephyra_style::SheetStyler;

    const VIEWPORT: (f32, f32) = (1400.0, 800.0);
    let theme = Theme::builtin(ThemeVariant::Dark);
    let ctx = Context::new(&theme);
    let sheet = std::sync::Arc::new(merged_sheet());
    let styler = || Box::new(SheetStyler(sheet.clone())) as Box<dyn zephyra_core::styler::Styler>;

    // 脚本覆盖四类会改外观的状态：路由、勾选、开关、折叠、选中、悬停/按下。

    // 一条门禁脚本：改哪儿（名字） + 怎么改（闭包）+ 期望（由闭包自己断言）。
    let script: GateScripts = interaction_scripts();

    let mut g = Gallery::default();
    let mut warm = FrameCache::new();
    warm.set_styler(styler());
    warm.frame(&g.view(&ctx), ctx, VIEWPORT);

    let mut step_no = 0;
    for (name, step) in script {
        step_no += 1;
        step(&mut g);
        warm.frame(&g.view(&ctx), ctx, VIEWPORT);

        let mut cold = FrameCache::new();
        cold.set_styler(styler());
        cold.frame(&g.view(&ctx), ctx, VIEWPORT);

        let (Some(w), Some(c)) = (warm.node(), cold.node()) else {
            panic!("第 {step_no} 步（{name}）之后树建不起来");
        };
        let diffs = compare_trees(w, c, &mut String::new());
        assert!(
            diffs.is_empty(),
            "第 {step_no} 步「{name}」之后，保留式树与冷启动树不一致：\n  {}\n\
             （这意味着上一状态的外观值赖在新树上 —— 点击后界面花掉就是它）",
            diffs.join("\n  ")
        );
    }
}

/// `node` 的子树里，**最深**的、矩形包含 `point` 的可交互节点。
///
/// # 为什么不能直接比 id
///
/// `ElementKey` 可以带序号后缀，而控件内部的叶子节点大量复用同一个名字
/// （每个按钮的图标节点都叫 `icon`）。比 id 在嵌套同名时会给出假阳性 ——
///
/// # 为什么只需要「有没有」，不需要「是哪一个」
///
/// 门禁要回答的是「这个可交互节点被点时会不会落到它**自己人**手里」。
/// 组合控件（SpinBox / Segmented / Card-with-buttons）里，子控件叠在父控件
/// 上方是**设计如此**，命中落到子控件上是正确的。真正要抓的是相反的情况：
/// 子树里一个都没有，命中却落到了外面某个不相干的节点上 —— 那才是「点 A 执行 B」。
fn deepest_interactive_in(node: &Node, point: zephyra_core::core::Point) -> Option<&Node> {
    fn contains(r: zephyra_core::core::Rect, p: zephyra_core::core::Point) -> bool {
        p.x >= r.x && p.x <= r.x + r.width && p.y >= r.y && p.y <= r.y + r.height
    }
    fn interactive(n: &Node) -> bool {
        n.props()
            .lookup(&zephyra_core::props::INTERACTIVE)
            .map(|(v, _)| matches!(v, zephyra_core::property::PropertyValue::Bool(true)))
            .unwrap_or(false)
    }

    let mut best = (interactive(node) && contains(node.rect(), point)).then_some(node);
    for c in node.children() {
        if let Some(d) = deepest_interactive_in(c, point) {
            // 深度优先、后来者覆盖 —— 子树里越深的越「新」。
            best = Some(d);
        }
    }
    best
}

/// 逐节点逐属性对比两棵树，返回差异描述（空 = 完全一致）。
///
/// 对比的键是**视觉上看得见的那批**：颜色、圆角、边、字号、尺寸、间距、对齐、
/// 折叠、文字与图标名 —— 任何一条不一致都会在画面上留下痕迹。
fn compare_trees(warm: &Node, cold: &Node, path: &mut String) -> Vec<String> {
    const KEYS: &[&zephyra_core::property::PropertyKey] = &[
        &zephyra_core::props::BACKGROUND,
        &zephyra_core::props::BORDER_COLOR,
        &zephyra_core::props::BORDER_WIDTH,
        &zephyra_core::props::CORNER_RADIUS,
        &zephyra_core::props::TEXT_COLOR,
        &zephyra_core::props::TEXT_SIZE,
        &zephyra_core::props::ICON_COLOR,
        &zephyra_core::props::ICON_NAME,
        &zephyra_core::props::TEXT,
        &zephyra_core::props::WIDTH,
        &zephyra_core::props::HEIGHT,
        &zephyra_core::props::PADDING,
        &zephyra_core::props::SPACING,
        &zephyra_core::props::DIRECTION,
        &zephyra_core::props::MAIN_ALIGN,
        &zephyra_core::props::CROSS_ALIGN,
        &zephyra_core::props::COLLAPSED,
        &zephyra_core::props::OPACITY,
    ];
    let mut out = Vec::new();
    if warm.style_names() != cold.style_names() {
        out.push(format!(
            "{path}分类名不同：{:?} vs {:?}",
            warm.style_names(),
            cold.style_names()
        ));
    }
    for key in KEYS {
        let a = warm.props().lookup(key);
        let b = cold.props().lookup(key);
        if a != b {
            out.push(format!("{path}{}: {:?} vs {:?}", key.name(), a, b));
        }
    }
    if (warm.rect().x - cold.rect().x).abs() > 0.01
        || (warm.rect().y - cold.rect().y).abs() > 0.01
        || (warm.rect().width - cold.rect().width).abs() > 0.01
        || (warm.rect().height - cold.rect().height).abs() > 0.01
    {
        out.push(format!(
            "{path}矩形不同：{:?} vs {:?}",
            warm.rect(),
            cold.rect()
        ));
    }
    if warm.children().len() != cold.children().len() {
        out.push(format!(
            "{path}子节点数不同：{} vs {}",
            warm.children().len(),
            cold.children().len()
        ));
        return out;
    }
    for (i, (wc, cc)) in warm.children().iter().zip(cold.children()).enumerate() {
        let label = wc
            .key()
            .map(|k| k.to_string())
            .unwrap_or_else(|| format!("#{i}"));
        let saved = path.len();
        path.push_str(&format!("{label}/"));
        out.extend(compare_trees(wc, cc, path));
        path.truncate(saved);
    }
    out
}

// -------------------------------------------------- 门禁 13：Mica 相关的边界

/// 半透明标题栏令牌必须真的半透明 —— 否则 `backdrop=true` 也不会有可见效果。
///
/// # 为什么这条重要
///
/// Mica 材质画在窗口**背后**，只有在"应用没把那块涂满不透明色"时才透得出。
/// 所以「材质是否可见」不取决于平台层，而取决于这个令牌的 alpha。
/// 若它被改成不透明，`ApplyOutcome.backdrop == true` 照样成立，而画面上
/// **完全看不出差别** —— 那种缺陷没有任何测试能自动发现（`--dump-cpu` 也没有
/// 背后材质），只能靠这条门禁。
#[test]
fn the_mica_titlebar_token_is_actually_translucent() {
    let dark = Theme::builtin(ThemeVariant::Dark);
    let light = Theme::builtin(ThemeVariant::Light);

    for (name, t) in [("dark", &dark), ("light", &light)] {
        let solid = t
            .role(zephyra_core::theme::roles::SURFACE_ALT)
            .expect("surface_alt 角色必须存在");
        let translucent = t
            .role(zephyra_core::theme::roles::SURFACE_ALT_TRANSLUCENT)
            .expect("surface_alt_translucent 角色必须存在");

        assert!(
            translucent.a < 1.0,
            "{name}: 半透明令牌的 alpha 应当 < 1.0，实际 {} —— \
             等于不透明的话 Mica 永远透不出来",
            translucent.a
        );
        assert!(
            translucent.a > 0.0,
            "{name}: alpha 为 0 会让标题栏文字没有任何底色"
        );
        // 颜色本身（RGB）应与实色一致 —— 半透明只改 alpha，不该顺手换色。
        // 差一点点是浮点误差，放宽到 0.02。
        assert!(
            (translucent.r - solid.r).abs() < 0.02
                && (translucent.g - solid.g).abs() < 0.02
                && (translucent.b - solid.b).abs() < 0.02,
            "{name}: 半透明令牌的 RGB 应与 surface_alt 一致（只改 alpha），\
             实得 {:?} vs {:?}",
            translucent,
            solid
        );
    }
}

/// 应用必须**无条件**发 `.mica` 分类名 —— 平台能不能开是平台层的事。
///
/// # 判据为什么这样定
///
/// 若让应用先问「平台支持吗」再决定发不发分类名，那么「不支持」这条分支
/// 在 Windows 上永远走不到（测试环境就是 Win11），于是它**恒不被执行**。
/// 而一旦将来真有不支持的平台，忘记加 `cfg` 就会编译失败或行为分叉。
///
/// 现在的形态是「无条件表达意图 + 平台层回答能力 + 样式表保证降级可读」，
/// 三个环节都可在各自平台上测试。
#[test]
fn the_app_always_asks_for_the_mica_look() {
    let theme = Theme::builtin(ThemeVariant::Dark);
    let ctx = Context::new(&theme);
    let sheet = merged_sheet();
    let mut g = Gallery::default();
    assert!(g.mica, "默认就应当请求 Mica 外观");

    let mut root = laid_out("home");
    sheet.apply_all(&mut root);

    // 找标题栏节点：它的样式分类名里应当有 `mica`。
    let mut found = false;
    walk(&root, &mut |n: &Node| {
        if n.key().map(|k| k.to_string()).as_deref() == Some("titlebar") {
            found = n.style_names().contains(&"mica");
        }
    });
    assert!(found, "标题栏必须带 `mica` 分类名（Mica 是默认请求）");
    let _ = (&mut g, ctx);
}
// ---------------------------------- 门禁 12（重写后）：路由表与页面表必须一致

/// [`ROUTES`] 与 [`PAGES`] 的 id 集合必须**完全相同**。
///
/// # 为什么这两张表并存（而不是合并成一张）
///
/// 它们分工不同：`ROUTES` 给导航栏与 `resolve_route`（只需要 id / 标题 / 图标），
/// `PAGES` 还多一个构造函数，首页要遍历它拿标题与图标。
///
/// 两份表天然会漂 —— 加了一页忘了加另一份时，`resolve_route` 认得这个路由
/// （于是导航栏能点进去）却查不到构造器（于是 `pages::page` 静默回首页）。
/// 症状是**"点得进去、进去是首页"**：地址栏那一瞬变了，随后一切正常，
/// 所以肉眼与截图都发现不了。
#[test]
fn route_table_and_page_table_agree() {
    // `ROUTES` 是导航项（不含 Settings，它由 shell 置底），`PAGES` 是**全部**
    // 可构造页面（含 Settings）。所以比较的一边要补上 Settings。
    let mut from_routes: Vec<&str> = ROUTES.iter().map(|(id, _, _)| *id).collect();
    from_routes.push(SETTINGS_ROUTE.0);
    let from_pages: Vec<&str> = PAGES.iter().map(|p| p.id).collect();

    assert_eq!(
        from_routes, from_pages,
        "ROUTES+settings 与 PAGES 的 id 集合或顺序不一致 —— 两份表漂了"
    );

    // Settings 单独置底，不在 ROUTES 里 —— 重复渲染会出两项。
    assert!(
        !ROUTES.iter().any(|(id, _, _)| *id == SETTINGS_ROUTE.0),
        "Settings 不该出现在 ROUTES 里（它由 shell 单独置底）"
    );
    assert_eq!(
        zephyra_gallery::app::resolve_route(SETTINGS_ROUTE.0),
        Some(SETTINGS_ROUTE.0),
        "Settings 必须在 resolve_route 的表里，否则侧栏那一项点了没反应"
    );
}

/// 每个路由渲染出来的**必须就是它自己那一页**。
///
/// # 这条是为「静默回落首页」写的
///
/// `pages::page()` 对未知路由的兜底是 `home::page(g)` —— 不崩、不报错，
/// 画面上却有「标题写着 Settings、正文却是首页」这种一眼假的组合。
///
/// 而**已有的门禁一条都抓不到**：首页的树足够大（`every_page_builds_a_real_tree`
/// 过）、每个节点都在客户区内（命中几何过）、分类名齐全（样式门禁过）。
/// 它们量的是「页面非空」，不是「页面是对的」。
///
/// 判据走**页根节点的 key**：每个页面都用 `page_root(<route id>, "page-<id>")`
/// 起手，所以「这一页是这一页」这件事在树里有痕迹可查。
#[test]
fn every_route_renders_its_own_page() {
    for id in zephyra_gallery::app::list_routes() {
        let root = laid_out(id);
        let want = format!("page-{id}");

        // ⚠ 不能用 `find_by_key(root, id)`：侧栏的导航项与页根**同 key**
        // （"home" 既是路由 id 也是导航项 key），深度优先先撞上的一定是导航项。
        // 所以判据是「key 与分类名**同时**对得上」的那个节点。
        let mut hit: Option<Vec<&str>> = None;
        walk(&root, &mut |n: &Node| {
            if hit.is_none()
                && n.key().and_then(|k| k.as_name()) == Some(id)
                && n.style_names().contains(&want.as_str())
            {
                hit = Some(n.style_names().to_vec());
            }
        });
        assert!(
            hit.is_some(),
            "路由 {id} 的树上找不到「key == {id} 且带 .{want}」的节点 —— \
             大概率是 pages::page() 对未知路由静默回落到了首页"
        );
    }
}

/// 每个页面的 key 必须是**合法路由** —— 否则导航点它会静默停在当前页。
///
/// # 判据不走 UI，而走 `resolve_route`
///
/// 那是所有点击的必经之路（[`Gallery::click`] 的第一条分支就是它）。走真源
/// 就不用模拟 11 次点击，也不会因为命中测试的细节而给出假阴性。
#[test]
fn every_page_key_is_a_legal_route() {
    let bad: Vec<&str> = PAGES
        .iter()
        .filter(|p| zephyra_gallery::app::resolve_route(p.id).is_none())
        .map(|p| p.id)
        .collect();
    assert!(
        bad.is_empty(),
        "这些页面 id 解析不出路由（点导航会没反应）：\n  {}",
        bad.join("\n  ")
    );
}

/// 每个页面必须真的造出**有内容**的树 —— 不是「标题 + 什么都没有」。
///
/// # 为什么这条对重写后的结构更重要
///
/// 旧结构有 44 个控件页，任何一页塌成空壳都很难批量发现（要挨个点开 44 次）。
/// 现在只有 11 页，肉眼是能扫完的 —— 但门禁仍要留，因为**塌页不报错**：
/// 一个只含标题的 `Container` 完全合法，树建得出来、路由认得、截图也有东西。
///
/// 判据用**可交互节点数**而不是节点数：侧栏 11 项 + 页头两个工具在每一页都
/// 出现，所以「只有外壳」的页面仍然有 13 个可交互节点 —— 拿总数判的话每一页
/// 都会过，而那正是这条要挡的情况。
#[test]
fn every_page_carries_its_own_controls() {
    let theme = Theme::builtin(ThemeVariant::Dark);
    let ctx = Context::new(&theme);

    // 每页的外壳固定贡献 13 个可交互节点（侧栏 10 + Settings + 两个工具）。
    // 页面**自己**必须至少再贡献 1 个，否则就是「只有外壳的空页」。
    const SHELL_ONLY: usize = 13;

    let mut thin: Vec<String> = Vec::new();
    let mut total = 0usize;
    for id in zephyra_gallery::app::list_routes() {
        let mut g = Gallery::default();
        g.route = id;
        let mut root = Node::from_element(&g.view(&ctx));
        merged_sheet().apply_all(&mut root);

        let mut n = 0usize;
        walk(&root, &mut |node: &Node| {
            if node
                .props()
                .lookup(&zephyra_core::props::INTERACTIVE)
                .map(|(v, _)| matches!(v, zephyra_core::property::PropertyValue::Bool(true)))
                .unwrap_or(false)
            {
                n += 1;
            }
        });
        total += n;
        if n <= SHELL_ONLY {
            thin.push(format!("{id}: {n} 个可交互节点（外壳就占 {SHELL_ONLY}）"));
        }
    }
    assert!(
        total > SHELL_ONLY * zephyra_gallery::app::list_routes().len(),
        "总共只数到 {total} 个可交互节点 —— 遍历可能没走完，这条有恒真的风险"
    );
    assert!(
        thin.is_empty(),
        "这些页面除了外壳什么都没有：\n  {}",
        thin.join("\n  ")
    );
}

/// 每个路由 id 在**每一门语言**下都必须有译文。
///
/// # 为什么必须逐语言查，而不是查「键集合齐全」
///
/// [`Gallery::tr`] 找不到键时回落到 `route.unknown` —— 那是个**合法**的
/// `&'static str`，画面上显示的字就是 `route.unknown` 这串字符，不崩、不空。
/// 所以「en 有 zh 没有」这类漏翻，只会在切语言的那一帧被人看见一眼就过去。
#[test]
fn every_route_id_is_translated_in_every_supported_language() {
    use zephyra_core::i18n::Preferences;
    use zephyra_gallery::app::{catalog_keys, messages, SUPPORTED};

    let keys = catalog_keys();
    let msgs = messages();
    let default = SUPPORTED[0];

    let mut problems: Vec<String> = Vec::new();
    for id in ROUTES
        .iter()
        .map(|(id, _, _)| *id)
        .chain([SETTINGS_ROUTE.0])
    {
        let key = format!("route.{id}");
        // 键本身必须存在：`route_key()` 找不到 `route.<id>` 时回落到
        // `route.unknown`，而那是个**合法**的 `&'static str` —— 不崩、只显示字面量。
        if !keys.contains(&key.as_str()) {
            problems.push(format!("{id}: 文案表里连键 {key} 都没有"));
            continue;
        }
        for lang in SUPPORTED {
            let prefs = Preferences::from_list(&[lang]);
            if msgs.select(&prefs, default, &key).is_none() {
                problems.push(format!("{id} / {lang}"));
            }
        }
    }
    assert!(
        problems.is_empty(),
        "这些路由在对应语言下查不到译文（画面上显示回落文案或键名）：\n  {}",
        problems.join("\n  ")
    );
}

/// 分段控件的段 id 与 [`Gallery::click`] 的分支必须一一对应。
///
/// # 为什么这条必须逐条列出来
///
/// `pages::toggles` 里写了三个段，`click()` 里写了三条 `seg:*` 分支。两处都
/// 是普通字符串，编译器看不出「少了一条」。而漏掉的后果是：那一段**永远选不中**
/// —— 点下去界面上什么都不变，而分段控件的其余两段完全正常。
#[test]
fn segment_keys_are_all_wired() {
    use zephyra_gallery::app::pages::toggles::RADIO_ITEMS;

    let seg_ids = ["seg:grid", "seg:list", "seg:card"];
    // 三条都必须真的出现在选择页的树上（否则是「click 里有、页面上没有」，
    // 症状相反：点了对的事，但界面上没有那个东西）。
    let root = laid_out("toggles");
    for id in seg_ids {
        assert!(
            find_by_key(&root, id).is_some(),
            "选择页上找不到段 {id} —— pages::toggles 的 SEGS 与它对不上"
        );
    }
    // 单选组同理。
    for (id, _) in RADIO_ITEMS {
        assert!(
            find_by_key(&root, id).is_some(),
            "选择页上找不到单选项 {id}"
        );
    }
    assert_eq!(
        RADIO_ITEMS.len(),
        3,
        "RADIO_ITEMS 的长度应与 click() 里的三条分支一致"
    );
}
// ------------------------------------------- 门禁 21：响应式（官方断点 + 列数）

/// 断点判据必须与官方文档的三个类别一致。
///
/// 出处：`screen-sizes-and-breakpoints-for-responsive-design` ——
/// 「小：最大可达 640 像素；中：641 - 1007px；大：1008px及以上」。
/// 数字本身在 `responsive.rs` 里有单测，这里额外钉住**汇编关系**：
/// 类别必须由宽度单调决定，不能出现「宽窗口落到更窄的档」这种倒挂。
#[test]
fn breakpoint_categories_match_doc_and_are_monotonic() {
    use zephyra_core::responsive::WidthClass;

    let mut prev = WidthClass::Small;
    for w in (0..2600).step_by(40) {
        let c = WidthClass::of_effective_width(w as f32);
        // 宽度只增，档位只能不变或更宽。
        let order = |c: WidthClass| match c {
            WidthClass::Small => 0,
            WidthClass::Medium => 1,
            WidthClass::Large => 2,
        };
        assert!(
            order(c) >= order(prev),
            "宽度 {w} 的档位 {c:?} 窄于上一档 {prev:?} —— 断点倒挂了"
        );
        prev = c;
    }
}

/// 列数必须真的随宽度变，且基准画布下仍是 5 列。
///
/// # 为什么这三条断言都要有
///
/// 1. **基准 5 列**：不能为了响应式把 1400×800 的参考外观改掉；
/// 2. **窄窗口降列**：否则窄窗口下仍排 5 列，内容溢出被裁（本轮实测的症状）；
/// 3. **宽窗口加列**：否则宽窗口下仍排 5 列，右侧留白（本轮实测的另一个症状）。
///
/// 三条一起才构成「列数是宽度的函数」。只有第 1 条时，代码完全可以
/// 退回写死 5 —— 基准图全绿，而真实窗口下症状原样存在。
#[test]
fn column_count_tracks_width_and_baseline_is_five() {
    use zephyra_core::responsive::WidthClass;
    use zephyra_gallery::app::{Gallery, BASE_VIEWPORT};

    let at = |w: u32, h: u32, scale: f32| {
        let mut g = Gallery::default();
        g.viewport = (w as f32, h as f32);
        g.scale = scale;
        (g.width_class(), g.card_columns())
    };

    // 基准：有效 1120 epx ⇒ 大档，5 列。这一条把「参考图」钉住。
    let (cls, cols) = at(BASE_VIEWPORT.0, BASE_VIEWPORT.1, 1.0);
    assert_eq!(cls, WidthClass::Large, "基准应落在官方的大档");
    assert_eq!(cols, BASE_COLUMNS, "基准画布下必须仍 5 列");

    // 窄：有效宽度 ≤ 640 ⇒ 小档，且列数必须严格少于基准。
    //
    // 取 700 物理 @1.25 = 560 epx（小）。注意这里用**物理**视口喂进去，
    // 由 Gallery 自己除 scale —— 判据必须是有效像素，官方
    // `AdaptiveTrigger.MinWindowWidth` 的定义就是 "in effective pixels"。
    let (cls, cols) = at(700, 600, 1.25);
    assert_eq!(cls, WidthClass::Small);
    assert!(
        cols < BASE_COLUMNS,
        "窄窗口下仍排 {cols} 列 —— 内容会溢出被裁"
    );
    assert!(
        gallery_at(700, 600, 1.25).is_compact(),
        "小档必须判定为紧凑"
    );

    // 宽：有效 ≥ 1008 ⇒ 大档，且列数必须多于基准。
    let (cls, cols) = at(2400, 1400, 1.0);
    assert_eq!(cls, WidthClass::Large);
    assert!(cols > BASE_COLUMNS, "宽窗口下仍排 {cols} 列 —— 右侧会留白");

    // 单调：宽度只增，列数只能不变或增。中途允许"不变"（步长小于一个卡宽），
    // 但不允许下降 —— 下降意味着列数受别的因素影响，难以归因。
    let mut prev = 0usize;
    for w in (600..2600u32).step_by(120) {
        let cols = gallery_at(w, 900, 1.0).card_columns();
        assert!(cols >= prev, "宽度 {w} 的列数 {cols} 少于上一档 {prev}");
        prev = cols;
    }
}

fn gallery_at(w: u32, h: u32, scale: f32) -> zephyra_gallery::app::Gallery {
    let mut g = zephyra_gallery::app::Gallery::default();
    g.viewport = (w as f32, h as f32);
    g.scale = scale;
    g
}

/// 内容区可用宽度必须 = 视口 − 导航 − 页边距，且紧凑档下导航归零。
///
/// 这是列数公式的输入。若它与外壳常量脱节，列数会悄悄算错 —— 而症状
/// （最后一列被裁/多出一列空白）看起来像"样式没调好"，不像算错。
#[test]
fn content_width_follows_breakpoint() {
    let wide = gallery_at(1400, 800, 1.0);
    assert_eq!(
        wide.content_width(),
        1400.0 - zephyra_gallery::app::shell::NAV_W - 28.0,
        "非紧凑档：内容区 = 视口 − 导航 − 页边距"
    );
    assert!(!wide.is_compact());

    // 折叠真的把导航宽让给内容：紧凑档的内容区 = 视口 - 页边距（导航归零），
    // 于是与非紧凑公式（视口 - 导航 - 页边距）之差恰为 NAV_W。
    //
    // 判据在**同一个**紧凑视口下取两个值：真实的一个（导航已归零），
    // 与按非紧凑公式手算的一个。不能靠"切换到另一个 viewport"来对比 ——
    // 那样差里混着窗口宽度的变化。（第一版正是这么写的，两次断言都假。）
    let mut compact = Gallery::default();
    compact.viewport = (700.0, 600.0); // 700 @1.25 = 560 epx => 官方小档
    compact.scale = 1.25;
    assert!(compact.is_compact());
    let collapsed = compact.content_width();
    let nav = zephyra_gallery::app::shell::NAV_W;
    let gutter = zephyra_gallery::app::PAGE_GUTTER;
    assert_eq!(collapsed, 700.0 - gutter, "紧凑档内容区 = 视口 - 页边距");
    assert_eq!(
        collapsed - (700.0 - nav - gutter),
        nav,
        "折叠前后的内容区差必须恰好是导航宽（折叠后更大）"
    );
    assert!(
        collapsed > 0.0 && collapsed.is_finite(),
        "内容区必须是有界正数"
    );
}

/// 触控目标必须按 scale 换算，保证视觉上达到官方的 40 epx 下限。
///
/// 出处：`touch-interactions`「命中目标」——「可触摸定义为最小 40 x 40 epx」。
/// 本仓布局单位是物理像素，所以 40 必须乘 scale；不乘会得到 40 物理 = 32 epx
/// （1.25 缩放下），**低于官方下限而代码看起来完全正确**。
#[test]
fn touch_target_converts_to_official_ephemeral_px() {
    let g = gallery_at(1400, 800, 1.25);
    assert_eq!(
        g.touch_target(zephyra_core::responsive::TOUCH_MIN_EPX),
        50.0
    );
    assert_eq!(
        g.touch_target(zephyra_core::responsive::TOUCH_OPTIMAL_EPX),
        55.0
    );
    // 100% 下恒等；退化 scale 不应 panic 或给出 0。
    let plain = gallery_at(1400, 800, 1.0);
    assert_eq!(
        plain.touch_target(zephyra_core::responsive::TOUCH_MIN_EPX),
        40.0
    );
    let bad = gallery_at(1400, 800, 0.0);
    assert!(bad.touch_target(zephyra_core::responsive::TOUCH_MIN_EPX) > 0.0);
}

// --------------------------------------------------- 门禁 26：折射参数可调

use zephyra_gallery::app::glass::{
    Glass, Param, BEZEL, BEZEL_TEXT, DISPERSION, DISPERSION_TEXT, IOR, IOR_TEXT, THICKNESS,
    THICKNESS_TEXT,
};

/// 收集树里所有带**局部** `FILTER` 的节点上的折射参数。
///
/// # 为什么读 `iter_local` 而不是 `get`
///
/// `Node::from_element` 之后样式表还没应用时，树上的属性全是**局部**的；这里
/// 先取局部值，正是为了在样式表**不含** `filter:` 的前提下读到源码写的值。
/// 若改用 `get`（查合成后的值），这条门禁会随「样式表里又加回了一档 filter」
/// 而悄悄改成测样式表 —— 那正是它要防的回归。
fn refraction_filters(root: &Node) -> Vec<zephyra_core::filter::Filter> {
    let mut out = Vec::new();
    walk(root, &mut |n: &Node| {
        for (k, v) in n.props().iter_local() {
            if !std::ptr::eq(k, &zephyra_core::props::FILTER) {
                continue;
            }
            if let zephyra_core::property::PropertyValue::Filter(chain) = v {
                for f in chain.filters() {
                    if matches!(f, zephyra_core::filter::Filter::Refraction { .. }) {
                        out.push(*f);
                    }
                }
            }
        }
    });
    out
}

/// 换一组参数重跑整棵树，返回该树上的折射参数。
///
/// ⚠ 走的是**真的** [`Gallery::view`] + `Node::from_element`，不是直接问
/// `Glass::filter()` —— 后者只会证明查表函数自己对自己成立，而门禁要证明的是
/// 「树上那个值」跟着参数变了。
fn filters_at(g: &Gallery) -> Vec<zephyra_core::filter::Filter> {
    let theme = Theme::builtin(ThemeVariant::Dark);
    let ctx = Context::new(&theme);
    let element = g.view(&ctx);
    let root = Node::from_element(&element);
    refraction_filters(&root)
}

/// 参数的四个字段必须随 `Gallery::glass` 变 —— **这是「可调」的定义**。
///
/// # 为什么这条必须按**四张表**逐张试，而不是「改一个字段看树变没变」
///
/// 只试一个字段的话，另三个字段写死在源码里这条门禁照样绿，而画面上
/// 「IOR 那一栏的加减按钮点了没反应」。四个字段各试一次，四处死值全被覆盖。
///
/// # 恒真防御
///
/// 每种参数都要先断言**树里确实有一块玻璃**（`!got.is_empty()`），否则下面
/// 那句 `assert_ne!` 在「玻璃块根本没造出来」时会因为两边都空而绿。
#[test]
fn glass_params_are_not_hard_coded() {
    use zephyra_core::filter::Filter;

    /// 把某个 `Filter::Refraction` 的四个字段抠出来。
    fn parts(f: &Filter) -> [f32; 4] {
        match f {
            Filter::Refraction {
                strength,
                bezel,
                dispersion,
                ior,
            } => [*strength, *bezel, *dispersion, *ior],
            other => panic!("树上出现了非折射的滤镜 {other:?}"),
        }
    }

    let pick = |idx: usize, v: &Vec<Filter>| parts(&v[idx]);

    // ---- 基准树 ----
    let mut base = Gallery::default();
    base.route = "effects";
    let warm = filters_at(&base);
    assert!(
        warm.len() >= 3,
        "液态玻璃页上只有 {} 块玻璃带折射滤镜 —— 这条门禁可能恒真",
        warm.len()
    );
    let b = pick(0, &warm);

    // ---- 逐个参数各挪一档 ----
    for param in Param::ALL {
        let mut g = Gallery::default();
        g.route = "effects";
        assert!(
            g.glass.nudge(param, 1),
            "默认档位下 `{param:?}` 加一档被 clamp 掉了 —— 表的下界取得不对"
        );
        let moved = filters_at(&g);
        assert!(
            !moved.is_empty(),
            "改了 `{param:?}` 之后玻璃块从树上消失了 —— 判据本身失效"
        );
        let m = pick(0, &moved);
        let bi = param.index();
        for (j, (x, y)) in b.iter().zip(m.iter()).enumerate() {
            if j == bi {
                assert_ne!(
                    x, y,
                    "`{param:?}` 加一档后树上的这个字段（{x} → {y}）没变 —— \
                     该参数被写死在源码里了，页面上的 [+] 是摆设"
                );
            } else {
                // 其余三个必须**原样不动**。反向判据同样必要：只断「该变的变了」
                // 挡不住 `nudge` 里错位写到别人字段上的形状，而那种形状在画面上
                // 是「点厚度、IOR 跳了」。
                assert_eq!(
                    x, y,
                    "`{param:?}` 加一档后第 {j} 个字段也跟着变了（{x} → {y}）—— \
                     nudge 写错了字段"
                );
            }
        }
        // 其余三块玻璃必须**跟着一起变**（共用一组参数）—— 若有人给某一块
        // 单独特判，这一条会绿而画面上三块玻璃参数不一致。
        for k in 1..moved.len() {
            assert_eq!(
                pick(0, &moved),
                pick(k, &moved),
                "`{param:?}` 改动后第 {k} 块玻璃与第 0 块参数不一致 —— 玻璃块本该共用一组参数"
            );
        }
    }

    // ---- 步进必须**可逆**：四个各加一档再各减一档，回到基准值 ----
    //
    // 判据是「加完减完回到原处」，不是「减一档等于基准」—— 后者只在从基准
    // 减起时成立，而这里真正的风险是 `nudge` 把加减**同一个方向**实现成两次
    // 加（那种形状下每条单参数门禁都绿，而画面上 `[−]` 与 `[+]` 效果一样）。
    let mut round = Gallery::default();
    round.route = "effects";
    for param in Param::ALL {
        assert!(round.glass.nudge(param, 1), "`{param:?}` 加一档被 clamp 了");
    }
    for param in Param::ALL.iter().rev() {
        assert!(
            round.glass.nudge(*param, -1),
            "`{param:?}` 加了一档却减不回来 —— 步进不可逆"
        );
    }
    assert_eq!(
        round.glass,
        Glass::default(),
        "四个参数各加一档再各减一档后没回到默认值"
    );
    assert_eq!(
        pick(0, &filters_at(&round)),
        b,
        "加减一轮之后树上的折射参数没回到基准值"
    );
}

/// 数值表与文本表必须**逐项对齐**。
///
/// # 为什么不靠 `format!` 生成文本、而是手写两张表
///
/// 因为每帧 `format!` 会走 `Box::leak`（见 `pages::basics::spin_text` 的理由），
/// 而这一页的参数每帧都要显示。手写两张表的代价就是**可能对不齐**，所以这条
/// 门禁是它的唯一保险。
///
/// # 判据为什么不是「文本能被解析回数值」
///
/// 那需要 `str::parse`，而这里要断的是**下标对应关系**：`DISPERSION_TEXT[7]`
/// 必须是 `DISPERSION[7]` 的显示，不是「某个能被解析的数」。所以逐下标比
/// `(值, 文本)` 对，并对不上的下标直接点名。
#[test]
fn glass_param_tables_line_up() {
    macro_rules! check {
        ($name:literal, $vals:expr, $texts:expr) => {{
            let v: &[f32] = &$vals;
            let t: &[&str] = &$texts;
            assert_eq!(
                v.len(),
                t.len(),
                "{}：数值表 {} 项、文本表 {} 项 —— 下标会越界或错位",
                $name,
                v.len(),
                t.len()
            );
            for (i, (val, text)) in v.iter().zip(t.iter()).enumerate() {
                assert!(
                    !text.is_empty(),
                    "{} 第 {i} 项文本为空 —— 面板上会是一个空白",
                    $name
                );
                // 文本的数值必须与表里那一项一致（允许文本多带小数位）。
                let parsed: f32 = text.parse().unwrap_or_else(|e| {
                    panic!("{} 第 {i} 项文本 {text:?} 不是一个数：{e}", $name)
                });
                let tol = if parsed.abs() >= 10.0 { 0.01 } else { 0.006 };
                assert!(
                    (parsed - val).abs() <= tol,
                    "{} 第 {i} 项：文本 {text:?} 解析成 {parsed}，表里却是 {val}",
                    $name
                );
            }
            // 必须**严格递增** —— 步进靠的是下标 +1，若有两档同值，用户连点两下
            // 会看到数值不变，症状与「按钮没接线」无法区分。
            for w in v.windows(2) {
                assert!(
                    w[1] > w[0],
                    "{}：{} 之后是 {} —— 档位没有严格递增，点两下看起来像没反应",
                    $name,
                    w[0],
                    w[1]
                );
            }
        }};
    }

    check!("THICKNESS", THICKNESS, THICKNESS_TEXT);
    check!("BEZEL", BEZEL, BEZEL_TEXT);
    check!("DISPERSION", DISPERSION, DISPERSION_TEXT);
    check!("IOR", IOR, IOR_TEXT);

    // IOR 必须含 1.00（不折射的诊断档）与 1.50（普通冕牌玻璃）。
    // 少了 1.00，「公式被改坏了吗」这个判断在页面上就没法做。
    assert!(
        IOR.contains(&1.0),
        "IOR 表里没有 1.00 —— 少了「不折射」这条诊断档"
    );
    assert!(
        IOR.contains(&1.5),
        "IOR 表里没有 1.50 —— 普通冕牌玻璃那档没了"
    );
    // 倒角下限不得为 0（除零，见 `BEZEL` 的文档）。
    //
    // ⚠ 判据写成「**每一档**都不小于 2」而不是「第 0 档不小于 2」：后者 clippy
    // 会判成恒真（编译期已知 `BEZEL[0]` 的值），而前者在中间档被改成 0 时才
    // 说得清是哪一档出的问题。
    for (i, b) in BEZEL.iter().enumerate() {
        assert!(
            *b >= 2.0,
            "BEZEL 第 {i} 档是 {b} —— 小于 2 会让着色器里的 `max(color.g, 1.0)` \
             把整块玻璃都塞进强折射区"
        );
    }
    // 默认档必须落在表内，否则 `Glass::default()` 会越界 panic。
    let d = Glass::default();
    assert!(
        d.thickness < THICKNESS.len(),
        "默认厚度档 {} 越界",
        d.thickness
    );
    assert!(d.bezel < BEZEL.len(), "默认倒角档 {} 越界", d.bezel);
    assert!(
        d.dispersion < DISPERSION.len(),
        "默认色散档 {} 越界",
        d.dispersion
    );
    assert!(d.ior < IOR.len(), "默认 IOR 档 {} 越界", d.ior);
}

/// 面板上每个 `[−] [+] 都必须在树上，且点了真的改参数。
///
/// # 为什么不能只看「key 在树上」
///
/// 「key 在树上」只保证按钮**画出来了**；而 `click()` 的分派是**运行期字符串
/// 解析**（剥 `glass-` 前缀 → 找 `Param::slug()` → 看后缀），编译器看不出
/// 「面板发的是 `glass-thickness:dec` 而 `slug()` 返回 `thickness` 但中间少个
/// 冒号」。所以这里**真的点一遍**再回读参数。
///
/// # 恒真防御
///
/// 断言 8 个按钮全在树上；按钮少画了就会在这里报，而不是静默通过。
#[test]
fn glass_param_keys_all_wired() {
    let root = laid_out("effects");
    let mut keys = Vec::new();
    for param in Param::ALL {
        keys.push(format!("glass-{}:dec", param.slug()));
        keys.push(format!("glass-{}:inc", param.slug()));
    }
    assert_eq!(keys.len(), 8, "四个参数 × 增减 = 8 个按钮");
    for k in &keys {
        assert!(
            find_by_key(&root, k).is_some(),
            "液态玻璃页上找不到按钮 {k} —— Param::ALL 与 effects.rs 的面板对不上"
        );
    }

    // 逐个点「加」，每次都要求参数真的动了。
    let mut g = Gallery::default();
    g.route = "effects";
    for param in Param::ALL {
        let before = g.glass;
        g.click(&format!("glass-{}:inc", param.slug()));
        assert_ne!(
            g.glass,
            before,
            "点 `glass-{}:inc` 之后参数没变 —— 分派漏了这个 key",
            param.slug()
        );
    }
    // 再逐个点「减」，回到默认值。
    for param in Param::ALL.iter().rev() {
        g.click(&format!("glass-{}:dec", param.slug()));
    }
    assert_eq!(g.glass, Glass::default(), "加减一轮之后没回到默认值");
}

/// 边界上的 `[−] [+] 必须是**无副作用**的 no-op。
///
/// # 为什么单独一条
///
/// 「已到上限还继续加」若写成了别的形状（例如 `*slot = max` 无条件赋值），
/// 画面上仍然一切正常，但**上一条门禁的判据就不再可信** —— 因为它靠
/// 「改参数 → 树变」来证明参数没被写死。`Glass::nudge` 返回 `bool` 就是给这条
/// 门禁用的，调用方（`click`）刻意忽略它，于是只有这里能验。
#[test]
fn glass_param_steppers_are_clamped_without_side_effects() {
    let mut g = Gallery::default();
    for param in Param::ALL {
        // 一路加到顶。默认档不在 0 档，所以先一路减到底再从 0 起数 ——
        // 否则「加了几次」与「离上限还有几档」永远差一个常数，这条判据就废了。
        while g.glass.nudge(param, -1) {}
        let steps = match param {
            Param::Thickness => THICKNESS.len() - 1,
            Param::Bezel => BEZEL.len() - 1,
            Param::Dispersion => DISPERSION.len() - 1,
            Param::Ior => IOR.len() - 1,
        };
        let mut n = 0;
        while g.glass.nudge(param, 1) {
            n += 1;
            assert!(n <= steps, "步进加到了 {n} 次还不停 —— clamp 没生效");
        }
        assert_eq!(n, steps, "从 0 档加到顶应是 {steps} 次，实测 {n}");
        // 顶上一格：no-op。
        let top = g.glass;
        assert!(!g.glass.nudge(param, 1), "已到上限却报告「动了」");
        assert_eq!(g.glass, top, "已到上限再加却改变了状态");
        // 顶上加两次（防「单次 clamp、双次累积」那种形状）。
        assert!(!g.glass.nudge(param, 1));
        assert_eq!(g.glass, top);
        // 一路减到底：步进次数必须与加法对称（到 0 档的距离相同）。
        let mut k = 0;
        while g.glass.nudge(param, -1) {
            k += 1;
            assert!(k <= steps, "步进减到了 {k} 次还不停 —— clamp 没生效");
        }
        assert_eq!(k, steps, "从顶档减到底应是 {steps} 次，实测 {k}");
        let bottom = g.glass;
        assert!(!g.glass.nudge(param, -1), "已到下限却报告「动了」");
        assert_eq!(g.glass, bottom, "已到下限再减却改变了状态");
        // 步进**一个**参数不得牵动其它三个 —— `nudge` 里那四条 match 分支若写成了
        // 「回写整个结构体」，这条会绿而画面上四个值一起跳。
        let mut probe = Gallery::default();
        let others = |g: &Glass| -> Vec<usize> { vec![g.thickness, g.bezel, g.dispersion, g.ior] };
        let before = others(&probe.glass);
        for p in Param::ALL {
            probe.glass.nudge(p, 1);
        }
        let after = others(&probe.glass);
        for (i, (b, a)) in before.iter().zip(after.iter()).enumerate() {
            assert_ne!(
                b, a,
                "四个参数都加了一档，但第 {i} 个没变 —— nudge 漏了某个字段"
            );
        }
    }
}

// --------------------------------------------------- 门禁 27：死规则反查

/// 样式表里每条规则都得**选得中至少一个元素** —— 反方向的门禁。
///
/// # 为什么前两条不够
///
/// `every_class_name_the_source_emits_exists_in_the_stylesheet` 与
/// `every_element_is_reachable_by_the_stylesheet` 都是**树 → 表**这一个方向：
/// 前者查「源码发的每个分类名在表里有规则吗」，后者查「每个元素有分类名或裸规则
/// 吗」。合起来仍漏掉一整类静默失效：**表里有规则，而树里没有任何元素能被它
/// 选中**。
///
/// # 实测过一次（写这条门禁的直接原因）
///
/// 给圆按钮调尺寸时写了 `Control.button.round { width: 28px; height: 28px; }`。
/// `controls::round_button` 发的是 `NodeTag::Control` + 分类名 `round` ——
/// **没有** `button` 这个分类名（`button` 是别的控件用的）。于是这条规则一个
/// 元素都选不中，尺寸**纹丝不动**，而两个方向的门禁全绿：分类名 `round` 有规则、
/// 元素有分类名。这正是「门禁全绿但界面是错的」那一类。
///
/// # 交互态为什么必须跑全
///
/// `.pressed` / `.checked` / `.hovered` 规则只在对应状态下选得中元素，所以判据
/// 取**全部路由 × 全部交互脚本**的并集（[`interaction_scripts`]），而不是只看
/// 默认状态。
///
/// # 为什么不查**控件层**（那 15 张表）
///
/// 控件层是**给宿主用的样式库**，它承诺覆盖一整套状态 —— `Text.focused`、
/// `Container.toast.error`、`Container.ring-track.paused`、`Container.suggest-row.selected`
/// 都在表里，而本 gallery 的界面**根本不存在**那些状态（没有 toast、没有输入框、
/// 没有建议列表）。按全表判，这些规则会全部报成死规则，于是唯一的选择是删规则
/// —— 那是把控件层改成「只服务当前 demo」，比留几条暂时用不上的规则坏得多。
///
/// 所以判据按 [`zephyra_gallery::styles::app_only`] 取**应用层**规则：应用层
/// 描述的就是这个界面，用不上的规则就是真死规则。控件层留待有对应界面时再查。
///
/// # 恒真防御
///
/// 先断言「参与判定的元素远多于规则数」，否则树遍历没走完时这条会变成「没有规则
/// 需要检查」而恒真地绿。
#[test]
fn every_stylesheet_rule_matches_something() {
    use std::collections::HashSet;
    use zephyra_core::id::ElementKey;

    type Shapes = HashSet<(NodeTag, Vec<&'static str>)>;

    // ⚠ 判据**只取应用层**（`shell.zstyle` + `pages.zstyle`）的规则，不看控件层。
    // 见函数文档「为什么不查控件层」那一节。
    let mut rules: Vec<(usize, String, zephyra_style::ir::Selector)> = Vec::new();
    for (sheet_name, src) in zephyra_gallery::styles::app_only() {
        let parsed = zstylec::parse(sheet_name, src);
        assert!(
            parsed.is_ok(),
            "{sheet_name}.zstyle must parse: {:?}",
            parsed.errors
        );
        for (i, r) in parsed.sheet.rules().iter().enumerate() {
            rules.push((i, (*sheet_name).to_string(), r.selector.clone()));
        }
    }

    let theme = Theme::builtin(ThemeVariant::Dark);
    let ctx = Context::new(&theme);

    /// 从默认状态重造一个 `Gallery`：路由 + 脚本 + 额外改一处。
    ///
    /// # 为什么每次**重造**而不是 clone
    ///
    /// `Gallery` 里挂着 `Arc<AtomicU8>`（主题意图，宿主每帧写）与 `Preferences`，
    /// 派生 `Clone` 会让「门禁克隆了一份宿主状态」变成可能 —— 而那条路一旦走歪，
    /// 门禁验的就是它自己造的世界。重造只有一条真相来源：`Gallery::default()`。
    fn build(
        route: &'static str,
        script: fn(&mut Gallery),
        extra: impl FnOnce(&mut Gallery),
    ) -> Gallery {
        let mut g = Gallery::default();
        g.route = route;
        script(&mut g);
        extra(&mut g);
        g
    }

    /// 看一棵树：登记所有出现过的「类型 + 分类名」形状，并交回可交互的 key。
    ///
    /// ⚠ 只登记**形状**不登记节点：`Selector::matches` 看的就是这两样，而把整棵
    /// 树留在哈希表里在几千次构造下要几百 MB。
    fn absorb(
        seen: &mut Shapes,
        elements: &mut usize,
        g: &Gallery,
        ctx: &Context<'_>,
    ) -> Vec<&'static str> {
        let element = g.view(ctx);
        let root = Node::from_element(&element);
        let mut keys = Vec::new();
        walk(&root, &mut |n: &Node| {
            *elements += 1;
            seen.insert((n.tag(), n.style_names().to_vec()));
            let interactive = n
                .props()
                .iter_local()
                .any(|(k, _)| std::ptr::eq(k, &zephyra_core::props::INTERACTIVE));
            if interactive {
                if let Some(k) = n.key().and_then(|k| k.as_name()) {
                    keys.push(k);
                }
            }
        });
        keys.sort_unstable();
        keys.dedup();
        keys
    }

    let routes: Vec<&str> = ROUTES
        .iter()
        .map(|(id, _, _)| *id)
        .chain(std::iter::once(SETTINGS_ROUTE.0))
        .collect();

    let mut seen: Shapes = HashSet::new();
    let mut elements = 0usize;

    // ⚠ 记下第一项再迭代：趟 D 要在循环里回头引用它，而 \or x in vec\ 会 move 掉 vec。
    let first_route = routes[0];
    for route in routes {
        // ---- 趟 A：全部交互脚本，只登记形状，顺便攒下这一页出现过的交互 key ----
        //
        // ⚠ 形状登记与 key 收集**必须一趟做完**：分两趟就要把树留下来，而树不留
        // （见 `absorb` 的注释），于是第二趟只能重跑一遍脚本。
        let mut keys: Vec<&'static str> = Vec::new();
        for (_, script) in interaction_scripts() {
            let g = build(route, script, |_| {});
            for k in absorb(&mut seen, &mut elements, &g, &ctx) {
                keys.push(k);
            }
        }
        keys.sort_unstable();
        keys.dedup();

        // ---- 趟 B：逐个交互 key 悬停 / 按住（用**默认**脚本造）----
        //
        // 为什么只在默认脚本下造悬停态：`hovered` / `pressed` 的**分类名**与脚本
        // 无关（`state()` 只看指针落在哪个 key 上），所以「哪些规则会因此被选中」
        // 只取决于**哪些 key 存在**，而 key 的全集刚在趟 A 里拿到了。
        //
        // ⚠ 这也是为什么不能手列 key：漏一个，那个元素身上的 `.hovered` /
        // `.pressed` 规则就会被误判成死规则，于是这条门禁的第一次失败会教人去
        // **删规则** —— 那比没有这条门禁更坏。
        for k in &keys {
            absorb(
                &mut seen,
                &mut elements,
                &build(route, |_| {}, |g| g.hovered = Some(ElementKey::Name(k))),
                &ctx,
            );
            absorb(
                &mut seen,
                &mut elements,
                &build(
                    route,
                    |_| {},
                    |g| {
                        g.pressed = Some(ElementKey::Name(k));
                        g.last_pressed = Some(ElementKey::Name(k));
                    },
                ),
                &ctx,
            );
        }

        // ---- 趟 C：断点三档（`.compact` 只在小档出现）----
        for w in [640.0_f32, 1000.0, 1400.0] {
            absorb(
                &mut seen,
                &mut elements,
                &build(route, |_| {}, |g| g.viewport = (w, 900.0)),
                &ctx,
            );
        }

        // ---- 趟 D：入场动画的 8 个档 ----
        //
        // ⚠ 采样必须**密**于档位数：`enter_class()` 走的是缓动函数
        // （`anim::progress` → `anim::step`），进度到档位边界的映射不是线性的，
        // 8 个均匀中点只落进其中 4~5 个桶 —— 于是 `.enter-3` 这类规则会被误判成
        // 死规则。取 8 倍密度的采样（64 个），代价是几十棵树，可以忽略。
        //
        // 只在**第一个路由**上跑：`.page` 与 `.enter-N` 是外壳发的，每个路由都有。
        if route == first_route {
            for i in 0..64 {
                let ms = (i as f32 + 0.5) * zephyra_style::anim::ENTER_MS / 64.0;
                absorb(
                    &mut seen,
                    &mut elements,
                    &build(route, |_| {}, |g| g.enter_ms = ms),
                    &ctx,
                );
            }
        }
    }

    assert!(
        elements > 5000,
        "只遍历了 {elements} 个元素 —— 树遍历没走完，这条门禁可能恒真"
    );
    assert!(
        rules.len() > 50,
        "应用层只有 {} 条规则 —— 判据可能没取到规则",
        rules.len()
    );

    let dead: Vec<String> = rules
        .iter()
        .filter(|(_, _, sel)| !seen.iter().any(|(tag, names)| sel.matches(*tag, names)))
        .map(|(i, sheet_name, sel)| {
            let names: Vec<String> = sel.names.iter().map(|n| format!(".{n}")).collect();
            format!(
                "{sheet_name}.zstyle 第 {i} 条 `{}{}`",
                sel.tag.map(|t| format!("{t:?}")).unwrap_or_default(),
                names.concat()
            )
        })
        .collect();

    assert!(
        dead.is_empty(),
        "样式表里这些规则一个元素都选不中（写了等于没写，而树→表那两条门禁不会报）：\n  {}",
        dead.join("\n  ")
    );
}

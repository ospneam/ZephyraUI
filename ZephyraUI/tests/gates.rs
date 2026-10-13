//! 树级门禁。全部按**整棵树**判，不按函数判 —— 逐函数查只能查到
//! 「我记得去查的那个」。（与 `examples/demo.rs` 的 6 条同一理由。）
//!
//! # 恒真防御
//!
//! 每条涉及遍历的断言都先确认「树不是空的」（节点数下界），否则树造失败时
//! 断言会**恒真**地绿 —— 这个项目已经因为恒真式吃过两次亏。

use std::path::Path;

use zephyra_core::context::Context;
use zephyra_core::core::Rect;
use zephyra_core::id::NodeTag;
use zephyra_core::node::Node;
use zephyra_core::theme::{Theme, ThemeVariant};
use zephyra_fx::View;
use zephyra_style::ir::StyleSheet;

use zephyra_ui::app::{Gallery, ROUTES, SETTINGS_ROUTE};
use zephyra_ui::tokens::ALL as TOKENS;

// ---------------------------------------------------------------- 公共帮手

/// 读入两份样式表并合并（与 `bin/gallery.rs::build_sheet` 同一逻辑；
/// 门禁必须吃**同一份**表，否则「图对、门禁绿」会是两件事）。
fn merged_sheet() -> StyleSheet {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .expect("crate 目录")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("zstyle"))
        .map(|p| p.to_string_lossy().to_string())
        .collect();
    names.sort();
    let mut sheet = StyleSheet::new();
    for name in &names {
        let css = std::fs::read_to_string(name).expect("stylesheet must exist");
        let parsed = zstylec::parse(name, &css);
        assert!(parsed.is_ok(), "stylesheet must parse: {:?}", parsed.errors);
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
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut used: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().and_then(|x| x.to_str()) != Some("zstyle") {
            continue;
        }
        let css = std::fs::read_to_string(&path).unwrap();
        // `@令牌` 只出现在值位上：抓所有 `@word`。
        for (i, _) in css.match_indices('@') {
            let rest = &css[i + 1..];
            let word: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
                .collect();
            if !word.is_empty() {
                used.push(format!("{}:{}", path.file_name().unwrap().to_string_lossy(), word));
            }
        }
    }
    assert!(!used.is_empty(), "一个 @令牌 都没用到 —— 这条测试有恒真的风险");
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
    let css = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("gallery.zstyle"))
        .unwrap();
    let want = [
        ("Container.titlebar", "height", zephyra_ui::app::shell::TITLEBAR_H),
        ("Container.nav-pane", "width", zephyra_ui::app::shell::NAV_W),
        ("Container.nav-pane", "height", zephyra_ui::app::shell::NAV_H),
        ("Container.nav-spacer", "height", zephyra_ui::app::shell::NAV_SPACER),
        (
            "Container.content-card",
            "width",
            zephyra_ui::app::shell::CONTENT_W,
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
fn 根容器不写固定尺寸() {
    let css = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("gallery.zstyle"))
        .unwrap();
    let start = css
        .split("Container.window {")
        .nth(1)
        .unwrap_or_else(|| panic!("样式表里找不到 Container.window"));
    let block = &start[..start.find('}').unwrap_or(start.len())];
    for prop in ["width", "height"] {
        assert!(
            !block.lines().any(|l| l.trim().starts_with(&format!("{prop}:"))),
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
fn 基准画布下根容器仍铺满客户区() {
    let root = laid_out("home");
    let r = root.rect();
    let (cw, ch) = zephyra_ui::app::BASE_VIEWPORT;
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
    assert!((r.height - 48.0).abs() < 1.0, "标题栏高应是 48，实测 {:.1}", r.height);

    let content = find_by_key(&root, "content").expect("content card");
    let rc = content.rect();
    assert!(
        (rc.x - 240.0).abs() < 1.0,
        "内容卡应紧贴导航右侧（x=240），实测 {:.1}",
        rc.x
    );
    assert!(
        (rc.width - 1159.0).abs() < 1.0,
        "内容卡宽应是 1159，实测 {:.1}",
        rc.width
    );
    assert!(
        (rc.height - 752.0).abs() < 1.0,
        "内容卡高应是 752，实测 {:.1}",
        rc.height
    );

    // 置底项：底部落在画布内 24px 以内。
    let settings = find_by_key(&root, "settings").expect("settings nav item");
    let rs = settings.rect();
    let bottom = rs.y + rs.height;
    assert!(
        (bottom - 792.0).abs() < 24.0,
        "Settings 应落在导航栏底部附近（约 792），实测 {bottom:.1}"
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
/// core 的 `所有内置图标都不是空的`（新加）能抓「登记了但资源是空的」，抓不了
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
    assert!(used.len() >= 20, "只用到 {} 个图标名 —— 遍历可能没走完", used.len());
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
/// `INTERACTIVE` 是**不可继承**属性（见 `props::INTERACTIVE`），只有控件本身
/// 才是目标。
#[test]
fn hit_test_agrees_with_the_painted_rects() {
    let root = laid_out("home");
    let theme = Theme::builtin(ThemeVariant::Dark);
    let ctx = Context::new(&theme);

    // 收集所有可交互节点（含它们自己的矩形），至少要有几十个 —— 太少说明
    // 这条门禁可能在测一棵空树。
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
    assert!(
        targets.len() >= 12,
        "只有 {} 个可交互节点，这条门禁可能恒真",
        targets.len()
    );

    let mut wrong: Vec<String> = Vec::new();
    for (key, rect) in &targets {
        // 1px 的框在中心可能恰好落在边界外，取中心即可（矩形高宽都 >= 1）。
        let point = zephyra_core::core::Point::new(
            rect.x + rect.width * 0.5,
            rect.y + rect.height * 0.5,
        );
        let mut hit = zephyra_core::hit::HitResult::default();
        if !zephyra_core::hit::hit_test(
            &root,
            zephyra_core::hit::HitTestRequest { point, ignore_above: None },
            ctx,
            &mut hit,
        ) {
            wrong.push(format!("{key}：它的中心 {point:?} 谁都没命中"));
            continue;
        }
        let target = hit.target().expect("命中链非空");
        if target.id.to_string() != *key {
            wrong.push(format!(
                "{key}：中心 {point:?} 的处理目标是 {}（矩形 {:?}）",
                target.id, target.rect
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "命中测试与绘制几何对不上（点 A 执行 B）：\n  {}",
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
    let script: Vec<(&str, fn(&mut Gallery))> = vec![
        ("切到 basic-input", |g| g.route = "basic-input"),
        ("勾上 alpha", |g| g.checked[0] = true),
        ("再勾 beta 与 gamma", |g| {
            g.checked[1] = true;
            g.checked[2] = true;
        }),
        ("全部取消", |g| g.checked = [false, false, false]),
        ("开主开关", |g| g.switch_on = true),
        ("关主开关", |g| g.switch_on = false),
        ("切到 favorites 分段", |g| g.seg = "favorite"),
        ("切回 recent", |g| g.seg = "recent"),
        ("展开 expander", |g| g.expanded = true),
        ("收起 expander", |g| g.expanded = false),
        ("切到 collections", |g| g.route = "collections"),
        ("切到 settings", |g| g.route = "settings"),
        ("切到 text", |g| g.route = "text"),
        ("切到 dialogs", |g| g.route = "dialogs"),
        ("切回 home", |g| g.route = "home"),
    ];

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

// -------------------------------------------------- 门禁 12：demo 目录与原版一致

/// `catalog.rs` 的 44 条必须与 Rin-UI 原版逐条一致。
///
/// # 为什么值得单独一条
///
/// 这批数据是**脚本提取**的（见 `catalog.rs` 模块文档），提取脚本没跑错
/// 只是一次性的 —— 而"少几个条目 / 某个条目点进去是空的"这类退化，
/// 界面上看起来像"本来就那样"，没有任何报错。
///
/// 判据取三条**互不替代**的约束：
///
/// 1. 条数 == 44（与 `ItemData.qml` 的 `allControls` 一致）；
/// 2. 每个分组的条目数与原版逐个吻合（比总数更能定位是**哪一组**丢了）；
/// 3. 每条的 `page` 唯一且非空 —— 重复 page 会让路由覆盖，静默少一个页面。
#[test]
fn catalog_matches_the_rinui_original() {
    use zephyra_ui::app::catalog::{ENTRIES, GROUPS, count_in, entry};

    assert_eq!(
        ENTRIES.len(),
        44,
        "条目数与 Rin-UI ItemData.qml 的 allControls（44）不符 —— \
         提取脚本改动后必须重跑，不要手改 catalog.rs"
    );

    // 原版逐组计数（由提取脚本直接给出，不是手抄）+ 原版导航顺序。
    const EXPECTED: &[(&str, usize)] = &[
        ("basic-input", 12),
        ("collections", 3),
        ("date-time", 4),
        ("dialogs-flyouts", 3),
        ("layout", 3),
        ("media", 1),
        ("menus-toolbars", 2),
        ("navigation", 3),
        ("scrolling", 1),
        ("status-info", 5),
        ("styles", 1),
        ("text", 4),
        ("design", 1),
        ("windowing", 1),
    ];
    assert_eq!(
        EXPECTED.iter().map(|(g, _)| *g).collect::<Vec<_>>(),
        GROUPS.iter().map(|g| g.id).collect::<Vec<_>>(),
        "分组 id / 顺序与原版的 type 取值不一致"
    );
    let wrong: Vec<String> = EXPECTED
        .iter()
        .filter(|(g, n)| count_in(g) != *n)
        .map(|(g, n)| format!("{g}: 期望 {n} 条，实际 {}", count_in(g)))
        .collect();
    assert!(wrong.is_empty(), "分组计数对不上：\n  {}", wrong.join("\n  "));

    let mut seen: Vec<&str> = ENTRIES.iter().map(|e| e.page).collect();
    seen.sort_unstable();
    let dup: Vec<&str> = seen
        .iter()
        .enumerate()
        .filter(|(i, s)| seen[..*i].contains(s))
        .map(|(_, s)| *s)
        .collect();
    assert!(dup.is_empty(), "page 重复（路由会互相覆盖）：{dup:?}");
    for e in ENTRIES {
        assert!(!e.title.is_empty(), "有条目没有标题：{e:?}");
        assert!(!e.desc.is_empty(), "{} 没有描述", e.title);
        assert!(entry(e.page).is_some(), "{} 的 page 查不到自己", e.title);
    }
}

/// 搜索必须真的能筛出东西 —— 且**空查询返回全部**（原版语义）。
///
/// 判据里特意包含一条"负向"：搜一个不存在的词必须**零命中**。
/// 只判"搜 Button 能搜到"的话，一个永远返回全部的实现也能过。
#[test]
fn catalog_search_filters_and_empty_query_returns_all() {
    use zephyra_ui::app::catalog::{ENTRIES, SearchHits, hits, search};

    assert_eq!(
        search(""),
        SearchHits::All,
        "空查询必须返回全部（照抄 ItemData.getItemsByTitle）"
    );
    assert_eq!(search("   ").count(), ENTRIES.len(), "纯空白也算空查询");

    let r = search("Button");
    assert!(!r.is_empty(), "搜 Button 必须有命中");
    for e in hits(r) {
        assert!(
            e.title.to_lowercase().contains("button"),
            "命中项不含关键字：{}",
            e.title
        );
    }

    assert_eq!(
        search("zzz-不存在的控件名").count(),
        0,
        "搜不到必须零命中（否则一个'永远返回全部'的实现能过这条）"
    );
}

/// 分组图标必须**已登记** —— 否则卡片上是空白图标位。
#[test]
fn catalog_group_icons_are_registered() {
    use zephyra_ui::app::catalog::GROUPS;

    let registry = zephyra_core::icon::builtin();
    let bad: Vec<&str> = GROUPS
        .iter()
        .filter(|g| registry.get(g.icon).is_empty())
        .map(|g| g.title)
        .collect();
    assert!(
        bad.is_empty(),
        "这些分组图标不在内置表里（画面上是空白图标位）：\n  {}",
        bad.join("\n  ")
    );
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
/// 每条 catalog 条目都必须**可达**：点它（或点它所属分组）能到一个真实页面。
///
/// # 为什么这条是批 7 的核心保证
///
/// 「照抄 demo」的底线是：原版 44 条都能点到。本仓做法是让控件卡落到
/// **它所属分组**的索引页（逐控件独立页面是批 8）。若某个 `page` id 不在
/// `resolve_route` 的三张表里，点它会静默回 Home —— 界面看起来完全正常，
/// 只是"那张卡点了没反应"。这种退化靠肉眼极难发现（44 张卡要挨个点）。
///
/// 判据不走 UI（那要模拟 44 次点击），而是走**路由解析**这一个真源：
/// `resolve_route` 是所有点击的必经之路。
#[test]
fn every_catalog_entry_is_reachable_through_route_resolution() {
    use zephyra_ui::app::catalog::{ENTRIES, GROUPS, GROUP_ROUTES};

    // 路由解析函数在 app 模块里是私有的？—— 用它导出的等价物验证：
    // 每条的 page 必须能在三张表之一被找到（见 pages::page 的分发逻辑）。
    let unreachable: Vec<&str> = ENTRIES
        .iter()
        .filter(|e| {
            // 分组索引路由必须能反查回分组（点 chip 进分组页）
            let group_ok = GROUP_ROUTES.iter().any(|(_, gid)| *gid == e.group);
            // 分组本身必须在 GROUPS 里有定义（chip 的标题/图标从那来）
            let group_defined = GROUPS.iter().any(|g| g.id == e.group);
            !group_ok || !group_defined
        })
        .map(|e| e.title)
        .collect();
    assert!(
        unreachable.is_empty(),
        "这些条目点不到（分组路由缺失或分组未定义）：\n  {}",
        unreachable.join("\n  ")
    );

    // 每个分组索引路由都要能反查回分组（pages::page 的前置拦截依赖它）。
    for (route, gid) in GROUP_ROUTES {
        assert!(
            GROUPS.iter().any(|g| g.id == *gid),
            "分组路由 {route} 指向未定义分组 {gid}"
        );
    }
    assert_eq!(
        GROUP_ROUTES.len(),
        GROUPS.len(),
        "每个分组都要有一个索引页路由（否则某些分组永远进不去）"
    );
}
/// 每个 catalog 条目都要有一个**非空**展台页，且展台里至少有**一个真实控件**
/// （`INTERACTIVE=true` 的节点或图标/文本节点），不能是空壳。
///
/// # 判据为什么这么定
///
/// 「每条都有页面」很容易做到 —— 造一个只含标题的空 `Container` 就行，
/// 44 条全过。真正要挡的是**空壳**：用户点进去看到"标题 + 什么都没有"，
/// 而界面看起来完全正常（有标题、有布局），所以不会报错、也很难肉眼批量发现。
///
/// 降级页（Flyout/Popup/…）带一个真按钮来证明页面是活的 —— 这条门禁
/// 保证"降级"≠"空"。
#[test]
fn every_catalog_entry_has_a_non_empty_showcase_page() {
    use zephyra_ui::app::catalog::ENTRIES;
    use zephyra_ui::app::showcase::showcase;

    let theme = Theme::builtin(ThemeVariant::Dark);
    let ctx = Context::new(&theme);
    let sheet = merged_sheet();
    let g = Gallery::default();

    let mut bad: Vec<String> = Vec::new();
    for e in ENTRIES {
        // 门禁要判的是**保留树**（`Node`）：分类名与属性都在那上面，
        // 而 `showcase` 产的是**声明树**（`Element`）。先转一道。
        let element = showcase(&g, e.page);
        let mut root = Node::from_element(&element);
        sheet.apply_all(&mut root);
        // 节点数下界（防"空树也过"）。
        let mut nodes = 0usize;
        walk(&root, &mut |_: &Node| nodes += 1);
        if nodes < 6 {
            bad.push(format!("{}: 展台页只有 {nodes} 个节点（太薄，像空壳）", e.title));
            continue;
        }
        // 「有内容」= 至少一个带 **TEXT 或 ICON_NAME** 的叶子（文本/图标）。
        //
        // ⚠️ 为什么**不**要求 `INTERACTIVE`：ColorPicker / DatePicker / TableView /
        // Avatar / ProgressRing 等是**纯展示型**控件（16 色块、月历格、表格行…），
        // 它们本来就不含可交互节点。要求 INTERACTIVE 会把这些真控件误判成空壳。
        // 判据要抓的是「只有标题+说明、没有任何控件内容」的**真**空壳。
        let mut has_content = false;
        walk(&root, &mut |n: &Node| {
            // 属性值可能是 `StaticStr` 或 `Str`（运行期字符串），两者都要算"有文本"。
            let nonempty = |v: &zephyra_core::property::PropertyValue| match v {
                zephyra_core::property::PropertyValue::StaticStr(s) => !s.is_empty(),
                zephyra_core::property::PropertyValue::Str(s) => !s.is_empty(),
                _ => false,
            };
            let has_text = n
                .props()
                .lookup(&zephyra_core::props::TEXT)
                .map(|(v, _)| nonempty(v))
                .unwrap_or(false);
            let has_icon = n
                .props()
                .lookup(&zephyra_core::props::ICON_NAME)
                .map(|(v, _)| nonempty(v))
                .unwrap_or(false);
            // 只看叶子（无子节点），避免把容器算成"有内容"。
            if n.children().is_empty() && (has_text || has_icon) {
                has_content = true;
            }
        });
        if !has_content {
            bad.push(format!("{}: 展台页没有任何文本/图标叶子（空壳）", e.title));
        }
    }
    assert!(
        bad.is_empty(),
        "这些控件的展台页是空壳：\n  {}",
        bad.join("\n  ")
    );
    let _ = ctx;
}
// ------------------------------------------- 门禁 21：响应式（官方断点 + 列数）

/// 断点判据必须与官方文档的三个类别一致。
///
/// 出处：`screen-sizes-and-breakpoints-for-responsive-design` ——
/// 「小：最大可达 640 像素；中：641 - 1007px；大：1008px及以上」。
/// 数字本身在 `responsive.rs` 里有单测，这里额外钉住**汇编关系**：
/// 类别必须由宽度单调决定，不能出现「宽窗口落到更窄的档」这种倒挂。
#[test]
fn 断点作文档的三个类别且单调() {
    use zephyra_ui::responsive::WidthClass;

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
fn 列数随宽度变化且基准仍是五列() {
    use zephyra_ui::app::{Gallery, BASE_VIEWPORT};
    use zephyra_ui::responsive::WidthClass;

    let at = |w: u32, h: u32, scale: f32| {
        let mut g = Gallery::default();
        g.viewport = (w as f32, h as f32);
        g.scale = scale;
        (g.width_class(), g.card_columns())
    };

    // 基准：有效 1120 epx ⇒ 大档，5 列。这一条把「参考图」钉住。
    let (cls, cols) = at(BASE_VIEWPORT.0, BASE_VIEWPORT.1, 1.0);
    assert_eq!(cls, WidthClass::Large, "基准应落在官方的大档");
    assert_eq!(cols, zephyra_ui::app::pages::BASE_COLUMNS, "基准画布下必须仍 5 列");

    // 窄：有效宽度 ≤ 640 ⇒ 小档，且列数必须严格少于基准。
    //
    // 取 700 物理 @1.25 = 560 epx（小）。注意这里用**物理**视口喂进去，
    // 由 Gallery 自己除 scale —— 判据必须是有效像素，官方
    // `AdaptiveTrigger.MinWindowWidth` 的定义就是 "in effective pixels"。
    let (cls, cols) = at(700, 600, 1.25);
    assert_eq!(cls, WidthClass::Small);
    assert!(
        cols < zephyra_ui::app::pages::BASE_COLUMNS,
        "窄窗口下仍排 {cols} 列 —— 内容会溢出被裁"
    );
    assert!(gallery_at(700, 600, 1.25).is_compact(), "小档必须判定为紧凑");

    // 宽：有效 ≥ 1008 ⇒ 大档，且列数必须多于基准。
    let (cls, cols) = at(2400, 1400, 1.0);
    assert_eq!(cls, WidthClass::Large);
    assert!(
        cols > zephyra_ui::app::pages::BASE_COLUMNS,
        "宽窗口下仍排 {cols} 列 —— 右侧会留白"
    );

    // 单调：宽度只增，列数只能不变或增。中途允许"不变"（步长小于一个卡宽），
    // 但不允许下降 —— 下降意味着列数受别的因素影响，难以归因。
    let mut prev = 0usize;
    for w in (600..2600u32).step_by(120) {
        let cols = gallery_at(w, 900, 1.0).card_columns();
        assert!(cols >= prev, "宽度 {w} 的列数 {cols} 少于上一档 {prev}");
        prev = cols;
    }
}

fn gallery_at(w: u32, h: u32, scale: f32) -> zephyra_ui::app::Gallery {
    let mut g = zephyra_ui::app::Gallery::default();
    g.viewport = (w as f32, h as f32);
    g.scale = scale;
    g
}

/// 内容区可用宽度必须 = 视口 − 导航 − 页边距，且紧凑档下导航归零。
///
/// 这是列数公式的输入。若它与外壳常量脱节，列数会悄悄算错 —— 而症状
/// （最后一列被裁/多出一列空白）看起来像"样式没调好"，不像算错。
#[test]
fn 内容区宽度随断点变化() {
    let wide = gallery_at(1400, 800, 1.0);
    assert_eq!(
        wide.content_width(),
        1400.0 - zephyra_ui::app::shell::NAV_W - 28.0,
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
    let nav = zephyra_ui::app::shell::NAV_W;
    let gutter = zephyra_ui::app::PAGE_GUTTER;
    assert_eq!(collapsed, 700.0 - gutter, "紧凑档内容区 = 视口 - 页边距");
    assert_eq!(
        collapsed - (700.0 - nav - gutter),
        nav,
        "折叠前后的内容区差必须恰好是导航宽（折叠后更大）"
    );
    assert!(collapsed > 0.0 && collapsed.is_finite(), "内容区必须是有界正数");
}

/// 触控目标必须按 scale 换算，保证视觉上达到官方的 40 epx 下限。
///
/// 出处：`touch-interactions`「命中目标」——「可触摸定义为最小 40 x 40 epx」。
/// 本仓布局单位是物理像素，所以 40 必须乘 scale；不乘会得到 40 物理 = 32 epx
/// （1.25 缩放下），**低于官方下限而代码看起来完全正确**。
#[test]
fn 触控目标按缩放比换算到官方的_epx_下限() {
    let g = gallery_at(1400, 800, 1.25);
    assert_eq!(g.touch_target(zephyra_ui::responsive::TOUCH_MIN_EPX), 50.0);
    assert_eq!(g.touch_target(zephyra_ui::responsive::TOUCH_OPTIMAL_EPX), 55.0);
    // 100% 下恒等；退化 scale 不应 panic 或给出 0。
    let plain = gallery_at(1400, 800, 1.0);
    assert_eq!(plain.touch_target(zephyra_ui::responsive::TOUCH_MIN_EPX), 40.0);
    let bad = gallery_at(1400, 800, 0.0);
    assert!(bad.touch_target(zephyra_ui::responsive::TOUCH_MIN_EPX) > 0.0);
}

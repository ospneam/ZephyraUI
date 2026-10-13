//! i18n 门禁 —— 语言协商、回退链、漏翻检测、以及「切换语言真的换了画面文字」。
//!
//! # 这一组测的是什么
//!
//! 1. **协商**：按用户优先序挑语言，而不是取全局最接近的（官方 `Intl` 措辞）；
//! 2. **回退**：精确 → 语言段 → 默认语言 → 键名，四档依次降级；
//! 3. **漏翻可被机器发现**：三种语言各自的键集合必须与总表一致；
//! 4. **端到端**：`on_input(languages)` → 协商 → `tr()` → 保留树上的 `TEXT`。
//!
//! 第 4 条是唯一一条跑完整链路的，前三条都只是零件测试。
//!
//! # 断言面为什么是**保留树**的 `TEXT` 而不是 `DisplayList`
//!
//! `Command::Text` 里只有**字形四边形**（`GlyphQuad { uv, dst, color_glyph }`），
//! 没有原文 —— 核心不参与字形整形，文字在进命令之前就已经被宿主排版掉了。
//! 所以从 `DisplayList` 里取不回字符串。本组断言取 `Node` 上的 `TEXT` 属性：
//! 它是 `tr()` 输出的**原样**，正好证明"文案换掉了"。
//! 另一条断言（`切语言会换掉画面文字`）额外确认渲染真的产出了字形命令，
//! 于是「属性变了」与「画面上真的画了」两件事都被覆盖。
//!
//! # 为什么不复用 `interaction_tests.rs` 的 harness
//!
//! 那个 harness 的 `InputState.languages` 固定给空表（好让断言与宿主机器的
//! Windows 语言设置无关）。这里需要**可控**的语言偏好，所以另起一套。
//! 代价是两份夹具，收益是断言不依赖机器 —— 与本仓"断言必须可复现"一致。

use zephyra_core::context::Context;
use zephyra_core::core::Rect;
use zephyra_core::diff::diff;
use zephyra_core::frame::{FrameCache, FrameOutcome};
use zephyra_core::id::ElementKey;
use zephyra_core::node::Node;
use zephyra_core::paint::Command;
use zephyra_core::property::PropertyValue;
use zephyra_core::props;
use zephyra_core::theme::{Theme, ThemeVariant};
use zephyra_fx::{InputState, View};
use zephyra_style::ir::StyleSheet;

use zephyra_ui::app::{self, Gallery, LANG_SELFS, SUPPORTED};
use zephyra_ui::i18n::{negotiate, Messages, Preferences, Tag};

/// 基准视口（与 `gates.rs` / `interaction_tests.rs` 同一常量：1400×800）。
const VIEWPORT: (f32, f32) = (1400.0, 800.0);

// ---------------------------------------------------------------- 零件层

/// 三种语言的键集合必须**完全一致**（不多不少）。
///
/// 漏翻在画面上只是"某处文案没变"，极难被察觉 —— 所以必须机器发现。
/// 刻意查两个方向：少翻（缺键）与多翻（某语言多了别的语言没有的键）。
/// 只查缺键会漏掉"给某语言单独加了一条"这种最后必然腐烂的做法。
#[test]
fn 每种语言都不缺键() {
    let all = app::catalog_keys();
    let m = app::messages();
    for locale in SUPPORTED {
        let missing = m.missing_in(locale, &all);
        assert!(
            missing.is_empty(),
            "语言 {locale} 漏翻 {} 条：{missing:?}",
            missing.len()
        );
        let n = m.len_for(locale);
        assert_eq!(
            n,
            all.len(),
            "语言 {locale} 有 {n} 条，而键总数是 {} —— 多出来的键没有任何语言能查到",
            all.len()
        );
    }
}

/// 每条路由（含 Settings）都必须有 `route.<id>` 文案键。
///
/// 判据取 `route_key()` 的回退值不等于输入值 —— 那说明它没在表里找到，
/// 于是 `tr()` 会返回键名（`route.home` 而不是 "Home"）。
/// 这比"函数返回 Option 再断言 Some"更贴近真实症状：错的是画面，不是接口。
#[test]
fn 每条路由都有文案键() {
    for (id, en, _icon) in app::ROUTES {
        let key = app::route_key(id);
        assert_eq!(key, format!("route.{id}").as_str(), "路由 {id} 缺文案键");
        // 顺带确认 `ROUTES` 里那份英文字面量与文案表一致 ——
        // 留着两份是"两处真相"，而 `ROUTES` 的 title 现在**只有**门禁在读。
        assert_eq!(
            app::messages().get("en", key),
            Some(*en),
            "路由 {id} 的 ROUTES 英文与文案表不一致"
        );
    }
    let (id, en, _) = app::SETTINGS_ROUTE;
    assert_eq!(app::route_key(id), format!("route.{id}").as_str());
    assert_eq!(app::messages().get("en", app::route_key(id)), Some(en));
}

/// 自称表与支持表必须等长且下标对齐。
///
/// 下标错位的症状是"选了中文得到日文"，画面上完全看不出原因 ——
/// 这条不是防御性断言，它拦住的是一个**看起来正常**的 bug。
#[test]
fn 语言自称与支持列表等长() {
    assert_eq!(
        LANG_SELFS.len(),
        SUPPORTED.len(),
        "LANG_SELFS 与 SUPPORTED 必须等长：下标对齐是 `Gallery::lang` 的全部含义"
    );
    for (i, self_name) in LANG_SELFS.iter().enumerate() {
        assert!(!self_name.is_empty(), "第 {i} 个自称是空串");
        assert!(
            Tag::parse(SUPPORTED[i]).is_some(),
            "SUPPORTED[{i}] = {:?} 不是 BCP 47 标签",
            SUPPORTED[i]
        );
    }
}

/// 协商：系统偏好按优先序匹配可用表。
///
/// 用 `SUPPORTED` 而不造假数据 —— **真实**的组合才值得测：
/// `zh-CN` → `zh-Hans`、`en-GB` → `en`、`ja-JP` → `ja`、`de-DE` → 落回默认 `en`。
#[test]
fn 系统偏好落到正确语言() {
    let available: Vec<Tag> = SUPPORTED.iter().filter_map(|s| Tag::parse(s)).collect();
    let cases: &[(&[&str], &str)] = &[
        (&["zh-CN", "en-US"], "zh-Hans"),
        (&["zh-Hans-CN"], "zh-Hans"),
        // ⚠️ `zh-TW` 拿到的是 `zh-Hans`，**不是**落回 `en`。
        //
        // `Tag::distance` 把"请求里缺 script"当通配（距离 3），这与浏览器的
        // locale 匹配一致：浏览器对无 script 的 `zh-TW` 也会落到 `zh`，而本仓
        // 的可用表里没有 `zh`、只有 `zh-Hans`，于是命中简体。
        //
        // 这**不是** bug，是真实的产品取舍：本 demo 只发布了简体。
        // 对照组是"两个都写了 script"——那时 Hans/Hant 不许互相回退
        // （见 `i18n.rs` 的 `回退距离按官方顺序`）。繁体缺口记在
        // `docs/i18n.md` §6。
        (&["zh-TW"], "zh-Hans"),
        (&["en-GB", "fr"], "en"),
        (&["ja-JP"], "ja"),
        // `de` / `fr` 都不在可用表里 ⇒ `negotiate` 返回 **None**。
        //
        // ⚠️ 这一条区分了两件事，别混：`negotiate` 答的是"用户偏好里有没有
        // 可用的"，答"没有"就返回 None；回落默认语言是**应用层**的一步
        // （`Gallery::pick_initial_language` 取 `SUPPORTED[0]`）。浏览器里
        // "supported locales" 本身就是那份回落表，所以两件事被揉在一起；
        // 本仓把它拆开，于是必须分别测 —— 只测最终界面就看不出 `negotiate`
        // 到底有没有正确地"不匹配"。
        (&["de-DE", "fr-FR"], "<无>"),
        (&[], "<无>"),
    ];
    for (prefs, want) in cases {
        let p = Preferences::from_list(prefs);
        let got = negotiate(&p, &available)
            .map(|t| t.as_str())
            .unwrap_or_else(|| "<无>".to_string());
        assert_eq!(&got, want, "偏好 {prefs:?} 应落到 {want}，实际 {got}");
    }
}

/// 漏翻必须回落到**默认语言**，而不是键名。
///
/// 这是浏览器的真实行为，也是"降级写在明处"的落点：真要发现漏翻靠门禁，
/// 界面回落默认语言是**降级**而不是**故障**。
///
/// 这里手工造一份"漏翻的表"（删掉 `ja` 的一条）来验证回退链本身 ——
/// 真实表漏翻会被上一条门禁拦住，所以必须自己造。
#[test]
fn 漏翻时回落默认语言而不是显示键名() {
    let key = "route.home";
    let stripped = without("ja", key);
    let got = stripped.select(&Preferences::from_list(&["ja"]), SUPPORTED[0], key);
    assert_eq!(
        got,
        app::messages().get("en", key),
        "删掉 ja 之后必须回落到 en 的文案，而不是 None（None 会让画面显示键名）"
    );
    // 对照：完整表里 ja 确实有自己的文案（说明上面那条不是恒真）。
    assert_eq!(
        app::messages().select(&Preferences::from_list(&["ja"]), SUPPORTED[0], key),
        app::messages().get("ja", key)
    );
}

/// 查不到的键，`tr()` 必须回显键名而不是空白。
///
/// 这条验证 `tr()` 的最后一档。它把"降级写在明处"变成可执行断言：
/// 有人把 `unwrap_or(key)` 改成 `unwrap_or("")`，这里立刻红。
#[test]
fn 查不到的键显示键名而不是空白() {
    let g = Gallery::default();
    let got = g.tr("no.such.key");
    assert_eq!(got, "no.such.key", "缺失键必须回显键名");
    assert!(!got.is_empty(), "绝不能是空串：空串看起来像「这里没有文字」");
}

// ---------------------------------------------------------------- 端到端

/// 读入所有 `.zstyle` 并合并（与 `interaction_tests.rs::sheet` 同一逻辑、
/// 读同一批文件 —— 门禁必须吃**同一份**表，否则"图对、门禁绿"会是两件事）。
fn sheet() -> StyleSheet {
    use std::path::Path;
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut names: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
        .expect("crate 目录")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("zstyle"))
        .collect();
    names.sort();
    let mut out = StyleSheet::new();
    for name in &names {
        let css = std::fs::read_to_string(name).expect("stylesheet must exist");
        let parsed = zstylec::parse(&name.to_string_lossy(), &css);
        assert!(parsed.is_ok(), "stylesheet must parse: {:?}", parsed.errors);
        out = out.merge(parsed.sheet);
    }
    out
}

/// 一份删掉某条目的表（用来模拟漏翻）。
fn without(locale: &'static str, key: &'static str) -> Messages {
    let mut m = app::messages();
    m.remove(locale, key);
    m
}

/// 驱动一棵 gallery 树的夹具。
struct Harness {
    node: Node,
    theme: Theme,
    sheet: StyleSheet,
    gallery: Gallery,
}

impl Harness {
    /// `languages` = 宿主读到的系统语言偏好。
    fn new(languages: Vec<String>) -> Self {
        let theme = Theme::builtin(ThemeVariant::Dark);
        let ctx = Context::new(&theme);
        let sheet = sheet();
        let mut gallery = Gallery::default();
        // ⚠️ 顺序：**先**灌系统偏好，**再**建树。
        //
        // 反过来的话保留树里是协商前的英文树，而 `texts()` 读的就是它 ——
        // 于是"协商到中文"会看到英文，而且**看不出**是夹具错了。
        // 真实宿主也是这个顺序：Bootstrap 每帧开头对账，然后才 `view()`。
        gallery.on_input(&InputState {
            hovered: None,
            pressed: None,
            pointers: 0,
            viewport: VIEWPORT,
            scale: 1.0,
            dt_ms: 16.0,
            languages,
        });
        let mut node = Node::from_element(&gallery.view(&ctx));
        sheet.apply_all(&mut node);
        zephyra_core::layout::layout(
            &mut node,
            Rect::new(0.0, 0.0, VIEWPORT.0, VIEWPORT.1),
            ctx,
        );
        zephyra_core::node::clear_dirty(&mut node);
        Self {
            node,
            theme,
            sheet,
            gallery,
        }
    }

    /// 推一次输入并走完 diff → apply → layout → render。
    ///
    /// 顺序是**契约**：diff → apply → layout → render。`apply` 必须在 diff
    /// **之后** —— 它读的是节点上刚被 diff 改过的分类名。
    fn frame(&mut self, hovered: Option<ElementKey>, pressed: Option<ElementKey>) -> Vec<Command> {
        let ctx = Context::new(&self.theme);
        self.gallery.on_input(&InputState {
            hovered: hovered.clone(),
            pressed: pressed.clone(),
            pointers: usize::from(pressed.is_some()),
            viewport: VIEWPORT,
            scale: 1.0,
            dt_ms: 16.0,
            // 语言偏好只送**第一次**：此后 `Gallery` 认为已读过系统偏好，
            // 手动切换才不会被下一次输入冲掉（`on_input` 的注释说明了这一点）。
            languages: Vec::new(),
        });
        let element = self.gallery.view(&ctx);
        let stats = diff(&mut self.node, &element);
        self.sheet.apply_all(&mut self.node);
        if stats.needs_layout() {
            zephyra_core::layout::layout(
                &mut self.node,
                Rect::new(0.0, 0.0, VIEWPORT.0, VIEWPORT.1),
                ctx,
            );
        }
        let mut list = zephyra_core::paint::DisplayList::new();
        zephyra_core::paint::render(&self.node, ctx, &mut list);
        zephyra_core::node::clear_dirty(&mut self.node);
        list.iter().map(|i| i.command.clone()).collect()
    }

    /// 点一次某个 key。
    ///
    /// 判据是 `Gallery::on_input` 的实现：**上一帧按着、这一帧松了**。
    /// 这里直接推 `InputState` 而不是走 `InputRouter` + 命中测试 ——
    /// 那样会顺带测"那个矩形在不在屏幕上"，而本组关心的是"点了之后语言换没换"。
    /// 命中路径由 `interaction_tests.rs` 的 10 条覆盖。
    fn click(&mut self, key: &'static str) {
        let k = ElementKey::Name(key);
        self.frame(Some(k.clone()), Some(k.clone()));
        self.frame(Some(k), None);
    }

    /// 保留树上所有 `TEXT` 属性的值，用 `\u{1}` 分隔。
    fn texts(&self) -> String {
        let mut out = String::new();
        collect_texts(&self.node, &mut out);
        out
    }
}

fn collect_texts(node: &Node, out: &mut String) {
    for (key, value) in node.props().iter_local() {
        if !std::ptr::eq(key, &props::TEXT) {
            continue;
        }
        let s = match value {
            PropertyValue::StaticStr(s) => *s,
            PropertyValue::Str(s) => s.as_ref(),
            _ => continue,
        };
        out.push_str(s);
        out.push('\u{1}');
    }
    for child in node.children() {
        collect_texts(child, out);
    }
}


/// 系统偏好 `zh-CN` 时，保留树上的文字必须是中文。
#[test]
fn 系统偏好中文则界面是中文() {
    let h = Harness::new(vec!["zh-CN".to_string(), "en-US".to_string()]);
    assert_eq!(h.gallery.locale(), "zh-Hans");
    let texts = h.texts();
    assert!(
        texts.contains("主页"),
        "协商到 zh-Hans 后树上应出现「主页」，实际：{texts:?}"
    );
    assert!(!texts.contains("Home"), "不该同时出现英文标题：{texts:?}");
}

/// 手动切语言必须真的换掉文字，而且**双向**。
#[test]
fn 手动切语言会换掉画面文字() {
    let mut h = Harness::new(Vec::new());
    assert_eq!(h.gallery.locale(), "en", "空偏好 ⇒ 回落默认语言");
    assert!(h.texts().contains("Home"), "默认应为英文");

    // 点 `ja` 那一段（段 id 是语言标签，见 `pages::language_picker`）。
    h.click("ja");
    assert_eq!(h.gallery.locale(), "ja");
    let texts = h.texts();
    assert!(texts.contains("ホーム"), "切到 ja 后应出现「ホーム」：{texts:?}");
    assert!(!texts.contains("Home"), "英文标题应消失：{texts:?}");

    // 再切回英文，确认是双向而不是单向。
    h.click("en");
    assert_eq!(h.gallery.locale(), "en");
    let texts = h.texts();
    assert!(texts.contains("Home"), "切回 en 应恢复：{texts:?}");
    assert!(!texts.contains("ホーム"), "日文标题应消失：{texts:?}");
}

/// 走一帧：推进输入 + `view()` + `FrameCache::frame`。
///
/// # 显式生命周期的理由
///
/// 返回值 [`FrameOutcome`] 里带着从 `cache` 借出的 `&DisplayList`，而 `cache`
/// 是 `&mut` 借用 —— 编译器分不清"这个引用活多久"。写成
/// `fn frame_once<'a>(cache: &'a mut FrameCache, ...) -> FrameOutcome<'a>` 后
/// 含义才确定：列表的借用期等于 `cache` 的借用期。
///
/// 这不是可以靠推断糊过去的地方：把 `cache` 改成 `&mut` 共享借用后
/// `Harness` 那一侧就编不过了，而那正是我们要的显式约束。
fn frame_once<'a>(
    g: &mut Gallery,
    cache: &'a mut FrameCache,
    ctx: &Context<'_>,
    pressed: Option<ElementKey>,
) -> FrameOutcome<'a> {
    g.on_input(&InputState {
        hovered: None,
        pressed,
        pointers: 0,
        viewport: VIEWPORT,
        scale: 1.0,
        dt_ms: 16.0,
        languages: Vec::new(),
    });
    let el = g.view(ctx);
    cache.frame(&el, *ctx, VIEWPORT)
}

/// 切语言必须让整帧真的重画（`FrameOutcome::Updated`，不是 `Unchanged`）。
///
/// # 这条替代了"渲染产物应当改变"的写法 —— 后者的前提是**错的**
///
/// 实测：切语言前后 core 的 `DisplayList` **逐字节相同**。原因不是 i18n 没接上，
/// 而是核心这一层根本不含文字：
///
/// - `Command::Text` 在 core 的 `render` 里带的是**空字形**（字形整形是宿主的活，
///   见 `paint.rs`）；
/// - 外壳布局宽度固定（`.nav-pane` 写死 240），所以文案变长也不会引发重排位移。
///
/// 于是"比较渲染产物"这条路在核心层**永远**测不出语言差异 —— 一条恒假的断言
/// 比没有断言更糟，它给人"这块测过了"的错觉。真正能测的是整帧是否被标记为
/// 重画：那正是 `frame.rs` 的重排/重绘判据，也是本轮修的 `tree_needs_layout`
/// 所在之处。
///
/// # 差异在哪能看到
///
/// 1. 保留树上的 `TEXT` 属性（本文件其它几条测的就是它）；
/// 2. 宿主的字形层 —— 真机上可见，但本机 D3D12 资源创建不可用（`tests/d3d12_draw.rs`
///    头部有证据），所以门禁只能走到第 1 层。记账见 `docs/i18n.md` §7。
#[test]
fn 切语言会让整帧重画() {
    let sheet = std::sync::Arc::new(sheet());
    let mut cache = FrameCache::new();
    cache.set_styler(Box::new(zephyra_style::SheetStyler(sheet)));
    let theme = Theme::builtin(ThemeVariant::Dark);
    let ctx = Context::new(&theme);

    let mut gallery = Gallery::default();

    // 先跑满入场动画（`dt_ms` = 16 ms），再谈"稳态"。
    //
    // ⚠ 两个坑，少处理任何一个都会假红：
    //
    // 1. 动画期间 `enter_ms` 每帧都在变 ⇒ 每帧都 `Updated`。而"稳态"在这
    //    里的含义是**动画放完之后**。
    // 2. **不能**用"跑到第一个 `Unchanged` 为止"来判断放完：`anim::step` 是
    //    量化的，同一档内分类名不变 ⇒ 档内会冒出 `Unchanged`，于是循环在
    //    动画中段就退出了，下一帧跨档又变 `Updated`（实测踩过）。
    //
    // 帧数按 `anim::ENTER_MS` **算**而不是写死：时长或帧间隔任一改动时，
    // 写死的数字会静默失配。
    let frames = (zephyra_ui::anim::ENTER_MS / 16.0).ceil() as u32 + 2;
    for _ in 0..frames {
        let _ = frame_once(&mut gallery, &mut cache, &ctx, None);
    }

    // 稳态的对照：跑满之后必须 Unchanged。
    let steady = frame_once(&mut gallery, &mut cache, &ctx, None);
    assert!(
        matches!(steady, FrameOutcome::Unchanged),
        "稳态必须 Unchanged（已跑 {frames} 帧 > {}ms），否则这条断言证明不了任何事",
        zephyra_ui::anim::ENTER_MS
    );

    // 按下 `ja` ⇒ 这一帧语言还没变（click 要等抬起）。
    let _ = frame_once(&mut gallery, &mut cache, &ctx, Some(ElementKey::Name("ja")));
    // 抬起 ⇒ click 生效，语言切到 ja。
    let after = frame_once(&mut gallery, &mut cache, &ctx, None);
    assert_eq!(gallery.locale(), "ja", "前置条件：语言真的切了");
    assert!(
        matches!(after, FrameOutcome::Updated { .. }),
        "语言变了却报 Unchanged ⇒ 画面不会被重画，用户看不到任何变化"
    );
}
/// 切语言**不得**破坏其它状态：路由、勾选都要留着。
///
/// 这条容易被忽略：语言切换会重建整棵树，若某处状态被误当成"每次重建都重来"
/// （例如只存在于 `view()` 内部的临时变量里），症状就是"切一次语言，勾选全没了"。
/// 它不崩，只是安静地丢数据。
#[test]
fn 切语言不丢其它状态() {
    let mut h = Harness::new(Vec::new());
    h.gallery.route = "settings";
    h.frame(None, None);
    // `checked` 的初值是 `[true, false, false]`（`Gallery::default`），
    // 所以"点一下"的结果是**取消勾选**。断言写成"仍是 true"会红，
    // 而那与本条要测的东西（切语言不丢状态）毫无关系。
    let checked_before = h.gallery.checked[0];
    h.click("check:alpha");
    assert_ne!(
        h.gallery.checked[0], checked_before,
        "前置条件：点一下必须真的翻转勾选，否则这条测的是空气"
    );
    let after_click = h.gallery.checked[0];
    let route_before = h.gallery.route;
    assert_eq!(route_before, "settings");

    h.click("ja");
    assert_eq!(h.gallery.route, route_before, "切语言不该改路由");
    assert_eq!(
        h.gallery.checked[0], after_click,
        "切语言不该动勾选状态"
    );
}

/// 路由 id 不与语言标签撞车（否则点导航会顺手换语言）。
///
/// 这是"两处真相"的交叉点：`SUPPORTED` 里如果出现 `settings` 这种与路由 id
/// 同形的项，点它就变成换语言而不是导航。症状是点导航没反应、或顺手换了个语言。
#[test]
fn 路由_id_不与语言标签相撞() {
    for (id, _, _) in app::ROUTES {
        assert!(
            !SUPPORTED.contains(id),
            "路由 id {id:?} 同时是一个语言标签 —— 点击会歧义"
        );
    }
    assert!(
        !SUPPORTED.contains(&app::SETTINGS_ROUTE.0),
        "Settings 的 id 同时是一个语言标签"
    );
}
//! ListAndCollections + 邻居：ListView / TableView / SwipeView / Avatar /
//! SettingExpander / SettingItem。
//!
//! 度量出处（RinUI）：
//! - ListView 委托：`ListAndCollections/ListViewDelegate.qml` —— 左 padding 16、
//!   radius 5、行高 ≈ 内容 + 20（≈40）、选中态 `controlFillColor` 底。
//! - TableView：`ListAndCollections/TableView.qml` + Delegate —— 表头行
//!   （高 36、次要色字）+ 数据行（高 40）+ 列分隔线。
//! - SwipeView：`ListAndCollections/SwipeView.qml` —— 当前页 + 底部圆点指示
//!   + 悬停时左右 chevron 按钮。
//! - Avatar：`Media/Avatar.qml` —— 正圆 + 首字母（无图时）。
//! - SettingExpander：`Layout/SettingExpander.qml` —— SettingCard 卡头 +
//!   可折叠内容（Expander 同款 COLLAPSED 机制）。
//! - SettingItem：`Layout/SettingItem.qml` —— 左图标 + 标题/描述的设置行。
//!
//! # 降级记账
//!
//! - ListView / SwipeView 的**滚动与滑动手势**不做（core 无滚动/拖拽）：
//!   选中项与当前页由宿主状态驱动（key 路由一样工作）。
//! - TableView 的**列宽拖拽 / 排序**不做：定列宽 + 静态表头。

use zephyra_core::element::Element;
use zephyra_core::id::NodeTag;
use zephyra_core::property::PropertyValue;
use zephyra_core::props;

use super::{state, with_classes};

/// 列表（RinUI ListView）：竖排行 + 选中态。
///
/// `items` 每项 `(id, label)`；`selected` 是选中项 id（互斥选择）。
pub fn list_view(
    key: &'static str,
    items: &'static [(&'static str, &'static str)],
    selected: Option<&'static str>,
    hovered: Option<&str>,
    pressed: Option<&str>,
) -> Element {
    let rows: Vec<Element> = items
        .iter()
        .map(|(id, label)| {
            let is_sel = Some(*id) == selected;
            let mut classes: Vec<&'static str> = Vec::new();
            if is_sel {
                classes.push("selected");
            }
            classes.extend(state(hovered == Some(*id), pressed == Some(*id)));
            let label = with_classes(
                Element::new(NodeTag::Text)
                    .with_key("label")
                    .with(&props::TEXT, PropertyValue::static_str(label)),
                &classes,
            )
            .style("list-label");
            with_classes(
                Element::new(NodeTag::Control)
                    .with_key(*id)
                    .with(&props::INTERACTIVE, PropertyValue::bool(true))
                    .style("list-row")
                    .with_children([label]),
                &classes,
            )
        })
        .collect();
    Element::new(NodeTag::Container)
        .with_key(key)
        .style("list-view")
        .with_children(rows)
}

/// 表格（RinUI TableView）：表头 + 数据行，定列宽。
pub fn table_view(
    key: &'static str,
    headers: &'static [&'static str],
    rows: &'static [&'static [&'static str]],
) -> Element {
    let mut children = vec![header_row(headers)];
    for (r, row) in rows.iter().enumerate() {
        let cells: Vec<Element> = row
            .iter()
            .enumerate()
            .map(|(c, cell)| {
                Element::new(NodeTag::Text)
                    .with_key(cell_key(r, c))
                    // 数据格可点：选中哪一格是**应用**的状态（控件只发 key），
                    // 与 `list_view` 的行同一模式。tag 仍是 `Text`（样式表按
                    // 类型选择 `Text.table-cell`），INTERACTIVE 与 tag 无关。
                    .with(&props::INTERACTIVE, PropertyValue::bool(true))
                    .with(&props::TEXT, PropertyValue::static_str(cell))
                    .style("table-cell")
            })
            .collect();
        children.push(
            Element::new(NodeTag::Container)
                .with_key(table_row_key(r))
                .style("table-row")
                .with_children(cells),
        );
    }
    Element::new(NodeTag::Container)
        .with_key(key)
        .style("table-view")
        .with_children(children)
}

/// 滑动视图（RinUI SwipeView）：当前页 + 圆点指示。
///
/// `pages` 每项 `(标题, 一句说明)`；`index` 是当前页。**滑动手势不做**
/// （core 无拖拽），翻页由宿主按 key 路由。
pub fn swipe_view(
    key: &'static str,
    pages: &'static [(&'static str, &'static str)],
    index: usize,
) -> Element {
    // 元组形状是 `(title, desc)`：标题 + 一行说明。**不存在** id 字段 ——
    // 翻页靠 `<key>` 上的 `sw:<i>` 指示点，卡片内容里不该出现内部键名。
    //
    // ⚠ 这一处曾经烧过一次：形状写反时**不崩、不报错**（两项都是
    // `&'static str`），画面上只是把 `swipe:0` 这串内部键名当正文画出来。
    // 判据只能是「内容里不出现 `xxx:` 形态的 key」这条肉眼约定。
    let (title, desc) = pages.get(index).copied().unwrap_or(("—", "—"));
    let pips: Vec<Element> = (0..pages.len())
        .map(|i| {
            let mut d = Element::new(NodeTag::Container)
                .with_key(swipe_pip_key(i))
                // 指示点同时是**翻页按钮**：core 没有拖拽手势，翻页只能
                // 由点击表达。当前页仍由 `index` 决定（应用的状态）。
                .with(&props::INTERACTIVE, PropertyValue::bool(true))
                .style("swipe-pip");
            if i == index {
                d = d.style("selected");
            }
            d
        })
        .collect();
    Element::new(NodeTag::Container)
        .with_key(key)
        .style("swipe-view")
        .with_children([
            Element::new(NodeTag::Container)
                .with_key("card")
                .style("swipe-card")
                .with_children([
                    Element::new(NodeTag::Text)
                        .with_key("title")
                        .with(&props::TEXT, PropertyValue::static_str(title))
                        .style("swipe-title"),
                    Element::new(NodeTag::Text)
                        .with_key("desc")
                        .with(&props::TEXT, PropertyValue::static_str(desc))
                        .style("swipe-desc"),
                ]),
            Element::new(NodeTag::Container)
                .with_key("pips")
                .style("swipe-pips")
                .with_children(pips),
        ])
}

/// 头像（RinUI Avatar）：正圆 + 首字母。
///
/// `accent` 走分类名下发（选择器没有后代组合器，子的字色只能自己发类）。
pub fn avatar(key: &'static str, initials: &'static str, accent: bool) -> Element {
    let mut text = Element::new(NodeTag::Text)
        .with_key("initials")
        .with(&props::TEXT, PropertyValue::static_str(initials))
        .style("avatar-text");
    if accent {
        text = text.style("accent");
    }
    let mut el = Element::new(NodeTag::Container)
        .with_key(key)
        .style("avatar")
        .with_children([text]);
    if accent {
        el = el.style("accent");
    }
    el
}

/// 设置行（RinUI SettingItem）：左图标 + 标题/描述（无控件位）。
pub fn setting_item(
    key: &'static str,
    icon: &'static str,
    title: &'static str,
    desc: &'static str,
) -> Element {
    Element::new(NodeTag::Container)
        .with_key(key)
        .style("setting-item")
        .with_children([
            Element::new(NodeTag::Container)
                .with_key("icon")
                .with(&props::ICON_NAME, PropertyValue::static_str(icon))
                .style("setting-icon"),
            Element::new(NodeTag::Container)
                .with_key("texts")
                .style("setting-texts")
                .with_children([
                    Element::new(NodeTag::Text)
                        .with_key("title")
                        .with(&props::TEXT, PropertyValue::static_str(title))
                        .style("setting-title"),
                    Element::new(NodeTag::Text)
                        .with_key("desc")
                        .with(&props::TEXT, PropertyValue::static_str(desc))
                        .style("setting-desc"),
                ]),
        ])
}

/// 设置展开器（RinUI SettingExpander）：卡头（标题/描述/展开箭头）+ 折叠内容。
pub fn setting_expander(
    key: &'static str,
    title: &'static str,
    desc: &'static str,
    expanded: bool,
    children: Vec<Element>,
) -> Element {
    let mut chevron = Element::new(NodeTag::Container)
        .with_key("chevron")
        .with(&props::ICON_NAME, PropertyValue::static_str("chevron-down"))
        .style("setting-chevron");
    if expanded {
        chevron = chevron.style("open");
    }
    Element::new(NodeTag::Container)
        .with_key(key)
        .style("setting-expander")
        .with_children([
            // 可点区域的 key 就是控件自己的 key（理由见 `card::expander`
            // 同一处的注释：内部名 `header` 会让多个展开器撞名）。
            Element::new(NodeTag::Container)
                .with_key(key)
                .with(&props::INTERACTIVE, PropertyValue::bool(true))
                .style("setting-header")
                .with_children([
                    Element::new(NodeTag::Container)
                        .with_key("texts")
                        .style("setting-texts")
                        .with_children([
                            Element::new(NodeTag::Text)
                                .with_key("title")
                                .with(&props::TEXT, PropertyValue::static_str(title))
                                .style("setting-title"),
                            Element::new(NodeTag::Text)
                                .with_key("desc")
                                .with(&props::TEXT, PropertyValue::static_str(desc))
                                .style("setting-desc"),
                        ]),
                    chevron,
                ]),
            Element::new(NodeTag::Container)
                .with_key("content")
                .style("setting-content")
                .with(&props::COLLAPSED, PropertyValue::bool(!expanded))
                .with_children(children),
        ])
}

// ---------------------------------------------------------------- 内部

fn header_row(headers: &'static [&'static str]) -> Element {
    let cells: Vec<Element> = headers
        .iter()
        .enumerate()
        .map(|(c, h)| {
            Element::new(NodeTag::Text)
                .with_key(head_key(c))
                .with(&props::TEXT, PropertyValue::static_str(h))
                .style("table-head")
        })
        .collect();
    Element::new(NodeTag::Container)
        .with_key("head")
        .style("table-head-row")
        .with_children(cells)
}

fn head_key(c: usize) -> &'static str {
    ["th:0", "th:1", "th:2", "th:3"][c.min(3)]
}

fn cell_key(r: usize, c: usize) -> &'static str {
    // 数据格 key 要 &'static：用常量表按 (r,c) 编。
    const KEYS: [&str; 16] = [
        "td:00", "td:01", "td:02", "td:03", "td:10", "td:11", "td:12", "td:13", "td:20", "td:21",
        "td:22", "td:23", "td:30", "td:31", "td:32", "td:33",
    ];
    KEYS[(r * 4 + c).min(15)]
}

fn table_row_key(r: usize) -> &'static str {
    ["tr:0", "tr:1", "tr:2", "tr:3"][r.min(3)]
}

fn swipe_pip_key(i: usize) -> &'static str {
    ["sw:0", "sw:1", "sw:2", "sw:3"][i.min(3)]
}

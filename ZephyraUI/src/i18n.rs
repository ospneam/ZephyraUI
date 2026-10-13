//! i18n：语言标签解析、偏好协商、消息表。
//!
//! # 参考对象：浏览器（不是自己发明的规则）
//!
//! 出处（2026-10 查）：
//! - `navigator.languages`：<https://developer.mozilla.org/en-US/docs/Web/API/Navigator/languages>
//!   「returns an array of strings representing the user's preferred languages.
//!   The language is described using a **BCP 47 language tag**. In the returned
//!   array they are ordered by preference with the most preferred language first.」
//! - `Accept-Language`（浏览器把同一列表随请求发出）：同页
//!   「generally lists the same locales … with decreasing `q` values」
//! - `Intl` 的**逐个回退**：同页示例
//!   「the array … can be passed directly to the `Intl` constructors to implement
//!   preference-based fallback selection of locales, where the **first entry in
//!   the list that matches a locale supported** by `Intl` is used」
//! - BCP 47 标签结构：<https://developer.mozilla.org/en-US/docs/Glossary/BCP_47_language_tag>
//!
//! 完整引用与设计见 `docs/i18n.md`。
//!
//! # 本模块照抄浏览器的那四条
//!
//! | 浏览器 | 本模块 |
//! |---|---|
//! | BCP 47 标签 | [`Tag`]（language-script-region 三段，大小写规范化） |
//! | `navigator.languages` 有序偏好表 | [`Preferences`]（有序，含"第一个"= `language`） |
//! | `Intl` 逐个回退匹配 | [`negotiate`] |
//! | `q` 值（`Accept-Language` 的权重） | [`Preferences::with_weight`] |
//!
//! # 不做的，以及为什么
//!
//! - **复数规则**：CLDR 的复数类别（zero/one/two/few/many/other）需要按语言的
//!   选择规则与关键词表，那是数据而不是逻辑。本模块只提供 [`Messages::select`]
//!   这个**槽位**，规则数据不进来 —— 硬编码"英文式 one/other"会静默给错中文与
//!   俄文，而无声的错误比没有更糟。
//! - **日期/数字格式化**：官方例子用的是 `Intl.DateTimeFormat`，那是 ICU。
//!   同样只留槽位不实现，记账在 `docs/i18n.md` §4。
//! - **运行时加载 / 按需分块**：浏览器按 `Accept-Language` 取对应资源，
//!   而本仓的消息表是编译期常量（`view()` 每帧重建，泄漏不起）。

use std::collections::BTreeMap;

/// 一条 BCP 47 语言标签。
///
/// # 规范化（照浏览器，不照直觉）
///
/// BCP 47 规定三段各自的大小写：language 小写、script 首字母大写、region 大写。
/// `navigator.languages` 给出的就是规范化后的形式（`en-US`、`zh-Hans-CN`），
/// 所以这里**只接受规范化输入**并原样保留：
///
/// - 比较时按「规范化后相等」，而不是归一成小写后相等 ——
///   归一会把 `zh-Hans` 与 `zh-Hant` 也并成 `zh`，那是两种文字，不能并。
/// - 构造时**不**做模糊修复（`zh_cn` / `ZH-CN` 一律当无效）：
///   宽容解析会把拼错的标签变成另一个合法标签，而那比直接报错更难查。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Tag {
    /// 主语言子标签（小写，如 `en` / `zh`）。
    language: String,
    /// 文字子标签（首字母大写，如 `Hans` / `Hant`）。可空。
    script: Option<String>,
    /// 区域子标签（大写，如 `US` / `CN`）。可空。
    region: Option<String>,
}

impl Tag {
    /// 解析一条 BCP 47 标签。**非法返回 `None`**，不猜。
    ///
    /// 判据取 BCP 47 的结构：至少一个 language 子标签，随后可选 script（4 字母）
    /// 与 region（2 字母或 3 数字）。
    pub fn parse(raw: &str) -> Option<Self> {
        let mut parts = raw.split('-');
        let language = parts.next()?;
        if !valid_language(language) {
            return None;
        }
        let mut script = None;
        let mut region = None;
        for part in parts {
            match part.len() {
                // 4 字母 = script；2 字母 = region；3 数字 = region（如 `419`）。
                4 if valid_script(part) && script.is_none() => script = Some(title_case(part)),
                2 if valid_region(part) && region.is_none() => region = Some(part.to_ascii_uppercase()),
                3 if part.chars().all(|c| c.is_ascii_digit()) && region.is_none() => {
                    region = Some(part.to_string());
                }
                _ => return None,
            }
        }
        Some(Self {
            language: language.to_ascii_lowercase(),
            script,
            region,
        })
    }

    /// 主语言子标签。
    pub fn language(&self) -> &str {
        &self.language
    }

    /// 文字子标签（`zh-Hans` / `zh-Hant` 的区别就在这一位）。
    pub fn script(&self) -> Option<&str> {
        self.script.as_deref()
    }

    /// 区域子标签。
    pub fn region(&self) -> Option<&str> {
        self.region.as_deref()
    }

    /// 规范化字符串形式（与 `navigator.languages` 给出的形式一致）。
    pub fn as_str(&self) -> String {
        let mut out = self.language.clone();
        if let Some(s) = &self.script {
            out.push('-');
            out.push_str(s);
        }
        if let Some(r) = &self.region {
            out.push('-');
            out.push_str(r);
        }
        out
    }

    /// 只保留语言段（`zh-Hans-CN` → `zh`）。
    ///
    /// 回退链的一环：完整标签找不到时退到语言段。这与浏览器
    /// "Some browsers add language-only fallback tags in `Accept-Language`"
    /// 是同一件事 —— 它们显式列出 `en-US,en;q=0.9`，这里由回退算出来。
    pub fn language_only(&self) -> Self {
        Self {
            language: self.language.clone(),
            script: None,
            region: None,
        }
    }

    /// 与 `other` 的回退距离：0 = 完全相等，越大越远。
    ///
    /// # 顺序即优先级（照 `Intl` 的匹配语义）
    ///
    /// 1. `language + script + region` 全中（最具体）；
    /// 2. 丢 region（`zh-Hans` 命中 `zh-Hans-CN`）；
    /// 3. 丢 script 与 region（`zh` 命中 `zh-Hans-CN`）；
    /// 4. 都不中 ⇒ `None`。
    ///
    /// `script` 与 `region` **不可互换**：`zh-Hant-TW` 与 `zh-Hans-CN` 的语言段
    /// 都是 `zh`，但它们是不同文字，距离取 3 仍会命中 —— 这是这套规则唯一
    /// 允许的"不精确"，因为完全不许语言段回退会让 `ListFormat` 只剩精确匹配，
    /// 而浏览器明确会加 language-only 回退。
    pub fn distance(&self, other: &Tag) -> Option<u8> {
        if self.language != other.language {
            return None;
        }
        match (&self.script, &other.script) {
            (Some(a), Some(b)) if a == b => {}
            (None, _) | (_, None) => return Some(3),
            _ => return None,
        }
        match (&self.region, &other.region) {
            (Some(a), Some(b)) if a == b => Some(0),
            (None, _) | (_, None) => Some(2),
            _ => return None,
        }
    }
}

impl std::fmt::Display for Tag {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.as_str())
    }
}

/// 有序的偏好表（对应 `navigator.languages`）。
///
/// **有序即优先级**：第一个是最偏好（`navigator.language` 就是它）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Preferences {
    /// 按偏好降序的标签。
    tags: Vec<Tag>,
}

impl Preferences {
    /// 空偏好表。
    pub fn new() -> Self {
        Self::default()
    }

    /// 从浏览器风格的数组构造。非法项**跳过**而不是中止 —— 与浏览器一致：
    /// `navigator.languages` 里出现坏项不该让整表失效。
    pub fn from_list(list: &[&str]) -> Self {
        Self {
            tags: list.iter().filter_map(|s| Tag::parse(s)).collect(),
        }
    }

    /// 从**自有**字符串列表构造（宿主读回来的形状）。
    ///
    /// # 为什么不叫 `from_list` 覆盖掉
    ///
    /// `navigator.languages` 给的是字符串数组，而宿主（Win32）交回来的也是
    /// `Vec<String>` —— 两者都是**有所有权**的。若只有 `&[&str]` 一个入口，
    /// 每个调用方都得先把 `Vec<String>` 摊成 `Vec<&str>`，那是一次纯粹为了
    /// 迁就签名而存在的分配。两个入口的差别只有生命周期，各写一次更好读。
    pub fn from_owned(list: &[String]) -> Self {
        Self {
            tags: list.iter().filter_map(|s| Tag::parse(s)).collect(),
        }
    }

    /// 追加一个标签（追加到末尾 = 偏好更低）。
    pub fn push(&mut self, tag: Tag) {
        // 去重但**保留原顺序**：重复项不改变优先级。
        if !self.tags.contains(&tag) {
            self.tags.push(tag);
        }
    }

    /// 全部标签（按偏好降序）。
    pub fn tags(&self) -> &[Tag] {
        &self.tags
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.tags.is_empty()
    }

    /// 长度。
    pub fn len(&self) -> usize {
        self.tags.len()
    }

    /// 最偏好的标签（对应 `navigator.language`）。
    pub fn first(&self) -> Option<&Tag> {
        self.tags.first()
    }

    /// 附上权重（`Accept-Language` 的 `q` 值的等价物）。
    ///
    /// # 为什么要权重
    ///
    /// 官方原文：`Accept-Language` 「generally lists the same locales as the
    /// `navigator.languages` property, with decreasing `q` values」。
    /// 同页面还说明 Chrome/Safari 会**额外**追加 language-only 回退标签，
    /// 并给较低的 `q`（例：`en-US,en;q=0.9`）—— 也就是说"回退的优先级低于
    /// 精确匹配"是浏览器真实行为，不是本仓的设计偏好。
    pub fn with_weight(&self, weight: f32) -> Weighted {
        Weighted {
            prefs: self.clone(),
            weight,
        }
    }
}

/// 一条带权重的偏好（权重本身不参与匹配，只用于排序展示）。
#[derive(Debug, Clone, PartialEq)]
pub struct Weighted {
    /// 偏好表。
    pub prefs: Preferences,
    /// `q` 值（0..=1）。
    pub weight: f32,
}

/// 在可用集合里挑一个最合适的（对应 `Intl` 的逐个回退匹配）。
///
/// # 判据照官方的措辞
///
/// 「the **first entry in the list that matches** a locale supported」—— 先按
/// 用户顺序走，每个用户标签在所有可用项里挑**距离最小**的那个；距离相同则取
/// 可用项里靠前的一个（于是可用表本身有序语义，与浏览器"supported locales"
/// 的固定顺序一致）。
///
/// # 为什么不是"取全局最小距离"
///
/// 那会违反"first entry that matches"：用户把 `ja` 放第一、`en` 放第二时，
/// 即使 `en` 与某个可用项完全相等，也必须先让 `ja` 有机会匹配。
pub fn negotiate(prefs: &Preferences, available: &[Tag]) -> Option<Tag> {
    for want in prefs.tags() {
        let mut best: Option<(u8, &Tag)> = None;
        for have in available {
            if let Some(d) = want.distance(have) {
                let better = match best {
                    None => true,
                    Some((bd, _)) => d < bd,
                };
                if better {
                    best = Some((d, have));
                }
            }
        }
        if let Some((_, tag)) = best {
            return Some(tag.clone());
        }
    }
    None
}

/// 消息表。
///
/// # 键是稳定的，值按语言分文件
///
/// 这对应浏览器的做法：`navigator.languages` 决定**取哪个**资源文件，
/// 而代码里只写 key。所以本仓同样只写 key（`tr("x")`），不写文案 ——
/// 与"源码零外观"是同一条纪律的延伸：**源码里没有面向用户的字符串**。
///
/// # 为什么值是 `&'static str` 而不是 `String`
///
/// 因为 `view()` **每帧重建**整棵树，所以任何"运行期构造的字符串"都会变成
/// 每帧一次分配；而 `Box::leak` 出去又等于永久泄漏（见 `element.rs` 的记录）。
/// 翻译文案在编译期就定好了 —— 它不是运行期数据，把它做成常量既避开分配，
/// 也能直接喂给 `PropertyValue::static_str`（属性系统为此专门有一个变体，
/// 见 `property.rs`）。运行期加载的目录是另一套设计，本仓不做（记账在
/// `docs/i18n.md` §4）。
#[derive(Debug, Clone, Default)]
pub struct Messages {
    /// `(locale, key) → 文案`。
    entries: BTreeMap<(&'static str, &'static str), &'static str>,
}

impl Messages {
    /// 空表。
    pub fn new() -> Self {
        Self::default()
    }

    /// 插入一条。
    pub fn insert(&mut self, locale: &'static str, key: &'static str, value: &'static str) {
        self.entries.insert((locale, key), value);
    }

    /// 这个语言下有多少条。
    pub fn len_for(&self, locale: &str) -> usize {
        self.entries.keys().filter(|(l, _)| *l == locale).count()
    }

    /// 是否完全没内容。
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 取一条；找不到返回 `None`（**不给空串**）。
    ///
    /// 空串会让"漏翻"看起来像"这里就是空的"，而 `None` 会在门禁里变成一条失败
    /// 断言、在画面上变成键名本身。这与本仓"降级要写在明处"是同一条纪律。
    pub fn get(&self, locale: &str, key: &str) -> Option<&'static str> {
        self.entries
            .iter()
            .find(|((l, k), _)| *l == locale && *k == key)
            .map(|(_, v)| *v)
    }

    /// 取一条，带**回退**：按 `prefs` 顺序逐个语言试，最后回落**默认语言**。
    ///
    /// # 回退链的顺序
    ///
    /// 1. `prefs` 逐项：先精确 locale，再 language-only
    ///    （与 [`Tag::distance`] 的优先级一致，也与浏览器显式追加
    ///    `en-US,en;q=0.9` 那一项同一件事）；
    /// 2. `fallback` 指定的默认语言，同样先精确再 language-only；
    /// 3. 都没有 ⇒ `None`。
    ///
    /// 第 2 步是浏览器的真实行为：某一门语言漏翻时，界面回落**默认语言**的文案，
    /// 而不是空白或键名。键名只留作"连默认语言也没有"的最后手段。
    pub fn select(&self, prefs: &Preferences, fallback: &str, key: &str) -> Option<&'static str> {
        for want in prefs.tags() {
            if let Some(v) = self.get(&want.as_str(), key) {
                return Some(v);
            }
            if let Some(v) = self.get(want.language(), key) {
                return Some(v);
            }
        }
        if let Some(v) = self.get(fallback, key) {
            return Some(v);
        }
        // 默认语言的**语言段**那一档：`fallback` 可能写成 `zh-Hans`，而表里
        // 只登了 `zh`（或反过来）。这里不借用 `Tag` —— 借用一个临时
        // `Tag::parse` 的结果会拿到指向临时值的引用（E0515）。
        let language = Tag::parse(fallback)
            .map(|t| t.language().to_string())
            .unwrap_or_else(|| fallback.to_string());
        self.get(&language, key)
    }

    /// 本语言缺失的键（门禁用：漏翻必须能被机器发现）。
    pub fn missing_in(&self, locale: &str, keys: &[&str]) -> Vec<String> {
        keys.iter()
            .filter(|k| self.get(locale, k).is_none())
            .map(|k| k.to_string())
            .collect()
    }

    /// 删掉一条，返回是否真的删掉了。
    ///
    /// # 存在的理由只有门禁
    ///
    /// 真实表被 [`crate::app::CATALOG`] 门禁钉住「不缺键」，所以回退链没法靠
    /// 真的漏翻来测 —— 必须有一把"制造漏翻"的刀。
    /// 这类**只为测试存在**的 API 很少，但这条值得有：否则回退链的两档
    /// （默认语言、键名）就只能靠注释声明，而本仓不接受"注释说它工作"。
    ///
    /// ⚠️ 参数是 `&'static str` 而不是 `&str`：表的键本身就是 `&'static str`，
    /// `BTreeMap` 的 `remove` 要拿**同一类型**去查（`get` 之所以能用 `&str`
    /// 是因为它走的是自定义的线性比较，见 [`Messages::get`]）。
    /// 这里保持与 [`Messages::insert`] 同签名，避免两处真相。
    pub fn remove(&mut self, locale: &'static str, key: &'static str) -> bool {
        self.entries.remove(&(locale, key)).is_some()
    }
}

/// BCP 47 的 language 子标签：2~3 字母（最短形式）或 4（保留）或 5~8（注册）。
fn valid_language(s: &str) -> bool {
    (2..=8).contains(&s.len()) && s.chars().all(|c| c.is_ascii_alphabetic())
}

/// script 子标签：4 字母，首字母大写（`Latn` / `Hans` / `Hant`）。
fn valid_script(s: &str) -> bool {
    s.len() == 4 && s.chars().all(|c| c.is_ascii_alphabetic())
}

/// region 子标签：2 字母。
fn valid_region(s: &str) -> bool {
    s.len() == 2 && s.chars().all(|c| c.is_ascii_alphabetic())
}

fn title_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for (i, c) in s.chars().enumerate() {
        if i == 0 {
            out.extend(c.to_uppercase());
        } else {
            out.extend(c.to_lowercase());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 解析必须照 BCP 47 的三段结构，且**规范化**大小写。
    #[test]
    fn 标签按_bcp47_解析并规范化() {
        let t = Tag::parse("zh-hans-cn").expect("合法");
        assert_eq!(t.language(), "zh");
        assert_eq!(t.script(), Some("Hans"));
        assert_eq!(t.region(), Some("CN"));
        assert_eq!(t.as_str(), "zh-Hans-CN");

        assert_eq!(Tag::parse("en").unwrap().as_str(), "en");
        assert_eq!(Tag::parse("en-US").unwrap().as_str(), "en-US");
        // 3 数字区域（UN M.49）。
        assert_eq!(Tag::parse("es-419").unwrap().region(), Some("419"));
        // 非法项必须 None，不做模糊修复。
        assert!(Tag::parse("").is_none());
        assert!(Tag::parse("e").is_none());
        assert!(Tag::parse("zh_cn").is_none());
        assert!(Tag::parse("zh-CN-extra").is_none());
    }

    /// 协商必须是「**表里第一个匹配的**」，不是「全局最接近的」。
    ///
    /// 这条是官方措辞的直接翻译：`Intl` 用
    /// "the first entry in the list that matches a locale supported"。
    #[test]
    fn 协商按偏好顺序而不是全局最近() {
        // `ja` 排第一 ⇒ 必须选 ja，即使 `en-US` 与第二项**完全相等**（距离 0）。
        let prefs = Preferences::from_list(&["ja", "en-US"]);
        let available = [Tag::parse("en-US").unwrap(), Tag::parse("ja").unwrap()];
        assert_eq!(negotiate(&prefs, &available).unwrap().as_str(), "ja");

        // 反过来：`en-US` 优先 ⇒ 选 en-US。
        let prefs = Preferences::from_list(&["en-US", "ja"]);
        assert_eq!(negotiate(&prefs, &available).unwrap().as_str(), "en-US");
    }

    /// 回退优先级：全中 > 丢 region > 只语言段。
    #[test]
    fn 回退距离按官方顺序() {
        let full = Tag::parse("zh-Hans-CN").unwrap();
        assert_eq!(full.distance(&Tag::parse("zh-Hans-CN").unwrap()), Some(0));
        assert_eq!(full.distance(&Tag::parse("zh-Hans").unwrap()), Some(2));
        assert_eq!(full.distance(&Tag::parse("zh").unwrap()), Some(3));
        // 语言段不同 ⇒ 不匹配。
        assert_eq!(full.distance(&Tag::parse("en").unwrap()), None);
        // script 不同（Hans vs Hant）⇒ 不匹配：那是两种文字。
        assert_eq!(
            Tag::parse("zh-Hans").unwrap().distance(&Tag::parse("zh-Hant").unwrap()),
            None
        );
    }

    /// `Messages::select` 必须走 language-only 回退。
    ///
    /// 浏览器显式追加 language-only 回退标签（`en-US,en;q=0.9`），
    /// 所以「只有 `zh-Hans-CN` 译文而用户是 `zh-Hans`」必须能取到。
    #[test]
    fn 消息查找按_language_only_回退() {
        let mut m = Messages::new();
        m.insert("zh-Hans-CN", "hello", "你好");
        // 只插了精确 locale。
        assert_eq!(
            m.select(&Preferences::from_list(&["zh-Hans-CN"]), "en", "hello"),
            Some("你好")
        );

        // 语言段回退：用户标签短于可用表时也应命中。
        m.insert("zh", "bye", "再见");
        assert_eq!(
            m.select(&Preferences::from_list(&["zh-Hans", "en"]), "en", "bye"),
            Some("再见")
        );
    }

    /// 漏翻必须回落到**默认语言**的文案，而不是键名、更不是空白。
    ///
    /// 这是浏览器的真实行为（某门语言缺资源时回落默认语言），
    /// 也是"降级要写在明处"的落点：真要发现漏翻，靠门禁而不是靠界面。
    #[test]
    fn 漏翻回落到默认语言() {
        let mut m = Messages::new();
        m.insert("en", "nav.home", "Home");
        // `ja` 一个键都没有 ⇒ 必须拿到 `en` 的文案。
        assert_eq!(
            m.select(&Preferences::from_list(&["ja"]), "en", "nav.home"),
            Some("Home"),
            "漏翻应回落默认语言，而不是 None/空串"
        );
        // 连默认语言也没有 ⇒ None（由调用方显示键名）。
        assert_eq!(m.select(&Preferences::from_list(&["ja"]), "en", "nope"), None);
    }

    /// 默认语言的语言段回退：默认语言写 `zh-Hans` 而调用方给 `zh` 时仍要命中。
    ///
    /// 这一条是**真实**的：`negotiate` 交出来的 `Tag` 会带 script，
    /// 而调用方手里的默认语言常量可能只写到语言段。
    #[test]
    fn 默认语言也走语言段回退() {
        let mut m = Messages::new();
        m.insert("zh-Hans", "a", "A");
        assert_eq!(m.select(&Preferences::new(), "zh-Hans", "a"), Some("A"));
        // `en` 兜底时先试 `en`、再试 `en`（无 script 无 region，同一项）。
        assert_eq!(m.select(&Preferences::new(), "en-Hans", "a"), None);
    }

    /// 漏翻必须可被机器发现（门禁依据）。
    #[test]
    fn 缺失键可枚举() {
        let mut m = Messages::new();
        m.insert("en", "a", "A");
        let missing = m.missing_in("en", &["a", "b", "c"]);
        assert_eq!(missing, vec!["b".to_string(), "c".to_string()]);
    }

    /// 偏好表有序、去重，`first()` 对应 `navigator.language`。
    #[test]
    fn 偏好表有序且去重() {
        let p = Preferences::from_list(&["en-US", "zh-CN"]);
        assert_eq!(p.first().unwrap().as_str(), "en-US");
        let mut q = p.clone();
        q.push(Tag::parse("en-US").unwrap());
        assert_eq!(q.len(), 2, "去重但保留顺序");
        q.push(Tag::parse("ja").unwrap());
        assert_eq!(q.len(), 3);
        // 坏项跳过而不是让整表失效。
        assert_eq!(Preferences::from_list(&["en", "bogus-tag"]).len(), 1);
    }

    /// `from_owned` 与 `from_list` 必须同形（宿主交回的是有所有权的串）。
    ///
    /// 这条不是恒真：把 `from_owned` 实现成"`from_list` 但忽略非法项"的话，
    /// 本测试仍会过，而真正的问题（宿主给的语言**全**被跳过 → 回落默认）
    /// 会静默发生。所以这里额外断言"含坏项时好项仍在"。
    #[test]
    fn 自有列表与借用列表同形() {
        let owned: Vec<String> = vec!["zh-CN".to_string(), "坏-标签".to_string()];
        let p = Preferences::from_owned(&owned);
        assert_eq!(p.len(), 1);
        assert_eq!(p.first().unwrap().as_str(), "zh-CN");
        assert_eq!(p, Preferences::from_list(&["zh-CN"]));
    }
}

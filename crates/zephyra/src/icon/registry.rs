//! 图标注册表：名字 → 已解析的 [`Icon`]。
//!
//! # 为什么要注册表而不是直接调 `parse_svg`
//!
//! 两个原因，都是硬约束：
//!
//! 1. **解析一次。** 图标是静态内容。若每次 `view()` 都调解析器，一个 8 按钮的
//!    工具栏每帧要跑 8 次全命令集 path 解析——这是纯浪费，且随图标复杂度线性增长。
//! 2. **身份必须稳定。** 栅格化缓存以图标数据的**地址**为键（非 `'static` 的图标
//!    可能在栈上，函数返回后地址被复用，另一个图标就会命中它的缓存条目）。
//!    `OnceLock` 保证了每个名字恰好对应一个 `'static` 地址。
//!
//! # 谁定义哪些图标
//!
//! [`assets`] 里是**框架自带**的通用字形（菜单、画笔、撤销…），任何应用都能用。
//! 应用专属的图标（相机、图片库、批注）由应用自己 [`IconSet::with`] 追加——
//! 框架不该知道 Viewstage 有个"拍照"按钮，反过来也不该让每个应用自带一份
//! `menu.svg`。

use std::collections::HashMap;
use std::sync::OnceLock;

use super::{parse_path_data, parse_svg, Icon};

/// 未知图标名的兜底，进程内唯一。
///
/// 必须是**同一个** `static` 而不是每次构造一个新的 `Icon::EMPTY`——缓存键取的是
/// 数据地址，而"所有未知名字共享同一个空图标"正是我们想要的：一个未知名字只占
/// 一个（永远不会被填上的）缓存条目。
static EMPTY: Icon = Icon::EMPTY;

/// 图标源。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconSource {
    /// 一份图标级 SVG 文本（通常来自 [`crate::icon::assets`] 的内嵌常量）。
    Svg(&'static str),
    /// 裸 path data（无 SVG 包装），`viewBox` 按 `0 0 24 24` 视为。
    ///
    /// 与 [`IconSource::Svg`] 走**同一个**解析器，所以只有少一层 XML 提取的差别，
    /// 形状语义完全一致。
    Path(&'static str),
}

impl IconSource {
    /// 解析成 [`Icon`]。
    fn parse(self) -> Result<Icon, String> {
        match self {
            IconSource::Svg(text) => parse_svg(text).map_err(|e| e.to_string()),
            IconSource::Path(d) => parse_path_data(d)
                .map(|segments| Icon {
                    view_box: (0.0, 0.0, 24.0, 24.0),
                    paint: super::Paint::CURRENT_COLOR,
                    segments,
                })
                .map_err(|e| e.to_string()),
        }
    }
}

/// 图标注册表。
///
/// 构造后 [`IconSet::get`] 在构造完成前只会返回 [`Icon::EMPTY`]；正常用法是先
/// [`IconSet::new`] + 若干 [`IconSet::with`]，再 [`IconSet::freeze`]。
///
/// # 必须放在 `static` 里
///
/// [`IconSet::get`] 返回 `&'static Icon`，所以 `self` 也必须是 `'static`——
/// 这是**刻意**的约束，不是签名上的省事。
///
/// 宿主侧的栅格化缓存以图标数据的**地址**为键（比字符串哈希便宜，且
/// `include_str!` 的地址天然稳定）。若图标能放在栈上或随 `IconSet` 一起析构，
/// 函数返回后地址就被下一个图标复用，于是 A 的缓存条目会被 B 命中——
/// 表现为"某个图标显示成另一个图标"，且不报任何错。
///
/// 把 `&'static self` 写进签名，这条不变量就从"文档里的叮嘱"变成了编译期事实。
pub struct IconSet {
    entries: Vec<(&'static str, IconSource)>,
    parsed: OnceLock<HashMap<&'static str, Icon>>,
}

impl Default for IconSet {
    fn default() -> Self {
        Self::new()
    }
}

impl IconSet {
    /// 空注册表。
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            parsed: OnceLock::new(),
        }
    }

    /// 以框架自带字形为基础的注册表。
    ///
    /// [`crate::icon::assets`] 的全部条目。应用在此之上追加自己的图标。
    pub fn with_builtin() -> Self {
        let mut set = Self::new();
        for (name, source) in super::assets::REGISTRY {
            set = set.with(name, *source);
        }
        set
    }

    /// 登记一个图标。
    ///
    /// 同一个名字重复登记时**后者覆盖前者**——而不是取第一个。那样更符合
    /// "追加覆盖"的直觉，也让应用可以覆盖框架自带的字形（比如换个更符合
    /// 品牌规范的 `menu`）而不必先删。
    pub fn with(mut self, name: &'static str, source: IconSource) -> Self {
        match self.entries.iter_mut().find(|(n, _)| *n == name) {
            Some(slot) => slot.1 = source,
            None => self.entries.push((name, source)),
        }
        self
    }

    /// 固化：此刻解析全部登记项。
    ///
    /// 可选——不调用的话 [`IconSet::get`] 会自己固化。但显式调用把"解析发生在哪里"
    /// 交给应用决定：放在启动路径上，解析耗时就落在启动里而不是第一帧。
    pub fn freeze(&self) {
        self.parsed.get_or_init(|| self.build());
    }

    /// 已登记的名字（顺序即登记顺序）。
    ///
    /// 给"我要哪些图标"的校验用：某个名字不在这里，就说明它没被登记，
    /// 渲染时会静默变成空白——那是最难查的一类问题。
    pub fn names(&self) -> Vec<&'static str> {
        self.entries.iter().map(|(n, _)| *n).collect()
    }

    /// 按名字取图标；未知名字回落到 [`Icon::EMPTY`]。
    pub fn get(&'static self, name: &str) -> &'static Icon {
        let map = self.parsed.get_or_init(|| self.build());
        map.get(name).unwrap_or(&EMPTY)
    }

    fn build(&self) -> HashMap<&'static str, Icon> {
        let mut map = HashMap::with_capacity(self.entries.len());
        for (name, source) in &self.entries {
            match source.parse() {
                Ok(icon) => {
                    map.insert(*name, icon);
                }
                Err(err) => {
                    // 解析失败**不静默**。debug 下打印，release 下也不吞掉——
                    // 因为后果是"某个图标不显示"，而没有任何其他症状会指向它。
                    //
                    // 这里选择 panic 而非跳过：注册表是编译期常量，解析失败意味着
                    // 某个写死的 SVG 资源坏了，那属于必须当场暴露的构建缺陷，
                    // 而不是可以靠"少一个图标"蒙混过去的运行时状况。
                    panic!("[zephyra::icon] 解析失败 name={name} err={err}");
                }
            }
        }
        map
    }
}
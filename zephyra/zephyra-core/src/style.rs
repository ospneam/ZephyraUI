//! 样式：把 [`Node`] 的属性表 + [`Context`] 翻译成具体值。
//!
//! # 三层读取顺序
//!
//! 每个 getter 走同一条链（见 [`Node::value`]）：本地 → 样式 → 继承 → 默认，
//! 然后按当前主题解析其中的资源键。区别只在**缺省值**与**是否该解析**。
//!
//! # 为什么大部分 getter 是 `pub` 而不是 `pub(crate)`
//!
//! 它们是控件层的公共词汇表 —— `Button` 想知道 hover 背景色，宿主想读 `opacity`
//! 算合成，都直接调即可。把它们藏起来只会逼出"复制一份读取逻辑"的代码，那份副本
//! 会在资源键解析上与本模块产生细微不一致。

use crate::context::Context;
use crate::node::Node;
use crate::property::PropertyValue;

impl Node {
    /// 是否被折叠（不占位、不绘制）。
    pub fn is_collapsed(&self, ctx: Context<'_>) -> bool {
        self.flag(&crate::props::COLLAPSED, ctx)
    }

    /// 内边距（四边统一值）。
    pub fn padding(&self, ctx: Context<'_>) -> crate::core::Thickness {
        crate::core::Thickness::uniform(self.number(&crate::props::PADDING, ctx))
    }

    /// 文本内容（`None` = 无文本）。
    ///
    /// 接受 `StaticStr`（编译期常量）与 `Str`（运行期字符串）两种来源 —— 属性
    /// 默认值只能是前者，但真实文本几乎总是运行期构造的（来自应用状态）。
    ///
    /// # 刻意不走 `value()`
    ///
    /// 其它属性读法都经 [`Node::value`]，因为它们可能是**资源键**，需要在读取那一刻
    /// 按当前 `&Theme` 解析。文本不是设计令牌、永远不会解析成资源，所以直接用
    /// [`crate::property::resolve`] 拿属性表里的 `&PropertyValue`。
    ///
    /// 这么做同时解决了借用问题：`value()` 返回 `Cow<'_, PropertyValue>`，它内部
    /// 的 `Deref` 只能借到局部绑定，于是从里面取出的 `&str` 反而活得比属性表短。
    /// 直接拿 `&PropertyValue` 就得到指向 `Arc<str>` 的引用，**零拷贝**。
    pub fn text(&self) -> Option<&str> {
        let (raw, _source) = crate::property::resolve(self.props(), &crate::props::TEXT, None);
        let s: &str = match raw {
            PropertyValue::StaticStr(s) => s,
            PropertyValue::Str(s) => s.as_ref(),
            _ => return None,
        };
        (!s.is_empty()).then_some(s)
    }

    /// 文本样式（由本节点的文本属性合成，不含字体 —— 字体是宿主的事）。
    pub fn text_style(&self, ctx: Context<'_>) -> crate::text::TextStyle {
        crate::text::TextStyle {
            // 字号缺省不写死：0.0 会被宿主当成"非法"，由宿主回落到自己的默认字号。
            size: self
                .number_or(&crate::props::TEXT_SIZE, 14.0, ctx)
                .max(1.0),
            weight: 400,
            family: None,
            line_height: {
                let h = self.number_or(&crate::props::TEXT_LINE_HEIGHT, 1.2, ctx);
                (h.is_finite() && h > 0.0).then_some(h)
            },
            scale: 1.0,
        }
    }

    /// 文本的最大宽度（`None` = 不限宽，不换行）。
    pub(crate) fn text_max_width(&self, ctx: Context<'_>) -> Option<f32> {
        finite(self.number_or(&crate::props::TEXT_MAX_WIDTH, f32::NAN, ctx))
    }

    /// 文本块的尺寸（`None` = 本节点无文本，或本进程无文本引擎）。
    ///
    /// 测量走 [`crate::text::TextEngine`] —— 核心不认识字体，所以这一步必然要
    /// 问宿主。没有引擎时返回 `None`，于是文本节点按零尺寸参与布局。
pub fn text_size(&self, ctx: Context<'_>) -> Option<crate::core::Size> {
        self.shape(ctx).map(|s| s.size)
    }

    /// 排版一次并返回**完整**结果（尺寸 + 字形位置）。
    ///
    /// # 为什么多一个入口而不是让 `text_size` 返回整个 `ShapedText`
    ///
    /// `text_size` 出现在**很多**地方，而其中绝大多数只关心 `size`。让它返回
    /// 整个 `ShapedText` 会逼每个调用方写 `.map(|s| s.size)`，那是噪音。
    /// 而绘制趟要的是完整的那个，于是单独给一个名字。
    ///
    /// 排版结果会经 [`crate::layout::MeasureResult::shaped`] 存进测量缓存，
    /// 绘制趟 [`crate::paint::text_command`] 从那里取 —— 于是每帧只有**第一次**
    /// 排版，之后的绘制趟复用。
    pub fn shape(&self, ctx: Context<'_>) -> Option<crate::text::ShapedText> {
        let text = self.text()?;
        let engine = ctx.engine?;
        let style = self.text_style(ctx);
        let max_width = self.text_max_width(ctx);
        Some(engine.borrow_mut().shape(text, &style, max_width))
    }

    /// 图像纹理（`None` = 无图像内容）。
    ///
    /// 接受 `StaticStr`（编译期常量）与 `Str`（运行期字符串）两种来源——属性定义
    /// 的默认值只能是前者，但真实纹理由宿主在运行时填入。
    pub fn image_texture(&self, ctx: Context<'_>) -> Option<crate::paint::TextureRef> {
        match self.value(&crate::props::IMAGE_TEXTURE, ctx).as_deref()? {
            PropertyValue::StaticStr(s) if !s.is_empty() => {
                Some(crate::paint::TextureRef(std::borrow::Cow::Borrowed(s)))
            }
            PropertyValue::Str(s) if !s.is_empty() => Some(crate::paint::TextureRef(
                std::borrow::Cow::Owned(s.as_ref().to_owned()),
            )),
            _ => None,
        }
    }

    /// 图标名（`None` = 无矢量图标内容）。
    ///
    /// 与 [`Self::image_texture`] 同一套取值方式：`StaticStr` 与 `Str` 都接受，
    /// 空串视为"没有"。
    ///
    /// ⚠ 返回的字符串是**借来的**，而 [`crate::paint::Command::Path`] 的顶点里
    /// 不含它 —— 名字只在本次降级里用来查图标集，之后就被丢掉。所以借用是安全的。
    pub fn icon_name(&self, ctx: Context<'_>) -> Option<std::borrow::Cow<'_, str>> {
        match self.value(&crate::props::ICON_NAME, ctx).as_deref()? {
            PropertyValue::StaticStr(s) if !s.is_empty() => {
                Some(std::borrow::Cow::Borrowed(*s))
            }
            PropertyValue::Str(s) if !s.is_empty() => Some(std::borrow::Cow::Owned(s.as_ref().to_owned())),
            _ => None,
        }
    }

    /// 图标填充色。
    pub fn icon_color(&self, ctx: Context<'_>) -> crate::core::Color {
        self.color(&crate::props::ICON_COLOR, ctx)
            .unwrap_or(crate::core::Color::rgb(1.0, 1.0, 1.0))
    }

    /// 图像源裁剪（`None` = 整张纹理）。
    ///
    /// 返回 `None` 而不是 `FULL`，是为了让 `Command::Image.src` 保持 `None` 的
    /// 语义："没裁剪"。后端据此可以跳过 uv 计算，也便于断言"这条命令没裁剪"。
    ///
    /// 越界或零尺寸的 uv 一律退化成 `None`（即整张），而不是原样转发或钳正。
    /// 转发会让采样器取到相邻图块的像素，钳正会让"我本来就想取这一块"的意图与
    /// 实际取到的区域不一致 —— 而这两种错误的症状都出现在采样之后，无从定位。
    /// 退化成整张至少让画面保持正确，让 bug 表现为"裁剪没生效"而非"花屏"。
    pub fn image_src(&self, ctx: Context<'_>) -> Option<crate::core::UvRect> {
        let uv = self
            .value(&crate::props::IMAGE_SRC, ctx)
            .and_then(|v| v.get::<crate::core::UvRect>())?;
        (uv.is_in_range() && !uv.is_full()).then_some(uv)
    }

    /// 堆叠描述。
    pub fn stack(&self, ctx: Context<'_>) -> crate::layout::Stack {
        crate::layout::Stack {
            direction: match self.value(&crate::props::DIRECTION, ctx).as_deref() {
                Some(PropertyValue::Direction(d)) => *d,
                _ => crate::layout::Direction::Horizontal,
            },
            spacing: self.number(&crate::props::SPACING, ctx),
            main_align: match self.value(&crate::props::MAIN_ALIGN, ctx).as_deref() {
                Some(PropertyValue::MainAlign(a)) => *a,
                _ => crate::layout::MainAxisAlign::Start,
            },
        }
    }

    /// 交叉轴对齐。
    pub fn cross_align(&self, ctx: Context<'_>) -> crate::layout::AlignItems {
        match self.value(&crate::props::CROSS_ALIGN, ctx).as_deref() {
            Some(PropertyValue::CrossAlign(a)) => *a,
            _ => crate::layout::AlignItems::Start,
        }
    }

    /// 背景色。
    pub fn background(&self, ctx: Context<'_>) -> crate::core::Color {
        self.color(&crate::props::BACKGROUND, ctx)
            .unwrap_or(crate::core::Color::rgba(0.0, 0.0, 0.0, 0.0))
    }

    /// 圆角半径。
    pub fn corner_radius(&self, ctx: Context<'_>) -> f32 {
        self.number(&crate::props::CORNER_RADIUS, ctx)
    }

    /// 描边色。
    pub fn border_color(&self, ctx: Context<'_>) -> crate::core::Color {
        self.color(&crate::props::BORDER_COLOR, ctx)
            .unwrap_or(crate::core::Color::rgba(0.0, 0.0, 0.0, 0.0))
    }

    /// 描边宽度。
    pub fn border_width(&self, ctx: Context<'_>) -> f32 {
        self.number(&crate::props::BORDER_WIDTH, ctx)
    }

    /// 整体不透明度（声明为继承）。
    ///
    /// ⚠ 名字里的"继承"指的是 [`crate::props::OPACITY`] 声明成 `inheritable`，
    /// 而**属性系统尚未接线**：`property::resolve` 的 `inherited` 参数在生产代码里
    /// 一律收到 `None`（见本文件 `value()`），所以这里取到的是**本节点**的值。
    /// 绘制趟自己沿树累乘（见 `paint.rs::PaintState::descend`），
    /// 于是**画面上的行为**是对的；不对的只有这个 getter 的名字。
    /// 记账见 `docs/task-anim-wire.md` §0。
    pub fn opacity(&self, ctx: Context<'_>) -> f32 {
        self.number_or(&crate::props::OPACITY, 1.0, ctx)
    }

    /// 绘制时 X 偏移（DIP）。
    ///
    /// 取本节点的值（不沿父链相加）—— 累加由绘制趟负责，理由同 [`Self::opacity`]。
    pub fn translate_x(&self, ctx: Context<'_>) -> f32 {
        self.number_or(&crate::props::TRANSLATE_X, 0.0, ctx)
    }

    /// 绘制时 Y 偏移（DIP）。见 [`Self::translate_x`]。
    pub fn translate_y(&self, ctx: Context<'_>) -> f32 {
        self.number_or(&crate::props::TRANSLATE_Y, 0.0, ctx)
    }

    /// 合成层。
    pub fn layer(&self, ctx: Context<'_>) -> crate::paint::Layer {
        match self.value(&crate::props::LAYER, ctx).as_deref() {
            Some(PropertyValue::Layer(l)) => *l,
            _ => crate::paint::Layer::Content,
        }
    }

    /// 滤镜链（默认空链 = 恒等）。
    ///
    /// 走 `Clone` 而非引用：属性表里存的是 `PropertyValue`，而
    /// `FilterChain` 内部是 `Vec`，借不出`&[Filter]`——`value()` 返回的
    /// `Option<&PropertyValue>` 借到的是外层枚举。复制一条空链不分配，只有
    /// 真的有滤镜时才分配一次。
    pub fn filter(&self, ctx: Context<'_>) -> crate::filter::FilterChain {
        match self.value(&crate::props::FILTER, ctx).as_deref() {
            Some(PropertyValue::Filter(f)) => f.clone(),
            _ => crate::filter::FilterChain::none(),
        }
    }

    /// 混合模式（默认 [`crate::filter::BlendMode::SrcOver`]）。
    pub fn blend_mode(&self, ctx: Context<'_>) -> crate::filter::BlendMode {
        match self.value(&crate::props::BLEND_MODE, ctx).as_deref() {
            Some(PropertyValue::BlendMode(b)) => *b,
            _ => crate::filter::BlendMode::default(),
        }
    }

    /// 图像边长（`-1` = 撑满节点矩形）。
    pub fn image_size(&self, ctx: Context<'_>) -> f32 {
        self.number(&crate::props::IMAGE_SIZE, ctx)
    }

    /// 图像着色。
    pub fn image_tint(&self, ctx: Context<'_>) -> crate::core::Color {
        self.color(&crate::props::IMAGE_TINT, ctx)
            .unwrap_or(crate::core::Color::rgb(1.0, 1.0, 1.0))
    }

    /// 是否可交互。
    pub fn is_interactive(&self, ctx: Context<'_>) -> bool {
        self.flag(&crate::props::INTERACTIVE, ctx)
    }

    /// 是否处于选中态。
    pub fn is_selected(&self, ctx: Context<'_>) -> bool {
        self.flag(&crate::props::SELECTED, ctx)
    }

    /// 是否禁用。
    pub fn is_disabled(&self, ctx: Context<'_>) -> bool {
        self.flag(&crate::props::DISABLED, ctx)
    }

    /// 读一个颜色属性，解析其中的资源键。
    pub(crate) fn color(
        &self,
        key: &crate::property::PropertyKey,
        ctx: Context<'_>,
    ) -> Option<crate::core::Color> {
        self.value(key, ctx)
            .and_then(|v| v.get::<crate::core::Color>())
    }

    /// 读一个属性值（本地→样式→继承→默认），并按当前主题解析其中的资源键。
    ///
    /// 返回 `Cow`：非资源值直接借用（零分配），资源键才现场解析。资源键解析失败
    /// （角色名拼错）时返回 `None` ——调用方据此决定降级，而不是拿到一个黑色。
    pub fn value<'a>(
        &'a self,
        key: &'a crate::property::PropertyKey,
        ctx: Context<'_>,
    ) -> Option<std::borrow::Cow<'a, crate::property::PropertyValue>> {
        let (raw, _source) = crate::property::resolve(self.props(), key, None);
        match crate::theme::resolve_resource(ctx.theme, raw) {
            Some(resolved) => Some(resolved),
            None if raw.is_resource() => None,
            None => Some(std::borrow::Cow::Borrowed(raw)),
        }
    }

    /// 读一个数值属性，缺省 0。
    pub(crate) fn number(&self, key: &crate::property::PropertyKey, ctx: Context<'_>) -> f32 {
        self.number_or(key, 0.0, ctx)
    }

    /// 读一个数值属性，带缺省。
    ///
    /// 过滤非有限值：属性默认值用 `NaN` 表示"未设置"，若直接透传会让
    /// `NaN` 混进几何计算并污染整条布局链（下游全变 `NaN`，且不会报错）。
    pub(crate) fn number_or(
        &self,
        key: &crate::property::PropertyKey,
        fallback: f32,
        ctx: Context<'_>,
    ) -> f32 {
        self.value(key, ctx)
            .and_then(|v| v.get::<f32>())
            .filter(|v| v.is_finite())
            .unwrap_or(fallback)
    }

    /// 读一个布尔属性。
    pub(crate) fn flag(&self, key: &crate::property::PropertyKey, ctx: Context<'_>) -> bool {
        self.value(key, ctx)
            .and_then(|v| v.get::<bool>())
            .unwrap_or(false)
    }
}

/// `NaN` 表示"未设置"，转成 `None`。
fn finite(v: f32) -> Option<f32> {
    (v.is_finite() && v >= 0.0).then_some(v)
}
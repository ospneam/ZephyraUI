//! 输入路由：指针事件 → 命中测试 → 控件状态变化。
//!
//! # 为什么路由在核心里
//!
//! hover / press 是**保留式** UI 的核心概念：控件自己持有"我正在被悬停"这个状态，
//! 并据此改变外观。它不是"每个应用各写一遍的事件分发"——那样每个应用都会各自实现
//! 捕获、穿透、按下-抬起配对，行为必然不一致。
//!
//! # 事件产出是零分配的
//!
//! 一次输入事件最多产生一条 [`RoutedEvent`]（见 [`MAX_EVENTS_PER_INPUT`] 的推导）。
//! 所以 [`InputRouter::on_pointer`] 往调用方提供的固定容量缓冲里写，**不返回 `Vec`**。
//! 指针移动是每秒上百次的路径，在那里分配一次 `Vec` 就足以在帧时间上留下尖峰。
//!
//! # 悬停变化用 revision 表达，而不是事件
//!
//! 悬停进出可能一次事件里发生两次（从 A 移到 B：先 exit A 再 enter B）。若把它们
//! 也做成事件流，宿主就得维护一份"当前悬停集合"再和控件属性对账。这里让路由器**持有**
//! 状态并用 [`InputRouter::revision`] 暴露版本号：宿主只需比较 revision 是否变化，
//! 变了就重绘。这是保留式 UI 的常规做法（GPUI 的 `Window`、egui 的 `repaint` 都这么做），
//! 因为宿主真正需要的从来不是"发生了什么"，而是"要不要重画"。

use crate::core::Point;
use crate::hit::{hit_test, HitResult, HitTestRequest};
use crate::input::recognizer::{Gesture, Recognizer};
use crate::input::pointer::{PointerEvent, PointerPhase};
use crate::context::Context;
use crate::id::ElementKey;

/// 单个输入事件最多产出的路由事件数。
///
/// 推导：一次 `Move` 至多产生「悬停 exit + 悬停 enter」两条（不算在事件流里，见上），
/// 加上至多一条手势。因此事件流里恒为 0 或 1 条。取 2 是为了让"按下 + 同帧手势"
/// 这种边界情形不必 panic——宁可留一个空位，也不要在热路径上做长度检查后
/// 静默丢弃事件（那会让一次点击消失且无从排查）。
pub const MAX_EVENTS_PER_INPUT: usize = 2;

/// 一条路由结果。
#[derive(Debug, Clone, PartialEq)]
pub enum RoutedEvent {
    /// 在某节点上按下。
    Press(ElementKey),
    /// 抬起。`pressed` 是按下时记录的节点，`released` 是抬起时命中的节点——
    /// 两者可以不同（按下按钮、拖出去松手），宿主据此决定"取消"还是"提交"。
    Release {
        /// 按下时命中的节点。
        pressed: ElementKey,
        /// 抬起时命中的节点（拖出控件外则缺省）。
        released: Option<ElementKey>,
    },
    /// 点击：按下与抬起落在**同一**节点上。
    Click(ElementKey),
    /// 手势（捏合增量）。
    Gesture(Gesture),
}

/// 固定容量的事件缓冲。
#[derive(Debug, Clone, Default)]
pub struct RoutedEvents {
    items: [Option<RoutedEvent>; MAX_EVENTS_PER_INPUT],
    len: usize,
}

impl RoutedEvents {
    /// 空缓冲。
    pub fn new() -> Self {
        Self {
            items: Default::default(),
            len: 0,
        }
    }

    /// 已产出条数。
    pub fn len(&self) -> usize {
        self.len
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// 逐条取出。
    pub fn iter(&self) -> impl Iterator<Item = &RoutedEvent> {
        self.items[..self.len].iter().filter_map(|e| e.as_ref())
    }

    fn clear(&mut self) {
        for slot in &mut self.items[..self.len] {
            *slot = None;
        }
        self.len = 0;
    }

    fn push(&mut self, event: RoutedEvent) {
        if self.len >= MAX_EVENTS_PER_INPUT {
            // 不静默丢弃：超长意味着上面的容量推导错了，那是个必须暴露的 bug。
            // 但热路径不能 panic，所以退化成"忽略"并在 debug 下可见。
            debug_assert!(
                false,
                "路由事件超出 MAX_EVENTS_PER_INPUT={MAX_EVENTS_PER_INPUT}；\
                 单个输入事件最多产生 1 条，出现 3 条说明状态机有 bug"
            );
            return;
        }
        self.items[self.len] = Some(event);
        self.len += 1;
    }
}

/// 输入路由器：手势识别 + 命中测试 + 悬停/按下状态。
///
/// # 捕获（capture）语义
///
/// 按下时记录 `pressed`；随后即使指针移出该节点，抬起仍归它所有（`released` 为
/// `None`）。这与 WinUI / 浏览器一致：按下按钮后横向拖出去松手，应当**取消**而不是
/// 触发另一个按钮。若抬起时才重新命中，最常见的故障是"在 A 上按下、拖到 B 上松开，
/// 于是 B 被点了一下"——那会误触发用户没有点的东西。
#[derive(Clone)]
pub struct InputRouter {
    recognizer: Recognizer,
    /// 当前悬停的可交互节点。
    hovered: Option<ElementKey>,
    /// 按下时命中的节点；抬起前一直被"捕获"，与实际悬停无关。
    pressed: Option<ElementKey>,
    /// 悬停/按下状态版本号；宿主比较它决定要不要重绘。
    revision: u64,
    /// 复用的命中测试结果，避免每次指针移动重新分配。
    hit: HitResult,
}

impl Default for InputRouter {
    fn default() -> Self {
        Self::new()
    }
}

impl InputRouter {
    /// 新建（无悬停、无按下）。
    pub fn new() -> Self {
        Self {
            recognizer: Recognizer::new(),
            hovered: None,
            pressed: None,
            revision: 0,
            hit: HitResult::default(),
        }
    }

    /// 悬停 / 按下状态的版本号。
    ///
    /// 任何一次导致"是否悬停"或"是否按下"发生变化的输入都会让它递增。宿主在每帧
    /// 末尾比较新旧值即可决定是否需要重绘——不必自己追踪状态。
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// 当前悬停的可交互节点。
    pub fn hovered(&self) -> Option<&ElementKey> {
        self.hovered.as_ref()
    }

    /// 按下时被捕获的节点。
    pub fn pressed(&self) -> Option<&ElementKey> {
        self.pressed.as_ref()
    }

    /// 手势识别器（只读；多指手势的出口）。
    pub fn recognizer(&self) -> &Recognizer {
        &self.recognizer
    }

    /// 当前活跃指针数。
    pub fn pointer_count(&self) -> usize {
        self.recognizer.pointer_count()
    }

    /// 最近一次命中测试的结果（复用缓冲，勿长期持有）。
    pub fn last_hit(&self) -> &HitResult {
        &self.hit
    }

    /// 全量复位（窗口失焦等异常恢复路径）。
    ///
    /// 必须同时清掉手势状态与按下状态：若按下状态残留，失焦后第一次点击会被判为
    /// "抬起"，从而误触发一个用户以为只是取消了的操作。
    pub fn reset(&mut self) {
        self.recognizer.reset();
        self.hovered = None;
        self.pressed = None;
        self.revision += 1;
        self.hit.clear();
    }

    /// 处理一个指针事件，产出路由结果。
    ///
    /// `root` 必须是**已布局**的树——命中测试读 [`crate::node::Node::rect`]，
    /// 未布局时所有矩形都是零，命中测试会一律返回"没点到"。
    pub fn on_pointer(
        &mut self,
        root: &crate::node::Node,
        ev: PointerEvent,
        ctx: Context<'_>,
        out: &mut RoutedEvents,
    ) {
        out.clear();
        let gesture = self.recognizer.on_pointer(ev);

        // 命中测试对三相位都需要：Down 要找按下目标，Move 要更新悬停，
        // Up 要知道松开了没有（可能已在控件外）。指针离开窗口时位置可能无意义，
        // 但 Move 事件一定带位置，Down/Up 也带，所以这里不需要额外的存在性判断。
        let hit_any = hit_test(
            root,
            HitTestRequest {
                point: Point::new(ev.x, ev.y),
                ignore_above: None,
            },
            ctx,
            &mut self.hit,
        );
        let target = if hit_any {
            self.hit
                .target()
                .map(|n| (n.id.clone(), n.rect))
        } else {
            None
        };

        match ev.phase {
            PointerPhase::Down => {
                if let Some((id, _)) = &target {
                    // 按在不可交互的空白上时**不**清除已有的按下捕获：
                    // 那属于"在空白处按下"，此时松开应当什么都不发生。
                    self.pressed = Some(id.clone());
                    out.push(RoutedEvent::Press(id.clone()));
                    // ⚠️ `pressed` 变了就必须 `revision++`。
                    //
                    // 宿主靠 `revision` 变化决定「要不要重绘」，而按下态是**画出来的**
                    // （`.pressed` 分类名 → 样式层换色）。漏了这一行，症状是
                    // 「按钮按下时没有按下样式，直到鼠标动一下才出现」。
                    self.revision += 1;
                }
            }
            PointerPhase::Move => {
                // 悬停只在没有按下捕获时跟随——拖动过程中让悬停乱跳会让按钮闪。
                if self.pressed.is_none() {
                    let next = target.map(|(id, _)| id);
                    if next != self.hovered {
                        self.hovered = next;
                        self.revision += 1;
                    }
                }
            }
            PointerPhase::Up => {
                let released = target.map(|(id, _)| id);
                if let Some(pressed) = self.pressed.take() {
                    let same = released.as_ref() == Some(&pressed);
                    out.push(RoutedEvent::Release {
                        pressed: pressed.clone(),
                        released: released.clone(),
                    });
                    if same {
                        out.push(RoutedEvent::Click(pressed));
                    }
                    // ⚠️ `pressed` 被清空同样要 `revision++`。
                    //
                    // 这一行曾经是**整个交互链路最隐蔽的一处漏**：宿主要靠
                    // `revision` 变化决定「这一帧要不要重绘」，而 `Click` 之后
                    // 应用通常立刻改了状态（切页、勾选、展开…）。若此时 hover
                    // 恰好没变（鼠标没动 —— 点按钮时最常见），`revision` 不变 ⇒
                    // 宿主判定「什么都没变」⇒ **不重绘**。
                    //
                    // 症状：第一次点击「没反应」，把鼠标移开再移回来才生效 ——
                    // 而画面上没有任何异常，看不出是重绘漏了。
                    self.revision += 1;
                }
                // 抬起后悬停立刻跟上指针当前位置：一次"按下→拖出→拖回"结束时，
                // 悬停应已经是"在按钮上"，否则画面会停在无高亮状态直到下次移动。
                if self.pressed.is_none() {
                    let next = released;
                    if next != self.hovered {
                        self.hovered = next;
                        self.revision += 1;
                    }
                }
            }
        }

        // 手势与 hover/press 是两条正交通道：捏合时也要出悬停变化，
        // 否则多指操作会让界面高亮停留在第一根手指落点。
        if gesture.pinch.is_some() || gesture.pinch_started {
            out.push(RoutedEvent::Gesture(gesture));
        }
    }
}
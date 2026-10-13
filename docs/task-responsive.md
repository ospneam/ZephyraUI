# 动态布局（WinUI 式）+ 触控适配 —— 任务清单

> 原则（用户要求）：**先查官方文档，不自己拍数字。**
> 文档源：`https://learn.microsoft.com/zh-cn/windows/apps/design/...`
> 与 `https://learn.microsoft.com/zh-cn/uwp/api/...`

## A. 文档（先做，产出权威数字）
- [x] A1 断点：`screen-sizes-and-breakpoints` —— WinUI/AdaptiveTrigger 的官方断点
- [x] A2 触摸目标：`touch-interactions` —— 最小触摸目标尺寸
- [x] A3 `AdaptiveTrigger` API 页：`MinWindowWidth` / `MinWindowHeight` 语义
- [x] A4 落成 `docs/responsive-and-touch.md`：每个数字标出处，写进代码前先查

## B. 修 resize 后的真实缺陷（要复现、不能靠推理）
- [x] B1 实测三尺寸（1400 / 1882 / 982）—— 已复现：宽窗口右侧未绘制、窄窗口溢出裁切
- [ ] B2 定位「resize 后没有重绘」的根因（trace 显示 `redrawn==false` 路径）
- [x] B3 门禁：`resize` 必须重排重绘（core 已有 `viewport` 判据，确认它真的生效）

## C. core 侧：动态布局所需的最小能力
- [x] C1 确认 `Container.window` 的固定 `width/height` 是自适应的唯一阻碍
- [x] C2 zstyle 无法按断点给值（无 media query）—— 决定断点在哪一层表达
- [ ] C3 若无 fill 语义，`cross-align: stretch` 是否已足够（读 layout.rs 确认）

## D. 应用层：断点 → 布局变体（像 WinUI AdaptiveTrigger）
- [x] D1 断点函数（名字与语义照抄 WinUI：`narrow` / `medium` / `wide`）
- [x] D2 导航：窄窗口折叠成汉堡（WinUI NavigationView 的默认行为）
- [x] D3 卡片网格列数随宽度变（WinUI 的 `ItemsRepeater`/`Grid` `*` 列效果）
- [ ] D4 banner 高度随宽度变、链接卡行/列随宽度变

## E. 触控适配（按 A2 的数字）
- [ ] E1 最小触摸目标：把关键交互元素的内边距/间距调到文档数字
- [ ] E2 触摸态：`pressed` 的可视反馈（WinUI 的 pointer 视觉反馈）
- [ ] E3 触控下的悬停不可用 —— 判定为「触摸」时不要依赖 hover
- [ ] E4 门禁：触摸目标不小于文档值

## F. 收尾
- [ ] F1 全量测试 + 零告警
- [ ] F2 真机验证：resize 三个尺寸各出一图，核对互不相同
- [ ] F3 `--list-routes` 逐路由出图核对 md5
- [ ] F4 清理临时截图（上一轮就因忘清被抓进 commit）
- [ ] F5 更新 REWRITE.md / MIGRATION.md

## 实际完成情况（与计划的差异）

- **B2 结论与最初的猜测相反**：加 trace 后实测 `redrawn=true` / `damage=Full`，
  core 每次 resize 都重排重绘了。所以"渲染异常"不是没重绘，而是
  **画出来的内容不响应窗口尺寸**（宽窗口右侧留白、窄窗口溢出被裁）。
  这正是用户判断"要做成动态布局"的原因。
- **B2 的诊断手段**（`sync_swapchain` / `frame()` 各加一行 trace）保留：
  它区分了"没重排"与"排了新缓冲没显示"两类症状，而两者看起来一模一样。
- **D4（banner 高度随宽度变）未做**：属高度方向，见 §8 的能力缺口。
- **E 组（触控）只完成了判据与换算**（`responsive.rs` + 两条门禁），
  **没有改动任何现有控件的实际尺寸**：改尺寸会动基准外观，
  而基准外观是参考图的全部依据。留给下一轮连同布局基准一起做。
- **新增：`InputState::viewport` + `scale`**（zephyra-fx）。
  宿主与视图之间的唯一每帧通道，断点必须走它。
- **新增门禁 6 条**，全部绿：
  `断点作文档的三个类别且单调` / `列数随宽度变化且基准仍是五列` /
  `内容区宽度随断点变化` / `触控目标按缩放比换算到官方的_epx_下限` /
  `根容器不写固定尺寸` / `基准画布下根容器仍铺满客户区`。
- **踩到并修掉的两个自己的错**（都记在测试注释里）：
  1. `content_width()` 里写死 `236.0`，而 `shell::NAV_W` 其实是 232 ——
     被门禁抓到，改成引用常量。
  2. 断点断言把 640 epx 判成 Medium，而官方"小 = 最大可达 640" ⇒ 640 属**小**。
  3. 两次把"不同视口下比宽度"当成"折叠前后比宽度"（差里混着窗口宽度变化）。
## 动画（同一轮追加）

- [x] 查官方 motion 页：缓动与时長表（83/167/250/333 ms；三条 cubic-bezier）
- [x] `zephyra_ui::anim`：缓动求值 + 时钟 + 档位映射，**纯函数**，12 条单测全绿
- [x] `InputState::dt_ms`：宿主用 `Instant` 实测帧间隔，两个后端都填
- [ ] **接入未做**：core 没有「容器不透明度」与「绘制时插值偏移」两块能力，
      官方表格里的属性一个都落不下去。记账在 `docs/animation.md` §4，
      没有用「把进度映射到颜色/圆角」去伪装。
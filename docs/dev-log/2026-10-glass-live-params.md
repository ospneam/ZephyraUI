# 折射参数运行期可调 + 死规则反查门禁

日期：2026-10。起因是用户一句话：「折射允许调参数，不要定死」。

---

## 一、现象

液态玻璃页（`gallery --route effects`）的三块玻璃参数**写死在 `.zstyle` 里**：

```css
Container.glass-pane.weak   { filter: refraction(14px, 18px, 0.08, 1.45); }
Container.glass-pane.medium { filter: refraction(28px, 26px, 0.22, 1.5); }
Container.glass-pane.strong { filter: refraction(48px, 34px, 0.40, 1.62); }
```

于是「调参数」只剩一条路：改样式表 → 重新加载 → 看效果。而那与这一页的
**职责**正好相反 —— 它是讲「折射采样的是背景」的实验台，参数应当是**运行期**
的东西。

更深一层的问题是：参数一旦写进源码，`the_whole_tree_writes_no_appearance_in_source`
那条门禁**看不见它**（它的 APPEARANCE 列表里只有颜色/尺寸/圆角那 19 个属性，
没有 `FILTER`），而 `zstylec` 那边也只会在解析期校验语法 —— 也就是说
「参数写在哪」这件事**两条门禁都不管**。

---

## 二、排查

### 2.1 `FILTER` 能不能在源码里带数值

先确认这不是「突破本仓纪律」的做法，三条独立证据：

| 查什么 | 结论 | 出处 |
|---|---|---|
| 门禁拦不拦 `FILTER` | **不拦** | `gallery/tests/gates.rs` 的 APPEARANCE 列表（19 项，无 `FILTER`） |
| 值类型是不是「外观」 | **不是** —— 是物理量 | `Filter::Refraction { strength, bezel, dispersion, ior }` 四个 `f32`，没有 `@令牌` 对应物，不随主题变 |
| 改了能不能被 diff 看见 | **能** | `PropertyValue::Filter` 参与 `PartialEq`，`property.rs` 的相等表里有一项 |

所以「参数进源码」与「颜色进源码」不是一回事：前者是这个 demo **存在的理由**，
后者才是纪律要防的。`gallery/src/app/glass.rs::Glass::filter` 的文档把这三点
写全了，避免下一个人以为可以照抄到 `BACKGROUND` 上。

### 2.2 为什么存**档位下标**而不是 `f32`

`view()` 每帧重建整棵树，参数面板的**数值文本每帧都要画**。若存 `f32` 就
得 `format!(\"%.2\", v)`，而那在热路径上是每帧一次 `Box::leak` ——
`pages::basics::spin_text` 的文档已经记着这个坑。

存下标之后：数值查表、文本查表、加减是 `±1` 后 clamp，**全是整数运算**，
既没有分配也没有浮点累加漂移（0.05 加 20 次会得到 1.0000001，然后与
`.min(1.0)` 的判据打架）。代价是「表必须逐项对齐」，那正是新门禁
`glass_param_tables_line_up` 的活。

### 2.3 三处写错的位置（每一次都当场编译过或测试绿，都不报错）

1. **步进分支插在 `match` 中间写成 `_ =>`。** `click()` 的 `match` 后面还有
   `round-a` / `dd:demo` / `bar-*` 等分支，一个 `_` 把它们全吃成了「不可达」。
   `cargo build` **通过**，只有 4 条 `unreachable pattern` 警告（且默认不 `-D`）。
   症状会是「圆角按钮、下拉、菜单栏全部点了没反应」。修法是挪到 `match` **末尾**。
2. **`FilterChain` 不是 `Copy`，闭包是 `FnMut`。** 三块玻璃共用一条链时，
   `map` 里按值取用捕获变量会 `E0507`。改成每块各自 `FilterChain::single(..)`
   —— 四次数组下标，零分配。
3. **`Control.button.round` 一个元素都选不中**（见第四节）。

---

## 三、排除的假设

### 3.1 「那就给每块玻璃各一组参数，互不干扰」

否。分成独立参数会让「改一个值看效果」变成「改三个值看三个效果」—— 而这页
要教的恰恰是**单项**参数的因果（厚度影响多少像素位移、IOR 到 1 时位移消失）。
三块玻璃共用一组，拖一次就能同时看到「三块一起变」。门禁
`glass_params_are_not_hard_coded` 因此额外断言：改一个参数后，**四块（实为三块）
玻璃上的值必须一致**。

### 3.2 「用滑条调更直观」

core 的滑条目前仍是**展示型控件**：`Gallery::click` 的 `match` 末尾那条分支
明确写着「滑条、表格、Tooltip 点了不改状态 —— core 还没有把它们做成输入
设备」，并指向 `docs/dev-log/README.md`。用滑条做这个面板会得到一个**看得见
摸不着**的控件，而那正是这一页最不该出现的东西。所以用 `[−] [+]` 步进器，
并复用既有的 `hit_test_agrees_with_the_painted_rects`（它逐路由量每个交互节点的
中心是否命中自身）来保证 8 个按钮**真的点得到**。

### 3.3 「四组竖排更整齐」

量过：内容卡 756 高，页头+标题占掉约 100，舞台 255。四行参数面板要 160，
会把舞台顶出客户区 —— 而 core **没有滚动也没有裁剪**（`docs/dev-log/README.md`
记着「core 目前无滚动」），溢出唯一的表现是「下沿被吃掉」。一行四组在 1139 宽
下占 684，实测落在 732 内。所以横排。

### 3.4 「死规则那条门禁查全表（含 15 张控件层）会不会太严」

试过一次，结果是**它会教人做错事**。按全表判时 `Text.focused`、
`Container.toast.error`、`Container.ring-track.paused`、`Container.suggest-row.selected`
等 ~25 条全部报成死规则 —— 而它们是控件层**承诺给宿主**的状态样式，
本 gallery 的界面里根本没有 toast / 输入框 / 建议列表。此时唯一的选择是
**删规则**，那等于把控件层改成「只服务当前 demo」，比留几条暂时用不上的规则
坏得多。所以判据按 `styles::app_only()` 只取应用层两张表。

---

## 四、结论：顺带抓出一条新的失效模式

写「步进按钮压到 28px」时我加了这条规则：

```css
Control.button.round { width: 28px; height: 28px; }
```

`controls::round_button` 实际发的是 `NodeTag::Control` + 分类名 `round` ——
**没有** `button` 这个分类名（`button` 是别的控件用的）。于是这条规则
**一个元素都选不中**，尺寸纹丝不动。

而既有两条门禁**都不会报**：
- `every_class_name_the_source_emits_exists_in_the_stylesheet` 查的是
  「源码发的每个分类名在表里有规则吗」—— 选中的是 `round`，`round` 有规则 ✓
- `every_element_is_reachable_by_the_stylesheet` 查的是
  「每个元素有分类名或裸规则吗」—— 元素有 `round` ✓

两条都是**树 → 表**方向，而漏掉的是**表 → 表**方向：表里有规则，树里没有任何
元素能被它选中。这是本仓第一次出现「门禁全绿但样式没生效」的静默失效。

### 修法与取舍

新门禁 `every_stylesheet_rule_matches_something`：把 11 条路由 × 27 条交互脚本
× 逐交互元素悬停/按住 × 三档断点 × 64 个入场动画采样点的「(类型, 分类名集合)」
全部收集起来，再反查应用层每条规则是否至少命中一次。跑一遍约 5.5 s。

判据取并集而不是只看默认状态，是因为 `.pressed` / `.checked` / `.compact` /
`.enter-N` 只在对应状态选得中元素；只跑默认态会把它们全误报成死规则，然后
**第一次失败就会教人去删规则**。

它当场在应用层抓到 5 条真死规则：

| 规则 | 真相 |
|---|---|
| `Text.app-title` | 源码发的是 `text_label("app-title", …, "text-body-strong")` —— `"app-title"` 是 **key**（给 hit-test 的身份），不是分类名 |
| `Text.page-title` | 同上；且它的值（22px / `@content`）与 `Text.text-title` **逐项相同** |
| `Container.row`（shell、pages 各一） | 重写后没有任何源码用这个编排类 |
| `Control.button.round` | 本轮我自己写错的那条 |

五条全部删除。**删一条可证明选不中的规则不可能改变渲染** —— 这正是这类清理
安全的根据，也是它值得一条门禁的理由。

`Text.app-title` 的值（14px / 二次色）与实际生效的 `Text.text-body-strong`
（15px / 主色）不同，但那条规则从来没生效过，所以删掉画面不变；若哪天想要
「标题栏标题更灰更小」的效果，正确做法是给 `text_label` 第三个参数换一个
分类名并写对应规则，而不是往 `app-title` 上补。

### 门禁

| 门禁 | 钉住什么 |
|---|---|
| `glass_params_are_not_hard_coded` | 四个参数各挪一档 → 树上**该变的那一个字段**变了、**其余三个没变**（只断「变了」挡不住错位写），且三块玻璃参数一致；加完减完回到默认值 |
| `glass_param_tables_line_up` | 数值表与文本表**逐项对齐**、可解析、**严格递增**（非递增时连点两下看起来像没反应）、`IOR` 含 1.00 与 1.50、`BEZEL` 每档 ≥ 2（防除零）、默认档不越界 |
| `glass_param_keys_all_wired` | 8 个按钮都在树上，且**真的点一遍**每个都改了参数（只查 key 在树上挡不住「面板发 key、分派不认」） |
| `glass_param_steppers_are_clamped_without_side_effects` | 边界上是**无副作用的 no-op**；步进次数与档位表长度吻合；挪一个参数不牵动其它三个 |
| `every_stylesheet_rule_matches_something` | 应用层每条规则至少在一个（路由 × 交互态 × 断点 × 入场档）组合里命中一个元素 |

---

## 五、验收

- `cargo fmt --all -- --check` 干净
- `cargo clippy --workspace --all-targets -- -D warnings` —— 我碰的 crate
  （gallery / ui / core / style 相关）0 警告；**全部剩余失败都在 `zephyra-vulkan`**，
  与本轮无关
- `cargo test --workspace` —— **690 passed / 1 failed / 0 ignored**，唯一失败仍是
  `zephyra-winit` 的 `backend_parity::every_blend_mode_looks_the_same_on_both_backends`
  （本轮之前就红）
- `cargo test -p zephyra-winit-d3d12 --test d3d12_draw` —— 16 passed / 3 ignored
  （诊断探针，`AGENTS.md` 明确别为了计数好看去删 `--ignore`）
- `cargo doc --no-deps -p zephyra-gallery` —— 退出 0
- gallery 窗口路（DirectComposition / D3D12）—— `refraction=3`、`unimplemented_filters=0`
- 几何实测（1400×800 画布）：参数面板 `glass-params` 684×44 落在内容卡
  1139×756 内；舞台 `glass-stage` 732×255，底沿 459 < 客户区 756

---

## 六、未做

- **CLI 出图路的参数覆盖**：`--dump-cpu` 不画滤镜，所以「出图时指定折射参数」
  目前没有意义。真要复现某组参数得走 GPU 路（`ZEPHYRA_FRAME_DUMP` 那条），
  那条路参数只能在窗口里点。
- **D3D11 的等价折射像素门禁**：与 D3D12 共用同一份 HLSL 与顶点数据，两条
  真机门禁的内容会重复。
- **Vulkan**：滤镜管线仍未接通，所以 Linux 上这一页与「没设滤镜」无异。
- **liquid-glass 的四层外观装饰**（镜面高光 / 内阴影 / 内边缘亮线 / 玻璃外投影）：
  玻璃外投影需要四边形画到元素矩形之外，而那正是折射四边形覆盖 bug 的成因，
  两者要一起改才安全。
- **`d3d12_draw` 的 17 条 `#[ignore]` 注释已过期**（写着「本机 D3D12 资源创建
  整体不可用」，实测 16 条全过，本机 NVIDIA MX330）。**要不要整体摘掉是独立
  决定** —— `AGENTS.md` 明确说别为了让计数好看去删，本轮不动。

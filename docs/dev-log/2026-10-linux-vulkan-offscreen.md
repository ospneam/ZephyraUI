# 2026-10 Linux Vulkan 离屏 pass + 14 档 PS 变体混合

> 现象 → 排查 → **排除的假设** → 结论 + 门禁
>
> 同一轮做完两件事：离屏（模糊 / 合成）pass，以及 14 档 PS 变体混合模式。
> 前半篇是离屏，后半篇是 PS 变体（附录）。

## 现象

接上离屏链（`plan_segments` → RT → 模糊乒乓 → 合成）之后，Linux 侧冒出两类问题，
而且**症状都不是「崩溃」或「全黑」**：

1. 启动后第 2 帧 `core dump`，无 backtrace（llvmpipe 在 `wait_for_fences` 里死）；
2. 崩溃修掉之后，画面比**接线前**还差：`ZEPHYRA_FRAME_DUMP` 出来的帧里
   两张深蓝卡片整个消失，直方图上 `(26,51,107)` 从 2.3% 掉到 1.61%。

第 2 条是这次真正要记的 —— 它「看起来像接线前的水平」，而接线前其实也不对，
只是错得没那么显眼。

## 排查

按「先定位崩溃、再定位画面」的顺序。崩溃没有 backtrace，只能插桩 + 二分：

- `record_frame` 打点走完 → `submit_and_present` 进去 → 停在 `wait_for_fences`：
  说明命令全部录对了，是 **GPU 侧执行**炸的。本机没有验证层
  （`/usr/share/vulkan/explicit_layer.d` 只有 INTEL_nullhw / MESA 两个），
  所以只能靠「改一个变量、看症状变不变」来二分。

画面那边直接上对照实验：把 `fs_composite` 改成 `return vec4f(1,0,1,1)`，
11474 个品红像素立刻出现在**恰好缺失**的那块区域 —— 由此确认
「合成趟确实在光栅化」，问题在它**采样的那张 RT**。

## 排除的假设

| 假设 | 怎么排除的 |
|---|---|
| 离屏段整体没录进去 | 计划 dump 显示 3 个离屏段、`fx_blurs=2/2/6`、`composite=true`，顶点偏移都填上了 |
| 合成趟没画 | 强制输出品红后 11474 px 出现 —— 画了，只是**画出来是空的** |
| 描述符没绑对 / 池耗尽 | 换成 replace-blend 仍然一个像素不变；按段重置描述符池反而引入了新的段间崩溃（见下） |
| 模糊着色器写错了 | `blur_quad` / `composite_quad` 来自 `zephyra_fx`，与三个后端共用，本轮没动过 |
| `cmd_resolve_image` 用了 `Filter` 参数 | ash 0.37 收的是 `&[ImageResolve]`；`ImageResolve` 只有**一个** `extent`（无 src/dst 之分），改成单次 `.extent(..)` 才编译过 |

## 结论

### 崩溃：四个独立的 Vulkan 规范细节

全部是「不报错、直接崩或直接错」那一类，逐个记：

1. **usage 位**。`create_rt` 只给了 `COLOR_ATTACHMENT`，而模糊趟要**采样**它 →
   缺 `SAMPLED` 直接 segfault。
2. **resolve 的方向搞反了**。`vkCmdResolveImage` 归在 transfer 操作里，
   要求**源**带 `TRANSFER_SRC`、**目标**带 `TRANSFER_DST`；
   我给 MSAA 目标加的是 `TRANSFER_DST`（那是 rt_a 需要的）。
   缺 `TRANSFER_SRC` 时不报错、resolve 静默产出全 0。
3. **`loadOp` 在 `AttachmentDescription` 里**，不在 `RenderPassBeginInfo`。
   我第一版把它当 begin 时参数传，结果形参从未被读、每趟都在 Clear。
   而它又是 render pass **兼容性**的判定项之一 ⇒ 建两条通道就得配两套管线。
   最后收敛成：**一条通道恒 `LOAD`**，清空交给显式的
   `cmd_clear_attachments`（通道内）/ 帧首 `image_barrier`（通道外）。
4. **`initial_layout` 必须与实际布局一致**。MSAA 图刚建好是 `UNDEFINED`，
   通道却声明 `COLOR_ATTACHMENT_OPTIMAL` —— 与声明不符是未定义行为。
   帧首补一条 `UNDEFINED → COLOR_ATTACHMENT_OPTIMAL` 的屏障（`old_layout`
   用 `UNDEFINED` 表示「旧内容不必保留」，于是不必去追踪 RT 当前处在哪个布局）。

另外两个静默错误（不崩、只是画面不对）：

- **`cmd_clear_attachments` 的 rects 不能是空切片** —— 等于一个矩形都不清，
  且不报错。`ClearAttachment.clear_value` 还是 `ClearValue` 联合体而非裸
  `ClearColorValue`。
- **描述符池不能在每段之间 reset**。第 2 段的 reset 把第 1 段已绑定、
  GPU 还要读的描述符集释放了；命令是全部录完才提交的，于是驱动在
  `wait_for_fences` 里读已释放内存才崩。**画面、帧计数、返回值全程正常。**
  现在每帧只 reset 一次（放在 `record_frame` 开头）。

### 画面：`expand_range_translated_into` 会清空 scratch

分段的直觉写法是「对同一个 scratch 逐段调用」。但那个函数每次调用都
`take` 走 `vertices`/`indices`/`runs` 再 `clear()`（见 `quad.rs` 的
`expand_range_translated_into`）—— 于是只剩**最后一段**的几何。
最阴的地方是 `RenderReport` 是逐段 `absorb` 的，**计数照样是对的**，
只有画面不对。

改成：每段展开到独立暂存，再由本层往全帧缓冲拼，并处理两处基址
（顶点基址、索引基址）。顺带把「索引偏移」从录制时推算挪到 `prepare` 里定死 ——
`runs[].first_vertex` 是**段内**坐标，而 `draw_indexed` 要**全帧**坐标，
两者不在同一坐标系；分段拼接下「四边形连续、路径在后」那条假设不成立。

### 画面：RT 里该装什么

真正让两张卡片消失的是这个。`OffscreenPass` 文档写得很清楚：

> `translate`：画进离屏 RT **之前**，几何要平移的量。`DropShadow` 的影子是
> 「剪影模糊后整体平移」，所以平移必须在**模糊之前**做。

也就是说 **RT 装的是「被模糊的那个东西」——这条命令自己**，不是它背后的屏幕。
我最初实现成「把整屏 resolve 进 RT 去模糊」：糊的是背景，而命令本身从没进过 RT，
对 `redraw_original == false` 的纯模糊，结果就是**这条命令整条消失**
（`redraw_original == true` 的投影因为后面还会补画原图，所以看不出来 ——
三个测试用例里恰好有两个是投影）。改成按 `translate` 把命令自身画进 RT。

配套的：RT 尺寸取该段的 `pass.rt_size`（= bounds 向上取整），不是屏幕尺寸 ——
`blur_quad` 的 `local / half_size` 必须与 RT 的真实像素一致，
否则 σ 会整整差一个数量级（模糊几乎看不见，而代码「正常」）。

---

# 附：14 档 PS 变体混合模式（同一轮做完）

## 现象

接线前 `frame()` 的 `未实现混合` 恒为 1（demo 里那条是 `Overlay`），
其余 28 档都走系数表。接线后计数归零，但**先崩了三次**，而且每次症状不同。

## 排除的假设

| 假设 | 怎么排除的 |
|---|---|
| `pipeline_for_blend` 两张表没接上 | 加 `ZEPHYRA_PS=off/no_snapshot/no_resolve/resolve_nosample/snapshot` 五档闸门逐个试 |
| PS 管线本身建不出来 | `no_snapshot` 档（跳过 resolve 与通道重开，只用 PS 管线画）**不崩** ⇒ 管线、两套描述符、draw 调用都没问题 |
| `cmdResolveImage` 有问题 | `resolve_nosample` 档（照常 end → resolve → 屏障 → begin，但 set 1 绑图集）**不崩** ⇒ resolve 与通道开合都没问题 |
| 通道开合破坏了状态 | 同上，`resolve_nosample` 里同样做了 end/begin ⇒ 也不是它 |

两档都过、只有「真的采样那张 RT」才崩 ⇒ 回到 usage 位。

## 结论

1. **`pipeline_for_blend` 里不能写 `blend_mode_index(blend)?`。**
   `?` 把「不在系数表里」当成「这个模式完全不可用」直接从函数返回，
   14 档 PS 变体永远走不到第二次查表 —— 症状是**全部退回默认混合**，
   计数从 1 变 0，画面看不出任何异常（默认混合本身是合法的画法）。
2. **目标副本的 usage 少 `SAMPLED` 就崩。** `COLOR_ATTACHMENT | TRANSFER_DST`
   建出来的图绑进描述符采样，llvmpipe 不报错、直接在片元里 core dump。
   与 `create_rt` 同一个坑，**本轮踩了两次** —— 值得单独立一条规矩：
   凡是要绑进描述符的图，`SAMPLED` 不是「按需」而是「必需」。

   更难受的是它与另两个位绑在一起：`COLOR_ATTACHMENT`（resolve 目标要求）
   与 `TRANSFER_DST`（spec 要求 resolve 目标带）都是「不加就静默出错」，
   三个位凑齐才不崩，而崩溃点在最远的片元阶段。

3. **PS 变体必须重开通道。** 目标此刻是附件，Vulkan 不允许「既是附件又被采样」，
   所以顺序是：结束主通道 → `cmdResolveImage` 到目标副本 RT →
   屏障转 `SHADER_READ_ONLY_OPTIMAL` → 重开主通道（`loadOp=LOAD`）→ 画。
   代价是每个 PS run 多一次通道开合 + 一次整屏 resolve。

4. **描述符池从 256 提到 1024。** 每个 draw 现在占**两套** set
   （set 0 纹理+采样器、set 1 目标），几百个 draw 时 256 不够 ——
   而「不够」的失败形状是 `allocate_descriptor_sets` 报错、那个 draw 被
   `continue` 跳过，画面少一块且零报错。

## 门禁

- `未实现混合` 由 1 → **0**（demo，185 quad / 137 640 字节）
- 与 Windows 参考帧的对比**没有回退**：95.79% 逐字节一致（接 PS 变体前后同值），
  直方图前 6 大颜色仍逐项对齐（`(26,51,107)` 3.08% vs 3.08%、
  `(61,132,188)` 0.72% vs 0.72%）
- `cargo clippy --workspace --all-targets -- -D warnings` → 0
- `cargo test --workspace` → 660 passed / 0 failed / 3 ignored

## 仍未定位（记账）

剩余差异集中在三处，**形状逐像素一致、只有颜色不同**：

| 区域 | 差异 |
|---|---|
| `x∈[0,183] y∈[384,643]` 184×260 | `(5,6,9)`↔`(40,44,52)`、`(202,202,212)`↔`(230,230,235)`，各 3786 px |
| `x∈[155,189] y∈[598,632]` 35×35 | `(255,102,12)`↔`(255,170,60)`，1020 px |
| `x∈[456,527] y∈[320,358]` 72×39 | `(26,40,65)`↔`(115,115,115)`，2782 px |

把第一块裁出来左右拼在一起看（放大 2×）就明白了：那是 demo 的
**alpha 棋盘格 + 渐变条 + 橙色圆点 + 混色卡片**一组测试用例，
两边布局、文字、图形**逐像素对得上**，只有棋盘格的深/浅两色、
渐变条的取样值、圆点的色相不同。

### 已排除的

- **全局偏移**：残余扫描最优点 dx=0 dy=0（逐行探针也确认）。
- **颜色表不同**：直方图前 6 大颜色逐项对齐。
- **布局 / 几何问题**：位置集合、包围盒、像素数三者在成对分组里都相同。

### 未排除的，以及为什么现在查不动

⚠ **Windows 侧的可信参考帧在本机拿不到**。`ZEPHYRA_FRAME_DUMP` 在 Windows 上
走 DirectComposition 路径，实测写出的 BMP 是空帧（`颜色数 1`，且伴随
`提交失败: Close 命令列表失败: 参数错误(0x80070057)`）；关掉 DComp 也没有
可用的回退路径。现有参考帧 `win-demo-client.png` 是**屏幕截图**，而日志里
明写 `inner_size(物理) 1100×700 scale 1.25` —— 也就是说那张图**经过了 1.25×
的 DPI 缩放**，外加上下翻转与 42 px 标题栏。

在这三重采集误差之下，「某几个测试色块的颜色差 ±1 档」这件事**无法归因**：
既可能是渲染差异，也可能是重采样。要判干净只有一条路——
把 Windows 的 in-app readback 修好（先解决 `0x80070057` 那个关闭命令列表失败），
拿到同尺寸同朝向的 BMP 再比。在那之前，这 4.21% 只能记成「未判定」，
不能记成「已修」。

下一轮起点：修 Windows 的 `Close 命令列表` 失败（`0x80070057`），
拿到可信参考帧后再回来判这三处。

## 门禁（两件事合起来）

- `cargo clippy --workspace --all-targets -- -D warnings` → 0 警告
- `cargo fmt --all -- --check` → 干净
- `cargo test --workspace` → **660 passed / 0 failed / 3 ignored**（Linux，WSL2）
- `未实现混合` 计数 1 → **0**（14 档 PS 变体全部接线）
- 与 Windows D3D12 参考帧的实测对比（demo，1100×700，4 段离屏、185 quad）：

  | 指标 | 接离线屏 | 接线后 |
  |---|---|---|
  | `(26,51,107)` 占幅 vs Windows | 1.61% vs 3.08% | **3.08% vs 3.08%** |
  | `(61,132,188)` 占幅 vs Windows | 0.37% vs 0.72% | **0.72% vs 0.72%** |
  | 逐字节相同 | — | **95.79%** |
  | 差 ≤ 8 | — | 97.38% |

  ⚠ 对照前必须先纠正参考帧：`win-demo-client.png` 是**屏幕**截图，
  相对 Linux 客户区**上下翻转**且多 42 px 标题栏。不纠正就比，会得出
  「差 20%、而且差在最上面 200 行」的结论 —— 那是截图的锅，不是渲染的。
  判据（`wherecolor.py`）：两图里同一控件的行号之和恒定（`wy = 650 - ly`）。

  剩余 4.21% 的差异集中在三处，尚未定位：
  `(5,6,9)/(202,202,212)` ↔ `(40,44,52)/(230,230,235)` 各 3786 px（同一个控件配色不同）、
  `(152,110,87)` ↔ `(115,115,115)` 2780 px、`(255,102,12)` ↔ `(255,170,60)` 1020 px。
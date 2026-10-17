# 2026-10 Linux GPU 后端：wgpu → 裸 Vulkan（ash）移植

## 现象

按用户要求把 Linux 后端从 wgpu 换成**手写 Vulkan**（ash 绑定）。移植后的 demo
在字体加载完成后立即 **native segfault**（core dump），且 **Rust backtrace 完全为空**
—— 不是 panic，是进程被信号杀死。

## 排查

在 `Renderer::new` 的每个步骤插入 `eprintln!` 逐步收窄，最终定位到
`vkAllocateMemory`（建顶点缓冲）处崩溃。但修复该处后崩溃点**漂移**——这是
「内存被卸载/悬垂」类故障的典型特征，而不是参数错误。

真正的根因与着色器、内存都无关：

**`Gpu::new` 里的 `ash::Entry` 是局部变量，函数返回时被 drop。**
`ash::Entry` 持有 `libloading::Library`（dlopen 的 libvulkan 句柄），drop 即
`dlclose`。之后 `ash::Instance` / `ash::Device` 函数表里的所有指针都指向
**已卸载的库**。Linux 的延迟卸载让前几个调用（描述符集布局、采样器、
`vkCreateBuffer`）碰巧成功，直到 `vkAllocateMemory` 踩到被回收的内存。

修复：`Gpu` 持有 `entry`，与 Instance/Device 同寿命。**探针不崩的原因**：
它的 `entry` 在 `main` 里，活到进程结束——同一个 bug 在两种所有权形状下
一崩一不崩，这就是它难查的原因。

## 排除的假设

| 假设 | 为什么排除 |
|---|---|
| CString 临时值悬垂（`.name(CString::new(..).unwrap().as_c_str())`） | 确实是 bug 且已修，但修后仍崩——它是**次生**问题不是根因 |
| ash builder 的内联 `&[...]` 临时数组悬垂（「先 let 后用」模式） | 同上：真实 bug（builder 存引用、临时数组语句末释放），修复 5 处后仍崩 |
| SPIR-V 对齐（include_bytes! 对齐 1 强转 `&[u32]` 对齐 4） | UB 实锤、已修（改为逐 word 复制），但不是崩溃根因 |
| 命令录制时序（`frame()` 里录命令时 cmd 尚未 begin） | 架构错误、已重构（`prepare`/`record_draws` 两段式），但与 segfault 无关 |
| `maxMemoryAllocationCount` 超限 | 只分配 1 次就崩，且 llvmpipe 上限远大于 1 |
| SPIR-V 无效 | naga 30（与 wgpu 30 同一前端）编译通过，8615 words |
| Wayland surface 不可用 | 探针实测可用；反而是 **X11** 路线走不通（winit 的 X11 后端要 dlopen `libxkbcommon-x11.so.0`，本机没有、装它要 sudo） |

另有一条 Windows 侧的意外发现：D3D12 后端的读回路径
（`Offscreen::record_readback`）**只被 18 条 `#[ignore]` 诊断探针引用过**，
在本机从未真正跑过——实测它让 `Close()` 稳定回 `E_INVALIDARG` 且整帧提交失败。
相关改动已全部撤回（改用屏幕捕获取 Windows 参考帧）。

## 结论 + 门禁

raw Vulkan（ash 0.37）后端 `zephyra-vulkan` 全链路跑通：

- instance → Wayland surface → llvmpipe 设备 → swapchain（B8G8R8A8_SRGB、4 images、
  PRE_MULTIPLIED）→ present → **读回 256000/256000 个目标像素**（探针验证）
- demo：1100×700、137400 字节顶点、**184 个四边形**（与 wgpu 版逐字节同数）、
  15 条混合模式管线、帧循环稳定
- gallery（完整控件库）同后端构建通过
- wgpu 版 `zephyra-winit-vk` 已删除，`examples` / `ZephyraUI` 的 `linux-vk`
  feature 全部指向 `zephyra-vulkan`

已知边界（诚实记账）：

- 着色器是**同一份 WGSL**（从 wgpu 版拷贝），构建期经 naga 编成 SPIR-V
  （等价于 Windows 侧 fxc 构建 HLSL）——对照测试仍是单一事实来源的守门人
- MSAA 暂为单采样（renderpass 需要 resolve 附件，后续补）；
  离屏模糊/合成、PS 变体混合（14 档）暂未接线（退回 SrcOver 并计数，
  与 wgpu 版早期策略一致）
- ash 钉在 **0.37**：0.38 删除了全部包装方法（只剩原始函数表），迁移是
  纯机械工作，等需要时再做
- 参考帧测量改用屏幕捕获（DPI-aware、客户区 1:1）——in-app 读回路径
  在 D3D12 侧从未被验证过（见上）

运行方式（Linux/WSL2）：

```bash
cargo run -p zephyra-foundation --bin demo --no-default-features --features linux-vk
```

# 2026-10-17 Linux 适配（WSL2 实测）

> 现象：在 WSL2（Ubuntu 24.04）上 `cargo check --workspace` 直接失败；修掉之后
> `cargo clippy --workspace --all-targets` 又在 `examples/demo.rs` 上失败；而
> `cargo test --workspace` 的第一条红是 `ZephyraCLI` 的
> `display_name_matches_what_the_user_types` —— 一个与本次改动毫无关系的 CLI 用例。

## 排查

分四段，每段都有独立的失败原因。**没有一段是「Linux 不支持」**。

### 1. `windows` crate 在非 Windows 上不是「能用但没东西」

```
windows-future-0.3.2/src/bindings.rs:87:51: error[E0425]:
  cannot find type `IMarshal` in module `windows_core::imp`
```

`zephyra-winit-d3d12` 的 `windows` 依赖**没有** target-gate，而它的传递依赖
`windows-future` 在非 Windows 目标上直接编译失败。断点在一个与本仓无关的第三方
crate 里，看起来像「整个 workspace 都坏了」。

### 2. 一个**纯 `std`** 的模块挂着 `#![cfg(windows)]`

`zephyra-fx/src/cpu_raster.rs` 顶部有 `#![cfg(windows)]`，于是非 Windows 上
`cpu_raster` 整个消失，`cpu_only` 跟着报 `unresolved import crate::cpu_raster`。
它是照着同目录的宿主 crate 抄来的 —— 那几个确实是 Windows 专属，而这个是
CPU 光栅器，一个系统 API 都不碰。副作用是 `--dump-cpu`（本仓唯一的像素级自验手段）
在 Linux 上**整个不存在**。

### 3. 集成测试是**独立目标**，不受宿主 crate 的门禁保护

`#![cfg(windows)]` 只让 lib 变空；`tests/*.rs` 照常参与编译。9 个文件因此在
Linux 上编不过（另外 5 个文件本来就有这道门禁）。

### 4. `--workspace` 在 Linux 上用的是**默认 feature**

`examples` 的 `default = ["d3d12"]`。而 d3d12 那个 crate 在 Linux 上是空的，
于是 `use zephyra_winit_d3d12::HostWindow` 报「找不到」。症状极具误导性：看起来像
「Linux 适配把 demo 弄坏了」，真相只是「默认 feature 指向一个在本平台为空的 crate」。

## 排除的假设

这一段是本条目最值钱的部分 —— 下面每一条都**实测排除**过。

| 假设 | 实测 | 结论 |
|---|---|---|
| 「wgpu 需要 `pkg-config` + vulkan 头文件」 | 一台**只有**运行时库（`libvulkan1` + mesa drivers）、**没有** `pkg-config`、没有 `libvulkan-dev` 的 WSL 上，`cargo build` 直接通过（45 s，255 个依赖） | **不需要**。ash/wgpu 都 dlopen `libvulkan`。这一条是「不用 apt 就能构建」的决定性理由 |
| 「WSLg 下开不了窗」 | `EventLoop::new()` + `create_window` + wgpu surface + 3 帧 present 全部成功 | 能开。adapter 是 `llvmpipe`（软件 Vulkan，WSL 没有真 GPU） |
| 「ws 必须改用 Wayland subsurface」 | winit 0.30 默认在 WSLg 上拿到 X11（`DISPLAY=:0`），走 GLX/XCB 就够 | 不必。Vulkan 的 `VK_KHR_xlib_surface` 直接可用 |
| 「NDC 的 y 方向要翻」 | `quad.rs` 已经把 DIP 翻成 NDC，WGSL 里照抄即可 | 不用翻。**真正**要翻的是 `g_dst` 的采样 uv（`0.5 - ndc.y*0.5`）—— 那是 D3D 那份就有的 |
| 「surface 格式可以挑一个好看的」 | 挑 `Rgba8Unorm` → `Requested format ... is not in list of supported formats: [Bgra8UnormSrgb, Bgra8Unorm]` | **取 caps 里的第一个**，且管线必须按它编译 |
| 「`write_buffer` 不必管对齐」 | gallery 首帧 panic：`Copy size 19686 does not respect COPY_BUFFER_ALIGNMENT` | 必须补齐到 4。**而且只在索引个数为奇数的那一帧出现** —— demo 的索引恰好是偶数，所以它一路躲过去了 |
| 「BMP dump 传 `encode_srgb = true` 就行」（照抄 Windows） | 整张图 209 572/256 000 个像素是纯黑 | **不能照抄**。Linux 的 surface 是 `Bgra8UnormSrgb`，读回来**已经是** sRGB，再编码一次等于把 sRGB 当线性 |
| 「渲染对了 = 没报错」 | — | 不成立。本条里「滤镜执行了没有」「文字重叠了没有」都是靠 `ZEPHYRA_FRAME_DUMP` 出图 + 与 `--dump-cpu` 对照才发现的 |

## 结论

- **新增两个 crate**（互不依赖，与 Windows 那两个后端同构）：
  - `zephyra/zephyra-base/zephyra-platform-linux` —— 平台能力实现（gsettings /
    `GTK_THEME` / `LANGUAGE` / `/etc/os-release`）。零依赖、零 unsafe。
  - `zephyra/zephyra-base/zephyra-winit-vk` —— winit + wgpu 后端（Vulkan 优先、
    GL 亦可），含 WGSL 版单一着色器、MSAA、离屏 blur/composite、几何跨帧复用。
- **宿主选择的判据从「feature」改成「平台 **且** feature」**。Rust 没有 cfg 别名，
  同一判据在 `demo.rs` 里出现三次，每次都写全 —— 少写平台那一维就是第 4 条那个坑。
- **四个真实缺陷**（都是「能编」与「能跑」之间的）：
  1. 带滤镜的命令被画两遍（分段时展开并画了，又走离屏画了一遍）⇒ 元素比该有的亮，
     且 `unimplemented_filters` 永远减不下去；
  2. `write_buffer` 未按 4 字节对齐补齐 ⇒ 索引数为奇数时 panic；
  3. `display_name` 用 `Path::file_name()` ⇒ Unix 上 `\` 是普通字符，`E:\...\zephyra.exe`
     原样打印（这不只是测试数据的问题：WSL 互操作里从 bash 调 Windows 版可执行文件时
     `argv[0]` **真的**就是 `E:\...`）；
  4. dump 的 sRGB 双重编码。
- **14 档 PS 变体混合模式未接线**（`Overlay`..`Luminosity`）。WGSL 入口写好了，但
  wgpu 下「正在渲染的附件」不能被采样（MSAA 更不支持采样），接通需要
  「resolve 到中间纹理 → 拷贝 → 第二个通道」。现状照常画、退回 `SrcOver`、计入
  `unimplemented_blends`。
- **滚轮不路由**：核心里 `InputRouter` 只有 `on_pointer`，滚轮手势还没进那套词汇表。
  接线点在 core，不在 base —— 所以这里只标脏，不在宿主里造私有的「滚轮 = 平移」语义。
- **两个方向的 `cargo doc` 各自踩了一次**：
  - 新写的 `zephyra-winit-vk` 用 `#![cfg(any(target_os = "linux", ...))]` 整体关掉，
    而 crate 级文档里有指向自家模块的 intra-doc 链接 ⇒ **Windows** 上
    `cargo doc --workspace` 退出 101。修法：那一段改成代码块。
    ⚠ 修的过程中**注释里为了举例又写了一个模块链接**，于是又红一次 —— 解释本身
    也不能带链接。
  - 反过来，三个 Windows-only crate 让 **Linux** 上的 `cargo doc --workspace` 必然失败
    （空 crate + `deny(broken_intra_doc_links)`）。**这一条是既有的**（那几个 crate 的
    `src/` 本轮一行没动），Linux 侧只能用逐 crate 的 doc 命令。

## 门禁

| 钉住的东西 | 门禁 |
|---|---|
| 平台能力解析（版本 / 深浅色 / 语言） | `cargo test -p zephyra-platform-linux`（19 条，纯函数 + 「首帧不炸」） |
| 混合模式入口顺序、预乘系数、alpha 对偶、顶点偏移 | `cargo test -p zephyra-winit-vk`（12 条） |
| Linux 上整仓能编、能测 | `cargo clippy --workspace --all-targets -- -D warnings` 与 `cargo test --workspace`（Linux：672 / 0 / 3 ignored） |
| Linux 后端真的被编一遍 | `cargo clippy -p zephyra-foundation -p zephyra-ui --all-targets --no-default-features --features zephyra-foundation/linux-vk,zephyra-ui/linux-vk -- -D warnings` |
| 「画对了」而不是「没报错」 | `ZEPHYRA_FRAME_DUMP=…` 出图，与 `--dump-cpu` 的 CPU 参考图对照 |
| surface 格式 / 对齐 / sRGB 三个坑 | 目前靠上面那条人工对照；**没有自动化门禁** —— 这是本条目留下的最大缺口 |
| 空 crate 的 crate 级文档链接 | 无自动化门禁，只能靠人记住「crate 级文档不用 intra-doc 链接」；§结论 里那两条 `cargo doc` 的方向性失败就是它的症状 |

⚠ 最后一行的缺口是**故意记下来的**：`--workspace` 在 CI / WSL 上不建窗口，所以
「真的画出像素」这件事目前只有人工路径。补法已经清楚（给 `zephyra-winit-vk` 加一个
离屏渲染 + 读回像素的集成测试，与 CPU 参考图逐像素比），但那要在
`zephyra-fx` 的 CPU 光栅器与 GPU 输出之间定义「允许的差异」（抗锯齿与字形采样），
不是一行测试能定下来的。

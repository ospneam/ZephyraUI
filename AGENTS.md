# AGENTS.md

> 给 AI 代理与新来的人看的**仓库规矩**。人手写的东西放 `docs/`，别放这里。

本文件回答三件事：**去哪读、怎么跑与验、哪些边界不能碰**。

---

## 一、先读这三处

| 文件 | 什么时候读 |
|---|---|
| [`docs/developer/README.md`](docs/developer/README.md) | 对外规范：数据流 / 样式表 / 帧流水线 / 后端 |
| [`docs/dev-log/README.md`](docs/dev-log/README.md) | 本仓**排除过的假设**。动 GPU / 呈现 / resize / 动画之前必读；末尾「长期存在的问题」写着哪些事**别当成已完成** |
| [`zephyra/zephyra-base/README.md`](zephyra/zephyra-base/README.md) | 三层结构、base 的三条禁令、这一层还缺什么 |

⚠ 文档里有一批路径还是上次搬家前的写法：`docs/win/…` 现为 `docs/developer/win/…`，
`docs/portability.md` 现为 `docs/developer/portability.md`。
`ZephyraUI/README.md` 顶部那两条 Markdown 链接是真断的，别照着改错位置。

---

## 二、跑与验

| 门禁 | 命令 |
|---|---|
| 格式 | `cargo fmt --all -- --check` |
| 代码规范 | `cargo clippy --workspace --all-targets -- -D warnings` |
| 测试 | `cargo test --workspace` —— 772 通过 / 0 失败 / 21 ignored（本机 2026-10） |
| 文档 | `cargo doc --no-deps --workspace` —— 退出 0。**警告数取决于重编范围**（warm 全量实测 43 条，全部来自 `zephyra-ui` / `zephyra-fx` / `zephyra-style` / `zephyra-winit-d3d12` 的既有链接；判据是「本轮有没有**新增**」而不是总数）。bin 目标不受门禁保护 |

单点验证：

```text
cargo test -p zephyra-core <filter>          # 单 crate / 单条用例
cargo test -p zephyra-cli                    # 21 条，每条都是实测踩过的坑
cargo test -p zephyra-dcomp --lib            # 纯逻辑断言，不要窗口
cargo test -p zephyra-dcomp --test smoke     # 真窗口 + 真 GPU
cargo run -p zephyra-ui --bin gallery -- --list-routes --dump-cpu --route <id>
cargo run -p zephyra-foundation --bin demo -- --dump-cpu --bench --alloc-probe
cargo run -p zstylec -- check <file.zstyle>  # 也可用 zephyra check（同一份编译器）
```

### `--workspace` 不是可选项

`default-members` 只有 core / fx / zephyra-winit / examples，所以裸 `cargo test`
只跑 **534 条**。另外这些 crate 在 `members` 里但**不在** `default-members`，
无差别的 `cargo build` / `cargo test` 连编都不编（这是有意的：默认构建不该编译
unsafe 与设备代码）：`zstylec`、`zephyra-style`、`zephyra-cli`、`zephyra-camera`、
`zephyra-dcomp`、`zephyra-platform`、`zephyra-platform-win`、
`zephyra-platform-linux`、`zephyra-winit-d3d12`、`zephyra-winit-vk`、`zephyra-ui`。
要跑要编就 `-p` 点名。

那 21 条 `ignored` 全在 `zephyra-winit/` 与 `zephyra-winit-d3d12/` 的测试里（诊断探针：
`d3d12_draw.rs` / `sweep.rs` / `pso_bisect.rs` 之类），它们会开真窗口、历史上真把设备
拖进过 TDR，所以默认 `#[ignore]`。**别为了让计数好看去删 `--ignore`。**

注意 `--workspace` 会把需要**真窗口 + 真 GPU** 的套件也跑起来（`zephyra-dcomp` 的
`smoke` 3 条不在 ignore 之列）。要在无显示器的机器上只验逻辑，退回 `cargo test`
（default-members）或显式 `--lib`。

### Linux（WSL2 实测，2026-10）

同一套门禁在 Linux 上可用，数字不同：

```bash
cargo fmt --all -- --check                                  # 干净
cargo clippy --workspace --all-targets -- -D warnings      # 0 警告
cargo test --workspace                                      # 672 / 0 / 3 ignored
```

⚠ 两个非显然的地方：

1. **`--workspace` 在 Linux 上编的是「没有宿主」那一态。** `examples` 的
   `default = ["d3d12"]`，而 d3d12 那个 crate 在非 Windows 上是空的
   （`#![cfg(windows)]`）。所以**判据必须是「平台 且 feature」**，只看 feature 会
   在 `use zephyra_winit_d3d12::HostWindow` 上炸 —— 症状像「Linux 适配把 demo 弄坏了」，
   真相只是「默认 feature 指向一个本平台为空的 crate」。真要把 Linux 后端编一遍：

   ```bash
   cargo clippy -p zephyra-foundation -p zephyra-ui --all-targets \
       --no-default-features \
       --features zephyra-foundation/linux-vk,zephyra-ui/linux-vk -- -D warnings
   cargo run -p zephyra-foundation --bin demo   --no-default-features --features linux-vk
   cargo run -p zephyra-ui --bin gallery        --no-default-features --features linux-vk
   ```

   只给 `--features linux-vk` 而**不**给 `--no-default-features` 会把 d3d12 也编进来：
   能跑（`#[cfg]` 选中的那个出画面），但多编一遍 D3D12 的依赖链。

2. **Linux 上测试数比 Windows 少 ~100 条**，因为 Windows-only 的套件
   （`zephyra-winit` / `zephyra-winit-d3d12` / `zephyra-dcomp` /
   `zephyra-platform-win` 的 `tests/`，以及那 18 条 `ignored` 诊断探针）在那边根本不存在。
   **不是**少跑了什么。

3. **`cargo doc --workspace` 在 Linux 上必然失败**，失败的是三个 Windows-only crate
   （`zephyra-winit` / `zephyra-winit-d3d12` / `zephyra-dcomp`）：它们在 Linux 上是
   空 crate，而 rustdoc 照样解析 crate 级文档，于是指向自家模块的链接解析不到，
   配上 `#![deny(rustdoc::broken_intra_doc_links)]` 就**整个失败**。这不是本轮引入的
   （那三个 crate 的 `src/` 本轮一行没动）。Linux 侧改用逐 crate：

   ```bash
   cargo doc --no-deps -p zephyra-platform-linux -p zephyra-winit-vk \
       -p zephyra-core -p zephyra-fx -p zephyra-platform      # 退出 0
   ```

   **新写 crate 级文档时**：指向自家模块的引用一律写**代码块**，不要用 intra-doc
   链接 —— 否则那个 crate 一旦被 `#![cfg(...)]` 整体关掉，就会在**别的平台**上
   把 doc 门禁弄红。模块**内部**随便用链接。

### WSL 上构建本仓的环境前置（**不是**仓库配置，别写进 manifest）

| 事项 | 做法 | 为什么 |
|---|---|---|
| `CARGO_TARGET_DIR` 指到 ext4 | `export CARGO_TARGET_DIR=~/zephyra-target` | 仓库在 `/mnt/e`（NTFS）上，Linux 构建绝不能写进 `E:\ZephyraUI\target`（那是 Windows 产物） |
| crates.io 走镜像 | `~/.cargo/config.toml` 里 `replace-with = 'rsproxy-sparse'` | 实测直连 `static.crates.io` 约 170 KB/s，`static.rust-lang.org` 约 37 KB/s，rsproxy 约 7 MB/s |
| 编译并发 | `-j 6` | WSL 常见 4 GB 内存；wgpu 依赖树并行编译容易 OOM |
| **不需要** `apt install` | 什么都不装也能构建 | wgpu / ash 都 dlopen `libvulkan`；实测在**没有** `pkg-config`、没有 vulkan 头文件的机器上 45 s 编完 255 个依赖 |

---

## 三、四条不能碰的边界

1. **「这一帧画什么」只有一个地方决定**：`zephyra_core::frame::FrameCache`。
   宿主只回答「要不要推上屏」，不读 `Node::dirty()` 决策内容。这条被破过
   （`HostWindow` 曾长期存 `node` / `list` / `needs_layout` 三块状态），破了的
   症状就是本仓 GPU 问题反复出现的根源，见 `docs/dev-log/2026-10-gpu-device-hung.md`。
2. **core 零依赖**：`cargo tree -p zephyra-core --depth 1` 只输出它自己一行。
   core 与 fx `forbid(unsafe_code)`；unsafe 只许出现在 `zephyra-base/`。
3. **外观不进源码**：控件只发 `key`（事件身份）与分类名（选择器），颜色 / 尺寸 /
   圆角全在 `.zstyle`，颜色走 `@令牌` → `Theme::role()`。`ZephyraUI/tests/gates.rs`
   按**整棵树**判（遍历全部页面 × 全部交互态），比你想的更容易红。
4. **三个渲染后端互不依赖**：`zephyra-winit`（D3D11）/ `zephyra-winit-d3d12` /
   `zephyra-winit-vk`（wgpu）只共同依赖 `zephyra-core` + `zephyra-fx`。**不要**把
   Linux 后端并进 Windows 那两个，也不要在宿主里写 `#[cfg(windows)]` 分支挑后端 ——
   D3D11 / D3D12 / Vulkan 是三套词汇，揉进一处后「画面一空时是哪个后端的问题」
   就没法回答了。后端由**特性开关**选，同一个二进制只会有一个。
   跨后端共享的只有「后端无关的那一半」，它住在 `zephyra-fx`
   （`DisplayList → 顶点流` / 字形图集 / 纹理表 / CPU 光栅器）。
   ⚠ `shaders/` 的 WGSL 与 HLSL 是**手抄的两份**，不是自动同步的 ——
   对应关系写在 `zephyra-winit-vk/shaders/zephyra.wgsl` 的文件头，靠纯逻辑测试 +
   `--dump-cpu` 对照钉住。

### 加一个属性要动两处

`zephyra-core/src/props.rs` → `zstylec/src/props_map.rs`（kebab-case 映射表，
`.zstyle` 侧**唯一**入口）。

- `affects_layout` 有门禁兜底：`layout_classification_tests.rs` 会因漏声明而红。
  漏声明的后果是「只改颜色」也触发全树重排。
- **映射表那一侧没有门禁**：`props_map.rs` 的模块文档写着「B2 里那条对齐测试会红」，
  但那条测试并不存在。所以漏加映射的症状是——那个属性在样式表里**永远设不了，
  且没有任何提示**。加属性时自己核对，别指望测试喊你。

---

## 四、注释与代码

### 1. 核心原则

1. **注释解释 WHY，不解释 WHAT** —— 设计意图、原因、边界情况。
2. **代码自解释优先** —— 靠命名与结构减少注释需求，注释是补充手段。
3. **注释必须准确、简洁、有用** —— 废话、误导、冗余都不要。
4. **注释需与代码同步更新** —— 过期注释比没有注释更糟。

### 2. 模块文档（`//!`）与函数文档（`///`）

模块文档超过一屏就搬去 `docs/`，源码只留三行摘要加一个链接。

**每个实测数字必须带测量条件**——一个孤零零的 `162 µs` 半年后没人知道是哪个场景、
什么分辨率、多少节点测的，那时它比没有更糟。

```rust
/// 四阶段合计 162 µs（demo，45 节点，1400×800）
// ↑ 数字 + 条件 + 规模，缺一不可
```

### 3. 本仓硬约定

1. **行内 `//`：写「不这样写就一定会错」的东西**，例如某个 API 要求 `flush` 之前
   不能改描述符、某段顺序被 spec 钉死、某处漏了会 panic。解释结构的搬进 `///`。
   （注：现状是生产源文件里约 3,300 行行内注释，`zephyra-base` 与 core/fx 占八成，
   Win32/COM/GPU 边界与不变量说明是它们的正当位置——所以判据是「说了会不会错」，
   不是「有没有」。）
2. **行内注释禁止出现十六进制回码**（`0x887A0001` 这类）。它们属于 `docs/dev-log/`：
   回码会随 Windows 版本变化，而它恰恰**最像**有价值的注释——越像，越该放到有日期、
   会复核的地方。
3. **定位信息必须与结论成对出现。**
   - 「Chromium `dc_layer_tree.cc:626` 是在提交之后才通知」✅ —— 路径是证据，
     去掉它这条结论就没法复核。
   - 「见 `foo.cc:81`」❌ —— 只有路径没有结论。这类是纯噪音，而且会随上游改动
     悄悄指错地方。

   十六进制回码（`0x887A0001`）**保留**，但必须与符号名成对：
   `DXGI_ERROR_INVALID_CALL（0x887A0001）`。符号名会随文档变，裸码不会；
   只写符号名又会让「去哪儿搜」变麻烦。
4. **标识符一律英文**（中文标识符已在 `a8c5a4a` 全量清零）。中文只出现在两处：
   文档注释的散文，以及**测试函数名**——`unchanged_之后不动脏标记` 这种断言命题
   是本仓写法（约 30 条测试这么写）。
5. **测试体不进生产文件**：源码里只留声明，实现放独立文件。

   ```rust
   #[cfg(test)]
   #[path = "tests/filter_tests.rs"]
   mod tests;
   ```

   用 `#[path]` 而不是直接改文件名，是为了让模块路径不变——`use super::*` 与对
   父模块私有项的访问照旧，不必把私有项提成 `pub(crate)`。
   `zephyra-core/src/lib.rs` 指向的是同目录的 `*_tests.rs`，属于另一种「本来就分离」。
   `examples/` 是特例：`tests/` 已被 cargo 集成测试占用，故另开 `testmods/`。
6. **命名规范就是「通过 clippy 的 `upper_case_acronyms` 且 clippy 零警告」**：
   `SRV` / `PSO` / `ID` 全大写。不要另立「缩写要不要展开」「变量该多长」这类规矩——
   机器查不了、也没有共识，写进规范只会变成口水仗。
7. **workspace 内依赖一律路径依赖，且必须同属一个 workspace**：`ZephyraCLI` 与
   `zstylec` 同 workspace，所以「CLI 看到的属性表」与「编译器看到的属性表」
   在结构上不可能是两份——否则会出现「CLI 说没错、编译器说有错」。
8. **不加「注释必须少」这类 lint**：它会逼出废话注释凑数，与第 1 条正着来。
   注释长度只能靠评审 + 下面那些间接门禁约束。

---

## 五、环境与工具链的坑

- `cargo fmt` 会刷一屏 `Warning: can't set wrap_comments = false`——那是 nightly-only
  选项，stable 直接忽略（stable 本来就不折行注释）。**别去删配置、别为它改 nightly。**
  实测：stable 上对 `zephyra-core/src/frame.rs` 跑一遍 rustfmt 零 diff，那张 µs 对照表
  的列对齐不会被冲散。
- **PowerShell 默认编码把中文打成乱码，文件本身是真 UTF-8。** 别照着 shell 输出
  「修」注释。仓库里已有 **54 个 U+FFFD 分布在 21 个文件**（如 `examples/demo.rs:9`、
  `zephyra-fx/src/quad.rs`），是历史损坏——看到别扩散，也别当成待修的谜团。
- **一个二进制只编一个后端**：`d3d12` / `d3d11` 由 feature 选，不做运行时切换
  （两个都编进来时「画面一空」无法归因）。`demo` 的 bin 带
  `required-features = ["d3d12"]`，选 d3d11 必须
  `--no-default-features --features d3d11`，否则 bin 编不过——这是有意的，不是 bug。
- 环境变量：`ZEPHYRA_TRACE=1`（每帧统计，`zephyra dev` 默认开）、`ZEPHYRA_DCOMP`
  （切 DirectComposition 呈现路径）、`ZEPHYRA_FRAME_DUMP=<path>`（把某帧后缓冲写成
  BMP，给「肉眼才看得见」的故障留证据）、`ZEPHYRA_MSAA`、`ZEPHYRA_ADAPTER`、
  `ZEPHYRA_FONT`（字体覆盖）。`--dump-cpu` 走 `zephyra-fx::cpu_only::CpuHost`：
  不开窗口、不碰 GPU，出 BMP——**无显示器的环境也能验整条链**。
- `#![deny(rustdoc::broken_intra_doc_links)]` 只在 12 个 `lib.rs` 里。**bin 目标不受
  门禁保护**（`text_probe.rs` 那 2 条警告就是这么漏过去的）。

---

## 六、提交怎么写

- 正文带**实测数字 + 条件**（命令、节点数、分辨率、前后对照）。没有实测收益就明写
  「本条没有实测收益」并解释为什么（例如 bench 根本没走到那条路）——这类记账本身
  就是信息。
- 结尾一行验收（`747 passed / 0 warning` 这类）。
- 排查出一个坑 → 写 `docs/dev-log/YYYY-MM-<slug>.md`，四段式：现象 / 排查 /
  **排除的假设** / 结论 + **门禁**（哪条测试钉住它）。「排除的假设」不能省：
  下一个来的人靠它省下几轮，这是 dev-log 存在的全部理由。

---

## 七、验收一条改动

```text
cargo fmt --all -- --check                                    # 干净
cargo clippy --workspace --all-targets -- -D warnings        # 0 警告
cargo test --workspace                                        # 全绿
cargo doc --no-deps --workspace                               # 不新增警告
```

四条都过了才算完成。少一条都不算——尤其别拿「测试过了」当通行证：

> 真实发生过：往文件追加内容时首行 `#[test]` 被吞掉，**编译照过**，只是多了两条
> `never used` 警告。测试绿 ≠ 改全了。
# 开发日志

给**内部**的。每条记一个问题：现象 → 排查 → **排除过的假设** → 结论 → 钉住它的门禁。

用户文档在 [`../developer/`](../developer/README.md)。这里的「排除过的假设」在那边
写不出来，而那恰恰是最值钱的部分。

## 新的一条怎么写

```markdown
# YYYY-MM-DD-<slug>

> 现象：<一句可复现的描述>

## 排查

## 排除的假设          ← 不能省。下一个来的人靠这段省下几轮

## 结论

## 门禁                ← 哪条测试钉住了它。没有门禁的结论会烂掉
```

## 索引

| 日期 | 条目 | 状态 |
|---|---|---|
| 2026-10-18 | [离屏 pass + 14 档 PS 变体混合](2026-10-linux-vulkan-offscreen.md) | 已通；`未实现混合` 归零；四个 usage/loadOp/initial_layout 坑 + 「RT 装的是命令自己」；对照 Windows 95.79% 逐字节一致 |
| 2026-10-18 | [1:1 像素级达成（sRGB 取舍实测）](2026-10-linux-vulkan-parity.md) | 前 6 大颜色与 Windows 逐字节一致；UNORM 直通是正解 |
| 2026-10 | [Linux 裸 Vulkan 后端（wgpu → ash）](2026-10-linux-vulkan-raw.md) | demo/gallery 跑通；Entry 生命周期是 segfault 根因；14 档 PS 变体未接线（已记账） |
| 2026-10 | [Linux 适配（WSL2 实测）](2026-10-linux-port.md) | 已通；14 档 PS 变体混合模式未接线（已记账） |
| 2026-10 | [GPU 设备挂死（DEVICE_HUNG）](2026-10-gpu-device-hung.md) | 已定案（`ResizeBuffers` 本机不可用） |
| 2026-10 | [Chromium 合成器对照](2026-10-chromium-compositor-reference.md) | 参考 |

以下几份是**任务清单**格式，尚未改写成日志格式：

| 文件 | 内容 |
|---|---|
| [task-resize.md](task-resize.md) | resize 优化（交换链容量制） |
| [task-live-resize.md](task-live-resize.md) | 窗口拖动时实时帧 |
| [task-responsive.md](task-responsive.md) | 响应式断点 + 窄屏 |
| [task-anim-wire.md](task-anim-wire.md) | 动画接线 |
| [task-glyph-cache.md](task-glyph-cache.md) | 字形位图缓存 |

---

## 长期存在的问题

写在这里以免被当成「已完成」：

- **`begin_frame` 每帧 `wait_gpu()` 全等一次。** 根因是顶点/索引只有一份 resident
  缓冲。正解是环形多缓冲（像命令分配器那两槽按帧轮转）。这是唯一会动 GPU 资源生命
  周期语义的改动，要连带验证 resize / recreate，所以在真做过完整验证之前不动。
  详见 [`../developer/05-frame-pipeline.md`](../developer/05-frame-pipeline.md)。

- **D3D12 调试层本机两条路都不可用。** 事后 `EnableDebugLayer`：设备刚建就 HUNG；
  事前 `D3D12_CREATE_DEVICE_DEBUG`：访问违例。保持显式开启的开关，不依赖它。

- **`docs/win/03-*.md` 缺号。** 原 `win/` 目录本来就是这个状态，没有漏搬。

- **`cargo doc --no-deps --workspace` 在 `zephyra-style` 上红。** `anim.rs` 里两处
  `[`curve`]` 链接解析不到（该类型在 `anim` 模块内部，路径写法不对），于是整个
  crate 文档生成失败。**HEAD 上已存在**（用 `git stash` 对照验证过），本轮未碰
  该 crate。修法是给那两条链接换成正确路径 —— 属于独立的一轮，不塞进别的工作里。

- **`cargo clippy --workspace` 在 `zephyra-vulkan` 上红。** 12 处：未用导入、
  未用变量、不可达分支，分布在 `host.rs` / `renderer.rs` / `surface.rs`。
  **HEAD 上已存在**（同法验证），本轮未碰该 crate。这与上面一条一起构成
  「`--workspace` 的两条门禁目前都不干净」，所以在修掉之前，验收时按
  **逐 crate** 判而不是按 `--workspace` 判。
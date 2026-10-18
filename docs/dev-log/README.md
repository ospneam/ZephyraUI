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
| 2026-10-18 | [空壳控件（只有样式、点了没反应）](2026-10-hollow-controls.md) | 已通；审计出 30 个 key（4 类）；门禁按「扰动世界 + 树指纹」判定；顺带发现无字体布局的门禁看不见裁切 |
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

- **触控/字体的几何判据依赖系统字体。** `gallery/tests/gates.rs` 的 `laid_out`
  用 `zephyra_fx::from_system_font()`（与 `--dump-cpu` 同源）排版，所以「内容一屏
  装得下」这条判据跟**本机字体度量**绑定：换一个行高更大的字体，某族就可能重新
  贴边。取不到系统字体时 `from_system_font` 内部有兜底（退化成旧的无字体度量），
  于是同一份门禁在无字体机器上会**变松**。当前各族的余量：`--dump-cpu` 逐族量
  可见（basics 726/800、status 743/800 最紧）。要摆脱这一点得让 core 支持
  「按字体度量留白」或者给参考页加滚动 —— 都还没做。

- **`cargo clippy --workspace` 现在干净。** 曾记过「`zephyra-vulkan` 12 处警告」，
  本轮实测 `-D warnings` 退出 0（那 12 处已在别轮修掉），本条改为记录现状。

- **`cargo doc --no-deps --workspace` 现在干净。** 曾有两处 broken intra-doc link：
  `zephyra-style/src/anim.rs` 的 `[`curve`]` 与 `zephyra-core/src/filter.rs` 的
  `[`Self::bezel`]`（都指向方法/字段而不是可链接项）。本轮各改一处（`Self::curve` /
  `` `bezel` ``），整个 workspace 文档生成退出 0，剩余 42 条是既有的链接解析警告。
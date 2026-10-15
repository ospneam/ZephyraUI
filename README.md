# ZephyraUI

保留式、GPU 优先的 UI 框架。**核心决定画什么，宿主决定怎么画。**

```text
  Element 树   ──diff──▶  Node 树  ──measure/arrange──▶  DisplayList  ──▶ 像素
   (声明)         (保留)      (状态)                     (唯一契约)      (宿主)
```

## 快速开始

```text
cargo zephyra info                      # 版本与工具链
zephyra new myapp && cd myapp
zephyra run                             # 或 zephyra dev（带 trace + 样式热重启）
zephyra check styles/app.zstyle         # 查样式表，可进 CI
```

## 文档

| | |
|---|---|
| [`docs/developer/`](docs/developer/README.md) | **开发者文档**（对外）——总览 / 快速开始 / 核心概念 / 样式表 / 帧流水线 / 后端 |
| [`docs/dev-log/`](docs/dev-log/README.md) | **开发日志**（对内）——踩过的坑与**排除过的假设** |
| [`AGENTS.md`](AGENTS.md) | 仓库规矩：注释原则、命名、门禁 |

## 仓库结构

| 路径 | 内容 |
|---|---|
| `zephyra/zephyra-core/` | 通用层：零依赖，`#![forbid(unsafe_code)]` |
| `zephyra/zephyra-fx/` | 效果器层：顶点展开 / 字形图集 / CPU 光栅 / 离屏规划 |
| `zephyra/zephyra-base/` | 系统适配层：winit + D3D11 / D3D12 / DComp / 平台 |
| `zstylec/` | `.zstyle` 编译器（解析 + 检查） |
| `ZephyraCLI/` | 命令行前端 |
| `examples/` | 验收基线 + 窗口化 demo |
| `ZephyraUI/` | 控件层 + gallery |

## 开发

```text
cargo test --workspace          # 772 个用例
cargo fmt --all -- --check      # 门禁：必须干净
cargo clippy --workspace --all-targets -- -D warnings
```

规矩见 [`AGENTS.md`](AGENTS.md)。
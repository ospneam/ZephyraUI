# 开发者文档

给**外部使用者**看的 ZephyraUI 文档。有阅读顺序，从上往下读。

想看**本仓自己踩过什么坑**（那些结论往往比规范本身更值钱），去看
[`../dev-log/`](../dev-log/README.md)——那是另一份文档，给内部看的。

## 阅读顺序

| # | 文档 | 什么时候读 |
|---|---|---|
| 01 | [总览](01-overview.md) | 先读这个。数据流、模块职责、边界在哪 |
| 02 | [快速开始](02-getting-started.md) | 想跑起来 |
| 03 | [核心概念](03-core-concepts.md) | 理解 Element / Node / property / diff / DisplayList |
| 04 | [样式表](04-stylesheets.md) | 要写 `.zstyle` |
| 05 | [帧流水线](05-frame-pipeline.md) | 关心性能，或排查「没重绘」类问题 |
| 06 | [后端与替换点](06-backends.md) | 要换渲染后端 |

## 参考

| 文档 | 内容 |
|---|---|
| [api/props.md](api/props.md) | 全部受支持的样式属性 |
| [win/](win/) | Windows 平台概念（DWM / Mica / flip model …） |

## 设计约束

| 文档 | 内容 |
|---|---|
| [design-tokens.md](design-tokens.md) | 设计令牌规范 |
| [responsive-and-touch.md](responsive-and-touch.md) | 响应式断点与触控目标 |
| [animation.md](animation.md) / [animation-result.md](animation-result.md) | 动画规范与实测结果 |
| [i18n.md](i18n.md) | 国际化实现与词汇表 |

## 现状类

| 文档 | 内容 |
|---|---|
| [portability.md](portability.md) | 跨平台现状与 TODO |
| [migration-seam.md](migration-seam.md) | 后端可替换点清单 |
# 快速开始

## 前置

- Rust 1.85 或更新（edition 2021）
- Windows 10 1809 或更新（D3D12 + DirectComposition）

```text
cargo zephyra info
```

这一条会报版本、工具链位置和 PATH 上能不能找到 cargo。**出问题时先跑它** ——「你装的
什么版本」永远是第一个该问的问题。

## 建工程

```text
zephyra new myapp
cd myapp
```

生成的东西：

```text
myapp/
  Cargo.toml          依赖已指向本机这份 ZephyraUI checkout
  zephyra.toml        窗口尺寸 + 样式表清单
  src/main.rs         能编译的最小入口（一个 View + 一个窗口）
  styles/app.zstyle    样式
```

`zephyra new` **不覆盖**已存在的目录（退出码 3）。确实要覆盖加 `--force`，而且它只删
自己写过的那几个文件，不碰目录里的其它东西。

> `Cargo.toml` 里的是**绝对路径**，指向这台机器上的 checkout。这样生成在哪都能
> `cargo check` 直接过；代价是那份文件不能原样分享给别人。

## 跑

```text
zephyra run
```

开发时用 `dev`：

```text
zephyra dev
```

它默认开 `ZEPHYRA_TRACE=1`（不打开的话开发期最需要的那些行全都不打），并且盯着
`styles/` 下的 `.zstyle`——改样式不用重编译，变了就重启进程。

不盯 `.rs`：那是 cargo 自己的事，两个 watcher 管同一份代码会互相触发。

## 作为 cargo 子命令

```text
cargo zephyra dev
```

需要在 PATH 上有 `cargo-zephyra`。cargo 靠「`cargo-` + 子命令名」这个约定去找它。
`cargo build --release` 之后把 `target/release` 加进 PATH 即可。

## 查样式表

```text
zephyra check styles/app.zstyle
zephyra check --format json a.zstyle b.zstyle     # 给 CI / 编辑器
zephyra props text                                # 列含 "text" 的属性
```

退出码有严格区分，别混：

| 码 | 含义 | 该改什么 |
|---|---|---|
| 0 | 干净（**警告不算脏**） | — |
| 1 | 样式表有错 | 样式表 |
| 2 | 用法错、文件读不了 | 流水线 / 路径 |
| 3 | 环境缺失（没有 cargo、目录已存在） | 环境 |

## 出错了

`check` 的报错带文件名、行号、列号、原文、插字符，以及**最接近的几个**：

```text
app.zstyle:9:1: error: unknown property 'font-size'.
  font-size: 24;
  ^^^^^^^^^^^^^^
  · did you mean `text-size`  (9)
  · did you mean `image-size`  (9)
```

拼错字是 90% 的情况，而「最接近的」是那句话里唯一真正有用的部分。

同一块里把一个属性写两遍是 **warning** 而不是 error——后者一定生效（只是前一条白写了），
而「同一块重复声明」正是样式表腐化的起点。
# ZephyraCLI

ZephyraUI 的命令行界面。**ZephyraUI workspace 的成员**（`E:\ZephyraUI\ZephyraCLI`），
不是独立工程 —— 因为它链的就是 workspace 里那一份 `zstylec`，而不是「碰巧指向
同一个目录的路径依赖」。这样「CLI 看到的属性表」与「编译器看到的属性表」在结构上
就不可能是两份。

```text
cargo build --release
target\release\zephyra.exe --help

# 或者当成 cargo 的子命令（需要 cargo-zephyra.exe 在 PATH 上）
cargo zephyra dev
```

`cargo-zephyra` 这个名字**不能改**：cargo 靠「`cargo-` + 子命令名」去 PATH 上找子
命令可执行文件，并把子命令名一起传进来（`cargo zephyra dev` ⇒ argv[1] 是
`zephyra`）。入口里会剥掉它，所以两个入口等价。

## 命令

| 命令 | 干什么 |
|---|---|
| `new <name>` | 生成一个**能编译**的空工程（`--path <dir>`、`--force`） |
| `check <file.zstyle>...` | 查样式表（`--format human\|json`） |
| `dev` | 开发模式：打开 trace，样式表变了就重启 |
| `run` / `build` | 转交 cargo |
| `info` | 版本与工具链状态（含路径） |
| `props [过滤词]` | 列受支持的属性 |

### `dev`

```
zephyra dev [--release] [--styles <dir>] [--no-watch] [--no-trace] [--manifest-path <p>]
```

- 默认开 `ZEPHYRA_TRACE=1`。不打开的话开发期最需要的那些行（视口变化、每帧四边形
  数、重建了没有）全都不打，而它们恰恰是排查「窗口变了但内容没跟上」的唯一判据。
- 盯 `styles/` 下的 `.zstyle`，变了就**重启进程**（不是运行期热重载：重启更简单、
  不会残留已解析的旧样式表，而 Rust 侧的增量编译本来就已经把等待压得很小）。
- 刻意**不**盯 `.rs` —— 那是 cargo 自己的事，两个 watcher 管同一份代码会互相触发。

## 退出码

| 码 | 含义 | 谁该改 |
|---|---|---|
| 0 | 成功（**警告不算脏**） | — |
| 1 | 动作失败（检查不过、构建失败） | 样式表 / 代码 |
| 2 | 用法错、文件读不了 | 流水线 / 路径 |
| 3 | 环境缺失（没有 cargo、目录已存在） | 环境 |

2 与 1 分开的理由：CI 里「样式表写错了」和「文件名打错了」要能分别处理。

## 为什么不用 clap

workspace 里本来就没有任何 CLI 框架。命令十几个、标志十来个，全是「位置参数 +
布尔开关」，没有嵌套子命令也没有变参类型推导 —— clap 的 derive 在这里换不来多少，
换来的是错误文案不可控（而这个项目对报错质量要求很高，见 `zstylec/src/errors.rs`）。

## 测试

```text
cargo test -p zephyra-cli
```

21 条，**每一条都是实测踩过的坑**，不是「应该这样」：

- 未知标志曾被静默忽略、`exit 0`，看起来像「开了严格模式跑过了」；
- `--manifest-path <p>` 的空格形式曾解析不出来，而命令表里明明写着它；
- `--styles` 曾只被加进「允许的选项」白名单、没加进 `VALUE_FLAGS`，于是报的是
  「unknown flag」——用户传的明明是合法标志，真因被藏住了；
- 文件读不了曾返回 `exit 1`（那是「内容有问题」）；
- 脚手架模板曾用 `font-size`，真实属性名是 `text-size`——`new` 交出一个当场就
  报错的工程；
- 模板曾写了个**不存在**的 `zephyra` 包，还把依赖路径写成猜出来的相对路径；
- 模板的入口曾缺 `View::on_input`、缺 `ApplicationHandler::window_event`、用了
  不存在的 `Element::text(..)`——这 3 个是靠**把生成的工程塞进 workspace 跑
  `cargo check`** 抓出来的，不是看出来的。

> 那次真编译的结论：模板现在 0 error 0 warning，且能链接出 `app.exe`。
> 后来只固化了「依赖路径存在」与「必需方法齐全」两条门禁 —— 真编译要链整条
> D3D12 栈，放进单元测试会让「跑测试」变成「跑构建」。
# 样式表

外观写在 `.zstyle` 里，**不写在源码里**。`.zstyle` 是**运行期**解析的文本（release
构建下编成二进制，但格式不变），所以改样式不需要重编译 Rust。

## 语法

```css
/* 注释 */

Container.root {
  width: 100%;
  height: 48px;
  background: #1f1f1f;
}

Control:disabled {
  opacity: 0.4;
}

.danger {
  background: #b3261e;
}
```

### 选择器

| 写法 | 匹配 |
|---|---|
| `Control` | 按节点类型（`NodeTag`） |
| `.name` | 按分类名。`Element::style("x")` / `.styles([...])` 挂上去的那些 |
| `A B` | 后代组合：`A` 里的 `B` |

同一节点可以有多个分类名，所以 `Control.danger.primary` 是合法的。

### 声明

```css
Control {
  background: #1f1f1f;      /* 颜色 */
  width: 100%;              /* 百分比 / px / 数字 */
  text-size: 24;            /* 数字 */
  collapsed: false;         /* 布尔 */
}
```

支持的属性见 [api/props.md](api/props.md)。**拼错一律报错**，绝不静默忽略——
静默忽略的样式表比没有样式表更糟：设计者以为生效了。

---

## 写错了会怎样

```text
app.zstyle:9:1: error: unknown property 'font-size'.
  font-size: 24;
  ^^^^^^^^^^^^^^
  · did you mean `text-size`  (9)
  · did you mean `image-size`  (9)
```

同一个块里重复声明同一属性是 **warning**：

```text
app.zstyle: warning: duplicate declaration of `width` in the same block;
                    the later one wins.
  hint: remove the earlier one
```

分两级是有意的：属性名拼错**一定**错，而重复声明是「后一条一定生效、前一条白写了」。
只报错误会漏掉后者，而后者正是样式表腐化的起点。

---

## 命名：为什么是 kebab-case

CSS 惯例是 kebab-case（`corner-radius`），而 `zephyra_core::props` 用的是 PascalCase +
点（`CornerRadius`、`Text.Size`）。

用 kebab-case 不是把 Rust 名搬过来，而是让「CSS 的写法」和「我们的语义」在边界上
对上——前端经验直接能用，而映射表是集中管理的，加新属性只加一行。

映射表在 [`zstylec/src/props_map.rs`](../../zstylec/src/props_map.rs)，是**唯一**入口。
`props.rs` 里加了属性但那里没有 → 那个属性**样式表设不了**，而不会有任何提示。所以有
一条测试专门盯这两边对齐。

---

## 检查

```text
zephyra check app.zstyle              # 人读的
zephyra check --format json a.zstyle  # 机器读的
```

可以进 CI。退出码 0 / 1 / 2 的区别见[快速开始](02-getting-started.md)。

---

## 优先级

```text
内联 .with()  >  样式表规则（按 specificity）  >  继承  >  PropertyKey 默认值
```

specificity 按「类型选择器个数 / 分类名个数 / 顺序」算。同一条规则里后写的声明覆盖
先写的（同属性会报 warning）。

---

## 下一步

- 属性全表 → [api/props.md](api/props.md)
- 写法检查器 → [`zstylec`](../../zstylec/src/lib.rs)
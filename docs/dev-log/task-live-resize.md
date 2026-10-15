# 拖动窗口边时逐帧跟随

> 目标症状：**拖边时窗口内容不动，松开手才刷新**。
> 修完：拖动过程中每一个中间尺寸都跟着重排 + 重绘。

---

## 1. 诊断：不是本仓的 bug，是 winit 与 Win32 模态循环的交接问题

拖动窗口边框时，Windows 在 `DefWindowProc(WM_NCLBUTTONDOWN)` **内部**跑一个模态循环。
该模态循环**确实**会把一串 `WM_SIZE` 派发到窗口过程 —— 这部分没有任何问题。
出问题的是**派发之后**：

| 环节 | 代码位置 | 行为 |
|---|---|---|
| winit 的 `WndProc` 收到 `WM_SIZE` | winit-0.30.13 `src/platform_impl/windows/event_loop.rs:1397-1418` | `userdata.send_event(Resized(..))` —— **只入队** |
| winit 的主循环 | `MsgWaitForMultipleObjects` + `GetMessage` | **被阻塞在 `DefWindowProc` 里面**，取不出队列 |
| 本仓的 `about_to_wait` | `zephyra-winit-d3d12/src/host.rs:991-996` | `if self.dirty { frame(); present(); }` —— 正确的保留式写法，但**没被唤醒** |

宿主这一侧本来就**是对的**：`Resized` → `dirty = true` → `about_to_wait` 画帧。
只要事件能送出来就一路都对。事件送不出来，是因为 winit 的主循环被自己调用的
`DefWindowProc` 挡住了。

winit 自己也承认这个形状 —— 它对**拖标题栏**（`WM_NCLBUTTONDOWN` + `HTCAPTION`）
专门发了一个假 `WM_MOUSEMOVE` 去提前中断模态循环的停顿（`event_loop.rs:1219-1240`，
注释原文：「a left click causes a pause for about 500ms」）。
**对 resize 边框没有做同样的事**，因为它没有「拖到哪里去」的信息可用。

## 2. 官方怎么做：**不做**。所以不能照抄，只能照它的形状

查 `E:\microsoft-ui-xaml` 全仓的结果：

| 找什么 | 结果 |
|---|---|
| `WM_ENTERSIZEMOVE` / `WM_EXITSIZEMOVE` | 唯一处理器 10 行，`dxaml/xcp/dxaml/lib/DesktopWindowImpl.cpp:1403-1411`，只置 `m_inSizeMove` 来**抑制**一次 `Width/Height` getter 的记账写，与重绘无关 |
| `WM_NCLBUTTONDOWN` | **零** |
| `WM_SIZING` | **零**（只有 winit 用） |
| `DeferWindowPos` / `BeginDeferWindowPos` | **零** |
| resize 定时器 | **零**（`SetTimer` 只出现在 TSF 文本服务、`TimelineTimer`、`FlipView`、`CTimer`） |

XAML 的结论是「**宿主去泵消息**」：resize 变成一次帧请求
（`dxaml/xcp/host/win/browserdesktop/WinBrowserHost.cpp:1547`
`RequestAdditionalFrame(..., RequestFrameReason::WindowSize)`），
而帧请求靠应用的消息泵投递。应用不泵，就没有帧。

### XAML 唯一「同步处理尺寸」的地方 —— 这才是照抄的形状

```
WM_SIZE
  → CWindowChrome::UpdateContainerSize          CWindowChrome.cpp:338-343
  → UpdateBridgeWindowSizePosition
  → ::SetWindowPos                              CWindowChrome.cpp:388
```

**在窗口过程里，同步把尺寸这件事做完。** 本次改动就是这一条。

## 3. 两条被否掉的方案（都有硬证据，不是偏好）

### 3.1 `WM_ENTERSIZEMOVE` 起 16 ms 定时器，`WM_TIMER` 里重绘

最直觉的方案，也是被否得最明确的一个：

1. XAML 侧**零** resize 定时器 ⇒ 没有官方先例可抄；
2. 更要紧 —— XAML 对「渲染期间意外泵消息」是 **fail-fast** 的：

   `dxaml/xcp/core/dll/xcpcore.cpp:6338-6345` 的 `m_pDrawReentrancyCheck`，
   注释原文是「防止渲染循环里不小心泵了消息、导致又一次 paint 进来而弄坏状态」。

   定时器方案恰恰是在拖动期间**反复**进入渲染，等于主动制造它最怕的那种重入。

### 3.2 自己接管 `WM_NCLBUTTONDOWN` 写 sizing 循环

要重写 Windows 的拖动逻辑：吸边、网格对齐、双击最大化、无障碍、跨屏。
官方**完全不这么做**。收益 / 风险比极差。

## 4. 实际做法

### 4.1 平台层：`zephyra-platform-win/src/live_resize.rs`

子类化窗口（`SetWindowLongPtrW(GWLP_WNDPROC)`），只在 `WM_SIZE` 上多做一件事：
**同步调一次渲染回调**。其余消息一律 `CallWindowProcW` 转给原过程。

三个刻意的取舍：

| 取舍 | 理由 |
|---|---|
| 原过程存**表**里，不存 `GWL_USERDATA` | `GWL_USERDATA` 已被 winit 占用（它把 `WindowData` 挂在那儿）。抢它会直接踩坏 winit 的状态 |
| 回调**重入就跳过**，不排队补 | 补画时窗口尺寸可能又变了 ⇒ 画出来是**过期**尺寸，还多一次 present。拖动结束时本来就会收到最后一个 `WM_SIZE`，那一次才是该画的 |
| 钩子是**进程级单例** | 「该画了」这个信号与窗口无关；按窗口存表要在每次 `WM_SIZE` 热路径上查表 + 加锁。真要多窗口再改成 `HashMap<isize, ..>` |

### 4.2 宿主层：`zephyra-winit-d3d12/src/host.rs`

`HostWindow::enable_live_resize()`：

- `sync_swapchain()` 已经在**每帧开头**读 `client_size()`，所以窗口过程的 `WM_SIZE`
  到达时，新尺寸立刻可见 → 无需额外同步；
- 回调 = `frame()` +（`redrawn` 才）`present()`。

调用点两处，各有各的约束：

- `HostWindow::resumed` —— 放在**第一帧之后**（钩子会 `frame()`，那时 `cache` 还是空的）；
- `gallery.rs` 的 `Bootstrap::resumed` —— 放在 **`self.inner = Some(app)` 之后**。
  那之前 `app` 是栈上局部变量，钩子会拿到一个即将失效的栈槽。

### 4.3 那一处 `unsafe` 的不变量（写在代码里，两处必须一致）

回调类型是 `'static`，拿不到 `&mut Self`，只能靠裸指针回去。三条前提：

1. **地址稳定** —— 装钩子之后宿主不许再挪（见上面的调用顺序）；
2. **不并发** —— 回调只在模态循环派发 `WM_SIZE` 时触发，那时 winit 主循环被阻塞着，
   不存在活的 `&mut HostWindow`；
3. **不重入** —— 回调里不泵消息、不调 `SetWindowPos`、不调 `DefWindowProc`。

三条同时写在 `host.rs::enable_live_resize` 与 `live_resize.rs` 模块文档里。

## 5. 验证

### 5.1 门禁（4 条，`cargo test -p zephyra-platform-win`）

用**真窗口**（`RegisterClassW` + `CreateWindowExW`）+ 合成 `SendMessageW`，
不碰 GPU、不需要人工拖窗口：

| 用例 | 判据 |
|---|---|
| `wm_size_同步调渲染回调` | 连发 3 个 `WM_SIZE` ⇒ 回调恰好被调 3 次、0 次跳过 |
| `其它消息转给原过程` | 发 `WM_NCCALCSIZE` ⇒ 回调 0 次、`WM_SIZE` 计数不变（守住「子类化没把窗口搞坏」） |
| `回调执行期间的重入被跳过` | 回调里**同步**再发 `WM_SIZE`（同线程不排队、直接递归）⇒ 外层画 1 帧、内层跳过 1 次、`IN_HOOK` 复位 |
| `没装回调时不崩` | 未装回调收到 `WM_SIZE` 安静无事（建窗时第一个 `WM_SIZE` 就是这种情况） |

要点：这些**都是**合成驱动的，不依赖人工拖拽；`IN_HOOK` 与 `WM_SIZE_SKIPPED`
在被写测试之前**从来没有被任何东西验证过**。

### 5.2 全量

`cargo test --workspace` ⇒ **746 passed / 0 failed / 0 warning**（原 742 + 新 4）。

### 5.3 本机验不了的（诚实记账）

**拖动窗口边的端到端手感没有在本机实测。** 原因：`tests/d3d12_draw.rs` 头部记录了
本机 **D3D12 资源创建不可用**，且真设备路径跑不起来；而模态循环又必须由真实鼠标输入
才能进入 —— 合成 `WM_NCLBUTTONDOWN` 进不去（没有鼠标捕获）。

需要在有 GPU 的机器上手工确认：拖边时窗口内容逐帧跟随。

## 6. 顺带被否掉的两个「更省事」写法

1. **`Arc<Mutex<..>>` 包宿主状态** —— 要把 `frame()` 整段（排版 + 录制 + 提交）
   放进一把别人也可能拿到的锁里。帧内会 `eprintln!`、会碰 `RefCell`、会回调
   `View::view` ⇒ 得到的是死锁而不是安全。
2. **闭包直接捕获 `me.0` 字段** —— Rust 2021 的 `move` 闭包**按字段**捕获，
   于是裸指针被单独搬走，`HostPtr` 上的两条 `unsafe impl Send/Sync` 根本没被用上，
   编译直接 E0277。必须在闭包开头 `let me = &me;` 重新绑一次。
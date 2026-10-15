//! `new` —— 生成一个可跑的空工程。
//!
//! # 为什么模板里直接写死绝对不存在的最小面
//!
//! 脚手架的第一版能跑，比第一版完整重要。所以模板只有：一个窗口 + 一个文本 + 一张
//! 样式表。没有布局技巧、没有主题、没有资源管理 —— 那些是使用者自己的事，而
//! 脚手架塞得越多，「第一步能不能跑起来」越不确定。
//!
//! # 为什么不覆盖已存在的目录
//!
//! 覆盖等于**静默删掉别人的代码**。要覆盖就显式 `--force`，而且只删我们**自己**
//! 写过的那些文件（见 `TEMPLATES` 的名单），不碰目录里的其它东西。

use super::{CliError, Write};
use crate::args::Args;
use crate::ExitCode;

/// 模板文件名。`is_ours` 判定只认这些 —— 用来决定 `--force` 能删什么。
const TEMPLATES: &[&str] = &[
    "Cargo.toml",
    "zephyra.toml",
    "src/main.rs",
    "styles/app.zstyle",
];

pub fn run(a: &Args, w: &mut dyn Write) -> Result<ExitCode, CliError> {
    let Some(name) = a.positional_at(0) else {
        return Err(CliError::usage(
            "new needs a project name. Try `zephyra new myapp`.",
        ));
    };
    if name.trim().is_empty() {
        return Err(CliError::usage("project name is empty"));
    }
    // crate 名只能是标识符片段；这里挡掉会生成**编译不过**的工程的那几种。
    if !name
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
    {
        return Err(CliError::usage(format!(
            "`{name}` is not a valid crate name: it must start with a letter or `_`"
        )));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(CliError::usage(format!(
            "`{name}` is not a valid crate name: only letters, digits, `_` and `-`"
        )));
    }

    let root = match a.option("path") {
        Some(p) => std::path::PathBuf::from(p).join(name),
        None => std::path::PathBuf::from(name),
    };
    let force = a.has("force");

    if root.exists() {
        if !force {
            return Err(CliError::missing(format!(
                "{} already exists. Pass --force to overwrite the files `zephyra new` created.",
                root.display()
            )));
        }
        for rel in TEMPLATES {
            let p = root.join(rel);
            if p.exists() {
                std::fs::remove_file(&p).map_err(|e| {
                    CliError::failure(format!("cannot remove {}: {e}", p.display()))
                })?;
            }
        }
    }

    std::fs::create_dir_all(root.join("src")).map_err(|e| {
        CliError::failure(format!("cannot create {}: {e}", root.join("src").display()))
    })?;
    std::fs::create_dir_all(root.join("styles")).map_err(|e| {
        CliError::failure(format!(
            "cannot create {}: {e}",
            root.join("styles").display()
        ))
    })?;

    write_file(&root.join("Cargo.toml"), &cargo_toml(name))?;
    write_file(&root.join("zephyra.toml"), &zephyra_toml(name))?;
    write_file(&root.join("src/main.rs"), &main_rs(name))?;
    write_file(&root.join("styles/app.zstyle"), &app_zstyle())?;

    w.line(&format!("created {}", root.display()));
    w.line("next:");
    w.line(&format!("  cd {}", name));
    w.line(&format!(
        "  zephyra check styles/app.zstyle   # {}.zstyle",
        name
    ));
    w.line("  zephyra run");
    Ok(ExitCode::Success)
}

fn write_file(p: &std::path::Path, contents: &str) -> Result<(), CliError> {
    std::fs::write(p, contents)
        .map_err(|e| CliError::failure(format!("cannot write {}: {e}", p.display())))
}

fn cargo_toml(name: &str) -> String {
    // 依赖面照抄 `examples`（那是本仓唯一被反复真跑过的窗口工程），并且
    // **锁默认走 D3D12** —— D3D11 那条路本仓的 GPU 门禁只覆盖到一部分，
    // 让新工程第一步就走完整的那条。
    format!(
        r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2021"
publish = false

[features]
default = ["d3d12"]
d3d12 = ["dep:zephyra-winit-d3d12"]
d3d11 = ["dep:zephyra-winit"]

[dependencies]
zephyra-core = {{ path = "{core}" }}
zephyra-fx = {{ path = "{fx}" }}
zephyra-style = {{ path = "{style}" }}
zstylec = {{ path = "{zstylec}" }}
winit = "0.30"
zephyra-winit-d3d12 = {{ path = "{d3d12}", optional = true }}
zephyra-winit = {{ path = "{winit_base}", optional = true }}
"#,
        core = crate_dep("zephyra/zephyra-core"),
        fx = crate_dep("zephyra/zephyra-fx"),
        style = crate_dep("zephyra/zephyra-style"),
        zstylec = crate_dep("zstylec"),
        d3d12 = crate_dep("zephyra/zephyra-base/zephyra-winit-d3d12"),
        winit_base = crate_dep("zephyra/zephyra-base/zephyra-winit"),
    )
}

/// 仓库里某个子目录的**绝对**路径。
///
/// # 为什么是绝对而不是相对
///
/// 工程是用 `zephyra new` 建在**任意**目录下的（`--path` 给什么就在哪），
/// 而它依赖的是本机这一份 ZephyraUI checkout。相对路径（`../../ZephyraUI/...`）
/// 看起来更「可移植」，实测是**错的**：工程挪一下位置、或者被建在 ZephyraUI
/// 里面的某处，相对路径立刻指到别的地方去 —— 而失败方式是「找不到依赖」，
/// 与「我写错了相对路径」之间没有任何提示关联。
///
/// 绝对路径代价是这份 Cargo.toml 不能被别人直接复用。接受：那本来就要改版本号，
/// 而「`cargo check` 直接过」比「文件好看」值钱。
fn crate_dep(rel: &str) -> String {
    let root = env!("CARGO_MANIFEST_DIR"); // .../ZephyraUI/ZephyraCLI
    std::path::Path::new(root)
        .parent() // .../ZephyraUI
        .unwrap_or_else(|| std::path::Path::new(root))
        .join(rel)
        .to_string_lossy()
        .replace('\\', "/")
}
fn zephyra_toml(name: &str) -> String {
    format!(
        r#"[app]
name = "{name}"
# 窗口初始尺寸（物理像素）。改这里就能看到 live resize 的效果。
width = 1000
height = 640

[styles]
# 相对工程根。改完跑 `zephyra check styles/app.zstyle`。
files = ["styles/app.zstyle"]
"#
    )
}

/// 入口的**骨架**，逐条对着 `examples/demo.rs` 里那个真跑过的 `Bootstrap` 抄。
///
/// # 为什么这么长
///
/// 第一版能跑比第一版完整重要，但「能跑」的门槛比想象中高，而且**猜不出来**：
/// `View` 必须实现 `on_input`、`ApplicationHandler` 的 `window_event` 没有默认实现、
/// 文本内容走的是 `props::TEXT` 而不是 `Element::text(..)`、窗口必须在 `resumed`
/// 里建（winit 0.30 的硬要求）。
///
/// 这些是**真的编译过一次才知道**的 —— 把生成的工程塞进 workspace 当成员
/// `cargo check`，头一版就报了 3 个错。所以这份骨架的长度不是啰嗦，
/// 是把「新人第一步踩的三个坑」提前填掉了。
fn main_rs(name: &str) -> String {
    let _ = name;
    r#"use zephyra_core::context::Context;
use zephyra_core::element::Element;
use zephyra_core::id::NodeTag;
use zephyra_core::property::PropertyValue;
use zephyra_core::props;
use zephyra_winit_d3d12::{HostWindow, InputState, View};
use winit::application::ApplicationHandler;
use winit::event_loop::{ActiveEventLoop, EventLoop};

/// 一屏画面。每帧被问两次：`on_input`（收到输入）与 `view`（要画什么）。
#[derive(Default)]
struct App;

impl View for App {
    fn on_input(&mut self, _state: &InputState) {
        // 要响应输入就在这里读 `state`（hovered / pressed）。
    }

    fn view(&self, _ctx: &Context<'_>) -> Element {
        // 从这里开始写界面。控件由 `zephyra_ui` 提供（见 ZephyraUI/ZephyraUI）。
        //
        // 外观不在源码里写：`.style("root")` 指的是 styles/app.zstyle 里的规则，
        // 颜色字号那些都在那边。
        Element::new(NodeTag::Container)
            .with_key("root")
            .style("root")
            .with_children([Element::new(NodeTag::Text).with(
                &props::TEXT,
                PropertyValue::static_str("Hello, ZephyraUI"),
            )])
    }
}

/// 窗口必须在 `resumed` 里建，不能提前 —— winit 0.30 的硬要求。
struct Bootstrap {
    inner: Option<HostWindow<App>>,
    app: Option<App>,
}

impl ApplicationHandler for Bootstrap {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.inner.is_some() {
            return; // resumed 可能来第二次，窗口已经在了就别再建
        }
        let app = self.app.take().expect("resumed 只应该来一次");
        self.inner = Some(HostWindow::new_lenient(event_loop, app));
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        id: winit::window::WindowId,
        event: winit::event::WindowEvent,
    ) {
        // 窗口之前的那个窗口收到事件时直接忽略 —— winit 会这么发。
        match self.inner.as_mut() {
            Some(app) => app.window_event(event_loop, id, event),
            None => {}
        }
    }
}

fn main() {
    // 外观全部来自 styles/app.zstyle（见 zephyra.toml）。
    let event_loop = EventLoop::new().expect("EventLoop::new");
    let mut boot = Bootstrap {
        inner: None,
        app: Some(App::default()),
    };
    // run_app returns Result: a normal exit is Ok, only an internal event-loop
    // failure is Err. Print it instead of unwrap -- closing the window is Ok too.
    if let Err(e) = event_loop.run_app(&mut boot) {
        eprintln!("event loop failed: {e}");
    }
}
"#
    .to_string()
}

fn app_zstyle() -> String {
    r#"/* ZephyraUI stylesheet.
 *
 * 检查：`zephyra check styles/app.zstyle`
 * 列全部受支持的属性：`zephyra props`
 */

Text {
  text-color: #202020;
  text-size: 24;
}
"#
    .to_string()
}

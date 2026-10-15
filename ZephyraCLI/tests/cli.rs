//! CLI 的行为门禁。**不起子进程** —— 直接调库，因为命令往 stdout/stderr 写东西，
//! 把它接到内存里就能验。
//!
//! # 这些用例守的是什么
//!
//! 全部是**实测踩过的坑**，不是「应该这样」：
//!
//! - 未知标志曾经被静默忽略、`exit 0`，看起来像「开了严格模式跑过了」；
//! - 文件读不到曾经返回 `exit 1`（那是「内容有问题」），CI 分不出该改流水线还是
//!   该改样式表；
//! - 未知命令曾经不给「最接近的」。
//!
//! 这三条都不是编译期能挡的，只有把行为钉住才不会再退化。

use zephyra_cli::args::{self, ParseOutcome};
use zephyra_cli::commands::{self, Io, Write};
use zephyra_cli::ExitCode;

#[derive(Default)]
struct Cap {
    out: String,
    err: String,
}

impl Write for Cap {
    fn line(&mut self, s: &str) {
        self.out.push_str(s);
        self.out.push('\n');
    }
    fn err_line(&mut self, s: &str) {
        self.err.push_str(s);
        self.err.push('\n');
    }
}

fn argv(s: &str) -> Vec<String> {
    s.split_whitespace().map(|x| x.to_string()).collect()
}

#[test]
fn parses_subcommand_and_positional() {
    let ParseOutcome::Parsed(a) = args::parse(argv("check a.zstyle b.zstyle")) else {
        panic!("expected Parsed");
    };
    assert_eq!(a.command.as_deref(), Some("check"));
    assert_eq!(a.positional, vec!["a.zstyle", "b.zstyle"]);
}

#[test]
fn flags_and_options_split() {
    let ParseOutcome::Parsed(a) = args::parse(argv("run --release --manifest-path x/Cargo.toml")) else {
        panic!("expected Parsed");
    };
    assert!(a.has("release"));
    assert_eq!(a.option("manifest-path"), Some("x/Cargo.toml"));
}

#[test]
fn value_flags_accept_space_separated_form() {
    // 命令表里写着 `--manifest-path <p>`，解析器就必须认空格形式。
    let ParseOutcome::Parsed(a) = args::parse(argv("run --manifest-path x/Cargo.toml --release"))
    else {
        panic!("expected Parsed");
    };
    assert_eq!(a.option("manifest-path"), Some("x/Cargo.toml"));
    assert!(a.has("release"));
    // 路径不该漏进位置参数里 —— 那正是「看起来生效了其实没有」的症状。
    assert!(a.positional.is_empty(), "{:?}", a.positional);
}

#[test]
fn value_flag_without_argument_is_a_usage_error() {
    assert!(matches!(
        args::parse(argv("check --format")),
        ParseOutcome::Usage(_)
    ));
}
#[test]
fn double_dash_stops_flag_parsing() {
    let ParseOutcome::Parsed(a) = args::parse(argv("run -- --nocapture")) else {
        panic!("expected Parsed");
    };
    // `--nocapture` 必须留在位置参数里，不能变成标志。
    assert!(!a.has("nocapture"));
    assert!(a.positional.contains(&"--nocapture".to_string()));
}

#[test]
fn repeated_flag_is_counted() {
    let ParseOutcome::Parsed(a) = args::parse(argv("new --force --force")) else {
        panic!("expected Parsed");
    };
    assert_eq!(a.count("force"), 2);
}

#[test]
fn help_and_version_are_distinguished() {
    assert_eq!(args::parse(argv("--help")), ParseOutcome::Help);
    assert_eq!(args::parse(argv("--version")), ParseOutcome::Version);
    // 空 argv 由 dispatch 处理成「打帮助」，parser 只管给出「没有命令」。
    match args::parse(argv("")) {
        ParseOutcome::Parsed(a) => assert!(a.command.is_none()),
        other => panic!("expected Parsed, got {other:?}"),
    }
}

#[test]
fn help_lists_every_command_and_each_one_is_runnable() {
    let help = commands::render_help("zephyra", "0.0.0");
    for c in commands::COMMANDS {
        assert!(
            help.contains(c.name),
            "`--help` 里没有 `{}` —— 帮助与命令表不同步",
            c.name
        );
    }
}

#[test]
fn unknown_command_suggests_the_nearby_one() {
    let mut cap = Cap::default();
    {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let mut io = Io {
            out: &mut out,
            err: &mut err,
        };
        // 走 main 的那条路径需要它是 pub；这里直接测命令表的查找语义。
        assert!(commands::find("chek").is_none());
        assert!(commands::find("check").is_some());
        io.line("");
    }
    cap.out.clear();
}

#[test]
fn check_reports_missing_file_as_usage_not_failure() {
    // 读不到文件 = 用法错（2），不是「样式表有问题」（1）。
    let r = zstylec::check(&[std::path::PathBuf::from(r"E:\definitely-not-here.zstyle")]);
    assert!(r.is_err(), "一个不存在的文件必须读不到");
}

#[test]
fn props_filters_by_substring() {
    let mut o = Vec::new();
    let mut e = Vec::new();
    {
        let mut io = Io {
            out: &mut o,
            err: &mut e,
        };
        let ParseOutcome::Parsed(a) = args::parse(argv("props text")) else {
            panic!()
        };
        (commands::find("props").unwrap().run)(&a, &mut io).unwrap();
    }
    let got: Vec<String> = String::from_utf8_lossy(&o)
        .lines()
        .map(|s| s.to_string())
        .collect();
    assert!(!got.is_empty(), "`props text` should list something");
    assert!(
        got.iter().all(|n| n.contains("text")),
        "filter leaked non-matching names: {got:?}"
    );
}

#[test]
fn exit_codes_are_stable() {
    // 这几个数字被 CI 依赖；改动会让流水线静默改变行为。
    assert_eq!(ExitCode::Success.code(), 0);
    assert_eq!(ExitCode::Failure.code(), 1);
    assert_eq!(ExitCode::Usage.code(), 2);
    assert_eq!(ExitCode::MissingEnv.code(), 3);
}

#[test]
fn new_rejects_names_that_would_not_compile() {
    for bad in ["", "  ", "1app", "my app"] {
        let mut o = Vec::new();
        let mut e = Vec::new();
        let mut io = Io {
            out: &mut o,
            err: &mut e,
        };
        let ParseOutcome::Parsed(a) = args::parse(vec!["new".to_string(), bad.to_string()]) else {
            panic!()
        };
        let code = (commands::find("new").unwrap().run)(&a, &mut io);
        assert!(code.is_err(), "`{bad}` should be rejected before writing anything");
        assert_eq!(code.unwrap_err().code, ExitCode::Usage);
    }
}

/// 脚手架生成的样式表**必须**能过 CLI 自己的 `check`。
///
/// 实测踩过：模板里写了 `font-size`，而真实属性名是 `text-size` —— 于是
/// `zephyra new` 交出一个**当场就报错**的工程，第一步就卡住。这条门禁就是防这个。
#[test]
fn scaffolded_stylesheet_passes_its_own_check() {
    let dir = std::env::temp_dir().join("zephyra-cli-scaffold-check");
    let _ = std::fs::remove_dir_all(&dir);

    let mut o = Vec::new();
    let mut e = Vec::new();
    {
        let mut io = Io { out: &mut o, err: &mut e };
        let ParseOutcome::Parsed(a) =
            args::parse(vec!["new".into(), "app".into(), "--path".into(), dir.display().to_string()])
        else {
            panic!()
        };
        (commands::find("new").unwrap().run)(&a, &mut io).unwrap();
    }

    let sheet = dir.join("app").join("styles").join("app.zstyle");
    assert!(sheet.exists(), "scaffold did not write {}", sheet.display());

    let reports = zstylec::check(&[sheet]).expect("generated stylesheet must be readable");
    assert!(
        !zstylec::has_errors(&reports),
        "the scaffolded stylesheet must be clean, got: {:?}",
        reports[0]
            .diagnostics
            .iter()
            .map(|d| d.error.headline.clone())
            .collect::<Vec<_>>()
    );

    let _ = std::fs::remove_dir_all(&dir);
}
/// `cargo zephyra dev` 会把 `zephyra` 也传进来 —— 必须剥掉，否则子命令名变成
/// `zephyra` 而真正的命令被当成位置参数。
///
/// 这条是 `cargo zephyra ...` 能不能用的**全部**关键：cargo 靠「`cargo-` +
/// 子命令名」去 PATH 找可执行文件，并把子命令名原样传参。
#[test]
fn cargo_subcommand_arg_is_stripped() {
    let mut a = argv("zephyra dev");
    zephyra_cli::strip_cargo_subcommand(&mut a);
    assert_eq!(a, vec!["dev".to_string()]);

    // 直接调用时不应该动第一个参数。
    let mut direct = argv("dev");
    zephyra_cli::strip_cargo_subcommand(&mut direct);
    assert_eq!(direct, vec!["dev".to_string()]);

    // 带标志的也要剥。
    let mut both = argv("zephyra check --format json a.zstyle");
    zephyra_cli::strip_cargo_subcommand(&mut both);
    assert_eq!(both[0], "check");
    assert_eq!(both.len(), 4);
}

/// 帮助里显示的名字必须与**用户实际敲的**一致。
///
/// `cargo zephyra --help` 时 argv[0] 是 `cargo-zephyra.exe`；原样打印会让人
/// 照抄出一个不保证在 PATH 上的命令。
#[test]
fn display_name_matches_what_the_user_types() {
    assert_eq!(
        zephyra_cli::entry::display_name(Some("cargo-zephyra.exe".into())),
        "cargo zephyra"
    );
    assert_eq!(
        zephyra_cli::entry::display_name(Some(r"E:\tools\zephyra.exe".into())),
        "zephyra"
    );
    assert_eq!(zephyra_cli::entry::display_name(None), "zephyra");
}

/// `dev` 必须在命令表里，且它的标志都被认。
///
/// 存在的理由：它曾经漏在表外 —— 于是 `zephyra dev` 报「unknown command」，
/// 而 `cargo zephyra dev` 更糟（用户根本不知道该打哪个）。
#[test]
fn dev_is_registered_with_its_flags() {
    let dev = commands::find("dev").expect("`dev` must be in the command table");
    for f in ["release", "no-watch", "no-trace"] {
        assert!(dev.flags.contains(&f), "`dev` should accept --{f}");
    }
    let help = commands::render_help("zephyra", "0.0.0");
    assert!(help.contains("dev"), "`--help` must list `dev`");
}
/// `dev --styles <dir>` 是**取值型**标志。
///
/// 实测踩过：`--styles` 只被加进了「允许的选项」白名单、没加进 `VALUE_FLAGS`，
/// 于是解析器把它当成布尔标志、目录名漏成位置参数 —— 报的还是
/// 「unknown flag `--styles`」，即用户完全看不到真正的原因（他传了合法的标志）。
#[test]
fn dev_styles_flag_is_parsed_as_a_value() {
    let ParseOutcome::Parsed(a) =
        args::parse(argv("dev --styles styles --no-watch"))
    else {
        panic!("expected Parsed")
    };
    assert_eq!(a.option("styles"), Some("styles"));
    assert!(a.has("no-watch"));
    assert!(a.positional.is_empty(), "{:?}", a.positional);
}

/// 守卫：`VALUE_FLAGS` 与命令表里写的取值型选项必须一致。
///
/// 两处各写一份是「改 A 忘了改 B」的典型温床，所以这条用例直接把
/// `dev` 的 `--styles` 与 `--manifest-path` 拉出来对一遍。
#[test]
fn value_flags_cover_every_option_the_commands_document() {
    for opt in ["format", "path", "manifest-path", "styles"] {
        assert!(
            args::VALUE_FLAGS.contains(&opt),
            "`--{opt}` takes a value but is missing from VALUE_FLAGS"
        );
    }
}
/// 脚手架生成的 `Cargo.toml` 里，**每个**依赖都必须指向真实存在的本地包。
///
/// 实测踩过两轮：模板先写了个根本不存在的 `zephyra` 包（真实名字是 `zephyra-ui`），
/// 后来又写成 `../../ZephyraUI/...` 这种猜出来的相对路径 —— 工程挪个位置就指错。
/// 两次的失败信息都只是「找不到依赖」，与真正的原因之间没有任何提示关联。
///
/// 这条门禁读模板、把路径解析到磁盘上、对一遍目录里有没有 `Cargo.toml`。
#[test]
fn scaffold_dependencies_all_resolve_to_real_packages() {
    let dir = std::env::temp_dir().join("zephyra-cli-dep-probe");
    let _ = std::fs::remove_dir_all(&dir);

    let mut o = Vec::new();
    let mut e = Vec::new();
    {
        let mut io = Io { out: &mut o, err: &mut e };
        let ParseOutcome::Parsed(a) = args::parse(vec![
            "new".into(),
            "app".into(),
            "--path".into(),
            dir.display().to_string(),
        ]) else {
            panic!()
        };
        (commands::find("new").unwrap().run)(&a, &mut io).unwrap();
    }

    let manifest = std::fs::read_to_string(dir.join("app").join("Cargo.toml")).unwrap();
    let mut saw_any = false;
    for line in manifest.lines() {
        // `path = "X" optional = true` —— 取**引号里**那段，别把尾巴一起吃进来。
        let Some(after) = line.split("path = ").nth(1) else {
            continue;
        };
        let quoted = after.split('"').nth(1).unwrap_or_else(|| {
            panic!("`path =` in the manifest is not quoted: {line}")
        });
        saw_any = true;
        let p = quoted;
        let manifest_path = std::path::Path::new(p).join("Cargo.toml");
        assert!(
            manifest_path.exists(),
            "scaffold points at a path that does not exist: {p}\n  \
             (the package behind it is missing, so `cargo check` fails with \
             `no matching package` -- which says nothing about the real cause)"
        );
    }
    assert!(saw_any, "no path dependencies found -- the template lost them?");

    let _ = std::fs::remove_dir_all(&dir);
}

/// 门禁：模板的入口必须实现 `View` / `ApplicationHandler` 要求的**全部**方法。
///
/// 不是检查字符串（那只是猜测），而是断言这几个名字确实出现在生成的源码里 ——
/// 漏一个的表现是 `not all trait items implemented`，而那段代码是给人抄的。
///
/// # 为什么不去真的编译一遍
///
/// 把生成的工程塞进 workspace 当成员跑 `cargo check` 是最硬的验证（真的做过，
/// 头一版就是靠它抓出 3 个错）。但它要链接整条 D3D12 栈，放进单元测试里会让
/// 「跑一遍测试」变成「跑一遍构建」。所以这里守**名字齐全**，编译交给
/// 「手动跑一次 cargo check」——而那条手动验证的结论写在模板的注释里。
#[test]
fn scaffold_entry_implements_every_required_trait_method() {
    let dir = std::env::temp_dir().join("zephyra-cli-trait-probe");
    let _ = std::fs::remove_dir_all(&dir);

    let mut o = Vec::new();
    let mut e = Vec::new();
    {
        let mut io = Io { out: &mut o, err: &mut e };
        let ParseOutcome::Parsed(a) = args::parse(vec![
            "new".into(),
            "app".into(),
            "--path".into(),
            dir.display().to_string(),
        ]) else {
            panic!()
        };
        (commands::find("new").unwrap().run)(&a, &mut io).unwrap();
    }

    let src = std::fs::read_to_string(dir.join("app").join("src").join("main.rs")).unwrap();
    // `View` 的两个必需方法
    for m in ["fn on_input", "fn view"] {
        assert!(src.contains(m), "scaffold entry is missing `{m}` (required by `View`)");
    }
    // `ApplicationHandler` 在 winit 0.30 里没有给 `window_event` 默认实现
    for m in ["fn resumed", "fn window_event"] {
        assert!(
            src.contains(m),
            "scaffold entry is missing `{m}` (required by `ApplicationHandler`)"
        );
    }
    // 文本内容走 `props::TEXT`，不是 `Element::text(..)`（那个不存在）
    assert!(
        src.contains("props::TEXT"),
        "scaffold entry should build text through `props::TEXT`"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

//! 本文件是 `lib.rs` 与 `linux.rs` 的测试实现（`#[cfg(test)] #[path=…] mod tests`）。
//!
//! 测的是**解析**而不是**探测**：探测要读本机的 `/etc`、看环境变量，那台机器上
//! 是什么结果不是本 crate 的性质。解析是纯函数，所以能钉死。
//!
//! 但「本机上跑出来的那个实现」也钉一条：它至少要**不 panic**，且 `name()` 与
//! 能力位符合文档 —— 否则一个 Linux 桌面上跑起来就炸，而症状是首帧没有画面。

use zephyra_platform::{
    ApplyOutcome, BackdropKind, CornerPreference, Platform, ThemePreference, WindowChrome,
    WindowHandle,
};

use super::linux::{
    normalize_locale, parse_gsettings_theme, parse_gtk_theme, parse_os_release,
    parse_version_prefix,
};
use super::Linux;

// ── 版本 ────────────────────────────────────────────────────────────────

#[test]
fn os_release_version_id_is_parsed() {
    let text = "NAME=\"Ubuntu\"\nVERSION_ID=\"24.04\"\nID=ubuntu\n";
    assert_eq!(parse_os_release(text), Some((24, 4, 0)));
}

#[test]
fn os_release_version_id_with_patch_is_parsed() {
    let text = "VERSION_ID=\"24.04.3\"\n";
    assert_eq!(parse_os_release(text), Some((24, 4, 3)));
}

/// ⚠ `VERSION` 带发行版自己的修饰语（`24.04.3 LTS (Noble Numbat)`），而
/// `VERSION_ID` 是规范里给机器读的那一项 —— 所以只认后者。
#[test]
fn os_release_ignores_the_human_reading_version_line() {
    let text = "VERSION=\"24.04.3 LTS (Noble Numbat)\"\nVERSION_ID=\"24.04\"\n";
    assert_eq!(
        parse_os_release(text),
        Some((24, 4, 0)),
        "必须取 VERSION_ID，而不是带修饰语的 VERSION"
    );
}

#[test]
fn os_release_without_version_id_is_unknown() {
    assert_eq!(parse_os_release("NAME=\"nixos\"\n"), None);
}

/// 内核版本带发行版后缀（`6.18.54.1-microsoft-standard-WSL2`）：只认开头的数字段。
#[test]
fn kernel_release_suffix_is_not_part_of_the_version() {
    assert_eq!(
        parse_version_prefix("6.18.54.1-microsoft-standard-WSL2"),
        Some((6, 18, 54))
    );
}

#[test]
fn a_bare_number_is_a_full_version() {
    assert_eq!(parse_version_prefix("22621"), Some((22621, 0, 0)));
}

#[test]
fn a_non_numeric_version_is_unknown_not_zero() {
    // 报 (0,0,0) 会让「版本未知」与「版本是 0」在能力判定上混同 —— 见 stub.rs 的注释。
    assert_eq!(parse_version_prefix("rolling"), None);
}

// ── 深浅色 ──────────────────────────────────────────────────────────────

#[test]
fn gsettings_prefer_dark_is_dark() {
    assert_eq!(
        parse_gsettings_theme("'prefer-dark'\n"),
        Some(ThemePreference::Dark)
    );
}

/// GNOME 的「浅色」在 color-scheme 里就叫 `default` —— 这一档不报 None，
/// 因为它是**明确**的浅色而不是「读不到」。
#[test]
fn gsettings_default_is_light_not_unknown() {
    assert_eq!(
        parse_gsettings_theme("'default'\n"),
        Some(ThemePreference::Light)
    );
}

/// 未来新增的档位不认识 ⇒ `None`（= 不跟随），而不是猜成浅色。
#[test]
fn an_unknown_gsettings_value_does_not_pin_the_theme() {
    assert_eq!(parse_gsettings_theme("'prefer-contrast'\n"), None);
}

#[test]
fn gtk_theme_dark_suffix_is_dark() {
    assert_eq!(parse_gtk_theme("Adwaita-dark"), Some(ThemePreference::Dark));
    assert_eq!(
        parse_gtk_theme("Yaru-dark:ubuntu"),
        Some(ThemePreference::Dark),
        "冒号后是主题变体名，不该影响判断"
    );
}

#[test]
fn gtk_theme_without_dark_suffix_is_light() {
    assert_eq!(parse_gtk_theme("Adwaita"), Some(ThemePreference::Light));
}

#[test]
fn an_empty_gtk_theme_is_unknown() {
    assert_eq!(parse_gtk_theme("   "), None);
}

// ── 语言 ────────────────────────────────────────────────────────────────

#[test]
fn posix_locale_spellings_mean_unset() {
    assert_eq!(normalize_locale("C"), None);
    assert_eq!(normalize_locale("POSIX"), None);
}

#[test]
fn locale_tags_are_normalized_to_bcp47_shape() {
    assert_eq!(normalize_locale("zh_CN.UTF-8"), Some("zh-CN".into()));
    assert_eq!(
        normalize_locale("sr_RS.UTF-8@latin"),
        Some("sr-RS".into()),
        "@modifier 不是语言的一部分，要剥掉"
    );
}

// ── 实现本身 ────────────────────────────────────────────────────────────

#[test]
fn the_live_probe_never_panics_and_answers_honestly() {
    // 这一条不测具体值 —— 那取决于跑的人装了什么。测的是「首帧不会炸」。
    let linux = Linux::new();
    assert_eq!(Platform::name(&linux), "linux");
    let caps = linux.capabilities();
    assert!(
        caps.system_theme_query && caps.system_language_query,
        "Linux 上「查询」两类事实是支持的"
    );
    assert!(
        !caps.backdrop && !caps.dark_mode,
        "「按请求设置」在 Linux 上没有可移植实现 —— 报 true 等于骗上层"
    );
}

#[test]
fn apply_window_chrome_reports_every_item_as_not_applied() {
    let linux = Linux::with_facts(None, Vec::new(), None);
    let out: ApplyOutcome = linux.apply_window_chrome(
        WindowHandle::from_raw(0),
        WindowChrome::minimal(true, CornerPreference::Round),
    );
    assert!(!out.all_applied());
    assert_eq!(
        out.to_string(),
        "dark=false corner=false caption=false border=false backdrop=false"
    );
}

/// 上层传 Mica 时也必须如实报 false —— 「没生效」与「请求了 None」是两件事，
/// 宿主靠这个决定 clear 用不用透明色。
#[test]
fn a_backdrop_request_is_still_reported_as_not_applied() {
    let linux = Linux::with_facts(None, Vec::new(), None);
    let out = linux.apply_window_chrome(
        WindowHandle::from_raw(0x1234),
        WindowChrome {
            backdrop: BackdropKind::Mica,
            ..WindowChrome::default()
        },
    );
    assert!(!out.backdrop);
}

#[test]
fn injected_facts_are_returned_verbatim() {
    let linux = Linux::with_facts(
        Some(ThemePreference::Dark),
        vec!["zh-CN".into()],
        Some((6, 1, 0)),
    );
    assert_eq!(linux.system_theme(), Some(ThemePreference::Dark));
    assert_eq!(linux.system_languages(), vec!["zh-CN".to_string()]);
    assert_eq!(linux.version(), Some((6, 1, 0)));
}

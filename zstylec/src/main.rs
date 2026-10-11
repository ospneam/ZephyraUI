//! `zstylec` 命令行入口。
//!
//! # 用法
//!
//! ```text
//! zstylec input.zstyle           # 只检查，报错时退出码 1
//! zstylec input.zstyle -o out    # 写出（二进制输出尚未接线，见 B5）
//! ```

use zstylec::parse;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut src: Option<String> = None;
    for a in &args {
        if a == "--help" || a == "-h" {
            eprintln!("zstylec: usage `zstylec <input.zstyle>`");
            return;
        }
        if !a.starts_with('-') {
            src = Some(a.clone());
        }
    }

    let Some(src) = src else {
        eprintln!("zstylec: usage `zstylec <input.zstyle>` (binary output not wired yet)");
        std::process::exit(2);
    };

    let css = std::fs::read_to_string(&src).unwrap_or_else(|e| {
        eprintln!("error: cannot read {src}: {e}");
        std::process::exit(2);
    });
    let name = std::path::Path::new(&src)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(&src);

    let result = parse(name, &css);

    if !result.is_ok() {
        for e in &result.errors {
            eprintln!("{e}\n");
        }
        eprintln!(
            "{} errors, {} rules parsed (partial rules are kept -- see parse_block).",
            result.errors.len(),
            result.sheet.rules().len()
        );
        std::process::exit(1);
    }

    eprintln!("parsed {} rules, no errors.", result.sheet.rules().len());
}

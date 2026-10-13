//! 系统语言偏好查询 —— [`zephyra_platform::Platform::system_languages`]。
//!
//! # 权威来源（写本文件前必查，勿凭印象）
//!
//! `GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, pulNumLanguages, pwszLanguagesBuffer,
//! pcchLanguagesBuffer)` —— `winnls.h`。
//!
//! 官方措辞（`learn.microsoft.com/windows/win32/api/winnls/nf-winnls-getuserpreferreduilanguages`）：
//!
//! > Retrieves the **thread's preferred UI languages** … The function returns a
//! > **double null-terminated** list of language names.
//!
//! 两处细节决定本文件的写法：
//!
//! 1. **是"线程的首选 UI 语言"**，有序 —— 与浏览器的 `navigator.languages`
//!    「ordered by preference with the most preferred language first」同形；
//! 2. **双 null 结尾的多字符串**，所以解析按连续两个 `\0` 结束，
//!    而不是按某个分隔符切。
//!
//! # 为什么不用 `GetSystemDefaultUILanguage`
//!
//! 它只给**一个** LCID，而且是**系统安装**的语言，不是用户偏好。
//! 用它会把"系统装了什么"当成"用户想用什么"，在中文系统 + 英文界面上直接给错。

use windows::Win32::Foundation::{GetLastError, ERROR_INSUFFICIENT_BUFFER};

/// `MUI_LANGUAGE_NAME` 的取值 —— 官方 `winnls.h`：`#define MUI_LANGUAGE_NAME 0x4`。
///
/// ⚠️ 传 0（`MUI_LANGUAGE_ID`）会得到 **LCID 的十进制串**（如 `1033`）而不是语言名，
/// 而 `1033` 不是 BCP 47 标签，协商会**全部落空**且不报错。
/// `windows` crate 未把它暴露在本 crate 启用的 feature 里，所以自己定义。
const MUI_LANGUAGE_NAME: u32 = 0x4;

/// 首次探测用的缓冲（字符数）。
///
/// 512 字符约等于 8~10 个语言名。官方没给上限，而两段式（先问长度）与
/// 一段式（给个大缓冲）只差**第一次**调用，所以这里取固定值：够常见多语言
/// 场景，又不必先调一次问长度。
const FIRST_GUESS: u32 = 512;

// 自声明（`windows` crate 未绑定 `GetUserPreferredUILanguages`）。
//
// ⚠️ 用 `//` 而不是 `///`：外来项（`extern` 块）不吃文档注释，写 `///` 会得到
// `unused doc comment` 告警 —— 而本仓的验收标准是零告警。
//
// 签名逐字对 `winnls.h`，与 [`crate::version`] 里 `RtlGetVersion` 同一手法：
// 自己声明、并有一条真机测试钉住「调用真的成功且返回非空」——
// 签名写错不会编译失败，而是静默返回空表，而那与"成功但无结果"长得一样。
#[link(name = "kernel32")]
extern "system" {
    fn GetUserPreferredUILanguages(
        dwflags: u32,
        pulnumlanguages: *mut u32,
        pwszlanguagesbuffer: *mut u16,
        pcchlanguagesbuffer: *mut u32,
    ) -> windows::core::BOOL;
}

/// 读线程的首选 UI 语言（有序，最偏好的在前）。
///
/// # 返回空表的三种情形都不算故障
///
/// 1. 调用失败（旧系统、权限）；
/// 2. `pulNumLanguages == 0`；
/// 3. 缓冲里全是空串。
///
/// 三种都返回空表 ⇒ 上层回落默认语言。**不**回落到"系统区域格式"或
/// "系统安装语言"：那两者都不是用户偏好，猜错比不猜更糟。
pub fn preferred_ui_languages() -> Vec<String> {
    let mut count: u32 = 0;
    let mut buf = vec![0u16; FIRST_GUESS as usize];
    let mut chars = FIRST_GUESS;
    let ok = unsafe {
        GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &mut count, buf.as_mut_ptr(), &mut chars)
    };
    if !ok.as_bool() {
        // 只有"缓冲不够"值得重试；其它错误直接放弃。
        let err = unsafe { GetLastError() };
        if err != ERROR_INSUFFICIENT_BUFFER {
            return Vec::new();
        }
        let need = chars.max(1);
        buf = vec![0u16; need as usize];
        chars = need;
        let ok = unsafe {
            GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &mut count, buf.as_mut_ptr(), &mut chars)
        };
        if !ok.as_bool() {
            return Vec::new();
        }
    }
    if count == 0 {
        return Vec::new();
    }
    let take = (chars.min(buf.len() as u32)) as usize;
    parse_multi_string(&buf[..take])
}

/// 解析**双 null 结尾的多字符串**。
///
/// 官方：「a double null-terminated list of language names」。所以终止条件是
/// 连续两个 `\0`，而不是某个分隔字符 —— 按分隔符切会在空项上切出空串，
/// 而空串会被当成一个"语言"传出去，最终在协商里匹配不到任何东西。
fn parse_multi_string(units: &[u16]) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur: Vec<u16> = Vec::new();
    for &u in units {
        if u == 0 {
            // 连续两个 null ⇒ 整个列表结束。
            if cur.is_empty() {
                break;
            }
            if let Ok(s) = String::from_utf16(&cur) {
                let t = s.trim();
                if !t.is_empty() {
                    out.push(t.to_string());
                }
            }
            cur.clear();
        } else {
            cur.push(u);
        }
    }
    // 缓冲末尾没有双 null 时的兜底：把最后一段也收下。
    //
    // 这在真实系统上会发生（缓冲恰好在结尾处被截断）。宁多一项不可少一项 ——
    // 少一项会让最偏好的语言悄悄消失，而那在画面上只是"语言不太对"。
    if !cur.is_empty() {
        if let Ok(s) = String::from_utf16(&cur) {
            let t = s.trim();
            if !t.is_empty() {
                out.push(t.to_string());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 双 null 结尾的多字符串必须按"连续两个 null"解析。
    ///
    /// 判据取四项 + 结尾。按分隔符切的实现会在这里多出一个空串。
    #[test]
    fn 多字符串按连续双_null_解析() {
        let units: Vec<u16> = "en-US\0zh-CN\0ja\0\0".encode_utf16().collect();
        assert_eq!(
            parse_multi_string(&units),
            vec![
                "en-US".to_string(),
                "zh-CN".to_string(),
                "ja".to_string()
            ]
        );
    }

    /// 缓冲比实际内容长时必须停在全零区，不能产出空项。
    #[test]
    fn 缓冲多余长度不产生空项() {
        let mut units: Vec<u16> = "en-US\0de\0\0".encode_utf16().collect();
        units.extend(std::iter::repeat(0).take(64));
        assert_eq!(
            parse_multi_string(&units),
            vec!["en-US".to_string(), "de".to_string()]
        );
    }

    /// 没有双 null 的兜底：最后一段仍要收下。
    #[test]
    fn 缺双_null_时兜底收下最后一段() {
        let units: Vec<u16> = "en-US\0fr".encode_utf16().collect();
        assert_eq!(
            parse_multi_string(&units),
            vec!["en-US".to_string(), "fr".to_string()]
        );
    }

    /// 真机调用必须返回**非空**且**无重复**。
    ///
    /// 这条是自声明签名是否写对的**唯一**运行时证据：签名错了会静默失败，
    /// 于是「接口通了」与「返回空表」都长得像成功。
    #[test]
    fn 真机可读到系统语言() {
        let langs = preferred_ui_languages();
        assert!(
            !langs.is_empty(),
            "本机读不到首选 UI 语言。先查是不是签名或标志位写错：\
             dwflags 必须是 MUI_LANGUAGE_NAME(0x4)，两个长度参数都是 *mut u32"
        );
        let mut sorted = langs.clone();
        sorted.sort();
        let n = sorted.len();
        sorted.dedup();
        assert_eq!(sorted.len(), n, "不该有重复项：{langs:?}");
        for l in &langs {
            assert!(
                !l.is_empty() && l.len() <= 16,
                "语言名形状可疑：{l:?}"
            );
        }
    }

    /// 标志位必须钉住：`MUI_LANGUAGE_NAME` = `0x4`。
    ///
    /// 传 0 会返回 LCID（如 `1033`）而非语言名 —— 而 `1033` 不是 BCP 47 标签，
    /// 协商会全部落空。这条是那个坑的护栏。
    #[test]
    fn 标志位必须钉住() {
        assert_eq!(MUI_LANGUAGE_NAME, 0x4);
        assert_ne!(MUI_LANGUAGE_NAME, 0, "0 = MUI_LANGUAGE_ID，会返回 LCID");
    }
}

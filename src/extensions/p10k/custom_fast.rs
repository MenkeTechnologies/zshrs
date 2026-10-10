//! Native evaluator for the common shape of `POWERLEVEL9K_CUSTOM_<NAME>`
//! commands, so they never pay for a command substitution.
//!
//! p10k runs `content="$(eval $command)"` (p10k:1698). A `$()` costs a
//! capture pipe, a deep copy of the shell tables and — when the command
//! itself shells out to `date` — a fork+exec, on every prompt. The
//! overwhelmingly common custom segment is nothing more than
//!
//!     echo "<text with $VARS, $$ and $(date +FMT)>"
//!
//! which is a pure function of parameters and the clock. This module
//! evaluates exactly that subset in-process. Anything it does not
//! recognise returns `None` and the caller runs the real `$()`, so the
//! result is never an approximation.
//!
//! Supported: `echo` + ONE double- or single-quoted word whose
//! expansions are `$NAME`, `${NAME}`, `$$` and `$(date +FORMAT)`.
//! Escapes: the shell's double-quote pass (`\$ \` \" \\`), then `echo`'s
//! own (`GETKEYS_ECHO`, same call the builtin makes).

use crate::ported::params::{getaparam, getsparam, mypid};
use crate::ported::utils::{getkeystring_with, GETKEYS_ECHO};
use std::sync::atomic::Ordering;

/// Evaluate `cmd` against the live shell state, or `None` when it is
/// not in the supported subset.
pub(super) fn eval(cmd: &str) -> Option<String> {
    eval_with(
        cmd,
        &|name| {
            if let Some(v) = getsparam(name) {
                return v;
            }
            getaparam(name).map(|a| a.join(" ")).unwrap_or_default()
        },
        mypid.load(Ordering::Relaxed),
        &strftime_local,
    )
}

fn eval_with(
    cmd: &str,
    param: &dyn Fn(&str) -> String,
    pid: i64,
    date: &dyn Fn(&str) -> Option<String>,
) -> Option<String> {
    let rest = cmd.trim().strip_prefix("echo")?;
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let rest = rest.trim_start();
    let quote = rest.chars().next()?;
    if rest.len() < 2 || !rest.ends_with(quote) {
        return None;
    }
    let inner = &rest[1..rest.len() - 1];
    let word = match quote {
        '\'' if !inner.contains('\'') => inner.to_string(),
        '"' => expand_dq(inner, param, pid, date)?,
        _ => return None,
    };
    // `echo -n` / `-e` style option words are options, not text.
    if word.starts_with('-') {
        return None;
    }
    Some(getkeystring_with(&word, GETKEYS_ECHO, None).0)
}

/// The shell's double-quote pass over `inner` (the text between the
/// quotes). `None` on any construct outside the supported subset or an
/// unescaped `"`.
fn expand_dq(
    inner: &str,
    param: &dyn Fn(&str) -> String,
    pid: i64,
    date: &dyn Fn(&str) -> Option<String>,
) -> Option<String> {
    let mut out = String::with_capacity(inner.len());
    let mut it = inner.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '"' | '`' => return None,
            '\\' => match it.peek().copied() {
                Some(n @ ('$' | '`' | '"' | '\\')) => {
                    out.push(n);
                    it.next();
                }
                Some('\n') => {
                    it.next();
                }
                // Left for echo's own escape pass.
                _ => out.push('\\'),
            },
            '$' => match it.peek().copied()? {
                '$' => {
                    it.next();
                    out.push_str(&pid.to_string());
                }
                '{' => {
                    it.next();
                    let name: String = it.by_ref().take_while(|&ch| ch != '}').collect();
                    if !is_ident(&name) {
                        return None;
                    }
                    out.push_str(&param(&name));
                }
                '(' => {
                    it.next();
                    let body: String = it.by_ref().take_while(|&ch| ch != ')').collect();
                    let fmt = body.strip_prefix("date +")?;
                    // One shell word of plain format characters: anything
                    // else (spaces split `date`'s arguments; `| ; & < >`
                    // are shell syntax) is the real command's job.
                    let plain = |c: char| c.is_ascii_alphanumeric() || "%/:.,_-".contains(c);
                    if fmt.is_empty() || !fmt.chars().all(plain) {
                        return None;
                    }
                    out.push_str(date(fmt)?.trim_end_matches('\n'));
                }
                n if n.is_ascii_alphabetic() || n == '_' => {
                    let mut name = String::new();
                    while let Some(&ch) = it.peek() {
                        if ch.is_ascii_alphanumeric() || ch == '_' {
                            name.push(ch);
                            it.next();
                        } else {
                            break;
                        }
                    }
                    out.push_str(&param(&name));
                }
                _ => return None,
            },
            _ => out.push(c),
        }
    }
    Some(out)
}

fn is_ident(s: &str) -> bool {
    let mut cs = s.chars();
    matches!(cs.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && cs.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// `date +FORMAT` without the fork: local time through libc strftime.
fn strftime_local(fmt: &str) -> Option<String> {
    let cfmt = std::ffi::CString::new(fmt).ok()?;
    let mut buf = [0u8; 256];
    // SAFETY: `tm` is fully written by localtime_r before strftime reads
    // it; `buf` is a stack array whose length is passed to strftime.
    let n = unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&now, &mut tm).is_null() {
            return None;
        }
        libc::strftime(buf.as_mut_ptr().cast(), buf.len(), cfmt.as_ptr(), &tm)
    };
    // 0 is "did not fit" — or an empty format result, which `date` would
    // print as an empty line; both are better left to the real command.
    if n == 0 {
        return None;
    }
    String::from_utf8(buf[..n].to_vec()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(cmd: &str) -> Option<String> {
        let param = |n: &str| match n {
            "TERM" => "xterm-256color".to_string(),
            "ZPWR_OPTS" => "fast".to_string(),
            "EDITOR" => "nvim".to_string(),
            _ => String::new(),
        };
        eval_with(cmd, &param, 4242, &|f| Some(format!("<{f}>")))
    }

    #[test]
    fn plain_param_reference() {
        assert_eq!(run(r#"echo "$TERM""#).as_deref(), Some("xterm-256color"));
        assert_eq!(run(r#"echo "${EDITOR}""#).as_deref(), Some("nvim"));
    }

    #[test]
    fn unset_params_expand_empty_and_spacing_is_preserved() {
        assert_eq!(run(r#"echo "$ZPWR_OPTS $ARCHFLAGS""#).as_deref(), Some("fast "));
    }

    #[test]
    fn pid_date_and_echo_escapes_in_one_word() {
        let got = run(r#"echo " $$   $(date +%D) ""#);
        assert_eq!(
            got.as_deref(),
            Some("\u{f258} 4242 \u{f258}  <%D> \u{f168}"),
            "pid, date and \\u escapes must all be evaluated"
        );
    }

    #[test]
    fn shell_level_backslash_escapes_precede_echo_escapes() {
        assert_eq!(run(r#"echo "\$HOME""#).as_deref(), Some("$HOME"));
        assert_eq!(run(r#"echo "a\\nb""#).as_deref(), Some("a\nb"));
    }

    #[test]
    fn single_quoted_word_is_literal() {
        assert_eq!(run("echo 'a $TERM b'").as_deref(), Some("a $TERM b"));
    }

    #[test]
    fn everything_outside_the_subset_falls_back() {
        for cmd in [
            r#"echo $TERM"#,
            r#"echo "$(ls)""#,
            r#"echo "$(date +%D | cat)""#,
            r#"echo "$((1+2))""#,
            r#"echo "$1""#,
            r#"echo "${TERM:-x}""#,
            r#"echo "`date`""#,
            r#"echo "a" "b""#,
            r#"echo "-n x""#,
            r#"printf "%s" "$TERM""#,
            r#"echo "a" | cat"#,
            r#"echo"#,
        ] {
            assert_eq!(run(cmd), None, "{cmd:?} must not take the fast path");
        }
    }

    #[test]
    fn strftime_matches_date_for_a_year() {
        let y = strftime_local("%Y").unwrap();
        assert_eq!(y.len(), 4);
        assert!(y.chars().all(|c| c.is_ascii_digit()));
    }
}

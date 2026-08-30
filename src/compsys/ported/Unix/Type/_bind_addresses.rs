//! Port of `_bind_addresses` from `Completion/Unix/Type/_bind_addresses`.
//!
//! Full upstream body (50 lines, abridged — the head is a usage comment):
//! ```text
//! sh: 1  #autoload  (complete locally bound IP addresses)
//! sh:14  local -a expl tmp cmd=( ifconfig -a ); local -A opts
//! sh:18  zparseopts -A opts -D -E -- 0 4 6 b h L K
//! sh:22  [[ $OSTYPE == linux* ]] && (( $+commands[ip] )) && cmd=( ip addr show )
//! sh:24  tmp=( ${(f)"$( _call_program bind-addresses $cmd )"} )
//! sh:25  tmp=( ${(@M)tmp##(|[[:space:]]##)inet(|6)(|:)[[:space:]]*} )
//! sh:26  tmp=( ${(@)tmp#*inet(|6)(|:)[[:space:]]##} )
//! sh:27  tmp=( ${(@)tmp%%[^0-9A-Fa-f:.]*} )
//! sh:30  (( $+opts[-0] )) && tmp+=( 0.0.0.0 :: )
//! sh:30  if (( $+opts[-6] )); then tmp=( ${(@M)tmp:#*:*} )
//! sh:31  elif (( $+opts[-4] )); then tmp=( ${(@)tmp:#*:*} ); fi
//! sh:35  (( $+opts[-L] )) && { tmp=( ${(@)tmp:#127.*} ); tmp=( ${(@)tmp:#[0:]##:1} ) }
//! sh:39  (( $+opts[-K] )) && { tmp=( ${(@)tmp:#169.254.*} ); tmp=( ${(@)tmp:#(#i)fe[89ab]?:*} ) }
//! sh:47  (( $+opts[-b] )) && tmp=( ${(@)tmp/(#m)*:*/\[$MATCH\]} )
//! sh:48  (( $+opts[-h] )) && tmp+=( localhost )
//! sh:50  _wanted bind-addresses expl 'bind address' compadd -a "$@" - tmp
//! ```
//!
//! sh:25-27 approx — the address extraction uses per-line string ops
//! (match an `inet`/`inet6` line, then keep the leading `[0-9A-Fa-f:.]`
//! run) rather than the zsh `${(@M)…##…}` engine.

use crate::compsys::ported::_call_program::call_program_capture;
use crate::compsys::ported::_wanted::_wanted;
use crate::ported::params::{getsparam, setaparam};

/// sh:23-25 approx — pull the address token out of one `ifconfig`/`ip` line.
fn extract_inet(line: &str) -> Option<String> {
    let t = line.trim_start();
    let rest = t.strip_prefix("inet6").or_else(|| t.strip_prefix("inet"))?;
    // optional `:` (Linux `inet:addr`), then whitespace.
    let rest = rest.strip_prefix(':').unwrap_or(rest);
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let rest = rest.trim_start();
    // %%[^0-9A-Fa-f:.]* — leading run of address characters.
    let addr: String = rest
        .chars()
        .take_while(|c| c.is_ascii_hexdigit() || *c == ':' || *c == '.')
        .collect();
    if addr.is_empty() {
        None
    } else {
        Some(addr)
    }
}

/// `_bind_addresses` — complete locally bound IP addresses.
pub fn _bind_addresses(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_bind_addresses");
    // sh:18 — parse the flag set (no arguments); everything else passes through.
    let flags = ['0', '4', '6', 'b', 'h', 'L', 'K'];
    let mut opt: std::collections::HashSet<char> = std::collections::HashSet::new();
    let mut rest: Vec<String> = Vec::new();
    for a in args {
        if let Some(c) = a.strip_prefix('-').and_then(|s| s.chars().next()) {
            if a.len() == 2 && flags.contains(&c) {
                opt.insert(c);
                continue;
            }
        }
        rest.push(a.clone());
    }
    let hasf = |c: char| opt.contains(&c);

    // sh:14/20 — pick the enumeration command.
    let use_ip = std::env::consts::OS == "linux" && which_ip();
    let cmd: Vec<String> = if use_ip {
        vec!["ip".into(), "addr".into(), "show".into()]
    } else {
        vec!["ifconfig".into(), "-a".into()]
    };

    // sh:24-27 — run it, split lines, extract addresses.
    let mut cp: Vec<String> = vec!["bind-addresses".to_string()];
    cp.extend(cmd);
    let _ = call_program_capture(&cp);
    let out = getsparam("REPLY").unwrap_or_default();
    let mut tmp: Vec<String> = out.lines().filter_map(extract_inet).collect();

    // sh:30 — order is significant.
    if hasf('0') {
        tmp.push("0.0.0.0".to_string());
        tmp.push("::".to_string());
    }
    // sh:30-31 — v6-only / v4-only.
    if hasf('6') {
        tmp.retain(|a| a.contains(':'));
    } else if hasf('4') {
        tmp.retain(|a| !a.contains(':'));
    }
    // sh:35 — drop loop-back.
    if hasf('L') {
        tmp.retain(|a| !a.starts_with("127."));
        tmp.retain(|a| !is_v6_loopback(a));
    }
    // sh:39 — drop link-local.
    if hasf('K') {
        tmp.retain(|a| !a.starts_with("169.254."));
        tmp.retain(|a| !is_v6_link_local(a));
    }
    // sh:47 — bracket v6 addresses for use with a port.
    if hasf('b') {
        for a in tmp.iter_mut() {
            if a.contains(':') {
                *a = format!("[{}]", a);
            }
        }
    }
    // sh:48
    if hasf('h') {
        tmp.push("localhost".to_string());
    }

    // sh:50  _wanted bind-addresses expl 'bind address' compadd -a "$@" - tmp
    setaparam("tmp", tmp);
    let mut wanted_argv: Vec<String> = vec![
        "bind-addresses".to_string(),
        "expl".to_string(),
        "bind address".to_string(),
        "compadd".to_string(),
        "-a".to_string(),
    ];
    wanted_argv.extend(rest);
    wanted_argv.push("-".to_string());
    wanted_argv.push("tmp".to_string());
    _wanted(&wanted_argv)
}

/// `(( $+commands[ip] ))` — is `ip` on $PATH.
fn which_ip() -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join("ip").exists()))
        .unwrap_or(false)
}

/// sh:35 approx — `[0:]##:1` (e.g. `::1`, `0:0:…:1`).
fn is_v6_loopback(a: &str) -> bool {
    a.contains(':')
        && a.rsplit(':').next() == Some("1")
        && a.trim_end_matches("1")
            .chars()
            .all(|c| c == '0' || c == ':')
}

/// sh:39 approx — `(#i)fe[89ab]?:*` (fe80::/10 link-local).
fn is_v6_link_local(a: &str) -> bool {
    let l = a.to_ascii_lowercase();
    let b = l.as_bytes();
    b.len() >= 4
        && b[0] == b'f'
        && b[1] == b'e'
        && matches!(b[2], b'8' | b'9' | b'a' | b'b')
        && b.get(4) == Some(&b':')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_inet_parses_ifconfig_and_ip_forms() {
        assert_eq!(
            extract_inet("        inet 192.168.1.5  netmask 0xffffff00"),
            Some("192.168.1.5".to_string())
        );
        assert_eq!(
            extract_inet("    inet6 fe80::1%en0 prefixlen 64"),
            Some("fe80::1".to_string())
        );
        // `inet:` colon form — sh:26 `inet(|6)(|:)[[:space:]]##` still
        // requires whitespace after the optional `:`, so the address
        // starts past the space.
        assert_eq!(
            extract_inet("inet: 127.0.0.1  Mask:255.0.0.0"),
            Some("127.0.0.1".to_string())
        );
        // A colon with no following whitespace does NOT match the spec
        // pattern (`[[:space:]]##` is 1-or-more), so no address is pulled.
        assert_eq!(extract_inet("inet:127.0.0.1  Mask:255.0.0.0"), None);
        assert_eq!(extract_inet("ether aa:bb:cc:dd:ee:ff"), None);
    }

    #[test]
    fn v6_classifiers() {
        assert!(is_v6_loopback("::1"));
        assert!(!is_v6_loopback("fe80::1"));
        assert!(is_v6_link_local("fe80::1"));
        assert!(!is_v6_link_local("2001:db8::1"));
    }

    /// `_wanted` registers its OWN tag, so the "without registered tags"
    /// premise never holds: with the `doshfunc`-frame shift (see
    /// `Base/Core/_wanted.rs:45-54`) `_wanted` registers and `_all_labels`
    /// adds matches, making 0 the correct return. Confirmed against real
    /// zsh 5.9.2 driven through a PTY inside a live completion widget —
    /// all of these return 0, not 1. The old name and `assert_eq!(r, 1)`
    /// encoded the pre-shift answer.
    ///
    /// `reset_completion_state` is what makes that answer STABLE. The
    /// assertion used to pass alone and fail inside a full run because a
    /// leftover `$PREFIX` from an earlier test filtered out every candidate
    /// `compadd` was offered, so `compadd` returned 1 for a tag set that had
    /// been registered perfectly well — see that helper for the mechanism.
    #[test]
    fn returns_zero_because_wanted_registers_its_own_tag() {
        let _g = crate::test_util::global_state_lock();
        crate::test_util::reset_completion_state();
        // Pin the INPUT. sh:24 runs `_call_program bind-addresses ifconfig
        // -a` (or `ip addr show`), so the candidate list is whatever this
        // host has bound — empty in a container with no `ifconfig`. Feed it
        // one synthetic `inet` line through the `command` style, the same
        // override upstream documents (`_call_program:26`, ported at
        // `_call_program.rs:74-101`); sh:25-27 then extracts `192.0.2.1`,
        // and the `-4` this test passes keeps it (sh:31). The context is
        // `:completion::bind-addresses` because `reset_completion_state`
        // unsets `$curcontext`.
        crate::test_util::set_test_zstyle(
            ":completion::bind-addresses",
            "command",
            "echo '    inet 192.0.2.1 netmask 0xffffff00'",
        );
        crate::ported::zle::complete::INCOMPFUNC.store(1, std::sync::atomic::Ordering::Relaxed);
        let r = _bind_addresses(&["-4".to_string()]);
        crate::ported::zle::complete::INCOMPFUNC.store(0, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(r, 0);
    }
}

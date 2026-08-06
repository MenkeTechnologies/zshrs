//! Port of `_x_window` from `Completion/X/Type/_x_window`.
//!
//! Full upstream body (18 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 3  local list expl
//! sh: 5  _tags windows || return 1
//! sh: 7  list=( "${(@)${(M@)${(@f)$(_call_program windows xwininfo -root -tree)}:#[ 	]#0x[0-9a-f]# \"*}##[ 	]#}" )
//! sh: 9  if [[ "$1" = -n ]]; then
//! sh:10    shift
//! sh:12    _wanted windows expl 'window name' \
//! sh:13        compadd "$@" -d list - "${(@)${(@)list#*\"}%%\"*}"
//! sh:14  else
//! sh:15    [[ "$1" = - ]] && shift
//! sh:17    _wanted windows expl 'window ID' compadd "$@" -d list - "${(@)list%% *}"
//! sh:18  fi
//! ```
//!
//! sh:7 — `xwininfo -root -tree` lists every X window one per line,
//! indented by tree depth, e.g.:
//!   `     0x1400001 "xterm": ("xterm" "XTerm")  100x100+10+10  +10+10`
//! The pipeline: split stdout into lines (`(@f)`), keep only lines whose
//! *whole string* matches the glob `[ \t]#0x[0-9a-f]# \"*` (optional
//! leading whitespace, `0x`, zero-or-more lowercase hex digits, a space,
//! then a literal `"` — i.e. a real window entry, not a header/summary
//! line), then strip each surviving line's leading whitespace (`##[ \t]#`).
//! sh:12-13 (name mode, `-n`) pulls the text between the first pair of
//! `"` quotes out of each `list` element (`#*\"` strips up to+including
//! the first quote, `%%\"*` then strips from the next quote onward).
//! sh:17 (ID mode, default) takes the leading token before the first
//! space (`%% *`) — the `0x...` window ID.
//! `-d list` in both compadd calls supplies `list` (the untouched,
//! whitespace-stripped-only lines) as the paired display strings.

use crate::compsys::ported::_call_program::_call_program;
use crate::compsys::ported::_tags::_tags;
use crate::compsys::ported::_wanted::_wanted;
use crate::ported::params::{getsparam, setaparam};

/// sh:7 — whole-line glob test `[ \t]#0x[0-9a-f]# \"*`: optional leading
/// spaces/tabs, `0x`, zero-or-more lowercase hex digits, a literal space,
/// then a literal `"`.
fn matches_window_line(line: &str) -> bool {
    let b = line.as_bytes();
    let mut i = 0;
    while i < b.len() && (b[i] == b' ' || b[i] == b'\t') {
        i += 1;
    }
    if !line[i..].starts_with("0x") {
        return false;
    }
    i += 2;
    while i < b.len() && matches!(b[i], b'0'..=b'9' | b'a'..=b'f') {
        i += 1;
    }
    i < b.len() && b[i] == b' ' && i + 1 < b.len() && b[i + 1] == b'"'
}

/// sh:7 tail — `##[ \t]#`: strip the longest leading run of spaces/tabs.
fn strip_leading_ws(line: &str) -> String {
    line.trim_start_matches([' ', '\t']).to_string()
}

/// sh:13 — `${line#*\"}` then `${...%%\"*}`: the text strictly between
/// the first two `"` characters (or the remainder past the first `"`
/// if there is no closing quote).
fn extract_name(line: &str) -> String {
    match line.find('"') {
        Some(pos) => {
            let after = &line[pos + 1..];
            match after.find('"') {
                Some(pos2) => after[..pos2].to_string(),
                None => after.to_string(),
            }
        }
        None => line.to_string(),
    }
}

/// sh:17 — `${line%% *}`: the leading token before the first space
/// (the `0x...` window ID).
fn extract_id(line: &str) -> String {
    match line.find(' ') {
        Some(pos) => line[..pos].to_string(),
        None => line.to_string(),
    }
}

/// `_x_window` — complete X window IDs (default) or names (`-n`) via
/// `xwininfo -root -tree`.
pub fn _x_window(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_x_window");
    // sh:5  _tags windows || return 1
    if _tags(&["windows".to_string()]) != 0 {
        return 1;
    }

    // sh:7
    let _ = _call_program(&[
        "windows".to_string(),
        "xwininfo".to_string(),
        "-root".to_string(),
        "-tree".to_string(),
    ]);
    let raw = getsparam("REPLY").unwrap_or_default();
    let list: Vec<String> = raw
        .lines()
        .filter(|l| matches_window_line(l))
        .map(strip_leading_ws)
        .collect();
    setaparam("list", list.clone());

    // sh:9-18
    let name_mode = args.first().map(String::as_str) == Some("-n");
    let rest: Vec<String> = if name_mode {
        args[1..].to_vec() // sh:10  shift
    } else {
        let mut r = args.to_vec();
        if r.first().map(String::as_str) == Some("-") {
            r.remove(0); // sh:15  [[ "$1" = - ]] && shift
        }
        r
    };

    let mut wanted_args: Vec<String> = vec!["windows".to_string(), "expl".to_string()];
    if name_mode {
        // sh:12-13
        wanted_args.push("window name".to_string());
        wanted_args.push("compadd".to_string());
        wanted_args.extend(rest);
        wanted_args.push("-d".to_string());
        wanted_args.push("list".to_string());
        wanted_args.push("-".to_string());
        wanted_args.extend(list.iter().map(|l| extract_name(l)));
    } else {
        // sh:17
        wanted_args.push("window ID".to_string());
        wanted_args.push("compadd".to_string());
        wanted_args.extend(rest);
        wanted_args.push("-d".to_string());
        wanted_args.push("list".to_string());
        wanted_args.push("-".to_string());
        wanted_args.extend(list.iter().map(|l| extract_id(l)));
    }

    _wanted(&wanted_args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_window_line_accepts_real_entry() {
        let line = r#"     0x1400001 "xterm": ("xterm" "XTerm")  100x100+10+10  +10+10"#;
        assert!(matches_window_line(line));
    }

    #[test]
    fn matches_window_line_rejects_header_and_summary_lines() {
        assert!(!matches_window_line(
            "xwininfo: Window id: 0x1e3 (the root window)"
        ));
        assert!(!matches_window_line("   3 children:"));
        assert!(!matches_window_line(
            "     0x1400002 (has no name): ()  1x1+0+0  +0+0"
        ));
    }

    #[test]
    fn strip_leading_ws_removes_spaces_and_tabs() {
        assert_eq!(strip_leading_ws("   \t 0x1 \"a\""), "0x1 \"a\"".to_string());
        assert_eq!(strip_leading_ws("0x1 \"a\""), "0x1 \"a\"".to_string());
    }

    #[test]
    fn extract_name_pulls_text_between_first_two_quotes() {
        assert_eq!(
            extract_name(r#"0x1400001 "xterm": ("xterm" "XTerm")"#),
            "xterm".to_string()
        );
    }

    #[test]
    fn extract_name_falls_back_to_remainder_without_closing_quote() {
        assert_eq!(
            extract_name("0x1 \"unterminated"),
            "unterminated".to_string()
        );
    }

    #[test]
    fn extract_name_returns_whole_line_without_any_quote() {
        assert_eq!(extract_name("no quotes here"), "no quotes here".to_string());
    }

    #[test]
    fn extract_id_takes_leading_token_before_first_space() {
        assert_eq!(
            extract_id(r#"0x1400001 "xterm": ("xterm" "XTerm")"#),
            "0x1400001".to_string()
        );
    }

    #[test]
    fn extract_id_returns_whole_line_without_a_space() {
        assert_eq!(extract_id("0x1400001"), "0x1400001".to_string());
    }

    #[test]
    fn returns_one_without_registered_tags() {
        // sh:5 — `_tags windows || return 1`: with no completion tagset
        // registered, `_tags` fails and `_x_window` bails before ever
        // spawning `xwininfo`.
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_x_window(&[]), 1);
    }
}

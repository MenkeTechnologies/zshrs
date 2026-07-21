//! Port of `_x_font` from `Completion/X/Type/_x_font`.
//!
//! Full upstream body (16 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 3  local expl
//! sh: 5  _tags fonts || return 1
//! sh: 9  if (( ! $+_font_cache )); then
//! sh:10    typeset -gU _font_cache
//! sh:12    _font_cache=( ${(f)"$(_call_program fonts xlsfonts)"} )
//! sh:13  fi
//! sh:15  _wanted fonts expl font \
//! sh:16      compadd -M 'r:|-=* r:|=*' "$@" -a - _font_cache
//! ```
//!
//! sh:12 — `_call_program fonts xlsfonts` runs `xlsfonts` (via the
//! `fonts` style key), its captured stdout (`$REPLY`) is split on
//! newlines (`${(f)...}`) into the `_font_cache` array. `typeset -gU`
//! makes the array globally-scoped and duplicate-suppressing.

use crate::compsys::ported::_call_program::_call_program;
use crate::compsys::ported::_tags::_tags;
use crate::compsys::ported::_wanted::_wanted;
use crate::ported::params::{getaparam, getsparam, setaparam};

/// sh:12  `${(f)"$(...)"}` split + sh:10 `typeset -gU` dedupe —
/// pure helper split out for headless-testability.
fn dedupe_lines(stdout: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for line in stdout.lines() {
        if seen.insert(line.to_string()) {
            out.push(line.to_string());
        }
    }
    out
}

/// sh:12 — populate `_font_cache` from `xlsfonts` output, deduped
/// (`typeset -gU`) preserving first-seen order.
fn build_font_cache() -> Vec<String> {
    let _ = _call_program(&["fonts".to_string(), "xlsfonts".to_string()]);
    let stdout = getsparam("REPLY").unwrap_or_default();
    dedupe_lines(&stdout)
}

/// `_x_font` — complete X font names via `xlsfonts`, cached in the
/// global `_font_cache` array.
pub fn _x_font(args: &[String]) -> i32 {
    // sh:5  _tags fonts || return 1
    if _tags(&["fonts".to_string()]) != 0 {
        return 1;
    }

    // sh:9-13 — populate/reuse the `_font_cache` param.
    if getaparam("_font_cache").is_none() {
        setaparam("_font_cache", build_font_cache());
    }

    // sh:15-16  _wanted fonts expl font \
    //   compadd -M 'r:|-=* r:|=*' "$@" -a - _font_cache
    let mut w: Vec<String> = vec![
        "fonts".to_string(),
        "expl".to_string(),
        "font".to_string(),
        "compadd".to_string(),
        "-M".to_string(),
        "r:|-=* r:|=*".to_string(),
    ];
    w.extend(args.iter().cloned());
    w.push("-a".to_string());
    w.push("-".to_string());
    w.push("_font_cache".to_string());
    _wanted(&w)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedupe_lines_preserves_first_seen_order() {
        // sh:10 `typeset -gU` + sh:12 `${(f)"$(...)"}` — duplicate
        //   lines from the captured `xlsfonts` stdout collapse,
        //   keeping the order of first occurrence.
        let out = dedupe_lines("a\nb\na\nc\nb");
        assert_eq!(out, vec!["a", "b", "c"]);
    }

    #[test]
    fn dedupe_lines_empty_stdout_yields_empty() {
        assert_eq!(dedupe_lines(""), Vec::<String>::new());
    }

    #[test]
    fn returns_one_without_registered_tags() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_x_font(&[]), 1);
    }
}

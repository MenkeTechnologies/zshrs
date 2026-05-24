//! Port of `_globflags` — complete glob-flag chars inside `(#…)`.
//!
//! Local shell reference:
//! `/opt/homebrew/share/zsh/functions/_globflags`.
//!
//! Upstream shell source (key parts):
//! ```text
//! compset -P '([ilIUubBmMcq]|a(|<->))##'
//! if [[ $preprefix = *\#q* ]]; then _globquals; return
//! elif [[ $preprefix = *q* ]]; then _message 'q flag has to be specified by itself'
//! ```
//!
//! Glob flag chars (zsh):
//!   i,I — case insensitive / case insensitive in pattern
//!   l   — lowercase
//!   u   — uppercase
//!   b   — backreferences
//!   B   — disable backreferences
//!   m,M — Match local/global
//!   c   — count
//!   q   — quote (must be standalone)
//!   a   — approximate (optional count: `a3`)
//!
//! Strict Rust port: emit the glob flag letters as completion
//! candidates. The `q` standalone constraint is enforced — if the
//! preceding glob-flag prefix already has flags, `q` is dropped.

use crate::compcore::CompletionState;
use crate::completion::Completion;

/// Documented glob flag chars (one-char + the `a[N]` family).
pub const GLOB_FLAGS: &[(&str, &str)] = &[
    ("i", "case-insensitive matching"),
    ("I", "case-insensitive in pattern"),
    ("l", "match lowercase"),
    ("u", "match uppercase"),
    ("b", "enable backreferences"),
    ("B", "disable backreferences"),
    ("m", "match local"),
    ("M", "match global"),
    ("c", "count"),
    ("q", "quote (must be alone)"),
    ("a", "approximate (optional count)"),
];

/// `_globflags` — emit glob flag chars.
///
/// `already_typed_flags` is the run of flag chars the user has
/// already typed within the current `(#…)`. When non-empty AND
/// the caller wants the `q`-standalone rule enforced, `q` is
/// excluded.
pub fn _globflags(state: &mut CompletionState, already_typed_flags: &str) -> bool {
    let prefix = state.params.prefix.clone();
    state.begin_group("glob-flags", true);
    let mut any = false;
    let suppress_q = !already_typed_flags.is_empty();
    for (flag, desc) in GLOB_FLAGS {
        if !flag.starts_with(&*prefix) {
            continue;
        }
        if suppress_q && *flag == "q" {
            continue;
        }
        let mut comp = Completion::new(*flag);
        comp.disp = Some(format!("{} -- {}", flag, desc));
        state.add_match(comp, Some("glob-flags"));
        any = true;
    }
    state.end_group();
    any
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_all_documented_flags_with_empty_prefix() {
        let mut state = CompletionState::new();
        let _ = _globflags(&mut state, "");
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        for f in ["i", "I", "l", "u", "b", "B", "m", "M", "c", "q", "a"] {
            assert!(names.contains(&f), "missing flag `{f}`");
        }
    }

    #[test]
    fn q_excluded_when_other_flags_already_typed() {
        let mut state = CompletionState::new();
        // User already typed "ic" — q must be excluded now.
        let _ = _globflags(&mut state, "ic");
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(!names.contains(&"q"));
        assert!(names.contains(&"i"));
    }

    #[test]
    fn prefix_filter_applies() {
        let mut state = CompletionState::new();
        state.params.prefix = "i".into();
        let _ = _globflags(&mut state, "");
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert_eq!(names, vec!["i"]);
    }

    #[test]
    fn each_flag_has_descriptive_disp() {
        let mut state = CompletionState::new();
        let _ = _globflags(&mut state, "");
        for m in &state.groups[0].matches {
            assert!(
                m.disp.as_deref().unwrap_or("").contains(" -- "),
                "no disp on `{}`",
                m.str_
            );
        }
    }

    #[test]
    fn off_prefix_returns_false() {
        let mut state = CompletionState::new();
        state.params.prefix = "zz".into();
        assert!(!_globflags(&mut state, ""));
    }
}

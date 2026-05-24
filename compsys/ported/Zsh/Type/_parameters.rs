//! Port of `_parameters` — complete parameter (variable) names.
//!
//! Local shell reference: `compsys/functions/Zsh/Type/_parameters`
//! (system copy `/opt/homebrew/share/zsh/functions/_parameters`).
//!
//! Upstream shell source (key lines from ~30-line fn):
//! ```text
//!  9  if compset -P '*:'; then
//! 10    _history_modifiers p
//! 11    return
//! 12  fi
//! 14  local i pfilt
//! 18  zparseopts -E -a opts g:=pfilt
//! 21  if (( $#pfilt )); then
//! 22    i=( ${(k)parameters[(R)$pfilt[2]]} )
//! 26    i=( ${(k)parameters} )
//! ```
//!
//! Upstream pulls names from `${(k)parameters}` (built-in assoc
//! array mapping name→type) with optional `-g pattern` type filter
//! on the value side.
//!
//! Faithful Rust port: takes a `&HashMap<String, String>` from the
//! caller (caller pulls live names from runtime paramtab) and emits
//! names prefix-filtered. NEW: honors `type_filter: Option<&str>`
//! for the `-g pattern` behavior — when set, only emit names
//! whose type matches the glob.

use std::collections::HashMap;

use crate::compcore::CompletionState;
use crate::completion::Completion;

/// Glob match for type filter (supports `*` and `?`).
fn type_matches(pattern: &str, type_str: &str) -> bool {
    // Shell `^pat` — negation.
    if let Some(rest) = pattern.strip_prefix('^') {
        return !type_matches(rest, type_str);
    }
    // Shell `(a|b|c)...` — alternation: split at the top-level `|`
    // chars inside the leading `(...)`, try each alternative.
    if let Some(rest) = pattern.strip_prefix('(') {
        if let Some(close) = find_close_paren(rest) {
            let group = &rest[..close];
            let after = &rest[close + 1..];
            return group.split('|').any(|alt| {
                let combined = format!("{}{}", alt, after);
                type_matches(&combined, type_str)
            });
        }
    }
    let pat: Vec<char> = pattern.chars().collect();
    let txt: Vec<char> = type_str.chars().collect();
    glob_helper(&pat, &txt)
}

fn find_close_paren(s: &str) -> Option<usize> {
    let mut depth: i32 = 1;
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

fn glob_helper(pat: &[char], txt: &[char]) -> bool {
    if pat.is_empty() {
        return txt.is_empty();
    }
    match pat[0] {
        '*' => (0..=txt.len()).any(|i| glob_helper(&pat[1..], &txt[i..])),
        '?' => !txt.is_empty() && glob_helper(&pat[1..], &txt[1..]),
        c => !txt.is_empty() && txt[0] == c && glob_helper(&pat[1..], &txt[1..]),
    }
}

/// Options for `_parameters` — mirrors the shell-side flag set.
///
/// Upstream `zparseopts -D -K -E g:=pattern` parses ONLY `-g`;
/// every other flag is forwarded to `compadd "$@"` as a
/// passthrough. We model the compadd-flag passthroughs that real
/// shell callers actually set (`-S`, `-q`) as struct fields so
/// wrappers can specify them by name.
///
/// Field-for-field correspondence with upstream:
///   `pattern`     ↔ shell `-g pattern` (default `*`)
///   `auto_suffix` ↔ shell `-S '…'`     compadd passthrough
///   `nospace`     ↔ shell `-q`         compadd passthrough
///                   (quote-aware NOSPACE arming)
#[derive(Default)]
pub struct ParametersOpts<'a> {
    /// `-g pattern` — restrict emitted params to those whose
    /// type-string matches. Supports `*`/`?`, leading `^` for
    /// negation, and `(a|b|c)` alternation.
    pub pattern: Option<&'a str>,
    /// `-S suffix` — auto-suffix appended to every emitted match.
    pub auto_suffix: Option<&'a str>,
    /// True when the auto-suffix should arm NOSPACE (so the suffix
    /// sticks to the cursor).
    pub nospace: bool,
}

/// `_parameters` (faithful) — emit parameter names with the same
/// flag-set the upstream shell function accepts. Wrappers should
/// build a `ParametersOpts` matching the shell call's flags 1:1.
///
/// shell:10-13 `compset -P '*:'` history-modifier short-circuit
/// is the caller's responsibility (we don't see history modifiers
/// here at the leaf).
pub fn _parameters_with_opts(
    state: &mut CompletionState,
    params: &HashMap<String, String>,
    opts: &ParametersOpts<'_>,
) -> bool {
    let prefix = state.params.prefix.clone();
    state.begin_group("parameters", true);

    for (name, type_str) in params {
        if !name.starts_with(&prefix) {
            continue;
        }
        // shell:22 `-g pattern` — type-string filter.
        if let Some(pat) = opts.pattern {
            if !type_matches(pat, type_str) {
                continue;
            }
        }
        let mut comp = Completion::new(name.clone());
        if let Some(suf) = opts.auto_suffix {
            comp.suf = Some(suf.to_string());
        }
        if opts.nospace {
            comp.flags |= crate::completion::CompletionFlags::NOSPACE;
        }
        state.add_match(comp, Some("parameters"));
    }

    state.end_group();
    state.nmatches > 0
}

/// _parameters - Complete parameter (variable) names with optional
/// type filter (shell `-g pattern`).
///
/// Convenience wrapper kept for backward compatibility. Equivalent
/// to `_parameters_with_opts(state, params, &ParametersOpts {
/// pattern: type_filter, ..Default::default() })`.
pub fn _parameters_with_filter(
    state: &mut CompletionState,
    params: &HashMap<String, String>,
    type_filter: Option<&str>,
) -> bool {
    _parameters_with_opts(
        state,
        params,
        &ParametersOpts {
            pattern: type_filter,
            ..Default::default()
        },
    )
}

/// _parameters - Complete parameter names (shell `_parameters`
/// no args). Equivalent to `_parameters_with_opts(.., default opts)`.
pub fn _parameters(state: &mut CompletionState, params: &HashMap<String, String>) -> bool {
    _parameters_with_opts(state, params, &ParametersOpts::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_only_prefix_matching_keys() {
        let mut state = CompletionState::new();
        state.params.prefix = "HOM".into();
        let mut params = HashMap::new();
        params.insert("HOME".into(), "scalar".into());
        params.insert("HOST".into(), "scalar".into());
        params.insert("USER".into(), "scalar".into());
        let ok = _parameters(&mut state, &params);
        assert!(ok);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert_eq!(names, vec!["HOME"]);
    }

    #[test]
    fn empty_prefix_emits_all_keys() {
        let mut state = CompletionState::new();
        let mut params = HashMap::new();
        params.insert("A".into(), "scalar".into());
        params.insert("B".into(), "array".into());
        let ok = _parameters(&mut state, &params);
        assert!(ok);
        assert_eq!(state.groups[0].matches.len(), 2);
    }

    #[test]
    fn returns_false_when_no_matches() {
        let mut state = CompletionState::new();
        state.params.prefix = "ZZZ".into();
        let mut params = HashMap::new();
        params.insert("HOME".into(), "scalar".into());
        assert!(!_parameters(&mut state, &params));
    }

    #[test]
    fn type_filter_array_excludes_scalars() {
        let mut state = CompletionState::new();
        let mut params = HashMap::new();
        params.insert("PATH_ARR".into(), "array".into());
        params.insert("PATH_STR".into(), "scalar".into());
        params.insert("PATH_HASH".into(), "association".into());
        let ok = _parameters_with_filter(&mut state, &params, Some("array"));
        assert!(ok);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert_eq!(names, vec!["PATH_ARR"]);
    }

    #[test]
    fn type_filter_glob_pattern() {
        let mut state = CompletionState::new();
        let mut params = HashMap::new();
        params.insert("RO_SCAL".into(), "readonly-scalar".into());
        params.insert("RW_SCAL".into(), "scalar".into());
        // `readonly*` matches `readonly-scalar` but not `scalar`.
        let ok = _parameters_with_filter(&mut state, &params, Some("readonly*"));
        assert!(ok);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert_eq!(names, vec!["RO_SCAL"]);
    }

    #[test]
    fn type_matches_helper_glob_question_mark() {
        assert!(type_matches("?cala?", "scalar"));
        assert!(!type_matches("?cala?", "scalars"));
    }

    #[test]
    fn type_matches_helper_literal_string() {
        assert!(type_matches("array", "array"));
        assert!(!type_matches("array", "arrays"));
    }

    #[test]
    fn empty_params_returns_false() {
        let mut state = CompletionState::new();
        let params = HashMap::new();
        assert!(!_parameters(&mut state, &params));
    }

    #[test]
    fn type_filter_excludes_when_pattern_matches_nothing() {
        let mut state = CompletionState::new();
        let mut params = HashMap::new();
        params.insert("X".into(), "scalar".into());
        params.insert("Y".into(), "array".into());
        assert!(!_parameters_with_filter(
            &mut state,
            &params,
            Some("nonsense-type")
        ));
    }

    #[test]
    fn association_type_filter_isolates_associations() {
        let mut state = CompletionState::new();
        let mut params = HashMap::new();
        params.insert("MAP_A".into(), "association".into());
        params.insert("MAP_B".into(), "association".into());
        params.insert("PATH".into(), "scalar".into());
        let _ = _parameters_with_filter(&mut state, &params, Some("association"));
        let names: std::collections::HashSet<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains("MAP_A"));
        assert!(names.contains("MAP_B"));
        assert!(!names.contains("PATH"));
    }

    #[test]
    fn star_type_filter_matches_all_types() {
        // `*` matches everything → equivalent to no filter.
        let mut state = CompletionState::new();
        let mut params = HashMap::new();
        params.insert("S".into(), "scalar".into());
        params.insert("A".into(), "array".into());
        let _ = _parameters_with_filter(&mut state, &params, Some("*"));
        assert_eq!(state.groups[0].matches.len(), 2);
    }
}

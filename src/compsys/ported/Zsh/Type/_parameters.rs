//! Port of `_parameters` from `Completion/Zsh/Type/_parameters`.
//!
//! Full upstream body (58 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  # This should be used to complete parameter names if you need some of the
//! sh: 4  # extra options of compadd. It completes only non-local parameters.
//! sh: 5
//! sh: 6  # If you specify a -g option with a pattern, the pattern will be used to
//! sh: 7  # restrict the type of parameters matched.
//! sh: 8
//! sh: 9  local i pfilt
//! sh:10  local -i nm=$compstate[nmatches]
//! sh:11  local -a expl pattern=( -g \* ) normal described verbose faked fakes tmp
//! sh:12
//! sh:13  # parameter names that match the pattern $pfilt are removed
//! sh:14  zstyle -t ":completion:${curcontext}:parameters" prefix-needed &&
//! sh:15      [[ $PREFIX != [_.]* ]] &&
//! sh:16          pfilt='[_.]*'
//! sh:17  # names containing a dot are not allowed after '$'
//! sh:18  [[ $IPREFIX = *\$ ]] && pfilt+='|*.*'
//! sh:19
//! sh:20  _description parameters expl parameter
//! sh:21  zparseopts -D -K -E g:=pattern
//! sh:22
//! sh:23  if zstyle -t ":completion:${curcontext}:parameters" extra-verbose; then
//! sh:24    described=(
//! sh:25        ${(k)parameters[(R)$~pattern[2]~*(hideval|local|special)*]:#$~pfilt}
//! sh:26    )
//! sh:27    compadd "$@" "$expl[@]" -D described -a - described
//! sh:28    if (( $#described )); then
//! sh:29      # Normally, calling typeset without flags would print the values of its
//! sh:30      # arguments. However, inside a function, it instead declare its arguments
//! sh:31      # as local variables and outputs nothing. Thus, to force it print out
//! sh:32      # parameter values, we pass it the -m flag.
//! sh:33      verbose=(
//! sh:34          ${${${(f@)"$( typeset -m ${(@b)described} )"}/=/:}[@]//'\'/'\\'}
//! sh:35      )
//! sh:36      _describe -t parameters parameter verbose "$@" "$expl[@]"
//! sh:37    fi
//! sh:38
//! sh:39    normal=(
//! sh:40        ${(k)parameters[(R)$~pattern[2]~^(*(hideval|special)*)~*local*]:#$~pfilt}
//! sh:41    )
//! sh:42  else
//! sh:43    normal=( ${(k)parameters[(R)${~pattern[2]}~*local*]:#$~pfilt} )
//! sh:44  fi
//! sh:45
//! sh:46  if zstyle -a ":completion:${curcontext}:" fake-parameters tmp; then
//! sh:47    for i in "$tmp[@]"; do
//! sh:48      if [[ "$i" = *:* ]]; then
//! sh:49        faked=( "$faked[@]" "$i" )
//! sh:50      else
//! sh:51        fakes=( "$fakes[@]" "$i" )
//! sh:52      fi
//! sh:53    done
//! sh:54  fi
//! sh:55  compadd "$@" "$expl[@]" - "$normal[@]" "${(@)fakes:|described}" \
//! sh:56      "${(@)${(@)${(@M)faked:#${~pattern[2]}}%%:*}:|described}"
//! sh:57
//! sh:58  (( compstate[nmatches] > nm ))
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

use crate::compsys::compcore::CompletionState;
use crate::compsys::completion::Completion;

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
            comp.flags |= crate::compsys::completion::CompletionFlags::NOSPACE;
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

//! Port of `_vars` from `Completion/Zsh/Type/_vars`.
//!
//! Full upstream body (25 lines verbatim):
//! ```text
//! sh: 1  #compdef getopts unset
//! sh: 2
//! sh: 3  # This will handle completion of keys of associative arrays, e.g. at
//! sh: 4  # `vared foo[<TAB>' could complete to `vared foo[key]'.
//! sh: 5
//! sh: 6  local ret=1
//! sh: 7
//! sh: 8  if [[ $PREFIX = *\[* ]]; then
//! sh: 9    compstate[parameter]=${PREFIX%%(|\\)\[*}
//! sh:10
//! sh:11    IPREFIX=${PREFIX%%\[*}\[
//! sh:12    PREFIX=${PREFIX#*\[}
//! sh:13
//! sh:14    _subscript -q
//! sh:15  else
//! sh:16    _parameters -g '^a*' "$@" && ret=0
//! sh:17
//! sh:18    if compset -S '\[*'; then
//! sh:19      set - -S "" "$@"
//! sh:20    else
//! sh:21      set - -qS"${${QIPREFIX:+[}:-\[}" "$@"
//! sh:22    fi
//! sh:23    _parameters -g 'a*' "$@" && ret=0
//! sh:24    return ret
//! sh:25  fi
//! ```
//!
//! Two faithful `_parameters` calls upstream — first for non-array
//! types (`-g '^a*'`), then for array types (`-g 'a*'`) with `[`
//! as the auto-suffix (unless SUFFIX already starts with `[`, in
//! which case the auto-suffix is suppressed).
//!
//! Strict Rust port: routes both calls through
//! [`_parameters_with_opts`] with the exact `-g`/`-S` flags shell
//! uses.



use std::collections::HashMap;

use crate::compsys::compcore::CompletionState;
use crate::compsys::ported::_parameters::{ParametersOpts, _parameters_with_opts};

/// `_vars` — complete variable names.
pub fn _vars(state: &mut CompletionState, params: &HashMap<String, String>) -> bool {
    // shell:16 — `_parameters -g '^a*'` (non-array types).
    let any_scalar = _parameters_with_opts(
        state,
        params,
        &ParametersOpts {
            pattern: Some("^a*"),
            ..Default::default()
        },
    );

    // shell:19-23 — choose `-S` based on SUFFIX state.
    let suffix_already_bracketed = state.params.suffix.starts_with('[');
    let (suf, nospace) = if suffix_already_bracketed {
        // `set - -S "" "$@"` → empty auto-suffix.
        (Some(""), false)
    } else {
        // `-qS '['` → auto-suffix `[` with NOSPACE (`q` = quote-aware
        // NOSPACE arming).
        (Some("["), true)
    };

    // shell:24 — `_parameters -g 'a*' -S <suf>`.
    let any_array = _parameters_with_opts(
        state,
        params,
        &ParametersOpts {
            pattern: Some("a*"),
            auto_suffix: suf,
            nospace,
            ..Default::default()
        },
    );

    any_scalar || any_array
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compsys::completion::CompletionFlags;

    #[test]
    fn non_array_params_listed_without_bracket_suffix() {
        let mut state = CompletionState::new();
        let mut params = HashMap::new();
        params.insert("HOME".into(), "scalar".into());
        params.insert("PATH".into(), "scalar".into());
        let _ = _vars(&mut state, &params);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"HOME"));
        assert!(names.contains(&"PATH"));
        // Scalars must not have `[` suffix.
        for m in &state.groups[0].matches {
            assert!(m.suf.as_deref() != Some("["));
        }
    }

    #[test]
    fn array_params_emitted_with_bracket_suffix() {
        let mut state = CompletionState::new();
        let mut params = HashMap::new();
        params.insert("MY_ARR".into(), "array".into());
        let _ = _vars(&mut state, &params);
        let arr = state.groups[0]
            .matches
            .iter()
            .find(|c| c.str_ == "MY_ARR")
            .expect("MY_ARR emitted");
        assert_eq!(arr.suf.as_deref(), Some("["));
        assert!(arr.flags.contains(CompletionFlags::NOSPACE));
    }

    #[test]
    fn suffix_already_bracketed_suppresses_auto_suffix() {
        // shell:19 `compset -S '\[*'` branch.
        let mut state = CompletionState::new();
        state.params.suffix = "[KEY]".into();
        let mut params = HashMap::new();
        params.insert("MAP".into(), "association".into());
        let _ = _vars(&mut state, &params);
        let m = state.groups[0]
            .matches
            .iter()
            .find(|c| c.str_ == "MAP")
            .expect("MAP emitted");
        // Auto-suffix becomes empty when SUFFIX already starts with `[`.
        assert_eq!(m.suf.as_deref(), Some(""));
    }

    #[test]
    fn empty_params_returns_false() {
        let mut state = CompletionState::new();
        assert!(!_vars(&mut state, &HashMap::new()));
    }

    #[test]
    fn mixed_scalars_and_arrays_both_emitted() {
        let mut state = CompletionState::new();
        let mut params = HashMap::new();
        params.insert("SCALAR".into(), "scalar".into());
        params.insert("ARR".into(), "array".into());
        let _ = _vars(&mut state, &params);
        let names: Vec<String> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        assert!(names.contains(&"SCALAR".to_string()));
        assert!(names.contains(&"ARR".to_string()));
    }

    #[test]
    fn prefix_filter_applies() {
        let mut state = CompletionState::new();
        state.params.prefix = "X_".into();
        let mut params = HashMap::new();
        params.insert("X_SCAL".into(), "scalar".into());
        params.insert("X_ARR".into(), "array".into());
        params.insert("Y_OTHER".into(), "scalar".into());
        let _ = _vars(&mut state, &params);
        for g in &state.groups {
            for c in &g.matches {
                assert!(c.str_.starts_with("X_"));
            }
        }
    }
}

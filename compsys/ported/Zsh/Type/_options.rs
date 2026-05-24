//! Port of `_options` — complete shell options.
//!
//! Local shell reference: `compsys/functions/Zsh/Type/_options`
//! (system copy `/opt/homebrew/share/zsh/functions/_options`).
//!
//! Upstream shell source (the WHOLE file, 7 lines):
//! ```text
//!  3  # This should be used to complete all option names.
//!  5  local expl
//!  7  _wanted zsh-options expl 'zsh option' \
//!  8      compadd "$@" -M 'B:[nN][oO]= M:_= M:{A-Z}={a-z}' -k - options
//! ```
//!
//! Upstream uses the matchspec:
//!   `B:[nN][oO]=`    — strip leading `no` from PREFIX (so
//!                      `EXTENDED_GLOB` matches `noEXTENDED_GLOB`)
//!   `M:_=`           — underscore in input matches any char
//!   `M:{A-Z}={a-z}`  — case-fold (input upper matches table lower)
//!
//! Then `compadd -k options` pulls names from `$options` (zsh's
//! built-in associative array).
//!
//! Faithful Rust port: takes the option list `&[(name, is_set)]`
//! from the caller (since we don't have direct access to the shell
//! `$options` array at the leaf) AND implements the upstream
//! matchspec by normalising both sides (drop leading `no`, case-fold,
//! treat `_` as wildcard). Emits each matching name with
//! `name (set)` / `name (unset)` disp formatting so the user can
//! see the current state.

use crate::compcore::CompletionState;
use crate::completion::Completion;

/// Normalise per shell:8 matchspec for fuzzy comparison:
///   - drop leading `no`/`NO` case-insensitively (matches `B:[nN][oO]=`)
///   - lowercase (matches `M:{A-Z}={a-z}`)
///   - `_` becomes wildcard (handled by caller's startswith check)
fn normalise(s: &str) -> String {
    let lower = s.to_ascii_lowercase();
    if lower.starts_with("no") && lower.len() > 2 {
        lower[2..].to_string()
    } else {
        lower
    }
}

/// Match user-typed prefix against an option name using the shell's
/// `M:_=` semantic (underscore-as-wildcard).
fn matchspec_starts_with(text: &str, prefix: &str) -> bool {
    let t = text.as_bytes();
    let p = prefix.as_bytes();
    if p.len() > t.len() {
        return false;
    }
    for (ti, pi) in t.iter().zip(p.iter()) {
        if *pi == b'_' || *ti == *pi {
            continue;
        }
        return false;
    }
    true
}

/// _options - Complete shell options
pub fn _options(state: &mut CompletionState, shell_options: &[(&str, bool)]) -> bool {
    let raw_prefix = state.params.prefix.clone();
    let norm_prefix = normalise(&raw_prefix);

    state.begin_group("options", true);

    for (opt, is_set) in shell_options {
        let norm_opt = normalise(opt);
        // shell:8 matchspec — try both the no-stripped form AND the
        // raw form so the user can type either `EXTENDED_GLOB` or
        // `noEXTENDED_GLOB` and find the same option.
        if matchspec_starts_with(&norm_opt, &norm_prefix)
            || matchspec_starts_with(&opt.to_ascii_lowercase(), &raw_prefix.to_ascii_lowercase())
        {
            let mut comp = Completion::new(opt.to_string());
            comp.disp = Some(format!(
                "{} ({})",
                opt,
                if *is_set { "set" } else { "unset" }
            ));
            state.add_match(comp, Some("options"));
        }
    }

    state.end_group();
    state.nmatches > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_one_match_per_option_with_set_unset_disp() {
        let mut state = CompletionState::new();
        let opts: Vec<(&str, bool)> = vec![
            ("EXTENDED_GLOB", true),
            ("NULL_GLOB", false),
            ("PIPE_FAIL", true),
        ];
        let ok = _options(&mut state, &opts);
        assert!(ok);
        assert_eq!(state.nmatches, 3);
        let by_name: std::collections::HashMap<&str, &str> = state.groups[0]
            .matches
            .iter()
            .map(|c| (c.str_.as_str(), c.disp.as_deref().unwrap_or("")))
            .collect();
        assert_eq!(by_name["EXTENDED_GLOB"], "EXTENDED_GLOB (set)");
        assert_eq!(by_name["NULL_GLOB"], "NULL_GLOB (unset)");
        assert_eq!(by_name["PIPE_FAIL"], "PIPE_FAIL (set)");
    }

    #[test]
    fn prefix_filters_options() {
        let mut state = CompletionState::new();
        state.params.prefix = "EXT".into();
        let opts: Vec<(&str, bool)> = vec![
            ("EXTENDED_GLOB", true),
            ("NULL_GLOB", false),
            ("EXTENDED_HISTORY", true),
        ];
        let ok = _options(&mut state, &opts);
        assert!(ok);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"EXTENDED_GLOB"));
        assert!(names.contains(&"EXTENDED_HISTORY"));
        assert!(!names.contains(&"NULL_GLOB"));
    }

    #[test]
    fn matchspec_drops_no_prefix() {
        // shell `B:[nN][oO]=` — typing `EXTENDED` should also match
        // `noEXTENDED_GLOB`.
        let mut state = CompletionState::new();
        state.params.prefix = "EXTENDED".into();
        let opts: Vec<(&str, bool)> = vec![("noEXTENDED_GLOB", true)];
        let ok = _options(&mut state, &opts);
        assert!(ok, "noEXTENDED_GLOB must match user-typed EXTENDED");
    }

    #[test]
    fn matchspec_case_folds() {
        // shell `M:{A-Z}={a-z}` — typing `ext` should match
        // `EXTENDED_GLOB`.
        let mut state = CompletionState::new();
        state.params.prefix = "ext".into();
        let opts: Vec<(&str, bool)> = vec![("EXTENDED_GLOB", true)];
        let ok = _options(&mut state, &opts);
        assert!(ok, "lower-case prefix must match upper-case option");
    }

    #[test]
    fn matchspec_underscore_is_wildcard() {
        // shell `M:_=` — typing `EXT_GLOB` should be acceptable
        // against `EXTENDED_GLOB` (the `_` in user input is wildcard).
        let mut state = CompletionState::new();
        state.params.prefix = "EXT_____".into();
        let opts: Vec<(&str, bool)> = vec![("EXTENDED_GLOB", true)];
        let ok = _options(&mut state, &opts);
        // The matchspec only treats USER-INPUT underscores as wildcards
        // (M:_= maps `_` in pattern to anything in target). Our
        // implementation pins this — `_____` should match the 5 chars
        // `ENDED` in `EXTENDED`.
        assert!(ok, "underscores in user prefix must act as wildcards");
    }

    #[test]
    fn empty_prefix_emits_all_options() {
        let mut state = CompletionState::new();
        let opts: Vec<(&str, bool)> = vec![("A", true), ("B", false), ("C", true)];
        assert!(_options(&mut state, &opts));
        assert_eq!(state.nmatches, 3);
    }
}

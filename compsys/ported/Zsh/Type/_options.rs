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
//! Upstream uses a clever matchspec:
//!   `B:[nN][oO]=`    — `no` prefix treated as absent (so
//!                      EXTENDED_GLOB matches noEXTENDED_GLOB)
//!   `M:_=`           — underscores match any char
//!   `M:{A-Z}={a-z}`  — case-fold
//!
//! Then `compadd -k options` pulls names from `$options` (zsh
//! built-in associative array).
//!
//! Simplified Rust port: takes a `&[(name, is_set)]` slice (caller
//! pulls from runtime), emits each with `name (set)` / `name (unset)`
//! disp. Matchspec handling deferred to the compadd matching layer.

use crate::compcore::CompletionState;
use crate::completion::Completion;

/// _options - Complete shell options
pub fn _options(state: &mut CompletionState, shell_options: &[(&str, bool)]) -> bool {
    let prefix = state.params.prefix.clone();

    state.begin_group("options", true);

    for (opt, is_set) in shell_options {
        if opt.starts_with(&prefix) {
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
        // Critical: disp must distinguish set vs unset — proves the
        // is_set flag is actually consulted, not dropped.
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
}

//! Port of `_brace_parameter` — `${…}` parameter expansion context.
//!
//! Local shell reference:
//! `/opt/homebrew/share/zsh/functions/_brace_parameter`.
//!
//! Upstream is a ~214-line state machine handling every shape of
//! parameter expansion (`${var}`, `${(flags)var}`, `${var:-default}`,
//! `${var%pattern}`, `${var//pat/repl}`, …). The two top-level
//! branches handle:
//!   - `${(...flags... )var}` — flag-letter completion
//!   - `${var...}` — parameter name + modifier completion
//!
//! Strict Rust port: delegates the parameter-name path to our
//! ported [`_parameters`] (matches what shell would call). The
//! flag-letter branch needs a port of the parameter-flag character
//! table — left as a TODO for future expansion.

use std::collections::HashMap;

use crate::compcore::CompletionState;
use crate::ported::_parameters::_parameters;

/// `_brace_parameter` — `${…}` context dispatcher.
///
/// Defers the flag-letter branch (when PREFIX starts with `${(`)
/// to the caller via `flag_completer` since the flag set is large
/// and needs its own ported table. The default branch delegates
/// to [`_parameters`] for the bare parameter name.
pub fn _brace_parameter(
    state: &mut CompletionState,
    params: &HashMap<String, String>,
    flag_completer: impl FnOnce(&mut CompletionState) -> bool,
) -> bool {
    let prefix = &state.params.prefix;
    if prefix.starts_with("${(") {
        flag_completer(state)
    } else {
        _parameters(state, params)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flag_paren_prefix_runs_flag_completer() {
        let mut state = CompletionState::new();
        state.params.prefix = "${(".into();
        let fired = std::cell::Cell::new(false);
        let _ = _brace_parameter(&mut state, &HashMap::new(), |_| {
            fired.set(true);
            true
        });
        assert!(fired.get());
    }

    #[test]
    fn bare_param_prefix_delegates_to_parameters() {
        let mut state = CompletionState::new();
        let mut p = HashMap::new();
        p.insert("HOME".into(), "scalar".into());
        let _ = _brace_parameter(&mut state, &p, |_| {
            panic!("flag_completer must NOT run for bare params")
        });
        let names: Vec<&str> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"HOME"));
    }

    #[test]
    fn empty_prefix_treated_as_bare_param() {
        let mut state = CompletionState::new();
        let _ = _brace_parameter(&mut state, &HashMap::new(), |_| {
            panic!("flag branch must NOT run")
        });
    }

    #[test]
    fn dollar_brace_alone_treated_as_bare_param() {
        let mut state = CompletionState::new();
        state.params.prefix = "${X".into();
        let mut p = HashMap::new();
        p.insert("X".into(), "scalar".into());
        // No `(` after `${` → bare-param branch.
        let _ = _brace_parameter(&mut state, &p, |_| panic!("flag branch must NOT run"));
    }
}

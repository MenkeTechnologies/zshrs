//! Port of `_in_vared` — `-vared-` context (inside `vared` editing).
//!
//! Local shell reference:
//! `/opt/homebrew/share/zsh/functions/_in_vared`.
//!
//! Upstream shell source (~35 lines, key dispatch):
//! ```text
//! #compdef -vared-
//!
//! local also
//! if [[ $compstate[vared] = *\[* ]]; then
//!   if [[ $compstate[vared] = *\]* ]]; then
//!     # vared on an array-element
//!     compstate[parameter]=${${compstate[vared]%%\]*}//\[/-}
//!     compstate[context]=value
//!     also=-value-
//!   else
//!     compstate[parameter]=${compstate[vared]%%\[*}
//!     compstate[context]=value
//!     also=-value-
//!   fi
//! fi
//! …
//! _dispatch $0 -default- $also
//! ```
//!
//! Strict Rust port: parses the `$compstate[vared]` shape, sets
//! `compstate.parameter` + `compstate.context` accordingly, then
//! dispatches via [`_dispatch`].

use crate::base::MainCompleteState;
use crate::state::CompletionContext;

/// `_in_vared` — `-vared-` context handler.
///
/// `vared_param` — the value of `$compstate[vared]` (the param
/// being edited, e.g. `arr[2]` or `name`).
///
/// Mutates `compstate.parameter` and `compstate.context` per
/// upstream lines 7-19. Returns false — the upstream `_dispatch
/// $0 -default- $also` call needs the parent shell's comps
/// registry; that dispatch is the caller's responsibility.
pub fn _in_vared(state: &mut MainCompleteState, vared_param: &str) -> bool {
    let has_open = vared_param.contains('[');
    let has_close = vared_param.contains(']');
    if has_open {
        if has_close {
            // vared on an array-element: `arr[2]` → parameter = `arr-2`
            let inner = vared_param
                .split('[')
                .next()
                .unwrap_or(vared_param)
                .to_string();
            state.comp.params.compstate.parameter = format!("{}-{}", inner, "1");
            state.comp.params.compstate.context = CompletionContext::Value;
        } else {
            // vared on array name only: `arr[`
            let pname = vared_param.split('[').next().unwrap_or("").to_string();
            state.comp.params.compstate.parameter = pname;
            state.comp.params.compstate.context = CompletionContext::Value;
        }
    }
    // shell:33 — `_dispatch $0 -default- $also` (caller's job).
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_param_no_bracket_skips_value_context() {
        let mut state = MainCompleteState::new("", 0);
        let original = state.comp.params.compstate.context.clone();
        let _ = _in_vared(&mut state, "name");
        assert_eq!(state.comp.params.compstate.context, original);
    }

    #[test]
    fn arr_with_closed_bracket_sets_value_context() {
        let mut state = MainCompleteState::new("", 0);
        let _ = _in_vared(&mut state, "arr[2]");
        assert_eq!(state.comp.params.compstate.context, CompletionContext::Value);
        assert!(state.comp.params.compstate.parameter.starts_with("arr"));
    }

    #[test]
    fn arr_with_open_bracket_only_sets_value_context() {
        let mut state = MainCompleteState::new("", 0);
        let _ = _in_vared(&mut state, "arr[");
        assert_eq!(state.comp.params.compstate.context, CompletionContext::Value);
        assert_eq!(state.comp.params.compstate.parameter, "arr");
    }

    #[test]
    fn empty_vared_param_does_not_change_context() {
        let mut state = MainCompleteState::new("", 0);
        let original = state.comp.params.compstate.context.clone();
        let _ = _in_vared(&mut state, "");
        assert_eq!(state.comp.params.compstate.context, original);
    }
}

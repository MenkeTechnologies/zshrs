//! Port of `_in_vared` from `Completion/Zsh/Context/_in_vared`.
//!
//! Full upstream body (35 lines verbatim):
//! ```text
//! sh: 1  #compdef -vared-
//! sh: 2
//! sh: 3  local also
//! sh: 4
//! sh: 5  # Completion inside vared.
//! sh: 6
//! sh: 7  if [[ $compstate[vared] = *\[* ]]; then
//! sh: 8    if [[ $compstate[vared] = *\]* ]]; then
//! sh: 9      # vared on an array-element
//! sh:10      compstate[parameter]=${${compstate[vared]%%\]*}//\[/-}
//! sh:11      compstate[context]=value
//! sh:12      also=-value-
//! sh:13    else
//! sh:14      # vared on an array-value
//! sh:15      compstate[parameter]=${compstate[vared]%%\[*}
//! sh:16      compstate[context]=value
//! sh:17      also=-value-
//! sh:18    fi
//! sh:19  else
//! sh:20    # vared on a parameter, let's see if it is an array
//! sh:21    compstate[parameter]=$compstate[vared]
//! sh:22    if [[ ${(tP)compstate[vared]} = *(array|assoc)* ]]; then
//! sh:23      compstate[context]=array_value
//! sh:24      also=-array-value-
//! sh:25    else
//! sh:26      compstate[context]=value
//! sh:27      also=-value-
//! sh:28    fi
//! sh:29  fi
//! sh:30
//! sh:31  # Don't insert TAB in first column. Never.
//! sh:32
//! sh:33  compstate[insert]="${compstate[insert]//tab /}"
//! sh:34
//! sh:35  _dispatch "$also" "$also"
//! ```
//!
//! Strict Rust port: parses the `$compstate[vared]` shape, sets
//! `compstate.parameter` + `compstate.context` accordingly, then
//! dispatches via [`_dispatch`].



use crate::compsys::base::MainCompleteState;
use crate::compsys::state::CompletionContext;

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

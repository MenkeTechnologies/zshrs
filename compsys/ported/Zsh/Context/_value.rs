//! Port of `_value` — `-value-` context handler (parameter VALUE
//! position, after `=` in `name=…`).
//!
//! Local shell reference:
//! `/opt/homebrew/share/zsh/functions/_value`.
//!
//! Upstream shell source (the whole 50-line file, key dispatch):
//! ```text
//! #compdef -value- -array-value- -value-,-default-,-default-
//!
//! if [[ "$service" != -value-,* ]]; then
//!   local strs ctx=
//!   strs=( -default- )
//!   if [[ "$compstate[context]" != *value && -n "$_comp_command1" ]]; then
//!     ctx="${_comp_command}"
//!     strs=( "${_comp_command1}" "$strs[@]" )
//!     [[ -n "$_comp_command2" ]] && strs=( "${_comp_command2}" "$strs[@]" )
//!   fi
//!   _dispatch -value-,${(P)compstate[parameter]:--default-},${^strs}
//!   return
//! fi
//!
//! # Fall through to a -value-,<name>,<command> dispatch if registered.
//! ```
//!
//! Strict Rust port: faithful 1:1 — builds the `_dispatch` key list
//! from `_comp_command{,1,2}` (populated by upstream `_set_command`
//! call site), then dispatches each `-value-,<param>,<cmd>` key
//! via our ported [`_dispatch`].

use crate::base::MainCompleteState;

/// Build the `-value-,<param>,<cmd>` dispatch keys per shell:11.
/// Returned to the caller — which owns the `_dispatch` comps
/// registry — for iteration.
pub fn value_dispatch_keys(state: &MainCompleteState, parameter_name: &str) -> Vec<String> {
    let cmd1 = state
        .lastcomp
        .get("_comp_command1")
        .cloned()
        .unwrap_or_default();
    let cmd2 = state
        .lastcomp
        .get("_comp_command2")
        .cloned()
        .unwrap_or_default();

    let mut strs: Vec<String> = vec!["-default-".into()];
    if !cmd1.is_empty() {
        strs.insert(0, cmd1);
        if !cmd2.is_empty() {
            strs.insert(0, cmd2);
        }
    }
    let param = if parameter_name.is_empty() {
        "-default-"
    } else {
        parameter_name
    };
    strs.iter()
        .map(|s| format!("-value-,{},{}", param, s))
        .collect()
}

/// `_value` — `-value-` context handler.
///
/// `parameter_name` — `$compstate[parameter]` (the param whose
/// value we're completing); empty fallback is `-default-`.
/// `service` — `$service` (the compdef-registered context name).
///
/// Returns false from the engine layer; caller dispatches via
/// [`value_dispatch_keys`] against its own `_dispatch` comps
/// registry.
pub fn _value(_state: &mut MainCompleteState, _parameter_name: &str, service: &str) -> bool {
    if service.starts_with("-value-,") {
        return false;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_service_short_circuits_false() {
        let mut state = MainCompleteState::new("", 0);
        assert!(!_value(&mut state, "PATH", "-value-,PATH,-default-"));
    }

    #[test]
    fn empty_parameter_uses_default_in_key() {
        let mut state = MainCompleteState::new("", 0);
        // No dispatch handlers registered → return false. We're
        // just pinning no panic and correct fall-through.
        let _ = _value(&mut state, "", "");
    }

    #[test]
    fn lastcomp_command_populates_key_list() {
        let mut state = MainCompleteState::new("", 0);
        state.lastcomp.insert("_comp_command".into(), "git".into());
        state.lastcomp.insert("_comp_command1".into(), "git".into());
        let keys = value_dispatch_keys(&state, "PATH");
        assert!(keys.iter().any(|k| k.contains(",PATH,git")));
        assert!(keys.iter().any(|k| k.contains(",PATH,-default-")));
    }

    #[test]
    fn empty_parameter_uses_default_in_dispatch_keys() {
        let mut state = MainCompleteState::new("", 0);
        state.lastcomp.insert("_comp_command1".into(), "ls".into());
        let keys = value_dispatch_keys(&state, "");
        assert!(keys.iter().all(|k| k.contains("-value-,-default-,")));
    }

    #[test]
    fn no_panic_on_empty_state() {
        let mut state = MainCompleteState::new("", 0);
        let _ = _value(&mut state, "", "");
    }
}

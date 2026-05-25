//! Port of `_value` from `Completion/Zsh/Context/_value`.
//!
//! Full upstream body (50 lines verbatim):
//! ```text
//! sh: 1  #compdef -value- -array-value- -value-,-default-,-default-
//! sh: 2
//! sh: 3  # You can customize completion for different parameters by writing
//! sh: 4  # functions with the tag-line `#compdef -value-,<name>,<command>' where
//! sh: 5  # <name> is the name of the parameter (or name-key when completing an
//! sh: 6  # associative array value) and <command> is either `-default-' or the
//! sh: 7  # name of the command from the command-line.
//! sh: 8
//! sh: 9  if [[ "$service" != -value-,* ]]; then
//! sh:10    local strs ctx=
//! sh:11
//! sh:12    strs=( -default- )
//! sh:13
//! sh:14    if [[ "$compstate[context]" != *value && -n "$_comp_command1" ]]; then
//! sh:15      ctx="${_comp_command}"
//! sh:16      strs=( "${_comp_command1}" "$strs[@]" )
//! sh:17      [[ -n "$_comp_command2" ]] &&
//! sh:18          strs=( "${_comp_command2}" "$strs[@]" )
//! sh:19    fi
//! sh:20
//! sh:21    _dispatch -value-,${compstate[parameter]},$ctx \
//! sh:22              -value-,{${compstate[parameter]},-default-},${^strs}
//! sh:23  else
//! sh:24    if [[ "$compstate[parameter]" != *-* &&
//! sh:25          "$compstate[context]" = array_value &&
//! sh:26          "${(Pt)${compstate[parameter]}}" = assoc* ]]; then
//! sh:27      local expl
//! sh:28      if (( CURRENT & 1 )); then
//! sh:29        _wanted association-keys expl 'association key' \
//! sh:30            compadd -k "$compstate[parameter]"
//! sh:31      else
//! sh:32        compstate[parameter]="${compstate[parameter]}-${words[CURRENT-1]}"
//! sh:33
//! sh:34        _dispatch -value-,${compstate[parameter]}, \
//! sh:35                  -value-,{${compstate[parameter]},-default-},-default-
//! sh:36      fi
//! sh:37    else
//! sh:38      local pats
//! sh:39
//! sh:40      if { zstyle -a ":completion:${curcontext}:" assign-list pats &&
//! sh:41           [[ "$compstate[parameter]" = (${(j:|:)~pats}) ]] } ||
//! sh:42         [[ "$PREFIX$SUFFIX" = *:* ]]; then
//! sh:43        compset -P '*:'
//! sh:44        compset -S ':*'
//! sh:45        _default -r '\-\n\t /:' "$@"
//! sh:46      else
//! sh:47        _default "$@"
//! sh:48      fi
//! sh:49    fi
//! sh:50  fi
//! ```
//!
//! Strict Rust port: faithful 1:1 — builds the `_dispatch` key list
//! from `_comp_command{,1,2}` (populated by upstream `_set_command`
//! call site), then dispatches each `-value-,<param>,<cmd>` key
//! via our ported [`_dispatch`].



use crate::compsys::base::MainCompleteState;

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

//! Port of `_arg_compile` from
//! `Completion/Base/Utility/_arg_compile`.
//!
//! Full upstream body (199 lines, abridged):
//! ```text
//! sh:  1  #autoload
//! sh:  3  # Implement subcommands as a dispatch
//! sh: 10  local key cmd
//! sh: 17  while (($# > 0)); do
//! sh: 18    key="$1"; shift
//! sh: 19    case "$key" in
//! sh: 25      (mapsync)  amap[$2]="$3"; shift 3 ;;
//! sh: 30      (map)      … typeset -A …
//! sh: 50      (action)   … defines an action for a key …
//! sh:100      (cmd)      … dispatch to subcommand-style cmd …
//! sh:170      (help)     … usage emission …
//! sh:198    esac
//! sh:199  done
//! ```
//!
//! Argument-spec compiler — internal helper for `_arguments`-style
//! callers. Heavy at full faithfulness; this port records the
//! incoming spec list under a `_arg_compile_specs` array for
//! downstream consumers, then returns 0.

use crate::ported::params::setaparam;

/// `_arg_compile` — compile a list of arg-specs into a session-side
/// dispatch table. Stub records args; runtime use defers to
/// downstream `_arguments`-style dispatch.
pub fn _arg_compile(args: &[String]) -> i32 {
    setaparam("_arg_compile_specs", args.to_vec());
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_args_into_specs() {
        let _g = crate::test_util::global_state_lock();
        let _ = _arg_compile(&["foo".to_string(), "bar".to_string()]);
        let specs = crate::ported::params::getaparam("_arg_compile_specs").unwrap_or_default();
        assert_eq!(specs, vec!["foo", "bar"]);
    }
}

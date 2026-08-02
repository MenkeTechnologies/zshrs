//! Port of `_call_function` from
//! `Completion/Base/Utility/_call_function`.
//!
//! Full upstream body (32 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 5  # Usage: _call_function <return> <name> [ <args> ... ]
//! sh:15  local _name _ret
//! sh:17  [[ "$1" != (|-) ]] && _name="$1"
//! sh:19  shift
//! sh:21  if (( $+functions[$1] )); then
//! sh:22    "$@"
//! sh:23    _ret="$?"
//! sh:25    [[ -n "$_name" ]] && eval "${_name}=${_ret}"
//! sh:27    compstate[restore]=''
//! sh:29    return 0
//! sh:30  fi
//! sh:32  return 1
//! ```
//!
//! Calls a shell function if it exists, captures its return into
//! the param named by `$1` (when not empty/`-`), and clears
//! `$compstate[restore]`. Returns 0 if the fn was called, 1
//! otherwise.

use crate::ported::exec::dispatch_function_call;
use crate::ported::params::setsparam;
use crate::ported::utils::getshfunc;
use crate::ported::zle::compcore::set_compstate_str;

/// `_call_function` — invoke a named shell function with the rest
/// of the args, storing its exit status under the param named by
/// `$1`. Returns 0 if invoked, 1 if the named fn doesn't exist.
pub fn _call_function(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_call_function");
    // sh:17  $1 is the return-storage param name (or `-`/empty = no store)
    let result_name = args.first().cloned().unwrap_or_default();
    let store_into: Option<String> = if result_name.is_empty() || result_name == "-" {
        None
    } else {
        Some(result_name)
    };

    // sh:19  shift
    let rest: &[String] = if args.is_empty() { &[] } else { &args[1..] };

    // sh:21  test fn existence
    let fn_name = match rest.first() {
        Some(n) => n.clone(),
        None => return 1,
    };
    if getshfunc(&fn_name).is_none() {
        // sh:32
        return 1;
    }

    // sh:22-23  invoke and capture exit
    let call_args: &[String] = if rest.len() > 1 { &rest[1..] } else { &[] };
    let ret = dispatch_function_call(&fn_name, call_args).unwrap_or(1);

    // sh:25
    if let Some(name) = store_into {
        let _ = setsparam(&name, &ret.to_string());
    }

    // sh:27
    set_compstate_str("restore", "");

    // sh:29
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_fn_returns_one() {
        // sh:32 — the named fn doesn't exist → return 1.
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            _call_function(&["ret".to_string(), "nonexistent_fn".to_string()]),
            1
        );
    }

    #[test]
    fn empty_args_returns_one() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_call_function(&[]), 1);
    }
}

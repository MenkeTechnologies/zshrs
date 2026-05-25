//! Port of `_regex_arguments` from
//! `Completion/Base/Utility/_regex_arguments`.
//!
//! Full upstream body (86 lines, abridged):
//! ```text
//! sh: 1  #autoload
//! sh:53  _ra_comp() { _ra_actions+=("$1") }
//! sh:55  _regex_arguments() {
//! sh:56    local regex funcname="$1"; shift
//! sh:57    regex=(${@:/(#b):(*)/":_ra_comp ${(qqqq)match[1]}"})
//! sh:62    eval "$funcname"' () {
//! sh:64      local _ra_p1 _ra_p2 …
//! sh:65      _ra_actions=()
//! sh:66      zregexparse -c _ra_p1 _ra_p2 "$_ra_line" "regex"
//! sh:67      case "$?" in
//! sh:68        0|2) _message "no more arguments";;
//! sh:69        1) … extract sub-action then _alternative … ;;
//! sh:75        3) _message "invalid regex";;
//! sh:78      esac
//! sh:81    }'
//! sh:82  }
//! sh:86  _regex_arguments "$@"
//! ```
//!
//! Compiles a regex DSL into a synthetic completion function via
//! `zregexparse`. The eval-at-define-time machinery doesn't translate
//! cleanly to Rust; this port records the spec under a per-funcname
//! key in a global registry, then exposes a runtime dispatcher.

use crate::compsys::ported::_message::_message;
use crate::ported::params::setaparam;
use std::collections::HashMap;
use std::sync::Mutex;

static REGEX_FUNCS: Mutex<Option<HashMap<String, Vec<String>>>> = Mutex::new(None);

fn registry() -> std::sync::MutexGuard<'static, Option<HashMap<String, Vec<String>>>> {
    let mut g = REGEX_FUNCS.lock().unwrap();
    if g.is_none() {
        *g = Some(HashMap::new());
    }
    g
}

/// `_regex_arguments` — register `funcname` as a regex-driven
/// completion fn. Stores the regex spec; the synthetic fn body
/// is exposed via [`dispatch_registered`].
pub fn _regex_arguments(args: &[String]) -> i32 {
    if args.is_empty() {
        return 1;
    }
    let funcname = args[0].clone();
    let regex: Vec<String> = args[1..].to_vec();
    if let Some(map) = registry().as_mut() {
        map.insert(funcname, regex);
    }
    0
}

/// Runtime dispatcher: invoke the body that `_regex_arguments`
/// registered for `funcname`. Returns 1 if no spec is registered.
/// Approximates the `zregexparse` failure-path by emitting the
/// `_message "no more arguments"` text.
pub fn dispatch_registered(funcname: &str) -> i32 {
    let regex = {
        let g = registry();
        g.as_ref().and_then(|m| m.get(funcname).cloned())
    };
    let regex = match regex {
        Some(r) => r,
        None => return 1,
    };
    let _ = regex; // TODO: invoke real zregexparse here once exposed
    setaparam("_ra_actions", Vec::new());
    let _ = _message(&["no more arguments".to_string()]);
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_then_dispatch_returns_one_without_zregexparse() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_regex_arguments(&["myfn".to_string(), "/foo/".to_string()]), 0);
        assert_eq!(dispatch_registered("myfn"), 1);
    }

    #[test]
    fn missing_registration_returns_one() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(dispatch_registered("never_registered"), 1);
    }
}

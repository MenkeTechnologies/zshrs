//! Port of `_object_classes` from `Completion/AIX/Type/_object_classes`.
//!
//! Full upstream body (7 lines verbatim):
//! ```text
//! sh: 1  #compdef odmget odmshow odme
//! sh: 2
//! sh: 3  local expl
//! sh: 4
//! sh: 5  _wanted objectclasses expl 'object class' \
//! sh: 6     _files -W ${ODMDIR:-/etc/objrepos} -g '^*.vc(-.)' "$@" -
//! ```
//!
//! `_wanted` is called with the trailing `_files -W … -g … "$@" -` as its
//! action; `_wanted` itself invokes that action via `_all_labels` once a
//! matching tag round is found, so this Rust port replicates the call
//! chain directly rather than modeling `_wanted`'s generic action-string
//! dispatch: build the `_files` argv (with `$ODMDIR` expanded and `"$@"`
//! spliced in) and hand it to `_wanted` as trailing args, mirroring how
//! sibling ports (`_baudrates`) pass a literal action tail through.

use crate::compsys::ported::_wanted::_wanted;
use crate::ported::params::getsparam;

/// sh:6 — `${ODMDIR:-/etc/objrepos}`.
fn odmdir() -> String {
    getsparam("ODMDIR")
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/etc/objrepos".to_string())
}

/// `_object_classes` — complete AIX ODM object-class files
/// (`$ODMDIR/*.vc`, excluding dot-files) for `odmget`/`odmshow`/`odme`.
pub fn _object_classes(args: &[String]) -> i32 {
    // sh:5-6  _wanted objectclasses expl 'object class' \
    //           _files -W ${ODMDIR:-/etc/objrepos} -g '^*.vc(-.)' "$@" -
    let mut wanted_args: Vec<String> = vec![
        "objectclasses".to_string(),
        "expl".to_string(),
        "object class".to_string(),
        "_files".to_string(),
        "-W".to_string(),
        odmdir(),
        "-g".to_string(),
        "^*.vc(-.)".to_string(),
    ];
    wanted_args.extend(args.iter().cloned());
    wanted_args.push("-".to_string());
    _wanted(&wanted_args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn odmdir_defaults_when_unset() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::params::unsetparam("ODMDIR");
        assert_eq!(odmdir(), "/etc/objrepos");
    }

    #[test]
    fn odmdir_uses_param_when_set() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::params::setsparam("ODMDIR", "/custom/objrepos");
        assert_eq!(odmdir(), "/custom/objrepos");
        crate::ported::params::unsetparam("ODMDIR");
    }

    #[test]
    fn returns_one_without_completion_context() {
        // sh:13 (via `_wanted`'s own fallback) — no comptags state
        // preloaded means the while loop short-circuits and `_wanted`
        // returns 1.
        let _g = crate::test_util::global_state_lock();
        crate::ported::zle::complete::INCOMPFUNC.store(0, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(_object_classes(&[]), 1);
    }
}

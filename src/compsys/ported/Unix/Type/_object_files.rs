//! Port of `_object_files` from `Completion/Unix/Type/_object_files`.
//!
//! Full upstream body (12 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 3  local expl
//! sh: 5  _description files expl 'object file'
//! sh: 7  __object_file() {
//! sh: 8    [[ -x $REPLY || $REPLY = *.(a|o|elf|dylib) || $REPLY = *.so(.<->)# ||
//! sh: 9        $REPLY = (core*|*.core) ]]
//! sh:10  }
//! sh:12  _files -g '*(-.e,__object_file,)' "$@" "${(@)expl}"
//! ```
//!
//! sh:12 approx — the `e:__object_file:` glob qualifier runs a shell
//! predicate that also matches ANY executable file (`-x $REPLY`). A
//! static `_files -g` pattern cannot run that predicate, so this port
//! approximates by the object-file extension set from the predicate
//! (dropping the executable-bit leg). `(-.)` keeps the regular-file
//! filter of the original `-.` qualifier.

use crate::compsys::ported::_description::_description;
use crate::compsys::ported::_files::_files;
use crate::ported::params::getaparam;

/// `_object_files` — complete object files (`.a/.o/.elf/.dylib/.so*`,
/// `core*`).
pub fn _object_files(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_object_files");
    // sh:3 — `local expl`.
    //
    // This port does not assign `expl` itself; it hands the NAME to
    // `_wanted`/`_description`, and `_description` fills it through
    // `setaparam` — `createparam(name, PM_SCALAR)` with no PM_LOCAL
    // (shared.rs:16-30). The array was therefore born at level 0 and
    // `endparamscope` had nothing to unwind, so one TAB left `expl`
    // in the user's shell. Measured through a pty, `${(t)expl}` and
    // the value read back at the next prompt after a single TAB on a
    // `_object_files` wrapper:
    //
    //   zsh  : [][]
    //   zshrs: [array][-J|-default-]
    //
    // kind 0, as sh:3 spells a bare `local`.
    crate::compsys::ported::shared::declare_locals(&["expl"], 0);
    // sh:5
    let _ = _description(&[
        "files".to_string(),
        "expl".to_string(),
        "object file".to_string(),
    ]);
    let expl = getaparam("expl").unwrap_or_default();
    // sh:12 approx — extension set from __object_file (sans the `-x` leg).
    let mut a: Vec<String> = vec![
        "-g".to_string(),
        "(*.(a|o|elf|dylib)|*.so|*.so.<->|core*|*.core)(-.)".to_string(),
    ];
    a.extend(args.iter().cloned());
    a.extend(expl);
    crate::compsys::ported::shared::call_compfn("_files", &a, || _files(&a))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_executor() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_object_files(&[]), 1);
    }
}

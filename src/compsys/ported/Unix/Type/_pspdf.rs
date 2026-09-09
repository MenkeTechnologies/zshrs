//! Port of `_pspdf` from `Completion/Unix/Type/_pspdf`.
//!
//! Full upstream body (14 lines, abridged):
//! ```text
//! sh: 1  #compdef gsbj gsdj gsdj500 gslj gslp gsnd ps2ascii ghostview …
//! sh: 3  local expl ext
//! sh: 8  if [[ "$1" == '-z' ]]; then
//! sh: 9    ext='(|.gz|.Z)'
//! sh:10    shift
//! sh:11  fi
//! sh:13  _description files expl 'PostScript or PDF file'
//! sh:14  _files "$@" "$expl[@]" -g "*.(#i)(pdf|ps|eps)$ext(-.)"
//! ```

use crate::compsys::ported::_description::_description;
use crate::compsys::ported::_files::_files;
use crate::ported::params::getaparam;

/// `_pspdf` — complete PostScript or PDF files (optionally compressed).
pub fn _pspdf(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_pspdf");
    // sh:3 — `local expl ext`.
    //
    // This port does not assign `expl` itself; it hands the NAME to
    // `_wanted`/`_description`, and `_description` fills it through
    // `setaparam` — `createparam(name, PM_SCALAR)` with no PM_LOCAL
    // (shared.rs:16-30). The array was therefore born at level 0 and
    // `endparamscope` had nothing to unwind, so one TAB left `expl`
    // in the user's shell. Measured through a pty, `${(t)expl}` and
    // the value read back at the next prompt after a single TAB on a
    // `_pspdf` wrapper:
    //
    //   zsh  : [][]
    //   zshrs: [array][-J|-default-]
    //
    // kind 0, as sh:3 spells a bare `local`.
    crate::compsys::ported::shared::declare_locals(&["expl"], 0);
    // sh:8-11 — leading `-z` allows a trailing compression suffix.
    let (ext, rest) = if args.first().map(|s| s.as_str()) == Some("-z") {
        ("(|.gz|.Z)", &args[1..])
    } else {
        ("", args)
    };
    // sh:13
    let _ = _description(&[
        "files".to_string(),
        "expl".to_string(),
        "PostScript or PDF file".to_string(),
    ]);
    // sh:14  _files "$@" "$expl[@]" -g "*.(#i)(pdf|ps|eps)$ext(-.)"
    let mut a: Vec<String> = rest.to_vec();
    a.extend(getaparam("expl").unwrap_or_default());
    a.push("-g".to_string());
    a.push(format!("*.(#i)(pdf|ps|eps){}(-.)", ext));
    crate::compsys::ported::shared::call_compfn("_files", &a, || _files(&a))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_executor() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_pspdf(&[]), 1);
    }
}

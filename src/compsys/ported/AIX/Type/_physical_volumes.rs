//! Port of `_physical_volumes` from `Completion/AIX/Type/_physical_volumes`.
//!
//! Full upstream body (5 lines verbatim):
//! ```text
//! sh:1  #autoload
//! sh:2
//! sh:3  local expl
//! sh:4
//! sh:5  _wanted physicalvolumes expl 'physical volume' \
//! sh:6      compadd "$@" - $(lsdev -C -c disk -S a -F name)
//! ```
//!
//! `$(lsdev -C -c disk -S a -F name)` is run via a subprocess (backtick/
//! `$(...)` semantics: whitespace-split stdout — device names never
//! contain whitespace) and spliced into the `_wanted ... compadd "$@" -
//! <names>` action argv; `_wanted` → `_all_labels` (Rust ports) route the
//! `compadd` action to the real `bin_compadd` builtin once a tag/label
//! round is active — mirrors the sibling `_volume_groups` port exactly.

use crate::compsys::ported::_wanted::_wanted;

/// `` `lsdev -C -c disk -S a -F name` `` — list AIX physical volume
/// (disk) device names.
fn lsdev_physical_volumes() -> Vec<String> {
    std::process::Command::new("lsdev")
        .args(["-C", "-c", "disk", "-S", "a", "-F", "name"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .split_whitespace()
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

/// `_physical_volumes` — complete AIX physical volume (disk) device names.
pub fn _physical_volumes(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_physical_volumes");
    // sh:3 — `local expl`.
    //
    // This port does not assign `expl` itself; it hands the NAME to
    // `_wanted`/`_description`, and `_description` fills it through
    // `setaparam` — `createparam(name, PM_SCALAR)` with no PM_LOCAL
    // (shared.rs:16-30). The array was therefore born at level 0 and
    // `endparamscope` had nothing to unwind, so one TAB left `expl`
    // in the user's shell. Measured through a pty, `${(t)expl}` and
    // the value read back at the next prompt after a single TAB on a
    // `_physical_volumes` wrapper:
    //
    //   zsh  : [][]
    //   zshrs: [array][-J|-default-]
    //
    // kind 0, as sh:3 spells a bare `local`.
    crate::compsys::ported::shared::declare_locals(&["expl"], 0);
    // sh:5-6  _wanted physicalvolumes expl 'physical volume' \
    //           compadd "$@" - $(lsdev -C -c disk -S a -F name)
    let mut w = vec![
        "physicalvolumes".to_string(),
        "expl".to_string(),
        "physical volume".to_string(),
        "compadd".to_string(),
    ];
    w.extend(args.iter().cloned());
    w.push("-".to_string());
    w.extend(lsdev_physical_volumes());
    _wanted(&w)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_registered_tags() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_physical_volumes(&[]), 1);
    }
}

//! Compsys backend router — Rust ports vs upstream shell functions.
//!
//! **zshrs-original — no C source counterpart.** C zsh has only one
//! compsys backend: the shell-function tree at `Completion/`,
//! autoloaded via `fpath`. zshrs ships a parallel Rust-port tree
//! under `src/compsys/ported/` whose entry points are reachable
//! through this router.
//!
//! The user picks per-machine via `~/.config/zshrs/config.toml`:
//! ```toml
//! [compsys]
//! backend = "rust"   # default — route _NAME calls to compsys::ported
//! # backend = "shell"  # bypass — let standard shfunc dispatch win
//! ```
//!
//! `try_rust_dispatch(name, args)` returns `Some(rc)` when the Rust
//! backend handled the call; `None` to defer to the regular shfunc
//! path (the standard zsh-compatible chain).

use crate::extensions::config::{current as current_config, CompsysBackend};

/// Resolve a `_NAME` to its Rust port fn pointer, gated on the
/// `[compsys] backend = "rust"` config.
///
/// Returns `None` when:
/// - `backend = "shell"` (user opted out of Rust ports);
/// - the name doesn't start with `_`;
/// - or no Rust port is registered for the name (graceful per-name
///   degradation — partial coverage falls through to shell autoload).
///
/// The caller MUST run the returned fn pointer **inside doshfunc's
/// scope** (i.e. as the `body_runner` closure) so the prologue/
/// epilogue side effects (locallevel++, FUNCSTACK push, BREAKS/
/// CONTFLAG/LASTVAL save+restore, trap_state, noerrexit clear,
/// pipestats deep-copy) all apply to the Rust port the same way C
/// applies them to a wordcode shfunc body. Skipping doshfunc would
/// leak per-call scope state into the caller — a real bug, not just
/// a minor difference.
pub fn try_rust_dispatch(name: &str) -> Option<fn(&[String]) -> i32> {
    if current_config().compsys.backend != CompsysBackend::Rust {
        return None;
    }
    if !name.starts_with('_') {
        return None;
    }
    rust_compsys_lookup(name)
}

/// Per-name dispatch table — maps `_NAME` to a `fn(&[String]) -> i32`
/// pointer to the Rust port entry. New ports register here one line
/// at a time as their wired sigs land.
fn rust_compsys_lookup(name: &str) -> Option<fn(&[String]) -> i32> {
    use crate::compsys::ported::*;
    match name {
        "_main_complete" => Some(_main_complete::_main_complete),
        "_setup" => Some(_setup::_setup),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_underscore_names() {
        assert!(rust_compsys_lookup("foo").is_none());
        assert!(rust_compsys_lookup("complete-word").is_none());
    }

    #[test]
    fn returns_fn_pointer_for_registered_names() {
        assert!(rust_compsys_lookup("_main_complete").is_some());
        assert!(rust_compsys_lookup("_setup").is_some());
    }

    #[test]
    fn returns_none_for_unregistered_names() {
        // `_nosuch_xyz` has no Rust port — caller falls back to
        // shfunc autoload.
        assert!(rust_compsys_lookup("_nosuch_xyz").is_none());
    }
}

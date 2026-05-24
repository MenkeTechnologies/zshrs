//! Port of `_comp_caller_options` — get shell options from calling
//! context.
//!
//! Local shell reference: this name appears in compsys completers
//! (e.g. `_complete`) as a snapshot of the `$options` associative
//! array captured at compsys-entry. There's no standalone shell
//! file — `_comp_caller_options` is a shell PARAMETER initialised
//! by `_main_complete` via:
//! ```text
//!   typeset -gA _comp_caller_options
//!   _comp_caller_options=( "${(@kv)options}" )
//! ```
//!
//! Then completer functions consult `${_comp_caller_options[X]}`
//! to find out what option X was set to OUTSIDE compsys (since
//! compsys itself toggles many options to its preferred values).
//!
//! Faithful Rust port: process-global registry (Mutex<HashMap>)
//! that callers populate via [`set`] / [`set_one`] when entering
//! compsys, then `_comp_caller_options()` returns the snapshot.
//! Mirrors the shell's `typeset -gA` semantic (one global table).

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

fn registry() -> &'static Mutex<HashMap<String, bool>> {
    static REG: OnceLock<Mutex<HashMap<String, bool>>> = OnceLock::new();
    REG.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Wholesale replace the caller-options snapshot. Used by
/// `_main_complete` (or the runtime equivalent) on every compsys
/// entry to record the current shell-side options.
pub fn set(snapshot: HashMap<String, bool>) {
    if let Ok(mut g) = registry().lock() {
        *g = snapshot;
    }
}

/// Set a single option in the snapshot. Useful for tests + for
/// runtime hot-updates when individual options change.
pub fn set_one(name: impl Into<String>, on: bool) {
    if let Ok(mut g) = registry().lock() {
        g.insert(name.into(), on);
    }
}

/// Look up a single option from the snapshot. Returns None when
/// the option wasn't captured (or no snapshot is active).
pub fn get(name: &str) -> Option<bool> {
    registry().lock().ok().and_then(|g| g.get(name).copied())
}

/// Clear the snapshot — used by tests + by the runtime when exiting
/// compsys to release the captured state.
pub fn clear() {
    if let Ok(mut g) = registry().lock() {
        g.clear();
    }
}

/// _comp_caller_options - return the captured shell-options snapshot.
///
/// Returns the full HashMap clone so callers can iterate without
/// holding the registry lock. Mirrors `${(kv)_comp_caller_options}`.
pub fn _comp_caller_options() -> HashMap<String, bool> {
    registry().lock().map(|g| g.clone()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lock() -> std::sync::MutexGuard<'static, ()> {
        static L: OnceLock<Mutex<()>> = OnceLock::new();
        L.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|p| p.into_inner())
    }

    #[test]
    fn empty_before_any_set() {
        let _g = lock();
        clear();
        assert!(_comp_caller_options().is_empty());
    }

    #[test]
    fn set_and_get_round_trip() {
        let _g = lock();
        clear();
        let mut snap = HashMap::new();
        snap.insert("EXTENDED_GLOB".into(), true);
        snap.insert("NULL_GLOB".into(), false);
        set(snap.clone());
        assert_eq!(_comp_caller_options(), snap);
        assert_eq!(get("EXTENDED_GLOB"), Some(true));
        assert_eq!(get("NULL_GLOB"), Some(false));
        assert_eq!(get("NOT_PRESENT"), None);
        clear();
    }

    #[test]
    fn set_one_modifies_in_place_without_clearing_others() {
        let _g = lock();
        clear();
        set_one("A", true);
        set_one("B", false);
        assert_eq!(get("A"), Some(true));
        assert_eq!(get("B"), Some(false));
        set_one("A", false);
        assert_eq!(get("A"), Some(false));
        assert_eq!(get("B"), Some(false), "B must be untouched");
        clear();
    }

    #[test]
    fn clear_wipes_snapshot() {
        let _g = lock();
        set_one("X", true);
        clear();
        assert!(_comp_caller_options().is_empty());
        assert_eq!(get("X"), None);
    }
}

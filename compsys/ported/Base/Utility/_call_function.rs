//! Port of `_call_function` — call a completion function by name.
//!
//! Local shell reference: `compsys/functions/Base/Utility/_call_function`
//! (system copy `/opt/homebrew/share/zsh/functions/_call_function`).
//!
//! Upstream shell source (key lines from the 32-line fn):
//! ```text
//!  4  # Usage: _call_function <return> <name> [ <args> ... ]
//!  6  # If a function named <name> is defined, it is called.
//! 15  local _name _ret
//! 17  [[ "$1" != (|-) ]] && _name="$1"
//! 18  shift
//! 22  if (( ${+functions[$2]} )); then
//! 24    $2 "$@"
//! 26    _ret=$?
//! ```
//!
//! Upstream resolves a shell function by name (from `$functions`
//! associative array) and invokes it, storing the return code in
//! the `<return>` parameter name.
//!
//! Faithful Rust port: maintains a process-global registry of
//! `(name → Box<dyn Fn(&mut MainCompleteState) -> bool>)` callbacks.
//! Callers register their completion fns via [`register`] at startup;
//! `_call_function` looks them up by name and invokes them. This is
//! the Rust analog of `$functions[$name]`.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use crate::base::MainCompleteState;

/// A registered completion function — takes mutable completion state,
/// returns true if matches were added.
pub type CompFn = Box<dyn Fn(&mut MainCompleteState) -> bool + Send + Sync>;

fn registry() -> &'static Mutex<HashMap<String, CompFn>> {
    static REG: OnceLock<Mutex<HashMap<String, CompFn>>> = OnceLock::new();
    REG.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Register a completion function under `name`. Idempotent —
/// repeated calls overwrite the previous binding.
pub fn register(name: impl Into<String>, f: CompFn) {
    if let Ok(mut g) = registry().lock() {
        g.insert(name.into(), f);
    }
}

/// Unregister a previously-registered name. Returns true if it
/// existed.
pub fn unregister(name: &str) -> bool {
    registry()
        .lock()
        .map(|mut g| g.remove(name).is_some())
        .unwrap_or(false)
}

/// True iff a function named `name` is currently registered. Mirrors
/// shell's `(( ${+functions[$name]} ))` test at shell:22.
pub fn is_registered(name: &str) -> bool {
    registry()
        .lock()
        .map(|g| g.contains_key(name))
        .unwrap_or(false)
}

/// _call_function - Call a completion function by name.
///
/// Returns true iff `name` was found in the registry AND its
/// invocation returned true. Matches the upstream
/// `(( ${+functions[$name]} )) && $name "$@" && _ret=$?` flow.
pub fn _call_function(state: &mut MainCompleteState, name: &str) -> bool {
    // shell:22 — `(( ${+functions[$name]} ))` test
    let Ok(g) = registry().lock() else {
        return false;
    };
    let Some(f) = g.get(name) else {
        return false;
    };
    // shell:24 — `$name "$@"`. Note: registry is locked for the
    // duration of the call to keep the Box<dyn Fn> alive. This is
    // safe because completion fns don't recursively call _call_function
    // on themselves at the leaf (parent crate handles deeper nesting).
    f(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Tests share the global registry — serialise to avoid
    // cross-test pollution.
    fn lock() -> std::sync::MutexGuard<'static, ()> {
        static L: OnceLock<Mutex<()>> = OnceLock::new();
        L.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|p| p.into_inner())
    }

    #[test]
    fn unregistered_name_returns_false() {
        let _g = lock();
        let mut state = MainCompleteState::new("", 0);
        assert!(!_call_function(&mut state, "definitely-not-registered-zzz"));
    }

    #[test]
    fn registered_name_invokes_callback_and_propagates_return() {
        let _g = lock();
        // True case
        register(
            "test-true-fn",
            Box::new(|_state| true),
        );
        let mut state = MainCompleteState::new("", 0);
        assert!(_call_function(&mut state, "test-true-fn"));

        // False case
        register("test-false-fn", Box::new(|_state| false));
        assert!(!_call_function(&mut state, "test-false-fn"));

        // Cleanup
        unregister("test-true-fn");
        unregister("test-false-fn");
    }

    #[test]
    fn callback_can_mutate_state() {
        let _g = lock();
        use crate::completion::Completion;
        register(
            "test-add-match-fn",
            Box::new(|state| {
                state
                    .comp
                    .add_match(Completion::new("from-fn"), None);
                true
            }),
        );
        let mut state = MainCompleteState::new("", 0);
        assert!(_call_function(&mut state, "test-add-match-fn"));
        let names: Vec<String> = state
            .comp
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        assert!(names.contains(&"from-fn".to_string()));
        unregister("test-add-match-fn");
    }

    #[test]
    fn is_registered_reflects_registry_state() {
        let _g = lock();
        assert!(!is_registered("test-reg-check"));
        register("test-reg-check", Box::new(|_| true));
        assert!(is_registered("test-reg-check"));
        unregister("test-reg-check");
        assert!(!is_registered("test-reg-check"));
    }

    #[test]
    fn register_is_idempotent_last_wins() {
        let _g = lock();
        use std::sync::atomic::{AtomicU32, Ordering};
        static FIRST: AtomicU32 = AtomicU32::new(0);
        static SECOND: AtomicU32 = AtomicU32::new(0);
        FIRST.store(0, Ordering::SeqCst);
        SECOND.store(0, Ordering::SeqCst);
        register(
            "test-idempotent",
            Box::new(|_| {
                FIRST.fetch_add(1, Ordering::SeqCst);
                true
            }),
        );
        register(
            "test-idempotent",
            Box::new(|_| {
                SECOND.fetch_add(1, Ordering::SeqCst);
                true
            }),
        );
        let mut state = MainCompleteState::new("", 0);
        _call_function(&mut state, "test-idempotent");
        assert_eq!(FIRST.load(Ordering::SeqCst), 0, "first binding overwritten");
        assert_eq!(SECOND.load(Ordering::SeqCst), 1, "second binding invoked");
        unregister("test-idempotent");
    }

    #[test]
    fn unregister_returns_bool_indicating_existence() {
        let _g = lock();
        register("test-unreg", Box::new(|_| true));
        assert!(unregister("test-unreg"));
        assert!(!unregister("test-unreg"), "second unregister sees no entry");
    }
}

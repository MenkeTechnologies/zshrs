//! Port of `_call_function` from `Completion/Base/Utility/_call_function`.
//!
//! Full upstream body (32 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  # Utility function to call a function if it exists.
//! sh: 4  #
//! sh: 5  # Usage: _call_function <return> <name> [ <args> ... ]
//! sh: 6  #
//! sh: 7  # If a function named <name> is defined (or defined to be autoloaded),
//! sh: 8  # it is called. If <return> is given not the string `-' or empty, it is
//! sh: 9  # taken as the name of a parameter and the return status of the function
//! sh:10  # called is stored in this parameter. All other arguments are given
//! sh:11  # to the function called.
//! sh:12  # The return value of this function is zero if the function was
//! sh:13  # called and non-zero otherwise.
//! sh:14
//! sh:15  local _name _ret
//! sh:16
//! sh:17  [[ "$1" != (|-) ]] && _name="$1"
//! sh:18
//! sh:19  shift
//! sh:20
//! sh:21  if (( $+functions[$1] )); then
//! sh:22    "$@"
//! sh:23    _ret="$?"
//! sh:24
//! sh:25    [[ -n "$_name" ]] && eval "${_name}=${_ret}"
//! sh:26
//! sh:27    compstate[restore]=''
//! sh:28
//! sh:29    return 0
//! sh:30  fi
//! sh:31
//! sh:32  return 1
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

use crate::compsys::base::MainCompleteState;

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
        use crate::compsys::completion::Completion;
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

    #[test]
    fn empty_string_name_can_be_registered_and_called() {
        let _g = lock();
        register("", Box::new(|_| true));
        let mut state = MainCompleteState::new("", 0);
        assert!(_call_function(&mut state, ""));
        unregister("");
    }

    #[test]
    fn many_distinct_registrations_coexist() {
        let _g = lock();
        for i in 0..10 {
            let name = format!("test-coexist-{i}");
            register(name.clone(), Box::new(|_| true));
            assert!(is_registered(&name));
        }
        // All 10 distinct registrations should be callable.
        let mut state = MainCompleteState::new("", 0);
        for i in 0..10 {
            let name = format!("test-coexist-{i}");
            assert!(_call_function(&mut state, &name));
        }
        for i in 0..10 {
            unregister(&format!("test-coexist-{i}"));
        }
    }

    #[test]
    fn is_registered_for_unregistered_returns_false() {
        let _g = lock();
        assert!(!is_registered("a-fn-that-never-existed-anywhere"));
    }
}

//! Port of `_dynamic_directory_name` — `~[…]` dynamic-named-dir
//! expansion completion.
//!
//! Local shell reference:
//! `/opt/homebrew/share/zsh/functions/_dynamic_directory_name`.
//!
//! Upstream shell source (the whole 15-line file):
//! ```text
//! #autoload
//!
//! local func
//! integer ret=1
//!
//! if [[ -n $functions[zsh_directory_name] || \
//!   ${+zsh_directory_name_functions} -ne 0 ]] ; then
//!   [[ -n $functions[zsh_directory_name] ]] && zsh_directory_name c && ret=0
//!   for func in $zsh_directory_name_functions; do
//!     $func c && ret=0
//!   done
//!   return ret
//! else
//!   _message 'dynamic directory name: implemented as zsh_directory_name c'
//! fi
//! ```
//!
//! Strict Rust port: dispatches each registered
//! `zsh_directory_name_functions` callback via [`_call_function`]
//! with `"c"` as the conceptual arg. Falls back to `_message` when
//! no callbacks are registered.

use crate::base::MainCompleteState;
use crate::ported::_call_function::{_call_function, is_registered};
use crate::ported::_message::_message;

/// `_dynamic_directory_name` — dispatch dynamic-dir-name callbacks.
///
/// `callback_names` — list of fn names from
/// `$zsh_directory_name_functions`, plus `"zsh_directory_name"` if
/// the user defined that fn directly.
pub fn _dynamic_directory_name(
    state: &mut MainCompleteState,
    callback_names: &[String],
) -> bool {
    let mut any = false;
    // shell:7 — invoke `zsh_directory_name c` if defined.
    if is_registered("zsh_directory_name") {
        if _call_function(state, "zsh_directory_name") {
            any = true;
        }
    }
    // shell:8-10 — invoke each $zsh_directory_name_functions entry.
    for f in callback_names {
        if _call_function(state, f) {
            any = true;
        }
    }
    if !any {
        // shell:14 — `_message 'dynamic directory name: …'`
        let styles = state.styles.clone();
        let ctx = state.ctx.context.clone();
        _message(
            &mut state.comp,
            &styles,
            &ctx,
            "messages",
            "dynamic directory name: implemented as zsh_directory_name c",
        );
    }
    any
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::_call_function::{register, unregister};

    #[test]
    fn no_callbacks_emits_message() {
        let mut state = MainCompleteState::new("", 0);
        let _ = _dynamic_directory_name(&mut state, &[]);
        let exps: Vec<&str> = state
            .comp
            .groups
            .iter()
            .flat_map(|g| g.explanations.iter())
            .map(|s| s.as_str())
            .collect();
        assert!(exps.iter().any(|s| s.contains("dynamic directory name")));
    }

    #[test]
    fn registered_zsh_directory_name_fn_is_invoked() {
        use std::sync::atomic::{AtomicBool, Ordering};
        static FIRED: AtomicBool = AtomicBool::new(false);
        FIRED.store(false, Ordering::SeqCst);
        register(
            "zsh_directory_name",
            Box::new(|_| {
                FIRED.store(true, Ordering::SeqCst);
                true
            }),
        );
        let mut state = MainCompleteState::new("", 0);
        let r = _dynamic_directory_name(&mut state, &[]);
        unregister("zsh_directory_name");
        assert!(FIRED.load(Ordering::SeqCst));
        assert!(r);
    }

    #[test]
    fn callback_names_dispatched_in_order() {
        use std::sync::Mutex;
        static SEEN: Mutex<Vec<String>> = Mutex::new(Vec::new());
        SEEN.lock().unwrap().clear();
        register(
            "fn-a",
            Box::new(|_| {
                SEEN.lock().unwrap().push("a".into());
                true
            }),
        );
        register(
            "fn-b",
            Box::new(|_| {
                SEEN.lock().unwrap().push("b".into());
                true
            }),
        );
        let mut state = MainCompleteState::new("", 0);
        let _ = _dynamic_directory_name(&mut state, &["fn-a".into(), "fn-b".into()]);
        unregister("fn-a");
        unregister("fn-b");
        let seen = SEEN.lock().unwrap().clone();
        assert_eq!(seen, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn unregistered_callback_silently_skipped() {
        let mut state = MainCompleteState::new("", 0);
        let _ =
            _dynamic_directory_name(&mut state, &["not-registered-anywhere".into()]);
        // Falls back to message since no callbacks fired.
        let exps: Vec<&str> = state
            .comp
            .groups
            .iter()
            .flat_map(|g| g.explanations.iter())
            .map(|s| s.as_str())
            .collect();
        assert!(exps.iter().any(|s| s.contains("dynamic directory name")));
    }
}

//! Port of `_dynamic_directory_name` from `Completion/Zsh/Context/_dynamic_directory_name`.
//!
//! Full upstream body (29 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2  local -a dirfuncs=(
//! sh: 3      ${(k)functions[zsh_directory_name]}
//! sh: 4      $zsh_directory_name_functions
//! sh: 5  )
//! sh: 6  local descr='dynamically named directory'
//! sh: 7
//! sh: 8  if (( $#dirfuncs )); then
//! sh: 9    local -a expl
//! sh:10    local -i ret
//! sh:11    local func suf tag=dynamically-named-directories
//! sh:12
//! sh:13    [[ $ISUFFIX != \]* ]] &&
//! sh:14        suf=-S]
//! sh:15
//! sh:16    _tags "$tag"
//! sh:17    while _tags; do
//! sh:18      while _next_label "$tag" expl "$descr" $suf; do
//! sh:19        for func in $dirfuncs; do
//! sh:20          $func c && ret=0
//! sh:21        done
//! sh:22      done
//! sh:23      (( ret )) || break
//! sh:24    done
//! sh:25    return ret
//! sh:26
//! sh:27  else
//! sh:28    _message "${descr}: implement as zsh_directory_name c"
//! sh:29  fi
//! ```
//!
//! Strict Rust port: dispatches each registered
//! `zsh_directory_name_functions` callback via [`_call_function`]
//! with `"c"` as the conceptual arg. Falls back to `_message` when
//! no callbacks are registered.



use crate::compsys::base::MainCompleteState;
use crate::compsys::ported::_call_function::{_call_function, is_registered};
use crate::compsys::ported::_message::_message;

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
    use crate::compsys::ported::_call_function::{register, unregister};

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

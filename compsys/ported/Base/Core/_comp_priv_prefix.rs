//! Port of `_comp_priv_prefix` — privilege escalation prefix
//! (sudo / doas / su, etc.).
//!
//! Local shell reference: there is NO standalone shell file —
//! `_comp_priv_prefix` is a shell ARRAY parameter (not a function).
//! Set by precommand-aware completers like `_sudo`, `_doas`, `_su`
//! before they dispatch the underlying command's completer; consumed
//! by `_call_program -p` to decide whether to re-prefix the probe
//! command.
//!
//! Shell-side pattern:
//! ```text
//! _sudo() {
//!   …
//!   local +h _comp_priv_prefix=( sudo )
//!   _normal -p sudo
//! }
//! ```
//!
//! The `+h` flag makes it locally-scoped (hidden from outer scope);
//! when `_normal` runs the sub-completer, it sees `_comp_priv_prefix`
//! through scope but the array is auto-cleaned when `_sudo` returns.
//!
//! Faithful Rust port: process-global Vec<String> with `push_scope`
//! / `pop_scope` helpers that mirror the shell's locally-scoped
//! `+h` semantic. `_comp_priv_prefix()` returns the current value
//! (whatever the innermost active scope set).

use std::sync::{Mutex, OnceLock};

fn stack() -> &'static Mutex<Vec<Vec<String>>> {
    static S: OnceLock<Mutex<Vec<Vec<String>>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(Vec::new()))
}

/// Push a new priv-prefix scope. Returned guard pops on drop —
/// mirrors shell's `local +h _comp_priv_prefix=( ... )` scope.
///
/// Usage:
/// ```ignore
/// let _g = push_scope(vec!["sudo".into()]);
/// // … _comp_priv_prefix() now returns ["sudo"] for the rest of
/// // this block; the prior value (if any) is restored on drop.
/// ```
pub fn push_scope(prefix: Vec<String>) -> PrivPrefixGuard {
    if let Ok(mut s) = stack().lock() {
        s.push(prefix);
    }
    PrivPrefixGuard { _private: () }
}

/// RAII guard returned by [`push_scope`]. Pops the scope on drop.
pub struct PrivPrefixGuard {
    _private: (),
}

impl Drop for PrivPrefixGuard {
    fn drop(&mut self) {
        if let Ok(mut s) = stack().lock() {
            s.pop();
        }
    }
}

/// _comp_priv_prefix - return the active priv prefix (innermost
/// scope wins). Empty Vec when no scope is active.
pub fn _comp_priv_prefix() -> Vec<String> {
    stack()
        .lock()
        .ok()
        .and_then(|s| s.last().cloned())
        .unwrap_or_default()
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
    fn empty_outside_any_scope() {
        let _g = lock();
        assert!(_comp_priv_prefix().is_empty());
    }

    #[test]
    fn scope_visible_during_lifetime() {
        let _g = lock();
        {
            let _guard = push_scope(vec!["sudo".into()]);
            assert_eq!(_comp_priv_prefix(), vec!["sudo".to_string()]);
        }
        assert!(_comp_priv_prefix().is_empty(), "scope popped on drop");
    }

    #[test]
    fn nested_scopes_innermost_wins_and_restores() {
        let _g = lock();
        let _outer = push_scope(vec!["sudo".into()]);
        assert_eq!(_comp_priv_prefix(), vec!["sudo".to_string()]);
        {
            let _inner = push_scope(vec!["doas".into(), "-u".into(), "root".into()]);
            assert_eq!(
                _comp_priv_prefix(),
                vec!["doas".to_string(), "-u".into(), "root".into()]
            );
        }
        // Inner popped → outer visible again.
        assert_eq!(_comp_priv_prefix(), vec!["sudo".to_string()]);
    }

    #[test]
    fn drop_order_is_lifo() {
        let _g = lock();
        let a = push_scope(vec!["A".into()]);
        let b = push_scope(vec!["B".into()]);
        let c = push_scope(vec!["C".into()]);
        assert_eq!(_comp_priv_prefix(), vec!["C".to_string()]);
        drop(c);
        assert_eq!(_comp_priv_prefix(), vec!["B".to_string()]);
        drop(b);
        assert_eq!(_comp_priv_prefix(), vec!["A".to_string()]);
        drop(a);
        assert!(_comp_priv_prefix().is_empty());
    }

    #[test]
    fn empty_prefix_in_scope_is_distinct_from_no_scope() {
        let _g = lock();
        // Outside scope, empty.
        assert!(_comp_priv_prefix().is_empty());
        {
            // Scope with empty Vec — still "in scope", just empty.
            let _guard = push_scope(vec![]);
            assert!(_comp_priv_prefix().is_empty());
        }
    }
}

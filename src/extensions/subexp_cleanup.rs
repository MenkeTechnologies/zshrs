//! Rust-only cleanup guard for the transient `__subexp_arr_*` array
//! temporaries that `paramsubst` (src/ported/subst.rs) stashes in the
//! global paramtab while expanding an array sub-expression
//! (`${(s: :)$(cmd)}`, `${${(s: :)v}[N]}`, …).
//!
//! In zsh C the sub-expression array lives in the per-paramsubst
//! `Value` struct (`v.isarr = 1; aval = …`) and never touches the
//! parameter hash, so it is invisible to `typeset -p`. zshrs reuses
//! the canonical paramtab as scratch so the existing splat / subscript
//! / filter code can resolve the temp through the normal var-lookup
//! paths — but without cleanup that leaks `__subexp_arr_N` into
//! user-visible state (it shows up in `typeset -p`, in `${(k)parameters}`,
//! etc.). This guard evicts every temp its owning `paramsubst` call
//! created when that call returns, on ALL paths — including the many
//! early error returns and the recursive nested-`${…}` returns —
//! restoring the C-visible behaviour that nothing leaks.
//!
//! This is deliberately NOT a `src/ported` helper: there is no zsh C
//! function it corresponds to. It is pure zshrs bookkeeping that exists
//! only because the port reuses the global paramtab as scratch, so it
//! lives under `src/extensions` per the project's Rust-only-code rule.

/// Tracks the `__subexp_arr_*` temp array names created during a single
/// `paramsubst` invocation and evicts them from the global paramtab on
/// `Drop` (i.e. when the invocation returns, on every code path).
pub struct SubexpTempGuard {
    names: Vec<String>,
}

impl SubexpTempGuard {
    #[inline]
    pub fn new() -> Self {
        Self { names: Vec::new() }
    }

    /// Register a temp array name for eviction when this guard drops.
    #[inline]
    pub fn track(&mut self, name: String) {
        self.names.push(name);
    }
}

impl Default for SubexpTempGuard {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for SubexpTempGuard {
    fn drop(&mut self) {
        if self.names.is_empty() {
            return;
        }
        // Direct paramtab eviction (not `unsetparam`): these temps are
        // plain `PM_ARRAY` nodes with no gsu / special-unset hooks, so a
        // straight `remove` is both correct and free of side effects.
        if let Ok(mut tab) = crate::ported::params::paramtab().write() {
            for n in &self.names {
                tab.remove(n);
            }
        }
    }
}

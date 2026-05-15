//! Ports of zsh's built-in builtins from `Src/builtin.c` (and the
//! sub-builtin code that lives elsewhere in `Src/`). Distinct from
//! `crate::modules`, which mirrors `Src/Modules/*.c` (the *loadable*
//! modules).
//!
//! Builtins land here when their C source is in `Src/builtin.c`,
//! `Src/exec.c`, etc. — i.e. things zsh has compiled into the core
//! shell, not loaded via `zmodload`.

pub mod rlimits;
pub mod sched;

#[cfg(test)]
mod tests {
    /// `ulimit` with no args lists current limits per POSIX. The
    /// canonical `bin_ulimit` exits with 0 on the listing path. Tests
    /// the most-used invocation (`$ ulimit`) doesn't error and doesn't
    /// panic — both regressions zsh users would hit immediately.
    #[test]
    fn ulimit_with_no_args_succeeds() {
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(), argscount: 0, argsalloc: 0,
        };
        let r = super::rlimits::bin_ulimit("ulimit", &[], &ops, 0);
        assert_eq!(r, 0, "`ulimit` with no args must succeed (POSIX listing path)");
    }
}

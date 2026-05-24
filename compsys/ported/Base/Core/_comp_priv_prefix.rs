//! Port of `_comp_priv_prefix` — prefix for privilege escalation
//! (sudo, doas, etc.).
//!
//! Local shell reference: there is NO standalone shell file —
//! `_comp_priv_prefix` is a shell ARRAY parameter (not a function).
//! Set by precommand-aware completers like `_sudo`, `_doas`, `_su`
//! before they dispatch the underlying command's completer; consumed
//! by `_call_program -p` to decide whether to re-prefix the probe
//! command.
//!
//! Upstream `_comp_priv_prefix` is a shell ARRAY parameter (not a
//! function file) holding the precommand stack — e.g. `( sudo )` or
//! `( doas -u root )`. Set by precommand-aware completers like
//! `_sudo`, `_doas`, `_su` before they dispatch the underlying
//! command's completer; consumed by `_call_program -p` to decide
//! whether to re-prefix the probe command.
//!
//! Rust port: returns an empty `Vec<String>` at the leaf. The actual
//! priv-prefix state belongs at the parent crate (zshrs runtime)
//! and is plumbed into `_call_program` via
//! `CallProgramOpts::priv_prefix` — see
//! `compsys/ported/Base/Utility/_call_program.rs`. Returning empty
//! here matches the contract "no priv prefix active by default".

/// _comp_priv_prefix - Prefix for privilege escalation (sudo, doas, etc.)
pub fn _comp_priv_prefix() -> Vec<String> {
    // Returns the privilege prefix if any
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_priv_prefix_until_caller_populates() {
        // The shell-side equivalent is set by `sudo`/`doas` completion
        // wrappers; the leaf default is the empty vector. Pin so any
        // future drift surfaces as a test diff.
        assert!(_comp_priv_prefix().is_empty());
    }
}

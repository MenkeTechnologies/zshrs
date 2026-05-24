//! Port of `_comp_locale` — set locale for completion. Moved from
//! `compsys/functions.rs`. Renamed from `comp_locale` to mirror zsh
//! shell function name `_comp_locale`.

/// _comp_locale - Set locale for completion
pub fn _comp_locale() {
    // Would set LC_ALL=C or similar
    // In Rust, this is handled differently — _call_program threads
    // the C-locale env vars directly via CallProgramOpts::skip_locale.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_op_does_not_panic() {
        // _comp_locale's shell behavior (set LC_ALL=C and friends in
        // the caller env) is handled at the _call_program layer in
        // the Rust port. The standalone fn intentionally does nothing
        // — pin that it stays a no-op (no env mutation, no panic).
        let before_lc = std::env::var("LC_ALL").ok();
        _comp_locale();
        let after_lc = std::env::var("LC_ALL").ok();
        assert_eq!(before_lc, after_lc, "_comp_locale must not mutate process env");
    }
}

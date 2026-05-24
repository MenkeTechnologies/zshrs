//! Port of `_comp_locale` — set locale for completion.
//!
//! Local shell reference: `compsys/functions/Base/Utility/_comp_locale`
//! (system copy `/opt/homebrew/share/zsh/functions/_comp_locale`).
//!
//! Upstream shell source (the whole 20-line fn):
//! ```text
//! 11  if ctype=${${(f)"$(locale 2>/dev/null)"}:#^LC_CTYPE=*}; then
//! 12      unset -m LC_\*
//! 13      [[ -n $ctype ]] && eval export $ctype
//! 14  else
//! 15      ctype=${LC_ALL:-${LC_CTYPE:-${LANG:-C}}}
//! 16      unset -m LC_\*
//! 17      export LC_CTYPE=$ctype
//! 18  fi
//! 19  export LANG=C
//! ```
//!
//! Upstream sets a C-locale env for sane completion-tool output
//! while keeping LC_CTYPE for filename byte interpretation. The
//! comment says it MUST be run in a subshell (changes calling
//! shell's env).
//!
//! Rust port: deliberate no-op. Our equivalent lives in
//! `compsys::ported::_call_program` via `CallProgramOpts::skip_locale`
//! — the locale env vars (LC_ALL=C, LANG=C, LC_MESSAGES=C) are set
//! per-exec on the Command, not via process-global mutation. Pin
//! that no env is touched.

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

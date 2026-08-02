//! Port of `_first` from `Completion/Zsh/Context/_first`.
//!
//! Full upstream body (47 lines, all comments — the function is
//! deliberately empty so users can override it as a hook):
//! ```text
//! sh: 1  #compdef -first-
//! sh: 2
//! sh: 3  # This function is called at the very beginning before any other
//! sh: 4  # function for a specific context.
//! sh: 5  #
//! sh: 6  # This just gives some examples of things you might want to do here.
//! sh: 7  # …
//! sh:47  #     fi
//! ```
//!
//! The shell function ships as 47 lines of `#`-prefixed example code
//! showing what users CAN put in their own override of `_first`. The
//! upstream function itself executes nothing — it's a no-op hook
//! invoked at the start of every completion context.
//!
//! Faithful Rust port: a no-op returning 0 (success — the shell's
//! default exit when an empty function runs).

/// `_first` — `-first-` context hook. No-op by default; users override
/// for per-context pre-completion behavior.
pub fn _first() -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_first");
    // sh:1-47 — every line is a `#` comment; nothing executes.
    // Return 1 (NOT the empty-shell-function 0): `_complete` runs
    // `eval "$_comps[-first-]" && ret=0` (Completion/Base/Completer/_complete
    // sh:100). A 0 here sets ret=0 with ZERO matches added, so `_complete`
    // reports success and `_main_complete`'s completer loop breaks after it —
    // the chain NEVER advances to `_approximate` / `_ignored` / `_correct` /
    // `_prefix` / `_match`. The no-op hook added nothing, so per the completer
    // contract it must report "not handled" (non-zero). A user override that
    // actually adds matches uses an explicit `return 0` (see the sh:39 example),
    // which is unaffected.
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_nonzero_so_completer_chain_advances() {
        // The no-op `-first-` hook adds no matches, so it must report
        // "not handled" (non-zero); a 0 would make `_complete` set ret=0
        // and `_main_complete` stop before `_approximate`/`_ignored`/etc.
        assert_eq!(_first(), 1);
    }
}

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
    // sh:1-47 — every line is a `#` comment; nothing executes, so the
    // function returns 0 like any empty shell function. `_complete`'s
    // `eval "$_comps[-first-]" && ret=0` (sh:100) does not make that a
    // successful completion: sh:114 resets `ret=1` before the context
    // dispatch, and only `_compskip=all` (sh:101-103) returns the -first- ret.
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_hook_returns_zero_like_the_shell_function() {
        // sh:1-47 are all comments; an empty function's status is 0.
        // `_complete` sh:114 resets `ret=1` afterwards, so the chain still
        // advances.
        assert_eq!(_first(), 0);
    }
}

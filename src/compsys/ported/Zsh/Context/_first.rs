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
    // sh:1-47 — every line is a `#` comment; nothing executes.
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_zero() {
        // The shell function has no body to evaluate; exit code 0
        // (success) is the shell default for an all-comments function.
        assert_eq!(_first(), 0);
    }
}

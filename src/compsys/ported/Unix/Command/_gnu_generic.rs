//! Port of `_gnu_generic` from `Completion/Unix/Command/_gnu_generic`.
//!
//! Full upstream body (6 lines verbatim):
//! ```text
//! sh:1  #autoload
//! sh:2
//! sh:3  # This is for GNU-like commands which understand the --help option,
//! sh:4  # but which do not otherwise require special completion handling.
//! sh:5
//! sh:6  _arguments '*:arg: _default' --
//! ```

use crate::ported::exec::dispatch_function_call;

/// `_gnu_generic` — fallback completer for GNU-style commands.
/// Pure delegation to `_arguments` (not yet ported as an engine fn).
pub fn _gnu_generic() -> i32 {
    dispatch_function_call(
        "_arguments",
        &["*:arg: _default".to_string(), "--".to_string()],
    )
    .unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_executor() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_gnu_generic(), 1);
    }
}

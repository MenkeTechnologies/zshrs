//! Port of `_jobs_fg` from `Completion/Zsh/Type/_jobs_fg`.
//!
//! Full upstream body (3 lines verbatim):
//! ```text
//! sh:1  #compdef disown fg
//! sh:2
//! sh:3  _jobs "$@"
//! ```

use crate::ported::exec::dispatch_function_call;

/// `_jobs_fg` — `fg` / `disown` completion: all jobs via plain
/// `_jobs`. Exit code = `_jobs` exit (1 when uncallable).
pub fn _jobs_fg(args: &[String]) -> i32 {
    dispatch_function_call("_jobs", args).unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_executor() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_jobs_fg(&[]), 1);
    }
}

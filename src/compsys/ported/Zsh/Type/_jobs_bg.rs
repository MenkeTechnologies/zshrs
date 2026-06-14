//! Port of `_jobs_bg` from `Completion/Zsh/Type/_jobs_bg`.
//!
//! Full upstream body (3 lines verbatim):
//! ```text
//! sh:1  #compdef bg
//! sh:2
//! sh:3  _jobs -s "$@"
//! ```
//!
//! Pure delegation to `_jobs` (a shell function not in the engine
//! cluster). Routes through `crate::ported::exec::dispatch_function_call`.

use crate::ported::exec::dispatch_function_call;

/// `_jobs_bg` — `bg` command completion: stopped jobs only via
/// `_jobs -s`. Exit code = `_jobs` exit (1 when uncallable).
pub fn _jobs_bg(args: &[String]) -> i32 {
    let mut argv: Vec<String> = vec!["-s".to_string()];
    argv.extend(args.iter().cloned());
    dispatch_function_call("_jobs", &argv).unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_executor() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_jobs_bg(&[]), 1);
    }
}

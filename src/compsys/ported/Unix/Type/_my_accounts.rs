//! Port of `_my_accounts` from `Completion/Unix/Type/_my_accounts`.
//!
//! Full upstream body (3 lines verbatim):
//! ```text
//! sh:1  #autoload
//! sh:2
//! sh:3  _user_at_host -t my-accounts "$@"
//! ```

use crate::ported::exec::dispatch_function_call;

/// `_my_accounts` — complete user@host pairs from the `my-accounts` tag.
pub fn _my_accounts(args: &[String]) -> i32 {
    // sh:3  _user_at_host -t my-accounts "$@"
    let mut a: Vec<String> = vec!["-t".to_string(), "my-accounts".to_string()];
    a.extend(args.iter().cloned());
    dispatch_function_call("_user_at_host", &a).unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_executor() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_my_accounts(&[]), 1);
    }
}

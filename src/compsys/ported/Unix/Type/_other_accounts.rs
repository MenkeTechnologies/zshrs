//! Port of `_other_accounts` from `Completion/Unix/Type/_other_accounts`.
//!
//! Full upstream body (3 lines verbatim):
//! ```text
//! sh:1  #compdef talk ntalk ytalk
//! sh:2
//! sh:3  _user_at_host -t other-accounts "$@"
//! ```

use crate::ported::exec::dispatch_function_call;

/// `_other_accounts` — complete user@host pairs from the `other-accounts` tag.
pub fn _other_accounts(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_other_accounts");
    // sh:3  _user_at_host -t other-accounts "$@"
    let mut a: Vec<String> = vec!["-t".to_string(), "other-accounts".to_string()];
    a.extend(args.iter().cloned());
    dispatch_function_call("_user_at_host", &a).unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_executor() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_other_accounts(&[]), 1);
    }
}

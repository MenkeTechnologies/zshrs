//! Port of `_users` from `Completion/Unix/Type/_users`.
//!
//! Full upstream body (10 lines verbatim):
//! ```text
//! sh: 1  #compdef passwd groups userdel chage chfn
//! sh: 2
//! sh: 3  local expl users
//! sh: 4
//! sh: 5  if zstyle -a ":completion:${curcontext}:users" users users; then
//! sh: 6    _wanted users expl user compadd "$@" -a - users
//! sh: 7    return
//! sh: 8  fi
//! sh: 9
//! sh:10  _wanted users expl user compadd "$@" -k - userdirs
//! ```

use crate::compsys::ported::_wanted::wanted_byname;
use crate::ported::modules::zutil::lookupstyle;
use crate::ported::params::{getsparam, setaparam};

/// `_users` — complete user names, from the `users` style if set, else
/// from the keys of the `userdirs` association.
pub fn _users(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_users");
    let curcontext = getsparam("curcontext").unwrap_or_default();
    let ctx = format!(":completion:{}:users", curcontext);

    // sh:5 — zstyle -a … users users
    let styled = lookupstyle(&ctx, "users");
    if !styled.is_empty() {
        // sh:6  _wanted users expl user compadd "$@" -a - users
        setaparam("users", styled);
        let mut w: Vec<String> = vec![
            "users".to_string(),
            "expl".to_string(),
            "user".to_string(),
            "compadd".to_string(),
        ];
        w.extend(args.iter().cloned());
        w.push("-a".to_string());
        w.push("-".to_string());
        w.push("users".to_string());
        return wanted_byname(&w);
    }

    // sh:10  _wanted users expl user compadd "$@" -k - userdirs
    let mut w: Vec<String> = vec![
        "users".to_string(),
        "expl".to_string(),
        "user".to_string(),
        "compadd".to_string(),
    ];
    w.extend(args.iter().cloned());
    w.push("-k".to_string());
    w.push("-".to_string());
    w.push("userdirs".to_string());
    wanted_byname(&w)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::zle::complete::INCOMPFUNC;
    use std::sync::atomic::Ordering;

    #[test]
    fn returns_one_without_registered_tags() {
        let _g = crate::test_util::global_state_lock();
        INCOMPFUNC.store(1, Ordering::Relaxed);
        let r = _users(&[]);
        INCOMPFUNC.store(0, Ordering::Relaxed);
        assert_eq!(r, 1);
    }
}

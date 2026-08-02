//! Port of `_pgids` from `Completion/Unix/Type/_pgids`.
//!
//! Full upstream body (5 lines verbatim):
//! ```text
//! sh:1  #autoload
//! sh:2
//! sh:3  local expl
//! sh:4
//! sh:5  _wanted pgids expl 'process group ID' compadd "$@" - \
//! sh:5      ${(un)$(_call_program pgids ps -A -o pgid=)}
//! ```

use crate::compsys::ported::_call_program::_call_program;
use crate::compsys::ported::_wanted::_wanted;
use crate::ported::params::getsparam;

/// `_pgids` — complete process-group IDs (`ps -A -o pgid=`).
pub fn _pgids(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_pgids");
    // sh:5  $(_call_program pgids ps -A -o pgid=)
    let _ = _call_program(&[
        "pgids".to_string(),
        "ps".to_string(),
        "-A".to_string(),
        "-o".to_string(),
        "pgid=".to_string(),
    ]);
    let out = getsparam("REPLY").unwrap_or_default();
    // sh:5 approx — `${(un)…}`: numeric sort + unique of the split words.
    let mut ids: Vec<String> = out.split_whitespace().map(str::to_string).collect();
    ids.sort_by_key(|s| s.parse::<i64>().unwrap_or(i64::MAX));
    ids.dedup();
    // sh:5  _wanted pgids expl 'process group ID' compadd "$@" - <ids>
    let mut w = vec![
        "pgids".to_string(),
        "expl".to_string(),
        "process group ID".to_string(),
        "compadd".to_string(),
    ];
    w.extend(args.iter().cloned());
    w.push("-".to_string());
    w.extend(ids);
    _wanted(&w)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_registered_tags() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_pgids(&[]), 1);
    }
}

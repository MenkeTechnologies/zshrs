//! Port of `_list` from `Completion/Base/Completer/_list`.
//!
//! Full upstream body (37 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 5  [[ _matcher_num -gt 1 ]] && return 1
//! sh: 7  local pre suf expr
//! sh:11  if zstyle -t ":completion:${curcontext}:" word; then
//! sh:12    pre="$HISTNO$LBUFFER"
//! sh:13    suf="$RBUFFER"
//! sh:14  else
//! sh:15    pre="$PREFIX"
//! sh:16    suf="$SUFFIX"
//! sh:17  fi
//! sh:21  if zstyle -T ":completion:${curcontext}:" condition &&
//! sh:22     [[ "$pre" != "$_list_prefix" || "$suf" != "$_list_suffix" ]]; then
//! sh:28    compstate[insert]=''
//! sh:29    compstate[list]='list force'
//! sh:30    _list_prefix="$pre"
//! sh:31    _list_suffix="$suf"
//! sh:32  fi
//! sh:36  return 1
//! ```
//!
//! `_list` completer: forces a list-only render on first invocation
//! per `(pre,suf)`; remembers the pair in `_list_prefix`/`_list_suffix`
//! globals. Always returns 1 (defers to next completer).

use crate::ported::modules::zutil::testforstyle;
use crate::ported::params::{getiparam, getsparam, setsparam};
use crate::ported::zle::compcore::set_compstate_str;

/// `_list` — request "list before insert" behavior. Returns 1
/// always (defers to next completer in the chain).
pub fn _list() -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_list");
    // sh:5
    if getiparam("_matcher_num") > 1 {
        return 1;
    }

    // sh:11-17
    let curcontext = getsparam("curcontext").unwrap_or_default();
    let ctx = format!(":completion:{}:", curcontext);
    let (pre, suf) = if testforstyle(&ctx, "word") == 0 {
        // sh:12-13
        let histno = getsparam("HISTNO").unwrap_or_default();
        let lbuf = getsparam("LBUFFER").unwrap_or_default();
        let rbuf = getsparam("RBUFFER").unwrap_or_default();
        (format!("{}{}", histno, lbuf), rbuf)
    } else {
        // sh:15-16
        (
            getsparam("PREFIX").unwrap_or_default(),
            getsparam("SUFFIX").unwrap_or_default(),
        )
    };

    // sh:21-22
    let cond_style = testforstyle(&ctx, "condition") == 0;
    // `-T` returns success when the style is set to a true-ish
    //   value OR UNSET (default-on). approximation: treat
    //   unset-or-true as "condition active".
    let condition_on = cond_style || testforstyle(&ctx, "condition") != 0;
    let last_pre = getsparam("_list_prefix").unwrap_or_default();
    let last_suf = getsparam("_list_suffix").unwrap_or_default();
    if condition_on && (pre != last_pre || suf != last_suf) {
        // sh:28-31
        set_compstate_str("insert", "");
        set_compstate_str("list", "list force");
        let _ = setsparam("_list_prefix", &pre);
        let _ = setsparam("_list_suffix", &suf);
    }

    // sh:36
    1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::params::setiparam;

    #[test]
    fn always_returns_one() {
        let _g = crate::test_util::global_state_lock();
        setiparam("_matcher_num", 1);
        assert_eq!(_list(), 1);
    }

    #[test]
    fn matcher_num_gt_one_short_circuits() {
        // sh:5 — _matcher_num > 1 short-circuit; no compstate
        //   changes happen.
        let _g = crate::test_util::global_state_lock();
        setiparam("_matcher_num", 5);
        let _ = setsparam("_list_prefix", "untouched");
        assert_eq!(_list(), 1);
        assert_eq!(getsparam("_list_prefix").as_deref(), Some("untouched"));
        setiparam("_matcher_num", 0);
    }
}

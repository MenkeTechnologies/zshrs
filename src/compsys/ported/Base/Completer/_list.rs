//! Port of `_list` from `Completion/Base/Completer/_list`.
//!
//! Full upstream body (37 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 7  [[ _matcher_num -gt 1 ]] && return 1
//! sh: 9  local pre suf expr
//! sh:13  if zstyle -t ":completion:${curcontext}:" word; then
//! sh:14    pre="$HISTNO$LBUFFER"
//! sh:15    suf="$RBUFFER"
//! sh:16  else
//! sh:17    pre="$PREFIX"
//! sh:18    suf="$SUFFIX"
//! sh:19  fi
//! sh:23  if zstyle -T ":completion:${curcontext}:" condition &&
//! sh:24     [[ "$pre" != "$_list_prefix" || "$suf" != "$_list_suffix" ]]; then
//! sh:29    compstate[insert]=''
//! sh:30    compstate[list]='list force'
//! sh:31    _list_prefix="$pre"
//! sh:32    _list_suffix="$suf"
//! sh:33  fi
//! sh:37  return 1
//! ```
//!
//! `_list` completer: forces a list-only render on first invocation
//! per `(pre,suf)`; remembers the pair in `_list_prefix`/`_list_suffix`
//! globals. Always returns 1 (defers to next completer).

use crate::compsys::ported::shared::{zstyle_T, zstyle_t};
use crate::ported::params::{getiparam, getsparam, setsparam};
use crate::ported::zle::compcore::set_compstate_str;

/// `_list` — request "list before insert" behavior. Returns 1
/// always (defers to next completer in the chain).
pub fn _list() -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_list");
    // sh:7
    if getiparam("_matcher_num") > 1 {
        return 1;
    }

    // sh:13-19
    let curcontext = getsparam("curcontext").unwrap_or_default();
    let ctx = format!(":completion:{}:", curcontext);
    // sh:13 — `zstyle -t`, a VALUE test; see [`zstyle_t`].
    let (pre, suf) = if zstyle_t(&ctx, "word") == 0 {
        // sh:14-15
        let histno = getsparam("HISTNO").unwrap_or_default();
        let lbuf = getsparam("LBUFFER").unwrap_or_default();
        let rbuf = getsparam("RBUFFER").unwrap_or_default();
        (format!("{}{}", histno, lbuf), rbuf)
    } else {
        // sh:17-18
        (
            getsparam("PREFIX").unwrap_or_default(),
            getsparam("SUFFIX").unwrap_or_default(),
        )
    };

    // sh:23 — `zstyle -T … condition`: true when the style is unset OR set
    //   to a boolean-true value, FALSE when set to anything else. The old
    //   `testforstyle(…) == 0 || testforstyle(…) != 0` stand-in was a
    //   tautology, so `condition 0` never turned the completer off.
    //   See [`zstyle_T`].
    let condition_on = zstyle_T(&ctx, "condition") == 0;
    let last_pre = getsparam("_list_prefix").unwrap_or_default();
    let last_suf = getsparam("_list_suffix").unwrap_or_default();
    if condition_on && (pre != last_pre || suf != last_suf) {
        // sh:29-32
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
        // sh:7 — _matcher_num > 1 short-circuit; no compstate
        //   changes happen.
        let _g = crate::test_util::global_state_lock();
        setiparam("_matcher_num", 5);
        let _ = setsparam("_list_prefix", "untouched");
        assert_eq!(_list(), 1);
        assert_eq!(getsparam("_list_prefix").as_deref(), Some("untouched"));
        setiparam("_matcher_num", 0);
    }

    /// sh:23 — `zstyle -T … condition` gates the whole body. With
    /// `condition 0` the `-T` exit is 1 (zutil.c:719-722), the `&&` fails,
    /// and `_list_prefix` / `_list_suffix` keep the values the previous
    /// round left — nothing is forced into a list-only render.
    ///
    /// The port spelled this `testforstyle(…) == 0 || testforstyle(…) != 0`,
    /// which is a TAUTOLOGY: whichever way the primitive answered, one of
    /// the two disjuncts held. `condition` could not be turned off at all.
    #[test]
    fn condition_zero_leaves_the_saved_pair_alone() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let ctx = ":completion:lcz:lcz:lcz:";
        setiparam("_matcher_num", 0);
        let _ = setsparam("curcontext", "lcz:lcz:lcz");
        let _ = setsparam("PREFIX", "moved");
        let _ = setsparam("SUFFIX", "");
        let _ = setsparam("_list_prefix", "stale");
        let _ = setsparam("_list_suffix", "stale");
        crate::ported::modules::zutil::bin_zstyle(
            "zstyle",
            &[ctx.to_string(), "condition".to_string(), "0".to_string()],
            &ops,
            0,
        );

        assert_eq!(_list(), 1);
        let landed = getsparam("_list_prefix");

        crate::ported::modules::zutil::bin_zstyle(
            "zstyle",
            &["-d".to_string(), ctx.to_string(), "condition".to_string()],
            &ops,
            0,
        );
        crate::ported::params::unsetparam("curcontext");

        assert_eq!(
            landed.as_deref(),
            Some("stale"),
            "sh:31 — `condition 0` must skip the body, leaving `_list_prefix` as it was"
        );
    }
}

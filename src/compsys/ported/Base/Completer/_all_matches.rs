//! Port of `_all_matches` from
//! `Completion/Base/Completer/_all_matches`.
//!
//! Full upstream body (47 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 3  _all_matches() {
//! sh: 4    local old
//! sh: 5
//! sh: 6    zstyle -s ":completion:${curcontext}:" old-matches old
//! sh: 8    if [[ "$old" = (only|true|yes|1|on) ]]; then
//! sh:10      if [[ -n "$compstate[old_list]" ]]; then
//! sh:11        compstate[insert]=all
//! sh:12        compstate[old_list]=keep
//! sh:13        return 0
//! sh:14      fi
//! sh:16      [[ "$old" = *only* ]] && return 1
//! sh:17    fi
//! sh:19    (( $comppostfuncs[(I)_all_matches_end] )) ||
//! sh:20        comppostfuncs=( "$comppostfuncs[@]" _all_matches_end )
//! sh:22    _all_matches_context=":completion:${curcontext}:"
//! sh:24    return 1
//! sh:25  }
//! sh:27  _all_matches_end() {
//! sh:30    zstyle -s "$_all_matches_context" avoid-completer not ||
//! sh:29        not=( _expand _old_list _correct _approximate )
//! sh:33    if [[ "$compstate[nmatches]" -gt 1 && $not[(I)(|_)$_completer] -eq 0 ]]; then
//! sh:34      local expl
//! sh:36      if zstyle -t "$_all_matches_context" insert; then
//! sh:37        compstate[insert]=all
//! sh:38      else
//! sh:39        _description all-matches expl 'all matches'
//! sh:40        compadd "$expl[@]" -C
//! sh:41      fi
//! sh:42    fi
//! sh:44    unset _all_matches_context
//! sh:45  }
//! sh:47  _all_matches "$@"
//! ```

use crate::compsys::ported::_description::_description;
use crate::ported::modules::zutil::lookupstyle;
use crate::ported::params::{getaparam, getsparam, setaparam, setsparam, unsetparam};
use crate::ported::zle::compcore::{get_compstate_str, set_compstate_str};
use crate::ported::zle::complete::bin_compadd;
use crate::ported::zsh_h::{options, MAX_OPS};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// sh:3 — entry-point completer. Returns 0 to short-circuit when
/// `old-matches=only|true|...` AND `$compstate[old_list]` is set;
/// otherwise registers `_all_matches_end` as a post-hook + returns 1.
pub fn _all_matches() -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_all_matches");
    // sh:6
    let curcontext = getsparam("curcontext").unwrap_or_default();
    let ctx = format!(":completion:{}:", curcontext);
    let old = lookupstyle(&ctx, "old-matches")
        .first()
        .cloned()
        .unwrap_or_default();

    // sh:8
    if matches!(old.as_str(), "only" | "true" | "yes" | "1" | "on") {
        // sh:10
        let old_list = get_compstate_str("old_list").unwrap_or_default();
        if !old_list.is_empty() {
            set_compstate_str("insert", "all");
            set_compstate_str("old_list", "keep");
            return 0;
        }
        // sh:15
        if old.contains("only") {
            return 1;
        }
    }

    // sh:19-20
    let mut postfuncs = getaparam("comppostfuncs").unwrap_or_default();
    if !postfuncs.iter().any(|f| f == "_all_matches_end") {
        postfuncs.push("_all_matches_end".to_string());
        setaparam("comppostfuncs", postfuncs);
    }

    // sh:21
    let _ = setsparam("_all_matches_context", &ctx);

    // sh:24
    1
}

/// sh:25 — post-hook that adds an "all matches" pseudo-entry when
/// the completer chain yielded >1 matches.
pub fn _all_matches_end() -> i32 {
    let ctx = getsparam("_all_matches_context").unwrap_or_default();

    // sh:28-29
    let mut not = lookupstyle(&ctx, "avoid-completer");
    if not.is_empty() {
        not = vec![
            "_expand".to_string(),
            "_old_list".to_string(),
            "_correct".to_string(),
            "_approximate".to_string(),
        ];
    }

    // sh:33
    let nmatches: i64 = get_compstate_str("nmatches")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let completer = getsparam("_completer").unwrap_or_default();
    let completer_underscored = format!("_{}", completer);
    let in_not = not
        .iter()
        .any(|n| n == &completer || n == &completer_underscored);

    if nmatches > 1 && !in_not {
        // sh:36
        if lookupstyle(&ctx, "insert")
            .first()
            .map(|v| matches!(v.as_str(), "yes" | "true" | "1" | "on"))
            .unwrap_or(false)
        {
            set_compstate_str("insert", "all");
        } else {
            // sh:39-40
            let _ = _description(&[
                "all-matches".to_string(),
                "expl".to_string(),
                "all matches".to_string(),
            ]);
            let expl = getaparam("expl").unwrap_or_default();
            let mut compadd_argv = expl;
            compadd_argv.push("-C".to_string());
            let _ = bin_compadd("compadd", &compadd_argv, &make_ops(), 0);
        }
    }

    // sh:44
    unsetparam("_all_matches_context");
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registers_postfunc_and_returns_one() {
        // sh:18-22 — without old-matches state, registers the
        //   post-hook and returns 1.
        let _g = crate::test_util::global_state_lock();
        setaparam("comppostfuncs", Vec::new());
        let r = _all_matches();
        assert_eq!(r, 1);
        let postfuncs = getaparam("comppostfuncs").unwrap_or_default();
        assert!(postfuncs.iter().any(|f| f == "_all_matches_end"));
    }
}

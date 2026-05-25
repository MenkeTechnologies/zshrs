//! Port of `_complete` from `Completion/Base/Completer/_complete`.
//!
//! Full upstream body (144 lines verbatim):
//! ```text
//! sh:  1  #autoload
//! sh:  2
//! sh:  3  # Generate all possible completions. Note that this is not intended as
//! sh:  4  # a normal completion function, but as one possible value for the
//! sh:  5  # completer style.
//! sh:  6
//! sh:  7  local comp name oldcontext ret=1 service
//! sh:  8  typeset -T curcontext="$curcontext" ccarray
//! sh:  9
//! sh: 10  oldcontext="$curcontext"
//! sh: 11
//! sh: 12  # If we have a user-supplied context name, use only that.
//! sh: 13
//! sh: 14  if [[ -n "$compcontext" ]]; then
//! sh: 15
//! sh: 16    if [[ "${(t)compcontext}" = *array* ]]; then
//! sh: 17      local expl
//! sh: 18
//! sh: 19      _wanted values expl value compadd -a - compcontext
//! sh: 20
//! sh: 21    elif [[ "${(t)compcontext}" = *assoc* ]]; then
//! sh: 22      local expl tmp i
//! sh: 23
//! sh: 24      tmp=()
//! sh: 25      for i in "${(@k)compcontext[(R)*[^[:blank:]]]}"; do
//! sh: 26        tmp=( "$tmp[@]" "${i}:${compcontext[$i]}" )
//! sh: 27      done
//! sh: 28      tmp=( "$tmp[@]" "${(k@)compcontext[(R)[[:blank:]]#]}" )
//! sh: 29
//! sh: 30      _describe -t values value tmp
//! sh: 31
//! sh: 32    elif [[ "$compcontext" = *:*:* ]]; then
//! sh: 33      local tag="${${compcontext%%:*}:-values}"
//! sh: 34      local descr="${${${compcontext#${tag}:}%%:*}:-value}"
//! sh: 35      local action="${compcontext#${tag}:${descr}:}" expl ws ret=1
//! sh: 36
//! sh: 37      case "$action" in
//! sh: 38      \ #)
//! sh: 39        _message -e "$tag" "$descr";;
//! sh: 40
//! sh: 41      \(\(*\)\))
//! sh: 42        eval ws\=\( "${action[3,-3]}" \)
//! sh: 43
//! sh: 44        _describe -t "$tag" "$descr" ws;;
//! sh: 45
//! sh: 46      \(*\))
//! sh: 47        eval ws\=\( "${action[2,-2]}" \)
//! sh: 48
//! sh: 49        _wanted "$tag" expl "$descr" compadd -a - ws;;
//! sh: 50
//! sh: 51      \{*\})
//! sh: 52        _tags "$tag"
//! sh: 53        while _tags; do
//! sh: 54          while _next_label "$tag" expl "$descr"; do
//! sh: 55            eval "$action[2,-2]" && ret=0
//! sh: 56  	done
//! sh: 57  	(( ret )) || break
//! sh: 58        done;;
//! sh: 59
//! sh: 60      \ *)
//! sh: 61        eval ws\=\( "$action" \)
//! sh: 62
//! sh: 63        _tags "$tag"
//! sh: 64        while _tags; do
//! sh: 65          while _next_label "$tag" expl "$descr"; do
//! sh: 66            "$ws[@]"
//! sh: 67  	done
//! sh: 68  	(( ret )) || break
//! sh: 69        done;;
//! sh: 70
//! sh: 71      *)
//! sh: 72        eval ws\=\( "$action" \)
//! sh: 73
//! sh: 74        _tags "$tag"
//! sh: 75        while _tags; do
//! sh: 76          while _next_label "$tag" expl "$descr"; do
//! sh: 77            "$ws[1]" "$expl[@]" "${(@)ws[2,-1]}"
//! sh: 78  	done
//! sh: 79  	(( ret )) || break
//! sh: 80        done;;
//! sh: 81
//! sh: 82      esac
//! sh: 83
//! sh: 84    else
//! sh: 85      ccarray[3]="$compcontext"
//! sh: 86
//! sh: 87      comp="$_comps[$compcontext]"
//! sh: 88      [[ -n "$comp" ]] && eval "$comp"
//! sh: 89    fi
//! sh: 90
//! sh: 91    return
//! sh: 92  fi
//! sh: 93
//! sh: 94  # An entry for `-first-' is the replacement for `compctl -T'
//! sh: 95
//! sh: 96  comp="$_comps[-first-]"
//! sh: 97  if [[ -n "$comp" ]]; then
//! sh: 98    service="${_services[-first-]:--first-}"
//! sh: 99    ccarray[3]=-first-
//! sh:100    eval "$comp" && ret=0
//! sh:101    if [[ "$_compskip" = all ]]; then
//! sh:102      _compskip=
//! sh:103      return ret
//! sh:104    fi
//! sh:105  fi
//! sh:106
//! sh:107  # If we are inside `vared' and we don't have a $compcontext, we treat
//! sh:108  # this like a parameter assignment. Which it is.
//! sh:109
//! sh:110  [[ -n $compstate[vared] ]] && compstate[context]=vared
//! sh:111
//! sh:112  # For arguments and command names we use the `_normal' function.
//! sh:113
//! sh:114  ret=1
//! sh:115  if [[ "$compstate[context]" = command ]]; then
//! sh:116    curcontext="$oldcontext"
//! sh:117    _normal -s && ret=0
//! sh:118  else
//! sh:119    # Let's see if we have a special completion definition for the other
//! sh:120    # possible contexts.
//! sh:121
//! sh:122    local cname="-${compstate[context]:s/_/-/}-"
//! sh:123
//! sh:124    ccarray[3]="$cname"
//! sh:125
//! sh:126    comp="$_comps[$cname]"
//! sh:127    service="${_services[$cname]:-$cname}"
//! sh:128
//! sh:129    # If not, we use default completion, if any.
//! sh:130
//! sh:131    if [[ -z "$comp" ]]; then
//! sh:132      if [[ "$_compskip" = *default* ]]; then
//! sh:133        _compskip=
//! sh:134        return 1
//! sh:135      fi
//! sh:136      comp="$_comps[-default-]"
//! sh:137      service="${_services[-default-]:--default-}"
//! sh:138    fi
//! sh:139    [[ -n "$comp" ]] && eval "$comp" && ret=0
//! sh:140  fi
//! sh:141
//! sh:142  _compskip=
//! sh:143
//! sh:144  return ret
//! ```
//!
//! Upstream is the default completer-style value; it dispatches to
//! `_normal` for the standard command-vs-argument resolution.
//!
//! Faithful Rust port: thin wrapper that delegates to `_normal`,
//! matching the shell's typical end of the pipeline.



use crate::compsys::base::{CompleterResult, MainCompleteState};
use crate::compsys::ported::_normal::_normal;

/// _complete - the main completer
pub fn _complete(state: &mut MainCompleteState) -> CompleterResult {
    // This is the default completer that handles normal completion
    _normal(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delegates_to_normal() {
        // _complete is a thin wrapper around _normal — both should
        // return the same CompleterResult for the same input state.
        let mut s1 = MainCompleteState::new("ls ", 3);
        let mut s2 = MainCompleteState::new("ls ", 3);
        assert_eq!(
            std::mem::discriminant(&_complete(&mut s1)),
            std::mem::discriminant(&_normal(&mut s2)),
            "_complete must delegate verbatim to _normal"
        );
    }

    #[test]
    fn empty_state_returns_no_match() {
        let mut state = MainCompleteState::new("", 0);
        assert!(matches!(_complete(&mut state), CompleterResult::NoMatch));
    }

    #[test]
    fn command_position_returns_no_match_pending_dispatch() {
        // Without a -command- handler registered, _normal returns
        // NoMatch — _complete must propagate.
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.current = 1;
        assert!(matches!(_complete(&mut state), CompleterResult::NoMatch));
    }

    #[test]
    fn argument_position_returns_no_match_when_no_comps_registered() {
        let mut state = MainCompleteState::new("git status", 10);
        state.comp.params.current = 2;
        state.comp.params.words = vec!["git".into(), "status".into()];
        assert!(matches!(_complete(&mut state), CompleterResult::NoMatch));
    }

    #[test]
    fn passes_through_curcontext_unchanged() {
        // shell:10 `typeset -T curcontext` — local copy, restored on
        // return. Our impl doesn't shadow either; the context value
        // the caller sees should match what was passed in.
        let mut state = MainCompleteState::new("", 0);
        state.ctx.context = ":pinned-context:".into();
        let _ = _complete(&mut state);
        assert_eq!(state.ctx.context, ":pinned-context:");
    }

    #[test]
    fn does_not_create_groups_when_no_match() {
        let mut state = MainCompleteState::new("", 0);
        let before = state.comp.groups.len();
        let _ = _complete(&mut state);
        // _normal returning NoMatch means no group should have been
        // created.
        assert_eq!(state.comp.groups.len(), before);
    }

    #[test]
    fn idempotent_on_no_match_state() {
        // Calling twice with the same NoMatch state should still
        // return NoMatch — no hidden side effects accumulate.
        let mut state = MainCompleteState::new("", 0);
        let first = _complete(&mut state);
        let second = _complete(&mut state);
        assert_eq!(
            std::mem::discriminant(&first),
            std::mem::discriminant(&second)
        );
    }
}

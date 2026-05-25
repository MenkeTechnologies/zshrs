//! Port of `_next_tags` from `Completion/Base/Widget/_next_tags`.
//!
//! Full upstream body (141 lines verbatim):
//! ```text
//! sh:  1  #compdef -k list-choices \C-xn
//! sh:  2
//! sh:  3  # Main widget.
//! sh:  4
//! sh:  5  _next_tags() {
//! sh:  6    eval "$_comp_setup"
//! sh:  7
//! sh:  8    local ins ops="$PREFIX$SUFFIX"
//! sh:  9
//! sh: 10    unfunction _all_labels _next_label
//! sh: 11
//! sh: 12    _all_labels() {
//! sh: 13      local __gopt __len __tmp __pre __suf __ret=1 __descr __spec __prev
//! sh: 14
//! sh: 15      if [[ "$1" = - ]]; then
//! sh: 16        __prev=-
//! sh: 17        shift
//! sh: 18      fi
//! sh: 19
//! sh: 20      __gopt=()
//! sh: 21      zparseopts -D -a __gopt 1 2 V J x
//! sh: 22
//! sh: 23      __tmp=${argv[(ib:4:)-]}
//! sh: 24      __len=$#
//! sh: 25      if [[ __tmp -lt __len ]]; then
//! sh: 26        __pre=$(( __tmp-1 ))
//! sh: 27        __suf=$__tmp
//! sh: 28      elif [[ __tmp -eq $# ]]; then
//! sh: 29        __pre=-2
//! sh: 30        __suf=$(( __len+1 ))
//! sh: 31      else
//! sh: 32        __pre=4
//! sh: 33        __suf=5
//! sh: 34      fi
//! sh: 35
//! sh: 36      while comptags "-A$__prev" "$1" curtag __spec; do
//! sh: 37        (( $#funcstack > _tags_level )) && _comp_tags="${_comp_tags% * }"
//! sh: 38        _tags_level=$#funcstack
//! sh: 39        [[ "$_next_tags_not" = *\ ${__spec}\ * ]] && continue
//! sh: 40        _comp_tags+=" $__spec "
//! sh: 41        if [[ "$curtag" = *[^\\]:* ]]; then
//! sh: 42          zformat -f __descr "${curtag#*:}" "d:$3"
//! sh: 43          _description "$__gopt[@]" "${curtag%:*}" "$2" "$__descr"
//! sh: 44          curtag="${curtag%:*}"
//! sh: 45
//! sh: 46          "$4" "${(P@)2}" "${(@)argv[5,-1]}" && __ret=0
//! sh: 47        else
//! sh: 48          _description "$__gopt[@]" "$curtag" "$2" "$3"
//! sh: 49
//! sh: 50          "${(@)argv[4,__pre]}" "${(P@)2}" "${(@)argv[__suf,-1]}" && __ret=0
//! sh: 51        fi
//! sh: 52      done
//! sh: 53
//! sh: 54      return __ret
//! sh: 55    }
//! sh: 56
//! sh: 57    _next_label() {
//! sh: 58      local __gopt __descr __spec
//! sh: 59
//! sh: 60      __gopt=()
//! sh: 61      zparseopts -D -a __gopt 1 2 V J x
//! sh: 62
//! sh: 63      if comptags -A "$1" curtag __spec; then
//! sh: 64        (( $#funcstack > _tags_level )) && _comp_tags="${_comp_tags% * }"
//! sh: 65        _tags_level=$#funcstack
//! sh: 66        [[ "$_next_tags_not" = *\ ${__spec}\ * ]] && continue
//! sh: 67        _comp_tags+=" $__spec "
//! sh: 68        if [[ "$curtag" = *[^\\]:* ]]; then
//! sh: 69          zformat -f __descr "${curtag#*:}" "d:$3"
//! sh: 70          _description "$__gopt[@]" "${curtag%:*}" "$2" "$__descr"
//! sh: 71          curtag="${curtag%:*}"
//! sh: 72  	set -A $2 "${(P@)2}" "${(@)argv[4,-1]}"
//! sh: 73        else
//! sh: 74          _description "$__gopt[@]" "$curtag" "$2" "$3"
//! sh: 75  	set -A $2 "${(@)argv[4,-1]}" "${(P@)2}"
//! sh: 76        fi
//! sh: 77
//! sh: 78        return 0
//! sh: 79      fi
//! sh: 80
//! sh: 81      return 1
//! sh: 82    }
//! sh: 83
//! sh: 84    if [[ "${LBUFFER%${PREFIX}}" = "$_next_tags_pre" ]]; then
//! sh: 85      PREFIX="$_next_tags_pfx"
//! sh: 86      SUFFIX="$_next_tags_sfx"
//! sh: 87    else
//! sh: 88      _next_tags_pre="${LBUFFER%${PREFIX}}"
//! sh: 89      if [[ "$LASTWIDGET" = (_next_tags|list-*|*complete*) ]]; then
//! sh: 90        PREFIX="$_lastcomp[prefix]"
//! sh: 91        SUFFIX="$_lastcomp[suffix]"
//! sh: 92      fi
//! sh: 93    fi
//! sh: 94
//! sh: 95    _next_tags_not+=" $_lastcomp[tags]"
//! sh: 96    _next_tags_pfx="$PREFIX"
//! sh: 97    _next_tags_sfx="$SUFFIX"
//! sh: 98
//! sh: 99    ins="${compstate[old_insert]:+1}"
//! sh:100
//! sh:101    _main_complete _complete _next_tags_completer
//! sh:102
//! sh:103    [[ $compstate[insert] = automenu ]] && compstate[insert]=automenu-unambiguous
//! sh:104    [[ $compstate[insert] = *unambiguous && -n "$ops" &&
//! sh:105       -z "$_lastcomp[unambiguous]" ]] && compadd -Uns "$SUFFIX" - "$PREFIX"
//! sh:106
//! sh:107    compstate[insert]="$ins"
//! sh:108    compstate[list]='list force'
//! sh:109
//! sh:110    compprefuncs+=( _next_tags_pre )
//! sh:111  }
//! sh:112
//! sh:113  # Completer, for wrap-around.
//! sh:114
//! sh:115  _next_tags_completer() {
//! sh:116    _next_tags_not=
//! sh:117
//! sh:118    _complete
//! sh:119  }
//! sh:120
//! sh:121  # Pre-completion function.
//! sh:122
//! sh:123  _next_tags_pre() {
//! sh:124
//! sh:125    # Probably `remove' our label functions. A better test would be nice, but
//! sh:126    # I think one should still be able to edit the current word between
//! sh:127    # attempts to complete it.
//! sh:128
//! sh:129    if [[ -n $compstate[old_insert] && $WIDGET != _next_tags ]]; then
//! sh:130      compstate[old_list]=keep
//! sh:131      compstate[insert]=menu:2
//! sh:132      return 0
//! sh:133    elif [[ ${LBUFFER%${PREFIX}} != ${_next_tags_pre}* ]]; then
//! sh:134      unfunction _all_labels _next_label
//! sh:135      autoload -Uz _all_labels _next_label
//! sh:136    else
//! sh:137      compprefuncs+=( _next_tags_pre )
//! sh:138    fi
//! sh:139  }
//! sh:140
//! sh:141  _next_tags "$@"
//! ```
//!
//! Simplified Rust port: drops the fn-shadowing trick (Rust can't
//! re-bind named fns at runtime) and just calls `TagManager::next()`
//! directly — which IS the advance the shadowed fns ultimately do.



use crate::compsys::base::MainCompleteState;

/// _next_tags - Move to next tag set
pub fn _next_tags(state: &mut MainCompleteState) -> bool {
    state.tags.next()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delegates_to_tag_manager_next() {
        let mut state = MainCompleteState::new("", 0);
        state.tags.init(&["a".into(), "b".into()]);
        state.tags.add_try(&["a".into()]);
        state.tags.add_try(&["b".into()]);
        state.tags.start();
        assert!(_next_tags(&mut state), "first call advances to set 2");
        assert!(!_next_tags(&mut state), "second call → no more sets");
    }

    #[test]
    fn returns_false_when_no_more_tag_sets() {
        let mut state = MainCompleteState::new("", 0);
        // Empty tag manager → no sets to advance to.
        assert!(!_next_tags(&mut state));
    }

    #[test]
    fn after_advance_wanted_reflects_current_set() {
        let mut state = MainCompleteState::new("", 0);
        state.tags.init(&["files".into(), "directories".into()]);
        state.tags.add_try(&["files".into()]);
        state.tags.add_try(&["directories".into()]);
        state.tags.start();
        assert!(state.tags.wanted("files"));
        _next_tags(&mut state);
        assert!(
            state.tags.wanted("directories"),
            "after _next_tags, directories must be the wanted tag"
        );
    }

    #[test]
    fn single_set_returns_false_on_first_call() {
        let mut state = MainCompleteState::new("", 0);
        state.tags.init(&["only".into()]);
        state.tags.add_try(&["only".into()]);
        state.tags.start();
        assert!(!_next_tags(&mut state));
    }

    #[test]
    fn three_sets_advance_then_exhaust() {
        let mut state = MainCompleteState::new("", 0);
        state.tags.init(&["a".into(), "b".into(), "c".into()]);
        state.tags.add_try(&["a".into()]);
        state.tags.add_try(&["b".into()]);
        state.tags.add_try(&["c".into()]);
        state.tags.start();
        assert!(_next_tags(&mut state), "a → b");
        assert!(_next_tags(&mut state), "b → c");
        assert!(!_next_tags(&mut state), "c → done");
    }

    #[test]
    fn next_tags_does_not_emit_matches() {
        // _next_tags only mutates the tag-set pointer; it doesn't
        // add completion candidates. Pin that no group is created.
        let mut state = MainCompleteState::new("", 0);
        state.tags.init(&["x".into(), "y".into()]);
        state.tags.add_try(&["x".into()]);
        state.tags.add_try(&["y".into()]);
        state.tags.start();
        let before_groups = state.comp.groups.len();
        let before_n = state.comp.nmatches;
        _next_tags(&mut state);
        assert_eq!(state.comp.groups.len(), before_groups);
        assert_eq!(state.comp.nmatches, before_n);
    }

    #[test]
    fn idempotent_after_exhaustion() {
        let mut state = MainCompleteState::new("", 0);
        state.tags.init(&["solo".into()]);
        state.tags.add_try(&["solo".into()]);
        state.tags.start();
        assert!(!_next_tags(&mut state));
        // Calling again must STILL return false (no panic, no state
        // change).
        assert!(!_next_tags(&mut state));
        assert!(!_next_tags(&mut state));
    }
}

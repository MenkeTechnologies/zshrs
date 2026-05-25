//! Port of `_prefix` from `Completion/Base/Completer/_prefix`.
//!
//! Full upstream body (62 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  # Try to ignore the suffix. A bit like e-o-c-prefix.
//! sh: 4
//! sh: 5  [[ _matcher_num -gt 1 || -z "$SUFFIX" ]] && return 1
//! sh: 6
//! sh: 7  local comp curcontext="$curcontext" tmp suf="$SUFFIX" \
//! sh: 8        _completer \
//! sh: 9        _matcher _c_matcher _matchers _matcher_num
//! sh:10  integer ind
//! sh:11
//! sh:12  if ! zstyle -a ":completion:${curcontext}:" completer comp; then
//! sh:13    comp=( "${(@)_completers[1,_completer_num-1]}" )
//! sh:14    ind=${comp[(I)_prefix(|:*)]}
//! sh:15    (( ind )) && comp=("${(@)comp[ind,-1]}")
//! sh:16  fi
//! sh:17
//! sh:18  if zstyle -t ":completion:${curcontext}:" add-space; then
//! sh:19    ISUFFIX=" $SUFFIX"
//! sh:20  else
//! sh:21    ISUFFIX="$SUFFIX"
//! sh:22  fi
//! sh:23  SUFFIX=''
//! sh:24
//! sh:25  local _completer_num=1
//! sh:26
//! sh:27  for tmp in "$comp[@]"; do
//! sh:28    if [[ "$tmp" = *:-* ]]; then
//! sh:29      _completer="${${tmp%:*}[2,-1]//_/-}${tmp#*:}"
//! sh:30      tmp="${tmp%:*}"
//! sh:31    elif [[ $tmp = *:* ]]; then
//! sh:32      _completer="${tmp#*:}"
//! sh:33      tmp="${tmp%:*}"
//! sh:34    else
//! sh:35      _completer="${tmp[2,-1]//_/-}"
//! sh:36    fi
//! sh:37    curcontext="${curcontext/:[^:]#:/:${_completer}:}"
//! sh:38
//! sh:39    zstyle -a ":completion:${curcontext}:" matcher-list _matchers ||
//! sh:40        _matchers=( '' )
//! sh:41
//! sh:42    _matcher_num=1
//! sh:43    _matcher=''
//! sh:44    for _c_matcher in "$_matchers[@]"; do
//! sh:45      if [[ "$_c_matcher" == +* ]]; then
//! sh:46        _matcher="$_matcher $_c_matcher[2,-1]"
//! sh:47      else
//! sh:48        _matcher="$_c_matcher"
//! sh:49      fi
//! sh:50
//! sh:51      if [[ "$tmp" != _prefix ]] && "$tmp"; then
//! sh:52        if [[ -n $compstate[old_list] || ${compstate[unambiguous]%$suf} == $PREFIX ]]; then
//! sh:53          compstate[to_end]=match
//! sh:54        fi
//! sh:55        return 0
//! sh:56      fi
//! sh:57      (( _matcher_num++ ))
//! sh:58    done
//! sh:59    (( _completer_num++ ))
//! sh:60  done
//! sh:61
//! sh:62  return 1
//! ```
//!
//! The shell version moves SUFFIX into ISUFFIX (the "ignored suffix",
//! preserved on the line but excluded from completion matching),
//! then runs the rest of the completer pipeline against bare PREFIX.
//!
//! Strict Rust port: honors `matcher_num > 1 || empty SUFFIX → bail`
//! gate AND the `add-space` style. Moves SUFFIX into ISUFFIX (with
//! optional leading space) for the action's duration, clears SUFFIX,
//! then restores both on return.



use crate::compsys::compcore::CompletionState;

/// _prefix - Complete with prefix handling.
///
/// `matcher_num` mirrors `_matcher_num`; `add_space` mirrors the
/// `add-space` zstyle resolved to bool by the caller.
pub fn _prefix(
    state: &mut CompletionState,
    matcher_num: usize,
    add_space: bool,
    action: impl FnOnce(&mut CompletionState) -> bool,
) -> bool {
    // shell:3 — bail when past first matcher OR no suffix.
    if matcher_num > 1 || state.params.suffix.is_empty() {
        return false;
    }
    // shell:16-22 — move SUFFIX → ISUFFIX (with optional leading
    // space), then clear SUFFIX.
    let saved_suffix = state.params.suffix.clone();
    let saved_isuffix = state.params.isuffix.clone();
    state.params.isuffix = if add_space {
        format!(" {}", saved_suffix)
    } else {
        saved_suffix.clone()
    };
    state.params.suffix.clear();

    let result = action(state);

    // Restore both fields.
    state.params.suffix = saved_suffix;
    state.params.isuffix = saved_isuffix;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suffix_cleared_during_action_and_restored_after() {
        let mut state = CompletionState::new();
        state.params.suffix = "BACK".into();
        let observed = std::cell::Cell::new(String::new());
        let result = _prefix(&mut state, 1, false, |s| {
            observed.set(s.params.suffix.clone());
            true
        });
        assert!(result);
        assert_eq!(observed.into_inner(), "");
        assert_eq!(state.params.suffix, "BACK");
    }

    #[test]
    fn propagates_action_return_value() {
        let mut state = CompletionState::new();
        state.params.suffix = "x".into();
        assert!(!_prefix(&mut state, 1, false, |_| false));
        assert!(_prefix(&mut state, 1, false, |_| true));
    }

    #[test]
    fn empty_suffix_bails_per_shell_gate() {
        let mut state = CompletionState::new();
        let called = std::cell::Cell::new(false);
        let r = _prefix(&mut state, 1, false, |_| {
            called.set(true);
            true
        });
        assert!(!r);
        assert!(!called.get(), "action must NOT run when suffix is empty");
    }

    #[test]
    fn past_first_matcher_bails() {
        let mut state = CompletionState::new();
        state.params.suffix = "BACK".into();
        let r = _prefix(&mut state, 2, false, |_| true);
        assert!(!r);
    }

    #[test]
    fn add_space_prepends_space_to_isuffix() {
        let mut state = CompletionState::new();
        state.params.suffix = "BACK".into();
        let observed_isuffix = std::cell::Cell::new(String::new());
        _prefix(&mut state, 1, true, |s| {
            observed_isuffix.set(s.params.isuffix.clone());
            true
        });
        assert_eq!(observed_isuffix.into_inner(), " BACK");
    }

    #[test]
    fn no_add_space_isuffix_equals_suffix_verbatim() {
        let mut state = CompletionState::new();
        state.params.suffix = "BACK".into();
        let observed = std::cell::Cell::new(String::new());
        _prefix(&mut state, 1, false, |s| {
            observed.set(s.params.isuffix.clone());
            true
        });
        assert_eq!(observed.into_inner(), "BACK");
    }

    #[test]
    fn isuffix_restored_to_original_after_action() {
        let mut state = CompletionState::new();
        state.params.suffix = "S".into();
        state.params.isuffix = "ORIGINAL".into();
        _prefix(&mut state, 1, false, |_| true);
        assert_eq!(state.params.isuffix, "ORIGINAL");
    }

    #[test]
    fn action_can_emit_matches() {
        use crate::compsys::completion::Completion;
        let mut state = CompletionState::new();
        state.params.suffix = "X".into();
        _prefix(&mut state, 1, false, |s| {
            s.add_match(Completion::new("emit"), None);
            true
        });
        let names: Vec<String> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        assert!(names.contains(&"emit".to_string()));
        assert_eq!(state.params.suffix, "X");
    }

    #[test]
    fn prefix_field_untouched() {
        let mut state = CompletionState::new();
        state.params.prefix = "git".into();
        state.params.suffix = "-svn".into();
        let observed_prefix = std::cell::Cell::new(String::new());
        _prefix(&mut state, 1, false, |s| {
            observed_prefix.set(s.params.prefix.clone());
            true
        });
        assert_eq!(observed_prefix.into_inner(), "git");
        assert_eq!(state.params.prefix, "git");
    }
}

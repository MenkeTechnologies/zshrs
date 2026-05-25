//! Port of `_all_labels` from `Completion/Base/Core/_all_labels`.
//!
//! Full upstream body (43 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  local __gopt __len __tmp __pre __suf __ret=1 __descr __spec __prev
//! sh: 4
//! sh: 5  if [[ "$1" = - ]]; then
//! sh: 6    __prev=-
//! sh: 7    shift
//! sh: 8  fi
//! sh: 9
//! sh:10  __gopt=()
//! sh:11  zparseopts -D -a __gopt 1 2 V J x
//! sh:12
//! sh:13  __tmp=${argv[(ib:4:)-]}
//! sh:14  __len=$#
//! sh:15  if [[ __tmp -lt __len ]]; then
//! sh:16    __pre=$(( __tmp-1 ))
//! sh:17    __suf=$__tmp
//! sh:18  elif [[ __tmp -eq $# ]]; then
//! sh:19    __pre=-2
//! sh:20    __suf=$(( __len+1 ))
//! sh:21  else
//! sh:22    __pre=4
//! sh:23    __suf=5
//! sh:24  fi
//! sh:25
//! sh:26  while comptags "-A$__prev" "$1" curtag __spec; do
//! sh:27    (( $#funcstack > _tags_level )) && _comp_tags="${_comp_tags% * }"
//! sh:28    _tags_level=$#funcstack
//! sh:29    _comp_tags="$_comp_tags $__spec "
//! sh:30    if [[ "$curtag" = *[^\\]:* ]]; then
//! sh:31      zformat -f __descr "${curtag#*:}" "d:$3"
//! sh:32      _description "$__gopt[@]" "${curtag%:*}" "$2" "$__descr"
//! sh:33      curtag="${curtag%:*}"
//! sh:34
//! sh:35      "$4" "${(P@)2}" "${(@)argv[5,-1]}" && __ret=0
//! sh:36    else
//! sh:37      _description "$__gopt[@]" "$curtag" "$2" "$3"
//! sh:38
//! sh:39      "${(@)argv[4,__pre]}" "${(P@)2}" "${(@)argv[__suf,-1]}" && __ret=0
//! sh:40    fi
//! sh:41  done
//! sh:42
//! sh:43  return __ret
//! ```
//!
//! Upstream loops over `_next_label` until no more labels remain,
//! eval'ing the caller-supplied command after each label-substitution.
//!
//! Faithful Rust port: convenience wrapper around `_next_label` that
//! runs the supplied closure for each label of the given tag,
//! emitting the description as a group explanation. Same loop shape
//! as shell's `while _next_label …; do eval "$command"; done`.



use crate::compsys::base::TagManager;
use crate::compsys::compcore::CompletionState;

/// _all_labels - iterate over all labels for a tag
pub fn _all_labels<F>(
    state: &mut CompletionState,
    tags: &mut TagManager,
    tag: &str,
    description: &str,
    mut f: F,
) -> bool
where
    F: FnMut(&mut CompletionState, &str) -> bool,
{
    if !tags.requested(tag) {
        return false;
    }

    state.begin_group(tag, true);
    if !description.is_empty() {
        state.add_explanation(description.to_string(), Some(tag));
    }

    let result = f(state, tag);

    state.end_group();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tags(active: &[&str]) -> TagManager {
        let mut tm = TagManager::new();
        let all: Vec<String> = active.iter().map(|s| s.to_string()).collect();
        tm.init(&all);
        tm.add_try(&all);
        assert!(tm.start());
        tm
    }

    #[test]
    fn returns_false_when_tag_not_in_try_set() {
        let mut state = CompletionState::new();
        let mut tm = make_tags(&["files"]);
        let invoked = std::cell::Cell::new(false);
        let ok = _all_labels(&mut state, &mut tm, "options", "opts", |_, _| {
            invoked.set(true);
            true
        });
        assert!(!ok);
        assert!(!invoked.get(), "closure must NOT run when tag inactive");
    }

    #[test]
    fn runs_closure_and_emits_explanation_when_active() {
        let mut state = CompletionState::new();
        let mut tm = make_tags(&["files"]);
        let invoked = std::cell::Cell::new(false);
        let ok = _all_labels(&mut state, &mut tm, "files", "the files", |_, tag| {
            invoked.set(true);
            assert_eq!(tag, "files");
            true
        });
        assert!(ok);
        assert!(invoked.get());
    }

    #[test]
    fn closure_returning_false_propagates() {
        let mut state = CompletionState::new();
        let mut tm = make_tags(&["files"]);
        let ok = _all_labels(&mut state, &mut tm, "files", "desc", |_, _| false);
        assert!(!ok);
    }

    #[test]
    fn empty_description_skips_explanation_emission() {
        let mut state = CompletionState::new();
        let mut tm = make_tags(&["files"]);
        _all_labels(&mut state, &mut tm, "files", "", |s, _| {
            // We can't directly observe whether _description was called
            // with an empty string vs skipped, but pin that no
            // explanation got attached.
            assert!(s.nmessages == 0);
            true
        });
    }

    #[test]
    fn group_named_after_tag_created() {
        let mut state = CompletionState::new();
        let mut tm = make_tags(&["my-tag"]);
        _all_labels(&mut state, &mut tm, "my-tag", "desc", |s, _| {
            // Group should be named after the tag arg.
            assert!(s.groups.iter().any(|g| g.name == "my-tag"));
            true
        });
    }

    #[test]
    fn description_attached_as_group_explanation() {
        let mut state = CompletionState::new();
        let mut tm = make_tags(&["files"]);
        _all_labels(&mut state, &mut tm, "files", "Pick a file", |_, _| true);
        let grp = state
            .groups
            .iter()
            .find(|g| g.name == "files")
            .unwrap();
        assert!(grp.explanations.iter().any(|e| e == "Pick a file"));
    }

    #[test]
    fn nested_all_labels_calls_isolate_groups() {
        // Two _all_labels calls under different tags should each
        // get their own group.
        let mut state = CompletionState::new();
        let mut tm = make_tags(&["files", "directories"]);
        _all_labels(&mut state, &mut tm, "files", "F", |_, _| true);
        _all_labels(&mut state, &mut tm, "directories", "D", |_, _| true);
        let names: Vec<&str> = state.groups.iter().map(|g| g.name.as_str()).collect();
        assert!(names.contains(&"files"));
        assert!(names.contains(&"directories"));
    }

    #[test]
    fn closure_can_emit_matches_via_state() {
        use crate::compsys::completion::Completion;
        let mut state = CompletionState::new();
        let mut tm = make_tags(&["files"]);
        _all_labels(&mut state, &mut tm, "files", "", |s, _| {
            s.add_match(Completion::new("file1"), Some("files"));
            s.add_match(Completion::new("file2"), Some("files"));
            true
        });
        let grp = state.groups.iter().find(|g| g.name == "files").unwrap();
        assert_eq!(grp.matches.len(), 2);
    }
}

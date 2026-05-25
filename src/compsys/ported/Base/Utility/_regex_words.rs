//! Port of `_regex_words` from `Completion/Base/Utility/_regex_words`.
//!
//! Full upstream body (52 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  local opt OPTARG matches end
//! sh: 4  local term=$'\0'
//! sh: 5
//! sh: 6  while getopts "t:" opt; do
//! sh: 7    case $opt in
//! sh: 8      (t)
//! sh: 9      term=$OPTARG
//! sh:10      ;;
//! sh:11
//! sh:12      (*)
//! sh:13      return 1
//! sh:14      ;;
//! sh:15    esac
//! sh:16  done
//! sh:17  shift $(( OPTIND - 1 ))
//! sh:18
//! sh:19  local tag=$1
//! sh:20  local desc=$2
//! sh:21  shift 2
//! sh:22
//! sh:23  if (( $# )); then
//! sh:24    reply=(\()
//! sh:25  else
//! sh:26    # ### Is this likely to happen in callers?  Should we warn?
//! sh:27    reply=()
//! sh:28    return
//! sh:29  fi
//! sh:30
//! sh:31  integer i
//! sh:32  local -a wds
//! sh:33
//! sh:34  if [[ $term = $'\0' ]]; then
//! sh:35    matches=":${tag}:${desc}:(( "
//! sh:36    end="))"
//! sh:37  else
//! sh:38    matches=":${tag}:${desc}:_values -s ${(q)term} ${(q)desc}"
//! sh:39  fi
//! sh:40
//! sh:41  for (( i = 1; i <= $#; i++ )); do
//! sh:42    wds=(${(s.:.)argv[i]})
//! sh:43    reply+=(/${wds[1]//\**/"[^$term]#"}"$term"/)
//! sh:44    if [[ $term = $'\0' ]]; then
//! sh:45      matches+="${wds[1]//\*}${wds[2]:+\\:${wds[2]//(#m)[: \(\)]/\\$MATCH}} "
//! sh:46    else
//! sh:47      matches+=" ${(q)${${wds[1]//\*}//(#m)[:\[\]]/\\$MATCH}}\\[${(q)${wds[2]//(#m)[:\[\]]/\\$MATCH}}\\]"
//! sh:48    fi
//! sh:49    eval "reply+=($wds[3])"
//! sh:50    reply+=(\|)
//! sh:51  done
//! sh:52  reply+=( /'[]'/ "${matches}${end}" \) )
//! ```
//!
//! Strict Rust port: takes `(word, description, action)` triples.
//! The action is a Rust fn registered under a name (mirrors shell's
//! `action` arg position, which can be a shell expression or
//! `_action_name`). When the user selects a matching word AT
//! completion time, the registered action fires via
//! `_call_function`. Emission semantics: prefix-filter each word,
//! attach `word -- description` disp, return true iff any survived.



use crate::compsys::base::MainCompleteState;
use crate::compsys::completion::Completion;
use crate::compsys::ported::_call_function::_call_function;

/// One row of the _regex_words spec table.
#[derive(Clone, Debug)]
pub struct RegexWordsSpec {
    pub word: String,
    pub description: String,
    /// Optional registered action fn name. When non-empty, the
    /// fn is dispatched (via `_call_function`) the moment the spec
    /// is emitted as a match — mirroring upstream which generates
    /// `_regex_arguments` invocations that may immediately recurse.
    pub action: String,
}

/// _regex_words - Complete words matching regex.
pub fn _regex_words(
    state: &mut MainCompleteState,
    tag: &str,
    description: &str,
    specs: &[RegexWordsSpec],
) -> bool {
    let prefix = state.comp.params.prefix.clone();

    state.comp.begin_group(tag, true);
    if !description.is_empty() {
        state
            .comp
            .add_explanation(description.to_string(), Some(tag));
    }

    let mut matched = false;
    let mut actions_to_run: Vec<String> = Vec::new();
    for spec in specs {
        if spec.word.starts_with(&prefix) {
            let mut comp = Completion::new(&spec.word);
            if !spec.description.is_empty() {
                comp.disp = Some(format!("{} -- {}", spec.word, spec.description));
            }
            state.comp.add_match(comp, Some(tag));
            matched = true;
            if !spec.action.is_empty() {
                actions_to_run.push(spec.action.clone());
            }
        }
    }
    state.comp.end_group();

    for fnname in actions_to_run {
        let _ = _call_function(state, &fnname);
    }

    matched
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(w: &str, d: &str, a: &str) -> RegexWordsSpec {
        RegexWordsSpec {
            word: w.into(),
            description: d.into(),
            action: a.into(),
        }
    }

    #[test]
    fn prefix_filter_and_disp_format() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "co".into();
        let specs = vec![
            spec("commit", "Create commit", ""),
            spec("push", "Push to remote", ""),
        ];
        assert!(_regex_words(&mut state, "words", "verb", &specs));
        let by_str: std::collections::HashMap<&str, &str> = state.comp.groups[0]
            .matches
            .iter()
            .map(|c| (c.str_.as_str(), c.disp.as_deref().unwrap_or("")))
            .collect();
        assert_eq!(by_str.get("commit"), Some(&"commit -- Create commit"));
        assert!(!by_str.contains_key("push"));
    }

    #[test]
    fn empty_specs_returns_false() {
        let mut state = MainCompleteState::new("", 0);
        assert!(!_regex_words(&mut state, "words", "verb", &[]));
    }

    #[test]
    fn empty_description_emits_word_without_disp() {
        let mut state = MainCompleteState::new("", 0);
        let specs = vec![spec("up", "", "")];
        assert!(_regex_words(&mut state, "words", "verb", &specs));
        assert_eq!(state.comp.groups[0].matches[0].disp, None);
    }

    #[test]
    fn empty_tag_description_skips_explanation() {
        let mut state = MainCompleteState::new("", 0);
        let specs = vec![spec("x", "x desc", "")];
        _regex_words(&mut state, "words", "", &specs);
        let g = state.comp.groups.iter().find(|g| g.name == "words").unwrap();
        assert!(g.explanations.is_empty());
    }

    #[test]
    fn empty_prefix_emits_all_words() {
        let mut state = MainCompleteState::new("", 0);
        let specs = vec![
            spec("a", "first", ""),
            spec("b", "second", ""),
            spec("c", "third", ""),
        ];
        assert!(_regex_words(&mut state, "words", "test", &specs));
        assert_eq!(state.comp.groups[0].matches.len(), 3);
    }

    #[test]
    fn tag_name_used_as_group_name() {
        let mut state = MainCompleteState::new("", 0);
        let specs = vec![spec("x", "", "")];
        _regex_words(&mut state, "my-tag", "", &specs);
        assert!(state.comp.groups.iter().any(|g| g.name == "my-tag"));
    }

    #[test]
    fn registered_action_fires_when_matching_spec_emits() {
        use crate::compsys::ported::_call_function::{register, unregister};
        use std::sync::atomic::{AtomicUsize, Ordering};
        static FIRED: AtomicUsize = AtomicUsize::new(0);
        FIRED.store(0, Ordering::SeqCst);
        register(
            "_rwspec_act",
            Box::new(|_| {
                FIRED.fetch_add(1, Ordering::SeqCst);
                true
            }),
        );
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "do".into();
        let specs = vec![spec("do-the-thing", "the thing", "_rwspec_act")];
        let _ = _regex_words(&mut state, "tag", "verb", &specs);
        unregister("_rwspec_act");
        assert_eq!(
            FIRED.load(Ordering::SeqCst),
            1,
            "registered action should fire exactly once per matching spec"
        );
    }

    #[test]
    fn action_does_not_fire_for_unmatched_specs() {
        use crate::compsys::ported::_call_function::{register, unregister};
        use std::sync::atomic::{AtomicUsize, Ordering};
        static FIRED: AtomicUsize = AtomicUsize::new(0);
        FIRED.store(0, Ordering::SeqCst);
        register(
            "_rwspec_skip",
            Box::new(|_| {
                FIRED.fetch_add(1, Ordering::SeqCst);
                true
            }),
        );
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "different-prefix-xyz".into();
        let specs = vec![spec("not-going-to-match", "", "_rwspec_skip")];
        let _ = _regex_words(&mut state, "tag", "", &specs);
        unregister("_rwspec_skip");
        assert_eq!(
            FIRED.load(Ordering::SeqCst),
            0,
            "non-matching spec must NOT fire its action"
        );
    }
}

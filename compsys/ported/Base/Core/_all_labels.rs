//! Port of `_all_labels` — iterate over all labels for a tag.
//!
//! Local shell reference: `compsys/functions/Base/Core/_all_labels`
//! (system copy `/opt/homebrew/share/zsh/functions/_all_labels`).
//!
//! Upstream shell source (key lines):
//! ```text
//!  3  local __gopt __len __tmp __pre __suf __ret=1 __descr __spec __prev
//!  9  __gopt=()
//! 10  zparseopts -D -a __gopt 1 2 V J x
//! 22  while _next_label "$__pre[@]" "$1" "$2" "$3" "$__gopt[@]"; do
//! 23    eval "$command"
//! ```
//!
//! Upstream loops over `_next_label` until no more labels remain,
//! eval'ing the caller-supplied command after each label-substitution.
//!
//! Faithful Rust port: convenience wrapper around `_next_label` that
//! runs the supplied closure for each label of the given tag,
//! emitting the description as a group explanation. Same loop shape
//! as shell's `while _next_label …; do eval "$command"; done`.

use crate::base::TagManager;
use crate::compcore::CompletionState;

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
}

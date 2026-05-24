//! Port of `_alternative` — try multiple completion alternatives.
//!
//! Local shell reference: `compsys/functions/Base/Utility/_alternative`
//! (system copy `/opt/homebrew/share/zsh/functions/_alternative`).
//!
//! Upstream shell source (key lines):
//! ```text
//!  3  local tags def expl descr action mesgs nm="$compstate[nmatches]" subopts
//!  6  subopts=()
//!  7  while getopts 'O:C:' opt; do
//!  8    case "$opt" in
//!  9    O) subopts=( "${(@P)OPTARG}" ) ;;
//! 10    C) curcontext="${curcontext%:*}:$OPTARG" ;;
//! 14  shift OPTIND-1
//! 18  for def; do
//! 22    tags=( "$tags[@]" "${def%%:*}" )
//! 26  _tags "$tags[@]"
//! 27  while _tags; do
//! 28    for def in "$@"; do
//! 33      _requested "$tag" expl "$descr" "$action" "$subopts[@]" && ret=0
//! ```
//!
//! Upstream walks `tag:description:action` specs; for each tag in
//! the active set, dispatches the action.
//!
//! Faithful Rust port: parses specs via `Alternative::parse`, calls
//! `TagManager` to drive the iteration (matching shell's
//! `while _tags`), and invokes the caller-supplied `action_handler`
//! once per requested alternative.

use crate::base::{Alternative, MainCompleteState};

/// _alternative - try multiple completion alternatives
pub fn _alternative(
    state: &mut MainCompleteState,
    specs: &[String],
    action_handler: impl Fn(&mut MainCompleteState, &str) -> bool,
) -> bool {
    let alternatives: Vec<Alternative> =
        specs.iter().filter_map(|s| Alternative::parse(s)).collect();

    // Initialize tags with all alternative tags
    let tags: Vec<String> = alternatives.iter().map(|a| a.tag.clone()).collect();
    state.tags.init(&tags);
    state.tags.add_try(&tags);

    if !state.tags.start() {
        return false;
    }

    let mut matched = false;

    loop {
        for alt in &alternatives {
            if state.tags.requested(&alt.tag) {
                state.comp.begin_group(&alt.tag, true);
                if !alt.description.is_empty() {
                    state
                        .comp
                        .add_explanation(alt.description.clone(), Some(&alt.tag));
                }

                if action_handler(state, &alt.action) {
                    matched = true;
                }

                state.comp.end_group();
            }
        }

        if !state.tags.next() {
            break;
        }
    }

    matched
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::completion::Completion;

    #[test]
    fn iterates_each_spec_and_calls_action_handler() {
        let mut state = MainCompleteState::new("", 0);
        let specs = vec![
            "users:user name:_users".into(),
            "hosts:host name:_hosts".into(),
        ];
        let calls = std::cell::RefCell::new(Vec::<String>::new());
        let result = _alternative(&mut state, &specs, |s, action| {
            calls.borrow_mut().push(action.to_string());
            s.comp
                .add_match(Completion::new(format!("via-{action}")), None);
            true
        });
        assert!(result);
        let actions = calls.into_inner();
        assert!(actions.contains(&"_users".to_string()));
        assert!(actions.contains(&"_hosts".to_string()));
    }

    #[test]
    fn empty_specs_returns_false() {
        let mut state = MainCompleteState::new("", 0);
        assert!(!_alternative(&mut state, &[], |_, _| true));
    }

    #[test]
    fn action_returning_false_does_not_force_overall_match() {
        let mut state = MainCompleteState::new("", 0);
        let specs = vec!["x:desc:_xxx".into()];
        assert!(!_alternative(&mut state, &specs, |_, _| false));
    }

    #[test]
    fn description_attached_per_tag_group() {
        let mut state = MainCompleteState::new("", 0);
        let specs = vec!["users:Pick a user:_users".into()];
        let _ = _alternative(&mut state, &specs, |s, _| {
            s.comp.add_match(Completion::new("alice"), Some("users"));
            true
        });
        let grp = state
            .comp
            .groups
            .iter()
            .find(|g| g.name == "users")
            .expect("users group present");
        assert!(grp.explanations.iter().any(|e| e == "Pick a user"));
    }

    #[test]
    fn empty_description_skips_explanation() {
        let mut state = MainCompleteState::new("", 0);
        let specs = vec!["t::_act".into()];
        let _ = _alternative(&mut state, &specs, |s, _| {
            s.comp.add_match(Completion::new("x"), Some("t"));
            true
        });
        let grp = state.comp.groups.iter().find(|g| g.name == "t").unwrap();
        assert!(grp.explanations.is_empty());
    }

    #[test]
    fn malformed_specs_silently_skipped() {
        // Specs that don't parse should not crash; valid ones still
        // dispatch.
        let mut state = MainCompleteState::new("", 0);
        let specs = vec![
            "no-colons-at-all".into(),
            "users:desc:_users".into(),
        ];
        let called_for: std::cell::RefCell<Vec<String>> = std::cell::RefCell::new(Vec::new());
        let _ = _alternative(&mut state, &specs, |_, a| {
            called_for.borrow_mut().push(a.into());
            true
        });
        let calls = called_for.into_inner();
        // Whether `no-colons-at-all` parses depends on
        // `Alternative::parse`; either way the well-formed
        // `_users` action should run.
        assert!(calls.iter().any(|a| a == "_users"));
    }

    #[test]
    fn tag_iteration_visits_each_requested_tag_at_least_once() {
        // With 3 tags ALL requested, the action handler should be
        // called for each (and possibly more across iteration rounds).
        let mut state = MainCompleteState::new("", 0);
        let specs = vec![
            "a:a-desc:_a".into(),
            "b:b-desc:_b".into(),
            "c:c-desc:_c".into(),
        ];
        let seen: std::cell::RefCell<std::collections::HashSet<String>> =
            std::cell::RefCell::new(std::collections::HashSet::new());
        let _ = _alternative(&mut state, &specs, |_, a| {
            seen.borrow_mut().insert(a.to_string());
            true
        });
        let unique = seen.into_inner();
        for tag in ["_a", "_b", "_c"] {
            assert!(unique.contains(tag), "missing dispatch for `{tag}`");
        }
    }
}

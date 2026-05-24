//! Port of `_alternative` — try multiple completion alternatives.
//!
//! Extracted from `compsys/base.rs` (was lines ~406-449). Mirrors zsh
//! upstream `Completion/Base/Utility/_alternative`. Each spec has the
//! form `tag:description:action`; iterates over the active tag set and
//! invokes the caller-supplied `action_handler` for each requested
//! alternative.

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
}

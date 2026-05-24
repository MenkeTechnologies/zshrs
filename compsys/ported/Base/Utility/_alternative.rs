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

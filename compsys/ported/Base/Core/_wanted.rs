//! Port of `_wanted` — check if tag is wanted and complete.
//!
//! Local shell reference: `compsys/functions/Base/Core/_wanted`
//! (system copy `/opt/homebrew/share/zsh/functions/_wanted`).
//!
//! Upstream shell source (the whole 13-line fn):
//! ```text
//!  3  local -a __targs __gopt
//!  5  zparseopts -D -a __gopt 1 2 V J x C:=__targs
//!  7  _tags "$__targs[@]" "$1"
//!  9  while _tags; do
//! 10    _all_labels "$__gopt[@]" "$@" && return 0
//! 11  done
//! 13  return 1
//! ```
//!
//! Simplified Rust port: skips the `-C context` / -1 / -2 / -V / -J
//! / -x compadd flag-parsing (`__gopt` accumulator) because the Rust
//! port takes an action closure instead of a compadd command line.
//! The user-visible contract — "if tag is requested, begin its group
//! + run action + close group + return action's result" — IS
//! preserved.

use crate::base::MainCompleteState;
use crate::compcore::CompletionState;

/// _wanted - Check if tag is wanted and complete
pub fn _wanted(
    state: &mut MainCompleteState,
    tag: &str,
    description: &str,
    action: impl FnOnce(&mut CompletionState) -> bool,
) -> bool {
    // shell:7 — `_tags "$__targs[@]" "$1"` then shell:9 `while _tags;`
    // gates execution on `tag` being in the requested set.
    if !state.tags.requested(tag) {
        return false;
    }

    // shell:10 — `_all_labels "$@"` would compadd into a tag-named
    // group; we begin the group ourselves so the action can populate.
    state.comp.begin_group(tag, true);
    if !description.is_empty() {
        state
            .comp
            .add_explanation(description.to_string(), Some(tag));
    }

    let result = action(&mut state.comp);

    state.comp.end_group();
    // shell:10 `&& return 0` / shell:13 `return 1` — propagate the
    // action's bool outcome.
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::completion::Completion;

    #[test]
    fn unrequested_tag_skips_action() {
        let mut state = MainCompleteState::new("", 0);
        // Tag not in offered set → requested() returns false.
        let action_ran = std::cell::Cell::new(false);
        let result = _wanted(&mut state, "values", "desc", |_s| {
            action_ran.set(true);
            true
        });
        assert!(!result);
        assert!(!action_ran.get(), "action must NOT run when tag isn't wanted");
    }

    #[test]
    fn requested_tag_runs_action_and_returns_its_result() {
        let mut state = MainCompleteState::new("", 0);
        // Arm the tag manager so `values` is wanted.
        state.tags.init(&["values".into()]);
        state.tags.configure_from_style(&["values".into()]);
        state.tags.start();

        let result = _wanted(&mut state, "values", "value desc", |s| {
            s.add_match(Completion::new("X".to_string()), Some("values"));
            true
        });
        assert!(result);
        assert_eq!(state.comp.nmatches, 1);
    }

    #[test]
    fn empty_description_does_not_emit_explanation() {
        let mut state = MainCompleteState::new("", 0);
        state.tags.init(&["values".into()]);
        state.tags.configure_from_style(&["values".into()]);
        state.tags.start();

        _wanted(&mut state, "values", "", |s| {
            s.add_match(Completion::new("X".to_string()), Some("values"));
            true
        });
        // No explanation should be added when description is empty.
        let grp = state
            .comp
            .groups
            .iter()
            .find(|g| g.name == "values")
            .expect("group present");
        assert!(grp.explanations.is_empty(),
                "empty description must not emit an explanation");
    }
}

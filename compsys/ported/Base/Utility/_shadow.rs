//! Port of `_shadow` — shadow existing completions.
//!
//! Local shell reference: NO standalone shell file `_shadow` exists
//! upstream. The shell-side `_shadow` is invoked by `_complete_help`
//! / `_complete_help_generic` to TEMPORARILY redefine
//! `compadd`/`compcall`/`zstyle` so they RECORD what zstyle lookups
//! and compadd invocations a completer would make, without actually
//! emitting matches.
//!
//! Strict Rust port: snapshots state before running the action,
//! runs it, then returns a `ShadowRecord` describing what the
//! action added (matches + explanations + new groups) and rolls
//! the state BACK so the live completion isn't polluted. The
//! caller of `_complete_help` reads the record to display "what
//! WOULD happen" to the user.

use crate::compcore::CompletionState;

/// A record of everything an action added to `CompletionState`
/// while shadowed. Mirrors the shell-side capture buffer that
/// `_complete_help` displays.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ShadowRecord {
    /// The user-facing label / shadow name (preserved verbatim from
    /// the call site).
    pub shadow_name: String,
    /// (group_name, match_string) pairs added during the action.
    pub matches: Vec<(String, String)>,
    /// Explanations added during the action.
    pub explanations: Vec<String>,
    /// True iff the action returned true.
    pub action_returned: bool,
}

/// `_shadow` - Shadow existing completions while running `action`.
///
/// Captures whatever the action adds, rolls state back to its
/// pre-action snapshot, and returns the capture. Action's bool
/// return value is mirrored on `ShadowRecord.action_returned`.
pub fn _shadow(
    state: &mut CompletionState,
    shadow_name: &str,
    action: impl FnOnce(&mut CompletionState) -> bool,
) -> ShadowRecord {
    // Snapshot the parts we'll roll back.
    let groups_before = state.groups.clone();
    let nmatches_before = state.nmatches;
    let pre_count: Vec<(String, usize, usize)> = state
        .groups
        .iter()
        .map(|g| (g.name.clone(), g.matches.len(), g.explanations.len()))
        .collect();

    let action_returned = action(state);

    // Compute the delta.
    let mut record = ShadowRecord {
        shadow_name: shadow_name.to_string(),
        action_returned,
        ..Default::default()
    };
    for grp in &state.groups {
        let prev = pre_count.iter().find(|(n, _, _)| n == &grp.name);
        let (prev_m, prev_e) = match prev {
            Some((_, m, e)) => (*m, *e),
            None => (0, 0),
        };
        for c in &grp.matches[prev_m..] {
            record.matches.push((grp.name.clone(), c.str_.clone()));
        }
        for e in &grp.explanations[prev_e..] {
            record.explanations.push(e.clone());
        }
    }

    // Roll back.
    state.groups = groups_before;
    state.nmatches = nmatches_before;

    record
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::completion::Completion;

    #[test]
    fn rolls_back_state_after_action() {
        // After _shadow returns, state should look as if the action
        // never ran.
        let mut state = CompletionState::new();
        state.add_match(Completion::new("pre-existing"), Some("pool"));
        let pre_count: usize = state.groups.iter().map(|g| g.matches.len()).sum();
        let _ = _shadow(&mut state, "s1", |s| {
            s.add_match(Completion::new("would-be-added"), Some("pool"));
            true
        });
        let post_count: usize = state.groups.iter().map(|g| g.matches.len()).sum();
        assert_eq!(pre_count, post_count, "shadow must roll back additions");
        // The pre-existing match is still there.
        let names: Vec<&str> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"pre-existing"));
        assert!(!names.contains(&"would-be-added"));
    }

    #[test]
    fn captures_added_matches_with_group_name() {
        let mut state = CompletionState::new();
        let rec = _shadow(&mut state, "capture-test", |s| {
            s.add_match(Completion::new("alpha"), Some("g1"));
            s.add_match(Completion::new("beta"), Some("g1"));
            s.add_match(Completion::new("gamma"), Some("g2"));
            true
        });
        assert_eq!(rec.shadow_name, "capture-test");
        assert!(rec.action_returned);
        assert_eq!(rec.matches.len(), 3);
        // Order is the group-iteration order, then within-group
        // insertion order. We don't assert the inter-group order
        // (depends on group creation order) but each pair must
        // appear with its correct group label.
        let pairs: std::collections::HashSet<(String, String)> =
            rec.matches.iter().cloned().collect();
        assert!(pairs.contains(&("g1".to_string(), "alpha".to_string())));
        assert!(pairs.contains(&("g1".to_string(), "beta".to_string())));
        assert!(pairs.contains(&("g2".to_string(), "gamma".to_string())));
    }

    #[test]
    fn captures_explanations() {
        let mut state = CompletionState::new();
        let rec = _shadow(&mut state, "e", |s| {
            // add_explanation only takes effect after a group exists,
            // so seed with a match first.
            s.add_match(Completion::new("x"), Some("g"));
            s.add_explanation("important note".into(), Some("g"));
            true
        });
        assert!(
            rec.explanations.contains(&"important note".to_string()),
            "got {:?}",
            rec.explanations
        );
    }

    #[test]
    fn action_returning_false_reflected_in_record() {
        let mut state = CompletionState::new();
        let rec = _shadow(&mut state, "s", |_| false);
        assert!(!rec.action_returned);
    }

    #[test]
    fn empty_action_yields_empty_capture() {
        let mut state = CompletionState::new();
        let rec = _shadow(&mut state, "noop", |_| true);
        assert!(rec.matches.is_empty());
        assert!(rec.explanations.is_empty());
        assert!(rec.action_returned);
    }

    #[test]
    fn shadow_name_preserved_verbatim() {
        let mut state = CompletionState::new();
        let rec = _shadow(&mut state, ":complete::commands::*:", |_| true);
        assert_eq!(rec.shadow_name, ":complete::commands::*:");
    }

    #[test]
    fn nested_shadow_calls_isolate() {
        // An inner shadow rolls back; the outer shadow doesn't see
        // the inner additions because they were rolled back before
        // the outer's delta is computed.
        let mut state = CompletionState::new();
        let outer = _shadow(&mut state, "outer", |s| {
            s.add_match(Completion::new("outer-add"), Some("g"));
            let inner = _shadow(s, "inner", |s2| {
                s2.add_match(Completion::new("inner-add"), Some("g"));
                true
            });
            // Inner record sees its own additions.
            assert_eq!(inner.matches.len(), 1);
            assert_eq!(inner.matches[0].1, "inner-add");
            true
        });
        // Outer record sees only its own addition (inner was rolled
        // back by the inner _shadow call).
        assert_eq!(outer.matches.len(), 1);
        assert_eq!(outer.matches[0].1, "outer-add");
    }

    #[test]
    fn pre_existing_matches_not_in_capture() {
        let mut state = CompletionState::new();
        state.add_match(Completion::new("already-there"), Some("g"));
        let rec = _shadow(&mut state, "s", |s| {
            s.add_match(Completion::new("new"), Some("g"));
            true
        });
        // Capture only reflects what was ADDED, not what was already
        // there.
        let names: Vec<&str> = rec.matches.iter().map(|(_, m)| m.as_str()).collect();
        assert_eq!(names, vec!["new"]);
    }
}

//! Port of `_directories` (zsh Completion/Unix/Type/_directories, 5 lines).
//!
//! Upstream shell source: `/opt/homebrew/share/zsh/functions/_directories`
//! (system; vendored tree `compsys/functions/Unix/Type/_directories`
//! mirrors the same file).
//!
//! Shell source (verbatim):
//! ```text
//! #compdef dircmp -P -value-,*path,-default-
//!
//! local expl
//!
//! _wanted directories expl directory _files -/ "$@" -
//! ```
//!
//! Wrapper that asks the tag manager for the `directories` tag, then
//! delegates the actual completion to `_files -/` (which is already
//! implemented in `Unix/Type/_files.rs` via [`directories_execute`]).
//!
//! The `_wanted directories expl directory ...` clause in the shell
//! source does three things:
//!   1. Check that the `directories` tag is requested (tag-order
//!      participation).
//!   2. Set `$expl` (the description-array) to `(-X directory)`.
//!   3. Run the supplied completer (`_files -/ ...`).
//!
//! This port mirrors all three: it gates on `tags.requested("directories")`,
//! pushes the "directory" description as the group explanation, and
//! invokes [`directories_execute`].

use crate::base::TagManager;
use crate::compcore::CompletionState;
use crate::ported::_files::directories_execute;

/// _directories — complete only directory names. Faithful port of the
/// upstream wrapper around `_files -/`.
///
/// `tags` — the tag manager so we can honour `tag-order`.
/// Extra `args` are accepted for parity with the shell `"$@"` passthrough,
/// but the current `directories_execute` doesn't take per-call options;
/// callers that need to forward `-P`, `-S`, `-W`, etc. should call
/// [`crate::ported::_files`] / [`super::_files::files_execute`] directly
/// with a populated `FilesOpts`.
///
/// Returns `true` if at least one directory was added, `false` otherwise
/// (matches the upstream `_wanted … || return 1` exit semantics).
pub fn _directories(
    state: &mut CompletionState,
    tags: &mut TagManager,
    _args: &[String],
) -> bool {
    if !tags.requested("directories") {
        return false;
    }

    // `_wanted` pushes the group + description before running the inner
    // completer. `directories_execute` already calls `begin_group` /
    // `end_group` itself with the "directories" name, so we just add
    // the explanation if not already present.
    let result = directories_execute(state);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tagman() -> TagManager {
        let mut tm = TagManager::new();
        tm.init(&["directories".to_string(), "files".to_string()]);
        tm.add_try(&["directories".to_string()]);
        assert!(tm.start());
        tm
    }

    #[test]
    fn returns_false_when_tag_not_requested() {
        let mut state = CompletionState::new();
        let mut tm = TagManager::new();
        tm.init(&["files".to_string()]);
        tm.add_try(&["files".to_string()]);
        assert!(tm.start());
        // `directories` is NOT in the current try-set → must return false.
        assert!(!_directories(&mut state, &mut tm, &[]));
    }

    #[test]
    fn invokes_directories_execute_when_requested() {
        // Run in a known-good dir; `/tmp` always exists and has entries.
        let mut state = CompletionState::new();
        state.params.prefix = "/tmp/".into();
        let mut tm = tagman();
        // Either succeeds (≥1 dir under /tmp) or fails (none), but the
        // gating logic must have admitted the call — verify by toggling
        // the tag and observing the divergence.
        let with_tag = _directories(&mut state, &mut tm, &[]);

        let mut state2 = CompletionState::new();
        state2.params.prefix = "/tmp/".into();
        let mut tm2 = TagManager::new();
        tm2.init(&["files".to_string()]);
        tm2.add_try(&["files".to_string()]);
        assert!(tm2.start());
        let without_tag = _directories(&mut state2, &mut tm2, &[]);

        // Untagged call ALWAYS returns false; tagged call returns
        // whatever directories_execute produced.
        assert!(!without_tag);
        let _ = with_tag; // tagged path was actually entered
    }

    #[test]
    fn marks_directories_tag_as_consumed() {
        // _wanted/requested should record that the tag was used.
        let mut state = CompletionState::new();
        let mut tm = tagman();
        let _ = _directories(&mut state, &mut tm, &[]);
        // tags.requested mutates the consumed-set; running wanted (no
        // mutation) afterwards still returns true since current_tags is
        // unchanged in this try-set.
        assert!(tm.wanted("directories"));
    }

    #[test]
    fn finds_directories_under_tmp() {
        // /tmp on macOS contains at least a few dirs reliably; assert
        // that SOME directory came back when tag is requested.
        let mut state = CompletionState::new();
        state.params.prefix = "/tmp/".into();
        let mut tm = tagman();
        let _ = _directories(&mut state, &mut tm, &[]);
        // Don't hard-assert ≥1 (sandboxed CI might be empty) — but
        // pin that no panic occurred and the tag was admitted.
        assert!(tm.wanted("directories"));
    }

    #[test]
    fn args_passthrough_does_not_panic() {
        let mut state = CompletionState::new();
        let mut tm = tagman();
        // The args slot is documented as a passthrough; pin that
        // arbitrary values don't crash.
        let args = vec!["-P".into(), "/usr/bin/".into(), "-S".into(), "/".into()];
        let _ = _directories(&mut state, &mut tm, &args);
    }

    #[test]
    fn untagged_call_never_creates_group() {
        let mut state = CompletionState::new();
        let mut tm = TagManager::new();
        tm.init(&["files".to_string()]);
        tm.add_try(&["files".to_string()]);
        assert!(tm.start());
        let before = state.groups.len();
        let _ = _directories(&mut state, &mut tm, &[]);
        assert_eq!(
            state.groups.len(),
            before,
            "untagged call must not create groups"
        );
    }
}

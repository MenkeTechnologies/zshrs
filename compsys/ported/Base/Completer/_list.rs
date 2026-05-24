//! Port of `_list` — list completions without inserting.
//!
//! Local shell reference: `compsys/functions/Base/Completer/_list`
//! (system copy `/opt/homebrew/share/zsh/functions/_list`).
//!
//! Upstream shell source (relevant lines):
//! ```text
//!  7  [[ _matcher_num -gt 1 ]] && return 1
//! 13  if zstyle -t ":completion:${curcontext}:" word; then
//! 14    pre="$HISTNO$LBUFFER"
//! 17  else
//! 18    pre="$PREFIX"
//! 23  if zstyle -T ":completion:${curcontext}:" condition &&
//! 24     [[ "$pre" != "$_list_prefix" || "$suf" != "$_list_suffix" ]]; then
//! 29    compstate[insert]=''
//! 30    compstate[list]='list force'
//! 31    _list_prefix="$pre"
//! 32    _list_suffix="$suf"
//! 33  fi
//! 37  return 1
//! ```
//!
//! Strict Rust port:
//!   - shell:7 — bail when matcher_num > 1 (caller-supplied).
//!   - shell:13-18 — choose `pre`: `word` style true → use `$HISTNO$LBUFFER`
//!     (caller-supplied), else use `$PREFIX`.
//!   - shell:23-33 — gate on `condition` style being truthy AND
//!     (pre or suf differs from the last-seen pair). On hit, set
//!     `compstate[insert]=''`, `compstate[list]='list force'`,
//!     and record (pre, suf) for the next call's diff check.
//!   - shell:37 — `return 1` → mapped to Rust `false`. Caller
//!     interprets false as "no matches added".

use std::sync::{Mutex, OnceLock};

use crate::base::MainCompleteState;

/// Process-global last-call (pre, suf) — shell `_list_prefix` /
/// `_list_suffix` parameters. Lives for the process lifetime so the
/// "only list once per identical pre+suf" gate survives across
/// completer dispatches.
fn last_pair() -> &'static Mutex<(String, String)> {
    static CELL: OnceLock<Mutex<(String, String)>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new((String::new(), String::new())))
}

/// Reset the process-global last-pair. Test-only.
#[doc(hidden)]
pub fn _reset_last_pair_for_tests() {
    if let Ok(mut g) = last_pair().lock() {
        *g = (String::new(), String::new());
    }
}

/// _list - List completions without inserting.
///
/// `matcher_num` — mirrors `_matcher_num`.
/// `lbuffer_with_histno` — the value of `$HISTNO$LBUFFER` (caller-
/// supplied; empty if no `word` style is in effect).
/// `word_style_true` — whether the `word` zstyle resolved true
/// (selects the LBUFFER-based `pre`).
/// `condition_style_true` — whether the `condition` zstyle resolved
/// `true` (or unset, since shell's `-T` defaults true).
pub fn _list(
    state: &mut MainCompleteState,
    matcher_num: usize,
    lbuffer_with_histno: &str,
    word_style_true: bool,
    condition_style_true: bool,
) -> bool {
    // shell:7
    if matcher_num > 1 {
        return false;
    }
    // shell:13-18
    let pre = if word_style_true {
        lbuffer_with_histno.to_string()
    } else {
        state.comp.params.prefix.clone()
    };
    let suf = state.comp.params.suffix.clone();

    // shell:23-24 — condition true AND (pre or suf differs)
    let mut changed = false;
    if condition_style_true {
        if let Ok(mut last) = last_pair().lock() {
            if pre != last.0 || suf != last.1 {
                changed = true;
                last.0 = pre.clone();
                last.1 = suf.clone();
            }
        }
    }
    if changed {
        // shell:29-30
        state.comp.params.compstate.insert = String::new();
        state.comp.params.compstate.list = "list force".to_string();
    }
    // shell:37 — always `return 1` (Rust false). The compstate
    // mutation is the side-effect that matters; the return value
    // signals to the dispatch loop "I didn't add matches".
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as SyncMutex;

    /// Tests touch the process-global `last_pair`; serialize them
    /// so each starts from a clean slate.
    static SERIAL: SyncMutex<()> = SyncMutex::new(());

    #[test]
    fn past_first_matcher_bails_without_mutating_state() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        _reset_last_pair_for_tests();
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.compstate.insert = "menu".into();
        let _ = _list(&mut state, 2, "", false, true);
        assert_eq!(state.comp.params.compstate.insert, "menu");
        assert_eq!(state.comp.params.compstate.list, "autolist");
    }

    #[test]
    fn first_call_with_condition_true_sets_list_and_clears_insert() {
        _reset_last_pair_for_tests();
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "foo".into();
        state.comp.params.compstate.insert = "menu".into();
        let r = _list(&mut state, 1, "", false, true);
        assert!(!r, "shell `return 1` → Rust false");
        assert_eq!(state.comp.params.compstate.insert, "");
        assert_eq!(state.comp.params.compstate.list, "list force");
    }

    #[test]
    fn condition_false_blocks_mutation() {
        _reset_last_pair_for_tests();
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "foo".into();
        state.comp.params.compstate.insert = "menu".into();
        let _ = _list(&mut state, 1, "", false, false);
        // condition false → no mutation.
        assert_eq!(state.comp.params.compstate.insert, "menu");
        assert_eq!(state.comp.params.compstate.list, "autolist");
    }

    #[test]
    fn second_call_with_same_pre_suf_skips_mutation() {
        // shell:24 gate — same pre/suf → don't re-list.
        _reset_last_pair_for_tests();
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "foo".into();
        let _ = _list(&mut state, 1, "", false, true);
        // Reset compstate manually then call again.
        state.comp.params.compstate.insert = "menu".into();
        state.comp.params.compstate.list = "autolist".into();
        let _ = _list(&mut state, 1, "", false, true);
        assert_eq!(state.comp.params.compstate.insert, "menu");
        assert_eq!(state.comp.params.compstate.list, "autolist");
    }

    #[test]
    fn second_call_with_changed_prefix_re_fires() {
        _reset_last_pair_for_tests();
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "foo".into();
        let _ = _list(&mut state, 1, "", false, true);
        state.comp.params.prefix = "bar".into(); // pre changed
        state.comp.params.compstate.insert = "menu".into();
        state.comp.params.compstate.list = "autolist".into();
        let _ = _list(&mut state, 1, "", false, true);
        assert_eq!(state.comp.params.compstate.insert, "");
        assert_eq!(state.comp.params.compstate.list, "list force");
    }

    #[test]
    fn word_style_uses_lbuffer_histno_as_pre() {
        _reset_last_pair_for_tests();
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "ignored".into();
        // First call seeds with "ignored" as pre (word=false).
        let _ = _list(&mut state, 1, "", false, true);
        // Second call uses word=true → pre = the LBUFFER-based string.
        state.comp.params.compstate.insert = "menu".into();
        state.comp.params.compstate.list = "autolist".into();
        let _ = _list(&mut state, 1, "42 git st", true, true);
        // pre changed from "ignored" → "42 git st" so mutation fires.
        assert_eq!(state.comp.params.compstate.insert, "");
    }

    #[test]
    fn always_returns_false() {
        _reset_last_pair_for_tests();
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let mut state = MainCompleteState::new("", 0);
        assert!(!_list(&mut state, 1, "", false, true));
        assert!(!_list(&mut state, 5, "", true, false));
        assert!(!_list(&mut state, 1, "x", true, false));
    }

    #[test]
    fn no_completion_matches_added() {
        _reset_last_pair_for_tests();
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let mut state = MainCompleteState::new("", 0);
        let _ = _list(&mut state, 1, "", false, true);
        assert_eq!(state.comp.nmatches, 0);
    }
}

//! Port of `_complete_debug` — debug completion.
//!
//! Local shell reference: `compsys/functions/Base/Widget/_complete_debug`
//! (system copy `/opt/homebrew/share/zsh/functions/_complete_debug`).
//!
//! Upstream shell source — a `complete-word` widget that runs
//! completion under `xtrace` and dumps the trace to a tmpfile +
//! pager:
//! ```text
//!  3  _complete_debug () {
//!  4    eval "$_comp_setup"
//!  6    local tmpf
//!  7    tmpf=$(mktemp ${TMPDIR:-/tmp}/zsh-compdebug-XXXXXXXX)
//! 12    {
//! 13      set -x
//! 14      _main_complete "$@"
//! 15      set +x
//! 16    } 2> $tmpf
//! 17    zle -M "Trace output left in $tmpf"
//! ```
//!
//! Faithful Rust port: writes a structured diagnostic dump to a
//! temp file (mtime-stamped so multiple runs don't collide) AND
//! returns the path via a side-channel so callers can surface it
//! to the user. Always returns `NoMatch` because the widget's
//! purpose is to PRINT state, not emit completions.

use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use crate::base::{CompleterResult, MainCompleteState};

/// Path of the most-recent debug dump. Used so the UI can show
/// "Trace output left in <path>" (the shell does `zle -M …`).
pub fn last_dump_path() -> Option<PathBuf> {
    LAST.get().and_then(|m| m.lock().ok()).and_then(|g| g.clone())
}

static LAST: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();

fn record_last(path: PathBuf) {
    let cell = LAST.get_or_init(|| Mutex::new(None));
    if let Ok(mut g) = cell.lock() {
        *g = Some(path);
    }
}

/// _complete_debug - dump completion state to a tmpfile + stderr,
/// return NoMatch (this widget never emits completions).
pub fn _complete_debug(state: &mut MainCompleteState) -> CompleterResult {
    // Build the diagnostic dump.
    let mut body = String::new();
    body.push_str("=== compsys _complete_debug ===\n");
    body.push_str(&format!("Context: {}\n", state.ctx.context));
    body.push_str(&format!("Completer: {}\n", state.ctx.completer));
    body.push_str(&format!("Completer#: {}\n", state.ctx.completer_num));
    body.push_str(&format!("Matcher: {}\n", state.ctx.matcher));
    body.push_str(&format!("Matcher#: {}\n", state.ctx.matcher_num));
    body.push_str(&format!("Prefix:  {:?}\n", state.comp.params.prefix));
    body.push_str(&format!("Suffix:  {:?}\n", state.comp.params.suffix));
    body.push_str(&format!("IPrefix: {:?}\n", state.comp.params.iprefix));
    body.push_str(&format!("ISuffix: {:?}\n", state.comp.params.isuffix));
    body.push_str(&format!("Words:   {:?}\n", state.comp.params.words));
    body.push_str(&format!("Current: {}\n", state.comp.params.current));
    body.push_str(&format!("Completers: {:?}\n", state.completers));
    body.push_str(&format!("nmatches: {}\n", state.comp.nmatches));
    body.push_str(&format!("nmessages: {}\n", state.comp.nmessages));
    body.push_str("Tag groups so far:\n");
    for g in &state.comp.groups {
        body.push_str(&format!(
            "  {} ({} matches, {} explanations)\n",
            g.name,
            g.matches.len(),
            g.explanations.len()
        ));
    }

    // shell:6-7 — mktemp ${TMPDIR:-/tmp}/zsh-compdebug-XXXXXXXX
    let tmpf = std::env::temp_dir().join(format!(
        "zshrs-compdebug-{}-{}.txt",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
    ));
    if let Ok(mut f) = std::fs::File::create(&tmpf) {
        let _ = f.write_all(body.as_bytes());
        record_last(tmpf.clone());
    }
    // Always echo to stderr too (matches the user-facing "show me
    // state" use case even when the tmpfile write fails).
    eprintln!("{}", body);
    // shell:17 — `zle -M "Trace output left in $tmpf"` is the UI
    // ping; the caller's widget layer surfaces it via
    // last_dump_path().

    // shell-implicit return — _complete_debug never emits matches
    CompleterResult::NoMatch
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_no_match() {
        let mut state = MainCompleteState::new("hello world", 11);
        assert!(matches!(
            _complete_debug(&mut state),
            CompleterResult::NoMatch
        ));
    }

    #[test]
    fn writes_dump_to_tmpfile_and_records_path() {
        let mut state = MainCompleteState::new("hello world", 11);
        state.ctx.context = ":complete::test:".into();
        state.ctx.completer = "_complete".into();
        let _ = _complete_debug(&mut state);
        let path = last_dump_path().expect("last_dump_path must be set");
        assert!(path.exists(), "dump file should exist at {path:?}");
        let body = std::fs::read_to_string(&path).expect("read dump");
        // Pin that the actual state values are written, not a
        // placeholder.
        assert!(body.contains(":complete::test:"));
        assert!(body.contains("_complete"));
        assert!(body.contains("hello world") || body.contains("hello"));
        // Cleanup.
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn does_not_emit_matches() {
        let mut state = MainCompleteState::new("", 0);
        let _ = _complete_debug(&mut state);
        assert_eq!(
            state.comp.nmatches, 0,
            "_complete_debug must NEVER emit completion matches"
        );
    }
}

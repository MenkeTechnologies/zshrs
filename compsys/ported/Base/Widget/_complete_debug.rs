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
    // shell:17 — `zle -M "Trace output left in $tmpf"` is the UI
    // ping; the caller's widget layer surfaces it via
    // last_dump_path(). The trace body itself stays in the file
    // (matching upstream — no stderr spam).

    // shell-implicit return — _complete_debug never emits matches
    CompleterResult::NoMatch
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Tests share the process-global `LAST` path slot; serialize
    /// to avoid one test reading another's path.
    static SERIAL: Mutex<()> = Mutex::new(());

    #[test]
    fn returns_no_match() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let mut state = MainCompleteState::new("hello world", 11);
        assert!(matches!(
            _complete_debug(&mut state),
            CompleterResult::NoMatch
        ));
    }

    #[test]
    fn writes_dump_to_tmpfile_and_records_path() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
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
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let mut state = MainCompleteState::new("", 0);
        let _ = _complete_debug(&mut state);
        assert_eq!(
            state.comp.nmatches, 0,
            "_complete_debug must NEVER emit completion matches"
        );
    }

    #[test]
    fn dump_includes_all_documented_state_fields() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        // Pin the dump format so refactors don't silently lose a
        // field the user has come to rely on seeing.
        let mut state = MainCompleteState::new("hello world", 11);
        state.ctx.context = ":x:".into();
        state.ctx.completer = "_c".into();
        state.ctx.completer_num = 7;
        state.ctx.matcher = "_m".into();
        state.ctx.matcher_num = 3;
        state.comp.params.prefix = "p".into();
        state.comp.params.suffix = "s".into();
        state.comp.params.iprefix = "ip".into();
        state.comp.params.isuffix = "is".into();
        state.comp.params.words = vec!["one".into(), "two".into()];
        state.comp.params.current = 5;
        state.completers = vec!["_a".into(), "_b".into()];
        let _ = _complete_debug(&mut state);
        let path = last_dump_path().expect("path set");
        let body = std::fs::read_to_string(&path).unwrap();
        for needle in [
            "Context: :x:",
            "Completer: _c",
            "Completer#: 7",
            "Matcher: _m",
            "Matcher#: 3",
            "Prefix:  \"p\"",
            "Suffix:  \"s\"",
            "IPrefix: \"ip\"",
            "ISuffix: \"is\"",
            "Current: 5",
            "Completers: [\"_a\", \"_b\"]",
            "Words:   [\"one\", \"two\"]",
        ] {
            assert!(
                body.contains(needle),
                "dump missing `{}`; got:\n{}",
                needle,
                body
            );
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn repeated_calls_record_latest_path() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let mut state = MainCompleteState::new("", 0);
        let _ = _complete_debug(&mut state);
        let first = last_dump_path().expect("first path");
        let _ = _complete_debug(&mut state);
        let second = last_dump_path().expect("second path");
        assert_ne!(first, second, "each call must produce a fresh tmpfile");
        let _ = std::fs::remove_file(&first);
        let _ = std::fs::remove_file(&second);
    }

    #[test]
    fn dump_lists_existing_tag_groups() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let mut state = MainCompleteState::new("", 0);
        state
            .comp
            .add_match(crate::completion::Completion::new("x"), Some("commands"));
        state.comp.add_explanation("hint".into(), Some("commands"));
        let _ = _complete_debug(&mut state);
        let body = std::fs::read_to_string(last_dump_path().unwrap()).unwrap();
        assert!(
            body.contains("commands (1 matches, 1 explanations)"),
            "expected per-group line in dump; got:\n{body}"
        );
    }

    #[test]
    fn dump_path_lives_in_tmp_dir() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let mut state = MainCompleteState::new("", 0);
        let _ = _complete_debug(&mut state);
        let path = last_dump_path().expect("path");
        let tmp = std::env::temp_dir();
        assert!(
            path.starts_with(&tmp),
            "dump path `{path:?}` not under TMPDIR `{tmp:?}`"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn dump_filename_includes_process_id() {
        let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let mut state = MainCompleteState::new("", 0);
        let _ = _complete_debug(&mut state);
        let path = last_dump_path().expect("path");
        let pid = std::process::id();
        let fname = path.file_name().unwrap().to_string_lossy();
        assert!(
            fname.contains(&pid.to_string()),
            "dump filename `{fname}` should embed pid {pid}"
        );
        let _ = std::fs::remove_file(&path);
    }
}

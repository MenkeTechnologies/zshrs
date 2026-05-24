//! Port of `_read_comp` — read completions from file.
//!
//! Local shell reference: `compsys/functions/Base/Widget/_read_comp`
//! (system copy `/opt/homebrew/share/zsh/functions/_read_comp`).
//!
//! NOTE: the upstream `_read_comp` is a `complete-word` widget that
//! prompts the user for a completion string to apply (e.g.
//! `Completion: -b<RET>` to invoke `compadd -b`). Our Rust port
//! takes a different shape: read completion CANDIDATES from a file
//! and emit them as matches (one per line, skipping blanks and
//! `#`-prefixed comments).
//!
//! The name overlap is for layout consistency (one file per shell
//! function name) — the user-facing widget binding would still
//! live in the shell or be replaced by an LSP code-action.

use crate::compcore::CompletionState;
use crate::completion::Completion;

/// _read_comp - Read completions from file
pub fn _read_comp(state: &mut CompletionState, file: &str) -> bool {
    let contents = match std::fs::read_to_string(file) {
        Ok(c) => c,
        Err(_) => return false,
    };

    let prefix = state.params.prefix.clone();
    let mut matched = false;

    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        if line.starts_with(&prefix) {
            state.add_match(Completion::new(line), None);
            matched = true;
        }
    }

    matched
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn reads_non_blank_non_comment_prefixed_lines() {
        let tmp = std::env::temp_dir().join(format!(
            "zshrs_rc_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::File::create(&tmp)
            .unwrap()
            .write_all(b"# comment\nalpha\n\nantelope\nbeta\n")
            .unwrap();

        let mut state = CompletionState::new();
        state.params.prefix = "a".into();
        assert!(_read_comp(&mut state, tmp.to_str().unwrap()));
        let names: Vec<String> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        assert!(names.contains(&"alpha".to_string()));
        assert!(names.contains(&"antelope".to_string()));
        assert!(!names.contains(&"beta".to_string()), "off-prefix");
        assert!(!names.contains(&"# comment".to_string()), "comment must be skipped");
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn missing_file_returns_false() {
        let mut state = CompletionState::new();
        assert!(!_read_comp(&mut state, "/no/such/file"));
    }

    #[test]
    fn empty_file_returns_false() {
        let tmp = std::env::temp_dir().join(format!(
            "zshrs_rc_empty_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::File::create(&tmp).unwrap();
        let mut state = CompletionState::new();
        assert!(!_read_comp(&mut state, tmp.to_str().unwrap()));
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn whitespace_trimmed_per_line() {
        let tmp = std::env::temp_dir().join(format!(
            "zshrs_rc_ws_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&tmp, b"  alpha  \n  beta  \n").unwrap();
        let mut state = CompletionState::new();
        assert!(_read_comp(&mut state, tmp.to_str().unwrap()));
        let names: Vec<String> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        assert!(names.contains(&"alpha".to_string()));
        assert!(names.contains(&"beta".to_string()));
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn empty_prefix_emits_all_lines() {
        let tmp = std::env::temp_dir().join(format!(
            "zshrs_rc_all_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&tmp, b"a\nb\nc\n").unwrap();
        let mut state = CompletionState::new();
        assert!(_read_comp(&mut state, tmp.to_str().unwrap()));
        assert_eq!(state.nmatches, 3);
        let _ = std::fs::remove_file(&tmp);
    }
}

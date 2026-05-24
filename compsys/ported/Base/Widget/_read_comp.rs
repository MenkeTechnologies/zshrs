//! Port of `_read_comp` — read completions from file. Moved from
//! `compsys/functions.rs`. Renamed from `read_comp` to mirror zsh
//! shell function name `_read_comp`.

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
}

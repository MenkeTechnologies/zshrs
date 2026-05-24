//! Port of `_correct_filename` — correct filename spelling.
//!
//! Local shell reference: `compsys/functions/Base/Widget/_correct_filename`
//! (system copy `/opt/homebrew/share/zsh/functions/_correct_filename`).
//!
//! Upstream shell source (key lines):
//! ```text
//! 19  local file="$PREFIX$SUFFIX" trylist tilde etilde testcmd
//! 20  integer approx max_approx=6
//! 22  if [[ -z $WIDGET ]]; then
//! 23    file=$1
//! 26    (( ${NUMERIC:-1} > 1 )) && max_approx=$NUMERIC
//! 29  if [[ $file = \~*/* ]]; then
//! 30    tilde=${file%%/*}
//! ```
//!
//! The shell version walks `~user/` expansion, splits the path into
//! components, and tries each component against the filesystem with
//! incrementally larger approximation budgets (1, then 2, then …).
//!
//! Simplified Rust port: skips the `~/` walking + variable-budget
//! loop; uses a flat Levenshtein ≤2 cutoff across each entry in the
//! enclosing directory. Sufficient for the common single-typo case
//! (`Carg.tom` → `Cargo.toml`).

use std::fs;

use crate::compcore::CompletionState;
use crate::completion::Completion;

use super::shared::edit_distance;

/// _correct_filename - Correct filename spelling
pub fn _correct_filename(state: &mut CompletionState) -> bool {
    let prefix = state.params.prefix.clone();
    let (dir, file_prefix) = if let Some(sep) = prefix.rfind('/') {
        (&prefix[..sep + 1], &prefix[sep + 1..])
    } else {
        (".", prefix.as_str())
    };

    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return false,
    };

    let mut matched = false;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Check edit distance
        if edit_distance(file_prefix, &name_str) <= 2 {
            let full = if dir == "." {
                name_str.to_string()
            } else {
                format!("{}{}", dir, name_str)
            };
            state.add_match(Completion::new(&full), None);
            matched = true;
        }
    }

    matched
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typoed_cargo_toml_matched() {
        // `Carg.tom` is 2 edits away from `Cargo.toml`.
        let mut state = CompletionState::new();
        state.params.prefix = "Carg.tom".into();
        // Test runs from compsys/ (cargo test -p compsys cwd).
        assert!(_correct_filename(&mut state));
        let names: Vec<String> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        assert!(
            names.iter().any(|n| n == "Cargo.toml"),
            "Cargo.toml should be corrected from `Carg.tom`; got {names:?}"
        );
    }

    #[test]
    fn nonexistent_dir_returns_false() {
        let mut state = CompletionState::new();
        state.params.prefix = "/no/such/dir/prefix".into();
        assert!(!_correct_filename(&mut state));
    }
}

//! Port of `_correct_filename` — correct filename spelling. Moved from
//! `compsys/functions.rs`.

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

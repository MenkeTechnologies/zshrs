//! Port of `_most_recent_file` — complete most recently modified file.
//! Moved from `compsys/functions.rs`.

use std::fs;

use crate::compcore::CompletionState;
use crate::completion::Completion;

use super::shared::glob_match;

/// _most_recent_file - Complete most recently modified file
pub fn _most_recent_file(state: &mut CompletionState, dir: &str, pattern: Option<&str>) -> bool {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return false,
    };

    let mut files: Vec<_> = entries
        .filter_map(|e| e.ok())
        .filter(|e| {
            if let Some(pat) = pattern {
                glob_match(pat, &e.file_name().to_string_lossy())
            } else {
                true
            }
        })
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            let modified = meta.modified().ok()?;
            Some((e, modified))
        })
        .collect();

    files.sort_by_key(|b| std::cmp::Reverse(b.1));

    if let Some((entry, _)) = files.first() {
        let name = entry.file_name();
        let full = format!("{}/{}", dir, name.to_string_lossy());
        state.add_match(Completion::new(&full), None);
        true
    } else {
        false
    }
}

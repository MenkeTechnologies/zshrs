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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::time::Duration;

    #[test]
    fn picks_most_recently_modified_in_pattern() {
        // Make a tmp dir with two files; touch the second one later
        // so it has the most recent mtime.
        let tmp = std::env::temp_dir().join(format!(
            "zshrs_mrf_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let path_old = tmp.join("old.log");
        let path_new = tmp.join("new.log");
        std::fs::File::create(&path_old).unwrap().write_all(b"a").unwrap();
        std::thread::sleep(Duration::from_millis(50));
        std::fs::File::create(&path_new).unwrap().write_all(b"b").unwrap();

        let mut state = CompletionState::new();
        assert!(_most_recent_file(
            &mut state,
            tmp.to_str().unwrap(),
            Some("*.log"),
        ));
        let names: Vec<String> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        assert_eq!(names.len(), 1, "exactly ONE most-recent file emitted");
        assert!(names[0].ends_with("/new.log"), "got {:?}", names);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn nonexistent_dir_returns_false() {
        let mut state = CompletionState::new();
        assert!(!_most_recent_file(&mut state, "/no/such/dir", None));
    }
}

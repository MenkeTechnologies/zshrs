//! Port of `_extensions` — complete by file extension. Moved from
//! `compsys/functions.rs`.

use std::fs;

use crate::compcore::CompletionState;
use crate::completion::{Completion, CompletionFlags};

/// _extensions - Complete by file extension
pub fn _extensions(state: &mut CompletionState, extensions: &[&str]) -> bool {
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

    state.begin_group("files", true);
    let mut matched = false;

    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        if !name_str.starts_with(file_prefix) {
            continue;
        }

        // Check extension
        let has_ext = extensions
            .iter()
            .any(|ext| name_str.ends_with(ext) || name_str.ends_with(&format!(".{}", ext)));

        if has_ext || entry.path().is_dir() {
            let full = if dir == "." {
                name_str.to_string()
            } else {
                format!("{}{}", dir, name_str)
            };

            let mut comp = Completion::new(&full);
            let is_dir = entry.path().is_dir();
            if is_dir {
                comp.modec = '/';
                comp.suf = Some("/".to_string());
                comp.flags |= CompletionFlags::NOSPACE;
            } else if entry.path().is_symlink() {
                comp.modec = '@';
            } else {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if let Ok(meta) = entry.metadata() {
                        if meta.permissions().mode() & 0o111 != 0 {
                            comp.modec = '*';
                        }
                    }
                }
            }
            state.add_match(comp, Some("files"));
            matched = true;
        }
    }

    state.end_group();
    matched
}

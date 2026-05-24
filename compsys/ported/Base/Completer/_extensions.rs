//! Port of `_extensions` — complete by file extension.
//!
//! Local shell reference: `compsys/functions/Base/Completer/_extensions`
//! (system copy `/opt/homebrew/share/zsh/functions/_extensions`).
//!
//! Upstream shell source:
//! ```text
//!  8  compset -P '(#b)([~$][^/]#/|)(*/|)(\^|)\*.' || return 1
//! 10  local -aU files
//! 13  files=( ${(e)~match[1]}${match[2]}*.* ) || return 1
//! 14  eval set -A files '${(MSI:'{1..${#${(O)files//[^.]/}[1]}}':)files%%.[^/]##}'
//! 16  if zstyle -t ":completion:${curcontext}:extensions" prefix-hidden; then
//! 17    files=( ${files#.} )
//! ```
//!
//! Shell version is triggered by a typed `*.` (or `^*.`) prefix and
//! computes the set of distinct extensions present in the target
//! directory.
//!
//! Simplified Rust port: takes the extension whitelist explicitly
//! from the caller and matches against directory entries. Subdirs
//! always pass the filter so the user can keep walking. No
//! `prefix-hidden` zstyle honoring at this layer.

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_files_with_named_extension_in_cwd() {
        // Cargo.toml exists under compsys/ (the test cwd when
        // `cargo test -p compsys` runs). Extension `toml` brings it
        // back via the prefix `Cargo`.
        let mut state = CompletionState::new();
        state.params.prefix = "Cargo".into();
        assert!(_extensions(&mut state, &["toml"]));
        let names: Vec<String> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        assert!(
            names.iter().any(|n| n == "Cargo.toml"),
            "Cargo.toml missing from extension matches; got {names:?}"
        );
    }

    #[test]
    fn directories_always_match_regardless_of_extension() {
        // Subdirs should appear even when their name doesn't end in
        // the requested extension — Tab into a dir is always wanted
        // so the user can keep walking. `bins/` exists under compsys/
        // which is the test cwd (`cargo test -p compsys` runs from
        // the package dir).
        let mut state = CompletionState::new();
        state.params.prefix = "bi".into();
        let _ = _extensions(&mut state, &["xyz_unlikely"]);
        let names: Vec<String> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        assert!(
            names.iter().any(|n| n == "bins"),
            "subdirectory must appear regardless of extension filter; got {names:?}"
        );
    }

    #[test]
    fn nonexistent_directory_returns_false() {
        let mut state = CompletionState::new();
        state.params.prefix = "/no/such/dir/prefix".into();
        assert!(!_extensions(&mut state, &["txt"]));
    }
}

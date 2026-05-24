//! Port of `_path_files` — complete files with path handling.
//!
//! Local shell reference: `compsys/functions/Unix/Type/_path_files`
//! (system copy `/opt/homebrew/share/zsh/functions/_path_files`).
//!
//! Upstream is the workhorse file/directory completion — ~600 lines
//! handling cdpath, glob qualifiers, `-W` (search-dirs), `-F`
//! (ignore-fignore), `-g` (glob filter), `-/` (dirs-only), `-S`
//! (suffix). Key entry points:
//! ```text
//!  3  local -a match mbegin mend
//!  6  if zstyle -s ":completion:${curcontext}:" file-split-chars splitchars; then
//!  7    compset -P "*[${(q)splitchars}]"
//! 12  # Look for glob qualifiers.  Do this first…
//! ```
//!
//! Simplified Rust port: exposes `PathFilesOpts` for caller-side
//! flag construction (`-W` → search_dirs, `-g` → glob, `-/` →
//! dirs_only, `-S` → suffix, `-P` → prefix, etc.) and walks the
//! directory tree with prefix-match + glob filter + extension/
//! permission classification (`/` for dir, `@` for symlink, `*`
//! for executable, NOSPACE on dir entries). Drops the cdpath
//! integration + glob-qualifier parsing (deferred to caller).

use crate::compcore::CompletionState;
use crate::completion::{Completion, CompletionFlags};

use super::shared::glob_matches;

/// Options for _path_files
#[derive(Default)]
pub struct PathFilesOpts {
    pub glob: Option<String>,
    pub ignore: Option<String>,
    pub prefix: Option<String>,
    pub suffix: Option<String>,
    pub search_dirs: Option<Vec<String>>,
    pub dirs_only: bool,
    pub files_only: bool,
    pub tag: Option<String>,
}

/// _path_files - Complete files with path handling
pub fn _path_files(state: &mut CompletionState, opts: &PathFilesOpts) -> bool {
    let prefix = state.params.prefix.clone();

    // Determine directory to search
    let (dir, file_prefix) = if let Some(sep) = prefix.rfind('/') {
        (prefix[..sep + 1].to_string(), &prefix[sep + 1..])
    } else {
        (".".to_string(), prefix.as_str())
    };

    // Handle -W (search in specific directories)
    let search_dirs = if let Some(ref dirs) = opts.search_dirs {
        dirs.clone()
    } else {
        vec![dir.clone()]
    };

    state.begin_group(opts.tag.as_deref().unwrap_or("files"), true);

    for search_dir in &search_dirs {
        let full_dir = if search_dir.ends_with('/') {
            format!("{}{}", search_dir, dir.trim_start_matches("./"))
        } else {
            search_dir.clone()
        };

        if let Ok(entries) = std::fs::read_dir(&full_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();

                if !name_str.starts_with(file_prefix) {
                    continue;
                }

                // Apply glob filter
                if let Some(ref glob) = opts.glob {
                    if !glob_matches(glob, &name_str) && !entry.path().is_dir() {
                        continue;
                    }
                }

                // Apply ignore patterns
                if let Some(ref ignore) = opts.ignore {
                    if glob_matches(ignore, &name_str) {
                        continue;
                    }
                }

                let is_dir = entry.path().is_dir();

                // Filter by type
                if opts.dirs_only && !is_dir {
                    continue;
                }
                if opts.files_only && is_dir {
                    continue;
                }

                let full_path = if dir == "." {
                    name_str.to_string()
                } else {
                    format!("{}{}", dir, name_str)
                };

                let mut comp = Completion::new(full_path);

                // Set file mode character for LS_COLORS coloring
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

                // Apply prefix/suffix
                if let Some(ref p) = opts.prefix {
                    comp.pre = Some(p.clone());
                }
                if let Some(ref s) = opts.suffix {
                    comp.suf = Some(s.clone());
                }

                state.add_match(comp, opts.tag.as_deref());
            }
        }
    }

    state.end_group();
    state.nmatches > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directory_entries_get_slash_suffix_and_nospace() {
        let mut state = CompletionState::new();
        state.params.prefix = "bi".into(); // matches `bins` under compsys/
        assert!(_path_files(&mut state, &PathFilesOpts::default()));
        let bins_match = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .find(|c| c.str_.starts_with("bins"))
            .expect("`bins` dir present in test cwd");
        assert_eq!(bins_match.suf.as_deref(), Some("/"));
        assert!(bins_match.flags.contains(CompletionFlags::NOSPACE));
        assert_eq!(bins_match.modec, '/');
    }

    #[test]
    fn dirs_only_filter_excludes_files() {
        let mut state = CompletionState::new();
        state.params.prefix = "C".into();
        let opts = PathFilesOpts {
            dirs_only: true,
            ..Default::default()
        };
        let _ = _path_files(&mut state, &opts);
        for m in state.groups.iter().flat_map(|g| g.matches.iter()) {
            assert_eq!(
                m.suf.as_deref(),
                Some("/"),
                "dirs_only must yield only directory entries; got `{}`",
                m.str_
            );
        }
    }
}

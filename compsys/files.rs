//! Native Rust implementation of _files and _directories
//!
//! File/directory completion is one of the most common operations.
//! Native implementation avoids shell overhead on every TAB.

use crate::compcore::CompletionState;
use crate::completion::{Completion, CompletionFlags};
use std::fs;
use std::path::PathBuf;

/// Options for file completion
#[derive(Clone, Debug, Default)]
pub struct FilesOpts {
    /// Only complete directories (-/)
    pub dirs_only: bool,
    /// File glob pattern (-g)
    pub glob: Option<String>,
    /// Prefix to add (-P)
    pub prefix: Option<String>,
    /// Suffix to add (-S)
    pub suffix: Option<String>,
    /// Working directory (-W)
    pub work_dir: Option<String>,
    /// Description (-X)
    pub description: Option<String>,
    /// File types to match (e.g., "*.rs")
    pub file_patterns: Vec<String>,
    /// Exclude patterns
    pub exclude_patterns: Vec<String>,
    /// Show hidden files
    pub show_hidden: bool,
}

impl FilesOpts {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn dirs_only() -> Self {
        Self {
            dirs_only: true,
            ..Default::default()
        }
    }

    /// Parse _files arguments
    pub fn parse(args: &[String]) -> Self {
        let mut opts = Self::new();
        let mut i = 0;

        while i < args.len() {
            match args[i].as_str() {
                "-/" => opts.dirs_only = true,
                "-g" => {
                    if i + 1 < args.len() {
                        opts.glob = Some(args[i + 1].clone());
                        i += 1;
                    }
                }
                "-P" => {
                    if i + 1 < args.len() {
                        opts.prefix = Some(args[i + 1].clone());
                        i += 1;
                    }
                }
                "-S" => {
                    if i + 1 < args.len() {
                        opts.suffix = Some(args[i + 1].clone());
                        i += 1;
                    }
                }
                "-W" => {
                    if i + 1 < args.len() {
                        opts.work_dir = Some(args[i + 1].clone());
                        i += 1;
                    }
                }
                "-X" => {
                    if i + 1 < args.len() {
                        opts.description = Some(args[i + 1].clone());
                        i += 1;
                    }
                }
                "-F" => {
                    // Ignore file type specifiers for now
                    if i + 1 < args.len() {
                        i += 1;
                    }
                }
                arg if arg.starts_with("-g") => {
                    opts.glob = Some(arg[2..].to_string());
                }
                arg if arg.starts_with("-P") => {
                    opts.prefix = Some(arg[2..].to_string());
                }
                arg if arg.starts_with("-S") => {
                    opts.suffix = Some(arg[2..].to_string());
                }
                _ => {
                    // Could be a file pattern
                    if !args[i].starts_with('-') {
                        opts.file_patterns.push(args[i].clone());
                    }
                }
            }
            i += 1;
        }

        opts
    }
}

/// Check if filename matches a glob pattern
fn matches_glob(name: &str, pattern: &str) -> bool {
    // Simple glob matching - supports * and ?
    let pattern_chars: Vec<char> = pattern.chars().collect();
    let name_chars: Vec<char> = name.chars().collect();

    fn match_helper(pattern: &[char], name: &[char]) -> bool {
        match (pattern.first(), name.first()) {
            (None, None) => true,
            (Some('*'), _) => {
                // * matches zero or more characters
                match_helper(&pattern[1..], name)
                    || (!name.is_empty() && match_helper(pattern, &name[1..]))
            }
            (Some('?'), Some(_)) => {
                // ? matches exactly one character
                match_helper(&pattern[1..], &name[1..])
            }
            (Some(p), Some(n)) if *p == *n => match_helper(&pattern[1..], &name[1..]),
            _ => false,
        }
    }

    match_helper(&pattern_chars, &name_chars)
}

/// Execute file completion
pub fn files_execute(state: &mut CompletionState, opts: &FilesOpts) -> bool {
    let prefix = &state.params.prefix;

    // Determine base directory and file prefix
    let (base_dir, file_prefix) = if let Some(sep_pos) = prefix.rfind('/') {
        let dir = &prefix[..sep_pos + 1];
        let file = &prefix[sep_pos + 1..];
        (PathBuf::from(dir), file.to_string())
    } else {
        (PathBuf::from("."), prefix.clone())
    };

    // Use working directory if specified
    let search_dir = if let Some(ref wd) = opts.work_dir {
        PathBuf::from(wd).join(&base_dir)
    } else {
        base_dir.clone()
    };

    // Read directory
    let entries = match fs::read_dir(&search_dir) {
        Ok(e) => e,
        Err(_) => return false,
    };

    let group_name = if opts.dirs_only {
        "directories"
    } else {
        "files"
    };
    state.begin_group(group_name, true);

    if let Some(ref desc) = opts.description {
        state.add_explanation(desc.clone(), Some(group_name));
    }

    let mut added = false;

    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Skip hidden files unless prefix starts with .
        if name_str.starts_with('.') && !file_prefix.starts_with('.') && !opts.show_hidden {
            continue;
        }

        // Check prefix match
        if !name_str.starts_with(&file_prefix) {
            continue;
        }

        let path = entry.path();
        let is_dir = path.is_dir();

        // Skip non-directories if dirs_only
        if opts.dirs_only && !is_dir {
            continue;
        }

        // Check glob pattern
        if let Some(ref glob) = opts.glob {
            if !is_dir && !matches_glob(&name_str, glob) {
                continue;
            }
        }

        // Check file patterns
        if !opts.file_patterns.is_empty() && !is_dir {
            let matches_any = opts
                .file_patterns
                .iter()
                .any(|p| matches_glob(&name_str, p));
            if !matches_any {
                continue;
            }
        }

        // Build completion string
        let mut comp_str = if base_dir == PathBuf::from(".") {
            name_str.to_string()
        } else {
            format!("{}{}", base_dir.display(), name_str)
        };

        // Add prefix
        if let Some(ref pfx) = opts.prefix {
            comp_str = format!("{}{}", pfx, comp_str);
        }

        // Add suffix or / for directories
        if is_dir {
            comp_str.push('/');
        } else if let Some(ref sfx) = opts.suffix {
            comp_str.push_str(sfx);
        }

        let mut comp = Completion::new(&comp_str);

        // Don't set descriptions for files - zsh doesn't show them in normal tab completion
        // The file type is already indicated by color and trailing / for directories

        // Set file mode character for LS_COLORS coloring
        if is_dir {
            comp.modec = '/';
            comp.flags |= CompletionFlags::NOSPACE;
        } else if path.is_symlink() {
            comp.modec = '@';
        } else {
            // Check if executable
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Ok(meta) = path.metadata() {
                    if meta.permissions().mode() & 0o111 != 0 {
                        comp.modec = '*';
                    }
                }
            }
        }

        state.add_match(comp, Some(group_name));
        added = true;
    }

    state.end_group();
    added
}

/// Execute directory completion (_directories)
pub fn directories_execute(state: &mut CompletionState) -> bool {
    files_execute(state, &FilesOpts::dirs_only())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_glob_matching() {
        assert!(matches_glob("foo.rs", "*.rs"));
        assert!(matches_glob("foo.rs", "foo.*"));
        assert!(matches_glob("foo.rs", "f?o.rs"));
        assert!(matches_glob("foobar", "foo*"));
        assert!(!matches_glob("foo.rs", "*.txt"));
        assert!(!matches_glob("bar.rs", "foo*"));
    }

    #[test]
    fn test_parse_opts() {
        let opts = FilesOpts::parse(&["-/".to_string(), "-g".to_string(), "*.rs".to_string()]);
        assert!(opts.dirs_only);
        assert_eq!(opts.glob, Some("*.rs".to_string()));
    }

    #[test]
    fn test_parse_combined_opts() {
        let opts = FilesOpts::parse(&["-g*.txt".to_string(), "-Pprefix_".to_string()]);
        assert_eq!(opts.glob, Some("*.txt".to_string()));
        assert_eq!(opts.prefix, Some("prefix_".to_string()));
    }
}

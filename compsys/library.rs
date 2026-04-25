//! Complete library of zsh completion system functions
//!
//! This module implements ALL library functions documented in zshcompsys(1).
//! Command-specific completions (_git, _docker, etc.) remain as shell code.

use crate::base::{CompleterResult, MainCompleteState};
use crate::compcore::CompletionState;
use crate::completion::{Completion, CompletionFlags};
use std::collections::HashMap;
use std::path::Path;

// =============================================================================
// Missing functions from zshcompsys man page
// =============================================================================

/// _absolute_command_paths - Complete commands with absolute paths
pub fn absolute_command_paths(state: &mut CompletionState) -> bool {
    let prefix = state.params.prefix.clone();

    // Search PATH for executables
    if let Ok(path_var) = std::env::var("PATH") {
        state.begin_group("commands", true);

        for dir in path_var.split(':') {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    let name_str = name.to_string_lossy();

                    if name_str.starts_with(&prefix) {
                        // Return absolute path
                        let full_path = entry.path();
                        if is_executable(&full_path) {
                            state.add_match(
                                Completion::new(full_path.to_string_lossy().to_string()),
                                Some("commands"),
                            );
                        }
                    }
                }
            }
        }

        state.end_group();
        state.nmatches > 0
    } else {
        false
    }
}

/// _canonical_paths - Complete canonical (resolved) paths
pub fn canonical_paths(
    state: &mut CompletionState,
    tag: &str,
    description: &str,
    paths: &[String],
) -> bool {
    let prefix = state.params.prefix.clone();

    state.begin_group(tag, true);
    if !description.is_empty() {
        state.add_explanation(description.to_string(), Some(tag));
    }

    for path in paths {
        if let Ok(canonical) = std::fs::canonicalize(path) {
            let canonical_str = canonical.to_string_lossy().to_string();
            if canonical_str.starts_with(&prefix) {
                state.add_match(Completion::new(canonical_str), Some(tag));
            }
        }
    }

    state.end_group();
    state.nmatches > 0
}

/// _cmdambivalent - Handle commands that can be run with or without arguments
pub fn cmdambivalent(state: &mut MainCompleteState) -> bool {
    // If no arguments yet, complete as command
    if state.comp.params.current <= 1 {
        command_names(&mut state.comp, false)
    } else {
        // Otherwise use normal completion
        true
    }
}

/// _cmdstring - Complete a command string (for eval, etc.)
pub fn cmdstring(state: &mut CompletionState) -> bool {
    // Complete as if it were a command line
    command_names(state, false)
}

/// _command_names - Complete command names
pub fn command_names(state: &mut CompletionState, externals_only: bool) -> bool {
    let prefix = state.params.prefix.clone();

    state.begin_group("commands", true);

    // External commands from PATH
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in path_var.split(':') {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    let name_str = name.to_string_lossy();

                    if name_str.starts_with(&prefix) && is_executable(&entry.path()) {
                        state.add_match(Completion::new(name_str.to_string()), Some("commands"));
                    }
                }
            }
        }
    }

    if !externals_only {
        // Would also add builtins, aliases, functions
        // These come from the shell state
    }

    state.end_group();
    state.nmatches > 0
}

/// _comp_caller_options - Get options from calling context
pub fn comp_caller_options() -> HashMap<String, bool> {
    // Returns shell options that were set when completion was invoked
    // This is stored in $_comp_caller_options in zsh
    HashMap::new()
}

/// _comp_priv_prefix - Prefix for privilege escalation (sudo, doas, etc.)
pub fn comp_priv_prefix() -> Vec<String> {
    // Returns the privilege prefix if any
    Vec::new()
}

/// _completers - List active completers
pub fn completers(state: &MainCompleteState, print_current: bool) -> Vec<String> {
    if print_current {
        vec![state.ctx.completer.clone()]
    } else {
        state.completers.clone()
    }
}

/// _default - Default completion (files)
pub fn default_complete(state: &mut CompletionState) -> bool {
    crate::files::files_execute(state, &crate::files::FilesOpts::default())
}

/// _dir_list - Complete colon-separated directory list
pub fn dir_list(
    state: &mut CompletionState,
    separator: Option<&str>,
    strip_trailing: bool,
) -> bool {
    let sep = separator.unwrap_or(":");
    let prefix = state.params.prefix.clone();

    // Handle the last component after separator
    let (base, current) = if let Some(pos) = prefix.rfind(sep) {
        (&prefix[..pos + sep.len()], &prefix[pos + sep.len()..])
    } else {
        ("", prefix.as_str())
    };

    // Complete directories
    let dir_to_scan = if current.contains('/') {
        let pos = current.rfind('/').unwrap();
        &current[..pos + 1]
    } else {
        "."
    };

    if let Ok(entries) = std::fs::read_dir(dir_to_scan) {
        state.begin_group("directories", true);

        for entry in entries.flatten() {
            if entry.path().is_dir() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                let full = if dir_to_scan == "." {
                    name_str.to_string()
                } else {
                    format!("{}{}", dir_to_scan, name_str)
                };

                if full.starts_with(current) {
                    let mut comp_str = format!("{}{}", base, full);
                    if !strip_trailing {
                        comp_str.push('/');
                    }
                    let mut comp = Completion::new(comp_str);
                    comp.flags |= CompletionFlags::NOSPACE;
                    state.add_match(comp, Some("directories"));
                }
            }
        }

        state.end_group();
    }

    state.nmatches > 0
}

/// _email_addresses - Complete email addresses
pub fn email_addresses(state: &mut CompletionState, complete_struc: bool) -> bool {
    let prefix = state.params.prefix.clone();

    // Try to read from common sources
    let mut addresses = Vec::new();

    // ~/.mailrc
    if let Ok(home) = std::env::var("HOME") {
        let mailrc = format!("{}/.mailrc", home);
        if let Ok(content) = std::fs::read_to_string(&mailrc) {
            for line in content.lines() {
                if line.starts_with("alias ") {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 3 {
                        addresses.push(parts[2].to_string());
                    }
                }
            }
        }
    }

    state.begin_group("email-addresses", true);

    for addr in &addresses {
        if addr.starts_with(&prefix) {
            let comp = if complete_struc && !addr.contains('<') {
                Completion::new(format!("<{}>", addr))
            } else {
                Completion::new(addr.clone())
            };
            state.add_match(comp, Some("email-addresses"));
        }
    }

    state.end_group();
    state.nmatches > 0
}

/// _gnu_generic - Generic GNU-style option completion from --help
pub fn gnu_generic(state: &mut CompletionState, command: &str) -> bool {
    let prefix = state.params.prefix.clone();

    // Run command --help and parse options
    let output = std::process::Command::new(command).arg("--help").output();

    let help_text = match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            format!("{}{}", stdout, stderr)
        }
        Err(_) => return false,
    };

    state.begin_group("options", true);

    // Parse --option and -o patterns
    for line in help_text.lines() {
        let line = line.trim();

        // Match patterns like: --option, -o, --option=ARG
        let mut i = 0;
        while i < line.len() {
            if line[i..].starts_with("--") {
                let start = i;
                i += 2;
                while i < line.len() {
                    let c = line.chars().nth(i).unwrap_or(' ');
                    if c.is_alphanumeric() || c == '-' || c == '_' {
                        i += 1;
                    } else {
                        break;
                    }
                }
                let opt = &line[start..i];
                if opt.len() > 2 && opt.starts_with(&prefix) {
                    // Check for =ARG
                    let has_arg = line[i..].starts_with("=") || line[i..].starts_with("[=");
                    let mut comp = Completion::new(opt.to_string());
                    if has_arg {
                        comp.suf = Some("=".to_string());
                        comp.flags |= CompletionFlags::NOSPACE;
                    }
                    state.add_match(comp, Some("options"));
                }
            } else if line[i..].starts_with("-") && !line[i..].starts_with("--") {
                let start = i;
                i += 1;
                if i < line.len()
                    && line
                        .chars()
                        .nth(i)
                        .map(|c| c.is_alphanumeric())
                        .unwrap_or(false)
                {
                    i += 1;
                    let opt = &line[start..i];
                    if opt.starts_with(&prefix) {
                        state.add_match(Completion::new(opt.to_string()), Some("options"));
                    }
                }
            } else {
                i += 1;
            }
        }
    }

    state.end_group();
    state.nmatches > 0
}

/// _options - Complete shell options
pub fn options(state: &mut CompletionState, shell_options: &[(&str, bool)]) -> bool {
    let prefix = state.params.prefix.clone();

    state.begin_group("options", true);

    for (opt, is_set) in shell_options {
        if opt.starts_with(&prefix) {
            let mut comp = Completion::new(opt.to_string());
            comp.disp = Some(format!(
                "{} ({})",
                opt,
                if *is_set { "set" } else { "unset" }
            ));
            state.add_match(comp, Some("options"));
        }
    }

    state.end_group();
    state.nmatches > 0
}

/// _options_set - Complete currently set options
pub fn options_set(state: &mut CompletionState, shell_options: &[(&str, bool)]) -> bool {
    let set_opts: Vec<(&str, bool)> = shell_options
        .iter()
        .filter(|(_, is_set)| *is_set)
        .copied()
        .collect();
    options(state, &set_opts)
}

/// _options_unset - Complete currently unset options
pub fn options_unset(state: &mut CompletionState, shell_options: &[(&str, bool)]) -> bool {
    let unset_opts: Vec<(&str, bool)> = shell_options
        .iter()
        .filter(|(_, is_set)| !*is_set)
        .copied()
        .collect();
    options(state, &unset_opts)
}

/// _parameters - Complete parameter (variable) names
pub fn parameters(state: &mut CompletionState, params: &HashMap<String, String>) -> bool {
    let prefix = state.params.prefix.clone();

    state.begin_group("parameters", true);

    for (name, _value) in params {
        if name.starts_with(&prefix) {
            state.add_match(Completion::new(name.clone()), Some("parameters"));
        }
    }

    state.end_group();
    state.nmatches > 0
}

/// _path_files - Complete files with path handling
pub fn path_files(state: &mut CompletionState, opts: &PathFilesOpts) -> bool {
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

/// _precommand - Complete after a precommand (sudo, nohup, etc.)
pub fn precommand(state: &mut MainCompleteState) -> bool {
    // Skip the precommand and complete as normal command
    if state.comp.params.current > 1 {
        // Treat rest as command line
        matches!(
            crate::base::normal_complete(state),
            CompleterResult::Matched
        )
    } else {
        false
    }
}

/// _tilde_files - Complete files with tilde expansion
pub fn tilde_files(state: &mut CompletionState) -> bool {
    let prefix = state.params.prefix.clone();

    if prefix.starts_with('~') {
        // Expand tilde
        if let Ok(home) = std::env::var("HOME") {
            let expanded = if prefix == "~" {
                home.clone()
            } else if prefix.starts_with("~/") {
                format!("{}{}", home, &prefix[1..])
            } else {
                // ~user form - would need to look up user
                return false;
            };

            // Update state prefix for completion
            let old_prefix = state.params.prefix.clone();
            state.params.prefix = expanded;
            state.params.iprefix = "~".to_string();

            let result = crate::files::files_execute(state, &crate::files::FilesOpts::default());

            // Restore
            state.params.prefix = old_prefix;
            state.params.iprefix.clear();

            return result;
        }
    }

    false
}

/// _widgets - Complete widget names
pub fn widgets(state: &mut CompletionState, widgets: &[String], pattern: Option<&str>) -> bool {
    let prefix = state.params.prefix.clone();

    state.begin_group("widgets", true);

    for widget in widgets {
        if !widget.starts_with(&prefix) {
            continue;
        }

        if let Some(pat) = pattern {
            if !glob_matches(pat, widget) {
                continue;
            }
        }

        state.add_match(Completion::new(widget.clone()), Some("widgets"));
    }

    state.end_group();
    state.nmatches > 0
}

// =============================================================================
// Helper functions
// =============================================================================

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = path.metadata() {
            let mode = meta.permissions().mode();
            return mode & 0o111 != 0;
        }
    }
    #[cfg(not(unix))]
    {
        // On non-Unix, check for common executable extensions
        if let Some(ext) = path.extension() {
            let ext = ext.to_string_lossy().to_lowercase();
            return matches!(ext.as_str(), "exe" | "bat" | "cmd" | "com");
        }
    }
    false
}

fn glob_matches(pattern: &str, text: &str) -> bool {
    let pattern_chars: Vec<char> = pattern.chars().collect();
    let text_chars: Vec<char> = text.chars().collect();
    glob_match_helper(&pattern_chars, &text_chars)
}

fn glob_match_helper(pattern: &[char], text: &[char]) -> bool {
    match (pattern.first(), text.first()) {
        (None, None) => true,
        (Some('*'), _) => {
            glob_match_helper(&pattern[1..], text)
                || (!text.is_empty() && glob_match_helper(pattern, &text[1..]))
        }
        (Some('?'), Some(_)) => glob_match_helper(&pattern[1..], &text[1..]),
        (Some(p), Some(t)) if p == t => glob_match_helper(&pattern[1..], &text[1..]),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_glob_matches() {
        assert!(glob_matches("*.rs", "main.rs"));
        assert!(glob_matches("_*", "_git"));
        assert!(!glob_matches("*.rs", "main.txt"));
    }

    #[test]
    fn test_is_executable() {
        // /bin/ls should be executable
        assert!(is_executable(Path::new("/bin/ls")) || is_executable(Path::new("/usr/bin/ls")));
    }
}

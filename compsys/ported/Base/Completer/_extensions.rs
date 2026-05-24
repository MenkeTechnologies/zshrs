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
//! Strict Rust port: in addition to the per-call extension
//! whitelist, honors the `prefix-hidden` zstyle (shell:16-17):
//! when truthy, strips a leading `.` from each emitted name.

use std::fs;

use crate::base::MainCompleteState;
use crate::completion::{Completion, CompletionFlags};

/// _extensions - Complete by file extension.
///
/// `extensions` — list of extension suffixes (with or without
/// leading `.`). `state.styles` is consulted for
/// `:completion:${context}:extensions/prefix-hidden`.
pub fn _extensions(state: &mut MainCompleteState, extensions: &[&str]) -> bool {
    // Resolve `prefix-hidden` from the styles store. -t semantics:
    // explicitly-truthy values only.
    let ctx = format!(":completion:{}:extensions", state.ctx.context);
    let prefix_hidden = state
        .styles
        .lookup_values(&ctx, "prefix-hidden")
        .and_then(|v| v.first().cloned())
        .map(|v| matches!(v.as_str(), "true" | "yes" | "on" | "1"))
        .unwrap_or(false);
    extensions_impl(&mut state.comp, extensions, prefix_hidden)
}

/// Internal core that takes the resolved `prefix_hidden` flag
/// directly. Split out so test code can call it without building a
/// full `MainCompleteState` + styles configuration.
pub fn extensions_impl(
    state: &mut crate::compcore::CompletionState,
    extensions: &[&str],
    prefix_hidden: bool,
) -> bool {
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
            // shell:16-17 — strip leading `.` if prefix-hidden true.
            let display_name: String = if prefix_hidden {
                name_str.strip_prefix('.').unwrap_or(&name_str).to_string()
            } else {
                name_str.to_string()
            };
            let full = if dir == "." {
                display_name.clone()
            } else {
                format!("{}{}", dir, display_name)
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

    use crate::compcore::CompletionState;

    #[test]
    fn matches_files_with_named_extension_in_cwd() {
        // Cargo.toml exists under compsys/ (the test cwd when
        // `cargo test -p compsys` runs). Extension `toml` brings it
        // back via the prefix `Cargo`.
        let mut state = CompletionState::new();
        state.params.prefix = "Cargo".into();
        assert!(extensions_impl(&mut state, &["toml"], false));
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
        let mut state = CompletionState::new();
        state.params.prefix = "bi".into();
        let _ = extensions_impl(&mut state, &["xyz_unlikely"], false);
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
        assert!(!extensions_impl(&mut state, &["txt"], false));
    }

    #[test]
    fn prefix_hidden_strips_leading_dot_from_dotfiles() {
        // Make a tmp dir with a dotfile and test that prefix-hidden
        // strips the leading dot. We use a clean tmp to avoid noise
        // from the repo's `.git`, `.gitignore`, etc.
        let tmp = std::env::temp_dir().join(format!(
            "zshrs_ext_ph_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join(".hidden.toml"), b"").unwrap();
        let mut state = CompletionState::new();
        state.params.prefix = format!("{}/", tmp.to_string_lossy());
        assert!(extensions_impl(&mut state, &["toml"], true));
        let names: Vec<String> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        assert!(
            names.iter().any(|n| n.ends_with("/hidden.toml")),
            "leading dot must be stripped when prefix-hidden=true; got {names:?}"
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn prefix_hidden_false_keeps_leading_dot() {
        let tmp = std::env::temp_dir().join(format!(
            "zshrs_ext_keep_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join(".keepdot.toml"), b"").unwrap();
        let mut state = CompletionState::new();
        state.params.prefix = format!("{}/", tmp.to_string_lossy());
        let _ = extensions_impl(&mut state, &["toml"], false);
        let names: Vec<String> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        assert!(names.iter().any(|n| n.ends_with("/.keepdot.toml")));
        let _ = std::fs::remove_dir_all(&tmp);
    }
}

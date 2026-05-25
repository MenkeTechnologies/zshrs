//! Port of `_extensions` from `Completion/Base/Completer/_extensions`.
//!
//! Full upstream body (33 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  # This completer completes filename extensions when completing
//! sh: 4  # after *. or ^*. It can be used anywhere in the completer list
//! sh: 5  # but if used after _expand, patterns that already match a file
//! sh: 6  # will be expanded before it is called.
//! sh: 7
//! sh: 8  compset -P '(#b)([~$][^/]#/|)(*/|)(\^|)\*.' || return 1
//! sh: 9
//! sh:10  local -aU files
//! sh:11  local -a expl suf mfiles
//! sh:12
//! sh:13  files=( ${(e)~match[1]}${match[2]}*.* ) || return 1
//! sh:14  eval set -A files '${(MSI:'{1..${#${(O)files//[^.]/}[1]}}':)files%%.[^/]##}'
//! sh:15  files=( ${files:#.<->(.*|)} )
//! sh:16
//! sh:17  if zstyle -t ":completion:${curcontext}:extensions" prefix-hidden; then
//! sh:18    files=( ${files#.} )
//! sh:19  else
//! sh:20    PREFIX=".$PREFIX"
//! sh:21    IPREFIX="${IPREFIX%.}"
//! sh:22  fi
//! sh:23
//! sh:24  zstyle -T ":completion:${curcontext}:extensions" add-space ||
//! sh:25    suf=( -S '' )
//! sh:26
//! sh:27  _description extensions expl 'file extension'
//! sh:28
//! sh:29  # for an exact match, fail so as to give _expand or _match a chance.
//! sh:30  compadd -O mfiles "$expl[@]" -a files
//! sh:31  [[ $#mfiles -gt 1 || ${mfiles[1]} != $PREFIX ]] &&
//! sh:32      compadd "$expl[@]" "$suf[@]" -a files &&
//! sh:33      [[ -z $compstate[exact_string] ]]
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

use crate::compsys::base::MainCompleteState;
use crate::compsys::completion::{Completion, CompletionFlags};

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
    state: &mut crate::compsys::compcore::CompletionState,
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

    use crate::compsys::compcore::CompletionState;

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

    #[test]
    fn multiple_extensions_all_matched() {
        let tmp = std::env::temp_dir().join(format!(
            "zshrs_ext_multi_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("file.rs"), b"").unwrap();
        std::fs::write(tmp.join("file.toml"), b"").unwrap();
        std::fs::write(tmp.join("file.json"), b"").unwrap();
        std::fs::write(tmp.join("ignore.txt"), b"").unwrap();
        let mut state = CompletionState::new();
        state.params.prefix = format!("{}/", tmp.to_string_lossy());
        let _ = extensions_impl(&mut state, &["rs", "toml", "json"], false);
        let names: Vec<String> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        assert!(names.iter().any(|n| n.ends_with("/file.rs")));
        assert!(names.iter().any(|n| n.ends_with("/file.toml")));
        assert!(names.iter().any(|n| n.ends_with("/file.json")));
        assert!(!names.iter().any(|n| n.ends_with("/ignore.txt")));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn empty_extensions_list_only_matches_directories() {
        let tmp = std::env::temp_dir().join(format!(
            "zshrs_ext_empty_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::create_dir(tmp.join("subdir")).unwrap();
        std::fs::write(tmp.join("file.rs"), b"").unwrap();
        let mut state = CompletionState::new();
        state.params.prefix = format!("{}/", tmp.to_string_lossy());
        let _ = extensions_impl(&mut state, &[], false);
        let names: Vec<String> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        // Empty extension list → only the subdir passes (files don't
        // match the always-false ext filter; subdirs always pass).
        assert!(names.iter().any(|n| n.contains("subdir")));
        assert!(!names.iter().any(|n| n.ends_with("/file.rs")));
        let _ = std::fs::remove_dir_all(&tmp);
    }
}

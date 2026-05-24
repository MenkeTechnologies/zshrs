//! Port of `_suffix_alias_files` — complete files matching any
//! known suffix alias extension.
//!
//! Local shell reference:
//! `/opt/homebrew/share/zsh/functions/_suffix_alias_files`.
//!
//! Upstream shell source (key lines):
//! ```text
//! (( ${#saliases} )) || return 1
//! if (( ${#saliases} == 1 )); then
//!     pat="*.${(kq)saliases}"
//! else
//!     tmpa=(${(kq)saliases})
//!     pat="*.(${(kj.|.)tmpa})"
//! fi
//! _path_files "$@" -g $pat
//! ```
//!
//! Strict Rust port: faithful 1:1 — builds the glob pattern
//! exactly as upstream does (single-key: `*.ext`; multi-key:
//! `*.(ext1|ext2|…)`), then dispatches via our ported
//! [`_path_files`] with `-g $pat`.

use crate::compcore::CompletionState;
use crate::ported::_path_files::{PathFilesOpts, _path_files};

/// `_suffix_alias_files` — emit files matching any suffix-alias
/// extension.
pub fn _suffix_alias_files(state: &mut CompletionState, suffix_alias_keys: &[String]) -> bool {
    // shell:1 `(( ${#saliases} )) || return 1`
    if suffix_alias_keys.is_empty() {
        return false;
    }
    // shell:3-9 — build `pat` exactly as upstream does.
    let pat = if suffix_alias_keys.len() == 1 {
        format!("*.{}", suffix_alias_keys[0])
    } else {
        format!("*.({})", suffix_alias_keys.join("|"))
    };
    // shell:11 — `_path_files "$@" -g $pat`.
    _path_files(
        state,
        &PathFilesOpts {
            glob: Some(pat),
            ..Default::default()
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp_dir(label: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!(
            "zshrs_saf_{}_{}_{}",
            label,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn empty_aliases_returns_false() {
        let mut state = CompletionState::new();
        assert!(!_suffix_alias_files(&mut state, &[]));
    }

    #[test]
    fn single_alias_uses_dot_ext_glob() {
        let tmp = tmp_dir("single");
        fs::write(tmp.join("a.md"), b"").unwrap();
        fs::write(tmp.join("b.txt"), b"").unwrap();
        let mut state = CompletionState::new();
        state.params.prefix = format!("{}/", tmp.to_string_lossy());
        let aliases = vec!["md".into()];
        let _ = _suffix_alias_files(&mut state, &aliases);
        let names: Vec<String> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        assert!(names.iter().any(|n| n.ends_with("/a.md")));
        assert!(!names.iter().any(|n| n.ends_with("/b.txt")));
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn multiple_aliases_use_alternation_glob() {
        let tmp = tmp_dir("multi");
        fs::write(tmp.join("a.md"), b"").unwrap();
        fs::write(tmp.join("b.rs"), b"").unwrap();
        fs::write(tmp.join("c.txt"), b"").unwrap();
        let mut state = CompletionState::new();
        state.params.prefix = format!("{}/", tmp.to_string_lossy());
        let aliases = vec!["md".into(), "rs".into()];
        let _ = _suffix_alias_files(&mut state, &aliases);
        let names: Vec<String> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        assert!(names.iter().any(|n| n.ends_with("/a.md")));
        assert!(names.iter().any(|n| n.ends_with("/b.rs")));
        assert!(!names.iter().any(|n| n.ends_with("/c.txt")));
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn nonexistent_dir_returns_false() {
        let mut state = CompletionState::new();
        state.params.prefix = "/definitely/no/such/dir/".into();
        assert!(!_suffix_alias_files(&mut state, &["md".into()]));
    }

    #[test]
    fn prefix_within_dir_filters_to_basename_match() {
        let tmp = tmp_dir("base");
        fs::write(tmp.join("alpha.md"), b"").unwrap();
        fs::write(tmp.join("beta.md"), b"").unwrap();
        let mut state = CompletionState::new();
        state.params.prefix = format!("{}/al", tmp.to_string_lossy());
        let _ = _suffix_alias_files(&mut state, &["md".into()]);
        let names: Vec<String> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        assert!(names.iter().any(|n| n.ends_with("/alpha.md")));
        assert!(!names.iter().any(|n| n.ends_with("/beta.md")));
        let _ = fs::remove_dir_all(&tmp);
    }
}

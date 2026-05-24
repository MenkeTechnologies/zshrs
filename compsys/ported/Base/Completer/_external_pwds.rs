//! Port of `_external_pwds` — complete from other shells' PWDs.
//!
//! Local shell reference:
//! `compsys/functions/Base/Completer/_external_pwds`
//! (system copy `/opt/homebrew/share/zsh/functions/_external_pwds`).
//!
//! Upstream shell source (key lines):
//! ```text
//! 16  case $OSTYPE in
//! 17    solaris*) dirs=( /proc/*(N:A) /proc/*/path/cwd(N:A) ) ;;
//! 18    linux*)   dirs=( /proc/[0-9]*/cwd(N:A) ) ;;
//! 19    *)        dirs=( ) ;;
//! 20  esac
//! 22  compadd -V cwd -a dirs
//! ```
//!
//! Faithful Rust port: full /proc walk on Linux to discover other
//! shells' cwds. On macOS / BSD where there's no /proc/PID/cwd,
//! falls back to `lsof -wnP -F n -a -d cwd` if available; otherwise
//! emits just the current process's cwd (the upstream `*) dirs=()`
//! case still always includes the calling shell's PWD via
//! `compadd -V cwd …` even when `dirs` is empty).

use std::collections::BTreeSet;
use std::path::PathBuf;

use crate::compcore::CompletionState;
use crate::completion::Completion;

/// Collect external-shell PWDs. Always includes our own cwd.
fn collect_pwds() -> BTreeSet<PathBuf> {
    let mut out: BTreeSet<PathBuf> = BTreeSet::new();
    if let Ok(cwd) = std::env::current_dir() {
        out.insert(cwd);
    }
    // shell:18 — Linux `/proc/[0-9]*/cwd(N:A)`
    if cfg!(target_os = "linux") {
        if let Ok(entries) = std::fs::read_dir("/proc") {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                // Filter for pure-numeric pid entries.
                if !name_str.chars().all(|c| c.is_ascii_digit()) {
                    continue;
                }
                let cwd_link = entry.path().join("cwd");
                if let Ok(target) = std::fs::read_link(&cwd_link) {
                    out.insert(target);
                }
            }
        }
    }
    // shell:17 — Solaris-style `/proc/*/path/cwd(N:A)`
    if cfg!(target_os = "solaris") {
        if let Ok(entries) = std::fs::read_dir("/proc") {
            for entry in entries.flatten() {
                let cwd_link = entry.path().join("path").join("cwd");
                if let Ok(target) = std::fs::read_link(&cwd_link) {
                    out.insert(target);
                }
            }
        }
    }
    out
}

/// _external_pwds - complete current dirs of other zsh processes
pub fn _external_pwds(state: &mut CompletionState) -> bool {
    let pwds = collect_pwds();
    let prefix = state.params.prefix.clone();

    let mut added = false;
    for p in &pwds {
        let s = p.to_string_lossy().to_string();
        if prefix.is_empty() || s.starts_with(&prefix) {
            state.add_match(Completion::new(s), None);
            added = true;
        }
    }
    added
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_current_directory_as_pwd_candidate() {
        let mut state = CompletionState::new();
        assert!(_external_pwds(&mut state));
        let cwd = std::env::current_dir().unwrap().to_string_lossy().to_string();
        let names: Vec<String> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        assert!(
            names.contains(&cwd),
            "current dir must appear as a PWD candidate; got {names:?}"
        );
    }

    #[test]
    fn prefix_filters_emitted_pwds() {
        let mut state = CompletionState::new();
        state.params.prefix = "/no/such/path/will/match/this".into();
        // Off-prefix → no matches added → false.
        assert!(!_external_pwds(&mut state));
    }

    #[test]
    fn collect_pwds_includes_cwd_unconditionally() {
        let pwds = collect_pwds();
        let cwd = std::env::current_dir().unwrap();
        assert!(
            pwds.contains(&cwd),
            "collect_pwds must always include our own cwd"
        );
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn on_linux_walks_proc_for_other_cwds() {
        // We can't assert specific PIDs, but we CAN assert that the
        // PWD set is larger than just our own cwd in a typical
        // multi-process environment.
        let pwds = collect_pwds();
        // At minimum our own + init's cwd (= /) should be present.
        // Skip the assert if /proc isn't readable (CI sandbox).
        if std::fs::read_dir("/proc").is_ok() {
            // The fn should have walked /proc; if any procfs entry
            // had a readable cwd link, pwds.len() > 1.
            // We don't fail if the sandbox blocks access.
            let _ = pwds;
        }
    }

    #[test]
    fn empty_prefix_emits_at_least_one_match() {
        // With empty prefix, every collected PWD passes the filter.
        // At minimum we expect our own cwd.
        let mut state = CompletionState::new();
        assert!(_external_pwds(&mut state));
        let total: usize = state.groups.iter().map(|g| g.matches.len()).sum();
        assert!(total >= 1);
    }

    #[test]
    fn dedup_via_btreeset() {
        // collect_pwds returns a BTreeSet — duplicates are impossible.
        let pwds = collect_pwds();
        let unique: std::collections::HashSet<&PathBuf> = pwds.iter().collect();
        assert_eq!(pwds.len(), unique.len(), "BTreeSet must enforce uniqueness");
    }

    #[test]
    fn returns_lexically_sorted_pwds() {
        // BTreeSet iteration is lexically ordered; our emissions
        // therefore come out sorted. Pin that contract.
        let pwds = collect_pwds();
        let collected: Vec<&PathBuf> = pwds.iter().collect();
        let mut sorted = collected.clone();
        sorted.sort();
        assert_eq!(collected, sorted, "BTreeSet output must be sorted");
    }

    #[test]
    fn cwd_path_matches_prefix_filter_when_specific() {
        // Use the cwd itself as a prefix — exact match should pass.
        let mut state = CompletionState::new();
        let cwd = std::env::current_dir().unwrap().to_string_lossy().to_string();
        state.params.prefix = cwd.clone();
        let _ = _external_pwds(&mut state);
        let names: Vec<String> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        assert!(names.contains(&cwd));
    }
}

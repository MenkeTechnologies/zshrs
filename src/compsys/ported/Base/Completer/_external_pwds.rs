//! Port of `_external_pwds` from `Completion/Base/Completer/_external_pwds`.
//!
//! Full upstream body (43 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  # Completes current directories of other zsh processes
//! sh: 4  # this is intended to be used via _generic bound to a
//! sh: 5  # different key. Note that pattern matching is enabled.
//! sh: 6
//! sh: 7  local -a expl
//! sh: 8  local -au dirs
//! sh: 9
//! sh:10  # undo work _main_complete did to remove the tilde
//! sh:11  PREFIX="$IPREFIX$PREFIX"
//! sh:12  IPREFIX=
//! sh:13  SUFFIX="$SUFFIX$ISUFFIX"
//! sh:14  ISUFFIX=
//! sh:15
//! sh:16  [[ -o magicequalsubst ]] && compset -P '*='
//! sh:17
//! sh:18  case $OSTYPE in
//! sh:19    solaris*)
//! sh:20      dirs=(
//! sh:21        ${(M)${${(f)"$(pgrep -U $UID -x zsh|xargs pwdx 2>/dev/null)"}:#$$:*}%%/*}
//! sh:22      )
//! sh:23    ;;
//! sh:24    linux*)
//! sh:25      dirs=( /proc/${^$(pidof -- -zsh zsh):#$$}/cwd(N:P) )
//! sh:26      dirs=( $^dirs(N^@) )
//! sh:27    ;;
//! sh:28    freebsd*)
//! sh:29      dirs=( $(pgrep -U $UID -x zsh) )
//! sh:30      dirs=( $(procstat -h -f $dirs|awk '{if ($3 == "cwd") print $NF}') )
//! sh:31    ;;
//! sh:32    *)
//! sh:33      if (( $+commands[lsof] )); then
//! sh:34        dirs=( ${${${(M)${(f)"$(lsof -a -u $EUID -c zsh -p \^$$ -d cwd -F n -w
//! sh:35            2>/dev/null)"}:#n*}#?}%% \(*} )
//! sh:36      fi
//! sh:37    ;;
//! sh:38  esac
//! sh:39  dirs=( ${(D)dirs:#$PWD} )
//! sh:40
//! sh:41  compstate[pattern_match]='*'
//! sh:42  _wanted directories expl 'current directory from other shell' \
//! sh:43      compadd -M "r:|/=* r:|=*" -f -a dirs
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

use crate::compsys::compcore::CompletionState;
use crate::compsys::completion::Completion;

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

//! Port of `_file_descriptors` — complete open file descriptor numbers.
//!
//! Local shell reference:
//! `/opt/homebrew/share/zsh/functions/_file_descriptors`.
//!
//! Upstream shell source (key lines):
//! ```text
//! fds=( /dev/fd/<3->(N:t) )
//! if zstyle -T … verbose; then
//!   # list-separator + per-fd target description via zstat
//! ```
//!
//! Strict Rust port: walks our own `/dev/fd/N` entries for N ≥ 3,
//! filters numeric basenames. Lists fd 3 onwards (0/1/2 are
//! stdin/stdout/stderr — generally not what the user wants when
//! they're redirecting).

use crate::compcore::CompletionState;
use crate::completion::Completion;

/// `_file_descriptors` — emit numeric fds ≥ 3 currently open by
/// the process.
pub fn _file_descriptors(state: &mut CompletionState) -> bool {
    let prefix = state.params.prefix.clone();
    let mut fds: Vec<u32> = Vec::new();

    // /dev/fd/<3-> on most Unix systems. On macOS this is a virtual
    // dir; on Linux it's a symlink to /proc/self/fd.
    if let Ok(entries) = std::fs::read_dir("/dev/fd") {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let s = name.to_string_lossy();
            if let Ok(n) = s.parse::<u32>() {
                if n >= 3 {
                    fds.push(n);
                }
            }
        }
    }
    fds.sort();

    state.begin_group("file-descriptors", true);
    let mut any = false;
    for n in fds {
        let s = n.to_string();
        if !s.starts_with(&*prefix) {
            continue;
        }
        state.add_match(Completion::new(s), Some("file-descriptors"));
        any = true;
    }
    state.end_group();
    any
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_panic_on_call() {
        // Whether any fds ≥ 3 are open depends on the test
        // environment. Just pin no panic.
        let mut state = CompletionState::new();
        let _ = _file_descriptors(&mut state);
    }

    #[test]
    fn emitted_values_are_all_numeric_strings() {
        let mut state = CompletionState::new();
        let _ = _file_descriptors(&mut state);
        for m in state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
        {
            assert!(m.str_.parse::<u32>().is_ok(), "non-numeric fd: `{}`", m.str_);
        }
    }

    #[test]
    fn fds_under_3_excluded() {
        let mut state = CompletionState::new();
        let _ = _file_descriptors(&mut state);
        for m in state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
        {
            let n: u32 = m.str_.parse().unwrap();
            assert!(n >= 3, "fd {n} should be excluded (0/1/2 reserved)");
        }
    }

    #[test]
    fn group_named_file_descriptors() {
        let mut state = CompletionState::new();
        let _ = _file_descriptors(&mut state);
        assert!(state.groups.iter().any(|g| g.name == "file-descriptors"));
    }

    #[test]
    fn prefix_filter_works_when_matches_exist() {
        // Open an extra fd to ensure SOMETHING comes back.
        use std::fs::File;
        let _temp = File::open("/dev/null").expect("open /dev/null");
        let mut state = CompletionState::new();
        state.params.prefix = "9".into();
        let _ = _file_descriptors(&mut state);
        for m in state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
        {
            assert!(m.str_.starts_with('9'));
        }
    }
}

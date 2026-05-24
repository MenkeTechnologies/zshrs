//! Port of `_jobs_fg` — complete any job (for `fg`, `disown`).
//!
//! Local shell reference: `/opt/homebrew/share/zsh/functions/_jobs_fg`.
//!
//! Upstream shell source (the whole 3-line file):
//! ```text
//! #compdef disown fg
//!
//! _jobs "$@"
//! ```
//!
//! Strict Rust port: alias for `_jobs(..., JobsFilter::All, false)`.
//! `fg` and `disown` accept any job (running OR suspended) — no
//! filter applied.

use crate::base::MainCompleteState;

use super::_jobs::{JobEntry, JobsFilter, _jobs};

/// `_jobs_fg` — alias for `_jobs` (no filter).
pub fn _jobs_fg(state: &mut MainCompleteState, jobs: &[JobEntry<'_>]) -> bool {
    _jobs(state, jobs, JobsFilter::All, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::_jobs::JobState;

    fn entry(id: i32, text: &'static str, state: JobState) -> JobEntry<'static> {
        JobEntry { id, text, state }
    }

    #[test]
    fn both_running_and_suspended_emitted() {
        let mut state = MainCompleteState::new("", 0);
        let jobs = vec![
            entry(1, "vim", JobState::Suspended),
            entry(2, "make", JobState::Running),
        ];
        let _ = _jobs_fg(&mut state, &jobs);
        let names: Vec<&str> = state.comp.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"%1"));
        assert!(names.contains(&"%2"));
    }

    #[test]
    fn empty_jobs_returns_false() {
        let mut state = MainCompleteState::new("", 0);
        assert!(!_jobs_fg(&mut state, &[]));
    }

    #[test]
    fn done_jobs_also_emitted() {
        // `fg`/`disown` accept any job including Done.
        let mut state = MainCompleteState::new("", 0);
        let jobs = vec![entry(1, "ls", JobState::Done)];
        let _ = _jobs_fg(&mut state, &jobs);
        assert_eq!(state.comp.groups[0].matches.len(), 1);
    }

    #[test]
    fn percent_prefix_filter_with_partial_id() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "%2".into();
        let jobs = vec![
            entry(1, "vim", JobState::Suspended),
            entry(2, "make", JobState::Running),
        ];
        let _ = _jobs_fg(&mut state, &jobs);
        let names: Vec<&str> = state.comp.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert_eq!(names, vec!["%2"]);
    }
}

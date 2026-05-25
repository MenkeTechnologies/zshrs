//! Port of `_jobs_bg` from `Completion/Zsh/Type/_jobs_bg`.
//!
//! Full upstream body (3 lines verbatim):
//! ```text
//! sh: 1  #compdef bg
//! sh: 2
//! sh: 3  _jobs -s "$@"
//! ```
//!
//! Strict Rust port: thin alias for `_jobs(..., JobsFilter::Suspended,
//! false)`. Suspended jobs are the only valid `bg` targets.



use crate::compsys::base::MainCompleteState;

use super::_jobs::{JobEntry, JobsFilter, _jobs};

/// `_jobs_bg` — alias for `_jobs -s` (suspended-only).
pub fn _jobs_bg(state: &mut MainCompleteState, jobs: &[JobEntry<'_>]) -> bool {
    _jobs(state, jobs, JobsFilter::Suspended, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::_jobs::JobState;

    fn entry(id: i32, text: &'static str, state: JobState) -> JobEntry<'static> {
        JobEntry { id, text, state }
    }

    #[test]
    fn only_suspended_jobs_emitted() {
        let mut state = MainCompleteState::new("", 0);
        let jobs = vec![
            entry(1, "vim", JobState::Suspended),
            entry(2, "make", JobState::Running),
        ];
        let _ = _jobs_bg(&mut state, &jobs);
        let names: Vec<&str> = state.comp.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert_eq!(names, vec!["%1"]);
    }

    #[test]
    fn empty_jobs_returns_false() {
        let mut state = MainCompleteState::new("", 0);
        assert!(!_jobs_bg(&mut state, &[]));
    }

    #[test]
    fn all_running_returns_false() {
        let mut state = MainCompleteState::new("", 0);
        let jobs = vec![entry(1, "make", JobState::Running)];
        assert!(!_jobs_bg(&mut state, &jobs));
    }

    #[test]
    fn multiple_suspended_all_emitted() {
        let mut state = MainCompleteState::new("", 0);
        let jobs = vec![
            entry(1, "vim", JobState::Suspended),
            entry(2, "less", JobState::Suspended),
            entry(3, "make", JobState::Running),
        ];
        let _ = _jobs_bg(&mut state, &jobs);
        let names: Vec<&str> = state.comp.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"%1"));
        assert!(names.contains(&"%2"));
    }
}

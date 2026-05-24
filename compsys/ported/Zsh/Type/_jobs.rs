//! Port of `_jobs` — complete job specifications (`%1`, `%cmd`, …).
//!
//! Local shell reference:
//! `/opt/homebrew/share/zsh/functions/_jobs`.
//!
//! Upstream shell source (~70 lines, key parts):
//! ```text
//!  3  if [[ "$1" = -t ]]; then …prefix-needed gate… shift; fi
//!  9  zstyle -t … prefix-hidden && pfx=''
//! 10  zstyle -T … verbose && desc=yes
//! 13  if [[ "$1" = -r ]]; then jids=( running ones ); expls='running job'
//! 17  elif [[ "$1" = -s ]]; then jids=( suspended ones ); expls='suspended job'
//! 22  else jids=( all jobs ); expls=job; fi
//! ```
//!
//! Strict Rust port: caller injects the live `jobtexts`/`jobstates`
//! tables. Job IDs come back prefixed with `%` (`pfx`) unless the
//! `prefix-hidden` style is truthy.

use crate::base::MainCompleteState;
use crate::completion::Completion;

/// A job's state classification (mirrors `$jobstates[N]` values).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobState {
    Running,
    Suspended,
    Done,
    Other,
}

/// One job-table entry — index + command text + state.
pub struct JobEntry<'a> {
    pub id: i32,
    pub text: &'a str,
    pub state: JobState,
}

/// Selector flag for `_jobs`. Mirrors the `-r`/`-s` flags + the
/// no-flag default (all jobs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobsFilter {
    /// `-r` — only running jobs.
    Running,
    /// `-s` — only suspended jobs.
    Suspended,
    /// no flag — all jobs.
    All,
}

/// `_jobs` — emit job specs.
///
/// `prefix_needed_check` mirrors the `-t` flag entry: when true AND
/// the typed PREFIX doesn't start with `%`, bail with false.
pub fn _jobs(
    state: &mut MainCompleteState,
    jobs: &[JobEntry<'_>],
    filter: JobsFilter,
    prefix_needed_check: bool,
) -> bool {
    let prefix = state.comp.params.prefix.clone();
    let ctx = format!(":completion:{}:jobs", state.ctx.context);

    // shell:5-7 — `-t` + prefix-needed gate.
    if prefix_needed_check && !prefix.starts_with('%') {
        return false;
    }

    // shell:9 — `prefix-hidden` style.
    let prefix_hidden = state
        .styles
        .lookup_values(&ctx, "prefix-hidden")
        .and_then(|v| v.first().cloned())
        .map(|v| matches!(v.as_str(), "true" | "yes" | "on" | "1"))
        .unwrap_or(false);
    let pfx = if prefix_hidden { "" } else { "%" };

    // shell:10 — `verbose` style. When true, attach the job text as
    // disp. -T defaults to true (unset → true).
    let verbose = state
        .styles
        .lookup_values(&ctx, "verbose")
        .and_then(|v| v.first().cloned())
        .map(|v| matches!(v.as_str(), "true" | "yes" | "on" | "1"))
        .unwrap_or(true);

    let group = match filter {
        JobsFilter::Running => "jobs",
        JobsFilter::Suspended => "jobs",
        JobsFilter::All => "jobs",
    };
    state.comp.begin_group(group, true);
    let mut emitted = false;
    for job in jobs {
        let pass = match filter {
            JobsFilter::Running => job.state == JobState::Running,
            JobsFilter::Suspended => job.state == JobState::Suspended,
            JobsFilter::All => true,
        };
        if !pass {
            continue;
        }
        let spec = format!("{}{}", pfx, job.id);
        // Compare against the user's typed prefix.
        if !spec.starts_with(&prefix) {
            // Allow `%cmd` substring style too — only if prefix starts
            // with `%` and is followed by something matching the
            // job text.
            if !(prefix.starts_with('%')
                && !prefix[1..].is_empty()
                && job.text.contains(&prefix[1..]))
            {
                continue;
            }
        }
        let mut comp = Completion::new(spec.clone());
        if verbose {
            comp.disp = Some(format!("{} -- {}", spec, job.text));
        }
        state.comp.add_match(comp, Some(group));
        emitted = true;
    }
    state.comp.end_group();
    emitted
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: i32, text: &str, state: JobState) -> JobEntry<'_> {
        JobEntry { id, text, state }
    }

    #[test]
    fn all_jobs_emitted_with_percent_prefix() {
        let mut state = MainCompleteState::new("", 0);
        let jobs = vec![
            entry(1, "vim", JobState::Suspended),
            entry(2, "make", JobState::Running),
        ];
        let _ = _jobs(&mut state, &jobs, JobsFilter::All, false);
        let names: Vec<&str> = state.comp.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"%1"));
        assert!(names.contains(&"%2"));
    }

    #[test]
    fn running_filter_excludes_suspended() {
        let mut state = MainCompleteState::new("", 0);
        let jobs = vec![
            entry(1, "vim", JobState::Suspended),
            entry(2, "make", JobState::Running),
        ];
        let _ = _jobs(&mut state, &jobs, JobsFilter::Running, false);
        let names: Vec<&str> = state.comp.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert_eq!(names, vec!["%2"]);
    }

    #[test]
    fn suspended_filter_excludes_running() {
        let mut state = MainCompleteState::new("", 0);
        let jobs = vec![
            entry(1, "vim", JobState::Suspended),
            entry(2, "make", JobState::Running),
        ];
        let _ = _jobs(&mut state, &jobs, JobsFilter::Suspended, false);
        let names: Vec<&str> = state.comp.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert_eq!(names, vec!["%1"]);
    }

    #[test]
    fn prefix_needed_check_blocks_when_no_percent() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "bare".into();
        let jobs = vec![entry(1, "vim", JobState::Suspended)];
        // -t + no %-prefix → false.
        assert!(!_jobs(&mut state, &jobs, JobsFilter::All, true));
    }

    #[test]
    fn prefix_hidden_drops_percent() {
        let mut state = MainCompleteState::new("", 0);
        state.ctx.context = ":t:".into();
        state.styles.set(
            ":completion::t::jobs",
            "prefix-hidden",
            vec!["true".into()],
            false,
        );
        let jobs = vec![entry(1, "vim", JobState::Running)];
        let _ = _jobs(&mut state, &jobs, JobsFilter::All, false);
        let names: Vec<&str> = state.comp.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert_eq!(names, vec!["1"]);
    }

    #[test]
    fn verbose_attaches_job_text_as_disp() {
        let mut state = MainCompleteState::new("", 0);
        let jobs = vec![entry(1, "vim file.rs", JobState::Suspended)];
        let _ = _jobs(&mut state, &jobs, JobsFilter::All, false);
        let disp = state.comp.groups[0].matches[0]
            .disp
            .as_deref()
            .unwrap_or("");
        assert!(disp.contains("vim file.rs"));
    }

    #[test]
    fn empty_jobs_returns_false() {
        let mut state = MainCompleteState::new("", 0);
        assert!(!_jobs(&mut state, &[], JobsFilter::All, false));
    }
}

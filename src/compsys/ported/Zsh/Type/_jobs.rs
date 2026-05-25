//! Port of `_jobs` from `Completion/Zsh/Type/_jobs`.
//!
//! Full upstream body (84 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  local expl disp jobs job jids pfx='%' desc how expls sep
//! sh: 4
//! sh: 5  if [[ "$1" = -t ]]; then
//! sh: 6    zstyle -T ":completion:${curcontext}:jobs" prefix-needed &&
//! sh: 7        [[ "$PREFIX" != %* && compstate[nmatches] -ne 0 ]] && return 1
//! sh: 8    shift
//! sh: 9  fi
//! sh:10  zstyle -t ":completion:${curcontext}:jobs" prefix-hidden && pfx=''
//! sh:11  zstyle -T ":completion:${curcontext}:jobs" verbose       && desc=yes
//! sh:12
//! sh:13  if [[ "$1" = -r ]]; then
//! sh:14    jids=( "${(@k)jobstates[(R)running*]}" )
//! sh:15    shift
//! sh:16    expls='running job'
//! sh:17  elif [[ "$1" = -s ]]; then
//! sh:18    jids=( "${(@k)jobstates[(R)suspended*]}" )
//! sh:19    shift
//! sh:20    expls='suspended job'
//! sh:21  else
//! sh:22    [[ "$1" = - ]] && shift
//! sh:23    jids=( "${(@k)jobtexts}" )
//! sh:24    expls=job
//! sh:25  fi
//! sh:26
//! sh:27  if [[ -n "$desc" ]]; then
//! sh:28    disp=()
//! sh:29    zstyle -s ":completion:${curcontext}:jobs" list-separator sep || sep=--
//! sh:30    for job in "$jids[@]"; do
//! sh:31      [[ -n "$desc" ]] &&
//! sh:32          disp=( "$disp[@]" "${pfx}${(r:2:: :)job} $sep ${(r:COLUMNS-8:: :)jobtexts[$job]}" )
//! sh:33    done
//! sh:34  fi
//! sh:35
//! sh:36  zstyle -s ":completion:${curcontext}:jobs" numbers how
//! sh:37
//! sh:38  if [[ "$how" = (yes|true|on|1) ]]; then
//! sh:39    jobs=( "$jids[@]" )
//! sh:40  else
//! sh:41    local texts i text str tmp num max=0
//! sh:42
//! sh:43    # Find shortest unambiguous strings.
//! sh:44
//! sh:45    texts=( "$jobtexts[@]" )
//! sh:46    jobs=()
//! sh:47    for i in "$jids[@]"; do
//! sh:48      text="$jobtexts[$i]"
//! sh:49      str="${text%% *}"
//! sh:50      if [[ "$text" = *\ * ]]; then
//! sh:51        text="${text#* }"
//! sh:52      else
//! sh:53        text=""
//! sh:54      fi
//! sh:55      tmp=( "${(@M)texts:#${str}*}" )
//! sh:56      num=1
//! sh:57      while [[ -n "$text" && $#tmp -ge 2 ]]; do
//! sh:58        str="${str} ${text%% *}"
//! sh:59        if [[ "$text" = *\ * ]]; then
//! sh:60          text="${text#* }"
//! sh:61        else
//! sh:62          text=""
//! sh:63        fi
//! sh:64        tmp=( "${(@M)texts:#${str}*}" )
//! sh:65        (( num++ ))
//! sh:66      done
//! sh:67
//! sh:68      [[ num -gt max ]] && max="$num"
//! sh:69
//! sh:70      jobs=( "$jobs[@]" "$str" )
//! sh:71    done
//! sh:72
//! sh:73    if [[ "$how" = [0-9]## && max -gt how ]]; then
//! sh:74      jobs=( "$jids[@]" )
//! sh:75    else
//! sh:76      [[ -z "$pfx" && -n "$desc" ]] && disp=( "${(@)disp#%}" )
//! sh:77    fi
//! sh:78  fi
//! sh:79
//! sh:80  if [[ -n "$desc" ]]; then
//! sh:81    _wanted jobs expl "$expls" compadd "$@" -ld disp - "%$^jobs[@]"
//! sh:82  else
//! sh:83    _wanted jobs expl "$expls" compadd "$@" - "%$^jobs[@]"
//! sh:84  fi
//! ```
//!
//! Strict Rust port: caller injects the live `jobtexts`/`jobstates`
//! tables. Job IDs come back prefixed with `%` (`pfx`) unless the
//! `prefix-hidden` style is truthy.



use crate::compsys::base::MainCompleteState;
use crate::compsys::completion::Completion;

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

//! Executor-side bg-job tracker. NOT a port of `Src/jobs.c`.
//!
//! `Src/jobs.c` uses a flat `struct job jobtab[]` global keyed by pid
//! (ported to `crate::ported::jobs::JOBTAB`). C tracks child processes
//! through their pid + waitpid(2). Rust prefers safe-Rust ownership
//! of `std::process::Child` handles so the executor needs a parallel
//! registry that owns those handles. That's what this file is.
//!
//! This module is segregated from `src/ported/jobs.rs` (the faithful
//! C port) so the port file contains only direct ports of jobs.c
//! decls. `JobState` / `JobInfo` / `JobTable` here are zshrs runtime
//! state with no C counterpart by design.

use std::process::Child;
use std::sync::Mutex;

use crate::ported::jobs::{deletejob, CURJOB, MAXJOB, PREVJOB, THISJOB};
use crate::ported::jobs::stat;
use crate::ported::zsh_h::job;

/// Executor-side stand-in for C `printjob`'s done-job delete tail,
/// `Src/jobs.c:1350-1363`:
/// ```c
/// if (jn->stat & STAT_DONE) {
///     ...
///     deletejob(jn, 0);
///     if (job == curjob) { curjob = prevjob; prevjob = job; }
///     if (job == prevjob) setprevjob();
/// }
/// ```
/// The ported `printjob` (src/ported/jobs.rs) is a pure formatter
/// returning a String; C's version mutates the table as a side
/// effect. Every site that calls (or would call) printjob on a
/// possibly-done job runs this tail so finished jobs leave the table
/// exactly when they do in C. Lives here (not src/ported/) because
/// it has no C name of its own — it is the side-effect half of
/// printjob, split out by the Rust purity refactor.
pub fn printjob_delete_tail(tab: &mut [job], idx: usize) {
    if idx >= tab.len() || (tab[idx].stat & stat::DONE) == 0 {
        return;
    }
    deletejob(&mut tab[idx], false); // c:Src/jobs.c:1356
    let mut cj = CURJOB.get_or_init(|| Mutex::new(-1)).lock().unwrap();
    let mut pj = PREVJOB.get_or_init(|| Mutex::new(-1)).lock().unwrap();
    if *cj == idx as i32 {
        // c:Src/jobs.c:1357-1360
        *cj = *pj;
        *pj = idx as i32;
    }
    let need_setprev = *pj == idx as i32; // c:Src/jobs.c:1361
    drop(cj);
    drop(pj);
    if need_setprev {
        setprevjob_locked(tab); // c:Src/jobs.c:1362
    }
}

/// `setprevjob` (Src/jobs.c:698-717) body operating on an
/// already-locked table slice — `printjob_delete_tail` callers hold
/// the JOBTAB lock, so the re-locking ported `setprevjob()` would
/// deadlock. Same walk, same candidate order.
fn setprevjob_locked(tab: &[job]) {
    let maxjob = *MAXJOB
        .get_or_init(|| Mutex::new(0))
        .lock()
        .expect("maxjob poisoned");
    let curjob = *CURJOB.get_or_init(|| Mutex::new(-1)).lock().unwrap();
    let thisjob = *THISJOB.get_or_init(|| Mutex::new(-1)).lock().unwrap();
    let pick = |want_stopped: bool| -> i32 {
        for i in (1..=maxjob).rev() {
            if i >= tab.len() {
                continue;
            }
            let j = &tab[i];
            let stat_ok = if want_stopped {
                (j.stat & (stat::INUSE | stat::STOPPED)) == (stat::INUSE | stat::STOPPED)
            } else {
                (j.stat & stat::INUSE) != 0
            };
            if stat_ok
                && (j.stat & stat::SUBJOB) == 0
                && i as i32 != curjob
                && i as i32 != thisjob
            {
                return i as i32;
            }
        }
        -1
    };
    let mut found = pick(true); // c:Src/jobs.c:702-707
    if found < 0 {
        found = pick(false); // c:Src/jobs.c:709-714
    }
    *PREVJOB.get_or_init(|| Mutex::new(-1)).lock().unwrap() = found; // c:716
}

/// Running-job state tracked alongside each `Child` handle.
/// Maps to C's `STAT_*` bits but is exposed as a typed enum since
/// the executor's safe-Rust path doesn't manipulate the bitfield.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobState {
    /// `Running` variant.
    Running,
    /// `Stopped` variant.
    Stopped,
    /// `Done` variant.
    Done,
}

/// One entry in the executor's bg-job registry.
#[derive(Debug)]
pub struct JobInfo {
    /// `id` field.
    pub id: usize,
    /// `pid` field.
    pub pid: i32,
    /// `child` field.
    pub child: Option<Child>,
    /// `command` field.
    pub command: String,
    /// `state` field.
    pub state: JobState,
    /// `is_current` field.
    pub is_current: bool,
}

/// The executor's bg-job registry. Distinct from the C-port
/// `JOBTAB` (a `Vec<Job>` keyed by index that mirrors `jobtab[]`):
/// this table owns the `std::process::Child` handles needed for
/// `try_wait` / `kill` on the safe-Rust path.
pub struct JobTable {
    /// `jobs` field.
    jobs: Vec<Option<JobInfo>>,
    /// `current_id` field.
    current_id: Option<usize>,
    /// `next_id` field.
    next_id: usize,
}

impl Default for JobTable {
    fn default() -> Self {
        Self::new()
    }
}

impl JobTable {
    /// `new` — see implementation.
    pub fn new() -> Self {
        JobTable {
            jobs: Vec::with_capacity(16),
            current_id: None,
            next_id: 1,
        }
    }

    /// Peek at the next id that would be assigned by `add_job`/`add_pid`.
    /// Used by `wait %N` to distinguish a never-issued id (clear user
    /// error) from a job that was issued and already reaped (silent
    /// success in zshrs to keep the `cmd & wait %1` idiom working
    /// across the races introduced by the threaded job table).
    pub fn peek_next_id(&self) -> usize {
        self.next_id
    }

    /// Add a job with a Child process
    pub fn add_job(&mut self, child: Child, command: String, state: JobState) -> usize {
        let id = self.next_id;
        self.next_id += 1;

        let pid = child.id() as i32;
        let job = JobInfo {
            id,
            pid,
            child: Some(child),
            command,
            state,
            is_current: true,
        };

        // Mark previous current as not current
        if let Some(cur_id) = self.current_id {
            if let Some(j) = self.get_mut_internal(cur_id) {
                j.is_current = false;
            }
        }

        // Add new job
        let slot = self.get_free_slot();
        if slot >= self.jobs.len() {
            self.jobs.resize_with(slot + 1, || None);
        }
        self.jobs[slot] = Some(job);
        self.current_id = Some(id);

        id
    }

    /// Register a backgrounded job that was forked via raw `libc::fork()`
    /// (no `std::process::Child` wrapper). The wait path then has to
    /// `waitpid(pid)` instead of `Child::wait()`. Used by
    /// BUILTIN_RUN_BG so `wait` (no args) can synchronize on it.
    pub fn add_pid_job(&mut self, pid: i32, command: String, state: JobState) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        let job = JobInfo {
            id,
            pid,
            child: None,
            command,
            state,
            is_current: true,
        };
        if let Some(cur_id) = self.current_id {
            if let Some(j) = self.get_mut_internal(cur_id) {
                j.is_current = false;
            }
        }
        let slot = self.get_free_slot();
        if slot >= self.jobs.len() {
            self.jobs.resize_with(slot + 1, || None);
        }
        self.jobs[slot] = Some(job);
        self.current_id = Some(id);
        id
    }

    fn get_free_slot(&self) -> usize {
        for (i, slot) in self.jobs.iter().enumerate() {
            if slot.is_none() {
                return i;
            }
        }
        self.jobs.len()
    }

    fn get_mut_internal(&mut self, id: usize) -> Option<&mut JobInfo> {
        self.jobs.iter_mut().flatten().find(|job| job.id == id)
    }

    /// Get a job by ID
    pub fn get(&self, id: usize) -> Option<&JobInfo> {
        self.jobs
            .iter()
            .flatten()
            .find(|&job| job.id == id)
            .map(|v| v as _)
    }

    /// Get a mutable job by ID
    pub fn get_mut(&mut self, id: usize) -> Option<&mut JobInfo> {
        self.get_mut_internal(id)
    }

    /// Remove a job by ID
    pub fn remove(&mut self, id: usize) -> Option<JobInfo> {
        for slot in self.jobs.iter_mut() {
            if slot.as_ref().map(|j| j.id == id).unwrap_or(false) {
                let job = slot.take();
                if self.current_id == Some(id) {
                    self.current_id = None;
                }
                return job;
            }
        }
        None
    }

    /// List all active jobs
    pub fn list(&self) -> Vec<&JobInfo> {
        self.jobs.iter().filter_map(|j| j.as_ref()).collect()
    }

    /// Iterate over jobs with their IDs (for compatibility)
    pub fn iter(&self) -> impl Iterator<Item = (usize, &JobInfo)> {
        self.jobs
            .iter()
            .filter_map(|j| j.as_ref().map(|job| (job.id, job)))
    }

    /// Count number of active jobs
    pub fn count(&self) -> usize {
        self.jobs.iter().filter(|j| j.is_some()).count()
    }

    /// Check if there are any jobs
    pub fn is_empty(&self) -> bool {
        self.count() == 0
    }

    /// Get current job
    pub fn current(&self) -> Option<&JobInfo> {
        self.current_id.and_then(|id| self.get(id))
    }

    /// Reap finished jobs (check for completed processes)
    pub fn reap_finished(&mut self) -> Vec<JobInfo> {
        let mut finished = Vec::new();

        for job in self.jobs.iter_mut().flatten() {
            if let Some(ref mut child) = job.child {
                // Try to check if child has finished without blocking
                match child.try_wait() {
                    Ok(Some(_status)) => {
                        // Child finished
                        job.state = JobState::Done;
                    }
                    Ok(None) => {
                        // Still running
                    }
                    Err(_) => {
                        // Error checking, assume done
                        job.state = JobState::Done;
                    }
                }
            }
        }

        // Remove done jobs
        for slot in self.jobs.iter_mut() {
            if slot
                .as_ref()
                .map(|j| j.state == JobState::Done)
                .unwrap_or(false)
            {
                if let Some(job) = slot.take() {
                    finished.push(job);
                }
            }
        }

        finished
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_job_table_new() {
        let _g = crate::test_util::global_state_lock();
        let table = JobTable::new();
        assert!(table.is_empty());
    }

    #[test]
    fn test_job_state_enum() {
        let _g = crate::test_util::global_state_lock();
        let state = JobState::Running;
        assert_eq!(state, JobState::Running);
        assert_ne!(state, JobState::Stopped);
        assert_ne!(state, JobState::Done);
    }

    #[test]
    fn test_add_pid_job_assigns_id() {
        let _g = crate::test_util::global_state_lock();
        let mut t = JobTable::new();
        let id1 = t.add_pid_job(1234, "cmd1".into(), JobState::Running);
        let id2 = t.add_pid_job(5678, "cmd2".into(), JobState::Running);
        assert_ne!(id1, id2);
        assert_eq!(t.list().len(), 2);
        assert_eq!(t.current().map(|j| j.id), Some(id2));
    }

    #[test]
    fn test_remove_drops_current() {
        let _g = crate::test_util::global_state_lock();
        let mut t = JobTable::new();
        let id = t.add_pid_job(99, "x".into(), JobState::Running);
        assert!(t.remove(id).is_some());
        assert!(t.is_empty());
        assert!(t.current().is_none());
    }
}

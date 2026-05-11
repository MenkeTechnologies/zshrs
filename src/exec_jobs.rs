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

/// Running-job state tracked alongside each `Child` handle.
/// Maps to C's `STAT_*` bits but is exposed as a typed enum since
/// the executor's safe-Rust path doesn't manipulate the bitfield.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobState {
    Running,
    Stopped,
    Done,
}

/// One entry in the executor's bg-job registry.
#[derive(Debug)]
pub struct JobInfo {
    pub id: usize,
    pub pid: i32,
    pub child: Option<Child>,
    pub command: String,
    pub state: JobState,
    pub is_current: bool,
}

/// The executor's bg-job registry. Distinct from the C-port
/// `JOBTAB` (a `Vec<Job>` keyed by index that mirrors `jobtab[]`):
/// this table owns the `std::process::Child` handles needed for
/// `try_wait` / `kill` on the safe-Rust path.
pub struct JobTable {
    jobs: Vec<Option<JobInfo>>,
    current_id: Option<usize>,
    next_id: usize,
}

impl Default for JobTable {
    fn default() -> Self {
        Self::new()
    }
}

impl JobTable {
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
        let table = JobTable::new();
        assert!(table.is_empty());
    }

    #[test]
    fn test_job_state_enum() {
        let state = JobState::Running;
        assert_eq!(state, JobState::Running);
        assert_ne!(state, JobState::Stopped);
        assert_ne!(state, JobState::Done);
    }

    #[test]
    fn test_add_pid_job_assigns_id() {
        let mut t = JobTable::new();
        let id1 = t.add_pid_job(1234, "cmd1".into(), JobState::Running);
        let id2 = t.add_pid_job(5678, "cmd2".into(), JobState::Running);
        assert_ne!(id1, id2);
        assert_eq!(t.list().len(), 2);
        assert_eq!(t.current().map(|j| j.id), Some(id2));
    }

    #[test]
    fn test_remove_drops_current() {
        let mut t = JobTable::new();
        let id = t.add_pid_job(99, "x".into(), JobState::Running);
        assert!(t.remove(id).is_some());
        assert!(t.is_empty());
        assert!(t.current().is_none());
    }
}

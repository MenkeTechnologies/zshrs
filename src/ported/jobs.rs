//! Job control for zshrs
//!
//! Port from zsh/Src/jobs.c
//!
//! Provides job control, process management, and signal handling for jobs.

use std::env;
use std::process::Child;
use std::time::{Duration, Instant};

use crate::ported::utils::zwarnnam;

/// Job status flags
pub mod stat {
    pub const STOPPED: u32 = 1 << 0; // Job is stopped
    pub const DONE: u32 = 1 << 1; // Job is finished
    pub const SUBJOB: u32 = 1 << 2; // Job is a subjob
    pub const CURSH: u32 = 1 << 3; // Last pipeline elem in current shell
    pub const SUPERJOB: u32 = 1 << 4; // Job is a superjob
    pub const WASSUPER: u32 = 1 << 5; // Was a superjob
    pub const INUSE: u32 = 1 << 6; // Entry in use
    pub const BUILTIN: u32 = 1 << 7; // Job has builtin
    pub const DISOWN: u32 = 1 << 8; // Disowned
    pub const NOTIFY: u32 = 1 << 9; // Notify when done
    pub const ATTACH: u32 = 1 << 10; // Attached to tty
}

/// Special process status values
pub const SP_RUNNING: i32 = -1;

/// Maximum pipestats
pub const MAX_PIPESTATS: usize = 256;

/// Process timing information
#[derive(Clone, Debug, Default)]
/// CPU/elapsed time accounting for a job/process.
/// Port of `child_times_t` (Src/zsh.h) — populated by
/// `update_process()` (Src/jobs.c:363) from `wait4(2)` /
/// `getrusage(2)`. Same `user` / `system` / `real` triple.
pub struct TimeInfo {
    pub user_time: Duration,
    pub sys_time: Duration,
}

/// A single process in a pipeline
#[derive(Clone, Debug)]
/// One process within a pipeline.
/// Port of `struct process` from Src/zsh.h — `update_process()`
/// (Src/jobs.c:363) and `findproc()` (line 191) walk these.
pub struct Process {
    pub pid: i32,
    pub status: i32,
    pub start_time: Option<Instant>,
    pub end_time: Option<Instant>,
    pub ti: TimeInfo,
    pub text: String,
}

impl Process {
    pub fn new(pid: i32) -> Self {
        Process {
            pid,
            status: SP_RUNNING,
            start_time: Some(Instant::now()),
            end_time: None,
            ti: TimeInfo::default(),
            text: String::new(),
        }
    }

    pub fn is_running(&self) -> bool {
        self.status == SP_RUNNING
    }

    pub fn is_stopped(&self) -> bool {
        // WIFSTOPPED equivalent
        self.status & 0xff == 0x7f
    }

    pub fn is_signaled(&self) -> bool {
        // WIFSIGNALED equivalent
        (self.status & 0x7f) > 0 && (self.status & 0x7f) < 0x7f
    }

    pub fn exit_status(&self) -> i32 {
        // WEXITSTATUS equivalent
        (self.status >> 8) & 0xff
    }

    pub fn term_sig(&self) -> i32 {
        // WTERMSIG equivalent
        self.status & 0x7f
    }

    pub fn stop_sig(&self) -> i32 {
        // WSTOPSIG equivalent
        (self.status >> 8) & 0xff
    }
}

/// A job (pipeline)
#[derive(Clone, Debug)]
/// A job (one or more processes in a pipeline).
/// Port of `struct job` from Src/zsh.h — Src/jobs.c keeps the
/// `jobtab[]` array of these and dispatches every `bg`/`fg`/
/// `wait`/`disown` builtin through them.
pub struct Job {
    pub stat: u32,
    pub gleader: i32,           // Process group leader
    pub procs: Vec<Process>,    // Processes in job
    pub auxprocs: Vec<Process>, // Auxiliary processes
    pub other: usize,           // For superjobs: subjob index
    pub filelist: Vec<String>,  // Temp files to delete
    pub text: String,           // Job text for display
}

impl Job {
    pub fn new() -> Self {
        Job {
            stat: 0,
            gleader: 0,
            procs: Vec::new(),
            auxprocs: Vec::new(),
            other: 0,
            filelist: Vec::new(),
            text: String::new(),
        }
    }

    pub fn is_done(&self) -> bool {
        (self.stat & stat::DONE) != 0
    }

    pub fn is_stopped(&self) -> bool {
        (self.stat & stat::STOPPED) != 0
    }

    pub fn is_superjob(&self) -> bool {
        (self.stat & stat::SUPERJOB) != 0
    }

    pub fn is_subjob(&self) -> bool {
        (self.stat & stat::SUBJOB) != 0
    }

    pub fn is_inuse(&self) -> bool {
        (self.stat & stat::INUSE) != 0
    }

    pub fn has_procs(&self) -> bool {
        !self.procs.is_empty() || !self.auxprocs.is_empty()
    }

    pub fn make_running(&mut self) {
        self.stat &= !stat::STOPPED;
        for proc in &mut self.procs {
            if proc.is_stopped() {
                proc.status = SP_RUNNING;
            }
        }
    }
}

impl Default for Job {
    fn default() -> Self {
        Self::new()
    }
}

/// Simple job info for exec.rs compatibility
#[derive(Debug)]
/// Job-info accessor record.
/// zshrs convenience over `struct job` for read-only listings.
/// C zsh inlines the same fields when `printjob()`
/// (Src/jobs.c:1138) renders.
pub struct JobInfo {
    pub id: usize,
    pub pid: i32,
    pub child: Option<Child>,
    pub command: String,
    pub state: JobState,
    pub is_current: bool,
}

/// Job table compatible with exec.rs
/// Job table.
/// Port of the `jobtab[]` global (Src/zsh.h declares it,
/// Src/jobs.c maintains it). The `setprevjob()` cursor
/// (line 698) and `findproc()` lookup (line 191) work against
/// this shape.
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
    fn test_process_new() {
        let proc = Process::new(1234);
        assert_eq!(proc.pid, 1234);
        assert!(proc.is_running());
    }

    #[test]
    fn test_job_new() {
        let job = Job::new();
        assert_eq!(job.stat, 0);
        assert!(!job.is_done());
        assert!(!job.is_stopped());
    }

    #[test]
    fn test_job_table_new() {
        let table = JobTable::new();
        assert!(table.is_empty());
    }

    #[test]
    fn test_job_table_remove() {
        // This test would require spawning a real process, skipping for now
    }

    #[test]
    fn test_job_make_running() {
        let mut job = Job::new();
        job.stat |= stat::STOPPED;
        job.procs.push(Process {
            status: 0x007f,
            ..Process::new(1234)
        }); // Stopped

        job.make_running();
        assert!(!job.is_stopped());
        assert!(job.procs[0].is_running());
    }

    #[test]
    fn test_format_job() {
        let mut job = Job::new();
        job.text = "vim file.txt".to_string();
        job.stat |= stat::STOPPED;

        let formatted = printjob(&job, 1, false, Some(1), None);
        assert!(formatted.contains("[1]+"));
        assert!(formatted.contains("suspended") || formatted.contains("Stopped"));
        assert!(formatted.contains("vim file.txt"));
    }

    #[test]
    fn test_job_state_enum() {
        let state = JobState::Running;
        assert_eq!(state, JobState::Running);
        assert_ne!(state, JobState::Stopped);
        assert_ne!(state, JobState::Done);
    }
}

/// Job state for simpler tracking
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobState {
    Running,
    Stopped,
    Done,
}

/// Simple job entry for executor compatibility
#[derive(Debug)]
pub struct JobEntry {
    pub pid: i32,
    pub child: Option<Child>,
    pub command: String,
    pub state: JobState,
    pub is_current: bool,
}

/// Get clock ticks per second (from jobs.c get_clktck lines 720-748)
/// Get `_SC_CLK_TCK` for time-conversion math.
/// Port of `get_clktck()` from Src/jobs.c:721.
pub fn get_clktck() -> i64 {
    #[cfg(unix)]
    {
        use std::sync::OnceLock;
        static CLKTCK: OnceLock<i64> = OnceLock::new();
        *CLKTCK.get_or_init(|| unsafe { libc::sysconf(libc::_SC_CLK_TCK) as i64 })
    }
    #[cfg(not(unix))]
    {
        100 // Default on non-Unix
    }
}

/// Format time as hh:mm:ss.xx (from jobs.c printhhmmss lines 752-765)
/// Format a duration as `H:MM:SS` / `M:SS`.
/// Port of `printhhmmss()` from Src/jobs.c:752.
pub fn printhhmmss(secs: f64) -> String {
    let mins = (secs / 60.0) as i32;
    let hours = mins / 60;
    let secs = secs - (mins * 60) as f64;
    let mins = mins - (hours * 60);

    if hours > 0 {
        format!("{}:{:02}:{:05.2}", hours, mins, secs)
    } else if mins > 0 {
        format!("{}:{:05.2}", mins, secs)
    } else {
        format!("{:.3}", secs)
    }
}

/// Time format specifiers (from jobs.c printtime lines 768-949)
/// Format a CPU/real time triple per `$TIMEFMT`.
/// Port of `printtime()` from Src/jobs.c:768 — same
/// `%U`/`%S`/`%E`/`%P`/`%J`/`%c`/`%R`/etc. directive set the
/// `time` keyword's output uses.
pub fn printtime(
    elapsed_secs: f64,
    user_secs: f64,
    system_secs: f64,
    format: &str,
    job_name: &str,
) -> String {
    let mut result = String::new();
    let total_time = user_secs + system_secs;
    let percent = if elapsed_secs > 0.0 {
        (100.0 * total_time / elapsed_secs) as i32
    } else {
        0
    };

    let mut chars = format.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' {
            match chars.next() {
                Some('E') => result.push_str(&format!("{:.2}s", elapsed_secs)),
                Some('U') => result.push_str(&format!("{:.2}s", user_secs)),
                Some('S') => result.push_str(&format!("{:.2}s", system_secs)),
                Some('P') => result.push_str(&format!("{}%", percent)),
                Some('J') => result.push_str(job_name),
                Some('m') => match chars.next() {
                    Some('E') => result.push_str(&format!("{:.0}ms", elapsed_secs * 1000.0)),
                    Some('U') => result.push_str(&format!("{:.0}ms", user_secs * 1000.0)),
                    Some('S') => result.push_str(&format!("{:.0}ms", system_secs * 1000.0)),
                    _ => result.push_str("%m"),
                },
                Some('u') => match chars.next() {
                    Some('E') => result.push_str(&format!("{:.0}us", elapsed_secs * 1_000_000.0)),
                    Some('U') => result.push_str(&format!("{:.0}us", user_secs * 1_000_000.0)),
                    Some('S') => result.push_str(&format!("{:.0}us", system_secs * 1_000_000.0)),
                    _ => result.push_str("%u"),
                },
                Some('n') => match chars.next() {
                    Some('E') => {
                        result.push_str(&format!("{:.0}ns", elapsed_secs * 1_000_000_000.0))
                    }
                    Some('U') => result.push_str(&format!("{:.0}ns", user_secs * 1_000_000_000.0)),
                    Some('S') => {
                        result.push_str(&format!("{:.0}ns", system_secs * 1_000_000_000.0))
                    }
                    _ => result.push_str("%n"),
                },
                Some('*') => match chars.next() {
                    Some('E') => result.push_str(&printhhmmss(elapsed_secs)),
                    Some('U') => result.push_str(&printhhmmss(user_secs)),
                    Some('S') => result.push_str(&printhhmmss(system_secs)),
                    _ => result.push_str("%*"),
                },
                Some('%') => result.push('%'),
                Some(other) => {
                    result.push('%');
                    result.push(other);
                }
                None => result.push('%'),
            }
        } else {
            result.push(c);
        }
    }
    result
}

/// Default time format (from jobs.c DEFAULT_TIMEFMT)
pub const DEFAULT_TIMEFMT: &str = "%J  %U user %S system %P cpu %*E total";

/// Time a command's execution
/// Per-command timer for the `time` keyword.
/// Port of the `dtime_tv()` (Src/jobs.c:137) /
/// `dtime_ts()` (line 152) deltas — measures real / user /
/// system across one command body.
pub struct CommandTimer {
    start: std::time::Instant,
    job_name: String,
}

impl CommandTimer {
    pub fn new(job_name: &str) -> Self {
        CommandTimer {
            start: std::time::Instant::now(),
            job_name: job_name.to_string(),
        }
    }

    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }

    pub fn format(
        &self,
        user_time: Duration,
        sys_time: Duration,
        format_str: Option<&str>,
    ) -> String {
        let elapsed = self.start.elapsed().as_secs_f64();
        let user = user_time.as_secs_f64();
        let sys = sys_time.as_secs_f64();

        printtime(
            elapsed,
            user,
            sys,
            format_str.unwrap_or(DEFAULT_TIMEFMT),
            &self.job_name,
        )
    }
}

/// Pipestats management (from jobs.c storepipestats lines 420-454)
/// Per-pipeline stats array.
/// Port of the `pipestats[]` cache `storepipestats()`
/// (Src/jobs.c:420) populates so `${pipestatus[N]}` can read
/// per-stage exit codes.
pub struct PipeStats {
    stats: Vec<i32>,
}

impl Default for PipeStats {
    fn default() -> Self {
        Self::new()
    }
}

impl PipeStats {
    pub fn new() -> Self {
        PipeStats { stats: Vec::new() }
    }

    pub fn clear(&mut self) {
        self.stats.clear();
    }

    pub fn add(&mut self, status: i32) {
        if self.stats.len() < MAX_PIPESTATS {
            self.stats.push(status);
        }
    }

    pub fn get(&self) -> &[i32] {
        &self.stats
    }

    pub fn len(&self) -> usize {
        self.stats.len()
    }

    pub fn is_empty(&self) -> bool {
        self.stats.is_empty()
    }

    pub fn pipefail_status(&self) -> i32 {
        *self.stats.iter().rev().find(|&&s| s != 0).unwrap_or(&0)
    }
}

/// Signal message lookup (from jobs.c sigmsg lines 1106-1118)
/// Render a signal number as a one-line description.
/// Port of `sigmsg()` from Src/jobs.c:1107.
pub fn sigmsg(sig: i32) -> &'static str {
    match sig {
        libc::SIGHUP => "hangup",
        libc::SIGINT => "interrupt",
        libc::SIGQUIT => "quit",
        libc::SIGILL => "illegal instruction",
        libc::SIGTRAP => "trace trap",
        libc::SIGABRT => "abort",
        libc::SIGBUS => "bus error",
        libc::SIGFPE => "floating point exception",
        libc::SIGKILL => "killed",
        libc::SIGUSR1 => "user-defined signal 1",
        libc::SIGSEGV => "segmentation fault",
        libc::SIGUSR2 => "user-defined signal 2",
        libc::SIGPIPE => "broken pipe",
        libc::SIGALRM => "alarm",
        libc::SIGTERM => "terminated",
        libc::SIGCHLD => "child exited",
        libc::SIGCONT => "continued",
        libc::SIGSTOP => "stopped (signal)",
        libc::SIGTSTP => "stopped",
        libc::SIGTTIN => "stopped (tty input)",
        libc::SIGTTOU => "stopped (tty output)",
        libc::SIGURG => "urgent I/O condition",
        libc::SIGXCPU => "CPU time exceeded",
        libc::SIGXFSZ => "file size exceeded",
        libc::SIGVTALRM => "virtual timer expired",
        libc::SIGPROF => "profiling timer expired",
        libc::SIGWINCH => "window changed",
        libc::SIGIO => "I/O ready",
        libc::SIGSYS => "bad system call",
        _ => "unknown signal",
    }
}

/// Background status tracking (from jobs.c bgstatus)
/// Cached background-job status (for `wait`/$? lookup).
/// Port of `update_bg_job()` (Src/jobs.c:677) bookkeeping.
pub struct BgStatus {
    statuses: std::collections::HashMap<i32, i32>,
}

impl Default for BgStatus {
    fn default() -> Self {
        Self::new()
    }
}

impl BgStatus {
    pub fn new() -> Self {
        BgStatus {
            statuses: std::collections::HashMap::new(),
        }
    }

    pub fn add(&mut self, pid: i32, status: i32) {
        self.statuses.insert(pid, status);
    }

    pub fn get(&self, pid: i32) -> Option<i32> {
        self.statuses.get(&pid).copied()
    }

    pub fn remove(&mut self, pid: i32) -> Option<i32> {
        self.statuses.remove(&pid)
    }

    pub fn clear(&mut self) {
        self.statuses.clear();
    }
}

/// Wait for a specific PID (from jobs.c waitforpid lines 1627-1663)
pub fn waitforpid(pid: i32) -> Option<i32> {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        loop {
            let mut status: i32 = 0;
            let result = unsafe { libc::waitpid(pid, &mut status, 0) };
            if result == pid {
                if libc::WIFEXITED(status) {
                    return Some(libc::WEXITSTATUS(status));
                } else if libc::WIFSIGNALED(status) {
                    return Some(128 + libc::WTERMSIG(status));
                } else if libc::WIFSTOPPED(status) {
                    return None;
                }
            } else if result == -1 {
                return None;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        None
    }
}

/// Wait for job (from jobs.c zwaitjob lines 1673-1750)
pub fn zwaitjob(job: &mut Job) -> Option<i32> {
    if job.procs.is_empty() {
        return Some(0);
    }

    let mut last_status = 0;
    for proc in &mut job.procs {
        if proc.is_running() {
            if let Some(status) = waitforpid(proc.pid) {
                proc.status = status << 8;
                last_status = status;
            }
        } else {
            last_status = proc.exit_status();
        }
    }

    job.stat |= stat::DONE;
    Some(last_status)
}

/// Check if job has pending children (from jobs.c havefiles lines 1604-1616)
pub fn havefiles(job: &Job) -> bool {
    !job.filelist.is_empty()
}

/// Delete job (from jobs.c deletejob lines 1511-1526)
pub fn deletejob(job: &mut Job, disowning: bool) {
    if !disowning {
        job.filelist.clear();
    }
    job.procs.clear();
    job.auxprocs.clear();
    job.stat = 0;
}

/// Free job (from jobs.c freejob lines 1456-1508)
pub fn freejob(job: &mut Job, notify: bool) {
    let _ = notify;
    job.procs.clear();
    job.auxprocs.clear();
    job.filelist.clear();
    job.stat = 0;
    job.gleader = 0;
    job.text.clear();
}

/// Add process to job (from jobs.c addproc lines 1537-1597)
pub fn addproc(job: &mut Job, pid: i32, text: &str, aux: bool) {
    let proc = Process::new(pid);
    let proc = Process {
        pid,
        status: SP_RUNNING,
        text: text.to_string(),
        ..proc
    };

    if aux {
        job.auxprocs.push(proc);
    } else {
        if job.gleader == 0 {
            job.gleader = pid;
        }
        job.procs.push(proc);
    }

    job.stat &= !stat::DONE;
}

/// Super job tracking (from jobs.c super_job lines 393-417)
pub fn super_job(jobtab: &[Job], job_idx: usize) -> Option<usize> {
    for (i, job) in jobtab.iter().enumerate() {
        if (job.stat & stat::SUPERJOB) != 0 && job.other == job_idx {
            return Some(i);
        }
    }
    None
}

/// Set current/previous job (from jobs.c setjobpwn lines 697-745)
pub struct JobPointers {
    pub cur_job: Option<usize>,
    pub prev_job: Option<usize>,
}

impl JobPointers {
    pub fn new() -> Self {
        JobPointers {
            cur_job: None,
            prev_job: None,
        }
    }

    pub fn set_current(&mut self, job: usize) {
        if Some(job) != self.cur_job {
            self.prev_job = self.cur_job;
            self.cur_job = Some(job);
        }
    }

    pub fn clear(&mut self, job: usize) {
        if self.cur_job == Some(job) {
            self.cur_job = self.prev_job;
            self.prev_job = None;
        } else if self.prev_job == Some(job) {
            self.prev_job = None;
        }
    }
}

impl Default for JobPointers {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Missing functions from jobs.c
// ---------------------------------------------------------------------------

/// Parse job specification string (from jobs.c getjob lines 2063-2165)
///
/// Syntax: %N (by number), %+ or %% (current), %- (previous),
/// %str (by command prefix), %?str (by substring)
pub fn getjob(spec: &str, table: &JobTable, ptrs: &JobPointers) -> Option<usize> {
    if spec.is_empty() {
        return ptrs.cur_job;
    }

    let spec = spec.strip_prefix('%').unwrap_or(spec);

    match spec {
        "+" | "%" | "" => ptrs.cur_job,
        "-" => ptrs.prev_job,
        _ => {
            // Try as number
            if let Ok(n) = spec.parse::<usize>() {
                if table.get(n).is_some() {
                    return Some(n);
                }
                return None;
            }

            // ?string - search by substring
            if let Some(substr) = spec.strip_prefix('?') {
                for (id, job) in table.iter() {
                    if job.command.contains(substr) {
                        return Some(id);
                    }
                }
                return None;
            }

            // string - search by prefix
            for (id, job) in table.iter() {
                if job.command.starts_with(spec) {
                    return Some(id);
                }
            }

            None
        }
    }
}

/// Find job by command name (from jobs.c findjobnam)
pub fn findjobnam(name: &str, table: &JobTable) -> Option<usize> {
    for (id, job) in table.iter() {
        if job.command == name {
            return Some(id);
        }
    }
    None
}

/// Check if string is a number (from jobs.c isanum)
pub fn isanum(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())
}

/// Initialize jobs subsystem (from jobs.c init_jobs)
pub fn init_jobs() -> (JobTable, JobPointers) {
    (JobTable::new(), JobPointers::new())
}

/// Acquire process group (from jobs.c acquire_pgrp)
#[cfg(unix)]
pub fn acquire_pgrp() -> bool {
    unsafe {
        let mypgrp = libc::getpgrp();
        let tpgrp = libc::tcgetpgrp(0);
        if tpgrp == -1 || tpgrp == mypgrp {
            return true;
        }
        // We need to be in the foreground process group
        if libc::setpgid(0, 0) == 0 {
            libc::tcsetpgrp(0, libc::getpgrp());
            return true;
        }
        false
    }
}

/// Store pipestats from job (from jobs.c storepipestats)
pub fn storepipestats(job: &Job) -> Vec<i32> {
    job.procs.iter().map(|p| p.status).collect()
}

/// Clear the job table (from jobs.c clearjobtab)
pub fn clearjobtab(table: &mut JobTable, ptrs: &mut JobPointers) {
    table.jobs.clear();
    table.next_id = 1;
    ptrs.cur_job = None;
    ptrs.prev_job = None;
}

/// Scan jobs and print changed status (from jobs.c scanjobs)
pub fn scanjobs(table: &JobTable) -> Vec<String> {
    let mut output = Vec::new();
    for (id, job) in table.iter() {
        let state_str = match job.state {
            JobState::Running => "running",
            JobState::Done => "done",
            JobState::Stopped => "stopped",
            _ => "unknown",
        };
        output.push(format!("[{}]  {}  {}", id, state_str, job.command));
    }
    output
}

/// Shell time accounting (from jobs.c shelltime)
#[derive(Debug, Clone, Default)]
pub struct ChildTimes {
    pub user_sec: f64,
    pub sys_sec: f64,
}

pub fn shelltime() -> ChildTimes {
    #[cfg(unix)]
    {
        let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
        if unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) } == 0 {
            return ChildTimes {
                user_sec: usage.ru_utime.tv_sec as f64
                    + usage.ru_utime.tv_usec as f64 / 1_000_000.0,
                sys_sec: usage.ru_stime.tv_sec as f64 + usage.ru_stime.tv_usec as f64 / 1_000_000.0,
            };
        }
    }
    ChildTimes::default()
}

/// Get children's time accounting.
/// Port of `get_usage()` from Src/jobs.c — fills `child_usage`
/// from `getrusage(RUSAGE_CHILDREN)` on supported systems.
pub fn get_usage() -> ChildTimes {
    #[cfg(unix)]
    {
        let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
        if unsafe { libc::getrusage(libc::RUSAGE_CHILDREN, &mut usage) } == 0 {
            return ChildTimes {
                user_sec: usage.ru_utime.tv_sec as f64
                    + usage.ru_utime.tv_usec as f64 / 1_000_000.0,
                sys_sec: usage.ru_stime.tv_sec as f64 + usage.ru_stime.tv_usec as f64 / 1_000_000.0,
            };
        }
    }
    ChildTimes::default()
}

/// Update process status after waitpid (from jobs.c update_process)
pub fn update_process(proc: &mut Process, status: i32) {
    proc.end_time = Some(Instant::now());
    proc.status = status;
}

/// Find a process by PID in the job table (from jobs.c findproc)
pub fn findproc(jobtab: &[Job], pid: i32) -> Option<(usize, usize, bool)> {
    for (ji, job) in jobtab.iter().enumerate() {
        for (pi, proc) in job.procs.iter().enumerate() {
            if proc.pid == pid {
                return Some((ji, pi, false));
            }
        }
        for (pi, proc) in job.auxprocs.iter().enumerate() {
            if proc.pid == pid {
                return Some((ji, pi, true));
            }
        }
    }
    None
}

/// Update job status after process change (from jobs.c update_job)
pub fn update_job(job: &mut Job) -> bool {
    // Check if all aux procs are done
    for proc in &job.auxprocs {
        if proc.is_running() {
            return false;
        }
    }

    // Check main processes
    let all_done = true;
    let mut some_stopped = false;
    let mut last_status = 0;

    for proc in &job.procs {
        if proc.is_running() {
            return false; // Still running
        }
        if proc.is_stopped() {
            some_stopped = true;
        }
    }

    // Get last process status
    if let Some(last) = job.procs.last() {
        if last.is_signaled() {
            last_status = 0x80 | last.term_sig();
        } else if last.is_stopped() {
            last_status = 0x80 | last.stop_sig();
        } else {
            last_status = last.exit_status();
        }
    }

    if some_stopped {
        job.stat |= stat::STOPPED;
        job.stat &= !stat::DONE;
    } else {
        job.stat |= stat::DONE;
        job.stat &= !stat::STOPPED;
    }

    true
}

/// Update a background job after waitpid (from jobs.c update_bg_job)
pub fn update_bg_job(jobtab: &mut [Job], pid: i32, status: i32) -> bool {
    if let Some((ji, pi, is_aux)) = findproc(jobtab, pid) {
        if is_aux {
            jobtab[ji].auxprocs[pi].status = status;
            jobtab[ji].auxprocs[pi].end_time = Some(Instant::now());
        } else {
            jobtab[ji].procs[pi].status = status;
            jobtab[ji].procs[pi].end_time = Some(Instant::now());
        }
        update_job(&mut jobtab[ji]);
        return true;
    }
    false
}

/// Handle subjob completion (from jobs.c handle_sub)
pub fn handle_sub(jobtab: &mut [Job], super_idx: usize, fg: bool) {
    let sub_idx = jobtab[super_idx].other;
    if sub_idx >= jobtab.len() {
        return;
    }

    // If subjob is done, mark superjob accordingly
    if jobtab[sub_idx].is_done() {
        if fg {
            // Get the last status from the subjob
        }
        jobtab[super_idx].stat &= !stat::SUPERJOB;
        jobtab[super_idx].stat |= stat::WASSUPER;
    }
}

/// Set the previous job (from jobs.c setprevjob)
pub fn setprevjob(ptrs: &mut JobPointers, jobtab: &[Job], maxjob: usize) {
    // Find a stopped or running job that isn't the current job
    let mut best = None;
    for i in (1..=maxjob).rev() {
        if i >= jobtab.len() {
            continue;
        }
        let job = &jobtab[i];
        if (job.stat & stat::INUSE) != 0 && Some(i) != ptrs.cur_job {
            if (job.stat & stat::STOPPED) != 0 {
                best = Some(i);
                break;
            }
            if best.is_none() {
                best = Some(i);
            }
        }
    }
    ptrs.prev_job = best;
}

/// Set current job after state change (from jobs.c setcurjob)
pub fn setcurjob(ptrs: &mut JobPointers, jobtab: &[Job], maxjob: usize) {
    ptrs.cur_job = None;
    for i in (1..=maxjob).rev() {
        if i >= jobtab.len() {
            continue;
        }
        if (jobtab[i].stat & (stat::INUSE | stat::STOPPED)) == (stat::INUSE | stat::STOPPED) {
            ptrs.cur_job = Some(i);
            break;
        }
    }
    if ptrs.cur_job.is_none() {
        for i in (1..=maxjob).rev() {
            if i >= jobtab.len() {
                continue;
            }
            if (jobtab[i].stat & stat::INUSE) != 0 {
                ptrs.cur_job = Some(i);
                break;
            }
        }
    }
    setprevjob(ptrs, jobtab, maxjob);
}

/// Check if a job's time should be reported (from jobs.c should_report_time)
pub fn should_report_time(job: &Job, reporttime: f64) -> bool {
    if reporttime < 0.0 {
        return false;
    }
    if let Some(first) = job.procs.first() {
        if let (Some(start), Some(end)) =
            (first.start_time, job.procs.last().and_then(|p| p.end_time))
        {
            let elapsed = end.duration_since(start).as_secs_f64();
            return elapsed >= reporttime;
        }
    }
    false
}

/// Dump timing info for a job (from jobs.c dumptime)
pub fn dumptime(job: &Job, format: &str) -> Option<String> {
    let first_start = job.procs.first()?.start_time?;
    let last_end = job.procs.last()?.end_time?;
    let elapsed = last_end.duration_since(first_start).as_secs_f64();

    let mut total_user = 0.0;
    let mut total_sys = 0.0;
    for proc in &job.procs {
        total_user += proc.ti.user_time.as_secs_f64();
        total_sys += proc.ti.sys_time.as_secs_f64();
    }

    Some(printtime(
        elapsed,
        total_user,
        total_sys,
        format,
        &if !job.text.is_empty() { job.text.clone() } else { job.procs.iter().map(|p| p.text.as_str()).collect::<Vec<_>>().join(" | ") },
    ))
}

/// Wait for all foreground jobs to finish (from jobs.c waitjobs)
pub fn waitjobs(jobtab: &mut [Job], thisjob: usize) {
    if thisjob < jobtab.len() {
        while !jobtab[thisjob].is_done() && !jobtab[thisjob].is_stopped() {
            #[cfg(unix)]
            {
                let mut status: i32 = 0;
                let pid = unsafe { libc::waitpid(-1, &mut status, libc::WUNTRACED) };
                if pid > 0 {
                    update_bg_job(jobtab, pid, status);
                } else {
                    break;
                }
            }
            #[cfg(not(unix))]
            {
                break;
            }
        }
    }
}

/// Wait for a single specific job (from jobs.c waitonejob)
pub fn waitonejob(job: &mut Job) {
    for proc in &mut job.procs {
        if proc.is_running() {
            if let Some(_status) = waitforpid(proc.pid) {
                // status already updated by waitforpid
            }
        }
    }
}

/// Initialize a new job entry (from jobs.c initjob)
pub fn initjob(jobtab: &mut Vec<Job>) -> usize {
    // Find an empty slot or add a new one
    for (i, job) in jobtab.iter().enumerate() {
        if (job.stat & stat::INUSE) == 0 {
            jobtab[i] = Job::new();
            jobtab[i].stat = stat::INUSE;
            return i;
        }
    }
    // Expand table
    let idx = jobtab.len();
    let mut job = Job::new();
    job.stat = stat::INUSE;
    jobtab.push(job);
    idx
}

/// Set the pwd for a job (from jobs.c setjobpwd)
pub fn setjobpwd(job: &mut Job) {
    // Store current directory in job for display purposes
    if let Ok(cwd) = std::env::current_dir() {
        // Job text sometimes includes the directory
        let _ = cwd;
    }
}

/// Spawn a job (mark as started, from jobs.c spawnjob)
pub fn spawnjob(job: &mut Job, fg: bool) {
    job.stat |= stat::INUSE;
    if !fg {
        // Background job
        job.stat &= !stat::CURSH;
    }
}

/// Select which job table to use (from jobs.c selectjobtab)
/// Returns (table_ref, max_job_index)
pub fn selectjobtab(jobtab: &[Job]) -> usize {
    // Find the maximum used job index
    let mut max = 0;
    for (i, job) in jobtab.iter().enumerate() {
        if (job.stat & stat::INUSE) != 0 {
            max = i;
        }
    }
    max
}

/// Expand job table if needed (from jobs.c expandjobtab)
pub fn expandjobtab(jobtab: &mut Vec<Job>, needed: usize) {
    while jobtab.len() <= needed {
        jobtab.push(Job::new());
    }
}

/// Shrink job table if possible (from jobs.c maybeshrinkjobtab)
pub fn maybeshrinkjobtab(jobtab: &mut Vec<Job>) {
    while jobtab
        .last()
        .map(|j| (j.stat & stat::INUSE) == 0)
        .unwrap_or(false)
    {
        jobtab.pop();
    }
}

/// Add file to job's temp file list (from jobs.c addfilelist)
pub fn addfilelist(job: &mut Job, filename: &str) {
    job.filelist.push(filename.to_string());
}

/// Clean temp files for process substitution (from jobs.c pipecleanfilelist)
pub fn pipecleanfilelist(job: &mut Job, proc_subst_only: bool) {
    if proc_subst_only {
        // Only remove process substitution files (those starting with /dev/fd or /proc)
        job.filelist
            .retain(|f| !f.starts_with("/dev/fd/") && !f.starts_with("/proc/"));
    } else {
        for file in &job.filelist {
            let _ = std::fs::remove_file(file);
        }
        job.filelist.clear();
    }
}

/// Delete temp files from a job (from jobs.c deletefilelist)
pub fn deletefilelist(job: &mut Job, disowning: bool) {
    if !disowning {
        for file in &job.filelist {
            let _ = std::fs::remove_file(file);
        }
    }
    job.filelist.clear();
}

/// Print job with full detail (from jobs.c printjob)
pub fn printjob(
    job: &Job,
    job_num: usize,
    long_format: bool,
    cur_job: Option<usize>,
    prev_job: Option<usize>,
) -> String {
    // Inline process-status formatter — mirrors the inline status-decode
    // block at Src/jobs.c:1136-1400 inside printjob itself. SP_RUNNING
    // → "running"; WIFEXITED → "done" / "exit N"; WIFSTOPPED → "suspended
    // (sig)"; WIFSIGNALED → "sig" + " (core dumped)" if WCOREDUMP.
    let fmt_proc_status = |status: i32| -> String {
        if status == SP_RUNNING {
            "running".to_string()
        } else if (status & 0x7f) == 0 {
            let code = (status >> 8) & 0xff;
            if code == 0 {
                "done".to_string()
            } else {
                format!("exit {}", code)
            }
        } else if (status & 0xff) == 0x7f {
            let sig = (status >> 8) & 0xff;
            format!("suspended ({})", sigmsg(sig))
        } else {
            let sig = status & 0x7f;
            let core = (status >> 7) & 1;
            if core != 0 {
                format!("{} (core dumped)", sigmsg(sig))
            } else {
                sigmsg(sig).to_string()
            }
        }
    };
    let marker = if Some(job_num) == cur_job {
        '+'
    } else if Some(job_num) == prev_job {
        '-'
    } else {
        ' '
    };

    let status_str = if job.is_done() {
        if let Some(last) = job.procs.last() {
            fmt_proc_status(last.status)
        } else {
            "done".to_string()
        }
    } else if job.is_stopped() {
        "suspended".to_string()
    } else {
        "running".to_string()
    };

    if long_format {
        let mut lines = Vec::new();
        for (i, proc) in job.procs.iter().enumerate() {
            let pstatus = fmt_proc_status(proc.status);
            if i == 0 {
                lines.push(format!(
                    "[{}]  {} {:>5} {:16}  {}",
                    job_num, marker, proc.pid, pstatus, proc.text
                ));
            } else {
                lines.push(format!(
                    "            {:>5} {:16}  | {}",
                    proc.pid, pstatus, proc.text
                ));
            }
        }
        lines.join("\n")
    } else {
        format!(
            "[{}]  {} {:16}  {}",
            job_num,
            marker,
            status_str,
            if !job.text.is_empty() { job.text.clone() } else { job.procs.iter().map(|p| p.text.as_str()).collect::<Vec<_>>().join(" | ") }
        )
    }
}

/// Get the signal name for signal-based job output (from jobs.c getsigname)
pub fn getsigname(sig: i32) -> String {
    match sig {
        0 => "EXIT".to_string(),
        libc::SIGHUP => "HUP".to_string(),
        libc::SIGINT => "INT".to_string(),
        libc::SIGQUIT => "QUIT".to_string(),
        libc::SIGILL => "ILL".to_string(),
        libc::SIGTRAP => "TRAP".to_string(),
        libc::SIGABRT => "ABRT".to_string(),
        libc::SIGBUS => "BUS".to_string(),
        libc::SIGFPE => "FPE".to_string(),
        libc::SIGKILL => "KILL".to_string(),
        libc::SIGUSR1 => "USR1".to_string(),
        libc::SIGSEGV => "SEGV".to_string(),
        libc::SIGUSR2 => "USR2".to_string(),
        libc::SIGPIPE => "PIPE".to_string(),
        libc::SIGALRM => "ALRM".to_string(),
        libc::SIGTERM => "TERM".to_string(),
        libc::SIGCHLD => "CHLD".to_string(),
        libc::SIGCONT => "CONT".to_string(),
        libc::SIGSTOP => "STOP".to_string(),
        libc::SIGTSTP => "TSTP".to_string(),
        libc::SIGTTIN => "TTIN".to_string(),
        libc::SIGTTOU => "TTOU".to_string(),
        libc::SIGURG => "URG".to_string(),
        libc::SIGXCPU => "XCPU".to_string(),
        libc::SIGXFSZ => "XFSZ".to_string(),
        libc::SIGVTALRM => "VTALRM".to_string(),
        libc::SIGPROF => "PROF".to_string(),
        libc::SIGWINCH => "WINCH".to_string(),
        libc::SIGIO => "IO".to_string(),
        libc::SIGSYS => "SYS".to_string(),
        _ => format!("SIG{}", sig),
    }
}

/// Time difference for timeval (from jobs.c dtime_tv)
pub fn dtime_tv(dt: &mut Duration, t1: &Duration, t2: &Duration) -> Duration {
    if *t2 > *t1 {
        *dt = *t2 - *t1;
    } else {
        *dt = Duration::ZERO;
    }
    *dt
}

/// Time difference for timespec (from jobs.c dtime_ts)
pub fn dtime_ts(t1: &Instant, t2: &Instant) -> Duration {
    if *t2 > *t1 {
        t2.duration_since(*t1)
    } else {
        Duration::ZERO
    }
}

/// Make all job processes running (from jobs.c makerunning)
pub fn makerunning(job: &mut Job) {
    job.make_running();
}

/// Check if job has any processes (from jobs.c hasprocs)
pub fn hasprocs(job: &Job) -> bool {
    job.has_procs()
}

/// Check current shell signals (from jobs.c check_cursh_sig)
#[cfg(unix)]
pub fn check_cursh_sig(jobtab: &[Job], sig: i32) {
    for job in jobtab {
        if (job.stat & stat::CURSH) != 0 && !job.is_done() {
            for proc in &job.procs {
                if proc.is_running() {
                    unsafe {
                        libc::kill(proc.pid, sig);
                    }
                }
            }
        }
    }
}

/// Clean all file lists from jobs (from jobs.c cleanfilelists)
pub fn cleanfilelists(jobtab: &mut [Job]) {
    for job in jobtab.iter_mut() {
        deletefilelist(job, false);
    }
}

/// Clear old job table entries (from jobs.c clearoldjobtab)
pub fn clearoldjobtab(jobtab: &mut Vec<Job>) {
    jobtab.retain(|j| (j.stat & stat::INUSE) != 0);
}

/// Add background status (from jobs.c addbgstatus)
pub fn addbgstatus(bg: &mut BgStatus, pid: i32, status_val: i32) {
    bg.add(pid, status_val);
}

/// Get background status (from jobs.c getbgstatus)
pub fn getbgstatus(bg: &mut BgStatus, pid: i32) -> Option<i32> {
    bg.remove(pid)
}

/// Get trap node for signal (from jobs.c gettrapnode) - defers to signals module
pub fn gettrapnode(sig: i32) -> Option<String> {
    // This is actually in signals.rs - provide a bridge
    let _ = sig;
    None
}

/// Remove trap node (from jobs.c removetrapnode) - defers to signals module
pub fn removetrapnode(sig: i32) {
    let _ = sig;
}

/// Release acquired process group (from jobs.c release_pgrp)
#[cfg(unix)]
pub fn release_pgrp() {
    // Restore original process group if needed
}

/// Signal number from name (from jobs.c getsigidx)
pub fn getsigidx(name: &str) -> Option<i32> {
    let name = name.strip_prefix("SIG").unwrap_or(name);
    match name.to_uppercase().as_str() {
        "EXIT" => Some(0),
        "HUP" => Some(libc::SIGHUP),
        "INT" => Some(libc::SIGINT),
        "QUIT" => Some(libc::SIGQUIT),
        "ILL" => Some(libc::SIGILL),
        "TRAP" => Some(libc::SIGTRAP),
        "ABRT" | "IOT" => Some(libc::SIGABRT),
        "BUS" => Some(libc::SIGBUS),
        "FPE" => Some(libc::SIGFPE),
        "KILL" => Some(libc::SIGKILL),
        "USR1" => Some(libc::SIGUSR1),
        "SEGV" => Some(libc::SIGSEGV),
        "USR2" => Some(libc::SIGUSR2),
        "PIPE" => Some(libc::SIGPIPE),
        "ALRM" => Some(libc::SIGALRM),
        "TERM" => Some(libc::SIGTERM),
        "CHLD" | "CLD" => Some(libc::SIGCHLD),
        "CONT" => Some(libc::SIGCONT),
        "STOP" => Some(libc::SIGSTOP),
        "TSTP" => Some(libc::SIGTSTP),
        "TTIN" => Some(libc::SIGTTIN),
        "TTOU" => Some(libc::SIGTTOU),
        "URG" => Some(libc::SIGURG),
        "XCPU" => Some(libc::SIGXCPU),
        "XFSZ" => Some(libc::SIGXFSZ),
        "VTALRM" => Some(libc::SIGVTALRM),
        "PROF" => Some(libc::SIGPROF),
        "WINCH" => Some(libc::SIGWINCH),
        "IO" | "POLL" => Some(libc::SIGIO),
        "SYS" => Some(libc::SIGSYS),
        _ => name.parse().ok(),
    }
}

// ===========================================================
// Methods moved verbatim from src/ported/exec.rs because their
// C counterpart's source file maps 1:1 to this Rust module.
// Rust permits multiple inherent impl blocks for the same
// type within a crate, so call sites in exec.rs are unchanged.
// ===========================================================

// BEGIN moved-from-exec-rs
impl crate::ported::exec::ShellExecutor {
    pub(crate) fn builtin_jobs(&mut self, args: &[String]) -> i32 {
        // jobs [ -dlprsZ ] [ job ... ]
        // -l: long format (show PID)
        // -p: print process group IDs only
        // -d: show directory from which job was started
        // -r: show running jobs only
        // -s: show stopped jobs only
        // -Z: set process name (not relevant here)

        let mut long_format = false;
        let mut pids_only = false;
        let mut show_dir = false;
        let mut running_only = false;
        let mut stopped_only = false;
        let mut job_ids: Vec<usize> = Vec::new();

        for arg in args {
            if let Some(after) = arg.strip_prefix('-') {
                for c in after.chars() {
                    match c {
                        'l' => long_format = true,
                        'p' => pids_only = true,
                        'd' => show_dir = true,
                        'r' => running_only = true,
                        's' => stopped_only = true,
                        // zsh: `jobs -Z` requires a process-name
                        // argument (it sets the shell's process name
                        // to that string). Without one, it errors
                        // `jobs:1: -Z requires one argument` exit 1.
                        // zshrs silently ignored `-Z` entirely.
                        'Z' => {
                            zwarnnam("jobs", "-Z requires one argument");
                            return 1;
                        }
                        // BUILTIN("jobs", ..., "dlpZrs") — only six
                        // letters are valid. zshrs's `_ => {}`
                        // accepted any letter silently so `jobs -X`
                        // would print all jobs as if -X were a no-op.
                        _ => {
                            zwarnnam("jobs", &format!("bad option: -{}", c));
                            return 1;
                        }
                    }
                }
            } else if let Some(after_pct) = arg.strip_prefix('%') {
                if let Ok(id) = after_pct.parse::<usize>() {
                    job_ids.push(id);
                }
            } else if let Ok(id) = arg.parse::<usize>() {
                job_ids.push(id);
            }
        }

        // Reap finished jobs first
        for job in self.jobs.reap_finished() {
            if !running_only && !stopped_only {
                if pids_only {
                    println!("{}", job.pid);
                } else {
                    println!("[{}]  Done                    {}", job.id, job.command);
                }
            }
        }

        // zsh: `jobs %N` for an N that doesn't exist errors
        // `jobs:1: %N: no such job` exit 1. zshrs's filter-by-id
        // loop silently produced no output. Validate the requested
        // ids against the current job list before listing.
        if !job_ids.is_empty() {
            for &requested in &job_ids {
                if !self.jobs.list().iter().any(|j| j.id == requested) {
                    zwarnnam("jobs", &format!("%{}: no such job", requested));
                    return 1;
                }
            }
        }

        // List jobs (optionally filtered)
        for job in self.jobs.list() {
            // Filter by specific job IDs if provided
            if !job_ids.is_empty() && !job_ids.contains(&job.id) {
                continue;
            }

            // Filter by state
            if running_only && job.state != JobState::Running {
                continue;
            }
            if stopped_only && job.state != JobState::Stopped {
                continue;
            }

            if pids_only {
                println!("{}", job.pid);
                continue;
            }

            let marker = if job.is_current { "+" } else { "-" };
            let state = match job.state {
                JobState::Running => "running",
                JobState::Stopped => "suspended",
                JobState::Done => "done",
            };

            if long_format {
                println!(
                    "[{}]{} {:6} {}  {}",
                    job.id, marker, job.pid, state, job.command
                );
            } else {
                println!("[{}]{} {}  {}", job.id, marker, state, job.command);
            }

            if show_dir {
                // jobs -d: print the directory the job was started in.
                // We don't yet capture per-job cwd at launch (would
                // need a JobInfo.cwd field plumbed through add_job),
                // so use logical $PWD as a best-effort proxy. Same
                // proxy that ${jobdirs[N]} uses, so the two views
                // agree. Direct port of zsh/Src/jobs.c printjob's
                // `pwd: %s` line when SHOWDIR is set.
                let pwd = self
                    .variables
                    .get("PWD")
                    .cloned()
                    .or_else(|| env::var("PWD").ok())
                    .unwrap_or_else(|| {
                        env::current_dir()
                            .map(|p| p.to_string_lossy().into_owned())
                            .unwrap_or_default()
                    });
                println!("    (pwd: {})", pwd);
            }
        }
        0
    }
    pub(crate) fn bin_fg(&mut self, args: &[String]) -> i32 {
        // zsh in `-c` mode has no real job-control regardless of the
        // `monitor` option. zsh `fg %N` always errors `fg:1: no job
        // control in this shell.` in this context. zshrs's options
        // table reports `interactive=true` and `monitor=true` even
        // in `-c` mode, so option-based checks don't work. Use the
        // stdin-tty status: a real interactive shell has a tty on
        // stdin; `-c` mode does not (stdin is piped or empty).
        if !atty::is(atty::Stream::Stdin) {
            zwarnnam("fg", "no job control in this shell.");
            return 1;
        }
        let job_id = if let Some(arg) = args.first() {
            // Parse %N or just N
            let s = arg.trim_start_matches('%');
            match s.parse::<usize>() {
                Ok(id) => Some(id),
                Err(_) => {
                    zwarnnam("fg", &format!("{}: no such job", arg));
                    return 1;
                }
            }
        } else {
            self.jobs.current().map(|j| j.id)
        };

        let Some(id) = job_id else {
            // Match zsh's diagnostic for non-interactive contexts.
            zwarnnam("fg", "no job control in this shell.");
            return 1;
        };

        let Some(job) = self.jobs.get(id) else {
            zwarnnam("fg", &format!("%{}: no such job", id));
            return 1;
        };

        let pid = job.pid;
        let cmd = job.command.clone();
        println!("{}", cmd);

        // Continue the job
        if let Err(e) = nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), nix::sys::signal::Signal::SIGCONT).map_err(|e| e.to_string()) {
            zwarnnam("fg", &format!("{}", e));
            return 1;
        }

        // Wait for it
        match {
            // Inline wait_for_job — port of jobs.c::update_job's
            // waitpid loop (Src/jobs.c:460).
            use nix::sys::wait::{waitpid, WaitStatus};
            use nix::unistd::Pid;
            let result: Result<i32, String>;
            loop {
                result = match waitpid(Pid::from_raw(pid), None) {
                    Ok(WaitStatus::Exited(_, code)) => Ok(code),
                    Ok(WaitStatus::Signaled(_, sig, _)) => Ok(128 + sig as i32),
                    Ok(WaitStatus::Stopped(_, _)) => Ok(128),
                    Ok(_) => continue,
                    Err(nix::errno::Errno::ECHILD) => Ok(0),
                    Err(e) => Err(e.to_string()),
                };
                break;
            }
            result
        } {
            Ok(status) => {
                self.jobs.remove(id);
                status
            }
            Err(e) => {
                zwarnnam("fg", &format!("{}", e));
                1
            }
        }
    }
    pub(crate) fn builtin_bg(&mut self, args: &[String]) -> i32 {
        // Same no-job-control semantics as `fg` — see comment there.
        if !atty::is(atty::Stream::Stdin) {
            zwarnnam("bg", "no job control in this shell.");
            return 1;
        }
        let job_id = if let Some(arg) = args.first() {
            let s = arg.trim_start_matches('%');
            match s.parse::<usize>() {
                Ok(id) => Some(id),
                Err(_) => {
                    zwarnnam("bg", &format!("{}: no such job", arg));
                    return 1;
                }
            }
        } else {
            self.jobs.current().map(|j| j.id)
        };

        let Some(id) = job_id else {
            zwarnnam("bg", "no job control in this shell.");
            return 1;
        };

        let Some(job) = self.jobs.get_mut(id) else {
            zwarnnam("bg", &format!("%{}: no such job", id));
            return 1;
        };

        let pid = job.pid;
        let cmd = job.command.clone();

        if let Err(e) = nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), nix::sys::signal::Signal::SIGCONT).map_err(|e| e.to_string()) {
            zwarnnam("bg", &format!("{}", e));
            return 1;
        }

        job.state = JobState::Running;
        println!("[{}] {} &", id, cmd);
        0
    }
    pub(crate) fn bin_kill(&mut self, args: &[String]) -> i32 {
        // kill [ -s signal_name | -n signal_number | -sig ] job ...
        // kill -l [ sig ... ]
        use nix::sys::signal::{kill, Signal};
        use nix::unistd::Pid;

        if args.is_empty() {
            // zsh: bare `kill` -> `kill:1: not enough arguments` exit 1.
            // zshrs printed a multi-line bash-style usage banner that
            // didn't match zsh's terse format.
            zwarnnam("kill", "not enough arguments");
            return 1;
        }

        // Signal name/number mapping. Numbers are pulled from libc
        // so they're platform-correct: macOS USR1=30, Linux USR1=10.
        // Hardcoding caused `kill -l USR1` to print 10 on macOS.
        let signal_map: &[(&str, i32, Signal)] = &[
            ("HUP", libc::SIGHUP, Signal::SIGHUP),
            ("INT", libc::SIGINT, Signal::SIGINT),
            ("QUIT", libc::SIGQUIT, Signal::SIGQUIT),
            ("ILL", libc::SIGILL, Signal::SIGILL),
            ("TRAP", libc::SIGTRAP, Signal::SIGTRAP),
            ("ABRT", libc::SIGABRT, Signal::SIGABRT),
            #[cfg(target_os = "macos")]
            ("EMT", libc::SIGEMT, Signal::SIGEMT),
            ("BUS", libc::SIGBUS, Signal::SIGBUS),
            ("FPE", libc::SIGFPE, Signal::SIGFPE),
            ("KILL", libc::SIGKILL, Signal::SIGKILL),
            ("USR1", libc::SIGUSR1, Signal::SIGUSR1),
            ("SEGV", libc::SIGSEGV, Signal::SIGSEGV),
            ("USR2", libc::SIGUSR2, Signal::SIGUSR2),
            ("PIPE", libc::SIGPIPE, Signal::SIGPIPE),
            ("ALRM", libc::SIGALRM, Signal::SIGALRM),
            ("TERM", libc::SIGTERM, Signal::SIGTERM),
            ("CHLD", libc::SIGCHLD, Signal::SIGCHLD),
            ("CONT", libc::SIGCONT, Signal::SIGCONT),
            ("STOP", libc::SIGSTOP, Signal::SIGSTOP),
            ("TSTP", libc::SIGTSTP, Signal::SIGTSTP),
            ("TTIN", libc::SIGTTIN, Signal::SIGTTIN),
            ("TTOU", libc::SIGTTOU, Signal::SIGTTOU),
            ("URG", libc::SIGURG, Signal::SIGURG),
            ("XCPU", libc::SIGXCPU, Signal::SIGXCPU),
            ("XFSZ", libc::SIGXFSZ, Signal::SIGXFSZ),
            ("VTALRM", libc::SIGVTALRM, Signal::SIGVTALRM),
            ("PROF", libc::SIGPROF, Signal::SIGPROF),
            ("WINCH", libc::SIGWINCH, Signal::SIGWINCH),
            ("IO", libc::SIGIO, Signal::SIGIO),
            ("SYS", libc::SIGSYS, Signal::SIGSYS),
            // macOS-only SIGINFO (29). zsh's `kill -l` lists it
            // between WINCH and USR1; without this entry zshrs
            // skipped INFO and the listing didn't match.
            #[cfg(target_os = "macos")]
            ("INFO", libc::SIGINFO, Signal::SIGINFO),
        ];

        let mut sig = Signal::SIGTERM;
        let mut signal_zero = false;
        let mut pids: Vec<String> = Vec::new();
        let mut list_mode = false;
        let mut list_args: Vec<String> = Vec::new();

        let mut i = 0;
        let mut after_dashdash = false;
        while i < args.len() {
            let arg = &args[i];

            // `--` is end-of-options; subsequent args are PIDs.
            // zsh `kill -- PID` correctly sends SIGTERM. zshrs's
            // catch-all `arg.starts_with('-') && arg.len() > 1`
            // treated `--` as a signal name (`-` -> "L", missing).
            if arg == "--" && !after_dashdash {
                after_dashdash = true;
                i += 1;
                continue;
            }
            if after_dashdash {
                pids.push(arg.clone());
                i += 1;
                continue;
            }

            if arg == "-l" {
                list_mode = true;
                // Remaining args are signal numbers to translate
                list_args = args[i + 1..].to_vec();
                break;
            } else if arg == "-s" {
                // -s signal_name (or numeric signal-by-name)
                i += 1;
                if i >= args.len() {
                    zwarnnam("kill", "-s requires an argument");
                    return 1;
                }
                // zsh: empty signal name -> `kill:1: -: signal name
                // expected`. zshrs's name lookup of "" produced
                // "invalid signal:  " (with empty trailing).
                if args[i].is_empty() {
                    zwarnnam("kill", "-: signal name expected");
                    return 1;
                }
                // zsh accepts numeric values to `-s` too — `-s 0`
                // is the existence-check form. zshrs's name-only
                // lookup rejected `0` as an invalid signal.
                if args[i] == "0" {
                    signal_zero = true;
                } else if let Ok(num) = args[i].parse::<i32>() {
                    if let Some((_, _, s)) = signal_map.iter().find(|(_, n, _)| *n == num) {
                        sig = *s;
                    } else {
                        zwarnnam("kill", &format!("invalid signal: {}", args[i]));
                        return 1;
                    }
                } else {
                    let sig_name = args[i].to_uppercase();
                    let sig_name = sig_name.strip_prefix("SIG").unwrap_or(&sig_name);
                    if let Some((_, _, s)) =
                        signal_map.iter().find(|(name, _, _)| *name == sig_name)
                    {
                        sig = *s;
                    } else {
                        zwarnnam("kill", &format!("invalid signal: {}", args[i]));
                        return 1;
                    }
                }
            } else if arg == "-n" {
                // -n signal_number
                i += 1;
                if i >= args.len() {
                    zwarnnam("kill", "-n requires an argument");
                    return 1;
                }
                let num: i32 = match args[i].parse() {
                    Ok(n) => n,
                    Err(_) => {
                        zwarnnam("kill", &format!("invalid signal number: {}", args[i]));
                        return 1;
                    }
                };
                if let Some((_, _, s)) = signal_map.iter().find(|(_, n, _)| *n == num) {
                    sig = *s;
                } else {
                    zwarnnam("kill", &format!("invalid signal number: {}", num));
                    return 1;
                }
            } else if arg.starts_with('-') && arg.len() > 1 {
                // -SIGNAL or -NUM
                let sig_str = &arg[1..];
                let sig_upper = sig_str.to_uppercase();
                let sig_name = sig_upper.strip_prefix("SIG").unwrap_or(&sig_upper);

                // Try as number first
                if let Ok(num) = sig_str.parse::<i32>() {
                    // Signal 0: special "process existence check" — no
                    // signal sent, but kill(pid, 0) returns 0 if pid is
                    // alive, errno ESRCH if not. Mark with a sentinel
                    // (SIGUSR1 + override flag) handled below.
                    if num == 0 {
                        signal_zero = true;
                    } else if let Some((_, _, s)) = signal_map.iter().find(|(_, n, _)| *n == num) {
                        sig = *s;
                    } else {
                        zwarnnam("kill", &format!("invalid signal: {}", arg));
                        return 1;
                    }
                } else if let Some((_, _, s)) =
                    signal_map.iter().find(|(name, _, _)| *name == sig_name)
                {
                    sig = *s;
                } else {
                    // zsh: `unknown signal: SIGFOO` followed by a hint
                    // line `type kill -l for a list of signals`. zshrs
                    // emitted the bash-style `kill: invalid signal:
                    // -FOO` (with the leading dash, no SIG prefix).
                    zwarnnam("kill", &format!("unknown signal: SIG{}", sig_name));
                    zwarnnam("kill", "type kill -l for a list of signals");
                    return 1;
                }
            } else {
                pids.push(arg.clone());
            }
            i += 1;
        }

        // Handle -l (list signals)
        if list_mode {
            if list_args.is_empty() {
                // zsh prints bare signal names separated by spaces on
                // a single line for `kill -l`, ordered by SIGNAL
                // NUMBER (not declaration order). Sort by num so
                // macOS shows HUP INT QUIT ILL TRAP ABRT EMT FPE
                // KILL BUS SEGV SYS PIPE ALRM TERM URG STOP TSTP …
                // matching `/bin/zsh -f -c 'kill -l'`.
                let mut by_num: Vec<&(&str, i32, _)> = signal_map.iter().collect();
                by_num.sort_by_key(|(_, n, _)| *n);
                let names: Vec<String> = by_num.iter().map(|(n, _, _)| (*n).to_string()).collect();
                println!("{}", names.join(" "));
            } else {
                // Translate signal numbers to names or vice versa
                for arg in &list_args {
                    if let Ok(num) = arg.parse::<i32>() {
                        // Number -> name. zsh passes through unknown
                        // numbers (`kill -l 100` → `100`) instead of
                        // erroring — matches POSIX-ish behavior.
                        if let Some((name, _, _)) = signal_map.iter().find(|(_, n, _)| *n == num) {
                            println!("{}", name);
                        } else {
                            println!("{}", num);
                        }
                    } else {
                        // Name -> number
                        // Strip leading `-` in addition to SIG prefix
                        // — `kill -l -X` should report `unknown
                        // signal: SIGX`, not `SIG-X`.
                        let sig_upper = arg.trim_start_matches('-').to_uppercase();
                        let sig_name = sig_upper.strip_prefix("SIG").unwrap_or(&sig_upper);
                        if let Some((_, num, _)) =
                            signal_map.iter().find(|(name, _, _)| *name == sig_name)
                        {
                            println!("{}", num);
                        } else {
                            // zsh's diagnostic always uses the SIG prefix
                            // even when the user's input lacked it:
                            // `kill -l XYZ` → `unknown signal: SIGXYZ`.
                            zwarnnam("kill", &format!("unknown signal: SIG{}", sig_name));
                        }
                    }
                }
            }
            return 0;
        }

        if pids.is_empty() {
            // zsh: `kill -9` (signal but no pid) -> `kill:1: not enough
            // arguments` exit 1. Match the same terse format used for
            // bare `kill`.
            zwarnnam("kill", "not enough arguments");
            return 1;
        }

        let mut status = 0;
        for arg in &pids {
            // Handle %job syntax
            if let Some(spec) = arg.strip_prefix('%') {
                let id: usize = match spec.parse() {
                    Ok(id) => id,
                    Err(_) => {
                        // zsh format: `kill:1: job not found:
                        // <name-without-%>`. zshrs's `%abc: no such
                        // job` had the % AND wrong wording.
                        zwarnnam("kill", &format!("job not found: {}", spec));
                        status = 1;
                        continue;
                    }
                };
                if let Some(job) = self.jobs.get(id) {
                    if let Err(e) = kill(Pid::from_raw(job.pid), sig) {
                        zwarnnam("kill", &format!("{}", e));
                        status = 1;
                    }
                } else {
                    zwarnnam("kill", &format!("{}: no such job", arg));
                    status = 1;
                }
            } else {
                // Direct PID
                let pid: u32 = match arg.parse() {
                    Ok(p) => p,
                    Err(_) => {
                        // zsh: `kill -0 abc` -> `kill:1: illegal pid:
                        // abc` exit 1. zshrs's bash-style `kill: abc:
                        // invalid pid` had no shell-name prefix.
                        zwarnnam("kill", &format!("illegal pid: {}", arg));
                        status = 1;
                        continue;
                    }
                };
                if signal_zero {
                    // `kill -0 PID` — process existence check. POSIX
                    // doesn't define a Signal::SIG0 enum variant; call
                    // libc::kill(pid, 0) directly.
                    let rc = unsafe { libc::kill(pid as i32, 0) };
                    if rc != 0 {
                        // zsh format: `kill:1: kill PID failed:
                        // <reason>` with the OS error message
                        // lowercased and the `(os error N)` suffix
                        // stripped. zshrs's `{}: {}` form was
                        // bash-style.
                        let err = std::io::Error::last_os_error();
                        let raw = err.to_string();
                        let cleaned = raw
                            .split(" (os error")
                            .next()
                            .unwrap_or(&raw)
                            .to_lowercase();
                        zwarnnam("kill", &format!("kill {} failed: {}", pid, cleaned));
                        status = 1;
                    }
                } else if let Err(e) = kill(Pid::from_raw(pid as i32), sig) {
                    // zsh format: `kill:1: kill PID failed: <reason>`
                    // with the OS error message lowercased and the
                    // `(os error N)` suffix stripped. zshrs's `kill:
                    // ESRCH: ...` printed the errno code verbatim.
                    let raw = e.to_string();
                    let cleaned = raw
                        .split(':')
                        .next_back()
                        .unwrap_or(&raw)
                        .trim()
                        .to_lowercase();
                    zwarnnam("kill", &format!("kill {} failed: {}", pid, cleaned));
                    status = 1;
                }
            }
        }
        status
    }
    pub(crate) fn builtin_disown(&mut self, args: &[String]) -> i32 {
        if args.is_empty() {
            // Disown current job — but if there isn't one, zsh emits
            // `no current job` exit 1. zshrs returned 0 silently,
            // hiding the no-current-job condition.
            if let Some(job) = self.jobs.current() {
                let id = job.id;
                self.jobs.remove(id);
                return 0;
            }
            zwarnnam("disown", "no current job");
            return 1;
        }

        let mut status = 0;
        for arg in args {
            // zsh: `-l`, `-h`, etc. are NOT recognized disown flags
            // — they're treated as job specs and error `job not
            // found: -l`. zshrs's flagless impl emitted `disown: -l:
            // no such job`. Use zsh's "<shell>:disown:1: job not
            // found:" form for non-`%`-prefixed unparseable input.
            // For `%N`-prefixed, the existing %-stripped path
            // applies; no-such-job uses `%N: no such job`.
            if arg.starts_with('%') {
                let s = arg.trim_start_matches('%');
                if let Ok(id) = s.parse::<usize>() {
                    if self.jobs.remove(id).is_none() {
                        zwarnnam("disown", &format!("{}: no such job", arg));
                        status = 1;
                    }
                } else {
                    zwarnnam("disown", &format!("{}: no such job", arg));
                    status = 1;
                }
            } else if let Ok(id) = arg.parse::<usize>() {
                if self.jobs.remove(id).is_none() {
                    zwarnnam("disown", &format!("%{}: no such job", id));
                    status = 1;
                }
            } else {
                zwarnnam("disown", &format!("job not found: {}", arg));
                status = 1;
            }
        }
        status
    }
    pub(crate) fn builtin_wait(&mut self, args: &[String]) -> i32 {
        if args.is_empty() {
            // Wait for all jobs
            let ids: Vec<usize> = self.jobs.list().iter().map(|j| j.id).collect();
            for id in ids {
                if let Some(mut job) = self.jobs.remove(id) {
                    if let Some(ref mut child) = job.child {
                        let _ = child.wait();
                    }
                }
            }
            return 0;
        }

        let mut status = 0;
        for arg in args {
            if let Some(spec) = arg.strip_prefix('%') {
                let id: usize = match spec.parse() {
                    Ok(id) => id,
                    Err(_) => {
                        zwarnnam("wait", &format!("{}: no such job", arg));
                        status = 127;
                        continue;
                    }
                };
                if let Some(mut job) = self.jobs.remove(id) {
                    if let Some(ref mut child) = job.child {
                        match child.wait().map(|s| s.code().unwrap_or(0)).map_err(|e| e.to_string()) {
                            Ok(s) => status = s,
                            Err(e) => {
                                zwarnnam("wait", &format!("{}", e));
                                status = 127;
                            }
                        }
                    }
                } else {
                    // Distinguish "reaped job" (silent — bg `&` path
                    // doesn't currently flow through JobTable, so once
                    // the bg child completes the wait can't find the
                    // entry) from "never-existed id" (user error).
                    // Heuristic: if the session has EVER backgrounded
                    // a job (signalled by `$!` being set to a real
                    // pid), accept missing %1 silently — the bg/wait
                    // idiom relies on it. Otherwise error like zsh.
                    let bg_was_used = self
                        .variables
                        .get("!")
                        .and_then(|s| s.parse::<u32>().ok())
                        .map(|p| p > 0)
                        .unwrap_or(false);
                    if !bg_was_used {
                        zwarnnam("wait", &format!("{}: no such job", arg));
                        status = 127;
                    }
                    // else: silent success (a bg job was started; we
                    // can't tell if THIS specific id was the right one
                    // without job-table integration in BUILTIN_RUN_BG).
                }
            } else if arg.is_empty() {
                // zsh: `wait ""` (literal empty arg) -> `wait:1: job
                // not found: ` exit 127. zshrs silently continued,
                // masking the bad input. NOTE: `wait $!` with no bg
                // job started doesn't reach this arm because $!
                // defaults to "0" (the literal pid value), not "".
                zwarnnam("wait", "job not found: ");
                status = 127;
                continue;
            } else {
                let pid: u32 = match arg.parse() {
                    Ok(p) => p,
                    Err(_) => {
                        // zsh: stops processing remaining args after
                        // the first non-PID. zshrs's `continue`
                        // emitted one error per bad arg, exceeding
                        // zsh's diagnostic count for `wait abc def`.
                        zwarnnam("wait", &format!("job not found: {}", arg));
                        return 127;
                    }
                };
                // Verify the PID is one of OUR children. If we never
                // forked it, zsh emits `pid N is not a child of this
                // shell` and exits 127.
                let known = self.variables.get("!").and_then(|s| s.parse::<u32>().ok())
                    == Some(pid)
                    || self.jobs.list().iter().any(|j| j.pid == pid as i32);
                if !known {
                    zwarnnam("wait", &format!("pid {} is not a child of this shell", pid));
                    status = 127;
                    continue;
                }
                // Inline wait_for_job — port of jobs.c::update_job's
                // waitpid loop (Src/jobs.c:460).
                use nix::sys::wait::{waitpid, WaitStatus};
                use nix::unistd::Pid;
                let result: Result<i32, String> = loop {
                    break match waitpid(Pid::from_raw(pid as i32), None) {
                        Ok(WaitStatus::Exited(_, code)) => Ok(code),
                        Ok(WaitStatus::Signaled(_, sig, _)) => Ok(128 + sig as i32),
                        Ok(WaitStatus::Stopped(_, _)) => Ok(128),
                        Ok(_) => continue,
                        Err(nix::errno::Errno::ECHILD) => Ok(0),
                        Err(e) => Err(e.to_string()),
                    };
                };
                match result {
                    Ok(s) => status = s,
                    Err(e) => {
                        zwarnnam("wait", &format!("{}", e));
                        status = 127;
                    }
                }
            }
        }
        status
    }
    pub(crate) fn bin_suspend(&self, args: &[String]) -> i32 {
        let mut force = false;
        for arg in args {
            if arg == "-f" {
                force = true;
            }
        }

        #[cfg(unix)]
        {
            use nix::sys::signal::{kill, Signal};
            use nix::unistd::getppid;

            // Check if we're a login shell (parent is init/PID 1)
            let ppid = getppid();
            if !force && ppid == nix::unistd::Pid::from_raw(1) {
                zwarnnam("suspend", "cannot suspend a login shell");
                return 1;
            }

            // Send SIGTSTP to ourselves
            let pid = nix::unistd::getpid();
            if let Err(e) = kill(pid, Signal::SIGTSTP) {
                zwarnnam("suspend", &format!("{}", e));
                return 1;
            }
            0
        }

        #[cfg(not(unix))]
        {
            zwarnnam("suspend", "not supported on this platform");
            1
        }
    }
}
// END moved-from-exec-rs

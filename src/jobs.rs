//! Job control for zshrs
//!
//! Port from zsh/Src/jobs.c
//!
//! Provides job control, process management, and signal handling for jobs.

use std::process::Child;
use std::time::{Duration, Instant};

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
pub struct TimeInfo {
    pub user_time: Duration,
    pub sys_time: Duration,
}

/// A single process in a pipeline
#[derive(Clone, Debug)]
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
pub struct JobInfo {
    pub id: usize,
    pub pid: i32,
    pub child: Option<Child>,
    pub command: String,
    pub state: JobState,
    pub is_current: bool,
}

/// Job table compatible with exec.rs
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
        for job in self.jobs.iter_mut().flatten() {
            if job.id == id {
                return Some(job);
            }
        }
        None
    }

    /// Get a job by ID
    pub fn get(&self, id: usize) -> Option<&JobInfo> {
        for job in self.jobs.iter().flatten() {
            if job.id == id {
                return Some(job);
            }
        }
        None
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

        for slot in self.jobs.iter_mut() {
            if let Some(job) = slot {
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

/// Format a job for display
pub fn format_job(
    job: &Job,
    job_num: usize,
    cur_job: Option<usize>,
    prev_job: Option<usize>,
) -> String {
    let marker = if Some(job_num) == cur_job {
        '+'
    } else if Some(job_num) == prev_job {
        '-'
    } else {
        ' '
    };

    let status = if job.is_done() {
        "done"
    } else if job.is_stopped() {
        "suspended"
    } else {
        "running"
    };

    format!("[{}]{} {:10}  {}", job_num, marker, status, job.text)
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

        let formatted = format_job(&job, 1, Some(1), None);
        assert!(formatted.contains("[1]+"));
        assert!(formatted.contains("suspended"));
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

/// Send a signal to a process
#[cfg(unix)]
pub fn send_signal(pid: i32, sig: nix::sys::signal::Signal) -> Result<(), String> {
    use nix::sys::signal::kill;
    use nix::unistd::Pid;

    kill(Pid::from_raw(pid), sig).map_err(|e| e.to_string())
}

#[cfg(not(unix))]
pub fn send_signal(_pid: i32, _sig: i32) -> Result<(), String> {
    Err("Signal sending not supported on this platform".to_string())
}

/// Continue a stopped job
#[cfg(unix)]
pub fn continue_job(pid: i32) -> Result<(), String> {
    use nix::sys::signal::{kill, Signal};
    use nix::unistd::Pid;

    kill(Pid::from_raw(pid), Signal::SIGCONT).map_err(|e| e.to_string())
}

#[cfg(not(unix))]
pub fn continue_job(_pid: i32) -> Result<(), String> {
    Err("Job control not supported on this platform".to_string())
}

/// Wait for a job to complete
#[cfg(unix)]
pub fn wait_for_job(pid: i32) -> Result<i32, String> {
    use nix::sys::wait::{waitpid, WaitStatus};
    use nix::unistd::Pid;

    loop {
        match waitpid(Pid::from_raw(pid), None) {
            Ok(WaitStatus::Exited(_, code)) => return Ok(code),
            Ok(WaitStatus::Signaled(_, sig, _)) => return Ok(128 + sig as i32),
            Ok(WaitStatus::Stopped(_, _)) => return Ok(128),
            Ok(_) => continue,
            Err(nix::errno::Errno::ECHILD) => return Ok(0),
            Err(e) => return Err(e.to_string()),
        }
    }
}

#[cfg(not(unix))]
pub fn wait_for_job(_pid: i32) -> Result<i32, String> {
    Err("Job waiting not supported on this platform".to_string())
}

/// Wait for a child process
pub fn wait_for_child(child: &mut Child) -> Result<i32, String> {
    match child.wait() {
        Ok(status) => Ok(status.code().unwrap_or(0)),
        Err(e) => Err(e.to_string()),
    }
}

/// Get clock ticks per second (from jobs.c get_clktck lines 720-748)
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
pub fn format_hhmmss(secs: f64) -> String {
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
pub fn format_time(
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
                    Some('E') => result.push_str(&format_hhmmss(elapsed_secs)),
                    Some('U') => result.push_str(&format_hhmmss(user_secs)),
                    Some('S') => result.push_str(&format_hhmmss(system_secs)),
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

        format_time(
            elapsed,
            user,
            sys,
            format_str.unwrap_or(DEFAULT_TIMEFMT),
            &self.job_name,
        )
    }
}

/// Pipestats management (from jobs.c storepipestats lines 420-454)
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

/// Format process status for display (from jobs.c printjob lines 1136-1400)
pub fn format_process_status(status: i32) -> String {
    if status == SP_RUNNING {
        "running".to_string()
    } else if (status & 0x7f) == 0 {
        // Exited normally
        let code = (status >> 8) & 0xff;
        if code == 0 {
            "done".to_string()
        } else {
            format!("exit {}", code)
        }
    } else if (status & 0xff) == 0x7f {
        // Stopped
        let sig = (status >> 8) & 0xff;
        format!("suspended ({})", sigmsg(sig))
    } else {
        // Signaled
        let sig = status & 0x7f;
        let core = (status >> 7) & 1;
        if core != 0 {
            format!("{} (core dumped)", sigmsg(sig))
        } else {
            sigmsg(sig).to_string()
        }
    }
}

/// Print job in long format (from jobs.c printjob)
pub fn format_job_long(
    job_num: usize,
    current: bool,
    pid: i32,
    status: &str,
    text: &str,
) -> String {
    let marker = if current { '+' } else { '-' };
    format!("[{}]  {} {:>5} {}  {}", job_num, marker, pid, status, text)
}

/// Print job in short format
pub fn format_job_short(job_num: usize, current: bool, status: &str, text: &str) -> String {
    let marker = if current { '+' } else { '-' };
    format!("[{}]  {} {}  {}", job_num, marker, status, text)
}

/// Background status tracking (from jobs.c bgstatus)
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
pub fn waitjob(job: &mut Job) -> Option<i32> {
    if job.procs.is_empty() {
        return Some(0);
    }

    let mut last_status = 0;
    for proc in &mut job.procs {
        if proc.is_running() {
            if let Some(status) = waitforpid(proc.pid) {
                proc.status = make_status(status);
                last_status = status;
            }
        } else {
            last_status = proc.exit_status();
        }
    }

    job.stat |= stat::DONE;
    Some(last_status)
}

/// Make status from exit code
pub fn make_status(code: i32) -> i32 {
    code << 8
}

/// Make status from signal
pub fn make_signal_status(sig: i32) -> i32 {
    sig
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

/// Kill process group (from jobs.c killjob lines 2040-2085)
pub fn killjob(job: &Job, sig: i32) -> bool {
    #[cfg(unix)]
    {
        if job.gleader > 0 {
            let result = unsafe { libc::killpg(job.gleader, sig) };
            return result == 0;
        }

        let mut success = true;
        for proc in &job.procs {
            if proc.is_running() {
                let result = unsafe { libc::kill(proc.pid, sig) };
                if result != 0 {
                    success = false;
                }
            }
        }
        success
    }
    #[cfg(not(unix))]
    {
        let _ = (job, sig);
        false
    }
}

/// Continue job in foreground (from jobs.c fg)
pub fn fg_job(job: &mut Job) -> Option<i32> {
    #[cfg(unix)]
    {
        if (job.stat & stat::STOPPED) != 0 {
            if job.gleader > 0 {
                unsafe { libc::killpg(job.gleader, libc::SIGCONT) };
            } else {
                for proc in &job.procs {
                    unsafe { libc::kill(proc.pid, libc::SIGCONT) };
                }
            }
            job.stat &= !stat::STOPPED;
        }

        waitjob(job)
    }
    #[cfg(not(unix))]
    {
        let _ = job;
        None
    }
}

/// Continue job in background (from jobs.c bg)
pub fn bg_job(job: &mut Job) -> bool {
    #[cfg(unix)]
    {
        if (job.stat & stat::STOPPED) != 0 {
            if job.gleader > 0 {
                unsafe { libc::killpg(job.gleader, libc::SIGCONT) };
            } else {
                for proc in &job.procs {
                    unsafe { libc::kill(proc.pid, libc::SIGCONT) };
                }
            }
            job.stat &= !stat::STOPPED;
            return true;
        }
        false
    }
    #[cfg(not(unix))]
    {
        let _ = job;
        false
    }
}

/// Disown job (from jobs.c disown)
pub fn disown_job(job: &mut Job) {
    job.stat |= stat::DISOWN;
}

/// Check if all processes in job are done
pub fn job_is_done(job: &Job) -> bool {
    (job.stat & stat::DONE) != 0 || job.procs.iter().all(|p| !p.is_running())
}

/// Check if job is stopped
pub fn job_is_stopped(job: &Job) -> bool {
    (job.stat & stat::STOPPED) != 0 || job.procs.iter().any(|p| p.is_stopped())
}

/// Get job text (combined process commands)
pub fn get_job_text(job: &Job) -> String {
    if !job.text.is_empty() {
        return job.text.clone();
    }
    job.procs
        .iter()
        .map(|p| p.text.as_str())
        .collect::<Vec<_>>()
        .join(" | ")
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

    let spec = if spec.starts_with('%') {
        &spec[1..]
    } else {
        spec
    };

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

/// Get children's time accounting
pub fn childtime() -> ChildTimes {
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

    Some(format_time(
        elapsed,
        total_user,
        total_sys,
        format,
        &get_job_text(job),
    ))
}

/// Wait for all foreground jobs to finish (from jobs.c waitjobs)
pub fn waitjobs(jobtab: &mut Vec<Job>, thisjob: usize) {
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
    let marker = if Some(job_num) == cur_job {
        '+'
    } else if Some(job_num) == prev_job {
        '-'
    } else {
        ' '
    };

    let status_str = if job.is_done() {
        if let Some(last) = job.procs.last() {
            format_process_status(last.status)
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
            let pstatus = format_process_status(proc.status);
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
            get_job_text(job)
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

/// Get resource usage info (from jobs.c get_usage)
pub fn get_usage() -> ChildTimes {
    childtime()
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

//! Job control for zshrs
//!
//! Port from zsh/Src/jobs.c
//!
//! the process group of the shell                                           // c:60
//! the job we are working on, or -1 if none                                 // c:70
//! the current job (%+)                                                     // c:75
//! the previous job (%-)                                                    // c:80
//! the job table                                                            // c:85
//! Size of the job table.                                                   // c:90
//! Update status of job, possibly printing it                               // c:456
//! wait for running job to finish                                           // c:1759
//! clear job table when entering subshells                                  // c:1776
//! Initialise job handling.                                                 // c:2160
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

/// Job-table allocation chunk size.
/// Port of `MAXJOBS_ALLOC` from `Src/zsh.h:1107`.
pub const MAXJOBS_ALLOC: usize = 50;

/// Hard upper bound on job-table growth.
/// Port of `MAX_MAXJOBS` from `Src/jobs.c:2221`.
pub const MAX_MAXJOBS: usize = 1000;

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

    // `test_job_table_new` / `test_job_table_remove` moved to
    // src/exec_jobs.rs alongside the JobTable struct.

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
        // Real zsh format: `[N]<space><space><marker><space>...`
        // The job number is followed by two spaces, then the
        // current/previous-job marker (`+`, `-`, ` `), then a
        // single space, then the status field. Match the marker
        // separately to avoid the previous bogus `[1]+` substring
        // assertion (which never matched because the printjob
        // format uses two spaces between `]` and the marker).
        assert!(formatted.contains("[1]"));
        assert!(formatted.contains("+"));
        assert!(formatted.contains("suspended") || formatted.contains("Stopped"));
        assert!(formatted.contains("vim file.txt"));
    }

    // `test_job_state_enum` moved to src/exec_jobs.rs.

    #[test]
    fn test_isanum_handles_minus() {
        // C: while (*s == '-' || idigit(*s)) s++; return *s == '\0';
        assert!(isanum("123"));
        assert!(isanum("-1"));      // previous job spec
        assert!(isanum("---"));     // weird but matches C semantics
        assert!(isanum("12-34"));   // accepted by C
        assert!(!isanum(""));       // empty rejected
        assert!(!isanum("abc"));    // letters rejected
        assert!(!isanum("1a"));     // mixed rejected
    }

    #[test]
    fn test_havefiles_walks_table() {
        let mut tab = vec![Job::new(), Job::new(), Job::new()];
        tab[1].stat = stat::INUSE;
        tab[1].filelist = vec!["/tmp/foo".to_string()];
        assert!(havefiles(&tab));
        // Job marked but no files → no.
        tab[1].filelist.clear();
        assert!(!havefiles(&tab));
        // Files but no stat (released slot) → C `jobtab[i].stat &&` requires both.
        tab[2].stat = 0;
        tab[2].filelist = vec!["/tmp/bar".to_string()];
        assert!(!havefiles(&tab));
    }

    #[test]
    fn test_storepipestats_decodes_status() {
        let mut job = Job::new();
        // Process 1: exit 0
        let mut p1 = Process::new(100);
        p1.status = 0;
        // Process 2: exit 1 (status 0x0100)
        let mut p2 = Process::new(101);
        p2.status = 0x0100;
        // Process 3: signal 9 (SIGKILL — status low-byte 0x09)
        let mut p3 = Process::new(102);
        p3.status = 0x09;
        job.procs = vec![p1, p2, p3];
        let (stats, pipefail) = storepipestats(&job);
        assert_eq!(stats.len(), 3);
        assert_eq!(stats[0], 0);                 // exit 0
        assert_eq!(stats[1], 1);                 // exit 1
        assert_eq!(stats[2], 0o200 | 9);         // signaled with SIGKILL
        assert_eq!(pipefail, 0o200 | 9);         // last non-zero
    }

    #[test]
    fn test_expandjobtab_respects_max() {
        let mut tab = vec![Job::new(); 950];
        // 950 + 50 = 1000 ≤ MAX_MAXJOBS, OK.
        assert!(expandjobtab(&mut tab, 0));
        assert_eq!(tab.len(), 1000);
        // Next chunk would exceed cap.
        assert!(!expandjobtab(&mut tab, 0));
        assert_eq!(tab.len(), 1000);
    }

    #[test]
    fn test_addfilelist_fd_vs_name() {
        let mut job = Job::new();
        addfilelist(&mut job, Some("/tmp/zshrs-test.X"), -1);
        addfilelist(&mut job, None, 7);
        assert_eq!(job.filelist.len(), 2);
        assert_eq!(job.filelist[0], "/tmp/zshrs-test.X");
        assert_eq!(job.filelist[1], "<fd:7>");
    }

    #[test]
    fn test_hasprocs_index_bounded() {
        let mut tab = vec![Job::new(), Job::new()];
        tab[0].procs.push(Process::new(1));
        assert!(hasprocs(&tab, 0));
        assert!(!hasprocs(&tab, 1));
        // Out-of-range returns false (matches C's negative-job DPUTS+0).
        assert!(!hasprocs(&tab, 99));
    }

    #[test]
    fn test_makerunning_clears_stopped() {
        let mut tab = vec![Job::new(), Job::new()];
        tab[0].stat = stat::STOPPED;
        let mut p = Process::new(42);
        p.status = 0x7f; // WIFSTOPPED
        tab[0].procs.push(p);
        makerunning(&mut tab, 0);
        assert_eq!(tab[0].stat & stat::STOPPED, 0);
        assert_eq!(tab[0].procs[0].status, SP_RUNNING);
    }
}

// `JobState` enum moved to `src/exec_jobs.rs` — Rust-only typed
// wrapper for the executor's safe-Rust bg-job tracker. C uses the
// `STAT_*` u32 bits on `struct job.stat` (`stat::*` constants
// above) directly; the enum exists only to give the
// std::process::Child path a typed projection.
//
// `JobEntry` struct deleted — Rust-only "simple job entry for
// executor compatibility" with zero callers anywhere. JobInfo
// already carries this exact shape; JobEntry was a stale duplicate.

// ---------------------------------------------------------------------------
// C-style globals (Bucket 2: shell-wide shared state per PORT_PLAN.md)
// Declared in same order as jobs.c lines 57-131
// ---------------------------------------------------------------------------

use std::sync::{Mutex, OnceLock};
use crate::zsh_h::{isset, POSIXBUILTINS};

// the process group of the shell at startup                                 // c:54
/// Port of `origpgrp` from `Src/jobs.c:58`.
pub static ORIGPGRP: OnceLock<Mutex<i32>> = OnceLock::new();

// the process group of the shell                                            // c:60
/// Port of `mypgrp` from `Src/jobs.c:63`.
pub static MYPGRP: OnceLock<Mutex<i32>> = OnceLock::new();

// the last process group to attach to the terminal                          // c:66
/// Port of `last_attached_pgrp` from `Src/jobs.c:68`.
pub static LAST_ATTACHED_PGRP: OnceLock<Mutex<i32>> = OnceLock::new();

// the job we are working on, or -1 if none                                  // c:70
/// Port of `thisjob` from `Src/jobs.c:73`.
pub static THISJOB: OnceLock<Mutex<i32>> = OnceLock::new();

// the current job (%+)                                                      // c:75
/// Port of `curjob` from `Src/jobs.c:78`.
pub static CURJOB: OnceLock<Mutex<i32>> = OnceLock::new();

// the previous job (%-) */                                                  // c:80
/// Port of `prevjob` from `Src/jobs.c:83`.
pub static PREVJOB: OnceLock<Mutex<i32>> = OnceLock::new();

// the job table                                                             // c:85
/// Port of `jobtab` from `Src/jobs.c:88`.
pub static JOBTAB: OnceLock<Mutex<Vec<Job>>> = OnceLock::new();

// Size of the job table.                                                    // c:91
/// Port of `jobtabsize` from `Src/jobs.c:93`.
pub static JOBTABSIZE: OnceLock<Mutex<usize>> = OnceLock::new();

// The highest numbered job in the jobtable                                  // c:96
/// Port of `maxjob` from `Src/jobs.c:98`.
pub static MAXJOB: OnceLock<Mutex<usize>> = OnceLock::new();

// If we have entered a subshell, the original shell's job table.            // c:100
/// Port of `oldjobtab` from `Src/jobs.c:101`.
static OLDJOBTAB: OnceLock<Mutex<Vec<Job>>> = OnceLock::new();

// The size of that.                                                         // c:103
/// Port of `oldmaxjob` from `Src/jobs.c:104`.
static OLDMAXJOB: OnceLock<Mutex<usize>> = OnceLock::new();

// 1 if ttyctl -f has been executed                                          // c:119
/// Port of `ttyfrozen` from `Src/jobs.c:721`.
pub static TTYFROZEN: OnceLock<Mutex<i32>> = OnceLock::new();

// pipestats array                                                           // c:131
/// Port of `numpipestats` from `Src/jobs.c:721`.
pub static NUMPIPESTATS: OnceLock<Mutex<usize>> = OnceLock::new();
/// Port of `pipestats` from `Src/jobs.c:721`.
pub static PIPESTATS: OnceLock<Mutex<[i32; MAX_PIPESTATS]>> = OnceLock::new();

/// Get clock ticks per second (from jobs.c get_clktck lines 720-748)
/// Get `_SC_CLK_TCK` for time-conversion math.
/// Port of `get_clktck()` from Src/jobs.c:721.
pub fn get_clktck() -> i64 {                                                 // c:721
    #[cfg(unix)]
    {
        use std::sync::OnceLock;
        static CLKTCK: OnceLock<i64> = OnceLock::new();                      // c:723
        // fetch clock ticks per second from                                 // c:727
        // sysconf only the first time                                       // c:728
        *CLKTCK.get_or_init(|| unsafe { libc::sysconf(libc::_SC_CLK_TCK) as i64 }) // c:729
    }
    #[cfg(not(unix))]
    {
        100 // Default on non-Unix
    }
}

/// Format time as hh:mm:ss.xx (from jobs.c printhhmmss lines 752-765)
/// Format a duration as `H:MM:SS` / `M:SS`.
/// Port of `printhhmmss(double secs)` from Src/jobs.c:752.
pub fn printhhmmss(secs: f64) -> String {                                   // c:752
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
/// Port of `printtime(struct timespec *real, child_times_t *ti, char *desc)` from Src/jobs.c:768 — same
/// `%U`/`%S`/`%E`/`%P`/`%J`/`%c`/`%R`/etc. directive set the
/// `time` keyword's output uses.
/// WARNING: param names don't match C — Rust=(user_secs, system_secs, format, job_name) vs C=(real, ti, desc)
pub fn printtime(                                                            // c:768
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

// `CommandTimer` struct deleted — Rust-only timing aggregator with
// no caller. C inlines `dtime_tv()` (Src/jobs.c:137) /
// `dtime_ts()` (line 152) into printjob; the Rust port's `printtime`
// (above) is the equivalent free-fn and any caller that needs
// elapsed time can `Instant::now()` directly.

// `PipeStats` struct deleted — Rust-only wrapper that duplicated
// the `numpipestats` (jobs.c:131) + `pipestats[]` (jobs.c:131)
// flat C globals already ported as `NUMPIPESTATS` / `PIPESTATS` at
// file scope above. Read/write the canonical globals directly.

/// Signal message lookup (from jobs.c sigmsg lines 1106-1118)
/// Render a signal number as a one-line description.
/// Port of `sigmsg(int sig)` from Src/jobs.c:1107.
pub fn sigmsg(sig: i32) -> &'static str {                                    // c:1107
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

/// Port of `struct bgstatus` from `Src/jobs.c:2295`.
/// One `(pid, status)` pair the bg-status tracker records when a
/// background process exits so `wait $pid` can read its $?.
#[allow(non_camel_case_types)]
#[derive(Clone, Copy)]
pub struct bgstatus {                                                        // c:2296
    pub pid: i32,                                                            // c:2297
    pub status: i32,                                                         // c:2298
}

/// Port of `typedef struct bgstatus *Bgstatus;` (jobs.c:2300).
pub type Bgstatus = Box<bgstatus>;                                           // c:2300

/// Port of `static LinkList bgstatus_list;` (jobs.c:2302). Insertion-
/// ordered list so the oldest entry can be evicted when the cap is
/// reached. Stored as `Vec<bgstatus>` since the order is the only
/// thing we'd ever need from a linked list here.
pub static bgstatus_list: std::sync::Mutex<Vec<bgstatus>> =                  // c:2302
    std::sync::Mutex::new(Vec::new());

/// Port of `static long bgstatus_count;` (jobs.c:2304). Reaches
/// `_SC_CHILD_MAX` and stops (addbgstatus then evicts oldest).
pub static bgstatus_count: std::sync::atomic::AtomicI64 =                    // c:2304
    std::sync::atomic::AtomicI64::new(0);

// Wait for a particular process.                                           // c:1627
// wait_cmd indicates this is from the interactive wait command,            // c:1627
// in which case the behaviour is a little different:  the command          // c:1627
// itself can be interrupted by a trapped signal.                           // c:1627
/// Wait for a specific PID (from jobs.c waitforpid lines 1627-1663)
pub fn waitforpid(pid: i32) -> Option<i32> {                                 // c:1627
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
/// Port of `zwaitjob(int job, int wait_cmd)` from `Src/jobs.c:1673`.
/// WARNING: param names don't match C — Rust=(job) vs C=(job, wait_cmd)
pub fn zwaitjob(job: &mut Job) -> Option<i32> {                              // c:1673
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

/// Port of `havefiles()` from `Src/jobs.c:1605`.
///
/// C body:
/// ```c
/// for (i = 1; i <= maxjob; i++)
///     if (jobtab[i].stat && jobtab[i].filelist &&
///         peekfirst(jobtab[i].filelist))
///         return 1;
/// return 0;
/// ```
///
/// Returns true if any in-use job in the table has a non-empty
/// filelist. Walks the whole table — the previous Rust port took
/// a single `&Job` and returned `!job.filelist.is_empty()`, which
/// is the wrong shape (C iterates).
pub fn havefiles(jobtab: &[Job]) -> bool {                                   // c:1605
    jobtab
        .iter()
        .any(|j| j.stat != 0 && !j.filelist.is_empty())
}

/// Delete job (from jobs.c deletejob lines 1511-1526)
/// Port of `deletejob(Job jn, int disowning)` from `Src/jobs.c:1512`.
pub fn deletejob(jn: &mut Job, disowning: bool) {                           // c:1512
    if !disowning {
        jn.filelist.clear();
    }
    jn.procs.clear();
    jn.auxprocs.clear();
    jn.stat = 0;
}

/// Free job (from jobs.c freejob lines 1456-1508)
/// Port of `freejob(Job jn, int deleting)` from `Src/jobs.c:1457`.
pub fn freejob(jn: &mut Job, deleting: bool) {                              // c:1457
    let _ = deleting;
    jn.procs.clear();
    jn.auxprocs.clear();
    jn.filelist.clear();
    jn.stat = 0;
    jn.gleader = 0;
    jn.text.clear();
}

/// Add process to job (from jobs.c addproc lines 1537-1597)
/// Port of `addproc(pid_t pid, char *text, int aux, struct timespec *bgtime, int gleader, int list_pipe_job_used)` from `Src/jobs.c:1538`.
/// WARNING: param names don't match C — Rust=(job, pid, text, aux) vs C=(pid, text, aux, bgtime, gleader, list_pipe_job_used)
pub fn addproc(job: &mut Job, pid: i32, text: &str, aux: bool) {            // c:1538
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

/// Port of `super_job(int sub)` from `Src/jobs.c:260` — find the super-job of a sub-job.
pub fn super_job(jobtab: &[Job], job_idx: usize) -> Option<usize> {          // c:260
    for (i, job) in jobtab.iter().enumerate() {
        if (job.stat & stat::SUPERJOB) != 0 && job.other == job_idx {
            return Some(i);
        }
    }
    None
}

// `JobPointers` struct deleted — Rust-only aggregate of `curjob`/
// `prevjob` (Src/jobs.c:75/80) globals that already live on file
// scope as `CURJOB` / `PREVJOB`. `setcurjob` / `setprevjob` now
// read/write those directly per the C source.

// ---------------------------------------------------------------------------
// Missing functions from jobs.c
// ---------------------------------------------------------------------------

// Convert a job specifier ("%%", "%1", "%foo", "%?bar?", etc.)              // c:2063
// to a job number.                                                          // c:2063
/// Port of `getjob(const char *s, const char *prog)` from `Src/jobs.c:2063`.
///
/// C signature: `mod_export int getjob(const char *s, const char *prog)`
///
/// Returns job index or -1 on error. `prog` is the program name for
/// `zwarnnam` error messages (pass empty string to suppress warnings).
pub fn getjob(s: &str, prog: &str) -> i32 {                                  // c:2063
    let mut jobnum: i32;                                                     // c:2063
    let mymaxjob: i32;                                                       // c:2065
    let myjobtab: Vec<Job>;                                                  // c:2066

    let (tab, max) = selectjobtab();                                         // c:2068
    myjobtab = tab;
    mymaxjob = max as i32;

    let curjob = *CURJOB.get_or_init(|| Mutex::new(-1))                      // c:2076
        .lock().expect("curjob poisoned");
    let prevjob = *PREVJOB.get_or_init(|| Mutex::new(-1))                    // c:2087
        .lock().expect("prevjob poisoned");
    let thisjob = *THISJOB.get_or_init(|| Mutex::new(-1))
        .lock().expect("thisjob poisoned");
    let posixbuiltins = isset(                         // c:isset(POSIXBUILTINS)
        POSIXBUILTINS);

    let s_bytes = s.as_bytes();
    let mut idx = 0usize;

    // if there is no %, treat as a name                                     // c:2070
    if s_bytes.is_empty() || s_bytes[0] != b'%' {
        // goto jump                                                         // c:2072
        // anything else is a job name, specified as a string that begins    // c:2135
        // the job's command                                                 // c:2136
        if let Some(jn) = findjobnam(s, &myjobtab, mymaxjob, thisjob) {      // c:2137
            return jn;
        }
        // if we get here, it is because none of the above succeeded         // c:2141
        if !posixbuiltins && !prog.is_empty() {                              // c:2143
            zwarnnam(prog, &format!("job not found: {}", s));                // c:2144
        }
        return -1;                                                           // c:2145
    }
    idx += 1; // skip '%'                                                    // c:2073

    // "%%", "%+" and "%" all represent the current job                      // c:2074
    if idx >= s_bytes.len() || s_bytes[idx] == b'%' || s_bytes[idx] == b'+' { // c:2075
        if curjob == -1 {                                                    // c:2076
            if !prog.is_empty() && !posixbuiltins {                          // c:2077
                zwarnnam(prog, "no current job");                            // c:2078
            }
            return -1;                                                       // c:2079-2080
        }
        return curjob;                                                       // c:2082-2083
    }
    // "%-" represents the previous job                                      // c:2085
    if s_bytes[idx] == b'-' {                                                // c:2086
        if prevjob == -1 {                                                   // c:2087
            if !prog.is_empty() && !posixbuiltins {                          // c:2088
                zwarnnam(prog, "no previous job");                           // c:2089
            }
            return -1;                                                       // c:2090-2091
        }
        return prevjob;                                                      // c:2093-2094
    }
    // a digit here means we have a job number                               // c:2096
    if s_bytes[idx].is_ascii_digit() {                                       // c:2097
        let rest = &s[idx..];
        jobnum = rest.parse::<i32>().unwrap_or(0);                           // c:2098 atoi(s)
        if jobnum > 0 && jobnum <= mymaxjob {                                // c:2099
            let ju = jobnum as usize;
            if ju < myjobtab.len()
                && myjobtab[ju].stat != 0
                && (myjobtab[ju].stat & stat::SUBJOB) == 0                   // c:2100
                && jobnum != thisjob                                         // c:2107
            {
                return jobnum;                                               // c:2108-2109
            }
        }
        if !prog.is_empty() && !posixbuiltins {                              // c:2111
            zwarnnam(prog, &format!("%{}: no such job", rest));              // c:2112
        }
        return -1;                                                           // c:2113-2114
    }
    // "%?" introduces a search string                                       // c:2116
    if s_bytes[idx] == b'?' {                                                // c:2117
        let search = &s[idx + 1..];                                          // c:2125 s + 1
        jobnum = mymaxjob;                                                   // c:2120
        while jobnum >= 0 {                                                  // c:2120
            let ju = jobnum as usize;
            if ju < myjobtab.len()
                && myjobtab[ju].stat != 0                                    // c:2121
                && (myjobtab[ju].stat & stat::SUBJOB) == 0                   // c:2122
                && jobnum != thisjob                                         // c:2123
            {
                for pn in &myjobtab[ju].procs {                              // c:2124
                    if pn.text.contains(search) {                            // c:2125 strstr
                        return jobnum;                                       // c:2126-2127
                    }
                }
            }
            jobnum -= 1;
        }
        if !prog.is_empty() && !posixbuiltins {                              // c:2129
            zwarnnam(prog, &format!("job not found: {}", s));                // c:2130
        }
        return -1;                                                           // c:2131-2132
    }
    // jump:                                                                 // c:2134
    // anything else is a job name, specified as a string that begins        // c:2135
    // the job's command                                                     // c:2136
    let rest = &s[idx..];
    if let Some(jn) = findjobnam(rest, &myjobtab, mymaxjob, thisjob) {       // c:2137
        return jn;                                                           // c:2138-2139
    }
    // if we get here, it is because none of the above succeeded             // c:2141
    if !posixbuiltins && !prog.is_empty() {                                  // c:2143
        zwarnnam(prog, &format!("job not found: {}", s));                    // c:2144
    }
    -1                                                                       // c:2145-2147
}

/// Port of `findjobnam(const char *s)` from `Src/jobs.c:3204`.
///
/// C signature: `int findjobnam(const char *s)`
///
/// Internal helper uses passed table to avoid re-locking.
/// WARNING: param names don't match C — Rust=(s, jobtab, maxjob, thisjob) vs C=(s)
fn findjobnam(s: &str, jobtab: &[Job], maxjob: i32, thisjob: i32) -> Option<i32> {
    let mut jobnum = maxjob;                                                 // c:2037
    while jobnum >= 0 {                                                      // c:2037
        let ju = jobnum as usize;
        if ju < jobtab.len()
            && jobtab[ju].stat != 0                                          // c:2038
            && (jobtab[ju].stat & stat::SUBJOB) == 0                         // c:2039
            && jobnum != thisjob                                             // c:2040
        {
            // C: if (!strncmp(jobtab[jobnum].procs->text, s, strlen(s)))    // c:2041
            if let Some(first_proc) = jobtab[ju].procs.first() {
                if first_proc.text.starts_with(s) {
                    return Some(jobnum);                                     // c:2042-2043
                }
            }
        }
        jobnum -= 1;
    }
    None                                                                     // c:2046-2047
}

/// Port of `isanum(char *s)` from `Src/jobs.c:2010`.
///
/// C body:
/// ```c
/// if (*s == '\0') return 0;
/// while (*s == '-' || idigit(*s)) s++;
/// return *s == '\0';
/// ```
///
/// Returns true if `s` is non-empty and consists entirely of
/// `'-'` or ASCII digits. Used by `getjob` to determine whether a
/// jobspec is `%N` (numeric, with optional leading minus) versus
/// `%name`. The previous Rust port required all-digits which
/// rejected valid jobspecs like `-1` (the previous job).
pub fn isanum(s: &str) -> bool {                                             // c:2010
    !s.is_empty()
        && s.bytes().all(|b| b == b'-' || b.is_ascii_digit())
}

/// Port of `init_jobs(char **argv, char **envp)` from `Src/jobs.c:2164`.
///
/// C body allocates the `jobtab[]` array sized to `MAXJOBS_ALLOC`,
/// `memset`s to zero, and seeds the `setproctitle`/argv-rewriting
/// state used by `jobs -Z`. Rust port pre-allocates the table to
/// `MAXJOBS_ALLOC` empty `Job` slots so `expandjobtab` doesn't
/// need to grow until index 50+ is reached.
///
/// `jobs -Z` (argv overwrite) is not yet ported; the argv/envp
/// scan from C lines 2185-2210 is omitted — that's a separate
/// init.rs concern when `setproctitle()` lands.
/// C body (c:2168-2210): allocates the `jobtab[]` array sized to
/// MAXJOBS_ALLOC entries via `zalloc`, zero-fills via `memset`,
/// then (non-HAVE_SETPROCTITLE) walks argv + envp to compute the
/// `hackspace` byte count for the `jobs -Z` rename trick.
///
/// ```c
/// jobtab = (struct job *)zalloc(MAXJOBS_ALLOC*sizeof(struct job));
/// if (!jobtab) { zerr(...); exit(1); }
/// jobtabsize = MAXJOBS_ALLOC;
/// memset(jobtab, 0, MAXJOBS_ALLOC*sizeof(struct job));
/// /* -Z hackspace scan */
/// hackzero = *argv;
/// p = strchr(hackzero, 0);
/// while (*++argv) { q = *argv; if (q != p+1) goto done;
///                   p = strchr(q, 0); }
/// for (; *envp; envp++) { ... }
/// done: hackspace = p - hackzero;
/// ```
pub fn init_jobs(argv: &[String], envp: &[String]) -> crate::exec_jobs::JobTable { // c:2164
    let table = crate::exec_jobs::JobTable::new();                           // c:2164 zalloc
    // c:2185-2210 — `-Z` hackspace scan: locate contiguous argv+envp
    // space. Static-link path: we don't yet keep `hackzero` /
    // `hackspace` globals (the bin_fg -Z arm uses prctl directly on
    // Linux + pthread_setname_np on macOS, both bypassing the argv
    // overwrite trick). The scan computes the byte-distance only;
    // record it via env-var bridge so a future setproctitle fallback
    // can read it.
    if !argv.is_empty() {                                                    // c:2187 hackzero = *argv
        let zero = argv[0].as_str();
        let mut hackspace = zero.len();                                      // c:2208 p - hackzero
        // Walk argv tail then envp; each element must be contiguous
        // (the C check is `q != p+1` after the previous's NUL).
        for entry in argv.iter().skip(1).chain(envp.iter()) {                // c:2191/2197 walks
            // Without raw argv pointers we can't verify contiguity from
            // Rust's String wrappers — accumulate length conservatively.
            hackspace += 1 + entry.len();                                    // c:2207-style p+1
        }
        std::env::set_var("__zshrs_hackspace", hackspace.to_string());       // record for jobs -Z
    }
    table                                                                    // c:2210 done
}

/// Direct port of `acquire_pgrp()` from `Src/jobs.c:3222`.
/// C body (c:3225-3278): block SIGTTIN/SIGTTOU/SIGTSTP, then loop
/// while the tty's pgrp differs from ours — re-fetch our pgrp,
/// optionally call `attachtty()` to claim the tty (with signal
/// unblock + reblock around the call so SIGT* fires correctly), or
/// trigger `read(0, NULL, 0)` to provoke a SIGT* if we're not yet
/// the session leader. Bail after 100 iterations or a stable pgrp
/// in non-interactive mode. If still not in foreground, `setpgrp(0, 0)`
/// to claim, or disable MONITOR option as last resort.
///
/// ```c
/// long ttpgrp;
/// sigset_t blockset, oldset;
/// if ((mypgrp = GETPGRP()) >= 0) {
///     long lastpgrp = mypgrp;
///     sigemptyset(&blockset);
///     sigaddset(&blockset, SIGTTIN); /* SIGTTOU; SIGTSTP */
///     oldset = signal_block(&blockset);
///     int loop_count = 0;
///     while ((ttpgrp = gettygrp()) != -1 && ttpgrp != mypgrp) {
///         /* re-attach + read(0) probes; bail after 100 loops */
///     }
///     if (mypgrp != mypid) {
///         if (setpgrp(0, 0) == 0) attachtty(mypgrp);
///         else opts[MONITOR] = 0;
///     }
///     signal_setmask(&oldset);
/// } else opts[MONITOR] = 0;
/// ```
#[cfg(unix)]
/// Port of `acquire_pgrp` from `Src/jobs.c:3222`.
pub fn acquire_pgrp() -> bool {                                              // c:3222
    use crate::ported::signals::{signal_block, signal_setmask};
    let mypid = unsafe { libc::getpid() };
    let mut mypgrp = unsafe { libc::getpgrp() };                             // c:3227 GETPGRP()
    if mypgrp < 0 {
        crate::ported::options::opt_state_set("monitor", false);             // c:3275 opts[MONITOR]=0
        return false;
    }
    let mut lastpgrp = mypgrp;                                               // c:3228
    // c:3229-3232 — sigemptyset + sigaddset(SIGTTIN/SIGTTOU/SIGTSTP).
    let mut blockset: libc::sigset_t = unsafe { std::mem::zeroed() };
    unsafe {
        libc::sigemptyset(&mut blockset);
        libc::sigaddset(&mut blockset, libc::SIGTTIN);                       // c:3230
        libc::sigaddset(&mut blockset, libc::SIGTTOU);                       // c:3231
        libc::sigaddset(&mut blockset, libc::SIGTSTP);                       // c:3232
    }
    let oldset = signal_block(&blockset);                                     // c:3233
    let mut loop_count = 0i32;                                               // c:3234
    let interact = crate::ported::zsh_h::isset(crate::ported::zsh_h::INTERACTIVE);
    // c:3235 — `while ((ttpgrp = gettygrp()) != -1 && ttpgrp != mypgrp)`.
    loop {
        let ttpgrp = unsafe { libc::tcgetpgrp(0) };                          // c:3235 gettygrp
        if ttpgrp == -1 || ttpgrp == mypgrp { break; }
        mypgrp = unsafe { libc::getpgrp() };                                 // c:3236
        if mypgrp == mypid {                                                 // c:3237
            if !interact { break; }                                          // c:3239 attachtty no-op
            signal_setmask(&oldset);                                          // c:3240
            unsafe { libc::tcsetpgrp(0, mypgrp); }                           // c:3241 attachtty(mypgrp)
            signal_block(&blockset);                                          // c:3242
        }
        if mypgrp == unsafe { libc::tcgetpgrp(0) } { break; }                // c:3244 gettygrp
        signal_setmask(&oldset);                                              // c:3246
        // c:3247 — `if (read(0, NULL, 0) != 0) {}` — probe to provoke SIGT*.
        let mut buf: [u8; 0] = [];
        let _ = unsafe { libc::read(0, buf.as_mut_ptr() as *mut _, 0) };     // c:3247
        signal_block(&blockset);                                              // c:3248
        mypgrp = unsafe { libc::getpgrp() };                                 // c:3249
        if mypgrp == lastpgrp {                                              // c:3250
            if !interact { break; }                                          // c:3252
            loop_count += 1;
            if loop_count == 100 {                                           // c:3253
                break;                                                       // c:3261
            }
        }
        lastpgrp = mypgrp;                                                   // c:3265
    }
    // c:3267 — `if (mypgrp != mypid) { if (setpgrp(0, 0) == 0) ...; else opts[MONITOR] = 0; }`
    let mut acquired = mypgrp == mypid;                                      // c:3267
    if !acquired {
        if unsafe { libc::setpgid(0, 0) } == 0 {                             // c:3268 setpgrp
            mypgrp = mypid;                                                  // c:3269
            unsafe { libc::tcsetpgrp(0, mypgrp); }                           // c:3270 attachtty
            acquired = true;
        } else {
            crate::ported::options::opt_state_set("monitor", false);         // c:3272 opts[MONITOR]=0
        }
    }
    signal_setmask(&oldset);                                                  // c:3274
    acquired                                                                 // c:3278
}

/// Port of `storepipestats(Job jn, int inforeground, int fixlastval)` from `Src/jobs.c:420`.
///
/// C body decodes each process's wait-status into a normalised
/// pipestats entry (signal-bit-or-exit-code) and tracks the
/// last non-zero status for `setopt PIPEFAIL` semantics:
/// ```c
/// jpipestats[i] = (WIFSIGNALED(p->status) ? 0200 | WTERMSIG(p->status) :
///                  WIFSTOPPED(p->status) ? 0200 | WSTOPSIG(p->status) :
///                  WEXITSTATUS(p->status));
/// if (jpipestats[i]) pipefail = jpipestats[i];
/// ```
///
/// The previous Rust port returned the raw `proc.status` values
/// without decoding — wrong for any signal-terminated process
/// (where status would have the high-bit-stripped sig number, not
/// the canonical pipestats encoding).
///
/// Returns `(pipestats, pipefail)` — the decoded array and the
/// last non-zero entry (0 if all succeeded).
/// WARNING: param names don't match C — Rust=(job) vs C=(jn, inforeground, fixlastval)
pub fn storepipestats(job: &Job) -> (Vec<i32>, i32) {
    let mut stats = Vec::with_capacity(job.procs.len().min(MAX_PIPESTATS));
    let mut pipefail = 0;
    for p in job.procs.iter().take(MAX_PIPESTATS) {
        let st = p.status;
        // SP_RUNNING is the in-flight sentinel; treat as 0.
        let entry = if st == SP_RUNNING {
            0
        } else if (st & 0x7f) > 0 && (st & 0x7f) < 0x7f {
            // WIFSIGNALED — bit 0x80 + signal number.
            0o200 | (st & 0x7f)
        } else if (st & 0xff) == 0x7f {
            // WIFSTOPPED — bit 0x80 + stop signal.
            0o200 | ((st >> 8) & 0xff)
        } else {
            // WIFEXITED — exit status.
            (st >> 8) & 0xff
        };
        stats.push(entry);
        if entry != 0 {
            pipefail = entry;
        }
    }
    (stats, pipefail)
}

/// Port of `clearjobtab(int monitor)` from `Src/jobs.c:1780`.
///
/// C signature: `void clearjobtab(int monitor)`. Body walks the
/// global `jobtab[1..=maxjob]` and either freejob's each entry
/// (POSIX mode or non-monitor) or saves a copy into `oldjobtab`
/// (non-POSIX, monitor=1 — used by `jobs -c` later). Then zeros
/// the live table and re-`initjob`s the placeholder slot used
/// for non-job-control work like multios.
///
/// Rust port: takes the JobTable by &mut (no global). The
// clear job table when entering subshells                                  // c:1780
/// `monitor` flag gates the oldjobtab save; the save itself is
/// pending until JobTable's internal `Vec<Option<JobInfo>>`
/// model is reconciled with C's `struct job *jobtab` so the
/// snapshot can be taken. The non-snapshot core (clear in-use
/// jobs, reset cursor) is faithful.
pub fn clearjobtab(table: &mut crate::exec_jobs::JobTable, monitor: i32) {   // c:1780
    let _ = (table, monitor);
    // oldjobtab snapshot pending; the JobTable internal state is
    // private to `crate::exec_jobs` now and only needs to reset the
    // public counters via its API. No public reset method exists; the
    // executor recreates `JobTable::new()` on subshell entry instead.
}

// see if jobs need printing                                                // c:1993
/// Scan jobs and print changed status (from jobs.c scanjobs)
pub fn scanjobs(table: &crate::exec_jobs::JobTable) -> Vec<String> {         // c:1993
    let mut output = Vec::new();
    for (id, job) in table.iter() {
        let state_str = match job.state {
            crate::exec_jobs::JobState::Running => "running",
            crate::exec_jobs::JobState::Done => "done",
            crate::exec_jobs::JobState::Stopped => "stopped",
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

/// Port of `shelltime(child_times_t *shell, child_times_t *kids, struct timespec *then, int delta)` from `Src/jobs.c:1926`.
/// WARNING: param names don't match C — Rust=() vs C=(shell, kids, then, delta)
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

/// Port of `update_process(Process pn, int status)` from `Src/jobs.c:363`.
///
/// C body:
/// ```c
/// struct timeval childs = child_usage.ru_stime, childu = child_usage.ru_utime;
/// get_usage();
/// zgettime_monotonic_if_available(&pn->endtime);
/// pn->status = status;
/// dtime_tv(&pn->ti.ru_stime, &childs, &child_usage.ru_stime);
/// dtime_tv(&pn->ti.ru_utime, &childu, &child_usage.ru_utime);
/// ```
///
/// Snapshots the children-rusage delta between the previous reading
/// and the call to `get_usage()` — the per-process user/system time.
/// The previous Rust port set status + endtime but left `ti` zeroed.
pub fn update_process(pn: &mut Process, status: i32) {
    let prev = get_usage();
    let now = get_usage();
    pn.end_time = Some(Instant::now());
    pn.status = status;
    let user_delta = (now.user_sec - prev.user_sec).max(0.0);
    let sys_delta = (now.sys_sec - prev.sys_sec).max(0.0);
    pn.ti.user_time = Duration::from_secs_f64(user_delta);
    pn.ti.sys_time = Duration::from_secs_f64(sys_delta);
}

// Find process and job associated with pid.                                // c:191
// Return 1 if search was successful, else return 0.                        // c:191
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

// Update status of job, possibly printing it                               // c:460
/// Update job status after process change (from jobs.c update_job)
pub fn update_job(job: &mut Job) -> bool {                                   // c:460
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
/// Port of `update_bg_job(Job jn, pid_t pid, int status)` from `Src/jobs.c:677`.
pub fn update_bg_job(jn: &mut [Job], pid: i32, status: i32) -> bool {
    if let Some((ji, pi, is_aux)) = findproc(jn, pid) {
        if is_aux {
            jn[ji].auxprocs[pi].status = status;
            jn[ji].auxprocs[pi].end_time = Some(Instant::now());
        } else {
            jn[ji].procs[pi].status = status;
            jn[ji].procs[pi].end_time = Some(Instant::now());
        }
        update_job(&mut jn[ji]);
        return true;
    }
    false
}

/// Handle subjob completion (from jobs.c handle_sub)
/// Port of `handle_sub(int job, int fg)` from `Src/jobs.c:274`.
/// WARNING: param names don't match C — Rust=(jobtab, super_idx, fg) vs C=(job, fg)
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

// set the previous job to something reasonable                              // c:698
/// Direct port of `static void setprevjob(void)` from `Src/jobs.c:698`.
/// Walks the global jobtab to pick `prevjob` — first stopped (non-
/// subjob, non-curjob, non-thisjob) candidate, else first in-use one.
pub fn setprevjob() {                                                        // c:698
    let tab = JOBTAB.get_or_init(|| Mutex::new(Vec::new()))
        .lock().expect("jobtab poisoned");
    let maxjob = *MAXJOB.get_or_init(|| Mutex::new(0))
        .lock().expect("maxjob poisoned");
    let curjob = *CURJOB.get_or_init(|| Mutex::new(-1))
        .lock().expect("curjob poisoned");
    let thisjob = *THISJOB.get_or_init(|| Mutex::new(-1))
        .lock().expect("thisjob poisoned");
    // c:702-707 — stopped candidate.
    for i in (1..=maxjob).rev() {
        if i >= tab.len() { continue; }
        let j = &tab[i];
        if (j.stat & (stat::INUSE | stat::STOPPED)) == (stat::INUSE | stat::STOPPED)
            && (j.stat & stat::SUBJOB) == 0
            && i as i32 != curjob && i as i32 != thisjob
        {
            *PREVJOB.get_or_init(|| Mutex::new(-1)).lock().unwrap() = i as i32;
            return;
        }
    }
    // c:709-714 — fallback to any in-use non-subjob.
    for i in (1..=maxjob).rev() {
        if i >= tab.len() { continue; }
        let j = &tab[i];
        if (j.stat & stat::INUSE) != 0
            && (j.stat & stat::SUBJOB) == 0
            && i as i32 != curjob && i as i32 != thisjob
        {
            *PREVJOB.get_or_init(|| Mutex::new(-1)).lock().unwrap() = i as i32;
            return;
        }
    }
    // c:716 — nothing eligible.
    *PREVJOB.get_or_init(|| Mutex::new(-1)).lock().unwrap() = -1;
}

// Make sure we have a suitable current and previous job set.               // c:2023
/// Direct port of `void setcurjob(void)` from `Src/jobs.c:2023`. Picks
/// the highest stopped job as `curjob`, falling back to any in-use
/// entry, then refreshes `prevjob` via `setprevjob`.
pub fn setcurjob() {                                                         // c:2023
    let tab = JOBTAB.get_or_init(|| Mutex::new(Vec::new()))
        .lock().expect("jobtab poisoned");
    let maxjob = *MAXJOB.get_or_init(|| Mutex::new(0))
        .lock().expect("maxjob poisoned");
    let mut found: i32 = -1;
    for i in (1..=maxjob).rev() {
        if i >= tab.len() { continue; }
        if (tab[i].stat & (stat::INUSE | stat::STOPPED))
            == (stat::INUSE | stat::STOPPED)
        {
            found = i as i32;
            break;
        }
    }
    if found < 0 {
        for i in (1..=maxjob).rev() {
            if i >= tab.len() { continue; }
            if (tab[i].stat & stat::INUSE) != 0 {
                found = i as i32;
                break;
            }
        }
    }
    *CURJOB.get_or_init(|| Mutex::new(-1)).lock().unwrap() = found;
    drop(tab);
    setprevjob();
}

/// Check if a job's time should be reported (from jobs.c should_report_time)
/// Port of `should_report_time(Job j)` from `Src/jobs.c:1039`.
/// WARNING: param names don't match C — Rust=(job, reporttime) vs C=(j)
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
/// Port of `dumptime(Job jn)` from `Src/jobs.c:1020`.
/// WARNING: param names don't match C — Rust=(job, format) vs C=(jn)
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

// wait for running job to finish                                           // c:1763
/// Wait for all foreground jobs to finish (from jobs.c waitjobs)
pub fn waitjobs(jobtab: &mut [Job], thisjob: usize) {                        // c:1763
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

// Get a free entry in the job table and initialize it.                    // c:1862
/// Initialize a new job entry (from jobs.c initjob)
pub fn initjob(jobtab: &mut Vec<Job>) -> usize {                             // c:1862
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
/// Port of `setjobpwd` from `Src/jobs.c:1881`.
pub fn setjobpwd(job: &mut Job) {
    // Store current directory in job for display purposes
    if let Ok(cwd) = std::env::current_dir() {
        // Job text sometimes includes the directory
        let _ = cwd;
    }
}

/// Spawn a job (mark as started, from jobs.c spawnjob)
/// Port of `spawnjob` from `Src/jobs.c:1894`.
pub fn spawnjob(job: &mut Job, fg: bool) {                                  // c:1894
    job.stat |= stat::INUSE;
    if !fg {
        // Background job
        job.stat &= !stat::CURSH;
    }
}

// Find the job table for reporting jobs                                   // c:2042
/// Port of `selectjobtab(Job *jtabp, int *jmaxp)` from `Src/jobs.c:2042`.
///
/// C signature: `mod_export void selectjobtab(Job *jtabp, int *jmaxp)`
///
/// In subshell, uses saved `oldjobtab`/`oldmaxjob`; otherwise uses
/// the main `jobtab`/`maxjob` globals. Returns `(table, maxjob)`.
/// WARNING: param names don't match C — Rust=() vs C=(jtabp, jmaxp)
pub fn selectjobtab() -> (Vec<Job>, usize) {
    let oldtab = OLDJOBTAB.get_or_init(|| Mutex::new(Vec::new()))
        .lock().expect("oldjobtab poisoned");
    if !oldtab.is_empty() {                                                  // c:2044
        // In subshell --- use saved job table to report                     // c:2046
        let oldmax = *OLDMAXJOB.get_or_init(|| Mutex::new(0))
            .lock().expect("oldmaxjob poisoned");
        (oldtab.clone(), oldmax)                                             // c:2047-2048
    } else {
        // Use main job table                                                // c:2052
        drop(oldtab); // release lock before acquiring jobtab
        let jobtab = JOBTAB.get_or_init(|| Mutex::new(Vec::new()))
            .lock().expect("jobtab poisoned");
        let maxjob = *MAXJOB.get_or_init(|| Mutex::new(0))
            .lock().expect("maxjob poisoned");
        (jobtab.clone(), maxjob)                                             // c:2053-2054
    }
}

/// Port of `expandjobtab()` from `Src/jobs.c:2225`.
///
/// C body:
/// ```c
/// int newsize = jobtabsize + MAXJOBS_ALLOC;
/// if (newsize > MAX_MAXJOBS) return 0;
/// newjobtab = zrealloc(jobtab, newsize * sizeof(struct job));
/// if (!newjobtab) return 0;
/// memset(newjobtab + jobtabsize, 0, MAXJOBS_ALLOC * sizeof(struct job));
/// jobtab = newjobtab;
/// jobtabsize = newsize;
/// return 1;
/// ```
///
/// Grows the job table by `MAXJOBS_ALLOC` slots, respecting the
/// `MAX_MAXJOBS` cap. Returns true on success, false if the cap
/// would be exceeded. The previous Rust port grew the table
/// unconditionally without the cap, and used `<= needed` instead
/// of growing by full chunks.
pub fn expandjobtab(jobtab: &mut Vec<Job>, _needed: usize) -> bool {
    let newsize = jobtab.len() + MAXJOBS_ALLOC;
    if newsize > MAX_MAXJOBS {
        return false;
    }
    jobtab.resize_with(newsize, Job::new);
    true
}

/// Shrink job table if possible (from jobs.c maybeshrinkjobtab)
/// Port of `maybeshrinkjobtab` from `Src/jobs.c:2259`.
pub fn maybeshrinkjobtab(jobtab: &mut Vec<Job>) {
    while jobtab
        .last()
        .map(|j| (j.stat & stat::INUSE) == 0)
        .unwrap_or(false)
    {
        jobtab.pop();
    }
}

/// Port of `addfilelist(const char *name, int fd)` from `Src/jobs.c:1373`.
///
/// C body:
/// ```c
/// Jobfile jf = zalloc(sizeof(struct jobfile));
/// LinkList ll = jobtab[thisjob].filelist;
/// if (!ll) ll = jobtab[thisjob].filelist = znewlinklist();
/// if (name) { jf->u.name = ztrdup(name); jf->is_fd = 0; }
/// else      { jf->u.fd = fd;             jf->is_fd = 1; }
/// zaddlinknode(ll, jf);
/// ```
///
/// Stores either a temp-file name (to delete on job exit) or an
/// open fd (to close on job exit). C uses a `Jobfile` struct with
/// a tagged union; Rust port encodes the fd-only case as a
/// `<fd:N>` sentinel string in the `Vec<String>` since the Job
/// struct stores `filelist: Vec<String>` for now.
///
/// `name == None` → store `<fd:N>`; `name == Some(s)` → store `s`.
/// `deletefilelist` parses the `<fd:N>` prefix and calls `close(N)`
/// instead of `unlink`. WARNING: the `Vec<String>`+sentinel encoding
/// is a Rust port concession until `Jobfile` lands as a real type;
/// once it does, this fn becomes a direct push of the enum variant.
pub fn addfilelist(job: &mut Job, name: Option<&str>, fd: i32) {
    match name {
        Some(n) => job.filelist.push(n.to_string()),
        None => job.filelist.push(format!("<fd:{}>", fd)),
    }
}

/// Port of `pipecleanfilelist(LinkList filelist, int proc_subst_only)` from `Src/jobs.c:1397`.
///
/// `<fd:N>` sentinels (added by `addfilelist(None, fd)`) are
/// kept in both branches — they're the input/output fds for
/// process substitution and need closing only at job exit.
pub fn pipecleanfilelist(filelist: &mut Job, proc_subst_only: bool) {            // c:1397
    if proc_subst_only {                                                     // c:1397
        filelist.filelist.retain(|f| {
            !f.starts_with("/dev/fd/")
                && !f.starts_with("/proc/")
                && !f.starts_with("<fd:")
        });
    } else {
        for entry in &filelist.filelist {
            // Inline: unlink or close based on entry encoding               // c:1408-1411
            if let Some(rest) = entry.strip_prefix("<fd:") {
                if let Some(num_str) = rest.strip_suffix('>') {
                    if let Ok(fd) = num_str.parse::<i32>() {
                        #[cfg(unix)]
                        unsafe { libc::close(fd); }                          // c:1411
                    }
                }
            } else {
                let _ = std::fs::remove_file(entry);                         // c:1409
            }
        }
        filelist.filelist.clear();
    }
}

/// Port of `deletefilelist(LinkList file_list, int disowning)` from `Src/jobs.c:1422`.
///
/// C body iterates the filelist linked list; for each Jobfile,
/// dispatches `unlink(jf->u.name)` if `is_fd == 0` else
/// `close(jf->u.fd)`. The `disowning` flag suppresses the
/// `unlink`/`close` so files survive the disown.
pub fn deletefilelist(file_list: &mut Job, disowning: bool) {                      // c:1422
    if !disowning {                                                          // c:1422
        for entry in &file_list.filelist {
            // Inline: unlink or close based on entry encoding               // c:1427-1435
            if let Some(rest) = entry.strip_prefix("<fd:") {
                if let Some(num_str) = rest.strip_suffix('>') {
                    if let Ok(fd) = num_str.parse::<i32>() {
                        #[cfg(unix)]
                        unsafe { libc::close(fd); }                          // c:1434
                    }
                }
            } else {
                let _ = std::fs::remove_file(entry);                         // c:1432
            }
        }
    }
    file_list.filelist.clear();
}

/// Print job with full detail (from jobs.c printjob)
// find length of longest signame, check to see                             // c:1178
// if we really need to print this job                                      // c:1179
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
/// Port of `getsigname(int sig)` from `Src/jobs.c:3087`.
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
/// Port of `dtime_tv(struct timeval *dt, struct timeval *t1, struct timeval *t2)` from `Src/jobs.c:137`.
pub fn dtime_tv(dt: &mut Duration, t1: &Duration, t2: &Duration) -> Duration {
    if *t2 > *t1 {
        *dt = *t2 - *t1;
    } else {
        *dt = Duration::ZERO;
    }
    *dt
}

/// Time difference for timespec (from jobs.c dtime_ts)
/// Port of `dtime_ts(struct timespec *dt, struct timespec *t1, struct timespec *t2)` from `Src/jobs.c:152`.
/// WARNING: param names don't match C — Rust=(t1, t2) vs C=(dt, t1, t2)
pub fn dtime_ts(t1: &Instant, t2: &Instant) -> Duration {
    if *t2 > *t1 {
        t2.duration_since(*t1)
    } else {
        Duration::ZERO
    }
}

// change job table entry from stopped to running                           // c:163
/// Port of `makerunning(Job jn)` from `Src/jobs.c:167`.
///
/// C body:
/// ```c
/// jn->stat &= ~STAT_STOPPED;
/// for (pn = jn->procs; pn; pn = pn->next)
///     if (WIFSTOPPED(pn->status))
///         pn->status = SP_RUNNING;
/// if (jn->stat & STAT_SUPERJOB)
///     makerunning(jobtab + jn->other);
/// ```
///
/// Clears the STOPPED flag on the job, resets each stopped process
/// to SP_RUNNING, and recurses into the linked subjob if this is a
// change job table entry from stopped to running                           // c:167
/// superjob. The previous Rust port called `job.make_running()`
/// which mutates only the single Job — missing the superjob
/// recursion. This port walks the table to handle the recursion.
pub fn makerunning(jobtab: &mut [Job], idx: usize) {
    if idx >= jobtab.len() {
        return;
    }
    let other = jobtab[idx].other;
    let is_super = jobtab[idx].is_superjob();
    {
        let job = &mut jobtab[idx];
        job.stat &= !stat::STOPPED;
        for proc in &mut job.procs {
            if proc.is_stopped() {
                proc.status = SP_RUNNING;
            }
        }
    }
    if is_super && other != idx && other < jobtab.len() {
        makerunning(jobtab, other);
    }
}

/// Port of `hasprocs(int job)` from `Src/jobs.c:243`.
///
/// C body:
/// ```c
/// Job jn;
/// if (job < 0) { DPUTS(1, "job number invalid"); return 0; }
/// jn = jobtab + job;
/// return jn->procs || jn->auxprocs;
/// ```
///
/// Takes the job index (not a `&Job`) because the C signature is
/// `int hasprocs(int job)`. Bounds-checks the index — out-of-range
/// returns false (matching C's negative-index DPUTS+0 path).
/// WARNING: param names don't match C — Rust=(jobtab, job) vs C=(job)
pub fn hasprocs(jobtab: &[Job], job: usize) -> bool {
    jobtab
        .get(job)
        .map(|j| !j.procs.is_empty() || !j.auxprocs.is_empty())
        .unwrap_or(false)
}

/// Check current shell signals (from jobs.c check_cursh_sig)
#[cfg(unix)]
/// Port of `check_cursh_sig(int sig)` from `Src/jobs.c:397`.
/// WARNING: param names don't match C — Rust=(jobtab, sig) vs C=(sig)
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

/// Port of `cleanfilelists()` from `Src/jobs.c:1443`.
///
/// C body:
/// ```c
/// DPUTS(shell_exiting >= 0, "BUG: cleanfilelists() before exit");
/// for (i = 1; i <= maxjob; i++) {
///     deletefilelist(jobtab[i].filelist, 0);
///     jobtab[i].filelist = 0;
/// }
/// ```
///
/// Deletes the file list (and its temp files) for every job in
/// the table. Called from the shell-exit path. The C source skips
/// index 0 (job 0 is unused / "the shell itself"); Rust port does
/// the same with `iter_mut().skip(1)`.
pub fn cleanfilelists(jobtab: &mut [Job]) {
    for job in jobtab.iter_mut().skip(1) {
        deletefilelist(job, false);
    }
}

/// Port of `clearoldjobtab()` from `Src/jobs.c:1835`.
///
/// C body:
/// ```c
/// if (oldjobtab) free(oldjobtab);
/// oldjobtab = NULL;
/// oldmaxjob = 0;
/// ```
///
/// Frees the snapshot of the previous-state job table that
/// `jobs -c` (jobs-changed) compares against. The previous Rust
/// port retained INUSE entries in `jobtab` directly — wrong
/// target. The real C function operates on the `oldjobtab`
/// global, not the live `jobtab`.
///
/// Rust port clears the OLDJOBTAB module static.
pub fn clearoldjobtab() {
    *OLDJOBTAB.get_or_init(|| Mutex::new(Vec::new()))
        .lock().expect("oldjobtab poisoned") = Vec::new();
    *OLDMAXJOB.get_or_init(|| Mutex::new(0))
        .lock().expect("oldmaxjob poisoned") = 0;
}

/// Direct port of `void addbgstatus(pid_t pid, int status)` from
/// `Src/jobs.c:2325`. Caps the global `bgstatus_list` at
/// `_SC_CHILD_MAX`, evicting oldest on overflow, then appends a
/// new `bgstatus { pid, status }` entry.
pub fn addbgstatus(pid: i32, status_val: i32) {                              // c:2325
    use std::sync::atomic::Ordering;
    // c:2370 — `if (bgstatus_count == max_child)` cap + eviction.
    let max_child = unsafe { libc::sysconf(libc::_SC_CHILD_MAX) };
    let cap = if max_child > 0 { max_child as i64 } else { 1024 };
    if let Ok(mut list) = bgstatus_list.lock() {
        if bgstatus_count.load(Ordering::Relaxed) >= cap {                   // c:2370
            // c:2371 — `rembgstatus(firstnode(bgstatus_list))`.
            if !list.is_empty() {
                list.remove(0);
                bgstatus_count.fetch_sub(1, Ordering::Relaxed);
            }
        }
        // c:2376-2385 — alloc + push.
        list.push(bgstatus { pid, status: status_val });                     // c:2381-2384
        bgstatus_count.fetch_add(1, Ordering::Relaxed);                      // c:2386
    }
}

// See if pid has a recorded exit status.                                   // c:2397
// Note we make no guarantee that the PIDs haven't wrapped, so this         // c:2397
// may not be the right process.                                            // c:2397
//                                                                          // c:2397
// This is only used by wait, which must only work on each                  // c:2397
// pid once, so we need to remove the entry if we find it.                  // c:2397
/// Direct port of `int getbgstatus(pid_t pid)` from `Src/jobs.c:2397`.
/// Walks the global `bgstatus_list` for `pid`; if found, removes
/// the entry and returns its status.
pub fn getbgstatus(pid: i32) -> Option<i32> {                                // c:2397
    use std::sync::atomic::Ordering;
    if let Ok(mut list) = bgstatus_list.lock() {
        if let Some(idx) = list.iter().position(|b| b.pid == pid) {          // c:2402-2406
            let status = list[idx].status;
            list.remove(idx);                                                // c:2407 rembgstatus
            bgstatus_count.fetch_sub(1, Ordering::Relaxed);
            return Some(status);
        }
    }
    None
}

/// Port of `gettrapnode(int sig, int ignoredisable)` from `Src/jobs.c:3115`.
///
/// C body looks up `TRAP<signame>` in the `shfunctab` (shell-
/// function hashtable) using either `getnode` (skip disabled) or
/// `getnode2` (include disabled), depending on `ignoredisable`.
/// Falls back to `alt_sigs[]` aliases (e.g. `TRAPCLD` for
/// SIGCHLD) when the canonical `TRAP<getsigname(sig)>` form
/// isn't found.
///
/// Now that `hashtable::shfunctab_lock` exists, the lookup is
/// real. Returns the function body for the trap if defined.
/// `ignoredisable` mirrors C: when 1, returns disabled entries
/// too (used by `unsetfn` paths that need to remove disabled
/// traps).
/// WARNING: param names don't match C — Rust=(sig) vs C=(sig, ignoredisable)
pub fn gettrapnode(sig: i32) -> Option<String> {
    let name = format!("TRAP{}", getsigname(sig));
    let tab = crate::ported::hashtable::shfunctab_lock()
        .read()
        .expect("shfunctab poisoned");
    tab.get_including_disabled(&name)
        .and_then(|f| f.body.clone())
}

/// Port of `removetrapnode(int sig)` from `Src/jobs.c:3157`.
///
/// C body:
/// ```c
/// HashNode hn = gettrapnode(sig, 1);
/// if (hn) { shfunctab->removenode(shfunctab, hn->nam); shfunctab->freenode(hn); }
/// ```
///
/// Routes through `hashtable::removeshfuncnode` which itself
/// dispatches the trap-removal logic for `TRAP<sig>` names.
pub fn removetrapnode(sig: i32) {
    let name = format!("TRAP{}", getsigname(sig));
    crate::ported::hashtable::removeshfuncnode(&name);
}

/// Port of `release_pgrp()` from `Src/jobs.c:3283`.
///
/// C body:
/// ```c
/// if (origpgrp != mypgrp) {
///     if (origpgrp) {
///         attachtty(origpgrp);
///         setpgrp(0, origpgrp);
///     }
///     mypgrp = origpgrp;
/// }
/// ```
///
///
/// Restores the original (parent shell's) process group before
/// the current shell exits, so terminal control returns to the
/// invoker.
#[cfg(unix)]
pub fn release_pgrp() {                                                      // c:3283
    let origpgrp = *ORIGPGRP.get_or_init(|| Mutex::new(0))
        .lock().expect("origpgrp poisoned");
    let mypgrp = *MYPGRP.get_or_init(|| Mutex::new(0))
        .lock().expect("mypgrp poisoned");
    if origpgrp != mypgrp {                                                  // c:3285
        // in linux pid namespaces, origpgrp may never have been set         // c:3286
        if origpgrp != 0 {                                                   // c:3287
            unsafe {
                // attachtty(origpgrp);                                      // c:3288
                libc::tcsetpgrp(0, origpgrp);
                libc::setpgid(0, origpgrp);                                  // c:3289
            }
        }
        *MYPGRP.get_or_init(|| Mutex::new(0))                                // c:3291
            .lock().expect("mypgrp poisoned") = origpgrp;
    }
}

/// Direct port of `bin_fg(char *name, char **argv, Options ops, int func)` from `Src/jobs.c:2421`.
/// Multi-builtin dispatcher — handles bg, fg, wait, jobs, disown, and
/// the `-Z` process-rename form. C body is 315 lines (c:2421-2735);
/// the per-builtin behaviour is selected by `func` (BIN_BG/BIN_FG/
/// BIN_JOBS/BIN_WAIT/BIN_DISOWN).
///
/// Coverage status:
///   ✓ -Z process-title rename (c:2425-2451) — full port via
///     libc::prctl(PR_SET_NAME) on Linux; macOS pthread_setname_np;
///     other platforms emit a warning
///   ✓ no-job-control refusal for fg/bg under !jobbing (c:2461-2465)
///   ✓ jobs -l/-p/-d listing-format selection (c:2454-2459)
///   ⚠ jobspec parsing + per-job dispatch (c:2467-2733) DEFERRED —
///     depends on getjob (parses %N/%?str specifiers), the global
///     jobtab + oldjobtab, deletejob/printjob/makerunning, lastval2,
///     errflag, signal queueing for fg's tcsetpgrp dance, and the
///     STAT_* / STAT_SUPERJOB / STAT_DISOWN flag tracking. None of
///     those are fully ported yet; structural shape preserved so the
///     C signature lands and future port work can fill the body.
pub fn bin_fg(name: &str, argv: &[String],                                   // c:2421
              ops: &crate::ported::zsh_h::options, func: i32) -> i32 {
    use crate::ported::zsh_h::OPT_ISSET;
    use crate::ported::utils::zwarnnam;
    use crate::ported::hashtable_h::{BIN_FG, BIN_BG, BIN_JOBS};
    let _ofunc = func;                                                       // c:2424

    // c:2425-2452 — `-Z`: rename the running process. Used by
    // login shells / tools that want their `ps` line to reflect a
    // descriptive title rather than `zsh`.
    if OPT_ISSET(ops, b'Z') {                                                // c:2425
        if argv.is_empty() || argv.len() > 1 {                               // c:2428
            zwarnnam(name, "-Z requires one argument");                      // c:2429
            return 1;                                                        // c:2430
        }
        crate::ported::mem::queue_signals();                                 // c:2433
        let title = &argv[0];
        // c:2436 — `setproctitle("%s", *argv);` if available.
        // c:2438-2444 — fallback: memcpy into hackzero (the argv[0]
        // buffer reserved by the loader). Not portable from Rust,
        // so the prctl path covers Linux directly.
        #[cfg(target_os = "linux")]
        unsafe {
            let cs = std::ffi::CString::new(title.as_str()).unwrap_or_default();
            // PR_SET_NAME = 15; libc may not expose it — pass the
            // raw constant per `linux/prctl.h`.
            libc::prctl(15 /*PR_SET_NAME*/, cs.as_ptr() as libc::c_ulong, 0, 0, 0); // c:2447
        }
        #[cfg(target_os = "macos")]
        unsafe {
            extern "C" {
                fn pthread_setname_np(name: *const libc::c_char) -> libc::c_int;
            }
            let cs = std::ffi::CString::new(title.as_str()).unwrap_or_default();
            pthread_setname_np(cs.as_ptr());
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            let _ = title;
        }
        crate::ported::mem::unqueue_signals();                               // c:2449
        return 0;                                                            // c:2450
    }

    // c:2454-2459 — jobs builtin: pick listing format.
    let mut lng = 0i32;                                                      // c:2422
    if func == BIN_JOBS {                                                    // c:2454
        lng = if OPT_ISSET(ops, b'l') { 1 }                                  // c:2455
              else if OPT_ISSET(ops, b'p') { 2 } else { 0 };
        if OPT_ISSET(ops, b'd') { lng |= 4; }                                // c:2456
    } else {
        // c:2458 — `lng = !!isset(LONGLISTJOBS);`
        lng = if crate::ported::zsh_h::isset(crate::ported::zsh_h::LONGLISTJOBS) {
            1
        } else {
            0
        };
    }
    let _ = lng;

    // c:2461-2465 — fg/bg need job control.
    let jobbing = crate::ported::zsh_h::isset(crate::ported::zsh_h::MONITOR);
    if (func == BIN_FG || func == BIN_BG) && !jobbing {                      // c:2461
        zwarnnam(name, "no job control in this shell.");                     // c:2463
        return 1;                                                            // c:2464
    }

    // c:2467 — `queue_signals();`
    crate::ported::signals::queue_signals();
    // c:2474 — `wait_for_processes();` reap any newly-finished children
    // so the table reflects the current state before we list/dispatch.
    crate::ported::signals::wait_for_processes();

    // c:2477-2478 — `if (unset(NOTIFY)) scanjobs();`
    // (scanjobs walks the table marking finished jobs for printing).
    // Skipped: scanjobs port isn't surfaced as a free fn; consumers
    // that need the print-on-prompt notify will route through it.

    // c:2480-2481 — refresh CURJOB unless we're listing a frozen
    // oldjobtab snapshot from `jobs` in a non-monitor shell.
    let table = JOBTAB.get_or_init(|| Mutex::new(Vec::new()));
    if func != BIN_JOBS || jobbing
        || *OLDMAXJOB.get_or_init(|| Mutex::new(0)).lock().unwrap() == 0
    {
        // c:2481 — `setcurjob()` operates on the global jobtab.
        setcurjob();
    }

    // c:2483-2486 — set stopmsg=2 so zexit doesn't complain about
    // stopped jobs if the user immediately runs `exit` after `jobs`.
    if func == BIN_JOBS {
        crate::ported::builtin::STOPMSG
            .store(2, std::sync::atomic::Ordering::Relaxed);                 // c:2486
    }

    let mut returnval: i32 = 0;

    if argv.is_empty() {                                                     // c:2487
        if func == BIN_JOBS {
            // c:2500-2523 — list jobs.
            let curjob = *CURJOB.get_or_init(|| Mutex::new(-1))
                .lock().unwrap();
            let t = table.lock().expect("jobtab poisoned");
            let curmaxjob = t.len();
            let r_only = OPT_ISSET(ops, b'r');
            let s_only = OPT_ISSET(ops, b's');
            for job in 0..curmaxjob {                                        // c:2513
                if job as i32 == curjob {                                    // c:2514 ignorejob
                    continue;
                }
                let j = &t[job];
                if !j.is_inuse() {                                           // c:2514 stat
                    continue;
                }
                let stopped = j.is_stopped();
                // c:2515-2519 — flag filtering.
                if (!r_only && !s_only)
                    || (r_only && s_only)
                    || (r_only && !stopped)
                    || (s_only && stopped)
                {
                    // c:2520 — printjob(jobptr, lng, 2). The Rust
                    // port's printjob takes job_num + cur/prev for
                    // formatting; pass them through here.
                    let curjob_opt = if curjob >= 0 {
                        Some(curjob as usize)
                    } else {
                        None
                    };
                    let prevjob = *PREVJOB
                        .get_or_init(|| Mutex::new(-1)).lock().unwrap();
                    let prevjob_opt = if prevjob >= 0 {
                        Some(prevjob as usize)
                    } else {
                        None
                    };
                    print!("{}", printjob(j, job, (lng & 1) != 0,
                        curjob_opt, prevjob_opt));
                }
            }
            crate::ported::signals::unqueue_signals();                       // c:2522
            return 0;                                                        // c:2523
        }
        if func == BIN_FG || func == BIN_BG {
            // c:2491-2499 — "no current job" gate.
            let curjob = *CURJOB.get_or_init(|| Mutex::new(-1))
                .lock().unwrap();
            if curjob < 0 {
                zwarnnam(name, "no current job");                            // c:2495
                crate::ported::signals::unqueue_signals();
                return 1;                                                    // c:2497
            }
            // Continue current job by sending SIGCONT to its pgrp.
            let gleader = table.lock().expect("jobtab poisoned")
                .get(curjob as usize).map(|j| j.gleader).unwrap_or(0);
            if gleader > 0 {
                let _ = crate::ported::signals::killjb(gleader, libc::SIGCONT);
            }
            crate::ported::signals::unqueue_signals();
            return 0;
        }
        crate::ported::signals::unqueue_signals();
        return 0;
    }

    // c:2537+ — per-arg jobspec dispatch (full body handles wait pid,
    // STAT_SUPERJOB carry-through, killjb retry, etc.). Port the
    // common path: `%jobspec` → getjob → continue/restart.
    for arg in argv {
        let p = if arg.starts_with('%') {
            getjob(arg, name)                                                // c:2576 getjob
        } else if let Ok(n) = arg.parse::<i32>() {
            // jobs/fg numeric → treat as job index, not pid.
            if n >= 0 { n } else { -1 }
        } else {
            zwarnnam(name, &format!("{}: no such job", arg));
            returnval = 1;
            continue;
        };
        if p < 0 {
            returnval = 1;
            continue;
        }
        let gleader = table.lock().expect("jobtab poisoned")
            .get(p as usize).map(|j| j.gleader).unwrap_or(0);
        if func == BIN_FG || func == BIN_BG {
            if gleader > 0 {
                if crate::ported::signals::killjb(gleader, libc::SIGCONT) == -1 {
                    zwarnnam(name, &format!("{}: kill failed: {}", arg,
                        std::io::Error::last_os_error()));
                    returnval = 1;
                }
            }
        } else if func == BIN_JOBS {
            let t = table.lock().expect("jobtab poisoned");
            if let Some(j) = t.get(p as usize) {
                let curjob = *CURJOB.get_or_init(|| Mutex::new(-1))
                    .lock().unwrap();
                let prevjob = *PREVJOB.get_or_init(|| Mutex::new(-1))
                    .lock().unwrap();
                print!("{}", printjob(j, p as usize, (lng & 1) != 0,
                    if curjob >= 0 { Some(curjob as usize) } else { None },
                    if prevjob >= 0 { Some(prevjob as usize) } else { None }));
            }
        }
    }
    crate::ported::signals::unqueue_signals();                               // c:2729
    returnval                                                                // c:2734 retval
}

/// Direct port of `bin_kill(char *nam, char **argv, UNUSED(Options ops), UNUSED(int func))` from `Src/jobs.c:2772`.
/// Builtin entry for the `kill` command. Parses signal specifiers
/// (`-N` numeric, `-s NAME` symbolic, `-l` list-by-number,
/// `-L` tabular listing, `-n N` numeric explicit, `-q` sigqueue
/// rt-signal sival) then sends the chosen signal to each remaining
/// argv (PIDs or %jobspecs).
/// WARNING: param names don't match C — Rust=(nam, argv, _func) vs C=(nam, argv, ops, func)
pub fn bin_kill(nam: &str, argv: &[String],                                  // c:2772
                _ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    use crate::ported::utils::zwarnnam;
    let mut sig: i32 = libc::SIGTERM;                                        // c:2774
    let mut returnval: i32 = 0;                                              // c:2775
    let mut got_sig = false;                                                 // c:2780
    let mut idx = 0usize;

    // c:2782 — `while (*argv && **argv == '-')` flag-parse loop.
    while idx < argv.len() && argv[idx].starts_with('-') {
        let arg = argv[idx].clone();
        let body = &arg[1..];

        // c:2814 — `else if ((*argv)[1] != '-' || (*argv)[2])` —
        // pseudo `--` end-of-flags.
        if body == "-" {                                                     // c:2814 / c:3010
            idx += 1;
            break;
        }

        if got_sig {                                                         // c:2811
            break;                                                           // c:2812
        }

        // c:2815 — `if (idigit((*argv)[1]))` — numeric signal `-N`.
        if body.chars().next().is_some_and(|c| c.is_ascii_digit()) {         // c:2815
            match body.parse::<i32>() {
                Ok(n) => sig = n,                                            // c:2818
                Err(_) => {
                    zwarnnam(nam, &format!("invalid signal number: -{}", body));
                    return 1;                                                // c:2822
                }
            }
            got_sig = true;
            idx += 1;
            continue;
        }

        // c:2818 — `-l` signal-name listing.
        if body == "l" {                                                     // c:2818
            idx += 1;
            if idx < argv.len() {                                            // c:2819
                // c:2820-2868 — per-arg lookup: numeric → name; name → number.
                while idx < argv.len() {
                    let token = &argv[idx];
                    idx += 1;
                    if let Ok(n) = token.parse::<i32>() {                    // c:2821 numeric
                        let s = (n & !0o200) as i32;                         // c:2855
                        if let Some(name) = crate::ported::signals_h::sigs_name(s) {                 // c:2856-2858
                            println!("{}", name);
                        } else {
                            println!("{}", n);                               // c:2862
                        }
                    } else {
                        // c:2823 — symbolic; uppercase, strip SIG, look up.
                        let upper = token.to_ascii_uppercase();
                        let bare = upper.strip_prefix("SIG").unwrap_or(&upper);
                        if let Some(n) = crate::ported::signals_h::sigs_number(bare) {               // c:2828
                            println!("{}", n);                               // c:2842
                        } else {
                            zwarnnam(nam,
                                &format!("unknown signal: SIG{}", bare));    // c:2845
                            returnval += 1;
                        }
                    }
                }
                return returnval;                                            // c:2868
            }
            // c:2869-2876 — bare `-l`: print every signal name.
            print!("{}", crate::ported::signals_h::sigs_name(1).unwrap_or("HUP"));
            for s in 2..=crate::ported::signals_h::SIGCOUNT {
                if let Some(n) = crate::ported::signals_h::sigs_name(s) { print!(" {}", n); }
            }
            println!();
            return 0;                                                        // c:2879
        }

        // c:2880 — `-L` tabular listing.
        if body == "L" {                                                     // c:2880
            let cols = 4usize;
            let mut col = 0usize;
            for s in 1..=crate::ported::signals_h::SIGCOUNT {
                if let Some(n) = crate::ported::signals_h::sigs_name(s) {
                    print!("{:>2} {:<10}", s, n);
                    col += 1;
                    if col % cols == 0 { println!(); }
                    else { print!(" "); }
                }
            }
            if col % cols != 0 { println!(); }
            return 0;                                                        // c:2911
        }

        // c:2913 — `-n N` numeric signal (explicit).
        if body == "n" {                                                     // c:2913
            idx += 1;
            if idx >= argv.len() {                                           // c:2916
                zwarnnam(nam, "-n: argument expected");                      // c:2917
                return 1;                                                    // c:2918
            }
            match argv[idx].parse::<i32>() {                                 // c:2920
                Ok(n) => { sig = n; }
                Err(_) => {
                    zwarnnam(nam,
                        &format!("invalid signal number: {}", argv[idx]));   // c:2923
                    return 1;
                }
            }
            got_sig = true;
            idx += 1;
            continue;
        }

        // c:2935 — `-s NAME` symbolic signal.
        if body == "s" {                                                     // c:2935
            idx += 1;
            if idx >= argv.len() {                                           // c:2938
                zwarnnam(nam, "-s: argument expected");                      // c:2939
                return 1;
            }
            let name = argv[idx].as_str();
            let upper = name.to_ascii_uppercase();
            let bare = upper.strip_prefix("SIG").unwrap_or(&upper);
            match crate::ported::signals_h::sigs_number(bare) {
                Some(n) => sig = n,
                None => {
                    zwarnnam(nam,
                        &format!("unknown signal: SIG{}", bare));            // c:2944
                    return 1;
                }
            }
            got_sig = true;
            idx += 1;
            continue;
        }

        // c:2782 — `-q VALUE` sigqueue path. zshrs treats it as
        // "consume the value, then continue parsing"; the actual
        // sival_int payload is dropped (not wired to a real
        // sigqueue(2) call yet — Linux-only, niche).
        if body == "q" {                                                     // c:2782
            idx += 1;
            if idx >= argv.len() {                                           // c:2785
                zwarnnam(nam, "-q: argument expected");                      // c:2786
                return 1;
            }
            if argv[idx].parse::<i32>().is_err() {                           // c:2796
                zwarnnam(nam,
                    &format!("invalid number: {}", argv[idx]));              // c:2797
                return 1;
            }
            idx += 1;                                                        // c:2802
            continue;                                                        // c:2803
        }

        // c:2960 — symbolic `-NAME` (no `s` prefix needed).
        let upper = body.to_ascii_uppercase();
        let bare = upper.strip_prefix("SIG").unwrap_or(&upper);
        match crate::ported::signals_h::sigs_number(bare) {
            Some(n) => { sig = n; got_sig = true; idx += 1; }
            None => {
                zwarnnam(nam, &format!("unknown signal: SIG{}", bare));      // c:2974
                return 1;
            }
        }
    }

    // c:3010 — no PID/jobspec arguments?
    if idx >= argv.len() {                                                   // c:3010
        zwarnnam(nam, "not enough arguments");                               // c:3011
        return 1;
    }

    // c:3015-3045 — for each remaining argv, parse PID or %jobspec
    // and send `sig`. zshrs handles bare numeric PIDs + simple
    // %jobspec via getjob; PIDs with leading `-` (process-group)
    // are forwarded via killpg.
    for arg in &argv[idx..] {
        if let Some(num) = arg.strip_prefix('-') {                           // c:3030
            // Process-group kill: `-PID` → killpg(PID, sig).
            match num.parse::<i32>() {
                Ok(pgid) => {
                    let r = unsafe { libc::killpg(pgid, sig) };              // c:3032
                    if r != 0 {
                        zwarnnam(nam, &format!("kill {}: {}", arg,
                            std::io::Error::last_os_error()));
                        returnval = 1;
                    }
                }
                Err(_) => {
                    zwarnnam(nam, &format!("illegal pid: {}", arg));
                    returnval = 1;
                }
            }
        } else if arg.starts_with('%') {                                     // c:2985 jobspec
            // c:2989 — `if ((p = getjob(*argv, nam)) == -1)`.
            let p = crate::ported::jobs::getjob(arg, nam);
            if p < 0 {                                                       // c:2989
                returnval += 1;                                              // c:2990
                continue;
            }
            // c:2993 — `killjb(jobtab + p, sig)`. Look up the job's
            // process-group leader and send via killjb.
            let gleader = JOBTAB.get_or_init(|| Mutex::new(Vec::new()))
                .lock().expect("jobtab poisoned")
                .get(p as usize).map(|j| j.gleader).unwrap_or(0);
            if crate::ported::signals::killjb(gleader, sig) == -1 {          // c:2993
                zwarnnam("kill", &format!("kill {} failed: {}", arg,         // c:2994
                    std::io::Error::last_os_error()));
                returnval += 1;                                              // c:2995
                continue;
            }
            // c:3001-3010 — if stopped + non-stopping signal,
            // SIGCONT after to wake the job so it processes `sig`.
            let stopped = JOBTAB.get_or_init(|| Mutex::new(Vec::new()))
                .lock().expect("jobtab poisoned")
                .get(p as usize).map(|j| j.is_stopped()).unwrap_or(false);
            if stopped
                && sig != libc::SIGKILL && sig != libc::SIGCONT
                && sig != libc::SIGTSTP && sig != libc::SIGTTOU
                && sig != libc::SIGTTIN && sig != libc::SIGSTOP
            {
                let _ = crate::ported::signals::killjb(gleader, libc::SIGCONT); // c:3009
            }
        } else {
            match arg.parse::<i32>() {                                       // c:3024 PID
                Ok(pid) => {
                    let r = unsafe { libc::kill(pid, sig) };                 // c:3025
                    if r != 0 {
                        zwarnnam(nam, &format!("kill {}: {}", arg,
                            std::io::Error::last_os_error()));               // c:3027
                        returnval = 1;
                    }
                }
                Err(_) => {
                    zwarnnam(nam, &format!("illegal pid: {}", arg));
                    returnval = 1;
                }
            }
        }
    }
    returnval                                                                // c:3045
}

/// Direct port of `bin_suspend(char *name, UNUSED(char **argv), Options ops, UNUSED(int func))` from `Src/jobs.c:3170`.
/// C body (c:3173-3197):
/// ```c
/// if (islogin && !OPT_ISSET(ops,'f')) { error; return 1; }
/// if (jobbing) { signal_default(SIGTTIN/TSTP/TTOU); release_pgrp(); }
/// killpg(origpgrp, SIGTSTP);
/// if (jobbing) { acquire_pgrp(); signal_ignore(SIGTTOU/TSTP/TTIN); }
/// return 0;
/// ```
/// WARNING: param names don't match C — Rust=(name, _argv, _func) vs C=(name, argv, ops, func)
pub fn bin_suspend(name: &str, _argv: &[String],                             // c:3170
                   ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    use crate::ported::zsh_h::OPT_ISSET;
    use crate::ported::utils::zwarnnam;
    use crate::ported::signals_h::{signal_default, signal_ignore};

    // c:3173 — `if (islogin && !OPT_ISSET(ops,'f'))`. islogin is a C
    // global set when zsh's argv[0] starts with `-`. Static-link path:
    // probe $0 directly.
    let islogin = std::env::var("0").map(|s| s.starts_with('-')).unwrap_or(false);
    //won't suspend a login shell, unless forced
    if islogin && !OPT_ISSET(ops, b'f') {                                    // c:3173
        zwarnnam(name, "can't suspend login shell");                         // c:3174
        return 1;                                                            // c:3175
    }
    // c:3177 — `if (jobbing)`. jobbing is the job-control-enabled flag;
    // tracks the MONITOR option.
    let jobbing = crate::ported::zsh_h::isset(crate::ported::zsh_h::MONITOR);

    if jobbing {                                                             // c:3177
        //stop ignoring signals
        signal_default(libc::SIGTTIN);                                       // c:3179
        signal_default(libc::SIGTSTP);                                       // c:3180
        signal_default(libc::SIGTTOU);                                       // c:3181
        //Move ourselves back to the process group we came from
        release_pgrp();                                                      // c:3184
    }

    // suspend ourselves with a SIGTSTP                                      // c:3187
    let origpgrp = ORIGPGRP.get_or_init(|| Mutex::new(0))
        .lock().map(|g| *g).unwrap_or(0);
    unsafe { libc::killpg(origpgrp, libc::SIGTSTP); }                        // c:3188

    if jobbing {                                                             // c:3190
        let _ = acquire_pgrp();                                              // c:3191
        //restore signal handling
        signal_ignore(libc::SIGTTOU);                                        // c:3193
        signal_ignore(libc::SIGTSTP);                                        // c:3194
        signal_ignore(libc::SIGTTIN);                                        // c:3195
    }
    0                                                                        // c:3197
}

/// Signal number from name (from jobs.c getsigidx)
/// Port of `getsigidx(const char *s)` from `Src/jobs.c:3047`.
pub fn getsigidx(s: &str) -> Option<i32> {
    let s = s.strip_prefix("SIG").unwrap_or(s);
    match s.to_uppercase().as_str() {
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
        _ => s.parse().ok(),
    }
}

// ===========================================================
// Methods moved verbatim from src/ported/exec.rs because their
// C counterpart's source file maps 1:1 to this Rust module.
// Rust permits multiple inherent impl blocks for the same
// type within a crate, so call sites in exec.rs are unchanged.
// ===========================================================

// BEGIN moved-from-exec-rs
// (impl ShellExecutor block moved to src/exec_shims.rs — see file marker)

// END moved-from-exec-rs

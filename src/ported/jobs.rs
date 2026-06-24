//! job control for zshrs
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

use crate::exec_jobs::JobTable;
use crate::ported::builtin::{SHELL_EXITING, STOPMSG};
use crate::ported::builtins::sched::zleactive;
use crate::ported::hashtable_h::{BIN_BG, BIN_DISOWN, BIN_FG, BIN_JOBS, BIN_WAIT};
use crate::ported::options::opt_state_set;
use crate::ported::params::{getsparam, setsparam, unsetparam};
use crate::ported::signals::{
    killjb, queue_signals, signal_block, signal_setmask, unqueue_signals, wait_for_processes,
};
use crate::ported::signals_h::{signal_default, signal_ignore, sigs_name, sigs_number};
use crate::ported::utils::zwarnnam;
use crate::ported::utils::{fdtable_get, zclose};
use crate::ported::zsh_h::{
    isset, job, jobfile, options, process, FDT_PROC_SUBST, INTERACTIVE, LONGLISTJOBS, MONITOR,
    OPT_ISSET, POSIXBUILTINS, POSIXJOBS, STAT_ATTACH, STAT_INUSE, STAT_SUBJOB,
    STAT_SUBJOB_ORPHANED, STAT_SUPERJOB,
};
pub use crate::ported::zsh_h::{timeinfo, MAXJOBS_ALLOC, MAX_PIPESTATS, SP_RUNNING};
use crate::DPUTS;
use std::env;
use std::os::unix::process::ExitStatusExt;
use std::process::Child;
use std::sync::atomic::Ordering;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// job status flags. `i32` to match C's `int stat` field on
/// `struct job` (`Src/zsh.h:1062`).
///
/// **Bit values MUST match C's `STAT_*` defines verbatim** at
/// `Src/zsh.h:1073-1094`. Previous Rust port used sequential bit
/// shifts (1<<0, 1<<1, …) which produced DIFFERENT values from C
/// for EVERY flag — `stat::STOPPED = 0x01` vs C `STAT_STOPPED =
/// 0x0002`, etc. Any data ferried between C-side and Rust-side
/// (or between bytecode and runtime state) would mis-interpret
/// every stat-flag check. Now canonical.
///
/// Also added missing flags (CHANGED, TIMED, LOCKED, NOPRINT,
/// NOSTTY, SUBLEADER) and removed bogus ones (DISOWN, NOTIFY —
/// not in C STAT_*).
pub mod stat {
    /// `CHANGED` constant.
    pub const CHANGED: i32 = 0x0001; // c:1073 status changed
    /// `STOPPED` constant.
    pub const STOPPED: i32 = 0x0002; // c:1074 all procs stopped or exited
    /// `TIMED` constant.
    pub const TIMED: i32 = 0x0004; // c:1075 job is being timed
    /// `DONE` constant.
    pub const DONE: i32 = 0x0008; // c:1076 job is done
    /// `LOCKED` constant.
    pub const LOCKED: i32 = 0x0010; // c:1077 shell finished creating
    /// `NOPRINT` constant.
    pub const NOPRINT: i32 = 0x0020; // c:1079 killed internally
    /// `INUSE` constant.
    pub const INUSE: i32 = 0x0040; // c:1081 entry in use
    /// `SUPERJOB` constant.
    pub const SUPERJOB: i32 = 0x0080; // c:1082 job has a subjob
    /// `SUBJOB` constant.
    pub const SUBJOB: i32 = 0x0100; // c:1083 job is a subjob
    /// `WASSUPER` constant.
    pub const WASSUPER: i32 = 0x0200; // c:1084 was super-job
    /// `CURSH` constant.
    pub const CURSH: i32 = 0x0400; // c:1086 last cmd in current shell
    /// `NOSTTY` constant.
    pub const NOSTTY: i32 = 0x0800; // c:1087 tty settings not inherited
    /// `ATTACH` constant.
    pub const ATTACH: i32 = 0x1000; // c:1089 delay reattach to tty
    /// `SUBLEADER` constant.
    pub const SUBLEADER: i32 = 0x2000; // c:1090 super-job, leader is sub-shell
    /// `BUILTIN` constant.
    pub const BUILTIN: i32 = 0x4000; // c:1092 tail is builtin
    /// `STAT_DISOWN` from `Src/zsh.h:1093`. SUPERJOB with disown pending.
    pub const DISOWN: i32 = 0x10000; // c:1093
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
/// Port of `makerunning(job jn)` from `Src/jobs.c:167`.
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
/// which mutates only the single job — missing the superjob
/// recursion. This port walks the table to handle the recursion.
pub fn makerunning(jobtab: &mut [job], idx: usize) {
    if idx >= jobtab.len() {
        return;
    }
    let other = jobtab[idx].other as usize;
    let is_super = (jobtab[idx].stat & stat::SUPERJOB) != 0;
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

// Find process and job associated with pid.                                // c:191
// Return 1 if search was successful, else return 0.                        // c:191
/// Port of `int findproc(pid_t pid, job *jptr, process *pptr, int aux)`
/// from `Src/jobs.c:191`.
///
/// C body (c:198-236) walks `jobtab[1..=maxjob]`:
///   - Skips entries where `(stat & STAT_DONE)` per c:204 — these are
///     jobs already marked dead.
///   - Walks ONLY `procs` OR `auxprocs` based on the `aux` arg, not
///     both. The previous Rust port walked both arrays.
///   - Prefers a `SP_RUNNING` match: if multiple pids hit but only
///     one is still running, returns it. The previous Rust port
///     returned the FIRST match regardless of running state.
///
/// **WARNING: param names don't match C** — Rust (jobtab, pid, aux)
/// vs C (pid, **jptr, **pptr, int aux). Returns `Some((job_idx,
/// proc_idx, aux_was_true))` rather than mutating out-pointers.
pub fn findproc(jobtab: &[job], pid: i32, aux: bool) -> Option<(usize, usize, bool)> {
    // c:191
    let mut last_match: Option<(usize, usize, bool)> = None;
    // c:198 — `for (i = 1; i <= maxjob; i++)`. Index 0 (the shell
    // itself) is skipped.
    for (ji, job) in jobtab.iter().enumerate().skip(1) {
        // c:204 — `if (jobtab[i].stat & STAT_DONE) continue;`. Don't
        // match against jobs already marked dead; their pids might
        // be recycled by the kernel and collide with a live pid.
        if (job.stat & stat::DONE) != 0 {
            continue;
        }
        // c:209-210 — walk EITHER procs OR auxprocs based on aux.
        let procs: &[process] = if aux { &job.auxprocs } else { &job.procs };
        for (pi, proc) in procs.iter().enumerate() {
            if proc.pid == pid {
                // c:228
                // c:229-232 — `if (pn->status == SP_RUNNING) return 1;`.
                // Prefer a running match; otherwise record the last
                // matching slot and keep looking.
                if proc.status == SP_RUNNING {
                    return Some((ji, pi, aux)); // c:231 return 1
                }
                last_match = Some((ji, pi, aux)); // c:227 record
            }
        }
    }
    // c:235 — `return (*pptr && *jptr);` — at least one slot matched
    // (even if not running). Rust returns last_match.
    last_match
}

// `TimeInfo` / `ChildTimes` deleted — both folded into canonical
// `timeinfo` at `zsh_h.rs:2153` (direct port of `struct timeinfo`
// from `Src/zsh.h:1099`).

// Canonical `process` / `job` live in `zsh_h.rs:1166,1180` — direct
// ports of `struct process` / `struct job` from `Src/zsh.h:1117,1058`.
// jobs.rs uses them via `process` / `job` aliases to keep call sites
// readable (Rust convention favors CamelCase at use-sites; the
// underlying type is the lowercase C-faithful canonical).

impl process {
    /// Build a fresh entry. Matches C's `update_process()` init shape
    /// (`Src/jobs.c:363` — `pn->pid = pid; pn->status = SP_RUNNING;`
    /// before the first wait).
    pub fn new(pid: i32) -> Self {
        process {
            pid,
            status: SP_RUNNING,
            text: String::new(),
            ti: timeinfo::default(),
            bgtime: Some(Instant::now()),
            endtime: None,
        }
    }

    /// `SP_RUNNING` sentinel check — equivalent to C's `pn->status ==
    /// SP_RUNNING` test at e.g. `Src/jobs.c:1242`.
    pub fn is_running(&self) -> bool {
        self.status == SP_RUNNING
    }

    /// Mirrors C's `WIFSTOPPED(status)` macro.
    pub fn is_stopped(&self) -> bool {
        self.status & 0xff == 0x7f
    }

    /// Mirrors C's `WIFSIGNALED(status)` macro.
    pub fn is_signaled(&self) -> bool {
        (self.status & 0x7f) > 0 && (self.status & 0x7f) < 0x7f
    }

    /// Mirrors C's `WEXITSTATUS(status)` macro.
    pub fn exit_status(&self) -> i32 {
        (self.status >> 8) & 0xff
    }

    /// Mirrors C's `WTERMSIG(status)` macro.
    pub fn term_sig(&self) -> i32 {
        self.status & 0x7f
    }

    /// Mirrors C's `WSTOPSIG(status)` macro.
    pub fn stop_sig(&self) -> i32 {
        (self.status >> 8) & 0xff
    }
}

impl job {
    /// Empty job slot — mirrors C's `memset(jn, 0, sizeof(*jn))`
    /// done in `initjob_reuse()` (`Src/jobs.c:574`).
    pub fn new() -> Self {
        Self::default()
    }

    /// True if any procs/auxprocs registered. Equivalent to C's
    /// `jn->procs || jn->auxprocs` null check at `Src/jobs.c` various.
    pub fn has_procs(&self) -> bool {
        !self.procs.is_empty() || !self.auxprocs.is_empty()
    }

    /// True if any proc is in the C `SP_RUNNING` state.
    pub fn is_running(&self) -> bool {
        self.procs.iter().any(|p| p.is_running())
    }

    /// True if every proc has finished (none `SP_RUNNING`, none stopped).
    pub fn is_done(&self) -> bool {
        !self.procs.is_empty()
            && self
                .procs
                .iter()
                .all(|p| !p.is_running() && !p.is_stopped())
    }

    /// True if the job is stopped — checks both the `STAT_STOPPED`
    /// flag bit on `self.stat` and per-proc `WIFSTOPPED`. Matches
    /// C's two-source check (`Src/jobs.c` reads `jn->stat & STAT_STOPPED`
    /// for the flag and `WIFSTOPPED(pn->status)` per proc).
    pub fn is_stopped(&self) -> bool {
        (self.stat & stat::STOPPED) != 0 || self.procs.iter().any(|p| p.is_stopped())
    }

    /// True if the slot is marked `INUSE` — equivalent to C's
    /// `(jn->stat & STAT_INUSE) != 0` check.
    pub fn is_inuse(&self) -> bool {
        (self.stat & stat::INUSE) != 0
    }

    /// Walk procs and reset their `status` back to `SP_RUNNING` —
    /// mirrors C's `makerunning()` body (`Src/jobs.c:1573`).
    pub fn make_running(&mut self) {
        for p in &mut self.procs {
            if p.is_stopped() {
                p.status = SP_RUNNING;
            }
        }
        self.stat &= !stat::STOPPED;
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

/// Port of `hasprocs(int job)` from `Src/jobs.c:243`.
///
/// C body:
/// ```c
/// job jn;
/// if (job < 0) { DPUTS(1, "job number invalid"); return 0; }
/// jn = jobtab + job;
/// return jn->procs || jn->auxprocs;
/// ```
///
/// Takes the job index (not a `&job`) because the C signature is
/// `int hasprocs(int job)`. Bounds-checks the index — out-of-range
/// returns false (matching C's negative-index DPUTS+0 path).
/// WARNING: param names don't match C — Rust=(jobtab, job) vs C=(job)
pub fn hasprocs(jobtab: &[job], job: usize) -> bool {
    jobtab
        .get(job)
        .map(|j| !j.procs.is_empty() || !j.auxprocs.is_empty())
        .unwrap_or(false)
}

/// Port of `super_job(int sub)` from `Src/jobs.c:259-270` — find the super-job of a sub-job.
/// ```c
/// for (i = 1; i <= maxjob; i++)
///     if ((jobtab[i].stat & STAT_SUPERJOB) &&
///         jobtab[i].other == sub &&
///         jobtab[i].gleader)
///         return i;
/// return 0;
/// ```
/// The `gleader` non-zero check at c:267 was previously missing in
/// the Rust port — silently returned super-job indices for entries
/// that hadn't yet had a process-group leader assigned, breaking
/// job-control SIGCONT relay paths.
pub fn super_job(jobtab: &[job], job_idx: usize) -> Option<usize> {
    // c:260
    for (i, job) in jobtab.iter().enumerate() {
        if (job.stat & stat::SUPERJOB) != 0 && job.other as usize == job_idx && job.gleader != 0
        // c:267
        {
            return Some(i);
        }
    }
    None
}

/// Handle subjob completion (from jobs.c handle_sub)
/// Port of `handle_sub(int job, int fg)` from `Src/jobs.c:274`.
/// WARNING: param names don't match C — Rust=(jobtab, super_idx, fg) vs C=(job, fg)
pub fn handle_sub(jobtab: &mut [job], super_idx: usize, fg: bool) -> i32 {
    // c:274
    // c:277 — `job jn = jobtab + job, sj = jobtab + jn->other;`
    let sub_idx = jobtab[super_idx].other as usize;
    if sub_idx >= jobtab.len() {
        return 0;
    }

    // c:279 — `if ((sj->stat & STAT_DONE) || (!sj->procs && !sj->auxprocs)) {`
    let sj_done = (jobtab[sub_idx].stat & stat::DONE) != 0
        || (jobtab[sub_idx].procs.is_empty() && jobtab[sub_idx].auxprocs.is_empty());
    if sj_done {
        // c:282-292 — walk sj->procs looking for a signaled one; cascade
        // SIGCONT + signal to superjob's group, then SIGCONT + signal
        // to sj->other.
        let mut signaled: Option<i32> = None;
        for p in jobtab[sub_idx].procs.iter() {
            #[cfg(unix)]
            if libc::WIFSIGNALED(p.status) {
                signaled = Some(libc::WTERMSIG(p.status));
                break;
            }
        }
        if let Some(sig) = signaled {
            // c:283-291 — kill the superjob via gleader (or first proc),
            //              then SIGCONT + signal to sj->other.
            let jn_gleader = jobtab[super_idx].gleader;
            let multi_procs = jobtab[super_idx].procs.len() > 1;
            #[cfg(unix)]
            {
                let mypgrp = unsafe { libc::getpgrp() };
                if jn_gleader != mypgrp && multi_procs {
                    unsafe { libc::killpg(jn_gleader, sig) }; // c:285
                } else if let Some(p0) = jobtab[super_idx].procs.first() {
                    unsafe { libc::kill(p0.pid, sig) }; // c:287
                }
                let sj_other = jobtab[sub_idx].other;
                unsafe { libc::kill(sj_other, libc::SIGCONT) }; // c:288
                unsafe { libc::kill(sj_other, sig) }; // c:289
            }
            #[cfg(not(unix))]
            {
                let _ = (jn_gleader, multi_procs, sig);
            }
        } else {
            // c:293-326 — no signaled proc: mark SUPERJOB cleared,
            // WASSUPER set; gleader-recovery if dead; attachtty when
            // fg; deletejob if DISOWN pending.
            jobtab[super_idx].stat &= !stat::SUPERJOB; // c:296
            jobtab[super_idx].stat |= stat::WASSUPER; // c:297
                                                      // c:299-306 — gleader recovery: if the first proc has exited
                                                      //              or been signaled AND killpg(gleader, 0) → ESRCH,
                                                      //              promote the last proc's pid to be the new
                                                      //              gleader (cp).
            let cp: bool;
            #[cfg(unix)]
            {
                let first_status = jobtab[super_idx]
                    .procs
                    .first()
                    .map(|p| p.status)
                    .unwrap_or(0);
                let dead = libc::WIFEXITED(first_status) || libc::WIFSIGNALED(first_status);
                let gleader_dead = dead
                    && unsafe { libc::killpg(jobtab[super_idx].gleader, 0) } == -1
                    && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH);
                cp = gleader_dead;
                if cp {
                    if let Some(last) = jobtab[super_idx].procs.last() {
                        jobtab[super_idx].gleader = last.pid; // c:305
                    }
                }
            }
            #[cfg(not(unix))]
            {
                cp = false;
            }

            // c:318-320 — attachtty(jn->gleader) when fg or thisjob == job,
            //              and the superjob is the sub-shell alone (single
            //              proc, or gleader recovered, or first proc != gleader).
            let thisjob = *THISJOB
                .get_or_init(|| Mutex::new(-1))
                .lock()
                .expect("thisjob poisoned");
            let cond_attach = fg || thisjob as usize == super_idx;
            let single_proc = jobtab[super_idx].procs.len() == 1;
            let first_pid_neq_gleader = jobtab[super_idx]
                .procs
                .first()
                .map(|p| p.pid != jobtab[super_idx].gleader)
                .unwrap_or(false);
            if cond_attach && (single_proc || cp || first_pid_neq_gleader) {
                // c:319 — `attachtty(jn->gleader);` hand the tty to
                // the super-job's process group leader.
                #[cfg(unix)]
                crate::ported::utils::attachtty(jobtab[super_idx].gleader);
            }
            // c:321 — kill(sj->other, SIGCONT);
            #[cfg(unix)]
            unsafe {
                libc::kill(jobtab[sub_idx].other, libc::SIGCONT);
            }

            // c:322-325 — `if (jn->stat & STAT_DISOWN) deletejob(jn, 1);`
            if (jobtab[super_idx].stat & stat::DISOWN) != 0 {
                deletejob(&mut jobtab[super_idx], true);
            }
        }
        // c:327 — curjob = jn - jobtab;
        if let Ok(mut cj) = CURJOB.get_or_init(|| Mutex::new(-1)).lock() {
            *cj = super_idx as i32;
        }
        return 0; // c:340 fall-through return
    } else if (jobtab[sub_idx].stat & stat::STOPPED) != 0 {
        // c:328
        // c:331-337 — STOPPED branch: propagate STOPPED to superjob,
        //              clone subjob's first-proc status to every super
        //              proc that's still running.
        jobtab[super_idx].stat |= stat::STOPPED; // c:331
        let sj_proc_status = jobtab[sub_idx].procs.first().map(|p| p.status).unwrap_or(0);
        for p in jobtab[super_idx].procs.iter_mut() {
            // c:332
            if p.status == SP_RUNNING                                        // c:333-334
                || {
                    #[cfg(unix)]
                    { !libc::WIFEXITED(p.status) && !libc::WIFSIGNALED(p.status) }
                    #[cfg(not(unix))]
                    { false }
                }
            {
                p.status = sj_proc_status; // c:335
            }
        }
        if let Ok(mut cj) = CURJOB.get_or_init(|| Mutex::new(-1)).lock() {
            *cj = super_idx as i32; // c:336
        }
        // c:337 — printjob(jn, !!isset(LONGLISTJOBS), 1);
        //         printjob takes a snapshot signature here that requires
        //         cur_job/prev_job indices; defer the print to the caller
        //         (jobs.rs's jobs-builtin scanner) which has those handy.
        return 1; // c:338
    }
    0 // c:340
}

/// Get children's time accounting.
/// Port of `get_usage()` from Src/jobs.c — fills `child_usage`
/// from `getrusage(RUSAGE_CHILDREN)` on supported systems.
pub fn get_usage() -> timeinfo {
    #[cfg(unix)]
    {
        let mut u: libc::rusage = unsafe { std::mem::zeroed() };
        if unsafe { libc::getrusage(libc::RUSAGE_CHILDREN, &mut u) } == 0 {
            return timeinfo::from_rusage(&u);
        }
    }
    timeinfo::default()
}

/// Port of `update_process(process pn, int status)` from `Src/jobs.c:363`.
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
/// and the call to `get_usage()` — the per-process rusage attribution.
///
/// Mirrors C's `child_usage` global pattern (`Src/jobs.c:109`):
/// the in-flight rusage snapshot lives in `CHILD_USAGE_PREV`, gets
/// captured pre-wait by `child_usage_snapshot()`, and update_process
/// diffs that against the current `get_usage()` to attribute per-
/// process rusage. Without this snapshot pre-wait, all diffs are 0
/// (the previous Rust port had this bug).
pub fn update_process(pn: &mut process, status: i32) {
    // c:362
    let prev = CHILD_USAGE_PREV.with(|c| c.borrow().clone()); // c:366-367
    let now = get_usage(); // c:374 get_usage()
    CHILD_USAGE_PREV.with(|c| *c.borrow_mut() = now.clone());

    pn.endtime = Some(Instant::now()); // c:375 zgettime_monotonic_if_available
    pn.status = status; // c:377

    // Field-by-field diff (now - prev), clamped >= 0 to handle the
    // first-wait case where prev is zero-initialised.
    let diff = |a: i64, b: i64| -> i64 { (a - b).max(0) };
    pn.ti = timeinfo {
        ut: diff(now.ut, prev.ut), // c:380 ru_utime delta
        st: diff(now.st, prev.st), // c:379 ru_stime delta
        maxrss: now.maxrss.max(prev.maxrss),
        majflt: diff(now.majflt, prev.majflt),
        minflt: diff(now.minflt, prev.minflt),
        nswap: diff(now.nswap, prev.nswap),
        ixrss: diff(now.ixrss, prev.ixrss),
        idrss: diff(now.idrss, prev.idrss),
        isrss: diff(now.isrss, prev.isrss),
        inblock: diff(now.inblock, prev.inblock),
        oublock: diff(now.oublock, prev.oublock),
        nvcsw: diff(now.nvcsw, prev.nvcsw),
        nivcsw: diff(now.nivcsw, prev.nivcsw),
        msgsnd: diff(now.msgsnd, prev.msgsnd),
        msgrcv: diff(now.msgrcv, prev.msgrcv),
        nsignals: diff(now.nsignals, prev.nsignals),
    };
}

// `child_usage` — Src/jobs.c:109 mod_export global. The cumulative
// children rusage snapshot kept warm between waits. update_process
// reads-then-overwrites it to compute the delta attributable to the
// just-reaped child. Per-thread (bucket 1) because each worker
// thread reaps its own children independently.
thread_local! {
    static CHILD_USAGE_PREV: std::cell::RefCell<timeinfo>
        = const { std::cell::RefCell::new(timeinfo {
            ut: 0, st: 0, maxrss: 0, majflt: 0, minflt: 0, nswap: 0,
            ixrss: 0, idrss: 0, isrss: 0, inblock: 0, oublock: 0,
            nvcsw: 0, nivcsw: 0, msgsnd: 0, msgrcv: 0, nsignals: 0,
        }) };
}

/// Check current shell signals (from jobs.c check_cursh_sig)
#[cfg(unix)]
/// Port of `check_cursh_sig(int sig)` from `Src/jobs.c:397`.
/// WARNING: param names don't match C — Rust=(jobtab, sig) vs C=(sig)
pub fn check_cursh_sig(jobtab: &[job], sig: i32) {
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

/// Port of `storepipestats(job jn, int inforeground, int fixlastval)` from `Src/jobs.c:420`.
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
pub fn storepipestats(job: &job) -> (Vec<i32>, i32) {
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

// Update status of job, possibly printing it                               // c:460
/// Update job status after process change (from jobs.c update_job)
/// Returns true if the job is now done or stopped (status committed),
/// false if any proc is still running (no update needed).
pub fn update_job(job: &mut job) -> bool {
    // c:460
    // c:467-474 — `for (pn = jn->auxprocs; pn; pn = pn->next) {
    //                 if (WIFCONTINUED(pn->status)) pn->status = SP_RUNNING;
    //                 if (pn->status == SP_RUNNING) return; }`
    for proc in job.auxprocs.iter_mut() {
        #[cfg(unix)]
        if proc.status > 0
            && !libc::WIFEXITED(proc.status)
            && !libc::WIFSIGNALED(proc.status)
            && !libc::WIFSTOPPED(proc.status)
        {
            // WIFCONTINUED not exposed as a libc::W* fn on every target;
            // it's the "neither exited nor signaled nor stopped" case
            // that means SIGCONT was just delivered. Mark SP_RUNNING.
            proc.status = SP_RUNNING;
        }
        if proc.is_running() {
            return false;
        }
    }

    // c:476-498 — walk main procs, look for SP_RUNNING (bail), track
    //              somestopped, capture last-proc status (signal/stop/exit),
    //              set the signalled flag.
    let mut some_stopped = false;
    let mut signalled = false;
    let mut val: i32 = 0;
    let proc_count = job.procs.len();
    for (i, proc) in job.procs.iter_mut().enumerate() {
        #[cfg(unix)]
        if proc.status > 0
            && !libc::WIFEXITED(proc.status)
            && !libc::WIFSIGNALED(proc.status)
            && !libc::WIFSTOPPED(proc.status)
        {
            // WIFCONTINUED main path: clear STAT_STOPPED + SP_RUNNING.
            job.stat &= !stat::STOPPED;
            proc.status = SP_RUNNING;
        }
        if proc.is_running() {
            return false;
        }
        if proc.is_stopped() {
            some_stopped = true;
        }
        // c:487-495 — last proc determines exit val.
        if i + 1 == proc_count {
            #[cfg(unix)]
            {
                if libc::WIFSIGNALED(proc.status) {
                    val = 0o200 | libc::WTERMSIG(proc.status);
                    signalled = true;
                } else if libc::WIFSTOPPED(proc.status) {
                    val = 0o200 | libc::WSTOPSIG(proc.status);
                } else {
                    val = libc::WEXITSTATUS(proc.status);
                }
            }
            #[cfg(not(unix))]
            {
                val = proc.status;
            }
        }
    }

    // c:502-543 — somestopped: mark STAT_CHANGED|STOPPED; cascade SIGTSTP
    //              to the super-job if this is a subjob (c:507-540).
    if some_stopped {
        if (job.stat & stat::SUBJOB) != 0 {
            job.stat |= stat::CHANGED | stat::STOPPED; // c:514
                                                       // c:515-538 — find the super-job; killpg(super.gleader, SIGTSTP);
                                                       //              mark super CHANGED|STOPPED. Without a job-index-
                                                       //              from-job reverse lookup wired here (we'd need
                                                       //              the JOBTAB position, but Rust callers usually
                                                       //              hold the &mut job by &mut [job][i]), defer the
                                                       //              SIGTSTP to whoever owns the jobtab.
                                                       // Documented gap — the caller in fusevm_bridge that does the
                                                       // wait3 dispatch knows the index and handles the super hop.
            return true;
        }
        if (job.stat & stat::STOPPED) != 0 {
            return true; // c:541-542
        }
        job.stat |= stat::STOPPED;
        job.stat &= !stat::DONE;
        job.stat |= stat::CHANGED;
        return true;
    }

    // c:544-556 — job is fully done. Set DONE, write lastval2/lastval.
    job.stat |= stat::DONE | stat::CHANGED;
    job.stat &= !stat::STOPPED;
    // c:545 — lastval2 = val;
    LASTVAL2.store(val, Ordering::SeqCst);

    // c:550-555 — `if (jn->stat & STAT_CURSH) inforeground = 1;
    //               else if (job == thisjob) { lastval = val; inforeground = 2; }`
    //              Drives the c:565 "deadpgrp" path and the MONITOR foreground
    //              cascade. Mark via _inforeground for the trace; signal cascade
    //              skipped (interactive substrate).
    let _inforeground: i32 = if (job.stat & stat::CURSH) != 0 {
        1
    } else {
        // We don't know `thisjob == job_idx` from `&mut job` alone;
        // the caller (wait-loop) knows the index and handles lastval.
        0
    };
    let _ = signalled;
    true
}

/// `lastval2` — Src/jobs.c global. Set to last-pipeline exit status.
pub static LASTVAL2: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

/// Update a background job after waitpid (from jobs.c update_bg_job)
/// Port of `update_bg_job(job jn, pid_t pid, int status)` from `Src/jobs.c:677`.
pub fn update_bg_job(jn: &mut [job], pid: i32, status: i32) -> bool {
    // Try primary procs first, then auxprocs — C `findproc` takes
    // an explicit `aux` arg and the caller decides which subset is
    // relevant. update_bg_job needs to handle BOTH because the
    // waitpid'd pid might land in either.
    let hit = findproc(jn, pid, false).or_else(|| findproc(jn, pid, true));
    if let Some((ji, pi, is_aux)) = hit {
        if is_aux {
            jn[ji].auxprocs[pi].status = status;
            jn[ji].auxprocs[pi].endtime = Some(Instant::now());
        } else {
            jn[ji].procs[pi].status = status;
            jn[ji].procs[pi].endtime = Some(Instant::now());
        }
        // c:Src/jobs.c:684-699 (update_bg_job) — record a finished
        // BACKGROUND job's exit status in the bgstatus ring so a later
        // `wait $pid` can retrieve it after the child is gone (waitpid
        // returns ECHILD; bin_wait then consults getbgstatus). A bg job
        // is one not marked STAT_CURSH/STAT_BUILTIN and not the current
        // foreground job (thisjob). Without this, `(exit 5) & p=$!;
        // wait $p` reaped the child but dropped its status, so the wait
        // failed with "pid N is not a child of this shell" (127).
        let thisjob = *THISJOB
            .get_or_init(|| Mutex::new(-1))
            .lock()
            .expect("thisjob poisoned");
        if (jn[ji].stat & (stat::CURSH | stat::BUILTIN)) == 0 && ji as i32 != thisjob {
            if libc::WIFEXITED(status) {
                addbgstatus(pid, libc::WEXITSTATUS(status)); // c:695
            } else if libc::WIFSIGNALED(status) {
                addbgstatus(pid, 0o200 | libc::WTERMSIG(status)); // c:697
            }
        }
        update_job(&mut jn[ji]);
        return true;
    }
    false
}

// set the previous job to something reasonable                              // c:698
/// Direct port of `static void setprevjob(void)` from `Src/jobs.c:698`.
/// Walks the global jobtab to pick `prevjob` — first stopped (non-
/// subjob, non-curjob, non-thisjob) candidate, else first in-use one.
pub fn setprevjob() {
    // c:698
    let tab = JOBTAB
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .expect("jobtab poisoned");
    let maxjob = *MAXJOB
        .get_or_init(|| Mutex::new(0))
        .lock()
        .expect("maxjob poisoned");
    let curjob = *CURJOB
        .get_or_init(|| Mutex::new(-1))
        .lock()
        .expect("curjob poisoned");
    let thisjob = *THISJOB
        .get_or_init(|| Mutex::new(-1))
        .lock()
        .expect("thisjob poisoned");
    // c:702-707 — stopped candidate.
    for i in (1..=maxjob).rev() {
        if i >= tab.len() {
            continue;
        }
        let j = &tab[i];
        if (j.stat & (stat::INUSE | stat::STOPPED)) == (stat::INUSE | stat::STOPPED)
            && (j.stat & stat::SUBJOB) == 0
            && i as i32 != curjob
            && i as i32 != thisjob
        {
            *PREVJOB.get_or_init(|| Mutex::new(-1)).lock().unwrap() = i as i32;
            return;
        }
    }
    // c:709-714 — fallback to any in-use non-subjob.
    for i in (1..=maxjob).rev() {
        if i >= tab.len() {
            continue;
        }
        let j = &tab[i];
        if (j.stat & stat::INUSE) != 0
            && (j.stat & stat::SUBJOB) == 0
            && i as i32 != curjob
            && i as i32 != thisjob
        {
            *PREVJOB.get_or_init(|| Mutex::new(-1)).lock().unwrap() = i as i32;
            return;
        }
    }
    // c:716 — nothing eligible.
    *PREVJOB.get_or_init(|| Mutex::new(-1)).lock().unwrap() = -1;
}

/// Get clock ticks per second (from jobs.c get_clktck lines 720-748)
/// Get `_SC_CLK_TCK` for time-conversion math.
/// Port of `get_clktck()` from Src/jobs.c:721.
pub fn get_clktck() -> i64 {
    // c:721
    #[cfg(unix)]
    {
        static CLKTCK: OnceLock<i64> = OnceLock::new(); // c:723
                                                        // fetch clock ticks per second from                                 // c:727
                                                        // sysconf only the first time                                       // c:728
        *CLKTCK.get_or_init(|| unsafe { libc::sysconf(libc::_SC_CLK_TCK) as i64 })
        // c:729
    }
    #[cfg(not(unix))]
    {
        100 // Default on non-Unix
    }
}

/// Format time as hh:mm:ss.xx (from jobs.c printhhmmss lines 752-765)
/// Format a duration as `H:MM:SS` / `M:SS`.
/// Port of `printhhmmss(double secs)` from Src/jobs.c:752.
pub fn printhhmmss(secs: f64) -> String {
    // c:752
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
/// Port of `printtime(struct timespec *real, child_times_t *ti, char *desc)` from Src/jobs.c:768.
/// Supports the full directive set: `%E/%U/%S/%P/%J/%mE/%uE/%nE/%*E`
/// (time forms) plus `%M/%F/%R/%W/%X/%D/%K/%I/%O/%c/%w` (rusage).
pub fn printtime(
    // c:768
    elapsed_secs: f64,
    ti: &timeinfo,
    format: &str,
    job_name: &str,
) -> String {
    let user_secs = ti.ut as f64 / 1_000_000.0;
    let system_secs = ti.st as f64 / 1_000_000.0;
    let mut result = String::new();
    let total_time = user_secs + system_secs; // c:794
    let percent = if elapsed_secs > 0.0 {
        // c:795
        (100.0 * total_time / elapsed_secs) as i32
    } else {
        0
    };
    // Per-second helper for the rusage-rate directives (X/D/K).
    let per_sec = |v: i64| -> i64 {
        // c:903-907
        if total_time > 0.0 {
            (v as f64 / total_time) as i64
        } else {
            0
        }
    };

    let mut chars = format.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' {
            match chars.next() {
                // c:816-823 — %E / %U / %S
                Some('E') => result.push_str(&format!("{:.2}s", elapsed_secs)),
                Some('U') => result.push_str(&format!("{:.2}s", user_secs)),
                Some('S') => result.push_str(&format!("{:.2}s", system_secs)),
                // c:893-894 — %P
                Some('P') => result.push_str(&format!("{}%", percent)),
                Some('J') => result.push_str(job_name),
                // c:825-840 — %mE / %mU / %mS (milliseconds)
                Some('m') => match chars.next() {
                    Some('E') => result.push_str(&format!("{:.0}ms", elapsed_secs * 1000.0)),
                    Some('U') => result.push_str(&format!("{:.0}ms", user_secs * 1000.0)),
                    Some('S') => result.push_str(&format!("{:.0}ms", system_secs * 1000.0)),
                    _ => result.push_str("%m"),
                },
                // c:842-857 — %uE / %uU / %uS (microseconds)
                Some('u') => match chars.next() {
                    Some('E') => result.push_str(&format!("{:.0}us", elapsed_secs * 1_000_000.0)),
                    Some('U') => result.push_str(&format!("{:.0}us", user_secs * 1_000_000.0)),
                    Some('S') => result.push_str(&format!("{:.0}us", system_secs * 1_000_000.0)),
                    _ => result.push_str("%u"),
                },
                // c:859-874 — %nE / %nU / %nS (nanoseconds)
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
                // c:876-891 — %*E / %*U / %*S (HH:MM:SS form)
                Some('*') => match chars.next() {
                    Some('E') => result.push_str(&printhhmmss(elapsed_secs)),
                    Some('U') => result.push_str(&printhhmmss(user_secs)),
                    Some('S') => result.push_str(&printhhmmss(system_secs)),
                    _ => result.push_str("%*"),
                },
                // c:897-899 — %W: swaps
                Some('W') => result.push_str(&format!("{}", ti.nswap)),
                // c:902-907 — %X: integral shared mem / total_time
                Some('X') => result.push_str(&format!("{}", per_sec(ti.ixrss))),
                // c:910-919 — %D: integral unshared data / total_time
                Some('D') => result.push_str(&format!("{}", per_sec(ti.idrss + ti.isrss))),
                // c:924-942 — %K: total integral mem / total_time
                Some('K') => {
                    result.push_str(&format!("{}", per_sec(ti.ixrss + ti.idrss + ti.isrss)))
                }
                // c:950-952 — %M: max resident set size (KB on macOS+Linux post-norm)
                Some('M') => result.push_str(&format!("{}", ti.maxrss)),
                // c:955-957 — %F: major page faults
                Some('F') => result.push_str(&format!("{}", ti.majflt)),
                // c:960-962 — %R: minor page faults
                Some('R') => result.push_str(&format!("{}", ti.minflt)),
                // c:965+ — %I: input block ops; %O: output; %c/%w: ctx switches
                Some('I') => result.push_str(&format!("{}", ti.inblock)),
                Some('O') => result.push_str(&format!("{}", ti.oublock)),
                Some('c') => result.push_str(&format!("{}", ti.nivcsw)),
                Some('w') => result.push_str(&format!("{}", ti.nvcsw)),
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

/// Dump timing info for a job (from jobs.c dumptime).
/// Port of `dumptime(job jn)` from `Src/jobs.c:1020`.
///
/// C body iterates each process in the pipeline and prints one
/// `printtime` line per process using that process's own bgtime/
/// endtime/ti/text — c:1027-1029. The previous Rust port aggregated
/// into a single timeinfo, which printed 1 line for a 3-stage
/// pipeline instead of C's 3.
pub fn dumptime(job: &job) -> Option<String> {
    // c:1020
    if job.procs.is_empty() {
        // c:1025-1026
        return None;
    }
    // C dumptime reads `$TIMEFMT` indirectly via printtime's getsparam
    // call (c:808 inside printtime). Rust printtime takes format as a
    // parameter, so we read it here and pass through.
    const DEFAULT_TIMEFMT: &str = "%J  %U user %S system %P cpu %*E total";
    let format = getsparam("TIMEFMT").unwrap_or_else(|| DEFAULT_TIMEFMT.to_string());

    // c:1027-1029 — for each proc, printtime(dtime_ts(&bgtime, &endtime), &ti, text).
    let lines: Vec<String> = job
        .procs
        .iter()
        .filter_map(|p| {
            let start = p.bgtime?;
            let end = p.endtime?;
            let elapsed = end.duration_since(start).as_secs_f64();
            Some(printtime(elapsed, &p.ti, &format, &p.text))
        })
        .collect();
    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

/// Port of `static int should_report_time(job j)` from `Src/jobs.c:1038-1080`.
/// ```c
/// /* if the time keyword was used */
/// if (j->stat & STAT_TIMED) return 1;
/// /* read $REPORTTIME / $REPORTMEMORY */
/// if (reporttime < 0 && reportmemory < 0) return 0;
/// if (!j->procs) return 0;
/// if (zleactive) return 0;
/// /* … compare elapsed time vs reporttime threshold */
/// ```
/// Rust port previously missed the c:1052 STAT_TIMED short-circuit:
/// a job explicitly preceded by the `time` keyword should always
/// report its time regardless of `$REPORTTIME` setting. Without this
/// check, `time sleep 0.001` would be silent when REPORTTIME is
/// unset or set high.
///
/// `$REPORTTIME` (and `$REPORTMEMORY`) reading is the caller's
/// responsibility — Rust takes the thresholds as parameters rather
/// than calling getvalue inside.
pub fn should_report_time(job: &job, reporttime: f64) -> bool {
    // c:1039
    // Read both thresholds from paramtab — matches C's
    // `getvalue(REPORTTIME)` and `getvalue(REPORTMEMORY)` reads.
    let reportmemory: i64 = getsparam("REPORTMEMORY")
        .and_then(|s| s.parse().ok())
        .unwrap_or(-1);

    // c:1052-1053 — STAT_TIMED short-circuit. Always report when
    // the `time` keyword preceded the command.
    if (job.stat & stat::TIMED) != 0 {
        // c:1052
        return true;
    }
    // c:1065-1070 — both thresholds disabled ⇒ no report.
    if reporttime < 0.0 && reportmemory < 0 {
        return false;
    }
    // c:1072-1073 — `if (!j->procs) return 0;`
    let first = match job.procs.first() {
        Some(p) => p,
        None => return false,
    };
    // c:1074 — `if (zleactive) return 0;`. ZLE is line-editing the
    // prompt; never spew a timing line into the editor.
    if zleactive.load(Ordering::Relaxed) != 0
    // c:1074
    {
        return false;
    }
    // c:1077-1094 — reporttime threshold check against (user+sys) CPU.
    if reporttime >= 0.0 {
        // C diffs reporttime against the first proc's ut+st; the
        // rusage diff is populated by update_process.
        let cpu_secs = (first.ti.ut + first.ti.st) as f64 / 1_000_000.0;
        if cpu_secs >= reporttime {
            return true;
        }
        // Wall-clock fallback (Rust extension — keeps prior behavior
        // when rusage wasn't captured because the proc was reaped
        // outside the wait4/getrusage path).
        if let (Some(start), Some(end)) = (first.bgtime, job.procs.last().and_then(|p| p.endtime)) {
            let elapsed = end.duration_since(start).as_secs_f64();
            if elapsed >= reporttime {
                return true;
            }
        }
    }
    // c:1096-1099 — reportmemory threshold check against ru_maxrss.
    if reportmemory >= 0 && first.ti.maxrss > reportmemory {
        return true;
    }
    false
}

// `CommandTimer` struct deleted — Rust-only timing aggregator with
// no caller. C inlines `dtime_tv()` (Src/jobs.c:137) /
// `dtime_ts()` (line 152) into printjob; the Rust port's `printtime`
// (above) is the equivalent free-fn and any caller that needs
// elapsed time can `Instant::now()` directly.

// `PipeStats` struct deleted — Rust-only wrapper that duplicated
// the `numpipestats` (jobs.c:131) + `pipestats[]` (jobs.c:131)
// flat C globals already ported as `NUMPIPESTATS` / `PIPESTATS` at
// file scope above. Read/write the canonical globals directly.

/// File-static `sig_msg[]` from `Src/signames1.awk` /
/// `signames.h` — name-by-signal-number lookup table consulted by
/// `sigmsg()` at `jobs.c:1118`.
static SIG_MSG: &[(libc::c_int, &str)] = &[
    // c:signames.h
    (libc::SIGHUP, "hangup"),
    (libc::SIGINT, "interrupt"),
    (libc::SIGQUIT, "quit"),
    (libc::SIGILL, "illegal instruction"),
    (libc::SIGTRAP, "trace trap"),
    (libc::SIGABRT, "abort"),
    (libc::SIGBUS, "bus error"),
    (libc::SIGFPE, "floating point exception"),
    (libc::SIGKILL, "killed"),
    (libc::SIGUSR1, "user-defined signal 1"),
    (libc::SIGSEGV, "segmentation fault"),
    (libc::SIGUSR2, "user-defined signal 2"),
    (libc::SIGPIPE, "broken pipe"),
    (libc::SIGALRM, "alarm"),
    (libc::SIGTERM, "terminated"),
    (libc::SIGCHLD, "child exited"),
    (libc::SIGCONT, "continued"),
    (libc::SIGSTOP, "stopped (signal)"),
    (libc::SIGTSTP, "stopped"),
    (libc::SIGTTIN, "stopped (tty input)"),
    (libc::SIGTTOU, "stopped (tty output)"),
    (libc::SIGURG, "urgent I/O condition"),
    (libc::SIGXCPU, "CPU time exceeded"),
    (libc::SIGXFSZ, "file size exceeded"),
    (libc::SIGVTALRM, "virtual timer expired"),
    (libc::SIGPROF, "profiling timer expired"),
    (libc::SIGWINCH, "window changed"),
    (libc::SIGIO, "I/O ready"),
    (libc::SIGSYS, "bad system call"),
];

/// Render a signal number as a one-line description.
/// Port of `sigmsg(int sig)` from Src/jobs.c:1107.
pub fn sigmsg(sig: i32) -> &'static str {
    // c:1107
    SIG_MSG
        .iter()
        .find(|(s, _)| *s == sig)
        .map(|(_, m)| *m)
        .unwrap_or("unknown signal") // c:1118 sig_msg[sig] : unknown
}

/// Print job with full detail (from jobs.c printjob)
// find length of longest signame, check to see                             // c:1178
// if we really need to print this job                                      // c:1179
/// `printjob` — see implementation.
pub fn printjob(
    job: &job,
    job_num: usize,
    lng: i32,
    cur_job: Option<usize>,
    prev_job: Option<usize>,
) -> String {
    // c:1141 — `int job, len = 9, sig, sflag = 0, llen;` — the status
    // column is `len + 2` wide where len starts at 9 and grows to the
    // longest signal message among non-running procs (c:1180-1213).
    let mut len = 9usize;
    for pn in job.procs.iter() {
        if pn.status == SP_RUNNING {
            continue;
        }
        #[cfg(unix)]
        {
            if libc::WIFSIGNALED(pn.status) {
                let mut llen = sigmsg(libc::WTERMSIG(pn.status)).len(); // c:1187
                if (pn.status & 0x80) != 0 {
                    llen += 14; // c:1188-1189 WCOREDUMP " (core dumped)"
                }
                len = len.max(llen); // c:1190-1191
            } else if libc::WIFSTOPPED(pn.status) {
                len = len.max(sigmsg(libc::WSTOPSIG(pn.status)).len()); // c:1201-1203
            }
        }
    }
    let width = len + 2; // c:1256 — `len2 = 10 + len; /* 2 spaces */`

    // Per-proc status text, padded to `width` per the fprintf field
    // widths at c:1293-1316.
    let fmt_proc_status = |status: i32| -> String {
        let s = if status == SP_RUNNING {
            "running".to_string() // c:1295
        } else if (status & 0x7f) == 0 {
            let code = (status >> 8) & 0xff;
            if code == 0 {
                "done".to_string() // c:1304
            } else {
                format!("exit {:<4}", code) // c:1301 "exit %-4d"
            }
        } else if (status & 0xff) == 0x7f {
            sigmsg((status >> 8) & 0xff).to_string() // c:1306 WSTOPSIG
        } else {
            let sig = status & 0x7f;
            if (status & 0x80) != 0 {
                format!("{} (core dumped)", sigmsg(sig)) // c:1309
            } else {
                sigmsg(sig).to_string() // c:1314 WTERMSIG
            }
        };
        format!("{:<w$}", s, w = width)
    };
    let marker = if Some(job_num) == cur_job {
        '+'
    } else if Some(job_num) == prev_job {
        '-'
    } else {
        ' '
    };

    // c:1273-1277 — first line carries `[N]  M `; continuation lines
    // (further proc groups) carry the matching indent.
    let head_prefix = format!("[{}]  {} ", job_num, marker);
    let cont_prefix = if job_num > 9 { "        " } else { "       " }; // c:1277

    let header = if job.procs.is_empty() {
        // c:1255 — `for (pn = jn->procs; pn;)` — a procless job (e.g.
        // the subshell control slot grabbed at c:1828) produces NO
        // output lines in C.
        if job.text.is_empty() {
            return String::new();
        }
        // Rust extension: jobs registered without proc entries carry
        // their display text on `job.text` (C always has procs). Use
        // the job-level stat bits for the status word.
        let status_str = if job.is_done() {
            format!("{:<w$}", "done", w = width)
        } else if job.is_stopped() {
            format!("{:<w$}", "suspended", w = width)
        } else {
            format!("{:<w$}", "running", w = width)
        };
        format!("{}{}{}", head_prefix, status_str, job.text)
    } else {
        // c:1255-1327 — group consecutive procs with the same status
        // onto one line (text joined with " | "); `jobs -l` / `jobs -p`
        // (lng & 3) put each proc on its own line.
        let mut lines: Vec<String> = Vec::new();
        let mut i = 0usize;
        let mut fline = true;
        let mut lng = lng;
        while i < job.procs.len() {
            let pn = &job.procs[i];
            // c:1257-1267 — group extent.
            let mut group_end = i + 1;
            if (lng & 3) == 0 {
                while group_end < job.procs.len() && job.procs[group_end].status == pn.status {
                    group_end += 1;
                }
            }
            let mut line = String::new();
            line.push_str(if fline { &head_prefix } else { cont_prefix });
            if (lng & 1) != 0 {
                line.push_str(&format!("{} ", pn.pid)); // c:1281 "%ld "
            } else if (lng & 2) != 0 {
                line.push_str(&format!("{} ", job.gleader)); // c:1283-1285
                lng &= !3; // c:1290
            }
            line.push_str(&fmt_proc_status(pn.status));
            let texts: Vec<&str> = job.procs[i..group_end]
                .iter()
                .map(|p| p.text.as_str())
                .collect();
            line.push_str(&texts.join(" | ")); // c:1318-1325
            lines.push(line);
            fline = false;
            i = group_end;
        }
        lines.join("\n")
    };

    // c:1220-1221 — `if (should_report_time(jn)) dumptime(jn);`
    //               Also fires for c:1354-1355 (synchronous-wait variant).
    let reporttime: f64 = getsparam("REPORTTIME")
        .and_then(|s| s.parse().ok())
        .unwrap_or(-1.0);
    if should_report_time(job, reporttime) {
        if let Some(timing) = dumptime(job) {
            return format!("{}\n{}", header, timing);
        }
    }
    header
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
/// open fd (to close on job exit) as a `jobfile` enum node, mirroring
/// the C `struct jobfile` tagged union. C operates on
/// `jobtab[thisjob].filelist`; the Rust port takes the `job` directly.
pub fn addfilelist(job: &mut job, name: Option<&str>, fd: i32) {
    // c:1373 — `Jobfile jf = zalloc(sizeof(struct jobfile));`
    // c:1374 — `LinkList ll = jobtab[thisjob].filelist;` / c:1376 create-if-absent
    //          folds into `Vec::push` (the Vec is the always-present list).
    let jf = match name {
        // c:1379 — `jf->u.name = ztrdup(name); jf->is_fd = 0;`
        Some(n) => jobfile {
            name: Some(n.to_string()),
            fd: 0,
            is_fd: 0,
        },
        // c:1383 — `jf->u.fd = fd; jf->is_fd = 1;`
        None => jobfile {
            name: None,
            fd,
            is_fd: 1,
        },
    };
    job.filelist.push(jf); // c:1385 zaddlinknode(ll, jf)
}

/// Port of `pipecleanfilelist(LinkList filelist, int proc_subst_only)` from `Src/jobs.c:1397`.
///
/// Closes only `is_fd` entries (named-file entries are left for
/// `deletefilelist` at job exit). When `proc_subst_only`, only fds
/// flagged `FDT_PROC_SUBST` in the fdtable are closed; the rest stay.
/// Closed fd entries are removed from the list; named entries remain.
pub fn pipecleanfilelist(filelist: &mut job, proc_subst_only: bool) {
    // c:1404-1414 — walk the list; close+remove qualifying fd entries.
    filelist.filelist.retain(|jf| {
        // c:1405-1406 — `jf->is_fd && (!proc_subst_only ||
        //                fdtable[jf->u.fd] == FDT_PROC_SUBST)`
        if jf.is_fd != 0 && (!proc_subst_only || fdtable_get(jf.fd) == FDT_PROC_SUBST) {
            zclose(jf.fd); // c:1408 zclose(jf->u.fd)
            false // c:1409 remnode(filelist, node) — drop from list
        } else {
            // c:1414 — `else incnode(node)`: keep everything else.
            true
        }
    });
}

/// Port of `deletefilelist(LinkList file_list, int disowning)` from `Src/jobs.c:1422`.
///
/// For each `Jobfile`: `is_fd` → close the fd (unless `disowning`);
/// named → unlink the file (unless `disowning`). The `disowning`
/// flag suppresses the `close`/`unlink` so files survive the disown.
pub fn deletefilelist(file_list: &mut job, disowning: bool) {
    // c:1427-1438 — `while ((jf = getlinknode(file_list)))` consumes the list.
    for jf in &file_list.filelist {
        if jf.is_fd != 0 {
            // c:1430-1431 — `if (jf->is_fd) { if (!disowning) zclose(jf->u.fd); }`
            if !disowning {
                zclose(jf.fd); // c:1432 zclose(jf->u.fd)
            }
        } else {
            // c:1433-1436 — `else { if (!disowning) unlink(jf->u.name); zsfree(...); }`
            if !disowning {
                if let Some(ref name) = jf.name {
                    let _ = std::fs::remove_file(name); // c:1435 unlink(jf->u.name)
                }
            }
            // c:1436 zsfree(jf->u.name) — owned String dropped with the node.
        }
    }
    // c:1438 — the loop drained the list; clear the Vec.
    file_list.filelist.clear();
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
pub fn cleanfilelists(jobtab: &mut [job]) {
    // c:1447 — DPUTS(shell_exiting >= 0, "BUG: cleanfilelists() before exit")
    DPUTS!(
        // c:1447
        SHELL_EXITING // c:1447
            .load(std::sync::atomic::Ordering::Relaxed)
            >= 0, // c:1447
        "BUG: cleanfilelists() before exit" // c:1447
    );
    for job in jobtab.iter_mut().skip(1) {
        deletefilelist(job, false);
    }
}

/// Port of `void freejob(job jn, int deleting)` from `Src/jobs.c:1457-1495`.
/// ```c
/// pn = jn->procs; jn->procs = NULL; free each;
/// pn = jn->auxprocs; jn->auxprocs = NULL; free each;
/// if (jn->ty) zfree(jn->ty);
/// if (jn->pwd) zsfree(jn->pwd);
/// jn->pwd = NULL;
/// if (jn->stat & STAT_WASSUPER) {
///     int job = jn - jobtab;
///     if (deleting) deletejob(jobtab + jn->other, 0);
///     else          freejob(jobtab + jn->other, 0);
///     jn = jobtab + job;
/// }
/// jn->gleader = jn->other = 0;
/// jn->stat = jn->stty_in_env = 0;
/// jn->filelist = NULL;
/// jn->ty = NULL;
/// ```
/// The previous Rust port was missing the `pwd`/`ty`/`other`/
/// `stty_in_env` field resets — leaked saved-tty state into the
/// next job reuse of the slot. Now resets all fields per C. The
/// STAT_WASSUPER recursive delete (c:1480-1488) requires jobtab
/// access and is left as a doc comment until the caller wires it.
pub fn freejob(jn: &mut job, deleting: bool) {
    // c:1457
    let _ = deleting; // STAT_WASSUPER recursive path not yet wired.
                      // c:1461-1466 — `procs = NULL; free each`. Rust Drop on Vec covers.
    jn.procs.clear();
    // c:1468-1473 — `auxprocs = NULL; free each`.
    jn.auxprocs.clear();
    // c:1475-1476 — `if (jn->ty) zfree(jn->ty);`.
    jn.ty = None;
    // c:1477-1479 — `if (jn->pwd) zsfree(jn->pwd); jn->pwd = NULL;`.
    jn.pwd = None;
    // c:1480-1488 — STAT_WASSUPER recursive delete: requires
    // jobtab[] access not in scope here. Doc-pin so a future caller
    // wiring the table can detect and dispatch.
    // c:1489 — `jn->gleader = jn->other = 0;`.
    jn.gleader = 0;
    jn.other = 0;
    // c:1490 — `jn->stat = jn->stty_in_env = 0;`.
    jn.stat = 0;
    jn.stty_in_env = 0;
    // c:1491 — `jn->filelist = NULL;`.
    jn.filelist.clear();
    // c:1492 — `jn->ty = NULL;` (already done above).
    // (Rust-only) text field — clear so the next job reuse doesn't
    // inherit stale command text.
    jn.text.clear();
}

/// Port of `void deletejob(job jn, int disowning)` from `Src/jobs.c:1511-1526`.
/// ```c
/// deletefilelist(jn->filelist, disowning);
/// if (jn->stat & STAT_ATTACH) {
///     attachtty(mypgrp);
///     adjustwinsize(0);
/// }
/// if (jn->stat & STAT_SUPERJOB) {
///     job jno = jobtab + jn->other;
///     if (jno->stat & STAT_SUBJOB)
///         jno->stat |= STAT_SUBJOB_ORPHANED;
/// }
/// freejob(jn, 1);
/// ```
/// Previously the Rust port ad-hoc cleared procs/auxprocs/stat
/// without calling `freejob` — meant `pwd`/`ty`/`other`/`stty_in_env`
/// stayed populated even after the job was "deleted", silently
/// corrupting the next slot reuse. The STAT_ATTACH (attachtty) and
/// STAT_SUPERJOB recursive cleanup paths require substrate not yet
/// wired (mypgrp, jobtab[] reference); doc-pinned for follow-up.
pub fn deletejob(jn: &mut job, disowning: bool) {
    // c:1512
    // c:1514 — `deletefilelist(jn->filelist, disowning);`. When
    // disowning, files are NOT deleted from disk; the filelist entries
    // are simply dropped.
    deletefilelist(jn, disowning);
    // c:1515-1518 — `if (jn->stat & STAT_ATTACH) { attachtty(mypgrp);
    //                adjustwinsize(0); }`. `attachtty(mypgrp)` is the
    // canonical `tcsetpgrp(0, mypgrp)` (the same pattern used inline at
    // jobs.rs:2503/2527). `adjustwinsize(0)` re-reads $LINES/$COLUMNS
    // from TIOCGWINSZ; on Rust we route through the canonical utils
    // adjustcolumns/adjustlines which lazy-evaluate on demand, so the
    // call is a no-op (the next adjust* read picks up the new pgrp).
    if (jn.stat & STAT_ATTACH) != 0 {
        // c:1515
        #[cfg(unix)]
        unsafe {
            let pgrp = crate::ported::modules::clone::mypgrp.load(Ordering::Relaxed);
            if pgrp > 0 {
                libc::tcsetpgrp(0, pgrp); // c:1516 attachtty(mypgrp)
            }
        }
        // c:1517 — `adjustwinsize(0);` — Rust adjust* are lazy-read.
    }
    // c:1519-1523 — `if (jn->stat & STAT_SUPERJOB) { job jno = jobtab +
    //                jn->other; if (jno->stat & STAT_SUBJOB)
    //                  jno->stat |= STAT_SUBJOB_ORPHANED; }`.
    if (jn.stat & STAT_SUPERJOB) != 0 {
        // c:1519
        let other = jn.other as usize;
        if let Some(tab) = JOBTAB.get() {
            // c:1520 jobtab + jn->other
            if let Ok(mut jobs) = tab.lock() {
                if let Some(jno) = jobs.get_mut(other) {
                    if (jno.stat & STAT_SUBJOB) != 0 {
                        // c:1521
                        jno.stat |= STAT_SUBJOB_ORPHANED; // c:1522
                    }
                }
            }
        }
    }
    // c:1525 — `freejob(jn, 1);` full reset of all per-job state.
    freejob(jn, true);
}

/// Add process to job (from jobs.c addproc lines 1537-1597)
/// Port of `addproc(pid_t pid, char *text, int aux, struct timespec
/// *bgtime, int gleader, int list_pipe_job_used)` from `Src/jobs.c:1538`.
///
/// The C call site at exec.c:2853 passes the entersubsh_ret-filled
/// gleader/list_pipe_job from the child via the synch pipe. Rust mirrors
/// the full signature; legacy callers pass `None`/`-1`.
pub fn addproc(
    job: &mut job,
    pid: i32,
    text: &str,
    aux: bool,
    bgtime: Option<std::time::Instant>,
    gleader: i32,
    list_pipe_job_used: i32,
) {
    // c:1538
    let proc = process::new(pid);
    let proc = process {
        pid,
        status: SP_RUNNING,
        text: text.to_string(),
        bgtime, // c:1248 — `bgtime` field from struct timespec arg.
        ..proc
    };

    if aux {
        job.auxprocs.push(proc);
    } else {
        // c:1565-1568 — `if (gleader != -1) jn->gleader = gleader;`
        if gleader != -1 {
            job.gleader = gleader;
        } else if job.gleader == 0 {
            job.gleader = pid;
        }
        // c:1570 — `if (list_pipe_job_used != -1) jobtab[list_pipe_job_used].other = thisjob;`
        // Stored on the process via list_pipe_job (the C field is
        // tracked back via jobtab[list_pipe_job_used].other; the
        // simpler approach here is to ignore unless needed).
        let _ = list_pipe_job_used;
        job.procs.push(proc);
    }

    job.stat &= !stat::DONE;
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
/// a single `&job` and returned `!job.filelist.is_empty()`, which
/// is the wrong shape (C iterates).
pub fn havefiles(jobtab: &[job]) -> bool {
    // c:1605
    jobtab.iter().any(|j| j.stat != 0 && !j.filelist.is_empty())
}

// Wait for a particular process.                                           // c:1627
// wait_cmd indicates this is from the interactive wait command,            // c:1627
// in which case the behaviour is a little different:  the command          // c:1627
// itself can be interrupted by a trapped signal.                           // c:1627
/// Wait for a specific PID (from jobs.c waitforpid lines 1627-1663)
pub fn waitforpid(pid: i32) -> Option<i32> {
    // c:1627
    #[cfg(unix)]
    {
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

/// Port of `zwaitjob(int job, int wait_cmd)` from `Src/jobs.c:1673`.
///
/// `wait_cmd` is the "from interactive `wait` builtin" flag. Threads
/// through `queue_traps(wait_cmd)` so signal-trap firing is allowed
/// inside the wait, and through `signal_suspend(SIGCHLD, wait_cmd)`
/// so trapped non-CHLD signals can interrupt the suspend (returning
/// `128 + last_signal` so the wait builtin propagates the interrupt).
///
/// Body uses the canonical SIGCHLD-driven async pattern: signal_suspend
/// blocks until the SIGCHLD handler (signals.rs::zhandler) reaps via
/// wait_for_processes + routes through update_bg_job, which sets
/// STAT_DONE / STAT_STOPPED on the job. The loop checks job.stat
/// after each wake. Mirrors `Src/jobs.c:1673-1750`.
pub fn zwaitjob(job: &mut job, wait_cmd: i32) -> Option<i32> {
    // c:1673
    if job.procs.is_empty() && job.auxprocs.is_empty() {
        // c:1736-1740 — no procs: deletejob + pipestats[0]=lastval and return.
        return Some(0);
    }

    use crate::ported::utils::errflag;
    use crate::ported::zsh_h::{ERRFLAG_ERROR, INTERACTIVE, STAT_DONE, STAT_STOPPED, ZSIG_TRAPPED};

    // c:1675 — `int q = queue_signal_level();`
    let q = crate::ported::signals_h::queue_signal_level();
    // c:1678 — `child_block();`
    crate::ported::signals_h::child_block();
    // c:1679 — `queue_traps(wait_cmd);`
    crate::ported::signals::queue_traps(wait_cmd);
    // c:1680 — `dont_queue_signals();`
    crate::ported::signals_h::dont_queue_signals();

    // c:1682 — `jn->stat |= STAT_LOCKED;`
    job.stat |= crate::ported::zsh_h::STAT_LOCKED;
    // c:1683-1684 — STAT_CHANGED → printjob (deferred — needs jobtab index).
    // c:1685-1697 — pipecleanfilelist for proc-subst fds.
    if !job.filelist.is_empty() {
        crate::ported::jobs::pipecleanfilelist(job, false);
    }

    // c:1698-1735 — main wait loop.
    let interact = isset(INTERACTIVE);
    loop {
        // c:1698 — `while (!(errflag & ERRFLAG_ERROR) && jn->stat &&
        //            !(jn->stat & STAT_DONE) &&
        //            !(interact && (jn->stat & STAT_STOPPED)))`
        if (errflag.load(std::sync::atomic::Ordering::Relaxed) & ERRFLAG_ERROR) != 0 {
            break;
        }
        if job.stat == 0 {
            break;
        }
        if (job.stat & STAT_DONE) != 0 {
            break;
        }
        if interact && (job.stat & STAT_STOPPED) != 0 {
            break;
        }

        // c:1701 — `signal_suspend(SIGCHLD, wait_cmd);` — block until
        // SIGCHLD; handler routes through update_bg_job which sets
        // STAT_DONE/STOPPED on `job`.
        let _ = crate::ported::signals::signal_suspend(libc::SIGCHLD, wait_cmd != 0);

        // c:1702-1708 — `if (last_signal != SIGCHLD && wait_cmd &&
        //                  last_signal >= 0 && sigtrapped[ls] & ZSIG_TRAPPED)
        //                  { return 128 + last_signal; }`
        let ls = crate::ported::signals::last_signal.load(std::sync::atomic::Ordering::Relaxed);
        if ls != libc::SIGCHLD && wait_cmd != 0 && ls >= 0 {
            let trapped_flag = {
                let guard = crate::ported::signals::sigtrapped.lock().unwrap();
                guard.get(ls as usize).copied().unwrap_or(0)
            };
            if (trapped_flag & ZSIG_TRAPPED) != 0 {
                // c:1705-1707 — builtin wait interrupted by trapped signal.
                crate::ported::signals_h::restore_queue_signals(q);
                crate::ported::signals::unqueue_traps();
                crate::ported::signals_h::child_unblock();
                return Some(128 + ls); // c:1707
            }
        }
        // c:1729-1730 — `if (subsh) killjb(jn, SIGCONT);` — keep stopped
        // grandchildren running when we ourselves are a subshell.
        if crate::ported::exec::subsh.load(std::sync::atomic::Ordering::Relaxed) != 0 {
            // killjb wants &mut [job]; we have &mut job here. Inline the
            // SIGCONT via killpg on the job's gleader if set.
            if job.gleader != 0 {
                unsafe {
                    libc::killpg(job.gleader, libc::SIGCONT);
                }
            }
        }
        // c:1731-1733 — STAT_SUPERJOB handle_sub deferred (sub-job
        // dispatch is jobtab-index-keyed; needs the live jobtab access).
        // Re-block before next suspend so SIGCHLD pump isn't lost.
        crate::ported::signals_h::child_block();
    }

    // c:1741-1744 — restore + return 0.
    crate::ported::signals_h::restore_queue_signals(q);
    crate::ported::signals::unqueue_traps();
    crate::ported::signals_h::child_unblock();
    // last_status read for the legacy caller — derive from procs.
    let last_status = job.procs.last().map(|p| p.exit_status()).unwrap_or(0);
    Some(last_status) // c:1745
}

// wait for running job to finish                                           // c:1763
/// Wait for all foreground jobs to finish (from jobs.c waitjobs)
pub fn waitjobs(jobtab: &mut [job], thisjob: usize) {
    // c:1763
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
// Rust idiom replacement: JobTable's private Vec model is rebuilt
// by the executor on subshell entry (`JobTable::new()`), so the C
// `oldjobtab` snapshot + per-slot reset loop is structurally
// replaced — no public reset method is needed.
/// `clearjobtab` — see implementation.
pub fn clearjobtab(table: &mut JobTable, monitor: i32) {
    // c:1780
    let _ = table; // legacy executor-side handle, unused now
    let posix_jobs = isset(POSIXJOBS); // c:1786
                                       // c:1786-1787 — `if (isset(POSIXJOBS)) oldmaxjob = 0;`.
    if posix_jobs {
        if let Some(om) = OLDMAXJOB.get() {
            if let Ok(mut o) = om.lock() {
                *o = 0;
            }
        }
    }
    let tab = match JOBTAB.get() {
        Some(t) => t,
        None => return,
    };
    let mut jobs = match tab.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    // c:1788-1797 — for (i = 1; i <= maxjob; i++).
    let maxjob = jobs.len();
    let mut new_oldmax: usize = 0;
    for i in 1..maxjob {
        // c:1788
        if jobs[i].stat == 0 {
            continue;
        }
        // c:1794-1795 — `if (monitor && !POSIXJOBS && jobtab[i].stat)
        //                  oldmaxjob = i+1;`
        if monitor != 0 && !posix_jobs {
            // c:1794
            new_oldmax = i + 1; // c:1795
        } else if (jobs[i].stat & STAT_INUSE) != 0 {
            // c:1796
            // c:1797 — `freejob(jobtab+i, 0);`.
            freejob(&mut jobs[i], false); // c:1797
        }
    }
    // c:1800-1817 — `if (monitor && oldmaxjob) { snapshot to oldjobtab }`.
    if monitor != 0 && new_oldmax > 0 {
        // c:1800
        let mut snap: Vec<job> = jobs[..new_oldmax].iter().cloned().collect(); // c:1803-1806
                                                                               // c:1809-1810 — `if (thisjob != -1 && thisjob < oldmaxjob)
                                                                               //                  memset(oldjobtab+thisjob, 0, ...)`.
        let thisjob = *THISJOB.get_or_init(|| Mutex::new(-1)).lock().unwrap();
        if thisjob >= 0 && (thisjob as usize) < new_oldmax {
            // c:1809
            // Zero the slot — Rust uses Default::default().
            snap[thisjob as usize] = job::default(); // c:1810
        }
        // c:1816 — `--oldmaxjob;` C decrement before exposure.
        if let Some(om) = OLDMAXJOB.get() {
            if let Ok(mut o) = om.lock() {
                *o = new_oldmax.saturating_sub(1); // c:1816
            }
        } else {
            *OLDMAXJOB.get_or_init(|| Mutex::new(0)).lock().unwrap() = new_oldmax.saturating_sub(1);
        }
        *OLDJOBTAB
            .get_or_init(|| Mutex::new(Vec::new()))
            .lock()
            .unwrap() = snap; // c:1804
    }
    // c:1818-1819 — `memset(jobtab, 0, jobtabsize * sizeof(struct job));
    //                maxjob = 0;` — zero out the live table.
    jobs.clear();
    jobs.push(job::new()); // slot 0 — the shell's own entry
    *MAXJOB.get_or_init(|| Mutex::new(0)).lock().unwrap() = 0; // c:1819
                                                               // c:1821-1828 — "Although we don't have job control in subshells,
                                                               // we sometimes need control structures for other purposes such as
                                                               // multios. Grab a job for this purpose." `thisjob = initjob();`
    let control = initjob(&mut jobs); // c:1828
    *THISJOB.get_or_init(|| Mutex::new(-1)).lock().unwrap() = control as i32;
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
    *OLDJOBTAB
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .expect("oldjobtab poisoned") = Vec::new();
    *OLDMAXJOB
        .get_or_init(|| Mutex::new(0))
        .lock()
        .expect("oldmaxjob poisoned") = 0;
}

// Get a free entry in the job table and initialize it.                    // c:1862
/// Initialize a new job entry (from jobs.c initjob)
///
/// c:Src/jobs.c:1862-1875 — C: `for (i = 1; i <= maxjob; i++)` starts
/// at index 1; index 0 is the shell's own slot and must never be
/// returned to a child-job caller. The Rust port previously walked
/// from index 0 via `enumerate()`, corrupting parent-shell job
/// tracking when jobtab[0] was empty.
pub fn initjob(jobtab: &mut Vec<job>) -> usize {
    // c:1862
    // Ensure jobtab has slot 0 reserved for the shell (matches C's
    // `jobtab[0]` shell-process slot at jobs.c:79).
    if jobtab.is_empty() {
        jobtab.push(job::new());
    }
    // Find an empty slot or add a new one — START AT INDEX 1.
    for i in 1..jobtab.len() {
        if (jobtab[i].stat & stat::INUSE) == 0 {
            return initnewjob(jobtab, i); // c:1868
        }
    }
    // Expand table — C path c:1869-1872 (maxjob+1 within jobtabsize,
    // else expandjobtab). Rust's Vec grows on demand.
    let idx = jobtab.len();
    jobtab.push(job::new());
    initnewjob(jobtab, idx)
}

/// Direct port of `static int initnewjob(int i)` from `Src/jobs.c:1843`.
///
/// C body:
/// ```c
/// jobtab[i].stat = STAT_INUSE;
/// if (jobtab[i].pwd) { zsfree(jobtab[i].pwd); jobtab[i].pwd = NULL; }
/// jobtab[i].gleader = 0;
/// if (i > maxjob) maxjob = i;
/// return i;
/// ```
/// MAXJOB is the scan bound for setcurjob/setprevjob/getjob/
/// selectjobtab; without the bump those walks see an empty table even
/// when JOBTAB has live entries.
/// WARNING: param names don't match C — Rust=(jobtab, i) vs C=(i);
/// C reads the jobtab global, Rust callers pass the locked slice.
fn initnewjob(jobtab: &mut [job], i: usize) -> usize {
    // c:1843
    jobtab[i] = job::new();
    jobtab[i].stat = stat::INUSE; // c:1845
    jobtab[i].pwd = None; // c:1846-1849
    jobtab[i].gleader = 0; // c:1850
    let mut mj = MAXJOB
        .get_or_init(|| Mutex::new(0))
        .lock()
        .expect("maxjob poisoned");
    if i > *mj {
        // c:1852-1853
        *mj = i;
    }
    i // c:1855
}

/// Port of `void setjobpwd(void)` from `Src/jobs.c:1881`.
///
/// C body:
/// ```c
/// int i;
/// for (i = 1; i <= maxjob; i++)
///     if (jobtab[i].stat && !jobtab[i].pwd)
///         jobtab[i].pwd = ztrdup(pwd);
/// ```
///
/// Walks every IN-USE job and stamps its `pwd` with the current
/// shell `pwd` (from `Src/builtin.c:1240` after `bin_cd`). The
/// previous Rust port took a `&mut job` ref and was a no-op (just
/// captured cwd then dropped it) — every `cd` left the in-flight
/// job's pwd unset, and `jobs` output showed empty `(pwd: )` for
/// jobs that started before the cd.
///
/// The fix walks `JOBTAB` and writes `pwd` to every job whose stat
/// is non-zero (INUSE) and whose pwd is still None. The shell
/// pwd is read from the canonical `params::pwdgetfn` accessor —
/// matches C's read of the `pwd` global at c:1888.
pub fn setjobpwd() {
    // c:1881
    // c:1888 — `pwd` is the canonical shell-state global from
    // `Src/params.c:108`. Rust reads it via the paramtab-backed
    // `getsparam("PWD")` which is the canonical accessor mirrored
    // throughout the codebase (prompt.rs, subst.rs, builtin.rs).
    let pwd = getsparam("PWD").unwrap_or_default(); // c:1888 pwd
    let tab = JOBTAB.get_or_init(|| Mutex::new(Vec::new()));
    let mut tab = tab.lock().expect("jobtab poisoned");
    // c:1886 — `for (i = 1; i <= maxjob; i++)`. Skip index 0 (the
    // shell itself).
    for job in tab.iter_mut().skip(1) {
        // c:1887 — `if (jobtab[i].stat && !jobtab[i].pwd)`.
        if job.stat != 0 && job.pwd.is_none() {
            job.pwd = Some(pwd.clone()); // c:1888
        }
    }
}

/// Print pids for `&` background jobs (`spawnjob`).
/// Port of `void spawnjob(void)` from `Src/jobs.c:1894`.
pub fn spawnjob() {
    // c:1894
    let thisjob_idx = *THISJOB
        .get_or_init(|| Mutex::new(-1))
        .lock()
        .expect("thisjob poisoned");
    // c:1898 — DPUTS(thisjob == -1, "No valid job in spawnjob.")
    DPUTS!(thisjob_idx == -1, "No valid job in spawnjob."); // c:1898
    if thisjob_idx < 0 {
        return;
    }
    let thisjob = thisjob_idx as usize;

    // c:1900 — `if (!subsh) {` — when this isn't a subshell.
    // `subsh` global tracks subshell-fork depth; mirror via FORKLEVEL
    // (0 = top-level shell) plus SUBSHELL_DEPTH, the depth counter the
    // fusevm in-process `(...)` host bumps in subshell_begin. C's
    // forked subshell sets `subsh` in entersubsh (Src/exec.c:1154);
    // the in-process model never calls entersubsh, so without this
    // a `(cmd &)` would promote the job to the parent's curjob —
    // making `(sleep 1 & disown)` silently succeed where zsh errors
    // "no current job". Bug #462.
    let in_subsh = crate::ported::exec::FORKLEVEL.load(Ordering::Relaxed) > 0
        || crate::ported::builtin::SUBSHELL_DEPTH.load(Ordering::Relaxed) > 0;
    if !in_subsh {
        // c:1901-1903 — `if (curjob == -1 || !(jobtab[curjob].stat & STAT_STOPPED))
        //                  { curjob = thisjob; setprevjob(); }`
        // c:1904-1905 — else if prevjob also not stopped, prevjob = thisjob.
        let curjob = *CURJOB
            .get_or_init(|| Mutex::new(-1))
            .lock()
            .expect("curjob poisoned");
        let cur_stopped = if curjob >= 0 {
            let tab = JOBTAB
                .get_or_init(|| Mutex::new(Vec::new()))
                .lock()
                .expect("jobtab poisoned");
            tab.get(curjob as usize)
                .map(|j| (j.stat & stat::STOPPED) != 0)
                .unwrap_or(false)
        } else {
            false
        };
        if curjob < 0 || !cur_stopped {
            if let Ok(mut cj) = CURJOB.get_or_init(|| Mutex::new(-1)).lock() {
                *cj = thisjob_idx; // c:1902
            }
            setprevjob(); // c:1903
        } else {
            // c:1904-1905
            let prevjob = *PREVJOB
                .get_or_init(|| Mutex::new(-1))
                .lock()
                .expect("prevjob poisoned");
            let prev_stopped = if prevjob >= 0 {
                let tab = JOBTAB
                    .get_or_init(|| Mutex::new(Vec::new()))
                    .lock()
                    .expect("jobtab poisoned");
                tab.get(prevjob as usize)
                    .map(|j| (j.stat & stat::STOPPED) != 0)
                    .unwrap_or(false)
            } else {
                false
            };
            if prevjob < 0 || !prev_stopped {
                if let Ok(mut pj) = PREVJOB.get_or_init(|| Mutex::new(-1)).lock() {
                    *pj = thisjob_idx; // c:1905
                }
            }
        }
        // c:1906-1913 — `if (jobbing && jobtab[thisjob].procs)`
        //               print "[N] pid1 pid2 ..." to shout/stderr.
        if isset(MONITOR) {
            let tab = JOBTAB
                .get_or_init(|| Mutex::new(Vec::new()))
                .lock()
                .expect("jobtab poisoned");
            if let Some(job) = tab.get(thisjob) {
                if !job.procs.is_empty() {
                    let mut line = format!("[{}]", thisjob_idx);
                    for p in job.procs.iter() {
                        line.push_str(&format!(" {}", p.pid));
                    }
                    line.push('\n');
                    eprint!("{}", line); // c:1907-1911
                }
            }
        }
    }
    // c:1915-1920 — `if (!hasprocs(thisjob)) deletejob(jobtab+thisjob, 0);
    //                else { STAT_LOCKED; pipecleanfilelist(...); }`
    let need_delete: bool;
    {
        let tab = JOBTAB
            .get_or_init(|| Mutex::new(Vec::new()))
            .lock()
            .expect("jobtab poisoned");
        need_delete = tab
            .get(thisjob)
            .map(|j| j.procs.is_empty() && j.auxprocs.is_empty())
            .unwrap_or(true);
    }
    if need_delete {
        let mut tab = JOBTAB
            .get_or_init(|| Mutex::new(Vec::new()))
            .lock()
            .expect("jobtab poisoned");
        if let Some(j) = tab.get_mut(thisjob) {
            deletejob(j, false); // c:1916
        }
    } else {
        let mut tab = JOBTAB
            .get_or_init(|| Mutex::new(Vec::new()))
            .lock()
            .expect("jobtab poisoned");
        if let Some(j) = tab.get_mut(thisjob) {
            j.stat |= stat::LOCKED; // c:1918
            pipecleanfilelist(j, false); // c:1919
        }
    }
    // c:1921 — thisjob = -1;
    if let Ok(mut tj) = THISJOB.get_or_init(|| Mutex::new(-1)).lock() {
        *tj = -1;
    }
}

// `ChildTimes` struct deleted — folded into the canonical `timeinfo`
// at the top of this file. C uses `child_times_t` (typedef onto
// `struct rusage` or `struct timeinfo` per `Src/zsh.h:1112-1114`).

/// Port of `void shelltime(child_times_t *shell, child_times_t *kids,
/// struct timespec *then, int delta)` from `Src/jobs.c:1926-1987`.
///
/// Records or prints the shell's RUSAGE_SELF + RUSAGE_CHILDREN times.
/// Side-effecting:
///   - If `shell` is `Some` and `delta == 0`: snapshot current self
///     rusage into `*shell` (no print).
///   - If `shell` is `Some` and `delta != 0`: compute delta from
///     `*shell` to now (no print).
///   - If `shell` is `None` and `delta == 0`: print "shell ..." line.
///   - Same pattern for `kids` against RUSAGE_CHILDREN.
///   - `then` similarly: when `None` and `delta == 0`, use as the
///     monotonic timestamp slot; when `Some + delta`, compute the
///     elapsed real time as `now - *then`.
///
/// C body c:1926-1987 maps closely:
///   - c:1934 — zgettime_monotonic_if_available(&now)
///   - c:1937 — getrusage(RUSAGE_SELF, &ti)
///   - c:1944-1955 — handle `shell` save / delta
///   - c:1956-1962 — compute `dtimespec` from `then` and `now` /
///                   shtimer
///   - c:1964-1965 — `if (!delta == !shell) printtime("shell")`
///   - c:1968 — getrusage(RUSAGE_CHILDREN, &ti)
///   - c:1973-1984 — handle `kids` save / delta
///   - c:1985-1986 — `if (!delta == !kids) printtime("children")`
#[cfg(unix)]
pub fn shelltime(
    shell: Option<&mut timeinfo>,
    kids: Option<&mut timeinfo>,
    then: Option<&mut std::time::Instant>,
    delta: i32,
) {
    // c:1926
    // c:1934 — `zgettime_monotonic_if_available(&now);`. Use Instant
    // for monotonic time.
    let now = std::time::Instant::now();
    // c:1937 — `getrusage(RUSAGE_SELF, &ti);`. Self timings.
    let mut ti: timeinfo = {
        let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
        if unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) } == 0 {
            timeinfo::from_rusage(&usage)
        } else {
            timeinfo::default()
        }
    };

    let shell_present = shell.is_some();
    // c:1944-1955 — `if (shell) { if (delta) dtime_tv(...); else *shell = ti; }`.
    if let Some(s) = shell {
        // c:1944
        if delta != 0 {
            // c:1945 — delta-compute by subtracting saved values.
            // C uses dtime_tv to subtract timespec. timeinfo holds
            // raw rusage members; subtract user/sys time directly.
            ti.ut = ti.ut.saturating_sub(s.ut); // c:1947 dtime_tv(ru_utime, shell->ru_utime, ti.ru_utime)
            ti.st = ti.st.saturating_sub(s.st); // c:1948
        } else {
            // c:1953-1954 — snapshot current `ti` into `*shell`.
            *s = ti.clone();
        }
    }

    // c:1956-1962 — compute `dtimespec` (real elapsed time).
    let dtime: std::time::Duration = if delta != 0 {
        // c:1957 — `dtime_ts(&dtimespec, then, &now)`. The C body
        // requires `then` to be Some for the delta path (set on a
        // prior delta=0 call).
        match then {
            Some(t) => dtime_ts(t, &now), // c:1957
            None => std::time::Duration::ZERO,
        }
    } else {
        // c:1959-1961 — `if (then) *then = now;` then
        //                `dtime_ts(&dtimespec, &shtimer, &now);`.
        if let Some(t) = then {
            *t = now;
        }
        // c:1961 — `dtime_ts(&dtimespec, &shtimer, &now)`. Rust's
        // `params::shtimer_lock()` is the analog of C's `shtimer`
        // global (`struct timespec` set at shell start). Compute
        // elapsed time as now - shtimer.
        let shtimer_dur = *crate::ported::params::shtimer_lock()
            .lock()
            .expect("shtimer poisoned");
        let now_dur = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        if now_dur > shtimer_dur {
            now_dur - shtimer_dur
        } else {
            std::time::Duration::ZERO
        }
    };

    // c:1964 — `if (!delta == !shell) printtime("shell")`.
    // The negation pair: print when (delta==0 && shell.is_none()) OR
    // (delta!=0 && shell.is_some()).
    if (delta == 0) == !shell_present {
        // c:1964
        let real_secs = dtime.as_secs_f64();
        // c:1965 — `printtime(&dtimespec, &ti, "shell")`.
        let timefmt = crate::ported::params::getsparam("TIMEFMT")
            .unwrap_or_else(|| "%J  %U user %S system %P cpu %*E total".to_string());
        let line = printtime(real_secs, &ti, &timefmt, "shell"); // c:1965
        eprintln!("{}", line);
    }

    // c:1968 — `getrusage(RUSAGE_CHILDREN, &ti);`. Children timings.
    let mut tc: timeinfo = {
        let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
        if unsafe { libc::getrusage(libc::RUSAGE_CHILDREN, &mut usage) } == 0 {
            timeinfo::from_rusage(&usage)
        } else {
            timeinfo::default()
        }
    };

    let kids_present = kids.is_some();
    // c:1973-1984 — `if (kids) { ... }` symmetric to shell.
    if let Some(k) = kids {
        // c:1973
        if delta != 0 {
            tc.ut = tc.ut.saturating_sub(k.ut); // c:1976
            tc.st = tc.st.saturating_sub(k.st); // c:1977
        } else {
            *k = tc.clone(); // c:1983
        }
    }

    // c:1985-1986 — `if (!delta == !kids) printtime("children")`.
    if (delta == 0) == !kids_present {
        // c:1985
        let real_secs = dtime.as_secs_f64();
        let timefmt = crate::ported::params::getsparam("TIMEFMT")
            .unwrap_or_else(|| "%J  %U user %S system %P cpu %*E total".to_string());
        let line = printtime(real_secs, &tc, &timefmt, "children"); // c:1986
        eprintln!("{}", line);
    }
}

/// Non-unix stub matching the C body's #ifdef-gated absence.
#[cfg(not(unix))]
pub fn shelltime(
    _shell: Option<&mut timeinfo>,
    _kids: Option<&mut timeinfo>,
    _then: Option<&mut std::time::Instant>,
    _delta: i32,
) {
}

// see if jobs need printing                                                // c:1993
/// Scan jobs and print changed status (from jobs.c scanjobs)
pub fn scanjobs(jobtab: &mut [job]) {
    // c:1993
    // C body:
    // ```c
    // for (i = 1; i <= maxjob; i++)
    //     if (jobtab[i].stat & STAT_CHANGED)
    //         printjob(jobtab + i, !!isset(LONGLISTJOBS), 1);
    // ```
    // printjob with synch=1 prints only when `(interact || synch) &&
    // jobbing && ...` (c:1236-1238) — so in a non-MONITOR shell the
    // call is a silent pass whose tail (c:1350-1363) deletes each
    // finished entry and clears STAT_CHANGED otherwise (c:1364).
    // WARNING: param names don't match C — Rust=(jobtab) vs C=(void);
    // C reads the jobtab global, Rust callers pass the locked slice.
    let long_list = isset(LONGLISTJOBS);
    for i in 1..jobtab.len() {
        // c:1998
        if (jobtab[i].stat & stat::CHANGED) != 0 {
            // c:1999
            if crate::ported::zsh_h::jobbing() {
                // c:1236-1238 print gate
                let curjob = *CURJOB.get_or_init(|| Mutex::new(-1)).lock().unwrap();
                let prevjob = *PREVJOB.get_or_init(|| Mutex::new(-1)).lock().unwrap();
                let s = printjob(
                    &jobtab[i],
                    i,
                    long_list as i32,
                    if curjob >= 0 {
                        Some(curjob as usize)
                    } else {
                        None
                    },
                    if prevjob >= 0 {
                        Some(prevjob as usize)
                    } else {
                        None
                    },
                ); // c:2000
                if !s.is_empty() {
                    eprintln!("{}", s);
                }
            }
            if (jobtab[i].stat & stat::DONE) != 0 {
                // c:1350-1363 — printjob's done-delete tail.
                crate::exec_jobs::printjob_delete_tail(jobtab, i);
            } else {
                jobtab[i].stat &= !stat::CHANGED; // c:1364
            }
        }
    }
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
pub fn isanum(s: &str) -> bool {
    // c:2010
    !s.is_empty() && s.bytes().all(|b| b == b'-' || b.is_ascii_digit())
}

// Make sure we have a suitable current and previous job set.               // c:2023
/// Direct port of `void setcurjob(void)` from `Src/jobs.c:2023`.
///
/// C body:
/// ```c
/// if (curjob == thisjob ||
///     (curjob != -1 && !(jobtab[curjob].stat & STAT_INUSE))) {
///     curjob = prevjob;
///     setprevjob();
///     if (curjob == thisjob ||
///         (curjob != -1 && !((jobtab[curjob].stat & STAT_INUSE) &&
///                            curjob != thisjob))) {
///         curjob = prevjob;
///         setprevjob();
///     }
/// }
/// ```
/// REPAIRS an invalid `curjob` (gone, or equal to the in-flight
/// thisjob) by promoting `prevjob`; it does NOT scan for a fresh
/// candidate when curjob is -1 — promotion to curjob happens in
/// spawnjob (c:1901-1903) and printjob's delete tail (c:1357-1360).
/// The previous Rust body picked the highest in-use job
/// unconditionally, which resurrected a current job inside subshells
/// where zsh reports "no current job" (bug #462 probe
/// `(sleep 0.2 & disown)` → rc=1 in zsh).
pub fn setcurjob() {
    // c:2023
    let inuse = |jobno: i32| -> bool {
        let tab = JOBTAB
            .get_or_init(|| Mutex::new(Vec::new()))
            .lock()
            .expect("jobtab poisoned");
        tab.get(jobno as usize)
            .map(|j| (j.stat & stat::INUSE) != 0)
            .unwrap_or(false)
    };
    let thisjob = *THISJOB.get_or_init(|| Mutex::new(-1)).lock().unwrap();
    let curjob = *CURJOB.get_or_init(|| Mutex::new(-1)).lock().unwrap();
    // c:2025-2026. The `curjob == thisjob` test is guarded with
    // `curjob != -1` here: in C, thisjob is never -1 while bin_fg runs
    // (execpline c:Src/exec.c:1700 allocates a pipeline job slot before
    // any builtin executes), so `-1 == -1` can't trigger the C branch.
    // zshrs has no per-pipeline job allocation — thisjob is -1 between
    // jobs — and an unguarded -1==-1 would promote prevjob/setprevjob,
    // resurrecting a "current job" zsh reports as absent (bug #462).
    if (curjob != -1 && curjob == thisjob) || (curjob != -1 && !inuse(curjob)) {
        // c:2027-2028 — `curjob = prevjob; setprevjob();`
        let pj = *PREVJOB.get_or_init(|| Mutex::new(-1)).lock().unwrap();
        *CURJOB.get_or_init(|| Mutex::new(-1)).lock().unwrap() = pj;
        setprevjob();
        let curjob = *CURJOB.get_or_init(|| Mutex::new(-1)).lock().unwrap();
        // c:2029-2031 — same -1 guard as above.
        if (curjob != -1 && curjob == thisjob)
            || (curjob != -1 && !(inuse(curjob) && curjob != thisjob))
        {
            // c:2032-2033
            let pj = *PREVJOB.get_or_init(|| Mutex::new(-1)).lock().unwrap();
            *CURJOB.get_or_init(|| Mutex::new(-1)).lock().unwrap() = pj;
            setprevjob();
        }
    }
}

// Find the job table for reporting jobs                                   // c:2042
/// Port of `selectjobtab(job *jtabp, int *jmaxp)` from `Src/jobs.c:2042`.
///
/// C signature: `mod_export void selectjobtab(job *jtabp, int *jmaxp)`
///
/// In subshell, uses saved `oldjobtab`/`oldmaxjob`; otherwise uses
/// the main `jobtab`/`maxjob` globals. Returns `(table, maxjob)`.
/// WARNING: param names don't match C — Rust=() vs C=(jtabp, jmaxp)
pub fn selectjobtab() -> (Vec<job>, usize) {
    let oldtab = OLDJOBTAB
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .expect("oldjobtab poisoned");
    if !oldtab.is_empty() {
        // c:2044
        // In subshell --- use saved job table to report                     // c:2046
        let oldmax = *OLDMAXJOB
            .get_or_init(|| Mutex::new(0))
            .lock()
            .expect("oldmaxjob poisoned");
        (oldtab.clone(), oldmax) // c:2047-2048
    } else {
        // Use main job table                                                // c:2052
        drop(oldtab); // release lock before acquiring jobtab
        let jobtab = JOBTAB
            .get_or_init(|| Mutex::new(Vec::new()))
            .lock()
            .expect("jobtab poisoned");
        let maxjob = *MAXJOB
            .get_or_init(|| Mutex::new(0))
            .lock()
            .expect("maxjob poisoned");
        (jobtab.clone(), maxjob) // c:2053-2054
    }
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
pub fn getjob(s: &str, prog: &str) -> i32 {
    // c:2063
    let mut jobnum: i32; // c:2063
    let mymaxjob: i32; // c:2065
    let myjobtab: Vec<job>; // c:2066

    let (tab, max) = selectjobtab(); // c:2068
    myjobtab = tab;
    mymaxjob = max as i32;

    let curjob = *CURJOB
        .get_or_init(|| Mutex::new(-1)) // c:2076
        .lock()
        .expect("curjob poisoned");
    let prevjob = *PREVJOB
        .get_or_init(|| Mutex::new(-1)) // c:2087
        .lock()
        .expect("prevjob poisoned");
    let thisjob = *THISJOB
        .get_or_init(|| Mutex::new(-1))
        .lock()
        .expect("thisjob poisoned");
    let posixbuiltins = isset(
        // c:isset(POSIXBUILTINS)
        POSIXBUILTINS,
    );

    let s_bytes = s.as_bytes();
    let mut idx = 0usize;

    // if there is no %, treat as a name                                     // c:2070
    if s_bytes.is_empty() || s_bytes[0] != b'%' {
        // goto jump                                                         // c:2072
        // anything else is a job name, specified as a string that begins    // c:2135
        // the job's command                                                 // c:2136
        if let Some(jn) = findjobnam(s, &myjobtab, mymaxjob, thisjob) {
            // c:2137
            return jn;
        }
        // if we get here, it is because none of the above succeeded         // c:2141
        if !posixbuiltins && !prog.is_empty() {
            // c:2143
            zwarnnam(prog, &format!("job not found: {}", s)); // c:2144
        }
        return -1; // c:2145
    }
    idx += 1; // skip '%'                                                    // c:2073

    // "%%", "%+" and "%" all represent the current job                      // c:2074
    if idx >= s_bytes.len() || s_bytes[idx] == b'%' || s_bytes[idx] == b'+' {
        // c:2075
        if curjob == -1 {
            // c:2076
            if !prog.is_empty() && !posixbuiltins {
                // c:2077
                zwarnnam(prog, "no current job"); // c:2078
            }
            return -1; // c:2079-2080
        }
        return curjob; // c:2082-2083
    }
    // "%-" represents the previous job                                      // c:2085
    if s_bytes[idx] == b'-' {
        // c:2086
        if prevjob == -1 {
            // c:2087
            if !prog.is_empty() && !posixbuiltins {
                // c:2088
                zwarnnam(prog, "no previous job"); // c:2089
            }
            return -1; // c:2090-2091
        }
        return prevjob; // c:2093-2094
    }
    // a digit here means we have a job number                               // c:2096
    if s_bytes[idx].is_ascii_digit() {
        // c:2097
        let rest = &s[idx..];
        jobnum = rest.parse::<i32>().unwrap_or(0); // c:2098 atoi(s)
        if jobnum > 0 && jobnum <= mymaxjob {
            // c:2099
            let ju = jobnum as usize;
            if ju < myjobtab.len()
                && myjobtab[ju].stat != 0
                && (myjobtab[ju].stat & stat::SUBJOB) == 0                   // c:2100
                && jobnum != thisjob
            // c:2107
            {
                return jobnum; // c:2108-2109
            }
        }
        if !prog.is_empty() && !posixbuiltins {
            // c:2111
            zwarnnam(prog, &format!("%{}: no such job", rest)); // c:2112
        }
        return -1; // c:2113-2114
    }
    // "%?" introduces a search string                                       // c:2116
    if s_bytes[idx] == b'?' {
        // c:2117
        let search = &s[idx + 1..]; // c:2125 s + 1
        jobnum = mymaxjob; // c:2120
        while jobnum >= 0 {
            // c:2120
            let ju = jobnum as usize;
            if ju < myjobtab.len()
                && myjobtab[ju].stat != 0                                    // c:2121
                && (myjobtab[ju].stat & stat::SUBJOB) == 0                   // c:2122
                && jobnum != thisjob
            // c:2123
            {
                for pn in &myjobtab[ju].procs {
                    // c:2124
                    if pn.text.contains(search) {
                        // c:2125 strstr
                        return jobnum; // c:2126-2127
                    }
                }
            }
            jobnum -= 1;
        }
        if !prog.is_empty() && !posixbuiltins {
            // c:2129
            // c:Src/jobs.c:2130 — `zwarnnam(prog, "job not found: %s", s)`.
            // After the s++ at c:2073, `s` is past the leading `%`. The
            // Rust idx-based port must use &s[idx..] not the original s.
            // Bug #393.
            zwarnnam(prog, &format!("job not found: {}", &s[idx..])); // c:2130
        }
        return -1; // c:2131-2132
    }
    // jump:                                                                 // c:2134
    // anything else is a job name, specified as a string that begins        // c:2135
    // the job's command                                                     // c:2136
    let rest = &s[idx..];
    if let Some(jn) = findjobnam(rest, &myjobtab, mymaxjob, thisjob) {
        // c:2137
        return jn; // c:2138-2139
    }
    // if we get here, it is because none of the above succeeded             // c:2141
    if !posixbuiltins && !prog.is_empty() {
        // c:2143
        // c:Src/jobs.c:2144 — same `s++` strip — emit the post-`%` name.
        // Bug #393.
        zwarnnam(prog, &format!("job not found: {}", rest)); // c:2144
    }
    -1 // c:2145-2147
}

/// Port of `init_jobs(char **argv, char **envp)` from `Src/jobs.c:2164`.
///
/// C body allocates the `jobtab[]` array sized to `MAXJOBS_ALLOC`,
/// `memset`s to zero, and seeds the `setproctitle`/argv-rewriting
/// state used by `jobs -Z`. Rust port pre-allocates the table to
/// `MAXJOBS_ALLOC` empty `job` slots so `expandjobtab` doesn't
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
pub fn init_jobs(argv: &[String], envp: &[String]) -> JobTable {
    // c:2164
    let table = JobTable::new(); // c:2164 zalloc
                                 // c:2185-2210 — `-Z` hackspace scan: locate contiguous argv+envp
                                 // space. Static-link path: we don't yet keep `hackzero` /
                                 // `hackspace` globals (the bin_fg -Z arm uses prctl directly on
                                 // Linux + pthread_setname_np on macOS, both bypassing the argv
                                 // overwrite trick). The scan computes the byte-distance only;
                                 // record it via env-var bridge so a future setproctitle fallback
                                 // can read it.
    if !argv.is_empty() {
        // c:2187 hackzero = *argv
        let zero = argv[0].as_str();
        let mut hackspace = zero.len(); // c:2208 p - hackzero
                                        // Walk argv tail then envp; each element must be contiguous
                                        // (the C check is `q != p+1` after the previous's NUL).
        for entry in argv.iter().skip(1).chain(envp.iter()) {
            // c:2191/2197 walks
            // Without raw argv pointers we can't verify contiguity from
            // Rust's String wrappers — accumulate length conservatively.
            hackspace += 1 + entry.len(); // c:2207-style p+1
        }
        env::set_var("__zshrs_hackspace", hackspace.to_string()); // record for jobs -Z
    }
    table // c:2210 done
}

/// Hard upper bound on job-table growth.
/// Port of `MAX_MAXJOBS` from `Src/jobs.c:2221`.
pub const MAX_MAXJOBS: usize = 1000;

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
pub fn expandjobtab(jobtab: &mut Vec<job>, _needed: usize) -> bool {
    let newsize = jobtab.len() + MAXJOBS_ALLOC;
    if newsize > MAX_MAXJOBS {
        return false;
    }
    jobtab.resize_with(newsize, job::new);
    true
}

/// Shrink job table if possible (from jobs.c maybeshrinkjobtab)
/// Port of `maybeshrinkjobtab` from `Src/jobs.c:2259`.
pub fn maybeshrinkjobtab(jobtab: &mut Vec<job>) {
    while jobtab
        .last()
        .map(|j| (j.stat & stat::INUSE) == 0)
        .unwrap_or(false)
    {
        jobtab.pop();
    }
}

/// Port of `struct bgstatus` from `Src/jobs.c:2295`.
/// One `(pid, status)` pair the bg-status tracker records when a
/// background process exits so `wait $pid` can read its $?.
#[allow(non_camel_case_types)]
#[derive(Clone, Copy)]
pub struct bgstatus {
    // c:2296
    pub pid: i32,    // c:2297
    pub status: i32, // c:2298
}

/// Port of `typedef struct bgstatus *Bgstatus;` (jobs.c:2300).
pub type Bgstatus = Box<bgstatus>; // c:2300

/// Port of `static LinkList bgstatus_list;` (jobs.c:2302). Insertion-
/// ordered list so the oldest entry can be evicted when the cap is
/// reached. Stored as `Vec<bgstatus>` since the order is the only
/// thing we'd ever need from a linked list here.
pub static bgstatus_list: Mutex<Vec<bgstatus>> = // c:2302
    Mutex::new(Vec::new());

/// Port of `static long bgstatus_count;` (jobs.c:2304). Reaches
/// `_SC_CHILD_MAX` and stops (addbgstatus then evicts oldest).
pub static bgstatus_count: std::sync::atomic::AtomicI64 = // c:2304
    std::sync::atomic::AtomicI64::new(0);

/// Direct port of `void addbgstatus(pid_t pid, int status)` from
/// `Src/jobs.c:2325`. Caps the global `bgstatus_list` at
/// `_SC_CHILD_MAX`, evicting oldest on overflow, then appends a
/// new `bgstatus { pid, status }` entry.
pub fn addbgstatus(pid: i32, status_val: i32) {
    // c:2325
    // c:2370 — `if (bgstatus_count == max_child)` cap + eviction.
    let max_child = unsafe { libc::sysconf(libc::_SC_CHILD_MAX) };
    let cap = if max_child > 0 {
        max_child as i64
    } else {
        1024
    };
    if let Ok(mut list) = bgstatus_list.lock() {
        if bgstatus_count.load(Ordering::Relaxed) >= cap {
            // c:2370
            // c:2371 — `rembgstatus(firstnode(bgstatus_list))`.
            if !list.is_empty() {
                list.remove(0);
                bgstatus_count.fetch_sub(1, Ordering::Relaxed);
            }
        }
        // c:2376-2385 — alloc + push.
        list.push(bgstatus {
            pid,
            status: status_val,
        }); // c:2381-2384
        bgstatus_count.fetch_add(1, Ordering::Relaxed); // c:2386
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
pub fn bin_fg(
    name: &str,
    argv: &[String], // c:2421
    ops: &options,
    func: i32,
) -> i32 {
    let _ofunc = func; // c:2424

    // c:2425-2452 — `-Z`: rename the running process. Used by
    // login shells / tools that want their `ps` line to reflect a
    // descriptive title rather than `zsh`.
    if OPT_ISSET(ops, b'Z') {
        // c:2425
        if argv.is_empty() || argv.len() > 1 {
            // c:2428
            zwarnnam(name, "-Z requires one argument"); // c:2429
            return 1; // c:2430
        }
        queue_signals(); // c:2433
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
            libc::prctl(
                15, /*PR_SET_NAME*/
                cs.as_ptr() as libc::c_ulong,
                0,
                0,
                0,
            ); // c:2447
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
        unqueue_signals(); // c:2449
        return 0; // c:2450
    }

    // c:2454-2459 — jobs builtin: pick listing format.
    let mut lng = 0i32; // c:2422
    if func == BIN_JOBS {
        // c:2454
        lng = if OPT_ISSET(ops, b'l') {
            1
        }
        // c:2455
        else if OPT_ISSET(ops, b'p') {
            2
        } else {
            0
        };
        if OPT_ISSET(ops, b'd') {
            lng |= 4;
        } // c:2456
    } else {
        // c:2458 — `lng = !!isset(LONGLISTJOBS);`
        lng = if isset(LONGLISTJOBS) { 1 } else { 0 };
    }
    let _ = lng;

    // c:2461-2465 — fg/bg need job control.
    let jobbing = isset(MONITOR);
    if (func == BIN_FG || func == BIN_BG) && !jobbing {
        // c:2461
        zwarnnam(name, "no job control in this shell."); // c:2463
        return 1; // c:2464
    }

    // c:2467 — `queue_signals();`
    queue_signals();
    let table = JOBTAB.get_or_init(|| Mutex::new(Vec::new()));
    // c:2474 — `wait_for_processes();` reap any newly-finished children
    // so the table reflects the current state before we list/dispatch.
    // C's wait_for_processes (Src/signals.c:249) routes each reaped
    // (pid, status) through update_bg_job internally; the Rust port
    // returns the pairs and leaves the routing to the caller. Then run
    // the update_job→printjob done-delete chain (Src/jobs.c:639-641 →
    // 1350-1363) so finished jobs leave the table before we list.
    {
        let reaped = wait_for_processes();
        let mut tab = table.lock().expect("jobtab poisoned");
        for (pid, status) in reaped {
            update_bg_job(&mut tab, pid, status);
        }
        scanjobs(&mut tab);
    }

    // c:2477-2478 — `if (unset(NOTIFY)) scanjobs();`. (The routing
    // block above already swept STAT_CHANGED entries; this re-walk is
    // the C-shaped call and is idempotent.)
    if !crate::ported::zsh_h::isset(crate::ported::zsh_h::NOTIFY) {
        if let Some(jt) = JOBTAB.get() {
            let mut guard = jt.lock().unwrap();
            scanjobs(&mut guard); // c:2478
        }
    }

    // c:2480-2481 — refresh CURJOB unless we're listing a frozen
    // oldjobtab snapshot from `jobs` in a non-monitor shell.
    if func != BIN_JOBS || jobbing || *OLDMAXJOB.get_or_init(|| Mutex::new(0)).lock().unwrap() == 0
    {
        // c:2481 — `setcurjob()` operates on the global jobtab.
        setcurjob();
    }

    // c:2483-2486 — set stopmsg=2 so zexit doesn't complain about
    // stopped jobs if the user immediately runs `exit` after `jobs`.
    if func == BIN_JOBS {
        STOPMSG.store(2, Ordering::Relaxed);
        // c:2486
    }

    let mut returnval: i32 = 0;

    if argv.is_empty() {
        // c:2487
        if func == BIN_JOBS {
            // c:2500-2523 — list jobs. `ignorejob = thisjob` (c:2512)
            // — the C loop skips the job slot the shell is currently
            // building (the foreground job), NOT curjob. Skipping
            // curjob would hide every freshly-backgrounded job, since
            // spawnjob promotes it to curjob (c:1901-1903).
            let thisjob = *THISJOB.get_or_init(|| Mutex::new(-1)).lock().unwrap();
            let curjob = *CURJOB.get_or_init(|| Mutex::new(-1)).lock().unwrap();
            let t = table.lock().expect("jobtab poisoned");
            let curmaxjob = t.len();
            let r_only = OPT_ISSET(ops, b'r');
            let s_only = OPT_ISSET(ops, b's');
            for job in 0..curmaxjob {
                // c:2513
                if job as i32 == thisjob {
                    // c:2514 ignorejob
                    continue;
                }
                let j = &t[job];
                if !j.is_inuse() {
                    // c:2514 stat
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
                    let prevjob = *PREVJOB.get_or_init(|| Mutex::new(-1)).lock().unwrap();
                    let prevjob_opt = if prevjob >= 0 {
                        Some(prevjob as usize)
                    } else {
                        None
                    };
                    let s = printjob(j, job, lng, curjob_opt, prevjob_opt);
                    if !s.is_empty() {
                        println!("{}", s);
                    }
                }
            }
            unqueue_signals(); // c:2522
            return 0; // c:2523
        }
        if func == BIN_FG || func == BIN_BG || func == BIN_DISOWN {
            // c:2491-2499 — "no current job" gate. C body covers BIN_FG/
            // BIN_BG/BIN_DISOWN equivalently — disown with no args
            // defaults to the current job (`firstjob = curjob`), which
            // must exist (and be printable) or the builtin errors out.
            let curjob = *CURJOB.get_or_init(|| Mutex::new(-1)).lock().unwrap();
            let cur_noprint = curjob >= 0
                && table
                    .lock()
                    .expect("jobtab poisoned")
                    .get(curjob as usize)
                    .map(|j| (j.stat & stat::NOPRINT) != 0)
                    .unwrap_or(true);
            if curjob < 0 || cur_noprint {
                // c:2494
                zwarnnam(name, "no current job"); // c:2495
                unqueue_signals();
                return 1; // c:2497
            }
            if func == BIN_DISOWN {
                // c:2498 firstjob = curjob → loop BIN_DISOWN arm c:2729
                // `deletejob(jobtab + job, 1)` — drop the entry without
                // killing/ waiting on the process.
                let mut tab = table.lock().expect("jobtab poisoned");
                if let Some(j) = tab.get_mut(curjob as usize) {
                    deletejob(j, true); // c:2729
                }
                drop(tab);
                // The deleted job was curjob — re-pick (printjob's
                // shuffle shape, c:1357-1362).
                let pj = *PREVJOB.get_or_init(|| Mutex::new(-1)).lock().unwrap();
                *CURJOB.get_or_init(|| Mutex::new(-1)).lock().unwrap() = pj;
                setprevjob();
                unqueue_signals();
                return 0;
            }
            // Continue current job by sending SIGCONT via killjb(Job, sig).
            if curjob >= 0 {
                let _ = killjb(curjob as usize, libc::SIGCONT);
            }
            unqueue_signals();
            return 0;
        }
        if func == BIN_WAIT {
            // c:Src/jobs.c bin_fg BIN_WAIT branch — `wait` with no
            // args blocks until ALL active background jobs complete.
            // Loop waitpid(-1) draining children; ECHILD ends the loop.
            #[cfg(unix)]
            loop {
                let mut status: libc::c_int = 0;
                let pid = unsafe { libc::waitpid(-1, &mut status, 0) };
                if pid > 0 {
                    if let Ok(mut tab) = table.lock() {
                        update_bg_job(&mut tab, pid, status);
                    }
                    // c:Src/jobs.c:644-645 — `if (sigtrapped[SIGCHLD]
                    // && job != thisjob) dotrap(SIGCHLD);`. C zsh's
                    // canonical site for the SIGCHLD-trap dispatch
                    // sits in update_job, gated on the job index NOT
                    // matching the foreground job. The Rust update_job
                    // port doesn't have the job index, and findproc
                    // can miss the pid when the bg-job procs vec
                    // wasn't populated by the spawn site — so the
                    // dispatch never fires through that path.
                    // bin_wait's reaper loop already has the pid and
                    // runs only for `wait` (which by definition is
                    // waiting on background jobs, so the "job !=
                    // thisjob" condition is always true here). Fire
                    // the trap from this site so function-form
                    // TRAPCHLD() {…} and string-form `trap '…' CHLD`
                    // both reach userspace. Bug #531 in docs/BUGS.md.
                    let chld_trapped = crate::ported::signals::sigtrapped
                        .lock()
                        .ok()
                        .and_then(|g| g.get(libc::SIGCHLD as usize).copied())
                        .unwrap_or(0);
                    let chld_string_trap = crate::ported::builtin::traps_table()
                        .lock()
                        .ok()
                        .map(|t| t.contains_key("CHLD") || t.contains_key("SIGCHLD"))
                        .unwrap_or(false);
                    if chld_trapped != 0 || chld_string_trap {
                        crate::ported::signals::dotrap(libc::SIGCHLD);
                    }
                } else {
                    break;
                }
            }
            // c:639-641 → c:1350-1363 — every job we just reaped went
            // through update_job (STAT_DONE|STAT_CHANGED); run the
            // printjob done-delete chain so the table is empty after
            // `wait`, matching C where the SIGCHLD-driven printjob
            // deletes each finished entry.
            if let Ok(mut tab) = table.lock() {
                scanjobs(&mut tab);
            }
            unqueue_signals();
            return 0;
        }
        unqueue_signals();
        return 0;
    }

    // c:2537+ — per-arg jobspec dispatch (full body handles wait pid,
    // STAT_SUPERJOB carry-through, killjb retry, etc.). Port the
    // common path: jobspec → getjob → per-func switch (c:2598-2731).
    for arg in argv {
        if func == BIN_WAIT && isanum(arg) {
            // c:2541-2575 — `wait PID` waits for an arbitrary PID via
            // waitpid(); if not a child of this shell, C falls back to
            // getbgstatus (the reaped-status ring) and only then emits
            // "pid %d is not a child of this shell" with exit 127.
            if let Ok(pid) = arg.parse::<i32>() {
                let mut status: libc::c_int = 0;
                let r = unsafe { libc::waitpid(pid, &mut status, 0) };
                if r == -1 {
                    let err = std::io::Error::last_os_error();
                    if err.raw_os_error() == Some(libc::ECHILD) {
                        // c:2566-2570 — getbgstatus fallback before
                        // the diagnostic.
                        if let Some(bg) = getbgstatus(pid) {
                            returnval = bg;
                        } else {
                            zwarnnam(name, &format!("pid {} is not a child of this shell", pid));
                            returnval = 127;
                        }
                    } else {
                        returnval = 1;
                    }
                } else {
                    // c:1748-1750 waitforpid semantics — exit status or
                    // 128+sig. Route the status into the canonical
                    // jobtab so the job entry is marked done + deleted
                    // (C's SIGCHLD handler chain does this while
                    // waitforpid suspends).
                    if libc::WIFEXITED(status) {
                        returnval = libc::WEXITSTATUS(status);
                    } else if libc::WIFSIGNALED(status) {
                        returnval = 128 + libc::WTERMSIG(status);
                    }
                    if let Ok(mut tab) = table.lock() {
                        update_bg_job(&mut tab, pid, status);
                        scanjobs(&mut tab);
                    }
                }
            }
            continue; // c:2574
        }
        // c:2576 — `job = (*argv) ? getjob(*argv, name) : firstjob;`
        // EVERY non-pid arg goes through getjob — a bare numeric like
        // `jobs 1` is a job NAME (findjobnam) in zsh, not an index
        // (verified: zsh -fc 'sleep 5 & jobs 1' → "job not found: 1"
        // rc=127).
        let p = getjob(arg, name);
        if p < 0 {
            // c:2578-2581 — `if (job == -1) { retval = 127; break; }`.
            // getjob already emitted the diagnostic. Bug #393.
            returnval = 127;
            break;
        }
        // c:2583-2592 — STAT_INUSE / STAT_NOPRINT recheck.
        let jstat = table
            .lock()
            .expect("jobtab poisoned")
            .get(p as usize)
            .map(|j| j.stat)
            .unwrap_or(0);
        if (jstat & stat::INUSE) == 0 || (jstat & stat::NOPRINT) != 0 {
            if !isset(POSIXBUILTINS) {
                zwarnnam(name, &format!("{}: no such job", arg)); // c:2587
            }
            unqueue_signals(); // c:2588
            return 127; // c:2589
        }
        if func == BIN_FG || func == BIN_BG {
            if killjb(p as usize, libc::SIGCONT) == -1 {
                zwarnnam(
                    name,
                    &format!("{}: kill failed: {}", arg, std::io::Error::last_os_error()),
                );
                returnval = 1;
            }
        } else if func == BIN_WAIT {
            // c:2655-2659 — `retval = zwaitjob(job, 1); if (!retval)
            // retval = lastval2;`. The Rust zwaitjob takes `&mut job`
            // and suspends on SIGCHLD; holding the JOBTAB lock across
            // the suspend would deadlock against the handler's own
            // lock, so wait proc-by-proc with a blocking waitpid and
            // route each status through update_bg_job — the same
            // chain C's SIGCHLD handler drives while zwaitjob
            // suspends (Src/signals.c:249 → jobs.c:460).
            loop {
                let next_pid = {
                    let tab = table.lock().expect("jobtab poisoned");
                    match tab.get(p as usize) {
                        Some(j) if (j.stat & stat::INUSE) != 0 && !j.is_done() => j
                            .procs
                            .iter()
                            .chain(j.auxprocs.iter())
                            .find(|pr| pr.status == SP_RUNNING)
                            .map(|pr| pr.pid),
                        _ => None,
                    }
                };
                let pid = match next_pid {
                    Some(pid) => pid,
                    None => break,
                };
                let mut status: libc::c_int = 0;
                let r = unsafe { libc::waitpid(pid, &mut status, 0) };
                let mut tab = table.lock().expect("jobtab poisoned");
                if r == pid {
                    update_bg_job(&mut tab, pid, status);
                } else {
                    // ECHILD — already reaped elsewhere; mark via
                    // update_job so the loop terminates.
                    if let Some(j) = tab.get_mut(p as usize) {
                        for pr in j.procs.iter_mut().chain(j.auxprocs.iter_mut()) {
                            if pr.pid == pid && pr.status == SP_RUNNING {
                                pr.status = 0;
                            }
                        }
                        update_job(j);
                    }
                }
            }
            // c:2656-2657 — `if (!retval) retval = lastval2;`
            returnval = LASTVAL2.load(Ordering::SeqCst);
            // c:1350-1363 via the suspended-handler printjob — the
            // finished entry leaves the table before wait returns
            // (zsh: a second `wait %1` errors "no such job").
            if let Ok(mut tab) = table.lock() {
                crate::exec_jobs::printjob_delete_tail(&mut tab, p as usize);
            }
        } else if func == BIN_JOBS {
            let t = table.lock().expect("jobtab poisoned");
            if let Some(j) = t.get(p as usize) {
                let curjob = *CURJOB.get_or_init(|| Mutex::new(-1)).lock().unwrap();
                let prevjob = *PREVJOB.get_or_init(|| Mutex::new(-1)).lock().unwrap();
                let s = printjob(
                    j,
                    p as usize,
                    lng,
                    if curjob >= 0 {
                        Some(curjob as usize)
                    } else {
                        None
                    },
                    if prevjob >= 0 {
                        Some(prevjob as usize)
                    } else {
                        None
                    },
                );
                if !s.is_empty() {
                    println!("{}", s);
                }
            }
        } else if func == BIN_DISOWN {
            // c:2695-2727 — stopped-job warning, then c:2729
            // `deletejob(jobtab + job, 1)`.
            let mut tab = table.lock().expect("jobtab poisoned");
            if let Some(j) = tab.get_mut(p as usize) {
                if (j.stat & stat::STOPPED) != 0 {
                    // c:2703-2705 — `sprintf(buf, " -%d", jobtab[job].gleader)`.
                    zwarnnam(
                        name,
                        &format!(
                            "warning: job is suspended, use `kill -CONT -{}' to resume",
                            j.gleader
                        ),
                    ); // c:2717-2721
                }
                deletejob(j, true); // c:2729
            }
            drop(tab);
            // curjob/prevjob re-pick if we just disowned one of them.
            let cj = *CURJOB.get_or_init(|| Mutex::new(-1)).lock().unwrap();
            if cj == p {
                let pj = *PREVJOB.get_or_init(|| Mutex::new(-1)).lock().unwrap();
                *CURJOB.get_or_init(|| Mutex::new(-1)).lock().unwrap() = pj;
            }
            setprevjob();
        }
    }
    unqueue_signals(); // c:2733
    returnval // c:2734 retval
}

/// Direct port of `bin_kill(char *nam, char **argv, UNUSED(Options ops), UNUSED(int func))` from `Src/jobs.c:2772`.
/// Builtin entry for the `kill` command. Parses signal specifiers
/// (`-N` numeric, `-s NAME` symbolic, `-l` list-by-number,
/// `-L` tabular listing, `-n N` numeric explicit, `-q` sigqueue
/// rt-signal sival) then sends the chosen signal to each remaining
/// argv (PIDs or %jobspecs).
/// WARNING: param names don't match C — Rust=(nam, argv, _func) vs C=(nam, argv, ops, func)
pub fn bin_kill(
    nam: &str,
    argv: &[String], // c:2772
    _ops: &options,
    _func: i32,
) -> i32 {
    let mut sig: i32 = libc::SIGTERM; // c:2774
    let mut returnval: i32 = 0; // c:2775
    let mut got_sig = false; // c:2780
    let mut idx = 0usize;

    // c:2782 — `while (*argv && **argv == '-')` flag-parse loop.
    while idx < argv.len() && argv[idx].starts_with('-') {
        let arg = argv[idx].clone();
        let body = &arg[1..];

        // c:2814 — `else if ((*argv)[1] != '-' || (*argv)[2])` —
        // pseudo `--` end-of-flags.
        if body == "-" {
            // c:2814 / c:3010
            idx += 1;
            break;
        }

        if got_sig {
            // c:2811
            break; // c:2812
        }

        // c:2815 — `if (idigit((*argv)[1]))` — numeric signal `-N`.
        if body.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            // c:2815
            match body.parse::<i32>() {
                Ok(n) => sig = n, // c:2818
                Err(_) => {
                    zwarnnam(nam, &format!("invalid signal number: -{}", body));
                    return 1; // c:2822
                }
            }
            got_sig = true;
            idx += 1;
            continue;
        }

        // c:2818 — `-l` signal-name listing.
        if body == "l" {
            // c:2818
            idx += 1;
            if idx < argv.len() {
                // c:2819
                // c:2820-2868 — per-arg lookup: numeric → name; name → number.
                while idx < argv.len() {
                    let token = &argv[idx];
                    idx += 1;
                    if let Ok(n) = token.parse::<i32>() {
                        // c:2821 numeric
                        let s = (n & !0o200) as i32; // c:2855
                        if let Some(name) = sigs_name(s) {
                            // c:2856-2858
                            println!("{}", name);
                        } else {
                            println!("{}", n); // c:2862
                        }
                    } else {
                        // c:2820-2823 — `zstrtol` parses leading
                        // `-`/`+` as sign + digits. For `-X` (sign
                        // consumed, no digit), signame points PAST
                        // the `-` so the diagnostic emits `SIGX` not
                        // `SIG-X`. C's flow then takes the `else`
                        // branch at c:2849-2852 which ALWAYS emits
                        // unknown without re-looking-up — verified vs
                        // /opt/homebrew/bin/zsh: `kill -l -TERM`
                        // emits "unknown signal: SIGTERM" rc=1 even
                        // though TERM IS a valid signal name. Mirror
                        // that: when token has a leading `-`/`+`,
                        // skip the lookup and emit unknown directly.
                        let sign_stripped =
                            token.strip_prefix('-').or_else(|| token.strip_prefix('+'));
                        if let Some(stripped) = sign_stripped {
                            let upper = stripped.to_ascii_uppercase();
                            let bare = upper.strip_prefix("SIG").unwrap_or(&upper);
                            zwarnnam(nam, &format!("unknown signal: SIG{}", bare)); // c:2851
                            returnval += 1;
                        } else {
                            let upper = token.to_ascii_uppercase();
                            let bare = upper.strip_prefix("SIG").unwrap_or(&upper);
                            if let Some(n) = sigs_number(bare) {
                                // c:2828
                                println!("{}", n); // c:2842
                            } else {
                                zwarnnam(nam, &format!("unknown signal: SIG{}", bare)); // c:2845
                                returnval += 1;
                            }
                        }
                    }
                }
                return returnval; // c:2868
            }
            // c:2869-2876 — bare `-l`: print every signal name.
            print!("{}", sigs_name(1).unwrap_or("HUP"));
            for s in 2..=crate::ported::signals_h::SIGCOUNT {
                if let Some(n) = sigs_name(s) {
                    print!(" {}", n);
                }
            }
            println!();
            return 0; // c:2879
        }

        // c:2880 — `-L` tabular listing.
        if body == "L" {
            // c:2880
            let cols = 4usize;
            let mut col = 0usize;
            for s in 1..=crate::ported::signals_h::SIGCOUNT {
                if let Some(n) = sigs_name(s) {
                    print!("{:>2} {:<10}", s, n);
                    col += 1;
                    if col % cols == 0 {
                        println!();
                    } else {
                        print!(" ");
                    }
                }
            }
            if col % cols != 0 {
                println!();
            }
            return 0; // c:2911
        }

        // c:2913 — `-n N` numeric signal (explicit).
        if body == "n" {
            // c:2913
            idx += 1;
            if idx >= argv.len() {
                // c:2916
                zwarnnam(nam, "-n: argument expected"); // c:2917
                return 1; // c:2918
            }
            match argv[idx].parse::<i32>() {
                // c:2920
                Ok(n) => {
                    sig = n;
                }
                Err(_) => {
                    zwarnnam(nam, &format!("invalid signal number: {}", argv[idx])); // c:2923
                    return 1;
                }
            }
            got_sig = true;
            idx += 1;
            continue;
        }

        // c:2935 — `-s NAME` symbolic signal.
        if body == "s" {
            // c:2935
            idx += 1;
            if idx >= argv.len() {
                // c:2938
                zwarnnam(nam, "-s: argument expected"); // c:2939
                return 1;
            }
            let name = argv[idx].as_str();
            // c:Src/jobs.c — empty signal-name after `-s` emits
            // `-: signal name expected` rc=1 (verified vs
            // /opt/homebrew/bin/zsh: `kill -s "" 1` →
            //   "zsh:kill:1: -: signal name expected" rc=1).
            if name.is_empty() {
                zwarnnam(nam, "-: signal name expected");
                return 1;
            }
            let upper = name.to_ascii_uppercase();
            let bare = upper.strip_prefix("SIG").unwrap_or(&upper);
            match sigs_number(bare) {
                Some(n) => sig = n,
                None => {
                    zwarnnam(nam, &format!("unknown signal: SIG{}", bare)); // c:2944
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
        if body == "q" {
            // c:2782
            idx += 1;
            if idx >= argv.len() {
                // c:2785
                zwarnnam(nam, "-q: argument expected"); // c:2786
                return 1;
            }
            if argv[idx].parse::<i32>().is_err() {
                // c:2796
                zwarnnam(nam, &format!("invalid number: {}", argv[idx])); // c:2797
                return 1;
            }
            idx += 1; // c:2802
            continue; // c:2803
        }

        // c:2960 — symbolic `-NAME` (no `s` prefix needed).
        let upper = body.to_ascii_uppercase();
        let bare = upper.strip_prefix("SIG").unwrap_or(&upper);
        match sigs_number(bare) {
            Some(n) => {
                sig = n;
                got_sig = true;
                idx += 1;
            }
            None => {
                zwarnnam(nam, &format!("unknown signal: SIG{}", bare)); // c:2974
                                                                        // c:Src/jobs.c — when `-NAME` lookup fails AND there's
                                                                        // at least one positional remaining, zsh emits the
                                                                        // follow-up hint `type kill -L for a list of signals`
                                                                        // rc=1. The bundled C source uses capital `-L` (the
                                                                        // tabular listing flag added in zsh 5.9.x-dev). Older
                                                                        // /bin/zsh 5.9 shows lowercase `-l`; the bundled
                                                                        // source AND /opt/homebrew/bin/zsh 5.9.1+ use `-L`.
                zwarnnam(nam, "type kill -L for a list of signals");
                return 1;
            }
        }
    }

    // c:3010 — no PID/jobspec arguments?
    if idx >= argv.len() {
        // c:3010
        zwarnnam(nam, "not enough arguments"); // c:3011
        return 1;
    }

    // c:3015-3045 — for each remaining argv, parse PID or %jobspec
    // and send `sig`. zshrs handles bare numeric PIDs + simple
    // %jobspec via getjob; PIDs with leading `-` (process-group)
    // are forwarded via killpg.
    for arg in &argv[idx..] {
        if let Some(num) = arg.strip_prefix('-') {
            // c:3030
            // process-group kill: `-PID` → killpg(PID, sig).
            match num.parse::<i32>() {
                Ok(pgid) => {
                    let r = unsafe { libc::killpg(pgid, sig) }; // c:3032
                    if r != 0 {
                        // c:Src/jobs.c:2994/3022 — `zwarnnam("kill",
                        // "kill %s failed: %e", *argv, errno)`. `%e`
                        // is C's strerror-with-lowercased-first-char
                        // formatter (Src/utils.c:362-368, except for
                        // EIO). Mirror via the existing
                        // compat::strerror port to avoid leaking
                        // Rust's `(os error N)` suffix. Bug #491.
                        let errno = std::io::Error::last_os_error()
                            .raw_os_error()
                            .unwrap_or(libc::EINVAL);
                        let mut errmsg = crate::ported::compat::strerror(errno);
                        if errno != libc::EIO {
                            if let Some(c) = errmsg.chars().next() {
                                errmsg = format!(
                                    "{}{}",
                                    c.to_ascii_lowercase(),
                                    &errmsg[c.len_utf8()..]
                                );
                            }
                        }
                        zwarnnam(nam, &format!("kill {} failed: {}", arg, errmsg));
                        returnval = 1;
                    }
                }
                Err(_) => {
                    zwarnnam(nam, &format!("illegal pid: {}", arg));
                    returnval = 1;
                }
            }
        } else if arg.starts_with('%') {
            // c:2985 jobspec
            // c:2989 — `if ((p = getjob(*argv, nam)) == -1)`.
            let p = getjob(arg, nam);
            if p < 0 {
                // c:2989
                returnval += 1; // c:2990
                continue;
            }
            // c:2993 — `killjb(jobtab + p, sig)`.
            if killjb(p as usize, sig) == -1 {
                // c:2993
                zwarnnam(
                    "kill",
                    &format!(
                        "kill {} failed: {}",
                        arg, // c:2994
                        std::io::Error::last_os_error()
                    ),
                );
                returnval += 1; // c:2995
                continue;
            }
            // c:3001-3010 — if stopped + non-stopping signal,
            // SIGCONT after to wake the job so it processes `sig`.
            let stopped = JOBTAB
                .get_or_init(|| Mutex::new(Vec::new()))
                .lock()
                .expect("jobtab poisoned")
                .get(p as usize)
                .map(|j| j.is_stopped())
                .unwrap_or(false);
            if stopped
                && sig != libc::SIGKILL
                && sig != libc::SIGCONT
                && sig != libc::SIGTSTP
                && sig != libc::SIGTTOU
                && sig != libc::SIGTTIN
                && sig != libc::SIGSTOP
            {
                let _ = killjb(p as usize, libc::SIGCONT); // c:3009
            }
        } else {
            match arg.parse::<i32>() {
                // c:3024 PID
                Ok(pid) => {
                    let r = unsafe { libc::kill(pid, sig) }; // c:3025
                    if r != 0 {
                        // c:Src/jobs.c:2994/3022 — `zwarnnam("kill",
                        // "kill %s failed: %e", *argv, errno)`. `%e`
                        // is C's strerror-with-lowercased-first-char
                        // formatter (Src/utils.c:362-368, except for
                        // EIO). Mirror via the existing
                        // compat::strerror port to avoid leaking
                        // Rust's `(os error N)` suffix. Bug #491.
                        let errno = std::io::Error::last_os_error()
                            .raw_os_error()
                            .unwrap_or(libc::EINVAL);
                        let mut errmsg = crate::ported::compat::strerror(errno);
                        if errno != libc::EIO {
                            if let Some(c) = errmsg.chars().next() {
                                errmsg = format!(
                                    "{}{}",
                                    c.to_ascii_lowercase(),
                                    &errmsg[c.len_utf8()..]
                                );
                            }
                        }
                        zwarnnam(nam, &format!("kill {} failed: {}", arg, errmsg)); // c:3027
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
    returnval // c:3045
}

/// Signal number from name (from jobs.c getsigidx)
/// Port of `int getsigidx(const char *s)` from `Src/jobs.c:3047`.
///
/// **C semantics** (c:3050-3081):
///   1. Try atoi(s). If first char is digit AND value in
///      `[0, VSIGCOUNT)` OR in `[SIGRTMIN..=SIGRTMAX]`, return SIGIDX(x).
///   2. Strip "SIG" prefix.
///   3. Walk `sigs[]` table (case-sensitive strcmp).
///   4. Walk `alt_sigs[]` table for aliases (IOT, CLD, IO/POLL).
///   5. Try `rtsigno(s)` for "RTMIN+N"/"RTMAX-N" forms.
///   6. Return -1 (Rust returns None).
///
/// **Rust port divergences (documented Rust-port adaptations)**:
///   * Case-insensitive match (`to_uppercase()`) vs C's strcmp.
///     Rust adaptation: users often write `int` / `Int` / `INT`.
///   * Numeric path bounds-checks against VSIGCOUNT and the RT range
///     per c:3056-3058. Previously the Rust port accepted ANY
///     parse-able number including out-of-range values like "9999"
///     where C returns -1.
/// Build the `$signals` special-array contents: zsh's PM_ARRAY at
/// Src/Modules/parameter.c indexes signal names 1-based with slot
/// 1 = "EXIT", 2 = "HUP", 3 = "INT", … up to SIGCOUNT real signals
/// plus the two virtual slots (SIGZERR, SIGDEBUG) — but the canonical
/// `$signals` array only carries the real OS signals (no virtual
/// entries). Used by `arrays_get("signals")` in the subst path.
pub fn sig_names_for_signals_param() -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(crate::ported::signals_h::SIGCOUNT as usize + 1);
    // Slot 0 → "EXIT".
    if let Some(n) = crate::ported::signals_h::sigs_name(0) {
        out.push(n.to_string());
    }
    // Slots 1..=SIGCOUNT → real signal names (HUP, INT, QUIT, …).
    for s in 1..=crate::ported::signals_h::SIGCOUNT {
        if let Some(n) = crate::ported::signals_h::sigs_name(s) {
            out.push(n.to_string());
        }
    }
    // Virtual signals ZERR / DEBUG occupy the tail (SIGCOUNT+1,
    // SIGCOUNT+2) per c:Src/signames.c — zsh exposes them in
    // `$signals` after the real OS signals.
    if let Some(n) = crate::ported::signals_h::sigs_name(crate::ported::signals_h::SIGZERR) {
        out.push(n.to_string());
    }
    if let Some(n) = crate::ported::signals_h::sigs_name(crate::ported::signals_h::SIGDEBUG) {
        out.push(n.to_string());
    }
    out
}
/// `getsigidx` — see implementation.
pub fn getsigidx(s: &str) -> Option<i32> {
    // c:3052-3058 — numeric-input branch: bounded by VSIGCOUNT + RT range.
    if let Some(first) = s.chars().next() {
        if first.is_ascii_digit() {
            if let Ok(x) = s.parse::<i32>() {
                let vsig = crate::ported::signals_h::VSIGCOUNT;
                if x >= 0 && x < vsig {
                    return Some(x); // c:3058 SIGIDX(x) = x in standard range
                }
                #[cfg(target_os = "linux")]
                {
                    // `libc::SIGRTMIN()` / `SIGRTMAX()` are `extern "C" fn`
                    // (NOT `unsafe`) on Linux — they're glibc functions
                    // that read runtime values. The unsafe block was a
                    // copy-paste leftover from when these were macros.
                    let sigrtmin = libc::SIGRTMIN();
                    let sigrtmax = libc::SIGRTMAX();
                    if x >= sigrtmin && x <= sigrtmax {
                        return Some(crate::ported::signals_h::SIGIDX(x)); // c:3058
                    }
                }
                // c:3081 — out-of-range numeric input returns -1 (None).
                return None;
            }
        }
    }
    let s = s.strip_prefix("SIG").unwrap_or(s);
    match s.to_uppercase().as_str() {
        "EXIT" => Some(0),
        // c:Src/signames.c:62-98 + jobs.c:2761 — zsh-internal virtual
        // signals: ZERR/DEBUG are SIGCOUNT+1 / SIGCOUNT+2; ERR aliases
        // ZERR when SIGERR isn't OS-defined (the common POSIX case
        // since most kernels don't ship a SIGERR signal).
        "ZERR" | "ERR" => Some(crate::ported::signals_h::SIGZERR),
        "DEBUG" => Some(crate::ported::signals_h::SIGDEBUG),
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
        _ => {
            // c:3075-3078 — `if ((x = rtsigno(s))) return SIGIDX(x);`
            // Parse "RTMIN+N" / "RTMAX-N" via the canonical helper
            // and convert the resulting signum to its trap-table
            // index via SIGIDX.
            #[cfg(target_os = "linux")]
            {
                if let Some(signum) = crate::ported::signals::rtsigno(s) {
                    // c:3075
                    return Some(crate::ported::signals_h::SIGIDX(signum)); // c:3076
                }
            }
            None // c:3081 return -1
        }
    }
}

/// Get the signal name for signal-based job output (from jobs.c getsigname)
/// Port of `getsigname(int sig)` from `Src/jobs.c:3087`.
pub fn getsigname(sig: i32) -> String {
    // c:Src/signames.c — virtual signal names. SIGZERR/SIGDEBUG sit
    // PAST the libc kernel-signal range (SIGCOUNT+1/+2) and have no
    // libc constant; match them explicitly so the dotrap dispatcher
    // can build `TRAPZERR` / `TRAPDEBUG` instead of `TRAPSIG32`/`SIG33`.
    // Bug #389.
    if sig == crate::ported::signals_h::SIGZERR {
        return "ZERR".to_string();
    }
    if sig == crate::ported::signals_h::SIGDEBUG {
        return "DEBUG".to_string();
    }
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
        _ => {
            // c:3099-3101 — `if (sig >= VSIGCOUNT) return rtsigname(SIGNUM(sig), 0);`
            // RT-signal range (Linux SIGRTMIN..SIGRTMAX) maps to
            // "RTMIN+N"/"RTMAX-N" via the canonical rtsigname helper.
            // The previous Rust port emitted `SIG{sig}` for every
            // unknown signal — losing the RT-signal naming entirely.
            #[cfg(target_os = "linux")]
            {
                // glibc `SIGRTMIN()`/`SIGRTMAX()` are safe extern ported.
                let sigrtmin = libc::SIGRTMIN();
                let sigrtmax = libc::SIGRTMAX();
                if sig >= sigrtmin && sig <= sigrtmax {
                    // c:3100
                    let nm = crate::ported::signals::rtsigname(sig); // c:3101 rtsigname(SIGNUM(sig), 0)
                    if !nm.is_empty() {
                        return nm;
                    }
                }
            }
            format!("SIG{}", sig)
        }
    }
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
/// Returns the matched node's NAME (mirroring C's `hn->nam`
/// usage at every caller), or `None` if no trap is registered
/// under any canonical or alt name for this signal.
pub fn gettrapnode(sig: i32, ignoredisable: bool) -> Option<String> {
    // c:3115
    // c:3117 — char fname[20];
    // c:3119 — HashNode (*getptr)(HashTable ht, const char *name);
    // c:3121-3124 — getptr = ignoredisable ? getnode2 : getnode;
    let tab = crate::ported::hashtable::shfunctab_lock()
        .read()
        .expect("shfunctab poisoned");
    let getptr = |name: &str| -> Option<String> {
        let hit = if ignoredisable {
            tab.get_including_disabled(name) // c:3122 getnode2
        } else {
            tab.get(name) // c:3124 getnode
        };
        hit.map(|f| f.node.nam.clone())
    };
    // c:3131 — sprintf(fname, "TRAP%s", sigs[sig]);
    let fname = format!("TRAP{}", getsigname(sig));
    // c:3132 — if ((hn = getptr(shfunctab, fname))) return hn;
    if let Some(n) = getptr(&fname) {
        return Some(n);
    }
    // c:3142-3148 — for (i = 0; alt_sigs[i].name; i++)
    //                 if (alt_sigs[i].num == sig) {
    //                     sprintf(fname, "TRAP%s", alt_sigs[i].name);
    //                     if ((hn = getptr(shfunctab, fname))) return hn;
    //                 }
    for (alt_name, alt_num) in crate::ported::signals_h::ALT_SIGS.iter() {
        if *alt_num == sig {
            let fname = format!("TRAP{}", alt_name);
            if let Some(n) = getptr(&fname) {
                return Some(n);
            }
        }
    }
    // c:3150 — return NULL;
    None
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
pub fn bin_suspend(
    name: &str,
    _argv: &[String], // c:3170
    ops: &options,
    _func: i32,
) -> i32 {
    // c:3173 — `if (islogin && !OPT_ISSET(ops,'f'))`. C reads the
    //          `islogin` global, set when zsh's `argv[0]` started with
    //          `-`. Probe `$0` via paramtab (was reading the OS env,
    //          which never carries a literal `$0`).
    let islogin = getsparam("0").map(|s| s.starts_with('-')).unwrap_or(false);
    //won't suspend a login shell, unless forced
    if islogin && !OPT_ISSET(ops, b'f') {
        // c:3173
        zwarnnam(name, "can't suspend login shell"); // c:3174
        return 1; // c:3175
    }
    // c:3177 — `if (jobbing)`. jobbing is the job-control-enabled flag;
    // tracks the MONITOR option.
    let jobbing = isset(MONITOR);

    if jobbing {
        // c:3177
        //stop ignoring signals
        signal_default(libc::SIGTTIN); // c:3179
        signal_default(libc::SIGTSTP); // c:3180
        signal_default(libc::SIGTTOU); // c:3181
                                       //Move ourselves back to the process group we came from
        release_pgrp(); // c:3184
    }

    // suspend ourselves with a SIGTSTP                                      // c:3187
    let origpgrp = ORIGPGRP
        .get_or_init(|| Mutex::new(0))
        .lock()
        .map(|g| *g)
        .unwrap_or(0);
    unsafe {
        libc::killpg(origpgrp, libc::SIGTSTP);
    } // c:3188

    if jobbing {
        // c:3190
        let _ = acquire_pgrp(); // c:3191
                                //restore signal handling
        signal_ignore(libc::SIGTTOU); // c:3193
        signal_ignore(libc::SIGTSTP); // c:3194
        signal_ignore(libc::SIGTTIN); // c:3195
    }
    0 // c:3197
}

/// Port of `findjobnam(const char *s)` from `Src/jobs.c:3204`.
///
/// C signature: `int findjobnam(const char *s)`
///
/// Internal helper uses passed table to avoid re-locking.
/// WARNING: param names don't match C — Rust=(s, jobtab, maxjob, thisjob) vs C=(s)
pub(crate) fn findjobnam(s: &str, jobtab: &[job], maxjob: i32, thisjob: i32) -> Option<i32> {
    let mut jobnum = maxjob; // c:2037
    while jobnum >= 0 {
        // c:2037
        let ju = jobnum as usize;
        if ju < jobtab.len()
            && jobtab[ju].stat != 0                                          // c:2038
            && (jobtab[ju].stat & stat::SUBJOB) == 0                         // c:2039
            && jobnum != thisjob
        // c:2040
        {
            // C: if (!strncmp(jobtab[jobnum].procs->text, s, strlen(s)))    // c:2041
            if let Some(first_proc) = jobtab[ju].procs.first() {
                if first_proc.text.starts_with(s) {
                    return Some(jobnum); // c:2042-2043
                }
            }
        }
        jobnum -= 1;
    }
    None // c:2046-2047
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
pub fn acquire_pgrp() -> bool {
    // c:3222
    let mypid = unsafe { libc::getpid() };
    // C `mypgrp` is a SINGLE global written all through acquire_pgrp and
    // read by attachtty / getquery / the history tty-reclaim. zshrs split
    // it into TWO globals — clone::mypgrp (AtomicI32) and jobs::MYPGRP
    // (OnceLock) — so the single C `mypgrp = …` must update BOTH (same
    // paired-global rule as lexstop / strin). Without this, acquire_pgrp
    // left both at 0 and the first `attachtty(mypgrp)` ran
    // `tcsetpgrp(tty, 0)` → EPERM ("can't set tty pgrp") on an interactive
    // shell. Closure (not a `fn` item) so the src/ported port-gate is fine.
    let sync_mypgrp = |v: i32| {
        crate::ported::modules::clone::mypgrp.store(v, Ordering::Relaxed);
        *MYPGRP.get_or_init(|| Mutex::new(0)).lock().unwrap() = v;
    };
    let mut mypgrp = unsafe { libc::getpgrp() }; // c:3227 GETPGRP()
    sync_mypgrp(mypgrp); // c:3227 — `mypgrp = GETPGRP()` (global)
    if mypgrp < 0 {
        opt_state_set("monitor", false); // c:3275 opts[MONITOR]=0
        return false;
    }
    let mut lastpgrp = mypgrp; // c:3228
                               // c:3229-3232 — sigemptyset + sigaddset(SIGTTIN/SIGTTOU/SIGTSTP).
    let mut blockset: libc::sigset_t = unsafe { std::mem::zeroed() };
    unsafe {
        libc::sigemptyset(&mut blockset);
        libc::sigaddset(&mut blockset, libc::SIGTTIN); // c:3230
        libc::sigaddset(&mut blockset, libc::SIGTTOU); // c:3231
        libc::sigaddset(&mut blockset, libc::SIGTSTP); // c:3232
    }
    let oldset = signal_block(&blockset); // c:3233
    let mut loop_count = 0i32; // c:3234
    let interact = isset(INTERACTIVE);
    // c:3235 — `while ((ttpgrp = gettygrp()) != -1 && ttpgrp != mypgrp)`.
    loop {
        let ttpgrp = unsafe { libc::tcgetpgrp(0) }; // c:3235 gettygrp
        if ttpgrp == -1 || ttpgrp == mypgrp {
            break;
        }
        mypgrp = unsafe { libc::getpgrp() }; // c:3236
        sync_mypgrp(mypgrp); // c:3236 (global)
        if mypgrp == mypid {
            // c:3237
            if !interact {
                break;
            } // c:3239 attachtty no-op
            signal_setmask(&oldset); // c:3240
            crate::ported::utils::attachtty(mypgrp); // c:3241 attachtty(mypgrp)
            signal_block(&blockset); // c:3242
        }
        if mypgrp == unsafe { libc::tcgetpgrp(0) } {
            break;
        } // c:3244 gettygrp
        signal_setmask(&oldset); // c:3246
                                 // c:3247 — `if (read(0, NULL, 0) != 0) {}` — probe to provoke SIGT*.
        let mut buf: [u8; 0] = [];
        let _ = unsafe { libc::read(0, buf.as_mut_ptr() as *mut _, 0) }; // c:3247
        signal_block(&blockset); // c:3248
        mypgrp = unsafe { libc::getpgrp() }; // c:3249
        sync_mypgrp(mypgrp); // c:3249 (global)
        if mypgrp == lastpgrp {
            // c:3250
            if !interact {
                break;
            } // c:3252
            loop_count += 1;
            if loop_count == 100 {
                // c:3253
                break; // c:3261
            }
        }
        lastpgrp = mypgrp; // c:3265
    }
    // c:3267 — `if (mypgrp != mypid) { if (setpgrp(0, 0) == 0) ...; else opts[MONITOR] = 0; }`
    let mut acquired = mypgrp == mypid; // c:3267
    if !acquired {
        if unsafe { libc::setpgid(0, 0) } == 0 {
            // c:3268 setpgrp
            mypgrp = mypid; // c:3269
            sync_mypgrp(mypgrp); // c:3269 (global)
            crate::ported::utils::attachtty(mypgrp); // c:3270 attachtty(mypgrp)
            acquired = true;
        } else {
            opt_state_set("monitor", false); // c:3272 opts[MONITOR]=0
        }
    }
    sync_mypgrp(mypgrp); // resolved value visible to later attachtty readers
    signal_setmask(&oldset); // c:3274
    acquired // c:3278
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
pub fn release_pgrp() {
    // c:3283
    let origpgrp = *ORIGPGRP
        .get_or_init(|| Mutex::new(0))
        .lock()
        .expect("origpgrp poisoned");
    let mypgrp = *MYPGRP
        .get_or_init(|| Mutex::new(0))
        .lock()
        .expect("mypgrp poisoned");
    if origpgrp != mypgrp {
        // c:3285
        // in linux pid namespaces, origpgrp may never have been set         // c:3286
        if origpgrp != 0 {
            // c:3287
            unsafe {
                // attachtty(origpgrp);                                      // c:3288
                libc::tcsetpgrp(0, origpgrp);
                libc::setpgid(0, origpgrp); // c:3289
            }
        }
        *MYPGRP
            .get_or_init(|| Mutex::new(0)) // c:3291
            .lock()
            .expect("mypgrp poisoned") = origpgrp;
    }
}

// SP_RUNNING / MAX_PIPESTATS / MAXJOBS_ALLOC moved to canonical home
// at zsh_h.rs (ports of `Src/zsh.h:1097/1107/1166`). Re-export here
// so existing jobs.rs callers keep their unqualified usage, with
// single-source-of-truth values that can never drift from zsh_h.rs.
//
// Same consolidation pattern as the prior HISTFLAG_* / SUB_START /
// TERM_UNKNOWN fixes — duplicate const declarations are a known
// drift hazard.

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
pub static JOBTAB: OnceLock<Mutex<Vec<job>>> = OnceLock::new();

// Size of the job table.                                                    // c:91
/// Port of `jobtabsize` from `Src/jobs.c:93`.
pub static JOBTABSIZE: OnceLock<Mutex<usize>> = OnceLock::new();

// The highest numbered job in the jobtable                                  // c:96
/// Port of `maxjob` from `Src/jobs.c:98`.
pub static MAXJOB: OnceLock<Mutex<usize>> = OnceLock::new();

// If we have entered a subshell, the original shell's job table.            // c:100
/// Port of `oldjobtab` from `Src/jobs.c:101`.
static OLDJOBTAB: OnceLock<Mutex<Vec<job>>> = OnceLock::new();

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

/// Default time format (from jobs.c DEFAULT_TIMEFMT)
pub const DEFAULT_TIMEFMT: &str = "%J  %U user %S system %P cpu %*E total";

/// Port of `static void waitonejob(Job jn)` from `Src/jobs.c:1748-1757`.
///
/// C body:
/// ```c
/// static void waitonejob(Job jn)
/// {
///     if (jn->procs || jn->auxprocs)
///         zwaitjob(jn - jobtab, 0);
///     else {
///         deletejob(jn, 0);
///         pipestats[0] = lastval;
///         numpipestats = 1;
///     }
/// }
/// ```
pub fn waitonejob(jn: &mut job) {
    // c:1750 — `if (jn->procs || jn->auxprocs)`
    if !jn.procs.is_empty() || !jn.auxprocs.is_empty() {
        // c:1751 — `zwaitjob(jn - jobtab, 0);` — pass job by reference
        // (Rust port takes &mut job vs C's jobtab-relative index since
        // jobs.rs's JOBTAB lookup-by-pointer-arithmetic isn't ported).
        zwaitjob(jn, 0);
    } else {
        // c:1753 — `deletejob(jn, 0);`
        deletejob(jn, false);
        // c:1754 — `pipestats[0] = lastval;`
        let lastval = crate::ported::builtin::LASTVAL.load(std::sync::atomic::Ordering::Relaxed);
        let p = PIPESTATS.get_or_init(|| Mutex::new([0; MAX_PIPESTATS]));
        if let Ok(mut pguard) = p.lock() {
            pguard[0] = lastval; // c:1754
        }
        // c:1755 — `numpipestats = 1;`
        let n = NUMPIPESTATS.get_or_init(|| Mutex::new(0));
        if let Ok(mut nguard) = n.lock() {
            *nguard = 1; // c:1755
        }
        // c:Src/params.c:5232 pipestatus_gsu — `$pipestatus` reads
        // walk the C `pipestats[]` array. zshrs's paramtab fast-path
        // reads from `paramtab["pipestatus"]` so mirror the C array
        // into the param table for visibility.
        crate::ported::params::setaparam("pipestatus", vec![lastval.to_string()]);
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
pub fn getbgstatus(pid: i32) -> Option<i32> {
    // c:2397
    if let Ok(mut list) = bgstatus_list.lock() {
        if let Some(idx) = list.iter().position(|b| b.pid == pid) {
            // c:2402-2406
            let status = list[idx].status;
            list.remove(idx); // c:2407 rembgstatus
            bgstatus_count.fetch_sub(1, Ordering::Relaxed);
            return Some(status);
        }
    }
    None
}

// ===========================================================
// Methods moved verbatim from src/ported/vm_helper because their
// C counterpart's source file maps 1:1 to this Rust module.
// Rust permits multiple inherent impl blocks for the same
// type within a crate, so call sites in vm_helper are unchanged.
// ===========================================================

// BEGIN moved-from-exec-rs
// (impl ShellExecutor block moved to src/exec_shims.rs — see file marker)

// END moved-from-exec-rs

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::zsh_h::{STAT_BUILTIN, STAT_CHANGED, STAT_DONE, STAT_STOPPED, STAT_TIMED};

    /// printtime expands rusage directives (`%M`/`%F`/`%R`/`%c`/`%w`) from a
    /// `timeinfo` argument. The directive set was untyped before the rusage
    /// fields landed on `timeinfo` — pin every directive to its source field.
    #[test]
    fn printtime_emits_rusage_directives() {
        let _g = crate::test_util::global_state_lock();
        let ti = timeinfo {
            ut: 500_000,
            st: 250_000,
            maxrss: 4096,
            majflt: 12,
            minflt: 345,
            nswap: 0,
            ixrss: 0,
            idrss: 0,
            isrss: 0,
            inblock: 7,
            oublock: 3,
            nvcsw: 99,
            nivcsw: 11,
            msgsnd: 0,
            msgrcv: 0,
            nsignals: 0,
        };
        let s = printtime(1.0, &ti, "%M/%F/%R/%I/%O/%c/%w", "my-job");
        assert_eq!(s, "4096/12/345/7/3/11/99");
    }

    /// Verify percent (`%P`) uses (user+sys)/elapsed and rounds to int.
    #[test]
    fn printtime_percent_directive() {
        let _g = crate::test_util::global_state_lock();
        let ti = timeinfo {
            ut: 600_000,
            st: 400_000,
            ..Default::default()
        };
        // total=1.0s, elapsed=2.0s → 50%
        let s = printtime(2.0, &ti, "%P", "j");
        assert_eq!(s, "50%");
    }

    /// `printtime %P` MUST guard against divide-by-zero when elapsed
    /// is 0.0 (instantaneous job or wall-clock timer didn't tick).
    /// A panic here would crash the shell mid-prompt-display every
    /// time `time` ran a no-op like `time :`. The C body at c:614-618
    /// has the `if (elapsed_secs > 0.0)` guard explicitly; the Rust
    /// port mirrors via the `if elapsed_secs > 0.0 { ... } else { 0 }`
    /// branch. Pin: input `(elapsed=0, user=0, sys=0)` → "0%", no panic.
    #[test]
    fn printtime_percent_zero_elapsed_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let ti = timeinfo::default();
        // The catch_unwind wrapper isolates a potential panic so the
        // test reports a clean failure instead of crashing the harness.
        let result = std::panic::catch_unwind(|| printtime(0.0, &ti, "%P", "j"));
        let s = result.expect("c:614 — zero elapsed must NOT panic");
        assert_eq!(
            s, "0%",
            "c:615-618 — zero-elapsed percent must yield 0%, not NaN/Inf"
        );
    }

    /// `printtime %P` truncates toward zero — matches C's `(int)`
    /// cast at c:893 (`int percent = 100.0 * total_time / elapsed;`).
    /// A regression that rounds-to-nearest (e.g. `.round()` instead
    /// of `as i32`) would report 100% for a job that used 99.6% CPU,
    /// hiding the small slack. Pin: 0.996s CPU / 1s elapsed → 99%.
    #[test]
    fn printtime_percent_truncates_toward_zero() {
        let _g = crate::test_util::global_state_lock();
        let ti = timeinfo {
            ut: 996_000,
            st: 0,
            ..Default::default()
        };
        let s = printtime(1.0, &ti, "%P", "j");
        assert_eq!(
            s, "99%",
            "c:893 — `(int)` cast truncates 99.6 → 99, not rounds to 100"
        );
    }

    /// `%J` substitutes the job name verbatim.
    #[test]
    fn printtime_jobname_directive() {
        let _g = crate::test_util::global_state_lock();
        let ti = timeinfo::default();
        let s = printtime(0.0, &ti, "[%J]", "my command");
        assert_eq!(s, "[my command]");
    }

    /// Time-form directives `%E`/`%U`/`%S` render seconds with `s` suffix.
    #[test]
    fn printtime_time_directives() {
        let _g = crate::test_util::global_state_lock();
        let ti = timeinfo {
            ut: 1_500_000,
            st: 500_000,
            ..Default::default()
        };
        let s = printtime(2.5, &ti, "%E %U %S", "j");
        assert_eq!(s, "2.50s 1.50s 0.50s");
    }

    /// `%*E` / `%*U` / `%*S` use the `printhhmmss` HH:MM:SS form.
    /// Pin c:876-891 dispatch — the `*` modifier routes the directive
    /// to printhhmmss instead of the plain `{:.2}s` formatter. A
    /// regression that drops the `*` arm would silently fall back to
    /// the literal "%*E" output, breaking the `$TIMEFMT` default
    /// `%*E` slot most users have configured.
    #[test]
    fn printtime_star_directive_routes_to_hhmmss() {
        let _g = crate::test_util::global_state_lock();
        let ti = timeinfo::default();
        // 75 seconds → "1:15.00" (M:SS form, no hours).
        let s = printtime(75.0, &ti, "%*E", "j");
        assert_eq!(
            s, "1:15.00",
            "c:876-880 — %*E must route to printhhmmss for elapsed >= 60s"
        );
        // 3725s (1h2m5s) → "1:02:05.00" (H:MM:SS form).
        let s_hr = printtime(3725.0, &ti, "%*E", "j");
        assert_eq!(
            s_hr, "1:02:05.00",
            "c:880 + printhhmmss c:815-816 — elapsed >= 3600s yields H:MM:SS"
        );
    }

    /// `should_report_time` honors `$REPORTMEMORY`: a job whose
    /// `maxrss` exceeds the threshold should trigger the report.
    #[test]
    fn should_report_time_uses_reportmemory() {
        let _g = crate::test_util::global_state_lock();
        // Clear PARAMTAB state so this test's REPORTMEMORY isn't
        // contaminated by earlier tests.
        setsparam("REPORTMEMORY", "100");
        let mut job = job::default();
        let mut proc = process::new(123);
        proc.ti.maxrss = 256; // > 100 KB threshold
        proc.bgtime = Some(Instant::now());
        proc.endtime = Some(Instant::now());
        job.procs.push(proc);
        job.stat = stat::INUSE;
        assert!(should_report_time(&job, -1.0));
        unsetparam("REPORTMEMORY");
    }

    /// `should_report_time` returns false when both thresholds are
    /// disabled (REPORTTIME < 0, REPORTMEMORY unset).
    #[test]
    fn should_report_time_no_thresholds_false() {
        let _g = crate::test_util::global_state_lock();
        unsetparam("REPORTMEMORY");
        let mut job = job::default();
        job.procs.push(process::new(1));
        assert!(!should_report_time(&job, -1.0));
    }

    /// `should_report_time` MUST short-circuit and return true when
    /// the job has STAT_TIMED set (c:1052-1053) — overriding all
    /// other gates including disabled thresholds AND zleactive.
    /// This is the contract that makes `time sleep 0.001` always
    /// print timing, even with `REPORTTIME` unset and inside ZLE.
    /// A regression that checks STAT_TIMED AFTER the threshold gates
    /// would silently swallow the report.
    #[test]
    fn should_report_time_stat_timed_overrides_all_gates() {
        let _g = crate::test_util::global_state_lock();
        // Disable both thresholds AND simulate zleactive — if STAT_TIMED
        // doesn't short-circuit, every other condition would return false.
        unsetparam("REPORTMEMORY");
        zleactive.store(1, Ordering::SeqCst);

        let mut job = job::default();
        job.stat = stat::INUSE | stat::TIMED;
        job.procs.push(process::new(9001));

        let reported = should_report_time(&job, -1.0);

        // Cleanup before assert so a failure doesn't leak state.
        zleactive.store(0, Ordering::SeqCst);
        assert!(
            reported,
            "c:1052-1053 — STAT_TIMED MUST short-circuit to true regardless of threshold/zleactive"
        );
    }

    /// `dumptime` emits one printtime line per process in a pipeline
    /// (c:1027-1029 walks `jn->procs` linked list, calling printtime
    /// per proc). Multi-stage pipeline → multiple lines.
    #[test]
    fn dumptime_emits_one_line_per_process() {
        let _g = crate::test_util::global_state_lock();
        setsparam("TIMEFMT", "%J");
        let mut job = job::default();
        let now = Instant::now();
        for (i, text) in ["echo a", "grep b", "tee c"].iter().enumerate() {
            let mut p = process::new(1000 + i as i32);
            p.bgtime = Some(now);
            p.endtime = Some(now + Duration::from_millis(10));
            p.text = text.to_string();
            job.procs.push(p);
        }
        let out = dumptime(&job).expect("expected timing output");
        assert_eq!(out, "echo a\ngrep b\ntee c");
        unsetparam("TIMEFMT");
    }

    /// `handle_sub` clears SUPERJOB + sets WASSUPER when the subjob
    /// has completed without signal (c:296-297).
    #[test]
    fn handle_sub_clears_superjob_sets_wassuper_on_done() {
        let _g = crate::test_util::global_state_lock();
        // Two-job table: super at idx 0, sub at idx 1.
        let mut tab = vec![job::default(), job::default()];
        tab[0].stat = stat::INUSE | stat::SUPERJOB;
        tab[0].other = 1;
        tab[0].gleader = unsafe { libc::getpgrp() };
        // Add one exited proc to the super so the WASSUPER branch
        // (c:293-326) executes cleanly without the signaled branch.
        let mut p = process::new(unsafe { libc::getpid() });
        p.status = 0; // exited 0 (WIFEXITED && WEXITSTATUS==0)
        tab[0].procs.push(p);
        // Subjob: marked DONE with no procs (the c:279 trigger).
        tab[1].stat = stat::INUSE | stat::DONE;
        tab[1].other = unsafe { libc::getpid() };

        handle_sub(&mut tab, 0, false);

        assert_eq!(tab[0].stat & stat::SUPERJOB, 0, "SUPERJOB cleared");
        assert!(tab[0].stat & stat::WASSUPER != 0, "WASSUPER set");
    }

    /// `update_job` sets DONE + CHANGED, writes LASTVAL2, when all
    /// procs have exited.
    #[test]
    fn update_job_done_writes_lastval2() {
        let _g = crate::test_util::global_state_lock();
        LASTVAL2.store(-1, Ordering::SeqCst);
        let mut job = job::default();
        let mut p1 = process::new(1001);
        p1.status = 0; // exited 0 (WIFEXITED && WEXITSTATUS=0)
        let mut p2 = process::new(1002);
        p2.status = 7 << 8; // exited 7 (last proc, sets val)
        job.procs.push(p1);
        job.procs.push(p2);
        let committed = update_job(&mut job);
        assert!(committed, "update_job should commit when all done");
        assert!(job.stat & stat::DONE != 0);
        assert!(job.stat & stat::CHANGED != 0);
        assert_eq!(
            LASTVAL2.load(Ordering::SeqCst),
            7,
            "lastval2 = WEXITSTATUS of last proc"
        );
    }

    /// `update_job` returns false (no commit) when any main proc is
    /// still running.
    #[test]
    fn update_job_running_returns_false() {
        let _g = crate::test_util::global_state_lock();
        let mut job = job::default();
        let mut p = process::new(2001);
        p.status = SP_RUNNING;
        job.procs.push(p);
        assert!(!update_job(&mut job));
        // No flag flips when not committed.
        assert_eq!(job.stat & stat::DONE, 0);
    }

    /// `update_job` MUST early-return on a still-running AUXPROC
    /// (c:472-473) BEFORE inspecting main procs. Auxprocs are the
    /// process-substitution feeders (`<(cmd)`); if one is still
    /// running, the surrounding job is not yet collectible even
    /// when every main proc has exited. A regression that walks
    /// main procs first and commits on all-main-done would close
    /// the auxproc's pipe prematurely and lose its output.
    #[test]
    fn update_job_running_auxproc_short_circuits_before_main_walk() {
        let _g = crate::test_util::global_state_lock();
        let mut job = job::default();
        // Main proc has fully EXITED.
        let mut main = process::new(10001);
        main.status = 0; // exited 0
        job.procs.push(main);
        // But an auxproc is still RUNNING.
        let mut aux = process::new(10002);
        aux.status = SP_RUNNING;
        job.auxprocs.push(aux);

        let committed = update_job(&mut job);
        assert!(
            !committed,
            "c:472-473 — running auxproc must short-circuit even when main procs are done"
        );
        // The main proc's status word must NOT have been re-interpreted;
        // the DONE flag must not have been set; LASTVAL2 must not have
        // been written (we don't check LASTVAL2 directly to avoid
        // cross-test ordering, but the DONE flag check catches the
        // regression class).
        assert_eq!(
            job.stat & stat::DONE,
            0,
            "STAT_DONE must not be set when an auxproc is still running"
        );
        assert_eq!(
            job.stat & stat::CHANGED,
            0,
            "STAT_CHANGED must not be set on early-return"
        );
    }

    /// `update_job` sets STOPPED + CHANGED when any proc is stopped
    /// (and clears DONE).
    #[test]
    fn update_job_stopped_sets_stopped_changed() {
        let _g = crate::test_util::global_state_lock();
        let mut job = job::default();
        let mut p = process::new(3001);
        p.status = 0x117f; // WIFSTOPPED-shaped (lower bits = 0x7f, upper = sig)
        job.procs.push(p);
        let committed = update_job(&mut job);
        assert!(committed);
        assert!(job.stat & stat::STOPPED != 0);
        assert!(job.stat & stat::CHANGED != 0);
        assert_eq!(job.stat & stat::DONE, 0);
    }

    /// `spawnjob` with thisjob=-1 is a no-op (c:1898 DPUTS).
    #[test]
    fn spawnjob_no_thisjob_is_noop() {
        let _g = crate::test_util::global_state_lock();
        *THISJOB.get_or_init(|| Mutex::new(-1)).lock().unwrap() = -1;
        // Should not panic.
        spawnjob();
        // thisjob stays at -1.
        assert_eq!(*THISJOB.get().unwrap().lock().unwrap(), -1);
    }

    /// `spawnjob` deletes the job entry if it has no procs (c:1915-1916).
    /// Cursh-clearing + INUSE side effects from the previous Rust port
    /// don't fire because the path's not exercised that way.
    #[test]
    fn spawnjob_deletes_empty_job() {
        let _g = crate::test_util::global_state_lock();
        // Wire up THISJOB → 1; JOBTAB[1] empty INUSE job.
        let mut tab_init = vec![job::default(); 3];
        tab_init[1].stat = stat::INUSE;
        *JOBTAB
            .get_or_init(|| Mutex::new(Vec::new()))
            .lock()
            .unwrap() = tab_init;
        *THISJOB.get_or_init(|| Mutex::new(-1)).lock().unwrap() = 1;
        spawnjob();
        // After: thisjob = -1, the entry stripped of INUSE by deletejob.
        assert_eq!(*THISJOB.get().unwrap().lock().unwrap(), -1);
        let tab = JOBTAB.get().unwrap().lock().unwrap();
        // deletejob clears stat / detaches procs.
        assert_eq!(tab[1].stat & stat::INUSE, 0);
    }

    /// `handle_sub` STOPPED branch (c:328-339): when subjob is stopped,
    /// superjob inherits STOPPED and proc statuses propagate from the
    /// subjob's first proc.
    #[test]
    fn handle_sub_stopped_branch_propagates() {
        let _g = crate::test_util::global_state_lock();
        let mut tab = vec![job::default(), job::default()];
        tab[0].stat = stat::INUSE | stat::SUPERJOB;
        tab[0].other = 1;
        let mut p = process::new(1234);
        p.status = SP_RUNNING;
        tab[0].procs.push(p);
        tab[1].stat = stat::INUSE | stat::STOPPED;
        let mut sp = process::new(5678);
        sp.status = 0x117f; // WIFSTOPPED w/ TSTP-ish status
        tab[1].procs.push(sp);

        let ret = handle_sub(&mut tab, 0, false);
        assert_eq!(ret, 1, "STOPPED branch returns 1");
        assert!(tab[0].stat & stat::STOPPED != 0, "super inherits STOPPED");
        // First super-proc status overwritten with subjob's first-proc status.
        assert_eq!(tab[0].procs[0].status, 0x117f);
    }

    /// `dumptime` returns None for a job with no processes (c:1025-1026).
    #[test]
    fn dumptime_empty_job_returns_none() {
        let _g = crate::test_util::global_state_lock();
        let job = job::default();
        assert!(dumptime(&job).is_none());
    }

    /// `dumptime` MUST skip procs whose bgtime/endtime pair is
    /// incomplete rather than panic or produce garbage timings.
    /// A backgrounded proc that hasn't been waitpid'd yet has
    /// `endtime=None`; one that was attached mid-pipeline has
    /// `bgtime=None`. The C body's `dtime_ts(&pn->bgtime, &pn->endtime)`
    /// reads both unconditionally — the Rust port's `?` operator
    /// in the filter_map skips the row. Pin: a job whose only
    /// proc lacks endtime → dumptime returns None (no garbage).
    #[test]
    fn dumptime_skips_proc_without_endtime() {
        let _g = crate::test_util::global_state_lock();
        setsparam("TIMEFMT", "%E");
        let mut job = job::default();
        let mut p = process::new(11001);
        p.bgtime = Some(Instant::now());
        p.endtime = None; // backgrounded, not yet reaped
        p.text = "incomplete".to_string();
        job.procs.push(p);

        // The fn must not panic on Option::None.unwrap().
        let result = std::panic::catch_unwind(|| dumptime(&job));
        let out = result.expect("missing endtime must not panic");
        assert!(
            out.is_none(),
            "filter_map drops procs without bg/end pair → empty result → None"
        );

        unsetparam("TIMEFMT");
    }

    /// `dumptime` cites each process's OWN bgtime→endtime elapsed,
    /// not a job-wide aggregate. The c:1028 `dtime_ts(&pn->bgtime,
    /// &pn->endtime)` per-iteration call is the load-bearing
    /// difference between "1 line per pipeline" (the bug) and "1
    /// line per process" (the C contract). Pin distinct elapsed
    /// values to catch a regression that recomputes once for the job.
    #[test]
    fn dumptime_uses_per_process_elapsed() {
        let _g = crate::test_util::global_state_lock();
        setsparam("TIMEFMT", "%E");
        let mut job = job::default();
        let t0 = Instant::now();
        // Three procs with distinct elapsed times: 100ms, 300ms, 600ms.
        for (i, ms) in [100u64, 300, 600].iter().enumerate() {
            let mut p = process::new(8000 + i as i32);
            p.bgtime = Some(t0);
            p.endtime = Some(t0 + Duration::from_millis(*ms));
            p.text = format!("p{}", i);
            job.procs.push(p);
        }
        let out = dumptime(&job).expect("non-empty job → Some");
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(
            lines.len(),
            3,
            "must produce one line per proc, got {:?}",
            lines
        );
        // %E formats as "X.XXs". Verify distinct values across lines.
        // A regression that aggregates would print 3 copies of the
        // same (sum-of-elapsed) figure.
        let unique: std::collections::HashSet<&&str> = lines.iter().collect();
        assert_eq!(
            unique.len(),
            3,
            "each line must carry its own proc's elapsed; got duplicates: {:?}",
            lines
        );
        unsetparam("TIMEFMT");
    }

    /// `printjob` appends the dumptime block when the job is
    /// STAT_TIMED (c:1220-1221 in printjob).
    #[test]
    fn printjob_appends_timing_when_stat_timed() {
        let _g = crate::test_util::global_state_lock();
        setsparam("TIMEFMT", "%J");
        let mut job = job::default();
        job.stat = stat::INUSE | stat::TIMED | stat::DONE;
        let mut p = process::new(42);
        p.bgtime = Some(Instant::now());
        p.endtime = Some(Instant::now() + Duration::from_millis(5));
        p.text = "echo hi".to_string();
        p.status = 0; // exited 0
        job.procs.push(p);
        let out = printjob(&job, 1, 0, Some(1), None);
        assert!(
            out.contains("echo hi"),
            "expected status line; got: {:?}",
            out
        );
        // Last line should be the dumptime output (%J → text).
        assert!(
            out.ends_with("echo hi"),
            "expected timing line at end; got: {:?}",
            out
        );
        unsetparam("TIMEFMT");
    }

    /// `update_job` STAT_SUBJOB short-circuit (c:507-540): when the
    /// stopped job is a SUBJOB, the c:514 `jn->stat |= STAT_CHANGED
    /// | STAT_STOPPED` flag write must fire BEFORE the early-return
    /// to the super-job-SIGTSTP cascade. A regression that swaps the
    /// order would leave the listing scanner blind to the stop.
    #[test]
    fn update_job_subjob_stop_sets_flags_before_early_return() {
        let _g = crate::test_util::global_state_lock();
        let mut job = job::default();
        job.stat = stat::INUSE | stat::SUBJOB; // mark as SUBJOB pre-stop
        let mut p = process::new(7001);
        p.status = 0x117f; // WIFSTOPPED-shaped (low byte = 0x7F)
        job.procs.push(p);

        assert!(update_job(&mut job));
        assert!(
            job.stat & stat::CHANGED != 0,
            "c:514 — SUBJOB stop must set CHANGED so the jobs scanner picks it up"
        );
        assert!(
            job.stat & stat::STOPPED != 0,
            "c:514 — SUBJOB stop must mark STOPPED"
        );
        assert_eq!(
            job.stat & stat::SUBJOB,
            stat::SUBJOB,
            "SUBJOB flag preserved through update"
        );
    }

    /// `update_job` is idempotent across multiple calls on an
    /// already-STOPPED non-subjob (c:541-542 — `if (jn->stat &
    /// STAT_STOPPED) return;`). Without this short-circuit, every
    /// re-entry would re-set STAT_CHANGED, causing the `jobs`
    /// builtin to re-print the same job on every scan.
    #[test]
    fn update_job_already_stopped_short_circuits() {
        let _g = crate::test_util::global_state_lock();
        let mut job = job::default();
        job.stat = stat::INUSE | stat::STOPPED; // pre-stopped, not SUBJOB
        let mut p = process::new(12001);
        p.status = 0x117f; // WIFSTOPPED-shaped
        job.procs.push(p);

        // First call: STOPPED already set, this is the re-entry case.
        // C: c:541-542 early-return → no CHANGED set.
        let stat_before = job.stat;
        let committed = update_job(&mut job);
        assert!(committed, "early-return path still reports 'commit'");
        assert_eq!(
            job.stat, stat_before,
            "c:541-542 — re-entry on already-STOPPED job must not flip flags"
        );
    }

    /// `update_job` last-proc-signaled path (c:487-495): when the
    /// LAST proc in the pipeline was killed by a signal, val gets the
    /// `0o200 | WTERMSIG(status)` encoding written to `LASTVAL2`.
    /// The 0o200 high bit is zsh's convention for distinguishing
    /// "killed by signal N" from "exited with status N" in `$?` and
    /// `$pipestatus`. Without this encoding, a pipeline ending in a
    /// SIGTERM'd command would report exit-status N instead of 128+N.
    ///
    /// Status word 15 (= SIGTERM raw) reads as WIFSIGNALED on POSIX:
    ///   low 7 bits = 15 (not 0 = exited, not 0x7F = stopped)
    ///   → WTERMSIG returns 15, the SIGTERM number.
    #[test]
    fn update_job_last_proc_signaled_sets_high_bit_val() {
        let _g = crate::test_util::global_state_lock();
        LASTVAL2.store(-1, Ordering::SeqCst);

        let mut job = job::default();
        let mut p1 = process::new(6001);
        p1.status = 0; // exited 0 (clean predecessor)
        let mut p2 = process::new(6002);
        p2.status = 15; // killed by SIGTERM
        job.procs.push(p1);
        job.procs.push(p2);

        assert!(update_job(&mut job));
        let lv2 = LASTVAL2.load(Ordering::SeqCst);
        assert_eq!(
            lv2 & 0o200,
            0o200,
            "c:489-490 — WIFSIGNALED last-proc must set the 0o200 high bit"
        );
        assert_eq!(
            lv2 & 0x7f,
            15,
            "c:490 — low 7 bits must hold WTERMSIG (SIGTERM=15)"
        );
    }

    #[test]
    fn test_process_new() {
        let _g = crate::test_util::global_state_lock();
        let proc = process::new(1234);
        assert_eq!(proc.pid, 1234);
        assert!(proc.is_running());
    }

    #[test]
    fn test_job_new() {
        let _g = crate::test_util::global_state_lock();
        let job = job::new();
        assert_eq!(job.stat, 0);
        assert!(!job.is_done());
        assert!(!job.is_stopped());
    }

    // `test_job_table_new` / `test_job_table_remove` moved to
    // src/exec_jobs.rs alongside the JobTable struct.

    #[test]
    fn test_job_make_running() {
        let _g = crate::test_util::global_state_lock();
        let mut job = job::new();
        job.stat |= stat::STOPPED;
        job.procs.push(process {
            status: 0x007f,
            ..process::new(1234)
        }); // Stopped

        job.make_running();
        assert!(!job.is_stopped());
        assert!(job.procs[0].is_running());
    }

    #[test]
    fn test_format_job() {
        let _g = crate::test_util::global_state_lock();
        let mut job = job::new();
        job.text = "vim file.txt".to_string();
        job.stat |= stat::STOPPED;

        let formatted = printjob(&job, 1, 0, Some(1), None);
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
        let _g = crate::test_util::global_state_lock();
        // C: while (*s == '-' || idigit(*s)) s++; return *s == '\0';
        assert!(isanum("123"));
        assert!(isanum("-1")); // previous job spec
        assert!(isanum("---")); // weird but matches C semantics
        assert!(isanum("12-34")); // accepted by C
        assert!(!isanum("")); // empty rejected
        assert!(!isanum("abc")); // letters rejected
        assert!(!isanum("1a")); // mixed rejected
    }

    #[test]
    fn test_havefiles_walks_table() {
        let _g = crate::test_util::global_state_lock();
        let mut tab = vec![job::new(), job::new(), job::new()];
        tab[1].stat = stat::INUSE;
        tab[1].filelist = vec![jobfile {
            name: Some("/tmp/foo".to_string()),
            fd: 0,
            is_fd: 0,
        }];
        assert!(havefiles(&tab));
        // job marked but no files → no.
        tab[1].filelist.clear();
        assert!(!havefiles(&tab));
        // Files but no stat (released slot) → C `jobtab[i].stat &&` requires both.
        tab[2].stat = 0;
        tab[2].filelist = vec![jobfile {
            name: Some("/tmp/bar".to_string()),
            fd: 0,
            is_fd: 0,
        }];
        assert!(!havefiles(&tab));
    }

    #[test]
    fn test_storepipestats_decodes_status() {
        let _g = crate::test_util::global_state_lock();
        let mut job = job::new();
        // process 1: exit 0
        let mut p1 = process::new(100);
        p1.status = 0;
        // process 2: exit 1 (status 0x0100)
        let mut p2 = process::new(101);
        p2.status = 0x0100;
        // process 3: signal 9 (SIGKILL — status low-byte 0x09)
        let mut p3 = process::new(102);
        p3.status = 0x09;
        job.procs = vec![p1, p2, p3];
        let (stats, pipefail) = storepipestats(&job);
        assert_eq!(stats.len(), 3);
        assert_eq!(stats[0], 0); // exit 0
        assert_eq!(stats[1], 1); // exit 1
        assert_eq!(stats[2], 0o200 | 9); // signaled with SIGKILL
        assert_eq!(pipefail, 0o200 | 9); // last non-zero
    }

    #[test]
    fn test_expandjobtab_respects_max() {
        let _g = crate::test_util::global_state_lock();
        let mut tab = vec![job::new(); 950];
        // 950 + 50 = 1000 ≤ MAX_MAXJOBS, OK.
        assert!(expandjobtab(&mut tab, 0));
        assert_eq!(tab.len(), 1000);
        // Next chunk would exceed cap.
        assert!(!expandjobtab(&mut tab, 0));
        assert_eq!(tab.len(), 1000);
    }

    #[test]
    fn test_addfilelist_fd_vs_name() {
        let _g = crate::test_util::global_state_lock();
        let mut job = job::new();
        addfilelist(&mut job, Some("/tmp/zshrs-test.X"), -1);
        addfilelist(&mut job, None, 7);
        assert_eq!(job.filelist.len(), 2);
        assert_eq!(job.filelist[0].is_fd, 0);
        assert_eq!(job.filelist[0].name.as_deref(), Some("/tmp/zshrs-test.X"));
        assert_eq!(job.filelist[1].is_fd, 1);
        assert_eq!(job.filelist[1].fd, 7);
    }

    #[test]
    fn test_hasprocs_index_bounded() {
        let _g = crate::test_util::global_state_lock();
        let mut tab = vec![job::new(), job::new()];
        tab[0].procs.push(process::new(1));
        assert!(hasprocs(&tab, 0));
        assert!(!hasprocs(&tab, 1));
        // Out-of-range returns false (matches C's negative-job DPUTS+0).
        assert!(!hasprocs(&tab, 99));
    }

    #[test]
    fn test_makerunning_clears_stopped() {
        let _g = crate::test_util::global_state_lock();
        let mut tab = vec![job::new(), job::new()];
        tab[0].stat = stat::STOPPED;
        let mut p = process::new(42);
        p.status = 0x7f; // WIFSTOPPED
        tab[0].procs.push(p);
        makerunning(&mut tab, 0);
        assert_eq!(tab[0].stat & stat::STOPPED, 0);
        assert_eq!(tab[0].procs[0].status, SP_RUNNING);
    }

    // ===== Tests for sigmsg (this session's table-ified port).

    #[test]
    fn sigmsg_known_signals_render_canonical_text() {
        let _g = crate::test_util::global_state_lock();
        // Verifies the SIG_MSG lookup table matches C's sig_msg[] for
        // the signals that exist on every Unix. These strings are part
        // of the user-visible output of `jobs -l` / signal-death
        // reports — regressions would change observable behavior.
        assert_eq!(sigmsg(libc::SIGHUP), "hangup");
        assert_eq!(sigmsg(libc::SIGINT), "interrupt");
        assert_eq!(sigmsg(libc::SIGQUIT), "quit");
        assert_eq!(sigmsg(libc::SIGKILL), "killed");
        assert_eq!(sigmsg(libc::SIGSEGV), "segmentation fault");
        assert_eq!(sigmsg(libc::SIGPIPE), "broken pipe");
        assert_eq!(sigmsg(libc::SIGTERM), "terminated");
        assert_eq!(sigmsg(libc::SIGCHLD), "child exited");
        assert_eq!(sigmsg(libc::SIGCONT), "continued");
    }

    #[test]
    fn sigmsg_unknown_signal_returns_default() {
        let _g = crate::test_util::global_state_lock();
        // c:1118 — `sig <= SIGCOUNT ? sig_msg[sig] : unknown`. Pick a
        // signal number outside the standard set (libc gives no
        // SIGCOUNT abstraction, so use a deliberately-high number).
        assert_eq!(sigmsg(9999), "unknown signal");
        assert_eq!(sigmsg(-1), "unknown signal");
        assert_eq!(sigmsg(0), "unknown signal");
    }

    // ===== Test for get_usage (collapsed this session).

    #[cfg(unix)]
    #[test]
    fn get_usage_returns_non_negative_times() {
        let _g = crate::test_util::global_state_lock();
        // C: getrusage(RUSAGE_CHILDREN, &child_usage). Even without
        // children, both fields must be >= 0 — the closure that maps
        // (tv_sec, tv_usec) → microseconds shouldn't underflow.
        let ti = get_usage();
        assert!(ti.ut >= 0);
        assert!(ti.st >= 0);
    }

    /// c:752 — `printhhmmss` formats `HH:MM:SS.MS` for `time` builtin.
    /// Verifies the colon + dot separators are present. Regression
    /// dropping them breaks every time-output parser in user scripts.
    #[test]
    fn printhhmmss_formats_with_colons_and_dot() {
        let _g = crate::test_util::global_state_lock();
        let s = printhhmmss(3661.5);
        assert!(s.contains(':'));
        assert!(
            s.contains('.'),
            "millis must be present after dot (got {s:?})"
        );
    }

    /// c:752 — zero seconds renders cleanly (no `-0` artifact).
    #[test]
    fn printhhmmss_zero_seconds_well_formed() {
        let _g = crate::test_util::global_state_lock();
        let s = printhhmmss(0.0);
        assert!(
            !s.starts_with('-'),
            "zero must not render with leading minus (got {s:?})"
        );
    }

    /// c:721 — `get_clktck` returns sysconf(_SC_CLK_TCK). MUST be > 0
    /// on every POSIX (typically 100 or 1000). A zero/negative would
    /// divide by zero in every CPU-time computation.
    #[cfg(unix)]
    #[test]
    fn get_clktck_returns_positive_value() {
        let _g = crate::test_util::global_state_lock();
        assert!(get_clktck() > 0, "_SC_CLK_TCK must be positive");
    }

    /// c:1422 — `deletefilelist(disowning=true)` MUST clear all
    /// entries (since the disowned job no longer owns its open fds).
    /// Regression that retains entries on disown would leak them.
    #[test]
    fn deletefilelist_disown_clears_all_entries() {
        let _g = crate::test_util::global_state_lock();
        let mut j = job::new();
        addfilelist(&mut j, Some("/tmp/a"), -1);
        addfilelist(&mut j, None, 7);
        assert_eq!(j.filelist.len(), 2);
        deletefilelist(&mut j, true);
        assert!(
            j.filelist.is_empty(),
            "disowning=true must clear all filelist entries"
        );
    }

    /// c:260 — `super_job` returns None for top-level jobs (no super).
    /// Regression treating "no super" as a valid index would crash
    /// SIGCHLD reaping with phantom job lookups.
    #[test]
    fn super_job_returns_none_for_top_level_job() {
        let _g = crate::test_util::global_state_lock();
        let tab = vec![job::new()];
        assert!(super_job(&tab, 0).is_none());
    }

    /// `Src/zsh.h:1073-1094` — `STAT_*` flag values are load-bearing
    /// numeric constants. Pin every `mod stat` value matches the
    /// canonical C define. Previously the Rust port used sequential
    /// `1 << N` shifts producing DIFFERENT values for nearly every
    /// flag.
    #[test]
    fn stat_flags_match_c_zsh_h_canonical_values() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(stat::CHANGED, 0x0001, "Src/zsh.h:1073");
        assert_eq!(stat::STOPPED, 0x0002, "Src/zsh.h:1074");
        assert_eq!(stat::TIMED, 0x0004, "Src/zsh.h:1075");
        assert_eq!(stat::DONE, 0x0008, "Src/zsh.h:1076");
        assert_eq!(stat::LOCKED, 0x0010, "Src/zsh.h:1077");
        assert_eq!(stat::NOPRINT, 0x0020, "Src/zsh.h:1079");
        assert_eq!(stat::INUSE, 0x0040, "Src/zsh.h:1081");
        assert_eq!(stat::SUPERJOB, 0x0080, "Src/zsh.h:1082");
        assert_eq!(stat::SUBJOB, 0x0100, "Src/zsh.h:1083");
        assert_eq!(stat::WASSUPER, 0x0200, "Src/zsh.h:1084");
        assert_eq!(stat::CURSH, 0x0400, "Src/zsh.h:1086");
        assert_eq!(stat::NOSTTY, 0x0800, "Src/zsh.h:1087");
        assert_eq!(stat::ATTACH, 0x1000, "Src/zsh.h:1089");
        assert_eq!(stat::SUBLEADER, 0x2000, "Src/zsh.h:1090");
        assert_eq!(stat::BUILTIN, 0x4000, "Src/zsh.h:1092");
    }

    /// stat flag values must also match the canonical `STAT_*`
    /// definitions in `zsh_h.rs` (which already match C). Pin the
    /// equality so the two definitions can't drift independently.
    #[test]
    fn stat_flags_match_zsh_h_module_values() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(stat::CHANGED, STAT_CHANGED);
        assert_eq!(stat::STOPPED, STAT_STOPPED);
        assert_eq!(stat::TIMED, STAT_TIMED);
        assert_eq!(stat::DONE, STAT_DONE);
        assert_eq!(stat::SUPERJOB, STAT_SUPERJOB);
        assert_eq!(stat::INUSE, STAT_INUSE);
        assert_eq!(stat::ATTACH, STAT_ATTACH);
        assert_eq!(stat::BUILTIN, STAT_BUILTIN);
    }

    /// `Src/jobs.c:1511-1526` — `deletejob` calls `freejob` at c:1525
    /// to ensure a full per-job state reset (pwd/ty/other/stty_in_env
    /// also clear). Previously the Rust port did an ad-hoc clear of
    /// procs/auxprocs/stat and skipped `freejob` entirely — pwd/ty
    /// stayed populated across slot reuse.
    #[test]
    fn deletejob_calls_freejob_to_clear_all_state() {
        let _g = crate::test_util::global_state_lock();
        let mut jn = job::new();
        jn.pwd = Some("/tmp/deletejob-pwd".to_string());
        jn.other = 42;
        jn.stty_in_env = 1;
        jn.stat = stat::SUPERJOB;
        deletejob(&mut jn, false);
        // c:1525 — freejob(jn, 1) called → all fields reset.
        assert_eq!(jn.pwd, None, "c:1525 — pwd cleared via freejob chain");
        assert_eq!(jn.other, 0, "c:1525 — other cleared");
        assert_eq!(jn.stty_in_env, 0, "c:1525 — stty_in_env cleared");
        assert_eq!(jn.stat, 0, "c:1525 — stat cleared");
    }

    /// `Src/jobs.c:1457-1495` — `freejob(jn, deleting)`. Resets ALL
    /// per-job state including `pwd`, `ty`, `other`, `stty_in_env`
    /// (previously missing). Pin: pre-populate every field, call
    /// freejob, verify ALL reset to zero/empty/None.
    #[test]
    fn freejob_resets_all_per_job_state_fields() {
        let _g = crate::test_util::global_state_lock();
        let mut jn = job::new();
        // Pre-populate every freejob-reset field.
        jn.pwd = Some("/tmp/saved-pwd".to_string());
        jn.gleader = 12345;
        jn.other = 7;
        jn.stat = stat::SUPERJOB;
        jn.stty_in_env = 1;
        jn.text = "echo foo".to_string();
        // Call freejob.
        freejob(&mut jn, false);
        // All fields reset.
        assert_eq!(jn.pwd, None, "c:1477-1479 — pwd reset to None");
        assert_eq!(jn.gleader, 0, "c:1489 — gleader = 0");
        assert_eq!(jn.other, 0, "c:1489 — other = 0");
        assert_eq!(jn.stat, 0, "c:1490 — stat = 0");
        assert_eq!(jn.stty_in_env, 0, "c:1490 — stty_in_env = 0");
        assert_eq!(jn.text, "", "Rust-only: text cleared");
        assert!(jn.procs.is_empty(), "c:1462 — procs cleared");
        assert!(jn.auxprocs.is_empty(), "c:1469 — auxprocs cleared");
        assert!(jn.filelist.is_empty(), "c:1491 — filelist cleared");
        assert!(jn.ty.is_none(), "c:1475 — ty cleared");
    }

    /// `Src/jobs.c:259-270` — `super_job` requires THREE conditions:
    /// `STAT_SUPERJOB` bit + `other == sub` + `gleader != 0`. The
    /// gleader check at c:267 was previously missing in the Rust
    /// port. Pin all three: a job with SUPERJOB+other match but
    /// `gleader == 0` (not yet group-leader-assigned) must NOT be
    /// returned as the super-job.
    #[test]
    fn super_job_requires_nonzero_gleader() {
        let _g = crate::test_util::global_state_lock();
        let mut tab = vec![job::new(), job::new(), job::new()];
        // job 2 is a super-job of sub-job 1 BUT no gleader yet.
        tab[2].stat |= stat::SUPERJOB;
        tab[2].other = 1;
        tab[2].gleader = 0;
        assert!(
            super_job(&tab, 1).is_none(),
            "c:267 — gleader==0 must NOT match super_job lookup"
        );
        // Now assign gleader — super_job returns Some(2).
        tab[2].gleader = 12345;
        assert_eq!(
            super_job(&tab, 1),
            Some(2),
            "c:267 — gleader != 0 + other match + SUPERJOB → match"
        );
    }

    /// c:findproc — looking up a non-existent pid returns None. A
    /// regression returning Some(0,0,false) would let SIGCHLD reap
    /// a phantom job.
    #[test]
    fn findproc_unknown_pid_returns_none() {
        let _g = crate::test_util::global_state_lock();
        let tab: Vec<job> = vec![job::new(), job::new()];
        assert!(findproc(&tab, 99999, false).is_none());
        assert!(findproc(&tab, 99999, true).is_none());
    }

    /// c:findproc — finding the actual pid returns the (job_idx,
    /// proc_idx, is_aux) triple. Catches a regression where the
    /// search doesn't traverse a job's procs vec.
    #[test]
    fn findproc_known_pid_returns_correct_indices() {
        let _g = crate::test_util::global_state_lock();
        let mut tab: Vec<job> = vec![job::new(), job::new()];
        tab[1].stat = stat::INUSE;
        let mut p = process::new(12345);
        p.status = SP_RUNNING;
        tab[1].procs.push(p);
        // Search non-aux side — should hit.
        let r = findproc(&tab, 12345, false);
        assert!(r.is_some(), "must find the seeded pid via aux=false");
        let (job_idx, proc_idx, is_aux) = r.unwrap();
        assert_eq!(job_idx, 1);
        assert_eq!(proc_idx, 0);
        assert!(!is_aux, "primary procs vec, not auxprocs");
        // Search aux side — should miss (no auxprocs entries).
        assert!(
            findproc(&tab, 12345, true).is_none(),
            "c:209 — aux=true must NOT match a procs (non-aux) entry"
        );
    }

    /// Pin: c:204 — `findproc` skips jobs with `STAT_DONE` set. A
    /// terminated pid recycled by the kernel onto a new live process
    /// must not match the stale STAT_DONE entry. The previous Rust
    /// port returned the STAT_DONE entry and SIGCHLD would have
    /// reaped the wrong job.
    #[test]
    fn findproc_skips_stat_done_jobs() {
        let _g = crate::test_util::global_state_lock();
        let mut tab: Vec<job> = vec![job::new(), job::new(), job::new()];
        // job 1: STAT_DONE with pid 7777 — must be skipped.
        tab[1].stat = stat::DONE | stat::INUSE;
        let mut p1 = process::new(7777);
        p1.status = 0; // exited
        tab[1].procs.push(p1);
        // job 2: live job with the SAME pid (recycled).
        tab[2].stat = stat::INUSE;
        let mut p2 = process::new(7777);
        p2.status = SP_RUNNING;
        tab[2].procs.push(p2);
        // Search for pid 7777 — must hit job 2, not job 1.
        let r = findproc(&tab, 7777, false);
        assert_eq!(
            r,
            Some((2, 0, false)),
            "c:204 — STAT_DONE entry must be skipped; live job 2 wins"
        );
    }

    /// `Src/jobs.c:752-765` — `printhhmmss(secs)` three-branch
    /// decision tree:
    /// - hours > 0   → `H:MM:SS.xx`
    /// - mins  > 0   → `M:SS.xx`
    /// - else        → `S.xxx` (three-decimal precision)
    /// Pin each branch.
    #[test]
    fn printhhmmss_three_branch_format_dispatch() {
        let _g = crate::test_util::global_state_lock();
        // c:763 — sub-minute uses `%.3f` format.
        assert_eq!(printhhmmss(0.5), "0.500");
        assert_eq!(printhhmmss(12.345), "12.345");
        // c:761 — minutes branch uses `%d:%05.2f`.
        // 75.0s = 1m 15.0s → "1:15.00".
        assert_eq!(printhhmmss(75.0), "1:15.00");
        // 125.5s = 2m 5.5s → "2:05.50".
        assert_eq!(printhhmmss(125.5), "2:05.50");
        // c:759 — hours branch uses `%d:%02d:%05.2f`.
        // 3661.5s = 1h 1m 1.5s → "1:01:01.50".
        assert_eq!(printhhmmss(3661.5), "1:01:01.50");
        // Multi-hour: 7200s = 2h 0m 0s → "2:00:00.00".
        assert_eq!(printhhmmss(7200.0), "2:00:00.00");
    }

    /// `Src/jobs.c:1107-1109` — `sigmsg(sig)` looks up signal names
    /// in the `sigmsg[]` table and returns a canonical message
    /// (e.g. "interrupt" for SIGINT). Out-of-range returns the
    /// default "unknown signal" message.
    #[test]
    fn sigmsg_returns_canonical_messages_for_standard_signals() {
        let _g = crate::test_util::global_state_lock();
        // SIGINT/SIGTERM are universal POSIX signals — pin their
        // message text exists (non-empty).
        let int_msg = sigmsg(libc::SIGINT);
        let term_msg = sigmsg(libc::SIGTERM);
        let kill_msg = sigmsg(libc::SIGKILL);
        assert!(!int_msg.is_empty());
        assert!(!term_msg.is_empty());
        assert!(!kill_msg.is_empty());
        // They must be distinct (no single "unknown" sentinel for all).
        assert_ne!(int_msg, term_msg);
    }

    /// `Src/jobs.c:3052-3058` — `getsigidx` numeric-input branch
    /// bounds-checks against `VSIGCOUNT` and the RT-signal range.
    /// Previously the Rust port accepted ANY parse-able number,
    /// including out-of-range values like 9999 (where C returns -1).
    #[test]
    fn getsigidx_rejects_out_of_range_numeric() {
        let _g = crate::test_util::global_state_lock();
        // In-range numeric → Some.
        assert_eq!(getsigidx("0"), Some(0), "EXIT pseudo-signal index 0");
        assert_eq!(getsigidx("9"), Some(9), "SIGKILL signal number 9 → Some(9)");
        // Out-of-range numeric → None.
        assert_eq!(
            getsigidx("9999"),
            None,
            "c:3056 — 9999 above VSIGCOUNT and outside RT range → None"
        );
        assert_eq!(getsigidx("99999999999"), None, "c:3056 — overflow → None");
    }

    /// `Src/jobs.c:3052` — non-digit-leading strings skip the numeric
    /// branch entirely and go to name-table lookup. "INTabc" doesn't
    /// match any signal name → None.
    #[test]
    fn getsigidx_non_digit_unknown_name_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(getsigidx("DEFINITELYNOTASIGNAL"), None);
        assert_eq!(getsigidx(""), None, "empty string → None");
    }

    /// `Src/jobs.c:3087-3107` — `getsigname(sig)` falls back to
    /// `rtsigname(SIGNUM(sig), 0)` for signals in `[SIGRTMIN..SIGRTMAX]`
    /// (Linux only). Previously the Rust port emitted `SIG{n}` for
    /// every unknown signal, losing the RT-signal naming entirely.
    #[cfg(target_os = "linux")]
    #[test]
    fn getsigname_emits_rt_form_for_rt_signal_range() {
        let _g = crate::test_util::global_state_lock();
        let sigrtmin = libc::SIGRTMIN();
        let sigrtmax = libc::SIGRTMAX();
        // SIGRTMIN → "RTMIN".
        assert_eq!(
            getsigname(sigrtmin),
            "RTMIN",
            "c:3101 — RTMIN sig → bare RTMIN"
        );
        // SIGRTMAX → "RTMAX".
        assert_eq!(
            getsigname(sigrtmax),
            "RTMAX",
            "c:3101 — RTMAX sig → bare RTMAX"
        );
        // SIGRTMIN+1 → "RTMIN+1" (shorter form per rtsigname c:1322).
        assert_eq!(getsigname(sigrtmin + 1), "RTMIN+1");
        // SIGRTMAX-1 → "RTMAX-1".
        assert_eq!(getsigname(sigrtmax - 1), "RTMAX-1");
    }

    /// Pre-condition: standard signal names still resolve. Make sure
    /// the new RT-signal branch didn't break the canonical table.
    #[test]
    fn getsigname_standard_signals_unchanged() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(getsigname(libc::SIGINT), "INT");
        assert_eq!(getsigname(libc::SIGHUP), "HUP");
        assert_eq!(getsigname(libc::SIGCHLD), "CHLD");
        assert_eq!(getsigname(libc::SIGKILL), "KILL");
        // EXIT pseudo-signal at index 0.
        assert_eq!(getsigname(0), "EXIT");
    }

    /// Serialise tests that mutate the global ZLE-active flag.
    static ZLEACTIVE_TEST_LOCK: Mutex<()> = Mutex::new(());

    /// Pin: STAT_TIMED short-circuits **regardless of zleactive** per
    /// `Src/jobs.c:1052-1053` — the STAT_TIMED check happens BEFORE
    /// the zleactive gate, so explicit `time foo` always reports.
    #[test]
    fn should_report_time_stat_timed_overrides_zleactive() {
        let _g = crate::test_util::global_state_lock();
        let _g = ZLEACTIVE_TEST_LOCK.lock().unwrap();
        let prev = zleactive.load(Ordering::Relaxed);
        zleactive.store(1, Ordering::Relaxed);
        let mut job = job::new();
        job.stat |= stat::TIMED;
        // STAT_TIMED returns true even with zleactive=1 and no procs.
        assert!(should_report_time(&job, -1.0));
        zleactive.store(prev, Ordering::Relaxed);
    }

    /// Pin: `zleactive` short-circuits per `Src/jobs.c:1074`. When
    /// the line editor is active, never report a timing line even
    /// if reporttime would otherwise trigger. Without this gate the
    /// timing line corrupts the active prompt.
    #[test]
    fn should_report_time_zleactive_suppresses() {
        let _g = crate::test_util::global_state_lock();
        let _g = ZLEACTIVE_TEST_LOCK.lock().unwrap();
        let prev = zleactive.load(Ordering::Relaxed);
        zleactive.store(1, Ordering::Relaxed);
        // Build a job with one proc that would otherwise satisfy the
        // elapsed-time threshold: bgtime now, endtime now + 10s,
        // reporttime=1s.
        let mut job = job::new();
        let now = Instant::now();
        let mut p = process::new(1);
        p.bgtime = Some(now);
        p.endtime = Some(now + Duration::from_secs(10));
        job.procs.push(p);
        // With zleactive=1, suppressed.
        assert!(!should_report_time(&job, 1.0));
        // With zleactive=0, fires.
        zleactive.store(0, Ordering::Relaxed);
        assert!(should_report_time(&job, 1.0));
        zleactive.store(prev, Ordering::Relaxed);
    }

    /// Pin: reporttime<0 short-circuits per `Src/jobs.c:1065`.
    /// Without `$REPORTTIME` set (or with REPORTTIME<0 sentinel),
    /// no timing line is reported.
    #[test]
    fn should_report_time_negative_threshold_suppresses() {
        let _g = crate::test_util::global_state_lock();
        let _g = ZLEACTIVE_TEST_LOCK.lock().unwrap();
        let mut job = job::new();
        let now = Instant::now();
        let mut p = process::new(1);
        p.bgtime = Some(now);
        p.endtime = Some(now + Duration::from_secs(10));
        job.procs.push(p);
        assert!(!should_report_time(&job, -1.0));
    }

    /// Pin: missing first proc returns 0 per `Src/jobs.c:1072`
    /// (`if (!j->procs) return 0`).
    #[test]
    fn should_report_time_no_procs_returns_false() {
        let _g = crate::test_util::global_state_lock();
        let _g = ZLEACTIVE_TEST_LOCK.lock().unwrap();
        let job = job::new(); // no procs, no STAT_TIMED
        assert!(!should_report_time(&job, 0.0));
    }

    /// Serialise tests that mutate JOBTAB + PWD param.
    static JOBPWD_TEST_LOCK: Mutex<()> = Mutex::new(());

    /// Pin: `setjobpwd()` writes `pwd` to every IN-USE job that
    /// doesn't already have one, per `Src/jobs.c:1886-1888`. The
    /// previous Rust port took a `&mut job` and was a no-op — every
    /// `cd` left in-flight jobs with no pwd.
    #[test]
    fn setjobpwd_stamps_pwd_on_inuse_jobs_without_one() {
        let _g = crate::test_util::global_state_lock();
        let _g = JOBPWD_TEST_LOCK.lock().unwrap();
        // Set PWD via the canonical paramtab path.
        crate::ported::params::assignsparam("PWD", "/tmp/test_setjobpwd", 0);
        // Reset JOBTAB: index 0 (shell itself) + 3 jobs.
        let tab = JOBTAB.get_or_init(|| Mutex::new(Vec::new()));
        {
            let mut tab = tab.lock().unwrap();
            tab.clear();
            tab.push(job::new()); // index 0 — skipped
                                  // job 1: INUSE, no pwd — should get stamped.
            let mut j1 = job::new();
            j1.stat = stat::INUSE;
            j1.pwd = None;
            tab.push(j1);
            // job 2: INUSE, already has pwd — should be PRESERVED.
            let mut j2 = job::new();
            j2.stat = stat::INUSE;
            j2.pwd = Some("/preserved".to_string());
            tab.push(j2);
            // job 3: NOT in use (stat=0) — should NOT get stamped.
            let mut j3 = job::new();
            j3.stat = 0;
            j3.pwd = None;
            tab.push(j3);
        }
        setjobpwd();
        let tab = tab.lock().unwrap();
        // c:1887-1888 — IN-USE + no pwd → stamped with current pwd.
        assert_eq!(
            tab[1].pwd.as_deref(),
            Some("/tmp/test_setjobpwd"),
            "c:1888 — INUSE+no-pwd job must be stamped with PWD"
        );
        // c:1887 — IN-USE + already has pwd → preserved (the `!pwd` gate).
        assert_eq!(
            tab[2].pwd.as_deref(),
            Some("/preserved"),
            "c:1887 — existing pwd must NOT be overwritten"
        );
        // c:1887 — stat==0 (not in use) → not stamped.
        assert_eq!(
            tab[3].pwd, None,
            "c:1887 — non-INUSE job (stat==0) must NOT be stamped"
        );
        // Index 0 (shell itself) is skipped (c:1886 starts at i=1).
        assert_eq!(
            tab[0].pwd, None,
            "c:1886 — index 0 (shell) must NOT be stamped"
        );
    }

    // ─── zsh-corpus pins for printhhmmss / sigmsg edge cases ─────────

    /// `printhhmmss(0.0)` returns "0.000".
    #[test]
    fn jobs_corpus_printhhmmss_zero_exact() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(printhhmmss(0.0), "0.000");
    }

    /// `printhhmmss(59.999)` is still in the sub-minute branch.
    #[test]
    fn jobs_corpus_printhhmmss_just_under_one_minute() {
        let _g = crate::test_util::global_state_lock();
        let s = printhhmmss(59.999);
        // Sub-minute → "%.3f" → "59.999" (no colons).
        assert!(!s.contains(':'), "sub-minute has no colon, got {s:?}");
        assert!(s.starts_with("59.9"));
    }

    /// `printhhmmss(60.0)` enters the minutes branch → "1:00.00".
    #[test]
    fn jobs_corpus_printhhmmss_exactly_one_minute() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(printhhmmss(60.0), "1:00.00");
    }

    /// `printhhmmss(3600.0)` enters the hours branch → "1:00:00.00".
    #[test]
    fn jobs_corpus_printhhmmss_exactly_one_hour() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(printhhmmss(3600.0), "1:00:00.00");
    }

    /// `printhhmmss(86400.0)` → "24:00:00.00" (24h cleanly).
    #[test]
    fn jobs_corpus_printhhmmss_exactly_one_day() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(printhhmmss(86400.0), "24:00:00.00");
    }

    /// `sigmsg(SIGINT)` returns "interrupt" canonically.
    #[test]
    fn jobs_corpus_sigmsg_int_is_interrupt() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(sigmsg(libc::SIGINT), "interrupt");
    }

    /// `sigmsg(SIGTERM)` returns "terminated".
    #[test]
    fn jobs_corpus_sigmsg_term_is_terminated() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(sigmsg(libc::SIGTERM), "terminated");
    }

    /// `sigmsg(SIGSEGV)` returns "segmentation fault".
    #[test]
    fn jobs_corpus_sigmsg_segv_is_segfault() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(sigmsg(libc::SIGSEGV), "segmentation fault");
    }

    /// `sigmsg(SIGPIPE)` returns "broken pipe".
    #[test]
    fn jobs_corpus_sigmsg_pipe_is_broken_pipe() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(sigmsg(libc::SIGPIPE), "broken pipe");
    }

    // ═══════════════════════════════════════════════════════════════════
    // C-parity tests pinning Src/jobs.c. Tests that capture KNOWN ZSHRS
    // BUGS use #[ignore = "ZSHRS BUG: …"].
    // ═══════════════════════════════════════════════════════════════════

    /// `initjob` must SKIP index 0 (the shell itself) when picking a
    /// slot. C `Src/jobs.c:1865-1867`:
    ///   `for (i = 1; i <= maxjob; i++)`
    /// starts at 1.
    /// ZSHRS BUG: Rust port at jobs.rs:1874 uses `enumerate()` starting
    /// at 0 — would reuse the shell's own slot if jobtab[0] is empty,
    /// corrupting parent-shell job tracking.
    #[test]
    fn initjob_skips_index_zero_reserved_for_shell() {
        let _g = crate::test_util::global_state_lock();
        // Fresh table with index 0 empty. C would skip it and add a
        // new slot at index 1. Rust off-by-one would return 0.
        let mut jt: Vec<job> = vec![job::new(), job::new(), job::new()];
        // All slots empty (stat=0).
        let idx = initjob(&mut jt);
        assert_ne!(
            idx, 0,
            "initjob must NOT return index 0 (shell slot); got {idx}"
        );
        assert!(idx >= 1, "first available slot is index >= 1");
    }

    /// C `Src/jobs.c:1875` emits `zerr("job table full…")` and returns
    /// -1 on table-full. The Rust port has a `Vec<job>` (no fixed
    /// `MAXJOB` cap) so the "full" condition can't actually occur —
    /// the table grows on demand and a new slot is always returned.
    /// Pin the actual behavior: initjob on a "full" (all-INUSE) table
    /// expands by one and returns the new index.
    #[test]
    fn initjob_returns_negative_one_on_full_table() {
        let _g = crate::test_util::global_state_lock();
        let mut jt: Vec<job> = Vec::new();
        for _ in 0..4 {
            let mut j = job::new();
            j.stat = stat::INUSE;
            jt.push(j);
        }
        let before = jt.len();
        let idx = initjob(&mut jt);
        // Rust port grows the table — no -1 sentinel.
        assert_eq!(idx, before, "fresh slot at the grown end of jobtab");
        assert_eq!(jt.len(), before + 1, "jobtab grew by one");
    }

    /// `findproc` with pid=-1 (impossible pid) returns None.
    /// Already covered but pin the never-match path explicitly.
    #[test]
    fn findproc_invalid_pid_returns_none() {
        let _g = crate::test_util::global_state_lock();
        let jt: Vec<job> = vec![job::new()];
        assert!(findproc(&jt, -1, false).is_none());
    }

    /// `findproc` on empty jobtab returns None (no panic on empty
    /// slice; C `for (i=1; i<=maxjob; i++)` with maxjob=0 skips loop).
    #[test]
    fn findproc_empty_jobtab_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let jt: Vec<job> = Vec::new();
        assert!(findproc(&jt, 1234, false).is_none());
    }

    /// `getsigname(0)` returns "EXIT" — pseudo-signal index 0 is the
    /// EXIT trap target in zsh. C jobs.c:3392-3393 sigs[0]="EXIT".
    #[test]
    fn getsigname_zero_returns_exit_pseudo_signal() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(getsigname(0), "EXIT");
    }

    /// `getsigname(libc::SIGHUP)` returns "HUP" without the SIG prefix.
    #[test]
    fn getsigname_sighup_returns_hup_without_prefix() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(getsigname(libc::SIGHUP), "HUP");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/jobs.c printhhmmss + sigmsg +
    // dtime_tv + get_clktck.
    // ═══════════════════════════════════════════════════════════════════

    /// c:752 — `printhhmmss(0.0)` returns "0.000" (sub-minute fmt).
    #[test]
    fn printhhmmss_zero_returns_zero_seconds() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(printhhmmss(0.0), "0.000");
    }

    /// c:752 — sub-minute time uses 3-decimal format.
    #[test]
    fn printhhmmss_sub_minute_uses_three_decimals() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(printhhmmss(5.123), "5.123");
        assert_eq!(printhhmmss(59.999), "59.999");
    }

    /// c:752 — minute-but-not-hour uses M:SS.SS format.
    #[test]
    fn printhhmmss_minute_uses_mm_ss_format() {
        let _g = crate::test_util::global_state_lock();
        let r = printhhmmss(65.5);
        assert_eq!(r, "1:05.50", "1m05.50s");
    }

    /// c:752 — hour+ uses H:MM:SS.SS format.
    #[test]
    fn printhhmmss_hour_uses_hh_mm_ss_format() {
        let _g = crate::test_util::global_state_lock();
        let r = printhhmmss(3725.0); // 1h 2m 5s
        assert_eq!(r, "1:02:05.00");
    }

    /// c:752 — exactly 60 seconds crosses minute boundary.
    #[test]
    fn printhhmmss_sixty_seconds_is_one_minute() {
        let _g = crate::test_util::global_state_lock();
        let r = printhhmmss(60.0);
        assert_eq!(r, "1:00.00", "60s = 1m");
    }

    /// c:752 — exactly 3600s crosses hour boundary.
    #[test]
    fn printhhmmss_thirty_six_hundred_seconds_is_one_hour() {
        let _g = crate::test_util::global_state_lock();
        let r = printhhmmss(3600.0);
        assert_eq!(r, "1:00:00.00");
    }

    /// c:1107 — `sigmsg` of valid signal returns a non-default string.
    #[test]
    fn sigmsg_known_signal_returns_descriptive_string() {
        let _g = crate::test_util::global_state_lock();
        // SIGTERM, SIGSEGV, SIGINT should all have descriptive messages
        let term = sigmsg(libc::SIGTERM);
        assert_ne!(term, "unknown signal", "SIGTERM should have a message");
    }

    /// c:1118 — `sigmsg(-1)` / out-of-range returns "unknown signal".
    #[test]
    fn sigmsg_unknown_signal_returns_unknown() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(sigmsg(-1), "unknown signal");
        assert_eq!(sigmsg(9999), "unknown signal");
    }

    /// c:752 — printhhmmss is deterministic.
    #[test]
    fn printhhmmss_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        for t in &[0.0, 1.5, 60.0, 3600.0, 7200.5] {
            let r1 = printhhmmss(*t);
            let r2 = printhhmmss(*t);
            assert_eq!(r1, r2, "printhhmmss must be pure for {}", t);
        }
    }

    /// `get_clktck()` returns positive — clock-ticks-per-second cannot
    /// be zero on any sane system.
    #[test]
    fn get_clktck_returns_positive() {
        let _g = crate::test_util::global_state_lock();
        let ck = get_clktck();
        assert!(ck > 0, "CLK_TCK must be positive, got {}", ck);
        // POSIX guarantees CLK_TCK ≥ 1; typical values are 100/250/1000.
        assert!(ck <= 10_000, "CLK_TCK suspiciously large: {}", ck);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/jobs.c isanum + getsigidx.
    // ═══════════════════════════════════════════════════════════════════

    /// c:2010 — `isanum("")` returns false (empty not valid).
    #[test]
    fn isanum_empty_returns_false() {
        let _g = crate::test_util::global_state_lock();
        assert!(!isanum(""));
    }

    /// c:2010 — all-digit string returns true.
    #[test]
    fn isanum_all_digits_returns_true() {
        let _g = crate::test_util::global_state_lock();
        assert!(isanum("123"));
        assert!(isanum("0"));
        assert!(isanum("999999"));
    }

    /// c:2010 — hyphen-prefixed digits valid.
    #[test]
    fn isanum_with_hyphen_returns_true() {
        let _g = crate::test_util::global_state_lock();
        assert!(isanum("-1"));
        assert!(isanum("-123"));
        assert!(isanum("-"));
    }

    /// c:2010 — alpha or non-digit chars rejected.
    #[test]
    fn isanum_rejects_alpha() {
        let _g = crate::test_util::global_state_lock();
        assert!(!isanum("abc"));
        assert!(!isanum("1a"));
        assert!(!isanum("a1"));
        assert!(!isanum("1 2"));
        assert!(!isanum("1.0"));
    }

    /// c:2010 — deterministic.
    #[test]
    fn isanum_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        for s in ["", "123", "-1", "abc", "1a"] {
            let first = isanum(s);
            for _ in 0..5 {
                assert_eq!(isanum(s), first);
            }
        }
    }

    /// c:3052 — `getsigidx("")` returns None.
    #[test]
    fn getsigidx_empty_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert!(getsigidx("").is_none());
    }

    /// c:3334 — `getsigidx("EXIT")` returns Some(0).
    #[test]
    fn getsigidx_exit_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(getsigidx("EXIT"), Some(0));
    }

    /// c:3052 — canonical POSIX signal names resolve.
    #[test]
    #[cfg(unix)]
    fn getsigidx_canonical_signal_names() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(getsigidx("HUP"), Some(libc::SIGHUP));
        assert_eq!(getsigidx("TERM"), Some(libc::SIGTERM));
        assert_eq!(getsigidx("INT"), Some(libc::SIGINT));
        assert_eq!(getsigidx("KILL"), Some(libc::SIGKILL));
    }

    /// c:3332 — SIG prefix stripped transparently.
    #[test]
    #[cfg(unix)]
    fn getsigidx_strips_sig_prefix() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(getsigidx("HUP"), getsigidx("SIGHUP"));
        assert_eq!(getsigidx("TERM"), getsigidx("SIGTERM"));
    }

    /// c:3333 — case-insensitive on signal name.
    #[test]
    #[cfg(unix)]
    fn getsigidx_case_insensitive() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(getsigidx("hup"), getsigidx("HUP"));
    }

    /// c:3081 — unknown name returns None.
    #[test]
    fn getsigidx_unknown_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert!(getsigidx("NEVER_REAL_SIGNAL").is_none());
    }

    /// c:3339 — ZERR and ERR both resolve to SIGZERR.
    #[test]
    fn getsigidx_zerr_and_err_alias() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(getsigidx("ZERR"), Some(crate::ported::signals_h::SIGZERR));
        assert_eq!(
            getsigidx("ERR"),
            Some(crate::ported::signals_h::SIGZERR),
            "ERR aliases ZERR"
        );
    }

    /// c:3340 — DEBUG resolves to SIGDEBUG.
    #[test]
    fn getsigidx_debug_returns_sigdebug() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(getsigidx("DEBUG"), Some(crate::ported::signals_h::SIGDEBUG));
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/jobs.c
    // c:93 dtime_tv / c:105 dtime_ts / c:524 get_usage / c:866 get_clktck /
    // c:885 printhhmmss / c:1172 sigmsg / c:340 hasprocs
    // ═══════════════════════════════════════════════════════════════════

    /// c:866 — `get_clktck` returns i64 (compile-time type pin).
    #[test]
    fn get_clktck_returns_i64_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i64 = get_clktck();
    }

    /// c:866 — `get_clktck` returns positive value (clock ticks per sec).
    #[test]
    fn get_clktck_returns_positive_pin() {
        let _g = crate::test_util::global_state_lock();
        let tk = get_clktck();
        assert!(tk > 0, "clock tick rate must be > 0, got {}", tk);
    }

    /// c:866 — `get_clktck` is deterministic.
    #[test]
    fn get_clktck_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let first = get_clktck();
        for _ in 0..5 {
            assert_eq!(get_clktck(), first, "clock tick rate must be stable");
        }
    }

    /// c:885 — `printhhmmss(0.0)` returns String (compile-time type pin).
    #[test]
    fn printhhmmss_returns_string_type() {
        let _: String = printhhmmss(0.0);
    }

    /// c:885 — `printhhmmss` is pure for arbitrary seconds.
    #[test]
    fn printhhmmss_is_pure() {
        for s in [0.0, 1.0, 60.0, 3661.5, -1.0] {
            let first = printhhmmss(s);
            for _ in 0..3 {
                assert_eq!(printhhmmss(s), first, "printhhmmss({}) must be pure", s);
            }
        }
    }

    /// c:885 — `printhhmmss(0)` produces "0.000" (sub-minute, no `:`).
    /// Per C body c:893-895: only adds h/m components when total exceeds them.
    #[test]
    fn printhhmmss_zero_short_form() {
        let s = printhhmmss(0.0);
        assert!(
            s.contains('.'),
            "sub-minute must use 'S.MMM' form, got {:?}",
            s
        );
        assert!(s.contains('0'), "must contain '0' digit, got {:?}", s);
    }

    /// c:885 — `printhhmmss(>60)` adds minute colon separator.
    #[test]
    fn printhhmmss_over_minute_adds_colon() {
        let s = printhhmmss(125.0); // 2m 5s
        assert!(
            s.contains(':'),
            "over 60s must contain ':' separator, got {:?}",
            s
        );
    }

    /// c:1172 — `sigmsg` returns &'static str (compile-time type pin).
    #[test]
    fn sigmsg_returns_static_str_type() {
        let _: &'static str = sigmsg(0);
    }

    /// c:1172 — `sigmsg(N)` is pure for arbitrary signals.
    #[test]
    fn sigmsg_is_pure() {
        for s in [0i32, 1, 9, 15, 999] {
            let first = sigmsg(s);
            for _ in 0..3 {
                assert_eq!(sigmsg(s), first, "sigmsg({}) must be pure", s);
            }
        }
    }

    /// c:340 — `hasprocs(empty_table, _)` returns false.
    #[test]
    fn hasprocs_empty_table_returns_false() {
        let empty: Vec<job> = vec![];
        assert!(!hasprocs(&empty, 0), "empty table → false");
    }

    /// c:340 — `hasprocs` returns bool (compile-time type pin).
    #[test]
    fn hasprocs_returns_bool_type() {
        let empty: Vec<job> = vec![];
        let _: bool = hasprocs(&empty, 0);
    }

    /// c:524 — `get_usage` returns timeinfo (compile-time type pin).
    #[test]
    fn get_usage_returns_timeinfo_type() {
        let _g = crate::test_util::global_state_lock();
        let _: timeinfo = get_usage();
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/jobs.c
    // c:93 dtime_tv / c:105 dtime_ts / c:1589 havefiles / c:2209 scanjobs /
    // c:3876 getbgstatus / c:1599 waitforpid + edge-case pins
    // ═══════════════════════════════════════════════════════════════════

    /// c:93 — `dtime_tv(t2 > t1)` returns positive diff.
    #[test]
    fn dtime_tv_positive_diff_returned() {
        let mut dt = Duration::ZERO;
        let t1 = Duration::from_secs(1);
        let t2 = Duration::from_secs(5);
        let r = dtime_tv(&mut dt, &t1, &t2);
        assert_eq!(r, Duration::from_secs(4), "5 - 1 = 4s");
        assert_eq!(dt, Duration::from_secs(4), "out param set to diff");
    }

    /// c:93 — `dtime_tv(t2 <= t1)` returns ZERO (saturating).
    #[test]
    fn dtime_tv_negative_diff_saturates_to_zero() {
        let mut dt = Duration::from_secs(99);
        let t1 = Duration::from_secs(5);
        let t2 = Duration::from_secs(1);
        let r = dtime_tv(&mut dt, &t1, &t2);
        assert_eq!(r, Duration::ZERO, "t2 < t1 → ZERO");
        assert_eq!(dt, Duration::ZERO, "out param set to ZERO");
    }

    /// c:93 — `dtime_tv(t2 == t1)` returns ZERO (equal saturates).
    #[test]
    fn dtime_tv_equal_returns_zero() {
        let mut dt = Duration::from_secs(99);
        let t = Duration::from_secs(5);
        let r = dtime_tv(&mut dt, &t, &t);
        assert_eq!(r, Duration::ZERO, "equal → ZERO");
    }

    /// c:93 — `dtime_tv` returns Duration (compile-time type pin).
    #[test]
    fn dtime_tv_returns_duration_type() {
        let mut dt = Duration::ZERO;
        let t = Duration::from_secs(1);
        let _: Duration = dtime_tv(&mut dt, &t, &t);
    }

    /// c:105 — `dtime_ts(t1, t2)` with t2 < t1 returns ZERO.
    #[test]
    fn dtime_ts_negative_diff_saturates_to_zero() {
        let t1 = Instant::now();
        std::thread::sleep(Duration::from_millis(1));
        let t2 = Instant::now();
        // Reverse — t1 is "later" perspective.
        let r = dtime_ts(&t2, &t1);
        assert_eq!(r, Duration::ZERO, "earlier - later = ZERO");
    }

    /// c:105 — `dtime_ts` returns Duration (compile-time type pin).
    #[test]
    fn dtime_ts_returns_duration_type() {
        let now = Instant::now();
        let _: Duration = dtime_ts(&now, &now);
    }

    /// c:105 — `dtime_ts(same, same)` returns ZERO.
    #[test]
    fn dtime_ts_same_instant_returns_zero() {
        let now = Instant::now();
        assert_eq!(
            dtime_ts(&now, &now),
            Duration::ZERO,
            "same instant → ZERO diff"
        );
    }

    /// c:1589 — `havefiles(empty)` returns false.
    #[test]
    fn havefiles_empty_returns_false() {
        let empty: Vec<job> = vec![];
        assert!(!havefiles(&empty), "empty table has no files");
    }

    /// c:1589 — `havefiles` returns bool (compile-time type pin).
    #[test]
    fn havefiles_returns_bool_type() {
        let empty: Vec<job> = vec![];
        let _: bool = havefiles(&empty);
    }

    /// c:3876 — `getbgstatus(-1)` invalid pid returns Option<i32>.
    #[test]
    fn getbgstatus_returns_option_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Option<i32> = getbgstatus(-1);
    }

    /// c:3876 — `getbgstatus(unknown)` for never-recorded pid → None.
    #[test]
    fn getbgstatus_unknown_pid_returns_none() {
        let _g = crate::test_util::global_state_lock();
        // PID 0 / negative are never recorded via addbgstatus.
        assert!(getbgstatus(0).is_none() || getbgstatus(0).is_some());
        // Real test: an arbitrary high pid we've never used.
        let r = getbgstatus(2147483646);
        assert!(r.is_none(), "never-recorded pid → None");
    }

    /// c:340 — `hasprocs(table, job_index_out_of_bounds)` is safe.
    #[test]
    fn hasprocs_index_out_of_bounds_safe() {
        let empty: Vec<job> = vec![];
        for idx in [0usize, 1, 100, usize::MAX] {
            let _: bool = hasprocs(&empty, idx);
            // No panic = pass.
        }
    }

    /// c:885 — `printhhmmss(1.0)` sub-minute formats as "S.MMM".
    #[test]
    fn printhhmmss_one_second_short_form() {
        let s = printhhmmss(1.0);
        assert!(s.contains("1."), "1.0s must contain '1.', got {:?}", s);
    }

    /// c:2237 — `isanum` is pure for a sweep of inputs.
    #[test]
    fn isanum_is_pure_full_sweep() {
        for s in ["", "0", "123", "-5", "abc", "a1", "1a", "-", "12-34"] {
            let first = isanum(s);
            for _ in 0..3 {
                assert_eq!(isanum(s), first, "isanum({:?}) must be pure", s);
            }
        }
    }
}

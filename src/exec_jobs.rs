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

use crate::ported::jobs::stat;
use crate::ported::jobs::{deletejob, CURJOB, MAXJOB, PREVJOB, THISJOB};
use crate::ported::zsh_h::job;

/// !!! WARNING: RUST-ONLY ADAPTER !!! The job status C derives from a
/// finished process's wait status — update_job's `val`
/// (c:Src/jobs.c:492-496): `WIFSIGNALED ? 0200 | WTERMSIG : WIFSTOPPED ?
/// 0200 | WSTOPSIG : WEXITSTATUS`. zshrs waits on its foreground externals
/// through `std::process::ExitStatus`, whose `code()` is `None` for a
/// signalled child; the call sites turned that into 1, so
/// `sh -c 'kill $$'; print $?` printed 1 where zsh prints 143.
pub fn wait_status_val(s: std::process::ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt as _;
    if let Some(sig) = s.signal() {
        0o200 | sig // c:493
    } else if let Some(sig) = s.stopped_signal() {
        0o200 | sig // c:495
    } else {
        s.code().unwrap_or(0) // c:496
    }
}

thread_local! {
    /// Non-zero while a pipeline's in-shell last stage runs. update_job's
    /// foreground tail reads the status of the job's process-group leader
    /// (c:Src/jobs.c:497-498), which is the FIRST stage, so the externals
    /// the last stage waits for do not drive it.
    ///
    /// !!! WARNING: RUST-ONLY COUNTER — C READS `jn->gleader` !!!
    pub static NOT_JOB_LEADER: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// !!! WARNING: RUST-ONLY HELPER — C WRITES THROUGH THE `shout` FILE* !!!
/// printjob's asynchronous report goes to `fout = (synch == 2 || !shout) ?
/// stdout : shout` (c:Src/jobs.c:1153), and an interactive shell's `shout`
/// is the terminal, `fdopen(SHTTY, "w")`, or stderr when there is no tty
/// (c:Src/init.c:734-748). Writing to fd 2 instead sent the report through
/// the command's own redirection: `sh -c 'kill -HUP $$' 2>/dev/null` lost
/// its `zsh: hangup` line.
fn shout_write(s: &str) {
    let tty = crate::ported::init::SHTTY.load(std::sync::atomic::Ordering::Relaxed);
    let fd = if tty >= 0 { tty } else { libc::STDERR_FILENO };
    let _ = crate::ported::utils::write_loop(fd, s.as_bytes());
}

/// !!! WARNING: RUST-ONLY ADAPTER !!! update_job's report for the foreground
/// job (c:Src/jobs.c:645-650), for a simple external zshrs waited for outside
/// the job table: `if ((isset(NOTIFY) || job == thisjob) && (jn->stat &
/// STAT_LOCKED)) printjob(jn, !!isset(LONGLISTJOBS), 0);`. The job is one
/// process whose text is the command's job text (`None` when the dispatch
/// carried none, e.g. inside a function, c:Src/exec.c:3536-3538).
///
/// printjob prints nothing for a job marked STAT_NOPRINT, and execpline marks
/// every job run while another pipeline is executing (`pline_level`,
/// c:Src/exec.c:1829-1831): a command inside a function, `eval`, a loop, a
/// `{ … }` or an `if` is never reported, only one at the top of the command
/// line — including either side of `&&`/`||`. `nested` is the caller's
/// in-process `( … )` (C forks it, and the child has no MONITOR).
///
/// Then printjob's own gate (exec_jobs::printjob_print): an interactive
/// shell with MONITOR reports a signal other than SIGINT/SIGPIPE as
/// `zsh: terminated  cmd`, and a SIGINT only with a newline (c:1203-1205,
/// c:1260-1263, c:1338-1341).
pub fn foreground_job_report(status: i32, text: Option<String>, nested: bool) {
    use crate::ported::zsh_h::{CS_CMDAND, CS_CMDOR, SFC_NONE};
    use std::sync::atomic::Ordering;
    let Some(text) = text else { return };
    if nested
        || !libc::WIFSIGNALED(status)
        || NOT_JOB_LEADER.with(|d| d.get()) > 0
        || crate::ported::exec::sfcontext.load(Ordering::Relaxed) != SFC_NONE
        || crate::vm_helper::EVAL_RECURSION_DEPTH.with(|d| d.get()) > 0
    {
        return;
    }
    // c:Src/exec.c:1829-1831 — nested in a compound command (cmdstack holds
    // more than the `&&`/`||` list connectors).
    let in_compound = crate::ported::prompt::CMDSTACK.with(|s| {
        s.borrow()
            .iter()
            .any(|&cs| cs as i32 != CS_CMDAND as i32 && cs as i32 != CS_CMDOR as i32)
    });
    if in_compound {
        return;
    }
    let jn = job {
        stat: stat::INUSE | stat::LOCKED | stat::DONE | stat::CHANGED,
        procs: vec![crate::ported::zsh_h::process {
            pid: 0,
            text,
            status,
            ti: Default::default(),
            bgtime: None,
            endtime: None,
        }],
        ..Default::default()
    };
    // The job sits at index 1 and is `thisjob` (c:Src/jobs.c:645).
    let mut tab = vec![job::default(), jn];
    printjob_print(&mut tab, 1, i32::from(crate::ported::zsh_h::isset(crate::ported::zsh_h::LONGLISTJOBS)), 0, 1);
}

/// !!! WARNING: RUST-ONLY ADAPTER !!! The tail of C `update_job` for the
/// FOREGROUND job (`job == thisjob`, not STAT_CURSH, so `inforeground = 2`,
/// c:Src/jobs.c:558-561), for an external zshrs waited for outside the job
/// table. `status` is the raw wait status of the job's last process.
///
/// c:Src/jobs.c:654-679 —
/// ```c
/// /* When MONITOR is set, the foreground process runs in a different *
///  * process group from the shell, so the shell will not receive     *
///  * terminal signals, therefore we pretend that the shell got       *
///  * the signal too.                                                 */
/// if (inforeground == 2 && isset(MONITOR) && WIFSIGNALED(status)) {
///     int sig = WTERMSIG(status);
///     if (sig == SIGINT || sig == SIGQUIT) {
///         if (sigtrapped[sig]) {
///             dotrap(sig);
///             if (errflag) breaks = loops;
///         } else {
///             breaks = loops;
///             errflag |= ERRFLAG_INT;
///         }
///         check_cursh_sig(sig);
///     }
/// }
/// ```
/// So a child that dies of SIGINT/SIGQUIT ends the rest of the command line
/// (`for i in 1 2; do sh -c 'kill -INT $$'; print $i; done` prints nothing)
/// unless a trap for the signal returns zero.
pub fn foreground_job_signalled(status: i32) {
    use crate::ported::zsh_h::{isset, ERRFLAG_INT, MONITOR};
    use std::sync::atomic::Ordering;
    if NOT_JOB_LEADER.with(|d| d.get()) > 0 || !isset(MONITOR) || !libc::WIFSIGNALED(status) {
        return;
    }
    let sig = libc::WTERMSIG(status); // c:659
    if sig != libc::SIGINT && sig != libc::SIGQUIT {
        return; // c:661
    }
    let loops = crate::ported::builtin::LOOPS.load(Ordering::Relaxed);
    let trapped = crate::ported::signals::sigtrapped
        .lock()
        .ok()
        .and_then(|t| t.get(sig as usize).copied())
        .unwrap_or(0);
    if trapped != 0 {
        crate::ported::signals::dotrap(sig); // c:663
        if crate::ported::utils::errflag.load(Ordering::Relaxed) != 0 {
            crate::ported::builtin::BREAKS.store(loops, Ordering::Relaxed); // c:671-672
        }
    } else {
        crate::ported::builtin::BREAKS.store(loops, Ordering::Relaxed); // c:674
        crate::ported::utils::errflag.fetch_or(ERRFLAG_INT, Ordering::Relaxed); // c:675
    }
    // c:677 — `check_cursh_sig(sig);`
    if let Some(tab) = crate::ported::jobs::JOBTAB.get() {
        if let Ok(jt) = tab.lock() {
            crate::ported::jobs::check_cursh_sig(&jt, sig);
        }
    }
}

/// !!! WARNING: RUST-ONLY SPLIT OF `printjob` — C HAS ONE FUNCTION !!!
/// Port of the done-job tail of C `printjob`, `Src/jobs.c:1350-1363`:
/// ```c
/// if (jn->stat & STAT_DONE) {
///     /* This looks silly, but see update_job() */
///     if (synch <= 1)
///         storepipestats(jn, job == thisjob, job == thisjob);
///     if (should_report_time(jn))
///         dumptime(jn);
///     deletejob(jn, 0);
///     if (job == curjob) { curjob = prevjob; prevjob = job; }
///     if (job == prevjob) setprevjob();
/// }
/// ```
/// `jn` is the table index of the job being deleted and `job` the number
/// printjob was called with: they differ when printjob reported a subjob in
/// place of its superjob (c:1171-1185). Callers that formatted the job
/// themselves (the `wait` builtin) run this tail alone, as C's update_job
/// reaches it through printjob.
///
/// `tab` is the caller's locked slice of `JOBTAB`; nothing here locks it.
pub fn printjob_delete_tail(tab: &mut [job], jn: usize, job: usize, synch: i32) {
    if jn >= tab.len() || (tab[jn].stat & stat::DONE) == 0 {
        return;
    }
    let thisjob = *THISJOB.get_or_init(|| Mutex::new(-1)).lock().unwrap_or_else(|e| e.into_inner());
    let is_thisjob = i32::from(job as i32 == thisjob);
    if synch <= 1 {
        crate::ported::jobs::storepipestats(&tab[jn], is_thisjob, is_thisjob); // c:1352
    }
    // c:1354-1355 — `if (should_report_time(jn)) dumptime(jn);`
    let reporttime: f64 = crate::ported::params::getsparam("REPORTTIME")
        .and_then(|s| s.parse().ok())
        .unwrap_or(-1.0);
    if crate::ported::jobs::should_report_time(&tab[jn], reporttime) {
        if let Some(timing) = crate::ported::jobs::dumptime(&tab[jn]) {
            eprintln!("{}", timing); // printtime writes to stderr
        }
    }
    deletejob(tab, jn, false); // c:1356
    let mut cj = CURJOB.get_or_init(|| Mutex::new(-1)).lock().unwrap_or_else(|e| e.into_inner());
    let mut pj = PREVJOB.get_or_init(|| Mutex::new(-1)).lock().unwrap_or_else(|e| e.into_inner());
    if *cj == job as i32 {
        // c:1357-1360
        *cj = *pj;
        *pj = job as i32;
    }
    let need_setprev = *pj == job as i32; // c:1361
    drop(cj);
    drop(pj);
    if need_setprev {
        setprevjob_locked(tab); // c:1362
    }
}

/// !!! WARNING: RUST-ONLY SPLIT OF `printjob` — C HAS ONE FUNCTION !!!
/// Port of C `printjob(Job jn, int lng, int synch)`, `Src/jobs.c:1147-1365`,
/// up to but excluding the done-job tail: the superjob redirect
/// (c:1171-1185), the scan that decides whether a state change is worth
/// reporting (c:1187-1221), the print gate and the status lines
/// (c:1226-1337), and the `(pwd now: …)` line (c:1344-1353). The lines
/// themselves are laid out by `ported::jobs::printjob`. Returns
/// `(doneprint, jn, skip_print)` where `jn` is the index of the job reported
/// (the subjob when it stood in for the superjob).
///
/// `ji` is C's `job` (`jn - jobtab` before the redirect), `thisjob` C's global.
///
/// `lng` is never negative here and `synch` is 0 or 1 or 2: the `fg`/`bg`
/// "continued" and POSIX plain-format arms (`lng < 0`, `synch == 3`) have
/// no caller that reaches this function.
///
/// WARNING: param names don't match C — Rust=(tab, ji, lng, synch, thisjob)
/// vs C=(jn, lng, synch)
pub fn printjob_print(tab: &mut [job], ji: usize, lng: i32, synch: i32, thisjob: i32) -> (bool, usize, bool) {
    use crate::ported::builtins::sched::zleactive;
    use crate::ported::zsh_h::{isset, INTERACTIVE, PRINTEXITVALUE, SHINSTDIN, SP_RUNNING};
    use std::sync::atomic::Ordering;
    let is_thisjob = ji as i32 == thisjob; // `job == thisjob`
    // c:1163-1164 — `if (jn->stat & STAT_NOPRINT) skip_print = 1;`
    let mut skip_print = (tab[ji].stat & stat::NOPRINT) != 0;
    let mut jn = ji;
    // c:1171-1185 — a subjob that still has processes is reported as if it
    // were the user-visible superjob.
    if (tab[jn].stat & stat::SUPERJOB) != 0 && tab[jn].other != 0 {
        let sjn = tab[jn].other as usize;
        if sjn < tab.len() && (!tab[sjn].procs.is_empty() || !tab[sjn].auxprocs.is_empty()) {
            jn = sjn;
        }
    }
    let mut sflag = false; // c:1150
    let mut doputnl = false; // c:1151
    let superjob = (tab[jn].stat & stat::SUPERJOB) != 0;
    let nprocs = tab[jn].procs.len();
    // c:1187-1221 — does any finished process force a report?
    for k in 0..nprocs {
        if superjob && tab[jn].procs[0].status == SP_RUNNING && k + 1 == nprocs {
            tab[jn].procs[k].status = SP_RUNNING; // c:1192-1194
        }
        let status = tab[jn].procs[k].status;
        if status == SP_RUNNING {
            continue; // c:1195
        }
        if libc::WIFSIGNALED(status) {
            let sig = libc::WTERMSIG(status); // c:1197
            if sig != libc::SIGINT && sig != libc::SIGPIPE {
                sflag = true; // c:1202-1203
            }
            if is_thisjob && sig == libc::SIGINT {
                doputnl = true; // c:1204-1205
            }
            if isset(PRINTEXITVALUE) && isset(SHINSTDIN) {
                sflag = true; // c:1206-1208
                skip_print = false;
            }
        } else if libc::WIFSTOPPED(status) {
            let sig = libc::WSTOPSIG(status); // c:1210
            if is_thisjob && sig == libc::SIGTSTP {
                doputnl = true; // c:1214-1215
            }
        } else if isset(PRINTEXITVALUE) && isset(SHINSTDIN) && libc::WEXITSTATUS(status) != 0 {
            sflag = true; // c:1216-1219
            skip_print = false;
        }
    }
    // c:1224-1238 — a skipped job is only deleted (the caller's tail).
    if skip_print {
        return (false, jn, true);
    }
    let mut doneprint = false; // c:1152
    let interact = isset(INTERACTIVE);
    let stopped = (tab[jn].stat & stat::STOPPED) != 0;
    // c:1248-1250 — `synch == 2 || ((interact || synch) && jobbing &&
    //                ((jn->stat & STAT_STOPPED) || sflag || job != thisjob))`
    if synch == 2
        || ((interact || synch != 0) && crate::ported::zsh_h::jobbing() && (stopped || sflag || !is_thisjob))
    {
        // c:1258-1259 — `if (!synch) zleentry(ZLE_CMD_TRASH);` (trashzle
        // itself is a no-op unless zleactive).
        if synch == 0 && zleactive.load(Ordering::Relaxed) != 0 {
            crate::ported::init::zleentry(crate::ported::zsh_h::ZLE_CMD_TRASH);
        }
        let mut out = String::new();
        if doputnl && synch == 0 {
            doneprint = true; // c:1262-1263
            out.push('\n');
        }
        let curjob = *CURJOB.get_or_init(|| Mutex::new(-1)).lock().unwrap_or_else(|e| e.into_inner());
        let prevjob = *PREVJOB.get_or_init(|| Mutex::new(-1)).lock().unwrap_or_else(|e| e.into_inner());
        let s = crate::ported::jobs::printjob(
            &tab[jn],
            ji,
            lng,
            (curjob >= 0).then_some(curjob as usize),
            (prevjob >= 0).then_some(prevjob as usize),
            is_thisjob && synch != 2, // c:1255 — `thisfmt = job == thisjob && synch != 2`
        );
        if !s.is_empty() {
            doneprint = true; // c:1275
            out.push_str(&s);
            out.push('\n');
        }
        shout_write(&out);
    } else if doputnl && interact && synch == 0 {
        // c:1338-1341
        doneprint = true;
        shout_write("\n");
    }
    // c:1344-1353 — `(pwd now: …)` once a later `cd` has moved the shell away
    // from the directory the job started in (`jobs -d`, lng & 4, is the
    // layout routine's).
    if (lng & 4) == 0 && interact && is_thisjob {
        if let Some(jpwd) = tab[jn].pwd.as_deref() {
            let pwd = crate::ported::params::getsparam("PWD").unwrap_or_default();
            if jpwd != pwd {
                doneprint = true;
                shout_write(&format!("(pwd now: {})\n", crate::ported::utils::fprintdir(&pwd)));
            }
        }
    }
    (doneprint, jn, false)
}

/// !!! WARNING: RUST-ONLY SPLIT OF `printjob` — C HAS ONE FUNCTION !!!
/// Port of C `printjob(Job jn, int lng, int synch)`, `Src/jobs.c:1147-1365`,
/// reading `thisjob` from the global: [`printjob_print`] followed by the
/// done-job tail ([`printjob_delete_tail`]) or, for a job that is not done,
/// `jn->stat &= ~STAT_CHANGED` (c:1364). Returns C's `doneprint`.
///
/// `tab` is the caller's locked slice of `JOBTAB`; nothing here locks it, so
/// update_job, handle_sub, zwaitjob and scanjobs call it with their guard
/// held.
///
/// WARNING: param names don't match C — Rust=(tab, ji, lng, synch) vs
/// C=(jn, lng, synch)
pub fn printjob_synch(tab: &mut [job], ji: usize, lng: i32, synch: i32) -> bool {
    let thisjob = *THISJOB.get_or_init(|| Mutex::new(-1)).lock().unwrap_or_else(|e| e.into_inner());
    let (doneprint, jn, skipped) = printjob_print(tab, ji, lng, synch, thisjob);
    if (tab[jn].stat & stat::DONE) != 0 {
        printjob_delete_tail(tab, jn, ji, synch);
    } else if !skipped {
        tab[jn].stat &= !stat::CHANGED; // c:1364
    }
    doneprint
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
            if stat_ok && (j.stat & stat::SUBJOB) == 0 && i as i32 != curjob && i as i32 != thisjob
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

/// One job slot held by a running synchronous pipeline.
///
/// !!! WARNING: RUST-ONLY ADAPTER !!! C keeps this on the C stack:
/// execpline's locals `pj` and `newjob` (c:Src/exec.c:1637,1656,1666) live
/// exactly as long as the pipeline runs. A compiled chunk has no such frame
/// around the pipeline, and can jump past its end (`break`, `return`, an
/// errflag abort), so the slot is recorded here with the VM that opened it
/// (`frame`) and the pipeline's nesting depth inside that chunk (`depth`).
struct HeldSlot {
    frame: usize,
    depth: u32,
    slot: usize,
    pj: i32,
}

thread_local! {
    /// The job slots of the synchronous pipelines running on this thread,
    /// outermost first. See [`HeldSlot`].
    static HELD_SLOTS: std::cell::RefCell<Vec<HeldSlot>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// !!! WARNING: RUST-ONLY ADAPTER !!! The opening half of execpline's job
/// frame, run by BUILTIN_EXECPLINE_CHILD_BLOCK:
/// ```c
/// pj = thisjob;                                  /* c:Src/exec.c:1656 */
/// if ((thisjob = newjob = initjob()) == -1) {   /* c:Src/exec.c:1666 */
/// ```
/// Every synchronous pipeline that is not `execsimple` holds a slot while
/// it runs, including a function call and a builtin. That is visible:
/// a job started inside a function gets the slot after the function's
/// own (`f() { sleep 1 & }; f` prints `[2]`), and setprevjob
/// (c:Src/jobs.c:672-677) can pick the running function's slot as
/// `prevjob`, which deletejob later frees without touching `prevjob`.
///
/// Slots this frame left at `depth` or deeper — skipped by a `break` or a
/// `continue` before their close ran — are released first.
///
/// Worker threads take no slot: the job table is the main shell's, and C
/// has no second thread to allocate from it.
pub fn execpline_slot_open(frame: usize, depth: u32) {
    if crate::worker::in_worker_thread() {
        return;
    }
    execpline_slot_close(frame, depth);
    let table = crate::ported::jobs::JOBTAB.get_or_init(|| Mutex::new(Vec::new()));
    let slot = {
        let mut tab = table.lock().unwrap_or_else(|e| e.into_inner());
        crate::ported::jobs::initjob(&mut tab) // c:Src/exec.c:1666
    };
    let mut tj = THISJOB.get_or_init(|| Mutex::new(-1)).lock().unwrap_or_else(|e| e.into_inner());
    let pj = *tj; // c:Src/exec.c:1656
    *tj = slot as i32; // c:Src/exec.c:1666
    drop(tj);
    HELD_SLOTS.with(|h| h.borrow_mut().push(HeldSlot { frame, depth, slot, pj }));
}

/// !!! WARNING: RUST-ONLY ADAPTER !!! The closing half of execpline's job
/// frame, run by BUILTIN_EXECPLINE_CHILD_UNBLOCK: the procs-less job is
/// deleted (waitonejob, c:Src/jobs.c:1750-1756) and `thisjob = pj` restores
/// the enclosing pipeline's (c:Src/exec.c:1981).
///
/// Closes the slot `frame` holds at `depth` together with every slot opened
/// after it: those belong to pipelines nested inside this one, which have
/// necessarily finished, whether or not their own close ran.
pub fn execpline_slot_close(frame: usize, depth: u32) {
    let start = HELD_SLOTS.with(|h| {
        h.borrow()
            .iter()
            .position(|s| s.frame == frame && s.depth >= depth)
    });
    if let Some(start) = start {
        release_held_slots(start);
    }
}

/// !!! WARNING: RUST-ONLY ADAPTER !!! How many pipeline slots are held now.
/// A chunk runner takes this before running a chunk and hands it to
/// [`execpline_slots_release_to`] afterwards, so a chunk abandoned mid-
/// pipeline (errflag, `return` from a sourced file) holds nothing after it.
pub fn execpline_slots_mark() -> usize {
    HELD_SLOTS.with(|h| h.borrow().len())
}

/// !!! WARNING: RUST-ONLY ADAPTER !!! Close every pipeline slot opened since
/// `mark`, as their closes would have.
pub fn execpline_slots_release_to(mark: usize) {
    release_held_slots(mark);
}

/// !!! WARNING: RUST-ONLY ADAPTER !!! Drop the record of every slot opened
/// since `mark` WITHOUT touching the job table. For the in-process stand-ins
/// for a fork (`$(...)`, `( ... )`): the slots were taken from the child's
/// cleared table, which the parent's restored table has since replaced, so
/// the indices name nothing of the child's any more.
pub fn execpline_slots_forget_to(mark: usize) {
    HELD_SLOTS.with(|h| h.borrow_mut().truncate(mark));
}

/// Port of execcmd_exec's current-shell arm (c:Src/exec.c:3674-3682), for
/// the VM's direct builtin and shell-function dispatch, which does not pass
/// through `execcmd_exec`:
/// ```c
/// } else if (is_cursh) {
///     jobtab[thisjob].stat |= STAT_CURSH;
///     if (!jobtab[thisjob].procs)
///         jobtab[thisjob].stat |= STAT_NOPRINT;
///     if (is_builtin)
///         jobtab[thisjob].stat |= STAT_BUILTIN;
/// ```
/// Without STAT_NOPRINT the running function's own slot is a live job to
/// getjob/bin_fg: `f() { wait %1 }; f` waited on it and returned 0 where
/// zsh reports `%1: no such job` and returns 127 (c:Src/jobs.c:2585-2590).
pub fn mark_thisjob_cursh(is_builtin: bool) {
    let tj = *THISJOB.get_or_init(|| Mutex::new(-1)).lock().unwrap_or_else(|e| e.into_inner());
    if tj < 0 {
        return;
    }
    let table = crate::ported::jobs::JOBTAB.get_or_init(|| Mutex::new(Vec::new()));
    let mut tab = table.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(j) = tab.get_mut(tj as usize) {
        j.stat |= stat::CURSH; // c:3678
        if j.procs.is_empty() {
            j.stat |= stat::NOPRINT; // c:3679-3680
        }
        if is_builtin {
            j.stat |= stat::BUILTIN; // c:3681-3682
        }
    }
}

/// The job slot of the innermost running pipeline, if one is held.
pub fn execpline_slot_current() -> Option<usize> {
    HELD_SLOTS.with(|h| h.borrow().last().map(|s| s.slot))
}

/// Delete the job slots held from position `start` on, innermost first, and
/// put `thisjob` back to what it was before the outermost of them opened.
fn release_held_slots(start: usize) {
    let released: Vec<HeldSlot> = HELD_SLOTS.with(|h| {
        let mut h = h.borrow_mut();
        if start >= h.len() {
            return Vec::new();
        }
        h.split_off(start)
    });
    let Some(outermost) = released.first() else {
        return;
    };
    let table = crate::ported::jobs::JOBTAB.get_or_init(|| Mutex::new(Vec::new()));
    {
        let mut tab = table.lock().unwrap_or_else(|e| e.into_inner());
        for held in released.iter().rev() {
            // A stopped job stays in the table for `fg` (zwaitjob returns on
            // STAT_STOPPED and nothing deletes it), and a superjob belongs to
            // the list_pipe machinery, not to this frame.
            if let Some(jn) = tab.get(held.slot) {
                if (jn.stat & stat::INUSE) != 0
                    && (jn.stat & (stat::STOPPED | stat::SUPERJOB)) == 0
                {
                    deletejob(&mut tab, held.slot, false); // c:Src/jobs.c:1754
                }
            }
        }
        // c:Src/jobs.c:1441-1443 — freejob's `maxjob` shrink, which the
        // ported freejob (one job, no table) cannot do.
        let mut mj = MAXJOB.get_or_init(|| Mutex::new(0)).lock().unwrap_or_else(|e| e.into_inner());
        while *mj > 0 && tab.get(*mj).map_or(true, |j| (j.stat & stat::INUSE) == 0) {
            *mj -= 1;
        }
    }
    *THISJOB.get_or_init(|| Mutex::new(-1)).lock().unwrap_or_else(|e| e.into_inner()) =
        outermost.pj; // c:Src/exec.c:1981 `thisjob = pj;`
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

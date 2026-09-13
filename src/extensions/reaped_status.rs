//! Raw wait statuses the SIGCHLD reaper collected, keyed by pid.
//!
//! !!! WARNING: RUST-ONLY HELPER — NO C COUNTERPART !!!
//!
//! C has no need for this because C has exactly ONE `waitpid` in the
//! whole shell: `wait_for_processes` (c:Src/signals.c:285). The
//! foreground waits do NOT call `waitpid` — `waitforpid` polls with
//! `kill(pid, 0)` and sleeps in `signal_suspend(SIGCHLD, …)`
//! (c:Src/jobs.c:1652-1666), and `zwaitjob` does the same on the job's
//! STAT_DONE bit (c:Src/jobs.c:1702-1710). So the reaper is the only
//! collector: it stores every status it takes into `pn->status` through
//! `update_process` (c:Src/jobs.c:366-388), and `storepipestats` reads
//! `$pipestatus` back out of those same `pn->status` fields
//! (c:Src/jobs.c:423-435). A status can never be "stolen" there because
//! nothing else ever wanted to collect it.
//!
//! zshrs's pipeline reaps its own forked stages with a targeted
//! `waitpid(pid)` (`fusevm_bridge::waitpid_eintr`), so it has a SECOND
//! collector that races the reaper. When the reaper wins, the targeted
//! wait gets `ECHILD` and the stage's status is gone: with the reaper
//! armed, `(exit 5) | (sleep 0.5; exit 3)` published `$pipestatus` as
//! `0 3` where zsh says `5 3`. This ring hands the status back to the
//! losing collector, i.e. it is what makes zshrs's two-collector design
//! observe what C's one-collector design observes.
//!
//! Written from `zhandler`'s SIGCHLD arm, so it is plain atomics: no
//! allocation, no locks, nothing that is unsafe to touch in a handler.

use std::sync::atomic::{AtomicI32, AtomicUsize, Ordering};

/// Number of slots. C bounds its own reaped-status store
/// (`bgstatus_list`) at `sysconf(_SC_CHILD_MAX)` and drops the OLDEST
/// entry once it is full (c:Src/jobs.c:2377-2379 — `if (bgstatus_count
/// == child_max) rembgstatus(firstnode(bgstatus_list));`). This ring has
/// the same shape with a fixed bound, which is what lets it be written
/// without allocating.
const SLOTS: usize = 64;

static PIDS: [AtomicI32; SLOTS] = [const { AtomicI32::new(0) }; SLOTS];
static STATUSES: [AtomicI32; SLOTS] = [const { AtomicI32::new(0) }; SLOTS];
/// Next slot to overwrite — the "drop the oldest" half of c:Src/jobs.c:2378.
static CURSOR: AtomicUsize = AtomicUsize::new(0);

/// Publish the raw status the reaper just took for `pid`.
///
/// Only a status that actually ENDED the process is recorded, which is
/// the same gate C puts on `addbgstatus` (c:Src/jobs.c:695-697 records
/// only `WIFEXITED` and `WIFSIGNALED`). A `WUNTRACED`/`WCONTINUED`
/// status leaves the child alive and reapable, so a later targeted
/// `waitpid` on it still collects the real thing and must not be handed
/// a stop status instead.
#[cfg(unix)]
pub fn record(pid: i32, status: i32) {
    if pid <= 0 || !(libc::WIFEXITED(status) || libc::WIFSIGNALED(status)) {
        return;
    }
    let slot = CURSOR.fetch_add(1, Ordering::SeqCst) % SLOTS;
    // Blank the key first so a concurrent reader can never pair this
    // slot's pid with the PREVIOUS occupant's status.
    PIDS[slot].store(0, Ordering::SeqCst);
    STATUSES[slot].store(status, Ordering::SeqCst);
    PIDS[slot].store(pid, Ordering::SeqCst);
}

/// Claim the status the reaper took for `pid`, if it is still on record.
///
/// Removing the entry on read is C's rule for its own reaped-status
/// store: "This is only used by wait, which must only work on each pid
/// once, so we need to remove the entry if we find it"
/// (c:Src/jobs.c:2398-2404, `getbgstatus`).
#[cfg(unix)]
pub fn take(pid: i32) -> Option<i32> {
    if pid <= 0 {
        return None;
    }
    for slot in 0..SLOTS {
        if PIDS[slot].load(Ordering::SeqCst) != pid {
            continue;
        }
        let status = STATUSES[slot].load(Ordering::SeqCst);
        // Claim it: only the thread that clears the key returns it, so
        // two waiters can never both be told they reaped the same child.
        if PIDS[slot]
            .compare_exchange(pid, 0, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            return Some(status);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    /// A recorded status comes back exactly once: the second claim has
    /// to miss, or two waiters would both believe they reaped the child
    /// (c:Src/jobs.c:2398-2404 removes the entry on read for the same
    /// reason).
    #[test]
    fn a_recorded_status_is_claimable_exactly_once() {
        // 0x0500 is `WEXITSTATUS == 5` in the wait-status encoding.
        super::record(999_001, 0x0500);
        assert_eq!(super::take(999_001), Some(0x0500));
        assert_eq!(super::take(999_001), None);
    }

    /// A stop/continue status leaves the child alive and reapable, so
    /// recording it would let a later `ECHILD` lookup for that pid claim
    /// a stop status as if it were an exit. C's `addbgstatus` gate is
    /// the same (c:Src/jobs.c:695-697 records only exited/signalled).
    #[test]
    fn a_stop_status_is_not_recorded() {
        // 0x117f is `WIFSTOPPED` with WSTOPSIG == SIGSTOP.
        super::record(999_002, 0x117f);
        assert_eq!(super::take(999_002), None);
    }
}

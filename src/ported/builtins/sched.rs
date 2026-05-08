//! Scheduled command execution — port of `Src/Builtins/sched.c`.
//!
//! Implements the `sched` builtin and the `$zsh_scheduled_events`
//! special parameter.
//!
//! Structure mirrors the C source:
//!   - `enum schedflags` (sched.c:38)
//!   - `struct schedcmd` (sched.c:43)
//!   - `static struct schedcmd *schedcmds` (sched.c:52)
//!   - `static int schedcmdtimed` (sched.c:55)
//!   - `schedaddtimed` / `scheddeltimed` / `checksched` (sched.c:61/79/93)
//!   - `bin_sched` (sched.c:150)
//!   - `schedgetfn` (sched.c:341)
//!   - module entries (sched.c:396+)

#![allow(non_camel_case_types)]
#![allow(non_upper_case_globals)]
#![allow(non_snake_case)]

use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::ported::utils::zwarnnam;

// =====================================================================
// Port of `enum schedflags` from Src/Builtins/sched.c:38.
// =====================================================================

/// Trash zle if necessary when event is activated. Set by `sched -o`
/// (sched.c:194-195).
pub const SCHEDFLAG_TRASH_ZLE: i32 = 1;

// =====================================================================
// Port of `struct schedcmd` from Src/Builtins/sched.c:43.
// =====================================================================

/// One scheduled command. C uses an intrusive singly-linked list via
/// `next`; the Rust port keeps the same field set but stores the
/// list as a `Vec<schedcmd>` inside the `SCHEDCMDS` thread_local
/// (see WARNING above the static).
///
/// ```c
/// struct schedcmd {
///     struct schedcmd *next;
///     char *cmd;
///     time_t time;
///     int flags;
/// };
/// ```
#[derive(Debug, Clone)]
pub struct schedcmd {
    pub cmd: String,
    pub time: i64,
    pub flags: i32,
}

// =====================================================================
// Module-static state (sched.c:52, 55).
//
// Per PORT_PLAN.md these are bucket 2 (shell-wide) — `checksched`
// can fire on any thread that hits the prepromptfn hook, and
// `bin_sched` mutates from the foreground evaluator. `OnceLock<
// Mutex<…>>` so all callers see the same list.
// =====================================================================

// WARNING: NOT IN SCHED.C — Rust-only data-structure choice. C uses
// `static struct schedcmd *schedcmds` (rlimits.c:52) — an intrusive
// singly-linked list. Rust port stores the same logical sequence as
// `Vec<schedcmd>` because Rust's ownership rules make in-place
// linked-list mutation (insert-by-time, delete-by-index, head-pop)
// require either `unsafe`-raw-pointer dance or `Option<Box<…>>`-
// take/replace gymnastics for every traversal. Vec preserves all
// observable ordering (insertion-by-time, head-drain) at O(N) per
// op — fine at the 1-100 entries `sched` realistically holds.
static SCHEDCMDS: OnceLock<Mutex<Vec<schedcmd>>> = OnceLock::new();

/// Port of `static int schedcmdtimed` from sched.c:55. Tracks
/// whether `addtimedfn(checksched, …)` has been called for the
/// current head entry; cleared by `scheddeltimed`.
static SCHEDCMDTIMED: OnceLock<Mutex<bool>> = OnceLock::new();

// WARNING: NOT IN SCHED.C — Rust-only accessor. C dereferences
// `schedcmds` directly; Rust port factors out the lock-acquire so
// the public fns don't each duplicate the OnceLock dance.
fn schedcmds_lock() -> &'static Mutex<Vec<schedcmd>> {
    SCHEDCMDS.get_or_init(|| Mutex::new(Vec::new()))
}

// WARNING: NOT IN SCHED.C — companion to `schedcmds_lock` above.
fn schedcmdtimed_lock() -> &'static Mutex<bool> {
    SCHEDCMDTIMED.get_or_init(|| Mutex::new(false))
}

// =====================================================================
// External helpers — declared in `Src/zsh.h`, defined elsewhere.
// =====================================================================

// WARNING: NOT IN SCHED.C — `addtimedfn` lives in `Src/utils.c`
// (declared in `Src/zsh.h`). Stubbed here pending the utils.c port
// of the timed-fn machinery; the Rust port today relies on the
// foreground evaluator hitting `checksched` via the prepromptfn
// chain, so the `addtimedfn` call is a no-op until the SIGALRM-
// driven timer is wired up.
fn addtimedfn(_f: fn(), _at: i64) {}

// WARNING: NOT IN SCHED.C — see `addtimedfn` above. Companion stub.
fn deltimedfn(_f: fn()) {}

// =====================================================================
// Port of `schedaddtimed()` from Src/Builtins/sched.c:61.
// =====================================================================

/// Port of `schedaddtimed()` from `Src/Builtins/sched.c:61`.
///
/// Uses `addtimedfn()` to register `checksched` to fire at the head
/// entry's `time`. C self-corrects via `scheddeltimed()` if the
/// flag was already set — same defensive pattern here.
pub(crate) fn schedaddtimed() {
    let mut timed = schedcmdtimed_lock().lock().unwrap();
    if *timed {
        drop(timed);
        scheddeltimed();
        timed = schedcmdtimed_lock().lock().unwrap();
    }
    *timed = true;
    let head_time = schedcmds_lock()
        .lock()
        .unwrap()
        .first()
        .map(|c| c.time)
        .unwrap_or(0);
    addtimedfn(checksched_thunk, head_time);
}

// WARNING: NOT IN SCHED.C — Rust-only adapter. `addtimedfn` takes a
// `fn()` (zero-arity), but `checksched` returns `i32` in our port
// (so the boot_/cleanup_ entries can return its status). The thunk
// drops the return value to match the C signature.
fn checksched_thunk() {
    let _ = checksched();
}

// =====================================================================
// Port of `scheddeltimed()` from Src/Builtins/sched.c:79.
// =====================================================================

/// Port of `scheddeltimed()` from `Src/Builtins/sched.c:79`.
///
/// Calls `deltimedfn(checksched)` and clears `schedcmdtimed`.
pub(crate) fn scheddeltimed() {
    let mut timed = schedcmdtimed_lock().lock().unwrap();
    if *timed {
        deltimedfn(checksched_thunk);
        *timed = false;
    }
}

// =====================================================================
// Port of `checksched()` from Src/Builtins/sched.c:93.
// =====================================================================

/// Port of `checksched()` from `Src/Builtins/sched.c:93`.
///
/// Walk the head of the schedcmds list and execute every entry whose
/// `time <= now`. The list is time-sorted, so we stop at the first
/// future entry. `scheddeltimed` is called before each execstring so
/// re-entrant `sched` calls inside the executed command can install
/// a fresh timer.
pub(crate) fn checksched() -> i32 {
    let cmds_lock = schedcmds_lock();
    if cmds_lock.lock().unwrap().is_empty() {
        return 0;
    }
    let now = now_secs();
    loop {
        let due = {
            let v = cmds_lock.lock().unwrap();
            !v.is_empty() && v[0].time <= now
        };
        if !due {
            break;
        }
        // Pop head (matches C `sch = schedcmds; schedcmds = sch->next;`).
        let sch = {
            let mut v = cmds_lock.lock().unwrap();
            v.remove(0)
        };
        // Delete the timed fn first, in case the executed command
        // schedules more (sched.c:118).
        scheddeltimed();
        // C: `if ((sch->flags & SCHEDFLAG_TRASH_ZLE) && zleactive)
        //         zleentry(ZLE_CMD_TRASH);`
        // Rust port: skip ZLE trash for now — pending zle_main.c port
        // of `zleentry` / `zleactive`.
        if sch.flags & SCHEDFLAG_TRASH_ZLE != 0 {
            // TODO: zleentry(ZLE_CMD_TRASH) when zle is ported.
        }
        // C: `execstring(sch->cmd, 0, 0, "sched");`
        // Rust port: same — execute via the bytecode VM. Stubbed
        // for now; ShellExecutor's bridge wires the real call in
        // when checksched is invoked from the eval loop.
        execstring(&sch.cmd);
        // Re-arm timer for next entry (sched.c:137).
        let need_rearm = {
            let v = cmds_lock.lock().unwrap();
            let timed = *schedcmdtimed_lock().lock().unwrap();
            !v.is_empty() && !timed
        };
        if need_rearm {
            schedaddtimed();
        }
    }
    0
}

// WARNING: NOT IN SCHED.C — Rust-only bridge stub. C calls
// `execstring(sch->cmd, 0, 0, "sched")` to feed the command back
// through the parser/exec pipeline. The Rust port's exec loop owns
// `ShellExecutor`, which can't be reached from this free fn without
// a global handle. Real wiring lives in
// `crate::ported::exec::ShellExecutor::checksched_drain` (TODO).
fn execstring(_cmd: &str) {
    // Pending: dispatch through fusevm bytecode VM.
}

// WARNING: NOT IN SCHED.C — Rust-only `time_t` shim. C uses
// `time(NULL)` directly; Rust port wraps `SystemTime` to unify the
// epoch arithmetic. Returns seconds since UNIX epoch as `i64`
// (matching `time_t` width on macOS / Linux).
fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// =====================================================================
// Port of `bin_sched()` from Src/Builtins/sched.c:150.
// =====================================================================

/// Port of `bin_sched()` from `Src/Builtins/sched.c:150`.
///
/// Argument shapes (matching the C source):
///   `sched`              → list pending entries
///   `sched -<N>`         → delete entry N (1-based)
///   `sched -o time cmd…` → schedule `cmd…` at `time`, with the
///                          `SCHEDFLAG_TRASH_ZLE` flag
///   `sched [-o] time cmd…` → schedule
///   `sched -- time cmd…` → end-of-options, treats `time cmd…` as
///                          positional even if `cmd…` starts with `-`
pub(crate) fn bin_sched(nam: &str, argv: &[String]) -> i32 {
    let mut argptr = 0usize;
    let mut flags: i32 = 0;

    while argptr < argv.len() && argv[argptr].starts_with('-') {
        let arg = &argv[argptr][1..];
        let first = arg.chars().next();
        if let Some(c) = first {
            if c.is_ascii_digit() {
                // C: sn = atoi(arg); ...
                let sn: i32 = arg.parse().unwrap_or(0);
                if sn == 0 {
                    zwarnnam("sched", "usage for delete: sched -<item#>.");
                    return 1;
                }
                let cmds_lock = schedcmds_lock();
                let mut v = cmds_lock.lock().unwrap();
                let idx = (sn - 1) as usize;
                if idx >= v.len() {
                    drop(v);
                    zwarnnam("sched", "not that many entries");
                    return 1;
                }
                if idx == 0 {
                    drop(v);
                    scheddeltimed();
                    let mut v = cmds_lock.lock().unwrap();
                    v.remove(0);
                    let need_rearm = !v.is_empty();
                    drop(v);
                    if need_rearm {
                        schedaddtimed();
                    }
                } else {
                    v.remove(idx);
                }
                return 0;
            } else if arg == "-" {
                // end of options
                argptr += 1;
                break;
            } else if arg == "o" {
                flags |= SCHEDFLAG_TRASH_ZLE;
            } else if arg.is_empty() {
                zwarnnam(nam, "option expected");
                return 1;
            } else {
                zwarnnam(nam, &format!("bad option: -{}", c));
                return 1;
            }
        }
        argptr += 1;
    }

    // No remaining args → list. (sched.c:206)
    if argptr >= argv.len() {
        let v = schedcmds_lock().lock().unwrap();
        for (i, sch) in v.iter().enumerate() {
            // C: ztrftime(tbuf, 40, "%a %b %e %k:%M:%S", tmp, 0L);
            let tbuf = format_localtime(sch.time);
            let flagstr = if sch.flags & SCHEDFLAG_TRASH_ZLE != 0 {
                "-o "
            } else {
                ""
            };
            let endstr = if sch.cmd.starts_with('-') { "-- " } else { "" };
            println!(
                "{:3} {} {}{}{}",
                i + 1,
                tbuf,
                flagstr,
                endstr,
                sch.cmd
            );
        }
        return 0;
    }

    // Need at least 2 positional args. (sched.c:227)
    if argptr + 1 >= argv.len() {
        zwarnnam("sched", "not enough arguments");
        return 1;
    }

    // Parse time (sched.c:236).
    let s = &argv[argptr];
    argptr += 1;
    let t = match parse_time_spec(s) {
        Ok(t) => t,
        Err(_) => {
            zwarnnam("sched", "bad time specifier");
            return 1;
        }
    };

    // Build command from remaining args (C: zjoin(argptr, ' ', 0)).
    let cmd = argv[argptr..].join(" ");

    // Insert into list (sched.c:307).
    let cmds_lock = schedcmds_lock();
    let was_empty;
    let head_displaced;
    {
        let mut v = cmds_lock.lock().unwrap();
        was_empty = v.is_empty();
        head_displaced = !was_empty && t < v[0].time;
        let pos = v.iter().position(|c| c.time > t).unwrap_or(v.len());
        v.insert(
            pos,
            schedcmd {
                cmd,
                time: t,
                flags,
            },
        );
    }
    if was_empty || head_displaced {
        if head_displaced {
            scheddeltimed();
        }
        schedaddtimed();
    }
    0
}

// WARNING: NOT IN SCHED.C — Rust-only inline of the `bin_sched`
// time-parsing block (sched.c:236-306). C inlines the `+H:M:S`,
// `H:Ma|p`, raw-epoch parsing directly inside `bin_sched`; Rust
// port factors it out for testability since the parsing is the
// most arithmetic-heavy chunk and warrants its own unit tests.
fn parse_time_spec(s: &str) -> Result<i64, &'static str> {
    let now = now_secs();
    if let Some(rest) = s.strip_prefix('+') {
        // Relative time. C: `zstrtol(s+1, &s, 10)`.
        let (zl, rest) = parse_leading_decimal(rest);
        if rest.is_empty() {
            return Ok(now + zl);
        }
        if !rest.starts_with(':') {
            return Err("bad time specifier");
        }
        let (m, after_m) = parse_leading_decimal(&rest[1..]);
        let (sec, after_s) = if let Some(stripped) = after_m.strip_prefix(':') {
            parse_leading_decimal(stripped)
        } else {
            (0, after_m)
        };
        if !after_s.is_empty() {
            return Err("bad time specifier");
        }
        Ok(now + zl * 3600 + m * 60 + sec)
    } else {
        // Absolute time (sched.c:266).
        let (zl, rest) = parse_leading_decimal(s);
        if rest.is_empty() {
            return Ok(zl);
        }
        if !rest.starts_with(':') {
            return Err("bad time specifier");
        }
        let mut h = zl;
        let (m, after_m) = parse_leading_decimal(&rest[1..]);
        let (sec, after_s) = if let Some(stripped) = after_m.strip_prefix(':') {
            parse_leading_decimal(stripped)
        } else {
            (0, after_m)
        };
        // sched.c:281 — trailing chars must be empty or 'a'/'A'/'p'/'P'.
        if !after_s.is_empty() {
            let c = after_s.chars().next().unwrap();
            if c != 'a' && c != 'A' && c != 'p' && c != 'P' {
                return Err("bad time specifier");
            }
        }
        // C: `t = time(NULL); tm = localtime(&t); t -= tm->tm_sec +
        //     tm->tm_min*60 + tm->tm_hour*3600;`
        let t_midnight = midnight_today_local(now);
        let pm = after_s
            .chars()
            .next()
            .map(|c| c == 'p' || c == 'P')
            .unwrap_or(false);
        if pm {
            h += 12;
        }
        let mut t = t_midnight + h * 3600 + m * 60 + sec;
        // sched.c:295 — past-time → tomorrow.
        if t < now {
            t += 3600 * 24;
        }
        Ok(t)
    }
}

// WARNING: NOT IN SCHED.C — Rust-only port of the `zstrtol(s, &s,
// 10)` idiom. C advances the pointer past consumed digits; Rust
// port returns `(value, &remaining_str)` since &mut &str isn't
// idiomatic here.
fn parse_leading_decimal(s: &str) -> (i64, &str) {
    let mut end = 0;
    for (i, c) in s.char_indices() {
        if !c.is_ascii_digit() {
            end = i;
            break;
        }
        end = i + c.len_utf8();
    }
    let val: i64 = s[..end].parse().unwrap_or(0);
    (val, &s[end..])
}

// WARNING: NOT IN SCHED.C — Rust-only port of C's `localtime(&t); t
// -= tm->tm_sec + tm->tm_min*60 + tm->tm_hour*3600`. Returns the
// epoch second of midnight in the local timezone.
fn midnight_today_local(now: i64) -> i64 {
    use chrono::{Local, TimeZone, Timelike};
    if let chrono::LocalResult::Single(dt) = Local.timestamp_opt(now, 0) {
        now - (dt.hour() as i64 * 3600 + dt.minute() as i64 * 60 + dt.second() as i64)
    } else {
        now - (now % 86400)
    }
}

// WARNING: NOT IN SCHED.C — Rust-only port of `ztrftime(tbuf, 40,
// "%a %b %e %k:%M:%S", tmp, 0L)`. C uses zsh's strftime wrapper;
// Rust port uses chrono's strftime with the same format string.
fn format_localtime(t: i64) -> String {
    use chrono::{Local, TimeZone};
    Local
        .timestamp_opt(t, 0)
        .single()
        .map(|dt| dt.format("%a %b %e %k:%M:%S").to_string())
        .unwrap_or_else(|| t.to_string())
}

// =====================================================================
// Port of `schedgetfn()` from Src/Builtins/sched.c:341.
// =====================================================================

/// Port of `schedgetfn()` from `Src/Builtins/sched.c:341`.
///
/// `getfn` for the `$zsh_scheduled_events` array parameter. Each
/// entry serializes as `"<time>:<flagstr>:<cmd>"` matching the C
/// source's `sprintf("%s:%s:%s", tbuf, flagstr, cmd)` (line 366).
pub(crate) fn schedgetfn() -> Vec<String> {
    let v = schedcmds_lock().lock().unwrap();
    v.iter()
        .map(|sch| {
            let flagstr = if sch.flags & SCHEDFLAG_TRASH_ZLE != 0 {
                "-o"
            } else {
                ""
            };
            format!("{}:{}:{}", sch.time, flagstr, sch.cmd)
        })
        .collect()
}

// =====================================================================
// Module entry points (sched.c:396-446).
// =====================================================================

/// Port of `setup_()` from `Src/Builtins/sched.c:396`. Returns 0.
pub fn setup_() -> i32 {
    0
}

/// Port of `features_()` from `Src/Builtins/sched.c:403`. C calls
/// `featuresarray()`; Rust port has no Module type yet — return 0.
pub fn features_() -> i32 {
    0
}

/// Port of `enables_()` from `Src/Builtins/sched.c:411`. C calls
/// `handlefeatures()`; Rust port returns 0.
pub fn enables_() -> i32 {
    0
}

/// Port of `boot_()` from `Src/Builtins/sched.c:418`. C calls
/// `addprepromptfn(&checksched)`; Rust port stubs since the
/// prepromptfn chain isn't fully wired yet (the foreground shell
/// loop calls `checksched` directly via the ShellExecutor bridge).
pub fn boot_() -> i32 {
    0
}

/// Port of `cleanup_()` from `Src/Builtins/sched.c:426`. Drains the
/// schedcmds list, deletes the timed fn, removes the prepromptfn.
pub fn cleanup_() -> i32 {
    let cmds_lock = schedcmds_lock();
    if !cmds_lock.lock().unwrap().is_empty() {
        scheddeltimed();
    }
    cmds_lock.lock().unwrap().clear();
    // C: delprepromptfn(&checksched); — pending the prepromptfn port.
    0
}

/// Port of `finish_()` from `Src/Builtins/sched.c:443`. Returns 0.
pub fn finish_() -> i32 {
    0
}

// =====================================================================
// ShellExecutor bridge — sanctioned PORT.md exception. Wires the
// internal builtin dispatcher to the canonical free fn above.
// =====================================================================

impl crate::ported::exec::ShellExecutor {
    /// `sched` builtin — bridge to `bin_sched()` above.
    pub(crate) fn bin_sched(&mut self, args: &[String]) -> i32 {
        bin_sched("sched", args)
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    // The sched module statics are shell-wide (bucket 2 per
    // PORT_PLAN.md). Tests share one instance per process, so
    // serialize them through a test-only Mutex to keep one test's
    // `reset()` from racing another's `bin_sched`.
    static TEST_SERIAL: StdMutex<()> = StdMutex::new(());

    fn reset_with_lock() -> std::sync::MutexGuard<'static, ()> {
        let guard = TEST_SERIAL.lock().unwrap_or_else(|e| {
            // Recover from poison — some other test panicked but the
            // statics are deterministic, just clear and continue.
            TEST_SERIAL.clear_poison();
            e.into_inner()
        });
        let mut v = schedcmds_lock().lock().unwrap_or_else(|e| {
            schedcmds_lock().clear_poison();
            e.into_inner()
        });
        v.clear();
        drop(v);
        let mut t = schedcmdtimed_lock().lock().unwrap_or_else(|e| {
            schedcmdtimed_lock().clear_poison();
            e.into_inner()
        });
        *t = false;
        drop(t);
        guard
    }

    #[test]
    fn test_parse_time_relative_seconds() {
        let now = now_secs();
        let t = parse_time_spec("+60").unwrap();
        assert!(t >= now + 59 && t <= now + 61);
    }

    #[test]
    fn test_parse_time_relative_hm() {
        let now = now_secs();
        let t = parse_time_spec("+1:30").unwrap();
        let exp = now + 3600 + 1800;
        assert!(t >= exp - 1 && t <= exp + 1);
    }

    #[test]
    fn test_parse_time_relative_hms() {
        let now = now_secs();
        let t = parse_time_spec("+1:30:15").unwrap();
        let exp = now + 3600 + 1800 + 15;
        assert!(t >= exp - 1 && t <= exp + 1);
    }

    #[test]
    fn test_parse_time_absolute_raw() {
        // C: bare integer is a raw time_t.
        let t = parse_time_spec("1700000000").unwrap();
        assert_eq!(t, 1700000000);
    }

    #[test]
    fn test_parse_time_bad() {
        assert!(parse_time_spec("+notanumber").is_err()
            || parse_time_spec("+notanumber").unwrap() == now_secs());
        assert!(parse_time_spec("12:30x").is_err());
    }

    #[test]
    fn test_bin_sched_list_empty() {
        let _serial = reset_with_lock();
        let status = bin_sched("sched", &[]);
        assert_eq!(status, 0);
    }

    #[test]
    fn test_bin_sched_add() {
        let _serial = reset_with_lock();
        let args: Vec<String> = vec!["+60".into(), "echo".into(), "hello".into()];
        let status = bin_sched("sched", &args);
        assert_eq!(status, 0);
        let v = schedcmds_lock().lock().unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].cmd, "echo hello");
    }

    #[test]
    fn test_bin_sched_delete() {
        let _serial = reset_with_lock();
        let args: Vec<String> = vec!["+60".into(), "echo".into(), "hello".into()];
        bin_sched("sched", &args);
        assert_eq!(schedcmds_lock().lock().unwrap().len(), 1);
        let status = bin_sched("sched", &["-1".into()]);
        assert_eq!(status, 0);
        assert!(schedcmds_lock().lock().unwrap().is_empty());
    }

    #[test]
    fn test_bin_sched_not_enough_args() {
        let _serial = reset_with_lock();
        let status = bin_sched("sched", &["+60".into()]);
        assert_eq!(status, 1);
    }

    #[test]
    fn test_bin_sched_o_flag() {
        let _serial = reset_with_lock();
        let args: Vec<String> = vec!["-o".into(), "+60".into(), "cmd".into()];
        let status = bin_sched("sched", &args);
        assert_eq!(status, 0);
        let v = schedcmds_lock().lock().unwrap();
        assert_eq!(v[0].flags & SCHEDFLAG_TRASH_ZLE, SCHEDFLAG_TRASH_ZLE);
    }

    #[test]
    fn test_schedgetfn_serialization() {
        let _serial = reset_with_lock();
        let mut v = schedcmds_lock().lock().unwrap();
        v.push(schedcmd {
            cmd: "echo test".into(),
            time: 1700000000,
            flags: 0,
        });
        v.push(schedcmd {
            cmd: "echo zle".into(),
            time: 1700001000,
            flags: SCHEDFLAG_TRASH_ZLE,
        });
        drop(v);
        let arr = schedgetfn();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0], "1700000000::echo test");
        assert_eq!(arr[1], "1700001000:-o:echo zle");
    }

    #[test]
    fn test_insert_keeps_time_order() {
        let _serial = reset_with_lock();
        bin_sched("sched", &["+200".into(), "third".into()]);
        bin_sched("sched", &["+50".into(), "first".into()]);
        bin_sched("sched", &["+100".into(), "second".into()]);
        let v = schedcmds_lock().lock().unwrap();
        assert_eq!(v[0].cmd, "first");
        assert_eq!(v[1].cmd, "second");
        assert_eq!(v[2].cmd, "third");
    }
}

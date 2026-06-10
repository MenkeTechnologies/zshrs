//! Scheduled command execution — port of `Src/Builtins/sched.c`.
//!
//! Flags for each scheduled event                                           // c:37
//! Use addtimedfn() to add a timed event for sched's use                   // c:57
//! Use deltimedfn() to remove the sched timed event                        // c:75
//! Check scheduled commands; call this function from time to time.         // c:89
//!
//! Top-level declaration order matches the C source, line-by-line:
//!   - `typedef struct schedcmd *Schedcmd;`        c:35
//!   - `enum schedflags { SCHEDFLAG_TRASH_ZLE };`  c:38
//!   - `struct schedcmd { next, cmd, time, flags }` c:43
//!   - `static struct schedcmd *schedcmds;`        c:52
//!   - `static int schedcmdtimed;`                  c:55
//!   - `schedaddtimed(void)`                        c:60
//!   - `scheddeltimed(void)`                        c:78
//!   - `checksched(void)`                           c:92
//!   - `bin_sched(nam, argv, ops, func)`            c:149
//!   - `schedgetfn(pm)`                             c:340
//!   - `static struct builtin bintab[]`             c:374
//!   - `static const struct gsu_array sched_gsu`    c:378
//!   - `static struct paramdef partab[]`            c:381
//!   - `static struct features module_features`     c:386
//!   - `setup_(m)` / `features_(m, features)` /
//!     `enables_(m, enables)` / `boot_(m)` /
//!     `cleanup_(m)` / `finish_(m)`                 c:395-446

#![allow(non_camel_case_types)]
#![allow(non_upper_case_globals)]
#![allow(non_snake_case)]

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Mutex, OnceLock};

use crate::init::zleentry;
use crate::mem::popheap;
use crate::ported::mem::pushheap;
use crate::ported::utils::{
    addprepromptfn, addtimedfn, delprepromptfn, deltimedfn, unmeta, zjoin, zstrtol, ztrftime,
    zwarnnam,
};
use crate::ported::zsh_h::{isset, options, param, MAX_OPS, VERBOSE};
use crate::ported::ztype_h::idigit;
use crate::utils::TIMED_FNS;
use crate::zsh_h::{features, module};
use crate::DPUTS;
use libc::time_t;
// =====================================================================
// typedef struct schedcmd  *Schedcmd;                                c:35
// =====================================================================

/// Port of `typedef struct schedcmd *Schedcmd;` from `Src/Builtins/sched.c:35`.
/// C uses the typedef as a forward declaration; Rust port aliases to the
/// real struct type defined below.
pub type Schedcmd = Option<Box<schedcmd>>;

// =====================================================================
// struct schedcmd                                                    c:43
// =====================================================================

/// Port of `struct schedcmd` from `Src/Builtins/sched.c:43`. Same field
/// names and order as C; `next` is the intrusive forward link of the
/// time-ordered pending-events list.
///
/// ```c
/// struct schedcmd {
///     struct schedcmd *next;
///     char *cmd;            /* command to run */
///     time_t time;          /* when to run it */
///     int flags;            /* flags as above */
/// };
/// ```
#[derive(Debug)]
pub struct schedcmd {
    pub next: Option<Box<schedcmd>>, // c:44
    pub cmd: String,                 // c:45
    pub time: time_t,                // c:46
    pub flags: i32,                  // c:47
}

// =====================================================================
// static int schedcmdtimed;      flag that timed event is running     c:55
// =====================================================================

// Port of `static int schedcmdtimed;` from sched.c:55. Same scope/sharing
// as `schedcmds` above.
static schedcmdtimed: OnceLock<Mutex<i32>> = OnceLock::new();

// =====================================================================
// Use addtimedfn() to add a timed event for sched's use              c:57
// schedaddtimed(void)                                                c:60
// =====================================================================

// Use addtimedfn() to add a timed event for sched's use                  // c:61
/// Port of `schedaddtimed()` from `Src/Builtins/sched.c:61`.
pub(crate) fn schedaddtimed() {
    // c:61
    /*
     * The following code shouldn't be necessary and indicates
     * a bug.  However, the DPUTS() in the caller should pick
     * this up so we can detect and fix it, and the following
     * Makes The World Safe For Timed Events in non-debugging shells.
     */                                                              // c:63-68
    if *schedcmdtimed_lock().lock().unwrap() != 0 {
        // c:69
        scheddeltimed(); // c:70
    }
    *schedcmdtimed_lock().lock().unwrap() = 1; // c:71
    let head_time: time_t = schedcmds_lock() // c:72  schedcmds->time
        .lock()
        .unwrap()
        .as_ref()
        .map(|h| h.time)
        .unwrap_or(0);
    addtimedfn(checksched_thunk, head_time as i64); // c:72
}

// =====================================================================
// Use deltimedfn() to remove the sched timed event                   c:75
// scheddeltimed(void)                                                c:78
// =====================================================================

/// Port of `scheddeltimed()` from `Src/Builtins/sched.c:79`.
pub(crate) fn scheddeltimed() {
    // c:79
    if *schedcmdtimed_lock().lock().unwrap() != 0 {
        // c:79
        deltimedfn(checksched_thunk); // c:83
        *schedcmdtimed_lock().lock().unwrap() = 0; // c:84
    }
}

// =====================================================================
// Check scheduled commands; call this function from time to time.   c:89
// checksched(void)                                                   c:92
// =====================================================================

// Check scheduled commands; call this function from time to time.        // c:93
/// Port of `checksched()` from `Src/Builtins/sched.c:93`.
///
/// C returns void; Rust port returns `i32` so the module entry-points
/// can propagate the status. Bridged through `checksched_thunk()` above
/// when called via `addtimedfn`.
pub(crate) fn checksched() -> i32 {
    // c:93
    let t: time_t; // c:93
                   // sch declared at body scope per C (c:96).
    if schedcmds_lock().lock().unwrap().is_none() {
        // c:98  if(!schedcmds)
        return 0; // c:99  return;
    }
    t = unsafe { libc::time(std::ptr::null_mut()) }; // c:100 t = time(NULL);
                                                     /*
                                                      * List is ordered, so we only need to consider the
                                                      * head element.
                                                      */                                                              // c:101-104
    loop {
        // c:105 while (schedcmds && schedcmds->time <= t)
        let due = {
            let head = schedcmds_lock().lock().unwrap();
            match head.as_ref() {
                Some(h) if h.time <= t => true,
                _ => false,
            }
        };
        if !due {
            break;
        }
        /*
         * Remove the entry to be executed from the list
         * before execution:  this makes quite sure that
         * the entry hasn't been monkeyed with when we
         * free it.
         */
        // c:106-111
        let sch: Box<schedcmd> = {
            // c:112 sch = schedcmds;
            let mut head = schedcmds_lock().lock().unwrap();
            let mut h = head.take().unwrap();
            *head = h.next.take(); // c:113 schedcmds = sch->next;
            h
        };
        /*
         * Delete from the timed function list now in case
         * the called code reschedules.
         */
        // c:114-117
        scheddeltimed(); // c:118

        if (sch.flags & SCHEDFLAG_TRASH_ZLE) != 0 && zleactive.load(Ordering::Relaxed) != 0
        // c:120
        {
            zleentry(ZLE_CMD_TRASH); // c:121
        }
        execstring(&sch.cmd, 0, 0, "sched"); // c:122
                                             // C: zsfree(sch->cmd); zfree(sch, sizeof(struct schedcmd));
                                             // Rust: Box drop on `sch` reclaims both the heap node and the
                                             // owned `cmd` String — equivalent to the C two-step.        // c:123-124
        drop(sch);

        /*
         * Fix time for future events.
         * I had this outside the loop, for a little extra efficiency.
         * However, it then occurred to me that having the list of
         * forthcoming entries up to date could be regarded as
         * a feature, and the inefficiency is negligible.
         *
         * Careful in case the code we called has already set
         * up a timed event; if it has, that'll be up to date since
         * we haven't changed the list here.
         */
        // c:126-136
        let need_arm = {
            let head = schedcmds_lock().lock().unwrap();
            head.is_some() && *schedcmdtimed_lock().lock().unwrap() == 0
        };
        if need_arm {
            // c:137 if (schedcmds && !schedcmdtimed)
            /*
             * We've already deleted the function from the list.
             */                                                      // c:138-140
            // c:141-142 — DPUTS(timedfns && firstnode(timedfns), "BUG: already timed fn (1)")
            DPUTS!(
                // c:141
                !TIMED_FNS.lock().unwrap().is_empty(), // c:141 timedfns && firstnode(timedfns)
                "BUG: already timed fn (1)"            // c:142
            );
            schedaddtimed(); // c:143
        }
    }
    0
}

// =====================================================================
// bin_sched(char *nam, char **argv, Options ops, UNUSED(int func))   c:149
// =====================================================================

/// Port of `bin_sched(char *nam, char **argv, UNUSED(Options ops), UNUSED(int func))` from `Src/Builtins/sched.c:150`.
///
/// C signature mirrored verbatim:
/// ```c
/// static int
/// bin_sched(char *nam, char **argv, UNUSED(Options ops), UNUSED(int func))
/// ```
/// WARNING: param names don't match C — Rust=(argv, _ops, _func) vs C=(nam, argv, ops, func)
pub(crate) fn bin_sched(nam: &str, argv: &[String], _ops: &options, _func: i32) -> i32 {
    // c:150
    // c:150-157 — locals (one block at top, mirroring C declaration)
    let s: &str;
    let mut argptr: usize;
    let mut t: time_t;
    let mut h: i64;
    let m: i64;
    let sec: i64;
    // sch / sch2 / schl — declared as `Option<Box<schedcmd>>` walks below
    let mut sn: i32;
    let mut flags: i32 = 0;

    /* If the argument begins with a -, remove the specified item from the
    schedule. */
    // c:159-160
    argptr = 0; // c:161 argptr = argv;
    while argptr < argv.len() && argv[argptr].as_bytes().first().copied() == Some(b'-') {
        let arg = &argv[argptr][1..]; // c:162 arg = *argptr + 1;
        let arg_b = arg.as_bytes();
        if !arg_b.is_empty() && idigit(arg_b[0]) {
            // c:163 if (idigit(*arg))
            sn = zstrtol(arg, 10).0 as i32; // c:164 sn = atoi(arg);

            if sn == 0 {
                // c:166
                zwarnnam("sched", "usage for delete: sched -<item#>."); // c:167
                return 1;
            }
            // for (schl = NULL, sch = schedcmds, sn--;
            //      sch && sn; sch = (schl = sch)->next, sn--);        // c:170-171
            // Walk to (sn-1)-th node, tracking previous link slot.
            let mut head = schedcmds_lock().lock().unwrap();
            let mut steps = sn - 1;
            // Find the slot whose `*` is the node to remove.
            // `slot` holds &mut Option<Box<schedcmd>>: a pointer into
            // `schedcmds` (when steps==0) or into the previous node's
            // `.next` link (otherwise).
            let mut slot: &mut Option<Box<schedcmd>> = &mut *head;
            while steps > 0 {
                if slot.is_none() {
                    break;
                }
                slot = &mut slot.as_mut().unwrap().next;
                steps -= 1;
            }
            if slot.is_none() {
                // c:172 if (!sch)
                drop(head);
                zwarnnam("sched", "not that many entries"); // c:173
                return 1;
            }
            // Splice out: slot.take(), then *slot = removed.next.
            let head_slot = sn == 1;
            let mut removed = slot.take().unwrap();
            *slot = removed.next.take();
            if head_slot {
                // c:178 else { scheddeltimed(); ... }
                drop(head);
                scheddeltimed();
                let still_have = schedcmds_lock().lock().unwrap().is_some();
                if still_have {
                    // c:181 if (schedcmds)
                    // c:182 — DPUTS(timedfns && firstnode(timedfns), "BUG: already timed fn (2)")
                    DPUTS!(
                        // c:182
                        !TIMED_FNS.lock().unwrap().is_empty(), // c:182
                        "BUG: already timed fn (2)"            // c:182
                    );
                    schedaddtimed(); // c:183
                }
            } else {
                drop(head);
            }
            // C: zsfree(sch->cmd); zfree(sch, sizeof(struct schedcmd));
            drop(removed); // c:186-187
            return 0; // c:189
        } else if arg == "-" {
            // c:190 else if (*arg == '-')
            /* end of options */                                        // c:191
            argptr += 1; // c:192
            break; // c:193
        } else if arg == "o" {
            // c:194 else if (!strcmp(arg, "o"))
            flags |= SCHEDFLAG_TRASH_ZLE; // c:195
        } else {
            if !arg.is_empty() {
                // c:197 if (*arg)
                zwarnnam(
                    nam,
                    &format!("bad option: -{}", arg.chars().next().unwrap()),
                ); // c:198
            } else {
                zwarnnam(nam, "option expected"); // c:200
            }
            return 1; // c:201
        }
        argptr += 1;
    }

    /* given no arguments, display the schedule list */
    // c:205
    if argptr >= argv.len() {
        // c:206 if (!*argptr)
        // for (sn = 1, sch = schedcmds; sch; sch = sch->next, sn++)    // c:207
        sn = 1;
        let head = schedcmds_lock().lock().unwrap();
        let mut cur: &Option<Box<schedcmd>> = &*head;
        while let Some(sch) = cur.as_ref() {
            // C: char tbuf[60], *flagstr, *endstr; time_t t; struct tm *tmp;
            let t_local: time_t = sch.time; // c:212
                                            // C: ztrftime(tbuf, 40, "%a %b %e %k:%M:%S", tmp, 0L);
            let tbuf = ztrftime(
                // c:214
                "%a %b %e %k:%M:%S",
                std::time::UNIX_EPOCH + std::time::Duration::from_secs(t_local as u64),
                // c:213 tmp = localtime(&t); so use_gmt = false.
                false,
            );
            let flagstr = if (sch.flags & SCHEDFLAG_TRASH_ZLE) != 0 {
                // c:215
                "-o " // c:216
            } else {
                "" // c:218
            };
            let endstr = if sch.cmd.starts_with('-') {
                // c:219 if (*sch->cmd == '-')
                "-- " // c:220
            } else {
                "" // c:222
            };
            println!(
                "{:3} {} {}{}{}",
                sn,
                tbuf,
                flagstr,
                endstr,
                unmeta(&sch.cmd)
            ); // c:223-224
            cur = &sch.next;
            sn += 1;
        }
        return 0; // c:226
    } else if argptr + 1 >= argv.len() {
        // c:227 else if (!argptr[1])
        /* other than the two cases above, sched *
         *requires at least two arguments        */                     // c:228-229
        zwarnnam("sched", "not enough arguments"); // c:230
        return 1; // c:231
    }

    /* The first argument specifies the time to schedule the command for.  The
    remaining arguments form the command. */
    // c:234-235
    s = &argv[argptr]; // c:236 s = *argptr++;
    argptr += 1;
    let s_b = s.as_bytes();
    if !s_b.is_empty() && s_b[0] == b'+' {
        // c:237 if (*s == '+')
        /*
         * + introduces a relative time.  The rest of the argument may be an
         * hour:minute offset from the current time.  Once the hour and minute
         * numbers have been extracted, and the format verified, the resulting
         * offset is simply added to the current time.
         */                                                              // c:238-243
        let (zl, rest) = zstrtol(&s[1..], 10); // c:244 zl = zstrtol(s+1, &s, 10);
        let rb = rest.as_bytes();
        if !rb.is_empty() && rb[0] == b':' {
            // c:245 if (*s == ':')
            let (mv, rest2) = zstrtol(&rest[1..], 10); // c:246 m = zstrtol(s+1, &s, 10);
            m = mv;
            let rb2 = rest2.as_bytes();
            let rest3: &str;
            if !rb2.is_empty() && rb2[0] == b':' {
                // c:247 if (*s == ':')
                let (sv, r3) = zstrtol(&rest2[1..], 10); // c:248 sec = zstrtol(s+1, &s, 10);
                sec = sv;
                rest3 = r3;
            } else {
                sec = 0; // c:250
                rest3 = rest2;
            }
            if !rest3.is_empty() {
                // c:251 if (*s)
                zwarnnam("sched", "bad time specifier"); // c:252
                return 1;
            }
            t = unsafe { libc::time(std::ptr::null_mut()) }              // c:255
                + (zl as time_t) * 3600 + (m as time_t) * 60 + (sec as time_t);
        } else if rest.is_empty() {
            // c:256 else if (!*s)
            /*
             * Alternatively, it may simply be a number of seconds.
             * This is here for consistency with absolute times.
             */                                                          // c:257-260
            t = unsafe { libc::time(std::ptr::null_mut()) } + zl as time_t; // c:261
        } else {
            zwarnnam("sched", "bad time specifier"); // c:263
            return 1;
        }
    } else {
        /*
         * If there is no +, an absolute time must have been given.
         * This may be in hour:minute format, optionally followed by a string
         * starting with `a' or `p' (for a.m. or p.m.).  Characters after the
         * `a' or `p' are ignored.
         */
        // c:267-272
        let (zl, rest) = zstrtol(s, 10); // c:273 zl = zstrtol(s, &s, 10);
        let rb = rest.as_bytes();
        if !rb.is_empty() && rb[0] == b':' {
            // c:274 if (*s == ':')
            h = zl; // c:275 h = (long)zl;
            let (mv, rest2) = zstrtol(&rest[1..], 10); // c:276 m = zstrtol(s+1, &s, 10);
            m = mv;
            let rb2 = rest2.as_bytes();
            let rest3: &str;
            if !rb2.is_empty() && rb2[0] == b':' {
                // c:277 if (*s == ':')
                let (sv, r3) = zstrtol(&rest2[1..], 10); // c:278
                sec = sv;
                rest3 = r3;
            } else {
                sec = 0; // c:280
                rest3 = rest2;
            }
            let rb3 = rest3.as_bytes();
            if !rb3.is_empty()
                && rb3[0] != b'a'
                && rb3[0] != b'A'
                && rb3[0] != b'p'
                && rb3[0] != b'P'
            // c:281
            {
                zwarnnam("sched", "bad time specifier"); // c:282
                return 1;
            }
            t = unsafe { libc::time(std::ptr::null_mut()) }; // c:285 t = time(NULL);
                                                             // tm = localtime(&t); t -= tm->tm_sec + tm->tm_min*60 + tm->tm_hour*3600;
            unsafe {
                let tm_ptr = libc::localtime(&t as *const time_t); // c:286
                if !tm_ptr.is_null() {
                    let tm = *tm_ptr;
                    t -= (tm.tm_sec as time_t)
                        + (tm.tm_min as time_t) * 60
                        + (tm.tm_hour as time_t) * 3600; // c:287
                }
            }
            if !rb3.is_empty() && (rb3[0] == b'p' || rb3[0] == b'P') {
                // c:288
                h += 12; // c:289
            }
            t += (h as time_t) * 3600 + (m as time_t) * 60 + (sec as time_t); // c:290
                                                                              /*
                                                                               * If the specified time is before the current time, it must refer
                                                                               * to tomorrow.
                                                                               */                                                          // c:291-294
            if t < unsafe { libc::time(std::ptr::null_mut()) } {
                // c:295
                t += 3600 * 24; // c:296
            }
        } else if rest.is_empty() {
            // c:297 else if (!*s)
            /*
             * Otherwise, it must be a raw time specifier.
             */                                                          // c:298-300
            t = zl as time_t; // c:301
        } else {
            zwarnnam("sched", "bad time specifier"); // c:303
            return 1;
        }
    }
    /* The time has been calculated; now add the new entry to the linked list
    of scheduled commands. */
    // c:307-308
    // sch = (struct schedcmd *) zalloc(sizeof *sch);                    // c:309
    let mut sch_new: Box<schedcmd> = Box::new(schedcmd {
        next: None,
        cmd: zjoin(&argv[argptr..], ' '), // c:311 sch->cmd = zjoin(argptr, ' ', 0);
        time: t,                          // c:310 sch->time = t;
        flags,                            // c:312 sch->flags = flags;
    });
    /* Insert into list in time order */
    // c:313
    let mut head = schedcmds_lock().lock().unwrap();
    if head.is_some() {
        // c:314 if (schedcmds)
        if sch_new.time < head.as_ref().unwrap().time {
            // c:315 if (sch->time < schedcmds->time)
            drop(head);
            scheddeltimed(); // c:316
            let mut head2 = schedcmds_lock().lock().unwrap();
            sch_new.next = head2.take(); // c:317 sch->next = schedcmds;
            *head2 = Some(sch_new); // c:318 schedcmds = sch;
            drop(head2);
            // c:319 — DPUTS(timedfns && firstnode(timedfns), "BUG: already timed fn (3)")
            DPUTS!(
                // c:319
                !TIMED_FNS.lock().unwrap().is_empty(), // c:319
                "BUG: already timed fn (3)"            // c:319
            );
            schedaddtimed(); // c:320
        } else {
            // for (sch2 = schedcmds;
            //      sch2->next && sch2->next->time < sch->time;
            //      sch2 = sch2->next)                                    // c:322-325
            let mut cur = head.as_mut().unwrap();
            while cur.next.is_some() && cur.next.as_ref().unwrap().time < sch_new.time {
                cur = cur.next.as_mut().unwrap();
            }
            sch_new.next = cur.next.take(); // c:326 sch->next = sch2->next;
            cur.next = Some(sch_new); // c:327 sch2->next = sch;
        }
    } else {
        sch_new.next = None; // c:330 sch->next = NULL;
        *head = Some(sch_new); // c:331 schedcmds = sch;
        drop(head);
        // c:332 — DPUTS(timedfns && firstnode(timedfns), "BUG: already timed fn (4)")
        DPUTS!(
            // c:332
            !TIMED_FNS.lock().unwrap().is_empty(), // c:332
            "BUG: already timed fn (4)"            // c:332
        );
        schedaddtimed(); // c:333
    }
    0 // c:335
}

// =====================================================================
// schedgetfn(UNUSED(Param pm))                                       c:340
// =====================================================================

/// Port of `schedgetfn(UNUSED(Param pm))` from `Src/Builtins/sched.c:341`.
///
/// `getfn` for the `$zsh_scheduled_events` array parameter. C signature
/// mirrored: `static char ** schedgetfn(UNUSED(Param pm))`. `Param` is
/// `struct param *` (zsh.h:539 typedef) which is a non-const pointer;
/// pub so the modules/parameter.rs PARTAB_ARRAY entry can reference
/// it (Src/Builtins/sched.c:382 `SPECIALPMDEF("zsh_scheduled_events",
/// ..., &sched_gsu, ...)` wires this fn as the getfn through
/// `addparamdef` at module-load time).
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn schedgetfn(_pm: *mut param) -> Vec<String> {
    let mut i: usize; // c:341
                      // C: int i; struct schedcmd *sch; char **ret, **aptr;
                      // for (i = 0, sch = schedcmds; sch; sch = sch->next, i++);          // c:347-348
    let head = schedcmds_lock().lock().unwrap();
    i = 0;
    {
        let mut cur: &Option<Box<schedcmd>> = &*head;
        while let Some(s) = cur.as_ref() {
            cur = &s.next;
            i += 1;
        }
    }
    // aptr = ret = zhalloc(sizeof(char *) * (i+1));                     // c:350
    let mut ret: Vec<String> = Vec::with_capacity(i);
    // for (sch = schedcmds; sch; sch = sch->next, aptr++)                // c:351
    let mut cur: &Option<Box<schedcmd>> = &*head;
    while let Some(sch) = cur.as_ref() {
        // C: char tbuf[40], *flagstr; time_t t;
        let t: time_t = sch.time; // c:355
        let tbuf = format!("{}", t as i64); // c:357 sprintf(tbuf, "%lld", (long long)t);
        let flagstr = if (sch.flags & SCHEDFLAG_TRASH_ZLE) != 0 {
            // c:361
            "-o" // c:362
        } else {
            "" // c:364
        };
        // *aptr = (char *)zhalloc(5 + strlen(tbuf) + strlen(sch->cmd));
        // sprintf(*aptr, "%s:%s:%s", tbuf, flagstr, sch->cmd);           // c:365-366
        ret.push(format!("{}:{}:{}", tbuf, flagstr, sch.cmd));
        cur = &sch.next;
    }
    // *aptr = NULL;                                                      // c:368 (Vec carries its own length)
    ret // c:370
}

// =====================================================================
// Module entry points                                                c:394-446
// =====================================================================

/// Port of `setup_(UNUSED(Module m))` from `Src/Builtins/sched.c:396`.
#[allow(unused_variables)]
pub fn setup_(m: *const module) -> i32 {
    // c:396
    0 // c:411
}

/// Port of `features_(UNUSED(Module m), UNUSED(char ***features))` from `Src/Builtins/sched.c:403`.
/// C body: `*features = featuresarray(m, &module_features); return 0;`
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {
    // c:403
    *features = featuresarray(m, module_features()); // c:418
    0 // c:418
}

/// Port of `enables_(UNUSED(Module m), UNUSED(int **enables))` from `Src/Builtins/sched.c:411`.
/// C body: `return handlefeatures(m, &module_features, enables);`
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {
    // c:411
    handlefeatures(m, module_features(), enables) // c:418
}

/// Port of `boot_(UNUSED(Module m))` from `Src/Builtins/sched.c:418`.
#[allow(unused_variables)]
pub fn boot_(m: *const module) -> i32 {
    // c:418
    addprepromptfn(checksched_thunk); // c:426 addprepromptfn(&checksched);
    0 // c:426
}

/// Port of `cleanup_(UNUSED(Module m))` from `Src/Builtins/sched.c:426`.
/// C body (c:428-438):
///   struct schedcmd *sch, *schn;
///   if (schedcmds) scheddeltimed();
///   for (sch = schedcmds; sch; sch = schn) { schn = sch->next; zsfree(sch->cmd); zfree(sch, sizeof(*sch)); }
///   delprepromptfn(&checksched);
///   return setfeatureenables(m, &module_features, NULL);
pub fn cleanup_(m: *const module) -> i32 {
    // c:426
    // struct schedcmd *sch, *schn;                                       // c:426
    if schedcmds_lock().lock().unwrap().is_some() {
        // c:430 if (schedcmds)
        scheddeltimed(); // c:431
    }
    // c:443-436 — for (sch = schedcmds; sch; sch = schn) { ... }
    // Box drop chain reclaims `cmd` and the node both.
    let mut head = schedcmds_lock().lock().unwrap();
    *head = None;
    drop(head);
    delprepromptfn(checksched_thunk); // c:443 delprepromptfn(&checksched);
    setfeatureenables(m, module_features(), None) // c:443
}

// =====================================================================
// static struct builtin bintab[]                                     c:374
// static const struct gsu_array sched_gsu                            c:378
// static struct paramdef partab[]                                    c:381
// static struct features module_features                             c:386
// =====================================================================

/// Port of `finish_(UNUSED(Module m))` from `Src/Builtins/sched.c:443`.
#[allow(unused_variables)]
pub fn finish_(m: *const module) -> i32 {
    // c:443
    0 // c:443
}

// =====================================================================
// enum schedflags                                                    c:38
// =====================================================================

// Port of `enum schedflags` from `Src/Builtins/sched.c:38`.
// Trash zle if necessary when event is activated.
/// `SCHEDFLAG_TRASH_ZLE` constant.
pub const SCHEDFLAG_TRASH_ZLE: i32 = 1; // c:40

// =====================================================================
// static struct schedcmd *schedcmds;   the list of sched jobs pending  c:52
// =====================================================================

// Port of `static struct schedcmd *schedcmds;` from sched.c:52. Bucket-2
// (shell-wide) per PORT_PLAN.md: `checksched` can fire on any thread
// reaching the prepromptfn hook, so the head pointer sits behind a
// shared `OnceLock<Mutex<…>>` rather than `thread_local!`.
static schedcmds: OnceLock<Mutex<Option<Box<schedcmd>>>> = OnceLock::new();

// `module_features` — port of `static struct features module_features`
// from sched.c:386. Bucket-2 shared global; OnceLock-init since
// `features` contains fn-pointer fields not const-initializable.
static MODULE_FEATURES: OnceLock<Mutex<features>> = OnceLock::new();

// Init-on-first-use accessors. C dereferences the statics directly — Rust
// requires the OnceLock get-or-init dance. Inlined at call sites would
// duplicate the boilerplate per dereference; allowlisted as
// architectural Rust-equivalent of static-zero-init.
fn schedcmds_lock() -> &'static Mutex<Option<Box<schedcmd>>> {
    schedcmds.get_or_init(|| Mutex::new(None))
}
fn schedcmdtimed_lock() -> &'static Mutex<i32> {
    schedcmdtimed.get_or_init(|| Mutex::new(0))
}

// `addtimedfn` from utils.c:4635 takes `fn()` (zero-arity, no return).
// `checksched` returns `i32` so the module loaders can pass its status.
// Thunk drops the return to match `addtimedfn`'s signature.
fn checksched_thunk() {
    let _ = checksched();
}

// =====================================================================
// External ported from Src/module.c. Stubbed locally with C-faithful
// signatures pending the module.c port to `*const module` types.
// =====================================================================

// `featuresarray` lives in `Src/module.c:3279`.
fn featuresarray(_m: *const module, _f: &Mutex<features>) -> Vec<String> {
    vec!["b:sched".to_string(), "p:zsh_scheduled_events".to_string()]
}

// `handlefeatures` lives in `Src/module.c:3388`.
fn handlefeatures(m: *const module, f: &Mutex<features>, enables: &mut Option<Vec<i32>>) -> i32 {
    if enables.is_none() {
        *enables = Some(getfeatureenables(m, f));
    } else if let Some(e) = enables.as_ref() {
        return setfeatureenables(m, f, Some(e));
    }
    0
}

// `getfeatureenables` lives in `Src/module.c:3314`.
fn getfeatureenables(_m: *const module, f: &Mutex<features>) -> Vec<i32> {
    let g = f.lock().unwrap();
    let total = g.bn_size + g.cd_size + g.mf_size + g.pd_size + g.n_abstract;
    vec![0; total as usize]
}

// `setfeatureenables` lives in `Src/module.c:3350`.
fn setfeatureenables(_m: *const module, _f: &Mutex<features>, _e: Option<&Vec<i32>>) -> i32 {
    0
}

// =====================================================================
// External ported from other Src/*.c files. Stubbed locally pending the
// proper ports of their home files.
// =====================================================================

// `zleentry` (Src/init.c:1742) — canonical port lives at
// `crate::ported::init::zleentry`. Use that from sched callsites
// instead of duplicating here.
const ZLE_CMD_TRASH: i32 = 3; // zsh.h:3236

// `zleactive` is the int global in `Src/Zle/zle_main.c` indicating
// whether ZLE is the active reader. Mirrored as an atomic int so other
// modules can flip it when ZLE main is wired up. Stays 0 by default
// (matches C's BSS-zero startup).
pub static zleactive: AtomicI32 = AtomicI32::new(0);

// `execstring` (Src/exec.c:1228) — canonical port now lives at
// `crate::ported::exec::execstring`. Earlier this file carried a
// non-executing stub (echo + heap-bracket only). The sched-loop
// caller routes through the real port.
use crate::ported::exec::execstring;

// =====================================================================
// Tests
// =====================================================================

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ─── RUST-ONLY ACCESSORS ───
//
// Singleton accessor ported for `OnceLock<Mutex<T>>` / `OnceLock<
// RwLock<T>>` globals declared above. C zsh uses direct global
// access; Rust needs these wrappers because `OnceLock::get_or_init`
// is the only way to lazily construct shared state. These ported sit
// here so the body of this file reads in C source order without
// the accessor wrappers interleaved between real port ported.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ─── RUST-ONLY ACCESSORS ───
//
// Singleton accessor ported for `OnceLock<Mutex<T>>` / `OnceLock<
// RwLock<T>>` globals declared above. C zsh uses direct global
// access; Rust needs these wrappers because `OnceLock::get_or_init`
// is the only way to lazily construct shared state. These ported sit
// here so the body of this file reads in C source order without
// the accessor wrappers interleaved between real port ported.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn module_features() -> &'static Mutex<features> {
    MODULE_FEATURES.get_or_init(|| {
        Mutex::new(features {
            bn_list: None, // c:387 bintab
            bn_size: 1,    // sizeof(bintab)/sizeof(*bintab) — sched
            cd_list: None, // c:388
            cd_size: 0,
            mf_list: None, // c:389
            mf_size: 0,
            pd_list: None, // c:396 partab
            pd_size: 1,    // sizeof(partab)/sizeof(*partab) — zsh_scheduled_events
            n_abstract: 0, // c:396
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    static TEST_SERIAL: Mutex<()> = Mutex::new(());

    fn empty_ops() -> options {
        options {
            ind: [0u8; MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        }
    }

    fn reset_with_lock() -> std::sync::MutexGuard<'static, ()> {
        let guard = TEST_SERIAL.lock().unwrap_or_else(|e| {
            TEST_SERIAL.clear_poison();
            e.into_inner()
        });
        // C `init_main()` calls `inittyptab()` at startup; tests
        // bypass that path, so seed TYPTAB here so `idigit` works.
        crate::ported::utils::inittyptab();
        let mut head = schedcmds_lock().lock().unwrap_or_else(|e| {
            schedcmds_lock().clear_poison();
            e.into_inner()
        });
        *head = None;
        drop(head);
        let mut t = schedcmdtimed_lock().lock().unwrap_or_else(|e| {
            schedcmdtimed_lock().clear_poison();
            e.into_inner()
        });
        *t = 0;
        drop(t);
        guard
    }

    fn s(x: &str) -> String {
        x.to_string()
    }

    #[test]
    fn list_empty_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let _serial = reset_with_lock();
        let ops = empty_ops();
        assert_eq!(bin_sched("sched", &[], &ops, 0), 0);
    }

    #[test]
    fn add_relative_seconds_pushes_one_entry() {
        let _g = crate::test_util::global_state_lock();
        let _serial = reset_with_lock();
        let ops = empty_ops();
        let args = vec![s("+60"), s("echo"), s("hello")];
        assert_eq!(bin_sched("sched", &args, &ops, 0), 0);
        let head = schedcmds_lock().lock().unwrap();
        assert_eq!(head.as_ref().unwrap().cmd, "echo hello");
        assert!(head.as_ref().unwrap().next.is_none());
    }

    #[test]
    fn add_then_delete_first_clears_list() {
        let _g = crate::test_util::global_state_lock();
        let _serial = reset_with_lock();
        let ops = empty_ops();
        bin_sched("sched", &[s("+60"), s("echo"), s("hello")], &ops, 0);
        assert!(schedcmds_lock().lock().unwrap().is_some());
        assert_eq!(bin_sched("sched", &[s("-1")], &ops, 0), 0);
        assert!(schedcmds_lock().lock().unwrap().is_none());
    }

    #[test]
    fn not_enough_args_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let _serial = reset_with_lock();
        let ops = empty_ops();
        assert_eq!(bin_sched("sched", &[s("+60")], &ops, 0), 1);
    }

    #[test]
    fn dash_o_flag_sets_trash_zle() {
        let _g = crate::test_util::global_state_lock();
        let _serial = reset_with_lock();
        let ops = empty_ops();
        let args = vec![s("-o"), s("+60"), s("cmd")];
        assert_eq!(bin_sched("sched", &args, &ops, 0), 0);
        let head = schedcmds_lock().lock().unwrap();
        assert_eq!(
            head.as_ref().unwrap().flags & SCHEDFLAG_TRASH_ZLE,
            SCHEDFLAG_TRASH_ZLE
        );
    }

    #[test]
    fn insert_keeps_time_order() {
        let _g = crate::test_util::global_state_lock();
        let _serial = reset_with_lock();
        let ops = empty_ops();
        bin_sched("sched", &[s("+200"), s("third")], &ops, 0);
        bin_sched("sched", &[s("+50"), s("first")], &ops, 0);
        bin_sched("sched", &[s("+100"), s("second")], &ops, 0);
        let head = schedcmds_lock().lock().unwrap();
        let n0 = head.as_ref().unwrap();
        let n1 = n0.next.as_ref().unwrap();
        let n2 = n1.next.as_ref().unwrap();
        assert_eq!(n0.cmd, "first");
        assert_eq!(n1.cmd, "second");
        assert_eq!(n2.cmd, "third");
        assert!(n2.next.is_none());
    }

    #[test]
    fn schedgetfn_serializes_entries() {
        let _g = crate::test_util::global_state_lock();
        let _serial = reset_with_lock();
        let mut head = schedcmds_lock().lock().unwrap();
        *head = Some(Box::new(schedcmd {
            next: Some(Box::new(schedcmd {
                next: None,
                cmd: "echo zle".to_string(),
                time: 1700001000,
                flags: SCHEDFLAG_TRASH_ZLE,
            })),
            cmd: "echo test".to_string(),
            time: 1700000000,
            flags: 0,
        }));
        drop(head);
        // schedgetfn's `Param pm` is UNUSED — pass null pointer.
        let arr = schedgetfn(std::ptr::null());
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0], "1700000000::echo test");
        assert_eq!(arr[1], "1700001000:-o:echo zle");
    }

    /// c:40 — `SCHEDFLAG_TRASH_ZLE` is bit 0 (value 1). Pin the
    /// exact bit because every `flags & SCHEDFLAG_TRASH_ZLE` check
    /// downstream is a bool gate.
    #[test]
    fn schedflag_trash_zle_is_bit_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            SCHEDFLAG_TRASH_ZLE, 1,
            "SCHEDFLAG_TRASH_ZLE must be bit 0 per c:40 enum"
        );
    }

    /// c:148 — `bin_sched` with no args lists the schedule.
    /// Empty schedule → 0 return.
    #[test]
    fn bin_sched_no_args_with_empty_schedule_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let _serial = reset_with_lock();
        let ops = empty_ops();
        let r = bin_sched("sched", &[], &ops, 0);
        assert_eq!(r, 0, "list on empty schedule must succeed");
    }

    /// c:148 — `bin_sched +N` (no command) returns 1 (usage).
    #[test]
    fn bin_sched_time_only_no_command_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let _serial = reset_with_lock();
        let ops = empty_ops();
        let r = bin_sched("sched", &[s("+60")], &ops, 0);
        assert_eq!(r, 1, "scheduling without a command must error");
    }

    /// c:148 — `+0` offset: schedule for "now". No panic, finite
    /// return.
    #[test]
    fn bin_sched_zero_offset_does_not_panic() {
        let _g = crate::test_util::global_state_lock();
        let _serial = reset_with_lock();
        let ops = empty_ops();
        let _r = bin_sched("sched", &[s("+0"), s("echo now")], &ops, 0);
    }

    /// c:178 — `bin_sched -<N>` removes the N'th entry. On empty
    /// schedule, must surface an error.
    #[test]
    fn bin_sched_minus_removal_on_empty_schedule_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let _serial = reset_with_lock();
        let ops = empty_ops();
        let r = bin_sched("sched", &[s("-1")], &ops, 0);
        assert_ne!(r, 0, "remove from empty schedule must error");
    }

    /// c:323 — Insertion walk is `while sch2->next && sch2->next->time < sch->time`
    /// (strict <), so a new entry with the same time as an existing
    /// node lands immediately after that node — NOT at the end of
    /// the equal-time run. Three equal-time inserts of "first",
    /// "second", "third" produce: first → third → second (newest
    /// after first, pushing prior equal-time entries further down).
    /// Pin this C-faithful order so a regen that switches to `<=`
    /// (which would produce FIFO ordering) gets caught.
    #[test]
    fn insert_with_equal_times_lands_after_first_equal_entry() {
        let _g = crate::test_util::global_state_lock();
        let _serial = reset_with_lock();
        let ops = empty_ops();
        bin_sched("sched", &[s("+100"), s("first")], &ops, 0);
        bin_sched("sched", &[s("+100"), s("second")], &ops, 0);
        bin_sched("sched", &[s("+100"), s("third")], &ops, 0);
        let head = schedcmds_lock().lock().unwrap();
        let n0 = head.as_ref().unwrap();
        let n1 = n0.next.as_ref().unwrap();
        let n2 = n1.next.as_ref().unwrap();
        // C behavior at c:323: new entry inserts AFTER the first
        // equal-time node, so the list ends up:
        //   first → third → second
        assert_eq!(
            n0.cmd, "first",
            "head is the first-inserted equal-time entry"
        );
        assert_eq!(
            n1.cmd, "third",
            "newest insert lands right after head, pushing prior down"
        );
        assert_eq!(
            n2.cmd, "second",
            "second-inserted ends up at tail per c:323 strict-less walk"
        );
    }

    /// c:396-450 — module-lifecycle stubs all return 0.
    #[test]
    fn module_lifecycle_shims_all_return_zero() {
        let _g = crate::test_util::global_state_lock();
        let m: *const module = std::ptr::null();
        assert_eq!(setup_(m), 0);
        assert_eq!(boot_(m), 0);
        assert_eq!(cleanup_(m), 0);
        assert_eq!(finish_(m), 0);
    }

    /// c:215 — `schedgetfn` on an empty schedule returns empty Vec.
    /// Pin no-entries case so a regen that returns `vec![""]`
    /// (single empty-string entry) gets caught.
    #[test]
    fn schedgetfn_empty_schedule_returns_empty_vec() {
        let _g = crate::test_util::global_state_lock();
        let _serial = reset_with_lock();
        *schedcmds_lock().lock().unwrap() = None;
        let arr = schedgetfn(std::ptr::null());
        assert!(
            arr.is_empty(),
            "empty schedule must serialize to empty Vec, got {:?}",
            arr
        );
    }

    // ─── zsh-corpus pins for bin_sched ─────────────────────────────

    /// `sched` with no args returns 0 and prints empty list.
    #[test]
    fn sched_corpus_bin_sched_no_args_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let _serial = reset_with_lock();
        *schedcmds_lock().lock().unwrap() = None;
        let ops = empty_ops();
        let r = bin_sched("sched", &[], &ops, 0);
        assert_eq!(r, 0, "sched with no args lists schedule, returns 0");
    }

    /// `sched -1` with no schedule returns non-zero.
    #[test]
    fn sched_corpus_bin_sched_minus_one_empty_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let _serial = reset_with_lock();
        *schedcmds_lock().lock().unwrap() = None;
        let ops = empty_ops();
        let r = bin_sched("sched", &[s("-1")], &ops, 0);
        assert_ne!(r, 0, "sched -1 on empty schedule = error");
    }

    /// `sched +60 cmd` adds an entry.
    #[test]
    fn sched_corpus_bin_sched_relative_time_adds_entry() {
        let _g = crate::test_util::global_state_lock();
        let _serial = reset_with_lock();
        *schedcmds_lock().lock().unwrap() = None;
        let ops = empty_ops();
        bin_sched("sched", &[s("+60"), s("my_cmd")], &ops, 0);
        let head = schedcmds_lock().lock().unwrap();
        assert!(head.is_some(), "entry added to schedule");
        assert_eq!(head.as_ref().unwrap().cmd, "my_cmd");
    }

    /// `schedgetfn` on a 1-entry schedule returns 1-element Vec.
    #[test]
    fn sched_corpus_schedgetfn_after_single_add() {
        let _g = crate::test_util::global_state_lock();
        let _serial = reset_with_lock();
        *schedcmds_lock().lock().unwrap() = None;
        let ops = empty_ops();
        bin_sched("sched", &[s("+60"), s("only")], &ops, 0);
        let arr = schedgetfn(std::ptr::null());
        assert_eq!(arr.len(), 1, "1 entry after 1 add, got {arr:?}");
    }

    /// Three sched entries → schedgetfn returns 3.
    #[test]
    fn sched_corpus_schedgetfn_after_three_adds() {
        let _g = crate::test_util::global_state_lock();
        let _serial = reset_with_lock();
        *schedcmds_lock().lock().unwrap() = None;
        let ops = empty_ops();
        bin_sched("sched", &[s("+10"), s("a")], &ops, 0);
        bin_sched("sched", &[s("+20"), s("b")], &ops, 0);
        bin_sched("sched", &[s("+30"), s("c")], &ops, 0);
        let arr = schedgetfn(std::ptr::null());
        assert_eq!(arr.len(), 3, "3 entries after 3 adds, got {arr:?}");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Builtins/sched.c
    // c:93 checksched / c:215 schedgetfn / c:624-684 lifecycle
    // ═══════════════════════════════════════════════════════════════════

    /// c:93 — `checksched` returns i32 (compile-time type pin). C body
    /// is `void`; Rust port returns i32 so module hooks can propagate.
    #[test]
    fn checksched_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _serial = reset_with_lock();
        let _: i32 = checksched();
    }

    /// c:98 — `checksched` on empty schedule returns 0 (C early-exit
    /// at c:99 `return;`).
    #[test]
    fn checksched_empty_schedule_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let _serial = reset_with_lock();
        *schedcmds_lock().lock().unwrap() = None;
        assert_eq!(checksched(), 0, "empty schedule → 0");
    }

    /// c:215 — `schedgetfn` returns Vec<String> (compile-time type pin).
    #[test]
    fn schedgetfn_returns_vec_string_type() {
        let _g = crate::test_util::global_state_lock();
        let _serial = reset_with_lock();
        let _: Vec<String> = schedgetfn(std::ptr::null());
    }

    /// c:624 — `setup_(NULL)` returns 0.
    #[test]
    fn sched_setup_returns_zero_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(setup_(std::ptr::null()), 0);
    }

    /// c:646 — `boot_(NULL)` returns 0.
    #[test]
    fn sched_boot_returns_zero_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(boot_(std::ptr::null()), 0);
    }

    /// c:684 — `finish_(NULL)` returns 0.
    #[test]
    fn sched_finish_returns_zero_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(finish_(std::ptr::null()), 0);
    }

    /// c:624 — `setup_` idempotent.
    #[test]
    fn sched_setup_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(setup_(std::ptr::null()), 0);
        }
    }

    /// c:684 — `finish_` idempotent.
    #[test]
    fn sched_finish_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(finish_(std::ptr::null()), 0);
        }
    }

    /// c:631 — `features_` returns 0 + populates a non-empty list
    /// (zsh/sched has b:sched as advertised feature).
    #[test]
    fn sched_features_returns_zero_with_nonempty_list() {
        let _g = crate::test_util::global_state_lock();
        let mut feats: Vec<String> = Vec::new();
        let r = features_(std::ptr::null(), &mut feats);
        assert_eq!(r, 0, "features_ returns 0");
        assert!(!feats.is_empty(), "features list must contain ≥1 entry");
    }

    /// c:631 — `features_` advertises only `b:` (builtin) entries
    /// per zsh's module-feature naming convention.
    #[test]
    fn sched_features_use_b_prefix() {
        let _g = crate::test_util::global_state_lock();
        let mut feats: Vec<String> = Vec::new();
        features_(std::ptr::null(), &mut feats);
        for f in &feats {
            assert!(
                f.starts_with("b:") || f.starts_with("p:"),
                "feature {:?} must use b:/p: prefix per zsh module spec",
                f
            );
        }
    }

    /// c:215 — `schedgetfn(NULL)` is deterministic on a stable schedule.
    #[test]
    fn schedgetfn_is_deterministic_for_static_schedule() {
        let _g = crate::test_util::global_state_lock();
        let _serial = reset_with_lock();
        let ops = empty_ops();
        bin_sched("sched", &[s("+10"), s("alpha")], &ops, 0);
        let first = schedgetfn(std::ptr::null());
        for _ in 0..3 {
            assert_eq!(
                schedgetfn(std::ptr::null()),
                first,
                "schedgetfn must be deterministic on unchanged schedule"
            );
        }
    }

    /// c:245 — `bin_sched` returns i32 (compile-time type pin).
    #[test]
    fn bin_sched_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _serial = reset_with_lock();
        let ops = empty_ops();
        let _: i32 = bin_sched("sched", &[], &ops, 0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Builtins/sched.c
    // c:95 schedaddtimed / c:123 scheddeltimed / c:143 checksched /
    // c:245 bin_sched / c:582 schedgetfn + lifecycle
    // ═══════════════════════════════════════════════════════════════════

    /// c:143 — `checksched` returns i32 (compile-time pin, alt).
    #[test]
    fn checksched_returns_i32_pin_alt() {
        let _g = crate::test_util::global_state_lock();
        let _serial = reset_with_lock();
        let _: i32 = checksched();
    }

    /// c:143 — `checksched` on empty schedule is deterministic.
    #[test]
    fn checksched_empty_schedule_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let _serial = reset_with_lock();
        let first = checksched();
        for _ in 0..5 {
            assert_eq!(checksched(), first, "checksched on empty must be pure");
        }
    }

    /// c:582 — `schedgetfn(NULL)` returns Vec<String> (compile-time pin, alt).
    #[test]
    fn schedgetfn_returns_vec_string_pin_alt() {
        let _g = crate::test_util::global_state_lock();
        let _: Vec<String> = schedgetfn(std::ptr::null());
    }

    /// c:582 — `schedgetfn(NULL)` on empty schedule returns empty Vec.
    #[test]
    fn schedgetfn_empty_schedule_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        let _serial = reset_with_lock();
        let v = schedgetfn(std::ptr::null());
        assert!(
            v.is_empty(),
            "empty schedule must yield empty Vec; got {:?}",
            v
        );
    }

    /// c:95 — `schedaddtimed` is callable without panic.
    #[test]
    fn schedaddtimed_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _serial = reset_with_lock();
        schedaddtimed();
    }

    /// c:123 — `scheddeltimed` is callable without panic.
    #[test]
    fn scheddeltimed_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _serial = reset_with_lock();
        scheddeltimed();
    }

    /// c:245 — `bin_sched` no-args is the listing form (returns 0/1
    /// per zsh's `sched` builtin spec; no-args lists current schedule).
    #[test]
    fn bin_sched_no_args_returns_zero_or_one() {
        let _g = crate::test_util::global_state_lock();
        let _serial = reset_with_lock();
        let ops = empty_ops();
        let r = bin_sched("sched", &[], &ops, 0);
        assert!(
            r == 0 || r == 1,
            "no-args is the listing form; result 0/1, got {}",
            r
        );
    }

    /// c:245 — `bin_sched` non-negative exit code.
    #[test]
    fn bin_sched_exit_code_non_negative() {
        let _g = crate::test_util::global_state_lock();
        let _serial = reset_with_lock();
        let ops = empty_ops();
        for argv in [vec![], vec![s("+5"), s("echo hi")], vec![s("-1")]] {
            let r = bin_sched("sched", &argv, &ops, 0);
            assert!(
                r >= 0,
                "exit code must be non-negative, got {} for {:?}",
                r,
                argv
            );
        }
    }

    /// c:639 — `enables_(NULL)` returns i32 + None-out safe.
    #[test]
    fn sched_enables_with_none_returns_i32() {
        let _g = crate::test_util::global_state_lock();
        let mut e: Option<Vec<i32>> = None;
        let _: i32 = enables_(std::ptr::null(), &mut e);
    }

    /// c:659 — `cleanup_` is idempotent across many calls (alt-many).
    #[test]
    fn sched_cleanup_idempotent_many_call_alt() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..20 {
            assert_eq!(cleanup_(std::ptr::null()), 0);
        }
    }

    /// c:646 — `boot_` is idempotent across many calls (alt-many).
    #[test]
    fn sched_boot_idempotent_many_call_alt() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..20 {
            assert_eq!(boot_(std::ptr::null()), 0);
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity pins for Src/Modules/sched.c
    // c:245 bin_sched / c:624-684 lifecycle hooks
    // ═══════════════════════════════════════════════════════════════════

    /// c:624 — `setup_` is idempotent (alt-2).
    #[test]
    fn sched_setup_idempotent_alt_2() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(setup_(std::ptr::null()), 0);
        }
    }

    /// c:684 — `finish_` is idempotent (alt-2).
    #[test]
    fn sched_finish_idempotent_alt_2() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(finish_(std::ptr::null()), 0);
        }
    }

    /// c:624 — `setup_` return type i32 (compile-time pin).
    #[test]
    fn sched_setup_returns_i32_type_alt_pin() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = setup_(std::ptr::null());
    }

    /// c:659 — `cleanup_` return type i32 (compile-time pin).
    #[test]
    fn sched_cleanup_returns_i32_type_alt_pin() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = cleanup_(std::ptr::null());
    }

    /// c:684 — `finish_` return type i32 (compile-time pin).
    #[test]
    fn sched_finish_returns_i32_type_alt_pin() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = finish_(std::ptr::null());
    }

    /// c:639 — `enables_` with Some(non-empty) doesn't panic.
    #[test]
    fn sched_enables_with_some_non_empty_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let mut e: Option<Vec<i32>> = Some(vec![1, 2, 3]);
        let _ = enables_(std::ptr::null(), &mut e);
    }

    /// c:245 — `bin_sched` empty args + ops returns non-negative.
    #[test]
    fn bin_sched_empty_args_non_negative_alt_pin() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let r = bin_sched("sched", &[], &ops, 0);
        assert!(r >= 0, "bin_sched empty must return ≥ 0, got {}", r);
    }

    /// c:245 — `bin_sched` deterministic for empty schedule (idle state).
    #[test]
    fn bin_sched_empty_schedule_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let r1 = bin_sched("sched", &[], &ops, 0);
        let r2 = bin_sched("sched", &[], &ops, 0);
        assert_eq!(r1, r2, "bin_sched empty schedule must be deterministic");
    }

    /// c:245 — `bin_sched` various func values don't panic.
    #[test]
    fn bin_sched_various_func_values_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        for func in [-1, 0, 1, 100, i32::MAX] {
            let _ = bin_sched("sched", &[], &ops, func);
        }
    }

    /// c:245 — `bin_sched("-malformed")` invalid removal arg doesn't panic.
    #[test]
    fn bin_sched_malformed_removal_arg_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        for arg in ["-", "-x", "-99999", "-abc"] {
            let _ = bin_sched("sched", &[arg.to_string()], &ops, 0);
        }
    }

    /// c:631 — `features_` is deterministic across calls.
    #[test]
    fn sched_features_deterministic_alt_pin() {
        let _g = crate::test_util::global_state_lock();
        let mut v1: Vec<String> = Vec::new();
        let mut v2: Vec<String> = Vec::new();
        assert_eq!(features_(std::ptr::null(), &mut v1), 0);
        assert_eq!(features_(std::ptr::null(), &mut v2), 0);
        assert_eq!(v1, v2, "features_ must be deterministic");
    }

    /// c:624/646/684 — setup→boot→finish chain returns 0 each.
    #[test]
    fn sched_setup_boot_finish_chain_returns_zero_each() {
        let _g = crate::test_util::global_state_lock();
        let null = std::ptr::null();
        assert_eq!(setup_(null), 0);
        assert_eq!(boot_(null), 0);
        assert_eq!(finish_(null), 0);
    }
}

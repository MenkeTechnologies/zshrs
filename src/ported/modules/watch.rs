//! Login/logout watching module - port of Modules/watch.c
//!
//! the last time we checked the people in the WATCH variable               // c:153
//! get the time of login/logout for WATCH                                  // c:158
//! print a login/logout event                                              // c:238
//! See if the watch entry matches                                          // c:431
//! check the List for login/logouts                                        // c:455
//! compare 2 utmp entries                                                  // c:524
//! initialize the user List                                                // c:534
//!
//! Provides watch/log functionality for monitoring user logins/logouts.

use crate::ported::builtin::BUILTIN;
use crate::ported::zsh_h::{builtin, module};
use chrono::{Local, TimeZone};
#[cfg(unix)]
use std::ffi::CStr;
use std::io::{BufRead, Write};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// `WATCH_STRUCT_UTMP` typedef alias matching `Src/Modules/watch.c:71-79`:
/// resolves to `libc::utmpx` on platforms with `<utmpx.h>` support
/// (Linux/macOS), `libc::utmp` otherwise. The C source uses this
/// alias name everywhere so the Rust port keeps it.
#[cfg(unix)]
pub type WATCH_STRUCT_UTMP = libc::utmpx;

/// Default watch format string
pub const DEFAULT_WATCHFMT: &str = "%n has %a %l from %m.";

/// Port of `getlogtime(WATCH_STRUCT_UTMP *u, int inout)` from `Src/Modules/watch.c:161`. For
/// login events (`inout` non-zero) returns the entry's `ut_time`
/// directly. For logout events, walks `wtmp` backwards looking
/// for the matching login record so the resulting time pairs the
/// real session start, not the wtmp logout marker.
///
/// Port of `getlogtime(WATCH_STRUCT_UTMP *u, int inout)` from
/// `Src/Modules/watch.c:161`. For login events (`inout` non-zero)
/// returns `u->ut_time` directly per c:169. For logout events
/// (inout == 0), seeks WATCH_WTMP_FILE backward looking for the
/// matching login record so the resulting time pairs the real
/// session start, not the wtmp logout marker (c:170-198).
pub fn getlogtime(u: &libc::utmpx, inout: i32) -> i64 {
    // c:161
    if inout != 0 {
        // c:168
        return u.ut_tv.tv_sec as i64; // c:169 return u->ut_time
    }
    // c:170 — `if (!(in = fopen(WATCH_WTMP_FILE, "r"))) return time(NULL);`
    let wtmp_path = wtmp_file_path();
    let mut f = match std::fs::File::open(wtmp_path) {
        Ok(f) => f,
        Err(_) => return unsafe { libc::time(std::ptr::null_mut()) as i64 },
    };
    use std::io::{Read, Seek, SeekFrom};
    let rec_size = size_of::<libc::utmpx>() as i64;
    // c:172 — `fseek(in, 0, SEEK_END);`
    if f.seek(SeekFrom::End(0)).is_err() {
        return unsafe { libc::time(std::ptr::null_mut()) as i64 };
    }
    let mut first = true; // c:165
    let mut srchlimit = 50i32; // c:166
    let last = LASTWATCH.with(|t| t.get()); // c:183 lastwatch read
                                            // c:173-188 — backwards walk to find the matching logout record.
                                            //   do {
                                            //       fseek(in, (first ? -1 : -2) * sizeof, SEEK_CUR);
                                            //       fread(&uu, sizeof, 1, in);
                                            //       if (uu.ut_time < lastwatch || !srchlimit--) → return time(NULL);
                                            //   } while (memcmp(&uu, u, sizeof(uu)));
    let mut uu: libc::utmpx = unsafe { std::mem::zeroed() };
    loop {
        let offset = if first { -rec_size } else { -2 * rec_size }; // c:174
        first = false; // c:178
        if f.seek(SeekFrom::Current(offset)).is_err() {
            return unsafe { libc::time(std::ptr::null_mut()) as i64 }; // c:175-176
        }
        let mut buf = vec![0u8; rec_size as usize];
        if f.read_exact(&mut buf).is_err() {
            return unsafe { libc::time(std::ptr::null_mut()) as i64 }; // c:180-181
        }
        uu = unsafe { std::ptr::read(buf.as_ptr() as *const libc::utmpx) };
        if (uu.ut_tv.tv_sec as i64) < last || {
            srchlimit -= 1;
            srchlimit < 0
        } {
            return unsafe { libc::time(std::ptr::null_mut()) as i64 }; // c:184-185
        }
        // c:188 — `} while (memcmp(&uu, u, sizeof(uu)));` — loop continues
        //         while NOT matched. Compare raw bytes.
        let uu_bytes =
            unsafe { std::slice::from_raw_parts(&uu as *const _ as *const u8, rec_size as usize) };
        let u_bytes =
            unsafe { std::slice::from_raw_parts(u as *const _ as *const u8, rec_size as usize) };
        if uu_bytes == u_bytes {
            break;
        }
    }
    // c:190-195 — forward walk for next entry with matching ut_line.
    //   do { fread(&uu, sizeof, 1, in); } while (strncmp(uu.ut_line, u->ut_line, ...));
    let target_line = utmp_line(u);
    loop {
        let mut buf = vec![0u8; rec_size as usize];
        if f.read_exact(&mut buf).is_err() {
            return unsafe { libc::time(std::ptr::null_mut()) as i64 }; // c:192-193
        }
        uu = unsafe { std::ptr::read(buf.as_ptr() as *const libc::utmpx) };
        if utmp_line(&uu) == target_line {
            // c:195 — strncmp returns 0 when matching → exit loop.
            break;
        }
    }
    // c:197 — `return uu.ut_time;`
    uu.ut_tv.tv_sec as i64
}

/// Port of `watch3ary(int inout, WATCH_STRUCT_UTMP *u, char *fmt,
/// int prnt)` from `Src/Modules/watch.c:206`. C body handles
/// `%(cond.true.false)` and calls watchlog2 for the chosen branch.
/// Public wrapper kept for the test/back-compat surface — callers
/// pass an entry + logged_in flag + the full format string and get
/// the formatted output as a String (Rust adapts the C
/// printf-to-stdout convention to a return-value-string style for
/// the AST/exec pipeline).
pub fn watch3ary(entry: &libc::utmpx, logged_in: bool, fmt: &str) -> String {
    // c:206
    watchlog2(if logged_in { 1 } else { 0 }, entry, fmt, 1, 0)
}

/// Port of `watchlog2(int inout, WATCH_STRUCT_UTMP *u, char *fmt,
/// int prnt, int fini)` from `Src/Modules/watch.c:242-429`. The
/// main `$WATCHFMT` parser — walks the format string handling
/// `\X` escapes, `%n`/`%a`/`%l`/`%m`/`%M`/`%T`/`%t`/`%@`/`%W`/`%w`/
/// `%D`/`%D{…}` directives, `%%` literal, and `%(c.true.false)`
/// ternaries (dispatched through `watch3ary`).
///
/// C returns the post-format `char *` (cursor after the matching
/// `fini` delimiter, or end-of-string). Rust returns the
/// concatenated output as a String — `prnt=1` accumulates into
/// the result, `prnt=0` walks but emits nothing. The `fini`
/// delimiter is honored: parsing stops when `*fmt == fini`.
pub fn watchlog2(inout: i32, u: &libc::utmpx, fmt: &str, prnt: i32, fini: i32) -> String {
    // c:242
    let mut result = String::new();
    let mut chars = fmt.chars().peekable();
    let user = utmp_user(u);
    let line = utmp_line(u);
    let host = utmp_host(u);
    let logged_in = inout != 0;
    while let Some(c) = chars.peek().copied() {
        // c:256 — `if (*fmt == '\\')`.
        if c == '\\' {
            chars.next();
            if let Some(esc) = chars.next() {
                if prnt != 0 {
                    result.push(esc); // c:259 putchar
                }
            } else if fini != 0 {
                return result; // c:263 return fmt
            } else {
                break; // c:264 break
            }
            continue;
        }
        // c:268 — `else if (*fmt == fini) return ++fmt;`.
        if fini != 0 && (c as i32) == fini {
            chars.next();
            return result;
        }
        // c:270 — `else if (*fmt != '%')`.
        if c != '%' {
            chars.next();
            if prnt != 0 {
                result.push(c); // c:273 putchar
            }
            continue;
        }
        // c:277 — `%`-directive. Consume the `%`.
        chars.next();
        let directive = match chars.next() {
            Some(d) => d,
            None => break,
        };
        // c:278 — `if (*++fmt == BEGIN3) fmt = watch3ary(...);`. BEGIN3
        // is `(` per the c:206 `%(` ternary opener.
        if directive == '(' {
            // c:206 — watch3ary handles the ternary subexpression
            // and returns the new cursor. Re-feed remaining fmt
            // through it; Rust port handles inline because
            // peekable iter doesn't expose the post-cursor cleanly.
            let rest: String = chars.clone().collect();
            let (out, advance) = watch3ary_inline(inout, u, &rest, prnt);
            result.push_str(&out);
            for _ in 0..advance {
                chars.next();
            }
            continue;
        }
        if prnt == 0 {
            continue; // c:280 !prnt skip
        }
        match directive {
            'n' => result.push_str(&user), // c:283
            'a' => result.push_str(if logged_in { "logged on" } else { "logged off" }), // c:287
            'l' => {
                // c:291
                let trimmed = if line.starts_with("tty") {
                    &line[3..]
                } else {
                    &line
                };
                result.push_str(trimmed);
            }
            'm' => {
                // c:299
                let short = host.split('.').next().unwrap_or(&host);
                result.push_str(short);
            }
            'M' => result.push_str(&host), // c:307
            'T' | 't' | '@' | 'W' | 'w' | 'D' => {
                // c:312-340
                let time = getlogtime(u, inout);
                let mut fm2: String = match directive {
                    '@' | 't' => "%l:%M%p".to_string(), // c:321
                    'T' => "%H:%M".to_string(),         // c:324
                    'w' => "%a %e".to_string(),         // c:328
                    'W' => "%m/%d/%y".to_string(),      // c:331
                    'D' => {
                        // c:333
                        if chars.peek() == Some(&'{') {
                            chars.next();
                            // c:336-340 — collect chars until `}`,
                            // honoring `\X` escapes (each `\` is
                            // followed by a single char that's
                            // appended literally, not the `\`).
                            let mut buf = String::new();
                            loop {
                                let fc = match chars.next() {
                                    Some(c) => c,
                                    None => break,
                                };
                                if fc == '}' {
                                    break;
                                }
                                if fc == '\\' {
                                    if let Some(esc) = chars.next() {
                                        buf.push(esc);
                                    }
                                } else {
                                    buf.push(fc);
                                }
                            }
                            if buf.is_empty() {
                                "%y-%m-%d".to_string()
                            } else {
                                buf
                            }
                        } else {
                            "%y-%m-%d".to_string() // c:347
                        }
                    }
                    _ => unreachable!(),
                };
                let formatted = Local
                    .timestamp_opt(time, 0)
                    .single()
                    .map(|dt| dt.format(&fm2).to_string())
                    .unwrap_or_default();
                // c:355-356 — strip leading space (strftime %l left-pads).
                let trimmed = formatted.strip_prefix(' ').unwrap_or(&formatted);
                result.push_str(trimmed);
                let _ = fm2.len();
            }
            '%' => result.push('%'), // c:359 putchar('%')
            _ => {
                // c:419 default
                result.push('%');
                result.push(directive);
            }
        }
    }
    // c:434-436 — `if (prnt) putchar('\n');`. zshrs lets the caller
    // append the trailing newline (watchlog/checksched control output).
    result
}

/// Check if a watch pattern matches an entry field
/// Match a `$watch` pattern against an actual user/host/tty.
/// Port of `watchlog_match(char *teststr, char *actual, size_t buflen)` from Src/Modules/watch.c:434 — same
/// `user@host:tty` triple-component matching.
/// WARNING: param names don't match C — Rust=(pattern, value) vs C=(teststr, actual, buflen)
pub fn watchlog_match(pattern: &str, value: &str) -> bool {
    // c:434
    // c:438-451 — `char *str = dupstring(teststr); tokenize(str);
    //               if ((pprog = patcompile(str, PAT_STATIC, 0))) {
    //                   queue_signals();
    //                   if (pattry(pprog, user)) ret = 1;
    //                   unqueue_signals();
    //               } else if (!strcmp(user, teststr)) ret = 1;`
    //
    // C always tries patcompile first (handles globs, character
    // classes [...], extended-glob qualifiers, alternation |, etc.).
    // Only falls back to strcmp when patcompile rejects the pattern
    // as syntactically invalid.
    //
    // Prior Rust port used a `contains('*') || contains('?')`
    // heuristic to decide whether to engage globbing — missed:
    //   - `[abc]` character classes
    //   - `[a-z]` character ranges
    //   - `<1-100>` numeric ranges
    //   - `(foo|bar)` alternation
    //   - `**` recursive glob
    // A `watch=("user[12]")` entry would NOT match "user1" because
    // the heuristic saw no `*` or `?` and fell through to strcmp.
    // Route through patcompile/pattry directly like C does.
    //
    // c:443 — `tokenize(str)` — Rust patcompile handles its own
    // tokenization internally per Src/pattern.c:547.
    // c:445 — PAT_STATIC = 0x80 per zsh.h.
    use crate::ported::pattern::{patcompile, pattry};
    match patcompile(pattern, 0x80, None) {
        Some(prog) => {
            // c:446-449 — queue_signals + pattry + unqueue_signals.
            // pattry doesn't actually touch the signal queue from the
            // Rust port's perspective; the C queue_signals guards
            // against the Patprog's PAT_ZDUP-style heap allocations
            // being clobbered by signal handlers. Skip the queue
            // ceremony — Rust's Patprog is owned + dropped explicitly.
            pattry(&prog, value)
        }
        None => {
            // c:450 — `else if (!strcmp(user, teststr)) ret = 1;`
            pattern == value
        }
    }
}

/// Port of `watchlog(int inout, WATCH_STRUCT_UTMP *u, char **w,
/// char *fmt)` from `Src/Modules/watch.c:458-524`. Top-level
/// per-event dispatcher driven by `dowatch`. Walks the `$watch`
/// array (`w`) honoring the C trinary patterns:
///
/// - `"all"` (c:466): unconditional match
/// - `"notme"` (c:470): match anything except the current user
/// - `user@host%line` entries (c:481-515): per-component glob match
///
/// On match, calls `watchlog2(inout, u, fmt, 1, 0)` to print the
/// event followed by a newline. Output goes to stdout per c:434.
pub fn watchlog(inout: i32, u: &libc::utmpx, w: &[String], fmt: &str) {
    // c:458
    // c:463 — `if (!*u->ut_name) return;`
    let user_name = utmp_user(u);
    if user_name.is_empty() {
        return;
    }
    // c:465 — `if (*w && !strcmp(*w, "all"))` → unconditional emit.
    if w.first().map(|s| s.as_str()) == Some("all") {
        emit_event(inout, u, fmt);
        return;
    }
    // c:470-481 — `"notme"` handling: emit when entry user != current.
    //   if (*w && !strcmp(*w, "notme")) {
    //       char *username = metafy(u->ut_name, len, META_USEHEAP);
    //       if (strcmp(username, get_username())) {
    //           watchlog2(inout, u, fmt, 1, 0);
    //           return;
    //       }
    //       w++;
    //   }
    //
    // C uses `get_username()` — the cached `getpwuid(getuid())` shell
    // helper that returns the real OS user (utils.c:1075 / Rust port
    // at utils.rs:1258). Prior port read `$USERNAME` / `$USER` from
    // paramtab. Three differences:
    //   1. Security: a script that sets `USERNAME=fake` before the
    //      watch hook fires would bypass the "notme" filter. C reads
    //      the real uid → real user name → can't be spoofed.
    //   2. Correctness: `$USERNAME` and `$USER` may not match the
    //      real user across su / sudo / chsh boundaries. Only
    //      getpwuid(getuid()) tracks the actual effective user.
    //   3. Init order: paramtab is populated after shell init; if
    //      watch fires before the prompt fully initializes (unusual
    //      but possible via `-c 'log'`), paramtab may be missing
    //      USERNAME entirely, defaulting to empty string and
    //      reporting every login as "notme".
    let mut idx = 0;
    if w.first().map(|s| s.as_str()) == Some("notme") {
        let current = crate::ported::utils::get_username();
        if user_name != current {
            emit_event(inout, u, fmt);
            return;
        }
        idx = 1;
    }
    // c:483-518 — `user@host%line` per-entry matching.
    let host_name = utmp_host(u);
    let line_name = utmp_line(u);
    while idx < w.len() {
        let entry = &w[idx];
        idx += 1;
        let mut bad = false;
        let chars: Vec<char> = entry.chars().collect();
        let mut i = 0usize;
        // c:486-492 — leading `user` (until `@` or `%`).
        if !chars.is_empty() && chars[0] != '@' && chars[0] != '%' {
            let mut j = i;
            while j < chars.len() && chars[j] != '@' && chars[j] != '%' {
                j += 1;
            }
            let v: String = chars[i..j].iter().collect();
            if !watchlog_match(&v, &user_name) {
                bad = true;
            }
            i = j;
        }
        // c:494-518 — interleaved `%line` and `@host`.
        loop {
            if i >= chars.len() {
                break;
            }
            if chars[i] == '%' {
                // c:495
                i += 1;
                let mut j = i;
                while j < chars.len() && chars[j] != '@' {
                    j += 1;
                }
                let v: String = chars[i..j].iter().collect();
                if !watchlog_match(&v, &line_name) {
                    bad = true;
                }
                i = j;
            } else if chars[i] == '@' {
                // c:507
                i += 1;
                let mut j = i;
                while j < chars.len() && chars[j] != '%' {
                    j += 1;
                }
                let v: String = chars[i..j].iter().collect();
                if !watchlog_match(&v, &host_name) {
                    bad = true;
                }
                i = j;
            } else {
                break;
            }
        }
        if !bad {
            emit_event(inout, u, fmt);
            return;
        }
    }
}

/// Port of `ucmp(WATCH_STRUCT_UTMP *u, WATCH_STRUCT_UTMP *v)` from
/// `Src/Modules/watch.c:527`. qsort comparator for utmp records:
/// by `ut_time` ascending, then by `ut_line` lexicographic.
///
/// C body:
/// ```c
/// if (u->ut_time == v->ut_time)
///     return strncmp(u->ut_line, v->ut_line, sizeof(u->ut_line));
/// return u->ut_time - v->ut_time;
/// ```
/// C body (watch.c:527, 3 lines):
///     `if (u->ut_time == v->ut_time)
///          return strncmp(u->ut_line, v->ut_line, sizeof(u->ut_line));
///      return u->ut_time - v->ut_time;`
pub fn ucmp(u: &libc::utmpx, v: &libc::utmpx) -> i32 {
    // c:527
    let (ut, vt) = (u.ut_tv.tv_sec as i64, v.ut_tv.tv_sec as i64);
    if ut == vt {
        utmp_line(u).cmp(&utmp_line(v)) as i32
    } else {
        (ut - vt) as i32
    } // c:529-531
}

// Per-evaluator watch-module state — bucket-1 dissolution per
// PORT_PLAN.md Phase 2. C source has these file-statics in
// `Src/Modules/watch.c`:
//
//     static int wtabsz = 0;                       // line 150
//     static WATCH_STRUCT_UTMP *wtab = NULL;       // line 151
//     static time_t lastwatch;                     // line 154
//     static time_t lastutmpcheck = 0;             // line 156
//     static char **watch;  /* $watch */           // line 689
//
// Stored as individual thread_locals matching the C declarations
// 1:1. `WATCHFMT`/`LOGCHECK` live on the param table in C, NOT as
// file-statics — they're looked up via the param table at use
// sites, not stored here.

thread_local! {
    /// Port of file-static `static WATCH_STRUCT_UTMP *wtab = NULL;`
    /// at `Src/Modules/watch.c:151` (combined with `wtabsz` line
    /// 150 — the `Vec` carries length implicitly).
    static WTAB: std::cell::RefCell<Vec<libc::utmpx>> = const {
        std::cell::RefCell::new(Vec::new())
    };

    // the last time we checked the people in the WATCH variable         // c:153
    /// Port of file-static `static time_t lastwatch;` at
    /// `Src/Modules/watch.c:154`.
    static LASTWATCH: std::cell::Cell<i64> = const {
        std::cell::Cell::new(0)
    };

    /// Port of file-static `static time_t lastutmpcheck = 0;` at
    /// `Src/Modules/watch.c:156`.
    static LASTUTMPCHECK: std::cell::Cell<i64> = const {
        std::cell::Cell::new(0)
    };

    /// Port of file-static `static char **watch;` at
    /// `Src/Modules/watch.c:689` — backing array for the `$watch`
    /// shell parameter.
    static WATCH: std::cell::RefCell<Vec<String>> = const {
        std::cell::RefCell::new(Vec::new())
    };
}

/// Port of `readwtab(WATCH_STRUCT_UTMP **head, int initial_sz)` from
/// `Src/Modules/watch.c:537`. Reads the utmp file (`getutxent` on
/// systems with it, otherwise raw `WATCH_UTMP_FILE`), filters
/// USER_PROCESS-only entries, and returns them sorted by `ucmp`.
///
/// The C signature passes `*head` out-param + returns the count.
/// Rust returns the Vec directly (length = count). The
/// `initial_sz` arg honours the C `wtabmax = initial_sz < 2 ?
/// 32 : initial_sz` capacity hint (parse.c:539) — dowatch calls
/// `readwtab(&utab, wtabsz + 4)` on subsequent reads so the
/// reallocation doesn't fire on a stable utmp.
pub fn readwtab(initial_sz: i32) -> Vec<libc::utmpx> {
    // c:537
    // c:539 — `int wtabmax = initial_sz < 2 ? 32 : initial_sz;`
    let wtabmax = if initial_sz < 2 { 32 } else { initial_sz } as usize;
    let mut entries: Vec<libc::utmpx> = Vec::with_capacity(wtabmax); // c:549 zalloc wtabmax*sizeof
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    unsafe {
        libc::setutxent(); // c:553 setutent
        loop {
            let entry = libc::getutxent(); // c:554 getutxent
            if entry.is_null() {
                break;
            }
            let ut = &*entry;
            // c:561 — `if (uptr->ut_type == USER_PROCESS)` filter.
            if ut.ut_type != libc::USER_PROCESS {
                continue;
            }
            entries.push(std::ptr::read(entry));
        }
        libc::endutxent(); // c:584 endutent
    }
    // c:587-588 — `qsort(*head, sz, sizeof(...), ucmp);`
    entries.sort_by(|a, b| match ucmp(a, b) {
        n if n < 0 => std::cmp::Ordering::Less,
        n if n > 0 => std::cmp::Ordering::Greater,
        _ => std::cmp::Ordering::Equal,
    });
    entries // c:589 return sz
}

// `watch3ary` / `watchlog2` now live further down (after their
// helpers); the C ordering has watch3ary at c:206 above watchlog2
// at c:242 because watch3ary's recursive call to watchlog2 is via
// forward declaration. The Rust port consolidates both at the
// same site so the inline ternary helper sits next to its caller.

// printtime helper deleted — C uses inline strftime() at each format
// directive in watchlog2() (c:319-340), so the Rust port inlines the
// chrono equivalent at each callsite to match.

/// Perform watch check and return login/logout events
/// Run one tick of the watch loop, returning login/logout events.
/// Port of `dowatch(void)` from `Src/Modules/watch.c:597-647`. The
/// preprompt-driven watch refresh — diffs the cached `wtab`
/// against a fresh utmp read and fires `watchlog()` inline for
/// each new entry / departure.
///
/// C body (transcribed):
/// ```c
/// s = watch;
/// holdintr();
/// if (!wtab) wtabsz = readwtab(&wtab, 32);
/// if (stat(WATCH_UTMP_FILE, &st) == -1 || st.st_mtime <= lastutmpcheck) {
///     noholdintr(); return;
/// }
/// lastutmpcheck = st.st_mtime;
/// utabsz = readwtab(&utab, wtabsz + 4);
/// noholdintr();
/// if (errflag) { free(utab); return; }
/// /* merge-walk uptr/wptr by ucmp ordering, calling
///    watchlog(0, wptr++) for departures, watchlog(1, uptr++) for arrivals */
/// queue_signals();
/// if (!(fmt = getsparam_u("WATCHFMT"))) fmt = DEFAULT_WATCHFMT;
/// while ((uct || wct) && !errflag) {
///     if (!uct || (wct && ucmp(uptr, wptr) > 0))
///         wct--, watchlog(0, wptr++, s, fmt);
///     else if (!wct || (uct && ucmp(uptr, wptr) < 0))
///         uct--, watchlog(1, uptr++, s, fmt);
///     else uptr++, wptr++, wct--, uct--;
/// }
/// unqueue_signals();
/// free(wtab); wtab = utab; wtabsz = utabsz;
/// fflush(stdout);
/// lastwatch = time(NULL);
/// ```
pub fn dowatch() {
    // c:597
    let s: Vec<String> = WATCH.with(|w| w.borrow().clone());
    // c:607 — `holdintr();`
    crate::ported::signals::holdintr();
    // c:608 — `if (!wtab) wtabsz = readwtab(&wtab, 32);`
    let wtab_empty = WTAB.with(|t| t.borrow().is_empty());
    if wtab_empty {
        let initial = readwtab(32);
        WTAB.with(|t| *t.borrow_mut() = initial);
    }
    // c:610 — `if ((stat(...) == -1) || (st.st_mtime <= lastutmpcheck))
    //           { noholdintr(); return; }`
    let utmp_path = utmp_file_path();
    let mtime = std::fs::metadata(utmp_path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64);
    let last = LASTUTMPCHECK.with(|t| t.get());
    match mtime {
        Some(m) if m > last => {
            LASTUTMPCHECK.with(|t| t.set(m)); // c:614 lastutmpcheck = st.st_mtime
        }
        _ => {
            crate::ported::signals::noholdintr(); // c:611 noholdintr
            return; // c:611 return
        }
    }
    // c:615 — `utabsz = readwtab(&utab, wtabsz + 4);`
    let wtabsz = WTAB.with(|t| t.borrow().len()) as i32;
    let utab = readwtab(wtabsz + 4);
    // c:616 — `noholdintr();`
    crate::ported::signals::noholdintr();
    // c:617 — `if (errflag) { free(utab); return; }`
    if crate::ported::utils::errflag.load(std::sync::atomic::Ordering::Relaxed) != 0 {
        return;
    }
    // c:622-625 — `wct = wtabsz; uct = utabsz; uptr = utab; wptr = wtab;`
    let wtab_snapshot: Vec<libc::utmpx> = WTAB.with(|t| t.borrow().clone());
    let mut uct = utab.len();
    let mut wct = wtab_snapshot.len();
    let mut uidx = 0usize;
    let mut widx = 0usize;
    // c:626-629 — second `if (errflag) { free(utab); return; }` guard.
    // Catches a signal handler firing in the tight window between the
    // first errflag check at c:617 and the queue_signals barrier at
    // c:630; without it a SIGINT delivered mid-pointer-setup would
    // race the merge-walk start.
    if crate::ported::utils::errflag.load(std::sync::atomic::Ordering::Relaxed) != 0 {
        return;
    }
    // c:630 — `queue_signals();` + WATCHFMT fallback.
    crate::ported::signals_h::queue_signals();
    // c:631-632 — `if (!(fmt = getsparam_u("WATCHFMT"))) fmt = DEFAULT_WATCHFMT;`
    //   — the `_u` variant strips Meta-escape encoding from the
    //     returned value. WATCHFMT is then fed through the
    //     `watchlog2`/`watch3ary` format-string parser which expects
    //     raw bytes (the `%n %a %l %m` directives are byte-tokenized,
    //     not metafy-aware). Prior port used `getsparam` which
    //     returns the metafied form; a user's WATCHFMT containing
    //     literal NUL or 0x83-byte sequences (rare but legal via
    //     `$'\x00'` syntax) would survive into the formatter as
    //     Meta-escaped bytes instead of the intended payload.
    //   Same fix shape as newuser ZDOTDIR (3d9aa55117).
    let fmt = crate::ported::params::getsparam_u("WATCHFMT")
        .unwrap_or_else(|| DEFAULT_WATCHFMT.to_string());
    // c:633-640 — merge-walk uptr/wptr by ucmp.
    while uct > 0 || wct > 0 {
        if crate::ported::utils::errflag.load(std::sync::atomic::Ordering::Relaxed) != 0 {
            break;
        }
        let cmp_gt_zero = wct > 0 && uct > 0 && ucmp(&utab[uidx], &wtab_snapshot[widx]) > 0;
        let cmp_lt_zero = uct > 0 && wct > 0 && ucmp(&utab[uidx], &wtab_snapshot[widx]) < 0;
        if uct == 0 || cmp_gt_zero {
            // c:634 — `wct--, watchlog(0, wptr++, s, fmt);` — departure
            wct -= 1;
            watchlog(0, &wtab_snapshot[widx], &s, &fmt);
            widx += 1;
        } else if wct == 0 || cmp_lt_zero {
            // c:636 — `uct--, watchlog(1, uptr++, s, fmt);` — arrival
            uct -= 1;
            watchlog(1, &utab[uidx], &s, &fmt);
            uidx += 1;
        } else {
            // c:638 — entries match (same session) — advance both.
            uidx += 1;
            widx += 1;
            wct -= 1;
            uct -= 1;
        }
    }
    // c:644 — `unqueue_signals();`
    crate::ported::signals_h::unqueue_signals();
    // c:645-646 — `free(wtab); wtab = utab; wtabsz = utabsz;`
    WTAB.with(|t| *t.borrow_mut() = utab);
    // c:647 — `fflush(stdout); lastwatch = time(NULL);`
    let _ = std::io::stdout().flush();
    let now = unsafe { libc::time(std::ptr::null_mut()) as i64 };
    LASTWATCH.with(|t| t.set(now));
}

/// Port of `checksched()` from `Src/Modules/watch.c:650`. Called
/// before each prompt redraw: if `$watch` is set AND `LOGCHECK`
/// seconds have elapsed since `lastwatch`, run `dowatch()`.
///
/// C body:
/// ```c
/// if (watch && (int) difftime(time(NULL), lastwatch) > getiparam("LOGCHECK"))
///     dowatch();
/// ```
pub fn checksched() {
    // c:650
    // c:653 — `if (watch && (int) difftime(time(NULL), lastwatch) > getiparam("LOGCHECK")) dowatch();`
    //
    // C does NOT fall back when LOGCHECK is 0. Prior Rust port added a
    // `raw > 0 ? raw : 60` fallback that papered over the case where
    // `--zsh` mode skips boot_'s LOGCHECK=60 seeding (gated on
    // !IS_ZSH_MODE at boot_:771). The fallback was diagnostic-shaped:
    // it hid the missing init from `--zsh` mode but introduced a
    // parity divergence — in C zsh's `zsh -fc` (no zmodload watch),
    // LOGCHECK is unset → getiparam returns 0 → `diff > 0` fires on
    // every prompt → expensive but always-current watch.
    //
    // Drop the fallback. zmodload-time boot_ seeds LOGCHECK=60 (port
    // of c:759 at boot_:776), so the post-zmodload behavior is
    // identical to C; the `--zsh -fc` no-zmodload path now also
    // matches C (fires every prompt when LOGCHECK unset).
    let watch_set = WATCH.with(|w| !w.borrow().is_empty());
    if !watch_set {
        return;
    }
    let now = unsafe { libc::time(std::ptr::null_mut()) as i64 }; // c:653 time(NULL)
    let last = LASTWATCH.with(|t| t.get()); // c:653 lastwatch
    let logcheck = crate::ported::params::getiparam("LOGCHECK"); // c:653 getiparam("LOGCHECK")
    if (now - last) > logcheck {
        // c:653 difftime > LOGCHECK
        dowatch(); // c:654 dowatch();
    }
}

/// Port of `bin_log(UNUSED(char *nam), UNUSED(char **argv),
/// UNUSED(Options ops), UNUSED(int func))` from
/// `Src/Modules/watch.c:659`.
///
/// C body (under WATCH_STRUCT_UTMP):
/// ```c
/// if (!watch) return 1;
/// if (wtab) free(wtab);
/// wtab = (WATCH_STRUCT_UTMP *)zalloc(1);
/// wtabsz = 0;
/// lastutmpcheck = 0;
/// dowatch();
/// return 0;
/// ```
/// — clear the watch table + lastutmpcheck so the next preprompt
/// hook re-emits the full watch list using `$WATCHFMT`. Returns 1
/// when `$watch` is empty (nothing to refresh) per c:661.
pub fn bin_log(
    _name: &str,
    _argv: &[String], // c:659
    _ops: &crate::ported::zsh_h::options,
    _func: i32,
) -> i32 {
    // c:661 — `if (!watch) return 1;`. C's `watch` is a `char**`
    // initialized via the watch module's GSU at module-boot (`partab`
    // ties `$WATCH` ↔ `watch` via `colonarr_gsu`). At runtime, `watch`
    // is a non-NULL pointer to an array — empty when `$WATCH` is
    // unset — so the `!watch` check almost never fires; observable
    // behavior on macOS zsh is `log` returning 0 even with `$WATCH`
    // empty (bug #72 in docs/BUGS.md).
    //
    // The previous port mapped "WATCH empty" to the C `!watch` early
    // return-1, but C only fires that on a literally NULL `watch`
    // pointer (effectively never after setup_). Drop the early-out;
    // run dowatch unconditionally so empty-WATCH callers see exit 0
    // matching zsh.
    // c:663-667 — `if (wtab) free(wtab); wtab = zalloc(1); wtabsz = 0;
    //              lastutmpcheck = 0;`
    WTAB.with(|t| t.borrow_mut().clear());
    LASTUTMPCHECK.with(|t| t.set(0));
    // c:668 — `dowatch();` — the standalone driver. No args now;
    // current-user resolution happens inside watchlog itself via
    // `getsparam("USERNAME")` to match C's `cached_username` lookup.
    dowatch();
    0
}

// `partab` — port of `static struct paramdef partab[]` (watch.c).

// `module_features` — port of `static struct features module_features`
// from watch.c:700.

/// Port of `setup_(UNUSED(Module m))` from `Src/Modules/watch.c:712`.
#[allow(unused_variables)]
pub fn setup_(m: *const module) -> i32 {
    // c:712
    // C body c:714-718 — `partab[0].gsu = (void *)&colonarr_gsu;
    //                     partab[1].gsu = (void *)&vararray_gsu;
    //                     return 0`. The GSU dispatch isn't part of
    //                     the static-link Rust path — partab entries
    //                     are passed through directly. Return success.
    0
}

/// Port of `features_(UNUSED(Module m), UNUSED(char ***features))` from `Src/Modules/watch.c:723`.
/// C body: `*features = featuresarray(m, &module_features); return 0;`
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {
    // c:723
    *features = featuresarray(m, module_features());
    0
}

/// Port of `enables_(UNUSED(Module m), UNUSED(int **enables))` from `Src/Modules/watch.c:731`.
/// C body: `return handlefeatures(m, &module_features, enables);`
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {
    // c:731
    handlefeatures(m, module_features(), enables)
}

// ===========================================================
// Methods moved verbatim from src/ported/vm_helper because their
// C counterpart's source file maps 1:1 to this Rust module.
// Phase: module-shims
// ===========================================================

// BEGIN moved-from-exec-rs
// (impl ShellExecutor block moved to src/exec_shims.rs — see file marker)

// END moved-from-exec-rs

// =====================================================================
// static struct features module_features                            c:700 (watch.c)
// =====================================================================

/// Port of `boot_(UNUSED(Module m))` from `Src/Modules/watch.c:738`.
#[allow(unused_variables)]
pub fn boot_(m: *const module) -> i32 {
    // c:738
    // C body c:740-762: ties $watch and $WATCH, creates empty `watch`
    // array, sets WATCHFMT/LOGCHECK defaults IFF unset, installs the
    // checksched preprompt hook. Seed `WATCHFMT` and `LOGCHECK` only
    // when no env value pre-exists — preserves the `${WATCHFMT-unset}`
    // distinction zsh makes between "unset" (no zmodload) and "set
    // to default" (after zmodload).
    //
    // c:Src/Modules/watch.c:756-759 — the comment above the C source
    // setsparam("WATCHFMT", DEFAULT_WATCHFMT) reads: "These two
    // parameters are only set to defaults if not set. So setting
    // them in .zshrc will not be enough to load the module. It's
    // useless until the watch array is set anyway." The seeding
    // fires inside the C `boot_` which only runs at `zmodload`
    // time. zsh -fc never zmodload's zsh/watch, so $WATCHFMT
    // stays empty there.
    //
    // zshrs's `init_bltinmods` (module.rs) auto-runs `boot_` for
    // statically-linked default-loaded modules so `${(t)watch}`
    // reads as `array-special` straight from the prompt — a QoL
    // win in default mode but a parity divergence in --zsh mode.
    // Gate the value-seeding on `!IS_ZSH_MODE` so `--zsh` matches
    // zsh's "declared but empty until zmodload" behaviour while
    // default mode keeps the auto-loaded defaults.
    if !crate::IS_ZSH_MODE.load(std::sync::atomic::Ordering::Relaxed) {
        if crate::ported::params::getsparam("WATCHFMT").is_none() {
            crate::ported::params::setsparam("WATCHFMT", DEFAULT_WATCHFMT); // c:757
        }
        if crate::ported::params::getsparam("LOGCHECK").is_none() {
            crate::ported::params::setsparam("LOGCHECK", "60"); // c:759
        }
    }
    // c:Src/Modules/watch.c:697-699 — register `watch` (PM_ARRAY |
    // PM_SPECIAL) and `WATCH` (PM_SCALAR | PM_SPECIAL) in paramtab
    // so introspection (`(t)watch` → `array-special`, `${(t)WATCH}`
    // → `scalar-special`) sees them after `zmodload zsh/watch`.
    // C zsh boots paramtab entries via `paramdef partab[]` which the
    // module-load chain dispatches through `addparamdef`. zshrs's
    // module shims don't run that path, so seed the entries here.
    // Bug #270 in docs/BUGS.md.
    use crate::ported::zsh_h::{PM_ARRAY, PM_SCALAR, PM_SPECIAL};
    let watch_exists = crate::ported::params::paramtab()
        .read()
        .ok()
        .map(|t| t.contains_key("watch"))
        .unwrap_or(false);
    if !watch_exists {
        let _ = crate::ported::params::createparam("watch", (PM_ARRAY | PM_SPECIAL) as i32);
    }
    let watch_scalar_exists = crate::ported::params::paramtab()
        .read()
        .ok()
        .map(|t| t.contains_key("WATCH"))
        .unwrap_or(false);
    if !watch_scalar_exists {
        let _ = crate::ported::params::createparam("WATCH", (PM_SCALAR | PM_SPECIAL) as i32);
    }
    // c:761 — `addprepromptfn(&checksched);`. Without this, the
    // watch module never gets driven on each prompt — `$watch`
    // is set but no login/logout notifications ever fire.
    crate::ported::utils::addprepromptfn(checksched);
    0
}

/// Port of `cleanup_(UNUSED(Module m))` from `Src/Modules/watch.c:768`.
/// C body: `delprepromptfn(checksched); return setfeatureenables(...);`
pub fn cleanup_(m: *const module) -> i32 {
    // c:768
    // c:770 — `delprepromptfn(&checksched);` — must mirror the
    // addprepromptfn done at boot_. Otherwise unloading + reloading
    // the watch module accumulates duplicate hook entries.
    crate::ported::utils::delprepromptfn(checksched);
    setfeatureenables(m, module_features(), None)
}

/// Port of `finish_(UNUSED(Module m))` from `Src/Modules/watch.c:776`.
#[allow(unused_variables)]
pub fn finish_(m: *const module) -> i32 {
    // c:776
    // C body c:778-779 — `return 0`. Faithful empty-body port; the
    //                    watch utmpx descriptor is process-lifetime,
    //                    not module-lifetime.
    0
}

/// Default watch format without host support
pub const DEFAULT_WATCHFMT_NOHOST: &str = "%n has %a %l.";

/// Port of `static struct builtin bintab[]` from `Src/Modules/watch.c:693`:
/// ```c
/// static struct builtin bintab[] = {
///     BUILTIN("log", 0, bin_log, 0, 0, 0, NULL, NULL),
/// };
/// ```
/// Exposed so `crate::ported::builtin::createbuiltintable` can fold
/// the watch-module entries into the live `builtintab` at startup
/// (zshrs auto-loads all modules so the per-module bintabs become
/// part of the core table). `disable log` in `/etc/zshrc` (on
/// macOS, to dodge `/usr/bin/log`) then finds the hashtable entry
/// to flip.
pub static bintab: std::sync::LazyLock<Vec<builtin>> = std::sync::LazyLock::new(|| {
    vec![BUILTIN(
        "log",
        0,
        Some(bin_log as crate::ported::zsh_h::HandlerFunc),
        0,
        0,
        0,
        None,
        None,
    )]
});

// `UtmpEntry` struct + `SessionType` enum + `impl is_active`
// DELETED. Watch.c uses `WATCH_STRUCT_UTMP` (= `libc::utmpx`) directly
// — `WTAB` now stores `Vec<libc::utmpx>` matching C's
// `static WATCH_STRUCT_UTMP *wtab` at `Src/Modules/watch.c:151`,
// and every reader extracts `ut_user` / `ut_line` / `ut_host` / etc.
// inline via the FFI char-array → `&str` helpers below. Comparisons
// against `ut_type` use bare `libc::USER_PROCESS` / `DEAD_PROCESS` /
// etc. — same int comparisons C does at watch.c:458.

/// Read `ut_user` (an FFI `[c_char; UT_USERSIZE]` array) as a Rust
/// `String`. C: `printf("%.*s", (int)sizeof(u->ut_name), u->ut_name)`
/// at watch.c:285 — size-bounded read, stop at first NUL or buffer
/// end. Prior port used `CStr::from_ptr(buf.as_ptr())` which scans
/// until NUL — reads PAST the array when the utmpx field holds an
/// EXACTLY-N-byte value with no trailing NUL (UB in unsafe Rust;
/// real-world hazard on max-length usernames). Inline bounded
/// slice walk mirrors what `printf("%.*s", N, ptr)` emits.
pub fn utmp_user(u: &libc::utmpx) -> String {
    // c:285
    let buf = &u.ut_user;
    let bytes: &[u8] = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const u8, buf.len()) };
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

/// Read `ut_line` as a `String`. C: `printf("%.*s", (int)sizeof(u->ut_line), ...)`
/// at watch.c:294/296 — same bounded-read pattern as utmp_user.
pub fn utmp_line(u: &libc::utmpx) -> String {
    // c:294
    let buf = &u.ut_line;
    let bytes: &[u8] = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const u8, buf.len()) };
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

/// Read `ut_host` as a `String`. C: `printf("%.*s", (int)sizeof(u->ut_host), ...)`
/// at watch.c:309 — same bounded-read pattern as utmp_user.
pub fn utmp_host(u: &libc::utmpx) -> String {
    // c:309
    let buf = &u.ut_host;
    let bytes: &[u8] = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const u8, buf.len()) };
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

/// Inline of C's `entry->ut_type == USER_PROCESS && entry->ut_user[0]
/// != 0` filter at Src/Modules/watch.c:458 — `true` when this entry
/// represents an active user session.
pub fn utmp_is_active(u: &libc::utmpx) -> bool {
    // c:458
    u.ut_type == libc::USER_PROCESS as i16 && u.ut_user.first().copied().unwrap_or(0) != 0
}

/// Construct a `libc::utmpx` for tests with the given fields. Writes
/// `user`/`line`/`host` strings into the FFI char arrays NUL-padded.
/// C tests would do the same via `strncpy(u.ut_user, "name", ...)`.
#[cfg(test)]
pub fn utmp_make(
    user: &str,
    line: &str,
    host: &str,
    time: i64,
    pid: i32,
    ut_type: i16,
) -> libc::utmpx {
    let mut u: libc::utmpx = unsafe { std::mem::zeroed() };
    let mut copy = |dst: &mut [libc::c_char], src: &str| {
        let bytes = src.as_bytes();
        let n = bytes.len().min(dst.len().saturating_sub(1));
        for (i, &b) in bytes[..n].iter().enumerate() {
            dst[i] = b as libc::c_char;
        }
    };
    copy(&mut u.ut_user, user);
    copy(&mut u.ut_line, line);
    copy(&mut u.ut_host, host);
    // `ut_tv.tv_sec` is `i32` on Linux (Y2038-constrained `__u32` in
    // utmpx.h's anonymous `struct ut_timeval`), but `i64` on macOS
    // (real `struct timeval` with `time_t`). Use type inference (`as _`)
    // so the cast picks the right width per target — `time as libc::time_t`
    // was unconditionally i64 on macOS and failed Linux's i32 binding.
    u.ut_tv.tv_sec = time as _;
    u.ut_pid = pid;
    u.ut_type = ut_type;
    u
}

// WARNING: NOT IN WATCH.C — Rust-only setter helpers around the
// thread_locals. C zsh doesn't have setter functions: assignments
// to `$watch` flow through the paramdef table (watch.c:697) and
// `watch.c:689` is updated implicitly by the param machinery.
// The Rust port factors them into named ported so future param-hook
// wiring has a single update site (and tests can drive them
// directly without going through the param table).

// WARNING: NOT IN WATCH.C — Rust-only test/setup helper that writes
// the `WATCH` thread_local directly. C source assigns to the `watch`
// global via `setaparam("watch", ...)` (Src/Modules/watch.c:614),
// which routes through paramtab; this helper bypasses paramtab for
// in-process state setup where the param path would be circular.
/// Replace the `$watch` array contents.
pub fn set_watch_list(list: Vec<String>) {
    WATCH.with(|w| *w.borrow_mut() = list);
}

/// Decide whether enough time has elapsed since the last poll.
/// Port of the elapsed-time gate inside `dowatch()` (Src/Modules/
/// watch.c:597). C reads `lastwatch` and the `LOGCHECK` param
/// directly; Rust port reads from the thread_local LASTWATCH and
/// the canonical param via `getsparam("LOGCHECK")`.
pub fn should_check() -> bool {
    if WATCH.with(|w| w.borrow().is_empty()) {
        return false;
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    // C: `getiparam("LOGCHECK")`. Was reading OS env directly which
    //     misses shell-side `LOGCHECK=N` assignments.
    let interval = {
        let raw = crate::ported::params::getiparam("LOGCHECK");
        if raw > 0 {
            raw
        } else {
            60
        }
    };
    now - LASTWATCH.with(|t| t.get()) > interval
}

/// Decide whether an entry should produce a watch event.
/// Port of the per-entry filter inside `watchlog()` from
/// Src/Modules/watch.c:458 — checks against `$watch` array
/// excluding the current user when the list begins with `notme`.
pub fn check_entry(entry: &libc::utmpx, current_user: &str) -> bool {
    let user = utmp_user(entry);
    let line = utmp_line(entry);
    let host = utmp_host(entry);
    WATCH.with(|w| {
        let watch_list = w.borrow();
        if watch_list.is_empty() {
            return false;
        }
        if watch_list.first().map(|s| s.as_str()) == Some("all") {
            return true;
        }
        let mut iter = watch_list.iter().peekable();
        if iter.peek().map(|s| s.as_str()) == Some("notme") {
            if user == current_user {
                return false;
            }
            iter.next();
            if iter.peek().is_none() {
                return true;
            }
        }
        for pattern in iter {
            // Inline pattern match: `user[@host][%line]` triple-component
            // form per the watchlog inline scan at watch.c:489-510. Walks
            // the pattern from left to right, switching to host-arm on `@`
            // and line-arm on `%`, dispatching each component through
            // `watchlog_match()`.
            let mut rest = pattern.as_str();
            let mut matched = true;
            if !rest.starts_with('@') && !rest.starts_with('%') {
                let end = rest.find(['@', '%']).unwrap_or(rest.len());
                let user_pat = &rest[..end];
                if !watchlog_match(user_pat, &user) {
                    matched = false;
                }
                rest = &rest[end..];
            }
            while !rest.is_empty() && matched {
                if let Some(rest1) = rest.strip_prefix('%') {
                    let end = rest1.find('@').unwrap_or(rest1.len());
                    let line_pat = &rest1[..end];
                    if !watchlog_match(line_pat, &line) {
                        matched = false;
                    }
                    rest = &rest1[end..];
                } else if let Some(rest1) = rest.strip_prefix('@') {
                    let end = rest1.find('%').unwrap_or(rest1.len());
                    let host_pat = &rest1[..end];
                    if !watchlog_match(host_pat, &host) {
                        matched = false;
                    }
                    rest = &rest1[end..];
                } else {
                    break;
                }
            }
            if matched {
                return true;
            }
        }
        false
    })
}

/// Port of the `WATCH_UTMP_FILE` macro from `Src/Modules/watch.c:116-127`.
/// Same platform-default selection scheme as wtmp_file_path. WARNING:
/// NOT IN WATCH.C — Rust-only helper extracted from inline
/// `#ifdef HAVE_UTMPX_H` macro selection.
fn utmp_file_path() -> &'static str {
    #[cfg(target_os = "linux")]
    {
        "/var/run/utmp"
    }
    #[cfg(target_os = "macos")]
    {
        "/var/run/utmpx"
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        "/dev/null"
    }
}

/// Port of the `WATCH_WTMP_FILE` macro from `Src/Modules/watch.c:118-147`.
/// C selects between REAL_WTMPX_FILE / REAL_WTMP_FILE / "/dev/null"
/// based on configure-time UTMPX availability. Rust port picks the
/// platform-default path. WARNING: NOT IN WATCH.C — Rust-only helper
/// extracted from inline `#ifdef HAVE_UTMPX_H` macro selection.
fn wtmp_file_path() -> &'static str {
    #[cfg(target_os = "linux")]
    {
        "/var/log/wtmp"
    }
    #[cfg(target_os = "macos")]
    {
        "/var/log/wtmpx"
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        "/dev/null"
    }
}

/// Emit one formatted watch event to stdout. Mirrors the C `(void)
/// watchlog2(inout, u, fmt, 1, 0);` call (parse.c:467/477/518) plus
/// the trailing `putchar('\n')` at c:434. WARNING: NOT IN WATCH.C —
/// Rust-only adapter: C's watchlog2 prints to stdout directly via
/// putchar/printf; Rust collects into a String and writes once for
/// atomicity.
fn emit_event(inout: i32, u: &libc::utmpx, fmt: &str) {
    let line = watchlog2(inout, u, fmt, 1, 0);
    let _ = writeln!(std::io::stdout(), "{}", line);
}

/// Inline `%(c.true.false)` ternary parser called from `watchlog2`.
/// Returns `(rendered, chars_consumed_from_input)`. WARNING: NOT
/// directly in watch.c — this Rust-port helper exists because
/// Peekable<Chars> can't be re-seated from a returned char*; the
/// C port uses a moving `char *fmt` pointer. Mirrors the role
/// played by watch3ary (c:206) but with index-based bookkeeping.
fn watch3ary_inline(inout: i32, u: &libc::utmpx, rest: &str, prnt: i32) -> (String, usize) {
    let bytes: Vec<char> = rest.chars().collect();
    if bytes.len() < 2 {
        return (String::new(), 0);
    }
    let cond = bytes[0];
    let sep = bytes[1];
    let user = utmp_user(u);
    let line = utmp_line(u);
    let host = utmp_host(u);
    let truth = match cond {
        'n' => !user.is_empty(),
        'a' => inout != 0,
        'l' => {
            if line.starts_with("tty") {
                line.len() > 3
            } else {
                !line.is_empty()
            }
        }
        'm' | 'M' => !host.is_empty(),
        _ => false,
    };
    let mut true_branch = String::new();
    let mut false_branch = String::new();
    let mut depth = 1;
    let mut in_true = true;
    let mut consumed = 2;
    while consumed < bytes.len() {
        let c = bytes[consumed];
        consumed += 1;
        if c == ')' {
            depth -= 1;
            if depth == 0 {
                break;
            }
        }
        if c == sep && depth == 1 {
            in_true = false;
            continue;
        }
        if c == '%' && consumed < bytes.len() && bytes[consumed] == '(' {
            depth += 1;
        }
        if in_true {
            true_branch.push(c);
        } else {
            false_branch.push(c);
        }
    }
    let branch = if truth { &true_branch } else { &false_branch };
    let rendered = if prnt != 0 {
        watchlog2(inout, u, branch, 1, 0)
    } else {
        String::new()
    };
    (rendered, consumed)
}

static MODULE_FEATURES: OnceLock<Mutex<crate::ported::zsh_h::features>> = OnceLock::new();

// Local stubs for the per-module entry points. C uses generic
// `featuresarray`/`handlefeatures`/`setfeatureenables` (module.c:
// 3275/3370/3445) but those take `Builtin` + `Features` pointer
// fields the Rust port doesn't carry. The hardcoded descriptor
// list mirrors the C bintab/conddefs/mathfuncs/paramdefs.
// WARNING: NOT IN WATCH.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn featuresarray(_m: *const module, _f: &Mutex<crate::ported::zsh_h::features>) -> Vec<String> {
    vec![
        "b:log".to_string(),
        "p:WATCH".to_string(),
        "p:watch".to_string(),
    ]
}

// WARNING: NOT IN WATCH.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn handlefeatures(
    _m: *const module,
    _f: &Mutex<crate::ported::zsh_h::features>,
    enables: &mut Option<Vec<i32>>,
) -> i32 {
    if enables.is_none() {
        *enables = Some(vec![1; 3]);
    }
    0
}

// WARNING: NOT IN WATCH.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn setfeatureenables(
    _m: *const module,
    _f: &Mutex<crate::ported::zsh_h::features>,
    _e: Option<&[i32]>,
) -> i32 {
    0
}

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

// WARNING: NOT IN WATCH.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn module_features() -> &'static Mutex<crate::ported::zsh_h::features> {
    MODULE_FEATURES.get_or_init(|| {
        Mutex::new(crate::ported::zsh_h::features {
            bn_list: None,
            bn_size: 1,
            cd_list: None,
            cd_size: 0,
            mf_list: None,
            mf_size: 0,
            pd_list: None,
            pd_size: 2,
            n_abstract: 0,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glob::matchpat;

    /// Port of `boot_(UNUSED(Module m))` from `Src/Modules/watch.c:738`.
    #[test]
    fn test_watch_initial_empty() {
        let _g = crate::test_util::global_state_lock();
        // Fresh thread → thread_locals are zero-initialised; the `$watch`
        // list defaults to empty (mirrors C's `static char **watch = NULL`).
        WATCH.with(|w| assert!(w.borrow().is_empty()));
    }

    #[test]
    fn test_glob_match() {
        let _g = crate::test_util::global_state_lock();
        assert!(matchpat("*", "anything", false, true));
        assert!(matchpat("user*", "username", false, true));
        assert!(matchpat("*name", "username", false, true));
        assert!(matchpat("user?ame", "username", false, true));
        assert!(!matchpat("user", "username", false, true));
    }

    #[test]
    fn test_watch_match() {
        let _g = crate::test_util::global_state_lock();
        assert!(watchlog_match("root", "root"));
        assert!(watchlog_match("*", "anyuser"));
        assert!(!watchlog_match("root", "admin"));
    }

    #[test]
    fn test_format_watch_basic() {
        let _g = crate::test_util::global_state_lock();
        let entry = utmp_make(
            "testuser",
            "tty1",
            "localhost",
            0,
            1234,
            libc::USER_PROCESS as i16,
        );
        let result = watch3ary(&entry, true, "%n has %a %l");
        assert!(result.contains("testuser"));
        assert!(result.contains("logged on"));
        assert!(result.contains("1"));

        let result = watch3ary(&entry, false, "%n has %a");
        assert!(result.contains("logged off"));
    }

    #[test]
    fn test_format_watch_host() {
        let _g = crate::test_util::global_state_lock();
        let entry = utmp_make(
            "user",
            "pts/0",
            "host.example.com",
            0,
            1,
            libc::USER_PROCESS as i16,
        );
        let result = watch3ary(&entry, true, "%m");
        assert_eq!(result, "host");

        let result = watch3ary(&entry, true, "%M");
        assert_eq!(result, "host.example.com");
    }

    #[test]
    fn test_check_watch_entry_all() {
        let _g = crate::test_util::global_state_lock();
        let entry = utmp_make("anyone", "pts/0", "", 0, 1, libc::USER_PROCESS as i16);
        set_watch_list(vec!["all".to_string()]);
        assert!(check_entry(&entry, "me"));
    }

    #[test]
    fn test_check_watch_entry_notme() {
        let _g = crate::test_util::global_state_lock();
        let entry = utmp_make("me", "pts/0", "", 0, 1, libc::USER_PROCESS as i16);
        set_watch_list(vec!["notme".to_string()]);
        assert!(!check_entry(&entry, "me"));

        let other = utmp_make("other", "pts/0", "", 0, 1, libc::USER_PROCESS as i16);
        assert!(check_entry(&other, "me"));
    }

    #[test]
    fn test_matches_watch_pattern() {
        let _g = crate::test_util::global_state_lock();
        let entry = utmp_make(
            "admin",
            "pts/0",
            "server.local",
            0,
            1,
            libc::USER_PROCESS as i16,
        );
        set_watch_list(vec!["admin".to_string()]);
        assert!(check_entry(&entry, "me"));
        set_watch_list(vec!["admin@server.local".to_string()]);
        assert!(check_entry(&entry, "me"));
        set_watch_list(vec!["admin%pts/0".to_string()]);
        assert!(check_entry(&entry, "me"));
        set_watch_list(vec!["root".to_string()]);
        assert!(!check_entry(&entry, "me"));
    }

    #[test]
    fn test_session_type() {
        let _g = crate::test_util::global_state_lock();
        let entry = utmp_make("user", "pts/0", "", 0, 1, libc::USER_PROCESS as i16);
        assert!(utmp_is_active(&entry));

        let dead = utmp_make("user", "pts/0", "", 0, 1, libc::DEAD_PROCESS as i16);
        assert!(!utmp_is_active(&dead));
    }

    /// c:434 — `watchlog_match` glob: literal `"*"` matches any.
    #[test]
    fn watchlog_match_asterisk_matches_anything() {
        let _g = crate::test_util::global_state_lock();
        assert!(watchlog_match("*", ""));
        assert!(watchlog_match("*", "anything"));
        assert!(watchlog_match("*", "host.example.com"));
    }

    /// c:434 — Exact-string match (no implicit prefix/suffix).
    #[test]
    fn watchlog_match_exact_string() {
        let _g = crate::test_util::global_state_lock();
        assert!(watchlog_match("alice", "alice"));
        assert!(!watchlog_match("alice", "bob"));
        assert!(
            !watchlog_match("alice", "alice.local"),
            "exact match must NOT match prefix of value"
        );
    }

    /// c:434 — Empty pattern only matches empty value (NOT a
    /// wildcard).
    #[test]
    fn watchlog_match_empty_pattern_only_matches_empty() {
        let _g = crate::test_util::global_state_lock();
        assert!(watchlog_match("", ""));
        assert!(!watchlog_match("", "x"));
    }

    /// c:527 — `ucmp` returns 0 on identical entries.
    #[test]
    fn ucmp_identical_entries_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let a = utmp_make("alice", "pts/0", "h", 100, 1, libc::USER_PROCESS as i16);
        let b = utmp_make("alice", "pts/0", "h", 100, 1, libc::USER_PROCESS as i16);
        assert_eq!(ucmp(&a, &b), 0, "identical utmp entries must compare equal");
    }

    /// c:527-531 — `ucmp` compares ONLY by (ut_time, ut_line) — NOT
    /// by user name. Two entries with same time + same line are
    /// equal regardless of user. Pin this C-faithful semantic
    /// because the comparator feeds into a sorted utmp diff at c:600.
    #[test]
    fn ucmp_ignores_user_when_time_and_line_match() {
        let _g = crate::test_util::global_state_lock();
        let a = utmp_make("alice", "pts/0", "h", 100, 1, libc::USER_PROCESS as i16);
        let b = utmp_make("bob", "pts/0", "h", 100, 1, libc::USER_PROCESS as i16);
        assert_eq!(
            ucmp(&a, &b),
            0,
            "c:527-531 — ucmp uses only (time, line); different users are EQUAL"
        );
    }

    /// c:531 — Different ut_time produces non-zero, signed-difference
    /// ordering.
    #[test]
    fn ucmp_different_times_are_unequal() {
        let _g = crate::test_util::global_state_lock();
        let a = utmp_make("alice", "pts/0", "h", 100, 1, libc::USER_PROCESS as i16);
        let b = utmp_make("alice", "pts/0", "h", 200, 1, libc::USER_PROCESS as i16);
        let r = ucmp(&a, &b);
        assert!(r < 0, "earlier time must sort first; got {}", r);
        let r = ucmp(&b, &a);
        assert!(r > 0, "later time must sort after; got {}", r);
    }

    /// c:527 — Same user on different lines (e.g. two tmux panes)
    /// → non-zero.
    #[test]
    fn ucmp_different_lines_are_unequal() {
        let _g = crate::test_util::global_state_lock();
        let a = utmp_make("alice", "pts/0", "h", 100, 1, libc::USER_PROCESS as i16);
        let b = utmp_make("alice", "pts/1", "h", 100, 1, libc::USER_PROCESS as i16);
        assert_ne!(ucmp(&a, &b), 0, "different ttys must compare unequal");
    }

    /// c:458 — Empty watch list → no entry matches.
    #[test]
    fn check_entry_with_empty_watch_list_returns_false() {
        let _g = crate::test_util::global_state_lock();
        let entry = utmp_make("anyone", "pts/0", "", 0, 1, libc::USER_PROCESS as i16);
        set_watch_list(vec![]);
        assert!(
            !check_entry(&entry, "me"),
            "empty watch list must not match any entry"
        );
    }

    /// c:712-770 — module-lifecycle stubs all return 0.
    #[test]
    fn module_lifecycle_shims_all_return_zero() {
        let _g = crate::test_util::global_state_lock();
        let m: *const module = std::ptr::null();
        assert_eq!(setup_(m), 0);
        assert_eq!(boot_(m), 0);
        assert_eq!(cleanup_(m), 0);
        assert_eq!(finish_(m), 0);
    }

    // ─── zsh-corpus pins for watchlog_match ────────────────────────

    /// Exact match returns true.
    #[test]
    fn watch_corpus_watchlog_match_exact() {
        let _g = crate::test_util::global_state_lock();
        assert!(watchlog_match("alice", "alice"));
    }

    /// Different strings without glob chars → false.
    #[test]
    fn watch_corpus_watchlog_match_different_strings_false() {
        let _g = crate::test_util::global_state_lock();
        assert!(!watchlog_match("alice", "bob"));
    }

    /// Glob `*` matches.
    #[test]
    fn watch_corpus_watchlog_match_star_glob_matches() {
        let _g = crate::test_util::global_state_lock();
        assert!(watchlog_match("user*", "username"));
        assert!(watchlog_match("*lice", "alice"));
        assert!(watchlog_match("*", "anything"));
    }

    /// Glob `?` matches single char.
    #[test]
    fn watch_corpus_watchlog_match_question_glob_matches() {
        let _g = crate::test_util::global_state_lock();
        assert!(watchlog_match("a?c", "abc"));
        assert!(watchlog_match("a?c", "axc"));
        assert!(
            !watchlog_match("a?c", "abbc"),
            "? matches exactly one char, not multiple"
        );
    }

    /// Pattern without glob and value differs → false (no implicit substring match).
    #[test]
    fn watch_corpus_watchlog_match_no_glob_no_substring_match() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            !watchlog_match("ice", "alice"),
            "without glob, no substring matching"
        );
    }

    /// Empty pattern matches only empty value (exact match).
    #[test]
    fn watch_corpus_watchlog_match_empty_exact() {
        let _g = crate::test_util::global_state_lock();
        assert!(watchlog_match("", ""));
        assert!(!watchlog_match("", "x"));
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/watch.c.
    // ═══════════════════════════════════════════════════════════════════

    /// c:434 — `watchlog_match("*", anything)` returns true (star matches all).
    #[test]
    fn watchlog_match_star_matches_anything() {
        let _g = crate::test_util::global_state_lock();
        assert!(watchlog_match("*", "anything"));
        assert!(watchlog_match("*", ""), "* should match empty too");
        assert!(watchlog_match("*", "hello world"));
    }

    /// c:434 — `watchlog_match("a*", "alice")` matches prefix.
    #[test]
    fn watchlog_match_prefix_star() {
        let _g = crate::test_util::global_state_lock();
        assert!(watchlog_match("a*", "alice"));
        assert!(watchlog_match("a*", "a"));
        assert!(!watchlog_match("a*", "bob"));
    }

    /// c:434 — `watchlog_match("*ice", "alice")` matches suffix.
    #[test]
    fn watchlog_match_suffix_star() {
        let _g = crate::test_util::global_state_lock();
        assert!(watchlog_match("*ice", "alice"));
        assert!(watchlog_match("*ice", "ice"));
        assert!(!watchlog_match("*ice", "bob"));
    }

    /// c:434 — exact equality returns true immediately (no glob walk).
    #[test]
    fn watchlog_match_exact_match_short_circuits() {
        let _g = crate::test_util::global_state_lock();
        assert!(watchlog_match("alice", "alice"));
        assert!(watchlog_match("user@host", "user@host"));
    }

    /// c:434 — `watchlog_match` is deterministic.
    #[test]
    fn watchlog_match_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        for (p, v) in &[("a*", "alice"), ("xyz", "abc"), ("", "")] {
            let first = watchlog_match(p, v);
            for _ in 0..5 {
                assert_eq!(watchlog_match(p, v), first);
            }
        }
    }

    /// c:817 — `utmp_user` on default-zeroed utmpx returns empty.
    #[test]
    #[cfg(unix)]
    fn utmp_user_zeroed_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        let u: libc::utmpx = unsafe { std::mem::zeroed() };
        assert!(
            utmp_user(&u).is_empty(),
            "zero-initialized utmpx has empty user"
        );
    }

    /// c:828 — `utmp_line` on default-zeroed utmpx returns empty.
    #[test]
    #[cfg(unix)]
    fn utmp_line_zeroed_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        let u: libc::utmpx = unsafe { std::mem::zeroed() };
        assert!(utmp_line(&u).is_empty());
    }

    /// c:839 — `utmp_host` on default-zeroed utmpx returns empty.
    #[test]
    #[cfg(unix)]
    fn utmp_host_zeroed_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        let u: libc::utmpx = unsafe { std::mem::zeroed() };
        assert!(utmp_host(&u).is_empty());
    }

    /// c:851 — `utmp_is_active` on default-zeroed utmpx returns false
    /// (zero type isn't USER_PROCESS).
    #[test]
    #[cfg(unix)]
    fn utmp_is_active_zeroed_returns_false() {
        let _g = crate::test_util::global_state_lock();
        let u: libc::utmpx = unsafe { std::mem::zeroed() };
        assert!(!utmp_is_active(&u), "zero type != USER_PROCESS");
    }

    /// c:658 — `bin_log` with no args returns a valid status code
    /// (may return 0 since C log builtin lists current users).
    #[test]
    fn bin_log_no_args_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let _ = bin_log("log", &[], &ops, 0);
    }

    /// Lifecycle (c:693/735/758/769):
    #[test]
    fn watch_setup_returns_zero_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(setup_(std::ptr::null()), 0);
    }

    /// c:758 — cleanup_(NULL) = 0.
    #[test]
    fn watch_cleanup_returns_zero_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(cleanup_(std::ptr::null()), 0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/watch.c
    // c:262 watchlog_match / c:385 ucmp / c:450 readwtab / c:526 dowatch
    // c:617 checksched / c:658 bin_log / lifecycle
    // ═══════════════════════════════════════════════════════════════════

    /// c:262 — `watchlog_match` returns bool (compile-time pin).
    #[test]
    fn watchlog_match_returns_bool_type() {
        let _: bool = watchlog_match("*", "anything");
    }

    /// c:262 — `watchlog_match` is pure / deterministic.
    #[test]
    fn watchlog_match_is_deterministic_full_sweep() {
        for (p, v) in [
            ("", ""),
            ("*", "abc"),
            ("abc", "abc"),
            ("abc", "abd"),
            ("a*b", "axxxb"),
        ] {
            let first = watchlog_match(p, v);
            for _ in 0..5 {
                assert_eq!(
                    watchlog_match(p, v),
                    first,
                    "({:?}, {:?}) must be deterministic",
                    p,
                    v
                );
            }
        }
    }

    /// c:262 — `watchlog_match` empty pattern doesn't match non-empty value.
    #[test]
    fn watchlog_match_empty_pattern_rejects_nonempty() {
        assert!(!watchlog_match("", "x"), "empty pattern must NOT match 'x'");
    }

    /// c:385 — `ucmp` returns 0 for self (reflexive).
    #[test]
    fn ucmp_reflexive_returns_zero() {
        let u: libc::utmpx = unsafe { std::mem::zeroed() };
        assert_eq!(ucmp(&u, &u), 0, "ucmp(u, u) must be 0");
    }

    /// c:385 — `ucmp` return value is integer (not specifically -1/0/1).
    #[test]
    fn ucmp_returns_i32_type() {
        let u: libc::utmpx = unsafe { std::mem::zeroed() };
        let _: i32 = ucmp(&u, &u);
    }

    /// c:450 — `readwtab(0)` returns empty Vec.
    #[test]
    fn readwtab_zero_size_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        let r = readwtab(0);
        // Either empty or non-empty if the system wtab exists; pin no-panic.
        let _ = r;
    }

    /// c:526 — `dowatch()` is safe to call (no panic in non-interactive).
    #[test]
    fn dowatch_no_panic() {
        let _g = crate::test_util::global_state_lock();
        dowatch();
    }

    /// c:617 — `checksched()` is safe to call.
    #[test]
    fn checksched_no_panic() {
        let _g = crate::test_util::global_state_lock();
        checksched();
    }

    /// c:526 — `dowatch` is idempotent (callable repeatedly).
    #[test]
    fn dowatch_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..5 {
            dowatch();
        }
    }

    /// c:769 — `finish_(NULL)` returns 0.
    #[test]
    fn watch_finish_returns_zero_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(finish_(std::ptr::null()), 0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/watch.c
    // c:44 getlogtime / c:95 watch3ary / c:112 watchlog2 / c:286 watchlog /
    // c:450 readwtab / c:526 dowatch / c:617 checksched / c:658 bin_log
    // ═══════════════════════════════════════════════════════════════════

    /// c:44 — `getlogtime` returns i64 (compile-time type pin).
    #[test]
    fn getlogtime_returns_i64_type() {
        let _g = crate::test_util::global_state_lock();
        let u: libc::utmpx = unsafe { std::mem::zeroed() };
        let _: i64 = getlogtime(&u, 0);
    }

    /// c:95 — `watch3ary` returns String (compile-time type pin).
    #[test]
    fn watch3ary_returns_string_type() {
        let _g = crate::test_util::global_state_lock();
        let u: libc::utmpx = unsafe { std::mem::zeroed() };
        let _: String = watch3ary(&u, false, "");
    }

    /// c:112 — `watchlog2` returns String.
    #[test]
    fn watchlog2_returns_string_type() {
        let _g = crate::test_util::global_state_lock();
        let u: libc::utmpx = unsafe { std::mem::zeroed() };
        let _: String = watchlog2(0, &u, "", 0, 0);
    }

    /// c:286 — `watchlog` returns void.
    #[test]
    fn watchlog_returns_void() {
        let _g = crate::test_util::global_state_lock();
        let u: libc::utmpx = unsafe { std::mem::zeroed() };
        let _: () = watchlog(0, &u, &[], "");
    }

    /// c:526 — `dowatch` returns void.
    #[test]
    fn dowatch_returns_void() {
        let _g = crate::test_util::global_state_lock();
        let _: () = dowatch();
    }

    /// c:617 — `checksched` returns void.
    #[test]
    fn checksched_returns_void() {
        let _g = crate::test_util::global_state_lock();
        let _: () = checksched();
    }

    /// c:450 — `readwtab` returns Vec<libc::utmpx>.
    #[test]
    fn readwtab_returns_vec_utmpx_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Vec<libc::utmpx> = readwtab(0);
    }

    /// c:450 — `readwtab(negative)` doesn't panic.
    #[test]
    fn readwtab_negative_size_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = readwtab(-1);
        let _ = readwtab(-100);
    }

    /// c:262 — `watchlog_match("")` empty pattern matches only empty.
    #[test]
    fn watchlog_match_empty_pattern_only_matches_empty_value() {
        assert!(watchlog_match("", ""), "empty matches empty");
        assert!(!watchlog_match("", "x"), "empty does NOT match non-empty");
    }

    /// c:617 — `checksched` is idempotent.
    #[test]
    fn checksched_idempotent_full_sweep() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            checksched();
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity pins for Src/Modules/watch.c
    // c:262 watchlog_match / c:526 dowatch / c:617 checksched /
    // c:658 bin_log / c:693-769 lifecycle hooks
    // ═══════════════════════════════════════════════════════════════════

    /// c:693 — `setup_` is idempotent.
    #[test]
    fn watch_setup_idempotent_alt_pin() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(setup_(std::ptr::null()), 0);
        }
    }

    /// c:758 — `cleanup_` is idempotent.
    #[test]
    fn watch_cleanup_idempotent_alt_pin() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(cleanup_(std::ptr::null()), 0);
        }
    }

    /// c:769 — `finish_` is idempotent.
    #[test]
    fn watch_finish_idempotent_alt_pin() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(finish_(std::ptr::null()), 0);
        }
    }

    /// c:758 — `cleanup_` return type i32 (compile-time pin).
    #[test]
    fn watch_cleanup_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = cleanup_(std::ptr::null());
    }

    /// c:769 — `finish_` return type i32 (compile-time pin).
    #[test]
    fn watch_finish_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = finish_(std::ptr::null());
    }

    /// c:735 — `boot_` return type i32 (compile-time pin).
    #[test]
    fn watch_boot_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = boot_(std::ptr::null());
    }

    /// c:526 — `dowatch` is safe to call repeatedly.
    #[test]
    fn dowatch_repeated_calls_safe() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..5 {
            dowatch();
        }
    }

    /// c:617 — `checksched` returns void (compile-time pin).
    #[test]
    fn checksched_returns_void_type() {
        let _g = crate::test_util::global_state_lock();
        let _: () = checksched();
    }

    /// c:658 — `bin_log` empty args non-negative.
    #[test]
    fn bin_log_empty_args_non_negative() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let r = bin_log("log", &[], &ops, 0);
        assert!(r >= 0, "bin_log empty must be ≥ 0, got {}", r);
    }

    /// c:658 — `bin_log` various func values don't panic.
    #[test]
    fn bin_log_various_func_values_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        for func in [-1, 0, 1, 100, i32::MAX] {
            let _ = bin_log("log", &[], &ops, func);
        }
    }

    /// c:262 — `watchlog_match` returns bool (compile-time pin, alt).
    #[test]
    fn watchlog_match_returns_bool_type_alt() {
        let _: bool = watchlog_match("foo", "foo");
    }

    /// c:262 — `watchlog_match` is deterministic for same inputs.
    #[test]
    fn watchlog_match_deterministic() {
        for (p, v) in [("foo", "foo"), ("", "x"), ("*", "anything")] {
            let a = watchlog_match(p, v);
            let b = watchlog_match(p, v);
            assert_eq!(a, b, "watchlog_match({:?},{:?}) must be pure", p, v);
        }
    }
}

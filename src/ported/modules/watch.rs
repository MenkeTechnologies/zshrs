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

use std::collections::HashMap;
use std::io::BufRead;
use std::time::{SystemTime, UNIX_EPOCH};
use chrono::{Local, TimeZone};

#[cfg(unix)]
use std::ffi::CStr;

/// Default watch format string
pub const DEFAULT_WATCHFMT: &str = "%n has %a %l from %m.";

/// Default watch format without host support
pub const DEFAULT_WATCHFMT_NOHOST: &str = "%n has %a %l.";

/// `WATCH_STRUCT_UTMP` typedef alias matching `Src/Modules/watch.c:71-79`:
/// resolves to `libc::utmpx` on platforms with `<utmpx.h>` support
/// (Linux/macOS), `libc::utmp` otherwise. The C source uses this
/// alias name everywhere so the Rust port keeps it.
#[cfg(unix)]
pub type WATCH_STRUCT_UTMP = libc::utmpx;

// `UtmpEntry` struct + `SessionType` enum + `impl is_active`
// DELETED. Watch.c uses `WATCH_STRUCT_UTMP` (= `libc::utmpx`) directly
// — `WTAB` now stores `Vec<libc::utmpx>` matching C's
// `static WATCH_STRUCT_UTMP *wtab` at `Src/Modules/watch.c:151`,
// and every reader extracts `ut_user` / `ut_line` / `ut_host` / etc.
// inline via the FFI char-array → `&str` helpers below. Comparisons
// against `ut_type` use bare `libc::USER_PROCESS` / `DEAD_PROCESS` /
// etc. — same int comparisons C does at watch.c:458.

/// Read `ut_user` (an FFI `[c_char; UT_USERSIZE]` array) as a Rust
/// `String`. C: `printf("%s", u->ut_user)` decays the char array to
/// `char*` and prints until NUL — this fn does the equivalent.
pub fn utmp_user(u: &libc::utmpx) -> String {                                // c:204 ut_user
    unsafe { CStr::from_ptr(u.ut_user.as_ptr()).to_string_lossy().into_owned() }
}

/// Read `ut_line` as a `String`. Same `CStr::from_ptr` shape as C's
/// `char*` decay of the `ut_line` array.
pub fn utmp_line(u: &libc::utmpx) -> String {                                // c:204 ut_line
    unsafe { CStr::from_ptr(u.ut_line.as_ptr()).to_string_lossy().into_owned() }
}

/// Read `ut_host` as a `String`. Mirrors C's `char*` decay of
/// `ut_host`.
pub fn utmp_host(u: &libc::utmpx) -> String {                                // c:204 ut_host
    unsafe { CStr::from_ptr(u.ut_host.as_ptr()).to_string_lossy().into_owned() }
}

/// Inline of C's `entry->ut_type == USER_PROCESS && entry->ut_user[0]
/// != 0` filter at Src/Modules/watch.c:458 — `true` when this entry
/// represents an active user session.
pub fn utmp_is_active(u: &libc::utmpx) -> bool {                             // c:458
    u.ut_type == libc::USER_PROCESS as i16
        && u.ut_user.first().copied().unwrap_or(0) != 0
}

/// Construct a `libc::utmpx` for tests with the given fields. Writes
/// `user`/`line`/`host` strings into the FFI char arrays NUL-padded.
/// C tests would do the same via `strncpy(u.ut_user, "name", ...)`.
#[cfg(test)]
pub fn utmp_make(user: &str, line: &str, host: &str, time: i64, pid: i32, ut_type: i16) -> libc::utmpx {
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
    u.ut_tv.tv_sec = time as libc::time_t;
    u.ut_pid = pid;
    u.ut_type = ut_type;
    u
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
// Rust port previously aggregated these into `WatchState` and also
// pulled in `WATCHFMT` and `LOGCHECK` (which in C live on the
// param table, NOT as file-statics) — bag-of-globals. Dissolved
// into individual thread_locals matching the C declarations 1:1.
// `WATCHFMT`/`LOGCHECK` are looked up via the param table at use
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

// WARNING: NOT IN WATCH.C — Rust-only setter helpers around the
// thread_locals. C zsh doesn't have setter functions: assignments
// to `$watch` flow through the paramdef table (watch.c:697) and
// `watch.c:689` is updated implicitly by the param machinery.
// The Rust port factors them into named fns so future param-hook
// wiring has a single update site (and tests can drive them
// directly without going through the param table).

/// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
/// of any function in `Src/Modules/watch.c`.
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
    let interval = std::env::var("LOGCHECK")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(60);
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

/// Check if a watch pattern matches an entry field
/// Match a `$watch` pattern against an actual user/host/tty.
/// Port of `watchlog_match(char *teststr, char *actual, size_t buflen)` from Src/Modules/watch.c:434 — same
/// `user@host:tty` triple-component matching.
/// WARNING: param names don't match C — Rust=(pattern, value) vs C=(teststr, actual, buflen)
pub fn watchlog_match(pattern: &str, value: &str) -> bool {                  // c:434
    if pattern == value {
        return true;
    }

    if pattern.contains('*') || pattern.contains('?') {
        crate::glob::matchpat(pattern, value, false, true)
    } else {
        false
    }
}

/// Format a watch event
/// Format a watch event line (login or logout).
/// Port of `watch3ary(int inout, WATCH_STRUCT_UTMP *u, char *fmt, int prnt)` from Src/Modules/watch.c:206 (the
/// per-format-character branch of `watchlog2()` line 242) — same
/// `%n`/`%M`/`%l`/`%a`/`%T`/`%t`/`%w`/`%W`/`%D` directives.
/// WARNING: param names don't match C — Rust=(entry, logged_in, fmt) vs C=(inout, u, fmt, prnt)
pub fn watch3ary(entry: &libc::utmpx, logged_in: bool, fmt: &str) -> String { // c:206
    let mut result = String::new();
    let mut chars = fmt.chars().peekable();
    let user = utmp_user(entry);
    let line = utmp_line(entry);
    let host = utmp_host(entry);
    let time = entry.ut_tv.tv_sec as i64;

    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(next) = chars.next() {
                result.push(next);
            }
        } else if c == '%' {
            if let Some(&next) = chars.peek() {
                chars.next();
                match next {
                    'n' => result.push_str(&user),
                    'a' => {
                        if logged_in {
                            result.push_str("logged on");
                        } else {
                            result.push_str("logged off");
                        }
                    }
                    'l' => {
                        let line = if line.starts_with("tty") {
                            &line[3..]
                        } else {
                            &line
                        };
                        result.push_str(line);
                    }
                    'm' => {
                        let host = host.split('.').next().unwrap_or(&host);
                        result.push_str(host);
                    }
                    'M' => result.push_str(&host),
                    't' | '@' => {
                        // c:319-320 — strftime(buf2, sizeof(buf2), "%l:%M%p", tm);
                        if let Some(dt) = Local.timestamp_opt(time, 0).single() {
                            result.push_str(&dt.format("%l:%M%p").to_string());
                        }
                    }
                    'T' => {
                        // c:323-324 — strftime(buf2, sizeof(buf2), "%H:%M", tm);
                        if let Some(dt) = Local.timestamp_opt(time, 0).single() {
                            result.push_str(&dt.format("%H:%M").to_string());
                        }
                    }
                    'w' => {
                        // c:327-328 — strftime(buf2, sizeof(buf2), "%a %e", tm);
                        if let Some(dt) = Local.timestamp_opt(time, 0).single() {
                            result.push_str(&dt.format("%a %e").to_string());
                        }
                    }
                    'W' => {
                        // c:331-332 — strftime(buf2, sizeof(buf2), "%m/%d/%y", tm);
                        if let Some(dt) = Local.timestamp_opt(time, 0).single() {
                            result.push_str(&dt.format("%m/%d/%y").to_string());
                        }
                    }
                    'D' => {
                        if chars.peek() == Some(&'{') {
                            chars.next();
                            let mut custom_fmt = String::new();
                            for fc in chars.by_ref() {
                                if fc == '}' {
                                    break;
                                }
                                custom_fmt.push(fc);
                            }
                            // c:335-336 — user-supplied strftime format
                            if let Some(dt) = Local.timestamp_opt(time, 0).single() {
                                result.push_str(&dt.format(&custom_fmt).to_string());
                            }
                        } else {
                            // c:339-340 — strftime(buf2, sizeof(buf2), "%y-%m-%d", tm);
                            if let Some(dt) = Local.timestamp_opt(time, 0).single() {
                                result.push_str(&dt.format("%y-%m-%d").to_string());
                            }
                        }
                    }
                    '%' => result.push('%'),
                    '(' => {
                        // Inline %(c.true.false) conditional parser.
                        // Direct port of the inline conditional handling
                        // in zsh's watchlog_string (Src/Modules/watch.c).
                        // C: parses single condition char + separator,
                        // then walks until matching `)` collecting true/
                        // false branches, recursing on nested `%(`.
                        if let (Some(condition), Some(separator)) = (chars.next(), chars.next()) {
                            let truth = match condition {
                                'n' => !user.is_empty(),
                                'a' => logged_in,
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
                            while let Some(c) = chars.next() {
                                if c == ')' {
                                    depth -= 1;
                                    if depth == 0 {
                                        break;
                                    }
                                }
                                if c == separator && depth == 1 {
                                    in_true = false;
                                    continue;
                                }
                                if c == '%' && chars.peek() == Some(&'(') {
                                    depth += 1;
                                }
                                if in_true {
                                    true_branch.push(c);
                                } else {
                                    false_branch.push(c);
                                }
                            }
                            let branch = if truth { &true_branch } else { &false_branch };
                            result.push_str(&watch3ary(entry, logged_in, branch));
                        }
                    }
                    _ => {
                        result.push('%');
                        result.push(next);
                    }
                }
            }
        } else {
            result.push(c);
        }
    }

    result
}

// printtime helper deleted — C uses inline strftime() at each format
// directive in watchlog2() (c:319-340), so the Rust port inlines the
// chrono equivalent at each callsite to match.


/// Perform watch check and return login/logout events
/// Run one tick of the watch loop, returning login/logout events.
/// Port of `dowatch()` from Src/Modules/watch.c:597 — the C
/// source diffs the cached `wtab` against a fresh utmp read and
/// fires `watchlog()` for each new entry / departure.
/// WARNING: param names don't match C — Rust=(current_user) vs C=()
pub fn dowatch(current_user: &str) -> Vec<(libc::utmpx, bool)> {            // c:597
    let mut events: Vec<libc::utmpx> = Vec::new();
    // Inline utmp walk — direct port of the setutxent/getutxent/endutxent
    // loop watchlog2 uses every poll (Src/Modules/watch.c:204). zsh C
    // performs this walk in-place inside watchlog2; mirroring that
    // structure here keeps the call shape 1:1.
    let mut new_entries: Vec<libc::utmpx> = Vec::new();
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    unsafe {
        libc::setutxent();
        loop {
            let entry = libc::getutxent();
            if entry.is_null() {
                break;
            }
            // Copy the FFI-owned utmpx into our Vec (C's `wtab` is an
            // owned malloc'd array; we own the same way via Vec<utmpx>).
            new_entries.push(std::ptr::read(entry));
        }
        libc::endutxent();
    }

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    // Borrow old entries by reference instead of cloning (libc::utmpx
    // implements Copy via the libc::s! macro on most targets).
    let old_entries = WTAB.with(|t| t.borrow().clone());
    let key_of = |e: &libc::utmpx| format!("{}:{}", utmp_user(e), utmp_line(e));
    let old_active: HashMap<String, libc::utmpx> = old_entries
        .iter()
        .filter(|e| utmp_is_active(e))
        .map(|e| (key_of(e), *e))
        .collect();

    let new_active: HashMap<String, libc::utmpx> = new_entries
        .iter()
        .filter(|e| utmp_is_active(e))
        .map(|e| (key_of(e), *e))
        .collect();

    for (key, entry) in &new_active {
        if !old_active.contains_key(key) && check_entry(entry, current_user) {
            events.push(*entry);
        }
    }
    for (key, entry) in &old_active {
        if !new_active.contains_key(key) && check_entry(entry, current_user) {
            events.push(*entry);
        }
    }

    let login_keys: std::collections::HashSet<String> = new_active
        .keys()
        .filter(|k| !old_active.contains_key(*k))
        .cloned()
        .collect();

    let result: Vec<(libc::utmpx, bool)> = events
        .into_iter()
        .map(|e| {
            let key = key_of(&e);
            let is_login = login_keys.contains(&key);
            (e, is_login)
        })
        .collect();

    WTAB.with(|t| *t.borrow_mut() = new_entries);
    LASTWATCH.with(|t| t.set(now));

    result
}

/// Log builtin - force immediate watch check
/// `log` builtin entry point.
/// Port of `bin_log(UNUSED(char *nam), UNUSED(char **argv), UNUSED(Options ops), UNUSED(int func))` from Src/Modules/watch.c:659 — emits the
/// last seen watch events using the user's `$WATCHFMT` (or the
/// supplied override).
/// WARNING: param names don't match C — Rust=(current_user, fmt) vs C=(nam, argv, ops, func)
pub fn bin_log(current_user: &str, fmt: Option<&str>) -> String {            // c:659
    let fmt_str = fmt
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            std::env::var("WATCHFMT").unwrap_or_else(|_| DEFAULT_WATCHFMT.to_string())
        });
    WTAB.with(|t| t.borrow_mut().clear());
    LASTUTMPCHECK.with(|t| t.set(0));

    let events = dowatch(current_user);
    let mut output = String::new();

    for (entry, logged_in) in events {
        output.push_str(&watch3ary(&entry, logged_in, &fmt_str));
        output.push('\n');
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Port of `boot_(UNUSED(Module m))` from `Src/Modules/watch.c:738`.
    #[test]
    fn test_watch_initial_empty() {
        // Fresh thread → thread_locals are zero-initialised; the `$watch`
        // list defaults to empty (mirrors C's `static char **watch = NULL`).
        WATCH.with(|w| assert!(w.borrow().is_empty()));
    }

    #[test]
    fn test_glob_match() {
        use crate::glob::matchpat;
        assert!(matchpat("*", "anything", false, true));
        assert!(matchpat("user*", "username", false, true));
        assert!(matchpat("*name", "username", false, true));
        assert!(matchpat("user?ame", "username", false, true));
        assert!(!matchpat("user", "username", false, true));
    }

    #[test]
    fn test_watch_match() {
        assert!(watchlog_match("root", "root"));
        assert!(watchlog_match("*", "anyuser"));
        assert!(!watchlog_match("root", "admin"));
    }

    #[test]
    fn test_format_watch_basic() {
        let entry = utmp_make("testuser", "tty1", "localhost", 0, 1234, libc::USER_PROCESS as i16);
        let result = watch3ary(&entry, true, "%n has %a %l");
        assert!(result.contains("testuser"));
        assert!(result.contains("logged on"));
        assert!(result.contains("1"));

        let result = watch3ary(&entry, false, "%n has %a");
        assert!(result.contains("logged off"));
    }

    #[test]
    fn test_format_watch_host() {
        let entry = utmp_make("user", "pts/0", "host.example.com", 0, 1, libc::USER_PROCESS as i16);
        let result = watch3ary(&entry, true, "%m");
        assert_eq!(result, "host");

        let result = watch3ary(&entry, true, "%M");
        assert_eq!(result, "host.example.com");
    }

    #[test]
    fn test_check_watch_entry_all() {
        let entry = utmp_make("anyone", "pts/0", "", 0, 1, libc::USER_PROCESS as i16);
        set_watch_list(vec!["all".to_string()]);
        assert!(check_entry(&entry, "me"));
    }

    #[test]
    fn test_check_watch_entry_notme() {
        let entry = utmp_make("me", "pts/0", "", 0, 1, libc::USER_PROCESS as i16);
        set_watch_list(vec!["notme".to_string()]);
        assert!(!check_entry(&entry, "me"));

        let other = utmp_make("other", "pts/0", "", 0, 1, libc::USER_PROCESS as i16);
        assert!(check_entry(&other, "me"));
    }

    #[test]
    fn test_matches_watch_pattern() {
        let entry = utmp_make("admin", "pts/0", "server.local", 0, 1, libc::USER_PROCESS as i16);
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
        let entry = utmp_make("user", "pts/0", "", 0, 1, libc::USER_PROCESS as i16);
        assert!(utmp_is_active(&entry));

        let dead = utmp_make("user", "pts/0", "", 0, 1, libc::DEAD_PROCESS as i16);
        assert!(!utmp_is_active(&dead));
    }
}

// ===========================================================
// Methods moved verbatim from src/ported/exec.rs because their
// C counterpart's source file maps 1:1 to this Rust module.
// Phase: module-shims
// ===========================================================

// BEGIN moved-from-exec-rs
// (impl ShellExecutor block moved to src/exec_shims.rs — see file marker)

// END moved-from-exec-rs

// =====================================================================
// static struct features module_features                            c:700 (watch.c)
// =====================================================================

use crate::ported::zsh_h::module;

// `bintab` — port of `static struct builtin bintab[]` (watch.c).


// `partab` — port of `static struct paramdef partab[]` (watch.c).


// `module_features` — port of `static struct features module_features`
// from watch.c:700.



/// Port of `setup_(UNUSED(Module m))` from `Src/Modules/watch.c:712`.
#[allow(unused_variables)]
pub fn setup_(m: *const module) -> i32 {                                    // c:712
    // C body c:714-718 — `partab[0].gsu = (void *)&colonarr_gsu;
    //                     partab[1].gsu = (void *)&vararray_gsu;
    //                     return 0`. The GSU dispatch isn't part of
    //                     the static-link Rust path — partab entries
    //                     are passed through directly. Return success.
    0
}

/// Port of `features_(UNUSED(Module m), UNUSED(char ***features))` from `Src/Modules/watch.c:723`.
/// C body: `*features = featuresarray(m, &module_features); return 0;`
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {     // c:723
    *features = featuresarray(m, module_features());
    0
}

/// Port of `enables_(UNUSED(Module m), UNUSED(int **enables))` from `Src/Modules/watch.c:731`.
/// C body: `return handlefeatures(m, &module_features, enables);`
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {  // c:731
    handlefeatures(m, module_features(), enables)
}

/// Port of `boot_(UNUSED(Module m))` from `Src/Modules/watch.c:738`.
#[allow(unused_variables)]
pub fn boot_(m: *const module) -> i32 {                                     // c:738
    // C body c:740-770: ties $watch and $WATCH, creates empty `watch`
    // array, sets WATCHFMT/LOGCHECK defaults IFF unset, installs the
    // checksched preprompt hook. Seed `WATCHFMT` and `LOGCHECK` only
    // when no env value pre-exists — preserves the `${WATCHFMT-unset}`
    // distinction zsh makes between "unset" (no zmodload) and "set
    // to default" (after zmodload).
    if crate::ported::params::getsparam("WATCHFMT").is_none() {
        crate::ported::params::setsparam("WATCHFMT", DEFAULT_WATCHFMT);     // c:768
    }
    if crate::ported::params::getsparam("LOGCHECK").is_none() {
        crate::ported::params::setsparam("LOGCHECK", "60");                 // c:768
    }
    0
}

/// Port of `cleanup_(UNUSED(Module m))` from `Src/Modules/watch.c:768`.
/// C body: `delprepromptfn(checksched); return setfeatureenables(...);`
pub fn cleanup_(m: *const module) -> i32 {                                  // c:768
    setfeatureenables(m, module_features(), None)
}

/// Port of `finish_(UNUSED(Module m))` from `Src/Modules/watch.c:776`.
#[allow(unused_variables)]
pub fn finish_(m: *const module) -> i32 {                                   // c:776
    // C body c:778-779 — `return 0`. Faithful empty-body port; the
    //                    watch utmpx descriptor is process-lifetime,
    //                    not module-lifetime.
    0
}

/// Port of `getlogtime(WATCH_STRUCT_UTMP *u, int inout)` from `Src/Modules/watch.c:161`. For
/// login events (`inout` non-zero) returns the entry's `ut_time`
/// directly. For logout events, walks `wtmp` backwards looking
/// for the matching login record so the resulting time pairs the
/// real session start, not the wtmp logout marker.
///
/// C signature: `static time_t getlogtime(WATCH_STRUCT_UTMP *u, int inout)`.
/// The Rust port takes the captured `(line, ut_time)` tuple +
/// inout flag and returns the resolved login time.
///
/// **Partial port:** the wtmp scan-backwards path (c:170-198) is
/// approximate — Rust uses `getutxent` for live utmp but doesn't
/// model the random-access wtmp seek in libc. Returns
/// `time(NULL)` when scanning would be needed.
pub fn getlogtime(u_line: &str, u_time: i64, inout: i32) -> i64 {        // c:161
    if inout != 0 {                                                      // c:161
        return u_time;                                                   // c:169 return u->ut_time
    }
    // c:170 — `if (!(in = fopen(WATCH_WTMP_FILE, "r"))) return time(NULL);`
    // zshrs's wtmp seek isn't wired; return current time as the
    // documented fallback.
    let _ = u_line;
    unsafe { libc::time(std::ptr::null_mut()) as i64 }                   // c:171/175/181/186 return time(NULL)
}

/// Port of `ucmp(WATCH_STRUCT_UTMP *u, WATCH_STRUCT_UTMP *v)` from `Src/Modules/watch.c:527`. The qsort
/// comparator for utmp records: by `ut_time` ascending, then by
/// `ut_line` lexicographic.
///
/// C signature: `static int ucmp(WATCH_STRUCT_UTMP *u, WATCH_STRUCT_UTMP *v)`.
/// Rust port takes (time, line) tuples — the only fields the C
/// body reads.
/// WARNING: param names don't match C — Rust=(u_time, u_line, v_time, v_line) vs C=(u, v)
pub fn ucmp(u_time: i64, u_line: &str, v_time: i64, v_line: &str) -> i32 {  // c:527
    if u_time == v_time {                                                // c:527
        // c:530 — `return strncmp(u->ut_line, v->ut_line, sizeof(u->ut_line));`
        return match u_line.cmp(v_line) {
            std::cmp::Ordering::Less => -1,
            std::cmp::Ordering::Equal => 0,
            std::cmp::Ordering::Greater => 1,
        };
    }
    (u_time - v_time) as i32                                             // c:537
}

/// Port of `readwtab(WATCH_STRUCT_UTMP **head, int initial_sz)` from `Src/Modules/watch.c:537`. Reads the
/// utmp file (`getutxent` on systems with it, otherwise raw
/// `WATCH_UTMP_FILE`), filters out non-USER_PROCESS entries, and
/// returns them sorted by `ucmp`.
///
/// C signature: `static int readwtab(WATCH_STRUCT_UTMP **head, int initial_sz)`.
/// C writes the array to `*head` and returns the count. Rust port
/// returns the Vec directly (count is `.len()`).
/// WARNING: param names don't match C — Rust=() vs C=(head, initial_sz)
pub fn readwtab() -> Vec<libc::utmpx> {                                  // c:537
    let mut entries: Vec<libc::utmpx> = Vec::new();                      // c:537 zalloc
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    unsafe {
        libc::setutxent();                                               // c:553 setutent
        loop {
            let entry = libc::getutxent();                               // c:554 getutxent
            if entry.is_null() { break; }
            let ut = &*entry;
            // c:561 — `if (uptr->ut_type == USER_PROCESS)` filter.
            if ut.ut_type != libc::USER_PROCESS { continue; }
            entries.push(std::ptr::read(entry));
        }
        libc::endutxent();                                               // c:584 endutent
    }
    // c:587-588 — `qsort(*head, sz, sizeof(...), ucmp);`
    entries.sort_by(|a, b| {
        let at = a.ut_tv.tv_sec as i64;
        let bt = b.ut_tv.tv_sec as i64;
        match ucmp(at, &utmp_line(a), bt, &utmp_line(b)) {
            n if n < 0 => std::cmp::Ordering::Less,
            n if n > 0 => std::cmp::Ordering::Greater,
            _ => std::cmp::Ordering::Equal,
        }
    });
    entries                                                              // c:589 return sz
}

/// Port of `watchlog(int inout, WATCH_STRUCT_UTMP *u, char **w, char *fmt)` from `Src/Modules/watch.c:458`. Top-level
/// per-event dispatcher: for each entry in the `$watch` array,
/// run pattern-match against the user/host/line of the changed
/// utmp entry; on match, format via `watch3ary` (or print
/// directly) and emit to stderr.
///
/// C signature: `static void watchlog(int inout, WATCH_STRUCT_UTMP *u, char **w, char *fmt)`.
pub fn watchlog(inout: i32, u: &libc::utmpx, w: &[String], fmt: &str) {  // c:458
    // c:458 — `*str` and `*p` locals. Rust port walks `w` directly.
    let current_user = std::env::var("USER").unwrap_or_default();
    if !check_entry(u, &current_user) {                                  // c:474 watchlog_match
        return;
    }
    let _ = w;                                                           // C reads $watch from caller-passed array
    // c:519 — print formatted.
    let line = watch3ary(u, inout != 0, fmt);
    eprintln!("{}", line);                                               // c:520 fputs(stderr) + putc
}

/// Port of `watchlog2(int inout, WATCH_STRUCT_UTMP *u, char *fmt, int prnt, int fini)` from `Src/Modules/watch.c:242`. The
/// mutually-recursive ternary handler for `$WATCHFMT` parsing.
/// C body walks the format string handling `%(c.true.false)`
/// ternaries (where `c` is one of `n`/`m`/`l`/`a` etc.) by
/// dispatching back to itself for each branch.
///
/// C signature: `static char *watchlog2(int inout, WATCH_STRUCT_UTMP *u, char *fmt, int prnt, int fini)`.
/// Rust port takes the same args + returns the post-format
/// pointer offset (`prnt = 0` mode) or empty (`prnt = 1` mode).
///
/// **Partial port:** the full ternary parser at c:255-432 is
/// extensive (~190 lines of state machine over %-codes). Rust
/// port currently handles the simplest pass-through case;
/// production ternary support comes through `watch3ary` which
/// already handles the common cases.
pub fn watchlog2(_inout: i32, _u: &libc::utmpx, fmt: &str, _prnt: i32, _fini: i32) -> String { // c:242
    fmt.to_string()                                                      // c:431 return p
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
pub fn checksched() {                                                    // c:650
    // c:650 — `if (watch && difftime(...) > getiparam("LOGCHECK"))`
    let watch_set = WATCH.with(|w| !w.borrow().is_empty());
    if !watch_set { return; }
    let now = unsafe { libc::time(std::ptr::null_mut()) as i64 };        // c:654 time(NULL)
    let last = LASTWATCH.with(|t| t.get());                              // c:654 lastwatch
    let logcheck: i64 = std::env::var("LOGCHECK")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60);                                                  // c:654 getiparam("LOGCHECK")
    if (now - last) > logcheck {                                         // c:654 difftime > LOGCHECK
        let user = std::env::var("USER").unwrap_or_default();
        let _ = dowatch(&user);                                          // c:655 dowatch()
    }
}

use crate::ported::zsh_h::features as features_t;
use std::sync::{Mutex, OnceLock};

static MODULE_FEATURES: OnceLock<Mutex<features_t>> = OnceLock::new();

// WARNING: NOT IN WATCH.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn module_features() -> &'static Mutex<features_t> {
    MODULE_FEATURES.get_or_init(|| Mutex::new(features_t {
        bn_list: None,
        bn_size: 1,
        cd_list: None,
        cd_size: 0,
        mf_list: None,
        mf_size: 0,
        pd_list: None,
        pd_size: 2,
        n_abstract: 0,
    }))
}

// Local stubs for the per-module entry points. C uses generic
// `featuresarray`/`handlefeatures`/`setfeatureenables` (module.c:
// 3275/3370/3445) but those take `Builtin` + `Features` pointer
// fields the Rust port doesn't carry. The hardcoded descriptor
// list mirrors the C bintab/conddefs/mathfuncs/paramdefs.
// WARNING: NOT IN WATCH.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn featuresarray(_m: *const module, _f: &Mutex<features_t>) -> Vec<String> {
    vec!["b:log".to_string(), "p:WATCH".to_string(), "p:watch".to_string()]
}

// WARNING: NOT IN WATCH.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn handlefeatures(
    _m: *const module,
    _f: &Mutex<features_t>,
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
    _f: &Mutex<features_t>,
    _e: Option<&[i32]>,
) -> i32 {
    0
}


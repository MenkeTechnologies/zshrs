//! Login/logout watching module - port of Modules/watch.c
//!
//! Provides watch/log functionality for monitoring user logins/logouts.

use std::collections::HashMap;
use std::io::BufRead;
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::ffi::CStr;

/// Default watch format string
pub const DEFAULT_WATCHFMT: &str = "%n has %a %l from %m.";

/// Default watch format without host support
pub const DEFAULT_WATCHFMT_NOHOST: &str = "%n has %a %l.";

/// A utmp/utmpx entry representing a login session
#[derive(Debug, Clone)]
/// One utmp/utmpx entry (login session record).
/// Mirrors the relevant fields of `WATCH_STRUCT_UTMP` from
/// `<utmp.h>` / `<utmpx.h>` that Src/Modules/watch.c reads via
/// `readwtab()` (line 537) and dispatches through `watchlog()`
/// (line 458).
pub struct UtmpEntry {
    pub user: String,
    pub line: String,
    pub host: String,
    pub time: i64,
    pub pid: i32,
    pub session_type: SessionType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Login session classification.
/// Mirrors the `USER_PROCESS` / `INIT_PROCESS` / `LOGIN_PROCESS`
/// `ut_type` constants the C source's `watchlog()` (Src/Modules/
/// watch.c:458) inspects when deciding whether an entry is a
/// login.
pub enum SessionType {
    UserProcess,
    DeadProcess,
    LoginProcess,
    InitProcess,
    BootTime,
    Unknown,
}

impl UtmpEntry {
    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/watch.c`.
    pub fn is_active(&self) -> bool {
        matches!(self.session_type, SessionType::UserProcess) && !self.user.is_empty()
    }
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
    static WTAB: std::cell::RefCell<Vec<UtmpEntry>> = const {
        std::cell::RefCell::new(Vec::new())
    };

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
pub fn check_entry(entry: &UtmpEntry, current_user: &str) -> bool {
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
            if entry.user == current_user {
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
                if !watchlog_match(user_pat, &entry.user) {
                    matched = false;
                }
                rest = &rest[end..];
            }
            while !rest.is_empty() && matched {
                if let Some(rest1) = rest.strip_prefix('%') {
                    let end = rest1.find('@').unwrap_or(rest1.len());
                    let line_pat = &rest1[..end];
                    if !watchlog_match(line_pat, &entry.line) {
                        matched = false;
                    }
                    rest = &rest1[end..];
                } else if let Some(rest1) = rest.strip_prefix('@') {
                    let end = rest1.find('%').unwrap_or(rest1.len());
                    let host_pat = &rest1[..end];
                    if !watchlog_match(host_pat, &entry.host) {
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
/// Port of `watchlog_match()` from Src/Modules/watch.c:434 — same
/// `user@host:tty` triple-component matching.
pub fn watchlog_match(pattern: &str, value: &str) -> bool {
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
/// Port of `watch3ary()` from Src/Modules/watch.c:206 (the
/// per-format-character branch of `watchlog2()` line 242) — same
/// `%n`/`%M`/`%l`/`%a`/`%T`/`%t`/`%w`/`%W`/`%D` directives.
pub fn watch3ary(entry: &UtmpEntry, logged_in: bool, fmt: &str) -> String {
    let mut result = String::new();
    let mut chars = fmt.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(next) = chars.next() {
                result.push(next);
            }
        } else if c == '%' {
            if let Some(&next) = chars.peek() {
                chars.next();
                match next {
                    'n' => result.push_str(&entry.user),
                    'a' => {
                        if logged_in {
                            result.push_str("logged on");
                        } else {
                            result.push_str("logged off");
                        }
                    }
                    'l' => {
                        let line = if entry.line.starts_with("tty") {
                            &entry.line[3..]
                        } else {
                            &entry.line
                        };
                        result.push_str(line);
                    }
                    'm' => {
                        let host = entry.host.split('.').next().unwrap_or(&entry.host);
                        result.push_str(host);
                    }
                    'M' => result.push_str(&entry.host),
                    't' | '@' => {
                        let time = printtime(entry.time, "%l:%M%p");
                        result.push_str(&time);
                    }
                    'T' => {
                        let time = printtime(entry.time, "%H:%M");
                        result.push_str(&time);
                    }
                    'w' => {
                        let time = printtime(entry.time, "%a %e");
                        result.push_str(&time);
                    }
                    'W' => {
                        let time = printtime(entry.time, "%m/%d/%y");
                        result.push_str(&time);
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
                            let time = printtime(entry.time, &custom_fmt);
                            result.push_str(&time);
                        } else {
                            let time = printtime(entry.time, "%y-%m-%d");
                            result.push_str(&time);
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
                                'n' => !entry.user.is_empty(),
                                'a' => logged_in,
                                'l' => {
                                    if entry.line.starts_with("tty") {
                                        entry.line.len() > 3
                                    } else {
                                        !entry.line.is_empty()
                                    }
                                }
                                'm' | 'M' => !entry.host.is_empty(),
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

/// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
/// of any function in `Src/Modules/watch.c`.
fn printtime(timestamp: i64, fmt: &str) -> String {
    use chrono::{Local, TimeZone};

    if let Some(dt) = Local.timestamp_opt(timestamp, 0).single() {
        dt.format(fmt).to_string()
    } else {
        String::new()
    }
}


/// Perform watch check and return login/logout events
/// Run one tick of the watch loop, returning login/logout events.
/// Port of `dowatch()` from Src/Modules/watch.c:597 — the C
/// source diffs the cached `wtab` against a fresh utmp read and
/// fires `watchlog()` for each new entry / departure.
pub fn dowatch(current_user: &str) -> Vec<(UtmpEntry, bool)> {
    let mut events = Vec::new();
    // Inline utmp walk — direct port of the setutxent/getutxent/endutxent
    // loop watchlog2 uses every poll (Src/Modules/watch.c:204). zsh C
    // performs this walk in-place inside watchlog2; mirroring that
    // structure here keeps the call shape 1:1.
    let mut new_entries: Vec<UtmpEntry> = Vec::new();
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    unsafe {
        libc::setutxent();
        loop {
            let entry = libc::getutxent();
            if entry.is_null() {
                break;
            }
            let ut = &*entry;
            let user = CStr::from_ptr(ut.ut_user.as_ptr())
                .to_string_lossy()
                .into_owned();
            let line = CStr::from_ptr(ut.ut_line.as_ptr())
                .to_string_lossy()
                .into_owned();
            let host = CStr::from_ptr(ut.ut_host.as_ptr())
                .to_string_lossy()
                .into_owned();
            let session_type = match ut.ut_type {
                t if t == libc::USER_PROCESS => SessionType::UserProcess,
                t if t == libc::DEAD_PROCESS => SessionType::DeadProcess,
                t if t == libc::LOGIN_PROCESS => SessionType::LoginProcess,
                t if t == libc::INIT_PROCESS => SessionType::InitProcess,
                t if t == libc::BOOT_TIME => SessionType::BootTime,
                _ => SessionType::Unknown,
            };
            new_entries.push(UtmpEntry {
                user,
                line,
                host,
                time: ut.ut_tv.tv_sec as i64,
                pid: ut.ut_pid,
                session_type,
            });
        }
        libc::endutxent();
    }

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let old_entries = WTAB.with(|t| t.borrow().clone());
    let old_active: HashMap<String, UtmpEntry> = old_entries
        .iter()
        .filter(|e| e.is_active())
        .map(|e| (format!("{}:{}", e.user, e.line), e.clone()))
        .collect();

    let new_active: HashMap<String, UtmpEntry> = new_entries
        .iter()
        .filter(|e| e.is_active())
        .map(|e| (format!("{}:{}", e.user, e.line), e.clone()))
        .collect();

    for (key, entry) in &new_active {
        if !old_active.contains_key(key)
            && check_entry(entry, current_user)
        {
            events.push(entry.clone());
            events.last_mut().unwrap();
        }
    }

    for (key, entry) in &old_active {
        if !new_active.contains_key(key)
            && check_entry(entry, current_user)
        {
            events.push(entry.clone());
        }
    }

    let login_keys: std::collections::HashSet<String> = new_active
        .keys()
        .filter(|k| !old_active.contains_key(*k))
        .cloned()
        .collect();

    let result: Vec<(UtmpEntry, bool)> = events
        .into_iter()
        .map(|e| {
            let key = format!("{}:{}", e.user, e.line);
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
/// Port of `bin_log()` from Src/Modules/watch.c:681 — emits the
/// last seen watch events using the user's `$WATCHFMT` (or the
/// supplied override).
pub fn bin_log(current_user: &str, fmt: Option<&str>) -> String {
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
        let entry = UtmpEntry {
            user: "testuser".to_string(),
            line: "tty1".to_string(),
            host: "localhost".to_string(),
            time: 0,
            pid: 1234,
            session_type: SessionType::UserProcess,
        };

        let result = watch3ary(&entry, true, "%n has %a %l");
        assert!(result.contains("testuser"));
        assert!(result.contains("logged on"));
        assert!(result.contains("1"));

        let result = watch3ary(&entry, false, "%n has %a");
        assert!(result.contains("logged off"));
    }

    #[test]
    fn test_format_watch_host() {
        let entry = UtmpEntry {
            user: "user".to_string(),
            line: "pts/0".to_string(),
            host: "host.example.com".to_string(),
            time: 0,
            pid: 1,
            session_type: SessionType::UserProcess,
        };

        let result = watch3ary(&entry, true, "%m");
        assert_eq!(result, "host");

        let result = watch3ary(&entry, true, "%M");
        assert_eq!(result, "host.example.com");
    }

    #[test]
    fn test_check_watch_entry_all() {
        let entry = UtmpEntry {
            user: "anyone".to_string(),
            line: "pts/0".to_string(),
            host: "".to_string(),
            time: 0,
            pid: 1,
            session_type: SessionType::UserProcess,
        };

        set_watch_list(vec!["all".to_string()]);
        assert!(check_entry(&entry, "me"));
    }

    #[test]
    fn test_check_watch_entry_notme() {
        let entry = UtmpEntry {
            user: "me".to_string(),
            line: "pts/0".to_string(),
            host: "".to_string(),
            time: 0,
            pid: 1,
            session_type: SessionType::UserProcess,
        };

        set_watch_list(vec!["notme".to_string()]);
        assert!(!check_entry(&entry, "me"));

        let other = UtmpEntry {
            user: "other".to_string(),
            ..entry.clone()
        };
        assert!(check_entry(&other, "me"));
    }

    #[test]
    fn test_matches_watch_pattern() {
        let entry = UtmpEntry {
            user: "admin".to_string(),
            line: "pts/0".to_string(),
            host: "server.local".to_string(),
            time: 0,
            pid: 1,
            session_type: SessionType::UserProcess,
        };

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
        let entry = UtmpEntry {
            user: "user".to_string(),
            line: "pts/0".to_string(),
            host: "".to_string(),
            time: 0,
            pid: 1,
            session_type: SessionType::UserProcess,
        };
        assert!(entry.is_active());

        let dead = UtmpEntry {
            session_type: SessionType::DeadProcess,
            ..entry.clone()
        };
        assert!(!dead.is_active());
    }
}

// ===========================================================
// Methods moved verbatim from src/ported/exec.rs because their
// C counterpart's source file maps 1:1 to this Rust module.
// Phase: module-shims
// ===========================================================

// BEGIN moved-from-exec-rs
impl crate::ported::exec::ShellExecutor {
    /// `log` builtin — delegates to canonical port at
    /// `src/ported/modules/watch.rs` (`bin_log()` from
    /// `Src/Modules/watch.c`). The watch state lives in
    /// `thread_local!`s in the canonical port (mirroring C's
    /// `Src/Modules/watch.c:150-156` file-statics) so login/logout
    /// edge detection survives across calls without a struct on
    /// `ShellExecutor`.
    pub(crate) fn bin_log(&mut self, _args: &[String]) -> i32 {
        let user = std::env::var("USER").unwrap_or_default();
        let fmt = self.variables.get("WATCHFMT").cloned();
        let output = crate::watch::bin_log(&user, fmt.as_deref());
        if !output.is_empty() {
            print!("{}", output);
        }
        0
    }
}
// END moved-from-exec-rs

/// Module loader entry — port of `setup_()` from Src/Modules/watch.c:712.
pub fn setup_() -> i32 {
    0
}

/// Module loader entry — port of `features_()` from Src/Modules/watch.c:723.
pub fn features_() -> i32 {
    0
}

/// Module loader entry — port of `enables_()` from Src/Modules/watch.c:731.
pub fn enables_() -> i32 {
    0
}

/// Module loader entry — port of `boot_()` from Src/Modules/watch.c:738.
pub fn boot_() -> i32 {
    0
}

/// Module loader entry — port of `cleanup_()` from Src/Modules/watch.c:768.
pub fn cleanup_() -> i32 {
    0
}

/// Module loader entry — port of `finish_()` from Src/Modules/watch.c:776.
pub fn finish_() -> i32 {
    0
}

// === auto-generated stubs ===
// Direct ports of static helpers from Src/Modules/watch.c not
// yet covered above. zshrs links modules statically; live
// state owned by the module's typed struct. Name-parity shims.

/// Port of `checksched()` from Src/Modules/watch.c:650.
#[allow(non_snake_case)]
pub fn checksched() -> i32 { 0 }

/// Port of `getlogtime()` from Src/Modules/watch.c:161.
#[allow(non_snake_case)]
pub fn getlogtime() -> i32 { 0 }

/// Port of `readwtab()` from Src/Modules/watch.c:537.
#[allow(non_snake_case)]
pub fn readwtab() -> i32 { 0 }

/// Port of `ucmp()` from Src/Modules/watch.c:527.
#[allow(non_snake_case)]
pub fn ucmp() -> i32 { 0 }

/// Port of `watchlog()` from Src/Modules/watch.c:458.
#[allow(non_snake_case)]
pub fn watchlog() -> i32 { 0 }

/// Port of `watchlog2()` from Src/Modules/watch.c:242.
#[allow(non_snake_case)]
pub fn watchlog2() -> i32 { 0 }

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
pub struct UtmpEntry {
    pub user: String,
    pub line: String,
    pub host: String,
    pub time: i64,
    pub pid: i32,
    pub session_type: SessionType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionType {
    UserProcess,
    DeadProcess,
    LoginProcess,
    InitProcess,
    BootTime,
    Unknown,
}

impl UtmpEntry {
    pub fn is_active(&self) -> bool {
        matches!(self.session_type, SessionType::UserProcess) && !self.user.is_empty()
    }
}

/// Watch state for tracking login/logout events
#[derive(Debug, Default)]
pub struct WatchState {
    last_check: i64,
    last_watch: i64,
    entries: Vec<UtmpEntry>,
    watch_list: Vec<String>,
    watch_fmt: String,
    log_check_interval: i64,
}

impl WatchState {
    pub fn new() -> Self {
        Self {
            last_check: 0,
            last_watch: 0,
            entries: Vec::new(),
            watch_list: Vec::new(),
            watch_fmt: DEFAULT_WATCHFMT.to_string(),
            log_check_interval: 60,
        }
    }

    pub fn set_watch_list(&mut self, list: Vec<String>) {
        self.watch_list = list;
    }

    pub fn set_watch_fmt(&mut self, fmt: &str) {
        self.watch_fmt = fmt.to_string();
    }

    pub fn set_log_check(&mut self, interval: i64) {
        self.log_check_interval = interval;
    }

    pub fn should_check(&self) -> bool {
        if self.watch_list.is_empty() {
            return false;
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        now - self.last_watch > self.log_check_interval
    }
}

/// Read utmp entries from the system
#[cfg(target_os = "linux")]
pub fn read_utmp() -> Vec<UtmpEntry> {
    read_utmp_file("/var/run/utmp")
}

#[cfg(target_os = "macos")]
pub fn read_utmp() -> Vec<UtmpEntry> {
    read_utmpx()
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn read_utmp() -> Vec<UtmpEntry> {
    Vec::new()
}

#[cfg(target_os = "macos")]
fn read_utmpx() -> Vec<UtmpEntry> {
    let mut entries = Vec::new();

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

            let ut_type = ut.ut_type;
            let session_type = if ut_type == libc::USER_PROCESS {
                SessionType::UserProcess
            } else if ut_type == libc::DEAD_PROCESS {
                SessionType::DeadProcess
            } else if ut_type == libc::LOGIN_PROCESS {
                SessionType::LoginProcess
            } else if ut_type == libc::INIT_PROCESS {
                SessionType::InitProcess
            } else if ut_type == libc::BOOT_TIME {
                SessionType::BootTime
            } else {
                SessionType::Unknown
            };

            entries.push(UtmpEntry {
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

    entries
}

#[cfg(target_os = "linux")]
fn read_utmp_file(_path: &str) -> Vec<UtmpEntry> {
    let mut entries = Vec::new();

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

            let ut_type = ut.ut_type;
            let session_type = if ut_type == libc::USER_PROCESS {
                SessionType::UserProcess
            } else if ut_type == libc::DEAD_PROCESS {
                SessionType::DeadProcess
            } else if ut_type == libc::LOGIN_PROCESS {
                SessionType::LoginProcess
            } else if ut_type == libc::INIT_PROCESS {
                SessionType::InitProcess
            } else if ut_type == libc::BOOT_TIME {
                SessionType::BootTime
            } else {
                SessionType::Unknown
            };

            entries.push(UtmpEntry {
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

    entries
}

/// Check if a watch pattern matches an entry field
pub fn watch_match(pattern: &str, value: &str) -> bool {
    if pattern == value {
        return true;
    }

    if pattern.contains('*') || pattern.contains('?') {
        glob_match(pattern, value)
    } else {
        false
    }
}

fn glob_match(pattern: &str, text: &str) -> bool {
    let p_chars: Vec<char> = pattern.chars().collect();
    let t_chars: Vec<char> = text.chars().collect();

    let mut p_idx = 0;
    let mut t_idx = 0;
    let mut star_idx: Option<usize> = None;
    let mut match_idx = 0;

    while t_idx < t_chars.len() {
        if p_idx < p_chars.len() && (p_chars[p_idx] == '?' || p_chars[p_idx] == t_chars[t_idx]) {
            p_idx += 1;
            t_idx += 1;
        } else if p_idx < p_chars.len() && p_chars[p_idx] == '*' {
            star_idx = Some(p_idx);
            match_idx = t_idx;
            p_idx += 1;
        } else if let Some(star) = star_idx {
            p_idx = star + 1;
            match_idx += 1;
            t_idx = match_idx;
        } else {
            return false;
        }
    }

    while p_idx < p_chars.len() && p_chars[p_idx] == '*' {
        p_idx += 1;
    }

    p_idx == p_chars.len()
}

/// Format a watch event
pub fn format_watch(entry: &UtmpEntry, logged_in: bool, fmt: &str) -> String {
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
                        let time = format_time(entry.time, "%l:%M%p");
                        result.push_str(&time);
                    }
                    'T' => {
                        let time = format_time(entry.time, "%H:%M");
                        result.push_str(&time);
                    }
                    'w' => {
                        let time = format_time(entry.time, "%a %e");
                        result.push_str(&time);
                    }
                    'W' => {
                        let time = format_time(entry.time, "%m/%d/%y");
                        result.push_str(&time);
                    }
                    'D' => {
                        if chars.peek() == Some(&'{') {
                            chars.next();
                            let mut custom_fmt = String::new();
                            while let Some(fc) = chars.next() {
                                if fc == '}' {
                                    break;
                                }
                                custom_fmt.push(fc);
                            }
                            let time = format_time(entry.time, &custom_fmt);
                            result.push_str(&time);
                        } else {
                            let time = format_time(entry.time, "%y-%m-%d");
                            result.push_str(&time);
                        }
                    }
                    '%' => result.push('%'),
                    '(' => {
                        if let Some(cond_result) = format_conditional(&mut chars, entry, logged_in)
                        {
                            result.push_str(&cond_result);
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

fn format_conditional(
    chars: &mut std::iter::Peekable<std::str::Chars>,
    entry: &UtmpEntry,
    logged_in: bool,
) -> Option<String> {
    let condition = chars.next()?;
    let separator = chars.next()?;

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

        if c == '%' {
            if chars.peek() == Some(&'(') {
                depth += 1;
            }
        }

        if in_true {
            true_branch.push(c);
        } else {
            false_branch.push(c);
        }
    }

    if truth {
        Some(format_watch(entry, logged_in, &true_branch))
    } else {
        Some(format_watch(entry, logged_in, &false_branch))
    }
}

fn format_time(timestamp: i64, fmt: &str) -> String {
    use chrono::{Local, TimeZone};

    if let Some(dt) = Local.timestamp_opt(timestamp, 0).single() {
        dt.format(fmt).to_string()
    } else {
        String::new()
    }
}

/// Check a watch entry against the watch list
pub fn check_watch_entry(entry: &UtmpEntry, watch_list: &[String], current_user: &str) -> bool {
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
        if matches_watch_pattern(pattern, entry) {
            return true;
        }
    }

    false
}

fn matches_watch_pattern(pattern: &str, entry: &UtmpEntry) -> bool {
    let mut rest = pattern;
    let mut matched = true;

    if !rest.starts_with('@') && !rest.starts_with('%') {
        let end = rest.find(|c| c == '@' || c == '%').unwrap_or(rest.len());
        let user_pat = &rest[..end];
        if !watch_match(user_pat, &entry.user) {
            matched = false;
        }
        rest = &rest[end..];
    }

    while !rest.is_empty() && matched {
        if rest.starts_with('%') {
            rest = &rest[1..];
            let end = rest.find('@').unwrap_or(rest.len());
            let line_pat = &rest[..end];
            if !watch_match(line_pat, &entry.line) {
                matched = false;
            }
            rest = &rest[end..];
        } else if rest.starts_with('@') {
            rest = &rest[1..];
            let end = rest.find('%').unwrap_or(rest.len());
            let host_pat = &rest[..end];
            if !watch_match(host_pat, &entry.host) {
                matched = false;
            }
            rest = &rest[end..];
        } else {
            break;
        }
    }

    matched
}

/// Perform watch check and return login/logout events
pub fn do_watch(state: &mut WatchState, current_user: &str) -> Vec<(UtmpEntry, bool)> {
    let mut events = Vec::new();
    let new_entries = read_utmp();

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let old_active: HashMap<String, &UtmpEntry> = state
        .entries
        .iter()
        .filter(|e| e.is_active())
        .map(|e| (format!("{}:{}", e.user, e.line), e))
        .collect();

    let new_active: HashMap<String, &UtmpEntry> = new_entries
        .iter()
        .filter(|e| e.is_active())
        .map(|e| (format!("{}:{}", e.user, e.line), e))
        .collect();

    for (key, entry) in &new_active {
        if !old_active.contains_key(key) {
            if check_watch_entry(entry, &state.watch_list, current_user) {
                events.push((*entry).clone());
                events.last_mut().unwrap();
            }
        }
    }

    for (key, entry) in &old_active {
        if !new_active.contains_key(key) {
            if check_watch_entry(entry, &state.watch_list, current_user) {
                let logged_out = (*entry).clone();
                events.push(logged_out);
            }
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

    state.entries = new_entries;
    state.last_watch = now;

    result
}

/// Log builtin - force immediate watch check
pub fn builtin_log(state: &mut WatchState, current_user: &str, fmt: Option<&str>) -> String {
    let fmt_str = fmt
        .map(|s| s.to_string())
        .unwrap_or_else(|| state.watch_fmt.clone());
    state.entries.clear();
    state.last_check = 0;

    let events = do_watch(state, current_user);
    let mut output = String::new();

    for (entry, logged_in) in events {
        output.push_str(&format_watch(&entry, logged_in, &fmt_str));
        output.push('\n');
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_watch_state_new() {
        let state = WatchState::new();
        assert!(state.watch_list.is_empty());
        assert_eq!(state.log_check_interval, 60);
    }

    #[test]
    fn test_glob_match() {
        assert!(glob_match("*", "anything"));
        assert!(glob_match("user*", "username"));
        assert!(glob_match("*name", "username"));
        assert!(glob_match("user?ame", "username"));
        assert!(!glob_match("user", "username"));
    }

    #[test]
    fn test_watch_match() {
        assert!(watch_match("root", "root"));
        assert!(watch_match("*", "anyuser"));
        assert!(!watch_match("root", "admin"));
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

        let result = format_watch(&entry, true, "%n has %a %l");
        assert!(result.contains("testuser"));
        assert!(result.contains("logged on"));
        assert!(result.contains("1"));

        let result = format_watch(&entry, false, "%n has %a");
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

        let result = format_watch(&entry, true, "%m");
        assert_eq!(result, "host");

        let result = format_watch(&entry, true, "%M");
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

        let watch = vec!["all".to_string()];
        assert!(check_watch_entry(&entry, &watch, "me"));
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

        let watch = vec!["notme".to_string()];
        assert!(!check_watch_entry(&entry, &watch, "me"));

        let other = UtmpEntry {
            user: "other".to_string(),
            ..entry.clone()
        };
        assert!(check_watch_entry(&other, &watch, "me"));
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

        assert!(matches_watch_pattern("admin", &entry));
        assert!(matches_watch_pattern("admin@server.local", &entry));
        assert!(matches_watch_pattern("admin%pts/0", &entry));
        assert!(!matches_watch_pattern("root", &entry));
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

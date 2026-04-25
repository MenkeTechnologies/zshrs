//! Compatibility and utility routines for zshrs
//!
//! Direct port from zsh/Src/compat.c
//!
//! Provides:
//! - High-resolution time functions
//! - Directory navigation utilities
//! - Path handling for long pathnames
//! - 64-bit integer formatting

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Time with nanosecond precision
#[derive(Debug, Clone, Copy, Default)]
pub struct TimeSpec {
    pub tv_sec: i64,
    pub tv_nsec: i64,
}

impl TimeSpec {
    pub fn new(sec: i64, nsec: i64) -> Self {
        TimeSpec {
            tv_sec: sec,
            tv_nsec: nsec,
        }
    }

    pub fn now() -> Self {
        match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(d) => TimeSpec {
                tv_sec: d.as_secs() as i64,
                tv_nsec: d.subsec_nanos() as i64,
            },
            Err(_) => TimeSpec::default(),
        }
    }

    pub fn as_duration(&self) -> Duration {
        Duration::new(self.tv_sec as u64, self.tv_nsec as u32)
    }

    pub fn as_secs_f64(&self) -> f64 {
        self.tv_sec as f64 + (self.tv_nsec as f64 / 1_000_000_000.0)
    }
}

impl std::ops::Sub for TimeSpec {
    type Output = TimeSpec;

    fn sub(self, other: TimeSpec) -> TimeSpec {
        let mut sec = self.tv_sec - other.tv_sec;
        let mut nsec = self.tv_nsec - other.tv_nsec;
        if nsec < 0 {
            sec -= 1;
            nsec += 1_000_000_000;
        }
        TimeSpec {
            tv_sec: sec,
            tv_nsec: nsec,
        }
    }
}

/// Get current time with nanosecond precision (real time)
pub fn zgettime() -> TimeSpec {
    TimeSpec::now()
}

/// Monotonic time tracking
static MONOTONIC_START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

/// Get monotonic time (for timing, doesn't go backwards)
pub fn zgettime_monotonic() -> TimeSpec {
    let start = MONOTONIC_START.get_or_init(Instant::now);
    let elapsed = start.elapsed();
    TimeSpec {
        tv_sec: elapsed.as_secs() as i64,
        tv_nsec: elapsed.subsec_nanos() as i64,
    }
}

/// Compute difference between two times
pub fn difftime(t2: i64, t1: i64) -> f64 {
    (t2 - t1) as f64
}

/// Get system's maximum open file descriptors
pub fn zopenmax() -> i64 {
    #[cfg(unix)]
    {
        // Try to get from system
        unsafe {
            let max = libc::sysconf(libc::_SC_OPEN_MAX);
            if max > 0 {
                // Cap at a reasonable value
                return max.min(1024 * 1024);
            }
        }
    }

    // Fallback
    1024
}

/// Get the current working directory
pub fn zgetcwd() -> Option<String> {
    env::current_dir()
        .ok()
        .and_then(|p| p.to_str().map(|s| s.to_string()))
}

/// Get directory with additional metadata
pub struct DirSav {
    pub dirname: Option<String>,
    #[cfg(unix)]
    pub ino: u64,
    #[cfg(unix)]
    pub dev: u64,
}

impl Default for DirSav {
    fn default() -> Self {
        DirSav {
            dirname: None,
            #[cfg(unix)]
            ino: 0,
            #[cfg(unix)]
            dev: 0,
        }
    }
}

/// Get current directory with optional metadata storage
pub fn zgetdir(d: Option<&mut DirSav>) -> Option<String> {
    let cwd = env::current_dir().ok()?;
    let cwd_str = cwd.to_str()?.to_string();

    #[cfg(unix)]
    if let Some(dirsav) = d {
        use std::os::unix::fs::MetadataExt;
        if let Ok(meta) = fs::metadata(&cwd) {
            dirsav.ino = meta.ino();
            dirsav.dev = meta.dev();
        }
        dirsav.dirname = Some(cwd_str.clone());
    }

    #[cfg(not(unix))]
    if let Some(dirsav) = d {
        dirsav.dirname = Some(cwd_str.clone());
    }

    Some(cwd_str)
}

/// Change directory with support for long pathnames
/// Returns 0 on success, -1 on normal failure, -2 if current directory is lost
pub fn zchdir(dir: &str) -> i32 {
    if dir.is_empty() {
        return 0;
    }

    // Try direct chdir first
    if env::set_current_dir(dir).is_ok() {
        return 0;
    }

    // For long paths, try changing incrementally
    let path = Path::new(dir);
    if !path.is_absolute() {
        return -1;
    }

    // Save current directory
    let saved_dir = env::current_dir().ok();

    // Try to change directory component by component
    let mut current = PathBuf::from("/");
    for component in path.components().skip(1) {
        current.push(component);
        if env::set_current_dir(&current).is_err() {
            // Try to restore
            if let Some(ref saved) = saved_dir {
                if env::set_current_dir(saved).is_err() {
                    return -2; // Lost current directory
                }
            }
            return -1;
        }
    }

    0
}

/// Format a 64-bit integer for output
pub fn output64(val: i64) -> String {
    val.to_string()
}

/// Format an unsigned 64-bit integer for output
pub fn output64u(val: u64) -> String {
    val.to_string()
}

/// Convert number to string with given base
pub fn convbase(val: i64, base: u32) -> String {
    if base == 0 || base == 10 {
        return val.to_string();
    }

    let is_negative = val < 0;
    let mut n = val.unsigned_abs();
    let mut result = String::new();

    if n == 0 {
        return "0".to_string();
    }

    let digits = b"0123456789abcdefghijklmnopqrstuvwxyz";
    while n > 0 {
        let digit = (n % base as u64) as usize;
        result.push(digits[digit] as char);
        n /= base as u64;
    }

    if is_negative {
        result.push('-');
    }

    result.chars().rev().collect()
}

/// Convert unsigned number to string with given base
pub fn convbaseu(val: u64, base: u32) -> String {
    if base == 0 || base == 10 {
        return val.to_string();
    }

    let mut n = val;
    let mut result = String::new();

    if n == 0 {
        return "0".to_string();
    }

    let digits = b"0123456789abcdefghijklmnopqrstuvwxyz";
    while n > 0 {
        let digit = (n % base as u64) as usize;
        result.push(digits[digit] as char);
        n /= base as u64;
    }

    result.chars().rev().collect()
}

/// Get hostname
pub fn gethostname() -> Option<String> {
    hostname::get().ok().and_then(|h| h.into_string().ok())
}

/// Check if a character is printable (ASCII safe version)
pub fn isprint_safe(c: char) -> bool {
    let b = c as u32;
    b >= 0x20 && b <= 0x7e
}

/// Unicode-aware character width
pub fn wcwidth(c: char) -> i32 {
    unicode_width::UnicodeWidthChar::width(c)
        .map(|w| w as i32)
        .unwrap_or(if c.is_control() { -1 } else { 1 })
}

/// Check if a wide character is printable
pub fn iswprint(c: char) -> bool {
    !c.is_control() && wcwidth(c) >= 0
}

/// String width accounting for unicode
pub fn strwidth(s: &str) -> usize {
    unicode_width::UnicodeWidthStr::width(s)
}

/// Metafy a string (encode special characters)
pub fn metafy(s: &str) -> String {
    let mut result = String::with_capacity(s.len() * 2);
    for c in s.chars() {
        let b = c as u32;
        if b < 32 || (b >= 0x83 && b <= 0x9b) {
            result.push('\u{83}'); // Meta marker
            result.push(char::from_u32(b ^ 32).unwrap_or(c));
        } else {
            result.push(c);
        }
    }
    result
}

/// Unmetafy a string (decode special characters)
pub fn unmetafy(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '\u{83}' {
            if let Some(&next) = chars.peek() {
                chars.next();
                let b = next as u32;
                result.push(char::from_u32(b ^ 32).unwrap_or(next));
            } else {
                result.push(c);
            }
        } else {
            result.push(c);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_timespec() {
        let t1 = TimeSpec::new(10, 500_000_000);
        let t2 = TimeSpec::new(12, 200_000_000);
        let diff = t2 - t1;
        assert_eq!(diff.tv_sec, 1);
        assert_eq!(diff.tv_nsec, 700_000_000);
    }

    #[test]
    fn test_timespec_negative() {
        let t1 = TimeSpec::new(10, 800_000_000);
        let t2 = TimeSpec::new(12, 200_000_000);
        let diff = t2 - t1;
        assert_eq!(diff.tv_sec, 1);
        assert_eq!(diff.tv_nsec, 400_000_000);
    }

    #[test]
    fn test_zgettime() {
        let t = zgettime();
        assert!(t.tv_sec > 0);
    }

    #[test]
    fn test_zgettime_monotonic() {
        let t1 = zgettime_monotonic();
        std::thread::sleep(std::time::Duration::from_millis(10));
        let t2 = zgettime_monotonic();
        let diff = t2 - t1;
        assert!(diff.tv_sec > 0 || diff.tv_nsec > 0);
    }

    #[test]
    fn test_convbase() {
        assert_eq!(convbase(255, 16), "ff");
        assert_eq!(convbase(8, 2), "1000");
        assert_eq!(convbase(-10, 10), "-10");
        assert_eq!(convbase(0, 16), "0");
    }

    #[test]
    fn test_convbaseu() {
        assert_eq!(convbaseu(255, 16), "ff");
        assert_eq!(convbaseu(8, 8), "10");
    }

    #[test]
    fn test_zgetcwd() {
        let cwd = zgetcwd();
        assert!(cwd.is_some());
        assert!(!cwd.unwrap().is_empty());
    }

    #[test]
    fn test_zopenmax() {
        let max = zopenmax();
        assert!(max > 0);
    }

    #[test]
    fn test_gethostname() {
        let host = gethostname();
        assert!(host.is_some());
    }

    #[test]
    fn test_isprint_safe() {
        assert!(isprint_safe('a'));
        assert!(isprint_safe('Z'));
        assert!(isprint_safe(' '));
        assert!(!isprint_safe('\x00'));
        assert!(!isprint_safe('\x1f'));
    }

    #[test]
    fn test_wcwidth() {
        assert_eq!(wcwidth('a'), 1);
        assert_eq!(wcwidth('中'), 2);
        assert!(wcwidth('\x00') <= 0);
    }

    #[test]
    fn test_strwidth() {
        assert_eq!(strwidth("hello"), 5);
        assert_eq!(strwidth("中文"), 4);
    }

    #[test]
    fn test_metafy_unmetafy() {
        let original = "hello\x00world";
        let meta = metafy(original);
        let unmeta = unmetafy(&meta);
        assert_eq!(unmeta, original);
    }
}

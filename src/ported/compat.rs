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

/// Get current real-time with nanosecond precision.
/// Port of `zgettime()` from Src/compat.c:101 — the C source
/// uses `clock_gettime(CLOCK_REALTIME)` (with fallbacks for older
/// systems); Rust's `SystemTime::now()` does the same.
pub fn zgettime() -> TimeSpec {
    TimeSpec::now()
}

/// Monotonic time tracking
static MONOTONIC_START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

/// Get monotonic time for elapsed-duration measurement.
/// Port of `zgettime_monotonic_if_available()` from
/// Src/compat.c:133 — uses `CLOCK_MONOTONIC` so the value never
/// goes backwards across NTP corrections. Rust's `Instant`
/// guarantees monotonicity.
pub fn zgettime_monotonic_if_available() -> TimeSpec {
    let start = MONOTONIC_START.get_or_init(Instant::now);
    let elapsed = start.elapsed();
    TimeSpec {
        tv_sec: elapsed.as_secs() as i64,
        tv_nsec: elapsed.subsec_nanos() as i64,
    }
}

/// Compute the difference between two times in seconds.
/// Port of `difftime()` from Src/compat.c:175 — wraps
/// libc's `difftime(3)` for systems lacking the prototype.
pub fn difftime(t2: i64, t1: i64) -> f64 {
    (t2 - t1) as f64
}

/// Get system's maximum open file descriptors. Direct port of
/// src/zsh/Src/compat.c:300-328 zopenmax.
///
/// Algorithm:
///   1. sysconf(_SC_OPEN_MAX). If <1, fallback to OPEN_MAX (256).
///   2. If sysconf returns absurdly high (e.g. "unlimited" via
///      ulimit), cap at ZSH_INITIAL_OPEN_MAX (1024) and walk fds
///      from OPEN_MAX upward to find the highest open one. Report
///      max(OPEN_MAX, highest_open_fd) — anything above that
///      causes inefficiency elsewhere in zsh per compat.c:307-313.
///
/// The previous Rust impl capped at 1MB which is way too high
/// for closem() loops; matched zsh's actual cap.
pub fn zopenmax() -> i64 {
    // ZSH_INITIAL_OPEN_MAX from zsh.h — 1024.
    const ZSH_INITIAL_OPEN_MAX: i64 = 1024;
    // OPEN_MAX fallback from sysconf failure — POSIX guarantees 20
    // for _POSIX_OPEN_MAX; zsh uses 256 historically.
    const OPEN_MAX: i64 = 256;

    #[cfg(unix)]
    {
        unsafe {
            let mut openmax = libc::sysconf(libc::_SC_OPEN_MAX);
            if openmax < 1 {
                openmax = OPEN_MAX;
            } else if openmax > OPEN_MAX {
                // compat.c:314-324 — walk fds to find highest open.
                if openmax > ZSH_INITIAL_OPEN_MAX {
                    openmax = ZSH_INITIAL_OPEN_MAX;
                }
                let mut j = OPEN_MAX;
                let mut i = j;
                while i < openmax {
                    let r = libc::fcntl(i as i32, libc::F_GETFL, 0);
                    if r < 0 {
                        // errno across platforms: macOS uses
                        // __error(), Linux/BSD use __errno_location().
                        // std::io::Error::last_os_error() abstracts
                        // both via the same OS error code.
                        let e = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
                        if e == libc::EBADF || e == libc::EINTR {
                            if e != libc::EINTR {
                                i += 1;
                            }
                            continue;
                        }
                    }
                    j = i;
                    i += 1;
                }
                openmax = j;
            }
            openmax
        }
    }

    #[cfg(not(unix))]
    {
        OPEN_MAX
    }
}

/// Get the current working directory.
/// Port of `zgetcwd()` from Src/compat.c:559 — wraps
/// `getcwd(3)` with a long-path-tolerant fallback. Rust's
/// `current_dir()` covers the same range.
pub fn zgetcwd() -> Option<String> {
    env::current_dir()
        .ok()
        .and_then(|p| p.to_str().map(|s| s.to_string()))
}

/// Saved-directory state (name + inode + device).
/// Port of `struct dirsav` from Src/zsh.h — populated by
/// `zgetdir()` (Src/compat.c:355) so `cd OLDDIR` can detect
/// when the named directory has been moved/replaced.
#[derive(Default)]
pub struct DirSav {
    pub dirname: Option<String>,
    #[cfg(unix)]
    pub ino: u64,
    #[cfg(unix)]
    pub dev: u64,
}

/// Get the current directory with optional metadata capture.
/// Port of `zgetdir()` from Src/compat.c:355 — when called with
/// a `dirsav` slot, fills inode/device the C source uses to
/// detect rename-replace cases.
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

/// Change directory with long-pathname support.
/// Port of `zchdir()` from Src/compat.c:579 — falls back to
/// component-by-component descent when a single `chdir(2)` call
/// fails (typically `ENAMETOOLONG`). Returns `0` on success,
/// `-1` on normal failure, `-2` if the cwd was lost mid-walk.
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

/// Format a 64-bit signed integer for output.
/// Port of `output64()` from Src/compat.c:638 — needed in C
/// because `%lld` printf support varied; Rust's `to_string()`
/// handles every target.
pub fn output64(val: i64) -> String {
    val.to_string()
}

/// Format a 64-bit unsigned integer for output.
/// Unsigned counterpart of `output64()` (Src/compat.c:638).
pub fn output64u(val: u64) -> String {
    val.to_string()
}

/// Convert a signed integer to a string in an arbitrary base.
/// Port of the inner radix-conversion loop `convbase()` in
/// Src/utils.c (math.c calls it from `bin_print -P`). Bases up
/// to 36 use the standard `0-9a-z` digit set.
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

/// Convert an unsigned integer to a string in an arbitrary base.
/// Unsigned counterpart of `convbase`.
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

/// Get the local hostname.
/// Port of the `gethostname()` shim from Src/compat.c:64 — the C
/// source provides this as a fallback for systems that lack the
/// POSIX prototype. Rust just uses the `hostname` crate.
pub fn gethostname() -> Option<String> {
    hostname::get().ok().and_then(|h| h.into_string().ok())
}

/// Check whether an ASCII byte is printable.
/// Port of `isprint_ascii()` from Src/compat.c:785 — locale-
/// independent printable check the C source uses when locale
/// data isn't safe to read (signal handlers, early init).
pub fn isprint_ascii(c: char) -> bool {
    let b = c as u32;
    (0x20..=0x7e).contains(&b)
}

/// Get the column width of a Unicode character.
/// Port of `u9_wcwidth()` from Src/compat.c:760 — the C source
/// ships its own Unicode 9 u9_wcwidth fallback because system
/// `u9_wcwidth(3)` data ages with libc. Rust uses the
/// `unicode-width` crate which tracks the latest UCD.
pub fn u9_wcwidth(c: char) -> i32 {
    unicode_width::UnicodeWidthChar::width(c)
        .map(|w| w as i32)
        .unwrap_or(if c.is_control() { -1 } else { 1 })
}

/// Check whether a wide character is printable.
/// Port of `u9_iswprint()` from Src/compat.c:770.
pub fn u9_iswprint(c: char) -> bool {
    !c.is_control() && u9_wcwidth(c) >= 0
}

/// Compute the column width of a UTF-8 string.
/// zshrs convenience over `u9_wcwidth` — closest C analog is the
/// per-character width sum the prompt-width logic does in
/// Src/prompt.c via repeated `WCWIDTH()` calls.
pub fn strwidth(s: &str) -> usize {
    unicode_width::UnicodeWidthStr::width(s)
}

/// Metafy a string (encode bytes the parser would otherwise eat).
/// Port of `metafy()` from Src/utils.c — escapes the high-bit
/// range zsh's parser reserves (`Meta`+`xor 32`). Most Rust code
/// paths don't need metafication (UTF-8 is native) but FFI to C
/// modules sometimes does.
pub fn metafy(s: &str) -> String {
    let mut result = String::with_capacity(s.len() * 2);
    for c in s.chars() {
        let b = c as u32;
        if b < 32 || (0x83..=0x9b).contains(&b) {
            result.push('\u{83}'); // Meta marker
            result.push(char::from_u32(b ^ 32).unwrap_or(c));
        } else {
            result.push(c);
        }
    }
    result
}

/// Unmetafy a string.
/// Port of `unmetafy()` from Src/utils.c — inverse of `metafy`.
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
        let t1 = zgettime_monotonic_if_available();
        std::thread::sleep(std::time::Duration::from_millis(10));
        let t2 = zgettime_monotonic_if_available();
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
        assert!(isprint_ascii('a'));
        assert!(isprint_ascii('Z'));
        assert!(isprint_ascii(' '));
        assert!(!isprint_ascii('\x00'));
        assert!(!isprint_ascii('\x1f'));
    }

    #[test]
    fn test_wcwidth() {
        assert_eq!(u9_wcwidth('a'), 1);
        assert_eq!(u9_wcwidth('中'), 2);
        assert!(u9_wcwidth('\x00') <= 0);
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

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

// `TimeSpec` Rust-only struct deleted — C uses `struct timespec`
// directly (Src/compat.c:101 `zgettime(struct timespec *ts)`).
// The canonical type port lives at
// `crate::ported::zsh_system_h::timespec` (Src/zsh_system.h:245).

use crate::ported::zsh_system_h::timespec;

/// Provide clock time with nanoseconds.
///
/// Port of `zgettime(ts)` from Src/compat.c:101.
/// C signature: `int zgettime(struct timespec *ts)`.
/// Returns 0 on success, -1 if `clock_gettime(CLOCK_REALTIME)`
/// failed and `gettimeofday` fallback succeeded, -2 if both
/// failed.
pub fn zgettime(ts: &mut timespec) -> i32 {                                  // c:101
    let mut ret: i32 = -1;                                                   // c:103
    unsafe {
        let mut dts: timespec = std::mem::zeroed();
        if libc::clock_gettime(libc::CLOCK_REALTIME, &mut dts) < 0 {         // c:107
            // c:108 — `zwarn("unable to retrieve time: %e", errno)`.
            crate::ported::utils::zwarn(&format!(
                "unable to retrieve time: {}",
                std::io::Error::last_os_error()
            ));
            ret -= 1;                                                        // c:109
        } else {                                                             // c:110
            ret += 1;                                                        // c:111
            ts.tv_sec = dts.tv_sec;                                          // c:112
            ts.tv_nsec = dts.tv_nsec;                                        // c:113
        }
        if ret != 0 {                                                        // c:117
            let mut dtv: libc::timeval = std::mem::zeroed();                 // c:118
            libc::gettimeofday(&mut dtv, std::ptr::null_mut());              // c:120
            ret += 1;                                                        // c:121
            ts.tv_sec = dtv.tv_sec;                                          // c:122
            ts.tv_nsec = (dtv.tv_usec as libc::c_long) * 1000;               // c:123
        }
    }
    ret                                                                      // c:126
}

/// Likewise with CLOCK_MONOTONIC if available.
///
/// Port of `zgettime_monotonic_if_available()` from
/// Src/compat.c:133. C signature: `int
/// zgettime_monotonic_if_available(struct timespec *ts)`.
/// Falls back to `zgettime` (CLOCK_REALTIME) when CLOCK_MONOTONIC
/// fails.
///
/// On at least some versions of macOS it appears that CLOCK_MONOTONIC // c:141
/// is not actually monotonic -- there are reports that it can go     // c:142
/// backwards. CLOCK_MONOTONIC_RAW does not have this problem. On top // c:143
/// of that, it is faster to read and it has nanosecond precision.    // c:144
pub fn zgettime_monotonic_if_available(ts: &mut timespec) -> i32 {           // c:133
    let mut ret: i32 = -1;                                                   // c:135
    unsafe {
        let mut dts: timespec = std::mem::zeroed();                          // c:138
        // c:147 — Apple prefers CLOCK_MONOTONIC_RAW; other systems
        // use CLOCK_MONOTONIC.
        #[cfg(target_os = "macos")]
        let clk = libc::CLOCK_MONOTONIC_RAW;
        #[cfg(not(target_os = "macos"))]
        let clk = libc::CLOCK_MONOTONIC;
        if libc::clock_gettime(clk, &mut dts) < 0 {                          // c:148/150
            // c:152 — `zwarn("unable to retrieve CLOCK_MONOTONIC time: %e", errno)`.
            crate::ported::utils::zwarn(&format!(
                "unable to retrieve CLOCK_MONOTONIC time: {}",
                std::io::Error::last_os_error()
            ));
            ret -= 1;                                                        // c:153
        } else {
            ret += 1;                                                        // c:155
            ts.tv_sec = dts.tv_sec;                                          // c:156
            ts.tv_nsec = dts.tv_nsec;                                        // c:157
        }
    }
    if ret != 0 {                                                            // c:161
        ret = zgettime(ts);                                                  // c:162
    }
    ret                                                                      // c:164
}

// compute the difference between two calendar times                        // c:168
/// Compute the difference between two times in seconds.
/// Port of `difftime(t2, t1)` from Src/compat.c:175 — wraps
/// libc's `difftime(3)` for systems lacking the prototype.
pub fn difftime(t2: i64, t1: i64) -> f64 {                                   // c:175
    (t2 - t1) as f64
}

// Neither of these should happen, but resort to OPEN_MAX rather            // c:291
// than return 0 or -1 just in case.                                        // c:292
//                                                                          // c:293
// We'll limit the open maximum to ZSH_INITIAL_OPEN_MAX to                  // c:294
// avoid probing ridiculous numbers of file descriptors.                    // c:295
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
pub fn zopenmax() -> i64 {                                                   // c:300
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
pub fn zgetcwd() -> Option<String> {                                        // c:559
    env::current_dir()
        .ok()
        .and_then(|p| p.to_str().map(|s| s.to_string()))
}

/// Saved-directory state (name + inode + device).
/// Port of `struct dirsav` from Src/zsh.h — populated by
// `struct dirsav` lives in `crate::ported::zsh_h::dirsav` per Rule C
// (its C definition is `Src/zsh.h:1159`, not compat.c). The previous
// Rust port had a partial Rust-only duplicate `pub struct DirSav`
// missing `dirfd` + `level`. Deleted; callers go through the
// canonical lowercase `dirsav` directly.

/// Get the current directory with optional metadata capture.
/// Port of `zgetdir(d)` from Src/compat.c:355 — when called with
/// a `dirsav` slot, fills inode/device the C source uses to
/// detect rename-replace cases.
///
/// C signature: `char *zgetdir(struct dirsav *d)`. Rust port keeps
/// the out-arg shape but adds `Option<&mut>` so callers can pass
/// `None` (matching the `NULL` legal value the C body checks for).
pub fn zgetdir(d: Option<&mut crate::ported::zsh_h::dirsav>) -> Option<String> { // c:355
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
/// Port of `zchdir(dir)` from Src/compat.c:579 — falls back to
/// component-by-component descent when a single `chdir(2)` call
/// fails (typically `ENAMETOOLONG`). Returns `0` on success,
/// `-1` on normal failure, `-2` if the cwd was lost mid-walk.
pub fn zchdir(dir: &str) -> i32 {                                           // c:579
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
/// Port of `output64(val)` from Src/compat.c:638 — needed in C
/// because `%lld` printf support varied; Rust's `to_string()`
/// handles every target.
pub fn output64(val: i64) -> String {                                        // c:638
    val.to_string()
}

// `convbase` moved out — canonical port lives at
// `crate::ported::utils::convbase` (Src/utils.c is the C source).
// `gethostname` moved out — canonical port lives at
// `crate::ported::utils::gethostname` (compat.c's body is
// `#ifndef HAVE_GETHOSTNAME` fallback shim; the active code path
// goes through libc directly via utils.rs).

/// Check whether an ASCII byte is printable.
/// Port of `isprint_ascii(c)` from Src/compat.c:785 — locale-
/// independent printable check the C source uses when locale
/// data isn't safe to read (signal handlers, early init).
pub fn isprint_ascii(c: char) -> bool {                                      // c:785
    let b = c as u32;
    (0x20..=0x7e).contains(&b)
}

/// Get the column width of a Unicode character.
/// Port of `u9_wcwidth(ucs)` from Src/compat.c:760 — the C source
/// ships its own Unicode 9 u9_wcwidth fallback because system
/// `u9_wcwidth(3)` data ages with libc. Rust uses the
/// `unicode-width` crate which tracks the latest UCD.
pub fn u9_wcwidth(c: char) -> i32 {                                          // c:760
    unicode_width::UnicodeWidthChar::width(c)
        .map(|w| w as i32)
        .unwrap_or(if c.is_control() { -1 } else { 1 })
}

/// Check whether a wide character is printable.
/// Port of `u9_iswprint(ucs)` from Src/compat.c:770.
pub fn u9_iswprint(c: char) -> bool {                                        // c:770
    !c.is_control() && u9_wcwidth(c) >= 0
}

// `metafy` / `unmetafy` moved out — canonical ports live at
// `crate::ported::utils::metafy` and `::unmetafy` (Src/utils.c
// is the C source, not compat.c). Callers wanting an owned
// `String` route through `utils::unmeta(&str) -> String` (the
// real port of `unmeta(file_name)` at Src/utils.c:4994).
//
// `strstr` / `gettimeofday` / `strtoul` removed — compat.c
// provides them as `#ifndef HAVE_*` fallback shims. On all
// targets zshrs supports (modern Linux/macOS/BSD with libc),
// the libc versions are linked directly; the compat.c shims
// are dead code on those targets.
//
// `zpathmax` removed — the C source has the entire body wrapped
// in `#if 0` (disabled since 2003 per compat.c:204 comment:
// "pathconf(_PC_PATH_MAX) is not currently useful to zsh").
// Rust port had it active for a dead C function.

/// Render an errno value as a human-readable string.
/// Port of `strerror(errnum)` from Src/compat.c:194 (`#ifndef
/// HAVE_STRERROR` fallback shim). C body: `return
/// sys_errlist[errnum]`. On HAVE_STRERROR systems the libc one
/// is used directly; Rust's `std::io::Error::from_raw_os_error`
/// routes through libc strerror internally.
pub fn strerror(errnum: i32) -> String {                                     // c:194
    std::io::Error::from_raw_os_error(errnum).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zgettime() {
        let mut ts: timespec = unsafe { std::mem::zeroed() };
        let r = zgettime(&mut ts);
        assert!(r >= 0);
        assert!(ts.tv_sec > 0);
    }

    #[test]
    fn test_zgettime_monotonic() {
        let mut t1: timespec = unsafe { std::mem::zeroed() };
        let mut t2: timespec = unsafe { std::mem::zeroed() };
        let r1 = zgettime_monotonic_if_available(&mut t1);
        std::thread::sleep(std::time::Duration::from_millis(10));
        let r2 = zgettime_monotonic_if_available(&mut t2);
        assert!(r1 >= 0 && r2 >= 0);
        // Elapsed must be strictly positive in ns.
        let elapsed_ns = (t2.tv_sec - t1.tv_sec) * 1_000_000_000
                       + (t2.tv_nsec - t1.tv_nsec) as i64;
        assert!(elapsed_ns > 0);
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

}

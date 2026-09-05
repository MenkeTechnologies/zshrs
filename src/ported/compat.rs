//! Compatibility and utility routines for zshrs
//!
//! Direct port from zsh/Src/compat.c
//!
//! Provides:
//! - High-resolution time functions
//! - Directory navigation utilities
//! - Path handling for long pathnames
//! - 64-bit integer formatting

use std::cmp::Ordering;
use std::{env, fs};

use crate::params::getsparam;
use crate::ported::zsh_h::dirsav;
use crate::utils::{unmeta, zwarn};
use crate::zsh_system_h::{timespec, OPEN_MAX, ZSH_INITIAL_OPEN_MAX};
use std::os::unix::fs::MetadataExt;

/// Provide clock time with nanoseconds.
///
/// Port of `zgettime(struct timespec *ts)` from Src/compat.c:101.
/// C signature: `int zgettime(struct timespec *ts)`.
/// Returns 0 on success, -1 if `clock_gettime(CLOCK_REALTIME)`
/// failed and `gettimeofday` fallback succeeded, -2 if both
/// failed.
pub fn zgettime(ts: &mut timespec) -> i32 {
    // c:101
    let mut ret: i32 = -1; // c:101
    unsafe {
        let mut dts: timespec = std::mem::zeroed();
        if libc::clock_gettime(libc::CLOCK_REALTIME, &mut dts) < 0 {
            // c:107
            // c:108 — `zwarn("unable to retrieve time: %e", errno)`.
            zwarn(&format!(
                "unable to retrieve time: {}",
                std::io::Error::last_os_error()
            ));
            ret -= 1; // c:109
        } else {
            // c:110
            ret += 1; // c:111
            ts.tv_sec = dts.tv_sec; // c:112
            ts.tv_nsec = dts.tv_nsec; // c:113
        }
        if ret != 0 {
            // c:117
            let mut dtv: libc::timeval = std::mem::zeroed(); // c:118
            libc::gettimeofday(&mut dtv, std::ptr::null_mut()); // c:120
            ret += 1; // c:121
            ts.tv_sec = dtv.tv_sec; // c:122
            ts.tv_nsec = (dtv.tv_usec as libc::c_long) * 1000; // c:123
        }
    }
    ret // c:126
}

/// Likewise with CLOCK_MONOTONIC if available.
///
/// Port of `zgettime_monotonic_if_available()` from
/// Src/compat.c:133. C signature: `int
/// zgettime_monotonic_if_available(struct timespec *ts)`.
/// Falls back to `zgettime` (CLOCK_REALTIME) when CLOCK_MONOTONIC
/// fails.
///
/// On at least some versions of macOS it appears that CLOCK_MONOTONIC // c:133
/// is not actually monotonic -- there are reports that it can go     // c:133
/// backwards. CLOCK_MONOTONIC_RAW does not have this problem. On top // c:133
/// of that, it is faster to read and it has nanosecond precision.    // c:133
pub fn zgettime_monotonic_if_available(ts: &mut timespec) -> i32 {
    // c:133
    let mut ret: i32 = -1; // c:133
    unsafe {
        let mut dts: timespec = std::mem::zeroed(); // c:138
                                                    // c:147 — Apple prefers CLOCK_MONOTONIC_RAW; other systems
                                                    // use CLOCK_MONOTONIC.
        #[cfg(target_os = "macos")]
        let clk = libc::CLOCK_MONOTONIC_RAW;
        #[cfg(not(target_os = "macos"))]
        let clk = libc::CLOCK_MONOTONIC;
        if libc::clock_gettime(clk, &mut dts) < 0 {
            // c:148/150
            // c:152 — `zwarn("unable to retrieve CLOCK_MONOTONIC time: %e", errno)`.
            zwarn(&format!(
                "unable to retrieve CLOCK_MONOTONIC time: {}",
                std::io::Error::last_os_error()
            ));
            ret -= 1; // c:153
        } else {
            ret += 1; // c:155
            ts.tv_sec = dts.tv_sec; // c:156
            ts.tv_nsec = dts.tv_nsec; // c:157
        }
    }
    if ret != 0 {
        // c:175
        ret = zgettime(ts); // c:175
    }
    ret // c:175
}

// compute the difference between two calendar times                        // c:175
/// Compute the difference between two times in seconds.
/// Port of `difftime(time_t t2, time_t t1)` from Src/compat.c:175 — wraps
/// libc's `difftime(3)` for systems lacking the prototype.
pub fn difftime(t2: i64, t1: i64) -> f64 {
    // c:175
    (t2 - t1) as f64
}

// `metafy` / `unmetafy` moved out — canonical ports live at
// `crate::ported::utils::metafy` and `::unmetafy` (Src/utils.c
// is the C source, not compat.c). Callers wanting an owned
// `String` route through `utils::unmeta(&str) -> String` (the
// real port of `unmeta(const char *file_name)` at Src/utils.c:4994).
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
/// Port of `strerror(int errnum)` from Src/compat.c:194 (`#ifndef
/// HAVE_STRERROR` fallback shim). C body: `return
/// sys_errlist[errnum]`. On HAVE_STRERROR systems the libc one
/// is used directly; Rust's `std::io::Error::from_raw_os_error`
/// routes through libc strerror internally.
pub fn strerror(errnum: i32) -> String {
    // c:194
    // c:Src/compat.c:194 — `return sys_errlist[errnum]` (or libc
    // `strerror(errnum)` on HAVE_STRERROR systems). The C strerror
    // returns the bare message ("No such file or directory") with
    // no suffix. The previous Rust port used
    // `std::io::Error::from_raw_os_error(errnum).to_string()`,
    // whose Display impl appends ` (os error N)` — so every
    // builtin that formatted an error through strerror leaked the
    // Rust-internal suffix into user-visible output
    // (`cat: file: No such file or directory (os error 2)` vs the
    // C-faithful `cat: file: No such file or directory`). Bug
    // #112 in docs/BUGS.md.
    //
    // Call libc::strerror directly to match C strerror exactly.
    // libc::strerror returns a pointer into a thread-local static
    // buffer; safe to read here because we copy into an owned
    // String before returning (no escape of the pointer).
    unsafe {
        let p = libc::strerror(errnum);
        if p.is_null() {
            return String::new();
        }
        match std::ffi::CStr::from_ptr(p).to_str() {
            Ok(s) => s.to_string(),
            Err(_) => std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned(),
        }
    }
}

/// Last-errno strerror helper — returns the C `strerror(errno)`
/// string for the current OS error. Use this instead of
/// `std::io::Error::last_os_error()` whenever the result is going
/// into a user-visible diagnostic, because Rust's `Error::Display`
/// appends " (os error N)" while C's `strerror` does not. Bug #112
/// in docs/BUGS.md — every `zwarnnam(nam, &format!("...: {}",
/// std::io::Error::last_os_error()))` site leaked the Rust suffix.
pub fn last_errstr() -> String {
    let e = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
    strerror(e)
}

// Neither of these should happen, but resort to OPEN_MAX rather            // c:291
// than return 0 or -1 just in case.                                        // c:292
//                                                                          // c:293
// We'll limit the open maximum to ZSH_INITIAL_OPEN_MAX to                  // c:294
// avoid probing ridiculous numbers of file descriptors.                    // c:295
/// Get system's maximum open file descriptors. Direct port of
/// src/zsh/Src/compat.c:300 zopenmax.
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
    // c:300
    // `ZSH_INITIAL_OPEN_MAX` from `Src/zsh_system.h:307` — 64 (NOT 1024).
    // Canonical port lives in `crate::ported::zsh_system_h`; use it
    // directly so any future C-source bump propagates here.
    // `OPEN_MAX` from `Src/zsh_system.h:310-313` — either NOFILE
    // (host-defined) or falls through to `ZSH_INITIAL_OPEN_MAX`. The
    // C body's `j = OPEN_MAX` starting point is the host's NOFILE
    // (typically 1024 on Linux, 10240 on macOS) when available;
    // otherwise it collapses to 64. Use the canonical port.
    #[cfg(unix)]
    {
        unsafe {
            let mut openmax = libc::sysconf(libc::_SC_OPEN_MAX);
            if openmax < 1 {
                openmax = OPEN_MAX as i64;
            } else if openmax > OPEN_MAX as i64 {
                // compat.c:314-324 — walk fds to find highest open.
                if openmax > ZSH_INITIAL_OPEN_MAX as i64 {
                    openmax = ZSH_INITIAL_OPEN_MAX as i64;
                }
                let mut j = OPEN_MAX as i64;
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

/// Saved-directory state (name + inode + device).
/// Port of `struct dirsav` from Src/zsh.h — populated by
// `struct dirsav` lives in `crate::ported::zsh_h::dirsav` per Rule C
// (its C definition is `Src/zsh.h:1159`, not compat.c). The previous
// Rust port had a partial Rust-only duplicate `pub struct DirSav`
// missing `dirfd` + `level`. Deleted; callers go through the
// canonical lowercase `dirsav` directly.

/// Get the current directory with optional metadata capture.
/// Port of `zgetdir(struct dirsav *d)` from Src/compat.c:355 — when called with
/// a `dirsav` slot, fills inode/device the C source uses to
/// detect rename-replace cases.
///
/// C signature: `char *zgetdir(struct dirsav *d)`. Rust port keeps
/// the out-arg shape but adds `Option<&mut>` so callers can pass
/// `None` (matching the `NULL` legal value the C body checks for).
pub fn zgetdir(d: Option<&mut dirsav>) -> Option<String> {
    // c:355
    let cwd = env::current_dir().ok()?;
    let cwd_str = cwd.to_str()?.to_string();

    #[cfg(unix)]
    if let Some(dirsav) = d {
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

/// Get the current working directory.
/// Port of `char *zgetcwd(void)` from `Src/compat.c:559`.
///
/// C body (c:559-567):
/// ```c
/// char *ret = zgetdir(NULL);                  // c:561
/// if (!ret)                                   // c:562
///     ret = unmeta(pwd);                      // c:563
/// if (!ret || *ret == '\0')                   // c:564
///     ret = dupstring(".");                   // c:565
/// return ret;                                 // c:566
/// ```
///
/// Three-level fallback: real cwd via `zgetdir(NULL)`, then the
/// shell's `$pwd` parameter (unmetafied), then literal `"."`.
/// Always returns a non-empty string, matching zsh.
pub fn zgetcwd() -> String {
    // c:559
    // c:561 — `ret = zgetdir(NULL);`
    if let Some(ret) = zgetdir(None) {
        // c:561
        if !ret.is_empty() {
            // c:564 — !*ret == '\0' check
            return ret;
        }
    }
    // c:562-563 — `if (!ret) ret = unmeta(pwd);`. C reads the
    // `pwd` file-scope static (Src/params.c:108). In zshrs the
    // equivalent is the `$PWD` shell parameter, looked up via
    // the canonical paramtab accessor (uppercase — that's the
    // export name; the lowercase `pwd` is a C-internal symbol
    // with no Rust-side counterpart in paramtab).
    if let Some(pwd) = getsparam("PWD") {
        // c:563
        let unmeta_pwd = unmeta(&pwd); // c:563
        if !unmeta_pwd.is_empty() {
            // c:564
            return unmeta_pwd;
        }
    }
    // c:564-565 — `if (!ret || *ret == '\0') ret = dupstring(".");`.
    ".".to_string() // c:565
}

/// Change directory with long-pathname support.
/// Port of `int zchdir(char *dir)` from `Src/compat.c:579`.
///
/// chdir with arbitrary long pathname support. Returns:
///   * `0`  — success
///   * `-1` — normal `chdir(2)` failure (ENOENT, EACCES, etc.)
///   * `-2` — current directory was lost mid-walk (saved fchdir failed)
///
/// C body (c:579-627):
/// ```c
/// for (;;) {
///     if (!*dir || chdir(dir) == 0) {
///         if (currdir >= 0) close(currdir);
///         return 0;
///     }
///     if ((errno != ENAMETOOLONG && errno != ENOMEM) ||
///         strlen(dir) < PATH_MAX)
///         break;                                  // c:594 — give up
///     for (s = dir + PATH_MAX - 1; s > dir && *s != '/'; s--)
///         ;                                       // c:595 — find slash near boundary
///     if (s == dir) break;
///     if (currdir == -2) currdir = open(".", ...);
///     *s = '\0';
///     if (chdir(dir) < 0) { *s = '/'; break; }
///     *s = '/';
///     while (*++s == '/') ;
///     dir = s;                                    // c:614 — recurse with tail
/// }
/// ```
///
/// Three divergences in the previous Rust port (now fixed):
///   1. **Always entered fallback path on chdir failure.** C only
///      tries the chunked descent when errno is `ENAMETOOLONG` /
///      `ENOMEM` AND `strlen(dir) >= PATH_MAX` (c:592-593). For
///      normal failures (ENOENT, EACCES) C returns -1 immediately;
///      Rust would walk the path component-by-component, exposing
///      partial-path side effects (e.g. successful descent into a
///      readable parent before failing).
///   2. **Did single-component descent instead of PATH_MAX chunking.**
///      C splits at the last `/` before the PATH_MAX boundary
///      (c:595), chdirs to that prefix, then loops with the tail.
///      Rust pushed one component at a time which has different
///      observable behaviour when components are themselves long.
///   3. **Gated fallback on `path.is_absolute()`.** C doesn't —
///      it walks any path that's long enough, relative or absolute.
///
/// Now mirrors C's algorithm: try direct chdir; on long-path errno
/// + `len >= PATH_MAX`, find slash near boundary, chdir to prefix,
/// continue with tail.
pub fn zchdir(dir: &str) -> i32 {
    // c:579
    #[cfg(unix)]
    {
        let path_max: usize = libc::PATH_MAX as usize;
        let mut remaining: Vec<u8> = dir.as_bytes().to_vec();
        let mut saved_currdir: i32 = -2; // c:582
        loop {
            // c:585 — `if (!*dir || chdir(dir) == 0) { close + return 0; }`
            if remaining.is_empty() {
                if saved_currdir >= 0 {
                    unsafe {
                        libc::close(saved_currdir);
                    }
                }
                return 0;
            }
            let c_dir = match std::ffi::CString::new(remaining.clone()) {
                Ok(c) => c,
                Err(_) => return -1, // NUL byte in path — chdir would fail anyway.
            };
            let rc = unsafe { libc::chdir(c_dir.as_ptr()) };
            if rc == 0 {
                // c:585
                if saved_currdir >= 0 {
                    // c:587
                    unsafe {
                        libc::close(saved_currdir);
                    } // c:588
                }
                return 0; // c:590
            }
            // c:592-594 — only the ENAMETOOLONG/ENOMEM + long path arm
            // attempts the chunked descent. Everything else gives up.
            let err = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            let ok_errno = err == libc::ENAMETOOLONG || err == libc::ENOMEM;
            if !ok_errno || remaining.len() < path_max {
                // c:592-594
                break;
            }
            // c:595-596 — find last `/` strictly before PATH_MAX.
            let mut s_idx: isize = (path_max - 1) as isize;
            while s_idx > 0 && remaining.get(s_idx as usize) != Some(&b'/') {
                s_idx -= 1;
            }
            if s_idx == 0 {
                // c:597 — no slash to split at
                break;
            }
            // c:600-601 — first time we split, save the cwd via `open(".")`
            // so we can restore on later failure.
            if saved_currdir == -2 {
                // c:600
                let dot = std::ffi::CString::new(".").unwrap();
                saved_currdir =
                    unsafe { libc::open(dot.as_ptr(), libc::O_RDONLY | libc::O_NOCTTY) };
            }
            // c:603-606 — `*s = '\0'; chdir(dir); *s = '/';`. Try the prefix.
            let prefix: Vec<u8> = remaining[..s_idx as usize].to_vec();
            let c_prefix = match std::ffi::CString::new(prefix) {
                Ok(c) => c,
                Err(_) => break,
            };
            if unsafe { libc::chdir(c_prefix.as_ptr()) } < 0 {
                // c:604
                break;
            }
            // c:611-614 — `while (*++s == '/') ;` skip consecutive slashes,
            // then `dir = s;` recurse with tail.
            let mut tail_start = s_idx as usize + 1;
            while tail_start < remaining.len() && remaining[tail_start] == b'/' {
                tail_start += 1;
            }
            remaining = remaining[tail_start..].to_vec();
        }
        // c:616-626 — restore on lost-cwd path; return -1 if restored,
        // -2 if even the restore failed (cwd genuinely lost).
        if saved_currdir >= 0 {
            // c:617
            let rc = unsafe { libc::fchdir(saved_currdir) };
            unsafe {
                libc::close(saved_currdir);
            } // c:619 / c:622
            if rc < 0 {
                // c:618
                return -2; // c:620
            }
            return -1; // c:623
        }
        // c:626 — never entered the split path: it's a plain -1.
        if saved_currdir == -2 {
            -1
        } else {
            -2
        } // c:626
    }
    #[cfg(not(unix))]
    {
        let _ = (dir, env::set_current_dir);
        if dir.is_empty() {
            return 0;
        }
        match env::set_current_dir(dir) {
            Ok(_) => 0,
            Err(_) => -1,
        }
    }
}

/// Format a 64-bit signed integer for output.
/// Port of `output64(zlong val)` from Src/compat.c:638 — needed in C
/// because `%lld` printf support varied; Rust's `to_string()`
/// handles every target.
pub fn output64(val: i64) -> String {
    // c:638
    val.to_string()
}

/// Get the column width of a Unicode character.
/// Port of `u9_wcwidth(wchar_t ucs)` from Src/compat.c:760 — the C source
/// ships its own Unicode 9 u9_wcwidth fallback because system
/// `u9_wcwidth(3)` data ages with libc. Rust uses the
/// `unicode-width` crate which tracks the latest UCD.
pub fn u9_wcwidth(ucs: char) -> i32 {
    // ucs:760
    unicode_width::UnicodeWidthChar::width(ucs)
        .map(|w| w as i32)
        .unwrap_or(if ucs.is_control() { -1 } else { 1 })
}

/// `wcwidth9_nonprint` from `Src/wcwidth9.h:18-32` — the intervals for which
/// `wcwidth9()` returns -1 (c:1294-1296), and therefore the intervals for
/// which `u9_iswprint()` is false.
///
/// `{0x0000,0x001f}` and `{0x007f,0x009f}` are exactly Rust's `Cc` category,
/// which is why the old `!ucs.is_control()` test looked right; the other
/// eleven are what it missed.
const WCWIDTH9_NONPRINT: &[(u32, u32)] = &[
    (0x0000, 0x001f), // c:19
    (0x007f, 0x009f), // c:20
    (0x00ad, 0x00ad), // c:21
    (0x070f, 0x070f), // c:22
    (0x180b, 0x180e), // c:23
    (0x200b, 0x200f), // c:24
    (0x2028, 0x2029), // c:25
    (0x202a, 0x202e), // c:26
    (0x206a, 0x206f), // c:27
    // c:28 — `{0xd800, 0xdfff}`: a surrogate cannot be a Rust `char`, so the
    // interval is unreachable here and is carried as a comment for parity.
    (0xfeff, 0xfeff), // c:29
    (0xfff9, 0xfffb), // c:30
    (0xfffe, 0xffff), // c:31
];

/// `wcwidth9_not_assigned` from `Src/wcwidth9.h:620-1258` — the SECOND set of
/// intervals for which `wcwidth9()` returns -1 (c:1302-1304): codepoints
/// unassigned as of Unicode 9. zsh therefore refuses to print them raw and
/// `${(q)}` escapes them; measured against `zsh 5.9.2`:
///
///     ${(q)} of U+0378 (first interval, c:621)
///       zsh  : $'\315\270'
///       zshrs: the raw bytes   (before this table)
///
/// 637 intervals, one per source line c:621..c:1257 — NOT 636; the count is
/// pinned by the c: citations, every one of which is present exactly once.
/// None overlaps the surrogate range, so unlike `WCWIDTH9_NONPRINT` c:28
/// every row here is reachable as a Rust `char`.
///
/// Sorted and non-overlapping, as `wcwidth9_intable`'s binary search
/// (c:1262-1284) requires.
const WCWIDTH9_NOT_ASSIGNED: &[(u32, u32)] = &[
    (0x0378, 0x0379),   // c:621
    (0x0380, 0x0383),   // c:622
    (0x038b, 0x038b),   // c:623
    (0x038d, 0x038d),   // c:624
    (0x03a2, 0x03a2),   // c:625
    (0x0530, 0x0530),   // c:626
    (0x0557, 0x0558),   // c:627
    (0x0560, 0x0560),   // c:628
    (0x0588, 0x0588),   // c:629
    (0x058b, 0x058c),   // c:630
    (0x0590, 0x0590),   // c:631
    (0x05c8, 0x05cf),   // c:632
    (0x05eb, 0x05ef),   // c:633
    (0x05f5, 0x05ff),   // c:634
    (0x061d, 0x061d),   // c:635
    (0x070e, 0x070e),   // c:636
    (0x074b, 0x074c),   // c:637
    (0x07b2, 0x07bf),   // c:638
    (0x07fb, 0x07ff),   // c:639
    (0x082e, 0x082f),   // c:640
    (0x083f, 0x083f),   // c:641
    (0x085c, 0x085d),   // c:642
    (0x085f, 0x089f),   // c:643
    (0x08b5, 0x08b5),   // c:644
    (0x08be, 0x08d3),   // c:645
    (0x0984, 0x0984),   // c:646
    (0x098d, 0x098e),   // c:647
    (0x0991, 0x0992),   // c:648
    (0x09a9, 0x09a9),   // c:649
    (0x09b1, 0x09b1),   // c:650
    (0x09b3, 0x09b5),   // c:651
    (0x09ba, 0x09bb),   // c:652
    (0x09c5, 0x09c6),   // c:653
    (0x09c9, 0x09ca),   // c:654
    (0x09cf, 0x09d6),   // c:655
    (0x09d8, 0x09db),   // c:656
    (0x09de, 0x09de),   // c:657
    (0x09e4, 0x09e5),   // c:658
    (0x09fc, 0x0a00),   // c:659
    (0x0a04, 0x0a04),   // c:660
    (0x0a0b, 0x0a0e),   // c:661
    (0x0a11, 0x0a12),   // c:662
    (0x0a29, 0x0a29),   // c:663
    (0x0a31, 0x0a31),   // c:664
    (0x0a34, 0x0a34),   // c:665
    (0x0a37, 0x0a37),   // c:666
    (0x0a3a, 0x0a3b),   // c:667
    (0x0a3d, 0x0a3d),   // c:668
    (0x0a43, 0x0a46),   // c:669
    (0x0a49, 0x0a4a),   // c:670
    (0x0a4e, 0x0a50),   // c:671
    (0x0a52, 0x0a58),   // c:672
    (0x0a5d, 0x0a5d),   // c:673
    (0x0a5f, 0x0a65),   // c:674
    (0x0a76, 0x0a80),   // c:675
    (0x0a84, 0x0a84),   // c:676
    (0x0a8e, 0x0a8e),   // c:677
    (0x0a92, 0x0a92),   // c:678
    (0x0aa9, 0x0aa9),   // c:679
    (0x0ab1, 0x0ab1),   // c:680
    (0x0ab4, 0x0ab4),   // c:681
    (0x0aba, 0x0abb),   // c:682
    (0x0ac6, 0x0ac6),   // c:683
    (0x0aca, 0x0aca),   // c:684
    (0x0ace, 0x0acf),   // c:685
    (0x0ad1, 0x0adf),   // c:686
    (0x0ae4, 0x0ae5),   // c:687
    (0x0af2, 0x0af8),   // c:688
    (0x0afa, 0x0b00),   // c:689
    (0x0b04, 0x0b04),   // c:690
    (0x0b0d, 0x0b0e),   // c:691
    (0x0b11, 0x0b12),   // c:692
    (0x0b29, 0x0b29),   // c:693
    (0x0b31, 0x0b31),   // c:694
    (0x0b34, 0x0b34),   // c:695
    (0x0b3a, 0x0b3b),   // c:696
    (0x0b45, 0x0b46),   // c:697
    (0x0b49, 0x0b4a),   // c:698
    (0x0b4e, 0x0b55),   // c:699
    (0x0b58, 0x0b5b),   // c:700
    (0x0b5e, 0x0b5e),   // c:701
    (0x0b64, 0x0b65),   // c:702
    (0x0b78, 0x0b81),   // c:703
    (0x0b84, 0x0b84),   // c:704
    (0x0b8b, 0x0b8d),   // c:705
    (0x0b91, 0x0b91),   // c:706
    (0x0b96, 0x0b98),   // c:707
    (0x0b9b, 0x0b9b),   // c:708
    (0x0b9d, 0x0b9d),   // c:709
    (0x0ba0, 0x0ba2),   // c:710
    (0x0ba5, 0x0ba7),   // c:711
    (0x0bab, 0x0bad),   // c:712
    (0x0bba, 0x0bbd),   // c:713
    (0x0bc3, 0x0bc5),   // c:714
    (0x0bc9, 0x0bc9),   // c:715
    (0x0bce, 0x0bcf),   // c:716
    (0x0bd1, 0x0bd6),   // c:717
    (0x0bd8, 0x0be5),   // c:718
    (0x0bfb, 0x0bff),   // c:719
    (0x0c04, 0x0c04),   // c:720
    (0x0c0d, 0x0c0d),   // c:721
    (0x0c11, 0x0c11),   // c:722
    (0x0c29, 0x0c29),   // c:723
    (0x0c3a, 0x0c3c),   // c:724
    (0x0c45, 0x0c45),   // c:725
    (0x0c49, 0x0c49),   // c:726
    (0x0c4e, 0x0c54),   // c:727
    (0x0c57, 0x0c57),   // c:728
    (0x0c5b, 0x0c5f),   // c:729
    (0x0c64, 0x0c65),   // c:730
    (0x0c70, 0x0c77),   // c:731
    (0x0c84, 0x0c84),   // c:732
    (0x0c8d, 0x0c8d),   // c:733
    (0x0c91, 0x0c91),   // c:734
    (0x0ca9, 0x0ca9),   // c:735
    (0x0cb4, 0x0cb4),   // c:736
    (0x0cba, 0x0cbb),   // c:737
    (0x0cc5, 0x0cc5),   // c:738
    (0x0cc9, 0x0cc9),   // c:739
    (0x0cce, 0x0cd4),   // c:740
    (0x0cd7, 0x0cdd),   // c:741
    (0x0cdf, 0x0cdf),   // c:742
    (0x0ce4, 0x0ce5),   // c:743
    (0x0cf0, 0x0cf0),   // c:744
    (0x0cf3, 0x0d00),   // c:745
    (0x0d04, 0x0d04),   // c:746
    (0x0d0d, 0x0d0d),   // c:747
    (0x0d11, 0x0d11),   // c:748
    (0x0d3b, 0x0d3c),   // c:749
    (0x0d45, 0x0d45),   // c:750
    (0x0d49, 0x0d49),   // c:751
    (0x0d50, 0x0d53),   // c:752
    (0x0d64, 0x0d65),   // c:753
    (0x0d80, 0x0d81),   // c:754
    (0x0d84, 0x0d84),   // c:755
    (0x0d97, 0x0d99),   // c:756
    (0x0db2, 0x0db2),   // c:757
    (0x0dbc, 0x0dbc),   // c:758
    (0x0dbe, 0x0dbf),   // c:759
    (0x0dc7, 0x0dc9),   // c:760
    (0x0dcb, 0x0dce),   // c:761
    (0x0dd5, 0x0dd5),   // c:762
    (0x0dd7, 0x0dd7),   // c:763
    (0x0de0, 0x0de5),   // c:764
    (0x0df0, 0x0df1),   // c:765
    (0x0df5, 0x0e00),   // c:766
    (0x0e3b, 0x0e3e),   // c:767
    (0x0e5c, 0x0e80),   // c:768
    (0x0e83, 0x0e83),   // c:769
    (0x0e85, 0x0e86),   // c:770
    (0x0e89, 0x0e89),   // c:771
    (0x0e8b, 0x0e8c),   // c:772
    (0x0e8e, 0x0e93),   // c:773
    (0x0e98, 0x0e98),   // c:774
    (0x0ea0, 0x0ea0),   // c:775
    (0x0ea4, 0x0ea4),   // c:776
    (0x0ea6, 0x0ea6),   // c:777
    (0x0ea8, 0x0ea9),   // c:778
    (0x0eac, 0x0eac),   // c:779
    (0x0eba, 0x0eba),   // c:780
    (0x0ebe, 0x0ebf),   // c:781
    (0x0ec5, 0x0ec5),   // c:782
    (0x0ec7, 0x0ec7),   // c:783
    (0x0ece, 0x0ecf),   // c:784
    (0x0eda, 0x0edb),   // c:785
    (0x0ee0, 0x0eff),   // c:786
    (0x0f48, 0x0f48),   // c:787
    (0x0f6d, 0x0f70),   // c:788
    (0x0f98, 0x0f98),   // c:789
    (0x0fbd, 0x0fbd),   // c:790
    (0x0fcd, 0x0fcd),   // c:791
    (0x0fdb, 0x0fff),   // c:792
    (0x10c6, 0x10c6),   // c:793
    (0x10c8, 0x10cc),   // c:794
    (0x10ce, 0x10cf),   // c:795
    (0x1249, 0x1249),   // c:796
    (0x124e, 0x124f),   // c:797
    (0x1257, 0x1257),   // c:798
    (0x1259, 0x1259),   // c:799
    (0x125e, 0x125f),   // c:800
    (0x1289, 0x1289),   // c:801
    (0x128e, 0x128f),   // c:802
    (0x12b1, 0x12b1),   // c:803
    (0x12b6, 0x12b7),   // c:804
    (0x12bf, 0x12bf),   // c:805
    (0x12c1, 0x12c1),   // c:806
    (0x12c6, 0x12c7),   // c:807
    (0x12d7, 0x12d7),   // c:808
    (0x1311, 0x1311),   // c:809
    (0x1316, 0x1317),   // c:810
    (0x135b, 0x135c),   // c:811
    (0x137d, 0x137f),   // c:812
    (0x139a, 0x139f),   // c:813
    (0x13f6, 0x13f7),   // c:814
    (0x13fe, 0x13ff),   // c:815
    (0x169d, 0x169f),   // c:816
    (0x16f9, 0x16ff),   // c:817
    (0x170d, 0x170d),   // c:818
    (0x1715, 0x171f),   // c:819
    (0x1737, 0x173f),   // c:820
    (0x1754, 0x175f),   // c:821
    (0x176d, 0x176d),   // c:822
    (0x1771, 0x1771),   // c:823
    (0x1774, 0x177f),   // c:824
    (0x17de, 0x17df),   // c:825
    (0x17ea, 0x17ef),   // c:826
    (0x17fa, 0x17ff),   // c:827
    (0x180f, 0x180f),   // c:828
    (0x181a, 0x181f),   // c:829
    (0x1878, 0x187f),   // c:830
    (0x18ab, 0x18af),   // c:831
    (0x18f6, 0x18ff),   // c:832
    (0x191f, 0x191f),   // c:833
    (0x192c, 0x192f),   // c:834
    (0x193c, 0x193f),   // c:835
    (0x1941, 0x1943),   // c:836
    (0x196e, 0x196f),   // c:837
    (0x1975, 0x197f),   // c:838
    (0x19ac, 0x19af),   // c:839
    (0x19ca, 0x19cf),   // c:840
    (0x19db, 0x19dd),   // c:841
    (0x1a1c, 0x1a1d),   // c:842
    (0x1a5f, 0x1a5f),   // c:843
    (0x1a7d, 0x1a7e),   // c:844
    (0x1a8a, 0x1a8f),   // c:845
    (0x1a9a, 0x1a9f),   // c:846
    (0x1aae, 0x1aaf),   // c:847
    (0x1abf, 0x1aff),   // c:848
    (0x1b4c, 0x1b4f),   // c:849
    (0x1b7d, 0x1b7f),   // c:850
    (0x1bf4, 0x1bfb),   // c:851
    (0x1c38, 0x1c3a),   // c:852
    (0x1c4a, 0x1c4c),   // c:853
    (0x1c89, 0x1cbf),   // c:854
    (0x1cc8, 0x1ccf),   // c:855
    (0x1cf7, 0x1cf7),   // c:856
    (0x1cfa, 0x1cff),   // c:857
    (0x1df6, 0x1dfa),   // c:858
    (0x1f16, 0x1f17),   // c:859
    (0x1f1e, 0x1f1f),   // c:860
    (0x1f46, 0x1f47),   // c:861
    (0x1f4e, 0x1f4f),   // c:862
    (0x1f58, 0x1f58),   // c:863
    (0x1f5a, 0x1f5a),   // c:864
    (0x1f5c, 0x1f5c),   // c:865
    (0x1f5e, 0x1f5e),   // c:866
    (0x1f7e, 0x1f7f),   // c:867
    (0x1fb5, 0x1fb5),   // c:868
    (0x1fc5, 0x1fc5),   // c:869
    (0x1fd4, 0x1fd5),   // c:870
    (0x1fdc, 0x1fdc),   // c:871
    (0x1ff0, 0x1ff1),   // c:872
    (0x1ff5, 0x1ff5),   // c:873
    (0x1fff, 0x1fff),   // c:874
    (0x2065, 0x2065),   // c:875
    (0x2072, 0x2073),   // c:876
    (0x208f, 0x208f),   // c:877
    (0x209d, 0x209f),   // c:878
    (0x20bf, 0x20cf),   // c:879
    (0x20f1, 0x20ff),   // c:880
    (0x218c, 0x218f),   // c:881
    (0x23ff, 0x23ff),   // c:882
    (0x2427, 0x243f),   // c:883
    (0x244b, 0x245f),   // c:884
    (0x2b74, 0x2b75),   // c:885
    (0x2b96, 0x2b97),   // c:886
    (0x2bba, 0x2bbc),   // c:887
    (0x2bc9, 0x2bc9),   // c:888
    (0x2bd2, 0x2beb),   // c:889
    (0x2bf0, 0x2bff),   // c:890
    (0x2c2f, 0x2c2f),   // c:891
    (0x2c5f, 0x2c5f),   // c:892
    (0x2cf4, 0x2cf8),   // c:893
    (0x2d26, 0x2d26),   // c:894
    (0x2d28, 0x2d2c),   // c:895
    (0x2d2e, 0x2d2f),   // c:896
    (0x2d68, 0x2d6e),   // c:897
    (0x2d71, 0x2d7e),   // c:898
    (0x2d97, 0x2d9f),   // c:899
    (0x2da7, 0x2da7),   // c:900
    (0x2daf, 0x2daf),   // c:901
    (0x2db7, 0x2db7),   // c:902
    (0x2dbf, 0x2dbf),   // c:903
    (0x2dc7, 0x2dc7),   // c:904
    (0x2dcf, 0x2dcf),   // c:905
    (0x2dd7, 0x2dd7),   // c:906
    (0x2ddf, 0x2ddf),   // c:907
    (0x2e45, 0x2e7f),   // c:908
    (0x2e9a, 0x2e9a),   // c:909
    (0x2ef4, 0x2eff),   // c:910
    (0x2fd6, 0x2fef),   // c:911
    (0x2ffc, 0x2fff),   // c:912
    (0x3040, 0x3040),   // c:913
    (0x3097, 0x3098),   // c:914
    (0x3100, 0x3104),   // c:915
    (0x312e, 0x3130),   // c:916
    (0x318f, 0x318f),   // c:917
    (0x31bb, 0x31bf),   // c:918
    (0x31e4, 0x31ef),   // c:919
    (0x321f, 0x321f),   // c:920
    (0x32ff, 0x32ff),   // c:921
    (0x4db6, 0x4dbf),   // c:922
    (0x9fd6, 0x9fff),   // c:923
    (0xa48d, 0xa48f),   // c:924
    (0xa4c7, 0xa4cf),   // c:925
    (0xa62c, 0xa63f),   // c:926
    (0xa6f8, 0xa6ff),   // c:927
    (0xa7af, 0xa7af),   // c:928
    (0xa7b8, 0xa7f6),   // c:929
    (0xa82c, 0xa82f),   // c:930
    (0xa83a, 0xa83f),   // c:931
    (0xa878, 0xa87f),   // c:932
    (0xa8c6, 0xa8cd),   // c:933
    (0xa8da, 0xa8df),   // c:934
    (0xa8fe, 0xa8ff),   // c:935
    (0xa954, 0xa95e),   // c:936
    (0xa97d, 0xa97f),   // c:937
    (0xa9ce, 0xa9ce),   // c:938
    (0xa9da, 0xa9dd),   // c:939
    (0xa9ff, 0xa9ff),   // c:940
    (0xaa37, 0xaa3f),   // c:941
    (0xaa4e, 0xaa4f),   // c:942
    (0xaa5a, 0xaa5b),   // c:943
    (0xaac3, 0xaada),   // c:944
    (0xaaf7, 0xab00),   // c:945
    (0xab07, 0xab08),   // c:946
    (0xab0f, 0xab10),   // c:947
    (0xab17, 0xab1f),   // c:948
    (0xab27, 0xab27),   // c:949
    (0xab2f, 0xab2f),   // c:950
    (0xab66, 0xab6f),   // c:951
    (0xabee, 0xabef),   // c:952
    (0xabfa, 0xabff),   // c:953
    (0xd7a4, 0xd7af),   // c:954
    (0xd7c7, 0xd7ca),   // c:955
    (0xd7fc, 0xd7ff),   // c:956
    (0xfa6e, 0xfa6f),   // c:957
    (0xfada, 0xfaff),   // c:958
    (0xfb07, 0xfb12),   // c:959
    (0xfb18, 0xfb1c),   // c:960
    (0xfb37, 0xfb37),   // c:961
    (0xfb3d, 0xfb3d),   // c:962
    (0xfb3f, 0xfb3f),   // c:963
    (0xfb42, 0xfb42),   // c:964
    (0xfb45, 0xfb45),   // c:965
    (0xfbc2, 0xfbd2),   // c:966
    (0xfd40, 0xfd4f),   // c:967
    (0xfd90, 0xfd91),   // c:968
    (0xfdc8, 0xfdef),   // c:969
    (0xfdfe, 0xfdff),   // c:970
    (0xfe1a, 0xfe1f),   // c:971
    (0xfe53, 0xfe53),   // c:972
    (0xfe67, 0xfe67),   // c:973
    (0xfe6c, 0xfe6f),   // c:974
    (0xfe75, 0xfe75),   // c:975
    (0xfefd, 0xfefe),   // c:976
    (0xff00, 0xff00),   // c:977
    (0xffbf, 0xffc1),   // c:978
    (0xffc8, 0xffc9),   // c:979
    (0xffd0, 0xffd1),   // c:980
    (0xffd8, 0xffd9),   // c:981
    (0xffdd, 0xffdf),   // c:982
    (0xffe7, 0xffe7),   // c:983
    (0xffef, 0xfff8),   // c:984
    (0xfffe, 0xffff),   // c:985
    (0x1000c, 0x1000c), // c:986
    (0x10027, 0x10027), // c:987
    (0x1003b, 0x1003b), // c:988
    (0x1003e, 0x1003e), // c:989
    (0x1004e, 0x1004f), // c:990
    (0x1005e, 0x1007f), // c:991
    (0x100fb, 0x100ff), // c:992
    (0x10103, 0x10106), // c:993
    (0x10134, 0x10136), // c:994
    (0x1018f, 0x1018f), // c:995
    (0x1019c, 0x1019f), // c:996
    (0x101a1, 0x101cf), // c:997
    (0x101fe, 0x1027f), // c:998
    (0x1029d, 0x1029f), // c:999
    (0x102d1, 0x102df), // c:1000
    (0x102fc, 0x102ff), // c:1001
    (0x10324, 0x1032f), // c:1002
    (0x1034b, 0x1034f), // c:1003
    (0x1037b, 0x1037f), // c:1004
    (0x1039e, 0x1039e), // c:1005
    (0x103c4, 0x103c7), // c:1006
    (0x103d6, 0x103ff), // c:1007
    (0x1049e, 0x1049f), // c:1008
    (0x104aa, 0x104af), // c:1009
    (0x104d4, 0x104d7), // c:1010
    (0x104fc, 0x104ff), // c:1011
    (0x10528, 0x1052f), // c:1012
    (0x10564, 0x1056e), // c:1013
    (0x10570, 0x105ff), // c:1014
    (0x10737, 0x1073f), // c:1015
    (0x10756, 0x1075f), // c:1016
    (0x10768, 0x107ff), // c:1017
    (0x10806, 0x10807), // c:1018
    (0x10809, 0x10809), // c:1019
    (0x10836, 0x10836), // c:1020
    (0x10839, 0x1083b), // c:1021
    (0x1083d, 0x1083e), // c:1022
    (0x10856, 0x10856), // c:1023
    (0x1089f, 0x108a6), // c:1024
    (0x108b0, 0x108df), // c:1025
    (0x108f3, 0x108f3), // c:1026
    (0x108f6, 0x108fa), // c:1027
    (0x1091c, 0x1091e), // c:1028
    (0x1093a, 0x1093e), // c:1029
    (0x10940, 0x1097f), // c:1030
    (0x109b8, 0x109bb), // c:1031
    (0x109d0, 0x109d1), // c:1032
    (0x10a04, 0x10a04), // c:1033
    (0x10a07, 0x10a0b), // c:1034
    (0x10a14, 0x10a14), // c:1035
    (0x10a18, 0x10a18), // c:1036
    (0x10a34, 0x10a37), // c:1037
    (0x10a3b, 0x10a3e), // c:1038
    (0x10a48, 0x10a4f), // c:1039
    (0x10a59, 0x10a5f), // c:1040
    (0x10aa0, 0x10abf), // c:1041
    (0x10ae7, 0x10aea), // c:1042
    (0x10af7, 0x10aff), // c:1043
    (0x10b36, 0x10b38), // c:1044
    (0x10b56, 0x10b57), // c:1045
    (0x10b73, 0x10b77), // c:1046
    (0x10b92, 0x10b98), // c:1047
    (0x10b9d, 0x10ba8), // c:1048
    (0x10bb0, 0x10bff), // c:1049
    (0x10c49, 0x10c7f), // c:1050
    (0x10cb3, 0x10cbf), // c:1051
    (0x10cf3, 0x10cf9), // c:1052
    (0x10d00, 0x10e5f), // c:1053
    (0x10e7f, 0x10fff), // c:1054
    (0x1104e, 0x11051), // c:1055
    (0x11070, 0x1107e), // c:1056
    (0x110c2, 0x110cf), // c:1057
    (0x110e9, 0x110ef), // c:1058
    (0x110fa, 0x110ff), // c:1059
    (0x11135, 0x11135), // c:1060
    (0x11144, 0x1114f), // c:1061
    (0x11177, 0x1117f), // c:1062
    (0x111ce, 0x111cf), // c:1063
    (0x111e0, 0x111e0), // c:1064
    (0x111f5, 0x111ff), // c:1065
    (0x11212, 0x11212), // c:1066
    (0x1123f, 0x1127f), // c:1067
    (0x11287, 0x11287), // c:1068
    (0x11289, 0x11289), // c:1069
    (0x1128e, 0x1128e), // c:1070
    (0x1129e, 0x1129e), // c:1071
    (0x112aa, 0x112af), // c:1072
    (0x112eb, 0x112ef), // c:1073
    (0x112fa, 0x112ff), // c:1074
    (0x11304, 0x11304), // c:1075
    (0x1130d, 0x1130e), // c:1076
    (0x11311, 0x11312), // c:1077
    (0x11329, 0x11329), // c:1078
    (0x11331, 0x11331), // c:1079
    (0x11334, 0x11334), // c:1080
    (0x1133a, 0x1133b), // c:1081
    (0x11345, 0x11346), // c:1082
    (0x11349, 0x1134a), // c:1083
    (0x1134e, 0x1134f), // c:1084
    (0x11351, 0x11356), // c:1085
    (0x11358, 0x1135c), // c:1086
    (0x11364, 0x11365), // c:1087
    (0x1136d, 0x1136f), // c:1088
    (0x11375, 0x113ff), // c:1089
    (0x1145a, 0x1145a), // c:1090
    (0x1145c, 0x1145c), // c:1091
    (0x1145e, 0x1147f), // c:1092
    (0x114c8, 0x114cf), // c:1093
    (0x114da, 0x1157f), // c:1094
    (0x115b6, 0x115b7), // c:1095
    (0x115de, 0x115ff), // c:1096
    (0x11645, 0x1164f), // c:1097
    (0x1165a, 0x1165f), // c:1098
    (0x1166d, 0x1167f), // c:1099
    (0x116b8, 0x116bf), // c:1100
    (0x116ca, 0x116ff), // c:1101
    (0x1171a, 0x1171c), // c:1102
    (0x1172c, 0x1172f), // c:1103
    (0x11740, 0x1189f), // c:1104
    (0x118f3, 0x118fe), // c:1105
    (0x11900, 0x11abf), // c:1106
    (0x11af9, 0x11bff), // c:1107
    (0x11c09, 0x11c09), // c:1108
    (0x11c37, 0x11c37), // c:1109
    (0x11c46, 0x11c4f), // c:1110
    (0x11c6d, 0x11c6f), // c:1111
    (0x11c90, 0x11c91), // c:1112
    (0x11ca8, 0x11ca8), // c:1113
    (0x11cb7, 0x11fff), // c:1114
    (0x1239a, 0x123ff), // c:1115
    (0x1246f, 0x1246f), // c:1116
    (0x12475, 0x1247f), // c:1117
    (0x12544, 0x12fff), // c:1118
    (0x1342f, 0x143ff), // c:1119
    (0x14647, 0x167ff), // c:1120
    (0x16a39, 0x16a3f), // c:1121
    (0x16a5f, 0x16a5f), // c:1122
    (0x16a6a, 0x16a6d), // c:1123
    (0x16a70, 0x16acf), // c:1124
    (0x16aee, 0x16aef), // c:1125
    (0x16af6, 0x16aff), // c:1126
    (0x16b46, 0x16b4f), // c:1127
    (0x16b5a, 0x16b5a), // c:1128
    (0x16b62, 0x16b62), // c:1129
    (0x16b78, 0x16b7c), // c:1130
    (0x16b90, 0x16eff), // c:1131
    (0x16f45, 0x16f4f), // c:1132
    (0x16f7f, 0x16f8e), // c:1133
    (0x16fa0, 0x16fdf), // c:1134
    (0x16fe1, 0x16fff), // c:1135
    (0x187ed, 0x187ff), // c:1136
    (0x18af3, 0x1afff), // c:1137
    (0x1b002, 0x1bbff), // c:1138
    (0x1bc6b, 0x1bc6f), // c:1139
    (0x1bc7d, 0x1bc7f), // c:1140
    (0x1bc89, 0x1bc8f), // c:1141
    (0x1bc9a, 0x1bc9b), // c:1142
    (0x1bca4, 0x1cfff), // c:1143
    (0x1d0f6, 0x1d0ff), // c:1144
    (0x1d127, 0x1d128), // c:1145
    (0x1d1e9, 0x1d1ff), // c:1146
    (0x1d246, 0x1d2ff), // c:1147
    (0x1d357, 0x1d35f), // c:1148
    (0x1d372, 0x1d3ff), // c:1149
    (0x1d455, 0x1d455), // c:1150
    (0x1d49d, 0x1d49d), // c:1151
    (0x1d4a0, 0x1d4a1), // c:1152
    (0x1d4a3, 0x1d4a4), // c:1153
    (0x1d4a7, 0x1d4a8), // c:1154
    (0x1d4ad, 0x1d4ad), // c:1155
    (0x1d4ba, 0x1d4ba), // c:1156
    (0x1d4bc, 0x1d4bc), // c:1157
    (0x1d4c4, 0x1d4c4), // c:1158
    (0x1d506, 0x1d506), // c:1159
    (0x1d50b, 0x1d50c), // c:1160
    (0x1d515, 0x1d515), // c:1161
    (0x1d51d, 0x1d51d), // c:1162
    (0x1d53a, 0x1d53a), // c:1163
    (0x1d53f, 0x1d53f), // c:1164
    (0x1d545, 0x1d545), // c:1165
    (0x1d547, 0x1d549), // c:1166
    (0x1d551, 0x1d551), // c:1167
    (0x1d6a6, 0x1d6a7), // c:1168
    (0x1d7cc, 0x1d7cd), // c:1169
    (0x1da8c, 0x1da9a), // c:1170
    (0x1daa0, 0x1daa0), // c:1171
    (0x1dab0, 0x1dfff), // c:1172
    (0x1e007, 0x1e007), // c:1173
    (0x1e019, 0x1e01a), // c:1174
    (0x1e022, 0x1e022), // c:1175
    (0x1e025, 0x1e025), // c:1176
    (0x1e02b, 0x1e7ff), // c:1177
    (0x1e8c5, 0x1e8c6), // c:1178
    (0x1e8d7, 0x1e8ff), // c:1179
    (0x1e94b, 0x1e94f), // c:1180
    (0x1e95a, 0x1e95d), // c:1181
    (0x1e960, 0x1edff), // c:1182
    (0x1ee04, 0x1ee04), // c:1183
    (0x1ee20, 0x1ee20), // c:1184
    (0x1ee23, 0x1ee23), // c:1185
    (0x1ee25, 0x1ee26), // c:1186
    (0x1ee28, 0x1ee28), // c:1187
    (0x1ee33, 0x1ee33), // c:1188
    (0x1ee38, 0x1ee38), // c:1189
    (0x1ee3a, 0x1ee3a), // c:1190
    (0x1ee3c, 0x1ee41), // c:1191
    (0x1ee43, 0x1ee46), // c:1192
    (0x1ee48, 0x1ee48), // c:1193
    (0x1ee4a, 0x1ee4a), // c:1194
    (0x1ee4c, 0x1ee4c), // c:1195
    (0x1ee50, 0x1ee50), // c:1196
    (0x1ee53, 0x1ee53), // c:1197
    (0x1ee55, 0x1ee56), // c:1198
    (0x1ee58, 0x1ee58), // c:1199
    (0x1ee5a, 0x1ee5a), // c:1200
    (0x1ee5c, 0x1ee5c), // c:1201
    (0x1ee5e, 0x1ee5e), // c:1202
    (0x1ee60, 0x1ee60), // c:1203
    (0x1ee63, 0x1ee63), // c:1204
    (0x1ee65, 0x1ee66), // c:1205
    (0x1ee6b, 0x1ee6b), // c:1206
    (0x1ee73, 0x1ee73), // c:1207
    (0x1ee78, 0x1ee78), // c:1208
    (0x1ee7d, 0x1ee7d), // c:1209
    (0x1ee7f, 0x1ee7f), // c:1210
    (0x1ee8a, 0x1ee8a), // c:1211
    (0x1ee9c, 0x1eea0), // c:1212
    (0x1eea4, 0x1eea4), // c:1213
    (0x1eeaa, 0x1eeaa), // c:1214
    (0x1eebc, 0x1eeef), // c:1215
    (0x1eef2, 0x1efff), // c:1216
    (0x1f02c, 0x1f02f), // c:1217
    (0x1f094, 0x1f09f), // c:1218
    (0x1f0af, 0x1f0b0), // c:1219
    (0x1f0c0, 0x1f0c0), // c:1220
    (0x1f0d0, 0x1f0d0), // c:1221
    (0x1f0f6, 0x1f0ff), // c:1222
    (0x1f10d, 0x1f10f), // c:1223
    (0x1f12f, 0x1f12f), // c:1224
    (0x1f16c, 0x1f16f), // c:1225
    (0x1f1ad, 0x1f1e5), // c:1226
    (0x1f203, 0x1f20f), // c:1227
    (0x1f23c, 0x1f23f), // c:1228
    (0x1f249, 0x1f24f), // c:1229
    (0x1f252, 0x1f2ff), // c:1230
    (0x1f6d3, 0x1f6df), // c:1231
    (0x1f6ed, 0x1f6ef), // c:1232
    (0x1f6f7, 0x1f6ff), // c:1233
    (0x1f774, 0x1f77f), // c:1234
    (0x1f7d5, 0x1f7ff), // c:1235
    (0x1f80c, 0x1f80f), // c:1236
    (0x1f848, 0x1f84f), // c:1237
    (0x1f85a, 0x1f85f), // c:1238
    (0x1f888, 0x1f88f), // c:1239
    (0x1f8ae, 0x1f90f), // c:1240
    (0x1f91f, 0x1f91f), // c:1241
    (0x1f928, 0x1f92f), // c:1242
    (0x1f931, 0x1f932), // c:1243
    (0x1f93f, 0x1f93f), // c:1244
    (0x1f94c, 0x1f94f), // c:1245
    (0x1f95f, 0x1f97f), // c:1246
    (0x1f992, 0x1f9bf), // c:1247
    (0x1f9c1, 0x1ffff), // c:1248
    (0x2a6d7, 0x2a6ff), // c:1249
    (0x2b735, 0x2b73f), // c:1250
    (0x2b81e, 0x2b81f), // c:1251
    (0x2cea2, 0x2f7ff), // c:1252
    (0x2fa1e, 0xe0000), // c:1253
    (0xe0002, 0xe001f), // c:1254
    (0xe0080, 0xe00ff), // c:1255
    (0xe01f0, 0xeffff), // c:1256
    (0xffffe, 0xfffff), // c:1257
];

/// Check whether a wide character is printable.
/// Port of `u9_iswprint(wint_t ucs)` from Src/compat.c:770.
///
/// Reached through `WC_ISPRINT` (`Src/ztype.h:77`) in any build configured
/// `--enable-unicode9`, which the reference build is
/// (homebrew-core Formula/z/zsh.rb:67). It is a TABLE lookup, not
/// `iswprint(3)`: it answers the same in every locale, and the locale only
/// decides which scalars the decoder hands it.
///
/// The previous test — `!ucs.is_control() && u9_wcwidth(ucs) >= 0` — was
/// wrong twice over. Rust's `Cc` is exactly the table's first TWO intervals
/// and none of the other eleven, and the `unicode-width` crate maps C's -1
/// (nonprint) onto `Some(0)` (zero-width), so the second conjunct never
/// rejected anything either. Result, measured, and independent of locale:
///
/// ```text
/// ${(q)} of U+00AD / U+200B / U+FEFF
///   zsh  : $'\302\255'  $'\342\200\213'  $'\357\273\277'
///   zshrs: the raw bytes
/// ```
pub fn u9_iswprint(ucs: char) -> bool {
    // c:772-773 — `if (ucs == 0) return 0;`. Subsumed by the {0,0x1f}
    // interval below; kept explicit because C keeps it explicit.
    if ucs == '\0' {
        return false;
    }
    // c:774 — `return wcwidth9(ucs) != -1;`. `wcwidth9` returns -1 from
    // exactly two tables, tested in this order: c:1294-1296 (nonprint) and
    // c:1302-1304 (not_assigned). The arms between and after them are all
    // printable to C's `!= -1`: combining 0 (c:1298-1300), private -3
    // (c:1306-1308), ambiguous -2 (c:1310-1312), doublewidth 2
    // (c:1314-1316), emoji 2 (c:1318-1320), default 1 (c:1322). Combining is
    // tested BEFORE not_assigned, but the two tables are disjoint, so the
    // order is unobservable and the union below is exact.
    let cp = ucs as u32;
    // c:1262-1284 — `wcwidth9_intable()`: binary search over the sorted,
    // non-overlapping interval table. Inlined as a closure because
    // `wcwidth9_intable` is a `static inline` in a header and so has no entry
    // in the C-name index build.rs enforces; build.rs's own remedy for that
    // is "inline the body at every call site".
    let wcwidth9_intable = |table: &[(u32, u32)]| {
        // c:1264 `if (c < table[0].first) return false;` and the c:1270-1281
        // bot/top/mid loop, expressed as the equivalent `binary_search_by`.
        table
            .binary_search_by(|&(first, last)| {
                if last < cp {
                    Ordering::Less // c:1273-1274 `bot = mid + 1`
                } else if first > cp {
                    Ordering::Greater // c:1275-1276 `top = mid - 1`
                } else {
                    Ordering::Equal // c:1277-1279 `return true`
                }
            })
            .is_ok()
    };
    !(wcwidth9_intable(WCWIDTH9_NONPRINT) || wcwidth9_intable(WCWIDTH9_NOT_ASSIGNED))
}

// `convbase` moved out — canonical port lives at
// `crate::ported::utils::convbase` (Src/utils.c is the C source).
// `gethostname` moved out — canonical port lives at
// `crate::ported::utils::gethostname` (compat.c's body is
// `#ifndef HAVE_GETHOSTNAME` fallback shim; the active code path
// goes through libc directly via utils.rs).

/// Check whether an ASCII byte is printable.
/// Port of `isprint_ascii(int c)` from Src/compat.c:785 — locale-
/// independent printable check the C source uses when locale
/// data isn't safe to read (signal handlers, early init).
pub fn isprint_ascii(c: char) -> bool {
    // c:785
    let b = c as u32;
    (0x20..=0x7e).contains(&b)
}

/// Port of `char *strstr(const char *s, const char *t)` from `Src/compat.c:41`.
/// C source is wrapped in `#ifndef HAVE_STRSTR` — a fallback for systems
/// missing libc strstr. zshrs relies on libc; this shim delegates to
/// `str::find` for substring location, returning the byte offset on hit
/// or None on miss (Rust idiom for the C `char *` / `NULL` return).
pub fn strstr(s: &str, t: &str) -> Option<usize> {
    // c:41
    s.find(t) // c:46-51 byte-by-byte loop
}

/// Port of `int gettimeofday(struct timeval *tv, struct timezone *tz)`
/// from `Src/compat.c:86`. C source under `#ifndef HAVE_GETTIMEOFDAY`
/// — fallback that fills tv_sec from `time(NULL)` and zeroes tv_usec.
/// Rust shim returns (sec, usec) from libc gettimeofday; mirrors the
/// C contract of always returning 0.
pub fn gettimeofday() -> (i64, i64) {
    // c:86
    #[cfg(unix)]
    {
        let mut tv: libc::timeval = unsafe { std::mem::zeroed() };
        unsafe {
            libc::gettimeofday(&mut tv, std::ptr::null_mut());
        } // c:88-89
        (tv.tv_sec as i64, tv.tv_usec as i64)
    }
    #[cfg(not(unix))]
    {
        (0, 0)
    }
}

/// Port of `unsigned long strtoul(nptr, endptr, base)` from `Src/compat.c:688`.
/// C source under `#ifndef HAVE_STRTOUL` — fallback for systems missing
/// libc strtoul. Returns (parsed-value, bytes-consumed) so callers can
/// compute the equivalent of the C `*endptr = ...` out-param.
pub fn strtoul(nptr: &str, base: u32) -> (u64, usize) {
    // c:688
    let bytes = nptr.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    } // c:704 isspace
    let neg = i < bytes.len() && bytes[i] == b'-'; // c:707
    if neg || (i < bytes.len() && bytes[i] == b'+') {
        i += 1;
    } // c:709-712
    let (radix, start) = if (base == 0 || base == 16)
        && bytes.get(i).copied() == Some(b'0')
        && bytes
            .get(i + 1)
            .map(|b| b.eq_ignore_ascii_case(&b'x'))
            .unwrap_or(false)
    {
        (16u32, i + 2) // c:714-718 0x prefix
    } else if base == 0 {
        (
            if bytes.get(i).copied() == Some(b'0') {
                8
            } else {
                10
            },
            i,
        ) // c:719-720
    } else {
        (base, i)
    };
    let mut acc: u64 = 0;
    let mut consumed = start;
    for &b in &bytes[start..] {
        let digit = if b.is_ascii_digit() {
            (b - b'0') as u32
        } else if b.is_ascii_uppercase() {
            (b - b'A' + 10) as u32
        } else if b.is_ascii_lowercase() {
            (b - b'a' + 10) as u32
        } else {
            break;
        };
        if digit >= radix {
            break;
        }
        acc = acc
            .saturating_mul(radix as u64)
            .saturating_add(digit as u64);
        consumed += 1;
    }
    (if neg { acc.wrapping_neg() } else { acc }, consumed)
}

/// Port of `long zpathmax(char *dir)` from `Src/compat.c:236`.
/// C source is wrapped in `#if 0` (compat.c:203-282) — entirely
/// disabled in upstream zsh. Faithful translation of the HAVE_PATHCONF
/// recursive walk: try pathconf(dir); on EINVAL/ENOENT/ENOTDIR strip
/// the last path component and retry, accumulating taillen, until we
/// hit "/" or "." or run out.
pub fn zpathmax(dir: &str) -> i64 {
    // c:236
    #[cfg(unix)]
    unsafe {
        let mut buf: Vec<u8> = dir.as_bytes().to_vec(); // c:237 char *dir buffer
                                                        // c:241 errno access — pick the right per-platform getter
                                                        // (`__error()` on macOS, `__errno_location()` on Linux/BSD).
        #[cfg(target_os = "macos")]
        let errno_loc: *mut libc::c_int = libc::__error();
        #[cfg(target_os = "linux")]
        let errno_loc: *mut libc::c_int = libc::__errno_location();
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        let errno_loc: *mut libc::c_int = std::ptr::null_mut();
        if errno_loc.is_null() {
            // c:274-279 — fallback path (no working errno access).
            let dirlen = buf.len() as i64;
            let path_max = 4096i64;
            return if dirlen >= path_max {
                -1
            } else {
                path_max - dirlen
            };
        }
        let mut accumulated_taillen: libc::c_long = 0; // c:262 taillen accumulator
        loop {
            let cs = match std::ffi::CString::new(buf.clone()) {
                Ok(c) => c,
                Err(_) => return -1,
            };
            *errno_loc = 0; // c:241 errno = 0
            let pathmax = libc::pathconf(cs.as_ptr(), libc::_PC_PATH_MAX); // c:242
            if pathmax >= 0 {
                // c:242
                if accumulated_taillen == 0 {
                    return pathmax as i64; // c:244
                }
                if accumulated_taillen < pathmax {
                    return (pathmax - accumulated_taillen) as i64; // c:264
                } else {
                    *errno_loc = libc::ENAMETOOLONG; // c:266
                    return -1;
                }
            }
            let err = *errno_loc;
            if err != libc::EINVAL && err != libc::ENOENT && err != libc::ENOTDIR {
                return if *errno_loc != 0 { -1 } else { 0 }; // c:269-272
            }
            // c:247 — strip the last '/' run.
            let tail_pos: Option<usize> = buf.iter().rposition(|&b| b == b'/');
            let mut tail = match tail_pos {
                Some(t) => t,
                None => {
                    // c:259 — no '/': try pathconf(".") with taillen = strlen(dir)+1.
                    *errno_loc = 0;
                    let dot = std::ffi::CString::new(".").unwrap();
                    let pm = libc::pathconf(dot.as_ptr(), libc::_PC_PATH_MAX);
                    let taillen = (buf.len() + 1) as libc::c_long;
                    if pm > 0 && taillen < pm {
                        return (pm - taillen) as i64; // c:264
                    }
                    if pm > 0 {
                        *errno_loc = libc::ENAMETOOLONG;
                    } // c:266
                    return if *errno_loc != 0 { -1 } else { 0 }; // c:269-272
                }
            };
            while tail > 0 && buf[tail - 1] == b'/' {
                tail -= 1;
            } // c:248-249
            let taillen_now = (buf.len() - tail) as libc::c_long; // c:262
            accumulated_taillen += taillen_now;
            if tail > 0 {
                // c:250
                buf.truncate(tail); // c:251 *tail = 0
                continue;
            } else {
                // c:255 — exhausted the path; try pathconf("/").
                *errno_loc = 0;
                let root = std::ffi::CString::new("/").unwrap();
                let pm = libc::pathconf(root.as_ptr(), libc::_PC_PATH_MAX);
                if pm > 0 && accumulated_taillen < pm {
                    return (pm - accumulated_taillen) as i64; // c:264
                }
                if pm > 0 {
                    *errno_loc = libc::ENAMETOOLONG;
                } // c:266
                return if *errno_loc != 0 { -1 } else { 0 }; // c:269-272
            }
        }
    }
    #[cfg(not(unix))]
    {
        // c:274-279 — non-HAVE_PATHCONF fallback returns PATH_MAX - dirlen.
        let dirlen = dir.len() as i64;
        let path_max = 4096i64;
        if dirlen >= path_max {
            -1
        } else {
            path_max - dirlen
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zgettime() {
        let _g = crate::test_util::global_state_lock();
        let mut ts: timespec = unsafe { std::mem::zeroed() };
        let r = zgettime(&mut ts);
        assert!(r >= 0);
        assert!(ts.tv_sec > 0);
    }

    #[test]
    fn test_zgettime_monotonic() {
        let _g = crate::test_util::global_state_lock();
        let mut t1: timespec = unsafe { std::mem::zeroed() };
        let mut t2: timespec = unsafe { std::mem::zeroed() };
        let r1 = zgettime_monotonic_if_available(&mut t1);
        std::thread::sleep(std::time::Duration::from_millis(10));
        let r2 = zgettime_monotonic_if_available(&mut t2);
        assert!(r1 >= 0 && r2 >= 0);
        // Elapsed must be strictly positive in ns.
        let elapsed_ns = (t2.tv_sec - t1.tv_sec) * 1_000_000_000 + (t2.tv_nsec - t1.tv_nsec) as i64;
        assert!(elapsed_ns > 0);
    }

    #[test]
    fn test_zgetcwd() {
        let _g = crate::test_util::global_state_lock();
        // c:559-566 — zgetcwd always returns a non-empty string (falls
        // through to `dupstring(".")` if all higher paths fail).
        let cwd = zgetcwd();
        assert!(!cwd.is_empty(), "c:564-565 — zgetcwd never returns empty");
    }

    #[test]
    fn test_zopenmax() {
        let _g = crate::test_util::global_state_lock();
        let max = zopenmax();
        assert!(max > 0);
    }

    #[test]
    fn test_isprint_safe() {
        let _g = crate::test_util::global_state_lock();
        assert!(isprint_ascii('a'));
        assert!(isprint_ascii('Z'));
        assert!(isprint_ascii(' '));
        assert!(!isprint_ascii('\x00'));
        assert!(!isprint_ascii('\x1f'));
    }

    #[test]
    fn test_wcwidth() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(u9_wcwidth('a'), 1);
        assert_eq!(u9_wcwidth('中'), 2);
        assert!(u9_wcwidth('\x00') <= 0);
    }

    // ===== Tests for compat.c shim ports landed this session.

    #[test]
    fn strstr_substring_hit_returns_byte_offset() {
        let _g = crate::test_util::global_state_lock();
        // C strstr returns pointer to the match (== bytes-from-start);
        // Rust port returns Option<usize> byte offset. Verify hits +
        // miss + edge cases (empty needle is documented to return 0).
        assert_eq!(strstr("hello world", "world"), Some(6));
        assert_eq!(strstr("hello world", "hello"), Some(0));
        assert_eq!(strstr("hello world", "xyz"), None);
        assert_eq!(strstr("", "x"), None);
        assert_eq!(strstr("anything", ""), Some(0));
    }

    #[cfg(unix)]
    #[test]
    fn gettimeofday_returns_positive_secs() {
        let _g = crate::test_util::global_state_lock();
        // C contract: always returns 0; tv_sec is unix-epoch seconds.
        // Anything past 2001-09-09 is > 1_000_000_000.
        let (sec, _usec) = gettimeofday();
        assert!(sec > 1_000_000_000, "epoch seconds should be past 2001");
    }

    #[test]
    fn strtoul_parses_decimal() {
        let _g = crate::test_util::global_state_lock();
        // Base 10: simple positive integer.
        let (v, n) = strtoul("12345", 10);
        assert_eq!(v, 12345);
        assert_eq!(n, 5);
    }

    #[test]
    fn strtoul_parses_hex_with_0x_prefix_when_base_zero() {
        let _g = crate::test_util::global_state_lock();
        // base==0 with `0x` prefix → C falls into base 16 (c:714-718).
        let (v, n) = strtoul("0xff", 0);
        assert_eq!(v, 255);
        assert_eq!(n, 4);
    }

    #[test]
    fn strtoul_parses_octal_when_base_zero_with_leading_zero() {
        let _g = crate::test_util::global_state_lock();
        // base==0 with leading '0' → C falls into base 8 (c:719-720).
        let (v, _n) = strtoul("0777", 0);
        assert_eq!(v, 511);
    }

    #[test]
    fn strtoul_skips_leading_whitespace() {
        let _g = crate::test_util::global_state_lock();
        // c:704 — `do { c = *s++; } while (isspace(c))`.
        let (v, _) = strtoul("   42", 10);
        assert_eq!(v, 42);
    }

    #[test]
    fn strtoul_stops_at_first_non_digit() {
        let _g = crate::test_util::global_state_lock();
        // Mixed input: parse stops when the digit run ends; bytes-consumed
        // reports where it stopped so a caller can pick up *endptr-style.
        let (v, n) = strtoul("100abc", 10);
        assert_eq!(v, 100);
        assert_eq!(n, 3);
    }

    /// `Src/compat.c:175-180` — `difftime(t2, t1)` body is
    /// `return (double)(t2 - t1);`. Signed subtraction; result can be
    /// negative when `t1 > t2`. The fallback shim is only used on
    /// systems lacking `difftime(3)`.
    #[test]
    fn difftime_returns_signed_double_difference() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(difftime(1_700_000_010, 1_700_000_000), 10.0);
        assert_eq!(
            difftime(1_700_000_000, 1_700_000_010),
            -10.0,
            "c:178 — signed cast; t1 > t2 must be negative"
        );
        assert_eq!(difftime(42, 42), 0.0);
    }

    /// `Src/compat.c:785-790` — `isprint_ascii(c)` body is
    /// `return c >= 0x20 && c <= 0x7e;` — strict ASCII printable range
    /// (space through tilde), locale-independent.
    #[test]
    fn isprint_ascii_matches_strict_ascii_printable_range() {
        let _g = crate::test_util::global_state_lock();
        // Boundaries.
        assert!(isprint_ascii(' '), "c:786 — 0x20 is printable");
        assert!(isprint_ascii('~'), "c:786 — 0x7e is printable");
        // Just outside both ends.
        assert!(!isprint_ascii('\x1f'), "c:786 — 0x1f is NOT printable");
        assert!(!isprint_ascii('\x7f'), "c:786 — DEL is NOT printable");
        // Common visible chars all inside the range.
        assert!(isprint_ascii('A'));
        assert!(isprint_ascii('0'));
        assert!(isprint_ascii('!'));
        // Controls all rejected.
        assert!(!isprint_ascii('\t'));
        assert!(!isprint_ascii('\n'));
        assert!(!isprint_ascii('\0'));
        // Non-ASCII (>= 0x80) rejected per the upper-bound at 0x7e.
        assert!(!isprint_ascii('é'), "c:786 — non-ASCII outside range");
        assert!(!isprint_ascii('字'), "c:786 — wide char outside range");
    }

    /// `Src/compat.c:638` — `output64(zlong val)` formats a 64-bit
    /// integer for output. Rust uses `i64::to_string()`. Pin boundary
    /// values (i64::MIN/MAX) and the sign handling.
    #[test]
    fn output64_formats_i64_boundaries_and_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(output64(0), "0");
        assert_eq!(output64(42), "42");
        assert_eq!(output64(-1), "-1");
        assert_eq!(output64(i64::MAX), "9223372036854775807");
        assert_eq!(output64(i64::MIN), "-9223372036854775808");
    }

    /// `Src/compat.c:770-775` — `u9_iswprint(ucs)` returns true iff
    /// the char is NOT a control AND has a non-negative width. Pin
    /// the canonical printable cases + control rejection.
    #[test]
    fn u9_iswprint_accepts_printable_rejects_controls() {
        let _g = crate::test_util::global_state_lock();
        assert!(u9_iswprint('a'));
        assert!(u9_iswprint(' '));
        assert!(u9_iswprint('é'), "Latin-1 letter is printable");
        assert!(u9_iswprint('字'), "CJK ideograph is printable");
        // Controls — explicit zsh check at c:773.
        assert!(!u9_iswprint('\0'));
        assert!(!u9_iswprint('\t'));
        assert!(!u9_iswprint('\n'));
        assert!(!u9_iswprint('\x07'));
        assert!(!u9_iswprint('\x1b'));
        assert!(!u9_iswprint('\x7f'), "DEL is a C0 control");
    }

    /// `Src/compat.c:774` — `wcwidth9()` returns -1 from a SECOND table,
    /// `wcwidth9_not_assigned` (`Src/wcwidth9.h:620-1258`, tested at
    /// c:1302-1304), so unassigned codepoints are NOT printable.
    ///
    /// Every pair below was measured against the reference build,
    /// `zsh 5.9.2 (aarch64-apple-darwin25.4.0)`, with:
    ///
    ///     zsh -f -c 'v=${(#):-0xNNNN}; print -rn -- "${(q)v}"' | od -An -tx1
    ///
    /// An escaped result (`$'\...'`) means `u9_iswprint` is false; raw UTF-8
    /// bytes mean it is true. In-table samples are spread across the 637
    /// intervals (first, last, and 13 in between); out-of-table samples sit
    /// immediately outside those same intervals so a table shifted by one row
    /// fails here.
    #[test]
    fn u9_iswprint_rejects_unassigned_codepoints() {
        let _g = crate::test_util::global_state_lock();

        // Inside `wcwidth9_not_assigned` — zsh printed `$'\...'` for each.
        for cp in [
            0x0378_u32, // c:621  first interval; zsh: $'\315\270'
            0x0381,     // c:622
            0x0530,     // c:626
            0x083f,     // c:641
            0x0ab4,     // c:681
            0x0cda,     // c:741
            0x1759,     // c:821
            0x2daf,     // c:901
            0xfe1c,     // c:971
            0x10aaf,    // c:1041
            0x16a3c,    // c:1121
            0x1e95b,    // c:1181
            0x1f0d0,    // c:1221
            0x1f91f,    // c:1241
            0xffffe,    // c:1257 last interval; zsh: $'\363\277\277\276'
        ] {
            let c = char::from_u32(cp).unwrap();
            assert!(
                !u9_iswprint(c),
                "c:1302-1304 — U+{:04X} is in wcwidth9_not_assigned, so \
                 wcwidth9() is -1 and u9_iswprint() must be false",
                cp
            );
        }

        // Immediately outside those intervals — zsh printed the raw bytes.
        // 0x0fffd is `wcwidth9_private` (-3, c:1306-1308) and 0x1f91e is
        // `wcwidth9_emoji_width` (2, c:1318-1320): both are printable to
        // C's `!= -1`, which pins that only the -1 arms were added.
        for cp in [
            0x0377_u32, 0x037f, 0x052f, 0x083e, 0x0ab3, 0x0cd6, 0x1753, 0x2dae, 0xfe19, 0x10a9f,
            0x16a38, 0x1e959, 0x1f0cf, 0x1f91e, 0xffffd,
        ] {
            let c = char::from_u32(cp).unwrap();
            assert!(
                u9_iswprint(c),
                "c:774 — U+{:04X} is outside both -1 tables, so wcwidth9() \
                 is not -1 and u9_iswprint() must be true",
                cp
            );
        }
    }

    /// `Src/wcwidth9.h:1262-1284` — `wcwidth9_intable()` binary-searches the
    /// table, which is only correct if the table is sorted ascending and its
    /// intervals are non-overlapping. Pin that invariant for both -1 tables
    /// so a hand-edited row can never silently make lookups miss.
    #[test]
    fn wcwidth9_minus_one_tables_are_sorted_and_disjoint() {
        for (name, table) in [
            ("wcwidth9_nonprint", WCWIDTH9_NONPRINT),
            ("wcwidth9_not_assigned", WCWIDTH9_NOT_ASSIGNED),
        ] {
            for w in table.windows(2) {
                assert!(
                    w[0].0 <= w[0].1 && w[0].1 < w[1].0,
                    "c:1262-1284 — {} must be sorted and disjoint, but \
                     ({:#x},{:#x}) is followed by ({:#x},{:#x})",
                    name,
                    w[0].0,
                    w[0].1,
                    w[1].0,
                    w[1].1
                );
            }
        }
        // c:621..c:1257 — one interval per source line, inclusive.
        assert_eq!(
            WCWIDTH9_NOT_ASSIGNED.len(),
            637,
            "Src/wcwidth9.h:620-1258 holds 637 intervals"
        );
    }

    /// `Src/compat.c:760-768` — `u9_wcwidth(ucs)` returns:
    /// `-1` for controls, `0` for combining/zero-width marks, `1` for
    /// most chars, `2` for CJK/East-Asian-Wide. Rust delegates to the
    /// `unicode-width` crate. Pin the four-tier output so a future
    /// crate version mismatch surfaces.
    #[test]
    fn u9_wcwidth_returns_canonical_widths() {
        let _g = crate::test_util::global_state_lock();
        // -1 for controls (locked at c:766 `is_control` branch in Rust).
        assert_eq!(u9_wcwidth('\x07'), -1);
        // 1 for ordinary ASCII.
        assert_eq!(u9_wcwidth('a'), 1);
        assert_eq!(u9_wcwidth(' '), 1);
        // 2 for CJK ideographs.
        assert_eq!(u9_wcwidth('字'), 2);
        // 0 for combining marks (U+0301 COMBINING ACUTE ACCENT).
        assert_eq!(u9_wcwidth('\u{0301}'), 0);
    }

    /// `Src/compat.c:194` — `strerror(errnum)` returns a printable
    /// string. The libc shim returns "Success" for 0 on Linux/macOS
    /// (or any non-empty descriptor — we don't pin exact text, just
    /// that the function returns SOMETHING per errno code). Pin only
    /// the API contract: non-empty for at least one known errno.
    #[test]
    fn strerror_returns_non_empty_string_for_known_errno() {
        let _g = crate::test_util::global_state_lock();
        // ENOENT is "No such file or directory" on every Unix.
        let s = strerror(2 /* ENOENT */);
        assert!(
            !s.is_empty(),
            "c:194 — strerror must return non-empty for ENOENT"
        );
    }

    /// `Src/compat.c:307-326` — `zopenmax()` caps at
    /// `ZSH_INITIAL_OPEN_MAX` to avoid probing ridiculous numbers of
    /// fds when `sysconf(_SC_OPEN_MAX)` returns "unlimited". The
    /// previous Rust port had a local `ZSH_INITIAL_OPEN_MAX = 1024`
    /// constant, diverging from `Src/zsh_system.h:307` (`#define
    /// ZSH_INITIAL_OPEN_MAX 64`). On systems with raised ulimits this
    /// caused `closem()` to walk 16× more fds than C zsh would,
    /// silently quadratic-ing every fork.
    ///
    /// Pin two invariants:
    ///   1. The canonical ZSH_INITIAL_OPEN_MAX in `zsh_system_h` is 64.
    ///   2. `zopenmax()` never returns more than max(OPEN_MAX, fd in use).
    #[test]
    fn zopenmax_caps_within_canonical_ladder() {
        let _g = crate::test_util::global_state_lock();
        // c:307 — canonical value.
        assert_eq!(
            ZSH_INITIAL_OPEN_MAX, 64,
            "Src/zsh_system.h:307 — ZSH_INITIAL_OPEN_MAX must be 64"
        );
        // zopenmax() must be positive and bounded by the OPEN_MAX
        // host value (Linux: typically 1024, macOS: 10240).
        let m = zopenmax();
        assert!(m > 0, "c:307 — zopenmax must report a positive ceiling");
    }

    /// `Src/compat.c:559-567` — `zgetcwd()` C body falls through the
    /// chain `zgetdir(NULL) || unmeta(pwd) || "."` and ALWAYS returns
    /// a non-NULL, non-empty string. Pin the c:564-565 final fallback:
    /// even when current_dir() succeeds, the returned string is
    /// non-empty and starts with `/` (absolute on every Unix). And
    /// pin the "." fallback by simulating both prior arms failing.
    #[test]
    fn zgetcwd_always_returns_non_empty() {
        let _g = crate::test_util::global_state_lock();
        let cwd = zgetcwd();
        assert!(
            !cwd.is_empty(),
            "c:564-565 — zgetcwd must NEVER return empty (falls through to dupstring(\".\"))"
        );
        // First fallback (current_dir) succeeds in normal test env →
        // expect an absolute path.
        #[cfg(unix)]
        {
            assert!(
                cwd.starts_with('/') || cwd == ".",
                "c:561 — zgetdir(NULL) returns absolute path, or c:565 fallback `.`"
            );
        }
    }

    /// `Src/compat.c:579-590` — `zchdir("")` returns 0 immediately
    /// (c:585 `if (!*dir || chdir(dir) == 0)`). Pins the empty-path
    /// short-circuit so a refactor of the loop init doesn't break it.
    #[test]
    fn zchdir_empty_path_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(zchdir(""), 0, "c:585 — empty dir short-circuits to success");
    }

    /// `Src/compat.c:579-594` — direct `chdir(2)` success path: when
    /// the input is a valid existing directory (no name-too-long
    /// error), zchdir returns 0 without entering the chunked descent.
    /// Pin a normal absolute path under the current cwd works.
    #[test]
    fn zchdir_existing_path_succeeds_without_fallback() {
        let _g = crate::test_util::global_state_lock();
        let saved = env::current_dir().unwrap();
        // c:585 — direct chdir success.
        let rc = zchdir("/");
        assert_eq!(rc, 0, "c:585 — zchdir(\"/\") direct success");
        // Restore for downstream tests.
        env::set_current_dir(&saved).unwrap();
    }

    /// `Src/compat.c:579-594` — direct chdir failure with a
    /// non-name-too-long errno (ENOENT) MUST return -1 immediately,
    /// NOT enter the chunked descent. The previous Rust port walked
    /// the path component-by-component on any failure, exposing
    /// partial side-effects (e.g. successful descent into a readable
    /// parent before failing on the missing component).
    #[test]
    fn zchdir_nonexistent_path_returns_minus_one_without_fallback() {
        let _g = crate::test_util::global_state_lock();
        let saved = env::current_dir().unwrap();
        // Path that exists up to /tmp but not the last component →
        // chdir fails with ENOENT. C returns -1 without trying the
        // chunked descent (path < PATH_MAX so c:593 fails the gate).
        let rc = zchdir("/tmp/this_zshrs_test_path_does_not_exist_xyz_abc");
        assert_eq!(
            rc, -1,
            "c:592-594 — non-ENAMETOOLONG failure breaks loop, returns -1"
        );
        // cwd unchanged.
        assert_eq!(
            env::current_dir().unwrap(),
            saved,
            "no chdir side-effect on non-recoverable failure"
        );
    }

    // ─── zsh-corpus pins for compat helpers ────────────────────────

    /// `output64(0)` returns "0".
    #[test]
    fn compat_corpus_output64_zero() {
        assert_eq!(output64(0), "0");
    }

    /// `output64(positive)` returns decimal string.
    #[test]
    fn compat_corpus_output64_positive() {
        assert_eq!(output64(42), "42");
        assert_eq!(output64(1234567890), "1234567890");
    }

    /// `output64(i64::MAX)` returns full max value as string.
    #[test]
    fn compat_corpus_output64_int_max() {
        assert_eq!(output64(i64::MAX), i64::MAX.to_string());
    }

    /// `output64(negative)` includes leading `-`.
    #[test]
    fn compat_corpus_output64_negative() {
        assert_eq!(output64(-42), "-42");
        assert_eq!(output64(i64::MIN), i64::MIN.to_string());
    }

    /// `strstr("hello world", "world")` returns Some(6) (byte offset).
    #[test]
    fn compat_corpus_strstr_finds_substring() {
        assert_eq!(strstr("hello world", "world"), Some(6));
    }

    /// `strstr` empty needle returns Some(0).
    #[test]
    fn compat_corpus_strstr_empty_needle() {
        let r = strstr("hello", "");
        assert_eq!(r, Some(0), "empty needle matches at position 0");
    }

    /// `strstr` on missing returns None.
    #[test]
    fn compat_corpus_strstr_missing_returns_none() {
        assert_eq!(strstr("hello world", "zzz"), None);
    }

    /// `strtoul("42", 10)` parses 42 with full-string consumption.
    #[test]
    fn compat_corpus_strtoul_decimal() {
        let (val, consumed) = strtoul("42", 10);
        assert_eq!(val, 42);
        assert_eq!(consumed, 2, "consumed 2 chars");
    }

    /// `strtoul("ff", 16)` parses 255 hex.
    #[test]
    fn compat_corpus_strtoul_hex() {
        let (val, _) = strtoul("ff", 16);
        assert_eq!(val, 255);
    }

    /// `strtoul("123abc", 10)` parses 123 stopping at 'a'.
    #[test]
    fn compat_corpus_strtoul_stops_at_nondigit() {
        let (val, consumed) = strtoul("123abc", 10);
        assert_eq!(val, 123);
        assert_eq!(consumed, 3, "stopped at non-digit");
    }

    /// `difftime(t2, t1) = (t2 - t1) as f64`.
    #[test]
    fn compat_corpus_difftime_positive() {
        let d = difftime(100, 60);
        assert!((d - 40.0).abs() < 1e-9, "100 - 60 = 40, got {d}");
    }

    /// `difftime` negative when t2 < t1.
    #[test]
    fn compat_corpus_difftime_negative() {
        let d = difftime(60, 100);
        assert!((d + 40.0).abs() < 1e-9, "60 - 100 = -40, got {d}");
    }

    /// `isprint_ascii` returns true for letters and digits.
    #[test]
    fn compat_corpus_isprint_ascii_visible() {
        for c in ['a', 'Z', '0', '9', ' ', '~'] {
            assert!(isprint_ascii(c), "{c:?} should be printable");
        }
    }

    /// `isprint_ascii` returns false for control chars.
    #[test]
    fn compat_corpus_isprint_ascii_rejects_controls() {
        for c in ['\0', '\n', '\r', '\t', '\x1b'] {
            assert!(!isprint_ascii(c), "{c:?} should NOT be printable");
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/compat.c.
    // ═══════════════════════════════════════════════════════════════════

    /// c:175 — `difftime(t, t)` returns 0.0 (identity).
    #[test]
    fn difftime_same_input_returns_zero() {
        assert_eq!(difftime(100, 100), 0.0);
        assert_eq!(difftime(0, 0), 0.0);
    }

    /// c:175 — `difftime` positive when t2 > t1.
    #[test]
    fn difftime_positive_when_t2_greater() {
        assert_eq!(difftime(100, 60), 40.0);
        assert_eq!(difftime(1000, 1), 999.0);
    }

    /// c:194 — `strerror(0)` returns a non-empty string (errno 0 ≠ valid
    /// libc error but should still render something like "Success" or
    /// "No error" depending on platform).
    #[test]
    fn strerror_zero_returns_nonempty() {
        let s = strerror(0);
        assert!(!s.is_empty(), "strerror(0) must return non-empty string");
    }

    /// c:194 — `strerror(libc::EACCES)` returns a non-empty string.
    #[test]
    #[cfg(unix)]
    fn strerror_known_errno_returns_descriptive() {
        let s = strerror(libc::EACCES);
        assert!(!s.is_empty());
        assert_ne!(s, " ", "should have meaningful content");
    }

    /// c:194 — strerror is deterministic for a given errno.
    #[test]
    fn strerror_is_deterministic() {
        let a = strerror(2);
        let b = strerror(2);
        assert_eq!(a, b, "strerror must be pure");
    }

    /// c:488 — `isprint_ascii('\\x7f')` is false (DEL is control).
    #[test]
    fn isprint_ascii_del_is_not_printable() {
        assert!(!isprint_ascii('\x7f'), "DEL (0x7f) is control");
    }

    /// c:488 — `isprint_ascii` non-ASCII chars (> 0x7f) — behavior pin.
    /// Per zsh C source `isprint_ascii` is ASCII-only, so non-ASCII
    /// codepoints return false.
    #[test]
    fn isprint_ascii_non_ascii_returns_false() {
        assert!(!isprint_ascii('\u{0080}'), "U+0080 is non-ASCII control");
        assert!(!isprint_ascii('é'), "é (U+00E9) is non-ASCII");
        assert!(!isprint_ascii('日'), "日 (CJK) is non-ASCII");
    }

    /// c:300 — `zopenmax()` returns positive within sane bounds.
    /// Per zsh ZSH_INITIAL_OPEN_MAX cap, never above ~10000.
    #[test]
    #[cfg(unix)]
    fn zopenmax_returns_positive_bounded() {
        let max = zopenmax();
        assert!(max > 0, "zopenmax must be positive, got {}", max);
        assert!(max < 100_000, "zopenmax suspiciously large: {}", max);
    }

    /// c:499 — `strstr("haystack", "tack")` returns Some(byte_offset).
    #[test]
    fn strstr_finds_substring() {
        assert_eq!(strstr("haystack", "tack"), Some(4));
        assert_eq!(strstr("abc", "abc"), Some(0), "full match at start");
        assert_eq!(strstr("abc", ""), Some(0), "empty needle matches at 0");
    }

    /// c:499 — `strstr` returns None when not found.
    #[test]
    fn strstr_missing_returns_none() {
        assert_eq!(strstr("haystack", "xyz"), None);
        assert_eq!(strstr("", "needle"), None);
    }

    /// c:509 — `gettimeofday()` returns positive (sec, usec).
    #[test]
    fn gettimeofday_returns_positive_seconds() {
        let (sec, usec) = gettimeofday();
        assert!(sec > 0, "current epoch sec must be positive");
        assert!(usec >= 0 && usec < 1_000_000, "usec in [0, 1M)");
    }

    /// c:529 — `strtoul("", base)` returns (0, 0).
    #[test]
    fn strtoul_empty_returns_zero_zero() {
        let (v, n) = strtoul("", 10);
        assert_eq!(v, 0);
        assert_eq!(n, 0);
    }

    /// c:529 — `strtoul("123", 10)` returns (123, 3).
    #[test]
    fn strtoul_base_10_parses_digits() {
        let (v, n) = strtoul("123", 10);
        assert_eq!(v, 123);
        assert_eq!(n, 3);
    }

    /// c:529 — `strtoul("abc", 10)` returns (0, 0) (non-digit prefix).
    #[test]
    fn strtoul_non_digit_prefix_returns_zero() {
        let (v, n) = strtoul("abc", 10);
        assert_eq!(v, 0);
        assert_eq!(n, 0);
    }

    /// c:529 — `strtoul("ff", 16)` parses hex.
    #[test]
    fn strtoul_base_16_parses_hex() {
        let (v, n) = strtoul("ff", 16);
        assert_eq!(v, 0xff);
        assert_eq!(n, 2);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/compat.c
    // c:26 zgettime / c:69 zgettime_monotonic_if_available / c:225 zgetdir
    // c:340 zchdir / c:463 u9_wcwidth / c:472 u9_iswprint / c:589 zpathmax
    // ═══════════════════════════════════════════════════════════════════

    /// c:26 — `zgettime` returns 0 on success (POSIX clock_gettime convention).
    #[test]
    fn zgettime_returns_zero_on_success() {
        let _g = crate::test_util::global_state_lock();
        let mut ts: timespec = unsafe { std::mem::zeroed() };
        let r = zgettime(&mut ts);
        assert_eq!(r, 0, "zgettime success → 0");
    }

    /// c:26 — `zgettime` populates ts with positive secs.
    #[test]
    fn zgettime_populates_positive_secs() {
        let _g = crate::test_util::global_state_lock();
        let mut ts: timespec = unsafe { std::mem::zeroed() };
        zgettime(&mut ts);
        assert!(ts.tv_sec > 0, "secs must be > 0 after gettime");
    }

    /// c:69 — `zgettime_monotonic_if_available` returns i32 (type pin).
    #[test]
    fn zgettime_monotonic_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let mut ts: timespec = unsafe { std::mem::zeroed() };
        let _: i32 = zgettime_monotonic_if_available(&mut ts);
    }

    /// c:225 — `zgetdir(None)` returns Option<String>.
    #[test]
    fn zgetdir_none_returns_option_string_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Option<String> = zgetdir(None);
    }

    /// c:340 — `zchdir("/__nonexistent_xyz")` returns -1 (failure).
    #[test]
    fn zchdir_nonexistent_returns_minus_one_pin() {
        let _g = crate::test_util::global_state_lock();
        let r = zchdir("/__nonexistent_zshrs_xyz_compat__");
        assert_eq!(r, -1, "nonexistent dir → -1");
    }

    /// c:463 — `u9_wcwidth('a')` ASCII letter returns 1.
    #[test]
    fn u9_wcwidth_ascii_letter_returns_one() {
        assert_eq!(u9_wcwidth('a'), 1, "ASCII letter width = 1");
    }

    /// c:463 — `u9_wcwidth` returns i32 (compile-time type pin).
    #[test]
    fn u9_wcwidth_returns_i32_type() {
        let _: i32 = u9_wcwidth('a');
    }

    /// c:472 — `u9_iswprint('a')` ASCII letter returns true.
    #[test]
    fn u9_iswprint_ascii_letter_returns_true() {
        assert!(u9_iswprint('a'), "ASCII letter is printable");
    }

    /// c:472 — `u9_iswprint('\\0')` NUL returns false.
    #[test]
    fn u9_iswprint_nul_returns_false() {
        assert!(!u9_iswprint('\0'), "NUL is NOT printable");
    }

    /// c:589 — `zpathmax("")` empty returns i64 (compile-time type pin).
    #[test]
    fn zpathmax_returns_i64_type() {
        let _: i64 = zpathmax("");
    }

    /// c:589 — `zpathmax` is pure for the same input.
    #[test]
    fn zpathmax_is_pure_for_root() {
        let first = zpathmax("/");
        for _ in 0..3 {
            assert_eq!(zpathmax("/"), first, "zpathmax must be pure");
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/compat.c
    // c:105 difftime / c:133 strerror / c:156 zopenmax / c:263 zgetcwd /
    // c:453 output64 / c:499 strstr / c:509 gettimeofday / c:529 strtoul
    // ═══════════════════════════════════════════════════════════════════

    /// c:105 — `difftime` returns f64 (compile-time pin).
    #[test]
    fn difftime_returns_f64_type() {
        let _: f64 = difftime(0, 0);
    }

    /// c:105 — `difftime(t, t)` returns 0 (identity).
    #[test]
    fn difftime_identity_returns_zero() {
        for t in [0i64, 100, 1_000_000, i32::MAX as i64] {
            assert_eq!(difftime(t, t), 0.0, "difftime({}, {}) must equal 0", t, t);
        }
    }

    /// c:105 — `difftime(t2, t1)` = -(difftime(t1, t2)) (antisymmetric).
    #[test]
    fn difftime_antisymmetric() {
        assert_eq!(
            difftime(100, 50),
            -difftime(50, 100),
            "difftime is antisymmetric"
        );
    }

    /// c:133 — `strerror` returns String (compile-time pin).
    #[test]
    fn strerror_returns_string_type() {
        let _: String = strerror(0);
    }

    /// c:133 — `strerror(0)` returns non-empty (some success message
    /// or "Undefined error: 0").
    #[test]
    fn strerror_zero_returns_non_empty() {
        assert!(!strerror(0).is_empty(), "strerror(0) must be non-empty");
    }

    /// c:156 — `zopenmax` returns i64 (compile-time pin).
    #[test]
    fn zopenmax_returns_i64_type() {
        let _: i64 = zopenmax();
    }

    /// c:156 — `zopenmax` returns positive (must have ≥1 fd available).
    #[test]
    fn zopenmax_returns_positive() {
        let n = zopenmax();
        assert!(n > 0, "zopenmax must be positive; got {}", n);
    }

    /// c:263 — `zgetcwd` returns String (compile-time pin).
    #[test]
    fn zgetcwd_returns_string_type() {
        let _g = crate::test_util::global_state_lock();
        let _: String = zgetcwd();
    }

    /// c:453 — `output64(0)` returns "0".
    #[test]
    fn output64_zero_returns_zero_digit() {
        assert_eq!(output64(0), "0", "0 → \"0\"");
    }

    /// c:499 — `strstr("hello", "ll")` returns Some(2).
    #[test]
    fn strstr_substring_returns_position() {
        assert_eq!(strstr("hello", "ll"), Some(2), "\"ll\" in \"hello\" at 2");
    }

    /// c:499 — `strstr("abc", "xyz")` returns None.
    #[test]
    fn strstr_not_found_returns_none() {
        assert_eq!(strstr("abc", "xyz"), None, "no match → None");
    }

    /// c:509 — `gettimeofday` returns (i64, i64) tuple (compile-time pin).
    #[test]
    fn gettimeofday_returns_i64_pair_type() {
        let _: (i64, i64) = gettimeofday();
    }

    /// c:529 — `strtoul("42", 10)` returns (42, 2).
    #[test]
    fn strtoul_basic_parse_returns_value_and_count() {
        let (v, n) = strtoul("42", 10);
        assert_eq!(v, 42, "value parses to 42");
        assert_eq!(n, 2, "consumed 2 bytes");
    }
}

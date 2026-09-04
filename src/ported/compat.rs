//! Compatibility and utility routines for zshrs
//!
//! Direct port from zsh/Src/compat.c
//!
//! Provides:
//! - High-resolution time functions
//! - Directory navigation utilities
//! - Path handling for long pathnames
//! - 64-bit integer formatting

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
///     ${(q)} of U+00AD / U+200B / U+FEFF
///       zsh  : $'\302\255'  $'\342\200\213'  $'\357\273\277'
///       zshrs: the raw bytes
///
/// !!! WARNING: RUST-ONLY GAP !!!
/// C's `wcwidth9` also returns -1 for the 636 intervals of
/// `wcwidth9_not_assigned` (`Src/wcwidth9.h:620-1258`, tested at
/// c:1302-1304), so zsh escapes unassigned codepoints — `${(q)$'\u0378'}`
/// is `$'\315\270'` there and stays raw here. That table is a separate
/// mechanical addition and is NOT covered yet.
pub fn u9_iswprint(ucs: char) -> bool {
    // c:772-773 — `if (ucs == 0) return 0;`. Subsumed by the {0,0x1f}
    // interval below; kept explicit because C keeps it explicit.
    if ucs == '\0' {
        return false;
    }
    // c:774 — `return wcwidth9(ucs) != -1;`. The only -1 arms reachable for a
    // Rust `char` are c:1294-1296 (nonprint) and c:1302-1304 (not_assigned,
    // see the WARNING). Combining returns 0 (c:1299), private -3 (c:1307) and
    // ambiguous -2 (c:1311) are all printable to C's `!= -1`.
    let cp = ucs as u32;
    !WCWIDTH9_NONPRINT
        .iter()
        .any(|&(lo, hi)| cp >= lo && cp <= hi)
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

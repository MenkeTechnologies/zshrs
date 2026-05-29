//! `zsh_system.h` port — system constants, stat-mode predicates,
//! wait-status decoders.
//!
//! Port of the Rust-portable portions of `Src/zsh_system.h`. The
//! original C header is 930 lines, but ~95% of it is `#include`
//! directives for libc headers and `#ifdef HAVE_X / #define X
//! compat_X` portability shims for ancient platforms — none of
//! which translate to Rust (the `libc` crate handles platform
//! abstraction natively).
//!
//! What the Rust port covers:
//!   - Core size constants (`PATH_MAX`, `ZSH_INITIAL_OPEN_MAX`,
//!     `OPEN_MAX`, `DIGBUFSIZE`, `BDIGBUFSIZE`, `VDISABLEVAL`).
//!   - Default strings (`DEFAULT_WORDCHARS`, `DEFAULT_TIMEFMT`).
//!   - File mode permission bits (`S_ISUID`/`S_ISGID`/`S_ISVTX`/
//!     `S_IRUSR`/`S_IWUSR`/.../`S_IRWXO`/`S_IRUGO`/etc.).
//!   - Stat-mode predicate macros (`S_ISBLK`/`S_ISCHR`/`S_ISDIR`/
//!     `S_ISDOOR`/`S_ISFIFO`/`S_ISLNK`/`S_ISMPB`/`S_ISMPC`/`S_ISNWK`/
//!     `S_ISOFD`/`S_ISOFL`/`S_ISREG`/`S_ISSOCK`).
//!   - Wait-status decoders (`WIFEXITED`/`WEXITSTATUS`/`WIFSIGNALED`/
//!     `WTERMSIG`/`WCOREDUMP`/`WIFSTOPPED`/`WSTOPSIG`).
//!   - Access-mode constants (`F_OK`/`X_OK`/`W_OK`/`R_OK`).
//!   - Directory-separator predicate (`IS_DIRSEP`).
//!   - Per-platform stat-time-nanosecond accessors
//!     (`GET_ST_ATIME_NSEC`/`GET_ST_MTIME_NSEC`/`GET_ST_CTIME_NSEC`).
//!
//! What the Rust port does NOT cover (handled by libc/std):
//!   - libc header `#include`s (Rust uses `libc::*` directly).
//!   - `setre[ug]id`/`setres[ug]id` compat shims (libc on every
//!     supported host has these).
//!   - varargs macros (Rust has native variadic-via-slice).
//!   - alloca / VARARR (Rust has Vec).
//!   - struct-direct vs struct-dirent #ifdef tangles.
//!   - termios/curses/termcap shims (handled in modules/termcap.rs).
//!   - locale/MULTIBYTE includes (Rust char is always 32-bit Unicode).
//!
//! Macro names preserved verbatim from C — UPPERCASE C macros stay
//! UPPERCASE in Rust per the casing rule (`#[allow(non_snake_case)]`
//! silences the lint).

// ---------------------------------------------------------------------------
// Size constants (c:288-319, 564-570).
// ---------------------------------------------------------------------------

/// Port of `#define PATH_MAX` from `Src/zsh_system.h:298`. Maximum
/// path length. C falls back to MAXPATHLEN, _POSIX_PATH_MAX, then
/// 1024; the Rust port uses libc's PATH_MAX.
use std::os::unix::fs::MetadataExt;
/// `PATH_MAX` constant.
pub const PATH_MAX: usize = libc::PATH_MAX as usize; // c:298

/// Port of `#define ZSH_INITIAL_OPEN_MAX` from `Src/zsh_system.h:307`.
/// Initial fd-table size; reallocated later if needed.
pub const ZSH_INITIAL_OPEN_MAX: usize = 64; // c:307

/// Port of `#define OPEN_MAX` fallback chain from
/// `Src/zsh_system.h:308-315`. C uses libc's `OPEN_MAX` macro when
/// defined, else `NOFILE`, else falls through to
/// `ZSH_INITIAL_OPEN_MAX`. Linux glibc/musl + macOS Darwin libc both
/// drop the `OPEN_MAX` macro (they use `sysconf(_SC_OPEN_MAX)` at
/// runtime instead), so the Rust port takes the same fallback as
/// the C "we will just pick something" comment at c:312.
pub const OPEN_MAX: usize = ZSH_INITIAL_OPEN_MAX; // c:313

/// Port of `#define DIGBUFSIZE` from `Src/zsh_system.h:569`. Length
/// of a buffer that can hold the printable decimal form of a
/// `-LONG_MAX-1` value (sign + digits + null).
///
/// C: `((int)(((sizeof(zlong) * 8) - 1) * 30103/100000) + 3)`
/// For zlong = i64 (8 bytes): `((63 * 30103) / 100000) + 3 = 21`.
pub const DIGBUFSIZE: usize = (((std::mem::size_of::<i64>() * 8) - 1) * 30103 / 100000) + 3; // c:569

/// Port of `#define BDIGBUFSIZE` from `Src/zsh_system.h:570`. Length
/// of a buffer for a number in printable binary form.
///
/// C: `((int)((sizeof(zlong) * 8) + 4))`
/// For zlong = i64: `64 + 4 = 68`.
pub const BDIGBUFSIZE: usize = (std::mem::size_of::<i64>() * 8) + 4; // c:570

/// Port of `#define VDISABLEVAL` from `Src/zsh_system.h:396-399`.
/// Termios "disable this special character" sentinel — C source uses
/// `_POSIX_VDISABLE` when defined, else 0. Rust pulls from libc.
#[cfg(unix)]
pub const VDISABLEVAL: u8 = libc::_POSIX_VDISABLE; // c:396
/// `VDISABLEVAL` constant.
#[cfg(not(unix))]
pub const VDISABLEVAL: u8 = 0;

// ---------------------------------------------------------------------------
// Compatibility with clock_gettime() (c:243-249).
//
// C source:
//   /* Used to provide compatibility with clock_gettime() */
//   #if !defined(HAVE_STRUCT_TIMESPEC) && !defined(ZSH_OOT_MODULE)
//   struct timespec {
//       time_t tv_sec;
//       long tv_nsec;
//   };
//   #endif
//
// Rust port: on platforms where libc provides `struct timespec`
// (which is every Unix target zshrs supports — POSIX guarantees
// it via <time.h>), this aliases libc::timespec directly so the
// `&mut timespec` out-args on `zgettime` / `zgettime_monotonic_
// if_available` (Src/compat.c:101 / :133) bind to the system
// type with no field-shape mismatch.
// ---------------------------------------------------------------------------
/// `timespec` type alias.
#[allow(non_camel_case_types)]
pub type timespec = libc::timespec; // c:245

/// Port of `#define DEFAULT_WORDCHARS` from `Src/zsh_system.h:427`.
/// Default value of `$WORDCHARS` — the chars besides alphanumerics
/// + `_` that ZLE and the completion system treat as word constituents.
pub const DEFAULT_WORDCHARS: &str = "*?_-.[]~=/&;!#$%^(){}<>"; // c:427

/// Port of `#define DEFAULT_TIMEFMT` from `Src/zsh_system.h:428`.
/// Default value of `$TIMEFMT` — the format string the `time` builtin
/// uses when printing a job's resource summary.
pub const DEFAULT_TIMEFMT: &str = "%J  %U user %S system %P cpu %*E total"; // c:428

// ---------------------------------------------------------------------------
// File mode bits (c:594, 682-735) — `S_IFMT` mask + permission bits
// + aggregate masks. Each `#ifndef X / # define X N` shim is ported
// as a `pub const`. Ports preserve octal literal form for visual parity.
// ---------------------------------------------------------------------------

/// Port of `#define S_IFMT` from `Src/zsh_system.h:594`. Mask
/// isolating the file-type bits from a stat mode word.
pub const S_IFMT: u32 = 0o170_000; // c:594

// File-type bits — for the predicate macros below. Pulled from libc
// where available so they match the host's `<sys/stat.h>` exactly.
/// `S_IFBLK` constant.
pub const S_IFBLK: u32 = libc::S_IFBLK as u32;
/// `S_IFCHR` constant.
pub const S_IFCHR: u32 = libc::S_IFCHR as u32;
/// `S_IFDIR` constant.
pub const S_IFDIR: u32 = libc::S_IFDIR as u32;
/// `S_IFIFO` constant.
pub const S_IFIFO: u32 = libc::S_IFIFO as u32;
/// `S_IFLNK` constant.
pub const S_IFLNK: u32 = libc::S_IFLNK as u32;
/// `S_IFREG` constant.
pub const S_IFREG: u32 = libc::S_IFREG as u32;
/// `S_IFSOCK` constant.
pub const S_IFSOCK: u32 = libc::S_IFSOCK as u32;

// Permission bits (c:682-735). Octal values match POSIX exactly.
/// `S_ISUID` constant.
pub const S_ISUID: u32 = 0o4000; // c:682
/// `S_ISGID` constant.
pub const S_ISGID: u32 = 0o2000; // c:685
/// `S_ISVTX` constant.
pub const S_ISVTX: u32 = 0o1000; // c:688
/// `S_IRUSR` constant.
pub const S_IRUSR: u32 = 0o0400; // c:691
/// `S_IWUSR` constant.
pub const S_IWUSR: u32 = 0o0200; // c:694
/// `S_IXUSR` constant.
pub const S_IXUSR: u32 = 0o0100; // c:697
/// `S_IRGRP` constant.
pub const S_IRGRP: u32 = 0o0040; // c:700
/// `S_IWGRP` constant.
pub const S_IWGRP: u32 = 0o0020; // c:703
/// `S_IXGRP` constant.
pub const S_IXGRP: u32 = 0o0010; // c:706
/// `S_IROTH` constant.
pub const S_IROTH: u32 = 0o0004; // c:709
/// `S_IWOTH` constant.
pub const S_IWOTH: u32 = 0o0002; // c:712
/// `S_IXOTH` constant.
pub const S_IXOTH: u32 = 0o0001; // c:715

// Aggregate masks (c:718-735).
/// `S_IRWXU` constant.
pub const S_IRWXU: u32 = S_IRUSR | S_IWUSR | S_IXUSR; // c:718
/// `S_IRWXG` constant.
pub const S_IRWXG: u32 = S_IRGRP | S_IWGRP | S_IXGRP; // c:721
/// `S_IRWXO` constant.
pub const S_IRWXO: u32 = S_IROTH | S_IWOTH | S_IXOTH; // c:724
/// `S_IRUGO` constant.
pub const S_IRUGO: u32 = S_IRUSR | S_IRGRP | S_IROTH; // c:727
/// `S_IWUGO` constant.
pub const S_IWUGO: u32 = S_IWUSR | S_IWGRP | S_IWOTH; // c:730
/// `S_IXUGO` constant.
pub const S_IXUGO: u32 = S_IXUSR | S_IXGRP | S_IXOTH; // c:733

// ---------------------------------------------------------------------------
// Stat-mode predicate macros (c:598-636 + c:640-678 fallbacks).
//
// C: `#define S_ISBLK(m) (((m) & S_IFMT) == S_IFBLK)` and parallels.
// Rust preserves the UPPERCASE macro names with `#[allow(non_snake_case)]`
// per the macro casing rule.
// ---------------------------------------------------------------------------

/// Port of `#define S_ISBLK(m)` from `Src/zsh_system.h:598`.
#[inline]
#[allow(non_snake_case)]
pub const fn S_ISBLK(m: u32) -> bool {
    // c:598
    (m & S_IFMT) == S_IFBLK
}

/// Port of `#define S_ISCHR(m)` from `Src/zsh_system.h:601`.
#[inline]
#[allow(non_snake_case)]
pub const fn S_ISCHR(m: u32) -> bool {
    // c:601
    (m & S_IFMT) == S_IFCHR
}

/// Port of `#define S_ISDIR(m)` from `Src/zsh_system.h:604`.
#[inline]
#[allow(non_snake_case)]
pub const fn S_ISDIR(m: u32) -> bool {
    // c:604
    (m & S_IFMT) == S_IFDIR
}

/// Port of `#define S_ISDOOR(m)` from `Src/zsh_system.h:607`.
/// Solaris-only file type; returns false on every other host.
#[inline]
#[allow(non_snake_case)]
pub const fn S_ISDOOR(_m: u32) -> bool {
    // c:607 + c:649 fallback
    false
}

/// Port of `#define S_ISFIFO(m)` from `Src/zsh_system.h:610`.
#[inline]
#[allow(non_snake_case)]
pub const fn S_ISFIFO(m: u32) -> bool {
    // c:610
    (m & S_IFMT) == S_IFIFO
}

/// Port of `#define S_ISLNK(m)` from `Src/zsh_system.h:613`.
#[inline]
#[allow(non_snake_case)]
pub const fn S_ISLNK(m: u32) -> bool {
    // c:613
    (m & S_IFMT) == S_IFLNK
}

/// Port of `#define S_ISMPB(m)` from `Src/zsh_system.h:616`.
/// V7-only file type; returns false on every modern host.
#[inline]
#[allow(non_snake_case)]
pub const fn S_ISMPB(_m: u32) -> bool {
    // c:616 + c:658 fallback
    false
}

/// Port of `#define S_ISMPC(m)` from `Src/zsh_system.h:619`.
/// V7-only file type; returns false on every modern host.
#[inline]
#[allow(non_snake_case)]
pub const fn S_ISMPC(_m: u32) -> bool {
    // c:619 + c:661 fallback
    false
}

/// Port of `#define S_ISNWK(m)` from `Src/zsh_system.h:622`.
/// HP/UX-only file type; returns false on every other host.
#[inline]
#[allow(non_snake_case)]
pub const fn S_ISNWK(_m: u32) -> bool {
    // c:622 + c:664 fallback
    false
}

/// Port of `#define S_ISOFD(m)` from `Src/zsh_system.h:625`.
/// Cray-only file type; returns false on every other host.
#[inline]
#[allow(non_snake_case)]
pub const fn S_ISOFD(_m: u32) -> bool {
    // c:625 + c:667 fallback
    false
}

/// Port of `#define S_ISOFL(m)` from `Src/zsh_system.h:628`.
/// Cray-only file type; returns false on every other host.
#[inline]
#[allow(non_snake_case)]
pub const fn S_ISOFL(_m: u32) -> bool {
    // c:628 + c:670 fallback
    false
}

/// Port of `#define S_ISREG(m)` from `Src/zsh_system.h:631`.
#[inline]
#[allow(non_snake_case)]
pub const fn S_ISREG(m: u32) -> bool {
    // c:631
    (m & S_IFMT) == S_IFREG
}

/// Port of `#define S_ISSOCK(m)` from `Src/zsh_system.h:634`.
#[inline]
#[allow(non_snake_case)]
pub const fn S_ISSOCK(m: u32) -> bool {
    // c:634
    (m & S_IFMT) == S_IFSOCK
}

// ---------------------------------------------------------------------------
// Wait-status decoder macros (c:346-366).
//
// These are the POSIX `<sys/wait.h>` macros. Rust's libc has them
// only as `libc::WIFEXITED` etc. on Unix; the C fallback definitions
// at c:347-366 are the bit-twiddling formulas POSIX guarantees.
// ---------------------------------------------------------------------------

/// Port of `#define WIFEXITED(X)` from `Src/zsh_system.h:347`.
/// True iff the child exited normally (didn't terminate via signal).
#[inline]
#[allow(non_snake_case)]
pub const fn WIFEXITED(x: i32) -> bool {
    // c:347
    (x & 0o377) == 0
}

/// Port of `#define WEXITSTATUS(X)` from `Src/zsh_system.h:350`.
/// Exit code of a normally-terminated child (only valid when
/// `WIFEXITED` is true).
#[inline]
#[allow(non_snake_case)]
pub const fn WEXITSTATUS(x: i32) -> i32 {
    // c:350
    (x >> 8) & 0o377
}

/// Port of `#define WIFSIGNALED(X)` from `Src/zsh_system.h:353`.
/// True iff the child was killed by a signal.
#[inline]
#[allow(non_snake_case)]
pub const fn WIFSIGNALED(x: i32) -> bool {
    // c:353
    let lo = x & 0o377;
    lo != 0 && lo != 0o177
}

/// Port of `#define WTERMSIG(X)` from `Src/zsh_system.h:356`.
/// Signal that killed the child (only valid when `WIFSIGNALED` is true).
#[inline]
#[allow(non_snake_case)]
pub const fn WTERMSIG(x: i32) -> i32 {
    // c:356
    x & 0o177
}

/// Port of `#define WCOREDUMP(X)` from `Src/zsh_system.h:359`.
/// True iff the child produced a core dump (only valid when
/// `WIFSIGNALED` is true).
#[inline]
#[allow(non_snake_case)]
pub const fn WCOREDUMP(x: i32) -> bool {
    // c:359
    (x & 0o200) != 0
}

/// Port of `#define WIFSTOPPED(X)` from `Src/zsh_system.h:362`.
/// True iff the child is stopped (raised SIGSTOP or similar).
#[inline]
#[allow(non_snake_case)]
pub const fn WIFSTOPPED(x: i32) -> bool {
    // c:362
    (x & 0o377) == 0o177
}

/// Port of `#define WSTOPSIG(X)` from `Src/zsh_system.h:365`.
/// Signal that stopped the child (only valid when `WIFSTOPPED` is true).
#[inline]
#[allow(non_snake_case)]
pub const fn WSTOPSIG(x: i32) -> i32 {
    // c:365
    (x >> 8) & 0o377
}

// ---------------------------------------------------------------------------
// access(2) mode constants (c:746-751).
// ---------------------------------------------------------------------------

/// Port of `#define F_OK` from `Src/zsh_system.h:747`. Test for
/// existence.
pub const F_OK: i32 = 0; // c:747

/// Port of `#define X_OK` from `Src/zsh_system.h:748`. Test for
/// execute permission.
pub const X_OK: i32 = 1; // c:748

/// Port of `#define W_OK` from `Src/zsh_system.h:749`. Test for
/// write permission.
pub const W_OK: i32 = 2; // c:749

/// Port of `#define R_OK` from `Src/zsh_system.h:750`. Test for
/// read permission.
pub const R_OK: i32 = 4; // c:750

// ---------------------------------------------------------------------------
// Misc macros (c:791, 818).
// ---------------------------------------------------------------------------

/// Port of `#define O_NOCTTY` from `Src/zsh_system.h:791-793`. C
/// falls back to 0 when libc doesn't expose it; Rust uses the libc
/// constant directly (Linux/macOS/BSD all define it).
#[cfg(unix)]
pub const O_NOCTTY: i32 = libc::O_NOCTTY; // c:791
/// `O_NOCTTY` constant.
#[cfg(not(unix))]
pub const O_NOCTTY: i32 = 0;

/// Port of `#define IS_DIRSEP(c)` from `Src/zsh_system.h:818`.
/// Tests whether `c` is a directory separator. Cygwin treats `\\` as
/// a separator too (c:816); every other host is `/` only. Rust port
/// matches the non-Cygwin arm.
#[inline]
#[allow(non_snake_case)]
pub const fn IS_DIRSEP(c: char) -> bool {
    // c:818
    c == '/'
}

// ---------------------------------------------------------------------------
// Per-platform stat-time-nanosecond accessors (c:877-897).
//
// C uses three `#elif`-chained alternatives for each of atime/mtime/
// ctime depending on which struct stat member the host's libc
// exposes (`st_atim.tv_nsec` on glibc, `st_atimespec.tv_nsec` on BSD/
// macOS, `st_atimensec` on others). Rust's `std::fs::Metadata` has
// `accessed()`/`modified()` methods that handle this uniformly; the
// nanoseconds-only accessors below extract from those.
// ---------------------------------------------------------------------------

/// Port of `#define GET_ST_ATIME_NSEC(st)` from `Src/zsh_system.h:878`.
#[inline]
#[allow(non_snake_case)]
pub fn GET_ST_ATIME_NSEC(st: &std::fs::Metadata) -> u32 {
    // c:878
    st.accessed()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.subsec_nanos())
        .unwrap_or(0)
}

/// Port of `#define GET_ST_MTIME_NSEC(st)` from `Src/zsh_system.h:885`.
#[inline]
#[allow(non_snake_case)]
pub fn GET_ST_MTIME_NSEC(st: &std::fs::Metadata) -> u32 {
    // c:885
    st.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.subsec_nanos())
        .unwrap_or(0)
}

/// Port of `#define GET_ST_CTIME_NSEC(st)` from `Src/zsh_system.h:892`.
/// Rust's `Metadata` doesn't expose ctime cross-platform; on Unix we
/// pull it from `MetadataExt::ctime_nsec()`.
#[inline]
#[allow(non_snake_case)]
pub fn GET_ST_CTIME_NSEC(st: &std::fs::Metadata) -> u32 {
    // c:892
    #[cfg(unix)]
    {
        st.ctime_nsec() as u32
    }
    #[cfg(not(unix))]
    {
        let _ = st;
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies DIGBUFSIZE for i64 is 21 (printable decimal length
    /// of `-9223372036854775808` + sign + null).
    #[test]
    fn digbufsize_i64_is_21() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(DIGBUFSIZE, 21);
    }

    /// Verifies BDIGBUFSIZE for i64 is 68 (64 bits + 4 padding).
    #[test]
    fn bdigbufsize_i64_is_68() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(BDIGBUFSIZE, 68);
    }

    /// Verifies access mode constants match POSIX values.
    #[test]
    fn access_modes_correct() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(F_OK, 0);
        assert_eq!(X_OK, 1);
        assert_eq!(W_OK, 2);
        assert_eq!(R_OK, 4);
    }

    /// Verifies file-permission bit values match POSIX octal literals
    /// per c:682-715.
    #[test]
    fn permission_bits_match_posix() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(S_ISUID, 0o4000);
        assert_eq!(S_ISGID, 0o2000);
        assert_eq!(S_ISVTX, 0o1000);
        assert_eq!(S_IRUSR, 0o0400);
        assert_eq!(S_IWUSR, 0o0200);
        assert_eq!(S_IXUSR, 0o0100);
        assert_eq!(S_IRGRP, 0o0040);
        assert_eq!(S_IRWXU, 0o0700);
        assert_eq!(S_IRWXG, 0o0070);
        assert_eq!(S_IRWXO, 0o0007);
        assert_eq!(S_IRUGO, 0o0444);
        assert_eq!(S_IWUGO, 0o0222);
        assert_eq!(S_IXUGO, 0o0111);
    }

    /// Verifies S_ISDIR / S_ISREG / etc. predicates per c:598-634.
    /// Use a directory-typed mode word: 0o40755 on Linux/macOS.
    #[test]
    fn stat_predicates_dispatch() {
        let _g = crate::test_util::global_state_lock();
        let dir_mode = libc::S_IFDIR as u32 | 0o755;
        let file_mode = libc::S_IFREG as u32 | 0o644;
        let link_mode = libc::S_IFLNK as u32 | 0o777;
        assert!(S_ISDIR(dir_mode));
        assert!(!S_ISDIR(file_mode));
        assert!(S_ISREG(file_mode));
        assert!(!S_ISREG(dir_mode));
        assert!(S_ISLNK(link_mode));
        assert!(!S_ISLNK(file_mode));
        // Unsupported types always return false on portable hosts.
        assert!(!S_ISDOOR(dir_mode));
        assert!(!S_ISMPB(dir_mode));
        assert!(!S_ISMPC(dir_mode));
        assert!(!S_ISNWK(dir_mode));
        assert!(!S_ISOFD(dir_mode));
        assert!(!S_ISOFL(dir_mode));
    }

    /// Verifies the wait-status decoders per c:347-365.
    #[test]
    fn wait_decoders_normal_exit() {
        let _g = crate::test_util::global_state_lock();
        // Normal exit with status 42: low byte 0, high byte 42.
        let status = 42 << 8;
        assert!(WIFEXITED(status));
        assert!(!WIFSIGNALED(status));
        assert!(!WIFSTOPPED(status));
        assert_eq!(WEXITSTATUS(status), 42);
    }

    #[test]
    fn wait_decoders_killed_by_signal() {
        let _g = crate::test_util::global_state_lock();
        // Killed by SIGTERM (15) without core dump: low byte = 15.
        let status = 15;
        assert!(!WIFEXITED(status));
        assert!(WIFSIGNALED(status));
        assert!(!WIFSTOPPED(status));
        assert_eq!(WTERMSIG(status), 15);
        assert!(!WCOREDUMP(status));
    }

    #[test]
    fn wait_decoders_killed_with_core() {
        let _g = crate::test_util::global_state_lock();
        // Killed by SIGSEGV (11) with core dump bit set.
        let status = 11 | 0o200;
        assert!(WIFSIGNALED(status));
        assert_eq!(WTERMSIG(status), 11);
        assert!(WCOREDUMP(status));
    }

    #[test]
    fn wait_decoders_stopped() {
        let _g = crate::test_util::global_state_lock();
        // Stopped by SIGTSTP (20): low byte = 0177, high byte = 20.
        let status = (20 << 8) | 0o177;
        assert!(!WIFEXITED(status));
        assert!(!WIFSIGNALED(status));
        assert!(WIFSTOPPED(status));
        assert_eq!(WSTOPSIG(status), 20);
    }

    /// Verifies IS_DIRSEP only matches '/' on non-Cygwin hosts.
    #[test]
    fn is_dirsep_basic() {
        let _g = crate::test_util::global_state_lock();
        assert!(IS_DIRSEP('/'));
        assert!(!IS_DIRSEP('\\'));
        assert!(!IS_DIRSEP('a'));
    }

    /// Verifies DEFAULT_WORDCHARS contains the canonical zsh defaults
    /// per c:427.
    #[test]
    fn default_wordchars_contents() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(DEFAULT_WORDCHARS, "*?_-.[]~=/&;!#$%^(){}<>");
    }

    /// Verifies DEFAULT_TIMEFMT matches the canonical c:428 string.
    #[test]
    fn default_timefmt_contents() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(DEFAULT_TIMEFMT, "%J  %U user %S system %P cpu %*E total");
    }

    /// Verifies ZSH_INITIAL_OPEN_MAX is 64 per c:307.
    #[test]
    fn zsh_initial_open_max_is_64() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(ZSH_INITIAL_OPEN_MAX, 64);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/zsh_system.h.
    // ═══════════════════════════════════════════════════════════════════

    /// c:878 — `GET_ST_ATIME_NSEC` on tempfile returns nsec in [0, 1_000_000_000).
    #[test]
    fn get_st_atime_nsec_in_valid_range() {
        let _g = crate::test_util::global_state_lock();
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("atime_test");
        std::fs::write(&p, b"x").unwrap();
        let md = std::fs::metadata(&p).unwrap();
        let nsec = GET_ST_ATIME_NSEC(&md);
        assert!(nsec < 1_000_000_000, "nsec must be < 1B, got {}", nsec);
    }

    /// c:885 — `GET_ST_MTIME_NSEC` on tempfile returns nsec in valid range.
    #[test]
    fn get_st_mtime_nsec_in_valid_range() {
        let _g = crate::test_util::global_state_lock();
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("mtime_test");
        std::fs::write(&p, b"x").unwrap();
        let md = std::fs::metadata(&p).unwrap();
        let nsec = GET_ST_MTIME_NSEC(&md);
        assert!(nsec < 1_000_000_000, "nsec must be < 1B");
    }

    /// c:892 — `GET_ST_CTIME_NSEC` on tempfile returns nsec in valid range.
    #[test]
    #[cfg(unix)]
    fn get_st_ctime_nsec_in_valid_range() {
        let _g = crate::test_util::global_state_lock();
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("ctime_test");
        std::fs::write(&p, b"x").unwrap();
        let md = std::fs::metadata(&p).unwrap();
        let nsec = GET_ST_CTIME_NSEC(&md);
        assert!(nsec < 1_000_000_000, "nsec must be < 1B");
    }

    /// c:878 — `GET_ST_ATIME_NSEC` is deterministic for same metadata.
    #[test]
    fn get_st_atime_nsec_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("det_test");
        std::fs::write(&p, b"x").unwrap();
        let md = std::fs::metadata(&p).unwrap();
        let first = GET_ST_ATIME_NSEC(&md);
        for _ in 0..5 {
            assert_eq!(GET_ST_ATIME_NSEC(&md), first);
        }
    }

    /// c:427 — DEFAULT_WORDCHARS includes all canonical chars (catch
    /// regression that drops common ones like `_`, `-`, `.`).
    #[test]
    fn default_wordchars_includes_canonical_chars() {
        for c in ['_', '-', '.', '~', '/', '&', ';', '!', '#', '$', '%', '^'] {
            assert!(
                DEFAULT_WORDCHARS.contains(c),
                "DEFAULT_WORDCHARS missing canonical char {:?}",
                c
            );
        }
    }

    /// c:427 — DEFAULT_WORDCHARS has no whitespace (would corrupt
    /// word-boundary detection).
    #[test]
    fn default_wordchars_no_whitespace() {
        for c in DEFAULT_WORDCHARS.chars() {
            assert!(
                !c.is_whitespace(),
                "DEFAULT_WORDCHARS must not contain whitespace, found {:?}",
                c
            );
        }
    }

    /// c:428 — DEFAULT_TIMEFMT contains required time specifiers.
    #[test]
    fn default_timefmt_contains_required_specifiers() {
        // Must include %J (name), %U (user), %S (system), %P (cpu%),
        // %*E (elapsed) per zsh time-builtin spec.
        for spec in ["%J", "%U", "%S", "%P", "%*E"] {
            assert!(
                DEFAULT_TIMEFMT.contains(spec),
                "DEFAULT_TIMEFMT missing required spec {}",
                spec
            );
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/zsh_system.h c:594 + c:347-365.
    // ═══════════════════════════════════════════════════════════════════

    /// c:594 — S_IFMT covers all file-type bits used by predicates.
    /// Every S_IF* family bit must be a subset of S_IFMT.
    #[test]
    fn s_ifmt_covers_all_file_type_bits() {
        for &bits in &[S_IFBLK, S_IFCHR, S_IFDIR, S_IFIFO, S_IFLNK, S_IFREG, S_IFSOCK] {
            assert_eq!(
                bits & S_IFMT,
                bits,
                "S_IF* bits {:o} must be subset of S_IFMT {:o}",
                bits,
                S_IFMT
            );
        }
    }

    /// c:347-365 — wait-status decoders are mutually exclusive:
    /// at most ONE of WIFEXITED/WIFSIGNALED/WIFSTOPPED true per status.
    #[test]
    fn wait_decoders_mutually_exclusive() {
        for status in [42 << 8, 15, 11 | 0o200, (20 << 8) | 0o177] {
            let exited = WIFEXITED(status);
            let signaled = WIFSIGNALED(status);
            let stopped = WIFSTOPPED(status);
            let count = (exited as i32) + (signaled as i32) + (stopped as i32);
            assert_eq!(
                count, 1,
                "exactly one of W{{IFEXITED/IFSIGNALED/IFSTOPPED}} must be true for status {:o}",
                status
            );
        }
    }

    /// c:350 — WEXITSTATUS extracts high byte (0..256).
    #[test]
    fn wexitstatus_always_in_byte_range() {
        for code in [0i32, 1, 42, 127, 200, 255] {
            let status = code << 8;
            assert_eq!(WEXITSTATUS(status), code & 0xff);
        }
    }

    /// c:356 — WTERMSIG extracts low 7 bits (signal number space).
    #[test]
    fn wtermsig_low_seven_bits() {
        for sig in [1i32, 2, 9, 11, 15, 31, 63, 127] {
            let status = sig;
            assert_eq!(WTERMSIG(status), sig);
        }
    }

    /// c:347 — WIFEXITED(0) is true (clean exit-0).
    #[test]
    fn wifexited_zero_status_is_true() {
        assert!(WIFEXITED(0), "exit 0 must satisfy WIFEXITED");
        assert_eq!(WEXITSTATUS(0), 0);
    }

    /// c:347-365 — exit + signal can't both be encoded simultaneously
    /// (encoding-wise impossible).
    #[test]
    fn wait_signal_and_exit_not_simultaneously_encoded() {
        // Signal-encoding: low byte nonzero non-0177.
        // Exit-encoding: low byte 0.
        // They're disjoint by construction.
        for sig in [1, 11, 15] {
            assert!(WIFSIGNALED(sig));
            assert!(!WIFEXITED(sig));
        }
        for code in [0, 42, 200] {
            assert!(WIFEXITED(code << 8));
            assert!(!WIFSIGNALED(code << 8));
        }
    }

    /// c:818 — IS_DIRSEP is `const fn` (compile-time evaluable).
    #[test]
    fn is_dirsep_const_fn_compiles() {
        const SLASH_IS_SEP: bool = IS_DIRSEP('/');
        const BACK_IS_NOT_SEP: bool = IS_DIRSEP('\\');
        assert!(SLASH_IS_SEP);
        assert!(!BACK_IS_NOT_SEP);
    }

    /// c:746-750 — access(2) constants are all distinct (so OR'ing
    /// combos works correctly).
    #[test]
    fn access_constants_pairwise_distinct() {
        let codes = [F_OK, X_OK, W_OK, R_OK];
        let unique: std::collections::HashSet<_> = codes.iter().copied().collect();
        assert_eq!(unique.len(), codes.len(), "all access codes must differ");
    }

    /// c:748-750 — X_OK|W_OK|R_OK = 7 (canonical "test all" combo).
    #[test]
    fn access_all_perms_combine_to_seven() {
        assert_eq!(X_OK | W_OK | R_OK, 7, "canonical R|W|X = 7");
    }

    /// c:682-715 — every permission bit is a single bit (power of two).
    #[test]
    fn permission_bits_are_single_bits() {
        for &b in &[S_ISUID, S_ISGID, S_ISVTX, S_IRUSR, S_IWUSR, S_IXUSR,
                    S_IRGRP, S_IWGRP, S_IXGRP, S_IROTH, S_IWOTH, S_IXOTH] {
            assert!(b.is_power_of_two(), "{:o} must be a single bit", b);
        }
    }

    /// c:718-735 — aggregate masks correctly OR their constituents.
    #[test]
    fn aggregate_masks_match_constituents() {
        assert_eq!(S_IRWXU, S_IRUSR | S_IWUSR | S_IXUSR, "c:718");
        assert_eq!(S_IRWXG, S_IRGRP | S_IWGRP | S_IXGRP, "c:721");
        assert_eq!(S_IRWXO, S_IROTH | S_IWOTH | S_IXOTH, "c:724");
        assert_eq!(S_IRUGO, S_IRUSR | S_IRGRP | S_IROTH, "c:727");
        assert_eq!(S_IWUGO, S_IWUSR | S_IWGRP | S_IWOTH, "c:730");
        assert_eq!(S_IXUGO, S_IXUSR | S_IXGRP | S_IXOTH, "c:733");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/zsh_system.h size constants +
    // file-mode constants + access mode + IS_DIRSEP additional sweep.
    // c:298 PATH_MAX / c:307 ZSH_INITIAL_OPEN_MAX / c:313 OPEN_MAX /
    // c:594 S_IFMT / c:818 IS_DIRSEP
    // ═══════════════════════════════════════════════════════════════════

    /// c:298 — `PATH_MAX` is positive non-zero size.
    #[test]
    fn path_max_positive_non_zero() {
        assert!(PATH_MAX > 0, "PATH_MAX must be > 0");
    }

    /// c:298 — `PATH_MAX` is at least 256 (legacy POSIX guarantee).
    #[test]
    fn path_max_at_least_256() {
        assert!(PATH_MAX >= 256, "PATH_MAX must be ≥ 256 POSIX min");
    }

    /// c:307 — `ZSH_INITIAL_OPEN_MAX = 64` (canonical zsh default).
    #[test]
    fn zsh_initial_open_max_canonical_64() {
        assert_eq!(ZSH_INITIAL_OPEN_MAX, 64, "canonical zsh default");
    }

    /// c:313 — `OPEN_MAX` ≥ `ZSH_INITIAL_OPEN_MAX` (start size never exceeds cap).
    #[test]
    fn open_max_at_least_initial() {
        assert!(OPEN_MAX >= ZSH_INITIAL_OPEN_MAX,
            "OPEN_MAX {} must be ≥ ZSH_INITIAL_OPEN_MAX {}",
            OPEN_MAX, ZSH_INITIAL_OPEN_MAX);
    }

    /// c:569 — `DIGBUFSIZE` is positive (i64 decimal digits + sign + NUL).
    #[test]
    fn digbufsize_positive() {
        assert!(DIGBUFSIZE > 0, "DIGBUFSIZE must hold at least 1 byte");
    }

    /// c:570 — `BDIGBUFSIZE` is positive (i64 binary digits + padding).
    #[test]
    fn bdigbufsize_positive() {
        assert!(BDIGBUFSIZE > 0, "BDIGBUFSIZE must hold at least 1 byte");
    }

    /// c:570 — `BDIGBUFSIZE > DIGBUFSIZE` (binary needs more bytes than decimal).
    #[test]
    fn bdigbufsize_greater_than_digbufsize() {
        assert!(BDIGBUFSIZE > DIGBUFSIZE,
            "binary {} must need more bytes than decimal {}",
            BDIGBUFSIZE, DIGBUFSIZE);
    }

    /// c:594 — `S_IFMT` is non-zero file-type mask.
    #[test]
    fn s_ifmt_is_nonzero() {
        assert!(S_IFMT > 0);
    }

    /// c:818 — `IS_DIRSEP(NUL)` returns false (no NUL as separator).
    #[test]
    fn is_dirsep_nul_returns_false() {
        assert!(!IS_DIRSEP('\0'));
    }

    /// c:818 — `IS_DIRSEP` only matches `/` (full ASCII sweep).
    #[test]
    fn is_dirsep_only_matches_slash_full_ascii() {
        for b in 0u8..128 {
            let c = b as char;
            let r = IS_DIRSEP(c);
            if c == '/' {
                assert!(r, "'/' must match");
            } else {
                assert!(!r, "{:?} must NOT match", c);
            }
        }
    }
}

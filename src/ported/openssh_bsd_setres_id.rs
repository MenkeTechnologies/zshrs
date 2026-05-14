//! Port of `Src/openssh_bsd_setres_id.c` — `setresuid()` /
//! `setresgid()` wrappers for platforms whose libc lacks the native
//! syscalls (or has broken `setreuid()` / `setregid()` like NetBSD).
//!
//! Strict 1:1 mirror of the upstream control flow. The configure-time
//! macros are translated to compile-time `cfg!` checks against
//! `target_os`:
//!
//! * `BROKEN_SETREUID` / `BROKEN_SETREGID` — set on NetBSD, where
//!   `setreuid()` / `setregid()` fail to reset the saved uid/gid when
//!   the real id isn't modified. Falls back to `setegid()` +
//!   `setgid()` (resp. `seteuid()` + `setuid()`) which reset all
//!   three.
//! * `SETEUID_BREAKS_SETUID` — not known to apply to any modern
//!   platform we target; the `seteuid()` branch is therefore always
//!   taken in the non-native fallback.
//!
//! On platforms whose libc already provides `setresuid(2)` /
//! `setresgid(2)` (Linux, FreeBSD, OpenBSD, DragonFly), prefer those
//! syscalls directly; the wrappers below are only used as a fallback
//! for platforms that lack them. This matches the upstream
//! `ZSH_IMPLEMENT_SETRES{U,G}ID` configure gate.

#![allow(clippy::needless_return)]

use crate::utils::zwarnnam;
use std::cell::UnsafeCell;

/// Port of `setresgid(gid_t rgid, gid_t egid, gid_t sgid)` from Src/openssh_bsd_setres_id.c:70.
///
/// Set the real, effective and saved group ids. Implementation
/// requires `rgid == sgid` (the only combination zsh ever passes);
/// any other combination returns `-1` with `errno = ENOSYS`, exactly
/// as the C source does.
///
/// # Safety
///
/// Calls into libc to alter the calling process's credentials. The
/// caller must have appropriate privileges; behaviour matches the
/// underlying syscalls.
#[cfg(unix)]
pub unsafe fn setresgid(rgid: libc::gid_t, egid: libc::gid_t, sgid: libc::gid_t) -> libc::c_int {
    let mut ret: libc::c_int = 0;
    let mut saved_errno: libc::c_int;

    if rgid != sgid {
        errno_set(libc::ENOSYS);
        return -1;
    }

    if have_native_setregid() && !broken_setregid() {
        if libc::setregid(rgid, egid) < 0 {
            saved_errno = errno_get();
            zwarnnam("setregid", &format!("to gid {}: {}", rgid as i64, errno_str(saved_errno)));
            errno_set(saved_errno);
            ret = -1;
        }
    } else {
        if libc::setegid(egid) < 0 {
            saved_errno = errno_get();
            zwarnnam("setegid", &format!("to gid {}: {}", egid as i64, errno_str(saved_errno)));
            errno_set(saved_errno);
            ret = -1;
        }
        if libc::setgid(rgid) < 0 {
            saved_errno = errno_get();
            zwarnnam("setgid", &format!("to gid {}: {}", rgid as i64, errno_str(saved_errno)));
            errno_set(saved_errno);
            ret = -1;
        }
    }
    ret
}

/// Port of `setresuid(uid_t ruid, uid_t euid, uid_t suid)` from Src/openssh_bsd_setres_id.c:105.
///
/// Set the real, effective and saved user ids. As with `setresgid()`,
/// only `ruid == suid` is supported; other combinations return `-1`
/// with `errno = ENOSYS`.
///
/// # Safety
///
/// Calls into libc to alter the calling process's credentials. The
/// caller must have appropriate privileges; behaviour matches the
/// underlying syscalls.
#[cfg(unix)]
pub unsafe fn setresuid(ruid: libc::uid_t, euid: libc::uid_t, suid: libc::uid_t) -> libc::c_int {
    let mut ret: libc::c_int = 0;
    let mut saved_errno: libc::c_int;

    if ruid != suid {
        errno_set(libc::ENOSYS);
        return -1;
    }

    if have_native_setreuid() && !broken_setreuid() {
        if libc::setreuid(ruid, euid) < 0 {
            saved_errno = errno_get();
            zwarnnam("setreuid", &format!("to uid {}: {}", ruid as i64, errno_str(saved_errno)));
            errno_set(saved_errno);
            ret = -1;
        }
    } else {
        if !seteuid_breaks_setuid() {
            if libc::seteuid(euid) < 0 {
                saved_errno = errno_get();
                zwarnnam("seteuid", &format!("to uid {}: {}", euid as i64, errno_str(saved_errno)));
                errno_set(saved_errno);
                ret = -1;
            }
        }
        if libc::setuid(ruid) < 0 {
            saved_errno = errno_get();
            zwarnnam("setuid", &format!("to uid {}: {}", ruid as i64, errno_str(saved_errno)));
            errno_set(saved_errno);
            ret = -1;
        }
    }
    ret
}

/// True on platforms whose `setregid()` does not reset the saved gid.
/// Mirrors `#define BROKEN_SETREGID` under `#ifdef __NetBSD__`.
#[inline]
const fn broken_setregid() -> bool {
    cfg!(target_os = "netbsd")
}

/// True on platforms whose `setreuid()` does not reset the saved uid.
/// Mirrors `#define BROKEN_SETREUID` under `#ifdef __NetBSD__`.
#[inline]
const fn broken_setreuid() -> bool {
    cfg!(target_os = "netbsd")
}

/// True on platforms whose libc provides a native `setregid()` we can
/// call. Mirrors `ZSH_HAVE_NATIVE_SETREGID`; configure detects this on
/// every Unix we currently support, so it is unconditionally true.
#[inline]
const fn have_native_setregid() -> bool {
    cfg!(unix)
}

/// True on platforms whose libc provides a native `setreuid()` we can
/// call. Mirrors `ZSH_HAVE_NATIVE_SETREUID`.
#[inline]
const fn have_native_setreuid() -> bool {
    cfg!(unix)
}

/// True on platforms where calling `seteuid()` first prevents a
/// later `setuid()` from succeeding. Mirrors `SETEUID_BREAKS_SETUID`,
/// which is not detected on any platform we target.
#[inline]
const fn seteuid_breaks_setuid() -> bool {
    false
}

// WARNING: NOT IN OPENSSH_BSD_SETRES_ID.C — Rust-only errno-read
// helper. C reads `errno` (thread-local int via libc) directly; Rust
// has no portable mutable errno accessor in `std`, so this fn wraps
// `std::io::Error::last_os_error().raw_os_error()`.
#[cfg(unix)]
#[inline]
fn errno_get() -> libc::c_int {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

// WARNING: NOT IN OPENSSH_BSD_SETRES_ID.C — Rust-only errno-write
// helper; see `errno_get` above. libc exposes `__errno_location()`
// on Linux, `__error()` on macOS/BSD; this fn dispatches per
// target_os to get the right thread-local accessor. Exotic targets
// fall back to a Rust thread_local; callers re-read via `errno_get()`.
#[cfg(unix)]
#[inline]
fn errno_set(e: libc::c_int) {
    unsafe {
        let p: *mut libc::c_int = {
            #[cfg(any(target_os = "linux", target_os = "android"))]
            { libc::__errno_location() }
            #[cfg(any(target_os = "macos", target_os = "ios", target_os = "freebsd", target_os = "dragonfly"))]
            { libc::__error() }
            #[cfg(any(target_os = "openbsd", target_os = "netbsd"))]
            {
                extern "C" {
                    fn __errno() -> *mut libc::c_int;
                }
                __errno()
            }
            #[cfg(not(any(
                target_os = "linux",
                target_os = "android",
                target_os = "macos",
                target_os = "ios",
                target_os = "freebsd",
                target_os = "dragonfly",
                target_os = "openbsd",
                target_os = "netbsd",
            )))]
            {
                thread_local! {
                    static ERRNO: UnsafeCell<libc::c_int> = const { UnsafeCell::new(0) };
                }
                ERRNO.with(|c| c.get())
            }
        };
        *p = e;
    }
}

// WARNING: NOT IN OPENSSH_BSD_SETRES_ID.C — Rust-only error-string
// formatter. C uses `strerror(errno)` directly; Rust uses
// `std::io::Error::from_raw_os_error(e).to_string()`.
#[cfg(unix)]
#[inline]
fn errno_str(e: libc::c_int) -> String {
    std::io::Error::from_raw_os_error(e).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `rgid != sgid` must be rejected with ENOSYS, matching the C
    /// pre-check at openssh_bsd_setres_id.c:74.
    #[test]
    #[cfg(unix)]
    fn setresgid_rejects_split_real_saved() {
        unsafe {
            let r = setresgid(1, 2, 3);
            assert_eq!(r, -1);
            assert_eq!(errno_get(), libc::ENOSYS);
        }
    }

    /// Likewise for `setresuid()` at openssh_bsd_setres_id.c:109.
    #[test]
    #[cfg(unix)]
    fn setresuid_rejects_split_real_saved() {
        unsafe {
            let r = setresuid(1, 2, 3);
            assert_eq!(r, -1);
            assert_eq!(errno_get(), libc::ENOSYS);
        }
    }

    /// Equal real/saved with the *current* uid must succeed (no
    /// privileges actually changed). Mirrors the success path at
    /// openssh_bsd_setres_id.c:113.
    #[test]
    #[cfg(unix)]
    fn setresuid_noop_succeeds() {
        unsafe {
            let me = libc::getuid();
            let r = setresuid(me, me, me);
            assert_eq!(r, 0);
        }
    }

    #[test]
    #[cfg(unix)]
    fn setresgid_noop_succeeds() {
        unsafe {
            let me = libc::getgid();
            let r = setresgid(me, me, me);
            assert_eq!(r, 0);
        }
    }
}

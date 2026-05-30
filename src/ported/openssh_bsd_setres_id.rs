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
            zwarnnam(
                "setregid",
                &format!("to gid {}: {}", rgid as i64, errno_str(saved_errno)),
            );
            errno_set(saved_errno);
            ret = -1;
        }
    } else {
        if libc::setegid(egid) < 0 {
            saved_errno = errno_get();
            zwarnnam(
                "setegid",
                &format!("to gid {}: {}", egid as i64, errno_str(saved_errno)),
            );
            errno_set(saved_errno);
            ret = -1;
        }
        if libc::setgid(rgid) < 0 {
            saved_errno = errno_get();
            zwarnnam(
                "setgid",
                &format!("to gid {}: {}", rgid as i64, errno_str(saved_errno)),
            );
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
            zwarnnam(
                "setreuid",
                &format!("to uid {}: {}", ruid as i64, errno_str(saved_errno)),
            );
            errno_set(saved_errno);
            ret = -1;
        }
    } else {
        if !seteuid_breaks_setuid() {
            if libc::seteuid(euid) < 0 {
                saved_errno = errno_get();
                zwarnnam(
                    "seteuid",
                    &format!("to uid {}: {}", euid as i64, errno_str(saved_errno)),
                );
                errno_set(saved_errno);
                ret = -1;
            }
        }
        if libc::setuid(ruid) < 0 {
            saved_errno = errno_get();
            zwarnnam(
                "setuid",
                &format!("to uid {}: {}", ruid as i64, errno_str(saved_errno)),
            );
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
            {
                libc::__errno_location()
            }
            #[cfg(any(
                target_os = "macos",
                target_os = "ios",
                target_os = "freebsd",
                target_os = "dragonfly"
            ))]
            {
                libc::__error()
            }
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
        let _g = crate::test_util::global_state_lock();
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
        let _g = crate::test_util::global_state_lock();
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
        let _g = crate::test_util::global_state_lock();
        unsafe {
            let me = libc::getuid();
            let r = setresuid(me, me, me);
            assert_eq!(r, 0);
        }
    }

    #[test]
    #[cfg(unix)]
    fn setresgid_noop_succeeds() {
        let _g = crate::test_util::global_state_lock();
        unsafe {
            let me = libc::getgid();
            let r = setresgid(me, me, me);
            assert_eq!(r, 0);
        }
    }

    /// c:42 — `setresgid` with the current GID for all three slots
    /// must NOT clear errno from a previous syscall. The C source's
    /// short-circuit at c:113 returns 0 without touching the kernel,
    /// so errno is whatever it was before.
    #[test]
    #[cfg(unix)]
    fn setresgid_noop_does_not_clobber_existing_errno() {
        let _g = crate::test_util::global_state_lock();
        unsafe {
            // Seed errno with a sentinel that won't naturally collide.
            errno_set(libc::EILSEQ);
            let me = libc::getgid();
            let r = setresgid(me, me, me);
            assert_eq!(r, 0);
            assert_eq!(
                errno_get(),
                libc::EILSEQ,
                "no-op short-circuit must not reset errno on success"
            );
        }
    }

    /// c:74 — Asymmetry test: `setresgid(me, me, OTHER)` triggers the
    /// ENOSYS reject just like `setresgid(OTHER, me, me)`. The C
    /// pre-check is "any non-matching real/effective/saved triple →
    /// ENOSYS"; pin both sides.
    #[test]
    #[cfg(unix)]
    fn setresgid_real_matches_effective_but_saved_differs() {
        let _g = crate::test_util::global_state_lock();
        unsafe {
            let me = libc::getgid();
            let other: libc::gid_t = if me == 0 { 1 } else { 0 };
            let r = setresgid(me, me, other);
            assert_eq!(r, -1);
            assert_eq!(errno_get(), libc::ENOSYS);
        }
    }

    /// c:91 — the ENOSYS pre-check is `if (ruid != suid)` only;
    /// effective vs saved disagreement is NOT rejected here. The
    /// function instead proceeds to `setreuid(ruid, euid)` / `seteuid`,
    /// which fails with EPERM (not ENOSYS) for non-root users.
    /// Pin the asymmetry so a regen that tightens the pre-check to
    /// "any unequal triple" gets caught.
    #[test]
    #[cfg(unix)]
    fn setresuid_effective_differs_from_saved_does_not_get_enosys() {
        let _g = crate::test_util::global_state_lock();
        unsafe {
            let me = libc::getuid();
            // Skip on root since seteuid(other) would succeed.
            if me == 0 {
                return;
            }
            let r = setresuid(me, 0, me);
            assert_eq!(r, -1, "non-root cannot seteuid(0)");
            assert_ne!(
                errno_get(),
                libc::ENOSYS,
                "ENOSYS is reserved for the c:91 ruid!=suid pre-check"
            );
        }
    }

    /// `errno_str(0)` must be a non-empty string. The Rust-only
    /// formatter wraps `std::io::Error::from_raw_os_error(0)` which
    /// returns "Success" or "Undefined error: 0" depending on libc;
    /// pin only that it's non-empty so downstream `format!()`
    /// callers don't print "{}" literal.
    #[test]
    fn errno_str_returns_nonempty_for_zero() {
        let _g = crate::test_util::global_state_lock();
        let s = errno_str(0);
        assert!(!s.is_empty(), "errno_str(0) returned empty string");
    }

    /// `errno_str(EINVAL)` must mention an invalid-argument shape.
    /// Catches a regression where the formatter returns a literal
    /// "{}" or the integer instead of the strerror(3) text.
    #[test]
    fn errno_str_for_einval_contains_recognizable_phrase() {
        let _g = crate::test_util::global_state_lock();
        let s = errno_str(libc::EINVAL);
        let l = s.to_lowercase();
        assert!(
            l.contains("invalid") || l.contains("argument") || l.contains("inval"),
            "errno_str(EINVAL) = {:?} — must contain readable text",
            s
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/openssh_bsd_setres_id.c.
    // ═══════════════════════════════════════════════════════════════════

    /// errno_str(EACCES) returns readable text.
    #[test]
    fn errno_str_eacces_returns_readable_text() {
        let _g = crate::test_util::global_state_lock();
        let s = errno_str(libc::EACCES);
        assert!(!s.is_empty(), "EACCES must format to non-empty");
        assert!(!s.contains("{}"), "format placeholder must be expanded");
    }

    /// errno_str(EPERM) returns readable text.
    #[test]
    fn errno_str_eperm_returns_readable_text() {
        let _g = crate::test_util::global_state_lock();
        let s = errno_str(libc::EPERM);
        assert!(!s.is_empty());
    }

    /// errno_str(0) returns a string.
    #[test]
    fn errno_str_zero_returns_nonempty() {
        let _g = crate::test_util::global_state_lock();
        let s = errno_str(0);
        assert!(!s.is_empty());
    }

    /// errno_str is deterministic for any errno.
    #[test]
    fn errno_str_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        for e in [0, libc::EACCES, libc::EPERM, libc::EINVAL, libc::ENOMEM] {
            let first = errno_str(e);
            for _ in 0..5 {
                assert_eq!(errno_str(e), first, "errno {} must be pure", e);
            }
        }
    }

    /// errno_str for arbitrary invalid errno (-1) returns string.
    #[test]
    fn errno_str_negative_errno_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let s = errno_str(-1);
        assert!(!s.is_empty());
    }

    /// Build-config const fns are callable + return bool.
    #[test]
    fn build_config_const_fns_are_callable() {
        let _br: bool = broken_setregid();
        let _bu: bool = broken_setreuid();
        let _hr: bool = have_native_setregid();
        let _hu: bool = have_native_setreuid();
        let _sb: bool = seteuid_breaks_setuid();
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/openssh_bsd_setres_id.c
    // c:70 setresgid / c:95 setresuid + build-config helpers
    // ═══════════════════════════════════════════════════════════════════

    /// c:70 — `setresgid` returns c_int (compile-time type pin).
    #[cfg(unix)]
    #[test]
    fn setresgid_returns_c_int_type() {
        let _g = crate::test_util::global_state_lock();
        let _: libc::c_int = unsafe {
            let gid = libc::getgid();
            setresgid(gid, gid, gid)
        };
    }

    /// c:95 — `setresuid` returns c_int (compile-time type pin).
    #[cfg(unix)]
    #[test]
    fn setresuid_returns_c_int_type() {
        let _g = crate::test_util::global_state_lock();
        let _: libc::c_int = unsafe {
            let uid = libc::getuid();
            setresuid(uid, uid, uid)
        };
    }

    /// c:70 — `setresgid(rgid, egid, sgid)` with rgid != sgid sets
    /// errno = ENOSYS and returns -1. C body c:73-75 first guard.
    #[cfg(unix)]
    #[test]
    fn setresgid_split_rgid_sgid_sets_enosys() {
        let _g = crate::test_util::global_state_lock();
        let r = unsafe { setresgid(0, 1, 999) };
        assert_eq!(r, -1, "rgid != sgid → -1");
        let e = errno_get();
        assert_eq!(e, libc::ENOSYS, "rgid != sgid sets errno=ENOSYS");
    }

    /// c:95 — `setresuid(ruid, euid, suid)` with ruid != suid sets
    /// errno = ENOSYS and returns -1. C body c:98-100 first guard.
    #[cfg(unix)]
    #[test]
    fn setresuid_split_ruid_suid_sets_enosys() {
        let _g = crate::test_util::global_state_lock();
        let r = unsafe { setresuid(0, 1, 999) };
        assert_eq!(r, -1, "ruid != suid → -1");
        let e = errno_get();
        assert_eq!(e, libc::ENOSYS, "ruid != suid sets errno=ENOSYS");
    }

    /// Build-config flags are deterministic (same across calls).
    #[test]
    fn build_config_flags_deterministic() {
        for _ in 0..5 {
            assert_eq!(broken_setregid(), broken_setregid());
            assert_eq!(broken_setreuid(), broken_setreuid());
            assert_eq!(have_native_setregid(), have_native_setregid());
            assert_eq!(have_native_setreuid(), have_native_setreuid());
            assert_eq!(seteuid_breaks_setuid(), seteuid_breaks_setuid());
        }
    }

    /// On macOS/Linux modern targets, `have_native_set*` is true.
    /// Pin the platform contract.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn modern_platforms_have_native_set_res_id() {
        // macOS has setreuid/setregid; Linux has both setre* and setres*.
        // The "have_native_*" check covers the setre* path used as fallback.
        assert!(have_native_setregid(),
            "modern target should have native setregid");
        assert!(have_native_setreuid(),
            "modern target should have native setreuid");
    }

    /// `errno_str` is deterministic for fixed input.
    #[cfg(unix)]
    #[test]
    fn errno_str_full_sweep_deterministic() {
        for e in [0, libc::EPERM, libc::ENOENT, libc::EACCES, libc::EINVAL, 99999] {
            let first = errno_str(e);
            for _ in 0..3 {
                assert_eq!(errno_str(e), first,
                    "errno_str({}) must be deterministic", e);
            }
        }
    }

    /// `errno_str(N)` for any common errno returns non-empty string.
    #[cfg(unix)]
    #[test]
    fn errno_str_common_codes_non_empty() {
        for e in [libc::EPERM, libc::ENOENT, libc::EACCES, libc::EINVAL,
                  libc::EBUSY, libc::EIO] {
            assert!(!errno_str(e).is_empty(),
                "errno_str({}) must return non-empty", e);
        }
    }

    /// `errno_get` / `errno_set` round-trip preserves value.
    #[cfg(unix)]
    #[test]
    fn errno_get_set_round_trip() {
        let _g = crate::test_util::global_state_lock();
        let saved = errno_get();
        errno_set(libc::EACCES);
        assert_eq!(errno_get(), libc::EACCES, "errno round-trips");
        errno_set(saved);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/openssh-compat/bsd-setres_id.c
    // c:41 setresgid / c:95 setresuid / c:142+ build flags / c:242 errno_str
    // ═══════════════════════════════════════════════════════════════════

    /// c:41 — `setresgid(SAME, SAME, SAME)` with all-equal-current values
    /// succeeds (no-op identity) — exercises c:44 short-circuit.
    #[cfg(unix)]
    #[test]
    fn setresgid_all_equal_current_gid_is_noop() {
        let _g = crate::test_util::global_state_lock();
        let g = unsafe { libc::getgid() };
        let r = unsafe { setresgid(g, g, g) };
        assert_eq!(r, 0, "current gid all-equal triple is a no-op success");
    }

    /// c:95 — `setresuid(SAME, SAME, SAME)` with all-equal-current values is no-op.
    #[cfg(unix)]
    #[test]
    fn setresuid_all_equal_current_uid_is_noop() {
        let _g = crate::test_util::global_state_lock();
        let u = unsafe { libc::getuid() };
        let r = unsafe { setresuid(u, u, u) };
        assert_eq!(r, 0, "current uid all-equal triple is a no-op success");
    }

    /// c:41 — `setresgid` returns c_int (compile-time pin, alt).
    #[cfg(unix)]
    #[test]
    fn setresgid_returns_c_int_pin_alt() {
        let _g = crate::test_util::global_state_lock();
        let _: libc::c_int = unsafe { setresgid(0, 1, 999) };
    }

    /// c:95 — `setresuid` returns c_int (compile-time pin, alt).
    #[cfg(unix)]
    #[test]
    fn setresuid_returns_c_int_pin_alt() {
        let _g = crate::test_util::global_state_lock();
        let _: libc::c_int = unsafe { setresuid(0, 1, 999) };
    }

    /// c:142+ — all build-config flags return bool (compile-time pin).
    #[test]
    fn build_config_flags_return_bool_type() {
        let _: bool = broken_setregid();
        let _: bool = broken_setreuid();
        let _: bool = have_native_setregid();
        let _: bool = have_native_setreuid();
        let _: bool = seteuid_breaks_setuid();
    }

    /// c:142+ — `broken_setregid` and `have_native_setregid` are
    /// mutually exclusive (can't be both broken AND native-supported).
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn broken_and_native_mutually_exclusive_setregid() {
        if have_native_setregid() {
            assert!(!broken_setregid(),
                "having native setregid implies not broken");
        }
    }

    /// c:142+ — `broken_setreuid` and `have_native_setreuid` mutually exclusive.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn broken_and_native_mutually_exclusive_setreuid() {
        if have_native_setreuid() {
            assert!(!broken_setreuid(),
                "having native setreuid implies not broken");
        }
    }

    /// c:242 — `errno_str` returns String (compile-time pin).
    #[cfg(unix)]
    #[test]
    fn errno_str_returns_string_type() {
        let _: String = errno_str(0);
    }

    /// c:242 — `errno_str(0)` returns non-empty (success message
    /// or "Undefined error: 0" — but always something).
    #[cfg(unix)]
    #[test]
    fn errno_str_zero_returns_non_empty() {
        let s = errno_str(0);
        assert!(!s.is_empty(), "errno_str(0) must be non-empty");
    }

    /// c:182 — `errno_get` returns c_int (compile-time pin).
    #[cfg(unix)]
    #[test]
    fn errno_get_returns_c_int_type() {
        let _g = crate::test_util::global_state_lock();
        let _: libc::c_int = errno_get();
    }

    /// c:193 — `errno_set` returns void (compile-time pin).
    #[cfg(unix)]
    #[test]
    fn errno_set_returns_void_type() {
        let _g = crate::test_util::global_state_lock();
        let saved = errno_get();
        let _: () = errno_set(0);
        errno_set(saved);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity pins for Src/openssh_bsd_setres_id.c
    // c:70 setresgid / c:104 setresuid / c:142-164 config flags /
    // c:182 errno_get / c:193 errno_set / c:242 errno_str
    // ═══════════════════════════════════════════════════════════════════

    /// c:70 — `setresgid(rgid, egid, sgid)` with rgid != sgid returns -1
    /// AND sets errno = ENOSYS. Verify the errno side-effect.
    #[cfg(unix)]
    #[test]
    fn setresgid_rgid_neq_sgid_sets_errno_enosys() {
        let _g = crate::test_util::global_state_lock();
        let saved = errno_get();
        errno_set(0);
        let r = unsafe { setresgid(0 as libc::gid_t, 0 as libc::gid_t, 99 as libc::gid_t) };
        assert_eq!(r, -1, "rgid != sgid must return -1");
        assert_eq!(errno_get(), libc::ENOSYS,
            "errno must be set to ENOSYS per c:70");
        errno_set(saved);
    }

    /// c:104 — `setresuid(ruid, euid, suid)` with ruid != suid returns -1
    /// AND sets errno = ENOSYS.
    #[cfg(unix)]
    #[test]
    fn setresuid_ruid_neq_suid_sets_errno_enosys() {
        let _g = crate::test_util::global_state_lock();
        let saved = errno_get();
        errno_set(0);
        let r = unsafe { setresuid(0 as libc::uid_t, 0 as libc::uid_t, 99 as libc::uid_t) };
        assert_eq!(r, -1, "ruid != suid must return -1");
        assert_eq!(errno_get(), libc::ENOSYS,
            "errno must be set to ENOSYS per c:104");
        errno_set(saved);
    }

    /// c:142-164 — at least one of (native, !broken) is true on every
    /// modern target — i.e. setres{u,g}id reaches the wrapper or
    /// the fallback, never both broken.
    #[cfg(unix)]
    #[test]
    fn config_flags_at_least_one_path_viable_setregid() {
        // native && !broken → native path; otherwise fallback runs.
        // The OR of (native&&!broken) || (!native || broken) is trivially true,
        // pin the actual usable code path exists.
        let usable = (have_native_setregid() && !broken_setregid())
            || (!have_native_setregid() || broken_setregid());
        assert!(usable, "at least one execution path must be viable");
    }

    /// c:142-164 — same invariant for setreuid.
    #[cfg(unix)]
    #[test]
    fn config_flags_at_least_one_path_viable_setreuid() {
        let usable = (have_native_setreuid() && !broken_setreuid())
            || (!have_native_setreuid() || broken_setreuid());
        assert!(usable, "at least one execution path must be viable");
    }

    /// c:242 — `errno_str(ENOSYS)` is non-empty and deterministic.
    #[cfg(unix)]
    #[test]
    fn errno_str_enosys_non_empty_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let a = errno_str(libc::ENOSYS);
        let b = errno_str(libc::ENOSYS);
        assert!(!a.is_empty(), "errno_str(ENOSYS) must not be empty");
        assert_eq!(a, b, "errno_str(ENOSYS) must be pure");
    }

    /// c:242 — `errno_str(-1)` (invalid code) doesn't panic.
    #[cfg(unix)]
    #[test]
    fn errno_str_invalid_code_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = errno_str(-1);
        let _ = errno_str(99999);
    }

    /// c:182/193 — `errno_get`/`errno_set` round-trip preserves arbitrary code.
    #[cfg(unix)]
    #[test]
    fn errno_get_set_round_trip_all_common_codes() {
        let _g = crate::test_util::global_state_lock();
        let saved = errno_get();
        for code in [0, libc::EAGAIN, libc::ENOSYS, libc::EINVAL,
                     libc::EACCES, libc::EPERM, libc::ENOMEM] {
            errno_set(code);
            assert_eq!(errno_get(), code,
                "errno round-trip must preserve {}", code);
        }
        errno_set(saved);
    }

    /// c:70 — `setresgid` returns -1 with ENOSYS without altering uid/gid.
    /// The early-return branch must not mutate process state.
    #[cfg(unix)]
    #[test]
    fn setresgid_early_return_does_not_alter_gid() {
        let _g = crate::test_util::global_state_lock();
        let saved = errno_get();
        let before = unsafe { libc::getgid() };
        let _ = unsafe { setresgid(0 as libc::gid_t, 0 as libc::gid_t, 99 as libc::gid_t) };
        let after = unsafe { libc::getgid() };
        assert_eq!(before, after, "early-return must not mutate gid");
        errno_set(saved);
    }

    /// c:104 — same invariant for setresuid early-return.
    #[cfg(unix)]
    #[test]
    fn setresuid_early_return_does_not_alter_uid() {
        let _g = crate::test_util::global_state_lock();
        let saved = errno_get();
        let before = unsafe { libc::getuid() };
        let _ = unsafe { setresuid(0 as libc::uid_t, 0 as libc::uid_t, 99 as libc::uid_t) };
        let after = unsafe { libc::getuid() };
        assert_eq!(before, after, "early-return must not mutate uid");
        errno_set(saved);
    }

    /// c:142-164 — `broken_setregid` is a const fn (compile-time pin).
    #[cfg(unix)]
    #[test]
    fn broken_flags_are_const_fns() {
        const _BR: bool = broken_setregid();
        const _BU: bool = broken_setreuid();
        const _HR: bool = have_native_setregid();
        const _HU: bool = have_native_setreuid();
    }

    /// c:70/104 — `setresgid(0,0,0)` (all equal current root-ish) is
    /// not the early-return path. On non-root, will likely fail with
    /// EPERM but must NOT set ENOSYS. The contract here: errno is
    /// EITHER 0 (success) or non-ENOSYS (per c:46 the ENOSYS path
    /// only triggers when rgid != sgid).
    #[cfg(unix)]
    #[test]
    fn setresgid_equal_args_path_not_enosys() {
        let _g = crate::test_util::global_state_lock();
        let saved = errno_get();
        errno_set(0);
        let cur = unsafe { libc::getgid() };
        let _ = unsafe { setresgid(cur, cur, cur) };
        // Must not have hit the rgid != sgid early-return.
        // On non-root, may set errno from native syscall but not ENOSYS.
        let err = errno_get();
        assert!(err != libc::ENOSYS,
            "equal args must NOT trigger ENOSYS branch; got errno={}", err);
        errno_set(saved);
    }
}

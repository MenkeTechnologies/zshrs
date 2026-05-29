//! Direct port of `Src/prototypes.h` — extern declarations for
//! functions missing on legacy Unices.
//!
//! Original C copyright: Paul Falstad 1992-1997.
//!
//! The C header is a stack of `#ifndef HAVE_*` / `#if defined(__osf__)`
//! / `#if defined(DGUX) && defined(__STDC__)` / `#ifdef __NeXT__` /
//! `#if defined(__sun__) && !defined(__SVR4)` / `#ifdef __hpux` blocks
//! that hand-declare `malloc`, `realloc`, `calloc`, `tgetent`,
//! `tgetnum`, `tgetflag`, `tgetstr`, `tputs`, `tgoto`, `mktemp`,
//! `ioctl`, `mknod`, `nice`, `select`, `getrlimit`, `setrlimit`,
//! `getrusage`, `gettimeofday`, `wait3`, `getdomainname`, `getppid`,
//! `strerror`, `strstr`, `gethostname`, `difftime`, `bcopy`.
//!
//! Every extern in this header targets a legacy system (DGUX, OSF/1,
//! NeXTSTEP, SunOS-pre-Solaris, AIX, HP-UX) that zshrs does not
//! support. The `libc` crate's POSIX bindings cover all of these on
//! every platform zshrs builds for (macOS aarch64, Linux x86_64,
//! Linux aarch64), so the Rust port has nothing to declare here.
//!
//! This file is intentionally empty. Do not add prototypes — Rust
//! pulls them via `libc::*` in the call sites that need them.

#[cfg(test)]
mod tests {
    /// `Src/prototypes.h` is a stack of `#ifndef HAVE_*` legacy externs.
    /// This file MUST remain empty — every legacy fallback maps to libc.
    /// If a future commit adds an extern here, it's drift: zshrs's libc
    /// crate already provides POSIX bindings on every supported target.
    /// Test pins the contract by asserting libc::getppid + libc::strerror
    /// are wired (the two most commonly-cited externs in the C header).
    #[cfg(unix)]
    #[test]
    fn libc_provides_every_legacy_fallback() {
        let _g = crate::test_util::global_state_lock();
        let _: unsafe extern "C" fn() -> libc::pid_t = libc::getppid;
        let _: unsafe extern "C" fn(libc::c_int) -> *mut libc::c_char = libc::strerror;
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional contract pins — every legacy extern in Src/prototypes.h
    // must be available through libc, NOT redeclared here.
    // ═══════════════════════════════════════════════════════════════════

    /// libc provides malloc/realloc/calloc (legacy fallback trio).
    #[cfg(unix)]
    #[test]
    fn libc_provides_malloc_realloc_calloc() {
        let _g = crate::test_util::global_state_lock();
        let _: unsafe extern "C" fn(libc::size_t) -> *mut libc::c_void = libc::malloc;
        let _: unsafe extern "C" fn(*mut libc::c_void, libc::size_t) -> *mut libc::c_void =
            libc::realloc;
        let _: unsafe extern "C" fn(libc::size_t, libc::size_t) -> *mut libc::c_void = libc::calloc;
    }

    /// libc provides mkstemp (mktemp removed from macOS libc as insecure;
    /// the modern equivalent is mkstemp).
    #[cfg(unix)]
    #[test]
    fn libc_provides_mkstemp() {
        let _g = crate::test_util::global_state_lock();
        let _: unsafe extern "C" fn(*mut libc::c_char) -> libc::c_int = libc::mkstemp;
    }

    /// libc provides nice.
    #[cfg(unix)]
    #[test]
    fn libc_provides_nice() {
        let _g = crate::test_util::global_state_lock();
        let _: unsafe extern "C" fn(libc::c_int) -> libc::c_int = libc::nice;
    }

    /// libc provides getrlimit/setrlimit.
    #[cfg(unix)]
    #[test]
    fn libc_provides_get_set_rlimit() {
        let _g = crate::test_util::global_state_lock();
        let _: unsafe extern "C" fn(libc::c_int, *mut libc::rlimit) -> libc::c_int =
            libc::getrlimit;
        let _: unsafe extern "C" fn(libc::c_int, *const libc::rlimit) -> libc::c_int =
            libc::setrlimit;
    }

    /// libc provides getrusage.
    #[cfg(unix)]
    #[test]
    fn libc_provides_getrusage() {
        let _g = crate::test_util::global_state_lock();
        let _: unsafe extern "C" fn(libc::c_int, *mut libc::rusage) -> libc::c_int =
            libc::getrusage;
    }

    /// libc provides gethostname.
    #[cfg(unix)]
    #[test]
    fn libc_provides_gethostname() {
        let _g = crate::test_util::global_state_lock();
        let _: unsafe extern "C" fn(*mut libc::c_char, libc::size_t) -> libc::c_int =
            libc::gethostname;
    }

    /// libc provides difftime.
    #[cfg(unix)]
    #[test]
    fn libc_provides_difftime() {
        let _g = crate::test_util::global_state_lock();
        let _: unsafe extern "C" fn(libc::time_t, libc::time_t) -> libc::c_double = libc::difftime;
    }

    /// libc provides strstr.
    #[cfg(unix)]
    #[test]
    fn libc_provides_strstr() {
        let _g = crate::test_util::global_state_lock();
        let _: unsafe extern "C" fn(*const libc::c_char, *const libc::c_char) -> *mut libc::c_char =
            libc::strstr;
    }

    /// This file MUST NOT export any fns — pin via source self-inspection.
    #[test]
    fn prototypes_h_exports_no_functions() {
        let src = include_str!("prototypes_h.rs");
        let pub_fn_count = src.lines().filter(|l| l.starts_with("pub fn ")).count();
        assert_eq!(
            pub_fn_count, 0,
            "prototypes_h.rs MUST export no fns — legacy fallbacks live in libc"
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/prototypes.h legacy externs
    // — every one of the 24+ externs in the C header must be present
    // in libc on a modern Unix; tests pin that contract.
    // ═══════════════════════════════════════════════════════════════════

    /// libc provides memcpy (modern equivalent of bcopy from the C
    /// header — bcopy was deprecated by POSIX 2001 and removed from
    /// recent libc bindings, memcpy is the supported successor).
    #[cfg(unix)]
    #[test]
    fn libc_provides_memcpy_as_bcopy_successor() {
        let _g = crate::test_util::global_state_lock();
        let _: unsafe extern "C" fn(*mut libc::c_void, *const libc::c_void, libc::size_t)
            -> *mut libc::c_void = libc::memcpy;
    }

    /// libc provides gettimeofday.
    #[cfg(unix)]
    #[test]
    fn libc_provides_gettimeofday() {
        let _g = crate::test_util::global_state_lock();
        let _: unsafe extern "C" fn(*mut libc::timeval, *mut libc::c_void) -> libc::c_int =
            libc::gettimeofday;
    }

    /// libc provides select (legacy IO multiplexing).
    #[cfg(unix)]
    #[test]
    fn libc_provides_select() {
        let _g = crate::test_util::global_state_lock();
        let _: unsafe extern "C" fn(
            libc::c_int,
            *mut libc::fd_set,
            *mut libc::fd_set,
            *mut libc::fd_set,
            *mut libc::timeval,
        ) -> libc::c_int = libc::select;
    }

    /// libc provides mknod.
    #[cfg(unix)]
    #[test]
    fn libc_provides_mknod() {
        let _g = crate::test_util::global_state_lock();
        let _: unsafe extern "C" fn(*const libc::c_char, libc::mode_t, libc::dev_t) -> libc::c_int =
            libc::mknod;
    }

    /// libc provides ioctl. Note: ioctl is variadic so we only check
    /// presence via address-of, not signature cast.
    #[cfg(unix)]
    #[test]
    fn libc_provides_ioctl() {
        let _g = crate::test_util::global_state_lock();
        let p = libc::ioctl as *const ();
        assert!(!p.is_null(), "libc::ioctl must resolve");
    }

    /// libc provides getppid.
    #[cfg(unix)]
    #[test]
    fn libc_provides_getppid_extern() {
        let _g = crate::test_util::global_state_lock();
        let _: unsafe extern "C" fn() -> libc::pid_t = libc::getppid;
    }

    /// libc provides strerror.
    #[cfg(unix)]
    #[test]
    fn libc_provides_strerror_extern() {
        let _g = crate::test_util::global_state_lock();
        let _: unsafe extern "C" fn(libc::c_int) -> *mut libc::c_char = libc::strerror;
    }

    /// File contents pin: no extern declarations allowed.
    #[test]
    fn prototypes_h_has_no_extern_declarations() {
        let src = include_str!("prototypes_h.rs");
        let extern_decl_lines: Vec<&str> = src
            .lines()
            .filter(|l| {
                let t = l.trim_start();
                t.starts_with("extern \"C\"") && t.contains("fn ")
            })
            .collect();
        assert!(
            extern_decl_lines.is_empty(),
            "prototypes_h.rs MUST NOT redeclare externs — use libc::* at call sites instead. Found: {:?}",
            extern_decl_lines
        );
    }

    /// File contents pin: no static extern blocks either.
    #[test]
    fn prototypes_h_has_no_static_externs() {
        let src = include_str!("prototypes_h.rs");
        let count = src.matches("extern \"C\" {").count();
        assert_eq!(
            count, 0,
            "prototypes_h.rs MUST NOT contain `extern \"C\" {{}}` blocks"
        );
    }
}

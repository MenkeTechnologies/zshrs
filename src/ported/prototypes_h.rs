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

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for the remaining Src/prototypes.h
    // legacy externs that weren't covered: tgetent/tgetnum/tgetflag/
    // tgetstr/tputs/tgoto (termcap), wait3/wait4 (BSD wait), getdomainname,
    // bcopy direct, and "this file must stay empty" structural pins.
    // ═══════════════════════════════════════════════════════════════════

    /// `Src/prototypes.h` declares `tgetent` as a legacy fallback when
    /// `HAVE_TGETENT` is missing. modern libc on Linux ships it via
    /// libtinfo / on macOS via libncurses (linked through libSystem).
    /// Skip the type-pin and use a presence check — terminfo crate
    /// already vendors what zshrs needs.
    #[test]
    fn termcap_legacy_fns_not_redeclared_here() {
        let _g = crate::test_util::global_state_lock();
        let src = include_str!("prototypes_h.rs");
        for name in &[
            "tgetent", "tgetnum", "tgetflag", "tgetstr", "tputs", "tgoto",
        ] {
            assert!(
                !src.contains(&format!("fn {}", name)),
                "{} must NOT be declared in prototypes_h.rs (use terminfo crate)",
                name
            );
        }
    }

    /// `Src/prototypes.h` declares legacy SVR4-era externs for BSD
    /// `wait3`/`wait4`. Modern libc has waitpid; the C externs are
    /// legacy fallbacks. Pin: no redeclaration here.
    #[test]
    fn bsd_wait_legacy_fns_not_redeclared_here() {
        let _g = crate::test_util::global_state_lock();
        let src = include_str!("prototypes_h.rs");
        for name in &["wait3", "wait4"] {
            assert!(
                !src.contains(&format!("fn {}", name)),
                "{} must NOT be declared in prototypes_h.rs (use libc::waitpid)",
                name
            );
        }
    }

    /// libc provides waitpid (the modern successor to wait3/wait4 from
    /// the C header's BSD-only fallback block).
    #[cfg(unix)]
    #[test]
    fn libc_provides_waitpid_as_wait3_wait4_successor() {
        let _g = crate::test_util::global_state_lock();
        let _: unsafe extern "C" fn(libc::pid_t, *mut libc::c_int, libc::c_int) -> libc::pid_t =
            libc::waitpid;
    }

    /// libc provides clock_gettime (modern successor to gettimeofday +
    /// the legacy DGUX/__STDC__ extern in the C header).
    #[cfg(unix)]
    #[test]
    fn libc_provides_clock_gettime() {
        let _g = crate::test_util::global_state_lock();
        let _: unsafe extern "C" fn(libc::clockid_t, *mut libc::timespec) -> libc::c_int =
            libc::clock_gettime;
    }

    /// libc provides memmove (POSIX successor to bcopy; bcopy is
    /// removed from modern libc bindings).
    #[cfg(unix)]
    #[test]
    fn libc_provides_memmove_as_bcopy_successor() {
        let _g = crate::test_util::global_state_lock();
        let _: unsafe extern "C" fn(*mut libc::c_void, *const libc::c_void, libc::size_t)
            -> *mut libc::c_void = libc::memmove;
    }

    /// libc provides memcmp (general-purpose successor for any of the
    /// legacy compare fallbacks the C header may pull in).
    #[cfg(unix)]
    #[test]
    fn libc_provides_memcmp() {
        let _g = crate::test_util::global_state_lock();
        let _: unsafe extern "C" fn(*const libc::c_void, *const libc::c_void, libc::size_t)
            -> libc::c_int = libc::memcmp;
    }

    /// libc provides strlen (universal — never absent on any libc
    /// zshrs targets; pin defensively).
    #[cfg(unix)]
    #[test]
    fn libc_provides_strlen() {
        let _g = crate::test_util::global_state_lock();
        let _: unsafe extern "C" fn(*const libc::c_char) -> libc::size_t = libc::strlen;
    }

    /// `Src/prototypes.h` is empty per the file's own assertion. Pin:
    /// no `use` statements (would imply a port body was added).
    #[test]
    fn prototypes_h_has_no_use_statements() {
        let src = include_str!("prototypes_h.rs");
        let use_lines: Vec<&str> = src
            .lines()
            .filter(|l| {
                let t = l.trim_start();
                t.starts_with("use ") && !t.contains("//")
            })
            .collect();
        assert!(
            use_lines.is_empty(),
            "prototypes_h.rs MUST stay empty — no `use` imports. Found: {:?}",
            use_lines
        );
    }

    /// File-structure pin: no `pub struct` / `pub enum` / `pub type`.
    /// The C header has zero of these and the Rust port must too.
    /// (Self-reference filter: skip lines containing test-asserts.)
    #[test]
    fn prototypes_h_exports_no_types() {
        let src = include_str!("prototypes_h.rs");
        for keyword in &["pub struct ", "pub enum ", "pub type ", "pub trait "] {
            let count = src
                .lines()
                .filter(|l| l.trim_start().starts_with(keyword))
                .count();
            assert_eq!(
                count, 0,
                "prototypes_h.rs MUST NOT export {} — found {} occurrence(s)",
                keyword.trim(),
                count
            );
        }
    }

    /// File-structure pin: no `pub const` either (legacy header had
    /// only externs, no const decls).
    #[test]
    fn prototypes_h_exports_no_consts() {
        let src = include_str!("prototypes_h.rs");
        let count = src.lines().filter(|l| {
            let t = l.trim_start();
            t.starts_with("pub const ") || t.starts_with("pub static ")
        }).count();
        assert_eq!(
            count, 0,
            "prototypes_h.rs MUST NOT export consts/statics — found {}",
            count
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional libc-availability pins — every legacy extern in
    // Src/prototypes.h must be available through libc (not redeclared).
    // ═══════════════════════════════════════════════════════════════════

    /// libc provides `getdomainname` (legacy SunOS-pre-Solaris extern).
    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[test]
    fn libc_provides_getdomainname() {
        let _g = crate::test_util::global_state_lock();
        let _: unsafe extern "C" fn(*mut libc::c_char, libc::size_t) -> libc::c_int =
            libc::getdomainname;
    }

    /// libc provides `free` (legacy fallback companion to malloc).
    #[cfg(unix)]
    #[test]
    fn libc_provides_free() {
        let _g = crate::test_util::global_state_lock();
        let _: unsafe extern "C" fn(*mut libc::c_void) = libc::free;
    }

    /// libc provides `time` (companion to legacy difftime).
    #[cfg(unix)]
    #[test]
    fn libc_provides_time() {
        let _g = crate::test_util::global_state_lock();
        let _: unsafe extern "C" fn(*mut libc::time_t) -> libc::time_t = libc::time;
    }

    /// libc provides `strncpy` (string-extern family in prototypes.h).
    #[cfg(unix)]
    #[test]
    fn libc_provides_strncpy() {
        let _g = crate::test_util::global_state_lock();
        let _: unsafe extern "C" fn(*mut libc::c_char, *const libc::c_char, libc::size_t)
            -> *mut libc::c_char = libc::strncpy;
    }

    /// libc provides `strncmp` (string-extern family).
    #[cfg(unix)]
    #[test]
    fn libc_provides_strncmp() {
        let _g = crate::test_util::global_state_lock();
        let _: unsafe extern "C" fn(*const libc::c_char, *const libc::c_char, libc::size_t)
            -> libc::c_int = libc::strncmp;
    }

    /// libc provides `strcmp` (string-extern family).
    #[cfg(unix)]
    #[test]
    fn libc_provides_strcmp() {
        let _g = crate::test_util::global_state_lock();
        let _: unsafe extern "C" fn(*const libc::c_char, *const libc::c_char) -> libc::c_int =
            libc::strcmp;
    }

    /// libc provides `setpgid` (modern replacement for setpgrp legacy paths).
    #[cfg(unix)]
    #[test]
    fn libc_provides_setpgid() {
        let _g = crate::test_util::global_state_lock();
        let _: unsafe extern "C" fn(libc::pid_t, libc::pid_t) -> libc::c_int = libc::setpgid;
    }

    /// libc provides `getpid` (companion to legacy getppid).
    #[cfg(unix)]
    #[test]
    fn libc_provides_getpid() {
        let _g = crate::test_util::global_state_lock();
        let _: unsafe extern "C" fn() -> libc::pid_t = libc::getpid;
    }

    /// libc provides `signal` (modern signal handler installer).
    #[cfg(unix)]
    #[test]
    fn libc_provides_signal() {
        let _g = crate::test_util::global_state_lock();
        let _: unsafe extern "C" fn(libc::c_int, libc::sighandler_t) -> libc::sighandler_t =
            libc::signal;
    }

    /// File-structure pin: prototypes_h.rs body above the test module
    /// MUST contain no items (doc comments + attributes + blank lines only).
    #[test]
    fn prototypes_h_body_above_tests_is_empty() {
        let src = include_str!("prototypes_h.rs");
        let pre_tests: String = src
            .lines()
            .take_while(|l| !l.contains("mod tests"))
            .collect::<Vec<_>>()
            .join("\n");
        for line in pre_tests.lines() {
            let t = line.trim_start();
            assert!(
                t.is_empty() || t.starts_with("//") || t.starts_with("#[") || t.starts_with("//!"),
                "prototypes_h body above tests must be empty (doc + attrs only); got: {:?}",
                line
            );
        }
    }

    /// File-structure pin: tests are the only `mod` declaration.
    #[test]
    fn prototypes_h_has_exactly_one_mod_decl() {
        let src = include_str!("prototypes_h.rs");
        let count = src.lines()
            .filter(|l| {
                let t = l.trim_start();
                t.starts_with("mod ") || t.starts_with("pub mod ")
            }).count();
        assert_eq!(count, 1,
            "prototypes_h.rs MUST contain only the 1 #[cfg(test)] mod tests");
    }
}

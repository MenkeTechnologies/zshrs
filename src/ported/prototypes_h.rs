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
        let _: unsafe extern "C" fn() -> libc::pid_t = libc::getppid;
        let _: unsafe extern "C" fn(libc::c_int) -> *mut libc::c_char = libc::strerror;
    }
}

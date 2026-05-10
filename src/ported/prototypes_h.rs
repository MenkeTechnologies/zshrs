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

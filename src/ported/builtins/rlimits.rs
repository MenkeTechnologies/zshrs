//! Resource limits — port of `Src/Builtins/rlimits.c`.
//!
//! Implements the `limit`, `ulimit`, and `unlimit` builtins.
//!
//! Structure mirrors the C source line-by-line:
//!   - `enum zlimtype` (rlimits.c:35)
//!   - `struct resinfo_T` (rlimits.c:43)
//!   - `static known_resources[]` (rlimits.c:60)
//!   - `static const resinfo_T **resinfo` (rlimits.c:190)
//!   - `set_resinfo` / `free_resinfo` / `find_resource` /
//!     `printrlim` / `zstrtorlimt` / `showlimitvalue` /
//!     `showlimits` / `printulimit` / `do_limit` / `bin_limit` /
//!     `do_unlimit` / `bin_unlimit` / `bin_ulimit`
//!   - module entries (`setup_`, `features_`, `enables_`, `boot_`,
//!     `cleanup_`, `finish_`)

#![allow(non_camel_case_types)]
#![allow(non_upper_case_globals)]
#![allow(non_snake_case)]
// `RLIMIT_*` constants are typed `__rlimit_resource_t` (= u32) on glibc
// Linux and `c_int` (= i32) on macOS / *BSD. The `as i32` casts are
// portable but redundant on whichever platform the type already
// matches — silence clippy's per-platform unnecessary_cast firing.
#![allow(clippy::unnecessary_cast)]

use std::sync::{Mutex, OnceLock};

#[cfg(unix)]
use libc::{
    geteuid, getrlimit, rlim_t, rlimit, setrlimit, RLIMIT_AS, RLIMIT_CORE, RLIMIT_CPU,
    RLIMIT_DATA, RLIMIT_FSIZE, RLIMIT_NOFILE, RLIMIT_STACK, RLIM_INFINITY, RLIM_NLIMITS,
};

use crate::ported::utils::{zstrtol, zwarnnam};
use crate::ported::zsh_h::{module, options, OPT_ISSET};

// =====================================================================
// Port of `enum zlimtype` from Src/Builtins/rlimits.c:35.
// =====================================================================

/// Tags each `RLIMIT_*` with the scaling unit `printrlim()` (line 253)
/// and `zstrtorlimt()` (line 272) interpret it under: KB-scaled
/// memory, raw count, seconds, microseconds, or "unknown" for
/// resources the build doesn't recognize.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum zlimtype {
    ZLIMTYPE_MEMORY,
    ZLIMTYPE_NUMBER,
    ZLIMTYPE_TIME,
    ZLIMTYPE_MICROSECONDS,
    ZLIMTYPE_UNKNOWN,
}

// =====================================================================
// Port of `typedef struct resinfo_T` from Src/Builtins/rlimits.c:43.
// =====================================================================

/// Per-resource metadata. C definition:
///
/// ```c
/// typedef struct resinfo_T {
///     int     res;       /* RLIMIT_XXX */
///     char*   name;      /* used by limit builtin */
///     enum zlimtype type;
///     int     unit;      /* 1, 512, or 1024 */
///     char    opt;       /* option character */
///     char*   descr;     /* used by ulimit builtin */
/// } resinfo_T;
/// ```
///
/// Rust port renames `type` (Rust keyword) to `r#type`.
#[derive(Debug, Clone)]
pub struct resinfo_T {
    pub res: i32,
    pub name: &'static str,
    pub r#type: zlimtype,
    pub unit: u32,
    pub opt: char,
    pub descr: &'static str,
}

// =====================================================================
// Port of `static const resinfo_T known_resources[]` (rlimits.c:60).
// =====================================================================

/// Table of known resources. C source uses `# ifdef` preprocessor
/// conditionals per platform; the Rust port limits the table to the
/// subset libc exposes on macOS / glibc Linux (CPU, FSIZE, DATA,
/// STACK, CORE, NOFILE, AS).
#[cfg(unix)]
pub static known_resources: &[resinfo_T] = &[
    resinfo_T {
        res: RLIMIT_CPU as i32,
        name: "cputime",
        r#type: zlimtype::ZLIMTYPE_TIME,
        unit: 1,
        opt: 't',
        descr: "cpu time (seconds)",
    },
    resinfo_T {
        res: RLIMIT_FSIZE as i32,
        name: "filesize",
        r#type: zlimtype::ZLIMTYPE_MEMORY,
        unit: 512,
        opt: 'f',
        descr: "file size (blocks)",
    },
    resinfo_T {
        res: RLIMIT_DATA as i32,
        name: "datasize",
        r#type: zlimtype::ZLIMTYPE_MEMORY,
        unit: 1024,
        opt: 'd',
        descr: "data seg size (kbytes)",
    },
    resinfo_T {
        res: RLIMIT_STACK as i32,
        name: "stacksize",
        r#type: zlimtype::ZLIMTYPE_MEMORY,
        unit: 1024,
        opt: 's',
        descr: "stack size (kbytes)",
    },
    resinfo_T {
        res: RLIMIT_CORE as i32,
        name: "coredumpsize",
        r#type: zlimtype::ZLIMTYPE_MEMORY,
        unit: 512,
        opt: 'c',
        descr: "core file size (blocks)",
    },
    resinfo_T {
        res: RLIMIT_NOFILE as i32,
        name: "descriptors",
        r#type: zlimtype::ZLIMTYPE_NUMBER,
        unit: 1,
        opt: 'n',
        descr: "file descriptors",
    },
    resinfo_T {
        res: RLIMIT_AS as i32,
        name: "addressspace",
        r#type: zlimtype::ZLIMTYPE_MEMORY,
        unit: 1024,
        opt: 'v',
        descr: "address space (kbytes)",
    },
];

#[cfg(not(unix))]
pub static known_resources: &[resinfo_T] = &[];

// =====================================================================
// Module-static state. Mirrors C `Src/Builtins/rlimits.c` and
// extern globals from `Src/exec.c:310`.
//
// Per PORT_PLAN.md these are bucket 2 (shell-wide shared globals) —
// a parallel `ulimit` from a worker thread must see the same table
// the foreground evaluator does. Hence `OnceLock<Mutex<…>>`, not
// `thread_local!`.
// =====================================================================

/// Port of `static const resinfo_T **resinfo` from rlimits.c:190.
/// Index by `RLIMIT_*` value to get the matching `resinfo_T`.
/// Populated by `set_resinfo()`, freed by `free_resinfo()`.
static RESINFO: OnceLock<Mutex<Vec<resinfo_T>>> = OnceLock::new();

/// Port of `mod_export struct rlimit current_limits[RLIM_NLIMITS]`
/// from `Src/exec.c:310`. Snapshot of the shell's resource limits as
/// of `init_main()` (Src/init.c:1287).
static CURRENT_LIMITS: OnceLock<Mutex<Vec<rlimit>>> = OnceLock::new();

/// Port of `mod_export struct rlimit limits[RLIM_NLIMITS]` from
/// `Src/exec.c:310`. The user-visible limits the next `setrlimit()`
/// will install. `setlimits(nam)` (Src/exec.c:331) flushes them via
/// `zsetlimit()`.
static LIMITS: OnceLock<Mutex<Vec<rlimit>>> = OnceLock::new();

#[cfg(unix)]
fn nlimits() -> usize {
    RLIM_NLIMITS as usize
}

#[cfg(not(unix))]
fn nlimits() -> usize {
    0
}

// WARNING: NOT IN RLIMITS.C — Rust-only initializer. C performs the
// equivalent rlimit-snapshot inside `init_main()` (Src/init.c:1287);
// the Rust port factors it out so `set_resinfo()`, `bin_limit()`,
// etc. can lazily seed `LIMITS`/`CURRENT_LIMITS` on first use without
// requiring a separate init pass. Allowlisted in
// tests/data/ported_fn_allowlist.txt.
#[cfg(unix)]
fn ensure_limits_initialized() {
    let init = || {
        let mut v: Vec<rlimit> = Vec::with_capacity(nlimits());
        for i in 0..nlimits() as i32 {
            let mut r = rlimit {
                rlim_cur: 0,
                rlim_max: 0,
            };
            unsafe {
                getrlimit(i as _, &mut r);
            }
            v.push(r);
        }
        v
    };
    LIMITS.get_or_init(|| Mutex::new(init()));
    CURRENT_LIMITS.get_or_init(|| Mutex::new(init()));
}

#[cfg(not(unix))]
fn ensure_limits_initialized() {}

// =====================================================================
// Port of `set_resinfo()` from Src/Builtins/rlimits.c:194.
// =====================================================================

/// Port of `set_resinfo()` from `Src/Builtins/rlimits.c:194`.
///
/// Build the `RESINFO` table indexed by `RLIMIT_*` value. Entries
/// for resources not in `known_resources[]` get a synthesized
/// `UNKNOWN-N` placeholder so `printulimit()` / `showlimitvalue()`
/// can format unknown limits without a NULL dereference.
pub(crate) fn set_resinfo() {
    RESINFO.get_or_init(|| {
        let mut v: Vec<resinfo_T> = Vec::with_capacity(nlimits());
        for i in 0..nlimits() as i32 {
            let entry = known_resources
                .iter()
                .find(|r| r.res == i)
                .cloned()
                .unwrap_or_else(|| resinfo_T {
                    res: -1,
                    name: leak_unknown_name(i),
                    r#type: zlimtype::ZLIMTYPE_UNKNOWN,
                    unit: 1,
                    opt: 'N',
                    descr: leak_unknown_name(i),
                });
            v.push(entry);
        }
        Mutex::new(v)
    });
}

// WARNING: NOT IN RLIMITS.C — Rust-only helper. C `set_resinfo()`
// (rlimits.c:206-214) writes the synthesized `UNKNOWN-N` text into a
// `zalloc(12)` heap buffer; Rust port leaks a `Box<str>` since
// `&'static str` is required for the `name`/`descr` fields and
// allocation is a one-shot per resource at module-init.
fn leak_unknown_name(i: i32) -> &'static str {
    Box::leak(format!("UNKNOWN-{}", i).into_boxed_str())
}

// =====================================================================
// Port of `free_resinfo()` from Src/Builtins/rlimits.c:222.
// =====================================================================

/// Port of `free_resinfo()` from `Src/Builtins/rlimits.c:222`.
///
/// C frees the heap-allocated `UNKNOWN-N` placeholders and the
/// `resinfo` pointer table, leaving the static `known_resources[]`
/// alone. Rust port: clears the `RESINFO` Mutex contents (the leaked
/// `&'static str`s remain leaked — there's no tracked-list-of-leaks
/// to free, matching C semantics where `cleanup_()` is only called
/// at module-unload during shell exit anyway).
pub(crate) fn free_resinfo() {
    if let Some(lock) = RESINFO.get() {
        if let Ok(mut v) = lock.lock() {
            v.clear();
        }
    }
}

// =====================================================================
// Port of `find_resource()` from Src/Builtins/rlimits.c:239.
// =====================================================================

// Find resource by its option character                                    // c:235
/// Port of `find_resource()` from `Src/Builtins/rlimits.c:239`.
///
/// Find a resource by its `ulimit` option character. Returns the
/// `RLIMIT_*` index, or `-1` on miss.
pub(crate) fn find_resource(c: char) -> i32 {
    set_resinfo();
    let lock = match RESINFO.get() {
        Some(l) => l,
        None => return -1,
    };
    let v = match lock.lock() {
        Ok(v) => v,
        Err(_) => return -1,
    };
    for (i, info) in v.iter().enumerate() {
        if info.opt == c {
            return i as i32;
        }
    }
    -1
}

// =====================================================================
// Port of `printrlim()` from Src/Builtins/rlimits.c:253.
// =====================================================================

// Print a value of type rlim_t                                             // c:249
/// Port of `printrlim()` from `Src/Builtins/rlimits.c:253`.
///
/// Print a `rlim_t` value with the supplied unit suffix to stdout.
/// C selects between `%qd` / `%lld` / `%lu` / `%ld` per
/// `RLIM_T_IS_*` macro; Rust port uses unified `Display` since
/// `rlim_t` is `u64` on macOS and Linux glibc.
pub(crate) fn printrlim(val: rlim_t, unit: &str) {
    print!("{}{}", val, unit);
}

// =====================================================================
// Port of `zstrtorlimt()` from Src/Builtins/rlimits.c:272.
// =====================================================================

/// Port of `zstrtorlimt()` from `Src/Builtins/rlimits.c:272`.
///
/// Parse a numeric limit string. Returns `(value, bytes_consumed)`.
/// Recognises `unlimited` as `RLIM_INFINITY`; for digit input,
/// honours the same base-detection (`0x` hex, `0` octal, else 10)
/// that C's `RLIM_T_IS_QUAD_T`/`RLIM_T_IS_LONG_LONG`/
/// `RLIM_T_IS_UNSIGNED` paths do.
#[cfg(unix)]
pub(crate) fn zstrtorlimt(s: &str, base: i32) -> (rlim_t, usize) {
    if s == "unlimited" || s.starts_with("unlimited") {
        return (RLIM_INFINITY, "unlimited".len());
    }
    let bytes = s.as_bytes();
    let mut pos: usize = 0;
    let mut base = base;
    if base == 0 {
        if pos < bytes.len() && bytes[pos] != b'0' {
            base = 10;
        } else {
            pos += 1;
            if pos < bytes.len() && (bytes[pos] == b'x' || bytes[pos] == b'X') {
                base = 16;
                pos += 1;
            } else {
                base = 8;
            }
        }
    }
    let mut ret: rlim_t = 0;
    if base <= 10 {
        while pos < bytes.len() {
            let c = bytes[pos];
            if !(c >= b'0' && c < b'0' + base as u8) {
                break;
            }
            ret = ret * base as rlim_t + (c - b'0') as rlim_t;
            pos += 1;
        }
    } else {
        while pos < bytes.len() {
            let c = bytes[pos];
            let is_digit = c.is_ascii_digit();
            let is_hex_lower = c >= b'a' && c < b'a' + (base as u8 - 10);
            let is_hex_upper = c >= b'A' && c < b'A' + (base as u8 - 10);
            if !(is_digit || is_hex_lower || is_hex_upper) {
                break;
            }
            let digit = if is_digit {
                (c - b'0') as rlim_t
            } else {
                ((c & 0x1f) + 9) as rlim_t
            };
            ret = ret * base as rlim_t + digit;
            pos += 1;
        }
    }
    (ret, pos)
}

#[cfg(not(unix))]
pub(crate) fn zstrtorlimt(_s: &str, _base: i32) -> (u64, usize) {
    (0, 0)
}

// =====================================================================
// Port of `showlimitvalue()` from Src/Builtins/rlimits.c:307.
// =====================================================================

/// Port of `showlimitvalue()` from `Src/Builtins/rlimits.c:307`.
///
/// Print one limit row: 16-column-padded resource name, then the
/// value formatted per the resource's `zlimtype` (time as
/// `H:MM:SS`, microseconds as `Nus`, memory as `kB`/`MB`, plain
/// numeric otherwise).
#[cfg(unix)]
pub(crate) fn showlimitvalue(lim: i32, val: rlim_t) {
    set_resinfo();
    let info = lookup_resinfo(lim);
    if (lim as usize) < nlimits() {
        if let Some(info) = info.as_ref() {
            print!("{:<16}", info.name);
        } else {
            print!("{:<16}", lim);
        }
    } else {
        print!("{:<16}", lim);
    }
    if val == RLIM_INFINITY {
        println!("unlimited");
    } else if (lim as usize) >= nlimits() {
        printrlim(val, "\n");
    } else if let Some(info) = info {
        match info.r#type {
            zlimtype::ZLIMTYPE_TIME => {
                println!(
                    "{}:{:02}:{:02}",
                    val / 3600,
                    (val / 60) % 60,
                    val % 60
                );
            }
            zlimtype::ZLIMTYPE_MICROSECONDS => printrlim(val, "us\n"),
            zlimtype::ZLIMTYPE_NUMBER | zlimtype::ZLIMTYPE_UNKNOWN => printrlim(val, "\n"),
            zlimtype::ZLIMTYPE_MEMORY => {
                if val >= 1024 * 1024 {
                    printrlim(val / (1024 * 1024), "MB\n");
                } else {
                    printrlim(val / 1024, "kB\n");
                }
            }
        }
    } else {
        printrlim(val, "\n");
    }
}

#[cfg(not(unix))]
pub(crate) fn showlimitvalue(_lim: i32, _val: u64) {}

// WARNING: NOT IN RLIMITS.C — Rust-only helper. C dereferences the
// `resinfo` pointer-table inline (`resinfo[lim]->name` etc.); Rust
// port factors out the bounds-checked + lock-acquired lookup so the
// rlimit-printing fns don't each duplicate the lock dance.
#[cfg(unix)]
fn lookup_resinfo(lim: i32) -> Option<resinfo_T> {
    if (lim as usize) >= nlimits() {
        return None;
    }
    set_resinfo();
    let lock = RESINFO.get()?;
    let v = lock.lock().ok()?;
    v.get(lim as usize).cloned()
}

// =====================================================================
// Port of `showlimits()` from Src/Builtins/rlimits.c:346.
// =====================================================================

/// Port of `showlimits()` from `Src/Builtins/rlimits.c:346`.
///
/// `lim == -1` means show all; `lim >= RLIM_NLIMITS` falls back to a
/// direct `getrlimit(2)` for resources the table doesn't know.
#[cfg(unix)]
pub(crate) fn showlimits(nam: &str, hard: bool, lim: i32) -> i32 {          // c:346
    ensure_limits_initialized();
    if (lim as usize) >= nlimits() && lim != -1 {
        let mut vals = rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        if unsafe { getrlimit(lim as _, &mut vals) } < 0 {
            zwarnnam(nam, &format!("can't read limit: {}", std::io::Error::last_os_error()));
            return 1;
        }
        showlimitvalue(lim, if hard { vals.rlim_max } else { vals.rlim_cur });
    } else if lim != -1 {
        let limits = match LIMITS.get() {
            Some(l) => l,
            None => return 1,
        };
        let v = limits.lock().unwrap();
        let r = &v[lim as usize];
        showlimitvalue(lim, if hard { r.rlim_max } else { r.rlim_cur });
    } else {
        let limits = match LIMITS.get() {
            Some(l) => l,
            None => return 1,
        };
        let v = limits.lock().unwrap();
        for (rt, r) in v.iter().enumerate() {
            showlimitvalue(rt as i32, if hard { r.rlim_max } else { r.rlim_cur });
        }
    }
    0
}

#[cfg(not(unix))]
pub(crate) fn showlimits(_nam: &str, _hard: bool, _lim: i32) -> i32 {
    0
}

// =====================================================================
// Port of `printulimit()` from Src/Builtins/rlimits.c:386.
// =====================================================================

/// Port of `printulimit()` from `Src/Builtins/rlimits.c:386`.
///
/// `ulimit`-style display. `head` controls whether to emit the
/// `-X: descr` heading column. `lim >= RLIM_NLIMITS` falls back to
/// a direct `getrlimit(2)`.
#[cfg(unix)]
pub(crate) fn printulimit(nam: &str, lim: i32, hard: bool, head: bool) -> i32 { // c:386
    ensure_limits_initialized();
    let limit: rlim_t = if (lim as usize) >= nlimits() {
        let mut vals = rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        if unsafe { getrlimit(lim as _, &mut vals) } < 0 {
            zwarnnam(nam, &format!("can't read limit: {}", std::io::Error::last_os_error()));
            return 1;
        }
        if hard { vals.rlim_max } else { vals.rlim_cur }
    } else {
        let limits = match LIMITS.get() {
            Some(l) => l,
            None => return 1,
        };
        let v = limits.lock().unwrap();
        let r = &v[lim as usize];
        if hard { r.rlim_max } else { r.rlim_cur }
    };
    if head {
        if (lim as usize) < nlimits() {
            if let Some(info) = lookup_resinfo(lim) {
                if info.opt == 'N' {
                    print!("-N {:>2}: {:<29}", lim, info.descr);
                } else {
                    print!("-{}: {:<32}", info.opt, info.descr);
                }
            } else {
                print!("-N {:>2}: {:<29}", lim, "");
            }
        } else {
            print!("-N {:>2}: {:<29}", lim, "");
        }
    }
    if limit == RLIM_INFINITY {
        println!("unlimited");
    } else if (lim as usize) < nlimits() {
        let info = lookup_resinfo(lim);
        let unit = info.map(|i| i.unit).unwrap_or(1) as rlim_t;
        printrlim(limit / unit, "\n");
    } else {
        printrlim(limit, "\n");
    }
    0
}

#[cfg(not(unix))]
pub(crate) fn printulimit(_nam: &str, _lim: i32, _hard: bool, _head: bool) -> i32 {
    0
}

// =====================================================================
// Port of `do_limit()` from Src/Builtins/rlimits.c:431.
// =====================================================================

/// Port of `do_limit()` from `Src/Builtins/rlimits.c:431`.
///
/// Apply `val` to resource `lim` per the `hard`/`soft`/`set` flags.
/// `set` corresponds to `OPT_ISSET(ops, 's')` — when false, the
/// limits-array is updated but `setrlimit(2)` is not called.
#[cfg(unix)]
pub(crate) fn do_limit(                                                     // c:431
    nam: &str,
    lim: i32,
    val: rlim_t,
    hard: bool,
    soft: bool,
    set: bool,
) -> i32 {
    ensure_limits_initialized();
    if (lim as usize) >= nlimits() {
        let mut vals = rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        if unsafe { getrlimit(lim as _, &mut vals) } < 0 {
            zwarnnam(nam, &format!("can't read limit: {}", std::io::Error::last_os_error()));
            return 1;
        }
        if hard {
            if val > vals.rlim_max && unsafe { geteuid() } != 0 {
                zwarnnam(nam, "can't raise hard limits");
                return 1;
            }
            vals.rlim_max = val;
            if val < vals.rlim_cur {
                vals.rlim_cur = val;
            }
        }
        if soft || !hard {
            if val > vals.rlim_max {
                zwarnnam(nam, "limit exceeds hard limit");
                return 1;
            }
            vals.rlim_cur = val;
        }
        if !set {
            zwarnnam(nam, &format!("warning: unrecognised limit {}, use -s to set", lim));
            return 1;
        } else if unsafe { setrlimit(lim as _, &vals) } < 0 {
            zwarnnam(nam, &format!("setrlimit failed: {}", std::io::Error::last_os_error()));
            return 1;
        }
    } else {
        let cur_max = CURRENT_LIMITS
            .get()
            .and_then(|l| l.lock().ok())
            .map(|v| v[lim as usize].rlim_max)
            .unwrap_or(0);
        if hard {
            if val > cur_max && unsafe { geteuid() } != 0 {
                zwarnnam(nam, "can't raise hard limits");
                return 1;
            }
            if let Some(lock) = LIMITS.get() {
                let mut v = lock.lock().unwrap();
                v[lim as usize].rlim_max = val;
                if val < v[lim as usize].rlim_cur {
                    v[lim as usize].rlim_cur = val;
                }
            }
        }
        if soft || !hard {
            let cur_lim_max = LIMITS
                .get()
                .and_then(|l| l.lock().ok())
                .map(|v| v[lim as usize].rlim_max)
                .unwrap_or(0);
            if val > cur_lim_max {
                if nam.starts_with('u') {
                    if val > cur_max && unsafe { geteuid() } != 0 {
                        zwarnnam(nam, "value exceeds hard limit");
                        return 1;
                    }
                    if let Some(lock) = LIMITS.get() {
                        let mut v = lock.lock().unwrap();
                        v[lim as usize].rlim_max = val;
                        v[lim as usize].rlim_cur = val;
                    }
                } else {
                    zwarnnam(nam, "limit exceeds hard limit");
                    return 1;
                }
            } else if let Some(lock) = LIMITS.get() {
                let mut v = lock.lock().unwrap();
                v[lim as usize].rlim_cur = val;
            }
            if set && zsetlimit(lim, nam) != 0 {
                return 1;
            }
        }
    }
    0
}

#[cfg(not(unix))]
pub(crate) fn do_limit(
    _nam: &str,
    _lim: i32,
    _val: u64,
    _hard: bool,
    _soft: bool,
    _set: bool,
) -> i32 {
    0
}

// WARNING: NOT IN RLIMITS.C — `zsetlimit` lives in `Src/exec.c:314`,
// not `rlimits.c`. The Rust port colocates the helper here because
// rlimits.rs is the only consumer; the eventual exec.rs port can
// move it. Inlines the C body: if LIMITS[i] differs from
// CURRENT_LIMITS[i], call `setrlimit`, then sync CURRENT_LIMITS[i]
// back to LIMITS[i].
#[cfg(unix)]
fn zsetlimit(limnum: i32, nam: &str) -> i32 {
    ensure_limits_initialized();
    let limits_lock = match LIMITS.get() {
        Some(l) => l,
        None => return 0,
    };
    let cur_lock = match CURRENT_LIMITS.get() {
        Some(l) => l,
        None => return 0,
    };
    let limits = limits_lock.lock().unwrap();
    let limits_v = limits[limnum as usize];
    drop(limits);
    let cur = cur_lock.lock().unwrap();
    let cur_v = cur[limnum as usize];
    drop(cur);
    if limits_v.rlim_max != cur_v.rlim_max || limits_v.rlim_cur != cur_v.rlim_cur {
        if unsafe { setrlimit(limnum as _, &limits_v) } < 0 {
            zwarnnam(nam, &format!("setrlimit failed: {}", std::io::Error::last_os_error()));
            // restore in-memory copy from current
            let mut limits = limits_lock.lock().unwrap();
            limits[limnum as usize] = cur_v;
            return 1;
        }
        let mut cur = cur_lock.lock().unwrap();
        cur[limnum as usize] = limits_v;
    }
    0
}

#[cfg(not(unix))]
fn zsetlimit(_limnum: i32, _nam: &str) -> i32 {
    0
}

// WARNING: NOT IN RLIMITS.C — `setlimits` lives in `Src/exec.c:331`.
// Loops over RLIM_NLIMITS and calls `zsetlimit()` on each. Same
// rationale as `zsetlimit` above.
#[cfg(unix)]
fn setlimits(nam: &str) -> i32 {
    let mut ret = 0;
    for i in 0..nlimits() as i32 {
        if zsetlimit(i, nam) != 0 {
            ret += 1;
        }
    }
    ret
}

#[cfg(not(unix))]
fn setlimits(_nam: &str) -> i32 {
    0
}

// =====================================================================
// Port of `bin_limit()` from Src/Builtins/rlimits.c:519.
// =====================================================================

/// Port of `bin_limit()` from `Src/Builtins/rlimits.c:519`.
///
/// C signature mirrored verbatim:
/// ```c
/// static int
/// bin_limit(char *nam, char **argv, Options ops, UNUSED(int func))
/// ```
/// `Options ops` becomes `&[bool; 256]` (the `OPT_ISSET` table from
/// `zsh.h:1396`); `char **argv` becomes `&[String]`; `UNUSED(int func)`
/// keeps the parameter for callsite compatibility.
#[cfg(unix)]
pub(crate) fn bin_limit(nam: &str, argv: &[String], ops: &options, _func: i32) -> i32 { // c:519
    // c:521-524 — locals
    let hard: bool;
    let mut limnum: i32;
    let mut lim: i32;
    let mut val: rlim_t;
    let mut ret: i32 = 0;

    ensure_limits_initialized();
    set_resinfo();

    hard = OPT_ISSET(ops, b'h'); // c:526
    if OPT_ISSET(ops, b's') && argv.is_empty() {
        return setlimits(""); // c:527-528 — C passes NULL
    }
    /* without arguments, display limits */ // c:529
    if argv.is_empty() {
        return showlimits(nam, hard, -1); // c:531
    }
    let mut argi = argv.iter(); // emulate `while ((s = *argv++))`
    while let Some(s_owned) = argi.next() {
        let s: &str = s_owned.as_str(); // c:532
        let sb = s.as_bytes();
        // Search for the appropriate resource name. (c:533-547)
        if !sb.is_empty() && sb[0].is_ascii_digit() { // c:536 idigit(*s)
            // c:538 — lim = (int)zstrtol(s, NULL, 10);
            lim = zstrtol(s, 10).0 as i32;
        } else {
            // c:541-547
            lim = -1;
            limnum = 0;
            let resinfo_lock = RESINFO.get().unwrap();
            let v = resinfo_lock.lock().unwrap();
            while (limnum as usize) < v.len() {
                if v[limnum as usize].name.starts_with(s) { // c:542 strncmp
                    if lim != -1 {
                        lim = -2;
                    } else {
                        lim = limnum;
                    }
                }
                limnum += 1;
            }
        }
        // c:548-555
        if lim < 0 {
            zwarnnam(
                nam,
                &if lim == -2 {
                    format!("ambiguous resource specification: {}", s)
                } else {
                    format!("no such resource: {}", s)
                },
            );
            return 1;
        }
        /* without value for limit, display the current limit */ // c:556
        let s_val: &str = match argi.next() {
            None => return showlimits(nam, hard, lim), // c:557-558
            Some(t) => t.as_str(),
        };
        if (lim as usize) >= RLIM_NLIMITS as usize { // c:559
            let (v, consumed) = zstrtorlimt(s_val, 10); // c:561
            if consumed != s_val.len() { // c:562 *s
                /* unknown limit, no idea how to scale */ // c:564
                zwarnnam(
                    nam,
                    &format!("unknown scaling factor: {}", &s_val[consumed..]),
                );
                return 1;
            }
            val = v;
        } else {
            // c:569 — resinfo[lim]->type
            let resinfo_lock = RESINFO.get().unwrap();
            let info_type = resinfo_lock.lock().unwrap()[lim as usize].r#type;
            match info_type {
                zlimtype::ZLIMTYPE_TIME => {
                    /* time-type resource (c:570-573) */
                    let (mut v, consumed) = zstrtorlimt(s_val, 10); // c:574
                    let rest = &s_val[consumed..];
                    let rb = rest.as_bytes();
                    if !rest.is_empty() { // c:575
                        if (rb[0] == b'h' || rb[0] == b'H') && rb.len() == 1 { // c:576
                            v = v.saturating_mul(3600);
                        } else if (rb[0] == b'm' || rb[0] == b'M') && rb.len() == 1 { // c:578
                            v = v.saturating_mul(60);
                        } else if rb[0] == b':' { // c:580
                            let (more, _) = zstrtorlimt(&rest[1..], 10);
                            v = v.saturating_mul(60).saturating_add(more); // c:581
                        } else { // c:582
                            zwarnnam(nam, &format!("unknown scaling factor: {}", rest));
                            return 1;
                        }
                    }
                    val = v;
                }
                zlimtype::ZLIMTYPE_NUMBER
                | zlimtype::ZLIMTYPE_UNKNOWN
                | zlimtype::ZLIMTYPE_MICROSECONDS => {
                    /* pure numeric resource (c:587-597) */
                    let t = s_val; // c:592
                    let (v, consumed) = zstrtorlimt(t, 10); // c:593
                    if consumed == 0 { // c:594 s == t
                        zwarnnam(nam, "limit must be a number"); // c:595
                        return 1;
                    }
                    val = v;
                }
                zlimtype::ZLIMTYPE_MEMORY => {
                    /* memory-type resource (c:598-612) */
                    let (mut v, consumed) = zstrtorlimt(s_val, 10); // c:601
                    let rest = &s_val[consumed..];
                    let rb = rest.as_bytes();
                    if rest.is_empty() || ((rb[0] == b'k' || rb[0] == b'K') && rb.len() == 1) { // c:602
                        if v != RLIM_INFINITY { // c:603
                            v = v.saturating_mul(1024); // c:604
                        }
                    } else if (rb[0] == b'M' || rb[0] == b'm') && rb.len() == 1 { // c:605
                        v = v.saturating_mul(1024 * 1024); // c:606
                    } else if (rb[0] == b'G' || rb[0] == b'g') && rb.len() == 1 { // c:607
                        v = v.saturating_mul(1024 * 1024 * 1024); // c:608
                    } else { // c:609
                        zwarnnam(nam, &format!("unknown scaling factor: {}", rest));
                        return 1;
                    }
                    val = v;
                }
            }
        }
        // c:614 — do_limit(nam, lim, val, hard, !hard, OPT_ISSET(ops,'s'))
        if do_limit(nam, lim, val, hard, !hard, OPT_ISSET(ops, b's')) != 0 {
            ret += 1; // c:615
        }
    }
    ret // c:617
}

#[cfg(not(unix))]
pub(crate) fn bin_limit(_nam: &str, _argv: &[String], _ops: &options, _func: i32) -> i32 {
    0
}


// =====================================================================
// Port of `do_unlimit()` from Src/Builtins/rlimits.c:622.
// =====================================================================

/// Port of `do_unlimit()` from `Src/Builtins/rlimits.c:622`.
#[cfg(unix)]
pub(crate) fn do_unlimit(                                                   // c:622
    nam: &str,
    lim: i32,
    hard: bool,
    soft: bool,
    set: bool,
    euid: u32,
) -> i32 {
    ensure_limits_initialized();
    if (lim as usize) >= nlimits() {
        let mut vals = rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        if unsafe { getrlimit(lim as _, &mut vals) } < 0 {
            zwarnnam(nam, &format!("can't read limit: {}", std::io::Error::last_os_error()));
            return 1;
        }
        if hard {
            if euid != 0 && vals.rlim_max != RLIM_INFINITY {
                zwarnnam(nam, "can't remove hard limits");
                return 1;
            }
            vals.rlim_max = RLIM_INFINITY;
        }
        if !hard || soft {
            vals.rlim_cur = vals.rlim_max;
        }
        if !set {
            zwarnnam(nam, &format!("warning: unrecognised limit {}, use -s to set", lim));
            return 1;
        } else if unsafe { setrlimit(lim as _, &vals) } < 0 {
            zwarnnam(nam, &format!("setrlimit failed: {}", std::io::Error::last_os_error()));
            return 1;
        }
    } else {
        if hard {
            let cur_max = CURRENT_LIMITS
                .get()
                .and_then(|l| l.lock().ok())
                .map(|v| v[lim as usize].rlim_max)
                .unwrap_or(0);
            if euid != 0 && cur_max != RLIM_INFINITY {
                zwarnnam(nam, "can't remove hard limits");
                return 1;
            } else if let Some(lock) = LIMITS.get() {
                let mut v = lock.lock().unwrap();
                v[lim as usize].rlim_max = RLIM_INFINITY;
            }
        }
        if !hard || soft {
            if let Some(lock) = LIMITS.get() {
                let mut v = lock.lock().unwrap();
                v[lim as usize].rlim_cur = v[lim as usize].rlim_max;
            }
        }
        if set && zsetlimit(lim, nam) != 0 {
            return 1;
        }
    }
    0
}

#[cfg(not(unix))]
pub(crate) fn do_unlimit(
    _nam: &str,
    _lim: i32,
    _hard: bool,
    _soft: bool,
    _set: bool,
    _euid: u32,
) -> i32 {
    0
}

// =====================================================================
// Port of `bin_unlimit()` from Src/Builtins/rlimits.c:670.
// =====================================================================

/// Port of `bin_unlimit()` from `Src/Builtins/rlimits.c:670`.
///
/// C signature mirrored verbatim:
/// ```c
/// static int
/// bin_unlimit(char *nam, char **argv, Options ops, UNUSED(int func))
/// ```
#[cfg(unix)]
pub(crate) fn bin_unlimit(nam: &str, argv: &[String], ops: &options, _func: i32) -> i32 { // c:670
    // c:672-674 — locals
    let hard: bool;
    let mut limnum: i32;
    let mut lim: i32;
    let mut ret: i32 = 0;
    let euid: libc::uid_t = unsafe { geteuid() };

    ensure_limits_initialized();
    set_resinfo();

    hard = OPT_ISSET(ops, b'h'); // c:676
    /* Without arguments, remove all limits. */ // c:677
    if argv.is_empty() {
        // c:679 — for (limnum = 0; limnum != RLIM_NLIMITS; limnum++)
        limnum = 0;
        while limnum != RLIM_NLIMITS as i32 {
            if hard { // c:680
                let cur_max = CURRENT_LIMITS
                    .get()
                    .and_then(|l| l.lock().ok())
                    .map(|v| v[limnum as usize].rlim_max)
                    .unwrap_or(0);
                if euid != 0 && cur_max != RLIM_INFINITY { // c:681
                    ret += 1; // c:682
                } else if let Some(lock) = LIMITS.get() { // c:684
                    let mut v = lock.lock().unwrap();
                    v[limnum as usize].rlim_max = RLIM_INFINITY;
                }
            } else if let Some(lock) = LIMITS.get() { // c:685-686
                let mut v = lock.lock().unwrap();
                v[limnum as usize].rlim_cur = v[limnum as usize].rlim_max;
            }
            limnum += 1;
        }
        if OPT_ISSET(ops, b's') { // c:688
            ret += setlimits(nam); // c:689
        }
        if ret != 0 { // c:690
            zwarnnam(nam, "can't remove hard limits"); // c:691
        }
    } else {
        // c:693 — for (; *argv; argv++)
        let mut argi = argv.iter();
        while let Some(arg) = argi.next() {
            let s: &str = arg.as_str();
            let sb = s.as_bytes();
            // c:698-707 — Search for the appropriate resource name
            if !sb.is_empty() && sb[0].is_ascii_digit() { // c:698 idigit(**argv)
                lim = zstrtol(s, 10).0 as i32; // c:699
            } else {
                lim = -1; // c:701
                limnum = 0;
                let resinfo_lock = RESINFO.get().unwrap();
                let v = resinfo_lock.lock().unwrap();
                while (limnum as usize) < v.len() {
                    if v[limnum as usize].name.starts_with(s) { // c:702 strncmp
                        if lim != -1 {
                            lim = -2; // c:704
                        } else {
                            lim = limnum; // c:706
                        }
                    }
                    limnum += 1;
                }
            }
            // c:711-715
            if lim < 0 {
                zwarnnam(
                    nam,
                    &if lim == -2 {
                        format!("ambiguous resource specification: {}", s)
                    } else {
                        format!("no such resource: {}", s)
                    },
                );
                return 1;
            } else if do_unlimit(nam, lim, hard, !hard, OPT_ISSET(ops, b's'), euid) != 0 {
                ret += 1; // c:717-719
            }
        }
    }
    ret // c:722
}

#[cfg(not(unix))]
pub(crate) fn bin_unlimit(_nam: &str, _argv: &[String], _ops: &options, _func: i32) -> i32 {
    0
}

// =====================================================================
// Port of `bin_ulimit()` from Src/Builtins/rlimits.c:729.
// =====================================================================

/// Port of `bin_ulimit()` from `Src/Builtins/rlimits.c:729`.
///
/// C signature mirrored verbatim:
/// ```c
/// static int
/// bin_ulimit(char *name, char **argv, UNUSED(Options ops), UNUSED(int func))
/// ```
/// `Options ops` and `int func` are UNUSED in C; kept as parameters
/// for callsite compatibility.
#[cfg(unix)]
pub(crate) fn bin_ulimit(
    name: &str,
    argv: &[String],
    _ops: &options,
    _func: i32,
) -> i32 {
    // c:731 — locals
    let mut res: i32;
    let mut resmask: u64 = 0;
    let mut hard: bool = false;
    let mut soft: bool = false;
    let mut nres: i32 = 0;
    let mut all: bool = false;
    let mut ret: i32 = 0;

    ensure_limits_initialized();
    set_resinfo();

    // C uses `do { ... } while (*argv);` with `argv++` increments
    // inline. We emulate by carrying a cursor index over argv.
    let mut argi: usize = 0;
    loop {
        // c:735 — options = *argv;
        let options: Option<&String> = argv.get(argi);
        // c:736 — if (options && *options == '-' && !options[1])
        if let Some(o) = options {
            if o == "-" {
                zwarnnam(name, "missing option letter"); // c:737
                return 1;
            }
        }
        res = -1; // c:740
        // c:741 — if (options && *options == '-')
        if options.map(|o| o.starts_with('-') && o.len() > 1).unwrap_or(false) {
            argi += 1; // c:742 argv++
            let opt_str = options.unwrap().clone();
            let opt_bytes = opt_str.as_bytes();
            let mut p: usize = 1; // skip leading '-'  ; emulates `while (*++options)` (c:743)
            while p < opt_bytes.len() {
                let mut c = opt_bytes[p];
                // c:744 — if (*options == Meta) *++options ^= 32;
                // (Meta-character handling skipped — argv strings are
                // already metafied in zsh; the Rust port doesn't model
                // Meta yet. See zsh.h:Meta = 0x83.)
                res = -1; // c:746
                let mut continue_outer = false;
                match c {
                    b'H' => { // c:748
                        hard = true;
                        p += 1;
                        continue_outer = true;
                    }
                    b'S' => { // c:751
                        soft = true;
                        p += 1;
                        continue_outer = true;
                    }
                    b'N' => { // c:754
                        // c:755-762 — number after -N
                        let number: String = if p + 1 < opt_bytes.len() {
                            let n = std::str::from_utf8(&opt_bytes[p + 1..])
                                .unwrap_or("")
                                .to_string();
                            n
                        } else if argi < argv.len() {
                            let n = argv[argi].clone();
                            argi += 1;
                            n
                        } else {
                            zwarnnam(name, "number required after -N"); // c:760
                            return 1;
                        };
                        // c:763 — res = (int)zstrtol(number, &eptr, 10);
                        let nb = number.as_bytes();
                        let mut consumed = 0;
                        let mut acc: i64 = 0;
                        while consumed < nb.len() && nb[consumed].is_ascii_digit() {
                            acc = acc.saturating_mul(10).saturating_add((nb[consumed] - b'0') as i64);
                            consumed += 1;
                        }
                        if consumed != nb.len() { // c:764 *eptr
                            zwarnnam(name, &format!("invalid number: {}", number));
                            return 1;
                        }
                        res = acc as i32;
                        // c:771 — fake it so it looks like we just finished an option
                        p = opt_bytes.len();
                    }
                    b'a' => { // c:774
                        if resmask != 0 {
                            zwarnnam(name, "no limits allowed with -a"); // c:776
                            return 1;
                        }
                        all = true;
                        resmask = (1u64 << RLIM_NLIMITS as i32) - 1; // c:780
                        nres = RLIM_NLIMITS as i32; // c:781
                        p += 1;
                        continue_outer = true;
                    }
                    _ => { // c:783
                        res = find_resource(c as char); // c:784
                        if res < 0 {
                            /* unrecognised limit */ // c:786
                            zwarnnam(name, &format!("bad option: -{}", c as char));
                            return 1;
                        }
                    }
                }
                if continue_outer {
                    continue;
                }
                if p + 1 < opt_bytes.len() { // c:792 options[1]
                    resmask |= 1u64 << res; // c:793
                    nres += 1; // c:794
                }
                if all && res != -1 { // c:796
                    zwarnnam(name, "no limits allowed with -a"); // c:797
                    return 1;
                }
                p += 1;
                // Handle c:763 case where -N consumed the rest:
                if c == b'N' {
                    // already advanced past
                }
                // Actually, we need to break out of inner `while` and
                // continue outer do-while when we hit end of `options`.
                let _ = c; // suppress unused if N path
                c = 0;     // discard
                let _ = c;
            }
        }
        // c:802 — if (!*argv || **argv == '-')
        let next_is_dash = argv
            .get(argi)
            .map(|a| a.starts_with('-'))
            .unwrap_or(true);
        if next_is_dash {
            if res < 0 { // c:803
                if argi < argv.len() || nres != 0 { // c:804
                    if argi >= argv.len() {
                        // *argv == NULL, fall through to break (do-while)
                        break;
                    }
                    continue; // c:805
                } else {
                    res = RLIMIT_FSIZE as i32; // c:807
                }
            }
            resmask |= 1u64 << res; // c:809
            nres += 1; // c:810
            if argi >= argv.len() {
                break; // do-while terminates
            }
            continue; // c:811
        }
        if all { // c:813
            zwarnnam(name, "no arguments allowed after -a"); // c:814
            return 1;
        }
        if res < 0 { // c:817
            res = RLIMIT_FSIZE as i32; // c:818
        }
        // c:819 — if (strcmp(*argv, "unlimited"))
        let arg = argv[argi].clone();
        argi += 1; // c:851 argv++
        if arg != "unlimited" {
            /* set limit to specified value */ // c:820
            let limit: rlim_t;
            if arg == "hard" { // c:823
                let mut vals = rlimit { rlim_cur: 0, rlim_max: 0 }; // c:824
                if unsafe { getrlimit(res as _, &mut vals) } < 0 { // c:826
                    zwarnnam(
                        name,
                        &format!("can't read limit: {}", std::io::Error::last_os_error()),
                    );
                    return 1;
                }
                limit = vals.rlim_max; // c:833
            } else {
                let (mut v, consumed) = zstrtorlimt(&arg, 10); // c:836
                if consumed != arg.len() { // c:837 *eptr
                    zwarnnam(name, &format!("invalid number: {}", arg)); // c:838
                    return 1;
                }
                /* scale appropriately */ // c:841
                if (res as usize) < RLIM_NLIMITS as usize { // c:842
                    let resinfo_lock = RESINFO.get().unwrap();
                    let unit = resinfo_lock.lock().unwrap()[res as usize].unit as rlim_t;
                    v = v.saturating_mul(unit); // c:843
                }
                limit = v;
            }
            if do_limit(name, res, limit, hard, soft, true) != 0 { // c:845
                ret += 1; // c:846
            }
        } else { // c:847
            if do_unlimit(name, res, hard, soft, true, unsafe { geteuid() }) != 0 { // c:848
                ret += 1; // c:849
            }
        }
        // c:852 — } while (*argv);
        if argi >= argv.len() {
            break;
        }
    }
    // c:853 — for (res = 0; resmask; res++, resmask >>= 1)
    res = 0;
    while resmask != 0 {
        if (resmask & 1) != 0 && printulimit(name, res, hard, nres > 1) != 0 { // c:854
            ret += 1; // c:855
        }
        res += 1;
        resmask >>= 1;
    }
    ret // c:856
}

#[cfg(not(unix))]
pub(crate) fn bin_ulimit(
    _name: &str,
    _argv: &[String],
    _ops: &options,
    _func: i32,
) -> i32 {
    0
}

// =====================================================================
// Module entry points (rlimits.c:883-924).
// =====================================================================

// =====================================================================
// static struct builtin bintab[]                                     c:867
// static struct features module_features                             c:873
//
// Static dispatch tables consumed by the C module loader. The
// `module_features` table below is referenced by features_/enables_/
// cleanup_ and built lazily on first access (Rust can't init
// module_features as a `static` literal because Builtin contains
// fn-pointer fields).
// =====================================================================

use crate::ported::zsh_h::features as features_t;

// Backing store for `module_features` — built on first call to a
// loader hook. Bucket-2 shared global per the same rationale as
// LIMITS/RESINFO above.
static MODULE_FEATURES: OnceLock<Mutex<features_t>> = OnceLock::new();

fn module_features() -> &'static Mutex<features_t> {
    MODULE_FEATURES.get_or_init(|| {
        Mutex::new(features_t {
            bn_list: None,                                                // c:874 bintab
            bn_size: 3,                                                   // c:874 sizeof(bintab)/sizeof(*bintab) — limit, ulimit, unlimit
            cd_list: None,                                                // c:875
            cd_size: 0,
            mf_list: None,                                                // c:876
            mf_size: 0,
            pd_list: None,                                                // c:877
            pd_size: 0,
            n_abstract: 0,                                                // c:878
        })
    })
}

// =====================================================================
// setup_(UNUSED(Module m))                                           c:881
// =====================================================================

/// Port of `setup_()` from `Src/Builtins/rlimits.c:883`.
pub fn setup_(_m: *const module) -> i32 {
    0                                                                    // c:885
}

/// Port of `features_()` from `Src/Builtins/rlimits.c:890`.
/// C body: `*features = featuresarray(m, &module_features); return 0;`
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {
    *features = featuresarray(m, module_features());                     // c:892
    0                                                                    // c:893
}

/// Port of `enables_()` from `Src/Builtins/rlimits.c:898`.
/// C body: `return handlefeatures(m, &module_features, enables);`
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {
    handlefeatures(m, module_features(), enables)                        // c:900
}

/// Port of `boot_()` from `Src/Builtins/rlimits.c:905`.
/// C body: `set_resinfo(); return 0;`
pub fn boot_(_m: *const module) -> i32 {
    set_resinfo();                                                        // c:907
    0                                                                    // c:908
}

/// Port of `cleanup_()` from `Src/Builtins/rlimits.c:913`.
/// C body: `free_resinfo(); return setfeatureenables(m, &module_features, NULL);`
pub fn cleanup_(m: *const module) -> i32 {
    free_resinfo();                                                       // c:915
    setfeatureenables(m, module_features(), None)                        // c:916
}

/// Port of `finish_()` from `Src/Builtins/rlimits.c:921`.
pub fn finish_(_m: *const module) -> i32 {
    0                                                                    // c:923
}

// =====================================================================
// External fns from Src/module.c. Stubbed locally with C-faithful
// signatures pending the module.c port from `&Module`/`&Features`
// (CamelCase Rust-native) to `*const module`/`&Mutex<features>`.
// =====================================================================

// `featuresarray` lives in `Src/module.c:3275`. C signature:
//   char **featuresarray(Module m, Features f);
// Returns a NUL-terminated array of feature descriptors like "b:limit".
// Stub builds the descriptor list inline since the existing
// `crate::ported::module::featuresarray` takes wrong-typed args.
fn featuresarray(_m: *const module, _f: &Mutex<features_t>) -> Vec<String> {
    vec!["b:limit".to_string(), "b:ulimit".to_string(), "b:unlimit".to_string()]
}

// `handlefeatures` lives in `Src/module.c:3370`. C signature:
//   int handlefeatures(Module m, Features f, int **enables);
// On NULL `*enables`, fills it with the current per-feature enable bits
// via `getfeatureenables`. On non-NULL, calls `setfeatureenables`.
fn handlefeatures(m: *const module, f: &Mutex<features_t>, enables: &mut Option<Vec<i32>>) -> i32 {
    if enables.is_none() {
        *enables = Some(getfeatureenables(m, f));
    } else if let Some(e) = enables.as_ref() {
        return setfeatureenables(m, f, Some(e));
    }
    0
}

// `getfeatureenables` lives in `Src/module.c:3314`. Stub returns
// the bn_size + cd_size + mf_size + pd_size + n_abstract zero-vector
// since no feature is enabled in the static-link path.
fn getfeatureenables(_m: *const module, f: &Mutex<features_t>) -> Vec<i32> {
    let g = f.lock().unwrap();
    let total = g.bn_size + g.cd_size + g.mf_size + g.pd_size + g.n_abstract;
    vec![0; total as usize]
}

// `setfeatureenables` lives in `Src/module.c:3445`. C disables every
// registered feature via `*_addbuiltin/_addparamdef/etc` reverse calls.
// Stub: no-op since static-link path doesn't register through the
// runtime module loader.
fn setfeatureenables(_m: *const module, _f: &Mutex<features_t>, _e: Option<&Vec<i32>>) -> i32 {
    0
}

// Bridge fns
// `bin_ulimit` live in `src/extensions/ext_builtins.rs` (the
// non-ported dispatcher layer). They construct a `struct options`
// from the leading flag run and delegate to the free fns above.

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(unix)]
    fn test_zstrtorlimt_unlimited() {
        let (v, consumed) = zstrtorlimt("unlimited", 10);
        assert_eq!(v, RLIM_INFINITY);
        assert_eq!(consumed, "unlimited".len());
    }

    #[test]
    #[cfg(unix)]
    fn test_zstrtorlimt_decimal() {
        let (v, consumed) = zstrtorlimt("1234", 10);
        assert_eq!(v, 1234);
        assert_eq!(consumed, 4);
    }

    #[test]
    #[cfg(unix)]
    fn test_zstrtorlimt_hex() {
        let (v, consumed) = zstrtorlimt("0xff", 0);
        assert_eq!(v, 255);
        assert_eq!(consumed, 4);
    }

    #[test]
    #[cfg(unix)]
    fn test_find_resource() {
        set_resinfo();
        assert!(find_resource('t') >= 0);
        assert!(find_resource('f') >= 0);
        assert_eq!(find_resource('z'), -1);
    }

    #[test]
    #[cfg(unix)]
    fn test_set_resinfo_populates_table() {
        set_resinfo();
        let lock = RESINFO.get().unwrap();
        let v = lock.lock().unwrap();
        assert_eq!(v.len(), nlimits());
    }
}

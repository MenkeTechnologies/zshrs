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

use crate::ported::utils::zwarnnam;

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
pub(crate) fn showlimits(nam: &str, hard: bool, lim: i32) -> i32 {
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
pub(crate) fn printulimit(nam: &str, lim: i32, hard: bool, head: bool) -> i32 {
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
pub(crate) fn do_limit(
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
/// `limit [-h] [-s] [resource [value]]…`. `-h` shows/sets hard
/// limits; `-s` flushes pending (hard/soft) changes via
/// `setrlimit(2)`. Without args, displays all current limits.
#[cfg(unix)]
pub(crate) fn bin_limit(nam: &str, argv: &[String], ops_h: bool, ops_s: bool) -> i32 {
    ensure_limits_initialized();
    set_resinfo();
    let hard = ops_h;
    if ops_s && argv.is_empty() {
        return setlimits(nam);
    }
    if argv.is_empty() {
        return showlimits(nam, hard, -1);
    }
    let mut idx = 0usize;
    let mut ret = 0;
    while idx < argv.len() {
        let s = argv[idx].as_str();
        idx += 1;
        let lim: i32 = if s.as_bytes().first().map(|c| c.is_ascii_digit()).unwrap_or(false) {
            s.parse().unwrap_or(0)
        } else {
            let resinfo_lock = RESINFO.get().unwrap();
            let v = resinfo_lock.lock().unwrap();
            let mut found: i32 = -1;
            for (limnum, info) in v.iter().enumerate() {
                if info.name.starts_with(s) {
                    if found != -1 {
                        found = -2;
                    } else {
                        found = limnum as i32;
                    }
                }
            }
            found
        };
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
        if idx >= argv.len() {
            return showlimits(nam, hard, lim);
        }
        let s = argv[idx].as_str();
        idx += 1;
        let val: rlim_t;
        if (lim as usize) >= nlimits() {
            let (v, consumed) = zstrtorlimt(s, 10);
            if consumed != s.len() {
                zwarnnam(nam, &format!("unknown scaling factor: {}", &s[consumed..]));
                return 1;
            }
            val = v;
        } else {
            let info = lookup_resinfo(lim).unwrap();
            match info.r#type {
                zlimtype::ZLIMTYPE_TIME => {
                    let (mut v, consumed) = zstrtorlimt(s, 10);
                    let rest = &s[consumed..];
                    if !rest.is_empty() {
                        let rb = rest.as_bytes();
                        if (rb[0] == b'h' || rb[0] == b'H') && rest.len() == 1 {
                            v *= 3600;
                        } else if (rb[0] == b'm' || rb[0] == b'M') && rest.len() == 1 {
                            v *= 60;
                        } else if rb[0] == b':' {
                            let (more, _consumed2) = zstrtorlimt(&rest[1..], 10);
                            v = v * 60 + more;
                        } else {
                            zwarnnam(nam, &format!("unknown scaling factor: {}", rest));
                            return 1;
                        }
                    }
                    val = v;
                }
                zlimtype::ZLIMTYPE_NUMBER
                | zlimtype::ZLIMTYPE_UNKNOWN
                | zlimtype::ZLIMTYPE_MICROSECONDS => {
                    let (v, consumed) = zstrtorlimt(s, 10);
                    if consumed == 0 {
                        zwarnnam(nam, "limit must be a number");
                        return 1;
                    }
                    val = v;
                }
                zlimtype::ZLIMTYPE_MEMORY => {
                    let (mut v, consumed) = zstrtorlimt(s, 10);
                    let rest = &s[consumed..];
                    if rest.is_empty() || (rest.len() == 1 && (rest == "k" || rest == "K")) {
                        if v != RLIM_INFINITY {
                            v *= 1024;
                        }
                    } else if rest.len() == 1 && (rest == "M" || rest == "m") {
                        v *= 1024 * 1024;
                    } else if rest.len() == 1 && (rest == "G" || rest == "g") {
                        v *= 1024 * 1024 * 1024;
                    } else {
                        zwarnnam(nam, &format!("unknown scaling factor: {}", rest));
                        return 1;
                    }
                    val = v;
                }
            }
        }
        if do_limit(nam, lim, val, hard, !hard, ops_s) != 0 {
            ret += 1;
        }
    }
    ret
}

#[cfg(not(unix))]
pub(crate) fn bin_limit(_nam: &str, _argv: &[String], _ops_h: bool, _ops_s: bool) -> i32 {
    0
}

// =====================================================================
// Port of `do_unlimit()` from Src/Builtins/rlimits.c:622.
// =====================================================================

/// Port of `do_unlimit()` from `Src/Builtins/rlimits.c:622`.
#[cfg(unix)]
pub(crate) fn do_unlimit(
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
#[cfg(unix)]
pub(crate) fn bin_unlimit(nam: &str, argv: &[String], ops_h: bool, ops_s: bool) -> i32 {
    ensure_limits_initialized();
    set_resinfo();
    let hard = ops_h;
    let mut ret: i32 = 0;
    let euid = unsafe { geteuid() };
    if argv.is_empty() {
        for limnum in 0..nlimits() {
            if hard {
                let cur_max = CURRENT_LIMITS
                    .get()
                    .and_then(|l| l.lock().ok())
                    .map(|v| v[limnum].rlim_max)
                    .unwrap_or(0);
                if euid != 0 && cur_max != RLIM_INFINITY {
                    ret += 1;
                } else if let Some(lock) = LIMITS.get() {
                    let mut v = lock.lock().unwrap();
                    v[limnum].rlim_max = RLIM_INFINITY;
                }
            } else if let Some(lock) = LIMITS.get() {
                let mut v = lock.lock().unwrap();
                v[limnum].rlim_cur = v[limnum].rlim_max;
            }
        }
        if ops_s {
            ret += setlimits(nam);
        }
        if ret != 0 {
            zwarnnam(nam, "can't remove hard limits");
        }
    } else {
        for arg in argv {
            let s = arg.as_str();
            let lim: i32 = if s
                .as_bytes()
                .first()
                .map(|c| c.is_ascii_digit())
                .unwrap_or(false)
            {
                s.parse().unwrap_or(0)
            } else {
                let resinfo_lock = RESINFO.get().unwrap();
                let v = resinfo_lock.lock().unwrap();
                let mut found: i32 = -1;
                for (limnum, info) in v.iter().enumerate() {
                    if info.name.starts_with(s) {
                        if found != -1 {
                            found = -2;
                        } else {
                            found = limnum as i32;
                        }
                    }
                }
                found
            };
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
            } else if do_unlimit(nam, lim, hard, !hard, ops_s, euid) != 0 {
                ret += 1;
            }
        }
    }
    ret
}

#[cfg(not(unix))]
pub(crate) fn bin_unlimit(_nam: &str, _argv: &[String], _ops_h: bool, _ops_s: bool) -> i32 {
    0
}

// =====================================================================
// Port of `bin_ulimit()` from Src/Builtins/rlimits.c:729.
// =====================================================================

/// Port of `bin_ulimit()` from `Src/Builtins/rlimits.c:729`.
///
/// POSIX-flavored `ulimit`. Recognises `-H`/`-S`/`-a`/`-N <num>` and
/// per-resource short flags resolved via `find_resource()`.
#[cfg(unix)]
pub(crate) fn bin_ulimit(name: &str, argv: &[String]) -> i32 {
    ensure_limits_initialized();
    set_resinfo();
    let mut argv_idx = 0usize;
    let mut resmask: u64 = 0;
    let mut hard = false;
    let mut soft = false;
    let mut nres = 0;
    let mut all = false;
    let mut ret = 0;
    loop {
        let mut res: i32 = -1;
        let options = argv.get(argv_idx).cloned();
        if let Some(ref o) = options {
            if o == "-" {
                zwarnnam(name, "missing option letter");
                return 1;
            }
        }
        if let Some(ref o) = options {
            if o.starts_with('-') {
                argv_idx += 1;
                let mut chars: Vec<char> = o.chars().skip(1).collect();
                let mut i = 0usize;
                while i < chars.len() {
                    let c = chars[i];
                    res = -1;
                    match c {
                        'H' => {
                            hard = true;
                            i += 1;
                            continue;
                        }
                        'S' => {
                            soft = true;
                            i += 1;
                            continue;
                        }
                        'N' => {
                            let number = if i + 1 < chars.len() {
                                let s: String = chars[i + 1..].iter().collect();
                                i = chars.len();
                                s
                            } else if argv_idx < argv.len() {
                                let s = argv[argv_idx].clone();
                                argv_idx += 1;
                                s
                            } else {
                                zwarnnam(name, "number required after -N");
                                return 1;
                            };
                            match number.parse::<i32>() {
                                Ok(n) => res = n,
                                Err(_) => {
                                    zwarnnam(name, &format!("invalid number: {}", number));
                                    return 1;
                                }
                            }
                            // fake: pretend just finished an option
                            chars.clear();
                        }
                        'a' => {
                            if resmask != 0 {
                                zwarnnam(name, "no limits allowed with -a");
                                return 1;
                            }
                            all = true;
                            resmask = (1u64 << nlimits()) - 1;
                            nres = nlimits() as i32;
                            i += 1;
                            continue;
                        }
                        _ => {
                            res = find_resource(c);
                            if res < 0 {
                                zwarnnam(name, &format!("bad option: -{}", c));
                                return 1;
                            }
                        }
                    }
                    if i + 1 < chars.len() {
                        resmask |= 1u64 << res;
                        nres += 1;
                    }
                    if all && res != -1 {
                        zwarnnam(name, "no limits allowed with -a");
                        return 1;
                    }
                    i += 1;
                }
            }
        }
        let next = argv.get(argv_idx);
        match next {
            None | Some(_) if next.is_none() || next.unwrap().starts_with('-') => {
                if res < 0 {
                    if next.is_some() || nres != 0 {
                        if next.is_none() {
                            break;
                        }
                        continue;
                    } else {
                        res = RLIMIT_FSIZE as i32;
                    }
                }
                resmask |= 1u64 << res;
                nres += 1;
                if next.is_none() {
                    break;
                } else {
                    continue;
                }
            }
            _ => {}
        }
        if all {
            zwarnnam(name, "no arguments allowed after -a");
            return 1;
        }
        if res < 0 {
            res = RLIMIT_FSIZE as i32;
        }
        let arg = argv[argv_idx].clone();
        argv_idx += 1;
        if arg != "unlimited" {
            let limit: rlim_t;
            if arg == "hard" {
                let mut vals = rlimit {
                    rlim_cur: 0,
                    rlim_max: 0,
                };
                if unsafe { getrlimit(res as _, &mut vals) } < 0 {
                    zwarnnam(name, &format!("can't read limit: {}", std::io::Error::last_os_error()));
                    return 1;
                }
                limit = vals.rlim_max;
            } else {
                let (mut v, consumed) = zstrtorlimt(&arg, 10);
                if consumed != arg.len() {
                    zwarnnam(name, &format!("invalid number: {}", arg));
                    return 1;
                }
                if (res as usize) < nlimits() {
                    if let Some(info) = lookup_resinfo(res) {
                        v = v.saturating_mul(info.unit as rlim_t);
                    }
                }
                limit = v;
            }
            if do_limit(name, res, limit, hard, soft, true) != 0 {
                ret += 1;
            }
        } else if do_unlimit(name, res, hard, soft, true, unsafe { geteuid() }) != 0 {
            ret += 1;
        }
        if argv_idx >= argv.len() {
            break;
        }
    }
    let mut res = 0;
    while resmask != 0 {
        if resmask & 1 != 0 && printulimit(name, res, hard, nres > 1) != 0 {
            ret += 1;
        }
        res += 1;
        resmask >>= 1;
    }
    ret
}

#[cfg(not(unix))]
pub(crate) fn bin_ulimit(_name: &str, _argv: &[String]) -> i32 {
    0
}

// =====================================================================
// Module entry points (rlimits.c:883-924).
// =====================================================================

/// Port of `setup_()` from `Src/Builtins/rlimits.c:883`. Module load
/// hook — C returns 0 unconditionally.
pub fn setup_() -> i32 {
    0
}

/// Port of `features_()` from `Src/Builtins/rlimits.c:890`. Reports
/// the module's feature array. C side calls `featuresarray()`; the
/// Rust port has no Module type yet — return 0 to match the
/// success-on-no-op convention.
pub fn features_() -> i32 {
    0
}

/// Port of `enables_()` from `Src/Builtins/rlimits.c:898`. C calls
/// `handlefeatures()`; Rust port returns 0 pending the module-loader
/// port.
pub fn enables_() -> i32 {
    0
}

/// Port of `boot_()` from `Src/Builtins/rlimits.c:905`. C calls
/// `set_resinfo()` then returns 0.
pub fn boot_() -> i32 {
    set_resinfo();
    0
}

/// Port of `cleanup_()` from `Src/Builtins/rlimits.c:913`. C calls
/// `free_resinfo()` then `setfeatureenables()`. Rust port skips the
/// latter pending the module-loader port.
pub fn cleanup_() -> i32 {
    free_resinfo();
    0
}

/// Port of `finish_()` from `Src/Builtins/rlimits.c:921`. C returns
/// 0 unconditionally.
pub fn finish_() -> i32 {
    0
}

// =====================================================================
// ShellExecutor bridge — sanctioned PORT.md exception. The Rust
// builtin dispatcher invokes these methods to reach the canonical
// port above. Each is a thin wrapper that parses the leading
// `-h`/`-s`/`-H`/`-S`/`-a` flag, unpacks `args` into `argv`, and
// delegates to the corresponding free fn.
// =====================================================================

impl crate::ported::exec::ShellExecutor {
    /// `ulimit` builtin entry. Bridge to `bin_ulimit()` above.
    pub(crate) fn bin_ulimit(&self, args: &[String]) -> i32 {
        bin_ulimit("ulimit", args)
    }

    /// `limit` builtin entry. Bridge to `bin_limit()` above. Parses
    /// the leading `-h`/`-s` flag (matching C's OPT_ISSET in
    /// rlimits.c:519).
    pub(crate) fn bin_limit(&self, args: &[String]) -> i32 {
        let mut hard = false;
        let mut soft_set = false;
        let mut start = 0usize;
        for arg in args.iter() {
            match arg.as_str() {
                "-h" => {
                    hard = true;
                    start += 1;
                }
                "-s" => {
                    soft_set = true;
                    start += 1;
                }
                _ => break,
            }
        }
        bin_limit("limit", &args[start..], hard, soft_set)
    }

    /// `unlimit` builtin entry. Bridge to `bin_unlimit()` above.
    /// Parses leading `-h`/`-s` (matching C's OPT_ISSET in
    /// rlimits.c:670).
    pub(crate) fn bin_unlimit(&self, args: &[String]) -> i32 {
        let mut hard = false;
        let mut soft_set = false;
        let mut start = 0usize;
        for arg in args.iter() {
            match arg.as_str() {
                "-h" => {
                    hard = true;
                    start += 1;
                }
                "-s" => {
                    soft_set = true;
                    start += 1;
                }
                _ => break,
            }
        }
        bin_unlimit("unlimit", &args[start..], hard, soft_set)
    }
}

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

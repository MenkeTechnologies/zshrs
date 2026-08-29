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
    geteuid, getrlimit, rlim_t, rlimit, setrlimit, RLIMIT_AS, RLIMIT_CORE, RLIMIT_CPU, RLIMIT_DATA,
    RLIMIT_FSIZE, RLIMIT_MEMLOCK, RLIMIT_NOFILE, RLIMIT_NPROC, RLIMIT_RSS, RLIMIT_STACK,
    RLIM_INFINITY, RLIM_NLIMITS,
};

use crate::ported::utils::{zstrtol, zwarnnam};
use crate::ported::zsh_h::{module, options, OPT_ISSET};
use crate::zsh_h::features;
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
    /// `res` field.
    pub res: i32,
    /// `name` field.
    pub name: &'static str,
    pub r#type: zlimtype,
    /// `unit` field.
    pub unit: u32,
    /// `opt` field.
    pub opt: char,
    /// `descr` field.
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
    // c:rlimits.c:78 — RLIMIT_RSS / RLIMIT_VMEM. On macOS VMEM is
    // aliased to RLIMIT_AS so this entry's `'m'` opt is the only
    // place RSS appears; on Linux RSS is a distinct rlimit.
    resinfo_T {
        res: RLIMIT_RSS as i32,
        name: "resident",
        r#type: zlimtype::ZLIMTYPE_MEMORY,
        unit: 1024,
        opt: 'm',
        descr: "resident set size (kbytes)",
    },
    // c:rlimits.c:95 — RLIMIT_NPROC (`-u`).
    resinfo_T {
        res: RLIMIT_NPROC as i32,
        name: "maxproc",
        r#type: zlimtype::ZLIMTYPE_NUMBER,
        unit: 1,
        opt: 'u',
        descr: "processes",
    },
    // c:rlimits.c:99 — RLIMIT_MEMLOCK (`-l`).
    resinfo_T {
        res: RLIMIT_MEMLOCK as i32,
        name: "memorylocked",
        r#type: zlimtype::ZLIMTYPE_MEMORY,
        unit: 1024,
        opt: 'l',
        descr: "locked-in-memory size (kbytes)",
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
pub(crate) static CURRENT_LIMITS: OnceLock<Mutex<Vec<rlimit>>> = OnceLock::new();

/// Port of `mod_export struct rlimit limits[RLIM_NLIMITS]` from
/// `Src/exec.c:310`. The user-visible limits the next `setrlimit()`
/// will install. `setlimits(nam)` (Src/exec.c:331) flushes them via
/// `zsetlimit()`.
pub(crate) static LIMITS: OnceLock<Mutex<Vec<rlimit>>> = OnceLock::new();

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
// Port of `find_resource(char c)` from Src/Builtins/rlimits.c:239.
// =====================================================================

// Find resource by its option character                                    // c:239
/// Port of `find_resource(char c)` from `Src/Builtins/rlimits.c:239`.
///
/// Find a resource by its `ulimit` option character. Returns the
/// `RLIMIT_*` index, or `-1` on miss.
/// WARNING: param names don't match C — Rust=() vs C=(c)
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
// Port of `printrlim(rlim_t val, const char *unit)` from Src/Builtins/rlimits.c:253.
// =====================================================================

// Print a value of type rlim_t                                             // c:253
/// Port of `printrlim(rlim_t val, const char *unit)` from `Src/Builtins/rlimits.c:253`.
///
/// Print a `rlim_t` value with the supplied unit suffix to stdout.
/// C selects between `%qd` / `%lld` / `%lu` / `%ld` per
/// `RLIM_T_IS_*` macro; Rust port uses unified `Display` since
/// `rlim_t` is `u64` on macOS and Linux glibc.
/// WARNING: param names don't match C — Rust=(unit) vs C=(val, unit)
pub(crate) fn printrlim(val: rlim_t, unit: &str) {
    print!("{}{}", val, unit);
}

// =====================================================================
// Port of `zstrtorlimt(const char *s, char **t, int base)` from Src/Builtins/rlimits.c:272.
// =====================================================================

/// Port of `zstrtorlimt(const char *s, char **t, int base)` from `Src/Builtins/rlimits.c:272`.
///
/// Parse a numeric limit string. Returns `(value, bytes_consumed)`.
/// Recognises `unlimited` as `RLIM_INFINITY`; for digit input,
/// honours the same base-detection (`0x` hex, `0` octal, else 10)
/// that C's `RLIM_T_IS_QUAD_T`/`RLIM_T_IS_LONG_LONG`/
/// `RLIM_T_IS_UNSIGNED` paths do.
#[cfg(unix)]
/// WARNING: param names don't match C — Rust=(base) vs C=(s, t, base)
pub(crate) fn zstrtorlimt(s: &str, base: i32) -> (rlim_t, usize) {
    // c:277-281 — `if (strcmp(s, "unlimited") == 0) { ... }`. EXACT
    // match per `strcmp`. The previous Rust port used `s.starts_with`
    // which incorrectly treated `"unlimited_garbage"` or `"unlimitedX"`
    // as unlimited, then advanced pos to 9 — leaving trailing junk
    // unconsumed. C only matches the exact 9-char word.
    if s == "unlimited" {
        // c:277 strcmp == 0
        return (RLIM_INFINITY, "unlimited".len()); // c:279 *t = s + 9
    }
    let bytes = s.as_bytes();
    let mut pos: usize = 0;
    let mut base = base;
    if base == 0 {
        // c:283-291 — `if (*s != '0') base = 10; else if (*++s == 'x' || *s == 'X')
        //              base = 16, s++; else base = 8;`
        // C reads `*s` even for empty-input — `*s` returns the NUL
        // terminator which compares unequal to '0', so base=10 (no s++).
        // The previous Rust port used `bytes[pos] != b'0'` guarded ONLY
        // by `pos < bytes.len()`; on empty input the short-circuit
        // evaluated `else` and incremented pos to 1, leaving pos past-end.
        // Match C: treat past-end the same as "not '0'" (base=10, no advance).
        if pos >= bytes.len() || bytes[pos] != b'0' {
            // c:285 *s != '0' (with NUL semantics)
            base = 10; // c:286
        } else {
            pos += 1; // c:287 ++s
            if pos < bytes.len() && (bytes[pos] == b'x' || bytes[pos] == b'X') {
                base = 16; // c:288
                pos += 1;
            } else {
                base = 8; // c:289
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
// Port of `showlimitvalue(int lim, rlim_t val)` from Src/Builtins/rlimits.c:307.
// =====================================================================

/// Port of `showlimitvalue(int lim, rlim_t val)` from `Src/Builtins/rlimits.c:307`.
///
/// Print one limit row: 16-column-padded resource name, then the
/// value formatted per the resource's `zlimtype` (time as
/// `H:MM:SS`, microseconds as `Nus`, memory as `kB`/`MB`, plain
/// numeric otherwise).
#[cfg(unix)]
/// WARNING: param names don't match C — Rust=(val) vs C=(lim, val)
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
                println!("{}:{:02}:{:02}", val / 3600, (val / 60) % 60, val % 60);
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
// rlimit-printing ported don't each duplicate the lock dance.
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
// Port of `showlimits(char *nam, int hard, int lim)` from Src/Builtins/rlimits.c:346.
// =====================================================================

/// Port of `showlimits(char *nam, int hard, int lim)` from `Src/Builtins/rlimits.c:346`.
///
/// `lim == -1` means show all; `lim >= RLIM_NLIMITS` falls back to a
/// direct `getrlimit(2)` for resources the table doesn't know.
#[cfg(unix)]
/// WARNING: param names don't match C — Rust=(hard, lim) vs C=(nam, hard, lim)
pub(crate) fn showlimits(nam: &str, hard: bool, lim: i32) -> i32 {
    // c:346
    ensure_limits_initialized();
    if (lim as usize) >= nlimits() && lim != -1 {
        let mut vals = rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        if unsafe { getrlimit(lim as _, &mut vals) } < 0 {
            zwarnnam(
                nam,
                &format!("can't read limit: {}", std::io::Error::last_os_error()),
            );
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
// Port of `printulimit(char *nam, int lim, int hard, int head)` from Src/Builtins/rlimits.c:386.
// =====================================================================

/// Port of `printulimit(char *nam, int lim, int hard, int head)` from `Src/Builtins/rlimits.c:386`.
///
/// `ulimit`-style display. `head` controls whether to emit the
/// `-X: descr` heading column. `lim >= RLIM_NLIMITS` falls back to
/// a direct `getrlimit(2)`.
#[cfg(unix)]
/// WARNING: param names don't match C — Rust=(lim, hard, head) vs C=(nam, lim, hard, head)
pub(crate) fn printulimit(nam: &str, lim: i32, hard: bool, head: bool) -> i32 {
    // c:386
    ensure_limits_initialized();
    let limit: rlim_t = if (lim as usize) >= nlimits() {
        let mut vals = rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        if unsafe { getrlimit(lim as _, &mut vals) } < 0 {
            zwarnnam(
                nam,
                &format!("can't read limit: {}", std::io::Error::last_os_error()),
            );
            return 1;
        }
        if hard {
            vals.rlim_max
        } else {
            vals.rlim_cur
        }
    } else {
        let limits = match LIMITS.get() {
            Some(l) => l,
            None => return 1,
        };
        let v = limits.lock().unwrap();
        let r = &v[lim as usize];
        if hard {
            r.rlim_max
        } else {
            r.rlim_cur
        }
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
// Port of `do_limit(char *nam, int lim, rlim_t val, int hard, int soft, int set)` from Src/Builtins/rlimits.c:431.
// =====================================================================

/// Port of `do_limit(char *nam, int lim, rlim_t val, int hard, int soft, int set)` from `Src/Builtins/rlimits.c:431`.
///
/// Apply `val` to resource `lim` per the `hard`/`soft`/`set` flags.
/// `set` corresponds to `OPT_ISSET(ops, 's')` — when false, the
/// limits-array is updated but `setrlimit(2)` is not called.
#[cfg(unix)]
/// WARNING: param names don't match C — Rust=(lim, val, hard, soft, set) vs C=(nam, lim, val, hard, soft, set)
pub(crate) fn do_limit(
    // c:431
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
            zwarnnam(
                nam,
                &format!("can't read limit: {}", std::io::Error::last_os_error()),
            );
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
            zwarnnam(
                nam,
                &format!("warning: unrecognised limit {}, use -s to set", lim),
            );
            return 1;
        } else if unsafe { setrlimit(lim as _, &vals) } < 0 {
            zwarnnam(
                nam,
                &format!("setrlimit failed: {}", std::io::Error::last_os_error()),
            );
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

// =====================================================================
// Port of `bin_limit(char *nam, char **argv, Options ops, UNUSED(int func))` from Src/Builtins/rlimits.c:519.
// =====================================================================

/// Port of `bin_limit(char *nam, char **argv, Options ops, UNUSED(int func))` from `Src/Builtins/rlimits.c:519`.
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
/// WARNING: param names don't match C — Rust=(argv, ops, _func) vs C=(nam, argv, ops, func)
pub(crate) fn bin_limit(nam: &str, argv: &[String], ops: &options, _func: i32) -> i32 {
    // c:519
    // c:519-524 — locals
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
    /* without arguments, display limits */
    // c:529
    if argv.is_empty() {
        return showlimits(nam, hard, -1); // c:531
    }
    let mut argi = argv.iter(); // emulate `while ((s = *argv++))`
    while let Some(s_owned) = argi.next() {
        let s: &str = s_owned.as_str(); // c:532
        let sb = s.as_bytes();
        // Search for the appropriate resource name. (c:533-547)
        if !sb.is_empty() && sb[0].is_ascii_digit() {
            // c:536 idigit(*s)
            // c:538 — lim = (int)zstrtol(s, NULL, 10);
            lim = zstrtol(s, 10).0 as i32;
        } else {
            // c:541-547
            lim = -1;
            limnum = 0;
            let resinfo_lock = RESINFO.get().unwrap();
            let v = resinfo_lock.lock().unwrap();
            while (limnum as usize) < v.len() {
                if v[limnum as usize].name.starts_with(s) {
                    // c:542 strncmp
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
        /* without value for limit, display the current limit */
        // c:556
        let s_val: &str = match argi.next() {
            None => return showlimits(nam, hard, lim), // c:557-558
            Some(t) => t.as_str(),
        };
        if (lim as usize) >= RLIM_NLIMITS as usize {
            // c:559
            let (v, consumed) = zstrtorlimt(s_val, 10); // c:561
            if consumed != s_val.len() {
                // c:562 *s
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
                    if !rest.is_empty() {
                        // c:575
                        if (rb[0] == b'h' || rb[0] == b'H') && rb.len() == 1 {
                            // c:576
                            v = v.saturating_mul(3600);
                        } else if (rb[0] == b'm' || rb[0] == b'M') && rb.len() == 1 {
                            // c:578
                            v = v.saturating_mul(60);
                        } else if rb[0] == b':' {
                            // c:580
                            let (more, _) = zstrtorlimt(&rest[1..], 10);
                            v = v.saturating_mul(60).saturating_add(more); // c:581
                        } else {
                            // c:582
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
                    if consumed == 0 {
                        // c:594 s == t
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
                    if rest.is_empty() || ((rb[0] == b'k' || rb[0] == b'K') && rb.len() == 1) {
                        // c:602
                        if v != RLIM_INFINITY {
                            // c:603
                            v = v.saturating_mul(1024); // c:604
                        }
                    } else if (rb[0] == b'M' || rb[0] == b'm') && rb.len() == 1 {
                        // c:605
                        v = v.saturating_mul(1024 * 1024); // c:606
                    } else if (rb[0] == b'G' || rb[0] == b'g') && rb.len() == 1 {
                        // c:607
                        v = v.saturating_mul(1024 * 1024 * 1024); // c:608
                    } else {
                        // c:609
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
// Port of `do_unlimit(char *nam, int lim, int hard, int soft, int set, int euid)` from Src/Builtins/rlimits.c:622.
// =====================================================================

/// Port of `do_unlimit(char *nam, int lim, int hard, int soft, int set, int euid)` from `Src/Builtins/rlimits.c:622`.
#[cfg(unix)]
/// WARNING: param names don't match C — Rust=(lim, hard, soft, set, euid) vs C=(nam, lim, hard, soft, set, euid)
pub(crate) fn do_unlimit(
    // c:622
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
            zwarnnam(
                nam,
                &format!("can't read limit: {}", std::io::Error::last_os_error()),
            );
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
            zwarnnam(
                nam,
                &format!("warning: unrecognised limit {}, use -s to set", lim),
            );
            return 1;
        } else if unsafe { setrlimit(lim as _, &vals) } < 0 {
            zwarnnam(
                nam,
                &format!("setrlimit failed: {}", std::io::Error::last_os_error()),
            );
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
// Port of `bin_unlimit(char *nam, char **argv, Options ops, UNUSED(int func))` from Src/Builtins/rlimits.c:670.
// =====================================================================

/// Port of `bin_unlimit(char *nam, char **argv, Options ops, UNUSED(int func))` from `Src/Builtins/rlimits.c:670`.
///
/// C signature mirrored verbatim:
/// ```c
/// static int
/// bin_unlimit(char *nam, char **argv, Options ops, UNUSED(int func))
/// ```
#[cfg(unix)]
/// WARNING: param names don't match C — Rust=(argv, ops, _func) vs C=(nam, argv, ops, func)
pub(crate) fn bin_unlimit(nam: &str, argv: &[String], ops: &options, _func: i32) -> i32 {
    // c:670
    // c:670-674 — locals
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
            if hard {
                // c:680
                let cur_max = CURRENT_LIMITS
                    .get()
                    .and_then(|l| l.lock().ok())
                    .map(|v| v[limnum as usize].rlim_max)
                    .unwrap_or(0);
                if euid != 0 && cur_max != RLIM_INFINITY {
                    // c:681
                    ret += 1; // c:682
                } else if let Some(lock) = LIMITS.get() {
                    // c:684
                    let mut v = lock.lock().unwrap();
                    v[limnum as usize].rlim_max = RLIM_INFINITY;
                }
            } else if let Some(lock) = LIMITS.get() {
                // c:685-686
                let mut v = lock.lock().unwrap();
                v[limnum as usize].rlim_cur = v[limnum as usize].rlim_max;
            }
            limnum += 1;
        }
        if OPT_ISSET(ops, b's') {
            // c:688
            ret += setlimits(nam); // c:689
        }
        if ret != 0 {
            // c:690
            zwarnnam(nam, "can't remove hard limits"); // c:691
        }
    } else {
        // c:693 — for (; *argv; argv++)
        let mut argi = argv.iter();
        while let Some(arg) = argi.next() {
            let s: &str = arg.as_str();
            let sb = s.as_bytes();
            // c:698-707 — Search for the appropriate resource name
            if !sb.is_empty() && sb[0].is_ascii_digit() {
                // c:698 idigit(**argv)
                lim = zstrtol(s, 10).0 as i32; // c:699
            } else {
                lim = -1; // c:701
                limnum = 0;
                let resinfo_lock = RESINFO.get().unwrap();
                let v = resinfo_lock.lock().unwrap();
                while (limnum as usize) < v.len() {
                    if v[limnum as usize].name.starts_with(s) {
                        // c:702 strncmp
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
// Port of `bin_ulimit(char *name, char **argv, UNUSED(Options ops), UNUSED(int func))` from Src/Builtins/rlimits.c:729.
// =====================================================================

/// Port of `bin_ulimit(char *name, char **argv, UNUSED(Options ops), UNUSED(int func))` from `Src/Builtins/rlimits.c:729`.
///
/// C signature mirrored verbatim:
/// ```c
/// static int
/// bin_ulimit(char *name, char **argv, UNUSED(Options ops), UNUSED(int func))
/// ```
/// `Options ops` and `int func` are UNUSED in C; kept as parameters
/// for callsite compatibility.
#[cfg(unix)]
/// WARNING: param names don't match C — Rust=(argv, _ops, _func) vs C=(name, argv, ops, func)
pub(crate) fn bin_ulimit(name: &str, argv: &[String], _ops: &options, _func: i32) -> i32 {
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
        if options
            .map(|o| o.starts_with('-') && o.len() > 1)
            .unwrap_or(false)
        {
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
                    b'H' => {
                        // c:748
                        hard = true;
                        p += 1;
                        continue_outer = true;
                    }
                    b'S' => {
                        // c:751
                        soft = true;
                        p += 1;
                        continue_outer = true;
                    }
                    b'N' => {
                        // c:754
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
                            acc = acc
                                .saturating_mul(10)
                                .saturating_add((nb[consumed] - b'0') as i64);
                            consumed += 1;
                        }
                        if consumed != nb.len() {
                            // c:764 *eptr
                            zwarnnam(name, &format!("invalid number: {}", number));
                            return 1;
                        }
                        res = acc as i32;
                        // c:771 — fake it so it looks like we just finished an option
                        p = opt_bytes.len();
                    }
                    b'a' => {
                        // c:774
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
                    _ => {
                        // c:783
                        res = find_resource(c as char); // c:784
                        if res < 0 {
                            /* unrecognised limit */
                            // c:786
                            zwarnnam(name, &format!("bad option: -{}", c as char));
                            return 1;
                        }
                    }
                }
                if continue_outer {
                    continue;
                }
                if p + 1 < opt_bytes.len() {
                    // c:792 options[1]
                    resmask |= 1u64 << res; // c:793
                    nres += 1; // c:794
                }
                if all && res != -1 {
                    // c:796
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
                c = 0; // discard
                let _ = c;
            }
        }
        // c:802 — if (!*argv || **argv == '-')
        let next_is_dash = argv.get(argi).map(|a| a.starts_with('-')).unwrap_or(true);
        if next_is_dash {
            if res < 0 {
                // c:803
                if argi < argv.len() || nres != 0 {
                    // c:804
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
        if all {
            // c:813
            zwarnnam(name, "no arguments allowed after -a"); // c:814
            return 1;
        }
        if res < 0 {
            // c:817
            res = RLIMIT_FSIZE as i32; // c:818
        }
        // c:819 — if (strcmp(*argv, "unlimited"))
        let arg = argv[argi].clone();
        argi += 1; // c:851 argv++
        if arg != "unlimited" {
            /* set limit to specified value */
            // c:820
            let limit: rlim_t;
            if arg == "hard" {
                // c:823
                let mut vals = rlimit {
                    rlim_cur: 0,
                    rlim_max: 0,
                }; // c:824
                if unsafe { getrlimit(res as _, &mut vals) } < 0 {
                    // c:826
                    zwarnnam(
                        name,
                        &format!("can't read limit: {}", std::io::Error::last_os_error()),
                    );
                    return 1;
                }
                limit = vals.rlim_max; // c:833
            } else {
                let (mut v, consumed) = zstrtorlimt(&arg, 10); // c:836
                if consumed != arg.len() {
                    // c:837 *eptr
                    zwarnnam(name, &format!("invalid number: {}", arg)); // c:838
                    return 1;
                }
                /* scale appropriately */
                // c:841
                if (res as usize) < RLIM_NLIMITS as usize {
                    // c:842
                    let resinfo_lock = RESINFO.get().unwrap();
                    let unit = resinfo_lock.lock().unwrap()[res as usize].unit as rlim_t;
                    v = v.saturating_mul(unit); // c:843
                }
                limit = v;
            }
            if do_limit(name, res, limit, hard, soft, true) != 0 {
                // c:845
                ret += 1; // c:846
            }
        } else {
            // c:847
            if do_unlimit(name, res, hard, soft, true, unsafe { geteuid() }) != 0 {
                // c:848
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
        if (resmask & 1) != 0 && printulimit(name, res, hard, nres > 1) != 0 {
            // c:854
            ret += 1; // c:855
        }
        res += 1;
        resmask >>= 1;
    }
    ret // c:856
}

#[cfg(not(unix))]
pub(crate) fn bin_ulimit(_name: &str, _argv: &[String], _ops: &options, _func: i32) -> i32 {
    0
}

// =====================================================================
// setup_(UNUSED(Module m))                                           c:881
// =====================================================================

/// Port of `setup_(UNUSED(Module m))` from `Src/Builtins/rlimits.c:883`.
#[allow(unused_variables)]
pub fn setup_(m: *const module) -> i32 {
    0 // c:898
}

/// Port of `features_(UNUSED(Module m), UNUSED(char ***features))` from `Src/Builtins/rlimits.c:890`.
/// C body: `*features = featuresarray(m, &module_features); return 0;`
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {
    *features = featuresarray(m, module_features()); // c:898
    0 // c:905
}

/// Port of `enables_(UNUSED(Module m), UNUSED(int **enables))` from `Src/Builtins/rlimits.c:898`.
/// C body: `return handlefeatures(m, &module_features, enables);`
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {
    handlefeatures(m, module_features(), enables) // c:905
}

/// Port of `boot_(UNUSED(Module m))` from `Src/Builtins/rlimits.c:905`.
/// C body: `set_resinfo(); return 0;`
#[allow(unused_variables)]
pub fn boot_(m: *const module) -> i32 {
    set_resinfo(); // c:913
    0 // c:921
}

/// Port of `cleanup_(UNUSED(Module m))` from `Src/Builtins/rlimits.c:913`.
/// C body: `free_resinfo(); return setfeatureenables(m, &module_features, NULL);`
pub fn cleanup_(m: *const module) -> i32 {
    free_resinfo(); // c:921
    setfeatureenables(m, module_features(), None) // c:921
}

/// Port of `finish_(UNUSED(Module m))` from `Src/Builtins/rlimits.c:921`.
#[allow(unused_variables)]
pub fn finish_(m: *const module) -> i32 {
    0 // c:921
}

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
// tests/data/fake_fn_allowlist.txt.
#[cfg(unix)]
pub(crate) fn ensure_limits_initialized() {
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

// Backing store for `module_features` — built on first call to a
// loader hook. Bucket-2 shared global per the same rationale as
// LIMITS/RESINFO above.
static MODULE_FEATURES: OnceLock<Mutex<features>> = OnceLock::new();

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
    // The C source (`Src/Builtins/rlimits.c:194-216`) zshcalloc's a
    // fresh `resinfo[]` array every call — `boot_()` invokes
    // set_resinfo and `cleanup_()` invokes free_resinfo, so the
    // pair runs once per module load/unload cycle. The Rust port
    // must ALWAYS re-populate the inner Vec (not just first time)
    // or a `boot_/cleanup_/boot_` sequence leaves the table empty.
    //
    // Previously this used `OnceLock::get_or_init` which only ran the
    // populate closure ONCE per process. After `free_resinfo()`
    // cleared the Mutex's inner Vec, subsequent `set_resinfo()` was
    // a silent no-op. Fixed 2026-05: init the OnceLock if needed,
    // then always rebuild the Vec inside the existing Mutex.
    let lock = RESINFO.get_or_init(|| Mutex::new(Vec::with_capacity(nlimits())));
    let mut v = lock.lock().unwrap_or_else(|e| e.into_inner());
    v.clear(); // c:194 fresh zshcalloc
    v.reserve(nlimits());
    for i in 0..nlimits() as i32 {
        let entry = known_resources
            .iter()
            .find(|r| r.res == i)
            .cloned()
            .unwrap_or_else(|| resinfo_T {
                res: -1,                            // c:209
                name: leak_unknown_name(i),         // c:210
                r#type: zlimtype::ZLIMTYPE_UNKNOWN, // c:211
                unit: 1,                            // c:212
                opt: 'N',
                descr: leak_unknown_name(i),
            });
        v.push(entry);
    }
}

// WARNING: NOT IN RLIMITS.C — Rust-only helper. C `set_resinfo()`
// (rlimits.c:206-214) writes the synthesized `UNKNOWN-N` text into a
// `zalloc(12)` heap buffer; Rust port leaks a `Box<str>` since
// `&'static str` is required for the `name`/`descr` fields and
// allocation is a one-shot per resource at module-init.
fn leak_unknown_name(i: i32) -> &'static str {
    Box::leak(format!("UNKNOWN-{}", i).into_boxed_str())
}

// WARNING: NOT IN RLIMITS.C — `zsetlimit` lives in `Src/exec.c:314`,
// not `rlimits.c`. The Rust port colocates the helper here because
// rlimits.rs is the only consumer; the eventual vm_helper port can
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
            zwarnnam(
                nam,
                &format!("setrlimit failed: {}", std::io::Error::last_os_error()),
            );
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
pub(crate) fn setlimits(nam: &str) -> i32 {
    let mut ret = 0;
    for i in 0..nlimits() as i32 {
        if zsetlimit(i, nam) != 0 {
            ret += 1;
        }
    }
    ret
}

#[cfg(not(unix))]
pub(crate) fn setlimits(_nam: &str) -> i32 {
    0
}

// =====================================================================
// External ported from Src/module.c. Stubbed locally with C-faithful
// signatures pending the module.c port from `&Module`/`&Features`
// (CamelCase Rust-native) to `*const module`/`&Mutex<features>`.
// =====================================================================

// `featuresarray` lives in `Src/module.c:3279`. C signature:
//   char **featuresarray(Module m, Features f);
// Returns a NUL-terminated array of feature descriptors like "b:limit".
// Stub builds the descriptor list inline since the existing
// `crate::ported::module::featuresarray` takes wrong-typed args.
fn featuresarray(_m: *const module, _f: &Mutex<features>) -> Vec<String> {
    vec![
        "b:limit".to_string(),
        "b:ulimit".to_string(),
        "b:unlimit".to_string(),
    ]
}

// `handlefeatures` lives in `Src/module.c:3388`. C signature:
//   int handlefeatures(Module m, Features f, int **enables);
// On NULL `*enables`, fills it with the current per-feature enable bits
// via `getfeatureenables`. On non-NULL, calls `setfeatureenables`.
fn handlefeatures(m: *const module, f: &Mutex<features>, enables: &mut Option<Vec<i32>>) -> i32 {
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
fn getfeatureenables(_m: *const module, f: &Mutex<features>) -> Vec<i32> {
    let g = f.lock().unwrap();
    let total = g.bn_size + g.cd_size + g.mf_size + g.pd_size + g.n_abstract;
    vec![0; total as usize]
}

// `setfeatureenables` lives in `Src/module.c:3350`. C disables every
// registered feature via `*_addbuiltin/_addparamdef/etc` reverse calls.
// Stub: no-op since static-link path doesn't register through the
// runtime module loader.
fn setfeatureenables(_m: *const module, _f: &Mutex<features>, _e: Option<&Vec<i32>>) -> i32 {
    0
}

// Bridge ported
// `bin_ulimit` live in `src/extensions/ext_builtins.rs` (the
// non-ported dispatcher layer). They construct a `struct options`
// from the leading flag run and delegate to the free ported above.

// =====================================================================
// Tests
// =====================================================================

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ─── RUST-ONLY ACCESSORS ───
//
// Singleton accessor ported for `OnceLock<Mutex<T>>` / `OnceLock<
// RwLock<T>>` globals declared above. C zsh uses direct global
// access; Rust needs these wrappers because `OnceLock::get_or_init`
// is the only way to lazily construct shared state. These ported sit
// here so the body of this file reads in C source order without
// the accessor wrappers interleaved between real port ported.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ─── RUST-ONLY ACCESSORS ───
//
// Singleton accessor ported for `OnceLock<Mutex<T>>` / `OnceLock<
// RwLock<T>>` globals declared above. C zsh uses direct global
// access; Rust needs these wrappers because `OnceLock::get_or_init`
// is the only way to lazily construct shared state. These ported sit
// here so the body of this file reads in C source order without
// the accessor wrappers interleaved between real port ported.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn module_features() -> &'static Mutex<features> {
    MODULE_FEATURES.get_or_init(|| {
        Mutex::new(features {
            bn_list: None, // c:874 bintab
            bn_size: 3,    // c:874 sizeof(bintab)/sizeof(*bintab) — limit, ulimit, unlimit
            cd_list: None, // c:875
            cd_size: 0,
            mf_list: None, // c:876
            mf_size: 0,
            pd_list: None, // c:877
            pd_size: 0,
            n_abstract: 0, // c:883
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(unix)]
    fn test_zstrtorlimt_unlimited() {
        let _g = crate::test_util::global_state_lock();
        let (v, consumed) = zstrtorlimt("unlimited", 10);
        assert_eq!(v, RLIM_INFINITY);
        assert_eq!(consumed, "unlimited".len());
    }

    #[test]
    #[cfg(unix)]
    fn test_zstrtorlimt_decimal() {
        let _g = crate::test_util::global_state_lock();
        let (v, consumed) = zstrtorlimt("1234", 10);
        assert_eq!(v, 1234);
        assert_eq!(consumed, 4);
    }

    #[test]
    #[cfg(unix)]
    fn test_zstrtorlimt_hex() {
        let _g = crate::test_util::global_state_lock();
        let (v, consumed) = zstrtorlimt("0xff", 0);
        assert_eq!(v, 255);
        assert_eq!(consumed, 4);
    }

    #[test]
    #[cfg(unix)]
    fn test_find_resource() {
        let _g = crate::test_util::global_state_lock();
        set_resinfo();
        assert!(find_resource('t') >= 0);
        assert!(find_resource('f') >= 0);
        assert_eq!(find_resource('z'), -1);
    }

    #[test]
    #[cfg(unix)]
    fn test_set_resinfo_populates_table() {
        let _g = crate::test_util::global_state_lock();
        set_resinfo();
        let lock = RESINFO.get().unwrap();
        let v = lock.lock().unwrap();
        assert_eq!(v.len(), nlimits());
    }

    /// c:286 — `zstrtorlimt` parses zero correctly. Pin `0` →
    /// `(0, 1)` so a regression that returns RLIM_INFINITY or treats
    /// `0` as the empty-string sentinel gets caught.
    #[test]
    #[cfg(unix)]
    fn zstrtorlimt_zero_is_zero_with_one_consumed() {
        let _g = crate::test_util::global_state_lock();
        let (v, consumed) = zstrtorlimt("0", 10);
        assert_eq!(v, 0);
        assert_eq!(consumed, 1);
    }

    /// c:286 — `zstrtorlimt` with empty input reports 0 chars
    /// consumed. Callers distinguish "no digits" from a real "0"
    /// by checking `consumed == 0`.
    #[test]
    #[cfg(unix)]
    fn zstrtorlimt_empty_input_consumed_is_zero() {
        let _g = crate::test_util::global_state_lock();
        let (_v, consumed) = zstrtorlimt("", 10);
        assert_eq!(consumed, 0, "empty input must report 0 chars consumed");
    }

    /// c:286 — `zstrtorlimt` stops at the first non-digit and
    /// reports the partial consumption. Catches a regression that
    /// over-counts by including trailing garbage in `consumed`.
    #[test]
    #[cfg(unix)]
    fn zstrtorlimt_stops_at_non_digit() {
        let _g = crate::test_util::global_state_lock();
        let (v, consumed) = zstrtorlimt("42abc", 10);
        assert_eq!(v, 42);
        assert_eq!(
            consumed, 2,
            "must consume only the digit prefix, not the alpha suffix"
        );
    }

    /// c:286 — `zstrtorlimt("unlimited", base)` ignores `base` and
    /// yields `(RLIM_INFINITY, len("unlimited"))` for any base.
    #[test]
    #[cfg(unix)]
    fn zstrtorlimt_unlimited_ignores_base() {
        let _g = crate::test_util::global_state_lock();
        for base in [0, 8, 10, 16] {
            let (v, consumed) = zstrtorlimt("unlimited", base);
            assert_eq!(
                v, RLIM_INFINITY,
                "base={} must still recognize 'unlimited'",
                base
            );
            assert_eq!(consumed, "unlimited".len());
        }
    }

    /// c:286 — `zstrtorlimt(_, 0)` auto-detects base: leading `0x`
    /// → hex, leading `0` → octal, else decimal. Pin the octal arm
    /// because a regression that drops it silently mis-parses
    /// classic Unix octal `0644` permission limits.
    #[test]
    #[cfg(unix)]
    fn zstrtorlimt_auto_base_octal_prefix() {
        let _g = crate::test_util::global_state_lock();
        let (v, _) = zstrtorlimt("0644", 0);
        assert_eq!(v, 0o644, "leading 0 must parse as octal in base=0");
    }

    /// c:239 — `find_resource('z')` returns -1 (unknown char). Pin
    /// the sentinel because callers downstream branch on `< 0`.
    #[test]
    #[cfg(unix)]
    fn find_resource_unknown_char_returns_negative() {
        let _g = crate::test_util::global_state_lock();
        set_resinfo();
        assert!(find_resource('z') < 0, "unknown limit char must return < 0");
        assert!(find_resource(' ') < 0);
        assert!(find_resource('@') < 0);
    }

    /// c:239 — `find_resource` is case-sensitive. zsh's limit chars
    /// are lowercase; 't' is cpu time, 'T' must not collide.
    #[test]
    #[cfg(unix)]
    fn find_resource_is_case_sensitive() {
        let _g = crate::test_util::global_state_lock();
        set_resinfo();
        let lower_t = find_resource('t');
        let upper_t = find_resource('T');
        assert!(lower_t >= 0, "'t' (cpu time) must resolve");
        assert_ne!(
            lower_t, upper_t,
            "case-sensitivity collision: 't' and 'T' both index {}",
            lower_t
        );
    }

    /// c:139 — `set_resinfo` is idempotent: calling it twice
    /// produces the same table. Pin re-entry safety because the
    /// RESINFO global is OnceLock+Mutex; a regen that re-inits the
    /// inner on every call would discard runtime state.
    #[test]
    #[cfg(unix)]
    fn set_resinfo_is_idempotent() {
        let _g = crate::test_util::global_state_lock();
        set_resinfo();
        let len_first = RESINFO.get().unwrap().lock().unwrap().len();
        set_resinfo();
        let len_second = RESINFO.get().unwrap().lock().unwrap().len();
        assert_eq!(len_first, len_second, "set_resinfo must be idempotent");
    }

    /// c:1235-1276 — module-lifecycle stubs all return 0 in C.
    #[test]
    fn module_lifecycle_shims_all_return_zero() {
        let _g = crate::test_util::global_state_lock();
        let m: *const module = std::ptr::null();
        assert_eq!(setup_(m), 0);
        assert_eq!(boot_(m), 0);
        assert_eq!(cleanup_(m), 0);
        assert_eq!(finish_(m), 0);
    }

    /// c:1241 — `features_` populates the feature names and returns
    /// 0. rlimits exports `limit` / `ulimit` / `unlimit` builtins;
    /// the feature array must be non-empty.
    #[test]
    fn features_returns_nonempty_list() {
        let _g = crate::test_util::global_state_lock();
        let m: *const module = std::ptr::null();
        let mut features: Vec<String> = Vec::new();
        assert_eq!(features_(m, &mut features), 0);
        assert!(
            !features.is_empty(),
            "rlimits module must advertise at least one feature"
        );
    }

    /// c:913-921 — `boot_/cleanup_` re-init cycle. The C source
    /// pairs `boot_()` (which calls `set_resinfo()`) with
    /// `cleanup_()` (which calls `free_resinfo()`) — a second
    /// `boot_` after `cleanup_` MUST re-populate the table.
    ///
    /// This pins the port fix landed 2026-05 that replaced the
    /// `OnceLock::get_or_init` (one-shot) pattern in `set_resinfo`
    /// with always-rebuild-the-inner-Vec semantics. Before the fix,
    /// a `boot/cleanup/boot` sequence left the table empty because
    /// the get_or_init closure only ran on the first call;
    /// subsequent `set_resinfo` invocations were silent no-ops.
    #[test]
    #[cfg(unix)]
    fn boot_cleanup_boot_cycle_repopulates_resinfo() {
        let _g = crate::test_util::global_state_lock();
        let m: *const module = std::ptr::null();
        let _ = boot_(m);
        let after_first_boot = RESINFO.get().unwrap().lock().unwrap().len();
        assert_eq!(
            after_first_boot,
            nlimits(),
            "first boot must fully populate"
        );
        let _ = cleanup_(m);
        let after_cleanup = RESINFO.get().unwrap().lock().unwrap().len();
        assert_eq!(after_cleanup, 0, "cleanup must empty the table");
        let _ = boot_(m);
        let after_second_boot = RESINFO.get().unwrap().lock().unwrap().len();
        assert_eq!(
            after_second_boot,
            nlimits(),
            "second boot must re-populate (port bug fixed 2026-05)"
        );
    }

    /// `Src/Builtins/rlimits.c:277-281` — `zstrtorlimt("unlimited")` uses
    /// `strcmp(s, "unlimited") == 0` — EXACT match. The previous Rust
    /// port used `s.starts_with("unlimited")` which falsely accepted
    /// `"unlimited_garbage"` and `"unlimitedXYZ"` as unlimited,
    /// advancing pos to 9 and leaving the trailing junk silently
    /// consumed by the caller. Pin the strcmp-equivalent behavior.
    #[test]
    #[cfg(unix)]
    fn zstrtorlimt_unlimited_requires_exact_match() {
        let _g = crate::test_util::global_state_lock();
        // c:277 — exact "unlimited" returns RLIM_INFINITY.
        let (v, n) = zstrtorlimt("unlimited", 10);
        assert_eq!(v, RLIM_INFINITY);
        assert_eq!(n, 9, "advance past the 9 chars of 'unlimited'");

        // c:277 — prefix-match must NOT trigger the unlimited branch.
        // "unlimitedX" → C strcmp fails → falls through to digit parse;
        // digit parse on 'u' fails (not 0-9) → returns (0, 0).
        let (v, n) = zstrtorlimt("unlimitedX", 10);
        assert_ne!(
            v, RLIM_INFINITY,
            "c:277 — strcmp requires exact match, not prefix"
        );
        assert_eq!(v, 0, "non-digit prefix → no consumption, ret=0");
        assert_eq!(n, 0, "c:294 — digit loop exits immediately on 'u'");
    }

    /// `Src/Builtins/rlimits.c:283-291` — `if (!base)` block reads `*s`
    /// directly; on empty input `*s` returns NUL which compares unequal
    /// to '0' so base=10 is chosen WITHOUT advancing s. The previous
    /// Rust port lacked a past-end guard, fell into the else branch,
    /// and incremented pos to 1 (past-end) — leaving the caller with
    /// a bogus consumed-bytes count.
    #[test]
    #[cfg(unix)]
    fn zstrtorlimt_empty_input_does_not_advance_pos() {
        let _g = crate::test_util::global_state_lock();
        let (v, n) = zstrtorlimt("", 0);
        assert_eq!(v, 0, "empty → no digits → 0");
        assert_eq!(
            n, 0,
            "c:285 — past-end NUL behaves like 'not 0' → base=10 path, no advance"
        );
    }

    /// `Src/Builtins/rlimits.c:283-291` — base==0 detects `0x` hex
    /// prefix. Pin so a regression that drops the hex-detection silently
    /// reads `0xff` as octal `0` (then chokes on 'x').
    #[test]
    #[cfg(unix)]
    fn zstrtorlimt_base_zero_detects_hex_prefix() {
        let _g = crate::test_util::global_state_lock();
        let (v, n) = zstrtorlimt("0xff", 0);
        assert_eq!(v, 255, "c:288 — 0x → base 16");
        assert_eq!(n, 4);
        let (v, _) = zstrtorlimt("0XAB", 0);
        assert_eq!(v, 0xAB, "c:288 — 0X (uppercase) also triggers hex");
    }

    /// `Src/Builtins/rlimits.c:283-291` — base==0 with leading `0` (no
    /// `x`/`X`) means octal. Pin so `"0777"` parses to 511.
    #[test]
    #[cfg(unix)]
    fn zstrtorlimt_base_zero_leading_zero_is_octal() {
        let _g = crate::test_util::global_state_lock();
        let (v, n) = zstrtorlimt("0777", 0);
        assert_eq!(v, 511, "c:289 — leading 0 (no x) → base 8");
        assert_eq!(n, 4);
        // Leading 0 then digit out of octal range stops at the digit.
        let (v, n) = zstrtorlimt("089", 0);
        assert_eq!(
            v, 0,
            "c:289 — base 8 doesn't accept '8'/'9'; stops immediately"
        );
        assert_eq!(n, 1, "consumed only the leading '0'");
    }

    /// `Src/Builtins/rlimits.c:294` — base<=10 loop accepts digits in
    /// the range `'0'..='0'+base`. Boundary check for base 2 (binary
    /// would only accept '0' and '1').
    #[test]
    #[cfg(unix)]
    fn zstrtorlimt_base_2_only_accepts_binary_digits() {
        let _g = crate::test_util::global_state_lock();
        let (v, n) = zstrtorlimt("1010", 2);
        assert_eq!(v, 10);
        assert_eq!(n, 4);
        // '2' is NOT a binary digit; consumption stops.
        let (v, n) = zstrtorlimt("12", 2);
        assert_eq!(v, 1, "base 2 stops at '2'");
        assert_eq!(n, 1);
    }

    /// `Src/Builtins/rlimits.c:297-301` — base>10 hex loop accepts
    /// lowercase 'a'..'a'+base-10 AND uppercase 'A'..'A'+base-10. Pin
    /// case-insensitive hex acceptance (and the boundary at base-10).
    #[test]
    #[cfg(unix)]
    fn zstrtorlimt_base_16_accepts_mixed_case_hex() {
        let _g = crate::test_util::global_state_lock();
        let (v, _) = zstrtorlimt("DEADbeef", 16);
        assert_eq!(v, 0xDEADBEEF, "mixed case hex");
        // Char beyond 'f' / 'F' in base 16 stops parse.
        let (v, n) = zstrtorlimt("ffg", 16);
        assert_eq!(v, 0xff);
        assert_eq!(n, 2, "stops at 'g' (out of base-16 range)");
    }

    /// `Src/Builtins/rlimits.c:294,297` — non-numeric leading char
    /// → no consumption, return (0, 0). Pin so the function never
    /// returns past-end pos for bogus input.
    #[test]
    #[cfg(unix)]
    fn zstrtorlimt_non_digit_prefix_consumes_zero_bytes() {
        let _g = crate::test_util::global_state_lock();
        let (v, n) = zstrtorlimt("abc", 10);
        assert_eq!(v, 0);
        assert_eq!(n, 0);
        let (v, n) = zstrtorlimt("!", 16);
        assert_eq!(v, 0);
        assert_eq!(n, 0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Builtins/rlimits.c utilities.
    // ═══════════════════════════════════════════════════════════════════

    /// c:277 — `zstrtorlimt("unlimited", _)` returns (RLIM_INFINITY, 9)
    /// regardless of base.
    #[test]
    #[cfg(unix)]
    fn zstrtorlimt_unlimited_returns_rlim_infinity() {
        let _g = crate::test_util::global_state_lock();
        let (v, n) = zstrtorlimt("unlimited", 10);
        assert_eq!(v, RLIM_INFINITY);
        assert_eq!(n, 9, "exactly 9 bytes consumed for 'unlimited'");
        // Base doesn't matter for the literal-word path.
        let (v16, n16) = zstrtorlimt("unlimited", 16);
        assert_eq!(v16, RLIM_INFINITY);
        assert_eq!(n16, 9);
    }

    /// c:277 — `zstrtorlimt("unlimitedX", _)` is NOT unlimited (exact
    /// strcmp); falls through to digit parser. 'X' isn't a digit so
    /// returns (0, 0).
    #[test]
    #[cfg(unix)]
    fn zstrtorlimt_unlimited_with_suffix_is_not_special() {
        let _g = crate::test_util::global_state_lock();
        // C uses strcmp(s, "unlimited"); s.starts_with would be wrong.
        let (v, n) = zstrtorlimt("unlimitedX", 10);
        // The 'u' isn't a digit → no consumption.
        assert_eq!(v, 0);
        assert_eq!(n, 0);
    }

    /// c:272 — `zstrtorlimt("", _)` returns (0, 0) (empty input).
    #[test]
    #[cfg(unix)]
    fn zstrtorlimt_empty_returns_zero_zero() {
        let _g = crate::test_util::global_state_lock();
        let (v, n) = zstrtorlimt("", 10);
        assert_eq!(v, 0);
        assert_eq!(n, 0);
    }

    /// c:272 — `zstrtorlimt("0", 10)` returns (0, 1).
    #[test]
    #[cfg(unix)]
    fn zstrtorlimt_single_zero_returns_zero_one() {
        let _g = crate::test_util::global_state_lock();
        let (v, n) = zstrtorlimt("0", 10);
        assert_eq!(v, 0);
        assert_eq!(n, 1, "consumed 1 byte");
    }

    /// c:272 — `zstrtorlimt("123abc", 10)` parses '123', stops at 'a'.
    #[test]
    #[cfg(unix)]
    fn zstrtorlimt_stops_at_non_digit_pin() {
        let _g = crate::test_util::global_state_lock();
        let (v, n) = zstrtorlimt("123abc", 10);
        assert_eq!(v, 123);
        assert_eq!(n, 3, "consumed only digit prefix");
    }

    /// c:244 — `find_resource('c')` finds RLIMIT_CORE (option char 'c').
    /// Pin: known POSIX limit char returns a valid (non-negative) index.
    #[test]
    #[cfg(unix)]
    fn find_resource_known_char_returns_nonneg() {
        let _g = crate::test_util::global_state_lock();
        let r = find_resource('c'); // core dump size
        assert!(r >= 0, "core limit option 'c' must resolve, got {}", r);
    }

    /// c:244 — `find_resource` for unknown char returns -1.
    #[test]
    #[cfg(unix)]
    fn find_resource_unknown_char_returns_neg_one() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(find_resource('Q'), -1, "unknown limit option → -1");
        assert_eq!(find_resource('!'), -1);
        assert_eq!(find_resource('\0'), -1);
    }

    /// c:244 — `find_resource` is deterministic across repeated calls.
    #[test]
    #[cfg(unix)]
    fn find_resource_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let first = find_resource('c');
        for _ in 0..10 {
            assert_eq!(find_resource('c'), first);
        }
    }

    /// c:147 (setup_) — returns 0.
    #[test]
    fn rlimits_setup_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(setup_(std::ptr::null()), 0);
    }

    /// c:155-181 — lifecycle stubs all return 0 (or via handlefeatures).
    #[test]
    fn rlimits_lifecycle_stubs_return_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(boot_(std::ptr::null()), 0);
        assert_eq!(cleanup_(std::ptr::null()), 0);
        assert_eq!(finish_(std::ptr::null()), 0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Builtins/rlimits.c
    // c:244 find_resource / c:291 zstrtorlimt / c:699 bin_limit /
    // c:965 bin_unlimit / c:1079 bin_ulimit + lifecycle
    // ═══════════════════════════════════════════════════════════════════

    /// c:244 — `find_resource` returns i32 (compile-time pin).
    #[cfg(unix)]
    #[test]
    fn find_resource_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = find_resource('c');
    }

    /// c:244 — `find_resource('f')` (file size) resolves to known limit.
    #[cfg(unix)]
    #[test]
    fn find_resource_file_size_resolves() {
        let _g = crate::test_util::global_state_lock();
        let r = find_resource('f');
        assert!(r >= 0, "file-size 'f' limit must resolve, got {}", r);
    }

    /// c:244 — `find_resource('t')` (cpu time) resolves to known limit.
    #[cfg(unix)]
    #[test]
    fn find_resource_cpu_time_resolves() {
        let _g = crate::test_util::global_state_lock();
        let r = find_resource('t');
        assert!(r >= 0, "cpu-time 't' limit must resolve, got {}", r);
    }

    /// c:291 — `zstrtorlimt("100", 10)` returns (100, 3).
    #[cfg(unix)]
    #[test]
    fn zstrtorlimt_three_digit_returns_value_and_count() {
        let _g = crate::test_util::global_state_lock();
        let (v, n) = zstrtorlimt("100", 10);
        assert_eq!(v, 100, "100 parses to 100");
        assert_eq!(n, 3, "consumed 3 bytes");
    }

    /// c:699 — `bin_limit` returns i32 (compile-time pin).
    #[cfg(unix)]
    #[test]
    fn bin_limit_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let _: i32 = bin_limit("limit", &[], &ops, 0);
    }

    /// c:965 — `bin_unlimit` returns i32 (compile-time pin).
    #[cfg(unix)]
    #[test]
    fn bin_unlimit_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let _: i32 = bin_unlimit("unlimit", &[], &ops, 0);
    }

    /// c:1079 — `bin_ulimit` returns i32 (compile-time pin).
    #[cfg(unix)]
    #[test]
    fn bin_ulimit_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let _: i32 = bin_ulimit("ulimit", &[], &ops, 0);
    }

    /// c:699/965/1079 — all rlimit builtin exit codes non-negative.
    #[cfg(unix)]
    #[test]
    fn rlimit_builtins_exit_codes_non_negative() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        for r in [
            bin_limit("limit", &[], &ops, 0),
            bin_unlimit("unlimit", &[], &ops, 0),
            bin_ulimit("ulimit", &[], &ops, 0),
        ] {
            assert!(r >= 0, "exit code must be non-negative, got {}", r);
        }
    }

    /// c:244 — `find_resource` for full a-z/A-Z sweep deterministic.
    /// Pins the lookup table stability across the full option-char alphabet.
    #[cfg(unix)]
    #[test]
    fn find_resource_full_alphabet_sweep_deterministic() {
        let _g = crate::test_util::global_state_lock();
        for c in ('a'..='z').chain('A'..='Z') {
            let first = find_resource(c);
            for _ in 0..3 {
                assert_eq!(
                    find_resource(c),
                    first,
                    "find_resource({:?}) must be pure",
                    c
                );
            }
        }
    }

    /// c:1336 — `features_` returns i32 (compile-time pin).
    #[test]
    fn rlimits_features_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let mut v: Vec<String> = Vec::new();
        let _: i32 = features_(std::ptr::null(), &mut v);
    }

    /// c:1343 — `enables_` returns i32 + None-out safe.
    #[test]
    fn rlimits_enables_with_none_returns_i32() {
        let _g = crate::test_util::global_state_lock();
        let mut e: Option<Vec<i32>> = None;
        let _: i32 = enables_(std::ptr::null(), &mut e);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity pins for Src/Modules/rlimits.c
    // c:699 bin_limit / c:965 bin_unlimit / c:1079 bin_ulimit /
    // c:1330-1364 lifecycle hooks
    // ═══════════════════════════════════════════════════════════════════

    /// c:1330 — `setup_` is idempotent.
    #[test]
    fn rlimits_setup_idempotent_alt_pin() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(setup_(std::ptr::null()), 0);
        }
    }

    /// c:1357 — `cleanup_` is idempotent.
    #[test]
    fn rlimits_cleanup_idempotent_alt_pin() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(cleanup_(std::ptr::null()), 0);
        }
    }

    /// c:1364 — `finish_` is idempotent.
    #[test]
    fn rlimits_finish_idempotent_alt_pin() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(finish_(std::ptr::null()), 0);
        }
    }

    /// c:1350 — `boot_` is idempotent.
    #[test]
    fn rlimits_boot_idempotent_alt_pin() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            let _ = boot_(std::ptr::null());
        }
    }

    /// c:1330 — `setup_` return type i32 (compile-time pin).
    #[test]
    fn rlimits_setup_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = setup_(std::ptr::null());
    }

    /// c:1357 — `cleanup_` return type i32 (compile-time pin).
    #[test]
    fn rlimits_cleanup_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = cleanup_(std::ptr::null());
    }

    /// c:1364 — `finish_` return type i32 (compile-time pin).
    #[test]
    fn rlimits_finish_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = finish_(std::ptr::null());
    }

    /// c:699 — `bin_limit` empty args non-negative.
    #[test]
    fn bin_limit_empty_args_non_negative() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let r = bin_limit("limit", &[], &ops, 0);
        assert!(r >= 0, "bin_limit empty must be ≥ 0, got {}", r);
    }

    /// c:965 — `bin_unlimit` empty args non-negative.
    #[test]
    fn bin_unlimit_empty_args_non_negative() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let r = bin_unlimit("unlimit", &[], &ops, 0);
        assert!(r >= 0, "bin_unlimit empty must be ≥ 0, got {}", r);
    }

    /// c:1079 — `bin_ulimit` empty args non-negative.
    #[test]
    fn bin_ulimit_empty_args_non_negative() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let r = bin_ulimit("ulimit", &[], &ops, 0);
        assert!(r >= 0, "bin_ulimit empty must be ≥ 0, got {}", r);
    }

    /// c:699 — `bin_limit` various func values don't panic.
    #[test]
    fn bin_limit_various_func_values_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        for func in [-1, 0, 1, 100, i32::MAX] {
            let _ = bin_limit("limit", &[], &ops, func);
        }
    }

    /// c:1336 — `features_` is deterministic across calls.
    #[test]
    fn rlimits_features_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let mut v1: Vec<String> = Vec::new();
        let mut v2: Vec<String> = Vec::new();
        let _ = features_(std::ptr::null(), &mut v1);
        let _ = features_(std::ptr::null(), &mut v2);
        assert_eq!(
            v1, v2,
            "features_ must populate identical vec for identical input"
        );
    }
}

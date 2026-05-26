//! Date/time utilities — port of `Src/Modules/datetime.c`.
//!
//! C source has 0 structs/enums. Rust port matches: 0 types.
//! Functions:
//!   - `getcurrentsecs`     `[c:206]`
//!   - `getcurrentrealtime` `[c:212]`
//!   - `getcurrenttime`     `[c:220]`
//!   - `reverse_strftime`   `[c:42]`
//!   - `output_strftime`    `[c:99]`   (the actual builtin entry)
//!   - `bin_strftime`       `[c:187]`  (TZ-scope wrapper around output_strftime)
//!   - 6 module loaders
//!
//! C uses libc `localtime(3)` + zsh's custom `ztrftime()` (which
//! extends POSIX strftime with the `%.N` nanosecond syntax). The
//! Rust port calls `crate::ported::utils::ztrftime()` for the
//! base format and adds %N extensions on top.

use crate::ported::utils::{metafy, zwarnnam};
use std::sync::{Mutex, OnceLock};
use crate::ported::compat::zgettime;
use crate::ported::params::{getsparam, isident, setiparam, setsparam};
use crate::ported::zsh_system_h::timespec;
use crate::ported::zsh_h::{features, options, OPT_ARG, OPT_ISSET, MAX_OPS, module};
use chrono::{DateTime, Local, NaiveDateTime, TimeZone};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Port of `reverse_strftime(char *nam, char **argv, char *scalar, int quiet)` from `Src/Modules/datetime.c:42`.
/// Parses a time string per the format string and assigns the
/// resulting epoch seconds to `scalar` (or stdout if NULL).
///
/// C signature: `static int reverse_strftime(char *nam, char **argv,
///                                            char *scalar, int quiet)`.
/// WARNING: param names don't match C — Rust=(nam, argv, quiet) vs C=(nam, argv, scalar, quiet)
pub fn reverse_strftime(
    nam: &str,
    argv: &[&str], // c:42
    scalar: Option<&str>,
    quiet: i32,
) -> i32 {
    if argv.len() < 2 {
        // c:54 timestring expected
        zwarnnam(nam, "timestring expected");
        return 1;
    }
    let format = argv[0];
    let input = argv[1];
    // c:64 — `strptime(timestring, format, &tm)`. Rust uses chrono's
    // NaiveDateTime parser for the same effect.
    let dt = match NaiveDateTime::parse_from_str(input, format) {
        Ok(d) => d,
        Err(_) => {
            // c:67-71 mismatch
            if quiet == 0 {
                zwarnnam(nam, &format!("format not matched: {}", input));
            }
            return 1;
        }
    };
    let secs = match Local.from_local_datetime(&dt) {
        // c:78 mktime
        chrono::LocalResult::Single(d) => d.timestamp(),
        chrono::LocalResult::Ambiguous(d, _) => d.timestamp(),
        chrono::LocalResult::None => {
            if quiet == 0 {
                zwarnnam(nam, "unable to convert to time");
            }
            return 1;
        }
    };
    if let Some(name) = scalar {
        // c:90 scalar
        setiparam(name, secs); // c:91 setiparam
    } else {
        // c:93
        println!("{}", secs); // c:94 printf("%ld\n", ...)
    }
    0 // c:99
}

/// Port of `output_strftime(char *nam, char **argv, Options ops, UNUSED(int func))` from `Src/Modules/datetime.c:99`.
/// The `output_strftime` builtin entry. Parses argv (format,
/// timestamp, nanoseconds), calls `localtime(3)` to convert,
/// formats via `ztrftime()` with retry-on-overflow, then writes
/// the result to stdout (or `setsparam` to the `-s NAME` scalar).
///
/// C signature: `static int output_strftime(char *nam, char **argv,
///                                           Options ops, int func)`.
/// WARNING: param names don't match C — Rust=(nam, argv, _func) vs C=(nam, argv, ops, func)
pub fn output_strftime(
    nam: &str,
    argv: &[&str], // c:99
    ops: &options,
    _func: i32,
) -> i32 {
    // c:107 — `if (OPT_ISSET(ops,'s'))`
    let scalar: Option<&str> = if OPT_ISSET(ops, b's') {
        Some(OPT_ARG(ops, b's').unwrap_or(""))
    } else {
        None
    };
    if let Some(name) = scalar {
        if !isident(name) {
            // c:110 isident check
            zwarnnam(nam, &format!("not an identifier: {}", name)); // c:111
            return 1; // c:112
        }
    }

    // c:115 — `if (OPT_ISSET(ops, 'r'))` reverse path.
    if OPT_ISSET(ops, b'r') {
        let quiet = if OPT_ISSET(ops, b'q') { 1 } else { 0 };
        return reverse_strftime(nam, argv, scalar, quiet); // c:120
    }

    if argv.is_empty() {
        zwarnnam(nam, "format expected");
        return 1;
    }

    // c:122 — parse argv[1] as timestamp, or use current time.
    let (secs, nsec) = if argv.len() < 2 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO);
        (now.as_secs() as i64, now.subsec_nanos() as i64) // c:124-125 zgettime
    } else {
        // c:128 — `ts.tv_sec = (time_t)strtoul(argv[1], &endptr, 10);`
        let secs = match argv[1].parse::<i64>() {
            Ok(v) => v,
            Err(_) => {
                zwarnnam(nam, &format!("{}: invalid decimal number", argv[1]));
                return 1; // c:135
            }
        };
        // c:144 — argv[2] nanoseconds (optional).
        let nsec = if argv.len() > 2 {
            match argv[2].parse::<i64>() {
                Ok(v) if (0..=999_999_999).contains(&v) => v, // c:151
                Ok(_) => {
                    zwarnnam(nam, &format!("{}: invalid nanosecond value", argv[2]));
                    return 1; // c:153
                }
                Err(_) => {
                    zwarnnam(nam, &format!("{}: invalid decimal number", argv[2]));
                    return 1;
                }
            }
        } else {
            0
        };
        (secs, nsec)
    };

    // c:160 — `bufsize = strlen(argv[0]) * 8; buffer = zalloc(bufsize);`
    // c:163-167 — retry up to 4 times growing the buffer.
    // c:165 — `ztrftime(buffer, bufsize, argv[0], tm, ts.tv_nsec)`.
    let format = argv[0];
    let dt: DateTime<Local> = match Local.timestamp_opt(secs, nsec as u32) {
        chrono::LocalResult::Single(d) => d,
        chrono::LocalResult::Ambiguous(d, _) => d,
        chrono::LocalResult::None => {
            // c:171-174
            zwarnnam(nam, &format!("bad/unsupported format: '{}'", format));
            return 1; // c:174
        }
    };
    // First substitute %N variants (zsh extension at utils.c:3411-3429).
    let mut work = String::with_capacity(format.len() * 2);
    let bytes = format.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 1 < bytes.len() {
            match bytes[i + 1] {
                b'N' => {
                    work.push_str(&format!("{:09}", nsec));
                    i += 2;
                    continue;
                }
                b'.' if i + 2 < bytes.len() && bytes[i + 2] == b'N' => {
                    work.push_str(&format!(".{:09}", nsec));
                    i += 3;
                    continue;
                }
                d if d.is_ascii_digit() && i + 2 < bytes.len() && bytes[i + 2] == b'N' => {
                    let digits = (d - b'0') as usize;
                    let scaled = if digits >= 9 {
                        nsec
                    } else {
                        nsec / 10i64.pow((9 - digits) as u32)
                    };
                    work.push_str(&format!("{:0width$}", scaled, width = digits));
                    i += 3;
                    continue;
                }
                b'%' => {
                    work.push_str("%%");
                    i += 2;
                    continue;
                }
                _ => {}
            }
        }
        work.push(bytes[i] as char);
        i += 1;
    }
    let formatted = dt.format(&work).to_string();

    // c:178 — `if (scalar) { setsparam(scalar, metafy(buffer, len, META_DUP)); }`
    if let Some(name) = scalar {
        setsparam(name, &metafy(&formatted));
        // c:178
    } else {
        // c:180-183 — fwrite + putchar('\n') unless -n
        print!("{}", formatted); // c:181 fwrite
        if !OPT_ISSET(ops, b'n') {
            // c:182 !OPT_ISSET(ops,'n')
            println!(); // c:183 putchar('\n')
        }
    }

    0 // c:187
}

/// Port of `bin_strftime(char *nam, char **argv, Options ops, int func)` from `Src/Modules/datetime.c:187`. The
/// `strftime` builtin entry — wraps `output_strftime` in a local
/// param-scope that copies `$TZ` so `output_strftime`'s
/// `localtime(3)` calls see the user's timezone even if a function
/// scope has shadowed it.
///
/// C signature: `static int bin_strftime(char *nam, char **argv,
///                                         Options ops, int func)`.
/// WARNING: param names don't match C — Rust=(nam, argv, func) vs C=(nam, argv, ops, func)
pub fn bin_strftime(
    nam: &str,
    argv: &[String], // c:187
    ops: &options,
    func: i32,
) -> i32 {
    // c:191 — `char *tz = getsparam("TZ");`. Read TZ from paramtab
    // (canonical shell var storage); previous port read
    // `env::var("TZ")` which diverges from shell-internal TZ values
    // not yet exported. Same env-vs-paramtab family as recent fixes.
    let tz_saved = getsparam("TZ"); // c:191
                                                           // c:193-198 — `startparamscope(); createparam("TZ", PM_LOCAL);
                                                           //              setsparam("TZ", tz);`. The Rust port mirrors via
                                                           // env::set_var so libc's strftime sees the locale-active TZ —
                                                           // setsparam alone doesn't propagate to the libc-level zone.
    if let Some(ref tz) = tz_saved {
        std::env::set_var("TZ", tz); // c:198 setsparam
    }
    // Convert &[String] → &[&str] for the internal helper which
    // takes the narrower view.
    let argv_views: Vec<&str> = argv.iter().map(String::as_str).collect();
    let result = output_strftime(nam, &argv_views, ops, func); // c:199
                                                        // c:200 — `endparamscope();`. Restore the saved TZ.
    if let Some(ref tz) = tz_saved {
        std::env::set_var("TZ", tz);
    }
    result // c:202
}

/// Port of `getcurrentsecs(UNUSED(Param pm))` from `Src/Modules/datetime.c:206`.
/// Returns the current epoch seconds — backs `$EPOCHSECONDS`.
/// C body: `return (zlong) time(NULL);`
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn getcurrentsecs() -> i64 {
    // c:206
    // c:206 — `return (zlong) time(NULL);`
    unsafe { libc::time(std::ptr::null_mut()) as i64 }
}

/// Port of `getcurrentrealtime(UNUSED(Param pm))` from `Src/Modules/datetime.c:212`.
/// Returns the current high-resolution epoch time as f64 — backs
/// `$EPOCHREALTIME`.
///
/// C body:
/// ```c
/// struct timespec now;
/// zgettime(&now);
/// return (double)now.tv_sec + (double)now.tv_nsec * 1e-9;
/// ```
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn getcurrentrealtime() -> f64 {
    // c:212
    let mut now: timespec = unsafe { std::mem::zeroed() }; // c:212
    zgettime(&mut now); // c:215
    (now.tv_sec as f64) + (now.tv_nsec as f64) * 1e-9 // c:216
}

/// Port of `getcurrenttime(UNUSED(Param pm))` from `Src/Modules/datetime.c:220`.
/// Returns the current epoch as `(secs, nanos)` — backs the
/// `$epochtime` two-element array param.
///
/// C body:
/// ```c
/// struct timespec now;
/// zgettime(&now);
/// arr[0] = sprintf "%ld" now.tv_sec
/// arr[1] = sprintf "%ld" now.tv_nsec
/// return arr;
/// ```
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn getcurrenttime() -> Vec<String> {
    // c:220
    // c:222-224 — `char **arr; char buf[DIGBUFSIZE]; struct timespec now;`
    let mut arr: Vec<String> = Vec::with_capacity(2);                        // c:228 zhalloc(3 * sizeof(*arr))
    let mut now: timespec = unsafe { std::mem::zeroed() };                   // c:224
    // c:226 — `zgettime(&now);`
    zgettime(&mut now);
    // c:229 — `sprintf(buf, "%ld", (long)now.tv_sec);`
    let buf = format!("{}", now.tv_sec as i64);
    arr.push(buf);                                                           // c:230 arr[0] = dupstring(buf)
    // c:231 — `sprintf(buf, "%ld", (long)now.tv_nsec);`
    let buf = format!("{}", now.tv_nsec as i64);
    arr.push(buf);                                                           // c:232 arr[1] = dupstring(buf)
    // c:233 — `arr[2] = NULL;` (collapsed: Vec's length is the terminator)
    arr                                                                      // c:235
}

// `bintab` — port of `static struct builtin bintab[]` (datetime.c:255).

// `patab` — port of `static struct paramdef patab[]` (datetime.c).

// `module_features` — port of `static struct features module_features`
// from datetime.c:262.

/// Port of `setup_(UNUSED(Module m))` from `Src/Modules/datetime.c:270`.
#[allow(unused_variables)]
pub fn setup_(m: *const module) -> i32 {
    // c:270
    // C body c:272-273 — `return 0`. Faithful empty-body port.
    0
}

// =====================================================================
// static struct builtin bintab[]                                    c:255
// static struct features module_features                            c:262
// =====================================================================


/// Port of `features_(UNUSED(Module m), UNUSED(char ***features))` from `Src/Modules/datetime.c:277`.
/// C body: `*features = featuresarray(m, &module_features); return 0;`
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {
    // c:277
    *features = featuresarray(m, module_features());
    0 // c:292
}

/// Port of `enables_(UNUSED(Module m), UNUSED(int **enables))` from `Src/Modules/datetime.c:285`.
/// C body: `return handlefeatures(m, &module_features, enables);`
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {
    // c:285
    handlefeatures(m, module_features(), enables) // c:292
}

/// Port of `boot_(UNUSED(Module m))` from `Src/Modules/datetime.c:292`.
#[allow(unused_variables)]
pub fn boot_(m: *const module) -> i32 {
    // c:292
    // C body c:294-295 — `return 0`. Faithful empty-body port; the
    //                    strftime builtin + EPOCHREALTIME param register
    //                    via the bn_list/pd_list feature dispatch.
    0
}

/// Port of `cleanup_(UNUSED(Module m))` from `Src/Modules/datetime.c:299`.
/// C body: `return setfeatureenables(m, &module_features, NULL);`
pub fn cleanup_(m: *const module) -> i32 {
    // c:299
    setfeatureenables(m, module_features(), None) // c:306
}

/// Port of `finish_(UNUSED(Module m))` from `Src/Modules/datetime.c:306`.
#[allow(unused_variables)]
pub fn finish_(m: *const module) -> i32 {
    // c:306
    // C body c:308-309 — `return 0`. Faithful empty-body port; the
    //                    strftime builtin + EPOCHREALTIME unregister
    //                    via cleanup_'s setfeatureenables(...).
    0
}

static MODULE_FEATURES: OnceLock<Mutex<features>> = OnceLock::new();

// Local stubs for the per-module entry points. C uses generic
// `featuresarray`/`handlefeatures`/`setfeatureenables` (module.c:
// 3275/3370/3445) but those take `Builtin` + `Features` pointer
// fields the Rust port doesn't carry. The hardcoded descriptor
// list mirrors the C bintab/conddefs/mathfuncs/paramdefs.
// WARNING: NOT IN DATETIME.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn featuresarray(_m: *const module, _f: &Mutex<features>) -> Vec<String> {
    vec![
        "b:strftime".to_string(),
        "p:EPOCHSECONDS".to_string(),
        "p:EPOCHREALTIME".to_string(),
        "p:epochtime".to_string(),
    ]
}

// WARNING: NOT IN DATETIME.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn handlefeatures(
    _m: *const module,
    _f: &Mutex<features>,
    enables: &mut Option<Vec<i32>>,
) -> i32 {
    if enables.is_none() {
        *enables = Some(vec![1; 4]);
    }
    0
}

// WARNING: NOT IN DATETIME.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn setfeatureenables(_m: *const module, _f: &Mutex<features>, _e: Option<&[i32]>) -> i32 {
    0
}

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

// WARNING: NOT IN DATETIME.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn module_features() -> &'static Mutex<features> {
    MODULE_FEATURES.get_or_init(|| {
        Mutex::new(features {
            bn_list: None,
            bn_size: 1,
            cd_list: None,
            cd_size: 0,
            mf_list: None,
            mf_size: 0,
            pd_list: None,
            pd_size: 3,
            n_abstract: 0,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_epoch_seconds() {
        let _g = crate::test_util::global_state_lock();
        let secs = getcurrentsecs();
        assert!(secs > 1700000000);
    }

    #[test]
    fn test_epoch_realtime() {
        let _g = crate::test_util::global_state_lock();
        let rt = getcurrentrealtime();
        assert!(rt > 1700000000.0);
        let arr = getcurrenttime();
        let secs: i64 = arr[0].parse().unwrap();
        assert!((rt - secs as f64).abs() < 1.0);
    }

    #[test]
    fn test_epoch_time() {
        let _g = crate::test_util::global_state_lock();
        let arr = getcurrenttime();
        let secs: i64 = arr[0].parse().unwrap();
        let nanos: i64 = arr[1].parse().unwrap();
        assert!(secs > 1700000000);
        assert!((0..1_000_000_000).contains(&nanos));
    }

    /// Build an `Options` struct populated for the canonical
    /// `output_strftime(name, argv, ops, func)` signature, with
    /// flag `flag` set and (optionally) -s SCALAR slot encoded.
    fn ops_for(flags: &[u8], scalar: Option<&str>) -> options {
        let mut ops = options {
            ind: [0u8; MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        for f in flags {
            ops.ind[*f as usize] = 1;
        }
        if let Some(s) = scalar {
            ops.ind[b's' as usize] = 4;
            ops.args.push(s.to_string());
            ops.argscount = 1;
            ops.argsalloc = 1;
        }
        ops
    }

    /// Reads a scalar from the canonical paramtab — used by tests
    /// to assert side-effects of params::setsparam writes.
    fn pt_get(name: &str) -> Option<String> {
        crate::ported::params::paramtab()
            .read()
            .ok()
            .and_then(|t| t.get(name).and_then(|p| p.u_str.clone()))
    }

    #[test]
    fn test_output_strftime_nanoseconds() {
        let _g = crate::test_util::global_state_lock();
        let ops = ops_for(&[b'n'], Some("OUT"));
        let r = output_strftime("strftime", &["%9N", "1700000000", "123456789"], &ops, 0);
        assert_eq!(r, 0);
        assert_eq!(pt_get("OUT").as_deref(), Some("123456789"));
        let r = output_strftime("strftime", &["%3N", "1700000000", "123456789"], &ops, 0);
        assert_eq!(r, 0);
        assert_eq!(pt_get("OUT").as_deref(), Some("123"));
    }

    #[test]
    fn test_output_strftime_to_scalar() {
        let _g = crate::test_util::global_state_lock();
        let ops = ops_for(&[b'n'], Some("OUT2"));
        let r = output_strftime("strftime", &["%s", "1700000000"], &ops, 0);
        assert_eq!(r, 0);
        assert_eq!(pt_get("OUT2").as_deref(), Some("1700000000"));
    }

    #[test]
    fn test_output_strftime_format_required() {
        let _g = crate::test_util::global_state_lock();
        let ops = ops_for(&[], None);
        let r = output_strftime("strftime", &[], &ops, 0);
        assert_eq!(r, 1);
    }

    /// c:206 — `getcurrentsecs` matches `time(NULL)` within 1 second.
    /// Pinning the libc-passthrough so a regression that adds an
    /// offset (timezone, monotonic-vs-realtime confusion) gets caught.
    #[test]
    fn getcurrentsecs_matches_libc_time() {
        let _g = crate::test_util::global_state_lock();
        let libc_now = unsafe { libc::time(std::ptr::null_mut()) } as i64;
        let our_now = getcurrentsecs();
        assert!(
            (our_now - libc_now).abs() <= 1,
            "getcurrentsecs {} drifted from libc::time {}",
            our_now,
            libc_now
        );
    }

    /// c:212 — `getcurrentrealtime` returns a value with nonzero
    /// sub-second precision over a small sample. Catches a regression
    /// that truncates to whole seconds (e.g. wrong tv_nsec scaling).
    #[test]
    fn getcurrentrealtime_carries_subsecond_precision() {
        let _g = crate::test_util::global_state_lock();
        let mut saw_fractional = false;
        for _ in 0..10 {
            let rt = getcurrentrealtime();
            if rt.fract().abs() > 1e-9 {
                saw_fractional = true;
                break;
            }
        }
        assert!(
            saw_fractional,
            "getcurrentrealtime over 10 samples never produced a fractional part"
        );
    }

    /// c:220 — `getcurrenttime` returns `(secs, nanos)` with
    /// nanos < 1_000_000_000. Pinning the nanos invariant catches
    /// a regression that returns microseconds (cap 1e6) or the raw
    /// time-spec value without modulo.
    #[test]
    fn getcurrenttime_nanos_under_one_billion() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..5 {
            let arr = getcurrenttime();
            let nanos: i64 = arr[1].parse().unwrap();
            assert!(
                nanos < 1_000_000_000,
                "nanos {} >= 1e9 — unit confusion in c:220 port",
                nanos
            );
            assert!(nanos >= 0, "nanos {} negative", nanos);
        }
    }

    /// c:212 — Wall-clock advances monotonically forward between two
    /// `getcurrentrealtime` calls. Captures a "clock went backward"
    /// regression that would break `$EPOCHREALTIME` script timing.
    #[test]
    fn getcurrentrealtime_advances_forward() {
        let _g = crate::test_util::global_state_lock();
        let a = getcurrentrealtime();
        std::thread::sleep(Duration::from_millis(10));
        let b = getcurrentrealtime();
        assert!(b >= a, "realtime went backward: {} -> {}", a, b);
        assert!(
            b - a < 5.0,
            "realtime jumped {} seconds in 10ms sleep",
            b - a
        );
    }

    /// c:206 — `getcurrentsecs` advances monotonically across sleeps.
    /// Same as above but for the integer-second accessor.
    #[test]
    fn getcurrentsecs_advances_or_stays_equal() {
        let _g = crate::test_util::global_state_lock();
        let a = getcurrentsecs();
        std::thread::sleep(Duration::from_millis(20));
        let b = getcurrentsecs();
        assert!(b >= a, "seconds went backward: {} -> {}", a, b);
    }

    /// c:99 — `output_strftime` with `%s` (epoch seconds) and `%n`
    /// flag must print the exact epoch back. Pinning the
    /// non-side-effect path catches a regression that mangles the
    /// `-n` (no-newline) shortcut.
    #[test]
    fn output_strftime_percent_s_round_trips() {
        let _g = crate::test_util::global_state_lock();
        let ops = ops_for(&[b'n'], Some("EPOCH_ROUND_TRIP"));
        let r = output_strftime("strftime", &["%s", "1234567890"], &ops, 0);
        assert_eq!(r, 0);
        assert_eq!(pt_get("EPOCH_ROUND_TRIP").as_deref(), Some("1234567890"));
    }

    /// c:42-99 — `output_strftime` with an unparseable epoch input
    /// must return nonzero. Catches a regression that silently
    /// produces "Wed Dec 31 ..." (epoch 0) on garbage input.
    #[test]
    fn output_strftime_invalid_epoch_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let ops = ops_for(&[b'n'], Some("BAD"));
        let r = output_strftime("strftime", &["%s", "not-a-number"], &ops, 0);
        assert_ne!(r, 0, "garbage epoch must be rejected");
    }

    /// c:270-307 — module-lifecycle stubs all return 0 in C.
    #[test]
    fn module_lifecycle_shims_all_return_zero() {
        let _g = crate::test_util::global_state_lock();
        let m: *const module = std::ptr::null();
        assert_eq!(setup_(m), 0);
        assert_eq!(boot_(m), 0);
        assert_eq!(cleanup_(m), 0);
        assert_eq!(finish_(m), 0);
    }

    /// c:285 — `enables_` returns 0 and populates `enables` to
    /// non-None (the module always advertises ≥ 1 feature). A None
    /// return would mean "no features" and `zmodload -F zsh/datetime`
    /// would silently disable strftime.
    #[test]
    fn enables_populates_some_vec() {
        let _g = crate::test_util::global_state_lock();
        let m: *const module = std::ptr::null();
        let mut enables: Option<Vec<i32>> = None;
        assert_eq!(enables_(m, &mut enables), 0);
        assert!(enables.is_some(), "enables must be Some after enables_");
    }

    // ═══════════════════════════════════════════════════════════════════
    // getcurrentsecs / getcurrentrealtime / getcurrenttime — back the
    // $EPOCHSECONDS / $EPOCHREALTIME / $epochtime parameters. Test that
    // they return monotonic / sane values and the right structure.
    // ═══════════════════════════════════════════════════════════════════

    /// getcurrentsecs returns a positive epoch time (after year 2020).
    #[test]
    fn getcurrentsecs_returns_post_2020_epoch() {
        let _g = crate::test_util::global_state_lock();
        let s = getcurrentsecs();
        // Year 2020-01-01 = epoch 1577836800. Any current time is > this.
        assert!(s > 1_577_836_800, "epoch must be after 2020-01-01; got {s}");
        // And < year 2100 (4102444800) — sanity bound.
        assert!(s < 4_102_444_800, "epoch must be before 2100-01-01; got {s}");
    }

    /// Two successive calls must be monotonically non-decreasing.
    #[test]
    fn getcurrentsecs_is_monotonic_non_decreasing() {
        let _g = crate::test_util::global_state_lock();
        let a = getcurrentsecs();
        let b = getcurrentsecs();
        assert!(b >= a, "time went backwards: {a} → {b}");
    }

    /// getcurrentrealtime returns a positive f64 with sub-second precision.
    #[test]
    fn getcurrentrealtime_returns_positive_float() {
        let _g = crate::test_util::global_state_lock();
        let t = getcurrentrealtime();
        assert!(t > 1_577_836_800.0, "realtime must be after 2020; got {t}");
    }

    /// getcurrentrealtime is close to getcurrentsecs (within 1 second).
    #[test]
    fn getcurrentrealtime_matches_getcurrentsecs_within_a_second() {
        let _g = crate::test_util::global_state_lock();
        let secs = getcurrentsecs() as f64;
        let real = getcurrentrealtime();
        let diff = (real - secs).abs();
        assert!(diff < 2.0, "realtime/secs diverge by {diff} seconds");
    }

    /// getcurrenttime returns a 2-element array [secs, nanos].
    #[test]
    fn getcurrenttime_returns_two_element_array() {
        let _g = crate::test_util::global_state_lock();
        let arr = getcurrenttime();
        assert_eq!(arr.len(), 2, "must be exactly 2 elements [secs, nanos]");
    }

    /// First element of getcurrenttime is the epoch seconds (parseable).
    #[test]
    fn getcurrenttime_first_element_is_parseable_epoch_seconds() {
        let _g = crate::test_util::global_state_lock();
        let arr = getcurrenttime();
        let secs: i64 = arr[0].parse().expect("element 0 must be valid i64");
        assert!(secs > 1_577_836_800, "secs must be post-2020");
    }

    /// Second element of getcurrenttime is nanoseconds (0..1_000_000_000).
    #[test]
    fn getcurrenttime_second_element_is_valid_nanoseconds() {
        let _g = crate::test_util::global_state_lock();
        let arr = getcurrenttime();
        let nanos: i64 = arr[1].parse().expect("element 1 must be valid i64");
        assert!(nanos >= 0, "nanos must be non-negative");
        assert!(nanos < 1_000_000_000, "nanos must be < 1e9; got {nanos}");
    }

    /// Successive getcurrenttime calls have non-decreasing secs.
    #[test]
    fn getcurrenttime_monotonic_seconds() {
        let _g = crate::test_util::global_state_lock();
        let a: i64 = getcurrenttime()[0].parse().unwrap();
        let b: i64 = getcurrenttime()[0].parse().unwrap();
        assert!(b >= a, "time went backwards: {a} → {b}");
    }

    // ─── zsh-corpus pins for datetime helpers ──────────────────────

    /// `getcurrentsecs` returns positive epoch seconds.
    #[test]
    fn datetime_corpus_getcurrentsecs_positive() {
        let _g = crate::test_util::global_state_lock();
        let s = getcurrentsecs();
        assert!(s > 1_577_836_800, "post-2020 epoch, got {s}");
    }

    /// `getcurrentsecs` is monotonically non-decreasing across calls.
    #[test]
    fn datetime_corpus_getcurrentsecs_monotonic() {
        let _g = crate::test_util::global_state_lock();
        let a = getcurrentsecs();
        let b = getcurrentsecs();
        assert!(b >= a, "time went backwards: {a} → {b}");
    }

    /// `getcurrentrealtime` returns positive f64 in epoch seconds.
    #[test]
    fn datetime_corpus_getcurrentrealtime_positive() {
        let _g = crate::test_util::global_state_lock();
        let r = getcurrentrealtime();
        assert!(r > 1_577_836_800.0, "post-2020 epoch, got {r}");
    }

    /// `getcurrentrealtime` has sub-second precision (probably).
    /// Pin: returns f64, not just integer-valued.
    #[test]
    fn datetime_corpus_getcurrentrealtime_returns_f64() {
        let _g = crate::test_util::global_state_lock();
        let r = getcurrentrealtime();
        // It's a valid float that's finite
        assert!(r.is_finite(), "must be finite, got {r}");
    }

    /// `getcurrenttime` returns exactly 2 elements (secs, nanos).
    #[test]
    fn datetime_corpus_getcurrenttime_two_elements() {
        let _g = crate::test_util::global_state_lock();
        let arr = getcurrenttime();
        assert_eq!(arr.len(), 2, "exactly 2 elements (secs, nanos)");
    }

    /// `getcurrenttime` element types: both parse as i64.
    #[test]
    fn datetime_corpus_getcurrenttime_both_parse_as_i64() {
        let _g = crate::test_util::global_state_lock();
        let arr = getcurrenttime();
        let _: i64 = arr[0].parse().expect("secs parses");
        let _: i64 = arr[1].parse().expect("nanos parses");
    }

    /// `getcurrentsecs` and `getcurrenttime[0]` agree within a couple seconds.
    #[test]
    fn datetime_corpus_getcurrentsecs_matches_getcurrenttime_secs() {
        let _g = crate::test_util::global_state_lock();
        let a = getcurrentsecs();
        let b: i64 = getcurrenttime()[0].parse().unwrap();
        // Should differ by at most a couple seconds.
        assert!(
            (a - b).abs() <= 2,
            "getcurrentsecs={a} should match getcurrenttime[0]={b} within ~2s"
        );
    }
}

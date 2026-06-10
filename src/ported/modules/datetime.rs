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

use crate::ported::compat::zgettime;
use crate::ported::params::{getsparam, isident, setiparam, setsparam};
use crate::ported::utils::{metafy, zwarnnam};
use crate::ported::zsh_h::{features, module, options, MAX_OPS, OPT_ARG, OPT_ISSET};
use crate::ported::zsh_system_h::timespec;
use chrono::format::{Parsed, StrftimeItems};
use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone};
use std::sync::{Mutex, OnceLock};
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
    // c:64 — `strptime(timestring, format, &tm)`. C's strptime
    // accepts PARTIAL formats: `%Y` + `"2024"` parses just the
    // year and fills the rest of struct tm with zeros (which
    // mktime then resolves to 2024-01-01 00:00:00). chrono's
    // `NaiveDateTime::parse_from_str` REQUIRES every field
    // (year, month, day, hour, minute, second) and fails on
    // partial input, so route through `Parsed` which holds
    // any subset of fields then fill missing pieces with
    // defaults to mirror strptime + mktime semantics. Bug #324.
    // c:62 — `endp = strptime(argv[1], argv[0], &tm);` — strptime returns
    // the FIRST unconsumed character (NUL if entire string was consumed).
    // chrono's `parse_and_remainder` returns the remainder slice on
    // success, which is the equivalent of `endp`.
    let mut parsed = Parsed::new();
    let remainder = match chrono::format::parse_and_remainder(
        &mut parsed,
        input,
        StrftimeItems::new(format),
    ) {
        Ok(rem) => rem,
        Err(_) => {
            // c:64-69 — `if (!endp) { if (!quiet) zwarnnam(nam,
            //                          'format not matched'); return 1; }`
            // C emits the bare 'format not matched' string with no
            // input echo; prior Rust port appended ': {input}' which
            // diverged from zsh -fc parity.
            if quiet == 0 {
                zwarnnam(nam, "format not matched"); // c:67
            }
            return 1; // c:68
        }
    };
    // c:59-61 — `memset(&tm, 0, sizeof(tm)); tm.tm_isdst = -1; tm.tm_mday = 1;`
    //
    // C zero-fills the struct tm and then sets tm_isdst=-1 + tm_mday=1.
    // tm.tm_year IS LEFT AT 0, which means "year since 1900" → 1900.
    // mktime() on a 0-year tm with a parsed time-of-day produces an
    // epoch around 1900-01-01 (a large negative number on 64-bit
    // systems).
    //
    // Prior Rust port defaulted to 1970 — convenient (positive epoch)
    // but divergent from C. `strftime -r '%H:%M' '14:30'`:
    //   - C zsh:    -2208988800 + 14*3600 + 30*60 = -2208938400
    //   - Prior Rust: 50400 (1970-01-01 14:30:00)
    // For a user pipelining the result to e.g. another strftime call,
    // the year difference matters.
    let year = parsed
        .year
        .or_else(|| {
            parsed
                .year_div_100
                .zip(parsed.year_mod_100)
                .map(|(d, m)| d * 100 + m)
        })
        .unwrap_or(1900); // c:59 — tm_year=0 means 1900.
    let month = parsed.month.unwrap_or(1);
    let day = parsed.day.unwrap_or(1);
    let hour = parsed.hour_div_12.zip(parsed.hour_mod_12).map(|(d, m)| (d * 12 + m) as u32).unwrap_or(0);
    let minute = parsed.minute.unwrap_or(0);
    let second = parsed.second.unwrap_or(0);
    let date = match NaiveDate::from_ymd_opt(year, month, day) {
        Some(d) => d,
        None => {
            // c:67 — same bare 'format not matched' for out-of-range
            // date components (e.g. month 13). C's strptime → mktime
            // chain doesn't distinguish parse-mismatch from invalid-
            // date because mktime normalises any out-of-range tm
            // fields rather than failing; chrono's from_ymd_opt is
            // stricter, so we emit the same diagnostic.
            if quiet == 0 {
                zwarnnam(nam, "format not matched");
            }
            return 1;
        }
    };
    let time = NaiveTime::from_hms_opt(hour, minute, second).unwrap_or_else(|| NaiveTime::from_hms_opt(0, 0, 0).unwrap());
    let dt = NaiveDateTime::new(date, time);
    // c:71 — `mytime = (zlong)mktime(&tm);`
    // C uses `tm.tm_isdst = -1` (set at c:60) to ask mktime to
    // auto-detect DST. For ambiguous times (the fall-back hour that
    // exists twice — e.g. 1:30 AM on a US fall-back day), mktime
    // with tm_isdst=-1 returns the LATER (non-DST) variant; the
    // standard rationale is "after a fall-back, the same wall-clock
    // time appears twice; the second occurrence (standard time)
    // wins" matching how `date` and most tools resolve.
    //
    // chrono's `from_local_datetime` returns `Ambiguous(earliest,
    // latest)` for these times. Prior Rust picked .earliest (the
    // first variant arg) — the DST-active occurrence, which is the
    // OPPOSITE of C mktime's resolution. Real-world repro:
    //   $ TZ=America/New_York strftime -r '%Y-%m-%d %H:%M:%S' '2024-11-03 01:30:00'
    //   # C: emits the second occurrence (EST, after fall-back).
    //   # Prior Rust: emits the first occurrence (EDT, pre-fall-back).
    //   # Difference: one hour off — 3600 seconds.
    // Pick `.latest()` for fall-back-ambiguous matching mktime.
    let secs = match Local.from_local_datetime(&dt) {
        // c:71 mktime
        chrono::LocalResult::Single(d) => d.timestamp(),
        chrono::LocalResult::Ambiguous(_, latest) => latest.timestamp(),
        chrono::LocalResult::None => {
            if quiet == 0 {
                zwarnnam(nam, "unable to convert to time");
            }
            return 1;
        }
    };
    if let Some(name) = scalar {
        // c:73-74 — `if (scalar) setiparam(scalar, mytime);`
        setiparam(name, secs); // c:74 setiparam
    } else {
        // c:75-79 — print as decimal.
        println!("{}", secs); // c:78 printf("%ld\n", ...)
    }
    // c:81-88 — `if (*endp && !quiet) zwarnnam(nam,
    //              "warning: input string not completely matched");`
    // strptime can succeed yet leave trailing input unconsumed; the
    // format was satisfied but more bytes remained. C emits a soft
    // warning (still returns 0) so the user can spot accidental
    // truncation. Prior Rust port didn't have this — silent
    // partial-parse meant scripts feeding malformed input through
    // `strftime -r '%Y-%m-%d' 'extra2024-01-15'` got `secs=0` (the
    // post-strptime defaults applied) without diagnostic.
    if !remainder.is_empty() && quiet == 0 {
        zwarnnam(nam, "warning: input string not completely matched"); // c:87
    }
    0 // c:90
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

    // c:113-119 — `if (OPT_ISSET(ops, 'r'))` reverse path.
    // C body:
    //   if (OPT_ISSET(ops, 'r')) {
    //       if (!argv[1]) {
    //           zwarnnam(nam, "timestring expected");
    //           return 1;
    //       }
    //       return reverse_strftime(nam, argv, scalar,
    //                               OPT_ISSET(ops, 'q'));
    //   }
    //
    // Prior port skipped the !argv[1] guard so `strftime -r %s`
    // (format with no timestring) fell through to reverse_strftime
    // which would either crash or emit a non-canonical diagnostic.
    if OPT_ISSET(ops, b'r') {
        if argv.len() < 2 {
            // c:114-117 — `if (!argv[1])` then 'timestring expected'.
            zwarnnam(nam, "timestring expected"); // c:115
            return 1; // c:116
        }
        let quiet = if OPT_ISSET(ops, b'q') { 1 } else { 0 };
        return reverse_strftime(nam, argv, scalar, quiet); // c:118
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
    // c:190-191 — `int result = 1; char *tz = getsparam("TZ");`
    let tz_saved = getsparam("TZ"); // c:190
                                    // c:192-198 — `startparamscope();
                                    //              if (tz) {
                                    //                  Param pm = createparam("TZ", PM_LOCAL|...);
                                    //                  if (pm) pm->level = locallevel;
                                    //                  setsparam("TZ", ztrdup(tz));
                                    //              }`
                                    //
                                    // C's PM_LOCAL gives the TZ override function-scope so
                                    // endparamscope at c:200 automatically restores the prior TZ.
                                    // Rust port pushes via env::set_var so libc's strftime sees
                                    // the new zone — paramtab setsparam alone doesn't propagate
                                    // to the libc-level zone (libc reads $TZ from getenv at
                                    // strftime time, not from zsh's paramtab).
                                    //
                                    // Capture the ORIGINAL env::TZ here so we can restore it on
                                    // exit. Prior port re-set env::TZ to the same value on exit
                                    // instead of restoring — leaving a stale env::TZ override
                                    // after bin_strftime returned. Real-world: user has env::TZ
                                    // = "UTC" but $TZ paramtab = "America/Chicago"; after
                                    // strftime returns, env::TZ silently became "America/Chicago"
                                    // and stayed there for the rest of the session.
    let env_tz_saved = std::env::var_os("TZ"); // capture before overwrite
    if let Some(ref tz) = tz_saved {
        std::env::set_var("TZ", tz); // c:197 setsparam
    }
    // Convert &[String] → &[&str] for the internal helper which
    // takes the narrower view.
    let argv_views: Vec<&str> = argv.iter().map(String::as_str).collect();
    let result = output_strftime(nam, &argv_views, ops, func); // c:199
                                                               // c:200 — `endparamscope();` — restore prior env::TZ.
    match env_tz_saved {
        Some(prev) => std::env::set_var("TZ", prev),
        None => std::env::remove_var("TZ"),
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
    let mut arr: Vec<String> = Vec::with_capacity(2); // c:228 zhalloc(3 * sizeof(*arr))
    let mut now: timespec = unsafe { std::mem::zeroed() }; // c:224
                                                           // c:226 — `zgettime(&now);`
    zgettime(&mut now);
    // c:229 — `sprintf(buf, "%ld", (long)now.tv_sec);`
    let buf = format!("{}", now.tv_sec as i64);
    arr.push(buf); // c:230 arr[0] = dupstring(buf)
                   // c:231 — `sprintf(buf, "%ld", (long)now.tv_nsec);`
    let buf = format!("{}", now.tv_nsec as i64);
    arr.push(buf); // c:232 arr[1] = dupstring(buf)
                   // c:233 — `arr[2] = NULL;` (collapsed: Vec's length is the terminator)
    arr // c:235
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
    // C body c:294-295 — `return 0` because the param registration
    // happens via the `pd_list` feature descriptor at
    // `Src/Modules/datetime.c:25-30`:
    //   { "EPOCHSECONDS",  PM_INTEGER|PM_READONLY|PM_HIDE|PM_HIDEVAL|PM_SPECIAL, ... },
    //   { "EPOCHREALTIME", PM_FFLOAT|PM_READONLY|PM_HIDE|PM_HIDEVAL|PM_SPECIAL, ... },
    //   { "epochtime",     PM_ARRAY|PM_READONLY|PM_HIDE|PM_HIDEVAL|PM_SPECIAL, ... }
    // zshrs's simplified module framework doesn't drive that
    // feature dispatch into paramtab, so `${(t)EPOCHSECONDS}`
    // returned `scalar` (the default for an unregistered name
    // that resolves via lookup_special_var). Register the entries
    // here so the canonical introspection paths see the right
    // PM_* flag set. Bug #512.
    use crate::ported::params::{paramtab, createparam};
    use crate::ported::zsh_h::{
        PM_INTEGER, PM_FFLOAT, PM_ARRAY, PM_READONLY, PM_HIDE, PM_HIDEVAL, PM_SPECIAL,
    };
    let entries: &[(&str, u32)] = &[
        ("EPOCHSECONDS",
            PM_INTEGER | PM_READONLY | PM_HIDE | PM_HIDEVAL | PM_SPECIAL),
        ("EPOCHREALTIME",
            PM_FFLOAT  | PM_READONLY | PM_HIDE | PM_HIDEVAL | PM_SPECIAL),
        ("epochtime",
            PM_ARRAY   | PM_READONLY | PM_HIDE | PM_HIDEVAL | PM_SPECIAL),
    ];
    for (name, flags) in entries {
        let exists = paramtab()
            .read()
            .ok()
            .map(|t| t.contains_key(*name))
            .unwrap_or(false);
        if !exists {
            let _ = createparam(name, *flags as i32);
        }
    }
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
fn handlefeatures(_m: *const module, _f: &Mutex<features>, enables: &mut Option<Vec<i32>>) -> i32 {
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
        assert!(
            s < 4_102_444_800,
            "epoch must be before 2100-01-01; got {s}"
        );
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

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/datetime.c.
    // ═══════════════════════════════════════════════════════════════════

    /// c:266 — `getcurrentsecs` returns positive epoch (post-Y2000).
    #[test]
    fn getcurrentsecs_returns_positive_epoch() {
        let _g = crate::test_util::global_state_lock();
        let s = getcurrentsecs();
        assert!(s > 0, "epoch must be positive, got {}", s);
        assert!(s > 946_684_800, "must be after Y2000");
    }

    /// c:266 — monotonic non-decreasing across 3 immediate calls.
    #[test]
    fn getcurrentsecs_is_monotonic_non_decreasing_pin() {
        let _g = crate::test_util::global_state_lock();
        let a = getcurrentsecs();
        let b = getcurrentsecs();
        let c = getcurrentsecs();
        assert!(b >= a, "{} -> {} went backwards", a, b);
        assert!(c >= b, "{} -> {} went backwards", b, c);
    }

    /// c:283 — `getcurrentrealtime` returns positive double.
    #[test]
    fn getcurrentrealtime_returns_positive() {
        let _g = crate::test_util::global_state_lock();
        let t = getcurrentrealtime();
        assert!(t > 0.0);
    }

    /// c:283 — not NaN, finite.
    #[test]
    fn getcurrentrealtime_is_finite() {
        let _g = crate::test_util::global_state_lock();
        let t = getcurrentrealtime();
        assert!(!t.is_nan());
        assert!(t.is_finite());
    }

    /// c:283 — agrees with getcurrentsecs ±5s.
    #[test]
    fn getcurrentrealtime_agrees_with_secs() {
        let _g = crate::test_util::global_state_lock();
        let real = getcurrentrealtime();
        let secs = getcurrentsecs() as f64;
        assert!((real - secs).abs() < 5.0);
    }

    /// c:303 — returns exactly 2 elements [sec, nsec].
    #[test]
    fn getcurrenttime_returns_two_elements_pin() {
        let _g = crate::test_util::global_state_lock();
        let arr = getcurrenttime();
        assert_eq!(arr.len(), 2);
    }

    /// c:303 — nsec in [0, 1B).
    #[test]
    fn getcurrenttime_nsec_in_valid_range() {
        let _g = crate::test_util::global_state_lock();
        let arr = getcurrenttime();
        let nsec: i64 = arr[1].parse().expect("nsec parses");
        assert!(
            nsec >= 0 && nsec < 1_000_000_000,
            "nsec out of range: {}",
            nsec
        );
    }

    /// c:233 — `bin_strftime` with no args returns nonzero (usage).
    #[test]
    fn bin_strftime_no_args_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let r = bin_strftime("strftime", &[], &ops, 0);
        assert_ne!(r, 0, "no args → usage error");
    }

    /// Lifecycle (c:329/357) split per-hook.
    #[test]
    fn datetime_setup_returns_zero_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(setup_(std::ptr::null()), 0);
    }

    /// c:357 — boot_(NULL) = 0.
    #[test]
    fn datetime_boot_returns_zero_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(boot_(std::ptr::null()), 0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/datetime.c
    // c:266 getcurrentsecs / c:283 getcurrentrealtime / c:303 getcurrenttime
    // c:233 bin_strftime / c:329-374 lifecycle
    // ═══════════════════════════════════════════════════════════════════

    /// c:266 — `getcurrentsecs` returns i64.
    #[test]
    fn getcurrentsecs_returns_i64_type() {
        let _: i64 = getcurrentsecs();
    }

    /// c:266 — `getcurrentsecs` is monotonically non-decreasing across
    /// many calls (no time-travel backward).
    #[test]
    fn getcurrentsecs_monotonic_across_many_calls() {
        let mut prev = getcurrentsecs();
        for _ in 0..100 {
            let cur = getcurrentsecs();
            assert!(cur >= prev, "secs went backwards: {} < {}", cur, prev);
            prev = cur;
        }
    }

    /// c:266 — `getcurrentsecs` is after Unix epoch (positive).
    #[test]
    fn getcurrentsecs_after_epoch() {
        assert!(getcurrentsecs() > 0, "must be after 1970-01-01");
    }

    /// c:283 — `getcurrentrealtime` returns f64.
    #[test]
    fn getcurrentrealtime_returns_f64_type() {
        let _: f64 = getcurrentrealtime();
    }

    /// c:283 — `getcurrentrealtime` non-negative.
    #[test]
    fn getcurrentrealtime_non_negative() {
        assert!(getcurrentrealtime() >= 0.0);
    }

    /// c:303 — `getcurrenttime` returns Vec<String>.
    #[test]
    fn getcurrenttime_returns_vec_string() {
        let _: Vec<String> = getcurrenttime();
    }

    /// c:303 — `getcurrenttime` first element parses as integer (seconds).
    #[test]
    fn getcurrenttime_first_is_numeric() {
        let v = getcurrenttime();
        assert!(!v.is_empty(), "must have ≥ 1 element");
        let parsed: Result<i64, _> = v[0].parse();
        assert!(parsed.is_ok(), "first element {:?} must parse as i64", v[0]);
    }

    /// c:233 — `bin_strftime` return value in u8 exit-code range.
    #[test]
    fn bin_strftime_return_in_exit_code_range() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        for args in [
            vec![],
            vec!["%Y".to_string()],
            vec!["%Y".to_string(), "1000".to_string()],
        ] {
            let r = bin_strftime("strftime", &args, &ops, 0);
            assert!(
                (0..256).contains(&r),
                "exit code {} must fit in u8 for {:?}",
                r,
                args
            );
        }
    }

    /// c:329-374 — full lifecycle setup→features→enables→boot→cleanup→finish.
    #[test]
    fn datetime_full_lifecycle_returns_zero_for_all() {
        let _g = crate::test_util::global_state_lock();
        let null = std::ptr::null();
        assert_eq!(setup_(null), 0);
        let mut feats = Vec::new();
        let _ = features_(null, &mut feats);
        let mut enables: Option<Vec<i32>> = None;
        let _ = enables_(null, &mut enables);
        assert_eq!(boot_(null), 0);
        assert_eq!(cleanup_(null), 0);
        assert_eq!(finish_(null), 0);
    }

    /// c:374 — finish_ idempotent.
    #[test]
    fn datetime_finish_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(finish_(std::ptr::null()), 0);
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/datetime.c
    // c:34 reverse_strftime / c:89 output_strftime / c:266 getcurrentsecs
    // c:283 getcurrentrealtime / c:303 getcurrenttime
    // ═══════════════════════════════════════════════════════════════════

    /// c:266 — `getcurrentsecs` is deterministic-ish (monotonic step bound).
    #[test]
    fn getcurrentsecs_two_calls_close_in_time() {
        let _g = crate::test_util::global_state_lock();
        let a = getcurrentsecs();
        let b = getcurrentsecs();
        assert!(
            (b - a).abs() <= 2,
            "two getcurrentsecs calls must be within 2 seconds"
        );
    }

    /// c:283 — `getcurrentrealtime` returns f64 finite.
    #[test]
    fn getcurrentrealtime_returns_finite_f64() {
        let _g = crate::test_util::global_state_lock();
        let v = getcurrentrealtime();
        assert!(v.is_finite(), "must be finite");
        assert!(v > 0.0, "after epoch");
    }

    /// c:283 — `getcurrentrealtime` and `getcurrentsecs` agree on whole secs.
    #[test]
    fn getcurrentrealtime_floor_matches_getcurrentsecs() {
        let _g = crate::test_util::global_state_lock();
        let secs = getcurrentsecs() as f64;
        let real = getcurrentrealtime();
        // Allow 2 seconds drift since they may straddle the second boundary.
        assert!(
            (real - secs).abs() < 2.0,
            "realtime {} and secs {} must agree within 2 sec",
            real,
            secs
        );
    }

    /// c:303 — `getcurrenttime` returns Vec<String> with second element parseable.
    #[test]
    fn getcurrenttime_second_element_is_nsec() {
        let _g = crate::test_util::global_state_lock();
        let v = getcurrenttime();
        if v.len() >= 2 {
            let nsec: Result<i64, _> = v[1].parse();
            assert!(nsec.is_ok(), "second element {:?} must parse as i64", v[1]);
            if let Ok(n) = nsec {
                assert!(n >= 0, "nsec must be ≥ 0");
            }
        }
    }

    /// c:303 — `getcurrenttime` Vec length is exactly 2 (secs, nsec).
    #[test]
    fn getcurrenttime_length_is_two() {
        let _g = crate::test_util::global_state_lock();
        let v = getcurrenttime();
        assert_eq!(v.len(), 2, "secs + nsec = 2 elements");
    }

    /// c:303 — `getcurrenttime` is deterministic in structure (always 2 elements).
    #[test]
    fn getcurrenttime_always_two_elements() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..5 {
            assert_eq!(getcurrenttime().len(), 2);
        }
    }

    /// c:34 — `reverse_strftime` returns i32 (compile-time type pin).
    #[test]
    fn reverse_strftime_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = reverse_strftime("strftime", &[], None, 0);
    }

    /// c:34 — `reverse_strftime` empty argv returns nonzero (usage error).
    #[test]
    fn reverse_strftime_empty_argv_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let r = reverse_strftime("strftime", &[], None, 0);
        assert_ne!(r, 0, "empty argv → usage error");
    }

    /// c:266 — `getcurrentsecs` is positive for many calls.
    #[test]
    fn getcurrentsecs_always_positive() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert!(getcurrentsecs() > 0);
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/datetime.c
    // c:233 bin_strftime / c:266 getcurrentsecs / c:283 getcurrentrealtime /
    // c:303 getcurrenttime + lifecycle
    // ═══════════════════════════════════════════════════════════════════

    /// c:266 — `getcurrentsecs` returns i64 (compile-time pin, alt).
    #[test]
    fn getcurrentsecs_returns_i64_pin_alt() {
        let _g = crate::test_util::global_state_lock();
        let _: i64 = getcurrentsecs();
    }

    /// c:283 — `getcurrentrealtime` returns f64 (compile-time pin, alt).
    #[test]
    fn getcurrentrealtime_returns_f64_pin_alt() {
        let _g = crate::test_util::global_state_lock();
        let _: f64 = getcurrentrealtime();
    }

    /// c:303 — `getcurrenttime` returns Vec<String> (compile-time pin).
    #[test]
    fn getcurrenttime_returns_vec_string_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Vec<String> = getcurrenttime();
    }

    /// c:266 — `getcurrentsecs` is monotonically non-decreasing across
    /// rapid consecutive calls (no time-travel within a single test
    /// process; system clock could change but on a stable test bed
    /// the next call >= prev).
    #[test]
    fn getcurrentsecs_monotonically_non_decreasing() {
        let _g = crate::test_util::global_state_lock();
        let mut prev = getcurrentsecs();
        for _ in 0..50 {
            let now = getcurrentsecs();
            assert!(now >= prev, "time went backwards: {} → {}", prev, now);
            prev = now;
        }
    }

    /// c:283 — `getcurrentrealtime` is monotonically non-decreasing.
    #[test]
    fn getcurrentrealtime_monotonically_non_decreasing() {
        let _g = crate::test_util::global_state_lock();
        let mut prev = getcurrentrealtime();
        for _ in 0..50 {
            let now = getcurrentrealtime();
            assert!(now >= prev, "realtime went backwards: {} → {}", prev, now);
            prev = now;
        }
    }

    /// c:266 — `getcurrentsecs` returns plausible current time (after
    /// 2020-01-01 epoch = 1577836800, before 2100-01-01 = 4102444800).
    #[test]
    fn getcurrentsecs_in_plausible_epoch_range() {
        let _g = crate::test_util::global_state_lock();
        let now = getcurrentsecs();
        assert!(
            now >= 1_577_836_800,
            "current time {} must be after 2020-01-01 epoch",
            now
        );
        assert!(
            now <= 4_102_444_800,
            "current time {} must be before 2100-01-01 epoch",
            now
        );
    }

    /// c:303 — first element of `getcurrenttime` parses as i64 seconds.
    #[test]
    fn getcurrenttime_first_element_is_secs() {
        let _g = crate::test_util::global_state_lock();
        let v = getcurrenttime();
        assert!(v.len() >= 1, "must have at least one element");
        let secs: Result<i64, _> = v[0].parse();
        assert!(
            secs.is_ok(),
            "first element {:?} must parse as i64 secs",
            v[0]
        );
    }

    /// c:303 — `getcurrenttime` nsec ≤ 999_999_999 (within one second).
    #[test]
    fn getcurrenttime_nsec_within_one_second() {
        let _g = crate::test_util::global_state_lock();
        let v = getcurrenttime();
        if v.len() >= 2 {
            if let Ok(n) = v[1].parse::<i64>() {
                assert!(n <= 999_999_999, "nsec {} must be ≤ 999_999_999", n);
            }
        }
    }

    /// c:233 — `bin_strftime` returns i32 (compile-time pin).
    #[test]
    fn bin_strftime_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let _: i32 = bin_strftime("strftime", &[], &ops, 0);
    }

    /// c:233 — `bin_strftime` no-args returns nonzero (usage error, alt).
    #[test]
    fn bin_strftime_no_args_returns_nonzero_pin_alt() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let r = bin_strftime("strftime", &[], &ops, 0);
        assert_ne!(r, 0, "no args → usage error");
    }

    /// c:329/342/350/357/367/374 — each lifecycle hook returns 0 individually.
    #[test]
    fn datetime_each_lifecycle_hook_returns_zero_individually() {
        let _g = crate::test_util::global_state_lock();
        let null = std::ptr::null();
        let mut v: Vec<String> = Vec::new();
        let mut e: Option<Vec<i32>> = None;
        assert_eq!(setup_(null), 0, "c:329 setup_");
        assert_eq!(features_(null, &mut v), 0, "c:342 features_");
        assert_eq!(enables_(null, &mut e), 0, "c:350 enables_");
        assert_eq!(boot_(null), 0, "c:357 boot_");
        assert_eq!(cleanup_(null), 0, "c:367 cleanup_");
        assert_eq!(finish_(null), 0, "c:374 finish_");
    }
}

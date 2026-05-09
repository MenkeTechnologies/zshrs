//! Date/time utilities — port of `Src/Modules/datetime.c`.
//!
//! C source has 0 structs/enums. Rust port matches: 0 types.
//! Functions:
//!   - `getcurrentsecs`     [c:206]
//!   - `getcurrentrealtime` [c:212]
//!   - `getcurrenttime`     [c:220]
//!   - `reverse_strftime`   [c:42]
//!   - `output_strftime`    [c:99]   (the actual builtin entry)
//!   - `bin_strftime`       [c:187]  (TZ-scope wrapper around output_strftime)
//!   - 6 module loaders
//!
//! C uses libc `localtime(3)` + zsh's custom `ztrftime()` (which
//! extends POSIX strftime with the `%.N` nanosecond syntax). The
//! Rust port calls `crate::ported::utils::ztrftime()` for the
//! base format and adds %N extensions on top.

use chrono::{DateTime, Local, NaiveDateTime, TimeZone};
use crate::ported::exec::ShellExecutor;
use crate::ported::utils::zwarnnam;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Port of `getcurrentsecs()` from `Src/Modules/datetime.c:206`.
/// Returns the current epoch seconds — backs `$EPOCHSECONDS`.
/// C body: `return (zlong) time(NULL);`
pub fn getcurrentsecs() -> i64 {                                         // c:206
    // c:208 — `return (zlong) time(NULL);`
    unsafe { libc::time(std::ptr::null_mut()) as i64 }
}

/// Port of `getcurrentrealtime()` from `Src/Modules/datetime.c:212`.
/// Returns the current high-resolution epoch time as f64 — backs
/// `$EPOCHREALTIME`.
///
/// C body:
/// ```c
/// struct timespec now;
/// zgettime(&now);
/// return (double)now.tv_sec + (double)now.tv_nsec * 1e-9;
/// ```
pub fn getcurrentrealtime() -> f64 {                                     // c:212
    let now = crate::compat::zgettime();                                 // c:215
    (now.tv_sec as f64) + (now.tv_nsec as f64) * 1e-9                    // c:216
}

/// Port of `getcurrenttime()` from `Src/Modules/datetime.c:220`.
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
pub fn getcurrenttime() -> (i64, i64) {                                  // c:220
    let now = crate::compat::zgettime();                                 // c:226
    (now.tv_sec, now.tv_nsec)                                            // c:228-231 sprintf %ld
}

/// Port of `reverse_strftime()` from `Src/Modules/datetime.c:42`.
/// Parses a time string per the format string and assigns the
/// resulting epoch seconds to `scalar` (or stdout if NULL).
///
/// C signature: `static int reverse_strftime(char *nam, char **argv,
///                                            char *scalar, int quiet)`.
pub fn reverse_strftime(
    exec: &mut ShellExecutor,
    nam: &str,
    argv: &[&str],
    scalar: Option<&str>,
    quiet: bool,
) -> i32 {                                                                // c:42
    if argv.len() < 2 {                                                   // c:54 timestring expected
        zwarnnam(nam, "timestring expected");
        return 1;
    }
    let format = argv[0];
    let input = argv[1];
    // c:64 — `strptime(timestring, format, &tm)`. Rust uses chrono's
    // NaiveDateTime parser for the same effect.
    let dt = match NaiveDateTime::parse_from_str(input, format) {
        Ok(d) => d,
        Err(_) => {                                                       // c:67-71 mismatch
            if !quiet {
                zwarnnam(nam, &format!("format not matched: {}", input));
            }
            return 1;
        }
    };
    let secs = match Local.from_local_datetime(&dt) {                    // c:78 mktime
        chrono::LocalResult::Single(d) => d.timestamp(),
        chrono::LocalResult::Ambiguous(d, _) => d.timestamp(),
        chrono::LocalResult::None => {
            if !quiet {
                zwarnnam(nam, "unable to convert to time");
            }
            return 1;
        }
    };
    if let Some(name) = scalar {                                          // c:90 scalar
        exec.variables.insert(name.to_string(), secs.to_string());        // c:91 setiparam
    } else {                                                              // c:93
        println!("{}", secs);                                             // c:94 printf("%ld\n", ...)
    }
    0                                                                     // c:96
}

/// Port of `output_strftime()` from `Src/Modules/datetime.c:99`.
/// The `output_strftime` builtin entry. Parses argv (format,
/// timestamp, nanoseconds), calls `localtime(3)` to convert,
/// formats via `ztrftime()` with retry-on-overflow, then writes
/// the result to stdout (or `setsparam` to the `-s NAME` scalar).
///
/// C signature: `static int output_strftime(char *nam, char **argv,
///                                           Options ops, int func)`.
pub fn output_strftime(
    exec: &mut ShellExecutor,
    nam: &str,
    argv: &[&str],
    ops: &[bool; 256],
    scalar: Option<&str>,
) -> i32 {                                                                // c:99
    // c:107 — `if (OPT_ISSET(ops,'s'))`
    if let Some(name) = scalar {
        if !is_ident(name) {                                              // c:110 isident check
            zwarnnam(nam, &format!("not an identifier: {}", name));       // c:111
            return 1;                                                     // c:112
        }
    }

    // c:115 — `if (OPT_ISSET(ops, 'r'))` reverse path.
    if ops[b'r' as usize] {
        return reverse_strftime(exec, nam, argv, scalar, ops[b'q' as usize]);  // c:120
    }

    if argv.is_empty() {
        zwarnnam(nam, "format expected");
        return 1;
    }

    // c:122 — parse argv[1] as timestamp, or use current time.
    let (secs, nsec) = if argv.len() < 2 {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or(Duration::ZERO);
        (now.as_secs() as i64, now.subsec_nanos() as i64)                 // c:124-125 zgettime
    } else {
        // c:128 — `ts.tv_sec = (time_t)strtoul(argv[1], &endptr, 10);`
        let secs = match argv[1].parse::<i64>() {
            Ok(v) => v,
            Err(_) => {
                zwarnnam(nam, &format!("{}: invalid decimal number", argv[1]));
                return 1;                                                 // c:135
            }
        };
        // c:144 — argv[2] nanoseconds (optional).
        let nsec = if argv.len() > 2 {
            match argv[2].parse::<i64>() {
                Ok(v) if (0..=999_999_999).contains(&v) => v,             // c:151
                Ok(_) => {
                    zwarnnam(nam, &format!("{}: invalid nanosecond value", argv[2]));
                    return 1;                                             // c:153
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
        chrono::LocalResult::None => {                                    // c:171-174
            zwarnnam(nam, &format!("bad/unsupported format: '{}'", format));
            return 1;                                                     // c:174
        }
    };
    // First substitute %N variants (zsh extension at utils.c:3411-3429).
    let mut work = String::with_capacity(format.len() * 2);
    let bytes = format.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 1 < bytes.len() {
            match bytes[i + 1] {
                b'N' => { work.push_str(&format!("{:09}", nsec)); i += 2; continue; }
                b'.' if i + 2 < bytes.len() && bytes[i + 2] == b'N' => {
                    work.push_str(&format!(".{:09}", nsec));
                    i += 3; continue;
                }
                d if d.is_ascii_digit() && i + 2 < bytes.len() && bytes[i + 2] == b'N' => {
                    let digits = (d - b'0') as usize;
                    let scaled = if digits >= 9 { nsec }
                                 else { nsec / 10i64.pow((9 - digits) as u32) };
                    work.push_str(&format!("{:0width$}", scaled, width = digits));
                    i += 3; continue;
                }
                b'%' => { work.push_str("%%"); i += 2; continue; }
                _ => {}
            }
        }
        work.push(bytes[i] as char);
        i += 1;
    }
    let formatted = dt.format(&work).to_string();

    // c:178 — `if (scalar) { setsparam(scalar, metafy(buffer, len, META_DUP)); }`
    if let Some(name) = scalar {
        exec.variables.insert(
            name.to_string(),
            crate::ported::utils::metafy(&formatted),
        );
    } else {
        // c:180-183 — fwrite + putchar('\n') unless -n
        print!("{}", formatted);                                          // c:181 fwrite
        if !ops[b'n' as usize] {                                          // c:182 !OPT_ISSET(ops,'n')
            println!();                                                   // c:183 putchar('\n')
        }
    }

    0                                                                     // c:185
}

/// Port of `bin_strftime()` from `Src/Modules/datetime.c:187`. The
/// `strftime` builtin entry — wraps `output_strftime` in a local
/// param-scope that copies `$TZ` so `output_strftime`'s
/// `localtime(3)` calls see the user's timezone even if a function
/// scope has shadowed it.
///
/// C signature: `static int bin_strftime(char *nam, char **argv,
///                                         Options ops, int func)`.
pub fn bin_strftime(
    exec: &mut ShellExecutor,
    nam: &str,
    argv: &[&str],
    ops: &[bool; 256],
    scalar: Option<&str>,
) -> i32 {                                                                // c:187
    // c:191 — `char *tz = getsparam("TZ");`
    let tz_saved = exec.variables.get("TZ").cloned();
    // c:193-198 — `startparamscope(); createparam("TZ", PM_LOCAL); setsparam("TZ", ...);`
    if let Some(ref tz) = tz_saved {
        std::env::set_var("TZ", tz);                                      // c:198 setsparam
    }
    let result = output_strftime(exec, nam, argv, ops, scalar);           // c:199
    // c:200 — `endparamscope();`
    if let Some(ref tz) = tz_saved {
        std::env::set_var("TZ", tz);
    }
    result                                                                // c:202
}

/// Identifier validity check matching zsh's `isident()` (Src/utils.c).
fn is_ident(s: &str) -> bool {
    if s.is_empty() { return false; }
    let mut chars = s.chars();
    let first = chars.next().unwrap();
    if first.is_ascii_digit() { return false; }
    if !(first.is_alphanumeric() || first == '_') { return false; }
    chars.all(|c| c.is_alphanumeric() || c == '_')
}

/// Port of `setup_()` from `Src/Modules/datetime.c:270`. C body is
/// `return 0;` (UNUSED `Module m`).
pub fn setup_() -> i32 {                                                  // c:270
    0                                                                     // c:273
}

/// Port of `features_()` from `Src/Modules/datetime.c:277`. C body
/// is `*features = featuresarray(m, &module_features); return 0;`.
pub fn features_() -> i32 {                                               // c:277
    0                                                                     // c:281
}

/// Port of `enables_()` from `Src/Modules/datetime.c:285`. C body
/// is `return handlefeatures(m, &module_features, enables);`.
pub fn enables_() -> i32 {                                                // c:285
    0                                                                     // c:288
}

/// Port of `boot_()` from `Src/Modules/datetime.c:292`. C body is
/// `return 0;` (UNUSED `Module m`).
pub fn boot_() -> i32 {                                                   // c:292
    0                                                                     // c:295
}

/// Port of `cleanup_()` from `Src/Modules/datetime.c:299`. C body
/// is `return setfeatureenables(m, &module_features, NULL);`.
pub fn cleanup_() -> i32 {                                                // c:299
    0                                                                     // c:302
}

/// Port of `finish_()` from `Src/Modules/datetime.c:306`. C body
/// is `return 0;` (UNUSED `Module m`).
pub fn finish_() -> i32 {                                                 // c:306
    0                                                                     // c:309
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_exec() -> ShellExecutor { ShellExecutor::new() }

    #[test]
    fn test_epoch_seconds() {
        let secs = getcurrentsecs();
        assert!(secs > 1700000000);
    }

    #[test]
    fn test_epoch_realtime() {
        let rt = getcurrentrealtime();
        assert!(rt > 1700000000.0);
        let (secs, _) = getcurrenttime();
        assert!((rt - secs as f64).abs() < 1.0);
    }

    #[test]
    fn test_epoch_time() {
        let (secs, nanos) = getcurrenttime();
        assert!(secs > 1700000000);
        assert!((0..1_000_000_000).contains(&nanos));
    }

    #[test]
    fn test_output_strftime_nanoseconds() {
        let mut e = fresh_exec();
        let mut ops = [false; 256];
        ops[b'n' as usize] = true;
        let r = output_strftime(&mut e, "strftime", &["%9N", "1700000000", "123456789"],
                                 &ops, Some("OUT"));
        assert_eq!(r, 0);
        assert_eq!(e.variables.get("OUT").map(|s| s.as_str()), Some("123456789"));
        let r = output_strftime(&mut e, "strftime", &["%3N", "1700000000", "123456789"],
                                 &ops, Some("OUT"));
        assert_eq!(r, 0);
        assert_eq!(e.variables.get("OUT").map(|s| s.as_str()), Some("123"));
    }

    #[test]
    fn test_output_strftime_to_scalar() {
        let mut e = fresh_exec();
        let mut ops = [false; 256];
        ops[b'n' as usize] = true; // suppress newline
        let r = output_strftime(&mut e, "strftime", &["%s", "1700000000"], &ops, Some("OUT"));
        assert_eq!(r, 0);
        assert_eq!(e.variables.get("OUT").map(|s| s.as_str()), Some("1700000000"));
    }

    #[test]
    fn test_output_strftime_format_required() {
        let mut e = fresh_exec();
        let r = output_strftime(&mut e, "strftime", &[], &[false; 256], None);
        assert_eq!(r, 1);
    }
}

// ========================================================
// Methods moved verbatim from src/ported/exec.rs because their
// C counterpart's source file maps 1:1 to this Rust module.
// ========================================================

impl ShellExecutor {
    /// `strftime` builtin shim — adapts `&[String]` argv to the
    /// canonical `bin_strftime()` above, parsing the option spec
    /// `"nrqs:"` (datetime.c BUILTIN registration).
    pub(crate) fn bin_strftime(&mut self, args: &[String]) -> i32 {
        let mut ops = [false; 256];
        let mut scalar: Option<String> = None;
        let mut positional: Vec<&str> = Vec::new();
        let mut iter = args.iter().peekable();
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "-s" => {
                    if let Some(name) = iter.next() {
                        scalar = Some(name.clone());
                    }
                }
                "-n" => ops[b'n' as usize] = true,
                "-r" => ops[b'r' as usize] = true,
                "-q" => ops[b'q' as usize] = true,
                "--" => {
                    for rest in iter.by_ref() {
                        positional.push(rest.as_str());
                    }
                    break;
                }
                s if s.starts_with('-') && s.len() > 1 => {
                    for ch in s[1..].chars() {
                        match ch {
                            'n' => ops[b'n' as usize] = true,
                            'r' => ops[b'r' as usize] = true,
                            'q' => ops[b'q' as usize] = true,
                            _ => {
                                zwarnnam("strftime", &format!("bad option: -{}", ch));
                                return 1;
                            }
                        }
                    }
                }
                _ => positional.push(arg.as_str()),
            }
        }
        bin_strftime(self, "strftime", &positional, &ops, scalar.as_deref())
    }
}

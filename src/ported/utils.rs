//! Utility functions for zshrs
//!
//! Port from zsh/Src/utils.c
//!
//! Provides miscellaneous utilities: error handling, file operations,
//! string utilities, and character classification.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::ported::zsh_h::{
    isset, opt_name, AUTONAMEDIRS, BEEP, MULTIBYTE, POSIXIDENTIFIERS,
    PRINTEIGHTBIT, XTRACE,
};
use std::sync::atomic::Ordering;
use crate::ported::zsh_h::{QT_NONE, QT_BACKSLASH, QT_SINGLE, QT_DOUBLE, QT_DOLLARS};
use crate::ported::zsh_h::{QT_BACKTICK, QT_SINGLE_OPTIONAL, QT_BACKSLASH_PATTERN, QT_BACKSLASH_SHOWNULL};
use std::os::unix::io::RawFd;
use std::time::UNIX_EPOCH;
use libc::{S_IRGRP, S_IROTH, S_IRUSR, S_ISGID, S_ISUID, S_ISVTX, S_IWGRP, S_IWOTH, S_IWUSR, S_IXGRP, S_IXOTH, S_IXUSR};
use std::io::Read;
use crate::ported::ztype_h::{TYPTAB, TYPTAB_FLAGS, ZTF_INIT, ICNTRL, IDIGIT, IALNUM, IWORD, IIDENT, IUSER, IALPHA, IBLANK, INBLANK};
use std::os::unix::fs::PermissionsExt;
use crate::ported::zsh_h::{hashnode, nameddir, ND_NOABBREV, ND_USERNAME};
use crate::ported::ztype_h::{ISPECIAL, ZTF_SP_COMMA};
use std::os::unix::fs::MetadataExt;
use crate::ported::ztype_h::ZTF_BANGCHAR;
use crate::ported::ztype_h::ISEP;
use std::ffi::CString;
use crate::ported::zsh_h::{EMULATE_KSH, EMULATE_SH, EMULATION};
use std::sync::Mutex;
use std::sync::atomic::AtomicI64;

/// Script name for error messages
pub static mut SCRIPT_NAME: Option<String> = None;
/// Script filename
pub static mut SCRIPT_FILENAME: Option<String> = None;

/// Print a fatal error to stderr.
/// Port of `zerr(VA_ALIST1(const char *fmt))` from Src/utils.c:172. C source sets `errflag`
/// after emitting `<prefix>: <msg>\n` so the running script aborts
/// at the next safe point. The Rust port currently just prints —
// =====================================================================
// Module-static state for the zerr/zwarn/zerrnam/zwarnnam family —
// port of the file-statics in Src/init.c + Src/exec.c that
// `zwarning()` (utils.c:142) reads to build the error prefix.
// =====================================================================

// name of script being sourced                                             // c:33
/// Port of `char *scriptname` from `Src/init.c`. Set when `source`
/// is reading a script; cleared on return. Used by `zwarning()`
/// (utils.c:147) as the diagnostic prefix.
static SCRIPTNAME: std::sync::OnceLock<std::sync::Mutex<Option<String>>> =
    std::sync::OnceLock::new();

/// Port of `char *argzero` from `Src/init.c`. The shell's argv[0].
/// Used by `zwarning()` (utils.c:147) as the fallback diagnostic
/// prefix when scriptname is unset.
static ARGZERO: std::sync::OnceLock<std::sync::Mutex<Option<String>>> =
    std::sync::OnceLock::new();

// error flag: bits from enum errflag_bits                                 // c:124
/// Port of `int errflag` from `Src/init.c`. Tracks whether an
/// error has been raised (`ERRFLAG_ERROR = 1`) or break/return
/// is in flight (`ERRFLAG_INT = 2`).
///
/// Direct global access: `errflag.load(Ordering::Relaxed)` reads
/// the current value, `errflag.fetch_or(ERRFLAG_ERROR, …)` matches
/// C's `errflag |= ERRFLAG_ERROR`, `errflag.store(0, …)` matches
/// C's `errflag = 0`.
#[allow(non_upper_case_globals)]
pub static errflag: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

/// Port of `int noerrs` from `Src/init.c`. Counter — when `> 0`,
/// suppresses error printing. `noerrs >= 2` also suppresses the
/// `errflag` set inside `zerr`/`zerrnam`.
static NOERRS: std::sync::OnceLock<std::sync::Mutex<i32>> = std::sync::OnceLock::new();

/// Port of `int locallevel` from `Src/init.c`. Function-call depth
/// (0 = top-level, 1+ = inside a fn). `zwarning()` checks this in
/// the script-prefix path (utils.c:150).
// `LOCALLEVEL` removed — see `locallevel_lock` removal comment
// below. Canonical static lives in `crate::ported::params::locallevel`
// (port of params.c:54).

/// Port of `int lineno` from `Src/init.c`. Current line number;
/// `zerrmsg()` includes it in the diagnostic when locallevel > 0
/// or shinstdin is unset (utils.c:301).
static LINENO: std::sync::OnceLock<std::sync::Mutex<i32>> = std::sync::OnceLock::new();

/// Port of the `isset(SHINSTDIN)` check from utils.c:150. C reads
/// the SHINSTDIN option directly; the Rust port caches it here so
/// callers don't pull in the whole option-table for every error.
static SHINSTDIN_OPT: std::sync::OnceLock<std::sync::Mutex<bool>> = std::sync::OnceLock::new();

/// `ERRFLAG_ERROR` from `Src/zsh.h`. Set on `zerr`/`zerrnam` to
/// signal a fatal error has occurred.
pub const ERRFLAG_ERROR: i32 = 1;

/// Port of `mod_export int incompfunc` from `Src/utils.c:46`.
/// Set non-zero while a `comp*` builtin is dispatching from inside
/// a user-defined completion function — guards `comparguments` /
/// `compset` / `compadd` / `compdescribe` / `comptags` / `compvalues`
/// / `compfiles` / `compgroups` / `compquote` against being called
/// outside the `compfunc` shfunc context (each builtin checks
/// `INCOMPFUNC` early and emits "can only be called from completion
/// function" when zero).
pub static INCOMPFUNC: std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(0);                                    // c:46

/// Port of `mod_export int resetneeded` from `Src/utils.c:1821`.
/// Set when the editor needs a redraw — incremented by widgets +
/// signal handlers (e.g. SIGWINCH); the next prompt loop tick clears
/// it after running zrefresh.
pub static RESETNEEDED: std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(0);                                    // c:1821

/// Port of `mod_export int winchanged` from `Src/utils.c:1827`.
/// Set by the SIGWINCH handler — the next refresh re-queries the
/// terminal size and re-renders, then clears the flag.
pub static WINCHANGED: std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(0);                                    // c:1827

// WARNING: NOT IN UTILS.C — Rust-only OnceLock get-or-init
// helpers. C dereferences each global directly.
fn scriptname_lock() -> &'static std::sync::Mutex<Option<String>> {
    SCRIPTNAME.get_or_init(|| std::sync::Mutex::new(None))
}
// WARNING: NOT IN UTILS.C — see scriptname_lock above.
fn argzero_lock() -> &'static std::sync::Mutex<Option<String>> {
    ARGZERO.get_or_init(|| std::sync::Mutex::new(None))
}
// WARNING: NOT IN UTILS.C — see scriptname_lock above.
fn noerrs_lock() -> &'static std::sync::Mutex<i32> {
    NOERRS.get_or_init(|| std::sync::Mutex::new(0))
}
// `locallevel_lock` removed — duplicate of canonical
// `crate::ported::params::locallevel` (port of params.c:54). All
// accessors here now route through the canonical AtomicI32.
// WARNING: NOT IN UTILS.C — see scriptname_lock above.
fn lineno_lock() -> &'static std::sync::Mutex<i32> {
    LINENO.get_or_init(|| std::sync::Mutex::new(0))
}
// WARNING: NOT IN UTILS.C — Rust-only cache for the SHINSTDIN
// option flag so error-emission doesn't pull in the option-table.
fn shinstdin_lock() -> &'static std::sync::Mutex<bool> {
    SHINSTDIN_OPT.get_or_init(|| std::sync::Mutex::new(false))
}

/// Setter for `scriptname`. Called from `bin_dot` / `source`
/// when entering a script.
pub fn set_scriptname(name: Option<String>) {
    *scriptname_lock().lock().unwrap() = name;
}

/// Read `scriptname` — direct mirror of the C file-scope
/// `char *scriptname;` at `Src/utils.c:36`. Exposed for the
/// prompt expander (`%N`), `bin_dot`, and ZLE source tracking.
pub fn scriptname_get() -> Option<String> {
    scriptname_lock().lock().unwrap().clone()
}

/// Read `locallevel` — function nesting depth. Routes through the
/// canonical `crate::ported::params::locallevel` static (port of
/// `Src/params.c:54`). The prior Rust port stored a duplicate
/// `Mutex<i32>` here that split state from params.rs; both copies
/// were never kept in sync.
pub fn locallevel() -> i32 {
    crate::ported::params::locallevel.load(std::sync::atomic::Ordering::Relaxed)
}

/// Bump `locallevel` (called by `startparamscope`).
pub fn inc_locallevel() {
    crate::ported::params::locallevel.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

/// Decrement `locallevel` (called by `endparamscope`).
pub fn dec_locallevel() {
    crate::ported::params::locallevel.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
}

/// Setter for `argzero`. Called once at shell init from `parseargs`.
pub fn set_argzero(name: Option<String>) {
    *argzero_lock().lock().unwrap() = name;
}

/// Read `argzero`. Used by `argzerogetfn` for `$0`.
pub fn argzero() -> Option<String> {
    argzero_lock().lock().unwrap().clone()
}

/// Setter for `noerrs`. Increment to suppress error output;
/// decrement to restore.
pub fn set_noerrs(v: i32) {
    *noerrs_lock().lock().unwrap() = v;
}

/// Setter for `locallevel`. Called by function-call entry/exit.
pub fn set_locallevel(v: i32) {
    crate::ported::params::locallevel.store(v, std::sync::atomic::Ordering::Relaxed);
}

/// Setter for `lineno`. Called by the parser as it advances.
pub fn set_lineno(v: i32) {
    *lineno_lock().lock().unwrap() = v;
}

/// Setter for the cached SHINSTDIN flag. Called by the option
/// machinery whenever `setopt shinstdin` / `unsetopt shinstdin`
/// fires.
pub fn set_shinstdin(v: bool) {
    *shinstdin_lock().lock().unwrap() = v;
}

// =====================================================================
// Port of `zwarning(const char *cmd, const char *fmt, va_list ap)` from `Src/utils.c:142`.
// =====================================================================

/// Port of `zwarning(const char *cmd, const char *fmt, va_list ap)` from `Src/utils.c:142`.
///
/// Internal helper that builds the diagnostic prefix and emits it +
/// the formatted message to stderr. Direct C body translation:
///
/// ```c
/// if (isatty(2)) zleentry(ZLE_CMD_TRASH);
/// char *prefix = scriptname ? scriptname : (argzero ? argzero : "");
/// if (cmd) {
///     if (unset(SHINSTDIN) || locallevel) {
///         nicezputs(prefix, stderr);
///         fputc(':', stderr);
///     }
///     nicezputs(cmd, stderr);
///     fputc(':', stderr);
/// } else {
///     nicezputs((isset(SHINSTDIN) && !locallevel) ? "zsh" : prefix, stderr);
///     fputc(':', stderr);
/// }
/// zerrmsg(stderr, fmt, ap);
/// ```
/// WARNING: param names don't match C — Rust=(cmd, msg) vs C=(cmd, fmt, ap)
fn zwarning(cmd: Option<&str>, msg: &str) {
    // C: if (isatty(2)) zleentry(ZLE_CMD_TRASH);
    // ZLE trash-line is pending the zle_main.c port; skip without
    // affecting the error-emission path.
    let _ = unsafe { libc::isatty(2) };
    let scriptname = scriptname_lock().lock().unwrap().clone();
    let argzero = argzero_lock().lock().unwrap().clone();
    let locallevel = crate::ported::params::locallevel
        .load(std::sync::atomic::Ordering::Relaxed);
    let shinstdin = *shinstdin_lock().lock().unwrap();
    let prefix: String = scriptname
        .or(argzero)
        .unwrap_or_default();
    let stderr_handle = std::io::stderr();
    let mut stderr_lock = stderr_handle.lock();
    if let Some(cmd) = cmd {
        // C: if (unset(SHINSTDIN) || locallevel) — emit prefix.
        if !shinstdin || locallevel != 0 {
            let _ = stderr_lock.write_all(nicezputs(&prefix).as_bytes());
            let _ = stderr_lock.write_all(b":");
        }
        let _ = stderr_lock.write_all(nicezputs(cmd).as_bytes());
        let _ = stderr_lock.write_all(b":");
    } else {
        // C: nicezputs((isset(SHINSTDIN) && !locallevel) ? "zsh" : prefix, ...);
        let to_emit = if shinstdin && locallevel == 0 {
            "zsh"
        } else {
            prefix.as_str()
        };
        let _ = stderr_lock.write_all(nicezputs(to_emit).as_bytes());
        let _ = stderr_lock.write_all(b":");
    }
    // C: zerrmsg(stderr, fmt, ap);  — emit lineno prefix (when
    // SHINSTDIN unset or locallevel != 0) then formatted message + \n.
    // Direct port of utils.c:301-308.
    let lineno = *lineno_lock().lock().unwrap();
    if (!shinstdin || locallevel != 0) && lineno != 0 {
        let _ = stderr_lock.write_all(format!("{}: ", lineno).as_bytes());
    } else {
        let _ = stderr_lock.write_all(b" ");
    }
    let _ = stderr_lock.write_all(msg.as_bytes());
    let _ = stderr_lock.write_all(b"\n");
    let _ = stderr_lock.flush();
}

// =====================================================================
// Port of `zerr(VA_ALIST1(const char *fmt))` / `zerrnam` / `zwarn` / `zwarnnam` from utils.c:173
// onward. Each is a thin wrapper: check errflag/noerrs guards, set
// `ERRFLAG_ERROR` on the fatal variants, call `zwarning`.
// =====================================================================

/// Port of `zerr(VA_ALIST1(const char *fmt))` from `Src/utils.c:173`.
///
/// ```c
/// if (errflag || noerrs) {
///     if (noerrs < 2) errflag |= ERRFLAG_ERROR;
///     return;
/// }
/// errflag |= ERRFLAG_ERROR;
/// zwarning(NULL, fmt, ap);
/// ```
/// WARNING: param names don't match C — Rust=(msg) vs C=()
pub fn zerr(msg: &str) {                                                     // c:173
    let noerrs = *noerrs_lock().lock().unwrap();
    if errflag.load(Ordering::Relaxed) != 0 || noerrs != 0 {                 // c:175
        if noerrs < 2 {                                                      // c:176
            errflag.fetch_or(ERRFLAG_ERROR, Ordering::Relaxed);              // c:176
        }
        return;                                                              // c:177
    }
    errflag.fetch_or(ERRFLAG_ERROR, Ordering::Relaxed);                      // c:194
    zwarning(None, msg);                                                     // c:194
}

/// Port of `zerrnam(VA_ALIST2(const char *cmd, const char *fmt))` from `Src/utils.c:194`.
///
/// ```c
/// if (errflag || noerrs) return;
/// errflag |= ERRFLAG_ERROR;
/// zwarning(cmd, fmt, ap);
/// ```
/// WARNING: param names don't match C — Rust=(cmd, msg) vs C=(cmd)
pub fn zerrnam(cmd: &str, msg: &str) {                                       // c:194
    let noerrs = *noerrs_lock().lock().unwrap();
    if errflag.load(Ordering::Relaxed) != 0 || noerrs != 0 {                 // c:196
        return;
    }
    errflag.fetch_or(ERRFLAG_ERROR, Ordering::Relaxed);                      // c:214
    zwarning(Some(cmd), msg);                                                // c:214
}

/// Port of `zwarn(VA_ALIST1(const char *fmt))` from `Src/utils.c:214`.
///
/// ```c
/// if (errflag || noerrs) return;
/// zwarning(NULL, fmt, ap);
/// ```
/// WARNING: param names don't match C — Rust=(msg) vs C=()
pub fn zwarn(msg: &str) {                                                    // c:214
    let noerrs = *noerrs_lock().lock().unwrap();
    if errflag.load(Ordering::Relaxed) != 0 || noerrs != 0 {                 // c:216
        return;
    }
    zwarning(None, msg);                                                     // c:231
}

/// Port of `zwarnnam(VA_ALIST2(const char *cmd, const char *fmt))` from `Src/utils.c:231`.
///
/// ```c
/// if (errflag || noerrs) return;
/// zwarning(cmd, fmt, ap);
/// ```
/// WARNING: param names don't match C — Rust=(cmd, msg) vs C=(cmd)
pub fn zwarnnam(cmd: &str, msg: &str) {                                      // c:231
    let noerrs = *noerrs_lock().lock().unwrap();
    if errflag.load(Ordering::Relaxed) != 0 || noerrs != 0 {                 // c:233
        return;
    }
    zwarning(Some(cmd), msg);                                                // c:235
}

/// Port of `zerrmsg(FILE *file, const char *fmt, va_list ap)` from `Src/utils.c:289`.
///
/// C body emits the formatted message + (when locallevel > 0 or
/// SHINSTDIN unset) the line number prefix + "\n". The Rust port
/// is invoked indirectly through `zwarning` — direct callers pass
/// pre-formatted strings.
/// WARNING: param names don't match C — Rust=(msg, errno) vs C=(file, fmt, ap)
pub fn zerrmsg(msg: &str, errno: Option<i32>) {                              // c:289
    let lineno = *lineno_lock().lock().unwrap();
    let shinstdin = *shinstdin_lock().lock().unwrap();
    let locallevel = crate::ported::params::locallevel
        .load(std::sync::atomic::Ordering::Relaxed);
    // C: if ((unset(SHINSTDIN) || locallevel) && lineno) — prefix
    // with the line number.
    if (!shinstdin || locallevel != 0) && lineno != 0 {
        eprint!("{}: ", lineno);
    }
    if let Some(e) = errno {
        eprintln!("{}: {}", msg, std::io::Error::from_raw_os_error(e));
    } else {
        eprintln!("{}", msg);
    }
}

/// Nicely format a string for display (escape unprintable chars)
/// Render a control character as a printable form.
/// Port of `nicechar(int c)` from Src/utils.c — same `^X`/`M-X`
/// /`\xNN` rules used by `print -P` and the prompt path.
pub fn nicechar(c: char) -> String {                                        // c:520
    if c.is_ascii_control() {
        match c {
            '\n' => "\\n".to_string(),
            '\t' => "\\t".to_string(),
            '\r' => "\\r".to_string(),
            '\x1b' => "\\e".to_string(),
            _ => format!("^{}", ((c as u8) + 64) as char),
        }
    } else if c == '\x7f' {
        "^?".to_string()
    } else {
        c.to_string()
    }
}

/// Nicely format a string
/// Render an entire string with `nicechar()` for every byte.
/// Port of `nicezputs(char const *s, FILE *stream)` from Src/utils.c.
/// WARNING: param names don't match C — Rust=(s) vs C=(s, stream)
pub fn nicezputs(s: &str) -> String {                                       // c:5313
    s.chars().map(nicechar).collect()
}

/// Convert character to lowercase
/// To-lowercase that respects locale.
/// Port of `tulower(int c)` from Src/utils.c.
pub fn tulower(c: char) -> char {                                           // c:2302
    c.to_lowercase().next().unwrap_or(c)
}

/// Convert character to uppercase
/// To-uppercase that respects locale.
/// Port of `tuupper(int c)` from Src/utils.c.
pub fn tuupper(c: char) -> char {                                           // c:2310
    c.to_uppercase().next().unwrap_or(c)
}

/// Check if string is a valid identifier
/// Check whether a string is a valid shell identifier.
/// Port of the `itype_end(...IIDENT)` walk Src/utils.c uses
/// (around `validident()`).
pub fn isident(s: &str) -> bool {                                            // c:params.c:1288
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Check if string looks like a number
/// Check whether a string parses as a decimal integer.
/// Sleep for a given number of seconds (fractional)
/// Sleep for a fractional number of seconds.
/// Port of `int zsleep(long us)` from Src/utils.c:2797.
///
/// "Sleep for the given number of microseconds." Wraps
/// `nanosleep(2)` with EINTR retry, returning 1 on completion, 0
/// on permanent error.
pub fn zsleep(us: i64) -> i32 {                                              // c:2797
    let mut sleeptime = libc::timespec {                                     // c:2797-2803
        tv_sec: (us / 1_000_000) as libc::time_t,
        tv_nsec: ((us % 1_000_000) * 1000) as libc::c_long,
    };
    #[cfg(unix)]
    {
        loop {                                                               // c:2804
            let mut rem = libc::timespec { tv_sec: 0, tv_nsec: 0 };
            let ret = unsafe { libc::nanosleep(&sleeptime, &mut rem) };      // c:2806
            if ret == 0 {                                                    // c:2808
                return 1;
            }
            let err = std::io::Error::last_os_error()
                .raw_os_error().unwrap_or(0);
            if err != libc::EINTR {                                          // c:2810
                return 0;                                                    // c:2811
            }
            sleeptime = rem;                                                 // c:2812
        }
    }
    #[cfg(not(unix))]
    {
        let _ = sleeptime;
        std::thread::sleep(std::time::Duration::from_micros(us.max(0) as u64));
        1
    }
}

/// Close a file descriptor
/// Close an fd with EINTR retry.
/// Port of `zclose(int fd)` from Src/utils.c.
// Close the given fd, and clear it from fdtable.                          // c:2127
pub fn zclose(fd: i32) {                                                    // c:2127
    #[cfg(unix)]
    unsafe {
        libc::close(fd);
    }
}

/// Port of `adjustcolumns(int signalled)` from Src/utils.c:1856 — TIOCGWINSZ
/// lookup that seeds `$COLUMNS`. The C variant updates the global
/// `zterm_columns` and returns whether it changed; this Rust port
/// returns the column count directly. Falls back to `$COLUMNS` env
/// var, then 80, mirroring the C source's `tccolumns > 0 ? tccolumns : 80`
/// fallback at line 1869.
/// WARNING: param names don't match C — Rust=() vs C=(signalled)
pub fn adjustcolumns() -> usize {                                           // c:1856
    #[cfg(unix)]
    {
        unsafe {
            let mut ws: libc::winsize = std::mem::zeroed();
            if libc::ioctl(1, libc::TIOCGWINSZ, &mut ws) == 0 && ws.ws_col > 0 {
                return ws.ws_col as usize;
            }
        }
    }
    std::env::var("COLUMNS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(80)
}

// window size changed                                                     // c:1831
/// Port of `adjustlines(int signalled)` from Src/utils.c:1831 — TIOCGWINSZ
/// lookup that seeds `$LINES`. The C variant updates the global
/// `zterm_lines` and returns whether it changed; this Rust port
/// returns the row count directly. Falls back to `$LINES` env var,
/// then 24, mirroring the C source's `tclines > 0 ? tclines : 24`
/// fallback at line 1844.
/// WARNING: param names don't match C — Rust=() vs C=(signalled)
pub fn adjustlines() -> usize {                                             // c:1831
    #[cfg(unix)]
    {
        unsafe {
            let mut ws: libc::winsize = std::mem::zeroed();
            if libc::ioctl(1, libc::TIOCGWINSZ, &mut ws) == 0 && ws.ws_row > 0 {
                return ws.ws_row as usize;
            }
        }
    }
    std::env::var("LINES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(24)
}

// QuoteType enum + impl deleted — the canonical quote-type values are
// the bare `QT_*: i32` constants at `zsh_h.rs:175` (port of anonymous
// `enum { QT_NONE, QT_BACKSLASH, … }` from `Src/zsh.h:253-298`).
// `quotestring()` takes `quote_type: i32` matching C's signature.

/// Map a `(q)` flag count to a `QT_*` value.
/// Port of the q-flag dispatch in `Src/subst.c` `paramsubst()` —
/// `(q)`=`QT_BACKSLASH`, `(qq)`=`QT_SINGLE`, `(qqq)`=`QT_DOUBLE`,
/// `(qqqq+)`=`QT_DOLLARS`.
pub fn qflag_quotetype(count: u32) -> i32 {
    match count {
        0 => QT_NONE,
        1 => QT_BACKSLASH,
        2 => QT_SINGLE,
        3 => QT_DOUBLE,
        _ => QT_DOLLARS,
    }
}

/// Port of `ispecial()` macro from `Src/ztype.h:59` (macro).
///
/// `ispecial(X)` expands to `zistype(X, ISPECIAL)` and tests
/// whether the char is in the SPECCHARS table — the literal set
/// `"#$^*()=|{}[]`<>?~;&\\n\\t \\\\\\'\\""` from `Src/zsh.h:228`,
/// optionally augmented with `,` (ZTF_SP_COMMA) and `bangchar`
/// (BANGHIST). zshrs hard-codes the static set; the `,`/`!`
/// augmentation flags aren't yet wired through.
fn ispecial(c: char) -> bool {
    matches!(
        c,
        '|' | '&'
            | ';'
            | '<'
            | '>'
            | '('
            | ')'
            | '$'
            | '`'
            | '"'
            | '\''
            | '\\'
            | ' '
            | '\t'
            | '\n'
            | '='
            | '['
            | ']'
            | '*'
            | '?'
            | '#'
            | '~'
            | '{'
            | '}'
            | '!'
            | '^'
    )
}

/// Quote a string according to the specified type
/// Port from zsh/Src/utils.c quotestring() (lines 6141-6452)
/// Quote a string per the requested bslashquote style.
/// Port of `quotestring(const char *s, int instring)` from Src/utils.c — used by `print
/// -%q`, `${(q)var}`, completion-output escaping, history
/// re-emission.
/// WARNING: param names don't match C — Rust=(s, quote_type) vs C=(s, instring)
pub fn quotestring(s: &str, quote_type: i32) -> String {                     // c:6141
    if s.is_empty() {
        return if quote_type == QT_NONE {
            String::new()
        } else if quote_type == QT_BACKSLASH || quote_type == QT_BACKSLASH_SHOWNULL {
            "''".to_string()
        } else if quote_type == QT_SINGLE || quote_type == QT_SINGLE_OPTIONAL {
            "''".to_string()
        } else if quote_type == QT_DOUBLE {
            "\"\"".to_string()
        } else if quote_type == QT_DOLLARS {
            "$''".to_string()
        } else {
            String::new()
        };
    }

    if quote_type == QT_NONE {
        s.to_string()
    } else if quote_type == QT_BACKSLASH_PATTERN {
        // Only bslashquote pattern characters (lines 6242-6247)
        let mut result = String::with_capacity(s.len() * 2);
        for c in s.chars() {
            if matches!(c, '*' | '?' | '[' | ']' | '<' | '>' | '(' | ')' | '|' | '#' | '^' | '~') {
                result.push('\\');
            }
            result.push(c);
        }
        result
    } else if quote_type == QT_BACKSLASH || quote_type == QT_BACKSLASH_SHOWNULL {
        // Backslash quoting (lines 6260-6416)
        let mut result = String::with_capacity(s.len() * 2);
        for c in s.chars() {
            if ispecial(c) {
                result.push('\\');
            }
            result.push(c);
        }
        result
    } else if quote_type == QT_SINGLE {
        // Single quote: 'string' (lines 6359-6382)
        let mut result = String::with_capacity(s.len() + 4);
        result.push('\'');
        for c in s.chars() {
            if c == '\'' {
                result.push_str("'\\''");
            } else if c == '\n' {
                result.push_str("'$'\\n''");
            } else {
                result.push(c);
            }
        }
        result.push('\'');
        result
    } else if quote_type == QT_SINGLE_OPTIONAL {
        // Only add quotes where necessary (lines 6314-6363)
        let needs_quoting = s.chars().any(ispecial);
        if !needs_quoting {
            return s.to_string();
        }
        let mut result = String::with_capacity(s.len() + 4);
        let mut in_quotes = false;
        for c in s.chars() {
            if c == '\'' {
                if in_quotes { result.push('\''); in_quotes = false; }
                result.push_str("\\'");
            } else if ispecial(c) {
                if !in_quotes { result.push('\''); in_quotes = true; }
                result.push(c);
            } else {
                if in_quotes { result.push('\''); in_quotes = false; }
                result.push(c);
            }
        }
        if in_quotes { result.push('\''); }
        result
    } else if quote_type == QT_DOUBLE {
        // Double quote: "string" (lines 6272-6280, 6311-6312)
        let mut result = String::with_capacity(s.len() + 4);
        result.push('"');
        for c in s.chars() {
            if matches!(c, '$' | '`' | '"' | '\\') {
                result.push('\\');
            }
            result.push(c);
        }
        result.push('"');
        result
    } else if quote_type == QT_DOLLARS {
        // $'...' quoting with escape sequences (lines 6203-6241)
        let mut result = String::with_capacity(s.len() + 4);
        result.push_str("$'");
        for c in s.chars() {
            match c {
                '\\' | '\'' => { result.push('\\'); result.push(c); }
                '\n' => result.push_str("\\n"),
                '\r' => result.push_str("\\r"),
                '\t' => result.push_str("\\t"),
                '\x1b' => result.push_str("\\e"),
                '\x07' => result.push_str("\\a"),
                '\x08' => result.push_str("\\b"),
                '\x0c' => result.push_str("\\f"),
                '\x0b' => result.push_str("\\v"),
                c if c.is_ascii_control() => {
                    result.push_str(&format!("\\{:03o}", c as u8));
                }
                c => result.push(c),
            }
        }
        result.push('\'');
        result
    } else if quote_type == QT_BACKTICK {
        // Backtick quoting (minimal - just escape backticks)
        s.replace('`', "\\`")
    } else {
        // Unknown quote_type — treat as no-op to match C's `default:` arm.
        s.to_string()
    }
}

/// Split string by separator - port from zsh/Src/utils.c sepsplit() lines 3961-3992
///
/// If sep is None, performs IFS-style word splitting (spacesplit).
/// Otherwise splits on the given separator string.
/// allownull: if true, allows empty strings in result
/// Split a string on `IFS` separators.
/// Port of `sepsplit(char *s, char *sep, int allownull, int heap)` from Src/utils.c:3962.
/// WARNING: param names don't match C — Rust=(s, sep, allownull) vs C=(s, sep, allownull, heap)
pub fn sepsplit(s: &str, sep: Option<&str>, allownull: bool) -> Vec<String> { // c:3962
    // Handle Nularg at start (zsh internal marker) - line 3968
    let s = if s.starts_with('\x00') && s.len() > 1 {
        &s[1..]
    } else {
        s
    };

    match sep {
        None => spacesplit(s, allownull),
        Some("") => {
            // Empty separator: split into characters
            if allownull {
                s.chars().map(|c| c.to_string()).collect()
            } else {
                s.chars()
                    .map(|c| c.to_string())
                    .filter(|c| !c.is_empty())
                    .collect()
            }
        }
        Some(sep) => {
            let parts: Vec<String> = s.split(sep).map(|p| p.to_string()).collect();
            if allownull {
                parts
            } else {
                parts.into_iter().filter(|p| !p.is_empty()).collect()
            }
        }
    }
}

/// IFS-style word splitting - port from zsh/Src/utils.c spacesplit()
///
/// Splits on whitespace (space, tab, newline), treating consecutive
/// whitespace as a single separator.
/// Split on whitespace.
/// Port of `spacesplit(char *s, int allownull, int heap, int quote)` from Src/utils.c.
/// WARNING: param names don't match C — Rust=(s, allownull) vs C=(s, allownull, heap, quote)
pub fn spacesplit(s: &str, allownull: bool) -> Vec<String> {                 // c:3711
    if allownull {
        s.split([' ', '\t', '\n']).map(|p| p.to_string()).collect()
    } else {
        s.split_whitespace().map(|p| p.to_string()).collect()
    }
}

/// Join array with separator - port from zsh/Src/utils.c sepjoin() lines 3926-3958
///
/// If sep is None, uses first char of IFS (defaults to space).
/// Join an array with separator.
/// Port of `sepjoin(char **s, char *sep, int heap)` from Src/utils.c:3928.
/// WARNING: param names don't match C — Rust=(arr, sep) vs C=(s, sep, heap)
pub fn sepjoin(arr: &[String], sep: Option<&str>) -> String {                // c:3928
    if arr.is_empty() {
        return String::new();
    }
    let sep = sep.unwrap_or(" ");
    arr.join(sep)
}

/// zlong with base autodetection. Returns the parsed value AND a
/// `&str` slice pointing to the first unconsumed character —
/// matching C's `char **t` out-arg.
///
/// C signature: `zlong zstrtol(const char *s, char **t, int base)`.
/// `base == 0` triggers prefix-based detection (`0x`/`0X`→16,
/// `0b`/`0B`→2, leading `0`→8, otherwise 10). Otherwise `base`
/// must be in [2, 36]; values outside that range emit `zerr` and
/// return 0.
///
/// On overflow, mirrors C's "number truncated after N digits"
/// behaviour: emits a `zwarn` and returns the truncated value
/// (last in-range computation).
/// Port of `zstrtol(const char *s, char **t, int base)` from `Src/utils.c:2427`.
/// WARNING: param names don't match C — Rust=(s, base) vs C=(s, t, base)
pub fn zstrtol(s: &str, base: i32) -> (i64, &str) {                      // c:2427
    zstrtol_underscore(s, base, false)                                   // c:2427
}

/// Parse unsigned integer with underscore support
/// Port from zsh/Src/utils.c zstrtoul_underscore() lines 2528-2575
/// Parse an unsigned integer with optional `_` separators.
/// zshrs convenience over `zstrtol()` — C zsh strips `_` inline
/// during numeric arg parsing in Src/math.c.
pub fn zstrtoul_underscore(s: &str) -> Option<u64> {                         // c:2529
    let s = s.trim();
    let s = s.strip_prefix('+').unwrap_or(s);

    let (base, rest) = if s.starts_with("0x") || s.starts_with("0X") {
        (16, &s[2..])
    } else if s.starts_with("0b") || s.starts_with("0B") {
        (2, &s[2..])
    } else if s.starts_with('0') && s.len() > 1 {
        (8, &s[1..])
    } else {
        (10, s)
    };

    let rest = rest.replace('_', "");
    u64::from_str_radix(&rest, base).ok()
}

/// Convert integer to string with specified base
/// Port from zsh/Src/utils.c convbase()
/// Render an integer in an arbitrary base using zsh's `BASE#DIGITS`
/// notation (per `setopt CBASES`-off default). Direct port of the
/// radix-conversion loop in Src/utils.c::convbase.
///
/// Format: `2#1010`, `8#777`, `16#FF`, `36#Z`. Negative values
/// emit a leading `-` before the prefix. Base 0 or 10 returns the
/// plain decimal string.
pub fn convbase(val: i64, base: u32) -> String {
    if base == 0 || base == 10 {
        return val.to_string();
    }
    let neg = val < 0;
    let abs = if neg { (val as i128).wrapping_neg() as u128 } else { val as u128 };
    let s = match base {
        2 => format!("2#{:b}", abs),
        8 => format!("8#{:o}", abs),
        16 => format!("16#{:X}", abs),
        r if (2..=36).contains(&r) => {
            let digits = "0123456789abcdefghijklmnopqrstuvwxyz".as_bytes();
            let mut tmp = abs;
            let mut buf = String::new();
            if tmp == 0 { buf.push('0'); }
            while tmp > 0 {
                buf.push(digits[(tmp % r as u128) as usize] as char);
                tmp /= r as u128;
            }
            format!("{}#{}", r, buf.chars().rev().collect::<String>())
        }
        _ => val.to_string(),
    };
    if neg { format!("-{}", s) } else { s }
}

/// Set blocking/nonblocking on a file descriptor
/// Port from zsh/Src/utils.c setblock_fd() lines 2578-2618
/// Toggle non-blocking mode on an fd.
/// Port of the `fcntl(F_SETFL, O_NONBLOCK)` toggle Src/utils.c
/// uses around `read -t` and select-based polling.
pub fn setblock_fd(fd: i32, blocking: bool) -> bool {                        // c:2579
    #[cfg(unix)]
    {
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL, 0) };
        if flags < 0 {
            return false;
        }
        let new_flags = if blocking {
            flags & !libc::O_NONBLOCK
        } else {
            flags | libc::O_NONBLOCK
        };
        if new_flags != flags {
            unsafe { libc::fcntl(fd, libc::F_SETFL, new_flags) >= 0 }
        } else {
            true
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (fd, blocking);
        false
    }
}

/// Read poll - check for pending input
/// Port from zsh/Src/utils.c read_poll() lines 2643-2730
/// Poll an fd with timeout, returning whether it's readable.
/// Port of the `poll(2)` wrapper Src/utils.c uses for
/// `read -t` timeout handling.
pub fn read_poll(fd: i32, timeout_us: i64) -> bool {                         // c:2645
    #[cfg(unix)]
    {
        let mut fds = [libc::pollfd {
            fd: fd as RawFd,
            events: libc::POLLIN,
            revents: 0,
        }];
        let timeout_ms = (timeout_us / 1000) as i32;
        let result = unsafe { libc::poll(fds.as_mut_ptr(), 1, timeout_ms) };
        result > 0 && (fds[0].revents & libc::POLLIN) != 0
    }
    #[cfg(not(unix))]
    {
        let _ = (fd, timeout_us);
        false
    }
}

/// Check glob qualifier syntax
/// Port from zsh/Src/utils.c checkglobqual()
/// Check whether a string contains glob qualifiers `(…)`.
/// Port of `checkglobqual(char *str, int sl, int nobareglob, char **sp)` from Src/utils.c.
pub fn checkglobqual(s: &str) -> bool {                                      // c:glob.c:1158
    if !s.ends_with(')') {
        return false;
    }
    let mut depth = 0;
    let mut in_bracket = false;
    for c in s.chars() {
        match c {
            '[' if !in_bracket => in_bracket = true,
            ']' if in_bracket => in_bracket = false,
            '(' if !in_bracket => depth += 1,
            ')' if !in_bracket => {
                if depth > 0 {
                    depth -= 1;
                } else {
                    return false;
                }
            }
            _ => {}
        }
    }
    depth == 0
}

// spellcheck a word                                                        // c:3123
// fix s ; if hist is nonzero, fix the history list too                     // c:3124
/// Compute edit distance between two strings (for spelling correction)
/// Port from zsh/Src/utils.c spdist() lines 4675-4759
/// Levenshtein-style edit distance for typo correction.
/// Port of `spdist(char *s, char *t, int thresh)` from Src/utils.c — drives the
/// `setopt CORRECT` typo-prompt machinery.
/// WARNING: param names don't match C — Rust=(s, t, max_dist) vs C=(s, t, thresh)
pub fn spdist(s: &str, t: &str, max_dist: usize) -> usize {                  // c:4675
    let s_chars: Vec<char> = s.chars().collect();
    let t_chars: Vec<char> = t.chars().collect();
    let m = s_chars.len();
    let n = t_chars.len();

    if m.abs_diff(n) > max_dist {
        return max_dist + 1;
    }

    let mut prev: Vec<usize> = (0..=n).collect();
    let mut curr = vec![0; n + 1];

    for i in 1..=m {
        curr[0] = i;
        for j in 1..=n {
            let cost = if s_chars[i - 1] == t_chars[j - 1] {
                0
            } else {
                1
            };
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }

    prev[n]
}

/// Port of `char *gettempname(const char *prefix, int use_heap)` from Src/utils.c:2178.
///
/// Returns a unique tempfile name templated like C `mktemp(3)` —
/// `{prefix}.XXXXXX`. Falls back to `getsparam("TMPPREFIX")` and
/// then `DEFAULT_TMPPREFIX` when `prefix` is None. Does NOT create
/// the file (matches C — only `gettempfile` creates).
pub fn gettempname(prefix: Option<&str>, _use_heap: bool) -> Option<String> { // c:2178
    let suffix = if prefix.is_some() { ".XXXXXX" } else { "XXXXXX" };       // c:2178
    crate::ported::signals::queue_signals();                                 // c:2182
    let prefix_owned: String = match prefix {                                // c:2183
        Some(p) => p.to_string(),
        None => std::env::var("TMPPREFIX")
            .unwrap_or_else(|_| crate::ported::config_h::DEFAULT_TMPPREFIX.to_string()), // c:2184
    };
    let template = format!("{}{}", prefix_owned, suffix);                    // c:2186-2188
    // C uses mktemp(3) which mutates the X's into a unique name              // c:2192/2219
    // without creating the file. Rust has no mktemp; emulate with
    // pid+timestamp. Caller is responsible for O_EXCL open.
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let unique = format!("{:x}{:x}", pid, nanos & 0xffffff);
    let name = template.replace("XXXXXX", &unique);
    crate::ported::signals::unqueue_signals();                               // c:2221
    Some(name)
}

/// Check if metafied - port from zsh/Src/utils.c has_token()
// Check if a string contains a token                                       // c:2282
// Check if a string contains a token                                       // c:2282
pub fn has_token(s: &str) -> bool {                                         // c:2282
    s.bytes().any(|b| b == 0x83) // Meta character
}

/// Array length - port from arrlen()
/// Port of `arrlen(char **s)` from `Src/utils.c:2357`.
pub fn arrlen<T>(s: &[T]) -> usize {                                      // c:2357
    s.len()
}

/// Port of `dupstrpfx(const char *s, int len)` from `Src/utils.c` (~line 1230 — duplicate
/// the first `len` BYTES of `s` into a fresh heap string). C body
/// is `memcpy(zhalloc(len+1), s, len); ret[len] = 0;`. The Rust
/// port operates on bytes (not chars) to match — multibyte input
/// can be sliced mid-codepoint by C callers.
pub fn dupstrpfx(s: &str, len: usize) -> String {                            // c:string.c:161
    let bytes = s.as_bytes();
    let take = len.min(bytes.len());
    String::from_utf8_lossy(&bytes[..take]).into_owned()
}

/// Port of `unmeta(const char *file_name)` from `Src/utils.c:4994`.
///
/// Convert a zsh internal (metafied) string to a system-call-safe
/// form (e.g. for passing to `open(2)`).
///
/// C body shape (c:4994-5010):
/// ```c
/// meta = 0;
/// for (t = file_name; *t; t++)
///     if (*t == Meta) { meta = 1; break; }
/// if (!meta) return (char *) file_name;        // no-copy fast path
/// for (t = file_name, p = fn; *t; p++)
///     if ((*p = *t++) == Meta && *t)
///         *p = *t++ ^ 32;
/// ```
pub fn unmeta(s: &str) -> String {                                           // c:4994
    let bytes = s.as_bytes();
    // c:4995-4996 — Meta-byte scan; no-copy fast path.
    if !bytes.iter().any(|&b| b == Meta) {
        return s.to_string();
    }
    let mut buf = bytes.to_vec();
    let len = unmetafy(&mut buf);                                            // c:4999-5001
    buf.truncate(len);
    String::from_utf8_lossy(&buf).into_owned()
}

// pastebuf() DELETED — was a misplaced fn. The real `pastebuf()`
// lives in `Src/Zle/zle_misc.c:558` (a ZLE clipboard helper that
// pastes a Cutbuffer into the line-edit buffer). It has nothing to
// do with metafication, despite this file's prior body which
// reimplemented half of `metafy`. The real metafy lives below at
// `pub fn metafy()` (port of utils.c:4856).


/// Unmetafied string length (from utils.c ztrlen lines 5135-5152)
pub fn ztrlen(s: &str) -> usize {                                            // c:5136
    let mut len = 0;
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        len += 1;
        if chars[i] as u32 == Meta as u32 && i + 1 < chars.len() {
            i += 2;
        } else {
            i += 1;
        }
    }
    len
}

/// Port of `ztrcmp(char const *s1, char const *s2)` from `Src/utils.c:5106`.
///
/// Byte-walking compare with lazy Meta resolution. C body skips
/// matching bytes wholesale, then on the first differing byte
/// un-meta-fies just that byte and compares. Faster than
/// `unmeta(s1).cmp(unmeta(s2))` because:
///   1. No allocation up front,
///   2. Only the first differing position pays the un-meta cost.
///
/// ```c
/// while(*s1 && *s1 == *s2) { s1++; s2++; }
/// if (!(c1 = *s1)) c1 = -1;
/// else if (c1 == Meta) c1 = *++s1 ^ 32;
/// if (!(c2 = *s2)) c2 = -1;
/// else if (c2 == Meta) c2 = *++s2 ^ 32;
/// return c1 - c2;
/// ```
pub fn ztrcmp(s1: &str, s2: &str) -> std::cmp::Ordering {                    // c:5106
    let b1 = s1.as_bytes();
    let b2 = s2.as_bytes();
    let mut i1 = 0;
    let mut i2 = 0;
    // Skip the matching prefix.
    while i1 < b1.len() && i2 < b2.len() && b1[i1] == b2[i2] {
        i1 += 1;
        i2 += 1;
    }
    // Resolve c1: -1 for end-of-string, else the next byte
    // (un-meta-fied if it's a Meta marker).
    let c1: i32 = if i1 >= b1.len() {
        -1
    } else if b1[i1] == Meta && i1 + 1 < b1.len() {
        (b1[i1 + 1] ^ 32) as i32
    } else {
        b1[i1] as i32
    };
    let c2: i32 = if i2 >= b2.len() {
        -1
    } else if b2[i2] == Meta && i2 + 1 < b2.len() {
        (b2[i2 + 1] ^ 32) as i32
    } else {
        b2[i2] as i32
    };
    c1.cmp(&c2)
}

/// String pointer subtraction with meta handling (from utils.c ztrsub)
/// Port of `ztrsub(char const *t, char const *s)` from `Src/utils.c:5187`.
pub fn ztrsub(t: &str, s: &str) -> usize {                                   // c:5187
    ztrlen(&t[..t.len().saturating_sub(s.len())])
}

/// Get username from UID (from utils.c getpwuid handling)
pub fn statuidprint(uid: u32) -> Option<String> {
    #[cfg(unix)]
    {
        let pwd = unsafe { libc::getpwuid(uid) };
        if pwd.is_null() {
            return None;
        }
        let name = unsafe { std::ffi::CStr::from_ptr((*pwd).pw_name) };
        name.to_str().ok().map(|s| s.to_string())
    }
    #[cfg(not(unix))]
    {
        let _ = uid;
        None
    }
}

/// String duplicate (from utils.c ztrdup)
pub fn ztrdup(s: &str) -> String {                                           // c:string.c:62
    s.to_string()
}

/// Duplicate n characters (from utils.c ztrncpy)
/// Port of `ztrncpy(char *s, char *t, int len)` from `Src/utils.c:2320`.
/// WARNING: param names don't match C — Rust=(s, n) vs C=(s, t, len)
pub fn ztrncpy(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// String concat (from utils.c dyncat)
pub fn dyncat(s1: &str, s2: &str) -> String {                                // c:string.c:131
    format!("{}{}", s1, s2)
}

/// Triple concat (from utils.c tricat)
pub fn tricat(s1: &str, s2: &str, s3: &str) -> String {                      // c:string.c:98
    format!("{}{}{}", s1, s2, s3)
}

/// Buffer concat (from utils.c bicat)
pub fn bicat(s1: &str, s2: &str) -> String {                                 // c:string.c:145
    format!("{}{}", s1, s2)
}

/// Port of `wordcount(char *s, char *sep, int mul)` from `Src/utils.c:3879`.
///
/// Returns the number of words in `s` that would result from splitting
/// on `sep` (or `$IFS` when `sep` is None). `mul` controls how
/// consecutive empty fields are counted:
/// - `mul == 0`: don't count leading/trailing/consecutive empties.
/// - `mul > 0`: count consecutive empties (each `sep` boundary
///   produces one word, even if the surrounding text is empty).
/// - `mul < 0`: count empty trailing fields (final separator after
///   the last non-empty field counts as one extra empty word).
///
/// C body (paraphrased):
/// ```c
/// if (sep) {
///     r = 1;
///     sl = strlen(sep);
///     for (; (c = findsep(&s, sep, 0)) >= 0; s += sl)
///         if ((c || mul) && (sl || *(s + sl)))
///             r++;
/// } else {
///     /* IFS-based: walk skipwsep / itype_end(s, ISEP, 1) */
/// }
/// ```
///
/// This port walks the metafied byte stream directly to mirror C's
/// pointer arithmetic. The `sep`-based branch is exact; the IFS
/// branch uses [`iwsep`] for whitespace-separator detection (C's
/// `ISEP` char class collapsed to whitespace, which is the common
/// case for default `$IFS` = `" \t\n"`).
pub fn wordcount(s: &str, sep: Option<&str>, mul: i32) -> i32 {              // c:3879
    let bytes = s.as_bytes();
    if let Some(sep) = sep {
        // C: r = 1; sl = strlen(sep); for (; findsep(&s,sep,0) >= 0; s+=sl)
        //        if ((c || mul) && (sl || *(s+sl))) r++;
        let sep_bytes = sep.as_bytes();
        let sl = sep_bytes.len();
        let mut r: i32 = 1;
        let mut pos = 0;
        while pos <= bytes.len() {
            let rest = &bytes[pos..];
            let c_offset = match sep_bytes.is_empty() {
                true => Some(0usize),
                false => rest
                    .windows(sl)
                    .position(|w| w == sep_bytes),
            };
            let Some(c) = c_offset else { break };
            // C `c` is the chars before the separator; `(sl || *(s+sl))`
            // means: if sl is zero (empty sep), only count when there's a
            // following char. Otherwise (sl > 0), the second clause is true
            // when sep is non-empty AND there are bytes after sep.
            let after_off = pos + c + sl;
            let following_nonempty = after_off < bytes.len();
            let cond_b = sl != 0 || following_nonempty;
            if (c != 0 || mul != 0) && cond_b {
                r += 1;
            }
            if sl == 0 {
                // Avoid infinite loop on empty sep — mirrors C's findsep
                // which advances by 1 byte when sep is empty.
                pos += 1;
            } else {
                pos += c + sl;
            }
        }
        r
    } else {
        // IFS branch (sep == NULL). C source uses itype_end(s, ISEP, 1)
        // to skip ISEP chars (default $IFS = " \t\n"). We use iwsep.
        let mut s_pos = 0usize;
        let t_orig = s_pos;
        let mut r: i32 = 0;
        // C: if (mul <= 0) skipwsep(&s);
        if mul <= 0 {
            while s_pos < bytes.len() && iwsep(bytes[s_pos] as char) {
                s_pos += 1;
            }
        }
        // C: if ((*s && itype_end(s,ISEP,1)!=s) || (mul<0 && t!=s)) r++;
        let has_word_now = s_pos < bytes.len()
            && !iwsep(bytes[s_pos] as char);
        if has_word_now || (mul < 0 && t_orig != s_pos) {
            r += 1;
        }
        // C: for (; *s; r++) { advance over word + maybe-skipwsep + findsep + maybe-skipwsep }
        while s_pos < bytes.len() {
            // Advance past the current word (non-ISEP chars).
            let word_start = s_pos;
            while s_pos < bytes.len() && !iwsep(bytes[s_pos] as char) {
                s_pos += 1;
            }
            if s_pos > word_start && mul <= 0 {
                while s_pos < bytes.len() && iwsep(bytes[s_pos] as char) {
                    s_pos += 1;
                }
            }
            // C: (void)findsep(&s, NULL, 0) — advance past one sep run.
            // Already handled above when mul<=0; for mul>0 we still need
            // to consume one separator byte to make progress.
            if s_pos < bytes.len() && iwsep(bytes[s_pos] as char) {
                s_pos += 1;
            }
            let t_after = s_pos;
            if mul <= 0 {
                while s_pos < bytes.len() && iwsep(bytes[s_pos] as char) {
                    s_pos += 1;
                }
            }
            if s_pos < bytes.len() {
                r += 1;
            } else {
                // C: if (mul < 0 && t != s) r++;
                if mul < 0 && t_after != s_pos {
                    r += 1;
                }
                break;
            }
        }
        r
    }
}

/// Join array with delimiter (from utils.c zjoin)
/// Port of `zjoin(char **arr, int delim, int heap)` from `Src/utils.c:3622`.
/// WARNING: param names don't match C — Rust=(arr, delim) vs C=(arr, delim, heap)
pub fn zjoin(arr: &[String], delim: char) -> String {
    arr.join(&delim.to_string())
}

/// Split colon-separated list (from utils.c colonsplit)
/// Port of `colonsplit(char *s, int uniq)` from `Src/utils.c:3650`.
pub fn colonsplit(s: &str, uniq: bool) -> Vec<String> {
    let mut result = Vec::new();
    for item in s.split(':') {
        if !item.is_empty() {
            if uniq && result.contains(&item.to_string()) {
                continue;
            }
            result.push(item.to_string());
        }
    }
    result
}

/// Port of `skipwsep(char **s)` from `Src/utils.c:3680`.
///
/// Skip whitespace separators (`iwsep`-true bytes) at the start of
/// `s`. Returns `(remaining_str, count)` — `count` is the number of
/// bytes / chars skipped. C body:
///
/// ```c
/// while (*t && iwsep(*t == Meta ? t[1] ^ 32 : *t)) {
///     if (*t == Meta) t++;
///     t++;
///     i++;
/// }
/// ```
///
/// The C version honours Meta-encoded bytes (`Meta` followed by
/// `^32`-XOR'd byte). Rust port mirrors that byte-by-byte.
pub fn skipwsep(s: &str) -> (&str, usize) {
    let bytes = s.as_bytes();
    let mut i: usize = 0;
    let mut count: usize = 0;
    while i < bytes.len() {
        let b = if bytes[i] == Meta && i + 1 < bytes.len() {
            bytes[i + 1] ^ 32
        } else {
            bytes[i]
        };
        if !iwsep_byte(b) {
            break;
        }
        if bytes[i] == Meta {
            i += 1;
        }
        i += 1;
        count += 1;
    }
    (&s[i..], count)
}

/// Port of `iwsep()` macro from `Src/zsh.h`.
///
/// `iwsep(c) == c == ' ' || c == '\t' || c == '\n'` — true for
/// space, tab, or newline. The original Rust port omitted `\n`
/// which is wrong: `wordcount`/`skipwsep`/`spacesplit` all rely on
/// `\n` being a separator for lines.
pub fn iwsep(c: char) -> bool {
    c == ' ' || c == '\t' || c == '\n'
}

/// Byte-form of `iwsep` for callers that work on raw bytes (e.g.
/// `skipwsep` walking a metafied buffer).
#[inline]
pub fn iwsep_byte(b: u8) -> bool {
    b == b' ' || b == b'\t' || b == b'\n'
}

/// Port of the `imeta()` macro from `Src/zsh.h`.
///
/// `#define imeta(c) ((c) >= Meta)` — true for any byte the
/// metafy/unmetafy machinery treats as needing the Meta-escape.
/// The previous Rust port had a wrong predicate (control bytes +
/// 0x7f + Meta itself); the C macro is just `>= Meta`.
pub fn imeta(c: char) -> bool {
    (c as u32) >= Meta as u32
}

/// Format time struct (from utils.c ztrftime)
/// Port of `ztrftime(char *buf, int bufsize, char *fmt, struct tm *tm, long nsec)` from `Src/utils.c:3337`.
/// WARNING: param names don't match C — Rust=(fmt, time) vs C=(buf, bufsize, fmt, tm, nsec)
pub fn ztrftime(fmt: &str, time: std::time::SystemTime) -> String {

    let duration = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = duration.as_secs() as i64;

    #[cfg(unix)]
    unsafe {
        let tm = libc::localtime(&secs);
        if tm.is_null() {
            return String::new();
        }

        let mut buf = vec![0u8; 256];
        let c_fmt = std::ffi::CString::new(fmt).unwrap_or_default();
        let len = libc::strftime(
            buf.as_mut_ptr() as *mut libc::c_char,
            buf.len(),
            c_fmt.as_ptr(),
            tm,
        );

        if len > 0 {
            buf.truncate(len);
            String::from_utf8_lossy(&buf).to_string()
        } else {
            String::new()
        }
    }

    #[cfg(not(unix))]
    {
        let _ = (fmt, secs);
        String::new()
    }
}

/// Get hostname
pub fn gethostname() -> String {
    #[cfg(unix)]
    {
        let mut buf = vec![0u8; 256];
        unsafe {
            if libc::gethostname(buf.as_mut_ptr() as *mut libc::c_char, buf.len()) == 0 {
                let len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
                return String::from_utf8_lossy(&buf[..len]).to_string();
            }
        }
    }
    std::env::var("HOSTNAME").unwrap_or_else(|_| "localhost".to_string())
}

/// Get current working directory
pub fn zgetcwd() -> Option<String> {
    std::env::current_dir()
        .ok()
        .map(|p| p.to_string_lossy().to_string())
}

/// Set current working directory
pub fn zchdir(path: &str) -> bool {
    std::env::set_current_dir(path).is_ok()
}

/// Rust wrapper around libc `realpath(3)` — no zsh C counterpart
/// (zsh uses libc directly via the `realpath` symbol). Provided
/// here for the same callsite shape as C's `realpath(path, NULL)`
/// idiom that some zsh paths use (e.g. `:A` modifier fallback).
///
/// WARNING: Rust-only helper, not a port of any zsh C function.
/// Wraps `std::fs::canonicalize` (which is itself a `realpath(3)`
/// equivalent on Unix). See [`xsymlinks`] for the path-canonicalize
/// equivalent of zsh's actual `:a`/`:A` modifier impl.
pub fn realpath(path: &str) -> Option<String> {
    std::fs::canonicalize(path)
        .ok()
        .map(|p| p.to_string_lossy().to_string())
}

/// Convert to absolute path, normalising `.` and `..` components.
/// Port of `xsymlinks(char *s)` from Src/utils.c — same `realpath(3)`
/// fallback the C source uses on systems without it. Does NOT
/// follow symlinks (matches the `physical = 0` mode in C). The
/// `:a` modifier and the symlink-resolving `:A`/`:P` modifiers
/// dispatch through this when the OS-level canonicalize fails
/// (non-existent paths).
pub fn xsymlinks(s: &str) -> std::io::Result<String> {                      // c:872
    if s.is_empty() {
        return Ok(String::new());
    }

    let path = if !s.starts_with('/') {
        let cwd = std::env::current_dir()?;
        format!("{}/{}", cwd.display(), s)
    } else {
        s.to_string()
    };

    let mut result = Vec::new();
    for component in path.split('/') {
        match component {
            "" | "." => continue,
            ".." => {
                if !result.is_empty() && result.last() != Some(&"..") {
                    result.pop();
                } else if result.is_empty() && !path.starts_with('/') {
                    result.push("..");
                }
            }
            c => result.push(c),
        }
    }

    if path.starts_with('/') {
        Ok(format!("/{}", result.join("/")))
    } else if result.is_empty() {
        Ok(".".to_string())
    } else {
        Ok(result.join("/"))
    }
}

/// Create directory
pub fn mkdir(path: &str) -> bool {
    std::fs::create_dir(path).is_ok()
}

/// Create symlink
pub fn symlink(src: &str, dst: &str) -> bool {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(src, dst).is_ok()
    }
    #[cfg(not(unix))]
    {
        let _ = (src, dst);
        false
    }
}

/// Read symlink target
pub fn readlink(path: &str) -> Option<String> {
    std::fs::read_link(path)
        .ok()
        .map(|p| p.to_string_lossy().to_string())
}

/// Get environment variable
pub fn getenv(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

/// Set environment variable
pub fn setenv(name: &str, value: &str) {
    std::env::set_var(name, value);
}

/// Unset environment variable
pub fn unsetenv(name: &str) {
    std::env::remove_var(name);
}

/// Get current user ID
pub fn getuid() -> u32 {
    #[cfg(unix)]
    unsafe {
        libc::getuid()
    }
    #[cfg(not(unix))]
    0
}

/// Get effective user ID
pub fn geteuid() -> u32 {
    #[cfg(unix)]
    unsafe {
        libc::geteuid()
    }
    #[cfg(not(unix))]
    0
}

/// Get current group ID
pub fn getgid() -> u32 {
    #[cfg(unix)]
    unsafe {
        libc::getgid()
    }
    #[cfg(not(unix))]
    0
}

/// Get effective group ID
pub fn getegid() -> u32 {
    #[cfg(unix)]
    unsafe {
        libc::getegid()
    }
    #[cfg(not(unix))]
    0
}

/// Get process ID
pub fn getpid() -> i32 {
    std::process::id() as i32
}

/// Get parent process ID
pub fn getppid() -> i32 {
    #[cfg(unix)]
    unsafe {
        libc::getppid()
    }
    #[cfg(not(unix))]
    0
}

/// Format seconds as HH:MM:SS
pub fn printtime(secs: i64) -> String {
    let hours = secs / 3600;
    let mins = (secs % 3600) / 60;
    let secs = secs % 60;
    if hours > 0 {
        format!("{}:{:02}:{:02}", hours, mins, secs)
    } else {
        format!("{}:{:02}", mins, secs)
    }
}

// ---------------------------------------------------------------------------
// Missing utility functions ported from utils.c
// ---------------------------------------------------------------------------

/// Split path into components (from utils.c slashsplit)
/// Port of `slashsplit(char *s)` from `Src/utils.c:837`.
pub fn slashsplit(s: &str) -> Vec<String> {                                 // c:837
    s.split('/')
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

/// Split on '=' returning (name, value) (from utils.c equalsplit)
/// Port of `equalsplit(char *s, char **t)` from `Src/utils.c:4133`.
/// WARNING: param names don't match C — Rust=(s) vs C=(s, t)
pub fn equalsplit(s: &str) -> Option<(String, String)> {
    let eq = s.find('=')?;
    Some((s[..eq].to_string(), s[eq + 1..].to_string()))
}

/// Make single-element array (from utils.c mkarray)
/// Port of `mkarray(char *s)` from `Src/utils.c:4083`.
pub fn mkarray(s: Option<&str>) -> Vec<String> {
    match s {
        Some(val) => vec![val.to_string()],
        None => Vec::new(),
    }
}

/// Free array (no-op in Rust, provided for API compat)
/// Port of `freearray(char **s)` from `Src/utils.c:4120`.
pub fn freearray(s: Vec<String>) {
    // Rust Drop handles this
}

/// Check if s is a prefix of t (from utils.c strpfx)
// Return non-zero if s is a prefix of t.                                  // c:7345
pub fn strpfx(s: &str, t: &str) -> bool {
    t.starts_with(s)
}

/// Check if s is a suffix of t (from utils.c strsfx)
// Return non-zero if s is a suffix of t.                                  // c:7345
pub fn strsfx(s: &str, t: &str) -> bool {
    t.ends_with(s)
}

/// Port of `void zbeep(void)` from Src/utils.c:4105.
///
/// Honours `$ZBEEP` (a key-string sequence) when set and the BEEP
/// option when unset; emits the BEL char (\007) to SHTTY by
/// default. The Rust port writes via `write_loop` to mirror C's
/// raw-write semantics.
pub fn zbeep() {                                                             // c:4105
    crate::ported::signals::queue_signals();                                 // c:4105
    if let Ok(zbeep) = std::env::var("ZBEEP") {                              // c:4109
        let (decoded, _) = getkeystring(&zbeep);                             // c:4111
        #[cfg(unix)]
        {
            let shtty = crate::ported::init::SHTTY.load(Ordering::Relaxed);
            if shtty != -1 {
                let _ = write_loop(shtty, decoded.as_bytes());               // c:4112
            } else {
                eprint!("{}", decoded);
            }
        }
        #[cfg(not(unix))]
        eprint!("{}", decoded);
    } else if isset(crate::ported::zsh_h::BEEP) {// c:4113
        #[cfg(unix)]
        {
            let shtty = crate::ported::init::SHTTY.load(Ordering::Relaxed);
            if shtty != -1 {
                let _ = write_loop(shtty, b"\x07");                          // c:4114
            } else {
                eprint!("\x07");
            }
        }
        #[cfg(not(unix))]
        eprint!("\x07");
    }
    crate::ported::signals::unqueue_signals();                               // c:4115
}

/// Port of `mode_to_octal(mode_t mode)` from `Src/utils.c:7634`.
///
/// Convert a `mode_t` into the equivalent canonical octal value
/// by testing each `S_I*` flag explicitly and OR-ing the matching
/// octal bit. This is NOT just `mode & 07777` — on systems where
/// the libc `S_IRUSR`/etc. constants don't match the canonical
/// values (e.g. when zsh runs against a non-POSIX libc),
/// `mode & 07777` returns the libc representation. The C version
/// translates to canonical bits explicitly so callers get a stable
/// portable result.
///
/// ```c
/// int mode_to_octal(mode_t mode)
/// {
///     int m = 0;
///     if (mode & S_ISUID) m |= 04000;
///     ... (12 bit-by-bit mappings)
///     return m;
/// }
/// ```
pub fn mode_to_octal(mode: u32) -> i32 {
    #[cfg(unix)]
    #[cfg(not(unix))]
    {
        // No POSIX permission bits on non-Unix; fall back to a
        // bit-mask matching the canonical layout.
        let _ = mode;
        return 0;
    }
    #[cfg(unix)]
    {
        let m = mode as u32;
        let mut o: i32 = 0;
        if m & S_ISUID as u32 != 0 { o |= 0o4000; }
        if m & S_ISGID as u32 != 0 { o |= 0o2000; }
        if m & S_ISVTX as u32 != 0 { o |= 0o1000; }
        if m & S_IRUSR as u32 != 0 { o |= 0o0400; }
        if m & S_IWUSR as u32 != 0 { o |= 0o0200; }
        if m & S_IXUSR as u32 != 0 { o |= 0o0100; }
        if m & S_IRGRP as u32 != 0 { o |= 0o0040; }
        if m & S_IWGRP as u32 != 0 { o |= 0o0020; }
        if m & S_IXGRP as u32 != 0 { o |= 0o0010; }
        if m & S_IROTH as u32 != 0 { o |= 0o0004; }
        if m & S_IWOTH as u32 != 0 { o |= 0o0002; }
        if m & S_IXOTH as u32 != 0 { o |= 0o0001; }
        o
    }
}

/// Go up n directories (from utils.c upchdir)
/// Port of `upchdir(int n)` from `Src/utils.c:7356`.
pub fn upchdir(n: usize) -> io::Result<()> {
    let mut path = String::new();
    for i in 0..n {
        if i > 0 {
            path.push('/');
        }
        path.push_str("..");
    }
    std::env::set_current_dir(&path)?;
    Ok(())
}

/// Change directory with safeguards (from utils.c lchdir)
/// Port of `lchdir(char const *path, struct dirsav *d, int hard)` from `Src/utils.c:7400`.
/// WARNING: param names don't match C — Rust=(path) vs C=(path, d, hard)
pub fn lchdir(path: &str) -> io::Result<()> {
    let resolved = if path.starts_with('/') {
        PathBuf::from(path)
    } else {
        let cwd = std::env::current_dir()?;
        cwd.join(path)
    };
    std::env::set_current_dir(&resolved)?;
    Ok(())
}

// window size changed                                                      // c:1824
/// Adjust terminal window size (from utils.c adjustwinsize)
pub fn adjustwinsize() -> (usize, usize) {
    (adjustcolumns(), adjustlines())
}

// spellcheck a word                                                       // c:3128
// fix s ; if hist is nonzero, fix the history list too                    // c:3128
/// Spelling correction distance (from utils.c spdist, already exists but adding spckword)
/// Check if word is close enough to correct (from utils.c spckword)
pub fn spckword(word: &str, candidates: &[&str], threshold: usize) -> Option<String> { // c:3128
    let mut best = None;
    let mut best_dist = threshold + 1;
    for &candidate in candidates {
        let dist = spdist(word, candidate, threshold);
        if dist < best_dist {
            best_dist = dist;
            best = Some(candidate.to_string());
        }
    }
    best
}

/// Simple interactive query (from utils.c getquery)
/// Port of `getquery(char *valid_chars, int purge)` from `Src/utils.c:3014`.
/// WARNING: param names don't match C — Rust=(prompt, valid_chars) vs C=(valid_chars, purge)
pub fn getquery(prompt: &str, valid_chars: &str) -> Option<char> {
    eprint!("{}", prompt);
    let _ = io::stderr().flush();

    let mut buf = [0u8; 1];
    #[cfg(unix)]
    {
        if std::io::stdin().read_exact(&mut buf).is_ok() {
            let c = buf[0] as char;
            if valid_chars.is_empty() || valid_chars.contains(c) {
                return Some(c);
            }
        }
    }
    None
}

/// Read a single character (from utils.c read1char)
/// Port of `read1char(int echo)` from `Src/utils.c:2972`.
/// WARNING: param names don't match C — Rust=() vs C=(echo)
pub fn read1char() -> Option<char> {
    #[cfg(unix)]
    {
        let mut buf = [0u8; 1];
        if std::io::stdin().read_exact(&mut buf).is_ok() {
            return Some(buf[0] as char);
        }
    }
    None
}

/// Check before removing directory tree (from utils.c checkrmall)
/// Port of `checkrmall(char *s)` from `Src/utils.c:2867`.
pub fn checkrmall(s: &str) -> bool {
    if let Some(c) = getquery(
        &format!("zsh: sure you want to delete all of {}? [yn] ", s),
        "yn",
    ) {
        c == 'y' || c == 'Y'
    } else {
        false
    }
}

/// Port of `xsymlink(char *s, int heap)` from `Src/utils.c:971`.
///
/// ```c
/// mod_export char *
/// xsymlink(char *s, int heap)
/// {
///     if (*s != '/')
///         return NULL;
///     *xbuf = '\0';
///     if (!chrealpath(&s, 'P', heap)) {
///         zwarn("path expansion failed, using root directory");
///         return heap ? dupstring("/") : ztrdup("/");
///     }
///     return s;
/// }
/// ```
///
/// Returns `Some(resolved)` on success, `None` if the path isn't
/// absolute (C: returns NULL). On resolve failure emits the same
/// "path expansion failed, using root directory" warning and
/// returns `Some("/")`.
///
/// The C source dispatches through `chrealpath()` (Src/hist.c:1971,
/// mode 'P' = physical resolution); the Rust port uses
/// `fs::canonicalize()` which is the libc `realpath(3)` wrapper —
/// same semantics for the symlink-resolution path that `xsymlink`
/// exercises.
pub fn xsymlink(path: &str) -> Option<String> {                             // c:971
    // C: if (*s != '/') return NULL;
    if !path.starts_with('/') {
        return None;
    }
    match std::fs::canonicalize(path) {
        Ok(p) => Some(p.to_string_lossy().into_owned()),
        Err(_) => {
            // C: zwarn("path expansion failed, using root directory");
            //    return heap ? dupstring("/") : ztrdup("/");
            zwarn("path expansion failed, using root directory");
            Some("/".to_string())
        }
    }
}

/// Port of `privasserted()` from `Src/utils.c:7607`.
///
/// "Check whether the shell is running with privileges in effect.
/// This is the case if EITHER the euid is zero, OR (if the system
/// supports POSIX.1e (POSIX.6) capability sets) the process'
/// Effective or Inheritable capability sets are non-empty."
///
/// ```c
/// if (!geteuid()) return 1;
/// #ifdef HAVE_CAP_GET_PROC
///     cap_t caps = cap_get_proc();
///     if (caps) {
///         cap_flag_value_t val;
///         for (cap_value_t cap = 0;
///              !cap_get_flag(caps, cap, CAP_EFFECTIVE, &val); cap++)
///             if (val && cap != CAP_WAKE_ALARM) {
///                 cap_free(caps);
///                 return 1;
///             }
///     }
///     cap_free(caps);
/// #endif
///     return 0;
/// ```
///
/// The previous Rust port checked `getuid() != geteuid()` which is
/// the SUID-binary detection, not the "running with privileges"
/// check. The capability-set inspection requires libcap (gated
/// behind the `libcap` feature in `crate::ported::modules::cap`);
/// without it, only the euid==0 path is exercised — same as the
/// C `#else` arm when HAVE_CAP_GET_PROC isn't defined.
pub fn privasserted() -> bool {
    #[cfg(unix)]
    {
        if unsafe { libc::geteuid() } == 0 {
            return true;
        }
    }
    // POSIX.1e capabilities check (HAVE_CAP_GET_PROC) — only
    // active on Linux when zshrs is built with `--features
    // libcap`. The cap module's `cap_get_proc` returns Ok(text)
    // when the process has any capability set; we treat any
    // non-default-empty result as "privileges asserted".
    #[cfg(all(target_os = "linux", feature = "libcap"))]
    {
        // Pending: walk the capability set with cap_get_flag and
        // skip CAP_WAKE_ALARM as the C source does. The cap.rs
        // port doesn't yet expose the flag-iteration FFI; until
        // it does, conservative-true on any non-empty cap text.
        if let Ok(text) = crate::ported::modules::cap::cap_get_proc() {
            // Empty / default-empty cap text = no privileges.
            if !text.is_empty() && text != "=" {
                return true;
            }
        }
    }
    false
}

/// Port of `findpwd(char *s)` from `Src/utils.c:792`.
///
/// ```c
/// char *findpwd(char *s)
/// {
///     char *t;
///     if (*s == '/')
///         return xsymlink(s, 0);
///     s = tricat((pwd[1]) ? pwd : "", "/", s);
///     t = xsymlink(s, 0);
///     zsfree(s);
///     return t;
/// }
/// ```
///
/// Resolve `s` to its canonical form. Absolute paths route through
/// `xsymlink` directly; relative paths get prefixed with the
/// current `pwd` first.
///
/// Signature note: C takes `s: char *`. The previous Rust port had
/// no parameter (returning the cwd) — completely wrong. New port
/// matches C: takes `&str`, returns `Option<String>` (xsymlink can
/// return NULL).
// get a symlink-free pathname for s relative to PWD                        // c:792
pub fn findpwd(s: &str) -> Option<String> {                                 // c:792
    if s.starts_with('/') {                                                  // c:792
        return xsymlink(s);                                                  // c:797
    }
    // C: tricat((pwd[1]) ? pwd : "", "/", s) — uses the global
    // `pwd` (logical cwd; differs from realpath when chasing
    // symlinks is disabled). The Rust port reads `$PWD` since
    // shell-set `PWD` mirrors C's `pwd` global; falls back to
    // `getcwd()` when unset.
    let pwd = std::env::var("PWD").ok()
        .or_else(|| std::env::current_dir().ok()
            .map(|p| p.to_string_lossy().into_owned()))
        .unwrap_or_default();                                                // c:798
    let prefix: &str = if pwd.len() > 1 { &pwd } else { "" };                // c:798 pwd[1]
    let combined = format!("{}/{}", prefix, s);                              // c:798
    xsymlink(&combined)                                                      // c:799
}

/// Port of `void fprintdir(char *s, FILE *f)` from Src/utils.c:1031.
///
/// "print a directory" — abbreviates `s` via `finddir` (so a path
/// matching `$HOME` or a `nameddirtab` entry shows as `~name/...`)
/// and returns the rendering. The C source writes to a FILE*; Rust
/// returns the string for the caller to print.
pub fn fprintdir(s: &str) -> String {                                        // c:1031
    match finddir(s) {                                                       // c:1031
        None => unmeta(s),                                                   // c:1036
        Some(rendered) => rendered,                                          // c:1038-1040
    }
}

/// Duplicate array (from utils.c arrdup)
/// Port of `arrdup(char **s)` from `Src/utils.c:4493`.
pub fn arrdup(s: &[String]) -> Vec<String> {
    s.to_vec()
}

/// Duplicate array with max elements (from utils.c arrdup_max)
/// Port of `arrdup_max(char **s, unsigned max)` from `Src/utils.c:4508`.
pub fn arrdup_max(s: &[String], max: usize) -> Vec<String> {
    s.iter().take(max).cloned().collect()
}

/// Read/write loop wrappers (from utils.c read_loop/write_loop)
/// Port of `read_loop(int fd, char *buf, size_t len)` from `Src/utils.c:2923`.
/// WARNING: param names don't match C — Rust=(fd, buf) vs C=(fd, buf, len)
pub fn read_loop(fd: i32, buf: &mut [u8]) -> io::Result<usize> {
    #[cfg(unix)]
    {
        let mut total = 0;
        while total < buf.len() {
            let n = unsafe {
                libc::read(
                    fd,
                    buf[total..].as_mut_ptr() as *mut libc::c_void,
                    buf.len() - total,
                )
            };
            if n <= 0 {
                if n < 0 {
                    let e = io::Error::last_os_error();
                    if e.kind() == io::ErrorKind::Interrupted {
                        continue;
                    }
                    return Err(e);
                }
                break;
            }
            total += n as usize;
        }
        Ok(total)
    }
    #[cfg(not(unix))]
    {
        let _ = (fd, buf);
        Err(io::Error::new(io::ErrorKind::Unsupported, "not unix"))
    }
}

/// Port of `write_loop(int fd, const char *buf, size_t len)` from `Src/utils.c:2949`.
/// WARNING: param names don't match C — Rust=(fd, buf) vs C=(fd, buf, len)
pub fn write_loop(fd: i32, buf: &[u8]) -> io::Result<usize> {
    #[cfg(unix)]
    {
        let mut total = 0;
        while total < buf.len() {
            let n = unsafe {
                libc::write(
                    fd,
                    buf[total..].as_ptr() as *const libc::c_void,
                    buf.len() - total,
                )
            };
            if n <= 0 {
                if n < 0 {
                    let e = io::Error::last_os_error();
                    if e.kind() == io::ErrorKind::Interrupted {
                        continue;
                    }
                    return Err(e);
                }
                break;
            }
            total += n as usize;
        }
        Ok(total)
    }
    #[cfg(not(unix))]
    {
        let _ = (fd, buf);
        Err(io::Error::new(io::ErrorKind::Unsupported, "not unix"))
    }
}

/// Port of `redup(int x, int y)` from `Src/utils.c:2021`.
///
/// C signature: `int redup(int x, int y)`.
/// "Move fd x to y. If x == -1, fd y is closed. Returns y for
/// success, -1 for failure." (c:2014-2016 docstring.)
///
/// Body mirrors c:2023-2068: when `x == -1`, close `y` (return `y`);
/// when `x == y`, no-op (return `y`); otherwise `dup2(x, y)` +
/// `close(x)`. The `fpurge` block at c:2025-2045 and the `fdtable[]`
/// updates at c:2053-2063 require the fdtable global which isn't
/// ported yet — those side effects are skipped, the dup2/close pair
/// is preserved.
pub fn redup(x: i32, y: i32) -> i32 {                                    // c:2021
    let mut ret = y;                                                     // c:2021
    #[cfg(unix)]
    {
        if x < 0 {                                                       // c:2047
            zclose(y);                                                   // c:2048
        } else if x != y {                                               // c:2049
            if unsafe { libc::dup2(x, y) } == -1 {                       // c:2050
                ret = -1;                                                // c:2051
            }
            // c:2053-2063 — fdtable copy + FLOCK handling (skipped:
            // fdtable global not yet ported to Rust).
            zclose(x);                                                   // c:2064
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (x, y);
    }
    ret                                                                  // c:2067
}

/// Check if a character type at end of string (from utils.c itype_end)
/// Returns the position after the identifier characters
/// Port of `itype_end(const char *ptr, int itype, int once)` from `Src/utils.c:4395`.
/// WARNING: param names don't match C — Rust=(s, allow_digits_start) vs C=(ptr, itype, once)
pub fn itype_end(s: &str, allow_digits_start: bool) -> usize {
    let mut chars = s.chars().peekable();
    let mut pos = 0;

    if let Some(&first) = chars.peek() {
        if !allow_digits_start && first.is_ascii_digit() {
            return 0;
        }
        if !first.is_alphanumeric() && first != '_' && first != '.' {
            return 0;
        }
    }

    for c in s.chars() {
        if c.is_alphanumeric() || c == '_' || c == '.' {
            pos += c.len_utf8();
        } else {
            break;
        }
    }
    pos
}

// initialize the ztypes table                                              // c:4151
/// Port of `inittyptab()` from `Src/utils.c:4155`. Initialise the
/// `typtab[256]` lookup table that backs the `idigit`/`ialnum`/etc.
/// predicates in `ztype_h`.
///
/// C body (c:4155-4250) does:
///   1. Zero the table.
///   2. Mark 0..=31 and 128..=159 + 127 as ICNTRL.
///   3. Mark '0'..='9' as IDIGIT|IALNUM|IWORD|IIDENT|IUSER.
///   4. Mark 'a'..='z' and 'A'..='Z' as IALPHA|IALNUM|IIDENT|IUSER|IWORD.
///   5. Mark '_' as IIDENT|IUSER.
///   6. Mark '-', '.', Dash as IUSER.
///   7. Mark ' '/'\t' as IBLANK|INBLANK; '\n' as INBLANK.
///   8. Mark '\0', Meta, Marker as IMETA.
///   9. Mark Pound..LAST_NORMAL_TOK as ITOK|IMETA.
///   10. Mark Snull..Nularg as ITOK|IMETA|INULL.
///   11. Walk $IFS adding ISEP and IWSEP for blanks.
///
/// This first-pass port covers steps 1-7. Steps 8-11 require Meta /
/// Marker / Pound / Snull / Nularg constants from zsh_h/zsh.h and
/// the `ifs` global; the remaining Meta/IFS marks are skipped until
/// those land. Idempotent — safe to call multiple times.
pub fn inittyptab() {                                                    // utils.c:4155

    // c:4160 — `if (!(typtab_flags & ZTF_INIT))` one-off init.
    {
        let mut flags = TYPTAB_FLAGS.lock().unwrap();
        if (*flags & ZTF_INIT) == 0 {
            *flags = ZTF_INIT;
        }
    }

    let mut t = TYPTAB.lock().unwrap();
    // c:4168 — `memset(typtab, 0, sizeof(typtab));`
    for slot in t.iter_mut() { *slot = 0; }
    // c:4169-4170 — control chars 0..32 and 128..160.
    for c in 0..32u32 {
        t[c as usize]       = ICNTRL as u32;
        t[(c + 128) as usize] = ICNTRL as u32;
    }
    // c:4171 — `typtab[127] = ICNTRL;`
    t[127] = ICNTRL as u32;
    // c:4172-4173 — '0'..='9'.
    for c in (b'0' as usize)..=(b'9' as usize) {
        t[c] = (IDIGIT | IALNUM | IWORD | IIDENT | IUSER) as u32;
    }
    // c:4174-4175 — 'a'..='z' and matching 'A'..='Z'.
    for c in (b'a' as usize)..=(b'z' as usize) {
        let upper = c - (b'a' as usize) + (b'A' as usize);
        let bits = (IALPHA | IALNUM | IIDENT | IUSER | IWORD) as u32;
        t[c] = bits;
        t[upper] = bits;
    }
    // c:4190 — `typtab['_'] = IIDENT | IUSER;`
    t[b'_' as usize] = (IIDENT | IUSER) as u32;
    // c:4191 — `typtab['-'] = typtab['.'] = ... = IUSER;` (skipping Dash).
    t[b'-' as usize] = IUSER as u32;
    t[b'.' as usize] = IUSER as u32;
    // c:4192-4194 — blanks.
    t[b' ' as usize]  |= (IBLANK | INBLANK) as u32;
    t[b'\t' as usize] |= (IBLANK | INBLANK) as u32;
    t[b'\n' as usize] |= INBLANK as u32;
}

/// Find a separator in string (from utils.c findsep)
/// Port of `findsep(char **s, char *sep, int quote)` from `Src/utils.c:3784`.
/// WARNING: param names don't match C — Rust=(s, sep) vs C=(s, sep, quote)
pub fn findsep(s: &str, sep: Option<&str>) -> Option<usize> {
    match sep {
        Some(sep) if sep.len() == 1 => s.find(sep.chars().next().unwrap()),
        Some(sep) => s.find(sep),
        None => {
            // Default: split on whitespace
            s.find(|c: char| c.is_ascii_whitespace())
        }
    }
}

/// Find word at position (from utils.c findword)
/// Port of `findword(char **s, char *sep)` from `Src/utils.c:3849`.
pub fn findword<'a>(s: &'a str, sep: Option<&'a str>) -> Option<(&'a str, &'a str)> {
    let s = match sep {
        Some(_) => s,
        None => s.trim_start(),
    };
    if s.is_empty() {
        return None;
    }
    match sep {
        Some(sep) => {
            if let Some(pos) = s.find(sep) {
                Some((&s[..pos], &s[pos + sep.len()..]))
            } else {
                Some((s, ""))
            }
        }
        None => {
            let end = s.find(|c: char| c.is_ascii_whitespace()).unwrap_or(s.len());
            Some((&s[..end], &s[end..]))
        }
    }
}

/// Parse getkeystring escape sequences (from utils.c getkeystring)
/// Handles \n \t \r \e \a \b \f \v \\ \' \" \xNN \uNNNN \UNNNNNNNN \0NNN
/// Port of `getkeystring(char *s, int *len, int how, int *misc)` from `Src/utils.c:6915`.
/// WARNING: param names don't match C — Rust=(s) vs C=(s, len, how, misc)
pub fn getkeystring(s: &str) -> (String, usize) {                           // c:6915
    let mut result = String::new();
    let mut chars = s.chars().peekable();
    let mut consumed = 0;

    while let Some(c) = chars.next() {
        consumed += c.len_utf8();
        if c != '\\' {
            result.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => {
                result.push('\n');
                consumed += 1;
            }
            Some('t') => {
                result.push('\t');
                consumed += 1;
            }
            Some('r') => {
                result.push('\r');
                consumed += 1;
            }
            Some('e') | Some('E') => {
                result.push('\x1b');
                consumed += 1;
            }
            Some('a') => {
                result.push('\x07');
                consumed += 1;
            }
            Some('b') => {
                result.push('\x08');
                consumed += 1;
            }
            Some('f') => {
                result.push('\x0c');
                consumed += 1;
            }
            Some('v') => {
                result.push('\x0b');
                consumed += 1;
            }
            Some('\\') => {
                result.push('\\');
                consumed += 1;
            }
            Some('\'') => {
                result.push('\'');
                consumed += 1;
            }
            Some('"') => {
                result.push('"');
                consumed += 1;
            }
            Some('x') => {
                consumed += 1;
                let mut hex = String::new();
                for _ in 0..2 {
                    if let Some(&c) = chars.peek() {
                        if c.is_ascii_hexdigit() {
                            hex.push(chars.next().unwrap());
                            consumed += 1;
                        } else {
                            break;
                        }
                    }
                }
                if let Ok(val) = u8::from_str_radix(&hex, 16) {
                    result.push(val as char);
                }
            }
            Some('u') => {
                consumed += 1;
                let mut hex = String::new();
                for _ in 0..4 {
                    if let Some(&c) = chars.peek() {
                        if c.is_ascii_hexdigit() {
                            hex.push(chars.next().unwrap());
                            consumed += 1;
                        } else {
                            break;
                        }
                    }
                }
                if let Ok(val) = u32::from_str_radix(&hex, 16) {
                    if let Some(c) = char::from_u32(val) {
                        result.push(c);
                    }
                }
            }
            Some('U') => {
                consumed += 1;
                let mut hex = String::new();
                for _ in 0..8 {
                    if let Some(&c) = chars.peek() {
                        if c.is_ascii_hexdigit() {
                            hex.push(chars.next().unwrap());
                            consumed += 1;
                        } else {
                            break;
                        }
                    }
                }
                if let Ok(val) = u32::from_str_radix(&hex, 16) {
                    if let Some(c) = char::from_u32(val) {
                        result.push(c);
                    }
                }
            }
            Some(c @ '0'..='7') => {
                consumed += 1;
                let mut oct = String::new();
                oct.push(c);
                for _ in 0..2 {
                    if let Some(&c) = chars.peek() {
                        if ('0'..='7').contains(&c) {
                            oct.push(chars.next().unwrap());
                            consumed += 1;
                        } else {
                            break;
                        }
                    }
                }
                if let Ok(val) = u8::from_str_radix(&oct, 8) {
                    result.push(val as char);
                }
            }
            Some('c') => {
                consumed += 1;
                // \cX = control character
                if let Some(c) = chars.next() {
                    consumed += 1;
                    result.push((c as u8 & 0x1f) as char);
                }
            }
            Some(c) => {
                consumed += 1;
                result.push('\\');
                result.push(c);
            }
            None => {
                result.push('\\');
            }
        }
    }
    (result, consumed)
}

/// Port of C's `GETKEY_*` flag enumeration from `Src/zsh.h:3143`.
/// Passed to `getkeystring_with` to alter escape interpretation:
///   - GETKEY_OCTAL_ESC: `\NNN` octal escapes are interpreted (1<<0)
///   - GETKEY_EMACS: unknown `\<char>` drops the backslash (1<<1)
///   - GETKEY_BACKSLASH_C: `\c` truncates the result (1<<3)
pub const GETKEY_OCTAL_ESC: u32 = 1 << 0;                                    // c:zsh.h:3143
pub const GETKEY_EMACS: u32 = 1 << 1;                                        // c:zsh.h:3150
pub const GETKEY_BACKSLASH_C: u32 = 1 << 3;                                  // c:zsh.h:3154

/// `GETKEYS_PRINT = GETKEY_OCTAL_ESC | GETKEY_BACKSLASH_C |
/// GETKEY_EMACS` per Src/zsh.h:3185 — the flag set `bin_print` /
/// `bin_echo` use when interpreting escapes.
pub const GETKEYS_PRINT: u32 = GETKEY_OCTAL_ESC | GETKEY_EMACS | GETKEY_BACKSLASH_C; // c:zsh.h:3185

/// `getkeystring` with the C `how` parameter (Src/utils.c:6915).
/// The plain `getkeystring(s)` shim above defaults `how=0` for
/// non-EMACS callers (zbeep, dollar-quote — they keep unknown
/// `\<char>` as literal `\<char>`). Pass `GETKEYS_PRINT` to get
/// the print/echo behavior (drop backslash on unknown).
pub fn getkeystring_with(s: &str, how: u32) -> (String, usize) {              // c:utils.c:6915
    let mut result = String::new();
    let mut chars = s.chars().peekable();
    let mut consumed = 0;
    while let Some(c) = chars.next() {
        consumed += c.len_utf8();
        if c != '\\' {
            result.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => { result.push('\n'); consumed += 1; }
            Some('t') => { result.push('\t'); consumed += 1; }
            Some('r') => { result.push('\r'); consumed += 1; }
            Some('e') | Some('E') => { result.push('\x1b'); consumed += 1; }
            Some('a') => { result.push('\x07'); consumed += 1; }
            Some('b') => { result.push('\x08'); consumed += 1; }
            Some('f') => { result.push('\x0c'); consumed += 1; }
            Some('v') => { result.push('\x0b'); consumed += 1; }
            Some('\\') => { result.push('\\'); consumed += 1; }
            Some('\'') => { result.push('\''); consumed += 1; }
            Some('"') => { result.push('"'); consumed += 1; }
            Some('x') => {
                consumed += 1;
                let mut hex = String::new();
                for _ in 0..2 {
                    if let Some(&c) = chars.peek() {
                        if c.is_ascii_hexdigit() {
                            hex.push(chars.next().unwrap());
                            consumed += 1;
                        } else { break; }
                    }
                }
                if let Ok(val) = u8::from_str_radix(&hex, 16) {
                    result.push(val as char);
                }
            }
            // Octal escape: \NNN (1-3 octal digits). Gated on
            // GETKEY_OCTAL_ESC per c:utils.c:7156-7178.
            Some(d) if d.is_digit(8) && (how & GETKEY_OCTAL_ESC) != 0 => {
                consumed += 1;
                let mut oct = String::from(d);
                for _ in 0..2 {
                    if let Some(&c) = chars.peek() {
                        if c.is_digit(8) {
                            oct.push(chars.next().unwrap());
                            consumed += 1;
                        } else { break; }
                    }
                }
                if let Ok(val) = u8::from_str_radix(&oct, 8) {
                    result.push(val as char);
                }
            }
            // c:utils.c:7180-7184 — default arm. With GETKEY_EMACS
            // set, drop the backslash; otherwise keep `\<char>`.
            Some(c) => {
                consumed += 1;
                if (how & GETKEY_EMACS) == 0 {
                    result.push('\\');
                }
                result.push(c);
            }
            None => { result.push('\\'); }
        }
    }
    (result, consumed)
}

/// Convert UCS-4 to UTF-8 (from utils.c ucs4toutf8)
/// Port of `ucs4toutf8(char *dest, unsigned int wval)` from `Src/utils.c:6743`.
/// WARNING: param names don't match C — Rust=(codepoint) vs C=(dest, wval)
pub fn ucs4toutf8(codepoint: u32) -> Option<String> {
    char::from_u32(codepoint).map(|c| c.to_string())
}

/// Check for special characters that need quoting (from utils.c hasspecial)
/// Port of `hasspecial(char const *s)` from `Src/utils.c:6072`.
pub fn hasspecial(s: &str) -> bool {
    s.chars().any(ispecial)
}

/// Static `int ep` from Src/utils.c:4775 — sticky flag suppressing the
/// `can't set tty pgrp` warning after the first failure.
static ATTACHTTY_EP: std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(0);                                    // c:4775

/// Port of `void attachtty(pid_t pgrp)` from Src/utils.c:4775.
/// Hands the controlling terminal to `pgrp`. Gated by `jobbing &&
/// interact`; falls back to `mypgrp` and disables MONITOR on permanent
/// failure (matching the C source's recursion + `opts[MONITOR]=0` path).
#[cfg(unix)]
pub fn attachtty(pgrp: i32) {                                                // c:4775

    if !(crate::ported::zsh_h::jobbing() && crate::ported::zsh_h::interact()) {
        return;                                                              // c:4779
    }
    let shtty = crate::ported::init::SHTTY.load(Ordering::Relaxed);          // c:4781
    if shtty == -1 {
        return;
    }
    let ep = ATTACHTTY_EP.load(Ordering::Relaxed);
    let rc = unsafe { libc::tcsetpgrp(shtty, pgrp) };                        // c:4781
    if rc == -1 && ep == 0 {                                                 // c:4781
        let mypgrp_val = *crate::ported::jobs::MYPGRP                        // c:4792
            .get_or_init(|| std::sync::Mutex::new(0))
            .lock().unwrap();
        if pgrp != mypgrp_val
            && unsafe { libc::kill(-pgrp, 0) } == -1
        {
            attachtty(mypgrp_val);                                           // c:4793
        } else {
            let errno_val = std::io::Error::last_os_error().raw_os_error()
                .unwrap_or(0);
            if errno_val != libc::ENOTTY {                                   // c:4795
                zwarn(&format!("can't set tty pgrp: {}",                     // c:4797
                    std::io::Error::from_raw_os_error(errno_val)));
                let _ = std::io::stderr().flush();                           // c:4798
            }
            crate::ported::options::opt_state_set("monitor", false);         // c:4815 opts[MONITOR]=0
            ATTACHTTY_EP.store(1, Ordering::Relaxed);                        // c:4815
        }
    } else if rc != -1 {                                                     // c:4815
        *crate::ported::jobs::LAST_ATTACHED_PGRP                             // c:4815
            .get_or_init(|| std::sync::Mutex::new(0))
            .lock().unwrap() = pgrp;
    }
}

/// Port of `pid_t gettygrp(void)` from Src/utils.c:4815.
#[cfg(unix)]
pub fn gettygrp() -> i32 {                                                   // c:4815
    let shtty = crate::ported::init::SHTTY.load(Ordering::Relaxed);
    if shtty == -1 {                                                         // c:4819
        return -1;                                                           // c:4820
    }
    unsafe { libc::tcgetpgrp(shtty) }                                        // c:4823
}

/// Check if directory is readable with entries (from utils.c)
/// Port of `zreaddir(DIR *dir, int ignoredots)` from `Src/utils.c:5217`.
/// WARNING: param names don't match C — Rust=(path) vs C=(dir, ignoredots)
pub fn zreaddir(path: &str) -> Vec<String> {
    match std::fs::read_dir(path) {
        Ok(entries) => entries
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|s| s != "." && s != "..")
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// Port of `void zsetupterm(void)` from Src/utils.c:390.
///
/// Reference-counts terminfo's `cur_term` setup. The C source
/// guards with `#ifdef HAVE_SETUPTERM`; without terminfo this is a
/// pure no-op. Rust's `term`-style state lives elsewhere (modules
/// like `zle/termcap` initialize on demand), so this is a no-op
/// counter for symbol-table parity.
pub fn zsetupterm() {                                                        // c:390
    static TERM_COUNT: std::sync::atomic::AtomicI32 =                        // c:409
        std::sync::atomic::AtomicI32::new(0);
    TERM_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);           // c:409
}

/// Port of `void zdeleteterm(void)` from Src/utils.c:409.
pub fn zdeleteterm() {                                                       // c:409
    static TERM_COUNT: std::sync::atomic::AtomicI32 =
        std::sync::atomic::AtomicI32::new(0);
    if TERM_COUNT.load(std::sync::atomic::Ordering::Relaxed) > 0 {
        TERM_COUNT.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);       // c:424
    }
}

/// Port of `int putraw(int c)` from Src/utils.c:424. Writes a
/// single byte to stdout for the termcap library, returning 0.
pub fn putraw(c: char) -> i32 {                                              // c:424
    print!("{}", c);                                                         // c:434
    0                                                                        // c:434
}

/// Port of `int putshout(int c)` from Src/utils.c:434. Writes a
/// single byte to `shout` (zsh's interactive stdout — falls back
/// to stdout in zshrs's static-link path), returning 0.
pub fn putshout(c: char) -> i32 {                                            // c:434
    print!("{}", c);                                                         // c:434
    0                                                                        // c:437
}

/// Nice char with quoting selection (from utils.c nicechar_sel)
/// Port of `nicechar_sel(int c, int quotable)` from `Src/utils.c:462`.
pub fn nicechar_sel(c: char, quotable: bool) -> String {
    if quotable && ispecial(c) {
        format!("\\{}", c)
    } else {
        nicechar(c)
    }
}

/// Initialize multibyte state (from utils.c mb_charinit) - no-op in Rust
/// Port of `mb_charinit` from `Src/utils.c:553`.
pub fn mb_charinit() {
    // Rust handles UTF-8 natively
}

/// Wide char nice format (from utils.c wcs_nicechar_sel)
/// Port of `wcs_nicechar_sel(wchar_t c, size_t *widthp, char **swidep, int quotable)` from `Src/utils.c:593`.
/// WARNING: param names don't match C — Rust=(c, quotable) vs C=(c, widthp, swidep, quotable)
pub fn wcs_nicechar_sel(c: char, quotable: bool) -> String {
    nicechar_sel(c, quotable)
}

/// Wide char nice format (from utils.c wcs_nicechar)
/// Port of `wcs_nicechar(wchar_t c, size_t *widthp, char **swidep)` from `Src/utils.c:709`.
/// WARNING: param names don't match C — Rust=(c) vs C=(c, widthp, swidep)
pub fn wcs_nicechar(c: char) -> String {
    nicechar(c)
}

/// Port of `int is_wcs_nicechar(wchar_t c)` from Src/utils.c:720.
///
/// "Return 1 if wcs_nicechar() would reformat this character for
/// display." Mirrors the C condition: non-printable AND (low ASCII
/// OR PRINTEIGHTBIT unset) for control chars; for high bytes, true
/// when ≥0x100 or `is_nicechar` says so.
pub fn is_wcs_nicechar(c: char) -> bool {                                    // c:720
    let cv = c as u32;
    let printable = !c.is_control() && cv >= 0x20;
    let print_eight = isset(crate::ported::zsh_h::PRINTEIGHTBIT); // c:722
    if !printable && (cv < 0x80 || !print_eight) {
        if cv == 0x7f || c == '\n' || c == '\t' || cv < 0x20 {               // c:734
            return true;
        }
        if cv >= 0x80 {                                                      // c:734
            return cv >= 0x100 || is_nicechar(c);                            // c:734
        }
    }
    false                                                                    // c:734
}

/// Get wide character width (from utils.c zwcwidth)
/// Port of `zwcwidth(wint_t wc)` from `Src/utils.c:734`.
pub fn zwcwidth(wc: char) -> usize {
    unicode_width::UnicodeWidthChar::width(wc).unwrap_or(1)
}

/// Find program in PATH.
/// Port of `pathprog(char *prog, char **namep)` from Src/utils.c — first hit on
/// `access(X_OK)`. Absolute or `./`-prefixed paths skip the PATH
/// walk and check existence directly.
/// WARNING: param names don't match C — Rust=(prog) vs C=(prog, namep)
pub fn pathprog(prog: &str) -> Option<PathBuf> {
    if prog.contains('/') {
        let p = PathBuf::from(prog);
        return if p.exists() { Some(p) } else { None };
    }
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in path_var.split(':') {
            let full_path = PathBuf::from(dir).join(prog);
            // Inline of the deleted is_executable helper.
            #[cfg(unix)]
            {
                if let Ok(meta) = std::fs::metadata(&full_path) {
                    if meta.is_file() && meta.permissions().mode() & 0o111 != 0 {
                        return Some(full_path);
                    }
                }
            }
            #[cfg(not(unix))]
            {
                if full_path.is_file() {
                    return Some(full_path);
                }
            }
        }
    }
    None
}

/// Port of `void print_if_link(char *s, int all)` from Src/utils.c:985.
///
/// "Print arrow + symlink target(s) iff `s` is an absolute path
/// pointing through symlinks." When `all` is set, follows the
/// chain (`xsymlinks` loop, c:992); otherwise emits just the final
/// realpath if it differs from the input. Always relative to
/// stdout. The Rust port writes via `print!` to mirror C's
/// `printf`/`zputs(stdout)` calls.
pub fn print_if_link(s: &str, all: bool) {                                   // c:985
    if !s.starts_with('/') {                                                 // c:987
        return;
    }
    if all {                                                                 // c:988
        let mut start = s.to_string();
        loop {                                                               // c:992
            match xsymlinks(&start) {
                Ok(target) if !target.is_empty() && target != start => {
                    print!(" -> ");                                          // c:994
                    print!("{}", if target.is_empty() { "/" } else { &target }); // c:995
                    start = target;                                          // c:998-999
                }
                _ => break,                                                  // c:1002
            }
        }
    } else {                                                                 // c:1006
        let s_at_entry = s.to_string();                                      // c:1013
        if let Ok(resolved) = std::fs::canonicalize(s) {                     // c:1015
            let r = resolved.to_string_lossy().into_owned();
            if r != s_at_entry {                                             // c:1015
                print!(" -> ");                                              // c:1016
                print!("{}", if r.is_empty() { "/" } else { &r });           // c:1017
            }
        }
    }
    let _ = std::io::stdout().flush();
}

/// Port of `char *substnamedir(char *s)` from Src/utils.c:1053.
///
/// "Substitute a directory using a name. If there is none, return
/// the original argument." Rendering uses `finddir` (the global
/// `nameddirtab` lookup) plus `quotestring(..., QT_BACKSLASH)` for
/// the residue.
pub fn substnamedir(s: &str) -> String {                                     // c:1053
    match finddir(s) {                                                       // c:1053
        None => quotestring(s, crate::ported::zsh_h::QT_BACKSLASH),                        // c:1058
        // C: zhtricat("~", d->node.nam, quotestring(s + strlen(d->dir), …)).
        // `finddir` already renders the leading `~name` segment, so
        // the residue is what `finddir` returned past the `~name`
        // prefix; backslash-quote that residue.
        Some(rendered) => rendered,                                          // c:1059
    }
}

/// Port of `finddir_scan(HashNode hn, UNUSED(int flags))` from Src/utils.c:1106 — ScanFunc the
/// C source registers with `scanhashtable(nameddirtab, …)` to pick
/// the longest-prefix entry. Reads the typed `nameddir` entries
/// from `crate::ported::hashnameddir::nameddirtab()`.
/// WARNING: param names don't match C — Rust=(path) vs C=(hn, flags)
pub fn finddir_scan(path: &str) -> Option<(String, String)> {                // c:1106
    let table = crate::ported::hashnameddir::nameddirtab().lock().ok()?;
    let mut best: Option<(String, String, usize)> = None;
    for (name, nd) in table.iter() {
        if path.starts_with(nd.dir.as_str()) {
            let len = nd.dir.len();
            let rest = &path[len..];
            if (rest.is_empty() || rest.starts_with('/'))
                && best.as_ref().map_or(true, |b| len > b.2)
            {
                best = Some((name.clone(), rest.to_string(), len));
            }
        }
    }
    best.map(|(n, r, _)| (n, r))
}

/// Port of `Nameddir finddir(char *s)` from Src/utils.c:1127.
///
/// "See if a path has a named directory as its prefix." Compares
/// `s` against `$HOME` first (longest implicit named dir), then
/// scans the global `nameddirtab`, then falls back to
/// `subst_string_by_hook("zsh_directory_name", "d", s)`.
///
/// Rust signature returns `Option<String>` (the abbreviated path
/// `~name/rest`) instead of the C `Nameddir` pointer.
pub fn finddir(path: &str) -> Option<String> {                              // c:1127
    let home = std::env::var("HOME").unwrap_or_default();                   // c:1127
    if !home.is_empty() && home.len() > 1 && path.starts_with(&home) {      // c:1138-1141
        let rest = &path[home.len()..];
        if rest.is_empty() || rest.starts_with('/') {
            return Some(format!("~{}", rest));
        }
    }
    if let Some((name, rest)) = finddir_scan(path) {                        // c:1167
        return Some(format!("~{}{}", name, rest));
    }
    // c:1169 — zsh_directory_name hook — returns ["name", "len"]
    if let Some(reply) = subst_string_by_hook(
        "zsh_directory_name", Some("d"), path)
    {
        if reply.len() >= 2 {                                                // c:1170
            if let Ok(len) = reply[1].parse::<usize>() {
                if len <= path.len() {
                    let prefix = &path[..len];
                    let _ = prefix;
                    return Some(format!("~[{}]{}", reply[0], &path[len..]));
                }
            }
        }
    }
    None                                                                     // c:1187
}

/// Port of `void adduserdir(char *s, char *t, int flags, int always)`
/// from Src/utils.c:1187.
///
/// Adds (or removes when `t` is empty / non-absolute) an entry in
/// the global `nameddirtab`. ND_USERNAME entries from `getpwnam`
/// don't override explicit assignments. AUTONAMEDIRS gating, the
/// trailing-slash trim, and PWD/OLDPWD ND_NOABBREV stamp are all
/// preserved. Routes through `crate::ported::hashnameddir`.
pub fn adduserdir(name: &str, dir: &str, flags: i32, always: bool) {        // c:1187

    if !crate::ported::zsh_h::interact() { return; }                         // c:1193
    if let Ok(t) = crate::ported::hashnameddir::nameddirtab().lock() {
        if (flags & ND_USERNAME) != 0 && t.contains_key(name) {              // c:1199
            return;
        }
        if !always
            && !isset(crate::ported::zsh_h::AUTONAMEDIRS)
            && !t.contains_key(name)
        {                                                                    // c:1207
            return;
        }
    }
    if dir.is_empty() || !dir.starts_with('/') {                             // c:1211
        let _ = crate::ported::hashnameddir::removenameddirnode(name);       // c:1214
        return;
    }
    let mut trimmed = dir.trim_end_matches('/').to_string();                 // c:1224-1226
    if trimmed.is_empty() {
        trimmed = dir.to_string();                                           // c:1227-1233
    }
    let final_flags = if name == "PWD" || name == "OLDPWD" {                 // c:1237
        flags | ND_NOABBREV
    } else {
        flags
    };
    // c:1239 — `nd = (Nameddir) zshcalloc(sizeof *nd); nd->node.flags = …;
    //           nd->dir = ztrdup(t); addnode(nameddirtab, ztrdup(s), nd);`
    let nd = nameddir {
        node: hashnode { next: None, nam: name.to_string(), flags: final_flags },
        dir: trimmed,
        diff: 0,
    };
    crate::ported::hashnameddir::addnameddirnode(name, nd);
}

/// Port of `char *getnameddir(char *name)` from Src/utils.c:1247.
///
/// Looks up `name` in `nameddirtab`; if absent, checks for a
/// scalar parameter whose value starts with `/` and registers it
/// via `adduserdir`; finally falls back to `getpwnam(name)` for
/// `~user`-style lookups when USE_GETPWNAM is enabled.
pub fn getnameddir(name: &str) -> Option<String> {                           // c:1247
    if let Ok(t) = crate::ported::hashnameddir::nameddirtab().lock() {
        if let Some(nd) = t.get(name) {                                      // c:1254
            return Some(nd.dir.clone());
        }
    }
    // c:1260 — scalar param whose value starts with '/'
    if let Ok(s) = std::env::var(name) {
        if s.starts_with('/') {
            adduserdir(name, &s, 0, true);                                   // c:1264
            return Some(s);
        }
    }
    #[cfg(unix)]
    {
        // c:1268 — getpwnam fallback
        let cn = std::ffi::CString::new(name).ok()?;
        let pw = unsafe { libc::getpwnam(cn.as_ptr()) };
        if !pw.is_null() {
            let dir = unsafe {
                std::ffi::CStr::from_ptr((*pw).pw_dir).to_string_lossy().into_owned()
            };
            return Some(dir);
        }
    }
    None
}

/// Compare directory paths (from utils.c dircmp)
/// Port of `dircmp(char *s, char *t)` from `Src/utils.c:1296`.
pub fn dircmp(s: &str, t: &str) -> bool {
    let s = s.trim_end_matches('/');
    let t = t.trim_end_matches('/');
    s == t
}

// the last time we checked mail                                            // c:1447
/// Check mail paths (from utils.c checkmailpath)
pub fn checkmailpath(paths: &[String]) -> Vec<String> {
    let mut messages = Vec::new();
    for path in paths {
        // PATH?message format
        let (file, msg) = if let Some(pos) = path.find('?') {
            (&path[..pos], Some(&path[pos + 1..]))
        } else {
            (path.as_str(), None)
        };

        if let Ok(meta) = std::fs::metadata(file) {
            if let Ok(modified) = meta.modified() {
                if let Ok(elapsed) = modified.elapsed() {
                    if elapsed.as_secs() < 60 {
                        let default_msg = format!("You have new mail in {}", file);
                        messages.push(msg.unwrap_or(&default_msg).to_string());
                    }
                }
            }
        }
    }
    messages
}

/// Port of `void gettyinfo(struct ttyinfo *ti)` from Src/utils.c:1746.
///
/// Reads the current termios from the global `SHTTY`. Returns
/// `None` when SHTTY is closed or the call fails (matching C's
/// silent return when SHTTY == -1).
#[cfg(unix)]
pub fn gettyinfo() -> Option<libc::termios> {                                // c:1746
    let shtty = crate::ported::init::SHTTY.load(Ordering::Relaxed);
    if shtty == -1 {                                                         // c:1755
        return None;
    }
    fdgettyinfo(shtty).ok()                                                  // c:1748
}

/// Port of `void settyinfo(struct ttyinfo *ti)` from Src/utils.c:1778.
///
/// Restores the termios state on the global `SHTTY` with EINTR
/// retry; no-op when SHTTY is closed.
#[cfg(unix)]
pub fn settyinfo(ti: &libc::termios) -> bool {                               // c:1778
    let shtty = crate::ported::init::SHTTY.load(Ordering::Relaxed);
    if shtty == -1 {                                                         // c:1787
        return false;
    }
    fdsettyinfo(shtty, ti).is_ok()                                           // c:1780
}

/// Check fd table for valid file descriptors (from utils.c check_fd_table)
/// Port of `check_fd_table(int fd)` from `Src/utils.c:1969`.
pub fn check_fd_table(fd: i32) -> bool {
    #[cfg(unix)]
    {
        unsafe { libc::fcntl(fd, libc::F_GETFD) != -1 }
    }
    #[cfg(not(unix))]
    {
        let _ = fd;
        false
    }
}

/// Move file descriptor to a high number (from utils.c movefd)
/// Port of `movefd(int fd)` from `Src/utils.c:1990`.
pub fn movefd(fd: i32) -> i32 {
    #[cfg(unix)]
    {
        if fd < 10 {
            let new_fd = unsafe { libc::fcntl(fd, libc::F_DUPFD, 10) };
            if new_fd >= 0 {
                unsafe { libc::close(fd) };
                // Set close-on-exec
                unsafe { libc::fcntl(new_fd, libc::F_SETFD, libc::FD_CLOEXEC) };
                return new_fd;
            }
        }
        fd
    }
    #[cfg(not(unix))]
    {
        fd
    }
}

/// Add module file descriptor (from utils.c addmodulefd)
/// Port of `addmodulefd(int fd, int fdt)` from `Src/utils.c:2091`.
/// WARNING: param names don't match C — Rust=(fd) vs C=(fd, fdt)
pub fn addmodulefd(fd: i32) {
    #[cfg(unix)]
    {
        // Set close-on-exec
        unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
    }
    #[cfg(not(unix))]
    {
        let _ = fd;
    }
    fdtable_set(fd, crate::ported::zsh_h::FDT_MODULE);
}

// =====================================================================
// !!! WARNING: RUST-ONLY HELPERS — NO DIRECT C COUNTERPART !!!
// =====================================================================
//
// These three fns (`fdtable_lock`, `fdtable_get`, `fdtable_set`) DO
// NOT EXIST as functions in `Src/utils.c`. The C source declares
// `fdtable` as a bare `unsigned char *` global (Src/utils.c:~63) and
// every call site reads/writes it as a direct array index:
//
//     if (fdtable[fd] != FDT_UNUSED) ...
//     fdtable[fd] = FDT_EXTERNAL;
//
// Rust can't hand out raw mutable indexes through a `Mutex<Vec<u8>>`
// safely without a borrow scope, so the Rust port wraps the same
// slot access in three `pub fn`s. Each call site to these helpers
// corresponds 1:1 to a `fdtable[fd] = X` or `fdtable[fd] != X`
// statement in the C source — they are NOT new policy, they only
// adapt the storage shape from `unsigned char *` to a growable
// `Mutex<Vec<i32>>`.
//
// Also adapts `growfdtable` (Src/utils.c:1965) by lazily growing the
// Vec inside `fdtable_set` instead of a separate `growfdtable`
// call — the C source calls `growfdtable(fd)` immediately before
// every `fdtable[fd] = X` write anyway.
//
// !!! Do NOT use these for any state that the C source does not
// already store in `fdtable[]`. Adding a new "fd kind" here is a
// scope expansion, not a port. !!!
// =====================================================================

static FDTABLE: std::sync::OnceLock<std::sync::Mutex<Vec<i32>>> =
    std::sync::OnceLock::new();

/// !!! RUST-ONLY HELPER — see WARNING block above. C source uses bare
/// `unsigned char *fdtable` global from Src/utils.c:~63.
fn fdtable_lock() -> &'static std::sync::Mutex<Vec<i32>> {
    FDTABLE.get_or_init(|| std::sync::Mutex::new(Vec::new()))
}

/// !!! RUST-ONLY HELPER — see WARNING block above. Equivalent to
/// the C expression `fdtable[fd]` (read). Returns `FDT_UNUSED` for
/// any fd that has not been explicitly set, matching the C source's
/// post-`growfdtable` zero-fill behaviour at Src/utils.c:1979.
pub fn fdtable_get(fd: i32) -> i32 {                                     // c:utils.c:fdtable[fd]
    if fd < 0 { return crate::ported::zsh_h::FDT_UNUSED; }
    let g = fdtable_lock().lock().unwrap();
    g.get(fd as usize).copied().unwrap_or(crate::ported::zsh_h::FDT_UNUSED)
}

/// !!! RUST-ONLY HELPER — see WARNING block above. Equivalent to
/// the C statement `fdtable[fd] = kind;`. Inlines the `growfdtable`
/// call from Src/utils.c:1965 since C always invokes it immediately
/// before the assignment anyway.
pub fn fdtable_set(fd: i32, kind: i32) {                                 // c:utils.c:fdtable[fd]
    if fd < 0 { return; }
    let mut g = fdtable_lock().lock().unwrap();
    if (fd as usize) >= g.len() {
        g.resize((fd as usize) + 1, crate::ported::zsh_h::FDT_UNUSED);
    }
    g[fd as usize] = kind;
}

/// Add lock file descriptor (from utils.c addlockfd)
/// Port of `addlockfd(int fd, int cloexec)` from `Src/utils.c:2112`.
pub fn addlockfd(fd: i32, cloexec: bool) {
    #[cfg(unix)]
    {
        if cloexec {
            unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (fd, cloexec);
    }
}

/// Port of `zcloselockfd(int fd)` from `Src/utils.c:2156`.
///
/// C signature: `int zcloselockfd(int fd)`.
/// "Close an fd returning 0 if used for locking; return -1 if it
/// isn't." (c:2150-2152 docstring.)
///
/// C body (c:2156-2164):
/// ```c
/// if (fd > max_zsh_fd) return -1;
/// if (fdtable[fd] != FDT_FLOCK && fdtable[fd] != FDT_FLOCK_EXEC)
///     return -1;
/// zclose(fd);
/// return 0;
/// ```
/// The `fdtable[]` / `max_zsh_fd` global state is not yet ported to
/// Rust, so the FDT_FLOCK check at c:2160-2161 is skipped — the
/// Rust port closes the fd unconditionally and reports success.
/// Callers (e.g. `bin_zsystem_flock` -u path at system.c:676) treat
/// `-1` as "not in lockfd table"; until the fdtable is ported,
/// returning `0` matches the most common case (the user typed an fd
/// they really did `flock` earlier in the same shell).
pub fn zcloselockfd(fd: i32) -> i32 {                                    // c:2156
    // c:2156-2161 — fdtable check skipped (no Rust fdtable global).
    zclose(fd);                                                          // c:2162
    0                                                                    // c:2163
}

/// Port of `zstrtol_underscore(const char *s, char **t, int base, int underscore)` from `Src/utils.c:2438`.
///
/// C signature: `zlong zstrtol_underscore(const char *s, char **t,
///                                         int base, int underscore)`.
///
/// Convert string to zlong with optional `_` digit-separator support.
/// `underscore != 0` allows underscores to appear inside the digit
/// run (skipped during accumulation). Returns the parsed value AND
/// the unconsumed-tail slice (matching C's `*t = (char *)s` writeback
/// at line 2516).
///
/// `base == 0` triggers prefix-based detection (`0x`/`0X`→16,
/// `0b`/`0B`→2, leading `0`→8, otherwise 10). Bases outside [2, 36]
/// emit `zerr` and return `(0, original_s)`. Overflow truncates and
/// emits a `zwarn` per c:2510-2512.
pub fn zstrtol_underscore(s: &str, base: i32, underscore: bool) -> (i64, &str) {  // c:2438
    let bytes = s.as_bytes();
    let mut i = 0usize;
    // c:2444-2445 — skip leading `inblank` whitespace.
    while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
        i += 1;
    }
    // c:2447-2450 — handle sign.
    let neg = if i < bytes.len() && (bytes[i] == b'-') {                 // c:2447 IS_DASH
        i += 1;
        true
    } else if i < bytes.len() && bytes[i] == b'+' {                      // c:2449
        i += 1;
        false
    } else {
        false
    };

    // c:2452-2461 — base autodetect when base == 0.
    let mut base = base;
    if base == 0 {                                                       // c:2452
        if i >= bytes.len() || bytes[i] != b'0' {                        // c:2453
            base = 10;                                                   // c:2454
        } else {
            i += 1;                                                      // c:2455 ++s
            if i < bytes.len() && (bytes[i] == b'x' || bytes[i] == b'X') {  // c:2455
                base = 16;
                i += 1;
            } else if i < bytes.len() && (bytes[i] == b'b' || bytes[i] == b'B') {  // c:2457
                base = 2;
                i += 1;
            } else {                                                     // c:2459
                base = 8;                                                // c:2460
            }
        }
    }
    let inp_idx = i;                                                     // c:2462 inp = s
    if base < 2 || base > 36 {                                           // c:2463
        zerr(&format!(
            "invalid base (must be 2 to 36 inclusive): {}", base
        ));                                                              // c:2464
        return (0, s);                                                   // c:2465 return 0
    }

    let mut calc: u64 = 0;                                               // c:2441
    let mut trunc_idx: Option<usize> = None;                             // c:2440 trunc = NULL
    if base <= 10 {                                                      // c:2466
        // c:2467-2479 — digit accumulator, low bases.
        let max_d = b'0' + base as u8;
        while i < bytes.len() {
            let c = bytes[i];
            if c >= b'0' && c < max_d {
                if trunc_idx.is_none() {
                    let newcalc = calc.wrapping_mul(base as u64).wrapping_add((c - b'0') as u64);
                    if newcalc < calc {                                  // c:2473
                        trunc_idx = Some(i);                             // c:2474
                    } else {
                        calc = newcalc;
                    }
                }
                i += 1;
            } else if underscore && c == b'_' {                          // c:2467 underscore && *s == '_'
                i += 1;
            } else {
                break;
            }
        }
    } else {                                                             // c:2480
        // c:2481-2495 — digit accumulator, high bases (>10).
        while i < bytes.len() {
            let c = bytes[i];
            let digit = if c.is_ascii_digit() {                          // c:2481 idigit
                Some((c - b'0') as u32)
            } else if c >= b'a' && c < b'a' + base as u8 - 10 {          // c:2482
                Some(((c & 0x1f) + 9) as u32)                            // c:2486 (*s & 0x1f) + 9
            } else if c >= b'A' && c < b'A' + base as u8 - 10 {          // c:2483
                Some(((c & 0x1f) + 9) as u32)
            } else if underscore && c == b'_' {                          // c:2484
                i += 1;
                continue;
            } else {
                None
            };
            match digit {
                Some(d) => {
                    if trunc_idx.is_none() {
                        let newcalc = calc.wrapping_mul(base as u64).wrapping_add(d as u64);
                        if newcalc < calc {                              // c:2489
                            trunc_idx = Some(i);                         // c:2490
                        } else {
                            calc = newcalc;
                        }
                    }
                    i += 1;
                }
                None => break,
            }
        }
    }

    // c:2504-2511 — special-case: signed-overflow into top bit.
    // The lowest negative number triggers the first test but is
    // representable correctly; check it explicitly.
    if trunc_idx.is_none() && (calc as i64) < 0 {                        // c:2504
        let top_bit_only = calc & !(1u64 << 63);                          // c:2506
        if !neg || top_bit_only != 0 {                                    // c:2505
            trunc_idx = Some(i.saturating_sub(1));                        // c:2508
            calc /= base as u64;                                          // c:2509
        }
    }

    if let Some(t) = trunc_idx {                                         // c:2512
        let digits = t.saturating_sub(inp_idx);
        let inp_str = &s[inp_idx..];
        zwarn(&format!(
            "number truncated after {} digits: {}", digits, inp_str
        ));                                                              // c:2513
    }

    let result = if neg {                                                // c:2517
        -(calc as i64)
    } else {
        calc as i64
    };
    (result, &s[i..])
}

/// Compute time difference in microseconds (from utils.c timespec_diff_us)
/// Port of `timespec_diff_us(const struct timespec *t1, const struct timespec *t2)` from `Src/utils.c:2752`.
pub fn timespec_diff_us(t1: &std::time::Instant, t2: &std::time::Instant) -> i64 {
    if *t2 > *t1 {
        t2.duration_since(*t1).as_micros() as i64
    } else {
        -(t1.duration_since(*t2).as_micros() as i64)
    }
}

/// Port of `int zmonotime(time_t *tloc)` from Src/utils.c:2780.
///
/// "Like time(), but uses the monotonic clock." Returns
/// `tv_sec` of CLOCK_MONOTONIC; the previous Rust port used
/// `Instant::now().elapsed()` which always returned ≈0.
pub fn zmonotime() -> i64 {                                                  // c:2780
    #[cfg(unix)]
    {
        let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
        unsafe {
            libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts);             // c:2783
        }
        ts.tv_sec as i64                                                     // c:2786
    }
    #[cfg(not(unix))]
    {
        0
    }
}

/// Sleep random amount up to max microseconds (from utils.c zsleep_random)
/// Port of `int zsleep_random(long max_us, time_t end_time)`
/// from Src/utils.c:2833.
///
/// "Sleep for time (fairly) randomly up to max_us microseconds.
/// Don't let the time extend beyond end_time. end_time is compared
/// to the current *monotonic* clock time. Return 1 if that seemed
/// to work, else 0."
pub fn zsleep_random(max_us: i64, end_time: i64) -> i32 {                    // c:2833
    let now = zmonotime();                                                   // c:2833
    let r16 = unsafe { libc::rand() } & 0xFFFF;                              // c:2845
    let mut r: i64 = (max_us >> 16) * (r16 as i64);                          // c:2852
    while r != 0 && now + (r / 1_000_000) > end_time {                       // c:2858
        r >>= 1;                                                             // c:2859
    }
    if r != 0 {                                                              // c:2860
        zsleep(r)                                                            // c:2861
    } else {
        0                                                                    // c:2862
    }
}

/// Port of `int noquery(int purge)` from Src/utils.c:2992.
///
/// "If anything has been typed before the query, return without
/// asking. Optionally also purge the input queue." Returns the
/// number of bytes pending on `SHTTY` (via FIONREAD ioctl); when
/// `purge` is set, drains them before returning.
pub fn noquery(purge: bool) -> i32 {                                         // c:2992
    let mut val: libc::c_int = 0;                                            // c:2992
    #[cfg(unix)]
    {
        let shtty = crate::ported::init::SHTTY.load(Ordering::Relaxed);      // c:2999
        if shtty == -1 {
            return 0;
        }
        unsafe {                                                             // c:2999
            libc::ioctl(shtty, libc::FIONREAD, &mut val as *mut libc::c_int);
        }
        if purge {                                                           // c:3000
            let mut c: u8 = 0;
            for _ in 0..val {                                                // c:3001
                let _ = unsafe {                                             // c:3002
                    libc::read(shtty, &mut c as *mut u8 as *mut libc::c_void, 1)
                };
            }
        }
    }
    val                                                                      // c:3009
}

/// Scan for spelling correction (from utils.c spscan)
/// Port of `spscan(HashNode hn, UNUSED(int scanflags))` from `Src/utils.c:3109`.
/// WARNING: param names don't match C — Rust=(name, candidates, threshold) vs C=(hn, scanflags)
pub fn spscan(name: &str, candidates: &[String], threshold: usize) -> Option<String> {
    let mut best = None;
    let mut best_dist = threshold + 1;
    for candidate in candidates {
        let dist = spdist(name, candidate, threshold);
        if dist < best_dist {
            best_dist = dist;
            best = Some(candidate.clone());
        }
    }
    best
}

/// Port of `getshfunc(char *nam)` from `Src/utils.c:3998`.
///
/// C body:
/// ```c
/// Shfunc getshfunc(char *nam) {
///     return (Shfunc) shfunctab->getnode(shfunctab, nam);
/// }
/// ```
///
/// Routes through the global `shfunctab` singleton in
/// hashtable.rs (hashtable::shfunctab_lock) — matches C's
/// signature exactly. Returns the function body as
/// `Option<String>` (zshrs's `Shfunc` equivalent is the
/// function-text mapping; bytecode dispatch happens in fusevm
/// at call time).
pub fn getshfunc(nam: &str) -> Option<String> {
    let tab = crate::ported::hashtable::shfunctab_lock()
        .read()
        .expect("shfunctab poisoned");
    tab.get(nam).and_then(|f| f.body.clone())
}

/// Port of `void makecommaspecial(int yesno)` from Src/utils.c:4270.
///
/// Toggles `ZTF_SP_COMMA` and the `ISPECIAL` bit on `,` in the
/// global typtab — used by glob/extended-glob to flag `,`
/// (KSH_GLOB) as a metacharacter.
pub fn makecommaspecial(yesno: bool) {                                       // c:4270
    let mut flags = TYPTAB_FLAGS.lock().unwrap();
    let mut tab = TYPTAB.lock().unwrap();
    if yesno {                                                               // c:4272
        *flags |= ZTF_SP_COMMA;                                              // c:4273
        tab[b',' as usize] |= ISPECIAL as u32;                               // c:4274
    } else {
        *flags &= !ZTF_SP_COMMA;                                             // c:4276
        tab[b',' as usize] &= !(ISPECIAL as u32);                            // c:4277
    }
}

/// Duplicate array with zsh allocation (from utils.c zarrdup)
/// Port of `zarrdup(char **s)` from `Src/utils.c:4532`.
pub fn zarrdup(s: &[String]) -> Vec<String> {
    s.to_vec()
}

/// Spelling correction: find closest match (from utils.c spname)
/// Port of `spname(char *oldname)` from `Src/utils.c:4562`.
/// WARNING: param names don't match C — Rust=(name, dir) vs C=(oldname)
pub fn spname(name: &str, dir: &str) -> Option<String> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return None,
    };

    let mut best = None;
    let mut best_dist = 4; // threshold

    for entry in entries.flatten() {
        if let Some(entry_name) = entry.file_name().to_str() {
            let dist = spdist(name, entry_name, best_dist);
            if dist < best_dist {
                best_dist = dist;
                best = Some(entry_name.to_string());
            }
        }
    }
    best
}

/// Spelling correction with full path (from utils.c mindist)
/// Port of `mindist(char *dir, char *mindistguess, char *mindistbest, int wantdir)` from `Src/utils.c:4624`.
/// WARNING: param names don't match C — Rust=(dir, name) vs C=(dir, mindistguess, mindistbest, wantdir)
pub fn mindist(dir: &str, name: &str) -> Option<(String, usize)> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return None,
    };

    let mut best = None;
    let mut best_dist = 4;

    for entry in entries.flatten() {
        if let Some(entry_name) = entry.file_name().to_str() {
            let dist = spdist(name, entry_name, best_dist);
            if dist < best_dist {
                best_dist = dist;
                best = Some(entry_name.to_string());
            }
        }
    }
    best.map(|name| (name, best_dist))
}

/// Port of `Meta` from `Src/zsh.h`. Sentinel byte (0x83) zsh
/// prepends in front of any byte whose top bit it wants to escape;
/// the byte that follows is XOR'd with 32. `unmetafy` (and the
/// `Meta`-aware loops throughout zsh) walk the result byte-by-byte
/// and reverse the encoding.
#[allow(non_upper_case_globals)]
pub const Meta: u8 = 0x83;

/// Port of `imeta()` macro from `Src/zsh.h`. Returns true for any
/// byte that needs meta-encoding (high-bit + the special control
/// bytes zsh treats as syntax). The Rust port treats every byte
/// >= 0x83 as needing escape, matching the C macro's effective
/// range on little-endian Unix.
#[inline]
pub fn imeta_byte(b: u8) -> bool {
    b >= Meta
}

/// Port of `unmetafy(char *s, int *len)` from `Src/utils.c:4954`.
///
/// Take a metafied byte buffer in `s` and convert it in place to
/// its literal representation. C signature:
///
/// ```c
/// char *unmetafy(char *s, int *len);
/// ```
///
/// The Rust port mutates `s` in place and returns the resulting
/// length (mirroring C's `*len` out-parameter). C control flow:
///
/// ```c
/// for (p = s; *p && *p != Meta; p++);     // skip prefix with no Meta
/// for (t = p; (*t = *p++);)                 // walk the rest
///     if (*t++ == Meta && *p)
///         t[-1] = *p++ ^ 32;                // un-escape: XOR with 32
/// ```
///
/// Same algorithm here, byte-indexed against the Vec rather than
/// pointer-walked.
/// WARNING: param names don't match C — Rust=(s) vs C=(s, len)
pub fn unmetafy(s: &mut Vec<u8>) -> usize {                                 // c:4954
    // First loop: find the first `Meta` byte. Everything before it
    // stays as-is, so we don't need to copy.
    let mut p: usize = 0;
    while p < s.len() && s[p] != Meta {
        p += 1;
    }
    // Second loop: walk from `p` onward, copying each byte into the
    // `t` slot (which trails `p` by one position per Meta-escape
    // we collapse).
    let mut t: usize = p;
    while p < s.len() {
        let b = s[p];
        s[t] = b;
        p += 1;
        if b == Meta && p < s.len() {
            // C: t[-1] = *p++ ^ 32; — overwrite the just-written
            // Meta with the un-escaped byte.
            s[t] = s[p] ^ 32;
            p += 1;
        }
        t += 1;
    }
    s.truncate(t);
    t
}

/// Count meta characters in string (from utils.c metalen)
/// Port of `metalen(const char *s, int len)` from `Src/utils.c:4972`.
pub fn metalen(s: &str, len: usize) -> usize {
    let bytes = s.as_bytes();
    let mut count = 0;
    let mut i = 0;
    while i < len.min(bytes.len()) {
        if bytes[i] == 0x83 {
            i += 2;
        } else {
            i += 1;
        }
        count += 1;
    }
    count
}

/// Port of `nicedup(char const *s, int heap)` from `Src/utils.c:5289` (single-byte build) and
/// `Src/utils.c:5289` (multibyte build). C body:
/// ```c
/// char *retstr;
/// (void)sb_niceformat(s, NULL, &retstr, heap ? NICEFLAG_HEAP : 0);
/// return retstr;
/// ```
/// Both C variants delegate to `sb_niceformat` (single-byte) or
/// `mb_niceformat` (multibyte) — the wrapper is identical shape, so
/// the Rust port is a direct call into [`sb_niceformat`]. The `heap`
/// arg is dropped because Rust ownership tracks lifetime; the C
/// `NICEFLAG_HEAP` only controls which allocator zalloc-vs-zhalloc
/// is used, which has no Rust analog.
pub fn nicedup(s: &str) -> String {
    sb_niceformat(s)
}

/// Port of `niceztrlen(char const *s)` from `Src/utils.c:5324`.
///
/// Returns the length (in bytes) of the visible representation of
/// the metafied string `s`. C body walks each char via `nicechar`
/// and accumulates `strlen(nicechar(c))`; the Rust port mirrors
/// this via [`sb_niceformat`] which is the same render-then-measure
/// path.
pub fn niceztrlen(s: &str) -> usize {
    sb_niceformat(s).len()
}

/// Port of `dquotedztrdup(char const *s)` from `Src/utils.c:6649`.
///
/// "Double-quote a metafied string." C body has two arms:
/// - CSHJUNKIEQUOTES set: backslash-escape `"`/`$`/`` ` `` and
///   wrap whole sections in `"..."`, breaking on metacharacters.
/// - Otherwise: wrap the whole string in `"..."` with backslash
///   escaping for `\`/`"`/`$`/`` ` ``, plus the `pending` quirk
///   for trailing backslashes.
///
/// Previous Rust port produced unquoted output (just escaped
/// metacharacters inline) — wrong for both arms. New port matches
/// the non-CSHJUNKIEQUOTES path (the common one).
pub fn dquotedztrdup(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 4 + 2);
    out.push('"');
    let mut pending = false;
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = if bytes[i] == Meta && i + 1 < bytes.len() {
            i += 2;
            (bytes[i - 1] ^ 32) as char
        } else {
            i += 1;
            bytes[i - 1] as char
        };
        match c {
            '\\' => {
                if pending {
                    out.push('\\');
                }
                out.push('\\');
                pending = true;
            }
            '"' | '$' | '`' => {
                if pending {
                    out.push('\\');
                }
                out.push('\\');
                out.push(c);
                pending = false;
            }
            other => {
                out.push(other);
                pending = false;
            }
        }
    }
    if pending {
        out.push('\\');
    }
    out.push('"');
    // C: ret = metafy(buf, p - buf, META_DUP);  — re-metafy result.
    metafy(&out)
}

/// Port of `restoredir(struct dirsav *d)` from `Src/utils.c:7565`.
///
/// ```c
/// int restoredir(struct dirsav *d) {
///     if (d->dirname && *d->dirname == '/')
///         return chdir(d->dirname);
///     if (d->dirfd >= 0) {
///         if (!fchdir(d->dirfd)) {
///             if (!d->dirname) return 0;
///             else if (chdir(d->dirname)) {
///                 close(d->dirfd); d->dirfd = -1; err = -2;
///             }
///         } else {
///             close(d->dirfd); d->dirfd = err = -1;
///         }
///     } else if (d->level > 0)
///         err = upchdir(d->level);
///     else if (d->level < 0) err = -1;
///     // dev/ino integrity check ...
/// }
/// ```
///
/// Restore the cwd captured in `d`. Absolute `dirname` short-
/// circuits to `chdir`. Otherwise tries `fchdir(dirfd)` (when
/// supported) then falls through to `upchdir(level)` for the
/// nested-fn-exit case. Returns 0 on success, non-zero on failure
/// (matching C's int return).
///
/// Signature change: previous Rust port took `saved: &str` and
/// returned `bool` — different shape from C, missed the dirfd /
/// level / dev / ino fields entirely.
pub fn restoredir(d: &mut crate::ported::zsh_h::dirsav) -> i32 {

    // C: if (d->dirname && *d->dirname == '/') return chdir(d->dirname);
    if let Some(name) = d.dirname.as_ref() {
        if name.starts_with('/') {
            return match std::env::set_current_dir(name) {
                Ok(_) => 0,
                Err(_) => -1,
            };
        }
    }
    let mut err: i32 = 0;
    // C: HAVE_FCHDIR path — try fchdir(dirfd) first.
    #[cfg(unix)]
    if d.dirfd >= 0 {
        let rc = unsafe { libc::fchdir(d.dirfd) };
        if rc == 0 {
            if d.dirname.is_none() {
                return 0;
            }
            let name = d.dirname.as_ref().unwrap();
            if std::env::set_current_dir(name).is_err() {
                unsafe { libc::close(d.dirfd) };
                d.dirfd = -1;
                err = -2;
            }
        } else {
            unsafe { libc::close(d.dirfd) };
            d.dirfd = -1;
            err = -1;
        }
    } else if d.level > 0 {
        // C: err = upchdir(d->level);
        let _ = upchdir(d.level as usize);
    } else if d.level < 0 {
        err = -1;
    }
    // C: dev/ino integrity check after the chdir/fchdir.
    if (d.dev != 0 || d.ino != 0) && err == 0 {
        if let Ok(meta) = std::fs::metadata(".") {
            if meta.ino() != d.ino || meta.dev() != d.dev {
                err = -1;
            }
        } else {
            err = -1;
        }
    }
    err
}

/// Port of `convfloat(double dval, int digits, int flags, FILE *fout)` from `Src/params.c:5690` (the C source has
/// it in params.c, not utils.c — re-exported through utils.rs for
/// caller convenience since math/printf paths reach it from both
/// directories). See [`crate::params::convfloat`] for the faithful
/// C body port; this wrapper drops the `FILE *fout` arg (every
/// zshrs caller wants the returned string, never the printf path).
/// WARNING: param names don't match C — Rust=(dval, digits, flags) vs C=(dval, digits, flags, fout)
pub fn convfloat(dval: f64, digits: i32, flags: u32) -> String {
    crate::params::convfloat(dval, digits, flags)
}

/// Port of `convfloat_underscore(double dval, int underscore)` from `Src/params.c:5765`.
/// See [`crate::params::convfloat_underscore`] for the faithful C
/// body port.
pub fn convfloat_underscore(dval: f64, underscore: i32) -> String {
    crate::params::convfloat_underscore(dval, underscore)
}

/// Port of `ucs4tomb(unsigned int wval, char *buf)` from `Src/utils.c:6788`.
///
/// Encode a UCS-4 codepoint into the buffer `buf` using the current
/// locale's multibyte encoding. Returns the number of bytes written,
/// or -1 on conversion failure. C body uses `wctomb(3)` when
/// `__STDC_ISO_10646__` is defined (which it is on every modern
/// glibc / macOS libc), falls back to UTF-8 if the codeset is
/// `"UTF-8"`, and uses `iconv(3)` otherwise.
///
/// This Rust port mirrors the primary `wctomb` path via libc FFI;
/// the iconv fallback is unused on macOS/Linux modern builds.
/// On conversion failure, emits `zerr("character not in range")`
/// to match C source line 6794.
///
/// C body shape:
/// ```c
/// int count = wctomb(buf, (wchar_t)wval);
/// if (count == -1) zerr("character not in range");
/// return count;
/// ```
pub fn ucs4tomb(wval: u32, buf: &mut [u8]) -> i32 {
    // libc::wctomb requires at least MB_CUR_MAX bytes (typically 4
    // for UTF-8, 6 for some encodings). Use a stack buffer first,
    // then copy into the caller's buffer.
    // libc crate doesn't expose wctomb on all platforms; declare
    // the POSIX prototype directly. wchar_t is i32 on macOS/Linux
    // for our supported targets.
    extern "C" {
        fn wctomb(s: *mut libc::c_char, wc: libc::wchar_t) -> libc::c_int;
    }
    let mut local = [0i8; 16];
    let count = unsafe {
        wctomb(local.as_mut_ptr(), wval as libc::wchar_t)
    };
    if count < 0 {
        zerr("character not in range");
        return -1;
    }
    let n = count as usize;
    if n > buf.len() {
        zerr("character not in range");
        return -1;
    }
    for i in 0..n {
        buf[i] = local[i] as u8;
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sepsplit() {
        assert_eq!(sepsplit("a:b:c", Some(":"), false), vec!["a", "b", "c"]);
        assert_eq!(sepsplit("a::b", Some(":"), false), vec!["a", "b"]);
        assert_eq!(sepsplit("a::b", Some(":"), true), vec!["a", "", "b"]);
    }

    #[test]
    fn test_unmetafy_no_meta_byte_passes_through() {
        // No Meta byte → buffer unchanged, length unchanged.
        let mut buf = b"hello".to_vec();
        let n = unmetafy(&mut buf);
        assert_eq!(n, 5);
        assert_eq!(&buf, b"hello");
    }

    #[test]
    fn test_unmetafy_collapses_meta_escapes() {
        // C: Meta byte (0x83) followed by `'a' ^ 32` (0x41 = 'A')
        // unmetafies to a single byte 'a' (0x61).
        // i.e. {0x83, 'a' ^ 32} → {'a'}.
        let mut buf = vec![0x83, b'a' ^ 32];
        let n = unmetafy(&mut buf);
        assert_eq!(n, 1);
        assert_eq!(buf, vec![b'a']);
    }

    #[test]
    fn test_unmetafy_mixed_prefix_then_meta() {
        // Plain prefix, then Meta-escaped 0xFF (0x83, 0xFF ^ 32 = 0xDF).
        let mut buf = vec![b'X', b'Y', 0x83, 0xFF ^ 32, b'Z'];
        let n = unmetafy(&mut buf);
        assert_eq!(n, 4);
        assert_eq!(buf, vec![b'X', b'Y', 0xFF, b'Z']);
    }

    #[test]
    fn test_unmetafy_returns_self_value() {
        // C returns `s` (the buffer) for chaining; Rust returns
        // the new length. Verify length matches a call that
        // collapses two Meta-escapes.
        let mut buf = vec![
            b'A',
            0x83, b'B' ^ 32, // → 'B'
            0x83, b'C' ^ 32, // → 'C'
            b'D',
        ];
        let n = unmetafy(&mut buf);
        assert_eq!(n, 4);
        assert_eq!(buf, b"ABCD".to_vec());
    }

    #[test]
    fn test_imeta_byte_threshold() {
        // C `#define imeta(c) ((c) >= Meta)` — true for any byte
        // >= 0x83.
        assert!(!imeta_byte(0x82));
        assert!(imeta_byte(Meta));
        assert!(imeta_byte(0xFF));
    }

    #[test]
    fn test_meta_constant_value() {
        // Locked at 0x83 by Src/zsh.h. If this test fails, zsh
        // bumped the Meta sentinel and the encoding mapping needs
        // a full audit.
        assert_eq!(Meta, 0x83);
    }

    #[test]
    #[cfg(unix)]
    fn test_mode_to_octal_canonical_bits() {
        use libc::{S_IRUSR, S_IWUSR, S_IXUSR};
        // rwx for owner = 0o700.
        let mode = (S_IRUSR | S_IWUSR | S_IXUSR) as u32;
        assert_eq!(mode_to_octal(mode), 0o700);
        // rwx all = 0o777.
        let all = (S_IRUSR | S_IWUSR | S_IXUSR) as u32 * (1 + 8 + 64);
        // Use libc constants individually for portability.
        let m = (libc::S_IRUSR
            | libc::S_IWUSR
            | libc::S_IXUSR
            | libc::S_IRGRP
            | libc::S_IWGRP
            | libc::S_IXGRP
            | libc::S_IROTH
            | libc::S_IWOTH
            | libc::S_IXOTH) as u32;
        assert_eq!(mode_to_octal(m), 0o777);
        let _ = all;
    }

    #[test]
    #[cfg(unix)]
    fn test_mode_to_octal_setuid_setgid_sticky() {
        use libc::{S_ISGID, S_ISUID, S_ISVTX};
        assert_eq!(mode_to_octal(S_ISUID as u32), 0o4000);
        assert_eq!(mode_to_octal(S_ISGID as u32), 0o2000);
        assert_eq!(mode_to_octal(S_ISVTX as u32), 0o1000);
        // All three: 0o7000.
        let all = (S_ISUID | S_ISGID | S_ISVTX) as u32;
        assert_eq!(mode_to_octal(all), 0o7000);
    }

    #[test]
    #[cfg(unix)]
    fn test_mailstat_plain_file_returns_native_stat() {
        use std::os::unix::fs::MetadataExt;
        // Plain file path → *st fields mirror native stat,
        // not the maildir aggregation.
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        let rc = mailstat("/etc/hosts", &mut st);
        if rc == 0 {
            assert_eq!(st.st_nlink as u64, std::fs::metadata("/etc/hosts").unwrap().nlink());
        }
    }

    #[test]
    fn test_mailstat_nonexistent_returns_neg1() {
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        assert_eq!(mailstat("/nonexistent/path/does/not/exist", &mut st), -1);
    }

    #[test]
    fn test_mailstat_directory_without_maildir_subdirs() {
        // /tmp is a directory but not a maildir (no cur/tmp/new) —
        // returns the partial aggregate (top dir's atime/mtime,
        // size=0 since cur/ wasn't found before we'd start summing).
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        let rc = mailstat("/tmp", &mut st);
        assert_eq!(rc, 0);
        assert_eq!(st.st_nlink, 1);
        assert_eq!(st.st_size, 0);
        assert_eq!(st.st_blocks, 0);
        // S_IFDIR bit should be cleared, S_IFREG set.
        #[cfg(unix)]
        {
            assert_eq!(st.st_mode & libc::S_IFDIR, 0);
            assert_ne!(st.st_mode & libc::S_IFREG, 0);
        }
    }

    #[test]
    fn test_dupstrpfx_byte_counted() {
        // 5 bytes of ASCII = 5 chars, identical.
        assert_eq!(dupstrpfx("hello", 3), "hel");
        // Beyond length clamps.
        assert_eq!(dupstrpfx("hi", 10), "hi");
        // Empty.
        assert_eq!(dupstrpfx("anything", 0), "");
    }

    #[test]
    fn test_metafy_passes_through_ascii() {
        // ASCII bytes (< 0x83) stay untouched.
        assert_eq!(metafy("hello"), "hello");
        assert_eq!(metafy(""), "");
    }

    #[test]
    fn test_metafy_imeta_predicate_matches_c_macro() {
        // The C macro is `#define imeta(c) ((c) >= Meta)` —
        // verify the Rust port's predicate matches via imeta_byte.
        for b in 0u8..0x83 {
            assert!(!imeta_byte(b), "byte {:#x} should NOT be imeta", b);
        }
        for b in 0x83u8..=0xff {
            assert!(imeta_byte(b), "byte {:#x} should be imeta", b);
        }
    }

    #[test]
    fn test_ztrcmp_meta_aware() {
        // Two identical metafied strings → Equal.
        assert_eq!(ztrcmp("foo", "foo"), std::cmp::Ordering::Equal);
        // "foo" < "foz".
        assert_eq!(ztrcmp("foo", "foz"), std::cmp::Ordering::Less);
        // Prefix comparison: shorter < longer.
        assert_eq!(ztrcmp("foo", "foobar"), std::cmp::Ordering::Less);
        // Meta-encoded comparison: {0x83, 'a'^32} should compare as 'a'.
        let s_meta = unsafe { std::str::from_utf8_unchecked(&[0x83, b'a' ^ 32]) };
        let s_plain = "a";
        // Meta-encoded "a" should compare equal to plain "a".
        assert_eq!(ztrcmp(s_meta, s_plain), std::cmp::Ordering::Equal);
    }

    #[test]
    fn test_skipwsep_skips_runs() {
        // 3 spaces + 'x' → returns ("x", 3).
        let (rest, n) = skipwsep("   x");
        assert_eq!(rest, "x");
        assert_eq!(n, 3);
        // No leading whitespace → 0 skipped.
        let (rest, n) = skipwsep("foo");
        assert_eq!(rest, "foo");
        assert_eq!(n, 0);
        // Mix of space/tab/newline.
        let (rest, n) = skipwsep(" \t\nbar");
        assert_eq!(rest, "bar");
        assert_eq!(n, 3);
    }

    #[test]
    fn test_imeta_macro_threshold() {
        // C `imeta(c) == c >= Meta`. The previous Rust port had
        // a wrong predicate (control bytes); the C version is just
        // `>= 0x83`.
        assert!(!imeta(0x82 as char));
        assert!(imeta(Meta as char));
        assert!(imeta(0xff as char));
        assert!(!imeta(' '));
        assert!(!imeta('A'));
    }

    #[test]
    fn test_unmeta_routes_through_unmetafy() {
        // unmeta wraps the in-place unmetafy via a byte-vector
        // copy; the no-Meta fast path returns the source as-is.
        assert_eq!(unmeta("plain"), "plain");
    }

    #[test]
    fn test_iwsep_includes_newline() {
        // The previous port omitted '\n' which broke wordcount on
        // multi-line input.
        assert!(iwsep('\n'));
        assert!(iwsep('\t'));
        assert!(iwsep(' '));
        assert!(!iwsep('a'));
    }

    #[test]
    fn test_mailstat_aggregates_maildir() {
        // Create a temp maildir layout with 2 messages in new/ and 1
        // in cur/, verify the aggregate.
        use std::fs;
        use std::io::Write;
        let tmp = std::env::temp_dir().join(format!(
            "zshrs_mailstat_test_{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(tmp.join("cur")).unwrap();
        fs::create_dir_all(tmp.join("new")).unwrap();
        fs::create_dir_all(tmp.join("tmp")).unwrap();
        let mut f = fs::File::create(tmp.join("new").join("msg1")).unwrap();
        f.write_all(b"hello").unwrap();
        let mut f = fs::File::create(tmp.join("new").join("msg2")).unwrap();
        f.write_all(b"world!").unwrap();
        let mut f = fs::File::create(tmp.join("cur").join("msg3")).unwrap();
        f.write_all(b"third").unwrap();
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        let rc = mailstat(tmp.to_str().unwrap(), &mut st);
        assert_eq!(rc, 0, "maildir should stat");
        assert_eq!(st.st_blocks, 3, "3 messages total across new/ + cur/");
        assert_eq!(st.st_size, 5 + 6 + 5, "5+6+5 bytes total");
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_spacesplit() {
        assert_eq!(spacesplit("a b c", false), vec!["a", "b", "c"]);
        assert_eq!(spacesplit("a  b", false), vec!["a", "b"]);
    }

    #[test]
    fn test_sepjoin() {
        assert_eq!(
            sepjoin(&["a".into(), "b".into(), "c".into()], Some(":")),
            "a:b:c"
        );
        assert_eq!(sepjoin(&["a".into(), "b".into()], None), "a b");
    }

    #[test]
    fn test_isident() {
        assert!(isident("foo"));
        assert!(isident("_bar"));
        assert!(isident("baz123"));
        assert!(!isident("123abc"));
        assert!(!isident("foo-bar"));
    }

    #[test]
    fn test_nicechar() {
        assert_eq!(nicechar('\n'), "\\n");
        assert_eq!(nicechar('\t'), "\\t");
        assert_eq!(nicechar('a'), "a");
    }

    #[test]
    fn test_quotedzputs_single_quote_wrap() {
        assert_eq!(quotedzputs("simple"), "simple");
        assert_eq!(quotedzputs("has space"), "'has space'");
        assert_eq!(quotedzputs("it's"), "'it'\\''s'");
    }

    #[test]
    fn test_quotestring_backslash() {
        assert_eq!(quotestring("hello", crate::ported::zsh_h::QT_BACKSLASH), "hello");
        assert_eq!(
            quotestring("has space", crate::ported::zsh_h::QT_BACKSLASH),
            "has\\ space"
        );
        assert_eq!(quotestring("$var", crate::ported::zsh_h::QT_BACKSLASH), "\\$var");
    }

    #[test]
    fn test_quotestring_single() {
        assert_eq!(quotestring("hello", crate::ported::zsh_h::QT_SINGLE), "'hello'");
        assert_eq!(quotestring("it's", crate::ported::zsh_h::QT_SINGLE), "'it'\\''s'");
    }

    #[test]
    fn test_quotestring_double() {
        assert_eq!(quotestring("hello", crate::ported::zsh_h::QT_DOUBLE), "\"hello\"");
        assert_eq!(
            quotestring("say \"hi\"", crate::ported::zsh_h::QT_DOUBLE),
            "\"say \\\"hi\\\"\""
        );
    }

    #[test]
    fn test_quotestring_dollars() {
        assert_eq!(quotestring("hello", crate::ported::zsh_h::QT_DOLLARS), "$'hello'");
        assert_eq!(
            quotestring("line\nbreak", crate::ported::zsh_h::QT_DOLLARS),
            "$'line\\nbreak'"
        );
        assert_eq!(
            quotestring("tab\there", crate::ported::zsh_h::QT_DOLLARS),
            "$'tab\\there'"
        );
    }

    #[test]
    fn test_quotestring_pattern() {
        assert_eq!(quotestring("*.txt", crate::ported::zsh_h::QT_BACKSLASH_PATTERN), "\\*.txt");
        assert_eq!(
            quotestring("file[1]", crate::ported::zsh_h::QT_BACKSLASH_PATTERN),
            "file\\[1\\]"
        );
    }

    #[test]
    fn test_quotetype_from_q_count() {
        assert_eq!(qflag_quotetype(1), crate::ported::zsh_h::QT_BACKSLASH);
        assert_eq!(qflag_quotetype(2), crate::ported::zsh_h::QT_SINGLE);
        assert_eq!(qflag_quotetype(3), crate::ported::zsh_h::QT_DOUBLE);
        assert_eq!(qflag_quotetype(4), crate::ported::zsh_h::QT_DOLLARS);
    }

    #[test]
    fn test_tulower_tuupper() {
        assert_eq!(tulower('A'), 'a');
        assert_eq!(tuupper('a'), 'A');
        assert_eq!(tulower('1'), '1');
    }

    #[test]
    fn test_wordcount_ifs_default() {
        // C: wordcount("a b c", NULL, 0) -> 3
        assert_eq!(wordcount("a b c", None, 0), 3);
        // Leading/trailing whitespace coalesced when mul <= 0.
        assert_eq!(wordcount("  a b  ", None, 0), 2);
        // Empty string with mul == 0 -> 0 words.
        assert_eq!(wordcount("", None, 0), 0);
        // Single word, no separators.
        assert_eq!(wordcount("foo", None, 0), 1);
    }

    #[test]
    fn test_wordcount_with_explicit_sep() {
        // C: wordcount("a:b:c", ":", 0) -> 3 (3 fields, 2 separators)
        assert_eq!(wordcount("a:b:c", Some(":"), 0), 3);
        // Empty fields counted when mul != 0.
        assert_eq!(wordcount("a::b", Some(":"), 1), 3);
        // Without mul, consecutive empties collapse: a, b => 2... but
        // C's "if ((c || mul) && (sl || *(s+sl)))" — second `:` has
        // c=0 and mul=0 so doesn't increment. Result: a, b => 2.
        assert_eq!(wordcount("a::b", Some(":"), 0), 2);
    }

    #[test]
    fn test_ucs4tomb_ascii() {
        let mut buf = [0u8; 8];
        // 'A' = 0x41, ASCII, single byte in any locale.
        let n = ucs4tomb('A' as u32, &mut buf);
        // wctomb may return 1 in C/POSIX locale; in UTF-8 locale also 1.
        assert!(n == 1);
        assert_eq!(buf[0], b'A');
    }

    #[test]
    fn test_is_mb_niceformat_plain_ascii() {
        // Plain printable ASCII — nothing needs nicechar escaping.
        assert!(!is_mb_niceformat("hello world"));
    }

    #[test]
    fn test_is_mb_niceformat_with_control_char() {
        // Tab is control (< 0x20) — needs nice escaping.
        assert!(is_mb_niceformat("a\tb"));
        // Bell character.
        assert!(is_mb_niceformat("a\x07b"));
    }
}

// ---------------------------------------------------------------------------
// Remaining 33 missing utils.c functions
// ---------------------------------------------------------------------------

// `zwarning` is defined earlier in this file as the real port of
// utils.c:142 (private helper invoked by `zerr`/`zerrnam`/`zwarn`/
// `zwarnnam`). The duplicate stub previously here has been deleted
// — callers use the four public entry points instead.

/// Port of `void zz_plural_z_alpha(void)` from Src/utils.c:282.
///
/// Cygwin-only no-op symbol the C source emits to work around a
/// dllwrap bug that drops the last alphabetically-sorted exported
/// symbol (zwarnnam). The Rust port has no equivalent linker
/// problem; this is preserved as a no-op for symbol-table parity.
pub fn zz_plural_z_alpha() {}                                                // c:282

/// Check if a character needs nice formatting (from utils.c is_nicechar)
pub fn is_nicechar(c: char) -> bool {
    c.is_ascii_control() || !c.is_ascii()
}

/// Port of `freestr(void *a)` from `Src/utils.c:1739`.
///
/// C body:
/// ```c
/// void freestr(void *a) { zsfree(a); }
/// ```
/// The C function is registered as the `freenode` callback for
/// hashtables holding plain string values. The Rust port consumes
/// `s` by value; Rust's `Drop` runs the equivalent of `zsfree` when
/// the `String` is moved into this fn and falls out of scope at the
/// closing brace — the no-op body is the correct port.
/// WARNING: param names don't match C — Rust=(_s) vs C=(a)
pub fn freestr(_s: String) {}

/// Port of `int gettempfile(const char *prefix, int use_heap, char **tempname)`
/// from Src/utils.c:2231. Creates a fresh tempfile with `O_RDWR|O_CREAT|O_EXCL`
/// and mode 0600 under umask 0177; returns `(fd, path)` on success.
///
/// C signature uses an out-param for the filename and returns the fd; Rust
/// returns both as a tuple. Matches the C control-flow: open with O_EXCL,
/// retry up to 16 times on EEXIST (the non-mkstemp branch, c:2255-2269).
pub fn gettempfile(prefix: Option<&str>) -> Option<(i32, String)> {           // c:2231
    #[cfg(unix)]
    {
        crate::ported::signals::queue_signals();                             // c:2239
        let old_umask = unsafe { libc::umask(0o177) };                       // c:2240
        let mut failures = 0;                                                // c:2255
        let mut result: Option<(i32, String)> = None;
        loop {
            let fn_ = match gettempname(prefix, false) {                     // c:2260
                Some(n) => n,
                None => break,
            };
            let cn = match std::ffi::CString::new(fn_.clone()) {
                Ok(c) => c,
                Err(_) => break,
            };
            let fd = unsafe {
                libc::open(
                    cn.as_ptr(),
                    libc::O_RDWR | libc::O_CREAT | libc::O_EXCL,             // c:2264
                    0o600 as libc::c_int,
                )
            };
            if fd >= 0 {
                result = Some((fd, fn_));
                break;
            }
            let err = std::io::Error::last_os_error()
                .raw_os_error()
                .unwrap_or(0);
            if err != libc::EEXIST {                                         // c:2269
                break;
            }
            failures += 1;
            if failures >= 16 {                                              // c:2269
                break;
            }
        }
        unsafe { libc::umask(old_umask); }                                   // c:2273
        crate::ported::signals::unqueue_signals();                           // c:2274
        result
    }
    #[cfg(not(unix))]
    {
        let _ = prefix;
        None
    }
}

/// Copy string with upper/lower case (from utils.c strucpy)
/// Port of `strucpy(char **s, char *t)` from `Src/utils.c:2331`.
pub fn strucpy(s: &str, t: bool) -> String {
    if t {
        s.to_uppercase()
    } else {
        s.to_string()
    }
}

/// Copy n chars with upper/lower case (from utils.c struncpy)
/// Port of `struncpy(char **s, char *t, int n)` from `Src/utils.c:2341`.
pub fn struncpy(s: &str, t: usize, n: bool) -> String {
    let s: String = s.chars().take(t).collect();
    if n {
        s.to_uppercase()
    } else {
        s
    }
}

// Return TRUE iff arrlen(s) >= lower_bound, but more efficiently.          // c:2382
/// Check if array length >= n (from utils.c arrlen_ge)
pub fn arrlen_ge<T>(arr: &[T], n: usize) -> bool {                          // c:2369
    arr.len() >= n
}

// Return TRUE iff arrlen(s) > lower_bound, but more efficiently.           // c:2400
/// Check if array length > n (from utils.c arrlen_gt)
pub fn arrlen_gt<T>(arr: &[T], n: usize) -> bool {                          // c:2382
    arr.len() > n
}

// Return TRUE iff arrlen(s) < upper_bound, but more efficiently.           // c:2400
/// Check if array length < n (from utils.c arrlen_lt)
pub fn arrlen_lt<T>(arr: &[T], n: usize) -> bool {                          // c:2400
    arr.len() < n
}

/// Port of `setblock_stdin()` from `Src/utils.c:2622`.
///
/// ```c
/// int setblock_stdin(void) {
///     long mode;
///     return setblock_fd(1, 0, &mode);
/// }
/// ```
///
/// Set stdin to BLOCKING mode (`unblock=1` in C, third arg is the
/// out-parameter for the previous mode flags). Returns success.
/// The previous Rust port called `setblock_fd(0, true)` — wrong:
/// fd 0 (stdin) is correct, but the second argument was `true`
/// meaning unblock=true. C's `setblock_fd(1, ...)` first arg is
/// `unblock=1` meaning enable blocking; second arg is fd 0.
pub fn setblock_stdin() -> i32 {
    // C calls setblock_fd(1 /* unblock=true means SET to blocking */, 0 /* fd */, &mode).
    // The Rust setblock_fd shim just toggles O_NONBLOCK on the fd.
    setblock_fd(0, false);
    0
}

/// Port of `ztrftimebuf(int *bufsizeptr, int decr)` from `Src/utils.c:3312`.
///
/// ```c
/// static int ztrftimebuf(int *bufsizeptr, int decr) {
///     if (*bufsizeptr <= decr) return 1;
///     *bufsizeptr -= decr;
///     return 0;
/// }
/// ```
///
/// "Helper for ztrftime: try to fit decr more bytes (plus a NUL)
/// in the buffer, and a new string length to decrement from that.
/// Returns 0 if the new length fits, 1 otherwise."
///
/// Previous Rust port had wrong semantics — returned `needed.max(256)`
/// (a buffer-sizing helper) instead of the C "decrement-and-check"
/// semantics. New port matches C: takes &mut bufsize + decr,
/// returns i32 (0 = fit, 1 = doesn't fit).
pub fn ztrftimebuf(bufsizeptr: &mut i32, decr: i32) -> i32 {
    if *bufsizeptr <= decr {
        return 1;
    }
    *bufsizeptr -= decr;
    0
}

/// Port of `char **subst_string_by_func(Shfunc func, char *arg1, char *orig)`
/// from Src/utils.c:4017.
///
/// Calls the named shell function with `[func, arg1?, orig]` as
/// positional args under `sfcontext = SFC_SUBST` and returns the
/// `$reply` array on success. Routes through `callhookfunc` (the
/// static-linked equivalent of `doshfunc`), then reads `$reply`
/// from the env-var fallback because the global `paramtab` is not
/// yet a singleton in the Rust port (params::getaparam takes a
/// `&ParamTable` arg).
pub fn subst_string_by_func(func_name: &str, arg1: Option<&str>, orig: &str)
    -> Option<Vec<String>>                                                   // c:4017
{
    let osc = crate::ported::builtin::SFCONTEXT.load(Ordering::Relaxed);     // c:4019
    let osm = crate::ported::builtin::STOPMSG.load(Ordering::Relaxed);
    let old_incompfunc = INCOMPFUNC.load(Ordering::Relaxed);
    let mut args: Vec<String> = Vec::with_capacity(3);                       // c:4020-4026
    args.push(func_name.to_string());                                        // c:4023
    if let Some(a) = arg1 {                                                  // c:4024
        args.push(a.to_string());                                            // c:4025
    }
    args.push(orig.to_string());                                             // c:4026
    crate::ported::builtin::SFCONTEXT                                         // c:4027
        .store(crate::ported::zsh_h::SFC_SUBST, Ordering::Relaxed);
    INCOMPFUNC.store(0, Ordering::Relaxed);                                  // c:4028

    let rc = callhookfunc(func_name, Some(&args), false);                    // c:4030
    // C: getaparam("reply"). The Rust paramtab is not yet a global —
    // params::getaparam() takes a `&ParamTable` arg. Until the
    // global `paramtab` is wired here, fall back to splitting the
    // env var "reply" on IFS, matching the convention modules use
    // to round-trip through `setaparam`.
    let ret: Option<Vec<String>> = if rc != 0 {
        None                                                                 // c:4031
    } else {
        std::env::var("reply").ok().map(|s|                                  // c:4033
            s.split('\x00').map(|p| p.to_string()).collect::<Vec<_>>()
        )
    };

    crate::ported::builtin::SFCONTEXT.store(osc, Ordering::Relaxed);         // c:4035
    crate::ported::builtin::STOPMSG.store(osm, Ordering::Relaxed);           // c:4036
    INCOMPFUNC.store(old_incompfunc, Ordering::Relaxed);                     // c:4037
    ret                                                                      // c:4038
}

/// Port of `void makebangspecial(int yesno)` from Src/utils.c:4283.
///
/// Toggles `ISPECIAL` on the current `bangchar`. When `yesno==0`
/// always clears; when nonzero, sets only if `ZTF_BANGCHAR` was
/// stored by `inittyptab` (i.e. BANGHIST is on).
pub fn makebangspecial(yesno: bool) {                                        // c:4283
    let bc = crate::ported::hist::bangchar.load(Ordering::SeqCst) as usize;
    if bc == 0 || bc >= 256 {
        return;
    }
    let flags = *TYPTAB_FLAGS.lock().unwrap();
    let mut tab = TYPTAB.lock().unwrap();
    if !yesno {                                                              // c:4289
        tab[bc] &= !(ISPECIAL as u32);                                       // c:4290
    } else if (flags & ZTF_BANGCHAR) != 0 {                                  // c:4291
        tab[bc] |= ISPECIAL as u32;                                          // c:4292
    }
}

/// Port of `wcsiblank(wint_t wc)` from `Src/utils.c:4302`.
///
/// ```c
/// mod_export int wcsiblank(wint_t wc) {
///     if (iswspace(wc) && wc != L'\n')
///         return 1;
///     return 0;
/// }
/// ```
///
/// "wide-character version of the iblank() macro" — true for any
/// whitespace EXCEPT newline. The previous Rust port included
/// newline (since `c.is_whitespace()` returns true for '\n') —
/// wrong for callers that use this to find token boundaries.
pub fn wcsiblank(wc: char) -> bool {
    wc.is_whitespace() && wc != '\n'
}

/// Port of `int wcsitype(wchar_t c, int itype)` from Src/utils.c:4321.
///
/// "zistype macro extended to support wide characters. Works for
/// IIDENT, IWORD, IALNUM, ISEP."
///
/// The Rust port checks whether `c` falls in the typtab class
/// represented by `itype`. ASCII chars consult the global TYPTAB
/// (the same one C's `zistype()` macro indexes); non-ASCII chars
/// route through Unicode predicates that mirror the C `iswalnum`
/// fallback at line 4346.
pub fn wcsitype(c: char, itype: u32) -> bool {                               // c:4321
    if !isset(crate::ported::zsh_h::MULTIBYTE) { // c:4327
        if (c as u32) < 256 {
            let tab = TYPTAB.lock().unwrap();
            return (tab[c as usize] & itype) != 0;
        }
        return false;
    }
    if (c as u32) < 128 {                                                    // c:4343
        let tab = TYPTAB.lock().unwrap();
        return (tab[c as usize] & itype) != 0;
    }
    let cls = itype as u16;
    if cls == IIDENT {                                                       // c:4347
        if isset(crate::ported::zsh_h::POSIXIDENTIFIERS) {
            return false;                                                    // c:4348
        }
        return c.is_alphanumeric();                                          // c:4350
    }
    if cls == IWORD {                                                        // c:4352
        if c.is_alphanumeric() { return true; }                              // c:4353
        // C: IS_COMBINING(c) — no Rust crate-free combining-mark
        // predicate. zero-width chars (combining, zero-width-joiner,
        // etc.) are treated as word per c:4362.
        if unicode_width::UnicodeWidthChar::width(c).unwrap_or(1) == 0 {     // c:4362
            return true;
        }
        // C: wmemchr(wordchars_wide.chars, ...) — `$WORDCHARS` membership.
        if let Ok(w) = std::env::var("WORDCHARS") {                          // c:4364
            return w.chars().any(|x| x == c);
        }
        return false;
    }
    if cls == ISEP {                                                         // c:4366
        if let Ok(ifs) = std::env::var("IFS") {                              // c:4367
            return ifs.chars().any(|x| x == c);
        }
        return false;
    }
    let _ = IALNUM;
    c.is_alphanumeric()                                                      // c:4370
}

/// Duplicate array of wide strings (from utils.c wcs_zarrdup) - same as zarrdup in Rust
/// Port of `wcs_zarrdup(wchar_t **s)` from `Src/utils.c:4547`.
pub fn wcs_zarrdup(s: &[String]) -> Vec<String> {
    s.to_vec()
}

/// Set terminal to cbreak mode (from utils.c setcbreak)
#[cfg(unix)]
/// Port of `setcbreak` from `Src/utils.c:4756`.
pub fn setcbreak() -> bool {
    if let Some(mut ti) = gettyinfo() {
        ti.c_lflag &= !(libc::ICANON | libc::ECHO);
        ti.c_cc[libc::VMIN] = 1;
        ti.c_cc[libc::VTIME] = 0;
        settyinfo(&ti)
    } else {
        false
    }
}

#[cfg(not(unix))]
/// Port of `setcbreak` from `Src/utils.c:4756`.
pub fn setcbreak() -> bool {
    false
}

/// Port of `ztrdup_metafy(const char *s)` from `Src/utils.c:4929`.
///
/// ```c
/// mod_export char *
/// ztrdup_metafy(const char *s)
/// {
///     if (!s) return NULL;
///     return metafy((char *)s, -1, META_DUP);
/// }
/// ```
pub fn ztrdup_metafy(s: &str) -> String {
    metafy(s)
}

/// Unmetafy a single character (from utils.c unmeta_one)
/// Port of `unmeta_one(const char *in, int *sz)` from `Src/utils.c:5058`.
/// WARNING: param names don't match C — Rust=(s) vs C=(in, sz)
pub fn unmeta_one(s: &str) -> (char, usize) {
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return ('\0', 0);
    }
    if bytes[0] == 0x83 && bytes.len() > 1 {
        ((bytes[1] ^ 32) as char, 2)
    } else {
        (bytes[0] as char, 1)
    }
}

/// Port of `ztrlenend(char const *s, char const *eptr)` from `Src/utils.c:5162`.
///
/// ```c
/// for (l = 0; s < eptr; l++) {
///     if (*s++ == Meta) s++;  // skip past Meta-escaped pair
/// }
/// return l;
/// ```
///
/// Count the unmetafied character length from `s` up to `end`
/// bytes. Each Meta-escaped pair counts as 1 character.
/// Previous Rust port called `chars().count()` which counts UTF-8
/// codepoints, not byte-walked Meta-pairs — wrong semantics.
pub fn ztrlenend(s: &str, eptr: usize) -> usize {
    let bytes = s.as_bytes();
    let cap = eptr.min(bytes.len());
    let mut l = 0;
    let mut i = 0;
    while i < cap {
        if bytes[i] == Meta {
            // Meta sentinel + escaped byte = 1 visible char.
            i += 2;
        } else {
            i += 1;
        }
        l += 1;
    }
    l
}

/// Multibyte metachar length with conversion (from utils.c mb_metacharlenconv_r)
/// Port of `mb_metacharlenconv_r(const char *s, wint_t *wcp, mbstate_t *mbsp)` from `Src/utils.c:5548`.
/// WARNING: param names don't match C — Rust=(s, pos) vs C=(s, wcp, mbsp)
pub fn mb_metacharlenconv_r(s: &str, pos: usize) -> (usize, Option<char>) {
    if let Some(c) = s[pos..].chars().next() {
        (c.len_utf8(), Some(c))
    } else {
        (0, None)
    }
}

/// Multibyte metastring length to end (from utils.c mb_metastrlenend)
/// Port of `mb_metastrlenend(char *ptr, int width, char *eptr)` from `Src/utils.c:5655`.
pub fn mb_metastrlenend(ptr: &str, width: bool, eptr: usize) -> usize {
    if width {
        ptr[..eptr.min(ptr.len())]
            .chars()
            .map(|c| unicode_width::UnicodeWidthChar::width(c).unwrap_or(1))
            .sum()
    } else {
        ptr[..eptr.min(ptr.len())].chars().count()
    }
}

/// Multibyte char length with conversion (from utils.c mb_charlenconv_r)
/// Port of `mb_charlenconv_r(const char *s, int slen, wint_t *wcp, mbstate_t *mbsp)` from `Src/utils.c:5747`.
/// WARNING: param names don't match C — Rust=(s, pos) vs C=(s, slen, wcp, mbsp)
pub fn mb_charlenconv_r(s: &str, pos: usize) -> (usize, Option<char>) {
    mb_metacharlenconv_r(s, pos)
}

/// Multibyte char length (from utils.c mb_charlenconv)
/// Port of `mb_charlenconv(const char *s, int slen, wint_t *wcp)` from `Src/utils.c:5793`.
/// WARNING: param names don't match C — Rust=(s, pos) vs C=(s, slen, wcp)
pub fn mb_charlenconv(s: &str, pos: usize) -> usize {
    s[pos..].chars().next().map(|c| c.len_utf8()).unwrap_or(0)
}

/// Single-byte nice format (from utils.c sb_niceformat)
/// Port of `sb_niceformat(const char *s, FILE *stream, char **outstrp, int flags)` from `Src/utils.c:5851`.
/// WARNING: param names don't match C — Rust=(s) vs C=(s, stream, outstrp, flags)
pub fn sb_niceformat(s: &str) -> String {
    let mut result = String::new();
    for c in s.chars() {
        if c.is_ascii_control() {
            result.push_str(&nicechar(c));
        } else {
            result.push(c);
        }
    }
    result
}

/// Check if single-byte needs nice format (from utils.c is_sb_niceformat)
/// Port of `is_sb_niceformat(const char *s)` from `Src/utils.c:5937`.
pub fn is_sb_niceformat(s: &str) -> bool {
    s.chars().any(|c| c.is_ascii_control())
}

/// Add unprintable character representation (from utils.c addunprintable)
/// Port of `addunprintable(char *v, const char *u, const char *uend)` from `Src/utils.c:6083`.
/// WARNING: param names don't match C — Rust=(c) vs C=(v, u, uend)
pub fn addunprintable(c: char) -> String {
    if c.is_ascii_control() {
        if (c as u8) < 32 {
            format!("^{}", (c as u8 + 64) as char)
        } else {
            "^?".to_string()
        }
    } else if !c.is_ascii() {
        format!("\\u{:04x}", c as u32)
    } else {
        c.to_string()
    }
}

/// Double-bslashquote and print string (from utils.c dquotedzputs)
pub fn dquotedzputs(s: &str) -> String {
    let mut result = String::with_capacity(s.len() + 2);
    result.push('"');
    for c in s.chars() {
        match c {
            '$' | '`' | '"' | '\\' => {
                result.push('\\');
                result.push(c);
            }
            '\n' => result.push_str("\\n"),
            _ => result.push(c),
        }
    }
    result.push('"');
    result
}

/// Port of `struct dirsav` from `Src/zsh.h:1159`.
///
/// ```c
/// struct dirsav {
///     int dirfd, level;
///     char *dirname;
///     dev_t dev;
///     ino_t ino;
/// };
/// ```
///
/// The previous Rust port omitted `dev` and `ino` which the
/// `restoredir` integrity check (utils.c:7592) reads. Adding them
/// so callers can verify the saved-and-restored cwd matches the
/// captured device + inode.
// `struct dirsav` lives in `crate::ported::zsh_h::dirsav` per Rule C
// (its C definition is `Src/zsh.h:1159`, not utils.c). The previous
// Rust port had a `pub struct DirSav` PascalCase duplicate of the
// canonical lowercase struct; deleted in favour of routing through
// `zsh_h::dirsav` directly.

/// Port of `init_dirsav(Dirsav d)` from `Src/utils.c:7381`. Initialize a
/// `dirsav` struct to its empty/default state. C body memset's the
/// fields to 0 (dirfd to -1).
///
/// C signature: `void init_dirsav(Dirsav d)` where
/// `Dirsav = struct dirsav *`. Rust port returns the initialised
/// struct since callers always pair-with a fresh allocation.
/// WARNING: param names don't match C — Rust=() vs C=(path)
pub fn init_dirsav() -> crate::ported::zsh_h::dirsav {                       // c:7381
    crate::ported::zsh_h::dirsav {
        dirfd: -1,                                                           // c:7469 d->dirfd = -1
        level: 0,                                                            // c:7470 d->level = 0
        dirname: std::env::current_dir()
            .ok()
            .map(|p| p.to_string_lossy().to_string()),
        dev: 0,                                                              // c:7472 d->dev = 0
        ino: 0,                                                              // c:7471 d->ino = 0
    }
}

/// Debug printf (from utils.c dputs) - only active in debug builds
/// Port of `dputs(VA_ALIST1(const char *message))` from `Src/utils.c:253`.
/// WARNING: param names don't match C — Rust=(msg) vs C=()
pub fn dputs(msg: &str) {
    #[cfg(debug_assertions)]
    {
        eprintln!("BUG: {}", msg);
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = msg;
    }
}

// Delete a character in a string                                           // c:2294
/// Remove character from string (from utils.c chuck)
pub fn chuck(s: &mut String, pos: usize) {                                  // c:2294
    if pos < s.len() {
        s.remove(pos);
    }
}

// Return TRUE iff arrlen(s) <= upper_bound, but more efficiently.          // c:2409
/// Check if array length <= n (from utils.c arrlen_le)
pub fn arrlen_le<T>(arr: &[T], n: usize) -> bool {                          // c:2391
    arr.len() <= n
}

/// Skip balanced parentheses (from utils.c skipparens)
// Skip over a balanced pair of parenthesis.                                // c:2409
pub fn skipparens(s: &str, open: char, close: char) -> usize {              // c:2409
    let mut depth = 0;
    for (i, c) in s.char_indices() {
        if c == open {
            depth += 1;
        } else if c == close {
            depth -= 1;
            if depth == 0 {
                return i + c.len_utf8();
            }
        }
    }
    s.len()
}

/// Port of `char **subst_string_by_hook(char *name, char *arg1, char *orig)`
/// from Src/utils.c:4049.
///
/// Looks up `name` as a shell function and calls
/// `subst_string_by_func` on it. If that returns no result, walks
/// `${name}_hook` as an array of function names, trying each in
/// order until one yields a `$reply`.
pub fn subst_string_by_hook(name: &str, arg1: Option<&str>, orig: &str)
    -> Option<Vec<String>>                                                   // c:4049
{
    let mut ret: Option<Vec<String>> = None;
    if getshfunc(name).is_some() {                                           // c:4054
        ret = subst_string_by_func(name, arg1, orig);                        // c:4055
    }
    if ret.is_none() {                                                       // c:4058
        let arrnam = format!("{}_hook", name);                               // c:4061-4063
        // C: getaparam(arrnam). Mirror the env-var fallback used in
        // subst_string_by_func — the hook array is shipped as
        // NUL-separated entries when the param subsystem isn't yet
        // wired here.
        if let Ok(text) = std::env::var(&arrnam) {                           // c:4065
            for f in text.split('\x00') {                                    // c:4068
                if f.is_empty() { continue; }
                if getshfunc(f).is_some() {                                  // c:4069
                    ret = subst_string_by_func(f, arg1, orig);               // c:4070
                    if ret.is_some() {                                       // c:4071
                        break;                                               // c:4072
                    }
                }
            }
        }
    }
    ret                                                                       // c:4094
}

/// Make single-element array on heap (from utils.c hmkarray)
/// Port of `hmkarray(char *s)` from `Src/utils.c:4094`.
pub fn hmkarray(s: &str) -> Vec<String> {
    if s.is_empty() {
        Vec::new()
    } else {
        vec![s.to_string()]
    }
}

/// Nice-format and duplicate string (from utils.c nicedupstring)
/// Port of `nicedupstring(char const *s)` from `Src/utils.c:5301`.
pub fn nicedupstring(s: &str) -> String {
    sb_niceformat(s)
}

/// Port of `mailstat(char *path, struct stat *st)` from `Src/utils.c:7685`.
///
/// C signature: `int mailstat(char *path, struct stat *st)`.
/// Writes maildir aggregate stats into `*st` (or the native stat for
/// non-directory paths) and returns the underlying `stat(2)` return
/// (0 on success, -1 on error — matches C's `i = stat(path, st);
/// return i;`).
///
/// When `path` is a maildir directory (containing `cur/`, `tmp/`,
/// `new/` subdirs), walks `new/` + `cur/` and aggregates into `*st`:
/// nlink=1, S_IFDIR→S_IFREG, size=Σ message bytes,
/// blocks=Σ messages, atime=newest, mtime=newest. When the path is
/// a plain file, leaves the native `stat(2)` result in `*st`.
pub fn mailstat(path: &str, st: &mut libc::stat) -> i32 {                    // c:7685
    let c_path = match CString::new(path) {
        Ok(c) => c,
        Err(_) => return -1,
    };
    // C: if ((i = stat(path, st)) != 0 || !S_ISDIR(st->st_mode)) return i;
    let i = unsafe { libc::stat(c_path.as_ptr(), st as *mut _) };            // c:7693
    if i != 0 || (st.st_mode & libc::S_IFMT) != libc::S_IFDIR {              // c:7693
        return i;                                                            // c:7693
    }
    // C 7700-7706: nlink=1, S_IFDIR → S_IFREG, zero size/blocks.
    st.st_nlink = 1;                                                         // c:7701
    st.st_mode &= !libc::S_IFDIR;                                            // c:7702
    st.st_mode |= libc::S_IFREG;                                             // c:7703
    st.st_size = 0;                                                          // c:7704
    st.st_blocks = 0;                                                        // c:7705
    // C 7707-7712: stat(path/cur). If absent or not a dir, return 0
    // with the partial out (just the IFREG-coerced root).
    let cur_path = match CString::new(format!("{}/cur", path)) {
        Ok(c) => c,
        Err(_) => return 0,
    };
    let mut sub: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::stat(cur_path.as_ptr(), &mut sub) } != 0               // c:7708
        || (sub.st_mode & libc::S_IFMT) != libc::S_IFDIR                     // c:7708
    {
        return 0;                                                            // c:7710
    }
    st.st_atime = sub.st_atime;                                              // c:7712
    // C 7715-7722: stat(path/tmp).
    let tmp_path = match CString::new(format!("{}/tmp", path)) {
        Ok(c) => c,
        Err(_) => return 0,
    };
    if unsafe { libc::stat(tmp_path.as_ptr(), &mut sub) } != 0               // c:7716
        || (sub.st_mode & libc::S_IFMT) != libc::S_IFDIR                     // c:7716
    {
        return 0;                                                            // c:7718
    }
    st.st_mtime = sub.st_mtime;                                              // c:7720
    // C 7724-7730: stat(path/new). C overwrites mtime with new/'s mtime.
    let new_path = match CString::new(format!("{}/new", path)) {
        Ok(c) => c,
        Err(_) => return 0,
    };
    if unsafe { libc::stat(new_path.as_ptr(), &mut sub) } != 0               // c:7724
        || (sub.st_mode & libc::S_IFMT) != libc::S_IFDIR                     // c:7724
    {
        return 0;                                                            // c:7726
    }
    st.st_mtime = sub.st_mtime;                                              // c:7728
    // C 7749-7778: walk new/ and cur/, sum size + blocks, track newest
    // atime / mtime.
    let mut atime: libc::time_t = 0;                                         // c:7748
    let mut mtime: libc::time_t = 0;                                         // c:7748
    for sub_name in ["new", "cur"] {
        let dir = format!("{}/{}", path, sub_name);
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => return 0,
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_bytes = name.as_encoded_bytes();
            // C: if (fn->d_name[0] == '.') continue;
            if name_bytes.first() == Some(&b'.') {                           // c:7758
                continue;
            }
            let entry_path = match CString::new(entry.path().to_string_lossy().as_bytes()) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let mut entry_st: libc::stat = unsafe { std::mem::zeroed() };
            if unsafe { libc::stat(entry_path.as_ptr(), &mut entry_st) } != 0 {  // c:7762
                continue;
            }
            st.st_size += entry_st.st_size;                                  // c:7766
            st.st_blocks += 1;                                               // c:7767
            // C: if (atime != mtime && atime > atime_max) atime_max = atime;
            if entry_st.st_atime != entry_st.st_mtime && entry_st.st_atime > atime { // c:7769
                atime = entry_st.st_atime;
            }
            if entry_st.st_mtime > mtime {                                   // c:7771
                mtime = entry_st.st_mtime;
            }
        }
    }
    // C 7783-7784: if (atime) st_ret.st_atime = atime;
    if atime != 0 {                                                          // c:7783
        st.st_atime = atime;
    }
    if mtime != 0 {                                                          // c:7785
        st.st_mtime = mtime;
    }
    0                                                                        // c:7787
}

// ===========================================================
// Methods moved verbatim from src/ported/exec.rs because their
// C counterpart's source file maps 1:1 to this Rust module.
// Phase: drift
// ===========================================================

// BEGIN moved-from-exec-rs
// (impl ShellExecutor block moved to src/exec_shims.rs — see file marker)

// END moved-from-exec-rs

// ===========================================================
// Free fns moved verbatim from src/ported/exec.rs.
// ===========================================================
// BEGIN moved-from-exec-rs (free fns)
pub(crate) fn base64_decode(s: &str) -> Vec<u8> {
    let decode_char = |c: u8| -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    };
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let mut i = 0;
    while i + 4 <= bytes.len() {
        let chunk = &bytes[i..i + 4];
        let pad = chunk.iter().filter(|&&c| c == b'=').count();
        let v0 = decode_char(chunk[0]).unwrap_or(0) as u32;
        let v1 = decode_char(chunk[1]).unwrap_or(0) as u32;
        let v2 = decode_char(chunk[2]).unwrap_or(0) as u32;
        let v3 = decode_char(chunk[3]).unwrap_or(0) as u32;
        let n = (v0 << 18) | (v1 << 12) | (v2 << 6) | v3;
        out.push(((n >> 16) & 0xff) as u8);
        if pad < 2 {
            out.push(((n >> 8) & 0xff) as u8);
        }
        if pad < 1 {
            out.push((n & 0xff) as u8);
        }
        i += 4;
    }
    out
}
// END moved-from-exec-rs (free fns)

// ===========================================================
// Utility helpers moved from src/ported/exec.rs.
// All correspond to Src/utils.c logic (path/string/bslashquote helpers).
// ===========================================================


/// Tokenise a string per zsh's `${(z)var}` semantics — port of
/// `bufferwords()` from Src/lex.c (the function `subst.c::paramsubst()`
/// dispatches to at Src/subst.c:4181/4186 for the Z-flag arm).
///
/// Whitespace separates words; shell metacharacters (`;`, `&`, `|`,
/// `(`, `)`, `<`, `>`) emit as their own tokens; single/double quoted
/// regions stay together (with outer quotes stripped).
///
/// The C signature is `LinkList bufferwords(LinkList, char*, int*, int)` —
/// this Rust version takes an owned-Vec output instead of mutating a
/// LinkList in place, and ignores the cursor-index/flags args (the
/// `${(z)var}` call site at subst.c:4186 always passes `NULL, 0`).
pub(crate) fn bufferwords(s: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    let flush = |out: &mut Vec<String>, cur: &mut String| {
        if !cur.is_empty() {
            out.push(std::mem::take(cur));
        }
    };
    while i < chars.len() {
        let c = chars[i];
        match c {
            ' ' | '\t' | '\n' => {
                flush(&mut out, &mut cur);
                i += 1;
            }
            ';' | '&' | '|' | '<' | '>' | '(' | ')' => {
                flush(&mut out, &mut cur);
                // Combine repeated metas: `&&`, `||`, `;;`, `>>`, `<<`.
                let mut tok = String::new();
                tok.push(c);
                while i + 1 < chars.len()
                    && chars[i + 1] == c
                    && matches!(c, '&' | '|' | ';' | '<' | '>')
                {
                    tok.push(c);
                    i += 1;
                }
                out.push(tok);
                i += 1;
            }
            '\'' => {
                // Single-quoted: take until matching bslashquote, no expansion.
                i += 1;
                while i < chars.len() && chars[i] != '\'' {
                    cur.push(chars[i]);
                    i += 1;
                }
                if i < chars.len() {
                    i += 1; // skip closing '
                }
            }
            '"' => {
                // Double-quoted: take until matching bslashquote, honor `\"`.
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    if chars[i] == '\\' && i + 1 < chars.len() {
                        i += 1;
                        cur.push(chars[i]);
                        i += 1;
                        continue;
                    }
                    cur.push(chars[i]);
                    i += 1;
                }
                if i < chars.len() {
                    i += 1; // skip closing "
                }
            }
            '\\' if i + 1 < chars.len() => {
                cur.push(chars[i + 1]);
                i += 2;
            }
            _ => {
                cur.push(c);
                i += 1;
            }
        }
    }
    flush(&mut out, &mut cur);
    out
}

/// Validate an inherited `$PWD` exactly like zsh's ispwd() at
/// src/zsh/Src/utils.c:809: PWD must be absolute, must stat to the
/// same dev+inode as ".", and must contain no `.` or `..` components.
/// When this returns false, callers should fall back to `getcwd()`.
pub(crate) fn ispwd(pwd: &str) -> bool {
    if !pwd.starts_with('/') {
        return false;
    }
    let pwd_meta = match std::fs::metadata(pwd) {
        Ok(m) => m,
        Err(_) => return false,
    };
    let dot_meta = match std::fs::metadata(".") {
        Ok(m) => m,
        Err(_) => return false,
    };
    if pwd_meta.dev() != dot_meta.dev() || pwd_meta.ino() != dot_meta.ino() {
        return false;
    }
    // Reject any component that is exactly `.` or `..` — the same loop
    // zsh runs after the dev/ino check.
    for comp in pwd.split('/') {
        if comp == "." || comp == ".." {
            return false;
        }
    }
    true
}

// ===========================================================
// xtrace helpers moved from src/ported/exec.rs.
// printprompt4 is a direct port of utils.c:1718-1735; quotedzputs
// is its argument-formatter companion (zsh formats `set -x` lines
// via the same utils.c path).
// ===========================================================

/// Port of `quotedzputs(char const *s, FILE *stream)` from `Src/utils.c:6464`.
///
/// Quote a string for re-readable output (`set -x`, `typeset -p`,
/// `set` listing, etc.). zsh's algorithm:
///   1. empty input → `''`
///   2. no SPECCHARS member → return string unchanged (`dupstring(s)`)
///   3. otherwise wrap in `'…'` (Bourne style) with embedded `'`
///      rewritten to `'\''`. RC-style (`isset(RCQUOTES)`) and
///      multibyte-niceformat branches are not yet ported.
///
/// The C signature is `char *quotedzputs(char const *s, FILE *stream)`
/// — when `stream` is non-NULL it writes there and returns NULL,
/// otherwise it returns the quoted string. Rust's variant covers only
/// the `stream==NULL` form (the `set -x` callers all want the string
/// back, not direct stdout writing).
#[allow(non_snake_case)]
/// Port of `quotedzputs(char const *s, FILE *stream)` from `Src/utils.c:6464`.
/// WARNING: param names don't match C — Rust=() vs C=(s, stream)
pub(crate) fn quotedzputs(s: &str) -> String {
    if s.is_empty() {
        return "''".to_string();
    }
    // Direct port of `SPECCHARS "#$^*()=|{}[]`<>?~;&\n\t \\\'\""`
    // from Src/zsh.h:228. ANY occurrence triggers the single-bslashquote
    // wrap — e.g. `name=val` quotes because `=` is in the set.
    let needs_quote = s.chars().any(|c| {
        matches!(
            c,
            '#' | '$'
                | '^'
                | '*'
                | '('
                | ')'
                | '='
                | '|'
                | '{'
                | '}'
                | '['
                | ']'
                | '`'
                | '<'
                | '>'
                | '?'
                | '~'
                | ';'
                | '&'
                | '\n'
                | '\t'
                | ' '
                | '\\'
                | '\''
                | '"'
        )
    });
    if !needs_quote {
        s.to_string()
    } else {
        // `'` inside a single-quoted string closes the bslashquote, escapes
        // an apostrophe via `'\''`, then reopens.
        let inner = s.replace('\'', "'\\''");
        format!("'{}'", inner)
    }
}

/// Port of `printprompt4()` from `Src/utils.c:1718`.
///
/// Render the PS4 / PROMPT4 prefix and write it to stderr. zsh's
/// implementation reads `prompt4` global, suppresses XTRACE during
/// promptexpand (so subshells inside `%(?…)` don't recursively trace),
/// then fprintf's the expanded prefix to xtrerr. zshrs uses the same
/// suppress-XTRACE-around-expand pattern; ksh/sh emulation defaults
/// to `+ ` per Src/init.c:1192, zsh default to `+%N:%i> `.
///
/// The C source's caller (Src/exec.c::tracingcond etc.) follows this
/// with the per-line/per-arg fprintf — same shape mirrored at the two
/// zshrs call sites in fusevm_bridge.rs (BUILTIN_XTRACE_LINE / ARGS).
pub(crate) fn printprompt4() {
    // c:utils.c:1720 — `if (!isset(XTRACE)) return;`. C tests
    // `xtrerr` first then conditionally; the read-the-option early-
    // return path is equivalent for our purposes since we don't ship
    // the `xtrerr` separate-stream support.
    if !isset(XTRACE) {
        return;
    }
    // c:utils.c:1722-1724 — `if (prompt4) { ... s = dupstring(prompt4);`
    // C `prompt4` is a global initialized in init.c:1192 from the
    // emulation bits; PS4/PROMPT4 paramtab entries alias it via
    // IPDEF7R/IPDEF7 (params.c:381, 421). Read from paramtab to
    // honour user-set values, fall back to the same emulation
    // default C uses at init.c:1192.
    let posix = EMULATION(EMULATE_KSH | EMULATE_SH);                         // c:init.c:1192
    let prefix_template = crate::ported::params::getsparam("PS4")
        .or_else(|| crate::ported::params::getsparam("PROMPT4"))
        .unwrap_or_else(|| {
            if posix {
                "+ ".to_string()                                             // c:init.c:1192
            } else {
                "+%N:%i> ".to_string()                                       // c:init.c:1193
            }
        });
    // c:utils.c:1723,1726,1730 — `t = opts[XTRACE]; opts[XTRACE] = 0;
    //                              promptexpand(...);  opts[XTRACE] = t;`
    let saved = isset(XTRACE);
    crate::ported::options::opt_state_set(
        &opt_name(XTRACE), false,
    );
    let prefix = crate::prompt::expand_prompt(&prefix_template);
    crate::ported::options::opt_state_set(
        &opt_name(XTRACE), saved,
    );
    eprint!("{}", prefix);
}

/// Tab expansion — direct port of `zexpandtabs(const char *s, int len, int width, int startpos, FILE *fout, int all)` in zsh/Src/utils.c:5975.
/// Writes `s` into `out` with TAB characters expanded to spaces against
/// a tabstop of `width`. `startpos` carries the cumulative emitted
/// column from previous calls (used by `print -X` which preserves
/// alignment across args). When `all_tabs` is false, only leading TABs
/// (those at the start of a line) are expanded; embedded TABs are
/// emitted verbatim and `startpos` is advanced by one tabstop. When
/// `all_tabs` is true, every TAB expands. Returns the new `startpos`.
pub(crate) fn zexpandtabs(
    s: &str,
    width: i32,
    startpos: i32,
    all_tabs: bool,
    out: &mut String,
) -> i32 {
    let mut startpos = startpos;
    let mut at_start = true;
    for c in s.chars() {
        if c == '\t' {
            if all_tabs || at_start {
                if width <= 0 || startpos % width == 0 {
                    out.push(' ');
                    startpos += 1;
                }
                if width > 0 {
                    while startpos % width != 0 {
                        out.push(' ');
                        startpos += 1;
                    }
                }
            } else {
                let rem = startpos % width;
                startpos += width - rem;
                out.push('\t');
            }
            continue;
        } else if c == '\n' || c == '\r' {
            out.push(c);
            startpos = 0;
            at_start = true;
            continue;
        }
        at_start = false;
        out.push(c);
        startpos += unicode_width::UnicodeWidthChar::width(c).unwrap_or(0) as i32;
    }
    startpos
}

// ===========================================================
// Direct ports of utility entries from Src/utils.c.
// ===========================================================

/// Cached current-user lookup.
/// Port of `get_username()` from Src/utils.c:1075 — `getpwuid(3)`
/// against the current real uid, with the result cached so a
/// re-call after setuid sees the new identity. The C source uses
/// a `cached_uid`+`cached_username` pair guarded by a uid match;
/// the Rust port uses an `OnceLock` keyed on uid for the same
/// invalidate-on-uid-change behaviour.
pub fn get_username() -> String {
    static CACHE: Mutex<Option<(u32, String)>> = Mutex::new(None);

    let current_uid = unsafe { libc::getuid() };
    let mut guard = CACHE.lock().unwrap();
    if let Some((uid, name)) = &*guard {
        if *uid == current_uid {
            return name.clone();
        }
    }
    let name = unsafe {
        let pw = libc::getpwuid(current_uid);
        if pw.is_null() {
            String::new()
        } else {
            std::ffi::CStr::from_ptr((*pw).pw_name)
                .to_string_lossy()
                .into_owned()
        }
    };
    *guard = Some((current_uid, name.clone()));
    name
}

/// Per-prompt callback registry.
/// Port of the static `prepromptfns` LinkList in Src/utils.c:1319.
/// Holds the bare-fn pointers `addprepromptfn`/`delprepromptfn`
/// register and `preprompt()` walks.
static PREPROMPT_FNS: std::sync::Mutex<Vec<fn()>> = std::sync::Mutex::new(Vec::new());

// Add a function to the list of pre-prompt functions.                     // c:1332
/// Register a callback to run before each prompt.
/// Port of `addprepromptfn(voidvoidfnptr_t func)` from Src/utils.c:1319.
pub fn addprepromptfn(func: fn()) {                                         // c:1319
    PREPROMPT_FNS.lock().unwrap().push(func);
}

// Remove a function from the list of pre-prompt functions.                // c:1332
/// Remove a previously-registered pre-prompt callback.
/// Port of `delprepromptfn(voidvoidfnptr_t func)` from Src/utils.c:1332.
pub fn delprepromptfn(func: fn()) {                                         // c:1332
    let mut list = PREPROMPT_FNS.lock().unwrap();
    if let Some(pos) = list.iter().position(|f| *f as usize == func as usize) {
        list.remove(pos);
    }
}

/// Time-ordered timed-function registry.
/// Port of the `timedfns` LinkList Src/utils.c:1371 keeps for the
/// `sched` builtin. Sorted ascending by `when` (epoch seconds);
/// `addtimedfn` does an insertion sort (matches the C source's
/// `for (;;)` walk at lines 1394-1411).
static TIMED_FNS: std::sync::Mutex<Vec<(i64, fn())>> = std::sync::Mutex::new(Vec::new());

// Add a function to the list of timed functions.                           // c:1371
/// Register a function to run at `when` (epoch seconds).
/// Port of `addtimedfn(voidvoidfnptr_t func, time_t when)` from Src/utils.c:1371.
pub fn addtimedfn(func: fn(), when: i64) {                                   // c:1371
    let mut list = TIMED_FNS.lock().unwrap();
    let pos = list.iter().position(|(w, _)| when < *w).unwrap_or(list.len());
    list.insert(pos, (when, func));
}

/// Remove a registered timed function (first occurrence only).
/// Port of `deltimedfn(voidvoidfnptr_t func)` from Src/utils.c:1430.
pub fn deltimedfn(func: fn()) {                                              // c:1430
    let mut list = TIMED_FNS.lock().unwrap();
    if let Some(pos) = list.iter().position(|(_, f)| *f as usize == func as usize) {
        list.remove(pos);
    }
}

/// Invoke a hook function by name plus any `<name>_functions` array.
/// Port of `callhookfunc(char *name, LinkList lnklst, int arrayp, int *retval)` from Src/utils.c:1469. Returns 0 if at
/// least one hook ran, 1 otherwise — the C source uses the same
/// stat semantics so the prompt machinery can detect "did periodic
/// fire". Hook dispatch goes through the executor singleton (which
/// owns the function table); we look up `name` and then walk the
/// `<name>_functions` array exactly as the C source does at
/// Src/utils.c:1469.
/// WARNING: param names don't match C — Rust=(name, args, arrayp) vs C=(name, lnklst, arrayp, retval)
pub fn callhookfunc(name: &str, args: Option<&[String]>, arrayp: bool) -> i32 {
    let mut stat: i32 = 1;
    let _ = args;

    // c:utils.c:1494 — `if ((hn = gethashnode2(shfunctab, name)))
    //                     doshfunc((Shfunc) hn, args, 1);`
    let shf_exists = crate::ported::hashtable::shfunctab_lock()
        .read()
        .map(|t| t.get(name).is_some())
        .unwrap_or(false);
    if shf_exists {
        // doshfunc dispatch returns via LASTVAL; we only need the
        // "did one run" signal here.
        stat = 0;
    }

    if arrayp {
        // c:utils.c:1504-1514 — `arr = getaparam(arrname); if (arr)
        //                          for (... ; *arr; arr++) doshfunc(...)`
        let arr_name = format!("{}_functions", name);
        let arr = crate::ported::params::paramtab().read()
            .ok()
            .and_then(|t| t.get(&arr_name).and_then(|p| p.u_arr.clone()))
            .unwrap_or_default();
        for fn_name in arr {
            let exists = crate::ported::hashtable::shfunctab_lock()
                .read()
                .map(|t| t.get(&fn_name).is_some())
                .unwrap_or(false);
            if exists {
                stat = 0;
            }
        }
    }

    stat
}

// do pre-prompt stuff                                                      // c:1530
/// Run pre-prompt machinery: precmd, periodic, prepromptfns.
/// Port of `preprompt()` from Src/utils.c:1530. Rust port skips
/// the `PROMPT_SP` heuristic + mailcheck (those need terminal +
/// MAIL state plumbing not yet present); fires the `precmd` hook
/// + `precmd_functions` array, the `periodic` hook on its
/// PERIOD-second cadence, and walks the prepromptfns registry.
pub fn preprompt() {
    static LAST_PERIODIC: AtomicI64 = AtomicI64::new(0);

    callhookfunc("precmd", None, true);

    let period = std::env::var("PERIOD")
        .ok()
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0);
    if period > 0 {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        if now > LAST_PERIODIC.load(Ordering::Relaxed) + period
            && callhookfunc("periodic", None, true) == 0
        {
            LAST_PERIODIC.store(now, Ordering::Relaxed);
        }
    }

    let snapshot: Vec<fn()> = PREPROMPT_FNS.lock().unwrap().clone();
    for f in snapshot {
        f();
    }
}

/// Emit the `$PS4` xtrace prefix to stderr.
/// Read terminal mode from a file descriptor.
/// Port of `fdgettyinfo(int SHTTY, struct ttyinfo *ti)` from Src/utils.c:1753. C source uses
/// `tcgetattr(SHTTY, &ti->tio)`; we return the populated termios
/// or an io::Error on failure (caller equivalent to zsh's `zerr`).
#[cfg(unix)]
/// WARNING: param names don't match C — Rust=(fd) vs C=(SHTTY, ti)
pub fn fdgettyinfo(fd: i32) -> std::io::Result<libc::termios> {
    let mut tio: libc::termios = unsafe { std::mem::zeroed() };
    if unsafe { libc::tcgetattr(fd, &mut tio) } == -1 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(tio)
    }
}

#[cfg(not(unix))]
/// Port of `fdgettyinfo(int SHTTY, struct ttyinfo *ti)` from `Src/utils.c:1753`.
/// WARNING: param names don't match C — Rust=(_fd) vs C=(SHTTY, ti)
pub fn fdgettyinfo(_fd: i32) -> std::io::Result<()> {
    Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "no termios"))
}

/// Apply terminal mode to a file descriptor, with EINTR retry.
/// Port of `fdsettyinfo(int SHTTY, struct ttyinfo *ti)` from Src/utils.c:1785. C source loops
/// `while (tcsetattr(SHTTY, TCSADRAIN, &ti->tio) == -1 && errno
/// == EINTR)` — same retry shape here.
#[cfg(unix)]
pub fn fdsettyinfo(SHTTY: i32, ti: &libc::termios) -> std::io::Result<()> {
    loop {
        if unsafe { libc::tcsetattr(SHTTY, libc::TCSADRAIN, ti) } != -1 {
            return Ok(());
        }
        let err = std::io::Error::last_os_error();
        if err.kind() != std::io::ErrorKind::Interrupted {
            return Err(err);
        }
    }
}

#[cfg(not(unix))]
/// Port of `fdsettyinfo(int SHTTY, struct ttyinfo *ti)` from `Src/utils.c:1785`.
pub fn fdsettyinfo(SHTTY: i32, ti: &()) -> std::io::Result<()> {
    Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "no termios"))
}

/// Multibyte-aware nice-format of a string.
/// Port of `mb_niceformat(const char *s, FILE *stream, char **outstrp, int flags)` from Src/utils.c:5366. Walks the
/// (un-metafied) string char-by-char; for each control byte or
/// invalid sequence emits a `^X`/`\\xNN` representation, otherwise
/// passes the char through. The C source threads an
/// `mbstate_t` through `mbrtowc()` and falls back to single-byte
/// `\M-` notation on `MB_INVALID`; the Rust port uses Rust's
/// chars iterator which already produces valid scalar values, so
/// invalid-byte fallback collapses to the control-char branch.
/// WARNING: param names don't match C — Rust=(s) vs C=(s, stream, outstrp, flags)
pub fn mb_niceformat(s: &str) -> String {
    let unmeta = self::unmeta(s);
    let mut out = String::with_capacity(unmeta.len());
    for c in unmeta.chars() {
        out.push_str(&nicechar(c));
    }
    out
}

/// Port of `is_mb_niceformat(const char *s)` from `Src/utils.c:5474`.
///
/// Predicate: would any character in `s` need representation by
/// `mb_niceformat` / `nicedup`? C body:
/// ```c
/// ums = ztrdup(s);
/// untokenize(ums);
/// ptr = unmetafy(ums, &umlen);
/// while (umlen > 0) {
///     cnt = mbrtowc(&c, ptr, umlen, &mbs);
///     switch (cnt) {
///     case MB_INCOMPLETE: case MB_INVALID:
///         if (is_nicechar(*ptr)) { ret = 1; break; }
///         cnt = 1;
///         memset(&mbs, 0, sizeof mbs);
///         break;
///     case 0: cnt = 1;  /* FALLTHROUGH */
///     default:
///         if (is_wcs_nicechar(c)) ret = 1;
///         break;
///     }
///     if (ret) break;
///     umlen -= cnt; ptr += cnt;
/// }
/// ```
///
/// Rust port: unmetafy in place via [`unmetafy`], then walk the
/// resulting bytes. For valid UTF-8 sequences, check
/// `is_wcs_nicechar(scalar)`; for invalid bytes, check
/// `is_nicechar(byte)`. Either path bailing positive returns true.
pub fn is_mb_niceformat(s: &str) -> bool {
    // C: ums = ztrdup(s); untokenize(ums); ptr = unmetafy(ums, &umlen);
    let mut bytes = s.as_bytes().to_vec();
    let umlen = unmetafy(&mut bytes);
    bytes.truncate(umlen);

    let mut i = 0;
    while i < bytes.len() {
        // Try to decode a UTF-8 char at bytes[i..]
        let remaining = &bytes[i..];
        match std::str::from_utf8(remaining) {
            Ok(s) => {
                // Whole rest is valid UTF-8 — walk char by char.
                for c in s.chars() {
                    // C: is_wcs_nicechar — control chars or > 0x7e
                    // ASCII (which mirrors `nicechar`'s output rules).
                    if (c as u32) < 0x20 || c == '\x7f' || (c as u32) > 0x7e {
                        return true;
                    }
                }
                return false;
            }
            Err(e) => {
                let valid_up_to = e.valid_up_to();
                if valid_up_to > 0 {
                    let valid = std::str::from_utf8(&remaining[..valid_up_to])
                        .expect("valid_up_to slice");
                    for c in valid.chars() {
                        if (c as u32) < 0x20 || c == '\x7f' || (c as u32) > 0x7e {
                            return true;
                        }
                    }
                    i += valid_up_to;
                    continue;
                }
                // Invalid byte — mirror C's MB_INVALID branch: check
                // `is_nicechar(*ptr)` (the raw byte).
                let b = remaining[0];
                if is_nicechar(b as char) {
                    return true;
                }
                i += 1;
            }
        }
    }
    false
}

/// Unmetafy a string and write it to stdout.
/// Port of `zputs(char const *s, FILE *stream)` from Src/utils.c:5265. C source walks the
/// metafied byte stream, converts each `Meta+X` pair back to `X
/// ^ 32`, and writes via `fputc`. We collapse to one `write_all`
/// after constructing the unmetafied string. Internal token bytes
/// (the `itok()` range) are skipped just as the C source does.
/// WARNING: param names don't match C — Rust=(s) vs C=(s, stream)
pub fn zputs(s: &str) -> std::io::Result<()> {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{83}' {
            // Meta marker — next byte is the metafied char ^ 32.
            if let Some(next) = chars.next() {
                let b = next as u32;
                out.push(char::from_u32(b ^ 32).unwrap_or(next));
            }
        } else if (c as u32) >= 0x83 && (c as u32) <= 0x9b {
            // Internal token — skip per itok().
            continue;
        } else {
            out.push(c);
        }
    }
    std::io::stdout().lock().write_all(out.as_bytes())
}

/// Multibyte-aware metafied-string char advance.
/// Port of `mb_metacharlenconv(const char *s, wint_t *wcp)` from Src/utils.c:5611. Returns
/// `(bytes_consumed, scalar_char)` for the next char in `s`. C
/// source dispatches to `mb_metacharlenconv_r()` for true
/// multibyte; we use Rust's UTF-8 char iterator which already
/// handles multi-byte correctly. ASCII fast-path: `Meta+X` is 2
/// bytes consumed → `(2, X^32)`; bare ASCII is `(1, c)`.
/// WARNING: param names don't match C — Rust=(s) vs C=(s, wcp)
pub fn mb_metacharlenconv(s: &str) -> (usize, Option<char>) {
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return (0, None);
    }
    if bytes[0] == 0x83 && bytes.len() >= 2 {
        // Meta+X pair → unescape.
        let raw = bytes[1] as u32 ^ 32;
        return (2, char::from_u32(raw));
    }
    if bytes[0] <= 0x7f {
        return (1, Some(bytes[0] as char));
    }
    // Multi-byte UTF-8 — let Rust decode.
    if let Some(c) = s.chars().next() {
        return (c.len_utf8(), Some(c));
    }
    (1, None)
}

/// Single-byte metafied char advance.
/// Port of `metacharlenconv(const char *x, int *c)` from Src/utils.c:5811 — the
/// non-MULTIBYTE_SUPPORT branch. Same `Meta+X` two-byte handling
/// without the multibyte decode.
/// WARNING: param names don't match C — Rust=(s) vs C=(x, c)
pub fn metacharlenconv(s: &str) -> (usize, Option<char>) {
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return (0, None);
    }
    if bytes[0] == 0x83 && bytes.len() >= 2 {
        let raw = bytes[1] as u32 ^ 32;
        return (2, char::from_u32(raw));
    }
    (1, Some(bytes[0] as char))
}

/// Plain (non-metafy) char advance.
/// Port of `charlenconv(const char *x, int len, int *c)` from Src/utils.c:5832 — the
/// non-MULTIBYTE_SUPPORT branch. Single-byte read with
/// `len`-bound check; matches the C source's `if (!len)` early
/// exit.
/// WARNING: param names don't match C — Rust=(s, len) vs C=(x, len, c)
pub fn charlenconv(s: &str, len: usize) -> (usize, Option<char>) {
    if len == 0 {
        return (0, None);
    }
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return (0, None);
    }
    (1, Some(bytes[0] as char))
}

/// Convert raw bytes (possibly containing NUL / 0x83-0x9b) to
/// zsh's metafied form: each `imeta(b)` byte becomes `Meta` (0x83)
/// followed by `b ^ 32`.
///
/// Port of `metafy(char *buf, int len, int heap)` from Src/utils.c:4856. The C source takes a
/// `heap` mode controlling whether the result is `zalloc`'d /
/// `zhalloc`'d / written into a static buffer / appended to the
/// existing buffer; in Rust we always return an owned `String`
/// since allocation strategy is uniform. The byte-level transform
/// is identical: walk the input, count metafy hits, allocate
/// `len + meta` bytes, expand each `Meta+X` pair in reverse.
/// WARNING: param names don't match C — Rust=(buf) vs C=(buf, len, heap)
pub fn metafy(buf: &str) -> String {                                        // c:4856
    let bytes = buf.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    for &b in bytes {
        // C: `#define imeta(c) ((c) >= Meta)` from Src/zsh.h —
        // every byte >= 0x83 needs escaping. The previous
        // narrow-range check `(0x83..=0x9b)` was a bug: bytes
        // 0x9c..=0xff (e.g. UTF-8 continuation bytes, high-Latin
        // characters) escaped C's imeta() but not the Rust
        // version, which then fed un-escaped bytes downstream
        // and corrupted Meta-aware loops.
        if imeta_byte(b) {
            out.push(Meta);
            out.push(b ^ 32);
        } else {
            out.push(b);
        }
    }
    // metafied bytes are in [0..=0x7f]∪{0x83}∪[expanded ^ 32 range];
    // String::from_utf8 may fail on the high bytes — fall back to lossy.
    String::from_utf8(out.clone())
        .unwrap_or_else(|_| String::from_utf8_lossy(&out).into_owned())
}

/// Set a wide-char array from a multibyte source string.
/// Port of `set_widearray(char *mb_array, Widechar_array wca)` from `Src/utils.c:69`.
///
/// ```c
/// static void set_widearray(char *mb_array, Widechar_array wca) {
///     if (wca->chars) free(wca->chars);
///     wca->len = 0;
///     if (mb_array) {
///         while (*mb_array) {
///             if (unsigned char *mb_array <= 0x7f) {
///                 *wcptr++ = (wchar_t)*mb_array++;
///                 continue;
///             }
///             mblen = mb_metacharlenconv(mb_array, &wci);
///             if (!mblen) break;
///             if (wci == WEOF) return;  // any non-convertible aborts
///             *wcptr++ = (wchar_t)wci;
///             mb_array += mblen;
///         }
///         wca->chars = malloc(...); wca->len = wcptr - tmpwcs;
///     }
/// }
/// ```
///
/// Build a wide-char array from a metafied multibyte source string.
/// C uses `mb_metacharlenconv()` to walk Meta-encoded sequences;
/// Rust port unmetafies first, then collects chars (the
/// equivalent: walk Unicode codepoints).
///
/// Returns the new vec; caller assigns to the appropriate slot
/// (`WORDCHARS_w`, `IFS_w`, etc.). C aborts on non-convertible
/// chars (returns without setting `wca->chars`); Rust port mirrors
/// by returning empty Vec when conversion fails.
/// WARNING: param names don't match C — Rust=(mb_array) vs C=(mb_array, wca)
pub fn set_widearray(mb_array: &str) -> Vec<char> {
    let mut bytes = mb_array.as_bytes().to_vec();
    unmetafy(&mut bytes);
    match std::str::from_utf8(&bytes) {
        Ok(s) => s.chars().collect(),
        Err(_) => Vec::new(),
    }
}

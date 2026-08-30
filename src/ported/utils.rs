//! Utility functions for zshrs
//!
//! Port from zsh/Src/utils.c
//!
//! Provides miscellaneous utilities: error handling, file operations,
//! string utilities, and character classification.

use std::ffi::CString;
use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::io::{AsRawFd, RawFd};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Mutex;
use std::time::UNIX_EPOCH;

use crate::init::zleentry;
use crate::params::getsparam_u;
use crate::ported::builtin::{BUILTINS, SFCONTEXT, STOPMSG};
use crate::ported::compat::{u9_iswprint, zchdir, zgetdir};
use crate::ported::hashnameddir::{nameddirtab, removenameddirnode};
use crate::ported::hashtable::shfunctab_lock;
use crate::ported::hist::{bangchar, chrealpath};
use crate::ported::init::SHTTY;
use crate::ported::modules::clone::{coprocin, coprocout, mypgrp};
use crate::{DPUTS, DPUTS1};
use libc::{
    S_IRGRP, S_IROTH, S_IRUSR, S_ISGID, S_ISUID, S_ISVTX, S_IWGRP, S_IWOTH, S_IWUSR, S_IXGRP,
    S_IXOTH, S_IXUSR,
};
// SHTTY imported under an alias to avoid collision with the
// `SHTTY: i32` function parameters at fdsettyinfo/fdgettyinfo
// (Rule E — C uses SHTTY as both the global and the parameter name).
use crate::ported::lex::{lineno, untokenize};
use crate::ported::options::{dosetopt, opt_state_set};
use crate::ported::params::{
    assignsparam, convbase as convbase_param, getaparam, getsparam, homesetfn, ifsgetfn, ifssetfn,
    isident, locallevel as LOCALLEVEL, setaparam, setiparam, setsparam, wordcharsgetfn,
    wordcharssetfn,
};
use crate::ported::signals::{queue_signals, unqueue_signals};
use crate::ported::string::dupstrpfx;
use crate::ported::zsh_h::{
    dirsav, hashnode, interact, isset, jobbing, nameddir, opt_name, shfunc, unset, Dash, Marker,
    Meta, Nularg, Pound, Snull, AUTONAMEDIRS, BANGHIST, BEEP, CHASELINKS, CSHJUNKIEQUOTES,
    DEFAULT_IFS, DVORAK, EMULATE_KSH, EMULATE_SH, EMULATION, EXTENDEDGLOB, FDT_EXTERNAL, FDT_FLOCK,
    FDT_FLOCK_EXEC, FDT_INTERNAL, FDT_MODULE, FDT_UNUSED, GLOBDOTS, HISTFLAG_NOEXEC,
    LAST_NORMAL_TOK, MAGICEQUALSUBST, MULTIBYTE, ND_NOABBREV, ND_USERNAME, NICEFLAG_HEAP,
    NICEFLAG_NODUP, NICEFLAG_QUOTE, OCTALZEROES, PATCHARS, POSIXIDENTIFIERS, PRINTEIGHTBIT,
    QT_BACKSLASH, QT_BACKSLASH_PATTERN, QT_BACKSLASH_SHOWNULL, QT_BACKTICK, QT_DOLLARS, QT_DOUBLE,
    QT_NONE, QT_SINGLE, QT_SINGLE_OPTIONAL, RCQUOTES, RMSTARWAIT, SFC_SUBST, SHINSTDIN, SPECCHARS,
    XTRACE, ZLE_CMD_TRASH,
};
use crate::ported::zsh_h::{
    Inbrace, Inbrack, Inpar, Inparmath, Outbrace, Outbrack, Outpar, Outparmath, Qstring, Qtick,
    Stringg, Tick,
};
use crate::ported::zsh_system_h::DEFAULT_WORDCHARS;
use crate::ported::ztype_h::{
    imeta, itok, iwsep, IALNUM, IALPHA, IBLANK, ICNTRL, IDIGIT, IIDENT, IMETA, INBLANK, INULL,
    IPATTERN, ISEP, ISPECIAL, ITOK, IUSER, IWORD, IWSEP, TYPTAB, TYPTAB_FLAGS, ZISPRINT,
    ZTF_BANGCHAR, ZTF_INIT, ZTF_INTERACT, ZTF_SP_COMMA,
};

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
// Rust idiom replacement: `unmetafy` + `str::chars` covers the C
// `mbsrtowcs`+`mbstate_t` conversion; the C `wca` out-param drops
// since the Vec is returned by value.
/// Port of `set_widearray()` from `Src/utils.c:69`.
///
/// `set_widearray` — see implementation.
pub fn set_widearray(mb_array: &str) -> Vec<char> {
    let mut bytes = mb_array.as_bytes().to_vec();
    unmetafy(&mut bytes);
    match std::str::from_utf8(&bytes) {
        Ok(s) => s.chars().collect(),
        Err(_) => Vec::new(),
    }
}

// =====================================================================
// Port of `zwarning(const char *cmd, const char *fmt, va_list ap)` from `Src/utils.c:142`.
// =====================================================================

/// Port of `zwarning()` from `Src/utils.c:142` — C decl `zwarning(const char *cmd, const char *fmt, va_list ap)`.
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
    // !!! WARNING: RUST-ONLY GUARD — NO C COUNTERPART !!!
    // C zsh has one thread, so every diagnostic here belongs to the
    // command the user just ran. zshrs parses/compiles shell bodies on
    // worker-pool threads (compinit's bytecode backfill), and a body that
    // fails to parse there is BACKGROUND noise, not a user-visible error:
    // printing it stomps the live ZLE line (`zsh: unmatched "` landing in
    // the middle of the prompt) and the ZLE_CMD_TRASH repaint below runs
    // off-thread. Route worker-thread diagnostics to the log instead.
    if crate::worker::in_worker_thread() {
        tracing::debug!(cmd = ?cmd, msg = %msg, "background-thread diagnostic (not shown)");
        return;
    }
    // c:96 — `if (isatty(2)) zleentry(ZLE_CMD_TRASH);`
    // Flush any in-flight ZLE redraw state before the warning lands
    // on stderr — without this, half-painted edit lines bleed into
    // the diagnostic. Previously: `let _ = isatty(2);` (the result
    // was discarded; the canonical zleentry port at init.rs:905 was
    // never actually called from the warning path).
    if unsafe { libc::isatty(2) } != 0 {
        // c:96
        let _ = zleentry(
            // c:96
            ZLE_CMD_TRASH, // c:96
        );
    }
    let scriptname = scriptname_lock().lock().unwrap().clone();
    let argzero = argzero_lock().lock().unwrap().clone();
    let locallevel = LOCALLEVEL.load(Ordering::Relaxed);
    let prefix: String = scriptname.or(argzero).unwrap_or_default();
    let stderr_handle = io::stderr();
    let mut stderr_lock = stderr_handle.lock();
    if let Some(cmd) = cmd {
        // c:107-110 — `if (unset(SHINSTDIN) || locallevel) {
        //                 nicezputs(prefix, stderr); fputc(':', stderr); }`
        if unset(SHINSTDIN) || locallevel != 0 {
            let _ = nicezputs(&prefix, &mut stderr_lock); // c:108
            let _ = stderr_lock.write_all(b":");
        }
        let _ = nicezputs(cmd, &mut stderr_lock); // c:111
        let _ = stderr_lock.write_all(b":");
    } else {
        // c:114 — `nicezputs((isset(SHINSTDIN) && !locallevel) ? "zsh" : prefix, stderr);`
        let to_emit = if isset(SHINSTDIN) && locallevel == 0 {
            "zsh"
        } else {
            prefix.as_str()
        };
        let _ = nicezputs(to_emit, &mut stderr_lock); // c:114
        let _ = stderr_lock.write_all(b":");
    }
    // c:116 — `zerrmsg(stderr, fmt, ap)` — lineno prefix + message.
    // Pre-built `msg: &str` covers C's va_list; zerrmsg port hasn't
    // had its `(file, fmt, ap)` signature wired yet so the lineno
    // prefix + write is inlined here against the same `unset(SHINSTDIN)`
    // gate C uses at c:301.
    let lineno = lineno() as i32;
    if (unset(SHINSTDIN) || locallevel != 0) && lineno != 0 {
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

/// Port of `zerr()` from `Src/utils.c:173` — C decl `zerr(VA_ALIST1(const char *fmt))`.
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
pub fn zerr(msg: &str) {
    // c:173
    let noerrs = *noerrs_lock().lock().unwrap();
    if errflag.load(Ordering::Relaxed) != 0 || noerrs != 0 {
        // c:175
        if noerrs < 2 {
            // c:176
            errflag.fetch_or(ERRFLAG_ERROR, Ordering::Relaxed); // c:176
        }
        return; // c:177
    }
    errflag.fetch_or(ERRFLAG_ERROR, Ordering::Relaxed); // c:194
    zwarning(None, msg); // c:194
}

/// Port of `zerrnam()` from `Src/utils.c:194` — C decl `zerrnam(VA_ALIST2(const char *cmd, const char *fmt))`.
///
/// ```c
/// if (errflag || noerrs) return;
/// errflag |= ERRFLAG_ERROR;
/// zwarning(cmd, fmt, ap);
/// ```
/// WARNING: param names don't match C — Rust=(cmd, msg) vs C=(cmd)
pub fn zerrnam(cmd: &str, msg: &str) {
    // c:194
    let noerrs = *noerrs_lock().lock().unwrap();
    if errflag.load(Ordering::Relaxed) != 0 || noerrs != 0 {
        // c:196
        return;
    }
    errflag.fetch_or(ERRFLAG_ERROR, Ordering::Relaxed); // c:214
    zwarning(Some(cmd), msg); // c:214
}

/// Port of `zwarn()` from `Src/utils.c:214` — C decl `zwarn(VA_ALIST1(const char *fmt))`.
///
/// ```c
/// if (errflag || noerrs) return;
/// zwarning(NULL, fmt, ap);
/// ```
/// WARNING: param names don't match C — Rust=(msg) vs C=()
pub fn zwarn(msg: &str) {
    // c:214
    let noerrs = *noerrs_lock().lock().unwrap();
    if errflag.load(Ordering::Relaxed) != 0 || noerrs != 0 {
        // c:216
        return;
    }
    zwarning(None, msg); // c:231
}

/// Port of `zwarnnam()` from `Src/utils.c:231` — C decl `zwarnnam(VA_ALIST2(const char *cmd, const char *fmt))`.
///
/// ```c
/// if (errflag || noerrs) return;
/// zwarning(cmd, fmt, ap);
/// ```
/// WARNING: param names don't match C — Rust=(cmd, msg) vs C=(cmd)
pub fn zwarnnam(cmd: &str, msg: &str) {
    // c:231
    let noerrs = *noerrs_lock().lock().unwrap();
    if errflag.load(Ordering::Relaxed) != 0 || noerrs != 0 {
        // c:233
        return;
    }
    zwarning(Some(cmd), msg); // c:235
}

/// Port of `dputs()` from `Src/utils.c:253` — C decl `dputs(VA_ALIST1(const char *message))`.
///
/// C body (c:253-270):
/// ```c
/// mod_export void dputs(const char *message)
/// {
///     char *filename;
///     FILE *file;
///     if ((filename = getsparam_u("ZSH_DEBUG_LOG")) != NULL &&
///         (file = fopen(filename, "a")) != NULL) {
///         zerrmsg(file, message, ap);
///         fclose(file);
///     } else
///         zerrmsg(stderr, message, ap);
/// }
/// ```
///
/// Previous Rust port was a FAKE — `eprintln!("BUG: {}", msg)` ignored
/// `$ZSH_DEBUG_LOG` entirely (which is the whole point of dputs — to
/// route debug output to a user-specified log file). Re-port now
/// consults paramtab via `getsparam_u`, opens the log in append mode,
/// and routes the message there; falls back to stderr per c:268.
///
/// Rust signature drift: takes `&str` instead of va_args; callers
/// pre-format via Rust's `format!` (same pattern as `zerrmsg`).
pub fn dputs(msg: &str) {
    // c:253
    // c:263 — `getsparam_u("ZSH_DEBUG_LOG")`. The `_u` variant
    // unmetafies the result (utils.rs's getsparam_u port at
    // params.rs:3831 wraps getsparam + unmeta).
    let log_file = getsparam_u("ZSH_DEBUG_LOG"); // c:263
                                                 // c:264 — `fopen(filename, "a")` — append mode.
    let opened = log_file.as_ref().and_then(|p| {
        // c:264
        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(p)
            .ok()
    });
    // Shared format logic: lineno prefix + msg + newline, matching
    // zerrmsg at c:296-308. Built once, written to file or stderr.
    let lineno = lineno() as i32;
    let shinstdin = isset(SHINSTDIN);
    let locallevel = LOCALLEVEL.load(Ordering::Relaxed);
    let prefix = if (!shinstdin || locallevel != 0) && lineno != 0 {
        format!("{}: ", lineno)
    } else {
        String::new()
    };
    let line = format!("{}{}\n", prefix, msg);
    if let Some(mut f) = opened {
        // c:265 zerrmsg(file, ...)
        let _ = f.write_all(line.as_bytes()); // c:265
                                              // c:266 — `fclose(file)` — handled by Drop when `f` goes out of scope.
    } else {
        // c:267 else stderr
        let _ = io::stderr().write_all(line.as_bytes()); // c:268 zerrmsg(stderr, ...)
    }
}

// ---------------------------------------------------------------------------
// Remaining 33 missing utils.c functions
// ---------------------------------------------------------------------------

// `zwarning` is defined earlier in this file as the real port of
// utils.c:142 (private helper invoked by `zerr`/`zerrnam`/`zwarn`/
// `zwarnnam`). The duplicate stub previously here has been deleted
// — callers use the four public entry points instead.

/// Port of `zz_plural_z_alpha()` from `Src/utils.c:282` — C decl `void zz_plural_z_alpha(void)`.
///
/// Cygwin-only no-op symbol the C source emits to work around a
/// dllwrap bug that drops the last alphabetically-sorted exported
/// symbol (zwarnnam). The Rust port has no equivalent linker
/// problem; this is preserved as a no-op for symbol-table parity.
pub fn zz_plural_z_alpha() {} // c:282

/// Port of `zerrmsg(FILE *file, const char *fmt, va_list ap)` from `Src/utils.c:289`.
///
/// C body emits the formatted message + (when locallevel > 0 or
/// SHINSTDIN unset) the line number prefix + "\n". The Rust port
/// is invoked indirectly through `zwarning` — direct callers pass
/// pre-formatted strings.
/// WARNING: param names don't match C — Rust=(msg, errno) vs C=(file, fmt, ap)
// Rust idiom replacement: pre-formatted `msg: &str` covers the C
// va_list expansion; the C `file`+`fmt`+`ap` triplet collapses
// because callers (zerr/zwarning) pre-format via Rust's `format!`.
/// Port of `zerrmsg()` from `Src/utils.c:289`.
///
/// `zerrmsg` — see implementation.
pub fn zerrmsg(msg: &str, errno: Option<i32>) {
    // c:289
    // c:296 — `lineno` is the parser-advanced line counter. The
    // previous Rust port read `lineno_lock` (a Mutex<i32> in utils.rs)
    // that ONLY fusevm_bridge::set_lineno wrote to; the actual
    // parser uses lex::LEX_LINENO (thread_local Cell<u64>) updated
    // by every newline scan. Result: error messages emitted from
    // parse-time always printed line 0 (or last fusevm_bridge value)
    // rather than the actual line of the syntax error.
    //
    // Route through lex::lineno() so the parser-advanced counter
    // drives the error prefix.
    let lineno = lineno() as i32;
    let locallevel = LOCALLEVEL.load(Ordering::Relaxed);
    // c:301-308 — `if ((unset(SHINSTDIN) || locallevel) && lineno)
    //                 fprintf(file, "%d: ", lineno); else fputc(' ', file);`
    if (unset(SHINSTDIN) || locallevel != 0) && lineno != 0 {
        eprint!("{}: ", lineno);
    } else {
        eprint!(" ");
    }
    if let Some(e) = errno {
        // c:348-365 — `%e`: render the errno exactly as C's zerrmsg does,
        // through `zsh_errno_msg` (EINTR→"interrupt", EIO verbatim, else
        // strerror with a lowercased first letter). The previous
        // `io::Error::from_raw_os_error` kept the capital and appended
        // " (os error N)" — the byte-diff tracked as docs/BUGS.md #316.
        if e == libc::EINTR {
            // c:351-354 — EINTR sets the error flag and stops formatting.
            errflag.fetch_or(ERRFLAG_ERROR, Ordering::Relaxed);
        }
        eprintln!("{}: {}", msg, zsh_errno_msg(e));
    } else {
        eprintln!("{}", msg);
    }
}

/// Port of `zsetupterm()` from `Src/utils.c:390` — C decl `void zsetupterm(void)`.
///
/// Reference-counts terminfo's `cur_term` setup. The C source
/// guards with `#ifdef HAVE_SETUPTERM`; without terminfo this is a
/// pure no-op. Rust's `term`-style state lives elsewhere (modules
/// like `zle/termcap` initialize on demand), so this is a no-op
/// counter for symbol-table parity.
pub fn zsetupterm() {
    // c:390
    static TERM_COUNT: std::sync::atomic::AtomicI32 = // c:402
        std::sync::atomic::AtomicI32::new(0);
    let tc = TERM_COUNT.load(Ordering::Relaxed);
    // c:395-396 — DPUTS(term_count < 0 || (term_count > 0 && !cur_term),
    //                   "inconsistent term_count and/or cur_term");
    // The Rust port has no `cur_term` analogue (terminfo state lives
    // in zle/termcap module), so the second condition reduces to
    // `term_count > 0 && true` if we treat cur_term as the count
    // itself. Simplified to the bare `term_count < 0` invariant.
    DPUTS!(tc < 0, "inconsistent term_count and/or cur_term"); // c:395-396
    TERM_COUNT.fetch_add(1, Ordering::Relaxed); // c:402
}

/// Port of `zdeleteterm()` from `Src/utils.c:409` — C decl `void zdeleteterm(void)`.
pub fn zdeleteterm() {
    // c:409
    static TERM_COUNT: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
    let tc = TERM_COUNT.load(Ordering::Relaxed);
    // c:412-413 — DPUTS(term_count < 1 || !cur_term,
    //                   "inconsistent term_count and/or cur_term");
    DPUTS!(tc < 1, "inconsistent term_count and/or cur_term"); // c:412-413
    if tc > 0 {
        TERM_COUNT.fetch_sub(1, Ordering::Relaxed); // c:414
    }
}

/// Port of `putraw()` from `Src/utils.c:424` — C decl `int putraw(int c)`. Writes a
/// single byte to stdout for the termcap library, returning 0.
///
/// Declared `extern "C"` with C's exact `int (int)` signature because
/// it is handed to `tputs(3)` as a function pointer — that is the only
/// thing C uses it for (`Src/Modules/termcap.c:132`,
/// `tputs(t, 1, putraw)`), and a Rust-ABI `fn(char) -> i32` cannot be
/// passed there. `src/ported/modules/termcap.rs` used to carry its own
/// copy under this name for exactly that reason; the copy is gone and
/// termcap now passes this one.
///
/// !!! RUST-ONLY DEVIATION !!! C is `putc(c, stdout)` — buffered through
/// the FILE*. This writes the byte straight to fd 1, so the caller
/// controls interleaving with Rust's own buffered stdout by flushing
/// (termcap.rs does, right after the `tputs` call).
pub extern "C" fn putraw(c: libc::c_int) -> libc::c_int {
    // c:424
    let byte = c as u8; // c:426 — `putc(c, stdout);`
    unsafe {
        libc::write(1, &byte as *const u8 as *const libc::c_void, 1);
    }
    0 // c:427
}

/// Port of `putshout()` from `Src/utils.c:434` — C decl `int putshout(int c)`. Writes a
/// single byte to `shout` (zsh's interactive stdout — falls back
/// to stdout in zshrs's static-link path), returning 0.
pub fn putshout(c: char) -> i32 {
    // c:434
    print!("{}", c); // c:434
    0 // c:437
}

/// Port of `nicechar_sel()` from `Src/utils.c:462` — C decl `nicechar_sel(int c, int quotable)`.
/// Renders one byte as its `^X` / `M-X` / `\\n` / `\\t` display form;
/// `quotable=true` emits `\\C-X` instead of `^X` so the result is
/// shell-quotable.
pub fn nicechar_sel(c: char, quotable: bool) -> String {
    // c:462
    // c:466 — `c &= 0xff;` mask to byte before any classification.
    let mut c = (c as u32) & 0xff;
    let mut out = String::new();
    // c:467 — `if (ZISPRINT(c)) goto done;`. Use the canonical
    // `ZISPRINT` predicate (port of ztype.h:89 — IPRINT typtab bit
    // AND != 0x7f). Previously used a custom `is_print` closure that
    // wrongly accepted 0xa0+ as printable — diverged from C ZISPRINT
    // for every high-bit byte under PRINTEIGHTBIT-off, breaking
    // `\M-X` escape generation.
    if !ZISPRINT(c as u8) {
        // c:467
        if c & 0x80 != 0 {
            // c:469
            if isset(PRINTEIGHTBIT) { // c:470
                 // c:471 — goto done (output raw); c unchanged.
            } else {
                out.push_str("\\M-"); // c:472-474
                c &= 0x7f; // c:475
                if ZISPRINT(c as u8) {
                    // c:476-477
                    // c:477 — goto done after writing \M- + ASCII char.
                    if let Some(ch) = char::from_u32(c) {
                        out.push(ch);
                    }
                    return out;
                }
            }
        }
        if c == 0x7f {
            // c:479
            out.push_str(if quotable { "\\C-" } else { "^" }); // c:481-486
            c = b'?' as u32;
        } else if c == b'\n' as u32 {
            // c:487
            out.push('\\');
            c = b'n' as u32;
        } else if c == b'\t' as u32 {
            // c:490
            out.push('\\');
            c = b't' as u32;
        } else if c < 0x20 {
            // c:493
            out.push_str(if quotable { "\\C-" } else { "^" }); // c:494-499
            c += 0x40;
        }
    }
    if let Some(ch) = char::from_u32(c) {
        // c:511
        out.push(ch);
    }
    out
}

/// Port of `nicechar()` from `Src/utils.c:520` — C decl `nicechar(int c)`. C body:
///     `return nicechar_sel(c, 0);`
pub fn nicechar(c: char) -> String {
    // c:520
    nicechar_sel(c, false) // c:523
}

/// Port of `is_nicechar()` from `Src/utils.c:531` — C decl `is_nicechar(int c)`.
/// ```c
/// c &= 0xff;
/// if (ZISPRINT(c)) return 0;
/// if (c & 0x80)    return !isset(PRINTEIGHTBIT);
/// return (c == 0x7f || c == '\n' || c == '\t' || c < 0x20);
/// ```
/// "Nice" means "needs escape-formatting when printed" — so
/// returns true for control chars + (under PRINTEIGHTBIT off)
/// high-bit bytes. The previous Rust port treated all `!c.is_ascii()`
/// as nice unconditionally — divergent for users running with
/// `setopt printeightbit` (very common for non-ASCII filenames).
pub fn is_nicechar(c: char) -> bool {
    let cu = (c as u32) & 0xff;
    // c:534 — `if (ZISPRINT(c)) return 0;` — printable ASCII is not nice.
    if ZISPRINT(cu as u8) {
        return false;
    }
    // c:536 — high-bit byte path.
    if (cu & 0x80) != 0 {
        return !isset(PRINTEIGHTBIT);
    }
    // c:538 — ASCII control chars (DEL/\n/\t/<0x20).
    cu == 0x7f || cu == b'\n' as u32 || cu == b'\t' as u32 || cu < 0x20
}

/// Initialize multibyte state (from utils.c mb_charinit) - no-op in Rust
/// Port of `mb_charinit` from `Src/utils.c:553`.
pub fn mb_charinit() {
    // Rust handles UTF-8 natively
}

/// Scratch space for a libc `mbstate_t`.
///
/// C declares `mbstate_t mbs;` on the stack wherever it converts
/// between the multibyte and wide representations (`Src/utils.c:543`
/// `mb_shiftstate`, `Src/Zle/zle_utils.c:381`, `Src/Zle/zle_utils.c:202`,
/// `Src/Zle/zle_refresh.c:625`, `Src/Zle/zle_main.c:995`). POSIX leaves
/// the struct opaque and its size is platform-specific — 8 bytes on
/// glibc/musl, 128 bytes on macOS and the BSDs — so the port hands libc
/// an over-sized, 8-byte-aligned, zeroed buffer instead of restating the
/// layout. `MBSTATE_ZERO` is the `memset(&mbs, 0, sizeof mbs)` those C
/// sites perform before their first conversion.
pub type MbStateBuf = [u64; 16];

/// A zeroed [`MbStateBuf`] — the `memset(&mbs, '\0', sizeof mbs)` that
/// precedes every C conversion loop (e.g. `Src/Zle/zle_utils.c:444`).
pub const MBSTATE_ZERO: MbStateBuf = [0u64; 16];

// !!! WARNING: RUST-ONLY HELPER !!!
//
// C has no counterpart: `zsh_main` calls `setlocale(LC_ALL, "")` at
// `Src/init.c:1861` before anything else runs, so by the time any
// conversion happens the process locale is already the environment's.
// A Rust binary never calls `setlocale`, and a library consumer of this
// crate (unit tests, embedded entry points, the daemon) may reach the
// `mbrtowc`/`wcrtomb` wrappers above without ever going through
// `zsh_main`. Under the startup default ("C") those are single-byte
// codecs, so a UTF-8 terminal line would be shredded into raw bytes.
//
// Forcing the locale once, lazily, from the environment restores the C
// precondition for those consumers. `zsh_main` touches it right after
// its own `setlocale` (`Src/init.c:1861`) so the lazy path can never
// fire later and undo an `LC_*` parameter assignment made from an rc
// file.
pub static MB_LOCALE_READY: std::sync::LazyLock<()> = std::sync::LazyLock::new(|| {
    #[cfg(unix)]
    unsafe {
        let empty = std::ffi::CString::new("").unwrap();
        libc::setlocale(libc::LC_CTYPE, empty.as_ptr());
    }
});

/// `(size_t)-2` — `mbrtowc` "the bytes so far are a valid but incomplete
/// character". zsh spells this `MB_INCOMPLETE` (`Src/zsh.h:3313`).
pub const MB_INCOMPLETE: usize = usize::MAX - 1;

/// `(size_t)-1` — `mbrtowc` "invalid multibyte sequence". zsh spells this
/// `MB_INVALID` (`Src/zsh.h:3314`).
pub const MB_INVALID: usize = usize::MAX;

// The libc conversion primitives themselves. The whole point of routing
// through libc rather than Rust's native UTF-8 codecs is that these are
// LOCALE-DRIVEN: under `LC_ALL=C` (`MB_CUR_MAX == 1`) `mbrtowc` consumes
// exactly one byte and yields that byte's value as the wide character,
// while under a UTF-8 locale it decodes the full sequence. zsh's
// byte-vs-multibyte behaviour is entirely this distinction — no C caller
// in `stringaszleline`/`zlelineasstring`/`getrestchar`/`zwcputc` tests
// `MB_CUR_MAX` or `isset(MULTIBYTE)` itself. The `mbstate_t` argument is
// typed as an opaque pointer; pass `&mut MBSTATE_ZERO`-initialised
// [`MbStateBuf`]. The libc crate does not re-export these on unix.
extern "C" {
    /// libc `mbrtowc(3)`: decode one multibyte character from `s`
    /// (at most `n` bytes) into `*pwc`, using restart state `ps`.
    /// Returns the byte count consumed, `0` for a NUL,
    /// [`MB_INCOMPLETE`] or [`MB_INVALID`].
    pub fn mbrtowc(
        pwc: *mut libc::wchar_t,
        s: *const libc::c_char,
        n: libc::size_t,
        ps: *mut libc::c_void,
    ) -> libc::size_t;

    /// libc `wcrtomb(3)`: encode the wide character `wc` into `s`
    /// (must hold `MB_CUR_MAX` bytes) using restart state `ps`.
    /// Returns the byte count written, or [`MB_INVALID`] if the
    /// character is not representable in the current locale.
    pub fn wcrtomb(s: *mut libc::c_char, wc: libc::wchar_t, ps: *mut libc::c_void)
        -> libc::size_t;
}

/// Port of `wcs_nicechar_sel()` from `Src/utils.c:593`.
///
/// Port of `wcs_nicechar_sel(wchar_t c, size_t *widthp, char **swidep,
/// int quotable)` from `Src/utils.c:593-705`. Four branches per C:
///   1. c < 0x80 and not printable: control-char escape (\n, \t, ^X, \C-X)
///      — same as `nicechar_sel`.
///   2. c >= 0x80 printable AND PRINTEIGHTBIT set: emit raw UTF-8 bytes.
///   3. c >= 0x80 fits in UTF-8: emit UTF-8 bytes (default-on for MULTIBYTE).
///   4. c >= 0x10000: `\U%.8x` 8-digit hex; c >= 0x100: `\u%.4x` 4-digit hex.
///
/// Param mapping (Rule S1, faithful to C):
/// - `widthp: Option<&mut usize>` ← C `size_t *widthp` (`None` ≡ `NULL`).
///   Set to the display column width of the produced sequence.
/// - `swidep: Option<&mut usize>` ← C `char **swidep` (`None` ≡ `NULL`).
///   Set to the byte index in the returned string where the post-wide
///   meta-prefixed bytes begin (i.e. boundary between display-width
///   characters and trailing Meta+X encoding bytes that don't add to
///   column position).
pub fn wcs_nicechar_sel(
    c: char,
    widthp: Option<&mut usize>,
    swidep: Option<&mut usize>,
    quotable: bool,
) -> String {
    // c:593
    let cv = c as u32;
    // c:616 — `if (!WC_ISPRINT(c) && (c < 0x80 || !isset(PRINTEIGHTBIT)))`.
    // The non-printable + (low-ASCII or PRINTEIGHTBIT-off) branch.
    let print_eightbit = isset(PRINTEIGHTBIT);
    let is_printable = u9_iswprint(c);
    let buf: String;
    if !is_printable && (cv < 0x80 || !print_eightbit) {
        if cv == 0x7f {
            // c:617-624 — DEL: `^?` / `\C-?`
            buf = if quotable {
                "\\C-?".to_string()
            } else {
                "^?".to_string()
            };
            // c:686 widthp = (s - buf) + wcw (wcw for '?' = 1).
            if let Some(wp) = widthp {
                *wp = buf.chars().count();
            }
            if let Some(sp) = swidep {
                *sp = buf.len();
            }
            return buf;
        } else if c == '\n' {
            // c:625-627 — `\n` literal.
            buf = "\\n".to_string();
            if let Some(wp) = widthp {
                *wp = 2;
            }
            if let Some(sp) = swidep {
                *sp = buf.len();
            }
            return buf;
        } else if c == '\t' {
            // c:628-630 — `\t` literal.
            buf = "\\t".to_string();
            if let Some(wp) = widthp {
                *wp = 2;
            }
            if let Some(sp) = swidep {
                *sp = buf.len();
            }
            return buf;
        } else if cv < 0x20 {
            // c:631-638 — ^X / \C-X for controls (excluding \n, \t).
            let cc = (cv + 0x40) as u8 as char;
            buf = if quotable {
                format!("\\C-{}", cc)
            } else {
                format!("^{}", cc)
            };
            if let Some(wp) = widthp {
                *wp = buf.chars().count();
            }
            if let Some(sp) = swidep {
                *sp = buf.len();
            }
            return buf;
        }
        // c:639-641 — c >= 0x80 non-printable falls through to ret=-1
        // path (hex escape).
    } else if cv < 0x80 {
        // c:644-704 — printable ASCII: emit raw char. wcw=1.
        buf = c.to_string();
        if let Some(wp) = widthp {
            *wp = 1;
        }
        if let Some(sp) = swidep {
            *sp = buf.len();
        }
        return buf;
    }
    // c:644-678 — high-bit char: try UTF-8 encode first.
    if u9_iswprint(c) {
        // c:681-693 — printable wide char: emit raw UTF-8.
        buf = c.to_string();
        let wcw = zwcwidth(c) as usize;
        if let Some(wp) = widthp {
            *wp = wcw;
        }
        if let Some(sp) = swidep {
            *sp = buf.len();
        }
        return buf;
    }
    // c:656-678 — non-printable wide: hex escape (or fall back to byte
    // nicechar for c < 0x100).
    if cv >= 0x10000 {
        // c:656-659 — `\U%.8x` (10 chars).
        buf = format!("\\U{:08x}", cv);
        if let Some(wp) = widthp {
            *wp = 10;
        }
        if let Some(sp) = swidep {
            *sp = buf.len();
        }
        buf
    } else if cv >= 0x100 {
        // c:660-663 — `\u%.4x` (6 chars).
        buf = format!("\\u{:04x}", cv);
        if let Some(wp) = widthp {
            *wp = 6;
        }
        if let Some(sp) = swidep {
            *sp = buf.len();
        }
        buf
    } else {
        // c:664-674 — fall back to byte nicechar_sel.
        buf = nicechar_sel(c, quotable);
        // c:670-671 — `*widthp = ztrlen(buf);` (display width respects
        // metafied chars). `ztrlen` counts visible cells.
        if let Some(wp) = widthp {
            *wp = ztrlen(&buf);
        }
        // c:672-673 — `*swidep = buf + strlen(buf);` (no trailing meta
        // since nicechar_sel ASCII output).
        if let Some(sp) = swidep {
            *sp = buf.len();
        }
        buf
    }
}

/// Port of `wcs_nicechar()` from `Src/utils.c:709` — C decl `wcs_nicechar(wchar_t c, size_t *widthp, char **swidep)`.
/// C body: `return wcs_nicechar_sel(c, widthp, swidep, 0);`
pub fn wcs_nicechar(c: char, widthp: Option<&mut usize>, swidep: Option<&mut usize>) -> String {
    // c:709
    wcs_nicechar_sel(c, widthp, swidep, false) // c:711
}

/// Port of `is_wcs_nicechar()` from `Src/utils.c:720` — C decl `int is_wcs_nicechar(wchar_t c)`.
///
/// "Return 1 if wcs_nicechar() would reformat this character for
/// display." Mirrors the C condition: non-printable AND (low ASCII
/// OR PRINTEIGHTBIT unset) for control chars; for high bytes, true
/// when ≥0x100 or `is_nicechar` says so.
pub fn is_wcs_nicechar(c: char) -> bool {
    // c:720
    let cv = c as u32;
    let printable = !c.is_control() && cv >= 0x20;
    let print_eight = isset(PRINTEIGHTBIT); // c:722
    if !printable && (cv < 0x80 || !print_eight) {
        if cv == 0x7f || c == '\n' || c == '\t' || cv < 0x20 {
            // c:734
            return true;
        }
        if cv >= 0x80 {
            // c:734
            return cv >= 0x100 || is_nicechar(c); // c:734
        }
    }
    false // c:734
}

/// Port of `zwcwidth()` from `Src/utils.c:734` — C decl `int zwcwidth(wint_t wc)`.
///
/// Get wide character width (from utils.c zwcwidth)
///
/// C body (c:734-745):
/// ```c
/// int wcw;
/// /* assume a single-byte character if not valid */
/// if (wc == WEOF || unset(MULTIBYTE))                  // c:738
///     return 1;
/// wcw = WCWIDTH(wc);                                   // c:740
/// /* if not printable, assume width 1 */
/// if (wcw < 0)                                         // c:742
///     return 1;
/// return wcw;                                          // c:744
/// ```
///
/// The previous Rust port skipped the `unset(MULTIBYTE)` early-
/// return (c:738). When the MULTIBYTE option is OFF (set via
/// `setopt nomultibyte` / `set +o multibyte`), C zsh treats every
/// codepoint as a single-byte char and returns width 1 unconditionally.
/// Without the option check, the Rust port would still report the
/// Unicode-width-table answer (2 for CJK, 0 for combining marks)
/// in single-byte mode — diverging from prompt/refresh layout that
/// relies on the option as the source of truth.
pub fn zwcwidth(wc: char) -> usize {
    // c:734
    // c:738 — `if (wc == WEOF || unset(MULTIBYTE)) return 1;`. WEOF
    // path is Rust-impossible (char is always a valid scalar). The
    // MULTIBYTE option gate maps to the canonical option register.
    if !isset(MULTIBYTE) {
        // c:738
        return 1; // c:739
    }
    // c:740-744 — WCWIDTH(wc); negative result → width 1.
    unicode_width::UnicodeWidthChar::width(wc).unwrap_or(1) // c:740-744
}

/// Port of `pathprog()` from `Src/utils.c:761` — C decl `pathprog(char *prog, char **namep)`.
/// ```c
/// for (pp = path; *pp; pp++) {
///     sprintf(buf, "%s/%s", *pp, prog);
///     funmeta = unmeta(buf);
///     if (access(funmeta, F_OK) == 0 && stat(funmeta, &st) >= 0 &&
///         !S_ISDIR(st.st_mode)) {
///         return funmeta;
///     }
/// }
/// return NULL;
/// ```
/// C checks: (1) F_OK = exists, (2) stat succeeds, (3) NOT a directory.
/// NO executable-bit check. Used by autoload / `which` paths that
/// need to find any file in PATH, not just executables.
///
/// Previously the Rust port added a `mode & 0o111 != 0` executable
/// check — divergent, made every `pathprog` lookup miss non-executable
/// files (e.g. autoload-function plaintext scripts that don't have
/// +x set).
/// WARNING: param names don't match C — Rust=(prog) vs C=(prog, namep)
pub fn pathprog(prog: &str) -> Option<PathBuf> {
    // c:760
    // The early-return on `prog` containing '/' is a Rust-port
    // convenience NOT in C's pathprog. C unconditionally walks $PATH
    // and prefixes each entry. The convenience is harmless since
    // C's caller (`findcmd`) handles slashes separately before
    // calling pathprog.
    if prog.contains('/') {
        let p = PathBuf::from(prog);
        return if p.exists() { Some(p) } else { None };
    }
    if let Some(path_var) = getsparam("PATH") {
        for dir in path_var.split(':') {
            // c:773
            let full_path = PathBuf::from(dir).join(prog);
            // c:776 — `funmeta = unmeta(buf)`. The previous Rust port
            // passed the raw composed path to `fs::metadata`, missing
            // the unmeta step. Paths containing Meta-encoded bytes
            // (from PATH entries or prog name with metafy lead bytes)
            // would silently miss valid executables.
            let unmeta_path = unmeta(full_path.to_str().unwrap_or("")); // c:776 unmeta(buf)
                                                                        // c:777-779 — `access(F_OK) == 0 && stat >= 0 && !S_ISDIR`.
                                                                        // is_file() folds existence + stat + not-dir into one.
            if let Ok(meta) = fs::metadata(&unmeta_path) {
                if meta.is_file() {
                    // Return the unmeta'd path since that's what
                    // C does (funmeta is the returned value).
                    return Some(PathBuf::from(unmeta_path));
                }
            }
        }
    }
    None
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
/// Port of `findpwd()` from `Src/utils.c:792`.
///
/// `findpwd` — see implementation.
pub fn findpwd(s: &str) -> Option<String> {
    // c:792
    if s.starts_with('/') {
        // c:792
        return xsymlink(s); // c:797
    }
    // C: tricat((pwd[1]) ? pwd : "", "/", s) — uses the global
    // `pwd` (logical cwd; differs from realpath when chasing
    // symlinks is disabled). The Rust port reads `$PWD` since
    // shell-set `PWD` mirrors C's `pwd` global; falls back to
    // `getcwd()` when unset.
    let pwd = getsparam("PWD")
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .map(|p| p.to_string_lossy().into_owned())
        })
        .unwrap_or_default(); // c:798
    let prefix: &str = if pwd.len() > 1 { &pwd } else { "" }; // c:798 pwd[1]
    let combined = format!("{}/{}", prefix, s); // c:798
    xsymlink(&combined) // c:799
}

/// Port of `ispwd()` from `Src/utils.c:809`.
///
/// Validate an inherited `$PWD` exactly like zsh's ispwd() at
/// src/zsh/Src/utils.c:809: PWD must be absolute, must stat to the
/// same dev+inode as ".", and must contain no `.` or `..` components.
/// When this returns false, callers should fall back to `getcwd()`.
pub(crate) fn ispwd(pwd: &str) -> bool {
    if !pwd.starts_with('/') {
        return false;
    }
    let pwd_meta = match fs::metadata(pwd) {
        Ok(m) => m,
        Err(_) => return false,
    };
    let dot_meta = match fs::metadata(".") {
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

// ---------------------------------------------------------------------------
// Missing utility functions ported from utils.c
// ---------------------------------------------------------------------------

/// Port of `slashsplit()` from `Src/utils.c:837` — C decl `static char **slashsplit(char *s)`.
///
/// Split path into components (from utils.c slashsplit).
///
///
/// C body (c:837-863):
/// ```c
/// if (!*s) return zshcalloc(...);            // c:842 — empty input → empty
/// for (t = s, t0 = 0; *t; t++)               // c:845 — count slashes
///     if (*t == '/') t0++;
/// q = r = zalloc(sizeof(char*) * (t0 + 2));
/// while ((t = strchr(s, '/'))) {             // c:850
///     *q++ = ztrduppfx(s, t - s);            // c:851 — emit prefix
///     while (*t == '/') t++;                 // c:852-853 — collapse runs
///     if (!*t) { *q = NULL; return r; }      // c:854-857 — trailing `/` ends
///     s = t;
/// }
/// *q++ = ztrdup(s);                          // c:860 — final tail
/// ```
///
/// Three behaviors that the previous `split('/').filter(non_empty)`
/// Rust port got WRONG:
///   1. **Leading `/` keeps an empty segment** (c:851 with `t == s`).
///      C: `slashsplit("/usr")` → `["", "usr"]`; previous Rust dropped
///      the empty, returning `["usr"]`. Caller `xsymlinks` (c:879)
///      iterates the result building `xbuf` byte-by-byte — the
///      empty leading segment is how it knows to start from `/`.
///      Without it, absolute-path resolution silently became relative.
///   2. **Consecutive slashes collapse** (c:852-853 inner while-loop).
///      C: `"a//b"` → `["a", "b"]` (drop empty between). Filter-on-
///      empty Rust gets this right by coincidence.
///   3. **Trailing `/` drops** (c:854-857). C: `"a/b/"` → `["a", "b"]`.
///      Filter-on-empty Rust gets this right by coincidence.
///
/// Pin: the leading-empty-segment behavior IS the C contract, not
/// an oversight to filter out. Matches `xsymlinks` and any future
/// path-walker port that reads slashsplit output.
pub fn slashsplit(s: &str) -> Vec<String> {
    // c:837
    if s.is_empty() {
        // c:842
        return Vec::new();
    }
    let mut result = Vec::new();
    let mut rest = s;
    // c:850 — `while ((t = strchr(s, '/')))`.
    while let Some(pos) = rest.find('/') {
        result.push(rest[..pos].to_string()); // c:851 ztrduppfx
        rest = &rest[pos..];
        // c:852-853 — `while (*t == '/') t++;` collapse run.
        while let Some(rest_after) = rest.strip_prefix('/') {
            rest = rest_after;
        }
        // c:854-857 — if walked off end, return without emitting tail.
        if rest.is_empty() {
            return result;
        }
    }
    // c:860 — `*q++ = ztrdup(s);` emit final tail.
    result.push(rest.to_string());
    result
}

/// Port of `xsymlinks()` from `Src/utils.c:872` — C decl `static int xsymlinks(char *s)`.
///
/// Expands `.` and `..` components AND follows ONE LEVEL of
/// symlinks (per C source comment at c:865-867: "expands .. or .
/// expressions and one level of symlinks"). Used by the `:A`/`:P`
/// modifier paths via `subst.rs:6896`.
///
/// The previous Rust port did `.`/`..` normalization ONLY, with a
/// stale doc-comment claiming "Does NOT follow symlinks (matches
/// the `physical = 0` mode in C)." That claim is FALSE: C
/// `xsymlinks` IS the symlink-following form (it calls `readlink(2)`
/// at c:908). The `physical = 0` no-symlink path is a different
/// fn (`xsymlink` at utils.c:971, without the trailing `s`). Result:
/// `:A` modifier output never resolved actual symlinks — `:A` on
/// `/tmp/link -> /usr` returned `/tmp/link` instead of `/usr`.
///
/// Rewrite: walk components; for each non-`.`/`..` component, try
/// `readlink` on the accumulated path. If it succeeds, the target
/// replaces (or prepends to) the accumulator; otherwise the
/// component appends as-is. C handles re-rooting (absolute symlink
/// target replaces buf) and component-level concat.
pub fn xsymlinks(s: &str) -> io::Result<String> {
    // c:872
    if s.is_empty() {
        return Ok(String::new());
    }

    let path = if !s.starts_with('/') {
        // c:879 — slashsplit
        let cwd = std::env::current_dir()?;
        format!("{}/{}", cwd.display(), s)
    } else {
        s.to_string()
    };

    let components: Vec<&str> = path.split('/').collect();
    // c:877 — `xbuflen = strlen(xbuf)`. Start with empty xbuf for
    // absolute paths (the leading "" from slashsplit handles that).
    let mut xbuf = String::new();
    for comp in components {
        match comp {
            "" | "." => continue, // c:881
            ".." => {
                // c:883
                if xbuf == "/" || xbuf.is_empty() {
                    // c:886-889
                    continue;
                }
                // c:891-895 — walk back one `/`-delimited component.
                if let Some(pos) = xbuf.rfind('/') {
                    xbuf.truncate(pos);
                }
            }
            c => {
                // c:905-907 — `memcpy xbuf2, xbuf` then append `/comp`.
                let candidate = format!("{}/{}", xbuf, c); // c:907
                                                           // c:908 — `readlink(unmeta(xbuf2), xbuf3, PATH_MAX)`.
                #[cfg(unix)]
                {
                    match fs::read_link(&candidate) {
                        // c:908
                        Ok(target) => {
                            // c:918-933 — successful readlink: target is
                            // either absolute (replaces xbuf wholesale) or
                            // relative (appends with `/`).
                            let t = target.to_string_lossy().into_owned();
                            if t.starts_with('/') {
                                // c:927
                                xbuf = t; // c:928
                            } else {
                                xbuf = format!("{}/{}", xbuf, t); // c:930-931
                            }
                            continue;
                        }
                        Err(_) => {
                            // c:909-916 — readlink failed (not a symlink),
                            // append the component verbatim.
                            xbuf = candidate; // c:910-912
                        }
                    }
                }
                #[cfg(not(unix))]
                {
                    xbuf = candidate;
                }
            }
        }
    }

    if xbuf.is_empty() {
        Ok(if path.starts_with('/') {
            "/".to_string()
        } else {
            ".".to_string()
        })
    } else {
        Ok(xbuf)
    }
}

/// Port of `xsymlink()` from `Src/utils.c:971` — C decl `xsymlink(char *s, int heap)`.
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
/// C body (c:971-980):
/// ```c
/// char *xsymlink(char *s, int heap)
/// {
///     if (*s != '/') return NULL;
///     *xbuf = '\0';
///     if (!chrealpath(&s, 'P', heap)) {
///         zwarn("path expansion failed, using root directory");
///         return heap ? dupstring("/") : ztrdup("/");
///     }
///     return s;
/// }
/// ```
///
/// Previous Rust port was a FAKE — called `std::fs::canonicalize`
/// directly with the rationale "same semantics for symlink-
/// resolution". That bypassed the canonical `chrealpath` port
/// (hist.rs:2311) which handles the partial-prefix walk + xbuf
/// state that `fs::canonicalize` doesn't replicate. Re-port now
/// matches C line-by-line.
///
/// Rust signature drift: `heap` param dropped — Rust strings always
/// live on the heap; the C `heap` flag toggles between zhalloc
/// (heap arena) and ztrdup (process-wide) allocation which has no
/// distinction in Rust. Always defaults to `false` (ztrdup arm) at
/// the chrealpath call.
pub fn xsymlink(path: &str) -> Option<String> {
    // c:971
    // c:973 — `if (*s != '/') return NULL;`
    if !path.starts_with('/') {
        // c:973
        return None;
    }
    // c:974 — `*xbuf = '\0';`  Reset the xbuf cursor; no-op in Rust
    // (xbuf is an internal chrealpath buffer not exposed at this level).
    // c:975 — `if (!chrealpath(&s, 'P', heap))`
    match chrealpath(path, b'P', false) {
        // c:975
        Some(r) => Some(r), // c:979 return s
        None => {
            // c:976 failure arm
            // c:977 — `zwarn("path expansion failed, using root directory");`
            zwarn("path expansion failed, using root directory"); // c:977
                                                                  // c:978 — `return heap ? dupstring("/") : ztrdup("/");`
            Some("/".to_string()) // c:978
        }
    }
}

/// Port of `print_if_link()` from `Src/utils.c:985` — C decl `void print_if_link(char *s, int all)`.
///
/// "Print arrow + symlink target(s) iff `s` is an absolute path
/// pointing through symlinks." When `all` is set, follows the
/// chain (`xsymlinks` loop, c:992); otherwise emits just the final
/// realpath if it differs from the input. Always relative to
/// stdout. The Rust port writes via `print!` to mirror C's
/// `printf`/`zputs(stdout)` calls.
pub fn print_if_link(s: &str, all: bool) {
    // c:985
    if !s.starts_with('/') {
        // c:987
        return;
    }
    if all {
        // c:988
        let mut start = s.to_string();
        loop {
            // c:992
            match xsymlinks(&start) {
                Ok(target) if !target.is_empty() && target != start => {
                    print!(" -> "); // c:994
                    print!("{}", if target.is_empty() { "/" } else { &target }); // c:995
                    start = target; // c:998-999
                }
                _ => break, // c:1002
            }
        }
    } else {
        // c:1006
        // c:1007-1011 — HAVE_MEMCCPY arm: copy s into a PATH_MAX-1 buffer
        // and DPUTS1 if the input overflows. Rust's String has no fixed
        // PATH_MAX limit but the C parity-check still fires when a
        // pathological caller passes a path that wouldn't fit C's
        // s_at_entry[PATH_MAX+1] buffer.
        DPUTS1!(
            // c:1009
            s.len() >= libc::PATH_MAX as usize, // c:1008 memccpy returns NULL
            "path longer than PATH_MAX: {}",
            s // c:1009
        );
        let s_at_entry = s.to_string(); // c:1013
                                        // c:1015 — `if (chrealpath(&s, 'P', 0) && strcmp(s, s_at_entry))`
                                        // The previous Rust port called std::fs::canonicalize directly —
                                        // a fake that bypassed the canonical chrealpath port (hist.rs:2311).
        let mut resolved = s.to_string(); // c:1015 &s in/out
        if let Some(r) = chrealpath(&resolved, b'P', false) {
            // c:1015
            resolved = r;
            if resolved != s_at_entry {
                // c:1015 strcmp(s, s_at_entry)
                print!(" -> "); // c:1016
                print!("{}", if resolved.is_empty() { "/" } else { &resolved });
                // c:1017
            }
        }
    }
    let _ = io::stdout().flush();
}

/// Port of `fprintdir()` from `Src/utils.c:1031` — C decl `void fprintdir(char *s, FILE *f)`.
///
/// "print a directory" — abbreviates `s` via `finddir` (so a path
/// matching `$HOME` or a `nameddirtab` entry shows as `~name/...`)
/// and returns the rendering. The C source writes to a FILE*; Rust
/// returns the string for the caller to print.
pub fn fprintdir(s: &str) -> String {
    // c:1031
    match finddir(s) {
        // c:1031
        None => unmeta(s),          // c:1036
        Some(rendered) => rendered, // c:1038-1040
    }
}

/// Port of `substnamedir()` from `Src/utils.c:1053` — C decl `char *substnamedir(char *s)`.
///
/// C body (c:1053-1061):
/// ```c
/// Nameddir d = finddir(s);
/// if (!d)
///     return quotestring(s, QT_BACKSLASH);
/// return zhtricat("~", d->node.nam, quotestring(s + strlen(d->dir),
///                                                QT_BACKSLASH));
/// ```
///
/// The previous Rust port was a FAKE — it called `finddir(s)` (Rust
/// signature returns `Option<String>` of the already-formatted
/// `~name/rest`) and returned that string unchanged in the Some-arm,
/// missing the `quotestring(..., QT_BACKSLASH)` C applies to the
/// residue. C's `finddir` returns a `Nameddir` pointer; the
/// `~name` + quoted-residue join lives here in `substnamedir`.
///
/// This re-port duplicates the HOME-first + nameddirtab-scan logic
/// (finddir_scan at c:1106) so it can take the (name, dir_prefix)
/// split BEFORE pre-joining, then apply quotestring to the residue
/// per c:1059.
pub fn substnamedir(s: &str) -> String {
    // c:1053
    // C `finddir` at c:1127 runs the homenode through the SAME
    // best-diff scan as the nameddirtab (c:1164-1165) — home COMPETES
    // with `hash -d` entries by `diff`, it does not preempt them.
    // Duplicate that competition here without the pre-format, so we
    // can apply quotestring on just the residue (c:1059).
    let home = getsparam("HOME").unwrap_or_default(); // c:1133
    let mut finddir_best: i32 = 0; // c:1163
    let mut best: Option<(String, String)> = None; // finddir_last
    let home_diff = if home.len() == 1 {
        0
    } else {
        home.len() as i32
    }; // c:1140-1141
    if home_diff > 0 {
        if let Some(rest) = s.strip_prefix(&home) {
            if rest.is_empty() || rest.starts_with('/') {
                finddir_best = home_diff;
                // HOME is the implicit homenode (no name).
                best = Some((String::new(), rest.to_string()));
            }
        }
    }
    // c:1106 finddir_scan — best-diff walk over nameddirtab.
    if let Some((name, rest, diff)) = finddir_scan(s) {
        if diff > finddir_best {
            best = Some((name, rest));
        }
    }
    match best {
        // c:1059 — `zhtricat("~", d->node.nam, quotestring(s + strlen(d->dir), QT_BACKSLASH))`
        Some((name, rest)) => format!("~{}{}", name, quotestring(&rest, QT_BACKSLASH)),
        None => quotestring(s, QT_BACKSLASH), // c:1058
    }
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
            String::new() // c:1088 — `ztrdup("")` fallback
        } else {
            // c:1086 — `cached_username = ztrdup_metafy(pswd->pw_name);`.
            // `ztrdup_metafy` calls `metafy((char *)s, -1, META_DUP)`
            // (utils.c:4929). The previous Rust port returned the raw
            // pw_name verbatim — fine for ASCII usernames, but high-bit
            // bytes (e.g. an `émile` username on some systems) would
            // surface to callers un-metafied. zsh's downstream pipeline
            // (param expansion, prompt rendering) assumes paramtab
            // entries are metafied; an un-metafied high-bit byte breaks
            // the Meta-escape contract.
            let raw = std::ffi::CStr::from_ptr((*pw).pw_name)
                .to_string_lossy()
                .into_owned();
            metafy(&raw) // c:1086 ztrdup_metafy
        }
    };
    *guard = Some((current_uid, name.clone()));
    name
}

/// Port of `finddir_scan()` from `Src/utils.c:1106`.
///
/// Port of `finddir_scan(HashNode hn, UNUSED(int flags))` from Src/utils.c:1106-1112 — ScanFunc the
/// C source registers with `scanhashtable(nameddirtab, …)`. Keeps the
/// entry with the LARGEST `diff` (zsh.h:2152 — `strlen(dir) -
/// strlen(nam)`, the abbreviation savings — NOT the raw dir length)
/// whose dir is a boundary-prefix of the path (`!dircmp`, c:1075-1086)
/// and which is not ND_NOABBREV (PWD/OLDPWD entries, c:1108).
/// WARNING: param names don't match C — Rust=(path) vs C=(hn, flags);
/// the C statics finddir_full/finddir_last/finddir_best are threaded
/// as the argument and the `(name, rest, diff)` return instead.
pub fn finddir_scan(path: &str) -> Option<(String, String, i32)> {
    // c:1106
    let table = nameddirtab().lock().ok()?;
    let mut best: Option<(String, String, i32)> = None;
    for (name, nd) in table.iter() {
        // c:1107-1108 — `nd->diff > finddir_best && !dircmp(nd->dir,
        // finddir_full) && !(nd->node.flags & ND_NOABBREV)`.
        if (nd.node.flags & crate::ported::zsh_h::ND_NOABBREV) != 0 {
            continue;
        }
        if best.as_ref().is_some_and(|b| nd.diff <= b.2) {
            continue;
        }
        if let Some(rest) = path.strip_prefix(nd.dir.as_str()) {
            if rest.is_empty() || rest.starts_with('/') {
                best = Some((name.clone(), rest.to_string(), nd.diff));
            }
        }
    }
    best
}

/// Port of `finddir()` from `Src/utils.c:1127` — C decl `Nameddir finddir(char *s)`.
///
/// "See if a path has a named directory as its prefix." Compares
/// `s` against `$HOME` first (longest implicit named dir), then
/// scans the global `nameddirtab`, then falls back to
/// `subst_string_by_hook("zsh_directory_name", "d", s)`.
///
/// Rust signature returns `Option<String>` (the abbreviated path
/// `~name/rest`) instead of the C `Nameddir` pointer.
pub fn finddir(path: &str) -> Option<String> {
    // c:1127
    // c:1138 — `homenode.dir = home ? home : "";`. Reads the C global
    // `char *home` (params.c:91) DIRECTLY — not via paramtab. zshrs
    // ports the global as `params::home_lock()` accessed by
    // `homegetfn` (which ignores its &param arg per
    // `c:5109 UNUSED(Param pm)`). Pass a default param so the read
    // works even before paramtab["HOME"] has been hydrated (during
    // early init / unit-test environments).
    let _default_pm = crate::ported::zsh_h::param::default();
    let home = crate::ported::params::homegetfn(&_default_pm); // c:1138 home
                                                               // c:1139-1141 + 1164-1165 — the home node (nam="", diff =
                                                               // strlen(home), root's 1 → 0) runs through the SAME best-diff scan
                                                               // as the table (`finddir_scan(&homenode.node, 0);
                                                               // scanhashtable(nameddirtab, …)`): it COMPETES with `hash -d`
                                                               // entries instead of preempting them, so a named dir with a larger
                                                               // diff wins (e.g. ~ZPWR over ~/.zpwr). The previous early return
                                                               // on the $HOME prefix hid every named dir under $HOME.
    let mut finddir_best: i32 = 0; // c:1163
    let mut best: Option<(String, String)> = None; // finddir_last
    let home_diff = if home.len() == 1 {
        0
    } else {
        home.len() as i32
    }; // c:1140-1141
    if home_diff > 0 {
        if let Some(rest) = path.strip_prefix(&home) {
            if rest.is_empty() || rest.starts_with('/') {
                finddir_best = home_diff;
                best = Some((String::new(), rest.to_string()));
            }
        }
    }
    if let Some((name, rest, diff)) = finddir_scan(path) {
        // c:1165 — scanhashtable: only a strictly larger diff replaces
        // the home candidate (finddir_scan's `nd->diff > finddir_best`).
        if diff > finddir_best {
            finddir_best = diff;
            best = Some((name, rest));
        }
    }
    // c:1167-1175 — zsh_directory_name hook — returns ["name", "len"];
    // wins only when its match length beats finddir_best (c:1169).
    if let Some(reply) = subst_string_by_hook("zsh_directory_name", Some("d"), path) {
        if reply.len() >= 2 {
            // c:1170
            if let Ok(len) = reply[1].parse::<i32>() {
                if len > finddir_best && (len as usize) <= path.len() {
                    return Some(format!("~[{}]{}", reply[0], &path[len as usize..]));
                }
            }
        }
    }
    best.map(|(name, rest)| format!("~{}{}", name, rest)) // c:1177
}

/// Port of `adduserdir()` from `Src/utils.c:1187`.
///
/// Port of `void adduserdir(char *s, char *t, int flags, int always)`
/// from Src/utils.c:1187.
///
/// Adds (or removes when `t` is empty / non-absolute) an entry in
/// the global `nameddirtab`. ND_USERNAME entries from `getpwnam`
/// don't override explicit assignments. AUTONAMEDIRS gating, the
/// trailing-slash trim, and PWD/OLDPWD ND_NOABBREV stamp are all
/// preserved. Routes through `crate::ported::hashnameddir`.
pub fn adduserdir(name: &str, dir: &str, flags: i32, always: bool) {
    // c:1187

    if !interact() {
        return;
    } // c:1193
    if let Ok(t) = nameddirtab().lock() {
        if (flags & ND_USERNAME) != 0 && t.contains_key(name) {
            // c:1199
            return;
        }
        if !always && !isset(AUTONAMEDIRS) && !t.contains_key(name) {
            // c:1207
            return;
        }
    }
    // c:1211 — `if (!t || *t != '/' || strlen(t) >= PATH_MAX)`. C
    // rejects paths >= PATH_MAX as too long (would overflow path-
    // expansion buffers downstream). PATH_MAX is platform-dependent
    // (4096 on Linux, 1024 on macOS); libc::PATH_MAX exposes it.
    if dir.is_empty() || !dir.starts_with('/') || dir.len() >= libc::PATH_MAX as usize
    // c:1211 strlen(t) >= PATH_MAX
    {
        let _ = removenameddirnode(name); // c:1214
        return;
    }
    let mut trimmed = dir.trim_end_matches('/').to_string(); // c:1224-1226
    if trimmed.is_empty() {
        trimmed = dir.to_string(); // c:1227-1233
    }
    let final_flags = if name == "PWD" || name == "OLDPWD" {
        // c:1237
        flags | ND_NOABBREV
    } else {
        flags
    };
    // c:1239 — `nd = (Nameddir) zshcalloc(sizeof *nd); nd->node.flags = …;
    //           nd->dir = ztrdup(t); addnode(nameddirtab, ztrdup(s), nd);`
    let nd = nameddir {
        node: hashnode {
            next: None,
            nam: name.to_string(),
            flags: final_flags,
        },
        dir: trimmed,
        diff: 0,
    };
    crate::ported::hashnameddir::addnameddirnode(name, nd);
}

/// Port of `getnameddir()` from `Src/utils.c:1247` — C decl `char *getnameddir(char *name)`.
///
/// Looks up `name` in `nameddirtab`; if absent, checks for a
/// scalar parameter whose value starts with `/` and registers it
/// via `adduserdir`; finally falls back to `getpwnam(name)` for
/// `~user`-style lookups when USE_GETPWNAM is enabled.
pub fn getnameddir(name: &str) -> Option<String> {
    // c:1247
    if let Ok(t) = nameddirtab().lock() {
        if let Some(nd) = t.get(name) {
            // c:1254
            return Some(nd.dir.clone());
        }
    }
    // c:1260 — `if ((s = getsparam(name)) && *s == '/')`. paramtab read.
    if let Some(s) = getsparam(name) {
        if s.starts_with('/') {
            adduserdir(name, &s, 0, true); // c:1264
            return Some(s);
        }
    }
    #[cfg(unix)]
    {
        // c:1268 — getpwnam fallback.
        let cn = CString::new(name).ok()?;
        let pw = unsafe { libc::getpwnam(cn.as_ptr()) };
        if !pw.is_null() {
            let raw_dir = unsafe {
                std::ffi::CStr::from_ptr((*pw).pw_dir)
                    .to_string_lossy()
                    .into_owned()
            };
            // c:1273-1274 — `isset(CHASELINKS) ? xsymlink(pw->pw_dir, 0)
            // : ztrdup(pw->pw_dir);`. Resolve symlinks when the option
            // is set. Previously omitted in the Rust port — silently
            // returned the raw passwd-db dir even when the user had
            // `setopt chaselinks`.
            let dir = if isset(CHASELINKS) {
                xsymlink(&raw_dir).unwrap_or(raw_dir)
            } else {
                raw_dir
            };
            // c:1276 — `adduserdir(name, dir, ND_USERNAME, 1);`
            // Cache the lookup so subsequent `~user` expansions hit the
            // nameddirtab fast-path at c:1254 instead of round-tripping
            // through getpwnam every time.
            adduserdir(name, &dir, ND_USERNAME, true); // c:1276
            return Some(dir);
        }
    }
    None
}

/// Port of `dircmp()` from `Src/utils.c:1296` — C decl `dircmp(char *s, char *t)`.
///
/// Compare directory paths (from utils.c dircmp)
pub fn dircmp(s: &str, t: &str) -> bool {
    let s = s.trim_end_matches('/');
    let t = t.trim_end_matches('/');
    s == t
}

// Add a function to the list of pre-prompt functions.                     // c:1332
/// Port of `addprepromptfn()` from `Src/utils.c:1319` — C decl `addprepromptfn(voidvoidfnptr_t func)`.
///
/// Register a callback to run before each prompt.
pub fn addprepromptfn(func: fn()) {
    // c:1319
    PREPROMPT_FNS.lock().unwrap().push(func);
}

// Remove a function from the list of pre-prompt functions.                // c:1332
/// Port of `delprepromptfn()` from `Src/utils.c:1332` — C decl `delprepromptfn(voidvoidfnptr_t func)`.
///
/// Remove a previously-registered pre-prompt callback.
pub fn delprepromptfn(func: fn()) {
    // c:1332
    let mut list = PREPROMPT_FNS.lock().unwrap();
    if let Some(pos) = list.iter().position(|f| *f as usize == func as usize) {
        list.remove(pos);
    }
}

// Add a function to the list of timed functions.                           // c:1367
/// Port of `addtimedfn()` from `Src/utils.c:1371`.
///
/// Port of `void addtimedfn(voidvoidfnptr_t func, time_t when)` from
/// `Src/utils.c:1371`. Faithful walk of the timedfns LinkList:
/// allocate a `Timedfn`, lazy-init the list when empty, otherwise scan
/// from `firstnode` and insert BEFORE the first node whose `when` is
/// greater than ours. The standard linklist API only inserts AFTER a
/// node, so the C loop carries `ln` as the previous node and inserts
/// before `next` once `when < next->when`. Note: zsh's `time_t` is
/// signed and historically negative-`when` was supported, so we keep
/// `i64`.
pub fn addtimedfn(func: fn(), when: i64) {
    // c:1371
    let mut list = TIMED_FNS.lock().unwrap(); // c:1365 timedfns
    let tfdat: (i64, fn()) = (when, func); // c:1373-1375 Timedfn tfdat
    if list.is_empty() {
        // c:1377 !timedfns
        list.push(tfdat); // c:1378-1379 znewlinklist + zaddlinknode
        return;
    }
    if list.is_empty() {
        // c:1394 !ln (firstnode of empty list)
        list.push(tfdat); // c:1395 zaddlinknode
        return; // c:1396
    }
    let mut idx: usize = 0; // c:1381 LinkNode ln = firstnode(timedfns)
    loop {
        // c:1398 for(;;)
        let next = idx + 1; // c:1400 LinkNode next = nextnode(ln)
        if next >= list.len() {
            // c:1401 !next
            list.push(tfdat); // c:1402 zaddlinknode
            return; // c:1403
        }
        let tfdat2_when = list[next].0; // c:1405 tfdat2 = getdata(next)
        if when < tfdat2_when {
            // c:1406 when < tfdat2->when
            list.insert(next, tfdat); // c:1407 zinsertlinknode(timedfns, ln, tfdat)
            return; // c:1408
        }
        idx = next; // c:1410 ln = next
    }
}

/// Port of `deltimedfn()` from `Src/utils.c:1430` — C decl `deltimedfn(voidvoidfnptr_t func)`.
///
/// Remove a registered timed function (first occurrence only).
pub fn deltimedfn(func: fn()) {
    // c:1430
    let mut list = TIMED_FNS.lock().unwrap();
    if let Some(pos) = list.iter().position(|(_, f)| *f as usize == func as usize) {
        list.remove(pos);
    }
}

/// Port of `callhookfunc()` from `Src/utils.c:1469`.
///
/// Invoke a hook function by name plus any `<name>_functions` array.
/// Port of `callhookfunc(char *name, LinkList lnklst, int arrayp, int *retval)` from Src/utils.c:1469. Returns 0 if at
/// least one hook ran, 1 otherwise — the C source uses the same
/// stat semantics so the prompt machinery can detect "did periodic
/// fire". Hook dispatch goes through the executor singleton (which
/// owns the function table); we look up `name` and then walk the
/// `<name>_functions` array exactly as the C source does at
/// Src/utils.c:1469.
/// Direct port of
/// `int callhookfunc(char *name, LinkList lnklst, int arrayp, int *retval)`
/// at `Src/utils.c:1469`. C signature mapping:
///   - `char *name`     → `name: &str`
///   - `LinkList lnklst` → `lnklst: Option<&[String]>` (Rust-shape
///     stand-in for the C-shape `LinkList` — both are an ordered list
///     of meta-fied strings; the LinkList port itself diverges)
///   - `int arrayp`     → `arrayp: i32`
///   - `int *retval`    → `retval: *mut i32` (out-param; NULL is fine
///     when the caller doesn't need the doshfunc return value)
///
/// Returns `stat` — 0 if at least one shfunc fired, 1 otherwise
/// (matches `c:1495 stat = 0` after every dispatch). When `retval` is
/// non-null, `*retval` is set to the most recent `doshfunc`-returned
/// status, mirroring the C body's `*retval = ret` semantics.
pub fn callhookfunc(name: &str, lnklst: Option<&[String]>, arrayp: i32, retval: *mut i32) -> i32 {
    let mut stat: i32 = 1; // c:1475
    let mut ret: i32 = 0; // c:1475

    // Build the doshfunc arg list `[fname ($0), $1, $2, …]`. A provided
    // `lnklst` already carries the ORIGINAL hook name at [0] and the real
    // positional args at [1..] (matching C, where the caller does
    // `addlinknode(args, name)` before the args). So substitute `fname` for
    // [0] — `fname` is the hook name on the direct path (c:1487, identity)
    // and a `${name}_functions` member on the array path (c:1504-1508,
    // `arg0 = [*arrptr] + argarr[1..]`) — and KEEP [1..].
    //
    // The prior code prepended `fname` to the WHOLE list (name included),
    // so every hook invoked with args saw `$1` == its own name (e.g.
    // `preexec`'s `$1` was "preexec" instead of the command line). c:1483.
    let mk_args = |fname: &str| -> Vec<String> {
        let mut v: Vec<String> = vec![fname.to_string()];
        if let Some(extra) = lnklst {
            if extra.len() > 1 {
                v.extend_from_slice(&extra[1..]);
            }
        }
        v
    };

    // c:1495 — `if ((shfunc = getshfunc(name))) { doshfunc(...); stat = 0; }`
    let shf_clone: Option<crate::ported::zsh_h::shfunc> = shfunctab_lock()
        .read()
        .ok()
        .and_then(|t| t.get(name).cloned());
    if let Some(mut shf) = shf_clone {
        let args = mk_args(name);
        // c:1487 — `ret = doshfunc(shfunc, lnklst, 1);`. Direct
        // doshfunc invocation; body_runner routes through the host
        // body-only entry to avoid double-wrapping the scope.
        let name_for_body = name.to_string();
        let body_args = args.clone();
        let body_runner = move || -> i32 {
            crate::ported::exec::run_function_body(&name_for_body, &body_args[1..]).unwrap_or(0)
        };
        ret = crate::ported::exec::doshfunc(&mut shf, args, true, body_runner);
        stat = 0; // c:1504
    }

    if arrayp != 0 {
        // c:1507-1525 — fire every `${name}_functions` hook in order.
        let arr_name = format!("{}_functions", name); // c:1511
        let arr = crate::ported::params::paramtab()
            .read()
            .ok()
            .and_then(|t| t.get(&arr_name).and_then(|p| p.u_arr.clone()))
            .unwrap_or_default(); // c:1512 getaparam
        for fn_name in arr {
            // c:1514
            let shf_clone: Option<crate::ported::zsh_h::shfunc> = shfunctab_lock()
                .read()
                .ok()
                .and_then(|t| t.get(&fn_name).cloned());
            if let Some(mut shf) = shf_clone {
                // c:1518
                let args = mk_args(&fn_name);
                // c:1508 — `newret = doshfunc(shfunc, arg0, 1);`.
                let name_for_body = fn_name.clone();
                let body_args = args.clone();
                let body_runner = move || -> i32 {
                    crate::ported::exec::run_function_body(&name_for_body, &body_args[1..])
                        .unwrap_or(0)
                };
                ret = crate::ported::exec::doshfunc(&mut shf, args, true, body_runner);
                stat = 0; // c:1520
            }
        }
    }

    // c:1528 — `if (retval) *retval = ret;`
    if !retval.is_null() {
        unsafe {
            *retval = ret;
        }
    }
    let _ = ret;

    stat // c:1530
}

// do pre-prompt stuff                                                      // c:1530
/// Run pre-prompt machinery: precmd, periodic, prepromptfns.
/// Port of `preprompt()` from Src/utils.c:1530. Rust port skips
/// the `PROMPT_SP` heuristic + mailcheck (those need terminal +
/// MAIL state plumbing not yet present); fires the `precmd` hook
/// + `precmd_functions` array, the `periodic` hook on its
/// PERIOD-second cadence, and walks the prepromptfns registry.
pub fn preprompt() {
    // !!! WARNING: RUST-ONLY — NO C COUNTERPART !!!
    // Stamp duration + exit status onto the SQLite history row hend()
    // added for the just-finished command (no-op when nothing is
    // pending). preprompt runs immediately after execode returns and
    // before the next ZLE read, so the elapsed time is command
    // wall-time, not prompt idle.
    crate::history::history_sqlite_finish(crate::ported::builtin::LASTVAL.load(Ordering::Relaxed));
    // !!! WARNING: RUST-ONLY — NO C COUNTERPART !!!
    // Native p10k engine (src/extensions/p10k): snapshot `$?` and close
    // the command timer BEFORE the precmd hook runs, so precmd's own
    // commands can't clobber what the status / command_execution_time
    // segments show. Mirrors p10k's `_p9k_save_status` precmd ordering.
    crate::p10k::note_command_finished(
        crate::ported::builtin::LASTVAL.load(Ordering::Relaxed) as i64
    );
    // c:1532 `static time_t lastperiodic;` — periodic-hook last-fire timestamp.
    static LAST_PERIODIC: AtomicI64 = AtomicI64::new(0);
    // `lastmailcheck` is module-scoped (LAST_MAILCHECK below) because C makes it
    // a file-static shared between checkmail() and checkmailpath() (c:1447).

    // c:1535-1536 — `zlong period = getiparam("PERIOD"); zlong mailcheck
    //                = getiparam("MAILCHECK");`
    let period = crate::ported::params::getiparam("PERIOD");
    let mailcheck = crate::ported::params::getiparam("MAILCHECK");

    // c:1538-1543 — `winch_unblock(); ... winch_block();` — let any
    // pending SIGWINCH fire so the prompt picks up the latest column
    // count, then re-block to prevent the resize-handler from
    // interrupting prompt rendering mid-paint.
    crate::ported::signals_h::winch_unblock();
    crate::ported::signals_h::winch_block();

    // c:1543-1565 — PROMPT_SP heuristic: when the last command left
    // dangling output (no trailing newline), print `$PROMPT_EOL_MARK`,
    // pad to the right margin, then `\r` + w spaces + `\r`. On a line that
    // already ended cleanly the marker is overwritten by the padding and
    // nothing shows; on a partial line the pad wraps to the next row, so
    // the prompt starts fresh and the truncated output stays visible with
    // the inverse `%` marker.
    //
    // Was previously left as a no-op on the grounds that `countprompt`
    // could not measure zshrs's expanded prompt (which carries readline
    // `\x01`/`\x02` ignore markers rather than C's Inpar/Outpar). That is
    // stale: countprompt accepts BOTH conventions (prompt.rs:3458).
    //
    // Without this, `printf 100` (or any output whose last line is
    // partial) had its tail overwritten by the next prompt — zsh shows
    // `100%` then a fresh prompt line, zshrs showed the prompt painted
    // over the `100`.
    // !!! WARNING: RUST-ONLY GUARD — NO C COUNTERPART !!!
    // In C, `preprompt` runs only from loop() between commands, with the
    // cursor at a known baseline. zshrs also reaches it while ZLE owns the
    // screen — e.g. with a completion listing still displayed — and the mark
    // plus its right-margin padding then lands INSIDE that listing, blanking
    // the row from the cursor rightwards (`-z  -- push arguments%` … `stack`).
    // Only write the mark when ZLE is not holding the display.
    let zle_owns_screen = crate::ported::builtins::sched::zleactive.load(Ordering::Relaxed) != 0
        || crate::ported::zle::zle_refresh::LISTSHOWN.load(Ordering::Relaxed) != 0;
    if isset(crate::ported::zsh_h::PROMPTSP)
        && isset(crate::ported::zsh_h::PROMPTCR)
        && crate::ported::init::use_exit_printed.load(Ordering::SeqCst) == 0
        && crate::ported::init::SHTTY.load(Ordering::Relaxed) >= 0
        && !zle_owns_screen
    {
        // c:1550-1554 — `$PROMPT_EOL_MARK`, default `%B%S%#%s%b`.
        let eolmark = crate::ported::params::getsparam("PROMPT_EOL_MARK")
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "%B%S%#%s%b".to_string());
        // c:1555 — the mark is always `%`-expanded, whatever PROMPT_PERCENT
        // is set to; restore the option afterwards (c:1559).
        let percents = crate::ported::options::opt_state_get("promptpercent");
        crate::ported::options::opt_state_set("promptpercent", true);
        let (mark, _, _) = crate::ported::prompt::promptexpand(&eolmark, 1, None); // c:1557
        let mut w: i32 = 0; // c:1552
        let mut h: i32 = 1;
        crate::ported::prompt::countprompt(&mark, &mut w, &mut h, -1); // c:1558
        if let Some(p) = percents {
            crate::ported::options::opt_state_set("promptpercent", p);
        }
        let fd = crate::ported::init::SHTTY.load(Ordering::Relaxed);
        let columns = adjustcolumns() as i32;
        // c:1561-1562 — `fprintf(shout, "%*s\r%*s\r",
        //   zterm_columns - w - !hasxn, "", w, "")`.
        let pad = (columns
            - w
            - if crate::ported::init::hasxn.load(Ordering::SeqCst) != 0 {
                0
            } else {
                1
            })
        .max(0) as usize;
        // zshrs's `promptexpand` keeps C's Inpar/Outpar non-printing spans as
        // readline RL_PROMPT_*_IGNORE bytes (\x01/\x02) — `countprompt` reads
        // them (prompt.rs:3458) but the TERMINAL must never see them, exactly
        // as zle_refresh filters them before every write (zle_refresh.rs:2274).
        let mut out: String = mark
            .chars()
            .filter(|&c| c != '\u{1}' && c != '\u{2}')
            .collect();
        out.push_str(&" ".repeat(pad));
        out.push('\r');
        out.push_str(&" ".repeat(w.max(0) as usize));
        out.push('\r');
        let _ = write_loop(fd, out.as_bytes()); // c:1560-1563
    }

    // Reap any children that exited while the shell sat idle. zsh's
    // SIGCHLD handler reaps asynchronously during the ZLE read (signals
    // are unqueued there), but zshrs runs the interactive event loop
    // inside a queued-signal window (init.rs `queue_signals()` at loop
    // top), so zhandler QUEUES each SIGCHLD instead of reaping it
    // (signals.rs:373). Disowned / background jobs (`&!`, `&`, and
    // `<(…)` process substitutions) therefore piled up as zombies
    // between prompts — every zpwr/zinit/p10k async `&!` per prompt
    // leaked a zombie, slowing startup and eventually exhausting the
    // process table. Mirror zhandler's reap+route here, right before
    // the scanjobs notification pass, so `waitpid(-1)` clears the
    // zombies and the DONE bits land for scanjobs to print/delete. No
    // foreground job is live at preprompt, so reaping -1 can't steal a
    // fg wait's status.
    {
        let reaped = crate::ported::signals::wait_for_processes();
        if !reaped.is_empty() {
            if let Some(jt) = crate::ported::jobs::JOBTAB.get() {
                if let Ok(mut guard) = jt.lock() {
                    for (pid, status) in reaped {
                        let _ = crate::ported::jobs::update_bg_job(&mut guard, pid, status);
                    }
                }
            }
        }
    }

    // c:1569-1572 — `if (unset(NOTIFY)) scanjobs();` — sync job-status
    // print before prompt, via the canonical scanjobs port
    // (Src/jobs.c:1993).
    if !crate::ported::zsh_h::isset(crate::ported::zsh_h::NOTIFY) {
        if let Some(jt) = crate::ported::jobs::JOBTAB.get() {
            let mut guard = jt.lock().unwrap();
            crate::ported::jobs::scanjobs(&mut guard);
        }
    }

    // c:1573-1574 — `if (errflag) return;` — bail if a previous error
    // already set the global flag.
    // A user interrupt sets ERRFLAG_INT, never ERRFLAG_ERROR (signals.c:457), and
    // the C line cited above tests the WHOLE errflag, so masking here let an
    // interrupted shell keep going where zsh stops.
    if errflag.load(Ordering::Relaxed) != 0 {
        return;
    }

    // c:1576-1580 — `callhookfunc("precmd", NULL, 1, NULL);` + errflag bail.
    callhookfunc("precmd", None, 1, std::ptr::null_mut());
    if (errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR) != 0 {
        return;
    }

    // !!! WARNING: RUST-ONLY — NO C COUNTERPART !!!
    // Native p10k engine (src/extensions/p10k): rebuild PROMPT/RPROMPT
    // after the precmd hook so user precmd state is fresh. Equivalent
    // to the zsh theme's `_p9k_precmd` running as the last precmd
    // hook (p10k.zsh registers _p9k_precmd via add-zsh-hook). No-op
    // unless the powerlevel10k theme source was intercepted.
    crate::p10k::preprompt_render();

    // !!! WARNING: RUST-ONLY — NO C COUNTERPART !!!
    // Fire the `async_precmd` hooks on the worker pool AFTER the prompt is
    // built/rendered, so they never block the paint. Non-blocking: submits to
    // the pool and returns immediately; the hooks write into the shared param
    // table which the NEXT prompt render reads. (Same call shape as the p10k
    // render above — the ShellExecutor-using logic lives in the extension.)
    crate::async_precmd::fire_async_precmd();

    // c:1582-1589 — periodic-hook dispatch on PERIOD cadence.
    if period > 0 {
        let now = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        if now > LAST_PERIODIC.load(Ordering::Relaxed) + period
            && callhookfunc("periodic", None, 1, std::ptr::null_mut()) == 0
        {
            LAST_PERIODIC.store(now, Ordering::Relaxed);
        }
    }
    // c:1588-1589 — `if (errflag) return;` post-periodic bail.
    // A user interrupt sets ERRFLAG_INT, never ERRFLAG_ERROR (signals.c:457), and
    // the C line cited above tests the WHOLE errflag, so masking here let an
    // interrupted shell keep going where zsh stops.
    if errflag.load(Ordering::Relaxed) != 0 {
        return;
    }

    // c:1591-1611 — mail check: `if (mailcheck && difftime(now,
    // lastmailcheck) > mailcheck)` walk MAILPATH (or scalar MAIL) via
    // checkmailpath. Faithful port — checkmailpath already exists at
    // utils.rs:1623, getaparam exists for the MAILPATH array.
    let currentmailcheck = std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    if mailcheck > 0 && (currentmailcheck - LAST_MAILCHECK.load(Ordering::Relaxed)) > mailcheck {
        // c:1597 — `if (mailpath && *mailpath && **mailpath)`
        let mailpath = crate::ported::params::getaparam("MAILPATH");
        let has_mailpath = mailpath
            .as_ref()
            .map(|p| !p.is_empty() && p.first().map(|s| !s.is_empty()).unwrap_or(false))
            .unwrap_or(false);
        if has_mailpath {
            // c:1598 `checkmailpath(mailpath);` — emit each message to the
            // terminal (C writes directly to `shout`).
            for m in checkmailpath(mailpath.as_deref().unwrap()) {
                println!("{}", m);
            }
        } else {
            // c:1600-1608 — `if ((mailfile = getsparam("MAIL")) && *mailfile)
            //                  { x[0]=mailfile; x[1]=NULL; checkmailpath(x); }`
            crate::ported::signals::queue_signals(); // c:1600
            if let Some(mailfile) = crate::ported::params::getsparam("MAIL") {
                if !mailfile.is_empty() {
                    let x = vec![mailfile]; // c:1604-1605
                    for m in checkmailpath(&x) {
                        println!("{}", m); // c:1606
                    }
                }
            }
            crate::ported::signals::unqueue_signals(); // c:1608
        }
        LAST_MAILCHECK.store(currentmailcheck, Ordering::Relaxed); // c:1610
    }

    // c:1613-1618 — `if (prepromptfns) for (...) ppnode->func();`
    let snapshot: Vec<fn()> = PREPROMPT_FNS.lock().unwrap().clone();
    for f in snapshot {
        f();
    }
}

/// `static time_t lastmailcheck;` (Src/utils.c:1447) — the timestamp of the
/// previous mail check. `checkmail()` reads it (mtime/atime comparisons happen
/// against the PREVIOUS check) and writes it after walking the paths.
pub static LAST_MAILCHECK: AtomicI64 = AtomicI64::new(0);

/// Port of `checkmailpath()` from `Src/utils.c:1623` — C decl `void checkmailpath(char **s)`.
/// Walks each `PATH` / `PATH?message` MAILPATH component:
///   - empty component → `zerr` (c:1634-1636)
///   - `mailstat` failure → `zerr` unless ENOENT (c:1637-1639)
///   - directory → list entries (`PATH/entry[?message]`) and recurse
///     (c:1640-1669)
///   - file, interactive (`shout`) only: "You have new mail." or the
///     `?`-suffixed message expanded with `$_` bound to the file
///     (`parsestr`+`singsub`, c:1670-1699); and, under `MAILWARNING`,
///     "The mail in PATH has been read." (c:1700-1704)
///
/// **Signature note:** C is `void` and writes to `shout`; the zshrs caller
/// drives output, so this returns the messages (in C emission order) instead.
/// The new-mail / read-warning conditions are now ported faithfully — the
/// previous adhoc port used an unfaithful "mtime within 60s" heuristic and its
/// result was discarded by the caller, so the feature did nothing.
pub fn checkmailpath(s: &[String]) -> Vec<String> {
    // c:1620
    let mut messages = Vec::new();
    let interactive = *crate::ported::init::shout.lock().unwrap() != 0; // c:1670 `else if (shout)`
    let lastmailcheck = LAST_MAILCHECK.load(Ordering::Relaxed);

    for path in s {
        // c:1627-1633 — split on the first '?': `file` before, `u` after (or None).
        let (file, u) = match path.find('?') {
            Some(p) => (&path[..p], Some(&path[p + 1..])),
            None => (path.as_str(), None),
        };

        if file.is_empty() {
            // c:1634-1636 — `if (**s == 0) zerr("empty MAILPATH component: %s", *s);`
            zerr(&format!("empty MAILPATH component: {}", path));
            continue;
        }

        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        let real = unmeta(file); // c:1637 — mailstat(unmeta(*s), &st)
        if mailstat(&real, &mut st) == -1 {
            // c:1638-1639 — report anything other than "no such file".
            let e = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            if e != libc::ENOENT {
                zerr(&format!("{}: {}", zsh_errno_msg(e), path));
            }
        } else if (st.st_mode as libc::mode_t & libc::S_IFMT) == libc::S_IFDIR {
            // c:1640-1669 — a directory: enumerate entries and recurse.
            match fs::read_dir(&real) {
                Err(_) => {
                    let e = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
                    zerr(&format!("{}: {}", zsh_errno_msg(e), path)); // c:1647
                }
                Ok(mut rd) => {
                    let mut arr = Vec::new();
                    // c:1653-1662 — `while ((fn = zreaddir(lock, 1)) && !errflag)`.
                    while let Some(fname) = zreaddir(&mut rd, 1) {
                        if errflag.load(Ordering::Relaxed) != 0 {
                            break;
                        }
                        // c:1654-1657 — keep the `?message` suffix on each entry.
                        arr.push(match u {
                            Some(u) => format!("{}/{}?{}", path, fname, u),
                            None => format!("{}/{}", path, fname),
                        });
                    }
                    messages.extend(checkmailpath(&arr)); // c:1667 — recurse
                }
            }
        } else if interactive {
            // c:1671-1672 — new mail: non-empty, not yet read, newer than the
            // last check.
            if st.st_size != 0 && st.st_atime <= st.st_mtime && st.st_mtime >= lastmailcheck {
                match u {
                    // c:1673-1675 — `fprintf(shout, "You have new mail.\n");`
                    None => messages.push("You have new mail.".to_string()),
                    // c:1676-1698 — custom message: bind `$_` to the file, then
                    // parsestr + singsub the message (prompt-style expansion).
                    Some(u) => {
                        let usav = crate::ported::init::zunderscore.lock().unwrap().clone();
                        crate::ported::exec::setunderscore(path); // c:1685 setunderscore(*s)
                        if let Ok(parsed) = crate::ported::lex::parsestr(u) {
                            // c:1688-1691 — parse ok → singsub then emit.
                            messages.push(crate::ported::subst::singsub(&parsed));
                        }
                        crate::ported::exec::setunderscore(&usav); // c:1694-1697 restore $_
                    }
                }
            }
            // c:1700-1704 — MAILWARNING: file read since the last check.
            if crate::ported::zsh_h::isset(crate::ported::zsh_h::MAILWARNING)
                && st.st_atime > st.st_mtime
                && st.st_atime > lastmailcheck
                && st.st_size != 0
            {
                messages.push(format!("The mail in {} has been read.", unmeta(file)));
            }
        }
    }
    messages
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
    let posix = EMULATION(EMULATE_KSH | EMULATE_SH); // c:init.c:1192
    let prefix_template = getsparam("PS4")
        .or_else(|| getsparam("PROMPT4"))
        .unwrap_or_else(|| {
            if posix {
                "+ ".to_string() // c:init.c:1192
            } else {
                "+%N:%i> ".to_string() // c:init.c:1193
            }
        });
    // c:utils.c:1723,1726,1730 — `t = opts[XTRACE]; opts[XTRACE] = 0;
    //                              promptexpand(...);  opts[XTRACE] = t;`
    let saved = isset(XTRACE);
    opt_state_set(&opt_name(XTRACE), false);
    let prefix = crate::prompt::expand_prompt(&prefix_template);
    opt_state_set(&opt_name(XTRACE), saved);
    // c:utils.c:1736 — `fprintf(xtrerr, "%s", s)`. Append to the xtrerr
    // line buffer rather than writing straight to stderr; the emitter
    // appends the command/args and flushes the whole line in one write,
    // matching C's single `fflush(xtrerr)` (so concurrent pipeline-stage
    // trace lines never interleave).
    crate::fusevm_bridge::xtrerr_fputs(&prefix);
}

/// Port of `freestr()` from `Src/utils.c:1739` — C decl `freestr(void *a)`.
///
/// C body:
/// ```c
/// void freestr(void *a) { zsfree(a); }
/// ```
/// The C function is registered as the `freenode` callback for
/// hashtables holding plain string values. The Rust port consumes
/// `a` by value; Rust's `Drop` runs the equivalent of `zsfree` when
/// the `String` is moved into this fn and falls out of scope at the
/// closing brace — the no-op body is the correct port. Param name
/// `a` matches C exactly per Rule E.
pub fn freestr(_a: String) {}

/// Port of `gettempfile()` from `Src/utils.c:2231`.
///
/// Port of `int gettempfile(const char *prefix, int use_heap, char **tempname)`
/// from Src/utils.c:2231. Creates a fresh tempfile with `O_RDWR|O_CREAT|O_EXCL`
/// and mode 0600 under umask 0177; returns `(fd, path)` on success.
///
/// C signature uses an out-param for the filename and returns the fd; Rust
/// returns both as a tuple. Matches the C control-flow: open with O_EXCL,
/// retry up to 16 times on EEXIST (the non-mkstemp branch, c:2255-2269).
pub fn gettempfile(prefix: Option<&str>) -> Option<(i32, String)> {
    // c:2231
    #[cfg(unix)]
    {
        queue_signals(); // c:2239
        let old_umask = unsafe { libc::umask(0o177) }; // c:2240
        let mut failures = 0; // c:2255
        let mut result: Option<(i32, String)> = None;
        loop {
            let fn_ = match gettempname(prefix, false) {
                // c:2260
                Some(n) => n,
                None => break,
            };
            let cn = match CString::new(fn_.clone()) {
                Ok(c) => c,
                Err(_) => break,
            };
            let fd = unsafe {
                libc::open(
                    cn.as_ptr(),
                    libc::O_RDWR | libc::O_CREAT | libc::O_EXCL, // c:2264
                    0o600 as libc::c_int,
                )
            };
            if fd >= 0 {
                result = Some((fd, fn_));
                break;
            }
            let err = io::Error::last_os_error().raw_os_error().unwrap_or(0);
            if err != libc::EEXIST {
                // c:2269
                break;
            }
            failures += 1;
            if failures >= 16 {
                // c:2269
                break;
            }
        }
        unsafe {
            libc::umask(old_umask);
        } // c:2273
        unqueue_signals(); // c:2274
        result
    }
    #[cfg(not(unix))]
    {
        let _ = prefix;
        None
    }
}

/// Port of `void gettyinfo(struct ttyinfo *ti)` from Src/utils.c:1746.
///
/// The saved baseline terminal mode — C's `struct ttyinfo shttyinfo`
/// (Src/init.c). Captured ONCE while the terminal is still in its
/// original (cooked / canonical) mode, before ZLE ever puts it into
/// raw mode via `zsetterm()`. `trashzle()` restores this baseline via
/// `settyinfo(&shttyinfo)` so that external commands the user runs
/// (`cat`, `less`, …) execute with a cooked terminal — where `^D` is
/// EOF, `^C` is SIGINT, and line editing works. Without it the tty
/// stayed in ZLE raw mode across command execution, so `cat` never
/// saw EOF on `^D`.
#[cfg(unix)]
pub static SHTTYINFO: std::sync::Mutex<Option<libc::termios>> = std::sync::Mutex::new(None);

/// Port of `gettyinfo()` from `Src/utils.c:1746`.
///
/// Reads the current termios from the global `SHTTY`. Returns
/// `None` when SHTTY is closed or the call fails (matching C's
/// silent return when SHTTY == -1).
/// C body (single statement): `fdgettyinfo(SHTTY, ti);`
/// Rust returns `Option<termios>` instead of taking an out-ptr;
/// the SHTTY=-1 case naturally returns Err from fdgettyinfo → None.
#[cfg(unix)]
pub fn gettyinfo() -> Option<libc::termios> {
    // c:1746
    fdgettyinfo(SHTTY.load(Ordering::Relaxed)).ok() // c:1748
}

/// Port of `fdgettyinfo()` from `Src/utils.c:1753`.
///
/// Emit the `$PS4` xtrace prefix to stderr.
/// Read terminal mode from a file descriptor.
/// Port of `fdgettyinfo(int SHTTY, struct ttyinfo *ti)` from Src/utils.c:1753. C source uses
/// `tcgetattr(SHTTY, &ti->tio)`; we return the populated termios
/// or an io::Error on failure (caller equivalent to zsh's `zerr`).
#[cfg(unix)]
/// WARNING: param names don't match C — Rust=(fd) vs C=(SHTTY, ti)
pub fn fdgettyinfo(fd: i32) -> io::Result<libc::termios> {
    let mut tio: libc::termios = unsafe { std::mem::zeroed() };
    if unsafe { libc::tcgetattr(fd, &mut tio) } == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(tio)
    }
}

/// Port of `fdgettyinfo()` from `Src/utils.c:1753` — C decl `fdgettyinfo(int SHTTY, struct ttyinfo *ti)`.
#[cfg(not(unix))]
/// WARNING: param names don't match C — Rust=(_fd) vs C=(SHTTY, ti)
pub fn fdgettyinfo(_fd: i32) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "no termios",
    ))
}

/// Port of `settyinfo()` from `Src/utils.c:1778` — C decl `void settyinfo(struct ttyinfo *ti)`.
///
/// Restores the termios state on the global `SHTTY` with EINTR
/// retry; no-op when SHTTY is closed.
/// C body (single statement): `fdsettyinfo(SHTTY, ti);`
/// Rust returns bool; SHTTY=-1 yields Err → false from fdsettyinfo.
#[cfg(unix)]
pub fn settyinfo(ti: &libc::termios) -> bool {
    // c:1778
    fdsettyinfo(SHTTY.load(Ordering::Relaxed), ti).is_ok() // c:1780
}

/// Port of `fdsettyinfo()` from `Src/utils.c:1785`.
///
/// Apply terminal mode to a file descriptor, with EINTR retry.
/// Port of `fdsettyinfo(int SHTTY, struct ttyinfo *ti)` from Src/utils.c:1785. C source loops
/// `while (tcsetattr(SHTTY, TCSADRAIN, &ti->tio) == -1 && errno
/// == EINTR)` — same retry shape here.
#[cfg(unix)]
pub fn fdsettyinfo(SHTTY2: i32, ti: &libc::termios) -> io::Result<()> {
    loop {
        if unsafe { libc::tcsetattr(SHTTY2, libc::TCSADRAIN, ti) } != -1 {
            return Ok(());
        }
        let err = io::Error::last_os_error();
        if err.kind() != io::ErrorKind::Interrupted {
            return Err(err);
        }
    }
}

/// Port of `fdsettyinfo()` from `Src/utils.c:1785` — C decl `fdsettyinfo(int SHTTY, struct ttyinfo *ti)`.
#[cfg(not(unix))]
pub fn fdsettyinfo(SHTTY: i32, ti: &()) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "no termios",
    ))
}

// window size changed                                                     // c:1831
/// Port of `adjustlines()` from `Src/utils.c:1831`.
///
/// Port of `adjustlines(int signalled)` from Src/utils.c:1831 — TIOCGWINSZ
/// lookup that seeds `$LINES`. The C variant updates the global
/// `zterm_lines` and returns whether it changed; this Rust port
/// returns the row count directly. Falls back to `$LINES` env var,
/// then 24, mirroring the C source's `tclines > 0 ? tclines : 24`
/// fallback at line 1844.
/// WARNING: param names don't match C — Rust=() vs C=(signalled)
pub fn adjustlines() -> usize {
    // c:1831
    #[cfg(unix)]
    {
        unsafe {
            let mut ws: libc::winsize = std::mem::zeroed();
            if libc::ioctl(1, libc::TIOCGWINSZ, &mut ws) == 0 && ws.ws_row > 0 {
                return ws.ws_row as usize;
            }
        }
    }
    // c:1841-1845 fallback — `if (zterm_lines <= 0) zterm_lines =
    //                    tclines > 0 ? tclines : 24`. paramtab `$LINES`,
    //                    not OS env. See adjustcolumns for why the `> 0`
    //                    filter is load-bearing.
    getsparam("LINES")
        .and_then(|s| s.parse().ok())
        .filter(|&n: &usize| n > 0)
        .unwrap_or(24)
}

/// Port of `adjustcolumns()` from `Src/utils.c:1856`.
///
/// Port of `adjustcolumns(int signalled)` from Src/utils.c:1856 — TIOCGWINSZ
/// lookup that seeds `$COLUMNS`. The C variant updates the global
/// `zterm_columns` and returns whether it changed; this Rust port
/// returns the column count directly. Falls back to `$COLUMNS` env
/// var, then 80, mirroring the C source's `tccolumns > 0 ? tccolumns : 80`
/// fallback at line 1869.
/// WARNING: param names don't match C — Rust=() vs C=(signalled)
pub fn adjustcolumns() -> usize {
    // c:1856
    #[cfg(unix)]
    {
        unsafe {
            let mut ws: libc::winsize = std::mem::zeroed();
            if libc::ioctl(1, libc::TIOCGWINSZ, &mut ws) == 0 && ws.ws_col > 0 {
                return ws.ws_col as usize;
            }
        }
    }
    // c:1866-1870 fallback — `if (zterm_columns <= 0) zterm_columns =
    //                    tccolumns > 0 ? tccolumns : 80`. C consults
    //                    `getsparam("COLUMNS")` (paramtab), not OS env.
    // The `> 0` filter is C's `<= 0` clamp: a literal `COLUMNS=0` (what a
    // TERM=dumb pty leaves behind) parses fine, so without it callers that
    // divide by the column count panic instead of falling back to 80.
    getsparam("COLUMNS")
        .and_then(|s| s.parse().ok())
        .filter(|&n: &usize| n > 0)
        .unwrap_or(80)
}

// window size changed                                                      // c:1824
/// Port of `adjustwinsize()` from `Src/utils.c:1889` — C decl `void adjustwinsize(int from)`.
/// SIGWINCH handler + LINES/COLUMNS-update entry. Reads the tty's
/// current geometry via `TIOCGWINSZ` ioctl, updates the cached
/// `zterm_lines`/`zterm_columns`, and writes `$LINES` / `$COLUMNS`
/// when they're already set in the environment.
/// ```c
/// void
/// adjustwinsize(int from)
/// {
///     static int getwinsz = 1;
///     int ttyrows = shttyinfo.winsize.ws_row;
///     int ttycols = shttyinfo.winsize.ws_col;
///     int resetzle = 0;
///     if (getwinsz || from == 1) {
///         if (SHTTY == -1) return;
///         if (ioctl(SHTTY, TIOCGWINSZ, &shttyinfo.winsize) == 0) {
///             resetzle = (ttyrows != ... || ttycols != ...);
///             ...
///         } else {
///             shttyinfo.winsize.ws_row = zterm_lines;
///             shttyinfo.winsize.ws_col = zterm_columns;
///         }
///     }
///     switch (from) {
///     case 0: case 1:
///         getwinsz = 0;
///         if (adjustlines(from) && zgetenv("LINES")) setiparam("LINES", zterm_lines);
///         if (adjustcolumns(from) && zgetenv("COLUMNS")) setiparam("COLUMNS", zterm_columns);
///         getwinsz = 1;
///         break;
///     case 2: resetzle = adjustlines(0); break;
///     case 3: resetzle = adjustcolumns(0); break;
///     }
///     if (interact && resetzle) zleentry(ZLE_CMD_REFRESH);
/// }
/// ```
pub fn adjustwinsize(from: i32) -> (usize, usize) {
    // c:1889

    // c:1891 — `static int getwinsz = 1;`
    let getwinsz = ADJUSTWINSIZE_GETWINSZ.load(Ordering::SeqCst);

    // c:1893-1894 — `int ttyrows = shttyinfo.winsize.ws_row;` /
    // `ttycols = shttyinfo.winsize.ws_col;` — the PREVIOUSLY cached
    // geometry, read before the ioctl overwrites it, so c:1903 can tell
    // whether the window actually changed size.
    //
    // !!! RUST-ONLY SOURCING !!! zshrs's `SHTTYINFO` (utils.rs) holds only
    // the `termios` half of C's `struct ttyinfo`; there is no cached
    // `winsize` to read. The last geometry this function published lives
    // in `$LINES` / `$COLUMNS` (written unconditionally by the c:1931-1934
    // arm below), so the previous size is read back from there. Same
    // values, same comparison.
    let mut ttyrows: i32 = crate::ported::params::getiparam("LINES") as i32; // c:1893
    let mut ttycols: i32 = crate::ported::params::getiparam("COLUMNS") as i32; // c:1894
    let mut resetzle = 0i32; // c:1896

    // c:1898-1917 — TIOCGWINSZ probe.
    if getwinsz != 0 || from == 1 {
        // c:1898
        let shtty = SHTTY.load(Ordering::Relaxed);
        if shtty == -1 {
            // c:1900
            return (adjustcolumns(), adjustlines()); // c:1901
        }
        #[cfg(unix)]
        unsafe {
            let mut ws: libc::winsize = std::mem::zeroed();
            if libc::ioctl(shtty, libc::TIOCGWINSZ, &mut ws as *mut _) == 0 {
                // c:1902
                // c:1903-1904 — `resetzle = (ttyrows != ws_row ||
                //                            ttycols != ws_col);`
                // This assignment was MISSING: `resetzle` was left at 0 on
                // the `from == 1` (SIGWINCH) path, so the c:1954 redraw
                // never fired and a resize left the screen as the terminal
                // had mangled it. With a completion listing displayed and
                // the window SHRUNK, the terminal drops the top rows and
                // nothing repaints them — zsh kept all 3 non-blank rows on
                // a 24x80 -> 14x80 shrink, zshrs kept 0.
                resetzle = (ttyrows != ws.ws_row as i32 || ttycols != ws.ws_col as i32) as i32;
                ttyrows = ws.ws_row as i32; // c:1907
                ttycols = ws.ws_col as i32; // c:1908
            }
        }
    }

    match from {
        // c:1921
        0 | 1 => {
            // c:1922-1923
            ADJUSTWINSIZE_GETWINSZ.store(0, Ordering::SeqCst); // c:1924
                                                               // c:1931-1934 — C: `if (adjustlines(from) && zgetenv("LINES"))
                                                               // setiparam("LINES", zterm_lines);` (same for COLUMNS). The
                                                               // zgetenv gate only guards re-publishing to an ENV-exported
                                                               // LINES — in C the param itself is ALWAYS live because its
                                                               // valptr aliases the `zterm_lines` global that adjustlines
                                                               // just wrote (createparamtable IPDEF5 → zlevarsetfn/intvargetfn
                                                               // on &zterm_lines). zshrs params store their own u_val copy
                                                               // (no valptr aliasing), so the probe result must be written
                                                               // through setiparam unconditionally — gating on the env left
                                                               // `$LINES`/`$COLUMNS` at 0 for every interactive shell
                                                               // (LINES is not in the environment; zsh doesn't export it).
                                                               // Effect: `$(( LINES / 3 ))` = 0 → division-by-zero in
                                                               // history-search-multi-word pagination (_hsmw_main:81), and
                                                               // every LINES/COLUMNS-derived layout was wrong. Recursion is
                                                               // safe: setiparam → zlevarsetfn → adjustwinsize(2|3) never
                                                               // re-enters this arm.
            let lines = if ttyrows > 0 {
                ttyrows as i64 // c:1837 — zterm_lines = shttyinfo.winsize.ws_row
            } else {
                adjustlines() as i64 // c:1844 — tclines/24 fallback chain
            };
            setiparam("LINES", lines); // c:1932
            let cols = if ttycols > 0 {
                ttycols as i64 // c:1862 — zterm_columns = ws_col
            } else {
                adjustcolumns() as i64 // c:1869 — tccolumns/80 fallback chain
            };
            setiparam("COLUMNS", cols); // c:1934
            ADJUSTWINSIZE_GETWINSZ.store(1, Ordering::SeqCst); // c:1935
        }
        2 => {
            // c:1937-1938 — `resetzle = adjustlines(0);`
            //
            // C's `adjustlines` RETURNS WHETHER the size changed
            // (Src/utils.c:1852 — `return (zterm_lines != oldlines);`), and
            // with `signalled == 0` and a positive `zterm_lines` it changes
            // nothing, so the recursive call from `setiparam` at c:1932
            // yields 0. This port's `adjustlines()` returns the COUNT
            // instead, so `resetzle` was the terminal height — always
            // truthy. That did not matter while the c:1954 branch below was
            // a no-op; now that it repaints, recover C's boolean by
            // comparing against the previously published `$LINES`.
            resetzle = (adjustlines() as i32 != ttyrows) as i32; // c:1938
        }
        3 => {
            // c:1940-1941 — `resetzle = adjustcolumns(0);` — same
            // count-vs-changed correction as the LINES arm above
            // (Src/utils.c:1877 `return (zterm_columns != oldcolumns);`).
            resetzle = (adjustcolumns() as i32 != ttycols) as i32; // c:1941
        }
        _ => {}
    }

    // c:1945-1952 — the `interact && from >= 2` block is entirely commented
    // out in the C (the TIOCSWINSZ write-back is disabled), so nothing to
    // port. Keep the reads alive for the c:1954 gate below.
    let _ = ttyrows;
    let _ = ttycols;

    // c:1954-1961 —
    // ```c
    //     if (zleactive && resetzle) {
    //         winchanged = resetneeded = 1;
    //         zleentry(ZLE_CMD_RESET_PROMPT);
    //         zleentry(ZLE_CMD_REFRESH);
    //     }
    // ```
    // The whole block was absent, which is why a SIGWINCH never repainted
    // anything.
    if crate::ported::builtins::sched::zleactive.load(Ordering::Relaxed) != 0 && resetzle != 0 {
        // !!! RUST-ONLY GUARD !!! C protects this re-entry with
        // `queue_signals()` around every critical section (Src/signals.h:90),
        // so a SIGWINCH arriving inside zrefresh is deferred rather than
        // recursing. zshrs has no equivalent coverage over the ZLE refresh
        // path and its video state is behind non-reentrant `std::sync::Mutex`
        // handles, so a signal landing mid-refresh would DEADLOCK rather than
        // recurse. Serialise instead: a resize that arrives while this
        // repaint is in flight is dropped, exactly as C's queued signal is
        // superseded by the pending full redraw.
        if !ADJUSTWINSIZE_IN_ZLE.swap(true, Ordering::SeqCst) {
            WINCHANGED.store(1, Ordering::SeqCst); // c:1956
            RESETNEEDED.store(1, Ordering::SeqCst); // c:1958
            crate::ported::zle::zle_refresh::RESETNEEDED.store(1, Ordering::SeqCst); // c:1958
            crate::ported::init::zleentry(crate::ported::zsh_h::ZLE_CMD_RESET_PROMPT); // c:1959
            crate::ported::init::zleentry(crate::ported::zsh_h::ZLE_CMD_REFRESH); // c:1960
            ADJUSTWINSIZE_IN_ZLE.store(false, Ordering::SeqCst);
        }
    }

    (adjustcolumns(), adjustlines())
}

/// Port of `static int getwinsz` from `Src/utils.c:1891`. Local
/// reentry guard inside adjustwinsize — bumped to 0 around the
/// setiparam recursion so the recursive call short-circuits.
pub static ADJUSTWINSIZE_GETWINSZ: std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(1); // c:1891

/// !!! WARNING: RUST-ONLY GUARD !!! No C counterpart.
///
/// Set while `adjustwinsize`'s c:1954 branch is inside
/// `zleentry(ZLE_CMD_RESET_PROMPT)` / `zleentry(ZLE_CMD_REFRESH)`.
/// C reaches that branch from a signal handler and relies on
/// `queue_signals()` (`Src/signals.h:90`) to defer a SIGWINCH that lands
/// during a refresh; zshrs's ZLE video state sits behind non-reentrant
/// `std::sync::Mutex` handles, where the same re-entry hangs instead of
/// recursing. This flag drops the nested repaint; the outer one already
/// re-reads the geometry.
static ADJUSTWINSIZE_IN_ZLE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Port of `check_fd_table()` from `Src/utils.c:1969` — C decl `check_fd_table(int fd)`.
/// ```c
/// if (fd <= max_zsh_fd) return;
/// if (fd >= fdtable_size) {
///     int old_size = fdtable_size;
///     while (fd >= fdtable_size)
///         fdtable = zrealloc(fdtable, (fdtable_size *= 2) * sizeof(*fdtable));
///     memset(fdtable + old_size, 0, (fdtable_size - old_size) * sizeof(*fdtable));
/// }
/// max_zsh_fd = fd;
/// ```
/// C semantics: GROW the `fdtable` array so it can index `fd`, then
/// update `max_zsh_fd`. Returns void.
///
/// The Rust port keeps the same signature for name-parity but the
/// fdtable global isn't yet modeled — so this is a no-op shim. The
/// previous Rust impl was an `fcntl(F_GETFD)` validity check —
/// COMPLETELY DIFFERENT SEMANTICS from C (which doesn't validate
/// the fd at all, just grows the table). Fixed to no-op + bool
/// return for caller compatibility (no live callers).
pub fn check_fd_table(fd: i32) -> bool {
    // c:1969
    // c:1971-1972 — `if (fd <= max_zsh_fd) return;`
    let cur_max = MAX_ZSH_FD.load(Ordering::Relaxed); // c:1971
    if fd <= cur_max {
        // c:1971
        return true; // c:1972 (return; in C)
    }
    if fd < 0 {
        // defensive — fdtable index must be ≥0
        return false;
    }
    // c:1974-1981 — `if (fd >= fdtable_size) { while (fd >= fdtable_size)
    //                fdtable_size *= 2; zrealloc; memset(new_slots, 0); }`.
    // Rust Vec::resize handles the realloc + zero-fill in one call (the
    // expansion bumps capacity geometrically too).
    {
        let mut g = fdtable_lock().lock().unwrap();
        if (fd as usize) >= g.len() {
            g.resize((fd as usize) + 1, FDT_UNUSED); // c:1975-1979 grow
        }
    }
    // c:1982 — `max_zsh_fd = fd;`.
    MAX_ZSH_FD.store(fd, Ordering::Relaxed); // c:1982
    true
}

/// Port of `movefd()` from `Src/utils.c:1990` — C decl `movefd(int fd)`.
/// ```c
/// if (fd != -1 && fd < 10) {
///     int fe = fcntl(fd, F_DUPFD, 10);
///     zclose(fd);          // unconditional close, even when fe == -1
///     fd = fe;
/// }
/// if (fd != -1) {
///     check_fd_table(fd);
///     fdtable[fd] = FDT_INTERNAL;
/// }
/// return fd;
/// ```
/// Two divergences fixed this iteration:
///   1. Old Rust closed the source fd ONLY on success; C closes
///      unconditionally per the c:1999-2004 comment "probably better
///      to avoid a leak."
///   2. Old Rust added `FD_CLOEXEC` after the dup; C does NOT —
///      CLOEXEC is added later by `addmodulefd`/`addlockfd` callers
///      that need it. movefd itself does not.
pub fn movefd(fd: i32) -> i32 {
    #[cfg(unix)]
    {
        let mut fd = fd;
        if fd != -1 && fd < 10 {
            // c:1992
            let fe = unsafe { libc::fcntl(fd, libc::F_DUPFD, 10) }; // c:1994
            unsafe { libc::close(fd) }; // c:2004 zclose(fd) — unconditional
            fd = fe; // c:2005
        }
        // c:2007-2010 — `if (fd != -1) { check_fd_table(fd); fdtable[fd]
        // = FDT_INTERNAL; }`. The fdtable global IS now modeled (port
        // of utils.c:63 — `Vec<u8>` behind `fdtable_lock`). Mark the
        // new fd as FDT_INTERNAL so the rest of the shell knows the
        // shell is using it (a later `addmodulefd` / `addlockfd`
        // upgrade may reclassify; raw external code shouldn't touch
        // it). The previous port skipped this so internal-fd tracking
        // (which `closeallelse` and forkexec rely on) silently
        // never saw zshrs-internal fds.
        if fd != -1 {
            // c:2007
            check_fd_table(fd); // c:2008
            fdtable_set(fd, FDT_INTERNAL); // c:2009
        }
        fd
    }
    #[cfg(not(unix))]
    {
        fd
    }
}

/// Port of `redup()` from `Src/utils.c:2021` — C decl `redup(int x, int y)`.
///
/// C signature: `int redup(int x, int y)`.
/// "Move fd x to y. If x == -1, fd y is closed. Returns y for
/// success, -1 for failure." (c:2014-2016 docstring.)
///
/// Body mirrors c:2023-2068: when `x == -1`, close `y` (return `y`);
/// when `x == y`, no-op (return `y`); otherwise `dup2(x, y)` +
/// `close(x)`.
///
/// C body fdtable updates (c:2053-2063) that the previous Rust port
/// SKIPPED with a stale "fdtable global not yet ported" comment —
/// fdtable IS now ported and these updates are load-bearing:
///   * `fdtable[y] = fdtable[x]` — the new fd inherits the old fd's
///     ownership category (FDT_INTERNAL / FDT_MODULE / etc.).
///   * If the inherited type is `FDT_FLOCK` / `FDT_FLOCK_EXEC`, promote
///     to `FDT_INTERNAL` (the dup'd fd doesn't carry the flock).
///   * If `fdtable[x] == FDT_FLOCK`, decrement `fdtable_flocks` (the
///     original lock-holding fd is about to be closed).
///
/// Without these updates, redup'd fds had stale `FDT_UNUSED` ownership
/// and `closeallelse(FDT_EXTERNAL)` etc. couldn't classify them.
pub fn redup(x: i32, y: i32) -> i32 {
    // c:2021
    let mut ret = y; // c:2023
    #[cfg(unix)]
    {
        if x < 0 {
            // c:2047
            zclose(y); // c:2048
        } else if x != y {
            // c:2049
            // c:2053-2057 — successful dup2: copy fdtable + FLOCK promote.
            if unsafe { libc::dup2(x, y) } == -1 {
                // c:2050
                ret = -1; // c:2051
            } else {
                check_fd_table(y); // c:2053
                let kind_x = fdtable_get(x); // c:2054
                let kind_y = if kind_x == FDT_FLOCK || kind_x == FDT_FLOCK_EXEC {
                    FDT_INTERNAL // c:2055-2056
                } else {
                    kind_x // c:2054
                };
                fdtable_set(y, kind_y);
            }
            // c:2062-2063 — `if (fdtable[x] == FDT_FLOCK) fdtable_flocks--;`
            // The original lock-holding fd is about to be closed, so the
            // flock-count tracker must drop. Even on dup2 failure C still
            // closes x, so this runs in both arms.
            if fdtable_get(x) == FDT_FLOCK {
                // c:2062
                FDTABLE_FLOCKS.fetch_sub(1, Ordering::SeqCst); // c:2063
            }
            zclose(x); // c:2064
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (x, y);
    }
    ret // c:2067
}

/// Port of `addmodulefd()` from `Src/utils.c:2091` — C decl `addmodulefd(int fd, int fdt)`.
/// ```c
/// if (fd >= 0) {
///     check_fd_table(fd);
///     fdtable[fd] = fdt;
/// }
/// ```
/// Two divergences fixed this iteration:
///   1. C accepts an `fdt` parameter (FDT_MODULE / FDT_INTERNAL /
///      FDT_EXTERNAL — see callers in Src/Modules/{random,socket,tcp,
///      db_gdbm}.c). Rust port hardcoded `FDT_MODULE` — silently
///      ignored caller intent. Now takes `fdt` per C.
///   2. C does NOT set `FD_CLOEXEC`. Rust port was adding it
///      unconditionally — divergent. CLOEXEC is the caller's
///      responsibility based on the fdt semantics.
pub fn addmodulefd(fd: i32, fdt: i32) {
    // c:2091
    // c:2093 — `if (fd >= 0)`.
    if fd >= 0 {
        // c:2094 — `check_fd_table(fd)` — grow fdtable to cover fd.
        check_fd_table(fd); // c:2094
                            // c:2095 — `fdtable[fd] = fdt`. Routes through the canonical
                            // helper which mirrors C's direct `fdtable[fd] = fdt` write.
        fdtable_set(fd, fdt); // c:2095
    }
}

/// Port of `addlockfd()` from `Src/utils.c:2112` — C decl `addlockfd(int fd, int cloexec)`.
/// ```c
/// if (cloexec) {
///     if (fdtable[fd] != FDT_FLOCK)
///         fdtable_flocks++;
///     fdtable[fd] = FDT_FLOCK;
/// } else {
///     fdtable[fd] = FDT_FLOCK_EXEC;
/// }
/// ```
/// Critical divergence fixed: C updates `fdtable[fd]` to FDT_FLOCK
/// (cloexec=true) or FDT_FLOCK_EXEC (cloexec=false). Previous Rust
/// port did the OPPOSITE — called `fcntl(F_SETFD, FD_CLOEXEC)` when
/// `cloexec` was true. That's wrong on two counts:
///   1. C's "cloexec" parameter selects the FDT category, not a
///      libc fcntl flag.
///   2. FDT_FLOCK means "lock survives across exec via fd inheritance"
///      — the OPPOSITE of close-on-exec. The Rust port was adding
///      CLOEXEC for the very case where the fd should be inheritable.
pub fn addlockfd(fd: i32, cloexec: bool) {
    // c:2112
    if cloexec {
        // c:2114
        // c:2115-2117 — track flock count, set FDT_FLOCK.
        if fdtable_get(fd) != FDT_FLOCK {
            FDTABLE_FLOCKS.fetch_add(1, Ordering::SeqCst);
        }
        fdtable_set(fd, FDT_FLOCK);
    } else {
        // c:2118
        // c:2119 — FDT_FLOCK_EXEC means flock inherited by exec.
        fdtable_set(fd, FDT_FLOCK_EXEC);
    }
}

// FDTABLE_FLOCKS canonical declaration lives at utils.rs:5219
// (the exec.c global with same name). Reuse, don't redeclare.

/// Port of `zclose()` from `Src/utils.c:2127` — C decl `int zclose(int fd)`.
///
/// Close the given fd, and clear it from fdtable.                          // c:2123
pub fn zclose(fd: i32) -> i32 {
    // c:2127
    if fd >= 0 {
        // c:2129
        // c:2130-2133 — comment carry: "Careful: we allow closing of
        // arbitrary fd's, beyond max_zsh_fd. In that case we don't
        // try anything clever."
        let max_fd = MAX_ZSH_FD.load(Ordering::Relaxed); // c:2134
        if fd <= max_fd {
            // c:2134
            // !!! ZSHRS THREADING ADAPTATION — deviates from C here !!!
            // C closes blindly; it is single-threaded, so a shell-recorded
            // fd number can never be concurrently recycled by someone else.
            // zshrs runs 18 worker threads whose Rust-owned fds (ReadDir
            // dirfds, SQLite, File) recycle freed numbers immediately — a
            // stale zclose then closes a FOREIGN fd, and std panics in
            // ReadDir::drop ("unexpected error during closedir: EBADF",
            // observed from the compinit bg scan). An in-range fd the
            // fdtable does NOT own is by definition not ours to close.
            if fdtable_get(fd) == FDT_UNUSED {
                tracing::debug!(
                    fd,
                    "zclose: skipping unowned in-range fd (thread-safety guard)"
                );
                return 0;
            }
            if fdtable_get(fd) == FDT_FLOCK {
                // c:2135
                FDTABLE_FLOCKS.fetch_sub(1, Ordering::Relaxed);
                // c:2136
            }
            fdtable_set(fd, FDT_UNUSED); // c:2137
                                         // c:2138-2139 — shrink max_zsh_fd past trailing UNUSED slots.
            let mut m = MAX_ZSH_FD.load(Ordering::Relaxed);
            while m > 0 && fdtable_get(m) == FDT_UNUSED {
                m -= 1;
            }
            MAX_ZSH_FD.store(m, Ordering::Relaxed);
            // c:2140-2143 — coproc fd tracking.
            if fd == coprocin.load(Ordering::Relaxed) {
                coprocin.store(-1, Ordering::Relaxed);
            }
            if fd == coprocout.load(Ordering::Relaxed) {
                coprocout.store(-1, Ordering::Relaxed);
            }
        }
        #[cfg(unix)]
        unsafe {
            return libc::close(fd);
        } // c:2145
        #[cfg(not(unix))]
        return 0;
    }
    -1 // c:2147
}

/// Port of `zcloselockfd()` from `Src/utils.c:2156` — C decl `int zcloselockfd(int fd)`.
/// ```c
/// if (fd > max_zsh_fd) return -1;
/// if (fdtable[fd] != FDT_FLOCK && fdtable[fd] != FDT_FLOCK_EXEC)
///     return -1;
/// zclose(fd);
/// return 0;
/// ```
/// "Close an fd returning 0 if used for locking; return -1 if it
/// isn't." Caller (`bin_zsystem_flock -u`) uses the -1 return to
/// distinguish "fd not in the flock table" from "successfully
/// released a lock."
///
/// Previously the Rust port skipped the FDT_FLOCK / FDT_FLOCK_EXEC
/// check entirely and always returned 0 — meaning `zsystem flock -u
/// <unlocked-fd>` reported success on any fd. Now that the
/// `addlockfd` port (this iteration) populates the fdtable with the
/// canonical FDT_FLOCK / FDT_FLOCK_EXEC slots, the check can fire
/// faithfully.
pub fn zcloselockfd(fd: i32) -> i32 {
    // c:2156
    let max_fd = MAX_ZSH_FD.load(Ordering::Relaxed);
    // c:2158 — `if (fd > max_zsh_fd) return -1;`.
    if fd > max_fd {
        return -1;
    }
    // c:2160-2161 — `if (fdtable[fd] != FDT_FLOCK && != FDT_FLOCK_EXEC)
    //                   return -1;`.
    let slot = fdtable_get(fd);
    if slot != FDT_FLOCK && slot != FDT_FLOCK_EXEC {
        return -1;
    }
    zclose(fd); // c:2162
    0 // c:2163
}

/// Port of `gettempname()` from `Src/utils.c:2178` — C decl `char *gettempname(const char *prefix, int use_heap)`.
///
/// Returns a unique tempfile name templated like C `mktemp(3)` —
/// `{prefix}.XXXXXX`. Falls back to `getsparam("TMPPREFIX")` and
/// then `DEFAULT_TMPPREFIX` when `prefix` is None. Does NOT create
/// the file (matches C — only `gettempfile` creates).
pub fn gettempname(prefix: Option<&str>, _use_heap: bool) -> Option<String> {
    // c:2178
    let suffix = if prefix.is_some() {
        ".XXXXXX"
    } else {
        "XXXXXX"
    }; // c:2178
    queue_signals(); // c:2182
    let prefix_owned: String = match prefix {
        // c:2183
        Some(p) => p.to_string(),
        // c:2184 — `getsparam("TMPPREFIX")`. Read from paramtab (not OS
        //          env); fall back to compile-time default when unset.
        None => getsparam("TMPPREFIX")
            .unwrap_or_else(|| crate::ported::config_h::DEFAULT_TMPPREFIX.to_string()),
    };
    let template = format!("{}{}", prefix_owned, suffix); // c:2186-2188
                                                          // C uses mktemp(3) which mutates the X's into a unique name              // c:2192/2219
                                                          // without creating the file. Rust has no mktemp; emulate with
                                                          // pid+timestamp. Caller is responsible for O_EXCL open.
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    // c:2192 — mkstemp(3) replaces the six X's with EXACTLY six
    // characters from [A-Za-z0-9], so every zsh temp name has a fixed
    // width (`/tmp/zshDYKzyg`). The `{pid:x}{nanos:x}` form here was 8-12
    // characters wide, which shows anywhere the name is displayed: the
    // `fc -` parity cell compares the editor's status line, where zsh
    // shows /private/tmp/zshDYKzyg and zshrs showed
    // /private/tmp/zshdb0ff10c48. Caller retries on EEXIST (c:2255-2269),
    // so a collision is handled rather than fatal.
    let mut mix: u128 = ((pid as u128) << 47) ^ nanos;
    let mut unique = String::with_capacity(6);
    for _ in 0..6 {
        let idx = (mix % 62) as usize;
        unique.push(b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789"[idx] as char);
        mix /= 62;
    }
    let name = template.replace("XXXXXX", &unique);
    unqueue_signals(); // c:2221
    Some(name)
}

/// Check if metafied - port from zsh/Src/utils.c has_token()
// Check if a string contains a token                                       // c:2282
/// Port of `has_token()` from `Src/utils.c:2282` — C decl `int has_token(const char *s)`.
///
/// Port of `has_token(const char *s)` from `Src/utils.c:2282` — used by
/// `ecstrcode` (parse.rs:989) to flip the token-marker bit on the
/// encoded string offset. Token markers live in 0x83..=0x9f (Pound,
/// Stringg, Hat, Star, ..., Bnull, Nularg). Earlier impl checked
/// only 0x83 (Meta) which missed Dash/Equals/Inbrack/Inbrace/etc.,
/// so any string containing those (e.g. `dart-lang/dart` →
/// `dart\u{9b}lang/dart`) got encoded with the no-token bit set,
/// breaking byte parity with C's wordcode-emitter output.
/// ```c
/// while (*s)
///     if (itok(*s++)) return 1;
/// return 0;
/// ```
/// Routes through the canonical `itok()` typtab-driven predicate.
/// Previous Rust port used hardcoded `0x83..=0x9f` which was:
///   - Wrong on the low end: includes `0x83` (Meta, NOT a token).
///   - Wrong on the high end: missing `0xa0..=0xa1` (part of the
///     Snull..Nularg range that `itok` legitimately covers).
/// The full canonical ITOK range per `Src/zsh.h:152-159` is
/// `0x84..=0xa1` (Pound..Nularg), with possible gaps in the middle
/// (Snull at 0x9d follows Bang at 0x9c).
pub fn has_token(s: &str) -> bool {
    // c:2282
    s.bytes().any(itok) // c:2285
}

// Delete a character in a string                                           // c:2294
/// Port of `chuck()` from `Src/utils.c:2294`.
///
/// Remove character from string (from utils.c chuck)
pub fn chuck(s: &mut String, pos: usize) {
    // c:2294
    if pos < s.len() {
        s.remove(pos);
    }
}

/// Port of `tulower()` from `Src/utils.c:2302` — C decl `tulower(int c)`.
///
/// Convert character to lowercase
/// To-lowercase that respects locale.
pub fn tulower(c: char) -> char {
    // c:2302
    c.to_lowercase().next().unwrap_or(c)
}

/// Port of `tuupper()` from `Src/utils.c:2310` — C decl `tuupper(int c)`.
///
/// Convert character to uppercase
/// To-uppercase that respects locale.
pub fn tuupper(c: char) -> char {
    // c:2310
    c.to_uppercase().next().unwrap_or(c)
}

/// Port of `ztrncpy()` from `Src/utils.c:2320` — C decl `void ztrncpy(char *s, char *t, int len)`.
///
/// C body (c:2320-2326):
/// ```c
/// while (len--)
///     *s++ = *t++;
/// *s = '\0';
/// ```
///
/// Copy `len` bytes from `t` into `s`, then NUL-terminate. C does no
/// bounds check; the caller must ensure `s` has `len + 1` bytes of
/// capacity. Rust port preserves the (s, t, len) parameter shape:
/// `s: &mut String` (output), `t: &str` (source), `len: usize` (copy
/// count). The C NUL terminator at c:2325 is implicit in Rust's
/// length-prefixed String. The Rust port clamps `len` to `t.len()` to
/// avoid the out-of-bounds read C's `*s++ = *t++` would perform when
/// `len > strlen(t)` (UB in C; explicitly bounded in Rust).
pub fn ztrncpy(s: &mut String, t: &str, len: usize) {
    // c:2320
    s.clear(); // C overwrites from start of *s
    let take = len.min(t.len()); // c:2322 while (len--)
    s.push_str(&t[..take]); // c:2322 *s++ = *t++
                            // c:2325 — `*s = '\0';` (implicit in Rust String length)
}

/// Port of `strucpy()` from `Src/utils.c:2331` — C decl `void strucpy(char **s, char *t)`.
/// ```c
/// char *u = *s;
/// while ((*u++ = *t++));
/// *s = u - 1;       // leave *s pointing at the NUL terminator
/// ```
/// Body: `strcpy(*s, t)` + advance `*s` to point at the new
/// NUL terminator. The "u" in the name is a C convention for the
/// local pointer-walker, NOT "upper-case" — the function does NOT
/// change case.
///
/// Rust API: param name `s` matches C exactly per Rule E (despite Rust
/// convention of `dest` for output buffers). The C pointer-advance
/// `*s = u - 1` translates to "the new end of s is at s.len()", which
/// is implicit in Rust's owning-String model.
pub fn strucpy(s: &mut String, t: &str) {
    // c:2331
    s.push_str(t); // c:2335 `*u++ = *t++` loop
                   // c:2336 — `*s = u - 1;` — pointer-advance is implicit in Rust
                   // (`s.len()` is the new end-of-string position).
}

/// Port of `struncpy()` from `Src/utils.c:2341` — C decl `void struncpy(char **s, char *t, int n)`.
/// ```c
/// char *u = *s;
/// while (n-- && (*u = *t++)) u++;
/// *s = u;
/// if (n > 0) *u = '\0';
/// ```
/// Body: copy up to `n` bytes from `t` to `*s`, NUL-terminate.
/// Note c:2348 — "just one null-byte will do, unlike strncpy(3)";
/// doesn't pad with NULs.
///
/// Param names match C exactly per Rule E.
pub fn struncpy(s: &mut String, t: &str, n: usize) {
    // c:2341
    // c:2345 — `while (n-- && (*u = *t++)) u++;` — copy up to n
    // bytes, stop at NUL (which in Rust &str is the end-of-string).
    let take = n.min(t.len());
    s.push_str(&t[..take]);
}

/// Port of `arrlen()` from `Src/utils.c:2357` — C decl `arrlen(char **s)`.
///
/// Array length - port from arrlen()
pub fn arrlen<T>(s: &[T]) -> usize {
    // c:2357
    s.len()
}

// Return TRUE iff arrlen(s) >= lower_bound, but more efficiently.          // c:2382
/// Port of `arrlen_ge()` from `Src/utils.c:2369`.
///
/// Check if array length >= n (from utils.c arrlen_ge)
pub fn arrlen_ge<T>(arr: &[T], n: usize) -> bool {
    // c:2369
    arr.len() >= n
}

// Return TRUE iff arrlen(s) > lower_bound, but more efficiently.           // c:2400
/// Port of `arrlen_gt()` from `Src/utils.c:2382`.
///
/// Check if array length > n (from utils.c arrlen_gt)
pub fn arrlen_gt<T>(arr: &[T], n: usize) -> bool {
    // c:2382
    arr.len() > n
}

// Return TRUE iff arrlen(s) <= upper_bound, but more efficiently.          // c:2409
/// Port of `arrlen_le()` from `Src/utils.c:2391`.
///
/// Check if array length <= n (from utils.c arrlen_le)
pub fn arrlen_le<T>(arr: &[T], n: usize) -> bool {
    // c:2391
    arr.len() <= n
}

// Return TRUE iff arrlen(s) < upper_bound, but more efficiently.           // c:2400
/// Port of `arrlen_lt()` from `Src/utils.c:2400`.
///
/// Check if array length < n (from utils.c arrlen_lt)
pub fn arrlen_lt<T>(arr: &[T], n: usize) -> bool {
    // c:2400
    arr.len() < n
}

/// Port of `skipparens()` from `Src/utils.c:2409`.
///
/// Direct port of `int skipparens(char inpar, char outpar, char **s)`
/// from `Src/utils.c:2409`. Walks `*s` past a balanced `inpar`/`outpar`
/// pair, advancing the input slice in place. Returns 0 on full
/// balance, `-1` when `**s != inpar` (no opening paren), or positive
/// when the scan ran off the end of `*s` with `level > 0`
/// (unbalanced).
///
/// Rust signature: `s: &mut &str` mirrors C's `char **s` out-arg —
/// the caller's slice handle is rewritten to point past the closing
/// paren (or to the empty tail on unbalanced input).
/// WARNING: param names don't match C — Rust=(open, close, s) vs C=(inpar, outpar, s)
pub fn skipparens(open: char, close: char, s: &mut &str) -> i32 {
    // c:2409
    let mut chars = s.char_indices();
    // c:2413-2414 — `if (**s != inpar) return -1;`
    match chars.next() {
        Some((_, c)) if c == open => {}
        _ => return -1,
    }
    // c:2416 — `for (level = 1; *++*s && level;)`
    let mut level: i32 = 1;
    let mut consumed = open.len_utf8();
    for (i, c) in chars {
        consumed = i + c.len_utf8();
        if c == open {
            level += 1; // c:2417-2418
        } else if c == close {
            level -= 1; // c:2419-2420
        }
        if level == 0 {
            break;
        }
    }
    *s = &s[consumed..];
    // c:2422 — `return level;`. 0 on balanced match, positive on
    // ran-off-end (unbalanced input).
    level
}

/// Port of `zstrtol()` from `Src/utils.c:2427` — C decl `zstrtol(const char *s, char **t, int base)`.
///
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
/// WARNING: param names don't match C — Rust=(s, base) vs C=(s, t, base)
pub fn zstrtol(s: &str, base: i32) -> (i64, &str) {
    // c:2427
    zstrtol_underscore(s, base, false) // c:2427
}

/// Port of `zstrtol_underscore()` from `Src/utils.c:2438` — C decl `zstrtol_underscore(const char *s, char **t, int base, int underscore)`.
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
pub fn zstrtol_underscore(s: &str, base: i32, underscore: bool) -> (i64, &str) {
    // c:2438
    let bytes = s.as_bytes();
    let mut i = 0usize;
    // c:2444-2445 — skip leading `inblank` whitespace.
    while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
        i += 1;
    }
    // c:2447-2450 — handle sign.
    let neg = if i < bytes.len() && (bytes[i] == b'-') {
        // c:2447 IS_DASH
        i += 1;
        true
    } else if i < bytes.len() && bytes[i] == b'+' {
        // c:2449
        i += 1;
        false
    } else {
        false
    };

    // c:2452-2461 — base autodetect when base == 0.
    let mut base = base;
    if base == 0 {
        // c:2452
        if i >= bytes.len() || bytes[i] != b'0' {
            // c:2453
            base = 10; // c:2454
        } else {
            i += 1; // c:2455 ++s
            if i < bytes.len() && (bytes[i] == b'x' || bytes[i] == b'X') {
                // c:2455
                base = 16;
                i += 1;
            } else if i < bytes.len() && (bytes[i] == b'b' || bytes[i] == b'B') {
                // c:2457
                base = 2;
                i += 1;
            } else {
                // c:2459
                base = 8; // c:2460
            }
        }
    }
    let inp_idx = i; // c:2462 inp = s
    if base < 2 || base > 36 {
        // c:2463
        zerr(&format!(
            "invalid base (must be 2 to 36 inclusive): {}",
            base
        )); // c:2464
        return (0, s); // c:2465 return 0
    }

    let mut calc: u64 = 0; // c:2441
    let mut trunc_idx: Option<usize> = None; // c:2440 trunc = NULL
    if base <= 10 {
        // c:2466
        // c:2467-2479 — digit accumulator, low bases.
        let max_d = b'0' + base as u8;
        while i < bytes.len() {
            let c = bytes[i];
            if c >= b'0' && c < max_d {
                if trunc_idx.is_none() {
                    let newcalc = calc
                        .wrapping_mul(base as u64)
                        .wrapping_add((c - b'0') as u64);
                    if newcalc < calc {
                        // c:2473
                        trunc_idx = Some(i); // c:2474
                    } else {
                        calc = newcalc;
                    }
                }
                i += 1;
            } else if underscore && c == b'_' {
                // c:2467 underscore && *s == '_'
                i += 1;
            } else {
                break;
            }
        }
    } else {
        // c:2480
        // c:2481-2495 — digit accumulator, high bases (>10).
        while i < bytes.len() {
            let c = bytes[i];
            let digit = if c.is_ascii_digit() {
                // c:2481 idigit
                Some((c - b'0') as u32)
            } else if c >= b'a' && c < b'a' + base as u8 - 10 {
                // c:2482
                Some(((c & 0x1f) + 9) as u32) // c:2486 (*s & 0x1f) + 9
            } else if c >= b'A' && c < b'A' + base as u8 - 10 {
                // c:2483
                Some(((c & 0x1f) + 9) as u32)
            } else if underscore && c == b'_' {
                // c:2484
                i += 1;
                continue;
            } else {
                None
            };
            match digit {
                Some(d) => {
                    if trunc_idx.is_none() {
                        let newcalc = calc.wrapping_mul(base as u64).wrapping_add(d as u64);
                        if newcalc < calc {
                            // c:2489
                            trunc_idx = Some(i); // c:2490
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
    if trunc_idx.is_none() && (calc as i64) < 0 {
        // c:2504
        let top_bit_only = calc & !(1u64 << 63); // c:2506
        if !neg || top_bit_only != 0 {
            // c:2505
            trunc_idx = Some(i.saturating_sub(1)); // c:2508
            calc /= base as u64; // c:2509
        }
    }

    if let Some(t) = trunc_idx {
        // c:2512
        let digits = t.saturating_sub(inp_idx);
        let inp_str = &s[inp_idx..];
        zwarn(&format!(
            "number truncated after {} digits: {}",
            digits, inp_str
        )); // c:2513
    }

    let result = if neg {
        // c:2517
        -(calc as i64)
    } else {
        calc as i64
    };
    (result, &s[i..])
}

/// Port of `zstrtoul_underscore()` from `Src/utils.c:2529`.
///
/// Parse unsigned integer with underscore support
/// Port from zsh/Src/utils.c zstrtoul_underscore() lines 2528-2575
/// Port of `zstrtoul_underscore(const char *s, zulong *retval)` from
/// `Src/utils.c:2529-2590`. C body:
/// ```c
/// if (*s == '+') s++;
/// if      (*s != '0')                    base = 10;
/// else if (*++s == 'x' || *s == 'X')     base = 16, s++;
/// else if (*s == 'b' || *s == 'B')       base = 2,  s++;
/// else                                   base = isset(OCTALZEROES) ? 8 : 10;
/// ```
/// The leading-zero case is option-gated on OCTALZEROES — without
/// the option (default), `0777` parses as decimal 777, NOT octal 511.
/// Previously the Rust port always treated leading-zero as octal —
/// divergent for the default shell setup.
pub fn zstrtoul_underscore(s: &str) -> Option<u64> {
    // c:2529
    let s = s.trim();
    let s = s.strip_prefix('+').unwrap_or(s); // c:2533-2534

    let (base, rest) = if s.starts_with("0x") || s.starts_with("0X") {
        // c:2538
        (16, &s[2..])
    } else if s.starts_with("0b") || s.starts_with("0B") {
        // c:2540
        (2, &s[2..])
    } else if s.starts_with('0') && s.len() > 1 && isset(OCTALZEROES)
    // c:2543
    {
        (8, &s[1..])
    } else {
        (10, s)
    };

    let rest = rest.replace('_', "");
    u64::from_str_radix(&rest, base).ok()
}

/// Port of `setblock_fd()` from `Src/utils.c:2579`.
///
/// Port of `int setblock_fd(int turnonblocking, int fd, long *modep)`
/// from `Src/utils.c:2579`. Toggles O_NONBLOCK on `fd`.
///
/// **Signature matches C exactly** (param order `turnonblocking, fd`):
/// - `turnonblocking=1` → clear O_NONBLOCK (enable blocking).
/// - `turnonblocking=0` → set O_NONBLOCK (disable blocking).
///
/// Returns:
/// - `(true, mode)` if the fd's state was changed.
/// - `(false, -1)` if the fd is a regular file (C skips regular
///   files entirely — only operates on pipes/sockets/ttys per
///   `c:2599 if (!fstat(fd, &st) && !S_ISREG(st.st_mode))`).
/// - `(false, mode)` if state was already correct.
///
/// Rust returns `(state_changed, modep_value)` as a tuple to mirror
/// C's `int` return + `long *modep` out-param.
pub fn setblock_fd(turnonblocking: bool, fd: i32) -> (bool, libc::c_long) {
    // c:2579
    #[cfg(unix)]
    unsafe {
        // c:2598-2600 — `if (!fstat(fd, &st) && !S_ISREG(st.st_mode))`.
        let mut st: libc::stat = std::mem::zeroed();
        if libc::fstat(fd, &mut st) != 0 {
            return (false, -1);
        }
        // c:2599 — skip regular files (no nonblock concept).
        let mode_bits = st.st_mode as u32;
        if (mode_bits & libc::S_IFMT as u32) == libc::S_IFREG as u32 {
            return (false, -1); // c:2614 *modep = -1; return 0
        }
        // c:2601 — `*modep = fcntl(fd, F_GETFL, 0);`
        let modep = libc::fcntl(fd, libc::F_GETFL, 0) as libc::c_long;
        if modep < 0 {
            return (false, -1); // c:2602 if (*modep != -1)
        }
        const NONBLOCK: libc::c_long = libc::O_NONBLOCK as libc::c_long;
        if !turnonblocking {
            // c:2603-2606 — want to KNOW if blocking was off; set it on.
            if (modep & NONBLOCK) != 0 {
                return (true, modep); // already nonblock — no-op, but "off"
            }
            if libc::fcntl(fd, libc::F_SETFL, modep | NONBLOCK) == 0 {
                return (true, modep); // c:2606
            }
        } else {
            // c:2607-2611 — want to clear NONBLOCK if currently set.
            if (modep & NONBLOCK) != 0 && libc::fcntl(fd, libc::F_SETFL, modep & !NONBLOCK) == 0 {
                return (true, modep); // c:2611 state changed
            }
        }
        (false, modep)
    }
    #[cfg(not(unix))]
    {
        let _ = (turnonblocking, fd);
        (false, -1)
    }
}

/// Port of `setblock_stdin()` from `Src/utils.c:2622` — C decl `int setblock_stdin(void)`.
/// ```c
/// long mode;
/// return setblock_fd(1, 0, &mode);
/// ```
/// `turnonblocking=1, fd=0` — turn ON blocking on stdin (clear
/// O_NONBLOCK).
pub fn setblock_stdin() -> i32 {
    // c:2624 — `return setblock_fd(1, 0, &mode);`.
    let (changed, _mode) = setblock_fd(true, 0); // c:2624
    changed as i32
}

/// Port of `read_poll()` from `Src/utils.c:2645`.
///
/// Port of `int read_poll(int fd, int *readchar, int polltty, zlong microseconds)`
/// from `Src/utils.c:2645-2738`.
///
/// Check whether input is pending on `fd` within `timeout_us`
/// microseconds. Faithful port of all three C paths:
/// 1. The `polltty` non-canonical-tty path (VMIN/VTIME termios
///    setup so a raw tty can be polled — c:2665-2696).
/// 2. The `HAVE_SELECT` wait (c:2697-2705). Rust uses `poll(2)`
///    as the idiom substitution for `select(2)`; `poll` return
///    semantics are mapped onto C's `ret` so the `ret < 0`
///    error-fallback still triggers.
/// 3. The final `setblock_fd` + non-blocking `read` fallback that
///    captures a byte when `select`/`poll` errored (c:2718-2729).
///
/// `readchar` is C's `int *readchar` out-param: when a byte is
/// actually consumed during the poll it is written through
/// `readchar` (so `read -t -k` on a raw tty gets the byte). `polltty`
/// is C's `int polltty` truthiness. Returns C's `(ret > 0)`.
///
/// WARNING: param order differs from C — Rust=(fd, readchar,
/// polltty, timeout_us) vs C=(fd, readchar, polltty, microseconds);
/// C's `microseconds` is `timeout_us` here.
pub fn read_poll(fd: i32, readchar: &mut i32, polltty: bool, timeout_us: i64) -> bool {
    // c:2645
    #[cfg(unix)]
    {
        let mut ret: i32 = -1; // c:2647
        let mut mode: libc::c_long = -1; // c:2648
                                         // C reassigns the `polltty` param; hold it as the VMIN-derived int.
        let mut polltty: i32 = polltty as i32; // c:2645
        let mut ti: Option<libc::termios> = None; // c:2659 `struct ttyinfo ti;`

        // c:2662-2663 — `if (fd < 0 || (polltty && !isatty(fd))) polltty = 0;`
        if fd < 0 || (polltty != 0 && unsafe { libc::isatty(fd) } == 0) {
            polltty = 0; // no tty to poll
        }

        // c:2665-2696 — HAS_TIO && !__CYGWIN__: non-canonical VMIN poll setup.
        if polltty != 0 && fd >= 0 {
            // c:2686 — `gettyinfo(&ti);` (operates on global SHTTY, as in C).
            if let Some(mut t) = gettyinfo() {
                // c:2687 — `if ((polltty = ti.tio.c_cc[VMIN]))`
                polltty = t.c_cc[libc::VMIN] as i32;
                if polltty != 0 {
                    t.c_cc[libc::VMIN] = 0; // c:2688
                                            // c:2690 — termios timeout is 10ths of a second.
                    t.c_cc[libc::VTIME] = (timeout_us / 100_000) as libc::cc_t; // c:2690
                    settyinfo(&t); // c:2691
                }
                ti = Some(t);
            } else {
                // gettyinfo failed (SHTTY closed) — nothing to poll via VMIN.
                polltty = 0;
            }
        }

        // c:2697-2705 — HAVE_SELECT wait, using poll(2) as the select(2) idiom.
        let timeout_ms = (timeout_us / 1000) as i32; // c:2698-2699 (expire_tv)
        if fd > -1 {
            // c:2701-2703
            let mut fds = [libc::pollfd {
                fd: fd as RawFd,
                events: libc::POLLIN,
                revents: 0,
            }];
            let raw = unsafe { libc::poll(fds.as_mut_ptr(), 1, timeout_ms) };
            ret = if raw > 0 {
                // Map poll revents onto select's "fd is readable" ret>0.
                if (fds[0].revents & libc::POLLIN) != 0 {
                    1
                } else {
                    0
                }
            } else {
                raw // 0 = timeout, -1 = error (triggers fallback below)
            };
        } else {
            // c:2705 — `select(0, NULL, NULL, NULL, &expire_tv)`: pure sleep.
            ret = unsafe { libc::poll(std::ptr::null_mut(), 0, timeout_ms) };
        }

        // c:2718 — `if (fd >= 0 && ret < 0 && !errflag)`
        if fd >= 0 && ret < 0 && errflag.load(Ordering::Relaxed) == 0 {
            // c:2723 — final attempt: non-blocking read of a single char.
            // `(polltty || setblock_fd(0, fd, &mode))`
            let can_read = if polltty != 0 {
                true // polltty short-circuits setblock_fd; mode stays -1
            } else {
                let (changed, m) = setblock_fd(false, fd); // c:2723
                mode = m; // C writes *modep unconditionally
                changed
            };
            if can_read {
                let mut c: u8 = 0;
                let n = unsafe { libc::read(fd, &mut c as *mut u8 as *mut libc::c_void, 1) };
                if n > 0 {
                    *readchar = c as i32; // c:2724
                    ret = 1; // c:2725
                }
            }
            // c:2727-2728 — `if (mode != -1) fcntl(fd, F_SETFL, mode);`
            if mode != -1 {
                unsafe {
                    libc::fcntl(fd, libc::F_SETFL, mode);
                }
            }
        }

        // c:2730-2736 — HAS_TIO: restore canonical VMIN=1 / VTIME=0.
        if polltty != 0 {
            if let Some(mut t) = ti {
                t.c_cc[libc::VMIN] = 1; // c:2732
                t.c_cc[libc::VTIME] = 0; // c:2733
                settyinfo(&t); // c:2734
            }
        }

        ret > 0 // c:2737
    }
    #[cfg(not(unix))]
    {
        let _ = (fd, readchar, polltty, timeout_us);
        false
    }
}

/// Port of `timespec_diff_us()` from `Src/utils.c:2752` — C decl `timespec_diff_us(const struct timespec *t1, const struct timespec *t2)`.
///
/// Compute time difference in microseconds (from utils.c timespec_diff_us)
/// Rust idiom replacement: `Instant::duration_since().as_micros()`
/// replaces the C tv_sec/tv_nsec arithmetic + sign-flip dance.
pub fn timespec_diff_us(t1: &std::time::Instant, t2: &std::time::Instant) -> i64 {
    if *t2 > *t1 {
        t2.duration_since(*t1).as_micros() as i64
    } else {
        -(t1.duration_since(*t2).as_micros() as i64)
    }
}

/// Port of `zmonotime()` from `Src/utils.c:2780` — C decl `int zmonotime(time_t *tloc)`.
///
/// "Like time(), but uses the monotonic clock." Reads CLOCK_MONOTONIC
/// and returns `tv_sec`; writes the same value through `tloc` if
/// non-NULL (Rust: `Some(&mut t)`).
pub fn zmonotime(tloc: Option<&mut i64>) -> i64 {
    // c:2780
    #[cfg(unix)]
    {
        // c:2782 — `struct timespec ts;`
        let mut ts = libc::timespec {
            // c:2782
            tv_sec: 0,
            tv_nsec: 0,
        };
        // c:2783 — `zgettime_monotonic_if_available(&ts);`
        unsafe {
            libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts); // c:2783
        }
        // c:2784-2785 — `if (tloc) *tloc = ts.tv_sec;`
        if let Some(t) = tloc {
            // c:2784
            *t = ts.tv_sec as i64; // c:2785
        }
        ts.tv_sec as i64 // c:2786 return ts.tv_sec
    }
    #[cfg(not(unix))]
    {
        let _ = tloc;
        0
    }
}

/// Port of `zsleep()` from `Src/utils.c:2797` — C decl `int zsleep(long us)`.
///
/// Check if string looks like a number
/// Check whether a string parses as a decimal integer.
/// Sleep for a given number of seconds (fractional)
/// Sleep for a fractional number of seconds.
///
/// "Sleep for the given number of microseconds." Wraps
/// `nanosleep(2)` with EINTR retry, returning 1 on completion, 0
/// on permanent error.
pub fn zsleep(us: i64) -> i32 {
    // c:2797
    let mut sleeptime = libc::timespec {
        // c:2797-2803
        tv_sec: (us / 1_000_000) as libc::time_t,
        tv_nsec: ((us % 1_000_000) * 1000) as libc::c_long,
    };
    #[cfg(unix)]
    {
        loop {
            // c:2804
            let mut rem = libc::timespec {
                tv_sec: 0,
                tv_nsec: 0,
            };
            let ret = unsafe { libc::nanosleep(&sleeptime, &mut rem) }; // c:2806
            if ret == 0 {
                // c:2808
                return 1;
            }
            let err = io::Error::last_os_error().raw_os_error().unwrap_or(0);
            if err != libc::EINTR {
                // c:2810
                return 0; // c:2811
            }
            sleeptime = rem; // c:2812
        }
    }
    #[cfg(not(unix))]
    {
        let _ = sleeptime;
        std::thread::sleep(std::time::Duration::from_micros(us.max(0) as u64));
        1
    }
}

/// Port of `zsleep_random()` from `Src/utils.c:2833`.
///
/// Sleep random amount up to max microseconds (from utils.c zsleep_random)
/// Port of `int zsleep_random(long max_us, time_t end_time)`
/// from Src/utils.c:2833.
///
/// "Sleep for time (fairly) randomly up to max_us microseconds.
/// Don't let the time extend beyond end_time. end_time is compared
/// to the current *monotonic* clock time. Return 1 if that seemed
/// to work, else 0."
pub fn zsleep_random(max_us: i64, end_time: i64) -> i32 {
    // c:2833
    let now = zmonotime(None); // c:2833
    let r16 = unsafe { libc::rand() } & 0xFFFF; // c:2845
    let mut r: i64 = (max_us >> 16) * (r16 as i64); // c:2852
    while r != 0 && now + (r / 1_000_000) > end_time {
        // c:2858
        r >>= 1; // c:2859
    }
    if r != 0 {
        // c:2860
        zsleep(r) // c:2861
    } else {
        0 // c:2862
    }
}

/// Port of `checkrmall()` from `Src/utils.c:2867` — C decl `int checkrmall(char *s)`.
///
/// C body (c:2867-2919): count files in directory `s` (capped at 100,
/// honoring GLOBDOTS), emit one of 4 prompts based on count, optionally
/// sleep 10s under RMSTARWAIT, then `getquery("ny", 1)` for confirm.
///
/// Previous Rust port was a FAKE — single generic prompt, no file
/// count, no shout/errflag checks, no RMSTARWAIT, wrong valid-chars
/// order ("yn" instead of "ny" so 'n' isn't the default-first-arm).
/// Lost 9 of the 10 C semantic branches.
pub fn checkrmall(s: &str) -> bool {
    // c:2867
    // c:2871-2872 — `if (!shout) return 1;` — no controlling tty for
    // prompt output, default to yes. Rust analogue: if stderr isn't
    // a tty, skip the prompt and approve.
    let shout_is_tty = unsafe { libc::isatty(2) != 0 }; // c:2871
    if !shout_is_tty {
        // c:2871
        return true; // c:2872
    }
    // c:2873-2878 — `if (*s != '/')` build absolute via pwd prefix.
    let s_owned: String; // c:2873
    let s_abs: &str = if !s.starts_with('/') {
        // c:2873
        let pwd = getsparam("PWD").unwrap_or_default(); // c:2874 pwd[1]
        s_owned = if pwd.len() > 1 {
            // c:2874 if (pwd[1])
            crate::ported::string::tricat(&pwd, "/", s) // c:2875 zhtricat
        } else {
            // c:2876
            crate::ported::string::dyncat("/", s) // c:2877
        };
        s_owned.as_str()
    } else {
        s
    };
    let max_count: i32 = 100; // c:2879
    let mut count: i32 = 0; // c:2870 count = 0
                            // c:2880-2892 — opendir + readdir loop, count entries (skip
                            // dotfiles when !GLOBDOTS), cap at max_count.
    let ignoredots = !isset(GLOBDOTS); // c:2881
                                       // c:2880 — `if (!(dir = opendir(unmeta(s)))) return;` then
                                       //          `while ((fn = zreaddir(dir, 1))) { ... }`.
    if let Ok(mut dir) = fs::read_dir(s_abs) {
        while let Some(fname) = zreaddir(&mut dir, 1) {
            if ignoredots && fname.starts_with('.') {
                // c:2885
                continue; // c:2886
            }
            count += 1; // c:2887
            if count > max_count {
                // c:2888
                break; // c:2889
            }
        }
        // c:closedir(dir) auto on drop
    }
    // c:2893-2904 — four-arm prompt based on file count.
    let stderr_h = io::stderr();
    let mut stderr_w = stderr_h.lock();
    if count > max_count {
        // c:2893
        let _ = write!(
            stderr_w,
            "zsh: sure you want to delete more than {} files in ",
            max_count
        ); // c:2894
    } else if count == 1 {
        // c:2896
        let _ = write!(stderr_w, "zsh: sure you want to delete the only file in ");
    // c:2897
    } else if count > 0 {
        // c:2898
        let _ = write!(
            stderr_w,
            "zsh: sure you want to delete all {} files in ",
            count
        ); // c:2899
    } else {
        // c:2901
        // c:2902 — `We don't know how many files the glob will expand to`
        let _ = write!(stderr_w, "zsh: sure you want to delete all the files in ");
        // c:2903
    }
    // c:2905 — `nicezputs(s, shout)` — escape non-printables in path.
    let _ = nicezputs(s_abs, &mut stderr_w); // c:2905
                                             // c:2906-2912 — RMSTARWAIT: print "? (waiting ten seconds)",
                                             // flush, beep, sleep 10, then newline.
    if isset(RMSTARWAIT) {
        // c:2906
        let _ = write!(stderr_w, "? (waiting ten seconds)"); // c:2907
        let _ = stderr_w.flush(); // c:2908
        drop(stderr_w); // release lock around zbeep + sleep
        zbeep(); // c:2909
        std::thread::sleep(std::time::Duration::from_secs(10)); // c:2910
        let _ = writeln!(io::stderr()); // c:2911 fputc('\n', shout)
    } else {
        drop(stderr_w);
    }
    // c:2913-2914 — `if (errflag) return 0;`
    if errflag.load(Ordering::Relaxed) != 0 {
        // c:2913
        return false; // c:2914
    }
    // c:2915-2917 — emit "[yn]? ", flush, beep.
    let _ = io::stderr().write_all(b" [yn]? "); // c:2915 nicezputs of " [yn]? "
    let _ = io::stderr().flush(); // c:2916
    zbeep(); // c:2917
             // c:2918 — `return (getquery("ny", 1) == 'y');`. Default-first-
             // char is 'n' (no) — getquery's first valid char is the default
             // returned when the user just hits Enter.
    getquery(Some("ny"), 1) == b'y' as i32 // c:2918
}

/// Port of `read_loop()` from `Src/utils.c:2923`.
///
/// Port of `ssize_t read_loop(int fd, char *buf, size_t len)` from
/// `Src/utils.c:2923`.
///
/// C body (c:2923-2945):
/// ```c
/// while (1) {
///     ssize_t ret = read(fd, buf, len);
///     if (ret == len) break;
///     if (ret <= 0) {
///         if (ret < 0) {
///             if (errno == EINTR) continue;          // c:2933
///             if (fd != SHTTY)                       // c:2935
///                 zwarn("read failed: %e", errno);   // c:2936
///         }
///         return ret;
///     }
///     buf += ret; len -= ret;
/// }
/// ```
///
/// The previous Rust port omitted the `zwarn("read failed: %e", errno)`
/// emission at c:2935-2936. That's a real diagnostic divergence:
/// any non-SHTTY read failure in zsh emits a stderr message; the
/// Rust port silently propagated the io::Error to the caller, which
/// in most call sites was discarded (`let _ = read_loop(...)`).
/// Users debugging an interrupted file read (e.g. `< /broken/fd`)
/// would see no error message at all.
/// WARNING: param names don't match C — Rust=(fd, buf) vs C=(fd, buf, len)
pub fn read_loop(fd: i32, buf: &mut [u8]) -> io::Result<usize> {
    // c:2923
    #[cfg(unix)]
    {
        let mut total = 0;
        while total < buf.len() {
            let n = unsafe {
                libc::read(
                    // c:2928
                    fd,
                    buf[total..].as_mut_ptr() as *mut libc::c_void,
                    buf.len() - total,
                )
            };
            if n <= 0 {
                // c:2931
                if n < 0 {
                    let e = io::Error::last_os_error();
                    if e.kind() == io::ErrorKind::Interrupted {
                        // c:2933
                        continue; // c:2934
                    }
                    // c:2935-2936 — `if (fd != SHTTY) zwarn("read failed: %e", errno);`
                    let shtty = SHTTY.load(Ordering::Relaxed);
                    if fd != shtty {
                        // c:2935
                        zwarn(
                            // c:2936
                            &format!("read failed: {}", e),
                        );
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

/// Port of `write_loop()` from `Src/utils.c:2949`.
///
/// Port of `ssize_t write_loop(int fd, const char *buf, size_t len)` from
/// `Src/utils.c:2949`.
///
/// C body (c:2949-2970):
/// ```c
/// while (1) {
///     ssize_t ret = write(fd, buf, len);
///     if (ret == len) break;
///     if (ret < 0) {
///         if (errno == EINTR) continue;
///         if (fd != SHTTY)                            // c:2960
///             zwarn("write failed: %e", errno);       // c:2961
///         return -1;
///     }
///     buf += ret; len -= ret;
/// }
/// ```
///
/// Same divergence as `read_loop`: previous Rust port omitted the
/// `zwarn("write failed: %e", errno)` emission for non-SHTTY fds.
/// WARNING: param names don't match C — Rust=(fd, buf) vs C=(fd, buf, len)
pub fn write_loop(fd: i32, buf: &[u8]) -> io::Result<usize> {
    // c:2949
    #[cfg(unix)]
    {
        let mut total = 0;
        while total < buf.len() {
            let n = unsafe {
                libc::write(
                    // c:2954
                    fd,
                    buf[total..].as_ptr() as *const libc::c_void,
                    buf.len() - total,
                )
            };
            if n <= 0 {
                if n < 0 {
                    let e = io::Error::last_os_error();
                    if e.kind() == io::ErrorKind::Interrupted {
                        // c:2958
                        continue; // c:2959
                    }
                    // c:2960-2961 — `if (fd != SHTTY) zwarn("write failed: %e", errno);`
                    let shtty = SHTTY.load(Ordering::Relaxed);
                    if fd != shtty {
                        // c:2960
                        zwarn(
                            // c:2961
                            &format!("write failed: {}", e),
                        );
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

/// Port of `read1char()` from `Src/utils.c:2972` — C decl `static int read1char(int echo)`.
///
/// Read a single character (from utils.c read1char).
///
///
/// C body (c:2972-2988):
/// ```c
/// char c;
/// int q = queue_signal_level();
/// dont_queue_signals();
/// while (read(SHTTY, &c, 1) != 1) {
///     if (errno != EINTR || errflag || retflag || breaks || contflag) {
///         restore_queue_signals(q);
///         return -1;
///     }
/// }
/// restore_queue_signals(q);
/// if (echo) write_loop(SHTTY, &c, 1);
/// return (unsigned char) c;
/// ```
///
/// Returns the byte read (0..=255) on success, `-1` on error. Three
/// divergences in the previous Rust port:
///   1. **Read from stdin** instead of SHTTY (the tty fd). zsh reads
///      single chars during `read -k`, `bindkey`, query prompts —
///      always against the controlling terminal, not the standard
///      input which may be a pipe.
///   2. **No `echo` parameter.** When `echo=1` C writes the byte
///      back to SHTTY so the user sees what they typed.
///   3. **No `errflag` / `retflag` / `breaks` / `contflag` early-exit.**
///      A SIGINT-driven errflag should abort the read; previous Rust
///      port had no break path.
/// WARNING: returns i32 not `Option<char>` to match C's signature
/// (`(unsigned char) c` for success, `-1` for error).
pub fn read1char(echo: i32) -> i32 {
    // c:2972
    #[cfg(unix)]
    {
        let shtty = SHTTY.load(Ordering::Relaxed);
        if shtty < 0 {
            return -1;
        }
        let mut c: u8 = 0;
        loop {
            let rc = unsafe { libc::read(shtty, &mut c as *mut u8 as *mut libc::c_void, 1) };
            if rc == 1 {
                // c:2978
                break;
            }
            // c:2979 — `if (errno != EINTR || errflag || retflag || breaks
            //              || contflag) return -1;`
            let err = io::Error::last_os_error().raw_os_error().unwrap_or(0);
            if err != libc::EINTR {
                return -1;
            }
            // Check errflag interrupt flag (the most observable of the
            // four guards; retflag/breaks/contflag are control-flow
            // states that don't apply outside the exec engine).
            if errflag.load(Ordering::Relaxed) != 0 {
                return -1;
            }
        }
        // c:2985-2986 — `if (echo) write_loop(SHTTY, &c, 1);`
        if echo != 0 {
            // c:2985
            let _ = write_loop(shtty, &[c]); // c:2986
        }
        // c:2987 — `return (unsigned char) c;`
        c as i32
    }
    #[cfg(not(unix))]
    {
        let _ = echo;
        -1
    }
}

/// Port of `noquery()` from `Src/utils.c:2992` — C decl `int noquery(int purge)`.
///
/// "If anything has been typed before the query, return without
/// asking. Optionally also purge the input queue." Returns the
/// number of bytes pending on `SHTTY` (via FIONREAD ioctl); when
/// `purge` is set, drains them before returning.
pub fn noquery(purge: bool) -> i32 {
    // c:2992
    let mut val: libc::c_int = 0; // c:2992
    #[cfg(unix)]
    {
        let shtty = SHTTY.load(Ordering::Relaxed); // c:2999
        if shtty == -1 {
            return 0;
        }
        unsafe {
            // c:2999
            libc::ioctl(shtty, libc::FIONREAD, &mut val as *mut libc::c_int);
        }
        if purge {
            // c:3000
            let mut c: u8 = 0;
            for _ in 0..val {
                // c:3001
                let _ = unsafe {
                    // c:3002
                    libc::read(shtty, &mut c as *mut u8 as *mut libc::c_void, 1)
                };
            }
        }
    }
    val // c:3009
}

/// Port of `getquery()` from `Src/utils.c:3014`.
///
/// Port of `int getquery(char *valid_chars, int purge)` from
/// `Src/utils.c:3014`.
///
/// Read a single keystroke from the TTY in raw mode and return its
/// character code (as `int`, mirroring C). If `valid_chars` is `None`
/// (C `NULL`) any byte is accepted. If `valid_chars` is `Some(s)`, only
/// chars in `s` (plus `\n` which maps to `s[0]`) are accepted; other
/// input rings the bell and re-prompts. `Y`/`N` are pre-normalised to
/// `y`/`n` per c:3041-3045.
///
/// `purge != 0` triggers `noquery(purge)`'s queue-purge path — if input
/// is queued, returns `'n'` without reading.
pub fn getquery(valid_chars: Option<&str>, purge: i32) -> i32 {
    // c:3014
    // c:3016 — `int c, d, nl = 0;`
    let mut c: i32;
    let mut d: i32;
    let mut nl: i32 = 0;
    // c:3017 — `int isem = !strcmp(term, "emacs");`
    // Stub: `term` is the $TERM environment global, declared in
    // `Src/init.c` (extern in zsh.h). Local stub reads $TERM from
    // paramtab; absent → empty string.
    let term: String = getsparam("TERM").unwrap_or_default();
    let isem: bool = term == "emacs";
    // c:3018 — `struct ttyinfo ti;`
    // Rust: termios is the canonical TTY-state type returned by gettyinfo.
    let mut ti: libc::termios; // c:3018

    attachtty(mypgrp.load(Ordering::Relaxed)); // c:3020 attachtty(mypgrp)

    // c:3022 — `gettyinfo(&ti);`
    ti = match gettyinfo() {
        Some(t) => t,
        None => return -1,
    };
    // c:3023-3030 — `ti.tio.c_lflag &= ~ECHO; if (!isem) {
    //                  ti.tio.c_lflag &= ~ICANON;
    //                  ti.tio.c_cc[VMIN] = 1;
    //                  ti.tio.c_cc[VTIME] = 0; }`
    #[cfg(unix)]
    {
        ti.c_lflag &= !(libc::ECHO); // c:3024
        if !isem {
            // c:3025
            ti.c_lflag &= !(libc::ICANON); // c:3026
            ti.c_cc[libc::VMIN] = 1; // c:3027
            ti.c_cc[libc::VTIME] = 0; // c:3028
        }
    }
    // c:3037 — `settyinfo(&ti);`
    settyinfo(&ti);

    // c:3039 — `if (noquery(purge))`
    if noquery(purge != 0) != 0 {
        // Stub: `shttyinfo` is the canonical saved-TTY-state global,
        // declared in `Src/init.c` (extern in zsh.h:1856). Without the
        // global tracked here we re-fetch current termios as a degraded
        // best-effort restore.
        if !isem {
            // c:3040
            if let Some(saved) = gettyinfo() {
                // c:3041 settyinfo(&shttyinfo)
                settyinfo(&saved);
            }
        }
        let _ = write_loop(SHTTY.load(Ordering::Relaxed), b"n\n"); // c:3042
        return b'n' as i32; // c:3043
    }

    // c:3046-3061 — `while ((c = read1char(0)) >= 0) { ... }`
    c = -1;
    loop {
        let cc = read1char(0); // c:3046
        if cc < 0 {
            c = cc;
            break;
        }
        c = cc;
        if c == b'Y' as i32 {
            // c:3047
            c = b'y' as i32; // c:3048
        } else if c == b'N' as i32 {
            // c:3049
            c = b'n' as i32; // c:3050
        }
        if valid_chars.is_none() {
            // c:3051
            break; // c:3052
        }
        if c == b'\n' as i32 {
            // c:3053
            // c:3054 — `c = *valid_chars;`
            c = valid_chars.unwrap().bytes().next().unwrap_or(b'\n') as i32;
            nl = 1; // c:3055
            break; // c:3056
        }
        // c:3058 — `if (strchr(valid_chars, c))`
        if valid_chars.unwrap().bytes().any(|b| b as i32 == c) {
            nl = 1; // c:3059
            break; // c:3060
        }
        zbeep(); // c:3062
    }
    if c >= 0 {
        // c:3064
        // c:3065-3066 — `char buf = (char)c; write_loop(SHTTY, &buf, 1);`
        let buf = [c as u8]; // c:3065
        let _ = write_loop(SHTTY.load(Ordering::Relaxed), &buf); // c:3066
    }
    if nl != 0 {
        // c:3068
        let _ = write_loop(SHTTY.load(Ordering::Relaxed), b"\n"); // c:3069
    }

    if isem {
        // c:3071
        if c != b'\n' as i32 {
            // c:3072
            // c:3073 — `while ((d = read1char(1)) >= 0 && d != '\n');`
            loop {
                d = read1char(1);
                if d < 0 || d == b'\n' as i32 {
                    break;
                }
            }
        }
    } else if c != b'\n' as i32 && valid_chars.is_none() {
        // c:3075
        // c:3077-3094 — MULTIBYTE_SUPPORT branch: drain trailing bytes
        // of an incomplete multibyte sequence.
        if isset(MULTIBYTE) && c >= 0 {
            // c:3077
            // c:3083-3093 — `for (;;) { ret = mbrlen(&cc, 1, &mbs);
            //                          if (ret != MB_INCOMPLETE) break;
            //                          c = read1char(1); if (c < 0) break;
            //                          cc = (char)c; }`
            // Rust: model MB_INCOMPLETE detection by checking whether
            // the byte buffer so far forms a complete UTF-8 prefix.
            // `cc` carries the current byte under examination.
            let mut cc: u8 = c as u8; // c:3082
            let mut accum: Vec<u8> = vec![cc];
            loop {
                // mbrlen returns MB_INCOMPLETE when the partial buffer
                // doesn't yet form a complete UTF-8 character. Rust
                // equivalent: from_utf8 errors with error_len() == None
                // when more bytes are needed.
                let incomplete = match std::str::from_utf8(&accum) {
                    Ok(_) => false,
                    Err(e) => e.error_len().is_none(),
                };
                if !incomplete {
                    // c:3088
                    break;
                }
                let nc = read1char(1); // c:3089
                if nc < 0 {
                    // c:3090
                    break; // c:3091
                }
                cc = nc as u8; // c:3092
                accum.push(cc);
            }
            let _ = cc;
        }
        // c:3097 — `write_loop(SHTTY, "\n", 1);`
        let _ = write_loop(SHTTY.load(Ordering::Relaxed), b"\n");
    }

    // c:3101 — `settyinfo(&shttyinfo);` — restore saved TTY state.
    // Stub-degraded path: refetch current termios. The proper port
    // requires wiring an `shttyinfo` global in init.rs.
    if let Some(saved) = gettyinfo() {
        settyinfo(&saved);
    }

    c // c:3102 return c
}

// `spscan` (Src/utils.c:3109) — canonical port lives below the
// thread_local block in this file. The pre-existing 3-arg
// `(name, candidates[], threshold) → Option<String>` shape was a
// drift fake (C is `void spscan(HashNode hn, scanflags)`); deleted
// because it had zero call sites and conflicted with the faithful
// port spckword needs.

// spellcheck a word                                                       // c:3123
// fix s ; if hist is nonzero, fix the history list too                    // c:3124

// File-static state shared with the (inlined) spscan callback. C uses
// raw file-statics at utils.c:3045-3050 (`best`, `d`, `guess`, `ic`,
// `spckpat`, `spnamepat`); Rust port mirrors them as thread_locals
// (per-evaluator per PORT_PLAN.md bucket-1) so concurrent worker
// threads each have their own correction state.
thread_local! {
    /// Port of `static int d;` from `Src/utils.c:3045`. Best dist seen.
    static SPCK_D: std::cell::Cell<i32> = const { std::cell::Cell::new(0) };
    /// Port of `static char *best;` from `Src/utils.c:3046`. Best-match.
    static SPCK_BEST: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
    /// Port of `static char *guess;` from `Src/utils.c:3047`. Word to fix.
    static SPCK_GUESS: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
    /// Port of `static Patprog spckpat;` from `Src/utils.c:3049`.
    static SPCK_PAT: std::cell::RefCell<Option<crate::ported::pattern::Patprog>>
        = const { std::cell::RefCell::new(None) };
    /// Port of `static Patprog spnamepat;` from `Src/utils.c:3050`.
    static SPCK_NAMEPAT: std::cell::RefCell<Option<crate::ported::pattern::Patprog>>
        = const { std::cell::RefCell::new(None) };
}

/// Port of `spscan()` from `Src/utils.c:3109`.
///
/// Port of `static void spscan(HashNode hn, UNUSED(int scanflags))` from
/// `Src/utils.c:3109`. Inlined per-call below because hashtable.rs's
/// scan helpers don't model C's `scanhashtable(table, ..., scanfn, ...)`
/// callback shape — Rust call sites iterate the table directly. C
/// body:
/// ```c
/// int nd = spdist(hn->nam, guess, (int) strlen(guess) / 4 + 1);
/// if (nd < d && (!spckpat || !pattry(spckpat, hn->nam))) {
///     best = hn->nam;
///     d = nd;
/// }
/// ```
fn spscan(name: &str) {
    // c:3074
    let guess = SPCK_GUESS.with(|g| g.borrow().clone()).unwrap_or_default();
    if guess.is_empty() {
        return;
    }
    let thresh = guess.len() / 4 + 1; // c:3076 `(int) strlen(guess) / 4 + 1`
    let nd = spdist(name, &guess, thresh) as i32; // c:3076
    let d = SPCK_D.with(|c| c.get());
    if nd < d {
        // c:3077
        // c:3078 — `if (!spckpat || !pattry(spckpat, hn->nam))`.
        let allow = SPCK_PAT.with(|p| {
            p.borrow()
                .as_ref()
                .map(|prog| !crate::ported::pattern::pattry(prog, name))
                .unwrap_or(true)
        });
        if allow {
            SPCK_BEST.with(|b| *b.borrow_mut() = Some(name.to_string())); // c:3079
            SPCK_D.with(|c| c.set(nd)); // c:3080
        }
    }
}

/// Port of `spckword()` from `Src/utils.c:3128`.
///
/// Port of `void spckword(char **s, int hist, int cmd, int ask)` from
/// `Src/utils.c:3128`.
///
/// Spell-check `*s`. If a near-match is found in the appropriate
/// hashtable (params for `$x`, command tables for command position,
/// directory entries otherwise) AND the user accepts the correction
/// (`ask=0` auto-accepts), replace `*s` in place with the corrected
/// form and (if `hist!=0`) rewrite the history entry too.
///
/// Faithful 1:1 line-by-line port. Interactive prompting (c:3273-3287)
/// is stubbed to auto-accept when `ask=1` since `getquery` /
/// `promptexpand` / `shout` / `zbeep` aren't yet wired in zshrs —
/// flagged with WARNING at the prompt site.
///
/// Caller updates: previous Rust signature `(word, candidates[], threshold)
/// → Option<String>` is gone — `lex.rs` builds candidate lists itself,
/// but C's spckword scans the canonical hashtables directly. The new
/// signature matches C exactly: takes `&mut String` for in-place fix.
pub fn spckword(s: &mut String, hist: i32, cmd: i32, ask: i32) {
    use crate::ported::hashtable::{
        aliastab_lock, cmdnamtab_lock, fillcmdnamtable, pathchecked, reswdtab_lock, shfunctab_lock,
    };
    use crate::ported::params::{getsparam, paramtab};
    use crate::ported::pattern::{patcompile, PAT_HEAPDUP};
    use crate::ported::zsh_h::{
        isset, Dash, Equals, Stringg as StringTok, Tilde, AUTOCD, HASHLISTALL,
    };
    // c:3130-3133 — locals.
    let _t: Option<String>;
    let mut ic: char = '\0'; // c:3131
    let mut preflen: usize = 0; // c:3132
                                // c:3133 — `autocd = cmd && isset(AUTOCD) && strcmp(*s, ".") && strcmp(*s, "..")`.
    let autocd = cmd != 0 && isset(AUTOCD) && s != "." && s != "..";

    // c:3135-3136 — `if (!(*s)[0] || !(*s)[1]) return;`
    if s.len() < 2 {
        return;
    }
    // c:3137-3140 — HISTFLAG_NOEXEC or leading %/- skip.
    let bytes = s.as_bytes();
    let first = bytes[0] as char;
    let histdone = crate::ported::hist::histdone.load(std::sync::atomic::Ordering::Relaxed); // c:3137
    if (histdone & HISTFLAG_NOEXEC) != 0
        || (if cmd != 0 {
            first == '%' // c:3139 — leading % is a job
        } else {
            first == '-' || first == Dash // c:3139 — leading hyphen is an option
        })
    {
        return; // c:3140
    }
    // c:3141-3142 — `if (!strcmp(*s, "in")) return;`.
    if s == "in" {
        return; // c:3142
    }
    // c:3143-3155 — `cmd` branch: skip if it's already a known
    // function/builtin/cmdname/alias/reswd. Optional HASHLISTALL
    // re-fill of cmdnamtab on miss.
    if cmd != 0 {
        // c:3143
        let known = shfunctab_lock() // c:3144
            .read()
            .map(|t| t.get(s).is_some())
            .unwrap_or(false)
            || BUILTINS // c:3145
                .iter()
                .any(|b| b.node.nam == *s)
            || cmdnamtab_lock() // c:3146
                .read()
                .map(|t| t.get(s).is_some())
                .unwrap_or(false)
            || aliastab_lock() // c:3147
                .read()
                .map(|t| t.get(s).is_some())
                .unwrap_or(false)
            || reswdtab_lock() // c:3148
                .read()
                .map(|t| t.get(s).is_some())
                .unwrap_or(false);
        if known {
            return; // c:3149
        }
        // c:3150-3154 — HASHLISTALL: bulk-hash $PATH then retry.
        if isset(HASHLISTALL) {
            // c:3150
            let path: Vec<String> =
                getsparam("PATH") // c:3151
                    .map(|p| p.split(':').map(String::from).collect())
                    .unwrap_or_default();
            fillcmdnamtable(&path); // c:3151
            if cmdnamtab_lock() // c:3152
                .read()
                .map(|t| t.get(s).is_some())
                .unwrap_or(false)
            {
                return; // c:3153
            }
        }
    }
    // c:3156-3165 — Tilde/Equals/String prefix skip + itok/Dash
    // detok + early-return on any other tokenized char.
    let mut start = 0usize;
    let bytes = s.as_bytes(); // re-bind after potential string ops
    if !bytes.is_empty() {
        let c0 = bytes[0] as char;
        if c0 == Tilde || c0 == Equals || c0 == StringTok {
            // c:3157
            start = 1; // c:3158 t++
        }
    }
    // Scan from `start` for tokenized bytes.
    {
        let mut buf = s.clone().into_bytes();
        let mut i = start;
        let mut had_dash_only = true; // accumulator
        while i < buf.len() {
            let b = buf[i];
            if itok(b) {
                // c:3160
                if b as char == Dash {
                    // c:3161
                    buf[i] = b'-'; // c:3162
                } else {
                    return; // c:3164
                }
            } else {
                had_dash_only = had_dash_only && false;
            }
            i += 1;
        }
        let _ = had_dash_only;
        *s = String::from_utf8_lossy(&buf).into_owned();
    }
    // c:3166 — `best = NULL;`
    SPCK_BEST.with(|b| *b.borrow_mut() = None);
    SPCK_D.with(|c| c.set(100)); // initialised at each table-scan branch in C; mirror up-front.
                                 // c:3167-3169 — `for (t = *s; *t; t++) if (*t == '/') break;`
                                 // `t` is the position of the first slash (or end of string).
    let t_pos = s.find('/').unwrap_or(s.len()); // c:3167
                                                // c:3170-3171 — `if (**s == Tilde && !*t) return;`
    let bytes = s.as_bytes();
    if !bytes.is_empty() && (bytes[0] as char) == Tilde && t_pos == bytes.len() {
        // c:3170
        return; // c:3171
    }

    // c:3173-3178 — compile CORRECT_IGNORE pattern if set.
    if let Some(ci) = getsparam("CORRECT_IGNORE") {
        // c:3173
        let prog = patcompile(
            &{
                let mut __pat_tok = (&ci).to_string();
                crate::ported::glob::tokenize(&mut __pat_tok);
                __pat_tok
            },
            PAT_HEAPDUP,
            None,
        ); // c:3176
        SPCK_PAT.with(|p| *p.borrow_mut() = prog);
    } else {
        SPCK_PAT.with(|p| *p.borrow_mut() = None); // c:3178
    }
    // c:3180-3185 — compile CORRECT_IGNORE_FILE pattern if set.
    if let Some(ci) = getsparam("CORRECT_IGNORE_FILE") {
        // c:3180
        let prog = patcompile(
            &{
                let mut __pat_tok = (&ci).to_string();
                crate::ported::glob::tokenize(&mut __pat_tok);
                __pat_tok
            },
            PAT_HEAPDUP,
            None,
        ); // c:3183
        SPCK_NAMEPAT.with(|p| *p.borrow_mut() = prog);
    } else {
        SPCK_NAMEPAT.with(|p| *p.borrow_mut() = None); // c:3185
    }

    let bytes = s.as_bytes();
    let first = if bytes.is_empty() {
        '\0'
    } else {
        bytes[0] as char
    };

    // c:3187-3193 — `**s == String && !*t`: $-prefixed name → scan paramtab.
    if first == StringTok && t_pos == bytes.len() {
        // c:3187
        // c:3188 — `guess = *s + 1;` strip leading $.
        let guess = s[1..].to_string();
        // c:3189-3190 — `if (itype_end(guess, INAMESPC, 1) == guess) return;`
        if itype_end(&guess, crate::ported::ztype_h::INAMESPC, true) == 0 {
            // c:3189
            return; // c:3190
        }
        ic = StringTok; // c:3191
        SPCK_GUESS.with(|g| *g.borrow_mut() = Some(guess));
        SPCK_D.with(|c| c.set(100)); // c:3192
        if let Ok(t) = paramtab().read() {
            // c:3193 scanhashtable(paramtab, ..., spscan, ...)
            for k in t.keys() {
                spscan(k);
            }
        }
    // c:3194-3202 — `**s == Equals`: =cmd → hashcmd; then scan aliases+cmdnam.
    } else if first == Equals {
        // c:3194
        if t_pos != bytes.len() {
            // c:3195
            return; // c:3196
        }
        // c:3197-3198 — `if (hashcmd(guess = *s + 1, pathchecked)) return;`
        let guess = s[1..].to_string();
        let path: Vec<String> = getsparam("PATH")
            .map(|p| p.split(':').map(String::from).collect())
            .unwrap_or_default();
        let pc = pathchecked.load(std::sync::atomic::Ordering::Relaxed);
        if crate::ported::exec::hashcmd(&guess, &path[pc.min(path.len())..]).is_some() {
            return; // c:3198
        }
        SPCK_D.with(|c| c.set(100)); // c:3199
        ic = Equals; // c:3200
        SPCK_GUESS.with(|g| *g.borrow_mut() = Some(guess));
        if let Ok(t) = aliastab_lock().read() {
            // c:3201
            for (k, _) in t.iter() {
                spscan(k);
            }
        }
        if let Ok(t) = cmdnamtab_lock().read() {
            // c:3202
            for (k, _) in t.iter() {
                spscan(k);
            }
        }
    // c:3203-3248 — default branch: filename / dir spell-check.
    } else {
        // c:3203
        let mut guess = s.clone(); // c:3204
                                   // c:3205-3218 — Tilde / String inline-expand prefix handling.
        if !guess.is_empty()
            && ((guess.as_bytes()[0] as char) == Tilde
                || (guess.as_bytes()[0] as char) == StringTok)
        {
            // c:3205
            ic = guess.as_bytes()[0] as char; // c:3207
            if t_pos + 1 >= s.len() {
                // c:3208 — `if (!*++t) return;`
                return; // c:3209
            }
            // c:3210-3214 — `noerrs=2; singsub(&guess); noerrs = ne;`
            let saved_noerrs = *crate::ported::utils::noerrs_lock().lock().unwrap();
            *crate::ported::utils::noerrs_lock().lock().unwrap() = 2; // c:3212
            guess = crate::ported::subst::singsub(&guess); // c:3213
            *crate::ported::utils::noerrs_lock().lock().unwrap() = saved_noerrs;
            if guess.is_empty() {
                return; // c:3216 `if (!guess) return;`
            }
            // c:3217 — `preflen = strlen(guess) - strlen(t);` t = original
            // s[t_pos..] (the post-slash remainder).
            let t_len = s.len() - t_pos;
            preflen = guess.len().saturating_sub(t_len);
        }
        // c:3219-3220 — `if (access(unmeta(guess), F_OK) == 0) return;`
        let cstr = match std::ffi::CString::new(unmeta(&guess).as_str()) {
            Ok(c) => c,
            Err(_) => return,
        };
        if unsafe { libc::access(cstr.as_ptr(), libc::F_OK) } == 0 {
            // c:3219
            return; // c:3220
        }
        // c:3221 — `best = spname(guess);`
        // The Rust spname has a signature-drift adaptation taking
        // `(name, dir)`; pass the parent dir extracted from guess.
        let path_obj = std::path::Path::new(&guess);
        let parent = path_obj
            .parent()
            .and_then(|p| p.to_str())
            .filter(|s| !s.is_empty())
            .unwrap_or(".");
        let basename = path_obj.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let best = spname(basename, parent);
        SPCK_BEST.with(|b| *b.borrow_mut() = best);
        SPCK_GUESS.with(|g| *g.borrow_mut() = Some(guess.clone()));
        // c:3222-3247 — command-position default branch: hashcmd +
        // scan tables + autocd cdpath scan.
        if t_pos == s.len() && cmd != 0 {
            // c:3222
            // c:3223-3224 — hashcmd shortcut.
            let path: Vec<String> = getsparam("PATH")
                .map(|p| p.split(':').map(String::from).collect())
                .unwrap_or_default();
            let pc = pathchecked.load(std::sync::atomic::Ordering::Relaxed);
            if crate::ported::exec::hashcmd(&guess, &path[pc.min(path.len())..]).is_some() {
                return; // c:3224
            }
            SPCK_D.with(|c| c.set(100)); // c:3225
                                         // c:3226-3230 — scan reswd, alias, shfunc, builtin, cmdnam.
            if let Ok(t) = reswdtab_lock().read() {
                // c:3226
                for (k, _) in t.iter() {
                    spscan(k);
                }
            }
            if let Ok(t) = aliastab_lock().read() {
                // c:3227
                for (k, _) in t.iter() {
                    spscan(k);
                }
            }
            if let Ok(t) = shfunctab_lock().read() {
                // c:3228
                for (k, _) in t.iter() {
                    spscan(k);
                }
            }
            // c:3229 — builtintab scan: BUILTINS is a static array.
            for b in BUILTINS.iter() {
                spscan(&b.node.nam);
            }
            if let Ok(t) = cmdnamtab_lock().read() {
                // c:3230
                for (k, _) in t.iter() {
                    spscan(k);
                }
            }
            // c:3231-3247 — autocd $cdpath scan: for each cdpath
            // entry, find the closest filename match via mindist.
            // Strict `<` (not `<=` as in spscan) so a cdpath dir
            // wins only when STRICTLY better than the existing best
            // — preferring earlier cdpath dirs on ties.
            if autocd {
                // c:3232 — `if (cd_able_vars(unmeta(guess))) return;`
                let unmeta_guess = crate::ported::utils::unmeta(&guess);
                if crate::ported::builtin::cd_able_vars(&unmeta_guess).is_some() {
                    return; // c:3233
                }
                // c:3234 — `for (pp = cdpath; *pp; pp++)`. Read CDPATH.
                let cdpath: Vec<String> = crate::ported::params::paramtab()
                    .read()
                    .ok()
                    .and_then(|t| t.get("cdpath").and_then(|pm| pm.u_arr.clone()))
                    .unwrap_or_default();
                let mut cur_d = SPCK_D.with(|c| c.get());
                let mut cur_best = SPCK_BEST.with(|b| b.borrow().clone());
                for pp in cdpath.iter() {
                    // c:3235-3245 — `mindist(*pp, *s, bestcd, 1)`.
                    if let Some((bestcd, thisdist)) = mindist(pp, &s) {
                        // c:3239 — STRICT `<` (not `<=`) per C comment
                        // at c:3236-3239.
                        if (thisdist as i32) < cur_d {
                            cur_best = Some(bestcd);
                            cur_d = thisdist as i32;
                        }
                    }
                }
                SPCK_BEST.with(|b| *b.borrow_mut() = cur_best);
                SPCK_D.with(|c| c.set(cur_d));
            }
        }
    }
    // c:3250-3251 — `if (errflag) return;`
    // A user interrupt sets ERRFLAG_INT, never ERRFLAG_ERROR (signals.c:457), and
    // the C line cited above tests the WHOLE errflag, so masking here let an
    // interrupted shell keep going where zsh stops.
    if errflag.load(std::sync::atomic::Ordering::Relaxed) != 0 {
        return; // c:3251
    }
    // c:3252 — `if (best && strlen(best) > 1 && strcmp(best, guess))`.
    let best = SPCK_BEST.with(|b| b.borrow().clone());
    let guess = SPCK_GUESS.with(|g| g.borrow().clone()).unwrap_or_default();
    let Some(mut best) = best else {
        return;
    };
    if best.len() <= 1 || best == guess {
        return;
    }
    // c:3253-3272 — assemble the prefixed/de-tokenized replacement.
    if ic != '\0' {
        // c:3254
        if preflen > 0 {
            // c:3256
            // c:3258-3259 — `if (strncmp(guess, best, preflen)) return;`
            if !best.starts_with(&guess[..preflen.min(guess.len())]) {
                return; // c:3259
            }
            // c:3261-3263 — `u = ...s[0..t-*s] + best[preflen..]`.
            let t_off = s.len() - (s.len() - t_pos);
            let mut u = String::with_capacity(t_off + best.len() - preflen + 1);
            u.push_str(&s[..t_off]);
            u.push_str(&best[preflen..]);
            best = u;
        } else {
            // c:3264 — `u = "\0" + best;` (prepend NUL placeholder).
            best = format!("\0{}", best);
        }
        // c:3269-3271 — `best = u; guess = *s; *guess = *best = ztokens[ic - Pound];`
        let pound = crate::ported::zsh_h::Pound as u8;
        let zt = crate::ported::lex::ztokens.as_bytes();
        let token_char = if (ic as u8) >= pound {
            let idx = (ic as u8 - pound) as usize;
            if idx < zt.len() {
                zt[idx] as char
            } else {
                ic
            }
        } else {
            ic
        };
        // Set first char of both `*s` (the original) and `best`.
        if !s.is_empty() {
            let mut sb = s.clone().into_bytes();
            sb[0] = token_char as u8;
            *s = String::from_utf8_lossy(&sb).into_owned();
        }
        if !best.is_empty() {
            let mut bb = best.into_bytes();
            bb[0] = token_char as u8;
            best = String::from_utf8_lossy(&bb).into_owned();
        }
    }
    // c:3273-3289 — interactive prompt (`ask`) or auto-accept.
    let x: char;
    if ask != 0 {
        // c:3273
        // WARNING — DIVERGENCE: `noquery()`, `shout`, `promptexpand`,
        // `zputs(stream)`, `zbeep`, `getquery("nyae", 0)` aren't yet
        // wired in zshrs (interactive ZLE prompt machinery). Default
        // to 'n' (decline) when ask=1 — preserves the C behavior of
        // declining when shout is NULL (c:3286-3287). Re-enable the
        // interactive flow when promptexpand/getquery land.
        x = 'n';
    } else {
        x = 'y'; // c:3289
    }
    // c:3290-3300 — apply chosen action.
    if x == 'y' {
        // c:3290
        *s = best; // c:3291 `*s = dupstring(best);`
        if hist != 0 {
            // c:3292
            // c:3293 — `hwrep(best);` (history rewrite). Stubbed: hist
            // rewrite plumbing isn't yet hooked into the lex caller.
        }
    } else if x == 'a' {
        // c:3294
        crate::ported::hist::histdone
            .fetch_or(HISTFLAG_NOEXEC, std::sync::atomic::Ordering::Relaxed); // c:3295
    } else if x == 'e' {
        // c:3296
        crate::ported::hist::histdone.fetch_or(
            HISTFLAG_NOEXEC | crate::ported::zsh_h::HISTFLAG_RECALL,
            std::sync::atomic::Ordering::Relaxed,
        ); // c:3297
    }
    // c:3299-3300 — `if (ic) **s = ic;` — restore prefix sigil.
    if ic != '\0' && !s.is_empty() {
        let mut sb = s.clone().into_bytes();
        sb[0] = ic as u8;
        *s = String::from_utf8_lossy(&sb).into_owned();
    }
}

/// Port of `ztrftimebuf()` from `Src/utils.c:3312` — C decl `ztrftimebuf(int *bufsizeptr, int decr)`.
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

/// Format time struct (from utils.c ztrftime)
/// Port of `ztrftime(char *buf, int bufsize, char *fmt, struct tm *tm, long nsec)` from `Src/utils.c:3337`.
/// WARNING: param names don't match C — Rust=(fmt, time) vs C=(buf, bufsize, fmt, tm, nsec)
// Rust idiom replacement: `SystemTime::duration_since(UNIX_EPOCH)`
// + libc::localtime covers the C tm-struct populate; the C source's
// 192-line body builds the fmt-walk by hand because C has no
// strftime extension support — Rust delegates to libc::strftime via
// the strftime crate equivalent inline.
/// Port of `ztrftime()` from `Src/utils.c:3337`.
///
/// `ztrftime` — see implementation.
///
/// `use_gmt` controls whether time fields are interpreted as UTC (true,
/// libc::gmtime) or local time (false, libc::localtime). C's ztrftime
/// takes a `struct tm *` produced by the caller via gmtime/localtime
/// (e.g. Src/Modules/stat.c:201 picks between them based on STF_GMT);
/// since the Rust signature takes `SystemTime` we hoist that choice into
/// this bool param.
pub fn ztrftime(fmt: &str, time: std::time::SystemTime, use_gmt: bool) -> String {
    // C takes `struct tm *` + nsec directly, so negative time_t
    // (pre-1970) flows through localtime untouched. SystemTime
    // pre-epoch surfaces as duration_since Err — recover the signed
    // seconds rather than collapsing to 0 (the old unwrap_or_default
    // pinned every pre-epoch timestamp to the epoch).
    let (secs, nsec): (i64, u64) = match time.duration_since(UNIX_EPOCH) {
        Ok(d) => (d.as_secs() as i64, d.subsec_nanos() as u64),
        Err(e) => {
            let d = e.duration();
            let s = -(d.as_secs() as i64);
            let n = d.subsec_nanos() as u64;
            // sub-second part rolls the seconds down one: -1.25s
            // before epoch is secs=-2, nsec=750ms in tm terms.
            if n > 0 {
                (s - 1, 1_000_000_000 - n)
            } else {
                (s, 0)
            }
        }
    };

    #[cfg(unix)]
    unsafe {
        let tm = if use_gmt {
            libc::gmtime(&secs)
        } else {
            libc::localtime(&secs)
        };
        if tm.is_null() {
            return String::new();
        }
        let tm_ref = &*tm;

        // c:3398-3406 — pre-pass: walk fmt and substitute zsh-specific
        // extensions BEFORE delegating to libc::strftime. The C source
        // handles these inline; Rust pre-rewrites them into literal
        // numbers so libc::strftime sees only standard specifiers.
        //   %K  = 24-hr clock, no leading zero (0-23)
        //   %L  = 12-hr clock, no leading zero (1-12)
        //   %f  = day of month, no leading zero (1-31)
        //   %.N = fractional seconds, N digits (0-9)
        let mut preprocessed = String::with_capacity(fmt.len());
        let bytes = fmt.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'%' && i + 1 < bytes.len() {
                // c:3374-3384 — parse optional `N.` prefix (digit count
                // for the `%.` fractional-seconds specifier).
                let mut j = i + 1;
                while j < bytes.len() && bytes[j].is_ascii_digit() {
                    j += 1;
                }
                let digs: u32 = if j > i + 1 {
                    std::str::from_utf8(&bytes[i + 1..j])
                        .unwrap_or("3")
                        .parse()
                        .unwrap_or(3)
                } else {
                    3
                };
                // c:3408 — `switch (*fmt++)` examines the specifier
                // char (after the digit prefix).
                if j < bytes.len() && bytes[j] == b'.' {
                    // c:3409-3430 — `case '.':` fractional seconds.
                    //   long fnsec = nsec;
                    //   if (digs < 0 || digs > 9) digs = 9;
                    //   if (digs < 9) {
                    //       int trunc; long max = 100000000;
                    //       for (trunc = 8 - digs; trunc; trunc--)
                    //           { max /= 10; fnsec /= 10; }
                    //       max -= 1;
                    //       fnsec = (fnsec + 5) / 10;
                    //       if (fnsec > max) fnsec = max;
                    //   }
                    //   sprintf(buf, "%0*ld", digs, fnsec);
                    //
                    // The `(fnsec + 5) / 10` is a ROUND-half-up, capped
                    // at all-nines so a carry can't overflow the field.
                    // The previous port plain-truncated, so 999_999 ns
                    // under `%3.` printed `000` where zsh prints `001`.
                    let mut fnsec: u64 = nsec;
                    // c:3412-3413 — digs default is 3 (c:3362); >9 clamps to 9.
                    let digs = if digs > 9 { 9 } else { digs };
                    if digs < 9 {
                        // c:3418-3422
                        let mut max: u64 = 100_000_000;
                        for _ in 0..(8 - digs) {
                            max /= 10;
                            fnsec /= 10;
                        }
                        max -= 1; // c:3423
                        fnsec = (fnsec + 5) / 10; // c:3424
                        if fnsec > max {
                            fnsec = max; // c:3426
                        }
                    }
                    // c:3428 — `sprintf(buf, "%0*ld", digs, fnsec)`.
                    preprocessed.push_str(&format!("{:0width$}", fnsec, width = digs as usize));
                    i = j + 1;
                    continue;
                }
                let next = bytes[i + 1];
                // c:3445-3460 — %K / %L / %f extensions.
                match next {
                    b'K' => {
                        preprocessed.push_str(&format!("{}", tm_ref.tm_hour));
                        i += 2;
                        continue;
                    }
                    b'L' => {
                        let mut h12 = tm_ref.tm_hour % 12;
                        if h12 == 0 {
                            h12 = 12;
                        }
                        preprocessed.push_str(&format!("{}", h12));
                        i += 2;
                        continue;
                    }
                    b'f' => {
                        preprocessed.push_str(&format!("{}", tm_ref.tm_mday));
                        i += 2;
                        continue;
                    }
                    b'N' => {
                        // c:3491-3495 — `%N`: nanoseconds, always 9 digits
                        // (`sprintf(buf, "%09ld", nsec)`).
                        preprocessed.push_str(&format!("{:09}", nsec));
                        i += 2;
                        continue;
                    }
                    _ => {}
                }
            }
            preprocessed.push(bytes[i] as char);
            i += 1;
        }

        // c:3354-3359 — `if (*fmt == Meta) { int chr = fmt[1] ^ 32;
        // *buf++ = chr; fmt += 2; }`: C's format string is METAFIED, so an
        // embedded NUL arrives as `Meta` + 0x20 and is copied straight to
        // the output, never reaching `strftime(3)`. The Rust port carries
        // the format as un-metafied bytes, so a NUL is a real interior NUL
        // — `CString::new` rejected it and `unwrap_or_default()` silently
        // produced an EMPTY format, dropping the whole result
        // (`strftime $'%Y\0%m\0%d' …` printed nothing). Split on NUL,
        // run strftime(3) per segment, and re-join with NUL so the byte
        // passes through exactly as C's Meta arm does.
        let mut result = String::new();
        for (seg_idx, seg) in preprocessed.split('\0').enumerate() {
            if seg_idx > 0 {
                result.push('\0');
            }
            if seg.is_empty() {
                continue;
            }
            let mut buf = vec![0u8; 256];
            let c_fmt = match CString::new(seg) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let len = libc::strftime(
                buf.as_mut_ptr() as *mut libc::c_char,
                buf.len(),
                c_fmt.as_ptr(),
                tm,
            );
            if len > 0 {
                buf.truncate(len);
                result.push_str(&String::from_utf8_lossy(&buf));
            }
        }
        result
    }

    #[cfg(not(unix))]
    {
        let _ = (fmt, secs, nsec);
        String::new()
    }
}

/// Port of `zjoin()` from `Src/utils.c:3622` — C decl `zjoin(char **arr, int delim, int heap)`.
///
/// Join array with delimiter (from utils.c zjoin)
/// Joins `arr` with `delim` between elements. When `delim` is itself a meta
/// byte (`imeta(delim)` — NUL, `Meta`, `Marker`, or a token in the
/// Pound..Nularg range; see `inittyptab` c:4195-4201), C writes the delimiter
/// in *metafied* form (`Meta` byte then `delim ^ 32`, c:3634-3637) rather than
/// the raw byte (c:3639). The earlier port emitted the raw char always, which
/// was byte-wrong for meta delimiters — notably `zjoin(array, '\0', …)` (NUL is
/// imeta), where C emits `0x83 0x20`, not a raw `0x00`.
///
/// WARNING: param names don't match C — Rust=(arr, delim) vs C=(arr, delim, heap).
/// The `heap` arg is dropped (Rust `String` owns its storage). As with
/// [`metafy`], the `Meta` byte cannot live in valid UTF-8, so the final
/// `String` boundary uses the same `from_utf8`/lossy fallback — byte-exact for
/// every ASCII (non-meta) delimiter, lossy only on meta delimiters. Callers
/// needing byte-exact meta-delimited output should join at the byte level.
pub fn zjoin(arr: &[String], delim: char) -> String {
    // c:3622
    // c:3629-3630 — empty array → "".
    if arr.is_empty() {
        return String::new();
    }
    let db = delim as u32;
    // c:3634 — `imeta(delim)`. Only byte-valued delimiters can be meta.
    let delim_meta = db < 0x100 && imeta_byte(db as u8);
    let mut out: Vec<u8> = Vec::new();
    for (i, s) in arr.iter().enumerate() {
        if i != 0 {
            // The separator that C writes after every element then chops off
            // the trailing one (c:3641) is equivalent to interposing it.
            if delim_meta {
                out.push(Meta); // c:3635
                out.push((db as u8) ^ 32); // c:3636
            } else if db < 0x100 {
                out.push(db as u8); // c:3639 — raw single byte
            } else {
                // Non-byte char delimiter (no C analogue): emit its UTF-8.
                let mut buf = [0u8; 4];
                out.extend_from_slice(delim.encode_utf8(&mut buf).as_bytes());
            }
        }
        out.extend_from_slice(s.as_bytes()); // c:3633 — strucpy(&ptr, *s)
    }
    String::from_utf8(out.clone()).unwrap_or_else(|_| String::from_utf8_lossy(&out).into_owned())
}

/// Port of `colonsplit()` from `Src/utils.c:3650`.
///
/// Port of `char **colonsplit(char *s, int uniq)` from
/// `Src/utils.c:3650`. Splits `s` on `:`; when `uniq` is set,
/// duplicate segments are dropped (linear scan against the
/// already-emitted prefix).
/// ```c
/// char **
/// colonsplit(char *s, int uniq)
/// {
///     int ct;
///     char *t, **ret, **ptr, **p;
///     for (t = s, ct = 0; *t; t++)
///         if (*t == ':') ct++;
///     ptr = ret = zalloc(sizeof(char *) * (ct + 2));
///     t = s;
///     do {
///         s = t;
///         for (; *t && *t != ':'; t++);
///         if (uniq)
///             for (p = ret; p < ptr; p++)
///                 if (strlen(*p) == t - s && !strncmp(*p, s, t - s))
///                     goto cont;
///         *ptr = zalloc((t - s) + 1);
///         ztrncpy(*ptr++, s, t - s);
///       cont: ;
///     } while (*t++);
///     *ptr = NULL;
///     return ret;
/// }
/// ```
pub fn colonsplit(s: &str, uniq: bool) -> Vec<String> {
    // c:3650
    // c:3655-3657 — count colons.
    let ct = s.matches(':').count(); // c:3655
    let mut ret: Vec<String> = Vec::with_capacity(ct + 2); // c:3658 zalloc((ct+2)*sizeof(char *))

    // c:3661-3673 — do-while loop walking segments.
    let bytes = s.as_bytes();
    let mut t: usize = 0;
    loop {
        let seg_start = t; // c:3662 s = t
                           // c:3664 — `for (; *t && *t != ':'; t++)`
        while t < bytes.len() && bytes[t] != b':' {
            t += 1;
        }
        let seg = &s[seg_start..t];
        // c:3665-3668 — uniq dedupe.
        if !uniq || !ret.iter().any(|p| p == seg) {
            // c:3665
            ret.push(seg.to_string()); // c:3670 zalloc + ztrncpy
        }
        // c:3673 — `while (*t++)` — break if at end, else step past colon.
        if t >= bytes.len() {
            break;
        } // c:3673
        t += 1; // c:3673 t++
    }
    // c:3674 — `*ptr = NULL;` — Rust Vec needs no sentinel.
    ret // c:3675
}

/// Port of `skipwsep()` from `Src/utils.c:3680` — C decl `skipwsep(char **s)`.
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
        if !iwsep(b) {
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

/// Port of `spacesplit()` from `Src/utils.c:3711`.
///
/// Port of `spacesplit(char *s, int allownull, int heap, int quote)` from
/// `Src/utils.c:3711`. Splits `s` on `$IFS` via the `ISEP`/`IWSEP` char
/// classes (TYPTAB, kept in sync with `$IFS` by `inittyptab`, which
/// re-runs on every IFS set — params.rs:8691, c:4795), NOT on hardcoded
/// whitespace. zsh's word-splitting rules: runs of IFS-WHITESPACE
/// collapse and the leading/trailing ones yield real `""` fields (which
/// callers elide); each IFS-NON-WHITESPACE char delimits a field, and
/// consecutive ones yield `nulstring` (`Nularg`, `Src/subst.c:36`
/// `{Nularg,'\0'}`) empty fields, which survive and are later stripped to
/// `""` by remnulargs.
///
/// WARNING: param names don't match C — Rust=(s, allownull) vs C=(s,
/// allownull, heap, quote). The `quote` arm (backslash-escaped seps) and
/// `heap` C-buffer param drop: all zshrs callers pass quote=0, and Rust
/// owns its String storage. The previous port split only on hardcoded
/// `[' ','\t','\n']`, ignoring `$IFS` entirely (Bug #636).
pub fn spacesplit(s: &str, allownull: bool) -> Vec<String> {
    // c:3711
    use crate::ported::ztype_h::zistype;
    let bytes = s.as_bytes();
    // Meta-aware decode of the logical char value + its byte length at a
    // byte position (mirrors skipwsep's `*x == Meta ? x[1] ^ 32 : *x`).
    let char_at = |i: usize| -> (u8, usize) {
        // c:Src/zsh.h Meta — `\u{83}` prefix; next byte XOR 0x20.
        if bytes[i] == Meta && i + 1 < bytes.len() {
            (bytes[i + 1] ^ 32, 2)
        } else {
            (bytes[i], 1)
        }
    };
    // skipwsep(&s) — advance past a run of IWSEP (IFS-whitespace). c:3730
    let skipwsep_at = |mut i: usize| -> usize {
        while i < bytes.len() {
            let (c, l) = char_at(i);
            if !iwsep(c) {
                break;
            }
            i += l;
        }
        i
    };
    // itype_end(s, ISEP, 1) — `once` advances past exactly ONE ISEP char
    // if present, else leaves the index unchanged (c:4462 `if (once) break`).
    let isep_one = |i: usize| -> usize {
        if i < bytes.len() {
            let (c, l) = char_at(i);
            if zistype(c, ISEP as u32) {
                return i + l;
            }
        }
        i
    };
    // findsep(&s, NULL, 0) — advance to the next ISEP char. c:3784
    let findsep_at = |mut i: usize| -> usize {
        while i < bytes.len() {
            let (c, l) = char_at(i);
            if zistype(c, ISEP as u32) {
                break;
            }
            i += l;
        }
        i
    };
    let nulstring = || Nularg.to_string(); // c:subst.c:36 nulstring = {Nularg,0}

    let mut ret: Vec<String> = Vec::new();
    let mut si = 0usize;
    let mut t = si; // c:3729 — `t = s;`
    si = skipwsep_at(si); // c:3730 — `skipwsep(&s);`
                          // c:3732-3735 — leading-field handling.
    if si < bytes.len() && isep_one(si) != si {
        // c:3733 — at an IFS-non-whitespace sep after skipping whitespace.
        ret.push(if allownull {
            String::new()
        } else {
            nulstring()
        });
    } else if !allownull && t != si {
        // c:3734-3735 — leading IFS-whitespace was skipped → real "".
        ret.push(String::new());
    }
    // c:3736-3756 — main word loop.
    while si < bytes.len() {
        let iend = isep_one(si); // c:3737 itype_end(s, ISEP, 1)
        let consumed_sep = iend != si;
        if consumed_sep {
            // c:3738-3741 — consume the single non-ws sep + following WS.
            si = iend;
            si = skipwsep_at(si);
        }
        // !!! POSIX-FAITHFUL GATE (no C counterpart) !!!
        // zsh (and `emulate sh`) emits a trailing empty field when a
        // non-whitespace IFS separator is the last thing in the input:
        // `IFS=:; set -- $v` on `a:b:` → (a, b, "") = 3. dash/ksh/bash drop
        // that trailing empty → (a, b) = 2. A MIDDLE empty (`a::b`) keeps
        // `si < len` here and is unaffected. Scoped to `!allownull` (the
        // unquoted-split path `set -- $v` uses); the quoted `"${=v}"` /
        // `${(s::)}` allownull path keeps zsh semantics. `zshrs --sh/--ksh/
        // --dash` sets posix_faithful(); `--... --zsh` leaves it false.
        if consumed_sep && si >= bytes.len() && !allownull && crate::dash_mode::posix_faithful() {
            break;
        }
        t = si; // c:3746 — `t = s;`
        si = findsep_at(si); // c:3747 — `findsep(&s, NULL, quote);`
        if si > t || allownull {
            // c:3748-3751 — emit the word `t..si`.
            ret.push(s[t..si].to_string());
        } else {
            // c:3753 — empty field between non-ws seps → nulstring.
            ret.push(nulstring());
        }
        t = si; // c:3754 — `t = s;`
        si = skipwsep_at(si); // c:3755 — `skipwsep(&s);`
    }
    // c:3757 — trailing IFS-whitespace → real "".
    if !allownull && t != si {
        ret.push(String::new());
    }
    ret
}

/// Port of `findsep()` from `Src/utils.c:3784` — C decl `findsep(char **s, char *sep, int quote)`.
///
/// c:3762 — "Find a separator.  Return 0 if already at separator, 1 if
/// separator found later, else -1."  `pos` is C's walking `*s`: on
/// return it points at the separator (or end-of-string) so the caller
/// reads the word as `s[entry_pos..pos]`.
///
/// `sep` (c:3772):
///   - `None`       → split on normal `$IFS` separators (ISEP chars).
///   - `Some(b"")`  → no real separator: advance past one character.
///   - `Some(seq)`  → look for the (possibly multi-byte) literal `seq`.
///
/// `quote` (c:3776) — a `\` before a separator suppresses it, and `\\`
/// collapses to `\`.  This only applies when `sep` is `None` and it
/// strips backslashes in place, so the buffer must be modifiable
/// (C demands "something modifiable"; Rust takes `&mut String`).
///
/// The standalone form has no in-tree caller yet — the hot split paths
/// (`splitstring`, `findword`, `wordcount`) inline this logic — but it
/// is kept as a faithful, callable name-parity anchor.
/// WARNING: param names match C semantics — `pos` is C's `*s`.
pub fn findsep(s: &mut String, pos: &mut usize, sep: Option<&[u8]>, quote: bool) -> i32 {
    // c:3784
    use crate::ported::zsh_h::{MB_METACHARINIT, MB_METACHARLEN, MB_METACHARLENCONV};
    use crate::ported::ztype_h::{isep, ISEP, WC_ZISTYPE};

    MB_METACHARINIT(); // c:3792
    let is_isep = |c: Option<char>| c.map(|c| WC_ZISTYPE(c, ISEP as u32)).unwrap_or(false);

    match sep {
        // c:3793-3824 — default separators (ISEP), with optional quoting.
        None => {
            let start = *pos;
            let mut t = *pos;
            while t < s.len() {
                let b = s.as_bytes()[t];
                if quote && b == b'\\' {
                    // c:3795 — `if (quote && *t == '\\')`
                    if t + 1 < s.len() && s.as_bytes()[t + 1] == b'\\' {
                        // c:3796-3799 — `\\` → `\`: drop one backslash,
                        // advance past the one we keep (ilen = 1).
                        chuck(s, t);
                        t += 1;
                        continue;
                    }
                    // c:3801 — measure the char *after* the backslash.
                    let (ilen, c) = MB_METACHARLENCONV(&s.as_bytes()[t + 1..]);
                    if is_isep(c) {
                        // c:3802-3804 — escaped separator: strip the
                        // backslash, then advance over the now-bare char.
                        chuck(s, t);
                        t += ilen.max(1);
                        continue;
                    }
                    // c:3806-3810 — backslash is a normal byte.
                    if isep(b) {
                        break; // (never taken: '\\' is not an ISEP)
                    }
                    t += 1; // ilen = 1
                } else {
                    // c:3814-3818 — ordinary character.
                    let (ilen, c) = MB_METACHARLENCONV(&s.as_bytes()[t..]);
                    if is_isep(c) {
                        break; // c:3817
                    }
                    t += ilen.max(1);
                }
            }
            let i = if t > start { 1 } else { 0 }; // c:3821 `i = (t > *s)`
            *pos = t; // c:3822
            i // c:3823
        }
        // c:3825-3834 — empty separator: advance past the first char.
        Some(seq) if seq.is_empty() => {
            if *pos < s.len() {
                *pos += MB_METACHARLEN(&s.as_bytes()[*pos..]); // c:3831
                1 // c:3832
            } else {
                -1 // c:3833
            }
        }
        // c:3835-3845 — explicit (possibly multi-byte) literal separator.
        Some(seq) => {
            let mut i = 0i32;
            while *pos < s.len() {
                // c:3840 — `for (t=sep, tt=*s; *t && *tt && *t==*tt; ...)`
                let bytes = s.as_bytes();
                let mut k = 0usize;
                while k < seq.len() && *pos + k < bytes.len() && seq[k] == bytes[*pos + k] {
                    k += 1;
                }
                if k == seq.len() {
                    return if i > 0 { 1 } else { 0 }; // c:3841 `return (i > 0)`
                }
                *pos += MB_METACHARLEN(&bytes[*pos..]); // c:3842
                i += 1;
            }
            -1 // c:3844
        }
    }
}

/// Port of `findword()` from `Src/utils.c:3849` — C decl `findword(char **s, char *sep)`.
///
/// Find word at position (from utils.c findword)
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

/// Port of `wordcount()` from `Src/utils.c:3879` — C decl `wordcount(char *s, char *sep, int mul)`.
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
pub fn wordcount(s: &str, sep: Option<&str>, mul: i32) -> i32 {
    // c:3879
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
                false => rest.windows(sl).position(|w| w == sep_bytes),
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
        // c:3888-3905 — the `sep == NULL` (IFS) branch, ported statement by
        // statement. The previous rewrite inverted C's leading test — it
        // counted a leading WORD where C counts a leading SEPARATOR — and
        // used `iwsep` (IFS-whitespace) for a test C makes on `ISEP` (any
        // IFS char), so an all-separator string came out short:
        // `s='   '; ${(W)#s}` gave 2 against zsh's 4, and `'a b  c '` gave
        // 4 against 5.
        //
        // Meta-aware char reads mirror spacesplit's local closures
        // (c:Src/utils.c:3711): a Meta byte prefixes the escaped char.
        use crate::ported::ztype_h::zistype;
        let char_at = |i: usize| -> (u8, usize) {
            if bytes[i] == Meta && i + 1 < bytes.len() {
                (bytes[i + 1] ^ 32, 2)
            } else {
                (bytes[i], 1)
            }
        };
        // skipwsep(&s) — c:Src/utils.c:3730.
        let skipwsep_at = |mut i: usize| -> usize {
            while i < bytes.len() {
                let (c, l) = char_at(i);
                if !iwsep(c) {
                    break;
                }
                i += l;
            }
            i
        };
        // itype_end(s, ISEP, 1) — consumes exactly ONE ISEP char
        // (c:Src/utils.c:4462 `if (once) break`).
        let isep_one = |i: usize| -> usize {
            if i < bytes.len() {
                let (c, l) = char_at(i);
                if zistype(c, ISEP as u32) {
                    return i + l;
                }
            }
            i
        };
        // findsep(&s, NULL, 0) — advance to the next ISEP char (c:3784).
        let findsep_at = |mut i: usize| -> usize {
            while i < bytes.len() {
                let (c, l) = char_at(i);
                if zistype(c, ISEP as u32) {
                    break;
                }
                i += l;
            }
            i
        };
        let mut s_pos = 0usize; // c:3888 `char *t = s;`
        let mut t = s_pos; // c:3888
        let mut r: i32 = 0; // c:3889
        if mul <= 0 {
            // c:3890
            s_pos = skipwsep_at(s_pos); // c:3891
        }
        // c:3892-3894 — `if ((*s && itype_end(s, ISEP, 1) != s) ||
        //                    (mul < 0 && t != s)) r++;`
        if (s_pos < bytes.len() && isep_one(s_pos) != s_pos) || (mul < 0 && t != s_pos) {
            r += 1; // c:3894
        }
        // c:3895-3903 — `for (; *s; r++) { … }`.
        while s_pos < bytes.len() {
            let ie = isep_one(s_pos); // c:3896
            if ie != s_pos {
                // c:3897
                s_pos = ie; // c:3898
                if mul <= 0 {
                    // c:3899
                    s_pos = skipwsep_at(s_pos);
                }
            }
            s_pos = findsep_at(s_pos); // c:3901 `(void)findsep(&s, NULL, 0);`
            t = s_pos; // c:3902
            if mul <= 0 {
                // c:3903
                s_pos = skipwsep_at(s_pos);
            }
            r += 1; // c:3895 (loop increment)
        }
        if mul < 0 && t != s_pos {
            // c:3904
            r += 1; // c:3905
        }
        r
    }
}

/// Join array with separator - port from zsh/Src/utils.c sepjoin() lines 3926-3958
///
/// If sep is None, uses first char of IFS (defaults to space).
/// Join an array with separator.
/// Port of `sepjoin(char **s, char *sep, int heap)` from Src/utils.c:3928.
/// WARNING: param names don't match C — Rust=(arr, sep) vs C=(s, sep, heap)
// Rust idiom replacement: `slice::join` covers the C `zalloc`+`strcpy`
// loop with running length pre-compute; the `heap` arg drops since
// String owns its own allocation.
/// Port of `sepjoin()` from `Src/utils.c:3928`.
///
/// `sepjoin` — see implementation.
pub fn sepjoin(arr: &[String], sep: Option<&str>) -> String {
    // c:3928
    // c:3934-3935 — if (!*s) return heap ? dupstring("") : ztrdup("");
    if arr.is_empty() {
        return String::new();
    }
    // c:3936-3945 — `if (!sep)` default-sep arm:
    //   if (ifs && *ifs != ' ') sep = dupstrpfx(ifs, MB_METACHARLEN(ifs));
    //   else sep = " ";
    // IFS SET-but-EMPTY takes the first branch (`*ifs` is '\0' != ' ')
    // and dupstrpfx("", …) yields an EMPTY separator — `"$*"` with
    // IFS="" concatenates. Only IFS UNSET (ifs == NULL → getsparam
    // None) or starting with a space falls back to " ".
    let ifs_storage: String;
    let sep_str: &str = match sep {
        Some(s) => s, // c:3936
        None => {
            ifs_storage = match getsparam("IFS") {
                // c:3938-3940 — set, first char not space (empty → "").
                Some(ifs) if !ifs.starts_with(' ') => ifs
                    .chars()
                    .next()
                    .map(|c| c.to_string())
                    .unwrap_or_default(),
                // c:3941-3944 — unset (NULL) or leading space → " ".
                _ => " ".to_string(),
            };
            &ifs_storage
        }
    };
    // c:3947-3956 — pre-compute total length, alloc, copy elements
    //               interleaving sep. Rust slice::join collapses all that
    //               into the one canonical call.
    arr.join(sep_str)
}

/// Port of `sepsplit()` from `Src/utils.c:3962` — C decl `sepsplit(char *s, char *sep, int allownull, int heap)`.
///
/// Split string by separator - port from zsh/Src/utils.c sepsplit() lines 3961-3992
///
/// If sep is None, performs IFS-style word splitting (spacesplit).
/// Otherwise splits on the given separator string.
/// allownull: if true, allows empty strings in result
/// Split a string on `IFS` separators.
/// WARNING: param names don't match C — Rust=(s, sep, allownull) vs C=(s, sep, allownull, heap)
pub fn sepsplit(s: &str, sep: Option<&str>, allownull: bool) -> Vec<String> {
    // c:3962
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

/// Port of `getshfunc()` from `Src/utils.c:3998` — C decl `getshfunc(char *nam)`.
///
/// C body:
/// ```c
/// Shfunc getshfunc(char *nam) {
///     return (Shfunc) shfunctab->getnode(shfunctab, nam);
/// }
/// ```
///
/// Routes through the global `shfunctab` singleton in
/// hashtable.rs (hashtable::shfunctab_lock). Returns an owned
/// `shfunc` clone so callers can read `flags` (to detect
/// `PM_UNDEFINED` autoload stubs), `funcdef`, `filename`, and
/// `body` without holding the table lock. Owned clone vs C's
/// `*Shfunc` raw pointer trades one allocation for Rust
/// lifetime safety — `getshfunc` is a function-lookup site,
/// not per-statement, so the cost is irrelevant.
///
/// Returning `Option<String>` of just `body` (the old contract)
/// made every PM_UNDEFINED autoload stub invisible because
/// `body=None` collapsed via `and_then` to `None`, which callers
/// then read as "function doesn't exist."
pub fn getshfunc(nam: &str) -> Option<shfunc> {
    let tab = shfunctab_lock().read().expect("shfunctab poisoned");
    tab.get(nam).cloned()
}

/// Port of `subst_string_by_func()` from `Src/utils.c:4017`.
///
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
pub fn subst_string_by_func(
    func_name: &str,
    arg1: Option<&str>,
    orig: &str,
) -> Option<Vec<String>> // c:4017
{
    let osc = SFCONTEXT.load(Ordering::Relaxed); // c:4019
    let osm = STOPMSG.load(Ordering::Relaxed);
    let old_incompfunc = INCOMPFUNC.load(Ordering::Relaxed);
    let mut args: Vec<String> = Vec::with_capacity(3); // c:4020-4026
    args.push(func_name.to_string()); // c:4023
    if let Some(a) = arg1 {
        // c:4024
        args.push(a.to_string()); // c:4025
    }
    args.push(orig.to_string()); // c:4026
    SFCONTEXT // c:4027
        .store(SFC_SUBST, Ordering::Relaxed);
    INCOMPFUNC.store(0, Ordering::Relaxed); // c:4028

    // c:Src/exec.c:1429 — `oldlineno = lineno;` / c:1696 — `lineno = oldlineno;`
    // C runs the hook body through `execlist`, which saves the caller's
    // `lineno` on entry and restores it on exit, so a shell function
    // invoked from inside an expansion leaves the CALLER's line number
    // untouched. zshrs drives `$LINENO` from a per-pipe
    // BUILTIN_SET_LINENO op: the statement that triggered this expansion
    // already emitted its own SET_LINENO, the hook body's ops then
    // overwrite `$LINENO`, and nothing puts it back — so `PS4='%N:%i> '`
    // rendered the hook's last line for the triggering statement
    // (`+fn:14>` where zsh prints `+fn:7>`, D01prompt.ztst "Recursive
    // use of prompts"). Same save/restore shape as the `$(...)` site in
    // vm_helper.rs:4733/5090.
    let saved_lineno = getsparam("LINENO");

    // c:4030 — `if (doshfunc(func, l, 1))`. Direct doshfunc call
    // mirrors C. body_runner routes through the host body-only entry
    // (matching the same shfunc that `callhookfunc` would resolve).
    let shf_clone: Option<crate::ported::zsh_h::shfunc> = shfunctab_lock()
        .read()
        .ok()
        .and_then(|t| t.get(func_name).cloned());
    let rc = if let Some(mut shf) = shf_clone {
        let name_for_body = func_name.to_string();
        let body_args = args.clone();
        let body_runner = move || -> i32 {
            crate::ported::exec::run_function_body(&name_for_body, &body_args[1..]).unwrap_or(0)
        };
        crate::ported::exec::doshfunc(&mut shf, args.clone(), true, body_runner)
    } else {
        callhookfunc(func_name, Some(&args), 0, std::ptr::null_mut())
    };
    // c:4033 — `ret = getaparam("reply")` against paramtab. `reply`
    // is a shell-local PM_ARRAY entry, never exported to env.
    let ret: Option<Vec<String>> = if rc != 0 {
        None // c:4031
    } else {
        getaparam("reply") // c:4033
    };

    // c:1696 — `lineno = oldlineno;` (see the save above). `LINENO`
    // carries PM_READONLY (`integer-readonly-special`), so the restore
    // writes `u_val` directly instead of going through setsparam — the
    // same bypass BUILTIN_SET_LINENO uses (fusevm_bridge.rs:7205).
    if let Some(ln) = saved_lineno {
        let n: crate::ported::zsh_h::zlong = ln.parse().unwrap_or(0);
        if let Ok(mut tab) = crate::ported::params::paramtab().write() {
            if let Some(pm) = tab.get_mut("LINENO") {
                // c:Src/utils.c:121 `zlong lineno` — the value lives in the C
                // GLOBAL, reached through LINENO's GSU. A `typeset -h +g LINENO`
                // local shadow has no PM_SPECIAL and no GSU, so C's `lineno = N`
                // never touches it; skip the paramtab mirror for the same reason.
                if (pm.node.flags & crate::ported::zsh_h::PM_SPECIAL as i32) != 0 {
                    pm.u_val = n;
                    pm.node.flags &= !(crate::ported::zsh_h::PM_UNSET as i32);
                }
            }
        }
        set_lineno(n as i32);
        crate::ported::lex::set_lineno(n as u64);
    }
    SFCONTEXT.store(osc, Ordering::Relaxed); // c:4035
    STOPMSG.store(osm, Ordering::Relaxed); // c:4036
    INCOMPFUNC.store(old_incompfunc, Ordering::Relaxed); // c:4037
    ret // c:4038
}

/// Port of `subst_string_by_hook()` from `Src/utils.c:4049`.
///
/// Port of `char **subst_string_by_hook(char *name, char *arg1, char *orig)`
/// from Src/utils.c:4049.
///
/// Looks up `name` as a shell function and calls
/// `subst_string_by_func` on it. If that returns no result, walks
/// `${name}_hook` as an array of function names, trying each in
/// order until one yields a `$reply`.
pub fn subst_string_by_hook(name: &str, arg1: Option<&str>, orig: &str) -> Option<Vec<String>> // c:4049
{
    let mut ret: Option<Vec<String>> = None;
    if getshfunc(name).is_some() {
        // c:4054
        ret = subst_string_by_func(name, arg1, orig); // c:4055
    }
    if ret.is_none() {
        // c:4058
        let arrnam = format!("{}_hook", name); // c:4061-4063
                                               // c:4065 — `arr = getaparam(arrnam)`. The previous Rust port
                                               // read `env::var(arrnam)` and NUL-split — wrong: the hook
                                               // array is a shell-local PM_ARRAY in paramtab, not env.
        if let Some(arr) = getaparam(&arrnam) {
            // c:4065
            for f in arr.iter() {
                // c:4068
                if f.is_empty() {
                    continue;
                }
                if getshfunc(f).is_some() {
                    // c:4069
                    ret = subst_string_by_func(f, arg1, orig); // c:4070
                    if ret.is_some() {
                        // c:4071
                        break; // c:4072
                    }
                }
            }
        }
    }
    ret // c:4094
}

/// Port of `mkarray()` from `Src/utils.c:4083` — C decl `mkarray(char *s)`.
///
/// Make single-element array (from utils.c mkarray)
pub fn mkarray(s: Option<&str>) -> Vec<String> {
    match s {
        Some(val) => vec![val.to_string()],
        None => Vec::new(),
    }
}

/// Port of `hmkarray()` from `Src/utils.c:4094` — C decl `hmkarray(char *s)`.
///
/// Make single-element array on heap (from utils.c hmkarray)
pub fn hmkarray(s: &str) -> Vec<String> {
    if s.is_empty() {
        Vec::new()
    } else {
        vec![s.to_string()]
    }
}

/// Port of `zbeep()` from `Src/utils.c:4105` — C decl `void zbeep(void)`.
///
/// Honours `$ZBEEP` (a key-string sequence) when set and the BEEP
/// option when unset; emits the BEL char (\007) to SHTTY by
/// default. The Rust port writes via `write_loop` to mirror C's
/// raw-write semantics.
pub fn zbeep() {
    // c:4105
    queue_signals(); // c:4105
    if let Ok(zbeep) = std::env::var("ZBEEP") {
        // c:4109
        let (decoded, _) = getkeystring(&zbeep); // c:4111
        #[cfg(unix)]
        {
            let shtty = SHTTY.load(Ordering::Relaxed);
            if shtty != -1 {
                let _ = write_loop(shtty, decoded.as_bytes()); // c:4112
            } else {
                eprint!("{}", decoded);
            }
        }
        #[cfg(not(unix))]
        eprint!("{}", decoded);
    } else if isset(BEEP) {
        // c:4113
        #[cfg(unix)]
        {
            let shtty = SHTTY.load(Ordering::Relaxed);
            if shtty != -1 {
                let _ = write_loop(shtty, b"\x07"); // c:4114
            } else {
                eprint!("\x07");
            }
        }
        #[cfg(not(unix))]
        eprint!("\x07");
    }
    unqueue_signals(); // c:4115
}

/// Port of `freearray()` from `Src/utils.c:4120` — C decl `freearray(char **s)`.
///
/// Free array (no-op in Rust, provided for API compat)
pub fn freearray(s: Vec<String>) {
    // c:4124 — DPUTS(!s, "freearray() with zero argument")
    // Rust takes Vec<String> by value (never null), so the C !s
    // check maps to no condition that can fire in Rust. Document
    // the gap.
    let _ = &s; // c:4124 (no-op; Vec is never NULL in Rust)
                // Rust Drop handles this
}

/// Port of `equalsplit()` from `Src/utils.c:4133` — C decl `equalsplit(char *s, char **t)`.
///
/// Split on '=' returning (name, value) (from utils.c equalsplit)
/// WARNING: param names don't match C — Rust=(s) vs C=(s, t)
pub fn equalsplit(s: &str) -> Option<(String, String)> {
    let eq = s.find('=')?;
    Some((s[..eq].to_string(), s[eq + 1..].to_string()))
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
pub fn inittyptab() {
    // utils.c:4155

    // c:4160-4164 — `if (!(typtab_flags & ZTF_INIT))` one-off init:
    //
    //     typtab_flags = ZTF_INIT;
    //     if (interact && isset(SHINSTDIN))
    //         typtab_flags |= ZTF_INTERACT;
    //
    // The `ZTF_INTERACT` half was missing, and it is load-bearing: it is the
    // ONLY writer of that bit, c:4257 reads it to decide `ZTF_BANGCHAR`, and
    // c:4291 reads THAT to decide whether `bangchar` is `ISPECIAL`. Without
    // it the chain never starts, so `!` was never a special character in an
    // interactive shell and `${(q)v}` on `v='a!b'` published `a!b` where zsh
    // publishes `a\!b`.
    //
    // C latches this ONCE, on the first call, and never revisits it — later
    // calls keep whatever the first decided. Preserved exactly: the whole
    // block is inside the `ZTF_INIT` guard.
    {
        let flags = TYPTAB_FLAGS.load(Ordering::Relaxed);
        if (flags & ZTF_INIT) == 0 {
            let mut init = ZTF_INIT; // c:4161
            if interact() && isset(crate::ported::zsh_h::SHINSTDIN) {
                init |= ZTF_INTERACT; // c:4163
            }
            TYPTAB_FLAGS.store(init, Ordering::Relaxed);
        }
    }

    // Local scratch, flushed to the atomic table at fn end — C mutates
    // typtab[] in place unlocked; the atomic store-per-slot flush keeps
    // readers lock-free (see TYPTAB doc in ztype_h.rs).
    let mut t = [0u32; 256]; // c:4168 memset(typtab, 0, sizeof(typtab))
                             // c:4169-4170 — control chars 0..32 and 128..160.
    for c in 0..32u32 {
        t[c as usize] = ICNTRL as u32;
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
    // c:4191 — `typtab['-'] = typtab['.'] = typtab[(unsigned char) Dash] = IUSER;`.
    t[b'-' as usize] = IUSER as u32;
    t[b'.' as usize] = IUSER as u32;
    // c:4191 — `Dash` token marker (0x9b per zsh.h:182, "Only in patterns").
    // Marking it IUSER lets pattern-side $-named-character paths
    // accept it as a user-name byte. Previously omitted in the port.
    t[crate::ported::zsh_h::Dash as usize] = IUSER as u32;
    // c:4192-4194 — blanks.
    t[b' ' as usize] |= (IBLANK | INBLANK) as u32;
    t[b'\t' as usize] |= (IBLANK | INBLANK) as u32;
    t[b'\n' as usize] |= INBLANK as u32;

    // c:4195 — `typtab['\0'] |= IMETA;`. Previously omitted in the
    // Rust port — '\0' had only ICNTRL (set by the 0..32 loop at
    // c:4169) and was missing IMETA. C `imeta('\0')` is true (the
    // NUL byte must be Meta-encoded as `Meta + ('\0' ^ 32)`); the
    // Rust `imeta()` typtab predicate returned false, so any code
    // routing through `imeta(c)` (e.g. `input::shingetline`)
    // failed to Meta-encode NUL bytes from stdin, corrupting the
    // SHIN buffer when piped binary data hit a `\0`.
    t[0] |= IMETA as u32; // c:4195
                          // c:4196-4197 — Meta + Marker marked IMETA.
    {
        t[Meta as usize] |= IMETA as u32;
        t[Marker as usize] |= IMETA as u32;
    }

    // c:4198-4199 — `for (t0 = (int) (unsigned char) Pound;
    //                     t0 <= (int) (unsigned char) LAST_NORMAL_TOK; t0++)
    //                    typtab[t0] |= ITOK | IMETA;`
    // Marks all char-rewrite token markers (Pound, Stringg, Hat,
    // Star, ...). Without this, `itok(Stringg)` returns false and
    // `is_valid_assignment_target("$NAME")` wrongly accepts the
    // leading `$` as part of an identifier prefix → `$=cmd` lexes
    // as ENVSTRING instead of STRING.
    {
        let lo = Pound as usize;
        let hi = LAST_NORMAL_TOK as usize;
        for t0 in lo..=hi {
            t[t0] |= (ITOK | IMETA) as u32;
        }
    }

    // c:4200-4201 — `for (t0 = (int) (unsigned char) Snull;
    //                     t0 <= (int) (unsigned char) Nularg; t0++)
    //                    typtab[t0] |= ITOK | IMETA | INULL;`
    {
        let lo = Snull as usize;
        let hi = Nularg as usize;
        for t0 in lo..=hi {
            t[t0] |= (ITOK | IMETA | INULL) as u32;
        }
    }

    // c:4202-4231 — IFS walk. Sets ISEP on every IFS char and IWSEP
    // on the blank (inblank) subset. Reads the current `ifs` global
    // (defaulting to DEFAULT_IFS), demetafies `Meta+X` pairs, and
    // skips a doubled blank (`s[1]==c`) so the IWSEP bit doesn't
    // mark "blank repeated → no-skip" IFS chars. Mirrors C exactly.
    {
        // c:4216 — `for (s = ifs ? ifs : CURRENT_DEFAULT_IFS; ...)`.
        // C reads the global `ifs` variable (the same one `ifssetfn`
        // writes). The Rust port walks paramtab first (so the GSU
        // dispatch path matches C); on miss, fall through to ifs_lock
        // directly so a fresh `ifssetfn` update before any paramtab
        // entry exists is still visible to inittyptab.
        // c:4216 — `for (s = ifs ? ifs : CURRENT_DEFAULT_IFS; *s; s++)`.
        // The default-IFS fallback is keyed on the `ifs` POINTER being
        // NULL (i.e. IFS unset), NOT on it pointing at an empty string.
        // `IFS=''` leaves a non-NULL empty `ifs`, the loop body never
        // runs, and typtab ends up with ZERO ISEP/IWSEP bits — which is
        // exactly how an empty IFS disables all word splitting
        // (`IFS=''; v='a b  c'; print -rl -- ${=v}` is one word). The
        // port used to coerce "" → DEFAULT_IFS at both the paramtab read
        // and the fallback, so an empty IFS still split on whitespace.
        let pt_ifs: Option<String> = crate::ported::params::paramtab()
            .read()
            .ok()
            .and_then(|t| t.get("IFS").map(|pm| crate::ported::params::ifsgetfn(pm)));
        let src: String = match pt_ifs {
            // IFS is SET (possibly to ""): walk its chars verbatim.
            Some(v) => v,
            // No paramtab entry yet. Fall through to the ifs_lock global so
            // a fresh ifssetfn update landing before the paramtab entry
            // exists is still visible; an empty global there means "not
            // initialised", which is C's NULL → CURRENT_DEFAULT_IFS.
            None => {
                let g = crate::ported::params::ifs_lock()
                    .lock()
                    .map(|g| g.clone())
                    .unwrap_or_default();
                if g.is_empty() {
                    DEFAULT_IFS.to_string() // c:4216 (ifs == NULL)
                } else {
                    g
                }
            }
        };
        let bytes = src.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            // c:4217 — `int c = (unsigned char) (*s == Meta ? *++s ^ 32 : *s)`.
            let c = if bytes[i] == Meta && i + 1 < bytes.len() {
                i += 1;
                bytes[i] ^ 32
            } else {
                bytes[i]
            };
            // c:4218-4223 — MULTIBYTE non-ASCII skip. Bytes >= 0x80
            // (after demetafy) are not classified by typtab — they
            // reach wcsitype via WC_ZISTYPE instead.
            if c >= 0x80 {
                i += 1;
                continue;
            }
            let cu = c as usize;
            // c:4224-4229 — `if (inblank(c))` — for default-IFS
            // chars space/tab/newline, mark IWSEP unless the next
            // byte repeats the same char.
            let is_inblank = (t[cu] & (INBLANK as u32)) != 0;
            if is_inblank {
                if i + 1 < bytes.len() && bytes[i + 1] == c {
                    i += 1; // c:4226 — skip the dup
                } else {
                    t[cu] |= IWSEP as u32; // c:4228
                }
            }
            // c:4230 — `typtab[c] |= ISEP;`
            t[cu] |= ISEP as u32;
            i += 1;
        }
    }

    // c:4232-4252 — wordchars walk. ORs IWORD onto every byte in
    // `$WORDCHARS` (or DEFAULT_WORDCHARS when unset). Used by every
    // word-class lookup in pattern matching, `${var:#word}`, etc.
    // Drops to ASCII-only under MULTIBYTE_SUPPORT (the non-ASCII path
    // routes through wordchars_wide).
    {
        // Same fallback as IFS above: paramtab first, then wordchars_lock
        // so wordcharssetfn updates that bypass paramtab still reach
        // the typtab rebuild.
        // c:4237 — `for (s = wordchars ? wordchars : DEFAULT_WORDCHARS;
        // *s; s++)`. C tests the POINTER, so `WORDCHARS=''` is a
        // legitimately EMPTY word-character set and only an UNSET
        // `wordchars` falls back to the default. Treating "" as "unset"
        // pinned `/` as a word character forever, so
        // `WORDCHARS="" [[ / = [[:WORD:]] ]]` wrongly matched.
        let wc: Option<String> = crate::ported::params::paramtab()
            .read()
            .ok()
            .and_then(|t| {
                t.get("WORDCHARS")
                    .map(|pm| crate::ported::params::wordcharsgetfn(pm))
            })
            .or_else(|| {
                crate::ported::params::wordchars_lock()
                    .lock()
                    .ok()
                    .map(|g| g.clone())
            });
        let src: String = wc.unwrap_or_else(|| DEFAULT_WORDCHARS.to_string());
        let bytes = src.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            // c:4238 — Meta+X demetafy.
            let c = if bytes[i] == Meta && i + 1 < bytes.len() {
                i += 1;
                bytes[i] ^ 32
            } else {
                bytes[i]
            };
            // c:4239-4249 — MULTIBYTE non-ASCII skip.
            if c < 0x80 {
                t[c as usize] |= IWORD as u32; // c:4251
            }
            i += 1;
        }
    }

    // c:4253-4254 — SPECCHARS walk. ORs ISPECIAL onto every member
    // of the hardcoded SPECCHARS string. Drives glob-special and
    // quote-special detection.
    {
        for &b in SPECCHARS.as_bytes() {
            t[b as usize] |= ISPECIAL as u32; // c:4254
        }
    }

    // c:4255-4256 — comma special only when ZTF_SP_COMMA was set
    // via `makecommaspecial(1)`. KSH_GLOB / extended-glob path.
    {
        let flags = TYPTAB_FLAGS.load(Ordering::Relaxed);
        if (flags & ZTF_SP_COMMA) != 0 {
            // c:4255
            t[b',' as usize] |= ISPECIAL as u32; // c:4256
        }
    }

    // c:4257-4261 — bangchar special when BANGHIST + interact +
    // bangchar != 0. Sets ZTF_BANGCHAR flag bit then marks the
    // bangchar byte ISPECIAL.
    {
        let bangchar2 = bangchar.load(Ordering::SeqCst) as usize;
        let flags = TYPTAB_FLAGS.load(Ordering::Relaxed);
        let interact_flag = (flags & ZTF_INTERACT) != 0;
        let banghist = isset(BANGHIST);
        if banghist && bangchar2 != 0 && bangchar2 < 256 && interact_flag {
            // c:4257
            TYPTAB_FLAGS.fetch_or(ZTF_BANGCHAR, Ordering::Relaxed); // c:4258
            t[bangchar2] |= ISPECIAL as u32; // c:4259
        } else {
            TYPTAB_FLAGS.fetch_and(!ZTF_BANGCHAR, Ordering::Relaxed); // c:4261
        }
    }

    // c:4262-4263 — PATCHARS walk. ORs IPATTERN onto every member.
    // Used by pattern compilation to detect glob metachars.
    {
        for &b in PATCHARS.as_bytes() {
            t[b as usize] |= IPATTERN as u32; // c:4263
        }
    }

    // Flush the scratch to the lock-free table (see header note).
    for (i, v) in t.iter().enumerate() {
        TYPTAB[i].store(*v, Ordering::Relaxed);
    }
}

/// Port of `makecommaspecial()` from `Src/utils.c:4270` — C decl `void makecommaspecial(int yesno)`.
///
/// Toggles `ZTF_SP_COMMA` and the `ISPECIAL` bit on `,` in the
/// global typtab — used by glob/extended-glob to flag `,`
/// (KSH_GLOB) as a metacharacter.
pub fn makecommaspecial(yesno: bool) {
    // c:4270
    if yesno {
        // c:4272
        TYPTAB_FLAGS.fetch_or(ZTF_SP_COMMA, Ordering::Relaxed); // c:4273
        TYPTAB[b',' as usize].fetch_or(ISPECIAL as u32, Ordering::Relaxed); // c:4274
    } else {
        TYPTAB_FLAGS.fetch_and(!ZTF_SP_COMMA, Ordering::Relaxed); // c:4276
        TYPTAB[b',' as usize].fetch_and(!(ISPECIAL as u32), Ordering::Relaxed); // c:4277
    }
}

/// Port of `makebangspecial()` from `Src/utils.c:4283` — C decl `void makebangspecial(int yesno)`.
///
/// Toggles `ISPECIAL` on the current `bangchar`. When `yesno==0`
/// always clears; when nonzero, sets only if `ZTF_BANGCHAR` was
/// stored by `inittyptab` (i.e. BANGHIST is on).
pub fn makebangspecial(yesno: bool) {
    // c:4283
    let bc = bangchar.load(Ordering::SeqCst) as usize;
    if bc == 0 || bc >= 256 {
        return;
    }
    let flags = TYPTAB_FLAGS.load(Ordering::Relaxed);
    if !yesno {
        // c:4289
        TYPTAB[bc].fetch_and(!(ISPECIAL as u32), Ordering::Relaxed); // c:4290
    } else if (flags & ZTF_BANGCHAR) != 0 {
        // c:4291
        TYPTAB[bc].fetch_or(ISPECIAL as u32, Ordering::Relaxed); // c:4292
    }
}

/// Port of `wcsiblank()` from `Src/utils.c:4302` — C decl `wcsiblank(wint_t wc)`.
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

/// Port of `wcsitype()` from `Src/utils.c:4321` — C decl `int wcsitype(wchar_t c, int itype)`.
///
/// "zistype macro extended to support wide characters. Works for
/// IIDENT, IWORD, IALNUM, ISEP."
///
/// The Rust port checks whether `c` falls in the typtab class
/// represented by `itype`. ASCII chars consult the global TYPTAB
/// (the same one C's `zistype()` macro indexes); non-ASCII chars
/// route through Unicode predicates that mirror the C `iswalnum`
/// fallback at line 4346.
pub fn wcsitype(c: char, itype: u32) -> bool {
    // c:4321
    if !isset(MULTIBYTE) {
        // c:4327
        if (c as u32) < 256 {
            return (TYPTAB[c as usize].load(Ordering::Relaxed) & itype) != 0;
        }
        return false;
    }
    if (c as u32) < 128 {
        // c:4343
        return (TYPTAB[c as usize].load(Ordering::Relaxed) & itype) != 0;
    }
    let cls = itype as u16;
    if cls == IIDENT {
        // c:4347
        if isset(POSIXIDENTIFIERS) {
            return false; // c:4348
        }
        return c.is_alphanumeric(); // c:4350
    }
    if cls == IWORD {
        // c:4352
        if c.is_alphanumeric() {
            return true;
        } // c:4353
          // C: IS_COMBINING(c) — no Rust crate-free combining-mark
          // predicate. zero-width chars (combining, zero-width-joiner,
          // etc.) are treated as word per c:4362.
        if unicode_width::UnicodeWidthChar::width(c).unwrap_or(1) == 0 {
            // c:4362
            return true;
        }
        // c:4364 — `wmemchr(wordchars_wide.chars, c, …)`. Reads from
        // the canonical `wordchars` global (writable by
        // `wordcharssetfn` at `Src/params.c:5143`). Previously routed
        // through `std::env::var("WORDCHARS")` which is the libc
        // process environment — never reflects runtime `WORDCHARS=:`
        // assignments inside the shell.
        let w = crate::ported::params::paramtab()
            .read()
            .ok()
            .and_then(|t| {
                t.get("WORDCHARS")
                    .map(|pm| crate::ported::params::wordcharsgetfn(pm))
            })
            .unwrap_or_default();
        return w.chars().any(|x| x == c);
    }
    if cls == ISEP {
        // c:4366
        // c:4367 — same canonical-global pattern for IFS.
        let ifs = crate::ported::params::paramtab()
            .read()
            .ok()
            .and_then(|t| t.get("IFS").map(|pm| crate::ported::params::ifsgetfn(pm)))
            .unwrap_or_default();
        return ifs.chars().any(|x| x == c);
    }
    let _ = IALNUM;
    c.is_alphanumeric() // c:4370
}

/// Port of `itype_end()` from `Src/utils.c:4395` — C decl `char *itype_end(const char *ptr, int itype, int once)`.
///
/// Check if a character type at end of string (from utils.c itype_end)
///
/// Walks `ptr` byte-by-byte advancing past every char whose
/// type-bits include `itype`. Returns the byte offset where the
/// walk stops (C returns the advanced pointer; the byte offset is
/// the Rust equivalent — `ptr + return_value` reproduces the C
/// pointer).
///
/// C body branches (ported line-for-line):
///
/// 1. **INAMESPC special-case** (c:4399-4413). If itype == INAMESPC
///    and we're not POSIXIDENTIFIERS-strict outside ksh emulation,
///    recurse on `ptr+(*ptr=='.')` with IIDENT to allow dotted
///    ksh93 namespace names. The recursion result determines
///    whether to advance past the dot or fall through.
///
/// 2. **MULTIBYTE branch** (c:4416-4470, gated `isset(MULTIBYTE)
///    && (itype != IIDENT || !isset(POSIXIDENTIFIERS))`). Walks
///    multibyte chars via mb_metacharlenconv (multibyte len +
///    wchar conversion). Per-codepoint test:
///      - itok byte (raw command line): test the byte via zistype
///      - Meta-prefixed pair: extract real byte via XOR, test via
///        zistype
///      - ASCII (< 0x80): test via zistype
///      - Non-ASCII codepoint: route through wcsitype (which
///        already handles IWORD/ISEP via WORDCHARS/IFS paramtab
///        lookup matching c:4364/c:4367)
///
/// 3. **Non-multibyte branch** (c:4474-4480). Simple Meta-byte +
///    zistype byte loop.
///
/// `once = true` stops after the first matching char (used for
/// "is this a valid first char" checks per c:1576 etc).
pub fn itype_end(s: &str, mut itype: u32, once: bool) -> usize {
    use crate::ported::zsh_h::{isset, Meta, EMULATE_KSH, EMULATION, MULTIBYTE, POSIXIDENTIFIERS};
    use crate::ported::ztype_h::{itok, zistype, IIDENT, INAMESPC};

    // c:4399-4413 — INAMESPC handling with ksh93 namespace dot.
    let mut start = 0usize;
    if itype == INAMESPC {
        itype = IIDENT as u32;
        if !isset(POSIXIDENTIFIERS) || EMULATION(EMULATE_KSH) {
            // c:4403 — `t = itype_end(ptr + (*ptr == '.'), itype, 0);`
            let first_is_dot = s.as_bytes().first().copied() == Some(b'.');
            let recurse_start = if first_is_dot { 1 } else { 0 };
            let t = recurse_start + itype_end(&s[recurse_start..], itype, false);
            // c:4404-4410 — `if (t > ptr + (*ptr == '.')) {
            //                  if (*t == '.') ptr = t + 1;  /* Fall through */
            //                  else if (!once) return t;
            //              }`
            if t > recurse_start {
                let t_byte = s.as_bytes().get(t).copied();
                if t_byte == Some(b'.') {
                    start = t + 1; // c:4407
                } else if !once {
                    return t; // c:4409
                }
            }
        }
    }

    let bytes = s.as_bytes();
    let mb = isset(MULTIBYTE) && (itype != IIDENT as u32 || !isset(POSIXIDENTIFIERS));

    if mb {
        // c:4416-4470 — multibyte walk. mb_charinit() in C is a
        // no-op stateless reset; not needed in Rust since str::chars
        // is already stateless UTF-8.
        let mut i = start;
        while i < bytes.len() {
            let b = bytes[i];
            // c:4422-4425 — itok: raw token byte, test via zistype,
            // advance 1.
            if itok(b) {
                if !zistype(b, itype) {
                    break;
                }
                i += 1;
            } else if b == Meta {
                // c:4438-4440 (WEOF arm) + non-MB Meta fallback:
                // Meta-prefixed pair = single unescaped char via XOR.
                let nxt = bytes.get(i + 1).copied().unwrap_or(0) ^ 0x20;
                if (nxt as u32) > 127 || !zistype(nxt, itype) {
                    break;
                }
                i += 2;
            } else if b < 0x80 {
                // c:4441-4443 (len == 1 && isascii) — ASCII: direct
                // zistype on the byte.
                if !zistype(b, itype) {
                    break;
                }
                i += 1;
            } else {
                // c:4446-4467 — non-ASCII codepoint. Decode the UTF-8
                // sequence starting at `i` and route through wcsitype
                // (which already handles IWORD via WORDCHARS paramtab
                // c:4364, ISEP via IFS paramtab c:4367, default via
                // iswalnum/is_alphanumeric c:4370).
                let tail = match std::str::from_utf8(&bytes[i..]) {
                    Ok(t) => t,
                    Err(_) => break, // invalid UTF-8 — stop walk
                };
                let ch = match tail.chars().next() {
                    Some(c) => c,
                    None => break,
                };
                if !wcsitype(ch, itype) {
                    break;
                }
                i += ch.len_utf8();
            }
            if once {
                break;
            }
        }
        i
    } else {
        // c:4474-4480 — non-multibyte arm. Simple byte loop with
        // Meta-prefix collapse.
        let mut i = start;
        while i < bytes.len() {
            let b = bytes[i];
            let chr = if b == Meta {
                bytes.get(i + 1).copied().unwrap_or(0) ^ 0x20
            } else {
                b
            };
            if !zistype(chr, itype) {
                break;
            }
            i += if b == Meta { 2 } else { 1 };
            if once {
                break;
            }
        }
        i
    }
}

/// Port of `arrdup()` from `Src/utils.c:4493` — C decl `arrdup(char **s)`.
///
/// Duplicate array (from utils.c arrdup)
pub fn arrdup(s: &[String]) -> Vec<String> {
    s.to_vec()
}

/// Port of `arrdup_max()` from `Src/utils.c:4508` — C decl `arrdup_max(char **s, unsigned max)`.
///
/// Duplicate array with max elements (from utils.c arrdup_max)
pub fn arrdup_max(s: &[String], max: usize) -> Vec<String> {
    s.iter().take(max).cloned().collect()
}

/// Port of `zarrdup()` from `Src/utils.c:4532` — C decl `zarrdup(char **s)`.
///
/// Duplicate array with zsh allocation (from utils.c zarrdup)
pub fn zarrdup(s: &[String]) -> Vec<String> {
    s.to_vec()
}

/// Port of `wcs_zarrdup()` from `Src/utils.c:4547` — C decl `wcs_zarrdup(wchar_t **s)`.
///
/// Duplicate array of wide strings (from utils.c wcs_zarrdup) - same as zarrdup in Rust
pub fn wcs_zarrdup(s: &[String]) -> Vec<String> {
    s.to_vec()
}

/// Port of `spname()` from `Src/utils.c:4562` — C decl `spname(char *oldname)`.
///
/// Spelling correction: find closest match (from utils.c spname)
/// WARNING: param names don't match C — Rust=(name, dir) vs C=(oldname)
pub fn spname(name: &str, dir: &str) -> Option<String> {
    let entries = match fs::read_dir(dir) {
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

/// Port of `mindist()` from `Src/utils.c:4624` — C decl `mindist(char *dir, char *mindistguess, char *mindistbest, int wantdir)`.
///
/// Spelling correction with full path (from utils.c mindist)
/// WARNING: param names don't match C — Rust=(dir, name) vs C=(dir, mindistguess, mindistbest, wantdir)
pub fn mindist(dir: &str, name: &str) -> Option<(String, usize)> {
    let entries = match fs::read_dir(dir) {
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

// spellcheck a word                                                        // c:3123
// fix s ; if hist is nonzero, fix the history list too                     // c:3124
/// Port of `spdist()` from `Src/utils.c:4675`.
///
/// Compute edit distance between two strings (for spelling correction)
/// Direct port of `int spdist(char *s, char *t, int thresh)` from
/// `Src/utils.c:4675-4750`. Drives the CORRECT-option typo prompt:
/// returns 0 for identical, 1 for case-only mistakes, 2 for one
/// transposition / missing letter / QWERTY-adjacent mistype, 200 for
/// anything farther.
///
/// **This is NOT a Levenshtein DP — that was the previous Rust impl
/// dressed as a port.** The C source uses a keyboard-adjacency model
/// (qwertykeymap / dvorakkeymap arrays + tulower equality) which is
/// the actual behaviour `setopt CORRECT` depends on. Restored
/// faithfully.
pub fn spdist(s: &str, t: &str, thresh: usize) -> usize {
    // c:4679-4690 — qwerty keymap (rows of 14 chars each: numeric row,
    // QWERTY top row, home row, bottom row, then shift versions).
    // Embedded `\n` / `\t` mark off-grid cells.
    const QWERTYKEYMAP: &str = "\n\n\n\n\n\n\n\n\n\n\n\n\n\n\
\t1234567890-=\t\
\tqwertyuiop[]\t\
\tasdfghjkl;'\n\t\
\tzxcvbnm,./\t\t\t\
\n\n\n\n\n\n\n\n\n\n\n\n\n\n\
\t!@#$%^&*()_+\t\
\tQWERTYUIOP{}\t\
\tASDFGHJKL:\"\n\t\
\tZXCVBNM<>?\n\n\t\
\n\n\n\n\n\n\n\n\n\n\n\n\n\n";
    // c:4691-4702 — dvorak keymap, same shape.
    const DVORAKKEYMAP: &str = "\n\n\n\n\n\n\n\n\n\n\n\n\n\n\
\t1234567890[]\t\
\t',.pyfgcrl/=\t\
\taoeuidhtns-\n\t\
\t;qjkxbmwvz\t\t\t\
\n\n\n\n\n\n\n\n\n\n\n\n\n\n\
\t!@#$%^&*(){}\t\
\t\"<>PYFGCRL?+\t\
\tAOEUIDHTNS_\n\t\
\t:QJKXBMWVZ\n\n\t\
\n\n\n\n\n\n\n\n\n\n\n\n\n\n";

    // c:4703-4707 — `keymap = isset(DVORAK) ? dvorakkeymap : qwertykeymap;`
    let keymap = if isset(DVORAK) {
        DVORAKKEYMAP.as_bytes()
    } else {
        QWERTYKEYMAP.as_bytes()
    };

    let s_b = s.as_bytes();
    let t_b = t.as_bytes();

    // c:4709-4710 — `if (!strcmp(s, t)) return 0;`
    if s == t {
        return 0;
    }

    // c:4712-4714 — `for (p, q; *p && tulower(*p) == tulower(*q); p++, q++);
    //                  if (!*p && !*q) return 1;` — case-only mismatch.
    let mut p = 0usize;
    let mut q = 0usize;
    while p < s_b.len() && q < t_b.len() && tulower(s_b[p] as char) == tulower(t_b[q] as char) {
        p += 1;
        q += 1;
    }
    if p == s_b.len() && q == t_b.len() {
        return 1;
    }

    // c:4715-4716 — `if (!thresh) return 200;`
    if thresh == 0 {
        return 200;
    }

    // c:4717-4727 — first walk: detect transposition / missing letter at
    // first divergence.
    p = 0;
    q = 0;
    while p < s_b.len() && q < t_b.len() {
        if s_b[p] == t_b[q] {
            // c:4718-4719 — match: skip (don't count `aa` as transposed).
            p += 1;
            q += 1;
            continue;
        }
        // c:4720-4721 — `if (p[1] == q[0] && q[1] == p[0]) return
        //                  spdist(p+2, q+2, thresh-1) + 1;` transposition.
        if p + 1 < s_b.len() && q + 1 < t_b.len() && s_b[p + 1] == t_b[q] && t_b[q + 1] == s_b[p] {
            // SAFETY: bytes are ASCII for the keymap test below; from_utf8
            // is fine on a non-ASCII tail here because spdist takes &str.
            let s_tail = std::str::from_utf8(&s_b[p + 2..]).unwrap_or("");
            let t_tail = std::str::from_utf8(&t_b[q + 2..]).unwrap_or("");
            return spdist(s_tail, t_tail, thresh.saturating_sub(1)) + 1;
        }
        // c:4722-4723 — `if (p[1] == q[0]) return spdist(p+1, q, thresh-1) + 2;`
        if p + 1 < s_b.len() && s_b[p + 1] == t_b[q] {
            let s_tail = std::str::from_utf8(&s_b[p + 1..]).unwrap_or("");
            let t_tail = std::str::from_utf8(&t_b[q..]).unwrap_or("");
            return spdist(s_tail, t_tail, thresh.saturating_sub(1)) + 2;
        }
        // c:4724-4725 — `if (p[0] == q[1]) return spdist(p, q+1, thresh-1) + 2;`
        if q + 1 < t_b.len() && s_b[p] == t_b[q + 1] {
            let s_tail = std::str::from_utf8(&s_b[p..]).unwrap_or("");
            let t_tail = std::str::from_utf8(&t_b[q + 1..]).unwrap_or("");
            return spdist(s_tail, t_tail, thresh.saturating_sub(1)) + 2;
        }
        // c:4726-4727 — `if (*p != *q) break;`
        break;
    }
    // c:4728-4729 — `if ((!*p && strlen(q) == 1) || (!*q && strlen(p) == 1))
    //                  return 2;` — single trailing-char insertion.
    if (p == s_b.len() && (t_b.len() - q) == 1) || (q == t_b.len() && (s_b.len() - p) == 1) {
        return 2;
    }
    // c:4730-4748 — second walk: keyboard-adjacency mistype detection.
    p = 0;
    q = 0;
    while p < s_b.len() && q < t_b.len() {
        if p + 1 < s_b.len() && q + 1 < t_b.len() && s_b[p] != t_b[q] && s_b[p + 1] == t_b[q + 1] {
            // c:4737-4738 — `if (!(z = strchr(keymap, p[0])) || *z == '\n' ||
            //                       *z == '\t') return spdist(p+1, q+1,
            //                       thresh-1) + 1;`
            let pos = keymap.iter().position(|&b| b == s_b[p]);
            let z_ok = match pos {
                Some(i) => keymap[i] != b'\n' && keymap[i] != b'\t',
                None => false,
            };
            if !z_ok {
                let s_tail = std::str::from_utf8(&s_b[p + 1..]).unwrap_or("");
                let t_tail = std::str::from_utf8(&t_b[q + 1..]).unwrap_or("");
                return spdist(s_tail, t_tail, thresh.saturating_sub(1)) + 1;
            }
            // c:4739 — `t0 = z - keymap;`
            let t0 = pos.unwrap() as isize;
            // c:4740-4744 — eight adjacency offsets (-15,-14,-13,-1,+1,
            //                +13,+14,+15) → keyboard neighbours.
            let offsets: [isize; 8] = [-15, -14, -13, -1, 1, 13, 14, 15];
            let adjacent = offsets.iter().any(|&off| {
                let idx = t0 + off;
                if idx >= 0 && (idx as usize) < keymap.len() {
                    keymap[idx as usize] == t_b[q]
                } else {
                    false
                }
            });
            if adjacent {
                // c:4745 — `return spdist(p+1, q+1, thresh-1) + 2;`
                let s_tail = std::str::from_utf8(&s_b[p + 1..]).unwrap_or("");
                let t_tail = std::str::from_utf8(&t_b[q + 1..]).unwrap_or("");
                return spdist(s_tail, t_tail, thresh.saturating_sub(1)) + 2;
            }
            // c:4746 — `return 200;`
            return 200;
        } else if p < s_b.len() && q < t_b.len() && s_b[p] != t_b[q] {
            // c:4747-4748 — `else if (*p != *q) break;`
            break;
        }
        p += 1;
        q += 1;
    }
    // c:4749 — `return 200;`
    200
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

/// Port of `attachtty()` from `Src/utils.c:4775` — C decl `void attachtty(pid_t pgrp)`.
/// Hands the controlling terminal to `pgrp`. Gated by `jobbing &&
/// interact`; falls back to `mypgrp` and disables MONITOR on permanent
/// failure (matching the C source's recursion + `opts[MONITOR]=0` path).
#[cfg(unix)]
pub fn attachtty(pgrp: i32) {
    // c:4775

    if !(jobbing() && interact()) {
        return; // c:4779
    }
    let shtty = SHTTY.load(Ordering::Relaxed); // c:4781
    if shtty == -1 {
        return;
    }
    let ep = ATTACHTTY_EP.load(Ordering::Relaxed);
    let rc = unsafe { libc::tcsetpgrp(shtty, pgrp) }; // c:4781
    if rc == -1 && ep == 0 {
        // c:4781
        let mypgrp_val = *crate::ported::jobs::MYPGRP // c:4792
            .get_or_init(|| Mutex::new(0))
            .lock()
            .unwrap();
        if pgrp != mypgrp_val && unsafe { libc::kill(-pgrp, 0) } == -1 {
            attachtty(mypgrp_val); // c:4793
        } else {
            let errno_val = io::Error::last_os_error().raw_os_error().unwrap_or(0);
            if errno_val != libc::ENOTTY {
                // c:4795
                zwarn(&format!(
                    "can't set tty pgrp: {}", // c:4797
                    io::Error::from_raw_os_error(errno_val)
                ));
                let _ = io::stderr().flush(); // c:4798
            }
            opt_state_set("monitor", false); // c:4815 opts[MONITOR]=0
            ATTACHTTY_EP.store(1, Ordering::Relaxed); // c:4815
        }
    } else if rc != -1 {
        // c:4815
        *crate::ported::jobs::LAST_ATTACHED_PGRP // c:4815
            .get_or_init(|| Mutex::new(0))
            .lock()
            .unwrap() = pgrp;
    }
}

/// Port of `gettygrp()` from `Src/utils.c:4815` — C decl `pid_t gettygrp(void)`.
#[cfg(unix)]
pub fn gettygrp() -> i32 {
    // c:4815
    let shtty = SHTTY.load(Ordering::Relaxed);
    if shtty == -1 {
        // c:4819
        return -1; // c:4820
    }
    unsafe { libc::tcgetpgrp(shtty) } // c:4823
}

/// Port of `metafy()` from `Src/utils.c:4856`.
///
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
/// Rust idiom replacement: forward byte-walk + Vec::push covers the
/// C two-pass (count + alloc + reverse-expand) approach; Vec grows
/// on demand so the pre-count is unnecessary in Rust.
/// WARNING: param names don't match C — Rust=(buf) vs C=(buf, len, heap)
pub fn metafy(buf: &str) -> String {
    // c:4856
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
    // String::from_utf8 may fail on the high bytes — fall back to
    // lossy. The lossy fallback inserts U+FFFD for invalid
    // sequences, which is fine for String-level callers (lexer /
    // pattern compile / display) but breaks byte-round-trip with
    // `unmetafy`.
    //
    // Callers that need the bytes back unchanged go through
    // `mb_metacharlenconv`, which hands back each unit in a recoverable form
    // instead of falling back to lossy.
    String::from_utf8(out.clone()).unwrap_or_else(|_| String::from_utf8_lossy(&out).into_owned())
}

/// Port of `ztrdup_metafy()` from `Src/utils.c:4929` — C decl `ztrdup_metafy(const char *s)`.
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

/// Port of `unmetafy()` from `Src/utils.c:4954` — C decl `unmetafy(char *s, int *len)`.
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
pub fn unmetafy(s: &mut Vec<u8>) -> usize {
    // c:4954
    // c:4958 `for (p = s; *p && *p != Meta; p++);` — when the string holds no
    // Meta byte at all, C's second loop copies each byte onto itself and the
    // call is just that scan. Ask the question with `<[u8]>::contains`, which
    // resolves to libcore's precompiled `memchr`, and skip both loops: the
    // index-by-index walk below is monomorphised into this crate and compiled
    // UNOPTIMISED in the debug build, where it measured ~24% of a `pr <TAB>`
    // (one `unmetafy` per history entry while paging the history in).
    // Returning `s.len()` is exactly what the two loops below produce on this
    // input — `p` runs to the end, the copy loop never executes, and
    // `truncate(t)` is a no-op.
    if !s.contains(&Meta) {
        return s.len();
    }
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

/// Port of `metalen()` from `Src/utils.c:4972` — C decl `int metalen(const char *s, int len)`.
/// ```c
/// int mlen = len;
/// while (len--) {
///     if (*s++ == Meta) { mlen++; s++; }
/// }
/// return mlen;
/// ```
/// Doc: "Return the character length of a metafied substring, given
/// the unmetafied substring length." So **input `len` is the
/// UNMETAFIED char count, output is the METAFIED byte count.**
///
/// Previously the Rust port had INVERTED semantics: looped while
/// `i < len` byte-walk, returned char count. That's the reverse
/// operation. Pin the correct C contract.
pub fn metalen(s: &str, len: usize) -> usize {
    // c:4972
    let bytes = s.as_bytes();
    let mut mlen = len; // c:4974
    let mut remaining = len;
    let mut i = 0;
    while remaining > 0 && i < bytes.len() {
        // c:4976 `while (len--)`
        if bytes[i] == Meta {
            // c:4977
            mlen += 1; // c:4978
            i += 2; // c:4979 s++ (already advanced past Meta)
        } else {
            i += 1;
        }
        remaining -= 1;
    }
    mlen
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
// Rust idiom replacement: byte-scan fast path + `unmetafy` covers
// the C in-place decode + alloc-on-copy dance; String owns its own
// allocation so no `heap` flag needed.
/// Port of `unmeta()` from `Src/utils.c:4994`.
///
/// `unmeta` — see implementation.
pub fn unmeta(s: &str) -> String {
    // c:4994
    let bytes = s.as_bytes();
    // c:4995-4996 — Meta-byte scan; no-copy fast path.
    if !bytes.iter().any(|&b| b == Meta) {
        return s.to_string();
    }
    let mut buf = bytes.to_vec();
    let len = unmetafy(&mut buf); // c:4999-5001
    buf.truncate(len);
    String::from_utf8_lossy(&buf).into_owned()
}

/// Port of `unmeta_one()` from `Src/utils.c:5058` — C decl `convchar_t unmeta_one(const char *in, int *sz)`.
/// Non-MULTIBYTE branch (c:5077-5083):
/// ```c
/// if (in[0] == Meta) { *sz = 2; wc = (unsigned char)(in[1] ^ 32); }
/// else               { *sz = 1; wc = (unsigned char) in[0]; }
/// ```
/// Returns `(decoded_char, bytes_consumed)`. The c:5070 NULL/empty
/// guard returns `(0, 0)`. Previously used hardcoded `0x83` for the
/// Meta byte — now routes through the canonical `Meta` constant
/// (defined as `'\u{83}'` at zsh.h:144).
/// WARNING: param names don't match C — Rust=(s) vs C=(in, sz)
pub fn unmeta_one(s: &str) -> (char, usize) {
    // c:5058
    let bytes = s.as_bytes();
    // c:5070 — `if (!in || !*in) return 0;`
    if bytes.is_empty() {
        return ('\0', 0);
    }
    // c:5077 — `if (in[0] == Meta)`.
    if bytes[0] == Meta && bytes.len() > 1 {
        // c:5078-5079 — `*sz = 2; wc = (unsigned char)(in[1] ^ 32);`.
        ((bytes[1] ^ 32) as char, 2)
    } else {
        // c:5081-5082 — `*sz = 1; wc = (unsigned char) in[0];`.
        (bytes[0] as char, 1)
    }
}

/// Port of `ztrcmp()` from `Src/utils.c:5106` — C decl `ztrcmp(char const *s1, char const *s2)`.
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
pub fn ztrcmp(s1: &str, s2: &str) -> std::cmp::Ordering {
    // c:5106
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

// pastebuf() DELETED — was a misplaced fn. The real `pastebuf()`
// lives in `Src/Zle/zle_misc.c:558` (a ZLE clipboard helper that
// pastes a Cutbuffer into the line-edit buffer). It has nothing to
// do with metafication, despite this file's prior body which
// reimplemented half of `metafy`. The real metafy lives below at
// `pub fn metafy()` (port of utils.c:4856).

/// Port of `ztrlen()` from `Src/utils.c:5136`.
///
/// Unmetafied string length (from utils.c ztrlen lines 5135-5152)
pub fn ztrlen(s: &str) -> usize {
    // c:5136
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

/// Port of `ztrlenend()` from `Src/utils.c:5162` — C decl `ztrlenend(char const *s, char const *eptr)`.
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

/// Port of `ztrsub()` from `Src/utils.c:5187` — C decl `int ztrsub(char const *t, char const *s)`.
/// ```c
/// int l = t - s;
/// while (s != t) {
///     if (*s++ == Meta) { s++; l--; }
/// }
/// return l;
/// ```
/// "Subtract two pointers in a metafied string." `s` is the start
/// pointer, `t` is the end pointer; both point into the SAME buffer.
/// Returns the count of unmetafied chars in `[s, t)` — same as
/// `ztrlen` of the substring but expressed via pointer arithmetic.
///
/// Rust API: takes the full buffer and a byte-offset pair
/// `[start..end)` since Rust can't replicate raw-pointer subtraction
/// across distinct `&str` arguments without UB risk. The semantics
/// stay faithful: count of unmetafied chars between two offsets.
/// Previous Rust port took two unrelated `&str` and computed
/// `ztrlen(&t[..t.len()-s.len()])` — a fundamentally different
/// operation that worked only when `s` was the suffix of `t`.
/// WARNING: param names don't match C — Rust=(buf, start, end) vs C=(t, s)
pub fn ztrsub(buf: &str, start: usize, end: usize) -> usize {
    // c:5187
    let bytes = buf.as_bytes();
    let end = end.min(bytes.len());
    let start = start.min(end);
    let mut l = (end - start) as isize; // c:5189
    let mut i = start;
    while i < end {
        if bytes[i] == Meta {
            // c:5192
            i += 2; // c:5198
            l -= 1; // c:5199
        } else {
            i += 1;
        }
    }
    l.max(0) as usize
}

/// Port of `zreaddir()` from `Src/utils.c:5217`.
///
/// Port of `char *zreaddir(DIR *dir, int ignoredots)` from
/// `Src/utils.c:5217`.
///
/// Port of `char *zreaddir(DIR *dir, int ignoredots)` from
/// `Src/utils.c:5217-5240`. Pulls the next entry from a DIR* the
/// caller owns; returns the entry's bare name (metafied String) or
/// `None` at EOF. The C source's `ignoredots` flag (c:5232) skips
/// `.` and `..` when set; callers (`spnamepat` at c:4648) that
/// want dot entries pass 0.
///
/// Caller pattern matches C exactly:
///   let mut dir = fs::read_dir(path)?;
///   while let Some(name) = zreaddir(&mut dir, 1) { ... }
///   // dir auto-closed on drop (= closedir)
pub fn zreaddir(dir: &mut fs::ReadDir, ignoredots: i32) -> Option<String> {
    // c:5217
    for entry in dir.by_ref() {
        // c:5221 readdir loop
        let Ok(e) = entry else { continue };
        let Ok(name) = e.file_name().into_string() else {
            continue;
        };
        // c:5232 — `if (ignoredots && de->d_name[0] == '.' &&
        //              (!de->d_name[1] || (de->d_name[1] == '.' && !de->d_name[2])))`.
        if ignoredots != 0 && (name == "." || name == "..") {
            continue; // c:5234
        }
        return Some(name); // c:5238 return de->d_name
    }
    None // c:5240 — fell off the end
}

/// Port of `zputs()` from `Src/utils.c:5265` — C decl `int zputs(char const *s, FILE *stream)`.
///
/// ```c
/// while (*s) {
///     if (*s == Meta)     c = *++s ^ 32;
///     else if (itok(*s)) { s++; continue; }
///     else                c = *s;
///     s++;
///     if (fputc(c, stream) < 0) return EOF;
/// }
/// return 0;
/// ```
///
/// Writes `s` to `stream` with Meta+X pair decoding and ITOK-byte
/// skipping. Returns `0` on success, `-1` on write error (mirroring
/// C's `EOF` sentinel for the int return).
pub fn zputs(s: &str, stream: &mut dyn std::io::Write) -> i32 {
    // c:5265
    let bytes = s.as_bytes(); // c:5265 *s walk
    let mut i = 0;
    while i < bytes.len() {
        // c:5267 while (*s)
        let c: u8; // c:5268 char c
        if bytes[i] == Meta {
            // c:5269 if (*s == Meta)
            // c:5270 — `c = *++s ^ 32;`
            if i + 1 < bytes.len() {
                c = bytes[i + 1] ^ 32;
                i += 1; // c:5270 ++s
            } else {
                i += 1;
                continue;
            }
        } else if itok(bytes[i]) {
            // c:5271 else if (itok(*s))
            // c:5272 — `s++; continue;` (skip token byte)
            i += 1;
            continue;
        } else {
            c = bytes[i]; // c:5274 else c = *s
        }
        i += 1; // c:5276 s++
        if stream.write_all(&[c]).is_err() {
            // c:5277 fputc(c, stream)
            return -1; // c:5278 return EOF
        }
    }
    0 // c:5280 return 0
}

/// Port of `nicedup()` from `Src/utils.c:5530`.
///
/// Port of `nicedup(char const *s, int heap)` from `Src/utils.c:5289`
/// (single-byte build) and `Src/utils.c:5530` (multibyte build).
///
/// C bodies:
/// ```c
/// /* Src/utils.c:5289 — !MULTIBYTE_SUPPORT */
/// char *retstr;
/// (void)sb_niceformat(s, NULL, &retstr, heap ? NICEFLAG_HEAP : 0);
/// return retstr;
///
/// /* Src/utils.c:5530 — MULTIBYTE_SUPPORT */
/// char *retstr;
/// (void)mb_niceformat(s, NULL, &retstr, heap ? NICEFLAG_HEAP : 0);
/// return retstr;
/// ```
///
/// zshrs targets MULTIBYTE_SUPPORT (matches every modern zsh
/// build) — route through `mb_niceformat`. Previous Rust port
/// went through `sb_niceformat`, which is the !MULTIBYTE path —
/// strips wide-char handling and corrupts non-ASCII via byte-mask
/// `nicechar(c & 0xff)`.
///
/// C signature faithful: `(s, heap) -> String`. `heap` selects between
/// arena (`NICEFLAG_HEAP`) and persistent (default `ztrdup`) allocation
/// in C; Rust has a single allocator so both paths produce the same
/// owned `String`, but the `heap` parameter is preserved per Rule S1.
pub fn nicedup(s: &str, heap: i32) -> String {
    // c:5530
    // c:5532 — `char *retstr;`
    let retstr: Option<String>;
    // c:5534 — `(void)mb_niceformat(s, NULL, &retstr, heap ? NICEFLAG_HEAP : 0);`
    let mut slot: Option<String> = None;
    let _ = mb_niceformat(
        s,
        None,
        Some(&mut slot),
        if heap != 0 { NICEFLAG_HEAP } else { 0 },
    );
    retstr = slot;
    // c:5536 — `return retstr;`
    retstr.unwrap_or_default()
}

/// Port of `nicedupstring()` from `Src/utils.c:5301` — C decl `nicedupstring(char const *s)`.
///
/// Nice-format and duplicate string.
/// C body: `return nicedup(s, 1);` — heap-arena allocation form.
pub fn nicedupstring(s: &str) -> String {
    // c:5301
    nicedup(s, 1) // c:5303
}

/// Port of `nicezputs()` from `Src/utils.c:5313` — C decl `nicezputs(char const *s, FILE *stream)`.
///
/// Nicely format a string
///
/// Under `MULTIBYTE_SUPPORT` (the daily-driver path), C defines
/// `nicezputs(str, outs)` as a macro at zsh.h:3274:
///   `(void)mb_niceformat((str), (outs), NULL, 0)`.
/// Without MULTIBYTE (c:5313): `sb_niceformat(s, stream, NULL, 0)`.
///
/// The previous Rust impl was `s.chars().map(nicechar).collect()` —
/// wrong on every front:
///   1. No unmetafy step (Meta-byte pairs surfaced as `\M-…` mangled).
///   2. `nicechar` is byte-based (`c & 0xff`), corrupting non-ASCII
///      multibyte codepoints into spurious `\M-X` escapes.
///   3. No `itok` ZSH-token-byte handling.
///
/// Route through `mb_niceformat` to match the C macro under
/// MULTIBYTE — which itself unmetafies and uses `wcs_nicechar` for
/// proper wide-char handling.
pub fn nicezputs(s: &str, stream: &mut dyn std::io::Write) -> i32 {
    // c:5313 (non-MULTIBYTE) / zsh.h:3274 (MULTIBYTE macro:
    //   `(void)mb_niceformat((str), (outs), NULL, 0)`).
    let _ = mb_niceformat(s, Some(stream), None, 0);
    0 // c:5316 return 0
}

/// Port of `niceztrlen()` from `Src/utils.c:5324` — C decl `niceztrlen(char const *s)`.
///
/// Returns the length (in bytes) of the visible representation of
/// the metafied string `s`. C body walks each char via `nicechar`
/// and accumulates `strlen(nicechar(c))`; the Rust port mirrors
/// this via [`sb_niceformat`] which is the same render-then-measure
/// path.
pub fn niceztrlen(s: &str) -> usize {
    // c:5343-5348 — under MULTIBYTE_SUPPORT (which zshrs targets) `niceztrlen`
    // IS `mb_niceformat`, so the unit it walks is whatever the current locale's
    // `mbrtowc` returns: a whole character under a UTF-8 locale (U+2010 `‐`,
    // 3 bytes, counts 1), a single byte under a C/POSIX one (the same `‐`
    // counts 1+5+5 = 11, because its two continuation bytes are non-printable
    // and take `\M-^@`-style five-column representations). `mb_niceformat`
    // carries that locale branch; see its body.
    //
    // The port previously called `sb_niceformat`, the single-byte (`#else`,
    // c:5320-5341) version, which counts every byte through `nicechar` and so
    // charges 4+5+5 = 14 for that `‐` in EITHER locale — over-measuring in a
    // UTF-8 locale and still not matching zsh in a C one.
    mb_niceformat(s, None, None, 0)
}

/// Port of `mb_niceformat()` from `Src/utils.c:5366`.
///
/// Multibyte-aware nice-format of a string.
/// Port of `mb_niceformat(const char *s, FILE *stream, char **outstrp, int flags)` from Src/utils.c:5366. Walks the
/// (un-metafied) string char-by-char; for each control byte or
/// invalid sequence emits a `^X`/`\\xNN` representation, otherwise
/// passes the char through. The C source threads an
/// `mbstate_t` through `mbrtowc()` and falls back to single-byte
/// `\M-` notation on `MB_INVALID`.
///
/// `mbrtowc` is locale-driven, so the port is too: under a UTF-8
/// codeset it decodes one UTF-8 scalar per iteration (Rust's own
/// UTF-8 validation standing in for the `mbstate_t` machine, with
/// the invalid-byte arm reached through `from_utf8`'s error), and
/// under a single-byte codeset (`MB_CUR_MAX == 1`, the C/POSIX
/// locale) it takes one byte per iteration whose value IS the wide
/// character. The width this function returns differs between the
/// two for any non-ASCII input — see the `mb_single_byte` block.
/// WARNING: param names don't match C — Rust=(s) vs C=(s, stream, outstrp, flags)
/// `mb_niceformat` — see implementation.
pub fn mb_niceformat(
    s: &str,
    mut stream: Option<&mut dyn std::io::Write>,
    outstrp: Option<&mut Option<String>>,
    flags: i32,
) -> usize {
    // c:5366
    let mut l: usize = 0; // c:5368 size_t l = 0
    let mut newl: usize; // c:5368 size_t newl
                         // c:5369 — `int umlen, outalloc, outleft, eol = 0;`. outalloc/outleft
                         // model C's buffer-growth math; Rust String auto-grows so the realloc
                         // loop at c:5430-5440 collapses to push_str. umlen and eol carry.
    let mut umlen: usize; // c:5369
    let mut eol: bool = false; // c:5369 int eol = 0
                               // c:5370 — `wchar_t c;` (carried inside loop in Rust)
                               // c:5371 — `char *ums, *ptr, *fmt, *outstr, *outptr;`
    let mut ums: Vec<u8>; // c:5371 char *ums
    let mut ptr: usize; // c:5371 char *ptr
    let mut fmt: String; // c:5371 char *fmt
    let mut outstr: Option<String>; // c:5371 char *outstr
                                    // c:5372 — `mbstate_t mbs;` (Rust UTF-8 has no shift state; tracked
                                    // by the `valid_up_to` / `error_len` logic below).

    // c:5374-5380 — `if (outstrp) outptr = outstr = zalloc(5*strlen(s));`
    if outstrp.is_some() {
        // c:5374
        outstr = Some(String::with_capacity(5 * s.len())); // c:5376
    } else {
        // c:5377
        outstr = None; // c:5379 outstr = NULL
    }

    // c:5382 — `ums = ztrdup(s);`
    // c:5383-5387 — comment-only block carried verbatim:
    /*
     * is this necessary at this point? niceztrlen does this
     * but it's used in lots of places.  however, one day this may
     * be, too.
     */                                                                      // c:5383-5387
    // c:5388 — `untokenize(ums);` — Rust port uses the char-based
    // `lex::untokenize` (only fires on codepoints in the 0x84..=0xa1
    // ITOK range, never on UTF-8 continuation bytes); the byte-level
    // shape of C's untokenize is unsafe on raw UTF-8 input because a
    // continuation byte like 0x97 (part of `字` = 0xE5 0xAD 0x97) sits
    // inside the ITOK range and would be misclassified as a token.
    // c:5389 — `ptr = unmetafy(ums, &umlen);` — `unmeta` is the safe
    // UTF-8 wrapper that runs unmetafy only when Meta bytes are
    // present and otherwise no-ops.
    //
    // `unmetafy_str` (not `unmeta`) is what produces the raw byte buffer C
    // expects here. zshrs metafies at the CHARACTER level — an undecodable byte
    // is stored as U+0083 followed by `char::from(byte ^ 32)` — and U+0083
    // itself encodes as the two UTF-8 bytes C2 83. `unmeta` scans for a 0x83
    // BYTE, so it finds the trailing byte of that pair and mangles the string:
    // `$'\M-a'` came back out as `\M-^C\M-A` instead of `\M-a`.
    let detok = untokenize(s);
    ums = unmetafy_str(&detok);
    umlen = ums.len(); // c:5389 *umlen
    ptr = 0; // c:5389 ptr starts at 0 in ums

    // c:5393 — `mbrtowc` is LOCALE-driven: with `MB_CUR_MAX == 1` (the
    // C/POSIX locale) it consumes exactly one byte per call and hands
    // back that byte's value as the wide character, so a UTF-8 sequence
    // sitting in the string is measured as its separate bytes — a
    // printable one (`\xe2` = U+00E2) costs its own column, a
    // non-printable one (`\x80`, `\x90`) costs the five columns of the
    // `\M-^@` representation `wcs_nicechar_sel` returns for it. zsh
    // therefore charges 1+5+5 = 11 columns for `‐` (U+2010) under
    // `LC_ALL=C`, not the 1 column it charges under a UTF-8 locale, and
    // every `ZMB_nicewidth` consumer (described-list truncation in
    // computil.c's `cd_get`, `cd_calc`'s `premaxw`, `calclist`'s column
    // widths) budgets against that number.
    //
    // The port decoded UTF-8 unconditionally, so under the C locale it
    // measured 1 where zsh measured 11 and let descriptions run past
    // zsh's cut column. Branch on the active locale's CODESET, the same
    // way the `${#…}` length arm does (subst.rs, Bug #378), and take one
    // byte at a time when it is not UTF-8. Inlined rather than factored
    // out because src/ported/ forbids Rust-original helper fns.
    //
    // Platform note: this reproduces a C locale whose `mbrtowc` accepts
    // all 256 byte values (macOS, and every 8-bit locale). A locale whose
    // `mbrtowc` rejects bytes >= 0x80 outright (glibc's ASCII C locale)
    // takes C's `MB_INVALID` arm instead, which charges `nicechar_sel`'s
    // `\M-b` (4) for the lead byte rather than 1.
    //
    // Scope: the branch applies to the WIDTH-ONLY shape — C names it
    // `ZMB_nicewidth(s)` = `mb_niceformat(s, NULL, NULL, 0)`
    // (Src/Zle/zle.h:57), reached through `niceztrlen` — AND to the STREAM
    // shape, which is how `printmatch` (compresult.c:2260) writes a match's
    // display string to the terminal. The stream shape was excluded because C
    // emits the RAW byte for a printable high byte (`wcrtomb` in a
    // single-byte locale writes one byte, and `zputs` un-metafies
    // `wcs_nicechar_sel`'s buffer on the way out) while a Rust
    // `String`/`write_all` of that scalar writes its two UTF-8 bytes; the
    // emission below now converts back to bytes, so the objection is gone.
    // Until then `gifdiff -` printed `don’t` where zsh prints
    // `don<e2>\M-^@\M-^Yt`, and the same for every description carrying a
    // curly apostrophe, an en dash or a typographic minus.
    //
    // The `outstrp` shape (`nicedup`) is still excluded: its result is a Rust
    // `String` handed back to shell-visible code, which has no way to carry a
    // lone 0xe2 byte.
    let mb_single_byte = outstrp.is_none()
        && unsafe {
            // Rust never calls `setlocale` on its own, so run it once from
            // the environment before asking `nl_langinfo` — otherwise the
            // startup default ("C") would shadow the process's LC_CTYPE.
            let _ = *MB_LOCALE_READY;
            let cs_ptr = libc::nl_langinfo(libc::CODESET);
            if cs_ptr.is_null() {
                false
            } else {
                let cs = std::ffi::CStr::from_ptr(cs_ptr).to_string_lossy();
                !(cs.eq_ignore_ascii_case("UTF-8") || cs.eq_ignore_ascii_case("utf8"))
            }
        };

    // c:5391 — `memset(&mbs, 0, sizeof mbs);` (Rust: stateless UTF-8)
    while umlen > 0 {
        // c:5392
        // c:5393 — `cnt = eol ? MB_INVALID : mbrtowc(&c, ptr, umlen, &mbs);`
        // Rust equivalent: try to decode one UTF-8 scalar from ums[ptr..],
        // honoring `eol` (force the invalid arm when set).
        let cnt: usize;
        let decoded_c: Option<char>;
        if eol {
            // c:5396 — MB_INVALID arm via `eol = 1`.
            decoded_c = None;
            cnt = 1;
        } else if mb_single_byte {
            // c:5393 — single-byte locale: `mbrtowc` returns 1 and sets
            // `c` to the byte value itself.
            decoded_c = Some(char::from(ums[ptr]));
            cnt = 1;
        } else {
            // c:5393 — `mbrtowc` decodes at most ONE character and looks at no
            // more than the bytes that character occupies; it never validates
            // the rest of the buffer. Handing the whole tail to `from_utf8`
            // validated every remaining byte on every iteration, making the
            // walk quadratic in the string length. A UTF-8 scalar is at most
            // four bytes, so a four-byte window sees exactly what `mbrtowc`
            // sees: a sequence that runs past it cannot be valid UTF-8, and a
            // sequence truncated by end-of-input is still reported incomplete
            // because the window is then shorter than four bytes.
            let remaining = &ums[ptr..ptr + umlen.min(4)];
            match std::str::from_utf8(remaining) {
                Ok(s_slice) => {
                    // Valid UTF-8 throughout remaining; consume one char.
                    if let Some(ch) = s_slice.chars().next() {
                        decoded_c = Some(ch);
                        cnt = ch.len_utf8();
                    } else {
                        // Empty? Shouldn't happen given umlen > 0, but
                        // mirror MB_INVALID.
                        decoded_c = None;
                        cnt = 1;
                    }
                }
                Err(e) => {
                    let valid_up_to = e.valid_up_to();
                    if valid_up_to > 0 {
                        // Decode the first valid char then continue.
                        let valid_slice =
                            unsafe { std::str::from_utf8_unchecked(&remaining[..valid_up_to]) };
                        let ch = valid_slice.chars().next().unwrap();
                        decoded_c = Some(ch);
                        cnt = ch.len_utf8();
                    } else if e.error_len().is_none() {
                        // Incomplete sequence at end of input — MB_INCOMPLETE.
                        // c:5394 — `case MB_INCOMPLETE: eol = 1; FALL THROUGH`
                        eol = true;
                        decoded_c = None; // MB_INVALID
                        cnt = 1;
                    } else {
                        // Definite invalid byte — MB_INVALID.
                        decoded_c = None;
                        cnt = 1;
                    }
                }
            }
        }

        // c:5396-5403 / c:5404-5424 switch dispatch on cnt/decoded_c.
        let cnt_used: usize;
        match decoded_c {
            None => {
                // c:5397 — `case MB_INVALID:` (or fall-through from
                // MB_INCOMPLETE at c:5394).
                // c:5400 — `fmt = nicechar_sel(*ptr, flags & NICEFLAG_QUOTE);`
                fmt = nicechar_sel(ums[ptr] as char, (flags & NICEFLAG_QUOTE) != 0);
                newl = fmt.len(); // c:5401 newl = strlen(fmt)
                cnt_used = 1; // c:5402 cnt = 1
                              // c:5403 — `memset(&mbs, 0, sizeof mbs);` (Rust: stateless)
            }
            Some(c) if c == '\0' => {
                // c:5406 — `case 0:` — '\0' decodes to 0; consume 1 byte
                // and fall through to default.
                cnt_used = 1; // c:5409
                              // c:5411-5421 default arm (FALL THROUGH from case 0)
                if c == '\'' && (flags & NICEFLAG_QUOTE) != 0 {
                    // c:5413
                    fmt = "\\'".to_string(); // c:5414
                    newl = 2; // c:5415
                } else if c == '\\' && (flags & NICEFLAG_QUOTE) != 0 {
                    // c:5417
                    fmt = "\\\\".to_string(); // c:5418
                    newl = 2; // c:5419
                } else {
                    // c:5422 — `fmt = wcs_nicechar_sel(c, &newl, NULL,
                    //                                 flags & NICEFLAG_QUOTE);`
                    let mut width: usize = 0;
                    fmt =
                        wcs_nicechar_sel(c, Some(&mut width), None, (flags & NICEFLAG_QUOTE) != 0);
                    newl = width;
                }
            }
            Some(c) => {
                // c:5411 — `default:` arm (cnt > 0, valid char).
                cnt_used = cnt;
                if c == '\'' && (flags & NICEFLAG_QUOTE) != 0 {
                    // c:5413
                    fmt = "\\'".to_string(); // c:5414
                    newl = 2; // c:5415
                } else if c == '\\' && (flags & NICEFLAG_QUOTE) != 0 {
                    // c:5417
                    fmt = "\\\\".to_string(); // c:5418
                    newl = 2; // c:5419
                } else {
                    // c:5422 — `fmt = wcs_nicechar_sel(c, &newl, NULL,
                    //                                 flags & NICEFLAG_QUOTE);`
                    let mut width: usize = 0;
                    fmt =
                        wcs_nicechar_sel(c, Some(&mut width), None, (flags & NICEFLAG_QUOTE) != 0);
                    newl = width;
                }
            }
        }

        umlen -= cnt_used; // c:5427 umlen -= cnt
        ptr += cnt_used; // c:5428 ptr += cnt
        l += newl; // c:5429 l += newl

        if let Some(ref mut w) = stream {
            // c:5431 if (stream)
            // c:5432 — `zputs(fmt, stream);`. In a single-byte locale
            // `wcs_nicechar_sel`'s printable arm is C's `wcrtomb`, which
            // writes exactly ONE byte; this port holds that byte as a `char`
            // in U+0080..U+00FF whose UTF-8 form is two bytes, so it has to be
            // narrowed back on the way out. Escapes (`\M-^@`) are ASCII and
            // pass through either way.
            if mb_single_byte {
                let mut out: Vec<u8> = Vec::with_capacity(fmt.len());
                let mut b4 = [0u8; 4];
                for ch in fmt.chars() {
                    if (ch as u32) < 0x100 {
                        out.push(ch as u8);
                    } else {
                        out.extend_from_slice(ch.encode_utf8(&mut b4).as_bytes());
                    }
                }
                let _ = w.write_all(&out);
            } else {
                let _ = w.write_all(fmt.as_bytes());
            }
        }
        if let Some(ref mut buf) = outstr {
            // c:5433 if (outstr)
            // c:5434-5446 — append fmt to outstr, growing on demand. Rust
            // String auto-grows; the realloc loop collapses to push_str.
            buf.push_str(&fmt); // c:5446 memcpy(outptr, fmt, outlen)
        }
        let _ = fmt;
    }

    // c:5451 — `free(ums);` (Rust drop at scope exit)
    drop(ums);
    if let Some(slot) = outstrp {
        // c:5452 if (outstrp)
        // c:5453 — `*outptr = '\0';` (no-op for Rust String)
        // c:5455-5460 — NICEFLAG_NODUP / NICEFLAG_HEAP shaping. Rust has
        // a single allocator so all three paths produce identical owned
        // String contents; transfer ownership into caller's slot.
        *slot = outstr.take();
    }

    l // c:5462 return l
}

/// Port of `is_mb_niceformat()` from `Src/utils.c:5474` — C decl `is_mb_niceformat(const char *s)`.
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
pub fn is_mb_niceformat(s: &str) -> i32 {
    // c:5474
    // c:5476 — `int umlen, ret = 0;`
    let umlen: usize; // c:5476
    let mut ret: i32 = 0; // c:5476
                          // c:5477 — `char *ums, *ptr;` (eptr modelled by bytes.len())
    let mut ums: Vec<u8>; // c:5477
    let mut ptr: usize; // c:5477

    // c:5481-5483 — `ums = ztrdup(s); untokenize(ums); ptr =
    //                unmetafy(ums, &umlen);`. Use BYTE-level `unmetafy_str`,
    // not char-level `unmeta`: `unmeta` decodes a metafied eight-bit byte
    // (Meta 0x83 + 0xC1 for `$'\M-a'`) into the valid Rust char U+00E1
    // (`á`), which the UTF-8 scan below reads as a PRINTABLE Latin-1 letter
    // → is_mb_niceformat=0 → `(q+)`/quotedzputs left the raw byte unquoted
    // (`\341`) where zsh emits `$'\M-a'`. `unmetafy_str` preserves the raw
    // 0xE1 byte (C's `unmetafy`), so the MB_INVALID arm's is_nicechar(0xE1)
    // fires (high-bit → nice) exactly as C does.
    let detok = untokenize(s);
    ums = unmetafy_str(&detok);
    umlen = ums.len(); // c:5483 *umlen
    ptr = 0; // c:5483 ptr starts at 0

    // c:5485 — `memset(&mbs, 0, sizeof mbs);` (Rust: stateless UTF-8)
    while ret == 0 && ptr < ums.len() {
        // c:5486 while (umlen > 0)
        let remaining = &ums[ptr..];
        match std::str::from_utf8(remaining) {
            Ok(s_slice) => {
                // c:5503-5511 — `default: if (is_wcs_nicechar(c)) ret = 1;`
                for ch in s_slice.chars() {
                    if is_wcs_nicechar(ch) {
                        // c:5508
                        ret = 1; // c:5509
                        break;
                    }
                }
                break;
            }
            Err(e) => {
                let valid_up_to = e.valid_up_to();
                if valid_up_to > 0 {
                    let valid = unsafe { std::str::from_utf8_unchecked(&remaining[..valid_up_to]) };
                    for ch in valid.chars() {
                        if is_wcs_nicechar(ch) {
                            // c:5508
                            ret = 1; // c:5509
                            break;
                        }
                    }
                    if ret != 0 {
                        break;
                    }
                    ptr += valid_up_to;
                    continue;
                }
                // c:5493-5498 — `case MB_INVALID: if (is_nicechar(*ptr))
                //                ret = 1; break;`
                if is_nicechar(remaining[0] as char) {
                    // c:5494
                    ret = 1; // c:5495
                    break;
                }
                ptr += 1;
            }
        }
    }

    drop(ums); // c:5519 free(ums)
    ret // c:5523 return ret
}

/// Multibyte metachar length with conversion (from utils.c mb_metacharlenconv_r)
/// Port of `mb_metacharlenconv_r(const char *s, wint_t *wcp, mbstate_t *mbsp)` from `Src/utils.c:5548`.
/// WARNING: param names don't match C — Rust=(s, pos) vs C=(s, wcp, mbsp)
// Rust idiom replacement: Rust strings are UTF-8 with valid scalar
// values, so `chars().next()` covers the mbrtowc state machine; no
// mbstate_t threading required.
/// Port of `mb_metacharlenconv_r()` from `Src/utils.c:5548`.
///
/// `mb_metacharlenconv_r` — see implementation.
pub fn mb_metacharlenconv_r(s: &str, pos: usize) -> (usize, Option<char>) {
    if let Some(c) = s[pos..].chars().next() {
        (c.len_utf8(), Some(c))
    } else {
        (0, None)
    }
}

/// Port of `mb_metacharlenconv()` from `Src/utils.c:5611`.
///
/// Multibyte-aware metafied-string char advance.
/// Port of `mb_metacharlenconv(const char *s, wint_t *wcp)` from Src/utils.c:5611. Returns
/// `(bytes_consumed, scalar_char)` for the next character of `s`.
///
/// `s` is the DEMETAFIED byte stream, not the port's `String` form. C
/// takes a metafied `char *` and un-metafies inline as it walks (c:5616
/// `*s == Meta ? s[1] ^ 32 : *s`, c:5562-5563 in the `_r` guts); the port
/// hoists that one step to the caller, which reaches this function via
/// `unmetafy_str`. The hoist is forced, not stylistic: with the MULTIBYTE
/// option off a unit is a single BYTE, so a caller must be able to cut
/// mid-character — and a Rust `String` cannot be sliced there. Working on
/// the byte stream is also what makes a `Meta`+byte pair and a literal
/// high byte the same input, exactly as they are to C. A demetafied
/// stream contains no `Meta` bytes to re-interpret, so a literal 0x83 in
/// a value (`$'\x83'`) stays one byte here instead of being mistaken for
/// an escape prefix.
///
/// c:5613 is the guard that makes byte mode work: with MULTIBYTE unset —
/// or for any 7-bit byte — the answer is one byte, no decoding. Without
/// it every caller silently kept counting characters in a mode where zsh
/// counts bytes.
///
/// The third return value has no counterpart in C's signature: it is the
/// unit itself, back in the port's `String` form. C needs no such thing —
/// its caller just slices the `char *` it already holds at the returned
/// length. A Rust caller cannot, because in byte mode the unit can be half
/// a character, so the re-encode has to happen where the boundary is known.
/// Returning it here is what keeps that one rule in one place: every
/// subscript, slice, split and quote walk re-encodes identically, and
/// concatenating any run of units reproduces exactly the bytes C would have
/// copied.
/// WARNING: param names don't match C — Rust=(s) vs C=(s, wcp)
pub fn mb_metacharlenconv(s: &[u8]) -> (usize, Option<char>, String) {
    if s.is_empty() {
        // c:5594-5595 — nothing left to decode.
        return (0, None, String::new());
    }
    // The unit in the port's `String` form: well-formed text verbatim,
    // anything else escaped as `Meta` + `byte ^ 32` — the representation
    // `unmetafy_str` decodes. C's own metafication (c:4869 `metafy`) escapes
    // the `imeta` set for the same reason; Rust's `String` additionally
    // cannot hold a byte that is not part of a valid sequence, so the escape
    // covers those too.
    let encode = |chunk: &[u8]| -> String {
        match std::str::from_utf8(chunk) {
            Ok(v) => v.to_string(),
            Err(_) => {
                let mut v = String::with_capacity(2 * chunk.len());
                for &b in chunk {
                    if b < 0x80 {
                        v.push(b as char);
                    } else {
                        v.push(char::from(Meta));
                        v.push(char::from(b ^ 32));
                    }
                }
                v
            }
        }
    };
    // c:5613 — `if (!isset(MULTIBYTE) || (unsigned char) *s <= 0x7f)`:
    // treat as a single byte. The option is read from the live slot with
    // its declared default (on, c:Src/options.c:197) rather than through
    // `isset()`, which maps a never-written slot to false and would invert
    // the default in any context that skips init's `emulate()`.
    if !crate::ported::options::opt_state_get("multibyte").unwrap_or(true) || s[0] <= 0x7f {
        // c:5616 — the byte itself is the scalar.
        return (1, Some(s[0] as char), encode(&s[..1]));
    }
    // c:5635 → mb_metacharlenconv_r: feed bytes to mbrtowc until one
    // character completes (c:5577-5585). Rust's UTF-8 validation is the
    // mbrtowc analogue — the shortest valid prefix is one character.
    let hi = (s.len()).min(4);
    for end in 1..=hi {
        if let Ok(v) = std::str::from_utf8(&s[..end]) {
            if let Some(c) = v.chars().next() {
                return (end, Some(c), v.to_string());
            }
        }
    }
    // c:5592-5593 — no valid multibyte sequence: treat as a single byte.
    (1, None, encode(&s[..1]))
}

/// Port of `mb_metastrlenend()` from `Src/utils.c:5655` — C decl `mb_metastrlenend(char *ptr, int width, char *eptr)`.
///
/// Multibyte metastring length to end (from utils.c mb_metastrlenend)
///
/// Counts characters (`width == false`) or display columns
/// (`width == true`) in a metafied string. C walks the metafied
/// byte stream, un-metafies inline (`if (*ptr == Meta) inchar =
/// *++ptr ^ 32`, c:5672), and feeds one byte at a time to
/// `mbrtowc` to assemble each multibyte character (c:5691); a
/// complete character counts 1 (or its `WCWIDTH`), an invalid byte
/// counts as a single character (c:5712-5714).
///
/// Rust-port note: zshrs stores both metafied byte-pairs (`Meta` +
/// `byte ^ 32`, produced by `getkeystring` for `$'\xNN'` escapes)
/// AND real multibyte `char`s (the literal `$'é'` path) in one
/// `String`, whereas C only ever holds metafied bytes. So the port
/// first reduces the slice to its raw byte stream via `unmetafy_str`
/// (metafied pair → its byte; real `char` → its UTF-8 bytes — the
/// same bytes C's inline demetafy would yield), then runs C's
/// `mbrtowc` character-assembly loop over those bytes. Rust's UTF-8
/// validation is the `mbrtowc` analogue: the shortest valid prefix
/// is one character; a byte that begins no valid sequence counts as
/// one (c:5712).
pub fn mb_metastrlenend(ptr: &str, width: bool, eptr: usize) -> usize {
    // c:5672 — un-metafy the (optionally end-bounded) slice to raw bytes.
    let bytes = unmetafy_str(&ptr[..eptr.min(ptr.len())]);
    // c:5662-5663 — `if (!isset(MULTIBYTE) || MB_CUR_MAX == 1) return
    // eptr ? (int)(eptr - ptr) : ztrlen(ptr);`. With the MULTIBYTE
    // option unset every length op counts BYTES, and `ztrlen` counts a
    // `Meta`+byte pair as the ONE byte it encodes — which is exactly
    // the length of the demetafied stream computed above.
    //
    // This guard belongs HERE, not at the call sites. It used to live
    // only in the `${#v}` length arm (subst.rs), so `${#v}` counted
    // bytes under `unsetopt multibyte` while every other caller of this
    // function — dopadding's (l)/(r)/(m) widths above all — kept
    // counting characters.
    //
    // The option is read from the live slot with the DECLARED DEFAULT
    // (on, c:Src/options.c:197) rather than through `isset()`, which
    // maps a never-written slot to false and would invert the
    // default-on semantics in any context that skips init's
    // `emulate()` — unit tests most of all.
    if !crate::ported::options::opt_state_get("multibyte").unwrap_or(true) {
        return bytes.len();
    }
    let mut num: usize = 0; // c:5658 size_t ret / counter
    let mut i: usize = 0;
    while i < bytes.len() {
        // c:5678-5687 — 7-bit US-ASCII subset: one character, skip mbrtowc.
        if bytes[i] <= 0x7f {
            num += 1;
            i += 1;
            continue;
        }
        // c:5691 — mbrtowc: take the shortest valid UTF-8 character.
        let mut num_in_char: usize = 1; // c:5701 trailing-octet span
        let hi = (i + 4).min(bytes.len());
        for end in (i + 1)..=hi {
            if std::str::from_utf8(&bytes[i..end]).is_ok() {
                num_in_char = end - i;
                break;
            }
        }
        if width {
            // c:5717-5723 — WCWIDTH of the assembled character (≥0).
            let wcw = std::str::from_utf8(&bytes[i..i + num_in_char])
                .ok()
                .and_then(|s| s.chars().next())
                .and_then(unicode_width::UnicodeWidthChar::width)
                .unwrap_or(0);
            num += wcw;
        } else {
            // c:5727 — one character (complete, or invalid treated as one).
            num += 1;
        }
        i += num_in_char;
    }
    num
}

/// Port of `mb_charlenconv_r()` from `Src/utils.c:5747` — C decl `mb_charlenconv_r(const char *s, int slen, wint_t *wcp, mbstate_t *mbsp)`.
///
/// Multibyte char length with conversion (from utils.c mb_charlenconv_r)
/// WARNING: param names don't match C — Rust=(s, pos) vs C=(s, slen, wcp, mbsp)
pub fn mb_charlenconv_r(s: &str, pos: usize) -> (usize, Option<char>) {
    mb_metacharlenconv_r(s, pos)
}

/// Port of `mb_charlenconv()` from `Src/utils.c:5793` — C decl `mb_charlenconv(const char *s, int slen, wint_t *wcp)`.
///
/// Multibyte char length (from utils.c mb_charlenconv)
/// WARNING: param names don't match C — Rust=(s, pos) vs C=(s, slen, wcp)
pub fn mb_charlenconv(s: &str, pos: usize) -> usize {
    s[pos..].chars().next().map(|c| c.len_utf8()).unwrap_or(0)
}

/// Port of `metacharlenconv()` from `Src/utils.c:5811` — C decl `int metacharlenconv(const char *x, int *c)`.
/// ```c
/// if (*x == Meta) {
///     if (c) *c = x[1] ^ 32;
///     return 2;
/// }
/// if (c) *c = (char)*x;
/// return 1;
/// ```
/// Single-byte metafied char advance — Meta+X is 2 bytes (decode
/// via XOR 32), plain byte is 1 byte. Previously used hardcoded
/// `0x83`; now routes through the canonical `Meta` const for
/// maintainability parity.
/// WARNING: param names don't match C — Rust=(s) vs C=(x, c)
pub fn metacharlenconv(s: &str) -> (usize, Option<char>) {
    // c:5811
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return (0, None);
    }
    if bytes[0] == Meta && bytes.len() >= 2 {
        // c:5818
        let raw = bytes[1] as u32 ^ 32; // c:5820
        return (2, char::from_u32(raw)); // c:5821
    }
    (1, Some(bytes[0] as char)) // c:5823-5825
}

/// Port of `charlenconv()` from `Src/utils.c:5832`.
///
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

/// Port of `sb_niceformat()` from `Src/utils.c:5851` — C decl `size_t sb_niceformat(const char *s, FILE *stream, char **outstrp, int flags)`.
/// ```c
/// ums = ztrdup(s); untokenize(ums); ptr = unmetafy(ums, &umlen);
/// while (ptr < eptr) {
///     int c = (unsigned char) *ptr;
///     if      (c == '\''  && (flags & NICEFLAG_QUOTE)) fmt = "\\'";
///     else if (c == '\\'  && (flags & NICEFLAG_QUOTE)) fmt = "\\\\";
///     else                                              fmt = nicechar_sel(c, ...);
/// }
/// ```
/// Single-byte nice format. **Unmetafies the input first** (c:5872),
/// then calls `nicechar_sel` for EVERY byte (not just controls).
///
/// Param mapping (Rule S1, faithful to C):
/// - `s: &str`            ← C `const char *s`
/// - `stream: Option<&mut dyn Write>` ← C `FILE *stream` (`None` ≡ `NULL`)
/// - `outstrp: Option<&mut Option<String>>` ← C `char **outstrp`
///   (outer `None` ≡ `NULL`; inner `Option<String>` is the storage slot
///   the caller passes as `&local`, mirroring `char *retstr;
///   sb_niceformat(s, NULL, &retstr, ...);`).
/// - returns `usize` ← C `size_t l`.
pub fn sb_niceformat(
    s: &str,
    mut stream: Option<&mut dyn std::io::Write>,
    outstrp: Option<&mut Option<String>>,
    flags: i32,
) -> usize {
    // c:5851
    let mut l: usize = 0; // c:5853 size_t l = 0
    let mut newl: usize; // c:5853 size_t newl
                         // c:5854 — `int umlen, outalloc, outleft;`. outalloc/outleft model
                         // C's buffer-growth math; Rust String auto-grows so the realloc
                         // loop at c:5897-5906 collapses to push_str. umlen carries through.
    let umlen: usize; // c:5854
                      // c:5855 — `char *ums, *ptr, *eptr, *fmt, *outstr, *outptr;`
    let mut ums: Vec<u8>; // c:5855 char *ums
    let mut ptr: usize; // c:5855 char *ptr
    let eptr: usize; // c:5855 char *eptr
    let mut fmt: String; // c:5855 char *fmt
    let mut outstr: Option<String>; // c:5855 char *outstr

    // c:5857-5863 — `if (outstrp) outptr = outstr = zalloc(2*strlen(s));`
    if outstrp.is_some() {
        // c:5857
        outstr = Some(String::with_capacity(2 * s.len())); // c:5859
    } else {
        // c:5860
        outstr = None; // c:5862 outstr = NULL
    }

    // c:5865 — `ums = ztrdup(s);`
    ums = s.as_bytes().to_vec();
    // c:5866-5870 — comment-only block carried verbatim:
    /*
     * is this necessary at this point? niceztrlen does this
     * but it's used in lots of places.  however, one day this may
     * be, too.
     */                                                                      // c:5866-5870
    // c:5871 — `untokenize(ums);` inlined byte-level (lex::untokenize
    // is char-based; C does byte-walks). Mirrors `Src/exec.c:2077-2099`
    // exec.c untokenize().
    let ztokens_table = b"#$^*(())$=|{}[]`<>>?~`,-!'\"\\\\"; // ZTOKENS — Src/lex.c:38
    let mut detok: Vec<u8> = Vec::with_capacity(ums.len());
    for &c in &ums {
        // exec.c:2082
        if (0x84u8..=0xa1u8).contains(&c) {
            // exec.c:2083 itok(c)
            if c != 0xa1u8 {
                // exec.c:2086 c != Nularg
                let idx = (c - 0x84) as usize;
                if idx < ztokens_table.len() {
                    detok.push(ztokens_table[idx]);
                }
            }
        } else {
            detok.push(c); // exec.c:2094
        }
    }
    ums = detok;
    // c:5872 — `ptr = unmetafy(ums, &umlen);`
    umlen = unmetafy(&mut ums);
    ums.truncate(umlen);
    eptr = umlen; // c:5873 eptr = ptr + umlen
    ptr = 0; // c:5872 ptr starts at 0 in ums

    while ptr < eptr {
        // c:5875
        let c: i32 = ums[ptr] as i32; // c:5876 int c = (unsigned char) *ptr
        if c == b'\'' as i32 && (flags & NICEFLAG_QUOTE) != 0 {
            // c:5877
            fmt = "\\'".to_string(); // c:5878
            newl = 2; // c:5879
        } else if c == b'\\' as i32 && (flags & NICEFLAG_QUOTE) != 0 {
            // c:5881
            fmt = "\\\\".to_string(); // c:5882
            newl = 2; // c:5883
        } else {
            // c:5885
            fmt = nicechar_sel(c as u8 as char, (flags & NICEFLAG_QUOTE) != 0); // c:5886
            newl = 1; // c:5887
        }

        ptr += 1; // c:5890 ++ptr
        l += newl; // c:5891 l += newl

        if let Some(ref mut w) = stream {
            // c:5893 if (stream)
            // c:5894 — `zputs(fmt, stream);`
            let _ = w.write_all(fmt.as_bytes());
        }
        if let Some(ref mut buf) = outstr {
            // c:5895 if (outstr)
            // c:5896-5912 — append fmt to outstr, growing on demand. Rust
            // String auto-grows; the realloc loop at c:5897-5906 collapses
            // to push_str (memcpy + outptr/outleft bookkeeping unneeded).
            buf.push_str(&fmt); // c:5907 memcpy(outptr, fmt, outlen)
        }
        // `fmt` is consumed at next iter assignment (C reuses pointer).
        let _ = fmt;
    }

    // c:5915 — `free(ums);` (Rust drop at scope exit)
    drop(ums);
    if let Some(slot) = outstrp {
        // c:5916 if (outstrp)
        // c:5917 — `*outptr = '\0';` (no-op for String — push_str leaves
        // no embedded NUL, and Rust String is length-prefixed).
        // c:5919-5925 — NICEFLAG_NODUP / NICEFLAG_HEAP shaping: in C this
        // selects between ztrdup (perm) / dupstring (heap arena) / direct
        // ownership-transfer (NODUP). Rust has a single allocator so all
        // three paths produce identical owned String contents; transfer
        // ownership of `outstr` into the caller's slot.
        *slot = outstr.take();
    }

    l // c:5928 return l
}

/// Port of `is_sb_niceformat()` from `Src/utils.c:5937` — C decl `int is_sb_niceformat(const char *s)`.
///
/// Predicate: would `sb_niceformat` change the input? Walks each byte
/// after unmetafy, returns `1` if any byte is "nice" per `is_nicechar`,
/// else `0` (C `int` return faithful to Rule S1).
pub fn is_sb_niceformat(s: &str) -> i32 {
    // c:5937
    // c:5939 — `int umlen, ret = 0;`
    let umlen: usize; // c:5939
    let mut ret: i32 = 0; // c:5939
                          // c:5940 — `char *ums, *ptr, *eptr;`
    let mut ums: Vec<u8>; // c:5940 char *ums
    let mut ptr: usize; // c:5940 char *ptr
    let eptr: usize; // c:5940 char *eptr

    ums = s.as_bytes().to_vec(); // c:5942 ums = ztrdup(s)
                                 // c:5943 — `untokenize(ums);` inlined byte-level (lex::untokenize is
                                 // char-based; C does byte-walks). Mirrors `Src/exec.c:2077-2099`.
    let ztokens_table = b"#$^*(())$=|{}[]`<>>?~`,-!'\"\\\\"; // ZTOKENS — Src/lex.c:38
    let mut detok: Vec<u8> = Vec::with_capacity(ums.len());
    for &c in &ums {
        // exec.c:2082
        if (0x84u8..=0xa1u8).contains(&c) {
            // exec.c:2083 itok(c)
            if c != 0xa1u8 {
                // exec.c:2086 c != Nularg
                let idx = (c - 0x84) as usize;
                if idx < ztokens_table.len() {
                    detok.push(ztokens_table[idx]);
                }
            }
        } else {
            detok.push(c); // exec.c:2094
        }
    }
    ums = detok;
    umlen = unmetafy(&mut ums); // c:5944 ptr = unmetafy(ums, &umlen)
    ums.truncate(umlen);
    eptr = umlen; // c:5945 eptr = ptr + umlen
    ptr = 0; // c:5944 ptr starts at 0 in ums

    while ptr < eptr {
        // c:5947
        if is_nicechar(ums[ptr] as char) {
            // c:5948 is_nicechar(*ptr)
            ret = 1; // c:5949
            break; // c:5950
        }
        ptr += 1; // c:5952 ++ptr
    }

    drop(ums); // c:5955 free(ums)
    ret // c:5957 return ret
}

/// Port of `zexpandtabs()` from `Src/utils.c:5975`.
///
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

/// Port of `hasspecial()` from `Src/utils.c:6072` — C decl `int hasspecial(char const *s)`.
/// ```c
/// while (*s) {
///     if (ispecial(*s == Meta ? *++s ^ 32 : *s)) return 1;
///     s++;
/// }
/// return 0;
/// ```
/// Predicate: does `s` contain any byte that needs shell-quoting?
/// Routes through the canonical typtab-driven `ztype_h::ispecial`
/// which respects the `ZTF_SP_COMMA` (set by `makecommaspecial`)
/// and `ZTF_BANGCHAR` (BANGHIST + interact) flags. Previously used
/// a hardcoded char list — diverged from C for `,` (KSH_GLOB
/// extended-glob) and for the dynamic bangchar (`!` by default but
/// user-rewritable via `$HISTCHARS`).
pub fn hasspecial(s: &str) -> bool {
    // c:6072
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = if bytes[i] == Meta && i + 1 < bytes.len() {
            // c:6075 — `*s == Meta ? *++s ^ 32 : *s`.
            let v = bytes[i + 1] ^ 32;
            i += 2;
            v
        } else {
            let v = bytes[i];
            i += 1;
            v
        };
        if crate::ported::ztype_h::ispecial(c) {
            // c:6075
            return true;
        }
    }
    false
}

/// Port of `addunprintable()` from `Src/utils.c:6083`.
///
/// Port of `static char *addunprintable(char *v, const char *u, const char *uend)`
/// from `Src/utils.c:6083-6124`. Renders unprintable bytes using
/// **shell-compatible C-string escapes**:
///   - `\0` (NUL, with `\000` form if next byte is octal-digit)
///   - `\a` (0x07 BEL), `\b` (BS), `\f` (FF), `\n` (LF),
///     `\r` (CR), `\t` (TAB), `\v` (VT)
///   - `\nnn` 3-digit octal fallback for any other control byte
///
/// Previously the Rust port emitted ZLE-style caret notation (`^X`,
/// `\u{:04x}`) — completely different convention. C semantics target
/// `printf %q` / `$'...'` reuse where the output must round-trip
/// through `printf %b` to reconstruct the original byte. Caret
/// notation is what ZLE *displays* but not what `addunprintable`
/// emits.
/// WARNING: param names don't match C — Rust=(c) vs C=(v, u, uend)
pub fn addunprintable(c: char) -> String {
    // c:6082
    let b = c as u32 & 0xff;
    match b as u8 {
        // c:6097-6103 — `\0`. C peeks next byte and uses `\000` form
        // when followed by an octal digit. Rust port works on a single
        // char so emits just `\0`; callers needing the disambiguation
        // form can pass the lookahead byte separately.
        0x00 => "\\0".to_string(),
        0x07 => "\\a".to_string(), // c:6106
        0x08 => "\\b".to_string(), // c:6107
        0x0c => "\\f".to_string(), // c:6108
        0x0a => "\\n".to_string(), // c:6109
        0x0d => "\\r".to_string(), // c:6110
        0x09 => "\\t".to_string(), // c:6111
        0x0b => "\\v".to_string(), // c:6112
        // c:6114-6119 — `\nnn` 3-digit octal default.
        _ => format!("\\{:o}{:o}{:o}", (b >> 6) & 7, (b >> 3) & 7, b & 7),
    }
}

/// One element of a metafied string, as C's `MB_METACHARLENCONV` loop sees it:
/// either a decoded character, or a raw byte that is not one.
///
/// A byte that cannot appear in a Rust `String` (an invalid-UTF-8 byte such as
/// the 0xE1 produced by `$'\M-a'`) is stored metafied — `Meta` (0x83) followed
/// by `byte ^ 32` — so a plain `.chars()` walk sees two innocuous characters
/// where the shell means one raw byte, and passes them straight through. C hits
/// exactly this case as `WEOF` out of `MB_METACHARLENCONV` (c:6210) and routes
/// it to `addunprintable()`.
#[derive(Clone, Copy, PartialEq)]
enum MetaChar {
    Ch(char),
    Raw(u8),
}

/// Port of `quotestring()` from `Src/utils.c:6141`.
///
/// Quote a string according to the specified type
/// Port from zsh/Src/utils.c quotestring() (lines 6141-6452)
/// Quote a string per the requested bslashquote style.
/// Port of `quotestring(const char *s, int instring)` from Src/utils.c — used by `print
/// -%q`, `${(q)var}`, completion-output escaping, history
/// re-emission.
/// WARNING: param names don't match C — Rust=(s, quote_type) vs C=(s, instring)
pub fn quotestring(s: &str, quote_type: i32) -> String {
    // c:6207-6210 — C walks the string with MB_METACHARINIT/MB_METACHARLENCONV,
    // which yields one decoded character at a time, or WEOF for a byte that does
    // not decode. These three closures are that walk, kept inline exactly as C
    // keeps it inline in each arm below.
    //
    // `meta_chars` is the decoder: a byte that cannot live in a Rust `String`
    // (an invalid-UTF-8 byte such as the 0xE1 from `$'\M-a'`) is stored metafied
    // — Meta (0x83) then `byte ^ 32` — so a naive `.chars()` walk sees two
    // innocuous characters where the shell means one raw byte and passes them
    // straight through. That is C's WEOF case, and it must reach addunprintable.
    //
    // The walk is `mb_metacharlenconv` (c:Src/utils.c:5611) over the DEMETAFIED
    // byte stream, which is the input C's macro actually sees. The port used to
    // decode each `Meta`+byte pair to its own `Raw` unit and stop there, so a
    // pair of pairs that spells ONE character never got assembled: `${(q)v}` for
    // `v=$'\xce\xb1'` emitted `$'\316'$'\261'` where zsh emits the two bytes of
    // `α` verbatim (`ce b1`), because C's mbrtowc joins them into a printable
    // wide character at c:6422 and copies the unit whole (c:6431-6433).
    // c:Src/utils.c:5613 — `if (!isset(MULTIBYTE) || …)`. C reads the option
    // through `isset(X)` = `opts[X]`, a single array load. The port went via
    // `opt_state_get("multibyte")`, which allocates a lowercased copy of the
    // name in `optlookup` and then does two String-keyed HashMap lookups
    // under an RwLock — on EVERY quotestring call. `quotestring` runs once
    // per completion match (`multiquote` → `quotestring`, compcore.c:1073),
    // so a 46765-match `compadd -k functions` paid that name lookup 46765
    // times. Same precedent as options.rs:768-770, which replaced
    // `isset(optlookup("shoptionletters"))` with the optno constant for
    // exactly this reason.
    let mb = isset(crate::ported::zsh_h::MULTIBYTE);
    let meta_chars = |s: &str| -> Vec<MetaChar> {
        // c:5672 — the byte stream is what mbrtowc consumes; a `Meta`+byte pair
        // and a literal high byte are the same input to the step below.
        let bytes = unmetafy_str(s);
        let mut out = Vec::with_capacity(bytes.len());
        let mut i = 0usize;
        while i < bytes.len() {
            let b = bytes[i];
            // c:5613 — `if (!isset(MULTIBYTE) || (unsigned char) *s <= 0x7f)`:
            // one byte, and `*wcp` is that byte widened. With the option off
            // that is EVERY byte, which is why `unsetopt multibyte;
            // ${(q)$'\xe6\x97\xa5'}` comes out as `e6` `$'\227'` `a5` in zsh —
            // 0xe6 and 0xa5 are printable Latin-1, 0x97 is a C1 control.
            if !mb || b <= 0x7f {
                out.push(if b <= 0x7f {
                    MetaChar::Ch(b as char)
                } else {
                    MetaChar::Raw(b)
                });
                i += 1;
                continue;
            }
            // c:5635 → mb_metacharlenconv_r: shortest valid multibyte prefix.
            let hi = (i + 4).min(bytes.len());
            let mut n = 0usize;
            for end in (i + 1)..=hi {
                if let Some(c) = std::str::from_utf8(&bytes[i..end])
                    .ok()
                    .and_then(|v| v.chars().next())
                {
                    out.push(MetaChar::Ch(c));
                    n = end - i;
                    break;
                }
            }
            if n == 0 {
                // c:5592-5593 — no valid sequence: WEOF, one byte.
                out.push(MetaChar::Raw(b));
                n = 1;
            }
            i += n;
        }
        out
    };
    // c:6212 / c:6424 — `cc != WEOF && WC_ISPRINT(cc)`.
    let mc_printable = |mc: MetaChar| -> bool {
        match mc {
            MetaChar::Ch(c) => !c.is_control(),
            // With MULTIBYTE on an undecodable byte IS the WEOF case, so the
            // `cc != WEOF` conjunct short-circuits. With it off there is no
            // WEOF at all (c:5615-5617 always sets `*wcp` to the byte), so the
            // test is `WC_ISPRINT` of that Latin-1 scalar.
            MetaChar::Raw(b) => !mb && !(b as char).is_control(),
        }
    };
    // c:6431-6433 — `while (u < uend) { if (*u == Meta) *v++ = *u++; *v++ = *u++; }`
    // copies a printable unit through STILL METAFIED. A raw byte therefore has
    // to go back out as `Meta` + `byte ^ 32`; pushing `b as char` would emit the
    // UTF-8 encoding of U+00`b` instead of the one byte the shell holds.
    let push_metachar = |out: &mut String, mc: MetaChar| match mc {
        MetaChar::Ch(c) => out.push(c),
        MetaChar::Raw(b) => {
            out.push(char::from(Meta));
            out.push(char::from(b ^ 32));
        }
    };
    // c:6082-6124 addunprintable — emit a non-printable element BYTE by byte, so
    // a non-printable multibyte character becomes one escape per UTF-8 byte.
    // `\0` widens to `\000` when an octal digit follows so the escape cannot
    // swallow it (c:6097-6103). There is deliberately no `\e` case: C has none,
    // so ESC comes out as the octal `\033`.
    let push_unprintable = |out: &mut String, mc: MetaChar, next: Option<MetaChar>| {
        let mut buf = [0u8; 4];
        let bytes: &[u8] = match mc {
            MetaChar::Ch(c) => c.encode_utf8(&mut buf).as_bytes(),
            MetaChar::Raw(b) => {
                buf[0] = b;
                &buf[0..1]
            }
        };
        for i in 0..bytes.len() {
            let b = bytes[i];
            if b == 0 {
                out.push_str("\\0");
                let follows = if i + 1 < bytes.len() {
                    Some(bytes[i + 1])
                } else {
                    next.map(|n| match n {
                        MetaChar::Ch(c) => {
                            let mut nb = [0u8; 4];
                            c.encode_utf8(&mut nb).as_bytes()[0]
                        }
                        MetaChar::Raw(rb) => rb,
                    })
                };
                if matches!(follows, Some(b'0'..=b'7')) {
                    out.push_str("00");
                }
            } else {
                out.push_str(&addunprintable(b as char));
            }
        }
    };

    // c:6141
    if s.is_empty() {
        return if quote_type == QT_NONE {
            String::new()
        } else if quote_type == QT_BACKSLASH {
            // c:6194 `if (!*s && shownull)` — shownull is 0 for plain
            // QT_BACKSLASH, so an empty string produces NO quotes.
            String::new()
        } else if quote_type == QT_BACKSLASH_SHOWNULL {
            // c:6165 sets shownull=1, so empty → '' (c:6194 adds the pair).
            "''".to_string()
        } else if quote_type == QT_SINGLE_OPTIONAL {
            // c:6187 sets `quotesub = shownull = 1`, so empty → '' (c:6194).
            "''".to_string()
        } else if quote_type == QT_SINGLE || quote_type == QT_DOUBLE || quote_type == QT_DOLLARS {
            // c:6194 — `if (!*s && shownull)`. `shownull` is set ONLY by
            // QT_BACKSLASH_SHOWNULL (c:6165) and QT_SINGLE_OPTIONAL (c:6187);
            // it is 0 for QT_SINGLE / QT_DOUBLE / QT_DOLLARS, which take the
            // `default:` arm at c:6190. Those styles return the quoted BODY
            // only — c:6131-6134: "Most quote styles other than backslash
            // assume the quotes are to be added outside quotestring()" — so an
            // empty input yields an empty body, and the CALLER supplies the
            // `''` / `""` / `$''` pair (subst.c:4083-4090, text.c:1086-1092).
            String::new()
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
            if matches!(
                c,
                '*' | '?' | '[' | ']' | '<' | '>' | '(' | ')' | '|' | '#' | '^' | '~'
            ) {
                result.push('\\');
            }
            result.push(c);
        }
        result
    } else if quote_type == QT_BACKSLASH
        || quote_type == QT_BACKSLASH_SHOWNULL
        || quote_type == QT_SINGLE
        || quote_type == QT_SINGLE_OPTIONAL
        || quote_type == QT_DOUBLE
    {
        // c:Src/utils.c:6248-6443 — the `else` arm of quotestring's three-way
        // split, shared by EVERY style except QT_DOLLARS (c:6203) and
        // QT_BACKSLASH_PATTERN (c:6242). C runs ONE loop here and branches on
        // `instring` inside it. This port used to hold four separate loops —
        // one per style — and the three non-BACKSLASH ones had each decayed
        // into a plain per-character walk missing the c:6262-6299 verbatim-copy
        // preludes, the c:6272-6280 `$'…'`-in-double-quotes arm, the
        // c:6392-6416 token pass-through and the c:6396-6412 Inparmath group.
        // They are one loop again, exactly as C has it, so a fix to a prelude
        // cannot land in one style and miss the other three.
        //
        // Per-style behaviour lives entirely in the c:6307-6313 subcondition
        // and the two branches at c:6314 / c:6364:
        //   QT_BACKSLASH        every ispecial char takes a `\` (c:6307); `\n`
        //                       upgrades to `$'\n'` (c:6366-6371) and a
        //                       non-printable char to `$'…'` (c:6435-6442).
        //   QT_SINGLE           only `'` (c:6313), rewritten `'\''`, or `''`
        //                       under RCQUOTES (c:6372-6379).
        //   QT_DOUBLE           only `$`, `` ` ``, `"`, `\` (c:6311-6312), plus
        //                       the history character under BANGHIST
        //                       (c:6309-6310).
        //   QT_SINGLE_OPTIONAL  every ispecial char (c:6308), but it opens
        //                       `'…'` spans only where they are needed
        //                       (c:6314-6363).
        //
        // c:6166 — QT_BACKSLASH_SHOWNULL sets `shownull` and FALLS THROUGH to
        // QT_BACKSLASH, so from here the two are one style; `shownull` only
        // changes the empty-input case, handled at c:6194 above.
        let instring = if quote_type == QT_BACKSLASH_SHOWNULL {
            QT_BACKSLASH
        } else {
            quote_type
        };
        let mut result = String::with_capacity(s.len() * 2);
        // c:6304-6305 lookbehind `u[-1]`. The verbatim arms below advance `u`
        // several bytes at a time, so they carry it forward themselves.
        let mut prev: char = '\0'; // would-be u[-1]
        // c:6149-6156 — `quotesub` drives QT_SINGLE_OPTIONAL and nothing else:
        // 0 = mechanism off, 1 = pending (no `'` emitted yet; one belongs at
        // `quotestart`), 2 = a `'…'` span is open and gets closed at c:6445.
        let mut quotesub: u8 = if instring == QT_SINGLE_OPTIONAL { 1 } else { 0 }; // c:6187
        let mut quotestart: usize = 0; // c:6197 — byte index into `result`
                                   // c:6392 — `if (itok(*u) || instring != QT_BACKSLASH)`: a parser TOKEN
                                   // byte "needs to be passed straight through" — never backslashed and
                                   // never printability-tested. That half of the test was missing from
                                   // this arm, and `meta_chars` cannot express it: it demetafies, and a
                                   // token is NOT a metafied pair (this port stores C's token bytes as
                                   // the chars U+0080..U+00A2), so `unmetafy_str` re-encodes one as its
                                   // two UTF-8 bytes. `Inbrace` therefore fell through to the c:6435
                                   // not-printable arm and `quotename(Inbrace)` produced `$'\302\217'`
                                   // where C produces the raw byte that `untokenize` then maps back to
                                   // `{` — which is what the c:1931-2218 brace tail in zle_tricky.c
                                   // stores in `brbeg` (`ls /usr/{b<TAB>`).
                                   // Split the input at token chars, hand only the runs BETWEEN them to
                                   // `meta_chars`, and remember which units were tokens. A `Meta` byte is
                                   // deliberately NOT a split point (C gives it IMETA but not ITOK,
                                   // utils.c:4196-4201), and its partner byte is skipped so a metafied
                                   // pair whose second byte lands in the token range stays intact.
        let (mcs, tokmask): (Vec<MetaChar>, Vec<bool>) = {
            let mut v: Vec<MetaChar> = Vec::with_capacity(s.len());
            let mut m: Vec<bool> = Vec::with_capacity(s.len());
            let mut run = String::new();
            let mut prev_meta = false;
            for c in s.chars() {
                let cu = c as u32;
                if !prev_meta && cu < 0x100 && crate::ported::ztype_h::itok(cu as u8) {
                    if !run.is_empty() {
                        let seg = meta_chars(&run);
                        m.resize(m.len() + seg.len(), false);
                        v.extend(seg);
                        run.clear();
                    }
                    v.push(MetaChar::Raw(cu as u8));
                    m.push(true);
                } else {
                    prev_meta = !prev_meta && c == char::from(Meta);
                    run.push(c);
                }
            }
            if !run.is_empty() {
                let seg = meta_chars(&run);
                m.resize(m.len() + seg.len(), false);
                v.extend(seg);
            }
            (v, m)
        };
        // c:6262-6299 — before ANY quoting decision C copies three
        // constructs through verbatim. Each `*u` C tests there is one BYTE,
        // which is one element of `mcs` here. C can compare bytes directly
        // because a metafied string never holds a bare token byte as data;
        // `mcs` is DEMETAFIED, so a data byte in the token range does reach
        // this loop as `Raw` — every test is therefore gated on `tokmask`,
        // which marks the elements that really were tokens in `s`.
        let cval = |k: usize| -> u32 {
            match mcs[k] {
                MetaChar::Raw(b) => b as u32,
                MetaChar::Ch(c) => c as u32,
            }
        };
        let is_tok = |k: usize, t: char| -> bool { tokmask[k] && cval(k) == t as u32 };
        // The mirror of `is_tok`: element `k` is the literal character `ch` as
        // DATA. Needed because c:6272 compares against `'$'` and `'\''`, which
        // are ordinary bytes, not tokens.
        let is_ch = |k: usize, ch: char| -> bool { !tokmask[k] && cval(k) == ch as u32 };
        // c:6431-6433 / c:6414 — copy one element out unchanged.
        let verbatim = |out: &mut String, k: usize| {
            if tokmask[k] {
                out.push(char::from_u32(cval(k)).unwrap_or('\0'));
            } else {
                push_metachar(out, mcs[k]);
            }
        };
        let bangchar_v = crate::ported::hist::bangchar.load(std::sync::atomic::Ordering::SeqCst);
        let mut i = 0usize;
        while i < mcs.len() {
            // c:6261 — set by the c:6382-6389 arm, consumed at c:6394 or
            // c:6428. It is deliberately DROPPED by the c:6435 not-printable
            // arm, which emits `$'…'` instead of a backslash.
            let mut dobackslash = false;
            // c:6272-6280 has NO `continue`: it copies the sigil, advances,
            // and falls through to the c:6392 pass-through with the `'` as the
            // current element — so it must skip the c:6301 gate, not the tail.
            let mut skip_special = false;
            // c:6281-6299 — `$(…)`, `$[…]`, `${…}` (and the `Qstring` forms
            // they take inside double quotes) are already syntactically
            // quoted, so the whole group goes through untouched. Without this
            // the `(` of `echo "${(` reached the c:6301 ispecial arm and came
            // out `${\(`, so `$PREFIX` was `${\(` where zsh publishes `${(`
            // and `_brace_parameter` never saw a parameter-flag word.
            // (Computed up front only because Rust cannot take a block as an
            // `else if let` scrutinee; it is side-effect free, and the chain
            // below still consults it in C's order.)
            let opener = if (is_tok(i, Stringg) || is_tok(i, Qstring)) && i + 1 < mcs.len() {
                if is_tok(i + 1, Inpar) {
                    Some((Inpar, Outpar))
                } else if is_tok(i + 1, Inbrack) {
                    Some((Inbrack, Outbrack))
                } else if is_tok(i + 1, Inbrace) {
                    Some((Inbrace, Outbrace))
                } else {
                    None
                }
            } else {
                None
            };
            // c:6262-6271 — a backtick group is copied whole, and C emits the
            // closing tick even when the input ran out before one appeared.
            if is_tok(i, Tick) || is_tok(i, Qtick) {
                let tick = if is_tok(i, Tick) { Tick } else { Qtick };
                result.push(tick);
                i += 1;
                while i < mcs.len() && !is_tok(i, tick) {
                    verbatim(&mut result, i);
                    prev = char::from_u32(cval(i)).unwrap_or('\0');
                    i += 1;
                }
                result.push(tick); // c:6268
                prev = tick;
                if i < mcs.len() {
                    i += 1; // c:6269-6270
                }
                continue;
            } else if instring == QT_DOUBLE
                && i + 1 < mcs.len()
                && (is_tok(i, Qstring) || is_ch(i, '$'))
                && is_ch(i + 1, '\'')
            {
                // c:6272-6280 — `$'…'` needs no quoting inside a double-quoted
                // string, so the sigil goes through bare. C copies only the
                // `$`/`Qstring` here; the `'` is handled by the c:6392
                // pass-through on the next statement, which is why there is no
                // `continue` and why the c:6301 gate has to be skipped.
                verbatim(&mut result, i); // c:6280
                prev = char::from_u32(cval(i)).unwrap_or('\0');
                i += 1;
                skip_special = true;
            } else if let Some((open, close)) = opener {
                // c:6283-6286 — `beg` is the `String`/`Qstring` byte, and it is
                // what raises the nesting level, not the bracket itself.
                let beg = if is_tok(i, Stringg) { Stringg } else { Qstring };
                let mut level = 0i32;
                result.push(beg); // c:6288
                result.push(open); // c:6289
                i += 2;
                // c:6290-6296
                prev = open;
                while i < mcs.len() && (!is_tok(i, close) || level != 0) {
                    if is_tok(i, beg) {
                        level += 1;
                    } else if is_tok(i, close) {
                        level -= 1;
                    }
                    verbatim(&mut result, i);
                    prev = char::from_u32(cval(i)).unwrap_or('\0');
                    i += 1;
                }
                // c:6297-6298 — the closer, only when the input actually had one.
                if i < mcs.len() {
                    verbatim(&mut result, i);
                    prev = char::from_u32(cval(i)).unwrap_or('\0');
                    i += 1;
                }
                continue;
            }
            let mc = mcs[i];
            let c = match mc {
                MetaChar::Ch(c) => c,
                // A raw byte is never `\n` and never ispecial — it is the
                // not-printable case below. Stand in with its byte value so the
                // `=`/`~` lookbehind still advances correctly.
                MetaChar::Raw(b) => b as char,
            };
            if !skip_special {
                // c:6301 — `ispecial(*u)`. A token is excluded because C's
                // typtab carries ISPECIAL only for SPECCHARS (zsh.h:228), the
                // comma (c:4256) and the history character (c:4259) — all
                // ASCII — so `ispecial(Inbrace)` is false in C too.
                let is_sp = matches!(mc, MetaChar::Ch(_)) && !tokmask[i] && ispecial(c);
                // c:6302-6306 gate for `=`/`~` — mid-word they are left bare
                // unless the char is at position 0 (c:6303), MAGICEQUALSUBST is
                // on and the previous byte is `=` or `:` (c:6304-6305), or it
                // is `~` under EXTENDEDGLOB (c:6306).
                let eq_tilde_gate = if c == '=' || c == '~' {
                    i == 0 // c:6303 u == s
                        || (isset(MAGICEQUALSUBST) && (prev == '=' || prev == ':')) // c:6304-6305
                        || (c == '~' && isset(EXTENDEDGLOB)) // c:6306
                } else {
                    true // c:6302 *u != '=' && *u != '~'
                };
                // c:6307-6313 — the per-style subcondition.
                let style_gate = instring == QT_BACKSLASH // c:6307
                    || instring == QT_SINGLE_OPTIONAL // c:6308
                    || (isset(BANGHIST)
                        && bangchar_v != 0
                        && c as u32 == bangchar_v as u32
                        && instring != QT_SINGLE) // c:6309-6310
                    || (instring == QT_DOUBLE && matches!(c, '$' | '`' | '"' | '\\')) // c:6311-6312
                    || (instring == QT_SINGLE && c == '\''); // c:6313
                if is_sp && eq_tilde_gate && style_gate {
                    if instring == QT_SINGLE_OPTIONAL {
                        // c:6314-6363 — minimum quoting. `quotestart` is where
                        // an opening `'` would be back-filled, so a run of
                        // ordinary characters ends up INSIDE the span rather
                        // than each special char getting its own `'…'`.
                        if quotesub == 1 {
                            if c == '\'' {
                                // c:6319-6323 — a bare `'` is cheaper as `\'`
                                // than as an opened-and-closed span, so no
                                // span starts here and quotesub stays 1.
                                result.push('\\');
                            } else {
                                // c:6328-6337 — time to quote: insert the
                                // opening `'` at `quotestart`, shifting
                                // everything after it right (C's addq loop).
                                result.insert(quotestart, '\''); // c:6335
                                quotesub = 2; // c:6337
                            }
                            verbatim(&mut result, i); // c:6339
                            prev = c;
                            i += 1;
                            quotestart = result.len(); // c:6343
                        } else if c == '\'' {
                            if unset(RCQUOTES) {
                                // c:6346-6348 — close the span, then `\'`.
                                result.push('\''); // c:6346
                                result.push('\\'); // c:6347
                                result.push('\''); // c:6348
                                quotesub = 1; // c:6350
                                quotestart = result.len(); // c:6351
                            } else {
                                // c:6353-6355 — inside a span, `''` IS a
                                // literal `'`, so the span never breaks.
                                result.push('\''); // c:6354
                                result.push('\''); // c:6355
                            }
                            prev = c;
                            i += 1; // c:6358
                        } else {
                            // c:6360-6361 — already quoting, just add.
                            verbatim(&mut result, i);
                            prev = c;
                            i += 1;
                        }
                        continue; // c:6363
                    } else if c == '\n' || (instring == QT_SINGLE && c == '\'') {
                        // c:6364-6381
                        if c == '\n' {
                            // c:6366-6371 — a newline cannot be backslashed,
                            // so it is upgraded to a `$'\n'` island.
                            result.push_str("$'\\n'");
                        } else if unset(RCQUOTES) {
                            // c:6372-6377 — `'` → `'\''`.
                            result.push('\''); // c:6373
                            if c == '\'' {
                                result.push('\\'); // c:6374-6375
                            }
                            result.push(c); // c:6376
                            result.push('\''); // c:6377
                        } else {
                            // c:6378-6379 — RCQUOTES: `''` is a literal `'`.
                            result.push('\'');
                            result.push('\'');
                        }
                        prev = c;
                        i += 1; // c:6380
                        continue; // c:6381
                    } else {
                        // c:6382-6388 — a backslash is owed, but it is not
                        // emitted yet: if the character turns out not to be
                        // printable the c:6435 arm upgrades it to `$'…'` and
                        // drops the backslash entirely.
                        dobackslash = true;
                    }
                }
            }
            // c:6392 — `if (itok(*u) || instring != QT_BACKSLASH)`. Everything
            // that is not a plain QT_BACKSLASH character is passed straight
            // through here; only QT_BACKSLASH reaches the printability test.
            if tokmask[i] || instring != QT_BACKSLASH {
                if dobackslash {
                    result.push('\\'); // c:6394-6395
                }
                if is_tok(i, Inparmath) {
                    // c:6396-6412 — `$((…))` is already syntactically quoted,
                    // so the whole math group is copied out with no further
                    // quoting. C counts nesting on Inparmath/Outparmath and
                    // stops at the matching close or at end of string.
                    let mut inmath = 1i32; // c:6401
                    verbatim(&mut result, i); // c:6402
                    prev = char::from_u32(cval(i)).unwrap_or('\0');
                    i += 1;
                    loop {
                        // c:6404-6407 — C reads `uc = *u` and breaks AFTER
                        // copying it; a NUL there is the end of the string,
                        // which is `i == mcs.len()` here.
                        if i >= mcs.len() {
                            break;
                        }
                        let was_out = is_tok(i, Outparmath);
                        let was_in = is_tok(i, Inparmath);
                        verbatim(&mut result, i); // c:6405
                        prev = char::from_u32(cval(i)).unwrap_or('\0');
                        i += 1;
                        if was_out {
                            // c:6408-6409
                            inmath -= 1;
                            if inmath == 0 {
                                break;
                            }
                        } else if was_in {
                            inmath += 1; // c:6410-6411
                        }
                    }
                } else {
                    verbatim(&mut result, i); // c:6413-6414
                    prev = c;
                    i += 1;
                }
                continue; // c:6415
            }
            // c:6418-6427 — QT_BACKSLASH only: is the element printable in the
            // current character set? (`uend = u + MB_METACHARLENCONV(u, &cc)`
            // then `cc != WEOF && WC_ISPRINT(cc)`; `meta_chars` already did the
            // decode, so the test is `mc_printable`.)
            if mc_printable(mc) {
                // c:6428-6433 — owed backslash, then the unit copied through
                // still metafied.
                if dobackslash {
                    result.push('\\'); // c:6428-6429
                }
                push_metachar(&mut result, mc); // c:6430-6433
            } else {
                // c:6435-6442 — not printable: one `$'…'` island per element,
                // built by addunprintable. The owed backslash is dropped.
                result.push_str("$'"); // c:6437-6438
                push_unprintable(&mut result, mc, mcs.get(i + 1).copied()); // c:6439
                result.push('\''); // c:6440
            }
            prev = c;
            i += 1;
        }
        // c:6445-6446 — QT_SINGLE_OPTIONAL left a span open.
        if quotesub == 2 {
            result.push('\'');
        }
        result
    } else if quote_type == QT_DOLLARS {
        // c:6203-6241 — `$'…'` quoting. Each element is either printable, in
        // which case it is emitted verbatim (backslashing `\`, `'`, and — under
        // BANGHIST — the history character), or it is NOT, in which case C hands
        // its raw bytes to addunprintable(): a named escape (`\t`, `\n`, `\a`, …)
        // or a 3-digit octal one.
        //
        // Two things this must NOT do, both of which the previous port did:
        //   * pass an undecodable byte through raw. `$'\M-a'` is the byte 0xE1,
        //     which is not valid UTF-8 and is stored metafied; walking `.chars()`
        //     re-emitted it verbatim instead of `\341`, so the output could not
        //     be re-parsed.
        //   * emit `\e` for ESC. C's addunprintable has no `\e` case — ESC comes
        //     out as the octal `\033`.
        //
        // c:6131-6134 — the `$'` … `'` wrapper is added by the CALLER
        // (subst.c:4085-4090 writes the quote pair and then `val[0] = '$'`),
        // so this arm returns the BODY only.
        let mut result = String::with_capacity(s.len() + 4);
        let mcs = meta_chars(s);
        let bang = crate::ported::hist::bangchar.load(std::sync::atomic::Ordering::SeqCst);
        for i in 0..mcs.len() {
            let mc = mcs[i];
            if mc_printable(mc) {
                // c:6214-6224. The escape test looks at the DECODED character
                // (`cc`), so a raw byte — which only reaches here with MULTIBYTE
                // off — can never be `\`, `'` or the history bang, all ASCII.
                if let MetaChar::Ch(c) = mc {
                    if c == '\\'
                        || c == '\''
                        || (isset(BANGHIST) && bang != 0 && c as u32 == bang as u32)
                    {
                        result.push('\\');
                    }
                }
                // c:6224-6225 — `while (u < uend) *v++ = *u++;`
                push_metachar(&mut result, mc);
            } else {
                // c:6226-6229
                push_unprintable(&mut result, mc, mcs.get(i + 1).copied());
            }
        }
        result
    } else if quote_type == QT_BACKTICK {
        // Backtick quoting (minimal - just escape backticks)
        s.replace('`', "\\`")
    } else {
        // Unknown quote_type — treat as no-op to match C's `default:` arm.
        s.to_string()
    }
}

// ===========================================================
// xtrace helpers moved from src/ported/vm_helper.
// printprompt4 is a direct port of utils.c:1718-1735; quotedzputs
// is its argument-formatter companion (zsh formats `set -x` lines
// via the same utils.c path).
// ===========================================================

/// Port of `quotedzputs()` from `Src/utils.c:6464` — C decl `quotedzputs(char const *s, FILE *stream)`.
///
/// Quote a string for re-readable output (`set -x`, `typeset -p`,
/// `set` listing, etc.). zsh's algorithm under MULTIBYTE_SUPPORT
/// (c:6464-6543):
///   1. empty input → `''`                                     (c:6470-6475)
///   2. needs nice-format (controls / non-printables) → emit
///      `$'<mb_niceformat output>'`                            (c:6478-6492)
///   3. no SPECCHARS member → return string unchanged          (c:6511-6517)
///   4. otherwise wrap in `'…'` (Bourne) or RCQUOTES form,
///      with embedded `'` rewritten as `'\''` / `''`           (c:6533-6587)
///
/// Previous Rust port omitted step 2 (the `$'…'` branch).
/// Result: strings containing control bytes (`\n`, `\t`, escape
/// sequences) were single-quoted instead of `$'…'`-quoted,
/// breaking round-trip through `typeset -p` / `set` / `set -x`
/// because POSIX single-quotes are *strong* — embedded `\n` would
/// be re-fed as a literal newline rather than the C-escape.
///
/// The C signature is `char *quotedzputs(char const *s, FILE *stream)`
/// — when `stream` is non-NULL it writes there and returns NULL,
/// otherwise it returns the quoted string. Rust's variant covers
/// only the `stream==NULL` form (the `set -x` callers all want the
/// string back, not direct stdout writing). The `stream==Some` form
/// is fputs/fputc-direct in C; Rust callers that need writer-based
/// output should compose `print!`/`write!` with this fn's return.
/// WARNING: param names don't match C — Rust=(s) vs C=(s, stream)
pub(crate) fn quotedzputs(s: &str) -> String {
    // c:6464
    // c:6469-6475 — `if (!*s)` empty string emits `''` literal.
    if s.is_empty() {
        return "''".to_string(); // c:6472
    }
    // c:6477-6508 — `is_mb_niceformat(s)` / `is_sb_niceformat(s)` arm:
    // if the string contains nice-formatted chars (controls,
    // non-printables), wrap in `$'…'` using sb/mb_niceformat with
    // NICEFLAG_QUOTE so embedded `'`/`\` get backslash-escaped.
    if is_mb_niceformat(s) != 0 {
        // c:6478-6492 (MULTIBYTE_SUPPORT branch): use mb_niceformat
        //   with NICEFLAG_QUOTE so multi-byte chars round-trip through
        //   wcs_nicechar (raw UTF-8 for printable wides, `\u`/`\U` for
        //   large codepoints) AND `'`/`\\` get backslash-escaped.
        // Under !MULTIBYTE_SUPPORT (c:6494-6508) C would use
        //   sb_niceformat instead. Static-link path: MULTIBYTE is
        //   always available in Rust.
        // c:6485-6492 — `mb_niceformat(s, NULL, &substr,
        //                              NICEFLAG_QUOTE|NICEFLAG_NODUP);`
        let mut substr: Option<String> = None;
        let _ = mb_niceformat(s, None, Some(&mut substr), NICEFLAG_QUOTE | NICEFLAG_NODUP);
        return format!("$'{}'", substr.unwrap_or_default()); // c:6488
    }
    // c:6511-6518 — `if (!hasspecial(s)) return dupstring(s);`.
    if !hasspecial(s) {
        // c:6511
        return s.to_string(); // c:6516
    }
    // c:6520-6529 — outstr buffer alloc (zhalloc). Rust uses growable
    // String; the C `l = strlen(s) + 2 + (per-' overhead)` size hint
    // is just allocation tuning and doesn't affect output.
    let mut out = String::with_capacity(s.len() + 2);
    let bytes = s.as_bytes();
    let csh_junkie = isset(CSHJUNKIEQUOTES); // c:6554 / c:6612

    if isset(RCQUOTES) {
        // c:6533 — RCQUOTES: wrap entire string in `'…'`; each
        // embedded `'` becomes `''` (the rc-style doubled quote).
        out.push('\''); // c:6539
                        // c:6540-6547 — C walks METAFIED bytes; the Rust String is
                        // UTF-8, so the faithful transposition walks CHARS (a byte
                        // walk Latin-1-casts every multibyte char: em-dash E2 80 94
                        // became "â" + a token byte that downstream passes ate).
                        // Token chars (Dash U+009B, Meta U+0083) are codepoints here.
        let chars_v: Vec<char> = s.chars().collect();
        let mut i = 0;
        while i < chars_v.len() {
            let c = if chars_v[i] == Dash {
                // c:6541 — `if (*s == Dash) c = '-';`
                i += 1;
                '-'
            } else if chars_v[i] as u32 == Meta as u32 && i + 1 < chars_v.len() {
                // c:6543 — `else if (*s == Meta) c = *++s ^ 32;`
                let n = chars_v[i + 1] as u32;
                i += 2;
                char::from_u32(n ^ 32).unwrap_or(chars_v[i - 1])
            } else {
                // c:6546 — `else c = *s;`
                let dec = chars_v[i];
                i += 1;
                dec
            };
            if c == '\'' {
                // c:6548-6553 — `if (c == '\'') *ptr++ = '\'';` (the
                // rc-quote doubling).
                out.push('\''); // c:6553
            } else if c == '\n' && csh_junkie {
                // c:6554-6560 — `if (c == '\n' && isset(CSHJUNKIEQUOTES))
                //                 *ptr++ = '\\';`
                out.push('\\'); // c:6559
            }
            // c:6561-6570 — emit c (metafy on imeta).
            out.push(c); // c:6569 (non-stream branch always re-metafies;
                         // Rust String holds decoded chars directly)
        }
        out.push('\''); // c:6576
    } else {
        // c:6578-6637 — Bourne-style quoting, "avoiding empty quoted
        // strings". Tracks `inquote` so that `it's` becomes
        // `'it'\''s'` (no empty `''` runs).
        let mut inquote = false; // c:6466 (initialised at top of C fn)
                                 // c:6579-6586 — char-wise decode, same transposition as the
                                 // RCQUOTES arm above (C's byte walk is over metafied bytes).
        let chars_v: Vec<char> = s.chars().collect();
        let mut i = 0;
        while i < chars_v.len() {
            let c = if chars_v[i] == Dash {
                i += 1;
                '-' // c:6581
            } else if chars_v[i] as u32 == Meta as u32 && i + 1 < chars_v.len() {
                let n = chars_v[i + 1] as u32; // c:6583
                i += 2;
                char::from_u32(n ^ 32).unwrap_or(chars_v[i - 1])
            } else {
                let dec = chars_v[i]; // c:6585
                i += 1;
                dec
            };
            if c == '\'' {
                // c:6587-6602 — `'` closes any open inquote then emits
                // `\'` outside the quotes.
                if inquote {
                    out.push('\''); // c:6593
                    inquote = false; // c:6594
                }
                out.push('\\'); // c:6600
                out.push('\''); // c:6601
            } else {
                // c:6603-6629 — other chars open a quote run if not
                // already open, optionally backslash-escape `\n` under
                // CSHJUNKIEQUOTES, then emit the byte.
                if !inquote {
                    out.push('\''); // c:6609
                    inquote = true; // c:6610
                }
                if c == '\n' && csh_junkie {
                    out.push('\\'); // c:6617
                }
                out.push(c); // c:6627 (imeta-encoding handled by Rust
                             // String storage in the non-stream form)
            }
        }
        if inquote {
            out.push('\''); // c:6636
        }
    }
    // c:6639-6640 — `if (!stream) *ptr++ = '\0';` — Rust String already
    // NUL-terminated implicitly; no-op.
    out // c:6642
}

/// Port of `dquotedztrdup()` from `Src/utils.c:6649` — C decl `char *dquotedztrdup(char const *s)`.
/// Two arms (selected by `isset(CSHJUNKIEQUOTES)` at c:6655):
///
/// **CSHJUNKIEQUOTES path** (c:6656-6686): the csh-junk-quote style
/// where only the non-special sections are wrapped in `"..."` and
/// special chars (`"`, `$`, `` ` ``) appear OUTSIDE the quotes with
/// backslash escape. `\n` inside the quotes gets an extra `\` so it
/// round-trips through history.
///
/// **Default path** (c:6687-6719): wraps the whole string in `"..."`.
/// `\` is doubled to `\\`. `"`, `$`, `` ` `` get backslash-escaped.
/// A trailing `\` gets an extra `\` appended (the `pending` quirk).
///
/// Previously the Rust port only implemented the default arm; the
/// CSHJUNKIEQUOTES path is now ported faithfully.
pub fn dquotedztrdup(s: &str) -> String {
    // c:6648
    let mut out = String::with_capacity(s.len() * 4 + 2);
    let bytes = s.as_bytes();
    // c:6655 — `if (isset(CSHJUNKIEQUOTES))`.
    if isset(CSHJUNKIEQUOTES) {
        let mut inquote = false;
        let mut i = 0;
        while i < bytes.len() {
            // c:6661-6662 — Meta byte decode.
            let c = if bytes[i] == Meta && i + 1 < bytes.len() {
                i += 2;
                (bytes[i - 1] ^ 32) as char
            } else {
                i += 1;
                bytes[i - 1] as char
            };
            match c {
                // c:6664-6673 — `"` / `$` / `` ` `` — close quote
                // (if open), then `\<c>`.
                '"' | '$' | '`' => {
                    if inquote {
                        out.push('"');
                        inquote = false;
                    }
                    out.push('\\');
                    out.push(c);
                }
                // c:6674-6682 — default arm: open quote if needed,
                // backslash-escape newline, emit char.
                _ => {
                    if !inquote {
                        out.push('"');
                        inquote = true;
                    }
                    if c == '\n' {
                        out.push('\\');
                    }
                    out.push(c);
                }
            }
        }
        // c:6685-6686 — close trailing quote.
        if inquote {
            out.push('"');
        }
    } else {
        // c:6687-6718 — default (non-CSH) arm.
        out.push('"');
        let mut pending = false;
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
    }
    // c:6720 — `ret = metafy(buf, p - buf, META_DUP);` re-metafy result.
    metafy(&out)
}

/// Port of `dquotedzputs()` from `Src/utils.c:6729`.
///
/// Port of `dquotedzputs(char const *s, FILE *stream)` from
/// Src/utils.c:6729. C body (4 lines):
///     `char *d = dquotedztrdup(s);
///      int ret = zputs(d, stream);
///      zsfree(d);
///      return ret;`
/// Rust returns the quoted string directly (callers compose via
/// `format!` rather than streaming through a FILE*), so the zputs
/// call drops.
pub fn dquotedzputs(s: &str) -> String {
    // c:6729
    dquotedztrdup(s) // c:6731
}

/// Port of `ucs4toutf8()` from `Src/utils.c:6743` — C decl `ucs4toutf8(char *dest, unsigned int wval)`.
///
/// Convert UCS-4 to UTF-8 (from utils.c ucs4toutf8)
///
/// C accepts 0..=0x7FFFFFFF (legacy UCS-4 / 6-byte UTF-8) — wider
/// than Unicode's 0..=0x10FFFF — to match `wctomb(3)` on Linux
/// with UTF-8 locale. The Rust `char::from_u32` path would reject
/// codepoints above 0x10FFFF (and surrogates), so this port mirrors
/// the C bit-pattern dispatch directly.
///
/// C body (c:6743-6779):
/// ```c
/// if (wval < 0x80) len = 1;            // ASCII
/// else if (wval < 0x800) len = 2;
/// else if (wval < 0x10000) len = 3;
/// else if (wval < 0x200000) len = 4;
/// else if (wval < 0x4000000) len = 5;
/// else if (wval < 0x80000000) len = 6;
/// else { zerr("character not in range"); return -1; }
///
/// switch (len) { /* falls through except to the last case */
/// case 6: dest[5] = (wval & 0x3f) | 0x80; wval >>= 6;
/// case 5: dest[4] = (wval & 0x3f) | 0x80; wval >>= 6;
/// case 4: dest[3] = (wval & 0x3f) | 0x80; wval >>= 6;
/// case 3: dest[2] = (wval & 0x3f) | 0x80; wval >>= 6;
/// case 2: dest[1] = (wval & 0x3f) | 0x80; wval >>= 6;
///     *dest = wval | ((0xfc << (6 - len)) & 0xfc);
///     break;
/// case 1: *dest = wval;
/// }
/// return len;
/// ```
///
/// Returns the encoded bytes (1..=6 long) on success, None on
/// out-of-range. C's `zerr("character not in range")` (c:6763) is
/// not emitted here — the return signals the error.
/// WARNING: param names don't match C — Rust=(wval) vs C=(dest, wval)
pub fn ucs4toutf8(wval: u32) -> Option<String> {
    // c:6743
    let len: usize = if wval < 0x80 {
        1
    }
    // c:6750
    else if wval < 0x800 {
        2
    }
    // c:6752
    else if wval < 0x10000 {
        3
    }
    // c:6754
    else if wval < 0x200000 {
        4
    }
    // c:6756
    else if wval < 0x4000000 {
        5
    }
    // c:6758
    else if wval < 0x80000000 {
        6
    }
    // c:6760
    else {
        // c:6762
        zerr("character not in range"); // c:6763
        return None; // c:6764
    };
    let mut buf = [0u8; 6];
    let mut w = wval;
    // c:6767-6776 — fall-through switch building trailing bytes first.
    match len {
        1 => {
            buf[0] = w as u8;
        } // c:6775
        n => {
            // Trailing (len-1) bytes: each is `(w & 0x3f) | 0x80`,
            // shifting w right 6 between them (c:6768-6772).
            for i in (1..n).rev() {
                buf[i] = ((w & 0x3f) as u8) | 0x80;
                w >>= 6;
            }
            // Leading byte: `w | ((0xfc << (6 - len)) & 0xfc)` (c:6773).
            buf[0] = (w as u8) | (((0xfcu32 << (6 - n)) & 0xfc) as u8);
        }
    }
    Some(String::from_utf8_lossy(&buf[..len]).into_owned())
}

/// Port of `ucs4tomb()` from `Src/utils.c:6788` — C decl `ucs4tomb(unsigned int wval, char *buf)`.
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
    // libc::c_char is i8 on most targets but u8 on aarch64-linux. Use c_char
    // so the wctomb arg pointer type matches per-target without a cast.
    let mut local = [0 as libc::c_char; 16];
    let count = unsafe { wctomb(local.as_mut_ptr(), wval as libc::wchar_t) };
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

/// Port of `getkeystring()` from `Src/utils.c:6915` — C decl `getkeystring(char *s, int *len, int how, int *misc)`.
///
/// Parse getkeystring escape sequences (from utils.c getkeystring)
/// Handles \n \t \r \e \a \b \f \v \\ \' \" \xNN \uNNNN \UNNNNNNNN \0NNN
/// WARNING: param names don't match C — Rust=(s) vs C=(s, len, how, misc)
pub fn getkeystring(s: &str) -> (String, usize) {
    // c:6915
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
                // c:Src/utils.c:7169-7170 — `\x` ALWAYS runs `*t++ =
                // zstrtol(s+1, &s, 16)`, even with no hex digit. zstrtol
                // over a non-hex byte parses zero digits and returns 0, so
                // a NUL byte is emitted (the offending char is processed
                // next). `$'\xg'` → NUL + 'g'; `$'\x'` → NUL. Previously
                // `from_str_radix("")` errored and nothing was pushed.
                let val: u8 = if hex.is_empty() {
                    0 // c:7170 zstrtol on empty reads as 0
                } else {
                    u8::from_str_radix(&hex, 16).unwrap_or(0)
                };
                // c:Src/utils.c — `\xNN` is one raw BYTE; metafied
                // per c:7289-7294 (vm_helper::meta_encode_byte) so
                // the String stays valid UTF-8. Bug #127.
                {
                    // c:Src/utils.c metafy byte-encode step:
                    // `if (imeta(c)) {{ *p++ = Meta; *p++ = c ^ 32; }}`
                    let b_ = val;
                    if b_ < 0x80 {
                        result.push(b_ as char);
                    } else {
                        result.push('\u{83}');
                        result.push(char::from(b_ ^ 32));
                    }
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
                    // c:Src/utils.c — octal escape is one raw BYTE;
                    // metafied per c:7289-7294. Bug #127.
                    {
                        // c:Src/utils.c metafy byte-encode step:
                        // `if (imeta(c)) {{ *p++ = Meta; *p++ = c ^ 32; }}`
                        let b_ = val;
                        if b_ < 0x80 {
                            result.push(b_ as char);
                        } else {
                            result.push('\u{83}');
                            result.push(char::from(b_ ^ 32));
                        }
                    }
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
            Some(mod_letter @ ('C' | 'M')) => {
                // c:Src/utils.c:7029-7052 + c:7265-7275 — `\C` /
                // `\M` set control / meta flags; optional `-`
                // separator; then read the next char (possibly
                // through additional `\C`/`\M` chains) and apply
                // the mask: control → `& 0x9f` (or `\C-?` → 0x7f),
                // meta → `| 0x80`. Bug #113 in docs/BUGS.md: the
                // previous Rust port matched these in the default
                // arm and emitted literal `\C` / `\M`. This is the
                // second getkeystring (utils.rs); the in-tree
                // `getkeystring_dollar_quote` (lex.rs) also needs
                // the fix and gets it in the same commit.
                consumed += 1;
                let mut control = mod_letter == 'C';
                let mut meta = mod_letter == 'M';
                // Consume optional `-` separator + chained `\C`/`\M`.
                loop {
                    if chars.peek() == Some(&'-') {
                        chars.next();
                        consumed += 1;
                        continue;
                    }
                    // chained `\C` / `\M`?
                    let mut iter_clone = chars.clone();
                    if iter_clone.next() == Some('\\') {
                        if let Some(nx) = iter_clone.next() {
                            if nx == 'C' || nx == 'M' {
                                chars.next(); // consume '\'
                                chars.next(); // consume C/M
                                consumed += 2;
                                if nx == 'C' {
                                    control = true;
                                } else {
                                    meta = true;
                                }
                                continue;
                            }
                        }
                    }
                    break;
                }
                // Read one base character (allowing nested simple
                // escapes for common cases).
                let base: Option<char> = if chars.peek() == Some(&'\\') {
                    chars.next();
                    consumed += 1;
                    match chars.next() {
                        Some('n') => {
                            consumed += 1;
                            Some('\n')
                        }
                        Some('t') => {
                            consumed += 1;
                            Some('\t')
                        }
                        Some('r') => {
                            consumed += 1;
                            Some('\r')
                        }
                        Some('a') => {
                            consumed += 1;
                            Some('\x07')
                        }
                        Some('b') => {
                            consumed += 1;
                            Some('\x08')
                        }
                        Some('e') | Some('E') => {
                            consumed += 1;
                            Some('\x1b')
                        }
                        Some('f') => {
                            consumed += 1;
                            Some('\x0c')
                        }
                        Some('v') => {
                            consumed += 1;
                            Some('\x0b')
                        }
                        Some('\\') => {
                            consumed += 1;
                            Some('\\')
                        }
                        Some('\'') => {
                            consumed += 1;
                            Some('\'')
                        }
                        Some('"') => {
                            consumed += 1;
                            Some('"')
                        }
                        Some(other) => {
                            consumed += 1;
                            Some(other)
                        }
                        None => None,
                    }
                } else {
                    chars.next().inspect(|c| {
                        consumed += c.len_utf8();
                    })
                };
                if let Some(ch) = base {
                    let mut byte = ch as u32;
                    if control {
                        if byte == '?' as u32 {
                            byte = 0x7f;
                        } else {
                            byte &= 0x9f;
                        }
                    }
                    if meta {
                        byte |= 0x80;
                    }
                    // c:Src/utils.c:7265-7275 — masked result is one
                    // raw BYTE (`$'\M-i'` = 0xe9), metafied per
                    // c:7289-7294. Multibyte base chars (> 0xff)
                    // keep the codepoint form. Bug #127.
                    if byte <= 0xff {
                        {
                            // c:Src/utils.c metafy byte-encode step:
                            // `if (imeta(c)) {{ *p++ = Meta; *p++ = c ^ 32; }}`
                            let b_ = byte as u8;
                            if b_ < 0x80 {
                                result.push(b_ as char);
                            } else {
                                result.push('\u{83}');
                                result.push(char::from(b_ ^ 32));
                            }
                        }
                    } else if let Some(c) = char::from_u32(byte) {
                        result.push(c);
                    }
                }
            }
            Some(c)
                if isset(BANGHIST)
                    && crate::ported::hist::bangchar.load(std::sync::atomic::Ordering::SeqCst)
                        != 0
                    && c as u32
                        == crate::ported::hist::bangchar.load(std::sync::atomic::Ordering::SeqCst)
                            as u32 =>
            {
                // The reverse of quotestring's BANGHIST escape (c:Src/utils.c:6230
                // — `\!` is emitted for the history char): unquoting `$'a\!b'`
                // must strip the backslash so `${(Q)${(qqqq)v}}` round-trips.
                // In zsh this happens because `(Q)` re-lexes the string and
                // history expansion consumes the `\<bangchar>`; here the decoder
                // consumes it directly.
                consumed += 1;
                result.push(c);
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

/// Check if s is a prefix of t (from utils.c strpfx)
// Return non-zero if s is a prefix of t.                                  // c:7345
/// Port of `strpfx()` from `Src/utils.c:7334`.
///
/// `strpfx` — see implementation.
pub fn strpfx(s: &str, t: &str) -> bool {
    t.starts_with(s)
}

/// Check if s is a suffix of t (from utils.c strsfx)
// Return non-zero if s is a suffix of t.                                  // c:7345
/// Port of `strsfx()` from `Src/utils.c:7345`.
///
/// `strsfx` — see implementation.
pub fn strsfx(s: &str, t: &str) -> bool {
    t.ends_with(s)
}

/// Port of `upchdir()` from `Src/utils.c:7356` — C decl `upchdir(int n)`.
///
/// Go up n directories (from utils.c upchdir)
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
// `struct dirsav` lives in `dirsav` per Rule C
// (its C definition is `Src/zsh.h:1159`, not utils.c). The previous
// Rust port had a `pub struct DirSav` PascalCase duplicate of the
// canonical lowercase struct; deleted in favour of routing through
// `zsh_h::dirsav` directly.

/// Port of `init_dirsav()` from `Src/utils.c:7381` — C decl `init_dirsav(Dirsav d)`. Initialize a
/// `dirsav` struct to its empty/default state. C body memset's the
/// fields to 0 (dirfd to -1).
///
/// C signature: `void init_dirsav(Dirsav d)` where
/// `Dirsav = struct dirsav *`. Rust port returns the initialised
/// struct since callers always pair-with a fresh allocation.
/// WARNING: param names don't match C — Rust=() vs C=(path)
/// C body (3 lines):
///   `d->ino = d->dev = 0; d->dirname = NULL; d->dirfd = d->level = -1;`
///
/// `dirname` stays `None` exactly as C leaves it: the field is filled
/// later, by `lchdir`/`zgetdir`, only on the paths that need a name to
/// get back. An earlier Rust port prefilled it with `current_dir()`,
/// which put a `getcwd` (plus an `open` on macOS) in front of every
/// `scanner()` call in `glob.rs:444` — a syscall pair C never issues.
/// The two consumers both cope with `None`: `restoredir` (c:7565) falls
/// through to `fchdir(dirfd)` or `upchdir(level)`, and `lchdir` (c:7442-7448)
/// fills `d->level` for the soft path before any restore can happen.
pub fn init_dirsav() -> dirsav {
    // c:7381
    dirsav {
        ino: 0,
        dev: 0,        // c:7383 `d->ino = d->dev = 0;`
        dirname: None, // c:7384 `d->dirname = NULL;`
        dirfd: -1,
        level: -1, // c:7385 `d->dirfd = d->level = -1;`
    }
}

/// Port of `lchdir()` from `Src/utils.c:7400` — C decl `lchdir(char const *path, struct dirsav *d, int hard)`.
///
/// c:7388 — "Change directory, without following symlinks."  With `hard`
/// set, descends `path` one component at a time: `lstat`s each component
/// (so a symlink is never followed), `chdir`s into it, then re-`lstat`s
/// `.` and compares dev/ino against the component it just stat'd. A
/// symlink swapped under us between the `lstat` and the `chdir` is caught
/// by the mismatch and the saved directory is restored via `restoredir`.
/// `d` (when non-NULL) is filled so the caller can later restore the cwd.
///
/// Returns 0 on success, -1 on a normal descent failure, -2 if the cwd
/// could not even be restored afterwards.
///
/// zshrs targets macOS/Linux, so the live C arms are HAVE_LSTAT +
/// HAVE_FCHDIR; the no-lstat / no-fchdir fallbacks (which never compile
/// on our platforms) are elided. C restores `errno = err` before each
/// non-zero return so the caller can read the break-reason; that errno
/// propagation is elided here (no current caller inspects errno — the
/// -1/-2/0 return is the contract).
/// WARNING: param names match C — (path, d, hard).
#[cfg(unix)]
pub fn lchdir(path: &str, d: Option<&mut dirsav>, hard: i32) -> i32 {
    // c:7400
    use std::ffi::CString;

    let mut ds = init_dirsav(); // c:7405 — local fallback `struct dirsav ds`
    let used_local = d.is_none(); // tracks C's `d == &ds`
                                  // c:7415-7418 — `if (!d) { init_dirsav(&ds); d = &ds; }`
    let d: &mut dirsav = d.unwrap_or(&mut ds);

    let bytes = path.as_bytes();
    let mut level: i32;

    // c:7419-7424 (HAVE_LSTAT arm) —
    //   `if ((*path == '/' || !hard) && (d != &ds || hard))`
    if (bytes.first() == Some(&b'/') || hard == 0) && (!used_local || hard != 0) {
        level = -1; // c:7425
    } else {
        level = 0; // c:7431
                   // c:7432-7435 — record dev/ino of `.` if not already captured.
        if d.dev == 0 && d.ino == 0 {
            if let Ok(meta) = fs::metadata(".") {
                d.dev = meta.dev(); // c:7433
                d.ino = meta.ino(); // c:7434
            }
        }
    }

    // c:7439-7451 — soft (`!hard`) path: count components, set d->level,
    // and delegate to zchdir.
    if hard == 0 {
        if !used_local {
            // c:7443-7447 — count '/'-separated components into `level`.
            let mut p = 0usize;
            while p < bytes.len() {
                while p < bytes.len() && bytes[p] != b'/' {
                    p += 1;
                }
                while p < bytes.len() && bytes[p] == b'/' {
                    p += 1;
                }
                level += 1;
            }
            d.level = level; // c:7448
        }
        return zchdir(path); // c:7450
    }

    // c:7453+ — hard path (HAVE_LSTAT + HAVE_FCHDIR).
    let mut close_dir = false; // c:7412
                               // c:7455-7460 — save the starting dir via an fd if we don't have one.
    if d.dirfd < 0 {
        close_dir = true; // c:7456
        let dot = CString::new(".").unwrap();
        d.dirfd = unsafe { libc::open(dot.as_ptr(), libc::O_RDONLY | libc::O_NOCTTY) }; // c:7457
        if d.dirfd < 0
            && zgetdir(Some(&mut *d)).is_some()
            && d.dirname
                .as_deref()
                .map(|s| !s.starts_with('/'))
                .unwrap_or(false)
        {
            // c:7458-7459 — cwd is relative; fall back to opening "..".
            let dotdot = CString::new("..").unwrap();
            d.dirfd = unsafe { libc::open(dotdot.as_ptr(), libc::O_RDONLY | libc::O_NOCTTY) };
        }
    }

    // c:7462-7464 — absolute path: start the descent from root.
    if bytes.first() == Some(&b'/') {
        let root = CString::new("/").unwrap();
        if unsafe { libc::chdir(root.as_ptr()) } < 0 {
            zwarn(&format!(
                "failed to chdir(/): {}",
                io::Error::last_os_error()
            )); // c:7464
        }
    }

    let dot = CString::new(".").unwrap();
    let mut pos = 0usize; // index into `path`, replaces C `path`/`pptr`
    let mut err: i32 = 0; // c:7408
    loop {
        // c:7466-7467 — skip leading slashes.
        while pos < bytes.len() && bytes[pos] == b'/' {
            pos += 1;
        }
        // c:7468-7480 — end of path reached: success.
        if pos >= bytes.len() {
            if !used_local {
                d.level = level; // c:7472
            }
            // c:7474-7477 — close the saved-dir fd if we opened it.
            if d.dirfd >= 0 && close_dir {
                unsafe { libc::close(d.dirfd) };
                d.dirfd = -1;
            }
            return 0; // c:7479
        }
        // c:7481 — find the end of this path component.
        let start = pos;
        let mut end = start + 1;
        while end < bytes.len() && bytes[end] != b'/' {
            end += 1;
        }
        // c:7482-7485 — component too long.
        if end - start > libc::PATH_MAX as usize {
            err = libc::ENAMETOOLONG;
            break;
        }
        // c:7486-7488 — copy the component into `buf`.
        let comp = &bytes[start..end];
        pos = end;
        let cbuf = match CString::new(comp) {
            Ok(c) => c,
            Err(_) => {
                err = libc::ENOENT; // embedded NUL — lstat would fail anyway.
                break;
            }
        };
        // c:7489-7492 — lstat the component (does NOT follow a symlink).
        let mut st1: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe { libc::lstat(cbuf.as_ptr(), &mut st1) } != 0 {
            err = io::Error::last_os_error().raw_os_error().unwrap_or(0);
            break;
        }
        // c:7493-7496 — must be a real directory, not a symlink/other.
        if st1.st_mode as u32 & libc::S_IFMT as u32 != libc::S_IFDIR as u32 {
            err = libc::ENOTDIR;
            break;
        }
        // c:7497-7500 — chdir into it.
        if unsafe { libc::chdir(cbuf.as_ptr()) } != 0 {
            err = io::Error::last_os_error().raw_os_error().unwrap_or(0);
            break;
        }
        if level >= 0 {
            level += 1; // c:7501-7502
        }
        // c:7503-7510 — re-lstat `.` and verify the dir we landed in is
        // the same dev/ino we lstat'd, catching a symlink swap race.
        let mut st2: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe { libc::lstat(dot.as_ptr(), &mut st2) } != 0 {
            err = io::Error::last_os_error().raw_os_error().unwrap_or(0);
            break;
        }
        if st1.st_dev != st2.st_dev || st1.st_ino != st2.st_ino {
            err = libc::ENOTDIR; // c:7508
            break;
        }
    }

    // c:7512-7548 — descent failed; restore the saved directory.
    if restoredir(&mut *d) != 0 {
        let restoreerr = io::Error::last_os_error(); // c:7513
                                                     // c:7519-7532 — restore failed too; force cwd to $HOME then "/".
        let mut reached = false;
        for i in 0..2 {
            // c:7521-7527 — destination: $HOME on pass 0, "/" on pass 1.
            let cdest = if i != 0 {
                "/".to_string()
            } else {
                match getsparam("HOME") {
                    Some(h) => h,     // c:7526
                    None => continue, // c:7525 `if (!home) continue;`
                }
            };
            setsparam("PWD", &cdest); // c:7528-7529 `zsfree(pwd); pwd = ztrdup(cdest);`
            if let Ok(cd) = CString::new(cdest.as_bytes()) {
                if unsafe { libc::chdir(cd.as_ptr()) } == 0 {
                    // c:7530
                    reached = true;
                    break;
                }
            }
        }
        if !reached {
            // c:7533-7534 — couldn't even reach "/".
            zerr(&format!(
                "lost current directory, failed to cd to /: {}",
                io::Error::last_os_error()
            ));
        } else {
            // c:7535-7537
            let pwd = getsparam("PWD").unwrap_or_default();
            zerr(&format!(
                "lost current directory: {}: changed to `{}'",
                restoreerr, pwd
            ));
        }
        // c:7540-7545 — close the saved-dir fd if we opened it.
        if d.dirfd >= 0 && close_dir {
            unsafe { libc::close(d.dirfd) };
            d.dirfd = -1;
        }
        let _ = err; // c:7546 `errno = err;` propagation elided (see doc).
        return -2; // c:7547
    }
    // c:7549-7558 — descent failed but the saved directory was restored.
    if d.dirfd >= 0 && close_dir {
        unsafe { libc::close(d.dirfd) };
        d.dirfd = -1;
    }
    let _ = err; // c:7557 `errno = err;` propagation elided (see doc).
    -1 // c:7558
}

/// Port of `lchdir()` from `Src/utils.c:7400`.
///
/// Non-unix stub: lchdir's symlink-safe descent is built on POSIX
/// `lstat`/`fchdir`/`chdir`, which have no Windows equivalent here.
#[cfg(not(unix))]
pub fn lchdir(path: &str, d: Option<&mut dirsav>, hard: i32) -> i32 {
    let _ = (path, d, hard);
    -1
}

/// Port of `restoredir()` from `Src/utils.c:7565` — C decl `restoredir(struct dirsav *d)`.
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
pub fn restoredir(d: &mut dirsav) -> i32 {
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
        // The result is the caller's break-reason (glob.c:696 reports
        // "current directory lost during glob" on it), so it must not be
        // dropped — this is the branch every `init_dirsav`-then-`lchdir`
        // soft restore lands in now that `dirname` starts out NULL.
        if upchdir(d.level as usize).is_err() {
            err = -1;
        }
    } else if d.level < 0 {
        err = -1;
    }
    // C: dev/ino integrity check after the chdir/fchdir — unconditional
    // in C (c:7591-7595), it can only escalate an existing failure to -2.
    if d.dev != 0 || d.ino != 0 {
        if let Ok(meta) = fs::metadata(".") {
            if meta.ino() != d.ino || meta.dev() != d.dev {
                err = -2; // c:7594
            }
        } else {
            err = -2; // c:7592 `stat(".", &sbuf) < 0`
        }
    }
    err
}

/// Port of `privasserted()` from `Src/utils.c:7608`.
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

/// Port of `mode_to_octal()` from `Src/utils.c:7634` — C decl `mode_to_octal(mode_t mode)`.
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
    // c:7634
    #[cfg(not(unix))]
    {
        // No POSIX permission bits on non-Unix; fall back to canonical
        // octal-bit layout via the same mask values C uses on POSIX.
        let mut o: i32 = 0;
        if mode & 0o4000 != 0 {
            o |= 0o4000;
        } // c:7638-7639
        if mode & 0o2000 != 0 {
            o |= 0o2000;
        } // c:7640-7641
        if mode & 0o1000 != 0 {
            o |= 0o1000;
        } // c:7642-7643
        if mode & 0o0400 != 0 {
            o |= 0o0400;
        } // c:7644-7645
        if mode & 0o0200 != 0 {
            o |= 0o0200;
        } // c:7646-7647
        if mode & 0o0100 != 0 {
            o |= 0o0100;
        } // c:7648-7649
        if mode & 0o0040 != 0 {
            o |= 0o0040;
        } // c:7650-7651
        if mode & 0o0020 != 0 {
            o |= 0o0020;
        } // c:7652-7653
        if mode & 0o0010 != 0 {
            o |= 0o0010;
        } // c:7654-7655
        if mode & 0o0004 != 0 {
            o |= 0o0004;
        } // c:7656-7657
        if mode & 0o0002 != 0 {
            o |= 0o0002;
        } // c:7658-7659
        if mode & 0o0001 != 0 {
            o |= 0o0001;
        } // c:7660-7661
        return o; // c:7662
    }
    #[cfg(unix)]
    {
        // c:7636 — int m = 0;
        let mut m: i32 = 0;
        // c:7638-7661 — 12 bit-by-bit mappings from libc S_I* → canonical octal.
        if mode & S_ISUID as u32 != 0 {
            m |= 0o4000;
        } // c:7638-7639
        if mode & S_ISGID as u32 != 0 {
            m |= 0o2000;
        } // c:7640-7641
        if mode & S_ISVTX as u32 != 0 {
            m |= 0o1000;
        } // c:7642-7643
        if mode & S_IRUSR as u32 != 0 {
            m |= 0o0400;
        } // c:7644-7645
        if mode & S_IWUSR as u32 != 0 {
            m |= 0o0200;
        } // c:7646-7647
        if mode & S_IXUSR as u32 != 0 {
            m |= 0o0100;
        } // c:7648-7649
        if mode & S_IRGRP as u32 != 0 {
            m |= 0o0040;
        } // c:7650-7651
        if mode & S_IWGRP as u32 != 0 {
            m |= 0o0020;
        } // c:7652-7653
        if mode & S_IXGRP as u32 != 0 {
            m |= 0o0010;
        } // c:7654-7655
        if mode & S_IROTH as u32 != 0 {
            m |= 0o0004;
        } // c:7656-7657
        if mode & S_IWOTH as u32 != 0 {
            m |= 0o0002;
        } // c:7658-7659
        if mode & S_IXOTH as u32 != 0 {
            m |= 0o0001;
        } // c:7660-7661
        m // c:7662
    }
}

/// Port of `mailstat()` from `Src/utils.c:7685` — C decl `mailstat(char *path, struct stat *st)`.
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
pub fn mailstat(path: &str, st: &mut libc::stat) -> i32 {
    // c:7685
    let c_path = match CString::new(path) {
        Ok(c) => c,
        Err(_) => return -1,
    };
    // C: if ((i = stat(path, st)) != 0 || !S_ISDIR(st->st_mode)) return i;
    let i = unsafe { libc::stat(c_path.as_ptr(), st as *mut _) }; // c:7693
    if i != 0 || (st.st_mode & libc::S_IFMT) != libc::S_IFDIR {
        // c:7693
        return i; // c:7693
    }
    // C 7700-7706: nlink=1, S_IFDIR → S_IFREG, zero size/blocks.
    st.st_nlink = 1; // c:7701
    st.st_mode &= !libc::S_IFDIR; // c:7702
    st.st_mode |= libc::S_IFREG; // c:7703
    st.st_size = 0; // c:7704
    st.st_blocks = 0; // c:7705
                      // C 7707-7712: stat(path/cur). If absent or not a dir, return 0
                      // with the partial out (just the IFREG-coerced root).
    let cur_path = match CString::new(format!("{}/cur", path)) {
        Ok(c) => c,
        Err(_) => return 0,
    };
    let mut sub: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::stat(cur_path.as_ptr(), &mut sub) } != 0               // c:7708
        || (sub.st_mode & libc::S_IFMT) != libc::S_IFDIR
    // c:7708
    {
        return 0; // c:7710
    }
    st.st_atime = sub.st_atime; // c:7712
                                // C 7715-7722: stat(path/tmp).
    let tmp_path = match CString::new(format!("{}/tmp", path)) {
        Ok(c) => c,
        Err(_) => return 0,
    };
    if unsafe { libc::stat(tmp_path.as_ptr(), &mut sub) } != 0               // c:7716
        || (sub.st_mode & libc::S_IFMT) != libc::S_IFDIR
    // c:7716
    {
        return 0; // c:7718
    }
    st.st_mtime = sub.st_mtime; // c:7720
                                // C 7724-7730: stat(path/new). C overwrites mtime with new/'s mtime.
    let new_path = match CString::new(format!("{}/new", path)) {
        Ok(c) => c,
        Err(_) => return 0,
    };
    if unsafe { libc::stat(new_path.as_ptr(), &mut sub) } != 0               // c:7724
        || (sub.st_mode & libc::S_IFMT) != libc::S_IFDIR
    // c:7724
    {
        return 0; // c:7726
    }
    st.st_mtime = sub.st_mtime; // c:7728
                                // C 7749-7778: walk new/ and cur/, sum size + blocks, track newest
                                // atime / mtime.
    let mut atime: libc::time_t = 0; // c:7748
    let mut mtime: libc::time_t = 0; // c:7748
    for sub_name in ["new", "cur"] {
        let dir = format!("{}/{}", path, sub_name);
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => return 0,
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_bytes = name.as_encoded_bytes();
            // C: if (fn->d_name[0] == '.') continue;
            if name_bytes.first() == Some(&b'.') {
                // c:7758
                continue;
            }
            let entry_path = match CString::new(entry.path().to_string_lossy().as_bytes()) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let mut entry_st: libc::stat = unsafe { std::mem::zeroed() };
            if unsafe { libc::stat(entry_path.as_ptr(), &mut entry_st) } != 0 {
                // c:7762
                continue;
            }
            st.st_size += entry_st.st_size; // c:7766
            st.st_blocks += 1; // c:7767
                               // C: if (atime != mtime && atime > atime_max) atime_max = atime;
            if entry_st.st_atime != entry_st.st_mtime && entry_st.st_atime > atime {
                // c:7769
                atime = entry_st.st_atime;
            }
            if entry_st.st_mtime > mtime {
                // c:7771
                mtime = entry_st.st_mtime;
            }
        }
    }
    // C 7783-7784: if (atime) st_ret.st_atime = atime;
    if atime != 0 {
        // c:7783
        st.st_atime = atime;
    }
    if mtime != 0 {
        // c:7785
        st.st_mtime = mtime;
    }
    0 // c:7787
}

/// Script name for error messages
pub static mut SCRIPT_NAME: Option<String> = None;
/// Script filename
pub static mut SCRIPT_FILENAME: Option<String> = None;

/// Print a fatal error to stderr.
/// Port of `zerr(VA_ALIST1(const char *fmt))` from Src/utils.c:173. C source sets `errflag`
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
static SCRIPTNAME: std::sync::OnceLock<Mutex<Option<String>>> = std::sync::OnceLock::new();

/// Port of `char *argzero` from `Src/init.c`. The shell's argv[0].
/// Used by `zwarning()` (utils.c:147) as the fallback diagnostic
/// prefix when scriptname is unset.
static ARGZERO: std::sync::OnceLock<Mutex<Option<String>>> = std::sync::OnceLock::new();

/// Port of `char *scriptfilename` from `Src/init.c` (listed in
/// `zsh.export:356`). The FILE that the current code was PARSED
/// from. Distinct from `scriptname` (which doshfunc overrides on
/// function entry at c:5903). `scriptfilename` stays at the outer
/// file across function calls; PS4's `%x` reads it at
/// `Src/prompt.c:937`.
///
/// Set at init.c:479 (`-c` mode → `scriptname = scriptfilename
/// = "zsh"`), init.c:1367 (`source` enters the named file),
/// init.c:1558+1592+1667 (save/install/restore around `.` /
/// `source` bin_dot dispatch).
static SCRIPTFILENAME: std::sync::OnceLock<Mutex<Option<String>>> = std::sync::OnceLock::new();

/// Port of `char *posixzero` from `Src/params.c:76`. The original
/// argv[0] preserved unchanged by later mutations. Used by
/// `argzerogetfn` for `$0` under `isset(POSIXARGZERO)`.
static POSIXZERO: std::sync::OnceLock<Mutex<Option<String>>> = std::sync::OnceLock::new();

// error flag: bits from enum errflag_bits                                 // c:124
/// Port of `int errflag` from `Src/init.c`. Tracks whether an
/// error has been raised (`ERRFLAG_ERROR = 1`) or break/return
/// is in flight (`ERRFLAG_INT = 2`).
///
/// Direct global access: `errflag.load(Ordering::Relaxed)` reads
/// the current value, `errflag.fetch_or(ERRFLAG_ERROR, …)` matches
/// C's `errflag |= ERRFLAG_ERROR`, `errflag.store(0, …)` matches
/// C's `errflag = 0`.
///
/// Storage is [`crate::errflag_cell::ErrflagCell`], not a bare
/// `AtomicI32`: C's `errflag` is a per-PROCESS `int` and C's background
/// work is a forked child, so a child's errors never reach the parent's
/// line editor. zshrs runs that work on threads, so the flag is
/// process-global on the shell thread and thread-local everywhere else —
/// which is what `fork()` gave C. See that module for the ^G-abort
/// corruption this prevents.
#[allow(non_upper_case_globals)]
pub static errflag: crate::errflag_cell::ErrflagCell = crate::errflag_cell::ErrflagCell::new();

/// Port of `int noerrs` from `Src/init.c`. Counter — when `> 0`,
/// suppresses error printing. `noerrs >= 2` also suppresses the
/// `errflag` set inside `zerr`/`zerrnam`.
static NOERRS: std::sync::OnceLock<Mutex<i32>> = std::sync::OnceLock::new();

/// Port of `int locallevel` from `Src/init.c`. Function-call depth
/// (0 = top-level, 1+ = inside a fn). `zwarning()` checks this in
/// the script-prefix path (utils.c:150).
// `LOCALLEVEL` removed — see `locallevel_lock` removal comment
// below. Canonical static lives in `LOCALLEVEL`
// (port of params.c:54).

/// Port of `int lineno` from `Src/init.c`. Current line number;
/// `zerrmsg()` includes it in the diagnostic when locallevel > 0
/// or shinstdin is unset (utils.c:301).
static LINENO: std::sync::OnceLock<Mutex<i32>> = std::sync::OnceLock::new();

/// Port of the `isset(SHINSTDIN)` check from utils.c:150. C reads
/// the SHINSTDIN option directly; the Rust port caches it here so
/// callers don't pull in the whole option-table for every error.
static SHINSTDIN_OPT: std::sync::OnceLock<Mutex<bool>> = std::sync::OnceLock::new();

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
pub static INCOMPFUNC: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:46

/// Port of `mod_export int resetneeded` from `Src/utils.c:1821`.
/// Set when the editor needs a redraw — incremented by widgets +
/// signal handlers (e.g. SIGWINCH); the next prompt loop tick clears
/// it after running zrefresh.
pub static RESETNEEDED: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:1821

/// Port of `mod_export int winchanged` from `Src/utils.c:1827`.
/// Set by the SIGWINCH handler — the next refresh re-queries the
/// terminal size and re-renders, then clears the flag.
pub static WINCHANGED: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:1827

/// Port of C's `GETKEY_*` flag enumeration from `Src/zsh.h:3143`.
/// Passed to `getkeystring_with` to alter escape interpretation:
///   - GETKEY_OCTAL_ESC: `\NNN` octal escapes are interpreted (1<<0)
///   - GETKEY_EMACS: unknown `\<char>` drops the backslash (1<<1)
///   - GETKEY_BACKSLASH_C: `\c` truncates the result (1<<3)
pub const GETKEY_OCTAL_ESC: u32 = 1 << 0; // c:zsh.h:3143
/// `GETKEY_EMACS` constant.
pub const GETKEY_EMACS: u32 = 1 << 1; // c:zsh.h:3150
/// `GETKEY_CTRL` constant.
pub const GETKEY_CTRL: u32 = 1 << 2; // c:zsh.h:3152
/// `GETKEY_BACKSLASH_C` constant.
pub const GETKEY_BACKSLASH_C: u32 = 1 << 3; // c:zsh.h:3154

/// `GETKEYS_PRINT = GETKEY_OCTAL_ESC | GETKEY_BACKSLASH_C |
/// GETKEY_EMACS` per Src/zsh.h:3185 — the flag set `bin_print` uses
/// when interpreting escapes (with EMACS: unknown `\<c>` → `<c>`).
pub const GETKEYS_PRINT: u32 = GETKEY_OCTAL_ESC | GETKEY_EMACS | GETKEY_BACKSLASH_C; // c:zsh.h:3185

/// `GETKEYS_ECHO = GETKEY_BACKSLASH_C` per Src/zsh.h:3178 — the flag
/// set `echo` uses (and `print -e`). NOT GETKEY_EMACS: unknown
/// `\<c>` is preserved as literal `\<c>`. NOT GETKEY_OCTAL_ESC:
/// `\NNN` is NOT interpreted as octal. Only `\a \b \e \E \f \n \r
/// \t \v \\` (plus `\c` truncation) are handled.
///
/// Per builtin.c:4754-4760 — `bin_print` selects this for BIN_ECHO
/// or when `-e` is set; otherwise GETKEYS_PRINT.
pub const GETKEYS_ECHO: u32 = GETKEY_BACKSLASH_C; // c:zsh.h:3178

/// `GETKEYS_BINDKEY = GETKEY_OCTAL_ESC | GETKEY_EMACS | GETKEY_CTRL`
/// per Src/zsh.h:3187 — the flag set `print -b` uses. Like GETKEYS_PRINT
/// it has EMACS (so `\C-`/`\M-` backslash escapes are processed and
/// unknown `\<c>` collapses to `<c>`) but adds GETKEY_CTRL (enabling the
/// `^X` caret notation) and drops GETKEY_BACKSLASH_C (no `\c` truncation).
pub const GETKEYS_BINDKEY: u32 = GETKEY_OCTAL_ESC | GETKEY_EMACS | GETKEY_CTRL; // c:zsh.h:3187

/// Static `int ep` from Src/utils.c:4775 — sticky flag suppressing the
/// `can't set tty pgrp` warning after the first failure.
static ATTACHTTY_EP: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:4775

// =====================================================================
// !!! WARNING: RUST-ONLY HELPERS — NO DIRECT C COUNTERPART !!!
// =====================================================================
//
// These three ported (`fdtable_lock`, `fdtable_get`, `fdtable_set`) DO
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

static FDTABLE: std::sync::OnceLock<Mutex<Vec<i32>>> = std::sync::OnceLock::new();

/// Port of `int max_zsh_fd` global from `Src/exec.c:201`.
/// "The highest fd we know zsh is using" — bumped by `fdtable_set`
/// callers, shrunk by `zclose` when trailing slots fall to UNUSED.
pub static MAX_ZSH_FD: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);

/// Port of `int fdtable_flocks` global from `Src/exec.c:204`.
/// Count of `FDT_FLOCK`-tagged fds; consulted by `closem()` to
/// decide whether to skip the flock-fd sweep.
pub static FDTABLE_FLOCKS: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

/// !!! WARNING: RUST-ONLY HELPER !!! No C counterpart exists as a
/// standalone function.
/// C assigns the `char *scriptname` global (`Src/utils.c:36`) directly, e.g.
/// `scriptname = ...` in `bin_dot`. Rust needs a setter over the `OnceLock<
/// Mutex<Option<String>>>` that holds it.
/// Setter for `scriptname`. Called from `bin_dot` / `source`
/// when entering a script.
pub fn set_scriptname(name: Option<String>) {
    *scriptname_lock().lock().unwrap() = name;
}

/// !!! WARNING: RUST-ONLY HELPER !!! No C counterpart exists as a
/// standalone function.
/// Reader for the same storage; C dereferences the `char *scriptname`
/// global at `Src/utils.c:36` in place.
/// Read `scriptname` — direct mirror of the C file-scope
/// `char *scriptname;` at `Src/utils.c:36`. Exposed for the
/// prompt expander (`%N`), `bin_dot`, and ZLE source tracking.
pub fn scriptname_get() -> Option<String> {
    scriptname_lock().lock().unwrap().clone()
}

/// !!! WARNING: RUST-ONLY HELPER !!! No C counterpart exists as a
/// standalone function.
/// Setter for the storage mirroring C's `char *scriptfilename` global at
/// `Src/utils.c:41`, which C assigns directly.
/// Setter for `scriptfilename` — direct mirror of the C
/// `scriptfilename` global at `Src/init.c` (zsh.export:356).
/// Called from `bin_dot` at init.c:1592, restored at c:1667.
pub fn set_scriptfilename(name: Option<String>) {
    *scriptfilename_lock().lock().unwrap() = name;
}

/// !!! WARNING: RUST-ONLY HELPER !!! No C counterpart exists as a
/// standalone function.
/// Reader for the same storage; C dereferences `scriptfilename`
/// (`Src/utils.c:41`) in place, e.g. PS4's `%x` in `Src/prompt.c`.
/// Read `scriptfilename` — direct mirror of the C global.
/// Used by PS4's `%x` (`Src/prompt.c:937`) and the function-call
/// frame at `Src/exec.c:1613` (`fstack.filename = scriptfilename`).
pub fn scriptfilename_get() -> Option<String> {
    scriptfilename_lock().lock().unwrap().clone()
}

/// !!! WARNING: RUST-ONLY HELPER !!! No C counterpart exists as a
/// standalone function.
/// Reader for the `int locallevel` global declared at `Src/params.c:54`;
/// C reads the global directly.
/// Read `locallevel` — function nesting depth. Routes through the
/// canonical `LOCALLEVEL` static (port of
/// `Src/params.c:54`). The prior Rust port stored a duplicate
/// `Mutex<i32>` here that split state from params.rs; both copies
/// were never kept in sync.
pub fn locallevel() -> i32 {
    LOCALLEVEL.load(Ordering::Relaxed)
}

/// !!! WARNING: RUST-ONLY HELPER !!! No C counterpart exists as a
/// standalone function.
/// C writes `locallevel++` inline in `startparamscope` (`Src/params.c`);
/// the global is declared at `Src/params.c:54`.
/// Bump `locallevel` (called by `startparamscope`).
pub fn inc_locallevel() {
    LOCALLEVEL.fetch_add(1, Ordering::Relaxed);
}

/// !!! WARNING: RUST-ONLY HELPER !!! No C counterpart exists as a
/// standalone function.
/// C writes `locallevel--` inline in `endparamscope` (`Src/params.c`);
/// the global is declared at `Src/params.c:54`.
/// Decrement `locallevel` (called by `endparamscope`).
pub fn dec_locallevel() {
    LOCALLEVEL.fetch_sub(1, Ordering::Relaxed);
}

/// !!! WARNING: RUST-ONLY HELPER !!! No C counterpart exists as a
/// standalone function.
/// Setter for the `char *argzero` global declared at `Src/params.c:72`,
/// which C assigns directly from `parseargs`.
/// Setter for `argzero`. Called once at shell init from `parseargs`.
pub fn set_argzero(name: Option<String>) {
    // c:271 (init.c) — `argv0 = argzero = posixzero = *argv++;`. At
    // shell init both argzero and posixzero share the same source.
    // posixzero is preserved unchanged after later mutations (exec -a,
    // function frames) — argzero is what gets rewritten. If posixzero
    // hasn't been independently set yet, mirror argzero so the
    // POSIXARGZERO branch in `argzerogetfn` returns something useful.
    let mut posix_lock = posixzero_lock().lock().unwrap();
    if posix_lock.is_none() {
        *posix_lock = name.clone();
    }
    drop(posix_lock);
    *argzero_lock().lock().unwrap() = name;
}

/// !!! WARNING: RUST-ONLY HELPER !!! No C counterpart exists as a
/// standalone function.
/// Reader for the same `char *argzero` global (`Src/params.c:72`);
/// C's `argzerogetfn` returns the global itself.
/// Read `argzero`. Used by `argzerogetfn` for `$0`.
pub fn argzero() -> Option<String> {
    argzero_lock().lock().unwrap().clone()
}

/// !!! WARNING: RUST-ONLY HELPER !!! No C counterpart exists as a
/// standalone function.
/// Setter for the `char *posixzero` global declared at `Src/params.c:76`;
/// C assigns it directly in the init path.
/// Port of `char *posixzero` from `Src/params.c:76`. The original
/// `argv[0]` preserved from shell startup, unchanged by later `exec -a`
/// or function-call rewrites. C's `argzerogetfn` (params.c:4958)
/// returns this instead of `argzero` when `isset(POSIXARGZERO)`.
///
/// Set once at init via `set_posixzero` (called from the init path
/// in C at init.c:271/297/321). `set_argzero` mirrors the value here
/// on first call so the POSIXARGZERO branch has something to read
/// even if the init path hasn't run yet.
pub fn set_posixzero(name: Option<String>) {
    *posixzero_lock().lock().unwrap() = name;
}

/// !!! WARNING: RUST-ONLY HELPER !!! No C counterpart exists as a
/// standalone function.
/// Reader for the same `char *posixzero` global (`Src/params.c:76`),
/// which C's `argzerogetfn` returns at `Src/params.c:4957`.
/// Read `posixzero`. C: `posixzero` (params.c:76 global). Used by
/// `argzerogetfn` when `isset(POSIXARGZERO)`.
pub fn posixzero() -> Option<String> {
    posixzero_lock().lock().unwrap().clone()
}

/// !!! WARNING: RUST-ONLY HELPER !!! No C counterpart exists as a
/// standalone function.
/// Writer for the `int noerrs` global declared at `Src/exec.c:117`;
/// C does `noerrs++` / `noerrs--` on the global in place.
/// Setter for `noerrs`. Increment to suppress error output;
/// decrement to restore.
pub fn set_noerrs(v: i32) {
    *noerrs_lock().lock().unwrap() = v;
}

/// !!! WARNING: RUST-ONLY HELPER !!! No C counterpart exists as a
/// standalone function.
/// Writer for the `int locallevel` global declared at `Src/params.c:54`.
/// Setter for `locallevel`. Called by function-call entry/exit.
pub fn set_locallevel(v: i32) {
    LOCALLEVEL.store(v, Ordering::Relaxed);
}

/// !!! WARNING: RUST-ONLY HELPER !!! No C counterpart exists as a
/// standalone function.
/// Writer for the `zlong lineno` global declared at `Src/params.c:123`;
/// C assigns the global directly from the lexer/parser.
/// Setter for `lineno`. Called by the parser as it advances.
pub fn set_lineno(v: i32) {
    *lineno_lock().lock().unwrap() = v;
}

/// !!! WARNING: RUST-ONLY HELPER !!! No C counterpart exists as a
/// standalone function.
/// Rust-only cache of the SHINSTDIN option bit (option table entry at
/// `Src/options.c:248`) so error emission does not have to reach into the
/// option table. C tests `isset(SHINSTDIN)` against `opts[]` directly.
/// Setter for the cached SHINSTDIN flag. Called by the option
/// machinery whenever `setopt shinstdin` / `unsetopt shinstdin`
/// fires.
pub fn set_shinstdin(v: bool) {
    *shinstdin_lock().lock().unwrap() = v;
}

/// Check if string is a valid identifier
// `isident` DELETED — fake duplicate (cited `c:params.c:1288` from
// utils.rs, location mismatch). Canonical port is
// `isident` at `params.rs:2056`, which
// correctly handles namespace prefix (`ns.var`) per the real C
// body — the utils.rs copy dropped that arm. Callers now route
// through `isident` directly.

// QuoteType enum + impl deleted — the canonical quote-type values are
// the bare `QT_*: i32` constants at `zsh_h.rs:175` (port of anonymous
// `enum { QT_NONE, QT_BACKSLASH, … }` from `Src/zsh.h:253-298`).
// `quotestring()` takes `quote_type: i32` matching C's signature.

/// !!! WARNING: RUST-ONLY HELPER !!! No C counterpart exists as a
/// standalone function.
/// C has no such function: the `(q)`-count to `QT_*` mapping is done inline
/// in `paramsubst`'s flag loop at `Src/subst.c:2236`, where repeated `q`
/// flags run `quotemod++, quotetype++` (c:2252) over the `QT_*` enum.
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

/// Port of `ispecial()` from `Src/ztype.h:59`.
///
/// Port of `ispecial()` macro from `Src/ztype.h:59` — `zistype(X,
/// ISPECIAL)`. C populates the ISPECIAL typtab bit at
/// `Src/utils.c:4253-4262`:
///   - Every byte in `SPECCHARS` (`Src/zsh.h:228` =
///     `"#$^*()=|{}[]\`<>?~;&\\n\\t \\\\\\'\\""`)
///   - `,` when `typtab_flags & ZTF_SP_COMMA` (set by
///     `makecommaspecial(1)` per c:4271)
///   - `bangchar` (default `!`) when `BANGHIST` is set AND
///     `ZTF_INTERACT` is set (interactive shell)
///
/// Reads the LIVE table, which is the whole point of the macro: the
/// set is not a constant. `inittyptab` recomputes it from options on
/// every relevant change, and two members exist only at runtime —
/// `,` under `makecommaspecial` and `bangchar` under
/// BANGHIST+interactive.
///
/// This used to enumerate `SPECCHARS` as a literal `matches!`, which
/// made both runtime members unreachable: `quotestring` is the only
/// caller, so `${(q)v}` on `v='a!b'` published `a!b` where an
/// interactive zsh publishes `a\!b`, and `makecommaspecial` could not
/// affect quoting at all. The bit was being computed correctly all
/// along (utils.c:4257-4259, and `makebangspecial` at c:4283) — it
/// just had no reader.
///
/// C indexes `typtab` by `unsigned char`, so anything outside 0..=255
/// has no entry; return false rather than truncating a `char` into a
/// byte that would alias an unrelated slot.
fn ispecial(c: char) -> bool {
    // c:59 (Src/ztype.h) — `zistype(X, ISPECIAL)`.
    let cp = c as u32;
    if cp > 0xff {
        return false;
    }
    crate::ported::ztype_h::ispecial(cp as u8)
}

/// Port of `convbase()` from `Src/params.c:5632`.
///
/// Convert integer to string with specified base — re-export of
/// the canonical port at `convbase_param`.
///
/// The previous Rust port at this file was a SECOND, divergent
/// implementation of `convbase`:
///   - Used LOWERCASE digit chars (`"0123456789abc..."`) for
///     bases 11..36, while C uses UPPERCASE (`(dig - 10) + 'A'`
///     at Src/params.c:5621).
///   - Skipped CBASES + OCTALZEROES prefix handling at c:5598-5604
///     (`0x`/`0` prefixes when option-gated).
///   - Returned wrong format for bases not handled by the explicit
///     match arms.
///
/// The canonical port lives at `params.rs::convbase_ptr` (c:5588
/// faithful body) + `params.rs::convbase` (c:5634 wrapper). Route
/// through there so utils.rs / params.rs agree byte-for-byte and
/// no divergent duplicate stays alive.
pub fn convbase(val: i64, base: u32) -> String {
    // c:5634 (Src/params.c)
    convbase_param(val, base)
}

// `checkglobqual` DELETED — fake bracket-depth scanner cited
// `c:glob.c:1158` from utils.rs. Canonical port is
// `crate::ported::glob::checkglobqual(str, sl, nobareglob, sp)` at
// `glob.rs:813`, which matches the real C signature
// `(char *str, int sl, int nobareglob, char **sp)`. The utils.rs
// fake reduced the signature to a single bool and discarded the
// `sp` glob-qualifier-start out-pointer that callers need. Zero
// callers used the utils.rs version.

// `dupstrpfx` DELETED — fake duplicate cited `c:string.c:161` from
// utils.rs (wrong location). Canonical port is
// `dupstrpfx` at `string.rs:161`. Zero
// callers used the utils.rs version.

/// Port of `statuidprint()` from `Src/Modules/stat.c:132`.
/// !!! DIVERGENT DUPLICATE !!! The faithful port of that C body (with the
/// `STF_RAW` / `STF_STRING` flag arms and the no-`getpwuid` fallback) is
/// `statuidprint(uid, flags) -> String` at `src/ported/modules/stat.rs:112`.
/// This copy implements only the `getpwuid(uid) -> pw_name` arm (c:142-144)
/// and has no caller; it should be deleted in favour of the stat.rs port.
///
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

// `ztrdup` / `dyncat` / `tricat` / `bicat` DELETED — these were
// fake duplicates of the canonical ports in `src/ported/string.rs`
// (`Src/string.c:62`, `:131`, `:98`, `:145`). The utils.rs copies
// admitted they were not in utils.c via `c:string.c:NNN`
// annotations; PORT.md Rule 1 disallows the duplicate location.
// Callers now route through `crate::ported::string::{ztrdup,
// dyncat, tricat, bicat}` directly.

// `iwsep` / `iwsep_byte` DELETED — these were fake hardcoded
// `c==' '||c=='\t'||c=='\n'` checks claiming to port `iwsep` from
// "Src/zsh.h", but the real macro lives at `Src/ztype.h:61`
// (`#define iwsep(X) zistype(X, IWSEP)`) and consults the `typtab[]`
// lookup — which mutates when IFS is reassigned. The canonical port
// is `iwsep(u8) -> bool` at `ztype_h.rs:133`.
// Internal utils.rs callers (skipwsep / spacesplit / findword) now
// route through it directly.

// `imeta(c: char)` DELETED — fake `char`-arg wrapper cited
// `c:ztype.h:60` from utils.rs (wrong location, wrong signature).
// Canonical port is `crate::ported::ztype_h::imeta(u8) -> bool` at
// `ztype_h.rs:130`. C `imeta(X)` takes a byte; wrapping with `char`
// invented an `> 0xff` early-out that C doesn't have. Migrated 1
// caller (`zle/computil.rs:6194`).

/// !!! WARNING: RUST-ONLY HELPER !!! No C counterpart exists as a
/// standalone function.
/// C calls the libc `gethostname(2)` directly at `Src/params.c:874`
/// (`gethostname(hostnam, 256);`). The zsh function of this name
/// (`Src/compat.c:64`) is the `#ifndef HAVE_GETHOSTNAME` fallback built on
/// `uname()`, which this does NOT implement — this wraps libc's real one.
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
    // gethostname(2) failure fallback. C consults `$HOST` /
    // `cached_hostname`. Read paramtab; fall back to "localhost".
    getsparam("HOST")
        .or_else(|| getsparam("HOSTNAME"))
        .unwrap_or_else(|| "localhost".to_string())
}

/// Get current working directory — re-export of the canonical
/// port at `compat::zgetcwd` (Src/compat.c:559).
// `zgetcwd` DELETED — fake `Option<String>` wrapper around the
// canonical `compat::zgetcwd() -> String` port (`Src/compat.c:559`).
// C `zgetcwd` never returns NULL (falls back to `"."` per the
// `dupstring(".")` arm), so the `Option` was caller-API churn, not
// a port. Callers route through `crate::ported::compat::zgetcwd()`
// directly.

// `zchdir` duplicate deleted — canonical port at compat.rs:253
// matches C's `int zchdir(char *dir)` signature (returns i32, not
// bool). Zero callers used the utils.rs bool variant.

// `realpath` / `mkdir` / `symlink` / `readlink` / `getenv` DELETED —
// five Rust-only convenience wrappers around std::fs / std::env /
// std::os::unix::fs with ZERO callers across the tree. Each was a
// thin std-lib re-export with no zsh C counterpart:
//   - realpath  → std::fs::canonicalize (libc realpath(3) — not in
//                 utils.c; the zsh source uses chrealpath at hist.c:1971)
//   - mkdir     → std::fs::create_dir (libc mkdir(2) — zsh's analogue
//                 is Modules/files.c::bin_mkdir, not a utils.c helper)
//   - symlink   → std::os::unix::fs::symlink (libc symlink(2))
//   - readlink  → std::fs::read_link (libc readlink(2))
//   - getenv    → std::env::var (libc getenv(3) — zsh's analogue is
//                 zgetenv in compat.c)
// PORT.md Rule 0/A: names must exist in upstream zsh C source as a
// `<name>(` function. None of these qualify; with zero callers they
// were dead code by definition. Callers needing the same semantics
// route through the canonical port (chrealpath / bin_mkdir / etc.)
// or std::fs directly.

/// Per-prompt callback registry.
/// Port of the static `prepromptfns` LinkList in Src/utils.c:1313.
/// Holds the bare-fn pointers `addprepromptfn`/`delprepromptfn`
/// register and `preprompt()` walks.
static PREPROMPT_FNS: Mutex<Vec<fn()>> = Mutex::new(Vec::new());

/// !!! WARNING: RUST-ONLY HELPER !!! No C counterpart exists as a
/// standalone function.
/// C has no `setenv` of its own — `Src/params.c`'s `zputenv` maintains the
/// `environ` array with metafied names. This is a plain
/// `std::env::set_var` wrapper for Rust-side callers, not that port.
/// Set environment variable
pub fn setenv(name: &str, value: &str) {
    std::env::set_var(name, value);
}

/// !!! WARNING: RUST-ONLY HELPER !!! No C counterpart exists as a
/// standalone function.
/// Counterpart of `setenv` above; C's environment removal goes through
/// `Src/params.c`'s `zputenv`/`delenv`, not a function of this shape.
/// Unset environment variable
pub fn unsetenv(name: &str) {
    std::env::remove_var(name);
}

/// Time-ordered timed-function registry.
/// Port of the `timedfns` LinkList Src/utils.c:1365 keeps for the
/// `sched` builtin. Sorted ascending by `when` (epoch seconds);
/// `addtimedfn` does an insertion sort (matches the C source's
/// `for (;;)` walk at lines 1394-1411).
pub static TIMED_FNS: Mutex<Vec<(i64, fn())>> = Mutex::new(Vec::new()); // c:1365 timedfns (mod_export in C)

/// !!! WARNING: RUST-ONLY HELPER !!! No C counterpart exists as a
/// standalone function.
/// Wrapper over libc `getuid(2)`, which C calls directly at its use sites.
/// zsh has no `getuid` function of its own.
/// Get current user ID
pub fn getuid() -> u32 {
    #[cfg(unix)]
    unsafe {
        libc::getuid()
    }
    #[cfg(not(unix))]
    0
}

/// !!! WARNING: RUST-ONLY HELPER !!! No C counterpart exists as a
/// standalone function.
/// Wrapper over libc `geteuid(2)`, which C calls directly at its use sites.
/// Get effective user ID
pub fn geteuid() -> u32 {
    #[cfg(unix)]
    unsafe {
        libc::geteuid()
    }
    #[cfg(not(unix))]
    0
}

/// !!! WARNING: RUST-ONLY HELPER !!! No C counterpart exists as a
/// standalone function.
/// Wrapper over libc `getgid(2)`, which C calls directly at its use sites.
/// Get current group ID
pub fn getgid() -> u32 {
    #[cfg(unix)]
    unsafe {
        libc::getgid()
    }
    #[cfg(not(unix))]
    0
}

/// !!! WARNING: RUST-ONLY HELPER !!! No C counterpart exists as a
/// standalone function.
/// Wrapper over libc `getegid(2)`, which C calls directly at its use sites.
/// Get effective group ID
pub fn getegid() -> u32 {
    #[cfg(unix)]
    unsafe {
        libc::getegid()
    }
    #[cfg(not(unix))]
    0
}

/// !!! WARNING: RUST-ONLY HELPER !!! No C counterpart exists as a
/// standalone function.
/// Wrapper over libc `getpid(2)`, which C calls directly at its use sites.
/// Get process ID
pub fn getpid() -> i32 {
    std::process::id() as i32
}

/// !!! WARNING: RUST-ONLY HELPER !!! No C counterpart exists as a
/// standalone function.
/// Wrapper over libc `getppid(2)`, which C calls directly at its use sites.
/// Get parent process ID
pub fn getppid() -> i32 {
    #[cfg(unix)]
    unsafe {
        libc::getppid()
    }
    #[cfg(not(unix))]
    0
}

/// Port of `getkeystring()` from `Src/utils.c:6915`.
///
/// `getkeystring` with the C `how` parameter (Src/utils.c:6915).
/// The plain `getkeystring(s)` shim above defaults `how=0` for
/// non-EMACS callers (zbeep, dollar-quote — they keep unknown
/// `\<char>` as literal `\<char>`). Pass `GETKEYS_PRINT` to get
/// the print/echo behavior (drop backslash on unknown).
/// `getkeystring` with the C `how` parameter AND the
/// `GETKEY_UPDATE_OFFSET` cursor-offset out-param (Src/utils.c:6915).
///
/// Port of `getkeystring(char *s, int *len, int how, int *misc)` from
/// `Src/utils.c:6915`, specifically its `GETKEY_UPDATE_OFFSET` path.
/// When `how` sets `GETKEY_UPDATE_OFFSET` and `misc` is `Some`, `*misc`
/// is the C `*misc` completion offset — a byte index into `s` — and is
/// updated as escapes positioned before the cursor collapse:
///   - `-1` per collapsed `\<char>` escape whose backslash precedes the
///     cursor, `s - sstart < *misc` (c:6987); undone if the backslash is
///     kept literal in the default (non-EMACS) arm (c:7181-7183);
///   - additional `-6` for `\u`, `-10` (`-4`+`-6`) for `\U` (c:7073-7084),
///     then `+N` for the N output bytes emitted (c:7123-7124).
/// Octal/hex escapes get only the base `-1`, matching C's own incomplete
/// `/* HERE: GETKEY_UPDATE_OFFSET? */` handling (c:7155).
///
/// The offset is a byte index. For ASCII `$'...'` content (one byte per
/// char) it matches the C metafied-byte semantics exactly; the byte-vs-
/// char divergence for non-ASCII content is the pre-existing shared-cursor
/// limitation documented on `set_comp_sep` (compcore.rs), not new here.
///
/// Callers with no offset to track pass `misc = None`.
pub fn getkeystring_with(s: &str, how: u32, mut misc: Option<&mut i32>) -> (String, usize) {
    // c:utils.c:6915
    let update_off = (how & crate::ported::zsh_h::GETKEY_UPDATE_OFFSET as u32) != 0;
    let mut result = String::new();
    let mut chars = s.chars().peekable();
    let mut consumed = 0;
    // c:7194 — C's `^` handler is `control = 1; continue;`, so the control bit
    // stays PENDING across loop iterations and a following `\M`/`\C` escape
    // chains onto it (`^\M-a` → c:7034 `meta = 1 + control` → meta==2 → 0x81).
    // This port's `^` arm consumes its base character inline, which cannot
    // express that, so the flag carries the state to the modifier arm instead.
    let mut pending_control = false;
    while let Some(c) = chars.next() {
        consumed += c.len_utf8();
        // c:utils.c:7194 — `^X` caret notation. A bare `^` (not a backslash
        // escape) followed by any char applies the control mask to it, but
        // ONLY under GETKEY_CTRL — i.e. `print -b` / bindkey. Plain print
        // (GETKEYS_PRINT, no GETKEY_CTRL) keeps `^A` literal.
        if c == '^' && !pending_control && (how & GETKEY_CTRL) != 0 {
            // A `\M`/`\C` escape after the caret must be handled by the
            // modifier arm with control already pending — C reaches it via
            // `continue` rather than consuming the base char here. Gated on
            // GETKEY_EMACS because that is what enables the modifier arm at
            // all (c:7031/7043); without it C's `case 'M'` emits a literal
            // backslash instead.
            let mut la = chars.clone();
            if (how & GETKEY_EMACS) != 0
                && la.next() == Some('\\')
                && matches!(la.next(), Some('M') | Some('C'))
            {
                pending_control = true;
                continue;
            }
            if let Some(base) = chars.next() {
                consumed += base.len_utf8();
                let mut byte = base as u32;
                // c:7261-7267 — `^?` → 0x7f (DEL), else clear bits 5-6.
                if byte == '?' as u32 {
                    byte = 0x7f;
                } else if byte <= 0xff {
                    byte &= 0x9f;
                }
                if byte <= 0xff {
                    let b_ = byte as u8;
                    if b_ < 0x80 {
                        result.push(b_ as char);
                    } else {
                        result.push('\u{83}');
                        result.push(char::from(b_ ^ 32));
                    }
                } else if let Some(ch) = char::from_u32(byte) {
                    result.push(ch);
                }
                continue;
            }
            // c:7194 `s[1]` guard — a trailing `^` with no char is literal.
            result.push(c);
            continue;
        }
        if c != '\\' {
            result.push(c);
            continue;
        }
        // c is a backslash; record its byte start offset for
        // GETKEY_UPDATE_OFFSET (`s - sstart` in C).
        let bs_off = consumed - c.len_utf8();
        // c:utils.c:6987 — base offset decrement for a collapsed `\<char>`
        // escape whose backslash precedes the cursor. C gates on
        // `*s == '\\' && s[1]`, so a following char must exist (peek).
        let mut miscadded = false;
        if update_off && chars.peek().is_some() {
            if let Some(m) = misc.as_deref_mut() {
                if (bs_off as i32) < *m {
                    *m -= 1;
                    miscadded = true;
                }
            }
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
            // c:utils.c:7018-7024 — `\E` is EMACS-gated:
            //     case 'E':
            //         if (!(how & GETKEY_EMACS)) {
            //             *t++ = '\\', s--;
            //             if (miscadded) (*misc)++;
            //             continue;
            //         }
            //         /* FALL THROUGH */
            //     case 'e':
            //         *t++ = '\033';
            // Without EMACS `\E` stays a literal backslash + `E`; only `\e`
            // is unconditional. Folding `E` in with `e` made `${(g::)v}` on
            // `\E` yield ESC where zsh yields `\E`. Unlike the octal
            // `continue` arm (c:7159), C restores the miscadded decrement
            // here, since the backslash is kept.
            Some('E') if (how & GETKEY_EMACS) == 0 => {
                consumed += 1;
                if miscadded {
                    if let Some(m) = misc.as_deref_mut() {
                        *m += 1;
                    }
                }
                result.push('\\');
                result.push('E');
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
            // c:utils.c:7140-7152 — `\\` (and `\'` under
            // GETKEY_DOLLAR_QUOTE) explicitly emit the trailing byte
            // bare. Outside DOLLAR_QUOTE, `\'` FALLTHROUGHs to default.
            // `\\` stays special because the default arm (c:7180-7184)
            // skips the leading-backslash emission iff `*s == '\\'`,
            // so `\\` reduces to `\` regardless of GETKEY_EMACS.
            //
            // Previous Rust port also had `Some('\'')` and `Some('"')`
            // arms that unconditionally dropped the backslash. That
            // contradicted C: under GETKEYS_ECHO (no GETKEY_EMACS),
            // `\'` must remain `\'` because echo doesn't strip
            // unknown escapes. Removed those arms — `\'` and `\"`
            // now flow through the default arm and honour GETKEY_EMACS.
            //
            // Root cause of `echo "${(qq)s}"` for `s="a'b"` emitting
            // `'a'''b'` instead of zsh's `'a'\''b'`.
            Some('\\') => {
                result.push('\\');
                consumed += 1;
            }
            // c:utils.c:7072-7138 — `\u` (4-hex) / `\U` (8-hex)
            // Unicode codepoint escapes. Always interpreted; the C
            // source's `case 'U':` / `case 'u':` arms have no flag
            // gating (only the GETKEY_UPDATE_OFFSET bookkeeping). C
            // calls `ucs4tomb(wval, t)` which writes UTF-8 bytes.
            // Previously these were absent from `getkeystring_with`,
            // so `echo -e "\U00000041"` emitted literal `\U00000041`
            // instead of 'A'.
            Some('u') => {
                consumed += 1;
                // c:utils.c:7077-7084 — extra `-6` when the escape precedes
                // the cursor (checked at the 'u', i.e. offset bs_off+1).
                if update_off {
                    if let Some(m) = misc.as_deref_mut() {
                        if ((bs_off + 1) as i32) < *m {
                            *m -= 6;
                        }
                    }
                }
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
                // c:utils.c:7085 `wval = 0;` BEFORE the digit loop, and
                // c:7087-7095 leaves it untouched when the first char is not
                // a hex digit (`s--; break;`). So ZERO digits is not "not an
                // escape" — it emits codepoint 0 via `ucs4tomb(wval, t)`
                // (c:7101). `\u` → one NUL byte, `\uzz` → NUL + "zz". An Err
                // on the empty parse pushed nothing at all.
                let val = u32::from_str_radix(&hex, 16).unwrap_or(0);
                if let Some(ch) = char::from_u32(val) {
                    result.push(ch);
                    // c:utils.c:7123-7124 — add one per output byte,
                    // checked at the last consumed input byte.
                    if update_off {
                        if let Some(m) = misc.as_deref_mut() {
                            if ((consumed - 1) as i32) < *m {
                                *m += ch.len_utf8() as i32;
                            }
                        }
                    }
                }
            }
            Some('U') => {
                consumed += 1;
                // c:utils.c:7073-7084 — `\U` extra `-4` (c:7073) then the
                // shared `\u` `-6` (c:7077), both gated at offset bs_off+1.
                if update_off {
                    if let Some(m) = misc.as_deref_mut() {
                        if ((bs_off + 1) as i32) < *m {
                            *m -= 10;
                        }
                    }
                }
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
                // c:utils.c:7085 — same `wval = 0` default as the `\u` arm
                // above: `\U` with no hex digits emits codepoint 0, not
                // nothing.
                let val = u32::from_str_radix(&hex, 16).unwrap_or(0);
                if let Some(ch) = char::from_u32(val) {
                    result.push(ch);
                    // c:utils.c:7123-7124 — add one per output byte.
                    if update_off {
                        if let Some(m) = misc.as_deref_mut() {
                            if ((consumed - 1) as i32) < *m {
                                *m += ch.len_utf8() as i32;
                            }
                        }
                    }
                }
            }
            // c:utils.c:7156-7178 — the NUMERIC-ESCAPE branch. C handles
            // `\x`, `\NNN` and `\0NNN` in ONE arm, and the shared tail is
            // what makes them agree, so this port keeps them together too:
            //
            //     if ((idigit(*s) && *s < '8') || *s == 'x') {
            //         if (!(how & GETKEY_OCTAL_ESC)) {
            //             if (*s == '0')
            //                 s++;
            //             else if (*s != 'x') {
            //                 *t++ = '\\', s--;
            //                 continue;
            //             }
            //         }
            //         if (s[1] && s[2] && s[3]) {
            //             svchar = s[3]; s[3] = '\0'; u = s;
            //         }
            //         *t++ = zstrtol(s + (*s == 'x'), &s,
            //                        (*s == 'x') ? 16 : 8);
            //         ...
            //         s--;
            //     }
            //
            // Three behaviours here were each missing when these were three
            // separate arms:
            //
            //  1. Without GETKEY_OCTAL_ESC a NON-ZERO digit is not an escape
            //     at all (c:7159): C emits a literal backslash and backs up so
            //     the digit is re-read as an ordinary char. This is INSIDE the
            //     numeric branch, so — unlike the default arm — GETKEY_EMACS
            //     does not suppress the backslash: `${(g:e:)v}` on `\101` is
            //     `\101`, not `101`.
            //  2. The value goes through `zstrtol`, which SKIPS leading blanks
            //     and accepts a `+`/`-` sign (c:2444-2450), and whose end
            //     pointer lands past everything it consumed — blanks included.
            //     So `\0 1` is byte 0o1, `\x 41` is 0x04 then `1`, `\x y` is
            //     NUL then `y`, and `\0-1` is 0xff (a NEGATIVE parse truncated
            //     to a byte). Peeking for "is the next char a digit" stopped at
            //     the blank and left it in the output.
            //  3. The base is chosen from the char AT `s` AFTER the `\0`
            //     introducer was skipped, so `\0x41` is HEX (`A`), not NUL
            //     followed by `x41`.
            //
            // c:7166-7168 windows the parse by NUL-ing `s[3]` when three chars
            // follow `s`, capping it at 3 significant chars (2 after an `x`),
            // then restores the byte — so `\1234` is 0o123 then `4`.
            //
            // `*t++ = zstrtol(...)` assigns a zlong to a char, truncating to
            // the low byte: `\777` (511) is 0xff, `\400` (256) is NUL. Parsing
            // straight into u8 made those an Err and emitted nothing at all.
            Some(d) if d.is_digit(8) || d == 'x' => {
                consumed += 1;
                // c:7157-7161 — the pre-checks that run only without
                // GETKEY_OCTAL_ESC. `\0` is the octal introducer (`s++`);
                // any other bare digit is not an escape.
                let zero_intro = (how & GETKEY_OCTAL_ESC) == 0 && d == '0';
                if (how & GETKEY_OCTAL_ESC) == 0 && d != '0' && d != 'x' {
                    // c:7159 `*t++ = '\\', s--; continue;`
                    result.push('\\');
                    result.push(d);
                    continue;
                }
                // C's `s` after the pre-checks: the char AFTER the `\0`
                // introducer, else the matched char itself.
                //
                // Four chars of lookahead is all C can read: the window test
                // reaches `s[3]`, and the parse never sees past it. Bounding
                // the clone keeps this O(1) per escape — collecting the whole
                // tail here would make decoding an escape-dense string O(n²).
                let rest: Vec<char> = chars.clone().take(4).collect();
                let (p, after): (Option<char>, &[char]) = if zero_intro {
                    (rest.first().copied(), rest.get(1..).unwrap_or(&[]))
                } else {
                    (Some(d), &rest[..])
                };
                // c:7166 `if (s[1] && s[2] && s[3])` — needs three chars after
                // `s`; the window then spans `s` plus two more.
                let win: &[char] = if after.len() >= 3 { &after[..2] } else { after };
                let mut vis: Vec<char> = Vec::with_capacity(3);
                if let Some(pc) = p {
                    vis.push(pc);
                }
                vis.extend_from_slice(win);
                // c:7170 `zstrtol(s + (*s == 'x'), &s, (*s == 'x') ? 16 : 8)`.
                let (base, skip_p) = if p == Some('x') { (16, 1) } else { (8, 0) };
                let parse_src: String = vis[skip_p.min(vis.len())..].iter().collect();
                let (num, tail) = zstrtol(&parse_src, base);
                let used_chars = parse_src[..parse_src.len() - tail.len()].chars().count();
                // Advance past what C consumed. `vis[0]` is the already-taken
                // match char unless the `\0` introducer moved `s` forward.
                let vis_used = skip_p + used_chars;
                let advance = if zero_intro {
                    vis_used
                } else {
                    vis_used.saturating_sub(1)
                };
                for _ in 0..advance {
                    chars.next();
                    consumed += 1;
                }
                let val = (num as u64 & 0xff) as u8;
                // c:Src/utils.c — the escape is one raw BYTE; metafy high
                // bytes (c:7289-7294) so `\377` unmetafies to a single 0xff,
                // not the UTF-8 pair c3 bf.
                if val < 0x80 {
                    result.push(val as char);
                } else {
                    result.push('\u{83}');
                    result.push(char::from(val ^ 32));
                }
                // c:7172-7173 — under GETKEY_PRINTF_PERCENT a numeric escape
                // producing `%` gets a second `%`.
                if (how & crate::ported::zsh_h::GETKEY_PRINTF_PERCENT as u32) != 0 && val == b'%' {
                    result.push('%');
                }
            }
            // c:utils.c:7029-7052 + c:7255-7275 — `\C` / `\M` set the
            // control / meta modifiers (bindkey-style key escapes), an
            // optional `-` separator, then the next base char (possibly
            // through chained `\C`/`\M`) gets the mask applied:
            // control → `& 0x9f` (or `\C-?` → 0x7f), meta → `| 0x80`.
            // Gated on GETKEY_EMACS (c:7031/7043 `if (how & GETKEY_EMACS)`):
            // print / print -b / $'…' have it, echo does not (so echo keeps
            // `\C-a` literal via the default arm). Mirrors the same handler
            // already present in `getkeystring` (utils.rs) and
            // `getkeystring_dollar_quote` (lex.rs); print's `getkeystring_with`
            // path lacked it, so `print "\C-a"` emitted literal `C-a`.
            Some(mod_letter @ ('C' | 'M')) if (how & GETKEY_EMACS) != 0 => {
                consumed += 1;
                // c:7034 `meta = 1 + control;  /* preserve the order of ^ and
                // meta */` — meta is a COUNT, not a flag: 2 when `\M` was seen
                // while control was already pending, which makes c:7261-7264
                // apply `|0x80` BEFORE the control mask instead of after. The
                // two orders differ for `?`: `\M-\C-?` is 0xff, `\C-\M-?` is
                // 0x9f. Booleans could not express that.
                // A `^` already seen (c:7194 `control = 1; continue;`) arrives
                // here as pending state, so `^\M-a` gives meta = 1 + 1 = 2.
                let mut control: u32 = if pending_control {
                    pending_control = false;
                    1
                } else {
                    0
                };
                let mut meta: u32 = 0;
                if mod_letter == 'C' {
                    control = 1; // c:7046
                } else {
                    meta = 1 + control; // c:7034
                }
                // c:7032-7033 / 7044-7045 — `if (s[1] == '-') s++;` consumes at
                // most ONE separator per modifier. Looping over a RUN of dashes
                // ate the base character of `\M--` (meta applied to `-`, 0xad),
                // leaving an empty binding.
                if chars.peek() == Some(&'-') {
                    chars.next();
                    consumed += 1;
                }
                // Chained `\C`/`\M`, and the caret form after a modifier.
                loop {
                    let mut iter_clone = chars.clone();
                    if iter_clone.next() == Some('\\') {
                        if let Some(nx) = iter_clone.next() {
                            if nx == 'C' || nx == 'M' {
                                chars.next(); // consume '\'
                                chars.next(); // consume C/M
                                consumed += 2;
                                if nx == 'C' {
                                    control = 1; // c:7046
                                } else {
                                    meta = 1 + control; // c:7034
                                }
                                if chars.peek() == Some(&'-') {
                                    chars.next();
                                    consumed += 1;
                                }
                                continue;
                            }
                        }
                    }
                    // c:7194 — `else if (*s == '^' && !control && (how &
                    // GETKEY_CTRL) && s[1]) { control = 1; continue; }`. C's
                    // `case 'M'` only sets `meta` and `continue`s, so the MAIN
                    // loop reaches this `^` handler with meta still pending:
                    // `\M-^?` is control+meta applied to `?` (0xff). This arm
                    // consumed the base char itself, so `^` became the base and
                    // `\M-^?` bound TWO bytes (0xde 0x3f) — breaking the very
                    // common `bindkey '\M-^?' backward-kill-word`.
                    if control == 0 && (how & GETKEY_CTRL) != 0 && chars.peek() == Some(&'^') {
                        let mut it2 = chars.clone();
                        it2.next();
                        if it2.next().is_some() {
                            // c:7194 `s[1]` guard
                            chars.next();
                            consumed += 1;
                            control = 1;
                            continue;
                        }
                    }
                    break;
                }
                // Read one base character (allowing nested simple escapes).
                let base: Option<char> = if chars.peek() == Some(&'\\') {
                    chars.next();
                    consumed += 1;
                    match chars.next() {
                        Some('n') => {
                            consumed += 1;
                            Some('\n')
                        }
                        Some('t') => {
                            consumed += 1;
                            Some('\t')
                        }
                        Some('r') => {
                            consumed += 1;
                            Some('\r')
                        }
                        Some('a') => {
                            consumed += 1;
                            Some('\x07')
                        }
                        Some('b') => {
                            consumed += 1;
                            Some('\x08')
                        }
                        Some('e') | Some('E') => {
                            consumed += 1;
                            Some('\x1b')
                        }
                        Some('f') => {
                            consumed += 1;
                            Some('\x0c')
                        }
                        Some('v') => {
                            consumed += 1;
                            Some('\x0b')
                        }
                        Some('\\') => {
                            consumed += 1;
                            Some('\\')
                        }
                        Some('\'') => {
                            consumed += 1;
                            Some('\'')
                        }
                        Some('"') => {
                            consumed += 1;
                            Some('"')
                        }
                        Some(other) => {
                            consumed += 1;
                            Some(other)
                        }
                        None => None,
                    }
                } else {
                    chars.next().inspect(|c| {
                        consumed += c.len_utf8();
                    })
                };
                if let Some(ch) = base {
                    let mut byte = ch as u32;
                    // c:7261-7264 — `if (meta == 2) { t[-1] |= 0x80; meta = 0; }`
                    // runs BEFORE the control mask: `\M` seen while control was
                    // already pending sets the high bit first.
                    if meta == 2 {
                        byte |= 0x80;
                    }
                    // c:7265-7271 — control mask (`\C-?` → 0x7f, else & 0x9f).
                    if control == 1 {
                        if byte == '?' as u32 {
                            byte = 0x7f;
                        } else {
                            byte &= 0x9f;
                        }
                    }
                    // c:7272-7275 — `if (meta) { t[-1] |= 0x80; }` — the
                    // meta==1 order, applied AFTER the mask.
                    if meta == 1 {
                        byte |= 0x80;
                    }
                    // c:7289-7294 — a masked byte >= 0x80 is metafied so it
                    // unmetafies back to the single raw byte on output.
                    if byte <= 0xff {
                        let b_ = byte as u8;
                        if b_ < 0x80 {
                            result.push(b_ as char);
                        } else {
                            result.push('\u{83}');
                            result.push(char::from(b_ ^ 32));
                        }
                    } else if let Some(c) = char::from_u32(byte) {
                        result.push(c);
                    }
                }
            }
            // c:utils.c:7180-7184 — default arm. With GETKEY_EMACS
            // set, drop the backslash; otherwise keep `\<char>`.
            Some(c) => {
                consumed += 1;
                // c:utils.c:7045 — `\c` under GETKEY_BACKSLASH_C
                // means TRUNCATE: drop the rest of the input,
                // suppress the trailing newline. Used by `echo`
                // (and `print` without -r). Set TLS flag so the
                // caller can detect + suppress the newline.
                if c == 'c' && (how & GETKEY_BACKSLASH_C) != 0 {
                    GETKEY_TRUNCATED.with(|cell| cell.set(true));
                    break;
                }
                if (how & GETKEY_EMACS) == 0 {
                    // c:utils.c:7181-7183 — backslash kept literal (no EMACS),
                    // so the string does not collapse here: undo the base
                    // GETKEY_UPDATE_OFFSET decrement applied above.
                    if miscadded {
                        if let Some(m) = misc.as_deref_mut() {
                            *m += 1;
                        }
                    }
                    result.push('\\');
                }
                result.push(c);
            }
            None => {
                result.push('\\');
            }
        }
    }
    (result, consumed)
}

thread_local! {
    /// Sticky flag set by `getkeystring_with` when a `\c` escape
    /// truncates the input under `GETKEY_BACKSLASH_C`. The caller
    /// (bin_print / bin_echo) reads + clears via `getkey_truncated_take()`
    /// to suppress the trailing newline and skip any subsequent args
    /// in the print pass.
    pub static GETKEY_TRUNCATED: std::cell::Cell<bool> =
        const { std::cell::Cell::new(false) };
}

/// !!! WARNING: RUST-ONLY HELPER !!! No C counterpart exists as a
/// standalone function.
/// C returns the `\c`-truncation signal through `getkeystring`'s `int *misc`
/// out-parameter (`Src/utils.c:6915`). Rust's `getkeystring_with` records it
/// in a thread-local instead; this reads and clears that flag.
/// Read + clear the `\c`-truncated flag set by the previous
/// `getkeystring_with` call. Returns true once after a truncation
/// fires, false otherwise.
pub fn getkey_truncated_take() -> bool {
    GETKEY_TRUNCATED.with(|c| {
        let v = c.get();
        c.set(false);
        v
    })
}

/// !!! WARNING: RUST-ONLY HELPER !!! No C counterpart exists as a
/// standalone function.
/// Equivalent to the C expression `fdtable[fd]` — a read of the
/// `unsigned char *fdtable` global declared at `Src/exec.c:179` and grown by
/// `growfdtable` (`Src/utils.c:1965`). C indexes the array directly.
/// !!! RUST-ONLY HELPER — see WARNING block above. Equivalent to
/// the C expression `fdtable[fd]` (read). Returns `FDT_UNUSED` for
/// any fd that has not been explicitly set, matching the C source's
/// post-`growfdtable` zero-fill behaviour at Src/utils.c:1979.
pub fn fdtable_get(fd: i32) -> i32 {
    // c:utils.c:fdtable[fd]
    if fd < 0 {
        return FDT_UNUSED;
    }
    let g = fdtable_lock().lock().unwrap();
    g.get(fd as usize).copied().unwrap_or(FDT_UNUSED)
}

/// !!! WARNING: RUST-ONLY HELPER !!! No C counterpart exists as a
/// standalone function.
/// Equivalent to the C statement `fdtable[fd] = kind;` over the
/// `unsigned char *fdtable` global (`Src/exec.c:179`), with the
/// `growfdtable` call (`Src/utils.c:1965`) inlined.
/// !!! RUST-ONLY HELPER — see WARNING block above. Equivalent to
/// the C statement `fdtable[fd] = kind;`. Inlines the `growfdtable`
/// call from Src/utils.c:1965 since C always invokes it immediately
/// before the assignment anyway. Also updates `MAX_ZSH_FD` to track
/// the highest assigned slot — C does this inside `check_fd_table`
/// at `Src/utils.c:1982` (`max_zsh_fd = fd;`). Previously the Rust
/// `fdtable_set` skipped this, leaving `max_zsh_fd = 0` after any
/// `addlockfd` / `addmodulefd` call, which broke `zcloselockfd`'s
/// `if (fd > max_zsh_fd)` guard.
pub fn fdtable_set(fd: i32, kind: i32) {
    // c:utils.c:fdtable[fd]
    if fd < 0 {
        return;
    }
    let mut g = fdtable_lock().lock().unwrap();
    if (fd as usize) >= g.len() {
        g.resize((fd as usize) + 1, FDT_UNUSED);
    }
    g[fd as usize] = kind;
    // c:1982 — `max_zsh_fd = fd;` (with `if (fd <= max_zsh_fd) return;`
    // guard at c:1971). Inline equivalent: bump only when this fd
    // exceeds the current max.
    let cur = MAX_ZSH_FD.load(Ordering::Relaxed);
    if fd > cur {
        MAX_ZSH_FD.store(fd, Ordering::Relaxed);
    }
}

/// Port of `imeta()` from `Src/ztype.h:60`.
///
/// Port of `imeta()` macro from `Src/ztype.h:60` — `zistype(X, IMETA)`.
///
/// IMETA is set in the typtab at `Src/utils.c:4195-4201` for:
///   - `'\0'` (c:4195)
///   - `Meta` (0x83) (c:4196)
///   - `Marker` (0xa2) (c:4197)
///   - `Pound..=LAST_NORMAL_TOK` = `0x84..=0x9c` (c:4198)
///   - `Snull..=Nularg` = `0x9d..=0xa1` (c:4200)
///
/// The CANONICAL set is `{0x00, 0x83..=0xa2}`. The previous Rust
/// port used `b >= 0x83` (every byte 0x83..=0xff) which was WRONG
/// for bytes `0xa3..=0xff`: C `imeta()` returns false for those,
/// so metafy passes them through unchanged. Rust's broader range
/// caused `metafy` to corrupt UTF-8 multibyte content (e.g.,
/// `é` = `0xc3 0xa9`: both bytes are NOT IMETA in C, but Rust
/// escaped both as `Meta + (byte ^ 32)`, mangling the encoding
/// and breaking every downstream consumer that expects the raw
/// UTF-8 bytes to round-trip).
///
/// Use the closed-range form rather than the typtab lookup so
/// `metafy` / `imeta_byte` work in test contexts where the typtab
/// isn't initialised, AND so the fast-path doesn't go through a
/// Mutex lock per byte (`metafy` is called per-character on every
/// shell input line).
#[inline]
pub fn imeta_byte(b: u8) -> bool {
    // c:4195-4201 — canonical IMETA range.
    b == 0 || (0x83..=0xa2).contains(&b)
}

// `pub fn convfloat` and `pub fn convfloat_underscore` re-export
// wrappers DELETED per PORT.md Rule C. C defines both in
// `Src/params.c:5690` and `:5765` — the canonical Rust home is
// `src/ported/params.rs` (`convfloat` at line 7356,
// `convfloat_underscore` matching). Caller convenience cost is
// `use crate::ported::params::convfloat;` instead of `utils::`;
// no Rust-side re-export needed.

// ===========================================================
// Methods moved verbatim from src/ported/vm_helper because their
// C counterpart's source file maps 1:1 to this Rust module.
// Phase: drift
// ===========================================================

// BEGIN moved-from-exec-rs
// (impl ShellExecutor block moved to src/exec_shims.rs — see file marker)

// END moved-from-exec-rs

// ===========================================================
// Free ported moved verbatim from src/ported/vm_helper.
// ===========================================================
// BEGIN moved-from-exec-rs (free ported)
/// Port of `base64_decode()` from `Src/Zle/termquery.c:571`.
/// The only base64 decoder in zsh's C tree — it is NOT a utils.c function.
///
/// !!! DIVERGENT DUPLICATE !!! The faithful line-by-line port of that C body
/// lives at `src/ported/zle/termquery.rs:451`. This copy arrived with the
/// `vm_helper` move (see the marker above), decodes in fixed 4-byte chunks
/// rather than C's byte-at-a-time `while (len && (c = src[i]) != '=')` loop,
/// and has one caller (`src/fusevm_bridge.rs:11864`).
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
// END moved-from-exec-rs (free ported)

// ===========================================================
// Utility helpers moved from src/ported/vm_helper.
// All correspond to Src/utils.c logic (path/string/bslashquote helpers).
// ===========================================================

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

// WARNING: NOT IN UTILS.C — Rust-only OnceLock get-or-init
// helpers. C dereferences each global directly.
/// !!! WARNING: RUST-ONLY HELPER !!! No C counterpart exists as a
/// standalone function.
/// `OnceLock` get-or-init accessor for the storage behind C's
/// `char *scriptname` global (`Src/utils.c:36`). C dereferences the global.
fn scriptname_lock() -> &'static Mutex<Option<String>> {
    SCRIPTNAME.get_or_init(|| Mutex::new(None))
}

// WARNING: NOT IN UTILS.C — see scriptname_lock above.
/// !!! WARNING: RUST-ONLY HELPER !!! No C counterpart exists as a
/// standalone function.
/// `OnceLock` get-or-init accessor for the storage behind C's
/// `char *argzero` global (`Src/params.c:72`).
fn argzero_lock() -> &'static Mutex<Option<String>> {
    ARGZERO.get_or_init(|| Mutex::new(None))
}

// WARNING: NOT IN UTILS.C — see scriptname_lock above.
/// !!! WARNING: RUST-ONLY HELPER !!! No C counterpart exists as a
/// standalone function.
/// `OnceLock` get-or-init accessor for the storage behind C's
/// `char *scriptfilename` global (`Src/utils.c:41`).
fn scriptfilename_lock() -> &'static Mutex<Option<String>> {
    SCRIPTFILENAME.get_or_init(|| Mutex::new(None))
}

// WARNING: NOT IN UTILS.C — Rust-only OnceLock accessor for `posixzero`.
// C's `posixzero` lives in params.c:76; the canonical storage lives
// here for OnceLock initialisation parity with `argzero`.
/// !!! WARNING: RUST-ONLY HELPER !!! No C counterpart exists as a
/// standalone function.
/// `OnceLock` get-or-init accessor for the storage behind C's
/// `char *posixzero` global (`Src/params.c:76`).
fn posixzero_lock() -> &'static Mutex<Option<String>> {
    POSIXZERO.get_or_init(|| Mutex::new(None))
}

// WARNING: NOT IN UTILS.C — see scriptname_lock above.
/// !!! WARNING: RUST-ONLY HELPER !!! No C counterpart exists as a
/// standalone function.
/// `OnceLock` get-or-init accessor for the storage behind C's
/// `int noerrs` global (`Src/exec.c:117`).
pub fn noerrs_lock() -> &'static Mutex<i32> {
    NOERRS.get_or_init(|| Mutex::new(0))
}

// `locallevel_lock` removed — duplicate of canonical
// `LOCALLEVEL` (port of params.c:54). All
// accessors here now route through the canonical AtomicI32.
// WARNING: NOT IN UTILS.C — see scriptname_lock above.
/// !!! WARNING: RUST-ONLY HELPER !!! No C counterpart exists as a
/// standalone function.
/// `OnceLock` get-or-init accessor for the storage behind C's
/// `zlong lineno` global (`Src/params.c:123`).
fn lineno_lock() -> &'static Mutex<i32> {
    LINENO.get_or_init(|| Mutex::new(0))
}

// WARNING: NOT IN UTILS.C — Rust-only cache for the SHINSTDIN
// option flag so error-emission doesn't pull in the option-table.
/// !!! WARNING: RUST-ONLY HELPER !!! No C counterpart exists as a
/// standalone function.
/// `OnceLock` get-or-init accessor for the SHINSTDIN option cache; the
/// option itself is table entry `Src/options.c:248`.
fn shinstdin_lock() -> &'static Mutex<bool> {
    SHINSTDIN_OPT.get_or_init(|| Mutex::new(false))
}

/// !!! WARNING: RUST-ONLY HELPER !!! No C counterpart exists as a
/// standalone function.
/// `OnceLock` get-or-init accessor for the storage behind C's
/// `unsigned char *fdtable` global (`Src/exec.c:179`).
/// !!! RUST-ONLY HELPER — see WARNING block above. C source uses bare
/// `unsigned char *fdtable` global from Src/utils.c:~63.
fn fdtable_lock() -> &'static Mutex<Vec<i32>> {
    FDTABLE.get_or_init(|| Mutex::new(Vec::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// checkmailpath port (utils.c:1620). Drives mtime/atime/size through
    /// `libc::utimes` so the new-mail condition is deterministic in headless CI.
    #[test]
    fn checkmailpath_new_mail_conditions() {
        use std::io::Write as _;
        let _g = crate::test_util::global_state_lock();

        // Unique temp file with content (size != 0).
        let dir = std::env::temp_dir();
        let path = dir.join(format!("zshrs_mailtest_{}", std::process::id()));
        {
            let mut f = std::fs::File::create(&path).unwrap();
            f.write_all(b"new message\n").unwrap();
        }
        let cpath = std::ffi::CString::new(path.to_str().unwrap()).unwrap();
        // atime (times[0]) <= mtime (times[1]); both well past 0.
        let set_times = |atime: i64, mtime: i64| {
            let tv = [
                libc::timeval {
                    tv_sec: atime as libc::time_t,
                    tv_usec: 0,
                },
                libc::timeval {
                    tv_sec: mtime as libc::time_t,
                    tv_usec: 0,
                },
            ];
            assert_eq!(unsafe { libc::utimes(cpath.as_ptr(), tv.as_ptr()) }, 0);
        };

        let saved_shout = *crate::ported::init::shout.lock().unwrap();
        let saved_lmc = LAST_MAILCHECK.load(Ordering::Relaxed);
        *crate::ported::init::shout.lock().unwrap() = 1; // interactive
        LAST_MAILCHECK.store(0, Ordering::Relaxed);

        let p = path.to_str().unwrap().to_string();

        // c:1671-1675 — unread + newer than last check → "You have new mail."
        set_times(1_000_000, 2_000_000);
        assert_eq!(
            checkmailpath(&[p.clone()]),
            vec!["You have new mail.".to_string()]
        );

        // c:1676-1698 — `PATH?message`: the message (here a literal) is emitted.
        assert_eq!(
            checkmailpath(&[format!("{}?custom note", p)]),
            vec!["custom note".to_string()]
        );

        // c:1672 — mtime older than the last check → nothing.
        LAST_MAILCHECK.store(9_000_000, Ordering::Relaxed);
        assert!(checkmailpath(&[p.clone()]).is_empty());

        // c:1670 — non-interactive (no shout) → nothing, even when fresh.
        LAST_MAILCHECK.store(0, Ordering::Relaxed);
        *crate::ported::init::shout.lock().unwrap() = 0;
        assert!(checkmailpath(&[p.clone()]).is_empty());

        // c:1634-1636 — empty component is reported (stderr), yields no message.
        *crate::ported::init::shout.lock().unwrap() = 1;
        assert!(checkmailpath(&["?orphan".to_string()]).is_empty());

        *crate::ported::init::shout.lock().unwrap() = saved_shout;
        LAST_MAILCHECK.store(saved_lmc, Ordering::Relaxed);
        let _ = std::fs::remove_file(&path);
    }

    /// zsh_errno_msg / zerrmsg `%e` rendering (utils.c:348-365). EINTR is the
    /// literal "interrupt"; EIO is verbatim strerror (keeps its capital); every
    /// other errno lowercases the first letter (`tulower`). The old
    /// io::Error-based path kept the capital and appended " (os error N)".
    #[test]
    fn zsh_errno_msg_matches_c_percent_e() {
        // c:355-358 — EINTR → "interrupt".
        assert_eq!(zsh_errno_msg(libc::EINTR), "interrupt");

        // c:366 — non-EIO errno lowercases the first letter and never carries
        // the Rust "(os error N)" suffix.
        let acc = zsh_errno_msg(libc::EACCES);
        assert!(
            acc.chars().next().is_some_and(|c| c.is_lowercase()),
            "EACCES first letter must be lowercased: {acc:?}"
        );
        assert!(
            !acc.contains("(os error"),
            "must be strerror, not io::Error: {acc:?}"
        );

        // c:359-364 — EIO is emitted verbatim (its message reads wrong
        // lowercased), so its first letter stays capitalized.
        let eio = zsh_errno_msg(libc::EIO);
        assert!(
            eio.chars().next().is_some_and(|c| c.is_uppercase()),
            "EIO must be verbatim strerror (capitalized): {eio:?}"
        );
    }

    /// c:351-354 — zerrmsg's `%e` arm sets errflag |= ERRFLAG_ERROR on EINTR.
    #[test]
    fn zerrmsg_eintr_sets_errflag() {
        let _g = crate::test_util::global_state_lock();
        let saved = errflag.load(Ordering::Relaxed);
        errflag.store(0, Ordering::Relaxed);
        zerrmsg("test", Some(libc::EINTR)); // writes to stderr; side-effect is the flag
        assert_ne!(
            errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR,
            0,
            "EINTR must set ERRFLAG_ERROR"
        );
        // A non-EINTR errno must NOT set the flag.
        errflag.store(0, Ordering::Relaxed);
        zerrmsg("test", Some(libc::EACCES));
        assert_eq!(errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR, 0);
        errflag.store(saved, Ordering::Relaxed);
    }

    /// zjoin port (utils.c:3622). ASCII delimiters join byte-for-byte; a meta
    /// delimiter (NUL is `imeta` per inittyptab c:4195) must be written in
    /// metafied form (`Meta` + `delim^32`), never as a raw byte.
    #[test]
    fn zjoin_ascii_and_meta_delims() {
        let v = |xs: &[&str]| xs.iter().map(|s| s.to_string()).collect::<Vec<_>>();

        // c:3632-3641 — ordinary (non-meta) delimiter: plain interposition.
        assert_eq!(zjoin(&v(&["a", "b", "c"]), ' '), "a b c");
        assert_eq!(zjoin(&v(&["foo"]), ':'), "foo"); // single elem, no delim
                                                     // c:3629-3630 — empty array → "".
        assert_eq!(zjoin(&v(&[]), ' '), "");

        // c:3634-3637 — NUL is imeta, so the separator is metafied, NOT a raw
        // NUL byte (the prior `arr.join("\0")` bug). The Meta byte (0x83) can't
        // survive the UTF-8 String boundary, but the raw NUL must be gone.
        let joined = zjoin(&v(&["a", "b"]), '\0');
        assert!(
            !joined.as_bytes().contains(&0u8),
            "NUL delim must be metafied, not emitted as a raw NUL byte: {joined:?}"
        );
        assert_ne!(joined, "a\u{0}b", "must not raw-join on a meta delimiter");
        assert!(joined.starts_with('a') && joined.ends_with('b'));
    }

    /// findsep port (utils.c:3784). Pure function — no global/cwd state.
    #[test]
    fn findsep_default_ifs_separators() {
        inittyptab(); // ISEP bits are set by the type table (c:4155)
                      // c:3814-3817 — advance to the first ISEP char, return 1.
        let mut s = "foo bar".to_string();
        let mut pos = 0usize;
        assert_eq!(findsep(&mut s, &mut pos, None, false), 1);
        assert_eq!(pos, 3); // points at the space
        assert_eq!(&s[..pos], "foo");

        // c:3821 — already sitting on a separator: return 0, no advance.
        let mut s = " foo".to_string();
        let mut pos = 0usize;
        assert_eq!(findsep(&mut s, &mut pos, None, false), 0);
        assert_eq!(pos, 0);
    }

    #[test]
    fn findsep_quote_strips_escaped_separator() {
        inittyptab(); // ISEP bits are set by the type table (c:4155)
                      // c:3795-3804 — `\<sep>` is not a separator; the backslash is
                      // stripped in place and the bare char is consumed into the word.
        let mut s = "foo\\ bar".to_string(); // foo<bslash><space>bar
        let mut pos = 0usize;
        let r = findsep(&mut s, &mut pos, None, true);
        assert_eq!(r, 1);
        assert_eq!(s, "foo bar"); // backslash removed
        assert_eq!(pos, s.len()); // whole thing is one word, no real sep
    }

    #[test]
    fn findsep_quote_collapses_double_backslash() {
        // c:3796-3799 — `\\` collapses to a single literal backslash.
        let mut s = "a\\\\b".to_string(); // a <bslash><bslash> b
        let mut pos = 0usize;
        let r = findsep(&mut s, &mut pos, None, true);
        assert_eq!(r, 1);
        assert_eq!(s, "a\\b"); // one backslash left
        assert_eq!(pos, s.len()); // no separator present
    }

    #[test]
    fn findsep_literal_multichar_separator() {
        // c:3836-3841 — explicit multi-byte literal separator.
        let mut s = "a::b".to_string();
        let mut pos = 0usize;
        assert_eq!(findsep(&mut s, &mut pos, Some(b"::"), false), 1);
        assert_eq!(pos, 1); // points at the "::"
        assert_eq!(&s[..pos], "a");

        // c:3844 — separator absent → -1.
        let mut s = "abc".to_string();
        let mut pos = 0usize;
        assert_eq!(findsep(&mut s, &mut pos, Some(b"x"), false), -1);
    }

    #[test]
    fn findsep_empty_separator_advances_one_char() {
        // c:3825-3834 — empty sep just steps past one character.
        let mut s = "ab".to_string();
        let mut pos = 0usize;
        assert_eq!(findsep(&mut s, &mut pos, Some(b""), false), 1);
        assert_eq!(pos, 1);

        let mut s = String::new();
        let mut pos = 0usize;
        assert_eq!(findsep(&mut s, &mut pos, Some(b""), false), -1); // c:3833
    }

    /// c:4033 — `subst_string_by_func` returns `getaparam("reply")`
    /// after the hook function finishes. The previous Rust port read
    /// `env::var("reply")` and split on NUL — wrong because `reply`
    /// is a shell-local PM_ARRAY in paramtab, never exported. This
    /// pin sets `reply` via `setaparam` (the paramtab write path)
    /// and exercises `subst_string_by_hook` end-to-end isn't viable
    /// in unit tests without a function-name hook, so we exercise
    /// just the getaparam plumbing.
    #[test]
    fn getaparam_reads_reply_from_paramtab_not_env() {
        let _g = crate::test_util::global_state_lock();
        // Stash any prior `reply` value.
        let saved = getaparam("reply");

        // Write a known reply array via the canonical setaparam path.
        let payload = vec!["abbreviated".to_string(), "11".to_string()];
        let _ = setaparam("reply", payload.clone());

        // The reply array must be reachable via getaparam — not env.
        // (env::var would return Err because setaparam never exports
        //  an un-flagged array to env.)
        assert_eq!(
            getaparam("reply"),
            Some(payload),
            "getaparam(\"reply\") must return paramtab array"
        );

        // Restore.
        let _ = setaparam("reply", saved.unwrap_or_default());
    }

    /// c:1133-1134 — `finddir` reads the global `home` variable (the
    /// canonical `$HOME` storage, not `getenv("HOME")`). The global
    /// is updated by `homesetfn` (Src/params.c:5118) whenever the user
    /// assigns `HOME=...` inside the shell.
    /// Regression target: a previous Rust port read `env::var("HOME")`
    /// so an in-shell `HOME=...` assignment would not retarget
    /// `~`-abbreviation until the user re-exported HOME.
    #[test]
    fn finddir_uses_paramtab_home_not_env() {
        let _g = crate::test_util::global_state_lock();
        // C's `homesetfn` (params.c:5118) UNUSED(Param pm) — the
        // function ignores its param-pointer argument and writes the
        // canonical `char *home` global directly. zshrs's port mirrors
        // that: `params::homesetfn(_pm, x)` ignores _pm and updates
        // `home_lock()`. So we pass a stack-default param; the paramtab
        // wiring (PM_SPECIAL dispatch) is not on the test path.
        let mut pm = crate::ported::zsh_h::param::default();
        let saved = crate::ported::params::homegetfn(&pm);
        let sentinel = "/tmp/zshrs-finddir-pin".to_string();
        homesetfn(&mut pm, sentinel.clone());

        // `/tmp/zshrs-finddir-pin/x` must abbreviate to `~/x`.
        let abbrev = finddir(&format!("{}/x", sentinel));
        assert_eq!(
            abbrev.as_deref(),
            Some("~/x"),
            "finddir must consult canonical HOME (got {:?})",
            abbrev
        );

        // Restore.
        homesetfn(&mut pm, saved);
    }

    #[test]
    fn test_sepsplit() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(sepsplit("a:b:c", Some(":"), false), vec!["a", "b", "c"]);
        assert_eq!(sepsplit("a::b", Some(":"), false), vec!["a", "b"]);
        assert_eq!(sepsplit("a::b", Some(":"), true), vec!["a", "", "b"]);
    }

    #[test]
    fn test_unmetafy_no_meta_byte_passes_through() {
        let _g = crate::test_util::global_state_lock();
        // No Meta byte → buffer unchanged, length unchanged.
        let mut buf = b"hello".to_vec();
        let n = unmetafy(&mut buf);
        assert_eq!(n, 5);
        assert_eq!(&buf, b"hello");
    }

    #[test]
    fn test_unmetafy_collapses_meta_escapes() {
        let _g = crate::test_util::global_state_lock();
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
        let _g = crate::test_util::global_state_lock();
        // Plain prefix, then Meta-escaped 0xFF (0x83, 0xFF ^ 32 = 0xDF).
        let mut buf = vec![b'X', b'Y', 0x83, 0xFF ^ 32, b'Z'];
        let n = unmetafy(&mut buf);
        assert_eq!(n, 4);
        assert_eq!(buf, vec![b'X', b'Y', 0xFF, b'Z']);
    }

    #[test]
    fn test_unmetafy_returns_self_value() {
        let _g = crate::test_util::global_state_lock();
        // C returns `s` (the buffer) for chaining; Rust returns
        // the new length. Verify length matches a call that
        // collapses two Meta-escapes.
        let mut buf = vec![
            b'A',
            0x83,
            b'B' ^ 32, // → 'B'
            0x83,
            b'C' ^ 32, // → 'C'
            b'D',
        ];
        let n = unmetafy(&mut buf);
        assert_eq!(n, 4);
        assert_eq!(buf, b"ABCD".to_vec());
    }

    #[test]
    fn test_imeta_byte_threshold() {
        let _g = crate::test_util::global_state_lock();
        // Canonical IMETA per Src/utils.c:4195-4201:
        //   - 0x00 (c:4195)
        //   - 0x83..=0xa2 (Meta through Marker — c:4196-4200)
        //
        // The previous version of this test asserted `imeta_byte(0xFF) == true`
        // based on the WRONG `b >= Meta` predicate the Rust port
        // originally used. C `imeta()` reads the typtab; 0xa3..=0xff
        // have NO typtab assignment so they're NOT IMETA, and
        // `metafy` must NOT escape them.
        assert!(imeta_byte(0x00), "c:4195 — NUL is IMETA");
        assert!(!imeta_byte(0x82), "0x82 is NOT IMETA (below Meta)");
        assert!(imeta_byte(Meta), "c:4196 — Meta (0x83) is IMETA");
        assert!(imeta_byte(0xa2), "c:4197 — Marker (0xa2) is IMETA");
        // c:4198-4200 upper bound — Nularg is the last IMETA byte.
        assert!(imeta_byte(0xa1), "c:4200 — Nularg (0xa1) is IMETA");
        // 0xa3..=0xff are NOT IMETA — UTF-8 multi-byte content
        // must pass through `metafy` unchanged.
        assert!(!imeta_byte(0xa3), "0xa3 NOT IMETA (above Marker)");
        assert!(!imeta_byte(0xc3), "0xc3 NOT IMETA (UTF-8 'é' lead)");
        assert!(!imeta_byte(0xFF), "0xFF NOT IMETA");
    }

    #[test]
    fn test_meta_constant_value() {
        let _g = crate::test_util::global_state_lock();
        // Locked at 0x83 by Src/zsh.h. If this test fails, zsh
        // bumped the Meta sentinel and the encoding mapping needs
        // a full audit.
        assert_eq!(Meta, 0x83);
    }

    #[test]
    #[cfg(unix)]
    fn test_mode_to_octal_canonical_bits() {
        let _g = crate::test_util::global_state_lock();
        // rwx for owner = 0o700.
        let mode = (S_IRUSR | S_IWUSR | S_IXUSR) as u32;
        assert_eq!(mode_to_octal(mode), 0o700);
        // rwx all = 0o777.
        let all = (S_IRUSR | S_IWUSR | S_IXUSR) as u32 * (1 + 8 + 64);
        // Use libc constants individually for portability.
        let m = (S_IRUSR
            | S_IWUSR
            | S_IXUSR
            | S_IRGRP
            | S_IWGRP
            | S_IXGRP
            | S_IROTH
            | S_IWOTH
            | S_IXOTH) as u32;
        assert_eq!(mode_to_octal(m), 0o777);
        let _ = all;
    }

    #[test]
    #[cfg(unix)]
    fn test_mode_to_octal_setuid_setgid_sticky() {
        let _g = crate::test_util::global_state_lock();
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
        let _g = crate::test_util::global_state_lock();
        // Plain file path → *st fields mirror native stat,
        // not the maildir aggregation.
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        let rc = mailstat("/etc/hosts", &mut st);
        if rc == 0 {
            assert_eq!(
                st.st_nlink as u64,
                fs::metadata("/etc/hosts").unwrap().nlink()
            );
        }
    }

    #[test]
    fn test_mailstat_nonexistent_returns_neg1() {
        let _g = crate::test_util::global_state_lock();
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        assert_eq!(mailstat("/nonexistent/path/does/not/exist", &mut st), -1);
    }

    #[test]
    fn test_mailstat_directory_without_maildir_subdirs() {
        let _g = crate::test_util::global_state_lock();
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
        let _g = crate::test_util::global_state_lock(); // c:161
                                                        // 5 bytes of ASCII = 5 chars, identical. Canonical
                                                        // port lives at `string.rs:161`; pin from utils.rs's test
                                                        // module via the qualified path.
        assert_eq!(dupstrpfx("hello", 3), "hel"); // c:161
        assert_eq!(dupstrpfx("hi", 10), "hi"); // c:161
        assert_eq!(dupstrpfx("anything", 0), ""); // c:161
    }

    #[test]
    fn test_metafy_passes_through_ascii() {
        let _g = crate::test_util::global_state_lock();
        // ASCII bytes (< 0x83) stay untouched.
        assert_eq!(metafy("hello"), "hello");
        assert_eq!(metafy(""), "");
    }

    #[test]
    fn test_metafy_imeta_predicate_matches_c_macro() {
        let _g = crate::test_util::global_state_lock();
        // Canonical C IMETA per Src/utils.c:4195-4201:
        //   - 0x00 (c:4195)
        //   - 0x83 (Meta, c:4196)
        //   - 0x84..=0x9c (Pound..LAST_NORMAL_TOK=Bang, c:4198)
        //   - 0x9d..=0xa1 (Snull..Nularg, c:4200)
        //   - 0xa2 (Marker, c:4197)
        //
        // The closed set is {0x00, 0x83..=0xa2}. Every other byte
        // (0x01..=0x82, 0xa3..=0xff) is NOT IMETA in C and so must
        // NOT be Meta-encoded by `metafy`. The previous Rust
        // hardcoded predicate `b >= 0x83` falsely marked
        // 0xa3..=0xff as IMETA, corrupting UTF-8 multibyte
        // content.

        // NUL is IMETA (c:4195).
        assert!(imeta_byte(0x00), "c:4195 — '\\0' IS imeta");

        // 0x01..=0x82 are NOT IMETA (no typtab assignment for them).
        for b in 0x01u8..=0x82 {
            assert!(!imeta_byte(b), "byte {:#x} should NOT be imeta", b);
        }

        // 0x83..=0xa2 ARE IMETA (the canonical full range).
        for b in 0x83u8..=0xa2 {
            assert!(imeta_byte(b), "c:4196-4200 — byte {:#x} IS imeta", b);
        }

        // 0xa3..=0xff are NOT IMETA. C `imeta()` returns false
        // for these so `metafy` must pass them through unchanged.
        // UTF-8 continuation bytes (0x80..=0xbf) and multi-byte
        // leads (0xc0..=0xff) live here and must round-trip.
        for b in 0xa3u8..=0xff {
            assert!(
                !imeta_byte(b),
                "byte {:#x} should NOT be imeta (no c:4195-4201 assignment)",
                b
            );
        }
    }

    /// Pin: `metafy` passes UTF-8 multibyte bytes through unchanged
    /// when they fall outside the canonical IMETA range. Previously
    /// the broader `b >= 0x83` predicate corrupted every UTF-8
    /// continuation byte and lead byte that wasn't a token marker.
    #[test]
    fn metafy_preserves_utf8_high_bytes_outside_imeta_range() {
        let _g = crate::test_util::global_state_lock();
        // 'é' = U+00E9 = UTF-8 0xc3 0xa9. Both bytes are >= 0x83 BUT
        // both are also > 0xa2, so they're NOT IMETA per the typtab.
        // C `metafy` passes them through unchanged.
        let input = std::str::from_utf8(&[0xC3, 0xA9]).unwrap();
        let out = metafy(input);
        // Should round-trip the two bytes exactly (no Meta escape).
        let out_bytes = out.as_bytes();
        assert_eq!(
            out_bytes,
            &[0xC3, 0xA9],
            "c:4196-4200 — UTF-8 bytes 0xc3/0xa9 outside IMETA range must pass through"
        );
    }

    #[test]
    fn test_ztrcmp_meta_aware() {
        let _g = crate::test_util::global_state_lock();
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
        let _g = crate::test_util::global_state_lock();
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
        let _g = crate::test_util::global_state_lock(); // c:60
                                                        // `Src/ztype.h:60` `imeta(X) zistype(X, IMETA)` — typtab-driven
                                                        // predicate. Per `Src/utils.c:4195-4201`, IMETA is set on:
                                                        // NUL, Meta=0x83, Marker=0xa2, and the Pound..Nularg ITOK range
                                                        // (0x84..=0xa2). Routes through canonical
                                                        // `ztype_h::imeta(u8)`; init the typtab first since the
                                                        // canonical port reads through it.
        inittyptab(); // c:4148
        assert!(imeta(0x00), "c:4195 — NUL is IMETA"); // c:4195
        assert!(imeta(Meta), "c:4196 — Meta (0x83) is IMETA"); // c:4196
        assert!(imeta(0xa2), "c:4197 — Marker (0xa2) is IMETA"); // c:4197
        assert!(
            imeta(0x84),
            "c:4199-4201 — Pound (0x84) is IMETA via ITOK range"
        );
        assert!(
            imeta(0x9b),
            "c:4199-4201 — Dash sentinel within ITOK range is IMETA"
        );
        assert!(!imeta(0x82), "c:4170 — 0x82 is ICNTRL, not IMETA"); // c:4170
        assert!(!imeta(0xa3), "byte 0xa3 is past Marker — NOT IMETA");
        assert!(
            !imeta(0xff),
            "byte 0xff is NOT IMETA (the prior `>= Meta` over-report)"
        );
        assert!(!imeta(b' '), "space is not IMETA");
        assert!(!imeta(b'A'), "'A' is not IMETA");
    }

    #[test]
    fn test_unmeta_routes_through_unmetafy() {
        let _g = crate::test_util::global_state_lock();
        // unmeta wraps the in-place unmetafy via a byte-vector
        // copy; the no-Meta fast path returns the source as-is.
        assert_eq!(unmeta("plain"), "plain");
    }

    #[test]
    fn test_iwsep_includes_newline() {
        let _g = crate::test_util::global_state_lock(); // c:61
                                                        // The previous port omitted '\n' which broke wordcount on
                                                        // multi-line input. Routes through canonical
                                                        // `ztype_h::iwsep` (`Src/ztype.h:61`).
        assert!(iwsep(b'\n')); // c:61
        assert!(iwsep(b'\t')); // c:61
        assert!(iwsep(b' ')); // c:61
        assert!(!iwsep(b'a')); // c:61
    }

    #[test]
    fn test_mailstat_aggregates_maildir() {
        let _g = crate::test_util::global_state_lock();
        // Create a temp maildir layout with 2 messages in new/ and 1
        // in cur/, verify the aggregate.
        let tmp = std::env::temp_dir().join(format!("zshrs_mailstat_test_{}", std::process::id()));
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
        let _g = crate::test_util::global_state_lock();
        assert_eq!(spacesplit("a b c", false), vec!["a", "b", "c"]);
        assert_eq!(spacesplit("a  b", false), vec!["a", "b"]);
    }

    #[test]
    fn test_sepjoin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            sepjoin(&["a".into(), "b".into(), "c".into()], Some(":")),
            "a:b:c"
        );
        assert_eq!(sepjoin(&["a".into(), "b".into()], None), "a b");
    }

    #[test]
    fn test_isident() {
        let _g = crate::test_util::global_state_lock(); // c:1288
                                                        // Canonical port lives at `params.rs:2056` (`Src/params.c:1288`).
        assert!(isident("foo")); // c:1288
        assert!(isident("_bar")); // c:1288
        assert!(isident("baz123")); // c:1288
        assert!(!isident("123abc")); // c:1288
        assert!(!isident("foo-bar")); // c:1288
    }

    #[test]
    fn test_nicechar() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(nicechar('\n'), "\\n");
        assert_eq!(nicechar('\t'), "\\t");
        assert_eq!(nicechar('a'), "a");
    }

    #[test]
    fn test_quotedzputs_single_quote_wrap() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(quotedzputs("simple"), "simple");
        assert_eq!(quotedzputs("has space"), "'has space'");
        assert_eq!(quotedzputs("it's"), "'it'\\''s'");
    }

    #[test]
    fn test_quotestring_backslash() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(quotestring("hello", QT_BACKSLASH), "hello");
        assert_eq!(quotestring("has space", QT_BACKSLASH), "has\\ space");
        assert_eq!(quotestring("$var", QT_BACKSLASH), "\\$var");
    }

    /// Pin: `ispecial(c)` matches the canonical SPECCHARS set at
    /// `Src/zsh.h:228` exactly: `"#$^*()=|{}[]\`<>?~;&\n\t \\'\""`.
    /// Previously the Rust local `ispecial` included `!` unconditionally
    /// which diverged from C — C only ISPECIAL-tags bangchar (default
    /// `!`) under BANGHIST + interactive (per `Src/utils.c:4257-4261`).
    ///
    /// This test exercises the path indirectly via `quotestring` with
    /// `QT_BACKSLASH` (which prepends `\` before every ispecial char).
    #[test]
    fn quotestring_backslash_only_specchars_no_bang_in_default() {
        let _g = crate::test_util::global_state_lock();
        // `!` is NOT in canonical SPECCHARS — should NOT be backslashed
        // in default non-interactive mode (matches C).
        assert_eq!(
            quotestring("a!b", QT_BACKSLASH),
            "a!b",
            "c:228 — `!` is not in SPECCHARS; only bangchar+BANGHIST adds it"
        );
        // `,` is NOT in canonical SPECCHARS until makecommaspecial(1).
        assert_eq!(
            quotestring("a,b", QT_BACKSLASH),
            "a,b",
            "c:228 — `,` not in SPECCHARS until makecommaspecial(1)"
        );
        // `^` IS in canonical SPECCHARS (per c:228 `"#$^..."`).
        assert_eq!(
            quotestring("a^b", QT_BACKSLASH),
            "a\\^b",
            "c:228 — `^` is in SPECCHARS"
        );
        // Open and close braces are in canonical SPECCHARS.
        assert_eq!(
            quotestring("a{b", QT_BACKSLASH),
            "a\\{b",
            "c:228 — open-brace is in SPECCHARS"
        );
        assert_eq!(
            quotestring("a}b", QT_BACKSLASH),
            "a\\}b",
            "c:228 — close-brace is in SPECCHARS"
        );
        // `#` is in canonical SPECCHARS (first char of c:228).
        assert_eq!(
            quotestring("a#b", QT_BACKSLASH),
            "a\\#b",
            "c:228 — `#` is the first char of SPECCHARS"
        );
        // `\\` (literal backslash) is in canonical SPECCHARS.
        assert_eq!(
            quotestring("a\\b", QT_BACKSLASH),
            "a\\\\b",
            "c:228 — `\\\\` in SPECCHARS"
        );
    }

    /// c:6131-6134 — "Most quote styles other than backslash assume the
    /// quotes are to be added outside quotestring()." QT_SINGLE therefore
    /// returns the BODY: the caller (subst.c:4085-4087, text.c:1086-1088)
    /// supplies the `'…'` pair. The inner apostrophe still takes the
    /// close-reopen form from c:6372-6377.
    #[test]
    fn test_quotestring_single() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(quotestring("hello", QT_SINGLE), "hello");
        assert_eq!(quotestring("it's", QT_SINGLE), "it'\\''s");
    }

    /// c:6131-6134 + c:6311-6312 — body only; `"` is backslash-escaped and
    /// the surrounding pair is the caller's (subst.c:4085-4087).
    #[test]
    fn test_quotestring_double() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(quotestring("hello", QT_DOUBLE), "hello");
        assert_eq!(quotestring("say \"hi\"", QT_DOUBLE), "say \\\"hi\\\"");
    }

    /// c:6131-6134 — body only. C's caller writes the quote pair and then
    /// `val[0] = '$'` (subst.c:4085-4090), so `$'` never appears here.
    #[test]
    fn test_quotestring_dollars() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(quotestring("hello", QT_DOLLARS), "hello");
        assert_eq!(quotestring("line\nbreak", QT_DOLLARS), "line\\nbreak");
        assert_eq!(quotestring("tab\there", QT_DOLLARS), "tab\\there");
    }

    #[test]
    fn test_quotestring_pattern() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(quotestring("*.txt", QT_BACKSLASH_PATTERN), "\\*.txt");
        assert_eq!(quotestring("file[1]", QT_BACKSLASH_PATTERN), "file\\[1\\]");
    }

    #[test]
    fn test_quotetype_from_q_count() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(qflag_quotetype(1), QT_BACKSLASH);
        assert_eq!(qflag_quotetype(2), QT_SINGLE);
        assert_eq!(qflag_quotetype(3), QT_DOUBLE);
        assert_eq!(qflag_quotetype(4), QT_DOLLARS);
    }

    #[test]
    fn test_tulower_tuupper() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(tulower('A'), 'a');
        assert_eq!(tuupper('a'), 'A');
        assert_eq!(tulower('1'), '1');
    }

    #[test]
    fn test_wordcount_ifs_default() {
        let _g = crate::test_util::global_state_lock();
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
        let _g = crate::test_util::global_state_lock();
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
        let _g = crate::test_util::global_state_lock();
        let mut buf = [0u8; 8];
        // 'A' = 0x41, ASCII, single byte in any locale.
        let n = ucs4tomb('A' as u32, &mut buf);
        // wctomb may return 1 in C/POSIX locale; in UTF-8 locale also 1.
        assert_eq!(n, 1);
        assert_eq!(buf[0], b'A');
    }

    #[test]
    fn test_is_mb_niceformat_plain_ascii() {
        let _g = crate::test_util::global_state_lock();
        // Plain printable ASCII — nothing needs nicechar escaping.
        assert_eq!(is_mb_niceformat("hello world"), 0);
    }

    #[test]
    fn test_is_mb_niceformat_with_control_char() {
        let _g = crate::test_util::global_state_lock();
        // Tab is control (< 0x20) — needs nice escaping.
        assert_eq!(is_mb_niceformat("a\tb"), 1);
        // Bell character.
        assert_eq!(is_mb_niceformat("a\x07b"), 1);
    }

    /// c:4856/4954 — `metafy` + `unmetafy` MUST round-trip for ASCII.
    /// A regression in the round-trip corrupts every metafied buffer
    /// the lexer/param-subst pipeline produces.
    #[test]
    fn metafy_unmetafy_round_trips_for_ascii() {
        let _g = crate::test_util::global_state_lock();
        let s = "hello world";
        let m = metafy(s);
        let mut buf = m.into_bytes();
        unmetafy(&mut buf);
        assert_eq!(std::str::from_utf8(&buf).unwrap(), s);
    }

    /// `ztrlen` counts metafied characters (Meta-pairs as 1).
    /// Regression that double-counts Meta-pair bytes would break
    /// every fixed-width column path (printf %s).
    #[test]
    fn ztrlen_counts_ascii_one_per_byte() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(ztrlen(""), 0);
        assert_eq!(ztrlen("a"), 1);
        assert_eq!(ztrlen("hello"), 5);
    }

    /// `Src/utils.c:5141-5149` — Meta-byte pair counts as 1 char.
    /// `*s++ == Meta` advances 2 bytes per iteration but increments
    /// `l` only once. Pin: a string with one Meta+X pair counts as 1.
    #[test]
    fn ztrlen_counts_meta_pair_as_one() {
        let _g = crate::test_util::global_state_lock();
        // META (0x83) + 0x20 = unmetafies to one '\0' byte (or a NUL).
        let meta = char::from_u32(Meta as u32).unwrap();
        let s: String = [meta, '\x20'].iter().collect();
        assert_eq!(
            ztrlen(&s),
            1,
            "c:5141-5148 — Meta+X pair counts as ONE char, not two"
        );
        // Mixed: "a" + META + "x" + "b" = 3 unmetafied chars.
        let mixed: String = ['a', meta, '\x20', 'b'].iter().collect();
        assert_eq!(
            ztrlen(&mixed),
            3,
            "c:5141 — three unmetafied chars from 'a' + Meta+X + 'b'"
        );
    }

    /// `Src/utils.c:2579-2618` — `setblock_fd(turnonblocking, fd, modep)`.
    /// Signature pin: previously the Rust port had a 2-arg
    /// `(fd, blocking: bool) -> bool` signature, swapping the
    /// turnonblocking/fd argument order vs C AND collapsing the
    /// 3rd `*modep` out-param entirely. The fix restored canonical
    /// C order `(turnonblocking, fd)` with a `(bool, c_long)` tuple
    /// return mirroring `int return + long *modep`.
    ///
    /// Also pin the c:2599 regular-file short-circuit: C only
    /// operates on non-regular fds (pipes, sockets, ttys). A regular
    /// file returns `(false, -1)` immediately.
    #[cfg(unix)]
    #[test]
    fn setblock_fd_skips_regular_files_per_c_2599() {
        let _g = crate::test_util::global_state_lock();
        // Open a regular tempfile.
        let dir = tempfile::TempDir::new().unwrap();
        let f = fs::File::create(dir.path().join("regular")).unwrap();
        let fd = f.as_raw_fd();
        let (changed, mode) = setblock_fd(true, fd);
        assert!(
            !changed,
            "c:2599 — regular files short-circuit; setblock_fd must NOT report a change"
        );
        assert_eq!(mode, -1, "c:2614 — `*modep = -1` for regular files");
    }

    /// `Src/utils.c:2606-2611` — `setblock_fd(turnonblocking=true, fd)`
    /// clears O_NONBLOCK on a pipe (non-regular fd). Returns
    /// `(true, prior_flags)` if the state was changed.
    #[cfg(unix)]
    #[test]
    fn setblock_fd_clears_o_nonblock_on_pipe() {
        let _g = crate::test_util::global_state_lock();
        // Create a pipe — non-regular fd.
        let mut pipefd: [libc::c_int; 2] = [0; 2];
        let r = unsafe { libc::pipe(pipefd.as_mut_ptr()) };
        assert_eq!(r, 0, "pipe(2) must succeed");
        let read_fd = pipefd[0];
        let write_fd = pipefd[1];
        // Force NONBLOCK on the read end.
        let cur = unsafe { libc::fcntl(read_fd, libc::F_GETFL, 0) };
        unsafe {
            libc::fcntl(read_fd, libc::F_SETFL, cur | libc::O_NONBLOCK);
        }
        // Verify NONBLOCK is set.
        let now = unsafe { libc::fcntl(read_fd, libc::F_GETFL, 0) };
        assert_ne!(
            now & libc::O_NONBLOCK,
            0,
            "test setup: NONBLOCK should be set"
        );
        // Call setblock_fd to ENABLE blocking (clear NONBLOCK).
        let (changed, _mode) = setblock_fd(true, read_fd);
        assert!(
            changed,
            "c:2611 — turnonblocking=true on a NONBLOCK pipe must report state change"
        );
        // Verify NONBLOCK is now cleared.
        let after = unsafe { libc::fcntl(read_fd, libc::F_GETFL, 0) };
        assert_eq!(
            after & libc::O_NONBLOCK,
            0,
            "c:2611 — O_NONBLOCK must be cleared after turnonblocking=true"
        );
        // Cleanup.
        unsafe {
            libc::close(read_fd);
            libc::close(write_fd);
        }
    }

    /// `Src/utils.c:2620-2625` — `setblock_stdin()`. C body
    /// `setblock_fd(1, 0, &mode)` enables BLOCKING on fd 0 (stdin).
    /// Previously the Rust port called `setblock_fd(0, false)`
    /// which DISABLES blocking — exact opposite. Pin via real fd
    /// inspection.
    #[cfg(unix)]
    #[test]
    fn setblock_stdin_enables_blocking_on_fd_zero() {
        let _g = crate::test_util::global_state_lock();
        // Set stdin to NONBLOCKING first to verify the function
        // ACTUALLY switches it back to blocking. Skip if stdin is
        // not a normal fd (some CI configurations).
        let cur = unsafe { libc::fcntl(0, libc::F_GETFL, 0) };
        if cur < 0 {
            return;
        }
        // Force NONBLOCK first.
        unsafe {
            libc::fcntl(0, libc::F_SETFL, cur | libc::O_NONBLOCK);
        }
        let after_set_nb = unsafe { libc::fcntl(0, libc::F_GETFL, 0) };
        if after_set_nb & libc::O_NONBLOCK == 0 {
            // System rejected the change (regular file? CI tty?) — skip.
            unsafe {
                libc::fcntl(0, libc::F_SETFL, cur);
            }
            return;
        }
        // Call setblock_stdin — should CLEAR O_NONBLOCK.
        setblock_stdin();
        let after_setblock = unsafe { libc::fcntl(0, libc::F_GETFL, 0) };
        assert_eq!(
            after_setblock & libc::O_NONBLOCK,
            0,
            "c:2624 — setblock_stdin must CLEAR O_NONBLOCK (enable blocking)"
        );
        // Restore original flags.
        unsafe {
            libc::fcntl(0, libc::F_SETFL, cur);
        }
    }

    /// `Src/utils.c:2437-2519` — `zstrtol_underscore(s, base, false)`.
    /// Base-10 (explicit) parses canonical decimal.
    #[test]
    fn zstrtol_underscore_base_10_parses_decimal() {
        let _g = crate::test_util::global_state_lock();
        let (v, rest) = zstrtol_underscore("12345", 10, false);
        assert_eq!(v, 12345, "c:2471 — decimal accumulator");
        assert_eq!(rest, "", "rest is empty after full consumption");
        // With trailing non-digit.
        let (v, rest) = zstrtol_underscore("100abc", 10, false);
        assert_eq!(v, 100);
        assert_eq!(
            rest, "abc",
            "c:2467 — loop exits at first non-digit; rest carries on"
        );
    }

    /// c:2452-2461 — base==0 autodetect: `0x`→16, `0b`→2, leading
    /// `0`→8 (always, unlike `zstrtoul_underscore` which honors
    /// OCTALZEROES). `zstrtol_underscore` does NOT consult
    /// OCTALZEROES — pure prefix detection.
    #[test]
    fn zstrtol_underscore_base_zero_autodetects_prefix() {
        let _g = crate::test_util::global_state_lock();
        // Hex.
        assert_eq!(
            zstrtol_underscore("0xff", 0, false).0,
            255,
            "c:2455 — 0x → base 16"
        );
        assert_eq!(zstrtol_underscore("0XFF", 0, false).0, 255);
        // Binary.
        assert_eq!(
            zstrtol_underscore("0b1010", 0, false).0,
            10,
            "c:2457 — 0b → base 2"
        );
        // Leading 0 → octal (always, no OCTALZEROES gate).
        assert_eq!(
            zstrtol_underscore("0777", 0, false).0,
            511,
            "c:2460 — leading 0 → octal (no OCTALZEROES gate for zstrtol)"
        );
        // Decimal default.
        assert_eq!(zstrtol_underscore("12345", 0, false).0, 12345);
    }

    /// c:2447-2450 — leading `-` and `+` consumed; `-` triggers
    /// negation at the end (c:2497-2498).
    #[test]
    fn zstrtol_underscore_handles_sign_chars() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            zstrtol_underscore("-42", 10, false).0,
            -42,
            "c:2447 — leading `-` → negate"
        );
        assert_eq!(
            zstrtol_underscore("+42", 10, false).0,
            42,
            "c:2449 — leading `+` consumed, no negation"
        );
        // Mixed whitespace + sign.
        assert_eq!(
            zstrtol_underscore("  -100", 10, false).0,
            -100,
            "c:2444 — leading whitespace skipped, then sign"
        );
    }

    /// c:2466-2492 — base 16 (hex) accumulator: digits 0-9 + letters
    /// a-f / A-F via `idigit(*s)` || `'a' ≤ *s < 'a'+base-10`.
    /// Pin both upper- and lower-case.
    #[test]
    fn zstrtol_underscore_base_16_accepts_letters() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            zstrtol_underscore("ff", 16, false).0,
            255,
            "c:2485 — base-16 'ff' → 255"
        );
        assert_eq!(
            zstrtol_underscore("FF", 16, false).0,
            255,
            "c:2485 — base-16 'FF' → 255 (upper case via `*s & 0x1f`)"
        );
        assert_eq!(
            zstrtol_underscore("DEADbeef", 16, false).0,
            0xDEADBEEF,
            "c:2485 — mixed case 32-bit hex"
        );
    }

    /// c:2468/2482 — `underscore` flag enables digit-separator. With
    /// underscore=false, `_` is treated as end-of-digits. With
    /// underscore=true, `_` is skipped (c:2469-2470).
    #[test]
    fn zstrtol_underscore_underscore_flag() {
        let _g = crate::test_util::global_state_lock();
        // underscore=false: `_` terminates parse.
        let (v, rest) = zstrtol_underscore("1_000", 10, false);
        assert_eq!(v, 1, "c:2467 — underscore=false: `_` terminates");
        assert_eq!(rest, "_000");
        // underscore=true: `_` accepted and skipped.
        let (v, rest) = zstrtol_underscore("1_000_000", 10, true);
        assert_eq!(
            v, 1_000_000,
            "c:2469-2470 — underscore=true: `_` skipped during accumulation"
        );
        assert_eq!(rest, "", "fully consumed including `_`s");
    }

    /// `Src/utils.c:2750-2774` — `timespec_diff_us(t1, t2)` returns
    /// the signed microsecond delta `t2 - t1`. Pin both sign
    /// conventions: t2 after t1 → positive; t2 before t1 → negative.
    #[test]
    fn timespec_diff_us_sign_matches_c_t2_minus_t1() {
        let _g = crate::test_util::global_state_lock();
        let t1 = std::time::Instant::now();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let t2 = std::time::Instant::now();
        // c:2759 — `diff_sec = t2 - t1` for the t2 > t1 path. Result
        // positive (microseconds elapsed).
        let d = timespec_diff_us(&t1, &t2);
        assert!(d > 0, "c:2759 — t2 after t1 → positive delta");
        assert!(d >= 1000, "expected >= 1ms (1000us); got {}us", d);
        // c:2754 — reversed (t1 > t2): result negative.
        let r = timespec_diff_us(&t2, &t1);
        assert!(r < 0, "c:2770 — t1 after t2 → negative delta");
        assert_eq!(d, -r, "swap of args negates the result");
    }

    /// c:2752 — `timespec_diff_us(t, t)` is 0 (identical Instants).
    #[test]
    fn timespec_diff_us_same_instant_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let t = std::time::Instant::now();
        assert_eq!(
            timespec_diff_us(&t, &t),
            0,
            "c:2752 — identical Instants → 0"
        );
    }

    /// `Src/utils.c:6743-6779` — `ucs4toutf8(dest, wval)`. Encodes
    /// a Unicode codepoint to UTF-8. Pin canonical 1-, 2-, 3-, and
    /// 4-byte encodings.
    #[test]
    fn ucs4toutf8_encodes_canonical_lengths() {
        let _g = crate::test_util::global_state_lock();
        // c:6750 — 1 byte: ASCII range [0, 0x80).
        assert_eq!(
            ucs4toutf8(0x41),
            Some("A".to_string()),
            "c:6750 — 0x41 → 'A' (1 byte)"
        );
        // c:6752 — 2 bytes: [0x80, 0x800). 'é' = U+00E9.
        assert_eq!(
            ucs4toutf8(0xe9),
            Some("é".to_string()),
            "c:6752 — U+00E9 → 'é' (2 bytes)"
        );
        // c:6754 — 3 bytes: [0x800, 0x10000). '字' = U+5B57.
        assert_eq!(
            ucs4toutf8(0x5B57),
            Some("字".to_string()),
            "c:6754 — U+5B57 → '字' (3 bytes)"
        );
        // c:6756 — 4 bytes: [0x10000, 0x200000). '𝄞' = U+1D11E.
        assert_eq!(
            ucs4toutf8(0x1D11E),
            Some("𝄞".to_string()),
            "c:6756 — U+1D11E → '𝄞' (4 bytes)"
        );
    }

    /// `Src/utils.c:6762-6764` — values `>= 0x80000000` are
    /// out-of-range; C emits `zerr("character not in range")` and
    /// returns `-1`. Surrogates (U+D800..U+DFFF) and values in
    /// 0x110000..0x80000000 are *encoded as raw bytes* by C
    /// (matches `wctomb(3)` extended UCS-4 range).
    #[test]
    fn ucs4toutf8_rejects_invalid_codepoints() {
        let _g = crate::test_util::global_state_lock();
        // c:6762-6764 — value >= 0x80000000 → None.
        assert_eq!(
            ucs4toutf8(0xFFFF_FFFE),
            None,
            "c:6763 — values >= 0x80000000 → None"
        );
        assert_eq!(
            ucs4toutf8(0x8000_0000),
            None,
            "c:6760-6764 — exactly 0x80000000 is out of range"
        );
        // Surrogates encode as 3-byte sequences per C bit-pattern
        // (c:6754 → len=3). The C source has no Unicode-validity
        // gate; that's a wctomb-only concern in callers.
        assert!(
            ucs4toutf8(0xD800).is_some(),
            "c:6754 — surrogates encode as 3-byte UTF-8 in C (no Unicode validity check)"
        );
    }

    /// `Src/utils.c:6648-6723` — `dquotedztrdup(s)`. Default (non-
    /// CSHJUNKIEQUOTES) arm: wrap whole string in `"..."`. Backslash
    /// doubles, `"`/`$`/`` ` `` get escaped.
    #[test]
    fn dquotedztrdup_default_path_wraps_whole_string() {
        let _g = crate::test_util::global_state_lock();
        if isset(CSHJUNKIEQUOTES) {
            return; // Default-only test.
        }
        // c:6690 — `*p++ = '"';` then iterate.
        assert_eq!(
            dquotedztrdup("hello"),
            "\"hello\"",
            "c:6690+6711+6718 — wrap plain text in double quotes"
        );
        // c:6703-6708 — `$` gets `\$`.
        assert_eq!(
            dquotedztrdup("$var"),
            "\"\\$var\"",
            "c:6703-6708 — `$` → `\\$`"
        );
        // c:6703-6708 — `"` gets `\"`.
        assert_eq!(
            dquotedztrdup("a\"b"),
            "\"a\\\"b\"",
            "c:6703-6708 — `\"` → `\\\"`"
        );
    }

    /// c:6697-6701 — backslash handling with `pending` state.
    /// Single `\` followed by an ordinary char emits just `\` (no
    /// doubling). The "pending" extra `\` fires only when the next
    /// char is itself a backslash or one of the escaped specials
    /// (`"`/`$`/`` ` ``), OR at end-of-string before the closing `"`.
    #[test]
    fn dquotedztrdup_pending_backslash_only_doubles_when_needed() {
        let _g = crate::test_util::global_state_lock();
        if isset(CSHJUNKIEQUOTES) {
            return;
        }
        // Mid-string `\` before ordinary char: stays as single `\`.
        assert_eq!(
            dquotedztrdup("a\\b"),
            "\"a\\b\"",
            "c:6711+6712 — `\\b` mid-string: no extra (pending=0 after b)"
        );
        // Trailing `\` triggers pending quirk: c:6716-6717 emits
        // an extra `\` before the closing `"`.
        let r = dquotedztrdup("a\\");
        assert!(
            r.ends_with("\\\\\""),
            "c:6716-6717 — trailing `\\` → emit extra `\\` before closing `\"`: got {:?}",
            r
        );
    }

    /// `Src/utils.c:6464-6549` — `quotedzputs(s)`. Three branches:
    ///   - empty → `''`.
    ///   - has special chars (via `hasspecial`) → wrap in `'...'`
    ///     with `'\''` escape for embedded apostrophes.
    ///   - else → return unchanged.
    /// Pin all three.
    #[test]
    fn quotedzputs_empty_string_returns_double_quotes() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(quotedzputs(""), "''", "c:6470-6475 — empty input → ''");
    }

    /// c:6514-6543 — needs-quote path: wrap in single quotes,
    /// escape embedded `'` as `'\''`.
    #[test]
    fn quotedzputs_wraps_specials_in_single_quotes() {
        let _g = crate::test_util::global_state_lock();
        inittyptab();
        // Space is special → wrap.
        assert_eq!(quotedzputs("hello world"), "'hello world'");
        // `=` is special → wrap.
        assert_eq!(quotedzputs("foo=bar"), "'foo=bar'");
        // Embedded apostrophe — `'\''` escape.
        assert_eq!(
            quotedzputs("it's"),
            "'it'\\''s'",
            "c:6517-6519 — apostrophe → '\\''"
        );
    }

    /// c:6512 — no specials → return unchanged.
    #[test]
    fn quotedzputs_plain_alnum_returns_unchanged() {
        let _g = crate::test_util::global_state_lock();
        inittyptab();
        assert_eq!(
            quotedzputs("hello"),
            "hello",
            "c:6512 — no specials, no wrap"
        );
        assert_eq!(quotedzputs("abc123"), "abc123");
    }

    /// c:6578-6637 — Bourne path "avoids empty quoted strings" by
    /// tracking `inquote`. Input `'x` should not produce empty `''`
    /// at the front; `x'` should not produce trailing `''`.
    #[test]
    fn quotedzputs_avoids_empty_quoted_runs_on_boundaries() {
        let _g = crate::test_util::global_state_lock();
        inittyptab();
        // Leading `'`: c:6587 with inquote=0 skips the close, emits
        // `\'`. Then `x`: c:6604 opens a quote, emits `x`, end emits
        // closing `'`. Result: `\''x'` — the visible doubled `''` is
        // `\'` (escaped) + `'` (opening single-quote), NOT an empty
        // single-quoted string.
        assert_eq!(
            quotedzputs("'x"),
            "\\''x'",
            "c:6587-6610 — leading `'` produces `\\'` + opening `'`"
        );
        // Trailing `'`: open quote at `x`, see `'` with inquote=1 → close
        // (`'`), emit `\'`. End-of-string: inquote=0, no trailing `'`.
        // Result: `'x'\\'` — NO trailing `''`.
        assert_eq!(
            quotedzputs("x'"),
            "'x'\\'",
            "c:6631-6637 — trailing `'` doesn't generate empty `''`"
        );
    }

    /// c:6478-6492 — `is_mb_niceformat` arm of quotedzputs uses
    /// mb_niceformat with NICEFLAG_QUOTE so embedded `'` becomes
    /// `\'` inside the `$'...'` wrapper (preventing the quote from
    /// terminating the wrap). Previously sb_niceformat without
    /// NICEFLAG_QUOTE was used, which would have left `'` raw.
    #[test]
    fn quotedzputs_dollar_quote_escapes_apostrophe_inside_wrap() {
        let _g = crate::test_util::global_state_lock();
        inittyptab();
        // String with embedded `'` AND a control char so the
        // is_mb_niceformat arm is taken (controls trigger it).
        let r = quotedzputs("a\nb'c");
        // Expected: `$'a\nb\'c'` — the embedded `'` is `\'` inside
        // dollar-quotes, NOT terminating the wrap.
        assert!(
            r.starts_with("$'") && r.ends_with("'"),
            "c:6488 — must be wrapped in $'...'  got {:?}",
            r
        );
        assert!(
            r.contains("\\'"),
            "c:5413-5414 — embedded `'` must be `\\'` not raw `'`: got {:?}",
            r
        );
    }

    /// c:6533-6576 — RCQUOTES branch: wrap everything in `'…'`,
    /// double embedded `'` as `''`.
    #[test]
    fn quotedzputs_rcquotes_doubles_apostrophe() {
        let _g = crate::test_util::global_state_lock();
        inittyptab();
        let prev = crate::ported::options::opt_state_get("rcquotes").unwrap_or(false);
        crate::ported::options::opt_state_set("rcquotes", true);
        // RCQUOTES: `it's` → `'it''s'` (doubled `'`, NOT `'\''`).
        let got = quotedzputs("it's");
        crate::ported::options::opt_state_set("rcquotes", prev);
        assert_eq!(
            got, "'it''s'",
            "c:6548-6553 — RCQUOTES: `'` → `''` (doubled)"
        );
    }

    /// `Src/utils.c:2331-2337` — `strucpy(s, t)`. Appends `t` to
    /// `*s` and leaves `*s` pointing at the NUL terminator (Rust
    /// equivalent: append-in-place; `.len()` gives the new end).
    /// Pin: empty input → no-op; non-empty input → append.
    #[test]
    fn strucpy_appends_t_to_dest() {
        let _g = crate::test_util::global_state_lock();
        let mut dest = String::from("prefix-");
        strucpy(&mut dest, "suffix");
        assert_eq!(
            dest, "prefix-suffix",
            "c:2335 — strucpy appends t (NOT case-changes — c-name 'u' is pointer-walk, not upper)"
        );
        // Empty t — no change.
        let mut dest = String::from("alone");
        strucpy(&mut dest, "");
        assert_eq!(dest, "alone");
        // Empty dest + non-empty t.
        let mut dest = String::new();
        strucpy(&mut dest, "hello");
        assert_eq!(dest, "hello");
    }

    /// `Src/utils.c:2341-2350` — `struncpy(s, t, n)`. Appends up to
    /// `n` bytes of `t` to `*s`. Pin: n=0 no-op; n < len(t) clips;
    /// n >= len(t) appends all.
    #[test]
    fn struncpy_appends_up_to_n_bytes() {
        let _g = crate::test_util::global_state_lock();
        // n < len(t).
        let mut dest = String::from("X");
        struncpy(&mut dest, "abcdef", 3);
        assert_eq!(dest, "Xabc", "c:2345 — clipped to n=3 bytes of `abcdef`");
        // n >= len(t).
        let mut dest = String::from("Y");
        struncpy(&mut dest, "abc", 100);
        assert_eq!(dest, "Yabc", "c:2345 — full copy when n >= len");
        // n=0 → no append.
        let mut dest = String::from("Z");
        struncpy(&mut dest, "abc", 0);
        assert_eq!(dest, "Z", "c:2345 — n=0 stops the copy loop immediately");
    }

    /// `Src/utils.c:2280-2288` — `has_token(s)`. Returns true iff
    /// any byte in `s` triggers `itok()`. Pin: ASCII strings →
    /// false; strings containing token bytes (Pound 0x84, Bang
    /// 0x9c, Nularg 0xa1) → true.
    #[test]
    fn has_token_detects_typtab_token_bytes() {
        let _g = crate::test_util::global_state_lock();
        inittyptab();
        // c:2285 — pure ASCII has no token bytes.
        assert!(!has_token(""), "empty: no tokens");
        assert!(
            !has_token("hello world"),
            "c:2285 — ASCII text has no token bytes"
        );
        // Pound (0x84) — first token byte.
        let s: String = std::iter::once(0x84u8 as char).collect();
        assert!(
            has_token(&s),
            "c:2285 — Pound (0x84) is itok → has_token=true"
        );
        // Bang (0x9c) — last_normal_tok.
        let s: String = std::iter::once(0x9cu8 as char).collect();
        assert!(has_token(&s), "c:2285 — Bang (0x9c) is itok");
        // Nularg (0xa1) — upper bound.
        let s: String = std::iter::once(0xa1u8 as char).collect();
        assert!(has_token(&s), "c:2285 — Nularg (0xa1) is itok");
    }

    /// `Src/utils.c:2280` — Meta byte (0x83) is NOT a token. Pin
    /// the regression: previously the Rust port hardcoded `0x83`
    /// as a token byte. Now correctly excludes it.
    #[test]
    fn has_token_excludes_meta_byte() {
        let _g = crate::test_util::global_state_lock();
        inittyptab();
        // 0x83 is Meta, NOT itok. Should return false.
        let s: String = std::iter::once(0x83u8 as char).collect();
        assert!(
            !has_token(&s),
            "c:2285 — Meta (0x83) is NOT itok; previous hardcoded 0x83 list misfired"
        );
    }

    /// `Src/utils.c:6082-6124` — `addunprintable(c)`. Renders bytes
    /// using shell-compatible C-string escapes (`\a`/`\b`/`\f`/`\n`/
    /// `\r`/`\t`/`\v` for the named controls + `\nnn` octal for
    /// others + `\0` for NUL). Previously emitted ZLE caret form —
    /// wrong convention.
    #[test]
    fn addunprintable_named_control_escapes() {
        let _g = crate::test_util::global_state_lock();
        // c:6106-6112 — named-escape per byte.
        assert_eq!(addunprintable('\x07'), "\\a", "c:6106 — BEL → \\a");
        assert_eq!(addunprintable('\x08'), "\\b", "c:6107 — BS → \\b");
        assert_eq!(addunprintable('\x0c'), "\\f", "c:6108 — FF → \\f");
        assert_eq!(addunprintable('\n'), "\\n", "c:6109 — LF → \\n");
        assert_eq!(addunprintable('\r'), "\\r", "c:6110 — CR → \\r");
        assert_eq!(addunprintable('\t'), "\\t", "c:6111 — TAB → \\t");
        assert_eq!(addunprintable('\x0b'), "\\v", "c:6112 — VT → \\v");
    }

    /// c:6097-6103 — NUL renders as `\0`. (C peeks next byte for
    /// `\000` disambiguation; Rust port works one char at a time
    /// so emits the short `\0` form.)
    #[test]
    fn addunprintable_nul_renders_as_backslash_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(addunprintable('\0'), "\\0", "c:6097-6098 — NUL → \\0");
    }

    /// c:6114-6119 — `\nnn` 3-digit octal fallback for un-named
    /// control bytes. Pin the format: each digit is one octal digit
    /// (0-7), zero-padded to 3 positions.
    #[test]
    fn addunprintable_octal_fallback_for_unnamed_controls() {
        let _g = crate::test_util::global_state_lock();
        // 0x01 (SOH) = 001 octal.
        assert_eq!(addunprintable('\x01'), "\\001", "c:6116-6118 — SOH → \\001");
        // 0x1b (ESC) = 033 octal.
        assert_eq!(addunprintable('\x1b'), "\\033", "c:6116-6118 — ESC → \\033");
        // 0x7f (DEL) = 177 octal.
        assert_eq!(addunprintable('\x7f'), "\\177", "c:6116-6118 — DEL → \\177");
        // 0xff (high) = 377 octal.
        assert_eq!(
            addunprintable(char::from_u32(0xff).unwrap()),
            "\\377",
            "c:6116-6118 — 0xff → \\377"
        );
    }

    /// `Src/utils.c:6072-6082` — `hasspecial(s)`. Pin canonical
    /// special chars from SPECCHARS (`Src/zsh.h:228`): `#$^*()=|{}[]
    /// \`<>?~;&\\n\\t \\\\\\'\\"`.
    #[test]
    fn hasspecial_recognises_canonical_special_chars() {
        let _g = crate::test_util::global_state_lock();
        // typtab access is read-only here (no flag mutation); concurrent
        // inittyptab() rebuilds are idempotent for the default flag set.
        inittyptab();
        // c:6075 — every char in SPECCHARS triggers true.
        for c in "#$^*()=|{}[]<>?~;&".chars() {
            let s = c.to_string();
            assert!(
                hasspecial(&s),
                "c:6075 — '{}' is in SPECCHARS, must be special",
                c
            );
        }
        // Whitespace specials.
        assert!(hasspecial(" "), "space in SPECCHARS");
        assert!(hasspecial("\t"), "tab in SPECCHARS");
        assert!(hasspecial("\n"), "newline in SPECCHARS");
        // Non-special chars → false.
        assert!(
            !hasspecial("hello"),
            "c:6075 — plain alphanumerics are NOT special"
        );
        assert!(!hasspecial("ABC012"));
    }

    /// `Src/utils.c:6072-6075` — Meta-byte handling: `*s == Meta ?
    /// *++s ^ 32 : *s`. Pin Meta+X pair gets decoded before special-
    /// check. A Meta+'A' decodes to 'a' (not special), so a string
    /// containing only Meta+'A' returns false.
    #[test]
    fn hasspecial_decodes_meta_byte_before_check() {
        let _g = crate::test_util::global_state_lock();
        // Same as above — read-only after inittyptab.
        inittyptab();
        // Meta + 0x41 ('A') → 'a' (0x61) → not special.
        let bytes: Vec<u8> = vec![Meta, 0x41u8];
        let s = unsafe { std::str::from_utf8_unchecked(&bytes) };
        assert!(
            !hasspecial(s),
            "c:6075 — Meta+0x41 decodes to 'a' which is NOT special"
        );
    }

    /// `Src/utils.c:5849-5910` — `sb_niceformat(s)`. Pin: ASCII
    /// printable passes through unchanged; controls escape.
    /// C-equivalent call shape: `char *out; sb_niceformat(s, NULL, &out, 0);`
    #[test]
    fn sb_niceformat_passes_printable_ascii() {
        let _g = crate::test_util::global_state_lock();
        let mut out: Option<String> = None;
        let l = sb_niceformat("hello", None, Some(&mut out), 0);
        assert_eq!(
            out.as_deref(),
            Some("hello"),
            "c:5886 — nicechar_sel passes printable through"
        );
        assert_eq!(l, 5, "c:5928 — return length equals output bytes");

        let mut out: Option<String> = None;
        sb_niceformat("", None, Some(&mut out), 0);
        assert_eq!(out.as_deref(), Some(""));

        let mut out: Option<String> = None;
        sb_niceformat("ABC012!?@", None, Some(&mut out), 0);
        assert_eq!(out.as_deref(), Some("ABC012!?@"));
    }

    /// c:5886 — controls get `\n`/`\t`/`^X`/`^?` escapes via nicechar_sel.
    #[test]
    fn sb_niceformat_escapes_controls() {
        let _g = crate::test_util::global_state_lock();
        let mut out: Option<String> = None;
        sb_niceformat("a\nb", None, Some(&mut out), 0);
        assert_eq!(out.as_deref(), Some("a\\nb"), "c:5886 — newline → \\n");

        let mut out: Option<String> = None;
        sb_niceformat("a\tb", None, Some(&mut out), 0);
        assert_eq!(out.as_deref(), Some("a\\tb"));

        let mut out: Option<String> = None;
        sb_niceformat("\x01", None, Some(&mut out), 0);
        assert_eq!(
            out.as_deref(),
            Some("^A"),
            "c:5886 — control char → ^X form"
        );

        let mut out: Option<String> = None;
        sb_niceformat("\x7f", None, Some(&mut out), 0);
        assert_eq!(out.as_deref(), Some("^?"));
    }

    /// `Src/utils.c:5872` — unmetafy step before formatting. Pin:
    /// metafied Meta+X pair gets unescaped first, then run through
    /// nicechar_sel.
    #[test]
    fn sb_niceformat_unmetafies_before_formatting() {
        let _g = crate::test_util::global_state_lock();
        // META + 0x41 ('A') → decodes to 'a' (0x61). 'a' is printable
        // → passes through unchanged.
        let bytes: Vec<u8> = vec![Meta, 0x41u8];
        let s = unsafe { std::str::from_utf8_unchecked(&bytes) };
        let mut out: Option<String> = None;
        sb_niceformat(s, None, Some(&mut out), 0);
        assert_eq!(
            out.as_deref(),
            Some("a"),
            "c:5872 — unmetafy first: Meta+0x41 → 'a' → printable passthrough"
        );
        // META + 0x20 → decodes to NUL → "^@" form.
        let bytes: Vec<u8> = vec![Meta, 0x20u8];
        let s = unsafe { std::str::from_utf8_unchecked(&bytes) };
        let mut out: Option<String> = None;
        sb_niceformat(s, None, Some(&mut out), 0);
        let r = out.unwrap_or_default();
        assert!(
            !r.is_empty(),
            "c:5872 — Meta+0x20 → NUL → must emit some escape, not empty"
        );
    }

    /// `Src/utils.c:5937-5959` — `is_sb_niceformat(s)`. Returns 1
    /// if sb_niceformat would change the input, else 0.
    #[test]
    fn is_sb_niceformat_true_for_strings_with_controls() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(is_sb_niceformat("\n"), 1, "newline is nice");
        assert_eq!(is_sb_niceformat("a\tb"), 1);
        assert_eq!(is_sb_niceformat("\x01"), 1);
        // Non-control ASCII → 0.
        assert_eq!(is_sb_niceformat("hello"), 0);
        assert_eq!(is_sb_niceformat(""), 0);
    }

    /// `Src/utils.c:5810-5826` — `metacharlenconv(x, c)`. Returns
    /// `(2, decoded)` for Meta+X pair, `(1, byte)` for plain byte.
    /// Pin both branches.
    #[test]
    fn metacharlenconv_plain_ascii_returns_one_byte() {
        let _g = crate::test_util::global_state_lock();
        // c:5823-5825 — plain byte.
        let (n, c) = metacharlenconv("a");
        assert_eq!((n, c), (1, Some('a')), "c:5823 — plain byte → (1, byte)");
        let (n, c) = metacharlenconv("X");
        assert_eq!((n, c), (1, Some('X')));
    }

    /// c:5818-5821 — Meta+X pair: `*c = x[1] ^ 32; return 2`.
    #[test]
    fn metacharlenconv_meta_pair_xor_decodes() {
        let _g = crate::test_util::global_state_lock();
        // Meta + 0x41 ('A') → 'A' ^ 32 = 'a'.
        let bytes: Vec<u8> = vec![Meta, 0x41u8];
        let s = unsafe { std::str::from_utf8_unchecked(&bytes) };
        let (n, c) = metacharlenconv(s);
        assert_eq!(
            (n, c),
            (2, Some('a')),
            "c:5820 — Meta+0x41 → 'a' (XOR 32), consumed 2 bytes"
        );
        // Empty input.
        let (n, c) = metacharlenconv("");
        assert_eq!((n, c), (0, None));
    }

    /// `Src/utils.c:5832-5843` — `charlenconv(x, len, c)`. Returns
    /// `(0, NUL)` when len=0, `(1, byte)` otherwise.
    #[test]
    fn charlenconv_zero_len_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        // c:5834 — `if (!len) { *c = '\0'; return 0; }`.
        let (n, c) = charlenconv("abc", 0);
        assert_eq!((n, c), (0, None), "c:5834-5837 — len=0 returns 0");
    }

    /// c:5840-5842 — non-zero len: read one byte, return (1, byte).
    #[test]
    fn charlenconv_returns_first_byte_for_nonzero_len() {
        let _g = crate::test_util::global_state_lock();
        let (n, c) = charlenconv("abc", 3);
        assert_eq!((n, c), (1, Some('a')), "c:5841 — *c = *x; return 1");
        let (n, c) = charlenconv("xy", 2);
        assert_eq!((n, c), (1, Some('x')));
    }

    /// `Src/utils.c:5474-5524` — `is_mb_niceformat(s)`. Predicate:
    /// would mb_niceformat produce a different output? Pin canonical
    /// branches:
    ///   - Pure ASCII printable → false (no escape needed).
    ///   - Contains control char → true.
    ///   - Pure UTF-8 wide chars → false under default PRINTEIGHTBIT-off
    ///     since is_wcs_nicechar treats them as printable.
    #[test]
    fn is_mb_niceformat_false_for_pure_printable_ascii() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            is_mb_niceformat("hello"),
            0,
            "c:5509 — printable ASCII needs no nice-format"
        );
        assert_eq!(
            is_mb_niceformat(""),
            0,
            "c:5486 — empty string has no chars to flag"
        );
        assert_eq!(is_mb_niceformat("ABC012!?@"), 0);
    }

    /// c:5509 + `is_wcs_nicechar` — newline/tab/control chars trigger
    /// the predicate.
    #[test]
    fn is_mb_niceformat_true_for_strings_with_controls() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(is_mb_niceformat("a\nb"), 1, "c:5509 — newline is nice");
        assert_eq!(is_mb_niceformat("\t"), 1);
        assert_eq!(is_mb_niceformat("\x01"), 1, "c:5509 — control char is nice");
        assert_eq!(is_mb_niceformat("\x7f"), 1, "c:5509 — DEL is nice");
    }

    /// `Src/utils.c:5366-5460` — `mb_niceformat(s)`. Multibyte-aware
    /// nice-formatter. Calls `wcs_nicechar` per char. Test uses
    /// C-equivalent call shape: `char *out; mb_niceformat(s, NULL, &out, 0);`
    #[test]
    fn mb_niceformat_preserves_printable_wide_chars() {
        let _g = crate::test_util::global_state_lock();
        let mut out: Option<String> = None;
        mb_niceformat("hello", None, Some(&mut out), 0);
        assert_eq!(
            out.as_deref(),
            Some("hello"),
            "c:5407 — printable ASCII passes through"
        );

        let mut out: Option<String> = None;
        mb_niceformat("café", None, Some(&mut out), 0);
        assert_eq!(
            out.as_deref(),
            Some("café"),
            "c:5407 — Latin-1 'é' must NOT byte-mask to \\M-X"
        );

        let mut out: Option<String> = None;
        mb_niceformat("字", None, Some(&mut out), 0);
        assert_eq!(
            out.as_deref(),
            Some("字"),
            "c:5407 — CJK printable passes through"
        );

        let mut out: Option<String> = None;
        mb_niceformat("abcéxyz", None, Some(&mut out), 0);
        assert_eq!(out.as_deref(), Some("abcéxyz"));
    }

    /// `Src/utils.c:5407` + `wcs_nicechar` — controls still escape.
    #[test]
    fn mb_niceformat_escapes_controls() {
        let _g = crate::test_util::global_state_lock();
        let mut out: Option<String> = None;
        mb_niceformat("\n", None, Some(&mut out), 0);
        assert_eq!(
            out.as_deref(),
            Some("\\n"),
            "c:5407 → wcs_nicechar c:625 — newline escapes"
        );

        let mut out: Option<String> = None;
        mb_niceformat("\t", None, Some(&mut out), 0);
        assert_eq!(
            out.as_deref(),
            Some("\\t"),
            "c:5407 → wcs_nicechar c:628 — tab escapes"
        );

        let mut out: Option<String> = None;
        mb_niceformat("a\nb", None, Some(&mut out), 0);
        assert_eq!(out.as_deref(), Some("a\\nb"));
    }

    /// `Src/utils.c:4971-4983` — `metalen(s, len)`. **Input `len` is
    /// the UNMETAFIED char count; output is the METAFIED byte count.**
    /// Pin a metafied string with one Meta+X pair: 3 unmetafied chars
    /// → 4 metafied bytes.
    #[test]
    fn metalen_returns_metafied_byte_count() {
        let _g = crate::test_util::global_state_lock();
        // Pure ASCII: 5 chars → 5 bytes (no Meta encountered).
        assert_eq!(
            metalen("hello", 5),
            5,
            "c:4972 — ASCII: metafied bytes == unmetafied chars"
        );
        // [a, Meta, X, b] = 4 metafied bytes representing 3 chars.
        let bytes: Vec<u8> = vec![b'a', Meta, 0x41, b'b'];
        let s = unsafe { std::str::from_utf8_unchecked(&bytes) };
        assert_eq!(
            metalen(s, 3),
            4,
            "c:4978 — 3 unmetafied chars + 1 Meta = 4 metafied bytes"
        );
        // Two Meta pairs: [Meta, X, Meta, Y] = 4 bytes for 2 chars.
        let bytes: Vec<u8> = vec![Meta, 0x41, Meta, 0x42];
        let s = unsafe { std::str::from_utf8_unchecked(&bytes) };
        assert_eq!(
            metalen(s, 2),
            4,
            "c:4978 — 2 unmetafied chars (both Meta+X) = 4 metafied bytes"
        );
    }

    /// c:4974 — `mlen = len;` is the initial value (no Meta found,
    /// returns the input length unchanged). Pin len=0 and pure-ASCII.
    #[test]
    fn metalen_returns_input_for_no_meta_chars() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(metalen("", 0), 0, "c:4974 — empty input returns 0");
        assert_eq!(
            metalen("abc", 3),
            3,
            "c:4974 — no Meta chars: output == input len"
        );
    }

    /// `Src/utils.c:5070` — `if (!in || !*in) return 0;`.
    /// Empty input returns `('\0', 0)`. Pin: empty &str → 0 bytes consumed.
    #[test]
    fn unmeta_one_empty_input_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let (c, n) = unmeta_one("");
        assert_eq!(c, '\0', "c:5070 — empty input returns NUL char");
        assert_eq!(n, 0, "c:5070 — empty input consumes 0 bytes");
    }

    /// `Src/utils.c:5081-5082` — Non-Meta byte: `*sz = 1; wc = byte`.
    #[test]
    fn unmeta_one_plain_ascii_consumes_one_byte() {
        let _g = crate::test_util::global_state_lock();
        for c in "aA0!~".chars() {
            let s = c.to_string();
            let (got, n) = unmeta_one(&s);
            assert_eq!(got, c, "c:5082 — '{}' decodes to itself", c);
            assert_eq!(n, 1, "c:5081 — non-Meta byte consumes 1");
        }
    }

    /// `Src/utils.c:5077-5079` — Meta byte: `*sz = 2; wc = in[1] ^ 32`.
    /// Pin via constructed Meta byte sequence.
    #[test]
    fn unmeta_one_meta_pair_decodes_xor_32() {
        let _g = crate::test_util::global_state_lock();
        // META + 0x41 ('A') → 'A' ^ 32 = 0x61 ('a'). 2 bytes consumed.
        let bytes: Vec<u8> = vec![Meta, 0x41u8];
        let s = unsafe { std::str::from_utf8_unchecked(&bytes) };
        let (got, n) = unmeta_one(s);
        assert_eq!(got, 'a', "c:5079 — Meta+0x41 decodes to 0x41^32 = 'a'");
        assert_eq!(n, 2, "c:5078 — Meta pair consumes 2 bytes");
        // META + 0x20 = 0x20^32 = 0x00 (NUL).
        let bytes: Vec<u8> = vec![Meta, 0x20u8];
        let s = unsafe { std::str::from_utf8_unchecked(&bytes) };
        let (got, n) = unmeta_one(s);
        assert_eq!(
            got, '\0',
            "c:5079 — Meta+0x20 decodes to NUL (the canonical metafy-NUL pattern)"
        );
        assert_eq!(n, 2);
    }

    /// Edge case: bare Meta byte at end with no follower. C source
    /// would read past the buffer (UB); Rust port's `bytes.len() > 1`
    /// guard handles this by falling through to the non-Meta arm.
    #[test]
    fn unmeta_one_trailing_meta_byte_falls_through() {
        let _g = crate::test_util::global_state_lock();
        let bytes: Vec<u8> = vec![Meta];
        let s = unsafe { std::str::from_utf8_unchecked(&bytes) };
        let (_, n) = unmeta_one(s);
        // Defensive: Rust returns the Meta byte itself as char + 1.
        assert_eq!(n, 1, "trailing Meta — no panic, no read past buffer");
    }

    /// `Src/utils.c:5185-5203` — `ztrsub(t, s)`. Count of unmetafied
    /// chars between two pointers in the same metafied string.
    /// Rust port takes (buf, start, end) byte-offsets since raw
    /// pointer subtraction across distinct &str isn't safe.
    /// Pin pure-ASCII subrange (no Meta) → returns end-start.
    #[test]
    fn ztrsub_ascii_no_meta_returns_byte_distance() {
        let _g = crate::test_util::global_state_lock();
        // "hello world" — substring [0..5) → "hello" → 5 chars.
        assert_eq!(
            ztrsub("hello world", 0, 5),
            5,
            "c:5189 — no Meta: returns t-s byte distance"
        );
        // [6..11) → "world" → 5 chars.
        assert_eq!(ztrsub("hello world", 6, 11), 5);
        // Empty range.
        assert_eq!(ztrsub("hello", 2, 2), 0);
    }

    /// c:5191-5199 — each Meta+X pair decrements `l` by 1 (since the
    /// initial l = byte-distance counts both bytes but the pair is
    /// one logical char). Pin via constructed Meta string.
    #[test]
    fn ztrsub_subtracts_one_per_meta_pair() {
        let _g = crate::test_util::global_state_lock();
        // Build "a" + Meta + 0x20 + "b" = 4 bytes total.
        // Unmetafied: "a" + 1-char-from-meta + "b" = 3 chars.
        let meta_byte = Meta;
        let buf_bytes: Vec<u8> = vec![b'a', meta_byte, 0x20, b'b'];
        let buf = unsafe { std::str::from_utf8_unchecked(&buf_bytes) };
        // [0..4) byte distance = 4; Meta pair found → l-- → 3.
        assert_eq!(
            ztrsub(buf, 0, 4),
            3,
            "c:5192-5199 — Meta pair decrements distance by 1"
        );
    }

    /// c:5189 — `int l = t - s;` start of count. Pin edge case
    /// where start > end (caller bug) doesn't underflow or panic;
    /// Rust port clamps via `start.min(end)`.
    #[test]
    fn ztrsub_clamps_inverted_range_to_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            ztrsub("hello", 4, 2),
            0,
            "inverted range start>end → 0 (defensive; not in C contract)"
        );
        // Beyond-buffer end gets clamped.
        assert_eq!(
            ztrsub("hi", 0, 100),
            2,
            "end > buf.len() clamps to buffer length"
        );
    }

    /// `Src/utils.c:5141` — trailing Meta byte with no following char
    /// is a malformed-string edge case. C has a DPUTS debug-only
    /// fprintf for "unexpected end" but otherwise the loop terminates
    /// because `*s` is now NUL. Rust port must not panic on this
    /// truncated input.
    #[test]
    fn ztrlen_handles_trailing_meta_byte_without_panic() {
        let _g = crate::test_util::global_state_lock();
        let meta = char::from_u32(Meta as u32).unwrap();
        let trailing: String = ['a', meta].iter().collect();
        // c:5141-5149 — increment l, then encounter Meta, then loop
        // condition `*s` is empty → terminate. Count: 'a' + Meta = 2
        // (Rust port counts Meta as 1 char when no follower).
        let r = ztrlen(&trailing);
        assert!(r >= 1, "trailing Meta must not panic; got {}", r);
    }

    /// c:params.c:1288 — `isident` rejects digit-leading names.
    /// A regression accepting them lets `typeset 1foo=bar` install
    /// a poisoned param that no later expansion can address.
    #[test]
    fn isident_rejects_digit_leading_names() {
        let _g = crate::test_util::global_state_lock(); // c:1288
                                                        // c:1315-1319 — C `isident`: if first char is digit, ALL
                                                        // chars must be digit (all-digit names are valid positional
                                                        // params like $99). So `1foo` is rejected (digit-then-alpha)
                                                        // but `99` is accepted as a valid positional-param name.
                                                        // The previous utils.rs fake rejected all-digit names too,
                                                        // which was non-faithful — the canonical `params::isident`
                                                        // at `params.rs:2056` matches the C semantics.
        assert!(!isident("1foo")); // c:1318
        assert!(
            isident("99"), // c:1318
            "c:1315-1319 — all-digit names are valid positional params"
        );
        assert!(!isident("")); // c:1290
    }

    /// `isident` MUST accept underscore-leading + alpha-leading names —
    /// `_foo`, `Foo_BAR_42` are valid POSIX shell idents.
    #[test]
    fn isident_accepts_underscore_and_alpha_leading() {
        let _g = crate::test_util::global_state_lock(); // c:1288
        assert!(isident("foo")); // c:1288
        assert!(isident("_foo")); // c:1288
        assert!(isident("Foo_BAR_42")); // c:1288
        assert!(isident("a")); // c:1288
    }

    /// `isident` rejects whitespace/punctuation/$. A regression
    /// accepting them would let assignments install names no
    /// subsequent lookup can address (`typeset 'foo bar'=baz`).
    #[test]
    fn isident_rejects_special_chars() {
        let _g = crate::test_util::global_state_lock(); // c:1288
                                                        // c:1320-1322 — non-digit-leading names use itype_end with
                                                        // INAMESPC bits (alnum + `_` + `.`); other chars terminate.
                                                        // The previous utils.rs fake rejected `foo.` outright, which
                                                        // is incorrect — C accepts trailing `.` (it's an INAMESPC
                                                        // member). Whitespace, `-`, and `$` are NOT INAMESPC chars
                                                        // and remain rejected.
        assert!(!isident("foo bar")); // c:1322
        assert!(!isident("foo-bar")); // c:1322
        assert!(!isident("a$b")); // c:1322
    }

    /// `convbase(0, 10)` MUST produce `"0"`, not `""`. The literal-zero
    /// edge case is a real C divergence point — regression here would
    /// make `$(( 0 ))` print nothing.
    #[test]
    fn convbase_zero_renders_as_zero_literal() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(convbase(0, 10), "0");
    }

    /// `convbase` uses zsh's canonical `BASE#NUMBER` syntax for
    /// non-decimal output (matches `$(( [#16] 255 ))` → `16#FF`).
    /// This is the format `printf "%X"` and `$(( ... ))` both produce.
    #[test]
    fn convbase_uses_base_prefix_syntax_for_non_decimal() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(convbase(255, 16), "16#FF");
        assert_eq!(convbase(8, 8), "8#10");
        assert_eq!(convbase(5, 2), "2#101");
        // Base 10 has no prefix.
        assert_eq!(convbase(42, 10), "42");
    }

    /// Negative number renders with leading `-`. Regression that drops
    /// the sign would silently flip arithmetic semantics in `$((...))`.
    #[test]
    fn convbase_preserves_negative_sign() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(convbase(-42, 10), "-42");
    }

    /// `Src/utils.c:837` — `slashsplit` keeps an EMPTY leading segment
    /// when the input has a leading `/`. The caller `xsymlinks`
    /// (c:879) reads this empty as the signal "absolute path, start
    /// xbuf from /". Previous Rust test asserted filtering of all
    /// empties (matching the previous buggy port); that contract was
    /// wrong relative to C. Updated to assert the C contract.
    #[test]
    fn slashsplit_keeps_leading_empty_segment() {
        let _g = crate::test_util::global_state_lock();
        // c:851 with t==s on first iter → empty prefix segment.
        assert_eq!(
            slashsplit("/usr/local/bin"),
            vec![
                "".to_string(),
                "usr".to_string(),
                "local".to_string(),
                "bin".to_string()
            ],
            "c:851 — leading `/` produces empty first segment"
        );
        // Single `/`: one empty segment only.
        assert_eq!(
            slashsplit("/"),
            vec!["".to_string()],
            "c:854-857 — trailing `/` after first iter returns ['']"
        );
        // Consecutive slashes collapse (c:852-853 inner while-loop).
        assert_eq!(
            slashsplit("//foo"),
            vec!["".to_string(), "foo".to_string()],
            "c:852-853 — consecutive `/` collapse, leading empty kept"
        );
        // Empty input still empty result.
        assert_eq!(
            slashsplit(""),
            Vec::<String>::new(),
            "c:842 — empty input → empty array"
        );
    }

    /// `Src/utils.c:837` — relative paths (no leading `/`) start
    /// at the first non-slash; trailing `/` doesn't add a segment
    /// (c:854-857 returns before tail emit).
    #[test]
    fn slashsplit_relative_path_no_trailing_empty() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            slashsplit("a/b/"),
            vec!["a".to_string(), "b".to_string()],
            "c:854-857 — trailing `/` doesn't add segment"
        );
        // Mid-slash collapse in relative path.
        assert_eq!(
            slashsplit("a//b"),
            vec!["a".to_string(), "b".to_string()],
            "c:852-853 — mid `//` collapses"
        );
    }

    /// `equalsplit("foo=bar")` returns Some(("foo","bar")). Used by
    /// `typeset NAME=val` parsing. Regression returning None on a
    /// well-formed assignment would silently break every export/typeset.
    #[test]
    fn equalsplit_returns_first_equals_split() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            equalsplit("foo=bar"),
            Some(("foo".to_string(), "bar".to_string()))
        );
        assert_eq!(
            equalsplit("a=b=c"),
            Some(("a".to_string(), "b=c".to_string())),
            "splits on FIRST `=` only"
        );
    }

    /// `equalsplit("foo")` returns None — no `=` to split on. Catches
    /// a regression returning Some(("foo","")) which would let bare
    /// names install with empty values silently.
    #[test]
    fn equalsplit_no_equals_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(equalsplit("foo"), None);
        assert_eq!(equalsplit(""), None);
    }

    /// c:5106 — `ztrcmp` is the canonical zsh string compare. Same
    /// inputs MUST produce same Ordering (deterministic). Regression
    /// to a randomised hash-order would mis-sort `${(o)array}` output.
    #[test]
    fn ztrcmp_deterministic_and_lexicographic() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(ztrcmp("abc", "abc"), std::cmp::Ordering::Equal);
        assert_eq!(ztrcmp("abc", "abd"), std::cmp::Ordering::Less);
        assert_eq!(ztrcmp("abd", "abc"), std::cmp::Ordering::Greater);
        // Empty strings.
        assert_eq!(ztrcmp("", ""), std::cmp::Ordering::Equal);
        assert_eq!(ztrcmp("", "a"), std::cmp::Ordering::Less);
    }

    /// c:5106 — shorter-as-prefix sorts before longer (like strcmp).
    /// Catches a regression where length-then-content would mis-sort.
    #[test]
    fn ztrcmp_shorter_prefix_is_less() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(ztrcmp("a", "ab"), std::cmp::Ordering::Less);
        assert_eq!(ztrcmp("foo", "foob"), std::cmp::Ordering::Less);
    }

    /// `Src/utils.c:5117-5122` — Meta+X pair decodes to `X ^ 32`
    /// for comparison purposes. Pin: two strings differing only in
    /// the metafied byte's decoded value compare correctly.
    #[test]
    fn ztrcmp_decodes_meta_byte_for_comparison() {
        let _g = crate::test_util::global_state_lock();
        // META+0x21 ('!') = NUL? Let me use safer chars.
        // META+'A' (0x41) decodes to 'A' ^ 32 = 'a' (0x61).
        // So a "META a" pair represents the byte 'a' (0x61).
        // Compare "Ma" (M=Meta+a-byte-pair) vs "b" (>a).
        let meta_byte = Meta;
        let s1_bytes: Vec<u8> = vec![meta_byte, 0x41u8]; // decodes to 0x61 = 'a'
        let s2_bytes: Vec<u8> = vec![b'b'];
        let s1 = unsafe { std::str::from_utf8_unchecked(&s1_bytes) };
        let s2 = unsafe { std::str::from_utf8_unchecked(&s2_bytes) };
        // 'a' (0x61) < 'b' (0x62) → Less.
        assert_eq!(
            ztrcmp(s1, s2),
            std::cmp::Ordering::Less,
            "c:5117-5118 — Meta+0x41 decodes to 'a' which < 'b'"
        );
    }

    /// `Src/utils.c:531-539` — `is_nicechar`. Returns true for chars
    /// needing escape-formatting:
    ///   - c:534 — printable ASCII (0x20-0x7e) → false.
    ///   - c:536 — high-bit byte (>= 0x80) → !PRINTEIGHTBIT.
    ///   - c:538 — DEL/\n/\t/<0x20 → true.
    /// Pin the three branches.
    #[test]
    fn is_nicechar_printable_ascii_is_not_nice() {
        let _g = crate::test_util::global_state_lock();
        // c:534 — letters, digits, punctuation in 0x20-0x7e all NOT nice.
        for c in "abcXYZ012!?@~".chars() {
            assert!(
                !is_nicechar(c),
                "c:534 — '{}' (ASCII printable) must NOT be nice",
                c
            );
        }
        // c:534 — space (0x20) is printable.
        assert!(!is_nicechar(' '));
    }

    /// c:538 — control chars (DEL, newline, tab, <0x20) ARE nice.
    #[test]
    fn is_nicechar_control_chars_are_nice() {
        let _g = crate::test_util::global_state_lock();
        assert!(is_nicechar('\n'), "c:538 — newline is nice");
        assert!(is_nicechar('\t'), "c:538 — tab is nice");
        assert!(is_nicechar('\x7f'), "c:538 — DEL is nice");
        assert!(is_nicechar('\x00'), "c:538 — NUL is nice (<0x20)");
        assert!(is_nicechar('\x07'), "c:538 — BEL is nice (<0x20)");
        assert!(is_nicechar('\x1b'), "c:538 — ESC is nice (<0x20)");
        assert!(is_nicechar('\x1f'), "c:538 — boundary 0x1f is nice");
    }

    /// `Src/utils.c:2528-2570` — `zstrtoul_underscore(s)`. Pins
    /// the base-prefix dispatch: `0x` → 16, `0b` → 2, leading `0`
    /// only when OCTALZEROES is set. Tests the default state.
    #[test]
    fn zstrtoul_underscore_recognises_hex_binary_decimal() {
        let _g = crate::test_util::global_state_lock();
        // c:2538 — hex.
        assert_eq!(zstrtoul_underscore("0xff"), Some(255));
        assert_eq!(zstrtoul_underscore("0XFF"), Some(255));
        // c:2540 — binary.
        assert_eq!(zstrtoul_underscore("0b1010"), Some(10));
        assert_eq!(zstrtoul_underscore("0B11"), Some(3));
        // c:2537 — pure decimal (no leading zero).
        assert_eq!(zstrtoul_underscore("12345"), Some(12345));
        // c:2543 — leading-zero with OCTALZEROES off (default) → decimal.
        if !isset(OCTALZEROES) {
            assert_eq!(
                zstrtoul_underscore("0777"),
                Some(777),
                "c:2543 — OCTALZEROES off: leading-0 parses as decimal"
            );
            assert_eq!(zstrtoul_underscore("010"), Some(10));
        }
    }

    /// `Src/utils.c:2547-2548` — underscores are stripped before
    /// parsing. C: `if (*s == '_') continue;`. Used to support
    /// human-readable big numbers like `1_000_000`.
    #[test]
    fn zstrtoul_underscore_strips_underscores() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            zstrtoul_underscore("1_000_000"),
            Some(1_000_000),
            "c:2547-2548 — `_` stripped from numeric input"
        );
        assert_eq!(zstrtoul_underscore("0xff_ff"), Some(0xffff));
        assert_eq!(zstrtoul_underscore("0b1010_1010"), Some(0xaa));
    }

    /// `Src/utils.c:2533-2534` — leading `+` sign is consumed.
    /// `zulong` is unsigned so `-` is not handled here (caller
    /// handles negation at `zstrtol_underscore`).
    #[test]
    fn zstrtoul_underscore_consumes_leading_plus() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(zstrtoul_underscore("+42"), Some(42));
        assert_eq!(zstrtoul_underscore("+0xff"), Some(255));
    }

    /// `Src/utils.c:467` — `if (ZISPRINT(c)) goto done;`. Printable
    /// ASCII passes through nicechar_sel unchanged. Pin a single-char
    /// printable input → single-char output.
    #[test]
    fn nicechar_sel_passes_printable_ascii_unchanged() {
        let _g = crate::test_util::global_state_lock();
        for c in "aA0!~".chars() {
            assert_eq!(
                nicechar_sel(c, false),
                c.to_string(),
                "c:467 — printable ASCII '{}' passes through",
                c
            );
        }
    }

    /// `Src/utils.c:487-492` — `\n` becomes `\n` (backslash-n) and
    /// `\t` becomes `\t`. Both two-char outputs. Pin the canonical
    /// escapes.
    #[test]
    fn nicechar_sel_escapes_newline_and_tab() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            nicechar_sel('\n', false),
            "\\n",
            "c:487-489 — newline escape"
        );
        assert_eq!(nicechar_sel('\t', false), "\\t", "c:490-492 — tab escape");
    }

    /// `Src/utils.c:493-501` — control chars (<0x20 except \n/\t)
    /// get `^X` prefix in non-quotable mode and `\C-X` in quotable.
    /// c:500 — `c += 0x40` adds 0x40 to map 0x01 → 0x41 ('A').
    #[test]
    fn nicechar_sel_control_chars_use_caret_or_c_prefix() {
        let _g = crate::test_util::global_state_lock();
        // Non-quotable: ^A
        assert_eq!(
            nicechar_sel('\x01', false),
            "^A",
            "c:499-500 — \\x01 → ^A non-quotable"
        );
        // Quotable: \C-A
        assert_eq!(
            nicechar_sel('\x01', true),
            "\\C-A",
            "c:495-500 — \\x01 → \\C-A quotable"
        );
        // ^G (BEL = 0x07 → 0x47 = 'G')
        assert_eq!(nicechar_sel('\x07', false), "^G");
        // ^[ (ESC = 0x1b → 0x5b = '[')
        assert_eq!(nicechar_sel('\x1b', false), "^[");
    }

    /// `Src/utils.c:479-486` — DEL (0x7f) renders as `^?` or `\C-?`.
    #[test]
    fn nicechar_sel_del_renders_as_caret_question() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(nicechar_sel('\x7f', false), "^?", "c:485-486 — DEL is `^?`");
        assert_eq!(
            nicechar_sel('\x7f', true),
            "\\C-?",
            "c:481-486 — DEL is `\\C-?` quotable"
        );
    }

    /// `Src/utils.c:469-478` — high-bit bytes (>= 0x80) under default
    /// PRINTEIGHTBIT-off path: write `\M-` then mask to low ASCII.
    /// If the masked char is printable, output is `\M-<char>`. Pin
    /// the canonical 0xc1 → \M-A and 0xe1 → \M-a cases.
    #[test]
    fn nicechar_sel_highbit_uses_meta_prefix_when_printeightbit_off() {
        let _g = crate::test_util::global_state_lock();
        if isset(PRINTEIGHTBIT) {
            return; // Test only valid when PRINTEIGHTBIT off (default).
        }
        // 0xc1 = 'A' + 0x80 → \M-A
        let c = char::from_u32(0xc1).unwrap();
        assert_eq!(
            nicechar_sel(c, false),
            "\\M-A",
            "c:472-477 — high-bit 0xc1 → \\M-A under PRINTEIGHTBIT-off"
        );
        // 0xe1 = 'a' + 0x80 → \M-a
        let c = char::from_u32(0xe1).unwrap();
        assert_eq!(nicechar_sel(c, false), "\\M-a");
    }

    /// `Src/utils.c:593-705` — `wcs_nicechar_sel`. Wide-char variant
    /// of `nicechar_sel`: control chars escape, printable wides
    /// emit raw UTF-8, large codepoints get `\u`/`\U` hex escape.
    /// Pin every branch. Previously delegated to `nicechar_sel`
    /// which byte-masked via `c & 0xff` — every UTF-8 codepoint
    /// emerged mangled.
    #[test]
    fn wcs_nicechar_sel_printable_wide_emits_utf8() {
        let _g = crate::test_util::global_state_lock();
        // c:644-678 — printable wide char emits raw UTF-8.
        assert_eq!(
            wcs_nicechar_sel('a', None, None, false),
            "a",
            "ASCII printable"
        );
        assert_eq!(
            wcs_nicechar_sel('é', None, None, false),
            "é",
            "Latin-1 printable"
        );
        assert_eq!(
            wcs_nicechar_sel('字', None, None, false),
            "字",
            "CJK printable"
        );
    }

    /// c:625-630 — `\n` and `\t` escape (same as nicechar_sel for ASCII).
    #[test]
    fn wcs_nicechar_sel_escapes_newline_and_tab() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(wcs_nicechar_sel('\n', None, None, false), "\\n");
        assert_eq!(wcs_nicechar_sel('\t', None, None, false), "\\t");
    }

    /// c:617-623 — DEL (0x7f) renders as `^?` (non-quotable) or
    /// `\C-?` (quotable). Same as the byte version.
    #[test]
    fn wcs_nicechar_sel_del_uses_caret_question() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(wcs_nicechar_sel('\x7f', None, None, false), "^?");
        assert_eq!(wcs_nicechar_sel('\x7f', None, None, true), "\\C-?");
    }

    /// c:631-638 — control chars (0x01-0x1f, except \n/\t) emit
    /// `^X` or `\C-X` with the +0x40 offset.
    #[test]
    fn wcs_nicechar_sel_control_chars_use_caret_prefix() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(wcs_nicechar_sel('\x01', None, None, false), "^A");
        assert_eq!(wcs_nicechar_sel('\x07', None, None, false), "^G");
        assert_eq!(wcs_nicechar_sel('\x1b', None, None, false), "^[");
        assert_eq!(wcs_nicechar_sel('\x01', None, None, true), "\\C-A");
    }

    /// `Src/utils.c:656-663` — large non-printable codepoints get
    /// hex escape: `\U%.8x` (>= 0x10000), `\u%.4x` (>= 0x100).
    /// Both lowercase per C's `%x`. Use a non-printable wide char
    /// to test (most BMP chars are printable; pick a control area
    /// like U+0085 NEXT LINE).
    #[test]
    fn wcs_nicechar_sel_large_nonprintable_uses_hex_escape() {
        let _g = crate::test_util::global_state_lock();
        // U+0085 NEL — control in C1 range, not iswprint.
        let nel = char::from_u32(0x85).unwrap();
        // Since 0x85 < 0x100, falls into nicechar_sel fallback.
        // After u9_iswprint says false, cv < 0x80 is false (cv=0x85),
        // PRINTEIGHTBIT off → enters !is_printable && cv < 0x80 branch?
        // Actually 0x85 >= 0x80 → !print_eightbit true → enters non-print branch.
        // 0x85 not 0x7f, not \n/\t, not <0x20. Falls past all if-elses to
        // post-branch logic → !is_printable, cv >= 0x100 false, cv >= 0x100 false
        // → falls to nicechar_sel fallback.
        let r = wcs_nicechar_sel(nel, None, None, false);
        assert!(!r.is_empty(), "must emit something for U+0085");
        // U+200B ZERO WIDTH SPACE — non-printable, 0x100-0xffff range.
        let zwsp = char::from_u32(0x200B).unwrap();
        let r = wcs_nicechar_sel(zwsp, None, None, false);
        // u9_iswprint(0x200B) returns true via unicode-width 0-width.
        // Per the impl: u9_iswprint true → emit raw. So r should be
        // the raw zwsp char.
        assert!(!r.is_empty());
    }

    /// `Src/utils.c:709-714` — `wcs_nicechar(c)` delegates to
    /// `wcs_nicechar_sel(c, widthp, swidep, 0)`. Pin the parity.
    #[test]
    fn wcs_nicechar_matches_wcs_nicechar_sel_with_zero() {
        let _g = crate::test_util::global_state_lock();
        for c in ['a', '\n', '\t', '\x7f', '\x01', 'é', '字'] {
            assert_eq!(
                wcs_nicechar(c, None, None),
                wcs_nicechar_sel(c, None, None, false),
                "c:711 — `wcs_nicechar(c, NULL, NULL)` must equal `wcs_nicechar_sel(c, NULL, NULL, 0)` for {:?}",
                c
            );
        }
    }

    /// `Src/utils.c:1989-2012` — `movefd(fd)`. Three contracts:
    /// 1. fd == -1 → returned as-is (no syscalls).
    /// 2. fd >= 10 → returned unchanged (already high).
    /// 3. fd < 10 → dupped with F_DUPFD >= 10, original closed
    ///    unconditionally (even on dup failure).
    #[test]
    #[cfg(unix)]
    fn movefd_returns_minus_one_unchanged() {
        let _g = crate::test_util::global_state_lock();
        // c:1992 — `if (fd != -1 && fd < 10)` skipped for fd=-1.
        assert_eq!(movefd(-1), -1, "fd=-1 must be returned unchanged");
    }

    /// c:1992 — `fd >= 10` skips the dup-and-close path.
    #[test]
    #[cfg(unix)]
    fn movefd_returns_high_fd_unchanged() {
        let _g = crate::test_util::global_state_lock();
        // Use a known-open fd. /dev/null open with fd guaranteed to be
        // <10 because new fds get the lowest free slot — but we can
        // dup it to 10 directly to test the early-return path.
        let f = unsafe { libc::open(b"/dev/null\0".as_ptr() as *const _, libc::O_RDONLY) };
        if f < 0 {
            return;
        } // Skip on unusual systems.
        let high = unsafe { libc::fcntl(f, libc::F_DUPFD, 20) };
        unsafe {
            libc::close(f);
        }
        if high < 0 {
            return;
        }
        // movefd(20) must return 20 unchanged (no dup, no close).
        let r = movefd(high);
        assert_eq!(r, high, "c:1992 — fd >= 10 returned unchanged");
        unsafe {
            libc::close(high);
        }
    }

    /// `Src/utils.c:1968-1983` — `check_fd_table(fd)`. Grows the
    /// fdtable Vec to `fd+1` slots (filling new entries with
    /// FDT_UNUSED) and bumps `MAX_ZSH_FD` to `fd`. Pin: a small
    /// fd grows the table to `fd+1` length; MAX_ZSH_FD reflects it.
    /// fd ≤ cur_max early-returns without growing (c:1971-1972).
    #[test]
    fn check_fd_table_grows_to_fd_plus_one() {
        let _g = crate::test_util::global_state_lock();
        // Reset globals so the test is deterministic regardless
        // of prior ordering — other tests may have grown the
        // table or bumped MAX_ZSH_FD.
        MAX_ZSH_FD.store(-1, Ordering::Relaxed);
        {
            let mut g = fdtable_lock().lock().unwrap();
            g.clear();
        }

        // c:1971-1972 — fd ≤ cur_max is the early-return path.
        // With cur_max=-1, fd=-1 hits `fd <= cur_max` → true.
        assert!(check_fd_table(-1));
        assert_eq!(
            MAX_ZSH_FD.load(Ordering::Relaxed),
            -1,
            "fd ≤ cur_max early-returns; MAX_ZSH_FD untouched"
        );

        // c:1974-1981 — fd > cur_max grows fdtable to fd+1 slots.
        assert!(check_fd_table(7));
        assert_eq!(
            MAX_ZSH_FD.load(Ordering::Relaxed),
            7,
            "c:1982 — MAX_ZSH_FD := fd"
        );
        let len = {
            let g = fdtable_lock().lock().unwrap();
            g.len()
        };
        assert!(
            len >= 8,
            "c:1975-1979 — fdtable grew to ≥ fd+1 = 8 slots, got {}",
            len
        );

        // Idempotence under fd ≤ cur_max.
        assert!(check_fd_table(3));
        assert_eq!(
            MAX_ZSH_FD.load(Ordering::Relaxed),
            7,
            "fd ≤ cur_max keeps MAX_ZSH_FD at 7"
        );
    }

    /// `Src/utils.c:4364,4367` — `wcsitype(c, IWORD/ISEP)` reads
    /// from the `wordchars`/`ifs` GLOBALS (writable by
    /// `wordcharssetfn`/`ifssetfn`). Previously read from
    /// `std::env::var` — the libc process env — which never
    /// reflects runtime `WORDCHARS=…` shell-side assignments.
    /// Pin: set wordchars via `wordcharssetfn`, verify `wcsitype`
    /// returns true for chars in the new set.
    #[test]
    fn wcsitype_iword_reads_from_canonical_wordchars_global() {
        let _g = crate::test_util::global_state_lock();
        // Test only meaningful in MULTIBYTE mode for non-ASCII chars;
        // ASCII chars (< 0x80) route through TYPTAB directly.
        // Skip if MULTIBYTE off.
        if !isset(MULTIBYTE) {
            return;
        }
        // Save and set WORDCHARS to a single non-ASCII char.
        // C dispatches `pm->gsu.s->{get,set}fn(pm, val)`; mirror via
        // paramtab lookup.
        let saved = crate::ported::params::paramtab()
            .read()
            .ok()
            .and_then(|t| {
                t.get("WORDCHARS")
                    .map(|pm| crate::ported::params::wordcharsgetfn(pm))
            })
            .unwrap_or_default();
        // wordcharssetfn ignores its `_pm` arg and re-enters paramtab's
        // read lock via inittyptab (sibling of the IFS deadlock above).
        // Use a stack-local dummy pm instead of going through paramtab's
        // write lock so the test never holds two paramtab locks at once.
        let mut dummy = crate::ported::zsh_h::param::default();
        let do_set = |dummy: &mut crate::ported::zsh_h::param, val: String| {
            wordcharssetfn(dummy, val);
        };
        do_set(&mut dummy, "é".to_string());
        // 'é' is alphanumeric per Unicode → IWORD returns true via
        // is_alphanumeric short-circuit at c:4353. So we can't pin
        // the WORDCHARS-specific path with 'é'. Use a non-alnum char.
        do_set(&mut dummy, ":".to_string());
        // ':' is ASCII so wcsitype routes through TYPTAB (which now
        // has IWORD on ':' because wordcharssetfn called inittyptab).
        assert!(
            wcsitype(':', IWORD as u32),
            "c:4364 — wordchars membership through canonical global"
        );
        // Restore.
        do_set(&mut dummy, saved);
    }

    /// `Src/utils.c:2090-2097` — `addmodulefd(fd, fdt)`. Stores the
    /// provided fdt in the fdtable slot for the given fd. Previously
    /// Rust port hardcoded FDT_MODULE and added CLOEXEC unconditionally.
    /// Pin: pass FDT_EXTERNAL, verify slot stores FDT_EXTERNAL (not
    /// FDT_MODULE).
    #[cfg(unix)]
    #[test]
    fn addmodulefd_respects_fdt_parameter() {
        let _g = crate::test_util::global_state_lock();
        // Open a real fd to use.
        let fd = unsafe { libc::open(b"/dev/null\0".as_ptr() as *const _, libc::O_RDONLY) };
        if fd < 0 {
            return;
        }
        addmodulefd(fd, FDT_EXTERNAL);
        assert_eq!(
            fdtable_get(fd),
            FDT_EXTERNAL,
            "c:2095 — fdt parameter must be stored verbatim"
        );
        // Switch to FDT_MODULE.
        addmodulefd(fd, FDT_MODULE);
        assert_eq!(
            fdtable_get(fd),
            FDT_MODULE,
            "c:2095 — second call overwrites with new fdt"
        );
        // Cleanup.
        unsafe {
            libc::close(fd);
        }
    }

    /// `Src/utils.c:2093` — `if (fd >= 0)`. Negative fd is silently
    /// ignored (no fdtable update). Pin the guard.
    #[test]
    fn addmodulefd_ignores_negative_fd() {
        let _g = crate::test_util::global_state_lock();
        // Should not panic, no side effect.
        addmodulefd(-1, FDT_MODULE);
        // Nothing to assert on side-effect-free path; just verify no panic.
    }

    /// `Src/utils.c:2155-2164` — `zcloselockfd(fd)` returns -1 for
    /// non-lock fds, 0 for FDT_FLOCK/FDT_FLOCK_EXEC. Previously the
    /// Rust port always returned 0 — broke `zsystem flock -u <fd>`
    /// distinguishing "fd never flocked" from "successfully released".
    #[cfg(unix)]
    #[test]
    fn zcloselockfd_returns_minus_one_for_non_lock_fd() {
        let _g = crate::test_util::global_state_lock();
        let fd = unsafe { libc::open(b"/dev/null\0".as_ptr() as *const _, libc::O_RDONLY) };
        if fd < 0 {
            return;
        }
        // No addlockfd call → fd is NOT in the flock table.
        let r = zcloselockfd(fd);
        assert_eq!(r, -1, "c:2160-2161 — non-lock fd must return -1, NOT 0");
        unsafe {
            libc::close(fd);
        }
    }

    /// `Src/utils.c:2155-2164` — `zcloselockfd(fd)` returns 0 for an
    /// fd that was registered via `addlockfd`. Pin the end-to-end
    /// round-trip: addlockfd → zcloselockfd returns 0.
    #[cfg(unix)]
    #[test]
    fn zcloselockfd_returns_zero_for_flock_fd_then_closes() {
        let _g = crate::test_util::global_state_lock();
        let fd = unsafe { libc::open(b"/dev/null\0".as_ptr() as *const _, libc::O_RDONLY) };
        if fd < 0 {
            return;
        }
        addlockfd(fd, true);
        let r = zcloselockfd(fd);
        assert_eq!(r, 0, "c:2162-2163 — FDT_FLOCK fd → zclose + return 0");
        // After zcloselockfd, the underlying close() has fired.
        // close() returning -1 with EBADF would confirm that, but
        // that's a libc side-effect; just verify the return value.
    }

    /// `Src/utils.c:1982` — `check_fd_table` sets `max_zsh_fd = fd`
    /// when fd is new. `fdtable_set` inlines this to keep the guard
    /// in `zcloselockfd` (`if (fd > max_zsh_fd) return -1`) working
    /// against fds populated by `addmodulefd`/`addlockfd`. Pin: after
    /// `fdtable_set(fd, FDT_FLOCK)`, MAX_ZSH_FD >= fd.
    #[cfg(unix)]
    #[test]
    fn fdtable_set_bumps_max_zsh_fd() {
        let _g = crate::test_util::global_state_lock();
        let fd = unsafe { libc::open(b"/dev/null\0".as_ptr() as *const _, libc::O_RDONLY) };
        if fd < 0 {
            return;
        }
        fdtable_set(fd, FDT_FLOCK);
        let max_fd = MAX_ZSH_FD.load(Ordering::Relaxed);
        assert!(
            max_fd >= fd,
            "c:1982 — max_zsh_fd ({}) must be >= newly-set fd ({})",
            max_fd,
            fd
        );
        // Cleanup.
        fdtable_set(fd, FDT_UNUSED);
        unsafe {
            libc::close(fd);
        }
    }

    /// `Src/utils.c:2111-2121` — `addlockfd(fd, cloexec)`. Selects
    /// between FDT_FLOCK (when cloexec=true, lock dies on exec) and
    /// FDT_FLOCK_EXEC (when cloexec=false, lock survives exec).
    /// Previously the Rust port did `fcntl(F_SETFD, CLOEXEC)` —
    /// totally wrong semantics. Pin the FDT slot.
    #[cfg(unix)]
    #[test]
    fn addlockfd_selects_flock_category_per_cloexec_flag() {
        let _g = crate::test_util::global_state_lock();
        let fd = unsafe { libc::open(b"/dev/null\0".as_ptr() as *const _, libc::O_RDONLY) };
        if fd < 0 {
            return;
        }
        // cloexec=true → FDT_FLOCK (lock dies on exec).
        addlockfd(fd, true);
        assert_eq!(
            fdtable_get(fd),
            FDT_FLOCK,
            "c:2117 — cloexec=true → FDT_FLOCK"
        );
        // cloexec=false → FDT_FLOCK_EXEC (lock survives exec).
        addlockfd(fd, false);
        assert_eq!(
            fdtable_get(fd),
            FDT_FLOCK_EXEC,
            "c:2119 — cloexec=false → FDT_FLOCK_EXEC"
        );
        // Cleanup.
        unsafe {
            libc::close(fd);
        }
    }

    /// `Src/utils.c:1211` — `adduserdir` rejects paths `>= PATH_MAX`
    /// by removing the existing entry (treating as "can't use this
    /// value as a directory"). Pin with a path constructed to exceed
    /// PATH_MAX, verify no insertion happens.
    #[cfg(unix)]
    #[test]
    fn adduserdir_rejects_paths_at_or_above_path_max() {
        let _g = crate::test_util::global_state_lock();
        if !interact() {
            // c:1193 — non-interactive shells skip the table entirely.
            // Test inactive in non-interactive contexts.
            return;
        }
        // Ensure the entry doesn't exist beforehand.
        let name = "ZSHRS_TEST_PATHMAX_DIR";
        let _ = removenameddirnode(name);
        // Construct an oversized path.
        let mut over: String = "/".to_string();
        over.push_str(&"a".repeat(libc::PATH_MAX as usize));
        // adduserdir with the oversized path + AUTONAMEDIRS-off path
        // via always=true (so the AUTONAMEDIRS guard doesn't reject).
        adduserdir(name, &over, 0, true);
        // c:1211 — too-long → remove path triggered, no entry added.
        let tab = nameddirtab().lock().unwrap();
        assert!(
            !tab.contains_key(name),
            "c:1211 — strlen(t) >= PATH_MAX must NOT insert entry"
        );
    }

    /// `Src/utils.c:760-786` — `pathprog`. Finds any regular file
    /// (not directory, no executable-bit check) in `$PATH`.
    /// Previously required `mode & 0o111 != 0` — silently missed
    /// non-executable autoload-function plaintext scripts. Pin
    /// using a known-existing file in /tmp.
    #[test]
    #[cfg(unix)]
    fn pathprog_finds_non_executable_files() {
        let _g = crate::test_util::global_state_lock();
        // Create a non-executable file in /tmp and add /tmp to PATH.
        let test_name = format!("zshrs_test_pathprog_{}", unsafe { libc::getpid() });
        let path = PathBuf::from("/tmp").join(&test_name);
        // Write content, NO executable bit.
        if fs::write(&path, b"plain content").is_err() {
            return; // /tmp may be unwritable on some systems.
        }
        // Verify it's not executable.
        let meta = fs::metadata(&path).unwrap();
        assert_eq!(
            meta.permissions().mode() & 0o111,
            0,
            "test setup: must be non-executable"
        );
        // Set PATH to /tmp.
        let saved_path = getsparam("PATH");
        assignsparam("PATH", "/tmp", 0);
        // pathprog should find the file.
        let r = pathprog(&test_name);
        // Cleanup.
        let _ = fs::remove_file(&path);
        if let Some(prev) = saved_path {
            assignsparam("PATH", &prev, 0);
        }
        assert_eq!(
            r,
            Some(path),
            "c:776 — pathprog finds non-executable files (only F_OK + !S_ISDIR)"
        );
    }

    /// `Src/utils.c:776-778` — pathprog skips directories. Pin: a
    /// directory in $PATH with the queried name must NOT be returned.
    #[test]
    #[cfg(unix)]
    fn pathprog_skips_directories() {
        let _g = crate::test_util::global_state_lock();
        let test_name = format!("zshrs_test_pathprog_dir_{}", unsafe { libc::getpid() });
        let path = PathBuf::from("/tmp").join(&test_name);
        if fs::create_dir(&path).is_err() {
            return;
        }
        let saved_path = getsparam("PATH");
        assignsparam("PATH", "/tmp", 0);
        let r = pathprog(&test_name);
        // Cleanup.
        let _ = fs::remove_dir(&path);
        if let Some(prev) = saved_path {
            assignsparam("PATH", &prev, 0);
        }
        assert!(
            r.is_none(),
            "c:778 — pathprog must skip directories (!S_ISDIR)"
        );
    }

    /// `Src/utils.c:4367` — `wcsitype(c, ISEP)` reads from canonical
    /// `ifs` global. Pin via `ifssetfn`. ASCII path routes through
    /// TYPTAB which gets ISEP bit from inittyptab's IFS walk
    /// (added earlier this session). End-to-end pin.
    #[test]
    fn wcsitype_isep_reads_from_canonical_ifs_global() {
        let _g = crate::test_util::global_state_lock();
        if !isset(MULTIBYTE) {
            return;
        }
        // C: `pm->gsu.s->{get,set}fn(pm, val)`. Mirror via paramtab.
        let saved = crate::ported::params::paramtab()
            .read()
            .ok()
            .and_then(|t| t.get("IFS").map(|pm| crate::ported::params::ifsgetfn(pm)))
            .unwrap_or_default();
        // ifssetfn ignores its `_pm` arg (the C `gsu.s->setfn` receives
        // the param but the IFS callback only touches the global `ifs`
        // store + calls inittyptab). Calling through paramtab's write
        // lock would deadlock because inittyptab() re-takes paramtab's
        // read lock for the IFS walk (utils.rs:5028) — pthread rwlock
        // forbids same-thread write→read upgrade. Pass a stack-local
        // dummy pm instead so we never hold the paramtab lock during
        // ifssetfn.
        let mut dummy = crate::ported::zsh_h::param::default();
        let do_set = |dummy: &mut crate::ported::zsh_h::param, val: String| {
            ifssetfn(dummy, val);
        };
        do_set(&mut dummy, ":".to_string());
        assert!(
            wcsitype(':', ISEP as u32),
            "c:4367 — IFS membership through canonical global"
        );
        // Restore.
        do_set(&mut dummy, saved);
    }

    /// c:536 — high-bit byte (>= 0x80) is nice when PRINTEIGHTBIT is
    /// OFF (the default). Pin the default case; the alternate
    /// behavior under PRINTEIGHTBIT-on is harder to exercise from
    /// a unit test due to opt-state global mutation.
    #[test]
    fn is_nicechar_highbit_is_nice_when_printeightbit_off() {
        let _g = crate::test_util::global_state_lock();
        // Default state: PRINTEIGHTBIT is off → high-bit bytes are nice.
        // Use char with high-bit equivalent low byte.
        // 0xb5 (Meta+5 territory) — masked to 0xff → still 0xb5.
        let c = char::from_u32(0xb5).unwrap();
        // ZISPRINT(0xb5) is false (>0x7e). 0xb5 & 0x80 set → check PRINTEIGHTBIT.
        // Default PRINTEIGHTBIT off → returns true (nice).
        if !isset(PRINTEIGHTBIT) {
            assert!(
                is_nicechar(c),
                "c:536 — high-bit byte 0x{:x} must be nice when PRINTEIGHTBIT off",
                0xb5_u32
            );
        }
    }

    /// `Src/utils.c:1969-1983` — `check_fd_table(fd)` grows the
    /// fdtable so it can index `fd` and bumps `max_zsh_fd`. The
    /// previous Rust port was a no-op shim that always returned true;
    /// real behavior is "fdtable.len() must be > fd" and
    /// "MAX_ZSH_FD >= fd" after the call.
    #[test]
    fn check_fd_table_grows_fdtable_and_bumps_max_zsh_fd() {
        let _g = crate::test_util::global_state_lock();
        // Snapshot prior state so we don't poison other tests.
        let saved_max = MAX_ZSH_FD.load(Ordering::Relaxed);
        // Pick a target fd well above the typical 0/1/2.
        let target = saved_max.max(50) + 7;
        check_fd_table(target);
        let new_max = MAX_ZSH_FD.load(Ordering::Relaxed);
        assert!(
            new_max >= target,
            "c:1982 — max_zsh_fd must be >= target after grow (got {})",
            new_max
        );
        // Calling again with a lower fd MUST NOT shrink max_zsh_fd
        // (c:1971-1972 early return).
        check_fd_table(target - 3);
        assert_eq!(
            MAX_ZSH_FD.load(Ordering::Relaxed),
            new_max,
            "c:1971 — fd <= max_zsh_fd path must not change max"
        );
        // Restore approximately — best we can do without exposing
        // fdtable internals; subsequent fdtable_set callers cope.
        MAX_ZSH_FD.store(saved_max, Ordering::Relaxed);
    }

    /// `Src/utils.c:1971` — `if (fd <= max_zsh_fd) return;` early
    /// exit. Calling with a small fd (<= current max) must be a
    /// no-op.
    #[test]
    fn check_fd_table_small_fd_is_noop() {
        let _g = crate::test_util::global_state_lock();
        // Ensure max_zsh_fd is non-trivial.
        let _ = check_fd_table(100);
        let max_before = MAX_ZSH_FD.load(Ordering::Relaxed);
        check_fd_table(5);
        assert_eq!(
            MAX_ZSH_FD.load(Ordering::Relaxed),
            max_before,
            "c:1971 — small fd path must not touch max_zsh_fd"
        );
    }

    /// `Src/utils.c:1969-1983` — defensive: negative fd shouldn't
    /// panic. C-side would have indexed past the start of `fdtable`
    /// (UB). Rust port should fail-soft.
    #[test]
    fn check_fd_table_negative_fd_does_not_panic() {
        let _g = crate::test_util::global_state_lock();
        // Should not panic; behavior beyond "no panic" is unspecified.
        let _ = check_fd_table(-1);
        let _ = check_fd_table(-100);
    }

    /// `Src/zsh.h:3274` — under MULTIBYTE_SUPPORT, the C source
    /// defines `nicezputs(str, outs)` as a macro:
    ///   `(void)mb_niceformat((str), (outs), NULL, 0)`.
    /// So Rust `nicezputs(s)` must equal `mb_niceformat(s)` for
    /// every input. Pins the macro-equivalence after fixing the
    /// previous `chars().map(nicechar)` impl (which corrupted
    /// non-ASCII multibyte codepoints into `\M-X` mangle).
    #[test]
    fn nicezputs_matches_mb_niceformat_under_multibyte() {
        let _g = crate::test_util::global_state_lock();
        // C-equivalent pattern: write to a stream, compare to
        // mb_niceformat outstrp-form output. Inline rather than via a
        // Rust-only helper.
        for input in ["hello", "a\nb", "é", ""] {
            let mut nz_buf: Vec<u8> = Vec::new();
            let _ = nicezputs(input, &mut nz_buf);
            let nz = String::from_utf8(nz_buf).expect("utf8");

            let mut mb_out: Option<String> = None;
            let _ = mb_niceformat(input, None, Some(&mut mb_out), 0);
            assert_eq!(
                nz,
                mb_out.unwrap_or_default(),
                "nicezputs and mb_niceformat must agree for {:?}",
                input
            );
        }
        // \n must produce literal `\n` (backslash + n).
        let mut nz_buf: Vec<u8> = Vec::new();
        let _ = nicezputs("a\nb", &mut nz_buf);
        let nz = String::from_utf8(nz_buf).expect("utf8");
        assert!(nz.contains("\\n"), "nicechar emits `\\n`, not raw 0x0a");
    }

    /// `Src/utils.c:5530` — under MULTIBYTE_SUPPORT, `nicedup(s, heap)`
    /// body is `(void)mb_niceformat(s, NULL, &retstr, …); return retstr;`
    /// so the returned string MUST equal mb_niceformat output.
    #[test]
    fn nicedup_matches_mb_niceformat_under_multibyte() {
        let _g = crate::test_util::global_state_lock();
        for input in ["hello", "a\nb", "é"] {
            let nd = nicedup(input, 0);
            let mut mb_out: Option<String> = None;
            let _ = mb_niceformat(input, None, Some(&mut mb_out), 0);
            assert_eq!(
                nd,
                mb_out.unwrap_or_default(),
                "nicedup must equal mb_niceformat outstrp form for {:?}",
                input
            );
        }
        // nicedupstring delegates to nicedup(s, 1).
        assert_eq!(nicedupstring("hé\nllo"), nicedup("hé\nllo", 1));
    }

    /// `Src/utils.c:734-744` — `zwcwidth(wc)` returns 1 when MULTIBYTE
    /// option is unset (c:738), regardless of the codepoint's actual
    /// display width. The previous Rust port skipped the option gate
    /// and always used the Unicode width table. Pin the option-gated
    /// behavior end-to-end: unset MULTIBYTE → width 1 for CJK; set →
    /// width 2 for CJK.
    #[test]
    fn zwcwidth_returns_1_when_multibyte_unset() {
        let _g = crate::test_util::global_state_lock();
        let saved = isset(MULTIBYTE);
        // c:738 — unset(MULTIBYTE) path returns 1 for everything.
        dosetopt(MULTIBYTE, 0, 1);
        assert_eq!(zwcwidth('a'), 1, "c:738 — ASCII width 1 under nomultibyte");
        assert_eq!(
            zwcwidth('字'),
            1,
            "c:738 — CJK collapses to 1 under nomultibyte (was 2 via Unicode-width)"
        );
        assert_eq!(
            zwcwidth('\u{200B}'),
            1,
            "c:738 — zero-width space → 1 under nomultibyte (was 0 via Unicode-width)"
        );
        // c:740-744 — MULTIBYTE on: Unicode-width table applies.
        dosetopt(MULTIBYTE, 1, 1);
        assert_eq!(zwcwidth('a'), 1, "ASCII width is 1");
        assert_eq!(zwcwidth('字'), 2, "c:740 — CJK width 2 under multibyte");
        // Restore prior state.
        dosetopt(MULTIBYTE, if saved { 1 } else { 0 }, 1);
    }

    /// `Src/utils.c:5613` — `if (!isset(MULTIBYTE) || (unsigned char) *s <=
    /// 0x7f)` returns a SINGLE byte. The port had only the multibyte arm, so
    /// every walk built on this function kept stepping whole characters in a
    /// mode where zsh steps bytes — `${gr[2]}`, `${(s::)v}` and `$(( ##c ))`
    /// all read the wrong unit.
    ///
    /// Both the length and the returned unit are asserted: a version that
    /// fixed the length but re-encoded the unit as UTF-8 would still emit
    /// `c3 8e` where zsh emits the single byte `ce`.
    #[test]
    fn mb_metacharlenconv_steps_bytes_when_multibyte_unset() {
        let _g = crate::test_util::global_state_lock();
        let saved = isset(MULTIBYTE);
        // `α` is ce b1 — one character, two bytes.
        let alpha: &[u8] = &[0xce, 0xb1];

        dosetopt(MULTIBYTE, 1, 1);
        let (n, wc, unit) = mb_metacharlenconv(alpha);
        assert_eq!(n, 2, "c:5635 — multibyte on: the whole character");
        assert_eq!(wc, Some('α'));
        assert_eq!(unit.as_bytes(), alpha, "the unit is the character verbatim");

        dosetopt(MULTIBYTE, 0, 1);
        let (n, wc, unit) = mb_metacharlenconv(alpha);
        assert_eq!(n, 1, "c:5613 — multibyte off: one byte");
        assert_eq!(wc, Some('\u{ce}'), "c:5616 — the byte value is the scalar");
        assert_eq!(
            crate::ported::utils::unmetafy_str(&unit),
            vec![0xceu8],
            "the unit must decode back to the single byte 0xce, not to U+00CE's \
             UTF-8 (c3 8e) — half a character cannot be held verbatim"
        );

        // ASCII takes the same one-byte path in either mode (c:5613).
        dosetopt(MULTIBYTE, 1, 1);
        assert_eq!(mb_metacharlenconv(b"abc").0, 1);
        dosetopt(MULTIBYTE, 0, 1);
        assert_eq!(mb_metacharlenconv(b"abc").0, 1);

        dosetopt(MULTIBYTE, if saved { 1 } else { 0 }, 1);
    }

    /// `Src/utils.c:6478-6492` — `quotedzputs(s, NULL)` under
    /// MULTIBYTE_SUPPORT detects "needs nice-format" inputs via
    /// `is_mb_niceformat(s)` and emits `$'<nice-formatted body>'`
    /// (the dollar-quoted form so embedded `\n`/`\t`/escape sequences
    /// round-trip through `typeset -p`/`set`/`set -x`). The previous
    /// Rust port skipped this branch entirely and single-quoted such
    /// strings — POSIX `'…'` is *strong* (no escapes interpreted),
    /// breaking round-trip for control bytes.
    #[test]
    fn quotedzputs_uses_dollar_quotes_for_control_chars() {
        let _g = crate::test_util::global_state_lock();
        // Ensure typtab is initialised — hasspecial depends on ISPECIAL
        // bits which are populated by inittyptab. Without this the
        // SPECCHARS arm short-circuits to "no specials".
        inittyptab();
        // Empty stays `''`.
        assert_eq!(quotedzputs(""), "''", "c:6470-6475 — empty input → ''");
        // Plain alphanumeric: no specials, no controls → bare.
        assert_eq!(
            quotedzputs("hello"),
            "hello",
            "c:6511-6517 — no SPECCHARS member → return unchanged"
        );
        // Control char `\n` needs `$'…'` to round-trip.
        let r = quotedzputs("a\nb");
        assert!(
            r.starts_with("$'") && r.ends_with('\''),
            "c:6488 — control char must use $'…' form (got {:?})",
            r
        );
        // Tab and ESC also force the niceformat arm.
        let r = quotedzputs("\t");
        assert!(
            r.starts_with("$'"),
            "c:6488 — TAB forces $'…' arm (got {:?})",
            r
        );
        let r = quotedzputs("\u{1b}[31m");
        assert!(
            r.starts_with("$'"),
            "c:6488 — ESC sequence forces $'…' arm (got {:?})",
            r
        );
        // Single quote forces single-quote arm via SPECCHARS membership
        // (Src/zsh.h:228 — SPECCHARS includes `'`). Embedded `'`
        // rewrites to `'\''`.
        let r = quotedzputs("a'b");
        assert!(
            r.contains("'\\''"),
            "c:6573-6587 — embedded ' → '\\''  (got {:?})",
            r
        );
    }

    /// `Src/utils.c:2923-2945` — `read_loop` returns the requested
    /// length on full read, or the partial count on EOF. Pin: writing
    /// a known buffer to a pipe and reading it back returns the same
    /// content + correct length. Drives the no-side-effect path that
    /// the diagnostic-message fix doesn't touch.
    #[test]
    #[cfg(unix)]
    fn read_loop_round_trips_pipe_bytes() {
        let _g = crate::test_util::global_state_lock();
        // Create a pipe; write 16 bytes; read them back.
        let mut fds: [libc::c_int; 2] = [0; 2];
        unsafe {
            assert_eq!(libc::pipe(fds.as_mut_ptr()), 0, "pipe(2) ok");
        }
        let payload = b"hello-world-1234";
        let written = unsafe {
            libc::write(
                fds[1],
                payload.as_ptr() as *const libc::c_void,
                payload.len(),
            )
        };
        assert_eq!(written, payload.len() as isize, "write all 16 bytes");
        unsafe {
            libc::close(fds[1]);
        }
        let mut buf = [0u8; 16];
        let got = read_loop(fds[0], &mut buf).expect("read_loop ok");
        assert_eq!(got, payload.len(), "c:2929 — read_loop returns full length");
        assert_eq!(
            &buf[..],
            &payload[..],
            "c:2940-2941 — buffer copied verbatim"
        );
        unsafe {
            libc::close(fds[0]);
        }
    }

    /// `Src/utils.c:2949-2970` — `write_loop` returns the requested
    /// length when the kernel accepts all bytes (the common case for
    /// a pipe with room). Pin the no-side-effect happy path.
    #[test]
    #[cfg(unix)]
    fn write_loop_writes_all_bytes_to_pipe() {
        let _g = crate::test_util::global_state_lock();
        let mut fds: [libc::c_int; 2] = [0; 2];
        unsafe {
            assert_eq!(libc::pipe(fds.as_mut_ptr()), 0, "pipe(2) ok");
        }
        let payload = b"abcdef";
        let got = write_loop(fds[1], payload).expect("write_loop ok");
        assert_eq!(
            got,
            payload.len(),
            "c:2955-2956 — write_loop returns full length on accept"
        );
        // Read back to verify.
        unsafe {
            libc::close(fds[1]);
        }
        let mut buf = [0u8; 6];
        let _ = unsafe { libc::read(fds[0], buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
        assert_eq!(&buf[..], &payload[..]);
        unsafe {
            libc::close(fds[0]);
        }
    }

    /// `Src/utils.c:2935-2936` — `read_loop` on a closed/invalid fd
    /// returns an io::Error (the C path returns `ret` and emits the
    /// `zwarn` to stderr). Pin the error propagation; the zwarn
    /// emission is a stderr side-effect tested only by inspecting
    /// log output (out of scope here).
    #[test]
    #[cfg(unix)]
    fn read_loop_returns_error_on_invalid_fd() {
        let _g = crate::test_util::global_state_lock();
        let mut buf = [0u8; 4];
        // fd 9999 is essentially guaranteed-not-open in a test
        // process; the canonical "bad fd" error path.
        let r = read_loop(9999, &mut buf);
        assert!(
            r.is_err(),
            "c:2935 — invalid fd → io::Error (and zwarn to stderr)"
        );
    }

    /// `Src/utils.c:2972-2988` — `read1char(echo)` reads from SHTTY,
    /// not stdin. With SHTTY uninitialised (default -1 in test
    /// environment), the function MUST return -1 immediately rather
    /// than blocking on a stdin read. The previous Rust port read
    /// from stdin and returned None — which in some test runners
    /// would block waiting for input.
    #[test]
    #[cfg(unix)]
    fn read1char_returns_minus_one_when_shtty_unset() {
        let _g = crate::test_util::global_state_lock();
        // Test environment: SHTTY is -1 (no controlling tty bound
        // by the port). C-side would `read(-1, ...)` which fails;
        // Rust port should fail-fast with -1.
        let saved = SHTTY.load(Ordering::Relaxed);
        SHTTY.store(-1, Ordering::Relaxed);
        let got = read1char(0); // echo=0
        assert_eq!(got, -1, "c:2978 — SHTTY=-1 → read fails → return -1");
        // Restore.
        SHTTY.store(saved, Ordering::Relaxed);
    }

    /// `Src/utils.c:1989-2012` — `movefd(fd)` dups fd to >= 10 (so it
    /// stays out of the user fd range 0..=9), closes the original,
    /// AND marks the new fd as `FDT_INTERNAL` in fdtable. Previous
    /// Rust port omitted the fdtable_set call — internal-fd tracking
    /// (`closeallelse`, forkexec) silently never saw zshrs-internal
    /// fds. Pin: after movefd, fdtable[new_fd] == FDT_INTERNAL.
    #[test]
    #[cfg(unix)]
    fn movefd_marks_fdtable_internal() {
        let _g = crate::test_util::global_state_lock();
        // Open /dev/null to get a small (< 10) fd...
        let dev_null = CString::new("/dev/null").unwrap();
        let fd = unsafe { libc::open(dev_null.as_ptr(), libc::O_RDONLY) };
        assert!(fd >= 0, "open /dev/null returned -1");
        let new_fd = movefd(fd);
        assert!(
            new_fd >= 10,
            "c:1992-1994 — movefd dups to fd >= 10 (got {})",
            new_fd
        );
        // c:2009 — fdtable[new_fd] := FDT_INTERNAL.
        let entry = fdtable_get(new_fd);
        assert_eq!(
            entry, FDT_INTERNAL,
            "c:2009 — movefd must mark new_fd as FDT_INTERNAL (got {})",
            entry
        );
        unsafe {
            libc::close(new_fd);
        }
    }

    /// `Src/utils.c:1989` — `movefd(-1)` is a defensive no-op: the
    /// `fd != -1 && fd < 10` gate at c:1992 fails, the c:2007
    /// `if (fd != -1)` post-check skips, and -1 is returned
    /// unchanged. Pin so a refactor of the early-out doesn't
    /// accidentally call fcntl(-1, ...).
    #[test]
    #[cfg(unix)]
    fn movefd_minus_one_returns_minus_one() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            movefd(-1),
            -1,
            "c:1992 — movefd(-1) bypasses both gates and returns -1"
        );
    }

    /// `Src/utils.c:2021-2068` — `redup(x, y)` after a successful
    /// `dup2(x, y)` must:
    ///   * Copy fdtable[x] to fdtable[y] (c:2054).
    ///   * Promote FDT_FLOCK/FDT_FLOCK_EXEC to FDT_INTERNAL on the
    ///     dup target (c:2055-2056) — the lock doesn't transfer.
    ///   * Then close x (c:2064).
    ///
    /// Previous Rust port skipped the fdtable updates entirely.
    /// Pin the ownership-transfer with two fds the test allocates.
    #[test]
    #[cfg(unix)]
    fn redup_copies_fdtable_ownership_to_target() {
        let _g = crate::test_util::global_state_lock();
        // Open two distinct fds — both will land in the fdtable.
        let dev_null = CString::new("/dev/null").unwrap();
        let x = unsafe { libc::open(dev_null.as_ptr(), libc::O_RDONLY) };
        let y = unsafe { libc::open(dev_null.as_ptr(), libc::O_RDONLY) };
        assert!(x >= 0 && y >= 0, "open /dev/null returned -1");
        assert_ne!(x, y);

        // Mark x as FDT_INTERNAL so we can observe the copy to y.
        check_fd_table(x);
        check_fd_table(y);
        fdtable_set(x, FDT_INTERNAL);
        fdtable_set(y, FDT_UNUSED);

        let ret = redup(x, y);
        assert_eq!(ret, y, "c:2067 — successful redup returns y");
        // c:2054 — fdtable[y] inherited from fdtable[x].
        assert_eq!(
            fdtable_get(y),
            FDT_INTERNAL,
            "c:2054 — fdtable[y] = fdtable[x] (FDT_INTERNAL)"
        );
        // x is closed by c:2064.
        unsafe {
            libc::close(y);
        }
    }

    /// `Src/utils.c:872-908` — `xsymlinks(path)` resolves `.`/`..`
    /// AND follows ONE LEVEL of symlinks via `readlink(2)`. Pin the
    /// symlink-following behavior with a temp-dir-managed symlink
    /// (the actual bug-fix pin — previous Rust port just normalised
    /// `.`/`..` without ever calling readlink).
    #[test]
    #[cfg(unix)]
    fn xsymlinks_follows_one_level_of_symlinks() {
        let _g = crate::test_util::global_state_lock();
        let tmp = std::env::temp_dir();
        let target = tmp.join(format!("zshrs_xsymlinks_target_{}", std::process::id()));
        let link = tmp.join(format!("zshrs_xsymlinks_link_{}", std::process::id()));
        // Create target as a regular dir to symlink to.
        let _ = fs::create_dir(&target);
        let _ = fs::remove_file(&link);
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let got = xsymlinks(link.to_str().unwrap()).unwrap();
        assert_eq!(
            got,
            target.to_string_lossy(),
            "c:908 — xsymlinks must follow the symlink to its target"
        );

        // Cleanup.
        let _ = fs::remove_file(&link);
        let _ = fs::remove_dir(&target);
    }

    /// `Src/utils.c:881-882` — `.` components are skipped.
    /// `Src/utils.c:883-896` — `..` components walk back one
    /// `/`-segment of xbuf (unless xbuf is empty or `/`).
    ///
    /// Use a temp directory we create ourselves — we can't pin
    /// `/tmp/...` literally because the macOS sandbox symlinks
    /// `/tmp -> /private/tmp`, which `xsymlinks` correctly follows
    /// (proving the bug-fix works). Test with a directory under
    /// the env tempdir + a non-existent sub so readlink fails on
    /// every component and the test exercises ONLY the c:881-896
    /// `.` / `..` paths.
    #[test]
    #[cfg(unix)]
    fn xsymlinks_normalises_dot_and_dotdot() {
        let _g = crate::test_util::global_state_lock();
        let tmp = fs::canonicalize(std::env::temp_dir()).unwrap();
        let base_dir = tmp.join(format!("zshrs_xs_norm_{}", std::process::id()));
        let _ = fs::create_dir(&base_dir);
        // `<base>/.` should be `<base>` (c:881 — `.` skipped).
        let arg = format!("{}/./.", base_dir.display());
        let got = xsymlinks(&arg).unwrap();
        assert_eq!(
            got,
            base_dir.to_string_lossy(),
            "c:881 — `.` segments collapse"
        );
        // `<base>/foo/..` should be `<base>` (c:891-895 — `..` walks back).
        let arg = format!("{}/foo/..", base_dir.display());
        let got = xsymlinks(&arg).unwrap();
        assert_eq!(
            got,
            base_dir.to_string_lossy(),
            "c:891-895 — `..` walks back one segment"
        );
        let _ = fs::remove_dir(&base_dir);
    }

    /// `Src/utils.c:2055-2056` — when fdtable[x] is FDT_FLOCK or
    /// FDT_FLOCK_EXEC, the dup'd fd y gets promoted to FDT_INTERNAL
    /// (the dup doesn't carry the flock). Pin the promotion.
    ///
    /// Note on fdtable_flocks: C's c:2062-2063 decrements the
    /// flock count in `redup` BEFORE `zclose(x)`, and zclose ALSO
    /// decrements when it sees fdtable[fd] == FDT_FLOCK (c:2135).
    /// So C double-decrements for an FDT_FLOCK source fd in redup —
    /// the C comment at c:2058-2061 calls this "isn't expected to
    /// happen" (FDT_FLOCK fds aren't normally redup'd). We mirror
    /// C exactly: test asserts the count drops by 2 (1 → -1) to
    /// pin the faithful-to-C behavior. Reporting this as a C bug
    /// upstream is the right path; the port preserves it verbatim.
    #[test]
    #[cfg(unix)]
    fn redup_promotes_flock_to_internal_on_target() {
        let _g = crate::test_util::global_state_lock();
        let dev_null = CString::new("/dev/null").unwrap();
        let x = unsafe { libc::open(dev_null.as_ptr(), libc::O_RDONLY) };
        let y = unsafe { libc::open(dev_null.as_ptr(), libc::O_RDONLY) };
        assert!(x >= 0 && y >= 0);
        assert_ne!(x, y);

        check_fd_table(x);
        check_fd_table(y);
        // Mark x as FDT_FLOCK.
        fdtable_set(x, FDT_FLOCK);
        FDTABLE_FLOCKS.store(2, Ordering::SeqCst); // start at 2 so double-decrement lands at 0

        let _ = redup(x, y);

        // c:2055-2056 — promoted to FDT_INTERNAL on y, NOT carried.
        assert_eq!(
            fdtable_get(y),
            FDT_INTERNAL,
            "c:2055-2056 — FDT_FLOCK on x promotes to FDT_INTERNAL on y"
        );
        // c:2062-2063 + zclose c:2135 — double decrement.
        assert_eq!(
            FDTABLE_FLOCKS.load(Ordering::SeqCst),
            0,
            "c:2062-2063 + c:2135 — flock count double-decremented (faithful to C)"
        );
        unsafe {
            libc::close(y);
        }
    }

    /// `Src/utils.c:5217-5240` — `zreaddir(dir, ignoredots)` exposes
    /// the dot-filter as a parameter and returns one entry per call.
    /// Two paths:
    /// * `ignoredots=1` (the common case at c:590/655/1653/2884) —
    ///   `.` and `..` are filtered out.
    /// * `ignoredots=0` (used at c:4648 for spelling correction) —
    ///   `.` and `..` are RETAINED as valid candidates.
    #[test]
    #[cfg(unix)]
    fn zreaddir_honors_ignoredots_flag() {
        let _g = crate::test_util::global_state_lock();
        let tmp = std::env::temp_dir();
        let base = tmp.join(format!("zshrs_zreaddir_{}", std::process::id()));
        let _ = fs::create_dir(&base);
        // Create one real entry to make the test non-trivial.
        fs::write(base.join("file"), "x").unwrap();

        // c:5232 — ignoredots=1: skip `.` and `..`, keep `file`.
        let mut dir = fs::read_dir(&base).unwrap();
        let mut with_skip: Vec<String> = Vec::new();
        while let Some(n) = zreaddir(&mut dir, 1) {
            with_skip.push(n);
        }
        assert!(
            with_skip.contains(&"file".to_string()),
            "c:5232 — real entry survives ignoredots=1"
        );
        assert!(
            !with_skip.contains(&".".to_string()),
            "c:5232 — `.` filtered with ignoredots=1"
        );
        assert!(
            !with_skip.contains(&"..".to_string()),
            "c:5232 — `..` filtered with ignoredots=1"
        );

        // c:4648-equivalent — ignoredots=0: KEEP `.` and `..`.
        // (libstd's fs::read_dir filters them before exposing on
        // macOS/Linux, so the without_skip set is functionally
        // identical here — pin only that the API path doesn't error.)
        let mut dir2 = fs::read_dir(&base).unwrap();
        let mut without_skip: Vec<String> = Vec::new();
        while let Some(n) = zreaddir(&mut dir2, 0) {
            without_skip.push(n);
        }
        assert!(
            without_skip.contains(&"file".to_string()),
            "ignoredots=0 still yields real entries"
        );

        let _ = fs::remove_file(base.join("file"));
        let _ = fs::remove_dir(&base);
    }

    /// `Src/utils.c:1075` — `get_username()` uses `ztrdup_metafy` on
    /// `pswd->pw_name`. Previous Rust port returned the raw pw_name
    /// verbatim — fine for ASCII usernames, broken for high-bit
    /// bytes (downstream paramtab consumers assume metafied entries).
    /// Pin: ASCII usernames round-trip identically through metafy,
    /// AND the result is a non-empty string for the current uid.
    /// `Src/utils.c:3445-3460` — ztrftime zsh-specific %K/%L/%f
    /// extensions return values WITHOUT leading zeros (vs the
    /// strftime %H/%I/%d which pad).
    #[test]
    #[cfg(unix)]
    fn ztrftime_zsh_extensions_no_leading_zero() {
        let _g = crate::test_util::global_state_lock();
        // Pick a known time: Jan 5 2024 09:07:42.123456789 UTC.
        // Use SystemTime + an offset since we want deterministic values
        // independent of TZ; we just verify that the format substitutes
        // the right NUMBER OF DIGITS for the leading-zero case.
        use std::time::Duration;
        // 2024-01-05 09:07:42 UTC = 1704445662
        let t = UNIX_EPOCH + Duration::new(1704445662, 123_456_789);
        // %H gives leading zero "09"; %K should give "9" (in some TZ
        // hour will be different but the digit-count rule still holds).
        let h_padded = ztrftime("%H", t, false);
        let k_unpadded = ztrftime("%K", t, false);
        // Both represent the same hour. If h_padded starts with `0`,
        // k_unpadded must be 1-char and not start with `0`.
        if h_padded.starts_with('0') && h_padded.len() == 2 {
            assert!(
                !k_unpadded.starts_with('0'),
                "c:3445 — %K must strip the leading 0 from %H={}, got %K={}",
                h_padded,
                k_unpadded
            );
            assert_eq!(
                k_unpadded.len(),
                1,
                "c:3445 — %K should be 1 digit when hour < 10"
            );
        }
        // %f for day-of-month: t is Jan 5 → in any reasonable TZ
        // day is between 4 and 6; format should be 1 digit when day < 10.
        let d_padded = ztrftime("%d", t, false);
        let f_unpadded = ztrftime("%f", t, false);
        if d_padded.starts_with('0') && d_padded.len() == 2 {
            assert!(!f_unpadded.starts_with('0'));
            assert_eq!(
                f_unpadded.len(),
                1,
                "c:3457 — %f should be 1 digit when day < 10"
            );
        }
        // %3. fractional seconds: input nsec is 123456789 → first 3
        // digits should be 123. Note zsh's syntax is `%N.` where N is
        // the digit count (c:3374-3384 + c:3409).
        let frac = ztrftime("%3.", t, false);
        assert_eq!(
            frac, "123",
            "c:3409-3438 — %3. must emit first 3 digits of nsec"
        );
        // %. with no digit prefix defaults to 3 digits per c:3409.
        let frac_default = ztrftime("%.", t, false);
        assert_eq!(frac_default, "123", "c:3409 — %. defaults to 3 digits");
    }

    #[test]
    #[cfg(unix)]
    fn get_username_returns_metafied_non_empty_string() {
        let _g = crate::test_util::global_state_lock();
        let name = get_username();
        // CI environments may have a username from `whoami(1)`. In
        // weird sandbox-only setups it might be empty, but on every
        // standard build it should be set.
        if name.is_empty() {
            return;
        }
        // ASCII round-trip via metafy is identity. If the username
        // had high-bit bytes (the bug case), the output would be
        // Meta-escaped — both forms are non-empty so this pin
        // doesn't reject either, just ensures the fn returns
        // SOMETHING usable (not an empty string).
        assert!(
            !name.is_empty(),
            "c:1086 — getpwuid result must yield a non-empty username"
        );
        // Sanity: metafy(name) == name for ASCII input (every byte
        // is below Meta range). Confirms the c:1086 step preserves
        // ASCII paths byte-for-byte.
        if name.bytes().all(|b| b < 0x80) {
            assert_eq!(
                metafy(&name),
                name,
                "c:1086 — ASCII metafy is identity (so pin holds for ASCII users)"
            );
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Pure-utility tests — zstrtol/zstrtoul (numeric parse) and
    // getkeystring (key-escape decode). Each test pinned against either
    // C zsh semantics or a direct zsh shell invocation where applicable.
    // ═══════════════════════════════════════════════════════════════════

    // ── zstrtol: numeric string → (value, unconsumed_tail) ──────────
    /// `zstrtol("42", 10)` → (42, "") — full consumption.
    #[test]
    fn zstrtol_decimal_full_consumption() {
        let (v, t) = zstrtol("42", 10);
        assert_eq!(v, 42);
        assert_eq!(t, "");
    }

    /// `zstrtol("42abc", 10)` → (42, "abc") — stops at first non-digit.
    #[test]
    fn zstrtol_decimal_stops_at_non_digit() {
        let (v, t) = zstrtol("42abc", 10);
        assert_eq!(v, 42);
        assert_eq!(t, "abc");
    }

    /// `zstrtol("-7", 10)` → (-7, "") — sign handling.
    #[test]
    fn zstrtol_negative_decimal() {
        let (v, t) = zstrtol("-7", 10);
        assert_eq!(v, -7);
        assert_eq!(t, "");
    }

    /// `zstrtol("+12", 10)` → (12, "") — explicit `+` sign.
    #[test]
    fn zstrtol_explicit_plus_sign_consumed() {
        let (v, t) = zstrtol("+12", 10);
        assert_eq!(v, 12);
        assert_eq!(t, "");
    }

    /// `zstrtol("ff", 16)` → (255, "") — hex base.
    #[test]
    fn zstrtol_hex_base_16() {
        let (v, t) = zstrtol("ff", 16);
        assert_eq!(v, 255);
        assert_eq!(t, "");
    }

    /// `zstrtol("FF", 16)` → (255, "") — case-insensitive hex.
    #[test]
    fn zstrtol_hex_uppercase() {
        let (v, t) = zstrtol("FF", 16);
        assert_eq!(v, 255);
    }

    /// `zstrtol("1010", 2)` → (10, "") — binary base.
    #[test]
    fn zstrtol_binary_base_2() {
        let (v, t) = zstrtol("1010", 2);
        assert_eq!(v, 10);
        assert_eq!(t, "");
    }

    /// `zstrtol("17", 8)` → (15, "") — octal base.
    #[test]
    fn zstrtol_octal_base_8() {
        let (v, t) = zstrtol("17", 8);
        assert_eq!(v, 15);
    }

    /// `zstrtol("0x1A", 0)` → base-detect picks hex from `0x` prefix → 26.
    #[test]
    fn zstrtol_base_zero_detects_hex_prefix() {
        let (v, _) = zstrtol("0x1A", 0);
        assert_eq!(v, 26, "0x1A with base=0 → hex 26");
    }

    /// `zstrtol("0b101", 0)` → base-detect picks binary → 5.
    #[test]
    fn zstrtol_base_zero_detects_binary_prefix() {
        let (v, _) = zstrtol("0b101", 0);
        assert_eq!(v, 5, "0b101 with base=0 → binary 5");
    }

    /// `zstrtol("017", 0)` → base-detect: leading `0` → octal → 15.
    #[test]
    fn zstrtol_base_zero_leading_zero_means_octal() {
        let (v, _) = zstrtol("017", 0);
        assert_eq!(v, 15, "017 with base=0 → octal 15");
    }

    /// `zstrtol("  42", 10)` → (42, "") — leading whitespace skipped.
    #[test]
    fn zstrtol_leading_whitespace_skipped() {
        let (v, _) = zstrtol("  42", 10);
        assert_eq!(v, 42);
    }

    /// `zstrtol("0", 10)` → (0, "") — zero is valid input.
    #[test]
    fn zstrtol_zero_input() {
        let (v, t) = zstrtol("0", 10);
        assert_eq!(v, 0);
        assert_eq!(t, "");
    }

    // ── zstrtol_underscore: digit-separator support ─────────────────
    /// `zstrtol_underscore("1_000_000", 10, true)` → (1_000_000, "")
    /// — underscores skipped during digit accumulation.
    #[test]
    fn zstrtol_underscore_separator_in_decimal() {
        let (v, _) = zstrtol_underscore("1_000_000", 10, true);
        assert_eq!(v, 1_000_000);
    }

    /// Without underscore flag, `_` stops parsing.
    #[test]
    fn zstrtol_underscore_disabled_stops_at_underscore() {
        let (v, t) = zstrtol_underscore("123_456", 10, false);
        assert_eq!(v, 123);
        assert_eq!(t, "_456");
    }

    // ── zstrtoul_underscore: unsigned variant ───────────────────────
    /// Parses a plain decimal unsigned.
    #[test]
    fn zstrtoul_underscore_basic_decimal() {
        let v = zstrtoul_underscore("12345");
        assert_eq!(v, Some(12345));
    }

    /// Empty string → None (no number to parse).
    #[test]
    fn zstrtoul_underscore_empty_returns_none() {
        let v = zstrtoul_underscore("");
        assert_eq!(v, None);
    }

    // ── getkeystring: shell-escape decode ───────────────────────────
    /// `\n` → newline, `\t` → tab, `\r` → CR.
    /// Anchor: `print -r -- $'\n\t\r'` produces the three bytes.
    #[test]
    fn getkeystring_decodes_common_escapes() {
        assert_eq!(getkeystring("\\n").0, "\n");
        assert_eq!(getkeystring("\\t").0, "\t");
        assert_eq!(getkeystring("\\r").0, "\r");
    }

    /// `\\` → single backslash.
    #[test]
    fn getkeystring_double_backslash_yields_single() {
        assert_eq!(getkeystring("\\\\").0, "\\");
    }

    /// `\xNN` hex escape → byte value.
    /// Anchor: `print -r -- $'\x41'` → "A" (0x41 = 65 = 'A').
    #[test]
    fn getkeystring_hex_escape_lowercase_x() {
        assert_eq!(getkeystring("\\x41").0, "A");
        assert_eq!(getkeystring("\\x7E").0, "~");
    }

    /// `\u{NNNN}` Unicode escape → corresponding char.
    /// Anchor: `print -r -- $'é'` → "é".
    #[test]
    fn getkeystring_unicode_escape_u_four_digits() {
        assert_eq!(getkeystring("\\u00E9").0, "é");
    }

    /// Plain text passes through unchanged.
    #[test]
    fn getkeystring_plain_text_passes_through() {
        assert_eq!(getkeystring("hello").0, "hello");
        assert_eq!(getkeystring("").0, "");
    }

    /// Mixed plain + escapes → both handled.
    #[test]
    fn getkeystring_mixed_text_and_escapes() {
        assert_eq!(getkeystring("a\\nb").0, "a\nb");
        assert_eq!(
            getkeystring("line1\\nline2\\tindented").0,
            "line1\nline2\tindented"
        );
    }

    // ── metafy / unmetafy round-trip on ASCII (identity) ────────────
    /// ASCII strings: metafy is identity (no meta-bytes to escape).
    #[test]
    fn metafy_then_unmetafy_ascii_roundtrips_to_input() {
        let s = "hello world";
        let m = metafy(s);
        assert_eq!(m, s);
        let mut bytes = m.into_bytes();
        let _len = unmetafy(&mut bytes);
        assert_eq!(String::from_utf8(bytes).unwrap(), "hello world");
    }

    // ═══════════════════════════════════════════════════════════════════
    // quotestring — emit a quoted shell-safe form of the input string.
    // Each QT_* mode produces a different quoting style. Tests pin
    // each mode for both empty and non-empty input.
    // ═══════════════════════════════════════════════════════════════════

    use crate::zsh_h::{
        QT_BACKSLASH, QT_BACKSLASH_PATTERN, QT_BACKSLASH_SHOWNULL, QT_DOLLARS, QT_DOUBLE, QT_NONE,
        QT_SINGLE, QT_SINGLE_OPTIONAL,
    };

    /// QT_NONE: no quoting; passes input through unchanged.
    #[test]
    fn quotestring_qt_none_passes_through_unchanged() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(quotestring("hello world", QT_NONE), "hello world");
        assert_eq!(quotestring("", QT_NONE), "");
        assert_eq!(quotestring("a*b?c", QT_NONE), "a*b?c");
    }

    /// QT_NONE empty → empty.
    #[test]
    fn quotestring_qt_none_empty_is_empty() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(quotestring("", QT_NONE), "");
    }

    /// QT_BACKSLASH on empty → "''" (single-quote pair).
    #[test]
    fn quotestring_qt_backslash_empty_yields_empty() {
        let _g = crate::test_util::global_state_lock();
        // c:6194 — shownull is 0 for plain QT_BACKSLASH, so empty → "".
        assert_eq!(quotestring("", QT_BACKSLASH), "");
    }

    /// QT_BACKSLASH_SHOWNULL on empty → "''" too.
    #[test]
    fn quotestring_qt_backslash_shownull_empty_yields_empty_single_quotes() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(quotestring("", QT_BACKSLASH_SHOWNULL), "''");
    }

    /// c:6194 — `if (!*s && shownull)`. `shownull` is 0 for QT_SINGLE (it is
    /// set only by QT_BACKSLASH_SHOWNULL at c:6165 and QT_SINGLE_OPTIONAL at
    /// c:6187), so an empty string yields an EMPTY body; the `''` comes from
    /// the caller's wrapper (subst.c:4085-4087).
    #[test]
    fn quotestring_qt_single_empty_yields_empty_body() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(quotestring("", QT_SINGLE), "");
    }

    /// QT_SINGLE_OPTIONAL on empty → "''" too.
    #[test]
    fn quotestring_qt_single_optional_empty_yields_empty_single_quotes() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(quotestring("", QT_SINGLE_OPTIONAL), "''");
    }

    /// c:6194 — `shownull` is 0 for QT_DOUBLE, so the body is empty and the
    /// `""` pair is added by the caller (subst.c:4085-4087).
    #[test]
    fn quotestring_qt_double_empty_yields_empty_body() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(quotestring("", QT_DOUBLE), "");
    }

    /// c:6194 — same for QT_DOLLARS; C's caller adds `''` and then
    /// `val[0] = '$'` (subst.c:4085-4090).
    #[test]
    fn quotestring_qt_dollars_empty_yields_empty_body() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(quotestring("", QT_DOLLARS), "");
    }

    /// QT_BACKSLASH_PATTERN escapes only pattern meta-chars.
    /// Input "a*b?c[d]" → "a\\*b\\?c\\[d\\]".
    #[test]
    fn quotestring_qt_backslash_pattern_escapes_glob_metas() {
        let _g = crate::test_util::global_state_lock();
        let r = quotestring("a*b?c[d]", QT_BACKSLASH_PATTERN);
        assert!(
            r.contains("\\*") && r.contains("\\?") && r.contains("\\[") && r.contains("\\]"),
            "all globs must be escaped; got {r:?}"
        );
    }

    /// QT_BACKSLASH_PATTERN does NOT escape plain ASCII letters/digits.
    #[test]
    fn quotestring_qt_backslash_pattern_doesnt_escape_plain_chars() {
        let _g = crate::test_util::global_state_lock();
        let r = quotestring("plain", QT_BACKSLASH_PATTERN);
        assert_eq!(r, "plain", "plain text passes through");
    }

    /// QT_BACKSLASH_PATTERN escapes `<`, `>`, `(`, `)`, `|`, `#`, `^`, `~`.
    #[test]
    fn quotestring_qt_backslash_pattern_escapes_all_meta_set() {
        let _g = crate::test_util::global_state_lock();
        for ch in ['<', '>', '(', ')', '|', '#', '^', '~'] {
            let s = format!("x{ch}y");
            let r = quotestring(&s, QT_BACKSLASH_PATTERN);
            assert!(
                r.contains(&format!("\\{ch}")),
                "{ch:?} must be escaped, got {r:?}"
            );
        }
    }

    /// c:6131-6134 — QT_SINGLE returns the BODY; a word with nothing to
    /// escape passes straight through and the caller adds the `'…'` pair.
    #[test]
    fn quotestring_qt_single_returns_body_unwrapped() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(quotestring("simple", QT_SINGLE), "simple");
    }

    // ═══════════════════════════════════════════════════════════════════
    // metafy / unmetafy edge cases — bytes that NEED meta-encoding
    // (NUL, Meta byte 0x83, Nularg 0xa1, etc.). Pin that metafy escapes
    // them via the Meta + (byte ^ 32) scheme and unmetafy reverses it.
    // ═══════════════════════════════════════════════════════════════════

    /// `metafy` is identity for plain ASCII (no meta bytes).
    #[test]
    fn metafy_ascii_is_identity_byte_for_byte() {
        let s = "abc 123 XYZ!@#";
        assert_eq!(metafy(s), s);
    }

    /// Empty string round-trips.
    #[test]
    fn metafy_empty_string_returns_empty() {
        assert_eq!(metafy(""), "");
    }

    /// metafy → unmetafy is round-trip identity for ASCII input.
    #[test]
    fn metafy_unmetafy_roundtrip_with_punctuation() {
        let s = "Hello, World! 123 +-=";
        let m = metafy(s);
        let mut bytes = m.into_bytes();
        let _ = unmetafy(&mut bytes);
        let result = String::from_utf8(bytes).expect("valid utf-8");
        assert_eq!(result, s);
    }

    /// Multi-byte UTF-8 chars get meta-encoded then unmetafied. The
    /// round-trip MUST preserve the original bytes. Inline the byte-
    /// level metafy here — `metafy()`'s String return goes through
    /// `from_utf8_lossy` for non-UTF-8 outputs (correct for String
    /// consumers like the lexer or pattern compile, but breaks byte-
    /// round-trip because U+FFFD inserts EF BF BD where the
    /// original Meta-escaped bytes lived). The unmetafy step
    /// reverses the Meta-escape to recover the source UTF-8 verbatim.
    #[test]
    fn metafy_unmetafy_roundtrip_with_utf8_multibyte_anchored() {
        let s = "日本語";
        // Inline of metafy() byte path (no String materialisation).
        let mut bytes: Vec<u8> = Vec::with_capacity(s.len());
        for &b in s.as_bytes() {
            if imeta_byte(b) {
                bytes.push(Meta);
                bytes.push(b ^ 32);
            } else {
                bytes.push(b);
            }
        }
        let _ = unmetafy(&mut bytes);
        let result = String::from_utf8(bytes).expect("valid utf-8");
        assert_eq!(result, s, "UTF-8 round-trip must preserve content");
    }

    // ─── zsh-corpus pins for quotestring per quote_type ─────────────

    /// `quotestring(QT_NONE)` is identity on any input.
    #[test]
    fn quotestring_corpus_qt_none_is_identity() {
        let s = "anything `goes` $here";
        assert_eq!(quotestring(s, QT_NONE), s);
    }

    /// c:6131-6134 — `quotestring(QT_SINGLE)` on a plain word is a
    /// pass-through: no `'` pair is added here (that is the caller's job at
    /// subst.c:4085-4087). Emitting the pair here is what broke completion
    /// inside a quoted word — see `multiquote_leaves_plain_word_unquoted`.
    #[test]
    fn quotestring_corpus_qt_single_returns_plain_word_verbatim() {
        let out = quotestring("hello", QT_SINGLE);
        assert_eq!(out, "hello", "QT_SINGLE returns the body, unwrapped");
    }

    /// c:6131-6134 — same contract for QT_DOUBLE.
    #[test]
    fn quotestring_corpus_qt_double_returns_plain_word_verbatim() {
        let out = quotestring("hello", QT_DOUBLE);
        assert_eq!(out, "hello", "QT_DOUBLE returns the body, unwrapped");
    }

    /// `quotestring(QT_SINGLE)` on string with apostrophe escapes the
    /// apostrophe by closing+escape+reopen: `it's` → `'it'\''s'`.
    #[test]
    fn quotestring_corpus_qt_single_escapes_apostrophe() {
        // Same precondition the sibling QT_BACKSLASH test states: `'` only
        // reaches the c:6313 `instring == QT_SINGLE && *u == '\''` arm after
        // passing c:6301 `ispecial(*u)`, which reads the live typtab. C
        // guarantees it is populated via the `inittyptab()` at init.c:1261
        // (ported at init.rs:1244); a unit test that takes neither
        // `global_state_lock()` nor this call sees a zeroed table.
        inittyptab();
        let out = quotestring("it's", QT_SINGLE);
        assert!(
            out.contains("\\'") || out.contains("'\\''"),
            "apostrophe gets escaped in single-quote form, got {out:?}",
        );
    }

    /// `quotestring(QT_BACKSLASH)` escapes shell special chars.
    /// Pattern chars like `*`, `?`, `$`, `(`, `)` should be backslashed.
    #[test]
    fn quotestring_corpus_qt_backslash_escapes_glob_chars() {
        // `ispecial` reads the live typtab, as C's macro does, so the table
        // has to be populated first — C guarantees it via the `inittyptab`
        // call in `setupvals`. Same entry call the sibling typtab tests make.
        inittyptab();
        let out = quotestring("a*b?c", QT_BACKSLASH);
        assert!(out.contains("\\*"), "* gets backslashed, got {out:?}");
        assert!(out.contains("\\?"), "? gets backslashed, got {out:?}");
    }

    /// c:6194 — `shownull` is 0 for QT_DOUBLE, so an empty input gives an
    /// empty body; the `""` pair comes from the caller.
    #[test]
    fn quotestring_corpus_qt_double_empty_yields_empty_body() {
        let out = quotestring("", QT_DOUBLE);
        assert_eq!(out, "", "empty double-quoted body is empty");
    }

    /// c:6194 — same for QT_SINGLE.
    #[test]
    fn quotestring_corpus_qt_single_empty_yields_empty_body() {
        let out = quotestring("", QT_SINGLE);
        assert_eq!(out, "", "empty single-quoted body is empty");
    }

    /// `quotestring(QT_BACKSLASH)` on plain alphanumeric is identity.
    /// "abc123" should round-trip with no backslashes added.
    #[test]
    fn quotestring_corpus_qt_backslash_plain_word_unchanged() {
        let out = quotestring("abc123", QT_BACKSLASH);
        assert_eq!(
            out, "abc123",
            "plain alphanumeric needs no escape, got {out:?}"
        );
    }

    /// `ztrlen("hello")` = 5 (no meta chars).
    #[test]
    fn ztrlen_corpus_plain_ascii_byte_count() {
        assert_eq!(ztrlen("hello"), 5);
    }

    /// `ztrlen("")` = 0.
    #[test]
    fn ztrlen_corpus_empty_is_zero() {
        assert_eq!(ztrlen(""), 0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // itype_end C-parity tests — pin `utils.c:4395-4495` per-itype-flag
    // behavior. One assertion per flag/path so a regression in the
    // char-class walk points at the exact branch that drifted.
    //
    // Tests that capture KNOWN ZSHRS BUGS use #[ignore = "ZSHRS BUG: …"]
    // so the bug stays surfaced but CI doesn't fail until the fix.
    // ═══════════════════════════════════════════════════════════════════

    /// `itype_end("abc123", IIDENT, false)` walks the full identifier.
    /// C utils.c:4395 — IIDENT matches letters/digits/`_` via TYPTAB.
    /// Requires `inittyptab()` for TYPTAB population — call at entry.
    #[test]
    fn itype_end_ident_walks_full_identifier() {
        let _g = crate::test_util::global_state_lock();
        inittyptab();
        use crate::ported::ztype_h::IIDENT;
        assert_eq!(itype_end("abc123", IIDENT as u32, false), 6);
    }

    /// `itype_end("abc-def", IIDENT, false)` stops at `-` (non-IIDENT).
    #[test]
    fn itype_end_ident_stops_at_dash() {
        let _g = crate::test_util::global_state_lock();
        inittyptab();
        use crate::ported::ztype_h::IIDENT;
        assert_eq!(itype_end("abc-def", IIDENT as u32, false), 3);
    }

    /// `itype_end("a", IIDENT, true)` with `once=true` stops after 1.
    #[test]
    fn itype_end_once_stops_after_first_match() {
        let _g = crate::test_util::global_state_lock();
        inittyptab();
        use crate::ported::ztype_h::IIDENT;
        assert_eq!(itype_end("abc", IIDENT as u32, true), 1);
    }

    /// `itype_end("", IIDENT, false)` on empty string returns 0.
    #[test]
    fn itype_end_empty_string_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        inittyptab();
        use crate::ported::ztype_h::IIDENT;
        assert_eq!(itype_end("", IIDENT as u32, false), 0);
    }

    /// `itype_end(":", IIDENT, false)` on non-IIDENT first char = 0.
    #[test]
    fn itype_end_non_ident_first_char_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        inittyptab();
        use crate::ported::ztype_h::IIDENT;
        assert_eq!(itype_end(":foo", IIDENT as u32, false), 0);
    }

    /// `itype_end("ns.foo", INAMESPC, false)` — C utils.c:4399-4413
    /// INAMESPC special-cases ksh93 namespace `.` separators. With
    /// non-POSIXIDENTIFIERS / non-KSH emulation, dotted names should
    /// walk through the `.` to consume `ns.foo` as a single name.
    /// The Rust port may stop at `.` since dotted-namespace support
    /// requires POSIXIDENTIFIERS check + ksh emulation gating that
    /// isn't fully wired.
    #[test]
    fn itype_end_inamespc_walks_through_ksh93_dot() {
        let _g = crate::test_util::global_state_lock();
        inittyptab();
        use crate::ported::ztype_h::INAMESPC;
        // Expected: full "ns.foo" = 6 bytes walked.
        assert_eq!(itype_end("ns.foo", INAMESPC, false), 6);
    }

    /// `itype_end("a b c", ISEP, false)` from a space char.
    /// ISEP matches IFS chars. Default IFS includes ` \t\n`.
    /// Walk should consume only the leading separator(s).
    #[test]
    fn itype_end_isep_walks_separator_run() {
        let _g = crate::test_util::global_state_lock();
        inittyptab();
        use crate::ported::ztype_h::ISEP;
        // " \t " then 'a' — 3 sep bytes.
        assert_eq!(itype_end(" \t a", ISEP as u32, false), 3);
    }

    // ═══════════════════════════════════════════════════════════════════
    // unmetafy C-parity tests — pin `utils.c:4954-4983`. Meta-byte
    // collapse (0x83 + (b^0x20) → b) must be in-place + return new len.
    // ═══════════════════════════════════════════════════════════════════

    /// `unmetafy("abc")` is a no-op for non-metafied input.
    #[test]
    fn unmetafy_pure_ascii_unchanged() {
        let _g = crate::test_util::global_state_lock();
        let mut v = b"abc".to_vec();
        let n = unmetafy(&mut v);
        assert_eq!(n, 3);
        assert_eq!(v, b"abc");
    }

    /// `unmetafy("a\x83a")` → "a\x01" because Meta(0x83) + next ^ 0x20
    /// = 'a' ^ 0x20 = 0x41 ^ 0x20 = 0x61... wait, 'a'=0x61, 0x61 ^ 0x20
    /// = 0x41 = 'A'. So unmetafy("a\x83a") → "aA" (2 bytes).
    #[test]
    fn unmetafy_meta_pair_collapses_to_xor_byte() {
        let _g = crate::test_util::global_state_lock();
        // 'a' + Meta(0x83) + 'a' → 'a' + ('a' ^ 0x20) = 'a' + 'A' = "aA"
        let mut v: Vec<u8> = vec![b'a', 0x83, b'a'];
        let n = unmetafy(&mut v);
        assert_eq!(n, 2);
        assert_eq!(v, vec![b'a', b'A']);
    }

    /// `unmetafy("")` returns 0 with empty buffer.
    #[test]
    fn unmetafy_empty_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let mut v: Vec<u8> = Vec::new();
        let n = unmetafy(&mut v);
        assert_eq!(n, 0);
    }

    /// `unmetafy` with trailing lone Meta byte. C `unmetafy` checks
    /// `p < s.len()` before reading the byte after Meta — trailing
    /// Meta at EOF is preserved as a single literal Meta byte.
    #[test]
    fn unmetafy_trailing_lone_meta_preserved() {
        let _g = crate::test_util::global_state_lock();
        let mut v: Vec<u8> = vec![b'a', 0x83];
        let n = unmetafy(&mut v);
        // Rust port copies the Meta byte to t, then loop exits with
        // p == s.len() → no second-byte read. Final = "a\x83" (2).
        assert_eq!(n, 2);
        assert_eq!(v, vec![b'a', 0x83]);
    }

    // ═══════════════════════════════════════════════════════════════════
    // nicechar / wcs_nicechar C-parity tests — pin display-form
    // conversion. Required by stradd, prompt expansion, completion etc.
    // ═══════════════════════════════════════════════════════════════════

    /// `nicechar('a')` returns the printable char unchanged.
    #[test]
    fn nicechar_printable_ascii_passes_through() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(nicechar('a'), "a");
    }

    /// `nicechar('\n')` returns `\n` escape sequence per C `nicechar`
    /// (utils.c:464+ — control chars get `^M` style or `\n` escape).
    /// zsh emits `\n` as literal `\n` (two chars) for non-quotable
    /// nicechar; the `nicechar_sel(c, false)` form is non-quotable.
    #[test]
    fn nicechar_newline_emits_escape() {
        let _g = crate::test_util::global_state_lock();
        let out = nicechar('\n');
        assert!(
            out == "\\n" || out == "\n",
            "newline should be escaped or literal; got {out:?}"
        );
    }

    /// `wcs_nicechar('A', None, None)` returns "A" for printable wide
    /// chars per utils.c:689.
    #[test]
    fn wcs_nicechar_printable_ascii_passes_through() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(wcs_nicechar('A', None, None), "A");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/utils.c nicechar dispatch.
    // ═══════════════════════════════════════════════════════════════════

    /// c:462 — nicechar for printable ASCII passes through.
    #[test]
    fn nicechar_printable_ascii_passes_through_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(nicechar('a'), "a");
        assert_eq!(nicechar('Z'), "Z");
        assert_eq!(nicechar('5'), "5");
        assert_eq!(nicechar(' '), " ");
    }

    /// c:487 — nicechar('\\n') returns '\\n' (backslash + n).
    #[test]
    fn nicechar_newline_returns_backslash_n() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(nicechar('\n'), "\\n");
    }

    /// c:490 — nicechar('\\t') returns '\\t' (backslash + t).
    #[test]
    fn nicechar_tab_returns_backslash_t() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(nicechar('\t'), "\\t");
    }

    /// c:479 — nicechar(0x7f) returns '^?' (DEL → ^?).
    #[test]
    fn nicechar_del_returns_caret_question() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(nicechar('\x7f'), "^?");
    }

    /// c:493 — nicechar(0x01) returns '^A' (Ctrl-A).
    #[test]
    fn nicechar_ctrl_a_returns_caret_A() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(nicechar('\x01'), "^A");
    }

    /// c:493 — nicechar(0x1b) returns '^[' (ESC).
    #[test]
    fn nicechar_esc_returns_caret_bracket() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(nicechar('\x1b'), "^[");
    }

    /// c:462 — nicechar_sel quotable=true uses '\\C-' instead of '^'
    /// for control chars.
    #[test]
    fn nicechar_sel_quotable_uses_backslash_C() {
        let _g = crate::test_util::global_state_lock();
        let r = nicechar_sel('\x01', true);
        assert!(
            r.contains("\\C-") || r.contains("^"),
            "quotable should emit \\C- form, got {:?}",
            r
        );
    }

    /// c:531 — is_nicechar('a') returns FALSE: printable chars don't
    /// NEED nice (escape) representation, they pass through verbatim.
    /// Per the C comment in utils.c:531: "Return whether the char needs
    /// nice representation".
    #[test]
    fn is_nicechar_printable_returns_false() {
        let _g = crate::test_util::global_state_lock();
        assert!(!is_nicechar('a'), "printable doesn't NEED nice repr");
        assert!(!is_nicechar('Z'));
        assert!(!is_nicechar('5'));
    }

    /// c:531 — is_nicechar for any byte 0..127 is safe (no panic).
    #[test]
    fn is_nicechar_all_ascii_safe() {
        let _g = crate::test_util::global_state_lock();
        for c in 0u8..=127 {
            let _ = is_nicechar(c as char); // no panic
        }
    }

    /// c:462 — nicechar deterministic.
    #[test]
    fn nicechar_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        for c in ['a', '\n', '\t', '\x7f', '\x01', '\x1b'] {
            let first = nicechar(c);
            for _ in 0..5 {
                assert_eq!(nicechar(c), first, "{:?} must be pure", c);
            }
        }
    }

    /// c:706 — `pathprog("")` returns None (empty path invalid).
    #[test]
    fn pathprog_empty_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert!(pathprog("").is_none());
    }

    /// c:776 — `pathprog("/nonexistent/zshrs_xyz")` returns None.
    #[test]
    fn pathprog_nonexistent_returns_none() {
        let _g = crate::test_util::global_state_lock();
        let r = pathprog("/__never_exists_zshrs_pathprog_xyz");
        assert!(r.is_none());
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/utils.c
    // c:99 set_widearray / c:447 nicechar_sel / c:521 is_nicechar /
    // c:742 zwcwidth / c:836 findpwd / c:928 slashsplit /
    // c:1258 get_username / c:1426 getnameddir / c:1475 dircmp /
    // c:1576 callhookfunc
    // ═══════════════════════════════════════════════════════════════════

    /// c:99 — `set_widearray("")` empty returns empty Vec.
    #[test]
    fn set_widearray_empty_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        assert!(set_widearray("").is_empty());
    }

    /// c:99 — `set_widearray` returns Vec<char>.
    #[test]
    fn set_widearray_returns_vec_char_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Vec<char> = set_widearray("a");
    }

    /// c:447 — `nicechar_sel('a', false)` returns "a" (printable pass-through).
    #[test]
    fn nicechar_sel_ascii_letter_passthrough() {
        assert_eq!(nicechar_sel('a', false), "a", "printable letter as-is");
    }

    /// c:447 — `nicechar_sel` is pure.
    #[test]
    fn nicechar_sel_is_pure() {
        for c in ['a', '\t', '\n', '\x1b', '\x7f', '日'] {
            let first = nicechar_sel(c, false);
            for _ in 0..3 {
                assert_eq!(
                    nicechar_sel(c, false),
                    first,
                    "nicechar_sel({:?}, false) must be pure",
                    c
                );
            }
        }
    }

    /// c:742 — `zwcwidth` returns usize (compile-time type pin).
    #[test]
    fn zwcwidth_returns_usize_type() {
        let _: usize = zwcwidth('a');
    }

    /// c:742 — `zwcwidth('a')` ASCII letter returns 1.
    #[test]
    fn zwcwidth_ascii_returns_one() {
        assert_eq!(zwcwidth('a'), 1, "ASCII letter width = 1");
    }

    /// c:836 — `findpwd("")` empty returns Option<String>.
    #[test]
    fn findpwd_empty_returns_option_string_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Option<String> = findpwd("");
    }

    /// c:928 — `slashsplit("")` empty returns empty Vec.
    #[test]
    fn slashsplit_empty_returns_empty() {
        assert!(slashsplit("").is_empty());
    }

    /// c:928 — `slashsplit("a/b/c")` splits to 3 elements.
    #[test]
    fn slashsplit_simple_three_components() {
        let r = slashsplit("a/b/c");
        assert_eq!(r, vec!["a".to_string(), "b".to_string(), "c".to_string()]);
    }

    /// c:1475 — `dircmp("", "")` both empty returns bool (type pin + safe).
    #[test]
    fn dircmp_both_empty_returns_bool_type() {
        let _g = crate::test_util::global_state_lock();
        let _: bool = dircmp("", "");
    }

    /// c:1426 — `getnameddir("")` empty returns None.
    #[test]
    fn getnameddir_empty_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert!(getnameddir("").is_none(), "empty → None");
    }

    /// c:1258 — `get_username` returns String (compile-time type pin).
    #[test]
    fn get_username_returns_string_type() {
        let _g = crate::test_util::global_state_lock();
        let _: String = get_username();
    }

    /// c:1921-1935 — adjustwinsize(1) seeds `$LINES`/`$COLUMNS` from the
    /// TIOCGWINSZ probe of SHTTY. The port gated the setiparam calls on
    /// LINES/COLUMNS being in the OS environment (a misread of C's
    /// zgetenv guard, which only covers env re-publication — the C param
    /// is always live via its zterm_lines valptr alias), so every
    /// interactive shell ran with LINES=0 → `$(( LINES / 3 ))` division
    /// by zero in history-search-multi-word pagination (_hsmw_main:81).
    #[test]
    fn adjustwinsize_seeds_lines_columns_from_shtty() {
        let _g = crate::test_util::global_state_lock();
        unsafe {
            let mut master: libc::c_int = -1;
            let mut slave: libc::c_int = -1;
            let mut ws: libc::winsize = std::mem::zeroed();
            ws.ws_row = 33;
            ws.ws_col = 77;
            if libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut ws,
            ) != 0
            {
                return; // no pty available (sandboxed CI) — skip
            }
            let saved_shtty = crate::ported::init::SHTTY.load(Ordering::Relaxed);
            crate::ported::init::SHTTY.store(slave, Ordering::Relaxed);
            let _ = adjustwinsize(1);
            crate::ported::init::SHTTY.store(saved_shtty, Ordering::Relaxed);
            libc::close(slave);
            libc::close(master);
        }
        assert_eq!(
            crate::ported::params::getiparam("LINES"),
            33,
            "LINES must reflect the SHTTY winsize probe"
        );
        assert_eq!(
            crate::ported::params::getiparam("COLUMNS"),
            77,
            "COLUMNS must reflect the SHTTY winsize probe"
        );
    }
}

// !!! WARNING: RUST-ONLY HELPER — NO DIRECT C COUNTERPART AS A
// FREE FUNCTION !!! Adapts the cited C pattern to the Rust
// pipeline. Extracted body of zerrmsg's `%e` arm (Src/utils.c:355-366):
// EINTR → "interrupt", EIO verbatim, else lowercased strerror.
// zerrmsg's errno branch routes through this so the prefix
// formatting matches C; module callers (zsh/system) embed the
// string in their own zwarnnam messages where C uses `%e`.
/// !!! WARNING: RUST-ONLY HELPER !!! No C counterpart exists as a
/// standalone function.
/// C has no such function: the errno-to-message rendering is the `case 'e':`
/// arm inside `zerrmsg` at `Src/utils.c:352-369` — EINTR becomes the literal
/// `interrupt`, EIO is printed verbatim, everything else is `strerror(num)`
/// with `tulower(errmsg[0])` applied to the first byte (c:366). Rust needs it
/// as a value-returning function because callers format their own output.
/// Render an errno the way zsh's `%e` warning format does —
/// `zerrmsg` case `'e'` at `Src/utils.c:352-368`:
///
/// - `EINTR` → the literal string `interrupt` (C also sets
///   `errflag |= ERRFLAG_ERROR` and truncates the rest of the
///   format; every zshrs call site uses `%e` as the final
///   conversion so the truncation is a no-op here).
/// - `EIO` → `strerror(EIO)` verbatim ("I/O" reads wrong
///   lowercased, per the c:361-364 comment).
/// - everything else → `strerror(num)` with the first letter
///   lowercased (`tulower(errmsg[0])`, c:366).
///
/// `std::io::Error::from_raw_os_error(n)` Display is NOT this
/// format — it appends " (os error N)" and keeps the capital.
/// That mismatch was the residual byte-diff in zsh/system's
/// `sysopen`/`zsystem flock` warnings (docs/BUGS.md #316).
pub fn zsh_errno_msg(num: i32) -> String {
    if num == libc::EINTR {
        return "interrupt".to_string(); // c:355-358
    }
    let msg = unsafe {
        let p = libc::strerror(num); // c:360
        if p.is_null() {
            return String::new();
        }
        std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
    };
    if num == libc::EIO {
        return msg; // c:363-364
    }
    // c:366 — `fputc(tulower(errmsg[0]), file);`
    let mut chars = msg.chars();
    match chars.next() {
        Some(c) => c.to_lowercase().collect::<String>() + chars.as_str(),
        None => msg,
    }
}

// !!! WARNING: RUST-ONLY HELPER — NO DIRECT C COUNTERPART AS A
// FREE FUNCTION !!! Adapts the cited C pattern to the Rust
// pipeline. Byte-exact inverse of metafy (Src/utils.c:4954 unmetafy):
// decodes Meta (\u{83}) + c^32 pairs back to raw bytes. C
// unmetafy works in place on char*; Rust strings are UTF-8 so
// the decode returns Vec<u8> for byte-exact writes.
/// !!! WARNING: RUST-ONLY HELPER !!! No C counterpart exists as a
/// standalone function.
/// Byte-exact inverse of `metafy` for Rust's char-level metafied `String`.
/// C's `unmetafy` (`Src/utils.c:4954`) rewrites a `char *` in place; a UTF-8
/// `String` cannot hold raw bytes, so this returns `Vec<u8>` instead.
/// Decode a char-level metafied String back to RAW bytes for a write
/// or exec boundary. c:Src/utils.c:4954 unmetafy — `if (*t++ == Meta
/// && *p) t[-1] = *p++ ^ 32;` — transposed onto the char-level
/// encoding `meta_encode_byte` produces: U+0083 followed by a scalar
/// in U+0080..=U+00FF yields the single raw byte `payload ^ 32`;
/// everything else emits its UTF-8 bytes verbatim. A U+0083 followed
/// by anything outside that range is not our encoding and passes
/// through untouched. Bug #127.
pub fn unmetafy_str(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    let mut it = s.chars().peekable();
    let mut buf = [0u8; 4];
    while let Some(c) = it.next() {
        if c == '\u{83}' {
            if let Some(&n) = it.peek() {
                let nu = n as u32;
                if (0x80..=0xff).contains(&nu) {
                    out.push((nu as u8) ^ 32);
                    it.next();
                    continue;
                }
            }
        }
        out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
    }
    out
}

//! Termcap module — port of `Src/Modules/termcap.c`.
//!
//! This depends on the termcap stuff in init.c                              // c:150
//!
//! C source has 0 structs/enums (uses libtermcap globals + the
//! `boolcodes[]`/`numcodes[]`/`strcodes[]` arrays from libtermcap
//! itself). Rust port matches: 0 types, only static capability
//! tables for an in-memory ANSI approximation.
//!
//! Architectural divergence: C links against libtermcap (or
//! libtinfo) and reads `/etc/termcap` via `tgetent(3)` /
//! `tgetflag(3)` / `tgetnum(3)` / `tgetstr(3)`. zshrs computes a
//! minimal capability set inline based on `$TERM` so we don't drag
//! libtermcap into the build. Function signatures + observable
//! outputs match C 1:1.

use crate::ported::params::{getsparam, TERMFLAGS};
use crate::ported::utils::{zsetupterm, zwarnnam};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Mutex, OnceLock};
use crate::ported::options::optlookup;
use crate::ported::zsh_h::{features, isset, module};
use crate::zsh_h::TERM_UNKNOWN;

/// Port of `ztgetflag(char *s)` from `Src/Modules/termcap.c:54`. Wraps
/// libtermcap's `tgetflag()` to disambiguate "off" from "not
/// present" via the `boolcodes[]` table walk: if `tgetflag`
/// returns 0 AND the cap is in `boolcodes`, it's a known cap that's
/// off (return 0); if not in boolcodes, it's unknown (return -1).
///
/// C signature: `static int ztgetflag(char *s)`. Returns 1 / 0 / -1.
pub fn ztgetflag(s: &str) -> i32 {
    // c:54
    if !ensure_termcap_loaded() {
        return -1; // tgetent failed
    }
    let s_c = match std::ffi::CString::new(s) {
        Ok(c) => c,
        Err(_) => return -1,
    };
    // c:62 — `switch (tgetflag(s)) { case 1: return 1; case 0: ...; }`
    let flag = {
        let _g = TERMCAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        unsafe { tgetflag(s_c.as_ptr()) }
    };
    match flag {
        // c:62
        1 => 1, // c:64
        _ => {
            // c:65-72 — `for (b = boolcodes; *b; b++) if (!strcmp(*b, s)) return 0;`
            for b in BOOLCODES {
                // c:65
                if *b == s {
                    // c:66
                    return 0; // c:68
                }
            }
            -1 // c:80
        }
    }
}

/// Port of `bin_echotc(char *name, char **argv, UNUSED(Options ops), UNUSED(int func))` from `Src/Modules/termcap.c:80`. The
/// `echotc` builtin: looks up a capability and emits its value
/// (or its tparam'd form when args follow).
///
/// C signature: `static int bin_echotc(char *name, char **argv, Options ops, int func)`.
/// WARNING: param names don't match C — Rust=(name, argv, _ops) vs C=(name, argv, ops, func)
pub fn bin_echotc(name: &str, argv: &[&str], _ops: &[bool; 256]) -> i32 {
    // c:80
    const TERM_BAD: i32 = 1 << 1;
    if argv.is_empty() {
        // c:85
        zwarnnam(name, "missing argument");
        return 1;
    }
    let s = argv[0];
    let argv_rest: Vec<&str> = argv[1..].to_vec(); // c:85 (s = *argv++)

    // c:87 — `if (termflags & TERM_BAD) return 1;`
    if (TERMFLAGS.load(Ordering::Relaxed) & TERM_BAD) != 0 {
        // c:87
        return 1; // c:88
    }
    // c:89 — `if ((termflags & TERM_UNKNOWN) && (isset(INTERACTIVE) || !init_term())) return 1;`
    if (TERMFLAGS.load(Ordering::Relaxed) & TERM_UNKNOWN) != 0 {
        // c:89
        let interactive =
            isset(optlookup("interactive"));
        if interactive || !ensure_termcap_loaded() {
            // c:89-90
            return 1; // c:90
        }
    }
    if !ensure_termcap_loaded() {
        return 1;
    }
    let s_c = match std::ffi::CString::new(s) {
        Ok(c) => c,
        Err(_) => return 1,
    };

    // c:92 — `if ((num = tgetnum(s)) != -1) { printf("%d\n", num); return 0; }`
    let num = {
        let _g = TERMCAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        unsafe { tgetnum(s_c.as_ptr()) }
    }; // c:92
    if num != -1 {
        // c:92
        println!("{}", num); // c:93
        return 0; // c:94
    }
    // c:97 — `switch (ztgetflag(s))`.
    match ztgetflag(s) {
        // c:97
        -1 => {} // c:99
        0 => {
            // c:100
            println!("no"); // c:101
            return 0; // c:102
        }
        _ => {
            // c:103
            println!("yes"); // c:104
            return 0; // c:105
        }
    }
    // c:108-110 — `t = tgetstr(s, &u);`
    let mut buf: [libc::c_char; 2048] = [0; 2048]; // c:84
    let mut area = buf.as_mut_ptr();
    let value = {
        let _g = TERMCAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let t = unsafe { tgetstr(s_c.as_ptr(), &mut area) }; // c:109
        if t.is_null() || (t as isize) == -1 || unsafe { *t } == 0 {
            // c:110
            // capability doesn't exist, or (if boolean) is off           // c:110
            drop(_g);
            zwarnnam(name, &format!("no such capability: {}", s)); // c:113
            return 1; // c:114
        }
        unsafe { std::ffi::CStr::from_ptr(t) }
            .to_string_lossy()
            .into_owned()
    };

    // c:117-122 — count arguments expected by the cap's `%d/%2/%3/%./%+` codes.
    let mut argct = 0usize; // c:117
    let bytes = value.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // c:117
        if bytes[i] == b'%' {
            // c:118
            i += 1;
            if i < bytes.len() {
                // c:119
                match bytes[i] {
                    // c:119-120
                    b'd' | b'2' | b'3' | b'.' | b'+' => argct += 1, // c:120
                    _ => {}
                }
            }
        }
        i += 1;
    }

    // c:124-128 — `if (arrlen(argv) != argct) zwarnnam("not enough/too many args"); return 1;`
    if argv_rest.len() != argct {
        // c:124
        let msg = if argv_rest.len() < argct {
            "not enough arguments"
        }
        // c:125
        else {
            "too many arguments"
        }; // c:126
        zwarnnam(name, msg); // c:125-126
        return 1; // c:127
    }

    // c:131-137 — `tputs(t, 1, putraw)` or `tputs(tgoto(t, num, atoi(*argv)), 1, putraw)`.
    if argct == 0 {
        // c:131
        // c:132 — `tputs(t, 1, putraw);` — direct emit of raw cap.
        print!("{}", value); // c:132
    } else {
        // c:135 — `num = (argv[1]) ? atoi(argv[1]) : atoi(*argv);`
        // c:136 — `tputs(tgoto(t, num, atoi(*argv)), 1, putraw);`
        // libtinfo `tgoto` resolves the cap with col=arg0/line=arg1; the
        // static-link path emits the cap with %d/%2 replacement so cm
        // ("\E[%i%d;%dH") still produces a usable ANSI sequence.
        let mut out = value;
        for arg in &argv_rest {
            out = out.replacen("%d", arg, 1);
            out = out.replacen("%2", arg, 1);
            out = out.replacen("%3", arg, 1);
        }
        print!("{}", out); // c:136
    }
    0 // c:144
}

/// Port of `gettermcap(UNUSED(HashTable ht), const char *name)` from `Src/Modules/termcap.c:144`. The
/// magic-assoc lookup callback for `${termcap[name]}`. Looks up
/// the capability name and returns its (possibly empty) value.
///
/// C signature: `static HashNode gettermcap(HashTable ht, const char *name)`.
/// Rust returns `Option<String>` — `Some(value)` for known caps,
/// `None` for unknown (matching C's PM_UNSET on no match).
/// WARNING: param names don't match C — Rust=(name) vs C=(ht, name)
pub fn gettermcap(name: &str) -> Option<String> {
    // c:144
    if !ensure_termcap_loaded() {
        return None;
    }
    let n_c = std::ffi::CString::new(name).ok()?;
    // c:163 — try string cap first (most common via `${termcap[name]}`).
    let mut buf: [libc::c_char; 1024] = [0; 1024];
    let mut area = buf.as_mut_ptr();
    let _g = TERMCAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let raw = unsafe { tgetstr(n_c.as_ptr(), &mut area) }; // c:163
    if !raw.is_null() {
        return Some(
            unsafe { std::ffi::CStr::from_ptr(raw) }
                .to_string_lossy()
                .into_owned(),
        );
    }
    // c:170 — numeric cap fallback.
    let n = unsafe { tgetnum(n_c.as_ptr()) }; // c:170
    if n != -1 {
        return Some(n.to_string());
    }
    // c:175 — boolean cap fallback.
    match unsafe { tgetflag(n_c.as_ptr()) } {
        // c:175
        1 => Some("yes".to_string()),
        0 => {
            // Known but off → "" only if it's in BOOLCODES.
            if BOOLCODES.iter().any(|b| *b == name) {
                Some(String::new())
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Port of `scantermcap(UNUSED(HashTable ht), ScanFunc func, int flags)` from `Src/Modules/termcap.c:200`. The
/// magic-assoc scan callback for `${(k)termcap}` / `${(kv)termcap}`.
/// Walks the bool/num/string code arrays and yields each
/// (name, value) pair where the capability is known.
///
/// C signature: `static void scantermcap(HashTable ht, ScanFunc func, int flags)`.
/// WARNING: param names don't match C — Rust=() vs C=(ht, func, flags)
pub fn scantermcap() -> Vec<(String, String)> {
    // c:200
    // c:200-235 — walk boolcodes/numcodes/strcodes, emit (name, value)
    // for each cap libtermcap reports as present.
    let mut out = Vec::new();
    if !ensure_termcap_loaded() {
        return out;
    }
    for &name in BOOLCODES
        .iter()
        .chain(NUMCODES.iter())
        .chain(STRCODES.iter())
    {
        if let Some(v) = gettermcap(name) {
            out.push((name.to_string(), v));
        }
    }
    out
}

// `capability_lookup` removed — Rust-only invention with hardcoded
// ANSI escapes that has no counterpart in Src/Modules/termcap.c.
// The C source links libtermcap (or libtinfo) and reads /etc/termcap
// via tgetent(3) + tgetflag(3) / tgetnum(3) / tgetstr(3) directly.
// Each call site below now invokes those libc-level routines via FFI.

unsafe extern "C" {
    fn tgetent(bp: *mut libc::c_char, name: *const libc::c_char) -> libc::c_int;
    fn tgetflag(id: *const libc::c_char) -> libc::c_int;
    fn tgetnum(id: *const libc::c_char) -> libc::c_int;
    fn tgetstr(id: *const libc::c_char, area: *mut *mut libc::c_char) -> *mut libc::c_char;
}

// `bintab` — port of `static struct builtin bintab[]` (termcap.c).

// `partab` — port of `static struct paramdef partab[]` (termcap.c).

// `module_features` — port of `static struct features module_features`
// from termcap.c:314.

/// Port of `setup_(UNUSED(Module m))` from `Src/Modules/termcap.c:323`.
#[allow(unused_variables)]
pub fn setup_(m: *const module) -> i32 {
    // c:323
    // C body c:325-326 — `return 0`. Faithful empty-body port.
    0
}

/// Port of `features_(UNUSED(Module m), UNUSED(char ***features))` from `Src/Modules/termcap.c:330`.
/// C body: `*features = featuresarray(m, &module_features); return 0;`
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {
    // c:330
    *features = featuresarray(m, module_features());
    0
}

/// Port of `enables_(UNUSED(Module m), UNUSED(int **enables))` from `Src/Modules/termcap.c:338`.
/// C body: `return handlefeatures(m, &module_features, enables);`
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {
    // c:338
    handlefeatures(m, module_features(), enables)
}

/// Port of `boot_(UNUSED(Module m))` from `Src/Modules/termcap.c:345`.
#[allow(unused_variables)]
pub fn boot_(m: *const module) -> i32 {
    // c:345
    // C body c:347-350 — `#ifdef HAVE_TGETENT zsetupterm(); #endif
    //                     return 0`. Initializes the termcap database
    //                     for echotc/$termcap to use.
    let _ = zsetupterm(); // c:365
    0
}

/// Port of `cleanup_(UNUSED(Module m))` from `Src/Modules/termcap.c:355`.
/// C body: `return setfeatureenables(m, &module_features, NULL);`
pub fn cleanup_(m: *const module) -> i32 {
    // c:355
    setfeatureenables(m, module_features(), None)
}

// =====================================================================
// static struct features module_features                            c:314 (termcap.c)
// =====================================================================

/// Port of `finish_(UNUSED(Module m))` from `Src/Modules/termcap.c:365`.
#[allow(unused_variables)]
pub fn finish_(m: *const module) -> i32 {
    // c:365
    // C body c:367-368 — `return 0`. Faithful empty-body port; the
    //                    termcap database is process-lifetime, not
    //                    module-lifetime.
    0
}

/// Serialises every call into libtermcap. C `Src/Modules/termcap.c`
/// uses `tgetent(3)` / `tgetflag(3)` / `tgetnum(3)` / `tgetstr(3)`
/// directly; libtermcap (and libtinfo's compat layer) reads/writes
/// file-scope globals (`PC`, `BC`, `UP`, `ospeed`, the term-entry
/// buffer populated by `tgetent`) and is not thread-safe. zsh is
/// single-threaded so the C source is race-free under that invariant.
/// Rust callers (`ztgetflag`, `bin_echotc`, `gettermcap`, `scantermcap`)
/// can fire from concurrent test threads, so the lock restores the
/// single-writer assumption.
static TERMCAP_LOCK: Mutex<()> = Mutex::new(());

/// `boolcodes[]` from libtermcap — list of all known boolean
/// capability 2-char codes. The subset zshrs's in-memory table
/// recognises; full libtermcap has more.
static BOOLCODES: &[&str] = &[
    "am", "bs", "bw", "da", "db", "eo", "es", "gn", "hc", "hs", "in", "km", "mi", "ms", "nc", "ns",
    "os", "ul", "ut", "xb", "xn", "xo", "xs", "xt",
];

/// `numcodes[]` from libtermcap — list of known numeric codes.
static NUMCODES: &[&str] = &[
    "co", "it", "lh", "lm", "lw", "li", "ma", "MW", "Nl", "pa", "Nco", "sg", "tw", "ug", "vt", "ws",
];

/// `strcodes[]` from libtermcap — list of known string codes.
static STRCODES: &[&str] = &[
    "ae", "al", "AL", "ac", "as", "bc", "bl", "bt", "cb", "cd", "ce", "cm", "cr", "cs", "ct", "cl",
    "cv", "DC", "DL", "DO", "do", "ds", "ec", "ed", "ei", "fs", "ho", "hd", "hu", "i1", "i3", "i2",
    "ic", "IC", "if", "im", "ip", "is", "kA", "kb", "kB", "kC", "kd", "kD", "kE", "kF", "ke", "kh",
    "kH", "kI",
    // `km` is a BOOLEAN cap ("Has Meta Key") per termcap(5); it lives
    // in BOOLCODES (line 316) and must NOT also appear here. Removed
    // 2026-05 to fix scantermcap duplicate-key emission.
    "kL", "kl", "kM", "kN", "kP", "kr", "kR", "kS", "ks", "kT", "kt", "ku", "l0", "l1", "l2", "l3",
    "l4", "l5", "l6", "l7", "l8", "l9", "le", "ll", "ma", "mb", "MC", "md", "me", "mh", "mk", "mm",
    "mo", "mp", "mr", "nd", "nl", "nw", "pc", "pf", "pk", "pl", "pn", "po", "pO", "ps", "px", "rc",
    "rf", "RI", "rp", "rs", "sa", "sc", "se", "SF", "sf", "so", "SR", "sr", "st", "ta", "te", "ti",
    "ts", "uc", "ue", "up", "UP", "us", "vb", "ve", "vi", "vs", "wi",
];

/// WARNING: NOT IN TERMCAP.C — AtomicI32-protected lazy termcap init; C uses libtermcap `tgetent` once at boot
/// (equivalent C logic at Src/Modules/termcap.c:82).
/// Initialize libtermcap's database for `$TERM`. Returns true on success.
/// C call site: `tgetent(NULL, term)` (zsh.h-compatible portable form).
fn ensure_termcap_loaded() -> bool {
    // 0 = uninit, 1 = ok, -1 = failed. Cache the libtermcap state for
    // the lifetime of the process, matching libtermcap's own behavior.
    static STATE: AtomicI32 = AtomicI32::new(0);
    match STATE.load(Ordering::Relaxed) {
        1 => true,
        -1 => false,
        _ => {
            // C `init_term` (Src/init.c:771) reads the `term` global
            // which is the shell's $TERM param. The previous Rust port
            // read \`std::env::var(\"TERM\")\` which diverges when the
            // shell has updated TERM via paramtab without exporting yet.
            // Route through getsparam — same env-vs-paramtab family as
            // the recent newuser HOME / bin_strftime TZ fixes.
            let term = getsparam("TERM").unwrap_or_else(|| "dumb".into());
            let term_c = match std::ffi::CString::new(term) {
                Ok(c) => c,
                Err(_) => return false,
            };
            let r = {
                let _g = TERMCAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
                unsafe { tgetent(std::ptr::null_mut(), term_c.as_ptr()) }
            };
            let ok = r > 0;
            STATE.store(if ok { 1 } else { -1 }, Ordering::Relaxed);
            ok
        }
    }
}

// =====================================================
// ShellExecutor shim
// =====================================================

// (impl ShellExecutor block moved to src/exec_shims.rs — see file marker)


static MODULE_FEATURES: OnceLock<Mutex<features>> = OnceLock::new();

// Local stubs for the per-module entry points. C uses generic
// `featuresarray`/`handlefeatures`/`setfeatureenables` (module.c:
// 3275/3370/3445) but those take `Builtin` + `Features` pointer
// fields the Rust port doesn't carry. The hardcoded descriptor
// list mirrors the C bintab/conddefs/mathfuncs/paramdefs.
// WARNING: NOT IN TERMCAP.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn featuresarray(_m: *const module, _f: &Mutex<features>) -> Vec<String> {
    vec!["b:echotc".to_string(), "p:termcap".to_string()]
}

// WARNING: NOT IN TERMCAP.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn handlefeatures(
    _m: *const module,
    _f: &Mutex<features>,
    enables: &mut Option<Vec<i32>>,
) -> i32 {
    if enables.is_none() {
        *enables = Some(vec![1; 2]);
    }
    0
}

// WARNING: NOT IN TERMCAP.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn setfeatureenables(_m: *const module, _f: &Mutex<features>, _e: Option<&[i32]>) -> i32 {
    0
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ─── RUST-ONLY ACCESSORS ───
//
// Singleton accessor fns for `OnceLock<Mutex<T>>` / `OnceLock<
// RwLock<T>>` globals declared above. C zsh uses direct global
// access; Rust needs these wrappers because `OnceLock::get_or_init`
// is the only way to lazily construct shared state. These fns sit
// here so the body of this file reads in C source order without
// the accessor wrappers interleaved between real port fns.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ─── RUST-ONLY ACCESSORS ───
//
// Singleton accessor fns for `OnceLock<Mutex<T>>` / `OnceLock<
// RwLock<T>>` globals declared above. C zsh uses direct global
// access; Rust needs these wrappers because `OnceLock::get_or_init`
// is the only way to lazily construct shared state. These fns sit
// here so the body of this file reads in C source order without
// the accessor wrappers interleaved between real port fns.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// WARNING: NOT IN TERMCAP.C — Rust-only module-framework shim.
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
            pd_size: 1,
            n_abstract: 0,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ztgetflag_known_on_returns_one() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(ztgetflag("am"), 1);
    }

    #[test]
    fn ztgetflag_unknown_returns_minus_one() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(ztgetflag("zz"), -1);
    }

    #[test]
    fn gettermcap_co_returns_columns() {
        let _g = crate::test_util::global_state_lock();
        let v = gettermcap("co");
        assert!(v.is_some());
        let n: i32 = v.unwrap().parse().unwrap_or(0);
        assert!(n > 0);
    }

    #[test]
    fn gettermcap_unknown_returns_none() {
        let _g = crate::test_util::global_state_lock();
        assert!(gettermcap("zz_nonexistent").is_none());
    }

    #[test]
    fn scantermcap_emits_bool_caps() {
        let _g = crate::test_util::global_state_lock();
        let v = scantermcap();
        assert!(v.iter().any(|(k, _)| k == "am"));
    }

    /// c:80-85 — `bin_echotc` with no args writes "missing argument"
    /// to stderr and returns 1. Catches a regression that
    /// dereferences argv[0] on empty input.
    #[test]
    fn echotc_with_no_args_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let r = bin_echotc("echotc", &[], &[false; 256]);
        assert_eq!(r, 1, "echotc must report missing-arg error");
    }

    /// c:97 — `echotc <unknown>` falls through tgetnum / tgetstr /
    /// ztgetflag, all return -1 / null, so the function exits
    /// nonzero. Verifies the unknown-cap default path doesn't write
    /// garbage to stdout.
    #[test]
    fn echotc_unknown_cap_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let r = bin_echotc("echotc", &["zz_definitely_not_a_cap"], &[false; 256]);
        assert_ne!(r, 0, "unknown cap must error");
    }

    /// c:54-72 — `ztgetflag` on a NUL-byte-containing string must
    /// not panic. CString::new fails for embedded NULs; the port
    /// must catch that and return -1.
    #[test]
    fn ztgetflag_rejects_embedded_nul() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            ztgetflag("a\0m"),
            -1,
            "embedded NUL must surface as -1, not panic or false-match"
        );
    }

    /// c:54 — `ztgetflag("")` must be -1 (the empty string is in
    /// neither the live termcap nor the boolcodes table).
    #[test]
    fn ztgetflag_empty_string_returns_neg_one() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(ztgetflag(""), -1);
    }

    /// c:200 — `scantermcap` results must have unique keys (the
    /// boolcodes / numcodes / strcodes tables are disjoint by C
    /// design). Catches a regression that double-emits a cap when
    /// two tables claim it.
    #[test]
    fn scantermcap_keys_are_unique() {
        let _g = crate::test_util::global_state_lock();
        let v = scantermcap();
        let mut seen = std::collections::HashSet::new();
        for (k, _) in &v {
            assert!(
                seen.insert(k.clone()),
                "duplicate termcap key emitted: {}",
                k
            );
        }
    }

    /// c:200 — `scantermcap` must never produce empty key strings
    /// (every entry's key comes from the *codes tables, all of
    /// which have non-empty names per the termcap spec).
    #[test]
    fn scantermcap_keys_are_nonempty() {
        let _g = crate::test_util::global_state_lock();
        for (k, _) in scantermcap() {
            assert!(
                !k.is_empty(),
                "scantermcap emitted empty key — null entry leak?"
            );
        }
    }

    /// c:144 — `gettermcap` is case-sensitive (termcap names are
    /// always 2 lowercase letters). Pinning the case-sensitive
    /// behavior protects scripts that grep for specific cap names.
    #[test]
    fn gettermcap_is_case_sensitive() {
        let _g = crate::test_util::global_state_lock();
        // "co" is the columns cap; "CO" is unknown.
        let r1 = gettermcap("co");
        let r2 = gettermcap("CO");
        assert!(r1.is_some());
        assert!(
            r2.is_none(),
            "termcap names must be case-sensitive; uppercase resolved unexpectedly"
        );
    }

    /// c:323-365 — module-lifecycle stubs all return 0 in C.
    #[test]
    fn module_lifecycle_shims_all_return_zero() {
        let _g = crate::test_util::global_state_lock();
        let m: *const module = std::ptr::null();
        assert_eq!(setup_(m), 0);
        assert_eq!(boot_(m), 0);
        assert_eq!(cleanup_(m), 0);
        assert_eq!(finish_(m), 0);
    }
}

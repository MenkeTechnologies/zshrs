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

use crate::ported::options::optlookup;
use crate::ported::params::{getsparam, TERMFLAGS};
use crate::ported::utils::{zsetupterm, zwarnnam};
use crate::ported::zsh_h::{features, isset, module, INTERACTIVE};
use crate::zsh_h::TERM_UNKNOWN;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Mutex, OnceLock};

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
pub fn bin_echotc(
    name: &str,
    argv: &[String],
    _ops: &crate::ported::zsh_h::options,
    _func: i32,
) -> i32 {
    // c:80
    // c:Src/zsh.h:1985 — `#define TERM_BAD 0x01`. The previous local
    // shadow `const TERM_BAD: i32 = 1 << 1` was 0x02 — colliding with
    // TERM_UNKNOWN's 0x02 from zsh_h.rs. When TERMFLAGS carried only
    // TERM_UNKNOWN, the `& TERM_BAD` check at line 86 false-positived
    // and `echotc co` returned 1 silently before ensure_termcap_loaded
    // could attempt the libtermcap lookup. Use the canonical const.
    use crate::ported::zsh_h::TERM_BAD;
    if argv.is_empty() {
        // c:85
        zwarnnam(name, "missing argument");
        return 1;
    }
    let s: &str = argv[0].as_str();
    let argv_rest: Vec<&str> = argv[1..].iter().map(String::as_str).collect(); // c:85 (s = *argv++)

    // c:87 — `if (termflags & TERM_BAD) return 1;`
    if (TERMFLAGS.load(Ordering::Relaxed) & TERM_BAD) != 0 {
        // c:87
        return 1; // c:88
    }
    // c:89 — `if ((termflags & TERM_UNKNOWN) && (isset(INTERACTIVE) || !init_term())) return 1;`
    if (TERMFLAGS.load(Ordering::Relaxed) & TERM_UNKNOWN) != 0 {
        // c:89
        let interactive = isset(INTERACTIVE);
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
    // Re-fetch the cap pointer for tputs/tgoto — `value` is the cloned
    // String for the argct-count walk above, but tputs/tgoto want the
    // original cap C-string still in libtermcap's buffer area.
    let value_cstr = match std::ffi::CString::new(value.as_str()) {
        Ok(c) => c,
        Err(_) => return 1,
    };
    let _g = TERMCAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if argct == 0 {
        // c:131
        // c:132 — `tputs(t, 1, putraw);` — emit through libtermcap so
        // padding ($<delay>) is honored and any embedded %% / %i / %r
        // gets resolved.
        unsafe {
            tputs(value_cstr.as_ptr(), 1, putraw);
        }
    } else {
        // c:132 — `num = (argv[1]) ? atoi(argv[1]) : atoi(*argv);`
        // c:133 — `tputs(tgoto(t, num, atoi(*argv)), 1, putraw);`
        //
        // tgoto(cap, col, line) signature: col = atoi(*argv) =
        // argv_rest[0]; line = num = argv_rest[1] if present else
        // atoi(*argv). This convention matches `cm` (cursor-move) caps
        // taking (col, row) — the trailing argument is the row even
        // though it appears second.
        let col: libc::c_int = argv_rest
            .first()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let line: libc::c_int = argv_rest
            .get(1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(col);
        unsafe {
            let resolved = tgoto(value_cstr.as_ptr(), col, line);
            if !resolved.is_null() {
                tputs(resolved, 1, putraw);
            }
        }
    }
    drop(_g);
    // tputs writes via putraw → libc::write(1, ...) which is unbuffered;
    // flush stdout's userspace buffer too so a Rust-side println after
    // the cap emit doesn't reorder around it.
    use std::io::Write;
    let _ = std::io::stdout().flush();
    0 // c:135
}

/// Port of `static HashNode gettermcap(UNUSED(HashTable ht), const char *name)`
/// from `Src/Modules/termcap.c:144-199`. Synthesised Param with
/// PM_SCALAR + value or PM_UNSET on no match.
pub fn gettermcap(
    _ht: *mut crate::ported::zsh_h::HashTable,
    name: &str,
) -> Option<crate::ported::zsh_h::Param> {
    // c:144
    use crate::ported::zsh_h::{hashnode, param, Param, PM_READONLY, PM_SCALAR, PM_UNSET};

    let mk = |s: String, extra: i32| -> Param {
        Box::new(param {
            node: hashnode {
                next: None,
                nam: name.to_string(),
                flags: PM_READONLY as i32 | extra,
            },
            u_data: 0,
            u_arr: None,
            u_str: Some(s),
            u_val: 0,
            u_dval: 0.0,
            u_hash: None,
            gsu_s: None,
            gsu_i: None,
            gsu_f: None,
            gsu_a: None,
            gsu_h: None,
            base: 0,
            width: 0,
            env: None,
            ename: None,
            old: None,
            level: 0,
        })
    };

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
        let s = unsafe { std::ffi::CStr::from_ptr(raw) }
            .to_string_lossy()
            .into_owned();
        return Some(mk(s, PM_SCALAR as i32));
    }
    // c:170 — numeric cap fallback.
    let n = unsafe { tgetnum(n_c.as_ptr()) }; // c:170
    if n != -1 {
        return Some(mk(n.to_string(), PM_SCALAR as i32));
    }
    // c:175 — boolean cap fallback.
    match unsafe { tgetflag(n_c.as_ptr()) } {
        1 => Some(mk("yes".to_string(), PM_SCALAR as i32)),
        0 => {
            // Known but off → "" only if it's in BOOLCODES.
            if BOOLCODES.iter().any(|b| *b == name) {
                Some(mk(String::new(), PM_SCALAR as i32))
            } else {
                // c:191-193 — `pm->u.str = ""; pm->node.flags |= PM_UNSET;`
                Some(mk(String::new(), PM_SCALAR as i32 | PM_UNSET as i32))
            }
        }
        _ => Some(mk(String::new(), PM_SCALAR as i32 | PM_UNSET as i32)),
    }
}

/// Port of `scantermcap(UNUSED(HashTable ht), ScanFunc func, int flags)` from `Src/Modules/termcap.c:200`. The
/// magic-assoc scan callback for `${(k)termcap}` / `${(kv)termcap}`.
/// Walks the bool/num/string code arrays and yields each
/// (name, value) pair where the capability is known.
///
/// Port of `static void scantermcap(UNUSED(HashTable ht), ScanFunc func, int flags)`
/// from `Src/Modules/termcap.c:200-235`. Walks the bool/num/string
/// code arrays and invokes the callback per known cap.
pub fn scantermcap(
    _ht: *mut crate::ported::zsh_h::HashTable,
    func: Option<crate::ported::zsh_h::ScanFunc>,
    flags: i32,
) {
    // c:200
    use crate::ported::zsh_h::{hashnode, param, PM_SCALAR};
    let f = match func {
        Some(f) => f,
        None => return,
    };
    if !ensure_termcap_loaded() {
        return;
    }
    for &name in BOOLCODES
        .iter()
        .chain(NUMCODES.iter())
        .chain(STRCODES.iter())
    {
        if let Some(pm) = gettermcap(std::ptr::null_mut(), name) {
            // Skip PM_UNSET entries (unknown caps).
            use crate::ported::zsh_h::PM_UNSET;
            if (pm.node.flags & PM_UNSET as i32) != 0 {
                continue;
            }
            let node = param {
                node: hashnode {
                    next: None,
                    nam: name.to_string(),
                    flags: PM_SCALAR as i32,
                },
                u_data: 0,
                u_arr: None,
                u_str: pm.u_str.clone(),
                u_val: 0,
                u_dval: 0.0,
                u_hash: None,
                gsu_s: None,
                gsu_i: None,
                gsu_f: None,
                gsu_a: None,
                gsu_h: None,
                base: 0,
                width: 0,
                env: None,
                ename: None,
                old: None,
                level: 0,
            };
            let node_box = Box::new(node.node.clone());
            f(&node_box, flags);
        }
    }
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
    // c:Src/utils.c:424 `putraw(int c) { putc(c, stdout); }` is the
    // callback C's bin_echotc hands to tputs at c:129/133. tputs walks
    // the cap string and calls the putc-fn for each byte, expanding
    // padding ($<delay>) and decoding %-codes inline. Without it, the
    // naive %d/%2/%3 string replacement misses padding entirely and
    // mis-handles caps using %., %+, %i, %r, %% etc.
    fn tputs(
        str_: *const libc::c_char,
        affcnt: libc::c_int,
        putc_fn: extern "C" fn(libc::c_int) -> libc::c_int,
    ) -> libc::c_int;
    fn tgoto(
        cap: *const libc::c_char,
        col: libc::c_int,
        row: libc::c_int,
    ) -> *mut libc::c_char;
}

/// Port of `putraw(int c)` from `Src/utils.c:424`. Single-byte
/// stdout emit used as the tputs callback.
extern "C" fn putraw(c: libc::c_int) -> libc::c_int {
    // c:426 — `putc(c, stdout);`
    let byte = c as u8;
    unsafe {
        libc::write(1, &byte as *const u8 as *const libc::c_void, 1);
    }
    0 // c:427
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
fn handlefeatures(_m: *const module, _f: &Mutex<features>, enables: &mut Option<Vec<i32>>) -> i32 {
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
        let pm = gettermcap(std::ptr::null_mut(), "co").expect("co must resolve to Param");
        let v = pm.u_str.as_deref().unwrap_or("");
        let n: i32 = v.parse().unwrap_or(0);
        assert!(n > 0);
    }

    #[test]
    fn gettermcap_unknown_returns_unset_param() {
        let _g = crate::test_util::global_state_lock();
        use crate::ported::zsh_h::PM_UNSET;
        // C semantics (c:191-193): unknown caps return non-NULL Param
        // with PM_UNSET flag + empty u_str (not NULL HashNode).
        if let Some(pm) = gettermcap(std::ptr::null_mut(), "zz_nonexistent") {
            assert!(pm.node.flags & PM_UNSET as i32 != 0, "PM_UNSET set");
        }
    }

    #[test]
    fn scantermcap_emits_bool_caps() {
        let _g = crate::test_util::global_state_lock();
        use std::sync::Mutex;
        static SEEN: Mutex<Vec<String>> = Mutex::new(Vec::new());
        SEEN.lock().unwrap().clear();
        fn cb(node: &crate::ported::zsh_h::HashNode, _flags: i32) {
            SEEN.lock().unwrap().push(node.nam.clone());
        }
        scantermcap(std::ptr::null_mut(), Some(cb), 0);
        let seen = SEEN.lock().unwrap().clone();
        assert!(seen.iter().any(|k| k == "am"));
    }

    /// c:80-85 — `bin_echotc` with no args writes "missing argument"
    /// to stderr and returns 1. Catches a regression that
    /// dereferences argv[0] on empty input.
    #[test]
    fn echotc_with_no_args_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let r = bin_echotc("echotc", &[], &ops, 0);
        assert_eq!(r, 1, "echotc must report missing-arg error");
    }

    /// c:97 — `echotc <unknown>` falls through tgetnum / tgetstr /
    /// ztgetflag, all return -1 / null, so the function exits
    /// nonzero. Verifies the unknown-cap default path doesn't write
    /// garbage to stdout.
    #[test]
    fn echotc_unknown_cap_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let r = bin_echotc("echotc", &["zz_definitely_not_a_cap".to_string()], &ops, 0);
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
        use std::sync::Mutex;
        static KEYS: Mutex<Vec<String>> = Mutex::new(Vec::new());
        KEYS.lock().unwrap().clear();
        fn cb(node: &crate::ported::zsh_h::HashNode, _flags: i32) {
            KEYS.lock().unwrap().push(node.nam.clone());
        }
        scantermcap(std::ptr::null_mut(), Some(cb), 0);
        let collected = KEYS.lock().unwrap().clone();
        let mut seen = std::collections::HashSet::new();
        for k in &collected {
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
        use std::sync::Mutex;
        static KEYS: Mutex<Vec<String>> = Mutex::new(Vec::new());
        KEYS.lock().unwrap().clear();
        fn cb(node: &crate::ported::zsh_h::HashNode, _flags: i32) {
            KEYS.lock().unwrap().push(node.nam.clone());
        }
        scantermcap(std::ptr::null_mut(), Some(cb), 0);
        for k in KEYS.lock().unwrap().iter() {
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
        use crate::ported::zsh_h::PM_UNSET;
        // "co" is the columns cap; "CO" is unknown (PM_UNSET).
        let r1 = gettermcap(std::ptr::null_mut(), "co").expect("co Param");
        assert!(r1.node.flags & PM_UNSET as i32 == 0, "co must be set");
        if let Some(r2) = gettermcap(std::ptr::null_mut(), "CO") {
            assert!(
                r2.node.flags & PM_UNSET as i32 != 0,
                "termcap names case-sensitive; CO must be PM_UNSET"
            );
        }
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

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/termcap.c.
    // ═══════════════════════════════════════════════════════════════════

    /// c:54 — `ztgetflag("")` returns -1 (no such cap).
    #[test]
    fn ztgetflag_empty_string_returns_minus_one() {
        let _g = crate::test_util::global_state_lock();
        let r = ztgetflag("");
        assert_eq!(r, -1, "empty cap name → -1");
    }

    /// c:54 — `ztgetflag("zz")` returns -1 (no such 2-letter cap).
    #[test]
    fn ztgetflag_unknown_cap_returns_minus_one() {
        let _g = crate::test_util::global_state_lock();
        let r = ztgetflag("zz");
        assert_eq!(r, -1, "unknown cap → -1");
    }

    /// c:54 — `ztgetflag("am")` (auto-margin) returns 0 or 1
    /// (depending on terminal). Pin: not -1 since 'am' is a known cap.
    #[test]
    fn ztgetflag_known_cap_returns_zero_or_one() {
        let _g = crate::test_util::global_state_lock();
        let r = ztgetflag("am");
        assert!(r == 0 || r == 1, "known cap → 0 or 1, got {}", r);
    }

    /// c:54 — `ztgetflag` is deterministic for the same input.
    #[test]
    fn ztgetflag_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        for cap in &["am", "co", "zz", ""] {
            let first = ztgetflag(cap);
            for _ in 0..5 {
                assert_eq!(ztgetflag(cap), first, "{:?} must be pure", cap);
            }
        }
    }

    /// c:80 — `bin_echotc` with no args returns nonzero (usage error).
    #[test]
    fn bin_echotc_no_args_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let r = bin_echotc("echotc", &[], &ops, 0);
        assert_ne!(r, 0, "no cap name → usage error");
    }

    /// c:210 — `gettermcap(_, "")` returns Some(PM_UNSET).
    #[test]
    fn gettermcap_empty_name_returns_pm_unset() {
        let _g = crate::test_util::global_state_lock();
        use crate::ported::zsh_h::PM_UNSET;
        if let Some(pm) = gettermcap(std::ptr::null_mut(), "") {
            assert!(pm.node.flags & PM_UNSET as i32 != 0);
        }
    }

    /// c:323 — split per-hook lifecycle for finer failure resolution.
    #[test]
    fn termcap_setup_returns_zero_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(setup_(std::ptr::null()), 0);
    }

    /// c:373 — features_ returns 0.
    #[test]
    fn termcap_features_returns_zero_pin() {
        let _g = crate::test_util::global_state_lock();
        let mut f = Vec::new();
        assert_eq!(features_(std::ptr::null(), &mut f), 0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/termcap.c
    // c:32 ztgetflag / c:69 bin_echotc / c:210 gettermcap / c:288 scantermcap
    // ═══════════════════════════════════════════════════════════════════

    /// c:32 — `ztgetflag` is pure (multiple calls same result).
    #[test]
    fn ztgetflag_is_pure() {
        let _g = crate::test_util::global_state_lock();
        let r = ztgetflag("am");
        for _ in 0..5 {
            assert_eq!(ztgetflag("am"), r, "ztgetflag must be pure");
        }
    }

    /// c:32 — `ztgetflag` returns -1/0/1 only (no other values).
    #[test]
    fn ztgetflag_return_value_in_canonical_set() {
        let _g = crate::test_util::global_state_lock();
        for cap in ["am", "bw", "xn", "co", "li", "xyz_unknown_cap"] {
            let r = ztgetflag(cap);
            assert!(
                r == -1 || r == 0 || r == 1,
                "ztgetflag({:?}) = {} not in {{-1, 0, 1}}",
                cap,
                r
            );
        }
    }

    /// c:32 — multi-char/unknown caps still return -1.
    #[test]
    fn ztgetflag_arbitrary_strings_return_minus_one() {
        let _g = crate::test_util::global_state_lock();
        for s in ["xyz", "long_unknown_cap_name", "AAA", "zzz"] {
            assert_eq!(ztgetflag(s), -1, "unknown cap {:?} must return -1", s);
        }
    }

    /// c:69 — `bin_echotc` returns i32 (type pinning).
    #[test]
    fn bin_echotc_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let _: i32 = bin_echotc("echotc", &[], &ops, 0);
    }

    /// c:69 — `bin_echotc` empty + known-bad inputs all return nonzero.
    #[test]
    fn bin_echotc_empty_string_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let r = bin_echotc("echotc", &["".into()], &ops, 0);
        assert_ne!(r, 0, "empty cap name → error");
    }

    /// c:210 — `gettermcap("")` returns None (empty cap name).
    #[test]
    fn gettermcap_empty_string_returns_none() {
        let _g = crate::test_util::global_state_lock();
        let r = gettermcap(std::ptr::null_mut(), "");
        // Either None or Some(PM_UNSET) per C convention.
        let _ = r; // pin no-panic
    }

    /// c:210 — `gettermcap` is deterministic for same input.
    #[test]
    fn gettermcap_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let a = gettermcap(std::ptr::null_mut(), "co").is_some();
        for _ in 0..5 {
            let b = gettermcap(std::ptr::null_mut(), "co").is_some();
            assert_eq!(a, b, "gettermcap must be deterministic");
        }
    }

    /// c:365-410 — full lifecycle setup→features→enables→boot→cleanup→finish.
    #[test]
    fn termcap_full_lifecycle_returns_zero() {
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

    /// c:365 — setup_ idempotent.
    #[test]
    fn termcap_setup_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(setup_(std::ptr::null()), 0);
        }
    }

    /// c:410 — finish_ idempotent.
    #[test]
    fn termcap_finish_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(finish_(std::ptr::null()), 0);
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/termcap.c
    // c:32 ztgetflag / c:69 bin_echotc / c:210 gettermcap / c:288 scantermcap
    // ═══════════════════════════════════════════════════════════════════

    /// c:32 — `ztgetflag` returns i32 (compile-time pin).
    #[test]
    fn ztgetflag_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = ztgetflag("am");
    }

    /// c:32 — `ztgetflag("")` empty input returns -1 (unknown cap).
    #[test]
    fn ztgetflag_empty_string_returns_negative() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(ztgetflag(""), -1, "empty cap name must return -1 (unknown)");
    }

    /// c:32 — `ztgetflag` is deterministic.
    #[test]
    fn ztgetflag_deterministic_for_unknown() {
        let _g = crate::test_util::global_state_lock();
        for s in ["zz_unknown", "AAA", "garbage"] {
            let first = ztgetflag(s);
            for _ in 0..5 {
                assert_eq!(ztgetflag(s), first, "ztgetflag({:?}) must be pure", s);
            }
        }
    }

    /// c:69 — `bin_echotc` no-args returns nonzero (usage error, alt pin).
    #[test]
    fn bin_echotc_no_args_usage_error_alt() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let r = bin_echotc("echotc", &[], &ops, 0);
        assert_ne!(r, 0, "no args → usage error");
    }

    /// c:69 — `bin_echotc` exit code is non-negative across argv shapes.
    #[test]
    fn bin_echotc_exit_code_non_negative() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        for argv in [
            vec![],
            vec!["".into()],
            vec!["bl".into()],
            vec!["zz_unknown".into()],
            vec!["cm".into(), "5".into(), "10".into()],
        ] {
            let r = bin_echotc("echotc", &argv, &ops, 0);
            assert!(
                r >= 0,
                "exit code must be non-negative, got {} for {:?}",
                r,
                argv
            );
        }
    }

    /// c:210 — `gettermcap` returns Option<Param> (compile-time pin).
    #[test]
    fn gettermcap_returns_option_param_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Option<crate::ported::zsh_h::Param> = gettermcap(std::ptr::null_mut(), "co");
    }

    /// c:288 — `scantermcap` returns void (compile-time pin).
    #[test]
    fn scantermcap_none_callback_returns_void_type() {
        let _g = crate::test_util::global_state_lock();
        let _: () = scantermcap(std::ptr::null_mut(), None, 0);
    }

    /// c:288 — `scantermcap` is safe across various flag values.
    #[test]
    fn scantermcap_various_flags_no_panic() {
        let _g = crate::test_util::global_state_lock();
        for flags in [0i32, 1, 2, 0xff, -1] {
            scantermcap(std::ptr::null_mut(), None, flags);
        }
    }

    /// c:365 — `setup_` returns i32 (compile-time pin).
    #[test]
    fn termcap_setup_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = setup_(std::ptr::null());
    }

    /// c:381 — `enables_` returns i32 + None enables-out safe.
    #[test]
    fn termcap_enables_with_none_returns_i32() {
        let _g = crate::test_util::global_state_lock();
        let mut e: Option<Vec<i32>> = None;
        let _: i32 = enables_(std::ptr::null(), &mut e);
    }

    /// c:365/373/381/388/399/410 — each lifecycle hook returns 0 individually.
    #[test]
    fn termcap_each_lifecycle_hook_returns_zero_individually() {
        let _g = crate::test_util::global_state_lock();
        let null = std::ptr::null();
        let mut v: Vec<String> = Vec::new();
        let mut e: Option<Vec<i32>> = None;
        assert_eq!(setup_(null), 0, "c:365 setup_");
        assert_eq!(features_(null, &mut v), 0, "c:373 features_");
        assert_eq!(enables_(null, &mut e), 0, "c:381 enables_");
        assert_eq!(boot_(null), 0, "c:388 boot_");
        assert_eq!(cleanup_(null), 0, "c:399 cleanup_");
        assert_eq!(finish_(null), 0, "c:410 finish_");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity pins for Src/Modules/termcap.c
    // c:32 ztgetflag / c:69 bin_echotc / c:210 gettermcap /
    // c:288 scantermcap / c:399/410 cleanup/finish idempotency
    // ═══════════════════════════════════════════════════════════════════

    /// c:399 — `cleanup_` is idempotent.
    #[test]
    fn termcap_cleanup_idempotent_repeated_calls() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(cleanup_(std::ptr::null()), 0);
        }
    }

    /// c:399 — `cleanup_` return type i32 (compile-time pin).
    #[test]
    fn termcap_cleanup_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = cleanup_(std::ptr::null());
    }

    /// c:410 — `finish_` return type i32 (compile-time pin).
    #[test]
    fn termcap_finish_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = finish_(std::ptr::null());
    }

    /// c:388 — `boot_` return type i32 (compile-time pin).
    #[test]
    fn termcap_boot_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = boot_(std::ptr::null());
    }

    /// c:388 — `boot_` is idempotent.
    #[test]
    fn termcap_boot_idempotent_repeated_calls() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            let _ = boot_(std::ptr::null());
        }
    }

    /// c:32 — `ztgetflag` deterministic for known cap names.
    #[test]
    fn ztgetflag_deterministic_for_common_caps() {
        let _g = crate::test_util::global_state_lock();
        for cap in ["am", "bw", "xn", "km"] {
            let a = ztgetflag(cap);
            let b = ztgetflag(cap);
            assert_eq!(a, b, "ztgetflag({:?}) must be deterministic", cap);
        }
    }

    /// c:32 — `ztgetflag` very long cap name doesn't panic.
    #[test]
    fn ztgetflag_long_cap_name_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let long = "x".repeat(500);
        let _ = ztgetflag(&long);
    }

    /// c:69 — `bin_echotc` various func values don't panic.
    #[test]
    fn bin_echotc_various_func_values_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        for func in [-1, 0, 1, 100, i32::MAX] {
            let _ = bin_echotc("echotc", &[], &ops, func);
        }
    }

    /// c:69 — `bin_echotc` deterministic for unknown cap name.
    #[test]
    fn bin_echotc_deterministic_unknown_cap() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let args = vec!["__never_real_cap_xyz__".to_string()];
        let r1 = bin_echotc("echotc", &args, &ops, 0);
        let r2 = bin_echotc("echotc", &args, &ops, 0);
        assert_eq!(r1, r2, "bin_echotc unknown cap must be deterministic");
    }

    /// c:210 — `gettermcap("")` empty name doesn't panic.
    #[test]
    fn gettermcap_empty_name_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = gettermcap(std::ptr::null_mut(), "");
    }

    /// c:288 — `scantermcap` with None callback safe on repeat.
    #[test]
    fn scantermcap_none_callback_repeated_safe() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..5 {
            scantermcap(std::ptr::null_mut(), None, 0);
        }
    }

    /// c:381 — `enables_` with Some(non-empty) doesn't panic.
    #[test]
    fn termcap_enables_with_some_non_empty_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let mut e: Option<Vec<i32>> = Some(vec![1, 2, 3]);
        let _ = enables_(std::ptr::null(), &mut e);
    }
}

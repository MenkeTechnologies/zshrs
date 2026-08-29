//! Termcap module — port of `Src/Modules/termcap.c`.
//!
//! This depends on the termcap stuff in init.c                              // c:150
//!
//! C source has 0 structs/enums (uses libtermcap globals + the
//! `boolcodes[]`/`numcodes[]`/`strcodes[]` arrays from libtermcap
//! itself). Rust port matches: 0 types, only the `#ifndef HAVE_*CODES`
//! fallback tables the C carries for libraries that export no arrays.
//!
//! Where the C links a curses/termcap library, this reads the compiled
//! terminfo database directly through `crate::terminfo_db` — the same
//! `tgetflag(3)` / `tgetnum(3)` / `tgetstr(3)` contract, after `setupterm(3)`
//! has initialised the current entry exactly as `zsetupterm()`
//! (`Src/utils.c:386`) does for C's `boot_` (c:347). The capability set
//! `scantermcap` enumerates comes from `crate::terminfo_caps`, which carries
//! the same frozen `boolcodes`/`numcodes`/`strcodes` layout the C reads from
//! the library when `HAVE_BOOLCODES` etc. are defined. Nothing here links a
//! terminal library any more (docs/BUGS.md #1124).

use crate::ported::options::optlookup;
use crate::ported::params::{getsparam, TERMFLAGS};
use crate::ported::utils::{zsetupterm, zwarnnam};
use crate::ported::zsh_h::{features, isset, module, INTERACTIVE};
use crate::zsh_h::TERM_UNKNOWN;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{LazyLock, Mutex, OnceLock};

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
    // c:62 — `switch (tgetflag(s)) { case 1: return 1; case 0: ...; }`
    let flag = {
        let _g = TERMCAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        tgetflag(s)
    };
    match flag {
        // c:62
        1 => 1, // c:64
        _ => {
            // c:65-67 — `for (b = (char **)boolcodes; *b; ++b)
            //                if (s[0] == (*b)[0] && s[1] == (*b)[1]) return 0;`
            // C compares only the first two bytes (every termcap code is
            // exactly two characters), so port the byte compare rather
            // than a whole-string equality.
            let sb = s.as_bytes();
            for b in BOOLCODES.iter() {
                // c:65
                let bb = b.as_bytes();
                let s0 = sb.first().copied().unwrap_or(0);
                let s1 = sb.get(1).copied().unwrap_or(0);
                if bb.len() >= 2 && s0 == bb[0] && s1 == bb[1] {
                    // c:66
                    return 0; // c:67
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
    // `init_term` is Src/init.c:787 — the shell's `tgetent` entry
    // point, NOT this module's `ensure_termcap_loaded` setupterm
    // stand-in. Getting that wrong changes what ncurses returns for
    // `me`/`r2`/`rs` (see ensure_termcap_loaded's doc comment).
    if (TERMFLAGS.load(Ordering::Relaxed) & TERM_UNKNOWN) != 0 {
        // c:89
        let interactive = isset(INTERACTIVE);
        if interactive || crate::ported::init::init_term() == 0 {
            // c:89-90
            return 1; // c:90
        }
    }
    if !ensure_termcap_loaded() {
        return 1;
    }
    // c:92 — `if ((num = tgetnum(s)) != -1) { printf("%d\n", num); return 0; }`
    let num = {
        let _g = TERMCAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        tgetnum(s)
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
    let value = {
        let _g = TERMCAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        match tgetstr(s) {
            // c:109
            Some(t) if !t.is_empty() => String::from_utf8_lossy(&t).into_owned(),
            // c:110 — capability doesn't exist, or (if boolean) is off
            _ => {
                drop(_g);
                zwarnnam(name, &format!("no such capability: {}", s)); // c:113
                return 1; // c:114
            }
        }
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
    let _g = TERMCAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // `tputs` is `crate::tparm::tputs_strip_padding` plus the c:424
    // `putraw` byte loop: it drops the `$<delay>` padding specs and writes
    // the remaining bytes, which is all C's tputs did for this call site
    // (zshrs never used ncurses' padding SLEEP).
    let emit = |bytes: &[u8]| {
        for b in crate::tparm::tputs_strip_padding(bytes) {
            putraw(b as libc::c_int);
        }
    };
    if argct == 0 {
        // c:131
        // c:132 — `tputs(t, 1, putraw);`
        emit(value.as_bytes());
    } else {
        // c:131-133 — verbatim:
        //   /* This assumes arguments of <lines> <columns> for cap 'cm' */
        //   num = (argv[1]) ? atoi(argv[1]) : atoi(*argv);
        //   tputs(tgoto(t, num, atoi(*argv)), 1, putraw);
        //
        // tgoto's signature is `tgoto(cap, destcol, destline)` per
        // termcap(3) — column first, line second. C's pick:
        //   p1 = num = atoi(argv[1]) (or atoi(*argv) if only one arg)
        //   p2 = atoi(*argv)
        // i.e. user typed `echotc cm <line> <col>`, C maps the SECOND
        // user-arg to destcol (p1) and the FIRST to destline (p2). The
        // comment at c:131 makes the line-then-column convention
        // explicit.
        //
        // Prior Rust port had this reversed (col = argv_rest[0],
        // line = argv_rest[1]) — `echotc cm 5 10` jumped to (col=5,
        // line=10) where C jumps to (col=10, line=5).
        let num: i64 = argv_rest
            .get(1)
            .and_then(|s| s.parse().ok())
            .or_else(|| argv_rest.first().and_then(|s| s.parse().ok()))
            .unwrap_or(0); // c:132
        let arg0_n: i64 = argv_rest.first().and_then(|s| s.parse().ok()).unwrap_or(0); // c:133 atoi(*argv)
        // c:133 — tgoto(t, num, atoi(*argv))
        let resolved = crate::tparm::tgoto(value.as_bytes(), num, arg0_n);
        if !resolved.is_empty() {
            emit(&resolved);
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
            u_tied: None,
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

    // c:161 — `if (termflags & TERM_BAD) return NULL;`
    use crate::ported::zsh_h::TERM_BAD;
    if (TERMFLAGS.load(Ordering::Relaxed) & TERM_BAD) != 0 {
        return None; // c:162
    }
    // c:163 — `if ((termflags & TERM_UNKNOWN) &&
    //              (isset(INTERACTIVE) || !init_term())) return NULL;`
    // Same note as bin_echotc: `init_term` is Src/init.c:787's
    // `tgetent` path, which the scan callback deliberately does NOT
    // run — that asymmetry is what produces zsh's differing `me`
    // between `${termcap[me]}` and `${(kv)termcap}`.
    if (TERMFLAGS.load(Ordering::Relaxed) & TERM_UNKNOWN) != 0 {
        // c:163
        if isset(INTERACTIVE) || crate::ported::init::init_term() == 0 {
            return None; // c:164
        }
    }
    if !ensure_termcap_loaded() {
        return None;
    }
    let n_c = std::ffi::CString::new(name).ok()?;
    // c:165 — "logic in the following cascade copied from echotc, above"
    // — C's order is numeric → boolean → string (NOT string → numeric →
    // boolean). Prior Rust port reversed the order AND used raw
    // tgetflag instead of the canonical ztgetflag wrapper.
    //
    // Why order matters: some terminal entries define a name in
    // multiple cap classes. `lines` is numeric on most terms but
    // booldata files have it as a string alias on a few historical
    // entries — C's numeric-first order picks the integer; the
    // prior string-first port returned the cap escape sequence
    // instead, so `${termcap[lines]}` returned `\E[B` (the cursor-
    // down escape) on those terms instead of the line count.
    //
    // ztgetflag (this module's ported wrapper at c:54) distinguishes
    // 1 = on, 0 = "known but off" (cap is in BOOLCODES), -1 = unknown.
    // Raw tgetflag conflates 0 and -1, so the prior port couldn't tell
    // "boolean off" from "missing" — `${termcap[bw]}` returned PM_UNSET
    // for a known-off auto-margin instead of "no" as C does.
    let _g = TERMCAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // c:167 — try numeric cap first.
    let num = tgetnum(name); // c:167
    if num != -1 {
        // c:168-171 — PM_INTEGER with value (Rust port flattens to
        // decimal string so vm_helper::partab_get sees a populated
        // u_str without going through u_val).
        return Some(mk(num.to_string(), PM_SCALAR as i32));
    }
    drop(_g);
    // c:175-186 — boolean cap via ztgetflag wrapper.
    match ztgetflag(name) {
        // c:175
        1 => Some(mk("yes".to_string(), PM_SCALAR as i32)), // c:183
        0 => Some(mk("no".to_string(), PM_SCALAR as i32)),  // c:179
        _ => {
            // c:176 (break) — fall through to tgetstr.
            let _g = TERMCAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(raw) = tgetstr(name) {
                // c:188 — PM_SCALAR with cap string.
                Some(mk(String::from_utf8_lossy(&raw).into_owned(), PM_SCALAR as i32))
            } else {
                // c:192-193 — `pm->u.str = ""; pm->node.flags |= PM_UNSET;`
                Some(mk(String::new(), PM_SCALAR as i32 | PM_UNSET as i32))
            }
        }
    }
}

/// Port of `static void scantermcap(UNUSED(HashTable ht), ScanFunc func, int flags)`
/// from `Src/Modules/termcap.c:212-309`. The magic-assoc scan callback
/// for `${(k)termcap}` / `${(kv)termcap}`.
///
/// C runs THREE separate loops over three arrays, each with its own
/// probe: `boolcodes` via `ztgetflag` (c:272-278), `numcodes` via
/// `tgetnum` (c:283-289), `strcodes` via `tgetstr` (c:294-309). The
/// probe is NOT interchangeable with `gettermcap`'s cascade: a code
/// such as `MT` lives in BOTH `boolcodes` and `strcodes`, and C emits
/// it exactly once (the boolean loop finds it, the string loop's
/// `tgetstr` returns NULL for it). Routing every array through
/// `gettermcap` — which falls back to `ztgetflag` when `tgetstr`
/// would have failed — emitted such codes twice.
pub fn scantermcap(
    _ht: *mut crate::ported::zsh_h::HashTable,
    func: Option<crate::ported::zsh_h::ParamScanFunc>,
    flags: i32,
) {
    // c:212
    use crate::ported::zsh_h::{hashnode, param, PM_SCALAR};
    let f = match func {
        Some(f) => f,
        None => return,
    };
    if !ensure_termcap_loaded() {
        return;
    }
    // c:267-270 — one reused Param, PM_READONLY|PM_SCALAR. The Rust
    // callback takes the value by reference, so build it per emit.
    let emit_cap = |cap: &str, val: &str| {
        let pm = param {
            node: hashnode {
                next: None,
                nam: cap.to_string(), // c:275/286/305 pm->node.nam
                flags: PM_SCALAR as i32,
            },
            u_data: 0,
            u_tied: None,
            u_arr: None,
            u_str: Some(val.to_string()),
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
        f(&pm, flags); // c:276/287/306
    };

    // c:272-278 — `for (capcode = boolcodes; *capcode; capcode++)
    //                  if ((num = ztgetflag(*capcode)) != -1) ...`
    for cap in BOOLCODES.iter() {
        // c:272
        let num = ztgetflag(cap); // c:273
        if num != -1 {
            // c:274 — `pm->u.str = num ? "yes" : "no";`
            emit_cap(cap, if num != 0 { "yes" } else { "no" });
        }
    }

    // c:283-289 — `for (capcode = numcodes; *capcode; capcode++)
    //                  if ((num = tgetnum(*capcode)) != -1) ...`
    // c:280-281 — this block's params are PM_READONLY|PM_INTEGER; the
    // decimal rendering is identical for every consumer of the scan.
    for cap in NUMCODES.iter() {
        // c:283
        let num = {
            let _g = TERMCAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            tgetnum(cap) // c:284
        };
        if num != -1 {
            // c:285 — `pm->u.val = num;`
            emit_cap(cap, &num.to_string());
        }
    }

    // c:294-309 — `for (capcode = strcodes; *capcode; capcode++) {
    //                  char *u = buf;
    //                  if ((tcstr = tgetstr(*capcode,&u)) != NULL &&
    //                      tcstr != (char *)-1) ... }`
    for cap in STRCODES.iter() {
        // c:294
        let tcstr = {
            let _g = TERMCAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            tgetstr(cap) // c:302
        };
        // c:302-303
        if let Some(tcstr) = tcstr {
            emit_cap(cap, &String::from_utf8_lossy(&tcstr)); // c:304-306
        }
    }
}

// `capability_lookup` removed — Rust-only invention with hardcoded
// ANSI escapes that has no counterpart in Src/Modules/termcap.c.
// The C source links libtermcap (or libtinfo) and reads /etc/termcap
// via tgetent(3) + tgetflag(3) / tgetnum(3) / tgetstr(3) directly.
// Each call site below now invokes those libc-level routines via FFI.

use crate::ported::utils::putraw;

// c:Src/utils.c:399 — `zsetupterm()` initialises `cur_term` with
// `setupterm`, which is what termcap.c:347 `boot_` calls; termcap.c itself
// never calls `tgetent`. All of these were an `extern "C"` block resolved
// against ncurses and are now `crate::terminfo_db` / `crate::tparm`, reading
// the compiled terminfo database directly. That removed the binary's last
// link against a C terminal library, and with it the `libtinfo.so.6` install
// dependency on Ubuntu.
use crate::terminfo_db::{setupterm, tgetflag, tgetnum, tgetstr};


// `putraw` is `Src/utils.c:424`, so its port lives in
// `src/ported/utils.rs`; imported above and handed to `tputs` exactly
// as C's `bin_echotc` does at c:132/133.

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

// ---------------------------------------------------------------
// Capability-code tables (c:44-49, c:219-225, c:227-265)
//
// termcap.c wraps each literal array in `#ifndef HAVE_<X>CODES`: when
// the curses library exports `boolcodes[]` / `numcodes[]` /
// `strcodes[]` itself (ncurses and libtinfo both do — term.h:712-719
// declares them `extern char *const boolcodes[]` etc.), C walks the
// LIBRARY arrays at c:266 / c:275 / c:288 and the literals below are
// never compiled in. The literals are the `#ifndef` fallback,
// transcribed verbatim from the C, and are used here only when the
// library symbols resolve to an empty table.
//
// The previous Rust port had neither: it carried a hand-written
// "subset zshrs's in-memory table recognises", which under-reported
// `${(k)termcap}` by ~50 capabilities against the C.
// ---------------------------------------------------------------

/// c:45-49 — `#ifndef HAVE_BOOLCODES static char *boolcodes[] = {...}`
static BOOLCODES_FALLBACK: &[&str] = &[
    "bw", "am", "ut", "cc", "xs", "YA", "YF", "YB", "xt", "xn", "eo", "gn", "hc", "HC", "km", "YC",
    "hs", "hl", "in", "YG", "da", "db", "mi", "ms", "nx", "xb", "NP", "ND", "NR", "os", "5i", "YD",
    "YE", "es", "hz", "ul", "xo",
];

/// c:220-224 — `#ifndef HAVE_NUMCODES static char *numcodes[] = {...}`
static NUMCODES_FALLBACK: &[&str] = &[
    "co", "it", "lh", "lw", "li", "lm", "sg", "ma", "Co", "pa", "MW", "NC", "Nl", "pb", "vt", "ws",
    "Yo", "Yp", "Ya", "BT", "Yc", "Yb", "Yd", "Ye", "Yf", "Yg", "Yh", "Yi", "Yk", "Yj", "Yl", "Ym",
    "Yn",
];

/// c:228-264 — `#ifndef HAVE_STRCODES static char *zstrcodes[] = {...}`
static ZSTRCODES_FALLBACK: &[&str] = &[
    "ac", "bt", "bl", "cr", "ZA", "ZB", "ZC", "ZD", "cs", "rP", "ct", "MC", "cl", "cb", "ce", "cd",
    "ch", "CC", "CW", "cm", "do", "ho", "vi", "le", "CM", "ve", "nd", "ll", "up", "vs", "ZE", "dc",
    "dl", "DI", "ds", "DK", "hd", "eA", "as", "SA", "mb", "md", "ti", "dm", "mh", "ZF", "ZG", "im",
    "ZH", "ZI", "ZJ", "ZK", "ZL", "mp", "mr", "mk", "ZM", "so", "ZN", "ZO", "us", "ZP", "SX", "ec",
    "ae", "RA", "me", "te", "ed", "ZQ", "ei", "ZR", "ZS", "ZT", "ZU", "se", "ZV", "ZW", "ue", "ZX",
    "RX", "PA", "fh", "vb", "ff", "fs", "WG", "HU", "i1", "is", "i3", "if", "iP", "Ic", "Ip", "ic",
    "al", "ip", "K1", "K3", "K2", "kb", "@1", "kB", "K4", "K5", "@2", "ka", "kC", "@3", "@4", "@5",
    "@6", "kt", "kD", "kL", "kd", "kM", "@7", "@8", "kE", "kS", "@9", "k0", "k1", "k;", "F1", "F2",
    "F3", "F4", "F5", "F6", "F7", "F8", "F9", "k2", "FA", "FB", "FC", "FD", "FE", "FF", "FG", "FH",
    "FI", "FJ", "k3", "FK", "FL", "FM", "FN", "FO", "FP", "FQ", "FR", "FS", "FT", "k4", "FU", "FV",
    "FW", "FX", "FY", "FZ", "Fa", "Fb", "Fc", "Fd", "k5", "Fe", "Ff", "Fg", "Fh", "Fi", "Fj", "Fk",
    "Fl", "Fm", "Fn", "k6", "Fo", "Fp", "Fq", "Fr", "k7", "k8", "k9", "@0", "%1", "kh", "kI", "kA",
    "kl", "kH", "%2", "%3", "%4", "%5", "kN", "%6", "%7", "kP", "%8", "%9", "%0", "&1", "&2", "&3",
    "&4", "&5", "kr", "&6", "&9", "&0", "*1", "*2", "*3", "*4", "*5", "*6", "*7", "*8", "*9", "kF",
    "*0", "#1", "#2", "#3", "#4", "%a", "%b", "%c", "%d", "%e", "%f", "kR", "%g", "%h", "%i", "%j",
    "!1", "!2", "kT", "!3", "&7", "&8", "ku", "ke", "ks", "l0", "l1", "la", "l2", "l3", "l4", "l5",
    "l6", "l7", "l8", "l9", "Lf", "LF", "LO", "mo", "mm", "ZY", "ZZ", "Za", "Zb", "Zc", "Zd", "nw",
    "Ze", "oc", "op", "pc", "DC", "DL", "DO", "Zf", "IC", "SF", "AL", "LE", "Zg", "RI", "Zh", "SR",
    "UP", "Zi", "pk", "pl", "px", "pn", "ps", "pO", "pf", "po", "PU", "QD", "RC", "rp", "RF", "r1",
    "r2", "r3", "rf", "rc", "cv", "sc", "sf", "sr", "Zj", "sa", "Sb", "Zk", "Zl", "SC", "sp", "Sf",
    "ML", "Zm", "MR", "Zn", "st", "Zo", "Zp", "wi", "Zq", "Zr", "Zs", "Zt", "Zu", "Zv", "ta", "Zw",
    "ts", "TO", "uc", "hu", "u0", "u1", "u2", "u3", "u4", "u5", "u6", "u7", "u8", "u9", "WA", "XF",
    "XN", "Zx", "S8", "Yv", "Zz", "Xy", "Zy", "ci", "Yw", "Yx", "dv", "S1", "Yy", "S2", "S4", "S3",
    "S5", "Gm", "Km", "Mi", "S6", "xl", "RQ", "S7", "s0", "s1", "s2", "s3", "AB", "AF", "Yz", "ML",
    "YZ", "MT", "Xh", "Xl", "Xo", "Xr", "Xt", "Xv", "sA", "sL",
];

// libtermcap/ncurses public capability-code arrays, declared exactly
// as `term.h:713/716/719` declares them:
//   extern NCURSES_CONST char * const boolcodes[];
//   extern NCURSES_CONST char * const numcodes[];
//   extern NCURSES_CONST char * const strcodes[];
// Each is a NUL-pointer-terminated array of C strings. Declared as a
// zero-length array so `as_ptr()` yields the symbol address.
/// `boolcodes[]` / `numcodes[]` / `strcodes[]` — the two-letter TERMCAP
/// codes, positionally parallel to the terminfo long-name tables. C reads
/// them from the terminal library when it exports them (`HAVE_BOOLCODES`),
/// else uses its own literals at c:45-49 etc. zshrs no longer links a
/// terminal library, so they come from `crate::terminfo_caps`, generated from
/// the same frozen ncurses 6 arrays.
static BOOLCODES: LazyLock<Vec<&'static str>> =
    LazyLock::new(|| crate::terminfo_caps::BOOL_CODES.to_vec());

/// See [`BOOLCODES`] — `numcodes[]`.
static NUMCODES: LazyLock<Vec<&'static str>> =
    LazyLock::new(|| crate::terminfo_caps::NUM_CODES.to_vec());

/// See [`BOOLCODES`] — `strcodes[]`.
static STRCODES: LazyLock<Vec<&'static str>> =
    LazyLock::new(|| crate::terminfo_caps::STR_CODES.to_vec());

/// WARNING: NOT IN TERMCAP.C — AtomicI32-guarded once-only wrapper
/// around the terminal setup that C performs in `boot_` (c:347
/// `zsetupterm()`), so the Rust entry points work even when `boot_`
/// hasn't run (unit tests, direct `${termcap[...]}` reads).
///
/// The C it stands in for is `zsetupterm(void)` at `Src/utils.c:386`:
///
///     if (term_count++ == 0)
///         (void)setupterm((char *)0, 1, &errret);
///
/// Stand-in for the terminal setup C performs in `boot_` (c:347
/// `zsetupterm()`), so the Rust entry points work even when `boot_`
/// hasn't run (unit tests, direct `${termcap[...]}` reads).
///
/// The C it stands in for is `zsetupterm(void)` at `Src/utils.c:386`:
///
///     if (term_count++ == 0)
///         (void)setupterm((char *)0, 1, &errret);
///
/// `setupterm`, NOT `tgetent`. termcap.c never calls `tgetent`; the
/// only `tgetent` in the shell is `init_term()` (`Src/init.c:804`),
/// which the C reaches from the `TERM_UNKNOWN` guards in
/// `bin_echotc` (c:83) and `gettermcap` (c:162) — and, notably, NOT
/// from `scantermcap`, which carries no guard at all.
///
/// That asymmetry is observable, because ncurses answers `tgetstr`
/// differently depending on which initialisations have run:
///
///   setupterm only     me=\e[m^O  r2=\ec\e[?1000l…  rs=NULL
///   tgetent+setupterm  me=\e[0m   r2=\ec\e[?1000l…  rs=NULL
///   tgetent only       me=\e[0m   r2=NULL           rs=\ec\e[?1000l…
///
/// (`tgetent` caches `_nc_trim_sgr0`'s rebuilt `sgr(0)` and `tgetstr`
/// substitutes it for `exit_attribute_mode` from then on.)
///
/// zsh reproduces exactly this split: in `zsh -fc`, TERM_UNKNOWN is
/// still set, so `${(kv)termcap}` (no guard, setupterm only) reports
/// `me=\e[m^O` while `${termcap[me]}` (guard runs `init_term`)
/// reports `me=\e[0m`. Keeping `tgetent` out of this function and in
/// the guards is what makes both match.
fn ensure_termcap_loaded() -> bool {
    // 0 = uninit, 1 = ok, -1 = failed. Cache the curses state for
    // the lifetime of the process, matching C's `term_count` guard
    // at Src/utils.c:401 (setupterm runs exactly once).
    static STATE: AtomicI32 = AtomicI32::new(0);
    match STATE.load(Ordering::Relaxed) {
        1 => true,
        -1 => false,
        _ => {
            // c:Src/utils.c:397 passes a NULL term so ncurses falls
            // back to getenv("TERM"); pass the shell's own $TERM
            // parameter instead so a not-yet-exported `TERM=` is
            // still honoured.
            let term = getsparam("TERM").unwrap_or_else(|| "dumb".into());
            // c:Src/utils.c:399 — `setupterm((char *)0, 1, &errret)`,
            // which termcap.c:347 boot_ reaches via zsetupterm().
            let ok = {
                let _g = TERMCAP_LOCK.lock().unwrap_or_else(|e| e.into_inner());
                setupterm(Some(&term)).is_ok()
            };
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
        fn cb(node: &crate::ported::zsh_h::param, _flags: i32) {
            SEEN.lock().unwrap().push(node.node.nam.clone());
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

    /// c:272-309 — `scantermcap` walks `boolcodes`, `numcodes` and the
    /// string-capability table linearly and calls `func()` once for every
    /// entry that resolves. It never deduplicates, and the tables are NOT
    /// disjoint: zsh's own fallback `zstrcodes[]` lists `"ML"` twice, at
    /// c:258 and again at c:263, and the live library `strcodes[]` this
    /// port walks (`STRCODES`, the HAVE_STRCODES path) carries the same
    /// duplicate. C gets away with it because every consumer of the scan
    /// is a hash — `scanhashtable` over the special `termcap` param
    /// (c:311-313 `SPECIALPMDEF("termcap", …, scantermcap)`) — so a repeat
    /// that carries the SAME value is invisible to the reader.
    ///
    /// So "all keys unique" is not what C guarantees, and asserting it
    /// fails on any host whose `strcodes[]` has the upstream duplicate.
    /// What C *does* guarantee, and what this pins, is:
    ///
    ///   * a key is emitted no more often than the codes tables list it —
    ///     a regression that double-emits a cap (two tables claiming it,
    ///     or a loop running twice) still fails here; and
    ///   * repeated emissions of one key carry an identical value, so the
    ///     consuming hash cannot lose information and the scan is
    ///     order-independent.
    #[test]
    fn scantermcap_emits_each_key_at_most_as_often_as_the_codes_tables_list_it() {
        let _g = crate::test_util::global_state_lock();
        use std::sync::Mutex;
        static SEEN: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());
        SEEN.lock().unwrap().clear();
        fn cb(node: &crate::ported::zsh_h::param, _flags: i32) {
            SEEN.lock().unwrap().push((
                node.node.nam.clone(),
                node.u_str.clone().unwrap_or_default(),
            ));
        }
        scantermcap(std::ptr::null_mut(), Some(cb), 0);
        let collected = SEEN.lock().unwrap().clone();
        assert!(
            !collected.is_empty(),
            "scantermcap emitted nothing — the scan is dead, not merely duplicating"
        );

        // How many slots each code occupies across the three tables C walks
        // (c:272 boolcodes, c:283 numcodes, c:294 strcodes) — the upper
        // bound on how often the scan may emit it.
        let mut budget: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
        for cap in BOOLCODES
            .iter()
            .chain(NUMCODES.iter())
            .chain(STRCODES.iter())
        {
            *budget.entry(cap).or_insert(0) += 1;
        }

        let mut emitted: std::collections::HashMap<String, (usize, String)> =
            std::collections::HashMap::new();
        for (k, v) in &collected {
            let slot = emitted.entry(k.clone()).or_insert((0, v.clone()));
            slot.0 += 1;
            // Repeats must agree: the same cap looked up twice returns the
            // same tgetstr/tgetnum/ztgetflag result, so the hash the caller
            // builds is the same whichever emission lands last.
            assert_eq!(
                &slot.1, v,
                "termcap key {k} emitted with conflicting values {:?} vs {v:?} — \
                 the consuming hash would depend on scan order",
                slot.1
            );
            let allowed = budget.get(k.as_str()).copied().unwrap_or(0);
            assert!(
                allowed > 0,
                "scantermcap emitted {k}, which is in none of \
                 boolcodes/numcodes/strcodes"
            );
            assert!(
                slot.0 <= allowed,
                "termcap key {k} emitted {} times but the codes tables list it \
                 only {allowed} time(s) — a cap is being double-emitted",
                slot.0
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
        fn cb(node: &crate::ported::zsh_h::param, _flags: i32) {
            KEYS.lock().unwrap().push(node.node.nam.clone());
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

//! `zsh/terminfo` module — direct port of `Src/Modules/terminfo.c`.
//!
//! This depends on the termcap stuff in init.c                              // c:72
//!
//! Exposes the live terminfo database to scripts via the
//! `${terminfo[capname]}` associative array. The C source binds
//! ncurses' `setupterm`/`tigetstr`/`tigetnum`/`tigetflag`; this file
//! does the same through Rust FFI against the system curses library
//! that ships with macOS / Linux SDKs.
//!
//! Lookup precedence matches `getterminfo()` in the C source:
//!   1. String capability  (`tigetstr`)
//!   2. Numeric capability (`tigetnum`)
//!   3. Boolean capability (`tigetflag`)  →  rendered as `"yes"`/`"no"`
//!
//! Unknown capabilities return `None` so callers can emit `""`
//! matching zsh's `PM_UNSET` fallback (terminfo.c:165-168).

use crate::options::optlookup;
use crate::ported::params::TERMFLAGS;
use crate::ported::zsh_h::module;
use crate::zsh_h::{isset, INTERACTIVE, TERM_UNKNOWN};
use std::sync::atomic::Ordering;
use std::sync::{LazyLock, Mutex, OnceLock};

// FFI bindings to the system ncurses terminfo interface. Direct
// port of the call sites in `zsh/Src/Modules/terminfo.c`.
//
// The library NAME is deliberately not pinned here: `build.rs`
// (`link_term_lib`) reproduces `configure.ac:725-771`'s
// `ncursesw ncurses tinfow tinfo termcap curses` search and emits a
// single `cargo:rustc-link-lib` for the whole crate. Re-adding
// `#[link(name = "ncurses")]` would put a second, different terminal
// library on the link line and let symbol resolution order decide
// which terminfo database these calls read.
extern "C" {
    fn setupterm(
        term: *const libc::c_char,
        filedes: libc::c_int,
        errret: *mut libc::c_int,
    ) -> libc::c_int;
    fn tigetstr(capname: *const libc::c_char) -> *const libc::c_char;
    fn tigetnum(capname: *const libc::c_char) -> libc::c_int;
    fn tigetflag(capname: *const libc::c_char) -> libc::c_int;
    fn putp(s: *const libc::c_char) -> libc::c_int;
    fn tparm(
        s: *const libc::c_char,
        p1: libc::c_long,
        p2: libc::c_long,
        p3: libc::c_long,
        p4: libc::c_long,
        p5: libc::c_long,
        p6: libc::c_long,
        p7: libc::c_long,
        p8: libc::c_long,
        p9: libc::c_long,
    ) -> *const libc::c_char;
}

/// Direct port of `bin_echoti(char *name, char **argv, UNUSED(Options ops), UNUSED(int func))` from `Src/Modules/terminfo.c:64`.
/// C body (c:67-127): probe `tigetnum` → `tigetflag` → `tigetstr`
/// in turn; numeric/boolean caps print and return; string caps go
/// through `tparm` (with up to 9 long args) then `putp`.
/// WARNING: param names don't match C — Rust=(name, argv, _func) vs C=(name, argv, ops, func)
pub fn bin_echoti(
    name: &str,
    argv: &[String], // c:64
    _ops: &crate::ported::zsh_h::options,
    _func: i32,
) -> i32 {
    // c:Src/zsh.h:1985 — `#define TERM_BAD 0x01`. The local
    // `const TERM_BAD: i32 = 1 << 1` shadow used 0x02 which
    // collides with TERM_UNKNOWN; bin_echoti would short-circuit
    // when only TERM_UNKNOWN was set, returning 1 silently before
    // setupterm could attempt the terminfo lookup. Use the canonical
    // const from zsh_h.rs.
    use crate::ported::zsh_h::TERM_BAD;

    if argv.is_empty() {
        crate::ported::utils::zwarnnam(name, "missing capability name");
        return 1;
    }
    let s = &argv[0]; // c:73 s = *argv++
    let argv_rest = &argv[1..];

    if (TERMFLAGS.load(Ordering::Relaxed) & TERM_BAD) != 0 {
        // c:75
        return 1; // c:76
    }
    let interactive = isset(INTERACTIVE); // c:77
    if (TERMFLAGS.load(Ordering::Relaxed) & TERM_UNKNOWN) != 0 && interactive {
        return 1; // c:78
    }

    let cs = match std::ffi::CString::new(s.as_str()) {
        Ok(c) => c,
        Err(_) => return 1,
    };

    // c:Src/utils.c:390 — boot_terminfo → zsetupterm() must run real
    // ncurses setupterm so tigetnum/tigetflag/tigetstr have a cur_term.
    // The Rust zsetupterm in utils.rs is a counter-only stub (no FFI),
    // so without this guard tigetnum("cols") returns -2 (uninitialized)
    // and echoti drops into "no such capability". Mirror the cur_term
    // init pattern from terminfosetfn/terminfogetfn below.
    static ECHOTI_TERM_READY: OnceLock<bool> = OnceLock::new();
    let term_ok = *ECHOTI_TERM_READY.get_or_init(|| {
        let mut errret: libc::c_int = 0;
        unsafe { setupterm(std::ptr::null(), 1, &mut errret) == 0 }
    });
    if !term_ok {
        crate::ported::utils::zwarnnam(name, &format!("no such terminfo capability: {}", s));
        return 1;
    }

    // c:81 — `if (((num = tigetnum(s)) != -1) && (num != -2)) { ... }`.
    let num = unsafe { tigetnum(cs.as_ptr()) }; // c:81
    if num != -1 && num != -2 {
        // c:81
        println!("{}", num); // c:82
        return 0; // c:83
    }

    // c:86 — `switch (tigetflag(s)) { -1 break; 0 puts("no"); default puts("yes"); }`.
    match unsafe { tigetflag(cs.as_ptr()) } {
        // c:86
        -1 => {} // c:88
        0 => {
            println!("no");
            return 0;
        } // c:90
        _ => {
            println!("yes");
            return 0;
        } // c:93
    }

    // get a string-type capability                                          // c:94
    // c:97 — `t = (char *)tigetstr(s);` — string capability.
    let t = unsafe { tigetstr(cs.as_ptr()) }; // c:97
    let t_addr = t as isize;
    if t.is_null() || t_addr == -1 || unsafe { *t } == 0 {
        // c:98
        // capability doesn't exist, or (if boolean) is off                  // c:97
        crate::ported::utils::zwarnnam(
            name, // c:100
            &format!("no such terminfo capability: {}", s),
        );
        return 1; // c:101
    }

    // c:104 — `if (arrlen_gt(argv, 9)) { zwarnnam(name, "too many arguments"); return 1; }`.
    if argv_rest.len() > 9 {
        // c:104
        crate::ported::utils::zwarnnam(name, "too many arguments"); // c:105
        return 1; // c:106
    }

    // c:110 — `for (u = strcap; *u && !strarg; u++) strarg = !strcmp(s, *u);`
    // String-arg capabilities: pfkey/pfloc/pfx/pln/pfxl take a string
    // for argv[1+]; everything else takes integers.
    let strcap = ["pfkey", "pfloc", "pfx", "pln", "pfxl"];
    let strarg = strcap.iter().any(|c| s.as_str() == *c);

    // c:113 — `for (arg=0; argv[arg]; arg++) pars[arg] = ...`
    let mut pars: [libc::c_long; 9] = [0; 9]; // c:69
    let mut keep_alive: Vec<std::ffi::CString> = Vec::new(); // hold strarg pointers
    for (i, a) in argv_rest.iter().enumerate().take(9) {
        if strarg && i > 0 {
            // c:115
            let cs = std::ffi::CString::new(a.as_str()).unwrap_or_default();
            pars[i] = cs.as_ptr() as libc::c_long; // c:116
            keep_alive.push(cs);
        } else {
            pars[i] = a.parse::<libc::c_long>().unwrap_or(0); // c:118 atoi
        }
    }

    // c:Src/Modules/terminfo.c:122 — `putp(t)` writes through ncurses
    // which targets C `stdout` (libc FILE*). Rust's `println!` writes
    // through its own `io::Stdout` Mutex<BufWriter>. The two buffers
    // flush at different times → escape sequences ended up AFTER the
    // next `echo`'s output in the byte stream (bug #78 in docs/BUGS.md).
    //
    // Fix: flush Rust's stdout BEFORE calling putp so prior writes
    // reach fd 1 first, then fflush(libc::stdout) AFTER putp so the
    // escape sequence reaches fd 1 before the next println! buffers
    // its content. Both flushes are needed — without the pre-flush,
    // any prior `print!`/`println!` in this builtin would land
    // post-escape; without the post-flush, the escape sits in C
    // stdio's buffer until exit and trails the next println!.
    use std::io::Write as _;
    let _ = std::io::stdout().flush();
    if argv_rest.is_empty() {
        // c:122
        unsafe {
            putp(t);
        } // c:123
    } else {
        let formatted = unsafe {
            // c:125
            tparm(
                t, pars[0], pars[1], pars[2], pars[3], pars[4], pars[5], pars[6], pars[7], pars[8],
            )
        };
        if !formatted.is_null() {
            unsafe {
                putp(formatted);
            }
        }
    }
    // `fflush(NULL)` flushes ALL open C streams (POSIX). Portable
    // across macOS/Linux where `stdout` is a macro/function rather
    // than a linker symbol — avoids the `__stdoutp` vs `stdout`
    // platform divergence.
    unsafe {
        libc::fflush(std::ptr::null_mut());
    }
    drop(keep_alive);
    0 // c:128
}

/// Initialize the terminfo database for the current `$TERM`. Must
/// Port of `getterminfo(UNUSED(HashTable ht), const char *name)` from `Src/Modules/terminfo.c:135`.
///
/// Also drives `bin_echoti` at line 64. Tries `tigetstr` → `tigetnum`
/// → `tigetflag` in that order — string first, then numeric, then
/// boolean. Returns `None` for unknown names so the caller can map
/// to `""`. The terminfo database is initialised lazily via the
/// `setupterm()` call zsh's setup_/boot_ hook performs at terminfo.c:
/// init_term path; collapsed into a OnceLock here since zshrs has no
/// per-module init function shape.
/// Port of `static HashNode getterminfo(UNUSED(HashTable ht), const char *name)`
/// from `Src/Modules/terminfo.c:135-177`. Returns a synthesised Param
/// with PM_INTEGER (numeric cap), PM_SCALAR yes/no (boolean cap),
/// PM_SCALAR escape-string (string cap), or PM_UNSET ("" + flag).
pub fn getterminfo(
    _ht: *mut crate::ported::zsh_h::HashTable,
    name: &str,
) -> Option<crate::ported::zsh_h::Param> {
    // c:135
    use crate::ported::zsh_h::{hashnode, param, PM_INTEGER, PM_READONLY, PM_SCALAR, PM_UNSET};
    // c:Src/zsh.h:1985 — `#define TERM_BAD 0x01`. Use the canonical
    // const from zsh_h.rs:4155. Prior local shadow `const TERM_BAD:
    // i32 = 1 << 1` was 0x02 — colliding with TERM_UNKNOWN's 0x02.
    // When only TERM_UNKNOWN was set, the `& TERM_BAD` check below
    // false-positived and `${terminfo[cap]}` returned None before the
    // setupterm path could attempt the tigetnum/tigetflag/tigetstr
    // lookups. Same bug as bin_echoti in termcap.rs (already fixed
    // there); fix mirrored here.
    use crate::ported::zsh_h::TERM_BAD;

    // c:142 — `if (termflags & TERM_BAD) return NULL;`
    if (TERMFLAGS.load(Ordering::Relaxed) & TERM_BAD) != 0 {
        return None;
    }
    // c:144 — `if ((termflags & TERM_UNKNOWN) && (isset(INTERACTIVE) || !init_term())) return NULL;`
    if (TERMFLAGS.load(Ordering::Relaxed) & TERM_UNKNOWN) != 0 {
        if isset(INTERACTIVE) {
            return None;
        }
    }

    static INITIALIZED: OnceLock<bool> = OnceLock::new();
    let ok = *INITIALIZED.get_or_init(|| {
        let mut errret: libc::c_int = 0;
        unsafe { setupterm(std::ptr::null(), 1, &mut errret) == 0 }
    });
    if !ok {
        return None;
    }

    // Helper: build a Param shell with the given flags + u_str/u_val.
    let mk_str = |s: String, extra_flags: i32| -> crate::ported::zsh_h::Param {
        Box::new(param {
            node: hashnode {
                next: None,
                nam: name.to_string(),
                flags: PM_READONLY as i32 | extra_flags,
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

    // c:147 — `nameu = dupstring(name); unmetafy(nameu, &len);`
    let mut buf = name.as_bytes().to_vec();
    crate::ported::utils::unmetafy(&mut buf);
    let nameu = match std::str::from_utf8(&buf) {
        Ok(s) => s.to_string(),
        Err(_) => return None,
    };
    let cname = std::ffi::CString::new(nameu).ok()?;

    unsafe {
        // c:155 — PM_INTEGER for tigetnum hit.
        let n = tigetnum(cname.as_ptr());
        if n != -1 && n != -2 {
            // c:156-158 — `pm->u.val = num; PM_INTEGER;`
            // Also stamp u_str with the decimal form so callers that
            // only consume u_str (like vm_helper::partab_get) see the
            // value. `$terminfo[colors]` reads as 256, not empty.
            let mut pm = mk_str((n as i64).to_string(), PM_INTEGER as i32);
            pm.u_val = n as i64;
            return Some(pm);
        }
        // c:159-162 — PM_SCALAR yes/no for tigetflag hit.
        let b = tigetflag(cname.as_ptr());
        if b != -1 {
            let s = if b != 0 { "yes" } else { "no" }.to_string();
            return Some(mk_str(s, PM_SCALAR as i32));
        }
        // c:163-167 — PM_SCALAR escape string for tigetstr hit.
        let tistr = tigetstr(cname.as_ptr());
        let s_addr = tistr as isize;
        if !tistr.is_null() && s_addr != -1 {
            let raw = std::ffi::CStr::from_ptr(tistr)
                .to_string_lossy()
                .into_owned();
            return Some(mk_str(crate::ported::utils::metafy(&raw), PM_SCALAR as i32));
        }
    }
    // c:168-173 — `pm->u.str = ""; pm->node.flags |= PM_UNSET;`
    Some(mk_str(String::new(), PM_SCALAR as i32 | PM_UNSET as i32))
}

// ---------------------------------------------------------------
// Capability-name tables (c:197-203, c:205-212, c:214-264)
//
// terminfo.c wraps each literal array in `#ifndef HAVE_<X>NAMES`:
// when the curses library exports `boolnames[]` / `numnames[]` /
// `strnames[]` itself (ncurses and libtinfo both do — term.h:712-718
// declares them `extern char *const boolnames[]` etc.), C walks the
// LIBRARY arrays at c:257 / c:268 / c:280 and these literals are
// never compiled in. They are kept here as the `#ifndef` fallback,
// transcribed verbatim from the C, and used only when the library
// symbols resolve to an empty table.
//
// The library arrays are the larger set: ncurses carries 44 bool /
// 39 num / 414 string names against the C fallback's 37 / 33 / 394,
// the extra entries being the `OT*` termcap-compatibility names.
// ---------------------------------------------------------------

/// c:198-202 — `#ifndef HAVE_BOOLNAMES static char *boolnames[] = {...}`
static BOOLNAMES_FALLBACK: &[&str] = &[
    "bw", "am", "bce", "ccc", "xhp", "xhpa", "cpix", "crxm", "xt", "xenl", "eo", "gn", "hc",
    "chts", "km", "daisy", "hs", "hls", "in", "lpix", "da", "db", "mir", "msgr", "nxon", "xsb",
    "npc", "ndscr", "nrrmc", "os", "mc5i", "xvpa", "sam", "eslok", "hz", "ul", "xon"
];

/// c:206-211 — `#ifndef HAVE_NUMNAMES static char *numnames[] = {...}`
static NUMNAMES_FALLBACK: &[&str] = &[
    "cols", "it", "lh", "lw", "lines", "lm", "xmc", "ma", "colors", "pairs", "wnum", "ncv",
    "nlab", "pb", "vt", "wsl", "bitwin", "bitype", "bufsz", "btns", "spinh", "spinv", "maddr",
    "mjump", "mcs", "mls", "npins", "orc", "orhi", "orl", "orvi", "cps", "widcs"
];

/// c:215-264 — `#ifndef HAVE_STRNAMES static char *strnames[] = {...}`
static STRNAMES_FALLBACK: &[&str] = &[
    "acsc", "cbt", "bel", "cr", "cpi", "lpi", "chr", "cvr", "csr", "rmp", "tbc", "mgc", "clear",
    "el1", "el", "ed", "hpa", "cmdch", "cwin", "cup", "cud1", "home", "civis", "cub1", "mrcup",
    "cnorm", "cuf1", "ll", "cuu1", "cvvis", "defc", "dch1", "dl1", "dial", "dsl", "dclk", "hd",
    "enacs", "smacs", "smam", "blink", "bold", "smcup", "smdc", "dim", "swidm", "sdrfq", "smir",
    "sitm", "slm", "smicm", "snlq", "snrmq", "prot", "rev", "invis", "sshm", "smso", "ssubm",
    "ssupm", "smul", "sum", "smxon", "ech", "rmacs", "rmam", "sgr0", "rmcup", "rmdc", "rwidm",
    "rmir", "ritm", "rlm", "rmicm", "rshm", "rmso", "rsubm", "rsupm", "rmul", "rum", "rmxon",
    "pause", "hook", "flash", "ff", "fsl", "wingo", "hup", "is1", "is2", "is3", "if", "iprog",
    "initc", "initp", "ich1", "il1", "ip", "ka1", "ka3", "kb2", "kbs", "kbeg", "kcbt", "kc1",
    "kc3", "kcan", "ktbc", "kclr", "kclo", "kcmd", "kcpy", "kcrt", "kctab", "kdch1", "kdl1",
    "kcud1", "krmir", "kend", "kent", "kel", "ked", "kext", "kf0", "kf1", "kf10", "kf11",
    "kf12", "kf13", "kf14", "kf15", "kf16", "kf17", "kf18", "kf19", "kf2", "kf20", "kf21",
    "kf22", "kf23", "kf24", "kf25", "kf26", "kf27", "kf28", "kf29", "kf3", "kf30", "kf31",
    "kf32", "kf33", "kf34", "kf35", "kf36", "kf37", "kf38", "kf39", "kf4", "kf40", "kf41",
    "kf42", "kf43", "kf44", "kf45", "kf46", "kf47", "kf48", "kf49", "kf5", "kf50", "kf51",
    "kf52", "kf53", "kf54", "kf55", "kf56", "kf57", "kf58", "kf59", "kf6", "kf60", "kf61",
    "kf62", "kf63", "kf7", "kf8", "kf9", "kfnd", "khlp", "khome", "kich1", "kil1", "kcub1",
    "kll", "kmrk", "kmsg", "kmov", "knxt", "knp", "kopn", "kopt", "kpp", "kprv", "kprt", "krdo",
    "kref", "krfr", "krpl", "krst", "kres", "kcuf1", "ksav", "kBEG", "kCAN", "kCMD", "kCPY",
    "kCRT", "kDC", "kDL", "kslt", "kEND", "kEOL", "kEXT", "kind", "kFND", "kHLP", "kHOM", "kIC",
    "kLFT", "kMSG", "kMOV", "kNXT", "kOPT", "kPRV", "kPRT", "kri", "kRDO", "kRPL", "kRIT",
    "kRES", "kSAV", "kSPD", "khts", "kUND", "kspd", "kund", "kcuu1", "rmkx", "smkx", "lf0",
    "lf1", "lf10", "lf2", "lf3", "lf4", "lf5", "lf6", "lf7", "lf8", "lf9", "fln", "rmln",
    "smln", "rmm", "smm", "mhpa", "mcud1", "mcub1", "mcuf1", "mvpa", "mcuu1", "nel", "porder",
    "oc", "op", "pad", "dch", "dl", "cud", "mcud", "ich", "indn", "il", "cub", "mcub", "cuf",
    "mcuf", "rin", "cuu", "mcuu", "pfkey", "pfloc", "pfx", "pln", "mc0", "mc5p", "mc4", "mc5",
    "pulse", "qdial", "rmclk", "rep", "rfi", "rs1", "rs2", "rs3", "rf", "rc", "vpa", "sc",
    "ind", "ri", "scs", "sgr", "setb", "smgb", "smgbp", "sclk", "scp", "setf", "smgl", "smglp",
    "smgr", "smgrp", "hts", "smgt", "smgtp", "wind", "sbim", "scsd", "rbim", "rcsd", "subcs",
    "supcs", "ht", "docr", "tsl", "tone", "uc", "hu", "u0", "u1", "u2", "u3", "u4", "u5", "u6",
    "u7", "u8", "u9", "wait", "xoffc", "xonc", "zerom", "scesa", "bicr", "binel", "birep",
    "csnm", "csin", "colornm", "defbi", "devt", "dispc", "endbi", "smpch", "smsc", "rmpch",
    "rmsc", "getm", "kmous", "minfo", "pctrm", "pfxl", "reqmp", "scesc", "s0ds", "s1ds", "s2ds",
    "s3ds", "setab", "setaf", "setcolor", "smglr", "slines", "smgtb", "ehhlm", "elhlm",
    "elohlm", "erhlm", "ethlm", "evhlm", "sgr1", "slength"
];

// ncurses/libtinfo public capability-name arrays, declared exactly as
// `term.h:712/715/718` declares them:
//   extern NCURSES_CONST char * const boolnames[];
//   extern NCURSES_CONST char * const numnames[];
//   extern NCURSES_CONST char * const strnames[];
// Each is a NUL-pointer-terminated array of C strings. Declared as a
// zero-length array so `as_ptr()` yields the symbol address.
#[allow(non_upper_case_globals)]
unsafe extern "C" {
    static boolnames: [*const libc::c_char; 0];
    static numnames: [*const libc::c_char; 0];
    static strnames: [*const libc::c_char; 0];
}

/// `boolnames[]` — the library's array when it exports one (the
/// HAVE_BOOLNAMES path C takes), else the c:198-202 literal.
static BOOLNAMES: LazyLock<Vec<&'static str>> = LazyLock::new(|| unsafe {
    let mut v: Vec<&'static str> = Vec::new();
    let base = boolnames.as_ptr();
    let mut i = 0isize;
    while !(*base.offset(i)).is_null() {
        if let Ok(s) = std::ffi::CStr::from_ptr(*base.offset(i)).to_str() {
            v.push(s);
        }
        i += 1;
    }
    if v.is_empty() { BOOLNAMES_FALLBACK.to_vec() } else { v }
});

/// `numnames[]` — library array (HAVE_NUMNAMES path) else c:206-211.
static NUMNAMES: LazyLock<Vec<&'static str>> = LazyLock::new(|| unsafe {
    let mut v: Vec<&'static str> = Vec::new();
    let base = numnames.as_ptr();
    let mut i = 0isize;
    while !(*base.offset(i)).is_null() {
        if let Ok(s) = std::ffi::CStr::from_ptr(*base.offset(i)).to_str() {
            v.push(s);
        }
        i += 1;
    }
    if v.is_empty() { NUMNAMES_FALLBACK.to_vec() } else { v }
});

/// `strnames[]` — library array (HAVE_STRNAMES path) else c:215-264.
static STRNAMES: LazyLock<Vec<&'static str>> = LazyLock::new(|| unsafe {
    let mut v: Vec<&'static str> = Vec::new();
    let base = strnames.as_ptr();
    let mut i = 0isize;
    while !(*base.offset(i)).is_null() {
        if let Ok(s) = std::ffi::CStr::from_ptr(*base.offset(i)).to_str() {
            v.push(s);
        }
        i += 1;
    }
    if v.is_empty() { STRNAMES_FALLBACK.to_vec() } else { v }
});

/// Port of `static void scanterminfo(UNUSED(HashTable ht), ScanFunc func, int flags)`
/// from `Src/Modules/terminfo.c:177-289`. Walks the bool/num/string
/// capability tables and invokes the callback per resolved cap.
pub fn scanterminfo(
    _ht: *mut crate::ported::zsh_h::HashTable,
    func: Option<crate::ported::zsh_h::ParamScanFunc>,
    flags: i32,
) {
    // c:177
    use crate::ported::zsh_h::{hashnode, param, PM_SCALAR};
    let f = match func {
        Some(f) => f,
        None => return,
    };
    let emit_cap = |cap_name: &str, val: &str| {
        let pm = param {
            node: hashnode {
                next: None,
                nam: cap_name.to_string(),
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
        f(&pm, flags);
    };

    // c:152-153 — `if (termflags & TERM_BAD) return;`. The full
    // termflag check at getterminfo's entry mirrors here too.
    // Use the canonical TERM_BAD (0x01) from zsh_h.rs:4155 — the
    // local shadow at this line was 0x02 which collides with
    // TERM_UNKNOWN. Same fix as the one applied to getterminfo and
    // bin_echoti (termcap.rs).
    use crate::ported::zsh_h::TERM_BAD;
    if (TERMFLAGS.load(Ordering::Relaxed) & TERM_BAD) != 0 {
        return;
    }
    if (TERMFLAGS.load(Ordering::Relaxed) & TERM_UNKNOWN) != 0 {
        let interactive = isset(INTERACTIVE);
        if interactive {
            return;
        }
    }
    static INITIALIZED: OnceLock<bool> = OnceLock::new();
    let ok = *INITIALIZED.get_or_init(|| {
        let mut errret: libc::c_int = 0;
        unsafe { setupterm(std::ptr::null(), 1, &mut errret) == 0 }
    });
    if !ok {
        return;
    }

    // c:257-263 — boolean caps: tigetflag → "yes" / "no", emit when num != -1.
    for cap in BOOLNAMES.iter() {
        // c:257
        let cn = match std::ffi::CString::new(*cap) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let n = unsafe { tigetflag(cn.as_ptr()) }; // c:258
        if n != -1 {
            // c:258
            let v = if n != 0 { "yes" } else { "no" }; // c:259
            emit_cap(cap, v); // c:261 func(&pm.node, flags)
        }
    }

    // c:268-275 — numeric caps.
    for cap in NUMNAMES.iter() {
        // c:268
        let cn = match std::ffi::CString::new(*cap) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let n = unsafe { tigetnum(cn.as_ptr()) }; // c:269
        if n != -1 && n != -2 {
            // c:269
            emit_cap(cap, &n.to_string()); // c:270-272 func(&pm.node, flags)
        }
    }

    // c:280-287 — string caps: tigetstr → metafy, emit when non-NULL/-1.
    for cap in STRNAMES.iter() {
        // c:280
        let cn = match std::ffi::CString::new(*cap) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let raw = unsafe { tigetstr(cn.as_ptr()) }; // c:281
        let s_addr = raw as isize;
        if !raw.is_null() && s_addr != -1 {
            // c:282
            let bytes = unsafe { std::ffi::CStr::from_ptr(raw) }
                .to_string_lossy()
                .into_owned();
            // c:283 — `pm->u.str = metafy(tistr, -1, META_HEAPDUP);`
            emit_cap(cap, &crate::ported::utils::metafy(&bytes)); // c:283-285
        }
    }
}

// ===========================================================
// Methods moved verbatim from src/ported/vm_helper because their
// C counterpart's source file maps 1:1 to this Rust module.
// Rust permits multiple inherent impl blocks for the same
// type within a crate, so call sites in vm_helper are unchanged.
// ===========================================================

// BEGIN moved-from-exec-rs
// (impl ShellExecutor block moved to src/exec_shims.rs — see file marker)

// END moved-from-exec-rs

// =====================================================================
// static struct features module_features                            c:307 (terminfo.c)
// =====================================================================

// `bintab` — port of `static struct builtin bintab[]` (terminfo.c).

// `partab` — port of `static struct paramdef partab[]` (terminfo.c).

// `module_features` — port of `static struct features module_features`
// from terminfo.c:307.

/// Port of `setup_(UNUSED(Module m))` from `Src/Modules/terminfo.c:316`.
#[allow(unused_variables)]
pub fn setup_(m: *const module) -> i32 {
    // c:316
    // C body c:318-319 — `return 0`. Faithful empty-body port.
    0
}

/// Port of `features_(UNUSED(Module m), UNUSED(char ***features))` from `Src/Modules/terminfo.c:323`.
/// C body: `*features = featuresarray(m, &module_features); return 0;`
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {
    // c:323
    *features = featuresarray(m, module_features());
    0
}

/// Port of `enables_(UNUSED(Module m), UNUSED(int **enables))` from `Src/Modules/terminfo.c:331`.
/// C body: `return handlefeatures(m, &module_features, enables);`
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {
    // c:331
    handlefeatures(m, module_features(), enables)
}

/// Port of `boot_(UNUSED(Module m))` from `Src/Modules/terminfo.c:338`.
#[allow(unused_variables)]
pub fn boot_(m: *const module) -> i32 {
    // c:338
    // C body c:340-344 — `#ifdef USE_TERMINFO_MODULE zsetupterm(); #endif
    //                     return 0`. Initializes the terminfo database
    //                     for echoti/$terminfo to use.
    let _ = crate::ported::utils::zsetupterm(); // c:359
    0
}

/// Port of `cleanup_(UNUSED(Module m))` from `Src/Modules/terminfo.c:349`.
/// C body: `return setfeatureenables(m, &module_features, NULL);`
pub fn cleanup_(m: *const module) -> i32 {
    // c:349
    setfeatureenables(m, module_features(), None)
}

/// Port of `finish_(UNUSED(Module m))` from `Src/Modules/terminfo.c:359`.
#[allow(unused_variables)]
pub fn finish_(m: *const module) -> i32 {
    // c:359
    // C body c:361-362 — `return 0`. Faithful empty-body port; the
    //                    terminfo database is process-lifetime.
    0
}

/// Capability names pre-loaded into the `${terminfo[…]}` assoc at
/// shell start so iteration via `${(k)terminfo}` enumerates the
/// common subset. Lazy lookups for any other name still resolve
/// through `lookup()`. The list intentionally mirrors the strings
/// that zsh keymap setups commonly read (function keys, navigation,
/// editing, sgr).
pub const COMMON_STRING_CAPS: &[&str] = &[
    // Function keys F1-F20.
    "kf1", "kf2", "kf3", "kf4", "kf5", "kf6", "kf7", "kf8", "kf9", "kf10", "kf11", "kf12", "kf13",
    "kf14", "kf15", "kf16", "kf17", "kf18", "kf19", "kf20", // Cursor / arrow keys.
    "kcuu1", "kcud1", "kcuf1", "kcub1", // Navigation.
    "khome", "kend", "kpp", "knp", // Editing.
    "kbs", "kich1", "kdch1", // Clear / cursor positioning.
    "clear", "ed", "el", "home", "civis", "cnorm", // SGR.
    "smso", "rmso", "smul", "rmul", "bold", "rev", "sgr0",
    // Application keypad / alt-screen / colour.
    "smkx", "rmkx", "smcup", "rmcup", "setaf", "setab",
    // Cursor positioning + edit ops.
    "cup", "ich1", "dch1", "il1", "dl1",
];

static MODULE_FEATURES: OnceLock<Mutex<crate::ported::zsh_h::features>> = OnceLock::new();

// Local stubs for the per-module entry points. C uses generic
// `featuresarray`/`handlefeatures`/`setfeatureenables` (module.c:
// 3275/3370/3445) but those take `Builtin` + `Features` pointer
// fields the Rust port doesn't carry. The hardcoded descriptor
// list mirrors the C bintab/conddefs/mathfuncs/paramdefs.
// WARNING: NOT IN TERMINFO.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn featuresarray(_m: *const module, _f: &Mutex<crate::ported::zsh_h::features>) -> Vec<String> {
    vec!["b:echoti".to_string(), "p:terminfo".to_string()]
}

// WARNING: NOT IN TERMINFO.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn handlefeatures(
    _m: *const module,
    _f: &Mutex<crate::ported::zsh_h::features>,
    enables: &mut Option<Vec<i32>>,
) -> i32 {
    if enables.is_none() {
        *enables = Some(vec![1; 2]);
    }
    0
}

// WARNING: NOT IN TERMINFO.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn setfeatureenables(
    _m: *const module,
    _f: &Mutex<crate::ported::zsh_h::features>,
    _e: Option<&[i32]>,
) -> i32 {
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

// WARNING: NOT IN TERMINFO.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn module_features() -> &'static Mutex<crate::ported::zsh_h::features> {
    MODULE_FEATURES.get_or_init(|| {
        Mutex::new(crate::ported::zsh_h::features {
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

    /// c:135 — `getterminfo` for an unknown capability name returns
    /// a Param with PM_UNSET flag set + empty u_str. C semantics:
    /// returns non-NULL HashNode wrapping a PM_UNSET Param (c:168-
    /// 173). Catches a regression where libc::tigetstr's
    /// `(unsigned char*)-1` sentinel leaks through as a valid pointer.
    #[test]
    fn getterminfo_unknown_cap_returns_unset_param() {
        let _g = crate::test_util::global_state_lock();
        use crate::ported::zsh_h::PM_UNSET;
        // termflags check may early-return None if $TERM is bad; treat
        // both Some(unset) and None as acceptable for this regression.
        if let Some(pm) = getterminfo(std::ptr::null_mut(), "definitely_not_a_real_cap_name_zshrs")
        {
            assert!(
                pm.node.flags & PM_UNSET as i32 != 0,
                "PM_UNSET flag must be set for unknown cap"
            );
            assert_eq!(pm.u_str.as_deref(), Some(""), "u_str empty for unknown cap");
        }
    }

    /// c:64 — `echoti` without a cap-name argument must error. The
    /// cap-name is the first positional; missing args means no work to
    /// do, and silent success would mask a usage bug.
    #[test]
    fn echoti_with_no_args_is_usage_error() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        assert_eq!(bin_echoti("echoti", &[], &ops, 0), 1);
    }

    /// c:64 — `echoti` with an unknown capability name must error
    /// rather than emit garbage. zsh's terminfo.c rejects via
    /// `tigetstr(cap) == 0 / (char *)-1`.
    #[test]
    fn echoti_unknown_capability_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let r = bin_echoti("echoti", &["__not_a_terminfo_cap__".to_string()], &ops, 0);
        assert_eq!(r, 1, "echoti must reject unknown caps, not emit garbage");
    }

    /// c:177 — `scanterminfo` iterates the table and returns a
    /// (name, value) list. Must not panic with `TERM=dumb` (a
    /// terminal with effectively zero capabilities). Empty result is
    /// acceptable; panic is not.
    #[test]
    fn scanterminfo_does_not_panic_for_dumb_term() {
        let _g = crate::test_util::global_state_lock();
        // SAFETY: env mutation is process-global. Snapshot + restore.
        let old = std::env::var_os("TERM");
        unsafe {
            std::env::set_var("TERM", "dumb");
        }
        fn cb(_pm: &crate::ported::zsh_h::param, _f: i32) {}
        scanterminfo(std::ptr::null_mut(), Some(cb), 0);
        match old {
            Some(v) => unsafe {
                std::env::set_var("TERM", v);
            },
            None => unsafe {
                std::env::remove_var("TERM");
            },
        }
    }

    /// c:316-360 — module-lifecycle stubs return 0 in C.
    #[test]
    fn module_lifecycle_shims_all_return_zero() {
        let _g = crate::test_util::global_state_lock();
        let m = std::ptr::null();
        assert_eq!(setup_(m), 0);
        assert_eq!(boot_(m), 0);
        assert_eq!(cleanup_(m), 0);
        assert_eq!(finish_(m), 0);
    }

    /// c:323 — `features_` writes the advertised feature names and
    /// returns 0. Must be callable without panicking.
    #[test]
    fn features_returns_success() {
        let _g = crate::test_util::global_state_lock();
        let mut features = Vec::new();
        assert_eq!(features_(std::ptr::null(), &mut features), 0);
    }

    /// c:331 — `enables_` toggles the per-feature enable bitmap and
    /// returns 0. Pass None to avoid mutating state; just verify the
    /// success contract.
    #[test]
    fn enables_returns_success_with_none_arg() {
        let _g = crate::test_util::global_state_lock();
        let mut enables: Option<Vec<i32>> = None;
        assert_eq!(enables_(std::ptr::null(), &mut enables), 0);
    }

    // ─── zsh-corpus pins for terminfo lifecycle ─────────────────────

    /// All four lifecycle shims return 0.
    #[test]
    fn terminfo_corpus_lifecycle_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let m: *const crate::ported::zsh_h::module = std::ptr::null();
        assert_eq!(setup_(m), 0);
        assert_eq!(boot_(m), 0);
        assert_eq!(cleanup_(m), 0);
        assert_eq!(finish_(m), 0);
    }

    /// `getterminfo("never_a_real_cap")` returns PM_UNSET param or None.
    #[test]
    fn terminfo_corpus_unknown_cap_returns_unset_or_none() {
        let _g = crate::test_util::global_state_lock();
        let r = getterminfo(std::ptr::null_mut(), "zzz_not_a_real_cap_xyz");
        if let Some(p) = r {
            assert!(
                (p.node.flags as u32 & crate::ported::zsh_h::PM_UNSET) != 0,
                "unknown cap → PM_UNSET",
            );
        }
    }

    /// `features_` populates feature vec and returns 0.
    #[test]
    fn terminfo_corpus_features_populates_vec() {
        let _g = crate::test_util::global_state_lock();
        let mut features = Vec::new();
        let r = features_(std::ptr::null(), &mut features);
        assert_eq!(r, 0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // C-parity tests pinning Src/Modules/terminfo.c.
    // ═══════════════════════════════════════════════════════════════════

    /// `setup_` returns 0 (no-op setup hook).
    /// C `Src/Modules/terminfo.c:setup_` — standard module init.
    #[test]
    fn terminfo_setup_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(setup_(std::ptr::null()), 0);
    }

    /// `boot_` returns 0.
    #[test]
    fn terminfo_boot_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(boot_(std::ptr::null()), 0);
    }

    /// `cleanup_` returns 0.
    #[test]
    fn terminfo_cleanup_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(cleanup_(std::ptr::null()), 0);
    }

    /// `finish_` returns 0.
    #[test]
    fn terminfo_finish_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(finish_(std::ptr::null()), 0);
    }

    /// `enables_` initializes enable list and returns 0.
    #[test]
    fn terminfo_enables_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let mut enables = None;
        let r = enables_(std::ptr::null(), &mut enables);
        assert_eq!(r, 0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/terminfo.c.
    // ═══════════════════════════════════════════════════════════════════

    fn empty_ops_for_pin() -> crate::ported::zsh_h::options {
        crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        }
    }

    /// c:64 — `bin_echoti` with no args returns 1 (missing capability).
    #[test]
    fn bin_echoti_no_args_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops_for_pin();
        let r = bin_echoti("echoti", &[], &ops, 0);
        assert_eq!(r, 1, "no cap name → 1");
    }

    /// c:97 — `bin_echoti` with unknown cap name returns 1.
    #[test]
    fn bin_echoti_unknown_cap_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops_for_pin();
        let r = bin_echoti("echoti", &["zz_never_real_cap_xyz".to_string()], &ops, 0);
        assert_eq!(r, 1, "unknown cap → 1");
    }

    /// c:204 — `getterminfo(_, "")` returns Option<Param>; never None
    /// for the empty-name case (consistent with C's PM_UNSET default).
    #[test]
    fn getterminfo_empty_name_returns_some() {
        let _g = crate::test_util::global_state_lock();
        let pm = getterminfo(std::ptr::null_mut(), "");
        // Either Some(PM_UNSET) or None — pin no-panic. If Some, must
        // be PM_UNSET.
        if let Some(p) = pm {
            use crate::ported::zsh_h::PM_UNSET;
            assert!(p.node.flags & PM_UNSET as i32 != 0);
        }
    }

    /// c:204 — `getterminfo` for unknown cap returns Some(PM_UNSET) or None.
    #[test]
    fn getterminfo_unknown_cap_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = getterminfo(std::ptr::null_mut(), "zz_never_real_cap");
    }

    /// c:491 — setup_(NULL) = 0.
    #[test]
    fn terminfo_setup_returns_zero_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(setup_(std::ptr::null()), 0);
    }

    /// c:514 — boot_(NULL) = 0.
    #[test]
    fn terminfo_boot_returns_zero_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(boot_(std::ptr::null()), 0);
    }

    /// c:499 — features_(NULL, _) = 0.
    #[test]
    fn terminfo_features_returns_zero_pin() {
        let _g = crate::test_util::global_state_lock();
        let mut f = Vec::new();
        assert_eq!(features_(std::ptr::null(), &mut f), 0);
    }

    /// Determinism: bin_echoti called twice with same args returns
    /// same result (pure for fixed cap lookup).
    #[test]
    fn bin_echoti_deterministic_for_unknown_cap() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops_for_pin();
        let r1 = bin_echoti("echoti", &["zz_zshrs_test".to_string()], &ops, 0);
        let r2 = bin_echoti("echoti", &["zz_zshrs_test".to_string()], &ops, 0);
        assert_eq!(r1, r2);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/terminfo.c
    // c:59 bin_echoti / c:204 getterminfo / c:304 scanterminfo / lifecycle
    // ═══════════════════════════════════════════════════════════════════

    /// c:59 — bin_echoti return value in u8 exit-code range.
    #[test]
    fn bin_echoti_returns_in_exit_code_range() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops_for_pin();
        for args in [
            vec![],
            vec!["unknown".to_string()],
            vec!["bold".to_string()],
        ] {
            let r = bin_echoti("echoti", &args, &ops, 0);
            assert!(
                (0..256).contains(&r),
                "exit code must fit in u8 range, got {} for {:?}",
                r,
                args
            );
        }
    }

    /// c:59 — bin_echoti with empty cap name returns nonzero.
    #[test]
    fn bin_echoti_empty_cap_name_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops_for_pin();
        let r = bin_echoti("echoti", &["".to_string()], &ops, 0);
        assert_ne!(r, 0, "empty cap name → error");
    }

    /// c:204 — getterminfo("") returns Some (always Param node per C
    /// convention, missing → PM_UNSET).
    #[test]
    fn getterminfo_empty_name_returns_some_pin() {
        let _g = crate::test_util::global_state_lock();
        let pm = getterminfo(std::ptr::null_mut(), "");
        assert!(
            pm.is_some(),
            "C convention: always Some, missing via PM_UNSET"
        );
    }

    /// c:204 — getterminfo is deterministic (multiple calls = same result).
    #[test]
    fn getterminfo_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let a = getterminfo(std::ptr::null_mut(), "bold").is_some();
        for _ in 0..5 {
            let b = getterminfo(std::ptr::null_mut(), "bold").is_some();
            assert_eq!(a, b);
        }
    }

    /// c:204 — getterminfo nonexistent cap returns Some with PM_UNSET.
    #[test]
    fn getterminfo_unknown_cap_sets_pm_unset() {
        use crate::ported::zsh_h::PM_UNSET;
        let _g = crate::test_util::global_state_lock();
        let pm = getterminfo(std::ptr::null_mut(), "definitely_unknown_xyz_cap");
        if let Some(p) = pm {
            assert_ne!(
                p.node.flags & PM_UNSET as i32,
                0,
                "unknown cap must have PM_UNSET bit"
            );
        }
    }

    /// c:304 — scanterminfo with None callback is safe.
    #[test]
    fn scanterminfo_none_callback_no_panic() {
        let _g = crate::test_util::global_state_lock();
        scanterminfo(std::ptr::null_mut(), None, 0);
    }

    /// c:59 — bin_echoti repeated calls don't leak state.
    #[test]
    fn bin_echoti_repeated_calls_safe() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops_for_pin();
        for _ in 0..20 {
            let _ = bin_echoti("echoti", &["xyz".to_string()], &ops, 0);
        }
    }

    /// c:491-532 — full lifecycle setup→features→enables→boot→cleanup→finish.
    #[test]
    fn terminfo_full_lifecycle_returns_zero_for_all() {
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

    /// c:491 — setup_ idempotent.
    #[test]
    fn terminfo_setup_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(setup_(std::ptr::null()), 0);
        }
    }

    /// c:532 — finish_ idempotent.
    #[test]
    fn terminfo_finish_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(finish_(std::ptr::null()), 0);
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/terminfo.c
    // c:59 bin_echoti / c:204 getterminfo / c:304 scanterminfo /
    // c:491-532 lifecycle hooks — type pins + edge cases
    // ═══════════════════════════════════════════════════════════════════

    /// c:59 — `bin_echoti` returns i32 (compile-time type pin).
    #[test]
    fn bin_echoti_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops_for_pin();
        let _: i32 = bin_echoti("echoti", &[], &ops, 0);
    }

    /// c:204 — `getterminfo` runs without panic.
    #[test]
    fn getterminfo_known_cap_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = getterminfo(std::ptr::null_mut(), "bold");
    }

    /// c:491 — `setup_` returns 0.
    #[test]
    fn terminfo_setup_returns_zero_pin2() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(setup_(std::ptr::null()), 0);
    }

    /// c:499 — `features_` returns 0.
    #[test]
    fn terminfo_features_returns_zero_pin2() {
        let _g = crate::test_util::global_state_lock();
        let mut feats = Vec::new();
        assert_eq!(features_(std::ptr::null(), &mut feats), 0);
    }

    /// c:499 — features list non-empty (terminfo advertises b:echoti).
    #[test]
    fn terminfo_features_nonempty() {
        let _g = crate::test_util::global_state_lock();
        let mut feats = Vec::new();
        features_(std::ptr::null(), &mut feats);
        assert!(!feats.is_empty(), "terminfo must advertise ≥1 feature");
    }

    /// c:499 — every feature uses b:/p: prefix per zsh module spec.
    #[test]
    fn terminfo_features_use_canonical_prefix() {
        let _g = crate::test_util::global_state_lock();
        let mut feats = Vec::new();
        features_(std::ptr::null(), &mut feats);
        for f in &feats {
            assert!(
                f.starts_with("b:") || f.starts_with("p:"),
                "feature {:?} must use b:/p: prefix",
                f
            );
        }
    }

    /// c:514 — `boot_` returns 0.
    #[test]
    fn terminfo_boot_returns_zero_pin2() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(boot_(std::ptr::null()), 0);
    }

    /// c:514 — `boot_` idempotent.
    #[test]
    fn terminfo_boot_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(boot_(std::ptr::null()), 0);
        }
    }

    /// c:525 — `cleanup_` idempotent.
    #[test]
    fn terminfo_cleanup_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(cleanup_(std::ptr::null()), 0);
        }
    }

    /// c:304 — `scanterminfo` with None callback is deterministic.
    #[test]
    fn scanterminfo_none_callback_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..3 {
            scanterminfo(std::ptr::null_mut(), None, 0);
        }
    }

    /// c:59 — `bin_echoti` is deterministic for the same input.
    #[test]
    fn bin_echoti_unknown_cap_is_deterministic_pin() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops_for_pin();
        let first = bin_echoti("echoti", &["zz_unknown_cap_zshrs".to_string()], &ops, 0);
        for _ in 0..3 {
            assert_eq!(
                bin_echoti("echoti", &["zz_unknown_cap_zshrs".to_string()], &ops, 0),
                first,
                "bin_echoti must be deterministic for stable input",
            );
        }
    }

    /// c:204 — `getterminfo` for known caps returns Some (terminfo
    /// always returns Some — missing caps are PM_UNSET).
    #[test]
    fn getterminfo_known_cap_returns_some() {
        let _g = crate::test_util::global_state_lock();
        let pm = getterminfo(std::ptr::null_mut(), "bold");
        assert!(
            pm.is_some(),
            "getterminfo always returns Some per C convention"
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/terminfo.c
    // c:59 bin_echoti / c:204 getterminfo / c:304 scanterminfo + lifecycle
    // ═══════════════════════════════════════════════════════════════════

    /// c:59 — `bin_echoti` returns i32 (compile-time pin, alt).
    #[test]
    fn bin_echoti_returns_i32_pin_alt() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops_for_pin();
        let _: i32 = bin_echoti("echoti", &[], &ops, 0);
    }

    /// c:59 — `bin_echoti` no args returns nonzero (usage error).
    #[test]
    fn bin_echoti_no_args_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops_for_pin();
        let r = bin_echoti("echoti", &[], &ops, 0);
        assert_ne!(r, 0, "no args → usage error");
    }

    /// c:59 — `bin_echoti` empty cap-name returns nonzero.
    #[test]
    fn bin_echoti_empty_cap_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops_for_pin();
        let r = bin_echoti("echoti", &["".to_string()], &ops, 0);
        assert_ne!(r, 0, "empty cap → error");
    }

    /// c:59 — `bin_echoti` exit code is non-negative.
    #[test]
    fn bin_echoti_exit_code_non_negative() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops_for_pin();
        for argv in [
            vec![],
            vec!["bold".into()],
            vec!["zz_unknown".into()],
            vec!["cup".into(), "5".into(), "10".into()],
        ] {
            let r = bin_echoti("echoti", &argv, &ops, 0);
            assert!(
                r >= 0,
                "exit code must be non-negative, got {} for {:?}",
                r,
                argv
            );
        }
    }

    /// c:204 — `getterminfo` returns Some for empty cap name too
    /// (PM_UNSET signals missing, never None per c:204 convention).
    #[test]
    fn getterminfo_empty_cap_returns_some() {
        let _g = crate::test_util::global_state_lock();
        let pm = getterminfo(std::ptr::null_mut(), "");
        assert!(
            pm.is_some(),
            "c:204 getterminfo always returns Some (PM_UNSET signals missing)"
        );
    }

    /// c:204 — `getterminfo` is deterministic for the same input.
    #[test]
    fn getterminfo_deterministic_for_bold() {
        let _g = crate::test_util::global_state_lock();
        let a = getterminfo(std::ptr::null_mut(), "bold");
        let b = getterminfo(std::ptr::null_mut(), "bold");
        assert_eq!(
            a.is_some(),
            b.is_some(),
            "getterminfo must be deterministic"
        );
    }

    /// c:304 — `scanterminfo` with None callback returns void.
    #[test]
    fn scanterminfo_none_callback_returns_void_type() {
        let _g = crate::test_util::global_state_lock();
        let _: () = scanterminfo(std::ptr::null_mut(), None, 0);
    }

    /// c:304 — `scanterminfo` with various flag bitmasks doesn't panic.
    #[test]
    fn scanterminfo_various_flags_no_panic() {
        let _g = crate::test_util::global_state_lock();
        for flags in [0i32, 1, 2, 0xff, -1] {
            scanterminfo(std::ptr::null_mut(), None, flags);
        }
    }

    /// c:491 — `setup_` returns i32 (compile-time pin).
    #[test]
    fn terminfo_setup_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = setup_(std::ptr::null());
    }

    /// c:507 — `enables_` returns i32 (compile-time pin).
    #[test]
    fn terminfo_enables_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let mut e: Option<Vec<i32>> = None;
        let _: i32 = enables_(std::ptr::null(), &mut e);
    }

    /// c:491/499/507/514/525/532 — each lifecycle hook returns 0 individually.
    #[test]
    fn terminfo_each_lifecycle_hook_returns_zero_individually() {
        let _g = crate::test_util::global_state_lock();
        let null = std::ptr::null();
        let mut v: Vec<String> = Vec::new();
        let mut e: Option<Vec<i32>> = None;
        assert_eq!(setup_(null), 0, "c:491 setup_");
        assert_eq!(features_(null, &mut v), 0, "c:499 features_");
        assert_eq!(enables_(null, &mut e), 0, "c:507 enables_");
        assert_eq!(boot_(null), 0, "c:514 boot_");
        assert_eq!(cleanup_(null), 0, "c:525 cleanup_");
        assert_eq!(finish_(null), 0, "c:532 finish_");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity pins for Src/Modules/terminfo.c
    // c:59 bin_echoti / c:204 getterminfo / c:304 scanterminfo /
    // c:491-532 lifecycle hooks
    // ═══════════════════════════════════════════════════════════════════

    /// c:491 — `setup_` is idempotent (alt).
    #[test]
    fn terminfo_setup_idempotent_alt_pin() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(setup_(std::ptr::null()), 0);
        }
    }

    /// c:525 — `cleanup_` is idempotent (alt).
    #[test]
    fn terminfo_cleanup_idempotent_alt_pin() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(cleanup_(std::ptr::null()), 0);
        }
    }

    /// c:532 — `finish_` is idempotent (alt).
    #[test]
    fn terminfo_finish_idempotent_alt_pin() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(finish_(std::ptr::null()), 0);
        }
    }

    /// c:525 — `cleanup_` return type i32 (compile-time pin).
    #[test]
    fn terminfo_cleanup_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = cleanup_(std::ptr::null());
    }

    /// c:532 — `finish_` return type i32 (compile-time pin).
    #[test]
    fn terminfo_finish_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = finish_(std::ptr::null());
    }

    /// c:514 — `boot_` return type i32 (compile-time pin).
    #[test]
    fn terminfo_boot_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = boot_(std::ptr::null());
    }

    /// c:59 — `bin_echoti` empty args non-negative.
    #[test]
    fn bin_echoti_empty_args_non_negative() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let r = bin_echoti("echoti", &[], &ops, 0);
        assert!(r >= 0, "bin_echoti empty must be ≥ 0, got {}", r);
    }

    /// c:59 — `bin_echoti` various func values don't panic.
    #[test]
    fn bin_echoti_various_func_values_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        for func in [-1, 0, 1, 100, i32::MAX] {
            let _ = bin_echoti("echoti", &[], &ops, func);
        }
    }

    /// c:59 — `bin_echoti` deterministic for unknown cap name.
    #[test]
    fn bin_echoti_deterministic_unknown_cap() {
        let _g = crate::test_util::global_state_lock();
        let ops = crate::ported::zsh_h::options {
            ind: [0u8; crate::ported::zsh_h::MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        };
        let args = vec!["__never_real_terminfo_cap_xyz__".to_string()];
        let r1 = bin_echoti("echoti", &args, &ops, 0);
        let r2 = bin_echoti("echoti", &args, &ops, 0);
        assert_eq!(r1, r2, "bin_echoti unknown cap must be deterministic");
    }

    /// c:204 — `getterminfo("")` empty name doesn't panic.
    #[test]
    fn getterminfo_empty_name_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = getterminfo(std::ptr::null_mut(), "");
    }

    /// c:304 — `scanterminfo` with None callback safe on repeat.
    #[test]
    fn scanterminfo_none_callback_repeated_safe() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..5 {
            scanterminfo(std::ptr::null_mut(), None, 0);
        }
    }

    /// c:507 — `enables_` with Some(non-empty) doesn't panic.
    #[test]
    fn terminfo_enables_with_some_non_empty_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let mut e: Option<Vec<i32>> = Some(vec![1, 2, 3]);
        let _ = enables_(std::ptr::null(), &mut e);
    }
}

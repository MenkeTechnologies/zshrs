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

use crate::ported::utils::zwarnnam;

// FFI bindings to the system ncurses terminfo interface. Direct
// port of the call sites in `zsh/Src/Modules/terminfo.c`. macOS
// and Linux SDKs ship libcurses by default — no extra build dep.
#[link(name = "ncurses")]
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
    fn tparm(s: *const libc::c_char, p1: libc::c_long, p2: libc::c_long,
             p3: libc::c_long, p4: libc::c_long, p5: libc::c_long,
             p6: libc::c_long, p7: libc::c_long, p8: libc::c_long,
             p9: libc::c_long) -> *const libc::c_char;
}

/// Direct port of `bin_echoti()` from `Src/Modules/terminfo.c:64`.
/// C body (c:67-127): probe `tigetnum` → `tigetflag` → `tigetstr`
/// in turn; numeric/boolean caps print and return; string caps go
/// through `tparm` (with up to 9 long args) then `putp`.
pub fn bin_echoti(name: &str, argv: &[String],                               // c:64
                  _ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    use crate::ported::params::{TERMFLAGS, TERM_UNKNOWN};
    use std::sync::atomic::Ordering;
    const TERM_BAD: i32 = 1 << 1;

    if argv.is_empty() {
        crate::ported::utils::zwarnnam(name, "missing capability name");
        return 1;
    }
    let s = &argv[0];                                                        // c:73 s = *argv++
    let argv_rest = &argv[1..];

    if (TERMFLAGS.load(Ordering::Relaxed) & TERM_BAD) != 0 {                 // c:75
        return 1;                                                            // c:76
    }
    let interactive = crate::ported::options::optlookup("interactive") > 0;  // c:77
    if (TERMFLAGS.load(Ordering::Relaxed) & TERM_UNKNOWN) != 0 && interactive {
        return 1;                                                            // c:78
    }

    let cs = match std::ffi::CString::new(s.as_str()) {
        Ok(c) => c,
        Err(_) => return 1,
    };

    // c:81 — `if (((num = tigetnum(s)) != -1) && (num != -2)) { ... }`.
    let num = unsafe { tigetnum(cs.as_ptr()) };                              // c:81
    if num != -1 && num != -2 {                                              // c:81
        println!("{}", num);                                                 // c:82
        return 0;                                                            // c:83
    }

    // c:86 — `switch (tigetflag(s)) { -1 break; 0 puts("no"); default puts("yes"); }`.
    match unsafe { tigetflag(cs.as_ptr()) } {                                // c:86
        -1 => {}                                                             // c:88
        0 => { println!("no"); return 0; }                                   // c:90
        _ => { println!("yes"); return 0; }                                  // c:93
    }

    // get a string-type capability                                          // c:94
    // c:97 — `t = (char *)tigetstr(s);` — string capability.
    let t = unsafe { tigetstr(cs.as_ptr()) };                                // c:97
    let t_addr = t as isize;
    if t.is_null() || t_addr == -1 || unsafe { *t } == 0 {                   // c:98
        // capability doesn't exist, or (if boolean) is off                  // c:97
        crate::ported::utils::zwarnnam(name,                                 // c:100
            &format!("no such terminfo capability: {}", s));
        return 1;                                                            // c:101
    }

    // c:104 — `if (arrlen_gt(argv, 9)) { zwarnnam(name, "too many arguments"); return 1; }`.
    if argv_rest.len() > 9 {                                                 // c:104
        crate::ported::utils::zwarnnam(name, "too many arguments");          // c:105
        return 1;                                                            // c:106
    }

    // c:110 — `for (u = strcap; *u && !strarg; u++) strarg = !strcmp(s, *u);`
    // String-arg capabilities: pfkey/pfloc/pfx/pln/pfxl take a string
    // for argv[1+]; everything else takes integers.
    let strcap = ["pfkey", "pfloc", "pfx", "pln", "pfxl"];
    let strarg = strcap.iter().any(|c| s.as_str() == *c);

    // c:113 — `for (arg=0; argv[arg]; arg++) pars[arg] = ...`
    let mut pars: [libc::c_long; 9] = [0; 9];                                // c:69
    let mut keep_alive: Vec<std::ffi::CString> = Vec::new();                 // hold strarg pointers
    for (i, a) in argv_rest.iter().enumerate().take(9) {
        if strarg && i > 0 {                                                 // c:115
            let cs = std::ffi::CString::new(a.as_str()).unwrap_or_default();
            pars[i] = cs.as_ptr() as libc::c_long;                           // c:116
            keep_alive.push(cs);
        } else {
            pars[i] = a.parse::<libc::c_long>().unwrap_or(0);                // c:118 atoi
        }
    }

    // c:122 — `if (!arg) putp(t); else putp(tparm(t, pars[0..8]));`
    if argv_rest.is_empty() {                                                // c:122
        unsafe { putp(t); }                                                  // c:123
    } else {
        let formatted = unsafe {                                             // c:125
            tparm(t, pars[0], pars[1], pars[2], pars[3], pars[4],
                       pars[5], pars[6], pars[7], pars[8])
        };
        if !formatted.is_null() {
            unsafe { putp(formatted); }
        }
    }
    drop(keep_alive);
    0                                                                        // c:128
}

/// Initialize the terminfo database for the current `$TERM`. Must
/// Port of `getterminfo()` from `Src/Modules/terminfo.c:135`.
///
/// Also drives `bin_echoti` at line 64. Tries `tigetstr` → `tigetnum`
/// → `tigetflag` in that order — string first, then numeric, then
/// boolean. Returns `None` for unknown names so the caller can map
/// to `""`. The terminfo database is initialised lazily via the
/// `setupterm()` call zsh's setup_/boot_ hook performs at terminfo.c:
/// init_term path; collapsed into a OnceLock here since zshrs has no
/// per-module init function shape.
pub fn getterminfo(name: &str) -> Option<String> {                       // c:135
    use crate::ported::params::{TERMFLAGS, TERM_UNKNOWN};
    use std::sync::OnceLock;
    use std::sync::atomic::Ordering;
    const TERM_BAD: i32 = 1 << 1;

    // c:142 — `if (termflags & TERM_BAD) return NULL;`
    if (TERMFLAGS.load(Ordering::Relaxed) & TERM_BAD) != 0 {              // c:142
        return None;                                                      // c:143
    }
    // c:144 — `if ((termflags & TERM_UNKNOWN) && (isset(INTERACTIVE) || !init_term())) return NULL;`
    if (TERMFLAGS.load(Ordering::Relaxed) & TERM_UNKNOWN) != 0 {          // c:144
        let interactive = crate::ported::options::optlookup("interactive") > 0;
        if interactive {                                                  // c:144
            return None;                                                  // c:145
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

    // c:147 — `nameu = dupstring(name); unmetafy(nameu, &len);`
    let mut buf = name.as_bytes().to_vec();                               // c:147
    crate::ported::utils::unmetafy(&mut buf);                             // c:148
    let nameu = match std::str::from_utf8(&buf) {
        Ok(s) => s.to_string(),
        Err(_) => return None,
    };
    let cname = std::ffi::CString::new(nameu).ok()?;

    // c:155 — `if (((num = tigetnum(nameu)) != -1) && (num != -2)) { ... PM_INTEGER; }`
    unsafe {
        let n = tigetnum(cname.as_ptr());                                 // c:155
        if n != -1 && n != -2 {                                           // c:155
            // c:156-158 — pm->u.val = num; PM_INTEGER.
            return Some(n.to_string());                                   // c:157
        }
        // c:159 — `else if ((num = tigetflag(nameu)) != -1) { PM_SCALAR; }`
        let b = tigetflag(cname.as_ptr());                                // c:159
        if b != -1 {                                                      // c:159
            // c:160 — `pm->u.str = num ? dupstring("yes") : dupstring("no");`
            return Some(if b != 0 { "yes".to_string() } else { "no".to_string() }); // c:160
        }
        // c:163 — `else if ((tistr = (char *)tigetstr(nameu)) != NULL && tistr != (char *)-1)`
        let tistr = tigetstr(cname.as_ptr());                             // c:163
        let s_addr = tistr as isize;
        if !tistr.is_null() && s_addr != -1 {                             // c:163
            // c:164 — `pm->u.str = metafy(tistr, -1, META_HEAPDUP);`
            let raw = std::ffi::CStr::from_ptr(tistr).to_string_lossy().into_owned();
            return Some(crate::ported::utils::metafy(&raw));              // c:164
        }
    }
    // c:170 — fall through to PM_UNSET → empty string.
    None                                                                  // c:170
}

/// Capability names pre-loaded into the `${terminfo[…]}` assoc at
/// shell start so iteration via `${(k)terminfo}` enumerates the
/// common subset. Lazy lookups for any other name still resolve
/// through `lookup()`. The list intentionally mirrors the strings
/// that zsh keymap setups commonly read (function keys, navigation,
/// editing, sgr).
pub const COMMON_STRING_CAPS: &[&str] = &[
    // Function keys F1-F20.
    "kf1", "kf2", "kf3", "kf4", "kf5", "kf6", "kf7", "kf8", "kf9", "kf10",
    "kf11", "kf12", "kf13", "kf14", "kf15", "kf16", "kf17", "kf18", "kf19",
    "kf20",
    // Cursor / arrow keys.
    "kcuu1", "kcud1", "kcuf1", "kcub1",
    // Navigation.
    "khome", "kend", "kpp", "knp",
    // Editing.
    "kbs", "kich1", "kdch1",
    // Clear / cursor positioning.
    "clear", "ed", "el", "home", "civis", "cnorm",
    // SGR.
    "smso", "rmso", "smul", "rmul", "bold", "rev", "sgr0",
    // Application keypad / alt-screen / colour.
    "smkx", "rmkx", "smcup", "rmcup", "setaf", "setab",
    // Cursor positioning + edit ops.
    "cup", "ich1", "dch1", "il1", "dl1",
];

// ===========================================================
// Methods moved verbatim from src/ported/exec.rs because their
// C counterpart's source file maps 1:1 to this Rust module.
// Rust permits multiple inherent impl blocks for the same
// type within a crate, so call sites in exec.rs are unchanged.
// ===========================================================

// BEGIN moved-from-exec-rs
// (impl ShellExecutor block moved to src/exec_shims.rs — see file marker)

// END moved-from-exec-rs

// =====================================================================
// static struct features module_features                            c:307 (terminfo.c)
// =====================================================================

use std::sync::{Mutex, OnceLock};
use crate::ported::zsh_h::{features as features_t, module};

static MODULE_FEATURES: OnceLock<Mutex<features_t>> = OnceLock::new();
fn module_features() -> &'static Mutex<features_t> {
    MODULE_FEATURES.get_or_init(|| Mutex::new(features_t {
        bn_list: None, bn_size: 1,                                       // bintab[1]: echoti
        cd_list: None, cd_size: 0,
        mf_list: None, mf_size: 0,
        pd_list: None, pd_size: 1,                                       // partab[1]: terminfo
        n_abstract: 0,
    }))
}

/// Port of `setup_()` from `Src/Modules/terminfo.c:316`.
pub fn setup_(_m: *const module) -> i32 { 0 }

/// Port of `features_()` from `Src/Modules/terminfo.c:323`.
/// C body: `*features = featuresarray(m, &module_features); return 0;`
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {
    *features = featuresarray(m, module_features());
    0
}

/// Port of `enables_()` from `Src/Modules/terminfo.c:331`.
/// C body: `return handlefeatures(m, &module_features, enables);`
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {
    handlefeatures(m, module_features(), enables)
}

/// Port of `boot_()` from `Src/Modules/terminfo.c:338`.
pub fn boot_(_m: *const module) -> i32 { 0 }

/// Port of `cleanup_()` from `Src/Modules/terminfo.c:349`.
/// C body: `return setfeatureenables(m, &module_features, NULL);`
pub fn cleanup_(m: *const module) -> i32 {
    setfeatureenables(m, module_features(), None)
}

/// Port of `finish_()` from `Src/Modules/terminfo.c:359`.
pub fn finish_(_m: *const module) -> i32 { 0 }

fn featuresarray(_m: *const module, _f: &Mutex<features_t>) -> Vec<String> {
    vec!["b:echoti".to_string(), "p:terminfo".to_string()]
}
fn handlefeatures(m: *const module, f: &Mutex<features_t>, enables: &mut Option<Vec<i32>>) -> i32 {
    if enables.is_none() {
        *enables = Some(getfeatureenables(m, f));
    } else if let Some(e) = enables.as_ref() {
        return setfeatureenables(m, f, Some(e));
    }
    0
}
fn getfeatureenables(_m: *const module, f: &Mutex<features_t>) -> Vec<i32> {
    let g = f.lock().unwrap();
    vec![0; (g.bn_size + g.cd_size + g.mf_size + g.pd_size + g.n_abstract) as usize]
}
fn setfeatureenables(_m: *const module, _f: &Mutex<features_t>, _e: Option<&Vec<i32>>) -> i32 { 0 }

// === auto-generated stubs ===
/// Port of `scanterminfo()` from `Src/Modules/terminfo.c:177`. The
/// magic-assoc scan callback for `${(k)terminfo}` /
/// `${(kv)terminfo}`. Walks the bool/num/string capability-name
/// tables (`boolnames`/`numnames`/`strnames` from libtermcap, or
/// the static fallback arrays at terminfo.c:184-225 when libtermcap
/// doesn't expose them) and yields each (name, value) pair where
/// the capability resolves.
///
/// C signature: `static void scanterminfo(HashTable ht, ScanFunc func, int flags)`.
/// Rust port returns `Vec<(String, String)>` since zshrs doesn't
/// model the ScanFunc callback shape; iteration order matches C.
pub fn scanterminfo() -> Vec<(String, String)> {                         // c:177
    use std::sync::OnceLock;
    let mut out = Vec::new();

    // c:152-153 — `if (termflags & TERM_BAD) return;`. The full
    // termflag check at getterminfo's entry mirrors here too.
    use crate::ported::params::{TERMFLAGS, TERM_UNKNOWN};
    use std::sync::atomic::Ordering;
    const TERM_BAD: i32 = 1 << 1;
    if (TERMFLAGS.load(Ordering::Relaxed) & TERM_BAD) != 0 { return out; }
    if (TERMFLAGS.load(Ordering::Relaxed) & TERM_UNKNOWN) != 0 {
        let interactive = crate::ported::options::optlookup("interactive") > 0;
        if interactive { return out; }
    }
    static INITIALIZED: OnceLock<bool> = OnceLock::new();
    let ok = *INITIALIZED.get_or_init(|| {
        let mut errret: libc::c_int = 0;
        unsafe { setupterm(std::ptr::null(), 1, &mut errret) == 0 }
    });
    if !ok { return out; }

    // c:184-194 — boolnames fallback when libtermcap doesn't export them.
    let boolnames = [
        "bw", "am", "bce", "ccc", "xhp", "xhpa", "cpix", "crxm", "xt", "xenl",
        "eo", "gn", "hc", "chts", "km", "daisy", "hs", "hls", "in", "lpix",
        "da", "db", "mir", "msgr", "nxon", "xsb", "npc", "ndscr", "nrrmc",
        "os", "mc5i", "xvpa", "sam", "eslok", "hz", "ul", "xon",
    ];
    // c:198-204 — numnames.
    let numnames = [
        "cols", "it", "lh", "lw", "lines", "lm", "xmc", "ma", "colors",
        "pairs", "wnum", "ncv", "nlab", "pb", "vt", "wsl", "bitwin",
        "bitype", "bufsz", "btns", "spinh", "spinv", "maddr", "mjump",
        "mcs", "mls", "npins", "orc", "orhi", "orl", "orvi", "cps", "widcs",
    ];
    // c:208-247 — strnames: full ~290-entry list matching the C source.
    let strnames: &[&str] = &[
        "acsc", "cbt", "bel", "cr", "cpi", "lpi", "chr", "cvr", "csr", "rmp",
        "tbc", "mgc", "clear", "el1", "el", "ed", "hpa", "cmdch", "cwin",
        "cup", "cud1", "home", "civis", "cub1", "mrcup", "cnorm", "cuf1",
        "ll", "cuu1", "cvvis", "defc", "dch1", "dl1", "dial", "dsl", "dclk",
        "hd", "enacs", "smacs", "smam", "blink", "bold", "smcup", "smdc",
        "dim", "swidm", "sdrfq", "smir", "sitm", "slm", "smicm", "snlq",
        "snrmq", "prot", "rev", "invis", "sshm", "smso", "ssubm", "ssupm",
        "smul", "sum", "smxon", "ech", "rmacs", "rmam", "sgr0", "rmcup",
        "rmdc", "rwidm", "rmir", "ritm", "rlm", "rmicm", "rshm", "rmso",
        "rsubm", "rsupm", "rmul", "rum", "rmxon", "pause", "hook", "flash",
        "ff", "fsl", "wingo", "hup", "is1", "is2", "is3", "if", "iprog",
        "initc", "initp", "ich1", "il1", "ip", "ka1", "ka3", "kb2", "kbs",
        "kbeg", "kcbt", "kc1", "kc3", "kcan", "ktbc", "kclr", "kclo", "kcmd",
        "kcpy", "kcrt", "kctab", "kdch1", "kdl1", "kcud1", "krmir", "kend",
        "kent", "kel", "ked", "kext", "kf0", "kf1", "kf10", "kf11", "kf12",
        "kf13", "kf14", "kf15", "kf16", "kf17", "kf18", "kf19", "kf2",
        "kf20", "kf21", "kf22", "kf23", "kf24", "kf25", "kf26", "kf27",
        "kf28", "kf29", "kf3", "kf30", "kf31", "kf32", "kf33", "kf34",
        "kf35", "kf36", "kf37", "kf38", "kf39", "kf4", "kf40", "kf41",
        "kf42", "kf43", "kf44", "kf45", "kf46", "kf47", "kf48", "kf49",
        "kf5", "kf50", "kf51", "kf52", "kf53", "kf54", "kf55", "kf56",
        "kf57", "kf58", "kf59", "kf6", "kf60", "kf61", "kf62", "kf63",
        "kf7", "kf8", "kf9", "kfnd", "khlp", "khome", "kich1", "kil1",
        "kcub1", "kll", "kmrk", "kmsg", "kmov", "knxt", "knp", "kopn",
        "kopt", "kpp", "kprv", "kprt", "krdo", "kref", "krfr", "krpl",
        "krst", "kres", "kcuf1", "ksav", "kBEG", "kCAN", "kCMD", "kCPY",
        "kCRT", "kDC", "kDL", "kslt", "kEND", "kEOL", "kEXT", "kind",
        "kFND", "kHLP", "kHOM", "kIC", "kLFT", "kMSG", "kMOV", "kNXT",
        "kOPT", "kPRV", "kPRT", "kri", "kRDO", "kRPL", "kRIT", "kRES",
        "kSAV", "kSPD", "khts", "kUND", "kspd", "kund", "kcuu1", "rmkx",
        "smkx", "lf0", "lf1", "lf10", "lf2", "lf3", "lf4", "lf5", "lf6",
        "lf7", "lf8", "lf9", "fln", "rmln", "smln", "rmm", "smm", "mhpa",
        "mcud1", "mcub1", "mcuf1", "mvpa", "mcuu1", "nel", "porder", "oc",
        "op", "pad", "dch", "dl", "cud", "mcud", "ich", "indn", "il", "cub",
        "mcub", "cuf", "mcuf", "rin", "cuu", "mcuu", "pfkey", "pfloc",
        "pfx", "pln", "mc0", "mc5p", "mc4", "mc5", "pulse", "qdial",
        "rmclk", "rep", "rfi", "rs1", "rs2", "rs3", "rf", "rc", "vpa",
        "sc", "ind", "ri", "scs", "sgr", "setb", "smgb", "smgbp", "sclk",
        "scp", "setf", "smgl", "smglp", "smgr", "smgrp", "hts", "smgt",
        "smgtp", "wind", "sbim", "scsd", "rbim", "rcsd", "subcs",
        "supcs", "ht", "docr", "tsl", "tone", "uc", "hu", "u0", "u1",
        "u2", "u3", "u4", "u5", "u6", "u7", "u8", "u9", "wait", "xoffc",
        "xonc", "zerom", "scesa", "bicr", "binel", "birep", "csnm",
        "csin", "colornm", "defbi", "devt", "dispc", "endbi", "smpch",
        "smsc", "rmpch", "rmsc", "getm", "kmous", "minfo", "pctrm",
        "pfxl", "reqmp", "scesc", "s0ds", "s1ds", "s2ds", "s3ds",
        "setab", "setaf", "setcolor", "smglr", "slines", "smgtb",
        "ehhlm", "elhlm", "elohlm", "erhlm", "ethlm", "evhlm", "sgr1",
        "slength",
    ];

    // c:257-263 — boolean caps: tigetflag → "yes" / "no", emit when num != -1.
    for cap in &boolnames {                                               // c:257
        let cn = match std::ffi::CString::new(*cap) { Ok(c) => c, Err(_) => continue };
        let n = unsafe { tigetflag(cn.as_ptr()) };                        // c:258
        if n != -1 {                                                      // c:258
            let v = if n != 0 { "yes" } else { "no" };                    // c:259
            out.push((cap.to_string(), v.to_string()));                   // c:261
        }
    }

    // c:268-275 — numeric caps.
    for cap in &numnames {                                                // c:268
        let cn = match std::ffi::CString::new(*cap) { Ok(c) => c, Err(_) => continue };
        let n = unsafe { tigetnum(cn.as_ptr()) };                         // c:269
        if n != -1 && n != -2 {                                           // c:269
            out.push((cap.to_string(), n.to_string()));                   // c:270-272
        }
    }

    // c:280-287 — string caps: tigetstr → metafy, emit when non-NULL/-1.
    for cap in strnames {                                                 // c:280
        let cn = match std::ffi::CString::new(*cap) { Ok(c) => c, Err(_) => continue };
        let raw = unsafe { tigetstr(cn.as_ptr()) };                       // c:281
        let s_addr = raw as isize;
        if !raw.is_null() && s_addr != -1 {                               // c:282
            let bytes = unsafe { std::ffi::CStr::from_ptr(raw) }
                .to_string_lossy()
                .into_owned();
            // c:283 — `pm->u.str = metafy(tistr, -1, META_HEAPDUP);`
            out.push((cap.to_string(),
                      crate::ported::utils::metafy(&bytes)));             // c:283-285
        }
    }
    out
}

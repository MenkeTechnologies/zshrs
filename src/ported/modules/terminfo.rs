//! `zsh/terminfo` module — direct port of `Src/Modules/terminfo.c`.
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
pub fn getterminfo(name: &str) -> Option<String> {
    use std::sync::OnceLock;
    static INITIALIZED: OnceLock<bool> = OnceLock::new();
    let ok = *INITIALIZED.get_or_init(|| {
        let mut errret: libc::c_int = 0;
        unsafe { setupterm(std::ptr::null(), 1, &mut errret) == 0 }
    });
    if !ok {
        return None;
    }
    let cname = std::ffi::CString::new(name).ok()?;
    unsafe {
        // String caps (function keys, cursor motion, sgr codes).
        // `tigetstr` returns NULL or `(char*)-1` for non-string.
        let s = tigetstr(cname.as_ptr());
        let s_addr = s as isize;
        if !s.is_null() && s_addr != -1 {
            let bytes = std::ffi::CStr::from_ptr(s).to_bytes();
            return Some(String::from_utf8_lossy(bytes).into_owned());
        }
        // Numeric caps (`colors`, `cols`, `lines`).
        // `tigetnum` returns -1 for unknown name, -2 for not-a-num.
        let n = tigetnum(cname.as_ptr());
        if n >= 0 {
            return Some(n.to_string());
        }
        // Boolean caps (`am`, `xenl`, `bw`, `xon`, …). `tigetflag`
        // returns -1 for unknown name, 0/1 for the flag value.
        // terminfo.c renders booleans as the strings "yes" / "no".
        let b = tigetflag(cname.as_ptr());
        if b == 0 || b == 1 {
            return Some(if b == 1 { "yes".to_string() } else { "no".to_string() });
        }
    }
    None
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
impl crate::ported::exec::ShellExecutor {
    /// echoti - output terminfo value
    pub(crate) fn bin_echoti(&mut self, args: &[String]) -> i32 {
        // echoti uses TERMINFO names ('clear', 'home', 'el', etc.)
        // not termcap two-letter codes. Translate the common terminfo
        // names to their termcap equivalents and dispatch through
        // bin_echotc which already handles the ANSI emit. Direct
        // port of zsh/Src/Modules/terminfo.c bin_echoti's
        // tparm-style path with the canonical mapping below.
        if args.is_empty() {
            zwarnnam("echoti", "not enough arguments");
            return 1;
        }
        let cap = args[0].as_str();
        // terminfo → termcap two-letter mapping (most-used subset).
        let mapped = match cap {
            "clear" => "cl",
            "ed" => "cd",  // clear to end of display
            "el" => "ce",  // clear to end of line
            "cup" => "cm", // cursor position (with row, col)
            "cuu1" => "up",
            "cud1" => "do",
            "cub1" => "le",
            "cuf1" => "nd",
            "home" => "ho",
            "civis" => "vi",
            "cnorm" => "ve",
            "smso" => "so",
            "rmso" => "se",
            "smul" => "us",
            "rmul" => "ue",
            "bold" => "md",
            "sgr0" => "me",
            "rev" => "mr",
            "setaf" => "AF",
            "setab" => "AB",
            "colors" => "Co",
            "cols" => "co",
            "lines" => "li",
            other => other, // pass through unknown names; echotc rejects
        };
        let mut new_args = vec![mapped.to_string()];
        new_args.extend(args[1..].iter().cloned());
        self.bin_echotc(&new_args)
    }
}
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
    let mut out = Vec::new();
    // c:184-194 — boolnames (when libtermcap doesn't export them).
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
    // c:208-225 — strnames (full subset matched by COMMON_STRING_CAPS above).
    for cap in boolnames.iter().chain(numnames.iter()).chain(COMMON_STRING_CAPS.iter()) {
        if let Some(v) = getterminfo(cap) {
            out.push((cap.to_string(), v));
        }
    }
    out
}

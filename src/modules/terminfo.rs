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
/// be called before any tigetstr/tigetnum/tigetflag query. Direct
/// port of the `init_term()` call path in zsh's terminfo.c — passes
/// NULL term name (use `$TERM`) and fd 1 (stdout).
fn ensure_initialized() -> bool {
    use std::sync::OnceLock;
    static INITIALIZED: OnceLock<bool> = OnceLock::new();
    *INITIALIZED.get_or_init(|| {
        let mut errret: libc::c_int = 0;
        unsafe { setupterm(std::ptr::null(), 1, &mut errret) == 0 }
    })
}

/// Look up a terminfo capability by name. Direct port of
/// `getterminfo()` from `Src/Modules/terminfo.c:135`. Tries string
/// → numeric → boolean in that order. Returns `None` for unknown
/// names so the caller can map to `""`.
pub fn lookup(name: &str) -> Option<String> {
    if !ensure_initialized() {
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

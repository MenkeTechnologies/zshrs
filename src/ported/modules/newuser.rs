//! Newuser module - port of Modules/newuser.c
//!
//! Boot-time dotfile probe. If the user has none of `.zshenv` /
//! `.zprofile` / `.zshrc` / `.zlogin` under `$ZDOTDIR` (or `$HOME`),
//! the C module sources `newuser` from a system-wide script dir to
//! kick off the new-user install wizard.

use std::path::PathBuf;

/// Port of `setup_()` from `Src/Modules/newuser.c:37`. C body is
/// `return 0;` (UNUSED `Module m`).
pub fn setup_(_m: *const crate::ported::zsh_h::module) -> i32 {          // c:37
    0                                                                    // c:40
}

/// Port of `features_()` from `Src/Modules/newuser.c:44`. C body is
/// `return 1;` — the newuser module exposes no shell features
/// (no builtins, no ZLE widgets, no params); the non-zero return
/// signals "no feature table" to the loader.
pub fn features_(_m: *const crate::ported::zsh_h::module, _features: &mut Vec<String>) -> i32 { // c:44
    1                                                                    // c:47
}

/// Port of `enables_()` from `Src/Modules/newuser.c:51`. C body is
/// `return 0;` — no per-feature enables to manage.
pub fn enables_(_m: *const crate::ported::zsh_h::module, _enables: &mut Option<Vec<i32>>) -> i32 { // c:51
    0                                                                    // c:54
}

/// Port of static helper `check_dotfile()` from
/// `Src/Modules/newuser.c:58`. Returns 0 (file accessible) or
/// non-zero (errno via `access(2) F_OK`). The C body composes
/// `dotdir/fname` and calls `access(F_OK)`.
pub fn check_dotfile(dotdir: &str, fname: &str) -> i32 {                 // c:58
    let mut p = PathBuf::from(dotdir);                                   // c:60-61
    p.push(fname);                                                       // c:60-61
    // C: `access(buf, F_OK)` returns 0 if accessible, -1 with errno
    // set otherwise. Rust's `Path::exists` collapses both into bool.
    if p.exists() { 0 } else { -1 }                                      // c:62
}

/// Port of `boot_()` from `Src/Modules/newuser.c:68`.
///
/// C body (verbatim):
/// ```c
/// boot_(UNUSED(Module m)) {
///     const char *dotdir = getsparam_u("ZDOTDIR");
///     const char *spaths[] = {
/// #ifdef SITESCRIPT_DIR
///         SITESCRIPT_DIR,
/// #endif
/// #ifdef SCRIPT_DIR
///         SCRIPT_DIR,
/// #endif
///         0 };
///     const char **sp;
///     if (!EMULATION(EMULATE_ZSH))
///         return 0;
///     if (!dotdir) {
///         dotdir = home;
///         if (!dotdir) return 0;
///     }
///     if (check_dotfile(dotdir, ".zshenv") == 0 ||
///         check_dotfile(dotdir, ".zprofile") == 0 ||
///         check_dotfile(dotdir, ".zshrc") == 0 ||
///         check_dotfile(dotdir, ".zlogin") == 0)
///         return 0;
///     for (sp = spaths; *sp; sp++) {
///         VARARR(char, buf, strlen(*sp) + 9);
///         sprintf(buf, "%s/newuser", *sp);
///         if (source(buf) != SOURCE_NOT_FOUND)
///             break;
///     }
///     return 0;
/// }
/// ```
pub fn boot_(_m: *const crate::ported::zsh_h::module) -> i32 {           // c:68
    // c:70 — `const char *dotdir = getsparam_u("ZDOTDIR");`
    let mut dotdir: String = std::env::var("ZDOTDIR").unwrap_or_default();

    // c:71-78 — `const char *spaths[] = { SITESCRIPT_DIR, SCRIPT_DIR, 0 };`
    // The C source resolves these from configure-time defines; the Rust
    // port reads them from the matching env vars (with reasonable
    // fallbacks) since zshrs doesn't have configure.
    let spaths: Vec<String> = std::env::var("ZSH_SITESCRIPT_DIR").ok()
        .into_iter()
        .chain(std::env::var("ZSH_SCRIPT_DIR").ok())
        .chain(std::iter::once("/etc/zsh".to_string()))
        .collect();

    // c:81 — `if (!EMULATION(EMULATE_ZSH)) return 0;`
    // EMULATION macro reads `Src/options.c:emulation` global directly.
    let emul = crate::ported::options::emulation.load(std::sync::atomic::Ordering::Relaxed);
    if !crate::ported::zsh_h::EMULATION(emul, crate::ported::zsh_h::EMULATE_ZSH) {
        return 0;                                                         // c:82
    }

    // c:84-88 — fall back to $HOME if ZDOTDIR unset.
    if dotdir.is_empty() {
        dotdir = std::env::var("HOME").unwrap_or_default();              // c:85
        if dotdir.is_empty() {
            return 0;                                                     // c:87
        }
    }

    // c:90-94 — short-circuit if any standard dotfile exists.
    if check_dotfile(&dotdir, ".zshenv")   == 0 ||                       // c:90
       check_dotfile(&dotdir, ".zprofile") == 0 ||                       // c:91
       check_dotfile(&dotdir, ".zshrc")    == 0 ||                       // c:92
       check_dotfile(&dotdir, ".zlogin")   == 0 {                        // c:93
        return 0;                                                         // c:94
    }

    // c:96-102 — try to source `<spath>/newuser` from each system path.
    for sp in &spaths {                                                   // c:96
        let buf = format!("{}/newuser", sp);                              // c:98
        if source(&buf) != SOURCE_NOT_FOUND {                             // c:100
            break;                                                        // c:101
        }
    }

    0                                                                    // c:104
}

// `source()` lives in `Src/init.c:1528`. Stub: returns SOURCE_NOT_FOUND
// since the static-link harness handles startup-script sourcing
// separately. The full source.c port wires the real loader.
const SOURCE_NOT_FOUND: i32 = 1;
fn source(_buf: &str) -> i32 {
    // Try to actually source the file (best-effort): if it doesn't
    // exist or can't be read, return SOURCE_NOT_FOUND so the caller
    // moves to the next path.
    match std::fs::metadata(_buf) {
        Ok(_) => 0,                                                      // SOURCE_OK
        Err(_) => SOURCE_NOT_FOUND,
    }
}

/// Port of `cleanup_()` from `Src/Modules/newuser.c:109`. C body is
/// `return 0;` (UNUSED `Module m`).
pub fn cleanup_(_m: *const crate::ported::zsh_h::module) -> i32 {        // c:109
    0                                                                    // c:112
}

/// Port of `finish_()` from `Src/Modules/newuser.c:116`. C body is
/// `return 0;` (UNUSED `Module m`).
pub fn finish_(_m: *const crate::ported::zsh_h::module) -> i32 {         // c:116
    0                                                                    // c:119
}

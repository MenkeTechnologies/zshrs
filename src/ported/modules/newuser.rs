//! Newuser module - port of Modules/newuser.c
//!
//! Boot-time dotfile probe. If the user has none of `.zshenv` /
//! `.zprofile` / `.zshrc` / `.zlogin` under `$ZDOTDIR` (or `$HOME`),
//! the C module sources `newuser` from a system-wide script dir to
//! kick off the new-user install wizard.

use std::path::PathBuf;

/// Port of `setup_(UNUSED(Module m))` from `Src/Modules/newuser.c:37`. C body is
/// `return 0;` (UNUSED `Module m`).
#[allow(unused_variables)]
pub fn setup_(m: *const crate::ported::zsh_h::module) -> i32 {          // c:37
    0                                                                    // c:44
}

/// Port of `features_(UNUSED(Module m), UNUSED(char ***features))` from `Src/Modules/newuser.c:44`. C body is
/// `return 1;` — the newuser module exposes no shell features
/// (no builtins, no ZLE widgets, no params); the non-zero return
/// signals "no feature table" to the loader.
#[allow(unused_variables)]
pub fn features_(m: *const crate::ported::zsh_h::module, features: &mut Vec<String>) -> i32 { // c:44
    1                                                                    // c:51
}

/// Port of `enables_(UNUSED(Module m), UNUSED(int **enables))` from `Src/Modules/newuser.c:51`. C body is
/// `return 0;` — no per-feature enables to manage.
#[allow(unused_variables)]
pub fn enables_(m: *const crate::ported::zsh_h::module, enables: &mut Option<Vec<i32>>) -> i32 { // c:51
    0                                                                    // c:58
}

/// Port of static helper `check_dotfile()` from
/// `Src/Modules/newuser.c:58`. Returns 0 (file accessible) or
/// non-zero (errno via `access(2) F_OK`). The C body composes
/// `dotdir/fname` and calls `access(F_OK)`.
pub fn check_dotfile(dotdir: &str, fname: &str) -> i32 {                 // c:58
    let mut p = PathBuf::from(dotdir);                                   // c:58-61
    p.push(fname);                                                       // c:60-61
    // C: `access(buf, F_OK)` returns 0 if accessible, -1 with errno
    // set otherwise. Rust's `Path::exists` collapses both into bool.
    if p.exists() { 0 } else { -1 }                                      // c:62
}

/// Port of `boot_(UNUSED(Module m))` from `Src/Modules/newuser.c:68`.
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
#[allow(unused_variables)]
pub fn boot_(m: *const crate::ported::zsh_h::module) -> i32 {           // c:4
    // c:70 — `const char *dotdir = getsparam_u("ZDOTDIR");`. paramtab read.
    let mut dotdir: String = crate::ported::params::getsparam("ZDOTDIR")
        .unwrap_or_default();

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
    if !crate::ported::zsh_h::EMULATION(crate::ported::zsh_h::EMULATE_ZSH) {
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
        if crate::ported::init::source(&buf) != SOURCE_NOT_FOUND {        // c:100
            break;                                                        // c:101
        }
    }

    0                                                                    // c:104
}

/// Port of `cleanup_(UNUSED(Module m))` from `Src/Modules/newuser.c:109`. C body is
/// `return 0;` (UNUSED `Module m`).
#[allow(unused_variables)]
pub fn cleanup_(m: *const crate::ported::zsh_h::module) -> i32 {        // c:109
    0                                                                    // c:116
}

/// Port of `finish_(UNUSED(Module m))` from `Src/Modules/newuser.c:116`. C body is
/// `return 0;` (UNUSED `Module m`).
#[allow(unused_variables)]
pub fn finish_(m: *const crate::ported::zsh_h::module) -> i32 {         // c:116
    0                                                                    // c:116
}

// `SOURCE_NOT_FOUND` is the C `source.c` return code for a missing
// startup script (`init.c:1551` family). zshrs's canonical
// `crate::ported::init::source` returns the same numeric code.
const SOURCE_NOT_FOUND: i32 = 1;

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// c:62 — `access(F_OK)` returns 0 when path exists, -1 otherwise.
    /// Test against a known-existing file and a known-missing file.
    #[test]
    fn check_dotfile_returns_zero_when_file_exists() {
        let tmp = std::env::temp_dir();
        let p = tmp.join("zshrs_test_dotfile_exists");
        fs::write(&p, "").expect("write tmp");
        assert_eq!(check_dotfile(tmp.to_str().unwrap(), "zshrs_test_dotfile_exists"), 0);
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn check_dotfile_returns_minus_one_when_missing() {
        let tmp = std::env::temp_dir();
        // Use a name guaranteed not to exist.
        assert_eq!(check_dotfile(tmp.to_str().unwrap(), "zshrs_test_definitely_nothere_xyz"), -1);
    }

    /// Module entry points all return 0 per `Src/Modules/newuser.c:37-116`.
    #[test]
    fn module_entry_points_return_zero() {
        assert_eq!(setup_(std::ptr::null()),   0);
        assert_eq!(cleanup_(std::ptr::null()), 0);
        assert_eq!(finish_(std::ptr::null()),  0);
    }
}

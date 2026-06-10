//! Newuser module - port of Modules/newuser.c
//!
//! Boot-time dotfile probe. If the user has none of `.zshenv` /
//! `.zprofile` / `.zshrc` / `.zlogin` under `$ZDOTDIR` (or `$HOME`),
//! the C module sources `newuser` from a system-wide script dir to
//! kick off the new-user install wizard.

use crate::ported::init::source;
use crate::ported::params::getsparam;
use crate::ported::zsh_h::{module, EMULATE_ZSH, EMULATION};
use std::path::PathBuf;

/// Port of `setup_(UNUSED(Module m))` from `Src/Modules/newuser.c:37`. C body is
/// `return 0;` (UNUSED `Module m`).
#[allow(unused_variables)]
pub fn setup_(m: *const module) -> i32 {
    // c:37
    0 // c:44
}

/// Port of `features_(UNUSED(Module m), UNUSED(char ***features))` from `Src/Modules/newuser.c:44`. C body is
/// `return 1;` — the newuser module exposes no shell features
/// (no builtins, no ZLE widgets, no params); the non-zero return
/// signals "no feature table" to the loader.
#[allow(unused_variables)]
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {
    // c:44
    1 // c:51
}

/// Port of `enables_(UNUSED(Module m), UNUSED(int **enables))` from `Src/Modules/newuser.c:51`. C body is
/// `return 0;` — no per-feature enables to manage.
#[allow(unused_variables)]
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {
    // c:51
    0 // c:58
}

/// Port of static helper `check_dotfile()` from
/// `Src/Modules/newuser.c:58`. Returns 0 (file accessible) or
/// non-zero (errno via `access(2) F_OK`). The C body composes
/// `dotdir/fname` and calls `access(F_OK)`.
pub fn check_dotfile(dotdir: &str, fname: &str) -> i32 {
    // c:58
    let mut p = PathBuf::from(dotdir); // c:58-61
    p.push(fname); // c:60-61
                   // C: `access(buf, F_OK)` returns 0 if accessible, -1 with errno
                   // set otherwise. Rust's `Path::exists` collapses both into bool.
    if p.exists() {
        0
    } else {
        -1
    } // c:62
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
pub fn boot_(m: *const module) -> i32 {
    // c:4
    // c:70 — `const char *dotdir = getsparam_u("ZDOTDIR");` — the `_u`
    // variant strips zsh's Meta-escape encoding from the returned value.
    // Required here because the path is concatenated with ".zshenv"
    // etc. and passed straight to access(2)/source(); access(2) reads
    // raw filesystem bytes, not zsh's metafied representation.
    //
    // Prior port used the non-`_u` `getsparam` which returns the
    // metafied form. For ASCII-only ZDOTDIR paths the gap was
    // invisible; for any path containing a Meta-escaped byte (e.g.
    // a user with NUL in their config-dir name from a paramtab
    // bytes-set via $'\x00'), the metafied form survived into
    // access(2) which would surface as ENOENT instead of resolving.
    let mut dotdir: String = crate::ported::params::getsparam_u("ZDOTDIR").unwrap_or_default();

    // c:71-78 — `const char *spaths[] = { SITESCRIPT_DIR, SCRIPT_DIR, 0 };`
    // The C source resolves these from configure-time defines; the Rust
    // port reads them from the matching env vars (with reasonable
    // fallbacks) since zshrs doesn't have configure.
    let spaths: Vec<String> = std::env::var("ZSH_SITESCRIPT_DIR")
        .ok()
        .into_iter()
        .chain(std::env::var("ZSH_SCRIPT_DIR").ok())
        .chain(std::iter::once("/etc/zsh".to_string()))
        .collect();

    // c:81 — `if (!EMULATION(EMULATE_ZSH)) return 0;`
    if !EMULATION(EMULATE_ZSH) {
        return 0; // c:82
    } else {
    }

    // c:84-88 — `if (!dotdir) { dotdir = home; if (!dotdir) return 0; }`.
    //
    // C reads the `home` global which is the shell's $HOME param,
    // ALREADY UNMETAFIED at param-init time (the `home` global is
    // populated from getpwuid()->pw_dir which is raw OS bytes — no
    // Meta encoding involved). Route through getsparam_u for the
    // same reason as ZDOTDIR above: the path feeds into access(2).
    if dotdir.is_empty() {
        dotdir = crate::ported::params::getsparam_u("HOME") // c:85
            .unwrap_or_default();
        if dotdir.is_empty() {
            return 0; // c:87
        }
    }

    // c:90-94 — short-circuit if any standard dotfile exists.
    if check_dotfile(&dotdir, ".zshenv")   == 0 ||                       // c:90
       check_dotfile(&dotdir, ".zprofile") == 0 ||                       // c:91
       check_dotfile(&dotdir, ".zshrc")    == 0 ||                       // c:92
       check_dotfile(&dotdir, ".zlogin")   == 0
    {
        // c:93
        return 0; // c:94
    }

    // c:96-102 — try to source `<spath>/newuser` from each system path.
    for sp in &spaths {
        // c:96
        let buf = format!("{}/newuser", sp); // c:98
        if source(&buf) != SOURCE_NOT_FOUND {
            // c:100
            break; // c:101
        }
    }

    0 // c:104
}

/// Port of `cleanup_(UNUSED(Module m))` from `Src/Modules/newuser.c:109`. C body is
/// `return 0;` (UNUSED `Module m`).
#[allow(unused_variables)]
pub fn cleanup_(m: *const module) -> i32 {
    // c:109
    0 // c:116
}

/// Port of `finish_(UNUSED(Module m))` from `Src/Modules/newuser.c:116`. C body is
/// `return 0;` (UNUSED `Module m`).
#[allow(unused_variables)]
pub fn finish_(m: *const module) -> i32 {
    // c:116
    0 // c:116
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
        let _g = crate::test_util::global_state_lock();
        let tmp = std::env::temp_dir();
        let p = tmp.join("zshrs_test_dotfile_exists");
        fs::write(&p, "").expect("write tmp");
        assert_eq!(
            check_dotfile(tmp.to_str().unwrap(), "zshrs_test_dotfile_exists"),
            0
        );
        let _ = fs::remove_file(&p);
    }

    #[test]
    fn check_dotfile_returns_minus_one_when_missing() {
        let _g = crate::test_util::global_state_lock();
        let tmp = std::env::temp_dir();
        // Use a name guaranteed not to exist.
        assert_eq!(
            check_dotfile(tmp.to_str().unwrap(), "zshrs_test_definitely_nothere_xyz"),
            -1
        );
    }

    /// Module entry points all return 0 per `Src/Modules/newuser.c:37-116`.
    #[test]
    fn module_entry_points_return_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(setup_(std::ptr::null()), 0);
        assert_eq!(cleanup_(std::ptr::null()), 0);
        assert_eq!(finish_(std::ptr::null()), 0);
    }

    // ─── zsh-corpus pins for check_dotfile / boot_ ─────────────────

    /// `check_dotfile` returns 0 (success) for an existing file.
    #[test]
    fn newuser_corpus_check_dotfile_existing_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(".zshrc");
        std::fs::File::create(&p).unwrap();
        assert_eq!(
            check_dotfile(dir.path().to_str().unwrap(), ".zshrc"),
            0,
            "existing file = 0 per c:62"
        );
    }

    /// `check_dotfile` returns -1 for a missing file in an existing dir.
    #[test]
    fn newuser_corpus_check_dotfile_missing_in_existing_dir() {
        let _g = crate::test_util::global_state_lock();
        let dir = tempfile::tempdir().unwrap();
        // Dir exists but file does not.
        assert_eq!(
            check_dotfile(dir.path().to_str().unwrap(), "zshrs_no_such_file_xyz"),
            -1,
            "missing file = -1",
        );
    }

    /// `check_dotfile` returns -1 for a missing dir.
    #[test]
    fn newuser_corpus_check_dotfile_missing_dir_returns_neg_one() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(check_dotfile("/never/exists/zshrs_xyz", ".zshrc"), -1,);
    }

    /// `boot_` returns 0 regardless of state.
    #[test]
    fn newuser_corpus_boot_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(boot_(std::ptr::null()), 0);
    }

    /// `features_` returns 1 — newuser module has no advertised
    /// features (zsh source: `Src/Modules/newuser.c:50-54` returns 1
    /// when feature list is empty).
    #[test]
    fn newuser_corpus_features_returns_one_no_features() {
        let _g = crate::test_util::global_state_lock();
        let mut features = Vec::new();
        assert_eq!(
            features_(std::ptr::null(), &mut features),
            1,
            "newuser has no advertised features"
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/newuser.c.
    // ═══════════════════════════════════════════════════════════════════

    /// c:58 — `check_dotfile` for empty fname checks the dir itself
    /// (with empty append). On real dir → 0; on missing → -1.
    #[test]
    fn check_dotfile_real_dir_empty_fname_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        // /tmp exists on every Unix → check_dotfile("/tmp", "") returns 0.
        let r = check_dotfile("/tmp", "");
        assert_eq!(r, 0, "/tmp exists → check_dotfile returns 0");
    }

    /// c:58 — `check_dotfile` for existing dotfile in /tmp → 0.
    #[test]
    fn check_dotfile_existing_in_tmp_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        // Create a real dotfile then check.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".zshrc_test"), "").unwrap();
        let r = check_dotfile(dir.path().to_str().unwrap(), ".zshrc_test");
        assert_eq!(r, 0, "existing file → 0");
    }

    /// c:58 — `check_dotfile` for missing file in existing dir → -1.
    #[test]
    fn check_dotfile_missing_in_real_dir_returns_neg_one() {
        let _g = crate::test_util::global_state_lock();
        let r = check_dotfile("/tmp", ".zshrs_never_real_xyz_zzz");
        assert_eq!(r, -1, "missing file → -1");
    }

    /// c:58 — `check_dotfile` for empty dotdir + missing fname → -1.
    #[test]
    fn check_dotfile_empty_dotdir_missing_returns_neg_one() {
        let _g = crate::test_util::global_state_lock();
        // Empty dotdir + non-existent fname → joined path doesn't exist.
        let r = check_dotfile("", "zshrs_never_exists_xyz");
        assert_eq!(r, -1);
    }

    /// c:16 — setup_(NULL) returns 0.
    #[test]
    fn newuser_setup_returns_zero_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(setup_(std::ptr::null()), 0);
    }

    /// c:34 — `enables_(NULL, _)` returns 0 (or non-panic per port).
    #[test]
    fn newuser_enables_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let mut e: Option<Vec<i32>> = None;
        let _ = enables_(std::ptr::null(), &mut e);
    }

    /// c:154 — `cleanup_(NULL)` returns 0.
    #[test]
    fn newuser_cleanup_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(cleanup_(std::ptr::null()), 0);
    }

    /// c:162 — `finish_(NULL)` returns 0.
    #[test]
    fn newuser_finish_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(finish_(std::ptr::null()), 0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/newuser.c
    // c:58 check_dotfile / c:68 boot_ / c:16-162 lifecycle hooks
    // ═══════════════════════════════════════════════════════════════════

    /// c:58 — `check_dotfile` returns boolean 0 / -1 only (never other).
    #[test]
    fn check_dotfile_returns_zero_or_minus_one_only() {
        let _g = crate::test_util::global_state_lock();
        for (dir, fname) in [
            ("/tmp", "xyz_nonexistent_zshrs_test"),
            ("/nonexistent_dir_xyz_zshrs", "anything"),
            ("/", "etc"),
            ("", ""),
        ] {
            let r = check_dotfile(dir, fname);
            assert!(
                r == 0 || r == -1,
                "result must be 0 or -1, got {} for ({:?}, {:?})",
                r,
                dir,
                fname
            );
        }
    }

    /// c:58 — `check_dotfile` is deterministic for same args.
    #[test]
    fn check_dotfile_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..5 {
            assert_eq!(check_dotfile("/tmp", "xyz_nonexistent"), -1);
        }
    }

    /// c:58 — relative path inputs don't panic.
    #[test]
    fn check_dotfile_relative_path_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = check_dotfile("relative", ".zshrc");
        let _ = check_dotfile(".", ".gitignore"); // possibly exists
        let _ = check_dotfile("..", "anything");
    }

    /// c:58 — multibyte/non-ASCII paths don't panic.
    #[test]
    fn check_dotfile_multibyte_path_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = check_dotfile("/tmp/日本", ".zshrc");
        let _ = check_dotfile("/tmp", "包含中文");
    }

    /// c:68 — `boot_(null)` is safe (returns 0).
    #[test]
    fn newuser_boot_null_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(boot_(std::ptr::null()), 0, "boot_ MUST return 0");
    }

    /// c:68 — boot_ is idempotent (safe to call multiple times).
    #[test]
    fn newuser_boot_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..5 {
            assert_eq!(boot_(std::ptr::null()), 0);
        }
    }

    /// c:16-162 — full lifecycle setup→features→enables→boot→cleanup→finish.
    #[test]
    fn newuser_full_lifecycle_returns_zero_for_all() {
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

    /// c:58 — empty fname against existing dir creates a path that
    /// is the dir itself (which exists) → returns 0.
    #[test]
    fn check_dotfile_empty_fname_with_tmp_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        // "/tmp" + "" = "/tmp/" which exists.
        assert_eq!(check_dotfile("/tmp", ""), 0);
    }

    /// c:58 — paths with embedded NUL handled safely (don't panic;
    /// the FFI layer treats it as terminator or returns error).
    /// Note: PathBuf may panic on interior NUL, so we use a non-NUL
    /// path with NUL-like content avoided.
    #[test]
    fn check_dotfile_special_chars_in_path_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = check_dotfile("/tmp", ".hidden");
        let _ = check_dotfile("/tmp", "file with spaces");
        let _ = check_dotfile("/tmp/sub/dir/that/does/not/exist", "file");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/newuser.c
    // c:37 setup_ / c:44 features_ / c:51 enables_ / c:58 check_dotfile /
    // c:68 boot_ + cleanup_ / finish_ semantic pins
    // ═══════════════════════════════════════════════════════════════════

    /// c:37 — `setup_` returns i32 (compile-time pin).
    #[test]
    fn newuser_setup_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = setup_(std::ptr::null());
    }

    /// c:44 — `features_` returns 1 (NOT 0) — the newuser module exposes
    /// no shell features and the non-zero return signals "no feature
    /// table" to the loader. Pin this distinct-from-0 behaviour vs.
    /// the sibling lifecycle hooks.
    #[test]
    fn newuser_features_returns_one_for_no_feature_table() {
        let _g = crate::test_util::global_state_lock();
        let mut v: Vec<String> = Vec::new();
        let r = features_(std::ptr::null(), &mut v);
        assert_eq!(
            r, 1,
            "c:44 — features_ MUST return 1 (no feature table), not 0"
        );
    }

    /// c:44 — `features_` does NOT populate the out-Vec (no features).
    #[test]
    fn newuser_features_does_not_populate_vec() {
        let _g = crate::test_util::global_state_lock();
        let mut v: Vec<String> = vec!["pre-existing".to_string()];
        let _ = features_(std::ptr::null(), &mut v);
        // Pre-existing content survives (no clobber); no additions.
        assert_eq!(v.len(), 1, "features_ must not add entries");
        assert_eq!(v[0], "pre-existing", "must not clear caller's Vec");
    }

    /// c:51 — `enables_` returns 0 (no per-feature enables to manage).
    /// Distinct from c:44's `1` return; pin separately.
    #[test]
    fn newuser_enables_returns_zero_distinct_from_features() {
        let _g = crate::test_util::global_state_lock();
        let mut e: Option<Vec<i32>> = None;
        let r = enables_(std::ptr::null(), &mut e);
        assert_eq!(r, 0, "c:51 — enables_ MUST return 0");
    }

    /// c:58 — `check_dotfile` returns i32 (compile-time pin).
    #[test]
    fn check_dotfile_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = check_dotfile("/tmp", "anything");
    }

    /// c:58 — `check_dotfile("/tmp", missing)` returns -1 (access F_OK
    /// fail), not 0 or any other sentinel.
    #[test]
    fn check_dotfile_missing_returns_minus_one() {
        let _g = crate::test_util::global_state_lock();
        let r = check_dotfile("/tmp", "__definitely_does_not_exist_xyz_42");
        assert_eq!(r, -1, "missing file MUST return -1, got {}", r);
    }

    /// c:58 — `check_dotfile("/", "tmp")` returns 0 (existing dir
    /// component).
    #[test]
    fn check_dotfile_existing_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let r = check_dotfile("/", "tmp");
        assert_eq!(r, 0, "/ + tmp must exist on Unix; got {}", r);
    }

    /// c:68 — `boot_` returns i32 (compile-time pin).
    #[test]
    fn newuser_boot_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = boot_(std::ptr::null());
    }

    /// c:58 — `check_dotfile` walks dotfile names deterministically:
    /// `.zshenv` / `.zprofile` / `.zshrc` / `.zlogin` against a missing
    /// dir all return -1 (one branch per filename per c:78-81).
    #[test]
    fn check_dotfile_all_four_dotfile_names_against_missing_dir() {
        let _g = crate::test_util::global_state_lock();
        for fname in &[".zshenv", ".zprofile", ".zshrc", ".zlogin"] {
            let r = check_dotfile("/__no_such_dir_xyz_zshrs__", fname);
            assert_eq!(r, -1, "missing dir + {} must return -1, got {}", fname, r);
        }
    }

    /// c:51 — `enables_` is deterministic (read-only path).
    #[test]
    fn newuser_enables_deterministic() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            let mut e: Option<Vec<i32>> = None;
            assert_eq!(enables_(std::ptr::null(), &mut e), 0);
        }
    }

    /// c:44 — `features_` is deterministic (always returns 1).
    #[test]
    fn newuser_features_deterministic() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            let mut v: Vec<String> = Vec::new();
            assert_eq!(features_(std::ptr::null(), &mut v), 1);
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity pins for Src/Modules/newuser.c
    // c:16 setup_ / c:34 enables_ / c:43 check_dotfile /
    // c:92 boot_ / c:154 cleanup_ / c:162 finish_
    // ═══════════════════════════════════════════════════════════════════

    /// c:16 — `setup_` is idempotent.
    #[test]
    fn newuser_setup_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(setup_(std::ptr::null()), 0);
        }
    }

    /// c:154 — `cleanup_` returns 0 + idempotent.
    #[test]
    fn newuser_cleanup_idempotent_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(cleanup_(std::ptr::null()), 0);
        }
    }

    /// c:162 — `finish_` returns 0 + idempotent.
    #[test]
    fn newuser_finish_idempotent_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(finish_(std::ptr::null()), 0);
        }
    }

    /// c:154 — `cleanup_` return type i32 (compile-time pin).
    #[test]
    fn newuser_cleanup_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = cleanup_(std::ptr::null());
    }

    /// c:162 — `finish_` return type i32 (compile-time pin).
    #[test]
    fn newuser_finish_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = finish_(std::ptr::null());
    }

    /// c:34 — `enables_` returns i32 (compile-time pin).
    #[test]
    fn newuser_enables_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let mut e: Option<Vec<i32>> = None;
        let _: i32 = enables_(std::ptr::null(), &mut e);
    }

    /// c:43 — `check_dotfile("", "")` empty inputs don't panic.
    #[test]
    fn check_dotfile_both_empty_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = check_dotfile("", "");
    }

    /// c:43 — `check_dotfile` is deterministic for the same input.
    #[test]
    fn check_dotfile_deterministic_for_same_input() {
        let _g = crate::test_util::global_state_lock();
        let r1 = check_dotfile("/tmp", ".zshrc");
        let r2 = check_dotfile("/tmp", ".zshrc");
        assert_eq!(r1, r2, "check_dotfile must be pure");
    }

    /// c:43 — `check_dotfile` return value always in {-1, 0, 1, ...} —
    /// pin against unexpected negative values beyond -1.
    #[test]
    fn check_dotfile_return_in_canonical_range() {
        let _g = crate::test_util::global_state_lock();
        for fname in [".zshrc", ".zshenv", ".zprofile", ".zlogin"] {
            let r = check_dotfile("/tmp/__never_exists_xyz__", fname);
            assert!(r >= -1, "check_dotfile must return ≥ -1; got {}", r);
        }
    }

    /// c:43 — `check_dotfile` very long path doesn't panic.
    #[test]
    fn check_dotfile_long_path_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let long = "x".repeat(500);
        let _ = check_dotfile(&long, ".zshrc");
    }

    /// c:92 — `boot_` is idempotent (alt).
    #[test]
    fn newuser_boot_idempotent_alt() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            let _ = boot_(std::ptr::null());
        }
    }

    /// c:43 — `check_dotfile` with absolute paths under /tmp doesn't panic.
    #[test]
    fn check_dotfile_various_dotdirs_no_panic() {
        let _g = crate::test_util::global_state_lock();
        for dir in ["/tmp", "/", "/var", "/usr"] {
            let _ = check_dotfile(dir, ".zshrc");
        }
    }
}

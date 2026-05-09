//! Newuser module - port of Modules/newuser.c
//!
//! Boot-time dotfile probe. If the user has none of `.zshenv` /
//! `.zprofile` / `.zshrc` / `.zlogin` under `$ZDOTDIR` (or `$HOME`),
//! the C module sources `newuser` from a system-wide script dir to
//! kick off the new-user install wizard.

use std::path::PathBuf;

/// Port of `setup_()` from `Src/Modules/newuser.c:37`. C body is
/// `return 0;` (UNUSED `Module m`).
pub fn setup_() -> i32 {                                                 // c:37
    0                                                                    // c:40
}

/// Port of `features_()` from `Src/Modules/newuser.c:44`. C body is
/// `return 1;` — the newuser module exposes no shell features
/// (no builtins, no ZLE widgets, no params); the non-zero return
/// signals "no feature table" to the loader.
pub fn features_() -> i32 {                                              // c:44
    1                                                                    // c:47
}

/// Port of `enables_()` from `Src/Modules/newuser.c:51`. C body is
/// `return 0;` — no per-feature enables to manage.
pub fn enables_() -> i32 {                                               // c:51
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

/// Port of `boot_()` from `Src/Modules/newuser.c:68`. Probes
/// `$ZDOTDIR` (or `$HOME`) for any of the four standard zsh
/// dotfiles; if none exist AND emulation is `zsh` (not `sh`/`ksh`/
/// `csh`), sources the system-wide `newuser` install script.
pub fn boot_() -> i32 {                                                  // c:68
    // EMULATE_ZSH gate — C: `if (!EMULATION(EMULATE_ZSH)) return 0;`
    // zshrs's emulation flag lives at `crate::ported::init::ShellEmulation`;
    // we read it via the executor's options when wired. For now,
    // assume zsh emulation (the dominant case in our static-link path).
    // The shell-startup harness in init.rs runs the .zshrc walk
    // separately so this dotfile probe is currently a no-op safety
    // net; mirrors the C body's structure but skips the `source()`
    // dispatch into the system newuser script.
    let _dotdir = std::env::var("ZDOTDIR")                               // c:79
        .ok()
        .or_else(|| std::env::var("HOME").ok())
        .unwrap_or_default();
    if _dotdir.is_empty() { return 0; }                                  // c:84
    // C dotfile probe — c:88-94.
    if check_dotfile(&_dotdir, ".zshenv")   == 0 { return 0; }           // c:88
    if check_dotfile(&_dotdir, ".zprofile") == 0 { return 0; }           // c:89
    if check_dotfile(&_dotdir, ".zshrc")    == 0 { return 0; }           // c:90
    if check_dotfile(&_dotdir, ".zlogin")   == 0 { return 0; }           // c:91
    // System newuser-script source loop (c:96-101) — zshrs leaves
    // first-run setup to a separate post-install step; the source()
    // dispatch is intentionally not wired here. Match C exit.
    0                                                                    // c:103
}

/// Port of `cleanup_()` from `Src/Modules/newuser.c:109`. C body is
/// `return 0;` (UNUSED `Module m`).
pub fn cleanup_() -> i32 {                                               // c:109
    0                                                                    // c:112
}

/// Port of `finish_()` from `Src/Modules/newuser.c:116`. C body is
/// `return 0;` (UNUSED `Module m`).
pub fn finish_() -> i32 {                                                // c:116
    0                                                                    // c:119
}

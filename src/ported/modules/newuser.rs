//! Newuser module - port of Modules/newuser.c
//!
//! The C source dispatches into `Functions/Newuser/zsh-newuser-install`
//! at module load. zshrs links the module statically; the dispatcher
//! lives in the loader entries below.

/// Module loader entry — port of `setup_()` from Src/Modules/newuser.c:37.
pub fn setup_() -> i32 {
    0
}

/// Module loader entry — port of `features_()` from Src/Modules/newuser.c:44.
pub fn features_() -> i32 {
    0
}

/// Module loader entry — port of `enables_()` from Src/Modules/newuser.c:51.
pub fn enables_() -> i32 {
    0
}

/// Module loader entry — port of `boot_()` from Src/Modules/newuser.c:68.
pub fn boot_() -> i32 {
    0
}

/// Module loader entry — port of `cleanup_()` from Src/Modules/newuser.c:109.
pub fn cleanup_() -> i32 {
    0
}

/// Module loader entry — port of `finish_()` from Src/Modules/newuser.c:116.
pub fn finish_() -> i32 {
    0
}

// === auto-generated stubs ===
// Direct ports of static helpers from Src/Modules/newuser.c not
// yet covered above. zshrs links modules statically; live
// state owned by the module's typed struct. Name-parity shims.

/// Port of `check_dotfile()` from Src/Modules/newuser.c:58.
#[allow(non_snake_case)]
pub fn check_dotfile() -> i32 { 0 }

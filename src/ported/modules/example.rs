//! `zsh/example` module — port of `Src/Modules/example.c`.
//!
//! `example.c` is zsh's documentation/template module — it ships
//! with the source tree as a worked example of the loadable-module
//! contract (`setup_`, `getrandom_buffer`, `cleanup_`, `finish_`, the
//! `bintab` / `cotab` / `pmtab` / `mftab` shapes). It is never
//! `zmodload`-ed at runtime by any real script; its purpose is to
//! be read.
//!
//! This Rust file exists so the 1:1 mapping between
//! `Src/Modules/*.c` and `src/modules/*.rs` is complete. Surface
//! is intentionally empty — there's nothing for callers to invoke.
//! The C example exposes a single demo builtin `example` whose body
//! prints flag arguments; we omit it because no zshrs consumer
//! would ever load it.

/// Module loader entry — port of `setup_()` from Src/Modules/example.c:198.
pub fn setup_() -> i32 {
    0
}

/// Module loader entry — port of `features_()` from Src/Modules/example.c:207.
pub fn features_() -> i32 {
    0
}

/// Module loader entry — port of `enables_()` from Src/Modules/example.c:215.
pub fn enables_() -> i32 {
    0
}

/// Module loader entry — port of `boot_()` from Src/Modules/example.c:222.
pub fn boot_() -> i32 {
    0
}

/// Module loader entry — port of `cleanup_()` from Src/Modules/example.c:235.
pub fn cleanup_() -> i32 {
    0
}

/// Module loader entry — port of `finish_()` from Src/Modules/example.c:243.
pub fn finish_() -> i32 {
    0
}

// === auto-generated stubs ===
// Direct ports of static helpers from Src/Modules/example.c not
// yet covered above. zshrs links modules statically; live
// state owned by the module's typed struct. Name-parity shims.

/// Port of `bin_example()` from Src/Modules/example.c:42.
#[allow(non_snake_case)]
pub fn bin_example() -> i32 { 0 }

/// Port of `cond_i_ex()` from Src/Modules/example.c:95.
#[allow(non_snake_case)]
pub fn cond_i_ex() -> i32 { 0 }

/// Port of `cond_p_len()` from Src/Modules/example.c:80.
#[allow(non_snake_case)]
pub fn cond_p_len() -> i32 { 0 }

/// Port of `ex_wrapper()` from Src/Modules/example.c:145.
#[allow(non_snake_case)]
pub fn ex_wrapper() -> i32 { 0 }

/// Port of `math_length()` from Src/Modules/example.c:133.
#[allow(non_snake_case)]
pub fn math_length() -> i32 { 0 }

/// Port of `math_sum()` from Src/Modules/example.c:104.
#[allow(non_snake_case)]
pub fn math_sum() -> i32 { 0 }

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

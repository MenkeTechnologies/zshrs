//! Ported subsystems — every submodule here is a faithful 1:1 port of
//! a corresponding upstream zsh C source file under `src/zsh/Src/`.
//!
//! Companion / opposite of `src/extensions/`. See `docs/PORT.md` for the
//! full ruleset. `tests/port_purity.rs` enforces:
//!   - Every `.rs` file under this directory has a matching `.c` file
//!     under `src/zsh/Src/` (byte-for-byte identical stem).
//!   - Every top-level `fn` carries a doc comment matching the PORT.md
//!     template: `/// Port of NAME() from Src/STEM.c:NNNN`.
//!   - No file may carry the `WARNING: THIS IS ADHOC IMPLEMENTATION`
//!     marker.
//!
//! The crate root re-exports every submodule (`pub use ported::*;` in
//! `src/lib.rs`) so historical call sites that reference
//! `crate::exec::`, `crate::subst::`, `crate::zle::`, etc. continue
//! to resolve unchanged.

pub mod compat;
pub mod cond;
pub mod context;
// Most of `Src/exec.c` is realised by the fusevm wordcode VM at the
// crate root (`src/vm_helper`) rather than in `src/ported/`. The
// genuinely faithful free-function ports from `Src/exec.c` — `gethere`,
// `getoutput`, `loadautofn`, `getfpfunc`, plus the file-static globals
// `trap_state` / `trap_return` / `forklevel` — live in `src/ported/exec.rs`.
// `crate::ported::vm_helper` stays as an alias for the runtime state
// struct + impl methods that hang off it.
pub use crate::vm_helper;
pub mod glob;
pub mod hashnameddir;
pub mod hashtable;
pub mod hashtable_h;
pub mod hist;
pub mod init;
pub mod input;
pub mod jobs;
pub mod linklist;
pub mod r#loop;
pub mod math;
pub mod mem;
pub mod modentry;
pub mod module;
pub mod modules;
pub mod openssh_bsd_setres_id;
pub mod options;
pub mod params;
pub mod pattern;
pub mod prompt;
pub mod signals;
pub mod sort;
pub mod string;
pub mod subst;
pub mod text;
pub mod utils;

pub mod builtin;
pub mod builtins;
pub mod config_h;
pub mod exec;
pub mod exec_hooks;
pub mod lex;
pub mod parse;
pub mod patchlevel;
mod prototypes_h;
pub mod signals_h;
pub mod zle;
pub mod zsh_h;
pub mod zsh_system_h;
pub mod ztype_h;

#[cfg(test)]
mod tests {
    use super::*;
}

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
// `exec` was moved to crate root (src/exec.rs) — it isn't a port of
// Src/exec.c (zshrs replaced the tree-walking interpreter with the
// fusevm bytecode VM). Keep `crate::ported::exec` as a path alias so
// the many existing `crate::ported::exec::*` call-sites still work.
pub use crate::exec;
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
pub mod zle;
pub mod zsh_h;
pub mod zsh_system;
pub mod ztype;
mod prototypes_h;
pub mod patchlevel;
pub mod signals_h;
pub mod config_h;

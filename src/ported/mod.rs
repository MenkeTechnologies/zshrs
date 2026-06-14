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
/// `cond` submodule.
pub mod cond;
/// `context` submodule.
pub mod context;
// Most of `Src/exec.c` is realised by the fusevm wordcode VM at the
// crate root (`src/vm_helper`) rather than in `src/ported/`. The
// genuinely faithful free-function ports from `Src/exec.c` — `gethere`,
// `getoutput`, `loadautofn`, `getfpfunc`, plus the file-static globals
// `trap_state` / `trap_return` / `forklevel` — live in `src/ported/exec.rs`.
// `crate::ported::vm_helper` stays as an alias for the runtime state
// struct + impl methods that hang off it.
pub use crate::vm_helper;
/// `builtin` submodule.
pub mod builtin;
/// `builtins` submodule.
pub mod builtins;
/// `config_h` submodule.
pub mod config_h;
/// `exec` submodule.
pub mod exec;
/// `glob` submodule.
pub mod glob;
/// `hashnameddir` submodule.
pub mod hashnameddir;
/// `hashtable` submodule.
pub mod hashtable;
/// `hashtable_h` submodule.
pub mod hashtable_h;
/// `hist` submodule.
pub mod hist;
/// `init` submodule.
pub mod init;
/// `input` submodule.
pub mod input;
/// `jobs` submodule.
pub mod jobs;
/// `lex` submodule.
pub mod lex;
/// `linklist` submodule.
pub mod linklist;
/// `r` submodule.
pub mod r#loop;
/// `math` submodule.
pub mod math;
/// `mem` submodule.
pub mod mem;
/// `modentry` submodule.
pub mod modentry;
/// `module` submodule.
pub mod module;
/// `modules` submodule.
pub mod modules;
/// `openssh_bsd_setres_id` submodule.
pub mod openssh_bsd_setres_id;
/// `options` submodule.
pub mod options;
/// `params` submodule.
pub mod params;
/// `parse` submodule.
pub mod parse;
/// `patchlevel` submodule.
pub mod patchlevel;
/// `pattern` submodule.
pub mod pattern;
/// `prompt` submodule.
pub mod prompt;
mod prototypes_h;
/// `signals` submodule.
pub mod signals;
/// `signals_h` submodule.
pub mod signals_h;
/// `sort` submodule.
pub mod sort;
/// `string` submodule.
pub mod string;
/// `subst` submodule.
pub mod subst;
/// `text` submodule.
pub mod text;
/// `utils` submodule.
pub mod utils;
/// `zle` submodule.
pub mod zle;
/// `zsh_h` submodule.
pub mod zsh_h;
/// `zsh_system_h` submodule.
pub mod zsh_system_h;
/// `ztype_h` submodule.
pub mod ztype_h;

#[cfg(test)]
mod tests {
    use super::*;
}

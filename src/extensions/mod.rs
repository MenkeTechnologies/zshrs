//! zshrs extensions — features zsh C does **not** have.
//!
//! This is the **only** sanctioned non-port directory in the tree
//! (alongside `src/recorder/`). Every file here implements
//! functionality that goes beyond upstream zsh: AOT compilation,
//! daemon coordination, autoload/script/plugin caches, fish-style
//! features (autosuggest, abbreviation expansion, syntax-highlight),
//! persistent worker pools, ZWC byte-code helpers used by the AOT
//! pipeline, etc. These have no corresponding C function and are not
//! expected to.
//!
//! Rules (see `docs/PORT.md` for the canonical statement):
//!
//!   1. Files here MUST implement a feature that zsh C demonstrably
//!      does **not** have. If a similar feature exists in zsh, port
//!      it instead — the port belongs under `src/ported/`.
//!   2. Files here MUST NOT duplicate or shadow any port. Extensions
//!      are additive only.
//!   3. The `port_purity` integration test exempts `src/extensions/`
//!      from the 1:1 file-existence rule on this basis.
//!
//! These submodules are also exposed at the crate root via
//! `#[path = "extensions/<name>.rs"] pub mod <name>;` declarations in
//! `src/lib.rs` and `src/ported/zle/mod.rs` so existing call-sites
//! that reference `crate::aot::`, `crate::completion::`, etc. continue
//! to resolve. Both paths point at the same file — they are aliases.

pub use crate::aot;
pub use crate::arith_compiler;
pub use crate::ast_sexp;
pub use crate::autoload_cache;
pub use crate::bash_arrays;
pub use crate::bash_prompt;
pub use crate::canonical_apply;
pub use crate::compile_zsh;
pub use crate::completion;
pub use crate::config;
pub use crate::daemon_presence;
pub use crate::dap;
pub use crate::dash_mode;
pub use crate::emulation_output;
pub use crate::emulation_startup;
pub use crate::ext_builtins;
pub use crate::fds;
pub use crate::fish_features;
pub use crate::global_rc;
pub use crate::heredoc_ast;
pub use crate::history;
pub use crate::log;
pub use crate::lsp;
pub use crate::overlay_snapshot;
pub use crate::p10k;
pub use crate::pkg;
pub use crate::plugin_cache;
pub use crate::plugin_host;
pub use crate::provenance;
pub use crate::regex_mod;
pub use crate::script_cache;
pub use crate::stringsort;
pub use crate::terminfo_caps;
pub use crate::terminfo_db;
pub use crate::tparm;
pub use crate::worker;
pub use crate::zsh_ast;
pub use crate::ztest;
pub use crate::zwc;
pub use crate::zwc_decode;

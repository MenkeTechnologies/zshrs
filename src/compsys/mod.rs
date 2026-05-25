//! Zsh-compatible completion system (compsys)
//!
//! This module implements zsh's new completion system with full compatibility
//! for compadd, compset, zstyle, and all completion special parameters.
//!
//! # Architecture
//!
//! ## Default Mode (SQLite-backed)
//! - Cache: `~/.zshrs/compsys.db` (55MB with 16,872 function bodies)
//! - `compinit`: Parallel fpath scan with rayon, stores bodies in SQLite
//! - `autoload -Xz`: Instant lookup from SQLite (~2.7µs)
//! - No .zcompdump file created
//!
//! ## --zsh-compat Mode (Traditional)
//! - Cache: `~/.zcompdump` (761KB)
//! - `compinit`: Sequential scan, creates .zcompdump
//! - `autoload -Xz`: Scans fpath/zwc files (~70µs)
//! - Full zsh behavior for debugging/compatibility
//!
//! # Usage
//! ```text
//! // Default mode (recommended)
//! zshrs -c 'compinit'
//!
//! // Compat mode
//! zshrs --zsh-compat -c 'compinit'
//! ```
//!
//! Architecture based on analysis of:
//! - zsh Src/Zle/compcore.c, complete.c, computil.c
//! - fish src/complete.rs (for Rust patterns)

#![allow(dead_code)]
#![allow(unused_variables)]

/// Canonical names of compsys-style completion functions (the
/// underscore-prefixed `_arguments` / `_files` / `_describe` / …)
/// that have native Rust implementations in this crate, so they
/// don't hit the slow shell-function autoload path.
///
/// Sources:
///   * Core dispatcher / completers: [`base`] (main_complete, normal,
///     dispatch, alternative, values, complete/ignored/approximate/
///     correct/expand/history/match/menu/prefix, description, message,
///     requested, wanted, all_labels, next_label, sep_parts)
///   * Arg-spec engine: [`_arguments`] (`_arguments`)
///   * Description rendering: [`_describe`] / [`computil`] (`_describe`)
///   * File / path completers: [`_files`] / [`library`] (`_files`,
///     `_directories`, `_path_files`, `_tilde_files`)
///   * Per-command completers wired in `main` (`_git`, `_docker`,
///     `_cargo`, `_kubectl`, `_terraform`, plus `_ls` / `_cd` / `_cp`
///     / `_mv` / `_rm` / `_cat` / `_grep` baseline stubs).
///
/// Used by `lsp::dump_reflection_json` to populate the IntelliJ tool
/// window "Compsys" tab and by `lsp::dump_reference_html` for the
/// `ch-lsp-compsys` chapter in `docs/reference.html`. When a new `_*`
/// function gets a Rust impl, add it here (sorted) so it surfaces in
/// the inventory.
pub const COMPSYS_FN_NAMES: &[&str] = &[
    "_all_labels",
    "_alternative",
    "_approximate",
    "_arguments",
    "_call_program",
    "_canonical_paths",
    "_cargo",
    "_cat",
    "_cd",
    "_combination",
    "_command_names",
    "_complete",
    "_completers",
    "_correct",
    "_cp",
    "_describe",
    "_description",
    "_dir_list",
    "_directories",
    "_dispatch",
    "_docker",
    "_email_addresses",
    "_expand",
    "_files",
    "_git",
    "_grep",
    "_history",
    "_ignored",
    "_kubectl",
    "_ls",
    "_main_complete",
    "_match",
    "_menu",
    "_message",
    "_multi_parts",
    "_mv",
    "_next_label",
    "_normal",
    "_numbers",
    "_path_files",
    "_pick_variant",
    "_prefix",
    "_requested",
    "_rm",
    "_sep_parts",
    "_sequence",
    "_tags",
    "_terraform",
    "_tilde_files",
    "_values",
    "_wanted",
    "_widgets",
];

pub mod base;
// builtin_bridge.rs deleted — middleman wrappers around bin_compadd/
// bin_zstyle/etc. Callers invoke `crate::ported::zle::complete::bin_*`,
// `crate::ported::modules::zutil::bin_*`, `lookupstyle`, `testforstyle`
// directly.
pub mod cache;
// compcore.rs deleted — dup of src/ported/zle/compcore.rs.
pub mod compdef;
// completion.rs deleted — Completion/CompletionFlags/CompletionGroup
// are dups of Cmatch/Cmgroup types in src/ported/zle/comp_h.rs.
// menu.rs deleted — was a stub of what used to be a 3567-LOC dup of
// src/ported/zle/complist.rs.
pub mod ported;
// state.rs deleted — dup of shell parameter table for completion
// (PREFIX/SUFFIX/IPREFIX/ISUFFIX/QIPREFIX/QISUFFIX/CURRENT/words/
// compstate) ported in src/ported/zle/complete.rs + accessed via
// getsparam/setsparam/getiparam/setiparam from src/ported/params.rs.
// `zstyle.rs` (the () facade) deleted as middleman.
// Engine ports call `crate::ported::modules::zutil::lookupstyle`
// / `testforstyle` directly inline.
// Deleted shell-builtin dups: compadd, compset, computil, zstyle (in
// src/ported/zle/{complete,computil}.rs + src/ported/modules/zutil.rs),
// compcore + completion + matching (in src/ported/zle/{compcore,
// comp_h,compmatch}.rs), menu (in src/ported/zle/complist.rs),
// state (shell-side globals in src/ported/zle/compcore.rs), zle (in
// src/ported/zle/zle_*.rs). Callers route through the real
// `bin_*`/state in `crate::ported::*`.
// system.rs deleted — dup of completion-extension code that depended
// on the deleted Completion/CompletionReceiver types.
pub mod zpwr_colors;

pub use base::{
    // Tag/spec types (still defined in base.rs)
    CompleterResult,
    CompletionContext as BaseCompletionContext,
    MainCompleteState,
    Value
};
// TagManager removed — dup of comptags machinery in
// src/ported/zle/computil.rs::bin_comptags.
// Engine-port re-exports gutted alongside compcore.rs deletion — every
// `_<name>` port body was stubbed because its body depended on the
// deleted `CompletionState`. Re-add as ports are properly migrated to
// use `crate::ported::zle::compcore::addmatch` against shell-side state.
pub use ported::compinit::{
    build_cache_from_fpath, cache_entry_count, cache_is_valid, check_dump, compdump, compinit,
    compinit_lazy, get_system_fpath, load_from_cache, CompDef, CompFile, CompFileDef, CompInitOpts,
    CompInitResult,
};

// completion::* re-exports removed alongside completion.rs deletion.
// Engine ports must use `crate::ported::zle::comp_h::{Cmatch, Cmgroup}`.
// state::* re-exports removed alongside state.rs deletion.
// menu::* re-exports removed alongside menu.rs deletion.

pub use zpwr_colors::{
    load_zpwr_config, parse_zstyles_from_config, parse_zstyles_from_content, zpwr_list_colors,
    HeaderColors, ParsedZstyle, ZstyleColors, DEFAULT_PREFIX_COLOR, MENU_SELECTION_COLOR,
};

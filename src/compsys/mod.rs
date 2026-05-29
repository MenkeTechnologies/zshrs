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
///   * Core dispatcher / completers: `base` (main_complete, normal,
///     dispatch, alternative, values, complete/ignored/approximate/
///     correct/expand/history/match/menu/prefix, description, message,
///     requested, wanted, all_labels, next_label, sep_parts)
///   * Arg-spec engine: `_arguments`
///   * Description rendering: `_describe` (in `computil`)
///   * File / path completers: `_files`, `_directories`,
///     `_path_files`, `_tilde_files`
///   * Per-command completers wired in `main`: `_git`, `_docker`,
///     `_cargo`, `_kubectl`, `_terraform`, plus `_ls` / `_cd` / `_cp`
///     / `_mv` / `_rm` / `_cat` / `_grep` baseline stubs.
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

// base.rs deleted — the type stubs (MainCompleteState /
// CompletionContext / CompleterResult / Value / CompleterFn) were
// never wired through the real engine ports. State flows through
// the shell-side param table via getsparam/setsparam directly; tag
// accounting goes through `bin_comptags` / `_tags` / `_requested`
// in `src/ported/zle/computil.rs` + `src/compsys/ported/Base/Core/`.
// builtin_bridge.rs deleted — middleman wrappers around bin_compadd/
// bin_zstyle/etc. Callers invoke `crate::ported::zle::complete::bin_*`,
// `crate::ported::modules::zutil::bin_*`, `lookupstyle`, `testforstyle`
// directly.
/// `cache` submodule.
pub mod cache;
/// In-editor compsys completion entry point — see
/// `docs/IN_EDITOR_COMPSYS_COMPLETION.md` for the design.
pub mod in_editor;
// completion.rs deleted — Completion/CompletionFlags/CompletionGroup
// are dups of Cmatch/Cmgroup types in src/ported/zle/comp_h.rs.
// menu.rs deleted — was a stub of what used to be a 3567-LOC dup of
// src/ported/zle/complist.rs.
/// `ported` submodule.
pub mod ported;
/// Rust-vs-shell backend dispatcher consulted by
/// `vm_helper::dispatch_function_call` before the standard shfunc
/// lookup. Honors `[compsys] backend = "rust"|"shell"` from the
/// TOML config (`extensions::config::CompsysConfig`).
pub mod router;
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
/// `zpwr_colors` submodule.
pub mod zpwr_colors;

// base::{CompleterResult, BaseCompletionContext, MainCompleteState,
// Value} re-exports deleted alongside base.rs — those types were
// never load-bearing in any active engine port. Engine ports use
// shell-side state via getsparam/setsparam directly.
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

#[cfg(test)]
mod tests {
    use super::*;

    // === COMPSYS_FN_NAMES inventory invariants ===

    #[test]
    fn compsys_fn_names_is_nonempty() {
        assert!(
            !COMPSYS_FN_NAMES.is_empty(),
            "inventory must list at least one native completer"
        );
        // The comment in the source enumerates ~50 entries across
        // base / core / utility / per-command completers. Sanity
        // floor to catch the table being accidentally truncated.
        assert!(
            COMPSYS_FN_NAMES.len() >= 30,
            "expected ≥30 entries in COMPSYS_FN_NAMES, got {}",
            COMPSYS_FN_NAMES.len()
        );
    }

    #[test]
    fn compsys_fn_names_all_start_with_underscore() {
        // The compsys convention (zsh Completion/) is that every
        // completion function name begins with `_`. Anything else
        // here is a typo and would break dispatcher routing.
        for name in COMPSYS_FN_NAMES {
            assert!(
                name.starts_with('_'),
                "compsys function name {name:?} must start with `_`"
            );
        }
    }

    #[test]
    fn compsys_fn_names_have_no_empty_entries() {
        for name in COMPSYS_FN_NAMES {
            assert!(!name.is_empty(), "empty entry in COMPSYS_FN_NAMES");
            // `_` alone is also invalid — needs a body after the
            // underscore prefix.
            assert!(name.len() >= 2, "stub entry {name:?} too short");
        }
    }

    #[test]
    fn compsys_fn_names_are_unique() {
        // The list is a source-of-truth inventory consumed by the
        // LSP / docs. Duplicates would inflate counts and confuse
        // the IntelliJ tool window enumeration.
        let mut seen = std::collections::HashSet::new();
        for name in COMPSYS_FN_NAMES {
            assert!(
                seen.insert(*name),
                "duplicate entry {name:?} in COMPSYS_FN_NAMES"
            );
        }
    }

    #[test]
    fn compsys_fn_names_are_sorted() {
        // The doc says "(sorted)". A pre-sorted slice supports binary
        // search if a hot path ever needs O(log n) lookup, and it
        // makes the source diff stable across edits.
        let mut sorted = COMPSYS_FN_NAMES.to_vec();
        sorted.sort();
        assert_eq!(
            sorted,
            COMPSYS_FN_NAMES.to_vec(),
            "COMPSYS_FN_NAMES must be sorted"
        );
    }

    #[test]
    fn compsys_fn_names_use_only_underscore_and_alnum() {
        // Compsys function names are valid zsh identifiers — they may
        // contain alphanumerics and underscore only, since they are
        // shell function names that also have native Rust impls.
        for name in COMPSYS_FN_NAMES {
            assert!(
                name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
                "bad character in compsys function name {name:?}"
            );
        }
    }

    #[test]
    fn compsys_fn_names_include_core_dispatcher() {
        // `_main_complete` is the top-level entry point for the
        // compsys engine — it must be in the inventory or the
        // completion path is broken.
        assert!(
            COMPSYS_FN_NAMES.contains(&"_main_complete"),
            "_main_complete must be in COMPSYS_FN_NAMES"
        );
    }

    #[test]
    fn compsys_fn_names_include_arguments_engine() {
        // `_arguments` is the arg-spec engine used by ~every per-tool
        // completer. Must be in the native inventory.
        assert!(COMPSYS_FN_NAMES.contains(&"_arguments"));
    }

    #[test]
    fn compsys_fn_names_include_file_path_completers() {
        // File/path completers are the most-used surface — verify
        // they're all in the native inventory.
        for fn_name in [
            "_files",
            "_directories",
            "_path_files",
            "_tilde_files",
            "_canonical_paths",
        ] {
            assert!(
                COMPSYS_FN_NAMES.contains(&fn_name),
                "path completer {fn_name:?} missing from inventory"
            );
        }
    }

    #[test]
    fn compsys_fn_names_include_per_command_wired_in_main() {
        // The header comment promises these are wired by `main` —
        // they must therefore appear in the inventory.
        for fn_name in ["_git", "_docker", "_cargo", "_kubectl", "_terraform"] {
            assert!(
                COMPSYS_FN_NAMES.contains(&fn_name),
                "per-command completer {fn_name:?} missing from inventory"
            );
        }
    }

    #[test]
    fn compsys_fn_names_include_baseline_unix_stubs() {
        // The header comment also lists baseline ls/cd/cp/mv/rm/cat/grep
        // stubs. Verify they're all present.
        for fn_name in ["_ls", "_cd", "_cp", "_mv", "_rm", "_cat", "_grep"] {
            assert!(
                COMPSYS_FN_NAMES.contains(&fn_name),
                "baseline stub {fn_name:?} missing from inventory"
            );
        }
    }

    #[test]
    fn compsys_fn_names_include_tag_machinery() {
        // The tag manager / requested-wanted dispatch lives in
        // `_tags` / `_requested` / `_wanted` — all in the inventory.
        for fn_name in [
            "_tags",
            "_requested",
            "_wanted",
            "_all_labels",
            "_next_label",
        ] {
            assert!(
                COMPSYS_FN_NAMES.contains(&fn_name),
                "tag machinery {fn_name:?} missing from inventory"
            );
        }
    }
}

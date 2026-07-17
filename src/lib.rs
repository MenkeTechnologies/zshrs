//! Zsh interpreter and parser in Rust
//!
//! This crate provides:
//! - A complete zsh lexer (`lexer` module)
//! - A zsh parser (`parser` module)  
//! - Shell execution engine (`exec` module)
//! - Job control (`jobs` module)
//! - History management (`history` module)
//! - ZLE (Zsh Line Editor) support (`zle` module)
//! - ZWC (compiled zsh) support (`zwc` module)
//! - Fish-style features (`fish_features` module)
//! - Mathematical expression evaluation (`math` module)

// Many doc comments reference C-source pages, shell constructs, and
// zsh-internal identifiers by name in `[...]` form; they don't resolve as
// rustdoc intra-doc links. Silence so docs build clean on CI.
#![allow(rustdoc::broken_intra_doc_links)]
#![allow(rustdoc::private_intra_doc_links)]
#![allow(rustdoc::invalid_html_tags)]
#![allow(dead_code)]
#![allow(unused_variables)]
#![allow(unused_imports)]
#![allow(unused_assignments)]
#![allow(unused_mut)]
#![allow(unused_parens)]
#![allow(unused_doc_comments)]
#![allow(unreachable_patterns)]
#![allow(deprecated)]
#![allow(unexpected_cfgs)]
// Allow zsh-canonical identifier names (lowercase statics/constants/types
// like `ca_parsed`, `convchar_t`, `P_ISBRANCH`) so the ports stay
// faithful to the C source per PORT.md.
#![allow(non_snake_case)]
#![allow(non_camel_case_types)]
#![allow(non_upper_case_globals)]
// Function-pointer-to-integer casts appear in ported dispatch tables.
#![allow(function_casts_as_integer)]
// Clippy: the C → Rust ports preserve idioms from the zsh source
// (raw pointer derefs, dead-loop `do { ... } while (0)` shapes, bitmasks
// that look redundant but match the C, etc.). Silence the whole group so
// port fidelity wins over Rust-idiom rewrites. New non-ported code
// should still aim for clippy-clean, but at file/function scope, not
// crate-wide.
#![allow(clippy::all)]

/// Runtime shell-mode flag set by the binary entrypoint (`bins/zshrs.rs`)
/// at startup. The library can't directly read `bins/zshrs.rs::shell_mode()`
/// (it lives in the binary crate), so the binary writes this atomic when
/// parsing `--zsh` / `--bash` / `--posix` and the library reads it from
/// bridge / dispatch sites that need to gate bash-compat-vs-zsh behavior.
/// Defaults to `false` (zshrs-native mode) when not explicitly set.
///
/// Bugs #475 / #504 / #555 in docs/BUGS.md — bash-only builtins
/// (`caller`/`help`/`mapfile`/`readarray`/`compgen`/etc.) should
/// dispatch as "command not found" when this is true.
pub static IS_ZSH_MODE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// `compsys` submodule.
pub mod compsys;
/// `exec_jobs` submodule.
pub mod exec_jobs;
/// `extensions` submodule.
pub mod extensions;
/// `ported` submodule.
pub mod ported;
/// `test_util` submodule.
#[cfg(test)]
pub mod test_util;

// Back-compat: re-export every ported submodule at the crate root so
// historical call sites (`crate::exec::`, `crate::subst::`,
// `crate::zle::`, `crate::modules::`, `crate::builtins::`, etc.)
// continue to resolve unchanged after the physical move into
// `src/ported/`. New code should prefer `crate::ported::<name>`.
pub use ported::*;
/// `aot` submodule.
#[path = "extensions/aot.rs"]
pub mod aot;
/// `arith_compiler` submodule.
#[path = "extensions/arith_compiler.rs"]
pub mod arith_compiler;
/// `autoload_cache` submodule.
#[path = "extensions/autoload_cache.rs"]
pub mod autoload_cache;
/// `bash_complete` submodule.
#[path = "extensions/bash_complete.rs"]
pub mod bash_complete;
/// `canonical_apply` submodule.
#[path = "extensions/canonical_apply.rs"]
pub mod canonical_apply;
/// `compile_zsh` submodule.
#[path = "extensions/compile_zsh.rs"]
pub mod compile_zsh;
/// `completion` submodule.
#[path = "extensions/completion.rs"]
pub mod completion;
/// `config` submodule.
#[path = "extensions/config.rs"]
pub mod config;
/// `daemon_presence` submodule.
#[path = "extensions/daemon_presence.rs"]
pub mod daemon_presence;
/// `overlay_snapshot` submodule.
#[path = "extensions/overlay_snapshot.rs"]
pub mod overlay_snapshot;
/// `fast_hash` submodule — dependency-free FxHash for internal name tables.
#[path = "extensions/fast_hash.rs"]
pub mod fast_hash;
/// `opts_cache` submodule — fast-path `isset()` option-state cache.
#[path = "extensions/opts_cache.rs"]
pub mod opts_cache;
/// `pat_cache` submodule — global compiled-pattern cache (Rust-only opt).
#[path = "extensions/pat_cache.rs"]
pub mod pat_cache;
/// `vm_pool` submodule — per-thread pool of recyclable fusevm VMs.
#[path = "extensions/vm_pool.rs"]
pub mod vm_pool;
/// `subexp_cleanup` submodule — RAII eviction of `__subexp_arr_*`
/// paramtab scratch temps created during array sub-expression expansion.
#[path = "extensions/subexp_cleanup.rs"]
pub mod subexp_cleanup;
/// `script_cache` submodule.
#[path = "extensions/script_cache.rs"]
pub mod script_cache;
// Daemon lives in the `zshrs-daemon` workspace crate. Re-export it as `daemon`
// so existing `crate::daemon::...` (in vm_helper) and `zsh::daemon::...` (in bins,
// integration tests) paths keep resolving without churn.
//
// The `daemon` feature gates the actual zshrs-daemon dep. When disabled
// (--no-default-features), a stub module covers the call sites in vm_helper.
// This lets the library compile in isolation while the daemon crate is
// being refactored in a concurrent session.
#[cfg(feature = "daemon")]
pub use zshrs_daemon as daemon;
/// `daemon` submodule.
#[cfg(not(feature = "daemon"))]
pub mod daemon {
    //! Stub module used when the `daemon` feature is disabled. Provides
    //! the minimal surface that `src/vm_helper` calls — the real
    //! implementation lives in the `zshrs-daemon` workspace crate.
    pub mod builtins {
        pub const ZSHRS_BUILTIN_NAMES: &[&str] = &[];
        /// `is_zshrs_builtin` — see implementation.
        pub fn is_zshrs_builtin(_name: &str) -> bool {
            false
        }
        /// `try_dispatch` — see implementation.
        pub fn try_dispatch(_name: &str, _argv: &[String]) -> Option<i32> {
            None
        }
        /// `dispatch` — see implementation.
        pub fn dispatch(_name: &str, _args: &[String]) -> Option<i32> {
            None
        }
    }
}
/// `ast_sexp` submodule.
#[path = "extensions/ast_sexp.rs"]
pub mod ast_sexp;
/// `dap` submodule.
#[path = "extensions/dap.rs"]
pub mod dap;
/// `dumpers` submodule.
#[path = "extensions/dumpers.rs"]
pub mod dumpers;
/// `ext_builtins` submodule.
#[path = "extensions/ext_builtins.rs"]
pub mod ext_builtins;
/// `fds` submodule.
#[path = "extensions/fds.rs"]
pub mod fds;
/// `fish_features` submodule.
#[path = "extensions/fish_features.rs"]
pub mod fish_features;
/// `fmt` submodule — zsh source formatter (CLI `--fmt` + LSP
/// `textDocument/formatting`).
#[path = "extensions/fmt.rs"]
pub mod fmt;
/// `func_body_fmt` submodule.
#[path = "extensions/func_body_fmt.rs"]
pub mod func_body_fmt;
/// `lsp` submodule.
#[path = "extensions/lsp.rs"]
pub mod lsp;
/// `lsp_symbols` submodule.
#[path = "extensions/lsp_symbols.rs"]
pub mod lsp_symbols;
// Lexer + parser live in `src/ported/lex.rs` and `src/ported/parse.rs`.
// Re-export the modules so existing call sites (`zsh::lex::…`,
// `zsh::parse::…`, `zsh::tokens::…`) keep resolving.
// `tokens` aliases `lex` because tokens.rs's contents (lextok enum +
// reserved-word table) now live inside lex.rs. Char tokens (Pound / Inpar /
// Equals / …) and the REDIR_* / COND_* constants are not duplicated — they
// live as flat `pub const` items in `ported::zsh_h` per `Src/zsh.h:144-679`.
pub use ported::lex;
pub use ported::lex as tokens;
pub use ported::parse;
/// `heredoc_ast` submodule.
#[path = "extensions/heredoc_ast.rs"]
pub mod heredoc_ast;
/// `history` submodule.
#[path = "extensions/history.rs"]
pub mod history;
/// `history_lazy` submodule — on-demand HISTFILE paging; the history
/// is never slurped whole.
#[path = "extensions/history_lazy.rs"]
pub mod history_lazy;
/// `log` submodule.
#[path = "extensions/log.rs"]
pub mod log;
/// `lowfd` submodule — keeps the shell's own descriptors out of the user's fd space.
#[path = "extensions/lowfd.rs"]
pub mod lowfd;
/// `zsh_ast` submodule.
#[path = "extensions/zsh_ast.rs"]
pub mod zsh_ast;
// Backwards-compat flat re-exports — call sites that still write
// `crate::datetime::…`, `crate::stat::…`, etc. resolve to the
// `crate::modules::<modname>` ports without churn. New code should
// reach for `crate::modules::<modname>` directly.
pub use builtins::sched;
pub use modules::attr;
pub use modules::cap;
pub use modules::clone;
pub use modules::curses;
pub use modules::datetime;
pub use modules::db_gdbm;
pub use modules::example;
pub use modules::files;
pub use modules::hlgroup;
pub use modules::ksh93;
pub use modules::langinfo;
pub use modules::mapfile;
pub use modules::mathfunc;
pub use modules::nearcolor;
pub use modules::newuser;
pub use modules::param_private;
pub use modules::parameter;
pub use modules::pcre;
pub use modules::random;
pub use modules::random_real;
pub use modules::regex as regex_module;
pub use modules::socket;
pub use modules::stat;
pub use modules::system;
pub use modules::tcp;
pub use modules::termcap;
pub use modules::terminfo;
pub use modules::watch;
pub use modules::zftp;
pub use modules::zprof;
pub use modules::zpty;
pub use modules::zselect;
pub use modules::zutil;
/// `compinit_bg` submodule.
#[path = "extensions/compinit_bg.rs"]
pub mod compinit_bg;
/// `fusevm_bridge` submodule.
pub mod fusevm_bridge;
/// `fusevm_disasm` submodule.
pub mod fusevm_disasm;
/// `intercepts` submodule.
#[path = "extensions/intercepts.rs"]
pub mod intercepts;
/// `p10k` submodule — native powerlevel10k prompt engine.
#[path = "extensions/p10k/mod.rs"]
pub mod p10k;
/// `plugin_cache` submodule.
#[path = "extensions/plugin_cache.rs"]
pub mod plugin_cache;
/// `recorder_ext` submodule.
#[path = "extensions/recorder.rs"]
pub mod recorder_ext;
// Plugin-Framework-Agnostic State-Modification Recorder. Entire module
// is `#![cfg(feature = "recorder")]` so it disappears from the default
// `zshrs` build at the rustc-expansion stage. See docs/RECORDER.md.
/// `gen_docs` submodule.
#[path = "extensions/gen_docs.rs"]
pub mod gen_docs;
/// `recorder` submodule.
#[cfg(feature = "recorder")]
pub mod recorder;
/// `regex_mod` submodule.
#[path = "extensions/regex_mod.rs"]
pub mod regex_mod;
/// `stringsort` submodule.
#[path = "extensions/stringsort.rs"]
pub mod stringsort;
/// `worker` submodule.
#[path = "extensions/worker.rs"]
pub mod worker;
/// `zsh_builtin_docs` submodule.
#[path = "extensions/zsh_builtin_docs.rs"]
pub mod zsh_builtin_docs;
/// `zsh_ext_builtin_docs` submodule.
#[path = "extensions/zsh_ext_builtin_docs.rs"]
pub mod zsh_ext_builtin_docs;
/// `zsh_keyword_docs` submodule.
#[path = "extensions/zsh_keyword_docs.rs"]
pub mod zsh_keyword_docs;
/// `zsh_option_docs` submodule.
#[path = "extensions/zsh_option_docs.rs"]
pub mod zsh_option_docs;
/// `zsh_special_var_docs` submodule.
#[path = "extensions/zsh_special_var_docs.rs"]
pub mod zsh_special_var_docs;
/// `ztest` submodule — shell-level unit test framework
/// (port of `../strykelang` test framework).
#[path = "extensions/ztest.rs"]
pub mod ztest;
/// `zle_param_sync` submodule — ZLE special-param write-back sync
/// (Rust-only adapter for C's live GSU setters).
#[path = "extensions/zle_param_sync.rs"]
pub mod zle_param_sync;
/// `zle_file_tester` submodule — file-existence/permission tests for native ZLE
/// syntax highlighting (port of fish highlight/file_tester.rs).
#[path = "extensions/zle_file_tester.rs"]
pub mod zle_file_tester;
/// `syntax_highlight` submodule — native command-line syntax highlighting
/// (port of fish highlight/highlight.rs, driven by the zshrs lexer).
#[path = "extensions/syntax_highlight.rs"]
pub mod syntax_highlight;
/// `autosuggest` submodule — native fish-style autosuggestions
/// (port of the reader.rs autosuggestion state machine).
#[path = "extensions/autosuggest.rs"]
pub mod autosuggest;
/// `history_search` submodule — native up-arrow prefix/substring/token history
/// search (port of fish reader/history_search.rs).
#[path = "extensions/history_search.rs"]
pub mod history_search;
/// `zle_fx` submodule — wiring for the native ZLE effects (autosuggest,
/// syntax highlight, history search, autopair) into zlecore + the renderer.
#[path = "extensions/zle_fx.rs"]
pub mod zle_fx;
/// `autopair` submodule — native bracket/quote auto-pairing
/// (port of hlissner/zsh-autopair).
#[path = "extensions/autopair.rs"]
pub mod autopair;
/// `zwc` submodule.
#[path = "extensions/zwc.rs"]
pub mod zwc;
/// `zwc_decode` submodule.
#[path = "extensions/zwc_decode.rs"]
pub mod zwc_decode;
// Backwards-compat re-export so `crate::rlimits::…` keeps resolving.
pub use builtins::rlimits;

// Top-level shell executor state + fusevm bridge glue. Not a port of
// any single Src/*.c file — zsh's native wordcode VM lives in `Src/exec.c`;
// zshrs runs fusevm instead (see src/fusevm_bridge.rs).
/// `vm_helper` submodule.
pub mod vm_helper;

pub use fish_features::{
    autosuggest_from_history,
    colorize_line,
    expand_abbreviation,
    // Syntax highlighting
    highlight_shell,
    // Private mode
    is_private_mode,
    // Killring
    kill_add,
    kill_replace,
    kill_yank,
    kill_yank_rotate,
    set_private_mode,
    validate_autosuggestion,
    // Validation
    validate_command,
    with_abbrs,
    with_abbrs_mut,
    AbbrPosition,
    // Abbreviations
    Abbreviation,
    AbbreviationSet,
    // Autosuggestions
    Autosuggestion,
    HighlightRole,
    HighlightSpec,
    KillRing,
    ValidationStatus,
};
pub use tokens::lextok;
pub use vm_helper::ShellExecutor;

// ── Stryke integration hook ──
// The fat binary registers a handler for @ prefix dispatch.
// The thin binary leaves this as None — @ is treated as a normal character.

use std::sync::OnceLock;

type StrykeHandler = Box<dyn Fn(&str) -> i32 + Send + Sync>;
static STRYKE_HANDLER: OnceLock<StrykeHandler> = OnceLock::new();

/// Register a handler for @ prefix lines (fat binary sets this to stryke::run).
pub fn set_stryke_handler<F>(f: F)
where
    F: Fn(&str) -> i32 + Send + Sync + 'static,
{
    let _ = STRYKE_HANDLER.set(Box::new(f));
}

/// Try to dispatch a line starting with @ to stryke.
/// Returns Some(exit_code) if handled, None if no handler registered.
pub fn try_stryke_dispatch(code: &str) -> Option<i32> {
    STRYKE_HANDLER.get().map(|f| f(code))
}

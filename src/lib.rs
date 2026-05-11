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

#![allow(dead_code)]
#![allow(unused_variables)]
#![allow(unused_imports)]
#![allow(unused_assignments)]
#![allow(unreachable_patterns)]
#![allow(deprecated)]
#![allow(unexpected_cfgs)]

pub mod extensions;
pub mod ported;

// Back-compat: re-export every ported submodule at the crate root so
// historical call sites (`crate::exec::`, `crate::subst::`,
// `crate::zle::`, `crate::modules::`, `crate::builtins::`, etc.)
// continue to resolve unchanged after the physical move into
// `src/ported/`. New code should prefer `crate::ported::<name>`.
pub use ported::*;

#[path = "extensions/aot.rs"] pub mod aot;
#[path = "extensions/arith_compiler.rs"] pub mod arith_compiler;
#[path = "extensions/autoload_cache.rs"] pub mod autoload_cache;
#[path = "extensions/script_cache.rs"] pub mod script_cache;
#[path = "extensions/compile_zsh.rs"] pub mod compile_zsh;
#[path = "extensions/completion.rs"] pub mod completion;
#[path = "extensions/bash_complete.rs"] pub mod bash_complete;
#[path = "extensions/config.rs"] pub mod config;
#[path = "extensions/canonical_apply.rs"] pub mod canonical_apply;
#[path = "extensions/overlay_snapshot.rs"] pub mod overlay_snapshot;
#[path = "extensions/daemon_presence.rs"] pub mod daemon_presence;
// Daemon lives in the `zshrs-daemon` workspace crate. Re-export it as `daemon`
// so existing `crate::daemon::...` (in exec.rs) and `zsh::daemon::...` (in bins,
// integration tests) paths keep resolving without churn.
//
// The `daemon` feature gates the actual zshrs-daemon dep. When disabled
// (--no-default-features), a stub module covers the call sites in exec.rs.
// This lets the library compile in isolation while the daemon crate is
// being refactored in a concurrent session.
#[cfg(feature = "daemon")]
pub use zshrs_daemon as daemon;

#[cfg(not(feature = "daemon"))]
pub mod daemon {
    //! Stub module used when the `daemon` feature is disabled. Provides
    //! the minimal surface that `src/exec.rs` calls — the real
    //! implementation lives in the `zshrs-daemon` workspace crate.
    pub mod builtins {
        pub const ZSHRS_BUILTIN_NAMES: &[&str] = &[];
        pub fn is_zshrs_builtin(_name: &str) -> bool {
            false
        }
        pub fn try_dispatch(_name: &str, _argv: &[String]) -> Option<i32> {
            None
        }
        pub fn dispatch(_name: &str, _args: &[String]) -> Option<i32> {
            None
        }
    }
}
#[path = "extensions/ext_builtins.rs"] pub mod ext_builtins;
#[path = "extensions/func_body_fmt.rs"] pub mod func_body_fmt;
#[path = "extensions/fds.rs"] pub mod fds;
#[path = "extensions/fish_features.rs"] pub mod fish_features;
#[path = "extensions/ast_sexp.rs"] pub mod ast_sexp;
// `tokens`, `lexer`, `parser` live in the standalone `zshrs-parse` crate
// so the daemon can use them too. Re-export so existing call sites
// (`crate::tokens::...`, `zsh::lexer::...`, etc.) keep resolving without
// touching consumers.
pub use zshrs_parse::lex;
pub use zshrs_parse::parse;
pub use zshrs_parse::tokens;
#[path = "extensions/history.rs"] pub mod history;
#[path = "extensions/log.rs"] pub mod log;
// Backwards-compat flat re-exports — call sites that still write
// `crate::datetime::…`, `crate::stat::…`, etc. resolve to the
// `crate::modules::<modname>` ports without churn. New code should
// reach for `crate::modules::<modname>` directly.
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
pub use builtins::sched;
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
#[path = "extensions/plugin_cache.rs"] pub mod plugin_cache;
#[path = "extensions/recorder.rs"] pub mod recorder_ext;
#[path = "extensions/intercepts.rs"] pub mod intercepts;
#[path = "extensions/hooks.rs"] pub mod hooks_ext;
#[path = "extensions/compinit_bg.rs"] pub mod compinit_bg;
pub mod fusevm_bridge;
pub mod exec_shims;
// Plugin-Framework-Agnostic State-Modification Recorder. Entire module
// is `#![cfg(feature = "recorder")]` so it disappears from the default
// `zshrs` build at the rustc-expansion stage. See docs/RECORDER.md.
#[cfg(feature = "recorder")]
pub mod recorder;
#[path = "extensions/regex_mod.rs"] pub mod regex_mod;
#[path = "extensions/stringsort.rs"] pub mod stringsort;
#[path = "extensions/subscript.rs"] pub mod subscript;
#[path = "extensions/worker.rs"] pub mod worker;
#[path = "extensions/zwc.rs"] pub mod zwc;
#[path = "extensions/zwc_decode.rs"] pub mod zwc_decode;
// Backwards-compat re-export so `crate::rlimits::…` keeps resolving.
pub use builtins::rlimits;

// Top-level shell executor state + fusevm bridge glue. Not a port of
// any single Src/*.c file — `Src/exec.c` is replaced by the fusevm
// bytecode VM (see src/fusevm_bridge.rs).
pub mod exec;

pub use exec::ShellExecutor;
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
pub use lex::ZshLexer;
pub use parse::ZshParser;
pub use tokens::{char_tokens, LexTok};

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

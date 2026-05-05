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

pub mod aot;
pub mod arith_compiler;
pub mod attr;
pub mod autoload_cache;
pub mod script_cache;
pub mod cap;
pub mod clone;
pub mod compat;
pub mod compile_zsh;
pub mod completion;
pub mod cond;
pub mod config;
pub mod context;
pub mod canonical_apply;
pub mod overlay_snapshot;
pub mod curses;
pub mod daemon_presence;
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
pub mod datetime;
pub mod db_gdbm;
pub mod exec;
pub mod fds;
pub mod files;
pub mod fish_features;
pub mod ast_sexp;
pub mod glob;
// `tokens`, `lexer`, `parser` live in the standalone `zshrs-parse` crate
// so the daemon can use them too. Re-export so existing call sites
// (`crate::tokens::...`, `zsh::lexer::...`, etc.) keep resolving without
// touching consumers.
pub use zshrs_parse::lexer;
pub use zshrs_parse::parser;
pub use zshrs_parse::tokens;
pub mod hashnameddir;
pub mod hashtable;
pub mod hist;
pub mod history;
pub mod hlgroup;
pub mod init;
pub mod input;
pub mod jobs;
pub mod ksh93;
pub mod langinfo;
pub mod linklist;
pub mod log;
pub mod loop_port;
pub mod mapfile;
pub mod math;
pub mod mathfunc;
pub mod mem;
pub mod modentry;
pub mod module;
pub mod nearcolor;
pub mod newuser;
pub mod options;
pub mod param_private;
pub mod parameter;
pub mod params;
pub mod pattern;
pub mod pcre;
pub mod plugin_cache;
pub mod prompt;
pub mod random;
pub mod random_real;
// Plugin-Framework-Agnostic State-Modification Recorder. Entire module
// is `#![cfg(feature = "recorder")]` so it disappears from the default
// `zshrs` build at the rustc-expansion stage. See docs/RECORDER.md.
#[cfg(feature = "recorder")]
pub mod recorder;
pub mod regex_mod;
pub mod rlimits;
pub mod sched;
pub mod signals;
pub mod socket;
pub mod sort;
pub mod stat;
pub mod string_port;
pub mod stringsort;
pub mod subscript;
// `pub mod subst` removed — the adhoc `subst.rs` was a parallel
// re-implementation of paramsubst that bypassed `subst_port.rs`
// (the actual C port). Per the "subst_port is the only paramsubst"
// directive, the adhoc file is deleted; `casemodify` / `CaseMod`
// (the only utilities other code imported from `subst::`) now live
// in `subst_port`.
pub mod subst_port;
pub mod system;
pub mod tcp;
pub mod termcap;
pub mod terminfo;
pub mod text;
pub mod utils;
pub mod watch;
pub mod worker;
pub mod zftp;
pub mod zle;
pub mod zprof;
pub mod zpty;
pub mod zselect;
pub mod zutil;
pub mod zwc;
pub mod zwc_decode;

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
pub use lexer::ZshLexer;
pub use parser::ZshParser;
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

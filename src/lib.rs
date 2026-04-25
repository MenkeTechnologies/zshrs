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

pub mod ast_opt;
pub mod attr;
pub mod cap;
pub mod compiler;
pub mod clone;
pub mod compat;
pub mod completion;
pub mod config;
pub mod cond;
pub mod context;
pub mod curses;
pub mod datetime;
pub mod db_gdbm;
pub mod exec;
pub mod fds;
pub mod files;
pub mod fish_features;
pub mod glob;
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
pub mod lexer;
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
pub mod parser;
pub mod pattern;
pub mod pcre;
pub mod plugin_cache;
pub mod prompt;
pub mod random;
pub mod random_real;
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
pub mod subst;
pub mod subst_port;
pub mod system;
pub mod tcp;
pub mod termcap;
pub mod terminfo;
pub mod text;
pub mod tokens;
pub mod utils;
pub mod watch;
pub mod zftp;
pub mod zle;
pub mod zprof;
pub mod zpty;
pub mod zselect;
pub mod shell_compiler;
pub mod worker;
pub mod zutil;
pub mod zwc;

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


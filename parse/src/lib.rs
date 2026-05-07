//! `zshrs-parse` — lexer + parser for zshrs.
//!
//! Direct port of zsh's `Src/lex.c` and `Src/parse.c`. Locked to the C
//! source via `tests/lexer_parity.rs` (byte-equal token streams against
//! `Src/Modules/zshrs_dump.c`'s `dumptokens` builtin) and
//! `tests/parity_harness.rs` (byte-equal AST sexp against `.zwc` decode).
//!
//! Shared between:
//! - `zshrs` (the runtime crate) — re-exports as `zsh::tokens`, `zsh::lexer`,
//!   `zsh::parser` so existing call sites resolve unchanged.
//! - `zshrs-daemon` — uses for static `.zshrc` analysis (catalog hydration,
//!   plugin walk, source-following). The daemon's older `zsh_lexer.rs` /
//!   `zsh_parser.rs` are partial reimplementations slated for replacement
//!   by this crate once parity is proved on the daemon's call sites.

#![allow(dead_code)]
#![allow(unused_variables)]
#![allow(unused_imports)]
#![allow(unused_assignments)]
#![allow(unreachable_patterns)]

pub mod lex;
pub mod parse;
pub mod tokens;

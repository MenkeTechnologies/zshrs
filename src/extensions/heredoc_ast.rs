//! Heredoc AST-glue types — Rust-only, NOT in zsh C.
//!
//! zsh tracks pending heredocs via the `struct heredocs` linked-list
//! node defined at `Src/zsh.h:1152-1157`:
//!
//! ```c
//! struct heredocs {
//!     struct heredocs *next;
//!     int type;
//!     int pc;
//!     char *str;
//! };
//! ```
//!
//! The C model defers body collection — the parser records `pc`
//! (wordcode offset) + `str` (terminator) at the `<<EOF` site, walks
//! past the redirection emitting normal wordcode for the rest of the
//! line, then `gethere()` (lex.c:1810) walks the linked list at
//! newline and reads each body from the input stream into the
//! wordcode buffer at the saved pc.
//!
//! zshrs's pre-wordcode parser collects each heredoc body inline
//! during lex (no pc, no later resolution), so the live shape of
//! per-heredoc state is different: `terminator`, `strip_tabs`,
//! `content`, `quoted`, `processed`. The Vec position carries
//! ordering (no `next` linked list).
//!
//! Both `HereDoc` (pending lexer state) and `HereDocInfo` (parsed
//! redir attachment) are scheduled to disappear when Phase 9 of
//! `docs/PORT_PLAN.md` lands — the wordcode port reinstates the
//! C `struct heredocs` shape + `gethere()` flow.

use serde::{Deserialize, Serialize};

/// Per-heredoc state collected by the lexer during `<<EOF` parsing.
/// Held in `ZshLexer.heredocs: Vec<HereDoc>` for later attachment to
/// `ZshRedir` entries (via `heredoc_idx`).
///
/// Rust-only — Phase 9 wordcode port replaces this with the canonical
/// `struct heredocs` linked list (zsh.h:1152) + `gethere()` deferred
/// body collection.
#[derive(Debug, Clone)]
pub struct HereDoc {
    pub terminator: String,
    pub strip_tabs: bool,
    pub content: String,
    /// True if the terminator was originally quoted (`<<'EOF'`,
    /// `<<"EOF"`, or `<<\EOF`). Disables variable expansion / command
    /// substitution / arithmetic in the body.
    pub quoted: bool,
    /// True once `process_heredocs` has read the body. Distinct from
    /// "content is empty" because an empty heredoc legitimately has
    /// empty content.
    pub processed: bool,
}

/// Heredoc body+metadata attached to a parsed `ZshRedir`. Carried
/// through the AST and consumed by the compiler when emitting
/// `Op::HereDoc(idx)` for the fusevm VM.
///
/// Rust-only — Phase 9 wordcode port deletes this together with the
/// rest of the AST tree; bodies will land directly in the wordcode
/// stream at the redirection's `pc` slot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HereDocInfo {
    pub content: String,
    pub terminator: String,
    /// Originally-quoted terminator (`<<'EOF'`, `<<"EOF"`). When true
    /// the body is passed verbatim — no `$var` / `$(cmd)` / `$((expr))`
    /// expansion. Plain `<<EOF` runs all expansions.
    #[serde(default)]
    pub quoted: bool,
}

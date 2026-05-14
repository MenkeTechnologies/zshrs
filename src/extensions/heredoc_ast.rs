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
//! `HereDoc` is the AST-glue Vec entry the AST consumer
//! (`fill_heredoc_bodies` in parse.rs) reads. The canonical
//! `struct heredocs` linked list (parse.c:84) + `gethere()`
//! (exec.c:4573) are ported as `parse::HDOCS` /
//! `crate::exec::gethere`; the inline `zshlex()` NEWLIN walk
//! (lex.c:278-306) writes body content into the next
//! unprocessed `LEX_HEREDOCS` entry directly (no helper fn).
//! `HereDocInfo` is the per-redir attachment that flows through
//! the AST.

use serde::{Deserialize, Serialize};

/// Per-heredoc state collected by the lexer during `<<EOF` parsing.
/// Held in the lexer-side `LEX_HEREDOCS: Vec<HereDoc>` thread_local
/// for later attachment to `ZshRedir` entries (via `heredoc_idx`).
///
/// Rust-only AST-glue Vec — runs parallel to the canonical
/// `struct heredocs *hdocs` linked list at `parse::HDOCS` (port of
/// `Src/parse.c:84`). The inline NEWLIN walk in `zshlex()` drains
/// both: it pops from HDOCS (the C-faithful list), calls `gethere`,
/// then walks `LEX_HEREDOCS` to find the next entry with
/// `processed == false` and writes the body there.
#[derive(Debug, Clone)]
pub struct HereDoc {
    pub terminator: String,
    pub strip_tabs: bool,
    pub content: String,
    /// True if the terminator was originally quoted (`<<'EOF'`,
    /// `<<"EOF"`, or `<<\EOF`). Disables variable expansion / command
    /// substitution / arithmetic in the body.
    pub quoted: bool,
    /// True once the inline NEWLIN walk in `zshlex()` has read the
    /// body via `gethere`. Distinct from "content is empty" because
    /// an empty heredoc legitimately has empty content.
    pub processed: bool,
}

/// Heredoc body+metadata attached to a parsed `ZshRedir`. Carried
/// through the AST and consumed by the compiler when emitting
/// `Op::HereDoc(idx)` for the fusevm VM.
///
/// Rust-only — the wordcode track stores bodies as strs-region
/// strings indexed by `WCB_REDIR` slot. The AST track keeps this
/// per-redir attachment so the fusevm compiler can emit a
/// `Op::HereDoc(idx)` referencing the body verbatim.
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

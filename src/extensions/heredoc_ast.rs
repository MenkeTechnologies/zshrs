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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heredoc_construct_and_field_access() {
        let h = HereDoc {
            terminator: "EOF".into(),
            strip_tabs: false,
            content: "hello\n".into(),
            quoted: false,
            processed: false,
        };
        assert_eq!(h.terminator, "EOF");
        assert!(!h.strip_tabs);
        assert_eq!(h.content, "hello\n");
        assert!(!h.quoted);
        assert!(!h.processed);
    }

    #[test]
    fn heredoc_clone_preserves_all_fields() {
        let h = HereDoc {
            terminator: "MARKER".into(),
            strip_tabs: true,
            content: "$x and `cmd`\n".into(),
            quoted: true,
            processed: true,
        };
        let c = h.clone();
        assert_eq!(c.terminator, h.terminator);
        assert_eq!(c.strip_tabs, h.strip_tabs);
        assert_eq!(c.content, h.content);
        assert_eq!(c.quoted, h.quoted);
        assert_eq!(c.processed, h.processed);
    }

    #[test]
    fn heredoc_info_serde_roundtrip_unquoted() {
        let info = HereDocInfo {
            content: "line1\nline2\n".into(),
            terminator: "END".into(),
            quoted: false,
        };
        let json = serde_json::to_string(&info).expect("serialize");
        let back: HereDocInfo = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.content, info.content);
        assert_eq!(back.terminator, info.terminator);
        assert_eq!(back.quoted, info.quoted);
    }

    #[test]
    fn heredoc_info_serde_roundtrip_quoted() {
        let info = HereDocInfo {
            content: "literal $var\n".into(),
            terminator: "EOF".into(),
            quoted: true,
        };
        let json = serde_json::to_string(&info).expect("serialize");
        let back: HereDocInfo = serde_json::from_str(&json).expect("deserialize");
        assert!(back.quoted);
        assert_eq!(back.content, "literal $var\n");
    }

    #[test]
    fn heredoc_info_quoted_defaults_to_false_when_missing() {
        // Older serialized payloads predate `quoted`; `#[serde(default)]`
        // must populate it as false instead of erroring.
        let json = r#"{"content":"body\n","terminator":"EOF"}"#;
        let info: HereDocInfo = serde_json::from_str(json).expect("deserialize");
        assert_eq!(info.content, "body\n");
        assert_eq!(info.terminator, "EOF");
        assert!(!info.quoted, "quoted should default to false");
    }

    #[test]
    fn heredoc_info_serializes_empty_body() {
        let info = HereDocInfo {
            content: String::new(),
            terminator: "X".into(),
            quoted: false,
        };
        let json = serde_json::to_string(&info).expect("serialize empty");
        let back: HereDocInfo = serde_json::from_str(&json).expect("deserialize empty");
        assert!(back.content.is_empty());
        assert_eq!(back.terminator, "X");
    }

    #[test]
    fn heredoc_info_preserves_special_chars_through_serde() {
        let info = HereDocInfo {
            content: "$(echo nested)\n\t`backtick`\n\"quoted\"\n".into(),
            terminator: "EOF".into(),
            quoted: false,
        };
        let json = serde_json::to_string(&info).expect("serialize");
        let back: HereDocInfo = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.content, info.content);
    }
}

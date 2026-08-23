//! !!! WARNING: RUST-ONLY MODULE — NOT A PORT OF ANY ZSH C FILE !!!
//!
//! Verbatim capture of function-body source text as the lexer consumes it.
//!
//! C zsh does not store function source at all. `par_funcdef`
//! (Src/parse.c:1672) compiles the body into the `ecbuf` wordcode array,
//! and `functions` / `typeset -f` / `whence -f` RECONSTRUCT the printed
//! text from that wordcode via `getpermtext` -> `gettext2`
//! (Src/text.c:189 / :296) — which is why zsh always prints canonical
//! tab-indented text regardless of how the user spaced the original.
//!
//! zshrs runs function bodies through the fusevm compile path, which never
//! builds a C-shaped `Eprog`; `shfunc.funcdef` (zsh_h.rs:868) is therefore
//! always `None` and `printshfuncnode` (hashtable.rs:1796) prints the RAW
//! SOURCE the parser captured instead. Until the fusevm path grows a real
//! `Eprog`, that raw source has to come from somewhere for EVERY input
//! source.
//!
//! It used to come only from a slice of the Rust-only `LEX_INPUT` window
//! (`lex::input_slice`), which exists for file / `-c` / `eval` input.
//! Interactive and `-s` stdin input reaches the lexer through
//! `hgetc` -> `ingetc` -> the `inbuf` stack and never touches `LEX_INPUT`,
//! so `LEX_POS` never moved and every function defined at a prompt or piped
//! in on stdin printed as `name () { }`.
//!
//! C's own history-line buffer (`chline`) is NOT a usable substitute: at
//! c:Src/hist.c:1119 `hbegin` sets
//! `stophist = (!interact || unset(SHINSTDIN)) ? 2 : 0` and then c:1129
//! `chline = hptr = NULL`, so for non-interactive stdin there is no history
//! line at all. The one funnel every input source does pass through is
//! `hgetc`, so this module hangs an echo buffer off it, active only between
//! [`body_mark_begin`] and [`body_text`].
//!
//! Lives outside `src/ported/` because it has no C counterpart (same
//! reasoning as `crate::tolerant_sort`).

use crate::ported::lex::{input_slice, pos};

thread_local! {
/// Verbatim echo of the characters the lexer has consumed while at least
/// one function-body capture is open. Fed by `hgetc` / `hungetc`
/// (src/ported/lex.rs); see the module doc above for why it exists and why
/// C's `chline` cannot stand in for it. Active only between
/// [`body_mark_begin`] and [`body_text`].
pub static LEX_SRC_CAPTURE: std::cell::RefCell<String>
    = const { std::cell::RefCell::new(String::new()) };
/// !!! WARNING: RUST-ONLY HELPER STATE !!!
///
/// Number of function-body captures currently open (nested `f() { g()
/// { ... } }` opens two). Zero means `hgetc` records nothing.
pub static LEX_SRC_CAPTURE_DEPTH: std::cell::Cell<usize>
    = const { std::cell::Cell::new(0) };
/// !!! WARNING: RUST-ONLY HELPER STATE !!!
///
/// Lockstep companion to `lex::LEX_UNGET_BUF`, exactly like
/// `lex::LEX_UNGET_HPTR`: one flag per queued character recording whether
/// `hungetc` removed it from [`LEX_SRC_CAPTURE`], so the matching
/// re-read in `hgetc` puts back exactly what was taken. Without it a
/// capture that opens or closes between the unget and the re-read
/// gains or loses one character.
pub static LEX_UNGET_SRCCAP: std::cell::RefCell<std::collections::VecDeque<bool>>
    = const { std::cell::RefCell::new(std::collections::VecDeque::new()) };
}

/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// A parser-side bookmark for "where the text of a function body starts".
/// C has no analogue — `par_funcdef` (Src/parse.c:1672) writes the body
/// into the `ecbuf` wordcode array and `functions` re-derives the printed
/// text from it (`getpermtext`, Src/text.c:189). zshrs's fusevm path never
/// builds that wordcode `Eprog`, so the parser must keep the raw source.
///
/// Carries BOTH cursors because zshrs has two input sources: the Rust-only
/// `LEX_INPUT` window (file / `-c` / `eval` text) and the ported `inbuf`
/// stack that `hgetc` reads for interactive and `-s` stdin input. See
/// [`LEX_SRC_CAPTURE`] for why the second one needs an echo buffer.
#[derive(Debug)]
pub struct BodyMark {
    /// `LEX_POS` at the time the mark was taken.
    lex_pos: usize,
    /// Offset into [`LEX_SRC_CAPTURE`] at the time the mark was taken.
    cap_pos: usize,
    /// Set by [`body_text`]; makes the `Drop` guard a no-op.
    closed: bool,
}

impl Drop for BodyMark {
    /// !!! RUST-ONLY !!! — a `YYERROR` return out of `par_funcdef`
    /// (c:Src/parse.c:1735 and friends) abandons the mark without asking
    /// for its text. Close the capture anyway so a syntax error in one
    /// body cannot leave the echo buffer recording into the next command.
    fn drop(&mut self) {
        if !self.closed {
            close_capture();
        }
    }
}

/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// Open a function-body capture and return the mark to close it with.
/// Every character `hgetc` returns from now until the matching
/// [`body_text`] is echoed into [`LEX_SRC_CAPTURE`]. See that static for
/// why this exists.
pub fn body_mark_begin() -> BodyMark {
    let cap_pos = LEX_SRC_CAPTURE.with_borrow(|b| b.len());
    LEX_SRC_CAPTURE_DEPTH.set(LEX_SRC_CAPTURE_DEPTH.get() + 1);
    BodyMark {
        lex_pos: pos(),
        cap_pos,
        closed: false,
    }
}

/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// Move an already-open mark forward to the current input position,
/// discarding what has been captured so far. The funcdef parsers take the
/// mark before `()` and then re-take it past every separator (`pos()` after
/// a `zshlex` is already one token ahead, so the mark has to be re-read
/// BEFORE each advance); this is that re-read for the capture side.
pub fn body_mark_restart(mark: &mut BodyMark) {
    mark.lex_pos = pos();
    mark.cap_pos = LEX_SRC_CAPTURE.with_borrow(|b| b.len());
}

/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// Close the capture opened by [`body_mark_begin`] and return the raw body
/// text, untrimmed. `LEX_INPUT` wins when it actually advanced (the file /
/// `-c` / `eval` paths, whose slice behaviour is unchanged by this
/// helper); the `hgetc` echo buffer is the fallback for the interactive
/// and `-s` stdin paths, where `LEX_POS` never moves.
pub fn body_text(mut mark: BodyMark) -> Option<String> {
    let end = pos();
    mark.closed = true;
    let captured = LEX_SRC_CAPTURE.with_borrow(|b| {
        // `hungetc` can pop a character that predates the mark, so clamp,
        // and land on a char boundary (a partially-rewound multibyte char
        // would otherwise panic the slice).
        let mut start = mark.cap_pos.min(b.len());
        while start > 0 && !b.is_char_boundary(start) {
            start -= 1;
        }
        b[start..].to_string()
    });
    close_capture();
    if end > mark.lex_pos {
        if let Some(s) = input_slice(mark.lex_pos, end) {
            return Some(s);
        }
    }
    Some(captured).filter(|s| !s.is_empty())
}

/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// Drop one level of capture nesting; returns the remaining depth.
fn close_capture() -> usize {
    let depth = LEX_SRC_CAPTURE_DEPTH.get().saturating_sub(1);
    LEX_SRC_CAPTURE_DEPTH.set(depth);
    if depth == 0 {
        LEX_SRC_CAPTURE.with_borrow_mut(|b| b.clear());
    }
    depth
}

/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// Drop any capture left open by a parse that bailed out, so a syntax error
/// in one function body cannot leak text into the next one. Called from
/// `lex_init` (src/ported/lex.rs) alongside the other lexer-state resets;
/// the `BodyMark` `Drop` guard covers the ordinary error returns.
pub fn src_capture_reset() {
    LEX_SRC_CAPTURE_DEPTH.set(0);
    LEX_SRC_CAPTURE.with_borrow_mut(|b| b.clear());
    LEX_UNGET_SRCCAP.with_borrow_mut(|b| b.clear());
}

/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// Echo one consumed character into [`LEX_SRC_CAPTURE`]. Returns whether it
/// was recorded, so `hungetc`'s pop and `hgetc`'s re-read stay symmetric.
/// Called from `hgetc` (src/ported/lex.rs) only.
pub fn src_capture_add(c: char) -> bool {
    if LEX_SRC_CAPTURE_DEPTH.get() == 0 {
        return false;
    }
    LEX_SRC_CAPTURE.with_borrow_mut(|b| b.push(c));
    true
}

/// !!! WARNING: RUST-ONLY HELPER !!!
///
/// Undo the last [`src_capture_add`], but only if the buffer really ends
/// with `c` — a character the `counts_lineno` gate refused was never
/// recorded and must not be taken from an earlier one. Returns whether
/// anything was removed. Called from `hungetc` (src/ported/lex.rs) only.
pub fn src_capture_back(c: char) -> bool {
    if LEX_SRC_CAPTURE_DEPTH.get() == 0 {
        return false;
    }
    LEX_SRC_CAPTURE.with_borrow_mut(|b| {
        if b.ends_with(c) {
            b.pop();
            true
        } else {
            false
        }
    })
}

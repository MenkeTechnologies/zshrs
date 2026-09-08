//! Completion on a PS2 CONTINUATION line sees the lines already entered.
//!
//! `docomplete` (Src/Zle/zle_tricky.c:640-653) splices the accumulated
//! history text — `chline`, or the copy `hist_context_save` published as
//! `zle_chline` at hist.c:252 — in front of the line the editor holds,
//! runs the lexer over the pair, then subtracts the prefix back out at
//! c:677-696. The subtraction has a guard: when `wb` lands INSIDE the
//! prefix (`wb < 0` at c:681) the cursor word began on an earlier line, so
//! the buffer is restored and `docomplete` returns 1 — `callcompfunc` is
//! never reached and not one match is offered.
//!
//! Without the prepend the lexer sees only the physical line, which parses
//! as a fresh command:
//!
//!     print 'aaa
//!     <fixture>/uniqued<TAB>      →  <fixture>/uniquedir/
//!
//! zsh completes nothing there, because the word is inside a single quote
//! opened on the line above. The two cases below pin both halves — the
//! continuation declines, and the SAME word inside the SAME quote on ONE
//! line still completes — so a shell that simply stopped completing after
//! a quote cannot pass.
//!
//! Skip pattern: no-ops silently when `zsh` isn't on PATH or when
//! `zsh/zpty` will not load. Harness contract: `zpty_probe`.

#![allow(non_snake_case)]
#![allow(clippy::doc_lazy_continuation)]

use crate::zpty_probe::{assert_same_dump, dump_widget, DRAIN, DUMP_KEY, OPEN};
use std::path::PathBuf;

/// A directory with a name nothing else in its parent shares a prefix
/// with, so TAB has exactly one candidate. Built under the cargo target
/// dir, and addressed ABSOLUTELY, so the completion does not depend on the
/// inner shell's working directory or on anything installed on the host.
fn fixture_dir() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("parity-chline-fixture");
    let _ = std::fs::create_dir_all(dir.join("uniquedir"));
    dir
}

/// The word typed before TAB: an unambiguous prefix of the fixture
/// directory, one character short of its name.
fn word() -> String {
    format!("{}/uniqued", fixture_dir().display())
}

/// TAB inside a single quote that was opened on the PREVIOUS line offers
/// nothing, so the buffer is byte-for-byte what was typed.
///
/// `$PREBUFFER` is dumped alongside `$BUFFER` on purpose: it is the proof
/// that the shell really is on a continuation line. A shell that executed
/// the first line instead of continuing it reports an empty `$PREBUFFER`
/// and the case would otherwise be measuring a plain first-line
/// completion.
#[test]
fn tab_inside_a_quote_opened_on_the_previous_line_completes_nothing() {
    let w = word();
    let driver = format!(
        "{OPEN}
zpty -w w 'unsetopt beep'
{}
zpty -w -n w $'print \\'aaa\\r'
sleep 2
zpty -w -n w '{w}'
sleep 1
zpty -w -n w $'\\t'
sleep 3
{DUMP_KEY}
{DRAIN}
",
        dump_widget("\"BUF=[$BUFFER] PRE=[$PREBUFFER]\"")
    );
    assert_same_dump(
        &driver,
        "TAB on a continuation line, inside a quote opened above, left the buffer alone",
    );
}

/// The control that makes the case above a statement about the
/// CONTINUATION rather than about the quote: the same word, inside the
/// same unterminated quote, with the newline removed. Here the quote opens
/// and the word sits on one line, `wb` stays inside the physical line, and
/// both shells complete it.
#[test]
fn tab_inside_a_quote_opened_on_the_same_line_still_completes() {
    let w = word();
    let driver = format!(
        "{OPEN}
zpty -w w 'unsetopt beep'
{}
zpty -w -n w \"print 'aaa {w}\"
sleep 1
zpty -w -n w $'\\t'
sleep 3
{DUMP_KEY}
{DRAIN}
",
        dump_widget("\"BUF=[$BUFFER] PRE=[$PREBUFFER]\"")
    );
    assert_same_dump(
        &driver,
        "TAB inside a quote opened on the same line completed the word",
    );
}

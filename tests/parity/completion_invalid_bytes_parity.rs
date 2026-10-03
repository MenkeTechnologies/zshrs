//! Completion matches carrying bytes the locale cannot decode.
//!
//! `add_match_data()` (Src/Zle/compcore.c:2870-2920, zsh-5.9.1) walks every
//! match string through `mbrtowc` and rewrites each byte that does not
//! convert into a `$'\NNN'` sequence before the match is stored. That stored
//! form is what the listing prints and what `do_single` inserts, so under a
//! UTF-8 locale
//!
//!     compadd -U -Q -- $'a\x9bb'
//!
//! puts the six characters `a$'\233'b` on the line — text that, when the line
//! runs, evaluates back to the original three bytes. A shell that skips the
//! rewrite inserts the raw byte instead, and zshrs' ZLE line then garbled it.
//!
//! Skip pattern: no-ops silently when `zsh` isn't on PATH or when
//! `zsh/zpty` will not load. Harness contract: `zpty_probe`.

#![allow(non_snake_case)]
#![allow(clippy::doc_lazy_continuation)]

use crate::zpty_probe::{assert_same_dump, sq, DUMP_KEY, DUMP_WIDGET, OPEN_PUMPED};

/// Bind `^Xw` to a `zle -C` completion widget whose function adds `matches`
/// (a shell word list) with `compadd -U -Q`, type `print `, fire it, then
/// dump the buffer through `(V)` so the dump file is plain ASCII on both
/// sides whatever bytes the line holds.
fn invalid_byte_driver(matches: &str) -> String {
    let setup = sq(&format!(
        "_mbw() {{ compadd -U -Q -- {matches} }}; zle -C mbw complete-word _mbw; bindkey \"^Xw\" mbw"
    ));
    format!(
        "{OPEN_PUMPED}
zpty -w w {setup}; pump
{DUMP_WIDGET}
pump
zpty -w -n w 'print '; pump
zpty -w -n w $'\\C-xw'; pump; pump
{DUMP_KEY}
pump
zpty -d w 2>/dev/null
",
        DUMP_WIDGET = DUMP_WIDGET.replace(
            r#"print -r -- "BUF=[$BUFFER] CUR=[$CURSOR]""#,
            r#"print -r -- "BUF=[${(V)BUFFER}] CUR=[$CURSOR]""#
        ),
    )
}

/// A lone C1 byte (0x9b) is MB_INVALID under UTF-8: the inserted text is
/// `a$'\233'b`, cursor after it.
#[test]
fn an_undecodable_byte_in_a_match_is_inserted_as_dollar_quote() {
    assert_same_dump(
        &invalid_byte_driver(r#"$'a\x9bb'"#),
        "compadd -U -Q of a match holding an invalid byte",
    );
}

/// A truncated multibyte sequence at the END of the match is MB_INCOMPLETE
/// with nothing left to complete it: c:2896-2906 emits every remaining byte,
/// one `$'\NNN'` each.
#[test]
fn a_truncated_sequence_at_the_end_of_a_match_is_dollar_quoted_bytewise() {
    assert_same_dump(
        &invalid_byte_driver(r#"$'x\xe6\x97'"#),
        "compadd -U -Q of a match ending in an incomplete sequence",
    );
}

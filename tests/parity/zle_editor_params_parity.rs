//! ZLE editor-parameter parity — the variables a widget reads to find
//! out where it is, dumped from inside a widget and compared exactly.
//!
//! `zle_buffer_state_parity` pins `$BUFFER` and `$CURSOR`. These are
//! the rest of the surface a plugin widget actually uses:
//!
//!   `$KEYMAP`      which keymap is live — how a vi-mode plugin knows
//!                  whether to draw an INSERT or NORMAL indicator
//!   `$LBUFFER` /   the line split at the cursor, which is how
//!   `$RBUFFER`     autosuggestions decides what to suggest and where
//!                  to paint it
//!   `$PREBUFFER`   the lines already accepted in a CONTINUATION — the
//!                  only place the earlier lines of a multi-line
//!                  command exist while it is still being typed
//!   `$LASTWIDGET`  the previously run widget, which is how
//!                  autosuggestions tells "the user just moved" from
//!                  "the user just typed"
//!   `$NUMERIC`     the numeric prefix, so `ESC-3 ^F` moves three
//!
//! Each is read at a moment where a wrong answer is a specific,
//! nameable bug rather than a cosmetic difference.
//!
//! Skip pattern: no-ops silently when `zsh` isn't on PATH or when
//! `zsh/zpty` will not load. Harness contract: `zpty_probe`.

#![allow(non_snake_case)]
#![allow(clippy::doc_lazy_continuation)]

use crate::zpty_probe::{assert_same_dump, dump_widget, sq, DUMP_KEY, OPEN};

/// Type `keys` under `keymap`, then dump `expr`.
///
/// Same two-second settle per keystroke as the buffer-state module, and
/// the same mandatory drain before the file is read — see `zpty_probe`
/// for why both are load-bearing rather than cautious.
fn driver(keymap: &str, expr: &str, keys: &[&str]) -> String {
    let widget = dump_widget(expr);
    let typed: String = keys
        .iter()
        .map(|k| {
            let w = if let Some(esc) = k.strip_prefix('\\') {
                format!("$'\\{esc}'")
            } else {
                sq(k)
            };
            format!("zpty -w -n w {w}\nsleep 2\n")
        })
        .collect();
    format!(
        "{OPEN}
zpty -w w 'unsetopt beep'
zpty -w w 'PS2=\"C2> \"'
zpty -w w 'bindkey {keymap}'
{widget}
sleep 2
{typed}{DUMP_KEY}
local out all=
integer i=0
while (( i++ < 50 )); do
  if zpty -r -t w out 2>/dev/null; then all+=\"$out\"; else sleep 0.1; fi
done
zpty -d w 2>/dev/null
"
    )
}

const KEYMAP: &str = r#""KM=[$KEYMAP]""#;
const SPLIT: &str = r#""L=[$LBUFFER] R=[$RBUFFER]""#;
const PREBUF: &str = r#""PRE=[$PREBUFFER] BUF=[$BUFFER]""#;
const WIDGETS: &str = r#""LAST=[$LASTWIDGET] W=[$WIDGET]""#;
const NUMERIC: &str = r#""NUM=[$NUMERIC] BUF=[$BUFFER] CUR=[$CURSOR]""#;

// ═══════════════════════════════════════════════════════════════════════
// $KEYMAP
// ═══════════════════════════════════════════════════════════════════════

/// Under `bindkey -e` the live keymap is `main`.
#[test]
fn keymap_is_main_under_emacs_bindings() {
    assert_same_dump(
        &driver("-e", KEYMAP, &["abc"]),
        "$KEYMAP reported main under emacs bindings",
    );
}

/// Under `bindkey -v` the INSERT keymap is still reported as `main` —
/// `viins` is what it is aliased FROM, not what `$KEYMAP` says. A vi
/// plugin that tests for the literal string `viins` here is the classic
/// way to get a mode indicator stuck.
#[test]
fn keymap_is_main_in_vi_insert_mode() {
    assert_same_dump(
        &driver("-v", KEYMAP, &["abc"]),
        "$KEYMAP reported main in vi insert mode",
    );
}

/// ESC switches to command mode and `$KEYMAP` becomes `vicmd` — the
/// transition every mode indicator hangs off.
#[test]
fn keymap_becomes_vicmd_after_escape() {
    assert_same_dump(
        &driver("-v", KEYMAP, &["abc", "\\e"]),
        "$KEYMAP became vicmd after ESC",
    );
}

// ═══════════════════════════════════════════════════════════════════════
// $LBUFFER / $RBUFFER
// ═══════════════════════════════════════════════════════════════════════

/// The two halves split AT the cursor and must reassemble into the
/// whole line: one character consumed by `^A ^F` on the left, the rest
/// on the right.
#[test]
fn lbuffer_and_rbuffer_split_at_the_cursor() {
    assert_same_dump(
        &driver("-e", SPLIT, &["abcd", "\\C-a", "\\C-f"]),
        "$LBUFFER/$RBUFFER split the line at the cursor",
    );
}

// ═══════════════════════════════════════════════════════════════════════
// $PREBUFFER — continuation lines
// ═══════════════════════════════════════════════════════════════════════

/// A trailing backslash continues the line: the accepted first line
/// moves into `$PREBUFFER` (newline included) and `$BUFFER` starts
/// empty for the continuation. A shell that loses `$PREBUFFER` has no
/// record of the earlier lines while the command is still being typed.
#[test]
fn prebuffer_holds_the_line_continued_by_a_backslash() {
    assert_same_dump(
        &driver("-e", PREBUF, &["print a \\", "\\r", "b"]),
        "$PREBUFFER held the backslash-continued line",
    );
}

/// An unclosed quote continues the same way, and the quote character
/// has to survive into `$PREBUFFER` intact.
#[test]
fn prebuffer_holds_the_line_continued_by_an_open_quote() {
    assert_same_dump(
        &driver("-e", PREBUF, &["print \"a", "\\r", "b"]),
        "$PREBUFFER held the quote-continued line",
    );
}

// ═══════════════════════════════════════════════════════════════════════
// $LASTWIDGET and $NUMERIC
// ═══════════════════════════════════════════════════════════════════════

/// `$LASTWIDGET` names the widget that ran BEFORE this one, and
/// `$WIDGET` names this one. Autosuggestions reads exactly this pair to
/// tell "the user moved the cursor" from "the user typed a character".
#[test]
fn lastwidget_names_the_previous_widget() {
    assert_same_dump(
        &driver("-e", WIDGETS, &["abcd", "\\C-a"]),
        "$LASTWIDGET named the previously run widget",
    );
}

/// `ESC-3` sets the numeric prefix without touching the line.
#[test]
fn a_numeric_prefix_is_visible_to_the_next_widget() {
    assert_same_dump(
        &driver("-e", NUMERIC, &["abcd", "\\e3"]),
        "$NUMERIC carried the prefix to the next widget",
    );
}

/// …and it is APPLIED: `ESC-3 ^F` moves three characters, not one.
#[test]
fn a_numeric_prefix_repeats_the_next_widget() {
    assert_same_dump(
        &driver("-e", NUMERIC, &["abcdef", "\\C-a", "\\e3", "\\C-f"]),
        "ESC-3 ^F moved three characters",
    );
}

/// The vi form of the same idea: a count before an operator. `3x`
/// deletes three characters.
#[test]
fn a_vi_count_repeats_the_operator() {
    assert_same_dump(
        &driver("-v", NUMERIC, &["abcdef", "\\e", "0", "3x"]),
        "vi 3x deleted three characters",
    );
}

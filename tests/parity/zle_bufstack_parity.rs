//! ZLE buffer-stack parity — the line a widget stashes must come BACK.
//!
//! `push-line`, `push-line-or-edit`, `push-input`, `accept-and-hold`,
//! `run-help`/`which-command` (via `processcmd`) and `print -z` all do
//! the same thing: hand the current line to the editor's buffer stack
//! and return. None of them restores anything themselves. The restore
//! is one block at the head of the NEXT `zleread`
//! (`Src/Zle/zle_main.c:1297-1312`), which pops one entry, `setline`s
//! it, and puts the cursor back at the saved column.
//!
//! zshrs pushed and never popped, so every one of those widgets
//! DELETED the line instead of parking it — `ESC-q` then Enter left the
//! text gone, and `M-h` ran the help command and came back to an empty
//! prompt. That is invisible to a test that only checks what ran: the
//! pushing keystroke and the accept both "work", and the loss shows up
//! one prompt later.
//!
//! So every case here measures the line at the prompt AFTER the one
//! that pushed, through `$BUFFER`/`$CURSOR` read inside a widget. The
//! cursor is part of the measurement on purpose: `stackcs` is a
//! separate slot from the text (c:1300-1305) and restoring the text
//! with the cursor at the wrong column is its own bug.
//!
//! Bindings are explicit (`^X^P` etc.) rather than the emacs defaults:
//! `push-line`'s stock `^Q` is tty flow control and never reaches ZLE
//! at all, which would make both shells report nothing and pass as
//! false agreement.
//!
//! Skip pattern: no-ops silently when `zsh` isn't on PATH or when
//! `zsh/zpty` will not load. Harness contract: `zpty_probe`.

#![allow(non_snake_case)]
#![allow(clippy::doc_lazy_continuation)]

use crate::zpty_probe::{assert_same_dump, dump_widget, OPEN};

/// Install `setup`, type `keys`, then dump `$BUFFER`/`$CURSOR` with
/// `^X^G` — from the prompt that comes AFTER whatever `keys` did.
///
/// The pty is drained before the file is read for the reason spelled
/// out in `zle_buffer_state_parity`: an inner shell whose output buffer
/// has filled blocks on write, and the dump widget then never runs.
///
/// Two seconds per keystroke. These sequences cross a prompt boundary,
/// so a key written while the inner shell is still redrawing the NEW
/// prompt is dropped, and the dump measures the wrong prompt.
fn driver(setup: &str, keys: &str) -> String {
    format!(
        "{OPEN}
zpty -w w 'unsetopt beep'
zpty -w w '{setup}'
{}
sleep 2
{keys}
sleep 2
zpty -w -n w $'\\C-x\\C-g'
sleep 2
local out
integer i=0
while (( i++ < 60 )); do
  if zpty -r -t w out 2>/dev/null; then :; else sleep 0.1; fi
done
zpty -d w 2>/dev/null
",
        dump_widget(r#""BUF=[$BUFFER] CUR=[$CURSOR]""#)
    )
}

/// The core case. `push-line` stashes `print first` with the cursor
/// parked two columns from the end, the now-empty line is accepted, and
/// the next prompt has to come up holding the same text with the cursor
/// back at column 9 — not at the end, and not at 0.
///
/// Before the `zleread` pop block existed, zshrs came back to
/// `BUF=[] CUR=[0]` here: the text was on the stack and nothing ever
/// took it off.
#[test]
fn push_line_restores_the_line_and_the_cursor_at_the_next_prompt() {
    assert_same_dump(
        &driver(
            r#"bindkey "^X^P" push-line"#,
            r#"zpty -w -n w 'print first'
sleep 2
zpty -w -n w $'\e[D\e[D'
sleep 2
zpty -w -n w $'\C-x\C-p'
sleep 2
zpty -w -n w $'\r'"#,
        ),
        "push-line restored the stashed line and its cursor column",
    );
}

/// Two pushes, two pops, and the ORDER is the measurement.
///
/// `zpushnode` inserts at the list head (`zsh.h:591`) and `getlinknode`
/// takes the head back off, so the stack is LIFO: the SECOND line
/// pushed is the first one handed back. A port that appends instead of
/// prepending gets both lines back but in the wrong order, which one
/// push can never detect.
#[test]
fn two_stacked_push_lines_come_back_newest_first() {
    assert_same_dump(
        &driver(
            r#"bindkey "^X^P" push-line"#,
            r#"zpty -w -n w 'print one'
sleep 2
zpty -w -n w $'\C-x\C-p'
sleep 2
zpty -w -n w 'print two'
sleep 2
zpty -w -n w $'\C-x\C-p'
sleep 2
zpty -w -n w $'\r'"#,
        ),
        "the newest of two stacked lines is restored first",
    );
}

/// `run-help` (`processcmd`, `Src/Zle/zle_tricky.c:2986`) replaces the
/// line with `run-help <cmdword>`, accepts it, and relies on the same
/// pop to bring the original text back one prompt later. It exercises
/// the block through a DIFFERENT caller than `push-line`: the widget
/// calls `pushline` itself and supplies its own `done = 1` (c:3007).
///
/// `run-help` is shadowed by a no-op function so the probe cannot hang
/// in a pager or depend on what documentation the box has installed.
#[test]
fn run_help_restores_the_line_it_replaced() {
    assert_same_dump(
        &driver(
            r#"run-help(){ : }; bindkey "^X^H" run-help"#,
            r#"zpty -w -n w 'echoo test'
sleep 2
zpty -w -n w $'\C-x\C-h'"#,
        ),
        "run-help gave back the line it replaced",
    );
}

/// `print -z` is the non-widget door onto the same stack
/// (`Src/builtin.c:5026`), so it pins the pop independently of ZLE's
/// own widgets: nothing is typed at the editor at all, yet the next
/// prompt has to come up pre-loaded with the queued text.
#[test]
fn print_z_text_arrives_at_the_next_prompt() {
    assert_same_dump(
        &driver(
            r#"true"#,
            r#"zpty -w -n w 'print -z print stashed'
sleep 2
zpty -w -n w $'\r'"#,
        ),
        "print -z queued a line onto the next prompt",
    );
}

/// `accept-and-hold` runs the line AND stashes a copy, so the same text
/// is waiting at the next prompt. It is the one caller that sets
/// `stackcs` from a line it is simultaneously executing (c:411-412), so
/// it pins that the saved column survives the accept.
#[test]
fn accept_and_hold_leaves_a_copy_at_the_next_prompt() {
    assert_same_dump(
        &driver(
            r#"bindkey "^X^A" accept-and-hold"#,
            r#"zpty -w -n w 'print held'
sleep 2
zpty -w -n w $'\C-x\C-a'"#,
        ),
        "accept-and-hold left a copy of the line for the next prompt",
    );
}

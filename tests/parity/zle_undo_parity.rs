//! ZLE undo bookkeeping as a widget sees it: `$UNDO_CHANGE_NO` and
//! `zle undo <change-number>`.
//!
//! Widgets that rewrite the line and then want it back record
//! `$UNDO_CHANGE_NO` on entry and finish with `zle undo $saved`
//! (Src/Zle/zle_utils.c:1604-1633). `bracketed-paste-magic` is the stock
//! user: it replays the paste as keystrokes into a cleared line and then
//! undoes the replay before inserting the paste for real.
//!
//! Each case gets its own pty session. Harness contract: `zpty_probe`.

use crate::zpty_probe::{assert_same_verdict, DRAIN, OPEN};

/// One session: silence the bell, install `setup`, run `keys`, then
/// report whether `needle` reached the screen.
fn driver(setup: &str, keys: &str, needle: &str) -> String {
    format!(
        "{OPEN}
zpty -w w 'unsetopt beep'
zpty -w w '{setup}'
sleep 2
{keys}
sleep 2
{DRAIN}
if [[ $all == *'{needle}'* ]]; then print \"K=yes\"; else print \"K=no\"; fi
"
    )
}

/// `zle undo N` first commits the edit in progress (c:1617 `handleundo()`)
/// and then walks back past every change numbered above N, so a widget can
/// take back what it did to the line itself. zshrs never recorded the
/// widget's own edit, so there was nothing to take back and the rewritten
/// line ran.
#[test]
fn undo_to_the_entry_change_number_restores_the_line() {
    assert_same_verdict(
        &driver(
            r#"uwq(){ integer n=$UNDO_CHANGE_NO; BUFFER=": "; zle undo $n; LBUFFER+="UN\${:-}DONE" }; zle -N uwq; bindkey "^G" uwq"#,
            r#"zpty -w -n w 'print '
sleep 1
zpty -w -n w $'\C-g'
sleep 2
zpty -w -n w $'\r'"#,
            "UNDONE",
        ),
        "K",
        "`zle undo $saved` took back the widget's own edit",
    );
}

/// Every line starts a new change list with `undo_changeno = 0`
/// (c:Src/Zle/zle_main.c:1295 `initundo()`, c:Src/Zle/zle_utils.c:1460).
/// zshrs never called `initundo`, so the counter ran on across lines.
#[test]
fn a_fresh_line_starts_at_change_zero() {
    assert_same_verdict(
        &driver(
            r#"cnq(){ BUFFER="print CN\${:-}=$UNDO_CHANGE_NO" }; zle -N cnq; bindkey "^G" cnq"#,
            r#"zpty -w -n w $'print one two three\r'
sleep 2
zpty -w -n w $'\C-g'
sleep 2
zpty -w -n w $'\r'"#,
            "CN=0",
        ),
        "K",
        "`$UNDO_CHANGE_NO` read 0 on a fresh line",
    );
}

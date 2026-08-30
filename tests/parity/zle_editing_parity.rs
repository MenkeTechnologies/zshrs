//! ZLE line-editing parity — emacs and vi keymaps, driven through a
//! real pty and judged by WHAT THE SHELL RAN, not by what the screen
//! looked like.
//!
//! Every case builds a command line with keystrokes, presses Return,
//! and checks whether the command that executed printed its marker. If
//! a movement, kill, yank or repeat lands in the wrong place, the
//! buffer is a different command and the marker never appears. That
//! makes the assertion immune to cursor-position and redraw
//! differences, which are exactly the things two shells are allowed to
//! disagree about byte-for-byte.
//!
//! The marker is assembled by the INNER shell at run time
//! (`print OUT${:-}M1` → `OUTM1`), so the terminal's echo of the typed
//! characters can never satisfy the match — without that, a shell whose
//! Return key did nothing at all would still look green.
//!
//! These lock in behaviour that until now was only ever checked with
//! throwaway PTY scripts: emacs kill/yank and word-kill, and the vi
//! `dd` / `x` / `.` / `yy`+`p` set.
//!
//! Skip pattern: no-ops silently when `zsh` isn't on PATH or when
//! `zsh/zpty` will not load. Harness contract: `zpty_probe`.

#![allow(non_snake_case)]
#![allow(clippy::doc_lazy_continuation)]

use crate::zpty_probe::{assert_same_verdict, sq, DRAIN, OPEN};

/// Build a driver that runs `keys` (a block of `zpty -w -n w …` lines)
/// under `keymap`, then reports whether `needle` reached the output.
///
/// `needle` must be text that CANNOT appear in the echo of the
/// keystrokes — either a marker the inner shell assembles at run time
/// (`OUTM${:-}1` → `OUTM1`), or a string that only exists once an
/// editing operation has rearranged the line (`Xzzz` from typing `zzz`
/// and inserting an `X` in front of it).
fn driver(keymap: &str, keys: &str, needle: &str) -> String {
    let needle_q = sq(needle);
    format!(
        "{OPEN}
zpty -w w 'bindkey {keymap}'
sleep 1
{keys}
sleep 2
{DRAIN}
local needle={needle_q}
if [[ $all == *${{~needle}}* ]]; then print \"K=yes\"; else print \"K=no\"; fi
"
    )
}

// ═══════════════════════════════════════════════════════════════════════
// emacs keymap
// ═══════════════════════════════════════════════════════════════════════

mod emacs {
    use super::*;

    /// `^A` (beginning-of-line) then an insert: the line is typed
    /// MISSING its first character, and only a correct
    /// beginning-of-line puts the `p` where `print` needs it.
    #[test]
    fn ctrl_a_moves_to_the_start_before_inserting() {
        assert_same_verdict(
            &driver(
                "-e",
                r#"zpty -w -n w 'rint OUT${:-}M1'
zpty -w -n w $'\C-a'
zpty -w -n w 'p'
zpty -w -n w $'\r'"#,
                "OUTM1",
            ),
            "K",
            "^A moved to the start of the line",
        );
    }

    /// `^U` (kill-whole-line) has to leave an EMPTY buffer — a partial
    /// kill leaves `junkjunk` glued to the retyped command.
    #[test]
    fn ctrl_u_kills_the_whole_line() {
        assert_same_verdict(
            &driver(
                "-e",
                r#"zpty -w -n w 'junkjunk'
zpty -w -n w $'\C-u'
zpty -w -n w 'print OUT${:-}M2'
zpty -w -n w $'\r'"#,
                "OUTM2",
            ),
            "K",
            "^U killed the whole line",
        );
    }

    /// `^A ^K ^Y` — kill the line into the cut buffer and yank it back.
    /// The round trip has to reproduce the command EXACTLY; a cut
    /// buffer that drops or duplicates a character changes what runs.
    #[test]
    fn ctrl_k_then_ctrl_y_round_trips_the_line() {
        assert_same_verdict(
            &driver(
                "-e",
                r#"zpty -w -n w 'print OUT${:-}M3'
zpty -w -n w $'\C-a'
zpty -w -n w $'\C-k'
zpty -w -n w $'\C-y'
zpty -w -n w $'\r'"#,
                "OUTM3",
            ),
            "K",
            "^K then ^Y round-tripped the line",
        );
    }

    /// `^W` (backward-kill-word) must take the trailing word and
    /// nothing more — one character too many eats into the marker.
    #[test]
    fn ctrl_w_kills_exactly_the_last_word() {
        assert_same_verdict(
            &driver(
                "-e",
                r#"zpty -w -n w 'print OUT${:-}M4 junkword'
zpty -w -n w $'\C-w'
zpty -w -n w $'\r'"#,
                "OUTM4",
            ),
            "K",
            "^W killed exactly the trailing word",
        );
    }

    /// `^A` then `^E` returns to the end, and only then does the typed
    /// `9` land where it makes the marker. If `^E` were a no-op the
    /// digit would land at the START and the command becomes `9print …`,
    /// which is not a command at all.
    #[test]
    fn ctrl_e_returns_to_the_end_of_the_line() {
        assert_same_verdict(
            &driver(
                "-e",
                r#"zpty -w -n w 'print OUTM${:-}E1'
sleep 1
zpty -w -n w $'\C-a'
sleep 1
zpty -w -n w $'\C-e'
sleep 1
zpty -w -n w '9'
sleep 1
zpty -w -n w $'\r'"#,
                "OUTME19",
            ),
            "K",
            "^E returned to the end of the line",
        );
    }

    /// `ESC-b` (backward-word) then an insert. The needle only exists if
    /// the cursor actually moved to the START of the last word —
    /// `Xzzz` cannot appear in the echo of `print zzz`.
    #[test]
    fn meta_b_moves_back_one_word() {
        assert_same_verdict(
            &driver(
                "-e",
                r#"zpty -w -n w 'print zzz'
sleep 1
zpty -w -n w $'\eb'
sleep 1
zpty -w -n w 'X'
sleep 1
zpty -w -n w $'\r'"#,
                "Xzzz",
            ),
            "K",
            "ESC-b moved back one word",
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════
// vi keymap
// ═══════════════════════════════════════════════════════════════════════

mod vi {
    use super::*;

    /// ESC leaves insert mode and `I` re-enters it at the start of the
    /// line — two keymap transitions plus a movement in one case.
    #[test]
    fn escape_then_I_inserts_at_the_line_start() {
        assert_same_verdict(
            &driver(
                "-v",
                r#"zpty -w -n w 'rint OUT${:-}M5'
zpty -w -n w $'\e'
zpty -w -n w 'I'
zpty -w -n w 'p'
zpty -w -n w $'\r'"#,
                "OUTM5",
            ),
            "K",
            "ESC then I inserted at the line start",
        );
    }

    /// `dd` kills the whole line from command mode, then `i` returns to
    /// insert mode for the real command.
    #[test]
    fn dd_kills_the_line_then_i_resumes_insert() {
        assert_same_verdict(
            &driver(
                "-v",
                r#"zpty -w -n w 'junkjunk'
zpty -w -n w $'\e'
zpty -w -n w 'dd'
zpty -w -n w 'i'
zpty -w -n w 'print OUT${:-}M6'
zpty -w -n w $'\r'"#,
                "OUTM6",
            ),
            "K",
            "dd killed the line and i resumed insert",
        );
    }

    /// `x` deletes the character under the cursor and `.` repeats it —
    /// the repeat register has to remember the last CHANGE, so both
    /// trailing characters go and the marker is left flush.
    #[test]
    fn x_then_dot_repeats_the_delete() {
        assert_same_verdict(
            &driver(
                "-v",
                r#"zpty -w -n w 'print OUT${:-}M7 xy'
zpty -w -n w $'\e'
zpty -w -n w 'x'
zpty -w -n w '.'
zpty -w -n w $'\r'"#,
                "OUTM7",
            ),
            "K",
            "x then . repeated the delete",
        );
    }

    /// `yy` `dd` `p` — yank the line, delete it, put it back. Exercises
    /// the cut buffer through both the yank and the delete path, which
    /// are separate writers to it.
    #[test]
    fn yy_dd_p_restores_the_line() {
        assert_same_verdict(
            &driver(
                "-v",
                r#"zpty -w -n w 'print OUT${:-}M8'
zpty -w -n w $'\e'
zpty -w -n w 'yy'
zpty -w -n w 'dd'
zpty -w -n w 'p'
zpty -w -n w $'\r'"#,
                "OUTM8",
            ),
            "K",
            "yy dd p restored the line",
        );
    }

    /// `A` appends at the END of the line from command mode — the vi
    /// motion plus a keymap switch in one keystroke.
    #[test]
    fn A_appends_at_the_end_of_the_line() {
        assert_same_verdict(
            &driver(
                "-v",
                r#"zpty -w -n w 'print OUT'
sleep 1
zpty -w -n w $'\e'
sleep 1
zpty -w -n w 'A'
sleep 1
zpty -w -n w 'M${:-}E3'
sleep 1
zpty -w -n w $'\r'"#,
                "OUTME3",
            ),
            "K",
            "A appended at the end of the line",
        );
    }

    /// `0` moves to column zero, then `i` inserts there. The line is
    /// typed missing its first character, so only a correct `0` puts the
    /// `p` where `print` needs it.
    #[test]
    fn zero_moves_to_the_start_before_inserting() {
        assert_same_verdict(
            &driver(
                "-v",
                r#"zpty -w -n w 'rint OUTM${:-}E4'
sleep 1
zpty -w -n w $'\e'
sleep 1
zpty -w -n w '0'
sleep 1
zpty -w -n w 'i'
sleep 1
zpty -w -n w 'p'
sleep 1
zpty -w -n w $'\r'"#,
                "OUTME4",
            ),
            "K",
            "0 moved to the start of the line",
        );
    }

    /// `cw` — an operator plus a motion, the composite form vi users
    /// live in. `0` then `w` parks the cursor on `junk`, which `cw`
    /// replaces with the marker.
    #[test]
    fn cw_changes_the_word_under_the_cursor() {
        assert_same_verdict(
            &driver(
                "-v",
                r#"zpty -w -n w 'print junk'
sleep 1
zpty -w -n w $'\e'
sleep 1
zpty -w -n w '0'
sleep 1
zpty -w -n w 'w'
sleep 1
zpty -w -n w 'cw'
sleep 1
zpty -w -n w 'OUTM${:-}E5'
sleep 1
zpty -w -n w $'\r'"#,
                "OUTME5",
            ),
            "K",
            "cw changed the word under the cursor",
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════
// history recall
// ═══════════════════════════════════════════════════════════════════════

/// Run a command, then recall it with `^P` and run it again. The
/// verdict is that the marker appears TWICE: once needs only the first
/// execution, so a shell that recalls the wrong line — or nothing —
/// still fails.
///
/// `HISTFILE` is unset on purpose. zshrs's default history sink is a
/// shared store rather than a per-`$ZDOTDIR` file, so a probe that let
/// it persist would both read other sessions' lines and write its own
/// into the user's real history.
#[test]
fn ctrl_p_recalls_and_reruns_the_previous_line() {
    let driver = format!(
        "{OPEN}
zpty -w w 'bindkey -e'
zpty -w w 'unset HISTFILE; HISTSIZE=100; SAVEHIST=0'
sleep 1
zpty -w -n w 'print OUT${{:-}}M9'
zpty -w -n w $'\\r'
sleep 1
zpty -w -n w $'\\C-p'
zpty -w -n w $'\\r'
sleep 2
{DRAIN}
integer n=0
local rest=\"$all\"
while [[ $rest == *OUTM9* ]]; do (( n++ )); rest=\"${{rest#*OUTM9}}\"; done
if (( n >= 2 )); then print \"K=yes\"; else print \"K=no\"; fi
"
    );
    assert_same_verdict(&driver, "K", "^P recalled and re-ran the previous line");
}

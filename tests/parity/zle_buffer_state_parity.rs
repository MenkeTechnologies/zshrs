//! ZLE buffer-state parity — the line editor's OWN view of the line,
//! read out of a widget rather than off the screen.
//!
//! `zle_editing_parity` judges editing by what the shell ended up
//! RUNNING. That is robust, but it can only ever answer "did the right
//! command execute": two shells whose cursor ends one column apart run
//! the same command and both pass. A cursor that is off by one is a
//! real bug — it is what makes the next keystroke land in the wrong
//! place — and it is structurally invisible to that style of test.
//!
//! These read `$BUFFER` and `$CURSOR` from inside a widget and write
//! them to a file, which the harness compares as EXACT strings. So a
//! case here fails on `CUR=[3]` vs `CUR=[4]` with identical buffers,
//! which is the point.
//!
//! Two things the harness enforces, both learned by getting them wrong:
//! the driver must DRAIN the pty before reading the file (an inner
//! shell whose output buffer fills blocks on write, and the widget then
//! never runs), and the dump key is bound in `vicmd` as well as `main`
//! (a vi probe dumps from command mode). Both failures produce empty
//! output on BOTH shells, which reads as agreement — so
//! `assert_same_dump` refuses to pass when the reference dumped
//! nothing.
//!
//! Skip pattern: no-ops silently when `zsh` isn't on PATH or when
//! `zsh/zpty` will not load. Harness contract: `zpty_probe`.

#![allow(non_snake_case)]
#![allow(clippy::doc_lazy_continuation)]

use crate::zpty_probe::{assert_same_dump, sq, DUMP_KEY, DUMP_WIDGET, OPEN};

/// Type `keys` under `keymap`, then dump `$BUFFER`/`$CURSOR`.
///
/// The drain loop is spelled out here rather than reused from `DRAIN`
/// because this driver reads a FILE afterwards instead of matching the
/// transcript — but draining first is mandatory either way.
///
/// Two seconds per keystroke, not one. This box runs many builds at
/// once and a keystroke written while the inner shell is still
/// redrawing is simply dropped; at one second the module was
/// intermittently flaky under `--test-threads=4`. The settle time is
/// the honest fix — retrying a mismatch would convert a real divergence
/// into a pass, which is exactly what a pin must never do.
fn driver(keymap: &str, keys: &[&str]) -> String {
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
zpty -w w 'bindkey {keymap}'
{DUMP_WIDGET}
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

// ═══════════════════════════════════════════════════════════════════════
// emacs keymap
// ═══════════════════════════════════════════════════════════════════════

mod emacs {
    use super::*;

    /// `^A` leaves the text alone and puts the cursor at column 0. The
    /// buffer half of the assertion is what proves it MOVED rather than
    /// edited.
    #[test]
    fn ctrl_a_parks_the_cursor_at_column_zero() {
        assert_same_dump(
            &driver("-e", &["abcd", "\\C-a"]),
            "^A left the text alone and parked the cursor at column 0",
        );
    }

    /// `^E` from column 0 returns to the end — cursor at the length of
    /// the buffer, not one short of it.
    #[test]
    fn ctrl_e_returns_the_cursor_to_the_end() {
        assert_same_dump(
            &driver("-e", &["abcd", "\\C-a", "\\C-e"]),
            "^E returned the cursor to the end of the line",
        );
    }

    /// `^W` removes the trailing word AND the cursor follows it. The
    /// trailing space in `print a ` is the detail: `^W` stops at the
    /// word boundary rather than eating the separator too.
    #[test]
    fn ctrl_w_leaves_the_separator_and_moves_the_cursor() {
        assert_same_dump(
            &driver("-e", &["print a b", "\\C-w"]),
            "^W killed the trailing word and left the separator",
        );
    }

    /// `ESC-b` moves to the start of the current word without touching
    /// the text.
    #[test]
    fn meta_b_lands_on_the_word_start() {
        assert_same_dump(
            &driver("-e", &["abc def", "\\eb"]),
            "ESC-b landed the cursor on the word start",
        );
    }
}

// ═══════════════════════════════════════════════════════════════════════
// vi keymap — command-mode motions, dumped from `vicmd`
// ═══════════════════════════════════════════════════════════════════════

mod vi {
    use super::*;

    /// `0` in command mode is a pure motion: column 0, text untouched.
    #[test]
    fn zero_is_a_pure_motion_to_column_zero() {
        assert_same_dump(
            &driver("-v", &["abcd", "\\e", "0"]),
            "vi 0 moved to column zero without editing",
        );
    }

    /// `$` parks on the LAST character, not past it — vi's cursor model
    /// differs from emacs' here, and the difference is exactly one
    /// column.
    #[test]
    fn dollar_parks_on_the_last_character() {
        assert_same_dump(
            &driver("-v", &["abcd", "\\e", "0", "$"]),
            "vi $ parked on the last character rather than past it",
        );
    }

    /// `dw` from column 0 deletes the first word AND its trailing
    /// space, leaving the cursor at the new start.
    #[test]
    fn dw_deletes_the_word_and_its_separator() {
        assert_same_dump(
            &driver("-v", &["abc def", "\\e", "0", "dw"]),
            "vi dw deleted the word and its separator",
        );
    }
}

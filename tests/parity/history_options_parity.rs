//! History-option parity — what actually lands in the history list, as
//! seen by recalling it with `^P`.
//!
//! These options only do anything in an interactive shell, and their
//! whole effect is on WHICH entry `^P` reaches. Each case types the same
//! commands, presses `^P` the same number of times, and dumps the
//! buffer: if an option is ignored, the walk lands somewhere else and
//! the dump differs.
//!
//! The `HIST_IGNORE_DUPS` pair is the load-bearing one. Identical
//! keystrokes, opposite settings, DIFFERENT destinations: with the
//! option off two identical commands take two slots, so `^P ^P` is
//! still inside them and lands on `print DUP`; with it on they take
//! one, so the same keystrokes walk past into the setup line before
//! them. A shell that ignores the option gives the same answer to both
//! and fails one of them, whichever way it errs. `HIST_IGNORE_SPACE` is
//! pinned as the same kind of pair.
//!
//! Three of these were CUT last round for flakiness and are back
//! because the cause was found: the driver now PUMPS the pty after
//! every write (`zpty_probe::OPEN_PUMPED`) instead of sleeping a fixed
//! two seconds. The inner shell blocks once its output buffer fills,
//! which is what swallowed the measurement partway through a
//! six-round-trip sequence; draining as it goes keeps the buffer empty
//! and makes the wait adaptive. Same cases, 3/3 instead of 1-in-3, and
//! several times faster.
//!
//! Skip pattern: no-ops silently when `zsh` isn't on PATH or when
//! `zsh/zpty` will not load. Harness contract: `zpty_probe`.

#![allow(non_snake_case)]
#![allow(clippy::doc_lazy_continuation)]

use crate::zpty_probe::{assert_same_dump, sq, OPEN_PUMPED};

/// Run `cmds` as whole lines, send `keys` as individual keystrokes,
/// then dump the buffer — pumping the pty after every write.
///
/// `HISTFILE` is unset and `SAVEHIST=0` so the probe neither reads nor
/// writes any real history: zshrs's default sink is a shared store, not
/// a per-`$ZDOTDIR` file, and a probe that let it persist would both
/// see other sessions' lines and leave its own behind.
fn driver(setopt: &str, cmds: &[&str], keys: &[&str]) -> String {
    let lines: String = cmds
        .iter()
        .map(|c| format!("zpty -w w {}; pump\n", sq(c)))
        .collect();
    let typed: String = keys
        .iter()
        .map(|k| format!("zpty -w -n w $'\\{k}'; pump\n"))
        .collect();
    format!(
        "{OPEN_PUMPED}
zpty -w w 'HISTSIZE=100; SAVEHIST=0; unset HISTFILE'; pump
zpty -w w {}; pump
zpty -w w 'bindkey -e'; pump
zpty -w w 'dumpbuf(){{ print -r -- \"BUF=[$BUFFER]\" >! $OUTFILE }}; zle -N dumpbuf; bindkey \"^X^G\" dumpbuf'; pump
{lines}{typed}zpty -w -n w $'\\C-x\\C-g'; pump
zpty -d w 2>/dev/null
",
        sq(setopt)
    )
}

/// Two DISTINCT commands, two `^P`s, so the walk lands on the older
/// one. The control: it fixes what "two entries back" means before any
/// option is allowed to change it.
#[test]
fn two_ctrl_p_walk_two_entries_back() {
    assert_same_dump(
        &driver(
            "unsetopt hist_ignore_dups",
            &["print A1", "print A2"],
            &["C-p", "C-p"],
        ),
        "^P ^P walked two entries back",
    );
}

/// With `NO_HIST_IGNORE_DUPS`, two identical commands occupy two
/// separate slots, so `^P ^P` is still inside them.
#[test]
fn duplicates_occupy_two_slots_without_the_option() {
    assert_same_dump(
        &driver(
            "unsetopt hist_ignore_dups",
            &["print DUP", "print DUP"],
            &["C-p", "C-p"],
        ),
        "duplicates took two history slots with NO_HIST_IGNORE_DUPS",
    );
}

/// With `HIST_IGNORE_DUPS` the second is not recorded, so the SAME
/// keystrokes reach what came before — a visibly different destination
/// from the case above, which is what makes the pair a test of the
/// option rather than of `^P`.
#[test]
fn duplicates_collapse_into_one_slot_with_the_option() {
    assert_same_dump(
        &driver(
            "setopt hist_ignore_dups",
            &["print DUP", "print DUP"],
            &["C-p", "C-p"],
        ),
        "duplicates collapsed to one slot with HIST_IGNORE_DUPS",
    );
}

/// `HIST_REDUCE_BLANKS` normalises runs of whitespace on the way into
/// the history, so recalling gives back the collapsed form rather than
/// what was typed.
#[test]
fn hist_reduce_blanks_collapses_whitespace_on_recall() {
    assert_same_dump(
        &driver("setopt hist_reduce_blanks", &["print   a   b"], &["C-p"]),
        "HIST_REDUCE_BLANKS collapsed the whitespace on recall",
    );
}

/// `HIST_IGNORE_SPACE` keeps a space-prefixed command out of the
/// history entirely, so `^P` skips it and gives back the command
/// before.
#[test]
fn hist_ignore_space_keeps_the_command_out_of_history() {
    assert_same_dump(
        &driver(
            "setopt hist_ignore_space",
            &["print KEEPME", " print SPACED"],
            &["C-p"],
        ),
        "HIST_IGNORE_SPACE kept the space-prefixed command out",
    );
}

/// …and without it the same command IS recorded, so `^P` gives it back.
/// The pair is what proves the option is read, rather than the shell
/// skipping the line for some unrelated reason.
#[test]
fn a_space_prefixed_command_is_recorded_without_the_option() {
    assert_same_dump(
        &driver(
            "unsetopt hist_ignore_space",
            &["print KEEPME", " print SPACED"],
            &["C-p"],
        ),
        "a space-prefixed command was recorded with NO_HIST_IGNORE_SPACE",
    );
}

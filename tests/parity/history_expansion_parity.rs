//! Interactive history-expansion parity — `!!` and `!$` at the prompt,
//! with and without `HIST_VERIFY`.
//!
//! History expansion happens in the line editor, so a `-c` script never
//! sees it. The two halves behave differently on purpose:
//!
//!   * without `HIST_VERIFY`, an expansion is substituted and the
//!     command RUNS immediately;
//!   * with `HIST_VERIFY` it is substituted into the BUFFER and left
//!     there for the user to look at, and Return has to be pressed
//!     again to run it. That is the entire point of the option — it is
//!     the guard against `!!` running something you did not mean.
//!
//! Both are pinned, because a shell that ignores the option passes a
//! test of the other half.
//!
//! Skip pattern: no-ops silently when `zsh` isn't on PATH or when
//! `zsh/zpty` will not load. Harness contract: `zpty_probe`.

#![allow(non_snake_case)]
#![allow(clippy::doc_lazy_continuation)]

use crate::zpty_probe::{assert_same_dump, dump_widget, sq, DUMP_KEY, OPEN};

/// Run `print MARKONE`, then type `recall` and press Return, and report
/// HOW MANY times the marker reached the terminal.
///
/// The count is the verdict because it separates the three outcomes
/// cleanly: 2 means the recall produced nothing, 3 means it was
/// substituted into the buffer but not run, 4 means it ran. Comparing
/// the two shells' counts therefore catches "ran when it should have
/// waited" and "vanished" as different failures.
fn count_driver(setopt: &str, recall: &str) -> String {
    format!(
        "{OPEN}
zpty -w w 'unsetopt beep'
zpty -w w 'HISTSIZE=100; SAVEHIST=0; unset HISTFILE'
zpty -w w {}
sleep 2
zpty -w -n w 'print MARKONE'
sleep 2
zpty -w -n w $'\\r'
sleep 2
zpty -w -n w {}
sleep 2
zpty -w -n w $'\\r'
sleep 3
local out all=
integer i=0
while (( i++ < 50 )); do
  if zpty -r -t w out 2>/dev/null; then all+=\"$out\"; else sleep 0.1; fi
done
zpty -d w 2>/dev/null
print -r -- \"N=$(print -r -- \"$all\" | grep -c MARKONE)\" >! $OUTFILE
",
        sq(setopt),
        sq(recall)
    )
}

/// Same interaction, but dumping the BUFFER after the recall's Return —
/// which is where `HIST_VERIFY` is supposed to leave the expansion.
fn buffer_driver(setopt: &str, recall: &str) -> String {
    let widget = dump_widget(r#""BUF=[$BUFFER] CUR=[$CURSOR]""#);
    format!(
        "{OPEN}
zpty -w w 'unsetopt beep'
zpty -w w 'HISTSIZE=100; SAVEHIST=0; unset HISTFILE'
zpty -w w {}
{widget}
sleep 2
zpty -w -n w 'print MARKONE'
sleep 2
zpty -w -n w $'\\r'
sleep 2
zpty -w -n w {}
sleep 2
zpty -w -n w $'\\r'
sleep 2
{DUMP_KEY}
local out all=
integer i=0
while (( i++ < 50 )); do
  if zpty -r -t w out 2>/dev/null; then all+=\"$out\"; else sleep 0.1; fi
done
zpty -d w 2>/dev/null
",
        sq(setopt),
        sq(recall)
    )
}

// ═══════════════════════════════════════════════════════════════════════
// NO_HIST_VERIFY — the expansion runs, and both shells agree
// ═══════════════════════════════════════════════════════════════════════

/// `!!` recalls the whole previous command and runs it.
#[test]
fn bang_bang_expands_and_runs_without_hist_verify() {
    assert_same_dump(
        &count_driver("unsetopt hist_verify", "!!"),
        "!! expanded and ran with NO_HIST_VERIFY",
    );
}

/// `!$` recalls the previous command's last argument.
#[test]
fn bang_dollar_expands_and_runs_without_hist_verify() {
    assert_same_dump(
        &count_driver("unsetopt hist_verify", "print !$"),
        "!$ expanded and ran with NO_HIST_VERIFY",
    );
}

// ═══════════════════════════════════════════════════════════════════════
// HIST_VERIFY — the expansion must land in the buffer, unrun
// ═══════════════════════════════════════════════════════════════════════

/// zshrs gap: under `HIST_VERIFY` the expanded line is DROPPED instead
/// of being left in the buffer for confirmation.
///
///     setopt hist_verify
///     print MARKONE          # runs
///     !!  <Return>
///       zsh    buffer becomes `print MARKONE`, nothing runs
///       zshrs  buffer is EMPTY, nothing runs, the line is gone
///
/// Measured both ways. By buffer: zsh `BUF=[print MARKONE] CUR=[8]`,
/// zshrs `BUF=[] CUR=[0]`. By marker count: zsh 3 (echo, output, and
/// the expansion sitting in the buffer), zshrs 2 (the expansion never
/// appears at all).
///
/// The control is the NO_HIST_VERIFY pair above, where both shells
/// score 4 — so history expansion itself is correct and this is the
/// verify path specifically. `setopt hist_verify` is in this repo's
/// daily-driver config, so the effect is that `!!` silently does
/// nothing there.
#[test]
#[ignore = "zshrs gap: HIST_VERIFY drops the expanded line instead of leaving it in the buffer"]
fn hist_verify_leaves_bang_bang_in_the_buffer() {
    assert_same_dump(
        &buffer_driver("setopt hist_verify", "!!"),
        "HIST_VERIFY left the !! expansion in the buffer",
    );
}

/// Same gap for a word designator rather than a whole line.
#[test]
#[ignore = "zshrs gap: HIST_VERIFY drops the expanded line instead of leaving it in the buffer"]
fn hist_verify_leaves_bang_dollar_in_the_buffer() {
    assert_same_dump(
        &buffer_driver("setopt hist_verify", "print !$"),
        "HIST_VERIFY left the !$ expansion in the buffer",
    );
}

/// The same divergence counted rather than dumped, so the gap is pinned
/// on both measurements: 3 (buffered) versus 2 (vanished).
#[test]
#[ignore = "zshrs gap: HIST_VERIFY drops the expanded line instead of leaving it in the buffer"]
fn hist_verify_does_not_lose_the_expansion() {
    assert_same_dump(
        &count_driver("setopt hist_verify", "!!"),
        "HIST_VERIFY kept the expansion visible",
    );
}

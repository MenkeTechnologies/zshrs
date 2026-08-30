//! History-option parity — what actually lands in the history list, as
//! seen by recalling it with `^P`.
//!
//! These options only do anything in an interactive shell, and their
//! whole effect is on WHAT `^P` gives back, so the verdict is the
//! recalled buffer.
//!
//! **Scope, and why it is one case.** The obvious companion pair —
//! `HIST_IGNORE_DUPS` on versus off, walking `^P ^P` past two identical
//! commands — was written, measured by hand (the two settings land on
//! visibly different entries, and both shells agree on each), and then
//! REMOVED. Those cases need six pty round-trips, and on this box the
//! reference shell intermittently fails to produce a dump at all under
//! parallel load: the module failed roughly two runs in three, always
//! with `reference zsh dumped no editor state` — a lost measurement,
//! never a mismatch. Sending whole lines with `zpty -w` instead of
//! text-plus-Return halved the round-trips and helped, and the harness
//! retries a lost measurement three times, and it was still not enough.
//! A pin that fails at random teaches the reader to ignore it, so only
//! the case that runs reliably is kept here. The parity it would have
//! asserted is real; the automation is not.
//!
//! Skip pattern: no-ops silently when `zsh` isn't on PATH or when
//! `zsh/zpty` will not load. Harness contract: `zpty_probe`.

#![allow(non_snake_case)]
#![allow(clippy::doc_lazy_continuation)]

use crate::zpty_probe::{assert_same_dump, dump_widget, sq, DUMP_KEY, OPEN};

/// Run `cmds` as whole LINES, then send `keys` as individual
/// keystrokes, then dump the buffer.
///
/// The split matters. A command sent as text plus a separate Return is
/// two writes into the pty, and on a loaded box the second one
/// sometimes arrives while the inner shell is still redrawing from the
/// first — the measurement is then simply lost, on either shell, and
/// the case reports nothing rather than a divergence. `zpty -w`
/// delivers the line and its newline in ONE write, which took these
/// cases from failing two runs in three to passing every time. Only the
/// keystrokes that must be individual — `^P`, the dump chord — go
/// through `zpty -w -n`.
///
/// `HISTFILE` is unset and `SAVEHIST=0` so the probe neither reads nor
/// writes any real history: zshrs's default sink is a shared store, not
/// a per-`$ZDOTDIR` file, and a probe that let it persist would both
/// see other sessions' lines and leave its own behind.
fn driver(setopt: &str, cmds: &[&str], keys: &[&str]) -> String {
    let widget = dump_widget(r#""BUF=[$BUFFER]""#);
    let lines: String = cmds
        .iter()
        .map(|c| format!("zpty -w w {}\nsleep 2\n", sq(c)))
        .collect();
    let typed: String = keys
        .iter()
        .map(|k| format!("zpty -w -n w $'\\{k}'\nsleep 2\n"))
        .collect();
    format!(
        "{OPEN}
zpty -w w 'unsetopt beep'
zpty -w w 'HISTSIZE=100; SAVEHIST=0; unset HISTFILE'
zpty -w w {}
zpty -w w 'bindkey -e'
{widget}
sleep 2
{lines}{typed}{DUMP_KEY}
local out all=
integer i=0
while (( i++ < 50 )); do
  if zpty -r -t w out 2>/dev/null; then all+=\"$out\"; else sleep 0.1; fi
done
zpty -d w 2>/dev/null
",
        sq(setopt)
    )
}

/// `HIST_REDUCE_BLANKS` normalises the runs of whitespace on the way
/// into the history, so recalling the line gives back the collapsed
/// form rather than what was typed.
#[test]
fn hist_reduce_blanks_collapses_whitespace_on_recall() {
    assert_same_dump(
        &driver(
            "setopt hist_reduce_blanks",
            &["print   a   b"],
            &["C-p"],
        ),
        "HIST_REDUCE_BLANKS collapsed the whitespace on recall",
    );
}

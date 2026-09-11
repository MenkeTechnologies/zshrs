//! `$PENDING` parity — the count of bytes sitting unread on the terminal.
//!
//! `$PENDING` is `noquery(0)` in C (`Src/Zle/zle_params.c:531-535` →
//! `Src/utils.c:2992-3011`), i.e. a `FIONREAD` ioctl on the tty. It is NOT the
//! unget buffer — that is `$KEYS_QUEUED_COUNT` (`c:470`, `kungetct`). zsh
//! exposes both because they are different queues, and a widget that conflates
//! them gets the wrong answer precisely when the user is typing fast.
//!
//! This is load-bearing for real plugins rather than cosmetic.
//! zsh-autosuggestions guards its recompute with
//!
//! ```text
//! if (( $PENDING > 0 || $KEYS_QUEUED_COUNT > 0 )); then
//!     POSTDISPLAY="$orig_postdisplay"
//!     return $retval
//! fi
//! ```
//!
//! — "don't fetch a new suggestion if there's more input to be read
//! immediately". `$PENDING` was hardcoded to 0, so that guard could never fire
//! on typed-ahead input and every fast keystroke fell through into the
//! recompute path instead of deferring. A constant is invisible in any test
//! that does not actually queue bytes, which is why this one does.
//!
//! The write is deliberately a SINGLE `zpty -w -n` carrying the dump key
//! followed by filler: the filler is still unread when the widget bound to the
//! dump key runs, so `FIONREAD` has something to report. Splitting it into two
//! writes would let the inner shell drain the first before the second arrived
//! and both shells would report 0 — a vacuous pass.
//!
//! Skip pattern: no-ops silently when `zsh` isn't on PATH or when `zsh/zpty`
//! will not load. Harness contract: `zpty_probe`.

#![allow(non_snake_case)]
#![allow(clippy::doc_lazy_continuation)]

use crate::zpty_probe::{assert_same_dump, dump_widget, OPEN_PUMPED};

/// Number of filler bytes queued behind the dump key.
const FILLER: usize = 10;

/// Drive both shells: bind a widget that prints `expr`, then send the dump key
/// plus `FILLER` bytes in one write.
fn driver(expr: &str) -> String {
    let widget = dump_widget(expr);
    let filler = "A".repeat(FILLER);
    // The burst below IS the dump trigger, so `DUMP_KEY` is deliberately not
    // used: firing the widget a second time would overwrite `$OUTFILE` with a
    // dump taken when the queue had already drained, i.e. 0 on both shells —
    // agreement that measures nothing.
    format!(
        "{OPEN_PUMPED}
{widget}
pump
zpty -w -n w $'\\C-x\\C-g{filler}'
pump
"
    )
}

/// With bytes still unread on the tty, `$PENDING` must report them.
///
/// Before the fix this returned 0 on zshrs and 10 on zsh. The assertion is a
/// parity one rather than `== 10` on purpose: the exact count is whatever the
/// tty layer has managed to deliver, and pinning a literal would make this
/// flake. What must not differ is the two shells' answer to the same burst.
#[test]
fn pending_counts_bytes_waiting_on_the_terminal() {
    assert_same_dump(&driver("$PENDING"), "PENDING-with-queued-input");
}

/// `$PENDING` and `$KEYS_QUEUED_COUNT` are separate queues, and the bug was
/// reading the wrong one. Dump both together so a future change that wires
/// `$PENDING` back to `kungetct` shows up here: that would make the two move
/// together, and against zsh they do not.
#[test]
fn pending_and_keys_queued_count_are_distinct_queues() {
    assert_same_dump(
        &driver("\"$PENDING/$KEYS_QUEUED_COUNT\""),
        "PENDING-vs-KEYS_QUEUED_COUNT",
    );
}

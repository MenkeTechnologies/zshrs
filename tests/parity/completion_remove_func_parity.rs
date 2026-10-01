//! `compadd -R func`: the suffix REMOVE FUNCTION runs on the next key.
//!
//! `makesuffixstr` (Src/Zle/zle_misc.c:1611-1612) stores the name in the
//! file-static `suffixfunc`, and `iremovesuffix` (c:1667-1702) calls it with
//! the suffix length as `$1`, inside a fresh parameter scope that carries
//! the ZLE parameters (c:1691-1696), so the function can edit `$LBUFFER`.
//! It runs INSTEAD of the ordinary suffix-character walk (c:1703 `else`).
//!
//!     _rmc() { compadd -S pre- -R rmf -- gamma }
//!     rmf()  { LBUFFER=${LBUFFER%pre-} }
//!
//!     rmc<TAB>      ->  rmc gammapre-
//!     x             ->  rmc gammax     (rmf stripped the suffix first)
//!
//! zshrs stored the name in one static and read another that nothing ever
//! wrote, so the function never ran and the line stayed `rmc gammapre-x`.
//!
//! The widget is a `zle -C` completion widget whose function calls
//! `compadd` directly, so no `compinit` and no function directory is needed.
//!
//! Skip pattern: no-ops silently when `zsh` isn't on PATH or when
//! `zsh/zpty` will not load. Harness contract: `zpty_probe`.

#![allow(non_snake_case)]
#![allow(clippy::doc_lazy_continuation)]

use crate::zpty_probe::{assert_same_dump, DRAIN, DUMP_KEY, DUMP_WIDGET, OPEN};

#[test]
fn remove_function_runs_on_the_next_key_and_can_edit_the_line() {
    let driver = format!(
        "{OPEN}
zpty -w w 'unsetopt beep'
zpty -w w '_rmc() {{ compadd -S pre- -R rmf -- gamma }}'
zpty -w w 'rmf() {{ LBUFFER=${{LBUFFER%pre-}} }}'
zpty -w w 'zle -C rmcw complete-word _rmc; bindkey \"^I\" rmcw'
{DUMP_WIDGET}
sleep 1
zpty -w -n w 'rmc '
sleep 1
zpty -w -n w $'\\t'
sleep 2
zpty -w -n w 'x'
sleep 2
{DUMP_KEY}
{DRAIN}
"
    );
    assert_same_dump(
        &driver,
        "the -R remove function stripped the suffix before the typed key",
    );
}

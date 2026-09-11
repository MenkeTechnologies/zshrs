//! What the user types must survive an `async_precmd` batch.
//!
//! `async_precmd` is a zshrs-only hook: `preprompt()` hands its functions to
//! a pool worker so they never delay the prompt. Those functions run
//! against the SAME `locallevel` counter and the SAME `paramtab` as the
//! shell thread — C keeps both for one thread of execution (`Src/params.c`
//! `:5837` `locallevel++`, `:5856` `locallevel--`, and `scanendscope`'s
//! `if (pm->level > locallevel)` delete at `:5904-5907`). A shell-function
//! widget publishes `$BUFFER` and its family at the scope level it just
//! pushed (`Src/Zle/zle_main.c:1533-1534`, `Src/Zle/zle_params.c:206`), so
//! a worker scope exit landing inside the widget deleted that `$BUFFER`,
//! and the write-back copied the now-empty value into the editor: every
//! character typed so far on the line vanished. `print -r …` reached the
//! executor as `int -r …`, or as nothing at all.
//!
//! The shape that exposes it is ordinary — `self-insert` wrapped by a shell
//! function, which is what zpwr's `zpwrSelfInsert` is, and typing while a
//! hook runs.
//!
//! The probe pins the timing instead of racing for it: the hook spins until
//! the driver creates a release file, so the typed characters are certain
//! to arrive while a batch is in flight. zsh has no `async_precmd`, the
//! array is inert there and the line always arrives whole; that is the
//! reference.
//!
//! Skip pattern: no-ops silently when `zsh` isn't on PATH or when
//! `zsh/zpty` will not load.

use crate::zpty_probe::{assert_same_verdict, OPEN_PUMPED};

/// The hook functions come from a SOURCED file, the way an rc file
/// registers them, so the worker finds them in the shared `shfunctab`.
///
/// Order of events: sourcing the file leaves a prompt, which fires the
/// batch; the hook then spins — churning parameter scopes on the worker —
/// until the release file appears. The probe line is typed into that window
/// WITHOUT a newline, the release lands two seconds later, and only then is
/// Return sent, so the line is accepted whatever the shell did with the
/// keystrokes and the verdict is purely whether its text survived them.
///
/// The marker is assembled at run time (`TYPED${:-}1-…`), so only the
/// command's OUTPUT can satisfy the match, never the terminal's echo of
/// what was typed. The spin is bounded so a shell that never sees the
/// release still finishes the test.
const TYPEAHEAD: &str = r#"
rel=${TMPPREFIX:-/tmp/zsh}-asyncrel-$$
cfg=${TMPPREFIX:-/tmp/zsh}-asynccfg-$$.zsh
rm -f $rel
{
  print -r -- 'wrapped_self_insert() { zle .self-insert }'
  print -r -- 'zle -N self-insert wrapped_self_insert'
  print -r -- 'hook_step() { local s=$1 }'
  print -r -- "spin_hook() { local i=0; while [[ ! -e $rel ]] && (( i < 400000 )); do hook_step \$(( i++ )); done }"
  print -r -- 'async_precmd_functions=(spin_hook)'
} > $cfg
zpty -w w "source $cfg"; pump
zpty -w -n w "print -r TYPED\${:-}1-abcdefghijklmnopqrstuvwxyz"
sleep 2
touch $rel
sleep 1
zpty -w -n w $'\r'
integer k=0
while (( k++ < 20 )) && [[ $all != *TYPED1-abcdefghijklmnopqrstuvwxyz* ]]; do pump; done
zpty -d w 2>/dev/null
rm -f $cfg $rel
setopt extended_glob
all="${all//$'\e'\[[0-9;?]#[a-zA-Z]/}"
if [[ $all == *TYPED1-abcdefghijklmnopqrstuvwxyz* ]]; then print TYPEAHEAD=yes; else print TYPEAHEAD=no; fi
"#;

#[test]
fn typed_text_survives_an_async_precmd_batch() {
    assert_same_verdict(
        &format!("{OPEN_PUMPED}{TYPEAHEAD}"),
        "TYPEAHEAD",
        "a line typed while an async_precmd hook was running ran intact",
    );
}

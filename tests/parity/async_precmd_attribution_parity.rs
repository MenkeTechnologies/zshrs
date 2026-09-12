//! A diagnostic must name the frame that RAISED it, not whatever an
//! `async_precmd` hook happens to be doing on a worker.
//!
//! `async_precmd` is a zshrs-only hook: `preprompt()` hands its functions
//! to a pool worker so they never delay the prompt. Running a shell
//! function moves the state C keeps for its single thread of execution —
//! `scriptname`, which `doshfunc` overwrites with the callee's name
//! (`Src/exec.c:5903`) and `zwarning` reads as the diagnostic prefix
//! (`Src/utils.c:147`), and `locallevel` (`Src/params.c:54`), which the
//! same function consults twice to decide the message's SHAPE
//! (`Src/utils.c:150` for the prefix, `:301` in `zerrmsg` for the line
//! number).
//!
//! With one copy of each shared between threads, an error the user caused
//! at the prompt was formatted as if it had happened inside the hook:
//!
//! ```text
//! spin_hook:4: parse error near `done'     zshrs, before the fix
//! zsh: parse error near `done'             zsh, and zshrs after it
//! ```
//!
//! `quiesce` (`crate::async_precmd::quiesce`) does not cover this. It
//! stops the shell thread EXECUTING shell code while a batch runs, and the
//! call that guards the accepted line sits in `init::loop_` AFTER
//! `parse_event` — so the LEXER's diagnostic is raised inside the batch by
//! construction. That makes the timing pinned rather than raced: the hook
//! spins until the driver creates a release file, and the rejected line is
//! typed into that window.
//!
//! zsh has no `async_precmd`, so the array is inert there and the hook
//! never runs; that is the reference.
//!
//! Skip pattern: no-ops silently when `zsh` isn't on PATH or when
//! `zsh/zpty` will not load.

use crate::zpty_probe::{assert_same_verdict, OPEN_PUMPED};

/// The hook functions come from a SOURCED file, the way an rc file
/// registers them, so the worker finds them in the shared `shfunctab`.
///
/// `done` as a whole line is rejected by the lexer with no input left to
/// ask for, so exactly one diagnostic is produced and it is produced
/// during `parse_event` — inside the batch. The verdict is that the
/// diagnostic does not carry the hook's name; `spin_hook` reaches the
/// inner shell only through the `source`d file, never as typed text, so
/// the only way the pty can show it is if the shell printed it.
const ATTRIBUTION: &str = r#"
rel=${TMPPREFIX:-/tmp/zsh}-attribrel-$$
cfg=${TMPPREFIX:-/tmp/zsh}-attribcfg-$$.zsh
rm -f $rel
{
  print -r -- 'hook_step() { local s=$1 }'
  print -r -- "spin_hook() { local i=0; while [[ ! -e $rel ]] && (( i < 2000000 )); do hook_step \$(( i++ )); done }"
  print -r -- 'async_precmd_functions=(spin_hook)'
} > $cfg
zpty -w w "source $cfg"; pump
zpty -w -n w $'done\r'
sleep 2
touch $rel
integer k=0
while (( k++ < 25 )); do pump; done
zpty -d w 2>/dev/null
rm -f $cfg $rel
setopt extended_glob
all="${all//$'\e'\[[0-9;?]#[a-zA-Z]/}"
if [[ $all == *"parse error near"* && $all != *spin_hook:* ]]; then
  print ATTRIB=yes
else
  print ATTRIB=no
fi
"#;

#[test]
fn a_lexer_diagnostic_is_not_attributed_to_an_async_precmd_hook() {
    assert_same_verdict(
        &format!("{OPEN_PUMPED}{ATTRIBUTION}"),
        "ATTRIB",
        "an error raised while an async_precmd hook ran named the shell, not the hook",
    );
}

//! A QUOTED command word must still reach its registered completer.
//!
//! `_set_command` (`Completion/Base/Utility/_set_command:8`) copies
//! `$words[1]` verbatim into `$_comp_command`, and `$words` holds the line
//! AS TYPED — quotes included. So `"env" …<TAB>` hands `_dispatch` the literal
//! five-character string `"env"`, and the only thing that turns it back into
//! the key `$_comps` is filled with is `_dispatch:51`'s `str=${(Q)str}`, whose
//! upstream comment reads "we look up the names of commands after stripping
//! quotes".
//!
//! zshrs skipped that dequote, so every quoted command word missed `$_comps`
//! and fell through to `-default-`. Measured against zsh 5.9.2:
//! `"builtin" "command" "sudo" -E "env" PATH="$PATH" 'rm' '-rf' '--' <TAB>`
//! offered plain files where zsh offers the environment-variable names `_env`
//! contributes; the same line unquoted was already correct, which is what
//! isolated the trigger to the quoting rather than to `_env`.
//!
//! Three cases, the same fixture and the same keystrokes throughout:
//!
//!   * an UNQUOTED command word, the control — everything but the quotes is
//!     identical, so a failure there means the fixture broke;
//!   * a quoted command word that owns a completer (`"zzqcmd"`);
//!   * a quoted PRECOMMAND (`"zzqpre"`), which has to resolve to
//!     `_precommand` before `_normal -p` can recurse onto the real command.
//!
//! The verdict is "did the registered completer RUN", written by the completer
//! itself to a per-run file, not "did some candidate reach the screen": a
//! shell that fell through to `-default-` still lists the directory, so a
//! screen match would score it as completion happening. The marker is
//! assembled at run time (`zzq${:-}ran`) because the inner shell echoes every
//! setup line it is handed.
//!
//! Skip pattern: no-ops silently when `zsh` isn't on PATH, when `zsh/zpty`
//! will not load, or when there is no stock function directory to `compinit`
//! against. Harness contract: `zpty_probe`.

#![allow(non_snake_case)]
#![allow(clippy::doc_lazy_continuation)]

use crate::zpty_probe::{assert_same_verdict, DRAIN, OPEN};

/// Does a stock zsh function directory exist to run `compinit` against?
///
/// The inherited `$FPATH` on a developer box holds thousands of completers
/// and takes minutes to scan in a debug build, so the inner shell is pinned
/// to `/usr/share/zsh/*/functions` — which also keeps `_precommand` and
/// `_normal` at a known version on both sides.
fn stock_fpath_exists() -> bool {
    std::fs::read_dir("/usr/share/zsh")
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .any(|e| e.path().join("functions").is_dir())
        })
        .unwrap_or(false)
}

/// A compsys session that registers `_zzqmark` for `zzqcmd` and
/// `_precommand` for `zzqpre`, types `buffer`, strikes TAB, and reports
/// whether `_zzqmark` ran.
///
/// The marker file's path is baked into the completer's body by the DRIVER
/// process rather than handed to the inner shell as a parameter, and it
/// carries `$$` — `assert_same_verdict` runs zsh and then zshrs as separate
/// processes, so a shared path would let zsh's marker file satisfy a zshrs
/// run that completed nothing at all. It is removed before the keystrokes
/// and again after the verdict.
///
/// No Return is ever sent: the pty is destroyed with the line still in the
/// editor, so whatever TAB inserted is never executed.
fn quoted_word_driver(buffer: &str) -> String {
    format!(
        "{OPEN}
zpty -w w 'unsetopt beep'
zpty -w w 'fpath=(/usr/share/zsh/*/functions(N))'
zpty -w w 'autoload -Uz compinit; compinit -u -D'
sleep 25
local zzqf=${{TMPDIR:-/tmp}}/zzq_quoted_word_parity_$$
command rm -f $zzqf
zpty -w w \"_zzqmark(){{ print -r -- zzq\\${{:-}}ran >>! $zzqf; compadd -- zzqcand }}\"
zpty -w w 'compdef _zzqmark zzqcmd'
zpty -w w 'compdef _precommand zzqpre'
sleep 2
zpty -w -n w {buffer}
sleep 3
zpty -w -n w $'\\t'
sleep 8
{DRAIN}
if [[ -s $zzqf ]] && [[ \"$(<$zzqf)\" == *zzqran* ]]; then print \"K=yes\"; else print \"K=no\"; fi
command rm -f $zzqf
"
    )
}

/// An UNQUOTED command word reaches its completer — the control that proves
/// the probe itself is live. Everything about the two cases below is
/// identical except for the quotes, so a failure here means the fixture
/// broke, not that dequoting did.
#[test]
fn an_unquoted_command_word_reaches_its_completer() {
    if !stock_fpath_exists() {
        eprintln!("skip: no /usr/share/zsh/*/functions to compinit against");
        return;
    }
    assert_same_verdict(
        &quoted_word_driver("'zzqcmd '"),
        "K",
        "an unquoted command word dispatched to its own completer",
    );
}

/// `"zzqcmd" <TAB>` — the command word carries its own quotes, so the
/// `$_comps` key only matches after `_dispatch:51`'s `${(Q)}`.
///
/// This is the minimal reproduction of the reported gap: of the four things
/// the user's line changed at once (quoted precommands, quoted `env`, a
/// quoted inner command, quoted inner arguments), quoting the ONE word that
/// owns the completer is enough on its own.
#[test]
fn a_quoted_command_word_still_reaches_its_completer() {
    if !stock_fpath_exists() {
        eprintln!("skip: no /usr/share/zsh/*/functions to compinit against");
        return;
    }
    assert_same_verdict(
        &quoted_word_driver("'\"zzqcmd\" '"),
        "K",
        "a double-quoted command word dispatched to its own completer",
    );
}

/// `"zzqpre" zzqcmd <TAB>` — the quoted word is a PRECOMMAND. It has to
/// resolve to `_precommand`, which drops it (`shift words; (( CURRENT-- ))`)
/// and re-enters `_normal -p`, so the completer that finally runs is the one
/// registered for the word AFTER it.
///
/// Same dequote, one level further out: with `_dispatch:51` missing, the
/// quoted precommand fell through to `-default-` and the recursion never
/// happened — which is why `"sudo" -E env …<TAB>` lost `_env` while the
/// unquoted `sudo -E env …<TAB>` kept it.
#[test]
fn a_quoted_precommand_still_recurses_onto_the_real_command() {
    if !stock_fpath_exists() {
        eprintln!("skip: no /usr/share/zsh/*/functions to compinit against");
        return;
    }
    assert_same_verdict(
        &quoted_word_driver("'\"zzqpre\" zzqcmd '"),
        "K",
        "a double-quoted precommand dispatched to _precommand and recursed",
    );
}

//! AUTO_REMOVE_SLASH parity: the `/` a directory completion appends is a
//! REMOVABLE suffix, not a provisional character.
//!
//! `makesuffix()` (Src/Zle/zle_misc.c:1598) registers the removal set as
//! `$ZLE_REMOVE_SUFFIX_CHARS`, defaulting to ` \t\n;&|`, and
//! `compresult.c:1117-1118` adds `/` itself. `iremovesuffix()`
//! (zle_misc.c:1699) drops the suffix only when the typed character is in
//! one of those sets — so typing a space after `dir/` removes the slash and
//! typing an ordinary letter does NOT:
//!
//!     f RustroverProjects<TAB>   →  f RustroverProjects/
//!     M                          →  f RustroverProjects/M
//!
//! A shell that treats the suffix as removable for EVERY character eats the
//! slash and leaves `f RustroverProjectsM`, which is a different path and,
//! for a `cd`-like command, a different directory.
//!
//! Both completion engines get the case, because they reach the suffix
//! through different code:
//!
//!   * the DEFAULT completion a shell has before `compinit`;
//!   * `compsys` after `compinit`, with the `zstyle` configuration a real
//!     user session carries — menu selection, `list-colors`, matcher-list
//!     and squeeze-slashes all touch the inserted suffix.
//!
//! Skip pattern: no-ops silently when `zsh` isn't on PATH or when
//! `zsh/zpty` will not load. Harness contract: `zpty_probe`.

#![allow(non_snake_case)]
#![allow(clippy::doc_lazy_continuation)]

use crate::zpty_probe::{
    assert_same_dump, assert_same_verdict, sq, DRAIN, DUMP_KEY, DUMP_WIDGET, OPEN,
};
use std::path::PathBuf;

/// A directory whose name is a strict prefix of nothing else, so TAB has
/// exactly one candidate and must insert the whole name plus the slash.
/// Built under the cargo target dir so both shells see the same layout.
fn fixture_dir() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("parity-suffix-fixture");
    let _ = std::fs::create_dir_all(dir.join("RustroverProjects"));
    dir
}

/// Does a stock zsh function directory exist to run `compinit` against?
fn stock_fpath_exists() -> bool {
    std::fs::read_dir("/usr/share/zsh")
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .any(|e| e.path().join("functions").is_dir())
        })
        .unwrap_or(false)
}

/// Open a pty in the fixture directory with the slash-suffix options in
/// their default (on) state, and the bell silenced so a failed completion
/// cannot be mistaken for output.
fn open_in_fixture() -> String {
    let dir = fixture_dir();
    format!(
        "{OPEN}
zpty -w w 'cd {}'
zpty -w w 'unsetopt beep'
zpty -w w 'setopt autoremoveslash autoparamslash'
sleep 1
",
        dir.display()
    )
}

/// The zstyle configuration a configured session carries. Every one of
/// these touches completion insertion, which is the point: the suffix has
/// to survive the same setup a real `.zshrc` installs.
const ZSTYLES: &str = "\
zstyle ':completion:*' menu select; \
zstyle ':completion:*' list-colors ''; \
zstyle ':completion:*' matcher-list 'm:{a-zA-Z}={A-Za-z}' 'r:|=*' 'l:|=* r:|=*'; \
zstyle ':completion:*' squeeze-slashes true; \
zstyle ':completion:*' special-dirs true; \
zstyle ':completion:*' group-name ''; \
zstyle ':completion:*' verbose true; \
zstyle ':completion:*:descriptions' format '%B%d%b'";

/// Return, so the completed line actually RUNS. The command is `print`,
/// which echoes its argument back: reading the verdict off the OUTPUT
/// rather than off the drawn line means a redraw artifact cannot decide
/// the case, and a shell whose Return did nothing scores no match at all.
const RUN_IT: &str = "\
zpty -w -n w $'\\r'
sleep 3
";

// ═══════════════════════════════════════════════════════════════════════
// Default completion — no compinit
// ═══════════════════════════════════════════════════════════════════════

/// TAB on a unique directory appends `/`, and an ordinary letter typed
/// next lands AFTER it. Pinning `RustroverProjects/M` rather than merely
/// "contains M" is what separates this from the bug: a shell that drops
/// the suffix draws `RustroverProjectsM`, which also contains an M.
#[test]
fn a_letter_after_a_completed_slash_keeps_the_slash() {
    let driver = format!(
        "{}
zpty -w -n w 'print Rustrover'
sleep 1
zpty -w -n w $'\\t'
sleep 3
zpty -w -n w 'M'
sleep 2
{RUN_IT}
{DRAIN}
if [[ $all == *'RustroverProjects/M'* ]]; then print \"K=yes\"; else print \"K=no\"; fi
",
        open_in_fixture()
    );
    assert_same_verdict(
        &driver,
        "K",
        "a letter typed after a completed directory slash kept the slash",
    );
}

/// The other half of the contract, which is what makes the case above a
/// statement about the removal SET rather than about removal being broken
/// altogether: a space IS in `$ZLE_REMOVE_SUFFIX_CHARS`, so it takes the
/// slash with it and the line reads `f RustroverProjects ` — no slash.
#[test]
fn a_space_after_a_completed_slash_removes_the_slash() {
    let driver = format!(
        "{}
zpty -w -n w 'print Rustrover'
sleep 1
zpty -w -n w $'\\t'
sleep 3
zpty -w -n w ' zz'
sleep 2
{RUN_IT}
{DRAIN}
if [[ $all == *'RustroverProjects zz'* && $all != *'RustroverProjects/ zz'* ]]; then
  print \"K=yes\"
else
  print \"K=no\"
fi
",
        open_in_fixture()
    );
    assert_same_verdict(
        &driver,
        "K",
        "a space typed after a completed directory slash removed the slash",
    );
}

// ═══════════════════════════════════════════════════════════════════════
// compsys — the same case after compinit, with a real zstyle setup
// ═══════════════════════════════════════════════════════════════════════

/// `compinit -u -D` over the stock function directory only — the inherited
/// `$FPATH` on a developer box holds thousands of completers and takes
/// minutes to scan in a debug build.
#[test]
fn compsys_with_zstyles_keeps_the_slash_before_a_letter() {
    if !stock_fpath_exists() {
        eprintln!("skip: no /usr/share/zsh/*/functions to compinit against");
        return;
    }
    let styles = sq(ZSTYLES);
    let driver = format!(
        "{}
zpty -w w 'fpath=(/usr/share/zsh/*/functions(N))'
zpty -w w 'autoload -Uz compinit; compinit -u -D'
sleep 20
zpty -w w {styles}
sleep 2
zpty -w -n w 'print Rustrover'
sleep 1
zpty -w -n w $'\\t'
sleep 4
zpty -w -n w 'M'
sleep 2
{RUN_IT}
{DRAIN}
if [[ $all == *'RustroverProjects/M'* ]]; then print \"K=yes\"; else print \"K=no\"; fi
",
        open_in_fixture()
    );
    assert_same_verdict(
        &driver,
        "K",
        "compsys with zstyles kept the slash before a typed letter",
    );
}

// ═══════════════════════════════════════════════════════════════════════
// Buffer truth — `$BUFFER` read from inside a widget
// ═══════════════════════════════════════════════════════════════════════

/// The same case judged on the LINE EDITOR'S OWN STATE rather than on
/// the transcript. A redraw can print `RustroverProjects/M` while the
/// buffer holds something else, and screen matching cannot tell the
/// difference; `$BUFFER` can.
#[test]
fn the_buffer_itself_keeps_the_slash_before_a_letter() {
    let driver = format!(
        "{}
{DUMP_WIDGET}
sleep 1
zpty -w -n w 'print Rustrover'
sleep 1
zpty -w -n w $'\\t'
sleep 3
zpty -w -n w 'M'
sleep 2
{DUMP_KEY}
{DRAIN}
",
        open_in_fixture()
    );
    assert_same_dump(
        &driver,
        "the buffer after typing a letter on a completed directory slash",
    );
}

/// The suffix is still on the line when the NEXT widget is a user
/// widget, because a `zle -N` widget does not tear it down.
///
/// `execzlefunc` runs `removesuffix()` / `fixsuffix()` inside the
/// `(w->flags & (WIDGET_INT|WIDGET_NCOMP))` branch only
/// (Src/Zle/zle_main.c:1449 + 1468-1473). A widget made by `zle -N`
/// carries flags `0` (zle_thingy.c:588) and takes the shell-function
/// branch at zle_main.c:1501, which does neither — so `dumpbuf`, fired
/// straight after TAB with nothing typed in between, sees the slash
/// `do_single` appended. Running the teardown for user widgets too
/// deleted the character from the line before the widget could see it,
/// and `$BUFFER` came back one character short.
#[test]
fn a_user_widget_fired_straight_after_tab_still_sees_the_slash() {
    let driver = format!(
        "{}
{DUMP_WIDGET}
sleep 1
zpty -w -n w 'print Rustrover'
sleep 1
zpty -w -n w $'\\t'
sleep 3
{DUMP_KEY}
{DRAIN}
",
        open_in_fixture()
    );
    assert_same_dump(
        &driver,
        "the buffer a user widget sees immediately after a directory completion",
    );
}

/// …and the slash DOES go when that user widget calls an internal
/// widget, because the inner `zle end-of-line` reaches `execzlefunc`
/// through `bin_zle_call` and takes the `WIDGET_INT` branch that does
/// run `removesuffix()`.
///
/// This is the wrapper shape plugins use (`w() { zle some-widget }`),
/// and it is the reason the teardown belongs in the branch rather than
/// in the key loop: put it only in the key loop and it fires for the
/// outer user widget that should skip it; leave it out of `execzlefunc`
/// and it never fires for the inner internal widget that should run it.
#[test]
fn an_internal_widget_called_from_a_user_widget_removes_the_slash() {
    let driver = format!(
        "{}
{DUMP_WIDGET}
zpty -w w 'wrapeol(){{ zle end-of-line }}; zle -N wrapeol; bindkey \"^X^E\" wrapeol'
sleep 1
zpty -w -n w 'print Rustrover'
sleep 1
zpty -w -n w $'\\t'
sleep 3
zpty -w -n w $'\\C-x\\C-e'
sleep 2
{DUMP_KEY}
{DRAIN}
",
        open_in_fixture()
    );
    assert_same_dump(
        &driver,
        "the buffer after a user widget delegated to an internal widget",
    );
}

// ═══════════════════════════════════════════════════════════════════════
// Not covered here: the native line-editor engines
// ═══════════════════════════════════════════════════════════════════════
//
// Every probe above opens its inner shell with `-f`, which unsets RCS —
// and `zle_fx.rs:98-104` gates the native autosuggest / highlight /
// history-search engines on RCS being set, while `zpty_probe::drive` also
// exports `ZSHRS_NATIVE_ZLE_FX=0`. So nothing in this file (or in the rest
// of the parity harness) types into the editor a configured user actually
// uses.
//
// A case that opens the inner shell WITH an rc — a fixture `$ZDOTDIR`
// carrying compinit and the styles above — was written and does not work:
// zsh drives it fine and reports `BUF=[print RustroverProjects/M]`, but the
// zshrs run dumps nothing at all, because for THAT run the outer driver
// shell is zshrs too and its `zpty` port never brings up an inner shell
// started without `-f`. The same inner shell, same binary, same rc, driven
// from a zsh outer shell reaches its prompt and completes correctly — so
// the gap is in the zpty port, not in the editor.
//
// Covering the native engines needs that fixed first; until then, do not
// read a green run of this file as a statement about them.

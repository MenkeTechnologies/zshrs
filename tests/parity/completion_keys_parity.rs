//! TAB-completion parity at the KEYBOARD, judged by what the shell
//! ended up running.
//!
//! The listing itself cannot be compared byte-for-byte between two
//! shells — the two disagree on prompt escapes and shell-integration
//! OSC sequences, which say nothing about completion. So each case
//! either checks that a specific candidate reached the SCREEN, or types
//! Return afterwards and checks that the completed command actually
//! EXECUTED. A completion that inserts the wrong text runs a different
//! command and the marker never appears.
//!
//! Both completion engines are covered, because they are different code
//! paths and only one of them is compsys:
//!
//!   * the DEFAULT completion a shell has before `compinit` — `compctl`
//!     territory, and the path a `-f` shell takes;
//!   * `compsys`, after `compinit -u -D` over the stock function
//!     directory.
//!
//! **Timing is the whole game here.** Every keystroke needs its own
//! settle window: a TAB written while the previous line is still being
//! redrawn is simply dropped, and the case then reports "no listing"
//! for both shells and looks like agreement. A first draft of these
//! probes did exactly that and made an ambiguous completion look like a
//! divergence it is not. The sleeps below are deliberate, and the
//! reference-shell assertion in `assert_same_verdict` is what catches
//! it if they ever stop being enough.
//!
//! Skip pattern: no-ops silently when `zsh` isn't on PATH or when
//! `zsh/zpty` will not load. Harness contract: `zpty_probe`.

#![allow(non_snake_case)]
#![allow(clippy::doc_lazy_continuation)]

use crate::zpty_probe::{assert_same_verdict, sq, DRAIN, OPEN};
use std::path::{Path, PathBuf};

/// A directory holding one unique name and two that share a prefix.
/// Built under the cargo target dir so both shells see the same layout
/// and nothing leaks into the user's tree.
fn fixture_dir() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("parity-completion-fixture");
    let _ = std::fs::create_dir_all(&dir);
    for name in ["fxunique_zzz", "fxa1", "fxa2"] {
        let _ = std::fs::File::create(dir.join(name));
    }
    dir
}

/// Open a pty, land in the fixture directory, and silence the bell so a
/// failed completion cannot be mistaken for output.
fn open_in_fixture() -> String {
    let dir = fixture_dir();
    format!(
        "{OPEN}
zpty -w w 'cd {}'
zpty -w w 'unsetopt beep'
sleep 1
",
        dir.display()
    )
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

// ═══════════════════════════════════════════════════════════════════════
// Default completion — no compinit, the path a `-f` shell takes
// ═══════════════════════════════════════════════════════════════════════

/// One candidate matches, so TAB has to insert the whole name. Return
/// then runs it, and `print` echoes the completed word back.
///
/// The verdict counts TWO occurrences on purpose: one for the text TAB
/// inserted into the line, one for the output of the command that ran.
/// A shell that completed correctly but whose Return did nothing scores
/// one and fails.
#[test]
fn tab_completes_a_unique_filename_and_runs_it() {
    let driver = format!(
        "{}
zpty -w -n w 'print fxuniq'
sleep 1
zpty -w -n w $'\\t'
sleep 2
zpty -w -n w $'\\r'
sleep 2
{DRAIN}
integer n=0
local rest=\"$all\"
while [[ $rest == *fxunique_zzz* ]]; do (( n++ )); rest=\"${{rest#*fxunique_zzz}}\"; done
if (( n >= 2 )); then print \"K=yes\"; else print \"K=no\"; fi
",
        open_in_fixture()
    );
    assert_same_verdict(&driver, "K", "TAB completed a unique filename and ran it");
}

/// Two candidates share the typed prefix, so there is nothing to
/// insert and TAB must LIST them instead.
#[test]
fn tab_lists_both_candidates_when_ambiguous() {
    let driver = format!(
        "{}
zpty -w -n w 'print fxa'
sleep 1
zpty -w -n w $'\\t'
sleep 2
zpty -w -n w $'\\t'
sleep 2
zpty -w -n w $'\\r'
sleep 2
{DRAIN}
if [[ $all == *fxa1* && $all == *fxa2* ]]; then print \"K=yes\"; else print \"K=no\"; fi
",
        open_in_fixture()
    );
    assert_same_verdict(&driver, "K", "an ambiguous TAB listed both candidates");
}

/// A second TAB on the same ambiguous prefix cycles into the first
/// candidate (AUTO_MENU), so the line becomes `print fxa1`. Pinning the
/// buffer text rather than the output distinguishes "menu inserted the
/// first candidate" from "menu inserted the second".
#[test]
fn a_second_tab_menu_completes_the_first_candidate() {
    let driver = format!(
        "{}
zpty -w -n w 'print fxa'
sleep 2
zpty -w -n w $'\\t'
sleep 4
zpty -w -n w $'\\t'
sleep 4
zpty -w -n w $'\\r'
sleep 3
{DRAIN}
if [[ $all == *'print fxa1'* ]]; then print \"K=yes\"; else print \"K=no\"; fi
",
        open_in_fixture()
    );
    assert_same_verdict(
        &driver,
        "K",
        "a second TAB menu-completed the first candidate",
    );
}

// ═══════════════════════════════════════════════════════════════════════
// compsys — the same case after `compinit`
// ═══════════════════════════════════════════════════════════════════════

/// The compsys path reaches the same listing through `_main_complete` →
/// `_complete` → `_files`, and with LIST_AMBIGUOUS there is still
/// nothing to insert, so the line is left alone.
///
/// `compinit -u -D` over the stock function directory only — the
/// inherited `$FPATH` on a developer box can hold thousands of
/// completers and takes minutes to scan in a debug build. This case is
/// the slow one in the file (~35s per shell), which is the price of
/// exercising the engine the user's completions actually run through.
#[test]
fn compsys_lists_both_candidates_when_ambiguous() {
    if !stock_fpath_exists() {
        eprintln!("skip: no /usr/share/zsh/*/functions to compinit against");
        return;
    }
    let driver = format!(
        "{}
zpty -w w 'fpath=(/usr/share/zsh/*/functions(N))'
zpty -w w 'autoload -Uz compinit; compinit -u -D'
sleep 20
zpty -w -n w 'print fxa'
sleep 1
zpty -w -n w $'\\t'
sleep 3
zpty -w -n w $'\\r'
sleep 2
{DRAIN}
if [[ $all == *fxa1* && $all == *fxa2* ]]; then print \"K=yes\"; else print \"K=no\"; fi
",
        open_in_fixture()
    );
    assert_same_verdict(
        &driver,
        "K",
        "compsys listed both candidates for an ambiguous prefix",
    );
}


// ═══════════════════════════════════════════════════════════════════════
// Described completions — `_describe`, `compadd -d`, and the
// `descriptions` format style
// ═══════════════════════════════════════════════════════════════════════

/// A compsys session with a custom completer installed for `mytest`,
/// which is then TAB-completed. `^U` clears the line afterwards so the
/// session ends on an empty command rather than running whatever the
/// completion inserted.
///
/// `compinit` runs over `/usr/share/zsh/*/functions` only — see the
/// note on the ambiguous-prefix case above for why the inherited
/// `$FPATH` is unusable here.
fn compsys_driver(setup: &str, needle: &str) -> String {
    let setup_q = sq(setup);
    let needle_q = sq(needle);
    format!(
        "{OPEN}
zpty -w w 'unsetopt beep'
zpty -w w 'fpath=(/usr/share/zsh/*/functions(N))'
zpty -w w 'autoload -Uz compinit; compinit -u -D'
sleep 20
zpty -w w {setup_q}
sleep 2
zpty -w -n w 'mytest '
sleep 2
zpty -w -n w $'\\t'
sleep 3
zpty -w -n w $'\\C-u'
sleep 1
zpty -w -n w $'\\r'
sleep 3
{DRAIN}
local needle={needle_q}
if [[ $all == *${{~needle}}* ]]; then print \"K=yes\"; else print \"K=no\"; fi
"
    )
}

const DESCRIBE: &str =
    r#"_mytest(){ _describe 'thing' '(alpha:first beta:second)' }; compdef _mytest mytest"#;

/// `_describe` is how most completers present a set of choices. Both
/// candidates have to reach the listing, in order.
#[test]
fn describe_lists_every_candidate() {
    if !stock_fpath_exists() {
        eprintln!("skip: no /usr/share/zsh/*/functions to compinit against");
        return;
    }
    assert_same_verdict(
        &compsys_driver(DESCRIBE, "alpha*beta"),
        "K",
        "_describe listed both candidates",
    );
}

/// …and the DESCRIPTION half of each `name:description` pair has to be
/// displayed next to it. A shell that lists the names but drops the
/// descriptions passes the case above and fails this one.
#[test]
fn describe_shows_the_description_text() {
    if !stock_fpath_exists() {
        eprintln!("skip: no /usr/share/zsh/*/functions to compinit against");
        return;
    }
    assert_same_verdict(
        &compsys_driver(DESCRIBE, "first"),
        "K",
        "_describe displayed the description text",
    );
}

/// The `descriptions` format style puts a header above each group —
/// the `-<<external command>>-` style banner a configured setup shows.
/// `%d` is substituted with the group's description.
#[test]
fn the_descriptions_format_style_draws_a_group_header() {
    if !stock_fpath_exists() {
        eprintln!("skip: no /usr/share/zsh/*/functions to compinit against");
        return;
    }
    let setup = format!(
        "{DESCRIBE}; zstyle ':completion:*:descriptions' format 'HDRZZ %d'"
    );
    assert_same_verdict(
        &compsys_driver(&setup, "HDRZZ"),
        "K",
        "the descriptions format style drew a group header",
    );
}

/// `compadd -d` supplies a display array PARALLEL to the match array:
/// the shell lists the display strings while completing the matches.
/// `_describe` is built on it, but plenty of completers call it directly,
/// and the two arrays going out of step is a whole bug class.
#[test]
fn compadd_d_lists_the_parallel_display_strings() {
    if !stock_fpath_exists() {
        eprintln!("skip: no /usr/share/zsh/*/functions to compinit against");
        return;
    }
    let setup = r#"_mytest(){ local -a m d; m=(k1 k2); d=('k1 -- DSCA' 'k2 -- DSCB'); compadd -d d -a m }; compdef _mytest mytest"#;
    assert_same_verdict(
        &compsys_driver(setup, "DSCA*DSCB"),
        "K",
        "compadd -d listed the parallel display strings",
    );
}

/// Guard for the fixture itself: if these three files ever stop
/// existing the completion cases above would all report "no" on both
/// sides and pass as false agreement.
#[test]
fn the_fixture_holds_the_three_expected_names() {
    let dir = fixture_dir();
    for name in ["fxunique_zzz", "fxa1", "fxa2"] {
        assert!(
            Path::new(&dir).join(name).exists(),
            "fixture file {name} missing from {}",
            dir.display()
        );
    }
}

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

use crate::zpty_probe::{assert_same_dump, assert_same_verdict, sq, DRAIN, OPEN, OPEN_PUMPED};
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
// Mid-word TAB — where the cursor is when completion starts
// ═══════════════════════════════════════════════════════════════════════

/// A session whose only completion for `print` is a fixed two-word list,
/// so the candidate set cannot drift with the filesystem the way the
/// `fx*` fixture files can. `setup` runs last, which is where a case
/// turns `COMPLETE_IN_WORD` on.
///
/// The keys are: type `print fxalp`, `^B` back over the `p`, TAB, Return.
/// `bindkey -e` is explicit because `$EDITOR` reaching the inner shell
/// would otherwise put it in vi mode and `^B` would page instead of
/// stepping back one character.
///
/// The setup ends by printing `MARKZZ`, and the verdict throws away
/// everything up to the LAST one. The inner shell echoes back every
/// setup line it is handed, so the `compctl` line puts `fxalpha_zzz` in
/// the transcript before the completion ever runs — a verdict looking at
/// the whole transcript sees the candidate whether or not TAB inserted
/// it, which is exactly how the suffix case below first passed on a
/// shell that had not completed anything.
fn midword_driver(setup: &str, verdict: &str) -> String {
    let setup_q = sq(setup);
    format!(
        "{OPEN}
zpty -w w 'unsetopt beep'
zpty -w w 'bindkey -e'
zpty -w w 'compctl -k \"(fxalpha_zzz fxbravo_zzz)\" print'
zpty -w w {setup_q}
zpty -w w 'print MARKZZ'
sleep 2
zpty -w -n w 'print fxalp'
sleep 2
zpty -w -n w $'\\C-b'
sleep 1
zpty -w -n w $'\\t'
sleep 3
zpty -w -n w $'\\r'
sleep 3
{DRAIN}
all=\"${{all##*MARKZZ}}\"
{verdict}
"
    )
}

/// With `COMPLETE_IN_WORD` unset — the default — a TAB struck with the
/// cursor inside a word completes the WHOLE word: the shell first moves
/// the cursor to the end of it. So `print fxal|p` completes against
/// `fxalp`, matches `fxalpha_zzz` uniquely, and inserts it.
///
/// Judged on the OUTPUT, not on the line: ZLE redraws an insertion as a
/// backspace plus the tail it appends, so the transcript carries
/// `print fxalp\x08ha_zzz` and never the whole candidate. `fxalpha_zzz`
/// can therefore only appear because `print` ran with the completed
/// word. `fxbravo_zzz` must NOT appear — that would mean the shell
/// listed both candidates instead of inserting the unique one.
///
/// zshrs scored zero here. `makecomplistflags` had been ported from
/// `Src/Zle/compctl.c:3070` onward and the cursor-to-end-of-word step
/// three lines above it (c:3066-3068) was missing, so `offs` still
/// pointed at the cursor, the word was split there, and every candidate
/// had to end in `p` to match. Nothing did, and the line was left alone.
#[test]
fn a_midword_tab_completes_the_whole_word_by_default() {
    let verdict = "if [[ $all == *fxalpha_zzz* && $all != *fxbravo_zzz* ]]; then
  print \"K=yes\"
else
  print \"K=no\"
fi";
    assert_same_verdict(
        &midword_driver("true", verdict),
        "K",
        "a mid-word TAB completed the whole word with COMPLETE_IN_WORD unset",
    );
}

/// The other half of the same switch, and the guard that keeps the case
/// above from being "passed" by moving the cursor unconditionally. With
/// `COMPLETE_IN_WORD` SET the cursor stays put, so the word is split
/// into the prefix `fxal` and the suffix `p` and a candidate has to
/// match both ends. Neither `fxalpha_zzz` nor `fxbravo_zzz` ends in `p`,
/// so there is nothing to insert and the line is still `print fxalp`.
#[test]
fn a_midword_tab_respects_a_suffix_under_complete_in_word() {
    let verdict = "if [[ $all == *'print fxalp'* && $all != *fxalpha_zzz* ]]; then
  print \"K=yes\"
else
  print \"K=no\"
fi";
    assert_same_verdict(
        &midword_driver("setopt completeinword", verdict),
        "K",
        "a mid-word TAB kept the suffix under COMPLETE_IN_WORD",
    );
}

/// A candidate is QUOTED before it is matched, and the quoting counts.
///
/// The line is `zzrun |-` — cursor inside the word, `COMPLETE_IN_WORD` set,
/// so `$PREFIX` is empty and `$SUFFIX` is `-`. The `r:|=*` match spec is
/// what lets a candidate carry text the word does not have at its right
/// edge, which is the only reason `-a` matches `-` at all. `-?` reaches the
/// matcher as `-\?`, because `compadd` runs every candidate through
/// `multiquote` first (compmatch.c:1172), and the backslash stops it
/// matching. So exactly one candidate survives, TAB inserts it, and the
/// function echoes what it was given.
///
/// zshrs kept both. `match_str`'s backslash skip (compmatch.c:1001) compares
/// `w[ind+1]` against `l[0]`; the port compared it against `l[ind]`, which is
/// the same byte only on the prefix pass — on the suffix pass the two
/// pointers walk backwards and `l[0]` is the line byte just consumed. With
/// two matches instead of one there was nothing unambiguous to insert, the
/// line stayed `zzrun -`, and `lsof -i :22 |-<TAB>` asked "do you wish to see
/// all 114 possibilities" where zsh lists 38.
#[test]
fn a_quoted_candidate_does_not_match_through_its_backslash() {
    const SETUP: &str = r#"zzrun() { print "RAN:$1" }; _zzc() { compadd -M 'r:|=*' - '-a' '-?' }; zle -C zzc complete-word _zzc; bindkey '^I' zzc"#;
    let setup_q = sq(SETUP);
    let driver = format!(
        "{OPEN}
zpty -w w 'unsetopt beep'
zpty -w w 'bindkey -e'
zpty -w w 'setopt completeinword'
zpty -w w {setup_q}
sleep 2
zpty -w -n w 'zzrun -'
sleep 2
zpty -w -n w $'\\C-b'
sleep 1
zpty -w -n w $'\\t'
sleep 3
zpty -w -n w $'\\r'
sleep 3
{DRAIN}
if [[ $all == *'RAN:-a'* ]]; then print \"K=yes\"; else print \"K=no\"; fi
"
    );
    assert_same_verdict(
        &driver,
        "K",
        "a backslash-quoted candidate was rejected and the plain one inserted",
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

/// `compadd -k NAME` must enumerate an association's keys in the same
/// order `${(k)NAME}` does.
///
/// That agreement is what makes `compadd -k NAME -d DISP` work at all:
/// the caller builds `DISP` from `${(kv)NAME}` — the shape
/// `Completion/Unix/Command/_git`'s `__git_diff_filters` uses — and the
/// two arrays are then paired ELEMENT-FOR-ELEMENT. C gets it for free
/// because both sides reach the SAME scan: `compadd -k` goes
/// `get_data_arr` → `fetchvalue` → `getarrvalue` → `getvaluearr` →
/// `paramvalarr` (`Src/params.c:735-736`), and `${(k)}` reaches
/// `paramvalarr` too, so each walks `scanhashtable`'s hash-bucket order
/// (`Src/params.c:718`, `Src/hashtable.c:426`).
///
/// zshrs instead read the keys straight out of its insertion-ordered
/// storage map, so `filters=(A added C copied b 'pairing broken')`
/// enumerated `A C b` where zsh scans `b A C`. Every key then drew
/// ANOTHER key's display string, and since `matchcmp` sorts by `disp`
/// whenever one is present (`Src/Zle/compcore.c:3179-3191`), two
/// otherwise-equal matches carrying mismatched `disp` stopped being
/// adjacent — so the consecutive-run dedup at `compcore.c:3271`, which
/// only collapses ADJACENT equals, left the duplicate behind and
/// `git diff --diff-filter=<TAB>` listed `b -- pairing broken` twice.
///
/// The completer dumps BOTH orders rather than a listing: the order is
/// the actual invariant, and two shells legitimately differ on how they
/// redraw a list. `A`/`C`/`b` are chosen because their bucket order
/// (`b A C`) differs from their insertion order (`A C b`), so a shell
/// that confuses the two cannot accidentally agree.
///
/// `OPEN_PUMPED` rather than `OPEN` + blind sleeps: `compinit` over the
/// stock function directory produces enough output to fill the inner
/// shell's pty buffer, and a shell blocked on write never reaches the
/// TAB — the reference then dumps NOTHING and the case fails as a
/// broken probe rather than a divergence. Draining between writes is
/// what keeps it unblocked (see the note on `OPEN_PUMPED`).
#[test]
fn compadd_k_enumerates_keys_in_scan_order() {
    if !stock_fpath_exists() {
        eprintln!("skip: no /usr/share/zsh/*/functions to compinit against");
        return;
    }
    let setup = sq(concat!(
        r#"_mytest(){ local -a outk; typeset -A filters; "#,
        r#"filters=( A added C copied b 'pairing broken' ); "#,
        r#"compadd -O outk -k filters; "#,
        r#"print -r -- "KEYS=[${(j: :)outk}] SCAN=[${(j: :)${(k)filters}}]" >! $OUTFILE; "#,
        r#"compadd -k filters }; compdef _mytest mytest"#,
    ));
    let driver = format!(
        "{OPEN_PUMPED}
zpty -w w 'fpath=(/usr/share/zsh/*/functions(N))'; pump
zpty -w w 'autoload -Uz compinit; compinit -u -D'; pump; pump
zpty -w w {setup}; pump
zpty -w -n w 'mytest '; pump
zpty -w -n w $'\\t'; pump; pump
zpty -w -n w $'\\C-u'; pump
zpty -w -n w $'\\r'; pump
zpty -d w 2>/dev/null
"
    );
    assert_same_dump(
        &driver,
        "compadd -k enumerated the assoc's keys in ${(k)} scan order",
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

// ═══════════════════════════════════════════════════════════════════════
// Command-word resolution inside a completion action
// ═══════════════════════════════════════════════════════════════════════

/// A `$fpath` directory holding ONE `_`-prefixed file that carries no
/// `#compdef` and no `#autoload` tag line.
///
/// `compinit` reads the first line of every `_`-file it finds and acts on
/// it only through `case $_i_tag in (\#compdef) compdef -na … ;;
/// (\#autoload) autoload -rUz … ;; esac` (`compinit` sh:533-548), so an
/// untagged file registers NOTHING: after `compinit` both shells report
/// `${+functions[_zzuntagged_action]}` as 0, while a tagged stock
/// completer such as `_cat` reports 1.
///
/// The body would `compadd` a marker if it were ever run. Nothing in
/// either shell should run it, and the marker is what says so when this
/// case fails.
fn untagged_fpath_fixture() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("parity-untagged-fpath-fixture");
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(
        dir.join("_zzuntagged_action"),
        "# not a compdef tag line\n_zzuntagged_action(){ compadd zzfromtheuntaggedfile }\n",
    );
    dir
}

/// An action naming something that is not a command must be DIAGNOSED,
/// not silently run off `$fpath`.
///
/// `execcmd` resolves a completion action's command word the same way it
/// resolves any other: `if (!(cflags & (BINF_BUILTIN | BINF_COMMAND)) &&
/// (hn = shfunctab->getnode(shfunctab, cmdarg))) { is_shfunc = 1; break; }`
/// (`Src/exec.c:3105-3109`, repeated at `:3484-3488`), then the builtin
/// table, then `$PATH`, and only then `zerr("command not found: %s",
/// arg0)` (`Src/exec.c:903`). `getnode` finds a node ALREADY in
/// `shfunctab` — a definition, or the autoload stub `autoload` /
/// `compinit` put there. A FILE in `$fpath` is not a command in zsh.
///
/// zshrs autoloaded it anyway: `src/vm_helper.rs:4816-4845` takes any
/// `_`-prefixed name with no shfunctab entry, probes `$fpath` with
/// `getfpfunc`, and on a hit runs `autoload -rUz -- NAME` and executes
/// the file. The gate keys on the FILENAME where `compinit` keys on the
/// TAG LINE, so an untagged `_`-file — registered by neither shell — ran
/// in zshrs and the diagnostic zsh prints was never reached.
///
/// Measured on a host whose `~/.zpwr/autoload/comp_utils` holds an
/// untagged `__fasd_files_comp` that the user's `_files` sh:167-169
/// offers as an `_alternative` action, on `cat zzzzqqq<TAB>`:
///
/// ```text
/// zsh    _alternative:71: command not found: __fasd_files_comp
///        _alternative:71: command not found: __fasd_dirs_comp
/// zshrs  (nothing at all)
/// ```
///
/// The verdict is the DIAGNOSTIC, not the absence of matches: a shell
/// that merely completed nothing would still be swallowing the error,
/// which is the bug. `_alternative` is the caller because it is where
/// this was found, and its sh:71 arm is the one that names the action as
/// a command word.
///
/// `OPEN_PUMPED` rather than `OPEN` + blind sleeps, for the reason its
/// own note gives: `compinit` over the 1203-file stock tree fills the
/// inner shell's pty buffer, and a shell blocked on write never reaches
/// the later writes. Measured with the sleeping form, the reference
/// shell's transcript ended at the `compdef` line — the `mytest ` write
/// and the TAB after it were dropped, so zsh scored `K=no` and
/// `assert_same_verdict` failed the case as a broken probe rather than
/// reporting a divergence that was not there.
#[test]
fn an_action_naming_an_untagged_fpath_file_is_reported_as_not_found() {
    if !stock_fpath_exists() {
        eprintln!("skip: no /usr/share/zsh/*/functions to compinit against");
        return;
    }
    let setup = sq(r#"_mytest(){ _alternative 'x:x:_zzuntagged_action' }; compdef _mytest mytest"#);
    let driver = format!(
        "{OPEN_PUMPED}
zpty -w w 'fpath=({fixture} /usr/share/zsh/*/functions(N))'; pump
zpty -w w 'autoload -Uz compinit; compinit -u -D'; pump; pump
zpty -w w {setup}; pump
zpty -w -n w 'mytest '; pump
zpty -w -n w $'\\t'; pump; pump
zpty -w -n w $'\\C-u'; pump
zpty -d w 2>/dev/null
setopt extended_glob
all=\"${{all//$'\\e'\\[[0-9;?]#[a-zA-Z]/}}\"
if [[ $all == *'command not found: _zzuntagged_action'* ]]; then print \"K=yes\"; else print \"K=no\"; fi
",
        fixture = untagged_fpath_fixture().display(),
    );
    assert_same_verdict(
        &driver,
        "K",
        "an action naming an untagged $fpath file was reported as not found",
    );
}

/// `compinit` through an `$fpath` DIGEST: compinit runs `autoload -rUz`
/// for `#autoload` files and `compdef -na` completers (Completion/compinit
/// sh:333, sh:540), and `-r` makes check_autoload look each name up with
/// getfpfunc (Src/builtin.c:3195-3226) → try_dump_file on `<dir>.zwc`
/// (Src/parse.c:3746-3789) → dump_find_func's in-place name scan
/// (c:3167-3176). That scan was rewritten for speed, so this pins what it
/// must still produce: the same `$_comps` as zsh and the same autoload stub.
///
/// zshrs additionally registers the completers it bundles (the repo's
/// `completions/` directory), so extras are allowed only when their value is
/// one of those files.
#[test]
fn compinit_through_a_zwc_digest_registers_the_same_comps() {
    use std::process::Command;
    if !crate::zpty_probe::zsh_available() {
        eprintln!("skip: zsh not found");
        return;
    }
    let zsh = crate::zpty_probe::zsh_path();
    // zsh's OWN default fpath (FPATH removed from its environment), and the
    // first directory in it that holds `compinit`. A glob qualifier inside a
    // parameter expansion does not glob, so this is a plain loop.
    let stock = Command::new(zsh)
        .args([
            "-fc",
            "for d in $fpath; do [[ -r $d/compinit ]] && { print -r -- $d; break }; done",
        ])
        .env_remove("FPATH")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    if stock.is_empty() {
        eprintln!("skip: no stock compinit on zsh's fpath");
        return;
    }
    let tmp = tempfile::TempDir::new().expect("tmp");
    let fp = tmp.path().join("fp");
    std::fs::create_dir_all(&fp).expect("mkdir fp");
    std::fs::write(fp.join("_zzfoo"), "#compdef zzfoo zzfoo2\n_message foo\n").expect("write");
    std::fs::write(fp.join("_zzbar"), "#compdef zzbar\n_message bar\n").expect("write");
    std::fs::write(fp.join("_zzauto"), "#autoload\nprint auto\n").expect("write");
    // The digest sits next to the directory, as `<dir>.zwc`, and is written
    // after the sources so try_dump_file's mtime test selects it.
    let built = Command::new(zsh)
        .args(["-fc", "zcompile fp.zwc fp/_zzfoo fp/_zzbar fp/_zzauto"])
        .current_dir(tmp.path())
        .status()
        .expect("zcompile");
    assert!(built.success(), "zsh could not build the fixture digest");
    let fpath = format!("{}:{}", fp.display(), stock);
    let script = "autoload -Uz compinit; compinit -u -D; \
                  print -rl -- ${(kv)_comps}; print -r -- ===DEF; \
                  print -r -- \"$functions[_zzauto]\"";
    let run = |shell: &Path, zshrs: bool| -> (std::collections::BTreeSet<(String, String)>, String) {
        let mut cmd = Command::new(shell);
        if zshrs {
            cmd.arg("--zsh");
        }
        let out = cmd
            .args(["-f", "-c", script])
            .current_dir(tmp.path())
            .env("FPATH", &fpath)
            .env_remove("ZSHRS_CACHE")
            .output()
            .expect("run shell");
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        let (comps, def) = text.split_once("===DEF\n").unwrap_or((&text, ""));
        let lines: Vec<&str> = comps.lines().collect();
        let pairs = lines
            .chunks(2)
            .filter(|c| c.len() == 2)
            .map(|c| (c[0].to_string(), c[1].to_string()))
            .collect();
        (pairs, def.to_string())
    };
    let (z_pairs, z_def) = run(Path::new(zsh), false);
    let (r_pairs, r_def) = run(&crate::zpty_probe::zshrs_bin(), true);

    // Fixture guard: the digest's completers really were registered by zsh.
    for (k, v) in [("zzfoo", "_zzfoo"), ("zzfoo2", "_zzfoo"), ("zzbar", "_zzbar")] {
        assert!(
            z_pairs.contains(&(k.to_string(), v.to_string())),
            "reference zsh did not register {k} → {v}; the fixture is broken"
        );
    }
    let missing: Vec<_> = z_pairs.difference(&r_pairs).collect();
    assert!(missing.is_empty(), "zshrs is missing zsh's _comps pairs: {missing:?}");
    let bundled = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("completions");
    let unexplained: Vec<_> = r_pairs
        .difference(&z_pairs)
        .filter(|(_, v)| !bundled.join(v).exists())
        .collect();
    assert!(
        unexplained.is_empty(),
        "zshrs registered _comps pairs zsh did not, and they are not bundled completers: {unexplained:?}"
    );
    assert_eq!(z_def, r_def, "the `autoload -rUz` stub for _zzauto differs");
}

/// A `zerr` inside a completion widget's own function must NOT abort the
/// edit line. The widget is `zle -C`, so there is no `_main_complete` and no
/// `eval` above the error to swallow it; what clears ERRFLAG_ERROR in zsh is
/// docomplete's `zcontext_restore()` (Src/Zle/zle_tricky.c:873), whose
/// parse_context_restore ends in `errflag &= ~ERRFLAG_ERROR`
/// (Src/parse.c:354). Without that, zlecore's `!errflag` gate
/// (Src/Zle/zle_main.c:1128) ended the line: the typed `Z` then ran alone.
///
/// Verdict: the command that runs after Return is `print KEPTZ`, so its
/// output line `KEPTZ` appears only when the buffer survived the widget.
#[test]
fn a_zerr_in_a_completion_widget_keeps_the_edit_line() {
    let driver = format!(
        "{OPEN}
zpty -w w 'unsetopt beep'
zpty -w w 'zmodload zsh/complete'
zpty -w w '_kwf() {{ readonly RO=1; RO=2; compadd x }}'
zpty -w w 'zle -C _kw list-choices _kwf'
zpty -w w 'bindkey \"^Xw\" _kw'
sleep 1
zpty -w -n w 'print KEPT'
sleep 1
zpty -w -n w $'\\C-xw'
sleep 2
zpty -w -n w $'Z\\r'
sleep 2
{DRAIN}
if [[ $all == *$'\\n'KEPTZ* ]]; then print \"K=yes\"; else print \"K=no\"; fi
"
    );
    assert_same_verdict(&driver, "K", "the edit line survived a zerr inside a zle -C widget");
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

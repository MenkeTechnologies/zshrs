//! `compctl` file completion (`-f`, `-/`, `-g`) at the keyboard, judged by
//! what the completed command actually ran with.
//!
//! The fixture directory holds `fxalpha.txt`, `fxalpine.rs`, `fxbeta.txt`
//! and the directory `fxadir`. A function `fxcmd` prints `GOT:<arg>:<n>`
//! where `<n>` is computed by `$(( 6*7 ))` at run time, so the needle can
//! only reach the screen when the TAB inserted exactly the expected
//! candidate and Return ran the line — the typed text and the function
//! source only ever contain `$(( 6*7 ))`. A completion that is ambiguous,
//! or that offers the wrong file class, leaves the prefix in place and the
//! needle never appears.
//!
//! Harness contract: `zpty_probe`.

use crate::zpty_probe::{assert_same_verdict, sq, DRAIN, OPEN};

/// One session in a fresh fixture directory: `compctl_spec` is the flag
/// list given to `compctl ... fxcmd`, `typed` is the word typed after
/// `fxcmd `, and `needle` must be on screen afterwards.
fn driver(compctl_spec: &str, typed: &str, needle: &str) -> String {
    format!(
        "d=$(mktemp -d)
mkdir $d/fxadir
: > $d/fxalpha.txt
: > $d/fxalpine.rs
: > $d/fxbeta.txt
{OPEN}
zpty -w w 'unsetopt beep'
zpty -w w 'bindkey -e'
zpty -w w \"cd $d\"
zpty -w w 'fxcmd(){{ print \"GOT:$1:$(( 6*7 ))\" }}'
zpty -w w \"compctl {compctl_spec} fxcmd\"
sleep 2
zpty -w -n w 'fxcmd {typed}'
sleep 2
zpty -w -n w $'\\t'
sleep 3
zpty -w -n w $'\\r'
sleep 3
{DRAIN}
command rm -rf $d
if [[ $all == *'{needle}'* ]]; then print \"K=yes\"; else print \"K=no\"; fi
"
    )
}

/// `compctl -f`: every file class is offered, and `fxbe` is a unique
/// prefix, so TAB completes it to `fxbeta.txt`.
#[test]
fn compctl_f_completes_a_unique_file_prefix() {
    assert_same_verdict(
        &driver("-f", "fxbe", "GOT:fxbeta.txt:42"),
        "K",
        "`compctl -f` completed `fxbe` to fxbeta.txt",
    );
}

/// `compctl -/`: directories only. `fxa` is a prefix of two plain files
/// and one directory; only the directory may be offered, so the completion
/// is unique (`gen_matches_files(1, 0, 0)`) and gets its trailing slash.
#[test]
fn compctl_slash_offers_only_directories() {
    assert_same_verdict(
        &driver("-/", "fxa", "GOT:fxadir/:42"),
        "K",
        "`compctl -/` completed `fxa` to the directory fxadir/",
    );
}

/// `compctl -g '*.txt'`: the glob restricts the candidates. `fxal` is a
/// prefix of `fxalpha.txt` and `fxalpine.rs`; only the first matches the
/// glob, so the completion is unique.
#[test]
fn compctl_g_restricts_candidates_to_the_glob() {
    assert_same_verdict(
        &driver("-g '*.txt'", "fxal", "GOT:fxalpha.txt:42"),
        "K",
        "`compctl -g '*.txt'` completed `fxal` to fxalpha.txt",
    );
}

/// Like `driver`, but `setup` is a list of full `compctl ...` commands, the
/// completed word is followed by `after_tab` before Return, and `fxcmd`
/// reports the LAST argument it was called with.
fn driver_setup(setup: &[&str], typed: &str, after_tab: &str, needle: &str) -> String {
    let compctls: String = setup
        .iter()
        .map(|c| format!("zpty -w w {}\nsleep 1\n", sq(c)))
        .collect();
    let typed_q = sq(&format!("fxcmd {typed}"));
    let after_q = sq(after_tab);
    format!(
        "d=$(mktemp -d)
mkdir $d/fxadir
: > $d/fxalpha.txt
: > $d/fxalpine.rs
: > $d/fxbeta.txt
{OPEN}
zpty -w w 'unsetopt beep'
zpty -w w 'bindkey -e'
zpty -w w \"cd $d\"
zpty -w w 'fxcmd(){{ print \"GOT:$argv[-1]:$(( 6*7 ))\" }}'
{compctls}sleep 1
zpty -w -n w {typed_q}
sleep 2
zpty -w -n w $'\\t'
sleep 3
zpty -w -n w {after_q}
zpty -w -n w $'\\r'
sleep 3
{DRAIN}
command rm -rf $d
if [[ $all == *'{needle}'* ]]; then print \"K=yes\"; else print \"K=no\"; fi
"
    )
}

/// `compctl -f -x 'c[-1,-f]' -/ -- fxcmd`: the extended condition looks at
/// the word BEFORE the cursor word. After `-f` only directories are
/// offered, so `fxa` completes uniquely to `fxadir/` even though two plain
/// files share the prefix.
#[test]
fn compctl_x_condition_on_previous_word_restricts_to_directories() {
    assert_same_verdict(
        &driver_setup(
            &["compctl -f -x 'c[-1,-f]' -/ -- fxcmd"],
            "-f fxa",
            "",
            "GOT:fxadir/:42",
        ),
        "K",
        "`c[-1,-f]` selected `-/` for the word after `-f`",
    );
}

/// The same spec without the `-f` before the cursor word: the condition is
/// false, the plain `-f` applies, and `fxbe` completes to `fxbeta.txt`.
#[test]
fn compctl_x_condition_false_keeps_the_plain_file_spec() {
    assert_same_verdict(
        &driver_setup(
            &["compctl -f -x 'c[-1,-f]' -/ -- fxcmd"],
            "fxbe",
            "",
            "GOT:fxbeta.txt:42",
        ),
        "K",
        "`c[-1,-f]` was false so `-f` completed `fxbe` to fxbeta.txt",
    );
}

/// `compctl -/ -x 'p[1]' -f -- fxcmd`: the position condition holds for the
/// first argument, so the `-f` alternative replaces the directories-only
/// base spec and `fxbe` completes to the plain file `fxbeta.txt`.
#[test]
fn compctl_x_position_condition_true_switches_to_files() {
    assert_same_verdict(
        &driver_setup(
            &["compctl -/ -x 'p[1]' -f -- fxcmd"],
            "fxbe",
            "",
            "GOT:fxbeta.txt:42",
        ),
        "K",
        "`p[1]` held for the first argument so `-f` completed `fxbe`",
    );
}

/// The same spec one word later: `p[1]` is false for the second argument,
/// the directories-only base spec applies and `fxbe` stays uncompleted.
#[test]
fn compctl_x_position_condition_false_keeps_directories_only() {
    assert_same_verdict(
        &driver_setup(
            &["compctl -/ -x 'p[1]' -f -- fxcmd"],
            "-f fxbe",
            "",
            "GOT:fxbeta.txt:42",
        ),
        "K",
        "`p[1]` was false for the second argument so only directories applied",
    );
}

/// `compctl -h fxinner fxcmd`: the quoted word is split on spaces
/// (`sep_comp_string`) and its parts are completed as arguments of
/// `fxinner`, whose `-/` offers directories only. Inside the single quote
/// `fxa` completes uniquely to `fxadir/`; the closing quote typed after the
/// TAB makes the whole quoted string the one argument.
#[test]
fn compctl_h_completes_inside_a_quoted_string_as_the_inner_command() {
    assert_same_verdict(
        &driver_setup(
            &["compctl -/ fxinner", "compctl -h fxinner fxcmd"],
            "'fxa",
            "'",
            "GOT:fxadir/:42",
        ),
        "K",
        "`compctl -h` completed `fxa` inside quotes with fxinner's `-/`",
    );
}

/// Same split, cursor in the SECOND space-separated part of the quoted
/// string: the part before the cursor word is a separate word of the inner
/// command line, so `c[-1,-f]` on the inner command sees `-f` and selects
/// directories.
#[test]
fn compctl_h_second_part_of_a_quoted_string_sees_the_first_as_previous_word() {
    assert_same_verdict(
        &driver_setup(
            &[
                "compctl -f -x 'c[-1,-f]' -/ -- fxinner",
                "compctl -h fxinner fxcmd",
            ],
            "'-f fxa",
            "'",
            "GOT:-f fxadir/:42",
        ),
        "K",
        "the inner word array carried `-f` as the word before the cursor word",
    );
}

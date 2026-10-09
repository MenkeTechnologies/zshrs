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

use crate::zpty_probe::{assert_same_verdict, DRAIN, OPEN};

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

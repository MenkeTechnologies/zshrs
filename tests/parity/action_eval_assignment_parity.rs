//! An action's word-list `eval` assigns even when its status is nonzero.
//!
//! `_alternative` sh:39 (`eval ws\=\( "${action[3,-3]}" \)`) and the
//! `_arguments` sh:453/463 evals turn an action's text into an array. A
//! command substitution that fails inside that list,
//!
//!     ((alpha\:"at `nosuchcmdzz` sub" five\:"b"))
//!
//! prints `(eval):1: command not found: nosuchcmdzz` and makes the eval
//! return 127, but zsh still assigns the array — only a PARSE error leaves
//! the target unassigned (and an `_arguments` action a scalar, whose first
//! character becomes the command word). The shared Rust helper keyed
//! "assigned" off the eval's status, so the failed substitution threw the
//! whole list away and `_alternative` added no matches.
//!
//! The second widget pins the parse-error side the helper must keep:
//! `_arguments:465: command not found: a` for an action that is not a word
//! list. Found by compsys_spec_fuzz seed 9519 case0031.
//!
//! Skip pattern: no-ops silently when `zsh` isn't on PATH, when `zsh/zpty`
//! will not load, or when there is no stock function directory to autoload
//! the helpers from. Harness contract: `zpty_probe`.

#![allow(non_snake_case)]

use crate::zpty_probe::{assert_same_dump, dump_widget, DUMP_KEY_PUMPED, OPEN_PUMPED};

fn stock_fpath_exists() -> bool {
    std::fs::read_dir("/usr/share/zsh")
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .any(|e| e.path().join("functions").is_dir())
        })
        .unwrap_or(false)
}

#[test]
fn failed_cmdsubst_in_an_action_still_assigns_the_word_list() {
    if !stock_fpath_exists() {
        eprintln!("skip: no /usr/share/zsh/*/functions to autoload from");
        return;
    }
    let driver = format!(
        "{OPEN_PUMPED}
zpty -w w 'fpath=(/usr/share/zsh/*/functions(N)); autoload -Uz $^fpath/_*(N:t)'; pump
zpty -w w '_t1() {{ _alternative \"x:y:((alpha\\\\:\\\"at \\`nosuchcmdzz\\` sub\\\" five\\\\:b))\" 2>/dev/null; n1=$compstate[nmatches] }}'; pump
zpty -w w '_t2() {{ _arguments \"1:x:app or factory:fn; env: U):\" 2>$OUTFILE.err; n2=$? }}'; pump
zpty -w w 'zle -C w1 complete-word _t1; zle -C w2 complete-word _t2; bindkey \"^Xa\" w1; bindkey \"^Xb\" w2'; pump
{}
pump
zpty -w -n w 'ea '; pump
zpty -w -n w $'\\C-xb'; pump; pump
zpty -w -n w $'\\C-xa'; pump; pump
{DUMP_KEY_PUMPED}",
        dump_widget(r#""N1=[$n1] ERR=[$(<$OUTFILE.err)] RC=[$n2]""#)
    );
    assert_same_dump(
        &driver,
        "an action eval whose command substitution failed still assigned its words",
    );
}

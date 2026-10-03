//! `compadd -p PPRE -C` when PPRE cannot match the word on the line.
//!
//! `addmatches` (c:Src/Zle/compcore.c:2313-2341) probes the hidden prefix
//! against the typed prefix; when neither is a prefix of the other it sets
//! `*argv = NULL`. That only empties the WORD LIST: the rest of the
//! function still runs, so the `-C` all-match is still added (c:2606-2609),
//! `mnum` moves, and `compadd` returns 0. On the word `a`,
//!
//!     compadd -p , -C      ->  the line becomes `x ` (the empty <all>), status 0
//!
//! The port returned straight out of `addmatches` at that point, skipping
//! the all-match, the explanation, the `-E` dummies and the `-A/-O/-D`
//! arrays: the line stayed `x a` and `compadd` returned 1.
//!
//! Reached through compsys (`compinit`, `compdef`), as compsys_spec_fuzz
//! seed 9513 case0028 found it.
//!
//! Skip pattern: no-ops silently when `zsh` isn't on PATH, when `zsh/zpty`
//! will not load, or when there is no stock function directory to
//! `compinit` against. Harness contract: `zpty_probe`.

#![allow(non_snake_case)]

use crate::zpty_probe::{assert_same_dump, dump_widget, DRAIN, DUMP_KEY, OPEN};

fn stock_fpath_exists() -> bool {
    std::fs::read_dir("/usr/share/zsh")
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .any(|e| e.path().join("functions").is_dir())
        })
        .unwrap_or(false)
}

#[test]
fn all_match_is_added_when_the_hidden_prefix_misses() {
    if !stock_fpath_exists() {
        eprintln!("skip: no /usr/share/zsh/*/functions to compinit against");
        return;
    }
    let driver = format!(
        "{OPEN}
zpty -w w 'unsetopt beep'
zpty -w w 'fpath=(/usr/share/zsh/*/functions(N))'
zpty -w w 'autoload -Uz compinit; compinit -u -D'
sleep 20
zpty -w w '_t() {{ compadd -p , -C; r=$? }}; compdef _t x'
{}
sleep 1
zpty -w -n w 'x a'
sleep 1
zpty -w -n w $'\\t'
sleep 3
{DUMP_KEY}
{DRAIN}
",
        dump_widget(r#""BUF=[$BUFFER] RC=[$r]""#)
    );
    assert_same_dump(
        &driver,
        "compadd -p with a non-matching hidden prefix still added the -C all-match",
    );
}

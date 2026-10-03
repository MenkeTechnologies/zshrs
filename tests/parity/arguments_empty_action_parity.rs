//! `_arguments` with an EMPTY action, reached without `_main_complete`.
//!
//! `_arguments` sh:413 tests `[[ "$action" = \ # ]]`, a glob match under
//! the caller's live options. `\ #` is "any run of spaces" only with
//! EXTENDED_GLOB, which `_comp_setup` turns on — but a `zle -C` widget
//! naming the completer directly (`#compdef -K`, `compdef -k`) never runs
//! `_main_complete`, so under `zsh -f` the pattern is the literal ` #`, an
//! empty action misses it, and sh:463-465 runs `"$action[1]"`: the empty
//! string as a command word. zsh's `execute()` walks `$path`
//! (c:Src/exec.c:820-833), every `dir/` fails EACCES, and c:837 reports
//!
//!     _arguments:465: permission denied:
//!
//! with status 1 out of `_arguments`. The Rust port treated an all-blank
//! action as the `_message` branch regardless of options, printed nothing,
//! and returned the message path's status.
//!
//! Skip pattern: no-ops silently when `zsh` isn't on PATH, when `zsh/zpty`
//! will not load, or when there is no stock function directory to autoload
//! `_arguments`'s helpers from. Harness contract: `zpty_probe`.

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
fn empty_action_without_extendedglob_runs_the_empty_command_word() {
    if !stock_fpath_exists() {
        eprintln!("skip: no /usr/share/zsh/*/functions to autoload from");
        return;
    }
    let driver = format!(
        "{OPEN}
zpty -w w 'unsetopt beep'
zpty -w w 'fpath=(/usr/share/zsh/*/functions(N)); autoload -Uz $^fpath/_*(N:t)'
zpty -w w '_ea() {{ _arguments \"*:n:\" 2>$OUTFILE.err; ea_rc=$? }}'
zpty -w w 'zle -C eaw complete-word _ea; bindkey \"^I\" eaw'
{}
sleep 1
zpty -w -n w 'ea '
sleep 1
zpty -w -n w $'\\t'
sleep 3
{DUMP_KEY}
{DRAIN}
",
        dump_widget(r#""ERR=[$(<$OUTFILE.err)] RC=[$ea_rc]""#)
    );
    assert_same_dump(
        &driver,
        "an empty _arguments action without EXTENDED_GLOB ran the empty command word",
    );
}

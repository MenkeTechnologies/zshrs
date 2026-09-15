//! Menu selection under NO_ALWAYS_LAST_PROMPT.
//!
//! `complistmatches` refuses to start a selection unless
//! `isset(USEZLE) && !termflags && complastprompt && *complastprompt`
//! (c:Src/Zle/complist.c:2031-2033). `complastprompt` is `""` whenever
//! ALWAYS_LAST_PROMPT is unset (c:Src/Zle/compcore.c:325), so a
//! `menu select=long-list` that `_main_complete` turns on for a list widget
//! lists the matches and never enters selection. The port checked only USEZLE
//! and started selection anyway — upstream Y01completion #33.
//!
//! The verdict is the `select-prompt` status line: complist draws it only while
//! selection runs, and `%p` expands to `Top` at run time, so the echo of the
//! `zstyle` command itself (`MSEL%p`) cannot match.
//!
//! Skip pattern: no-ops silently when `zsh` isn't on PATH or when `zsh/zpty`
//! will not load. Harness contract: `zpty_probe`.

use crate::zpty_probe::{assert_same_verdict, OPEN_PUMPED};

/// Does a stock zsh function directory exist to run `compinit` against?
fn stock_fpath_exists() -> bool {
    std::fs::read_dir("/usr/share/zsh")
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .any(|e| e.path().join("functions").is_dir())
        })
        .unwrap_or(false)
}

/// 26 matches on a 24-row terminal from `^D` (delete-char-or-list). The
/// screen is snapshotted right after `^D`; `K=yes` when selection started
/// (`want_selection`) or did not (`!want_selection`).
fn long_list_driver(lastprompt: &str, want_selection: bool) -> String {
    let (yes, no) = if want_selection { ("yes", "no") } else { ("no", "yes") };
    format!(
        "{OPEN_PUMPED}
zpty -w w 'stty rows 24 columns 80'; pump
zpty -w w 'fpath=(/usr/share/zsh/*/functions(N)); autoload -Uz compinit; compinit -u -D'; pump
zpty -w w 'zmodload zsh/complist; setopt {lastprompt}'; pump
zpty -w w '_mytest() {{ local disp=( {{a..z}} ); compadd -ld disp $disp[@] }}; compdef _mytest mytest'; pump
zpty -w w 'zstyle \":completion:*\" menu select=long-list; zstyle \":completion:*:default\" select-prompt \"MSEL%p\"'; pump
all=
zpty -w -n w 'mytest '; pump
zpty -w -n w $'\\C-d'; pump
local shot=\"$all\"
zpty -w -n w $'\\C-g'; pump
zpty -w -n w $'\\C-u'; pump
zpty -w -n w $'\\r'; pump
zpty -d w
setopt extended_glob
shot=\"${{shot//$'\\e'\\[[0-9;?]#[a-zA-Z]/}}\"
if [[ $shot == *MSELTop* ]]; then print \"K={yes}\"; else print \"K={no}\"; fi
"
    )
}

/// The control: ALWAYS_LAST_PROMPT on, selection starts in both shells.
#[test]
fn long_list_menu_selection_starts_with_always_last_prompt() {
    if !stock_fpath_exists() {
        eprintln!("skip: no /usr/share/zsh/*/functions to compinit against");
        return;
    }
    assert_same_verdict(
        &long_list_driver("alwayslastprompt", true),
        "K",
        "menu select=long-list entered selection under ALWAYS_LAST_PROMPT",
    );
}

/// The fix: ALWAYS_LAST_PROMPT off, the list is shown and selection never
/// starts (c:Src/Zle/complist.c:2031-2033).
#[test]
fn long_list_menu_selection_does_not_start_without_always_last_prompt() {
    if !stock_fpath_exists() {
        eprintln!("skip: no /usr/share/zsh/*/functions to compinit against");
        return;
    }
    assert_same_verdict(
        &long_list_driver("noalwayslastprompt", false),
        "K",
        "menu select=long-list stayed out of selection under NO_ALWAYS_LAST_PROMPT",
    );
}

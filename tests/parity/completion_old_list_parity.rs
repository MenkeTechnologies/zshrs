//! `compstate[old_list]=keep` hands back the list that is ON SCREEN, with
//! the geometry `calclist` computed for it.
//!
//! `makecomplist` keeps two names for one match list: after
//! `amatches = pmatches; ... lastmatches = pmatches;`
//! (Src/Zle/compcore.c:1022/1026) they are the same groups, and every write
//! made to `amatches` while the list is displayed — `calclist`'s per-group
//! `dcount`/`cols`/`lins`/`widths` above all — is in `lastmatches` too. When
//! the next completion sets `compstate[old_list]=keep`, c:1001
//! `amatches = lastmatches` therefore restores a list that can be printed
//! again without recomputing it, and `calclist` (compresult.c:1494) does not
//! recompute it: `listdat.valid` still holds.
//!
//! `_history-complete-older` is the stock widget that lives on this: its
//! second press keeps the old list (`_history_complete_word` sh:51-53) and
//! inserts the next history word. When that word wraps the command line the
//! list below it has been overwritten, so zrefresh redraws it
//! (zle_refresh.c:1814, `showinglist > 0 && showinglist < nlnct`). A port
//! that cloned the two lists apart came back with a kept list that had a
//! match count but no rows or columns, and the redraw printed nothing:
//!
//!     echo zz<TAB>        both shells list zza… and zzb…
//!     <TAB>               zsh inserts one and lists BOTH again;
//!                         zshrs inserted one and drew no list
//!
//! The words are 83 characters so the inserted one wraps an 80-column line.
//! Both words must appear in the output of the SECOND press alone, so the
//! first press's listing is drained and thrown away before it.
//!
//! Skip pattern: no-ops silently when `zsh` isn't on PATH or when
//! `zsh/zpty` will not load. Harness contract: `zpty_probe`.

#![allow(non_snake_case)]
#![allow(clippy::doc_lazy_continuation)]

use crate::zpty_probe::{assert_same_verdict, OPEN};

/// Does a stock zsh function directory exist to run `compinit` against?
fn stock_fpath_exists() -> bool {
    std::fs::read_dir("/usr/share/zsh")
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .any(|e| e.path().join("functions").is_dir())
        })
        .unwrap_or(false)
}

#[test]
fn history_complete_older_second_press_redraws_the_kept_list() {
    if !stock_fpath_exists() {
        eprintln!("skip: no /usr/share/zsh/*/functions to compinit against");
        return;
    }
    let a = format!("zz{}", "a".repeat(81));
    let b = format!("zz{}", "b".repeat(81));
    let driver = format!(
        "{OPEN}
zpty -w w 'unsetopt beep'
zpty -w w 'stty columns 80 rows 24'
zpty -w w 'fpath=(/usr/share/zsh/*/functions(N))'
zpty -w w 'autoload -Uz compinit; compinit -u -D'
sleep 20
zpty -w w 'bindkey \"^I\" _history-complete-older'
zpty -w w ': {a}'
zpty -w w ': {b}'
sleep 2
zpty -w -n w 'echo zz'
sleep 1
zpty -w -n w $'\\t'
sleep 4
local out all=
integer i=0
while (( i++ < 30 )); do
  if zpty -r -t w out 2>/dev/null; then all+=\"$out\"; else sleep 0.1; fi
done
all=
zpty -w -n w $'\\t'
sleep 4
i=0
while (( i++ < 60 )); do
  if zpty -r -t w out 2>/dev/null; then all+=\"$out\"; else sleep 0.1; fi
done
zpty -d w 2>/dev/null
setopt extended_glob
all=\"${{all//$'\\e'\\[[0-9;?]#[a-zA-Z]/}}\"
if [[ $all == *{a}* && $all == *{b}* ]]; then print \"K=yes\"; else print \"K=no\"; fi
"
    );
    assert_same_verdict(
        &driver,
        "K",
        "the second _history-complete-older press redrew the kept list",
    );
}

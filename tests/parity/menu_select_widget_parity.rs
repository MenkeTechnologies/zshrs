//! The `menu-select` WIDGET path through `domenuselect`.
//!
//! `domenuselect` has two callers in C (Src/Zle/complist.c): the
//! `menu_start` hook, which passes `&cdat`, and the `menu-select` widget
//! (c:3512), which calls `domenuselect(NULL, NULL)`. With `dat == NULL`
//! the return at c:3517 is `!noselect ^ acc`, so a key that ACCEPTS the
//! selection returns 0 and c:3512-3513 does not run a second
//! `menucomplete`. The accepted match keeps the space `do_single` added
//! (Src/Zle/compresult.c:1153-1155) and the typed key lands after it.
//!
//! zshrs treated `dat` as non-NULL on both paths, returned 1, and
//! re-completed the word it had just accepted; once that was fixed the
//! accepted space still never reached the editor buffer. Both showed as
//! `fzc -ba` where zsh runs `fzc -b a`.
//!
//! Skip pattern: no-ops silently when `zsh` isn't on PATH, `zsh/zpty` will
//! not load, or there is no stock function directory to `compinit` against.

use crate::zpty_probe::{assert_same_verdict, OPEN_PUMPED};

fn stock_fpath_exists() -> bool {
    std::fs::read_dir("/usr/share/zsh")
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .any(|e| e.path().join("functions").is_dir())
        })
        .unwrap_or(false)
}

/// `^Xw ^Xw a` on a `compdef -k _generic menu-select '^Xw'` binding: the
/// second `^Xw` (a completion widget, WIDGET_NCOMP) restarts the menu, `a`
/// (self-insert) accepts `-b` and is then inserted after its space.
const ACCEPT_BY_TYPING: &str = r#"
zpty -w w 'zmodload zsh/complist'; pump
zpty -w w 'fpath=(/usr/share/zsh/*/functions(N)); autoload -Uz compinit; compinit -u -D'; pump
zpty -w w 'fzc(){ print -r -- "ARGS:$*" }'; pump
zpty -w w '_fzc(){ local ret=1; _arguments -w "-b[first]" "-s[second]" && ret=0; compadd -U -Q -J grp -X hdr -- "i=$compstate[insert]"; return ret }'; pump
zpty -w w 'compdef _fzc fzc; compdef -k _generic menu-select "^Xw"; bindkey "^I" complete-word'; pump
zpty -w -n w 'fzc '; pump
zpty -w -n w $'\C-xw'; pump
zpty -w -n w $'\C-xw'; pump
zpty -w -n w 'a'; pump
zpty -w -n w $'\r'; pump
zpty -d w 2>/dev/null
if [[ $all == *ARGS:-b\ a* ]]; then print K=yes; else print K=no; fi
"#;

#[test]
fn a_key_that_accepts_the_selection_keeps_the_match_suffix() {
    if !stock_fpath_exists() {
        eprintln!("skip: no /usr/share/zsh/*/functions to compinit against");
        return;
    }
    assert_same_verdict(
        &format!("{OPEN_PUMPED}{ACCEPT_BY_TYPING}"),
        "K",
        "self-insert accepted `-b` with its space and typed `a` after it",
    );
}

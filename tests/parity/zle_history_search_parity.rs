//! The history-search widgets the stock `up-line-or-beginning-search` /
//! `down-line-or-beginning-search` functions call, and `magic-space`'s
//! return status — the bindings Oh-My-Zsh's `key-bindings.zsh` installs on
//! the arrow keys and space.
//!
//! Each case gets its own pty session. Harness contract: `zpty_probe`.

use crate::zpty_probe::{assert_same_verdict, DRAIN, OPEN};

/// One session: silence the bell, install `setup`, run `keys`, then report
/// whether `needle` reached the screen.
fn driver(setup: &str, keys: &str, needle: &str) -> String {
    format!(
        "{OPEN}
zpty -w w 'unsetopt beep'
zpty -w w '{setup}'
sleep 2
{keys}
sleep 2
{DRAIN}
if [[ $all == *'{needle}'* ]]; then print \"K=yes\"; else print \"K=no\"; fi
"
    )
}

const BIND_UP: &str = r#"print -s "print RECALL_\$((6*7))"; print -s "print other"; autoload -U up-line-or-beginning-search; zle -N up-line-or-beginning-search; bindkey "^[[A" up-line-or-beginning-search"#;

/// With an empty buffer the prefix is empty, so up-arrow must recall the
/// previous history line. zshrs walked the history ring starting from
/// `histline`, which its ZLE never maintains (it navigates `entries` with
/// `cursor`), so the widget found nothing and the arrow key did nothing.
#[test]
fn up_line_or_beginning_search_recalls_with_an_empty_buffer() {
    assert_same_verdict(
        &driver(
            BIND_UP,
            r#"zpty -w -n w $'\e[A'
sleep 1
zpty -w -n w $'\e[A'
sleep 1
zpty -w -n w $'\r'"#,
            "RECALL_42",
        ),
        "K",
        "two up-arrows recalled the older line and it ran",
    );
}

/// A typed prefix restricts the search: `print R` + up skips the newer
/// `print other` and lands on `print RECALL_$((6*7))`.
#[test]
fn up_line_or_beginning_search_honours_the_typed_prefix() {
    assert_same_verdict(
        &driver(
            BIND_UP,
            r#"zpty -w -n w 'print R'
sleep 1
zpty -w -n w $'\e[A'
sleep 1
zpty -w -n w $'\r'"#,
            "RECALL_42",
        ),
        "K",
        "prefix `print R` + up-arrow recalled the matching older line",
    );
}

/// Down-arrow after going up returns to the line being typed: the restored
/// `print R` + `$((1+2))` prints `R3`; an unrestored recalled line would print
/// `RECALL_423`.
#[test]
fn down_line_or_beginning_search_returns_to_the_typed_line() {
    assert_same_verdict(
        &driver(
            r#"print -s "print RECALL_\$((6*7))"; autoload -U up-line-or-beginning-search down-line-or-beginning-search; zle -N up-line-or-beginning-search; zle -N down-line-or-beginning-search; bindkey "^[[A" up-line-or-beginning-search; bindkey "^[[B" down-line-or-beginning-search"#,
            r#"zpty -w -n w 'print R'
sleep 1
zpty -w -n w $'\e[A'
sleep 1
zpty -w -n w $'\e[B'
sleep 1
zpty -w -n w $'$((1+2))\r'"#,
            "R3",
        ),
        "K",
        "up then down restored the typed line",
    );
}

/// `magic-space` inserts the space and reports success (c:2916-2918
/// returns `selfinsert`'s status). zshrs returned the status of
/// `expandhistory` with its sense inverted, so every space typed under
/// OH-My-Zsh's `bindkey ' ' magic-space` rang the bell.
#[test]
fn magic_space_reports_success() {
    assert_same_verdict(
        &driver(
            r#"ms(){ zle magic-space; LBUFFER+="rc$(( $? + 40 ))" }; zle -N ms; bindkey " " ms"#,
            r#"zpty -w -n w 'print'
sleep 1
zpty -w -n w ' '"#,
            "rc40",
        ),
        "K",
        "`zle magic-space` returned 0",
    );
}

/// `^Xr` reports whether the buffer holds the expansion, through the
/// arithmetic-computed marker `Y$((6*7))` / `N$((6*7))`. The typed keys and
/// the widget source only ever contain `$((6*7))`, so `Y42` reaches the
/// screen only when the widget ran and took the `Y` branch. Checking the
/// buffer BEFORE Enter matters: `accept-line` performs history expansion
/// itself, so a run-the-line check cannot tell a working widget from none.
fn report_setup(prefix: &str) -> String {
    format!(
        r#"rep(){{ if [[ $BUFFER == "{prefix}"* ]]; then LBUFFER+="Y$((6*7))"; else LBUFFER+="N$((6*7))"; fi }}; zle -N rep; bindkey "^Xr" rep"#
    )
}

/// `!!` followed by a space under `bindkey " " magic-space` is rewritten in
/// the buffer to the previous history line (c:2916 `doexpandhist`). Before
/// `doexpandhist` was a real port the buffer still read `!! `.
#[test]
fn magic_space_expands_the_history_reference_in_the_buffer() {
    let setup = format!(
        r#"print -s "print HX_\$((6*7))"; bindkey " " magic-space; {}"#,
        report_setup("print HX_")
    );
    assert_same_verdict(
        &driver(
            &setup,
            r#"zpty -w -n w '!!'
sleep 1
zpty -w -n w ' '
sleep 1
zpty -w -n w $'\x18r'"#,
            "Y42",
        ),
        "K",
        "`!! ` under magic-space became the previous command in the buffer",
    );
}

/// `expand-history` bound directly expands `!!` in the buffer.
#[test]
fn expand_history_widget_expands_the_history_reference() {
    let setup = format!(
        r#"print -s "print EH_\$((6*7))"; bindkey "^X^E" expand-history; {}"#,
        report_setup("print EH_")
    );
    assert_same_verdict(
        &driver(
            &setup,
            r#"zpty -w -n w '!!'
sleep 1
zpty -w -n w $'\x18\x05'
sleep 1
zpty -w -n w $'\x18r'"#,
            "Y42",
        ),
        "K",
        "expand-history rewrote `!!` in the buffer",
    );
}

/// `expand-or-complete` on a bare history reference expands it instead of
/// completing (the `doexpandhist` call at the top of `docomplete`, c:628).
#[test]
fn tab_on_a_history_reference_expands_it() {
    let setup = format!(
        r#"print -s "print TB_\$((6*7))"; {}"#,
        report_setup("print TB_")
    );
    assert_same_verdict(
        &driver(
            &setup,
            r#"zpty -w -n w '!!'
sleep 1
zpty -w -n w $'\t'
sleep 1
zpty -w -n w $'\x18r'"#,
            "Y42",
        ),
        "K",
        "TAB on `!!` expanded it before any completion ran",
    );
}

/// A `!` that is not an event reference — followed by a blank, as in
/// `[[ ! -f x ]]` and `print a! b` — must leave the buffer untouched when
/// space triggers `doexpandhist`; the failure path restores `ol` verbatim.
#[test]
fn magic_space_leaves_a_non_event_bang_alone() {
    let setup = format!(
        r#"bindkey " " magic-space; rep(){{ if [[ $BUFFER == "[[ ! -f /no_such_path ]] && print a! b " ]]; then LBUFFER+="Y$((6*7))"; else LBUFFER+="N$((6*7))"; fi }}; zle -N rep; bindkey "^Xr" rep"#
    );
    assert_same_verdict(
        &driver(
            &setup,
            r#"zpty -w -n w '[[ ! -f /no_such_path ]] && print a! b '
sleep 1
zpty -w -n w $'\x18r'"#,
            "Y42",
        ),
        "K",
        "magic-space kept a line holding non-event `!` intact",
    );
}

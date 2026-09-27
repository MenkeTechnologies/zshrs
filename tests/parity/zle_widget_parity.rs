//! `zle -N` widget parity — the surface every ZLE plugin is built on.
//!
//! zsh-autosuggestions, zsh-syntax-highlighting, history-substring-search
//! and fzf-tab all work the same way: define a shell function, register
//! it with `zle -N`, bind it to a key, and have it read and write the
//! line through `$BUFFER` / `$LBUFFER` / `$RBUFFER` / `$CURSOR` while
//! calling built-in widgets as `zle .name`. If any part of that contract
//! slips, plugins misbehave in ways that look like unrelated bugs.
//!
//! Each case gets its OWN pty session. A first draft shared one session
//! across three cases and was flaky: a single dropped keystroke or slow
//! redraw derails everything after it, and the whole session then
//! reports "no". One case per session is what the other interactive
//! modules do and what stays stable.
//!
//! **Do not bind a probe to a key the tty driver eats.** `^C` (INTR),
//! `^O` (DISCARD on macOS), `^V` (LNEXT), `^U` (KILL), `^D` (EOF), `^W`
//! (WERASE), `^R` (REPRINT), `^S`/`^Q` (flow control), `^Z` (SUSP) and
//! `^Y` (DSUSP) never reach ZLE at all — a probe bound to one of them
//! reports "no" on both shells and passes as false agreement. Two cases
//! here were originally bound to `^O` and `^X^C` and did exactly that.
//! The bindings below are all tty-safe.
//!
//! Skip pattern: no-ops silently when `zsh` isn't on PATH or when
//! `zsh/zpty` will not load. Harness contract: `zpty_probe`.

#![allow(non_snake_case)]
#![allow(clippy::doc_lazy_continuation)]

use crate::zpty_probe::{assert_same_verdict, DRAIN, OPEN};

/// One session: silence the bell, install `setup`, run `keys`, then
/// report whether `needle` reached the screen.
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

/// A registered widget can append to `$LBUFFER`, and what it appended
/// is what runs. The text is assembled by the widget at run time
/// (`OUTM${:-}A`), so the echo of the typed characters cannot satisfy
/// the match.
#[test]
fn a_registered_widget_can_append_to_lbuffer() {
    assert_same_verdict(
        &driver(
            r#"w1(){ LBUFFER+="OUTM${:-}A" }; zle -N w1; bindkey "^G" w1"#,
            r#"zpty -w -n w 'print '
sleep 1
zpty -w -n w $'\C-g'
sleep 2
zpty -w -n w $'\r'"#,
            "OUTMA",
        ),
        "K",
        "a zle -N widget appended to LBUFFER",
    );
}

/// `$WIDGET` names the running widget and `$KEYS` holds the keys that
/// invoked it — two chars here, because the binding is a chord. Both
/// are read by essentially every plugin widget.
#[test]
fn widget_sees_its_own_name_and_the_keys_that_ran_it() {
    assert_same_verdict(
        &driver(
            r#"w2(){ BUFFER="print W:$WIDGET K:${#KEYS}"; CURSOR=$#BUFFER }; zle -N w2; bindkey "^X^B" w2"#,
            r#"zpty -w -n w $'\C-x\C-b'
sleep 2
zpty -w -n w $'\r'"#,
            "W:w2 K:2",
        ),
        "K",
        "$WIDGET and $KEYS were visible inside the widget",
    );
}

/// `$CURSOR`, `$LBUFFER` and `$RBUFFER` describe the line as the widget
/// finds it: four characters typed, cursor at the end, nothing to the
/// right.
#[test]
fn widget_sees_cursor_lbuffer_and_rbuffer() {
    assert_same_verdict(
        &driver(
            r#"w3(){ BUFFER="print CUR:$CURSOR L:${#LBUFFER} R:${#RBUFFER}"; CURSOR=$#BUFFER }; zle -N w3; bindkey "^X^X" w3"#,
            r#"zpty -w -n w 'abcd'
sleep 1
zpty -w -n w $'\C-x\C-x'
sleep 2
zpty -w -n w $'\r'"#,
            "CUR:4 L:4 R:0",
        ),
        "K",
        "$CURSOR/$LBUFFER/$RBUFFER matched the typed line",
    );
}

/// A widget can invoke a BUILT-IN widget through `zle .name` and then
/// keep editing. Here `.backward-delete-char` removes the `Q` and the
/// widget appends its own text, so the command that runs proves both
/// halves happened in order.
#[test]
fn widget_can_call_a_builtin_widget_then_keep_editing() {
    assert_same_verdict(
        &driver(
            r#"w4(){ zle .backward-delete-char; LBUFFER+="OUTM${:-}D" }; zle -N w4; bindkey "^X^F" w4"#,
            r#"zpty -w -n w 'print Q'
sleep 1
zpty -w -n w $'\C-x\C-f'
sleep 2
zpty -w -n w $'\r'"#,
            "OUTMD",
        ),
        "K",
        "a widget called `zle .backward-delete-char` and kept editing",
    );
}

/// `zle-line-init` runs as each new line starts — the hook every
/// autosuggestion and highlighting plugin installs. Pre-filling
/// `$LBUFFER` from it means the very next Return runs that text without
/// anything being typed at all.
#[test]
fn zle_line_init_runs_for_each_new_line() {
    assert_same_verdict(
        &driver(
            r#"zle-line-init(){ LBUFFER="print OUTM${:-}F" }; zle -N zle-line-init"#,
            r#"zpty -w -n w $'\r'
sleep 2
zpty -w -n w $'\r'"#,
            "OUTMF",
        ),
        "K",
        "zle-line-init pre-filled the new line",
    );
}

/// `where-is` (c:Src/Zle/zle_main.c:1956-1972) reads a widget name through
/// the same `Where is: ` prompt and reports its bindings with showmsg.
#[test]
fn where_is_reports_the_bindings_of_a_named_widget() {
    assert_same_verdict(
        &driver(
            r#"bindkey -e; bindkey "^Xw" where-is"#,
            r#"zpty -w -n w $'\C-xw'
sleep 2
zpty -w -n w 'beginning-of-line'
sleep 2
zpty -w -n w $'\r'
sleep 2"#,
            "beginning-of-line is on",
        ),
        "K",
        "where-is printed the bindings of beginning-of-line",
    );
}

/// `zle-line-pre-redraw` (c:Src/Zle/zle_main.c:1066-1106) runs before
/// every redisplay, and what it writes to `$BUFFER` is what gets drawn and
/// run. The port queued the hook on a list nothing ever drained, so the
/// widget never ran and the typed `pr` was executed as a command.
#[test]
fn zle_line_pre_redraw_runs_before_each_redisplay() {
    assert_same_verdict(
        &driver(
            r#"zle-line-pre-redraw(){ [[ $BUFFER == pr ]] && BUFFER="print PRE${:-}DRAW" && CURSOR=$#BUFFER }; zle -N zle-line-pre-redraw"#,
            r#"zpty -w -n w 'pr'
sleep 2
zpty -w -n w $'\r'"#,
            "PREDRAW",
        ),
        "K",
        "zle-line-pre-redraw rewrote the line before it ran",
    );
}

/// `$LASTWIDGET` names the last widget run, including one run with
/// `zle NAME` inside another widget (c:Src/Zle/zle_main.c:1560-1567), and
/// `zle NAME -f nolast` runs a widget without recording it (X04zlehighlight
/// "zle $widgetname -f nolast"). The port never updated `lbindk` from
/// execzlefunc, so `$LASTWIDGET` still named whatever ran before `f`.
#[test]
fn lastwidget_names_the_inner_widget_and_nolast_keeps_it() {
    assert_same_verdict(
        &driver(
            r#"f(){ zle beginning-of-line; zle g -f nolast; BUFFER="print L:$LASTWIDGET" }; g(){ }; zle -N f; zle -N g; bindkey "^G" f"#,
            r#"zpty -w -n w $'\C-g'
sleep 2
zpty -w -n w $'\r'"#,
            "L:beginning-of-line",
        ),
        "K",
        "$LASTWIDGET after `zle beginning-of-line; zle g -f nolast`",
    );
}

/// `zle -T tc FUNC` hands every termcap capability ZLE outputs to FUNC,
/// which writes `$REPLY` instead (c:Src/Zle/zle_refresh.c:2396-2419). The
/// port recorded the function but tcout/tcoutarg never consulted it.
/// Clear-screen outputs the `cl` capability.
#[test]
fn zle_T_tc_transforms_the_capabilities_zle_outputs() {
    assert_same_verdict(
        &driver(
            r#"tcfunc(){ REPLY="<cap:$1>" }; zle -T tc tcfunc"#,
            r#"zpty -w -n w $'\C-l'
sleep 2"#,
            "<cap:cl>",
        ),
        "K",
        "zle -T tc rewrote the clear-screen capability",
    );
}

/// A put over a visual selection (put-replace-selection) edits the line,
/// and any change to the line ends the region (c:Src/Zle/zle_utils.c:844,
/// :924 `region_active = 0`), which drops the `visual` local keymap. The
/// port never cleared it, so the `a` after the put was read in the visual
/// keymap and feeped instead of appending.
#[test]
fn a_visual_put_ends_the_region() {
    assert_same_verdict(
        &driver(
            r#"bindkey -v"#,
            r#"zpty -w -n w 'print -r x${:-}ab'
sleep 1
zpty -w -n w $'\e'
sleep 1
zpty -w -n w 'yl'
sleep 1
zpty -w -n w 'vp'
sleep 1
zpty -w -n w 'ac'
sleep 1
zpty -w -n w $'\r'"#,
            "xabc",
        ),
        "K",
        "`a` appended after a visual-mode put",
    );
}

/// A terminal with no usable capabilities (`TERM=`) gets the single-line
/// display, where each keystroke redraws only what changed
/// (c:Src/Zle/zle_refresh.c:1120-1124 sets TERM_SHORT, c:1181-1184 hands
/// the frame to singlerefresh). The port never dispatched to
/// singlerefresh, and when it did the start-of-frame buffer swap left the
/// old-line buffer a frame behind, so every keystroke re-sent the whole
/// line from column 0.
#[test]
fn a_dumb_terminal_draws_typed_text_once() {
    let d = format!(
        "export TERM=\n{}",
        driver(
            "",
            r#"zpty -w -n w 'e'
sleep 0.5
zpty -w -n w 'c'
sleep 0.5
zpty -w -n w 'h'
sleep 0.5
zpty -w -n w 'o'
sleep 1"#,
            "NEVER",
        )
    );
    let d = d.replace(
        "if [[ $all == *'NEVER'* ]]; then print \"K=yes\"; else print \"K=no\"; fi",
        "local -a parts=( \"${(@s:ec:)all}\" ); if (( $#parts == 2 )); then print \"K=yes\"; else print \"K=no\"; fi",
    );
    assert_same_verdict(&d, "K", "`ec` reached a TERM= display exactly once");
}

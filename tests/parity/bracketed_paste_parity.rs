//! Bracketed paste — the path every paste into a modern terminal takes.
//!
//! A terminal with bracketed paste enabled wraps pasted text in
//! `\e[200~` … `\e[201~`, and ZLE runs the `bracketed-paste` widget on the
//! opening sequence. Plain `bracketed-paste` inserts the text; the stock
//! `bracketed-paste-magic` (shipped in Functions/Zle and enabled by many
//! configurations) captures it with `zle .bracketed-paste NAME`, replays it
//! through `zle -U` / `zle .read-command`, takes the replay back with
//! `zle .undo`, and inserts the result. Every one of those steps has to
//! work or the paste is executed as keystrokes.
//!
//! Each case gets its own pty session. Harness contract: `zpty_probe`.

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

/// `zle .bracketed-paste NAME` stores the pasted text in `$NAME` instead of
/// inserting it (c:Src/Zle/zle_misc.c:832-833 `setsparam(*args, pbuf)`).
/// zshrs wrote the text into the process environment, so `$NAME` stayed
/// empty — and its reader went around the ZLE input pump, so it did not see
/// the pasted bytes at all.
#[test]
fn bracketed_paste_with_a_name_stores_the_pasted_text() {
    assert_same_verdict(
        &driver(
            r#"bpq(){ local PASTED; zle .bracketed-paste PASTED; BUFFER="print CAP${:-}:${#PASTED}:$PASTED" }; zle -N bracketed-paste bpq"#,
            r#"zpty -w -n w $'\e[200~abcdef\e[201~'
sleep 2
zpty -w -n w $'\r'"#,
            "CAP:6:abcdef",
        ),
        "K",
        "`zle .bracketed-paste NAME` captured the pasted text",
    );
}

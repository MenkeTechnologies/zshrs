//! `vared` — the builtin that runs a ZLE edit on a parameter's value
//! (c:Src/Zle/zle_main.c:1678 `bin_vared`).
//!
//! Each case gets its own pty session, runs `vared` from the command line
//! with the edit keys typed after it, then prints a value computed from the
//! parameter's length. The marker is arithmetic (`G$((${#v}*7))`), so the
//! typed command line and the editor's echo never contain the needle: it
//! reaches the screen only when `vared` stored the edited value.
//! Harness contract: `zpty_probe`.

use crate::zpty_probe::{assert_same_verdict, DRAIN, OPEN};

/// One session: silence the bell, run `setup`, run `keys`, then report
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

/// `-c` creates a missing parameter; the typed line becomes its value.
/// `abc` has length 3, so the marker reads `G21`.
#[test]
fn vared_c_creates_the_parameter_from_the_typed_line() {
    assert_same_verdict(
        &driver(
            "true",
            r#"zpty -w w 'vared -c v'
sleep 1
zpty -w w 'abc'
sleep 1
zpty -w w 'print G$((${#v}*7))'"#,
            "G21",
        ),
        "K",
        "`vared -c v` stored the typed line",
    );
}

/// The existing value is the initial buffer with the cursor at its end
/// (c:1822 pushes it on the buffer stack, `zleread` pops it with
/// ZSL_TOEND): `xyz` + typed `12` has length 5, marker `G35`.
#[test]
fn vared_edits_the_existing_value_in_place() {
    assert_same_verdict(
        &driver(
            "v=xyz",
            r#"zpty -w w 'vared v'
sleep 1
zpty -w w '12'
sleep 1
zpty -w w 'print G$((${#v}*7))'"#,
            "G35",
        ),
        "K",
        "`vared v` appended to the existing value",
    );
}

/// `-p` is the left prompt, expanded and drawn by the editor. The needle
/// `P42>` only exists once the command's own `$((6*7))` expansion has fed
/// the prompt argument and `vared` has painted it.
#[test]
fn vared_p_draws_the_prompt() {
    assert_same_verdict(
        &driver(
            "true",
            r#"zpty -w w 'vared -p "P$((6*7))> " -c v'
sleep 1
zpty -w -n w $'\r'"#,
            "P42>",
        ),
        "K",
        "`vared -p` painted its prompt",
    );
}

/// `-h` gives the edit access to history (ZLRF_HISTORY, c:1837): ^P recalls the
/// newest history line, which is the `vared -h -c v` command itself (13
/// characters), so `v` ends up 13 long and the marker is `G91`.
#[test]
fn vared_h_lets_the_editor_recall_history() {
    assert_same_verdict(
        &driver(
            "true",
            r#"zpty -w w 'vared -h -c v'
sleep 1
zpty -w -n w $'\x10'
sleep 1
zpty -w -n w $'\r'
sleep 1
zpty -w w 'print G$((${#v}*7))'"#,
            "G91",
        ),
        "K",
        "`vared -h` recalled the history line",
    );
}

/// `-a -c` creates an array from the typed words; a backslash-quoted blank
/// stays inside one element (c:1878 `spacesplit(t, 1, 0, 1)`).
/// `one two\ three` gives 2 elements, the second 9 characters long:
/// `2*7 + 9*100` = `G914`.
#[test]
fn vared_a_splits_the_line_into_array_elements() {
    assert_same_verdict(
        &driver(
            "true",
            r#"zpty -w w 'vared -a -c arr'
sleep 1
zpty -w w 'one two\ three'
sleep 1
zpty -w w 'print G$((${#arr}*7+${#arr[2]}*100))'"#,
            "G914",
        ),
        "K",
        "`vared -a` honoured the quoted blank",
    );
}

/// An array's elements are shown with separators quoted (c:1738-1785) and
/// split back on accept, so an unedited round trip is the identity:
/// `a` and `b c` stay 2 elements, the second 3 characters: `G314`.
#[test]
fn vared_round_trips_an_array_with_a_blank_in_an_element() {
    assert_same_verdict(
        &driver(
            r#"arr=(a "b c")"#,
            r#"zpty -w w 'vared arr'
sleep 1
zpty -w -n w $'\r'
sleep 1
zpty -w w 'print G$((${#arr}*7+${#arr[2]}*100))'"#,
            "G314",
        ),
        "K",
        "`vared arr` re-quoted and re-split the element holding a blank",
    );
}

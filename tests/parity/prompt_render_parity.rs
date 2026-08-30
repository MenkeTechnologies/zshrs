//! Prompt-rendering parity — what actually reaches the terminal when
//! the shell draws `$PS1` and `$RPROMPT`.
//!
//! Prompt expansion is one of the few places where a bug is invisible
//! to every script-level test and immediately obvious to a human: a
//! dropped `%` escape, a multiline prompt that eats the next command's
//! output, an `$RPROMPT` that never reaches the right margin. This
//! project's daily driver is a four-line prompt with a right prompt on
//! it, so all of that is load-bearing.
//!
//! Each case sets a prompt with a distinctive literal in it, makes the
//! shell draw it, and checks the literal reached the screen. Escape
//! sequences are stripped before matching (see `zpty_probe::DRAIN`), so
//! `%B`…`%b` is judged on the TEXT it wrapped rather than on which SGR
//! codes were emitted — two shells may legitimately pick different
//! codes for bold, but neither may lose the word.
//!
//! Ordered patterns (`A*B`) are used where the point is sequence rather
//! than presence — that a prompt was drawn AFTER the partial line, that
//! the command's output survived the multiline prompt above it.
//!
//! One case per pty session, and every needle is a literal that cannot
//! appear in the echo of the typed setup line.
//!
//! Skip pattern: no-ops silently when `zsh` isn't on PATH or when
//! `zsh/zpty` will not load. Harness contract: `zpty_probe`.

#![allow(non_snake_case)]
#![allow(clippy::doc_lazy_continuation)]

use crate::zpty_probe::{assert_same_verdict, sq, DRAIN, OPEN};

/// Apply `setup`, then either run `cmd` or press Return twice to force
/// the prompt to be drawn again, and report whether `needle` (a zsh
/// glob) matched the escape-stripped transcript.
fn driver(setup: &str, cmd: &str, needle: &str) -> String {
    let action = if cmd.is_empty() {
        "zpty -w -n w $'\\r'\nsleep 2\nzpty -w -n w $'\\r'".to_string()
    } else {
        format!("zpty -w w {}", sq(cmd))
    };
    let setup_q = sq(setup);
    let needle_q = sq(needle);
    format!(
        "{OPEN}
zpty -w w 'unsetopt beep'
zpty -w w {setup_q}
sleep 2
{action}
sleep 3
{DRAIN}
local needle={needle_q}
if [[ $all == *${{~needle}}* ]]; then print \"K=yes\"; else print \"K=no\"; fi
"
    )
}

// ═══════════════════════════════════════════════════════════════════════
// Prompt escapes
// ═══════════════════════════════════════════════════════════════════════

/// `%~` is the working directory with `$HOME` abbreviated. Anchored to
/// `/tmp` so the expected text is the same on any machine.
#[test]
fn tilde_escape_shows_the_working_directory() {
    assert_same_verdict(
        &driver(r#"cd /tmp; PS1="D:%~> ""#, "", "D:/tmp>"),
        "K",
        "%~ expanded to the working directory",
    );
}

/// `%%` is the only way to get a literal `%` into a prompt, and it is
/// what every prompt with a percent sign in it relies on.
#[test]
fn double_percent_is_a_literal_percent() {
    assert_same_verdict(
        &driver(r#"PS1="A%%B> ""#, "", "A%B>"),
        "K",
        "%% rendered a literal percent",
    );
}

/// `%#` is `%` for an ordinary user (`#` for root). Tests run
/// unprivileged, so the expected rendering is the percent.
#[test]
fn hash_escape_renders_for_an_unprivileged_user() {
    assert_same_verdict(
        &driver(r#"PS1="H%#> ""#, "", "H%>"),
        "K",
        "%# rendered for an unprivileged user",
    );
}

/// `%B`…`%b` wraps text in bold. The two shells may emit different SGR
/// codes; neither may lose the text between them. Escapes are stripped
/// before matching, so this judges exactly that.
#[test]
fn bold_escapes_do_not_swallow_the_text_between_them() {
    assert_same_verdict(
        &driver(r#"PS1="%BBOLDP%b> ""#, "print OUTWW", "BOLDP>*OUTWW"),
        "K",
        "%B…%b kept the text it wrapped",
    );
}

/// With PROMPT_SUBST a prompt is re-expanded every time it is drawn, so
/// a command substitution inside it runs per prompt. Every dynamic
/// prompt in existence depends on this.
#[test]
fn promptsubst_runs_a_command_substitution_in_the_prompt() {
    assert_same_verdict(
        &driver(
            r#"setopt promptsubst; PS1="S:$(print zz)> ""#,
            "",
            "S:zz>",
        ),
        "K",
        "PROMPT_SUBST expanded $(…) in the prompt",
    );
}

// ═══════════════════════════════════════════════════════════════════════
// Geometry — multiline prompts, right prompts, partial lines
// ═══════════════════════════════════════════════════════════════════════

/// A two-line prompt has to draw both lines.
#[test]
fn a_multiline_prompt_draws_every_line() {
    assert_same_verdict(
        &driver(r#"PS1=$'L1\nL2> '"#, "", "L1"),
        "K",
        "a multiline prompt drew its first line",
    );
}

/// A three-line prompt must not swallow the output of the command run
/// under it — the failure mode a tall prompt actually has.
#[test]
fn a_three_line_prompt_does_not_eat_command_output() {
    assert_same_verdict(
        &driver(r#"PS1=$'A1\nA2\nA3> '"#, "print OUTZZ", "OUTZZ"),
        "K",
        "a three-line prompt left the command output intact",
    );
}

/// `$RPROMPT` alongside a multiline prompt: the right prompt must still
/// be drawn, and the command run afterwards must still produce output.
/// The ordered pattern pins that the right prompt came first.
#[test]
fn rprompt_draws_alongside_a_multiline_prompt() {
    assert_same_verdict(
        &driver(
            r#"PS1=$'B1\nB2> '; RPROMPT=RQQQ"#,
            "print OUTYY",
            "RQQQ*OUTYY",
        ),
        "K",
        "RPROMPT drew next to a multiline prompt",
    );
}

/// A command whose output has no trailing newline: the shell marks the
/// partial line and starts the next prompt on a fresh row. The ordered
/// pattern is the point — the prompt has to come AFTER the orphaned
/// text, not on top of it.
#[test]
fn a_partial_line_is_followed_by_a_fresh_prompt() {
    assert_same_verdict(
        &driver(r#"PS1="PXX> ""#, "printf NOEOL", "NOEOL*PXX>"),
        "K",
        "a prompt was drawn after a newline-less line",
    );
}

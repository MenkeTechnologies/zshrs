//! Interactive history-expansion parity — `!!` and `!$` at the prompt,
//! with and without `HIST_VERIFY`.
//!
//! History expansion happens in the line editor, so a `-c` script never
//! sees it. The two halves behave differently on purpose:
//!
//!   * without `HIST_VERIFY`, an expansion is substituted and the
//!     command RUNS immediately;
//!   * with `HIST_VERIFY` it is substituted into the BUFFER and left
//!     there for the user to look at, and Return has to be pressed
//!     again to run it. That is the entire point of the option — it is
//!     the guard against `!!` running something you did not mean.
//!
//! Both are pinned, because a shell that ignores the option passes a
//! test of the other half.
//!
//! Skip pattern: no-ops silently when `zsh` isn't on PATH or when
//! `zsh/zpty` will not load. Harness contract: `zpty_probe`.

#![allow(non_snake_case)]
#![allow(clippy::doc_lazy_continuation)]

use crate::zpty_probe::{assert_same_dump, dump_widget, sq, DUMP_KEY, OPEN};

/// Run `print MARKONE`, then type `recall` and press Return, and report
/// HOW MANY times the marker reached the terminal.
///
/// The count is the verdict because it separates the three outcomes
/// cleanly: 2 means the recall produced nothing, 3 means it was
/// substituted into the buffer but not run, 4 means it ran. Comparing
/// the two shells' counts therefore catches "ran when it should have
/// waited" and "vanished" as different failures.
fn count_driver(setopt: &str, recall: &str) -> String {
    format!(
        "{OPEN}
zpty -w w 'unsetopt beep'
zpty -w w 'HISTSIZE=100; SAVEHIST=0; unset HISTFILE'
zpty -w w {}
sleep 2
zpty -w -n w 'print MARKONE'
sleep 2
zpty -w -n w $'\\r'
sleep 2
zpty -w -n w {}
sleep 2
zpty -w -n w $'\\r'
sleep 3
local out all=
integer i=0
while (( i++ < 50 )); do
  if zpty -r -t w out 2>/dev/null; then all+=\"$out\"; else sleep 0.1; fi
done
zpty -d w 2>/dev/null
print -r -- \"N=$(print -r -- \"$all\" | grep -c MARKONE)\" >! $OUTFILE
",
        sq(setopt),
        sq(recall)
    )
}

/// Same interaction, but dumping the BUFFER after the recall's Return —
/// which is where `HIST_VERIFY` is supposed to leave the expansion.
fn buffer_driver(setopt: &str, recall: &str) -> String {
    let widget = dump_widget(r#""BUF=[$BUFFER] CUR=[$CURSOR]""#);
    format!(
        "{OPEN}
zpty -w w 'unsetopt beep'
zpty -w w 'HISTSIZE=100; SAVEHIST=0; unset HISTFILE'
zpty -w w {}
{widget}
sleep 2
zpty -w -n w 'print MARKONE'
sleep 2
zpty -w -n w $'\\r'
sleep 2
zpty -w -n w {}
sleep 2
zpty -w -n w $'\\r'
sleep 2
{DUMP_KEY}
local out all=
integer i=0
while (( i++ < 50 )); do
  if zpty -r -t w out 2>/dev/null; then all+=\"$out\"; else sleep 0.1; fi
done
zpty -d w 2>/dev/null
",
        sq(setopt),
        sq(recall)
    )
}

// ═══════════════════════════════════════════════════════════════════════
// NO_HIST_VERIFY — the expansion runs, and both shells agree
// ═══════════════════════════════════════════════════════════════════════

/// `!!` recalls the whole previous command and runs it.
#[test]
fn bang_bang_expands_and_runs_without_hist_verify() {
    assert_same_dump(
        &count_driver("unsetopt hist_verify", "!!"),
        "!! expanded and ran with NO_HIST_VERIFY",
    );
}

/// `!$` recalls the previous command's last argument.
#[test]
fn bang_dollar_expands_and_runs_without_hist_verify() {
    assert_same_dump(
        &count_driver("unsetopt hist_verify", "print !$"),
        "!$ expanded and ran with NO_HIST_VERIFY",
    );
}

// ═══════════════════════════════════════════════════════════════════════
// HIST_VERIFY — the expansion must land in the buffer, unrun
// ═══════════════════════════════════════════════════════════════════════

/// Under `HIST_VERIFY` the expanded line is left in the buffer for
/// confirmation, not dropped.
///
///     setopt hist_verify
///     print MARKONE          # runs
///     !!  <Return>
///       zsh    buffer becomes `print MARKONE`, nothing runs
///
/// The mechanism spans two files: `hend` pushes the expansion onto the
/// buffer stack (c:Src/hist.c:1562-1563 `zpushnode(bufstack, ptr)`), and
/// the next `zleread` pops it into the line (c:Src/Zle/zle_main.c
/// `getlinknode(bufstack)`). With either half missing the line is
/// simply gone: `BUF=[] CUR=[0]` and a marker count of 2 instead of 3.
///
/// The control is the NO_HIST_VERIFY pair above, where both shells
/// score 4 — so these isolate the verify path. `setopt hist_verify` is
/// in this repo's daily-driver config, so a regression here means `!!`
/// silently does nothing there.
#[test]
fn hist_verify_leaves_bang_bang_in_the_buffer() {
    assert_same_dump(
        &buffer_driver("setopt hist_verify", "!!"),
        "HIST_VERIFY left the !! expansion in the buffer",
    );
}

/// The same for a word designator rather than a whole line.
#[test]
fn hist_verify_leaves_bang_dollar_in_the_buffer() {
    assert_same_dump(
        &buffer_driver("setopt hist_verify", "print !$"),
        "HIST_VERIFY left the !$ expansion in the buffer",
    );
}

/// The same behaviour counted rather than dumped, so it is pinned on
/// both measurements: 3 (buffered) versus 2 (vanished).
#[test]
fn hist_verify_does_not_lose_the_expansion() {
    assert_same_dump(
        &count_driver("setopt hist_verify", "!!"),
        "HIST_VERIFY kept the expansion visible",
    );
}

// ═══════════════════════════════════════════════════════════════════════
// `^old^new` quick substitution, closing delimiter omitted
// ═══════════════════════════════════════════════════════════════════════
//
// `^old^new` is `!!:s^old^new^` with the trailing `^` left off, which is
// how it is normally typed. The replacement is read up to the delimiter
// OR a newline, and the newline has to be put back (c:Src/hist.c:2606-
// 2607 in `hdynread2`) because it is what ends the command. A reader
// that swallows it leaves the expanded line unterminated: the lexer
// takes the NEXT line as more of the same command, so at a prompt the
// substitution echoes nothing, runs nothing, and the shell sits in a
// continuation read. That is independent of HIST_VERIFY, so both
// settings are pinned.

/// Without HIST_VERIFY the substituted line runs. Counted on MARKONE:
/// the first line's echo and output, then the substituted line's echo
/// and output — 4. An unterminated line scores 2.
#[test]
fn quick_substitution_runs_without_hist_verify() {
    assert_same_dump(
        &count_driver("unsetopt hist_verify", "^print^print -r --"),
        "^old^new substituted and ran with NO_HIST_VERIFY",
    );
}

/// With HIST_VERIFY the substituted line is left in the buffer, exactly
/// as `!!` is.
#[test]
fn hist_verify_leaves_quick_substitution_in_the_buffer() {
    assert_same_dump(
        &buffer_driver("setopt hist_verify", "^ONE^TWO"),
        "HIST_VERIFY left the ^old^new substitution in the buffer",
    );
}

/// `:s` shares the reader, so `!!:s/old/new` with its closing `/`
/// omitted lost its terminator the same way.
#[test]
fn hist_verify_leaves_open_s_modifier_in_the_buffer() {
    assert_same_dump(
        &buffer_driver("setopt hist_verify", "!!:s/ONE/TWO"),
        "HIST_VERIFY left the unclosed :s substitution in the buffer",
    );
}

// ═══════════════════════════════════════════════════════════════════════
// `subst()` — what the substitution DID, and what it reports
// ═══════════════════════════════════════════════════════════════════════
//
// These do not need a pty. History expansion runs for any interactive
// shell, so piping a script into `-fis` exercises it, and the shell
// ECHOES the line it expanded — which is the whole verdict, together
// with the exit status and the error text when it refuses.
//
// The distinction under test is that C's `subst()` (c:Src/hist.c:2336)
// returns a STATUS, 0 for "a substitution was made" and 1 for "the
// pattern did not match" (c:2386 vs c:2390), and that status is
// independent of whether the line CHANGED. `^old^old` replaces `old`
// with `old`: zsh runs the line, a shell that decides by comparing the
// before and after text says `substitution failed`.
//
// The same block also gates `#` and `%` as anchors and the glob
// matcher behind `HIST_SUBST_PATTERN` (c:2344), so every case that can
// tell the two matchers apart is pinned with the option both ways.

/// Feed a script to an interactive `$UNDER_TEST` and keep the lines
/// that carry the verdict: the marker (so the echoed expansion and its
/// output both show), the status, and the two refusals `subst` can
/// produce. Everything else is prompt noise, which differs between the
/// shells for reasons that have nothing to do with substitution.
fn subst_driver(setup: &str, recall: &str) -> String {
    format!(
        r#"
local out
out=$({{ print -rl -- \
  'unsetopt promptcr promptsp' 'HISTFILE=/dev/null' 'HISTSIZE=200' 'SAVEHIST=0' \
  {} \
  'print SETUPDONE' \
  'print HSMARK old xx' \
  {} \
  'print "HSRC=$?"' ; }} | PS1= RPS1= PROMPT= $UNDER_TEST -f -i -s 2>&1)
print -rl -- ${{(M)${{(f)out}}:#*(HSMARK|HSRC=|substitution failed|no previous substitution)*}} >! $OUTFILE
"#,
        sq(setup),
        sq(recall)
    )
}

/// `^old^old` — the replacement equals what it replaced. zsh
/// substitutes and runs the line; the text is unchanged, which is not
/// the same thing as the substitution failing. This is the case the
/// "did the string change?" test gets wrong, and it reaches `subst`
/// through `histsubchar` (c:632) rather than through `modify`.
#[test]
fn quick_substitution_with_an_identical_replacement_succeeds() {
    assert_same_dump(
        &subst_driver("unsetopt hist_subst_pattern", "^old^old"),
        "^old^old substituted and ran",
    );
    assert_same_dump(
        &subst_driver("setopt hist_subst_pattern", "^old^old"),
        "^old^old substituted and ran under HIST_SUBST_PATTERN",
    );
}

/// The control: a pattern that genuinely is not there must still fail,
/// with zsh's own message and status. Without this the case above
/// could be satisfied by never failing at all.
#[test]
fn quick_substitution_with_no_match_still_fails() {
    assert_same_dump(
        &subst_driver("unsetopt hist_subst_pattern", "^zzz^q"),
        "^zzz^q reported substitution failed",
    );
    assert_same_dump(
        &subst_driver("setopt hist_subst_pattern", "^zzz^q"),
        "^zzz^q reported substitution failed under HIST_SUBST_PATTERN",
    );
}

/// The `:s` spelling of the same thing, which reaches `subst` from the
/// modifier loop (c:905) instead. Both call sites had to branch on the
/// status, so both are pinned.
#[test]
fn s_modifier_with_an_identical_replacement_succeeds() {
    assert_same_dump(
        &subst_driver("unsetopt hist_subst_pattern", "!!:s/old/old/"),
        ":s with an identical replacement substituted and ran",
    );
    assert_same_dump(
        &subst_driver("setopt hist_subst_pattern", "!!:s/old/old/"),
        ":s with an identical replacement under HIST_SUBST_PATTERN",
    );
}

/// `&` in the replacement stands for the pattern — but only on the
/// literal arm (c:2375 `convamps`). On the pattern arm C hands `out`
/// to `getmatch` untouched (c:2367), so `&` is an ordinary character
/// there and the expanded line differs between the two option states.
#[test]
fn ampersand_in_the_replacement_stands_for_the_pattern() {
    assert_same_dump(
        &subst_driver("unsetopt hist_subst_pattern", "!!:s/old/&X/"),
        "`&` expanded to the pattern",
    );
    assert_same_dump(
        &subst_driver("setopt hist_subst_pattern", "!!:s/old/&X/"),
        "`&` stayed literal under HIST_SUBST_PATTERN",
    );
}

/// `\&` and `\/` — the backslash is eaten by the reader (c:2597-2598
/// in `hdynread2`), so `\/` is how a `/` gets past the delimiter and
/// `\&` does NOT protect the `&` from `convamps` on the literal arm.
#[test]
fn escaped_ampersand_and_slash_reach_the_replacement() {
    assert_same_dump(
        &subst_driver("unsetopt hist_subst_pattern", r"!!:s/old/A\&B/"),
        r"`\&` in the replacement",
    );
    assert_same_dump(
        &subst_driver("setopt hist_subst_pattern", r"!!:s/old/A\&B/"),
        r"`\&` in the replacement under HIST_SUBST_PATTERN",
    );
    assert_same_dump(
        &subst_driver("unsetopt hist_subst_pattern", r"!!:s/old/A\/B/"),
        r"`\/` in the replacement",
    );
}

/// A blank in the replacement. The replacement is read to the
/// delimiter, so `A B` is two words of one replacement — and on the
/// pattern arm it goes through `parse_subst_string`, whose lexer must
/// NOT treat the blank as the end of a word (c:Src/lex.c:968, the
/// `!sub` term).
#[test]
fn a_blank_in_the_replacement_survives() {
    assert_same_dump(
        &subst_driver("unsetopt hist_subst_pattern", "!!:s/old/A B/"),
        "a blank in the replacement",
    );
    assert_same_dump(
        &subst_driver("setopt hist_subst_pattern", "!!:s/old/A B/"),
        "a blank in the replacement under HIST_SUBST_PATTERN",
    );
}

/// `#` and `%` anchor the pattern ONLY on the pattern arm (c:2349,
/// c:2354 — both inside the `isset(HISTSUBSTPATTERN) || forcepat`
/// block at c:2344). Under plain `:s` they are ordinary characters, so
/// `#print` is four characters that are not in the line and zsh
/// refuses; with the option it anchors and succeeds. The pair is what
/// makes the gate visible: a shell that always anchors passes the
/// second and fails the first.
#[test]
fn the_head_anchor_only_applies_under_hist_subst_pattern() {
    assert_same_dump(
        &subst_driver("unsetopt hist_subst_pattern", "!!:s/#print/echo/"),
        "`#print` is literal without HIST_SUBST_PATTERN",
    );
    assert_same_dump(
        &subst_driver("setopt hist_subst_pattern", "!!:s/#print/echo/"),
        "`#print` anchors at the head under HIST_SUBST_PATTERN",
    );
}

/// The tail anchor, same gate.
#[test]
fn the_tail_anchor_only_applies_under_hist_subst_pattern() {
    assert_same_dump(
        &subst_driver("unsetopt hist_subst_pattern", "!!:s/%xx/yy/"),
        "`%xx` is literal without HIST_SUBST_PATTERN",
    );
    assert_same_dump(
        &subst_driver("setopt hist_subst_pattern", "!!:s/%xx/yy/"),
        "`%xx` anchors at the tail under HIST_SUBST_PATTERN",
    );
}

/// A glob metacharacter in the pattern: `strstr` on the literal arm
/// (so `o*d` is not in the line and the substitution is refused),
/// `getmatch` on the pattern arm (so it matches `old`).
#[test]
fn a_glob_in_the_pattern_matches_only_under_hist_subst_pattern() {
    assert_same_dump(
        &subst_driver("unsetopt hist_subst_pattern", "!!:s/o*d/G/"),
        "`o*d` is literal without HIST_SUBST_PATTERN",
    );
    assert_same_dump(
        &subst_driver("setopt hist_subst_pattern", "!!:s/o*d/G/"),
        "`o*d` matched as a pattern under HIST_SUBST_PATTERN",
    );
}

/// `:gs` replaces every match, on both arms — c:2384 loops the
/// `strstr` arm, and c:2347-2348 passes `SUB_GLOBAL` into `getmatch`
/// on the pattern arm. `[x]` is a glob, so the two arms disagree about
/// whether there is anything to replace at all.
#[test]
fn global_substitution_replaces_every_match() {
    assert_same_dump(
        &subst_driver("unsetopt hist_subst_pattern", "!!:gs/x/y/"),
        ":gs replaced both x's",
    );
    assert_same_dump(
        &subst_driver("setopt hist_subst_pattern", "!!:gs/[x]/y/"),
        ":gs with a glob replaced both x's",
    );
    assert_same_dump(
        &subst_driver("setopt hist_subst_pattern", "!!:gs/x*/Q/"),
        ":gs with a greedy glob",
    );
}

/// An empty pattern reuses the last one: `getsubsargs` leaves `hsubl`
/// alone when the pattern it read is empty (c:531-535), so `!!:s//Q/`
/// after `!!:s/old/new/` substitutes `old` again.
#[test]
fn an_empty_pattern_reuses_the_previous_one() {
    assert_same_dump(
        &subst_driver(
            "unsetopt hist_subst_pattern",
            "!!:s/old/new/ ; print HSMARK old yy ; !!:s//Q/",
        ),
        "an empty pattern reused the previous one",
    );
    assert_same_dump(
        &subst_driver("unsetopt hist_subst_pattern", "!!:s//Q/"),
        "an empty pattern with nothing to reuse",
    );
}

/// `:&` repeats the last substitution, and falls into the same arm as
/// `:s` (c:903), so it inherits the same status branch.
#[test]
fn the_repeat_modifier_reuses_the_last_substitution() {
    assert_same_dump(
        &subst_driver(
            "unsetopt hist_subst_pattern",
            "!!:s/old/new/ ; print HSMARK old yy ; !!:&",
        ),
        ":& repeated the last substitution",
    );
}

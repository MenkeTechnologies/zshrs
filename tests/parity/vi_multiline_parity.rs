//! vi-mode vertical motion over a MULTI-LINE buffer, judged on the line
//! editor's own state.
//!
//! `k` and `j` in vicmd are `vi-up-line-or-history` /
//! `vi-down-line-or-history`, and neither is a plain line move
//! (`Src/Zle/zle_hist.c:302, 390`):
//!
//! ```c
//! int col = lastcol;
//! uplineorhistory(args);
//! lastcol = col;
//! return vifirstnonblank(args);
//! ```
//!
//! Two things beyond the move: the cursor lands on the first NON-BLANK of
//! the new line, and `lastcol` — the column a vertical motion aims for,
//! latched by `upline`/`downline` at c:369-372 — is restored afterwards so
//! a run of `j`/`k` keeps tracking the column it started from instead of
//! drifting one line at a time.
//!
//! Both are invisible on a single-line buffer, which is why a port can lose
//! them and still pass every one-line vi case: `uplineorhistory` on line 1
//! of 1 just walks history. The cases here build a real multi-line buffer
//! with `o` (`vi-open-line-below`) and read `$BUFFER`/`$CURSOR` from inside
//! a widget, so the verdict is the editor's state and not the transcript.
//!
//! Skip pattern: no-ops silently when `zsh` isn't on PATH or when
//! `zsh/zpty` will not load. Harness contract: `zpty_probe`.

#![allow(non_snake_case)]
#![allow(clippy::doc_lazy_continuation)]

use crate::zpty_probe::{assert_same_dump, DRAIN, DUMP_KEY, DUMP_WIDGET, OPEN};

/// A vi-mode session holding a three-line buffer with DIFFERENT indents,
/// so "first non-blank" is a different column on every line and a widget
/// that merely keeps the old column lands somewhere else.
///
///     __alpha        (2 spaces)
///     ______beta     (6 spaces)
///     x              (none)
///
/// Built with `o` from command mode rather than by pasting newlines: `o`
/// is the vi key that makes a multi-line buffer in the first place, and
/// each one leaves the editor in insert mode on the new line.
fn three_indented_lines(keys: &str) -> String {
    format!(
        "{OPEN}
zpty -w w 'bindkey -v'
zpty -w w 'unset HISTFILE; HISTSIZE=100; SAVEHIST=0'
sleep 1
{DUMP_WIDGET}
sleep 1
zpty -w -n w '  alpha'
sleep 1
zpty -w -n w $'\\e'
sleep 1
zpty -w -n w 'o'
sleep 1
zpty -w -n w '      beta'
sleep 1
zpty -w -n w $'\\e'
sleep 1
zpty -w -n w 'o'
sleep 1
zpty -w -n w 'x'
sleep 1
zpty -w -n w $'\\e'
sleep 1
{keys}
{DUMP_KEY}
{DRAIN}
"
    )
}

/// `k` from the last line lands on the first NON-BLANK of the line above,
/// not on the column the cursor happened to hold.
#[test]
fn k_lands_on_the_first_nonblank_of_the_line_above() {
    assert_same_dump(
        &three_indented_lines("zpty -w -n w 'k'\nsleep 1\n"),
        "vicmd k from the last line of a three-line buffer",
    );
}

/// Two `k` in a row cross two differently-indented lines. A widget that
/// clobbers `lastcol` drifts on the second one.
#[test]
fn two_k_in_a_row_cross_both_lines() {
    assert_same_dump(
        &three_indented_lines("zpty -w -n w 'k'\nsleep 1\nzpty -w -n w 'k'\nsleep 1\n"),
        "vicmd k twice from the last line of a three-line buffer",
    );
}

/// `j` back down after `k` returns to the first non-blank of the lower
/// line — the mirror case, which is what pins `vidownlineorhistory`.
#[test]
fn j_after_k_returns_to_the_first_nonblank_below() {
    assert_same_dump(
        &three_indented_lines(
            "zpty -w -n w 'kk'\nsleep 1\nzpty -w -n w 'j'\nsleep 1\n",
        ),
        "vicmd k k then j over a three-line buffer",
    );
}

/// `k` with the cursor parked at the END of a long line: the column to
/// aim for is past the end of the shorter line above, which is exactly
/// the case `lastcol` exists to arbitrate.
#[test]
fn k_from_the_end_of_a_longer_line() {
    assert_same_dump(
        &three_indented_lines("zpty -w -n w 'kk$'\nsleep 1\nzpty -w -n w 'j'\nsleep 1\n"),
        "vicmd to the end of line 1 then j to a shorter line",
    );
}

/// The buffer itself, with no motion at all. If `o` builds a different
/// buffer in the two shells, every case above is comparing the wrong
/// thing — this is the control that says they are not.
#[test]
fn o_builds_the_same_three_line_buffer() {
    assert_same_dump(
        &three_indented_lines(""),
        "the three-line buffer `o` builds from command mode",
    );
}

// ═══════════════════════════════════════════════════════════════════════
// Undo chaining
// ═══════════════════════════════════════════════════════════════════════

/// A vi session that makes THREE separate insert-mode edits, each its own
/// change record, then presses `u` `count` times from command mode.
///
/// Every `u` must step one change further back. What breaks that is a
/// missing `setlastline()` (`zle_utils.c:1628`): `handleundo()` diffs the
/// live line against `lastline`, so an undo that does not reset the
/// baseline makes the NEXT widget record the undo itself as a fresh
/// change, and the following `u` undoes that phantom instead of chaining.
/// One `u` therefore looks correct while two do not — the reason this
/// needs its own case rather than an assertion on a single undo.
fn three_edits_then_undo(count: usize) -> String {
    let undos = "zpty -w -n w 'u'\nsleep 2\n".repeat(count);
    format!(
        "{OPEN}
zpty -w w 'bindkey -v'
zpty -w w 'unset HISTFILE; HISTSIZE=100; SAVEHIST=0'
sleep 1
{DUMP_WIDGET}
sleep 1
zpty -w -n w 'aaa'
sleep 2
zpty -w -n w $'\\e'
sleep 2
zpty -w -n w 'abbb'
sleep 2
zpty -w -n w $'\\e'
sleep 2
zpty -w -n w 'accc'
sleep 2
zpty -w -n w $'\\e'
sleep 2
{undos}{DUMP_KEY}
{DRAIN}
"
    )
}

/// One `u` undoes the last edit. The control: if this diverges, the two
/// shells disagree about what a change IS and the chaining cases below
/// are measuring the wrong thing.
#[test]
fn one_u_undoes_the_last_edit() {
    assert_same_dump(&three_edits_then_undo(1), "vicmd u once after three edits");
}

/// Two `u` in a row reach the edit before that.
///
/// OPEN DIVERGENCE, not a flake. With three insert sessions on the line,
/// the second `u` is a no-op in zshrs while zsh unwinds the rest of the
/// line:
///
///     zsh    BUF=[]        CUR=[0]
///     zshrs  BUF=[aaabbb]  CUR=[5]
///
/// The first `u` agrees exactly (see the case above), so the change
/// records and the first step of the chain are right; what stops is the
/// walk. `undo` follows CH_PREV backward from `curchange`
/// (zle_utils.c:1630) and `mergeundo` sets those flags at ESC — the
/// remaining gap is which records end up flagged when the FIRST insert
/// session was never opened by `startvitext` (typing at the initial viins
/// prompt latches no `vistartchange`), so C merges it into the group and
/// this port does not.
///
/// Ignored so the suite stays honest about what passes; delete the
/// attribute with the fix.
#[ignore = "open: the CH_PREV chain stops after one step — see the doc comment"]
#[test]
fn two_u_chain_back_two_edits() {
    assert_same_dump(&three_edits_then_undo(2), "vicmd u twice after three edits");
}

/// Three `u` unwind every edit. Pressing `u` more times than there are
/// changes must also agree — zsh stops at the oldest change rather than
/// emptying the buffer.
///
/// Blocked on the same chain gap as `two_u_chain_back_two_edits`.
#[ignore = "open: the CH_PREV chain stops after one step — see two_u_chain_back_two_edits"]
#[test]
fn u_past_the_oldest_change_stops_where_zsh_stops() {
    assert_same_dump(&three_edits_then_undo(5), "vicmd u five times after three edits");
}

// ═══════════════════════════════════════════════════════════════════════
// Text objects
// ═══════════════════════════════════════════════════════════════════════

/// A vi session over `echo "hello world" tail` with the cursor parked on
/// the `h` of `hello` — inside the quotes, inside a word, inside a shell
/// argument, so every object zsh ships has something to select.
///
/// zsh binds exactly six text objects, in `viopp` and `visual`
/// (`zle_keymap.c:1381-1386`): `aa` `ia` (shell word), `aw` `iw` (word),
/// `aW` `iW` (blank word). `i"` is NOT among them — quote and bracket
/// objects come from plugins, not the shell — so a case for `ci"` is a
/// statement about what zsh does with an unbound sequence, which is still
/// worth pinning: both shells have to do the SAME nothing.
fn quoted_line(keys: &str) -> String {
    format!(
        "{OPEN}
zpty -w w 'bindkey -v'
zpty -w w 'unset HISTFILE; HISTSIZE=100; SAVEHIST=0'
sleep 1
{DUMP_WIDGET}
sleep 1
zpty -w -n w 'echo \\\"hello world\\\" tail'
sleep 2
zpty -w -n w $'\\e'
sleep 1
zpty -w -n w '0'
sleep 1
zpty -w -n w 'fh'
sleep 1
{keys}
sleep 2
zpty -w -n w $'\\e'
sleep 1
{DUMP_KEY}
{DRAIN}
"
    )
}

/// `ciw` changes the word under the cursor: `hello` goes, the rest stays.
#[test]
fn ciw_changes_the_word_under_the_cursor() {
    assert_same_dump(
        &quoted_line("zpty -w -n w 'ciw'\nsleep 1\nzpty -w -n w 'X'\n"),
        "vicmd ciw on a word inside quotes",
    );
}

/// `caw` takes the trailing whitespace with it, which is the whole
/// difference between `a` and `i`.
#[test]
fn caw_takes_the_word_and_its_whitespace() {
    assert_same_dump(
        &quoted_line("zpty -w -n w 'caw'\nsleep 1\nzpty -w -n w 'X'\n"),
        "vicmd caw on a word inside quotes",
    );
}

/// `cia` selects the shell ARGUMENT — here the whole quoted string,
/// quotes included or not per zsh's own splitting.
#[test]
fn cia_changes_the_shell_argument() {
    assert_same_dump(
        &quoted_line("zpty -w -n w 'cia'\nsleep 1\nzpty -w -n w 'X'\n"),
        "vicmd cia on a quoted shell argument",
    );
}

/// `diw` — the same object under a different operator, which is what
/// separates "the object is broken" from "the operator is broken".
#[test]
fn diw_deletes_the_word_under_the_cursor() {
    assert_same_dump(
        &quoted_line("zpty -w -n w 'diw'\n"),
        "vicmd diw on a word inside quotes",
    );
}

/// `ci\"` is not a zsh binding. Whatever zsh does with the unbound `\"`
/// after `ci`, this shell has to do the same — including leaving the
/// buffer untouched.
#[test]
fn ci_quote_does_what_zsh_does_with_an_unbound_object() {
    assert_same_dump(
        &quoted_line("zpty -w -n w 'ci\\\"'\nsleep 1\n"),
        "vicmd ci\" — an object zsh does not bind",
    );
}

/// `ci"` through zsh's OWN `select-quoted` function, autoloaded and bound
/// in `viopp` the way `Functions/Zle/select-quoted`'s header documents.
///
/// The object is a shell function, and its last two lines are
///
///     MARK=found
///     CURSOR=end
///
/// Both are bare identifiers assigned to ZLE integer specials, which zsh
/// evaluates ARITHMETICALLY. `CURSOR` was routed to the live-editor write
/// path (params.rs:6827) with the raw word, which parsed as 0, while
/// `MARK` — not in that list — evaluated correctly. Every quote and
/// bracket object therefore selected from the mark to the START OF THE
/// LINE: `ci"` on `echo "hello world" tail` produced
/// `Xhello world" tail`.
///
/// Nothing in the six built-in text objects catches this: they are C
/// widgets that set the cursor through `zlecs` directly and never assign
/// `$CURSOR` as a shell parameter.
#[test]
fn ci_quote_via_the_autoloaded_select_quoted_function() {
    let driver = format!(
        "{OPEN}
zpty -w w 'fpath=(/usr/share/zsh/*/functions(N) /opt/homebrew/share/zsh/functions(N) $fpath)'
zpty -w w 'autoload -Uz select-quoted; zle -N select-quoted'
zpty -w w 'for m in visual viopp; do bindkey -M $m i\\\" select-quoted; bindkey -M $m a\\\" select-quoted; done'
zpty -w w 'bindkey -v'
zpty -w w 'unset HISTFILE; HISTSIZE=100; SAVEHIST=0'
sleep 1
{DUMP_WIDGET}
sleep 1
zpty -w -n w 'echo \\\"hello world\\\" tail'
sleep 2
zpty -w -n w $'\\e'
sleep 1
zpty -w -n w '0'
sleep 1
zpty -w -n w 'ci\\\"'
sleep 2
zpty -w -n w 'X'
sleep 1
zpty -w -n w $'\\e'
sleep 1
{DUMP_KEY}
{DRAIN}
"
    );
    assert_same_dump(&driver, "ci\" through the autoloaded select-quoted function");
}

/// A vi session with the WHOLE key sequence sent in ONE write, the way
/// zsh's own `Test/X02zlevi.ztst` drives it (`zletest`, Test/comptest).
/// One write matters: an ESC followed at once by the next key goes
/// through the ESC-prefix lookahead in `getkeymapcmd`, which is where the
/// lookahead byte must be taken back out of the vi change being recorded
/// (`zle_keymap.c:1706-1707`). `keys` is the body of a `$'…'` string.
fn vi_one_write(keys: &str) -> String {
    format!(
        "{OPEN}
zpty -w w 'bindkey -v; KEYTIMEOUT=1'
zpty -w w 'unset HISTFILE; HISTSIZE=100; SAVEHIST=0'
sleep 1
{DUMP_WIDGET}
sleep 1
zpty -w -n w $'{keys}'
sleep 1
{DUMP_KEY}
{DRAIN}
"
    )
}

/// `cl` on the LAST character: under an operator `vi-forward-char` may
/// reach the end of the line (`zle_move.c:674-675` only pulls the limit
/// back when `!virangeflag`), so the range covers that character.
#[test]
fn cl_changes_the_last_character_of_the_buffer() {
    assert_same_dump(&vi_one_write("goox\\ecld"), "X02 #1: goox ESC cld");
}

/// `c%` forward to the matching bracket: with `virangeflag` set,
/// `vimatchbracket` steps past the match so the range includes it, and
/// scans forward from a non-bracket to the first bracket on the line
/// (`zle_move.c:601-656`).
#[test]
fn c_percent_changes_through_the_matching_bracket() {
    assert_same_dump(&vi_one_write("{ ({[}]) }\\e0c%chg"), "X02 #2: c% forward");
}

/// `d%` backwards: the mark is advanced over the starting bracket so the
/// range includes it (`zle_move.c:637-638`).
#[test]
fn d_percent_deletes_back_to_the_matching_bracket() {
    assert_same_dump(&vi_one_write("s( match )\\ed%"), "X02 #3: d% backward");
}

/// `"a` / `"A` read the register name as the NEXT key
/// (`visetbuffer`, `zle_vi.c:1032` `getfullchar(0)`); uppercase appends.
#[test]
fn named_register_set_and_appended() {
    assert_same_dump(
        &vi_one_write("first\\e\"ay0ddasecond\\e\"Add\"aP"),
        "X02 #27: appending to named register",
    );
}

/// A cut into a named register ALSO lands in the unnamed cut buffer:
/// `cuttext` falls through from the `MOD_VIBUF` arm to the cutbuf update
/// (`zle_utils.c:964-1043`), so a plain `P` pastes it.
#[test]
fn named_register_cut_also_sets_the_unnamed_register() {
    assert_same_dump(
        &vi_one_write("err\\eddahello\\e\"hddP"),
        "X02 #26: setting named register also sets unnamed register",
    );
}

/// `D` kills to the end of the line through `forekill`, so the text goes
/// to register "1 (`vikilleol`, `zle_vi.c:1060-1071`).
#[test]
fn D_puts_the_killed_text_in_register_one() {
    assert_same_dump(
        &vi_one_write("yankee doodle\\ebhDyy0\"1P"),
        "X02 #24: paste register 1 to get last deletion",
    );
}

/// `.` right after the first insert repeats that insert: zleread starts
/// the change for a line that opens in viins (`zle_main.c:1318-1319`
/// `viinsert_init()`), and the `.` that the ESC lookahead read and gave
/// back is not left inside the recorded change (which made `.` replay
/// itself forever).
#[test]
fn dot_repeats_the_initial_insert() {
    assert_same_dump(&vi_one_write("text\\e."), "X02 #42: repeat initial edit");
}

/// `v` after an operator forces a character-wise range
/// (`visualmode`, `zle_move.c:518-523`).
#[test]
fn v_after_an_operator_forces_a_characterwise_range() {
    assert_same_dump(
        &vi_one_write("keepnot\\eo  unwanted\\ekhhcvj "),
        "X02 #54: force character-wise change to join lines",
    );
}

/// `V` after an operator forces a line-wise range
/// (`visuallinemode`, `zle_move.c:542-547`).
#[test]
fn V_after_an_operator_forces_a_linewise_range() {
    assert_same_dump(&vi_one_write("keep\\eOdel\\edVh"), "X02 #56: force line-wise delete");
}

/// `y-` is line-wise because `vi-up-line-or-history` carries
/// `ZLE_LINEMOVE`, which `execzlefunc` turns into `vilinerange = 1`
/// (`zle_main.c:1474-1475`).
#[test]
fn delete_up_is_linewise() {
    assert_same_dump(
        &vi_one_write("long\\eolong\\eo  s\\eolong\\ekjd-k"),
        "X02 #20: delete up clears lastcol",
    );
}

/// A multi-digit count: `handleprefixes` promotes tmult into mult but
/// leaves MOD_TMULT set (`zle_main.c:1620-1627`), so the second digit
/// accumulates (`zle_misc.c:972-980`) and a `0` after a digit is a digit
/// rather than beginning-of-line (`zle_vi.c:1136`).
#[test]
fn a_two_digit_count_accumulates() {
    assert_same_dump(
        &vi_one_write("abcdefghijklmnop\\e010x"),
        "vicmd 10x deletes ten characters",
    );
}

#[test]
fn a_twelve_count_is_twelve_not_two() {
    assert_same_dump(
        &vi_one_write("abcdefghijklmnopqrstu\\e012x"),
        "vicmd 12x deletes twelve characters",
    );
}

#!/usr/bin/env python3
"""parity_corpus.py — the shared case / keystroke / combo tables for the two
completion parity harnesses.

`compsys_parity.py` (drives `zshrs --zsh`, the emulation path) and
`comptab_parity.py` (drives native `zshrs -f -i`, the binary you actually
launch) both import from here, so the corpus cannot drift between them: a case
added for one is exercised by both.

Three tables:

  KEYS            name -> the exact bytes a terminal sends for that key.
  KEY_SEQUENCES   name -> list of key names, i.e. one interaction to replay.
  CASES           the command-line prefixes typed before the keys.

`matrix()` is the cross product, minus the pairs that cannot say anything
(see `_applicable`).

`discover_cases()` builds cases from THIS HOST: every `_name` completer in the
live `$fpath` that also has a `name` binary on `$PATH`. The hand-written CASES
are the fixed, machine-independent floor; discovery is the ceiling (4k+ cases
on the author's box) and is opt-in per run because its size and content depend
on what is installed.

Beyond the fixed tables, the module carries the SHARED fuzz machinery both
harnesses need, so an input found by one is replayable by the other:

  gen_keyseq(rng, n)        random key path (TAB / navigation / filter chars).
  gen_buffer(rng)           random command line drawn from the surface classes
                            the CASES table is grouped by.
  gen_option_set(rng)       random COHERENT set of shell options (`setopt`),
                            the axis zstyle fuzzing cannot reach: `menucomplete`
                            decides whether an ambiguous TAB inserts or lists,
                            `completeinword` where the cursor is when the
                            completer starts, `caseglob` whether a wrong-case
                            path component matches at all.
  mutate_buffer(buf, rng)   one small structured edit to a command line.
  mutate_keys(keys, rng)    one small structured edit to a key path.
  mutate_option_set(o, rng) one small structured edit to an option set.
  fingerprint(a, b)         stable id for a divergence, with digits, paths and
                            hex masked out, so the same bug seen in two cells
                            reports one id instead of two.

The option axis carries its own tables: `SHELL_OPTIONS` (name, `zsh -f`
default, group, doc citation), `OPTION_MASKS` / `OPTION_REQUIRES` /
`OPTION_PAIRS` (the documented interactions, so a generated set never claims to
test an option another member overrides), `OPTION_PROFILES` (coherent bases a
person would actually run), and `OPTION_STYLE_MASKS` (the zstyles that override
an option outright). `option_statements()` renders a set as `setopt`/`unsetopt`
lines, so an option set drops into the same `random_subset()` / `shrink()`
machinery the zstyle combos use and a two-axis failure shrinks across both.
Cases whose outcome an option demonstrably changes carry the `optsens` tag plus
the option's own name (`cases_for_option("completeinword")`).

Every generator is a pure function of the `random.Random` it is handed, so
(seed, index) reproduces an input exactly on any machine; `_validate()` proves
that at import for a fixed sample, along with the table invariants.

Run this file directly to print the tables:

    scripts/parity_corpus.py --list-keys
    scripts/parity_corpus.py --list-cases
    scripts/parity_corpus.py --list-sequences
    scripts/parity_corpus.py --list-discovered
    scripts/parity_corpus.py --list-options
    scripts/parity_corpus.py --gen-options 5
    scripts/parity_corpus.py --check-option-defaults
    scripts/parity_corpus.py --matrix-size
"""

from __future__ import annotations

import argparse
import hashlib
import os
import random
import re
import shutil
import subprocess
from dataclasses import dataclass, field

# ── keystroke vocabulary ─────────────────────────────────────────────────────
#
# Byte sequences are what an xterm-family terminal actually transmits, because
# that is what the line editor parses. Cursor keys are sent in NORMAL (CSI)
# mode, not application (SS3) mode: zsh's `zle-line-init` does not switch the
# keypad, so a real terminal sends CSI here.
KEYS: dict[str, bytes] = {
    # completion
    "tab": b"\t",
    "btab": b"\x1b[Z",          # shift-tab — reverse-menu-complete
    "ctrl-d": b"\x04",          # list-choices (delete-char-or-list on a word)
    "ctrl-o": b"\x0f",          # accept-line-and-down-history
    # cursor / menu navigation
    "up": b"\x1b[A",
    "down": b"\x1b[B",
    "right": b"\x1b[C",
    "left": b"\x1b[D",
    "home": b"\x1b[H",
    "end": b"\x1b[F",
    "pgup": b"\x1b[5~",
    "pgdn": b"\x1b[6~",
    "delete": b"\x1b[3~",       # forward-delete (its own CSI, not ^D)
    "ctrl-p": b"\x10",          # up-line-or-history / menu up
    "ctrl-n": b"\x0e",          # down-line-or-history / menu down
    "ctrl-f": b"\x06",          # forward-char / menu right
    "ctrl-b": b"\x02",          # backward-char / menu left
    "ctrl-a": b"\x01",          # beginning-of-line
    "ctrl-e": b"\x05",          # end-of-line
    # incremental search. `menusel_isearch_back` referenced "ctrl-r" while the
    # table did not define it, and the harness fallback (`KEYS.get(name,
    # name.encode())`) silently transmitted the ASCII text `ctrl-r` — six
    # literal characters self-inserted into the menu instead of the ^R the
    # sequence is named for. `key_bytes()` below now rejects an unknown
    # multi-character name outright so a typo can never masquerade as input.
    "ctrl-r": b"\x12",          # history-incremental-search-backward
    "ctrl-s": b"\x13",          # history-incremental-search-forward
    # accept / abort / edit
    "cr": b"\r",
    "esc": b"\x1b",
    "esc-esc": b"\x1b\x1b",
    "ctrl-c": b"\x03",
    "ctrl-g": b"\x07",          # send-break — abort the menu
    "ctrl-u": b"\x15",          # kill-whole-line
    "ctrl-w": b"\x17",          # backward-kill-word
    "ctrl-h": b"\x08",          # backward-delete-char
    "ctrl-k": b"\x0b",          # kill-line
    "ctrl-y": b"\x19",          # yank
    "bs": b"\x7f",              # backspace
    "ctrl-_": b"\x1f",          # undo
    "ctrl-x-ctrl-x": b"\x18\x18",
    "ctrl-l": b"\x0c",          # clear-screen
    "space": b" ",
    "slash": b"/",
}


class UnknownKey(KeyError):
    """A key sequence named something KEYS does not define."""


def key_bytes(name: str) -> bytes:
    """The bytes for one key name, strictly.

    A single character is taken literally — that is how the interactive
    menuselect filter sequences type `s`, `r`, `c`. Anything longer MUST be a
    defined key name; the old `KEYS.get(name, name.encode())` fallback turned a
    misspelled or missing entry into six self-inserted characters that looked
    like a completion bug on both shells at once (see the `ctrl-r` note above).
    """
    if name in KEYS:
        return KEYS[name]
    if len(name) == 1:
        return name.encode()
    raise UnknownKey(name)


# ── keystroke sequences ──────────────────────────────────────────────────────
#
# Each entry is one interaction worth replaying against every case. The point
# of the matrix is that a completion bug frequently shows up only on the SECOND
# tab (menu entry), or only after moving off the first match, or only on the
# abort path that has to restore the original line.
KEY_SEQUENCES: dict[str, list[str]] = {
    # how many presses before the screen is compared
    "tab1": ["tab"],
    "tab2": ["tab", "tab"],
    "tab3": ["tab", "tab", "tab"],
    # arrow navigation inside the menu
    "tab_down": ["tab", "down"],
    "tab_down2": ["tab", "down", "down"],
    "tab_up": ["tab", "up"],
    "tab_right": ["tab", "right"],
    "tab_left": ["tab", "left"],
    "tab_down_up": ["tab", "down", "up"],
    "tab_right_left": ["tab", "right", "left"],
    # wrap-around: walk far enough to run off the end of the match list
    "tab_wrap_down": ["tab"] + ["down"] * 12,
    "tab_wrap_up": ["tab"] + ["up"] * 12,
    # emacs-key equivalents of the same moves (different widget bindings)
    "tab_ctrl_n": ["tab", "ctrl-n"],
    "tab_ctrl_p": ["tab", "ctrl-p"],
    "tab_ctrl_f": ["tab", "ctrl-f"],
    "tab_ctrl_b": ["tab", "ctrl-b"],
    # paging inside menuselect
    "tab_pgdn": ["tab", "pgdn"],
    "tab_pgup": ["tab", "pgup"],
    "tab_home": ["tab", "home"],
    "tab_end": ["tab", "end"],
    # reverse menu
    "btab1": ["btab"],
    "btab2": ["btab", "btab"],
    "tab_btab": ["tab", "btab"],
    "tab2_btab": ["tab", "tab", "btab"],
    # list without inserting
    "ctrl_d": ["ctrl-d"],
    "ctrl_d2": ["ctrl-d", "ctrl-d"],
    "tab_ctrl_d": ["tab", "ctrl-d"],
    # abort paths — the original line has to come back intact
    "tab_ctrl_g": ["tab", "ctrl-g"],
    "tab_esc": ["tab", "esc"],
    "tab2_ctrl_g": ["tab", "tab", "ctrl-g"],
    "tab_down_ctrl_g": ["tab", "down", "ctrl-g"],
    # accept paths
    "tab_cr": ["tab", "cr"],
    "tab_down_cr": ["tab", "down", "cr"],
    "tab2_cr": ["tab", "tab", "cr"],
    # edit-after-complete: the completion state has to be discarded cleanly
    "tab_bs": ["tab", "bs"],
    "tab_ctrl_w": ["tab", "ctrl-w"],
    "tab_ctrl_u": ["tab", "ctrl-u"],
    "tab_undo": ["tab", "ctrl-_"],
    "tab_delete": ["tab", "delete"],
    # continue typing after a completion
    "tab_slash_tab": ["tab", "slash", "tab"],
    "tab_space_tab": ["tab", "space", "tab"],
    # re-complete after editing / aborting: the second TAB must start from a
    # clean state, not from whatever the first one left behind.
    "tab_bs_tab": ["tab", "bs", "tab"],
    "tab_esc_tab": ["tab", "esc", "tab"],
    "tab_down_tab": ["tab", "down", "tab"],

    # ── cursor NOT at end of line ───────────────────────────────────────
    #
    # Every sequence above completes with the cursor at the end of the
    # buffer, which is the one position `compset -p`/`PREFIX`/`SUFFIX` math
    # cannot get wrong. Moving left first makes the word split into a real
    # prefix and a real suffix, and completing at column 0 exercises the
    # command-position path with text already to the right of the cursor.
    "left_tab": ["left", "tab"],
    "left2_tab": ["left", "left", "tab"],
    "left_tab_tab": ["left", "tab", "tab"],
    "home_tab": ["home", "tab"],
    "ctrl_a_tab": ["ctrl-a", "tab"],
    "bs_tab": ["bs", "tab"],

    # ── redraw after the list is on screen ──────────────────────────────
    #
    # ^L has to repaint the prompt, the command line AND decide what happens
    # to the completion listing below it. That is the exact surface where a
    # multiline prompt made the list climb up the screen, and no sequence
    # above ever redraws once a list exists.
    "tab_ctrl_l": ["tab", "ctrl-l"],
    "tab_down_ctrl_l": ["tab", "down", "ctrl-l"],

    # ── menuselect INTERACTIVE mode ─────────────────────────────────────
    #
    # With `menu select interactive` (zsh/complist), printable characters
    # typed inside the menu FILTER the list live instead of being inserted
    # into the line. That is a whole second keymap — `menuselect` — with its
    # own redraw path, and nothing above reaches it: every sequence so far
    # either stays in the normal keymap or only sends control/cursor keys.
    # Needs a combo that sets the style, e.g. force-menu-select-interactive.
    "menusel_type_s": ["tab", "tab", "s"],
    "menusel_type_word": ["tab", "tab", "s", "r", "c"],
    "menusel_type_nomatch": ["tab", "tab", "q", "z", "x"],
    "menusel_type_bs": ["tab", "tab", "s", "bs"],
    "menusel_type_arrows": ["tab", "tab", "s", "down", "down", "up"],
    "menusel_type_cr": ["tab", "tab", "s", "cr"],
    "menusel_type_ctrl_g": ["tab", "tab", "s", "ctrl-g"],
    "menusel_type_esc": ["tab", "tab", "s", "esc"],
    "menusel_retype": ["tab", "tab", "s", "bs", "d"],
    # `/` starts an incremental search inside menuselect; `^R`/`^S` are the
    # history-incremental-search bindings the menuselect keymap inherits.
    "menusel_slash_search": ["tab", "tab", "slash", "s"],
    "menusel_isearch_back": ["tab", "tab", "ctrl-r", "s"],
    "menusel_isearch_fwd": ["tab", "tab", "ctrl-s", "s"],

    # ── typing a character BETWEEN two completions ──────────────────────
    #
    # Outside an interactive menu the character self-inserts, so the second
    # TAB completes a DIFFERENT word than the first one did. Nothing above
    # re-completes after a self-insert: `tab_slash_tab`/`tab_space_tab` type
    # a word SEPARATOR, which ends the word instead of extending it.
    "tab_char_tab": ["tab", "s", "tab"],
    "tab_char2_tab": ["tab", "s", "r", "tab"],
    "tab_char_bs_tab": ["tab", "s", "bs", "tab"],

    # ── interactive filter: narrow, then widen again ────────────────────
    #
    # `menusel_type_bs` deletes the only filter character. These narrow by
    # two and back off by one or two, which is where the filter has to
    # RECOMPUTE a wider match set rather than drop back to the full list,
    # and where one backspace too many leaves the menuselect keymap.
    "menusel_filter_bs1": ["tab", "tab", "s", "r", "bs"],
    "menusel_filter_bs2": ["tab", "tab", "s", "r", "bs", "bs"],
    "menusel_filter_bs_over": ["tab", "tab", "s", "bs", "bs"],
    "menusel_filter_retype": ["tab", "tab", "s", "r", "bs", "c"],
    "menusel_filter_bs_arrows": ["tab", "tab", "s", "r", "bs", "down", "up"],

    # ── undo ────────────────────────────────────────────────────────────
    #
    # `tab_undo` undoes a single completion. Undo after the menu has been
    # navigated, after a second TAB, and twice in a row are different
    # states: the change group a completion opens has to be closed exactly
    # once no matter how many menu entries were cycled through.
    "tab_undo2": ["tab", "ctrl-_", "ctrl-_"],
    "tab_undo_tab": ["tab", "ctrl-_", "tab"],
    "tab2_undo": ["tab", "tab", "ctrl-_"],
    "tab_down_undo": ["tab", "down", "ctrl-_"],
    "tab_accept_undo": ["tab", "down", "space", "ctrl-_"],

    # ── accept-and-hold (^O) ────────────────────────────────────────────
    #
    # ^O is accept-line-and-down-history in BOTH shells (verified with
    # `bindkey "^O"` under `-f`), so it is a comparable path: it accepts the
    # line WITH a menu still open, which has to tear the menu down and leave
    # the next prompt clean.
    "tab_ctrl_o": ["tab", "ctrl-o"],
    "tab_down_ctrl_o": ["tab", "down", "ctrl-o"],
    "tab_ctrl_o2": ["tab", "ctrl-o", "ctrl-o"],

    # ── list, then complete / abort ─────────────────────────────────────
    "tab_ctrl_d_ctrl_g": ["tab", "ctrl-d", "ctrl-g"],
    "tab_ctrl_d_tab": ["tab", "ctrl-d", "tab"],
    "ctrl_d_tab_ctrl_g": ["ctrl-d", "tab", "ctrl-g"],

    # ── long walks in the REVERSE direction ─────────────────────────────
    #
    # `tab_wrap_up` walks 12 up; these wrap backwards through the reverse
    # widget, the emacs binding and the horizontal axis, and walk far enough
    # to wrap TWICE and to come back to the entry they started on. A list
    # that scrolls correctly forwards can still leave the pager one row off
    # coming back.
    "tab_wrap_btab": ["tab"] + ["btab"] * 12,
    "tab_wrap_left": ["tab"] + ["left"] * 12,
    "tab_wrap_ctrl_p": ["tab"] + ["ctrl-p"] * 12,
    "tab_wrap_pgup": ["tab"] + ["pgup"] * 4,
    "tab_wrap_down_up": ["tab"] + ["down"] * 12 + ["up"] * 12,
    "tab_wrap_up_down": ["tab"] + ["up"] * 12 + ["down"] * 12,
    "tab_wrap_btab_tab": ["tab"] + ["btab"] * 12 + ["tab"] * 12,

    # ── deeper mid-word positions ───────────────────────────────────────
    #
    # `left_tab`/`left2_tab` stop one or two characters in. Three characters
    # back lands inside a path COMPONENT rather than on its separator, and
    # re-completing after moving right again has to rebuild PREFIX/SUFFIX
    # from a cursor that moved without an edit.
    "left3_tab": ["left", "left", "left", "tab"],
    "left_tab_right_tab": ["left", "tab", "right", "tab"],
    "left_tab_bs_tab": ["left", "tab", "bs", "tab"],
    "left_ctrl_d": ["left", "ctrl-d"],
    "left2_tab_tab": ["left", "left", "tab", "tab"],
}

# One sequence per printable filter character. Menu filtering is per-CHARACTER
# work (the menuselect keymap dispatches each byte), so "typing filters" is not
# a single behaviour to spot-check — a digit and a letter take different paths,
# and a character that matches nothing takes a third. These are generated
# rather than typed out so the full a-z0-9 range is covered without 36
# hand-written entries drifting out of sync.
MENUSELECT_FILTER_CHARS = "abcdefghijklmnopqrstuvwxyz0123456789"
for _ch in MENUSELECT_FILTER_CHARS:
    KEY_SEQUENCES[f"menusel_char_{_ch}"] = ["tab", "tab", _ch]
del _ch

# A small default battery for runs that cannot afford the full matrix. Chosen
# to hit one of each shape: single, menu-entry, navigation, reverse, list,
# abort, accept, mid-word, redraw.
#
# All FOUR arrow directions are in the default set, not just `down`. They are
# not interchangeable: `cd /<TAB><UP>` diverges (zsh clears the completion
# listing when stepping back off the first entry, zshrs leaves all four list
# lines on screen) while `<DOWN>`, `<RIGHT>` and `<LEFT>` on the same case all
# pass. A battery carrying only `tab_down` reports that case green and the bug
# stays invisible.
DEFAULT_SEQUENCES = [
    "tab1",
    "tab2",
    "tab_down",
    "tab_up",
    "tab_right",
    "tab_left",
    "tab_down_up",
    "tab_wrap_down",
    "tab_wrap_up",
    "tab_btab",
    "ctrl_d",
    "tab_ctrl_g",
    "tab_cr",
    # cursor not at end of buffer, and a repaint with a list on screen —
    # neither shape was reachable from any other default sequence.
    "left_tab",
    "tab_ctrl_l",
    # menuselect interactive filtering — only meaningful under a combo that
    # sets `menu select interactive`, but harmless elsewhere (the characters
    # just self-insert on both shells, which must ALSO match).
    "menusel_type_s",
    "menusel_type_word",
    "menusel_type_nomatch",
    "menusel_type_bs",
    "menusel_type_arrows",
    "menusel_type_ctrl_g",
]

# A second, OPT-IN battery: the paths added after DEFAULT_SEQUENCES was fixed.
# It is deliberately NOT merged into DEFAULT_SEQUENCES — that list sizes every
# routine sweep, and silently multiplying it would change the runtime and the
# baseline of every harness that imports this module. A run that wants the
# wider space asks for it (`--sequences "$(...)"` / `FUZZ_SEQUENCES`).
FUZZ_SEQUENCES = [
    "tab_char_tab",
    "tab_char_bs_tab",
    "menusel_filter_bs2",
    "menusel_filter_bs_over",
    "menusel_filter_retype",
    "tab_undo2",
    "tab_undo_tab",
    "tab_down_undo",
    "tab_accept_undo",
    "tab_ctrl_o",
    "tab_ctrl_d_ctrl_g",
    "tab_wrap_btab",
    "tab_wrap_left",
    "tab_wrap_ctrl_p",
    "tab_wrap_down_up",
    "tab_wrap_up_down",
    "left3_tab",
    "left_tab_right_tab",
    "left_tab_bs_tab",
]


@dataclass
class Case:
    """One command line typed before the keystrokes run."""

    name: str
    buffer: str
    note: str = ""
    # Free-form labels used to select subsets and to skip inapplicable pairs.
    tags: tuple[str, ...] = field(default_factory=tuple)


# ── cases ────────────────────────────────────────────────────────────────────
#
# Grouped by the completion surface each one exercises. Anything needing a
# binary that may not exist on the host is tagged `optional`; the harness skips
# a case when neither shell can complete it, rather than reporting a false diff.
CASES: list[Case] = [
    # command position
    Case("cmd_empty", "", "command position — every command, function, alias", ("cmd", "huge")),
    Case("cmd_partial", "pr", "partial command name across builtins/functions/path", ("cmd",)),
    Case("cmd_dash", "-", "leading dash in command position", ("cmd",)),
    Case("cmd_tilde", "~", "tilde in command position", ("cmd",)),
    # paths
    Case("path_root", "cd /", "top-level directory completion", ("path",)),
    Case("path_deep", "cd /usr/", "second-level directory completion", ("path",)),
    Case("path_home", "cd ~/", "home-relative directory completion", ("path",)),
    Case("path_dot", "ls ./", "explicit-cwd path completion", ("path",)),
    Case("path_dotdot", "ls ../", "parent-relative path completion", ("path",)),
    Case("path_partial", "ls /us", "unique-prefix directory completion", ("path",)),
    Case("path_glob", "ls *", "glob in the word being completed", ("path", "glob")),
    Case("path_space", "ls /Applications/", "path whose entries contain spaces", ("path",)),
    Case("path_squeeze", "ls //usr//", "doubled slashes — squeeze-slashes style", ("path",)),
    Case("redir_target", "echo x > /tm", "redirection target completion", ("path", "redir")),
    Case("redir_in", "cat < /etc/ho", "input-redirection completion", ("path", "redir")),
    # options
    Case("opt_ssh", "ssh -", "short option completion", ("opt",)),
    Case("opt_ls", "ls -", "short options, dense list", ("opt",)),
    Case("opt_grep", "grep -", "short options with descriptions", ("opt",)),
    Case("opt_long", "git log --", "long-option completion", ("opt", "git")),
    Case("opt_long_partial", "git log --onel", "unique long option", ("opt", "git")),
    Case("opt_after_ddash", "git log -- ", "operand position after `--`", ("opt", "git")),
    Case("opt_tar", "tar -", "options for an external with many flags", ("opt",)),
    Case("opt_zsh", "zsh -", "options for zsh itself", ("opt",)),
    Case("opt_typeset", "typeset -", "builtin option completion", ("opt", "builtin")),
    Case("opt_bindkey", "bindkey -", "builtin option completion", ("opt", "builtin")),
    # subcommands
    Case("sub_git", "git ", "subcommand list with group-name grouping", ("sub", "git")),
    Case("sub_git_partial", "git chec", "single-candidate unique completion", ("sub", "git")),
    Case("sub_git_ref", "git checkout ", "ref completion inside a repo", ("sub", "git")),
    Case("sub_git_add", "git add ", "modified-file completion inside a repo", ("sub", "git")),
    Case("sub_brew", "brew ", "subcommands from a site-functions completer", ("sub", "optional")),
    Case("sub_cargo", "cargo ", "subcommand list", ("sub", "optional")),
    Case("sub_docker", "docker ", "subcommand list", ("sub", "optional")),
    Case("sub_tmux", "tmux ", "subcommand list", ("sub", "optional")),
    Case("sub_npm", "npm ", "subcommand list", ("sub", "optional")),
    Case("sub_zinit", "zinit ", "zinit subcommands — user's own plugin manager", ("sub", "optional")),
    # parameters
    Case("param_scalar", "echo $PA", "parameter-name completion", ("param",)),
    Case("param_brace", "echo ${PA", "braced parameter-name completion", ("param",)),
    Case("param_subscript", "echo $path[", "array subscript completion", ("param",)),
    Case("param_assoc", "echo $commands[", "assoc-key completion on a magic hash", ("param",)),
    Case("param_flag", "echo ${(", "parameter-expansion flag completion", ("param",)),
    Case("param_unset", "unset ", "parameter names for unset", ("param", "builtin")),
    # builtins with their own completers
    Case("builtin_zstyle", "zstyle ", "zstyle context completion", ("builtin",)),
    Case("builtin_zstyle_ctx", "zstyle ':completion:*' ", "zstyle style-name completion", ("builtin",)),
    Case("builtin_zmodload", "zmodload ", "module-name completion", ("builtin",)),
    Case("builtin_setopt", "setopt ", "option-name completion", ("builtin",)),
    Case("builtin_unsetopt", "unsetopt no", "negated option-name completion", ("builtin",)),
    Case("builtin_kill", "kill -", "signal completion", ("builtin",)),
    Case("builtin_fc", "fc -", "fc options", ("builtin",)),
    Case("builtin_print", "print -", "print options", ("builtin",)),
    Case("builtin_functions", "functions ", "function-name completion", ("builtin",)),
    Case("builtin_which", "which ", "command-name completion via which", ("builtin",)),
    Case("builtin_man", "man ", "manpage completion — very large match set", ("huge",)),
    # external commands with large/awkward option sets (migrated from
    # comptab_parity.BUILTIN_CORPUS so there is a single corpus)
    Case("opt_wget", "wget -", "long option set", ("opt", "optional")),
    Case("opt_curl", "curl -", "very large option set", ("opt", "optional")),
    Case("opt_find", "find -", "options that look like operands", ("opt",)),
    Case("opt_ps", "ps -", "BSD/GNU-divergent option set", ("opt",)),
    Case("opt_du", "du -", "short options", ("opt",)),
    Case("opt_df", "df -", "short options", ("opt",)),
    Case("opt_date", "date -", "short options", ("opt",)),
    Case("opt_mkdir", "mkdir -", "short options", ("opt",)),
    Case("opt_cp", "cp -", "short options", ("opt",)),
    Case("opt_mv", "mv -", "short options", ("opt",)),
    Case("opt_rm", "rm -", "short options", ("opt",)),
    Case("opt_jq", "jq -", "short options", ("opt", "optional")),
    Case("arg_chmod", "chmod ", "mode-argument completion", ("sub",)),
    Case("sub_rustup", "rustup ", "subcommand list", ("sub", "optional")),
    # glob qualifiers / patterns
    Case("glob_qual", "ls *(", "glob-qualifier completion", ("glob",)),
    Case("glob_recursive", "ls **/", "recursive glob completion", ("glob",)),
    # user's own surface
    Case("zpwr_verb", "zpwr ", "zpwr verb completion", ("optional", "zpwr")),

    # ── `--opt=` : the argument half of a long option ────────────────────
    #
    # `--opt<TAB>` and `--opt=<TAB>` are different code paths: the second one
    # has to strip the option name with `compset -P '*='`, look up that
    # option's ARGUMENT spec, and complete against it with an ignored prefix.
    # Nothing above reached it — every option case stopped at the name.
    Case("opteq_git_format", "git log --format=", "argument after `=` on a long option", ("opt", "opteq", "git")),
    Case("opteq_git_pretty", "git log --pretty=", "named-value argument after `=`", ("opt", "opteq", "git")),
    Case("opteq_partial", "git log --format=medi", "partial argument after `=`", ("opt", "opteq", "git")),
    Case("opteq_ls_color", "ls --color=", "GNU-style `=` argument", ("opt", "opteq", "optional")),
    Case("opteq_grep_color", "grep --color=", "GNU-style `=` argument", ("opt", "opteq", "optional")),

    # ── quoted words ─────────────────────────────────────────────────────
    #
    # Inside quotes the word is not word-split, the completer has to re-quote
    # what it inserts, and `compquote`/`compset -q` decide what the suffix
    # looks like. A path case outside quotes says nothing about any of it.
    Case("quote_dq_path", 'ls "/us', "path completion inside double quotes", ("quote", "path")),
    Case("quote_sq_path", "ls '/us", "path completion inside single quotes", ("quote", "path")),
    Case("quote_dq_param", 'echo "$PA', "parameter completion inside double quotes", ("quote", "param")),
    Case("quote_bs_space", "ls /Applications/Ut", "path needing a backslash-escaped space", ("quote", "path")),
    Case("quote_dq_open", 'echo "', "completion just inside an opened quote", ("quote",)),

    # ── `$var` in the word being completed ───────────────────────────────
    Case("var_in_path", "ls $HOME/", "path completion through a parameter", ("param", "path")),
    Case("var_brace_path", "ls ${HOME}/", "path completion through a braced parameter", ("param", "path")),
    Case("var_partial", "ls $HOM", "parameter name mid-word", ("param",)),

    # ── `~user` / named directories ──────────────────────────────────────
    Case("tilde_user", "ls ~root/", "path under another user's home", ("path", "tilde")),
    Case("tilde_partial", "ls ~ro", "username completion after `~`", ("path", "tilde")),

    # ── precommands ──────────────────────────────────────────────────────
    #
    # `sudo`/`command`/`env` shift the command position one word right. The
    # completer has to recognise the precommand and re-dispatch, which is a
    # different path from plain command completion.
    Case("pre_sudo", "sudo ", "command position after a precommand", ("cmd", "pre", "huge")),
    Case("pre_sudo_partial", "sudo gi", "partial command after a precommand", ("cmd", "pre")),
    Case("pre_sudo_args", "sudo git ", "subcommand two words after a precommand", ("sub", "pre", "git")),
    Case("pre_command", "command l", "partial command after `command`", ("cmd", "pre")),
    Case("pre_env", "env ", "command position after `env`", ("cmd", "pre", "huge")),
    Case("pre_assign", "FOO=bar l", "command position after a prefix assignment", ("cmd", "pre")),

    # ── command position that is NOT the first word ──────────────────────
    Case("cmd_after_pipe", "ls | gr", "command position after a pipe", ("cmd", "compound")),
    Case("cmd_after_semi", "true; gr", "command position after `;`", ("cmd", "compound")),
    Case("cmd_after_andand", "true && gr", "command position after `&&`", ("cmd", "compound")),
    Case("cmd_in_subshell", "(gr", "command position inside an unclosed subshell", ("cmd", "compound")),
    Case("cmd_in_cmdsubst", "echo $(gr", "command position inside `$(`", ("cmd", "compound")),
    Case("cmd_in_backtick", "echo `gr", "command position inside a backtick", ("cmd", "compound")),
    Case("cmd_after_for", "for f in /us", "word inside a `for` list", ("compound", "path")),
    Case("cond_test", "[[ -", "condition operator completion", ("compound",)),

    # ── more redirection shapes ──────────────────────────────────────────
    Case("redir_append", "echo x >> /tm", "append-redirection target", ("path", "redir")),
    Case("redir_fd", "echo x 2> /tm", "fd-qualified redirection target", ("path", "redir")),
    Case("redir_herestring", "cat <<< $PA", "here-string operand", ("param", "redir")),

    # ── namespaces with their own completers ─────────────────────────────
    Case("ns_unalias", "unalias ", "alias-name completion", ("builtin",)),
    Case("ns_unfunction", "unfunction ", "function-name completion", ("builtin",)),
    Case("ns_bindkey_widget", "zle ", "widget-name completion", ("builtin",)),
    Case("ns_hash", "hash -d ", "named-directory assignment", ("builtin",)),
    Case("ns_jobs", "fg %", "job-spec completion", ("builtin",)),
    Case("ns_kill_signame", "kill -s ", "signal NAME completion (argument of -s)", ("builtin", "opteq")),
    Case("ns_chown_user", "chown ", "username completion", ("sub",)),
    Case("ns_ssh_host", "ssh ", "hostname completion from known_hosts/config", ("sub", "host")),
    Case("ns_make_target", "make ", "makefile target completion", ("sub", "optional")),

    # ── brace / glob shapes not covered above ────────────────────────────
    Case("brace_expand", "ls /usr/{b", "word inside a brace expansion", ("glob", "path")),
    Case("glob_suffix", "ls /usr/bin/z*", "trailing glob with a literal prefix", ("glob", "path")),
    Case("glob_qual_partial", "ls *(.", "partially typed glob qualifier", ("glob",)),

    # ── words the cursor sits INSIDE ─────────────────────────────────────
    #
    # Every case above is completed with the cursor at the end of the buffer.
    # These are written to be paired with the `left*_tab` / `home_tab`
    # sequences: the word then has a real SUFFIX, so `compset -p`, PREFIX,
    # SUFFIX and the inserted-suffix logic all have something to get wrong.
    Case("midword_path", "ls /usr/share/zsh", "cursor inside a path component, text to its right", ("path", "midword")),
    Case("midword_opt", "git log --oneline", "cursor inside a long option name", ("opt", "midword", "git")),
    Case("midword_arg", "grep pattern file", "cursor inside a middle word of three", ("midword",)),

    # ── quoted words with nothing typed yet ──────────────────────────────
    #
    # `quote_dq_path`/`quote_sq_path` already carry a partial path inside the
    # quotes. An EMPTY quote is the harder shape: the completer has to decide
    # the word is quoted from the opening character alone, and re-quote every
    # match it inserts with no existing text to pattern off.
    Case("quote_dq_bare", 'ls "', "completion just inside an opened double quote", ("quote", "path")),
    Case("quote_sq_bare", "ls '", "completion just inside an opened single quote", ("quote", "path")),
    Case("quote_bs_escaped_space", "ls /Applications/Google\\ ", "backslash-escaped space inside a path word", ("quote", "path", "optional")),
    Case("quote_dq_var_path", 'ls "$HOME/', "path through a parameter inside double quotes", ("quote", "param", "path")),

    # ── parameter expansion with nothing typed after the sigil ───────────
    Case("param_bare", "echo $", "every parameter name — the widest param set", ("param", "huge")),
    Case("param_bare_brace", "echo ${", "every parameter name, braced", ("param", "huge")),
    Case("param_brace_subscript", "echo ${path[", "subscript inside a braced expansion", ("param",)),
    Case("param_subscript_flag", "echo $path[(", "subscript-FLAG completion, not an index", ("param",)),
    Case("param_assoc_key_partial", "echo $commands[z", "partial assoc key inside a subscript", ("param",)),
    Case("param_arith", "echo $((", "arithmetic context — parameters without a sigil", ("param", "arith")),
    Case("param_arith_partial", "echo $((RAN", "partial parameter name inside arithmetic", ("param", "arith")),
    Case("param_arith_subscript", "echo $arr[$((", "arithmetic nested inside a subscript", ("param", "arith")),

    # ── glob qualifier interior ──────────────────────────────────────────
    Case("glob_qual_dir", "ls *(/", "qualifier list continuing after a type qualifier", ("glob",)),
    Case("glob_qual_order", "ls *(om", "ordering qualifier — takes its own argument", ("glob",)),
    Case("glob_qual_mod", "ls *(.:", "history-style modifier after a qualifier", ("glob",)),

    # ── brace expansion ──────────────────────────────────────────────────
    Case("brace_alt", "ls {a,", "second alternative of a brace expansion", ("glob",)),
    Case("brace_alt_path", "ls {/usr/b,/etc/h", "brace alternatives that are paths", ("glob", "path")),

    # ── command substitution / process substitution interiors ────────────
    #
    # `cmd_in_cmdsubst` stops at the command word. These continue PAST it, so
    # the inner command's own completer has to run in a nested parse context.
    Case("cmdsubst_arg", "echo $(git ", "subcommand completion inside `$(`", ("sub", "compound", "git")),
    Case("cmdsubst_path", "echo $(ls /us", "path completion inside `$(`", ("path", "compound")),
    Case("procsubst_in", "diff <(", "command position inside `<(`", ("cmd", "compound", "huge")),
    Case("procsubst_arg", "diff <(git ", "subcommand inside `<(`", ("sub", "compound", "git")),
    Case("procsubst_out", "tee >(", "command position inside `>(`", ("cmd", "compound", "huge")),

    # ── more precommands ─────────────────────────────────────────────────
    Case("pre_noglob", "noglob ls /us", "path completion under `noglob`", ("pre", "path")),
    Case("pre_nocorrect", "nocorrect gi", "command position after `nocorrect`", ("pre", "cmd")),
    Case("pre_builtin", "builtin ", "builtin-name position after `builtin`", ("pre", "builtin", "huge")),
    Case("pre_sudo_opt", "sudo -", "sudo's OWN options, not the command it runs", ("opt", "pre", "optional")),

    # ── alias / equals expansion in command position ─────────────────────
    #
    # Both are host-dependent (`ll` has to be aliased, `=ls` needs an `ls` on
    # $PATH and the EQUALS option), hence `optional`.
    Case("alias_cmd", "ll -", "options completed for an ALIASED command name", ("cmd", "alias", "optional")),
    Case("equals_cmd", "=ls", "equals-expansion in command position", ("cmd", "equals", "optional")),
    Case("equals_arg", "echo =gr", "equals-expansion in an argument word", ("equals", "optional")),

    # ── history expansion ────────────────────────────────────────────────
    Case("hist_bang", "echo !", "history-expansion word — the `!` must not be completed as a glob", ("hist",)),
    Case("hist_bang_cmd", "!gi", "history-expansion in command position", ("hist", "cmd")),
    Case("hist_modifier", "echo !!:", "history word modifier", ("hist",)),

    # ── assignment right-hand sides ──────────────────────────────────────
    #
    # `pre_assign` completes the COMMAND after an assignment. These complete
    # the assignment's VALUE, which is a path context the parser has to reach
    # through the `=`, and (for PATH/fpath) a colon-separated list.
    Case("assign_rhs_path", "FOO=/us", "path completion on an assignment RHS", ("assign", "path")),
    Case("assign_rhs_tilde", "FOO=~/", "tilde path on an assignment RHS", ("assign", "path", "tilde")),
    Case("assign_rhs_colon", "PATH=/usr/bin:/us", "second element of a colon-separated path list", ("assign", "path")),
    Case("assign_array", "fpath=(/us", "path inside an array-assignment literal", ("assign", "path")),
    Case("assign_typeset", "typeset FOO=/us", "assignment RHS as an argument of typeset", ("assign", "path", "builtin")),

    # ── reserved-word contexts ───────────────────────────────────────────
    Case("kw_function", "function ", "function-definition position — a name, not a command", ("kw",)),
    Case("kw_always", "{ true } always { tr", "command position inside an `always` block", ("kw", "cmd", "compound")),
    Case("kw_case", "case $x in ", "pattern position of a `case`", ("kw", "compound")),
    Case("kw_case_body", "case $x in *) tr", "command position inside a `case` arm", ("kw", "cmd", "compound")),
    Case("kw_do", "for f in a b; do tr", "command position inside a `do` block", ("kw", "cmd", "compound")),
    Case("kw_if", "if tr", "command position after `if`", ("kw", "cmd", "compound")),
    Case("kw_coproc", "coproc gr", "command position after `coproc`", ("kw", "cmd")),

    # ── shell OPTIONS change the answer ──────────────────────────────────
    #
    # Every case above is completed under one shell configuration: the `zsh -f`
    # defaults. These are written so that a named option demonstrably moves the
    # outcome — a mid-word cursor for COMPLETE_IN_WORD, an ambiguous prefix
    # with nothing to insert for AUTO_LIST, a wrong-case component for
    # CASE_GLOB. Each carries the `optsens` tag plus the NAME of every option it
    # exercises, so `cases_for_option("recexact")` selects them and
    # `OPTION_CASE_SEQUENCES` names the key paths that make them speak.
    Case("optsens_ciw_midword", "ls /usr/bin",
         "with the cursor moved back three, the word is prefix `/usr/` + suffix "
         "`bin`: COMPLETE_IN_WORD matches from both ends (bin, sbin), otherwise "
         "the cursor jumps to the end first and `/usr/bin` is unique",
         ("path", "midword", "optsens", "completeinword", "alwaystoend")),
    Case("optsens_to_end", "cd /usr/bin",
         "the same mid-word shape for ALWAYS_TO_END, which decides where the "
         "cursor lands once a full completion is inserted",
         ("path", "midword", "optsens", "alwaystoend", "completeinword")),
    Case("optsens_autolist", "ls /usr/s",
         "ambiguous with NOTHING to insert (share/sbin/standalone share only "
         "the typed `s`) — AUTO_LIST lists on the first TAB, BASH_AUTO_LIST "
         "waits for the second",
         ("path", "optsens", "autolist", "bashautolist", "listambiguous")),
    Case("optsens_listambiguous", "ls /usr/li",
         "ambiguous WITH an unambiguous prefix to insert (lib, libexec share "
         "`lib`) — the one shape LIST_AMBIGUOUS changes",
         ("path", "optsens", "listambiguous", "autolist", "bashautolist")),
    Case("optsens_recexact", "ls /usr/lib",
         "the typed word is itself a match and another match extends it "
         "(libexec) — REC_EXACT accepts the exact one",
         ("path", "optsens", "recexact")),
    Case("optsens_menu", "cd /usr/l",
         "an ambiguous set: MENU_COMPLETE inserts the first match at once, "
         "AUTO_MENU only on the second request",
         ("path", "optsens", "menucomplete", "automenu")),
    Case("optsens_listing", "ls /usr/share/",
         "a listing wide enough for LIST_PACKED, LIST_ROWS_FIRST and "
         "LIST_TYPES to rearrange it",
         ("path", "optsens", "listpacked", "listrowsfirst", "listtypes",
          "listbeep")),
    Case("optsens_globcomplete", "ls /usr/l*",
         "a glob in the word — GLOB_COMPLETE generates matches and cycles them "
         "instead of inserting the whole expansion",
         ("glob", "path", "optsens", "globcomplete", "completeinword")),
    Case("optsens_caseglob", "ls /usr/BI",
         "a wrong-case path component — with NO_CASE_GLOB and no matcher set, "
         "file matching is case-insensitive (compsys.yo:2138-2141)",
         ("path", "optsens", "caseglob")),
    Case("optsens_globdots", "cat ~/",
         "a word that does NOT start with a dot — GLOB_DOTS decides whether "
         "dotfiles are offered at all",
         ("path", "optsens", "globdots")),
    Case("optsens_nomatch", "ls /usr/zzzz*",
         "a pattern with no matches — NOMATCH errors, NULL_GLOB and "
         "CSH_NULL_GLOB delete it instead",
         ("glob", "path", "optsens", "nomatch", "nullglob", "cshnullglob")),
    Case("optsens_markdirs", "echo /usr/l*",
         "a glob whose matches are directories — MARK_DIRS appends a slash to "
         "each",
         ("glob", "path", "optsens", "markdirs")),
    Case("optsens_numericsort", "ls /usr/share/zoneinfo/Etc/GMT*",
         "numeric filenames (GMT-1, GMT-10, GMT-2 sort differently numerically "
         "than lexicographically) — NUMERIC_GLOB_SORT reorders the listing. "
         "NOT /dev/tty*: that set changes between the two shells' pty sessions "
         "as terminals come and go, which made the cell permanently FLAKY "
         "(16, 13, 12 rows differing, never reproducibly) — a case whose match "
         "set is not stable measures the host, not the shell",
         ("glob", "path", "optsens", "numericglobsort", "optional")),
    Case("optsens_equals", "cat =ls",
         "`=cmd` filename expansion — EQUALS decides whether it resolves to a "
         "path or stays literal",
         ("equals", "optsens", "optional")),
    Case("optsens_magicequal", "echo foo=~/",
         "an argument that merely LOOKS like an assignment — "
         "MAGIC_EQUAL_SUBST expands the `~` after the `=`",
         ("assign", "path", "optsens", "magicequalsubst", "kshtypeset")),
    Case("optsens_bareglobqual", "ls /usr/*(",
         "a trailing parenthesis — BARE_GLOB_QUAL decides whether it opens a "
         "qualifier list or is literal text",
         ("glob", "optsens", "bareglobqual", "extendedglob")),
    Case("optsens_extendedglob", "ls /usr/^l",
         "`^` in the word — EXTENDED_GLOB makes it negation rather than an "
         "ordinary character",
         ("glob", "optsens", "extendedglob")),
    Case("optsens_ignorebraces", "echo /usr/{b",
         "a brace expansion — IGNORE_BRACES turns it into literal text",
         ("glob", "optsens", "ignorebraces")),
    Case("optsens_pathdirs", "bin/l",
         "a command name containing a slash — PATH_DIRS has it searched along "
         "$PATH anyway",
         ("cmd", "optsens", "pathdirs", "hashlistall")),
    Case("optsens_autocd", "/usr/b",
         "a bare directory in command position — AUTO_CD makes it a command",
         ("cmd", "path", "optsens", "autocd")),
    Case("optsens_banghist", "cat !",
         "a `!` word — BANG_HIST decides whether it is history expansion or a "
         "literal character to complete after",
         ("hist", "optsens", "banghist")),
    Case("optsens_rcquotes", "ls 'don''t",
         "a doubled single quote inside single quotes — RC_QUOTES makes it one "
         "literal quote, so the word is still open",
         ("quote", "optsens", "rcquotes")),
    Case("optsens_paramslash", "cd $HOM",
         "a parameter whose value is a directory — AUTO_PARAM_SLASH appends "
         "`/` rather than a space",
         ("param", "optsens", "autoparamslash", "autonamedirs")),
    Case("optsens_paramkeys", "echo ${HOM",
         "a braced parameter — AUTO_PARAM_KEYS removes the auto-inserted "
         "character when the next one typed has to follow the name directly",
         ("param", "optsens", "autoparamkeys")),
    Case("optsens_removeslash", "ls /usr",
         "a completion that ends in a slash, followed by a delimiter — "
         "AUTO_REMOVE_SLASH takes the slash back",
         ("path", "optsens", "autoremoveslash", "autoparamslash")),
    Case("optsens_shwordsplit", "ls $PATH/",
         "an unquoted parameter expansion in the word — SH_WORD_SPLIT field-"
         "splits it and GLOB_SUBST makes the result glob-eligible",
         ("param", "path", "optsens", "shwordsplit", "globsubst")),
    Case("optsens_cbases", "echo $((0x",
         "hexadecimal in arithmetic — C_BASES changes the form the value "
         "prints back in",
         ("arith", "param", "optsens", "cbases")),
    Case("optsens_kshoptionprint", "setopt no",
         "option-name completion — KSH_OPTION_PRINT changes how `setopt` "
         "reports state, which is the text option completion reads",
         ("builtin", "optsens", "kshoptionprint")),
    Case("optsens_aliases", "ll --",
         "options for an ALIASED name — ALIASES decides whether the alias "
         "exists and COMPLETE_ALIASES whether it is expanded before completing",
         ("cmd", "alias", "optsens", "aliases", "completealiases", "optional")),
    Case("optsens_correct", "grpe /usr/",
         "a misspelled command word — CORRECT and CORRECT_ALL offer an "
         "interactive spelling prompt on the accepted line",
         ("cmd", "optsens", "correct", "correctall", "optional")),
]

# Sequences that cannot say anything for a given case tag, so the matrix skips
# them instead of burning a pty round-trip on a guaranteed-identical screen.
# Case names are ids: they key JSON results, report rows and `--case` lookups
# across commits, so they are restricted to a shape that survives all three.
_CASE_NAME_RE = re.compile(r"^[a-z][a-z0-9_]*$")

# `setopt` spellings: lowercase, no underscores — the form this corpus emits, so
# that one option has exactly one id (`setopt complete_in_word` and `setopt
# completeinword` are the same option and must not become two).
_OPTION_NAME_RE = re.compile(r"^[a-z][a-z0-9]*$")

# Every option, mask and pair has to point at the line of the zsh doc tree that
# states the behaviour. A citation-shaped string is not proof the line says what
# the note claims, but an EMPTY one is proof nobody looked.
_CITE_RE = re.compile(r"^(options|compsys|zshexpn|zshmodules)\.yo:\d+(-\d+)?"
                      r"(, ?(options|compsys|zshexpn|zshmodules)\.yo:\d+(-\d+)?)*$")

# Tags that describe the completion SURFACE rather than a shell option. Kept as
# a closed set so a typo in an option-sensitive case's tags fails at import
# instead of silently selecting nothing.
_SURFACE_TAGS = frozenset({
    "adhoc", "alias", "arith", "assign", "auto", "builtin", "cmd", "compound",
    "equals", "git", "glob", "hist", "host", "huge", "kw", "midword",
    "opt", "opteq", "optional", "optsens", "param", "path", "pre", "quote",
    "redir", "sub", "tilde", "zpwr",
})

# Surface tags that happen to spell an option name. `equals` was a SURFACE tag
# on `equals_cmd` / `equals_arg` long before the option table existed, and those
# cases are ids in saved results, so the tag cannot move. The reverse check
# below ("names an option but is not selectable by an option run") exempts
# exactly these, and nothing else, so a genuinely mis-tagged case still fails.
_LEGACY_OPTION_NAME_TAGS = frozenset({"equals"})

_SKIP: dict[str, set[str]] = {
    # A command-position TAB on an empty line lists thousands of matches; the
    # paging sequences are the interesting ones there, single-tab is not.
    "huge": {"tab_cr", "tab_down_cr"},
}


def _validate() -> None:
    """Corpus integrity, checked at import.

    Two cases with the same NAME collide in every by-name report; two with the
    same BUFFER collide in `gen_compsys_parity_report.py`'s buffer -> case map,
    so one of them is silently attributed to the other's tags and note. Both
    are silent mislabelling of results, which is exactly what an audit corpus
    cannot afford — fail loudly at import instead.
    """
    for what, seen in (("name", [c.name for c in CASES]),
                       ("buffer", [c.buffer for c in CASES])):
        dupes = sorted({v for v in seen if seen.count(v) > 1})
        if dupes:
            raise ValueError(f"parity_corpus: duplicate case {what}(s): {dupes}")
    for seq, keys in KEY_SEQUENCES.items():
        for k in keys:
            if k not in KEYS and len(k) != 1:
                raise ValueError(
                    f"parity_corpus: sequence {seq!r} names undefined key {k!r}")
    for what, names in (("DEFAULT_SEQUENCES", DEFAULT_SEQUENCES),
                        ("FUZZ_SEQUENCES", FUZZ_SEQUENCES)):
        unknown = [s for s in names if s not in KEY_SEQUENCES]
        if unknown:
            raise ValueError(f"parity_corpus: {what} has unknown {unknown}")
        dupes = sorted({s for s in names if names.count(s) > 1})
        if dupes:
            raise ValueError(f"parity_corpus: {what} lists {dupes} twice")

    # A case with no note or no tags is unselectable by `--tag` and unreadable
    # in a report; a name that is not a stable identifier cannot be a JSON
    # result id. Both are silent losses, so they fail here instead.
    for c in CASES:
        if not c.note:
            raise ValueError(f"parity_corpus: case {c.name!r} has no note")
        if not c.tags:
            raise ValueError(f"parity_corpus: case {c.name!r} has no tags")
        if not _CASE_NAME_RE.match(c.name):
            raise ValueError(f"parity_corpus: case name {c.name!r} is not a stable id")

    # An empty sequence compares two screens that were never keyed, i.e. a
    # guaranteed pass — an audit corpus must not contain one. Keys go through
    # `key_bytes` so the sequence check is exactly the rule the harness
    # transmits by, not a looser copy of it.
    for seq, keys in KEY_SEQUENCES.items():
        if not keys:
            raise ValueError(f"parity_corpus: sequence {seq!r} is empty")
        for k in keys:
            try:
                key_bytes(k)
            except UnknownKey:
                raise ValueError(
                    f"parity_corpus: sequence {seq!r} names undefined key {k!r}"
                ) from None

    # A `_SKIP` entry keyed on a tag no case carries, or naming a sequence that
    # no longer exists, silently stops skipping anything — or silently skips
    # nothing while looking like coverage policy. Both must fail loudly.
    all_tags = {t for c in CASES for t in c.tags}
    for tag, seqs in _SKIP.items():
        if tag not in all_tags:
            raise ValueError(f"parity_corpus: _SKIP keyed on unused tag {tag!r}")
        gone = sorted(s for s in seqs if s not in KEY_SEQUENCES)
        if gone:
            raise ValueError(f"parity_corpus: _SKIP[{tag!r}] names unknown {gone}")

    # ── shell-option tables ─────────────────────────────────────────────
    #
    # Same class of failure as the case tables above: an option name the shell
    # does not know, a `default` that does not match `zsh -f`, or a mask
    # naming an option that is not in the table all produce a run that reports
    # on a configuration it never actually installed. `default` itself needs a
    # live shell, so it is checked by `check_option_defaults()` rather than
    # here — this covers everything that can be checked without booting one.
    for name, opt in SHELL_OPTIONS.items():
        if name != opt.name:
            raise ValueError(
                f"parity_corpus: option keyed {name!r} but named {opt.name!r}")
        if not _OPTION_NAME_RE.match(name):
            raise ValueError(
                f"parity_corpus: option {name!r} is not a `setopt` spelling")
        if not opt.note:
            raise ValueError(f"parity_corpus: option {name!r} has no note")
        if not _CITE_RE.match(opt.cite):
            raise ValueError(
                f"parity_corpus: option {name!r} cite {opt.cite!r} is not a "
                "zsh doc reference")
        if opt.group not in OPTION_GROUPS:
            raise ValueError(
                f"parity_corpus: option {name!r} has unknown group {opt.group!r}")
    for name in OPTION_INTERACTIVE:
        if name not in SHELL_OPTIONS:
            raise ValueError(
                f"parity_corpus: OPTION_INTERACTIVE names unknown {name!r}")
    for m in OPTION_MASKS:
        for field_name in (m.masking, m.masked):
            if field_name not in SHELL_OPTIONS:
                raise ValueError(
                    f"parity_corpus: OPTION_MASKS names unknown {field_name!r}")
        if m.masking == m.masked:
            raise ValueError(f"parity_corpus: option {m.masked!r} masks itself")
        if not _CITE_RE.match(m.cite):
            raise ValueError(f"parity_corpus: mask cite {m.cite!r} is not a "
                             "zsh doc reference")
    for name, needed in OPTION_REQUIRES.items():
        if name not in SHELL_OPTIONS:
            raise ValueError(f"parity_corpus: OPTION_REQUIRES names unknown {name!r}")
        unknown = [n for n in needed if n not in SHELL_OPTIONS]
        if unknown or not needed:
            raise ValueError(
                f"parity_corpus: OPTION_REQUIRES[{name!r}] is {needed!r}")
    for p in OPTION_PAIRS:
        if p.a not in SHELL_OPTIONS or p.b not in SHELL_OPTIONS:
            raise ValueError(f"parity_corpus: OPTION_PAIRS names unknown "
                             f"{(p.a, p.b)!r}")
        if p.a == p.b:
            raise ValueError(f"parity_corpus: option {p.a!r} paired with itself")
        if not _CITE_RE.match(p.cite):
            raise ValueError(f"parity_corpus: pair cite {p.cite!r} is not a "
                             "zsh doc reference")
    for style, name, cite in OPTION_STYLE_MASKS:
        if name not in SHELL_OPTIONS:
            raise ValueError(
                f"parity_corpus: OPTION_STYLE_MASKS names unknown {name!r}")
        if not style or not _CITE_RE.match(cite):
            raise ValueError(
                f"parity_corpus: OPTION_STYLE_MASKS[{style!r}] cite {cite!r}")

    # A profile is a DELTA: an entry that restates the default silently gives
    # one configuration two ids. An INCOHERENT profile is worse — it claims to
    # test an option another member overrides, so the run reports coverage it
    # does not have.
    for pname, deltas in OPTION_PROFILES.items():
        if normalize_option_set(deltas) != dict(sorted(deltas.items())):
            raise ValueError(
                f"parity_corpus: profile {pname!r} restates a default or names "
                "an unknown option")
        masked = masked_options(deltas)
        if masked:
            raise ValueError(
                f"parity_corpus: profile {pname!r} is incoherent: {masked}")

    # Option-sensitive cases: the `optsens` tag is what a harness selects on,
    # and the option names in the tags are what it selects BY. A tag that is
    # neither a known option nor a known surface is a typo that silently
    # narrows a selection to nothing.
    for c in CASES:
        named = [t for t in c.tags if t in SHELL_OPTIONS]
        if OPTION_TAG in c.tags:
            if not named:
                raise ValueError(
                    f"parity_corpus: case {c.name!r} is tagged {OPTION_TAG!r} "
                    "but names no shell option")
            unknown = [t for t in c.tags
                       if t not in SHELL_OPTIONS and t not in _SURFACE_TAGS]
            if unknown:
                raise ValueError(
                    f"parity_corpus: case {c.name!r} has unknown tag(s) {unknown}")
        elif [t for t in named if t not in _LEGACY_OPTION_NAME_TAGS]:
            raise ValueError(
                f"parity_corpus: case {c.name!r} names option(s) {named} but is "
                f"not tagged {OPTION_TAG!r}, so no option run selects it")
    for cname, seqs in OPTION_CASE_SEQUENCES.items():
        case = next((c for c in CASES if c.name == cname), None)
        if case is None:
            raise ValueError(
                f"parity_corpus: OPTION_CASE_SEQUENCES names unknown case {cname!r}")
        if OPTION_TAG not in case.tags:
            raise ValueError(
                f"parity_corpus: OPTION_CASE_SEQUENCES[{cname!r}] is not an "
                f"{OPTION_TAG!r} case")
        gone = [s for s in seqs if s not in KEY_SEQUENCES]
        if gone or not seqs:
            raise ValueError(
                f"parity_corpus: OPTION_CASE_SEQUENCES[{cname!r}] names {gone or 'nothing'}")


    _validate_generators()


def _applicable(case: Case, seq: str) -> bool:
    return not any(seq in _SKIP.get(tag, ()) for tag in case.tags)


def cases_by_tag(tag: str | None) -> list[Case]:
    if not tag:
        return list(CASES)
    return [c for c in CASES if tag in c.tags]


def adhoc_case(buffer: str, prefix: str = "adhoc") -> Case:
    """A Case for a buffer that is not in CASES (`--case`, `--corpus FILE`).

    The name is derived from the buffer, so the same ad-hoc buffer gets the
    same stable id in every run and across machines — a JSON result stream is
    only diffable across commits if the ids do not move.
    """
    known = {c.buffer: c for c in CASES}
    if buffer in known:
        return known[buffer]
    digest = hashlib.sha1(buffer.encode()).hexdigest()[:8]
    return Case(f"{prefix}_{digest}", buffer, "ad-hoc", ("adhoc",))


def matrix(
    cases: list[Case] | None = None,
    sequences: list[str] | None = None,
) -> list[tuple[Case, str, list[str]]]:
    """Cross product of cases and key sequences: (case, sequence-name, keys)."""
    cases = cases if cases is not None else CASES
    sequences = sequences if sequences is not None else DEFAULT_SEQUENCES
    out = []
    for case in cases:
        for name in sequences:
            if not _applicable(case, name):
                continue
            out.append((case, name, KEY_SEQUENCES[name]))
    return out


# ── zstyle combo generation ──────────────────────────────────────────────────
#
# The parity bar is not "these N curated configs agree" — it is that ANY
# combination of the user's zstyles renders byte-identically. The named combos
# in scripts/parity_combos/ only label the interesting axes; coverage comes
# from fuzzing random SUBSETS of the live fixture, because every `zstyle` line
# is independent and any subset is therefore a valid config.


def read_statements(path: str) -> list[str]:
    """The `zstyle ...` statements in a fixture, one per line."""
    out = []
    with open(path) as f:
        for line in f:
            s = line.rstrip("\n")
            if s.strip() and not s.lstrip().startswith("#"):
                out.append(s)
    return out


def random_subset(statements: list[str], keep: float, rng) -> list[str]:
    """Keep each statement with probability `keep`. Order is preserved, which
    matters: zstyle resolution is most-specific-first, but two statements with
    the same specificity are resolved in DEFINITION order."""
    return [s for s in statements if rng.random() < keep]


def shrink(statements: list[str], still_fails, max_probes: int = 60) -> list[str]:
    """Delta-debug a failing statement set down to a minimal one.

    `still_fails(subset) -> bool` re-runs the case against `subset`. A random
    100-statement subset that diverges says nothing actionable; the 1-2
    statements that actually cause it do. Bounded by `max_probes` because each
    probe boots two shells.

    Classic ddmin shape: try ever-finer partitions, keep any chunk-removal that
    preserves the failure, stop when nothing can be removed.
    """
    current = list(statements)
    probes = 0
    n = 2
    while len(current) >= 2 and probes < max_probes:
        chunk = max(1, len(current) // n)
        removed_any = False
        i = 0
        while i < len(current) and probes < max_probes:
            candidate = current[:i] + current[i + chunk:]
            if not candidate:
                i += chunk
                continue
            probes += 1
            if still_fails(candidate):
                current = candidate
                removed_any = True
                n = max(n - 1, 2)
                continue          # same index, list got shorter
            i += chunk
        if not removed_any:
            if n >= len(current):
                break
            n = min(n * 2, len(current))
    return current



# ── shell options ────────────────────────────────────────────────────────────
#
# The zstyle machinery above fuzzes the completion SYSTEM's configuration. It
# says nothing about the shell's own options, and those change completion just
# as fundamentally: `menucomplete` decides whether an ambiguous TAB inserts or
# lists, `completeinword` decides where the cursor is when the completer starts,
# `caseglob` decides whether a wrong-case path component matches at all. A
# corpus that never varies them reports "completion agrees" for exactly one
# point in that space — the default one.
#
# Sources read for this section (line numbers in ~/forkedRepos/zsh, the tree
# this port treats as its spec):
#
#   Doc/Zsh/options.yo:213-418    the whole "Completion" subsection: every
#                                 option below whose group is menu/listing/word.
#   Doc/Zsh/options.yo:421-760    "Expansion and Globbing": the glob group.
#   Doc/Zsh/options.yo:861-864    BANG_HIST.
#   Doc/Zsh/options.yo:1170-1172  ALIASES.
#   Doc/Zsh/options.yo:1207-1231  CORRECT / CORRECT_ALL, incl. the SPROMPT
#                                 `[nyae]` prompt that makes them interactive.
#   Doc/Zsh/options.yo:1328-1341  PATH_DIRS.
#   Doc/Zsh/options.yo:1379-1384  RC_QUOTES.
#   Doc/Zsh/options.yo:1680-1688  C_BASES.
#   Doc/Zsh/options.yo:2129-2133  KSH_OPTION_PRINT.
#   Doc/Zsh/options.yo:2397-2400  SH_WORD_SPLIT.
#   Doc/Zsh/compsys.yo:1929-1934  the `list` style overrides AUTO_LIST.
#   Doc/Zsh/compsys.yo:1979-1984  the `list-packed` style overrides LIST_PACKED.
#   Doc/Zsh/compsys.yo:2016-2020  `list-rows-first` overrides LIST_ROWS_FIRST.
#   Doc/Zsh/compsys.yo:2138-2141  NO_CASE_GLOB makes file matching
#                                 case-insensitive only when no matcher is set.
#   Doc/Zsh/compsys.yo:2188-2205  the `menu` style overrides MENU_COMPLETE and
#                                 can stand in for AUTO_MENU.
#   Doc/Zsh/compsys.yo:2358-2365  accept-exact-dirs interacts with
#                                 COMPLETE_IN_WORD.
#
# `default` is the state under `zsh -f` (`emulate zsh`), which is how both
# harnesses boot their children — the `<D>`/`<Z>` markers in options.yo, and
# confirmed against the two shells with
#   zsh -f -c 'print -r -- ${options[completeinword]}'
# (`check_option_defaults()` below re-runs exactly that for every option, so
# the table is checkable rather than merely asserted).


@dataclass(frozen=True)
class Opt:
    """One shell option this corpus is allowed to vary."""

    name: str        # the spelling `setopt` takes (lowercase, no underscores)
    default: bool    # state under `zsh -f`
    group: str       # the completion surface it moves
    cite: str        # file:line in the zsh doc tree
    note: str        # what it changes, in one line


_OPTS: tuple[Opt, ...] = (
    # ── where the cursor is, and what counts as the word ──────────────
    Opt("completeinword", False, "word", "options.yo:310-312",
        "cursor stays put and completion runs from BOTH ends, instead of "
        "jumping to the end of the word first"),
    Opt("alwaystoend", False, "word", "options.yo:213-217",
        "after a full completion the cursor is moved to the end of the word "
        "— only observable when it started inside one"),
    # ── insert vs list vs menu ────────────────────────────────────────
    Opt("automenu", True, "menu", "options.yo:232-235",
        "the second consecutive completion request starts menu completion"),
    Opt("menucomplete", False, "menu", "options.yo:401-407",
        "an ambiguous completion inserts the first match immediately rather "
        "than listing or beeping"),
    Opt("recexact", False, "menu", "options.yo:414-417",
        "a word that exactly matches a completion is accepted even when "
        "another match extends it"),
    Opt("globcomplete", False, "menu", "options.yo:318-330",
        "a word containing a pattern generates matches and cycles them "
        "instead of inserting the whole expansion"),
    Opt("autolist", True, "listing", "options.yo:224-225",
        "an ambiguous completion lists the choices automatically"),
    Opt("bashautolist", False, "listing", "options.yo:287-294",
        "list only on the SECOND consecutive call; takes precedence over "
        "AUTO_LIST and does not work with MENU_COMPLETE"),
    Opt("listambiguous", True, "listing", "options.yo:347-353",
        "when there is an unambiguous prefix to insert, insert it and do NOT "
        "list; auto-listing happens only when nothing would be inserted"),
    Opt("listbeep", True, "listing", "options.yo:361-365",
        "an ambiguous completion returns status 1, which beeps if BEEP is "
        "also set"),
    Opt("listpacked", False, "listing", "options.yo:372-374",
        "columns get individual widths so the listing occupies fewer lines"),
    Opt("listrowsfirst", False, "listing", "options.yo:381-384",
        "matches are laid out horizontally: the second is to the RIGHT of "
        "the first, not under it"),
    Opt("listtypes", True, "listing", "options.yo:392-394",
        "file completions carry a trailing type mark in the listing"),
    # ── what gets appended / taken back ───────────────────────────────
    Opt("autoparamslash", True, "suffix", "options.yo:268-270",
        "completing a parameter whose value is a directory appends `/`, not "
        "a space"),
    Opt("autoparamkeys", True, "suffix", "options.yo:254-262",
        "an auto-inserted character is removed when the next character typed "
        "has to come directly after the parameter name"),
    Opt("autoremoveslash", True, "suffix", "options.yo:277-280",
        "a trailing slash left by a completion is removed when the next "
        "character typed is a word delimiter"),
    Opt("markdirs", False, "suffix", "options.yo:675-677",
        "directory names produced by globbing get a trailing `/`"),
    # ── globbing, which is what file completion matches with ──────────
    Opt("caseglob", True, "glob", "options.yo:460-466",
        "globbing is case-sensitive; unset, any glob-special character in "
        "the word makes matching case-insensitive"),
    Opt("globdots", False, "glob", "options.yo:568-569",
        "a leading `.` no longer has to be matched explicitly, so dotfiles "
        "are offered for a word that does not start with one"),
    Opt("extendedglob", False, "glob", "options.yo:521-524",
        "`#`, `~` and `^` become pattern characters in the typed word"),
    Opt("bareglobqual", True, "glob", "options.yo:439-442",
        "a trailing parenthesised group is a qualifier list rather than "
        "literal text"),
    Opt("numericglobsort", False, "glob", "options.yo:733-735",
        "numeric filenames sort numerically, which reorders the listing"),
    Opt("nomatch", True, "glob", "options.yo:711-716",
        "a pattern with no matches is an error instead of being left alone"),
    Opt("nullglob", False, "glob", "options.yo:723-726",
        "a pattern with no matches is deleted; overrides NOMATCH"),
    Opt("cshnullglob", False, "glob", "options.yo:501-506",
        "a non-matching pattern is deleted unless every pattern in the "
        "command failed; overrides NOMATCH"),
    Opt("globsubst", False, "glob", "options.yo:590-595",
        "characters produced by parameter expansion become eligible for "
        "filename generation"),
    # ── word-level expansions the completer has to see through ────────
    Opt("equals", True, "expand", "options.yo:512-514",
        "`=cmd` is filename-expanded to the command's path"),
    Opt("magicequalsubst", False, "expand", "options.yo:655-667",
        "an argument that merely LOOKS like an assignment gets filename "
        "expansion on its right-hand side; respects KSH_TYPESET"),
    Opt("kshtypeset", False, "expand", "options.yo:2140",
        "assignment-looking arguments to typeset and friends are not word "
        "split — the option MAGIC_EQUAL_SUBST defers to"),
    Opt("ignorebraces", False, "expand", "options.yo:614-616",
        "brace expansion is off, so a `{a,b}` word is literal text"),
    Opt("rcquotes", False, "expand", "options.yo:1379-1383",
        "`''` inside a single-quoted string is one literal quote"),
    Opt("shwordsplit", False, "expand", "options.yo:2397-2400",
        "unquoted parameter expansions are field-split"),
    Opt("banghist", True, "expand", "options.yo:861-863",
        "`!` starts csh-style history expansion instead of being literal"),
    # ── command position ──────────────────────────────────────────────
    Opt("aliases", True, "command", "options.yo:1170-1171",
        "aliases are expanded at all"),
    Opt("completealiases", False, "command", "options.yo:301-304",
        "an alias is NOT substituted before completion, so it completes as a "
        "command of its own"),
    Opt("pathdirs", False, "command", "options.yo:1328-1340",
        "command names containing a slash are still searched along $PATH"),
    Opt("hashlistall", True, "command", "options.yo:336-339",
        "the whole command path is hashed before a command completion or a "
        "spelling correction"),
    Opt("autocd", False, "command", "options.yo:65-72",
        "a bare directory name in command position is a `cd`"),
    Opt("autonamedirs", False, "command", "options.yo:242-248",
        "any parameter holding an absolute directory becomes a `~name`, so "
        "it shows up when completing a word starting with `~`"),
    Opt("correct", False, "command", "options.yo:1207-1217",
        "command words are spell-checked, with an interactive [nyae] prompt"),
    Opt("correctall", False, "command", "options.yo:1223-1230",
        "every argument is spell-checked as a filename, each prompting"),
    # ── output shapes a completer reads back ──────────────────────────
    Opt("kshoptionprint", False, "misc", "options.yo:2129-2132",
        "`setopt` prints every option marked on/off instead of two lists — "
        "which is the text option completion parses"),
    Opt("cbases", False, "misc", "options.yo:1680-1687",
        "hexadecimal arithmetic output is `0xFF`, not `16#FF`"),
)

SHELL_OPTIONS: dict[str, Opt] = {o.name: o for o in _OPTS}

# Group -> the options in it, sorted. A run that suspects the listing code can
# fuzz `OPTION_GROUPS["listing"]` alone instead of the whole space.
OPTION_GROUPS: dict[str, tuple[str, ...]] = {
    g: tuple(sorted(o.name for o in _OPTS if o.group == g))
    for g in sorted({o.group for o in _OPTS})
}

# Options that make the shell STOP AND ASK. CORRECT/CORRECT_ALL print the
# SPROMPT `[nyae]` prompt and wait for a keypress (options.yo:1213-1214), which
# a pty harness replaying a fixed key path cannot answer: the run does not fail,
# it hangs or captures a prompt instead of a completion. Excluded from
# generation unless a caller asks for them explicitly — this suppresses no
# comparison, it keeps the generator from emitting inputs that measure the
# harness's timeout rather than either shell's completion.
OPTION_INTERACTIVE = frozenset({"correct", "correctall"})


@dataclass(frozen=True)
class OptionMask:
    """`masking` in state `state` makes `masked` unobservable."""

    masking: str
    state: bool
    masked: str
    cite: str
    note: str


# The documented overrides. Generating `menucomplete` and `automenu` together
# is not a bug in the shell — it is a wasted cell: options.yo:407 says the
# second one has no effect, so the run learns nothing about AUTO_MENU while
# reporting that it tested it. `gen_option_set` drops the masked half.
OPTION_MASKS: tuple[OptionMask, ...] = (
    OptionMask("menucomplete", True, "automenu", "options.yo:407",
               "MENU_COMPLETE overrides AUTO_MENU"),
    OptionMask("menucomplete", True, "bashautolist", "options.yo:292-294",
               "BASH_AUTO_LIST does not work with MENU_COMPLETE, because "
               "repeated calls cycle the list immediately"),
    OptionMask("bashautolist", True, "autolist", "options.yo:290",
               "BASH_AUTO_LIST takes precedence over AUTO_LIST"),
    OptionMask("nullglob", True, "nomatch", "options.yo:726",
               "NULL_GLOB overrides NOMATCH"),
    OptionMask("cshnullglob", True, "nomatch", "options.yo:506",
               "CSH_NULL_GLOB overrides NOMATCH"),
)

# Options that only do anything while something else is ON. LIST_AMBIGUOUS is
# defined as a modifier of the auto-listing options (options.yo:348-349), so an
# otherwise-fine set that turns both of those off cannot observe it.
OPTION_REQUIRES: dict[str, tuple[str, ...]] = {
    "listambiguous": ("autolist", "bashautolist"),
}


@dataclass(frozen=True)
class OptionPair:
    """Two options whose behaviour is defined in terms of each other."""

    a: str
    b: str
    cite: str
    note: str


# Not masks — these are the combinations worth generating TOGETHER, because the
# documented behaviour of one is stated in terms of the other. Fuzzing them
# independently reaches the interesting quadrant only by accident.
OPTION_PAIRS: tuple[OptionPair, ...] = (
    OptionPair("globcomplete", "completeinword", "options.yo:322-323",
               "the implicit `*` is appended, or inserted AT THE CURSOR when "
               "COMPLETE_IN_WORD is set"),
    OptionPair("alwaystoend", "completeinword", "options.yo:213-217",
               "ALWAYS_TO_END only has a cursor to move when completion "
               "started inside a word"),
    OptionPair("magicequalsubst", "kshtypeset", "options.yo:665-667",
               "MAGIC_EQUAL_SUBST respects KSH_TYPESET: together, "
               "assignment-looking arguments are not word split"),
    OptionPair("correct", "hashlistall", "options.yo:1209-1211",
               "without HASH_LIST_ALL, CORRECT falsely reports spelling "
               "errors the first time a command is used"),
    OptionPair("caseglob", "bareglobqual", "options.yo:464-466",
               "the NO_CASE_GLOB example depends on the trailing `(/)` being "
               "read as a qualifier, i.e. on BARE_GLOB_QUAL"),
    OptionPair("nullglob", "cshnullglob", "options.yo:501-506, options.yo:723-726",
               "two different answers to the same question — a pattern that "
               "matched nothing"),
    OptionPair("listpacked", "listrowsfirst", "options.yo:372-384",
               "both rewrite the listing geometry, and the layout code has to "
               "compose them"),
    OptionPair("autoparamslash", "autoremoveslash", "options.yo:268-280",
               "one adds the trailing slash, the other takes it back"),
)

_OPTION_PARTNERS: dict[str, tuple[str, ...]] = {}
for _p in OPTION_PAIRS:
    for _x, _y in ((_p.a, _p.b), (_p.b, _p.a)):
        _OPTION_PARTNERS[_x] = _OPTION_PARTNERS.get(_x, ()) + (_y,)
del _p, _x, _y

# A zstyle that overrides an option outright. A combo run that fuzzes both axes
# at once can pull these out of the zstyle subset when it wants the OPTION
# measured; nothing here drops anything on its own.
OPTION_STYLE_MASKS: tuple[tuple[str, str, str], ...] = (
    ("list", "autolist", "compsys.yo:1929-1934"),
    ("list-packed", "listpacked", "compsys.yo:1979-1984"),
    ("list-rows-first", "listrowsfirst", "compsys.yo:2016-2020"),
    ("menu", "menucomplete", "compsys.yo:2198-2200"),
    ("menu", "automenu", "compsys.yo:2194-2196"),
    ("matcher-list", "caseglob", "compsys.yo:2138-2141"),
    ("matcher", "caseglob", "compsys.yo:2138-2141"),
)


def styles_masking(opts: dict) -> set[str]:
    """Style names that would override an option in `opts` if the same run also
    installed them. Advisory: the caller decides what to do about it."""
    return {style for style, opt, _ in OPTION_STYLE_MASKS if opt in opts}


# Coherent starting points, each one a delta from the `zsh -f` defaults. Named
# rather than generated because the interesting configurations are the ones a
# PERSON would write: the whole point of `menucomplete` is that someone turns
# it on and lives in it, and the bugs live in that lived-in state.
OPTION_PROFILES: dict[str, dict[str, bool]] = {
    # the baseline: what every existing cell in this corpus already runs under
    "zsh_default": {},
    # menu behaviour
    "menu_complete": {"menucomplete": True},
    "menu_off": {"automenu": False, "autolist": False},
    "bash_style": {"bashautolist": True, "automenu": False},
    "bash_style_menu": {"bashautolist": True},
    "rec_exact": {"recexact": True},
    "glob_complete": {"globcomplete": True, "completeinword": True},
    # listing geometry
    "packed_rows": {"listpacked": True, "listrowsfirst": True},
    "list_always": {"listambiguous": False},
    "quiet_list": {"listbeep": False, "listtypes": False},
    # cursor / word
    "in_word": {"completeinword": True},
    "in_word_to_end": {"completeinword": True, "alwaystoend": True},
    # suffixes
    "slash_off": {"autoparamslash": False, "autoremoveslash": False},
    "mark_dirs": {"markdirs": True},
    # globbing
    "case_insensitive": {"caseglob": False},
    "dotfiles": {"globdots": True},
    "extended_glob": {"extendedglob": True},
    "no_glob_qual": {"bareglobqual": False},
    "null_glob": {"nullglob": True},
    "csh_null_glob": {"cshnullglob": True},
    "no_match_off": {"nomatch": False},
    "numeric_sort": {"numericglobsort": True},
    # word expansions
    "expansion_off": {"equals": False, "banghist": False, "ignorebraces": True},
    "ksh_ish": {"shwordsplit": True, "kshtypeset": True,
                "kshoptionprint": True, "magicequalsubst": True},
    "rc_quotes": {"rcquotes": True},
    "glob_subst": {"globsubst": True},
    # command position
    "no_aliases": {"aliases": False},
    "complete_aliases": {"completealiases": True},
    "path_hunting": {"pathdirs": True, "autocd": True},
    "named_dirs": {"autonamedirs": True},
    "no_hash": {"hashlistall": False},
    "c_bases": {"cbases": True},
    # interactive — excluded from generation unless asked for by name
    "correcting": {"correct": True, "correctall": True},
}


def option_defaults() -> dict[str, bool]:
    """The `zsh -f` state of every option this module varies."""
    return {name: o.default for name, o in SHELL_OPTIONS.items()}


def normalize_option_set(opts: dict) -> dict[str, bool]:
    """Drop entries that restate the default, and reject unknown names.

    An option set is a DELTA from `zsh -f`, so `{"autolist": True}` and `{}`
    describe the same shell. Keeping both spellings would give one
    configuration two `option_set_id`s and split its results in a report.
    """
    out: dict[str, bool] = {}
    for name, value in opts.items():
        opt = SHELL_OPTIONS.get(name)
        if opt is None:
            raise ValueError(f"parity_corpus: unknown shell option {name!r}")
        if not isinstance(value, bool):
            raise ValueError(
                f"parity_corpus: option {name!r} wants a bool, got {value!r}")
        if value != opt.default:
            out[name] = value
    return dict(sorted(out.items()))


def effective_options(opts: dict) -> dict[str, bool]:
    """Defaults with the delta applied — the state the shell actually runs in."""
    eff = option_defaults()
    eff.update(normalize_option_set(opts))
    return eff


def masked_options(opts: dict) -> list[tuple[str, str]]:
    """`(masked, reason)` for every entry the set cannot observe.

    A diagnostic, not a filter: a caller that WANTS the masked combination
    (both shells still have to agree on it) passes `allow_masked=True` to the
    generator and ignores this.
    """
    opts = normalize_option_set(opts)
    eff = effective_options(opts)
    out: set[tuple[str, str]] = set()
    for m in OPTION_MASKS:
        if m.masked in opts and eff[m.masking] == m.state:
            out.add((m.masked, f"{m.masking}={'on' if m.state else 'off'}"))
    for name, needed in OPTION_REQUIRES.items():
        if name in opts and not any(eff[n] for n in needed):
            out.add((name, "needs " + " or ".join(needed)))
    return sorted(out)


def cohere_option_set(opts: dict) -> dict[str, bool]:
    """Drop every entry another member makes unobservable, until stable."""
    current = normalize_option_set(opts)
    while True:
        masked = masked_options(current)
        if not masked:
            return current
        current = {k: v for k, v in current.items()
                   if k not in {name for name, _ in masked}}


def option_statements(opts: dict) -> list[str]:
    """The shell lines that install an option set, sorted.

    Statements, not a blob, so an option set drops straight into the same
    `random_subset()` / `shrink()` machinery the zstyle combos use: a failing
    run carrying both axes can be delta-debugged down to the one `setopt` line
    and the one `zstyle` line that matter, together.
    """
    return [f"{'setopt' if value else 'unsetopt'} {name}"
            for name, value in normalize_option_set(opts).items()]


def parse_option_statements(lines) -> dict[str, bool]:
    """Inverse of `option_statements` — read a set back off a fixture file."""
    out: dict[str, bool] = {}
    for line in lines:
        text = line.split("#", 1)[0].strip()
        if not text:
            continue
        parts = text.split()
        if len(parts) != 2 or parts[0] not in ("setopt", "unsetopt"):
            raise ValueError(f"parity_corpus: not an option statement: {line!r}")
        out[parts[1]] = parts[0] == "setopt"
    return normalize_option_set(out)


def describe_option_set(opts: dict) -> str:
    """`+menucomplete -automenu` — short enough for a result row."""
    opts = normalize_option_set(opts)
    if not opts:
        return "zsh-default"
    return " ".join(f"{'+' if v else '-'}{n}" for n, v in opts.items())


def option_set_id(opts: dict) -> str:
    """A stable id for one option set, for JSON result keys and fixture names."""
    statements = option_statements(opts)
    if not statements:
        return "opt:default"
    digest = hashlib.sha1("\n".join(statements).encode()).hexdigest()[:10]
    return f"opt:{digest}"


def render_option_set(opts: dict, header: str | None = None) -> str:
    """The text of a sourceable fixture: a header comment plus the statements."""
    lines = [f"# {header}"] if header else []
    lines.append(f"# option set {option_set_id(opts)}: {describe_option_set(opts)}")
    lines.extend(option_statements(opts))
    return "\n".join(lines) + "\n"


def write_option_file(opts: dict, path: str, header: str | None = None) -> str:
    """Write a set where a harness's init file can `source` it. Returns `path`."""
    with open(path, "w") as f:
        f.write(render_option_set(opts, header))
    return path


def gen_option_set(rng, profile: str | None = None, max_extra: int = 3,
                   include_interactive: bool = False,
                   allow_masked: bool = False,
                   pair_prob: float = 0.5) -> dict[str, bool]:
    """A random, COHERENT option set: a delta from the `zsh -f` defaults.

    Coherent, not independent coin flips, in three senses:

      * it starts from an `OPTION_PROFILES` base, so the common axes arrive as
        a person would set them rather than as an unlikely scattering;
      * an option drawn on top of that pulls in its documented partner with
        probability `pair_prob` (`OPTION_PAIRS`), because the behaviour of one
        is DEFINED in terms of the other;
      * the result is put through `cohere_option_set()`, which drops any entry
        another member overrides (`OPTION_MASKS`, `OPTION_REQUIRES`) — a cell
        that sets `automenu` under `menucomplete` measures nothing while
        reporting that it measured AUTO_MENU.

    Pure in `rng`: every choice iterates a SORTED sequence, so `(seed, index)`
    reproduces a set on any machine and in any Python.
    """
    if max_extra < 0:
        raise ValueError("gen_option_set: max_extra must be >= 0")
    names = sorted(OPTION_PROFILES)
    if profile is None:
        if not include_interactive:
            names = [n for n in names
                     if not (set(OPTION_PROFILES[n]) & OPTION_INTERACTIVE)]
        profile = rng.choice(names)
    if profile not in OPTION_PROFILES:
        raise ValueError(f"gen_option_set: unknown profile {profile!r}")

    opts = dict(OPTION_PROFILES[profile])
    pool = [n for n in sorted(SHELL_OPTIONS)
            if n not in opts
            and (include_interactive or n not in OPTION_INTERACTIVE)]
    for name in rng.sample(pool, min(rng.randint(0, max_extra), len(pool))):
        opts[name] = not SHELL_OPTIONS[name].default
        for partner in _OPTION_PARTNERS.get(name, ()):
            if partner in opts:
                continue
            if not include_interactive and partner in OPTION_INTERACTIVE:
                continue
            if rng.random() < pair_prob:
                opts[partner] = not SHELL_OPTIONS[partner].default
    opts = normalize_option_set(opts)
    return opts if allow_masked else cohere_option_set(opts)


OPTION_MUTATIONS = ("add", "drop", "swap_in_group", "add_partner",
                    "merge_profile")


def _apply_option_mutation(opts: dict, op: str, rng,
                           include_interactive: bool) -> dict:
    """One named edit. Returns `opts` unchanged when the edit does not apply."""
    out = dict(opts)
    pool = [n for n in sorted(SHELL_OPTIONS)
            if n not in out
            and (include_interactive or n not in OPTION_INTERACTIVE)]
    if op == "add":
        if not pool:
            return out
        name = rng.choice(pool)
        out[name] = not SHELL_OPTIONS[name].default
        return out
    if op == "drop":
        if not out:
            return out
        del out[rng.choice(sorted(out))]
        return out
    if op == "swap_in_group":
        # Replace one member with another option from the SAME group: the
        # surface under test stays put while the knob moves, which is the edit
        # that separates "the listing code is wrong" from "LIST_PACKED is
        # wrong".
        if not out:
            return out
        name = rng.choice(sorted(out))
        siblings = [n for n in OPTION_GROUPS[SHELL_OPTIONS[name].group]
                    if n not in out
                    and (include_interactive or n not in OPTION_INTERACTIVE)]
        if not siblings:
            return out
        del out[name]
        pick = rng.choice(siblings)
        out[pick] = not SHELL_OPTIONS[pick].default
        return out
    if op == "add_partner":
        candidates = [(n, p) for n in sorted(out)
                      for p in _OPTION_PARTNERS.get(n, ())
                      if p not in out
                      and (include_interactive or p not in OPTION_INTERACTIVE)]
        if not candidates:
            return out
        _, partner = rng.choice(candidates)
        out[partner] = not SHELL_OPTIONS[partner].default
        return out
    if op == "merge_profile":
        names = sorted(OPTION_PROFILES)
        if not include_interactive:
            names = [n for n in names
                     if not (set(OPTION_PROFILES[n]) & OPTION_INTERACTIVE)]
        out.update(OPTION_PROFILES[rng.choice(names)])
        return out
    raise ValueError(f"_apply_option_mutation: unknown op {op!r}")


def mutate_option_set(opts: dict, rng, include_interactive: bool = False,
                      allow_masked: bool = False) -> dict[str, bool]:
    """One small structured edit to an option set — never the input back, and
    never empty.

    Same reasoning as `mutate_buffer`/`mutate_keys`: once a set diverges, its
    neighbours bound the bug, and the neighbour that differs by ONE option is
    the shrink step a report can name. An empty result would silently re-run
    the default configuration every other existing cell already covers, so the
    fallback adds an option instead of returning it.
    """
    base = normalize_option_set(opts)
    for _ in range(8):
        out = _apply_option_mutation(base, rng.choice(OPTION_MUTATIONS), rng,
                                     include_interactive)
        out = normalize_option_set(out)
        if not allow_masked:
            out = cohere_option_set(out)
        if out and out != base:
            return out
    pool = [n for n in sorted(SHELL_OPTIONS)
            if n not in base
            and (include_interactive or n not in OPTION_INTERACTIVE)]
    fallback = dict(base)
    if pool:
        name = rng.choice(pool)
        fallback[name] = not SHELL_OPTIONS[name].default
    elif not fallback:
        fallback["completeinword"] = True
    return normalize_option_set(fallback)


# Cases the harness should run an option set against, per option. Built from
# the case tags rather than a second hand-written table, so a case added below
# is selectable the moment it names the option it exercises.
OPTION_TAG = "optsens"


def cases_for_option(name: str) -> list[Case]:
    """Curated cases whose outcome the named option demonstrably changes."""
    if name not in SHELL_OPTIONS:
        raise ValueError(f"parity_corpus: unknown shell option {name!r}")
    return [c for c in CASES if OPTION_TAG in c.tags and name in c.tags]


def option_cases() -> list[Case]:
    """Every option-sensitive case."""
    return [c for c in CASES if OPTION_TAG in c.tags]


def options_exercised_by(case: Case) -> tuple[str, ...]:
    """The options a case was written to expose, from its tags."""
    return tuple(t for t in case.tags if t in SHELL_OPTIONS)


# The key paths that make an option-sensitive case actually say something.
# `optsens_ciw_midword` under `tab1` completes at the end of the buffer, where
# COMPLETE_IN_WORD has nothing to change; it needs a cursor that moved first.
# Advisory, and deliberately NOT merged into DEFAULT_SEQUENCES, which sizes
# every routine sweep.
OPTION_CASE_SEQUENCES: dict[str, tuple[str, ...]] = {
    "optsens_ciw_midword": ("left3_tab", "left_tab_right_tab", "left2_tab_tab"),
    "optsens_to_end": ("left3_tab", "left_tab_bs_tab"),
    "optsens_autolist": ("tab1", "tab2", "tab3"),
    "optsens_listambiguous": ("tab1", "tab2", "tab3"),
    "optsens_menu": ("tab1", "tab2", "tab_btab"),
    "optsens_listing": ("ctrl_d", "tab2"),
    "optsens_globcomplete": ("tab1", "tab2"),
    "optsens_removeslash": ("tab_space_tab", "tab_slash_tab"),
    "optsens_paramkeys": ("tab_char_tab",),
    "optsens_correct": ("tab_cr",),
}


def sequences_for_case(case: Case, fallback=None) -> list[str]:
    """The recommended key paths for a case, or `fallback` (DEFAULT_SEQUENCES)."""
    named = OPTION_CASE_SEQUENCES.get(case.name)
    if named:
        return list(named)
    return list(fallback if fallback is not None else DEFAULT_SEQUENCES)


# ── option defaults, checked against a real shell ────────────────────────────
#
# The `default` column above is the whole basis for `normalize_option_set`: get
# one wrong and a "delta" silently installs the default, so the run reports on a
# configuration it never set. `_validate()` cannot check it — it must not boot a
# shell at import — so the check is a function a run calls, and `main()` exposes
# it as `--check-option-defaults`.

_OPTION_PROBE = ("for o in {names}; do print -r -- \"$o=${{options[$o]}}\"; done")


def probe_option_defaults(argv=("zsh", "-f"), timeout: int = 20) -> dict[str, bool]:
    """Ask a real shell for the `-f` state of every option in the table."""
    script = _OPTION_PROBE.format(names=" ".join(sorted(SHELL_OPTIONS)))
    out = subprocess.run(list(argv) + ["-c", script],
                         capture_output=True, text=True, timeout=timeout).stdout
    state: dict[str, bool] = {}
    for line in out.splitlines():
        name, _, value = line.partition("=")
        if name in SHELL_OPTIONS and value in ("on", "off"):
            state[name] = value == "on"
    return state


def check_option_defaults(argv=("zsh", "-f")) -> list[tuple[str, bool, str]]:
    """`(option, table_default, shell_state)` for every disagreement.

    An option the shell does not know comes back as `"missing"`, which is just
    as wrong as a flipped default: the generator would emit a `setopt` line
    that shell ignores.
    """
    state = probe_option_defaults(argv)
    bad = []
    for name, opt in sorted(SHELL_OPTIONS.items()):
        if name not in state:
            bad.append((name, opt.default, "missing"))
        elif state[name] != opt.default:
            bad.append((name, opt.default, "on" if state[name] else "off"))
    return bad

# ── fuzz generators ──────────────────────────────────────────────────────────
#
# The tables above are the reproducible floor; fuzzing is how the space gets
# WIDE. Both harnesses want the same two random inputs (a key path and a
# command line) and both were growing private copies — `compsys_parity.py`
# already had one for key paths. Private generators mean an input that diverges
# under one harness cannot be replayed under the other, which is exactly what a
# shared corpus exists to prevent, so they live here.
#
# Every generator is a pure function of the `random.Random` handed in. Nothing
# reads the global RNG, the clock, or the environment, so `(seed, index)` is a
# complete description of an input and a failing cell replays anywhere.

# Key classes, by what the keystroke DOES once a completion is on screen.
GEN_KEY_CLASSES: dict[str, tuple[str, ...]] = {
    # another completion keystroke — cycle the menu forwards or backwards
    "complete": ("tab", "btab"),
    # move the selection without changing the line
    "nav": ("down", "up", "left", "right", "ctrl-n", "ctrl-p",
            "pgdn", "pgup", "home", "end"),
    # a printable character: self-inserts normally, FILTERS inside an
    # interactive menuselect — one keystroke, two entirely different keymaps
    "filter": tuple(MENUSELECT_FILTER_CHARS),
    # edit the line under the completion; completion state must be discarded
    "edit": ("bs", "ctrl-w", "ctrl-h", "delete", "space", "slash"),
    # leave the menu; the original line has to come back intact
    "abort": ("ctrl-g", "esc", "ctrl-_"),
    # list without inserting
    "list": ("ctrl-d",),
}

# Default mix. `complete`/`nav`/`filter` in these proportions is the shape the
# harness-private generator already fuzzed with, kept so switching to the
# shared one does not silently change what a given seed explores. `edit`,
# `abort` and `list` are available but off by default: they END the menu, so a
# long path spends most of its keys outside completion when they are mixed in.
GEN_KEY_WEIGHTS: dict[str, float] = {
    "complete": 0.35,
    "nav": 0.25,
    "filter": 0.40,
}


def gen_keyseq(rng, length: int, start: str | None = "tab",
               weights: dict[str, float] | None = None) -> list[str]:
    """A random key path `length` keys long, as key NAMES.

    Starts with `start` (default TAB) because a path that never completes says
    nothing about completion; pass `start=None` for a path that begins in the
    normal keymap (e.g. `ctrl-d` first, or cursor movement before the TAB).

    `weights` selects among `GEN_KEY_CLASSES` — a run that wants the abort and
    edit paths fuzzed passes them in. Classes are iterated in SORTED order so
    the same seed picks the same keys regardless of dict insertion order.

    Every name is put through `key_bytes()` before returning: a generator that
    can emit an undefined name would have the harness self-insert its letters
    and report the result as a completion divergence on both shells at once.
    """
    if length <= 0:
        return []
    weights = dict(weights) if weights else dict(GEN_KEY_WEIGHTS)
    unknown = sorted(set(weights) - set(GEN_KEY_CLASSES))
    if unknown:
        raise ValueError(f"gen_keyseq: unknown key class(es) {unknown}")
    classes = sorted(weights)
    total = sum(max(0.0, weights[c]) for c in classes)
    if total <= 0:
        raise ValueError("gen_keyseq: weights sum to zero")

    seq: list[str] = []
    if start is not None:
        seq.append(start)
    while len(seq) < length:
        r = rng.random() * total
        chosen = classes[-1]
        for c in classes:
            r -= max(0.0, weights[c])
            if r <= 0:
                chosen = c
                break
        seq.append(rng.choice(GEN_KEY_CLASSES[chosen]))
    for name in seq[:length]:
        key_bytes(name)
    return seq[:length]


# Buffer fragments, grouped by the same surfaces the CASES table is grouped by.
# They are deliberately PARTIAL (`/usr/l`, `${(`, `*(`) — a completion bug lives
# in what the completer does with an incomplete word, not a finished one.
_GEN_COMMANDS = ("ls", "cd", "cat", "cp", "grep", "git", "echo", "chmod",
                 "kill", "print", "zstyle", "man", "ssh", "find")
_GEN_PATHS = ("/", "/usr/", "/usr/bin/", "/usr/l", "/etc/", "/et", "~/", "~ro",
              "./", "../", "//usr//", "/usr/bin/z")
_GEN_PARAMS = ("$", "${", "$PA", "${(", "$path[", "$commands[", "$HOME/",
               "$((", "${HOME}/", "$HOM")
_GEN_GLOBS = ("*", "*(", "*(.", "*(/", "**/", "z*", "{a,", "{b")
_GEN_OPTS = ("-", "--", "-l -", "--col")
_GEN_REDIRS = ("> ", ">> ", "< ", "2> ")
_GEN_PREFIXES = ("", "", "", "sudo ", "command ", "noglob ", "env ",
                 "FOO=bar ", "nocorrect ")
_GEN_SUBCMDS = ("git ", "git log ", "git checkout ", "git log --",
                "zstyle ", "kill -")
_GEN_ASSIGN_NAMES = ("FOO", "PATH", "fpath", "MANPATH")
_GEN_QUOTE_CHARS = ('"', "'")

# The surface a generated buffer belongs to. Named so a run can narrow the fuzz
# to one surface (`gen_buffer(rng, classes=("param",))`) once a bug is smelled
# there, without hand-writing a corpus file.
GEN_BUFFER_CLASSES = ("cmd", "path", "quoted", "param", "glob", "opt",
                      "redir", "sub", "cmdsubst", "assign")


def gen_buffer(rng, classes: tuple[str, ...] | None = None) -> str:
    """A random command line to complete on, drawn from one surface class.

    The classes mirror the groupings in CASES, so a generated buffer is always
    a plausible neighbour of a curated one rather than random noise: fuzzing
    finds bugs near the shapes that already have them, and a buffer nothing can
    complete only costs two pty round-trips to learn nothing.

    Command words are truncated at a random point, which is what makes the
    space large: `g`, `gi`, `gre` and `grep` complete against different-sized
    match sets and fail independently.
    """
    classes = tuple(classes) if classes else GEN_BUFFER_CLASSES
    unknown = sorted(set(classes) - set(GEN_BUFFER_CLASSES))
    if unknown:
        raise ValueError(f"gen_buffer: unknown buffer class(es) {unknown}")
    kind = rng.choice(classes)
    pre = rng.choice(_GEN_PREFIXES)
    cmd = rng.choice(_GEN_COMMANDS)

    if kind == "cmd":
        return pre + cmd[:rng.randint(0, len(cmd))]
    if kind == "path":
        return f"{pre}{cmd} {rng.choice(_GEN_PATHS)}"
    if kind == "quoted":
        return (f"{pre}{cmd} {rng.choice(_GEN_QUOTE_CHARS)}"
                f"{rng.choice(_GEN_PATHS + _GEN_PARAMS)}")
    if kind == "param":
        return f"{pre}echo {rng.choice(_GEN_PARAMS)}"
    if kind == "glob":
        return f"{pre}{cmd} {rng.choice(_GEN_GLOBS)}"
    if kind == "opt":
        return f"{pre}{cmd} {rng.choice(_GEN_OPTS)}"
    if kind == "redir":
        return f"{cmd} x {rng.choice(_GEN_REDIRS)}{rng.choice(_GEN_PATHS)}"
    if kind == "sub":
        sub = rng.choice(_GEN_SUBCMDS)
        word = rng.choice(("", "", "che", "--", "-"))
        return pre + sub + word
    if kind == "cmdsubst":
        opener = rng.choice(("$(", "`", "<(", ">("))
        inner = rng.choice(("", cmd[:rng.randint(0, len(cmd))], f"{cmd} /us"))
        return f"echo {opener}{inner}"
    # assign
    name = rng.choice(_GEN_ASSIGN_NAMES)
    if rng.random() < 0.25:
        return f"{name}=({rng.choice(_GEN_PATHS)}"
    return f"{name}={rng.choice(_GEN_PATHS)}"


# One structured edit per name. Mutation fuzzing beats pure generation once a
# divergence exists: the neighbours of a failing input are where the boundary
# of the bug is, and a neighbour is far more likely to fail than a fresh random
# buffer is.
BUFFER_MUTATIONS = ("truncate", "extend", "space", "quote", "prefix",
                    "glob", "drop")


def _apply_buffer_mutation(buffer: str, op: str, rng) -> str:
    """One named edit. Returns `buffer` unchanged when the edit does not apply
    (truncating an empty buffer, dropping a word that is not there)."""
    head, sep, word = buffer.rpartition(" ")
    if op == "truncate":
        return buffer[:-rng.randint(1, 3)] if buffer else buffer
    if op == "extend":
        return buffer + rng.choice("abcdefghijklmnopqrstuvwxyz")
    if op == "space":
        return buffer[:-1] if buffer.endswith(" ") else buffer + " "
    if op == "quote":
        if not word:
            return buffer
        return f"{head}{sep}{rng.choice(_GEN_QUOTE_CHARS)}{word}"
    if op == "prefix":
        pre = rng.choice([p for p in _GEN_PREFIXES if p])
        return pre + buffer
    if op == "glob":
        return buffer + rng.choice(("*", "(", "[", "?"))
    if op == "drop":
        return head + sep if sep else buffer
    raise ValueError(f"_apply_buffer_mutation: unknown op {op!r}")


def mutate_buffer(buffer: str, rng) -> str:
    """One small structured edit to a command line — never the input back.

    Returning the input would spend two pty boots re-running a cell that was
    already run, and (worse) would report as a fresh confirmation of whatever
    the original said. Ops that do not apply are retried; the fallback appends
    a letter, which always changes the buffer.
    """
    for _ in range(8):
        out = _apply_buffer_mutation(buffer, rng.choice(BUFFER_MUTATIONS), rng)
        if out != buffer:
            return out
    return buffer + rng.choice("abcdefghijklmnopqrstuvwxyz")


# Keys that reach the same WIDGET by a different binding, or the same axis in
# the other direction. Swapping within a pair is the mutation that finds
# bindings implemented on only one of the two paths — `tab_down` passing while
# `tab_ctrl_n` diverges is a real shape (they are different keymap entries for
# the same menu move).
RELATED_KEYS: dict[str, tuple[str, ...]] = {
    "tab": ("btab",),
    "btab": ("tab",),
    "down": ("up", "ctrl-n"),
    "up": ("down", "ctrl-p"),
    "left": ("right", "ctrl-b"),
    "right": ("left", "ctrl-f"),
    "ctrl-n": ("down", "ctrl-p"),
    "ctrl-p": ("up", "ctrl-n"),
    "ctrl-f": ("right", "ctrl-b"),
    "ctrl-b": ("left", "ctrl-f"),
    "pgdn": ("pgup", "down"),
    "pgup": ("pgdn", "up"),
    "home": ("end",),
    "end": ("home",),
    "bs": ("ctrl-h", "delete"),
    "ctrl-h": ("bs",),
    "delete": ("bs",),
    "esc": ("ctrl-g",),
    "ctrl-g": ("esc",),
    "ctrl-c": ("ctrl-g",),
    "ctrl-w": ("ctrl-u",),
    "ctrl-u": ("ctrl-w",),
    "cr": ("ctrl-o",),
    "ctrl-o": ("cr",),
    "ctrl-r": ("ctrl-s",),
    "ctrl-s": ("ctrl-r",),
}

KEY_MUTATIONS = ("swap", "insert_filter", "duplicate", "drop", "append")

# Keys worth appending: each ENDS the interaction in a different way, which is
# where the teardown paths (menu removal, line restore, listing erase) live.
_KEY_TAILS = ("tab", "btab", "ctrl-d", "ctrl-g", "esc", "bs", "ctrl-_", "cr")


def mutate_keys(keys: list[str], rng) -> list[str]:
    """One small structured edit to a key path — never the input back.

    Same reasoning as `mutate_buffer`: neighbours of a failing path bound the
    bug. The result is re-checked with `key_bytes()`, because a mutation that
    could produce an undefined name would be transmitted as literal letters.
    """
    for _ in range(8):
        out = list(keys)
        op = rng.choice(KEY_MUTATIONS)
        if op == "swap" and out:
            idx = [i for i, k in enumerate(out) if k in RELATED_KEYS]
            if idx:
                i = rng.choice(idx)
                out[i] = rng.choice(RELATED_KEYS[out[i]])
        elif op == "insert_filter":
            out.insert(rng.randint(1, len(out)) if out else 0,
                       rng.choice(MENUSELECT_FILTER_CHARS))
        elif op == "duplicate" and out:
            i = rng.randrange(len(out))
            out.insert(i, out[i])
        elif op == "drop" and len(out) > 1:
            del out[rng.randrange(len(out))]
        elif op == "append":
            out.append(rng.choice(_KEY_TAILS))
        if out and out != keys:
            for name in out:
                key_bytes(name)
            return out
    return list(keys) + ["tab"]


# ── divergence fingerprinting ────────────────────────────────────────────────
#
# A fuzz run reports the same underlying bug from many cells: the pid in a
# prompt, the temp dir in a path and the match COUNT in a listing all differ
# per cell while the bug is one bug. Masking those before hashing collapses
# them to one id, which is what makes a sweep's output triageable — and it is a
# grouping key only: nothing here decides whether two screens MATCH, so it
# cannot make a run greener.
FINGERPRINT_NONE = "fp:none"          # the two screens do not differ at all
FINGERPRINT_VOLATILE = "fp:volatile"  # they differ ONLY in masked-out text

_VOLATILE_PATTERNS = (
    (re.compile(r"0x[0-9a-fA-F]+"), "0xH"),      # addresses
    (re.compile(r"\b[0-9a-f]{7,}\b"), "HEX"),    # git shas, temp-name digests
    (re.compile(r"/[^\s'\"|,;:()\[\]]+"), "/P"), # absolute paths, incl. pids
    (re.compile(r"\d+"), "#"),                   # pids, counts, sizes, times
)


def mask_volatile(text: str) -> str:
    """Replace the per-run text in one screen row.

    Order matters: paths are masked BEFORE bare digits, so `/tmp/x-1234/f`
    becomes one `/P` token rather than `/tmp/x-#/f` — otherwise two runs in two
    temp dirs still fingerprint differently.
    """
    for pattern, replacement in _VOLATILE_PATTERNS:
        text = pattern.sub(replacement, text)
    return text.rstrip()


def fingerprint(rows_a: list[str], rows_b: list[str]) -> str:
    """A stable id for ONE divergence between two screens.

    Built from the SET of masked (reference, test) row pairs that differ, not
    from row indices: the same bug lands on a different screen row when the
    prompt or the listing above it is a different height, and an id that moved
    with the row would report one bug as several.

    Returns `FINGERPRINT_NONE` when the screens are identical and
    `FINGERPRINT_VOLATILE` when every differing row masks to the same text —
    the caller decided there was a divergence, this only says the id carries no
    information beyond "something volatile moved".
    """
    n = max(len(rows_a), len(rows_b))
    pairs = set()
    differed = False
    for i in range(n):
        a = rows_a[i] if i < len(rows_a) else ""
        b = rows_b[i] if i < len(rows_b) else ""
        if a == b:
            continue
        differed = True
        ma, mb = mask_volatile(a), mask_volatile(b)
        if ma != mb:
            pairs.add((ma, mb))
    if not differed:
        return FINGERPRINT_NONE
    if not pairs:
        return FINGERPRINT_VOLATILE
    digest = hashlib.sha1(repr(sorted(pairs)).encode()).hexdigest()[:12]
    return f"fp:{digest}"


def _validate_generators(samples: int = 64) -> None:
    """Generator invariants, checked at import alongside the tables.

    The generators feed a fuzzer whose whole value is that a failing cell can
    be replayed from `(seed, index)`. A generator that reads a global RNG, or
    that can emit a key name the harness would type as letters, breaks that
    silently — the run still produces numbers, they are just not reproducible
    and not about completion. So the properties are asserted, not assumed.
    """
    for i in range(samples):
        rng = random.Random(f"parity_corpus-selfcheck:{i}")
        twin = random.Random(f"parity_corpus-selfcheck:{i}")

        keys = gen_keyseq(rng, 6)
        if keys != gen_keyseq(twin, 6):
            raise ValueError("parity_corpus: gen_keyseq is not seed-reproducible")
        if len(keys) != 6 or keys[0] != "tab":
            raise ValueError(f"parity_corpus: gen_keyseq shape wrong: {keys}")

        buf = gen_buffer(rng)
        if buf != gen_buffer(twin):
            raise ValueError("parity_corpus: gen_buffer is not seed-reproducible")

        mutated = mutate_buffer(buf, rng)
        if mutated != mutate_buffer(buf, twin):
            raise ValueError("parity_corpus: mutate_buffer is not seed-reproducible")
        if mutated == buf:
            raise ValueError(f"parity_corpus: mutate_buffer returned its input {buf!r}")

        mkeys = mutate_keys(keys, rng)
        if mkeys != mutate_keys(keys, twin):
            raise ValueError("parity_corpus: mutate_keys is not seed-reproducible")
        if not mkeys or mkeys == keys:
            raise ValueError(f"parity_corpus: mutate_keys returned its input {keys}")
        for name in mkeys:
            key_bytes(name)

        # Option sets: same reproducibility contract as the buffers and key
        # paths above, plus the two invariants the option axis adds — a
        # generated set is COHERENT (nothing in it is overridden by something
        # else in it, so the cell measures what it says it measures) and it
        # round-trips through the shell statements a harness sources.
        opts = gen_option_set(rng)
        if opts != gen_option_set(twin):
            raise ValueError("parity_corpus: gen_option_set is not seed-reproducible")
        if masked_options(opts):
            raise ValueError(
                f"parity_corpus: gen_option_set emitted a masked set {opts}")
        stray = sorted(set(opts) & OPTION_INTERACTIVE)
        if stray:
            raise ValueError(
                f"parity_corpus: gen_option_set emitted interactive {stray} "
                "without being asked for it")
        if parse_option_statements(option_statements(opts)) != opts:
            raise ValueError(
                f"parity_corpus: option statements do not round-trip: {opts}")
        if normalize_option_set(opts) != opts:
            raise ValueError(
                f"parity_corpus: gen_option_set emitted a non-normal set {opts}")

        mopts = mutate_option_set(opts, rng)
        if mopts != mutate_option_set(opts, twin):
            raise ValueError("parity_corpus: mutate_option_set is not seed-reproducible")
        if not mopts or mopts == opts:
            raise ValueError(
                f"parity_corpus: mutate_option_set returned its input {opts}")
        if masked_options(mopts):
            raise ValueError(
                f"parity_corpus: mutate_option_set emitted a masked set {mopts}")
        if sorted(set(mopts) & OPTION_INTERACTIVE):
            raise ValueError(
                f"parity_corpus: mutate_option_set emitted interactive options {mopts}")
        if option_set_id(mopts) == option_set_id(opts):
            raise ValueError(
                f"parity_corpus: two different option sets share an id {opts} {mopts}")


    # The empty delta is a legal configuration — it is the `zsh -f` baseline
    # every other cell in this corpus already runs under — and it must have its
    # own stable id rather than colliding with a real set.
    if option_set_id({}) != "opt:default" or option_statements({}):
        raise ValueError("parity_corpus: the empty option set is not the baseline")
    if cohere_option_set({"automenu": False, "menucomplete": True}) != {"menucomplete": True}:
        raise ValueError("parity_corpus: cohere_option_set did not drop a masked option")
    if cohere_option_set({"listambiguous": False, "autolist": False}) != {"autolist": False}:
        raise ValueError("parity_corpus: cohere_option_set ignored OPTION_REQUIRES")

    # Fingerprints: identical screens have no id, a purely volatile difference
    # is labelled as such, the SAME divergence under two different match counts
    # collapses to one id, and two DIFFERENT divergences must not.
    if fingerprint(["x"], ["x"]) != FINGERPRINT_NONE:
        raise ValueError("parity_corpus: fingerprint invented a divergence")
    if fingerprint(["pid 1234"], ["pid 5678"]) != FINGERPRINT_VOLATILE:
        raise ValueError("parity_corpus: fingerprint did not mask volatile text")
    if (fingerprint(["3 matches: foo"], ["3 matches: bar"])
            != fingerprint(["17 matches: foo"], ["17 matches: bar"])):
        raise ValueError("parity_corpus: fingerprint is not stable across counts")
    if (fingerprint(["alpha"], ["beta"])
            == fingerprint(["gamma"], ["delta"])):
        raise ValueError("parity_corpus: fingerprint collides distinct divergences")


# Tables and generators are both in scope by here, so the import-time check can
# cover both. (The call used to sit directly under `_validate`; it moved down,
# it did not go away.)
_validate()


# ── host discovery ───────────────────────────────────────────────────────────
#
# The hand-written CASES above are a fixed floor that any machine reproduces.
# They are not the coverage ceiling: this host carries 44k `_name` completers
# across 50 fpath directories, ~4.1k of which name a binary that is actually
# installed. Every one of those is a completer neither shell has ever been
# compared on. `discover_cases()` turns them into cases mechanically.
#
# Discovery is OPT-IN per run (`--discover N` on the harness) because the set
# depends on what is installed, so two machines produce different corpora and
# the results are not directly comparable. Order is sorted, so a given N always
# selects the same prefix of the same list on the same host.

# Commands whose COMPLETER may run the command itself. `_pick_variant` and
# `_call_program` execute `$cmd --version` / `$cmd --help` to decide which
# variant is installed, so discovering a case for one of these means the
# harness may actually run it — twice per cell, once per shell. This list is
# about not rebooting the machine mid-sweep; it suppresses no comparison and no
# divergence. `--discover-all` includes them anyway.
DISCOVER_UNSAFE = frozenset("""
    dd fdisk gdisk halt init mkfs mkswap newfs nvram parted poweroff pkill
    reboot rm shutdown sudo su swapoff swapon sysctl systemctl telinit umount
    mount kill killall diskutil launchctl pmset scutil softwareupdate
    csrutil erase_all_content_and_settings
""".split())


def _fpath_dirs() -> list[str]:
    """The fpath a bare `zsh -f` sees on this host."""
    try:
        out = subprocess.run(
            ["zsh", "-f", "-c", "print -rl -- $fpath"],
            capture_output=True, text=True, timeout=15,
        ).stdout
    except Exception:
        return []
    return [d for d in out.splitlines() if d and os.path.isdir(d)]


def completer_names(dirs: list[str] | None = None) -> list[str]:
    """Sorted command names that have a `_name` completer somewhere in fpath."""
    names: set[str] = set()
    for d in (dirs if dirs is not None else _fpath_dirs()):
        try:
            entries = os.listdir(d)
        except OSError:
            continue
        for e in entries:
            # `_name`, but not `__helper`, not `_name.zwc`, not a bare `_`.
            if (len(e) > 1 and e[0] == "_" and e[1] != "_"
                    and "." not in e and "/" not in e):
                names.add(e[1:])
    return sorted(names)


def discover_cases(limit: int | None = None, include_unsafe: bool = False,
                   dirs: list[str] | None = None) -> list[Case]:
    """Cases for every installed command that ships a completer on this host.

    Two cases per command — `cmd ` (operand/subcommand position) and `cmd -`
    (option position) — because those are the two shapes almost every completer
    implements and they fail independently.
    """
    out: list[Case] = []
    taken = {c.buffer for c in CASES}
    for name in completer_names(dirs):
        if not include_unsafe and name in DISCOVER_UNSAFE:
            continue
        if not shutil.which(name):
            continue
        for suffix, kind, tag in ((" ", "arg", "sub"), (" -", "opt", "opt")):
            buf = name + suffix
            if buf in taken:
                continue
            taken.add(buf)
            out.append(Case(f"auto_{kind}_{name}", buf,
                            f"discovered completer _{name}",
                            ("auto", tag, "optional")))
        if limit is not None and len(out) >= limit:
            break
    return out[:limit] if limit is not None else out


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--list-keys", action="store_true")
    ap.add_argument("--list-cases", action="store_true")
    ap.add_argument("--list-sequences", action="store_true")
    ap.add_argument("--list-discovered", action="store_true",
                    help="cases this host would contribute via discover_cases()")
    ap.add_argument("--discover-limit", type=int, default=None)
    ap.add_argument("--discover-all", action="store_true",
                    help="include commands whose completer may execute them")
    ap.add_argument("--list-options", action="store_true",
                    help="the shell options this corpus varies, with their "
                         "`zsh -f` default and doc citation")
    ap.add_argument("--list-option-cases", action="store_true",
                    help="cases whose outcome a shell option changes")
    ap.add_argument("--gen-options", type=int, default=0, metavar="N",
                    help="print N seeded option sets")
    ap.add_argument("--option-profile", default=None,
                    help="force a profile for --gen-options")
    ap.add_argument("--check-option-defaults", nargs="*", default=None,
                    metavar="SHELL",
                    help="verify the option DEFAULT column against a real "
                         "shell (default: `zsh -f`); repeatable")
    ap.add_argument("--matrix-size", action="store_true")
    ap.add_argument("--all-sequences", action="store_true",
                    help="size the matrix against every sequence, not the default battery")
    ap.add_argument("--gen", type=int, default=0, metavar="N",
                    help="print N seeded (buffer, key-path) fuzz inputs")
    ap.add_argument("--gen-keys", type=int, default=5, metavar="N",
                    help="key-path length for --gen")
    ap.add_argument("--seed", type=int, default=0, help="seed for --gen")
    args = ap.parse_args()

    if args.list_keys:
        for name, seq in KEYS.items():
            print(f"{name:16s} {seq!r}")
    if args.list_sequences:
        for name, keys in KEY_SEQUENCES.items():
            mark = "*" if name in DEFAULT_SEQUENCES else (
                "+" if name in FUZZ_SEQUENCES else " ")
            print(f"{mark} {name:18s} {','.join(keys)}")
    if args.list_cases:
        for c in CASES:
            print(f"{c.name:20s} {c.buffer!r:28s} [{','.join(c.tags)}] {c.note}")
    if args.list_discovered:
        found = discover_cases(args.discover_limit, args.discover_all)
        for c in found:
            print(f"{c.name:32s} {c.buffer!r}")
        print(f"# {len(found)} discovered case(s) on this host")
    if args.list_options:
        for group in OPTION_GROUPS:
            print(f"# {group}")
            for name in OPTION_GROUPS[group]:
                o = SHELL_OPTIONS[name]
                flag = "on " if o.default else "off"
                mark = "!" if name in OPTION_INTERACTIVE else " "
                print(f"{mark} {name:18s} {flag}  {o.cite:24s} {o.note}")
        print(f"# {len(SHELL_OPTIONS)} option(s) in {len(OPTION_GROUPS)} group(s), "
              f"{len(OPTION_PROFILES)} profile(s), {len(OPTION_MASKS)} mask(s), "
              f"{len(OPTION_PAIRS)} pair(s), "
              f"{len(OPTION_INTERACTIVE)} interactive")
    if args.list_option_cases:
        for c in option_cases():
            seqs = OPTION_CASE_SEQUENCES.get(c.name, ())
            print(f"{c.name:24s} {c.buffer!r:20s} "
                  f"[{','.join(options_exercised_by(c))}]"
                  + (f" seqs={','.join(seqs)}" if seqs else ""))
        print(f"# {len(option_cases())} option-sensitive case(s), "
              f"{len({o for c in option_cases() for o in options_exercised_by(c)})}"
              f"/{len(SHELL_OPTIONS)} option(s) covered")
    if args.gen_options:
        rng = random.Random(args.seed)
        for _ in range(args.gen_options):
            opts = gen_option_set(rng, profile=args.option_profile)
            print(f"{option_set_id(opts):16s} {describe_option_set(opts)}")
    if args.check_option_defaults is not None:
        shells = args.check_option_defaults or ["zsh -f"]
        rc = 0
        for shell in shells:
            argv = shell.split()
            try:
                bad = check_option_defaults(argv)
            except Exception as exc:            # noqa: BLE001 — report, not raise
                print(f"{shell}: could not probe: {exc}")
                rc = 1
                continue
            for name, want, got in bad:
                print(f"{shell}: {name}: table says "
                      f"{'on' if want else 'off'}, shell says {got}")
            print(f"# {shell}: {len(SHELL_OPTIONS) - len(bad)}/"
                  f"{len(SHELL_OPTIONS)} option default(s) agree")
            rc = rc or (1 if bad else 0)
        if rc:
            return rc
    if args.matrix_size:
        seqs = list(KEY_SEQUENCES) if args.all_sequences else DEFAULT_SEQUENCES
        cells = matrix(CASES, seqs)
        print(f"cases={len(CASES)} sequences={len(seqs)} cells={len(cells)}")
    if args.gen:
        rng = random.Random(args.seed)
        for _ in range(args.gen):
            buf = gen_buffer(rng)
            keys = gen_keyseq(rng, args.gen_keys)
            print(f"{buf!r:34s} {','.join(keys)}")
    if not any((args.list_keys, args.list_sequences, args.list_cases,
                args.list_discovered, args.matrix_size, args.gen,
                args.list_options, args.list_option_cases, args.gen_options,
                args.check_option_defaults is not None)):
        seqs = DEFAULT_SEQUENCES
        print(f"cases={len(CASES)} keys={len(KEYS)} "
              f"sequences={len(KEY_SEQUENCES)} "
              f"default={len(seqs)} fuzz={len(FUZZ_SEQUENCES)} "
              f"cells={len(matrix(CASES, seqs))}")
        print(f"tags={','.join(sorted({t for c in CASES for t in c.tags}))}")
        covered = {o for c in option_cases() for o in options_exercised_by(c)}
        print(f"options={len(SHELL_OPTIONS)} groups={len(OPTION_GROUPS)} "
              f"profiles={len(OPTION_PROFILES)} masks={len(OPTION_MASKS)} "
              f"pairs={len(OPTION_PAIRS)} style-masks={len(OPTION_STYLE_MASKS)} "
              f"option-cases={len(option_cases())} "
              f"options-covered={len(covered)}/{len(SHELL_OPTIONS)}")
        print("generators: " + ", ".join(sorted(
            ("gen_keyseq", "gen_buffer", "gen_option_set", "mutate_buffer",
             "mutate_keys", "mutate_option_set", "fingerprint",
             "mask_volatile"))))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

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

Run this file directly to print the tables:

    scripts/parity_corpus.py --list-keys
    scripts/parity_corpus.py --list-cases
    scripts/parity_corpus.py --list-sequences
    scripts/parity_corpus.py --matrix-size
"""

from __future__ import annotations

import argparse
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
    "ctrl-p": b"\x10",          # up-line-or-history / menu up
    "ctrl-n": b"\x0e",          # down-line-or-history / menu down
    "ctrl-f": b"\x06",          # forward-char / menu right
    "ctrl-b": b"\x02",          # backward-char / menu left
    "ctrl-a": b"\x01",          # beginning-of-line
    "ctrl-e": b"\x05",          # end-of-line
    # accept / abort / edit
    "cr": b"\r",
    "esc": b"\x1b",
    "esc-esc": b"\x1b\x1b",
    "ctrl-c": b"\x03",
    "ctrl-g": b"\x07",          # send-break — abort the menu
    "ctrl-u": b"\x15",          # kill-whole-line
    "ctrl-w": b"\x17",          # backward-kill-word
    "ctrl-h": b"\x08",          # backward-delete-char
    "bs": b"\x7f",              # backspace
    "ctrl-_": b"\x1f",          # undo
    "ctrl-x-ctrl-x": b"\x18\x18",
    "ctrl-l": b"\x0c",          # clear-screen
    "space": b" ",
    "slash": b"/",
}

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
    "tab_btab": ["tab", "btab"],
    "tab2_btab": ["tab", "tab", "btab"],
    # list without inserting
    "ctrl_d": ["ctrl-d"],
    "tab_ctrl_d": ["tab", "ctrl-d"],
    # abort paths — the original line has to come back intact
    "tab_ctrl_g": ["tab", "ctrl-g"],
    "tab_esc": ["tab", "esc"],
    "tab2_ctrl_g": ["tab", "tab", "ctrl-g"],
    "tab_down_ctrl_g": ["tab", "down", "ctrl-g"],
    # accept paths
    "tab_cr": ["tab", "cr"],
    "tab_down_cr": ["tab", "down", "cr"],
    # edit-after-complete: the completion state has to be discarded cleanly
    "tab_bs": ["tab", "bs"],
    "tab_ctrl_w": ["tab", "ctrl-w"],
    "tab_undo": ["tab", "ctrl-_"],
    # continue typing after a completion
    "tab_slash_tab": ["tab", "slash", "tab"],
    "tab_space_tab": ["tab", "space", "tab"],

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
# abort, accept.
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
]

# Sequences that cannot say anything for a given case tag, so the matrix skips
# them instead of burning a pty round-trip on a guaranteed-identical screen.
_SKIP: dict[str, set[str]] = {
    # A command-position TAB on an empty line lists thousands of matches; the
    # paging sequences are the interesting ones there, single-tab is not.
    "huge": {"tab_cr", "tab_down_cr"},
}


def _applicable(case: Case, seq: str) -> bool:
    return not any(seq in _SKIP.get(tag, ()) for tag in case.tags)


def cases_by_tag(tag: str | None) -> list[Case]:
    if not tag:
        return list(CASES)
    return [c for c in CASES if tag in c.tags]


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


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--list-keys", action="store_true")
    ap.add_argument("--list-cases", action="store_true")
    ap.add_argument("--list-sequences", action="store_true")
    ap.add_argument("--matrix-size", action="store_true")
    ap.add_argument("--all-sequences", action="store_true",
                    help="size the matrix against every sequence, not the default battery")
    args = ap.parse_args()

    if args.list_keys:
        for name, seq in KEYS.items():
            print(f"{name:16s} {seq!r}")
    if args.list_sequences:
        for name, keys in KEY_SEQUENCES.items():
            mark = "*" if name in DEFAULT_SEQUENCES else " "
            print(f"{mark} {name:18s} {','.join(keys)}")
    if args.list_cases:
        for c in CASES:
            print(f"{c.name:20s} {c.buffer!r:28s} [{','.join(c.tags)}] {c.note}")
    if args.matrix_size:
        seqs = list(KEY_SEQUENCES) if args.all_sequences else DEFAULT_SEQUENCES
        cells = matrix(CASES, seqs)
        print(f"cases={len(CASES)} sequences={len(seqs)} cells={len(cells)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

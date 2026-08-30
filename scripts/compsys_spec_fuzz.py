#!/bin/sh
""""exec sh -c 'for c in "$SPEC_FUZZ_PYTHON" python3 python3.14 python3.13 python3.12 /opt/homebrew/bin/python3 /usr/local/bin/python3 /usr/bin/python3; do [ -n "$c" ] && command -v "$c" >/dev/null 2>&1 && "$c" -c "import sys, pyte; pyte.Screen(4,4); sys.exit(0 if sys.implementation.name[:1] == chr(99) else 1)" >/dev/null 2>&1 && exec "$c" "$@"; done; echo compsys_spec_fuzz: found no CPython with pyte installed - pip install pyte, or set SPEC_FUZZ_PYTHON >&2; exit 2' sh "$0" "$@" #"""
__doc__ = """compsys_spec_fuzz.py — GENERATIVE, HERMETIC compsys parity fuzzer.

Every other completion-parity harness in this tree draws its cases from
something this host happens to have: `comptab_parity.py` completes real
commands against the user's ~/.zpwr dump, `compsys_parity.py` replays a
curated corpus. Both measure "does zshrs complete *the completers installed
here* the same way zsh does". Neither can reach a corner of the compsys
ENGINE that no installed completer happens to use, and neither reproduces on
a different machine.

This harness inverts that. It SYNTHESISES a random completer definition,
drops it into a throwaway `$fpath`, and drives the identical init through a
pty on real `zsh -f -i` and on `zshrs --zsh -f -i`, comparing the rendered
screen byte for byte. Nothing but the zsh installation's own function
directory is on `$fpath`; `$HOME`, `$PWD` and the whole environment are
synthesised per case. The same `--seed` regenerates the same specs anywhere.

Generation covers three layers, and `--kind` selects between them.

The SPEC layer — `arguments`, `values`, `describe`, `alternative` — draws
`_arguments` / `_values` / `_describe` / `_alternative` specs from a seeded
grammar, aimed at the surface where this codebase's real completion bugs
have lived:

    quote-blind splitting of `(...)` action bodies    (compsys_action_word_split_quoting)
    `{-h,--help}` shared-description rows             (compsys_parity_harness_and_rust_completers)
    `*::rest` CAA_RARGS option handling               (compsys_caa_rargs_option_completion)
    `_describe -O` adding zero matches                (same)
    `_values -s ,` continuation                       (compsys_gsu_param_global_gap)

The BUILTIN layer — `compadd`, `compset`, `tags`, `nested` — drives what
those four shell functions sit ON: the C-ported builtins. This matters
because a builtin bug reaches a spec-level case only if some generated spec
happens to route through the broken flag, and several flags are not
reachable from a spec at all:

    `compadd`   the full flag surface, weighted towards the combinations
                already shipped wrong here — `-U` clearing CAF_MATCH (was
                CMF_HIDE), `-r`/`-R` and CMF_REMOVE, `-e` and CMF_ISPAR
                (was CAF_NOSORT), `-d` arrays of the wrong length, `-W`
                with `-f`, `-o` orderings, `-X` prompt escapes, and the
                `-O`/`-A`/`-D` out-parameter forms, whose returned arrays
                are echoed back through the listing so a divergence in the
                VALUE is visible and not only in the final match set.
    `compset`   every `-p -P -s -S -n -N -q` form, with `$PREFIX`,
                `$SUFFIX`, `$IPREFIX`, `$ISUFFIX`, `$QIPREFIX`, `$QISUFFIX`,
                `$CURRENT`, `$words` and the builtin's return status all
                added as matches, so the two shells are compared on the
                PARAMETERS as well as on the completion they produce.
    `tags`      `_tags` / `_requested` / `_wanted` / `_next_label` /
                `_setup` / `_message` / `_describe -x` as nested loops.
    `nested`    a completer whose `*::args:->rest` dispatch re-enters
                completion two levels deep, so state leaking across a
                rewritten `words`/`CURRENT` boundary is exercised.

The SUBSTRATE layer — `substrate` — is under BOTH of those. compsys is not an
API, it is a 179-file shell PROGRAM, and the two layers above test the API
while assuming the language it is written in works. That assumption failed
measurably: `argv+=(...)` appends to the positional parameters in zsh and
REPLACES them here, so `Completion/Base/Core/_description:83` — the line every
description in the system passes through — builds the wrong argument list, and
every completion in the tree renders `h:file` where zsh renders `file`. Running
zsh's own completion suite scored 32/229 against zsh's 222/229, and 75 of the
93 failures trace to that single line. No amount of `_arguments`-spec fuzzing
could have found it.

So `--kind substrate` generates completer bodies out of a catalogue of the
shell-language constructs the stock completion functions actually use, derived
by READING them (`--list-substrate` prints the catalogue with the
`Completion/...` file:line that justifies every entry) in seven categories:

    positional  `argv+=`, `argv[N]=`, `set --`, `shift [n] [name]`, `$#`,
                `$@` vs `$*`, `${(@)argv[4,-1]}`, `${argv[(ib:4:)-]}`,
                `zparseopts -D` rewriting the caller's own arguments
    array       `local -a`, `+=`, subscript and slice assignment, sparse,
                `${a[(r)pat]}` / `(i)` / `(I)` / `(R)`, `set -A $name`,
                `${(@P)name}`, the match/description pairing idioms
    assoc       `local -A`, `typeset -gA`, sorted `${(@ok)h}`, `${h[(K)pat]}`,
                `${(k)h[(R)val]}`, `$+h[k]`, keys holding colons and NULs
    flags       `(P) (e) (s:x:) (j:x:) (q) (qq) (qqqq) (q-) (q+) (Q) (f) (M)
                (o) (O) (u) (b) (z) (t) (ps.\0.)` and the nested combinations
    scope       `local` shadowing a special, `typeset -g`/`-gH`,
                `setopt localoptions`, `emulate -L`/`-LR`, `$funcstack`
                depth and the chained `${funcstack[2,-1][(I)_description]}`
    control     `always` blocks, `return` from a nested function, `$?`
                propagation, `&&`-chained return conventions, trap scope
    ctx         the completion specials themselves — `$words`, `$CURRENT`,
                `$PREFIX`, `$curcontext`, `$_comp_tags`, and the completer's
                own `$@` — which are the constructs that only mean anything
                once the completion system is the one running the code

Every fragment is used TWO ways from one source, so the two cannot drift: it
runs inside a real completion (its result added as a match, so a divergence is
visible on the screen) and it runs STANDALONE on `zsh -f -c` on both shells.
The standalone side is the deliverable, because it both reduces a finding to a
plain shell one-liner of the `argv+=` kind and ATTRIBUTES it:

    the two shells already differ standalone   -> CORE-LANGUAGE bug; the
                                                  printed `repro` line is the
                                                  whole reproducer
    they agree standalone and the completion
    case still diverged                        -> COMPLETION bug; the language
                                                  is fine, the fixture is the
                                                  reproducer

A fragment real zsh REJECTS is a GENERATOR bug, not a finding, exactly as with
the widget and hostile-text axes: every one is run on the reference shell
before a case is generated, in every usable locale, and has to produce its
probe line with nothing on stderr. Rejections are counted in their own category
and printed. `--check-substrate` runs that check alone.

Each case is judged over a key PATH, not one press: `--keys auto` (the
default) draws a sequence per case — menu start, cycling, reverse cycling,
a filter letter typed into an open menu, and the two abort routes — because
a listing that is right on the first TAB can still be wrong on the second.

And each case is judged through a WIDGET, drawn by `--widget auto` from 36
entry points. Every other completion harness in this tree reaches completion
one way: TAB, on whatever compinit left there. That is one of a family, and
the widget decides which dispatch inside the completion core runs —

    the eight builtin completion widgets compinit rebinds to `_main_complete`
    (Completion/compinit:539-543), plus `menu-select` (:544), plus
    `accept-and-menu-complete` and `expand-word`, which are bindable but are
    NOT legal `zle -C` bases (no ZLE_ISCOMP — Src/Zle/zle_thingy.c:612)

    the compsys widget files compinit installs from their own `#compdef -k` /
    `-K` headers: `_complete_help`, `_correct_word`, `_expand_word`,
    `_history-complete-older`, `_bash_complete-word`, `_bash_list-choices`

    `zle -C` user completion widgets over all nine legal bases, with the
    widget's function either the generated completer itself (the completion
    core calls it with nothing between it and the builtins) or `_generic`
    (back through `_main_complete`)

    `compdef -k` / `compdef -K`, and the `#compdef -k` / `#compdef -K` FILE
    HEADER forms, which make compinit itself parse the declaration

A declaration real zsh REJECTS is a generator bug, not a finding, so every
one of the 36 is run on the reference shell before a case is generated and
proved to have installed the binding it claims (`zle -C` prints nothing on
success, so each entry carries a `verify` command whose output must name the
widget). The same check runs on zshrs: an id zsh accepts and zshrs does not
is reported as a divergence, never used to prune. `--check-widgets` runs the
check alone; `--list-widgets` prints the table.

`--compstate-probe` echoes the widget-visible `$compstate` fields — insert,
list, list_max, last_prompt, to_end, old_list, exact, pattern_insert — into
the listing the way the `compset` kind echoes `$PREFIX`, so a divergence in
the STATE a widget set is visible even when the rendered list matches. It
found `old_list=shown` vs `old_list=yes` on a re-invoked `zle -C` widget, a
case that is otherwise pixel-identical.

And each case runs in a LOCALE, drawn by `--locale auto`. Every other
completion harness in this tree pins `LC_ALL=C`, which is the one setting in
which the entire multibyte pathway is switched OFF: under `C` every byte
above 0x7f is a separate unprintable character, so display-width math,
metafication, combining marks and invalid sequences are never exercised at
all. That is the opposite of the environment this shell is meant to replace,
and it is where this codebase's width bugs have actually lived —
`niceztrlen` counting bytes instead of calling `mb_niceformat` truncated
described-list tails; `WCWIDTH` returning 0 where C returns -1 made
`IS_COMBINING` wrongly true; metafied `.zwc` pool bytes decoded as valid but
wrong UTF-8.

    --locale auto     draw one per case from the locales this host actually
                      has AND the reference shell actually accepts
    --locale C,en_US.UTF-8,zh_CN.GB2312
    --check-locales   run only the availability probe and the generated-text
                      self-check, print both tables, and exit

BOTH shells always get the identical `LANG`/`LC_ALL`; it is recorded in the
fixture (`@locale`) and replayed from it. Availability is PROVED, never
assumed: each candidate is run on the reference shell and has to report the
multibyte behaviour its family claims (a UTF-8 locale must count `日本語` as
3 characters and 6 columns, a single-byte locale as 9 of each). A candidate
that is not installed, or that the reference shell does not honour, is named
and skipped — never silently treated as tested.

`--hostile` widens the generated description / display-string / match
alphabet to text chosen to break width and encoding math, in named
categories: `combining` (base + U+0301), `precomposed`/`decomposed` (the
same grapheme both ways), `wide` (East Asian double-width), `zwj` (emoji
joiner sequences), `bidi` (RLM/LRM), `replacement` (U+FFFD), `c0`/`c1`
(control bytes, including the 0x01/0x02 this shell uses as its own prompt
marks), `meta` (0x83/0x84 — the bytes zsh's metafication layer reserves) and
`invalid` (lone continuation bytes, overlong encodings, truncated
sequences). A generated string the REFERENCE shell cannot represent in the
chosen locale is a GENERATOR bug, not a finding: every entry is written into
a file, read back through the reference shell's own lexer and proved to
round-trip byte for byte before it may be generated. Rejections are counted
per category and printed. The same probe runs on zshrs, and that side is
never used to prune — an entry zsh represents and zshrs does not is a
divergence and is reported as one.

`--width-probe` echoes `${#text}` (characters) and `${(m)#text}` (display
COLUMNS) for every hostile string a case used, as matches. A width bug that
happens not to shift a column is still caught, because the two numbers are
compared directly.

`--cols auto` draws the terminal width per case from a pool chosen so a
double-width glyph straddles the last column and the wrap point, which is
where column math breaks if it is counting the wrong unit.

Verdicts — none of which is ever softened to make a run look greener:

    PASS        both shells rendered the identical screen
    PASS(err)   both shells rendered the identical screen AND that screen is
                a compsys ERROR — the generated spec was malformed. Counted
                and reported separately, because a run that is 100% PASS(err)
                has tested nothing.
    FAIL        the screens differ, one shell errored and the other did not,
                a shell crashed, or a shell never reached a prompt
    SKIP        the case cannot be compared at all, with a named reason. Two
                reasons exist. `unstable-reference`: real zsh disagreed with
                ITSELF on a re-run, so there is no reference to compare
                against — proven by a second reference capture, printed in
                full, never assumed. `locale-unavailable`: the case's locale
                is not installed or the reference shell does not honour it,
                so the two shells were never put in the same environment.
                Neither is ever a pass.

A stream-only difference (identical grid, different escape bytes) is always
reported and counted; `--strict-stream` additionally makes it fail. A
difference in the PRINTABLE BYTES either shell wrote, when the decoded grids
still match, is always a FAIL (`text-bytes-diff`) — pyte's decoder maps every
malformed sequence to one U+FFFD, so two different wrong byte strings can
render as the same cell, and a comparison that stopped at the grid would
call that a pass.

One reference is not enough — `--split-scan`.

Everything above compares zshrs against ONE zsh: `/opt/homebrew/bin/zsh`,
release 5.9.2. That is not the tree the ports are written from. Those are read
from `~/forkedRepos/zsh`, which is 5.9.999.3-test and carries four years of
post-5.9 development, and it is also the tree that supplies the ztst oracle.
Where the two disagree, a port written faithfully from the C FAILS against the
binary, and a port that passes the binary is unfaithful to the C — and no
harness in this tree can tell those two apart, so the disagreements get found
by accident, one at a time, after the fact:

    zparseopts long specs   88d51a2400. Ported to the dev semantics, which
                            BROKE zinit; reverted behind LONG_SPEC_NEEDS_GUARD.
    chained (R)/(I)         8a3ee5a802. Resolved towards 5.9.2 at wantvals_c1523.
    dotted parameter names  a9ba166216. Reported as a zshrs bug; the reference
                            was simply older than the feature.
    ZLE attribute reset     cc672f1c3b. Ported from the dev tree; turned a
                            checked-in guard red on a stream difference that
                            has nothing to do with what the guard guards.

Two of those were resolved towards 5.9.2 and two towards the dev tree, none by
policy. `--split-scan` turns that into an inventory: it runs each probe on BOTH
references and on zshrs and classifies the result three ways —

    the two references AGREE, zshrs differs   ZSHRS-BUG. What every other
                                              harness here measures. A split
                                              classification can never excuse
                                              one, because the two references
                                              agreeing is MEASURED, not assumed.
    the two references DISAGREE               VERSION SPLIT. zshrs cannot match
                                              both; which side it takes is a
                                              policy decision, and the finding
                                              is that a decision is REQUIRED.
                                              The side it takes is reported.
    all three agree                           AGREEMENT.

A probe neither reference can run is a generator bug in its own category, and
one both reject identically is counted apart as agreement-on-rejection, so a
scan cannot look busy on malformed probes. Generation is aimed at the four
surfaces the confirmed splits sit on, because a split is not spread evenly over
the language — it lives where upstream has been working: `paramname`,
`builtinopt`, `subscript`, `pattern`, plus `zle`, which is judged on the SGR
escape signature through the same pty driver because the fourth split is
invisible in the rendered grid.

Every split is also scanned against the corpus on this host — `~/.zinit`,
`~/.zpwr`, the 8k-file completion collection, the stock compsys functions, and
this repo's own checked-in guards — because a split no code exercises is an
inventory entry and one zinit relies on is urgent. A seeded probe's corpus
patterns were written for that split; a generated probe's are derived
mechanically and are printed with a `~`, and only curated hits can put a split
on the urgent list. The annotation never changes a classification.

The dev reference has to be BUILT: `~/forkedRepos/zsh` is a read-only source
tree with no `Src/zsh`. `--build-dev-zsh` copies it into
`target/spec-fuzz-devzsh` and configures it at a prefix INSIDE that copy, so
the built shell's compiled-in `module_path` points at its own modules and all
three shells can run the byte-identical probe with no per-shell preamble. A
candidate reporting the same `$ZSH_VERSION` as `--zsh` is refused: a second
reference of the same version is not a second reference.

Typical use:

    scripts/compsys_spec_fuzz.py --seed 1 --cases 8
    scripts/compsys_spec_fuzz.py --seed 1 --cases 200 --jobs 4 --json out.json
    scripts/compsys_spec_fuzz.py --seed 5 --cases 20 --kind compadd,compset
    scripts/compsys_spec_fuzz.py --seed 5 --cases 20 --keys tab,tab,down
    scripts/compsys_spec_fuzz.py --seed 6 --cases 20 --kind substrate
    scripts/compsys_spec_fuzz.py --check-substrate
    scripts/compsys_spec_fuzz.py --list-substrate
    scripts/compsys_spec_fuzz.py --check-widgets
    scripts/compsys_spec_fuzz.py --check-locales
    scripts/compsys_spec_fuzz.py --seed 7 --cases 12 --widget zle-C:list-choices
    scripts/compsys_spec_fuzz.py --seed 7 --cases 12 --compstate-probe always
    scripts/compsys_spec_fuzz.py --seed 9 --cases 12 --hostile --cols auto
    scripts/compsys_spec_fuzz.py --seed 9 --cases 12 --locale zh_CN.GB2312
    scripts/compsys_spec_fuzz.py --replay target/spec-fuzz-1/case0003.min.zsh
    scripts/compsys_spec_fuzz.py --spec '1:c:((a\\:"add files" b\\:"bench"))'
    scripts/compsys_spec_fuzz.py --split-scan --build-dev-zsh
    scripts/compsys_spec_fuzz.py --split-scan --zsh-dev /path/to/zsh/Src/zsh
    scripts/compsys_spec_fuzz.py --split-scan --split-cases 200 \\
                                 --split-families subscript,paramname
"""

import argparse
import difflib
import fcntl
import json
import os
import pty
import random
import re
import select
import shlex
import shutil
import signal
import string
import struct
import subprocess
import sys
import termios
import threading
import time

try:
    import pyte
except ImportError:
    sys.exit("compsys_spec_fuzz: pyte not installed (pip install pyte)")


REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SELF = os.path.join("scripts", "compsys_spec_fuzz.py")
SENTINEL = "@SFZ@"


# ── byte-transparent text plumbing ───────────────────────────────────────────
#
# Generated text has to be able to contain bytes that are not valid UTF-8 —
# that is the point of the `invalid` category, and it is a shape real zsh
# handles (it reads a script as bytes). Python strings cannot hold those bytes
# directly, so every hostile string is carried as `str` decoded with
# `surrogateescape`: a byte that is not valid UTF-8 becomes a lone surrogate
# U+DC80..U+DCFF and encodes back to the original byte. All of the generator's
# existing str machinery (quoting, splitting, formatting) then works unchanged;
# only the file writes and the terminal prints need to know.

def bdec(b):
    """bytes -> str, keeping invalid bytes recoverable."""
    return b.decode("utf-8", "surrogateescape")


def benc(s):
    """str -> bytes, undoing bdec exactly."""
    return s.encode("utf-8", "surrogateescape")


def _wopen(path):
    """open() for a file a shell will read as BYTES.

    Plain `open(path, "w")` raises on a lone surrogate; this writes the byte it
    stands for, which is what the generated string was in the first place.
    """
    return open(path, "w", encoding="utf-8", errors="surrogateescape",
                newline="\n")


_DISP_SAFE = re.compile(r"[\x00-\x08\x0b-\x1f\x7f\udc80-\udcff]")


def disp(s):
    """One generated string, safe to print on THIS terminal.

    A lone surrogate cannot be encoded to stdout and a raw control byte would
    move the harness's own cursor, so both are shown as `\\xNN` escapes. Only
    the REPORT is escaped — what the shells were handed is unchanged.
    """
    if not isinstance(s, str):
        return s

    def sub(m):
        ch = m.group()
        o = ord(ch)
        return "\\x%02x" % (o - 0xdc00) if 0xdc80 <= o <= 0xdcff else "\\x%02x" % o
    return _DISP_SAFE.sub(sub, s)

# Serialises pty.fork()+exec: the child is a fork of a threaded interpreter and
# must not touch a lock another thread held at fork time. Free at --jobs 1.
_FORK_LOCK = threading.Lock()


class _TolerantScreen(pyte.Screen):
    """pyte.Screen that survives a private-mode SGR (``CSI ? ... m``).

    pyte forwards ``private=True`` for any CSI containing ``?``, but
    ``select_graphic_rendition`` takes no such keyword and raises TypeError
    mid-feed, aborting a whole sweep. Swallow the flag, render normally.
    (Same fix as scripts/comptab_parity.py; the two harnesses are deliberately
    independent modules, so it is repeated rather than imported.)
    """

    def select_graphic_rendition(self, *attrs, **kwargs):
        kwargs.pop("private", None)
        return super().select_graphic_rendition(*attrs, **kwargs)


# ── keys ─────────────────────────────────────────────────────────────────────

KEYS = {
    "tab": b"\t",
    "btab": b"\x1b[Z",
    "down": b"\x1b[B",
    "up": b"\x1b[A",
    "right": b"\x1b[C",
    "left": b"\x1b[D",
    "ctrl-n": b"\x0e",
    "ctrl-p": b"\x10",
    "ctrl-d": b"\x04",
    "ctrl-c": b"\x03",
    "ctrl-g": b"\x07",
    "ctrl-u": b"\x15",
    "ctrl-a": b"\x01",
    "ctrl-e": b"\x05",
    "ctrl-w": b"\x17",
    "bs": b"\x7f",
    "esc": b"\x1b",
    "space": b" ",
    "enter": b"\r",
    # Multi-byte sequences the WIDGET axis fires. Named after the sequence, not
    # after a widget: which widget sits on one is decided per case, and the
    # generated init always binds it explicitly so both shells are driven
    # through the identical binding.
    "cx-w": b"\x18w",            # ^Xw — unbound after a stock compinit
    "cx-v": b"\x18v",            # ^Xv — likewise
    "cx-z": b"\x18z",            # ^Xz — likewise
    "cx-h": b"\x18h",            # ^Xh — _complete_help
    "cx-c": b"\x18c",            # ^Xc — _correct_word
    "cx-e": b"\x18e",            # ^Xe — _expand_word
    "cx-star": b"\x18*",         # ^X* — expand-word
    "cx-tilde": b"\x18~",        # ^X~ — _bash_list-choices
    "esc-slash": b"\x1b/",       # \e/ — _history-complete-older
    "esc-tilde": b"\x1b~",       # \e~ — _bash_complete-word
}

# Keys that only mean anything once a completion LIST is on screen. A case
# whose path contains one gets menu selection switched on in its init (see
# `menu_setup`), otherwise the key would just be `down-line-or-history` and
# the case would prove nothing about the menu engine.
NAV_KEYS = frozenset(("down", "up", "left", "right", "btab", "ctrl-n", "ctrl-p"))

# The key PATHS a generated case is judged over. One `tab` is the shallowest
# thing a completion harness can ask; everything below it exercises a state the
# engine only reaches after a first listing — a second `tab` (menu start /
# re-list), cycling, reverse cycling, typing a filter letter into an open menu,
# and the two abort routes (`ctrl-g` send-break, `ctrl-u` kill-whole-line),
# which have to leave the screen in the same state on both shells.
KEY_PATHS = [
    ["tab"],
    ["tab"],
    ["tab"],
    ["tab", "tab"],
    ["tab", "tab", "tab"],
    ["tab", "down"],
    ["tab", "tab", "down", "down"],
    ["tab", "ctrl-n", "ctrl-n"],
    ["tab", "tab", "btab"],
    ["tab", "tab", "a"],
    ["tab", "tab", "ctrl-g"],
    ["tab", "ctrl-u"],
    ["tab", "space", "tab"],
    ["tab", "bs", "tab"],
]

# Init lines added to a case whose key path navigates. `zsh/complist` is what
# supplies menu selection at all — without the module loaded, `menu select`
# silently degrades to a plain listing on real zsh too (memory:
# compsys_list_colors), so loading it is what makes the two shells comparable
# rather than what makes them agree.
MENU_SETUP = [
    "zmodload -i zsh/complist",
    "zstyle ':completion:*' menu 'select=1'",
]


# ═════════════════════════════════════════════════════════════════════════════
# the widget axis
# ═════════════════════════════════════════════════════════════════════════════
#
# Every case in every completion harness in this tree reaches completion by
# sending TAB to whatever compinit left bound there — `expand-or-complete`,
# rebound to `_main_complete`. That is ONE entry point out of a family, and the
# entry point decides which dispatch inside the completion core runs:
#
#   Completion/compinit:539-544  rebinds EIGHT builtin completion widgets to
#                                `_main_complete`. They are eight different
#                                Widget flag sets (Src/Zle/iwidgets.list:34,40,
#                                61,62,83,86,87,103) reaching one shell
#                                function, and `list-choices` alone also
#                                carries ZLE_LASTCOL.
#   Completion/compinit:544      `menu-select` joins them, but only once
#                                zsh/complist has registered it
#                                (Src/Zle/complist.c:3589).
#   Src/Zle/zle_thingy.c:599-628 `zle -C` refuses any base widget without
#                                ZLE_ISCOMP ("invalid widget `%s'"), so the
#                                legal base set is exactly those nine and
#                                nothing else — `accept-and-menu-complete`
#                                (iwidgets.list:13) and `expand-word`
#                                (iwidgets.list:63) are NOT legal bases, which
#                                is why they appear here only as bindings.
#   Completion/compinit:516-520  a `#compdef -k` / `#compdef -K` FILE HEADER is
#                                turned into `compdef -kna` / `-Kna`, which
#                                calls `zle -C` and `bindkey` itself
#                                (compinit:311-355). This project's own records
#                                note `#compdef -k` support as missing; a
#                                generated case is how that gets settled rather
#                                than assumed.
#
# Every entry is bound EXPLICITLY in the generated init — never left to
# whatever compinit happened to do — and every init line is recorded in the
# fixture as an `@setup` header, so `--replay` reproduces the binding exactly.

# The function name a generated `zle -C` / `compdef -k` widget calls.
SFZ_FN = "_sfz_widget"

# Bases legal for `zle -C` (ZLE_ISCOMP, see above). `menu-select` is listed
# apart because it only exists after `zmodload zsh/complist`.
ZLE_C_BASES = [
    "complete-word", "delete-char-or-list", "expand-or-complete",
    "expand-or-complete-prefix", "list-choices", "menu-complete",
    "menu-expand-or-complete", "reverse-menu-complete",
]
ZLE_C_BASE_COMPLIST = "menu-select"

# Widgets compinit installs from the `#compdef -k`/`-K` headers of the files in
# Completion/Base/Widget. Verified present after `compinit -u -D` in a hermetic
# `zsh -f`; each is bound here to TAB as well, so a case's key path reaches it.
COMPSYS_WIDGETS = [
    "_complete_help",            # Widget/_complete_help  `#compdef -k complete-word \C-xh`
    "_correct_word",             # Widget/_correct_word   `#compdef -k complete-word \C-xc`
    "_expand_word",              # Widget/_expand_word    `#compdef -K ... complete-word \C-xe`
    "_history-complete-older",   # Widget/_history_complete_word `#compdef -K ... \e/`
    "_bash_complete-word",       # Widget/_bash_completions `#compdef -K ... \e~`
    "_bash_list-choices",        # Widget/_bash_completions `... list-choices ^X~`
]


def widget_ids():
    """Every widget entry point the axis can drive, in a stable order."""
    ids = ["default"]
    ids += ["std:" + w for w in ZLE_C_BASES]
    ids += ["std:menu-select", "std:expand-word", "std:accept-and-menu-complete"]
    ids += ["fn:" + w for w in COMPSYS_WIDGETS]
    ids += ["zle-C:" + b for b in ZLE_C_BASES + [ZLE_C_BASE_COMPLIST]]
    ids += ["compdef-k:" + b for b in ("complete-word", "list-choices",
                                       "menu-complete", ZLE_C_BASE_COMPLIST)]
    ids += ["compdef-K:complete-word", "compdef-K:list-choices"]
    ids += ["fpdef-k:complete-word", "fpdef-k:list-choices", "fpdef-K"]
    return ids


WIDGET_IDS = widget_ids()

# Placeholder in a `verify` expectation: the name of the widget compinit
# derives from a `#compdef -k` file, which is the file's own basename and so
# differs between a generated case (`_fzc0007`) and the self-check probe.
FILEFN = "%FILEFN%"


def widget_plan(wid, fn=SFZ_FN):
    """How one widget id is declared, fired, and proved to be bound.

    Returns a dict:
      label     the id itself, used in the per-widget report
      pre       init lines that must run before the declaration (modules)
      decl      the declaration + binding lines, run after `compinit`
      header    a replacement first line for the completer FILE, or None. Only
                the `fpdef-*` forms use one: their whole point is that compinit
                itself has to parse the header.
      fire      the harness key name that invokes the widget
      prime     keys sent before the case's key path (open a menu, etc.)
      complist  whether zsh/complist has to be loaded first
      verify    (shell command, token) — the token must appear as a word in the
                command's output for the binding to count as installed. This is
                what the self-check asserts, on both shells.
    """
    p = {"label": wid, "pre": [], "decl": [], "header": None, "fire": "tab",
         "prime": [], "complist": False, "verify": None}
    if wid == "default":
        # The control: no binding at all, i.e. exactly what every other harness
        # in this tree does. Kept in the pool so a widget-axis run always
        # contains cases judged the old way to compare against.
        return p

    what, _, arg = wid.partition(":")

    if what == "std":
        if arg == "accept-and-menu-complete":
            # iwidgets.list:13 — ZLE_MENUCMP|ZLE_KEEPSUFFIX, no ZLE_ISCOMP. It
            # accepts the current menu match and starts the next completion, so
            # it means nothing until a menu is open: TAB opens one via
            # menu-complete, then ^Xw accepts-and-continues.
            p["complist"] = True
            p["decl"] = ["bindkey '^I' menu-complete",
                         "bindkey '^Xw' accept-and-menu-complete"]
            p["fire"] = "cx-w"
            p["prime"] = ["tab"]
            p["verify"] = ("bindkey '^Xw'", "accept-and-menu-complete")
            return p
        if arg == "expand-word":
            # iwidgets.list:63 — flags 0. Not a completion widget at all; it
            # does history/alias/parameter expansion. Bound on TAB on purpose:
            # it is the widget sitting next to completion in every real keymap,
            # and nothing here has ever compared it.
            pass
        if arg == ZLE_C_BASE_COMPLIST:
            # compinit:544 — `zle -la menu-select && zle -C menu-select
            # .menu-select _main_complete`. It cannot have run at compinit time
            # here (complist is loaded afterwards), so the compsys wiring is
            # redone verbatim; without it `menu-select` is complist's raw
            # widget and never reaches a generated completer at all.
            p["complist"] = True
            p["pre"] = ["zle -C menu-select .menu-select _main_complete"]
        p["decl"] = ["bindkey '^I' %s" % arg]
        p["verify"] = ("bindkey '^I'", arg)
        return p

    if what == "fn":
        # Installed by compinit from a Widget/ file header; bound to TAB here so
        # the ordinary key paths reach it. If a shell never installed the
        # widget, the bindkey itself fails and the case reports it.
        p["decl"] = ["bindkey '^I' %s" % arg]
        p["verify"] = ("bindkey '^I'", arg)
        return p

    if what == "zle-C":
        p["complist"] = (arg == ZLE_C_BASE_COMPLIST)
        p["decl"] = ["zle -C _sfz_w %s %s" % (arg, fn),
                     "bindkey '^I' _sfz_w"]
        p["verify"] = ("bindkey '^I'", "_sfz_w")
        return p

    if what == "compdef-k":
        # compinit:333-355 — `compdef -k func comp-widget key...` does the
        # `zle -C "$func" ".$comp-widget" "$func"` and the bindkey itself, so
        # the widget is NAMED after the function.
        p["complist"] = (arg == ZLE_C_BASE_COMPLIST)
        p["decl"] = ["compdef -k %s %s '^Xw'" % (fn, arg),
                     "bindkey '^I' complete-word"]
        p["fire"] = "cx-w"
        p["verify"] = ("bindkey '^Xw'", fn)
        return p

    if what == "compdef-K":
        # compinit:318-331 — `compdef -K func widget comp-widget key ...`, in
        # triples. Two widgets over one function, on two keys; the id says
        # which of the two the case fires.
        p["decl"] = ["compdef -K %s _sfz_kw1 complete-word '^Xw' "
                     "_sfz_kw2 list-choices '^Xv'" % fn,
                     "bindkey '^I' complete-word"]
        if arg == "list-choices":
            p["fire"], p["verify"] = "cx-v", ("bindkey '^Xv'", "_sfz_kw2")
        else:
            p["fire"], p["verify"] = "cx-w", ("bindkey '^Xw'", "_sfz_kw1")
        return p

    if what == "fpdef-k":
        # The header form. compinit:516-517 turns it into `compdef -kna <file
        # basename> <comp-widget> <key>`; `-n` means it will NOT rebind a key
        # that is already bound, which is why ^Xw (free after a stock compinit)
        # is used.
        p["header"] = "#compdef -k %s ^Xw" % arg
        p["decl"] = ["bindkey '^I' complete-word"]
        p["fire"] = "cx-w"
        p["verify"] = ("bindkey '^Xw'", FILEFN)
        return p

    if wid == "fpdef-K":
        p["header"] = ("#compdef -K _sfz_fw1 complete-word ^Xw "
                       "_sfz_fw2 list-choices ^Xv")
        p["decl"] = ["bindkey '^I' complete-word"]
        p["fire"] = "cx-w"
        p["verify"] = ("bindkey '^Xw'", "_sfz_fw1")
        return p

    raise AssertionError("no plan for widget id %r" % wid)


class UnknownKey(Exception):
    pass


def key_bytes(name):
    """Bytes for one key name, strictly.

    A single character is literal; anything longer must be a defined name. No
    `.get(name, name.encode())` fallback — that silently turns a typo into
    self-inserted characters that look like a completion bug on BOTH shells.
    """
    if name in KEYS:
        return KEYS[name]
    if len(name) == 1:
        return name.encode()
    raise UnknownKey(name)


# ── crash / diagnostic scanning ──────────────────────────────────────────────

CRASH_MARKERS = (
    "panicked at",
    "capacity overflow",
    "run with `RUST_BACKTRACE",   # the panic FOOTER, not the bare env var name
    "Segmentation fault",
    "Abort trap",
    "fatal runtime error",
)

# Complaint-shaped lines. Matched per side, then set-differenced: a message
# BOTH shells print cancels; only a one-sided message is a divergence signal.
# The first pattern carries the calling frames (`_describe:compadd:114: ...`),
# which say WHERE a message came from.
DIAG_PATTERNS = tuple(re.compile(p, re.I) for p in (
    r"\b_[a-z_][a-z0-9_]*(?::[a-z_][a-z0-9_]*)*:\d+:.*",
    r"_arguments:.*",
    r"_values:.*",
    r"_describe:.*",
    r"_alternative:.*",
    r"command not found\b.*",
    r"no such file or directory\b.*",
    r"parse error\b.*",
    r"bad pattern\b.*",
    r"bad substitution\b.*",
    r"bad math expression\b.*",
    r"bad set of key/value pairs\b.*",
    r"not valid in this context\b.*",
    r"function definition file not found\b.*",
    r"invalid argument\b.*",
    r"bad option\b.*",
    r"unknown (?:option|module|signal)\b.*",
    r"no matches found\b.*",
    r"maximum nested function level reached\b.*",
    r"can't (?:open|find|read)\b.*",
    r"not enough arguments\b.*",
    r"too many arguments\b.*",
))

_ANSI_RE = re.compile(r"\x1b\[[0-9;?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b[@-Z\\-_]")
_SHELLPREFIX_RE = re.compile(r"^(?:zsh|zshrs)(?:\s*\(\w+\))?:\s*")

QUIET, NO_OUTPUT, CAPPED = "quiet", "no-output", "capped"


def diagnostics(raw, pid):
    """Complaint-shaped lines in one shell's output, normalised.

    The shell-name prefix and this child's own pid are stripped so `zsh: foo`
    and `zshrs: foo` are the SAME message; only a genuinely one-sided message
    survives the later set difference.
    """
    text = _ANSI_RE.sub("", raw.decode("utf-8", "replace"))
    if pid:
        text = text.replace(str(pid), "<PID>")
    found = set()
    for line in re.split(r"[\r\n]+", text):
        line = line.strip()
        if not line or SENTINEL in line:
            continue
        line = _SHELLPREFIX_RE.sub("", line)
        for pat in DIAG_PATTERNS:
            m = pat.search(line)
            if m:
                found.add(re.sub(r"\s+", " ", m.group()).strip())
                break
    return found


# ── raw stream tokenisation (for the stream-level diff) ──────────────────────

_TOKEN_RE = re.compile(
    rb"\x1b\[[0-?]*[ -/]*[@-~]"                  # CSI
    rb"|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)"       # OSC
    rb"|\x1b[P^_X][^\x1b]*(?:\x1b\\)?"           # DCS / PM / APC / SOS
    rb"|\x1b[@-Z\\-_]"                           # two-byte ESC
    rb"|[\x00-\x08\x0b-\x1a\x1c-\x1f\x7f]"       # lone control byte
    rb"|[\t\n\r]"                                # layout controls, one each
    rb"|[^\x00-\x1f\x7f]+"                       # printable run
)

_CTRL_NAMES = {0x07: r"\a", 0x08: r"\b", 0x09: r"\t", 0x0a: r"\n",
               0x0b: r"\v", 0x0c: r"\f", 0x0d: r"\r", 0x1b: r"\e", 0x7f: r"\x7f"}


def esc_bytes(b):
    out = []
    for ch in b:
        if ch in _CTRL_NAMES:
            out.append(_CTRL_NAMES[ch])
        elif ch < 0x20 or ch >= 0x7f:
            # High bytes are escaped as well as control bytes. Under the
            # locale axis the streams being diffed carry multibyte sequences
            # and deliberately malformed ones, and rendering them as text
            # would either move this terminal's own cursor or show two
            # different byte strings as the same glyph.
            out.append("\\x%02x" % ch)
        else:
            out.append(chr(ch))
    return "".join(out)


def tokenize_stream(raw):
    out, i, n = [], 0, len(raw)
    while i < n:
        m = _TOKEN_RE.match(raw, i)
        if m and m.end() > i:
            out.append(esc_bytes(m.group()))
            i = m.end()
        else:
            out.append(esc_bytes(raw[i:i + 1]))
            i += 1
    return out


_PRINTABLE_RE = re.compile(rb"[^\x00-\x1f\x7f]+")


def printable_bytes(raw):
    """The bytes a shell actually PAINTED, with escapes and motion removed.

    pyte's UTF-8 decoder maps every malformed sequence to one U+FFFD, so two
    shells that wrote different wrong byte strings render the identical cell
    and a grid diff calls it a pass. This is the byte-level answer to that: it
    drops the escape sequences (which legitimately differ — cursor motion is
    not content) and keeps the payload, so a divergence in what was WRITTEN
    survives whatever the decoder did to it.
    """
    out = bytearray()
    i, n = 0, len(raw)
    while i < n:
        m = _TOKEN_RE.match(raw, i)
        if not m or m.end() <= i:
            i += 1
            continue
        tok = m.group()
        if _PRINTABLE_RE.fullmatch(tok):
            out += tok
        i = m.end()
    return bytes(out)


def stream_diff(ref, test, max_lines=30):
    a, b = tokenize_stream(ref), tokenize_stream(test)
    sm = difflib.SequenceMatcher(None, a, b, autojunk=False)
    lines = []
    for tag, i1, i2, j1, j2 in sm.get_opcodes():
        if tag == "equal":
            continue
        if len(lines) >= max_lines:
            lines.append("    ... (truncated)")
            break
        ctx = "".join(a[max(0, i1 - 2):i1])
        if ctx:
            lines.append("  after %s" % ctx[-60:])
        if tag in ("replace", "delete"):
            lines.append("    zsh   - %s" % "".join(a[i1:i2])[:180])
        if tag in ("replace", "insert"):
            lines.append("    zshrs + %s" % "".join(b[j1:j2])[:180])
    return lines


# ═════════════════════════════════════════════════════════════════════════════
# the grammar
# ═════════════════════════════════════════════════════════════════════════════
#
# Every generated construct below exists because the compsys engine has a
# distinct code path for it, and several exist because that path has already
# been wrong once in this codebase (citations in the module docstring).

# Description / value text. Deliberately hostile: each entry carries at least
# one character with meaning to the spec parser, the quoting layer, or the
# display-width math. The reason `((a\:"add files to archive"))` was worth
# generating is that the SAME string has to survive `_arguments` spec parsing,
# `compdescribe` column layout, and the shell quoting around all of it.
NASTY_WORDS = [
    "add files",
    "keep going",
    "it's here",
    'say "hi" now',
    "back\\slash",
    "100% done",
    "$HOME path",
    "`cmd` sub",
    "[bracket] in",
    "a:b colon",
    "semi;colon",
    "amp&and",
    "star*glob",
    "pipe|bar",
    "tilde~home",
    "paren(s) here",
    "brace{s} here",
    "eq=sign",
    "hash#mark",
    "at@sign",
    "plain",
    "two  spaces",
    "dash-dash --opt",
    "trailing ",
]

NON_ASCII_WORDS = [
    "h\u00e9llo w\u00f6rld",
    "caf\u00e9 na\u00efve",
    "\u65e5\u672c\u8a9e desc",
    "arrow \u2192 here",
]


# \u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550
# the locale / encoding axis
# \u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550
#
# Every harness in this tree \u2014 this one included, until now \u2014 pinned
# `LANG=LC_ALL=C`. That is the degenerate setting: under `C` MB_CUR_MAX is 1,
# so every byte above 0x7f is its own unprintable character and the whole
# multibyte pathway (width computation, metafication, combining marks, invalid
# sequences) is either never entered or entered only through its escape hatch.
# The bugs this codebase has actually shipped live on the other side of that
# switch, so the locale is a fuzzable dimension here, not a constant.
#
# `family` is what the probe has to PROVE before a locale may be used:
#   single  MB_CUR_MAX 1 \u2014 the canary counts one character per byte
#   utf8    the canary decodes as UTF-8 and its columns exceed its characters
#   mb      a multibyte encoding that is NOT UTF-8 \u2014 the canary must be
#           re-segmented by that encoding's own rules, so it agrees with
#           neither of the above
#
# `codec` is what pyte is told. pyte's ByteStream only has two modes \u2014 UTF-8,
# or one codepoint per byte (`use_utf8 = False`) \u2014 so a non-UTF-8 multibyte
# locale is rendered byte-per-cell. That is deliberately the STRICTER of the
# two: it never collapses two different byte strings into one cell, so a
# padding divergence driven by a width miscount is still visible even though
# pyte cannot draw the glyph.

LOCALE_CANDIDATES = [
    # id                family    codec       why it is in the pool
    ("C",               "single", "latin-1",  "the degenerate baseline every "
                                              "other harness pins"),
    ("en_US.UTF-8",     "utf8",   "utf-8",    "the environment this shell is "
                                              "meant to replace"),
    ("C.UTF-8",         "utf8",   "utf-8",    "UTF-8 without a language \u2014 "
                                              "collation off, encoding on"),
    ("zh_CN.GB2312",    "mb",     "latin-1",  "multibyte and NOT UTF-8: the "
                                              "only family that separates "
                                              "'decodes UTF-8' from 'honours "
                                              "the locale'"),
    ("ja_JP.eucJP",     "mb",     "latin-1",  "a second non-UTF-8 multibyte "
                                              "encoding, different lead-byte "
                                              "ranges"),
    ("en_US.ISO8859-1", "single", "latin-1",  "single-byte but every byte "
                                              "printable \u2014 the opposite "
                                              "degenerate case from C"),
]

# The string the availability probe measures. Three CJK characters: nine bytes
# in UTF-8, three characters and six columns when the locale decodes UTF-8,
# and something else again under a non-UTF-8 multibyte encoding that
# re-segments the same bytes by its own lead-byte rules.
LOCALE_CANARY = b"\xe6\x97\xa5\xe6\x9c\xac\xe8\xaa\x9e"

# What each family must report for the canary as `characters:columns`.
# `single` is exact (one character per byte); the other two are asserted by
# shape, because the exact numbers are the encoding's business and not
# something this harness should hard-code as a fact about someone's libc.
LOCALE_EXPECT = {
    "single": lambda ch, co: ch == co == len(LOCALE_CANARY),
    "utf8":   lambda ch, co: ch == 3 and co == 6,
    "mb":     lambda ch, co: 1 < ch < len(LOCALE_CANARY) and co > ch,
}


# \u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550
# hostile text
# \u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550\u2550
#
# Carried as BYTES, because several entries are not valid UTF-8 and zsh reads a
# script as bytes. Each is a shape that has broken width or encoding handling
# somewhere in this tree, with the category naming what it attacks.

HOSTILE_BYTES = [
    # (name,          category,       bytes)
    ("precomposed",   "precomposed",  b"caf\xc3\xa9 file"),
    ("decomposed",    "decomposed",   b"cafe\xcc\x81 file"),
    ("combining",     "combining",    b"e\xcc\x81cole \xc3\xa0 \x63\xcc\xa7a"),
    ("combining-x3",  "combining",    b"a\xcc\x81\xcc\x82\xcc\x83 stack"),
    # 3 characters, 6 columns: the grapheme whose display width is not its
    # code-point count, which is the whole of the niceztrlen bug.
    ("wide-cjk",      "wide",         b"\xe6\x97\xa5\xe6\x9c\xac\xe8\xaa\x9e"),
    ("wide-mixed",    "wide",         b"a\xe6\x97\xa5b\xe6\x9c\xacc"),
    ("wide-hangul",   "wide",         b"\xed\x95\x9c\xea\xb5\xad\xec\x96\xb4"),
    ("emoji-zwj",     "zwj",          b"\xf0\x9f\x91\xa9\xe2\x80\x8d"
                                      b"\xf0\x9f\x92\xbb dev"),
    ("emoji-flag",    "zwj",          b"\xf0\x9f\x8f\xb4\xf3\xa0\x81\xa7"
                                      b"\xf3\xa0\x81\xa2\xf3\xa0\x81\xb3"
                                      b"\xf3\xa0\x81\xa3\xf3\xa0\x81\xb4"
                                      b"\xf3\xa0\x81\xbf"),
    ("zwj-bare",      "zwj",          b"a\xe2\x80\x8db join"),
    ("rtl-mark",      "bidi",         b"a\xe2\x80\x8fb rtl"),
    ("rtl-arabic",    "bidi",         b"\xd8\xa7\xd9\x84\xd9\x85\xd9\x84\xd9"
                                      b"\x81 file"),
    ("lrm",           "bidi",         b"x\xe2\x80\x8ey"),
    ("replacement",   "replacement",  b"\xef\xbf\xbd tofu"),
    # 0x01/0x02 are what this shell's own prompt layer uses as marks
    # (memory: prompt_sp_dangling_output), so a description carrying them is a
    # test of whether the listing path strips something it should not.
    ("c0-soh-stx",    "c0",           b"a\x01b\x02c"),
    ("c0-tab-vt",     "c0",           b"a\x0bb\x0cc"),
    ("c1-csi",        "c1",           b"a\x9bb c1"),
    ("c1-nel",        "c1",           b"x\x85y nel"),
    # 0x83 is zsh's Meta byte (Src/zsh.h STOUC/Meta) and 0x84 sits next to it;
    # a value carrying either has to survive compadd -> display -> insertion
    # unchanged or the metafication layer is eating it.
    ("meta-83",       "meta",         b"a\x83b meta"),
    ("meta-83-84",    "meta",         b"x\x83\x84y meta"),
    ("meta-83-esc",   "meta",         b"p\x83\x5cq"),
    # Not valid UTF-8 in any form. zsh reads these; a shell that insists on
    # decoding its input as UTF-8 cannot even open the file.
    ("lone-cont",     "invalid",      b"a\x80b"),
    ("overlong",      "invalid",      b"a\xc0\xafb"),
    ("truncated",     "invalid",      b"a\xe6\x97b"),
    ("bare-lead",     "invalid",      b"a\xf0b"),
    ("gb2312-text",   "invalid",      b"\xc4\xe3\xba\xc3 gb"),
]

HOSTILE_CATEGORIES = sorted({c for _n, c, _b in HOSTILE_BYTES})


class Hostile:
    """One hostile string, as bytes, as str, and as something printable."""

    __slots__ = ("name", "category", "raw", "text", "valid_utf8")

    def __init__(self, name, category, raw):
        self.name = name
        self.category = category
        self.raw = raw
        self.text = bdec(raw)
        # Whether the bytes are valid UTF-8 is DECODED, never inferred from the
        # category. The `meta` and `c1` entries are lone bytes above 0x7f and so
        # are not valid UTF-8 either, though they attack something else
        # entirely; grouping the probe scripts by intent instead of by encoding
        # put them in the shared script, where one shell's refusal to read that
        # whole file was then reported as every entry in it failing.
        try:
            raw.decode("utf-8")
            self.valid_utf8 = True
        except UnicodeDecodeError:
            self.valid_utf8 = False

    def __repr__(self):
        return "<%s %s %r>" % (self.name, self.category, self.raw)


HOSTILE = [Hostile(n, c, b) for n, c, b in HOSTILE_BYTES]

# Terminal widths drawn by `--cols auto`. 80 is the control. The rest sit at
# and around the points where a two-column glyph cannot fit: the last column of
# a full-width line, and the boundaries a listing computes its column count
# from. A width bug that counts characters instead of columns puts the glyph
# one cell over the edge, and the wrap it causes is what the grid diff sees.
COLS_POOL = [80, 80, 79, 78, 41, 40, 39, 31, 30, 21, 20]

# Plain identifiers, used where the spec grammar cannot carry a space.
PLAIN_WORDS = ["alpha", "beta", "gamma", "delta", "eps", "zeta", "eta", "theta",
               "one", "two", "three", "four", "five", "six"]

LONG_WORDS = ["--long-option-name", "verylongvaluename0123456789", "x"]


def zq(s):
    """zsh single-quoted form of an arbitrary string."""
    return "'" + s.replace("'", "'\\''") + "'"


def esc_bracket(s):
    """Description text going between `[` and `]` in a spec."""
    return s.replace("\\", "\\\\").replace("]", "\\]").replace("[", "\\[")


def emit_atom(a):
    """One spec atom as it appears on the generated command line.

    Almost every atom is a single quoted word. The `{-h,--help}'[desc]'`
    shared-description form is the exception: the brace list is expanded by the
    SHELL, so quoting it whole would hand `_arguments` one option literally
    named `{-h,--help}` and never exercise the shared-description path at all.
    """
    if a.startswith("{") and "}" in a:
        i = a.index("}")
        return a[:i + 1] + zq(a[i + 1:])
    return zq(a)


def esc_colon(s):
    """Text going into a `:`-delimited spec field."""
    return s.replace("\\", "\\\\").replace(":", "\\:")


# ═════════════════════════════════════════════════════════════════════════════
# the builtin layer
# ═════════════════════════════════════════════════════════════════════════════
#
# `_arguments`, `_values`, `_describe` and `_alternative` are SHELL functions
# standing on top of the C-ported builtins `compadd`, `compset` and `comptags`.
# A builtin bug therefore only reaches a spec-level case if some generated spec
# happens to route through the broken flag — `-U` is reachable from an
# `_arguments` spec only via a completer that passes it through, and `-D` is
# not reachable at all. The kinds below drive the builtins directly so the flag
# surface is sampled on purpose rather than by accident.
#
# The sampling is deliberately NOT uniform: every combination this codebase has
# actually shipped wrong is weighted up, with the reason recorded at the site.

# `-M` match specs. Each is a distinct compmatch code path.
COMPADD_MATCH_SPECS = [
    "m:{a-z}={A-Z}",
    "m:{a-zA-Z}={A-Za-z}",
    "r:|-=* r:|=*",
    "l:|=* r:|=*",
    "L:|no=",
    "b:-=+",
    "m:{[:lower:]}={[:upper:]}",
]

# `-X` / `-x` explanations. Prompt escapes are legal here (compwid.yo:589-611)
# and `%{...%}` is the zero-width form the listing's column math has to honour
# without counting its bytes, so an explanation carrying one is a width test as
# much as a text test.
COMPADD_EXPLS = [
    "plain expl",
    "%Bbold%b expl",
    "%Uunder%u expl",
    "%F{red}red%f expl",
    "%K{blue}bg%k expl",
    "100%% done",
    "%{\\e[32m%}zero-width%{\\e[0m%} expl",
    "%Sstandout%s expl",
    "expl with  two spaces",
]

# Literal strings for the six prefix/suffix flags. `-P`/`-S` are visible on the
# line, `-p`/`-s` are hidden, `-i`/`-I` are ignored — four different places the
# same text can end up, and they have been confused before.
COMPADD_AFFIXES = ["pre-", "=", ",", "/", "a:", "x y", "%", "-"]

COMPADD_ORDERS = ["", "match", "nosort", "numeric", "reverse", "match,reverse"]

# Words handed to a raw `compadd`. Hostile on purpose: a leading dash needs the
# `-`/`--` terminator to survive, a space needs the quoting layer, a colon and a
# glob char need the display/insert path to keep them literal.
COMPADD_WORDS = [
    "alpha", "beta", "gamma", "delta",
    "-dashfirst", "--doubledash",
    "with space", "co:lon", "star*", "back\\slash",
    "eq=sign", "tilde~x", "up", "u",
]

FS = "\x1f"      # field separator inside a structured (multi-field) atom


def split_atom(a, n):
    """Structured atom -> exactly `n` fields (missing ones become '')."""
    parts = a.split(FS)
    return (parts + [""] * n)[:n]


# ═════════════════════════════════════════════════════════════════════════════
# the SUBSTRATE catalogue
# ═════════════════════════════════════════════════════════════════════════════
#
# Every other kind in this file generates a completion API CALL — an
# `_arguments` spec, a `compadd` flag set, a `_tags` loop. That tests the
# completion engine and says nothing about the shell language the completion
# system is WRITTEN IN, and compsys is a large shell program: 179 files under
# `Completion/`, every one of them ordinary zsh.
#
# A bug in that substrate is invisible to a spec-level fuzzer and breaks
# completion everywhere at once. The measured case: `argv+=(...)` appends to the
# positional parameters in zsh and REPLACES them in this codebase, so
# `Completion/Base/Core/_description:83` — which is how every description in the
# system acquires its `h:`/`m:`/`r:`/`o:` zformat fields — builds the wrong
# argument list, and every completion in the tree renders `h:file` where zsh
# renders `file`. Running zsh's own completion test suite scored 32/229 against
# zsh's 222/229, and 75 of the 93 failures trace to that one line.
#
# So this catalogue is derived by READING the stock functions, not by
# imagination: every entry carries the `Completion/...` file:line that uses the
# construct, and an entry with no stock caller says so in its citation rather
# than borrowing one. Each entry is a self-contained fragment that sets
# `_sfz_r`, and it is used TWO ways from that single source, so the two can
# never drift:
#
#   in the completer   the fragment runs inside a real completion, under the
#                      widget/locale/key-path axes, and its `_sfz_r` is added
#                      as a match so a divergence is visible on the screen
#   standalone         the same fragment runs on `zsh -f -c` on both shells.
#                      That is the deliverable: a divergence reduces to a plain
#                      shell one-liner of the `argv+=` kind, which is what
#                      makes it fixable — and it ATTRIBUTES the divergence,
#                      because a construct that already differs outside
#                      completion is a core-language bug and one that differs
#                      only inside it is a completion bug.
#
# `%V` in a fragment is replaced by a per-slot identifier prefix, so two
# constructs in the same generated body cannot collide and the shrinker can
# delete any one of them without leaving a dangling reference.

class Construct:
    """One shell-language construct compsys depends on."""

    __slots__ = ("id", "category", "cite", "code")

    def __init__(self, cid, category, cite, code):
        self.id = cid
        self.category = category
        self.cite = cite
        self.code = [ln for ln in code.strip("\n").split("\n")]

    def lines(self, slot):
        """The fragment's shell lines, with `%V` bound to this slot."""
        pre = "_s%d_" % slot
        return [ln.replace("%V", pre) for ln in self.code]

    def script(self, slot=0):
        """The fragment as a standalone program printing one probe line.

        The result is folded exactly as the in-completion collector folds it
        (`_SUB_FOLD`), so the standalone value and the value that reached the
        listing are the same string and can be compared directly.
        """
        return "\n".join(["_sfz_r="] + self.lines(slot) +
                         ['print -r -- "@@S %s %s"' % (self.id, _SUB_FOLD)]) + "\n"


# (id, category, Completion/... citation, fragment).
#
# The fragment must run clean on `zsh -f` with nothing but this file's own
# hermetic work directory around it — that is PROVED before a run generates
# anything (see check_substrate()), and a fragment real zsh rejects is counted
# as a GENERATOR bug in its own category, never as a finding.
SUBSTRATE_SPECS = [

    # ── positional parameters ────────────────────────────────────────────────
    # The layer that broke. `argv` is an ordinary array name for the positional
    # parameters, so every array operation is also a positional operation, and
    # compsys uses that identity constantly: it appends to `argv` to extend its
    # own argument list, slices it to forward a tail, and searches it to find a
    # separator.
    ("argv-append", "positional", "Base/Core/_description:83", r"""
%Vf() { argv+=( h:$1 ); _sfz_r="$#:${(j:,:)argv}" }
%Vf one two
"""),
    ("argv-append-fields", "positional", "Base/Core/_description:84-86", r"""
%Vf() { argv+=( m:a ); argv+=( r:b ); argv+=( o:c ); _sfz_r="$#:${(j:,:)argv}" }
%Vf d e
"""),
    ("argv-append-defaults", "positional", "Base/Utility/_arguments:200-205", r"""
%Vf() { argv+=( '*=FILE*:file:_files' '*: :  ' ); _sfz_r="$#:${argv[-1]}" }
%Vf '-a[one]' '-b[two]'
"""),
    ("argv-append-persist", "positional", "Base/Core/_description:83", r"""
%Vf() { argv+=(X); _sfz_r="$_sfz_r$#," }
_sfz_r=
%Vf a
%Vf a b
%Vf a b c
"""),
    ("argv-subscript-assign", "positional", "Base/Utility/_as_if:5", r"""
%Vf() { argv[1]=("$@"); _sfz_r="$#:${(j:,:)argv}" }
%Vf a b c
"""),
    ("argv-set-tail", "positional", "Base/Utility/_arguments:35,56", r"""
%Vf() { integer l=$argv[(I)--]; set -- "${(@)argv[l+1,-1]}"; _sfz_r="$#:${(j:,:)argv}" }
%Vf a b -- c d
"""),
    ("argv-set-restore", "positional", "Base/Utility/_describe:56,72", r"""
%Vf() { %Vo=( "$@" ); set -- z; set -- "$%Vo[@]"; _sfz_r="$#:${(j:,:)argv}" }
%Vf a "b c"
"""),
    ("argv-set-single-dash", "positional", "Base/Utility/_describe:118", r"""
%Vf() { set - p "q r"; _sfz_r="$#:${(j:,:)argv}" }
%Vf a
"""),
    ("argv-shift", "positional", "Base/Core/_dispatch:18", r"""
%Vf() { shift; _sfz_r="$#:${(j:,:)argv}" }
%Vf a b c
"""),
    ("argv-shift2", "positional", "Base/Core/_description:78", r"""
%Vf() { shift 2; _sfz_r="$#:${(j:,:)argv}" }
%Vf a b c d
"""),
    ("argv-shift-named", "positional", "Base/Core/_description:116", r"""
%Vf() { %Vo=(-F _comp_ignore -J g); shift 2 %Vo; _sfz_r=${(j:,:)%Vo} }
%Vf
"""),
    ("argv-while-count", "positional", "Base/Utility/_arguments:207", r"""
%Vf() { while (( $# )); do _sfz_r="$_sfz_r$1."; shift; done }
_sfz_r=
%Vf a b c
"""),
    ("argv-at-vs-star", "positional", "Base/Utility/_describe:56", r"""
%Vf() { _sfz_r="Q@[$@] Q*[$*] U@[${(j:|:)${(@)argv}}]" }
%Vf "a b" c
"""),
    ("argv-slice-2", "positional", "Base/Core/_description:89", r"""
%Vf() { _sfz_r=${(j:,:)${(@)argv[2,-1]}} }
%Vf a b c
"""),
    ("argv-slice-5", "positional", "Base/Core/_all_labels:35", r"""
%Vf() { _sfz_r="${#${(@)argv[5,-1]}}:${(j:,:)${(@)argv[5,-1]}}" }
%Vf a b c d e f
"""),
    ("argv-slice-empty", "positional", "Base/Core/_all_labels:39", r"""
%Vf() { %Vp=-2; _sfz_r="[${(j:,:)${(@)argv[4,%Vp]}}][${(j:,:)${(@)argv[6,-1]}}]" }
%Vf a b c d e
"""),
    ("argv-last", "positional", "Base/Core/_dispatch:61,84", r"""
%Vf() { _sfz_r=${argv[-1]} }
%Vf a b c
"""),
    ("argv-ib-search", "positional", "Base/Core/_all_labels:13", r"""
%Vf() { _sfz_r=${argv[(ib:4:)-]} }
%Vf - b - d -
"""),
    ("argv-I-pattern", "positional", "Base/Completer/_approximate:57", r"""
%Vf() { setopt localoptions extendedglob; _sfz_r=${argv[(I)-[a-zA-Z]#U[a-zA-Z]#]} }
%Vf -a -xUy b
"""),
    ("argv-chain-subscript", "positional", "Base/Completer/_approximate:66-67", r"""
%Vf() { setopt localoptions extendedglob
  _sfz_r="[${argv[1,(r)-(|-)][(I)-*[JV]]}][${argv[1,(r)-(-|)][(R)-*[JV]]}]" }
%Vf -J g1 -x - -V h1
"""),
    ("argv-colon-hash", "positional", "Base/Core/_tags:50", r"""
%Vf() { setopt localoptions extendedglob
  %Vt=(a c); _sfz_r=${(j:,:)${(@)argv:#(${(j:|:)~%Vt})}} }
%Vf a b c d
"""),
    ("argv-regex-subst", "positional", "Base/Utility/_regex_arguments:60", r"""
%Vf() { setopt localoptions extendedglob
  _sfz_r=${(j:,:)${@:/(#b):(*)/"[${match[1]}]"}} }
%Vf :foo :bar plain
"""),
    ("argv-at-offset", "positional",
     "no stock caller — required by the round-6 brief", r"""
%Vf() { _sfz_r="${(j:,:)${@:2}}|${(j:,:)${@:1:2}}" }
%Vf a b c d
"""),
    ("argv-zparseopts-D", "positional", "Base/Core/_requested:6", r"""
%Vf() { local -a g; zparseopts -D -a g 1 2 V J x
  _sfz_r="g=${(j:,:)g} rest=$#:${(j:,:)argv}" }
%Vf -1 -J foo bar
"""),
    ("argv-zparseopts-KD", "positional", "Base/Core/_description:11", r"""
%Vf() { local -a n gr ign xo; gr=(keep)
  zparseopts -K -D -a n 1 2 V=gr J=ign x=xo
  _sfz_r="n=${(j:,:)n} gr=${(j:,:)gr} rest=${(j:,:)argv}" }
%Vf -2 -V vv tail
"""),
    ("argv-zparseopts-AD", "positional", "Base/Core/_normal:6", r"""
%Vf() { local -A o; local -a pc; zparseopts -A o -D - P p+:-=pc s
  _sfz_r="P=${+o[-P]} pc=${(j:,:)pc} rest=${(j:,:)argv}" }
%Vf -P -p x cmd
"""),
    ("argv-slice-call", "positional", "Base/Core/_all_labels:39", r"""
%Vg() { _sfz_r="g:$#:${(j:,:)argv}" }
%Vf() { "${(@)argv[1,1]}" "${(@)argv[2,-1]}" }
%Vf %Vg x "y z"
"""),
    ("argv-nested-frames", "positional", "Base/Core/_description:83", r"""
%Vg() { argv+=(G); _sfz_r="g:$#" }
%Vf() { %Vg m; _sfz_r="$_sfz_r f:$#:${(j:,:)argv}" }
%Vf a b
"""),

    # ── arrays ───────────────────────────────────────────────────────────────
    ("arr-local-a", "array", "Base/Utility/_arguments:37,39", r"""
%Vf() { local -a t; t=( "${(@)argv[1,-2]}" ); _sfz_r="${#t}:${(j:,:)t}" }
%Vf a "b c" d
"""),
    ("arr-append-args", "array", "Base/Utility/_arguments:24-25", r"""
%Vf() { local -a s; s+=( $1 ); s+=( $1 $2 ); _sfz_r=${(j:,:)s} }
%Vf -S -A
"""),
    ("arr-append-elem", "array", "Base/Core/_description:60", r"""
%Vq=(w1 "w 2" w3); %Vc=2; %Va=()
%Va+=( $%Vq[%Vc] )
_sfz_r="${#%Va}:${(j:,:)%Va}"
"""),
    ("arr-append-slices", "array", "Base/Core/_description:64-65", r"""
%Vq=(w1 w2 w3 w4); %Vc=2; %Va=()
%Va+=( $%Vq[1,%Vc-1] $%Vq[%Vc+1,-1] )
_sfz_r="${#%Va}:${(j:,:)%Va}"
"""),
    ("arr-subscript-assign", "array", "Base/Completer/_complete:85,99", r"""
%Va=(x y z); %Va[3]="q r"; _sfz_r="${#%Va}:${(j:,:)%Va}"
"""),
    ("arr-subscript-in", "array", "Base/Utility/_combination:80", r"""
%Vk=(a b a b); %Vp=()
%Vp[$%Vk[(in:2:)a]]=hit
_sfz_r="${#%Vp}:${(j:,:)%Vp}"
"""),
    ("arr-count", "array", "Base/Utility/_arguments:100,160", r"""
%Vt=(1 2 3)
(( $#%Vt )) && _sfz_r="dollar=$#%Vt brace=${#%Vt}"
"""),
    ("arr-typeset-Ua", "array", "Base/Utility/_arguments:49", r"""
%Vf() { typeset -Ua l; l=(a b a c b); _sfz_r="${(t)l}:${(j:,:)l}" }
%Vf
"""),
    ("arr-set-A-name", "array", "Base/Core/_description:94,100", r"""
%Vn=%Vt; %Vo=(-F _comp_ignore); set -A $%Vn "$%Vo[@]" -J -default-
_sfz_r=${(j:,:)%Vt}
"""),
    ("arr-P-splat", "array", "Base/Core/_description:114", r"""
%Vt=(o1 "o 2"); %Vn=%Vt; _sfz_r=${(j:,:)${(@P)%Vn}}
"""),
    ("arr-P-exists", "array", "Base/Utility/_arguments:47", r"""
%Vn=%Vabsent; _sfz_r=${(P)+%Vn}
%Vabsent=1; _sfz_r="$_sfz_r/${(P)+%Vn}"
"""),
    ("arr-P-assign", "array", "Base/Utility/_pick_variant:20,40", r"""
%Vv=; %Vo=(-r %Vv); : ${(P)%Vo[2]::=through-P}
_sfz_r=$%Vv
"""),
    ("arr-descr-pair", "array", "Base/Core/_description:112", r"""
setopt localoptions extendedglob
%Vm=(a:one b c:two)
%Vd=( "${(@M)%Vm:#*[^\\]:*}" )
_sfz_r="${#%Vd}:${(j:,:)%Vd}"
"""),
    ("arr-desc-unescape", "array", "Base/Core/_description:118", r"""
setopt localoptions extendedglob
%Vm=('a\:x:one' 'b:two' plain)
_sfz_r=${(j:,:)${(@)${(@)%Vm:#*[^\\]:*}:s/\\:/:/}}
"""),
    ("arr-mats-strip", "array", "Base/Utility/_describe:112,115", r"""
setopt localoptions extendedglob
%Vs=('a\:b:desc' 'c:d'); %Vn=%Vs
_sfz_r=${(j:,:)${(@)${(@M)${(@P)%Vn}##([^:\\]|\\?)##}//\\(#b)(?)/$match[1]}}
"""),
    ("arr-sparse", "array",
     "no stock caller — required by the round-6 brief", r"""
%Va=(); %Va[4]=four; _sfz_r="${#%Va}:[${(j:,:)%Va}]"
"""),
    ("arr-slice-assign", "array",
     "no stock caller — required by the round-6 brief", r"""
%Va=(1 2 3 4); %Va[2,3]=( x ); _sfz_r="${#%Va}:${(j:,:)%Va}"
"""),
    # `${(@ps.\0.)help_funcs[$i][2,-1]}` is a nested expansion over a CHAINED
    # subscript on an assoc element. `_complete_help` writes it into a `for`
    # list; the same expansion on an unquoted assignment RHS is the shape that
    # has been wrong here before, so both bases are pinned separately.
    ("arr-nested-chain-print", "array", "Base/Widget/_complete_help:53", r"""
%Va=(abcdef)
_sfz_r="[${(j:,:)${(s.c.)%Va[1][2,-1]}}]"
"""),
    # A slice whose BOUND is a search flag, then a second subscript on the
    # result — `_approximate` picks the group option this way, so getting it
    # wrong sends every corrected match into the wrong group.
    ("arr-slice-search-chain", "array", "Base/Completer/_approximate:66-67", r"""
setopt localoptions extendedglob
%Va=(A b C d)
_sfz_r="[${%Va[1,(r)b][(R)C]}][${%Va[1,(r)d][(I)C]}][${%Va[(r)d,4][(I)C]}]"
"""),
    ("arr-nested-chain-assign", "array",
     "Base/Widget/_complete_help:53 (same expansion, unquoted assignment RHS)", r"""
%Va=(abcdef)
%Vr=${(j:,:)${(s.c.)%Va[1][2,-1]}}
_sfz_r="[$%Vr]"
"""),
    ("arr-split-flag-assign", "array",
     "Base/Utility/_regex_words:42 (same (s) flag, unquoted assignment RHS)", r"""
%Vv=abcdef
%Vr=${(s.c.)%Vv[2,-1]}
_sfz_r="[$%Vr]"
"""),

    # ── associative arrays ───────────────────────────────────────────────────
    # Iteration order over a hash is not defined, so every construct here that
    # walks one sorts first — exactly as `_complete_help:50` does with `(@ok)`.
    # A case whose expected output depended on hash order would be flaky on
    # BOTH shells and would prove nothing about either.
    ("h-local-A", "assoc", "Base/Core/_normal:4,39", r"""
%Vf() { local -A o; o=( -s , -P "pre fix" )
  for %Vk in ${(ok)o}; do _sfz_r="$_sfz_r$%Vk=$o[$%Vk];"; done }
_sfz_r=
%Vf
"""),
    ("h-typeset-gA", "assoc", "Base/Utility/_pick_variant:7,41", r"""
%Vf() { (( $+%VG )) || typeset -gA %VG; %VG[$1]="$2" }
%Vf k1 v1
%Vf k2 "v 2"
_sfz_r="${(t)%VG}:${(j:,:)${(@ok)%VG}}:$%VG[k2]"
"""),
    ("h-keys-sorted", "assoc", "Base/Widget/_complete_help:50,63", r"""
typeset -A %Vh; %Vh=(b 2 a 1 c 3)
_sfz_r="${(j:,:)${(@ok)%Vh}}/${(j:,:)${(@o)${(@v)%Vh}}}"
"""),
    ("h-val-by-i", "assoc", "Zsh/Command/_zstyle:331", r"""
typeset -A %Vh; %Vh=(-e x -s y)
_sfz_r=${(v)%Vh[(i)(-e|-s|-b)]}
"""),
    ("h-K-sub", "assoc", "Base/Core/_dispatch:27,71", r"""
typeset -A %Vh; %Vh=(ab p ac q b r)
_sfz_r=${(j:,:)${(@o)${(@)%Vh[(K)a*]}}}
"""),
    ("h-k-R", "assoc", "Base/Core/_main_complete:18-19", r"""
typeset -A %Vh; %Vh=(x on y off z on)
_sfz_r=${(j:,:)${(@o)${(k)%Vh[(R)on]}}}
"""),
    ("h-plus-key", "assoc", "Base/Utility/_pick_variant:20,31", r"""
typeset -A %Vh; %Vh=(-r name)
(( $+%Vh[-r] )) && _sfz_r=have-r
(( $+%Vh[-b] )) || _sfz_r="$_sfz_r/no-b"
"""),
    ("h-append-colon-key", "assoc", "Base/Widget/_complete_help:33,87", r"""
typeset -A %Vh; %Vh=()
%Vh[:ctx:x]=",t1"
%Vh[:ctx:x]+=",t2"
_sfz_r="${(j:,:)${(@ok)%Vh}}=$%Vh[:ctx:x]"
"""),
    ("h-nul-key-split", "assoc", "Base/Widget/_complete_help:86,53", r"""
typeset -A %Vh; %Vh=()
%Vh[ctx]=$'\0'"f1"
%Vh[ctx]+=$'\0'"f2"
_sfz_r=${(j:,:)${(@ps.\0.)%Vh[ctx][2,-1]}}
"""),
    ("h-space-key", "assoc",
     "no stock caller — required by the round-6 brief", r"""
typeset -A %Vh; %Vh=("a b" 1 "c:d" 2)
%Vk="a b"
_sfz_r="${(j:|:)${(@ok)%Vh}}/$%Vh[$%Vk]"
"""),
    ("h-hide-special", "assoc", "Zsh/Type/_command_names:70", r"""
%Vf() { local -A +h commands; commands=(fake /bin/fake); _sfz_r="${(t)commands}:$commands[fake]" }
%Vf
"""),
    # The same shadow WITHOUT `+h`, so a divergence can be attributed: if both
    # forms break, `+h` is not what is wrong — shadowing a special HASHED
    # parameter with a local assoc is.
    ("h-shadow-special", "assoc", "Zsh/Type/_command_names:70 (same shadow, no +h)", r"""
%Vf() { local -A commands; commands=(fake /bin/fake); _sfz_r="${(t)commands}:$commands[fake]" }
%Vf
"""),

    # ── parameter expansion flags ────────────────────────────────────────────
    ("pf-s", "flags", "Base/Utility/_regex_words:42", r"""
%Va=(x:y:z); _sfz_r=${(j:,:)${(s.:.)%Va[1]}}
"""),
    ("pf-s-slash", "flags", "Base/Utility/_combination:69", r"""
%Vv=a-b-c; _sfz_r=${(j:,:)${(s/-/)%Vv}}
"""),
    ("pf-j-colon", "flags", "Base/Core/_main_complete:393", r"""
%Va=("=(#i)a*==34" "=b*==31"); _sfz_r=${(j.:.)%Va}
"""),
    ("pf-j-comma", "flags", "Base/Core/_description:42", r"""
%Va=(match reverse); _sfz_r=${(j.,.)%Va}
"""),
    ("pf-j-pipe-tilde", "flags", "Base/Core/_tags:50", r"""
setopt localoptions extendedglob
%Vt=(a b); %Va=(a x b y)
_sfz_r=${(j:,:)${(@)%Va:#(${(j:|:)~%Vt})}}
"""),
    ("pf-j-paren", "flags", "Zsh/Command/_zle:63", r"""
%Va=(w1 w2); _sfz_r="(${(j(|))%Va})"
"""),
    ("pf-q", "flags", "Base/Completer/_expand:77", r"""
%Vv='a b|c*'; _sfz_r=${(q)%Vv}
"""),
    ("pf-qq", "flags", "Base/Widget/_complete_debug:7", r"""
%Va=(x "y z"); _sfz_r=${(qq)%Va}
"""),
    ("pf-qqqq", "flags", "Base/Utility/_regex_arguments:60", r"""
%Vv='a b'; _sfz_r=${(qqqq)%Vv}
"""),
    ("pf-q-dash", "flags", "Base/Utility/_shadow:58,61", r"""
%Vv='a b$c'; _sfz_r=${(q-)%Vv}
"""),
    ("pf-q-plus", "flags", "Zsh/Type/_command_names:44", r"""
typeset -A %Vh; %Vh=(k1 "v 1" k2 v2)
_sfz_r=${(j:,:)${(@q+)${(@ok)%Vh}}}
"""),
    ("pf-Q", "flags", "Base/Core/_dispatch:51", r"""
%Vv="'a b' c"; _sfz_r=${(Q)%Vv}
"""),
    ("pf-e", "flags", "Base/Completer/_expand:38", r"""
%Vx=hello; %Vw='${%Vx} world'; _sfz_r=${(e)%Vw}
"""),
    ("pf-e-tilde-glob", "flags", "Base/Completer/_extensions:13", r"""
setopt localoptions extendedglob nonomatch
%Vx=al; %Vw='${%Vx}pha*'
_sfz_r=${(j:,:)${(e)~%Vw}}
"""),
    ("pf-f", "flags", "Zsh/Command/_sched:7", r"""
%Vv=$'a\nb\nc'; _sfz_r="${#${(f)%Vv}}:${(j:,:)${(f)%Vv}}"
"""),
    ("pf-M", "flags", "Base/Utility/_arguments:215", r"""
setopt localoptions extendedglob
%Vv='pre:post'; _sfz_r="${${(M)%Vv#*[^\\]:}[1,-2]}"
"""),
    ("pf-M-colhash", "flags", "Base/Core/_main_complete:252,270", r"""
%Va=(yes=long no=x true=y)
_sfz_r=${(j:,:)${(@M)%Va:#(yes|true|1|on)*}}
"""),
    ("pf-o", "flags", "Base/Completer/_user_expand:63", r"""
%Va=(gamma alpha beta); _sfz_r=${(j:,:)${(@o)%Va}}
"""),
    ("pf-O-len", "flags", "Base/Completer/_extensions:14", r"""
setopt localoptions extendedglob
%Vf=(a.b.c x.y); _sfz_r=${#${(O)%Vf//[^.]/}[1]}
"""),
    ("pf-u", "flags", "Zsh/Type/_command_names:65", r"""
%Vp=(a b a c); _sfz_r=${(j:,:)${(u)%Vp}}
"""),
    ("pf-ps-nul", "flags", "Base/Widget/_complete_help:53,66", r"""
%Vv=$'a\0b\0c'; _sfz_r=${(j:,:)${(@ps.\0.)%Vv}}
"""),
    ("pf-nest-MSI", "flags", "Base/Completer/_extensions:14", r"""
setopt localoptions extendedglob
%Vf=(a.b.c); _sfz_r=${(MSI:1:)%Vf%%.[^/]##}
"""),
    ("pf-nest-jq", "flags", "Base/Utility/_regex_arguments:67", r"""
%Va=(x "y z"); _sfz_r=${(j: :)${(qqqq)%Va[@]}}
"""),
    ("pf-b", "flags", "Zsh/Command/_zstyle:305,311", r"""
%Vv='a*b?c'; _sfz_r=${(b)%Vv}
"""),
    ("pf-z", "flags", "Base/Completer/_expand_alias:48", r"""
%Vv="a  b 'c d'"; _sfz_r=${(j:|:)${(z)%Vv}}
"""),
    ("pf-Pt", "flags", "Base/Utility/_store_cache:47", r"""
%Va=(1); %Vn=%Va; %Vs=x; %Vn2=%Vs
_sfz_r="${(Pt)%Vn}/${(Pt)%Vn2}"
"""),
    ("pf-t", "flags", "Base/Completer/_complete:16,21", r"""
typeset -A %Vh; %Va=(1); integer %Vi=1
_sfz_r="${(t)%Vh}/${(t)%Va}/${(t)%Vi}"
"""),
    ("pf-hash-arith", "flags",
     "no stock caller — required by the round-6 brief", r"""
%Vv=65; _sfz_r="${(#)%Vv}"
"""),

    # ── scoping ──────────────────────────────────────────────────────────────
    ("sc-local-curcontext", "scope", "Base/Core/_dispatch:4", r"""
%Vf() { local curcontext="$curcontext"; curcontext=":sfz:in:fn"; _sfz_r="in=$curcontext" }
%Vc=$curcontext
%Vf
_sfz_r="$_sfz_r out-unchanged=$([[ $curcontext == $%Vc ]] && print yes || print NO)"
"""),
    ("sc-typeset-g", "scope", "Base/Widget/_read_comp:29", r"""
%Vf() { typeset -g %VG=set-in-fn }
%Vf
_sfz_r="$%VG/${(t)%VG}"
"""),
    ("sc-typeset-gHi", "scope", "Base/Utility/_shadow:36", r"""
%Vf() { builtin typeset -gHi %VD=0; (( %VD++ )) }
%Vf
%Vf
_sfz_r="$%VD/${(t)%VD}"
"""),
    ("sc-typeset-gHa", "scope", "Base/Utility/_shadow:37,42", r"""
%Vf() { builtin typeset -gHa %VS; %VS+=( "$1" ) }
%Vf e1
%Vf "e 2"
_sfz_r="${(j:,:)%VS}/${(t)%VS}"
"""),
    ("sc-localoptions", "scope", "Base/Completer/_expand:10", r"""
%Vb=$options[nonomatch]
%Vf() { setopt localoptions nonomatch; _sfz_r=$options[nonomatch] }
%Vf
_sfz_r="before=$%Vb in=$_sfz_r after=$options[nonomatch]"
"""),
    ("sc-emulate-L", "scope", "Base/Utility/_arg_compile:91", r"""
%Vb=$options[shwordsplit]
%Vf() { emulate -L zsh; _sfz_r="$options[shwordsplit]$options[kshglob]" }
%Vf
_sfz_r="before=$%Vb in=$_sfz_r after=$options[shwordsplit]"
"""),
    ("sc-emulate-LR", "scope", "Base/Widget/_correct_filename:16", r"""
%Vb=$options[extendedglob]
%Vf() { emulate -LR zsh; _sfz_r=$options[extendedglob] }
%Vf
_sfz_r="before=$%Vb in=$_sfz_r after=$options[extendedglob]"
"""),
    ("sc-integer-status", "scope", "Base/Widget/_complete_debug:26", r"""
%Vg() { return 3 }
%Vf() { %Vg; integer ret=$?; _sfz_r="ret=$ret type=${(t)ret}" }
%Vf
"""),
    ("sc-funcstack-depth", "scope", "Base/Core/_all_labels:27-28", r"""
%Vg() { _sfz_r="depth=$#funcstack" }
%Vf() { %Vg }
%Vf
"""),
    ("sc-funcstack-chain", "scope", "Base/Core/_description:106", r"""
%Vg() { _sfz_r="idx=${funcstack[2,-1][(I)%Vf]}" }
%Vf() { %Vg }
%Vf
"""),
    ("sc-functrace", "scope", "Base/Utility/_shadow:42", r"""
%Vg() { _sfz_r="caller=${funcstack[2]}" }
%Vf() { %Vg }
%Vf
"""),
    ("sc-nested-dynamic", "scope", "Base/Core/_description:71", r"""
%Vg() { _sfz_r="seen=$%VL" }
%Vf() { local %VL=inner; %Vg }
%VL=outer
%Vf
_sfz_r="$_sfz_r after=$%VL"
"""),

    # ── control flow ─────────────────────────────────────────────────────────
    ("cf-always", "control", "Base/Core/_main_complete:384", r"""
%Vf() { { _sfz_r=body; false } always { _sfz_r="$_sfz_r/always" }
  _sfz_r="$_sfz_r/rc=$?" }
%Vf
"""),
    ("cf-always-return", "control", "Base/Completer/_approximate:113", r"""
%Vf() { { return 4 } always { _sfz_r=alw } }
%Vf
_sfz_r="$_sfz_r/rc=$?"
"""),
    ("cf-always-nested", "control", "Base/Utility/_call_program:36", r"""
%Vf() { { { return 5 } always { _sfz_r=inner } } always { _sfz_r="$_sfz_r/outer" } }
%Vf
_sfz_r="$_sfz_r/rc=$?"
"""),
    ("cf-return-nested", "control", "Base/Core/_all_labels:35", r"""
%Vg() { return 2 }
%Vf() { %Vg && _sfz_r=ok || _sfz_r="fail=$?"; return $? }
%Vf
_sfz_r="$_sfz_r/outer=$?"
"""),
    ("cf-andand-ret", "control", "Base/Core/_all_labels:35,39", r"""
%Vg() { return $1 }
%Vf() { local r=1; %Vg 0 && r=0; %Vg 1 && r=9; _sfz_r="r=$r" }
%Vf
"""),
    ("cf-while-break", "control", "Base/Core/_tags:26 (the _tags loop shape)", r"""
%Vf() { local i=0 ret=1
  while (( i < 4 )); do (( i++ )); (( i == 2 )) && ret=0; (( ret )) || break; done
  _sfz_r="i=$i ret=$ret" }
%Vf
"""),
    ("cf-trap-scope", "control", "Base/Core/_main_complete:159", r"""
%Vf() { setopt localoptions localtraps; TRAPINT() { return 130 }
  _sfz_r="in=${+functions[TRAPINT]}" }
%Vf
_sfz_r="$_sfz_r after=${+functions[TRAPINT]}"
"""),
    ("cf-status-reread", "control", "Base/Widget/_complete_debug:26", r"""
%Vg() { return 7 }
%Vf() { %Vg; _sfz_r="first=$?"; _sfz_r="$_sfz_r second=$?" }
%Vf
"""),

    # ── the completion context itself ────────────────────────────────────────
    # These read state that only exists inside a completion. They still run
    # clean standalone (the parameters are simply empty), which is the point:
    # the standalone side proving IDENTICAL while the in-completion side
    # diverges is what attributes a divergence to the completion system rather
    # than to the language.
    ("ctx-completer-argc", "ctx", "Base/Core/_dispatch:84 (how a completer is called)", r"""
_sfz_r="argc=$# argv=${(j:|:)argv}"
"""),
    ("ctx-words-current", "ctx", "Base/Core/_description:60,64", r"""
_sfz_r="cur=$CURRENT n=${#words} w=${(j:|:)words}"
"""),
    ("ctx-prefix-suffix", "ctx", "Base/Widget/_next_tags:8", r"""
_sfz_r="P=$PREFIX S=$SUFFIX IP=$IPREFIX IS=$ISUFFIX"
"""),
    ("ctx-curcontext", "ctx", "Base/Core/_dispatch:4", r"""
_sfz_r="cc=$curcontext"
"""),
    ("ctx-comp-tags", "ctx", "Base/Core/_all_labels:29 / _description:71", r"""
_sfz_r="tags=$_comp_tags ignore=${(j:,:)_comp_ignore}"
"""),
    ("ctx-funcstack", "ctx", "Base/Core/_main_complete:160", r"""
_sfz_r="stack=${(j:>:)funcstack}"
"""),
]

# A newline or a NUL in a result would break the listing into rows that no
# longer line up with the matches they came from. Both are folded to a visible
# two-character escape, on BOTH shells, so a divergence in the character itself
# still shows as a different fold. `${(V)...}` would be the obvious tool and is
# deliberately not used: its multibyte handling is itself a known divergence
# here, and injecting that into every substrate case would mask the constructs.
_SUB_FOLD = "${${_sfz_r//$'\\n'/\\\\n}//$'\\0'/\\\\0}"

SUBSTRATE = [Construct(*s) for s in SUBSTRATE_SPECS]
SUBSTRATE_BY_ID = {c.id: c for c in SUBSTRATE}
# Ordered as the catalogue declares them, so the printed tables read in the
# order a human would read the constructs, not alphabetically.
SUBSTRATE_CATEGORIES = list(dict.fromkeys(c.category for c in SUBSTRATE))


class Gen:
    """Seeded generator state for ONE case.

    `rng` is seeded from (seed, index) alone, so `--seed N` plus a case index
    reproduces a generation exactly no matter how many jobs ran or in what
    order.
    """

    def __init__(self, seed, idx, non_ascii, hostile=()):
        self.rng = random.Random("compsys_spec_fuzz:%d:%d" % (seed, idx))
        self.non_ascii = non_ascii
        # Hostile entries the reference shell has already been PROVED to
        # represent in this case's locale. An entry it could not is a generator
        # bug and never reaches here — see check_generated_text().
        self.hostile = list(hostile)
        # Every hostile entry this case actually used, so the width probe can
        # measure exactly those and the report can name the categories.
        self.used = []
        self.letters = list(string.ascii_lowercase)
        self.rng.shuffle(self.letters)
        self.li = 0
        self.wi = 0
        self.words = list(PLAIN_WORDS)
        self.rng.shuffle(self.words)

    def letter(self):
        c = self.letters[self.li % len(self.letters)]
        self.li += 1
        return c

    def word(self):
        w = self.words[self.wi % len(self.words)]
        self.wi += 1
        return w

    def hostile_text(self):
        """One proved-representable hostile string, recorded as used."""
        h = self.rng.choice(self.hostile)
        if h not in self.used:
            self.used.append(h)
        return h.text

    def desc(self):
        pool = NASTY_WORDS + (NON_ASCII_WORDS if self.non_ascii else [])
        n = self.rng.choice((1, 1, 2))
        parts = []
        for _ in range(n):
            # Hostile text competes with the ASCII pool rather than replacing
            # it: a description that is ENTIRELY exotic exercises the width
            # math but not the interaction between exotic text and the spec
            # parser's own metacharacters, which is where the two layers meet.
            if self.hostile and self.rng.random() < 0.45:
                parts.append(self.hostile_text())
            else:
                parts.append(self.rng.choice(pool))
        return " ".join(parts).strip() or "d"

    def msg(self):
        return self.rng.choice(["file", "value", "arg name", "what:ever", "n"])

    # ── actions ──────────────────────────────────────────────────────────────

    def wordlist(self):
        n = self.rng.randint(2, 4)
        return "(%s)" % " ".join(self.word() for _ in range(n))

    def described_list(self):
        """`((a\\:"desc with spaces" b\\:'other desc'))`.

        The construct that broke `_arguments`/`_alternative`/`_values` action
        splitting: the body is one shell word containing spaces and BOTH quote
        flavours, and a splitter that is blind to quoting tears it apart.
        """
        n = self.rng.randint(2, 3)
        parts = []
        for _ in range(n):
            item = self.word()
            d = self.desc()
            q = self.rng.choice(('"', "'"))
            if q == "'":
                # A single quote inside a single-quoted spec has to survive two
                # levels; keep the inner text free of `'` for the `'...'` form.
                d = d.replace("'", "")
            else:
                d = d.replace('"', "")
            parts.append("%s\\:%s%s%s" % (esc_colon(item), q, d, q))
        return "((%s))" % " ".join(parts)

    def lead(self, act):
        """Half the function actions get a LEADING SPACE.

        `_arguments` has two calling conventions for a function action and they
        are one space apart (Src/../Completion/Base/Utility/_arguments:449 vs
        :465): a leading space means "just call it", no leading space means
        "call it with the description arguments" —
        `"$action[1]" "$subopts[@]" "$expl[@]" "${(@)action[2,-1]}"`. A shell
        that drops `$expl` on the second form loses the group/description
        options for every match the action adds, and no spec that only ever
        used one convention would show it.
        """
        return (" " + act) if self.rng.random() < 0.5 else act

    def action(self, states, helpers, cmd, allow_state=True):
        choices = ["wordlist", "described", "files", "alternative",
                   "values-brace", "helper", "nothing"]
        if allow_state:
            choices += ["state", "state"]
        kind = self.rng.choice(choices)
        if kind == "wordlist":
            return self.wordlist()
        if kind == "described":
            return self.described_list()
        if kind == "files":
            return self.lead(self.rng.choice(
                ["_files", "_files -/", "_files -g '*.txt'"]))
        if kind == "alternative":
            return self.lead("_alternative %s %s" % (
                zq("%s:%s:%s" % (self.word(), esc_colon(self.desc()), self.wordlist())),
                zq("%s:%s:_files" % (self.word(), esc_colon(self.desc())))))
        if kind == "values-brace":
            return "{ _values %s %s %s }" % (
                zq(self.desc()),
                zq("%s[%s]" % (self.word(), esc_bracket(self.desc()))),
                zq("%s[%s]" % (self.word(), esc_bracket(self.desc()))))
        if kind == "helper":
            h = "_%s_helper%d" % (cmd, len(helpers))
            helpers.append(h)
            return self.lead(h)
        if kind == "state":
            s = "st%d" % len(states)
            states.append(s)
            return self.rng.choice(["->%s" % s, "->%s" % s])
        return ""

    # ── raw compadd ──────────────────────────────────────────────────────────

    def compadd_flags(self, pre, probes):
        """Flag tokens for one raw `compadd`.

        `pre` collects the shell lines the chosen flags REFERENCE (arrays,
        a removal function). They are emitted unconditionally, so the shrinker
        can delete any flag token without leaving a dangling reference.
        `probes` collects the names of arrays a `-O`/`-A`/`-D` flag writes to;
        those are echoed back as an extra always-visible group, because the
        whole point of those three flags is a value the completion listing
        would otherwise never show.

        Returns (flags, mode) where mode is "" (literal words), "array" (`-a`:
        the words name arrays) or "assoc" (`-k`: the words name assoc arrays).
        """
        r = self.rng
        f = []

        # -U clears CAF_MATCH — no matching at all is done. It was mis-ported
        # as CMF_HIDE (memory: compadd_flag_misports, ead9387dc6), which broke
        # every -U completer in the tree, so it is the single most valuable
        # flag here. -M is documented as ignored under -U (compwid.yo:683-689),
        # so a match spec is only generated on the other branch — that keeps
        # the two paths separable in a shrunk repro.
        if r.random() < 0.35:
            f.append("-U")
        elif r.random() < 0.35:
            f.append("-M " + zq(r.choice(COMPADD_MATCH_SPECS)))

        if r.random() < 0.30:
            f.append("-Q")
        if r.random() < 0.30:
            f.append("-P " + zq(r.choice(COMPADD_AFFIXES)))
        if r.random() < 0.30:
            f.append("-S " + zq(r.choice(COMPADD_AFFIXES)))
            if r.random() < 0.40:
                f.append("-q")          # only meaningful together with -S
        if r.random() < 0.15:
            f.append("-p " + zq(r.choice(COMPADD_AFFIXES)))
        if r.random() < 0.15:
            f.append("-s " + zq(r.choice(COMPADD_AFFIXES)))
        if r.random() < 0.15:
            f.append("-i " + zq(r.choice(COMPADD_AFFIXES)))
        if r.random() < 0.15:
            f.append("-I " + zq(r.choice(COMPADD_AFFIXES)))

        # Grouping. -o forms part of the group name space and is documented as
        # ignored on the default group (compwid.yo:540-551), so it is only ever
        # generated alongside an explicit -J/-V.
        grouped = False
        g = r.random()
        if g < 0.25:
            f.append("-J g1")
            grouped = True
        elif g < 0.45:
            f.append("-V g1")
            grouped = True
        if r.random() < 0.20:
            if not grouped:
                f.append("-J gord")
                grouped = True
            o = r.choice(COMPADD_ORDERS)
            f.append("-o" if not o else "-o " + zq(o))
        if grouped and r.random() < 0.30:
            f.append(r.choice(("-1", "-2")))
        if r.random() < 0.35:
            f.append("-X " + zq(r.choice(COMPADD_EXPLS)))
        elif r.random() < 0.20:
            f.append("-x " + zq(r.choice(COMPADD_EXPLS)))

        # -r/-R are the auto-removable-suffix path; -r was ported without
        # CMF_REMOVE at one point (memory: compadd_flag_misports, cd8503246b).
        if r.random() < 0.20:
            f.append("-r " + zq(r.choice([" \t\n;", "-,", "/", "="])))
        if r.random() < 0.10:
            pre.append("_sfz_rm() { compstate[list]=list }")
            f.append("-R _sfz_rm")

        # -W only does anything with -f (compwid.yo:663-668); generating the
        # pair is the only way the file-type tests actually run.
        if r.random() < 0.25:
            f.append("-f")
            if r.random() < 0.50:
                f.append("-W " + zq(r.choice(["./", "adir", "."])))

        # -e sets CMF_ISPAR (AUTO_PARAM_SLASH / AUTO_PARAM_KEYS); it was
        # mis-ported as CAF_NOSORT (memory: compadd_flag_misports).
        if r.random() < 0.12:
            f.append("-e")
        if r.random() < 0.12:
            f.append("-n")
        if r.random() < 0.08:
            f.append("-C")
        if r.random() < 0.10:
            f.append("-E " + str(r.randint(1, 3)))
        if r.random() < 0.12:
            if r.random() < 0.5:
                f.append("-F " + zq("(*.log *?x)"))
            else:
                pre.append("_sfz_ign=( '*a*' 'be*' )")
                f.append("-F _sfz_ign")

        # -d display strings. The interesting case is the WRONG length: the
        # documented behaviour is that surplus completions display unchanged
        # and surplus display strings are silently ignored (compwid.yo:519-534),
        # both of which are easy to get wrong by one element.
        if r.random() < 0.30:
            n = r.choice((1, 2, 3, 5))
            pre.append("_sfz_disp=( %s )" % " ".join(
                zq("disp%d %s" % (i, self.desc())) for i in range(n)))
            f.append("-d _sfz_disp")
            if r.random() < 0.40:
                f.append("-l")          # only has an effect together with -d

        mode = ""
        if r.random() < 0.10:
            pre.append("_sfz_arr=( ay be ce de )")
            f.append("-a")
            mode = "array"
        elif r.random() < 0.08:
            pre.append("typeset -A _sfz_assoc")
            pre.append("_sfz_assoc=( ka va kb vb kc vc )")
            f.append("-k")
            mode = "assoc"

        # -O / -A / -D never add a match; they hand a value back through an
        # array. Nothing in the listing shows it, so the caller echoes the
        # arrays afterwards (see Case._compadd_body) — otherwise these three
        # flags would be compared only by their side effect of adding nothing.
        for flag, arr in (("-O", "_sfz_o"), ("-A", "_sfz_a"), ("-D", "_sfz_d")):
            if r.random() < 0.10:
                if flag == "-D":
                    # -D deletes the non-matching elements of an array that
                    # must already be populated, in parallel with the words.
                    pre.append("%s=( d1 d2 d3 d4 d5 d6 )" % arr)
                else:
                    pre.append("%s=()" % arr)
                f.append("%s %s" % (flag, arr))
                probes.append(arr)

        return f, mode

    def compadd_words(self, mode):
        r = self.rng
        if mode == "array":
            return r.choice([["_sfz_arr"], ["_sfz_arr", "_sfz_arr[2,-1]"]])
        if mode == "assoc":
            return r.choice([["_sfz_assoc"], ["_sfz_assoc[(R)v*]"]])
        n = r.randint(2, 5)
        pool = list(COMPADD_WORDS)
        r.shuffle(pool)
        out = pool[:n]
        # A hostile MATCH, not only a hostile description. This is the
        # metafication round trip: the value has to survive `compadd` ->
        # the display path -> insertion on the command line unchanged, and a
        # match is the only thing that travels all three.
        if self.hostile and r.random() < 0.5:
            out.insert(r.randrange(len(out) + 1), self.hostile_text())
        return out

    # ── compset ──────────────────────────────────────────────────────────────

    def compset_op(self):
        """One `compset` invocation, as the shell line that runs it.

        Every form in compwid.yo:772-834 is represented. The `&& hit+=(...)`
        tail records the RETURN STATUS in the probe group, so a case where both
        shells end up with the same PREFIX/SUFFIX but disagreed about whether
        the test succeeded is still caught.
        """
        r = self.rng
        form = r.choice([
            "P", "P", "Pn", "S", "Sn", "p", "s", "n", "N", "q",
        ])
        if form == "P":
            pat = r.choice(["*\\=", "*:", "-*", "*,", "[^a-z]#", "?"])
            code, tag = "compset -P %s" % zq(pat), "P:%s" % pat
        elif form == "Pn":
            k = r.choice(["1", "2", "-1"])
            pat = r.choice(["*\\=", "*:", "*,"])
            code, tag = "compset -P %s %s" % (k, zq(pat)), "P%s:%s" % (k, pat)
        elif form == "S":
            pat = r.choice(["\\=*", ":*", ",*", "?"])
            code, tag = "compset -S %s" % zq(pat), "S:%s" % pat
        elif form == "Sn":
            k = r.choice(["1", "-1"])
            pat = r.choice(["\\=*", ":*"])
            code, tag = "compset -S %s %s" % (k, zq(pat)), "S%s:%s" % (k, pat)
        elif form == "p":
            k = r.randint(1, 3)
            code, tag = "compset -p %d" % k, "p:%d" % k
        elif form == "s":
            k = r.randint(1, 2)
            code, tag = "compset -s %d" % k, "s:%d" % k
        elif form == "n":
            beg = r.choice(["1", "2", "-2"])
            end = r.choice(["", "", "-1", "3"])
            code = "compset -n %s%s" % (beg, (" " + end) if end else "")
            tag = "n:%s%s" % (beg, end)
        elif form == "N":
            beg = r.choice(["'--'", "'-*'", "':'", "'*=*'"])
            end = r.choice(["", "", " '-*'"])
            code, tag = "compset -N %s%s" % (beg, end), "N:%s%s" % (beg, end)
        else:
            code, tag = "compset -q", "q"
        return code + FS + tag

    def compset_buffer(self, cmd):
        """A line whose current word actually contains what compset splits on."""
        return "%s %s" % (cmd, self.rng.choice([
            "a=b=c", "-x=val", "foo:bar:", "one,two,", "--opt=", "'a b' c",
            "x y z", "a=b c=d", "pre-", "a:b:c d", "\"q w\" e",
        ]))

    # ── tag machinery ────────────────────────────────────────────────────────

    def tag_branch(self, i):
        """One branch of a generated `_tags` loop, as a structured atom.

        Fields: form, tag, description, action. The four `_requested` /
        `_wanted` / `_next_label` / `_all_labels` entry points differ in when
        they consult `comptags` and in whether they hand `$expl` to the action,
        which is exactly where a group/description gets dropped.
        """
        r = self.rng
        form = r.choice(["requested", "requested", "wanted", "next_label",
                         "message", "message_r", "describe_x", "setup"])
        tag = "t%d%s" % (i, self.letter())
        return FS.join((form, tag, self.desc(),
                        r.choice(["compadd", "compadd", "_files", "_files -/"])))

    # ── substrate ────────────────────────────────────────────────────────────

    def substrate_ids(self, pool, n):
        """`n` distinct construct ids, drawn so every CATEGORY gets a turn.

        A flat draw from the catalogue would be dominated by whichever category
        has the most entries (`positional` and `flags` are both ~28), and a case
        that happened to draw six parameter-expansion flags would say nothing
        about scoping. So the first pick is a category, and only then an entry
        inside it.
        """
        bycat = {}
        for c in pool:
            bycat.setdefault(c.category, []).append(c)
        cats = sorted(bycat)
        out = []
        while len(out) < n and any(bycat.values()):
            cat = self.rng.choice([c for c in cats if bycat[c]])
            out.append(bycat[cat].pop(self.rng.randrange(len(bycat[cat]))).id)
        return out


# ── one generated case ───────────────────────────────────────────────────────

class Case:
    """A generated (or replayed, or injected) completer + the line to type."""

    def __init__(self, idx, seed, cmd, kind, atoms, flags, extra, buffer, keys,
                 locale="C", cols=80):
        self.idx = idx
        self.seed = seed
        self.cmd = cmd
        self.kind = kind
        self.atoms = list(atoms)     # reducible: spec strings / array entries
        self.flags = list(flags)     # reducible: harness-level flag tokens
        self.extra = dict(extra)     # states, helpers, header, arrays
        self.buffer = buffer
        self.keys = list(keys)
        # The locale BOTH shells run under, and the terminal width BOTH shells
        # are given. Never reduced by the shrinker: they are the environment
        # the divergence happened in, not a construct that caused it.
        self.locale = locale
        self.cols = cols
        self.body_override = None    # set only by --replay

    @property
    def name(self):
        return "case%04d" % self.idx if self.idx >= 0 else "adhoc"

    def clone(self, atoms=None, flags=None):
        c = Case(self.idx, self.seed, self.cmd, self.kind,
                 self.atoms if atoms is None else atoms,
                 self.flags if flags is None else flags,
                 self.extra, self.buffer, self.keys, self.locale, self.cols)
        c.body_override = self.body_override
        return c

    # `$compstate` fields a completion WIDGET can set. Every one of them is
    # documented in compwid.yo as read/write from the widget function, and none
    # of them is visible in a rendered listing — a shell that agrees on the
    # printed matches can still have decided differently about whether to
    # insert, whether to list, or whether the old list is still valid. Echoed
    # through the listing the same way the `compset` kind echoes $PREFIX.
    COMPSTATE_KEYS = ("insert", "list", "list_max", "last_prompt", "to_end",
                      "old_list", "exact", "pattern_insert")

    def body_lines(self):
        """The completer's shell body — no `#compdef` header, no helpers."""
        builders = {
            "arguments": self._arguments_body,
            "values": self._values_body,
            "describe": self._describe_body,
            "alternative": self._alternative_body,
            "compadd": self._compadd_body,
            "compset": self._compset_body,
            "tags": self._tags_body,
            "nested": self._nested_body,
            "substrate": self._substrate_body,
        }
        body = builders[self.kind]()
        groups = []
        if self.extra.get("compstate_probe"):
            entries = ['"%s=$compstate[%s]"' % (k, k) for k in self.COMPSTATE_KEYS]
            entries.append('"ret=$_sfz_ret"')
            groups.append(([], entries, "compstate", "_sfzcs"))
        wp = self.extra.get("width_texts") or []
        if wp:
            groups.append(self._width_probe(wp))
        if groups:
            body = self._probe_wrap(body, groups)
        return body

    # `${#s}` counts CHARACTERS, `${(m)#s}` counts display COLUMNS
    # (zshexpn.yo, the `m` flag applied to `#`). They are equal for ASCII and
    # for every string in a single-byte locale, and they diverge for exactly
    # the text that breaks column math: a double-width glyph, a combining mark,
    # a zero-width joiner. Comparing the two NUMBERS catches a width bug even
    # when the layout it produced happens to line up, which a grid diff cannot.
    def _width_probe(self, texts):
        pre, entries = [], []
        for i, (name, text) in enumerate(texts):
            var = "_sfz_w%d" % i
            pre.append("%s=%s" % (var, zq(text)))
            entries.append('"%s=${#%s}/${(m)#%s}:$%s"' % (name, var, var, var))
        return (pre, entries, "width chars/cols", "_sfzwd")

    def _probe_wrap(self, body, groups):
        """Run the generated body, then add probe values as visible matches.

        The body cannot simply be followed by a probe: most kinds end in
        `return ret`, so anything appended would never run. Wrapping it in a
        function keeps the body byte-identical (the shrinker still reduces the
        same atoms) and still lets the probes observe the state the body left
        behind, plus the status it returned.
        """
        out = ["_sfz_main() {"]
        out += ["  " + ln for ln in body]
        out.append("}")
        # `"$@"` is forwarded, not dropped. Without it the wrapper silently
        # changes what the body sees: a completer's own positional parameters
        # are part of how compsys calls it (Base/Core/_dispatch:84), and the
        # `substrate` kind reads them directly. Applied identically to both
        # shells, so it cannot make a comparison greener — only more faithful.
        out.append('_sfz_main "$@"; _sfz_ret=$?')
        for pre, entries, label, group in groups:
            out += pre
            out += self._probe_group(entries, label, group=group)
        out.append("return $_sfz_ret")
        return out

    def helper_lines(self):
        out = []
        for h in self.extra.get("helpers", []):
            out.append("%s() { compadd -- %s }" % (h, " ".join(PLAIN_WORDS[:3])))
        if self.extra.get("helpers"):
            out.append("")
        pre = self.extra.get("pre", [])
        if pre:
            out += list(pre) + [""]
        return out

    # The completer file dropped into the throwaway fpath. Its first line is
    # normally `#compdef <cmd>`; the `fpdef-*` widget forms replace it with a
    # `#compdef -k` / `-K` header, which is the whole point of those forms —
    # compinit itself has to parse it (Completion/compinit:516-520).
    def completer(self):
        if self.body_override is not None:
            return self.body_override
        out = [self.extra.get("compdef_header") or ("#compdef %s" % self.cmd), ""]
        out += self.helper_lines()
        out += self.body_lines()
        return "\n".join(out) + "\n"

    # Every line the generated init file runs after `compinit`, in order:
    # modules, the widget function (when the widget calls one), then the
    # declaration and its binding. A replayed fixture carries these verbatim in
    # its `@setup` headers, so `--replay` reconstructs the exact shell state.
    def init_lines(self):
        if self.extra.get("setup_verbatim"):
            return list(self.extra["setup_verbatim"])
        out = list(self.extra.get("setup_pre", []))
        if self.extra.get("widget_fn") == "body":
            # The widget's function IS the generated completer, so a shrunk
            # atom list shrinks the widget too — the body is rebuilt from
            # self.atoms here, never snapshotted at generation time.
            out.append("%s() {" % SFZ_FN)
            out += ["  " + ln for ln in self.helper_lines() + self.body_lines()]
            out.append("}")
        out += list(self.extra.get("setup", []))
        return out

    # ── the builtin layer ────────────────────────────────────────────────────

    def _probe_group(self, entries, label="probe", group="_sfzprobe"):
        """A `compadd` whose matches ARE the values under test.

        A completion listing only ever shows what got added as a match, so a
        parameter (`$PREFIX` after a `compset`) or an out-parameter (the array
        a `compadd -O` filled) is invisible to a screen diff. Adding them as
        `-U -Q` matches puts them through the ordinary listing path and makes a
        divergence in the VALUE visible, not only a divergence in the final
        list. `-U` is required: the values do not match the word on the line
        and would otherwise be filtered out before they were ever displayed.
        """
        return ["compadd -U -Q -J %s -X %s -- %s"
                % (group, zq(label), " ".join(entries))]

    def _compadd_body(self):
        term = self.extra.get("term", "--")
        flags = " ".join(self.flags)
        words = " ".join(self.atoms)
        out = []
        if words:
            out.append("compadd %s %s %s" % (flags, term, words))
        else:
            # Every word was reduced away: still exercise the flag parse.
            out.append("compadd %s %s" % (flags, term))
        probes = self.extra.get("probes", [])
        if probes:
            out.append("")
            out += self._probe_group(
                ['"%s=${(j:,:)%s}"' % (p, p) for p in probes], "out-arrays")
        return out

    def _compset_body(self):
        out = ["local -a _sfz_hit"]
        for a in self.atoms:
            code, tag = split_atom(a, 2)
            out.append("%s && _sfz_hit+=( %s )" % (code, zq("hit:" + tag)))
        out.append("")
        # Every parameter compset is documented to move (compwid.yo:772-834),
        # plus the word array it can rewrite, echoed back through the listing.
        out += self._probe_group([
            '"PRE=$PREFIX"', '"SUF=$SUFFIX"',
            '"IPRE=$IPREFIX"', '"ISUF=$ISUFFIX"',
            '"QIPRE=$QIPREFIX"', '"QISUF=$QISUFFIX"',
            '"CUR=$CURRENT"', '"WORDS=${(j:|:)words}"',
            '"HIT=${(j:,:)_sfz_hit}"',
        ], "compset state")
        out.append("")
        out.append("compadd -- %s" % " ".join(self.extra.get("tail", PLAIN_WORDS[:3])))
        return out

    def _substrate_body(self):
        """The shell-language constructs, run inside a real completion.

        Each atom is a catalogue id. Its fragment runs at the completer's own
        top level — not inside a helper — so the constructs that read the
        completer's positional parameters and the completion specials see what
        compsys itself would see. The result each fragment leaves in `_sfz_r`
        is collected and then added as a match, so a divergence in a VALUE is
        visible on the rendered screen and not only in a match set.

        The completer still adds ordinary matches afterwards, so the widget and
        key-path axes have a real listing to drive; a body that only ever added
        probe values would never open a menu.
        """
        out = ["local -a _sfz_sub", "local _sfz_r", ""]
        for slot, cid in enumerate(self.atoms):
            c = SUBSTRATE_BY_ID.get(cid)
            if c is None:                      # only reachable via a hand-edited
                continue                       # fixture naming a removed entry
            out.append("# %s  (%s)  %s" % (c.id, c.category, c.cite))
            out.append("_sfz_r=")
            out += c.lines(slot)
            # A newline or a NUL in a result would break the listing into rows
            # that no longer line up with the matches they came from. Folded to
            # a visible escape on BOTH shells, so a divergence in the character
            # itself still shows as a different fold.
            out.append("_sfz_sub+=( \"%s=%s\" )" % (c.id, _SUB_FOLD))
            out.append("")
        if self.atoms:
            out += self._probe_group(
                ['"$_sfz_sub[%d]"' % (i + 1) for i in range(len(self.atoms))],
                "substrate", group="_sfzsub")
            out.append("")
        out.append("compadd -- %s" % " ".join(self.extra.get("tail",
                                                             PLAIN_WORDS[:3])))
        return out

    def _tags_body(self):
        """A generated `_tags` offer/loop, plus the standalone tag callers.

        `_requested` / `_next_label` / `_message` / `_setup` belong INSIDE the
        `while _tags` loop — they consult the offer `_tags` already made.
        `_wanted` and `_describe` make their own offer (`_wanted` calls `_tags
        "$1"` itself), so they are emitted outside it; nesting one inside the
        loop would re-offer mid-iteration and is not a shape any real completer
        has, which would make a divergence unattributable.
        """
        branches = [split_atom(a, 4) for a in self.atoms]
        _solo_forms = ("wanted", "describe_x", "message", "message_r")
        loop = [b for b in branches if b[0] not in _solo_forms]
        solo = [b for b in branches if b[0] in _solo_forms]
        out = ["local expl ret=1", ""]

        for form, tag, descr, act in solo:
            if form == "wanted":
                # _wanted takes the command INLINE and runs the whole
                # _tags/_all_labels loop itself, so the command must not repeat
                # `$expl` — _all_labels is what passes it.
                cmd = ("compadd " + " ".join(PLAIN_WORDS[:3])
                       if act == "compadd" else act)
                out.append("_wanted %s expl %s %s && ret=0"
                           % (tag, zq(descr), cmd))
            elif form in ("message", "message_r"):
                # `_message` opens its own `_tags messages` offer, so it is a
                # standalone caller too; running it inside another `_tags` loop
                # re-offers mid-iteration and the loop stops terminating.
                out.append("_message %s%s && ret=0"
                           % ("-r " if form == "message_r" else "", zq(descr)))
            else:
                # `-x` makes the description show even with no matches, via
                # `compadd -x` rather than `-X` (_describe -> _description).
                out.append("_describe -x -t %s %s _sfz_dx && ret=0"
                           % (tag, zq(descr)))
        if solo:
            out.append("")

        if loop:
            out.append("_tags %s" % " ".join(b[1] for b in loop))
            out.append("while _tags; do")
            for form, tag, descr, act in loop:
                add = ("compadd \"$expl[@]\" %s" % " ".join(PLAIN_WORDS[:3])
                       if act == "compadd" else "%s \"$expl[@]\"" % act)
                if form == "requested":
                    out.append("  _requested %s expl %s && { %s && ret=0 }"
                               % (tag, zq(descr), add))
                elif form == "next_label":
                    out.append("  _requested %s && while _next_label %s expl %s; do"
                               % (tag, tag, zq(descr)))
                    out.append("    %s && ret=0" % add)
                    out.append("  done")
                else:  # setup
                    out.append("  _setup %s" % tag)
                    out.append("  _requested %s expl %s && { %s && ret=0 }"
                               % (tag, zq(descr), add))
            out.append("  (( ret )) || break")
            out.append("done")
            out.append("")

        out.append("return ret")
        return out

    def _nested_body(self):
        """Outer completer -> sub-command completer -> a third level.

        `_arguments '*::args:->rest'` followed by `shift words; (( CURRENT-- ))`
        is the standard sub-command dispatch, and it is the point where the
        completion context is re-entered with rewritten `words`/`CURRENT` while
        `curcontext`, `$state`, `$line` and `opt_args` from the OUTER call are
        still live. State leaking across that boundary is invisible to any
        single-level case.
        """
        inner = self.extra.get("inner", [])
        deep = self.extra.get("deep", [])
        sub = self.extra.get("sub", "sub")
        out = []
        out.append("_%s_deep() {" % self.cmd)
        out.append("  local curcontext=\"$curcontext\" state line ret=1")
        out.append("  typeset -A opt_args")
        if deep:
            out.append("  _arguments -C \\")
            for i, a in enumerate(deep):
                out.append("    %s%s" % (emit_atom(a),
                                         " \\" if i < len(deep) - 1 else " && ret=0"))
        else:
            out.append("  _describe -t deep 'deep' _sfz_deep && ret=0")
        out.append("  return ret")
        out.append("}")
        out.append("")
        out.append("_%s_sub() {" % self.cmd)
        out.append("  local curcontext=\"$curcontext\" state line ret=1")
        out.append("  typeset -A opt_args")
        out.append("  _arguments -C \\")
        for a in inner:
            out.append("    %s \\" % emit_atom(a))
        out.append("    '*::deeper:->deeper' && ret=0")
        out.append("  case $state in")
        out.append("    (deeper) shift words; (( CURRENT-- )); _%s_deep && ret=0 ;;"
                   % self.cmd)
        out.append("  esac")
        out.append("  return ret")
        out.append("}")
        out.append("")
        out.append("local curcontext=\"$curcontext\" state line ret=1")
        out.append("typeset -A opt_args")
        out.append("_arguments -C \\")
        for a in self.atoms:
            out.append("  %s \\" % emit_atom(a))
        out.append("  '1:command:((%s\\:\"the sub-command\" other\\:\"another one\"))' \\"
                   % sub)
        out.append("  '*::args:->rest' && ret=0")
        out.append("")
        out.append("case $state in")
        out.append("  (rest) shift words; (( CURRENT-- )); _%s_sub && ret=0 ;;" % self.cmd)
        out.append("esac")
        out.append("")
        out.append("return ret")
        return out

    def _arguments_body(self):
        out = ["local curcontext=\"$curcontext\" state state_descr line ret=1",
               "typeset -A opt_args", ""]
        flags = " ".join(self.flags)
        if not self.atoms:
            out.append("_arguments %s && ret=0" % flags)
        else:
            out.append("_arguments %s \\" % flags)
            for i, a in enumerate(self.atoms):
                tail = " \\" if i < len(self.atoms) - 1 else " && ret=0"
                out.append("  %s%s" % (emit_atom(a), tail))
        states = self.extra.get("states", [])
        if states:
            out.append("")
            out.append("case $state in")
            for s, act in states:
                out.append("  (%s) %s && ret=0 ;;" % (s, act))
            out.append("esac")
        out.append("")
        out.append("return ret")
        return out

    def _values_body(self):
        flags = " ".join(self.flags)
        parts = [zq(self.extra.get("header", "value"))] + [zq(a) for a in self.atoms]
        return ["_values %s %s" % (flags, " ".join(parts))]

    def _describe_body(self):
        out = ["local -a _m _d"]
        out.append("_m=( %s )" % " ".join(zq(a) for a in self.atoms))
        if self.extra.get("two_array"):
            out.append("_d=( %s )" % " ".join(
                zq(a.split(":", 1)[1] if ":" in a else a) for a in self.atoms))
        flags = " ".join(self.flags)
        arrays = "_m _d" if self.extra.get("two_array") else "_m"
        out.append("_describe %s %s %s" % (flags, zq(self.extra.get("header", "hdr")), arrays))
        return out

    def _alternative_body(self):
        if not self.atoms:
            return ["_alternative"]
        out = ["_alternative \\"]
        for i, a in enumerate(self.atoms):
            out.append("  %s%s" % (zq(a), " \\" if i < len(self.atoms) - 1 else ""))
        return out


def generate(seed, idx, args):
    # The ENVIRONMENT axes are drawn from their own seeded stream, not from
    # `g.rng`. Two reasons: the hostile-text pool depends on the locale, so the
    # locale has to exist before Gen does; and drawing them separately keeps
    # the spec stream for a given (seed, index) byte-identical to what it was
    # before this axis existed, so a fixture saved by an earlier run still
    # regenerates.
    pick = random.Random("compsys_spec_fuzz:env:%d:%d" % (seed, idx))
    locale = pick.choice(args.locale_pool)
    cols = pick.choice(args.cols_pool) if args.cols_pool else args.cols
    hostile = args.text_pool.get(locale, []) if args.hostile else []

    g = Gen(seed, idx, args.non_ascii, hostile)
    cmd = "fzc%04d" % (idx if idx >= 0 else 9999)
    kinds = args.kinds
    kind = g.rng.choice(kinds)
    states = []
    helpers = []
    atoms = []
    flags = []
    extra = {}

    if kind == "arguments":
        for f in ("-s", "-S", "-C", "-w", "-W"):
            if g.rng.random() < 0.35:
                flags.append(f)
        if g.rng.random() < 0.15:
            flags.append('-A "-*"')
        n = g.rng.randint(2, 6)
        made_rest = False
        made_num = 0
        for _ in range(n):
            forms = ["flag", "flag", "optarg", "eqarg", "excl", "repeat",
                     "shared", "plusflag", "num"]
            if not made_rest:
                forms += ["rest", "rest2", "rest3"]
            form = g.rng.choice(forms)
            if form == "flag":
                atoms.append("-%s[%s]" % (g.letter(), esc_bracket(g.desc())))
            elif form == "plusflag":
                atoms.append("+%s[%s]" % (g.letter(), esc_bracket(g.desc())))
            elif form == "optarg":
                atoms.append("-%s+[%s]:%s:%s" % (
                    g.letter(), esc_bracket(g.desc()), esc_colon(g.msg()),
                    g.action(states, helpers, cmd)))
            elif form == "eqarg":
                atoms.append("--%s=-[%s]:%s:%s" % (
                    g.word(), esc_bracket(g.desc()), esc_colon(g.msg()),
                    g.action(states, helpers, cmd)))
            elif form == "excl":
                a, b = g.letter(), g.word()
                atoms.append("(-%s --%s)-%s[%s]" % (a, b, a, esc_bracket(g.desc())))
                atoms.append("(-%s --%s)--%s[%s]" % (a, b, b, esc_bracket(g.desc())))
            elif form == "repeat":
                atoms.append("*-%s[%s]" % (g.letter(), esc_bracket(g.desc())))
            elif form == "shared":
                # `{-h,--help}[desc]` — one description shared by two names.
                # compdescribe has to interleave the alias names with the
                # description into ONE row; getting that wrong is a pinned bug.
                c, w = g.letter(), g.word()
                atoms.append("{-%s,--%s}[%s]" % (c, w, esc_bracket(g.desc())))
            elif form == "num":
                made_num += 1
                atoms.append("%d:%s:%s" % (made_num, esc_colon(g.msg()),
                                           g.action(states, helpers, cmd)))
            elif form in ("rest", "rest2", "rest3"):
                made_rest = True
                colons = {"rest": ":", "rest2": "::", "rest3": ":::"}[form]
                atoms.append("*%s%s:%s" % (colons, esc_colon(g.msg()),
                                           g.action(states, helpers, cmd,
                                                    allow_state=(form == "rest"))))
        extra["states"] = [(s, g.action([], helpers, cmd, allow_state=False) or
                            "_files") for s in states]

    elif kind == "values":
        if g.rng.random() < 0.6:
            flags.append("-s " + zq(g.rng.choice([",", ":", "+"])))
        if g.rng.random() < 0.3:
            flags.append("-S " + zq(g.rng.choice(["=", ":"])))
        if g.rng.random() < 0.2:
            flags.append("-w")
        extra["header"] = g.desc()
        for _ in range(g.rng.randint(2, 5)):
            form = g.rng.choice(["plain", "plain", "witharg", "excl"])
            if form == "plain":
                atoms.append("%s[%s]" % (g.word(), esc_bracket(g.desc())))
            elif form == "witharg":
                atoms.append("%s[%s]:%s:%s" % (
                    g.word(), esc_bracket(g.desc()), esc_colon(g.msg()),
                    g.action([], helpers, cmd, allow_state=False)))
            else:
                a, b = g.word(), g.word()
                atoms.append("(%s)%s[%s]" % (a, b, esc_bracket(g.desc())))

    elif kind == "describe":
        for f in ("-o", "-O", "-V", "-1", "-2", "-J", "-x"):
            if g.rng.random() < 0.18:
                flags.append(f)
        if g.rng.random() < 0.4:
            flags.append("-t " + g.word())
        extra["header"] = g.desc()
        extra["two_array"] = g.rng.random() < 0.35
        opts = "-o" in flags or "-O" in flags
        shared = g.desc()          # deliberately reused: shared-description rows
        for i in range(g.rng.randint(2, 5)):
            name = ("-" + g.letter()) if opts else g.word()
            d = shared if g.rng.random() < 0.45 else g.desc()
            atoms.append("%s:%s" % (esc_colon(name), d))

    elif kind == "alternative":
        for _ in range(g.rng.randint(2, 4)):
            act = g.action([], helpers, cmd, allow_state=False) or "_files"
            atoms.append("%s:%s:%s" % (g.word(), esc_colon(g.desc()), act))

    elif kind == "compadd":
        pre = []
        probes = []
        flags, mode = g.compadd_flags(pre, probes)
        atoms = g.compadd_words(mode)
        extra["pre"] = pre
        extra["probes"] = probes
        extra["term"] = g.rng.choice(["--", "--", "-"])

    elif kind == "compset":
        for _ in range(g.rng.randint(1, 3)):
            atoms.append(g.compset_op())
        extra["tail"] = PLAIN_WORDS[:3]

    elif kind == "tags":
        extra["pre"] = ["_sfz_dx=( %s )" % " ".join(
            zq("%s:%s" % (w, g.desc())) for w in PLAIN_WORDS[:3])]
        for i in range(g.rng.randint(1, 3)):
            atoms.append(g.tag_branch(i))

    elif kind == "substrate":
        # The construct pool is what the reference shell has already been
        # PROVED to accept (check_substrate); a fragment real zsh rejects is a
        # generator bug and never reaches a case.
        extra["tail"] = PLAIN_WORDS[:3]
        atoms = g.substrate_ids(args.substrate_pool, args.substrate_count)

    elif kind == "nested":
        extra["pre"] = ["_sfz_deep=( %s )" % " ".join(
            zq("%s:%s" % (w, g.desc())) for w in PLAIN_WORDS[:3])]
        extra["sub"] = g.word()
        extra["inner"] = ["-%s[%s]" % (g.letter(), esc_bracket(g.desc())),
                          "-%s+[%s]:%s:%s" % (g.letter(), esc_bracket(g.desc()),
                                              esc_colon(g.msg()),
                                              g.action([], helpers, cmd,
                                                       allow_state=False) or "_files")]
        extra["deep"] = ["--%s=-[%s]:%s:%s" % (g.word(), esc_bracket(g.desc()),
                                               esc_colon(g.msg()), g.described_list()),
                         "*:%s:%s" % (esc_colon(g.msg()), g.wordlist())]
        for _ in range(g.rng.randint(0, 2)):
            atoms.append("-%s[%s]" % (g.letter(), esc_bracket(g.desc())))

    else:
        raise AssertionError("no generator for kind %r" % kind)

    extra["helpers"] = helpers

    if kind == "compset":
        buf = g.compset_buffer(cmd)
    elif kind == "nested":
        # Deep enough that the sub-command dispatch has actually happened:
        # `<cmd> <sub> [-]` is inside `_<cmd>_sub`, one more word is inside
        # `_<cmd>_deep`.
        buf = "%s %s %s" % (cmd, extra["sub"],
                            g.rng.choice(["", "-", "--", "x ", "-a ", "x -"]))
    else:
        prefix = g.rng.choice(["", "", "-", "--", "-" + g.letters[0],
                               PLAIN_WORDS[0][0], "a", "x"])
        buf = "%s %s" % (cmd, prefix)

    keys = list(args.keys) if args.keys else list(g.rng.choice(KEY_PATHS))
    setup_pre = []
    if set(keys) & NAV_KEYS:
        # Without menu selection the nav keys are just movement commands and
        # the case would prove nothing about the menu engine.
        setup_pre += list(MENU_SETUP)

    # ── the widget the case is judged through ────────────────────────────────
    wid = g.rng.choice(args.widget_pool)
    plan = widget_plan(wid)
    extra["widget"] = wid
    extra["compdef_header"] = plan["header"]
    if plan["complist"] and "zmodload -i zsh/complist" not in setup_pre:
        setup_pre.insert(0, "zmodload -i zsh/complist")
    setup_pre += plan["pre"]
    extra["setup"] = list(plan["decl"])
    if plan["decl"] and wid.startswith(("zle-C:", "compdef-k:", "compdef-K:")):
        # `zle -C` / `compdef -k` name a FUNCTION to generate the matches. Two
        # forms exist and they are not the same code path: `_generic` routes
        # the widget back through `_main_complete` and the whole compsys
        # machinery, while a direct function is called by the completion core
        # with nothing between it and the builtins.
        if g.rng.random() < 0.5:
            extra["widget_fn"] = "body"
        else:
            extra["setup"] = [ln.replace(SFZ_FN, "_generic") for ln in plan["decl"]]
    extra["setup_pre"] = setup_pre

    # The key path is written in terms of `tab`; a widget bound elsewhere has
    # every `tab` in it rewritten to the key it actually sits on, so the same
    # fourteen paths (menu start, cycling, filter letter, abort) are judged
    # through every entry point rather than only through TAB.
    if plan["fire"] != "tab":
        keys = [plan["fire"] if k == "tab" else k for k in keys]
    keys = list(plan["prime"]) + keys

    if args.compstate_probe == "always":
        extra["compstate_probe"] = True
    elif args.compstate_probe == "auto":
        extra["compstate_probe"] = g.rng.random() < 0.35

    # The width probe measures exactly the hostile strings this case used —
    # not the whole corpus — so a shrunk repro carries the two numbers for the
    # one string that mattered and nothing else.
    if g.used and (args.width_probe == "always"
                   or (args.width_probe == "auto" and pick.random() < 0.6)):
        extra["width_texts"] = [(h.name, h.text) for h in g.used]
    extra["hostile_used"] = [(h.name, h.category) for h in g.used]

    return Case(idx, seed, cmd, kind, atoms, flags, extra, buf, keys,
                locale=locale, cols=cols)


# ═════════════════════════════════════════════════════════════════════════════
# hermetic environment
# ═════════════════════════════════════════════════════════════════════════════

def hermetic_fpath(zsh):
    """The fpath a completely clean `zsh -f` compiles in.

    Read from `env -i` on purpose: this session exports FPATH (the user's ~50
    plugin dirs), and `zsh -f` honours an inherited FPATH, so reading it any
    other way silently drags the whole host completion set into a harness whose
    entire point is that it does not depend on one.
    """
    out = subprocess.run([zsh, "-f", "-c", "print -rl -- $fpath"],
                         capture_output=True, text=True, timeout=20, env={}).stdout
    dirs = [d for d in out.splitlines() if d and os.path.isdir(d)]
    if not dirs:
        sys.exit("compsys_spec_fuzz: %s reports an empty fpath — cannot build a "
                 "hermetic compsys environment" % zsh)
    return dirs


# Files the `_files` actions complete against. Fixed content so a `_files`
# action is a deterministic comparison instead of whatever the cwd holds.
WORK_FILES = ["alpha.txt", "beta.txt", "gamma.log", "delta.conf"]
WORK_DIRS = ["adir", "bdir"]


def build_case_dir(root, case, fpath_dirs):
    """Materialise one case's throwaway $fpath, $HOME, cwd and init file."""
    d = os.path.join(root, case.name)
    fp = os.path.join(d, "fpath")
    work = os.path.join(d, "work")
    os.makedirs(fp, exist_ok=True)
    os.makedirs(work, exist_ok=True)
    for f in WORK_FILES:
        p = os.path.join(work, f)
        if not os.path.exists(p):
            open(p, "w").close()
    for sub in WORK_DIRS:
        os.makedirs(os.path.join(work, sub), exist_ok=True)

    # Byte-safe: a generated description can contain bytes that are not valid
    # UTF-8, and zsh reads the file as bytes. See _wopen().
    with _wopen(os.path.join(fp, "_" + case.cmd)) as f:
        f.write(case.completer())

    init = os.path.join(d, "init.zsh")
    with _wopen(init) as f:
        f.write(
            "# generated by %s — seed %d %s\n"
            "fpath=( %s )\n"
            "PROMPT='%s '\n"
            "RPROMPT=''\n"
            "PS2='> '\n"
            "setopt no_beep\n"
            "builtin cd %s\n"
            "autoload -Uz compinit\n"
            "compinit -u -D\n"
            "%s"
            "print -u2 ''\n"
            % (SELF, case.seed, case.name,
               " ".join(shlex.quote(p) for p in [fp] + fpath_dirs),
               SENTINEL, shlex.quote(work),
               "".join(ln + "\n" for ln in case.init_lines())))
    return d, init


def child_env(case_dir, locale):
    """The environment BOTH shells get — built from scratch, nothing inherited.

    HOME and cwd are inside the case directory, so `~` and `_files` are
    hermetic too. PATH is the parent's only so the harness can find the shells'
    own helpers; no completion in the generated grammar shells out to a
    host command.

    `locale` sets LANG and LC_ALL IDENTICALLY on both sides. It is never
    derived per shell and never defaulted differently: a comparison in which
    the two shells sat in different locales would not be a comparison.
    """
    env = {
        "TERM": "xterm-256color",
        "LANG": locale,
        "LC_ALL": locale,
        "PATH": "/usr/bin:/bin:/usr/sbin:/sbin",
        "HOME": case_dir,
        # zshrs's autosuggest / syntax-highlight layer has no zsh counterpart;
        # its ghost text would diff on every case. Silences that LAYER only —
        # the completion engine under test is untouched.
        "ZSHRS_NATIVE_ZLE_FX": "0",
        # zshrs ships ~145 builtins zsh does not; any listing that enumerates
        # $builtins diverges by construction. Hides the extension names for the
        # comparison only; dispatch is unchanged.
        "ZSHRS_HIDE_EXT_BUILTINS": "1",
        "RUST_BACKTRACE": "1",
    }
    for k in ("ZSHRS_LOG", "RUST_LOG"):
        if k in os.environ:
            env[k] = os.environ[k]
    return env


# ═════════════════════════════════════════════════════════════════════════════
# one shell on one pty
# ═════════════════════════════════════════════════════════════════════════════

class Session:
    def __init__(self, argv, env, rows, cols, settle_ms, cwd, use_utf8=True):
        self.rows, self.cols = rows, cols
        self.settle = settle_ms / 1000.0
        self.screen = _TolerantScreen(cols, rows)
        self.stream = pyte.ByteStream(self.screen)
        # pyte decodes bytes as UTF-8 or as one codepoint per byte, and has no
        # third mode. A UTF-8 locale gets the UTF-8 decoder, which is what
        # makes pyte apply the same double-width rule the real terminal does,
        # so a column shifted by a width miscount shows up in the GRID.
        # Everything else gets byte-per-cell — the stricter reading: it can
        # draw no glyph, but it also never collapses two different byte
        # strings into one cell the way the UTF-8 decoder's U+FFFD does.
        self.stream.use_utf8 = use_utf8
        self.raw = bytearray()
        self.mark = 0
        self.dead = False
        self.status = None
        self.events = []
        with _FORK_LOCK:
            self.pid, self.fd = pty.fork()
            if self.pid == 0:
                try:
                    os.chdir(cwd)
                    os.execvpe(argv[0], argv, env)
                except BaseException as exc:   # pragma: no cover — child only
                    try:
                        os.write(2, ("exec failed: %s\n" % exc).encode())
                    finally:
                        os._exit(127)
        try:
            fcntl.ioctl(self.fd, termios.TIOCSWINSZ,
                        struct.pack("HHHH", rows, cols, 0, 0))
        except OSError:
            self.close()
            raise

    def _read_once(self, timeout):
        r, _, _ = select.select([self.fd], [], [], timeout)
        if not r:
            return False
        try:
            data = os.read(self.fd, 65536)
        except OSError:
            self.dead = True
            return False
        if not data:
            self.dead = True
            return False
        self.raw.extend(data)
        self.stream.feed(data)
        return True

    def settle_out(self, max_wait=10.0, first_wait=6.0, phase="?"):
        """Read until the screen stops changing; record HOW the wait ended.

        QUIET is the only outcome meaning "this screen is final". CAPPED (still
        flowing at the budget) and NO_OUTPUT (nothing ever arrived) are kept so
        a verdict taken under them can be reported as worth less.
        """
        start = last = time.monotonic()
        seen = False
        first = None
        while True:
            now = time.monotonic()
            if now - start > max_wait:
                outcome = CAPPED if seen else NO_OUTPUT
                self.events.append((phase, outcome, first, now - start))
                return outcome
            got = self._read_once(0.05)
            now = time.monotonic()
            if got:
                if first is None:
                    first = now - start
                seen, last = True, now
            elif not seen:
                if now - start > first_wait:
                    self.events.append((phase, NO_OUTPUT, first, now - start))
                    return NO_OUTPUT
            elif now - last >= self.settle:
                self.events.append((phase, QUIET, first, now - start))
                return QUIET

    def wait_prompt(self, timeout=40.0):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            self._read_once(0.05)
            if any(SENTINEL in line for line in self.screen.display):
                return True
            if self.dead:
                return False
        return False

    def send(self, data):
        try:
            os.write(self.fd, data)
        except OSError:
            self.dead = True

    def clear(self):
        self.send(b"\x0c")
        self.settle_out(max_wait=3.0, first_wait=2.0, phase="clear")

    def grid(self):
        rows = [r.rstrip() for r in self.screen.display]
        while rows and rows[-1] == "":
            rows.pop()
        return [self._mask(r) for r in rows]

    def attrs(self, nrows):
        buf = self.screen.buffer
        out = []
        for y in range(min(nrows, self.rows)):
            row = buf[y]
            out.append(tuple(
                (c.fg, c.bg, c.bold, c.italics, c.underscore,
                 c.strikethrough, c.reverse, c.blink)
                for c in (row[x] for x in range(self.cols))
            ))
        return out

    def cursor(self):
        return (self.screen.cursor.y, self.screen.cursor.x)

    def _mask(self, row):
        """Replace THIS child's own pid with a stable token.

        Two live processes cannot share a pid, so a screen that shows `$$`
        could never compare equal however correct zshrs is. Only this session's
        exact pid is substituted — not a general digit mask; any other number
        on the screen still has to match byte for byte.
        """
        return row.replace(str(self.pid), "<PID>") if self.pid else row

    def crashed(self):
        text = self.raw.decode("utf-8", "replace")
        return [m for m in CRASH_MARKERS if m in text]

    def close(self):
        for b in (b"\x03", b"\x04"):
            try:
                os.write(self.fd, b)
            except OSError:
                pass
        try:
            os.close(self.fd)
        except OSError:
            pass
        for sig in (signal.SIGHUP, signal.SIGTERM, signal.SIGKILL):
            try:
                pid, st = os.waitpid(self.pid, os.WNOHANG)
                if pid == self.pid:
                    self.status = st
                    return
                os.kill(self.pid, sig)
            except OSError:
                return
            for _ in range(20):
                try:
                    pid, st = os.waitpid(self.pid, os.WNOHANG)
                    if pid == self.pid:
                        self.status = st
                        return
                except OSError:
                    return
                select.select([], [], [], 0.025)


def exit_note(status):
    if status is None:
        return None
    if os.WIFSIGNALED(status):
        sig = os.WTERMSIG(status)
        if sig in (signal.SIGHUP, signal.SIGTERM, signal.SIGKILL):
            return None                      # our own teardown
        try:
            name = signal.Signals(sig).name
        except ValueError:
            name = "?"
        return "killed by signal %d (%s)" % (sig, name)
    if os.WIFEXITED(status) and os.WEXITSTATUS(status) not in (0, 1):
        return "exited %d" % os.WEXITSTATUS(status)
    return None


def _secs(v):
    return "none" if v is None else "%.2fs" % v


class Capture:
    def __init__(self, grid=None, reason=None, crash=None, attrs=None,
                 cursor=None, raw=b"", diags=None, events=(), status=None):
        self.grid = grid
        self.reason = reason
        self.crash = crash or []
        self.attrs = attrs or []
        self.cursor = cursor
        self.raw = raw
        self.diags = diags or set()
        self.events = list(events)
        self.status = status

    def warnings(self):
        out = []
        for phase, outcome, first, waited in self.events:
            if outcome == CAPPED:
                out.append("settle capped after %s — screen may be mid-render "
                           "(first byte %s, waited %.1fs)"
                           % (phase, _secs(first), waited))
            elif outcome == NO_OUTPUT and phase.startswith("key "):
                out.append("no output at all after %s (waited %.1fs)"
                           % (phase, waited))
        note = exit_note(self.status)
        if note:
            out.append("child %s" % note)
        return out


def capture(argv, env, args, init_file, cwd, buf, keys, cols=None, use_utf8=True):
    sess = Session(argv, env, args.rows, cols or args.cols, args.settle, cwd,
                   use_utf8)
    result = Capture(reason="capture aborted before any screen was taken")
    try:
        sess.settle_out(max_wait=4.0, first_wait=3.0, phase="boot")
        sess.send(("source %s\n" % shlex.quote(init_file)).encode())
        if not sess.wait_prompt(timeout=args.boot_timeout):
            result = Capture(reason="never reached a prompt (compinit hang or crash)",
                             crash=sess.crashed())
        else:
            sess.clear()
            sess.mark = len(sess.raw)
            if buf:
                sess.send(buf.encode())
                sess.settle_out(max_wait=3.0, first_wait=1.0, phase="buffer")
            for k in keys:
                sess.send(key_bytes(k))
                sess.settle_out(max_wait=15.0, first_wait=10.0, phase="key %r" % k)
            sess.settle_out(max_wait=3.0, first_wait=0.6, phase="final")
            rows = sess.grid()
            result = Capture(grid=rows, attrs=sess.attrs(len(rows)),
                             cursor=sess.cursor(),
                             raw=bytes(sess.raw[sess.mark:]),
                             diags=diagnostics(sess.raw[sess.mark:], sess.pid))
            crash = sess.crashed()
            if crash:
                result.reason = "crashed: " + ", ".join(crash)
                result.crash = crash
            elif sess.dead:
                result.reason = "shell exited mid-case"
    except Exception as exc:
        result = Capture(reason="harness error during capture: %r" % (exc,))
    finally:
        sess.close()
        result.events = list(sess.events)
        result.status = sess.status
    return result


# ═════════════════════════════════════════════════════════════════════════════
# comparison
# ═════════════════════════════════════════════════════════════════════════════

def diff_grids(ref, test):
    n = max(len(ref), len(test))
    out = []
    for i in range(n):
        a = ref[i] if i < len(ref) else "<absent>"
        b = test[i] if i < len(test) else "<absent>"
        if a != b:
            out.append((i, a, b))
    return out


def first_diff_cell(a, b):
    for i, (x, y) in enumerate(zip(a, b)):
        if x != y:
            return i
    return min(len(a), len(b))


def attr_diff_rows(ref, test):
    return [i for i in range(min(len(ref), len(test))) if ref[i] != test[i]]


PASS, PASS_ERR, FAIL, SKIP = "PASS", "PASS(err)", "FAIL", "SKIP"

# Failure classes worth reducing, and worth proving the reference is stable for.
# `ref-broken` / `test-broken` are excluded: a shell that never booted or
# crashed is not a property of any one spec atom, so delta-debugging it would
# just blame whichever atom happened to survive.
SHRINKABLE = ("grid-diff", "one-sided-error", "strict", "text-bytes-diff")


class Verdict:
    def __init__(self, case, status, detail, ref, test, diffs, category=""):
        self.case = case
        self.status = status
        self.detail = detail
        self.ref = ref
        self.test = test
        self.diffs = diffs or []
        self.category = category      # coarse failure class, used by the shrinker
        self.stream_only = False
        self.attr_rows = []
        self.cursor_differs = False
        self.ref_only_diags = set()
        self.test_only_diags = set()
        self.duration = 0.0
        self.fixture = None
        # [(construct, "core-language"|"completion"|"rejected", ref, test, err)]
        self.attribution = []
        # Set only when a SECOND reference zsh was available and this case
        # failed. Purely additive: the status above is decided against
        # `--zsh` alone and is never rewritten by what the dev reference did.
        self.dev = None
        self.split_class = ""
        self.split_side = ""
        self.split_note = ""
        # True only when the dev reference never reached a prompt, which is the
        # one shape in which this failure has NO classification. Kept apart
        # from `split_note`, which carries a secondary observation about a
        # failure that IS classified.
        self.split_unrun = False
        if ref and test and ref.grid is not None and test.grid is not None:
            text_diff = {i for i, _a, _b in self.diffs}
            self.attr_rows = [r for r in attr_diff_rows(ref.attrs, test.attrs)
                              if r not in text_diff]
            self.cursor_differs = ref.cursor != test.cursor
            self.ref_only_diags = ref.diags - test.diags
            self.test_only_diags = test.diags - ref.diags
            # "matched on screen, differs in the stream" is a SOFTER report
            # than `text-bytes-diff`, which already failed the case on the
            # painted bytes. Counting a case in both would list the same
            # divergence twice under two names.
            self.stream_only = ((not self.diffs) and ref.raw != test.raw
                                and category != "text-bytes-diff")


def render(rows):
    return "\n".join("  %2d| %s" % (i, r) for i, r in enumerate(rows)) or "  <empty>"


def judge(args, case, ref, test):
    """Returns (status, detail, diffs, category).

    Ordering matters. A shell that never booted or crashed is a FAIL naming
    that, never a quiet skip. `PASS(err)` is reserved for the case where the
    two shells rendered the IDENTICAL screen and that screen contains a compsys
    diagnostic — the generated spec was malformed and both shells agreed about
    it, which is a real (if weak) parity observation and is counted apart from
    a clean pass so a run cannot look green on nothing but garbage specs.
    """
    if ref.reason:
        return FAIL, "reference zsh: %s" % ref.reason, None, "ref-broken"
    if test.reason:
        return FAIL, "zshrs: %s" % test.reason, None, "test-broken"
    d = diff_grids(ref.grid, test.grid)
    if d:
        cat = "grid-diff"
        if bool(ref.diags) != bool(test.diags):
            cat = "one-sided-error"
        return FAIL, "%d row(s) differ" % len(d), d, cat
    # Grids match. That is not yet proof the two shells wrote the same TEXT:
    # the decoder folds every malformed byte sequence onto U+FFFD, and in a
    # byte-per-cell locale it folds nothing but still cannot tell a width
    # miscount from a correct one if the padding happens to land. Compare the
    # painted bytes directly. This is a FAIL and not a soft report, because
    # unlike an escape-sequence difference it is a difference in CONTENT.
    pa, pb = printable_bytes(ref.raw), printable_bytes(test.raw)
    if pa != pb:
        i = first_diff_cell(pa, pb)
        return (FAIL,
                "grids match but the painted BYTES differ at offset %d "
                "(zsh %r vs zshrs %r)" % (i, pa[max(0, i - 12):i + 12],
                                          pb[max(0, i - 12):i + 12]),
                [], "text-bytes-diff")
    extra = []
    if args.compare_attrs:
        rows = attr_diff_rows(ref.attrs, test.attrs)
        if rows:
            extra.append("%d row(s) differ in SGR attributes only" % len(rows))
    if args.strict_cursor and ref.cursor != test.cursor:
        extra.append("cursor %s vs %s" % (ref.cursor, test.cursor))
    if args.strict_stream and ref.raw != test.raw:
        extra.append("raw escape streams differ")
    if extra:
        return FAIL, "; ".join(extra), [], "strict"
    if ref.diags:
        return PASS_ERR, "both shells report: %s" % "; ".join(sorted(ref.diags))[:160], [], ""
    return PASS, "", [], ""


def run_case(args, root, fpath_dirs, case):
    """Drive one case on both shells and judge it."""
    t0 = time.monotonic()
    # A locale the reference shell does not honour means the two shells were
    # never put in the same environment, so there is nothing to compare. That
    # is a counted SKIP with a named reason — never a pass, and never silently
    # rewritten to a locale that does work.
    if case.locale not in args.locale_ok:
        v = Verdict(case, SKIP, "locale %s is not available on this host "
                    "(%s)" % (case.locale,
                              args.locale_why.get(case.locale, "not probed")),
                    None, None, [], "locale-unavailable")
        v.duration = time.monotonic() - t0
        return v
    cdir, init = build_case_dir(root, case, fpath_dirs)
    env = child_env(cdir, case.locale)
    work = os.path.join(cdir, "work")
    utf8 = args.locale_ok.get(case.locale, {}).get("codec") == "utf-8"
    ref = capture([args.zsh, "-f", "-i"], env, args, init, work, case.buffer,
                  case.keys, case.cols, utf8)
    test = capture(args.test_argv, env, args, init, work, case.buffer,
                   case.keys, case.cols, utf8)
    status, detail, diffs, cat = judge(args, case, ref, test)

    v = Verdict(case, status, detail, ref, test, diffs, cat)

    # A failure is only meaningful if the REFERENCE is stable. Prove it rather
    # than assume it: re-run zsh against itself. Disagreeing with itself is the
    # one and only condition under which a case is skipped, and the second
    # reference capture is printed so the claim is checkable.
    if status == FAIL and args.self_check and cat in SHRINKABLE:
        ref2 = capture([args.zsh, "-f", "-i"], env, args, init, work,
                       case.buffer, case.keys, case.cols, utf8)
        why = None
        if ref2.reason:
            why = ref2.reason
        elif ref.grid != ref2.grid:
            why = ("%d row(s) differ between the two zsh runs"
                   % len(diff_grids(ref.grid, ref2.grid)))
        elif cat == "text-bytes-diff" and (printable_bytes(ref.raw)
                                           != printable_bytes(ref2.raw)):
            # The byte-level verdict needs a byte-level stability proof: a
            # reference whose GRID repeats but whose painted bytes do not is
            # not a reference for a painted-bytes claim.
            why = "the two zsh runs painted different bytes"
        if why:
            v.status = SKIP
            v.category = "unstable-reference"
            v.detail = ("real zsh disagreed with itself on a re-run "
                        "(%s) — nothing to compare against" % why)
            v.ref2 = ref2

    # A THREE-way label on a surviving failure, when a second reference zsh is
    # available. This is ADDITIVE and nothing above it moves: the PASS/FAIL
    # verdict stays measured against `--zsh`, and a `version-split` label never
    # turns a FAIL into a pass. What it adds is the one thing the single-
    # reference comparison cannot say — whether the two references AGREE, which
    # is the difference between "zshrs is wrong" and "there are two right
    # answers and zshrs picked one".
    if (v.status == FAIL and getattr(args, "zsh_dev", None)
            and v.category in SHRINKABLE):
        dev = capture([args.zsh_dev, "-f", "-i"], env, args, init, work,
                      case.buffer, case.keys, case.cols, utf8)
        v.dev = dev
        if dev.reason:
            v.split_unrun = True
            v.split_note = ("dev reference did not run this case (%s), so the "
                            "failure could not be classified" % dev.reason)
        else:
            # The comparison has to be made on the SAME CRITERION that produced
            # this case's verdict, and on nothing else.
            #
            # `judge` combines several criteria, and using its combined answer
            # here is WRONG in the one direction that matters: with
            # --strict-stream on, the two references differ in the raw escape
            # stream on essentially every case (that is the confirmed
            # cc672f1c3b split), so a combined comparison would report
            # "references disagree" for a case that actually failed on a
            # missing word in row 0 — and would file a real zshrs bug as a
            # version question. Measured on the case that caught it: both
            # references rendered `@SFZ@ fzc0001 delta exact=`, zshrs rendered
            # `@SFZ@ fzc0001 delta`, and the combined comparison still said
            # "split".
            #
            # So: a screen failure is classified by comparing SCREENS, a
            # painted-bytes failure by comparing painted bytes, and a
            # --strict-* failure by the full judge. The other observation is
            # not discarded — it is printed as a note.
            def screen_differs(a, b):
                return (bool(diff_grids(a.grid, b.grid))
                        or printable_bytes(a.raw) != printable_bytes(b.raw)
                        or bool(a.diags) != bool(b.diags))

            def strictly_differs(a, b):
                return judge(args, case, a, b)[0] not in (PASS, PASS_ERR)

            differs = (strictly_differs if v.category == "strict"
                       else screen_differs)
            if differs(ref, dev):
                v.split_class = SPLIT
                # `test` already differs from `ref` — that is why this is a
                # FAIL — so the only open question is whether it matches dev.
                v.split_side = (SIDE_DEV if not differs(dev, test)
                                else SIDE_NEITHER)
            else:
                v.split_class = ZSHRS_BUG
            # The secondary observation, always reported: two references that
            # painted the same screen can still have written different bytes to
            # get there, and that difference is a version split of its own even
            # though it is not what failed this case.
            if v.category != "strict" and ref.raw != dev.raw:
                v.split_note = ("the two references painted the SAME screen "
                                "but wrote different escape streams — a split "
                                "that is not what failed this case")
    v.duration = time.monotonic() - t0
    return v


# ═════════════════════════════════════════════════════════════════════════════
# shrinking
# ═════════════════════════════════════════════════════════════════════════════

def ddmin(units, still_bad, max_probes):
    """Classic delta-debugging minimisation over a list of units.

    `still_bad(subset)` must return True when the subset still reproduces. The
    probe budget is a hard cap: each probe is two full pty sessions.
    """
    probes = [0]

    def test(sub):
        if probes[0] >= max_probes:
            raise StopIteration
        probes[0] += 1
        return still_bad(sub)

    cur = list(units)
    n = 2
    try:
        while len(cur) >= 2:
            chunk = max(1, len(cur) // n)
            parts = [cur[i:i + chunk] for i in range(0, len(cur), chunk)]
            reduced = False
            for i, p in enumerate(parts):
                comp = [u for j, q in enumerate(parts) if j != i for u in q]
                if comp and test(comp):
                    cur, n, reduced = comp, max(n - 1, 2), True
                    break
            if not reduced:
                if n >= len(cur):
                    break
                n = min(len(cur), n * 2)
    except StopIteration:
        pass
    return cur, probes[0]


def shrink_case(args, root, fpath_dirs, case, category):
    """Reduce a diverging case to a minimal set of spec atoms and flags.

    Units are the individual `_arguments` specs / `_values` values / `_describe`
    entries plus each harness-level flag, so the reproducer that comes out is a
    sentence about ONE construct rather than a wall of six.

    The reduction must keep diverging for the SAME coarse reason; a subset that
    merely becomes malformed and errors on one side is not accepted as a
    reduction of a rendering divergence.
    """
    units = ([("atom", i) for i in range(len(case.atoms))] +
             [("flag", i) for i in range(len(case.flags))])

    def build(sub):
        keep_a = sorted(i for k, i in sub if k == "atom")
        keep_f = sorted(i for k, i in sub if k == "flag")
        c = case.clone([case.atoms[i] for i in keep_a],
                       [case.flags[i] for i in keep_f])
        c.idx = case.idx
        return c

    def still_bad(sub):
        c = build(sub)
        # Shrink probes run in their own directory so the original case's
        # materialised fixture is never overwritten mid-reduction.
        v = run_case(args, os.path.join(root, "shrink"), fpath_dirs, c)
        return v.status == FAIL and v.category == category

    minimal, probes = ddmin(units, still_bad, args.shrink_probes)
    return build(minimal), probes


# ═════════════════════════════════════════════════════════════════════════════
# fixtures (write / replay)
# ═════════════════════════════════════════════════════════════════════════════

HEREDOC = "SPEC_FUZZ_COMPLETER"


def write_fixture(path, args, case, note):
    """A runnable .zsh reproducer.

    Running it materialises the same throwaway fpath and drops into an
    interactive shell with the completer loaded; `--replay`ing it re-runs the
    exact comparison the harness ran. The header carries the replay command so
    the file is self-describing when it turns up in `target/` weeks later.
    """
    os.makedirs(os.path.dirname(path), exist_ok=True)
    rel = os.path.relpath(path, REPO)
    body = case.completer()
    used = case.extra.get("hostile_used") or []
    with _wopen(path) as f:
        f.write(
            "#!/usr/bin/env zsh\n"
            "# compsys_spec_fuzz minimal reproducer — %s\n"
            "# %s\n"
            "#\n"
            "# replay (authoritative — re-runs the exact parity comparison,\n"
            "# in the same locale and at the same terminal width):\n"
            "#     %s --replay %s\n"
            "# manual (drops you in a shell with this completer loaded):\n"
            "#     LC_ALL=%s zsh %s   # then type:  %s   and press: %s\n"
            "#\n"
            "# @seed %d\n"
            "# @case %d\n"
            "# @cmd %s\n"
            "# @kind %s\n"
            "# @widget %s\n"
            "# @locale %s\n"
            "# @cols %d\n"
            "# @hostile %s\n"
            "# @buffer %s\n"
            "# @keys %s\n"
            "# @shell %s\n"
            % (case.name, note, SELF, rel, case.locale, rel,
               disp(case.buffer), ",".join(case.keys),
               case.seed, case.idx, case.cmd, case.kind,
               case.extra.get("widget", "default"), case.locale, case.cols,
               " ".join("%s(%s)" % (n, c) for n, c in used) or "-",
               case.buffer, ",".join(case.keys), " ".join(args.test_argv)))
        # For a substrate case: which language construct each surviving atom is,
        # the stock function that justifies it, and the standalone command that
        # runs it with no completion around it at all. Informational — `@setup`
        # and the heredoc below are what `--replay` reconstructs from — but it
        # is the part a human reads first when the file turns up later.
        for a in case.atoms:
            c = SUBSTRATE_BY_ID.get(a)
            if c is not None:
                f.write("# @construct %s  (%s)  %s\n"
                        % (c.id, c.category, c.cite))
                f.write("# @repro %s\n" % substrate_repro_cmd(c))
        # One header line per init line, so `--replay` reconstructs the exact
        # shell state the divergence needed (the widget declaration and its
        # binding, the widget's own function, menu selection, styles, modules).
        for ln in case.init_lines():
            f.write("# @setup %s\n" % ln)
        # `_d` is EXPORTED: the generated .zshrc keeps `$_d` literal (the
        # heredoc below is quoted, so a `$` in a widget declaration or in a
        # generated function body reaches the file unmangled) and the rc is
        # read by a fresh `zsh -i`, which only sees `_d` if it is in the
        # environment.
        f.write("\nemulate -L zsh\ntypeset -x _d=${TMPDIR:-/tmp}/spec-fuzz-repro.$$\n"
                "mkdir -p $_d/fpath\n")
        f.write("cat >$_d/fpath/_%s <<'%s'\n%s%s\n" % (case.cmd, HEREDOC, body, HEREDOC))
        f.write(
            "cat >$_d/.zshrc <<'RC'\n"
            "fpath=( $_d/fpath %s )\n"
            "PROMPT='%s '\n"
            "autoload -Uz compinit\n"
            "compinit -u -D\n"
            "%s"
            "RC\n"
            % (" ".join(shlex.quote(p) for p in args.fpath_dirs), SENTINEL,
               "".join(ln + "\n" for ln in case.init_lines())))
        f.write("print -r -- \"# fpath dir: $_d/fpath   buffer: %s\"\n"
                % case.buffer.replace('"', '\\"'))
        # The manual reproducer has to reproduce the ENVIRONMENT too: this
        # divergence may exist only at this locale and this width, and a shell
        # started in the caller's own locale would not show it.
        f.write("ZDOTDIR=$_d LANG=%s LC_ALL=%s exec "
                "${SPEC_FUZZ_SHELL:-zsh} -i\n" % (case.locale, case.locale))
    os.chmod(path, 0o755)
    return path


def read_fixture(path):
    meta = {}
    multi = {}
    # Byte-safe, symmetrically with _wopen: a saved divergence can be ABOUT a
    # byte that is not valid UTF-8, and reading it back with the default
    # decoder would either raise or silently replace the thing under test.
    with open(path, encoding="utf-8", errors="surrogateescape") as fh:
        lines = fh.read().splitlines()
    for ln in lines:
        m = re.match(r"^#\s*@(\w+)\s+(.*)$", ln)
        if m:
            meta.setdefault(m.group(1), m.group(2))
            multi.setdefault(m.group(1), []).append(m.group(2))
    try:
        start = lines.index("cat >$_d/fpath/_%s <<'%s'" % (meta["cmd"], HEREDOC)) + 1
    except (KeyError, ValueError):
        sys.exit("compsys_spec_fuzz: %s is not a fixture written by this harness "
                 "(no @cmd header or no completer heredoc)" % path)
    end = lines.index(HEREDOC, start)
    body = "\n".join(lines[start:end]) + "\n"
    # `setup_verbatim` — a replayed fixture's init is taken exactly as written,
    # never rebuilt: the widget declaration, its function and the module loads
    # are all already in the `@setup` headers, and re-deriving them from a
    # widget id would silently drift from the file the divergence was saved in.
    case = Case(int(meta.get("case", -1)), int(meta.get("seed", 0)), meta["cmd"],
                meta.get("kind", "replay"), [], [],
                {"setup_verbatim": multi.get("setup", []),
                 "widget": meta.get("widget", "default")},
                meta["buffer"], meta["keys"].split(","),
                # A fixture written before the locale axis existed carries
                # neither header; it was recorded under the old fixed
                # environment, so that is what it replays in.
                locale=meta.get("locale", "C"),
                cols=int(meta.get("cols", 80)))
    case.body_override = body
    return case


# ═════════════════════════════════════════════════════════════════════════════
# locale availability probe
# ═════════════════════════════════════════════════════════════════════════════
#
# A locale name in a table is not a locale on this host, and a locale on this
# host is not a locale the shell honours. Both have to be PROVED before a case
# may be generated for one, because a case that ran in a locale the reference
# shell silently fell back out of is not a comparison of anything: both shells
# would land in `C` and the run would look green while testing the axis it
# claims to have added.
#
# The proof is the shell's own arithmetic on a known byte string. `${#s}` is
# characters and `${(m)#s}` is display columns; the two together identify the
# family without this harness having to hard-code anyone's charmap.

def _locale_probe_script(canary_literal):
    return ("s=%s\nprint -r -- \"@@L ${#s} ${(m)#s}\"\n" % canary_literal)


def probe_locales(args, candidates):
    """-> ({id: info}, [(id, family, why-skipped)]) — availability, PROVED.

    `info` carries the codec pyte is given and the numbers the shell reported,
    so the printed table is evidence and not an assertion.
    """
    root = os.path.join(REPO, "target", "spec-fuzz-%d" % args.seed, "locale-check")
    os.makedirs(root, exist_ok=True)
    script = os.path.join(root, "canary.zsh")
    with _wopen(script) as f:
        f.write(_locale_probe_script("'%s'" % bdec(LOCALE_CANARY)))

    have = set()
    try:
        out = subprocess.run(["locale", "-a"], capture_output=True, text=True,
                             timeout=20).stdout
        have = {ln.strip() for ln in out.splitlines() if ln.strip()}
    except (OSError, subprocess.SubprocessError):
        have = set()          # no `locale -a`: fall through to the shell probe

    ok, skipped = {}, []
    for loc, family, codec, why in candidates:
        if have and loc not in have and loc != "C":
            skipped.append((loc, family, "not in `locale -a` on this host"))
            continue
        env = {"LANG": loc, "LC_ALL": loc, "PATH": "/usr/bin:/bin"}
        try:
            p = subprocess.run([args.zsh, "-f", script], capture_output=True,
                               text=True, timeout=20, env=env)
        except (OSError, subprocess.SubprocessError) as exc:
            skipped.append((loc, family, "reference shell failed: %r" % (exc,)))
            continue
        m = re.search(r"@@L (\d+) (\d+)", p.stdout)
        if not m:
            skipped.append((loc, family, "reference shell printed no probe line "
                                         "(%r)" % (p.stdout + p.stderr)[:70]))
            continue
        chars, cols = int(m.group(1)), int(m.group(2))
        if not LOCALE_EXPECT[family](chars, cols):
            # The name resolved but the behaviour is not the family's: almost
            # always a silent fallback to C. Naming it is the whole point —
            # a run that quietly tested `C` three times would be a lie.
            skipped.append((loc, family,
                            "reference shell reports %d char/%d col for the "
                            "canary — not %s behaviour (fell back?)"
                            % (chars, cols, family)))
            continue
        ok[loc] = {"family": family, "codec": codec, "why": why,
                   "chars": chars, "cols": cols}
    return ok, skipped


def probe_locales_test_side(args, locales):
    """The same canary on zshrs. Reported, NEVER used to prune."""
    root = os.path.join(REPO, "target", "spec-fuzz-%d" % args.seed, "locale-check")
    script = os.path.join(root, "canary.zsh")
    out = {}
    for loc in locales:
        env = {"LANG": loc, "LC_ALL": loc, "PATH": "/usr/bin:/bin",
               "ZSHRS_NATIVE_ZLE_FX": "0"}
        try:
            p = subprocess.run(args.test_base + ["-f", script],
                               capture_output=True, text=True, timeout=30, env=env)
        except (OSError, subprocess.SubprocessError) as exc:
            out[loc] = (None, None, "failed: %r" % (exc,))
            continue
        m = re.search(r"@@L (\d+) (\d+)", p.stdout)
        if not m:
            out[loc] = (None, None, (p.stdout + p.stderr).strip()[:90] or "no output")
        else:
            out[loc] = (int(m.group(1)), int(m.group(2)), "")
    return out


def print_locale_check(ok, skipped, test_side, want):
    """-> ids where zshrs disagreed with zsh about the canary."""
    print("# locale availability — PROVED on the reference shell, not assumed")
    print("#   %-18s %-7s %-9s %-14s %s"
          % ("locale", "family", "pyte", "zsh char/col", "zshrs char/col"))
    bad = []
    for loc in want:
        if loc not in ok:
            continue
        i = ok[loc]
        t = test_side.get(loc, (None, None, "not probed"))
        tn = ("%d/%d" % (t[0], t[1])) if t[0] is not None else ("ERR " + t[2][:40])
        mark = ""
        if t[0] is not None and (t[0], t[1]) != (i["chars"], i["cols"]):
            mark, _ = "  <-- DIVERGES", bad.append(loc)
        elif t[0] is None:
            mark, _ = "  <-- DIVERGES", bad.append(loc)
        print("#   %-18s %-7s %-9s %-14s %s%s"
              % (loc, i["family"], i["codec"],
                 "%d/%d" % (i["chars"], i["cols"]), tn, mark))
    if skipped:
        print("#   skipped (named, never counted as tested):")
        for loc, family, why in skipped:
            print("#     %-18s %-7s %s" % (loc, family, why))
    print("# %d locale(s) usable, %d skipped, %d where zshrs read the canary "
          "differently from zsh" % (len(ok), len(skipped), len(bad)))
    sys.stdout.flush()
    return bad


# ═════════════════════════════════════════════════════════════════════════════
# generated-text self-check
# ═════════════════════════════════════════════════════════════════════════════
#
# A string real zsh cannot represent in the chosen locale is a GENERATOR bug,
# not a finding: every case drawn with it would diverge for a reason that says
# nothing about zshrs. So each hostile entry is written into a file, read back
# through the reference shell's OWN lexer — the same path a generated completer
# takes — and required to come out byte for byte identical before it may be
# generated for that locale. Rejections are counted by category and printed.
#
# The identical probe runs on zshrs, and that side is never used to prune. An
# entry zsh represents and zshrs does not is a divergence and is reported.

_TEXT_RE = re.compile(rb"@@T (\S+) (\d+) (\d+) (.*)")


def _text_probe_script(entries):
    """One script that echoes every entry's characters, columns and bytes."""
    out = []
    for h in entries:
        out.append("s=%s" % zq(h.text))
        out.append("print -r -- \"@@T %s ${#s} ${(m)#s} $s\"" % h.name)
    return "\n".join(out) + "\n"


def _run_text_probe(argv, env, script_path, _entries, timeout):
    """-> {name: (chars, cols, bytes)} or ({}, error) if the shell refused."""
    try:
        p = subprocess.run(list(argv) + ["-f", script_path],
                           stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                           timeout=timeout, env=env)
    except (OSError, subprocess.SubprocessError) as exc:
        return {}, "could not run: %r" % (exc,)
    got = {}
    for line in p.stdout.split(b"\n"):
        m = _TEXT_RE.match(line)
        if m:
            got[m.group(1).decode()] = (int(m.group(2)), int(m.group(3)),
                                        m.group(4))
    if not got:
        err = (p.stderr or p.stdout).decode("utf-8", "replace").strip()
        # The script path is long and repeats twice in a zshrs diagnostic; it
        # would otherwise fill the whole quoted message and hide the reason.
        err = err.replace(script_path, "<probe>")
        err = re.sub(r"\s+", " ", err)
        return {}, err[:150] or "no probe line and no message"
    return got, None


def check_generated_text(args, locales, entries):
    """-> (pool, report). `pool` is {locale: [Hostile]} — proved-representable.

    `report` is {locale: {"ok": n, "rejected": [(name, cat, why)],
                          "test_diff": [(name, cat, why)]}}.
    """
    root = os.path.join(REPO, "target", "spec-fuzz-%d" % args.seed, "text-check")
    os.makedirs(root, exist_ok=True)

    # Entries whose bytes ARE valid UTF-8 share one script. The invalid ones
    # each get their own, because a shell that refuses to read a file with a
    # malformed byte in it fails the WHOLE file — batching them would hide
    # every other entry behind the first one, and hiding is what this check
    # exists to prevent.
    batches = [("valid", [h for h in entries if h.valid_utf8])]
    batches += [(h.name, [h]) for h in entries if not h.valid_utf8]
    paths = {}
    for label, group in batches:
        if not group:
            continue
        p = os.path.join(root, "probe-%s.zsh" % re.sub(r"\W", "_", label))
        with _wopen(p) as f:
            f.write(_text_probe_script(group))
        paths[label] = (p, group)

    pool, report = {}, {}
    for loc in locales:
        env = {"LANG": loc, "LC_ALL": loc, "PATH": "/usr/bin:/bin"}
        tenv = dict(env, ZSHRS_NATIVE_ZLE_FX="0")
        good, rejected, test_diff = [], [], []
        for label, (path, group) in paths.items():
            ref, rerr = _run_text_probe([args.zsh], env, path, group,
                                        args.text_check_timeout)
            test, terr = _run_text_probe(args.test_base, tenv, path, group,
                                         args.text_check_timeout)
            for h in group:
                if rerr or h.name not in ref:
                    rejected.append((h.name, h.category,
                                     "reference shell could not represent it "
                                     "(%s)" % (rerr or "no probe line")))
                    continue
                chars, cols, raw = ref[h.name]
                if raw != h.raw:
                    rejected.append((h.name, h.category,
                                     "reference shell round-tripped %r, not %r"
                                     % (raw[:24], h.raw[:24])))
                    continue
                good.append(h)
                if terr or h.name not in test:
                    test_diff.append((h.name, h.category,
                                      "zshrs could not represent it (%s)"
                                      % (terr or "no probe line")))
                elif test[h.name] != (chars, cols, raw):
                    tc, tw, traw = test[h.name]
                    test_diff.append(
                        (h.name, h.category,
                         "zsh %d char/%d col %r vs zshrs %d char/%d col %r"
                         % (chars, cols, raw[:20], tc, tw, traw[:20])))
        pool[loc] = good
        report[loc] = {"ok": len(good), "rejected": rejected,
                       "test_diff": test_diff}
    return pool, report


def print_text_check(report, entries):
    """-> total generator rejections. Prints the per-category breakdown."""
    n = len(entries)
    print("# generated-text self-check — every hostile string proved to "
          "round-trip through the reference shell's own lexer")
    print("#   %-18s %-9s %-10s %s"
          % ("locale", "generated", "rejected", "zsh-vs-zshrs divergences"))
    total_rej = 0
    for loc in sorted(report):
        r = report[loc]
        total_rej += len(r["rejected"])
        print("#   %-18s %-9s %-10s %s"
              % (loc, "%d" % n, "%d" % len(r["rejected"]), len(r["test_diff"])))
    for loc in sorted(report):
        r = report[loc]
        if r["rejected"]:
            byc = {}
            for name, cat, why in r["rejected"]:
                byc.setdefault(cat, []).append(name)
            print("#   %s — GENERATOR-rejected (dropped from that locale's "
                  "pool; a string zsh cannot represent is a generator bug, "
                  "not a finding):" % loc)
            for cat in sorted(byc):
                print("#     %-14s %d  (%s)" % (cat, len(byc[cat]),
                                                " ".join(byc[cat])))
            for name, cat, why in r["rejected"][:4]:
                print("#       %-14s %s" % (name, why[:160]))
        if r["test_diff"]:
            byc = {}
            for name, cat, why in r["test_diff"]:
                byc.setdefault(cat, []).append(name)
            print("#   %s — TEXT DIVERGENCE (kept in the pool; zsh represented "
                  "these and zshrs did not agree):" % loc)
            for cat in sorted(byc):
                print("#     %-14s %d  (%s)" % (cat, len(byc[cat]),
                                                " ".join(byc[cat])))
            for name, cat, why in r["test_diff"][:6]:
                print("#       %-14s %s" % (name, why[:170]))
    print("# %d string(s) x %d locale(s): %d generator-rejected"
          % (n, len(report), total_rej))
    sys.stdout.flush()
    return total_rej


# ═════════════════════════════════════════════════════════════════════════════
# substrate self-check + standalone attribution
# ═════════════════════════════════════════════════════════════════════════════
#
# Two jobs, one mechanism.
#
# The SELF-CHECK is the same contract every other axis in this file carries: a
# construct real zsh REJECTS is a generator bug, not a finding, so every
# fragment is run on the reference shell before a case is generated and proved
# to produce its probe line with nothing on stderr. Rejections are counted in
# their own named category and printed; they never reach a case.
#
# The ATTRIBUTION is the deliverable. The identical fragment is also run on
# zshrs, outside any completion, and the two outputs compared. That answers the
# question a screen diff cannot:
#
#   the standalone outputs already differ  -> CORE-LANGUAGE bug. The printed
#                                             `zsh -f -c '...'` line IS the
#                                             reproducer, of the kind
#                                             `f(){ argv+=(X); print -l "$@" }`
#                                             that made the last round's
#                                             largest failure fixable.
#   the standalone outputs are identical
#   and the in-completion case diverged    -> COMPLETION bug. The language is
#                                             fine; the construct only breaks
#                                             once the completion system is the
#                                             one running it, and the fixture is
#                                             the reproducer.
#
# The zshrs side is NEVER used to prune. A construct zsh accepts and zshrs does
# not is a divergence and is reported as one.

_SUB_RE = re.compile(rb"^@@S (\S+)(?: (.*))?$")


def _run_substrate(argv, env, path, cwd, timeout):
    """-> (value-bytes or None, error-text or None)."""
    try:
        p = subprocess.run(list(argv) + ["-f", path],
                           stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                           timeout=timeout, env=env, cwd=cwd)
    except (OSError, subprocess.SubprocessError) as exc:
        return None, "could not run: %r" % (exc,)
    val = None
    for line in p.stdout.split(b"\n"):
        m = _SUB_RE.match(line.rstrip(b"\r"))
        if m:
            val = m.group(2) or b""
    err = (p.stderr or b"").decode("utf-8", "replace").strip()
    err = re.sub(r"\s+", " ", err.replace(path, "<probe>"))
    if val is None:
        return None, (err or "no @@S line and no message")[:180]
    return val, (err[:180] or None)


def substrate_probe(args, root, cwd, locale, constructs):
    """Run every fragment standalone on BOTH shells, in one locale.

    -> {id: {"ref": bytes|None, "ref_err": str|None,
             "test": bytes|None, "test_err": str|None}}
    """
    os.makedirs(root, exist_ok=True)
    env = child_env(cwd, locale)
    out = {}
    for c in constructs:
        p = os.path.join(root, "sub-%s.zsh" % re.sub(r"\W", "_", c.id))
        with _wopen(p) as f:
            f.write(c.script())
        rv, re_ = _run_substrate([args.zsh], env, p, cwd, args.substrate_timeout)
        tv, te_ = _run_substrate(args.test_base, env, p, cwd,
                                 args.substrate_timeout)
        out[c.id] = {"ref": rv, "ref_err": re_, "test": tv, "test_err": te_,
                     "script": p}
    return out


def check_substrate(args, constructs):
    """-> (pool, report). `pool` is the constructs real zsh proved it accepts.

    `report` is {locale: {id: probe}} for every locale in the pool, because a
    fragment can be locale-sensitive (one of them globs) and a check run in one
    locale would not be a check of the environment the cases run in.
    """
    root = os.path.join(REPO, "target", "spec-fuzz-%d" % args.seed,
                        "substrate-check")
    work = os.path.join(root, "work")
    os.makedirs(work, exist_ok=True)
    for f in WORK_FILES:
        p = os.path.join(work, f)
        if not os.path.exists(p):
            open(p, "w").close()
    for sub in WORK_DIRS:
        os.makedirs(os.path.join(work, sub), exist_ok=True)

    report = {}
    for loc in args.locale_pool:
        report[loc] = substrate_probe(args, root, work, loc, constructs)
    # A fragment the reference shell refused, or complained about, in ANY
    # locale is out of the pool everywhere: generating it in the locales where
    # it happens to work would make the catalogue's coverage depend silently on
    # which locales this host has.
    bad = set()
    for loc, probes in report.items():
        for cid, pr in probes.items():
            if pr["ref"] is None or pr["ref_err"]:
                bad.add(cid)
    pool = [c for c in constructs if c.id not in bad]
    return pool, report


def print_substrate_check(report, constructs):
    """-> (generator-rejected ids, core-language-divergent ids). Prints both."""
    ids = [c.id for c in constructs]
    bycat = {}
    for c in constructs:
        bycat.setdefault(c.category, []).append(c.id)
    print("# substrate self-check — every construct's fragment run STANDALONE "
          "on both shells")
    print("#   catalogue: %d construct(s) in %d categor(ies) — %s"
          % (len(ids), len(bycat),
             "  ".join("%s:%d" % (k, len(bycat[k])) for k in SUBSTRATE_CATEGORIES
                       if k in bycat)))

    rejected, diverged = {}, {}
    for loc in sorted(report):
        for cid in ids:
            pr = report[loc].get(cid)
            if pr is None:
                continue
            if pr["ref"] is None or pr["ref_err"]:
                rejected.setdefault(cid, []).append(
                    (loc, pr["ref_err"] or "no probe line"))
            elif pr["test"] is None or pr["test"] != pr["ref"]:
                diverged.setdefault(cid, []).append(
                    (loc, pr["ref"], pr["test"], pr["test_err"]))

    print("#   %-22s %-9s %-10s %s"
          % ("locale", "checked", "rejected", "zsh-vs-zshrs divergences"))
    for loc in sorted(report):
        nrej = sum(1 for cid in ids
                   if report[loc][cid]["ref"] is None or report[loc][cid]["ref_err"])
        ndiv = sum(1 for cid in ids
                   if not (report[loc][cid]["ref"] is None or report[loc][cid]["ref_err"])
                   and report[loc][cid]["test"] != report[loc][cid]["ref"])
        print("#   %-22s %-9d %-10d %d" % (loc, len(ids), nrej, ndiv))

    if rejected:
        print("#   GENERATOR-rejected (dropped from the pool; a fragment real "
              "zsh refuses is a generator bug, not a finding):")
        for cid in ids:
            if cid in rejected:
                loc, why = rejected[cid][0]
                print("#     %-24s %-14s %s" % (cid, loc, why[:120]))
    if diverged:
        print("#   CORE-LANGUAGE DIVERGENCE — the construct differs OUTSIDE "
              "completion, so this is a shell bug and not a completion bug:")
        for cid in ids:
            if cid not in diverged:
                continue
            c = SUBSTRATE_BY_ID[cid]
            loc, rv, tv, terr = diverged[cid][0]
            print("#     %-24s %-9s %s" % (cid, c.category, c.cite))
            print("#       zsh   : %s" % disp(bdec(rv))[:150])
            print("#       zshrs : %s"
                  % (disp(bdec(tv))[:150] if tv is not None
                     else "<no probe line: %s>" % (terr or "?")))
            print("#       repro : %s" % substrate_repro_cmd(c))
    print("# %d construct(s): %d generator-rejected, %d core-language divergent"
          % (len(ids), len(rejected), len(diverged)))
    sys.stdout.flush()
    return sorted(rejected), sorted(diverged)


def substrate_repro_cmd(c, shell="zsh"):
    """The standalone reproducer, as a command line that can be pasted."""
    body = "; ".join(ln.strip() for ln in c.lines(0) if ln.strip()
                     and not ln.strip().startswith("#"))
    return "%s -f -c %s" % (shell, shlex.quote(
        "%s; print -r -- \"%s=$_sfz_r\"" % (body, c.id)))


def substrate_attribute(args, root, case):
    """Standalone-diff every construct a substrate case still contains.

    Run in the case's OWN work directory and locale, so the comparison happens
    in the environment the divergence happened in — one of the fragments globs,
    and a probe run somewhere else would not be the same probe.

    -> [(construct, verdict, ref-bytes, test-bytes, err)] with verdict one of
       "core-language" (already differs outside completion),
       "completion"    (identical outside completion),
       "rejected"      (the reference shell refused it here).
    """
    cdir = os.path.join(root, case.name)
    work = os.path.join(cdir, "work")
    if not os.path.isdir(work):
        return []
    cs = [SUBSTRATE_BY_ID[a] for a in case.atoms if a in SUBSTRATE_BY_ID]
    if not cs:
        return []
    probes = substrate_probe(args, os.path.join(cdir, "standalone"), work,
                             case.locale, cs)
    out = []
    for c in cs:
        pr = probes[c.id]
        if pr["ref"] is None or pr["ref_err"]:
            out.append((c, "rejected", pr["ref"], pr["test"], pr["ref_err"]))
        elif pr["test"] is None or pr["test"] != pr["ref"]:
            out.append((c, "core-language", pr["ref"], pr["test"],
                        pr["test_err"]))
        else:
            out.append((c, "completion", pr["ref"], pr["test"], None))
    return out


# ═════════════════════════════════════════════════════════════════════════════
# version splits — a THREE-way comparison against two reference zshs
# ═════════════════════════════════════════════════════════════════════════════
#
# Every comparison above this line has ONE reference. That reference is
# `/opt/homebrew/bin/zsh`, release 5.9.2, and it is not the tree the ports are
# written from: those are read from `~/forkedRepos/zsh`, which is 5.9.999.3-test
# and carries four years of post-5.9 development. When the two disagree, a
# port written faithfully from the C is a FAIL against the binary, and a port
# that passes the binary is unfaithful to the C. Neither harness can tell which
# it is looking at, so the disagreement is discovered by accident, one at a
# time, after the fact:
#
#   `zparseopts` long specs   upstream 88d51a2400. Ported to the dev tree's
#                             semantics, which BROKE zinit, and was reverted
#                             behind a named switch (LONG_SPEC_NEEDS_GUARD,
#                             src/ported/modules/zutil.rs:2988).
#   chained subscript (R)/(I) upstream 8a3ee5a802. Deliberately resolved
#                             towards 5.9.2 at a named switch
#                             (wantvals_c1523, src/ported/subst.rs:10775).
#   dotted parameter names    upstream a9ba166216. Reported as a zshrs bug
#                             ("zshrs accepts `ns.var`"); the reference was
#                             simply older than the feature.
#   ZLE attribute-reset       upstream cc672f1c3b. Ported from the dev tree,
#                             which turned a CHECKED-IN regression guard red
#                             on a stream difference that has nothing to do
#                             with what the guard guards.
#
# Two of those were resolved towards 5.9.2 and two towards the dev tree, none
# of them by policy. This layer turns that into an inventory: run the SAME
# generated probe on both references and on zshrs, and classify.
#
#     the two references AGREE, zshrs differs   -> ZSHRS-BUG. This is what
#                                                  every other harness in this
#                                                  tree measures, and a split
#                                                  classification must never
#                                                  be allowed to excuse one —
#                                                  which is why the references
#                                                  agreeing is MEASURED here
#                                                  and not assumed.
#     the two references DISAGREE               -> VERSION SPLIT. zshrs cannot
#                                                  match both. Which side it
#                                                  takes is a policy decision,
#                                                  and the finding is that a
#                                                  decision is REQUIRED. The
#                                                  side it currently takes is
#                                                  reported, not judged.
#     all three agree                           -> AGREEMENT.
#
# A probe NEITHER reference can run (no probe line and no message on both) is a
# GENERATOR bug in its own category, exactly as in the substrate check — never
# a finding, and never quietly dropped. A probe both references REJECT with the
# same message is an agreement about the rejection and is counted as one: it is
# weak evidence and is named separately so a run cannot look busy on nothing
# but malformed probes.
#
# The dev reference has to be BUILT — `~/forkedRepos/zsh` is a source tree and
# is read only. `--build-dev-zsh` copies it to a scratch prefix and runs
# configure/make/install there, which gives a shell whose compiled-in
# `module_path` points inside the scratch tree, so all three shells run the
# byte-identical probe script with no per-shell preamble.

SPLIT_FAMILIES = ["paramname", "builtinopt", "subscript", "pattern", "zle"]

# Classifications.
AGREE, ZSHRS_BUG, SPLIT, BOTH_REJECT, UNUSABLE, UNSTABLE = (
    "agreement", "zshrs-bug", "version-split", "both-reject", "unusable",
    "unstable-reference")

# Which reference zshrs currently sides with, on a split.
SIDE_REL, SIDE_DEV, SIDE_NEITHER = "release", "dev", "neither"
# A side cannot be attributed from an output that does not repeat.
SIDE_UNSTABLE = "unattributable"


class Split(Construct):
    """One probe aimed at a surface where upstream zsh is known to have churned.

    `mode` is "cmd" (a `zsh -f <script>` one-liner, judged on the printed value
    AND on stderr, because "this version refuses it" is exactly what half the
    known splits look like) or "zle" (a pty session judged on the RAW escape
    stream, because the fourth known split exists only there).

    `impact` is a list of regexes for the corpus scan. A split no real code
    exercises is an inventory entry; one zinit relies on is urgent. That
    annotation NEVER changes a classification.
    """

    __slots__ = ("family", "mode", "impact", "seeded", "init", "buffer",
                 "keys", "bad_impact")

    def __init__(self, cid, family, cite, code, impact=(), mode="cmd",
                 seeded=False, init=(), buffer="", keys=()):
        Construct.__init__(self, cid, family, cite, code or "_sfz_r=")
        self.family = family
        self.mode = mode
        # A pattern that is not a valid regex would make the corpus scan report
        # "incomplete" for every root and silently lose the annotation, so it
        # is rejected HERE and named, not passed to rg to fail on.
        self.impact, self.bad_impact = [], []
        for pat in impact:
            try:
                re.compile(pat)
            except re.error as exc:
                self.bad_impact.append("%s (%s)" % (pat, exc))
            else:
                self.impact.append(pat)
        self.seeded = seeded          # a KNOWN split, carried as a seed case
        self.init = list(init)        # mode="zle": extra rc lines
        self.buffer = buffer          # mode="zle": what to type
        self.keys = list(keys) or ["tab"]


# ── the four confirmed splits, carried as seeds ──────────────────────────────
#
# Each is a probe that was verified by hand to separate the two references
# before it was written down. They are in the catalogue for two reasons: they
# are the shape the generator is aimed at, and they are the validation target —
# a scan that does not classify these four as splits is not working.
SPLIT_SEEDS = [
    Split("zparseopts-long-unguarded", "builtinopt",
          "upstream 88d51a2400 (54376); switch LONG_SPEC_NEEDS_GUARD at "
          "src/ported/modules/zutil.rs:2988", r"""
zmodload zsh/zutil
%Vf() { local -a o; zparseopts -D -E -a o -foo:; _sfz_r="st=$? o=${(j:|:)o} rest=$*" }
%Vf --foo=bar x
""",
          impact=[r"zparseopts[^\n]*[ \t]-[a-z][a-z]"], seeded=True),

    Split("zparseopts-stacking", "builtinopt",
          "upstream 88d51a2400 (54376) — the other half: -DFa now stacks",
          r"""
zmodload zsh/zutil
%Vf() { local -a o; zparseopts -DFa o - a; _sfz_r="st=$? o=${(j:|:)o} rest=$*" }
%Vf -a b
""",
          impact=[r"zparseopts[ \t]+-[A-Za-z]{2,}"], seeded=True),

    Split("subscript-search-slice-then-I", "subscript",
          "upstream 8a3ee5a802 (54093); switch wantvals_c1523 at "
          "src/ported/subst.rs:10775", r"""
%Va=(A b C d)
_sfz_r=${%Va[1,(r)d][(I)C]}
""",
          impact=[r"\]\[\((?:[iIrR])\)", r"wantvals_c1523",
                  r"chained_subscript"], seeded=True),

    Split("paramname-dotted", "paramname",
          "upstream a9ba166216 (51483) — ksh namespace prefixes, post-5.9",
          r"""
typeset %Vns.var=1
_sfz_r="got=${%Vns.var}"
""",
          impact=[r"typeset[ \t]+[A-Za-z_][A-Za-z_0-9]*\.[A-Za-z_]",
                  r"dotted_parameter_name"], seeded=True),

    # The fourth split lives on the escape stream, not in any printed value, so
    # it is a pty probe. `TCALLATTRSOFF` at the reset frame: 5.9.2 emits three
    # attribute-off caps, the dev tree emits one (Src/Zle/zle_refresh.c:1136-1138).
    Split("zle-attr-reset", "zle",
          "upstream cc672f1c3b (51289) — ~/forkedRepos/zsh/Src/Zle/"
          "zle_refresh.c:1136-1138 vs 5.9.2's three attribute-off caps",
          "", mode="zle",
          init=["PROMPT=$'%%B%s%%b '" % SENTINEL],
          buffer="ls a", keys=["tab", "tab", "x"],
          impact=[r"TCSTANDOUTEND", r"TCUNDERLINEEND",
                  r"listing_row_erase_to_eol"], seeded=True),
]


# ── the generator ────────────────────────────────────────────────────────────
#
# Aimed at the four surfaces the confirmed splits sit on, because a version
# split is not distributed evenly over the language: it lives where upstream
# has been WORKING. Reading the dev tree's ChangeLog for the post-5.9 range
# names those surfaces, and the four seeds above are one sample from each of
# the first four.

# Parameter-name shapes. The `.` forms are the ksh-namespace feature (a9ba166216);
# the rest are the neighbouring rules that feature had to not break.
_PN_NAMES = ["ns.var", "a.b.c", "_x.y", "ns.", ".lead", "ns..v", "ns.v1",
             "N.V", "x.y_z", "9ns.v", "ns.v-w", "a.b[1]"]
_PN_USES = [
    ("typeset",     "typeset %s=1",              "${%s}"),
    ("bare-assign", "%s=1",                      "${%s}"),
    ("typeset-a",   "typeset -a %s=(p q)",       "${(j:,:)%s}"),
    ("typeset-A",   "typeset -A %s=(k v)",       "${(kv)%s}"),
    ("local",       "local %s=1",                "${%s}"),
    ("export",      "typeset -x %s=1",           "${%s}"),
    ("integer",     "integer %s=3",              "$(( %s + 1 ))"),
    ("indirect",    "typeset %s=1; _n=%s",       "${(P)_n}"),
    ("typeset-p",   "typeset %s=1",              "${$(typeset -p %s)//$'\\n'/;}"),
    ("unset",       "typeset %s=1; unset %s",    "${+%s}"),
    ("subst-plus",  "typeset %s=1",              "${+%s}"),
]

# `zparseopts` is the builtin the confirmed split is in, and 88d51a2400 changed
# how it reads its OWN options, so the axis is the flag word and the guarding.
_BO_ZP_FLAGS = ["-D -E", "-DE", "-D -F", "-DF", "-DEK", "-D -E -K", "-DFG",
                "-D -F -G", "-M -D", "-DM", "-K -a", "-E"]
_BO_ZP_SPECS = ["-foo:", "- -foo:", "-- -foo:", "f -foo:", "-foo", "- -foo",
                "x", "- x", "''", "- ''", "-DEK", "- -DEK"]
_BO_ZP_ARGS = ["--foo=bar x", "--foo bar", "-x y", "-DEK", "--DEK", "-f -o -o",
               "-a b", "--"]

# Builtins whose own option parsing has been touched post-5.9, each with a
# small set of argument words that sit on the edges the changes were about.
_BO_OTHER = [
    ("print",    ["-r -- -x", "-rn -- a", "-l -- a b", "-v v -- a",
                  "-f '%s\\n' -- a", "-P -- '%%B'", "-s -- a", "-x 4 -- a"]),
    ("printf",   ["'%s\\n' a b", "-v v '%s' a", "'%d\\n' 0x10", "'%q\\n' 'a b'",
                  "'%(%Y)T\\n' 0"]),
    ("typeset",  ["-g -x v=1", "-U -a v", "-p v", "-T A B", "-hg v",
                  "+g v", "-m 'v*'", "-1 v=1"]),
    ("read",     ["-r -k 0 v", "-d '' v", "-A v", "-E -r v", "-t 0 v"]),
    ("fc",       ["-p -a", "-l -n -1", "-A"]),
    ("functions",["-M f", "-T", "-c a b", "-x 2"]),
    ("whence",   ["-w print", "-p print", "-S print", "-a -w print"]),
    ("unset",    ["-m 'v*'", "-v v", "-f f"]),
    ("hash",     ["-d -m 'x*'", "-L -d", "-r"]),
    ("getopts",  ["ab: o"]),
    ("zparseopts", None),   # handled by the dedicated axis above
]

# Subscript / expansion flags: the surface 8a3ee5a802 rewrote (`isarr` and
# SCANPM_WANTVALS), so the axis is a bound that raises WANTVALS chained to a
# second subscript that reads it.
_SS_BOUNDS = ["1", "2", "-1", "(r)d", "(R)d", "(i)C", "(I)C", "(n:1:)b",
              "(r)[bd]", "(R)*", "(e)C", "(ne)C"]
_SS_SECOND = ["(I)C", "(i)C", "(r)C", "(R)C", "1", "-1", "2,-1", "(I)*",
              "(r)*", "(k)C", "(K)C"]
_SS_ASSOC = ["(k)k1", "(K)k*", "(R)v1", "(r)v1", "(i)k1", "(I)k*", "(e)k1"]
_SS_PARAMFLAGS = ["", "(@)", "(o)", "(u)", "(P)", "(Q)", "(t)", "(M)", "(b)"]

# Pattern semantics. `(#b)`/`(#m)`/`(#a)`/`(#s)`/`(#e)` and the bracket classes
# are where the pattern engine has moved.
_PAT_SUBJECTS = ["abc", "aBc", "a.c", "abcabc", "", "a/b/c", "a-c", "]a["]
_PAT_PATTERNS = ["a*c", "(#i)ABC", "(#I)ABC", "a#c", "(a|b)*", "[[:alpha:]]##",
                 "(#s)a*", "*c(#e)", "(#a1)abd", "(#b)(a)*(c)", "(#m)a*c",
                 "^a*", "a~ab*", "[a-c]#", "(#c3)a", "?(#c2)c", "**/c",
                 "[[:IDENT:]]##", "(#q.)"]


def gen_split(seed, idx, families):
    """Draw one generated probe. Deterministic in (seed, idx)."""
    rng = random.Random("compsys_spec_fuzz:split:%d:%d" % (seed, idx))
    fam = rng.choice([f for f in families if f != "zle"] or families)
    v = "_sp%d_" % idx
    if fam == "paramname":
        name = rng.choice(_PN_NAMES)
        uid, setup, read = rng.choice(_PN_USES)
        full = v + name if name[0] not in "._9" else v + name
        code = ("%s\n_sfz_r=%s"
                % (setup.replace("%s", full), read.replace("%s", full)))
        return Split("gen%04d-paramname-%s-%s" % (idx, uid,
                                                  re.sub(r"\W", "", name)),
                     "paramname",
                     "generated: parameter-name rules (upstream a9ba166216 "
                     "changed this surface)", code,
                     impact=[re.escape(name)])
    if fam == "builtinopt":
        if rng.random() < 0.6:
            flags = rng.choice(_BO_ZP_FLAGS)
            spec = rng.choice(_BO_ZP_SPECS)
            argw = rng.choice(_BO_ZP_ARGS)
            code = ("zmodload zsh/zutil\n"
                    "%sf() { local -a o; zparseopts %s a o %s\n"
                    "  _sfz_r=\"st=$? o=${(j:|:)o} rest=$*\" }\n"
                    "%sf %s" % (v, flags, spec, v, argw))
            return Split("gen%04d-zparseopts" % idx, "builtinopt",
                         "generated: zparseopts own-option parsing "
                         "(upstream 88d51a2400)", code,
                         impact=[r"zparseopts[ \t]+%s" % re.escape(flags.split()[0])])
        cand = [(b, a) for b, a in _BO_OTHER if a]
        bi, argl = rng.choice(cand)
        argw = rng.choice(argl)
        code = ("%sf() { local -a v; %s %s\n"
                "  _sfz_r=\"st=$? v=${(j:|:)v}\" }\n"
                "%sf" % (v, bi, argw, v))
        return Split("gen%04d-%s" % (idx, bi), "builtinopt",
                     "generated: %s option parsing" % bi, code,
                     impact=[r"\b%s\b[^\n]*%s" % (bi,
                                                  re.escape(argw.split()[0]))])
    if fam == "subscript":
        if rng.random() < 0.25:
            sub = rng.choice(_SS_ASSOC)
            code = ("typeset -A %sh=(k1 v1 k2 v2)\n"
                    "_sfz_r=${%sh[%s]}" % (v, v, sub))
            return Split("gen%04d-assoc-sub" % idx, "subscript",
                         "generated: associative subscript flags "
                         "(upstream 8a3ee5a802 rewrote the isarr fields)",
                         code, impact=[re.escape(sub)])
        lo, hi = rng.choice(_SS_BOUNDS), rng.choice(_SS_BOUNDS)
        second = rng.choice(_SS_SECOND)
        pf = rng.choice(_SS_PARAMFLAGS)
        inner = "%sa[%s,%s][%s]" % (v, lo, hi, second)
        code = ("%sa=(A b C d)\n_sfz_r=\"${%s%s}\"" % (v, pf, inner))
        return Split("gen%04d-chained-sub" % idx, "subscript",
                     "generated: chained subscript after a search bound "
                     "(upstream 8a3ee5a802)", code,
                     impact=[re.escape("][" + second)])
    # pattern
    subj, pat = rng.choice(_PAT_SUBJECTS), rng.choice(_PAT_PATTERNS)
    form = rng.choice(("cond", "case", "subst", "glob-qual"))
    if form == "cond":
        code = ('%ss=%s\n[[ $%ss == %s ]] && _sfz_r=y || _sfz_r=n\n'
                '_sfz_r="$_sfz_r ${match[*]:-} ${MATCH:-}"'
                % (v, zq(subj), v, zq(pat)))
    elif form == "case":
        code = ('%ss=%s\ncase $%ss in (%s) _sfz_r=y ;; (*) _sfz_r=n ;; esac'
                % (v, zq(subj), v, pat))
    elif form == "subst":
        code = '%ss=%s\n_sfz_r="${%ss//%s/X}"' % (v, zq(subj), v, pat)
    else:
        code = ('%ss=%s\n_sfz_r="${%ss:#%s}"' % (v, zq(subj), v, pat))
    return Split("gen%04d-pattern" % idx, "pattern",
                 "generated: pattern semantics", code,
                 impact=[re.escape(pat)])


# ── the dev reference: found, or built in scratch ────────────────────────────

DEV_TREE_DEFAULT = os.path.expanduser("~/forkedRepos/zsh")


def _zsh_version(binary):
    """-> the $ZSH_VERSION a binary reports, or None if it will not run."""
    try:
        p = subprocess.run([binary, "-f", "-c", "print -r -- $ZSH_VERSION"],
                           capture_output=True, text=True, timeout=20, env={})
    except (OSError, subprocess.SubprocessError):
        return None
    v = p.stdout.strip()
    return v or None


def _modules_load(binary):
    """Positive evidence that the binary can dlopen its own modules."""
    try:
        p = subprocess.run([binary, "-f", "-c", "zmodload zsh/zutil && "
                            "print -r -- @@MOD ok"],
                           capture_output=True, text=True, timeout=20, env={})
    except (OSError, subprocess.SubprocessError) as exc:
        return False, repr(exc)
    if "@@MOD ok" in p.stdout:
        return True, ""
    return False, re.sub(r"\s+", " ", (p.stderr or "").strip())[:200]


def build_dev_zsh(tree, dest):
    """Build `tree` at prefix `dest` and return the built binary's path.

    `~/forkedRepos/zsh` is READ ONLY, so the tree is copied first. The prefix
    is inside the scratch build, which is the whole point: the resulting shell
    has a compiled-in `module_path` pointing at its OWN modules, so all three
    shells can run the byte-identical probe script with no per-shell preamble.
    A preamble would mean the dev reference ran a different program from the
    other two, which is not a comparison.
    """
    src = os.path.join(dest, "src")
    prefix = os.path.join(dest, "inst")
    log = os.path.join(dest, "build.log")
    os.makedirs(dest, exist_ok=True)
    if not os.path.isdir(tree):
        return None, "no such tree: %s" % tree
    if not os.path.isdir(src):
        r = subprocess.run(["cp", "-a", tree, src], capture_output=True)
        if r.returncode:
            return None, "cp -a failed: %s" % r.stderr.decode()[:200]
        shutil.rmtree(os.path.join(src, ".git"), ignore_errors=True)
    steps = []
    if not os.path.exists(os.path.join(src, "configure")):
        steps.append(["./Util/preconfig"])
    # `install.modules` and NOT `install`: a full install also builds and
    # installs the man pages, which needs yodl, and a host without yodl fails
    # the whole install at `install.man` after the modules are already in
    # place. The binary is used in the build tree; only the modules have to be
    # at the prefix the shell was compiled to look in.
    steps += [["./configure", "--prefix=" + prefix],
              ["make", "-j%d" % (os.cpu_count() or 4)],
              ["make", "install.modules"]]
    with open(log, "w") as fh:
        for cmd in steps:
            fh.write("\n\n### %s\n" % " ".join(cmd))
            fh.flush()
            r = subprocess.run(cmd, cwd=src, stdout=fh, stderr=subprocess.STDOUT)
            if r.returncode:
                return None, ("%s failed (rc %d) — see %s"
                              % (" ".join(cmd), r.returncode, log))
    binary = os.path.join(src, "Src", "zsh")
    if not os.path.isfile(binary):
        return None, "build produced no %s (see %s)" % (binary, log)
    return binary, ""


def dev_tree_version(tree):
    """The version the SOURCE tree claims, read from Config/version.mk.

    The dev reference is only a reference for the tree the ports are read
    from. This host also has an older dev snapshot at `src/zsh` (5.9.0.3-test),
    which is neither the release nor the tree, and a scan that silently used it
    would attribute its findings to the wrong source.
    """
    try:
        with open(os.path.join(tree, "Config", "version.mk"),
                  encoding="utf-8", errors="replace") as fh:
            m = re.search(r"^VERSION=(\S+)", fh.read(), re.M)
    except OSError:
        return None
    return m.group(1) if m else None


def resolve_dev_zsh(args):
    """-> (path, version, note). Never guesses: every candidate is RUN.

    A candidate that reports the SAME $ZSH_VERSION as the release reference is
    refused. A second reference that is the same version is not a second
    reference, and a three-way classification built on one would report
    "agreement" for every split there is.
    """
    cands = []
    if args.zsh_dev:
        cands.append(os.path.expanduser(args.zsh_dev))
    if os.environ.get("SPEC_FUZZ_ZSH_DEV"):
        cands.append(os.path.expanduser(os.environ["SPEC_FUZZ_ZSH_DEV"]))
    cands.append(os.path.join(REPO, "target", "spec-fuzz-devzsh", "src",
                              "Src", "zsh"))
    cands.append(os.path.join(REPO, "src", "zsh", "Src", "zsh"))
    tried = []
    for c in cands:
        if not (c and os.path.isfile(c) and os.access(c, os.X_OK)):
            tried.append("%s: not an executable file" % c)
            continue
        v = _zsh_version(c)
        if not v:
            tried.append("%s: would not run" % c)
            continue
        if v == args.zsh_version:
            tried.append("%s: reports %s, the SAME version as the release "
                         "reference — not a second reference" % (c, v))
            continue
        ok, why = _modules_load(c)
        if not ok:
            tried.append("%s (%s): cannot load its own modules (%s)"
                         % (c, v, why))
            continue
        want = dev_tree_version(args.dev_tree)
        note = "found"
        if want and v != want:
            # Reported, not refused: a second reference of a DIFFERENT version
            # still measures a version split. But it is not the tree the ports
            # are read from, so every finding it produces is about a version
            # nobody is porting against, and saying so is the difference
            # between an inventory and a misattribution.
            note = ("found, but reports %s while %s claims %s — this is NOT "
                    "the tree the ports are read from"
                    % (v, args.dev_tree, want))
        return c, v, note
    if args.build_dev_zsh:
        dest = os.path.join(REPO, "target", "spec-fuzz-devzsh")
        print("# building the dev reference from %s into %s "
              "(this takes a few minutes)" % (args.dev_tree, dest))
        sys.stdout.flush()
        binary, why = build_dev_zsh(args.dev_tree, dest)
        if not binary:
            tried.append("build: %s" % why)
        else:
            v = _zsh_version(binary)
            ok, mwhy = _modules_load(binary)
            if v == args.zsh_version:
                tried.append("built %s but it reports %s, the same version as "
                             "the release reference" % (binary, v))
            elif not ok:
                tried.append("built %s (%s) but it cannot load its own "
                             "modules (%s)" % (binary, v, mwhy))
            else:
                return binary, v, "built from %s" % args.dev_tree
    return None, None, "\n#   ".join(tried)


# ── running and classifying one probe ────────────────────────────────────────

_SPLIT_LOC_RE = re.compile(r"^(?:zsh|zshrs)(?:\(\w+\))?:(?:[^:]*:)?(?:\d+:)?\s*")


def _errkey(err):
    """The COMPARABLE part of a diagnostic.

    Strips only the emitter's own location prefix — the shell name, the
    function name and the line number — because those differ between two
    shells for reasons that are not the behaviour under test and would
    otherwise report every rejection as a divergence. The MESSAGE is compared
    in full and both raw texts are printed with every finding, so nothing is
    hidden by this.
    """
    if not err:
        return ""
    return _SPLIT_LOC_RE.sub("", err.strip())


def _outcome(val, err):
    return (val, _errkey(err))


_SGR_RE = re.compile(rb"\x1b\[[0-9;]*m")


def _attr_signature(raw):
    """The ordered SGR sequences in a stream.

    This is the quantity a `zle` probe is judged on, and it is a NEW
    measurement, not a relaxed one: the confirmed attribute-reset split
    (cc672f1c3b) is invisible in the rendered grid and shows up in the stream
    as `\\e[0m\\e[27m\\e[24m` against `\\e[0m`. Full raw-stream equality is
    recorded alongside it and printed, so a probe that matches on the signature
    and differs elsewhere is still reported as differing.
    """
    return tuple(_SGR_RE.findall(raw or b""))


def run_split_cmd(args, root, probe, shells):
    """`shells` is [(id, argv)]. -> {id: {"val","err","outcome"}}."""
    os.makedirs(root, exist_ok=True)
    work = os.path.join(root, "work")
    os.makedirs(work, exist_ok=True)
    path = os.path.join(root, "probe-%s.zsh" % re.sub(r"\W", "_", probe.id))
    with _wopen(path) as f:
        f.write(probe.script())
    env = child_env(work, args.locale_pool[0])
    out = {}
    for sid, argv in shells:
        val, err = _run_substrate(argv, env, path, work, args.substrate_timeout)
        out[sid] = {"val": val, "err": err, "outcome": _outcome(val, err)}
    out["_script"] = path
    return out


def run_split_zle(args, root, probe, shells):
    """A pty probe, judged on the SGR signature. Reuses capture() — the one
    pty driver in this file; there is deliberately no second one."""
    d = os.path.join(root, "zle-%s" % re.sub(r"\W", "_", probe.id))
    work = os.path.join(d, "work")
    os.makedirs(work, exist_ok=True)
    for f in WORK_FILES:
        p = os.path.join(work, f)
        if not os.path.exists(p):
            open(p, "w").close()
    init = os.path.join(d, "init.zsh")
    with _wopen(init) as f:
        f.write("fpath=( %s )\n"
                "PROMPT='%s '\nRPROMPT=''\nsetopt no_beep\n"
                "builtin cd %s\n"
                "autoload -Uz compinit\ncompinit -u -D\n"
                "%s"
                "print -u2 ''\n"
                % (" ".join(shlex.quote(p) for p in args.fpath_dirs),
                   SENTINEL, shlex.quote(work),
                   "".join(ln + "\n" for ln in probe.init)))
    env = child_env(d, args.locale_pool[0])
    out = {}
    for sid, argv in shells:
        cap = capture(list(argv) + ["-f", "-i"], env, args, init, work,
                      probe.buffer, probe.keys, args.cols, True)
        sig = _attr_signature(cap.raw)
        # A shell that never reached a prompt produced no signature, and an
        # EMPTY signature would compare equal to another dead shell's — two
        # corpses agreeing is not agreement. `None` routes it to the unusable /
        # rejection arms of classify_split() the same way a missing probe line
        # does for a `cmd` probe.
        val = None if cap.reason else b"|".join(sig)
        out[sid] = {"val": val, "err": cap.reason or "",
                    "outcome": (val, _errkey(cap.reason or "")),
                    "raw": cap.raw, "sig": sig}
    # A pty probe has to prove its reference is STABLE before its answer means
    # anything. This was not theory: under load the `zle` probe's SGR signature
    # moved, and the split's SIDE attribution moved with it (dev on a quiet
    # host, neither on a busy one) while the split itself stayed. Re-run the
    # release reference and require the signature to repeat; a reference that
    # disagrees with itself gets the probe reported as unstable rather than
    # given a side it cannot support. This mirrors run_case()'s --self-check.
    def restable(sid, argv):
        """-> "" if this shell repeats its own signature, else why not."""
        two = capture(list(argv) + ["-f", "-i"], env, args, init, work,
                      probe.buffer, probe.keys, args.cols, True)
        if two.reason:
            return "%s did not run a second time (%s)" % (sid, two.reason)
        if _attr_signature(two.raw) != out[sid]["sig"]:
            return ("%s produced a DIFFERENT escape signature on a re-run "
                    "(%s vs %s)"
                    % (sid,
                       disp(bdec(b"|".join(_attr_signature(two.raw))))[:80],
                       disp(bdec(b"|".join(out[sid]["sig"])))[:80]))
        return ""

    why = restable(*shells[0])
    if why:
        out["_unstable"] = why
    # And the shell UNDER TEST, separately. These mean different things: an
    # unstable reference means the probe cannot be judged at all, while an
    # unstable zshrs still leaves the split between the two references
    # standing — what it removes is the right to say which side zshrs is on.
    # Not hypothetical: on the `zle-attr-reset` probe zshrs emitted
    # `\e[1m \e[0m \e[0m` on one run and `\e[1m \e[0m \e[0m \e[27m
    # \e[24m` on another, which without this check is reported as "takes dev"
    # and "takes neither" by two runs of the same tool.
    twhy = restable("zshrs", args.test_base)
    if twhy:
        out["_test_unstable"] = twhy
    out["_script"] = init
    return out


def classify_split(res):
    """-> (classification, side). The ONLY place a probe is judged.

    `zshrs-bug` requires the two references to have been MEASURED to agree.
    A `version-split` never absorbs one: the two classes are decided by the
    same comparison and cannot both apply.
    """
    rel, dev, rs = res["release"], res["dev"], res["zshrs"]
    # A reference that disagrees with ITSELF is not a reference. Proven, not
    # assumed — see run_split_zle(). Never a pass and never a split: the probe
    # simply could not be judged, and it is counted under its own name.
    if res.get("_unstable"):
        return UNSTABLE, ""
    # A probe NEITHER reference could even start is a generator bug.
    if rel["val"] is None and dev["val"] is None and not rel["err"] and not dev["err"]:
        return UNUSABLE, ""
    if rel["outcome"] == dev["outcome"]:
        if rs["outcome"] == rel["outcome"]:
            # Both references refused it with the same message. That IS an
            # agreement, but it is agreement about a rejection and is counted
            # apart so a scan cannot look busy on malformed probes.
            return (BOTH_REJECT if rel["val"] is None else AGREE), ""
        return ZSHRS_BUG, ""
    if res.get("_test_unstable"):
        # The split stands — it is a property of the two references — but the
        # side does not, because the output it would be read from does not
        # repeat.
        return SPLIT, SIDE_UNSTABLE
    side = (SIDE_REL if rs["outcome"] == rel["outcome"] else
            SIDE_DEV if rs["outcome"] == dev["outcome"] else SIDE_NEITHER)
    return SPLIT, side


# ── does any real code depend on it? ─────────────────────────────────────────
#
# A split no installed code exercises is an inventory entry. One zinit relies
# on is urgent — and today's zinit breakage was found only because somebody
# thought to grep. This annotation NEVER changes a classification; it is
# printed beside one.

CORPUS_ROOTS = [
    ("zinit", "~/.zinit"),
    ("zpwr", "~/.zpwr"),
    ("more-completions",
     "~/.zinit/plugins/MenkeTechnologies---zsh-more-completions/src"),
    ("stock-compsys", "/opt/homebrew/share/zsh"),
    # This repo's own checked-in guards. The fourth confirmed split cost a red
    # guard (tests/compsys_fixtures/listing_row_erase_to_eol) and no user code
    # at all, so a corpus that stops at the user's dotfiles would have scored
    # it "not urgent" — exactly backwards.
    ("zshrs-guards", os.path.join(REPO, "tests")),
]


def corpus_roots():
    """-> [(label, path, present)]. A missing root is REPORTED, never a
    silent zero: 'nothing depends on this' and 'the corpus was not there' are
    different findings."""
    out = []
    for label, p in CORPUS_ROOTS:
        rp = os.path.expanduser(p)
        out.append((label, rp, os.path.isdir(rp)))
    return out


def corpus_scan(patterns, roots, limit=3, timeout=120):
    """-> {label: {"n": int, "hits": [str], "why": str}}"""
    tool = shutil.which("rg")
    res = {}
    for label, path, present in roots:
        if not present:
            res[label] = {"n": 0, "hits": [], "why": "root not present: %s" % path}
            continue
        hits, err = [], ""
        for pat in patterns:
            if tool:
                cmd = [tool, "--no-heading", "-n", "-e", pat, "--", path]
            else:
                cmd = ["grep", "-rnE", "-e", pat, path]
            try:
                p = subprocess.run(cmd, capture_output=True, timeout=timeout)
            except (OSError, subprocess.SubprocessError) as exc:
                err = repr(exc)
                continue
            if p.returncode not in (0, 1):
                err = (p.stderr or b"").decode("utf-8", "replace")[:120]
            for ln in (p.stdout or b"").decode("utf-8", "replace").splitlines():
                if ln:
                    hits.append(ln)
        res[label] = {"n": len(hits),
                      "hits": [h[:160] for h in hits[:limit]],
                      "why": err}
    return res


# ── the scan ─────────────────────────────────────────────────────────────────

def split_scan(args):
    """Run the whole split layer and print the inventory. -> exit status."""
    shells = [("release", [args.zsh]),
              ("dev", [args.zsh_dev]),
              ("zshrs", list(args.test_base))]
    root = os.path.join(REPO, "target", "spec-fuzz-%d" % args.seed, "splits")
    os.makedirs(root, exist_ok=True)

    probes = []
    if args.split_seeds:
        probes += [p for p in SPLIT_SEEDS if p.family in args.split_families]
    # `zle` is a pty family with no grammar behind it — its one probe is the
    # confirmed split, carried as a seed. Say so rather than generate nothing
    # and let the count look like a choice.
    genfams = [f for f in args.split_families if f != "zle"]
    ngen = args.split_cases if genfams else 0
    for i in range(ngen):
        probes.append(gen_split(args.seed, args.split_start + i, genfams))
    if args.split_cases and not genfams:
        print("# --split-cases %d generated NOTHING: `zle` is the only family "
              "in scope and it has no grammar — its probe is the seeded one"
              % args.split_cases)

    print("# compsys_spec_fuzz — VERSION-SPLIT scan (three-way)")
    print("# reference A (release): %s   $ZSH_VERSION %s"
          % (args.zsh, args.zsh_version))
    print("# reference B (dev)    : %s   $ZSH_VERSION %s   [%s]"
          % (args.zsh_dev, args.zsh_dev_version, args.zsh_dev_note))
    print("# under test           : %s" % " ".join(args.test_base))
    print("# dev C source read by the ports: %s" % args.dev_tree)
    print("# seed %d   %d probe(s): %d seeded + %d generated %d..%d   "
          "families=%s   locale=%s"
          % (args.seed, len(probes),
             sum(1 for p in probes if p.seeded),
             sum(1 for p in probes if not p.seeded),
             args.split_start, args.split_start + max(ngen, 1) - 1,
             ",".join(args.split_families), args.locale_pool[0]))
    off = [f for f in SPLIT_FAMILIES if f not in args.split_families]
    if off:
        print("# famil(ies) this scan EXCLUDED, so NOT tested: %s" % " ".join(off))
    print()
    sys.stdout.flush()

    roots = corpus_roots()
    missing = [l for l, _p, ok in roots if not ok]
    if missing:
        print("# corpus root(s) NOT on this host, so no dependency claim can "
              "be made about them: %s" % " ".join(missing))
        print()

    rows = []
    for p in probes:
        try:
            res = (run_split_zle(args, root, p, shells) if p.mode == "zle"
                   else run_split_cmd(args, root, p, shells))
        except Exception as exc:
            print("%-14s %-12s %-34s could not run: %r"
                  % ("ERROR", p.family, p.id[:34], exc))
            continue
        cls, side = classify_split(res)
        impact = corpus_scan(p.impact, roots) if cls == SPLIT else {}
        rows.append((p, res, cls, side, impact))
        tag = {AGREE: "agree", ZSHRS_BUG: "ZSHRS-BUG", SPLIT: "VERSION-SPLIT",
               BOTH_REJECT: "agree(rej)", UNUSABLE: "unusable",
               UNSTABLE: "UNSTABLE"}[cls]
        note = ""
        if cls == SPLIT:
            n = sum(v["n"] for v in impact.values())
            note = "zshrs takes %-8s  corpus hits %s%d%s" % (
                side, "" if p.seeded else "~", n,
                "  (" + " ".join("%s=%d" % (l, impact[l]["n"])
                                 for l, _p, ok in roots
                                 if ok and impact[l]["n"]) + ")" if n else "")
        print("%-14s %-12s %-34s %s" % (tag, p.family, p.id[:34], note))
        if cls in (SPLIT, ZSHRS_BUG):
            print_split(p, res, cls, side, impact, roots)
        sys.stdout.flush()

    return summarise_splits(args, rows, roots)


def _show(res, sid):
    d = res[sid]
    v = "<no value>" if d["val"] is None else disp(bdec(d["val"]))
    e = (" | stderr: %s" % disp(d["err"])) if d["err"] else ""
    return (v + e)[:220]


def print_split(p, res, cls, side, impact, roots):
    print("    cite   : %s" % p.cite)
    print("    probe  : %s" % res["_script"])
    for sid, label in (("release", "zsh 5.9.2 "), ("dev", "zsh dev  "),
                       ("zshrs", "zshrs    ")):
        print("      %s: %s" % (label, _show(res, sid)))
    if cls == SPLIT:
        print("    -> the two REFERENCES disagree. zshrs cannot match both; "
              "it currently matches: %s" % side)
        if side == SIDE_UNSTABLE:
            print("       (zshrs did not repeat its own output between two "
                  "runs of this probe — %s)" % res.get("_test_unstable", ""))
        if p.mode == "cmd":
            print("    repro  : zsh -f %s   (and the dev build, and "
                  "`zshrs --zsh -f`)" % res["_script"])
        total = sum(v["n"] for v in impact.values())
        if total:
            print("    installed code / checked-in guards that TEXTUALLY "
                  "match this surface (%s):"
                  % ("patterns written for this split — a lead to audit, not "
                     "a proof of dependency" if p.seeded else
                     "patterns derived MECHANICALLY from the generated "
                     "construct — a coincidence until read"))
            for label, _path, ok in roots:
                if not ok or not impact[label]["n"]:
                    continue
                print("      %-18s %d hit(s)" % (label, impact[label]["n"]))
                for h in impact[label]["hits"]:
                    print("        %s" % disp(h))
        else:
            print("    no hit in %s — an inventory entry, not an urgent one"
                  % " ".join(l for l, _p, ok in roots if ok))
        for label, _p, ok in roots:
            if ok and impact.get(label, {}).get("why"):
                print("      ! corpus scan of %s was incomplete: %s"
                      % (label, impact[label]["why"]))
        # A pattern that would not compile is dropped at construction. Say so:
        # a corpus count taken with fewer patterns than the probe declared is
        # a smaller number than the probe asked for, and silence would make it
        # look like a clean zero.
        for bad in p.bad_impact:
            print("      ! corpus pattern DROPPED (not a valid regex), so the "
                  "count above is short by whatever it would have matched: %s"
                  % bad)
    else:
        print("    -> the two REFERENCES AGREE and zshrs differs. This is a "
              "zshrs bug, not a version question.")


def summarise_splits(args, rows, roots):
    counts = {k: 0 for k in (AGREE, BOTH_REJECT, ZSHRS_BUG, SPLIT, UNUSABLE,
                             UNSTABLE)}
    for _p, _r, cls, _s, _i in rows:
        counts[cls] += 1
    print()
    print("=" * 78)
    print("# %d probe(s): %d agreement, %d agreement-on-rejection, "
          "%d VERSION-SPLIT, %d ZSHRS-BUG, %d unusable, %d unstable-reference"
          % (len(rows), counts[AGREE], counts[BOTH_REJECT], counts[SPLIT],
             counts[ZSHRS_BUG], counts[UNUSABLE], counts[UNSTABLE]))
    perf = {}
    for p, _r, cls, _s, _i in rows:
        row = perf.setdefault(p.family, dict.fromkeys(counts, 0))
        row[cls] += 1
    print("#   by family:   probes  agree  agree(rej)  SPLIT  ZSHRS-BUG  "
          "unusable  unstable")
    for fam in SPLIT_FAMILIES:
        if fam in perf:
            r = perf[fam]
            print("#     %-12s %5d  %5d  %10d  %5d  %9d  %8d  %8d"
                  % (fam, sum(r.values()), r[AGREE], r[BOTH_REJECT], r[SPLIT],
                     r[ZSHRS_BUG], r[UNUSABLE], r[UNSTABLE]))
    splits = [(p, s, i) for p, _r, cls, s, i in rows if cls == SPLIT]
    if splits:
        print("#")
        print("#   VERSION-SPLIT INVENTORY — a decision is required for each")
        print("#   %-34s %-9s %-8s %s"
              % ("probe", "zshrs", "corpus", "family"))
        print("#   (a `~` corpus count comes from a pattern derived from the "
              "generated construct, not written for the split)")
        for p, side, impact in splits:
            n = sum(v["n"] for v in impact.values())
            print("#   %-34s %-9s %-8s %s"
                  % (p.id[:34], side,
                     ("%d" % n if p.seeded else "~%d" % n) if n else "-",
                     p.family))
        by_side = {}
        for _p, side, _i in splits:
            by_side[side] = by_side.get(side, 0) + 1
        if by_side.get(SIDE_UNSTABLE):
            print("#   %d split(s) where zshrs's OWN output did not repeat "
                  "between two runs, so NO side could be attributed — the "
                  "split still stands, the side does not"
                  % by_side[SIDE_UNSTABLE])
        print("#   sides taken: %s"
              % ("  ".join("%s=%d" % kv for kv in sorted(by_side.items()))))
        if len(by_side) > 1:
            print("#   zshrs currently resolves splits BOTH ways. That is the "
                  "finding: there is no policy, only accidents.")
        # Only a CURATED pattern can put a split on the urgent list. A
        # mechanically derived one that happens to match is not evidence that
        # anything depends on the split, and a list padded with coincidences
        # is worth less than no list.
        urgent = [p.id for p, _s, i in splits
                  if p.seeded and sum(v["n"] for v in i.values())]
        if urgent:
            print("#   split(s) with a CURATED corpus pattern that hits real "
                  "installed code or a checked-in guard — urgent: %s"
                  % " ".join(urgent))
        loose = [p.id for p, _s, i in splits
                 if not p.seeded and sum(v["n"] for v in i.values())]
        if loose:
            print("#   generated split(s) whose mechanical pattern hit "
                  "something — a lead only, NOT counted urgent: %s"
                  % " ".join(loose))
    bugs = [p.id for p, _r, cls, _s, _i in rows if cls == ZSHRS_BUG]
    if bugs:
        print("#   ZSHRS-BUG(s) — BOTH references agree and zshrs does not: %s"
              % " ".join(bugs))
    unst = [(p.id, r.get("_unstable", "")) for p, r, cls, _s, _i in rows
            if cls == UNSTABLE]
    if unst:
        print("#   probe(s) whose REFERENCE disagreed with itself, so they "
              "could not be judged at all — never counted as agreement:")
        for pid, why in unst:
            print("#     %-34s %s" % (pid, why))
    unus = [p.id for p, _r, cls, _s, _i in rows if cls == UNUSABLE]
    if unus:
        print("#   generator-rejected probe(s) — NEITHER reference could run "
              "them, so they say nothing about zshrs: %s" % " ".join(unus))
    seeds_seen = {p.id: cls for p, _r, cls, _s, _i in rows if p.seeded}
    if seeds_seen:
        print("#   seeded (already-confirmed) splits, as classified by this "
              "run — these are the validation target:")
        for sid in sorted(seeds_seen):
            print("#     %-34s %s" % (sid, seeds_seen[sid]))
        wrong = [s for s, c in seeds_seen.items()
                 if c not in (SPLIT, UNSTABLE)]
        if wrong:
            print("#   !! a confirmed split did NOT classify as one: %s"
                  % " ".join(wrong))
    if args.split_json:
        with open(args.split_json, "w") as fh:
            json.dump({
                "seed": args.seed,
                "release": {"path": args.zsh, "version": args.zsh_version},
                "dev": {"path": args.zsh_dev, "version": args.zsh_dev_version,
                        "note": args.zsh_dev_note, "tree": args.dev_tree},
                "zshrs": args.test_base,
                "corpus_roots": [{"label": l, "path": p, "present": ok}
                                 for l, p, ok in roots],
                "counts": counts,
                "probes": [{
                    "id": p.id, "family": p.family, "cite": p.cite,
                    "mode": p.mode, "seeded": p.seeded,
                    "class": cls, "zshrs_side": s,
                    "release": bdec(r["release"]["val"] or b"") ,
                    "release_err": r["release"]["err"],
                    "dev": bdec(r["dev"]["val"] or b""),
                    "dev_err": r["dev"]["err"],
                    "zshrs": bdec(r["zshrs"]["val"] or b""),
                    "zshrs_err": r["zshrs"]["err"],
                    "corpus": {k: {"n": v["n"], "hits": v["hits"]}
                               for k, v in i.items()},
                } for p, r, cls, s, i in rows],
            }, fh, indent=1)
        print("# json: %s" % args.split_json)
    # A split is not a failure of zshrs, so it does not fail the scan; a
    # measured zshrs bug and a probe neither reference can run both do.
    return 1 if (counts[ZSHRS_BUG] or counts[UNUSABLE]
                 or counts[UNSTABLE]) else 0


# ═════════════════════════════════════════════════════════════════════════════
# widget self-check
# ═════════════════════════════════════════════════════════════════════════════
#
# A binding real zsh REJECTS is a generator bug, not a finding: every case
# drawn through it would diverge for a reason that says nothing about zshrs.
# So before a run generates anything, every widget declaration is executed on
# the reference shell and proved to have installed the binding it claims —
# `zle -C` writes no message on success, so "no complaint" is not enough
# evidence and each plan carries a `verify` command whose output must name the
# widget.
#
# The same script runs on zshrs. That side is NOT used to exclude anything: an
# id real zsh accepts and zshrs rejects is a divergence, and it is reported as
# one.

OK, REJECTED, NOTBOUND = "ok", "REJECTED", "not-bound"

_CHECK_RE = re.compile(r"^@@(B|RC|V|E) (\S+)(?: (.*))?$")


def _probe_body(fn):
    return ["%s() { compadd sfzprobe }" % fn]


def _check_block(wid, plan, fn):
    """The lines that declare one widget and prove the binding landed."""
    out = ["print -r -- '@@B %s'" % wid]
    if plan["complist"]:
        out.append("zmodload -i zsh/complist")
    out += plan["pre"] + plan["decl"]
    out.append("print -r -- \"@@RC %s $?\"" % wid)
    if plan["verify"]:
        cmd = plan["verify"][0]
        out.append("_sfz_v=$( { %s } 2>&1 )" % cmd)
        out.append("print -r -- \"@@V %s ${_sfz_v//$'\\n'/ }\"" % wid)
    out.append("print -r -- '@@E %s'" % wid)
    return out


def _parse_check(text, wid_order, plans, filefn=None):
    """-> {id: (status, message)} from one annotated run."""
    blocks = {}
    cur, buf = None, []
    rc, ver = {}, {}
    for line in text.splitlines():
        m = _CHECK_RE.match(line.strip())
        if not m:
            if cur:
                buf.append(line.rstrip())
            continue
        tag, wid, rest = m.group(1), m.group(2), (m.group(3) or "")
        if tag == "B":
            cur, buf = wid, []
        elif tag == "RC":
            rc[wid] = rest.strip()
            blocks[wid] = [b for b in buf if b.strip()]
            buf = []
        elif tag == "V":
            ver[wid] = rest.strip()
        elif tag == "E":
            cur = None
    out = {}
    for wid in wid_order:
        if wid not in rc:
            out[wid] = (REJECTED, "the shell never reached this declaration")
            continue
        noise = blocks.get(wid) or []
        if noise:
            out[wid] = (REJECTED, "; ".join(noise)[:160])
            continue
        if rc[wid] not in ("0", ""):
            out[wid] = (REJECTED, "declaration exited %s" % rc[wid])
            continue
        v = plans[wid]["verify"]
        if v:
            token = v[1].replace(FILEFN, filefn) if filefn else v[1]
            got = ver.get(wid, "")
            if token not in got.split():
                out[wid] = (NOTBOUND, "%s -> %r (expected %s)"
                            % (v[0], got[:100], token))
                continue
        out[wid] = (OK, "")
    return out


def _run_check(argv, env, cwd, script, timeout):
    try:
        p = subprocess.run(list(argv) + ["-f", "-c", script],
                           stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                           text=True, timeout=timeout, env=env, cwd=cwd)
    except subprocess.TimeoutExpired:
        return "", "timed out after %ds" % timeout
    except OSError as exc:
        return "", "could not run: %r" % (exc,)
    return p.stdout, None


def check_widgets(args, ids):
    """Run every widget declaration on both shells. -> {shell: {id: (st, msg)}}"""
    root = os.path.join(REPO, "target", "spec-fuzz-%d" % args.seed, "widget-check")
    os.makedirs(root, exist_ok=True)
    # The widget check is about DECLARATIONS, not text, so it runs in the
    # pool's first locale rather than drawing one — the same environment on
    # both shells, which is all this check needs.
    env = child_env(root, args.locale_pool[0] if args.locale_pool else "C")
    fp = " ".join(shlex.quote(p) for p in args.fpath_dirs)
    plans = {w: widget_plan(w) for w in ids}

    head = ["fpath=( %s )" % fp, "zmodload zsh/zle",
            "autoload -Uz compinit", "compinit -u -D"] + _probe_body(SFZ_FN)

    # Phase A — everything declared from the init file. One shell, in order:
    # each block is verified immediately after it is declared, so later blocks
    # rebinding the same key cannot make an earlier one look installed.
    inline = [w for w in ids if plans[w]["header"] is None and plans[w]["decl"]]
    script_a = "\n".join(head + [ln for w in inline
                                 for ln in _check_block(w, plans[w], SFZ_FN)])

    # Phase B — the `#compdef -k` / `-K` FILE headers. compinit only reads them
    # while it scans $fpath, so each needs its own fpath and its own shell.
    filed = [w for w in ids if plans[w]["header"] is not None]
    scripts_b = {}
    for w in filed:
        d = os.path.join(root, re.sub(r"[^\w.-]", "_", w))
        os.makedirs(d, exist_ok=True)
        with _wopen(os.path.join(d, "_sfzprobe")) as f:
            f.write("%s\n\ncompadd sfzprobe\n" % plans[w]["header"])
        scripts_b[w] = "\n".join(
            ["fpath=( %s %s )" % (shlex.quote(d), fp), "zmodload zsh/zle",
             "print -r -- '@@B %s'" % w,
             "autoload -Uz compinit", "compinit -u -D"] +
            ["print -r -- \"@@RC %s $?\"" % w] +
            _check_block(w, dict(plans[w], pre=[], decl=[]), SFZ_FN)[1:])

    out = {}
    for label, argv in (("zsh", [args.zsh]), ("zshrs", args.test_base)):
        res = {}
        if inline:
            text, err = _run_check(argv, env, root, script_a, args.widget_check_timeout)
            if err:
                res.update({w: (REJECTED, err) for w in inline})
            else:
                res.update(_parse_check(text, inline, plans))
        for w in filed:
            text, err = _run_check(argv, env, root, scripts_b[w],
                                   args.widget_check_timeout)
            if err:
                res[w] = (REJECTED, err)
            else:
                res.update(_parse_check(text, [w], plans, filefn="_sfzprobe"))
        for w in ids:
            res.setdefault(w, (OK, "no declaration to check"))
        out[label] = res
    return out


def print_widget_check(report, ids):
    """-> (ids real zsh rejected, ids only zshrs rejected)"""
    zsh, zshrs = report["zsh"], report["zshrs"]
    bad_ref = [w for w in ids if zsh[w][0] != OK]
    bad_test = [w for w in ids if zsh[w][0] == OK and zshrs[w][0] != OK]
    print("# widget self-check — every declaration run on BOTH shells")
    print("#   %-30s %-10s %s" % ("id", "zsh", "zshrs"))
    for w in ids:
        a, b = zsh[w], zshrs[w]
        if a[0] == OK and b[0] == OK:
            continue
        print("#   %-30s %-10s %s" % (w, a[0], b[0]))
        if a[1]:
            print("#     zsh  : %s" % a[1])
        if b[1]:
            print("#     zshrs: %s" % b[1])
    print("# %d declaration(s) checked: %d rejected by real zsh, "
          "%d accepted by zsh but not by zshrs"
          % (len(ids), len(bad_ref), len(bad_test)))
    if bad_ref:
        print("#   generator-rejected (dropped from the pool — a binding real "
              "zsh refuses is a generator bug, not a finding):")
        for w in bad_ref:
            print("#     %-28s %s" % (w, zsh[w][1][:90]))
    if bad_test:
        print("#   widget-decl-divergence (KEPT in the pool — real zsh "
              "installed these and zshrs did not):")
        for w in bad_test:
            print("#     %-28s %s" % (w, zshrs[w][1][:90]))
    sys.stdout.flush()
    return bad_ref, bad_test


# ═════════════════════════════════════════════════════════════════════════════
# reporting
# ═════════════════════════════════════════════════════════════════════════════

def print_case_spec(case):
    print("  cmd      : %s   kind=%s   widget=%s"
          % (case.cmd, case.kind, case.extra.get("widget", "default")))
    print("  env      : locale=%s   cols=%d%s"
          % (case.locale, case.cols,
             "   hostile=" + " ".join("%s(%s)" % (n, c) for n, c
                                      in case.extra["hostile_used"])
             if case.extra.get("hostile_used") else ""))
    print("  buffer   : %s   keys=%s"
          % (disp(repr(case.buffer)), ",".join(case.keys)))
    for a in case.atoms:
        c = SUBSTRATE_BY_ID.get(a)
        if c is not None:
            print("  construct: %-24s %-9s %s" % (c.id, c.category, c.cite))
    # The init lines are part of the case now: with a `zle -C` / `compdef -k`
    # widget the function under test lives here, not in the fpath file.
    for ln in case.init_lines():
        print("  init %s" % disp(ln))
    for ln in case.completer().rstrip("\n").split("\n"):
        print("  | %s" % disp(ln))


def print_failure(v, args):
    c = v.case
    print()
    print("=" * 78)
    print("%s %s — %s" % (v.status, c.name, disp(v.detail)))
    print("=" * 78)
    print_case_spec(c)
    m = getattr(v, "minimal", None)
    if m is not None and m.completer() != c.completer():
        print("  --- reduced to (%d spec(s), %d flag(s), %d probes) ---"
              % v.shrunk)
        for ln in m.completer().rstrip("\n").split("\n"):
            print("  > %s" % disp(ln))
    ref, test = v.ref, v.test
    if ref is None or test is None:
        # The only shape with no capture at all is a skipped case whose locale
        # this host does not have; there is nothing to render, and saying so is
        # the report.
        print("  (no capture — %s)" % v.category)
        sys.stdout.flush()
        return
    for label, cap in (("zsh", ref), ("zshrs", test)):
        for w in cap.warnings():
            print("  ! %-5s %s" % (label, w))
    if ref.grid is not None and test.grid is not None:
        if v.diffs:
            i, a, b = v.diffs[0]
            col = first_diff_cell(a, b)
            print("  first differing cell: row %d col %d" % (i, col))
            print("    zsh   | %s" % a)
            print("    zshrs | %s" % b)
            print("          | %s^" % (" " * col))
        print("  --- zsh grid ---")
        print(render(ref.grid))
        print("  --- zshrs grid ---")
        print(render(test.grid))
        if v.diffs and len(v.diffs) > 1:
            print("  --- differing rows (%d) ---" % len(v.diffs))
            for i, a, b in v.diffs:
                print("   %2d zsh   | %s" % (i, a))
                print("   %2d zshrs | %s" % (i, b))
        if v.cursor_differs:
            print("  cursor: zsh %s   zshrs %s" % (ref.cursor, test.cursor))
        if v.attr_rows:
            print("  rows differing in SGR attributes only: %s" % v.attr_rows)
    # When a second reference ran this case, print ITS grid too. A
    # "version-split" claim is only worth as much as the third screen behind
    # it, so the screen is shown rather than summarised.
    if v.dev is not None and v.dev.grid is not None:
        print("  --- zsh DEV grid (%s) ---" % args.zsh_dev_version)
        print(render(v.dev.grid))
        if v.split_class == SPLIT:
            print("  -> the two REFERENCES disagree on this case. zshrs cannot "
                  "match both; it currently matches: %s" % v.split_side)
            print("     The FAIL above is still measured against %s."
                  % args.zsh)
        elif v.split_class == ZSHRS_BUG:
            print("  -> both references rendered the SAME screen. This is a "
                  "zshrs bug, not a version question.")
    if v.split_note:
        print("  -> %s" % v.split_note)
    if v.ref_only_diags:
        print("  only zsh reported : %s" % "; ".join(sorted(v.ref_only_diags)))
    if v.test_only_diags:
        print("  only zshrs report : %s" % "; ".join(sorted(v.test_only_diags)))
    if getattr(v, "ref2", None) is not None and v.status == SKIP:
        print("  --- second zsh grid (self-check) ---")
        print(render(v.ref2.grid) if v.ref2.grid is not None else "  <no grid>")
    if ref.raw and test.raw and ref.raw != test.raw:
        sd = stream_diff(ref.raw, test.raw, args.raw_diff_lines)
        if sd:
            print("  --- raw escape-stream diff ---")
            for ln in sd:
                print(ln)
    if v.attribution:
        print("  --- standalone attribution (each surviving construct re-run "
              "OUTSIDE completion, on both shells) ---")
        for c, verdict, rv, tv, err in v.attribution:
            print("  %-14s %-24s %-9s %s"
                  % (verdict, c.id, c.category, c.cite))
            if verdict == "core-language":
                print("      zsh   : %s" % disp(bdec(rv))[:150])
                print("      zshrs : %s"
                      % (disp(bdec(tv))[:150] if tv is not None
                         else "<no probe line: %s>" % (err or "?")))
                print("      repro : %s" % substrate_repro_cmd(c))
            elif verdict == "rejected":
                print("      the reference shell refused it here: %s"
                      % (err or "?"))
        core = [c.id for c, k, _r, _t, _e in v.attribution
                if k == "core-language"]
        comp = [c.id for c, k, _r, _t, _e in v.attribution if k == "completion"]
        if core:
            print("  => CORE-LANGUAGE bug(s): %s  (reproduce with the `repro` "
                  "line above; no completion needed)" % " ".join(core))
        if comp and not core:
            print("  => COMPLETION bug: every surviving construct agrees "
                  "OUTSIDE completion (%s), so the language is not what broke "
                  "— replay the fixture" % " ".join(comp))
    if v.fixture:
        print("  fixture  : %s" % os.path.relpath(v.fixture, REPO))
        print("  replay   : %s --replay %s"
              % (SELF, os.path.relpath(v.fixture, REPO)))
    sys.stdout.flush()


def to_json(v):
    def side(c):
        return {
            "grid": c.grid,
            "reason": c.reason,
            "crash": c.crash,
            "cursor": list(c.cursor) if c.cursor else None,
            "diags": sorted(c.diags),
            "warnings": c.warnings(),
        }
    return {
        "case": v.case.name,
        "seed": v.case.seed,
        "index": v.case.idx,
        "kind": v.case.kind,
        "widget": v.case.extra.get("widget", "default"),
        "compstate_probe": bool(v.case.extra.get("compstate_probe")),
        "cmd": v.case.cmd,
        # JSON cannot carry a lone surrogate, so generated text reaches the
        # report in its escaped form. The FIXTURE keeps the real bytes; this
        # is the readable copy, and it says so by being escaped.
        "locale": v.case.locale,
        "cols": v.case.cols,
        "hostile": [{"name": n, "category": c}
                    for n, c in v.case.extra.get("hostile_used", [])],
        "width_probe": bool(v.case.extra.get("width_texts")),
        "constructs": [{"id": c.id, "category": c.category, "cite": c.cite}
                       for c in (SUBSTRATE_BY_ID[a] for a in v.case.atoms
                                 if a in SUBSTRATE_BY_ID)],
        "attribution": [{"id": c.id, "verdict": k,
                         "zsh": disp(bdec(r)) if r is not None else None,
                         "zshrs": disp(bdec(t)) if t is not None else None,
                         "repro": substrate_repro_cmd(c),
                         "error": e}
                        for c, k, r, t, e in v.attribution],
        "split_class": v.split_class,
        "split_side": v.split_side,
        "split_note": v.split_note,
        "dev": side(v.dev) if v.dev else None,
        "buffer": disp(v.case.buffer),
        "keys": v.case.keys,
        "init": [disp(x) for x in v.case.init_lines()],
        "completer": disp(v.case.completer()),
        "status": v.status,
        "category": v.category,
        "detail": disp(v.detail),
        "stream_only": v.stream_only,
        "fixture": v.fixture,
        "rows_differing": len(v.diffs),
        "ref": side(v.ref) if v.ref else None,
        "test": side(v.test) if v.test else None,
        "timing": {"seconds": round(v.duration, 2)},
    }


# ═════════════════════════════════════════════════════════════════════════════

def main():
    ap = argparse.ArgumentParser(
        description="generative, hermetic compsys completion-parity fuzzer")
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--cases", type=int, default=8)
    ap.add_argument("--start", type=int, default=0,
                    help="first case index (with --seed, fixes the generation)")
    ap.add_argument("--zshrs", default=os.path.join(REPO, "target", "debug", "zshrs"))
    ap.add_argument("--zsh", default="zsh")
    ap.add_argument("--mode", choices=("zsh", "native"), default="zsh",
                    help="zsh: run the binary as `zshrs --zsh` (default; the "
                         "emulation path these specs target). native: run it "
                         "as-is.")
    ap.add_argument("--rows", type=int, default=24)
    ap.add_argument("--cols", default=None,
                    help="terminal width, or `auto` to draw one per case from "
                         "%s — widths at which a double-width glyph straddles "
                         "the last column and the wrap point. BOTH shells "
                         "always get the same width."
                         % ",".join(str(c) for c in sorted(set(COLS_POOL))))
    ap.add_argument("--locale", default="auto",
                    help="the locale BOTH shells run in. `auto` (default) "
                         "draws one per case from every candidate this host "
                         "has and the reference shell honours; `all` uses the "
                         "same pool; or a comma-separated list of: "
                         + ",".join(l for l, _f, _c, _w in LOCALE_CANDIDATES))
    ap.add_argument("--check-locales", action="store_true",
                    help="run only the locale availability probe and the "
                         "generated-text self-check, print both, and exit")
    ap.add_argument("--hostile", action="store_true",
                    help="widen the generated description / display-string / "
                         "match alphabet to text built to break width and "
                         "encoding math, in categories: "
                         + ",".join(HOSTILE_CATEGORIES))
    ap.add_argument("--no-hostile", dest="hostile", action="store_false",
                    help="keep the ASCII-only alphabet")
    ap.add_argument("--hostile-categories", default="all",
                    help="restrict --hostile to these categories (comma "
                         "separated). A generation-scope control, NOT a "
                         "comparison one: whatever is generated is still "
                         "compared in full, and every excluded category is "
                         "named in the summary as untested. Use it to get "
                         "past a category whose divergence is so coarse it "
                         "hides the others — `invalid` makes a shell that "
                         "cannot read the file fail every case in it. "
                         "Categories: " + ",".join(HOSTILE_CATEGORIES))
    ap.add_argument("--width-probe", choices=("auto", "always", "never"),
                    default="auto",
                    help="echo ${#text} (characters) and ${(m)#text} "
                         "(display COLUMNS) for every hostile string a case "
                         "used, as matches, so a width bug is caught even "
                         "when the layout it produced happens to line up. "
                         "auto: ~60%% of the cases that used one.")
    ap.add_argument("--text-check-timeout", type=float, default=60.0)
    ap.add_argument("--keys", default="auto",
                    help="comma-separated key PATH sent after the buffer, or "
                         "`auto` (default) to let each case draw its own path "
                         "from KEY_PATHS — menu start, cycling, a filter "
                         "letter, an abort. Names: " + ",".join(sorted(KEYS)))
    ap.add_argument("--kind", default="all",
                    help="comma-separated: arguments,values,describe,"
                         "alternative,compadd,compset,tags,nested,substrate")
    ap.add_argument("--substrate-count", type=int, default=4,
                    help="how many language constructs one --kind substrate "
                         "case runs (default 4). Each is drawn from a "
                         "different category where the pool allows it.")
    ap.add_argument("--substrate-categories", default="all",
                    help="restrict --kind substrate to these construct "
                         "categories (comma separated). A generation-scope "
                         "control, NOT a comparison one: whatever is generated "
                         "is still compared in full, and every excluded "
                         "category is named in the summary as untested. "
                         "Categories: " + ",".join(SUBSTRATE_CATEGORIES))
    ap.add_argument("--check-substrate", action="store_true",
                    help="run only the substrate self-check — every catalogue "
                         "fragment STANDALONE on both shells, in every usable "
                         "locale — print the table, and exit. A construct real "
                         "zsh rejects is a GENERATOR bug and is counted as "
                         "one; a construct that already differs outside "
                         "completion is a core-language bug and is named.")
    ap.add_argument("--list-substrate", action="store_true",
                    help="print the construct catalogue with the stock "
                         "Completion/... citation that justifies each entry, "
                         "and exit")
    ap.add_argument("--substrate-timeout", type=float, default=30.0)
    ap.add_argument("--widget", default="auto",
                    help="which WIDGET the completion is reached through. "
                         "`auto` (default) draws one per case from the whole "
                         "pool, `default` keeps the round-2 behaviour (whatever "
                         "compinit left on TAB), or a comma-separated list of "
                         "ids: " + ",".join(WIDGET_IDS))
    ap.add_argument("--list-widgets", action="store_true",
                    help="print every widget id with the declaration and the "
                         "key it is fired by, and exit")
    ap.add_argument("--check-widgets", action="store_true",
                    help="run only the widget self-check (every declaration on "
                         "both shells) and exit")
    ap.add_argument("--no-widget-check", dest="widget_check",
                    action="store_false", default=True,
                    help="skip the pre-run widget self-check. A binding real "
                         "zsh rejects then reaches the fuzzer as a bogus FAIL, "
                         "so this is for iterating, not for reporting.")
    ap.add_argument("--widget-check-timeout", type=float, default=120.0)
    ap.add_argument("--compstate-probe", choices=("auto", "always", "never"),
                    default="auto",
                    help="echo the widget-visible $compstate fields (%s) into "
                         "the completion listing so a divergence in the STATE "
                         "is visible even when the rendered list matches. "
                         "auto: ~35%%%% of cases."
                         % ",".join(Case.COMPSTATE_KEYS))
    ap.add_argument("--jobs", type=int, default=1)
    ap.add_argument("--json", default=None)
    ap.add_argument("--verbose", action="store_true",
                    help="print every generated completer, not just failures")
    ap.add_argument("--replay", default=None,
                    help="re-run a saved fixture. An explicit --keys re-judges "
                         "it over a different key path, which is how a "
                         "multi-key divergence gets localised to one press.")
    ap.add_argument("--spec", action="append", default=None,
                    help="inject a literal _arguments spec atom (repeatable) "
                         "instead of generating; runs as one ad-hoc case")
    ap.add_argument("--spec-buffer", default=None,
                    help="buffer to type for --spec (default '<cmd> ')")
    ap.add_argument("--non-ascii", action="store_true",
                    help="add the four benign non-ASCII descriptions to the "
                         "ASCII pool. This is the round-2 flag and it is NOT "
                         "the locale axis: it no longer changes the locale "
                         "(that is --locale) and it is not the hostile-text "
                         "corpus (that is --hostile).")
    ap.add_argument("--settle", type=int, default=400,
                    help="ms of quiet before a screen is considered final")
    ap.add_argument("--boot-timeout", type=float, default=60.0)
    ap.add_argument("--shrink", dest="shrink", action="store_true", default=True)
    ap.add_argument("--no-shrink", dest="shrink", action="store_false")
    ap.add_argument("--shrink-probes", type=int, default=40)
    ap.add_argument("--self-check", dest="self_check", action="store_true", default=True,
                    help="on a failure, re-run real zsh against itself; a "
                         "reference that disagrees with itself makes the case "
                         "SKIP(unstable-reference) instead of a false FAIL")
    ap.add_argument("--no-self-check", dest="self_check", action="store_false")
    ap.add_argument("--strict-stream", action="store_true",
                    help="fail a case whose grids match but whose raw escape "
                         "streams differ (always reported either way)")
    ap.add_argument("--strict-cursor", action="store_true")
    ap.add_argument("--compare-attrs", action="store_true",
                    help="fail on SGR-attribute-only differences too")
    ap.add_argument("--raw-diff-lines", type=int, default=30)

    # ── the version-split layer: a SECOND reference ──────────────────────────
    ap.add_argument("--split-scan", action="store_true",
                    help="run the three-way VERSION-SPLIT scan and exit. Every "
                         "probe runs on BOTH reference zshs and on zshrs, and "
                         "is classified: the two references agree and zshrs "
                         "differs (a zshrs bug), the two references disagree "
                         "(a version split — zshrs cannot match both and which "
                         "side it takes is a policy decision), or all three "
                         "agree. Needs --zsh-dev or --build-dev-zsh.")
    ap.add_argument("--zsh-dev", default=None,
                    help="the SECOND reference: a built zsh from the "
                         "development tree the ports are written from. Must "
                         "report a different $ZSH_VERSION from --zsh, or it is "
                         "refused — a second reference of the same version is "
                         "not a second reference.")
    ap.add_argument("--build-dev-zsh", action="store_true",
                    help="if no usable --zsh-dev is found, BUILD one: copy "
                         "--dev-tree into target/spec-fuzz-devzsh (the tree "
                         "itself is read only), configure it at a prefix "
                         "inside that copy, make, make install.modules "
                         "(a full `make install` also builds the man pages "
                         "and fails on a host without yodl). The prefix "
                         "matters: it makes the built shell's compiled-in "
                         "module_path point at its own modules, so all three "
                         "shells run the byte-identical probe with no "
                         "per-shell preamble.")
    ap.add_argument("--dev-tree", default=DEV_TREE_DEFAULT,
                    help="the zsh source tree the ports are read from "
                         "(default %(default)s). Read only; only ever copied.")
    ap.add_argument("--split-cases", type=int, default=24,
                    help="how many probes to GENERATE, on top of the seeded "
                         "already-confirmed splits")
    ap.add_argument("--split-start", type=int, default=0)
    ap.add_argument("--split-families", default="all",
                    help="comma-separated: " + ",".join(SPLIT_FAMILIES)
                         + ". Excluded families are named in the summary as "
                           "untested.")
    ap.add_argument("--no-split-seeds", dest="split_seeds",
                    action="store_false", default=True,
                    help="drop the four already-confirmed splits from the "
                         "scan. They are the VALIDATION target — a scan that "
                         "does not classify them as splits is not working — so "
                         "this is for iterating, not for reporting.")
    ap.add_argument("--split-json", default=None)
    args = ap.parse_args()

    # Resolve both shells to absolute paths BEFORE anything execs them: the
    # child environment is built from scratch with a fixed PATH, and this host
    # has a second, older zsh at /bin/zsh that a bare name would silently
    # select on one side of the comparison.
    zsh_abs = shutil.which(args.zsh)
    if not zsh_abs:
        sys.exit("compsys_spec_fuzz: %s not found on PATH" % args.zsh)
    args.zsh = os.path.abspath(zsh_abs)
    args.zshrs = os.path.abspath(args.zshrs)
    if not os.path.exists(args.zshrs):
        sys.exit("compsys_spec_fuzz: %s not found (build it, or pass --zshrs)"
                 % args.zshrs)
    args.test_base = ([args.zshrs] if args.mode == "native"
                      else [args.zshrs, "--zsh"])
    args.test_argv = args.test_base + ["-f", "-i"]

    # The release reference's own identity, read from it rather than assumed:
    # it is what the dev candidate has to differ from, and it is printed with
    # every finding so a reader knows which zsh a number was measured against.
    args.zsh_version = _zsh_version(args.zsh)
    args.dev_tree = os.path.expanduser(args.dev_tree)
    args.zsh_dev_version = args.zsh_dev_note = None
    if args.split_families.strip() in ("all", ""):
        args.split_families = list(SPLIT_FAMILIES)
    else:
        args.split_families = [f.strip()
                               for f in args.split_families.split(",")
                               if f.strip()]
        for f in args.split_families:
            if f not in SPLIT_FAMILIES:
                sys.exit("compsys_spec_fuzz: unknown --split-families %r (have "
                         "all, or: %s)" % (f, ",".join(SPLIT_FAMILIES)))
    if args.split_scan or args.zsh_dev or args.build_dev_zsh:
        dev, dver, note = resolve_dev_zsh(args)
        if not dev:
            msg = ("compsys_spec_fuzz: no usable SECOND reference zsh.\n#   %s"
                   % note)
            if args.split_scan:
                sys.exit(msg + "\n"
                         "\nThe split scan compares TWO reference zshs, and "
                         "cannot run on one.\nBuild the second one with:\n"
                         "  %s --split-scan --build-dev-zsh\n"
                         "or point --zsh-dev at an existing built "
                         "<tree>/Src/zsh." % SELF)
            print(msg)
            print("#   continuing WITHOUT the three-way classification: every "
                  "verdict below is measured against %s alone" % args.zsh)
            args.zsh_dev = None
        else:
            # ABSOLUTE, always: every probe runs with cwd set to the case's
            # own work directory, so a relative binary path would not resolve
            # there — and the failure would look like "the dev reference
            # crashed" rather than "the path was relative".
            args.zsh_dev = os.path.abspath(dev)
            args.zsh_dev_version, args.zsh_dev_note = dver, note

    # `auto` leaves args.keys empty and lets each case draw its own key PATH
    # from KEY_PATHS, so a generated spec is judged over a sequence (menu
    # start, cycling, a filter letter, an abort) instead of one press.
    if args.keys.strip() == "auto":
        args.keys = []
    else:
        args.keys = [k for k in args.keys.split(",") if k]
        for k in args.keys:
            try:
                key_bytes(k)             # fail loudly on a typo, not silently
            except UnknownKey:
                sys.exit("compsys_spec_fuzz: unknown key %r in --keys (have "
                         "`auto`, any single character, or: %s)"
                         % (k, ",".join(sorted(KEYS))))
    all_kinds = ["arguments", "values", "describe", "alternative",
                 "compadd", "compset", "tags", "nested", "substrate"]
    args.kinds = all_kinds if args.kind == "all" else [
        k.strip() for k in args.kind.split(",") if k.strip()]
    for k in args.kinds:
        if k not in all_kinds:
            sys.exit("compsys_spec_fuzz: unknown --kind %r (have %s)"
                     % (k, ",".join(all_kinds)))

    if args.widget.strip() in ("auto", "all"):
        args.widget_pool = list(WIDGET_IDS)
    elif args.widget.strip() == "default":
        args.widget_pool = ["default"]
    else:
        args.widget_pool = [w.strip() for w in args.widget.split(",") if w.strip()]
        for w in args.widget_pool:
            if w not in WIDGET_IDS:
                sys.exit("compsys_spec_fuzz: unknown --widget %r (have auto, "
                         "default, or: %s)" % (w, ",".join(WIDGET_IDS)))

    # ── the environment axes ─────────────────────────────────────────────────
    known_locales = [l for l, _f, _c, _w in LOCALE_CANDIDATES]
    # A fixture records the locale its divergence happened in, and a replay
    # that ran in a different one would not be a replay. Peek at the header
    # before the probe so that locale is among the ones proved available.
    replay_locale = None
    if args.replay:
        try:
            with open(args.replay, encoding="utf-8", errors="surrogateescape") as fh:
                m = re.search(r"^#\s*@locale\s+(\S+)$", fh.read(), re.M)
            replay_locale = m.group(1) if m else None
        except OSError:
            replay_locale = None
    if args.locale.strip() in ("auto", "all"):
        want_locales = list(known_locales)
    else:
        want_locales = [l.strip() for l in args.locale.split(",") if l.strip()]
        for l in want_locales:
            if l not in known_locales:
                sys.exit("compsys_spec_fuzz: unknown --locale %r (have auto, "
                         "all, or: %s)" % (l, ",".join(known_locales)))
    # What the USER asked for, before the replayed fixture's own locale is
    # added to the probe set. A replay override has to be decided from this,
    # not from the probe set: appending the fixture's locale would otherwise
    # make an explicit single `--locale C` look like a two-locale request and
    # silently ignore it.
    explicit_locales = ([] if args.locale.strip() in ("auto", "all")
                        else list(want_locales))
    if replay_locale and replay_locale not in want_locales:
        want_locales.append(replay_locale)
    # `None` means the flag was not given, which is not the same as being
    # given the default: on a replay an explicit `--cols 80` re-judges the
    # fixture at 80, and an absent one keeps the width the fixture recorded.
    cols_explicit = args.cols is not None
    if cols_explicit and args.cols.strip() == "auto":
        args.cols_pool = list(COLS_POOL)
        args.cols = COLS_POOL[0]
    else:
        try:
            args.cols = int(args.cols) if cols_explicit else 80
        except ValueError:
            sys.exit("compsys_spec_fuzz: --cols takes a number or `auto`")
        args.cols_pool = []

    if args.list_substrate:
        for cat in SUBSTRATE_CATEGORIES:
            print("# %s" % cat)
            for c in SUBSTRATE:
                if c.category == cat:
                    print("  %-24s %s" % (c.id, c.cite))
        print("# %d construct(s) in %d categor(ies)"
              % (len(SUBSTRATE), len(SUBSTRATE_CATEGORIES)))
        return 0

    if args.list_widgets:
        for w in WIDGET_IDS:
            p = widget_plan(w)
            print("%-30s fire=%-9s prime=%-6s %s"
                  % (w, p["fire"], ",".join(p["prime"]) or "-",
                     "; ".join(p["pre"] + p["decl"]) or
                     ("header %s" % p["header"] if p["header"] else
                      "(no binding — whatever compinit left on TAB)")))
        return 0

    args.fpath_dirs = hermetic_fpath(args.zsh)

    # ── locale availability, PROVED before anything is generated ─────────────
    candidates = [c for c in LOCALE_CANDIDATES if c[0] in want_locales]
    if replay_locale and replay_locale not in known_locales:
        # A fixture from a host with a locale this table does not list. Probe
        # it anyway, by the family its name claims — if the shell disagrees the
        # probe says so and the replay skips, which is the honest outcome.
        fam = "utf8" if replay_locale.upper().endswith("UTF-8") else "single"
        candidates.append((replay_locale, fam,
                           "utf-8" if fam == "utf8" else "latin-1",
                           "carried by the fixture being replayed"))
    args.locale_ok, locale_skipped = probe_locales(args, candidates)
    locale_test = probe_locales_test_side(args, list(args.locale_ok))
    locale_bad = print_locale_check(args.locale_ok, locale_skipped, locale_test,
                                    want_locales)
    args.locale_why = {l: why for l, _f, why in locale_skipped}
    args.locale_pool = [l for l in want_locales if l in args.locale_ok]
    if not args.locale_pool:
        sys.exit("compsys_spec_fuzz: none of the requested locales is usable "
                 "on this host — nothing to generate")
    print()

    # ── the three-way version-split scan ─────────────────────────────────────
    # Placed after the locale probe because a probe run in a locale the
    # reference does not honour is not a probe, and before everything else
    # because the scan has nothing to do with the widget or hostile-text axes.
    if args.split_scan:
        return split_scan(args)

    # ── generated-text self-check ────────────────────────────────────────────
    if args.hostile_categories.strip() in ("all", ""):
        want_cats = list(HOSTILE_CATEGORIES)
    else:
        want_cats = [c.strip() for c in args.hostile_categories.split(",")
                     if c.strip()]
        for c in want_cats:
            if c not in HOSTILE_CATEGORIES:
                sys.exit("compsys_spec_fuzz: unknown --hostile-categories %r "
                         "(have all, or: %s)" % (c, ",".join(HOSTILE_CATEGORIES)))
    args.hostile_cats = want_cats
    entries = [h for h in HOSTILE if h.category in want_cats]

    args.text_pool, text_report, text_rejected = {}, {}, 0
    if args.hostile or args.check_locales:
        args.text_pool, text_report = check_generated_text(
            args, args.locale_pool, entries)
        text_rejected = print_text_check(text_report, entries)
        excluded = sorted(set(HOSTILE_CATEGORIES) - set(want_cats))
        if excluded:
            print("# NOT generated by this run (--hostile-categories), so NOT "
                  "tested by it: %s" % " ".join(excluded))
        print()
    if args.check_locales:
        return 1 if (locale_bad or text_rejected) else 0

    # ── substrate self-check ─────────────────────────────────────────────────
    if args.substrate_categories.strip() in ("all", ""):
        want_sub = list(SUBSTRATE_CATEGORIES)
    else:
        want_sub = [c.strip() for c in args.substrate_categories.split(",")
                    if c.strip()]
        for c in want_sub:
            if c not in SUBSTRATE_CATEGORIES:
                sys.exit("compsys_spec_fuzz: unknown --substrate-categories %r "
                         "(have all, or: %s)"
                         % (c, ",".join(SUBSTRATE_CATEGORIES)))
    args.substrate_cats = want_sub
    args.substrate_pool = [c for c in SUBSTRATE if c.category in want_sub]
    sub_rejected, sub_core = [], []
    if args.check_substrate or ("substrate" in args.kinds and not args.replay
                                and not args.spec):
        wanted = args.substrate_pool
        args.substrate_pool, sub_report = check_substrate(args, wanted)
        sub_rejected, sub_core = print_substrate_check(sub_report, wanted)
        excluded = [c for c in SUBSTRATE_CATEGORIES if c not in want_sub]
        if excluded:
            print("#   categor(ies) this run EXCLUDED "
                  "(--substrate-categories), so NOT tested: %s"
                  % " ".join(excluded))
        print()
        if args.check_substrate:
            return 1 if sub_rejected else 0
        if not args.substrate_pool:
            sys.exit("compsys_spec_fuzz: real zsh rejected EVERY substrate "
                     "construct in scope — nothing left to generate")

    # ── widget self-check ────────────────────────────────────────────────────
    widget_bad_ref, widget_bad_test = [], []
    if args.check_widgets or (args.widget_check and args.widget_pool != ["default"]
                              and not args.replay and not args.spec):
        report = check_widgets(args, args.widget_pool)
        widget_bad_ref, widget_bad_test = print_widget_check(report, args.widget_pool)
        print()
        if args.check_widgets:
            return 1 if widget_bad_ref else 0
        # A declaration real zsh refuses cannot produce a meaningful case, so
        # it leaves the pool — loudly, above, never silently.
        args.widget_pool = [w for w in args.widget_pool if w not in widget_bad_ref]
        if not args.widget_pool:
            sys.exit("compsys_spec_fuzz: real zsh rejected EVERY requested "
                     "widget declaration — nothing left to generate")

    # ── case list ────────────────────────────────────────────────────────────
    if args.replay:
        cases = [read_fixture(args.replay)]
        args.seed = cases[0].seed
        if args.keys:
            # An explicit --keys re-judges the saved completer over a DIFFERENT
            # key path. That is how a multi-key divergence gets localised: run
            # the same fixture at tab, then tab,bs, then tab,bs,tab and see
            # which press first splits the two shells.
            cases[0].keys = list(args.keys)
        # …and an explicit single --locale / --cols re-judges it in a DIFFERENT
        # environment, which is how a divergence gets attributed to the locale
        # rather than to the spec: replay the same fixture at C and at UTF-8
        # and see which one splits the two shells.
        if len(explicit_locales) == 1:
            if explicit_locales[0] not in args.locale_ok:
                sys.exit("compsys_spec_fuzz: --locale %s is not usable on this "
                         "host, so the fixture cannot be re-judged in it"
                         % explicit_locales[0])
            cases[0].locale = explicit_locales[0]
        if cols_explicit and not args.cols_pool:
            cases[0].cols = args.cols
    elif args.spec:
        c = Case(-1, args.seed, "fzc9999", "arguments", args.spec, ["-s"],
                 {"helpers": [], "states": []},
                 args.spec_buffer or "fzc9999 ", args.keys or ["tab"],
                 locale=args.locale_pool[0],
                 cols=args.cols_pool[0] if args.cols_pool else args.cols)
        cases = [c]
    else:
        cases = [generate(args.seed, args.start + i, args)
                 for i in range(args.cases)]

    root = os.path.join(REPO, "target", "spec-fuzz-%d" % args.seed)
    os.makedirs(root, exist_ok=True)

    print("# compsys_spec_fuzz — generative hermetic completion parity")
    print("# zsh    : %s   $ZSH_VERSION %s" % (args.zsh, args.zsh_version))
    if getattr(args, "zsh_dev", None):
        print("# zsh dev: %s   $ZSH_VERSION %s   — every FAIL is ALSO run on "
              "this, and labelled: the two references agreeing means a zshrs "
              "bug, disagreeing means a version split"
              % (args.zsh_dev, args.zsh_dev_version))
    else:
        print("# zsh dev: none — every verdict below is measured against ONE "
              "reference, and a failure caused by a 5.9.2-vs-dev split cannot "
              "be told from a zshrs bug (pass --zsh-dev or --build-dev-zsh)")
    print("# zshrs  : %s" % " ".join(args.test_argv))
    print("# fpath  : %s" % " ".join(args.fpath_dirs))
    print("# seed   : %d   cases %s   keys=%s   rows=%d cols=%s   jobs=%d"
          % (args.seed,
             "%d..%d" % (args.start, args.start + len(cases) - 1)
             if cases else "<none>",
             ",".join(args.keys) if args.keys else "auto (per case)",
             args.rows,
             "auto (per case)" if args.cols_pool else str(args.cols),
             args.jobs))
    print("# locales: %s   hostile=%s%s"
          % (",".join(args.locale_pool),
             ("%d string(s) in %d categor(ies)"
              % (len(HOSTILE), len(HOSTILE_CATEGORIES))) if args.hostile
             else "off",
             "   width-probe=%s" % args.width_probe if args.hostile else ""))
    print("# kinds  : %s%s" % (",".join(args.kinds),
                               "   non-ascii" if args.non_ascii else ""))
    print("# widgets: %s   compstate-probe=%s"
          % ("%d id(s) (%s)" % (len(args.widget_pool), args.widget)
             if len(args.widget_pool) > 3 else ",".join(args.widget_pool),
             args.compstate_probe))
    print("# outdir : %s" % os.path.relpath(root, REPO))
    print()
    sys.stdout.flush()

    results = []
    lock = threading.Lock()

    def work(case):
        v = run_case(args, root, args.fpath_dirs, case)
        if args.replay:
            v.fixture = args.replay
        elif v.status == FAIL and args.shrink and v.category in SHRINKABLE:
            try:
                minimal, probes = shrink_case(args, root, args.fpath_dirs,
                                              case, v.category)
                v.shrunk = (len(minimal.atoms), len(minimal.flags), probes)
                v.minimal = minimal
                v.fixture = write_fixture(
                    os.path.join(root, "%s.min.zsh" % case.name), args, minimal,
                    "%s: %s (shrunk to %d spec(s), %d flag(s) in %d probes)"
                    % (v.category, v.detail, len(minimal.atoms),
                       len(minimal.flags), probes))
            except Exception as exc:
                print("  ! shrink failed for %s: %r" % (case.name, exc))
        elif v.status == FAIL and not args.replay:
            v.fixture = write_fixture(
                os.path.join(root, "%s.min.zsh" % case.name), args, case,
                "%s: %s (not shrunk)" % (v.category, v.detail))
        # A substrate failure is only actionable once it is known WHICH layer
        # broke, so every construct that survived the reduction is re-run
        # standalone on both shells. That produces the plain-shell reproducer
        # and, for the constructs that agree standalone, the proof that the
        # language is fine and the completion system is not.
        if v.status == FAIL and v.case.kind == "substrate":
            target = getattr(v, "minimal", None) or v.case
            try:
                v.attribution = substrate_attribute(args, root, target)
            except Exception as exc:
                print("  ! substrate attribution failed for %s: %r"
                      % (case.name, exc))
        with lock:
            results.append(v)
            mark = {PASS: "PASS", PASS_ERR: "PASS(err)", FAIL: "FAIL", SKIP: "SKIP"}[v.status]
            extra = ""
            if v.stream_only:
                extra = "  [raw escape streams differ]"
            if v.status == FAIL and getattr(v, "shrunk", None):
                extra = "  [shrunk to %d spec(s)/%d flag(s) in %d probes]" % v.shrunk
            if v.split_class == SPLIT:
                extra += "  [VERSION-SPLIT: the two references disagree; " \
                         "zshrs matches %s]" % v.split_side
            elif v.split_class == ZSHRS_BUG:
                extra += "  [both references agree — a zshrs bug]"
            if v.split_note:
                extra += "  [%s]" % v.split_note
            print("%-9s %-9s %-10s %-22s %-15s %-4s %-14s %s%s"
                  % (mark, v.case.name, v.case.kind,
                     v.case.extra.get("widget", "default")[:22],
                     v.case.locale[:15], v.case.cols,
                     ",".join(v.case.keys)[:14], disp(v.detail)[:40], extra))
            sys.stdout.flush()
            if args.verbose and v.status in (PASS, PASS_ERR):
                print_case_spec(v.case)
            if v.status in (FAIL, SKIP):
                print_failure(v, args)
        return v

    if args.jobs > 1:
        from concurrent.futures import ThreadPoolExecutor
        with ThreadPoolExecutor(max_workers=args.jobs) as ex:
            list(ex.map(work, cases))
    else:
        for c in cases:
            work(c)

    results.sort(key=lambda v: v.case.idx)

    npass = sum(1 for v in results if v.status == PASS)
    nerr = sum(1 for v in results if v.status == PASS_ERR)
    nfail = sum(1 for v in results if v.status == FAIL)
    nskip = sum(1 for v in results if v.status == SKIP)
    nstream = sum(1 for v in results if v.stream_only)
    nsplit = sum(1 for v in results if v.split_class == SPLIT)
    ntwobug = sum(1 for v in results if v.split_class == ZSHRS_BUG)

    print()
    print("=" * 78)
    print("# %d case(s): %d PASS, %d PASS(err), %d FAIL, %d SKIP"
          % (len(results), npass, nerr, nfail, nskip))

    # Per-KIND, because the four spec-level kinds and the four builtin-level
    # kinds test different layers: a run that is green only because it drew
    # nothing but `describe` cases has not tested `compadd`, and the totals
    # above cannot say so.
    per = {}
    for v in results:
        row = per.setdefault(v.case.kind, dict.fromkeys((PASS, PASS_ERR, FAIL, SKIP), 0))
        row[v.status] += 1
    print("#   by kind:      cases   PASS  PASS(err)   FAIL   SKIP")
    for kind in sorted(per):
        row = per[kind]
        print("#     %-12s %5d  %5d  %9d  %5d  %5d"
              % (kind, sum(row.values()), row[PASS], row[PASS_ERR],
                 row[FAIL], row[SKIP]))
    # Per-WIDGET, for the same reason as per-kind: the entry point decides
    # which dispatch inside the completion core runs, so a run that drew only
    # `default` has tested exactly one of them however green it looks.
    perw = {}
    for v in results:
        row = perw.setdefault(v.case.extra.get("widget", "default"),
                              dict.fromkeys((PASS, PASS_ERR, FAIL, SKIP), 0))
        row[v.status] += 1
    if len(perw) > 1 or set(perw) != {"default"}:
        print("#   by widget:    cases   PASS  PASS(err)   FAIL   SKIP")
        for w in sorted(perw):
            row = perw[w]
            print("#     %-30s %3d  %5d  %9d  %5d  %5d"
                  % (w, sum(row.values()), row[PASS], row[PASS_ERR],
                     row[FAIL], row[SKIP]))
    # Per-LOCALE, for the same reason as per-kind: the whole point of the axis
    # is that `C` and a UTF-8 locale run different code, so the totals above
    # cannot say whether the multibyte pathway was reached at all. A locale
    # whose whole row is SKIP was not tested, however green the run looks.
    perl_ = {}
    for v in results:
        row = perl_.setdefault(v.case.locale,
                               dict.fromkeys((PASS, PASS_ERR, FAIL, SKIP), 0))
        row[v.status] += 1
    print("#   by locale:    cases   PASS  PASS(err)   FAIL   SKIP   family")
    for loc in sorted(perl_):
        row = perl_[loc]
        fam = args.locale_ok.get(loc, {}).get("family", "unavailable")
        print("#     %-18s %5d  %5d  %9d  %5d  %5d   %s"
              % (loc, sum(row.values()), row[PASS], row[PASS_ERR],
                 row[FAIL], row[SKIP], fam))
    # Per terminal WIDTH, when the geometry axis was on: a wide-glyph bug is a
    # bug about a boundary, so which widths were actually visited is part of
    # what the run proved.
    if args.cols_pool:
        perc = {}
        for v in results:
            row = perc.setdefault(v.case.cols,
                                  dict.fromkeys((PASS, PASS_ERR, FAIL, SKIP), 0))
            row[v.status] += 1
        print("#   by cols:      cases   FAIL")
        for c in sorted(perc):
            print("#     %-18d %5d  %5d"
                  % (c, sum(perc[c].values()), perc[c][FAIL]))
    if args.hostile:
        percat = {}
        for v in results:
            for _n, cat in v.case.extra.get("hostile_used", []):
                row = percat.setdefault(cat, [0, 0])
                row[0] += 1
                row[1] += (v.status == FAIL)
        if percat:
            print("#   by hostile category:  uses   in FAILing cases")
            for cat in sorted(percat):
                print("#     %-20s %5d  %5d" % (cat, percat[cat][0], percat[cat][1]))
        off = sorted(set(HOSTILE_CATEGORIES) - set(args.hostile_cats))
        if off:
            print("#   hostile categor(ies) this run EXCLUDED "
                  "(--hostile-categories) — untested: %s" % " ".join(off))
        unused = sorted(set(args.hostile_cats) - set(percat))
        if unused:
            print("#   hostile categor(ies) in scope that NO case happened to "
                  "draw — also untested: %s" % " ".join(unused))
        nwp = sum(1 for v in results if v.case.extra.get("width_texts"))
        print("#   %d case(s) also compared ${#text} vs ${(m)#text} "
              "(characters vs display columns)" % nwp)
    # Per-CONSTRUCT, for the same reason as per-kind: a substrate run is only
    # evidence about the constructs it actually drew, and the totals cannot say
    # which those were. A construct with 0 uses was not tested.
    # A replayed fixture carries its completer as a verbatim body and no atom
    # list, so there is no per-construct coverage to report and printing an
    # "untested" list would name the whole catalogue — true of a one-case
    # replay and useless as a statement about it.
    subs = [] if args.replay else [v for v in results
                                   if v.case.kind == "substrate"]
    if subs:
        percon, percat = {}, {}
        for v in subs:
            for a in v.case.atoms:
                c = SUBSTRATE_BY_ID.get(a)
                if c is None:
                    continue
                row = percon.setdefault(a, [0, 0])
                row[0] += 1
                row[1] += (v.status == FAIL)
                crow = percat.setdefault(c.category, [0, 0])
                crow[0] += 1
                crow[1] += (v.status == FAIL)
        print("#   by substrate category:  uses   in FAILing cases")
        for cat in SUBSTRATE_CATEGORIES:
            if cat in percat:
                print("#     %-20s %5d  %5d" % (cat, percat[cat][0],
                                                percat[cat][1]))
        hit = [c for c in percon if percon[c][1]]
        if hit:
            print("#   construct(s) present in a FAILing case: %s"
                  % " ".join(sorted(hit)))
        untested = sorted(c.id for c in args.substrate_pool
                          if c.id not in percon)
        if untested:
            print("#   construct(s) in scope that NO case happened to draw — "
                  "untested by this run: %s" % " ".join(untested))
        offcat = [c for c in SUBSTRATE_CATEGORIES
                  if c not in args.substrate_cats]
        if offcat:
            print("#   substrate categor(ies) this run EXCLUDED — untested: %s"
                  % " ".join(offcat))
        core, comp = set(), set()
        for v in subs:
            for c, k, _r, _t, _e in v.attribution:
                (core if k == "core-language" else
                 comp if k == "completion" else set()).add(c.id)
        if core:
            print("#   CORE-LANGUAGE bug(s) — the construct differs with NO "
                  "completion involved: %s" % " ".join(sorted(core)))
        if comp - core:
            print("#   construct(s) that agreed standalone and were present in "
                  "a failing completion (COMPLETION-side): %s"
                  % " ".join(sorted(comp - core)))
    if sub_rejected:
        print("#   generator-rejected substrate construct(s), dropped before "
              "generating (real zsh refused the fragment): %d  (%s)"
              % (len(sub_rejected), " ".join(sub_rejected)))
    if sub_core:
        print("#   substrate construct(s) that ALREADY diverge outside "
              "completion (core-language): %d  (%s)"
              % (len(sub_core), " ".join(sub_core)))
    if locale_bad:
        print("#   locale(s) where zshrs read the canary differently from "
              "zsh: %d  (%s)" % (len(locale_bad), " ".join(locale_bad)))
    if text_report:
        td = {loc: r["test_diff"] for loc, r in text_report.items() if r["test_diff"]}
        if td:
            print("#   generated-text divergences (zsh represented the string, "
                  "zshrs did not agree):")
            for loc in sorted(td):
                print("#     %-18s %d  (%s)"
                      % (loc, len(td[loc]),
                         " ".join(sorted({n for n, _c, _w in td[loc]}))))
        if text_rejected:
            print("#   generator-rejected string(s) (never generated; a string "
                  "real zsh cannot represent is a generator bug): %d"
                  % text_rejected)
    ncs = sum(1 for v in results if v.case.extra.get("compstate_probe"))
    if ncs:
        print("#   %d case(s) also compared the widget-visible $compstate "
              "fields (%s)" % (ncs, ",".join(Case.COMPSTATE_KEYS)))
    if widget_bad_ref:
        print("#   generator-rejected widget id(s), dropped before generating: "
              "%d  (%s)" % (len(widget_bad_ref), " ".join(widget_bad_ref)))
    if widget_bad_test:
        print("#   widget-decl-divergence, declarations real zsh installed and "
              "zshrs did not: %d  (%s)"
              % (len(widget_bad_test), " ".join(widget_bad_test)))
    kpaths = {}
    for v in results:
        kpaths.setdefault(",".join(v.case.keys), []).append(v.status)
    if len(kpaths) > 1:
        print("#   by key path:  cases   FAIL")
        for k in sorted(kpaths):
            st = kpaths[k]
            print("#     %-24s %3d  %5d" % (k, len(st), st.count(FAIL)))
    if nerr:
        print("#   PASS(err) = the generated spec was malformed and BOTH shells")
        print("#   printed the identical compsys error. Counted apart from a")
        print("#   clean pass: a run that is all PASS(err) tested nothing.")
    if nstream:
        print("#   %d case(s) matched on screen but differ in the RAW escape "
              "stream" % nstream)
        for v in results:
            if v.stream_only:
                print("#     %s (%s)" % (v.case.name, v.case.kind))
        if not args.strict_stream:
            print("#   (reported only; --strict-stream makes these fail)")
    if nskip:
        print("#   skipped, by reason:")
        reasons = {}
        for v in results:
            if v.status == SKIP:
                reasons.setdefault(v.category, []).append(v.case.name)
        for r, names in sorted(reasons.items()):
            print("#     %-22s %d  (%s)" % (r, len(names), " ".join(names)))
    if getattr(args, "zsh_dev", None):
        print("#   second reference: %s ($ZSH_VERSION %s) — every FAIL above "
              "was ALSO run on it" % (args.zsh_dev, args.zsh_dev_version))
        print("#     %d failure(s) where the two references AGREE — zshrs "
              "bugs" % ntwobug)
        print("#     %d failure(s) where the two references DISAGREE — "
              "version splits, zshrs cannot match both:" % nsplit)
        for v in results:
            if v.split_class == SPLIT:
                print("#       %-10s %-10s zshrs matches %s"
                      % (v.case.name, v.case.kind, v.split_side))
        unclass = [v.case.name for v in results if v.split_unrun]
        if unclass:
            print("#     %d failure(s) the dev reference could not run, so "
                  "they are UNCLASSIFIED: %s"
                  % (len(unclass), " ".join(unclass)))
        also = [v.case.name for v in results
                if v.split_note and not v.split_unrun]
        if also:
            print("#     %d failure(s) where the two references painted the "
                  "SAME screen but wrote DIFFERENT escape streams — a split "
                  "that is not what failed the case: %s"
                  % (len(also), " ".join(also)))
        print("#   (the PASS/FAIL verdicts above are measured against %s "
              "alone and are unchanged by this)" % args.zsh)
    if nfail:
        print("#   failures, by class:")
        classes = {}
        for v in results:
            if v.status == FAIL:
                classes.setdefault(v.category, []).append(v.case.name)
        for cname, names in sorted(classes.items()):
            print("#     %-22s %d  (%s)" % (cname, len(names), " ".join(names)))
        print("#   fixtures under %s" % os.path.relpath(root, REPO))

    if args.json:
        with open(args.json, "w") as f:
            json.dump({
                "seed": args.seed,
                "cases": len(results),
                "pass": npass, "pass_err": nerr, "fail": nfail, "skip": nskip,
                "stream_only": nstream,
                "locales_used": sorted(perl_),
                "locales_available": {l: i for l, i in args.locale_ok.items()},
                "locales_skipped": [{"locale": l, "family": f, "why": w}
                                    for l, f, w in locale_skipped],
                "locale_canary_divergence": locale_bad,
                "hostile": args.hostile,
                "text_check": {loc: {"ok": r["ok"],
                                     "rejected": [list(x) for x in r["rejected"]],
                                     "test_diff": [list(x) for x in r["test_diff"]]}
                               for loc, r in text_report.items()},
                "substrate_catalogue": [
                    {"id": c.id, "category": c.category, "cite": c.cite}
                    for c in SUBSTRATE],
                "substrate_generator_rejected": sub_rejected,
                "substrate_core_language_divergent": sub_core,
                "widgets": sorted(perw),
                "widget_generator_rejected": widget_bad_ref,
                "widget_decl_divergence": widget_bad_test,
                "compstate_probed": ncs,
                "zsh": args.zsh,
                "zsh_version": args.zsh_version,
                "zsh_dev": getattr(args, "zsh_dev", None),
                "zsh_dev_version": args.zsh_dev_version,
                "version_splits": [v.case.name for v in results
                                   if v.split_class == SPLIT],
                "two_reference_bugs": [v.case.name for v in results
                                       if v.split_class == ZSHRS_BUG],
                "zshrs": args.test_argv,
                "results": [to_json(v) for v in results],
            }, f, indent=1)
        print("# json: %s" % args.json)

    bad = nfail + (nstream if args.strict_stream else 0)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())

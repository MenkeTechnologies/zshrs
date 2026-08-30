#!/usr/bin/env python3
"""compsys_parity.py — PTY-driven byte-for-byte completion parity harness.

Drives the reference shell (`zsh`) and `zshrs --zsh` side by side through a
real pseudo-terminal, sends completion keystrokes (TAB, arrows, ...), renders
each shell's terminal output with pyte, and diffs the resulting screen grids
cell-for-cell. A divergence is a concrete compsys bug with a reproducible
keystroke sequence.

Both shells are booted interactive against a throwaway ZDOTDIR/HOME so only the
harness's generated `.zshrc` runs — identical init on both sides:

    autoload -Uz compinit
    compinit -d <dump>

pointed at the same completion dump and fpath. That isolates the completion
*engine* difference from environment drift.

Usage:
    scripts/compsys_parity.py                      # run built-in case set
    scripts/compsys_parity.py --case 'git ' --tab  # one ad-hoc case
    scripts/compsys_parity.py --list               # list built-in cases
    scripts/compsys_parity.py --only cd_slash       # run one built-in case
    scripts/compsys_parity.py -v                    # show both grids on PASS too

Fuzz mode (`--random-combos N`) drives N randomised cells in lockstep and
diffs both screens after EVERY keystroke. Three independent axes are fuzzed:

    --random-combos N   zstyle subsets (always on in fuzz mode)
    --buffer-fuzz       the COMMAND LINE itself, from a documented surface set
                        (mid-word cursor, quotes, $var/${, globs, braces,
                        tildes, redirection targets, assignment RHS,
                        command-substitution interior, sudo-prefixed,
                        backslash-escaped spaces, partial long options)
    --geometry-fuzz     terminal (rows, cols) from a seeded pool that includes
                        narrow (40 cols), tiny-rows (6-8) and wide (200 cols)
    --edit-fuzz         the LINE EDITING that produced the buffer — see below
    --multiline-fuzz    the CONTINUATION context the completion fires in —
                        see below
    --fstree-fuzz       the FILESYSTEM the completion runs against — a seeded,
                        hermetic, deliberately hostile tree — see below
    --interrupt-fuzz    what INTERRUPTS the completion (SIGWINCH, SIGINT,
                        type-ahead) — see below

and a failure is delta-debugged down to the minimal edit program, keystroke
path and zstyle subset that still diverge at the SAME first-diff cell:

    --shrink-probes N   probe budget per axis (0 disables shrinking)
    --jobs N            run N cells concurrently (independent pty pairs)
    --json PATH         machine-readable result document ('-' for stdout)

Every failure prints a copy-pasteable `--lockstep` replay carrying the seed,
buffer, edit program, editing mode, minimal key path, geometry and the saved
zstyle fixture.

Edit fuzz (`--edit-fuzz`)
    Every other case in this file (and in every sibling harness) types a buffer
    left to right and then completes. Real lines arrive at the completion point
    after backspaces, word kills, pastes, cursor moves, undo and vi-mode
    motions, and completion re-derives everything from the line and the cursor
    at TAB time — so the EDIT HISTORY can change what completion sees even when
    the final text is byte-identical.

    `--edit-fuzz` generates a seeded EDIT PROGRAM that runs between the buffer
    and the completion keys, and asserts parity after EVERY token of it as well
    as after every completion key. It covers emacs kills/motions/transpose/
    yank/undo/backspace-runs/retype-over, vi normal-mode motions and
    dw/db/dd/cw/x/./u under `bindkey -v`, and paste-shaped bursts delivered in
    one write (bracketed and raw).

    It also generates CONVERGENT PAIRS: two DIFFERENT edit programs claimed to
    produce the IDENTICAL final line. If the reference shell ends both on the
    same screen and zshrs does not, the line is not the variable — the history
    that built it is. A pair that does not converge in the reference shell is a
    counted SKIP, never a pass.

        --edit-cases N         edit cells per combo (default 4)
        --convergent-cases N   convergent pairs per combo (default 2)
        --edit-modes LIST      emacs,vi,emacs-nobp (bracketed paste off)

Multiline fuzz (`--multiline-fuzz`)
    Every other case here — and in every sibling harness — completes on ONE
    physical line. Completion re-parses the whole buffer through the lexer, so
    a word inside a continuation (after a trailing `\\`, inside a quote or a
    `$( )` that spans lines, in a `for`/`while`/`if`/`case` body, in an `x=( `
    array literal, in a heredoc body or on its terminator line, after a
    trailing `|`/`&&`/`||`) reaches `get_comp_string` through a different path
    than the same word on one line. The continuation also drags in PS2 and the
    prompt-height accounting this project has had real bugs in, so these run
    under `--geometry-fuzz` too.

    Every generated buffer is deliberately INCOMPLETE at each newline, so the
    Enter that ends a line is answered with PS2 on both shells and never
    executes anything.

Filesystem fuzz (`--fstree-fuzz`)
    Path completion is the most-used surface in the shell, and every path case
    in this file and in every sibling harness completes against whatever this
    disk happens to contain — `/usr/`, `~/`, the repo. Nothing constructs an
    adversarial tree, so the interesting inputs are never tried.

    `--fstree-fuzz` builds a SEEDED, HERMETIC tree in a scratch directory, `cd`s
    BOTH shells into it (identical tree, same absolute path, same init line) and
    completes inside it. It covers spaces, tabs, leading/trailing spaces and a
    literal newline in filenames; glob metacharacters (`*` `?` `[` `]` `{` `}`
    `~` `#` `^`) as literal name content; quote and escape characters (`'` `"`
    `\\` backtick `$` `!`); option-looking names (`-dash`, `--ddash`, a file
    named `-`) and dotfiles including one beginning `..`; a NAME_MAX-length name
    and a directory chain near PATH_MAX; a long shared prefix (ambiguous
    completion) and a directory of a few thousand entries (the listing/paging
    path); symlinks to a directory, to a file, dangling, and a symlink LOOP;
    directories with no read and with no execute permission; and case-colliding
    names on what is a case-INSENSITIVE APFS volume.

    Nothing about the tree is assumed. Every planned entry is read back off the
    filesystem after creation and classified created / folded / rejected, and a
    category whose entries did not land is DISABLED and counted — a name the
    filesystem folded or refused is a GENERATOR issue, named in the report,
    never a parity finding. `--fstree-verify` builds the tree, prints exactly
    what this disk accepted, and exits without booting a shell.

    The tree is a pure function of `--fstree-seed` and `--fstree-big`, and the
    seed is printed on every fstree result line and carried in every replay, so
    a replay rebuilds the identical tree at the identical path first.

Interruption fuzz (`--interrupt-fuzz`)
    Nothing in this harness ever interrupts a completion, and real completions
    are interrupted constantly. This project has shipped a SIGWINCH-triggered
    infinite `zrefresh` recursion that reproduced only at tiny row counts, and a
    type-ahead-eaten-after-accept bug.

    Three kinds, delivered identically to both shells: a REAL terminal resize
    (`TIOCSWINSZ` on the pty master, which is what raises SIGWINCH — not a
    `kill -WINCH`, which would skip the size change the handler reads), SIGINT
    to the shell's process group, and a burst of type-ahead written in one
    write. Three anchors: `before` the first TAB, at `menu` (the first key has
    settled, so a listing is on screen), and `midkey<N>` — delivered
    `--interrupt-delay-ms` after key N's write and BEFORE the screen is drained,
    which is genuinely mid-computation and therefore genuinely racy.

    A shell that DIES gets its own verdict, `DIED`, naming which side went and
    with what signal. It is never a pass, never a plain FAIL and never a
    timeout: round 3 of this tooling established that a crashed REFERENCE shell
    mislabelled as a timeout hid a real upstream zsh segfault for two rounds.
    The liveness check runs on every step of every mode, not only under this
    flag.

Session fuzz (`--session-fuzz N`)
    Every other mode here boots two shells, types once, completes once and
    kills them, so state that ACCUMULATES over a process's lifetime is
    invisible by construction. Real sessions run thousands of completions in
    one process, and several confirmed bugs in this project are exactly that
    shape: completion-time state leaking into the parameter table, a
    `$compstate[old_list]` that says `yes` where zsh says `shown` and only on
    the SECOND invocation, `_tags_level` desyncing from `$#funcstack` across a
    dispatch chain, a list that is valid versus currently-shown.

    A session is N EPISODES in ONE pair of shells: clear the screen, type a
    buffer, complete it key by key, end the line, repeat with a different
    buffer. Parity is asserted after EVERY step of EVERY episode, not once at
    the end. The fuzzed buffer is NEVER executed — an episode ends with ^G+^U
    (abort) or, for the accept path, ^G+^U and then a fixed literal `true`.

    Between episodes both shells write their own state to a file and the files
    are diffed here (a file, not the screen: the point is to NAME the
    parameters that drifted, and a grid read caps every answer at the window
    width). Probed: the name sets of `$parameters`, `$functions`, `$aliases`,
    `$galiases` and `$commands`; the full `name=on|off` option set and `setopt`;
    and `$#funcstack` at a known point plus whether `compstate`, `WIDGET`,
    `LASTWIDGET`, `_tags_level`, `_comp_tags`, `PREFIX`/`SUFFIX` and
    `curcontext` are bound outside completion at all.

    Each shell is compared against ITS OWN baseline, and the two DELTAS are
    what is compared across shells — the two shells do not start from the same
    table (zshrs carries parameters zsh does not), and reporting that
    pre-existing difference once per episode would bury the finding. The
    baseline difference is still reported, by name, in its own category. A
    probe that moved identically on both shells is zsh's own accumulation
    faithfully reproduced and is counted separately, never as a verdict; a
    probe that could not be taken is NAMED, never read as agreement.

    Deliberately excluded, and printed in the run header so the exclusion can
    be audited: parameter VALUES (only the name set is compared, so HISTCMD /
    SECONDS / RANDOM / `$?` churn cannot manufacture a finding, while a
    parameter that appears or disappears still does), the history size, and the
    pid.

    `--session-repeat M` runs each episode M times back to back and requires
    every repetition to render identically to the first — on EACH shell
    independently, BEFORE the two are compared with each other. A shell that
    disagrees with itself is a different bug from a zsh-vs-zshrs difference and
    is named as its own finding, on whichever shell did it.

    A drift is reduced with the same ddmin the other three axes use, over the
    EPISODE SEQUENCE, to the shortest run of episodes that still drifts at the
    same named key-set difference; the reduced sequence replays through
    `--session-replay`.

Latency (`--latency`)
    Every assertion in this file is about WHAT the two shells drew, never how
    long they took — and a completion that takes 25 seconds where zsh takes
    0.1 is a defect in this project, not a footnote. `--latency` measures, per
    keystroke and for BOTH shells, the time to first byte and the time to the
    last byte before the screen settles (the trailing quiet window is excluded;
    it is a harness constant, not shell work).

    It is a SEPARATE, additively-reported axis. It cannot change a correctness
    verdict in either direction, and a correctness PASS stays a PASS no matter
    how slow the cell was.

    Noise defences, because this box runs many concurrent sessions:
      * best-of-K runs per cell (`--latency-runs`, default 3), min per side —
        best-of is the standard defence against load noise, a mean is not;
      * a sample is only ever reported or flagged once the ABSOLUTE delta
        clears `--latency-min-ms` (default 25), so 2ms vs 1ms is never called
        a 2x regression;
      * `--jobs > 1` is REFUSED outright: concurrent cells contend for the same
        cores and the numbers would be fiction;
      * while measuring, both ptys are drained in ONE select loop so neither
        shell's clock includes the other's wait.

    The zshrs binary under test is a DEBUG build and is uniformly slower than
    an optimised zsh, so the raw ratio has a floor that is a build artefact.
    The signal is therefore the OUTLIER against the harness's own baseline
    distribution (median ratio and MAD over every sample of the run), which is
    printed with the results. `--latency-threshold N` additionally flags any
    cell more than N times slower than the reference; it defaults to off so
    existing runs keep their verdicts.

Env / flags of note:
    --zshrs PATH      zshrs binary (default: target/debug/zshrs under repo)
    --zsh PATH        reference zsh (default: `zsh` on PATH)
    --dump PATH       compinit dump (default: ~/.zpwr/local/.zcompdump-*)
    --fpath DIR       extra fpath dir (repeatable); prepended on both shells
    --rows N --cols N terminal geometry (default 24x80)
    --settle MS       quiet-window before a screen is considered settled
"""

from __future__ import annotations

import argparse
import functools
import glob
import json
import os
import pty
import random
import select
import shlex
import shutil
import signal
import statistics
import sys
import tempfile
import termios
import threading
import time
from collections import Counter, namedtuple
from dataclasses import dataclass, field

try:
    import pyte
except ImportError:
    sys.exit("compsys_parity: pyte not installed (pip install pyte)")


class _TolerantScreen(pyte.Screen):
    """pyte.Screen that survives a private-mode SGR (``CSI ? ... m``).

    pyte's parser forwards ``private=True`` for any CSI it saw a ``?`` in,
    but ``Screen.select_graphic_rendition`` takes no such keyword, so one
    of those sequences raises ``TypeError`` mid-``feed`` and aborts the
    whole sweep (it killed a 207-cell native run partway through, while
    capturing the REFERENCE shell). Swallow the flag and render the
    attributes normally; nothing about the comparison changes.
    """

    def select_graphic_rendition(self, *attrs, **kwargs):
        kwargs.pop("private", None)
        return super().select_graphic_rendition(*attrs, **kwargs)

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

# Unique, glob-free, unlikely-to-appear-in-completions prompt sentinel. Trailing
# space so the cursor sits one cell right of it and we can spot readiness.
PROMPT_SENTINEL = "@ZP@"

# ── keystroke vocabulary ──────────────────────────────────────────────────────
#
# Keys, key SEQUENCES and the case corpus are shared with comptab_parity.py so
# the two harnesses can never drift apart. See scripts/parity_corpus.py.
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from parity_corpus import (  # noqa: E402
    CASES as SHARED_CASES,
    DEFAULT_SEQUENCES,
    KEY_SEQUENCES,
    KEYS,
    cases_by_tag,
    key_bytes,
    shrink as ddmin,
)

# `pty.fork()` forks a process that is running Python threads (--jobs > 1). The
# child execs immediately, but between fork and exec it must not touch a lock
# another thread held at fork time. Serialising the fork+exec pair keeps that
# window to one thread at a time; it costs nothing at --jobs 1. Same guard as
# comptab_parity._FORK_LOCK.
_FORK_LOCK = threading.Lock()


# ── edit programs (`--edit-fuzz`) ─────────────────────────────────────────────
#
# Every case in every harness here types a buffer left to right and then
# completes. Real lines do not arrive that way: they reach the completion point
# after backspaces, word kills, pastes, cursor moves, undo and — under
# `bindkey -v` — vi normal-mode motions. That matters because completion
# re-derives everything from the line and the cursor at TAB time
# (`get_comp_string`), so the EDIT HISTORY that produced a line can change what
# completion sees even when the final text is byte-identical. This project has
# shipped bugs of exactly that shape (a filter keystroke dropped mid-menu, a
# `^@` plus a stale line leaking out of interactive menu reconstruction,
# type-ahead eaten after accept-line, vi/emacs editing divergences).
#
# An EDIT PROGRAM is a list of tokens run after the buffer is typed and before
# the completion keys, with the two screens diffed after EVERY token exactly as
# they are after every completion key.
#
# Token grammar — one comma-separated list, payloads percent-encoded so a comma
# or a space inside a paste can never be read as a separator:
#
#   k:<keyname>   one named key (parity_corpus.KEYS, or EDIT_KEYS below)
#   t:<text>      text TYPED — one write() per character, with a read between,
#                 so the shell sees separate arrivals like a human typing
#   p:<text>      text PASTED — the whole payload in ONE write(), which is the
#                 queued-keystroke / type-ahead path
#   b:<text>      the same single write, wrapped in the bracketed-paste
#                 ESC[200~ / ESC[201~ brackets a terminal emulator sends
#
# The whole program round-trips through `--edit-program` so every failure
# replays verbatim.
EDIT_KEYS: dict[str, bytes] = {
    # Meta-prefixed emacs motions/kills that parity_corpus.KEYS does not carry.
    # Verified against `zsh -f -c 'bindkey -M emacs'` on this host:
    #   "^[b" backward-word   "^[f" forward-word
    #   "^[d" kill-word       "^[t" transpose-words
    "alt-b": b"\x1bb",
    "alt-f": b"\x1bf",
    "alt-d": b"\x1bd",
    "alt-t": b"\x1bt",
    # "^T" transpose-chars, "^X^U" undo, "^X^K" kill-buffer (same listing).
    "ctrl-t": b"\x14",
    "ctrl-x-ctrl-u": b"\x18\x15",
    "ctrl-x-ctrl-k": b"\x18\x0b",
}

BRACKET_PASTE_START = b"\x1b[200~"
BRACKET_PASTE_END = b"\x1b[201~"


def resolve_key(name: str) -> bytes:
    """Bytes for one key name: the local edit vocabulary first, then the shared
    corpus table (which still rejects an unknown multi-character name outright,
    so a typo can never masquerade as that many self-inserted characters)."""
    if name in EDIT_KEYS:
        return EDIT_KEYS[name]
    return key_bytes(name)


def K(name: str) -> str:
    return "k:" + name


def T(text: str) -> str:
    return "t:" + _quote_payload(text)


def P(text: str) -> str:
    return "p:" + _quote_payload(text)


def B(text: str) -> str:
    return "b:" + _quote_payload(text)


def _quote_payload(text: str) -> str:
    from urllib.parse import quote
    return quote(text, safe="")


def _unquote_payload(text: str) -> str:
    from urllib.parse import unquote
    return unquote(text)


class BadEditToken(ValueError):
    """An edit token the DSL does not define."""


def edit_validate(tokens) -> list:
    """Reject a malformed program BEFORE any shell is booted.

    Same principle as `parity_corpus.key_bytes`: a mistyped token must be an
    error, never something that quietly turns into self-inserted characters on
    both shells at once and reads like a passing case."""
    out = []
    for tok in tokens:
        kind, sep, payload = tok.partition(":")
        if not sep or kind not in ("k", "t", "p", "b"):
            raise BadEditToken(f"{tok!r}: expected k:/t:/p:/b:")
        if kind == "k":
            resolve_key(payload)          # raises UnknownKey on a typo
        elif not _unquote_payload(payload):
            raise BadEditToken(f"{tok!r}: empty payload")
        out.append(tok)
    return out


def edit_encode(tokens) -> str:
    return ",".join(tokens)


def edit_decode(spec: str) -> list:
    return edit_validate([t for t in spec.split(",") if t])


def edit_label(tok: str) -> str:
    """Short human form for a report line."""
    kind, _, payload = tok.partition(":")
    if kind == "k":
        return payload
    return f"{kind}:{_unquote_payload(payload)!r}"


def edit_program_str(tokens) -> str:
    return "+".join(edit_label(t) for t in tokens) or "<none>"


def resolve_dump(explicit: str | None) -> str | None:
    if explicit:
        return explicit
    home = os.path.expanduser("~")
    for pat in (
        os.path.join(home, ".zpwr/local/.zcompdump*"),
        os.path.join(home, ".zcompdump*"),
    ):
        hits = sorted(glob.glob(pat))
        if hits:
            return hits[0]
    return None


def default_zshrs() -> str:
    return os.path.join(REPO, "target", "debug", "zshrs")


def std_fpath_dirs() -> list[str]:
    """Standard zsh completion dirs present on this host (where `_git` etc.
    ship). Kept minimal + deterministic so both shells scan the same set."""
    cands = sorted(glob.glob("/usr/share/zsh/*/functions")) + [
        "/opt/homebrew/share/zsh/site-functions",
        "/usr/local/share/zsh/site-functions",
        "/usr/share/zsh/site-functions",
    ]
    return [d for d in cands if os.path.isdir(d)]


@dataclass
class Case:
    name: str
    # Literal text typed into the line editor before the keystrokes.
    buffer: str
    # Sequence of key names from KEYS (or raw str, typed literally).
    keys: list[str] = field(default_factory=list)
    # Human note.
    note: str = ""


# ── per-keystroke timing ─────────────────────────────────────────────────────
#
# `ttfb`   ms from the write() that delivered the key to the FIRST byte the
#          shell wrote back. This is the shell's think time before it renders
#          anything — the number a human perceives as "did it react".
# `settle` ms from that same write() to the LAST byte before the screen went
#          quiet. The trailing quiet window (`--settle`) is deliberately NOT
#          included: it is a harness constant, identical for both shells, and
#          adding it to both sides would compress every ratio toward 1.
#
# Both are None when the key produced no output at all (a key the shell chose
# to ignore); such a sample is dropped rather than timed as zero.
KeyTiming = namedtuple("KeyTiming", "ttfb settle")


class ShellSession:
    """One shell child on its own PTY, screen mirrored through pyte."""

    def __init__(self, argv, env, rows, cols, label, settle_ms):
        self.label = label
        self.rows = rows
        self.cols = cols
        self.settle = settle_ms / 1000.0
        # Timing state: `_t0` is stamped by every send(), `_first_at`/`_last_at`
        # by every byte actually read, and `timing` is published when a drain
        # finishes. Recorded unconditionally (it costs two monotonic() calls per
        # read) but only ever REPORTED under --latency, which also forces the
        # concurrent drain that makes the numbers comparable.
        self._t0 = None
        self._first_at = None
        self._last_at = None
        self.timing = None
        # Latched exit status of this child, once reaped. A shell that DIED is
        # its own verdict — never a pass and never a plain FAIL — so the harness
        # has to be able to say which side went and with what signal. See
        # `exit_status`.
        self._exit = None
        self.screen = _TolerantScreen(cols, rows)
        self.stream = pyte.ByteStream(self.screen)
        with _FORK_LOCK:
            self.pid, self.fd = pty.fork()
            if self.pid == 0:  # child
                try:
                    # Fixed geometry so completion column math is identical.
                    import fcntl
                    import struct

                    winsz = struct.pack("HHHH", rows, cols, 0, 0)
                    fcntl.ioctl(sys.stdout.fileno(), termios.TIOCSWINSZ, winsz)
                    os.execvpe(argv[0], argv, env)
                except Exception as exc:  # pragma: no cover - child
                    os.write(2, f"exec failed: {exc}\n".encode())
                    os._exit(127)
        # parent: put the pty master in raw-ish mode isn't needed; slave already tty.

    # ── low-level io ──────────────────────────────────────────────────────────
    def _drain_once(self, timeout):
        r, _, _ = select.select([self.fd], [], [], timeout)
        if not r:
            return False
        try:
            data = os.read(self.fd, 65536)
        except OSError:
            return False
        if not data:
            return False
        now = time.monotonic()
        if self._first_at is None:
            self._first_at = now
        self._last_at = now
        self.stream.feed(data)
        return True

    def _finish_timing(self):
        """Publish `self.timing` for the drain that just ended."""
        if self._t0 is None:
            self.timing = None
            return
        ttfb = (self._first_at - self._t0) * 1000.0 if self._first_at else None
        settle = (self._last_at - self._t0) * 1000.0 if self._last_at else None
        self.timing = KeyTiming(ttfb, settle)

    def drain_settled(self, max_wait=8.0, first_wait=5.0):
        """Read until output settles. Waits up to `first_wait` for the FIRST
        byte, then returns after `settle` seconds of quiet. Hard-capped at
        `max_wait`. The first-byte wait matters because a cold zshrs
        completion autoloads the interpreted `_main_complete` chain from
        fpath and has real latency before it renders anything — a naive
        quiet-detector would fire in that gap and capture an empty screen."""
        start = time.monotonic()
        last = start
        seen = False
        try:
            while True:
                now = time.monotonic()
                if now - start > max_wait:
                    return
                got = self._drain_once(0.05)
                now = time.monotonic()
                if got:
                    seen = True
                    last = now
                elif not seen:
                    if now - start > first_wait:
                        return
                elif now - last >= self.settle:
                    return
        finally:
            self._finish_timing()

    def wait_for_prompt(self, timeout=15.0):
        """Wait for the prompt sentinel against a WALL-CLOCK deadline.

        The previous loop added a flat `step` per iteration no matter how long
        the iteration actually took. `_drain_once` returns as soon as data is
        ready — often in well under a millisecond — so a child that was writing
        steadily burned the whole counter in a fraction of `timeout` seconds
        and a shell that was booting normally, just chattily, was declared
        never to have reached a prompt. A monotonic deadline measures the thing
        the argument is named for. (Same fix as comptab_parity.Session.
        wait_prompt.)
        """
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            self._drain_once(0.05)
            if self._prompt_visible():
                return True
        return False

    def _prompt_visible(self):
        # Prompt sentinel present on any row, cursor past it.
        for line in self.screen.display:
            if PROMPT_SENTINEL in line:
                return True
        return False

    def send(self, data: bytes):
        # Stamp the clock HERE, immediately before the write: everything the
        # shell does about this key happens after this instant, and nothing the
        # harness does before it should be charged to the shell.
        self._t0 = time.monotonic()
        self._first_at = None
        self._last_at = None
        os.write(self.fd, data)

    def type_text(self, text: str):
        self.send(text.encode())

    # A buffer line that ENDS a physical line. `\r` is what a terminal sends
    # for Return, so both shells take the same accept-line path; every
    # multiline surface is deliberately incomplete at that point, so the shell
    # answers with PS2 instead of running anything.
    def buffer_lines(self, text: str):
        """Split a possibly multi-line buffer into the writes that type it.

        Yields `bytes` chunks in order. A `\\n` in the buffer becomes a real
        Return keystroke, so the harness never has to model PS2 itself — it
        just types the line and lets both shells continue it."""
        parts = text.split("\n")
        for i, part in enumerate(parts):
            if part:
                yield part.encode()
            if i < len(parts) - 1:
                yield b"\r"

    def send_key(self, name: str):
        # STRICT: parity_corpus.key_bytes rejects an unknown multi-character
        # name outright. The old `KEYS.get(name, name.encode())` fallback turned
        # a typo into that many self-inserted characters on BOTH shells, which
        # looked like a passing case for a key that was never sent.
        # `resolve_key` adds the local edit vocabulary (alt-b, ^T, ...) on top;
        # it is a strict superset, so a name that used to raise still raises.
        self.send(resolve_key(name))

    def type_slow(self, text: str):
        """Type `text` one character per write(), reading between characters.

        `type_text` hands the whole string to a single write(), which the shell
        sees as one arrival — that is the PASTE shape, not the TYPING shape.
        Some of the bugs this mode exists to find live in the difference (a
        keystroke queued behind another has a different path through the input
        layer than a keystroke that arrives alone), so the two must not be the
        same call."""
        for ch in text:
            self.send(ch.encode())
            self._drain_once(0.004)

    def send_edit_token(self, tok: str):
        """Deliver one edit-program token. See the DSL comment above `EDIT_KEYS`."""
        kind, _, payload = tok.partition(":")
        if kind == "k":
            self.send(resolve_key(payload))
        elif kind == "t":
            self.type_slow(_unquote_payload(payload))
        elif kind == "p":
            self.send(_unquote_payload(payload).encode())
        elif kind == "b":
            self.send(BRACKET_PASTE_START
                      + _unquote_payload(payload).encode()
                      + BRACKET_PASTE_END)
        else:
            raise BadEditToken(tok)

    # ── screen access ─────────────────────────────────────────────────────────
    def grid(self):
        """Normalized screen: rstripped rows, trailing blank rows dropped."""
        rows = [row.rstrip() for row in self.screen.display]
        while rows and rows[-1] == "":
            rows.pop()
        return [self._mask_pid(r) for r in rows]

    def _mask_pid(self, row):
        """Replace THIS shell's own pid with a stable token.

        Ported verbatim from comptab_parity.py's Session._mask_pid so the two
        harnesses cannot drift. `$$` is the pid of the shell under test, so the
        reference and the candidate necessarily report different values — two
        live processes cannot share a pid. A case that displays it (`unset
        <TAB>` lists every parameter with its value, `$` among them) can
        therefore never compare equal no matter how correct zshrs is, and
        scored as a permanent failure on every key sequence.

        Only the exact pid of this session's own child is substituted, taken
        from the fork in ShellSession.__init__ — not a general digit mask. Any
        other number on the screen, including one that merely looks like a pid,
        still has to match byte for byte.
        """
        return row.replace(str(self.pid), "<PID>") if self.pid else row

    def fresh_prompt(self):
        """Clear to a clean prompt at row 0 via the shell's own clear-screen
        widget (Ctrl-L), so the case captures from the top of the screen.

        Crucially does NOT reset the pyte screen: a reset wipes pyte's cell
        contents while the real shell still believes the prompt is displayed,
        so the shell's relative cursor moves during completion redraw (zsh
        emits `\\r\\e[5C` to step over the prompt rather than reprint it) land
        `cd /` over blank cells and render `cd / cd /`. Letting the shell's own
        clear-screen sequence drive pyte keeps content and cursor in sync, so
        the command-line row is captured faithfully and compared like any
        other row."""
        self.send(b"\x0c")   # Ctrl-L → clear-screen, redraw prompt at row 0
        self.drain_settled(max_wait=3.0, first_wait=2.0)

    # ── liveness / interruption ───────────────────────────────────────────────
    def exit_status(self):
        """`(kind, value)` once this child has exited, else None. Never blocks.

        `kind` is "signal" (value = signal number: a CRASH or a kill) or "exit"
        (value = exit status). Latched, so a later call still reports it after
        the pid has been reaped.

        This exists because round 3 of this tooling established that a crashed
        REFERENCE shell mislabelled as a timeout hid a real upstream zsh
        segfault for two rounds. A shell that dies has to be NAMED — which side,
        which signal — not folded into whatever verdict the surviving grid
        happened to produce.
        """
        if self._exit is not None:
            return self._exit
        try:
            pid, st = os.waitpid(self.pid, os.WNOHANG)
        except OSError:
            return None
        if pid != self.pid:
            return None
        if os.WIFSIGNALED(st):
            self._exit = ("signal", os.WTERMSIG(st))
        elif os.WIFEXITED(st):
            self._exit = ("exit", os.WEXITSTATUS(st))
        else:                                        # pragma: no cover
            self._exit = ("status", st)
        return self._exit

    def resize(self, rows, cols):
        """A REAL terminal resize: TIOCSWINSZ on the pty master.

        The kernel raises SIGWINCH on the slave's foreground process group as a
        side effect, so this is the actual signal a window drag delivers — not a
        `kill -WINCH`, which would skip the size change the handler reads. This
        project has a documented SIGWINCH-triggered infinite `zrefresh`
        recursion that reproduced only at tiny row counts, so the size change
        and the signal have to arrive together the way they really do.

        pyte's screen is resized to match, otherwise every row captured after
        the resize would be compared at the old width."""
        import fcntl
        import struct
        try:
            fcntl.ioctl(self.fd, termios.TIOCSWINSZ,
                        struct.pack("HHHH", rows, cols, 0, 0))
        except OSError:
            return False
        self.rows, self.cols = rows, cols
        self.screen.resize(rows, cols)
        return True

    def send_signal(self, sig):
        """Send `sig` to this child's process GROUP (pty.fork setsid()s the
        child, so its pgid is its pid and the shell plus anything it forked are
        both in it — which is what a ^C from a terminal driver reaches).

        Refuses once the child is known to have exited, so a reaped-and-recycled
        pid can never be signalled by accident."""
        if self.exit_status() is not None:
            return False
        try:
            os.killpg(self.pid, sig)
            return True
        except OSError:
            return False

    def close(self):
        try:
            os.write(self.fd, b"\x03")   # ctrl-c out of any menu
            os.write(self.fd, b"\x04")   # ctrl-d / EOF
        except OSError:
            pass
        try:
            os.close(self.fd)
        except OSError:
            pass
        # Non-blocking reap with escalation; never block teardown on a
        # child stuck in menu-select or a redraw loop.
        for sig in (signal.SIGHUP, signal.SIGTERM, signal.SIGKILL):
            try:
                pid, _ = os.waitpid(self.pid, os.WNOHANG)
                if pid == self.pid:
                    return
                os.kill(self.pid, sig)
            except OSError:
                return
            for _ in range(20):  # up to ~0.5s between signals
                try:
                    pid, _ = os.waitpid(self.pid, os.WNOHANG)
                    if pid == self.pid:
                        return
                except OSError:
                    return
                select.select([], [], [], 0.025)


def drain_concurrent(sessions, max_wait=8.0, first_wait=5.0):
    """Drain several sessions in ONE select loop, each to its own quiet window.

    The sequential `for s in (ref, test): s.drain_settled()` the harness uses
    everywhere else is correct for COMPARISON — the end state is the same
    either way — but it is useless for TIMING: the second shell's clock starts
    at its own write() and its first byte is not read until the first shell has
    finished settling, so the second shell is charged for the first shell's
    entire wait. Under --latency the pair is drained here instead, where each
    session's bytes are read as they arrive and each session's `first_wait` /
    quiet window is judged against its own arrivals.

    Nothing about what either shell DREW changes: the same bytes are fed to the
    same pyte screens, in the same per-session order.
    """
    start = time.monotonic()
    state = {id(s): [False, start] for s in sessions}   # [seen, last]
    pending = list(sessions)
    while pending:
        now = time.monotonic()
        if now - start > max_wait:
            break
        try:
            ready, _, _ = select.select([s.fd for s in pending], [], [], 0.02)
        except (OSError, ValueError):
            break
        now = time.monotonic()
        for s in list(pending):
            st = state[id(s)]
            if s.fd in ready:
                if s._drain_once(0.0):
                    st[0] = True
                    st[1] = time.monotonic()
                    continue
                # select said readable and the read produced nothing: the pty
                # is at EOF (the child died). Drop it NOW — leaving it in
                # `pending` would spin this loop at full CPU on a permanently
                # readable fd until first_wait/max_wait expired, which is both
                # a busy-wait and a timing measurement of the harness.
                pending.remove(s)
                continue
            if st[0]:
                if time.monotonic() - st[1] >= s.settle:
                    pending.remove(s)
            elif now - start > first_wait:
                pending.remove(s)
    for s in sessions:
        s._finish_timing()


@functools.lru_cache(maxsize=1)
def ref_ostype() -> str | None:
    """`$OSTYPE` as the REFERENCE shell reports it.

    Cached: `build_init_file` now runs once per shrink probe, and the value is
    a compile-time constant of the reference binary, so re-forking a zsh for it
    every probe only costs time.

    OSTYPE is baked into the binary at compile time from the build host's
    uname, not computed at run time: the reference `zsh` here says
    `darwin25.4.0` (built on macOS 25.4) while the host now runs 25.5.0, so
    zshrs — compiled today — says `darwin25.5.0`. Neither value is wrong; each
    records the machine its binary was built on, and no zshrs change can make
    them agree. `unset <TAB>` lists every parameter with its value, OSTYPE
    among them, so that one row failed on every key sequence for the case.

    Pinning BOTH shells to the reference's own string leaves zsh's behaviour
    untouched and aligns only the constant. Returns None if the reference
    cannot be asked, in which case no assignment is emitted.
    """
    try:
        import subprocess
        out = subprocess.run(
            ["zsh", "-f", "-c", "print -r -- $OSTYPE"],
            capture_output=True, text=True, timeout=10,
        ).stdout.strip()
        return out or None
    except Exception:
        return None


@functools.lru_cache(maxsize=1)
def _user_fpath_cached() -> tuple:
    """The user's real completion fpath, as `zsh -f` sees it on this host
    (global rc populates it even under -f). Used so both shells scan the
    identical function set the user's `.zcompdump` was built from."""
    try:
        import subprocess
        out = subprocess.run(
            ["zsh", "-f", "-c", "print -rl -- $fpath"],
            capture_output=True, text=True, timeout=10,
        ).stdout
        return tuple(d for d in out.splitlines() if d and os.path.isdir(d))
    except Exception:
        return ()


def user_fpath() -> list[str]:
    return list(_user_fpath_cached())


# The editing modes `--edit-fuzz` runs under. Each is a line appended to the
# shared init file, so BOTH shells are configured identically and the mode is
# part of the replay.
#
#   emacs        explicit `bindkey -e`. `zsh -f` already lands here (no
#                $EDITOR/$VISUAL in child_env), but saying so keeps the mode a
#                property of the fixture instead of a property of the host.
#   vi           `bindkey -v`. A distinct keymap: completion fired from a
#                vi-NORMAL-mode cursor position is a surface no other case in
#                any harness reaches.
#   emacs-nobp   emacs with bracketed paste disabled. `man zshparam`,
#                zle_bracketed_paste: "Unsetting the parameter has the effect
#                of ensuring that bracketed paste remains disabled." A `p:`
#                burst therefore takes the raw type-ahead path with no
#                ESC[200~ bracket around it.
EDIT_MODES: dict[str, str] = {
    "emacs": "bindkey -e\n",
    "vi": "bindkey -v\n",
    "emacs-nobp": "bindkey -e\nunset zle_bracketed_paste\n",
}


def build_init_file(dump, fpath_dirs, zstyle_file, editing_mode=None, cwd=None,
                    probe=False):
    """Write the init script both shells source after launching with `-f`.
    Matches the spec: same fpath, same zstyles, same compinit + dump, so the
    only variable left is the shell under test.

    `editing_mode` (None by default, which emits nothing and leaves every
    pre-existing caller byte-identical) appends one of EDIT_MODES.

    `cwd` (None by default, likewise emitting nothing) `cd`s BOTH shells into
    one directory — the hermetic tree `--fstree-fuzz` builds. It is emitted as
    a hard `cd ... || return 1` so a shell that cannot get there fails at init
    and is reported as never having reached a prompt, rather than silently
    completing against $HOME and comparing two wrong screens.

    `probe` (False by default, which emits nothing and leaves every
    pre-existing caller byte-identical) appends the `_cp_probe` state-probe
    function `--session-fuzz` calls between episodes. It is defined HERE, in the
    init, so it already exists when the session's BASELINE probe runs and is
    therefore in `$functions` on both shells from the start — a probe that
    defined itself later would show up as its own first drift."""
    d = tempfile.mkdtemp(prefix="compsys_parity_")
    fpath_line = ""
    if fpath_dirs:
        joined = " ".join(shlex.quote(p) for p in fpath_dirs)
        fpath_line = f"fpath=( {joined} )\n"
    zstyle_line = ""
    if zstyle_file and os.path.exists(zstyle_file):
        zstyle_line = f"source {shlex.quote(zstyle_file)}\n"
    # The user's `completer` chain names custom completers that live in
    # ~/.zpwr; `compinit -C` trusts the dump and skips the fpath rescan, so
    # they aren't autoloaded. Pull in the ones that exist (e.g. _megacomplete),
    # and — crucially for parity — define `return 1` stubs for the ones that are
    # NOT installed in this sandbox (the fasd triggers; fasd isn't present under
    # `-f`). Without a definition, when the completer chain REACHES such a name
    # (e.g. `cd <tab>` with no earlier match), zsh prints "command not found: N"
    # to the completion display while zshrs's direct dispatch stays silent — a
    # divergence that is purely a missing-function artifact, not a completion
    # difference. A `return 1` stub matches what real fasd does when there is no
    # trigger, so both shells skip it identically. Applied to BOTH shells.
    zpwr_comp = os.path.expanduser("~/.zpwr/autoload/comp_utils")
    referenced = ("_megacomplete", "_fasd_zsh_word_complete_trigger",
                  "_fasd_zsh_word_complete", "_fasd_zsh_word_complete_f",
                  "_fasd_zsh_word_complete_d")
    have = []
    missing = []
    for f in referenced:
        if os.path.isdir(zpwr_comp) and os.path.exists(os.path.join(zpwr_comp, f)):
            have.append(f)
        else:
            missing.append(f)
    autoload_line = ""
    if have:
        autoload_line += (
            f"fpath=( {shlex.quote(zpwr_comp)} $fpath )\n"
            f"autoload -Uz {' '.join(have)}\n"
        )
    for f in missing:
        autoload_line += f"{f}() {{ return 1 }}\n"
    # `_megacomplete`'s `ret==1` fallbacks (`_complete_plus_last_command_args`,
    # `_complete_clipboard`) and its `CURRENT==1` fallback (`_complete_hist`) are
    # zpwr-custom fns that read HISTORY / CLIPBOARD / the last command — content
    # that differs between the two independent PTY sessions and is not part of
    # the compsys ENGINE (they run only AFTER `\_complete` has already produced
    # the engine result). Under `-f`'s minimal init they are undefined, so zsh
    # prints `command not found: _complete_...` into the completion display while
    # zshrs stays silent — a pure reference-shell artifact, not an engine
    # difference. Stub them `return 1` in BOTH shells (same rationale as the fasd
    # triggers above) so the fallback is a no-op and the diff isolates the
    # engine. NOT a number-fudge: no zshrs completion result is suppressed —
    # the engine matches were already emitted before these are reached.
    always_stub = ("_complete_hist", "_complete_plus_last_command_args",
                   "_complete_clipboard")
    for f in always_stub:
        autoload_line += f"{f}() {{ return 1 }}\n"
    if dump:
        # -C: trust the dump — skip the security check AND the fpath rescan for
        # new/changed completers. Matches the user's fast-startup setup and
        # avoids reading every fpath file (a broken symlink there hangs -u).
        compinit = f"autoload -Uz compinit\ncompinit -C -d {shlex.quote(dump)}\n"
    else:
        compinit = "autoload -Uz compinit\ncompinit -u\n"
    ost = ref_ostype()
    ostype_line = f"OSTYPE={shlex.quote(ost)}\n" if ost else ""
    if editing_mode is not None and editing_mode not in EDIT_MODES:
        raise ValueError(f"unknown editing mode: {editing_mode}")
    mode_line = EDIT_MODES.get(editing_mode, "")
    cwd_line = f"cd {shlex.quote(cwd)} || return 1\n" if cwd else ""
    probe_line = PROBE_FUNCTION if probe else ""
    init = f"""\
# generated by compsys_parity.py — sourced into `zsh -f` and `zshrs --zsh -f`
PROMPT='{PROMPT_SENTINEL} '
RPROMPT=''
PS2=''
setopt no_beep
{ostype_line}\

{fpath_line}{zstyle_line}{compinit}{autoload_line}{mode_line}{cwd_line}{probe_line}
# Readiness barrier: block the prompt until the completion map is populated so
# a keystroke fired right after the prompt isn't racing an unfinished compinit
# (real zsh fills `_comps` synchronously; zshrs may register asynchronously).
integer _cp_tries=0
while (( ${{#_comps}} == 0 && _cp_tries < 200 )); do
  _cp_tries+=1
  sleep 0.05
done
print -u2 ''
"""
    path = os.path.join(d, "init.zsh")
    with open(path, "w") as f:
        f.write(init)
    return path


def child_env(rows: int = 24, cols: int = 80) -> dict:
    """Environment for BOTH children. COLUMNS/LINES follow the geometry the pty
    is actually sized to — they used to be hardcoded 80x24, so a run at any
    other `--rows/--cols` handed the shells a window size that disagreed with
    their own tty. Both shells always get the identical pair."""
    env = {
        "TERM": "xterm-256color",
        "LANG": "C",
        "LC_ALL": "C",
        "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
        "COLUMNS": str(cols),
        "LINES": str(rows),
        # zshrs ships ~145 builtins zsh does not have (peach, async, zf_*,
        # dbview, ...), so any listing that enumerates $builtins diverges by
        # construction. Those are deliberate zshrs FEATURES, not compat
        # regressions, and scoring them as parity failures buries the real
        # ones. This hides the extension names from the `builtins` table and
        # the compctl namespace dump for the duration of the comparison ONLY
        # — dispatch is untouched (`whence -w peach` still says builtin).
        "ZSHRS_HIDE_EXT_BUILTINS": "1",
        # Autosuggestion / syntax-highlight ghost text has no zsh counterpart,
        # so it lands in the grid as extra cells on the command-line row and
        # every case with a history hit diffs on it — `echo $PATH` came back as
        # `echo $PATH | head -c 30`, which reads like a completion bug and is
        # not one. comptab_parity.py already suppressed this; the two harnesses
        # must boot the children identically or their results are not
        # comparable. Suppresses the fx LAYER only; the completion engine is
        # untouched.
        "ZSHRS_NATIVE_ZLE_FX": "0",
    }
    # Preserve HOME so the user's real `.zcompdump`/cache paths resolve.
    if "HOME" in os.environ:
        env["HOME"] = os.environ["HOME"]
    return env


def normalize_rows(rows):
    """Minimal, symmetric normalization only — nothing that could hide a real
    divergence. pyte pads every row to the full terminal width, so trailing
    spaces are meaningless; rstrip them and drop trailing blank rows. The
    injected `@ZP@ ` prompt renders identically on both shells (Ctrl-L redraws
    it at row 0 on each), so it is left in and compared like any other cell."""
    out = [r.rstrip() for r in rows]
    while out and out[-1] == "":
        out.pop()
    return out


# ── latency axis ─────────────────────────────────────────────────────────────
#
# STRICTLY ADDITIVE. A latency finding is reported in its own named category,
# never mixed into the parity verdict set, and can neither create nor suppress
# a correctness FAIL/FLAKY/PASS. The reason is not politeness: the two measure
# different things, and a harness that let a slow cell read as a compatibility
# divergence (or vice versa) would make both numbers useless.
LAT_DEBUG_NOTE = (
    "the zshrs under test is a DEBUG build and is uniformly slower than an "
    "optimised zsh — the ratio has a floor that is a BUILD artefact, so the "
    "signal below is the OUTLIER against this run's own baseline, not the raw "
    "ratio"
)


@dataclass
class LatSample:
    """One key, timed on both shells, already reduced to best-of-K."""
    cell: str
    idx: int
    label: str
    ref_ms: float
    test_ms: float
    ref_ttfb: float = None
    test_ttfb: float = None
    note: str = ""

    @property
    def delta(self) -> float:
        return self.test_ms - self.ref_ms

    @property
    def ratio(self) -> float:
        return self.test_ms / self.ref_ms if self.ref_ms > 0 else float("inf")


def merge_best_timings(runs):
    """Element-wise BEST-OF over K timed runs of the same key path.

    Best-of, not mean: this box runs many concurrent agent sessions, and load
    only ever makes a measurement WORSE. The minimum is the closest thing to
    the shell's own cost that a loaded machine can show; a mean folds the noise
    straight into the number and a single 300ms scheduling hiccup would invent
    a regression. Each side is minimised independently — the question is what
    each shell can do, not what one pass happened to do.

    Runs are truncated to the shortest one, and at the first label mismatch:
    a run that stopped earlier (a divergence ends the lockstep) simply
    contributes no sample for the keys it never reached.
    """
    runs = [r for r in runs if r]
    if not runs:
        return []
    out = []
    for i in range(min(len(r) for r in runs)):
        labels = {r[i][0] for r in runs}
        if len(labels) != 1:
            break
        label = labels.pop()

        def best(which, field):
            vals = [getattr(r[i][which], field) for r in runs
                    if r[i][which] is not None
                    and getattr(r[i][which], field) is not None]
            return min(vals) if vals else None

        out.append((label,
                    KeyTiming(best(1, "ttfb"), best(1, "settle")),
                    KeyTiming(best(2, "ttfb"), best(2, "settle"))))
    return out


class LatencyBook:
    """Collects every timed key of a run, then answers two questions:
    what does this harness's ratio distribution look like on THIS machine and
    THIS build, and which cells are outliers against it."""

    # Below this many samples the median/MAD is not a distribution, it is a
    # rumour — the book says so and flags nothing rather than inventing a cut.
    MIN_BASELINE_SAMPLES = 8

    def __init__(self, min_delta_ms, threshold, runs, concurrent_drain):
        self.min_delta = min_delta_ms
        self.threshold = threshold
        self.runs = runs
        self.concurrent_drain = concurrent_drain
        self.samples: list = []
        self.dropped_no_output = 0
        self.dropped_zero_ref = 0
        # Cells that produced no timing at all. A convergent PAIR is not timed
        # as a pair — it is two runs, and "the cell's ratio" would be ambiguous
        # — though a pair that collapses to a single failing LEG is an ordinary
        # cell by then and is timed like one. Named in the report rather than
        # silently absent, so the latency section never implies coverage it
        # does not have.
        self.unmeasured: list = []

    def not_measured(self, cell_id):
        self.unmeasured.append(cell_id)

    def record(self, cell_id, merged, note=""):
        for i, (label, rt, tt) in enumerate(merged, 1):
            if rt is None or tt is None or rt.settle is None or tt.settle is None:
                # A key one of the shells answered with no output at all cannot
                # be timed. Counted, never treated as zero.
                self.dropped_no_output += 1
                continue
            if rt.settle <= 0:
                self.dropped_zero_ref += 1
                continue
            self.samples.append(LatSample(
                cell=cell_id, idx=i, label=label,
                ref_ms=rt.settle, test_ms=tt.settle,
                ref_ttfb=rt.ttfb, test_ttfb=tt.ttfb, note=note))

    # ── distribution ─────────────────────────────────────────────────────────
    def baseline(self):
        """(n, median, mad, p90, cut) over EVERY sample of the run.

        Deliberately computed over every sample, including the ones too small
        to report: the baseline is a description of this build's constant
        handicap, and filtering it to the big deltas would bias it upward and
        hide the very outliers it exists to find. The min-delta filter applies
        to what is REPORTED, not to what the distribution is measured from.
        """
        ratios = sorted(s.ratio for s in self.samples if s.ref_ms > 0)
        if len(ratios) < self.MIN_BASELINE_SAMPLES:
            return (len(ratios), None, None, None, None)
        med = statistics.median(ratios)
        mad = statistics.median([abs(r - med) for r in ratios])
        p90 = ratios[min(len(ratios) - 1, int(0.9 * len(ratios)))]
        # 1.4826*MAD is the MAD-based estimate of sigma for a normal
        # distribution; median + 3 sigma is the ordinary robust outlier cut.
        # Floored at 1.5x the median so a degenerate MAD of 0 (every cell
        # equally slow) cannot make the median itself the cut and flag half the
        # run.
        cut = max(med + 3 * 1.4826 * mad, med * 1.5)
        return (len(ratios), med, mad, p90, cut)

    def reportable(self):
        """Samples whose ABSOLUTE delta clears the floor.

        A 2ms-vs-1ms key is a 2x ratio and means nothing — it is one scheduler
        slice. Nothing below `--latency-min-ms` of real, measured difference is
        allowed to be called a regression, printed as a ratio, or flagged."""
        return [s for s in self.samples if s.delta >= self.min_delta]

    def worst_by_cell(self):
        """cell -> its worst REPORTABLE sample."""
        worst = {}
        for s in self.reportable():
            cur = worst.get(s.cell)
            if cur is None or s.ratio > cur.ratio:
                worst[s.cell] = s
        return worst

    def verdicts(self):
        """cell -> (verdict, sample). Verdicts live in their OWN namespace
        (`LAT-OUTLIER` / `LAT-OVER-THRESHOLD`) precisely so no reader can
        mistake one for a parity verdict."""
        _, _, _, _, cut = self.baseline()
        out = {}
        for cell, s in self.worst_by_cell().items():
            if self.threshold and s.ratio > self.threshold:
                out[cell] = ("LAT-OVER-THRESHOLD", s)
            elif cut is not None and s.ratio > cut:
                out[cell] = ("LAT-OUTLIER", s)
        return out

    # ── report ───────────────────────────────────────────────────────────────
    def report(self, limit=10):
        n, med, mad, p90, cut = self.baseline()
        print()
        print("# ── latency (its OWN category — never a parity verdict) ──────")
        print(f"#   {LAT_DEBUG_NOTE}")
        print(f"#   best-of-{self.runs} per key, each side minimised "
              f"independently; both ptys drained "
              f"{'concurrently' if self.concurrent_drain else 'sequentially'}")
        print(f"#   a sample is reported only when zshrs-zsh >= "
              f"{self.min_delta:.0f}ms of real measured difference")
        if not self.samples:
            print("#   no timed samples (no key produced output on both shells)")
            return 0
        if med is None:
            print(f"#   baseline: {n} samples — fewer than "
                  f"{self.MIN_BASELINE_SAMPLES}, too thin to call anything an "
                  f"outlier; ratios below are raw")
        else:
            print(f"#   baseline: {n} samples  median {med:.2f}x  "
                  f"MAD {mad:.2f}  p90 {p90:.2f}x  ->  outlier cut "
                  f"{cut:.2f}x"
                  + (f"   (--latency-threshold {self.threshold:g}x)"
                     if self.threshold else ""))
        if self.dropped_no_output or self.dropped_zero_ref:
            print(f"#   not timed: {self.dropped_no_output} keys drew nothing, "
                  f"{self.dropped_zero_ref} with a zero reference time")
        if self.unmeasured:
            print(f"#   {len(self.unmeasured)} cells carry no timing at all "
                  f"(convergent pairs are not timed): "
                  + ", ".join(self.unmeasured[:4])
                  + (" ..." if len(self.unmeasured) > 4 else ""))
        verdicts = self.verdicts()
        rep = sorted(self.reportable(), key=lambda s: -s.ratio)
        if not rep:
            print(f"#   no sample cleared the {self.min_delta:.0f}ms floor")
        else:
            print(f"#   slowest cells ({len(rep)} samples over the floor):")
            for s in rep[:limit]:
                verdict = verdicts.get(s.cell)
                tag = f"  {verdict[0]}" if verdict and verdict[1] is s else ""
                ttfb = ""
                if s.ref_ttfb is not None and s.test_ttfb is not None:
                    ttfb = (f"  ttfb {s.ref_ttfb:.0f}->{s.test_ttfb:.0f}ms")
                print(f"#     {s.ratio:6.2f}x  {s.cell:26s} key #{s.idx} "
                      f"{s.label!r:12s}  zsh {s.ref_ms:8.1f}ms -> zshrs "
                      f"{s.test_ms:8.1f}ms{ttfb}{tag}")
            if len(rep) > limit:
                print(f"#     ... {len(rep) - limit} more")
        n_out = sum(1 for v, _ in verdicts.values() if v == "LAT-OUTLIER")
        n_over = sum(1 for v, _ in verdicts.values()
                     if v == "LAT-OVER-THRESHOLD")
        print(f"#   latency verdicts: {n_out} LAT-OUTLIER, "
              f"{n_over} LAT-OVER-THRESHOLD"
              + ("" if self.threshold else
                 " (--latency-threshold unset: report only)"))
        return n_over

    def json_doc(self):
        n, med, mad, p90, cut = self.baseline()
        verdicts = self.verdicts()
        return {
            "note": LAT_DEBUG_NOTE,
            "build": "debug",
            "runs_best_of": self.runs,
            "min_delta_ms": self.min_delta,
            "threshold": self.threshold,
            "concurrent_drain": self.concurrent_drain,
            "baseline": {"samples": n, "median_ratio": med, "mad": mad,
                         "p90_ratio": p90, "outlier_cut": cut},
            "not_timed": {"no_output": self.dropped_no_output,
                          "zero_reference": self.dropped_zero_ref},
            "verdicts": {cell: {"verdict": v, "ratio": s.ratio,
                                "key": s.label, "key_index": s.idx,
                                "ref_ms": s.ref_ms, "test_ms": s.test_ms}
                         for cell, (v, s) in sorted(verdicts.items())},
            "samples": [{"cell": s.cell, "key_index": s.idx, "key": s.label,
                         "ref_ms": round(s.ref_ms, 2),
                         "test_ms": round(s.test_ms, 2),
                         "ref_ttfb_ms": (round(s.ref_ttfb, 2)
                                         if s.ref_ttfb is not None else None),
                         "test_ttfb_ms": (round(s.test_ttfb, 2)
                                          if s.test_ttfb is not None else None),
                         "ratio": round(s.ratio, 3),
                         "delta_ms": round(s.delta, 2)}
                        for s in sorted(self.reportable(),
                                        key=lambda x: -x.ratio)],
        }


def run_case(sess: ShellSession, case: Case):
    sess.fresh_prompt()
    # `key_timings` is published for --latency; the correctness path ignores it.
    sess.key_timings = []
    if case.buffer:
        # One write per PHYSICAL line: a `\n` in the buffer is a real Return on
        # a deliberately incomplete line, which the shell answers with PS2. A
        # single-line buffer takes exactly the write it always took.
        for chunk in sess.buffer_lines(case.buffer):
            sess.send(chunk)
            # Buffer chars just echo — instant; short first-byte wait.
            sess.drain_settled(max_wait=2.0, first_wait=1.0)
    for key in case.keys:
        sess.send_key(key)
        # A cold completion can take seconds to first render; wait for it.
        # A LITERAL character (not a named key) typed into an interactive menu
        # re-runs the whole completion, which is far slower than a TAB or an
        # arrow — see the note on the final drain below — so hold it to the
        # same quiet-window floor.
        prev = sess.settle
        if key not in KEYS:
            sess.settle = max(prev, 1.2)
        try:
            sess.drain_settled(max_wait=12.0, first_wait=8.0)
        finally:
            sess.settle = prev
        sess.key_timings.append((key, sess.timing))
    # Settle the FINAL screen against a longer quiet window than the
    # per-keystroke one. A literal key typed into an interactive menu re-runs
    # the whole completion, and an unoptimized zshrs build regularly takes
    # longer than the 250ms default to finish that last redraw — so the grid
    # got captured mid-flight and the last typed character was missing from
    # BOTH the command line and the `interactive:` status. That reads exactly
    # like a dropped keystroke: `git checkout <TAB><TAB>src` compared as
    # `interactive: src[]` (zsh) vs `interactive: sr[]` (zshrs), and four cells
    # of the corpus scored as divergences that pass verbatim at --settle 1200.
    #
    # This is a MEASUREMENT window, not a comparison weakening: it only waits
    # longer for output to stop before the screens are diffed, applies to both
    # shells identically, and hides no difference in what either one drew. An
    # explicit --settle above the floor still wins.
    prev_settle = sess.settle
    sess.settle = max(prev_settle, 1.2)
    try:
        sess.drain_settled(max_wait=6.0, first_wait=0.6)
    finally:
        sess.settle = prev_settle
    return normalize_rows(sess.grid())


def diff_grids(ref, test):
    """Return list of (row_index, ref_line, test_line) for mismatched rows."""
    n = max(len(ref), len(test))
    diffs = []
    for i in range(n):
        a = ref[i] if i < len(ref) else "<absent>"
        b = test[i] if i < len(test) else "<absent>"
        if a != b:
            diffs.append((i, a, b))
    return diffs


def render_grid(rows):
    return "\n".join(f"  {i:2d}| {r}" for i, r in enumerate(rows)) or "  <empty>"


# The built-in case set is the SHARED corpus, one entry per (case, sequence)
# pair so the existing `--only NAME` / `--list` UI still addresses a single
# runnable unit. `--sequences` picks which sequences are expanded.
def builtin_cases(sequences=None, tag=None, skip_optional=False):
    seqs = sequences or DEFAULT_SEQUENCES
    out = []
    for c in cases_by_tag(tag):
        if skip_optional and 'optional' in c.tags:
            continue
        for name in seqs:
            out.append(Case(f'{c.name}.{name}', c.buffer,
                            list(KEY_SEQUENCES[name]), c.note))
    return out


def parse_zstyle_statements(path):
    """Split a zstyle fixture into individual statements (one per non-comment,
    non-blank line). Each `zstyle ...` line is independent, so a random subset
    of them is a valid config — the parity bar is that ANY such combo renders
    byte-identically on zsh and zshrs."""
    out = []
    with open(path) as f:
        for line in f:
            s = line.rstrip("\n")
            if not s.strip() or s.strip().startswith("#"):
                continue
            out.append(s)
    return out


def run_cases_against(init_file, cases, args, env, confirm=1):
    """Run every case through `zsh -f` and `zshrs --zsh -f`, both sourcing
    init_file. Returns (passed, failed, fail_records) where each fail_record is
    (case, ref_grid, test_grid, diffs|None).

    `confirm` re-runs a failing case up to that many extra times. A pass on
    re-run means the case is NONDETERMINISTIC, which is itself a bug class (see
    the worker-pool tty race) — it is reported as FLAKY and counted as a
    failure. It used to be counted as a pass and dropped, which is exactly the
    trust problem comptab_parity.py was written to get away from: the number
    the harness prints has to be the number of cells that agreed EVERY time.

    Returns (passed, failed, fail_records) where each fail_record is
    (case, ref_grid, test_grid, diffs|None); a flaky record carries its last
    failing capture."""
    ref_argv = [args.zsh, "-f", "-i"]
    test_argv = [args.zshrs, "--zsh", "-f", "-i"]
    source_cmd = f"source {shlex.quote(init_file)}\n".encode()

    def capture(argv, label, case):
        sess = ShellSession(argv, env, args.rows, args.cols, label, args.settle)
        try:
            sess.drain_settled(max_wait=3.0, first_wait=2.0)
            sess.send(source_cmd)
            if not sess.wait_for_prompt(timeout=25.0):
                return None
            return run_case(sess, case)
        finally:
            sess.close()

    def attempt(case):
        ref_grid = capture(ref_argv, "zsh", case)
        test_grid = capture(test_argv, "zshrs", case)
        if ref_grid is None or test_grid is None:
            return (ref_grid, test_grid, None)  # never-reached-prompt
        return (ref_grid, test_grid, diff_grids(ref_grid, test_grid))

    passed = failed = 0
    fails = []
    for case in cases:
        ref_grid, test_grid, diffs = attempt(case)
        if not diffs:  # empty diff list = pass (None = prompt failure, handled below)
            if diffs == []:
                passed += 1
                continue
        # Either a real diff or a prompt failure — re-run only to LABEL it.
        flaky = False
        for _ in range(max(0, confirm)):
            r2, t2, d2 = attempt(case)
            if d2 == []:            # passed on re-run -> nondeterministic
                flaky = True
                break
            ref_grid, test_grid, diffs = r2, t2, d2  # keep the latest failing capture
        failed += 1
        fails.append((case, ref_grid, test_grid, diffs, "FLAKY" if flaky else "FAIL"))
    return passed, failed, fails


# ── terminal geometry ────────────────────────────────────────────────────────
#
# Rows and columns are not cosmetic for completion. The column count decides how
# many columns compsys packs a listing into and where a long line wraps; the row
# count decides whether the listing is paged ("do you wish to see all N
# possibilities"), how far the display scrolls, and how much of the prompt the
# redraw has to reconstruct. Two real bugs in this project were geometry-only:
# a completion list that climbed upward under a multi-line prompt, and a
# SIGWINCH-triggered infinite `zrefresh` recursion that reproduced only at tiny
# row counts. A fuzzer pinned to 24x80 cannot see either.
#
# BOTH shells always get the SAME geometry: one Geom drives the TIOCSWINSZ of
# both children and the COLUMNS/LINES of both environments. The fuzz varies
# which geometry a CELL runs under, never which shell gets which.
Geom = namedtuple("Geom", "rows cols")

GEOMETRY_POOL: list = [
    Geom(24, 80),    # the default every other harness uses
    Geom(24, 40),    # narrow: single-column listings, mid-word wrapping
    Geom(8, 80),     # tiny rows: forces the list pager
    Geom(6, 100),    # tinier still: the SIGWINCH / zrefresh-recursion shape
    Geom(40, 200),   # wide: many listing columns, nothing wraps
    Geom(30, 120),   # ordinary large terminal
    Geom(12, 60),    # awkward middle: pages AND wraps
]


def geom_str(g) -> str:
    return f"{g.rows}x{g.cols}"


def pick_geom(rng, args):
    """The geometry for one cell. Without --geometry-fuzz this is exactly the
    explicit --rows/--cols, so the default behaviour is unchanged."""
    if not getattr(args, "geometry_fuzz", False):
        return Geom(args.rows, args.cols)
    return rng.choice(GEOMETRY_POOL)


# ── buffer surfaces ──────────────────────────────────────────────────────────
#
# `--combo-commands` is a fixed five-entry list ("git ", "ssh -", "cd /",
# "kill -", "echo $PA"). Everything the completion engine does BEFORE it picks a
# completer — the word-splitting, quoting, cursor-position and special-context
# analysis in get_comp_string / _main_complete — is therefore never fuzzed at
# all. Each Surface below is one of those pre-completer contexts, with a small
# seeded family of concrete buffers.
#
# `make(rng) -> (buffer_text, pre_keys)`. `pre_keys` are sent (and parity-
# asserted, like every other key) BEFORE the random key path, which is how the
# mid-word-cursor surface gets the cursor off the end of the word.
#
# `needs` names binaries the surface is about. When one is absent the surface is
# DROPPED AT GENERATION TIME and counted under `unavailable-surface` in the
# summary — the harness declines to claim coverage of a completer this host does
# not have. It never drops a cell that was already generated, and it never
# converts a divergence into anything but a divergence.
@dataclass
class Surface:
    name: str
    note: str
    make: object
    needs: tuple = ()


def _pick(rng, *choices):
    return rng.choice(choices)


BUFFER_SURFACES: list = [
    Surface("midword", "cursor parked mid-word before TAB (suffix must survive)",
            lambda rng: (_pick(rng, "ls /usr/share/zsh", "ls /usr/local/bin",
                               "cd /usr/share/man", "ls /etc/paths.d"),
                         ["left"] * rng.randint(1, 4))),
    Surface("dquote", "inside an unterminated double quote",
            lambda rng: (_pick(rng, 'ls "', 'ls "/us', 'echo "$HO',
                               'ls "/usr/sh'), [])),
    Surface("squote", "inside an unterminated single quote",
            lambda rng: (_pick(rng, "ls '", "ls '/us", "ls '/usr/sh"), [])),
    Surface("param", "$var prefix — parameter-name completion context",
            lambda rng: (_pick(rng, "echo $PA", "echo $HO", "print $ZSH_",
                               "echo $fpa"), [])),
    Surface("braceparam", "${ prefix — braced parameter context",
            lambda rng: (_pick(rng, "echo ${PA", "echo ${HO", "echo ${fpat",
                               "echo ${#PA"), [])),
    Surface("glob", "glob metacharacters in the word being completed",
            lambda rng: (_pick(rng, "ls /usr/*", "ls /etc/?", "ls /usr/[bl]",
                               "ls /usr/sh*/", "ls /usr/**/z"), [])),
    Surface("brace", "brace expansion in the word being completed",
            lambda rng: (_pick(rng, "ls /usr/{bin,lo", "echo {a,b}",
                               "ls /{etc,usr}/", "ls /usr/{share,li"), [])),
    Surface("tilde", "~ / ~user / ~/ prefixes",
            lambda rng: (_pick(rng, "ls ~", "cd ~/", "ls ~roo", "ls ~/.z"), [])),
    Surface("redir", "redirection target — completes as a file, not an argument",
            lambda rng: (_pick(rng, "echo hi > /tm", "cat < /et",
                               "echo x >> /var/lo", "ls 2> /tm"), [])),
    Surface("assign", "assignment RHS — completes as a value, not a command",
            lambda rng: (_pick(rng, "FOO=/us", "PATH=/bin:/us", "X=/et",
                               "typeset Y=/usr/sh"), [])),
    Surface("cmdsubst", "interior of $( ) — a nested command position",
            lambda rng: (_pick(rng, "echo $(ls /us", "echo $(cd /et",
                               "echo $(print $HO"), [])),
    Surface("bslash", "backslash-escaped space inside a path",
            lambda rng: (_pick(rng, "ls /tmp/a\\ b", "ls foo\\ ba",
                               "ls /usr/sh\\ "), [])),
    Surface("sudo", "sudo-prefixed — the completer re-dispatches on argv[1]",
            lambda rng: (_pick(rng, "sudo ls /us", "sudo -u root ls /et",
                               "sudo "), []), ("sudo",)),
    Surface("opt_partial", "a partially typed long option",
            lambda rng: (_pick(rng, "git log --on", "git commit --amen",
                               "git diff --stat-c"), []), ("git",)),
    Surface("opt_equals", "a long option whose =VALUE is being completed",
            lambda rng: (_pick(rng, "git log --format=", "git log --pretty=on",
                               "git log --date="), []), ("git",)),
    Surface("subcmd", "subcommand position of a dispatching command",
            lambda rng: (_pick(rng, "git ", "git checkout ", "git rebase --",
                               "ssh -", "kill -", "cd /"), []), ("git",)),
]


# ── multiline / continuation surfaces ────────────────────────────────────────
#
# Every buffer above — and every case in every sibling harness — is ONE physical
# line. Completion re-parses the entire buffer through the lexer on each TAB
# (`get_comp_string`), so the same word reached through a CONTINUATION is a
# different parse: the lexer is resumed inside an open quote / open `$( )` /
# open compound command, `CURRENT` and the word offsets are computed over a
# buffer containing newlines, and the redisplay has PS2 rows above the cursor
# that the completion list has to be drawn around. None of that is exercised by
# a single-line corpus.
#
# Every buffer here is deliberately INCOMPLETE at each newline, so the Return
# that ends a line is answered with PS2 by both shells and nothing is ever
# executed. `\n` in the text is that Return — see ShellSession.buffer_lines.
#
# These compose with the ordinary buffer surfaces (`--buffer-fuzz
# --multiline-fuzz` fuzzes both pools) and with `--geometry-fuzz`, which is
# where they earn the most: a completion list drawn under a multi-row prompt is
# exactly the shape of the bug this project has already shipped once (a list
# that climbed the screen with a two-line prompt).
MULTILINE_SURFACES: list = [
    Surface("ml_backslash", "word completed after a trailing \\ continuation",
            lambda rng: (_pick(rng,
                               "ls /usr/share \\\n/usr/lo",
                               "cd \\\n/usr/sh",
                               "echo hi \\\n/etc/pa"), [])),
    Surface("ml_dquote", "unterminated double quote spanning lines",
            lambda rng: (_pick(rng,
                               'echo "first line\n/usr/sh',
                               'ls "one\ntwo/etc/pa',
                               'echo "a b\n$HO'), [])),
    Surface("ml_squote", "unterminated single quote spanning lines",
            lambda rng: (_pick(rng,
                               "echo 'first line\n/usr/sh",
                               "ls 'one\ntwo/etc/pa"), [])),
    Surface("ml_cmdsubst", "$( ) spanning lines — nested command position",
            lambda rng: (_pick(rng,
                               "echo $(\nls /usr/sh",
                               "echo $(ls /usr\nprint /etc/pa",
                               "echo $(\ncd /usr/lo"), [])),
    Surface("ml_backtick", "backquoted command substitution spanning lines",
            lambda rng: (_pick(rng,
                               "echo `\nls /usr/sh",
                               "echo `ls /usr\nprint /etc/pa"), [])),
    Surface("ml_for_body", "for body, after the newline that follows `do`",
            lambda rng: (_pick(rng,
                               "for f in a b\ndo\nls /usr/sh",
                               "for f in /etc/pa\ndo\ncd /usr/lo",
                               "for f in a b\ndo\nl"), [])),
    Surface("ml_while_body", "while body, after `do`",
            lambda rng: (_pick(rng,
                               "while true\ndo\nls /et",
                               "while true\ndo\nls /usr/sh"), [])),
    Surface("ml_if_body", "if body, after `then`",
            lambda rng: (_pick(rng,
                               "if true\nthen\nls /usr/sh",
                               "if true\nthen\ncd /et",
                               "if true\nthen\nl"), [])),
    Surface("ml_case_body", "case body, after the newline that follows `in`",
            lambda rng: (_pick(rng,
                               "case /usr in\n/usr/sh",
                               "case $x in\n  a) ls /et"), [])),
    Surface("ml_array", "array literal x=( spanning lines",
            lambda rng: (_pick(rng,
                               "x=(\n/usr/sh",
                               "x=( /usr/bin\n/usr/lo",
                               "typeset -a y=(\n/etc/pa"), [])),
    Surface("ml_heredoc_body", "inside a heredoc BODY",
            lambda rng: (_pick(rng,
                               "cat <<EOF\n/usr/sh",
                               "cat <<EOF\nline one\n/etc/pa"), [])),
    Surface("ml_heredoc_term", "on the heredoc TERMINATOR line",
            lambda rng: (_pick(rng,
                               "cat <<EOF\nbody\nEO",
                               "cat <<END\na\nEN"), [])),
    Surface("ml_pipe", "after a trailing | continuation",
            lambda rng: (_pick(rng,
                               "ls /usr |\nls /usr/sh",
                               "echo hi |\ngrep /et",
                               "ls /usr |\nl"), [])),
    Surface("ml_andor", "after a trailing && / || continuation",
            lambda rng: (_pick(rng,
                               "true &&\nls /usr/sh",
                               "false ||\ncd /et",
                               "true &&\nl"), [])),
]


# ── adversarial filesystem trees (`--fstree-fuzz`) ───────────────────────────
#
# Every path-completion case in this file and in every sibling harness completes
# against whatever this disk happens to contain — `/usr/`, `~/`, the repo. Path
# completion is the most-used surface in the shell and the one with the most
# quoting, globbing and escaping in it, and none of the inputs that are actually
# hostile to a completer are ever tried, because nothing constructs them.
#
# `--fstree-fuzz` builds a SEEDED, HERMETIC tree in a scratch directory, `cd`s
# BOTH shells into it (same absolute path, same init line) and completes inside
# it. The tree is a pure function of `--fstree-seed` and `--fstree-big`, so the
# seed printed in every failure line rebuilds the identical tree for a replay.
#
# Nothing here trusts the filesystem. Every planned entry is VERIFIED on disk
# after creation and classified created / folded / rejected:
#
#   created   the entry exists with the exact name that was asked for;
#   folded    the name landed on an entry that already existed (this is a
#             case-INSENSITIVE APFS volume, so `Coll.txt` and `coll.txt` are one
#             file, and pretending otherwise would invent a completion case that
#             cannot exist here);
#   rejected  the filesystem refused the name outright (ENAMETOOLONG, ...).
#
# Folded and rejected entries are a GENERATOR issue, counted and named in the
# report, and every surface built from a category is dropped when its entries
# did not land. They are never a parity finding.
FSTREE_NAME_LEN = 255          # APFS NAME_MAX, in bytes
FSTREE_DEEP_TARGET = 900       # abs path length to build toward (PATH_MAX 1024)


@dataclass
class FsPlanEntry:
    cat: str                   # completion category this entry belongs to
    rel: str                   # path relative to the tree root
    kind: str                  # "dir" | "file" | "link:<target>"
    mode: int = None           # chmod applied after creation (dirs only)
    note: str = ""


@dataclass
class FsTree:
    root: str
    seed: int
    tok: str
    plan: list
    created: list = field(default_factory=list)     # rel paths from the plan
    # Entries the plan cannot name because their COUNT is decided at creation
    # time: the deep chain is extended until the ABSOLUTE path approaches
    # PATH_MAX, which depends on how long the scratch root happens to be. Kept
    # apart from `created` so "planned N -> created N" reconciles exactly.
    runtime: list = field(default_factory=list)
    folded: list = field(default_factory=list)      # (rel, note)
    rejected: list = field(default_factory=list)    # (rel, reason)
    skipped: list = field(default_factory=list)     # (rel, reason)
    # cat -> [completion prefixes], only for categories whose entries landed.
    prefixes: dict = field(default_factory=dict)

    def ok(self, cat) -> bool:
        return bool(self.prefixes.get(cat))


def fstree_plan(seed: int, big_n: int) -> tuple:
    """(token, [FsPlanEntry, ...]) — the tree this seed asks for.

    Deterministic: same seed, same names, same order. The seeded token is woven
    into the directory and file names so two seeds are genuinely different
    trees, while a replay of one seed is byte-identical."""
    rng = random.Random(f"compsys-fstree:{seed}")
    tok = "".join(rng.choice("abcdefghijklmnopqrstuvwxyz") for _ in range(4))
    fill = rng.choice("LMNXYZ")
    p = []

    def d(cat, rel, mode=None, note=""):
        p.append(FsPlanEntry(cat, rel, "dir", mode, note))

    def f(cat, rel, note=""):
        p.append(FsPlanEntry(cat, rel, "file", None, note))

    def ln(cat, rel, target, note=""):
        p.append(FsPlanEntry(cat, rel, f"link:{target}", None, note))

    # ── whitespace in names ──────────────────────────────────────────────────
    sp = f"sp{tok}"
    d("space", sp)
    f("space", f"{sp}/a b.txt", "one space")
    f("space", f"{sp}/a bb.txt", "shares the 'a b' prefix — ambiguous WITH a space")
    f("space", f"{sp}/a\tb.txt", "literal TAB in the name")
    f("space", f"{sp}/ lead.txt", "LEADING space")
    f("space", f"{sp}/trail .txt", "TRAILING space before the extension")
    f("space", f"{sp}/nl\nline.txt", "literal NEWLINE in the name")

    # ── glob metacharacters as literal content ───────────────────────────────
    mt = f"meta{tok}"
    d("meta", mt)
    for name, note in (("star*.txt", "literal *"), ("q?.txt", "literal ?"),
                       ("br[x].txt", "literal [ ]"), ("br]y.txt", "unbalanced ]"),
                       ("brace{a}.txt", "literal { }"), ("tilde~.txt", "literal ~"),
                       ("hash#.txt", "literal #"), ("caret^.txt", "literal ^")):
        f("meta", f"{mt}/{name}", note)

    # ── quote / escape characters ────────────────────────────────────────────
    qt = f"quote{tok}"
    d("quote", qt)
    for name, note in (("sq'.txt", "single quote"), ('dq".txt', "double quote"),
                       ("bs\\.txt", "backslash"), ("back`.txt", "backtick"),
                       ("dollar$.txt", "dollar"), ("bang!.txt", "history bang")):
        f("quote", f"{qt}/{name}", note)

    # ── option-looking and dot names ─────────────────────────────────────────
    op = f"opt{tok}"
    d("optname", op)
    f("optname", f"{op}/-dash.txt", "leading -")
    f("optname", f"{op}/--ddash.txt", "leading --")
    f("optname", f"{op}/-", "a file named exactly -")
    d("dotname", f"{op}/.dotdir")
    f("dotname", f"{op}/.dot1", "dotfile")
    f("dotname", f"{op}/.dot2", "second dotfile — ambiguous among hidden names")
    f("dotname", f"{op}/..dotdot", "name beginning .. — adjacent to the ..  entry")
    f("dotname", f"{op}/.{tok}hidden", "seeded dotfile")

    # ── very long name, very deep path ───────────────────────────────────────
    lg = f"long{tok}"
    d("longname", lg)
    f("longname", f"{lg}/{fill * (FSTREE_NAME_LEN - len(tok))}{tok}",
      f"{FSTREE_NAME_LEN}-byte name (NAME_MAX)")
    f("longname", f"{lg}/{fill * 40}{tok}", "short sibling sharing the prefix")
    # deep chain: components are added until the ABSOLUTE path approaches
    # PATH_MAX; the exact count depends on the scratch root, so it is computed
    # at creation time and the plan just names the head.
    d("deep", f"deep{tok}")

    # ── ambiguity and a large listing ────────────────────────────────────────
    am = f"amb{tok}"
    d("ambiguous", am)
    for i in range(40):
        f("ambiguous", f"{am}/{tok}_common_{i:03d}", "shares a long common prefix")
    bg = f"big{tok}"
    d("bigdir", bg)
    for i in range(big_n):
        f("bigdir", f"{bg}/e{i:05d}", "one of a few thousand — the pager path")

    # ── symlinks ─────────────────────────────────────────────────────────────
    lk = f"link{tok}"
    d("symlink", lk)
    ln("symlink", f"{lk}/to_dir", f"../{am}", "symlink to a DIRECTORY")
    ln("symlink", f"{lk}/to_file", f"../{am}/{tok}_common_000",
       "symlink to a FILE")
    ln("symlink", f"{lk}/to_dangling", f"./nowhere_{tok}", "DANGLING symlink")
    ln("symlink_loop", f"{lk}/loop_a", "loop_b", "half of a symlink LOOP")
    ln("symlink_loop", f"{lk}/loop_b", "loop_a", "other half — resolves ELOOP")

    # ── unreadable / untraversable directories ───────────────────────────────
    pm = f"perm{tok}"
    d("perm", pm)
    f("perm", f"{pm}/{tok}_inside_noread", "created BEFORE the chmod")
    f("perm", f"{pm}/{tok}_inside_noexec", "created BEFORE the chmod")
    d("perm_noread", f"{pm}/noread", 0o333, "no READ permission (--wx)")
    f("perm_noread", f"{pm}/noread/{tok}_hidden_by_mode")
    d("perm_noexec", f"{pm}/noexec", 0o600, "no EXECUTE permission (rw-)")
    f("perm_noexec", f"{pm}/noexec/{tok}_hidden_by_mode")

    # ── case collision (this volume is case-INSENSITIVE; verified, not assumed)
    cs = f"case{tok}"
    d("casecoll", cs)
    f("casecoll", f"{cs}/Coll{tok}.txt", "upper-case first")
    f("casecoll", f"{cs}/coll{tok}.txt", "lower-case twin — folds on APFS")
    return tok, p


def _fs_make(root, e):
    """Create one planned entry. Returns (status, note): "created", "folded"
    (the name resolved onto an entry that already existed) or "rejected"."""
    path = os.path.join(root, e.rel)
    pre_existing = os.path.lexists(path)
    try:
        if e.kind == "dir":
            if pre_existing:
                return ("folded", "a directory of this name already resolved")
            os.mkdir(path)
        elif e.kind == "file":
            if pre_existing:
                st = os.lstat(path)
                return ("folded", f"resolved onto inode {st.st_ino} "
                                  f"(case-insensitive volume)")
            with open(path, "w") as fh:
                fh.write(e.rel + "\n")
        elif e.kind.startswith("link:"):
            if pre_existing:
                return ("folded", "a link of this name already resolved")
            os.symlink(e.kind[5:], path)
        else:                                        # pragma: no cover
            return ("rejected", f"unknown kind {e.kind!r}")
    except OSError as exc:
        return ("rejected", f"{type(exc).__name__}: {exc.strerror or exc}")
    return ("created", "")


def _fs_verify(root, e) -> tuple:
    """Read the entry BACK off the filesystem. `os.listdir` is the authority —
    `lexists` alone would accept a case-folded match on this volume and report a
    name as present that the shell will never see in a listing."""
    path = os.path.join(root, e.rel)
    parent = os.path.dirname(path) or root
    base = os.path.basename(path)
    try:
        names = os.listdir(parent)
    except OSError as exc:
        return (False, f"parent unlistable: {exc.strerror or exc}")
    if base in names:
        return (True, "")
    lowered = [n for n in names if n.lower() == base.lower()]
    if lowered:
        return (False, f"folded onto {lowered[0]!r}")
    return (False, "absent after creation")


def build_fstree(seed: int, big_n: int, root: str = None) -> FsTree:
    """Materialise the seeded tree and VERIFY it against its own plan.

    The tree root is derived from the seed, so a replay lands on the identical
    absolute path — which matters, because the path is baked into the `cd` line
    both shells run and into every buffer that completes inside it."""
    root = root or os.path.join(tempfile.gettempdir(),
                                f"compsys_parity_fstree_{seed}")
    if os.path.isdir(root):
        fstree_cleanup(root)
    os.makedirs(root, exist_ok=True)
    tok, plan = fstree_plan(seed, big_n)
    tree = FsTree(root=root, seed=seed, tok=tok, plan=plan)
    euid_root = (os.geteuid() == 0)
    chmods = []
    for e in plan:
        if e.cat.startswith("perm_") and euid_root:
            # As root a mode-000 directory is still readable, so the case would
            # test nothing. Named and counted, never quietly generated.
            tree.skipped.append((e.rel, "euid 0: permission bits do not apply"))
            continue
        status, note = _fs_make(root, e)
        if status == "rejected":
            tree.rejected.append((e.rel, note))
            continue
        if status == "folded":
            tree.folded.append((e.rel, note))
            continue
        ok, why = _fs_verify(root, e)
        if not ok:
            tree.folded.append((e.rel, why))
            continue
        tree.created.append(e.rel)
        if e.mode is not None:
            chmods.append((os.path.join(root, e.rel), e.mode))
    # The deep chain: components until the absolute path approaches PATH_MAX.
    deep_head = f"deep{tok}"
    deep_rel = deep_head
    mid_rel = deep_head
    if deep_head in tree.created:
        i = 0
        while len(os.path.join(root, deep_rel)) < FSTREE_DEEP_TARGET:
            nxt = os.path.join(deep_rel, f"d{i:02d}{tok}")
            try:
                os.mkdir(os.path.join(root, nxt))
            except OSError as exc:
                tree.rejected.append((nxt, f"{type(exc).__name__}: "
                                           f"{exc.strerror or exc}"))
                break
            deep_rel = nxt
            tree.runtime.append(nxt)
            # A MID-depth anchor as well as the near-PATH_MAX one: a buffer
            # ~900 characters long does not fit on a small terminal and would
            # be skipped as `fstree-buffer-exceeds-screen` at every narrow
            # geometry, so the deep category would only ever run on the wide
            # ones. The mid anchor keeps the category alive everywhere.
            if len(os.path.join(root, nxt)) < FSTREE_DEEP_TARGET // 4:
                mid_rel = nxt
            i += 1
        leaf = os.path.join(deep_rel, f"leaf_{tok}.txt")
        try:
            with open(os.path.join(root, leaf), "w") as fh:
                fh.write("deep\n")
            tree.runtime.append(leaf)
        except OSError as exc:
            tree.rejected.append((leaf, f"{type(exc).__name__}: "
                                        f"{exc.strerror or exc}"))
    # chmods LAST: the files inside an unreadable directory have to be created
    # while it is still writable.
    for path, mode in chmods:
        try:
            os.chmod(path, mode)
        except OSError as exc:                       # pragma: no cover
            tree.rejected.append((os.path.relpath(path, root),
                                  f"chmod: {exc.strerror or exc}"))
    tree.prefixes = fstree_prefixes(tree, deep_rel, mid_rel)
    return tree


def fstree_prefixes(tree: FsTree, deep_rel: str, mid_rel: str) -> dict:
    """Completion prefixes per category, built ONLY from entries that verified.

    A category whose entries did not land contributes nothing, so the harness
    never claims to have tested a completion case the filesystem refused to
    create."""
    made = set(tree.created)
    tok = tree.tok
    out: dict = {}

    def add(cat, *prefixes):
        keep = [p for p in prefixes if p]
        if keep:
            out.setdefault(cat, []).extend(keep)

    if f"sp{tok}/a b.txt" in made:
        add("space", f"sp{tok}/a", f"sp{tok}/a b", f"sp{tok}/a\\ b")
    if f"sp{tok}/ lead.txt" in made:
        add("space_edge", f"sp{tok}/", f"sp{tok}/tr")
    if f"sp{tok}/nl\nline.txt" in made:
        # The PREFIX is typed; the newline lives in the completion the shell has
        # to produce and quote. A raw newline is never typed into the buffer.
        add("space_newline", f"sp{tok}/nl")
    if f"meta{tok}/star*.txt" in made:
        add("meta", f"meta{tok}/star", f"meta{tok}/q", f"meta{tok}/br",
            f"meta{tok}/brace", f"meta{tok}/tilde", f"meta{tok}/hash",
            f"meta{tok}/caret", f"meta{tok}/")
    if f"quote{tok}/sq'.txt" in made:
        add("quote", f"quote{tok}/sq", f"quote{tok}/dq", f"quote{tok}/bs",
            f"quote{tok}/back", f"quote{tok}/dollar", f"quote{tok}/bang",
            f"quote{tok}/")
    if f"opt{tok}/-dash.txt" in made:
        add("optname", f"opt{tok}/-", f"opt{tok}/--", f"opt{tok}/")
    if f"opt{tok}/.dot1" in made:
        add("dotname", f"opt{tok}/.", f"opt{tok}/.d", f"opt{tok}/..",
            f"opt{tok}/.{tok}")
    if any(r.startswith(f"long{tok}/") for r in made):
        # The two long names share a prefix, so a short prefix is AMBIGUOUS and
        # a longer one is unique — both against a NAME_MAX-length candidate.
        longest = max((r for r in made if r.startswith(f"long{tok}/")),
                      key=len, default="")
        add("longname", f"long{tok}/",
            longest[:len(f"long{tok}/") + 20] if longest else "",
            longest[:len(f"long{tok}/") + 100] if longest else "")
    if f"deep{tok}" in made and tree.runtime:
        # The near-PATH_MAX directory itself, a mid-depth anchor that fits on a
        # small terminal, and the very first component (ambiguous with nothing,
        # but it exercises directory completion at the head of the chain).
        add("deep", deep_rel + "/", mid_rel + "/", f"deep{tok}/d0")
    if f"amb{tok}/{tok}_common_000" in made:
        add("ambiguous", f"amb{tok}/{tok}_c", f"amb{tok}/{tok}_common_0",
            f"amb{tok}/")
    if f"big{tok}/e00000" in made:
        add("bigdir", f"big{tok}/", f"big{tok}/e", f"big{tok}/e0000")
    if f"link{tok}/to_dir" in made:
        add("symlink", f"link{tok}/to_", f"link{tok}/to_dir/",
            f"link{tok}/to_dangling")
    if f"link{tok}/loop_a" in made:
        add("symlink_loop", f"link{tok}/loop_a/", f"link{tok}/loop_")
    if f"perm{tok}/noread" in made:
        add("perm_noread", f"perm{tok}/noread/", f"perm{tok}/nor")
    if f"perm{tok}/noexec" in made:
        add("perm_noexec", f"perm{tok}/noexec/", f"perm{tok}/noe")
    if f"case{tok}/Coll{tok}.txt" in made:
        add("casecoll", f"case{tok}/C", f"case{tok}/c", f"case{tok}/")
    return out


# The completion contexts each category is driven through. `cmd` varies so the
# same tree is reached through file completion (`ls`), directory-only completion
# (`cd`), a redirection target and a quoted word — the four paths that quote and
# escape a filename differently.
FSTREE_CMDS = ("ls ", "ls -l ", "cd ", "cat ", "echo ")


def fstree_surfaces(tree: FsTree, skips: Counter) -> list:
    """One Surface per tree category whose entries actually landed.

    A category the filesystem refused (or folded) is DROPPED here and counted
    under `fstree-category-absent:<cat>`, exactly like a missing binary is
    dropped from BUFFER_SURFACES — the harness does not claim coverage of a
    completion case that could not be constructed on this host."""
    notes = {
        "space": "filename containing a literal space",
        "space_edge": "leading/trailing space in a filename",
        "space_newline": "filename containing a literal NEWLINE",
        "meta": "glob metacharacters (* ? [ ] { } ~ # ^) as literal name content",
        "quote": "quote and escape characters (' \" \\ ` $ !) in a filename",
        "optname": "filename that looks like an option (-dash, --ddash, -)",
        "dotname": "dotfiles and a name beginning ..",
        "longname": "a NAME_MAX-length filename",
        "deep": "a directory chain near PATH_MAX",
        "ambiguous": "many entries sharing one long common prefix",
        "bigdir": "a directory with thousands of entries — the listing pager",
        "symlink": "symlinks to a dir, to a file, and dangling",
        "symlink_loop": "a symlink LOOP (resolves ELOOP)",
        "perm_noread": "a directory with no READ permission",
        "perm_noexec": "a directory with no EXECUTE permission",
        "casecoll": "case-colliding names on a case-insensitive volume",
    }
    out = []
    for cat, note in notes.items():
        prefixes = tree.prefixes.get(cat)
        if not prefixes:
            skips[f"fstree-category-absent:{cat}"] += 1
            continue

        def make(rng, _p=tuple(prefixes)):
            return (rng.choice(FSTREE_CMDS) + rng.choice(_p), [])

        out.append(Surface(f"fs_{cat}", note, make))
    return out


def fstree_report(tree: FsTree, limit=6) -> list:
    """Lines describing what the FILESYSTEM actually accepted, for the run
    header. This is the proof that the tree matched its specification; a
    category that folded or was rejected is named here and disabled above."""
    lines = [
        f"# fstree: seed={tree.seed} token={tree.tok!r} root={tree.root}",
        f"#   planned {len(tree.plan)} entries  ->  {len(tree.created)} created, "
        f"{len(tree.folded)} folded, {len(tree.rejected)} rejected, "
        f"{len(tree.skipped)} skipped",
        f"#   plus {len(tree.runtime)} run-time entries the plan cannot name "
        f"(the deep chain, sized against PATH_MAX from this scratch root)",
    ]
    for label, items in (("folded", tree.folded), ("rejected", tree.rejected),
                         ("skipped", tree.skipped)):
        for rel, why in items[:limit]:
            lines.append(f"#   {label:8s} {rel!r}: {why}")
        if len(items) > limit:
            lines.append(f"#   {label:8s} ... {len(items) - limit} more")
    lines.append("#   usable categories: "
                 + (", ".join(sorted(tree.prefixes)) or "NONE"))
    return lines


def fstree_cleanup(root: str):
    """Remove the tree, restoring the permission bits that would otherwise stop
    the walk. A replay rebuilds it from the seed, so nothing is lost."""
    if not root or not os.path.isdir(root):
        return
    for dirpath, dirnames, _ in os.walk(root):
        for name in dirnames:
            try:
                os.chmod(os.path.join(dirpath, name), 0o755)
            except OSError:
                pass
    shutil.rmtree(root, ignore_errors=True)


# ── interruption axis (`--interrupt-fuzz`) ───────────────────────────────────
#
# Nothing in this harness — or any sibling — ever interrupts a completion, and
# real completions are interrupted constantly: the window is resized, ^C is
# pressed, the next command is typed before the current one has finished
# drawing. This project has shipped a SIGWINCH-triggered infinite `zrefresh`
# recursion that reproduced only at tiny row counts, and a type-ahead-eaten-
# after-accept bug.
#
# Three kinds, delivered IDENTICALLY to both shells at one controlled point:
#
#   winch   a REAL terminal resize (TIOCSWINSZ on the pty master), which is what
#           raises SIGWINCH on the shell — not `kill -WINCH`, which would skip
#           the size change the handler reads.
#   int     SIGINT to the shell's process group, the way the tty driver delivers
#           a ^C.
#   type    a burst of type-ahead written in ONE write while the shell is still
#           computing.
#
# Three anchors:
#
#   before      after the buffer has settled, before the first TAB      (exact)
#   menu        after the first completion key has SETTLED, i.e. while a
#               listing / menu is on screen                             (exact)
#   midkey<N>   `--interrupt-delay-ms` after key N's write, BEFORE the screen is
#               drained — genuinely mid-computation, and therefore genuinely
#               racy: the two shells do not compute for the same length of time,
#               so the interrupt does not always land at the same point in each.
#               That is a property of the thing being tested, not a defect in
#               the harness; `--confirm` labels a non-reproducing divergence
#               FLAKY exactly as it does everywhere else.
INTERRUPT_KINDS = ("winch", "int", "type")

# Resize targets. Small row counts first: that is the shape the known zrefresh
# recursion needed. Both shells always get the SAME target.
WINCH_TARGETS = [Geom(6, 100), Geom(8, 40), Geom(24, 80), Geom(40, 200),
                 Geom(12, 60), Geom(30, 120)]

# Type-ahead payloads: a plain word, a word plus a TAB (a completion queued
# behind a completion), and a control character that is a widget in both keymaps.
TYPEAHEAD_PAYLOADS = ["ab", "z", "e\t", "ab\t", "\x02\x02", "xy"]


@dataclass
class Interrupt:
    kind: str                 # one of INTERRUPT_KINDS
    at: str                   # "before" | "menu" | "midkey<N>"
    geom: object = None       # winch: the target size
    payload: str = ""         # type: the bytes written

    def short(self) -> str:
        if self.kind == "winch":
            return f"winch->{geom_str(self.geom)}"
        if self.kind == "type":
            return f"type{self.payload!r}"
        return "SIGINT"

    def label(self) -> str:
        return f"{self.short()}@{self.at}"

    def midkey(self):
        """The 1-based key index this interrupt is anchored mid-computation to,
        or None."""
        if self.at.startswith("midkey"):
            try:
                return int(self.at[len("midkey"):])
            except ValueError:
                return None
        return None

    def apply(self, sess) -> bool:
        if self.kind == "winch":
            return sess.resize(self.geom.rows, self.geom.cols)
        if self.kind == "int":
            return sess.send_signal(signal.SIGINT)
        if self.kind == "type":
            if sess.exit_status() is not None:
                return False
            try:
                sess.send(self.payload.encode())
                return True
            except OSError:
                return False
        raise ValueError(f"unknown interrupt kind: {self.kind}")


def interrupt_encode(iv: Interrupt) -> str:
    if iv is None:
        return ""
    if iv.kind == "winch":
        return f"winch@{iv.at}:{iv.geom.rows}x{iv.geom.cols}"
    if iv.kind == "type":
        return f"type@{iv.at}:{_quote_payload(iv.payload)}"
    return f"int@{iv.at}"


def interrupt_decode(spec: str) -> Interrupt:
    """Parse `kind@anchor[:param]`. Strict — a typo is an error, never a
    silently-skipped interrupt that would make the cell read as a pass for an
    interruption that never happened."""
    kind, sep, rest = spec.partition("@")
    if not sep or kind not in INTERRUPT_KINDS:
        raise ValueError(f"{spec!r}: expected <{'|'.join(INTERRUPT_KINDS)}>@anchor")
    at, _, param = rest.partition(":")
    if at != "before" and at != "menu" and not at.startswith("midkey"):
        raise ValueError(f"{spec!r}: anchor must be before|menu|midkey<N>")
    if at.startswith("midkey"):
        try:
            n = int(at[len("midkey"):])
        except ValueError:
            raise ValueError(f"{spec!r}: midkey needs a key index, e.g. midkey1")
        if n < 1:
            raise ValueError(f"{spec!r}: midkey index is 1-based")
    if kind == "winch":
        try:
            r, c = param.split("x")
            geom = Geom(int(r), int(c))
        except Exception:
            raise ValueError(f"{spec!r}: winch needs a :ROWSxCOLS target")
        return Interrupt("winch", at, geom=geom)
    if kind == "type":
        payload = _unquote_payload(param)
        if not payload:
            raise ValueError(f"{spec!r}: type needs a payload")
        return Interrupt("type", at, payload=payload)
    return Interrupt("int", at)


def gen_interrupt(rng, kinds, presses) -> Interrupt:
    """One seeded interrupt. `menu` is only offered when there is a first key
    for the menu to be drawn by, and `midkey<N>` only for keys that exist."""
    kind = rng.choice(list(kinds))
    anchors = ["before"]
    if presses >= 1:
        anchors.append("menu")
        anchors += [f"midkey{n}" for n in range(1, min(presses, 3) + 1)]
    at = rng.choice(anchors)
    if kind == "winch":
        return Interrupt("winch", at, geom=rng.choice(WINCH_TARGETS))
    if kind == "type":
        return Interrupt("type", at, payload=rng.choice(TYPEAHEAD_PAYLOADS))
    return Interrupt("int", at)


def death_report(sessions) -> list:
    """[(label, kind, value), ...] for every session whose child has exited."""
    out = []
    for s in sessions:
        st = s.exit_status()
        if st is not None:
            out.append((s.label, st[0], st[1]))
    return out


def death_str(deaths) -> str:
    parts = []
    for label, kind, value in deaths:
        if kind == "signal":
            try:
                name = signal.Signals(value).name
            except ValueError:                        # pragma: no cover
                name = f"signal {value}"
            parts.append(f"{label} killed by {name}")
        else:
            parts.append(f"{label} exited {value}")
    return "; ".join(parts) or "no death"


def available_surfaces(skips: Counter, pool=None) -> list:
    """The surfaces usable on THIS host. Every dropped surface is counted under
    `unavailable-surface:<name>` and printed in the summary — never silently
    omitted.

    `pool` defaults to the single-line BUFFER_SURFACES, so every pre-existing
    caller is unchanged; `--multiline-fuzz` passes a pool that also carries
    MULTILINE_SURFACES."""
    out = []
    for s in (BUFFER_SURFACES if pool is None else pool):
        missing = [b for b in s.needs if shutil.which(b) is None]
        if missing:
            skips[f"unavailable-surface:{s.name}(no {','.join(missing)})"] += 1
            continue
        out.append(s)
    return out


def gen_buffer(rng, surfaces):
    """One fuzzed command line. Returns (surface_name, buffer, pre_keys)."""
    s = rng.choice(surfaces)
    text, pre = s.make(rng)
    return s.name, text, list(pre)


# ── edit surfaces ─────────────────────────────────────────────────────────────
#
# One entry per SHAPE of line editing, each producing a concrete edit program.
# `expect` is the command line the generator CLAIMS the program leaves behind.
# It is an assertion on the GENERATOR, checked against the REFERENCE shell and
# reported in its own counted category — never a comparison between the two
# shells, and never able to change a cell's parity verdict. `None` means the
# generator does not claim a final line (a pure cursor move whose effect is a
# cursor position, or a widget whose exact result is not worth asserting), and
# the parity comparison is unaffected either way.
@dataclass
class EditProg:
    gen: str
    mode: str
    buffer: str
    tokens: list
    expect: str = None
    note: str = ""


@dataclass
class EditSurface:
    name: str
    modes: tuple          # EDIT_MODES keys this shape is meaningful under
    note: str
    make: object          # make(rng) -> (buffer, tokens, expect)


EDIT_SURFACES: list = [
    # ── emacs ────────────────────────────────────────────────────────────────
    EditSurface("kill_word_retype", ("emacs", "emacs-nobp"),
                "^W the argument off, type a different one over it",
                lambda rng: _pick(rng,
                    ("ls /usr/local", [K("ctrl-w"), T("/usr/sh")], "ls /usr/sh"),
                    ("ls /usr/share/zsh", [K("ctrl-w"), T("/etc/pa")], "ls /etc/pa"),
                    ("cd /usr/share", [K("ctrl-w"), T("/usr/lo")], "cd /usr/lo"))),
    EditSurface("backspace_run", ("emacs", "emacs-nobp"),
                "a run of backspaces walks the word back to a completable prefix",
                lambda rng: _pick(rng,
                    ("ls /usr/shareXYZ", [K("bs")] * 3, "ls /usr/share"),
                    ("ls /usr/local/binQQ", [K("bs")] * 2, "ls /usr/local/bin"),
                    ("cd /etc/pathsQ", [K("bs")], "cd /etc/paths"))),
    EditSurface("kill_line_head", ("emacs", "emacs-nobp"),
                "^A ^K empties the line, a different command is typed over it",
                lambda rng: _pick(rng,
                    ("ls /usr/sh", [K("ctrl-a"), K("ctrl-k"), T("cd /et")], "cd /et"),
                    ("echo $PATH", [K("ctrl-a"), K("ctrl-k"), T("ls /usr/sh")],
                     "ls /usr/sh"))),
    EditSurface("motion_only", ("emacs", "emacs-nobp"),
                "line unchanged, cursor moved — completion fires mid-line",
                lambda rng: _pick(rng,
                    ("ls /usr/share/zsh", [K("ctrl-a"), K("alt-f")],
                     "ls /usr/share/zsh"),
                    ("ls /usr/share/zsh", [K("ctrl-e"), K("alt-b")],
                     "ls /usr/share/zsh"),
                    ("cd /usr/local/bin", [K("ctrl-a"), K("ctrl-f"), K("ctrl-f"),
                                           K("ctrl-f")], "cd /usr/local/bin"))),
    EditSurface("transpose", ("emacs", "emacs-nobp"),
                "^T / M-t rewrite the tail of the word in place",
                lambda rng: _pick(rng,
                    ("ls /usr/sh", [K("ctrl-t")], None),
                    ("ls /usr/share zsh", [K("alt-t")], None))),
    EditSurface("kill_yank", ("emacs", "emacs-nobp"),
                "^U into the kill ring, ^Y straight back out — same text, "
                "different history",
                lambda rng: _pick(rng,
                    ("ls /usr/sh", [K("ctrl-u"), K("ctrl-y")], "ls /usr/sh"),
                    ("cd /usr/share", [K("ctrl-u"), K("ctrl-y")], "cd /usr/share"))),
    EditSurface("undo_after_kill", ("emacs", "emacs-nobp"),
                "^W then undo — the line comes back, the completion must too",
                lambda rng: _pick(rng,
                    ("ls /usr/share", [K("ctrl-w"), K("ctrl-_")], "ls /usr/share"),
                    ("cd /usr/lo", [K("ctrl-w"), K("ctrl-x-ctrl-u")], "cd /usr/lo"))),
    EditSurface("paste_burst", ("emacs", "emacs-nobp"),
                "the argument arrives as ONE write() — the type-ahead path",
                lambda rng: _pick(rng,
                    ("ls ", [P("/usr/sh")], "ls /usr/sh"),
                    ("cd ", [P("/usr/share/z")], "cd /usr/share/z"),
                    ("ls ", [P("/usr/local/b")], "ls /usr/local/b"))),
    EditSurface("bracketed_paste", ("emacs", "emacs-nobp"),
                "the same burst inside ESC[200~ / ESC[201~ brackets",
                lambda rng: _pick(rng,
                    ("ls ", [B("/usr/sh")], "ls /usr/sh"),
                    ("cd ", [B("/usr/share/z")], "cd /usr/share/z"))),
    EditSurface("paste_then_edit", ("emacs", "emacs-nobp"),
                "paste a long path, then kill it and retype a different one",
                lambda rng: _pick(rng,
                    ("ls ", [P("/usr/share/zsh"), K("ctrl-w"), T("/etc/pa")],
                     "ls /etc/pa"),
                    ("cd ", [B("/usr/local/bin"), K("ctrl-w"), T("/usr/sh")],
                     "cd /usr/sh"))),
    EditSurface("midline_insert", ("emacs", "emacs-nobp"),
                "an option is inserted BEFORE an argument already on the line",
                lambda rng: _pick(rng,
                    ("ls /usr/sh", [K("ctrl-a"), K("alt-f"), T(" -l")],
                     "ls -l /usr/sh"),
                    ("ls /usr/share", [K("ctrl-a"), K("alt-f"), T(" -a")],
                     "ls -a /usr/share"))),

    # ── vi ───────────────────────────────────────────────────────────────────
    #
    # `bindkey -v` is a different keymap, and completion fired from a
    # vi-NORMAL-mode cursor is a surface nothing else in this repo's harnesses
    # reaches. A TAB in vicmd is not bound to a completion widget in `zsh -f`
    # at all — the two shells still have to do the same nothing.
    EditSurface("vi_x_delete", ("vi",),
                "ESC to vicmd, `x` the junk off the end, `A` back to insert",
                lambda rng: _pick(rng,
                    ("ls /usr/shXYZ", [K("esc"), T("xxx"), T("A")], "ls /usr/sh"),
                    ("cd /usr/shareQ", [K("esc"), T("x"), T("A")], "cd /usr/share"))),
    EditSurface("vi_dw", ("vi",),
                "`0` then `dw` drops the command word",
                lambda rng: _pick(rng,
                    ("ls /usr/share/zsh", [K("esc"), T("0"), T("dw")], None),
                    ("cd /usr/local", [K("esc"), T("0"), T("dw")], None))),
    EditSurface("vi_db", ("vi",),
                "`db` deletes backwards from the cursor",
                lambda rng: _pick(rng,
                    ("ls /usr/share/zsh", [K("esc"), T("db")], None),
                    ("cd /usr/local/bin", [K("esc"), T("b"), T("db")], None))),
    EditSurface("vi_dd_retype", ("vi",),
                "`dd` clears the line, `i` re-enters insert, retype",
                lambda rng: _pick(rng,
                    ("ls /usr/share", [K("esc"), T("dd"), T("i"), T("cd /et")],
                     "cd /et"),
                    ("echo $PATH", [K("esc"), T("dd"), T("i"), T("ls /usr/sh")],
                     "ls /usr/sh"))),
    EditSurface("vi_cw", ("vi",),
                "`cw` changes a word in place",
                lambda rng: _pick(rng,
                    ("ls /usr/share", [K("esc"), T("b"), T("cw"), T("etc")], None),
                    ("cd /usr/local", [K("esc"), T("cw"), T("sh")], None))),
    EditSurface("vi_motion", ("vi",),
                "line unchanged, cursor parked mid-line in vicmd — TAB from there",
                lambda rng: _pick(rng,
                    ("ls /usr/share/zsh", [K("esc"), T("b")], "ls /usr/share/zsh"),
                    ("ls /usr/share/zsh", [K("esc"), T("0")], "ls /usr/share/zsh"),
                    ("cd /usr/local/bin", [K("esc"), T("b"), T("h")],
                     "cd /usr/local/bin"),
                    ("ls /usr/sh", [K("esc"), T("0"), T("$")], "ls /usr/sh"),
                    ("ls /usr/sh", [K("esc"), T("e")], "ls /usr/sh"))),
    EditSurface("vi_repeat", ("vi",),
                "`x` then `.` — the repeat register has to carry the change",
                lambda rng: _pick(rng,
                    ("ls /usr/shXY", [K("esc"), T("x"), T("."), T("A")],
                     "ls /usr/sh"),
                    ("cd /usr/shareQQ", [K("esc"), T("x"), T("."), T("A")],
                     "cd /usr/share"))),
    EditSurface("vi_undo", ("vi",),
                "`dw` then `u` — the line comes back, the completion must too",
                lambda rng: _pick(rng,
                    ("ls /usr/share", [K("esc"), T("dw"), T("u"), T("A")],
                     "ls /usr/share"),
                    ("cd /usr/local", [K("esc"), T("db"), T("u"), T("A")],
                     "cd /usr/local"))),
    EditSurface("vi_insert_bol", ("vi",),
                "`I` inserts at the beginning of the line",
                lambda rng: _pick(rng,
                    ("s /usr/sh", [K("esc"), T("I"), T("l"), K("esc"), T("A")],
                     "ls /usr/sh"),
                    ("d /usr/share", [K("esc"), T("I"), T("c"), K("esc"), T("A")],
                     "cd /usr/share"))),
    EditSurface("vi_paste_burst", ("vi",),
                "a burst pasted in viins, then ESC to vicmd before TAB",
                lambda rng: _pick(rng,
                    ("ls ", [P("/usr/share/zsh"), K("esc"), T("b")],
                     "ls /usr/share/zsh"),
                    ("cd ", [B("/usr/local/bin"), K("esc")],
                     "cd /usr/local/bin"))),
]


def available_edit_surfaces(modes) -> list:
    """The edit surfaces meaningful under the requested mode set."""
    return [(s, m) for s in EDIT_SURFACES for m in s.modes if m in modes]


def gen_edit_program(rng, pairs) -> EditProg:
    """One fuzzed edit program. `pairs` comes from available_edit_surfaces."""
    surface, mode = rng.choice(pairs)
    buf, tokens, expect = surface.make(rng)
    return EditProg(gen=surface.name, mode=mode, buffer=buf,
                    tokens=edit_validate(list(tokens)), expect=expect,
                    note=surface.note)


# ── convergent edit programs ─────────────────────────────────────────────────
#
# A pair of DIFFERENT edit programs that produce the IDENTICAL final line. If
# the reference shell ends both legs on the same screen and zshrs does not, the
# difference cannot be "zshrs completes this line differently" — the line is
# the same — so it isolates EDIT-HISTORY LEAKAGE: state carried out of the
# editing that produced the line and into the completion that reads it. That is
# the sharpest available evidence for this bug class, which is why the pairs are
# generated as pairs rather than hoping two independent cells happen to land on
# the same text.
#
# Convergence is never assumed. Every pair is CHECKED empirically against the
# reference shell (`ref_A == ref_B`); a pair that does not converge there is
# reported as a counted SKIP, because the claim the comparison rests on could
# not be established. It is never scored as a pass.
@dataclass
class ConvPair:
    name: str
    mode: str
    target: str
    a: EditProg
    b: EditProg
    note: str = ""


def _conv(name, mode, target, a, b, note=""):
    """(buffer, tokens) pairs -> a ConvPair with both legs validated."""
    return ConvPair(
        name=name, mode=mode, target=target, note=note,
        a=EditProg(gen=f"{name}/A", mode=mode, buffer=a[0],
                   tokens=edit_validate(list(a[1])), expect=target),
        b=EditProg(gen=f"{name}/B", mode=mode, buffer=b[0],
                   tokens=edit_validate(list(b[1])), expect=target))


CONVERGENT_PAIRS: list = [
    _conv(
        "bs_vs_direct", "emacs", "ls /usr/sh",
        ("ls /usr/shXYZ", [K("bs")] * 3),
        ("ls /usr/sh", []),
        "three backspaces vs typing the line straight"),
    _conv(
        "killword_vs_bs", "emacs", "ls /usr/sh",
        ("ls /usr/local", [K("ctrl-w"), T("/usr/sh")]),
        ("ls /usr/local", [K("bs")] * 5 + [T("sh")]),
        "same start, same end, two different ways of getting there"),
    _conv(
        "prefix_vs_direct", "emacs", "ls /usr/sh",
        ("/usr/sh", [K("ctrl-a"), T("ls "), K("ctrl-e")]),
        ("ls /usr/sh", []),
        "the command word inserted in front of an argument already typed"),
    _conv(
        "paste_vs_typed", "emacs", "ls /usr/sh",
        ("ls ", [P("/usr/sh")]),
        ("ls /usr/sh", []),
        "argument delivered in one write vs typed character by character"),
    _conv(
        "bpaste_vs_typed", "emacs", "ls /usr/sh",
        ("ls ", [B("/usr/sh")]),
        ("ls /usr/sh", []),
        "bracketed paste vs typed"),
    _conv(
        "killyank_vs_direct", "emacs", "ls /usr/sh",
        ("ls /usr/sh", [K("ctrl-u"), K("ctrl-y")]),
        ("ls /usr/sh", []),
        "the line round-tripped through the kill ring"),
    _conv(
        "undo_vs_direct", "emacs", "ls /usr/share",
        ("ls /usr/share", [K("ctrl-w"), K("ctrl-_")]),
        ("ls /usr/share", []),
        "killed and undone vs never touched"),
    _conv(
        "vi_x_vs_direct", "vi", "ls /usr/sh",
        ("ls /usr/shX", [K("esc"), T("x")]),
        ("ls /usr/sh", [K("esc")]),
        "both legs end in vicmd on the last character"),
    _conv(
        "vi_dw_vs_direct", "vi", "/usr/sh",
        ("ls /usr/sh", [K("esc"), T("0"), T("dw"), T("A")]),
        ("/usr/sh", [K("esc"), T("A")]),
        "command word deleted vs never typed"),
    _conv(
        "vi_undo_vs_direct", "vi", "ls /usr/share",
        ("ls /usr/share", [K("esc"), T("dw"), T("u"), T("A")]),
        ("ls /usr/share", [K("esc"), T("A")]),
        "vi undo vs never edited"),
    _conv(
        "vi_paste_vs_typed", "vi", "ls /usr/sh",
        ("ls ", [P("/usr/sh"), K("esc"), T("A")]),
        ("ls /usr/sh", [K("esc"), T("A")]),
        "burst pasted in viins vs typed"),
]


def gen_conv_pair(rng, modes) -> ConvPair:
    """One convergent pair, restricted to the requested editing modes."""
    usable = [p for p in CONVERGENT_PAIRS if p.mode in modes]
    if not usable:
        return None
    return rng.choice(usable)


def line_after_prompt(grid, cols):
    """The command line as the screen shows it, or None if it cannot be read
    unambiguously.

    Returns None (rather than a guess) when the line is long enough to have
    wrapped: pyte pads every row to the window width and `normalize_rows`
    rstrips it, which destroys the information needed to rejoin a wrapped line
    exactly. A guess there would make the generator-sanity check report
    mismatches that are artifacts of the reader."""
    for row in grid:
        idx = row.find(PROMPT_SENTINEL)
        if idx < 0:
            continue
        text = row[idx + len(PROMPT_SENTINEL):]
        if text.startswith(" "):
            text = text[1:]
        # Prompt + text filling the row means the next row may be a wrap.
        if idx + len(PROMPT_SENTINEL) + 1 + len(text) >= cols:
            return None
        return text
    return None


# ── keystroke paths ──────────────────────────────────────────────────────────
#
# The vocabulary deliberately EXCLUDES:
#   cr / ctrl-o  — accept-line; would run the fuzzed buffer as a command.
#   ctrl-u / ctrl-h / ctrl-w — they empty the line, after which a later ctrl-d
#                  is EOF and kills the shell mid-cell (a harness artefact, not
#                  a completion difference).
#   esc / esc-esc — a bare ESC makes the NEXT random letter a meta binding
#                  (M-d kill-word, ...), which lands back in the empty-line case
#                  above. Meta keys belong in the fixed corpus, not the fuzz.
# Everything else that navigates, pages, cycles, aborts or filters is in.
_NAV_KEYS = ["down", "up", "left", "right", "ctrl-n", "ctrl-p",
             "ctrl-f", "ctrl-b", "home", "end", "pgup", "pgdn"]
_LETTERS = "abcdefghijklmnopqrstuvwxyz"


def gen_keyseq(rng, length):
    """Generate a random lockstep key path: always start with a TAB (list +
    enter menu-select), then a random mix of more TABs (cycle/page), reverse
    TABs, arrows and emacs motion keys (menu navigation), pager keys, ctrl-d
    (list-choices), ctrl-g (abort the menu) and literal letters (interactive
    filter narrowing)."""
    seq = ["tab"]
    for _ in range(max(0, length - 1)):
        r = rng.random()
        if r < 0.30:
            seq.append("tab")
        elif r < 0.55:
            seq.append(rng.choice(_NAV_KEYS))
        elif r < 0.62:
            seq.append("btab")
        elif r < 0.67:
            seq.append("ctrl-d")          # list-choices without inserting
        elif r < 0.72:
            seq.append("ctrl-g")          # send-break — abort the menu
        else:
            seq.append(rng.choice(_LETTERS))  # interactive-filter keystroke
    return seq


def saved_path(outdir, seed, n, suffix=""):
    return os.path.join(outdir, f"combo_{seed}_{n}{suffix}.zsh")


def run_keyseq(init_file, buffer, keys, args, env, geom, edits=None,
               timings_out=None, interrupt=None, deaths_out=None):
    """Drive `zsh -f` and `zshrs --zsh -f` in LOCKSTEP: source init, type
    `buffer`, optionally run an EDIT PROGRAM over it, then send each key in
    `keys` one at a time, capturing + byte-diffing BOTH screens AFTER EACH
    STEP. `keys` may mix "tab", arrows ("down"/"up"/...), and literal filter
    characters ("a".."z") — so menu-cycling, list-prompt paging, arrow
    navigation, AND interactive-filter narrowing (typing letters to filter the
    menu) are all verified per-key.

    `edits` (None = no edit phase, the pre-existing behaviour) is a list of
    edit-program tokens (see the DSL above `EDIT_KEYS`). When it is supplied
    the screens are ALSO diffed once right after the buffer is typed and once
    after every token, so a divergence is attributed to the exact edit that
    caused it rather than to the completion that followed it.

    `geom` sizes BOTH ptys and BOTH environments identically.

    `timings_out` (None = not measuring, the pre-existing behaviour) is a list
    the per-key `(key, ref_timing, test_timing)` triples are appended to. When
    it is supplied the two ptys are drained CONCURRENTLY (one select loop over
    both) instead of one after the other, because the sequential drain charges
    the second shell for the first shell's entire wait — see drain_concurrent.
    Nothing about what is compared changes; only the read interleaving does.

    A `\\n` in `buffer` is typed as a real Return on a deliberately incomplete
    line, so both shells continue with PS2 — that is the multiline surface.

    `interrupt` (None = the pre-existing behaviour, nothing is ever delivered)
    is one Interrupt applied IDENTICALLY to both shells at its anchor: a real
    TIOCSWINSZ resize, a SIGINT to the shell's process group, or a type-ahead
    burst. See the Interrupt comment block.

    `deaths_out` (None = not collected) is a list the harness appends
    `(side, kind, value)` to as soon as either child is seen to have exited.
    A shell that DIED is its own verdict upstream — never a pass and never a
    plain FAIL — so the run stops there and names which side went and how.

    Returns (fail_step, records): fail_step is the 1-based index of the first
    STEP whose screens diverge (0 if all match); records is
    [(step, label, ref_grid, test_grid, diffs), ...]. Stops at first divergence
    (the two shells desync past that point)."""
    source_cmd = f"source {shlex.quote(init_file)}\n".encode()
    env = dict(env, COLUMNS=str(geom.cols), LINES=str(geom.rows))
    ref = ShellSession([args.zsh, "-f", "-i"], env, geom.rows, geom.cols, "zsh", args.settle)
    test = ShellSession([args.zshrs, "--zsh", "-f", "-i"], env, geom.rows, geom.cols, "zshrs", args.settle)
    records = []
    step = [0]
    measuring = timings_out is not None

    def drain_both(max_wait, first_wait):
        """Settle both shells. Concurrent only while measuring — the sequential
        form is what every existing verdict in this repo was scored under and
        it stays the default."""
        if measuring:
            drain_concurrent((ref, test), max_wait=max_wait,
                             first_wait=first_wait)
        else:
            for s in (ref, test):
                s.drain_settled(max_wait=max_wait, first_wait=first_wait)

    def compare(label):
        """One parity assertion. Appends a record and returns its diffs."""
        step[0] += 1
        rg = normalize_rows(ref.grid())
        tg = normalize_rows(test.grid())
        d = diff_grids(rg, tg)
        records.append((step[0], label, rg, tg, d))
        return d

    def died():
        """True once either child has exited, with the fact recorded.

        Checked after EVERY drain, not only under --interrupt-fuzz: a shell that
        crashed mid-cell is a finding in its own right, and the alternative is
        comparing a live shell's screen against a dead one's last frame and
        calling the result a completion divergence (or, when the frames happen
        to match, a pass). WNOHANG only — nothing here ever blocks."""
        d = death_report((ref, test))
        if not d:
            return False
        if deaths_out is not None:
            deaths_out.extend(d)
        return True

    def interrupt_at(anchor, iv):
        """Deliver `iv` to BOTH shells when its anchor matches. Returns the
        diffs of the assertion that follows, or None when nothing was
        delivered."""
        if iv is None or iv.at != anchor:
            return None
        for s in (ref, test):
            iv.apply(s)
        drain_both(max_wait=10.0, first_wait=2.0)
        return compare("intr:" + iv.label())

    delay = max(0.0, getattr(args, "interrupt_delay_ms", 40) / 1000.0)

    try:
        for s in (ref, test):
            s.drain_settled(max_wait=3.0, first_wait=2.0)
            s.send(source_cmd)
            if not s.wait_for_prompt(timeout=25.0):
                # Record a death HERE too: a shell that crashed during init also
                # "never reached a prompt", and round 3 established that
                # labelling that as a timeout is how a real segfault stayed
                # hidden for two rounds.
                died()
                return (1, [(1, "(init)", None, None, None)])
        for s in (ref, test):
            s.fresh_prompt()
        if buffer:
            # One write per PHYSICAL line, both shells in step: a single-line
            # buffer is exactly the one write it always was, and a multiline
            # one lets each PS2 continuation land on both shells before the
            # next line is typed.
            for chunk in ref.buffer_lines(buffer):
                for s in (ref, test):
                    s.send(chunk)
                drain_both(max_wait=2.0, first_wait=1.0)
        if edits is not None:
            # Baseline assertion: if the two shells already disagree on the
            # plain typed line, no edit below caused it and the report must not
            # say one did.
            if compare("(buffer)"):
                return (step[0], records)
            for tok in edits:
                for s in (ref, test):
                    s.send_edit_token(tok)
                # An edit is a local line redraw, not a completion: short
                # first-byte wait, but a real quiet window (a paste burst on a
                # narrow terminal reflows several rows).
                drain_both(max_wait=6.0, first_wait=1.5)
                if compare("edit:" + edit_label(tok)):
                    return (step[0], records)
                if died():
                    return (step[0], records)
        # An interrupt anchored BEFORE the first completion key: the buffer is
        # on screen and settled, nothing is computing.
        if interrupt_at("before", interrupt):
            return (step[0], records)
        if died():
            return (step[0], records)
        for kn, key in enumerate(keys, 1):
            for s in (ref, test):
                s.send_key(key)
            # An interrupt anchored MID-COMPUTATION: the key has been written to
            # both shells and neither screen has been drained, so the shells are
            # still working on it. The delay is nominal and identical on both
            # sides, but the two shells do not compute for the same length of
            # time, so where it lands inside each is not identical — that is
            # inherent to interrupting a running computation, and --confirm
            # labels a non-reproducing divergence FLAKY as usual.
            mid = interrupt.midkey() if interrupt is not None else None
            key_label = key
            if mid == kn:
                if delay:
                    select.select([], [], [], delay)
                for s in (ref, test):
                    interrupt.apply(s)
                key_label = f"{key}!intr:{interrupt.short()}"
            # the FIRST completion keystroke is cold (autoload chain) → long
            # first-byte wait; later keys are warm menu redraws / filter edits.
            # Keyed on position within the KEY phase, not on the global step
            # index: with an edit program in front, the cold key is no longer
            # step 1 and it would otherwise get the warm (4s) window.
            fw = 8.0 if kn == 1 else 4.0
            drain_both(max_wait=12.0, first_wait=fw)
            if measuring:
                timings_out.append((key, ref.timing, test.timing))
            if compare(key_label):
                return (step[0], records)
            if died():
                return (step[0], records)
            # An interrupt anchored at MENU: the first completion key has
            # settled, so a listing / menu is on screen and the resize (or ^C,
            # or type-ahead) lands on a shell that is DISPLAYING a menu rather
            # than computing one. This is the exact anchor the known
            # SIGWINCH/zrefresh recursion needed.
            if kn == 1 and interrupt_at("menu", interrupt):
                return (step[0], records)
            if died():
                return (step[0], records)
        return (0, records)
    finally:
        ref.close()
        test.close()


def first_diff_cell(diffs):
    """(row, col) of the first differing CELL, not just the first differing row."""
    row, a, b = diffs[0]
    col = next((i for i, (x, y) in enumerate(zip(a, b)) if x != y),
               min(len(a), len(b)))
    return row, col


def signature(records):
    """Fingerprint of a divergence, used as the delta-debugging invariant.

    Deliberately NOT the step index: shrinking the key path changes the index of
    the failing key, so keying on it would reject every real reduction. The
    (row, col) of the first differing cell is what "the same bug" means here —
    a candidate that diverges somewhere ELSE on the screen is a DIFFERENT
    divergence and must not be accepted as a reduction of this one.

    ("boot", -1) marks the un-shrinkable case where a shell never reached a
    prompt."""
    step, key, rg, tg, diffs = records[-1]
    if not diffs:
        return None
    return first_diff_cell(diffs)


# ── delta debugging ──────────────────────────────────────────────────────────

def shrink_edits(cell, args, env, target_sig, budget, run):
    """Reduce the EDIT PROGRAM to a subsequence that still diverges at
    `target_sig`.

    A minimal edit program is the whole value of an edit-fuzz finding:
    "diverges after `ls ` + paste + ^W + retype + TAB + down + q" is not a bug
    report, "diverges after `ls /usr/local` + ^W + TAB" is. Same ddmin, same
    first-diff-cell invariant, same probe budget as the other two axes.

    Nothing is pinned here (unlike the key path, whose leading TAB is what
    makes it a completion at all): every token of an edit program is a
    candidate for removal, including the first."""
    if budget <= 0 or not cell.edit_tokens:
        return list(cell.edit_tokens or []), 0
    probes = [0]

    def still_fails(candidate):
        probes[0] += 1
        fs, rec = run(cell.buffer, cell.keys, cell.init_file, cell.geom,
                      list(candidate))
        return bool(fs) and signature(rec) == target_sig

    # THE EMPTY PROGRAM IS PROBED FIRST, and it is the most valuable candidate
    # of all: if the divergence survives with no edits at all, the edit phase is
    # not implicated and the finding is an ordinary completion bug that this
    # mode merely happened to be running when it fired. `parity_corpus.shrink`
    # cannot discover that on its own — its inner loop does
    # `if not candidate: continue`, so the empty set is never a candidate and a
    # one-token program is its floor whether or not that token matters. Without
    # this probe the harness would report "reduced to 1 token" for a bug the
    # token has nothing to do with, which is a claim about causation it has not
    # earned.
    if still_fails([]):
        return [], probes[0]
    if len(cell.edit_tokens) == 1:
        return list(cell.edit_tokens), probes[0]
    return (ddmin(list(cell.edit_tokens), still_fails,
                  max_probes=max(0, budget - probes[0])),
            probes[0])


def shrink_keys(cell, args, env, target_sig, budget, run, edits=None):
    """Reduce the key path to a subsequence that still diverges at `target_sig`.

    "diverges at step #7 (path tab+down+q+tab+j+up+tab)" is a report you cannot
    act on; "diverges after tab+q" is. Uses parity_corpus.shrink (ddmin) over
    the key list, with `still_fails` re-running the whole lockstep. Bounded by
    `budget` probes because each probe boots two shells.

    The first key is pinned: every path starts with the TAB that enters
    completion, and a candidate without it tests a different thing entirely."""
    if budget <= 0 or len(cell.keys) <= 1:
        return list(cell.keys), 0
    probes = [0]

    def still_fails(candidate):
        probes[0] += 1
        keys = [cell.keys[0]] + list(candidate)
        fs, rec = run(cell.buffer, keys, cell.init_file, cell.geom, edits)
        return bool(fs) and signature(rec) == target_sig

    tail = ddmin(list(cell.keys[1:]), still_fails, max_probes=budget)
    return [cell.keys[0]] + tail, probes[0]


def shrink_styles(cell, args, env, target_sig, budget, run, build_init, keys,
                  edits=None):
    """Reduce the zstyle set to a subset that still diverges at `target_sig`.

    A 100-statement random subset that diverges says nothing; the one or two
    statements that actually cause it are the bug report. Reuses the same ddmin
    from parity_corpus that comptab_parity uses for its combos."""
    if budget <= 0 or len(cell.statements) <= 1:
        return list(cell.statements), 0
    probes = [0]

    def still_fails(candidate):
        probes[0] += 1
        init = build_init(list(candidate))
        fs, rec = run(cell.buffer, keys, init, cell.geom, edits)
        return bool(fs) and signature(rec) == target_sig

    return ddmin(list(cell.statements), still_fails, max_probes=budget), probes[0]


# ── fuzz cells ───────────────────────────────────────────────────────────────

@dataclass
class Cell:
    idx: int
    # Unique per CELL ("<combo>_<case>"), not per combo: two cells of the same
    # combo can both fail and both write a shrunk fixture, and a per-combo name
    # would have them overwrite each other's replay file (and race at --jobs>1).
    uid: str
    surface: str
    buffer: str
    keys: list
    geom: object
    statements: list
    zstyle_path: str
    init_file: str
    workdir: str
    # ── edit-fuzz (all default to the pre-existing "no edit phase" shape) ──
    # `edit_tokens is None` means run_keyseq takes the original path: buffer
    # typed, no baseline assertion, straight into the key path.
    edit_tokens: list = None
    edit_mode: str = None
    edit_gen: str = ""
    expect: str = None
    # ── fstree-fuzz: the hermetic tree BOTH shells are cd'd into (None = the
    # pre-existing behaviour, no `cd` line in the init at all). `fstree_seed` is
    # carried so every failure line and every replay can rebuild the identical
    # tree from the seed alone.
    cwd: str = None
    fstree_seed: int = None
    # ── interrupt-fuzz: one Interrupt applied identically to both shells at its
    # anchor (None = nothing is ever delivered, the pre-existing behaviour).
    interrupt: object = None


@dataclass
class ConvCell(Cell):
    """A convergent PAIR. `buffer`/`edit_tokens` are leg A (so every field the
    ordinary reporting path reads is populated); leg B lives alongside."""
    b_buffer: str = ""
    b_edit_tokens: list = field(default_factory=list)
    b_gen: str = ""
    target: str = ""
    conv_note: str = ""


@dataclass
class SessionCell(Cell):
    """A SESSION: N completion episodes run inside ONE pair of shells.

    `buffer`/`keys` stay empty — a session has no single buffer and no single
    key path, and populating them with the first episode's would make every
    report line quietly claim the cell was about that one completion. Everything
    a session is lives in `episodes`."""
    episodes: list = field(default_factory=list)
    repeat: int = 1


@dataclass
class CellResult:
    cell: object
    # PASS | FAIL | FLAKY | SKIP | DIED. DIED is its OWN verdict: a shell that
    # exited mid-cell is never a pass and never a plain FAIL — the report has to
    # name which side went and with what signal, because a crashed reference
    # shell mislabelled as a timeout is how a real upstream zsh segfault stayed
    # hidden for two rounds of this tooling.
    status: str = "PASS"
    detail: str = ""
    fail_step: int = 0
    fail_key: str = ""
    sig: object = None
    diffs: list = field(default_factory=list)
    ref_grid: list = field(default_factory=list)
    test_grid: list = field(default_factory=list)
    min_keys: list = None
    min_styles: list = None
    probes: int = 0
    # True when a ddmin pass stopped because it ran out of probes rather than
    # because it converged. The reduction is still valid (every kept element was
    # shown to be load-bearing under the probes that DID run) but it is not a
    # minimal set, and the report must not claim it is.
    shrink_exhausted: bool = False
    replay: str = ""
    min_edits: list = None
    # Which axes the reduction actually exercised, so the report never implies
    # a key path was minimised when the divergence happened before any key was
    # sent.
    shrink_notes: list = field(default_factory=list)
    # Generator-sanity annotation, NOT a parity verdict: whether the reference
    # shell's command line matched what the edit generator claimed it would be.
    expect_ok: object = None
    expect_saw: str = None
    # --latency only: [(key, ref_KeyTiming, test_KeyTiming), ...] already
    # reduced to best-of-K. Never consulted by anything that decides `status`.
    latency: list = None
    # [(side, "signal"|"exit", value), ...] when status == "DIED".
    deaths: list = field(default_factory=list)
    # ── session-fuzz only (all empty for every other mode) ────────────────────
    # The raw SessionRun, kept so the reporter can print per-episode detail.
    session: object = None
    # Probes that moved DIFFERENTLY on the two shells: the actionable finding.
    drifts: list = field(default_factory=list)
    # Probes that moved IDENTICALLY on both shells — zsh's own accumulation,
    # faithfully reproduced. Counted and printed, never a verdict.
    shared_drift: object = None
    # The cross-shell table difference that existed BEFORE any completion ran.
    baseline_diffs: list = field(default_factory=list)
    # NAMED reasons a state probe could not be read. A probe that did not run is
    # never reported as a probe that agreed.
    probe_failures: list = field(default_factory=list)
    # {"zsh": [...], "zshrs": [...]} — where a shell did not render the SAME
    # episode the same way twice, judged per shell before the two are compared.
    idempotence: object = None
    # The reduced episode sequence for a drift finding.
    min_episodes: list = None


def replay_command(args, buffer, keys, geom, zstyle_path,
                   edits=None, editing_mode=None, fstree_seed=None,
                   interrupt=None):
    """A copy-pasteable command that reproduces exactly this divergence.

    `--fstree-seed` is what makes an fstree finding replayable: the tree is a
    pure function of the seed (and `--fstree-big`), so the replay REBUILDS the
    identical tree at the identical absolute path before typing the buffer. The
    seed therefore appears in every fstree failure line, not just in the run
    header."""
    extra = ""
    if edits is not None:
        extra += f" --edit-program {shlex.quote(edit_encode(edits))}"
    if editing_mode:
        extra += f" --editing-mode {editing_mode}"
    if fstree_seed is not None:
        extra += (f" --fstree-seed {fstree_seed}"
                  f" --fstree-big {getattr(args, 'fstree_big', 3000)}")
    if interrupt is not None:
        extra += f" --interrupt {shlex.quote(interrupt_encode(interrupt))}"
        extra += f" --interrupt-delay-ms {getattr(args, 'interrupt_delay_ms', 40)}"
    return ("scripts/compsys_parity.py --lockstep"
            f" --seed {args.seed}"
            f" --zstyle {shlex.quote(zstyle_path)}"
            f" --case {shlex.quote(buffer)}"
            f" --keys {','.join(keys)}"
            + extra
            + f" --rows {geom.rows} --cols {geom.cols} -v")


def conv_replay_command(args, cell, keys, zstyle_path):
    """A copy-pasteable command that re-runs one convergent PAIR.

    A convergent finding is a claim about TWO runs, so a single-leg `--lockstep`
    line cannot reproduce it. The whole pair travels as one JSON argument."""
    spec = {
        "name": cell.edit_gen,
        "mode": cell.edit_mode,
        "target": cell.target,
        "geom": [cell.geom.rows, cell.geom.cols],
        "zstyle": zstyle_path,
        "keys": list(keys),
        "a": {"buffer": cell.buffer, "edits": edit_encode(cell.edit_tokens)},
        "b": {"buffer": cell.b_buffer, "edits": edit_encode(cell.b_edit_tokens)},
    }
    return ("scripts/compsys_parity.py --conv-replay "
            + shlex.quote(json.dumps(spec, separators=(",", ":"))) + " -v")


def measure_latency(cell, args, env):
    """Time one cell's key path K times and keep the BEST of each side.

    Deliberately its own runs rather than a reading taken off the verdict run:
    the verdict run drains sequentially (the shape every existing result in
    this repo was produced under) and a timing taken from it would charge the
    second shell for the first shell's wait. These runs drain both ptys in one
    select loop instead, which is the only configuration whose numbers mean
    anything — and they are compared exactly as usual, so a run that diverges
    stops where it always stops and simply contributes no sample for the keys
    it never reached.
    """
    runs = []
    for _ in range(max(1, args.latency_runs)):
        t = []
        run_keyseq(cell.init_file, cell.buffer, cell.keys, args, env,
                   cell.geom, edits=cell.edit_tokens, timings_out=t)
        runs.append(t)
    return merge_best_timings(runs)


def lat_cell_id(cell) -> str:
    return f"{cell.surface}#{cell.idx}.{cell.uid}"


def lat_worst(merged, min_delta):
    """The slowest REPORTABLE key of one cell, or None.

    "Reportable" is the absolute-delta floor: below it the ratio is scheduler
    noise wearing a percentage sign, and this returns None rather than a
    number nobody should act on."""
    best = None
    for i, (label, rt, tt) in enumerate(merged or [], 1):
        if (rt is None or tt is None
                or rt.settle is None or tt.settle is None or rt.settle <= 0):
            continue
        if tt.settle - rt.settle < min_delta:
            continue
        ratio = tt.settle / rt.settle
        if best is None or ratio > best[0]:
            best = (ratio, i, label, rt.settle, tt.settle)
    return best


def run_cell(cell, args, env, dump, fpath_dirs, outdir) -> CellResult:
    """One fuzz cell: lockstep run, flake labelling, then delta debugging."""
    if isinstance(cell, ConvCell):
        return run_conv_cell(cell, args, env, dump, fpath_dirs, outdir)
    if isinstance(cell, SessionCell):
        return run_session_cell(cell, args, env, dump, fpath_dirs, outdir)
    res = CellResult(cell=cell)

    def run(buffer, keys, init_file, geom, edits=None, deaths_out=None):
        # `edits=None` means "whatever the cell currently carries" — which is
        # the ORIGINAL program on the first run and the REDUCED one after
        # shrink_edits rewrote it. A default argument would have frozen the
        # original list object at def time and quietly re-run the long program
        # in every later probe.
        return run_keyseq(init_file, buffer, keys, args, env, geom,
                          edits=cell.edit_tokens if edits is None else edits,
                          interrupt=cell.interrupt, deaths_out=deaths_out)

    deaths: list = []
    fail_step, records = run(cell.buffer, cell.keys, cell.init_file, cell.geom,
                             deaths_out=deaths)
    if deaths:
        # A shell DIED. This is neither a pass nor an ordinary divergence, and
        # the report says which side and with what signal instead of comparing a
        # live screen against a dead one's last frame. Not shrunk: ddmin's
        # oracle is "the same first-diff cell", and a cell that has no screens
        # to diff has no such invariant.
        res.status = "DIED"
        res.deaths = deaths
        # --confirm re-runs LABEL the death, exactly as they label a divergence,
        # and exactly as there they can only ever ADD information: the verdict
        # stays DIED either way and the run still exits non-zero. The label
        # matters because this box kills processes under memory pressure while
        # sixteen agents share it, and "died once, not again" is a different
        # claim from "dies every time" — the first is worth re-running, the
        # second is worth reporting upstream.
        again = 0
        for _ in range(max(0, args.confirm)):
            d2: list = []
            run(cell.buffer, cell.keys, cell.init_file, cell.geom,
                deaths_out=d2)
            if d2:
                again += 1
        res.detail = (death_str(deaths)
                      + (f"; reproduced in {again}/{args.confirm} re-runs"
                         if args.confirm else "")
                      + ("  — did NOT reproduce, so this may be the machine "
                         "(memory pressure) rather than the shell"
                         if args.confirm and again == 0 else ""))
        step, key, rg, tg, diffs = records[-1]
        res.fail_step, res.fail_key = step, key
        res.ref_grid, res.test_grid = rg or [], tg or []
        res.diffs = diffs or []
        res.replay = replay_command(args, cell.buffer, cell.keys, cell.geom,
                                    cell.zstyle_path, cell.edit_tokens,
                                    cell.edit_mode, cell.fstree_seed,
                                    cell.interrupt)
        return res
    # Generator sanity: did the REFERENCE shell end the edit phase on the line
    # the generator claims? Recorded and counted on its own; it can neither
    # create nor suppress a parity verdict.
    _check_expect(res, cell, records)
    # Latency, measured HERE — after the verdict-producing run and BEFORE any
    # shrinking rewrites `cell.keys`/`cell.edit_tokens`, so the times belong to
    # the cell as generated. These are extra runs whose SCREENS are not scored:
    # the parity verdict above is already fixed and nothing below can move it.
    if getattr(args, "latency", False):
        res.latency = measure_latency(cell, args, env)
    if fail_step == 0:
        res.status = "PASS"
        return res

    step, key, rg, tg, diffs = records[-1]
    res.fail_step, res.fail_key = step, key
    res.ref_grid, res.test_grid = rg or [], tg or []
    if diffs is None:
        # A shell that never reached a prompt is a FAILURE, not a skip: the
        # comparison was attempted and one side could not be observed.
        res.status = "FAIL"
        res.detail = "a shell never reached prompt"
        res.replay = replay_command(args, cell.buffer, cell.keys, cell.geom,
                                    cell.zstyle_path, cell.edit_tokens,
                                    cell.edit_mode, cell.fstree_seed,
                                    cell.interrupt)
        return res

    res.diffs = diffs
    res.sig = signature(records)

    # --confirm re-runs LABEL the failure; they never turn it into a pass. A
    # cell that diverges once and not again is NONDETERMINISTIC, which is its
    # own bug class (worker-pool tty races have produced exactly this), so it is
    # reported as FLAKY in its own counted category — not quietly dropped, and
    # not promoted to a clean divergence either.
    reproduced = True
    for _ in range(max(0, args.confirm)):
        fs2, rec2 = run(cell.buffer, cell.keys, cell.init_file, cell.geom)
        if fs2 == 0 or signature(rec2) != res.sig:
            reproduced = False
            break
        records = rec2
    res.status = "FAIL" if reproduced else "FLAKY"
    res.detail = f"{len(diffs)} rows differ"

    min_keys, min_styles, min_edits = None, None, None
    zstyle_for_replay = cell.zstyle_path
    # Only a REPRODUCIBLE failure is shrunk: ddmin's oracle is "does this
    # candidate still diverge at the same cell", and a flaky oracle would
    # happily delete keys that matter. A FLAKY cell keeps its full path.
    if args.shrink_probes > 0 and res.status == "FAIL":
        # Edit program FIRST: it runs before the keys, so a shorter one usually
        # makes every later probe cheaper as well as making the report shorter.
        if cell.edit_tokens:
            full = len(cell.edit_tokens)
            min_edits, p0 = shrink_edits(cell, args, env, res.sig,
                                         args.shrink_probes, run)
            res.probes += p0
            res.shrink_exhausted = p0 >= args.shrink_probes
            cell.edit_tokens = min_edits   # later probes run the reduced program
            res.shrink_notes.append(
                f"edit program reduced {len(min_edits)}/{full} tokens"
                + ("  — the divergence survives with NO edits at all, so the "
                   "edit phase is not implicated: this is an ordinary "
                   "completion divergence on the typed buffer"
                   if not min_edits else ""))
        # A divergence that happened DURING the edit phase was reached before a
        # single completion key was sent, so the key path is not implicated and
        # reducing it would only spend probes proving that. Say so instead of
        # printing a reduction that means nothing.
        edit_phase_failure = isinstance(res.fail_key, str) and (
            res.fail_key.startswith("edit:") or res.fail_key == "(buffer)")
        if edit_phase_failure:
            res.shrink_notes.append(
                "key path not reduced: the divergence happens in the edit "
                "phase, before any completion key is sent")
            min_keys = list(cell.keys)
        else:
            min_keys, p1 = shrink_keys(cell, args, env, res.sig,
                                       args.shrink_probes, run,
                                       cell.edit_tokens)
            res.probes += p1
            res.shrink_exhausted = res.shrink_exhausted or p1 >= args.shrink_probes

        def build_init(subset):
            d = tempfile.mkdtemp(prefix="shrink_", dir=cell.workdir)
            path = os.path.join(d, "zstyle.zsh")
            with open(path, "w") as f:
                f.write("\n".join(subset) + "\n")
            return build_init_file(dump, fpath_dirs, path, cell.edit_mode,
                                   cell.cwd)

        min_styles, p2 = shrink_styles(cell, args, env, res.sig,
                                       args.shrink_probes, run, build_init,
                                       min_keys, cell.edit_tokens)
        res.probes += p2
        res.shrink_exhausted = res.shrink_exhausted or p2 >= args.shrink_probes
        if len(min_styles) < len(cell.statements):
            zstyle_for_replay = saved_path(outdir, args.seed, cell.uid, ".min")
            with open(zstyle_for_replay, "w") as f:
                f.write(f"# shrunk from {len(cell.statements)} statements "
                        f"(seed={args.seed} combo={cell.idx} "
                        f"surface={cell.surface} geom={geom_str(cell.geom)})\n")
                f.write("\n".join(min_styles) + "\n")
    res.min_keys, res.min_styles, res.min_edits = min_keys, min_styles, min_edits
    res.replay = replay_command(args, cell.buffer, min_keys or cell.keys,
                                cell.geom, zstyle_for_replay,
                                cell.edit_tokens, cell.edit_mode,
                                cell.fstree_seed, cell.interrupt)
    return res


def _check_expect(res, cell, records):
    """Compare the REFERENCE shell's command line after the edit phase against
    what the generator said the program would produce.

    This is an assertion on the HARNESS, not on zshrs. It is recorded and
    counted separately and never touches the parity verdict: a mismatch means
    a generator's `expect` is wrong (or the line wrapped and could not be read
    back), and suppressing the cell over it would throw away a perfectly good
    zsh-vs-zshrs comparison."""
    if cell.expect is None or not records:
        return
    last_edit = None
    for step, label, rg, tg, diffs in records:
        if label == "(buffer)" or (isinstance(label, str)
                                   and label.startswith("edit:")):
            last_edit = rg
    if last_edit is None:
        return
    saw = line_after_prompt(last_edit, cell.geom.cols)
    res.expect_saw = saw
    res.expect_ok = (saw == cell.expect) if saw is not None else None


def run_conv_cell(cell, args, env, dump, fpath_dirs, outdir) -> CellResult:
    """One CONVERGENT PAIR: two different edit programs that are claimed to end
    on the same line, run under the identical init, geometry and key path.

    Four verdicts, in the order the evidence supports them:

      1. either leg diverges zsh-vs-zshrs per step  -> the ordinary FAIL/FLAKY,
         reported against that leg (the pair adds nothing the single-leg report
         does not already say);
      2. the reference shell does NOT end both legs on the same screen -> SKIP
         `non-convergent-in-reference`. The pair's whole claim is that the two
         programs converge; if that could not be established in zsh, the
         comparison it enables was not made and must not be scored as a pass;
      3. reference identical, zshrs different              -> FAIL, edit-history
         leakage: zshrs carries something out of HOW the line was built into
         what completion does with it;
      4. reference identical, zshrs identical              -> PASS.
    """
    res = CellResult(cell=cell)
    # Liveness is checked on the PAIR path too. Without it, a cell in which BOTH
    # shells died would leave two identical (empty) screens and score as
    # "both programs converge on both shells" — a pass produced by two corpses.
    pair_deaths: list = []

    def run(buffer, keys, init_file, geom, edits):
        return run_keyseq(init_file, buffer, keys, args, env, geom, edits=edits,
                          deaths_out=pair_deaths)

    def both_legs(keys, init_file):
        """(bad_leg, records_A, records_B). `bad_leg` names the leg that
        diverged zsh-vs-zshrs on its own, in which case only that leg's records
        are returned (the pair cannot be compared past it)."""
        fa, ra = run(cell.buffer, keys, init_file, cell.geom, cell.edit_tokens)
        if fa:
            return ("legA", ra, None)
        fb, rb = run(cell.b_buffer, keys, init_file, cell.geom,
                     cell.b_edit_tokens)
        if fb:
            return ("legB", rb, None)
        return (None, ra, rb)

    leg, ra, rb = both_legs(cell.keys, cell.init_file)
    if leg is not None:
        # Case 1 — a plain per-step divergence on ONE leg, before the pair could
        # be compared at all. A leg is an ordinary edit cell in every respect,
        # so hand it to the ordinary single-leg path and let it get the SAME
        # --confirm labelling and the SAME three-axis shrinking every other edit
        # cell gets.
        #
        # This is not a refactor for tidiness. The first leg divergence this
        # harness produced (`vi_dw_vs_direct` leg A, seed 11) was reported as a
        # flat FAIL by an earlier version of this function, which re-ran
        # nothing — and the replay it printed then PASSED. A divergence that
        # does not reproduce is FLAKY, and the rest of this file exists to make
        # sure the harness says so; the convergent path must not be the one
        # place that quietly does not.
        leg_cell = Cell(
            idx=cell.idx, uid=f"{cell.uid}_{leg}",
            surface=f"{cell.surface}/{leg}",
            buffer=cell.buffer if leg == "legA" else cell.b_buffer,
            keys=list(cell.keys), geom=cell.geom, statements=cell.statements,
            zstyle_path=cell.zstyle_path, init_file=cell.init_file,
            workdir=cell.workdir,
            edit_tokens=list(cell.edit_tokens if leg == "legA"
                             else cell.b_edit_tokens),
            edit_mode=cell.edit_mode,
            edit_gen=f"{cell.edit_gen}/{leg}",
            expect=cell.target,
            cwd=cell.cwd, fstree_seed=cell.fstree_seed,
            interrupt=cell.interrupt)
        return run_cell(leg_cell, args, env, dump, fpath_dirs, outdir)

    if pair_deaths:
        # Checked BEFORE the convergence comparison: two dead shells draw
        # identical (empty) screens, which is not evidence that two edit
        # programs converge.
        res.status = "DIED"
        res.deaths = pair_deaths
        res.detail = death_str(pair_deaths)
        res.fail_step, res.fail_key = len(ra), "(convergence)"
        res.replay = conv_replay_command(args, cell, cell.keys, cell.zstyle_path)
        return res

    ref_a, test_a = ra[-1][2], ra[-1][3]
    ref_b, test_b = rb[-1][2], rb[-1][3]
    if ref_a != ref_b:
        # Case 2 — the pair did not converge in the REFERENCE shell.
        res.status = "SKIP"
        res.detail = ("non-convergent-in-reference: zsh itself ends the two "
                      "programs on different screens, so the pair proves "
                      "nothing about zshrs")
        res.ref_grid, res.test_grid = ref_a, ref_b
        res.diffs = diff_grids(ref_a, ref_b)
        res.replay = conv_replay_command(args, cell, cell.keys, cell.zstyle_path)
        return res

    conv_diffs = diff_grids(test_a, test_b)
    if not conv_diffs:
        res.status = "PASS"
        res.detail = "both programs converge on both shells"
        return res

    # Case 3 — edit-history leakage.
    res.diffs = conv_diffs
    res.sig = first_diff_cell(conv_diffs)
    res.ref_grid, res.test_grid = test_a, test_b
    res.detail = (f"edit-history leakage: zsh ends both programs on the SAME "
                  f"screen, zshrs on {len(conv_diffs)} differing rows")
    res.fail_step, res.fail_key = len(ra), "(convergence)"

    def leakage_reproduces(keys, init_file):
        lg, _, xa, xb, _ = both_legs(keys, init_file)
        if lg is not None or xa is None or xb is None:
            return False
        r_a, t_a = xa[-1][2], xa[-1][3]
        r_b, t_b = xb[-1][2], xb[-1][3]
        d = diff_grids(t_a, t_b)
        return r_a == r_b and bool(d) and first_diff_cell(d) == res.sig

    reproduced = True
    for _ in range(max(0, args.confirm)):
        if not leakage_reproduces(cell.keys, cell.init_file):
            reproduced = False
            break
    res.status = "FAIL" if reproduced else "FLAKY"

    min_keys = list(cell.keys)
    min_styles = None
    zstyle_for_replay = cell.zstyle_path
    if args.shrink_probes > 0 and res.status == "FAIL":
        # The EDIT programs are deliberately NOT shrunk here. Removing a token
        # from one leg changes the line that leg produces, which destroys the
        # convergence the whole finding rests on — a "reduction" that no longer
        # compares two programs with the same final line is not a reduction of
        # this bug, it is a different (and unproven) claim. The key path and
        # the zstyle set are identical on both legs, so both reduce soundly.
        res.shrink_notes.append(
            "edit programs not reduced: removing a token breaks the "
            "convergence the finding depends on")
        probes = [0]

        def keys_still_leak(candidate):
            probes[0] += 1
            return leakage_reproduces([cell.keys[0]] + list(candidate),
                                      cell.init_file)

        if len(cell.keys) > 1:
            tail = ddmin(list(cell.keys[1:]), keys_still_leak,
                         max_probes=args.shrink_probes)
            min_keys = [cell.keys[0]] + tail
            res.probes += probes[0]
            res.shrink_exhausted = probes[0] >= args.shrink_probes

        sprobes = [0]

        def styles_still_leak(candidate):
            sprobes[0] += 1
            d = tempfile.mkdtemp(prefix="shrink_", dir=cell.workdir)
            path = os.path.join(d, "zstyle.zsh")
            with open(path, "w") as f:
                f.write("\n".join(candidate) + "\n")
            return leakage_reproduces(
                min_keys, build_init_file(dump, fpath_dirs, path,
                                          cell.edit_mode, cell.cwd))

        if len(cell.statements) > 1:
            min_styles = ddmin(list(cell.statements), styles_still_leak,
                               max_probes=args.shrink_probes)
            res.probes += sprobes[0]
            res.shrink_exhausted = (res.shrink_exhausted
                                    or sprobes[0] >= args.shrink_probes)
            if len(min_styles) < len(cell.statements):
                zstyle_for_replay = saved_path(outdir, args.seed,
                                               cell.uid, ".min")
                with open(zstyle_for_replay, "w") as f:
                    f.write(f"# shrunk from {len(cell.statements)} statements "
                            f"(convergent pair {cell.edit_gen}, seed="
                            f"{args.seed}, geom={geom_str(cell.geom)})\n")
                    f.write("\n".join(min_styles) + "\n")
    res.min_keys, res.min_styles = min_keys, min_styles
    res.replay = conv_replay_command(args, cell, min_keys, zstyle_for_replay)
    return res


# ── session fuzz (`--session-fuzz`) ──────────────────────────────────────────
#
# Every other mode in this file boots two shells, types once, completes once and
# kills them. A real session runs thousands of completions in ONE process, and
# the bugs that only a long-lived process can show are invisible to a one-shot
# cell by construction. This project has confirmed several of exactly that
# shape: completion-time state leaking into the parameter table, a
# `$compstate[old_list]` that reports `yes` where zsh reports `shown` and only on
# the SECOND invocation, `_tags_level` desyncing from `$#funcstack` across a
# dispatch chain, and a completion list that is valid versus currently-shown.
#
# A SESSION is one pair of shells running N EPISODES in sequence. An episode is
# "clear the screen, type a buffer, complete it key by key, end the line", and
# parity is asserted after EVERY step of EVERY episode — not once at the end.
#
# THE FUZZED BUFFER IS NEVER EXECUTED. An episode ends one of two ways, chosen
# by the seed and delivered identically to both shells:
#
#   abort    ^G (send-break, leaves any menu) then ^U (kill-whole-line).
#   accept   the same, and THEN a fixed literal `true` plus Return. accept-line,
#            the post-command hook chain and the history append are on the path
#            that leaks state between completions, so a session that never
#            accepts anything does not exercise them — but running a FUZZED line
#            would execute arbitrary text, so what is accepted is always the
#            same harmless builtin.
#
# `^D` is deliberately not used to end anything: on an empty line it is EOF and
# would kill the shell mid-session, which is a harness artefact, not a finding.
EPISODE_ENDS = ("abort", "accept")

# Delivered through `send_edit_token`, so an episode terminator takes exactly the
# same code path as an edit-fuzz token — there is no second key writer.
END_TOKENS: dict[str, list] = {
    "abort": ["k:ctrl-g", "k:ctrl-u"],
    "accept": ["k:ctrl-g", "k:ctrl-u", "t:true", "k:cr"],
}


# ── state probes ─────────────────────────────────────────────────────────────
#
# Between episodes both shells are asked to write their own state to a FILE and
# the files are diffed here. Reading it off the pyte grid instead would cap every
# answer at the window width and lose it to wrapping, and the whole point is to
# name the parameters that drifted rather than to count them.
#
# SET probes are compared as SETS OF NAMES; a finding names the names.
# LINE probes are compared as sets of `key=value` LINES, so a changed value
# shows up as one removed line and one added line, both printed.
PROBE_SET_FILES = ("parameters", "functions", "aliases", "galiases", "commands")
PROBE_LINE_FILES = ("options", "setopt", "misc")
PROBE_FILES = PROBE_SET_FILES + PROBE_LINE_FILES

# What the state axis deliberately does NOT compare, and why. Printed in the
# run header: an exclusion that is not named is an exclusion nobody can audit.
PROBE_EXCLUDED = (
    "parameter VALUES — only the NAME SET of $parameters is compared, so a "
    "parameter whose value legitimately changes every command (HISTCMD, "
    "SECONDS, EPOCHSECONDS, RANDOM, LINENO, _, ?) cannot manufacture a finding; "
    "a parameter that APPEARS or DISAPPEARS still does",
    "$? — the probe command itself sets it",
    "history size / HISTCMD — every episode appends to history by design; a "
    "history that did not grow would be the bug",
    "$$ / $PPID — two live processes cannot share a pid",
)

# The probe itself. Defined in the init file, so it exists before the FIRST
# probe and is therefore in `$functions` on both shells at the baseline: it can
# never appear as a drift. It writes with `>|` so a leaked `noclobber` cannot
# silence it, and it takes no `emulate -L` (that would reset the very options
# the `setopt` probe exists to read).
#
# It takes only a TAG (`e3r1`) and resolves the directory through
# `$_CP_PROBE_ROOT`, an environment variable whose VALUE differs per shell and
# whose NAME is identical on both. That is not decoration. The probe command is
# accepted at the prompt, so it lands in the shell's history — and the moment a
# fuzzed key path contains `up` / `ctrl-p` / `pgup`, history recall puts that
# command line back on the screen. With the directory spelled out in the
# command, the two shells necessarily recalled two different paths (`.../zsh/
# e1r1` vs `.../zshrs/e1r1`) and the harness reported its own bookkeeping as a
# completion divergence — the same class of self-inflicted mismatch
# `_mask_pid` exists for. With the tag form both shells recall a byte-identical
# line, and the differing value lives in a variable whose name (not value) is
# what the parameter probe compares.
PROBE_FUNCTION = r"""_cp_probe() {
  local d=$_CP_PROBE_ROOT/$1 k
  print -rl -- ${(ok)parameters} >| $d/parameters
  print -rl -- ${(ok)functions}  >| $d/functions
  print -rl -- ${(ok)aliases}    >| $d/aliases
  print -rl -- ${(ok)galiases}   >| $d/galiases
  print -rl -- ${(ok)commands}   >| $d/commands
  for k in ${(ok)options}; do print -r -- "$k=$options[$k]"; done >| $d/options
  setopt >| $d/setopt
  {
    print -r -- "funcstack_depth=$#funcstack"
    print -r -- "compstate_set=${+compstate}"
    print -r -- "widget_set=${+WIDGET} widget=${WIDGET-}"
    print -r -- "lastwidget_set=${+LASTWIDGET} lastwidget=${LASTWIDGET-}"
    print -r -- "tags_level_set=${+_tags_level} tags_level=${_tags_level-}"
    print -r -- "comp_tags_set=${+_comp_tags} comp_tags=${_comp_tags-}"
    print -r -- "curcontext=${curcontext-}"
    print -r -- "compprefix_set=${+PREFIX} compsuffix_set=${+SUFFIX}"
  } >| $d/misc
  print -r -- OK >| $d/ok
}
"""


@dataclass
class Episode:
    """One completion episode inside a session."""
    surface: str
    buffer: str
    keys: list
    end: str            # "abort" | "accept"

    def label(self) -> str:
        return (f"{self.surface}:{self.buffer!r}"
                f"+{'+'.join(self.keys)}/{self.end}")


def gen_episode(rng, surfaces, presses) -> Episode:
    surface, buf, pre = gen_buffer(rng, surfaces)
    keys = pre + gen_keyseq(rng, presses)
    return Episode(surface, buf, keys, rng.choice(EPISODE_ENDS))


def episodes_encode(eps) -> list:
    return [{"surface": e.surface, "buffer": e.buffer, "keys": list(e.keys),
             "end": e.end} for e in eps]


def episodes_decode(raw) -> list:
    out = []
    for e in raw:
        end = e.get("end", "abort")
        if end not in EPISODE_ENDS:
            raise ValueError(f"unknown episode end: {end!r}")
        out.append(Episode(e.get("surface", "replay"), e["buffer"],
                           list(e["keys"]), end))
    return out


def read_probe(d) -> tuple:
    """(probe_dict, reason). `reason` is a NAMED failure when the shell did not
    produce a complete probe; the dict is None then.

    `ok` is written last, so its absence means the probe function did not run to
    completion — which is a state axis that was NOT asserted, and it is counted
    and printed as such rather than being read as agreement."""
    if not os.path.exists(os.path.join(d, "ok")):
        return None, "probe did not complete (no sentinel file)"
    out = {}
    for name in PROBE_FILES:
        p = os.path.join(d, name)
        if not os.path.exists(p):
            return None, f"probe file missing: {name}"
        with open(p, "r", errors="replace") as f:
            out[name] = [ln.rstrip("\n") for ln in f if ln.strip()]
    return out, None


def probe_delta(base, cur) -> dict:
    """{probe: (added, removed)} for ONE shell against ITS OWN baseline.

    Per shell, deliberately. The two shells do not start from the same table —
    zshrs carries parameters zsh does not — so comparing the raw sets across
    shells would report that pre-existing difference once per episode and bury
    the thing this mode exists to find. The baseline difference IS reported, in
    its own named category, exactly once (see `baseline_diffs`); what is
    compared per episode is how each shell moved from where IT started."""
    out = {}
    for name in PROBE_FILES:
        b, c = set(base.get(name, ())), set(cur.get(name, ()))
        out[name] = (tuple(sorted(c - b)), tuple(sorted(b - c)))
    return out


def baseline_diffs(base_ref, base_test) -> list:
    """[(probe, only_in_zsh, only_in_zshrs)] at the session baseline.

    Not a drift and not suppressed: a pre-existing table difference is a real
    finding, it is simply a DIFFERENT finding from "this completion added a
    name", and reporting it once by name is what makes the per-episode deltas
    readable."""
    out = []
    for name in PROBE_FILES:
        r, t = set(base_ref.get(name, ())), set(base_test.get(name, ()))
        if r != t:
            out.append((name, tuple(sorted(r - t)), tuple(sorted(t - r))))
    return out


@dataclass
class Drift:
    """One probe that moved differently on the two shells at one episode."""
    episode: int
    rep: int
    probe: str
    ref_added: tuple
    ref_removed: tuple
    test_added: tuple
    test_removed: tuple
    after: str            # the episode that ran immediately before this probe
    prev_matched: bool    # the same probe agreed at the PREVIOUS episode

    def sig(self):
        """The identity of this drift, with the EPISODE INDEX deliberately left
        out: shrinking removes episodes, which renumbers every later one, so an
        index in the signature would reject every real reduction — the same
        reasoning as `signature()` for the key path."""
        return (self.probe,
                tuple(sorted(set(self.test_added) ^ set(self.ref_added))),
                tuple(sorted(set(self.test_removed) ^ set(self.ref_removed))))

    def named(self) -> list:
        """The actionable form: which NAMES, on which side. A count-only report
        ('527 vs 528') is not something anyone can act on."""
        lines = []
        for what, r, t in (("added", self.ref_added, self.test_added),
                           ("removed", self.ref_removed, self.test_removed)):
            only_t = sorted(set(t) - set(r))
            only_r = sorted(set(r) - set(t))
            if only_t:
                lines.append(f"{what} by zshrs only: {', '.join(only_t)}")
            if only_r:
                lines.append(f"{what} by zsh only:   {', '.join(only_r)}")
        return lines


@dataclass
class SessionRun:
    """Everything one session produced. Nothing here decides a verdict; the
    verdict is computed from it in `run_session_cell`."""
    fail_step: int = 0
    records: list = field(default_factory=list)
    # (side, episode, rep) -> probe dict
    probes: dict = field(default_factory=dict)
    # (episode, rep) -> [(short_label, ref_grid, test_grid), ...]
    frames: dict = field(default_factory=dict)
    # episode index -> Episode, for attribution
    ran: dict = field(default_factory=dict)
    # NAMED reasons a probe could not be read. A state axis that was not
    # asserted is never reported as an axis that agreed.
    probe_failures: list = field(default_factory=list)
    episodes_done: int = 0


def _probe_dir(root, side, ei, rep) -> str:
    d = os.path.join(root, side, f"e{ei}r{rep}")
    os.makedirs(d, exist_ok=True)
    return d


def run_session(init_file, episodes, args, env, geom, repeat, probe_root,
                deaths_out=None) -> SessionRun:
    """Drive ONE pair of shells through `episodes`, `repeat` times each.

    Parity is asserted after the buffer, after every completion key and after
    the terminator of EVERY repetition of EVERY episode — the run stops at the
    first divergence, because past it the two shells are in different states and
    every later assertion would be reporting the same bug again.

    Between episodes both shells write their own state (see PROBE_FUNCTION) and
    the files are kept for `run_session_cell` to diff. The screens are cleared
    with the shell's own Ctrl-L before each episode and before each probe, so
    the probe's command line can be read back from row 0 and a leftover buffer
    is NAMED rather than silently prepended to the probe command."""
    source_cmd = f"source {shlex.quote(init_file)}\n".encode()
    env = dict(env, COLUMNS=str(geom.cols), LINES=str(geom.rows))
    # Same NAME on both shells, different VALUE — see PROBE_FUNCTION. The name
    # set is what the parameter probe compares, so this adds one name to both
    # sides and can never itself read as a drift.
    ref_env = dict(env, _CP_PROBE_ROOT=os.path.join(probe_root, "zsh"))
    test_env = dict(env, _CP_PROBE_ROOT=os.path.join(probe_root, "zshrs"))
    ref = ShellSession([args.zsh, "-f", "-i"], ref_env, geom.rows, geom.cols,
                       "zsh", args.settle)
    test = ShellSession([args.zshrs, "--zsh", "-f", "-i"], test_env, geom.rows,
                        geom.cols, "zshrs", args.settle)
    run = SessionRun()
    step = [0]

    def drain_both(max_wait, first_wait):
        for s in (ref, test):
            s.drain_settled(max_wait=max_wait, first_wait=first_wait)

    def compare(label, bucket=None):
        step[0] += 1
        rg = normalize_rows(ref.grid())
        tg = normalize_rows(test.grid())
        d = diff_grids(rg, tg)
        run.records.append((step[0], label, rg, tg, d))
        if bucket is not None:
            bucket.append((label, rg, tg))
        return d

    def died():
        d = death_report((ref, test))
        if not d:
            return False
        if deaths_out is not None:
            deaths_out.extend(d)
        return True

    def probe(ei, rep):
        """Both shells write their state; both files are read back here."""
        for side, s in (("zsh", ref), ("zshrs", test)):
            s.fresh_prompt()
            line = line_after_prompt(normalize_rows(s.grid()), geom.cols)
            if line:
                # The episode terminator did not leave an empty line on this
                # shell, so the probe command would be appended to whatever is
                # there. NAMED, never silently attempted.
                run.probe_failures.append(
                    f"e{ei}r{rep} {side}: line not empty before probe "
                    f"({line!r}) — state not asserted here")
                continue
            d = _probe_dir(probe_root, side, ei, rep)
            # The TAG, never the path: the command line is accepted into
            # history and a later `up` recalls it onto the compared screen.
            s.send(f"_cp_probe e{ei}r{rep}\r".encode())
            s.drain_settled(max_wait=15.0, first_wait=8.0)
            got, why = read_probe(d)
            if got is None:
                run.probe_failures.append(f"e{ei}r{rep} {side}: {why}")
            else:
                run.probes[(side, ei, rep)] = got

    try:
        for s in (ref, test):
            s.drain_settled(max_wait=3.0, first_wait=2.0)
            s.send(source_cmd)
            if not s.wait_for_prompt(timeout=25.0):
                died()
                run.fail_step = 1
                run.records.append((1, "(init)", None, None, None))
                return run
        # Episode 0: the baseline. Taken by RUNNING the probe, not by reading
        # the tables directly, so whatever the probe command itself perturbs is
        # already perturbed in the baseline and cannot read as drift later.
        probe(0, 1)
        if died():
            return run
        for ei, ep in enumerate(episodes, 1):
            run.ran[ei] = ep
            for rep in range(1, max(1, repeat) + 1):
                bucket = []
                run.frames[(ei, rep)] = bucket
                for s in (ref, test):
                    s.fresh_prompt()
                if ep.buffer:
                    for chunk in ref.buffer_lines(ep.buffer):
                        for s in (ref, test):
                            s.send(chunk)
                        drain_both(max_wait=2.0, first_wait=1.0)
                if compare(f"e{ei}r{rep} (buffer)", bucket):
                    run.fail_step = step[0]
                    return run
                if died():
                    run.fail_step = step[0]
                    return run
                for kn, key in enumerate(ep.keys, 1):
                    for s in (ref, test):
                        s.send_key(key)
                    # The first key of the FIRST episode is cold (the autoload
                    # chain has never run in this process); every later one is
                    # warm, including the first key of episode 2 — which is
                    # exactly the difference this mode exists to exercise.
                    fw = 8.0 if (kn == 1 and ei == 1 and rep == 1) else 4.0
                    drain_both(max_wait=12.0, first_wait=fw)
                    if compare(f"e{ei}r{rep} {key}", bucket):
                        run.fail_step = step[0]
                        return run
                    if died():
                        run.fail_step = step[0]
                        return run
                for tok in END_TOKENS[ep.end]:
                    for s in (ref, test):
                        s.send_edit_token(tok)
                    drain_both(max_wait=8.0, first_wait=1.5)
                if compare(f"e{ei}r{rep} end:{ep.end}", bucket):
                    run.fail_step = step[0]
                    return run
                if died():
                    run.fail_step = step[0]
                    return run
                probe(ei, rep)
                if died():
                    run.fail_step = step[0]
                    return run
                run.episodes_done += 1
        return run
    finally:
        ref.close()
        test.close()


def probe_order(run) -> list:
    """The (episode, rep) keys in the order they were taken, baseline first."""
    seen = sorted({(ei, rep) for (_, ei, rep) in run.probes})
    return seen


def analyse_drift(run) -> tuple:
    """(drifts, shared, baseline, unpaired).

    `drifts`   the actionable findings: a probe that moved DIFFERENTLY on the
               two shells, with the names on each side.
    `shared`   Counter of probes that moved IDENTICALLY on both shells. That is
               zsh's own behaviour reproduced faithfully, so it is not a zshrs
               bug — but it is state accumulating inside a live session and it
               is counted and printed rather than dropped.
    `baseline` the cross-shell table difference that was already there before a
               single completion ran, named once.
    `unpaired` (episode, rep) points where only one side produced a probe, so
               no comparison could be made there. Named, never treated as
               agreement."""
    base_r = run.probes.get(("zsh", 0, 1))
    base_t = run.probes.get(("zshrs", 0, 1))
    if base_r is None or base_t is None:
        return [], Counter(), [], ["baseline probe missing on "
                                   + ("zsh" if base_r is None else "zshrs")]
    base = baseline_diffs(base_r, base_t)
    drifts, shared, unpaired = [], Counter(), []
    prev_ok = {name: True for name in PROBE_FILES}
    for ei, rep in probe_order(run):
        if ei == 0:
            continue
        cur_r = run.probes.get(("zsh", ei, rep))
        cur_t = run.probes.get(("zshrs", ei, rep))
        if cur_r is None or cur_t is None:
            unpaired.append(f"e{ei}r{rep}: only "
                            + ("zshrs" if cur_r is None else "zsh")
                            + " produced a probe")
            continue
        dr = probe_delta(base_r, cur_r)
        dt = probe_delta(base_t, cur_t)
        ep = run.ran.get(ei)
        for name in PROBE_FILES:
            ra, rr = dr[name]
            ta, tr = dt[name]
            if (ra, rr) == (ta, tr):
                if ra or rr:
                    shared[name] += 1
                prev_ok[name] = True
                continue
            drifts.append(Drift(
                episode=ei, rep=rep, probe=name,
                ref_added=ra, ref_removed=rr, test_added=ta, test_removed=tr,
                after=(ep.label() if ep else "?"),
                prev_matched=prev_ok[name]))
            prev_ok[name] = False
    return drifts, shared, base, unpaired


@dataclass
class IdemPoint:
    """One place a shell did not render the same episode the same way twice."""
    episode: int
    rep: int
    step: int
    label: str
    first_diff: tuple

    def where(self):
        """The identity of the point, so the two shells' points can be compared
        as sets. Deliberately NOT the first-diff cell: two shells that both
        redraw the same step differently are both non-idempotent AT THAT STEP,
        which is the thing being compared."""
        return (self.episode, self.rep, self.step)

    def __str__(self):
        return (f"e{self.episode}: rep{self.rep} renders {self.label!r} "
                f"differently from rep1 (step {self.step}), first diff "
                f"row {self.first_diff[0]} col {self.first_diff[1]}")


def analyse_idempotence(run) -> dict:
    """{side: [IdemPoint, ...]} — where a shell did NOT render one episode the
    same way twice IN ITS OWN OUTPUT.

    Judged per shell BEFORE the two are compared with each other, because it is
    a different bug: a shell that is not self-consistent across repetitions is
    broken on its own terms, and folding that into a zsh-vs-zshrs difference
    would name the wrong thing.

    Non-idempotence is NOT automatically a defect. A second `echo $<TAB>` in the
    same process legitimately lists more parameters than the first, because the
    first completion loaded some — zsh does exactly that, and a zshrs that did
    it too has reproduced the reference faithfully. So the caller compares the
    two shells' point SETS: a point where only zshrs is non-idempotent is the
    finding; a point where both are is zsh behaviour and belongs in the counted
    observations, not in a verdict."""
    out = {"zsh": [], "zshrs": []}
    reps = sorted({rep for (_, rep) in run.frames})
    if len(reps) < 2:
        return out
    for ei in sorted({e for (e, _) in run.frames}):
        first = run.frames.get((ei, reps[0]))
        if not first:
            continue
        for rep in reps[1:]:
            other = run.frames.get((ei, rep))
            if not other or len(other) != len(first):
                continue
            for idx, ((la, ra, ta), (lb, rb, tb)) in enumerate(
                    zip(first, other), 1):
                if ra != rb:
                    out["zsh"].append(IdemPoint(
                        ei, rep, idx, lb, first_diff_cell(diff_grids(ra, rb))))
                if ta != tb:
                    out["zshrs"].append(IdemPoint(
                        ei, rep, idx, lb, first_diff_cell(diff_grids(ta, tb))))
                if ra != rb or ta != tb:
                    break
    return out


def idem_split(idem) -> tuple:
    """(zshrs_only, shared, zsh_only) — the three groups a caller must keep
    apart. Only `zshrs_only` is a defect in this project; `shared` is zsh's own
    non-idempotence reproduced, and `zsh_only` is a finding about the REFERENCE
    shell, which is worth printing and is not a zshrs verdict."""
    r = {p.where(): p for p in idem.get("zsh", [])}
    t = {p.where(): p for p in idem.get("zshrs", [])}
    zshrs_only = [t[k] for k in sorted(set(t) - set(r))]
    shared = [t[k] for k in sorted(set(t) & set(r))]
    zsh_only = [r[k] for k in sorted(set(r) - set(t))]
    return zshrs_only, shared, zsh_only


def shrink_episodes(cell, args, env, target_sig, budget, run_once):
    """Reduce the EPISODE SEQUENCE to a subsequence that still drifts at
    `target_sig`.

    "the parameter table diverges after 8 episodes" is not a bug report; "the
    parameter table diverges after these two episodes, and these are the names"
    is. Same ddmin, same probe budget accounting and the same 'reduced, not
    minimal' honesty as the other three axes — a probe here is more expensive
    than any of them (it boots two shells and runs the whole subsequence), which
    is why it has its own smaller default budget."""
    if budget <= 0 or len(cell.episodes) <= 1:
        return list(cell.episodes), 0
    probes = [0]

    def still_drifts(candidate):
        probes[0] += 1
        r = run_once(list(candidate))
        if r is None or r.fail_step:
            return False
        d, _, _, _ = analyse_drift(r)
        return bool(d) and d[0].sig() == target_sig

    return (ddmin(list(cell.episodes), still_drifts, max_probes=budget),
            probes[0])


def session_replay_command(args, cell, episodes) -> str:
    """A copy-pasteable command that re-runs one session. The whole episode
    sequence travels as one JSON argument: a session finding is a claim about an
    ORDER of completions, and no single `--case` line can carry that."""
    spec = {
        "geom": [cell.geom.rows, cell.geom.cols],
        "zstyle": cell.zstyle_path,
        "repeat": cell.repeat,
        "episodes": episodes_encode(episodes),
    }
    return ("scripts/compsys_parity.py --session-replay "
            + shlex.quote(json.dumps(spec, separators=(",", ":"))) + " -v")


def run_session_cell(cell, args, env, dump, fpath_dirs, outdir) -> CellResult:
    """One session cell: N episodes in ONE pair of shells, parity after every
    step, state probes between episodes, an idempotence check across
    repetitions, and ddmin over the episode sequence for a drift.

    Verdict order, most-specific first:
      DIED   a shell exited mid-session (its own verdict, as everywhere else);
      FAIL   a per-step screen divergence — the ordinary comparison, reported
             exactly as the single-shot modes report it;
      FAIL   a STATE DRIFT: a probe that moved differently on the two shells;
      FAIL   zshrs did not render an episode the same way twice;
      PASS   every step agreed, every probe moved the same way on both shells.
    A probe that could not be read is NEVER agreement: it is named, counted, and
    the cell says so in its detail.

    `--latency` does not time a session and does not pretend to: a session's
    keystrokes are deliberately not independent (the whole point is that the
    process is warm and carrying state), so a best-of-K per key would be
    measuring a different thing from every other cell in the book. Session cells
    therefore reach `LatencyBook.not_measured` and are listed there as carrying
    no timing, rather than contributing a number that is not comparable."""
    res = CellResult(cell=cell)
    root = tempfile.mkdtemp(prefix="cp_probe_", dir=cell.workdir)

    def run_once(episodes, deaths_out=None, tag="run"):
        r = tempfile.mkdtemp(prefix=f"cp_{tag}_", dir=cell.workdir)
        return run_session(cell.init_file, episodes, args, env, cell.geom,
                           cell.repeat, r, deaths_out=deaths_out)

    deaths: list = []
    run = run_session(cell.init_file, cell.episodes, args, env, cell.geom,
                      cell.repeat, root, deaths_out=deaths)
    res.session = run
    res.replay = session_replay_command(args, cell, cell.episodes)
    if deaths:
        res.status = "DIED"
        res.deaths = deaths
        again = 0
        for _ in range(max(0, args.confirm)):
            d2: list = []
            run_once(cell.episodes, deaths_out=d2, tag="confirm")
            if d2:
                again += 1
        res.detail = (death_str(deaths)
                      + (f"; reproduced in {again}/{args.confirm} re-runs"
                         if args.confirm else ""))
        if run.records:
            step, key, rg, tg, diffs = run.records[-1]
            res.fail_step, res.fail_key = step, key
            res.ref_grid, res.test_grid = rg or [], tg or []
            res.diffs = diffs or []
        return res

    # 1. The ordinary per-step screen comparison, first: past a divergence the
    #    two shells are in different states and no probe taken after it means
    #    anything.
    if run.fail_step:
        step, key, rg, tg, diffs = run.records[-1]
        res.fail_step, res.fail_key = step, key
        res.ref_grid, res.test_grid = rg or [], tg or []
        if diffs is None:
            res.status = "FAIL"
            res.detail = "a shell never reached prompt"
            return res
        res.diffs = diffs
        res.sig = signature(run.records)
        reproduced = True
        for _ in range(max(0, args.confirm)):
            r2 = run_once(cell.episodes, tag="confirm")
            if not r2.fail_step or signature(r2.records) != res.sig:
                reproduced = False
                break
        res.status = "FAIL" if reproduced else "FLAKY"
        res.detail = (f"{len(diffs)} rows differ at {key!r} "
                      f"(episode {run.episodes_done + 1} of "
                      f"{len(cell.episodes)})")
        if args.shrink_probes > 0 and res.status == "FAIL":
            res.shrink_notes.append(
                "episode sequence not reduced: this is a per-step SCREEN "
                "divergence, which the single-shot modes already minimise on "
                "the key path — the sequence is only reduced for a STATE DRIFT")
        return res

    drifts, shared, base, unpaired = analyse_drift(run)
    idem = analyse_idempotence(run)
    res.drifts = drifts
    res.shared_drift = shared
    res.baseline_diffs = base
    res.probe_failures = list(run.probe_failures) + list(unpaired)
    res.idempotence = idem

    notes = []
    if res.probe_failures:
        notes.append(f"{len(res.probe_failures)} probe(s) unreadable — the "
                     f"state axis was NOT asserted there")
    if any(r.startswith("baseline probe missing") for r in unpaired):
        # Without a baseline there is nothing to measure drift AGAINST, so the
        # state axis — the entire reason this mode exists — was never asserted.
        # The per-step screen comparison did run and did agree, and that is said
        # here; but a session whose state was never probed is not a session that
        # passed, and calling it one would be the exact weakening this file
        # refuses. SKIP is counted, named and printed, like every other
        # comparison that could not be made.
        res.status = "SKIP"
        res.detail = ("state axis NOT established: " + "; ".join(unpaired)
                      + f" — the per-step screen comparison DID run and agreed "
                        f"across {len(run.records)} assertions, but drift "
                        f"cannot be measured without a baseline")
        return res
    if drifts:
        res.status = "FAIL"
        d0 = drifts[0]
        res.sig = d0.sig()
        res.fail_step = d0.episode
        res.fail_key = f"probe:{d0.probe}"
        res.detail = (f"state drift in ${d0.probe} at episode {d0.episode}"
                      + (" (agreed at the previous episode)"
                         if d0.prev_matched else " (already drifting before)")
                      + (f"; {len(drifts)} drift point(s) total"
                         if len(drifts) > 1 else ""))
        reproduced = True
        for _ in range(max(0, args.confirm)):
            r2 = run_once(cell.episodes, tag="confirm")
            if r2.fail_step:
                reproduced = False
                break
            d2, _, _, _ = analyse_drift(r2)
            if not d2 or d2[0].sig() != res.sig:
                reproduced = False
                break
        res.status = "FAIL" if reproduced else "FLAKY"
        if args.session_shrink_probes > 0 and res.status == "FAIL":
            full = len(cell.episodes)
            min_eps, p = shrink_episodes(cell, args, env, res.sig,
                                         args.session_shrink_probes,
                                         lambda eps: run_once(eps, tag="shrink"))
            res.probes += p
            res.shrink_exhausted = p >= args.session_shrink_probes
            res.min_episodes = min_eps
            res.shrink_notes.append(
                f"episode sequence reduced {len(min_eps)}/{full} "
                f"[{p} probes, each one a whole session]")
            res.replay = session_replay_command(args, cell, min_eps)
    elif idem_split(idem)[0]:
        # ONLY the points where zshrs is non-idempotent and zsh is NOT. A point
        # where both shells redraw the same step differently is zsh's own
        # behaviour (a second completion in one process legitimately sees more
        # loaded state than the first) reproduced faithfully, and calling that a
        # zshrs failure would be a false positive in an audit instrument.
        only = idem_split(idem)[0]
        res.status = "FAIL"
        res.fail_step = only[0].episode
        res.fail_key = "idempotence"
        res.detail = (f"zshrs is NOT self-consistent across repetitions of the "
                      f"same episode at {len(only)} point(s) where zsh IS — a "
                      f"different bug from a zsh-vs-zshrs difference, found "
                      f"without comparing the two shells to each other")
    else:
        res.status = "PASS"
        res.detail = (f"{len(cell.episodes)} episodes x{cell.repeat} = "
                      f"{run.episodes_done} episode runs, "
                      f"{len(run.records)} parity assertions, "
                      f"{len(run.probes)} state probes")
    if notes:
        res.detail = (res.detail + "; " if res.detail else "") + "; ".join(notes)
    return res


def build_cells(args, dump, fpath_dirs, statements, surfaces, outdir, skips,
                edit_pairs=None, tree=None, fs_surfaces=None,
                session_surfaces=None):
    """Materialise every fuzz cell up front (zstyle subset, buffer, key path,
    geometry, init file) so the work list is fixed and reproducible from the
    seed alone before any shell is booted."""
    fixed = [c.strip() for c in args.combo_commands.split(",") if c.strip()]
    per_combo = args.buffer_cases if args.buffer_cases > 0 else len(fixed)
    cells = []
    for n in range(args.random_combos):
        rng = random.Random(f"{args.seed}:{n}")
        subset = [s for s in statements if rng.random() < args.combo_keep]
        workdir = tempfile.mkdtemp(prefix=f"combo_{args.seed}_{n}_")
        combo_path = os.path.join(workdir, "zstyle.zsh")
        with open(combo_path, "w") as f:
            f.write(f"# random combo seed={args.seed} index={n}: "
                    f"{len(subset)}/{len(statements)} statements\n")
            f.write("\n".join(subset) + "\n")
        with open(saved_path(outdir, args.seed, n), "w") as f:
            f.write(f"# random combo seed={args.seed} index={n}: "
                    f"{len(subset)}/{len(statements)} statements\n")
            f.write("\n".join(subset) + "\n")
        # One init per (editing mode, cwd), built once per combo and shared by
        # every cell of that combo (booting a shell is the expensive part;
        # writing an init file is not, but a fresh one per cell would multiply
        # the temp-dir churn for no benefit). `(None, None)` is the plain init
        # every pre-existing cell used.
        mode_inits = {}

        def init_for(mode=None, cwd=None, probe=False):
            key = (mode, cwd, probe)
            if key not in mode_inits:
                mode_inits[key] = build_init_file(dump, fpath_dirs, combo_path,
                                                  mode, cwd, probe)
            return mode_inits[key]

        init_file = init_for()

        # `fuzz_buffers` is `--buffer-fuzz` OR `--multiline-fuzz`: both draw the
        # buffer from a surface pool, they just contribute different surfaces to
        # it. With neither, the fixed --combo-commands list is used exactly as
        # before.
        fuzzing = getattr(args, "fuzz_buffers", args.buffer_fuzz)
        count = per_combo if fuzzing else len(fixed)
        for ci in range(count):
            crng = random.Random(f"{args.seed}:{n}:{ci}")
            if fuzzing:
                surface, buf, pre = gen_buffer(crng, surfaces)
            else:
                surface, buf, pre = "fixed", fixed[ci], []
            buffer = buf if buf.endswith(" ") or fuzzing else buf + " "
            geom = pick_geom(crng, args)
            # A buffer that cannot even be TYPED inside the window (prompt +
            # text wider than the whole screen) is not a comparison the harness
            # can make; it is SKIPPED with a counted reason rather than compared
            # against a truncated grid.
            if len(PROMPT_SENTINEL) + 1 + len(buffer) >= geom.rows * geom.cols:
                skips[f"buffer-exceeds-screen:{geom_str(geom)}"] += 1
                continue
            # A MULTILINE buffer needs one row per physical line (plus its
            # wraps) before completion has drawn anything. If the buffer alone
            # fills the window there is no comparison to make, only a scrolled
            # grid — same treatment, counted under its own reason.
            if "\n" in buffer:
                need = sum(1 + (len(PROMPT_SENTINEL) + 1 + len(ln)) // geom.cols
                           for ln in buffer.split("\n"))
                if need + 1 > geom.rows:
                    skips[f"multiline-exceeds-rows:{geom_str(geom)}"] += 1
                    continue
            keys = pre + gen_keyseq(
                random.Random(f"{args.seed}:{n}:{ci}:keys"), args.presses)
            cells.append(Cell(idx=n, uid=f"{n}_{ci}", surface=surface,
                              buffer=buffer, keys=keys,
                              geom=geom, statements=subset,
                              zstyle_path=saved_path(outdir, args.seed, n),
                              init_file=init_file, workdir=workdir))
        # ── fstree cells (`--fstree-fuzz`) ──────────────────────────────────
        #
        # Their own cells rather than another entry in the shared surface pool,
        # because they are the only ones that need BOTH shells `cd`'d into the
        # hermetic tree — they carry `cwd` (which puts a `cd` line in their init)
        # and `fstree_seed` (which puts `--fstree-seed` in their replay, so the
        # replay rebuilds the identical tree before typing the buffer).
        if fs_surfaces and tree is not None:
            fs_init = init_for(None, tree.root)
            for fi in range(args.fstree_cases):
                frng = random.Random(f"{args.seed}:{n}:f{fi}")
                surface, buf, pre = gen_buffer(frng, fs_surfaces)
                geom = pick_geom(frng, args)
                if len(PROMPT_SENTINEL) + 1 + len(buf) >= geom.rows * geom.cols:
                    skips[f"fstree-buffer-exceeds-screen:{geom_str(geom)}"] += 1
                    continue
                keys = pre + gen_keyseq(
                    random.Random(f"{args.seed}:{n}:f{fi}:keys"), args.presses)
                cells.append(Cell(
                    idx=n, uid=f"{n}_f{fi}", surface=surface, buffer=buf,
                    keys=keys, geom=geom, statements=subset,
                    zstyle_path=saved_path(outdir, args.seed, n),
                    init_file=fs_init, workdir=workdir,
                    cwd=tree.root, fstree_seed=tree.seed))
        # ── interrupt cells (`--interrupt-fuzz`) ────────────────────────────
        #
        # Also their own cells: an interrupt has to be a property of a cell that
        # the replay can carry (`--interrupt <spec>`), and folding it into the
        # existing cells would silently change what every pre-existing surface
        # measures. The buffer is drawn from whatever surface pools this run
        # has — including the fstree pool, so an interrupt can land on a
        # completion inside the adversarial tree.
        if args.interrupt_fuzz:
            i_pool = list(surfaces) + list(fs_surfaces or [])
            for ii in range(args.interrupt_cases):
                irng = random.Random(f"{args.seed}:{n}:i{ii}")
                if i_pool:
                    surface, buf, pre = gen_buffer(irng, i_pool)
                    in_tree = surface.startswith("fs_")
                elif fixed:
                    b = irng.choice(fixed)
                    surface, buf, pre = "fixed", (b if b.endswith(" ")
                                                  else b + " "), []
                    in_tree = False
                else:
                    skips["interrupt-no-buffer-source"] += 1
                    continue
                geom = pick_geom(irng, args)
                if len(PROMPT_SENTINEL) + 1 + len(buf) >= geom.rows * geom.cols:
                    skips[f"interrupt-buffer-exceeds-screen:{geom_str(geom)}"] += 1
                    continue
                keys = pre + gen_keyseq(
                    random.Random(f"{args.seed}:{n}:i{ii}:keys"), args.presses)
                iv = gen_interrupt(irng, args.interrupt_kinds_list, len(keys))
                cells.append(Cell(
                    idx=n, uid=f"{n}_i{ii}",
                    surface=f"intr/{iv.kind}@{iv.at}/{surface}",
                    buffer=buf, keys=keys, geom=geom, statements=subset,
                    zstyle_path=saved_path(outdir, args.seed, n),
                    init_file=(init_for(None, tree.root) if in_tree and tree
                               else init_file),
                    workdir=workdir,
                    cwd=(tree.root if in_tree and tree else None),
                    fstree_seed=(tree.seed if in_tree and tree else None),
                    interrupt=iv))
        # ── session cells (`--session-fuzz N`) ───────────────────────────────
        #
        # Their own cells, and the only ones whose init carries `_cp_probe`: a
        # session is N episodes inside ONE pair of shells, so it cannot be
        # folded into a per-cell surface without changing what every other cell
        # measures. The episodes are drawn from whatever surface pool this run
        # has, so a session composes with --buffer-fuzz / --multiline-fuzz /
        # --geometry-fuzz exactly like every other mode.
        if args.session_fuzz > 0 and session_surfaces:
            sess_init = init_for(None, None, True)
            for si in range(args.session_cases):
                srng = random.Random(f"{args.seed}:{n}:s{si}")
                geom = pick_geom(srng, args)
                eps = []
                for k in range(args.session_fuzz):
                    ep = gen_episode(srng, session_surfaces, args.presses)
                    if len(PROMPT_SENTINEL) + 1 + len(ep.buffer) \
                            >= geom.rows * geom.cols:
                        skips[f"session-buffer-exceeds-screen:"
                              f"{geom_str(geom)}"] += 1
                        continue
                    if "\n" in ep.buffer:
                        need = sum(
                            1 + (len(PROMPT_SENTINEL) + 1 + len(ln)) // geom.cols
                            for ln in ep.buffer.split("\n"))
                        if need + 1 > geom.rows:
                            skips[f"session-multiline-exceeds-rows:"
                                  f"{geom_str(geom)}"] += 1
                            continue
                    eps.append(ep)
                if not eps:
                    skips["session-no-usable-episodes"] += 1
                    continue
                cells.append(SessionCell(
                    idx=n, uid=f"{n}_s{si}",
                    surface=f"session/{len(eps)}ep_x{args.session_repeat}",
                    buffer="", keys=[], geom=geom, statements=subset,
                    zstyle_path=saved_path(outdir, args.seed, n),
                    init_file=sess_init, workdir=workdir,
                    episodes=eps, repeat=args.session_repeat))
        if not args.edit_fuzz:
            continue
        # ── edit-fuzz cells ─────────────────────────────────────────────────
        for ei in range(args.edit_cases):
            erng = random.Random(f"{args.seed}:{n}:e{ei}")
            prog = gen_edit_program(erng, edit_pairs)
            geom = pick_geom(erng, args)
            longest = max([len(prog.buffer)]
                          + [len(_unquote_payload(t.partition(':')[2]))
                             for t in prog.tokens if t[0] in "tpb"])
            if len(PROMPT_SENTINEL) + 1 + len(prog.buffer) + longest \
                    >= geom.rows * geom.cols:
                skips[f"edit-buffer-exceeds-screen:{geom_str(geom)}"] += 1
                continue
            keys = gen_keyseq(random.Random(f"{args.seed}:{n}:e{ei}:keys"),
                              args.presses)
            cells.append(Cell(
                idx=n, uid=f"{n}_e{ei}", surface=f"edit/{prog.gen}",
                buffer=prog.buffer, keys=keys, geom=geom, statements=subset,
                zstyle_path=saved_path(outdir, args.seed, n),
                init_file=init_for(prog.mode), workdir=workdir,
                edit_tokens=prog.tokens, edit_mode=prog.mode,
                edit_gen=prog.gen, expect=prog.expect))
        # ── convergent pairs ────────────────────────────────────────────────
        for vi in range(args.convergent_cases):
            vrng = random.Random(f"{args.seed}:{n}:v{vi}")
            pair = gen_conv_pair(vrng, args.edit_modes_list)
            if pair is None:
                skips["no-convergent-pair-for-modes"] += 1
                continue
            geom = pick_geom(vrng, args)
            keys = gen_keyseq(random.Random(f"{args.seed}:{n}:v{vi}:keys"),
                              args.presses)
            cells.append(ConvCell(
                idx=n, uid=f"{n}_v{vi}", surface=f"conv/{pair.name}",
                buffer=pair.a.buffer, keys=keys, geom=geom, statements=subset,
                zstyle_path=saved_path(outdir, args.seed, n),
                init_file=init_for(pair.mode), workdir=workdir,
                edit_tokens=pair.a.tokens, edit_mode=pair.mode,
                edit_gen=pair.name, expect=pair.target,
                b_buffer=pair.b.buffer, b_edit_tokens=pair.b.tokens,
                b_gen=pair.b.gen, target=pair.target, conv_note=pair.note))
    return cells


def run_random_combos(args, dump, fpath_dirs, env):
    """Fuzz random subsets of the user's zstyles — and, with --buffer-fuzz /
    --geometry-fuzz, the command line and the terminal geometry too.

    Each cell is an independent pty PAIR, so cells are safe to run concurrently
    (--jobs N). Load slows a redraw, which can flip a marginal cell; that is
    exactly why --confirm re-runs stay on and why a non-reproducing divergence
    is reported as FLAKY in its own category instead of being counted either
    way."""
    statements = parse_zstyle_statements(args.zstyle)
    skips: Counter = Counter()
    # Surface POOL: `--buffer-fuzz` contributes the single-line surfaces,
    # `--multiline-fuzz` the continuation ones, and both together fuzz the
    # union — a multiline surface is an ordinary Surface, so everything
    # downstream (shrinking, replay, geometry) already handles it.
    pool = []
    if args.buffer_fuzz:
        pool += BUFFER_SURFACES
    if args.multiline_fuzz:
        pool += MULTILINE_SURFACES
    surfaces = available_surfaces(skips, pool) if pool else []
    if pool and not surfaces:
        sys.exit("compsys_parity: no usable buffer surfaces on this host")
    edit_pairs = (available_edit_surfaces(args.edit_modes_list)
                  if args.edit_fuzz else [])
    if args.edit_fuzz and not edit_pairs:
        sys.exit("compsys_parity: --edit-fuzz has no edit surfaces for modes "
                 + ",".join(args.edit_modes_list))
    outdir = os.path.join(tempfile.gettempdir(),
                          f"compsys_parity_failing_combos_{args.seed}")
    os.makedirs(outdir, exist_ok=True)

    # The hermetic filesystem tree. Built ONCE per run, verified against its own
    # plan, and shared by every fstree cell — one tree at one absolute path, so
    # both shells complete against literally the same directory.
    tree = None
    fs_surfaces = []
    if args.fstree_fuzz:
        tree = build_fstree(args.fstree_seed, args.fstree_big)
        fs_surfaces = fstree_surfaces(tree, skips)
        if not fs_surfaces:
            fstree_cleanup(tree.root)
            sys.exit("compsys_parity: --fstree-fuzz produced no usable "
                     "categories — the filesystem accepted none of the planned "
                     "entries (see the plan in fstree_plan)")

    # `--session-fuzz` needs buffers to complete, and it is a complete request on
    # its own: with no --buffer-fuzz/--multiline-fuzz the shared pool is empty,
    # so it falls back to the documented single-line surface set rather than
    # silently generating nothing.
    session_surfaces = []
    if args.session_fuzz > 0:
        session_surfaces = surfaces or available_surfaces(skips)
        if not session_surfaces:
            sys.exit("compsys_parity: --session-fuzz has no usable buffer "
                     "surfaces on this host")

    cells = build_cells(args, dump, fpath_dirs, statements, surfaces, outdir,
                        skips, edit_pairs, tree, fs_surfaces, session_surfaces)

    print(f"# random-combo fuzz: {args.random_combos} combos, {len(cells)} cells, "
          f"{args.presses}-key paths (parity asserted after EACH key)")
    print(f"# base zstyle: {args.zstyle} ({len(statements)} statements)")
    print(f"# seed={args.seed}  keep-prob={args.combo_keep}  confirm={args.confirm}  "
          f"jobs={max(1, args.jobs)}  shrink-probes={args.shrink_probes}")
    print(f"# buffer-fuzz={args.buffer_fuzz} multiline-fuzz={args.multiline_fuzz} "
          f"({len(surfaces)} surfaces)  "
          f"geometry-fuzz={args.geometry_fuzz}  "
          f"geom={'pool' if args.geometry_fuzz else geom_str(Geom(args.rows, args.cols))}")
    if args.multiline_fuzz:
        n_ml = sum(1 for c in cells if "\n" in c.buffer)
        print(f"# multiline-fuzz=True  {len(MULTILINE_SURFACES)} continuation "
              f"surfaces  {n_ml} cells complete inside a continuation "
              f"(every one is INCOMPLETE at each newline, so both shells "
              f"answer Return with PS2 and nothing is executed)")
    if args.latency:
        print(f"# latency=True  best-of-{args.latency_runs} per key  "
              f"min-delta={args.latency_min_ms:g}ms  "
              f"threshold={f'{args.latency_threshold:g}x' if args.latency_threshold else 'off (report only)'}"
              f"  — reported in its OWN category, never a parity verdict")
    if args.edit_fuzz:
        n_edit = sum(1 for c in cells if not isinstance(c, ConvCell)
                     and c.edit_tokens is not None)
        n_conv = sum(1 for c in cells if isinstance(c, ConvCell))
        print(f"# edit-fuzz=True  modes={','.join(args.edit_modes_list)}  "
              f"{len(edit_pairs)} (surface,mode) generators  "
              f"{n_edit} edit cells + {n_conv} convergent pairs "
              f"(parity asserted after EVERY edit token too)")
    if tree is not None:
        n_fs = sum(1 for c in cells if c.cwd == tree.root)
        for line in fstree_report(tree):
            print(line)
        print(f"#   {len(fs_surfaces)} usable surfaces, {n_fs} cells complete "
              f"INSIDE the tree (both shells cd'd to the same absolute path); "
              f"a folded/rejected entry disables its category and is counted, "
              f"never reported as a finding")
    if args.interrupt_fuzz:
        n_int = sum(1 for c in cells if c.interrupt is not None)
        print(f"# interrupt-fuzz=True  kinds={','.join(args.interrupt_kinds_list)}"
              f"  delay={args.interrupt_delay_ms}ms  {n_int} interrupted cells "
              f"(delivered identically to both shells; a shell that DIES is its "
              f"own verdict, never a pass and never a plain FAIL)")
    if args.session_fuzz > 0:
        n_sess = sum(1 for c in cells if isinstance(c, SessionCell))
        print(f"# session-fuzz={args.session_fuzz} episodes x"
              f"{args.session_repeat} repetitions, {n_sess} sessions "
              f"({len(session_surfaces)} surfaces) — ONE pair of shells per "
              f"session, parity asserted after EVERY step of EVERY episode, "
              f"state probed between episodes")
        print(f"#   probes: {', '.join(PROBE_FILES)}  "
              f"(set-compared: {', '.join(PROBE_SET_FILES)}; "
              f"line-compared: {', '.join(PROBE_LINE_FILES)})")
        print("#   the fuzzed buffer is NEVER executed: an episode ends with "
              "^G+^U (abort) or ^G+^U then a fixed literal `true` (accept)")
        print("#   the harness injects exactly one parameter, $_CP_PROBE_ROOT, "
              "with the SAME NAME on both shells and a per-shell value, and the "
              "probe command is `_cp_probe <tag>` on both — so a history recall "
              "(`up`) puts a byte-identical line back on the compared screen")
        print("#   deliberately NOT compared:")
        for why in PROBE_EXCLUDED:
            print(f"#     - {why}")
    print(f"# failing combos saved to: {outdir}")
    print()

    def work(cell):
        return run_cell(cell, args, env, dump, fpath_dirs, outdir)

    if args.jobs > 1:
        from concurrent.futures import ThreadPoolExecutor
        pool = ThreadPoolExecutor(max_workers=args.jobs)
        # `map` yields in submission order, so the log is deterministic no
        # matter which cell finishes first.
        stream = pool.map(work, cells)
    else:
        pool = None
        stream = (work(c) for c in cells)

    passed = failed = flaky = died = 0
    by_category: Counter = Counter()
    deaths_by_side: Counter = Counter()
    expect_bad: Counter = Counter()
    # Session-fuzz observations that are NOT parity verdicts and must not be
    # read as any: state both shells accumulate identically (that is zsh's own
    # behaviour), a shell that is not self-consistent across repetitions, and
    # probes that could not be taken at all. Counted and printed in their own
    # section so none of them is silently absorbed into a PASS.
    session_notes: Counter = Counter()
    results = []
    book = (LatencyBook(args.latency_min_ms, args.latency_threshold,
                        args.latency_runs, concurrent_drain=True)
            if args.latency else None)
    try:
        for res in stream:
            c = res.cell
            cat = (f"{c.edit_gen}[{c.edit_mode}]" if c.edit_gen
                   else f"{c.surface}[-]")
            by_category[(cat, res.status)] += 1
            if book is not None:
                if res.latency:
                    book.record(lat_cell_id(c), res.latency)
                else:
                    book.not_measured(lat_cell_id(c))
            if res.expect_ok is False:
                expect_bad[f"{c.edit_gen}: claimed {c.expect!r}, "
                           f"zsh showed {res.expect_saw!r}"] += 1
            if isinstance(c, SessionCell):
                # A session has no single buffer and no single key path, so it
                # gets its own reporting rather than being squeezed into a line
                # that would claim it was about one completion.
                for name, cnt in (res.shared_drift or Counter()).items():
                    session_notes[f"both-shells-drift:${name}"] += cnt
                session_notes["probe-not-asserted"] += len(res.probe_failures)
                idem = res.idempotence or {"zsh": [], "zshrs": []}
                only_t, shared_i, only_r = idem_split(idem)
                session_notes["not-idempotent:BOTH shells (zsh behaviour "
                              "reproduced)"] += len(shared_i)
                session_notes["not-idempotent:zsh only (reference shell "
                              "finding)"] += len(only_r)
                session_notes["not-idempotent:zshrs only"] += len(only_t)
                shead = (f"{res.status:5s} combo {c.idx:3d} [{c.surface:18s}] "
                         f"{geom_str(c.geom):>7s} {len(c.episodes)} episodes"
                         f" x{c.repeat}")
                print(f"{shead}  ({res.detail})")
                for i, ep in enumerate(c.episodes, 1):
                    print(f"      ep {i:2d}    : {ep.label()}")
                # `b_`-prefixed deliberately: `only_r`/`only_t` above are the
                # IDEMPOTENCE lists and are printed further down. Reusing those
                # names here silently replaced them with a baseline probe's name
                # lists whenever a baseline difference existed.
                for name, b_only_r, b_only_t in res.baseline_diffs:
                    print(f"      baseline  : ${name} already differs BEFORE "
                          f"any completion (own category, not a drift)")
                    if b_only_r:
                        print(f"                  only zsh  : "
                              f"{', '.join(b_only_r[:12])}")
                    if b_only_t:
                        print(f"                  only zshrs: "
                              f"{', '.join(b_only_t[:12])}")
                for name, cnt in sorted((res.shared_drift or Counter()).items()):
                    print(f"      state     : ${name} moved IDENTICALLY on both "
                          f"shells at {cnt} probe(s) — zsh's own accumulation, "
                          f"reproduced; not a finding")
                for reason in res.probe_failures:
                    print(f"      probe     : {reason}")
                for pt in only_t:
                    print(f"      IDEMPOTENCE: zshrs ONLY (zsh renders this "
                          f"step identically both times) — {pt}")
                for pt in shared_i:
                    print(f"      idempotence: BOTH shells — {pt}  (zsh's own "
                          f"non-idempotence, reproduced; not a finding)")
                for pt in only_r:
                    print(f"      idempotence: zsh (reference) ONLY — {pt}")
                if not idem["zsh"] and not idem["zshrs"] and c.repeat > 1:
                    print(f"      idempotence: both shells render every episode "
                          f"identically across all {c.repeat} repetitions")
                for d in res.drifts:
                    print(f"      DRIFT     : ${d.probe} at episode "
                          f"{d.episode} (rep {d.rep}) — "
                          + ("agreed at the previous episode"
                             if d.prev_matched else "already drifting before"))
                    print(f"                  after: {d.after}")
                    for ln in d.named():
                        print(f"                  {ln}")
                if res.status in ("FAIL", "FLAKY") and res.diffs:
                    print(f"      path      : diverges at step #{res.fail_step} "
                          f"(step {res.fail_key!r})")
                    for i, a, b in res.diffs[: (12 if args.verbose else 3)]:
                        print(f"        row {i:2d}: zsh  = {a!r}")
                        print(f"                 zshrs= {b!r}")
                    if args.verbose and res.ref_grid:
                        print("      --- zsh (ref) ---")
                        print(render_grid(res.ref_grid))
                        print("      --- zshrs (test) ---")
                        print(render_grid(res.test_grid))
                for note in res.shrink_notes:
                    print(f"      note      : {note}")
                if res.min_episodes is not None:
                    label = ("reduced (budget exhausted, not minimal)"
                             if res.shrink_exhausted
                             else "reduced (ddmin converged)")
                    print(f"      {label}")
                    for i, ep in enumerate(res.min_episodes, 1):
                        print(f"        ep {i:2d}: {ep.label()}")
                if res.replay:
                    print(f"      replay    : {res.replay}")
                if res.status == "PASS":
                    passed += 1
                elif res.status == "SKIP":
                    # The state axis could not be established. Neither a pass
                    # nor a failure — a counted, named, printed skip, exactly as
                    # a non-convergent pair is.
                    skips["session-state-axis-not-established"] += 1
                elif res.status == "DIED":
                    died += 1
                    for side, kind, value in res.deaths:
                        name = value
                        if kind == "signal":
                            try:
                                name = signal.Signals(value).name
                            except ValueError:      # pragma: no cover
                                name = f"signal {value}"
                        deaths_by_side[f"{side} {kind} {name}"] += 1
                elif res.status == "FLAKY":
                    flaky += 1
                else:
                    failed += 1
                results.append(_cell_json(res))
                continue
            head = (f"{res.status:5s} combo {c.idx:3d} [{c.surface:18s}] "
                    f"{geom_str(c.geom):>7s} {c.buffer!r}")
            if c.edit_tokens:
                head += f" +{edit_program_str(c.edit_tokens)} ({c.edit_mode})"
            if c.interrupt is not None:
                head += f" !{c.interrupt.label()}"
            if c.fstree_seed is not None:
                # The tree seed travels on EVERY fstree line, pass or fail: it
                # is the only thing that rebuilds the tree the buffer refers to.
                head += f" [fstree-seed {c.fstree_seed}]"
            if book is not None and res.latency:
                # Inline, clearly namespaced `lat`, and RAW: the outlier
                # judgement needs the whole run's distribution and is made in
                # the latency section at the end, never here.
                w = lat_worst(res.latency, args.latency_min_ms)
                head += (f"  lat {w[0]:.2f}x (key #{w[1]} {w[2]!r} "
                         f"{w[3]:.0f}->{w[4]:.0f}ms)" if w
                         else f"  lat <{args.latency_min_ms:g}ms")
            if res.status == "PASS":
                passed += 1
                print(f"{head}  keys={'+'.join(c.keys)}")
                results.append(_cell_json(res))
                continue
            if res.status == "DIED":
                # Its OWN verdict. Not a pass (the comparison never completed),
                # not a plain FAIL (the two shells did not disagree about a
                # screen — one of them stopped existing), and emphatically not a
                # timeout: round 3 of this tooling established that calling a
                # crashed reference shell a TIMEOUT hid a real upstream zsh
                # segfault for two rounds.
                died += 1
                for side, kind, value in res.deaths:
                    name = value
                    if kind == "signal":
                        try:
                            name = signal.Signals(value).name
                        except ValueError:            # pragma: no cover
                            name = f"signal {value}"
                    deaths_by_side[f"{side} {kind} {name}"] += 1
                print(f"{head}  (DIED: {res.detail})")
                print(f"      path      : {'+'.join(c.keys)}"
                      f"  → died at step #{res.fail_step} "
                      f"(step {res.fail_key!r})")
                if c.interrupt is not None:
                    print(f"      interrupt : {c.interrupt.label()}")
                print("      note      : not shrunk — a cell with no screens to "
                      "diff has no first-diff-cell invariant for ddmin")
                print(f"      replay    : {res.replay}")
                results.append(_cell_json(res))
                continue
            if res.status == "SKIP":
                # A convergent pair whose two programs did not converge in the
                # REFERENCE shell. Nothing about zshrs was established, so it
                # is neither a pass nor a failure — it is a counted, named,
                # printed skip.
                skips[f"non-convergent-in-reference:{c.edit_gen}"] += 1
                print(f"{head}  ({res.detail})")
                print(f"      pair      : A={c.buffer!r}+"
                      f"{edit_program_str(c.edit_tokens)}   "
                      f"B={c.b_buffer!r}+{edit_program_str(c.b_edit_tokens)}")
                for i, a, b in res.diffs[:4]:
                    print(f"        row {i:2d}: zsh A = {a!r}")
                    print(f"                 zsh B = {b!r}")
                print(f"      replay    : {res.replay}")
                results.append(_cell_json(res))
                continue
            if res.status == "FLAKY":
                flaky += 1
            else:
                failed += 1
            print(f"{head}  ({res.detail})")
            if isinstance(c, ConvCell):
                print(f"      pair      : A={c.buffer!r}+"
                      f"{edit_program_str(c.edit_tokens)}   "
                      f"B={c.b_buffer!r}+{edit_program_str(c.b_edit_tokens)}")
                print(f"      target    : {c.target!r}   ({c.conv_note})")
            elif c.edit_tokens is not None:
                print(f"      edits     : {edit_program_str(c.edit_tokens)}"
                      f"   mode={c.edit_mode}  gen={c.edit_gen}")
            if c.interrupt is not None:
                print(f"      interrupt : {c.interrupt.label()}"
                      f"   (delay {args.interrupt_delay_ms}ms)")
            if c.fstree_seed is not None:
                print(f"      fstree    : seed={c.fstree_seed} "
                      f"big={args.fstree_big} root={c.cwd}"
                      f"   (the replay rebuilds this tree)")
            print(f"      path      : {'+'.join(c.keys)}"
                  f"  → diverges at step #{res.fail_step} (key {res.fail_key!r})")
            if res.sig:
                print(f"      first diff: row {res.sig[0]}, col {res.sig[1]}")
            for note in res.shrink_notes:
                print(f"      note      : {note}")
            if res.min_edits is not None:
                print(f"      reduced edits: {edit_program_str(res.min_edits)}")
            if res.min_keys is not None:
                # "reduced", not "minimal": ddmin proves every element it KEPT
                # was load-bearing under the probes it ran, not that no smaller
                # set exists — and less so when the budget ran out. Raise
                # --shrink-probes to reduce further.
                label = ("reduced (budget exhausted, not minimal)"
                         if res.shrink_exhausted else "reduced (ddmin converged)")
                print(f"      {label}")
                print(f"        keys  : {'+'.join(res.min_keys)}"
                      f"  ({len(res.min_keys)}/{len(c.keys)})")
                if res.min_styles is not None:
                    print(f"        styles: {len(res.min_styles)}/"
                          f"{len(c.statements)}  [{res.probes} probes]")
                    for s in res.min_styles[:8]:
                        print(f"          {s}")
                else:
                    print(f"        styles: not reduced  [{res.probes} probes]")
            elif res.status == "FLAKY":
                print("      not shrunk: a flaky divergence is not a sound "
                      "delta-debugging oracle")
            for i, a, b in res.diffs[: (12 if args.verbose else 3)]:
                print(f"        row {i:2d}: zsh  = {a!r}")
                print(f"                 zshrs= {b!r}")
            if args.verbose and res.ref_grid:
                print("      --- zsh (ref) ---")
                print(render_grid(res.ref_grid))
                print("      --- zshrs (test) ---")
                print(render_grid(res.test_grid))
            print(f"      replay    : {res.replay}")
            results.append(_cell_json(res))
    finally:
        if pool is not None:
            pool.shutdown(wait=True)

    skipped = sum(skips.values())
    # Cells that produced no verdict at all, so `passed + failed + flaky` and
    # `cells run` visibly reconcile instead of quietly not adding up.
    skipped_cells = sum(n for (_, st), n in by_category.items() if st == "SKIP")
    print()
    print(f"# {passed} passed, {failed} failed, {flaky} flaky, "
          + (f"{died} died, " if died else "")
          + (f"{skipped_cells} not compared, " if skipped_cells else "")
          + f"{len(cells)} cells run")
    if deaths_by_side:
        print("# shells that DIED (own verdict — never a pass, never a plain "
              "FAIL, never a timeout):")
        for reason, count in sorted(deaths_by_side.items()):
            print(f"#   {reason}: {count}")
    if by_category and (args.edit_fuzz or args.fstree_fuzz
                        or args.interrupt_fuzz or args.session_fuzz):
        print("# per-category (generator[mode]):")
        cats = sorted({c for c, _ in by_category})
        for cat in cats:
            counts = {st: by_category[(cat, st)]
                      for st in ("PASS", "FAIL", "FLAKY", "SKIP", "DIED")
                      if by_category[(cat, st)]}
            total = sum(counts.values())
            detail = "  ".join(f"{k}={v}" for k, v in counts.items())
            print(f"#   {cat:34s} {total:3d}  {detail}")
    if session_notes:
        # NOT verdicts. State both shells accumulate identically is zsh's own
        # behaviour faithfully reproduced; a shell that is not self-consistent
        # across repetitions is a finding about THAT shell, not about the pair;
        # a probe that could not be taken is an assertion that was not made.
        # All three are printed and counted here precisely so that none of them
        # can be mistaken for the pair agreeing.
        print("# session-fuzz observations (counted, NOT parity verdicts):")
        for reason, count in sorted(session_notes.items()):
            if count:
                print(f"#   {reason}: {count}")
    if expect_bad:
        # Generator sanity, NOT a parity result. A mismatch here says the
        # harness's own claim about what an edit program produces is wrong
        # (or the line wrapped and could not be read back); it changed no
        # cell's verdict and suppressed no comparison.
        print("# generator-sanity mismatches (harness bug, verdicts unaffected):")
        for reason, count in sorted(expect_bad.items()):
            print(f"#   {reason}  x{count}")
    if skips:
        print(f"# {skipped} skipped (never compared):")
        for reason, count in sorted(skips.items()):
            print(f"#   {reason}: {count}")
    lat_over = book.report() if book is not None else 0
    if args.json:
        doc = {
            "schema": "compsys-parity/1",
            "mode": "fuzz",
            "argv": sys.argv[1:],
            "zshrs": args.zshrs,
            "zsh": args.zsh,
            "dump": dump,
            "zstyle": args.zstyle,
            "seed": args.seed,
            "buffer_fuzz": args.buffer_fuzz,
            "multiline_fuzz": args.multiline_fuzz,
            "geometry_fuzz": args.geometry_fuzz,
            "edit_fuzz": args.edit_fuzz,
            "edit_modes": list(args.edit_modes_list),
            "fstree_fuzz": args.fstree_fuzz,
            "fstree": ({"seed": tree.seed, "root": tree.root, "token": tree.tok,
                        "big": args.fstree_big,
                        "planned": len(tree.plan),
                        "created": len(tree.created),
                        "folded": [{"entry": r, "why": w} for r, w in tree.folded],
                        "rejected": [{"entry": r, "why": w}
                                     for r, w in tree.rejected],
                        "skipped": [{"entry": r, "why": w}
                                    for r, w in tree.skipped],
                        "categories": sorted(tree.prefixes)}
                       if tree is not None else None),
            "session_fuzz": args.session_fuzz,
            "session_repeat": args.session_repeat,
            "session_probes": list(PROBE_FILES),
            "session_excluded": list(PROBE_EXCLUDED),
            # Its own key, never inside `summary`: an observation is not a
            # verdict and no consumer should be able to read it as one.
            "session_notes": dict(session_notes),
            "interrupt_fuzz": args.interrupt_fuzz,
            "interrupt_kinds": list(args.interrupt_kinds_list),
            "interrupt_delay_ms": args.interrupt_delay_ms,
            "deaths": dict(deaths_by_side),
            "categories": {f"{cat}|{st}": n
                           for (cat, st), n in sorted(by_category.items())},
            "generator_sanity_mismatches": dict(expect_bad),
            "geom": {"rows": args.rows, "cols": args.cols, "settle_ms": args.settle},
            "jobs": max(1, args.jobs),
            "confirm": args.confirm,
            "shrink_probes": args.shrink_probes,
            "summary": {"passed": passed, "failed": failed, "flaky": flaky,
                        # Its own key: a DIED cell is not a parity failure and
                        # must not be read as one, but it is also never a pass —
                        # it makes the run exit non-zero on its own.
                        "died": died,
                        "skipped": skipped, "cells": len(cells)},
            "skips": dict(skips),
            # Its own top-level key, never folded into `summary`: a latency
            # finding is not a parity result and no consumer should be able to
            # read it as one by accident.
            "latency": (book.json_doc() if book is not None else None),
            "results": results,
        }
        _write_json(args.json, doc)
    # Flaky is NOT a pass: a cell that diverges only sometimes is still a cell
    # whose two shells did not agree, so it fails the run.
    #
    # `lat_over` is counted SEPARATELY and only ever ADDS: it is non-zero only
    # when the user asked for a --latency-threshold and a cell crossed it. It
    # can never turn a correctness failure into a pass, and with the flag unset
    # (the default) it is always 0, so every pre-existing run keeps its exit
    # code exactly.
    #
    # `died` likewise only ever ADDS: a run in which a shell crashed cannot
    # exit 0 no matter what the surviving screens said.
    if args.fstree_fuzz and tree is not None and not args.fstree_keep:
        fstree_cleanup(tree.root)
    elif args.fstree_fuzz and tree is not None:
        print(f"# fstree kept at {tree.root} (--fstree-keep)")
    return 1 if (failed or flaky or died or lat_over) else 0


def _cell_json(res) -> dict:
    c = res.cell
    doc = {
        "id": f"combo{c.idx}.{c.surface}",
        "combo": c.idx,
        "surface": c.surface,
        "buffer": c.buffer,
        "keys": list(c.keys),
        "geom": {"rows": c.geom.rows, "cols": c.geom.cols},
        "status": res.status,
        "detail": res.detail,
        "fail_step": res.fail_step,
        "fail_key": res.fail_key,
        "first_diff": ({"row": res.sig[0], "col": res.sig[1]} if res.sig else None),
        "rows_differ": len(res.diffs),
        "diff_rows": [{"row": i, "ref": a, "test": b} for i, a, b in res.diffs[:50]],
        "min_keys": res.min_keys,
        "min_styles": res.min_styles,
        "shrink_probes": res.probes,
        "shrink_exhausted": res.shrink_exhausted,
        "shrink_notes": list(res.shrink_notes),
        "zstyle_file": c.zstyle_path,
        "replay": res.replay,
    }
    if c.fstree_seed is not None:
        doc.update(fstree_seed=c.fstree_seed, fstree_root=c.cwd)
    if c.interrupt is not None:
        doc.update(interrupt=interrupt_encode(c.interrupt),
                   interrupt_kind=c.interrupt.kind,
                   interrupt_at=c.interrupt.at)
    if res.deaths:
        doc["deaths"] = [{"side": s, "kind": k, "value": v}
                         for s, k, v in res.deaths]
    if res.latency:
        # Under `latency`, never under `status`/`detail`: the cell's parity
        # verdict is what the fields above say and nothing here modifies it.
        doc["latency"] = [
            {"key": label, "key_index": i,
             "ref_ttfb_ms": (round(rt.ttfb, 2)
                             if rt and rt.ttfb is not None else None),
             "ref_settle_ms": (round(rt.settle, 2)
                               if rt and rt.settle is not None else None),
             "test_ttfb_ms": (round(tt.ttfb, 2)
                              if tt and tt.ttfb is not None else None),
             "test_settle_ms": (round(tt.settle, 2)
                                if tt and tt.settle is not None else None)}
            for i, (label, rt, tt) in enumerate(res.latency, 1)]
    if c.edit_tokens is not None:
        doc.update(
            edit_mode=c.edit_mode,
            edit_gen=c.edit_gen,
            edit_program=edit_encode(c.edit_tokens),
            min_edit_program=(edit_encode(res.min_edits)
                              if res.min_edits is not None else None),
            expect=c.expect,
            expect_ok=res.expect_ok,
            expect_saw=res.expect_saw,
        )
    if isinstance(c, SessionCell):
        idem = res.idempotence or {"zsh": [], "zshrs": []}
        doc.update(
            session=True,
            episodes=episodes_encode(c.episodes),
            repeat=c.repeat,
            min_episodes=(episodes_encode(res.min_episodes)
                          if res.min_episodes is not None else None),
            episodes_done=(res.session.episodes_done if res.session else 0),
            parity_assertions=(len(res.session.records) if res.session else 0),
            state_probes=(len(res.session.probes) if res.session else 0),
            drifts=[{"episode": d.episode, "rep": d.rep, "probe": d.probe,
                     "after": d.after, "agreed_at_previous": d.prev_matched,
                     "ref_added": list(d.ref_added),
                     "ref_removed": list(d.ref_removed),
                     "test_added": list(d.test_added),
                     "test_removed": list(d.test_removed)}
                    for d in res.drifts],
            # Explicitly NOT a verdict — see `session_notes` in the run doc.
            shared_drift=dict(res.shared_drift or {}),
            baseline_diffs=[{"probe": n, "only_zsh": list(r),
                             "only_zshrs": list(t)}
                            for n, r, t in res.baseline_diffs],
            probe_failures=list(res.probe_failures),
            not_idempotent={
                # Three groups, never one number: only `zshrs_only` is a defect
                # here, and a consumer that summed them would be counting zsh's
                # own behaviour against zshrs.
                "zshrs_only": [str(p) for p in idem_split(idem)[0]],
                "both_shells": [str(p) for p in idem_split(idem)[1]],
                "zsh_only": [str(p) for p in idem_split(idem)[2]]},
        )
    if isinstance(c, ConvCell):
        doc.update(
            convergent=True,
            target=c.target,
            leg_b_buffer=c.b_buffer,
            leg_b_edit_program=edit_encode(c.b_edit_tokens),
        )
    return doc


def _write_json(path, doc):
    text = json.dumps(doc, indent=2)
    if path == "-":
        print(text)
    else:
        with open(path, "w") as f:
            f.write(text + "\n")
        print(f"# json: {path}")


def run_lockstep_case(args, init_file, env):
    """`--case ... --keys ... --lockstep`: the ad-hoc case run the way the
    fuzzer runs it — both screens diffed after EVERY key — so a replay line
    printed by the fuzzer reproduces the same first-diff cell."""
    keys = [k.strip() for k in args.keys.split(",") if k.strip()]
    if not keys:
        sys.exit("compsys_parity: --lockstep needs at least one key in --keys")
    geom = Geom(args.rows, args.cols)
    edits = args.edit_tokens
    deaths: list = []
    fail_step, records = run_keyseq(init_file, args.case, keys, args, env, geom,
                                    edits=edits, interrupt=args.interrupt_obj,
                                    deaths_out=deaths)
    step, key, rg, tg, diffs = records[-1]
    # Latency, measured in its OWN runs after the verdict-producing one above,
    # so the number reported here is never the thing that decided PASS/FAIL.
    book = None
    if args.latency:
        book = LatencyBook(args.latency_min_ms, args.latency_threshold,
                           args.latency_runs, concurrent_drain=True)
        runs = []
        for _ in range(args.latency_runs):
            t = []
            run_keyseq(init_file, args.case, keys, args, env, geom,
                       edits=edits, timings_out=t)
            runs.append(t)
        book.record(f"lockstep:{args.case}", merge_best_timings(runs))
    if edits is not None:
        print(f"# edit program : {edit_program_str(edits)}"
              f"   mode={args.editing_mode or 'shell default'}")
    if args.interrupt_obj is not None:
        print(f"# interrupt    : {args.interrupt_obj.label()}"
              f"   (delay {args.interrupt_delay_ms}ms)")
    if deaths:
        # Named BEFORE any grid comparison is reported: a dead shell's last
        # frame is not a screen the other shell can be measured against.
        print(f"DIED lockstep {args.case!r} [{'+'.join(keys)}] {geom_str(geom)}"
              f"  at step #{step} (step {key!r}): {death_str(deaths)}")
        if args.json:
            _write_json(args.json, {
                "schema": "compsys-parity/1", "mode": "lockstep",
                "argv": sys.argv[1:], "zshrs": args.zshrs, "zsh": args.zsh,
                "summary": {"passed": 0, "failed": 0, "flaky": 0, "died": 1,
                            "skipped": 0, "cells": 1},
                "results": [{"id": "lockstep", "status": "DIED",
                             "fail_step": step, "fail_key": key,
                             "deaths": [{"side": s, "kind": k, "value": v}
                                        for s, k, v in deaths]}],
            })
        return 1
    if fail_step == 0:
        print(f"PASS lockstep {args.case!r} [{'+'.join(keys)}] {geom_str(geom)}"
              f"  ({len(records)} steps, screens identical after every one)")
        rc = 0
        doc_res = {"status": "PASS"}
    else:
        row, col = first_diff_cell(diffs) if diffs else (-1, -1)
        print(f"FAIL lockstep {args.case!r} [{'+'.join(keys)}] {geom_str(geom)}"
              f"  diverges at step #{step} (step {key!r})"
              + (f", {len(diffs)} rows differ, first diff row {row} col {col}"
                 if diffs else " (a shell never reached prompt)"))
        for i, a, b in (diffs or []):
            print(f"  row {i:2d}: zsh  = {a!r}")
            print(f"          zshrs= {b!r}")
        if args.verbose and rg is not None:
            print("  --- zsh (ref) ---")
            print(render_grid(rg))
            print("  --- zshrs (test) ---")
            print(render_grid(tg))
        rc = 1
        doc_res = {"status": "FAIL", "fail_step": step, "fail_key": key,
                   "first_diff": {"row": row, "col": col},
                   "diff_rows": [{"row": i, "ref": a, "test": b}
                                 for i, a, b in (diffs or [])[:50]]}
    lat_over = book.report() if book is not None else 0
    if args.json:
        doc_res.update(id="lockstep", buffer=args.case, keys=keys,
                       edit_program=(edit_encode(edits) if edits else None),
                       editing_mode=args.editing_mode,
                       geom={"rows": geom.rows, "cols": geom.cols})
        _write_json(args.json, {
            "schema": "compsys-parity/1", "mode": "lockstep",
            "argv": sys.argv[1:], "zshrs": args.zshrs, "zsh": args.zsh,
            # `summary` stays purely the CORRECTNESS verdict — `rc` here is the
            # parity result and latency is not allowed to touch it.
            "summary": {"passed": 1 - rc, "failed": rc, "flaky": 0,
                        "skipped": 0, "cells": 1},
            "latency": (book.json_doc() if book is not None else None),
            "results": [doc_res],
        })
    # Exit code only: a threshold the user explicitly asked for, ADDED to the
    # correctness result. `rc` itself — and everything reported above and in the
    # JSON — is untouched by latency.
    return 1 if (rc or lat_over) else 0


def run_conv_replay(args, dump, fpath_dirs, env):
    """`--conv-replay '<json>'`: re-run one convergent PAIR exactly as the
    fuzzer ran it, and print the four-way comparison the verdict rests on."""
    spec = json.loads(args.conv_replay)
    geom = Geom(*spec["geom"])
    keys = list(spec["keys"])
    a_edits = edit_decode(spec["a"]["edits"])
    b_edits = edit_decode(spec["b"]["edits"])
    zstyle = spec.get("zstyle") or None
    if zstyle and not os.path.exists(zstyle):
        sys.exit(f"compsys_parity: zstyle fixture from the replay is gone: {zstyle}")
    init = build_init_file(dump, fpath_dirs, zstyle, spec.get("mode"))
    print(f"# convergent pair : {spec['name']}  mode={spec.get('mode')}  "
          f"{geom_str(geom)}  keys={'+'.join(keys)}")
    print(f"# target line     : {spec.get('target')!r}")
    print(f"# leg A           : {spec['a']['buffer']!r} + "
          f"{edit_program_str(a_edits)}")
    print(f"# leg B           : {spec['b']['buffer']!r} + "
          f"{edit_program_str(b_edits)}")

    legs = {}
    for tag, side, edits in (("A", spec["a"], a_edits), ("B", spec["b"], b_edits)):
        fs, rec = run_keyseq(init, side["buffer"], keys, args, env, geom,
                             edits=edits)
        if fs:
            step, label, rg, tg, diffs = rec[-1]
            print(f"FAIL leg {tag} diverges zsh-vs-zshrs at step #{step} "
                  f"({label!r}) before the pair could be compared")
            for i, x, y in (diffs or [])[:20]:
                print(f"  row {i:2d}: zsh  = {x!r}")
                print(f"          zshrs= {y!r}")
            return 1
        legs[tag] = (rec[-1][2], rec[-1][3])

    (ref_a, test_a), (ref_b, test_b) = legs["A"], legs["B"]
    if ref_a != ref_b:
        print("SKIP non-convergent-in-reference: zsh itself ends the two "
              "programs on different screens — the pair establishes nothing")
        for i, x, y in diff_grids(ref_a, ref_b)[:20]:
            print(f"  row {i:2d}: zsh A = {x!r}")
            print(f"          zsh B = {y!r}")
        return 1
    d = diff_grids(test_a, test_b)
    if not d:
        print("PASS both edit programs converge on BOTH shells")
        return 0
    row, col = first_diff_cell(d)
    print(f"FAIL edit-history leakage: zsh ends both programs on the SAME "
          f"screen, zshrs on {len(d)} differing rows "
          f"(first diff row {row}, col {col})")
    for i, x, y in d:
        print(f"  row {i:2d}: zshrs A = {x!r}")
        print(f"          zshrs B = {y!r}")
    if args.verbose:
        print("  --- zsh (both legs, identical) ---")
        print(render_grid(ref_a))
        print("  --- zshrs leg A ---")
        print(render_grid(test_a))
        print("  --- zshrs leg B ---")
        print(render_grid(test_b))
    return 1


def run_session_replay(args, dump, fpath_dirs, env):
    """`--session-replay '<json>'`: re-run one SESSION exactly as the fuzzer ran
    it, and print the per-episode parity, the state probes and the idempotence
    check again.

    A session finding is a claim about an ORDER of completions inside one
    process, so a single-completion `--lockstep` line cannot reproduce it — the
    whole episode sequence travels as one JSON argument, the way a convergent
    pair does."""
    spec = json.loads(args.session_replay)
    geom = Geom(*spec["geom"])
    repeat = int(spec.get("repeat", 1))
    episodes = episodes_decode(spec["episodes"])
    zstyle = spec.get("zstyle") or None
    if zstyle and not os.path.exists(zstyle):
        sys.exit(f"compsys_parity: zstyle fixture from the replay is gone: "
                 f"{zstyle}")
    init = build_init_file(dump, fpath_dirs, zstyle, None, None, True)
    root = tempfile.mkdtemp(prefix="cp_replay_")
    print(f"# session replay : {len(episodes)} episodes x{repeat}  "
          f"{geom_str(geom)}")
    for i, ep in enumerate(episodes, 1):
        print(f"#   ep {i:2d} : {ep.label()}")
    deaths: list = []
    run = run_session(init, episodes, args, env, geom, repeat, root,
                      deaths_out=deaths)
    if deaths:
        step, key = (run.records[-1][0], run.records[-1][1]) if run.records \
            else (0, "(init)")
        print(f"DIED at step #{step} ({key!r}): {death_str(deaths)}")
        return 1
    if run.fail_step:
        step, key, rg, tg, diffs = run.records[-1]
        print(f"FAIL session diverges at step #{step} ({key!r})"
              + (f", {len(diffs)} rows differ" if diffs
                 else " (a shell never reached prompt)"))
        for i, a, b in (diffs or []):
            print(f"  row {i:2d}: zsh  = {a!r}")
            print(f"          zshrs= {b!r}")
        if args.verbose and rg is not None:
            print("  --- zsh (ref) ---")
            print(render_grid(rg))
            print("  --- zshrs (test) ---")
            print(render_grid(tg))
        return 1
    drifts, shared, base, unpaired = analyse_drift(run)
    idem = analyse_idempotence(run)
    for name, only_r, only_t in base:
        print(f"# baseline  : ${name} already differs before any completion")
        if only_r:
            print(f"#   only zsh  : {', '.join(only_r[:12])}")
        if only_t:
            print(f"#   only zshrs: {', '.join(only_t[:12])}")
    for name, cnt in sorted(shared.items()):
        print(f"# state     : ${name} moved identically on BOTH shells at "
              f"{cnt} probe(s) — not a finding")
    for reason in list(run.probe_failures) + list(unpaired):
        print(f"# probe     : {reason}  (state NOT asserted here)")
    only_t, shared_i, only_r = idem_split(idem)
    for pt in only_t:
        print(f"# IDEMPOTENCE: zshrs ONLY — {pt}")
    for pt in shared_i:
        print(f"# idempotence: BOTH shells — {pt}  (zsh's own, reproduced)")
    for pt in only_r:
        print(f"# idempotence: zsh (reference) ONLY — {pt}")
    if not drifts:
        tail = (f"{run.episodes_done} episode runs, {len(run.records)} parity "
                f"assertions, {len(run.probes)} state probes, no drift")
        # zshrs disagreeing with ITSELF where zsh does not is a failure even
        # when every cross-shell assertion agreed. Points where BOTH shells are
        # non-idempotent are not: a second completion in one process
        # legitimately sees state the first one loaded, zsh does exactly that,
        # and scoring a faithful reproduction as a defect would be a false
        # positive in an instrument whose whole value is that it has none.
        if only_t:
            print(f"FAIL zshrs is not self-consistent across repetitions at "
                  f"{len(only_t)} point(s) where zsh IS; {tail}")
            return 1
        print(f"PASS {tail}")
        return 0
    d0 = drifts[0]
    print(f"FAIL state drift in ${d0.probe} at episode {d0.episode} "
          + ("(agreed at the previous episode)" if d0.prev_matched
             else "(already drifting before)"))
    for d in drifts:
        print(f"  ${d.probe} @ episode {d.episode} rep {d.rep}  after "
              f"{d.after}")
        for ln in d.named():
            print(f"    {ln}")
    return 1


def main():
    ap = argparse.ArgumentParser(description="compsys parity harness")
    ap.add_argument("--zshrs", default=default_zshrs())
    ap.add_argument("--zsh", default="zsh")
    ap.add_argument("--dump", default=None)
    ap.add_argument("--no-dump", action="store_true",
                    help="force fresh compinit fpath-scan on both shells (no cached dump)")
    ap.add_argument("--fpath", action="append", default=[],
                    help="fpath dir prepended on both shells (repeatable)")
    ap.add_argument("--std-fpath", action="store_true",
                    help="use only the standard zsh completion dirs on both shells")
    ap.add_argument("--user-fpath", action="store_true", default=True,
                    help="use the user's real fpath (default)")
    ap.add_argument("--zstyle", default=os.path.join(REPO, "scripts", "parity_zstyle.zsh"),
                    help="zstyle fixture sourced into both shells")
    ap.add_argument("--no-zstyle", action="store_true", help="skip the zstyle fixture")
    ap.add_argument("--random-combos", type=int, default=0, metavar="N",
                    help="fuzz N random subsets of the zstyle fixture (parity must hold for ALL)")
    ap.add_argument("--seed", type=int, default=0, help="RNG seed for --random-combos (reproducible)")
    ap.add_argument("--combo-keep", type=float, default=0.5,
                    help="per-statement keep probability for random combos (default 0.5)")
    ap.add_argument("--combo-commands", default="git ,ssh -,cd /,kill -,echo $PA",
                    help="comma-separated buffers to complete in each random combo")
    ap.add_argument("--buffer-fuzz", action="store_true",
                    help="fuzz the COMMAND LINE too, from the documented surface "
                         "set (mid-word cursor, quotes, $var/${, globs, braces, "
                         "tildes, redirection targets, assignment RHS, $( ) "
                         "interior, sudo-prefixed, backslash-escaped spaces, "
                         "partial long options). Without it the fixed "
                         "--combo-commands list is used, unchanged.")
    ap.add_argument("--buffer-cases", type=int, default=0, metavar="N",
                    help="fuzzed buffers per combo (default: as many as "
                         "--combo-commands has entries)")
    ap.add_argument("--edit-fuzz", action="store_true",
                    help="fuzz the LINE EDITING that produces the buffer: "
                         "emacs word-kills / ^A^K / motions / transpose / "
                         "kill-yank / undo / backspace runs / retype-over, vi "
                         "normal-mode motions and dw/db/dd/cw/x/./u under "
                         "`bindkey -v`, and paste-shaped bursts (bracketed and "
                         "raw). Parity is asserted after EVERY edit token as "
                         "well as after every completion key, so a divergence "
                         "is attributed to the edit that caused it. Also "
                         "generates CONVERGENT PAIRS: two different edit "
                         "programs with the same final line, which isolate "
                         "edit-history leakage from ordinary completion "
                         "behaviour. Implies --random-combos 1 if that is 0.")
    ap.add_argument("--edit-cases", type=int, default=4, metavar="N",
                    help="edit-program cells per combo (default 4)")
    ap.add_argument("--convergent-cases", type=int, default=2, metavar="N",
                    help="convergent PAIRS per combo; each runs two lockstep "
                         "legs, so it costs twice an ordinary cell (default 2, "
                         "0 disables)")
    ap.add_argument("--edit-modes", default="emacs,vi,emacs-nobp",
                    help="comma-separated editing modes for --edit-fuzz: "
                         + ", ".join(EDIT_MODES))
    ap.add_argument("--edit-program", default=None, metavar="SPEC",
                    help="edit program for --lockstep, in the k:/t:/p:/b: DSL "
                         "(the form the fuzzer's replay lines carry)")
    ap.add_argument("--editing-mode", default=None, choices=sorted(EDIT_MODES),
                    help="editing mode for --lockstep (appends bindkey -e / "
                         "-v, and for emacs-nobp unsets zle_bracketed_paste, "
                         "to the shared init on BOTH shells)")
    ap.add_argument("--conv-replay", default=None, metavar="JSON",
                    help="re-run one convergent PAIR from the JSON blob a "
                         "convergent failure prints")
    ap.add_argument("--multiline-fuzz", action="store_true",
                    help="fuzz the CONTINUATION context the completion fires "
                         "in: trailing-backslash continuation, an unterminated "
                         "single/double quote spanning lines, $( ) and "
                         "backquotes spanning lines, a for/while/if/case body "
                         "after the newline (and after do/then), an x=( array "
                         "literal, a heredoc body and its terminator line, and "
                         "a trailing |, && or ||. Every generated buffer is "
                         "INCOMPLETE at each newline, so Return is answered "
                         "with PS2 on both shells and nothing is executed. "
                         "Composes with --buffer-fuzz (both surface pools) and "
                         "with --geometry-fuzz, where a completion list under a "
                         "multi-row prompt is the shape of a bug this project "
                         "has already shipped. Implies --random-combos 1 if "
                         "that is 0.")
    ap.add_argument("--fstree-fuzz", action="store_true",
                    help="build a SEEDED, HERMETIC directory tree in a scratch "
                         "dir, cd BOTH shells into it (same absolute path) and "
                         "complete inside it. Covers names that are hostile to "
                         "a completer: spaces/tabs/leading+trailing spaces and "
                         "a literal newline; glob metacharacters (* ? [ ] { } ~ "
                         "# ^) as literal name content; quote and escape "
                         "characters (' \" \\ ` $ !); option-looking names "
                         "(-dash, --ddash, -) and dotfiles; a NAME_MAX-length "
                         "name and a chain near PATH_MAX; a long shared prefix "
                         "and a directory of thousands of entries (the pager); "
                         "symlinks to a dir, to a file, dangling, and a LOOP; "
                         "directories with no read and no execute permission; "
                         "and case-colliding names. Every planned entry is "
                         "verified on disk and a folded/rejected one disables "
                         "its category as a counted GENERATOR issue, never a "
                         "finding. Implies --random-combos 1 if that is 0.")
    ap.add_argument("--fstree-seed", type=int, default=None, metavar="N",
                    help="seed for the tree (default: --seed). The tree is a "
                         "pure function of this seed and --fstree-big, so this "
                         "is what a replay rebuilds it from; it is printed on "
                         "every fstree result line and in every replay.")
    ap.add_argument("--fstree-big", type=int, default=3000, metavar="N",
                    help="entries in the large directory that exercises the "
                         "listing/paging path (default 3000)")
    ap.add_argument("--fstree-cases", type=int, default=6, metavar="N",
                    help="fstree cells per combo (default 6)")
    ap.add_argument("--fstree-keep", action="store_true",
                    help="do NOT delete the tree at the end of the run "
                         "(default: removed; a replay rebuilds it from the seed)")
    ap.add_argument("--fstree-verify", action="store_true",
                    help="build the tree, print what the FILESYSTEM actually "
                         "accepted, folded or rejected, remove it and exit. "
                         "Runs no shell — this is the proof that the tree "
                         "matched its specification.")
    ap.add_argument("--interrupt-fuzz", action="store_true",
                    help="interrupt the completion. Delivers, IDENTICALLY to "
                         "both shells at one controlled anchor: a real terminal "
                         "resize (TIOCSWINSZ on the pty, which is what raises "
                         "SIGWINCH), SIGINT to the shell's process group, or a "
                         "burst of type-ahead. Anchors are `before` (the first "
                         "TAB), `menu` (a listing is on screen) and `midkey<N>` "
                         "(--interrupt-delay-ms after key N's write, while the "
                         "shell is still computing). A shell that DIES gets its "
                         "own verdict naming the side and the signal — never a "
                         "pass, never a plain FAIL, never a timeout. Implies "
                         "--random-combos 1 if that is 0.")
    ap.add_argument("--interrupt-cases", type=int, default=4, metavar="N",
                    help="interrupted cells per combo (default 4)")
    ap.add_argument("--interrupt-kinds", default=",".join(INTERRUPT_KINDS),
                    help="comma-separated subset of " + ",".join(INTERRUPT_KINDS))
    ap.add_argument("--interrupt-delay-ms", type=int, default=40, metavar="MS",
                    help="delay between a key's write and a midkey interrupt "
                         "(default 40). Nominal and identical on both sides; "
                         "the two shells do not compute for the same length of "
                         "time, so a midkey finding can legitimately be FLAKY.")
    ap.add_argument("--interrupt", default=None, metavar="SPEC",
                    help="interrupt for --lockstep, in the form the fuzzer's "
                         "replay lines carry: winch@menu:8x100, int@midkey1, "
                         "type@midkey1:<percent-encoded>")
    ap.add_argument("--session-fuzz", type=int, default=0, metavar="N",
                    help="run N completion EPISODES inside ONE pair of shells "
                         "instead of booting a fresh pair per completion. "
                         "Parity is asserted after every step of every episode, "
                         "and between episodes both shells write their own "
                         "state (parameter/function/alias/command name sets, "
                         "the full option set, setopt, funcstack depth, "
                         "compstate/WIDGET/LASTWIDGET/_tags_level/_comp_tags "
                         "binding) to a file which is diffed here. A probe that "
                         "moves DIFFERENTLY on the two shells is a drift "
                         "finding and names the parameters; one that moves "
                         "identically on both is zsh's own behaviour and is "
                         "counted separately. The fuzzed buffer is NEVER "
                         "executed. Implies --random-combos 1 if that is 0.")
    ap.add_argument("--session-cases", type=int, default=1, metavar="N",
                    help="sessions per combo (default 1). Each one is a whole "
                         "pair of shells kept alive for N episodes.")
    ap.add_argument("--session-repeat", type=int, default=2, metavar="M",
                    help="run each episode M times back-to-back and require "
                         "every repetition to render identically to the first "
                         "— on EACH shell independently, before the two are "
                         "compared with each other (default 2). A shell that is "
                         "not self-consistent across repetitions is a finding "
                         "of its own and is named as such; 1 disables the "
                         "check.")
    ap.add_argument("--session-shrink-probes", type=int, default=8, metavar="N",
                    help="delta-debugging budget for reducing the EPISODE "
                         "SEQUENCE of a drift finding (default 8). Its own "
                         "budget because one probe here re-runs a whole session "
                         "— far more expensive than a key-path probe. 0 "
                         "disables sequence reduction.")
    ap.add_argument("--session-replay", default=None, metavar="SPEC",
                    help="re-run one session from the JSON spec a session "
                         "finding prints, and show the per-episode parity and "
                         "the state probes again")
    ap.add_argument("--latency", action="store_true",
                    help="also MEASURE how long each keystroke takes on both "
                         "shells (time to first byte and to settle) and report "
                         "the zshrs/zsh ratio per key and per cell. Strictly "
                         "additive: latency lives in its own verdict namespace "
                         "(LAT-OUTLIER / LAT-OVER-THRESHOLD) and can never "
                         "change a correctness PASS/FAIL/FLAKY. Refused at "
                         "--jobs > 1, where the numbers would be fiction.")
    ap.add_argument("--latency-runs", type=int, default=3, metavar="K",
                    help="time each cell K times and keep the BEST of each "
                         "side (default 3). Best-of, not mean: load on this box "
                         "only ever makes a measurement worse, and a mean folds "
                         "one scheduling hiccup straight into the ratio.")
    ap.add_argument("--latency-min-ms", type=float, default=25.0, metavar="MS",
                    help="minimum ABSOLUTE zshrs-minus-zsh difference before a "
                         "ratio is reported or flagged at all (default 25). "
                         "Without it a 2ms-vs-1ms key reads as a 2x regression.")
    ap.add_argument("--latency-threshold", type=float, default=0.0, metavar="N",
                    help="flag a cell LAT-OVER-THRESHOLD when zshrs is more "
                         "than N times slower than the reference on some key "
                         "(and make the run exit non-zero for it). Default 0 = "
                         "report only, so existing runs keep their verdicts. "
                         "Note the binary under test is a DEBUG build, so the "
                         "ratio has a floor that is a build artefact — the "
                         "unthresholded OUTLIER flagging against this run's own "
                         "baseline distribution is usually the better signal.")
    ap.add_argument("--geometry-fuzz", action="store_true",
                    help="draw (rows, cols) per cell from the seeded pool "
                         "(narrow 40-col, tiny 6-8-row, wide 200-col, ...). Both "
                         "shells always get the IDENTICAL geometry; the chosen "
                         "one is printed in every result line and replay.")
    ap.add_argument("--shrink-probes", type=int, default=20, metavar="N",
                    help="delta-debugging budget PER AXIS (key path, then zstyle "
                         "subset) for each reproducible failure; 0 disables "
                         "shrinking. Each probe boots two shells. A reduction "
                         "that hits the budget is reported as 'budget "
                         "exhausted, not minimal' — raise it to reduce further.")
    ap.add_argument("--jobs", type=int, default=1, metavar="N",
                    help="run N cells concurrently. Every cell is an independent "
                         "pty pair, so the comparison itself is unaffected, but "
                         "load slows a redraw and a marginal cell flips verdict: "
                         "comptab_parity.py records two back-to-back sweeps at "
                         "--jobs 4 --confirm 0 disagreeing on 3 cells. Keep "
                         "--confirm on in parallel so those are LABELLED flaky "
                         "instead of landing on whichever verdict the load "
                         "happened to produce.")
    ap.add_argument("--lockstep", action="store_true",
                    help="run --case in lockstep (diff both screens after EVERY "
                         "key, report the first diverging step) — the mode the "
                         "fuzzer's replay lines use")
    ap.add_argument("--presses", type=int, default=5, metavar="N",
                    help="length of the random key path per combo case (TAB cycle/page + arrow "
                         "menu-nav + literal-letter interactive filter), parity asserted after "
                         "EACH key; default 5")
    ap.add_argument("--confirm", type=int, default=2, metavar="K",
                    help="re-run a failing cell K times to LABEL it: a divergence "
                         "that does not reproduce at the same first-diff cell is "
                         "reported as FLAKY in its own counted category. Never "
                         "turns a failure into a pass — nondeterminism is a bug "
                         "class of its own here (default 2)")
    ap.add_argument("--rows", type=int, default=24)
    ap.add_argument("--cols", type=int, default=80)
    ap.add_argument("--settle", type=int, default=250, help="quiet window ms")
    ap.add_argument("--case", help="ad-hoc buffer text to type")
    ap.add_argument("--keys", default="tab", help="comma keys for --case")
    ap.add_argument("--only", help="run one built-in case by name")
    ap.add_argument("--list", action="store_true", help="list built-in cases")
    ap.add_argument("--sequences", default=None,
                    help="comma-separated names from parity_corpus.KEY_SEQUENCES, "
                         "or 'default' / 'all'. Each case is expanded once per sequence.")
    ap.add_argument("--tag", default=None,
                    help="only run shared-corpus cases carrying this tag")
    ap.add_argument("--skip-optional", action="store_true",
                    help="drop cases needing a binary that may be absent")
    ap.add_argument("--json", default=None, metavar="PATH",
                    help="write the result document here ('-' for stdout)")
    ap.add_argument("-v", "--verbose", action="store_true")
    args = ap.parse_args()

    args.edit_modes_list = [m.strip() for m in args.edit_modes.split(",")
                            if m.strip()]
    unknown_modes = [m for m in args.edit_modes_list if m not in EDIT_MODES]
    if unknown_modes:
        sys.exit("unknown editing mode(s): " + ", ".join(unknown_modes))
    # Both surface pools are drawn from the same generator, so one flag is
    # enough to know a buffer is fuzzed rather than taken from --combo-commands.
    args.fuzz_buffers = args.buffer_fuzz or args.multiline_fuzz
    if args.multiline_fuzz and args.random_combos == 0:
        args.random_combos = 1
    # `--fstree-seed` defaults to the run seed, so one `--seed N` reproduces the
    # whole run including the tree; an explicit value pins the tree alone — and
    # in --lockstep it is ALSO the signal to rebuild the tree and cd into it,
    # which is how a replay line printed by an fstree failure reproduces.
    args.fstree_explicit = args.fstree_seed is not None
    if args.fstree_seed is None:
        args.fstree_seed = args.seed
    if args.fstree_big < 1:
        sys.exit("compsys_parity: --fstree-big must be at least 1")
    args.interrupt_kinds_list = [k.strip() for k in args.interrupt_kinds.split(",")
                                 if k.strip()]
    bad_kinds = [k for k in args.interrupt_kinds_list if k not in INTERRUPT_KINDS]
    if bad_kinds:
        sys.exit("unknown interrupt kind(s): " + ", ".join(bad_kinds))
    if args.interrupt_fuzz and not args.interrupt_kinds_list:
        sys.exit("compsys_parity: --interrupt-fuzz needs at least one kind")
    try:
        args.interrupt_obj = (interrupt_decode(args.interrupt)
                              if args.interrupt else None)
    except ValueError as exc:
        sys.exit(f"compsys_parity: {exc}")
    if (args.fstree_fuzz or args.interrupt_fuzz) and args.random_combos == 0:
        args.random_combos = 1
    if args.session_fuzz < 0:
        sys.exit("compsys_parity: --session-fuzz must not be negative")
    if args.session_repeat < 1:
        sys.exit("compsys_parity: --session-repeat must be at least 1")
    if args.session_cases < 1:
        sys.exit("compsys_parity: --session-cases must be at least 1")
    if args.session_fuzz > 0 and args.random_combos == 0:
        args.random_combos = 1
    # Latency is REFUSED under --jobs > 1 rather than reported with a caveat.
    # Concurrent cells contend for the same cores, so every number would be a
    # measurement of the harness's own scheduling, and a wrong number in an
    # audit instrument is worse than no number.
    if args.latency and args.jobs > 1:
        sys.exit("compsys_parity: --latency refuses --jobs > 1 — concurrent "
                 "cells contend for the same cores and the timings would be "
                 "measurements of the harness, not of the shells. Re-run with "
                 "--jobs 1 (correctness sweeps are unaffected).")
    if args.latency_runs < 1:
        sys.exit("compsys_parity: --latency-runs must be at least 1")
    if args.edit_fuzz and args.random_combos == 0:
        # `--edit-fuzz` on its own is a complete request. One combo is the
        # smallest thing that carries a zstyle subset for the cells to run
        # under; `--random-combos N` still means exactly what it meant.
        args.random_combos = 1
    # `is not None`, not truthiness: `--edit-program ''` is the EMPTY program a
    # shrunk finding prints when it proved the edits were not load-bearing, and
    # it is not the same thing as no `--edit-program` at all. The empty program
    # still runs the edit phase's baseline parity assertion after the buffer, so
    # the replay's step numbering matches the report it came from.
    args.edit_tokens = (edit_decode(args.edit_program)
                        if args.edit_program is not None else None)

    sel = (args.sequences or "default").strip()
    if sel == "all":
        seq_names = list(KEY_SEQUENCES)
    elif sel == "default":
        seq_names = list(DEFAULT_SEQUENCES)
    else:
        seq_names = [x.strip() for x in sel.split(",") if x.strip()]
        unknown = [x for x in seq_names if x not in KEY_SEQUENCES]
        if unknown:
            sys.exit("unknown sequence(s): " + ", ".join(unknown))
    builtin = builtin_cases(seq_names, args.tag, args.skip_optional)

    if args.list:
        for c in builtin:
            print(f"{c.name:16s} {c.buffer!r:20s} {'+'.join(c.keys):20s} {c.note}")
        return 0

    if args.fstree_verify:
        # Build, read back, report, remove. No shell is booted: this answers
        # "did the filesystem accept the tree the generator specified", which is
        # a question about THIS disk, not about either shell.
        tree = build_fstree(args.fstree_seed, args.fstree_big)
        try:
            for line in fstree_report(tree, limit=200):
                print(line)
            # Verified from what LANDED, not assumed from the platform: the
            # case-colliding pair is planned as two files, and whether the
            # second one folded onto the first is a property of THIS volume.
            case_folded = [w for r, w in tree.folded
                           if r.startswith(f"case{tree.tok}/")]
            print("#   case-insensitivity: "
                  + (f"CONFIRMED — the colliding twin {case_folded[0]}"
                     if case_folded
                     else "the case-colliding pair did NOT fold: this volume "
                          "is case-SENSITIVE and both names exist"))
            for cat, prefixes in sorted(tree.prefixes.items()):
                print(f"#   {cat:14s} {len(prefixes)} prefixes: "
                      + ", ".join(repr(p) for p in prefixes[:3]))
        finally:
            if not args.fstree_keep:
                fstree_cleanup(tree.root)
            else:
                print(f"# kept at {tree.root} (--fstree-keep)")
        return 0

    dump = None if args.no_dump else resolve_dump(args.dump)
    if args.std_fpath:
        fpath_dirs = std_fpath_dirs() + list(args.fpath)
    elif args.fpath:
        fpath_dirs = list(args.fpath)
    else:
        fpath_dirs = user_fpath()
    zstyle_file = None if args.no_zstyle else args.zstyle
    if not os.path.exists(args.zshrs):
        sys.exit(f"zshrs binary not found: {args.zshrs} (run: cargo build --bin zshrs)")

    if args.conv_replay:
        return run_conv_replay(args, dump, fpath_dirs,
                               child_env(args.rows, args.cols))

    if args.session_replay:
        return run_session_replay(args, dump, fpath_dirs,
                                  child_env(args.rows, args.cols))

    if args.random_combos > 0:
        return run_random_combos(args, dump, fpath_dirs,
                                 child_env(args.rows, args.cols))

    if args.lockstep:
        if args.case is None:
            sys.exit("compsys_parity: --lockstep needs --case")
        # A replay line from an fstree failure carries --fstree-seed. Rebuild
        # the identical tree at the identical absolute path FIRST, then cd both
        # shells into it, so the buffer's relative path means what it meant in
        # the run that reported the failure.
        tree = None
        if args.fstree_explicit or args.fstree_fuzz:
            tree = build_fstree(args.fstree_seed, args.fstree_big)
            for line in fstree_report(tree):
                print(line)
        try:
            return run_lockstep_case(
                args,
                build_init_file(dump, fpath_dirs, zstyle_file,
                                args.editing_mode,
                                tree.root if tree else None),
                child_env(args.rows, args.cols))
        finally:
            if tree is not None and not args.fstree_keep:
                fstree_cleanup(tree.root)

    if args.case is not None:
        cases = [Case("adhoc", args.case, [k.strip() for k in args.keys.split(",") if k.strip()])]
    elif args.only:
        cases = [c for c in builtin if c.name == args.only
                 or c.name.split('.')[0] == args.only]
        if not cases:
            sys.exit(f"no such case: {args.only}")
    else:
        cases = builtin

    init_file = build_init_file(dump, fpath_dirs, zstyle_file)
    env = child_env(args.rows, args.cols)

    print(f"# dump   : {dump or '<none>'}")
    print(f"# fpath  : {len(fpath_dirs)} dirs" + (f" (first: {fpath_dirs[0]})" if fpath_dirs else ""))
    print(f"# zstyle : {zstyle_file or '<none>'}")
    print(f"# init   : {init_file}")
    print(f"# zshrs  : {args.zshrs}")
    print(f"# zsh    : {args.zsh}")
    print(f"# geom   : {args.rows}x{args.cols}  settle={args.settle}ms")
    print(f"# jobs   : {max(1, args.jobs)}")
    print()

    # `-f`: no rc files; the harness sources the identical init explicitly.
    ref_argv = [args.zsh, "-f", "-i"]
    test_argv = [args.zshrs, "--zsh", "-f", "-i"]
    source_cmd = f"source {shlex.quote(init_file)}\n".encode()

    def capture(argv, label, case):
        # Fresh shell per case: no cross-case buffer/menu contamination, and
        # no dependence on Ctrl-C abort semantics matching between shells.
        sess = ShellSession(argv, env, args.rows, args.cols, label, args.settle)
        try:
            # Let the bare `-f` shell reach its first prompt, then load the
            # shared completion init and wait for OUR prompt sentinel.
            sess.drain_settled(max_wait=3.0, first_wait=2.0)
            sess.send(source_cmd)
            if not sess.wait_for_prompt(timeout=25.0):
                return None, []
            # `sess.key_timings` is per-key (key, KeyTiming); the correctness
            # path ignores it entirely. It is honest with no concurrent
            # draining here because each shell is captured in its OWN run, so
            # neither one is ever waiting on the other.
            return run_case(sess, case), list(sess.key_timings)
        finally:
            sess.close()

    # Flake labelling in the DEFAULT sweep is enabled only at --jobs > 1. At
    # --jobs 1 the harness is serial, which is the baseline every existing
    # corpus verdict was scored under, and re-running every failure would
    # triple the cost of a sweep for no new information. In parallel, a
    # marginal cell demonstrably flips under load (see the --jobs help), so the
    # re-run is what keeps the verdict honest — it can only ever turn FAIL into
    # FLAKY, which is still a failure, never into PASS.
    confirm_runs = args.confirm if args.jobs > 1 else 0

    def measure(case):
        """Best-of-K timing for one case, in its own runs.

        Runs AFTER the verdict is already fixed, and its screens are not
        scored: a latency number can never be what decided a parity result."""
        runs = []
        for _ in range(args.latency_runs):
            _, rt = capture(ref_argv, "zsh", case)
            _, tt = capture(test_argv, "zshrs", case)
            if not rt or not tt:
                continue
            runs.append([(k, a, b) for (k, a), (_, b) in zip(rt, tt)])
        return merge_best_timings(runs)

    def evaluate(case):
        """One cell: capture both shells, diff, and (in parallel mode) re-run a
        failure to label nondeterminism. Returns (status, ref, test, diffs,
        detail, latency)."""
        ref_grid, _ = capture(ref_argv, "zsh", case)
        test_grid, _ = capture(test_argv, "zshrs", case)
        lat = measure(case) if args.latency else None
        if ref_grid is None or test_grid is None:
            who = "zsh" if ref_grid is None else "zshrs"
            return ("FAIL", ref_grid, test_grid, None,
                    f"{who} never reached prompt", lat)
        diffs = diff_grids(ref_grid, test_grid)
        if not diffs:
            return "PASS", ref_grid, test_grid, [], "", lat
        for _ in range(max(0, confirm_runs)):
            r2, _ = capture(ref_argv, "zsh", case)
            t2, _ = capture(test_argv, "zshrs", case)
            if r2 is None or t2 is None:
                continue
            d2 = diff_grids(r2, t2)
            if not d2 or first_diff_cell(d2) != first_diff_cell(diffs):
                return ("FLAKY", ref_grid, test_grid, diffs,
                        f"{len(diffs)} rows differ, not reproducible", lat)
            ref_grid, test_grid, diffs = r2, t2, d2
        return ("FAIL", ref_grid, test_grid, diffs,
                f"{len(diffs)} rows differ", lat)

    if args.jobs > 1:
        from concurrent.futures import ThreadPoolExecutor
        _pool = ThreadPoolExecutor(max_workers=args.jobs)
        # `map` yields in submission order: the log stays deterministic no
        # matter which cell finishes first.
        verdicts = zip(cases, _pool.map(evaluate, cases))
    else:
        _pool = None
        verdicts = ((c, evaluate(c)) for c in cases)

    passed = failed = flaky = 0
    results = []
    book = (LatencyBook(args.latency_min_ms, args.latency_threshold,
                        args.latency_runs, concurrent_drain=False)
            if args.latency else None)
    for case, (status, ref_grid, test_grid, diffs, detail, lat) in verdicts:
        keyspec = "+".join(case.keys)
        if book is not None and lat:
            book.record(case.name, lat)
        record = {
            # `case.name` is already `<corpus case>.<sequence>` for the built-in
            # set, so it is a stable id across runs and machines.
            "id": case.name,
            "buffer": case.buffer,
            "keys": list(case.keys),
            "status": "PASS",
            "detail": "",
            "rows_differ": 0,
            "first_diff": None,
            "diff_rows": [],
        }
        results.append(record)
        if diffs is None:
            failed += 1
            record["status"] = "FAIL"
            record["detail"] = detail
            print(f"FAIL {case.name:16s} {case.buffer!r} [{keyspec}]  ({detail})")
            continue
        if status == "PASS":
            passed += 1
            lat_note = ""
            if book is not None:
                w = lat_worst(lat, args.latency_min_ms)
                lat_note = (f"  lat {w[0]:.2f}x (key #{w[1]} {w[2]!r} "
                            f"{w[3]:.0f}->{w[4]:.0f}ms)" if w
                            else f"  lat <{args.latency_min_ms:g}ms")
            print(f"PASS {case.name:16s} {case.buffer!r} [{keyspec}]{lat_note}")
            if args.verbose:
                print(render_grid(ref_grid))
        else:
            if status == "FLAKY":
                flaky += 1
            else:
                failed += 1
            row, a, b = diffs[0]
            col = next((i for i, (x, y) in enumerate(zip(a, b)) if x != y),
                       min(len(a), len(b)))
            record.update(status=status, detail=detail,
                          rows_differ=len(diffs),
                          first_diff={"row": row, "col": col, "ref": a, "test": b},
                          diff_rows=[{"row": i, "ref": x, "test": y}
                                     for i, x, y in diffs[:50]],
                          replay=replay_command(
                              args, case.buffer, list(case.keys),
                              Geom(args.rows, args.cols), zstyle_file or "/dev/null"))
            print(f"{status:4s} {case.name:16s} {case.buffer!r} [{keyspec}]  ({detail})")
            print("  --- zsh (ref) ---")
            print(render_grid(ref_grid))
            print("  --- zshrs (test) ---")
            print(render_grid(test_grid))
            print(f"  --- first divergence: row {row}, col {col} ---")
            print(f"  zsh  : {a}")
            print(f"  zshrs: {b}")
            print("  " + "-" * (col + 7) + "^")
            print("  --- row diffs ---")
            for i, x, y in diffs:
                print(f"  row {i:2d}: zsh  = {x!r}")
                print(f"          zshrs= {y!r}")
            print(f"  replay: {record['replay']}")
    if _pool is not None:
        _pool.shutdown(wait=True)

    print()
    print(f"# {passed} passed, {failed} failed, {flaky} flaky, {len(cases)} total"
          + (f"  ({failed + flaky} cells did not agree)" if failed + flaky else ""))
    lat_over = book.report() if book is not None else 0
    if args.json:
        doc = {
            "schema": "compsys-parity/1",
            "mode": "cases",
            "argv": sys.argv[1:],
            "zshrs": args.zshrs,
            "zsh": args.zsh,
            "dump": dump,
            "zstyle": zstyle_file,
            "geom": {"rows": args.rows, "cols": args.cols, "settle_ms": args.settle},
            "jobs": max(1, args.jobs),
            # `failed` counts every cell whose two shells did not agree, FLAKY
            # included: a nondeterministic divergence is still a divergence, and
            # downstream consumers (parity_matrix._collect) must never see a
            # smaller number because a failure was relabelled. `flaky` is
            # additional detail, not a subtraction.
            "summary": {"passed": passed, "failed": failed + flaky,
                        "flaky": flaky, "skipped": 0, "cells": len(cases)},
            # Separate key, never inside `summary`: parity_matrix and friends
            # read `summary`, and a latency finding is not a parity result.
            "latency": (book.json_doc() if book is not None else None),
            "results": results,
        }
        _write_json(args.json, doc)
    # Latency contributes to the EXIT CODE only, and only when the user asked
    # for a --latency-threshold. Every counter and every printed verdict above
    # is the correctness result, unchanged.
    return 1 if (failed or flaky or lat_over) else 0


if __name__ == "__main__":
    sys.exit(main())

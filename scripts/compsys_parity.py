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


def build_init_file(dump, fpath_dirs, zstyle_file, editing_mode=None):
    """Write the init script both shells source after launching with `-f`.
    Matches the spec: same fpath, same zstyles, same compinit + dump, so the
    only variable left is the shell under test.

    `editing_mode` (None by default, which emits nothing and leaves every
    pre-existing caller byte-identical) appends one of EDIT_MODES."""
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
    init = f"""\
# generated by compsys_parity.py — sourced into `zsh -f` and `zshrs --zsh -f`
PROMPT='{PROMPT_SENTINEL} '
RPROMPT=''
PS2=''
setopt no_beep
{ostype_line}\

{fpath_line}{zstyle_line}{compinit}{autoload_line}{mode_line}
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
               timings_out=None):
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

    try:
        for s in (ref, test):
            s.drain_settled(max_wait=3.0, first_wait=2.0)
            s.send(source_cmd)
            if not s.wait_for_prompt(timeout=25.0):
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
        for kn, key in enumerate(keys, 1):
            for s in (ref, test):
                s.send_key(key)
            # the FIRST completion keystroke is cold (autoload chain) → long
            # first-byte wait; later keys are warm menu redraws / filter edits.
            # Keyed on position within the KEY phase, not on the global step
            # index: with an edit program in front, the cold key is no longer
            # step 1 and it would otherwise get the warm (4s) window.
            fw = 8.0 if kn == 1 else 4.0
            drain_both(max_wait=12.0, first_wait=fw)
            if measuring:
                timings_out.append((key, ref.timing, test.timing))
            if compare(key):
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
class CellResult:
    cell: object
    status: str = "PASS"          # PASS | FAIL | FLAKY | SKIP
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


def replay_command(args, buffer, keys, geom, zstyle_path,
                   edits=None, editing_mode=None):
    """A copy-pasteable command that reproduces exactly this divergence."""
    extra = ""
    if edits is not None:
        extra += f" --edit-program {shlex.quote(edit_encode(edits))}"
    if editing_mode:
        extra += f" --editing-mode {editing_mode}"
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
    res = CellResult(cell=cell)

    def run(buffer, keys, init_file, geom, edits=None):
        # `edits=None` means "whatever the cell currently carries" — which is
        # the ORIGINAL program on the first run and the REDUCED one after
        # shrink_edits rewrote it. A default argument would have frozen the
        # original list object at def time and quietly re-run the long program
        # in every later probe.
        return run_keyseq(init_file, buffer, keys, args, env, geom,
                          edits=cell.edit_tokens if edits is None else edits)

    fail_step, records = run(cell.buffer, cell.keys, cell.init_file, cell.geom)
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
                                    cell.edit_mode)
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
            return build_init_file(dump, fpath_dirs, path, cell.edit_mode)

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
                                cell.edit_tokens, cell.edit_mode)
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

    def run(buffer, keys, init_file, geom, edits):
        return run_keyseq(init_file, buffer, keys, args, env, geom, edits=edits)

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
            expect=cell.target)
        return run_cell(leg_cell, args, env, dump, fpath_dirs, outdir)

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
                min_keys, build_init_file(dump, fpath_dirs, path, cell.edit_mode))

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


def build_cells(args, dump, fpath_dirs, statements, surfaces, outdir, skips,
                edit_pairs=None):
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
        init_file = build_init_file(dump, fpath_dirs, combo_path)
        # One init per editing mode, built once per combo and shared by every
        # cell of that combo (booting a shell is the expensive part; writing an
        # init file is not, but a fresh one per cell would multiply the
        # temp-dir churn for no benefit).
        mode_inits = {}

        def init_for(mode):
            if mode not in mode_inits:
                mode_inits[mode] = build_init_file(dump, fpath_dirs,
                                                   combo_path, mode)
            return mode_inits[mode]

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

    cells = build_cells(args, dump, fpath_dirs, statements, surfaces, outdir,
                        skips, edit_pairs)

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

    passed = failed = flaky = 0
    by_category: Counter = Counter()
    expect_bad: Counter = Counter()
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
            head = (f"{res.status:5s} combo {c.idx:3d} [{c.surface:18s}] "
                    f"{geom_str(c.geom):>7s} {c.buffer!r}")
            if c.edit_tokens:
                head += f" +{edit_program_str(c.edit_tokens)} ({c.edit_mode})"
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
          + (f"{skipped_cells} not compared, " if skipped_cells else "")
          + f"{len(cells)} cells run")
    if by_category and args.edit_fuzz:
        print("# per-category (generator[mode]):")
        cats = sorted({c for c, _ in by_category})
        for cat in cats:
            counts = {st: by_category[(cat, st)]
                      for st in ("PASS", "FAIL", "FLAKY", "SKIP")
                      if by_category[(cat, st)]}
            total = sum(counts.values())
            detail = "  ".join(f"{k}={v}" for k, v in counts.items())
            print(f"#   {cat:34s} {total:3d}  {detail}")
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
            "categories": {f"{cat}|{st}": n
                           for (cat, st), n in sorted(by_category.items())},
            "generator_sanity_mismatches": dict(expect_bad),
            "geom": {"rows": args.rows, "cols": args.cols, "settle_ms": args.settle},
            "jobs": max(1, args.jobs),
            "confirm": args.confirm,
            "shrink_probes": args.shrink_probes,
            "summary": {"passed": passed, "failed": failed, "flaky": flaky,
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
    return 1 if (failed or flaky or lat_over) else 0


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
    fail_step, records = run_keyseq(init_file, args.case, keys, args, env, geom,
                                    edits=edits)
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

    if args.random_combos > 0:
        return run_random_combos(args, dump, fpath_dirs,
                                 child_env(args.rows, args.cols))

    if args.lockstep:
        if args.case is None:
            sys.exit("compsys_parity: --lockstep needs --case")
        return run_lockstep_case(
            args,
            build_init_file(dump, fpath_dirs, zstyle_file, args.editing_mode),
            child_env(args.rows, args.cols))

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

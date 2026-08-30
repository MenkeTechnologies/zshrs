#!/usr/bin/env python3
"""comptab_parity.py — TAB-completion parity, run the way the shell is run.

Replaces the trust problems in compsys_parity.py:

  * Drives the NATIVE binary (`zshrs -f -i`) by default — the same path the
    user's daily shell takes (native `builtin_compinit`, SQLite/rkyv cache,
    `-C` worker-pool backfill). `--mode zsh` opts into `zshrs --zsh` for
    comparing the emulation path; it is NOT the default, because a green
    `--zsh` run says nothing about the binary that gets launched.
  * A divergence is a FAIL. Full stop. Re-running is only ever used to LABEL
    a failure `FLAKY`; it never converts one into a pass. Nondeterminism is a
    bug class (see the worker-pool tty race), not noise to average out.
  * A child that panics, dies, or never reaches a prompt is a FAIL with that
    reason — not a silently skipped case.
  * PS2 is left at its real `> ` value, so a shell that drops into a bogus
    continuation prompt shows it. (compsys_parity.py sets `PS2=''`, which
    renders that exact bug as an empty string.)
  * Both shells' raw PTY byte streams are scanned for panics and for
    diagnostics that appear on only one side.

What a FAIL prints, so it can be acted on without re-running anything:

    both grids, the (row, col) of the FIRST differing cell with a caret,
    every differing row, the cursor position on each side, rows that match as
    text but differ in SGR attributes, diagnostics only one shell emitted, any
    settle window that was truncated while output was still flowing, each
    child's exit status, a token-level diff of the two raw escape-sequence
    streams for the case interaction, and a copy-pasteable command that
    replays exactly that cell (geometry, fixture and dump included).

Signals that are REPORTED but do not by themselves fail a cell, because the
default verdict set is the text grid: SGR attribute divergence
(`--compare-attrs` promotes it to FAIL), cursor-position divergence
(`--strict-cursor`), and one-sided stream diagnostics (`--strict-stream`).
Each is counted in the summary and carried in `--json`, so the gap is visible
whether or not the flag is on.

Cases come from the shared corpus (`parity_corpus.CASES`), a corpus file (one
command-line prefix per line, `#` comments ignored), `--case`, or `--discover`
(every installed command on this host that ships a `_name` completer).

Four verdicts, and the difference between them is the whole point:

    PASS      both screens are byte-identical.
    FAIL      they are not — a divergence, including FLAKY (a FAIL that
              passed on a confirm run; nondeterminism is still a failure).
    TIMEOUT   at least one side ran out of MEASUREMENT budget (a settle
              window capped while bytes were still arriving, or one side
              alone produced nothing within the per-key budget), so the two
              screens were never both final and the comparison says nothing.
              Re-run once serially — with every other cell drained — before
              the label sticks; if that re-run diverges cleanly it is
              promoted to FAIL. Counted and printed separately, NEVER
              scored as a pass, and still a non-zero exit.
    SKIP      the case cannot run here at all (its command is not installed
              on this host, so neither shell can reach a completer and "both
              rendered nothing" is not evidence of parity). Counted and
              printed, never a pass. ON by default — `--no-skip-missing`
              scores those cells the old way, as passes.

TIMEOUT exists because ~80% of the failures in a --jobs 8..10 sweep were the
debug build missing the harness's per-key budget under load, not a divergence
(see the header of scripts/comptab_divergent_cases.txt). Folding those into
FAIL made every parallel sweep's number meaningless.

Every FAIL carries a FINGERPRINT: a hash over the invariant part of the
divergence (first-diff cell class, the two differing rows with digits/paths
masked, and the one-sided diagnostics). N cells hitting one bug collapse to one
fingerprint, and the summary reports "N cells, M distinct fingerprints" plus
the SMALLEST cell per fingerprint as its representative.

Usage:
    scripts/comptab_parity.py                       # shared corpus
    scripts/comptab_parity.py --corpus cases.txt    # your own list
    scripts/comptab_parity.py --case 'wget -'       # one ad-hoc case
    scripts/comptab_parity.py --keys tab,tab        # keystrokes per case
    scripts/comptab_parity.py --discover 200        # 200 host-discovered cases
    scripts/comptab_parity.py --json out.json       # machine-readable results
    scripts/comptab_parity.py --jobs 4              # 4 cells at a time
    scripts/comptab_parity.py -v                    # print grids for passes too
    scripts/comptab_parity.py --mode zsh            # emulation path instead
    scripts/comptab_parity.py --corpus-seed         # fill the fuzz corpus dir
    scripts/comptab_parity.py --mutate 20           # 20 MUTATED corpus inputs
    scripts/comptab_parity.py --style-fuzz 20       # 20 GENERATED zstyle configs
    scripts/comptab_parity.py --style-fuzz-list 30 # just show what it generates

Exit status: 0 only when every case is byte-identical (a TIMEOUT or a SKIP is
not byte-identical evidence, so neither one exits 0 either).
"""

from __future__ import annotations

import argparse
import contextlib
import difflib
import fcntl
import glob
import hashlib
import json
import os
import pty
import random
import re
import select
import shlex
import signal
import struct
import sys
import tempfile
import termios
import threading
import time

try:
    import pyte
except ImportError:
    sys.exit("comptab_parity: pyte not installed (pip install pyte)")


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
SELF = os.path.join("scripts", "comptab_parity.py")
SENTINEL = "@CT@"

# Keystrokes, cases and key SEQUENCES all live in parity_corpus so this
# harness and compsys_parity.py exercise the identical corpus — a case added
# for one is automatically run by both.
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from parity_corpus import (  # noqa: E402
    CASES,
    DEFAULT_SEQUENCES,
    KEY_SEQUENCES,
    UnknownKey,
    adhoc_case,
    cases_by_tag,
    discover_cases,
    key_bytes,
    random_subset,
    read_statements,
    shrink,
)

# Panic / abort signatures either shell may emit onto its pty.
CRASH_MARKERS = (
    "panicked at",
    "capacity overflow",
    # The panic FOOTER, not the bare variable name: the harness exports
    # RUST_BACKTRACE=1 into both children, so any completion listing that
    # enumerates the environment (`unset <TAB>`, `$parameters`, `$commands[`)
    # contains the literal string and was scored as a crash — on the
    # REFERENCE zsh, no less, which turned real passes into fake failures and
    # buried the genuine divergence on the same case.
    "run with `RUST_BACKTRACE",
    "Segmentation fault",
    "Abort trap",
    "fatal runtime error",
)

# Lines that mean "a shell complained". Matched against the raw pty text of
# each side with the escape sequences stripped, then set-differenced: a message
# BOTH shells print is not a divergence, one only zsh or only zshrs prints is.
# That comparison is what makes the patterns safe to keep broad — a completion
# listing that happens to contain the words appears on both sides and cancels.
DIAG_PATTERNS = tuple(re.compile(p, re.I) for p in (
    # zsh's own diagnostic shape. It is `funcname:lineno: message` for a
    # top-level shell function but nests one segment per frame when the error
    # comes from something the function called — the real emission that
    # motivated this was `_describe:compadd:114: bad option: -b`, which a
    # single-segment pattern did not match, so the harness printed 24 rows of
    # `<absent>` without ever naming the `compadd -b` gap that caused them.
    # FIRST in the tuple on purpose: the scan keeps the first pattern that
    # matches, and this one carries the calling frames that say WHERE the
    # message came from, which the bare message text does not.
    r"\b_[a-z_][a-z0-9_]*(?::[a-z_][a-z0-9_]*)*:\d+:.*",
    r"command not found\b.*",
    r"no such file or directory\b.*",
    r"permission denied\b.*",
    r"parse error\b.*",
    r"bad pattern\b.*",
    r"bad substitution\b.*",
    r"bad math expression\b.*",
    r"bad set of key/value pairs\b.*",
    r"unknown file attribute\b.*",
    r"not valid in this context\b.*",
    r"function definition file not found\b.*",
    r"invalid argument\b.*",
    r"bad option\b.*",
    r"unknown (?:option|module|signal)\b.*",
    r"no matches found\b.*",
    r"event not found\b.*",
    r"maximum nested function level reached\b.*",
    r"can't (?:open|find|read)\b.*",
))

_ANSI_RE = re.compile(r"\x1b\[[0-9;?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b[@-Z\\-_]")
_SHELLPREFIX_RE = re.compile(r"^(?:zsh|zshrs)(?:\s*\(\w+\))?:\s*")

# Settle outcomes. Which one ended a wait is a property of the MEASUREMENT, and
# a FAIL captured under `capped` (output still flowing) or a PASS captured under
# `no-output` (nothing ever rendered) is worth strictly less than one captured
# under `quiet` — so the harness records it instead of pretending all three are
# the same observation.
QUIET, NO_OUTPUT, CAPPED = "quiet", "no-output", "capped"

# `pty.fork()` forks a process that is running Python threads. The child execs
# immediately, but between fork and exec it must not touch a lock another
# thread held at fork time. Serialising the fork+exec pair keeps that window to
# one thread at a time; it costs nothing at --jobs 1.
_FORK_LOCK = threading.Lock()


class SerialGate:
    """Many cells at once, but drainable to exactly one.

    A budget-exhausted cell has to be re-measured with the machine quiet,
    otherwise the re-run inherits the same load that exhausted the budget and
    the verdict is no more trustworthy than the first attempt. `shared()` is
    what every normal cell holds; `exclusive()` blocks new cells from starting
    and waits for the ones in flight to finish before it runs. At --jobs 1 both
    are free.
    """

    def __init__(self):
        self._cv = threading.Condition()
        self._active = 0
        self._draining = False

    @contextlib.contextmanager
    def shared(self):
        with self._cv:
            while self._draining:
                self._cv.wait()
            self._active += 1
        try:
            yield
        finally:
            with self._cv:
                self._active -= 1
                self._cv.notify_all()

    @contextlib.contextmanager
    def exclusive(self):
        with self._cv:
            while self._draining:
                self._cv.wait()
            self._draining = True
            while self._active:
                self._cv.wait()
        try:
            yield
        finally:
            with self._cv:
                self._draining = False
                self._cv.notify_all()


# Persistent fuzz corpus: seed inputs and, more importantly, the minimal
# reproducer of every fingerprint the fuzzer has ever found. `--mutate` draws
# from here rather than re-rolling the dice from scratch, so a run starts from
# everything earlier runs learned.
CORPUS_DIR = os.path.join(REPO, "scripts", "parity_corpus_fuzz")
DIVERGENT_FILE = os.path.join(REPO, "scripts", "comptab_divergent_cases.txt")

# The case corpus now lives in parity_corpus.CASES (shared with
# compsys_parity.py). `--corpus FILE` still overrides it.


def resolve_dump(explicit):
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


def user_fpath():
    """The fpath `zsh -f` sees on this host — the set the dump was built from."""
    import subprocess
    try:
        out = subprocess.run(
            ["zsh", "-f", "-c", "print -rl -- $fpath"],
            capture_output=True, text=True, timeout=10,
        ).stdout
        return [d for d in out.splitlines() if d and os.path.isdir(d)]
    except Exception:
        return []


def build_init(dump, fpath_dirs, zstyle_file):
    d = tempfile.mkdtemp(prefix="comptab_parity_")
    fpath_line = ""
    if fpath_dirs:
        fpath_line = "fpath=( %s )\n" % " ".join(shlex.quote(p) for p in fpath_dirs)
    zstyle_line = ""
    if zstyle_file and os.path.exists(zstyle_file):
        zstyle_line = "source %s\n" % shlex.quote(zstyle_file)
    if dump:
        compinit = "autoload -Uz compinit\ncompinit -C -d %s\n" % shlex.quote(dump)
    else:
        compinit = "autoload -Uz compinit\ncompinit -u\n"
    # PS1 is a fixed sentinel so both grids anchor identically. PS2 keeps its
    # real value on purpose: a shell that wrongly renders a continuation
    # prompt must show it in the diff.
    init = f"""\
# generated by comptab_parity.py
PROMPT='{SENTINEL} '
RPROMPT=''
PS2='> '
setopt no_beep
{fpath_line}{zstyle_line}{compinit}
print -u2 ''
"""
    path = os.path.join(d, "init.zsh")
    with open(path, "w") as f:
        f.write(init)
    return path


def child_env():
    env = {
        "TERM": "xterm-256color",
        "LANG": "C",
        "LC_ALL": "C",
        "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
        # zshrs's autosuggest / syntax highlight have no zsh counterpart, so
        # their ghost text would diff on every case with a history hit. This
        # silences the fx LAYER only; the completion engine is untouched.
        "ZSHRS_NATIVE_ZLE_FX": "0",
        # Same principle, applied to the shell's own namespace: zshrs ships
        # ~145 builtins zsh does not have (peach, async, zf_*, dbview, ...).
        # Every listing that enumerates $builtins therefore diverges by
        # construction — `pr<TAB>` gained printenv/profile, `rustup <TAB>`'s
        # compctl fallback dumped 52080 names against zsh's 52017. Those are
        # deliberate zshrs FEATURES, not compat regressions, and flagging
        # them as parity failures buries the real ones. The flag hides the
        # extension names from the `builtins` table and the compctl
        # namespace dump for the duration of the comparison ONLY; dispatch
        # is untouched (`whence -w peach` still reports a builtin) and
        # nothing else about the shell changes.
        "ZSHRS_HIDE_EXT_BUILTINS": "1",
        "RUST_BACKTRACE": "1",
    }
    # The child env is built from scratch — nothing of the parent's leaks in
    # except what is listed here — and BOTH shells get the identical dict, so
    # the environment is not a variable in the comparison.
    #
    # HOME is deliberately the real one rather than a throwaway: `-f` already
    # guarantees no rc file is read on either side, the dump lives under the
    # user's HOME, and `cd ~/<TAB>` is only a meaningful case if it completes
    # against a real home. HISTFILE is deliberately NOT pinned either — a
    # `<UP>` on an empty line was measured to render nothing on both shells, so
    # there is no history state to isolate and inventing an isolation for it
    # would only add a difference from how the shell really starts.
    if "HOME" in os.environ:
        env["HOME"] = os.environ["HOME"]
    # Debug passthrough: the child env is built from scratch, so without this
    # `ZSHRS_LOG=debug scripts/comptab_parity.py ...` silently produced no
    # trace and a divergence could not be chased into the engine.
    for k in ("ZSHRS_LOG", "RUST_LOG"):
        if k in os.environ:
            env[k] = os.environ[k]
    return env


# ── raw pty stream analysis ──────────────────────────────────────────────────

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


def esc_bytes(b: bytes) -> str:
    """One token rendered readably: `\\e[K`, `\\r`, `cd /usr/`."""
    out = []
    for ch in b:
        if ch in _CTRL_NAMES:
            out.append(_CTRL_NAMES[ch])
        elif ch < 0x20:
            out.append("\\x%02x" % ch)
        else:
            out.append(chr(ch))
    return "".join(out)


def tokenize_stream(raw: bytes) -> list[str]:
    """Split a raw pty stream into escape sequences and printable runs.

    Diffing the streams line-by-line is useless (a redraw stream has almost no
    newlines) and byte-by-byte is unreadable. One token per escape sequence is
    the granularity the bug actually lives at: `zsh emitted \\e[5C where zshrs
    emitted \\e[K` is a sentence about the redraw path.
    """
    out, i, n = [], 0, len(raw)
    while i < n:
        m = _TOKEN_RE.match(raw, i)
        if m and m.end() > i:
            out.append(esc_bytes(m.group()))
            i = m.end()
        else:                       # unmatchable byte (e.g. stray 0x80-0xff)
            out.append(esc_bytes(raw[i:i + 1]))
            i += 1
    return out


def stream_diff(ref: bytes, test: bytes, max_lines: int = 40) -> list[str]:
    """Token-level unified diff of the two raw streams."""
    a, b = tokenize_stream(ref), tokenize_stream(test)
    sm = difflib.SequenceMatcher(None, a, b, autojunk=False)
    lines: list[str] = []
    for tag, i1, i2, j1, j2 in sm.get_opcodes():
        if tag == "equal":
            continue
        if len(lines) >= max_lines:
            lines.append("... (truncated; raise --raw-diff-lines)")
            break
        ctx = "".join(a[max(0, i1 - 3):i1])
        if ctx:
            lines.append("  after %s" % ctx)
        if tag in ("replace", "delete"):
            lines.append("    zsh  - %s" % "".join(a[i1:i2])[:200])
        if tag in ("replace", "insert"):
            lines.append("    zshrs+ %s" % "".join(b[j1:j2])[:200])
    return lines


def diagnostics(raw: bytes, pid: int | None) -> set[str]:
    """Complaint-shaped lines in one shell's raw output, normalised.

    The shell name prefix and this session's own pid are stripped so `zsh: no
    such file` and `zshrs: no such file` are the SAME message and cancel out —
    only a message one side emits and the other does not survives the set
    difference.
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


# ── one shell on one pty ─────────────────────────────────────────────────────

class Session:
    def __init__(self, argv, env, rows, cols, settle_ms):
        self.rows, self.cols = rows, cols
        self.settle = settle_ms / 1000.0
        self.screen = _TolerantScreen(cols, rows)
        self.stream = pyte.ByteStream(self.screen)
        self.raw = bytearray()
        self.mark = 0            # raw offset where the case interaction starts
        self.dead = False
        self.status = None       # child wait status once reaped
        self.events = []         # (phase, settle-outcome, first-byte s, waited s)
        with _FORK_LOCK:
            self.pid, self.fd = pty.fork()
            if self.pid == 0:
                # A failed exec must never fall through into the parent's code
                # — the child is a fork of a Python process midway through a
                # sweep, and returning from here would run the rest of the run
                # a second time.
                try:
                    os.execvpe(argv[0], argv, env)
                except BaseException as exc:  # pragma: no cover — child only
                    try:
                        os.write(2, ("exec failed: %s\n" % exc).encode())
                    finally:
                        os._exit(127)
        try:
            fcntl.ioctl(self.fd, termios.TIOCSWINSZ,
                        struct.pack("HHHH", rows, cols, 0, 0))
        except OSError:
            # Geometry is not optional — the completion column math depends on
            # it — so this is fatal for the cell. Reap the child first: raising
            # out of __init__ means no caller holds this Session and nothing
            # would ever close its pty or wait for the shell.
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
        """Read until the screen stops changing; report HOW the wait ended.

        Returns QUIET (a real quiet window elapsed — the only outcome that
        means "this screen is final"), NO_OUTPUT (nothing arrived at all
        within first_wait) or CAPPED (max_wait hit while bytes were still
        arriving, so the screen may be mid-render).
        """
        start = last = time.monotonic()
        seen = False
        # Time from entering the wait to the FIRST byte the child produced.
        # Without it a NO_OUTPUT verdict is indistinguishable between "the
        # shell had nothing to say" and "the shell was still computing when
        # the budget ran out" — which is the single most common triage
        # question on a discovered-corpus sweep. Purely additive: no wait
        # budget, outcome or comparison depends on it.
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

    def wait_prompt(self, timeout=30.0):
        """Wait for the prompt sentinel, against a WALL-CLOCK deadline.

        The previous loop added a flat 0.05 per iteration regardless of how
        long the iteration took. When the child was writing steadily, each
        `_read_once` returned as soon as data was ready — often in under a
        millisecond — so the counter reached `timeout` after a small fraction
        of that many seconds and a shell that was booting normally, just
        chattily, was declared "never reached a prompt". A monotonic deadline
        measures the thing the flag is named for.
        """
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
        return [self._mask_pid(r) for r in rows]

    def attrs(self, nrows):
        """Per-cell SGR attributes for the first `nrows` rows.

        `screen.display` throws every attribute away, so a listing drawn in the
        wrong colour, without the bold on the selected menu entry, or missing
        the `list-colors` SGR run compares byte-identical as text. This is the
        raw material for reporting that (see `--compare-attrs`).
        """
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

    def _mask_pid(self, row):
        """Replace THIS shell's own pid with a stable token.

        `$$` is the pid of the shell under test, so the reference and the
        candidate necessarily report different values -- two live processes
        cannot share a pid.  A case that displays it (`unset <TAB>` lists
        every parameter with its value, `$` among them) therefore can never
        compare equal no matter how correct zshrs is, and scored as a
        permanent failure on all six key sequences.

        Only the exact pid of this session's own child is substituted, taken
        from the fork in Session.__init__ -- not a general digit mask.  Any
        other number on the screen, including one that merely looks like a
        pid, still has to match byte for byte.
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
    """`waitpid` status rendered as text, or None when the child was killed by
    the harness's own teardown (which says nothing about the shell)."""
    if status is None:
        return None
    if os.WIFSIGNALED(status):
        sig = os.WTERMSIG(status)
        if sig in (signal.SIGHUP, signal.SIGTERM, signal.SIGKILL):
            return None                    # our own teardown
        try:
            name = signal.Signals(sig).name
        except ValueError:
            name = "?"
        return "killed by signal %d (%s)" % (sig, name)
    if os.WIFEXITED(status) and os.WEXITSTATUS(status) not in (0, 1):
        return "exited %d" % os.WEXITSTATUS(status)
    return None


# ── one shell's capture for one case ─────────────────────────────────────────

def _secs(v):
    return "none" if v is None else "%.2fs" % v


class Capture:
    def __init__(self, grid=None, reason=None, crash=None, attrs=None,
                 cursor=None, raw=b"", diags=None, events=(), status=None):
        self.grid = grid
        self.reason = reason      # None on success
        self.crash = crash or []
        self.attrs = attrs or []
        self.cursor = cursor
        self.raw = raw            # the case interaction only, not the boot
        self.diags = diags or set()
        self.events = list(events)
        self.status = status

    def warnings(self):
        """Reasons this capture is worth less than a clean one."""
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


def capture(argv, env, args, init_file, buf, keys):
    sess = Session(argv, env, args.rows, args.cols, args.settle)
    # Pre-seeded so an exception anywhere below still yields a Capture that
    # says what happened, instead of propagating and losing the whole cell.
    result = Capture(reason="capture aborted before any screen was taken")
    try:
        sess.settle_out(max_wait=4.0, first_wait=3.0, phase="boot")
        sess.send(("source %s\n" % shlex.quote(init_file)).encode())
        if not sess.wait_prompt(timeout=args.boot_timeout):
            result = Capture(
                reason="never reached a prompt (boot/compinit hang or crash)",
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
            result = Capture(
                grid=rows,
                attrs=sess.attrs(len(rows)),
                cursor=sess.cursor(),
                raw=bytes(sess.raw[sess.mark:]),
                diags=diagnostics(sess.raw[sess.mark:], sess.pid),
            )
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
        # The settle history and the child's exit status are only complete
        # after the reap, so both are attached here rather than lost with the
        # session.
        result.events = list(sess.events)
        result.status = sess.status
    return result


# ── comparison ───────────────────────────────────────────────────────────────

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
    """Column of the first differing character in two rows."""
    for i, (x, y) in enumerate(zip(a, b)):
        if x != y:
            return i
    return min(len(a), len(b))


def attr_diff_rows(ref, test):
    """Row indices whose cells differ in SGR attributes.

    Text-identical rows only — a row that already differs as text is reported
    by the text diff and adding it here would just be noise. Caveat: this is a
    column-aligned comparison, so a row where the `<PID>` mask replaced pids of
    different lengths shifts every later cell; that is why the signal is
    reported rather than scored by default.
    """
    out = []
    for i in range(min(len(ref), len(test))):
        if ref[i] != test[i]:
            out.append(i)
    return out


class Verdict:
    """One cell's outcome."""

    def __init__(self, case, seq, keys, status, detail, ref, test, diffs):
        self.case, self.seq, self.keys = case, seq, keys
        self.status, self.detail = status, detail
        self.ref, self.test, self.diffs = ref, test, diffs or []
        self.attr_rows = []
        self.cursor_differs = False
        self.ref_only_diags = set()
        self.test_only_diags = set()
        self.duration = 0.0
        # Why the measurement was not usable (TIMEOUT) / why the cell could not
        # run at all (SKIP). Both stay empty on a real PASS or FAIL.
        self.timeouts = []
        self.skip_reason = None
        self.recheck = None       # verdict of the serial TIMEOUT re-run
        # The zstyle statements this cell ran under, when it came from the fuzz
        # corpus rather than the fixture as a whole. Needed to write a
        # reproducer that actually reproduces.
        self.statements = None
        self.zstyle_file = None   # the fixture file those statements live in
        if ref and test and ref.grid is not None and test.grid is not None:
            text_diff = {i for i, _a, _b in self.diffs}
            self.attr_rows = [r for r in attr_diff_rows(ref.attrs, test.attrs)
                              if r not in text_diff]
            self.cursor_differs = ref.cursor != test.cursor
            self.ref_only_diags = ref.diags - test.diags
            self.test_only_diags = test.diags - ref.diags

    @property
    def id(self):
        return "%s.%s" % (self.case.name, self.seq)

    def side_signals(self):
        return bool(self.attr_rows) or self.cursor_differs or \
            bool(self.ref_only_diags) or bool(self.test_only_diags)

    @property
    def fingerprint(self):
        return fingerprint(self)

    def size(self):
        """How small this reproduction is — smaller is a better representative."""
        return (len(self.keys), len(self.case.buffer),
                len(self.statements or ()), self.case.buffer)


# ── failure fingerprinting ───────────────────────────────────────────────────
#
# Without this, a sweep that trips one bug on 40 commands prints 40 reports and
# the reader has to eyeball which ones are the same thing. The fingerprint is a
# hash over the parts of a divergence that do NOT change from command to
# command: where the first differing cell sits, what the two rows look like
# once the command-specific detail (digits, paths, pids) is masked out, and
# which diagnostics only one shell emitted.
#
# It is deliberately computed only for FAIL / FLAKY. A TIMEOUT is not evidence
# of a divergence, so fingerprinting one would invent a bug out of a
# measurement failure, and grouping by it would let a slow cell masquerade as a
# known bug.

_FP_PATH_RE = re.compile(r"(?:/[\w.+@%~-]+){2,}/?")
_FP_HEX_RE = re.compile(r"\b(?:0x)?[0-9a-f]{6,}\b", re.I)
_FP_NUM_RE = re.compile(r"\d+")


def fp_normalize(text):
    """One row/message stripped of everything case-specific."""
    t = text.replace(SENTINEL, "$").replace("<PID>", "#")
    t = _FP_PATH_RE.sub("<path>", t)
    t = _FP_HEX_RE.sub("<hex>", t)
    t = _FP_NUM_RE.sub("#", t)
    return re.sub(r"\s+", " ", t).strip()


def fp_row_class(row):
    """Coarse class of a row index.

    The exact row a divergence lands on moves with the number of matches the
    command happens to have, so it is not part of the bug. Whether it is the
    command line itself (row 0), the first listing rows, or deep in the listing
    is.
    """
    if row == 0:
        return "row0"
    if row <= 3:
        return "rowtop"
    return "rowlist"


def fp_parts(v):
    """The invariant components of a divergence, or None if it is not one."""
    if v.status not in ("FAIL", "FLAKY"):
        return None
    parts = []
    for side, cap in (("zsh", v.ref), ("zshrs", v.test)):
        if cap is not None and cap.reason:
            parts.append("reason/%s/%s" % (side, fp_normalize(cap.reason)))
    if v.diffs:
        row, a, b = v.diffs[0]
        col = first_diff_cell(a, b)
        parts.append("cell/%s/col%d" % (fp_row_class(row), col // 10))
        parts.append("ref/%s" % fp_normalize(a)[:160])
        parts.append("test/%s" % fp_normalize(b)[:160])
    elif not parts:
        # A strict-flag failure (attrs / cursor / stream) has no row diff; the
        # detail line is the only description of it there is.
        parts.append("signal/%s" % fp_normalize(v.detail))
    for m in sorted(v.ref_only_diags):
        parts.append("diag-zsh/%s" % fp_normalize(m))
    for m in sorted(v.test_only_diags):
        parts.append("diag-zshrs/%s" % fp_normalize(m))
    return parts


def fingerprint(v):
    parts = fp_parts(v)
    if parts is None:
        return None
    return hashlib.sha1("\n".join(parts).encode()).hexdigest()[:10]


def fp_label(v):
    """A one-line human name for the fingerprint's bug shape."""
    parts = fp_parts(v) or []
    return " | ".join(parts[:3])[:200]


def group_by_fingerprint(failures):
    """{fingerprint: [verdicts]}, insertion-ordered by first sighting."""
    groups = {}
    for v in failures:
        groups.setdefault(v.fingerprint, []).append(v)
    return groups


def render(rows):
    return "\n".join("  %2d| %s" % (i, r) for i, r in enumerate(rows)) or "  <empty>"


def repro_cmd(args, buf, keys, zstyle=None):
    """A command line that replays exactly this cell.

    `zstyle` overrides `args.zstyle` for a cell that ran under a generated
    statement subset (a random combo, a fuzz-corpus input) rather than the
    fixture as a whole — pasting the command has to reproduce THAT config.

    The old version printed only `--case` and `--keys`, which silently dropped
    the geometry, the zstyle fixture, the dump and the mode — so pasting it
    reproduced a DIFFERENT cell whenever the run was not using every default.
    """
    cmd = [SELF]
    if args.mode != "native":
        cmd += ["--mode", args.mode]
    if args.zshrs != os.path.join(REPO, "target", "debug", "zshrs"):
        cmd += ["--zshrs", shlex.quote(args.zshrs)]
    if args.zsh != "zsh":
        cmd += ["--zsh", shlex.quote(args.zsh)]
    if args.no_dump:
        cmd += ["--no-dump"]
    elif args.dump:
        cmd += ["--dump", shlex.quote(args.dump)]
    fixture = zstyle if zstyle is not None else args.zstyle
    if fixture:
        cmd += ["--zstyle", shlex.quote(fixture)]
    cmd += ["--case", shlex.quote(buf), "--keys", ",".join(keys)]
    cmd += ["--rows", str(args.rows), "--cols", str(args.cols)]
    if args.settle != 300:
        cmd += ["--settle", str(args.settle)]
    return " ".join(cmd)


def print_failure(v, args):
    """Everything needed to act on a FAIL without re-running it."""
    ref, test = v.ref, v.test
    print("  --- fingerprint %s ---" % v.fingerprint)
    print("  " + fp_label(v))
    print("  --- zsh (ref) ---")
    print(render(ref.grid or []))
    print("  --- zshrs ---")
    print(render(test.grid or []))

    if v.diffs:
        row, a, b = v.diffs[0]
        col = first_diff_cell(a, b)
        print("  --- first divergence: row %d, col %d ---" % (row, col))
        print("  zsh  : %s" % a)
        print("  zshrs: %s" % b)
        print("  %s^" % ("-" * (col + 7)))
        print("  --- row diffs (%d) ---" % len(v.diffs))
        for i, x, y in v.diffs[:args.max_diff_rows]:
            print("  row %2d: zsh  = %r" % (i, x))
            print("          zshrs= %r" % (y,))
        if len(v.diffs) > args.max_diff_rows:
            print("  ... %d more row(s)" % (len(v.diffs) - args.max_diff_rows))

    if ref.cursor and test.cursor and ref.cursor != test.cursor:
        print("  --- cursor ---")
        print("  zsh  : row %d col %d      zshrs: row %d col %d"
              % (ref.cursor[0], ref.cursor[1], test.cursor[0], test.cursor[1]))
    if v.attr_rows:
        print("  --- style-only rows (identical text, different SGR) ---")
        print("  rows: %s" % ", ".join(str(r) for r in v.attr_rows[:20]))
    if v.ref_only_diags or v.test_only_diags:
        print("  --- one-sided diagnostics ---")
        for m in sorted(v.ref_only_diags)[:10]:
            print("  only zsh  : %s" % m)
        for m in sorted(v.test_only_diags)[:10]:
            print("  only zshrs: %s" % m)
    warn = [("zsh", w) for w in ref.warnings()] + [("zshrs", w) for w in test.warnings()]
    if warn:
        print("  --- capture warnings ---")
        for who, w in warn:
            print("  %-5s: %s" % (who, w))
    if args.raw_diff and ref.raw and test.raw:
        lines = stream_diff(ref.raw, test.raw, args.raw_diff_lines)
        if lines:
            print("  --- raw stream diff (case interaction only) ---")
            for line in lines:
                print("  " + line)
    print("  --- repro ---")
    print("  " + repro_cmd(args, v.case.buffer, v.keys, zstyle=v.zstyle_file))
    print()


def to_json(v):
    ref, test = v.ref, v.test
    def side(c):
        if c is None:
            return None
        return {
            "reason": c.reason,
            "crash": c.crash,
            "cursor": list(c.cursor) if c.cursor else None,
            "rows": len(c.grid) if c.grid is not None else None,
            "warnings": c.warnings(),
            "diagnostics": sorted(c.diags),
            # Per-wait timing, so a sweep's JSON can separate "answered
            # differently" from "answered too late to be captured".
            "phases": [{"phase": p, "outcome": o,
                        "first_byte": (round(f, 3) if f is not None else None),
                        "waited": round(w, 3)}
                       for p, o, f, w in c.events],
        }
    first = None
    if v.diffs:
        row, a, b = v.diffs[0]
        first = {"row": row, "col": first_diff_cell(a, b), "ref": a, "test": b}
    return {
        "id": v.id,
        "case": v.case.name,
        "buffer": v.case.buffer,
        "tags": list(v.case.tags),
        "sequence": v.seq,
        "keys": v.keys,
        "status": v.status,
        "detail": v.detail,
        # None unless the status is FAIL/FLAKY: a TIMEOUT is a measurement
        # failure, not a bug shape, and must never be grouped as one.
        "fingerprint": v.fingerprint,
        "fingerprint_label": fp_label(v) if v.fingerprint else None,
        "timeouts": list(v.timeouts),
        "timeout_recheck": v.recheck,
        "skip_reason": v.skip_reason,
        "statements": list(v.statements) if v.statements is not None else None,
        "rows_differ": len(v.diffs),
        "first_diff": first,
        "diff_rows": [{"row": i, "ref": a, "test": b} for i, a, b in v.diffs[:50]],
        "attr_only_rows": v.attr_rows,
        "cursor_differs": v.cursor_differs,
        "diagnostics_only_ref": sorted(v.ref_only_diags),
        "diagnostics_only_test": sorted(v.test_only_diags),
        "ref": side(ref),
        "test": side(test),
        # Volatile — excluded from a `jq 'del(.results[].timing)'` comparison
        # between two commits' runs.
        "timing": {"seconds": round(v.duration, 2)},
    }


# ── running one cell ─────────────────────────────────────────────────────────

def timeout_reasons(ref, test):
    """Ways this cell's MEASUREMENT ran out of budget, not ways it diverged.

    Three shapes count, and only three:

      * a side never reached a prompt inside --boot-timeout;
      * a settle window was CAPPED — bytes were still arriving when the wait
        ended, so that screen was never final and diffing it is diffing a
        half-drawn frame;
      * a side produced NOTHING within the per-key budget while the OTHER side
        produced something. One-sidedness is the whole test here: a key that
        legitimately draws nothing produces NO_OUTPUT on both shells, and
        calling that a timeout would bury a real divergence that came from an
        earlier key in the sequence.

    Everything else — a crash, a shell that exited, a harness error — stays a
    FAIL.
    """
    out = []
    for side, cap, other in (("zsh", ref, test), ("zshrs", test, ref)):
        if cap is None:
            continue
        if cap.reason and "never reached a prompt" in cap.reason:
            out.append("%s never reached a prompt (boot budget)" % side)
        others = {p: o for p, o, _f, _w in (other.events if other else [])}
        for phase, outcome, _first, waited in cap.events:
            if outcome == CAPPED:
                out.append("%s: settle capped after %s at %.1fs — still rendering"
                           % (side, phase, waited))
            elif (outcome == NO_OUTPUT and phase.startswith("key ")
                  and others.get(phase) != NO_OUTPUT):
                out.append("%s: nothing at all after %s within %.1fs"
                           % (side, phase, waited))
    return out


def run_case(args, env, init_file, case, seq_name, keys):
    """Returns a Verdict whose status is PASS / FAIL / FLAKY / TIMEOUT.

    FLAKY is a FAIL that did not reproduce on the confirm run — reported as a
    failure with the nondeterminism called out, never scored as a pass.

    TIMEOUT is a non-PASS where at least one side ran out of measurement budget
    (see `timeout_reasons`). It is NOT a pass and NOT a divergence; the caller
    re-runs it serially before the label sticks.
    """
    t0 = time.monotonic()
    buf = case.buffer
    ref = capture([args.zsh, "-f", "-i"], env, args, init_file, buf, keys)
    test = capture(args.test_argv, env, args, init_file, buf, keys)

    def strict_extra(r, t):
        """Signals that are reported always and fail only when asked for.

        Applied inside `judge` so the confirm re-run is judged by exactly the
        same rule as the first attempt — otherwise a cell failed on, say, a
        cursor divergence would be re-judged on text alone, pass, and get
        mislabelled FLAKY.
        """
        if r.grid is None or t.grid is None:
            return []
        out = []
        if args.compare_attrs:
            text_diff = {i for i, _a, _b in diff_grids(r.grid, t.grid)}
            rows = [i for i in attr_diff_rows(r.attrs, t.attrs) if i not in text_diff]
            if rows:
                out.append("%d row(s) differ in SGR attributes only" % len(rows))
        if args.strict_cursor and r.cursor != t.cursor:
            out.append("cursor %s vs %s" % (r.cursor, t.cursor))
        if args.strict_stream and (r.diags ^ t.diags):
            out.append("one-sided diagnostics")
        return out

    def judge(r, t):
        if r.reason:
            return "FAIL", "reference zsh: %s" % r.reason, None
        if t.reason:
            return "FAIL", "zshrs: %s" % t.reason, None
        d = diff_grids(r.grid, t.grid)
        if d:
            return "FAIL", "%d row(s) differ" % len(d), d
        extra = strict_extra(r, t)
        if extra:
            return "FAIL", "; ".join(extra), []
        return "PASS", "", []

    status, detail, diffs = judge(ref, test)
    v = Verdict(case, seq_name, keys, status, detail, ref, test, diffs)

    if v.status != "PASS":
        v.timeouts = timeout_reasons(ref, test)
    if v.timeouts:
        # Do not spend confirm runs on it and do not fingerprint it: the screens
        # were never both final, so there is nothing here to characterise. The
        # caller re-measures it with the machine drained.
        v.status = "TIMEOUT"
        v.detail = "budget exhausted, not a divergence: %s" % "; ".join(v.timeouts[:3])
    elif v.status != "PASS" and args.confirm > 0:
        # Confirm ONLY to label nondeterminism. A pass on re-run means the case
        # is flaky, which is still a failure.
        for _ in range(args.confirm):
            r2 = capture([args.zsh, "-f", "-i"], env, args, init_file, buf, keys)
            t2 = capture(args.test_argv, env, args, init_file, buf, keys)
            if judge(r2, t2)[0] == "PASS":
                v.status = "FLAKY"
                v.detail += " (passed on re-run — nondeterministic)"
                break
    v.duration = time.monotonic() - t0
    return v


class Cell:
    """One unit of work: a case, a key sequence, and the init it runs under."""

    def __init__(self, case, seq, keys, init_file, statements=None,
                 zstyle_file=None, origin=None):
        self.case, self.seq, self.keys = case, seq, keys
        self.init_file = init_file
        self.statements = statements
        self.zstyle_file = zstyle_file
        self.origin = origin


_WHENCE_CACHE = {}


def command_exists(word):
    """Is `word` runnable on this host — as a binary, builtin, function or alias?

    `shutil.which` alone answers only the first, so `cd `/`unset ` would be
    reported missing and skipped. The zsh fallback runs once per distinct word
    and is cached.
    """
    if word in _WHENCE_CACHE:
        return _WHENCE_CACHE[word]
    import shutil
    import subprocess
    ok = shutil.which(word) is not None
    if not ok:
        try:
            ok = subprocess.run(["zsh", "-f", "-c", "whence -w -- %s" % shlex.quote(word)],
                                capture_output=True, timeout=10).returncode == 0
        except Exception:
            ok = False
    _WHENCE_CACHE[word] = ok
    return ok


def skip_reason(buffer):
    """Why this buffer cannot say anything on this host, or None.

    ONLY applies to a buffer that has already COMMITTED to a command — i.e.
    something followed by whitespace. `gi<TAB>` is a perfectly good case even
    though no `gi` is installed (it completes command NAMES); `ansible-galaxy
    <TAB>` on a host without ansible is not, because neither shell can reach a
    completer that does not exist and "both printed nothing" is not evidence of
    parity. Mined corpora travel between machines, so this is the difference
    between a corpus that degrades gracefully and one that reports fake passes.
    """
    parts = buffer.split()
    if not parts or not buffer[:1].strip():
        return None
    if buffer == buffer.strip():            # still typing the first word
        return None
    word = parts[0]
    if "/" in word or "=" in word or word.startswith("$"):
        return None
    if command_exists(word):
        return None
    return "command %r is not installed on this host" % word


def run_cell(args, env, cell, gate):
    """One cell, including the serial re-measurement of a budget-exhausted one."""
    reason = skip_reason(cell.case.buffer) if args.skip_missing else None
    if reason:
        v = Verdict(cell.case, cell.seq, cell.keys, "SKIP", reason, None, None, [])
        v.skip_reason = reason
        v.statements = cell.statements
        v.zstyle_file = cell.zstyle_file
        return v

    with gate.shared():
        v = run_case(args, env, cell.init_file, cell.case, cell.seq, cell.keys)
    v.statements = cell.statements
    v.zstyle_file = cell.zstyle_file

    if v.status == "TIMEOUT" and args.timeout_recheck:
        # Drain every other cell first: re-running under the same load that
        # blew the budget measures the load again, not the shell.
        with gate.exclusive():
            v2 = run_case(args, env, cell.init_file, cell.case, cell.seq, cell.keys)
        v2.statements = cell.statements
        v2.zstyle_file = cell.zstyle_file
        if v2.status in ("FAIL", "FLAKY"):
            v2.detail += (" (first attempt was budget-exhausted; this is the "
                          "serial re-run, which diverged cleanly)")
            v2.recheck = "promoted-to-fail"
            return v2
        v.recheck = v2.status
        v.detail += ("; serial re-run: %s — still NOT scored as a pass, the "
                     "first measurement was never valid" % v2.status)
    return v


def cell_stream(args, env, cells, on_done=None):
    """Yield a Verdict per cell, in submission order."""
    gate = SerialGate()

    def work(cell):
        v = run_cell(args, env, cell, gate)
        if on_done:
            on_done(v)
        return v

    if args.jobs > 1:
        from concurrent.futures import ThreadPoolExecutor
        pool = ThreadPoolExecutor(max_workers=args.jobs)
        try:
            # `map` yields in submission order, so the log stays deterministic
            # no matter which cell finishes first.
            yield from pool.map(work, cells)
        finally:
            pool.shutdown(wait=True)
    else:
        for cell in cells:
            yield work(cell)


def print_timeout(v, args):
    """A TIMEOUT is not a bug report, so it does not get one — but it does get
    everything needed to decide whether to chase it."""
    print("  --- budget exhausted (no verdict on parity) ---")
    for r in v.timeouts:
        print("  ! %s" % r)
    if v.recheck:
        print("  ! serial re-run: %s" % v.recheck)
    for who, cap in (("zsh", v.ref), ("zshrs", v.test)):
        if cap is None:
            continue
        for w in cap.warnings():
            print("  ~ %-5s: %s" % (who, w))
    print("  --- repro ---")
    print("  " + repro_cmd(args, v.case.buffer, v.keys, zstyle=v.zstyle_file))
    print()


def print_fingerprint_groups(failures, args, prefix="# "):
    """The summary that turns N reports into M bugs."""
    groups = group_by_fingerprint(failures)
    print("%s%d failing cell(s), %d distinct fingerprint(s)"
          % (prefix, len(failures), len(groups)))
    for fp, vs in groups.items():
        rep = min(vs, key=lambda v: v.size())
        print("%s  %s  x%-3d smallest: --case %s --keys %s"
              % (prefix, fp, len(vs), shlex.quote(rep.case.buffer), ",".join(rep.keys)))
        print("%s     %s" % (prefix, fp_label(rep)))
        if len(vs) > 1:
            print("%s     also: %s" % (prefix, ", ".join(v.id for v in vs[1:9])))
    return groups


def run_random_combos(args, env, dump, fpath_dirs):
    """Fuzz RANDOM SUBSETS of the zstyle fixture.

    Every `zstyle` line is independent, so any subset is a valid config — and
    the bar is that any of them renders byte-identically, not just the curated
    axes in scripts/parity_combos/. Each combo is reproducible from (seed,
    index); a diverging combo is then SHRUNK to the minimal set of statements
    that still diverges, because "these 97 styles disagree" is not actionable
    and "these two do" is.
    """
    import random

    statements = read_statements(args.zstyle)
    cases = [c for c in cases_by_tag(args.tag)
             if not (args.skip_optional and "optional" in c.tags)]
    if args.combo_cases:
        want = {c.strip() for c in args.combo_cases.split(",")}
        cases = [c for c in cases if c.name in want or c.buffer in want]
    seq = args.combo_sequence
    keys = KEY_SEQUENCES[seq]

    outdir = os.path.join(REPO, "target", f"parity-combos-{args.seed}")
    os.makedirs(outdir, exist_ok=True)

    print("# random-combo fuzz")
    print("# fixture : %s (%d statements)" % (args.zstyle, len(statements)))
    print("# combos  : %d   keep-prob=%.2f   seed=%d" %
          (args.random_combos, args.combo_keep, args.seed))
    print("# cases   : %d   sequence=%s (%s)" % (len(cases), seq, "+".join(keys)))
    print("# outdir  : %s" % outdir)
    print()

    def init_for(subset):
        path = os.path.join(outdir, "subset.zsh")
        with open(path, "w") as f:
            f.write("\n".join(subset) + "\n")
        return build_init(dump, fpath_dirs, path)

    def diverges(subset, only=None):
        """The first case that does NOT pass under this subset, with its verdict.

        A TIMEOUT is returned too — it is not a pass — but the caller has to
        tell the two apart, because delta-debugging a cell whose screens were
        never final would shrink towards whichever subset happened to be slow.
        """
        init = init_for(subset)
        for case in (only or cases):
            v = run_case(args, env, init, case, seq, keys)
            if v.status != "PASS":
                return case, v
        return None, None

    bad = 0
    timed_out = 0
    for n in range(args.random_combos):
        rng = random.Random((args.seed << 20) ^ n)
        subset = random_subset(statements, args.combo_keep, rng)
        culprit, verdict = diverges(subset)
        if culprit is None:
            print("PASS combo %-4d (%3d statements)" % (n, len(subset)))
            sys.stdout.flush()
            continue

        if verdict.status == "TIMEOUT":
            timed_out += 1
            print("TIMEOUT combo %-4d (%3d statements) on %r — %s"
                  % (n, len(subset), culprit.buffer,
                     verdict.timeouts[0] if verdict.timeouts else "?"))
            print("     no verdict on parity; not shrunk (a shrink would chase "
                  "the slow subset, not a bug). Re-run at --jobs 1.")
            sys.stdout.flush()
            continue

        bad += 1
        # This exact line shape is parsed by gen_compsys_parity_report.py
        # (RC_FAIL_RE, which takes everything after `on ` as the case) — the
        # fingerprint goes on its own line rather than suffixed onto it.
        print("FAIL combo %-4d (%3d statements) on %r"
              % (n, len(subset), culprit.buffer))
        print("     fingerprint: %s" % verdict.fingerprint)
        sys.stdout.flush()

        # Does it still diverge with NO styles at all? If so the combo is
        # irrelevant — the case diverges under compsys defaults — and shrinking
        # would misleadingly name whichever statement survived last.
        if diverges([], only=[culprit])[0] is not None:
            print("     config-INDEPENDENT: diverges with zero zstyles set")
            sys.stdout.flush()
            continue

        minimal = subset
        if args.shrink:
            minimal = shrink(
                subset,
                lambda sub: diverges(sub, only=[culprit])[0] is not None,
                max_probes=args.shrink_probes,
            )
        path = os.path.join(outdir, "combo_%d_%d.zsh" % (args.seed, n))
        with open(path, "w") as f:
            f.write("# combo %d (seed %d) — diverges on %r with keys %s\n"
                    % (n, args.seed, culprit.buffer, "+".join(keys)))
            f.write("\n".join(minimal) + "\n")
        print("     minimal set: %d statement(s) -> %s" % (len(minimal), path))
        for s in minimal[:10]:
            print("       %s" % s)
        sys.stdout.flush()

    print("\n# %d/%d combo(s) diverged" % (bad, args.random_combos))
    if timed_out:
        print("# %d/%d combo(s) ran out of measurement budget — no parity verdict "
              "either way" % (timed_out, args.random_combos))
    return 1 if (bad or timed_out) else 0


# ── persistent fuzz corpus ───────────────────────────────────────────────────
#
# A one-shot random sweep re-rolls the same dice every run: it cannot get
# better at finding bugs, and everything it found is gone when the process
# exits. The corpus fixes both. It is a directory of small JSON files, each one
# a complete input — buffer, key sequence, zstyle statements — and `--mutate`
# derives new inputs from what is already in there. When a run finds a
# fingerprint the corpus has never seen, the minimal reproducer is written back
# in, so the NEXT run starts from it and mutates around a known-bad region
# instead of around the seeds.

class FuzzInput:
    """One complete fuzz input."""

    def __init__(self, buffer, keys, statements=(), origin="", fingerprint=None,
                 path=None, note=""):
        self.buffer = buffer
        self.keys = list(keys)
        self.statements = list(statements)
        self.origin = origin
        self.fingerprint = fingerprint
        self.path = path
        self.note = note

    def key(self):
        return (self.buffer, tuple(self.keys), tuple(self.statements))

    def stem(self, prefix):
        h = hashlib.sha1(repr(self.key()).encode()).hexdigest()[:10]
        return "%s_%s" % (prefix, h)

    def to_dict(self):
        return {
            "buffer": self.buffer,
            "keys": self.keys,
            "statements": self.statements,
            "origin": self.origin,
            "fingerprint": self.fingerprint,
            "note": self.note,
        }


def corpus_load(d):
    out = []
    if not os.path.isdir(d):
        return out
    for name in sorted(os.listdir(d)):
        if not name.endswith(".json"):
            continue
        path = os.path.join(d, name)
        try:
            with open(path) as f:
                obj = json.load(f)
        except (OSError, ValueError) as exc:
            print("# corpus: skipping %s (%s)" % (name, exc))
            continue
        keys = obj.get("keys") or ["tab"]
        out.append(FuzzInput(obj.get("buffer", ""), keys,
                             obj.get("statements") or [],
                             obj.get("origin", ""), obj.get("fingerprint"),
                             path, obj.get("note", "")))
    return out


def corpus_write(d, inp, prefix):
    os.makedirs(d, exist_ok=True)
    path = os.path.join(d, inp.stem(prefix) + ".json")
    with open(path, "w") as f:
        json.dump(inp.to_dict(), f, indent=2, sort_keys=True)
        f.write("\n")
    inp.path = path
    return path


def corpus_seed(args, statements):
    """Populate the corpus from the shared corpus and the mined divergences.

    Both sources are already curated: `parity_corpus.CASES` is the hand-written
    floor every machine reproduces, and `comptab_divergent_cases.txt` is 29
    buffers that were each CONFIRMED SERIALLY to diverge. Starting a mutation
    run from those is starting next to known-interesting territory.
    """
    seqs = [s for s in args.seed_sequences.split(",") if s.strip()]
    unknown = [s for s in seqs if s not in KEY_SEQUENCES]
    if unknown:
        sys.exit("--seed-sequences names unknown sequence(s): %s" % ", ".join(unknown))
    inputs = []
    for case in CASES:
        for s in seqs:
            inputs.append(FuzzInput(case.buffer, KEY_SEQUENCES[s], statements,
                                    origin="CASES/%s/%s" % (case.name, s),
                                    note=case.note))
    if os.path.exists(DIVERGENT_FILE):
        with open(DIVERGENT_FILE) as f:
            for line in f:
                buf = line.rstrip("\n")
                if not buf.strip() or buf.lstrip().startswith("#"):
                    continue
                inputs.append(FuzzInput(buf, KEY_SEQUENCES["tab1"], statements,
                                        origin="divergent-cases/tab1",
                                        note="mined, serially confirmed"))
    have = {i.key() for i in corpus_load(args.corpus_dir)}
    written = 0
    for inp in inputs:
        if inp.key() in have:
            continue
        corpus_write(args.corpus_dir, inp, "seed")
        have.add(inp.key())
        written += 1
    return inputs, written


# ── mutation ─────────────────────────────────────────────────────────────────
#
# Every mutator takes (buffer, keys, statements) and returns a new triple. They
# are deliberately SMALL edits: the corpus entry already reaches an interesting
# part of the completion system, and the point is to step just off it — one
# character further into a word, one key swapped for its cousin, one zstyle
# dropped — not to land somewhere unrelated, which is what sampling from
# scratch does.

# Keys that address the same operation by another route, or the obvious
# neighbour. A completion bug that only shows on `ctrl-n` and not `down` is a
# widget-binding bug; one that shows on both is an engine bug.
KEY_NEIGHBOURS = {
    "tab": ("btab", "ctrl-d"),
    "btab": ("tab",),
    "ctrl-d": ("tab", "btab"),
    "down": ("ctrl-n", "up"),
    "up": ("ctrl-p", "down"),
    "right": ("ctrl-f", "left"),
    "left": ("ctrl-b", "right"),
    "ctrl-n": ("down", "ctrl-p"),
    "ctrl-p": ("up", "ctrl-n"),
    "ctrl-f": ("right",),
    "ctrl-b": ("left",),
    "cr": ("ctrl-g", "esc"),
    "esc": ("ctrl-g", "cr"),
    "ctrl-g": ("esc", "cr"),
    "bs": ("ctrl-h", "ctrl-w"),
    "ctrl-h": ("bs",),
    "home": ("ctrl-a", "end"),
    "end": ("ctrl-e", "home"),
    "pgdn": ("pgup", "down"),
    "pgup": ("pgdn", "up"),
}

# What a menuselect filter or a partial word is typed out of.
FILTER_CHARS = "abcdefgilmnoprstuvxz-_./"


def _trailing_word(buf):
    i = len(buf)
    while i and not buf[i - 1].isspace():
        i -= 1
    return buf[:i], buf[i:]


def _mut_truncate(buf, keys, stmts, rng):
    """Drop trailing characters — the shorter buffer is often the real bug."""
    if not buf:
        return buf, keys, stmts
    return buf[:-rng.randint(1, min(3, len(buf)))], keys, stmts


def _mut_extend(buf, keys, stmts, rng):
    return buf + rng.choice(FILTER_CHARS), keys, stmts


def _mut_swap_key(buf, keys, stmts, rng):
    idx = [i for i, k in enumerate(keys) if k in KEY_NEIGHBOURS]
    if not idx:
        return buf, keys, stmts
    i = rng.choice(idx)
    out = list(keys)
    out[i] = rng.choice(KEY_NEIGHBOURS[keys[i]])
    return buf, out, stmts


def _mut_add_filter_key(buf, keys, stmts, rng):
    """Append a single-character key — how the interactive menu filter is typed."""
    return buf, list(keys) + [rng.choice(FILTER_CHARS)], stmts


def _mut_add_nav_key(buf, keys, stmts, rng):
    return buf, list(keys) + [rng.choice(
        ("tab", "btab", "down", "up", "left", "right", "ctrl-g", "cr", "bs"))], stmts


def _mut_drop_key(buf, keys, stmts, rng):
    if len(keys) <= 1:
        return buf, keys, stmts
    i = rng.randrange(len(keys))
    return buf, keys[:i] + keys[i + 1:], stmts


def _mut_dup_key(buf, keys, stmts, rng):
    if not keys:
        return buf, keys, stmts
    i = rng.randrange(len(keys))
    return buf, keys[:i + 1] + [keys[i]] + keys[i + 1:], stmts


def _mut_retype_word(buf, keys, stmts, rng):
    """Replace the trailing partial word with a different partial word."""
    head, word = _trailing_word(buf)
    n = rng.randint(1, 3)
    return head + "".join(rng.choice(FILTER_CHARS) for _ in range(n)), keys, stmts


def _mut_toggle_dash(buf, keys, stmts, rng):
    """Add or remove the `-` that switches a completer into option mode."""
    head, word = _trailing_word(buf)
    if word.startswith("-"):
        return head + word[1:], keys, stmts
    return head + "-" + word, keys, stmts


def _mut_drop_statement(buf, keys, stmts, rng):
    if not stmts:
        return buf, keys, stmts
    i = rng.randrange(len(stmts))
    return buf, keys, stmts[:i] + stmts[i + 1:]


def MUTATORS(pool):
    """The mutator table. `pool` is the full statement list to draw additions
    from, so a shrunk corpus entry can grow a style back."""
    def add_statement(buf, keys, stmts, rng):
        missing = [s for s in pool if s not in stmts]
        if not missing:
            return buf, keys, stmts
        return buf, keys, list(stmts) + [rng.choice(missing)]

    return [_mut_truncate, _mut_extend, _mut_swap_key, _mut_add_filter_key,
            _mut_add_nav_key, _mut_drop_key, _mut_dup_key, _mut_retype_word,
            _mut_toggle_dash, _mut_drop_statement, add_statement]


def corpus_weight(inp):
    """How often an entry is drawn as a mutation parent.

    Uniform sampling over the corpus wastes the thing that makes it a corpus:
    one promoted reproducer among 500 seeds gets picked 0.2% of the time, so
    the region around a KNOWN bug — where the neighbouring bugs are — is
    explored no harder than an arbitrary `echo ` case. Entries earn weight by
    evidence: a reproducer this fuzzer mined and minimised outranks a buffer
    that was only ever observed to diverge, which outranks a hand-written case
    that has always passed.
    """
    if inp.fingerprint:
        return 12
    if inp.origin.startswith("divergent-cases"):
        return 6
    return 1


def mutate_input(inp, rng, pool):
    """One or two small edits applied to a corpus entry."""
    buf, keys, stmts = inp.buffer, list(inp.keys), list(inp.statements)
    muts = MUTATORS(pool)
    applied = []
    for _ in range(rng.choice((1, 1, 2))):
        m = rng.choice(muts)
        buf, keys, stmts = m(buf, keys, stmts, rng)
        applied.append(m.__name__.replace("_mut_", ""))
    keys = [k for k in keys if _key_ok(k)] or ["tab"]
    return FuzzInput(buf, keys, stmts,
                     origin="mutate(%s)<-%s" % ("+".join(applied),
                                                inp.origin or os.path.basename(inp.path or "?")))


def _key_ok(name):
    try:
        key_bytes(name)
        return True
    except UnknownKey:
        return False


def write_statements(stmts, outdir, stem):
    """A zstyle fixture file for one statement subset, or None for no styles."""
    if not stmts:
        return None
    os.makedirs(outdir, exist_ok=True)
    path = os.path.join(outdir, stem + ".zsh")
    with open(path, "w") as f:
        f.write("\n".join(stmts) + "\n")
    return path


# ── three-dimension shrink ───────────────────────────────────────────────────

def shrink_input(args, env, dump, fpath_dirs, inp, target_fp, outdir, budget):
    """Minimise an input in ALL THREE dimensions it has.

    Shrinking only the zstyle set (what the combo fuzzer did) leaves a
    reproducer that still carries whatever 40-character command line and
    six-key sequence the fuzzer happened to roll, and the reader cannot tell
    which parts matter. Statements first (each probe is the same cost, and
    dropping styles usually collapses the fastest), then keys, then the buffer.

    The invariant is the FINGERPRINT, not merely "still fails": shrinking to a
    DIFFERENT bug would be worse than not shrinking, because the reproducer
    would then be evidence for a bug it does not actually demonstrate.

    Returns (buffer, keys, statements, probes_spent).
    """
    spent = [0]
    cache = {}

    def probe(buf, keys, stmts):
        if spent[0] >= budget:
            return False                     # budget gone: stop removing
        k = (buf, tuple(keys), tuple(stmts))
        if k in cache:
            return cache[k]
        spent[0] += 1
        stem = "shrink_%s_%d" % (target_fp, spent[0])
        init = build_init(dump, fpath_dirs, write_statements(stmts, outdir, stem))
        v = run_case(args, env, init, adhoc_case(buf), "shrink", list(keys))
        ok = v.status in ("FAIL", "FLAKY") and v.fingerprint == target_fp
        cache[k] = ok
        return ok

    buf, keys, stmts = inp.buffer, list(inp.keys), list(inp.statements)

    if stmts:
        stmts = shrink(stmts, lambda sub: probe(buf, keys, sub),
                       max_probes=max(0, budget - spent[0]))

    i = len(keys) - 1
    while i >= 0 and len(keys) > 1 and spent[0] < budget:
        cand = keys[:i] + keys[i + 1:]
        if probe(buf, cand, stmts):
            keys = cand
        i -= 1

    # Trailing characters, halving ladder: cheap on a long buffer, exact on a
    # short one.
    step = max(1, len(buf) // 2)
    while step >= 1 and spent[0] < budget:
        if len(buf) > step and probe(buf[:-step], keys, stmts):
            buf = buf[:-step]
        else:
            step //= 2

    return buf, keys, stmts, spent[0]


def run_mutate(args, env, dump, fpath_dirs):
    """Fuzz by MUTATING the persistent corpus, and grow the corpus from what it
    finds."""
    pool = read_statements(args.zstyle) if args.zstyle else []
    outdir = os.path.join(REPO, "target", "parity-fuzz-%d" % args.seed)
    os.makedirs(outdir, exist_ok=True)

    corpus = corpus_load(args.corpus_dir)
    if not corpus:
        _, written = corpus_seed(args, pool)
        print("# corpus was empty — seeded %d input(s) into %s"
              % (written, args.corpus_dir))
        corpus = corpus_load(args.corpus_dir)
    if not corpus:
        sys.exit("fuzz corpus is empty and could not be seeded: %s" % args.corpus_dir)

    known_fps = {i.fingerprint for i in corpus if i.fingerprint}
    if args.corpus_origin:
        picked = [i for i in corpus if args.corpus_origin in i.origin]
        if not picked:
            sys.exit("--corpus-origin %r matches none of the %d corpus input(s)"
                     % (args.corpus_origin, len(corpus)))
    else:
        picked = corpus
    weights = [corpus_weight(i) for i in picked]
    rng = random.Random((args.seed << 20) ^ args.mutate)

    inputs, seen, tries = [], set(), 0
    while len(inputs) < args.mutate and tries < args.mutate * 200:
        tries += 1
        cand = mutate_input(rng.choices(picked, weights)[0], rng, pool)
        if cand.key() in seen:
            continue
        seen.add(cand.key())
        inputs.append(cand)

    print("# mutation fuzz")
    print("# corpus : %s (%d input(s), %d known fingerprint(s))"
          % (args.corpus_dir, len(corpus), len(known_fps)))
    print("# parents: %d drawn from (weights: promoted=12 mined=6 case=1)%s"
          % (len(picked),
             "" if not args.corpus_origin else
             "  --corpus-origin %r" % args.corpus_origin))
    print("# mutants: %d   seed=%d   fixture=%s (%d statement(s))"
          % (len(inputs), args.seed, args.zstyle or "<none>", len(pool)))
    print("# mode   : %s (%s)" % (args.mode, " ".join(args.test_argv)))
    print("# jobs   : %d   shrink=%s probes<=%d   outdir=%s"
          % (max(1, args.jobs), args.shrink, args.shrink_probes, outdir))
    print()

    cells = []
    for n, inp in enumerate(inputs):
        zfile = write_statements(inp.statements, outdir, "mut%03d" % n)
        cells.append(Cell(adhoc_case(inp.buffer), "mut%03d" % n, inp.keys,
                          build_init(dump, fpath_dirs, zfile),
                          inp.statements, zfile, inp.origin))

    counts = {"PASS": 0, "FAIL": 0, "FLAKY": 0, "TIMEOUT": 0, "SKIP": 0}
    failures, results = [], []
    for cell, v in zip(cells, cell_stream(args, env, cells)):
        results.append(v)
        counts[v.status] = counts.get(v.status, 0) + 1
        line = "%-7s %-8s %r" % (v.status, v.seq, v.case.buffer)
        if v.status in ("FAIL", "FLAKY"):
            line += "  [%s]" % v.fingerprint
        print(line + (("  (%s)" % v.detail) if v.detail else ""))
        print("        keys=%s  styles=%d  from %s"
              % (",".join(v.keys), len(v.statements or ()), cell.origin))
        sys.stdout.flush()
        if v.status in ("FAIL", "FLAKY"):
            failures.append(v)
            print_failure(v, args)
        elif v.status == "TIMEOUT":
            print_timeout(v, args)
        sys.stdout.flush()

    print()
    groups = print_fingerprint_groups(failures, args) if failures else {}
    if not failures:
        print("# 0 failing cell(s), 0 distinct fingerprint(s)")

    # Promotion. A fingerprint the corpus has never recorded is new knowledge,
    # so its minimal reproducer becomes a corpus entry and every later run
    # starts from it.
    promoted = 0
    for fp, vs in groups.items():
        if fp in known_fps:
            print("# fingerprint %s already in the corpus — not re-promoted" % fp)
            continue
        rep = min(vs, key=lambda v: v.size())
        inp = FuzzInput(rep.case.buffer, rep.keys, rep.statements or [],
                        origin="promoted/%s" % rep.seq, fingerprint=fp,
                        note=fp_label(rep))
        buf, keys, stmts, probes = rep.case.buffer, list(rep.keys), list(rep.statements or []), 0
        if args.shrink:
            buf, keys, stmts, probes = shrink_input(
                args, env, dump, fpath_dirs, inp, fp, outdir, args.shrink_probes)
        minimal = FuzzInput(buf, keys, stmts, origin="promoted/%s" % rep.seq,
                            fingerprint=fp, note=fp_label(rep))
        zfile = write_statements(stmts, args.corpus_dir, minimal.stem("fp") + "_styles")
        path = corpus_write(args.corpus_dir, minimal, "fp")
        promoted += 1
        print("# NEW fingerprint %s promoted into the corpus" % fp)
        print("#   before: buffer=%r keys=%s statements=%d"
              % (rep.case.buffer, ",".join(rep.keys), len(rep.statements or ())))
        print("#   after : buffer=%r keys=%s statements=%d  (%d shrink probe(s) spent)"
              % (buf, ",".join(keys), len(stmts), probes))
        print("#   file  : %s" % path)
        print("#   replay: %s" % repro_cmd(args, buf, keys, zstyle=zfile))
        known_fps.add(fp)

    total = len(inputs)
    print("\n# %d passed, %d failed, %d cell(s)"
          % (counts["PASS"], counts["FAIL"] + counts["FLAKY"], total))
    print("# categories: PASS=%d FAIL=%d FLAKY=%d TIMEOUT=%d SKIP=%d"
          % (counts["PASS"], counts["FAIL"], counts["FLAKY"],
             counts["TIMEOUT"], counts["SKIP"]))
    if counts["TIMEOUT"]:
        print("# %d cell(s) ran out of MEASUREMENT budget — not divergences, not "
              "passes; re-run them at --jobs 1" % counts["TIMEOUT"])
    if counts["SKIP"]:
        print("# %d cell(s) skipped: command not installed here (--no-skip-missing "
              "to run them anyway)" % counts["SKIP"])
    print("# %d new fingerprint(s) promoted into %s" % (promoted, args.corpus_dir))

    if args.json:
        write_json(args, {
            "schema": "comptab-parity-fuzz/1",
            "mode": args.mode,
            "argv": sys.argv[1:],
            "corpus_dir": args.corpus_dir,
            "seed": args.seed,
            "summary": {
                "cells": total,
                "passed": counts["PASS"],
                "failed": counts["FAIL"] + counts["FLAKY"],
                "timeout": counts["TIMEOUT"],
                "skipped": counts["SKIP"],
                "fingerprints": len(groups),
                "promoted": promoted,
            },
            "fingerprints": fingerprint_doc(groups),
            "results": [to_json(v) for v in results],
        })
    return 1 if (failures or counts["TIMEOUT"] or counts["SKIP"]) else 0


# ── generated zstyle VALUES (--style-fuzz) ───────────────────────────────────
#
# `--random-combos` and `--mutate` only ever sample SUBSETS of a fixed fixture
# (scripts/parity_zstyle.zsh, the user's real styles). Every value they run is
# a value that was already in that file, so the richest part of the compsys
# configuration surface — the VALUE grammar of each style — was never
# exercised at all. That is the part this codebase's history keeps naming as
# the root cause: matcher reconstruction, tag-order, group-order, list-colors,
# ignore_prefix/ignore_suffix and completer-chain ORDER have each been a
# shipped bug.
#
# So this GENERATES statements instead of picking them. The grammar is taken
# from the zsh sources, not invented — the citations are `~/forkedRepos/zsh`:
#
#   Doc/Zsh/compwid.yo:937-1161   match specifications ("Completion Matching
#                                 Control"): a matcher is a case-sensitive
#                                 letter, `:`, one or more `|`-separated
#                                 patterns, `=`, and another pattern.
#   Src/Zle/complete.c:259-292    the parser: unknown letter -> `unknown match
#                                 specification character`, missing `:` ->
#                                 `missing ':'`.
#   Src/Zle/complete.c:359-378    `*`/`**` must be the WHOLE match-pat and need
#                                 an l/L/r/R matcher (`need anchor for '*'`);
#                                 word-pat and match-pat both empty is an error
#                                 (`need non-empty word or line pattern`).
#   Doc/Zsh/compwid.yo:970-1002   brace correspondence classes; no negation;
#                                 nth element on the left pairs with the nth on
#                                 the right.
#   Doc/Zsh/compsys.yo:2092-2098  a matcher-list element prefixed `+` ADDS to
#                                 the previous element instead of replacing it.
#   Doc/Zsh/compsys.yo:2131-2133  each element is a separate, complete pass.
#   Doc/Zsh/compsys.yo:2655-2698  tag-order: `-`, `!tags`, `tag:label`,
#                                 `tag:label:description`, `{pat1,pat2}`.
#   Completion/Base/Core/_tags:47-51  the three arms that actually parse it.
#   Doc/Zsh/compsys.yo:1297-1326  completer: `_name` or `_name:label`; a label
#                                 starting with `-` is appended to the name.
#   Doc/Zsh/compsys.yo:2189-2245  menu: the yes=/no=, select= and mode words
#                                 combine as separate list elements.
#   Doc/Zsh/compsys.yo:2146-2173  max-errors: `N`, `N numeric`, `N not-numeric`.
#   Completion/Unix/Type/_path_files:156-166  file-sort: the value is
#                                 SUBSTRING-matched, and `reverse`/`follow` are
#                                 independent substrings.
#   Doc/Zsh/mod_complist.yo:20-146  list-colors element forms.
#   Doc/Zsh/compsys.yo:573        the context layout.
#
# Three things are deliberately NOT generated, because the sources say they do
# not exist and emitting them would manufacture a finding out of a generator
# bug: `||` as an "or end of word" anchor (it is only anchor||coanchor),
# `lpat==tpat` as a distinct form (the parser splits on the FIRST `=`, so the
# second is a literal), and `*` as the match-pat of an `m:`/`M:` matcher.

# Standard tags — from the live fixture (scripts/parity_zstyle.zsh, captured
# from a real `zstyle -L`) plus the tags the file and option paths use, so a
# generated context names something that actually occurs.
GEN_TAGS = (
    "options", "arguments", "values", "commands", "aliases", "builtins",
    "functions", "parameters", "files", "globbed-files", "all-files",
    "directories", "local-directories", "named-directories", "corrections",
    "original", "expansions", "jobs", "signals", "users", "hosts", "packages",
    "history-words", "argument-rest", "strings", "descriptions", "messages",
    "warnings", "default", "paths", "reserved-words", "suffix-aliases",
    "global-aliases", "urls", "contexts",
)

# Commands whose completers exist on essentially any host, plus the special
# `-command-` context (Doc/Zsh/compsys.yo:317-379), so a command-specific
# context is not silently dead.
GEN_COMMANDS = ("git", "ls", "ssh", "kill", "make", "grep", "find", "tar",
                "cd", "chmod", "man", "-command-", "-default-")

# The `completer` field: the completer function's name with the leading `_`
# stripped and remaining `_` turned into `-`. `_approximate` / `_correct`
# rewrite it to `approximate-<n>` / `correct-<n>`, which is why those are here.
GEN_COMPLETER_FIELDS = ("complete", "approximate", "approximate-1",
                        "correct", "correct-1", "expand", "match", "prefix",
                        "ignored", "list", "menu", "oldlist", "history", "*")

# The `function` field: empty for an ordinary TAB, the widget name otherwise.
GEN_FUNCTIONS = ("*", "", "_complete_help", "_correct_word", "_expand_word")


def gen_context(rng, tag=None):
    """A `:completion:` context PATTERN, at a random specificity.

    Specificity is its own fuzzing axis: the same style at `':completion:*'`
    and at `':completion:*:*:git:*:*'` is a different configuration, a literal
    context beats a pattern and a longer pattern beats `*`
    (Doc/Zsh/compsys.yo:699-705), and equally specific statements resolve in
    DEFINITION order — so both the pattern and its position in the file are
    part of the input being fuzzed.
    """
    t = tag or rng.choice(GEN_TAGS)
    cmd = rng.choice(GEN_COMMANDS)
    comp = rng.choice(GEN_COMPLETER_FIELDS)
    fn = rng.choice(GEN_FUNCTIONS)
    forms = (
        ":completion:*",
        ":completion:*:*",
        ":completion:*:%s" % t,
        ":completion:*:*:%s:*" % cmd,
        ":completion:*:*:%s:*:*" % cmd,
        ":completion:*:%s:*" % comp,
        ":completion:*:*:*:*:%s" % t,
        ":completion:*:*:*:*:*",
        ":completion:%s:%s:%s:*:*" % (fn, comp, cmd),
        ":completion:*:*:*:*:default",
        ":completion:*:%s:%s:*:%s" % (comp, cmd, t),
        ":completion:%s:%s:%s::%s" % (fn, comp, cmd, t),
    )
    return rng.choice(forms)


# ── match specifications ─────────────────────────────────────────────────────
#
# compwid.yo:970-1002 — a brace expression is a list of literal characters,
# ranges and character classes, and the nth element on the left corresponds to
# the nth on the right. These are the correspondences that MEAN something (case
# folding, the user's own `-`/`_` fold), not random braces.
MATCH_BRACE_PAIRS = (
    ("{a-z}", "{A-Z}"),
    ("{A-Z}", "{a-z}"),
    ("{a-zA-Z}", "{A-Za-z}"),
    ("{[:lower:]}", "{[:upper:]}"),
    ("{[:upper:]}", "{[:lower:]}"),
    ("{[:lower:][:upper:]}", "{[:upper:][:lower:]}"),
    (r"{a-z\-}", r"{A-Z\_}"),          # verbatim from the live fixture
    ("{-_}", "{_-}"),
    ("{_-}", "{-_}"),
    ("{a-z-}", "{A-Z_}"),
)

# Patterns legal inside a matcher: literals (backslash-quotable), `?`, bracket
# expressions and brace expressions — "Other shell patterns are not allowed"
# (compwid.yo:951-968).
MATCH_WORD_PATS = ("_", "-", ".", "?", "[._-]", "[[:alpha:]]", "[^[:alpha:]]",
                   "[[:upper:]]", "[[:lower:]]", "[A-Z0-9]", "[^A-Z0-9]",
                   "[.,_-]", "no-", "--", "0", "[-+]")
MATCH_ANCHORS = ("[._-]", ".", "-", "--", "_", "/", "[[:upper:]]",
                 "[[:alpha:]]", "[A-Z0-9]", "?")
MATCH_PLAIN_TARGETS = ("", "_", "-", "+", ".", "?", "by")


def gen_matcher(rng):
    """ONE matcher, in one of the documented shapes.

    Legality is enforced HERE rather than discovered by the reference shell: a
    spec zsh refuses (`unknown match specification character`, `unterminated
    character class`, `need anchor for '*'`) is a generator bug that wastes a
    cell and produces a diagnostic on both sides, so `*` is only emitted for
    l/L/r/R, `**` only when the matcher is anchored, every bracket and brace
    expression is emitted balanced, and the word-pat and match-pat are never
    both empty (complete.c:373-378).
    """
    shape = rng.choices(
        ("brace", "plain", "edge", "anchor", "coanchor", "x"),
        weights=(28, 15, 20, 22, 12, 3))[0]

    if shape == "brace":
        lp, rp = rng.choice(MATCH_BRACE_PAIRS)
        return "%s:%s=%s" % (rng.choice("mM"), lp, rp)

    if shape == "plain":
        # compwid.yo:1023-1062 — m/M anywhere, b/B at the beginning, e/E at the
        # end. No `*` here: it needs an l/L/r/R matcher.
        return "%s:%s=%s" % (rng.choice(("m", "M", "b", "B", "e", "E")),
                             rng.choice(MATCH_WORD_PATS),
                             rng.choice(MATCH_PLAIN_TARGETS))

    if shape == "edge":
        # compwid.yo:1064-1067 — `l:|word-pat=match-pat`, `r:word-pat|=match-pat`.
        # `*` is legal as the match-pat here; `**` is not (it needs an anchor).
        letter = rng.choice("lLrR")
        target = rng.choice(MATCH_PLAIN_TARGETS + ("*", "*", "*"))
        wp = rng.choice(("",) + MATCH_WORD_PATS)
        if not wp and not target:
            target = "*"                 # never both empty: complete.c:373-378
        return ("%s:|%s=%s" % (letter, wp, target) if letter in "lL"
                else "%s:%s|=%s" % (letter, wp, target))

    if shape == "anchor":
        # compwid.yo:1089-1092 — `l:anchor|word-pat=`, `r:word-pat|anchor=`.
        # With an anchor present `**` becomes legal too (compwid.yo:1105-1111):
        # `*` cannot cross an anchor match, `**` can.
        letter = rng.choice("lLrR")
        target = rng.choice(MATCH_PLAIN_TARGETS + ("*", "*", "**"))
        anchor = rng.choice(MATCH_ANCHORS)
        wp = rng.choice(("",) + MATCH_WORD_PATS)
        if not wp and not target:
            target = "*"
        return ("%s:%s|%s=%s" % (letter, anchor, wp, target) if letter in "lL"
                else "%s:%s|%s=%s" % (letter, wp, anchor, target))

    if shape == "coanchor":
        # compwid.yo:1124-1131 — `l:anchor||coanchor=`, `r:coanchor||anchor=`.
        letter = rng.choice("lLrR")
        target = rng.choice(MATCH_PLAIN_TARGETS[1:] + ("*", "**"))
        a, co = rng.sample(MATCH_ANCHORS, 2)
        return ("%s:%s||%s=%s" % (letter, a, co, target) if letter in "lL"
                else "%s:%s||%s=%s" % (letter, co, a, target))

    return "x:"                          # compwid.yo:1150-1159


def gen_matcher_element(rng):
    """One ELEMENT of matcher-list.

    Within an element the matchers are whitespace-separated and applied one at
    a time, left to right, each broadening the pattern further
    (compwid.yo:915-918). A trailing `x:` makes everything to its right
    inert, which is how one specification overrides another.
    """
    n = rng.choices((1, 2, 3), weights=(52, 32, 16))[0]
    parts = [gen_matcher(rng) for _ in range(n)]
    if rng.random() < 0.05:
        parts.append("x:")
    return " ".join(parts)


def gen_matcher_list(rng):
    """A matcher-list VALUE: each element is a separate, complete completion
    PASS, tried in order (compsys.yo:2131-2133), so the ORDER and the COUNT are
    both semantic.

    The leading `''` is the standard "try plain matching first" idiom and opens
    the user's own fixture, so it is weighted rather than left to chance. A `+`
    prefix on a later element ADDS to the previous element's spec instead of
    replacing it (compsys.yo:2092-2098) — a form nothing in the fixture uses.
    """
    out = []
    if rng.random() < 0.5:
        out.append("")
    for i in range(rng.choices((1, 2, 3), weights=(48, 36, 16))[0]):
        el = gen_matcher_element(rng)
        if i and out and out[-1] and rng.random() < 0.2:
            el = "+" + el
        out.append(el)
    return out


# ── the rest of the style grammar ────────────────────────────────────────────

GEN_COMPLETERS = ("_complete", "_approximate", "_expand", "_expand_alias",
                  "_match", "_prefix", "_ignored", "_correct", "_list",
                  "_menu", "_oldlist", "_history")

# Orderings that are known to interact. `completer` is a CHAIN and its order is
# semantic: a completer that returns 0 ends the chain, which is exactly the
# `_first` regression (a no-op `-first-` hook returned 0 and silently reduced
# every multi-completer config to `_complete` alone). A repeated entry is legal
# and must not double-list.
GEN_COMPLETER_CHAINS = (
    ("_complete",),
    ("_complete", "_approximate"),
    ("_expand", "_complete", "_ignored", "_approximate"),
    ("_oldlist", "_complete"),
    ("_complete", "_match"),
    ("_prefix", "_complete"),
    ("_complete", "_ignored", "_correct", "_approximate"),
    ("_expand_alias", "_complete"),
    ("_menu", "_complete"),
    ("_list", "_complete"),
    ("_history", "_complete"),
    ("_complete", "_complete"),
    ("_ignored", "_complete"),
    ("_approximate", "_complete"),
    ("_match", "_complete", "_approximate"),
    ("_expand", "_complete"),
)


def gen_completer(rng):
    """An ORDERED completer chain.

    compsys.yo:1297-1326 — an element may also be `_name:label`, and a label
    starting with `-` is appended to the derived name, which is how the same
    completer is run twice under two different style contexts.
    """
    if rng.random() < 0.5:
        out = list(rng.choice(GEN_COMPLETER_CHAINS))
    else:
        out = [rng.choice(GEN_COMPLETERS) for _ in range(rng.randint(1, 4))]
    if rng.random() < 0.15:
        i = rng.randrange(len(out))
        out[i] += rng.choice((":-alt", ":second", ":-two"))
    return out


def gen_tag_order(rng):
    """compsys.yo:2655-2698 / _tags:47-51 — three arms parse this: `-` alone,
    a string starting with `!` (those tags are excluded), and anything else,
    which is pattern-matched."""
    def one():
        r = rng.random()
        if r < 0.05:
            return "-"
        if r < 0.15:
            return rng.choice(("!", "! ")) + rng.choice(GEN_TAGS)
        if r < 0.25:
            return "%s:-%s" % (rng.choice(GEN_TAGS),
                               rng.choice(("alt", "second", "non-comp")))
        if r < 0.33:
            return "%s:%s:%s" % (rng.choice(GEN_TAGS),
                                 rng.choice(("alt", "grp")),
                                 rng.choice(("long\\ options",
                                             "other\\ matches", "%d")))
        if r < 0.40:
            return "{%s,%s}" % (rng.choice(GEN_TAGS), rng.choice(GEN_TAGS))
        return " ".join(rng.sample(GEN_TAGS, rng.randint(1, 3)))
    return [one() for _ in range(rng.randint(1, 3))]


GEN_GROUPS = ("options", "commands", "files", "directories", "aliases",
              "builtins", "functions", "parameters", "corrections",
              "globbed-files", "all-files", "original", "expansions",
              "argument-rest", "-default-")

# compwid.yo:593-601 — the escapes `compadd -X` accepts. `%G` is NOT one of
# them, so it is not generated.
GEN_FORMATS = (
    "%d",
    "-- %d --",
    "%B%d%b",
    "%U%d%u",
    "%F{yellow}%d%f",
    "%K{blue}%F{white}%d%f%k",
    "Completing %d",
    "%SNo matches for: %d%s",
    "$'\\C-[[1;31m-<<\\C-[[0;34m%d\\C-[[1;31m>>-\\C-[[0m'",
)

# mod_complist.yo:30-133 — `name=value` for a file type, `*suffix=value`,
# `=pattern=value` (with `(#b)` back-references feeding extra `=`-separated
# codes), any of them optionally prefixed with a `(group-pattern)`.
GEN_LIST_COLORS = (
    "ma=37;1;4;44", "di=1;34", "ln=35", "ex=31;1", "no=0", "fi=0;37",
    "so=32", "or=31;1", "sp=33", "ec=",
    "=(#b)(*)=1;30=1;32;43", "=(#b)(*)=1;30=1;36;44", "=(#b)(*)/(*)==1;35=1;33",
    "*.rs=32", "*.md=33", "=*=1;35", "(files)*.o=90",
)

# compsys.yo:1800-1810 — EXTENDED_GLOB is in force here, so `#`, `~` and `^`
# are special.
GEN_IGNORED_PATTERNS = ("_*", "*.o", "*~", "[-+]?", "(*/)#CVS", "*.(o|a)",
                        ".*", "*?.zwc", "[0-9]*", "???*", "--*",
                        "[-+](|-|[^-]*)")

# _path_files:156-166 — the value is substring-matched into a glob qualifier.
GEN_FILE_SORT = ("name", "size", "links", "modification", "time", "date",
                 "access", "inode", "change")


def _bool(rng, *extra):
    """compsys.yo:1097-1103 — the true set is true/on/yes/1, the false set is
    false/off/no/0. `zstyle -t` is true only for a ONE-element value, so a
    boolean style is always emitted as a single word."""
    return rng.choice(("true", "false", "yes", "no", "on", "off", "1", "0")
                      + tuple(extra))


def gen_menu(rng):
    """compsys.yo:2189-2245 — the yes=/no= part, the select part and the mode
    part are independent list elements that combine ("either alongside or
    instead of")."""
    base = rng.choice((None, "yes", "no", "true", "false", "auto", "1", "0",
                       "yes=2", "yes=long", "yes=long-list", "no=10"))
    sel = rng.choice((None, "select", "select=0", "select=2", "select=5",
                      "select=long", "select=long-list", "no-select"))
    mode = rng.choice((None, None, "interactive", "search", "search-backward"))
    out = [p for p in (base, sel, mode) if p]
    return out or ["yes"]


def gen_max_errors(rng):
    """compsys.yo:2146-2173 — `N`, or N together with `numeric` /
    `not-numeric`. `0 numeric` disables correction unless a numeric argument
    is given."""
    n = str(rng.choice((0, 1, 2, 3, 5)))
    tail = rng.choice((None, "numeric", "not-numeric"))
    return [n] if tail is None else [n, tail]


def gen_file_sort(rng):
    """compsys.yo:1560-1574 — a base ordering, plus `reverse` and `follow` as
    independent substrings of the same value."""
    words = [rng.choice(GEN_FILE_SORT)]
    if rng.random() < 0.35:
        words.append("reverse")
    if rng.random() < 0.2:
        words.append("follow")
    return words


# style -> (value generator, tags whose CONTEXT the style is read under).
#
# The tag matters. `format` is read for descriptions / messages / warnings /
# corrections, so generating it at `':completion:*:options'` would set a style
# nothing ever reads and the cell would compare two identical no-ops — a
# guaranteed pass that measures nothing.
GEN_STYLES = {
    "matcher-list":       (gen_matcher_list, None),
    "matcher":            (lambda r: [gen_matcher_element(r)], None),
    "completer":          (gen_completer, None),
    "tag-order":          (gen_tag_order, None),
    "group-order":        (lambda r: r.sample(GEN_GROUPS, r.randint(2, 6)), None),
    "group-name":         (lambda r: [r.choice(("", "", "%t", "matches"))], None),
    "format":             (lambda r: [r.choice(GEN_FORMATS)],
                           ("descriptions", "messages", "warnings", "corrections")),
    "auto-description":   (lambda r: [r.choice(("Specify: %d", "%d", "arg: %d"))],
                           None),
    "list-colors":        (lambda r: [r.choice(GEN_LIST_COLORS)
                                      for _ in range(r.randint(1, 3))], None),
    "ignored-patterns":   (lambda r: [r.choice(GEN_IGNORED_PATTERNS)
                                      for _ in range(r.randint(1, 2))], None),
    "squeeze-slashes":    (lambda r: [_bool(r)], ("paths",)),
    "list-dirs-first":    (lambda r: [_bool(r)], None),
    "menu":               (gen_menu, ("default",)),
    "max-errors":         (gen_max_errors, None),
    "insert-unambiguous": (lambda r: [_bool(r, "pattern")], None),
    "accept-exact":       (lambda r: [_bool(r, "continue")], ("default", "paths")),
    "special-dirs":       (lambda r: [_bool(r, "..")], None),
    "verbose":            (lambda r: [_bool(r)], None),
    "extra-verbose":      (lambda r: [_bool(r)], None),
    "file-sort":          (gen_file_sort, None),
    "use-cache":          (lambda r: [_bool(r)], None),
    "single-ignored":     (lambda r: [r.choice(("show", "menu"))], None),
    "hidden":             (lambda r: [_bool(r, "all")], None),
    "prefix-needed":      (lambda r: [_bool(r)],
                           ("options", "signals", "jobs", "functions",
                            "parameters")),
    "ambiguous":          (lambda r: [_bool(r)], ("paths",)),
    "sort":               (lambda r: [_bool(r, "match", "nosort", "numeric",
                                            "reverse")], None),
    "list-packed":        (lambda r: [_bool(r)], None),
    "list-rows-first":    (lambda r: [_bool(r)], None),
    "list-grouped":       (lambda r: [_bool(r)], None),
    "list-separator":     (lambda r: [r.choice(("--", "#", "/////", "->"))], None),
    "original":           (lambda r: [_bool(r)], ("corrections", "original")),
    "keep-prefix":        (lambda r: [_bool(r, "changed")], None),
    "add-space":          (lambda r: [_bool(r, "file", "subst")], None),
    "substitute":         (lambda r: [_bool(r)], None),
    "expand":             (lambda r: r.sample(("prefix", "suffix"),
                                              r.randint(1, 2)), ("paths",)),
    "complete-options":   (lambda r: [_bool(r)], None),
    "last-prompt":        (lambda r: [_bool(r)], None),
    "list-suffixes":      (lambda r: [_bool(r)], ("paths",)),
    "accept-exact-dirs":  (lambda r: [_bool(r)], None),
    "list-prompt":        (lambda r: [r.choice((
                              "%SAt %p: Hit TAB for more, or the character to insert%s",
                              "%p", "%SScrolling: %M%p%s", "%l %m %P", ""))], None),
    "select-prompt":      (lambda r: [r.choice((
                              "%SScrolling active: current selection at %p%s",
                              "%p", "%m %p", ""))], None),
    "cache-policy":       (None, None),   # emitted specially — see gen_statement
}

# `cache-policy` names a FUNCTION (compsys.yo:1224-1227). A generated name that
# does not exist is a config zsh refuses, so the definition is emitted on the
# SAME LINE as the statement: the shrinker treats a line as an atom, so the
# pair can never be split into a dangling reference.
_CACHE_POLICY_LINE = ("_ctp_policy_%(n)d() { return %(n)d }; "
                      "zstyle %(ctx)s cache-policy _ctp_policy_%(n)d")


def gen_statement(rng, style=None):
    """One complete, syntactically valid `zstyle` line."""
    style = style or rng.choice(sorted(GEN_STYLES))
    gen, pref = GEN_STYLES[style]
    if style == "cache-policy":
        return _CACHE_POLICY_LINE % {"n": rng.choice((0, 1)),
                                     "ctx": shlex.quote(gen_context(rng))}
    tag = rng.choice(pref) if pref else None
    ctx = gen_context(rng, tag=tag)
    values = gen(rng)
    return "zstyle %s %s %s" % (shlex.quote(ctx), style,
                                " ".join(shlex.quote(v) for v in values))


def gen_config(rng, n_styles, only=None):
    """A whole generated configuration.

    No two statements set the same style at the same context (the later one
    would simply overwrite the earlier, wasting a slot), but the same style AT
    A DIFFERENT context is deliberately allowed — that overlap is where the
    most-specific-first resolution rule is actually tested.
    """
    pool = sorted(only) if only else sorted(GEN_STYLES)
    out, seen = [], set()
    for _ in range(n_styles * 8):
        if len(out) >= n_styles:
            break
        stmt = gen_statement(rng, rng.choice(pool))
        key = " ".join(stmt.split()[:3])
        if key in seen:
            continue
        seen.add(key)
        out.append(stmt)
    return out


# ── validating a generated config against the reference shell ────────────────
#
# A statement REAL zsh refuses is a generator bug, not a finding. Scoring one
# as a divergence would be inventing a zshrs bug out of this script's own
# mistake, and scoring it as a pass would be worse. So every generated config
# is put to `zsh` itself before a single pty is booted, and anything it rejects
# is counted under its own category and printed with the offending line.
#
# `zstyle` validates the CONTEXT PATTERN at definition time and nothing else
# (measured: `zstyle ':completion:*(' menu yes` -> `zsh: zstyle: invalid
# pattern`, while `zstyle ':completion:*' matcher-list 'm:{a-z}={A-Z'` is
# accepted and only fails later, inside compadd). Both layers are therefore
# checked: `zsh -c` for what the builtin rejects outright, and — after the cell
# has run — the REFERENCE shell's own diagnostics for what it rejects at
# completion time.

# What the reference zsh prints when it refuses a generated style VALUE at
# completion time. Measured on this host by feeding zsh deliberately bad
# fixtures (2026-08-30):
#   matcher-list 'm:{a-z}={A-Z'  -> _describe:compadd:114: unterminated character class
#   matcher-list 'q:foo=bar'     -> ...: unknown match specification character `q'
#   a bad pattern in a spec      -> _path_files:compadd:717: invalid pattern character `='
#   completer _nosuchcompleter   -> _main_complete:218: command not found: _nosuchcompleter
# zsh silently TOLERATES a bad list-colors, ignored-patterns, menu, max-errors,
# file-sort or tag-order value, so those cannot be validated this way and are
# generated conservatively instead.
REF_REJECT_RE = tuple(re.compile(p, re.I) for p in (
    r"unknown match specification character",
    r"unterminated character class",
    r"invalid pattern character",
    r"need anchor for",
    r"need non-empty word or line pattern",
    r"missing ':'",
    r"command not found: _",
    r"bad match specification",
))


def ref_rejects(cap):
    """The reference shell's own complaints about the generated config."""
    if cap is None:
        return []
    return sorted(m for m in cap.diags
                  if any(r.search(m) for r in REF_REJECT_RE))


def validate_config(statements, zsh, outdir, generated=None):
    """Ask REAL zsh whether it accepts these statements.

    Returns [(statement, complaint)] — empty when the config is clean. The
    whole file is checked in ONE `zsh -c` first (the common case is clean, and
    that costs a single process); only when that complains is each statement
    re-checked individually, so the report can name the exact offending line
    instead of the file.

    The round-trip check is the second half: after sourcing, `zstyle -L` must
    list at least as many definitions as there are DISTINCT (context, style)
    pairs. A statement zsh parsed but silently failed to store would otherwise
    look valid and produce a cell that measures nothing.

    `generated` is the subset THIS SCRIPT produced, and the round-trip count is
    applied to it alone, in its own `zsh -c`. A `--style-fuzz-mix` config also
    carries hand-written fixture lines, and those are not one-statement-per-line
    (scripts/parity_zstyle.zsh defines caching-policy FUNCTIONS spanning several
    lines); counting them textually against `zstyle -L` output compares two
    different things and reported 2 of 2 valid configs as invalid. The syntax
    half still covers every line either way.
    """
    import subprocess

    def ask(lines):
        path = os.path.join(outdir, "validate.zsh")
        with open(path, "w") as f:
            f.write("\n".join(lines) + "\n")
        try:
            p = subprocess.run([zsh, "-f", "-c", "source %s; zstyle -L"
                                % shlex.quote(path)],
                               capture_output=True, text=True, timeout=20)
        except (OSError, subprocess.SubprocessError) as exc:
            return "could not run %s: %s" % (zsh, exc), 0
        err = (p.stderr or "").strip()
        if p.returncode != 0 and not err:
            err = "zsh exited %d" % p.returncode
        return err, len([l for l in (p.stdout or "").splitlines() if l.strip()])

    err, _kept = ask(statements)
    if err:
        bad = []
        for s in statements:
            e, _ = ask([s])
            if e:
                bad.append((s, e.splitlines()[0]))
        if bad:
            return bad
        return [("<whole config>", err.splitlines()[0])]

    gen = statements if generated is None else generated
    if not gen:
        return []
    _err, kept = ask(gen)
    want = len({" ".join(g.split()[:3]) for g in gen})
    if kept < want:
        return [("<generated statements>",
                 "zsh stored only %d of the %d distinct (context, style) "
                 "definition(s) it was given" % (kept, want))]
    return []
def _style_fuzz_cases(args):
    """The case pool a generated config is judged on."""
    cases = [c for c in cases_by_tag(args.tag)
             if not (args.skip_optional and "optional" in c.tags)]
    if args.combo_cases:
        want = {c.strip() for c in args.combo_cases.split(",")}
        cases = [c for c in cases if c.name in want or c.buffer in want]
    if not cases:
        sys.exit("--style-fuzz has no cases to run (check --tag / --combo-cases)")
    return cases


def run_style_fuzz(args, env, dump, fpath_dirs):
    """Fuzz GENERATED zstyle statements, not subsets of a fixed fixture.

    Everything downstream of the config is the machinery that already exists:
    the same Cell / cell_stream / run_case path, the same fingerprint, the same
    three-dimension `shrink_input`, the same corpus promotion. What is new is
    only where the statements come from.

    Categories, and why each one is separate:

        PASS            valid config, both screens byte-identical.
        FAIL / FLAKY    a divergence. Unchanged meaning.
        TIMEOUT / SKIP  unchanged meaning.
        INVALID-CONFIG  the reference zsh REFUSED the generated statement at
                        definition time. A generator bug. The cell is not run,
                        because comparing two shells on a config neither can
                        hold says nothing.
        REF-REFUSED     the config parsed, but the reference zsh complained
                        about the VALUE at completion time (a bad match spec, a
                        completer that does not exist). The cell IS still run
                        and still compared — zshrs is required to refuse it
                        identically — but it is tallied separately so a green
                        sweep can never be assembled out of configs zsh itself
                        rejects.

    A cell is never scored a pass on the strength of an invalid config, and no
    category is silently dropped: every one is counted, printed, and keeps the
    exit status non-zero.
    """
    outdir = os.path.join(REPO, "target", "parity-style-fuzz-%d" % args.seed)
    os.makedirs(outdir, exist_ok=True)

    only = None
    if args.style_fuzz_only:
        only = [s.strip() for s in args.style_fuzz_only.split(",") if s.strip()]
        unknown = [s for s in only if s not in GEN_STYLES]
        if unknown:
            sys.exit("--style-fuzz-only names unknown style(s): %s\nknown: %s"
                     % (", ".join(unknown), ", ".join(sorted(GEN_STYLES))))

    # Only self-contained `zstyle` lines are mixable. scripts/parity_zstyle.zsh
    # is a sourced shell file, not a list of statements: it also defines the
    # caching-policy FUNCTIONS the styles name, across several lines each.
    # Drawing a random subset of its lines therefore splits a function body and
    # hands both shells a dangling `}` — measured, `zsh` answered
    # `parse error near '}'` and the config was thrown out as invalid. Keeping
    # the pool to `zstyle` lines composes the two sources without inventing
    # broken shell; nothing about the comparison changes.
    pool = [s for s in (read_statements(args.zstyle) if args.zstyle else [])
            if s.lstrip().startswith("zstyle ")]
    if args.style_fuzz_mix > 0 and not pool:
        sys.exit("--style-fuzz-mix needs --zstyle FIXTURE to draw `zstyle` "
                 "statements from")

    cases = _style_fuzz_cases(args)
    keys = KEY_SEQUENCES[args.combo_sequence]

    print("# style-value fuzz (GENERATED zstyle statements)")
    print("# configs : %d   styles/config=%d   seed=%d" %
          (args.style_fuzz, args.style_fuzz_styles, args.seed))
    print("# styles  : %s" % (", ".join(only) if only
                              else "%d generated style(s)" % len(GEN_STYLES)))
    print("# mix     : %.2f from %s" % (args.style_fuzz_mix,
                                        args.zstyle or "<none>"))
    print("# cases   : %d   sequence=%s (%s)"
          % (len(cases), args.combo_sequence, "+".join(keys)))
    print("# mode    : %s (%s)" % (args.mode, " ".join(args.test_argv)))
    print("# jobs    : %d   shrink=%s probes<=%d" %
          (max(1, args.jobs), args.shrink, args.shrink_probes))
    print("# outdir  : %s" % outdir)
    print()

    # ── generate, then let REAL zsh vet every statement before any pty boots ──
    configs, invalid = [], []
    for n in range(args.style_fuzz):
        rng = random.Random((args.seed << 20) ^ 0x57F0 ^ n)
        gen_stmts = gen_config(rng, args.style_fuzz_styles, only)
        stmts = gen_stmts
        if pool and args.style_fuzz_mix > 0:
            # The fixture subset goes FIRST: equally specific statements resolve
            # in definition order, so a generated statement placed after one of
            # the user's real ones is the case that actually tests the override.
            stmts = random_subset(pool, args.style_fuzz_mix, rng) + gen_stmts
        bad = validate_config(stmts, args.zsh, outdir, generated=gen_stmts)
        if bad:
            invalid.append((n, stmts, bad))
            continue
        configs.append((n, stmts, rng.choice(cases)))

    if invalid:
        print("# %d generated config(s) REJECTED BY zsh ITSELF — generator bugs, "
              "not findings. Not run, not passed:" % len(invalid))
        for n, _stmts, bad in invalid:
            for stmt, err in bad[:4]:
                print("#   cfg %-4d %s" % (n, err))
                print("#            %s" % stmt)
        print()

    cells = []
    for n, stmts, case in configs:
        zfile = write_statements(stmts, outdir, "cfg%04d" % n)
        cells.append(Cell(case, "cfg%04d" % n, keys,
                          build_init(dump, fpath_dirs, zfile),
                          stmts, zfile, "style-fuzz/%d" % n))

    counts = {"PASS": 0, "FAIL": 0, "FLAKY": 0, "TIMEOUT": 0, "SKIP": 0}
    failures, results, refused = [], [], []
    for cell, v in zip(cells, cell_stream(args, env, cells)):
        results.append(v)
        counts[v.status] = counts.get(v.status, 0) + 1
        rejects = ref_rejects(v.ref)
        label = v.status
        if rejects:
            refused.append((v, rejects))
            label = "%s(ref-refused)" % v.status
        line = "%-20s %-9s %r" % (label, v.seq, v.case.buffer)
        if v.status in ("FAIL", "FLAKY"):
            line += "  [%s]" % v.fingerprint
        print(line + (("  (%s)" % v.detail) if v.detail else ""))
        print("        keys=%s  styles=%d  fixture=%s"
              % (",".join(v.keys), len(v.statements or ()), cell.zstyle_file))
        for s in (v.statements or [])[:args.style_fuzz_styles + 2]:
            print("          %s" % s)
        for m in rejects:
            print("        ! zsh itself refused this config: %s" % m)
        sys.stdout.flush()
        if v.status in ("FAIL", "FLAKY"):
            failures.append(v)
            print_failure(v, args)
        elif v.status == "TIMEOUT":
            print_timeout(v, args)
        sys.stdout.flush()

    print()
    groups = print_fingerprint_groups(failures, args) if failures else {}
    if not failures:
        print("# 0 failing cell(s), 0 distinct fingerprint(s)")

    # Promotion + three-dimension shrink, exactly as --mutate does it: a
    # fingerprint the corpus has never seen becomes a corpus entry, minimised
    # first, so the next run starts from the generated config that found it.
    known_fps = {i.fingerprint for i in corpus_load(args.corpus_dir)
                 if i.fingerprint}
    promoted = 0
    for fp, vs in groups.items():
        rep = min(vs, key=lambda v: v.size())
        if fp in known_fps:
            print("# fingerprint %s already in the corpus — not re-promoted" % fp)
            continue
        inp = FuzzInput(rep.case.buffer, rep.keys, rep.statements or [],
                        origin="style-fuzz/%s" % rep.seq, fingerprint=fp,
                        note=fp_label(rep))
        buf, kys, stmts, probes = (rep.case.buffer, list(rep.keys),
                                   list(rep.statements or []), 0)
        if args.shrink:
            buf, kys, stmts, probes = shrink_input(
                args, env, dump, fpath_dirs, inp, fp, outdir, args.shrink_probes)
        minimal = FuzzInput(buf, kys, stmts, origin="style-fuzz/%s" % rep.seq,
                            fingerprint=fp, note=fp_label(rep))
        zfile = write_statements(stmts, args.corpus_dir,
                                 minimal.stem("fp") + "_styles")
        path = corpus_write(args.corpus_dir, minimal, "fp")
        promoted += 1
        print("# NEW fingerprint %s promoted into the corpus" % fp)
        print("#   before: buffer=%r keys=%s statements=%d"
              % (rep.case.buffer, ",".join(rep.keys), len(rep.statements or ())))
        print("#   after : buffer=%r keys=%s statements=%d  (%d shrink probe(s))"
              % (buf, ",".join(kys), len(stmts), probes))
        for s in stmts:
            print("#     %s" % s)
        print("#   file  : %s" % path)
        print("#   styles: %s" % (zfile or "<none>"))
        print("#   replay: %s" % repro_cmd(args, buf, kys, zstyle=zfile))
        known_fps.add(fp)

    total = args.style_fuzz
    print("\n# %d passed, %d failed, %d config(s)"
          % (counts["PASS"], counts["FAIL"] + counts["FLAKY"], total))
    print("# categories: PASS=%d FAIL=%d FLAKY=%d TIMEOUT=%d SKIP=%d "
          "INVALID-CONFIG=%d REF-REFUSED=%d"
          % (counts["PASS"], counts["FAIL"], counts["FLAKY"], counts["TIMEOUT"],
             counts["SKIP"], len(invalid), len(refused)))
    if invalid:
        print("# %d config(s) INVALID: zsh's own `zstyle` refused the statement, "
              "so the cell was never run. This is a bug in the GENERATOR, not "
              "in zshrs — fix the grammar above." % len(invalid))
    if refused:
        print("# %d cell(s) ran under a config the reference zsh complained "
              "about at completion time. They were still compared (zshrs has to "
              "refuse identically), but they are NOT clean passes:" % len(refused))
        for v, ms in refused[:10]:
            print("#   cfg %s: %s" % (v.seq, ms[0]))
    if counts["TIMEOUT"]:
        print("# %d cell(s) ran out of MEASUREMENT budget — not divergences, not "
              "passes; re-run them at --jobs 1" % counts["TIMEOUT"])
    if counts["SKIP"]:
        print("# %d cell(s) skipped: command not installed here" % counts["SKIP"])
    print("# %d new fingerprint(s) promoted into %s" % (promoted, args.corpus_dir))

    if args.json:
        write_json(args, {
            "schema": "comptab-parity-style-fuzz/1",
            "mode": args.mode,
            "argv": sys.argv[1:],
            "seed": args.seed,
            "outdir": outdir,
            "summary": {
                "configs": total,
                "passed": counts["PASS"],
                "failed": counts["FAIL"] + counts["FLAKY"],
                "timeout": counts["TIMEOUT"],
                "skipped": counts["SKIP"],
                "invalid_config": len(invalid),
                "ref_refused": len(refused),
                "fingerprints": len(groups),
                "promoted": promoted,
            },
            "invalid_configs": [{"config": n, "statements": s,
                                 "rejected": [{"statement": a, "error": b}
                                              for a, b in bad]}
                                for n, s, bad in invalid],
            "ref_refused": [{"id": v.id, "statements": list(v.statements or []),
                             "messages": ms} for v, ms in refused],
            "fingerprints": fingerprint_doc(groups),
            "results": [to_json(v) for v in results],
        })
    return 1 if (failures or counts["TIMEOUT"] or counts["SKIP"]
                 or invalid or refused) else 0


def run_style_fuzz_list(args):
    """Print generated statements and what zsh makes of them, without booting a
    single shell pair. This is how the GENERATOR is checked — a grammar bug
    should be caught here, in a second, not after an hour of pty boots."""
    outdir = os.path.join(REPO, "target", "parity-style-fuzz-%d" % args.seed)
    os.makedirs(outdir, exist_ok=True)
    only = None
    if args.style_fuzz_only:
        only = [s.strip() for s in args.style_fuzz_only.split(",") if s.strip()]
        unknown = [s for s in only if s not in GEN_STYLES]
        if unknown:
            sys.exit("--style-fuzz-only names unknown style(s): %s"
                     % ", ".join(unknown))
    rng = random.Random(args.seed)
    stmts = [gen_statement(rng, rng.choice(sorted(only or GEN_STYLES)))
             for _ in range(args.style_fuzz_list)]
    bad = dict(validate_config(stmts, args.zsh, outdir))
    print("# %d generated statement(s), seed=%d, validated against %s"
          % (len(stmts), args.seed, args.zsh))
    print()
    for s in stmts:
        print("%s" % s)
        if s in bad:
            print("    !! REJECTED BY zsh: %s" % bad[s])
    print()
    print("# %d accepted, %d rejected by zsh (a rejection is a GENERATOR bug)"
          % (len(stmts) - len(bad), len(bad)))
    return 1 if bad else 0


def fingerprint_doc(groups):
    out = {}
    for fp, vs in groups.items():
        rep = min(vs, key=lambda v: v.size())
        out[fp] = {
            "count": len(vs),
            "label": fp_label(rep),
            "representative": {
                "id": rep.id,
                "buffer": rep.case.buffer,
                "keys": rep.keys,
                "statements": list(rep.statements) if rep.statements else [],
            },
            "cells": [v.id for v in vs],
        }
    return out


def write_json(args, doc):
    text = json.dumps(doc, indent=2, sort_keys=False)
    if args.json == "-":
        print(text)
    else:
        with open(args.json, "w") as f:
            f.write(text + "\n")
        print("# json: %s" % args.json)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--zshrs", default=os.path.join(REPO, "target", "debug", "zshrs"))
    ap.add_argument("--zsh", default="zsh")
    ap.add_argument("--mode", choices=("native", "zsh"), default="native",
                    help="native: run the zshrs binary as-is (default). "
                         "zsh: run it with --zsh (emulation path).")
    ap.add_argument("--dump", default=None)
    ap.add_argument("--no-dump", action="store_true")
    ap.add_argument("--zstyle", default=None, help="zstyle fixture sourced by both shells")
    ap.add_argument("--corpus", default=None, help="file of case buffers, one per line")
    ap.add_argument("--case", default=None, help="single ad-hoc case buffer")
    ap.add_argument("--keys", default=None,
                    help="comma-separated keys per case (overrides --sequences)")
    ap.add_argument("--sequences", default=None,
                    help="comma-separated names from parity_corpus.KEY_SEQUENCES, "
                         "or 'default' / 'all'. Each case runs once per sequence.")
    ap.add_argument("--tag", default=None,
                    help="only run shared-corpus cases carrying this tag "
                         "(cmd, path, opt, sub, param, builtin, glob, ...)")
    ap.add_argument("--skip-optional", action="store_true",
                    help="drop cases tagged `optional` (need a binary that may be absent)")
    ap.add_argument("--discover", type=int, default=0, metavar="N",
                    help="ADD N cases discovered on this host: installed commands "
                         "that ship a `_name` completer in the live fpath. Sorted, "
                         "so the same N is the same set on the same machine.")
    ap.add_argument("--discover-all", action="store_true",
                    help="include commands whose completer may execute them "
                         "(reboot, shutdown, dd, ...) — see parity_corpus.DISCOVER_UNSAFE")
    ap.add_argument("--discover-only", action="store_true",
                    help="run ONLY the discovered cases, not the shared corpus too")
    ap.add_argument("--random-combos", type=int, default=0, metavar="N",
                    help="fuzz N random SUBSETS of --zstyle instead of running the "
                         "fixture as-is. Any subset is a valid config, and the bar "
                         "is that every one of them is byte-identical.")
    ap.add_argument("--combo-keep", type=float, default=0.5,
                    help="probability a statement survives into a random combo")
    ap.add_argument("--seed", type=int, default=0,
                    help="RNG seed — (seed, combo index) reproduces a combo exactly")
    ap.add_argument("--combo-cases", default=None,
                    help="restrict random-combo runs to these case names/buffers")
    ap.add_argument("--combo-sequence", default="tab1",
                    help="which key sequence random combos are judged on")
    ap.add_argument("--shrink", action="store_true", default=True,
                    help="delta-debug a diverging combo to its minimal statement set; "
                         "in --mutate mode, minimise the buffer and key sequence too")
    ap.add_argument("--no-shrink", dest="shrink", action="store_false")
    ap.add_argument("--shrink-probes", type=int, default=60,
                    help="max shrink re-runs per failing combo. In --mutate mode "
                         "this is the budget for ALL THREE dimensions together "
                         "(statements, keys, buffer) and the spend is reported.")
    # ── persistent fuzz corpus ──
    ap.add_argument("--corpus-dir", default=CORPUS_DIR,
                    help="directory of fuzz inputs (buffer + keys + zstyle subset), "
                         "one small JSON file each. Grows itself: a NEW failure "
                         "fingerprint's minimal reproducer is written back here.")
    ap.add_argument("--corpus-seed", action="store_true",
                    help="populate --corpus-dir from parity_corpus.CASES and "
                         "scripts/comptab_divergent_cases.txt, then exit")
    ap.add_argument("--seed-sequences", default="tab1,tab2,tab_down",
                    help="key sequences each seeded case is stored under")
    ap.add_argument("--corpus-origin", default=None, metavar="SUBSTR",
                    help="draw mutation parents only from corpus entries whose "
                         "`origin` contains SUBSTR (e.g. 'divergent-cases' to fuzz "
                         "around the mined divergences, 'promoted' to fuzz around "
                         "what this fuzzer has already found)")
    ap.add_argument("--mutate", type=int, default=0, metavar="N",
                    help="fuzz N inputs MUTATED from --corpus-dir (truncate/extend "
                         "the buffer, swap a key for its neighbour, add a filter "
                         "letter, retype the trailing word, add/remove a `-`, "
                         "drop/add a zstyle) instead of sampling from scratch")
    # ── generated zstyle VALUES ──
    ap.add_argument("--style-fuzz", type=int, default=0, metavar="N",
                    help="fuzz N GENERATED zstyle configurations instead of "
                         "subsets of --zstyle. The values themselves are "
                         "generated from the documented grammar (matcher-list "
                         "match specs, ordered completer chains, tag-order, "
                         "group-order, format, list-colors, menu, max-errors, "
                         "file-sort, ...), each at a randomly chosen context "
                         "specificity. Every statement is put to real zsh "
                         "first; one zsh refuses is counted as a GENERATOR bug "
                         "(INVALID-CONFIG), never as a finding and never as a "
                         "pass.")
    ap.add_argument("--style-fuzz-styles", type=int, default=4, metavar="N",
                    help="statements per generated configuration")
    ap.add_argument("--style-fuzz-mix", type=float, default=0.0, metavar="P",
                    help="also draw each --zstyle fixture statement with "
                         "probability P, so a generated config is COMPOSED "
                         "with a subset of the real one instead of replacing it")
    ap.add_argument("--style-fuzz-only", default=None, metavar="STYLES",
                    help="restrict generation to these comma-separated style "
                         "names (e.g. matcher-list,completer) — how a single "
                         "surface gets hammered")
    ap.add_argument("--style-fuzz-list", type=int, default=0, metavar="N",
                    help="print N generated statements with zsh's verdict on "
                         "each and exit, without booting any shell pair. The "
                         "generator's own self-check.")
    ap.add_argument("--timeout-recheck", action="store_true", default=True,
                    help="re-run a budget-exhausted cell ONCE serially, with every "
                         "other cell drained, before labelling it TIMEOUT. A clean "
                         "divergence on that re-run is promoted to FAIL.")
    ap.add_argument("--no-timeout-recheck", dest="timeout_recheck",
                    action="store_false")
    ap.add_argument("--skip-missing", action="store_true", default=None,
                    help="SKIP a case whose command is not installed here. ON by "
                         "default: without it, such a cell is scored PASS because "
                         "both shells complete nothing, which is a FAKE pass. "
                         "Skips are counted and printed, never counted as passes.")
    ap.add_argument("--no-skip-missing", dest="skip_missing", action="store_false",
                    help="run those cells anyway, scoring 'both completed nothing' "
                         "as a PASS. Only for reproducing a pre-flip number.")
    ap.add_argument("--rows", type=int, default=40)
    ap.add_argument("--cols", type=int, default=110)
    ap.add_argument("--settle", type=int, default=300)
    ap.add_argument("--boot-timeout", type=float, default=40.0)
    ap.add_argument("--confirm", type=int, default=1,
                    help="re-runs used to LABEL a failure flaky (never to pass it)")
    ap.add_argument("--jobs", type=int, default=1, metavar="N",
                    help="run N cells concurrently. Every cell is an independent "
                         "pty pair, so the comparison is unaffected, but load "
                         "slows a redraw and a marginal cell flips: two "
                         "back-to-back 113-cell sweeps at --jobs 4 --confirm 0 "
                         "disagreed on 3 cells. Keep --confirm on when running "
                         "in parallel so those get LABELLED flaky instead of "
                         "landing on whichever verdict the load produced.")
    ap.add_argument("--compare-attrs", action="store_true",
                    help="FAIL a cell whose rows match as text but differ in SGR "
                         "attributes (colour/bold). Reported either way.")
    ap.add_argument("--strict-cursor", action="store_true",
                    help="FAIL a cell whose final cursor position differs. Reported "
                         "either way.")
    ap.add_argument("--strict-stream", action="store_true",
                    help="FAIL a cell where one shell emitted a diagnostic the other "
                         "did not, even if the screens match. Reported either way.")
    ap.add_argument("--raw-diff", action="store_true", default=True,
                    help="on FAIL, diff the two raw escape-sequence streams")
    ap.add_argument("--no-raw-diff", dest="raw_diff", action="store_false")
    ap.add_argument("--raw-diff-lines", type=int, default=40)
    ap.add_argument("--max-diff-rows", type=int, default=12)
    ap.add_argument("--json", default=None, metavar="PATH",
                    help="write the full result document here ('-' for stdout)")
    ap.add_argument("--jsonl", default=None, metavar="PATH",
                    help="append one JSON object per cell as it finishes; survives "
                         "a killed run and can be tailed live")
    ap.add_argument("-v", "--verbose", action="store_true")
    args = ap.parse_args()

    if not os.path.exists(args.zshrs):
        sys.exit("zshrs binary not found: %s (cargo build --bin zshrs)" % args.zshrs)
    args.test_argv = ([args.zshrs, "-f", "-i"] if args.mode == "native"
                      else [args.zshrs, "--zsh", "-f", "-i"])

    if args.skip_missing is None:
        # ON by default. A case that names a command this host does not have
        # cannot reach a completer on EITHER shell, so both render nothing and
        # the cell was scored PASS — a pass that says nothing about parity and
        # that silently inflated every sweep (42 of the 3360 cells in the
        # default shared-corpus sweep on this host, 6 of the 513 fuzz-corpus
        # inputs). Flipping it can only ever move a cell from PASS to an
        # explicitly counted SKIP; it can never turn a FAIL into a pass, and a
        # SKIP is still a non-zero exit. `--no-skip-missing` reproduces the old
        # number.
        args.skip_missing = True

    dump = None if args.no_dump else resolve_dump(args.dump)
    fpath_dirs = user_fpath()
    env = child_env()

    if args.corpus_seed:
        pool = read_statements(args.zstyle) if args.zstyle else []
        inputs, written = corpus_seed(args, pool)
        print("# corpus dir : %s" % args.corpus_dir)
        print("# sequences  : %s" % args.seed_sequences)
        print("# statements : %d per input (from %s)"
              % (len(pool), args.zstyle or "<none>"))
        print("# candidates : %d (%d CASES x %d sequence(s) + %s)"
              % (len(inputs), len(CASES), len(args.seed_sequences.split(",")),
                 os.path.basename(DIVERGENT_FILE)))
        print("# written    : %d new file(s); corpus now holds %d input(s)"
              % (written, len(corpus_load(args.corpus_dir))))
        if args.mutate <= 0:
            return 0

    if args.style_fuzz_list > 0:
        return run_style_fuzz_list(args)

    if args.style_fuzz > 0:
        if args.combo_sequence not in KEY_SEQUENCES:
            sys.exit("unknown --combo-sequence: %s" % args.combo_sequence)
        return run_style_fuzz(args, env, dump, fpath_dirs)

    if args.mutate > 0:
        return run_mutate(args, env, dump, fpath_dirs)

    if args.random_combos > 0:
        if not args.zstyle:
            sys.exit("--random-combos needs --zstyle FIXTURE to draw subsets from")
        if args.combo_sequence not in KEY_SEQUENCES:
            sys.exit("unknown --combo-sequence: %s" % args.combo_sequence)
        return run_random_combos(args, env, dump, fpath_dirs)

    init_file = build_init(dump, fpath_dirs, args.zstyle)

    # Which key sequences each case is replayed under. `--keys` pins one
    # explicit sequence (the old single-shot behaviour); otherwise every case
    # runs once per named sequence from the shared corpus.
    if args.keys:
        seq_names = ["adhoc"]
        seq_keys = {"adhoc": [k.strip() for k in args.keys.split(",") if k.strip()]}
    else:
        sel = (args.sequences or "default").strip()
        if sel == "all":
            seq_names = list(KEY_SEQUENCES)
        elif sel == "default":
            seq_names = list(DEFAULT_SEQUENCES)
        else:
            seq_names = [s.strip() for s in sel.split(",") if s.strip()]
            unknown = [s for s in seq_names if s not in KEY_SEQUENCES]
            if unknown:
                sys.exit("unknown sequence(s): %s" % ", ".join(unknown))
        seq_keys = {n: KEY_SEQUENCES[n] for n in seq_names}

    # A key name that is not in KEYS and is not a single literal character is a
    # typo, and the old fallback transmitted the NAME as text. Reject it before
    # two shells spend a minute agreeing about nonsense.
    for name, keys in seq_keys.items():
        for k in keys:
            try:
                key_bytes(k)
            except UnknownKey:
                sys.exit("sequence %r names undefined key %r "
                         "(add it to parity_corpus.KEYS)" % (name, k))

    # Case set: ad-hoc, a corpus file, the shared corpus, host discovery, or
    # the shared corpus plus discovery.
    dropped_optional = 0
    if args.case is not None:
        cases = [adhoc_case(args.case)]
    elif args.corpus:
        with open(args.corpus) as f:
            cases = [adhoc_case(l.rstrip("\n")) for l in f
                     if l.strip() and not l.lstrip().startswith("#")]
    else:
        shared = cases_by_tag(args.tag)
        dropped_optional = 0
        if args.skip_optional:
            keep = [c for c in shared if "optional" not in c.tags]
            dropped_optional = len(shared) - len(keep)
            shared = keep
        cases = [] if args.discover_only else list(shared)
    if args.discover:
        found = discover_cases(args.discover, args.discover_all)
        have = {c.buffer for c in cases}
        cases += [c for c in found if c.buffer not in have]

    cells = [Cell(c, name, seq_keys[name], init_file)
             for c in cases for name in seq_names]

    print("# mode   : %s (%s)" % (args.mode, " ".join(args.test_argv)))
    print("# dump   : %s" % (dump or "<none>"))
    print("# init   : %s" % init_file)
    print("# zstyle : %s" % (args.zstyle or "<none>"))
    print("# geom   : %dx%d  settle=%dms" % (args.rows, args.cols, args.settle))
    print("# jobs   : %d" % max(1, args.jobs))
    print("# strict : attrs=%s cursor=%s stream=%s"
          % (args.compare_attrs, args.strict_cursor, args.strict_stream))
    print("# cases  : %d x %d sequence(s) = %d cell(s)"
          % (len(cases), len(seq_names), len(cells)))
    print("# seqs   : %s" % ", ".join(seq_names))
    if dropped_optional:
        print("# dropped: %d case(s) tagged `optional` (--skip-optional)"
              % dropped_optional)
    print()

    jsonl = open(args.jsonl, "w") if args.jsonl else None
    jsonl_lock = threading.Lock()
    started = time.monotonic()

    def on_done(v):
        if jsonl:
            with jsonl_lock:
                jsonl.write(json.dumps(to_json(v)) + "\n")
                jsonl.flush()

    stream = cell_stream(args, env, cells, on_done=on_done)

    passed = 0
    results = []
    failures = []
    timeouts = []
    skipped = []
    warned = attr_only = cursor_only = stream_only = 0
    try:
        for v in stream:
            results.append(v)
            # This exact line shape is parsed by scripts/gen_compsys_parity_report.py
            # (status, sequence, repr(buffer), optional "  (detail)") — keep it.
            # NOTE: that scraper's CELL_RE matches only PASS|FAIL|FLAKY, so a
            # TIMEOUT or SKIP row is currently dropped from its HTML table. The
            # summary line below still reports them (cells > passed + failed),
            # and they are in --json, but the report needs its regex widened to
            # show them per cell.
            label = "%-6s %-18s %r" % (v.status, v.seq, v.case.buffer)
            print(label + (("  (%s)" % v.detail) if v.detail else ""))
            warnings = ([("zsh", w) for w in (v.ref.warnings() if v.ref else [])]
                        + [("zshrs", w) for w in (v.test.warnings() if v.test else [])])
            if v.status == "PASS":
                passed += 1
                if warnings:
                    warned += 1
                    for who, w in warnings:
                        print("  ~ %s: %s" % (who, w))
                if v.attr_rows:
                    attr_only += 1
                    print("  ~ %d row(s) identical as text but differ in SGR attributes: %s"
                          % (len(v.attr_rows), ", ".join(map(str, v.attr_rows[:8]))))
                if v.cursor_differs:
                    cursor_only += 1
                    print("  ~ cursor differs: zsh %s vs zshrs %s"
                          % (v.ref.cursor, v.test.cursor))
                if v.ref_only_diags or v.test_only_diags:
                    stream_only += 1
                    for m in sorted(v.ref_only_diags)[:5]:
                        print("  ~ only zsh emitted: %s" % m)
                    for m in sorted(v.test_only_diags)[:5]:
                        print("  ~ only zshrs emitted: %s" % m)
                if args.verbose:
                    print(render(v.test.grid or []))
                sys.stdout.flush()
                continue
            if v.status == "TIMEOUT":
                timeouts.append(v)
                print_timeout(v, args)
                sys.stdout.flush()
                continue
            if v.status == "SKIP":
                skipped.append(v)
                sys.stdout.flush()
                continue
            failures.append(v)
            print_failure(v, args)
            sys.stdout.flush()
    finally:
        if jsonl:
            jsonl.close()

    elapsed = time.monotonic() - started
    # Consumed by gen_compsys_parity_report.py and parity_matrix.py — keep the
    # wording.
    print("\n# %d passed, %d failed, %d cell(s)" % (passed, len(failures), len(cells)))
    print("# categories: PASS=%d FAIL=%d TIMEOUT=%d SKIP=%d"
          % (passed, len(failures), len(timeouts), len(skipped)))
    print("# elapsed: %.1fs (%.1fs/cell)" % (elapsed, elapsed / max(1, len(cells))))
    if timeouts:
        # Named, counted, printed — and NOT a pass. The reason this category
        # exists is that at --jobs 8..10 roughly 80% of the "failures" in the
        # mined sweep were the debug build missing the per-key budget, which
        # made every parallel number untrustworthy in BOTH directions.
        print("# %d cell(s) ran out of MEASUREMENT budget (TIMEOUT): the screens "
              "were never both final, so they are neither divergences nor passes. "
              "Re-run them at --jobs 1." % len(timeouts))
        for v in timeouts[:20]:
            print("#   --case %s --keys %s   (%s)"
                  % (shlex.quote(v.case.buffer), ",".join(v.keys),
                     v.timeouts[0] if v.timeouts else "?"))
        if len(timeouts) > 20:
            print("#   ... %d more" % (len(timeouts) - 20))
    if skipped:
        print("# %d cell(s) SKIPPED (not run, not passed):" % len(skipped))
        for v in skipped[:20]:
            print("#   --case %s   (%s)" % (shlex.quote(v.case.buffer), v.skip_reason))
        if len(skipped) > 20:
            print("#   ... %d more" % (len(skipped) - 20))
    if warned:
        print("# %d pass(es) captured under a truncated or empty settle window "
              "— the screens agreed, but the capture is worth less than a clean one"
              % warned)
    if attr_only:
        print("# %d pass(es) differ in SGR attributes only (--compare-attrs to fail them)"
              % attr_only)
    if cursor_only:
        print("# %d pass(es) differ in final cursor position (--strict-cursor to fail them)"
              % cursor_only)
    if stream_only:
        print("# %d pass(es) where one shell emitted a diagnostic the other did not "
              "(--strict-stream to fail them)" % stream_only)
    if failures:
        # (sequence, buffer) so a failure is replayable verbatim. The bare
        # `--case/--keys` line is what parity_matrix.py scrapes; the full
        # command underneath it is what a human should paste.
        print("# failing cells:")
        for v in failures:
            print("#   --case %s --keys %s"
                  % (shlex.quote(v.case.buffer), ",".join(v.keys)))
        print("# failing cell ids: %s" % ", ".join(v.id for v in failures))
        groups = print_fingerprint_groups(failures, args)
    else:
        groups = {}

    if args.json:
        doc = {
            "schema": "comptab-parity/1",
            "mode": args.mode,
            "argv": sys.argv[1:],
            "zshrs": args.zshrs,
            "zsh": args.zsh,
            "dump": dump,
            "zstyle": args.zstyle,
            "geom": {"rows": args.rows, "cols": args.cols, "settle_ms": args.settle},
            "strict": {"attrs": args.compare_attrs, "cursor": args.strict_cursor,
                       "stream": args.strict_stream},
            "sequences": seq_names,
            "summary": {
                "passed": passed,
                "failed": len(failures),
                # A TIMEOUT is not in `failed` and never in `passed`: it is its
                # own category because it is not evidence either way.
                "timeout": len(timeouts),
                "skipped": len(skipped),
                "fingerprints": len(groups),
                "cells": len(cells),
                "warned": warned,
                "attr_only": attr_only,
                "cursor_only": cursor_only,
                "stream_only": stream_only,
                "dropped_optional": dropped_optional,
                "elapsed_seconds": round(elapsed, 1),
            },
            "fingerprints": fingerprint_doc(groups),
            "results": [to_json(v) for v in results],
        }
        write_json(args, doc)
    # A TIMEOUT or a SKIP is not byte-identical evidence, so neither exits 0.
    return 1 if (failures or timeouts or skipped) else 0


if __name__ == "__main__":
    sys.exit(main())

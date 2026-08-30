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

`--layout-fuzz` runs the same comparison over a different axis: not what is
typed or how the shell is configured, but where the completers are STORED and
how `compinit` is told to FIND them — `.zwc` digest composition and precedence,
fpath composition, the compinit flag matrix, the dump's state (including one
written by the OTHER shell), and compaudit's security conditions. Both shells
always get the byte-identical layout; a layout the reference zsh itself refuses
is counted as INVALID-LAYOUT and never run, exactly as `--style-fuzz` treats a
config zsh's `zstyle` rejects.

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
    scripts/comptab_parity.py --layout-fuzz 8      # 8 STORAGE/LOOKUP layouts
    scripts/comptab_parity.py --layout-list        # the layout catalog
    scripts/comptab_parity.py --dump-xshell        # cross-shell .zcompdump report

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


# Signals the harness itself sends during teardown. A child that ends on one of
# these says nothing about the shell (see `Session.close`).
_TEARDOWN_SIGNALS = (signal.SIGHUP, signal.SIGTERM, signal.SIGKILL)
# What a real crash ends on. SIGSEGV/SIGBUS are the memory faults; SIGILL,
# SIGFPE, SIGABRT and SIGTRAP are the other ways a shell dies of its own bug.
_CRASH_SIGNALS = (signal.SIGSEGV, signal.SIGBUS, signal.SIGILL, signal.SIGFPE,
                  signal.SIGABRT, signal.SIGTRAP, signal.SIGSYS)


def crash_note(cap):
    """How this shell DIED, or None if it did not.

    A crashed shell is not a slow shell. Before this existed, a reference zsh
    that took SIGSEGV produced no output, missed the boot budget, and was
    labelled "budget exhausted, not a divergence" — factually wrong, and the
    reason a real upstream zsh crash (`stripkshdef` <- `loadautofn`, faulting
    on a large `.zwc` digest in `fpath`) sat unnoticed across two rounds of
    sweeps. The evidence was already in hand on both sides: the child's
    `waitpid` status, and the crash markers `Session.crashed()` scans the pty
    text for.
    """
    if cap is None:
        return None
    st = cap.status
    if st is not None and os.WIFSIGNALED(st):
        sig = os.WTERMSIG(st)
        if sig in _CRASH_SIGNALS:
            try:
                name = signal.Signals(sig).name
            except ValueError:
                name = "?"
            return "died on signal %d (%s)" % (sig, name)
        if sig not in _TEARDOWN_SIGNALS:
            return "died on signal %d" % sig
    if cap.crash:
        return "crash marker(s) on the terminal: %s" % ", ".join(cap.crash)
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
        # Which side died, and how. Set only on REF-CRASHED / TEST-CRASHED,
        # which — exactly like TIMEOUT — is neither a pass nor a divergence: a
        # shell that crashed produced no comparison to judge.
        self.crashes = []
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
        "crashes": [{"side": side, "note": note} for side, note in v.crashes],
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

    # A crash outranks a budget label: a dead shell exhausts every budget it
    # was given, so checking TIMEOUT first would keep mislabelling it. It also
    # outranks FAIL — the two screens were never both produced, so the row diff
    # describes one shell's output against a blank, not a divergence.
    ref_crash, test_crash = crash_note(ref), crash_note(test)
    if ref_crash or test_crash:
        v.status = "REF-CRASHED" if ref_crash else "TEST-CRASHED"
        v.crashes = [("zsh", ref_crash)] if ref_crash else []
        if test_crash:
            v.crashes.append(("zshrs", test_crash))
        v.detail = "; ".join("%s %s" % (side, note) for side, note in v.crashes)
        v.duration = time.monotonic() - t0
        return v

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


# Categories that are NOT a pass and NOT a divergence. Each is counted, printed
# and keeps the exit status non-zero; none is ever folded into PASS.
CRASH_STATUSES = ("REF-CRASHED", "TEST-CRASHED")


def print_crash_counts(counts):
    """The lines a summary adds when a shell died. Silent when none did, so no
    existing output changes on a run with no crashes."""
    if counts.get("REF-CRASHED"):
        print("# %d cell(s) where the REFERENCE zsh CRASHED — an upstream zsh "
              "bug, not a zshrs divergence, and not a measurement failure "
              "either. No comparison happened; never scored as a pass."
              % counts["REF-CRASHED"])
    if counts.get("TEST-CRASHED"):
        print("# %d cell(s) where ZSHRS CRASHED. Not fingerprinted as a "
              "divergence (there was no second screen to diverge from), but it "
              "is a zshrs bug and the repro is printed above."
              % counts["TEST-CRASHED"])


def crashed(counts):
    return sum(counts.get(k, 0) for k in CRASH_STATUSES)


def print_crash(v, args):
    """A crashed shell is not a bug report about parity — it is a bug report
    about that shell, so it gets what is needed to chase it there."""
    print("  --- a shell CRASHED (no verdict on parity) ---")
    for side, note in v.crashes:
        print("  ! %s %s" % (side, note))
    if any(side == "zsh" for side, _n in v.crashes):
        print("  ! the REFERENCE shell died. This is an upstream zsh bug, not a "
              "zshrs divergence; the comparison never happened.")
    print("  ! the OS may have written a report: "
          "~/Library/Logs/DiagnosticReports/ (macOS), coredumpctl (systemd)")
    for side, cap in (("zsh", v.ref), ("zshrs", v.test)):
        if cap is not None and cap.reason:
            print("  ! %s also reported: %s" % (side, cap.reason))
    print("  --- repro ---")
    print("  %s" % repro_cmd(args, v.case.buffer, v.keys,
                             zstyle=v.zstyle_file))


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
    # An entry the coverage-guided run kept because it reached a screen shape
    # or an engine path nothing in the corpus had reached before. It has no
    # fingerprint (it did not fail), but it is evidence of new territory, so it
    # outranks a seed that has only ever produced what everything else does.
    if inp.origin.startswith("cov/"):
        return 3
    return 1


def mutate_input(inp, rng, pool, mut_weights=None):
    """One or two small edits applied to a corpus entry.

    `mut_weights` is the coverage-guided schedule's only reach into mutation:
    a per-mutator weight vector, in `MUTATORS(pool)` order, so a mutation kind
    that has been buying features gets drawn more often. `None` (every caller
    that existed before guidance) keeps the uniform draw exactly as it was.
    """
    buf, keys, stmts = inp.buffer, list(inp.keys), list(inp.statements)
    muts = MUTATORS(pool)
    applied = []
    for _ in range(rng.choice((1, 1, 2))):
        m = (rng.choices(muts, mut_weights)[0] if mut_weights
             else rng.choice(muts))
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
        elif v.status in ("REF-CRASHED", "TEST-CRASHED"):
            print_crash(v, args)
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
    print("# categories: PASS=%d FAIL=%d FLAKY=%d TIMEOUT=%d SKIP=%d "
          "REF-CRASHED=%d TEST-CRASHED=%d"
          % (counts["PASS"], counts["FAIL"], counts["FLAKY"],
             counts["TIMEOUT"], counts["SKIP"],
             counts.get("REF-CRASHED", 0), counts.get("TEST-CRASHED", 0)))
    print_crash_counts(counts)
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
    return 1 if (failures or counts["TIMEOUT"] or counts["SKIP"]
                 or crashed(counts)) else 0


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
        elif v.status in ("REF-CRASHED", "TEST-CRASHED"):
            print_crash(v, args)
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
          "INVALID-CONFIG=%d REF-REFUSED=%d REF-CRASHED=%d TEST-CRASHED=%d"
          % (counts["PASS"], counts["FAIL"], counts["FLAKY"], counts["TIMEOUT"],
             counts["SKIP"], len(invalid), len(refused),
             counts.get("REF-CRASHED", 0), counts.get("TEST-CRASHED", 0)))
    print_crash_counts(counts)
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
                 or invalid or refused or crashed(counts)) else 0


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


# ── feedback: what did this cell TEACH the fuzzer? ───────────────────────────
#
# Everything above this line is BLIND. `--mutate` draws a weighted parent and
# re-rolls the mutator dice; `--style-fuzz` re-rolls the generator's dice. Both
# spend one pty pair (~5-19s) per input and keep an input only if it FAILED.
# An input that passed is thrown away even when it was the first input in the
# corpus's history to reach `_approximate`, render a described listing, or draw
# a menu — so the fuzzer cannot become better at reaching new code than it was
# on its first run, and a thousand inputs that all drive `_complete` -> `_files`
# cost a thousand pty boots and buy one bit.
#
# A coverage-guided fuzzer fixes this by keeping an input that produced a NEW
# EDGE, failure or not. That needs an execution signal. What zshrs actually
# exposes, measured (see `SIGNAL` below), is two of them:
#
#   1. OUTPUT SHAPE — free, always available, and symmetric: it is computed
#      from the two `Capture`s the harness already takes, so it costs nothing,
#      cannot perturb the run, works at any `--jobs`, and describes the
#      REFERENCE shell as well as zshrs. It is a proxy for the code path, not
#      the code path: two inputs with the same shape may still have taken
#      different routes to it. It is the default.
#
#   2. ENGINE TRACE — real execution feedback, opt-in (`--cov-log`), serial
#      only. zshrs's `compsys_args` tracing target names the completer that
#      resolved, the tag context, the `_arguments` gate outcome, the
#      `addmatches` candidate loop and the `do_completion` branch taken. That
#      IS the code path. Its cost is the constraint, not the CPU: see below.
#
# ─ SIGNAL: what settled the choice ───────────────────────────────────────────
#
# `src/extensions/log.rs:53-67` — the log directory is `$ZSHRS_HOME`, else
# `$HOME/.zshrs`, and the file is unconditionally `zshrs.log`. There is no
# separate path knob: the ONLY way to redirect the log is `ZSHRS_HOME`, which
# also relocates `compsys.db` (194 MB here), `autoloads.rkyv`, `plugins.db`,
# `zshrs.toml` and the history db. A child pointed at a scratch `ZSHRS_HOME` is
# a cold, unconfigured shell — a DIFFERENT shell from the one under test — so
# redirecting the log to isolate it would silently change what is being
# compared. Rejected on that ground alone.
#
# `src/extensions/log.rs:163` — the level comes from `$ZSHRS_LOG` (NOT
# `RUST_LOG`: `strings target/debug/zshrs | grep -c RUST_LOG` == 0), defaulting
# to `info`. `child_env` already forwards it, so no new plumbing is needed.
#
# Measured, `ZSHRS_LOG=debug` on `--case 'git ' --keys tab`: 70 lines, 10773
# bytes, of which 22 carry the `compsys_args` target and name the path taken.
# `compsys_args` is a dedicated target (39 `tracing::debug!` sites), so
# `ZSHRS_LOG=compsys_args=debug` narrows the same 8306 bytes to nothing BUT the
# completion trace —
#
#     compsys_args: zlecore widget resolution name=expand-or-complete variant="Comp"
#     compsys_args: get_comp_string result s=Some("") wb=4 we=4 lincmd=0 inwhat=0
#     compsys_args: _dispatch resolution argv=[...] comp=_git name=git last_arg=-default-
#     compsys_args: _tags default sort done order=["common-commands"] ... ctx=:completion::complete:git:argument-1:
#     compsys_args: _arguments options gate requested_options=0 hasopts=false matched=false aret=true
#     compsys_args: addmatches candidate-loop done added=23 mnum=23 doadd=true
#     compsys_args: makecomplist RETURN nm=23 nmsg=0 errset=false
#     compsys_args: do_completion branch point nm=23 dm=1 useline=1 uselist=3 iforcemenu=0
#
# `ZSHRS_LOG` is also the gate for `ftime` (src/extensions/ftime.rs:21 — a
# SUBSTRING test on the same variable, so `compsys_args=debug,ftime` turns on
# both). `ftime` instruments `dispatch_function_call` (src/ported/exec.rs:8573)
# and a Drop guard in `docomplete` (src/ported/zle/zle_tricky.rs:733-739) dumps
# the per-function inclusive times to /tmp/ftime.log once per completion —
#
#      total_ms   calls  name (inclusive)
#       336.063       1  _normal
#       325.332       1  _dispatch
#         8.683       1  _set_command
#         2.773       5  _tags
#         2.328       2  _next_label
#
# The NAMES are the point: that is the set of compsys shell functions this
# completion actually entered, which is function-level edge coverage and the
# single most direct signal available. The file is rewritten (not appended) per
# completion, so it needs no slicing at all — only the same `--jobs 1`, since
# the path is hardcoded.
#
# Cost is NOT the objection — three runs each, same cell: 4.74 / 4.72 / 4.73 s
# without, 4.61 / 4.73 / 4.77 s with. 70 log writes are nothing against a 4.7 s
# pty boot, so turning it on does not push a cell towards the TIMEOUT budget.
#
# ATTRIBUTION is the objection. The log is one shared append-only file (461 MB
# on this host) and only the single `zshrs starting ... pid=N` line carries a
# pid — every other line is `TS LEVEL <thread> <target>: ...` with the thread
# named `main` in every process. So lines cannot be attributed to a process
# after the fact. Slicing by byte offset around a cell is exact only while this
# harness is the sole writer: with `--jobs > 1` this harness's OWN children
# interleave, and on a busy host a peer zshrs would too (measured ambient
# growth here: 0 bytes / 8 s, but that is a reading, not a guarantee).
#
# Hence: `--cov-log` is opt-in, forces `--jobs 1`, and every slice is reported
# with the number of shell boots it contains so contamination is visible rather
# than silently folded into the coverage set. Shape coverage carries the run
# when it is off.
#
# Everything else the binary exposes was checked and is not a per-completion
# signal: `--doctor` is a one-shot environment report (and refuses under
# `--zsh`), `zprof`/`--features profiling|flamegraph|prometheus` are compile-time
# (`zshrs --doctor` prints "disabled (build with --features ...)"), and the three
# env-gated diagnostics that DO take a path — `ZSHRS_COMPLIST_LOG`
# (src/ported/zle/complist.rs:3704, compresult.rs:3977), `ZSHRS_CAPDBG`
# (src/compsys/in_editor.rs:223), `ZSHRS_CSDBG` -> hardcoded /tmp/cs.log
# (src/compsys/ported/Base/Utility/_call_program.rs:167) — are three narrow
# probes, not coverage. The first two are folded in when `--cov-log` is on,
# since they cost nothing extra and DO get a per-cell path.
#
# ─ THE RULE THIS CODE MUST NOT BREAK ─────────────────────────────────────────
#
# Guidance decides WHICH inputs are run. It must never touch how a run input is
# JUDGED. Nothing below reads or writes `Verdict.status`, `fingerprint`,
# `timeouts` or `skip_reason`; `run_cell` is called unmodified, promotion of a
# new failure fingerprint is the same code `--mutate` uses, and TIMEOUT / SKIP
# / INVALID-CONFIG keep every property they had. A guided run and a blind run
# reach identical verdicts on identical inputs — `--guide-off` runs the same
# loop with the feedback disabled precisely so that can be measured.

# Coarse, monotone buckets. Exact counts would make almost every cell unique,
# which is indistinguishable from having no signal: the set of seen features
# would grow linearly with the number of cells and "new feature" would stop
# meaning anything. Buckets make "the listing got materially bigger" the
# observation instead of "the listing has 24 rows rather than 23".
_SHAPE_BUCKETS = (0, 1, 2, 3, 5, 8, 13, 21, 34, 55)

# A row carrying an interior run of 2+ spaces between two non-blanks is a
# COLUMNED row — a described completion listing (`_describe`, `verbose`), or a
# multi-column match list. Distinguishing that from a plain listing is most of
# what "did the display shape change" means for compsys.
_SHAPE_COLGAP_RE = re.compile(r"\S {2,}(?=\S)")


def _bucket(n):
    for b in _SHAPE_BUCKETS:
        if n <= b:
            return str(b)
    return "%d+" % _SHAPE_BUCKETS[-1]


def side_shape(side, cap):
    """The SHAPE of one shell's rendered result, as a set of feature strings.

    Deliberately about the display's structure, never its correctness: a
    feature fires the same way whether the shell was right or wrong. That is
    what keeps guidance and judging separate.
    """
    if cap is None:
        return {"%s/absent" % side}
    out = set()
    if cap.reason:
        out.add("%s/reason/%s" % (side, fp_normalize(cap.reason)[:60]))
    for m in cap.diags:
        out.add("%s/diag/%s" % (side, fp_normalize(m)[:60]))
    if cap.grid is None:
        out.add("%s/nogrid" % side)
        return out

    rows = [r.rstrip() for r in cap.grid]
    filled = [r for r in rows if r.strip()]
    out.add("%s/rows/%s" % (side, _bucket(len(filled))))

    # More than one prompt on screen means the shell redrew one — the
    # duplicate-prompt and trailing-prompt families of bug live here.
    out.add("%s/prompts/%s" % (side, _bucket(sum(r.count(SENTINEL) for r in rows))))

    # Anything drawn below the command line that is not itself a prompt.
    listing = [r for r in filled[1:] if SENTINEL not in r]
    out.add("%s/list/%s" % (side, _bucket(len(listing))))
    gaps = max((len(_SHAPE_COLGAP_RE.findall(r)) for r in listing), default=0)
    out.add("%s/desc/%s" % (side, "yes" if gaps else "no"))
    if gaps:
        out.add("%s/cols/%s" % (side, _bucket(gaps + 1)))

    # What the completion did to the line itself — the effect, as opposed to
    # the display. `lineclass` is the masked text, so `ls foo` and `ls bar`
    # collapse but `ls -` and `ls foo` do not.
    line0 = rows[0] if rows else ""
    if SENTINEL in line0:
        typed = line0.split(SENTINEL, 1)[1].strip()
        out.add("%s/line/%s" % (side, _bucket(len(typed))))
        out.add("%s/lineclass/%s" % (side, fp_normalize(typed)[:40]))

    if cap.cursor:
        out.add("%s/cur/row%s" % (side, _bucket(cap.cursor[0])))
        out.add("%s/cur/col%s" % (side, _bucket(cap.cursor[1] // 8)))

    # Distinct SGR signatures on screen: a menu selection highlight, a
    # `list-colors` run and a plain listing are three different numbers here
    # and identical as text.
    if cap.attrs:
        sigs = {c for row in cap.attrs for c in row}
        out.add("%s/attrs/%s" % (side, _bucket(len(sigs))))
    return out


def shape_features(v):
    """The shape of a whole cell: both sides, plus how they related.

    `status/` is included because reaching a TIMEOUT or a SKIP for the first
    time on a given class of input IS new information about where the budget
    goes — but it is a feature, not a verdict, and nothing here feeds back into
    what the cell was scored.
    """
    out = {"status/%s" % v.status}
    out |= side_shape("zsh", v.ref)
    out |= side_shape("zshrs", v.test)
    out.add("diff/rows/%s" % _bucket(len(v.diffs)))
    if v.diffs:
        row, a, b = v.diffs[0]
        out.add("diff/first/%s/col%d" % (fp_row_class(row), first_diff_cell(a, b) // 10))
    if v.attr_rows:
        out.add("diff/attrs/%s" % _bucket(len(v.attr_rows)))
    if v.cursor_differs:
        out.add("diff/cursor")
    return out


# ── engine trace features (--cov-log) ────────────────────────────────────────

# `TS LEVEL <span> <span> ... <target>: <message> <field>=<value> ...`
_ENG_LINE_RE = re.compile(r"^\S+Z\s+(?:TRACE|DEBUG|INFO|WARN|ERROR)\s+(.*)$")
# The head of a message is everything before the first `ident=` field, which is
# where the per-case detail starts.
_ENG_HEAD_RE = re.compile(r"^(.*?)(?:\s+[a-z_][a-z0-9_]*=|$)")
# Fields whose VALUE names a code path rather than a datum. A completer name, a
# tag context or a widget variant is an edge; `nm=23` is not.
_ENG_PATH_FIELDS = ("comp", "ctx", "context", "variant", "fn_name", "func_name",
                    "widget", "group", "exp", "linwhat", "inwhat", "branch")
_ENG_FIELD_RE = re.compile(
    r"\b(%s)=(\"[^\"]*\"|\S+)" % "|".join(_ENG_PATH_FIELDS))
# Only targets that say something about the completion engine. `zsh::lowfd`
# registering a descriptor is noise that fires identically on every cell.
_ENG_TARGETS = ("compsys_args", "zsh::compsys", "zshrs::compsys",
                "zsh::ext_builtins", "zsh::plugin_cache")


def engine_features(text):
    """Features from one cell's slice of zshrs's own tracing log."""
    out = set()
    for line in text.splitlines():
        m = _ENG_LINE_RE.match(line)
        if not m:
            continue
        body = m.group(1)
        # Keep the LAST target on the line: `DEBUG main execute_script:
        # compsys_args: ...` is a span then a target, and the target is the one
        # that names the subsystem.
        best = None
        for t in _ENG_TARGETS:
            i = body.rfind(t + ": ")
            if i >= 0 and (best is None or i > best[0]):
                best = (i, t)
        if best is None:
            continue
        msg = body[best[0] + len(best[1]) + 2:]
        head = _ENG_HEAD_RE.match(msg).group(1)
        out.add("eng/%s/%s" % (best[1], fp_normalize(head)[:80]))
        for name, val in _ENG_FIELD_RE.findall(msg):
            out.add("eng/f/%s=%s" % (name, fp_normalize(val.strip('"'))[:60]))
    # The two env-gated per-path probes, folded in for free when they are on.
    return out


# src/extensions/ftime.rs:70 — hardcoded, rewritten per completion.
FTIME_PATH = "/tmp/ftime.log"
_FTIME_ROW_RE = re.compile(r"^\s*[\d.]+\s+(\d+)\s+(\S+)")


def ftime_features(text):
    """Which compsys shell functions this completion entered, and how hard.

    The closest thing to real edge coverage this binary offers without a custom
    build: one row per function that `dispatch_function_call` saw. The call
    COUNT is bucketed in as well — `_tags` called once and `_tags` called five
    times are different traversals of `_arguments`.
    """
    out = set()
    for line in text.splitlines():
        m = _FTIME_ROW_RE.match(line)
        if m:
            out.add("fn/%s" % m.group(2))
            out.add("fn/%s/x%s" % (m.group(2), _bucket(int(m.group(1)))))
    return out


def probe_features(text, kind):
    """Features from `ZSHRS_COMPLIST_LOG` / `ZSHRS_CAPDBG`, which take a PATH
    and so are already per-cell — no slicing, no attribution problem."""
    out = set()
    for line in text.splitlines():
        out.add("%s/%s" % (kind, fp_normalize(line)[:80]))
    return out


def log_path_for(env):
    """Where the child under `env` will write `zshrs.log`.

    src/extensions/log.rs:53-67 — `$ZSHRS_HOME` else `$HOME/.zshrs`, then
    `zshrs.log`. Mirrored rather than assumed, because the harness builds the
    child env from scratch and pins HOME itself.
    """
    home = env.get("ZSHRS_HOME") or os.path.join(
        env.get("HOME", os.path.expanduser("~")), ".zshrs")
    return os.path.join(home, "zshrs.log")


def _read_slice(path, start, end):
    try:
        with open(path, "rb") as f:
            f.seek(start)
            return f.read(max(0, end - start)).decode("utf-8", "replace")
    except OSError:
        return ""


def _drain(path):
    """Read and remove a per-cell probe file."""
    try:
        with open(path, "r", errors="replace") as f:
            text = f.read()
        os.unlink(path)
        return text
    except OSError:
        return ""


# ── input classes, and the yield each one has actually earned ────────────────
#
# Task: spend pty boots where they pay. To do that at all, an input has to be
# CLASSIFIABLE — the fuzzer has to be able to say "inputs like this one" — and
# the classes have to be the axes the generator can actually steer. Four:
#
#   style:<name>  the zstyle surface a config exercises (matcher-list,
#                 completer, tag-order, ...). This is the axis the round-2
#                 value generator opened up.
#   case:<class>  what the buffer is completing (a command word, an option, a
#                 path, a parameter, a subcommand) — derived from the buffer so
#                 it works for a mutated ad-hoc case, which carries no tags.
#   keys:<sig>    the key path through ZLE (one tab, two tabs, tab then menu
#                 navigation, a filter letter, ...).
#   mut:<kind>    the mutation that produced the input.
#
# The table below is deliberately a RATE, not a count: a class that finds two
# features in 90 s is worse than one that finds one in 5 s, and the whole point
# is to allocate seconds. Both numerator and denominator get a prior, so an
# untried class scores like a mediocre one rather than like a dead one — that
# plus the explore floor is what stops a class being starved permanently on the
# strength of one unlucky sample.

_STYLE_NAME_RE = re.compile(r"^\s*zstyle\s+(?:-\S+\s+)*(?:'[^']*'|\"[^\"]*\"|\S+)\s+(\S+)")


def statement_styles(stmts):
    out = set()
    for s in stmts or ():
        m = _STYLE_NAME_RE.match(s)
        if m:
            out.add(m.group(1).strip("'\""))
    return out


def buffer_class(buf):
    """What the trailing word of a buffer is asking the completion system for.

    Mirrors the shared corpus's tag vocabulary (cmd / opt / path / param / sub)
    without needing the case object, because a mutated buffer is an ad-hoc case
    with no tags at all.
    """
    tail = buf.split()[-1] if buf.split() else ""
    trailing_space = buf != buf.rstrip()
    if not buf.strip():
        return "empty"
    if trailing_space:
        tail = ""
    if tail.startswith("$"):
        return "param"
    if tail.startswith("-"):
        return "opt"
    if "/" in tail or tail.startswith("~"):
        return "path"
    if "=" in tail:
        return "assign"
    if len(buf.split()) <= 1 and not trailing_space:
        return "cmd"
    return "sub"


_MUT_ORIGIN_RE = re.compile(r"^mutate\(([^)]*)\)")


def input_classes(inp):
    """The classes one input belongs to. An input belongs to several — it has a
    buffer AND a key path AND a style set — and the schedule scores it on all
    of them, so a productive style can lift a mediocre buffer and vice versa."""
    out = ["case:%s" % buffer_class(inp.buffer),
           "keys:%s" % ",".join(inp.keys[:4])]
    m = _MUT_ORIGIN_RE.match(inp.origin or "")
    for kind in (m.group(1).split("+") if m else ["seed"]):
        out.append("mut:%s" % kind)
    for s in sorted(statement_styles(inp.statements))[:4]:
        out.append("style:%s" % s)
    if not inp.statements:
        out.append("style:<none>")
    return out


class YieldTable:
    """Per-class reward rate, and the sampling weight that follows from it."""

    # A new fingerprint is the thing the fuzzer exists to find; a new feature is
    # progress towards one. Weighted 4:1 so a class that finds bugs outranks a
    # class that merely finds novel screens.
    FP_WEIGHT = 4.0

    def __init__(self, prior_reward=1.0, prior_secs=8.0):
        self.prior_reward = prior_reward
        self.prior_secs = prior_secs
        self.cells = {}
        self.secs = {}
        self.feats = {}
        self.fps = {}

    def observe(self, classes, secs, new_feats, new_fps):
        for c in classes:
            self.cells[c] = self.cells.get(c, 0) + 1
            self.secs[c] = self.secs.get(c, 0.0) + secs
            self.feats[c] = self.feats.get(c, 0) + new_feats
            self.fps[c] = self.fps.get(c, 0) + new_fps

    def score(self, cls):
        reward = self.feats.get(cls, 0) + self.FP_WEIGHT * self.fps.get(cls, 0)
        return ((reward + self.prior_reward)
                / (self.secs.get(cls, 0.0) + self.prior_secs))

    def weight(self, classes):
        """One input's sampling weight: the MEAN of its classes' rates.

        Mean, not product: a product would let one cold class veto an input
        that is otherwise in the hottest region of the corpus, and would make
        the weight depend on how many classes an input happens to have.
        """
        if not classes:
            return self.score("<none>")
        return sum(self.score(c) for c in classes) / len(classes)

    def rows(self):
        out = []
        for c in sorted(set(self.cells)):
            out.append((c, self.cells[c], self.secs[c], self.feats[c],
                        self.fps[c], self.score(c)))
        # Hottest first — the head of this table is the finding.
        out.sort(key=lambda r: (-r[5], r[0]))
        return out


def print_yield_table(table, limit=28):
    rows = table.rows()
    if not rows:
        return
    print("# per-class yield — where the budget went and what it bought")
    print("#   %-28s %5s %8s %6s %5s %9s"
          % ("class", "cells", "secs", "feats", "fps", "rate/s"))
    for c, cells, secs, feats, fps, score in rows[:limit]:
        print("#   %-28s %5d %8.1f %6d %5d %9.4f"
              % (c[:28], cells, secs, feats, fps, score))
    if len(rows) > limit:
        print("#   ... %d more class(es)" % (len(rows) - limit))
    dead = [c for c, _n, _s, f, p, _sc in rows if f == 0 and p == 0]
    if dead:
        print("# %d class(es) bought nothing this run: %s"
              % (len(dead), ", ".join(dead[:8]) + (" ..." if len(dead) > 8 else "")))


# ── the guided loop ──────────────────────────────────────────────────────────

def _cov_note(feats, n_new):
    return "coverage: %d feature(s), %d new" % (len(feats), n_new)


def run_guided(args, env, dump, fpath_dirs):
    """Coverage-guided fuzzing over the persistent corpus.

    The difference from `--mutate` is one sentence: an input is RETAINED when
    it produced information, not only when it failed. Everything about how a
    cell is judged is `--mutate`'s code, unchanged and untouched.

    The loop, per step:
      1. choose a parent — uniformly with probability `--explore-floor`, else
         weighted by `corpus_weight` x the parent's classes' observed yield;
      2. mutate it, choosing the mutator the same way;
      3. run the cell through the ordinary `run_cell`;
      4. extract features (shape always; engine trace under `--cov-log`);
      5. retain the input in the corpus if it carried a feature never seen
         before in this run, and promote+shrink it if it carried a failure
         fingerprint the corpus has never recorded — the existing rule;
      6. credit the observation to every class the input belonged to.
    """
    pool = read_statements(args.zstyle) if args.zstyle else []
    outdir = os.path.join(REPO, "target", "parity-guided-%d" % args.seed)
    os.makedirs(outdir, exist_ok=True)

    corpus = corpus_load(args.corpus_dir)
    if not corpus:
        _, written = corpus_seed(args, pool)
        print("# corpus was empty — seeded %d input(s) into %s"
              % (written, args.corpus_dir))
        corpus = corpus_load(args.corpus_dir)
    if not corpus:
        sys.exit("fuzz corpus is empty and could not be seeded: %s" % args.corpus_dir)

    if args.corpus_origin:
        picked = [i for i in corpus if args.corpus_origin in i.origin]
        if not picked:
            sys.exit("--corpus-origin %r matches none of the %d corpus input(s)"
                     % (args.corpus_origin, len(corpus)))
    else:
        picked = list(corpus)

    known_fps = {i.fingerprint for i in corpus if i.fingerprint}
    guided = not args.guide_off
    rng = random.Random((args.seed << 24) ^ (args.guided * 7919))

    # Engine coverage needs the log slice attributed to one child, which is
    # only sound while this harness is the log's sole writer.
    logpath = None
    # The env a PLAIN run uses. `--cov-log` has to put `ZSHRS_LOG` (and the two
    # probe paths) into the child environment, and a completion that enumerates
    # the environment — `tee >(<TAB><TAB>` reaches the parameter listing — then
    # renders those variables as MATCHES. Both shells get the identical dict so
    # a PASS is still a valid PASS, but a reproducer minted under instrumented
    # env would carry `ZSHRS_LOG -- compsys_args=debug,ftime` in its own
    # fingerprint and would NOT reproduce from the `replay:` command printed
    # next to it. Measured: one such fingerprint was manufactured on `tee >(`
    # before this was added. So promotion re-measures on THIS env, and only a
    # divergence that survives without the instrumentation is written to the
    # corpus.
    base_env = env
    if args.cov_log:
        env = dict(env)
        # The narrow target filter, not `debug`: 8306 bytes of pure completion
        # trace instead of 10773 bytes of mostly `zsh::lowfd` noise, and
        # `ftime` rides on the same variable.
        env.setdefault("ZSHRS_LOG", "compsys_args=debug,ftime")
        logpath = log_path_for(env)
        print("# --cov-log: reading zshrs's own trace from %s (ZSHRS_LOG=%s)"
              % (logpath, env["ZSHRS_LOG"]))
        print("#            + per-function coverage from %s" % FTIME_PATH)
        _drain(FTIME_PATH)          # a stale dump is not this run's coverage

    table = YieldTable()
    seen = set()
    mutators = MUTATORS(pool)
    # The loop is sequential by construction: the whole point is that cell N+1
    # is chosen with cell N's result in hand, so there is nothing for --jobs to
    # parallelise. One gate for the whole run, as `cell_stream` would build.
    gate = SerialGate()

    print("# coverage-guided fuzz%s" % ("" if guided else "  [--guide-off: BLIND control run]"))
    print("# corpus : %s (%d input(s), %d known fingerprint(s))"
          % (args.corpus_dir, len(corpus), len(known_fps)))
    print("# parents: %d drawn from%s" % (len(picked),
          "" if not args.corpus_origin else "  --corpus-origin %r" % args.corpus_origin))
    print("# cells  : %d   seed=%d   explore-floor=%.2f   fixture=%s (%d statement(s))"
          % (args.guided, args.seed, args.explore_floor,
             args.zstyle or "<none>", len(pool)))
    print("# signal : shape%s"
          % ("  +  engine trace (compsys_args) + ftime function coverage"
             if args.cov_log else ""))
    print("# mode   : %s (%s)" % (args.mode, " ".join(args.test_argv)))
    print("# jobs   : 1 (a feedback loop is sequential by construction)   "
          "shrink=%s probes<=%d" % (args.shrink, args.shrink_probes))
    print("# outdir : %s" % outdir)
    print()

    counts = {"PASS": 0, "FAIL": 0, "FLAKY": 0, "TIMEOUT": 0, "SKIP": 0}
    failures, results = [], []
    retained, promoted, artifacts, tried = 0, 0, 0, set()
    curve = []                      # (cell index, cumulative distinct features)
    contaminated = 0

    def pick_parent():
        if not guided or rng.random() < args.explore_floor:
            return rng.choice(picked), "explore"
        w = [corpus_weight(i) * table.weight(input_classes(i)) for i in picked]
        if not any(w):
            return rng.choice(picked), "explore"
        return rng.choices(picked, w)[0], "exploit"

    def pick_mutator():
        if not guided or rng.random() < args.explore_floor:
            return None             # mutate_input picks uniformly, as always
        w = [table.score("mut:%s" % m.__name__.replace("_mut_", ""))
             for m in mutators]
        return w if any(w) else None

    for n in range(args.guided):
        parent, how = pick_parent()
        inp = None
        for _ in range(40):
            cand = mutate_input(parent, rng, pool, mut_weights=pick_mutator())
            if cand.key() not in tried:
                inp = cand
                break
        if inp is None:
            inp = mutate_input(parent, rng, pool)
        tried.add(inp.key())
        classes = input_classes(inp)

        zfile = write_statements(inp.statements, outdir, "gui%04d" % n)
        cell = Cell(adhoc_case(inp.buffer), "gui%04d" % n, inp.keys,
                    build_init(dump, fpath_dirs, zfile),
                    inp.statements, zfile, inp.origin)

        if logpath:
            # Per-cell probe files: these take a PATH, so they need no slicing.
            env["ZSHRS_COMPLIST_LOG"] = os.path.join(outdir, "clist%04d.log" % n)
            env["ZSHRS_CAPDBG"] = os.path.join(outdir, "capdbg%04d.log" % n)
        start = os.path.getsize(logpath) if logpath and os.path.exists(logpath) else 0
        t0 = time.monotonic()
        v = run_cell(args, env, cell, gate)
        secs = time.monotonic() - t0

        feats = shape_features(v)
        if logpath:
            end = os.path.getsize(logpath) if os.path.exists(logpath) else start
            sl = _read_slice(logpath, start, end)
            boots = sl.count("zshrs starting")
            # One boot per capture; a confirm re-run legitimately adds more. Any
            # OTHER writer's lines are in here too and cannot be separated, so
            # the count is reported rather than silently trusted.
            if boots > 1 + args.confirm:
                contaminated += 1
            feats |= engine_features(sl)
            feats |= probe_features(_drain(env["ZSHRS_COMPLIST_LOG"]), "clist")
            feats |= probe_features(_drain(env["ZSHRS_CAPDBG"]), "capdbg")
            feats |= ftime_features(_drain(FTIME_PATH))

        new = feats - seen
        seen |= feats
        curve.append((n + 1, len(seen)))
        table.observe(classes, secs, len(new), 0)

        results.append(v)
        counts[v.status] = counts.get(v.status, 0) + 1
        line = "%-7s %-8s %r" % (v.status, v.seq, v.case.buffer)
        if v.status in ("FAIL", "FLAKY"):
            line += "  [%s]" % v.fingerprint
        print(line + (("  (%s)" % v.detail) if v.detail else ""))
        print("        keys=%s  styles=%d  pick=%s  from %s"
              % (",".join(v.keys), len(v.statements or ()), how, cell.origin))
        print("        %s%s" % (_cov_note(feats, len(new)),
                                "  <- RETAINED" if (new and guided) else ""))
        for f in sorted(new)[:6]:
            print("          + %s" % f)
        if len(new) > 6:
            print("          + ... %d more" % (len(new) - 6))
        sys.stdout.flush()

        if v.status in ("FAIL", "FLAKY"):
            failures.append(v)
            print_failure(v, args)
        elif v.status == "TIMEOUT":
            print_timeout(v, args)
        elif v.status in ("REF-CRASHED", "TEST-CRASHED"):
            print_crash(v, args)

        # Retention for INFORMATION. A failing input is promoted below by the
        # existing rule regardless; this is the addition — an input that merely
        # went somewhere new becomes a parent for every later run.
        if new and guided and v.status not in ("FAIL", "FLAKY"):
            keep = FuzzInput(inp.buffer, inp.keys, inp.statements,
                             origin="cov/%s" % (inp.origin or "?"),
                             note="%d new feature(s): %s"
                                  % (len(new), ", ".join(sorted(new)[:3])))
            if _cov_admit(args, keep, len(new)):
                picked.append(keep)
                corpus.append(keep)
                retained += 1
        sys.stdout.flush()

    print()
    groups = print_fingerprint_groups(failures, args) if failures else {}
    if not failures:
        print("# 0 failing cell(s), 0 distinct fingerprint(s)")

    # Promotion + three-dimension shrink: byte-for-byte the rule `--mutate`
    # uses. Guidance changed which inputs were run, and nothing else.
    for fp, vs in groups.items():
        if fp in known_fps:
            print("# fingerprint %s already in the corpus — not re-promoted" % fp)
            continue
        rep = min(vs, key=lambda v: v.size())
        if args.cov_log:
            # One extra cell, on the un-instrumented env, before anything is
            # written down. This can only ever REMOVE a reproducer the fuzzer
            # would otherwise have claimed; the cell itself is still reported
            # above as the FAIL it was.
            zf = write_statements(rep.statements, outdir, "clean_%s" % fp)
            clean = run_case(args, base_env,
                             build_init(dump, fpath_dirs, zf),
                             rep.case, rep.seq, rep.keys)
            if clean.status not in ("FAIL", "FLAKY"):
                artifacts += 1
                print("# fingerprint %s does NOT survive without --cov-log's env "
                      "(clean re-run: %s) — an artefact of the instrumentation, "
                      "not promoted" % (fp, clean.status))
                continue
            if clean.fingerprint != fp:
                artifacts += 1
                print("# fingerprint %s became %s without --cov-log's env — the "
                      "instrumentation was part of the evidence, so this "
                      "reproducer would not replay. Not promoted; re-run "
                      "without --cov-log to mine it."
                      % (fp, clean.fingerprint))
                continue
            rep = clean
        inp = FuzzInput(rep.case.buffer, rep.keys, rep.statements or [],
                        origin="guided/%s" % rep.seq, fingerprint=fp,
                        note=fp_label(rep))
        buf, keys, stmts, probes = (rep.case.buffer, list(rep.keys),
                                    list(rep.statements or []), 0)
        if args.shrink:
            # Clean env here too: a shrink probe judged under instrumentation
            # could keep a reduction that only holds while ZSHRS_LOG is set.
            buf, keys, stmts, probes = shrink_input(
                args, base_env, dump, fpath_dirs, inp, fp, outdir,
                args.shrink_probes)
        minimal = FuzzInput(buf, keys, stmts, origin="guided/%s" % rep.seq,
                            fingerprint=fp, note=fp_label(rep))
        zfile = write_statements(stmts, args.corpus_dir,
                                 minimal.stem("fp") + "_styles")
        path = corpus_write(args.corpus_dir, minimal, "fp")
        promoted += 1
        table.observe(input_classes(minimal), 0.0, 0, 1)
        print("# NEW fingerprint %s promoted into the corpus" % fp)
        print("#   before: buffer=%r keys=%s statements=%d"
              % (rep.case.buffer, ",".join(rep.keys), len(rep.statements or ())))
        print("#   after : buffer=%r keys=%s statements=%d  (%d shrink probe(s) spent)"
              % (buf, ",".join(keys), len(stmts), probes))
        print("#   file  : %s" % path)
        print("#   replay: %s" % repro_cmd(args, buf, keys, zstyle=zfile))
        known_fps.add(fp)

    total = args.guided
    print("\n# %d passed, %d failed, %d cell(s)"
          % (counts["PASS"], counts["FAIL"] + counts["FLAKY"], total))
    print("# categories: PASS=%d FAIL=%d FLAKY=%d TIMEOUT=%d SKIP=%d "
          "REF-CRASHED=%d TEST-CRASHED=%d"
          % (counts["PASS"], counts["FAIL"], counts["FLAKY"],
             counts["TIMEOUT"], counts["SKIP"],
             counts.get("REF-CRASHED", 0), counts.get("TEST-CRASHED", 0)))
    print_crash_counts(counts)
    if counts["TIMEOUT"]:
        print("# %d cell(s) ran out of MEASUREMENT budget — not divergences, not "
              "passes; re-run them at --jobs 1" % counts["TIMEOUT"])
    if counts["SKIP"]:
        print("# %d cell(s) skipped: command not installed here (--no-skip-missing "
              "to run them anyway)" % counts["SKIP"])
    print()

    # ── what the run LEARNED ──
    print("# coverage")
    print("#   %d distinct feature(s) seen across %d cell(s)" % (len(seen), total))
    if curve:
        first_half = curve[len(curve) // 2][1]
        print("#   discovery curve: %s"
              % " ".join("%d:%d" % (i, f) for i, f in curve[::max(1, len(curve) // 8)]))
        print("#   %d feature(s) by the halfway point, %d by the end — %d found in "
              "the second half" % (first_half, len(seen), len(seen) - first_half))
    print("#   %.2f new feature(s) per cell" % (len(seen) / total if total else 0.0))
    print("#   %d input(s) RETAINED for information (a new feature, no failure)"
          % retained)
    print("#   %d fingerprint(s) promoted for failing" % promoted)
    if artifacts:
        print("#   %d fingerprint(s) withheld: they did not survive a re-run on "
              "the un-instrumented env, so they were the instrumentation's, not "
              "zshrs's" % artifacts)
    cov_entries = [i for i in corpus if (i.origin or "").startswith("cov/")]
    print("#   corpus now holds %d entry(ies), %d of them retained for coverage"
          % (len(corpus), len(cov_entries)))
    if contaminated:
        print("#   ! %d log slice(s) contained more shell boots than this run "
              "started — another zshrs was writing the shared log, so those "
              "engine features may not be this harness's. Re-run alone to be "
              "sure." % contaminated)
    print()
    print_yield_table(table)

    if args.json:
        write_json(args, {
            "schema": "comptab-parity-guided/1",
            "mode": args.mode,
            "argv": sys.argv[1:],
            "corpus_dir": args.corpus_dir,
            "seed": args.seed,
            "guided": guided,
            "summary": {
                "cells": total,
                "passed": counts["PASS"],
                "failed": counts["FAIL"] + counts["FLAKY"],
                "timeout": counts["TIMEOUT"],
                "skipped": counts["SKIP"],
                "fingerprints": len(groups),
                "promoted": promoted,
                "withheld_as_instrumentation_artefacts": artifacts,
                "features": len(seen),
                "retained": retained,
                "contaminated_slices": contaminated,
            },
            "features": sorted(seen),
            "discovery_curve": curve,
            "yield": [{"class": c, "cells": n, "secs": round(s, 2),
                       "features": f, "fingerprints": p, "rate": round(sc, 5)}
                      for c, n, s, f, p, sc in table.rows()],
            "fingerprints": fingerprint_doc(groups),
            "results": [to_json(v) for v in results],
        })
    return 1 if (failures or counts["TIMEOUT"] or counts["SKIP"]
                 or crashed(counts)) else 0


def _cov_admit(args, keep, n_new):
    """Write a coverage-retained input, keeping the corpus bounded.

    A corpus that grows without limit makes every later run slower to load and
    dilutes the weighted draw. At the cap, the least informative coverage entry
    is evicted — never a `fp_*` reproducer, which is a finding and is not the
    fuzzer's to discard.
    """
    d = args.corpus_dir
    existing = sorted(f for f in os.listdir(d) if f.startswith("cov_")
                      and f.endswith(".json")) if os.path.isdir(d) else []
    if len(existing) >= args.cov_corpus_max:
        worst, worst_n = None, None
        for name in existing:
            try:
                with open(os.path.join(d, name)) as f:
                    m = re.match(r"(\d+) new", json.load(f).get("note", ""))
            except (OSError, ValueError):
                m = None
            n = int(m.group(1)) if m else 0
            if worst_n is None or n < worst_n:
                worst, worst_n = name, n
        if worst is None or worst_n >= n_new:
            return False
        try:
            os.unlink(os.path.join(d, worst))
        except OSError:
            return False
    corpus_write(d, keep, "cov")
    return True


# ── storage and lookup: how a completer is STORED and FOUND (--layout-fuzz) ──
#
# Everything above this line varies what is TYPED (`--mutate`), what the shell
# is CONFIGURED with (`--style-fuzz`), or which case is run. None of it varies
# the layer underneath: every cell so far has used one fpath (the user's real
# dirs), one dump, loaded one way (`compinit -C -d`). That layer is not inert —
# round 3 traced seven "timeouts" to the REFERENCE zsh segfaulting while
# autoloading out of a 35MB `.zwc` digest, which vanished when the same
# completers were plain files.
#
# So this axis varies the store and the lookup, and holds both shells to what
# the documentation says, quoted here so a verdict can be argued from the spec
# rather than from this script's opinion:
#
#   Doc/Zsh/func.yo:93-130   For each fpath `element` the shell looks for
#       `element.zwc`, `element/function.zwc` and `element/function`, "the
#       newest of which is used to load the definition for the function", and
#       "if element already includes a .zwc extension ... element is searched
#       for the definition of the function without comparing its age to that
#       of other files".  Also: "if more than one of these contains a
#       definition for the function that is sought, the leftmost in the fpath
#       is chosen".
#   Doc/Zsh/compsys.yo:154-171  `-D` turns dumping off; `-d dumpfile` names
#       it; "The check performed to see if there are new functions can be
#       omitted by giving the option -C".
#   Doc/Zsh/compsys.yo:182-201  The security check; `-u` uses everything
#       found without asking, `-i` "silently ignore[s] all insecure files and
#       directories", and "This security check is skipped entirely when the
#       -C option is given, provided the dumpfile exists".  Setting `_compdir`
#       to the empty string forces "a check of exactly the directories
#       currently named in fpath" — which is what makes a scratch fpath a
#       controlled experiment instead of a suggestion.
#   Completion/compinit:469-499  The dump is sourced unconditionally under
#       `-C`; otherwise only when its `#files:` count equals `$#_i_files` AND
#       its `version:` equals `$ZSH_VERSION`.
#   Completion/compinit:504-528  The registration scan reads the FIRST LINE of
#       every non-`.zwc` file in each fpath dir, in fpath order, and
#       `compdef -na` keeps the first claim on a command name.
#   Completion/compaudit:125-163  What "insecure" means: a group- or
#       world-writable fpath directory (or the PARENT of one), or a file in
#       one, not owned by root or by this user.
#
# Both shells always get the byte-identical layout, the byte-identical init
# file, and the same reset of any dump the layout defines — a shell that
# autodumps must not hand the second shell a different starting state.

LAYOUT_MARK = "@LY@"

# Which implementation body a lookup resolved to. It is the completion output,
# so a precedence bug is visible on the screen instead of having to be inferred.
IMPL_PLAIN, IMPL_DIGEST = "PLAIN", "DIGEST"

# How the implementation helper is stored next to the registration stubs.
LAYOUT_STORES = (
    "plain",            # `_zzimpl` is a plain file. The control.
    "digest",           # only in `<dir>.zwc`; the plain file is gone.
    "digest-stale",     # both, digest mtime forced OLD -> the dir must win.
    "digest-shadow",    # both, digest mtime forced NEW -> the digest must win.
    "digest-explicit",  # fpath names the `.zwc` itself (func.yo:105-112).
    "digest-corrupt",   # `<dir>.zwc` truncated mid-file; the plain file remains.
)

# How the dirs are threaded onto fpath.
LAYOUT_FPATHS = (
    "single",           # one dir. The control.
    "dup",              # the same dir twice.
    "missing",          # a nonexistent dir first.
    "unreadable",       # a mode-000 dir first.
    "symlink",          # reached through a symlink instead of its real path.
    "two-dirs",         # the same completer in two dirs, different bodies.
    "tag-mismatch",     # `_zzalt` whose `#compdef` claims zz01, ahead of `_zz01`.
)

# How compinit is called.
LAYOUT_COMPINITS = (
    "C-d",              # -C -d DUMP   skip the check, source the dump as-is
    "i-d",              # -i -d DUMP   ignore insecure
    "u-d",              # -u -d DUMP   use everything found
    "d",                # -d DUMP      checked load, autodump on mismatch
    "D",                # -D           no dump at all
    "bare",             # compinit     dump at $ZDOTDIR/.zcompdump
    "ask",              # compinit     with the default insecure-dir prompt
)

# What is sitting at the dump path when compinit starts.
LAYOUT_DUMPS = (
    "zsh-written",      # written by the reference zsh from THIS layout
    "zshrs-written",    # written by zshrs from THIS layout
    "missing",          # no file there
    "stale",            # a real dump whose `#files:` count does not match
    "corrupt",          # not a dump at all
    "none",             # the layout names no dump (-D / bare)
)

LAYOUT_SECURITY = (
    "secure",           # every fpath dir 0755, owned by this user
    "world-writable",   # the completer dir is 0777 (compaudit:125)
    "other-owner",      # a file owned by another user — needs privileges
)


class Layout:
    """One storage/lookup configuration, and everything built for it."""

    def __init__(self, name, store, fpath, compinit, dump, security, note=""):
        self.name = name
        self.store, self.fpath = store, fpath
        self.compinit, self.dump, self.security = compinit, dump, security
        self.note = note
        self.dirs = []              # the fpath list BOTH shells get
        self.init_file = None
        self.dump_path = None
        self.dump_template = None
        self.notes = []             # generator facts, printed verbatim
        self.unbuildable = None     # why this host cannot construct it
        self.preflight = None       # (rc, stdout, stderr) from real zsh
        self.restore = []           # (path, mode) to put back before rmtree

    @property
    def axes(self):
        return "store=%s fpath=%s compinit=%s dump=%s sec=%s" % (
            self.store, self.fpath, self.compinit, self.dump, self.security)

    def spec_note(self):
        """The documented rule this layout is holding both shells to."""
        return {
            "plain": "func.yo:120-122 — element/function is the definition",
            "digest": "func.yo:97-103 — element.zwc is searched like the directory",
            "digest-stale": "func.yo:93-94,125-130 — the NEWER of digest and directory wins",
            "digest-shadow": "func.yo:93-94,125-130 — the NEWER of digest and directory wins",
            "digest-explicit": "func.yo:105-112 — an explicit .zwc element is used without an age comparison",
            "digest-corrupt": "no documented behaviour: both shells must reject it identically and fall back",
        }.get(self.store, "")


def layout_catalog():
    """The curated matrix, in a fixed order so `--layout-fuzz N` is the same N.

    Ordered so the cheapest and most load-bearing conditions come first: the
    control, then the digest precedence rules, then fpath composition, then the
    compinit/dump matrix, then security.
    """
    L = Layout
    return [
        # ── the control, and the digest precedence rules ──
        L("plain-C", "plain", "single", "C-d", "zsh-written", "secure",
          "control: plain files, prebuilt dump, security check skipped"),
        L("digest-only", "digest", "single", "C-d", "zsh-written", "secure",
          "the helper exists ONLY inside <dir>.zwc"),
        L("digest-stale", "digest-stale", "single", "C-d", "zsh-written", "secure",
          "digest older than the directory: the plain file must win"),
        L("digest-shadow", "digest-shadow", "single", "C-d", "zsh-written", "secure",
          "digest newer than the directory: the digest must win"),
        L("digest-explicit", "digest-explicit", "single", "C-d", "zsh-written", "secure",
          "fpath element IS the .zwc, no directory of that name"),
        L("digest-corrupt", "digest-corrupt", "single", "C-d", "zsh-written", "secure",
          "truncated digest: both shells must reject it and fall back"),
        L("digest-corrupt-D", "digest-corrupt", "single", "D", "none", "secure",
          "truncated digest with no dump: the scan itself has to survive it"),
        # ── fpath composition ──
        L("fpath-dup", "plain", "dup", "C-d", "zsh-written", "secure",
          "the same directory listed twice"),
        L("fpath-missing", "plain", "missing", "C-d", "zsh-written", "secure",
          "a nonexistent directory ahead of the real one"),
        L("fpath-unreadable", "plain", "unreadable", "C-d", "zsh-written", "secure",
          "a mode-000 directory ahead of the real one"),
        L("fpath-symlink", "plain", "symlink", "C-d", "zsh-written", "secure",
          "the directory reached through a symlink"),
        L("fpath-two-dirs", "plain", "two-dirs", "D", "none", "secure",
          "same completer in two dirs: func.yo:128-129 leftmost wins"),
        L("fpath-tag-mismatch", "plain", "tag-mismatch", "D", "none", "secure",
          "a file whose #compdef claims a command another file is named for"),
        L("fpath-two-dirs-dump", "plain", "two-dirs", "u-d", "missing", "secure",
          "leftmost-wins, then dumped: the dump must record the same winner"),
        # ── compinit mode / dump state ──
        L("dump-missing-u", "plain", "single", "u-d", "missing", "secure",
          "no dump: full scan, then autodump"),
        L("dump-missing-i", "plain", "single", "i-d", "missing", "secure",
          "-i with nothing insecure: must behave as -u does here"),
        L("dump-stale-C", "plain", "single", "C-d", "stale", "secure",
          "-C sources the stale dump unconditionally (compinit:493-496)"),
        L("dump-stale-d", "plain", "single", "d", "stale", "secure",
          "no -C: the #files count must be rechecked and the dump rebuilt"),
        L("dump-corrupt-C", "plain", "single", "C-d", "corrupt", "secure",
          "-C sources garbage: both shells must fail the same way"),
        L("dump-corrupt-d", "plain", "single", "d", "corrupt", "secure",
          "checked load of garbage"),
        L("dump-foreign-zsh", "plain", "single", "C-d", "zsh-written", "secure",
          "a dump written by the REFERENCE zsh, read by both"),
        L("dump-foreign-zshrs", "plain", "single", "C-d", "zshrs-written", "secure",
          "a dump written by ZSHRS, read by both"),
        L("dump-foreign-zshrs-d", "plain", "single", "d", "zshrs-written", "secure",
          "a zshrs-written dump under the CHECKED load path"),
        L("dump-none-D", "plain", "single", "D", "none", "secure",
          "-D: no dump is read and none is written"),
        L("dump-bare", "plain", "single", "bare", "missing", "secure",
          "no -d: the dump lands at $ZDOTDIR/.zcompdump (compsys.yo:157-159)"),
        # ── security ──
        L("insecure-C", "plain", "single", "C-d", "zsh-written", "world-writable",
          "-C skips the check entirely (compsys.yo:189-190)"),
        L("insecure-i", "plain", "single", "i-d", "missing", "world-writable",
          "-i must silently drop the insecure dir from fpath"),
        L("insecure-u", "plain", "single", "u-d", "missing", "world-writable",
          "-u must use it anyway"),
        L("insecure-ask", "plain", "single", "ask", "missing", "world-writable",
          "the default prompt path (compinit:436-451)"),
        L("insecure-other-owner", "plain", "single", "i-d", "missing", "other-owner",
          "a completer owned by another user"),
    ]


def layout_random(rng, n, catalog_names):
    """Seeded combinations beyond the curated catalog.

    Only combinations that are internally consistent are emitted: a `-D` or
    `bare` compinit does not read a named dump, and `-C` with no dump file is
    the one case the documentation calls out as NOT skipping the security check
    ("provided the dumpfile exists", compsys.yo:189-190), so it is generated
    deliberately rather than avoided.
    """
    out = []
    for i in range(n):
        store = rng.choice(LAYOUT_STORES)
        fp = rng.choice(LAYOUT_FPATHS)
        ci = rng.choice(LAYOUT_COMPINITS)
        sec = rng.choice(("secure", "secure", "world-writable"))
        if ci in ("D", "bare", "ask"):
            dump = "none" if ci == "D" else "missing"
        else:
            dump = rng.choice([d for d in LAYOUT_DUMPS if d != "none"])
        name = "rnd%03d" % i
        if name in catalog_names:
            continue
        out.append(Layout(name, store, fp, ci, dump, sec, "seeded combination"))
    return out


# ── materialising a layout on disk ───────────────────────────────────────────

# `#autoload` with no options on purpose: compinit:522-524 autoloads it and
# stores it in `_compautos` ONLY when the tag line carries options, so this file
# also pins that branch.
_IMPL_BODY = """\
#autoload
compadd -- ${1}-%(mark)s-alpha ${1}-%(mark)s-beta ${1}-%(mark)s-gamma
"""

# The registration stub. Every store variant keeps these, so the `#compdef`
# scan is constant and the only thing that moves is where `_zzimpl` came from.
_STUB_BODY = """\
#compdef %(cmd)s
_zzimpl %(tag)s
"""


def _write(path, text, mode=0o644):
    with open(path, "w") as f:
        f.write(text)
    os.chmod(path, mode)


def _impl_text(mark):
    return _IMPL_BODY % {"mark": mark}


def _zcompile(zsh, cwd, out, sources):
    """Build a `.zwc` with the REFERENCE zsh's own zcompile.

    Deliberately zsh's: the digest format is zsh's, and a digest zshrs wrote is
    a different experiment (one this axis does not claim to have run).
    """
    import subprocess
    cmd = "zcompile -U %s %s" % (shlex.quote(out),
                                 " ".join(shlex.quote(s) for s in sources))
    p = subprocess.run([zsh, "-f", "-c", cmd], cwd=cwd,
                       capture_output=True, text=True, timeout=60)
    return p.returncode, (p.stderr or "").strip()


def _set_mtime(path, when):
    os.utime(path, (when, when))


OLD_TIME = 946684800.0      # 2000-01-01, unambiguously older than anything here
NEW_TIME = 1893456000.0     # 2030-01-01, unambiguously newer


def build_layout(base, lay, args, sysdir, nfuncs=6):
    """Create every file this layout needs and fill in lay.dirs / lay.init_file.

    `base` is the run's scratch root, `sysdir` a private copy of the zsh
    distribution functions (compinit, compaudit, compdump, _main_complete, ...)
    with sane modes — the Homebrew copy on this host is 0777, which makes every
    layout insecure and would have made the security axis untestable.
    """
    root = os.path.join(base, "L_" + lay.name)
    os.makedirs(root, exist_ok=True)
    d = os.path.join(root, "fp")
    os.makedirs(d, exist_ok=True)

    # Registration stubs: one per synthetic command, constant across variants.
    for i in range(1, nfuncs + 1):
        cmd = "zz%02d" % i
        _write(os.path.join(d, "_" + cmd), _STUB_BODY % {"cmd": cmd, "tag": cmd})

    impl = os.path.join(d, "_zzimpl")
    zwc = d + ".zwc"
    src = os.path.join(root, "src")
    os.makedirs(src, exist_ok=True)

    # ── store ──
    if lay.store == "plain":
        _write(impl, _impl_text(IMPL_PLAIN))
    elif lay.store == "digest":
        _write(os.path.join(src, "_zzimpl"), _impl_text(IMPL_DIGEST))
        rc, err = _zcompile(args.zsh, root, "fp.zwc", ["src/_zzimpl"])
        if rc != 0:
            lay.unbuildable = "zcompile failed: %s" % err
            return lay
        _set_mtime(zwc, NEW_TIME)
    elif lay.store in ("digest-stale", "digest-shadow"):
        _write(impl, _impl_text(IMPL_PLAIN))
        _write(os.path.join(src, "_zzimpl"), _impl_text(IMPL_DIGEST))
        rc, err = _zcompile(args.zsh, root, "fp.zwc", ["src/_zzimpl"])
        if rc != 0:
            lay.unbuildable = "zcompile failed: %s" % err
            return lay
        if lay.store == "digest-stale":
            _set_mtime(zwc, OLD_TIME)
            _set_mtime(d, NEW_TIME)
            lay.notes.append("digest mtime 2000-01-01, directory mtime 2030-01-01")
        else:
            _set_mtime(d, OLD_TIME)
            _set_mtime(zwc, NEW_TIME)
            lay.notes.append("directory mtime 2000-01-01, digest mtime 2030-01-01")
    elif lay.store == "digest-explicit":
        _write(os.path.join(src, "_zzimpl"), _impl_text(IMPL_DIGEST))
        rc, err = _zcompile(args.zsh, root, "explicit.zwc", ["src/_zzimpl"])
        if rc != 0:
            lay.unbuildable = "zcompile failed: %s" % err
            return lay
    elif lay.store == "digest-corrupt":
        _write(impl, _impl_text(IMPL_PLAIN))
        _write(os.path.join(src, "_zzimpl"), _impl_text(IMPL_DIGEST))
        rc, err = _zcompile(args.zsh, root, "fp.zwc", ["src/_zzimpl"])
        if rc != 0:
            lay.unbuildable = "zcompile failed: %s" % err
            return lay
        os.chmod(zwc, 0o644)
        with open(zwc, "r+b") as f:
            f.truncate(60)
        _set_mtime(zwc, NEW_TIME)
        lay.notes.append("digest truncated to 60 bytes, mtime 2030-01-01")
    else:
        lay.unbuildable = "unknown store %r" % lay.store
        return lay

    # ── fpath composition ──
    dirs = []
    if lay.fpath == "single":
        dirs = [d]
    elif lay.fpath == "dup":
        dirs = [d, d]
    elif lay.fpath == "missing":
        dirs = [os.path.join(root, "does-not-exist"), d]
    elif lay.fpath == "unreadable":
        u = os.path.join(root, "unreadable")
        os.makedirs(u, exist_ok=True)
        _write(os.path.join(u, "_zz01"), _STUB_BODY % {"cmd": "zz01", "tag": "UNREADABLE"})
        os.chmod(u, 0o000)
        lay.restore.append((u, 0o755))
        dirs = [u, d]
    elif lay.fpath == "symlink":
        link = os.path.join(root, "link")
        if not os.path.islink(link):
            os.symlink(d, link)
        dirs = [link]
    elif lay.fpath == "two-dirs":
        b = os.path.join(root, "fpB")
        os.makedirs(b, exist_ok=True)
        _write(os.path.join(b, "_zz01"), _STUB_BODY % {"cmd": "zz01", "tag": "DIRB"})
        _write(os.path.join(b, "_zzimpl"), _impl_text(IMPL_PLAIN))
        dirs = [d, b]
    elif lay.fpath == "tag-mismatch":
        a = os.path.join(root, "fpA")
        os.makedirs(a, exist_ok=True)
        # Named _zzalt, but its tag line claims zz01 — compinit:519 registers by
        # the TAG, `compdef -na` keeps the first claim, and the file name only
        # decides which function gets autoloaded.
        _write(os.path.join(a, "_zzalt"), _STUB_BODY % {"cmd": "zz01", "tag": "ALTFILE"})
        _write(os.path.join(a, "_zzimpl"), _impl_text(IMPL_PLAIN))
        dirs = [a, d]
    else:
        lay.unbuildable = "unknown fpath composition %r" % lay.fpath
        return lay

    if lay.store == "digest-explicit":
        dirs = dirs + [os.path.join(root, "explicit.zwc")]

    # ── security ──
    if lay.security == "world-writable":
        os.chmod(d, 0o777)
        lay.restore.append((d, 0o755))
        lay.notes.append("%s is mode 0777 (compaudit:125 calls that insecure)" % d)
    elif lay.security == "other-owner":
        # compaudit flags files not owned by root or by this user. Creating one
        # needs a second uid, which this harness does not have; say so instead
        # of quietly running a DIFFERENT layout under the same name.
        lay.unbuildable = ("needs a file owned by another user; chown to a "
                           "second uid requires privileges this process does "
                           "not have (euid %d)" % os.geteuid())
        return lay
    elif lay.security != "secure":
        lay.unbuildable = "unknown security condition %r" % lay.security
        return lay

    lay.dirs = dirs + [sysdir]
    return lay


def _layout_script(lay, dump_line, compinit_line, zstyle_file=None,
                   prompt=True, zdotdir=None):
    """The init BOTH shells source. One text, no per-shell branches."""
    lines = []
    if prompt:
        lines += ["# generated by comptab_parity.py --layout-fuzz",
                  "PROMPT='%s '" % SENTINEL, "RPROMPT=''", "PS2='> '",
                  "setopt no_beep"]
    if zdotdir:
        lines.append("ZDOTDIR=%s" % shlex.quote(zdotdir))
    # compsys.yo:199-201 — force compaudit to check EXACTLY these directories
    # instead of wandering off to add _compdir's siblings, which would make the
    # scratch fpath a suggestion rather than the experiment.
    lines.append("_compdir=''")
    lines.append("fpath=( %s )" % " ".join(shlex.quote(p) for p in lay.dirs))
    if zstyle_file and os.path.exists(zstyle_file):
        lines.append("source %s" % shlex.quote(zstyle_file))
    if dump_line:
        lines.append(dump_line)
    lines.append("autoload -Uz compinit")
    lines.append(compinit_line)
    if prompt:
        lines.append("print -u2 ''")
    return "\n".join(lines) + "\n"


def _dump_reset_line(lay):
    """The line that puts the dump back into its layout-defined state.

    It runs INSIDE the measured shell, identically on both sides, because a
    compinit that autodumps would otherwise hand whichever shell runs second a
    dump the first one wrote — a different input under the same layout name.
    """
    if not lay.dump_path:
        return ""
    if lay.dump == "missing":
        return "command rm -f %s" % shlex.quote(lay.dump_path)
    if lay.dump_template:
        return "command cp -f %s %s" % (shlex.quote(lay.dump_template),
                                        shlex.quote(lay.dump_path))
    return ""


def _compinit_line(lay):
    flags = {
        "C-d": "-C -d %s",
        "i-d": "-i -d %s",
        "u-d": "-u -d %s",
        "d": "-d %s",
    }
    if lay.compinit in flags:
        return "compinit " + flags[lay.compinit] % shlex.quote(lay.dump_path)
    if lay.compinit == "D":
        return "compinit -D -u"
    if lay.compinit == "bare":
        return "compinit"
    if lay.compinit == "ask":
        return "compinit"
    return None


def write_layout_dump(shell_argv, lay, base, path, args):
    """Have one shell write a dump for THIS layout, and report how.

    Returns (ok, how, stderr). `how` is "autodump" when `compinit -d` produced
    the file by itself and "explicit-compdump" when it did not and `compdump`
    had to be called by hand — a difference between the two shells that is
    printed, never smoothed over.
    """
    import subprocess
    script = _layout_script(
        lay, "command rm -f %s" % shlex.quote(path),
        "compinit -u -d %s" % shlex.quote(path), prompt=False)
    script += (
        "if [[ ! -f %(p)s ]]; then\n"
        "  autoload -Uz compdump\n"
        "  typeset -g _comp_dumpfile=%(p)s\n"
        "  compdump\n"
        "  print -r -- '%(m)s fallback'\n"
        "else\n"
        "  print -r -- '%(m)s autodump'\n"
        "fi\n" % {"p": shlex.quote(path), "m": LAYOUT_MARK})
    sf = os.path.join(base, "mkdump_%s_%s.zsh"
                      % (lay.name, os.path.basename(shell_argv[0])))
    _write(sf, script)
    try:
        p = subprocess.run(list(shell_argv) + ["-f", "-c", "source " + shlex.quote(sf)],
                           capture_output=True, text=True, timeout=120)
    except (OSError, subprocess.SubprocessError) as exc:
        return False, "error", str(exc)
    how = "autodump" if (LAYOUT_MARK + " autodump") in (p.stdout or "") else \
          ("explicit-compdump" if (LAYOUT_MARK + " fallback") in (p.stdout or "")
           else "no-marker")
    return os.path.exists(path), how, (p.stderr or "").strip()


def prepare_layout_dump(lay, base, args, env):
    """Materialise the dump TEMPLATE this layout starts from."""
    if lay.dump == "none":
        lay.dump_path = None
        return
    root = os.path.join(base, "L_" + lay.name)
    lay.dump_path = (os.path.join(root, ".zcompdump") if lay.compinit in ("bare", "ask")
                     else os.path.join(root, "dump"))
    if lay.dump == "missing":
        lay.dump_template = None
        return
    tpl = os.path.join(root, "dump.template")
    if lay.dump == "corrupt":
        with open(tpl, "wb") as f:
            f.write(b"#files: 12\tversion: 5.9.2\n\x00\x01\x02not a dump("
                    b"\nunterminated=( 'x'\n")
        lay.dump_template = tpl
        lay.notes.append("dump template is deliberate garbage")
        return
    if lay.dump == "stale":
        # A REAL dump, with the one field compinit:472-473 checks made wrong.
        ok, how, err = write_layout_dump([args.zsh], lay, base, tpl, args)
        if not ok:
            lay.unbuildable = "could not write a dump to make stale: %s" % err
            return
        with open(tpl) as f:
            text = f.read()
        head, rest = text.split("\n", 1)
        with open(tpl, "w") as f:
            f.write("#files: 3\tversion: 5.9.2\n" + rest)
        lay.dump_template = tpl
        lay.notes.append("dump `#files:` count rewritten from its real value to 3 "
                         "(compinit:472 compares it with $#_i_files)")
        return
    writer = [args.zsh] if lay.dump == "zsh-written" else [args.zshrs]
    if lay.dump == "zshrs-written" and args.mode == "zsh":
        writer = [args.zshrs, "--zsh"]
    ok, how, err = write_layout_dump(writer, lay, base, tpl, args)
    if not ok:
        lay.unbuildable = ("%s could not write a dump for this layout: %s"
                           % (os.path.basename(writer[0]), err or "no file produced"))
        return
    lay.dump_template = tpl
    lay.notes.append("dump written by %s via %s (%d bytes)"
                     % (os.path.basename(writer[0]), how, os.path.getsize(tpl)))
    if how == "explicit-compdump":
        lay.notes.append("!! %s's `compinit -d FILE` did NOT write the dump; "
                         "compdump had to be called by hand"
                         % os.path.basename(writer[0]))


def finish_layout(lay, base, args, zstyle_file):
    """Write the init file both shells will source."""
    line = _compinit_line(lay)
    if line is None:
        lay.unbuildable = "unknown compinit mode %r" % lay.compinit
        return
    zdot = os.path.join(base, "L_" + lay.name) if lay.compinit in ("bare", "ask") else None
    script = _layout_script(lay, _dump_reset_line(lay), line, zstyle_file,
                            zdotdir=zdot)
    path = os.path.join(base, "L_" + lay.name, "init.zsh")
    _write(path, script)
    lay.init_file = path


# ── letting REAL zsh vet the layout before any pty boots ─────────────────────
#
# Identical in spirit to validate_config: a layout the reference shell itself
# refuses is a fact about the GENERATOR (or about zsh), not a zshrs finding.
# Scoring one as a divergence would invent a bug; scoring it as a pass would be
# worse. So it gets its own counted, printed category and the cell is not run.

def preflight_layout(lay, args, env):
    """Ask the reference zsh what it makes of this layout.

    Returns (verdict, detail) where verdict is one of:
      "ok"        zsh initialised cleanly and registered the test command
      "warned"    zsh initialised but printed something
      "invalid"   zsh refused: compinit aborted, or nothing got registered
    """
    import subprocess
    probe = os.path.join(os.path.dirname(lay.init_file), "preflight.zsh")
    _write(probe,
           "source %s\n"
           "print -r -- '%s' comps=${#_comps} zz01=${_comps[zz01]-<unset>}\n"
           % (shlex.quote(lay.init_file), LAYOUT_MARK))
    try:
        p = subprocess.run([args.zsh, "-f", "-c", "source " + shlex.quote(probe)],
                           capture_output=True, text=True, timeout=120, env=env)
    except (OSError, subprocess.SubprocessError) as exc:
        return "invalid", "could not run %s: %s" % (args.zsh, exc)
    out = (p.stdout or "")
    err = "\n".join(l for l in (p.stderr or "").splitlines() if l.strip())
    marker = [l for l in out.splitlines() if l.startswith(LAYOUT_MARK)]
    if not marker:
        return "invalid", ("reference zsh produced no marker (rc=%d): %s"
                           % (p.returncode, err.splitlines()[0] if err else "<silent>"))
    line = marker[0]
    if "comps=0" in line:
        return "invalid", ("reference zsh registered NO completions under this "
                           "layout: %s%s" % (line, ("; " + err.splitlines()[0]) if err else ""))
    if err:
        return "warned", "%s || zsh said: %s" % (line, err.splitlines()[0])
    return "ok", line


# ── the runner ───────────────────────────────────────────────────────────────

def _layout_selection(args):
    catalog = layout_catalog()
    names = {l.name for l in catalog}
    if args.layout_only:
        want = [s.strip() for s in args.layout_only.split(",") if s.strip()]
        unknown = [w for w in want if w not in names]
        if unknown:
            sys.exit("--layout-only names unknown layout(s): %s\nknown: %s"
                     % (", ".join(unknown), ", ".join(sorted(names))))
        return [l for l in catalog if l.name in want]
    if args.layout_random > 0:
        # Seeded combinations INSTEAD of the catalog: the catalog is the set of
        # conditions someone thought of, this is the set nobody did.
        rng = random.Random((args.seed << 21) ^ 0x1A70)
        return layout_random(rng, args.layout_random, names)
    n = args.layout_fuzz
    if n <= len(catalog):
        return catalog[:n]
    rng = random.Random((args.seed << 21) ^ 0x1A70)
    return catalog + layout_random(rng, n - len(catalog), names)


def build_layout_base(args, env):
    """The per-run scratch root: a private, SECURE copy of the zsh functions,
    plus executable stubs for the synthetic commands.

    The distribution functions are copied because the installed copy on this
    host is mode 0777 (`/opt/homebrew/Cellar/zsh/5.9.2/share/zsh/functions`),
    which compaudit:125 flags as insecure — every layout would have been
    "insecure" and the security axis would have measured nothing.
    """
    import shutil
    import subprocess
    base = tempfile.mkdtemp(prefix="comptab_layout_")
    os.chmod(base, 0o755)
    sysdir = os.path.join(base, "sys")
    src = None
    try:
        p = subprocess.run([args.zsh, "-f", "-c", "print -rl -- $fpath"],
                           capture_output=True, text=True, timeout=20,
                           env={"PATH": os.environ.get("PATH", "/usr/bin:/bin")})
        for cand in (p.stdout or "").splitlines():
            if os.path.isdir(cand) and os.path.exists(os.path.join(cand, "compinit")):
                src = cand
                break
    except (OSError, subprocess.SubprocessError):
        src = None
    if src is None:
        shutil.rmtree(base, ignore_errors=True)
        sys.exit("--layout-fuzz needs the zsh distribution functions directory "
                 "(the one holding `compinit`); none of $fpath under `zsh -f` "
                 "contains it")
    shutil.copytree(src, sysdir)
    for root, dirs, files in os.walk(sysdir):
        os.chmod(root, 0o755)
        for f in files:
            os.chmod(os.path.join(root, f), 0o644)
    bindir = os.path.join(base, "bin")
    os.makedirs(bindir)
    for i in range(1, 7):
        stub = os.path.join(bindir, "zz%02d" % i)
        _write(stub, "#!/bin/sh\nexit 0\n", 0o755)
    return base, sysdir, bindir, src


def layout_repro_cmd(args, lay, v):
    """The command that replays THIS cell — layout included.

    `print_failure`'s repro line is `--case ... --keys ...`, which replays the
    buffer against the DEFAULT fpath and dump: for a layout cell that is a
    different experiment under the same name, and the scratch tree it needs is
    deleted when the run ends. So the layout replay is printed next to it,
    named, and `--layout-keep` is spelled out because the tree a failure was
    read out of is otherwise gone.
    """
    cmd = [SELF]
    if args.mode != "native":
        cmd += ["--mode", args.mode]
    if args.zshrs != os.path.join(REPO, "target", "debug", "zshrs"):
        cmd += ["--zshrs", shlex.quote(args.zshrs)]
    if args.zsh != "zsh":
        cmd += ["--zsh", shlex.quote(args.zsh)]
    if args.zstyle:
        cmd += ["--zstyle", shlex.quote(args.zstyle)]
    if lay.name.startswith("rnd"):
        # A seeded combination has no name to ask for; (seed, count) is its id.
        cmd += ["--layout-random", str(args.layout_random or 0),
                "--seed", str(args.seed)]
    else:
        cmd += ["--layout-only", lay.name]
    cmd += ["--layout-cases", shlex.quote(v.case.buffer),
            "--combo-sequence", args.combo_sequence,
            "--rows", str(args.rows), "--cols", str(args.cols)]
    if args.settle != 300:
        cmd += ["--settle", str(args.settle)]
    return " ".join(cmd)


def print_layout_table(layouts, header):
    print(header)
    for l in layouts:
        print("#   %-22s %s" % (l.name, l.axes))
        if l.note:
            print("#     %s" % l.note)
        if l.spec_note():
            print("#     spec: %s" % l.spec_note())


def run_layout_list(args):
    """The catalog, and what each entry holds the shells to. No shell booted."""
    layouts = _layout_selection(args) \
        if (args.layout_only or args.layout_fuzz or args.layout_random) \
        else layout_catalog()
    print("# %d storage/lookup layout(s)" % len(layouts))
    print("# axes: store=%s" % ", ".join(LAYOUT_STORES))
    print("#       fpath=%s" % ", ".join(LAYOUT_FPATHS))
    print("#       compinit=%s" % ", ".join(LAYOUT_COMPINITS))
    print("#       dump=%s" % ", ".join(LAYOUT_DUMPS))
    print("#       security=%s" % ", ".join(LAYOUT_SECURITY))
    print()
    print_layout_table(layouts, "# catalog:")
    return 0


def run_dump_xshell(args, env):
    """Answer the cross-shell dump question, in both directions, with evidence.

    Not a parity cell: this is a direct experiment. One layout, four
    combinations — each shell WRITES a dump, then each shell READS both dumps —
    and the state each one ends up with is printed. Nothing here is scored, and
    nothing here can turn a divergence into a pass; it exists because "does
    zshrs read a zsh-written .zcompdump identically, and vice versa" had never
    been measured.
    """
    import shutil
    import subprocess
    base, sysdir, bindir, srcdir = build_layout_base(args, env)
    try:
        lay = Layout("xshell", "plain", "single", "u-d", "missing", "secure")
        build_layout(base, lay, args, sysdir)
        if lay.unbuildable:
            print("# UNBUILDABLE: %s" % lay.unbuildable)
            return 1
        print("# cross-shell .zcompdump compatibility")
        print("# fpath  : %s" % " ".join(lay.dirs))
        print("# zsh    : %s" % args.zsh)
        print("# zshrs  : %s" % " ".join(args.test_argv[:2]))
        print()
        dumps = {}
        for label, argv in (("zsh", [args.zsh]),
                            ("zshrs", args.test_argv[:1] if args.mode == "native"
                             else args.test_argv[:2])):
            path = os.path.join(base, "dump.%s" % label)
            ok, how, err = write_layout_dump(argv, lay, base, path, args)
            dumps[label] = path if ok else None
            print("# WRITE %-6s -> %-8s %s"
                  % (label, "ok" if ok else "FAILED",
                     ("%d bytes, produced by %s" % (os.path.getsize(path), how))
                     if ok else (err or "no file")))
            if ok and how == "explicit-compdump":
                print("#   !! `compinit -u -d FILE` wrote NOTHING on %s; the "
                      "dump above exists only because compdump was called by "
                      "hand afterwards. zsh writes it from compinit itself "
                      "(compinit:532-535)." % label)
            if ok:
                with open(path) as f:
                    print("#   header: %s" % f.readline().rstrip("\n").replace("\t", "\\t"))
        print()
        if dumps["zsh"] and dumps["zshrs"]:
            a = open(dumps["zsh"]).read().splitlines()
            b = open(dumps["zshrs"]).read().splitlines()
            diff = list(difflib.unified_diff(a, b, "zsh-written", "zshrs-written",
                                             lineterm="", n=0))
            print("# WRITE comparison: %d line(s) differ" % max(0, len(diff) - 2))
            for line in diff[:24]:
                print("#   %s" % line)
            if len(diff) > 24:
                print("#   ... %d more diff line(s)" % (len(diff) - 24))
            print()
        print("# READ back — each shell sources each dump under `compinit -C -d`")
        rows = []
        for writer, path in dumps.items():
            if not path:
                continue
            for reader, argv in (("zsh", [args.zsh]),
                                 ("zshrs", args.test_argv[:1] if args.mode == "native"
                                  else args.test_argv[:2])):
                probe = os.path.join(base, "read.zsh")
                copy = os.path.join(base, "read.dump")
                shutil.copyfile(path, copy)
                _write(probe, _layout_script(
                    lay, "", "compinit -C -d %s" % shlex.quote(copy), prompt=False)
                    + "print -r -- comps=${#_comps} services=${#_services} "
                      "compautos=${#_compautos} patcomps=${#_patcomps} "
                      "zz01=${_comps[zz01]-<unset>}\n")
                try:
                    p = subprocess.run(list(argv) + ["-f", "-c", "source " + shlex.quote(probe)],
                                       capture_output=True, text=True, timeout=120, env=env)
                    out = [l for l in (p.stdout or "").splitlines() if l.startswith("comps=")]
                    state = out[-1] if out else "<no output> rc=%d %s" % (
                        p.returncode, (p.stderr or "").strip().splitlines()[:1])
                except (OSError, subprocess.SubprocessError) as exc:
                    state = "error: %s" % exc
                rows.append((writer, reader, state))
                print("#   %-6s dump read by %-6s -> %s" % (writer, reader, state))
        print()
        states = {}
        for writer, reader, state in rows:
            states.setdefault(writer, set()).add(state)
        verdict = []
        for writer, seen in states.items():
            if len(seen) == 1:
                verdict.append("both shells end in the SAME state from the "
                               "%s-written dump" % writer)
            else:
                verdict.append("the two shells end in DIFFERENT states from the "
                               "%s-written dump: %s" % (writer, " | ".join(sorted(seen))))
        for v in verdict:
            print("# %s" % v)
        return 0
    finally:
        if not args.layout_keep:
            shutil.rmtree(base, ignore_errors=True)
        else:
            print("# kept: %s" % base)


def run_layout_fuzz(args, env):
    """Fuzz the STORAGE and LOOKUP layer instead of the typed line or the config.

    Downstream of the layout, everything is the machinery that already exists:
    the same Cell / cell_stream / run_case path, the same fingerprints, the same
    crash and timeout separation. What is new is only where the completers live
    and how compinit is told to find them.

    Categories, and why each is separate:

        PASS / FAIL / FLAKY / TIMEOUT / SKIP   unchanged meanings.
        INVALID-LAYOUT   the REFERENCE zsh refused this layout (compinit
                         aborted, or it registered nothing at all). The cell is
                         not run: comparing two shells on a layout neither can
                         hold says nothing. A fact about the generator or about
                         zsh, never a zshrs finding, never a pass.
        REF-WARNED       zsh initialised but printed a diagnostic (an insecure
                         directory, an invalid digest). The cell IS run and IS
                         compared — zshrs must complain identically — but it is
                         tallied apart so a green sweep cannot be assembled out
                         of layouts zsh itself grumbles about.
        UNBUILDABLE      this host cannot construct the layout (a file owned by
                         another user needs privileges). Counted, printed, never
                         a pass.
    """
    import shutil
    layouts = _layout_selection(args)
    if not layouts:
        sys.exit("--layout-fuzz selected no layouts")
    keys = KEY_SEQUENCES[args.combo_sequence]
    buffers = [b for b in (args.layout_cases or "zz01 ").split(",") if b]
    cases = [adhoc_case(b, prefix="layout") for b in buffers]

    base, sysdir, bindir, srcdir = build_layout_base(args, env)
    # The synthetic commands have to be runnable for `skip_reason` to let the
    # cell run at all, and BOTH shells get the identical PATH.
    env = dict(env)
    env["PATH"] = bindir + os.pathsep + env.get("PATH", "")
    os.environ["PATH"] = bindir + os.pathsep + os.environ.get("PATH", "")

    print("# storage/lookup fuzz (how completers are STORED and FOUND)")
    print("# layouts : %d   seed=%d" % (len(layouts), args.seed))
    print("# cases   : %s   sequence=%s (%s)"
          % (", ".join(repr(b) for b in buffers), args.combo_sequence, "+".join(keys)))
    print("# mode    : %s (%s)" % (args.mode, " ".join(args.test_argv)))
    print("# scratch : %s" % base)
    print("# sysdir  : copied from %s (0755/0644 — the installed copy is 0777, "
          "which compaudit calls insecure)" % srcdir)
    print("# jobs    : %d" % max(1, args.jobs))
    print()

    try:
        built, unbuildable, invalid, warned = [], [], [], []
        for lay in layouts:
            build_layout(base, lay, args, sysdir)
            if not lay.unbuildable:
                prepare_layout_dump(lay, base, args, env)
            if not lay.unbuildable:
                finish_layout(lay, base, args, args.zstyle)
            if lay.unbuildable:
                unbuildable.append(lay)
                continue
            verdict, detail = preflight_layout(lay, args, env)
            lay.preflight = (verdict, detail)
            if verdict == "invalid":
                invalid.append(lay)
                continue
            if verdict == "warned":
                warned.append(lay)
            built.append(lay)

        if unbuildable:
            print("# %d layout(s) UNBUILDABLE on this host — not run, not passed:"
                  % len(unbuildable))
            for l in unbuildable:
                print("#   %-22s %s" % (l.name, l.unbuildable))
            print()
        if invalid:
            print("# %d layout(s) the REFERENCE zsh REFUSED — a fact about the "
                  "layout, not a zshrs finding. Not run, not passed:" % len(invalid))
            for l in invalid:
                print("#   %-22s %s" % (l.name, l.axes))
                print("#     %s" % l.preflight[1])
            print()

        cells = []
        for lay in built:
            for c in cases:
                cells.append(Cell(c, lay.name, keys, lay.init_file,
                                  origin="layout/%s" % lay.name))
        by_seq = {lay.name: lay for lay in built}

        counts = {"PASS": 0, "FAIL": 0, "FLAKY": 0, "TIMEOUT": 0, "SKIP": 0}
        failures, results, unobserved = [], [], []
        for v in cell_stream(args, env, cells):
            results.append(v)
            counts[v.status] = counts.get(v.status, 0) + 1
            lay = by_seq.get(v.seq)
            label = v.status
            if lay is not None and lay.preflight and lay.preflight[0] == "warned":
                label = "%s(ref-warned)" % v.status
            line = "%-20s %-22s %r" % (label, v.seq, v.case.buffer)
            if v.status in ("FAIL", "FLAKY"):
                line += "  [%s]" % v.fingerprint
            print(line + (("  (%s)" % v.detail) if v.detail else ""))
            if lay is not None:
                print("        %s" % lay.axes)
                if lay.spec_note():
                    print("        spec: %s" % lay.spec_note())
                for n in lay.notes:
                    print("        note: %s" % n)
                if lay.preflight and lay.preflight[0] == "warned":
                    print("        ! zsh itself warned about this layout: %s"
                          % lay.preflight[1])
                print("        init: %s" % lay.init_file)
            sys.stdout.flush()
            if v.status in ("FAIL", "FLAKY"):
                failures.append(v)
                print_failure(v, args)
            elif v.status == "TIMEOUT":
                print_timeout(v, args)
            elif v.status in CRASH_STATUSES:
                print_crash(v, args)
            if v.status == "PASS":
                # A PASS where NEITHER screen carries a marker from the
                # layout's own completer means both shells agreed about
                # something else — the layout was never actually observed. That
                # is not a divergence, but it is worth strictly less than a
                # pass that saw the completer run, so it is counted and printed
                # rather than folded into the total silently.
                seen = "\n".join((v.test.grid or []) + (v.ref.grid or []))
                if not any(m in seen for m in (IMPL_PLAIN, IMPL_DIGEST,
                                               "ALTFILE", "DIRB")):
                    unobserved.append(v)
                    print("        ~ neither shell ran this layout's completer: "
                          "the two screens agree, but the layout was not observed")
                if args.verbose:
                    print(render(v.test.grid or []))
            if v.status not in ("PASS", "SKIP") and lay is not None:
                # The line above it replays the BUFFER; this one replays the
                # buffer AND the layout, which is the thing under test here.
                print("  --- layout replay (the repro line above does NOT "
                      "rebuild this layout) ---")
                print("  " + layout_repro_cmd(args, lay, v))
                print("  add --layout-keep to keep the scratch fpath tree")
                print()
            sys.stdout.flush()

        print()
        groups = print_fingerprint_groups(failures, args) if failures else {}
        if not failures:
            print("# 0 failing cell(s), 0 distinct fingerprint(s)")

        print("\n# %d passed, %d failed, %d cell(s) over %d layout(s)"
              % (counts["PASS"], counts["FAIL"] + counts["FLAKY"], len(cells),
                 len(built)))
        print("# categories: PASS=%d FAIL=%d FLAKY=%d TIMEOUT=%d SKIP=%d "
              "INVALID-LAYOUT=%d REF-WARNED=%d UNBUILDABLE=%d REF-CRASHED=%d "
              "TEST-CRASHED=%d"
              % (counts["PASS"], counts["FAIL"], counts["FLAKY"],
                 counts["TIMEOUT"], counts["SKIP"], len(invalid), len(warned),
                 len(unbuildable), counts.get("REF-CRASHED", 0),
                 counts.get("TEST-CRASHED", 0)))
        print_crash_counts(counts)

        # Per-layout breakdown: the point of the axis is which STORE / LOOKUP
        # broke, so the summary is indexed by layout, not only by fingerprint.
        print("# per-layout:")
        seen = {}
        for v in results:
            seen.setdefault(v.seq, []).append(v.status)
        for lay in layouts:
            if lay in unbuildable:
                st = "UNBUILDABLE"
            elif lay in invalid:
                st = "INVALID-LAYOUT"
            else:
                st = ",".join(sorted(set(seen.get(lay.name, ["<not run>"]))))
                if lay in warned:
                    st += " (ref-warned)"
            print("#   %-22s %-28s %s" % (lay.name, st, lay.axes))
        if counts["TIMEOUT"]:
            print("# %d cell(s) ran out of MEASUREMENT budget — not divergences, "
                  "not passes; re-run them at --jobs 1" % counts["TIMEOUT"])
        if counts["SKIP"]:
            print("# %d cell(s) skipped: command not installed here" % counts["SKIP"])
        if unobserved:
            print("# %d pass(es) where NEITHER shell ran the layout's own "
                  "completer — the screens agreed, but nothing about the layout "
                  "was exercised. Worth less than an observed pass:"
                  % len(unobserved))
            for v in unobserved[:20]:
                print("#   %s" % v.seq)

        if args.json:
            write_json(args, {
                "schema": "comptab-parity-layout-fuzz/1",
                "mode": args.mode,
                "argv": sys.argv[1:],
                "seed": args.seed,
                "scratch": base,
                "sysdir_source": srcdir,
                "summary": {
                    "layouts": len(layouts),
                    "run": len(built),
                    "passed": counts["PASS"],
                    "failed": counts["FAIL"] + counts["FLAKY"],
                    "timeout": counts["TIMEOUT"],
                    "skipped": counts["SKIP"],
                    "invalid_layout": len(invalid),
                    "ref_warned": len(warned),
                    "unbuildable": len(unbuildable),
                    "unobserved_passes": len(unobserved),
                    "fingerprints": len(groups),
                },
                "layouts": [{
                    "name": l.name,
                    "store": l.store, "fpath": l.fpath,
                    "compinit": l.compinit, "dump": l.dump,
                    "security": l.security,
                    "note": l.note,
                    "spec": l.spec_note(),
                    "dirs": l.dirs,
                    "notes": l.notes,
                    "unbuildable": l.unbuildable,
                    "preflight": list(l.preflight) if l.preflight else None,
                } for l in layouts],
                "fingerprints": fingerprint_doc(groups),
                "results": [to_json(v) for v in results],
            })
        return 1 if (failures or counts["TIMEOUT"] or counts["SKIP"] or invalid
                     or unbuildable or crashed(counts)) else 0
    finally:
        for lay in layouts:
            for path, mode in lay.restore:
                try:
                    os.chmod(path, mode)
                except OSError:
                    pass
        if args.layout_keep:
            print("# kept scratch tree: %s" % base)
        else:
            shutil.rmtree(base, ignore_errors=True)


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
    # ── storage and lookup (how completers are stored and found) ──
    ap.add_argument("--layout-fuzz", type=int, default=0, metavar="N",
                    help="run N STORAGE/LOOKUP layouts: .zwc digest "
                         "composition (plain / digest / stale digest / digest "
                         "shadowed by a newer file / explicit .zwc element / "
                         "truncated digest), fpath composition (duplicate, "
                         "missing, unreadable, symlinked, two dirs claiming "
                         "one command, a #compdef tag naming another command), "
                         "the compinit mode matrix (-C/-i/-u/-D/-d/bare) and "
                         "the dump state (missing, stale, corrupt, written by "
                         "the OTHER shell). Both shells always get the "
                         "byte-identical layout. A layout the reference zsh "
                         "itself refuses is counted as INVALID-LAYOUT, never "
                         "as a finding and never as a pass.")
    ap.add_argument("--layout-only", default=None, metavar="NAMES",
                    help="run only these comma-separated layout names "
                         "(--layout-list prints them)")
    ap.add_argument("--layout-random", type=int, default=0, metavar="N",
                    help="run N SEEDED RANDOM layouts instead of the curated "
                         "catalog: a store, an fpath composition, a compinit "
                         "mode, a dump state and a security condition drawn "
                         "independently. (seed, index) reproduces one exactly.")
    ap.add_argument("--layout-list", action="store_true",
                    help="print the layout catalog, the axes and the "
                         "documented rule each one holds the shells to, then "
                         "exit without booting a shell")
    ap.add_argument("--layout-cases", default=None, metavar="BUFFERS",
                    help="comma-separated case buffers the layouts are judged "
                         "on (default 'zz01 ', the synthetic command the "
                         "scratch fpath provides a completer for)")
    ap.add_argument("--layout-keep", action="store_true",
                    help="keep the scratch fpath tree instead of removing it, "
                         "so a failing layout can be inspected by hand")
    ap.add_argument("--dump-xshell", action="store_true",
                    help="answer the cross-shell .zcompdump question directly: "
                         "each shell writes a dump for one controlled layout, "
                         "then each shell reads BOTH dumps, and the state each "
                         "ends up in is printed. Not scored.")
    # ── coverage-guided fuzzing ──
    ap.add_argument("--guided", type=int, default=0, metavar="N",
                    help="run N cells COVERAGE-GUIDED: keep an input in the "
                         "corpus when it produced a feature no earlier input "
                         "produced, not only when it failed, and bias the next "
                         "draw towards the classes that have been buying "
                         "features per second. Judging is untouched — guidance "
                         "only chooses which inputs get run.")
    ap.add_argument("--guide-off", action="store_true",
                    help="run the --guided loop with the feedback DISABLED: "
                         "same loop, same RNG stream, uniform parent draw, no "
                         "coverage retention. The blind control for measuring "
                         "whether guidance is actually worth anything.")
    ap.add_argument("--cov-log", action="store_true",
                    help="add REAL execution coverage from zshrs's own tracing "
                         "(ZSHRS_LOG=compsys_args=debug,ftime) on top of the "
                         "output-shape signal: which completer resolved, which "
                         "tag context, which _arguments branch, and which "
                         "compsys shell functions ran. Requires --jobs 1 — the "
                         "log is one shared append-only file whose lines carry "
                         "no pid, so a slice can only be attributed to a cell "
                         "while this harness is its sole writer.")
    ap.add_argument("--explore-floor", type=float, default=0.25, metavar="P",
                    help="probability a draw ignores the yield table entirely "
                         "and picks uniformly. The floor is what stops a class "
                         "being starved permanently on one unlucky sample.")
    ap.add_argument("--cov-corpus-max", type=int, default=160, metavar="N",
                    help="cap on coverage-retained (`cov_*.json`) corpus "
                         "entries. At the cap the least informative one is "
                         "evicted; a `fp_*` reproducer is never evicted.")
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

    if args.layout_list:
        return run_layout_list(args)

    if args.dump_xshell:
        return run_dump_xshell(args, env)

    if args.layout_fuzz > 0 or args.layout_only or args.layout_random > 0:
        if args.combo_sequence not in KEY_SEQUENCES:
            sys.exit("unknown --combo-sequence: %s" % args.combo_sequence)
        if (args.layout_only or args.layout_random > 0) and args.layout_fuzz <= 0:
            args.layout_fuzz = len(layout_catalog())
        return run_layout_fuzz(args, env)

    if args.style_fuzz_list > 0:
        return run_style_fuzz_list(args)

    if args.style_fuzz > 0:
        if args.combo_sequence not in KEY_SEQUENCES:
            sys.exit("unknown --combo-sequence: %s" % args.combo_sequence)
        return run_style_fuzz(args, env, dump, fpath_dirs)

    if args.guided > 0:
        if args.cov_log and args.jobs > 1:
            sys.exit("--cov-log needs --jobs 1: zshrs's log is a single shared "
                     "append-only file and its lines carry no pid, so with "
                     "concurrent children a byte slice cannot be attributed to "
                     "the cell that produced it. Drop --cov-log to keep --jobs "
                     "%d (shape coverage is unaffected by concurrency)."
                     % args.jobs)
        return run_guided(args, env, dump, fpath_dirs)

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
    crashes = []
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
            if v.status in CRASH_STATUSES:
                crashes.append(v)
                print_crash(v, args)
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
    print("# categories: PASS=%d FAIL=%d TIMEOUT=%d SKIP=%d "
          "REF-CRASHED=%d TEST-CRASHED=%d"
          % (passed, len(failures), len(timeouts), len(skipped),
             sum(1 for v in crashes if v.status == "REF-CRASHED"),
             sum(1 for v in crashes if v.status == "TEST-CRASHED")))
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
    if crashes:
        print("# %d cell(s) where a SHELL CRASHED (not run to a comparison, not "
              "passed, and NOT a TIMEOUT — a dead shell is not a slow one):"
              % len(crashes))
        for v in crashes[:20]:
            print("#   --case %s --keys %s   (%s)"
                  % (shlex.quote(v.case.buffer), ",".join(v.keys), v.detail))
        if len(crashes) > 20:
            print("#   ... %d more" % (len(crashes) - 20))
        print_crash_counts({"REF-CRASHED": sum(1 for v in crashes
                                               if v.status == "REF-CRASHED"),
                            "TEST-CRASHED": sum(1 for v in crashes
                                                if v.status == "TEST-CRASHED")})
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
    return 1 if (failures or timeouts or skipped or crashes) else 0


if __name__ == "__main__":
    sys.exit(main())

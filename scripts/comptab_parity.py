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

Exit status: 0 only when every case is byte-identical.
"""

from __future__ import annotations

import argparse
import difflib
import fcntl
import glob
import json
import os
import pty
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
        self.events = []         # (phase, settle-outcome) per wait
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
        while True:
            now = time.monotonic()
            if now - start > max_wait:
                outcome = CAPPED if seen else NO_OUTPUT
                self.events.append((phase, outcome))
                return outcome
            got = self._read_once(0.05)
            now = time.monotonic()
            if got:
                seen, last = True, now
            elif not seen:
                if now - start > first_wait:
                    self.events.append((phase, NO_OUTPUT))
                    return NO_OUTPUT
            elif now - last >= self.settle:
                self.events.append((phase, QUIET))
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
        for phase, outcome in self.events:
            if outcome == CAPPED:
                out.append("settle capped after %s — screen may be mid-render"
                           % phase)
            elif outcome == NO_OUTPUT and phase.startswith("key "):
                out.append("no output at all after %s" % phase)
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


def render(rows):
    return "\n".join("  %2d| %s" % (i, r) for i, r in enumerate(rows)) or "  <empty>"


def repro_cmd(args, buf, keys):
    """A command line that replays exactly this cell.

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
    if args.zstyle:
        cmd += ["--zstyle", shlex.quote(args.zstyle)]
    cmd += ["--case", shlex.quote(buf), "--keys", ",".join(keys)]
    cmd += ["--rows", str(args.rows), "--cols", str(args.cols)]
    if args.settle != 300:
        cmd += ["--settle", str(args.settle)]
    return " ".join(cmd)


def print_failure(v, args):
    """Everything needed to act on a FAIL without re-running it."""
    ref, test = v.ref, v.test
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
    print("  " + repro_cmd(args, v.case.buffer, v.keys))
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

def run_case(args, env, init_file, case, seq_name, keys):
    """Returns a Verdict whose status is PASS / FAIL / FLAKY.

    FLAKY is a FAIL that did not reproduce on the confirm run — reported as a
    failure with the nondeterminism called out, never scored as a pass.
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

    if v.status != "PASS" and args.confirm > 0:
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
        """True when ANY case (or just `only`) diverges under this subset."""
        init = init_for(subset)
        for case in (only or cases):
            if run_case(args, env, init, case, seq, keys).status != "PASS":
                return case
        return None

    bad = 0
    for n in range(args.random_combos):
        rng = random.Random((args.seed << 20) ^ n)
        subset = random_subset(statements, args.combo_keep, rng)
        culprit = diverges(subset)
        if culprit is None:
            print("PASS combo %-4d (%3d statements)" % (n, len(subset)))
            sys.stdout.flush()
            continue

        bad += 1
        print("FAIL combo %-4d (%3d statements) on %r"
              % (n, len(subset), culprit.buffer))
        sys.stdout.flush()

        # Does it still diverge with NO styles at all? If so the combo is
        # irrelevant — the case diverges under compsys defaults — and shrinking
        # would misleadingly name whichever statement survived last.
        if diverges([], only=[culprit]) is not None:
            print("     config-INDEPENDENT: diverges with zero zstyles set")
            sys.stdout.flush()
            continue

        minimal = subset
        if args.shrink:
            minimal = shrink(
                subset,
                lambda sub: diverges(sub, only=[culprit]) is not None,
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
    return 1 if bad else 0


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
                    help="delta-debug a diverging combo to its minimal statement set")
    ap.add_argument("--no-shrink", dest="shrink", action="store_false")
    ap.add_argument("--shrink-probes", type=int, default=60,
                    help="max shrink re-runs per failing combo")
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

    dump = None if args.no_dump else resolve_dump(args.dump)
    fpath_dirs = user_fpath()
    env = child_env()

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
    if args.case is not None:
        cases = [adhoc_case(args.case)]
    elif args.corpus:
        with open(args.corpus) as f:
            cases = [adhoc_case(l.rstrip("\n")) for l in f
                     if l.strip() and not l.lstrip().startswith("#")]
    else:
        shared = cases_by_tag(args.tag)
        if args.skip_optional:
            shared = [c for c in shared if "optional" not in c.tags]
        cases = [] if args.discover_only else list(shared)
    if args.discover:
        found = discover_cases(args.discover, args.discover_all)
        have = {c.buffer for c in cases}
        cases += [c for c in found if c.buffer not in have]

    cells = [(c, name) for c in cases for name in seq_names]

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
    print()

    jsonl = open(args.jsonl, "w") if args.jsonl else None
    jsonl_lock = threading.Lock()
    started = time.monotonic()

    def work(cell):
        case, seq_name = cell
        v = run_case(args, env, init_file, case, seq_name, seq_keys[seq_name])
        if jsonl:
            with jsonl_lock:
                jsonl.write(json.dumps(to_json(v)) + "\n")
                jsonl.flush()
        return v

    if args.jobs > 1:
        from concurrent.futures import ThreadPoolExecutor
        pool = ThreadPoolExecutor(max_workers=args.jobs)
        # `map` yields in submission order, so the log stays deterministic no
        # matter which cell finishes first.
        stream = pool.map(work, cells)
    else:
        pool = None
        stream = (work(c) for c in cells)

    passed = 0
    results = []
    failures = []
    warned = attr_only = cursor_only = stream_only = 0
    try:
        for v in stream:
            results.append(v)
            # This exact line shape is parsed by scripts/gen_compsys_parity_report.py
            # (status, sequence, repr(buffer), optional "  (detail)") — keep it.
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
            failures.append(v)
            print_failure(v, args)
            sys.stdout.flush()
    finally:
        if pool:
            pool.shutdown(wait=True)
        if jsonl:
            jsonl.close()

    elapsed = time.monotonic() - started
    # Consumed by gen_compsys_parity_report.py and parity_matrix.py — keep the
    # wording.
    print("\n# %d passed, %d failed, %d cell(s)" % (passed, len(failures), len(cells)))
    print("# elapsed: %.1fs (%.1fs/cell)" % (elapsed, elapsed / max(1, len(cells))))
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
                "cells": len(cells),
                "warned": warned,
                "attr_only": attr_only,
                "cursor_only": cursor_only,
                "stream_only": stream_only,
                "elapsed_seconds": round(elapsed, 1),
            },
            "results": [to_json(v) for v in results],
        }
        text = json.dumps(doc, indent=2, sort_keys=False)
        if args.json == "-":
            print(text)
        else:
            with open(args.json, "w") as f:
                f.write(text + "\n")
            print("# json: %s" % args.json)
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())

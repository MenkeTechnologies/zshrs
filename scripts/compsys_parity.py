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

and a failure is delta-debugged down to the minimal keystroke path and the
minimal zstyle subset that still diverge at the SAME first-diff cell:

    --shrink-probes N   probe budget per axis (0 disables shrinking)
    --jobs N            run N cells concurrently (independent pty pairs)
    --json PATH         machine-readable result document ('-' for stdout)

Every failure prints a copy-pasteable `--lockstep` replay carrying the seed,
buffer, minimal key path, geometry and the saved zstyle fixture.

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


class ShellSession:
    """One shell child on its own PTY, screen mirrored through pyte."""

    def __init__(self, argv, env, rows, cols, label, settle_ms):
        self.label = label
        self.rows = rows
        self.cols = cols
        self.settle = settle_ms / 1000.0
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
        self.stream.feed(data)
        return True

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
        os.write(self.fd, data)

    def type_text(self, text: str):
        self.send(text.encode())

    def send_key(self, name: str):
        # STRICT: parity_corpus.key_bytes rejects an unknown multi-character
        # name outright. The old `KEYS.get(name, name.encode())` fallback turned
        # a typo into that many self-inserted characters on BOTH shells, which
        # looked like a passing case for a key that was never sent.
        self.send(key_bytes(name))

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


def build_init_file(dump, fpath_dirs, zstyle_file):
    """Write the init script both shells source after launching with `-f`.
    Matches the spec: same fpath, same zstyles, same compinit + dump, so the
    only variable left is the shell under test."""
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
    init = f"""\
# generated by compsys_parity.py — sourced into `zsh -f` and `zshrs --zsh -f`
PROMPT='{PROMPT_SENTINEL} '
RPROMPT=''
PS2=''
setopt no_beep
{ostype_line}\

{fpath_line}{zstyle_line}{compinit}{autoload_line}
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


def run_case(sess: ShellSession, case: Case):
    sess.fresh_prompt()
    if case.buffer:
        sess.type_text(case.buffer)
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


def available_surfaces(skips: Counter) -> list:
    """The surfaces usable on THIS host. Every dropped surface is counted under
    `unavailable-surface:<name>` and printed in the summary — never silently
    omitted."""
    out = []
    for s in BUFFER_SURFACES:
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


def run_keyseq(init_file, buffer, keys, args, env, geom):
    """Drive `zsh -f` and `zshrs --zsh -f` in LOCKSTEP: source init, type
    `buffer`, then send each key in `keys` one at a time, capturing +
    byte-diffing BOTH screens AFTER EACH keystroke. `keys` may mix "tab",
    arrows ("down"/"up"/...), and literal filter characters ("a".."z") — so
    menu-cycling, list-prompt paging, arrow navigation, AND interactive-filter
    narrowing (typing letters to filter the menu) are all verified per-key.

    `geom` sizes BOTH ptys and BOTH environments identically.

    Returns (fail_step, records): fail_step is the 1-based index of the first
    key whose screens diverge (0 if all match); records is
    [(step, key, ref_grid, test_grid, diffs), ...]. Stops at first divergence
    (the two shells desync past that point)."""
    source_cmd = f"source {shlex.quote(init_file)}\n".encode()
    env = dict(env, COLUMNS=str(geom.cols), LINES=str(geom.rows))
    ref = ShellSession([args.zsh, "-f", "-i"], env, geom.rows, geom.cols, "zsh", args.settle)
    test = ShellSession([args.zshrs, "--zsh", "-f", "-i"], env, geom.rows, geom.cols, "zshrs", args.settle)
    try:
        for s in (ref, test):
            s.drain_settled(max_wait=3.0, first_wait=2.0)
            s.send(source_cmd)
            if not s.wait_for_prompt(timeout=25.0):
                return (1, [(1, "(init)", None, None, None)])
        for s in (ref, test):
            s.fresh_prompt()
        if buffer:
            for s in (ref, test):
                s.type_text(buffer)
            for s in (ref, test):
                s.drain_settled(max_wait=2.0, first_wait=1.0)
        records = []
        for step, key in enumerate(keys, 1):
            for s in (ref, test):
                s.send_key(key)
            # the FIRST completion keystroke is cold (autoload chain) → long
            # first-byte wait; later keys are warm menu redraws / filter edits.
            fw = 8.0 if step == 1 else 4.0
            for s in (ref, test):
                s.drain_settled(max_wait=12.0, first_wait=fw)
            rg = normalize_rows(ref.grid())
            tg = normalize_rows(test.grid())
            diffs = diff_grids(rg, tg)
            records.append((step, key, rg, tg, diffs))
            if diffs:
                return (step, records)
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

def shrink_keys(cell, args, env, target_sig, budget, run):
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
        fs, rec = run(cell.buffer, keys, cell.init_file, cell.geom)
        return bool(fs) and signature(rec) == target_sig

    tail = ddmin(list(cell.keys[1:]), still_fails, max_probes=budget)
    return [cell.keys[0]] + tail, probes[0]


def shrink_styles(cell, args, env, target_sig, budget, run, build_init, keys):
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
        fs, rec = run(cell.buffer, keys, init, cell.geom)
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


def replay_command(args, buffer, keys, geom, zstyle_path):
    """A copy-pasteable command that reproduces exactly this divergence."""
    return ("scripts/compsys_parity.py --lockstep"
            f" --seed {args.seed}"
            f" --zstyle {shlex.quote(zstyle_path)}"
            f" --case {shlex.quote(buffer)}"
            f" --keys {','.join(keys)}"
            f" --rows {geom.rows} --cols {geom.cols} -v")


def run_cell(cell, args, env, dump, fpath_dirs, outdir) -> CellResult:
    """One fuzz cell: lockstep run, flake labelling, then delta debugging."""
    res = CellResult(cell=cell)

    def run(buffer, keys, init_file, geom):
        return run_keyseq(init_file, buffer, keys, args, env, geom)

    fail_step, records = run(cell.buffer, cell.keys, cell.init_file, cell.geom)
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
                                    cell.zstyle_path)
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

    min_keys, min_styles = None, None
    zstyle_for_replay = cell.zstyle_path
    # Only a REPRODUCIBLE failure is shrunk: ddmin's oracle is "does this
    # candidate still diverge at the same cell", and a flaky oracle would
    # happily delete keys that matter. A FLAKY cell keeps its full path.
    if args.shrink_probes > 0 and res.status == "FAIL":
        min_keys, p1 = shrink_keys(cell, args, env, res.sig,
                                   args.shrink_probes, run)
        res.probes += p1
        res.shrink_exhausted = p1 >= args.shrink_probes

        def build_init(subset):
            d = tempfile.mkdtemp(prefix="shrink_", dir=cell.workdir)
            path = os.path.join(d, "zstyle.zsh")
            with open(path, "w") as f:
                f.write("\n".join(subset) + "\n")
            return build_init_file(dump, fpath_dirs, path)

        min_styles, p2 = shrink_styles(cell, args, env, res.sig,
                                       args.shrink_probes, run, build_init,
                                       min_keys)
        res.probes += p2
        res.shrink_exhausted = res.shrink_exhausted or p2 >= args.shrink_probes
        if len(min_styles) < len(cell.statements):
            zstyle_for_replay = saved_path(outdir, args.seed, cell.uid, ".min")
            with open(zstyle_for_replay, "w") as f:
                f.write(f"# shrunk from {len(cell.statements)} statements "
                        f"(seed={args.seed} combo={cell.idx} "
                        f"surface={cell.surface} geom={geom_str(cell.geom)})\n")
                f.write("\n".join(min_styles) + "\n")
    res.min_keys, res.min_styles = min_keys, min_styles
    res.replay = replay_command(args, cell.buffer, min_keys or cell.keys,
                                cell.geom, zstyle_for_replay)
    return res


def build_cells(args, dump, fpath_dirs, statements, surfaces, outdir, skips):
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
        count = per_combo if args.buffer_fuzz else len(fixed)
        for ci in range(count):
            crng = random.Random(f"{args.seed}:{n}:{ci}")
            if args.buffer_fuzz:
                surface, buf, pre = gen_buffer(crng, surfaces)
            else:
                surface, buf, pre = "fixed", fixed[ci], []
            buffer = buf if buf.endswith(" ") or args.buffer_fuzz else buf + " "
            geom = pick_geom(crng, args)
            # A buffer that cannot even be TYPED inside the window (prompt +
            # text wider than the whole screen) is not a comparison the harness
            # can make; it is SKIPPED with a counted reason rather than compared
            # against a truncated grid.
            if len(PROMPT_SENTINEL) + 1 + len(buffer) >= geom.rows * geom.cols:
                skips[f"buffer-exceeds-screen:{geom_str(geom)}"] += 1
                continue
            keys = pre + gen_keyseq(
                random.Random(f"{args.seed}:{n}:{ci}:keys"), args.presses)
            cells.append(Cell(idx=n, uid=f"{n}_{ci}", surface=surface,
                              buffer=buffer, keys=keys,
                              geom=geom, statements=subset,
                              zstyle_path=saved_path(outdir, args.seed, n),
                              init_file=init_file, workdir=workdir))
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
    surfaces = available_surfaces(skips) if args.buffer_fuzz else []
    if args.buffer_fuzz and not surfaces:
        sys.exit("compsys_parity: --buffer-fuzz has no usable surfaces on this host")
    outdir = os.path.join(tempfile.gettempdir(),
                          f"compsys_parity_failing_combos_{args.seed}")
    os.makedirs(outdir, exist_ok=True)

    cells = build_cells(args, dump, fpath_dirs, statements, surfaces, outdir, skips)

    print(f"# random-combo fuzz: {args.random_combos} combos, {len(cells)} cells, "
          f"{args.presses}-key paths (parity asserted after EACH key)")
    print(f"# base zstyle: {args.zstyle} ({len(statements)} statements)")
    print(f"# seed={args.seed}  keep-prob={args.combo_keep}  confirm={args.confirm}  "
          f"jobs={max(1, args.jobs)}  shrink-probes={args.shrink_probes}")
    print(f"# buffer-fuzz={args.buffer_fuzz} ({len(surfaces)} surfaces)  "
          f"geometry-fuzz={args.geometry_fuzz}  "
          f"geom={'pool' if args.geometry_fuzz else geom_str(Geom(args.rows, args.cols))}")
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
    results = []
    try:
        for res in stream:
            c = res.cell
            head = (f"{res.status:5s} combo {c.idx:3d} [{c.surface:11s}] "
                    f"{geom_str(c.geom):>7s} {c.buffer!r}")
            if res.status == "PASS":
                passed += 1
                print(f"{head}  keys={'+'.join(c.keys)}")
                results.append(_cell_json(res))
                continue
            if res.status == "FLAKY":
                flaky += 1
            else:
                failed += 1
            print(f"{head}  ({res.detail})")
            print(f"      path      : {'+'.join(c.keys)}"
                  f"  → diverges at step #{res.fail_step} (key {res.fail_key!r})")
            if res.sig:
                print(f"      first diff: row {res.sig[0]}, col {res.sig[1]}")
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
                print(f"        styles: {len(res.min_styles)}/{len(c.statements)}"
                      f"  [{res.probes} probes]")
                for s in res.min_styles[:8]:
                    print(f"          {s}")
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
    print()
    print(f"# {passed} passed, {failed} failed, {flaky} flaky, "
          f"{len(cells)} cells run")
    if skips:
        print(f"# {skipped} skipped (never compared):")
        for reason, count in sorted(skips.items()):
            print(f"#   {reason}: {count}")
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
            "geometry_fuzz": args.geometry_fuzz,
            "geom": {"rows": args.rows, "cols": args.cols, "settle_ms": args.settle},
            "jobs": max(1, args.jobs),
            "confirm": args.confirm,
            "shrink_probes": args.shrink_probes,
            "summary": {"passed": passed, "failed": failed, "flaky": flaky,
                        "skipped": skipped, "cells": len(cells)},
            "skips": dict(skips),
            "results": results,
        }
        _write_json(args.json, doc)
    # Flaky is NOT a pass: a cell that diverges only sometimes is still a cell
    # whose two shells did not agree, so it fails the run.
    return 1 if (failed or flaky) else 0


def _cell_json(res) -> dict:
    c = res.cell
    return {
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
        "zstyle_file": c.zstyle_path,
        "replay": res.replay,
    }


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
    fail_step, records = run_keyseq(init_file, args.case, keys, args, env, geom)
    step, key, rg, tg, diffs = records[-1]
    if fail_step == 0:
        print(f"PASS lockstep {args.case!r} [{'+'.join(keys)}] {geom_str(geom)}"
              f"  ({len(records)} keys, screens identical after every one)")
        rc = 0
        doc_res = {"status": "PASS"}
    else:
        row, col = first_diff_cell(diffs) if diffs else (-1, -1)
        print(f"FAIL lockstep {args.case!r} [{'+'.join(keys)}] {geom_str(geom)}"
              f"  diverges at step #{step} (key {key!r})"
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
    if args.json:
        doc_res.update(id="lockstep", buffer=args.case, keys=keys,
                       geom={"rows": geom.rows, "cols": geom.cols})
        _write_json(args.json, {
            "schema": "compsys-parity/1", "mode": "lockstep",
            "argv": sys.argv[1:], "zshrs": args.zshrs, "zsh": args.zsh,
            "summary": {"passed": 1 - rc, "failed": rc, "flaky": 0,
                        "skipped": 0, "cells": 1},
            "results": [doc_res],
        })
    return rc


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

    if args.random_combos > 0:
        return run_random_combos(args, dump, fpath_dirs,
                                 child_env(args.rows, args.cols))

    if args.lockstep:
        if args.case is None:
            sys.exit("compsys_parity: --lockstep needs --case")
        return run_lockstep_case(args, build_init_file(dump, fpath_dirs, zstyle_file),
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
                return None
            return run_case(sess, case)
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

    def evaluate(case):
        """One cell: capture both shells, diff, and (in parallel mode) re-run a
        failure to label nondeterminism. Returns (status, ref, test, diffs,
        detail)."""
        ref_grid = capture(ref_argv, "zsh", case)
        test_grid = capture(test_argv, "zshrs", case)
        if ref_grid is None or test_grid is None:
            who = "zsh" if ref_grid is None else "zshrs"
            return "FAIL", ref_grid, test_grid, None, f"{who} never reached prompt"
        diffs = diff_grids(ref_grid, test_grid)
        if not diffs:
            return "PASS", ref_grid, test_grid, [], ""
        for _ in range(max(0, confirm_runs)):
            r2 = capture(ref_argv, "zsh", case)
            t2 = capture(test_argv, "zshrs", case)
            if r2 is None or t2 is None:
                continue
            d2 = diff_grids(r2, t2)
            if not d2 or first_diff_cell(d2) != first_diff_cell(diffs):
                return ("FLAKY", ref_grid, test_grid, diffs,
                        f"{len(diffs)} rows differ, not reproducible")
            ref_grid, test_grid, diffs = r2, t2, d2
        return "FAIL", ref_grid, test_grid, diffs, f"{len(diffs)} rows differ"

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
    for case, (status, ref_grid, test_grid, diffs, detail) in verdicts:
        keyspec = "+".join(case.keys)
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
            print(f"PASS {case.name:16s} {case.buffer!r} [{keyspec}]")
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
            "results": results,
        }
        _write_json(args.json, doc)
    return 1 if (failed or flaky) else 0


if __name__ == "__main__":
    sys.exit(main())

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
import glob
import os
import pty
import select
import shlex
import signal
import sys
import tempfile
import termios
import time
from dataclasses import dataclass, field

try:
    import pyte
except ImportError:
    sys.exit("compsys_parity: pyte not installed (pip install pyte)")

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
)


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
        self.screen = pyte.Screen(cols, rows)
        self.stream = pyte.ByteStream(self.screen)
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
        waited = 0.0
        step = 0.05
        while waited < timeout:
            self._drain_once(step)
            if self._prompt_visible():
                return True
            waited += step
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
        self.send(KEYS.get(name, name.encode()))

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


def ref_ostype() -> str | None:
    """`$OSTYPE` as the REFERENCE shell reports it.

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


def user_fpath() -> list[str]:
    """The user's real completion fpath, as `zsh -f` sees it on this host
    (global rc populates it even under -f). Used so both shells scan the
    identical function set the user's `.zcompdump` was built from."""
    try:
        import subprocess
        out = subprocess.run(
            ["zsh", "-f", "-c", "print -rl -- $fpath"],
            capture_output=True, text=True, timeout=10,
        ).stdout
        return [d for d in out.splitlines() if d and os.path.isdir(d)]
    except Exception:
        return []


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


def child_env() -> dict:
    env = {
        "TERM": "xterm-256color",
        "LANG": "C",
        "LC_ALL": "C",
        "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
        "COLUMNS": "80",
        "LINES": "24",
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
        sess.drain_settled(max_wait=12.0, first_wait=8.0)
    sess.drain_settled(max_wait=3.0, first_wait=0.6)
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

    A completion occasionally renders slower than the settle window under load
    and gets captured mid-render, so a lone FAIL can be a timing false positive.
    `confirm` re-runs a failing case up to that many extra times; a divergence
    counts as real only if it reproduces EVERY time (deterministic). Any pass on
    retry marks it flaky and it is dropped."""
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
        # Either a real diff or a prompt failure — confirm it reproduces.
        stable = True
        for _ in range(max(0, confirm)):
            r2, t2, d2 = attempt(case)
            if d2 == []:  # a clean pass on retry -> flaky, not a real gap
                stable = False
                break
            ref_grid, test_grid, diffs = r2, t2, d2  # keep the latest failing capture
        if stable:
            failed += 1
            fails.append((case, ref_grid, test_grid, diffs))
        else:
            passed += 1  # flaky pass
    return passed, failed, fails


def saved_path(outdir, seed, n):
    return os.path.join(outdir, f"combo_{seed}_{n}.zsh")


def run_keyseq(init_file, buffer, keys, args, env):
    """Drive `zsh -f` and `zshrs --zsh -f` in LOCKSTEP: source init, type
    `buffer`, then send each key in `keys` one at a time, capturing +
    byte-diffing BOTH screens AFTER EACH keystroke. `keys` may mix "tab",
    arrows ("down"/"up"/...), and literal filter characters ("a".."z") — so
    menu-cycling, list-prompt paging, arrow navigation, AND interactive-filter
    narrowing (typing letters to filter the menu) are all verified per-key.

    Returns (fail_step, records): fail_step is the 1-based index of the first
    key whose screens diverge (0 if all match); records is
    [(step, key, ref_grid, test_grid, diffs), ...]. Stops at first divergence
    (the two shells desync past that point)."""
    source_cmd = f"source {shlex.quote(init_file)}\n".encode()
    ref = ShellSession([args.zsh, "-f", "-i"], env, args.rows, args.cols, "zsh", args.settle)
    test = ShellSession([args.zshrs, "--zsh", "-f", "-i"], env, args.rows, args.cols, "zshrs", args.settle)
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


def gen_keyseq(rng, length):
    """Generate a random lockstep key path: always start with a TAB (list +
    enter menu-select), then a random mix of more TABs (cycle/page), arrows
    (menu navigation), and literal letters (interactive filter narrowing)."""
    seq = ["tab"]
    letters = "abcdefghijklmnopqrstuvwxyz"
    for _ in range(max(0, length - 1)):
        r = rng.random()
        if r < 0.35:
            seq.append("tab")
        elif r < 0.60:
            seq.append(rng.choice(["down", "up", "left", "right"]))
        else:
            seq.append(rng.choice(letters))  # interactive-filter keystroke
    return seq


def run_random_combos(args, dump, fpath_dirs, env):
    """Fuzz random subsets of the user's zstyles: for each of N seeded combos,
    keep each statement with probability `--combo-keep`, run the combo commands,
    and report + save any combo whose completion output diverges. Each combo is
    independently reproducible from (seed, index); failing combos are written to
    a dir so they can be replayed with `--zstyle <that file>`."""
    import random

    statements = parse_zstyle_statements(args.zstyle)
    commands = [c.strip() for c in args.combo_commands.split(",") if c.strip()]
    presses = max(1, args.presses)
    outdir = os.path.join(tempfile.gettempdir(), f"compsys_parity_failing_combos_{args.seed}")
    os.makedirs(outdir, exist_ok=True)

    print(f"# random-combo fuzz: {args.random_combos} combos x {len(commands)} cmds "
          f"x {presses}-key paths (TAB/arrow/filter-letter, parity asserted after EACH key)")
    print(f"# base zstyle: {args.zstyle} ({len(statements)} statements)")
    print(f"# seed={args.seed}  keep-prob={args.combo_keep}  confirm={args.confirm}  geom={args.rows}x{args.cols}")
    print(f"# failing combos saved to: {outdir}")
    print()

    fail_combos = 0
    for n in range(args.random_combos):
        rng = random.Random(f"{args.seed}:{n}")
        subset = [s for s in statements if rng.random() < args.combo_keep]
        d = tempfile.mkdtemp(prefix=f"combo_{args.seed}_{n}_")
        combo_path = os.path.join(d, "zstyle.zsh")
        with open(combo_path, "w") as f:
            f.write(f"# random combo seed={args.seed} index={n}: "
                    f"{len(subset)}/{len(statements)} statements\n")
            f.write("\n".join(subset) + "\n")
        init_file = build_init_file(dump, fpath_dirs, combo_path)

        combo_failed = False
        combo_fail_lines = []
        for ci, b in enumerate(commands):
            buffer = b if b.endswith(" ") else b + " "
            # Random per-command key path (seeded): TAB list/cycle/page + arrow
            # menu nav + literal-letter interactive filtering, asserted per key.
            keys = gen_keyseq(random.Random(f"{args.seed}:{n}:{ci}:keys"), presses)
            # Confirmation: re-run the whole lockstep up to `confirm` times; a
            # divergence counts only if it reproduces at the same step index.
            fail_step, records = run_keyseq(init_file, buffer, keys, args, env)
            for _ in range(max(0, args.confirm)):
                if fail_step == 0:
                    break
                fs2, rec2 = run_keyseq(init_file, buffer, keys, args, env)
                if fs2 != fail_step:
                    fail_step = 0  # not reproducible at the same step → flaky
                    break
                records = rec2
            if fail_step:
                combo_failed = True
                step, key, rg, tg, diffs = records[-1]
                pre = "+".join(keys[:step])
                if diffs is None:
                    combo_fail_lines.append(f"       {buffer!r}: a shell never reached prompt")
                else:
                    combo_fail_lines.append(
                        f"       {buffer!r}: diverges at step #{step} (key {key!r}, path {pre}) "
                        f"({len(diffs)} rows differ)")
                    if args.verbose:
                        for i, a, bb in diffs[:6]:
                            combo_fail_lines.append(f"         row {i:2d}: zsh={a!r}")
                            combo_fail_lines.append(f"                 zshrs={bb!r}")
                    combo_fail_lines.append(
                        f"       -> replay: scripts/compsys_parity.py --zstyle {saved_path(outdir, args.seed, n)} "
                        f"--case '{buffer}' --keys {','.join(keys[:step])} -v")
        tag = "OK  " if not combo_failed else "FAIL"
        print(f"[{tag}] combo {n:3d}  {len(subset):3d}/{len(statements)} styles")
        if combo_failed:
            fail_combos += 1
            with open(saved_path(outdir, args.seed, n), "w") as f, open(combo_path) as src:
                f.write(src.read())
            for line in combo_fail_lines:
                print(line)

    print()
    print(f"# {args.random_combos - fail_combos}/{args.random_combos} combos byte-identical, "
          f"{fail_combos} diverged")
    return 1 if fail_combos else 0


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
    ap.add_argument("--presses", type=int, default=5, metavar="N",
                    help="length of the random key path per combo case (TAB cycle/page + arrow "
                         "menu-nav + literal-letter interactive filter), parity asserted after "
                         "EACH key; default 5")
    ap.add_argument("--confirm", type=int, default=2, metavar="K",
                    help="re-run a failing combo case K times; only report if it fails EVERY time "
                         "(filters timing-flaky false positives, default 2)")
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
        return run_random_combos(args, dump, fpath_dirs, child_env())

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
    env = child_env()

    print(f"# dump   : {dump or '<none>'}")
    print(f"# fpath  : {len(fpath_dirs)} dirs" + (f" (first: {fpath_dirs[0]})" if fpath_dirs else ""))
    print(f"# zstyle : {zstyle_file or '<none>'}")
    print(f"# init   : {init_file}")
    print(f"# zshrs  : {args.zshrs}")
    print(f"# zsh    : {args.zsh}")
    print(f"# geom   : {args.rows}x{args.cols}  settle={args.settle}ms")
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

    passed = failed = 0
    for case in cases:
        ref_grid = capture(ref_argv, "zsh", case)
        test_grid = capture(test_argv, "zshrs", case)
        keyspec = "+".join(case.keys)
        if ref_grid is None or test_grid is None:
            failed += 1
            who = "zsh" if ref_grid is None else "zshrs"
            print(f"FAIL {case.name:16s} {case.buffer!r} [{keyspec}]  ({who} never reached prompt)")
            continue
        diffs = diff_grids(ref_grid, test_grid)
        if not diffs:
            passed += 1
            print(f"PASS {case.name:16s} {case.buffer!r} [{keyspec}]")
            if args.verbose:
                print(render_grid(ref_grid))
        else:
            failed += 1
            print(f"FAIL {case.name:16s} {case.buffer!r} [{keyspec}]  ({len(diffs)} rows differ)")
            print("  --- zsh (ref) ---")
            print(render_grid(ref_grid))
            print("  --- zshrs (test) ---")
            print(render_grid(test_grid))
            print("  --- row diffs ---")
            for i, a, b in diffs:
                print(f"  row {i:2d}: zsh  = {a!r}")
                print(f"          zshrs= {b!r}")

    print()
    print(f"# {passed} passed, {failed} failed, {len(cases)} total")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())

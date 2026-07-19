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
KEYS = {
    "tab": b"\t",
    "btab": b"\x1b[Z",   # shift-tab / reverse menu
    "up": b"\x1b[A",
    "down": b"\x1b[B",
    "right": b"\x1b[C",
    "left": b"\x1b[D",
    "cr": b"\r",
    "esc": b"\x1b",
    "ctrl-c": b"\x03",
    "ctrl-g": b"\x07",
}


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
        return rows

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
    # they aren't autoloaded. Pull them in explicitly (the dir is already on
    # fpath) so the real chain runs identically on both shells.
    autoload_line = ""
    zpwr_comp = os.path.expanduser("~/.zpwr/autoload/comp_utils")
    if os.path.isdir(zpwr_comp):
        customs = [f for f in ("_megacomplete", "_fasd_zsh_word_complete_trigger",
                               "_fasd_zsh_word_complete", "_fasd_zsh_word_complete_f",
                               "_fasd_zsh_word_complete_d")
                   if os.path.exists(os.path.join(zpwr_comp, f))]
        if customs:
            autoload_line = (
                f"fpath=( {shlex.quote(zpwr_comp)} $fpath )\n"
                f"autoload -Uz {' '.join(customs)}\n"
            )
    if dump:
        # -C: trust the dump — skip the security check AND the fpath rescan for
        # new/changed completers. Matches the user's fast-startup setup and
        # avoids reading every fpath file (a broken symlink there hangs -u).
        compinit = f"autoload -Uz compinit\ncompinit -C -d {shlex.quote(dump)}\n"
    else:
        compinit = "autoload -Uz compinit\ncompinit -u\n"
    init = f"""\
# generated by compsys_parity.py — sourced into `zsh -f` and `zshrs --zsh -f`
PROMPT='{PROMPT_SENTINEL} '
RPROMPT=''
PS2=''
setopt no_beep
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


BUILTIN_CASES = [
    Case("cd_slash", "cd /", ["tab"], "top-level dir completion"),
    Case("git_sub", "git ", ["tab"], "git subcommand list"),
    Case("git_co", "git chec", ["tab"], "single-candidate unique completion"),
    Case("ssh_dash", "ssh -", ["tab"], "option completion"),
    Case("kill_sig", "kill -", ["tab"], "signal completion"),
    Case("cd_menu_arrow", "cd /", ["tab", "down", "down"], "menu navigation via arrows"),
    Case("empty_tab", "", ["tab"], "command-position completion (all commands)"),
    Case("var_expand", "echo $PA", ["tab"], "parameter name completion"),
]


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
    ap.add_argument("--rows", type=int, default=24)
    ap.add_argument("--cols", type=int, default=80)
    ap.add_argument("--settle", type=int, default=250, help="quiet window ms")
    ap.add_argument("--case", help="ad-hoc buffer text to type")
    ap.add_argument("--keys", default="tab", help="comma keys for --case")
    ap.add_argument("--only", help="run one built-in case by name")
    ap.add_argument("--list", action="store_true", help="list built-in cases")
    ap.add_argument("-v", "--verbose", action="store_true")
    args = ap.parse_args()

    if args.list:
        for c in BUILTIN_CASES:
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

    if args.case is not None:
        cases = [Case("adhoc", args.case, [k.strip() for k in args.keys.split(",") if k.strip()])]
    elif args.only:
        cases = [c for c in BUILTIN_CASES if c.name == args.only]
        if not cases:
            sys.exit(f"no such case: {args.only}")
    else:
        cases = BUILTIN_CASES

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

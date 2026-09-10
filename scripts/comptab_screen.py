#!/usr/bin/env python3
"""Drive ONE shell in a PTY, send a buffer + keys, print the settled screen.

Companion to comptab_parity.py, which scores cells but does not show what was
on them. This prints CONTENT, so a probe can be checked for LIVENESS: a bare
PASS cannot distinguish "the fix works" from "both shells printed nothing".
Run it once per shell and diff the two outputs yourself.

    comptab_screen.py --shell "zsh -f -i"       --init fx.zsh --buffer 'echo $HIST' --keys tab,tab
    comptab_screen.py --shell "/path/zshrs -f -i" --init fx.zsh --buffer 'echo $HIST' --keys tab,tab

Three gotchas, each of which produces a silently WRONG capture rather than an
error:

1. TERM must be set in the child or ZLE never engages and the keystrokes echo
   literally (you see a bare '^L' in the output). Set here.

2. Never sleep a fixed interval waiting for the shell to be ready. zsh is much
   slower than zshrs to finish compinit, so a fixed wait lands the keys before
   ZLE is listening and they go into the tty buffer in canonical mode. This
   waits for the PS1 marker instead.

3. The fixture should pin a minimal fpath, e.g.

       fpath=( /usr/share/zsh/5.9/functions )
       autoload -Uz compinit
       compinit -u -d ${TMPDIR:-/tmp}/some_scratch_dump

   Otherwise zsh inherits the ambient ~50-directory FPATH and, with no dump,
   scans all of it — which reads as a hang.

Needs CPython + pyte. `python3` on a host where a Rust python shadows it will
die at import; use a real CPython venv.
"""
import os, pty, sys, time, select, argparse, fcntl, termios, struct

# The `python3` on this host's PATH may be pythonrs, a Rust reimplementation.
# It is NOT a drop-in: `sys.argv` arrives correctly and `import pyte` succeeds,
# but `argparse.parse_args()` does not consume the arguments and exits 2, so the
# script dies claiming its required options are missing while the command line
# plainly had them. That reads as user error and has cost real time. Fail with
# the actual reason instead.
#
# (`comptab_parity.py` fails differently under the same interpreter: it dies at
# IMPORT on the PEP 585 `list[str]` annotations, before argparse is reached.)
if "pythonrs" in sys.version:
    raise SystemExit(
        "comptab_screen: refusing to run under pythonrs (%s).\n"
        "  argparse there does not parse arguments, so this would fail as\n"
        "  'required arguments missing' no matter what you passed.\n"
        "  Use a real CPython, e.g. /opt/homebrew/bin/python3, or a venv with pyte."
        % sys.version.split()[0]
    )

ap = argparse.ArgumentParser()
ap.add_argument("--shell", required=True)          # full argv, space separated
ap.add_argument("--init", required=True)           # file to source
ap.add_argument("--buffer", required=True)
ap.add_argument("--keys", default="tab")
ap.add_argument("--rows", type=int, default=40)
ap.add_argument("--cols", type=int, default=110)
ap.add_argument(
    "--env",
    action="append",
    default=[],
    metavar="KEY=VAL",
    help="override or add one child env var; repeatable",
)
ap.add_argument(
    "--settle",
    type=float,
    default=0.5,
    help="quiet interval that counts as settled, in seconds (default 0.5)",
)
ap.add_argument(
    "--budget",
    type=float,
    default=20.0,
    help="max wait per key before giving up AND SAYING SO (default 20s)",
)
ap.add_argument(
    "--lang",
    default="C",
    help="LANG/LC_ALL for the child (default C, matching comptab_parity.py). "
    "Use en_US.UTF-8 only when deliberately testing multibyte behaviour, and "
    "pass it to BOTH shells.",
)
a = ap.parse_args()

import pyte
# Named keys. A name is only needed for something that is not one literal
# character; single characters are sent as themselves (so `--keys tab,s`
# types a TAB then an `s`).
KEYS = {
    "tab": "\t",
    "cr": "\r",
    "nl": "\n",
    "esc": "\x1b",
    "space": " ",
    "bs": "\x7f",
    "del": "\x1b[3~",
    "up": "\x1b[A",
    "down": "\x1b[B",
    "right": "\x1b[C",
    "left": "\x1b[D",
    "home": "\x1b[H",
    "end": "\x1b[F",
    "pgup": "\x1b[5~",
    "pgdn": "\x1b[6~",
}


def keyseq(name):
    """Resolve one --keys token, or die.

    Never fall back to sending the token as literal text. An earlier version
    did (`KEYS.get(k, k)`), so `--keys tab,right` typed the WORD "right" into
    the buffer and the run looked valid while measuring nothing. A silent
    wrong measurement is the failure this whole script exists to prevent, so
    an unknown name is a hard error.
    """
    if name in KEYS:
        return KEYS[name]
    if len(name) == 1:
        return name
    raise SystemExit(
        "comptab_screen: unknown key %r. Named keys: %s. "
        "Anything else must be a single literal character."
        % (name, ", ".join(sorted(KEYS)))
    )


def child_env():
    """Build the child environment FROM SCRATCH, mirroring comptab_parity.py.

    Inheriting the parent environment is not neutral. Two zshrs-only knobs
    decide whether the two shells are even comparable, and both are set by the
    scored harness (`comptab_parity.py` `child_env`), so a probe that omits
    them can disagree with the harness on the same case:

      ZSHRS_NATIVE_ZLE_FX=0     zshrs's autosuggest / syntax highlight have no
                                zsh counterpart. Their ghost text lands on the
                                screen for any buffer with a history hit --
                                `zmodload zsh/<TAB>` rendered a recalled
                                history line here and looked like a divergence.
                                Silences the fx LAYER only; the completion
                                engine is untouched.
      ZSHRS_HIDE_EXT_BUILTINS=1 zshrs ships ~145 builtins zsh lacks (peach,
                                async, zf_*, dbview...). Any listing that
                                enumerates $builtins therefore diverges by
                                construction. Hides them from the `builtins`
                                table and the compctl namespace dump for the
                                comparison only; dispatch is unchanged.

    LANG/LC_ALL default to C for the same reason the harness uses it: the
    locale changes collation and width handling, so it must be pinned and
    identical on both sides. --lang overrides it when multibyte behaviour is
    the thing under test.
    """
    env = {
        "PS1": "READY%% ",
        "TERM": "xterm-256color",
        "LANG": a.lang,
        "LC_ALL": a.lang,
        "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
        "HOME": os.environ.get("HOME", "/tmp"),
        "ZDOTDIR": "/nonexistent-zdotdir",
        "ZSHRS_NATIVE_ZLE_FX": "0",
        "ZSHRS_HIDE_EXT_BUILTINS": "1",
        "RUST_BACKTRACE": "1",
    }
    for item in a.env:
        if "=" not in item:
            raise SystemExit("comptab_screen: --env needs KEY=VAL, got %r" % item)
        k, v = item.split("=", 1)
        env[k] = v
    return env


argv = a.shell.split()
pid, fd = pty.fork()
if pid == 0:
    os.environ.clear()
    os.environ.update(child_env())
    os.execvp(argv[0], argv)

fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", a.rows, a.cols, 0, 0))
screen = pyte.Screen(a.cols, a.rows)
stream = pyte.ByteStream(screen)

def pump(seconds):
    end = time.time() + seconds
    while time.time() < end:
        r, _, _ = select.select([fd], [], [], 0.05)
        if r:
            try:
                data = os.read(fd, 65536)
            except OSError:
                return
            if not data:
                return
            stream.feed(data)

def pump_until_quiet(quiet, budget):
    """Pump until the shell stops emitting for `quiet` seconds, or `budget` runs out.

    Returns (saw_any_output, quiesced). `quiesced=False` means the budget ran
    out while bytes were still arriving, so the screen is mid-render.

    A FIXED pump is the wrong tool here and produced three false findings in one
    session. Debug-build zshrs can be an order of magnitude slower than zsh on
    the same case -- `env <TAB>` answers in 0.21s on zsh and 2.40s on a debug
    zshrs -- so a 1.2s-per-key pump caught zsh and missed zshrs, and an empty
    capture is indistinguishable from "the shell computed nothing". That was
    filed as "zshrs never raises the LISTMAX query" and as a `ls ` state
    divergence; both were this timer.
    """
    saw = False
    last = time.time()
    end = last + budget
    while time.time() < end:
        r, _, _ = select.select([fd], [], [], 0.05)
        if r:
            try:
                data = os.read(fd, 65536)
            except OSError:
                return saw, True
            if not data:
                return saw, True
            stream.feed(data)
            saw = True
            last = time.time()
        elif time.time() - last >= quiet:
            return saw, True
    return saw, False

def wait_prompt(timeout=30.0, need=1):
    """Pump until READY% has appeared `need` times on screen, or timeout."""
    end = time.time() + timeout
    while time.time() < end:
        pump(0.2)
        seen = sum(l.count("READY%") for l in screen.display)
        if seen >= need:
            return True
    return False

wait_prompt(30.0, need=1)
os.write(fd, f"source {a.init}\r".encode())
wait_prompt(40.0, need=2)
os.write(fd, b"\x0c")          # ctrl-L: clear, so only the probe line remains
pump(0.5)
os.write(fd, a.buffer.encode())
pump(0.6)
truncated = []
for k in a.keys.split(","):
    os.write(fd, keyseq(k).encode())
    _saw, _quiet = pump_until_quiet(a.settle, a.budget)
    if not _quiet:
        truncated.append(k)
_saw, _quiet = pump_until_quiet(a.settle, a.budget)
if not _quiet:
    truncated.append("<tail>")

for line in screen.display:
    if line.strip():
        print(line.rstrip())
if truncated:
    print(
        "comptab_screen: WARNING - still receiving output when the budget ran "
        "out after key(s) %s. The screen above is MID-RENDER, not settled; a "
        "missing line may just be a slow shell. Raise --budget."
        % ", ".join(truncated),
        file=sys.stderr,
    )
os.close(fd)

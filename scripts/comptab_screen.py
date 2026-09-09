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

ap = argparse.ArgumentParser()
ap.add_argument("--shell", required=True)          # full argv, space separated
ap.add_argument("--init", required=True)           # file to source
ap.add_argument("--buffer", required=True)
ap.add_argument("--keys", default="tab")
ap.add_argument("--rows", type=int, default=40)
ap.add_argument("--cols", type=int, default=110)
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


argv = a.shell.split()
pid, fd = pty.fork()
if pid == 0:
    os.environ["PS1"] = "READY%% "
    os.environ["TERM"] = "xterm-256color"
    os.environ["LANG"] = "en_US.UTF-8"
    os.environ["ZDOTDIR"] = "/nonexistent-zdotdir"
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
for k in a.keys.split(","):
    os.write(fd, keyseq(k).encode())
    pump(1.2)
pump(0.8)

for line in screen.display:
    if line.strip():
        print(line.rstrip())
os.close(fd)

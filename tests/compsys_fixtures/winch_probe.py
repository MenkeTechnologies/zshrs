#!/usr/bin/env python3
"""Replay a RESIZE fixture: complete, then change the window size mid-session.

The three sibling harnesses each set the pty geometry once, before the shell
boots, and never touch it again — so none of them can reach a defect that only
appears when the window changes size WHILE a completion listing is on screen.
This replayer exists for that one axis and does nothing else.

What one cell does, in order:

    boot the shell on a pty at rows x cols  ->  type the buffer  ->  send the
    completion keys  ->  settle  ->  count the non-blank rows  ->  set the new
    geometry (TIOCSWINSZ, then SIGWINCH explicitly, because a shell that only
    polls the size on a signal and one that re-reads it on the next redraw must
    both be given the same chance)  ->  settle  ->  count again.

What is compared is the SHAPE of that move on each shell: how many non-blank
rows survived the resize on zsh against how many survived on zshrs, plus the
surviving text. Not the exact rendering — the two shells legitimately re-lay a
listing out differently once the width changes, and this cell is not the place
to relitigate the column math. A cell PASSES when both shells keep the same
number of non-blank rows; it FAILS when one of them loses rows the other keeps.

The controls that make this a finding rather than a screenshot are in the
fixture: the same completion with the window GROWN, and with the window set to
the size it already had.

Result document is the shape the pty harnesses emit — ``{"results": [ {...} ]}``
with a ``status`` of ``PASS``/``FAIL`` — so ``compsys_regressions.py`` scores it
through the same code path as everything else.
"""
from __future__ import annotations

import argparse
import fcntl
import json
import os
import pty
import select
import signal
import struct
import sys
import termios
import time

SELF = os.path.basename(__file__)
REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

KEYS = {
    "tab": b"\t",
    "btab": b"\x1b[Z",
    "ctrl-d": b"\x04",
    "ctrl-n": b"\x0e",
    "ctrl-p": b"\x10",
    "ctrl-g": b"\x07",
    "space": b" ",
}


class Session:
    """One shell on one pty, with a screen we can measure."""

    def __init__(self, argv, rows, cols):
        import pyte
        self.rows, self.cols = rows, cols
        self.screen = pyte.Screen(cols, rows)
        self.stream = pyte.ByteStream(self.screen)
        self.pid, self.fd = pty.fork()
        if self.pid == 0:
            env = dict(os.environ)
            env["TERM"] = "xterm-256color"
            env["PS1"] = "PP "
            # The ambient session exports a 50-entry FPATH; inheriting it would
            # make `-f` mean something different here than it does anywhere
            # else in this directory.
            env.pop("FPATH", None)
            env.pop("ZDOTDIR", None)
            try:
                os.execvpe(argv[0], argv, env)
            except BaseException as exc:  # pragma: no cover — child only
                os.write(2, ("exec failed: %s\n" % exc).encode())
            os._exit(127)
        self._winsize(rows, cols)

    def _winsize(self, rows, cols):
        fcntl.ioctl(self.fd, termios.TIOCSWINSZ,
                    struct.pack("HHHH", rows, cols, 0, 0))

    def settle(self, quiet=0.6, cap=10.0):
        start = last = time.monotonic()
        while time.monotonic() - start < cap:
            r, _, _ = select.select([self.fd], [], [], 0.05)
            if r:
                try:
                    data = os.read(self.fd, 65536)
                except OSError:
                    return
                if not data:
                    return
                self.stream.feed(data)
                last = time.monotonic()
            elif time.monotonic() - last > quiet:
                return

    def send(self, data):
        os.write(self.fd, data)

    def resize(self, rows, cols):
        self.rows, self.cols = rows, cols
        self.screen.resize(rows, cols)
        self._winsize(rows, cols)
        # Both paths, deliberately: a shell that only reacts to the signal and
        # one that re-reads the size on its next redraw must get the same
        # chance, or the cell measures the harness rather than the shell.
        try:
            os.kill(self.pid, signal.SIGWINCH)
        except OSError:
            pass

    def rows_nonblank(self):
        return [l.rstrip() for l in self.screen.display if l.strip()]

    def close(self):
        """Tear the pty down without ever blocking.

        Deliberately not graceful: an earlier version sent ^C^D and then
        `waitpid(pid, 0)`, and a zshrs that is mid-listing does not always
        reach a state where it reads them — the wait then blocked forever and
        the whole cell timed out with the measurement already taken. Closing
        the master fd first delivers the hangup; SIGKILL removes any doubt;
        every wait is WNOHANG.
        """
        try:
            os.close(self.fd)
        except OSError:
            pass
        try:
            os.kill(self.pid, signal.SIGKILL)
        except OSError:
            pass
        deadline = time.monotonic() + 2.0
        while time.monotonic() < deadline:
            try:
                pid, _ = os.waitpid(self.pid, os.WNOHANG)
            except OSError:
                return
            if pid:
                return
            time.sleep(0.05)


def one_shell(argv, run):
    rows, cols = run.get("rows", 24), run.get("cols", 80)
    s = Session(argv, rows, cols)
    try:
        s.settle()
        s.send(run["buffer"].encode())
        s.settle(quiet=0.4)
        for name in run.get("keys", ["tab"]):
            if name not in KEYS:
                raise SystemExit("%s: unknown key %r" % (SELF, name))
            s.send(KEYS[name])
            s.settle()
        before = s.rows_nonblank()
        s.resize(run["new_rows"], run.get("new_cols", cols))
        s.settle()
        after = s.rows_nonblank()
    finally:
        s.close()
    return {"before": before, "after": after,
            "n_before": len(before), "n_after": len(after)}


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--run", required=True, help="JSON document: the fixture's `run` block")
    ap.add_argument("--zshrs", default=os.path.join(REPO, "target", "debug", "zshrs"))
    ap.add_argument("--zsh", default="zsh")
    ap.add_argument("--json", default=None, metavar="PATH")
    args = ap.parse_args()

    with open(args.run) as f:
        run = json.load(f)
    if not os.path.exists(args.zshrs):
        print("# %s: zshrs binary not found: %s" % (SELF, args.zshrs), file=sys.stderr)
        return 2

    zsh_argv = [args.zsh] + list(run.get("zsh_flags") or ["-f"])
    zshrs_argv = [args.zshrs] + list(run.get("zshrs_flags") or ["--zsh", "-f"])
    ref = one_shell(zsh_argv, run)
    sut = one_shell(zshrs_argv, run)

    rows = []
    if ref["n_after"] != sut["n_after"]:
        rows.append({"row": -1, "field": "non-blank rows after resize",
                     "zsh": str(ref["n_after"]), "zshrs": str(sut["n_after"])})
    for i in range(max(len(ref["after"]), len(sut["after"]))):
        lv = ref["after"][i] if i < len(ref["after"]) else "<absent>"
        rv = sut["after"][i] if i < len(sut["after"]) else "<absent>"
        if lv != rv:
            rows.append({"row": i, "field": "after", "zsh": lv, "zshrs": rv})

    status = "FAIL" if ref["n_after"] != sut["n_after"] else "PASS"
    detail = ("zsh kept %d non-blank row(s), zshrs kept %d"
              % (ref["n_after"], sut["n_after"]))
    result = {"case": run.get("case", "winch-probe"),
              "status": status, "detail": detail, "rows": rows,
              "geometry": "%dx%d -> %dx%d" % (run.get("rows", 24), run.get("cols", 80),
                                              run["new_rows"], run.get("new_cols", run.get("cols", 80))),
              "zsh": ref, "zshrs": sut,
              "zsh_argv": zsh_argv, "zshrs_argv": zshrs_argv}
    document = {"harness": SELF, "results": [result]}
    if args.json:
        with open(args.json, "w") as f:
            json.dump(document, f, indent=1)
    print("%-8s %s  (%s)" % (status, detail, result["geometry"]))
    for row in rows[:12]:
        print("  %-28s %3s  zsh=%r  zshrs=%r"
              % (row["field"], row["row"], row["zsh"], row["zshrs"]))
    return 0 if status == "PASS" else 1


if __name__ == "__main__":
    sys.exit(main())

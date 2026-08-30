#!/usr/bin/env python3
"""Replay a PLAIN-SHELL fixture: one script, two shells, no pty.

Sibling of ``zsh_reference_probe.py``. That one boots only zsh, to make a claim
about the reference shell; this one boots both and compares what a
non-interactive script prints.

Why a fixture kind that is not a pty cell
-----------------------------------------

Some completion divergences are not completion bugs. ``argv+=( ... )`` losing
the positional parameters is a parameter bug in the shell core that happens to
reprice every description compsys renders, because
``Completion/Base/Core/_description:83`` builds its ``zformat`` spec list with
exactly that append. Pinning it through a pty harness would assert a screen,
and a screen carries the prompt, the terminal geometry, the completion
listing's layout and the rest of the completion system with it — a dozen ways
for the cell to change shape for reasons that have nothing to do with the bug
being pinned. Two lines of ``print -l`` in ``-f -c`` isolate the same defect
with no terminal in the picture, run in milliseconds, and cannot be moved by a
layout change.

So: a finding whose reproducer is a script, not a keystroke, is pinned here.

What is compared
----------------

STDOUT and the exit status, always. STDERR only when the fixture asks
(``compare_stderr``), because the two shells prefix diagnostics differently
(``probe.zsh:4:`` against ``zsh:1:``) — a real divergence, but not the one any
of these fixtures is about, and one that would make every stderr-carrying cell
fail for the same uninteresting reason. A fixture that IS about a diagnostic
sets the flag and pins the text.

Result document
---------------

The same shape the pty harnesses emit — ``{"results": [ {...} ]}`` with a
``status`` of ``PASS``/``FAIL`` — so ``compsys_regressions.py`` scores it
through the identical code path and no new verdict logic exists to get wrong.
"""
from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile

SELF = os.path.basename(__file__)
REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
DIR_TOKEN = "@DIR@"


def materialise(run, directory):
    """Write the fixture's files and its script into a scratch directory."""
    for spec in run.get("files") or []:
        path = os.path.join(directory, spec["path"])
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w") as f:
            f.write(spec.get("content", ""))
        if spec.get("mode"):
            os.chmod(path, int(spec["mode"], 8))
    for spec in run.get("dirs") or []:
        path = os.path.join(directory, spec["path"])
        os.makedirs(path, exist_ok=True)
        if spec.get("mode"):
            os.chmod(path, int(spec["mode"], 8))
    script = os.path.join(directory, "probe.zsh")
    with open(script, "w") as f:
        f.write("\n".join(run["script"]) + "\n")
    return script


def normalise(text, directory):
    """The scratch path is different every run; nothing else is rewritten."""
    return text.replace(directory, DIR_TOKEN)


def run_shell(argv, directory, env_extra, timeout):
    env = dict(os.environ)
    # -f is passed by the caller; these two make the two shells' *inherited*
    # state the same, which the ambient FPATH/ZDOTDIR of an interactive session
    # otherwise breaks (a `zsh -f` here still inherits a 50-entry FPATH).
    env.pop("FPATH", None)
    env.pop("ZDOTDIR", None)
    env.update(env_extra or {})
    try:
        proc = subprocess.run(argv, cwd=directory, env=env, timeout=timeout,
                              capture_output=True, text=True,
                              stdin=subprocess.DEVNULL)
    except subprocess.TimeoutExpired:
        return {"timeout": True, "stdout": "", "stderr": "", "exit": None}
    return {"timeout": False,
            "stdout": normalise(proc.stdout, directory),
            "stderr": normalise(proc.stderr, directory),
            "exit": proc.returncode}


def compare(a, b, compare_stderr):
    """The differing lines, side by side, in the pty harnesses' `rows` shape."""
    fields = ["stdout"] + (["stderr"] if compare_stderr else [])
    rows = []
    for field in fields:
        left, right = a[field].splitlines(), b[field].splitlines()
        for i in range(max(len(left), len(right))):
            lv = left[i] if i < len(left) else "<absent>"
            rv = right[i] if i < len(right) else "<absent>"
            if lv != rv:
                rows.append({"row": i, "field": field, "zsh": lv, "zshrs": rv})
    if a["exit"] != b["exit"]:
        rows.append({"row": -1, "field": "exit",
                     "zsh": str(a["exit"]), "zshrs": str(b["exit"])})
    return rows


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--run", required=True,
                    help="JSON document: the fixture's `run` block")
    ap.add_argument("--zshrs", default=os.path.join(REPO, "target", "debug", "zshrs"))
    ap.add_argument("--zsh", default="zsh")
    ap.add_argument("--timeout", type=float, default=30.0)
    ap.add_argument("--keep", action="store_true")
    ap.add_argument("--json", default=None, metavar="PATH")
    args = ap.parse_args()

    with open(args.run) as f:
        run = json.load(f)

    if not os.path.exists(args.zshrs):
        print("# %s: zshrs binary not found: %s" % (SELF, args.zshrs),
              file=sys.stderr)
        return 2

    directory = tempfile.mkdtemp(prefix="shell_probe_")
    try:
        script = materialise(run, directory)
        extra = list(run.get("argv") or [])
        zsh_argv = [args.zsh, "-f", script] + extra
        zshrs_argv = ([args.zshrs] + list(run.get("zshrs_flags") or ["--zsh"])
                      + ["-f", script] + extra)
        ref = run_shell(zsh_argv, directory, run.get("env"), args.timeout)
        sut = run_shell(zshrs_argv, directory, run.get("env"), args.timeout)
    finally:
        if args.keep:
            print("# %s: artifacts %s" % (SELF, directory), file=sys.stderr)
        else:
            shutil.rmtree(directory, ignore_errors=True)

    compare_stderr = bool(run.get("compare_stderr"))
    if ref["timeout"] or sut["timeout"]:
        status, rows = "TIMEOUT", []
        detail = "%s did not finish in %ss" % (
            "zsh" if ref["timeout"] else "zshrs", args.timeout)
    else:
        rows = compare(ref, sut, compare_stderr)
        status = "FAIL" if rows else "PASS"
        detail = ("%d line(s) differ" % len(rows)) if rows \
            else "byte-identical stdout%s and exit status" % (
                " and stderr" if compare_stderr else "")

    result = {"case": run.get("case", "shell-probe"),
              "status": status,
              "detail": detail,
              "rows": rows,
              "compare_stderr": compare_stderr,
              "zsh": ref, "zshrs": sut,
              "zsh_argv": zsh_argv, "zshrs_argv": zshrs_argv}
    document = {"harness": SELF, "results": [result]}
    if args.json:
        with open(args.json, "w") as f:
            json.dump(document, f, indent=1)
    print("%-8s %s" % (status, detail))
    for row in rows[:20]:
        print("  %-6s %3s  zsh=%r  zshrs=%r"
              % (row["field"], row["row"], row["zsh"], row["zshrs"]))
    return 0 if status == "PASS" else 1


if __name__ == "__main__":
    sys.exit(main())

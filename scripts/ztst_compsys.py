#!/usr/bin/env python3
"""Run zsh's own completion test suite (Test/Y0*.ztst) against an arbitrary shell.

Why this exists
---------------
Every other compsys parity harness in this repo (``compsys_parity.py``,
``comptab_parity.py``, ``compsys_spec_fuzz.py``) compares zshrs against zsh on
cases *we* invented.  zsh ships its own completion tests together with its own
expected output, and its driver (``Test/comptest``) was built to drive an
arbitrary shell binary::

    comptestinit -z /path/to/shell

So upstream's tests can be pointed at zshrs and upstream's expected output
becomes the oracle.  A failure here is a compatibility gap stated in upstream's
own terms rather than ours.

How the plumbing works
----------------------
``Test/ztst.zsh`` is the .ztst interpreter.  It is executed *by a zsh binary*
(the "harness"), and the harness is what needs ``zsh/zpty`` -- the shell under
test never loads zpty itself.  ``ztst.zsh`` sets ``ZTST_testdir=$PWD`` and
``ZTST_srcdir=${0%/*}``.  Every Y0*.ztst ``%prep`` section then calls::

    comptestinit -z $ZTST_testdir/../Src/zsh

i.e. the shell under test is always ``<cwd>/../Src/zsh``.  That is the hook this
runner uses: it builds a throwaway run directory

    <run>/Test/            <- cwd, so ZTST_testdir points here
    <run>/Test/Modules/zsh <- so the harness can zmodload zsh/zpty
    <run>/Src/zsh          <- symlink to the shell under test

and runs ``<harness> +Z -f <zsh-build>/Test/ztst.zsh <zsh-build>/Test/Y0X.ztst``
from ``<run>/Test``.  No .ztst file and no driver file is ever modified.

Required setup
--------------
A *built* zsh source tree is needed: its ``Src/zsh`` is the baseline shell, its
``Completion/`` supplies the compsys functions fed to whichever shell is under
test, and its ``Test/`` supplies the .ztst files and ``comptest``.  All three
must come from the same tree, because the Y tests are version-locked to the
compsys functions next to them (a 5.9.999-era ``Completion/compinit`` uses
``${ ... }`` nofork command substitution, which zsh 5.9.2 rejects with
"bad substitution", which kills ``compdef`` and hangs the whole suite).

``--zsh-build`` points at that tree.  With no flag the gitignored ``src/zsh``
inside this repo is used, which needs no setup but only carries Y01-Y03; the
reference checkout carries all six.  To build that one::

    cp -a ~/forkedRepos/zsh /tmp/zshsrc && cd /tmp/zshsrc
    ./Util/preconfig && ./configure && make

Adaptations (declared, because they change what is being measured)
------------------------------------------------------------------
* ``--fx {off,on}``: zshrs enables its own native ZLE effects (autosuggest,
  syntax highlight) by default, even under ``-f``.  Those paint colour into the
  pty and inject phantom suggestion text, which corrupts the capture that
  ``comptest`` parses.  ``--fx off`` (the default) exports
  ``ZSHRS_NATIVE_ZLE_FX=0``.  Runs made with ``--fx on`` measure the shell as
  shipped and are reported separately; they are not comparable to the zsh
  baseline.
* The shell under test is reached through a *symlink*, never a shell wrapper
  script.  ``/bin/sh`` drops the exported ``PS1``, and ``comptest`` keys every
  single read on ``<PROMPT>``; a wrapper therefore hangs the suite outright.
  Extra environment for the shell under test is passed via ``--sut-env``, which
  is exported into the harness and inherited down through zpty.

Nothing in this runner ever edits, filters, or relaxes an upstream assertion.

What it can do
--------------
Run the suite (default)::

    ztst_compsys.py --sut <shell> --zsh-build <tree> [--tests Y01completion,...]
    ztst_compsys.py --baseline --zsh-build <tree>

Turn a failing assertion into a minimal standalone repro (``--minimize``)::

    ztst_compsys.py --sut <shell> --zsh-build <tree> --minimize Y03arguments#4

  Reduction is differential, not oracle-based: it shrinks the setup, the
  preceding assertions and the keystroke string while the two shells keep
  disagreeing *about the same kinds of comptest output line* (see
  ``divergence_signature``).  What comes out is a standalone ``.zsh`` that
  drives ``comptestinit``/``comptest`` directly -- no ztst.zsh, no expected
  output, just "run this against each shell and diff".  The reduced script is a
  derived artifact; the upstream .ztst is still the oracle and is never touched.
  Reduction is budgeted, and the report says "converged" or
  "budget exhausted" -- never silently the former when it was the latter.

Gate a run against a pinned state (``--pin`` / ``--gate``)::

    ztst_compsys.py --sut <shell> --zsh-build <tree> --pin
    ztst_compsys.py --sut <shell> --zsh-build <tree> --gate

  The pin records every assertion's status plus the identity of the binary that
  produced it (size, mtime, --version, sha256 prefix).  A gate run reports
  REGRESSED, FIXED, CHANGED, NEW and MISSING separately, so a fixture that has
  quietly been fixed shows up as loudly as a fresh break.  A FIXED verdict is
  reported together with whether the binary moved since the pin: "fixed" with an
  unchanged binary means the run is flaky, not that anything was fixed.

  Exit codes: 0 unchanged, 1 something regressed, 2 something moved without
  regressing, 3 the runner itself failed.

Run the wider suite (``--core``)::

    ztst_compsys.py --sut <shell> --zsh-build <tree> --core

  Every non-Y .ztst -- A* grammar, B* builtins, C* arith/traps, D* expansion and
  globbing, E* options, K*, V* modules, W* history/jobs, X* zle, Z*.  These have
  no ``comptestinit``: their assertions run *in* the ztst harness, so the shell
  under test becomes its own harness.  Compsys is written in this language, so
  these results explain compsys divergences -- but they are shell-parity
  numbers, not compsys numbers, and must be reported as their own section.
"""

from __future__ import annotations

import argparse
import concurrent.futures as cf
import hashlib
import itertools
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass, field, asdict
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent

# Exit codes.  They have to stay distinct: the gate's caller must be able to
# tell "nothing moved" from "something moved" from "the runner broke".
EXIT_UNCHANGED = 0       # gate: every pinned assertion matched
EXIT_REGRESSED = 1       # gate: at least one pinned pass now fails
EXIT_MOVED = 2           # gate: movement, but nothing regressed
EXIT_RUNNER_FAILED = 3   # setup/usage/crash -- not a statement about the shell


def die(msg: str) -> "typing.NoReturn":  # noqa: F821 - annotation only
    print(msg, file=sys.stderr)
    sys.exit(EXIT_RUNNER_FAILED)


class _Parser(argparse.ArgumentParser):
    """Usage errors exit with EXIT_RUNNER_FAILED, not argparse's 2."""

    def error(self, message: str) -> "typing.NoReturn":  # noqa: F821
        self.print_usage(sys.stderr)
        die(f"{self.prog}: error: {message}")


# The six upstream completion test files, in upstream's own order.
ALL_TESTS = [
    "Y01completion",
    "Y02compmatch",
    "Y03arguments",
    "Y04regexargs",
    "Y05describe",
    "Y06values",
]

# ztst.zsh, with ZTST_verbose=1, brackets every assertion with these markers.
RE_RUNNING = re.compile(r"^Running test: (.*)$")
RE_SUCCESS = re.compile(r"^Test successful\.$")
RE_XFAIL = re.compile(r"^Test failed, as expected\.$")
RE_FAILED = re.compile(r"^Test (\S+) failed: (.*)$")
RE_XPASSED = re.compile(r"^Test (\S+) was expected to fail, but passed\.$")
RE_WASTESTING = re.compile(r"^Was testing: (.*)$")
RE_SKIPPED = re.compile(r"^Test case skipped: (.*)$")
RE_STARTING = re.compile(r"^(\S+): starting\.$")
RE_ALLOK = re.compile(r"^(\S+): all tests successful\.$")
RE_FILESKIP = re.compile(r"^(\S+): skipped \((.*)\)$")

# A .ztst assertion header: "0:message", "0f:message", "-:message", "1D:msg".
RE_ZTST_STATUS = re.compile(r"^([-0-9]+)([A-Za-z]*)(?::(.*))?$")


@dataclass
class Assertion:
    """One upstream assertion (one ``NN[flags]:message`` block)."""

    index: int
    message: str
    status: str = "unknown"  # pass | fail | xfail | xpass | skip | notrun
    reason: str = ""
    expected: list[str] = field(default_factory=list)
    actual: list[str] = field(default_factory=list)
    diff: list[str] = field(default_factory=list)


@dataclass
class FileResult:
    """Result of running one Y0*.ztst file."""

    name: str
    exit_code: int = -1
    timed_out: bool = False
    unimplemented: str = ""
    raw: str = ""
    assertions: list[Assertion] = field(default_factory=list)

    @property
    def counts(self) -> dict[str, int]:
        out: dict[str, int] = {}
        for a in self.assertions:
            out[a.status] = out.get(a.status, 0) + 1
        return out


def find_zsh_build(explicit: str | None) -> Path:
    """Locate a *built* zsh source tree (Src/zsh + Completion/ + Test/*.ztst)."""
    candidates: list[Path] = []
    if explicit:
        candidates.append(Path(explicit).expanduser())
    if os.environ.get("ZTST_ZSH_BUILD"):
        candidates.append(Path(os.environ["ZTST_ZSH_BUILD"]).expanduser())
    candidates.append(REPO / "src" / "zsh")
    for cand in candidates:
        if (cand / "Src" / "zsh").is_file() and (cand / "Test" / "comptest").is_file():
            return cand.resolve()
    tried = "\n  ".join(str(c) for c in candidates)
    die(
        "no built zsh source tree found (need Src/zsh + Test/comptest). Tried:\n  "
        + tried
        + "\n\nBuild one with:\n"
        "  cp -a ~/forkedRepos/zsh /tmp/zshsrc && cd /tmp/zshsrc\n"
        "  ./Util/preconfig && ./configure && make\n"
        "then pass --zsh-build /tmp/zshsrc"
    )


def collect_modules(zsh_build: Path, dest: Path) -> int:
    """Populate ``dest/zsh`` with the build tree's modules so zpty can load.

    ``make check`` does this with ``make install.modules``; symlinking the
    already-built ``.so`` files is equivalent for ``dlopen`` and does not touch
    the source tree.  Returns the number of modules linked.
    """
    src = zsh_build / "Src"
    dest.mkdir(parents=True, exist_ok=True)
    (dest / "zsh").mkdir(exist_ok=True)
    count = 0
    for so in sorted(src.rglob("*.so")):
        link = dest / "zsh" / so.name
        if link.exists() or link.is_symlink():
            link.unlink()
        link.symlink_to(so)
        count += 1
    return count


def parse_expected_assertions(ztst: Path) -> list[tuple[int, str, str]]:
    """Read a .ztst file and return every assertion header in the %test section.

    Returns ``(index, flags, message)`` triples in file order.  This is how the
    total assertion count is derived -- it is never typed as a literal.
    """
    out: list[tuple[int, str, str]] = []
    in_test = False
    idx = 0
    for line in ztst.read_text(errors="replace").splitlines():
        if line.startswith("%"):
            in_test = line.startswith("%test")
            continue
        if not in_test:
            continue
        m = RE_ZTST_STATUS.match(line)
        if m:
            idx += 1
            out.append((idx, m.group(2), (m.group(3) or "").strip()))
    return out


def parse_run(text: str, expected: list[tuple[int, str, str]]) -> list[Assertion]:
    """Turn one ztst.zsh run's stdout into per-assertion results.

    With ``ZTST_verbose=1`` ztst prints ``Running test: <msg>`` before each
    assertion and ``Test successful.`` after a passing one, so results can be
    attributed to individual assertions rather than to the file as a whole.
    """
    assertions = [Assertion(index=i, message=msg) for i, _flags, msg in expected]
    by_index = {a.index: a for a in assertions}
    cur: Assertion | None = None
    seen = 0
    pending: list[str] = []  # lines since the last marker: the diff, if any

    lines = text.splitlines()
    i = 0
    while i < len(lines):
        line = lines[i]
        m = RE_RUNNING.match(line)
        if m:
            seen += 1
            cur = by_index.get(seen)
            if cur is None:
                cur = Assertion(index=seen, message=m.group(1))
                assertions.append(cur)
                by_index[seen] = cur
            # ztst's message is authoritative; the .ztst scrape is the fallback.
            if m.group(1):
                cur.message = m.group(1)
            cur.status = "notrun"
            pending = []
            i += 1
            continue
        if RE_SUCCESS.match(line):
            if cur is not None:
                cur.status = "pass"
            pending = []
            i += 1
            continue
        if RE_XFAIL.match(line):
            if cur is not None:
                cur.status = "xfail"
            pending = []
            i += 1
            continue
        m = RE_SKIPPED.match(line)
        if m:
            if cur is not None:
                cur.status = "skip"
                cur.reason = m.group(1)
            pending = []
            i += 1
            continue
        m = RE_XPASSED.match(line)
        if m:
            if cur is not None:
                cur.status = "xpass"
                cur.reason = "expected to fail, but passed"
            pending = []
            i += 1
            continue
        m = RE_FAILED.match(line)
        if m:
            if cur is not None:
                cur.status = "fail"
                cur.reason = m.group(2)
                cur.diff = [ln for ln in pending if ln.strip()]
                cur.expected = [
                    ln[1:] for ln in cur.diff if ln.startswith("-") and not ln.startswith("---")
                ]
                cur.actual = [
                    ln[1:] for ln in cur.diff if ln.startswith("+") and not ln.startswith("+++")
                ]
            pending = []
            i += 1
            continue
        pending.append(line)
        i += 1

    return assertions


def run_one(
    *,
    name: str,
    sut: Path,
    zsh_build: Path,
    harness: Path,
    modules: Path,
    run_root: Path,
    timeout: int,
    sut_env: dict[str, str],
    verbose: bool,
) -> FileResult:
    """Run a single Y0*.ztst file against ``sut`` and parse the result."""
    ztst = zsh_build / "Test" / f"{name}.ztst"
    res = FileResult(name=name)
    if not ztst.is_file():
        res.unimplemented = f"{ztst} not present in this zsh source tree"
        return res

    rundir = run_root / name
    if rundir.exists():
        shutil.rmtree(rundir)
    (rundir / "Test").mkdir(parents=True)
    (rundir / "Src").mkdir()
    (rundir / "Src" / "zsh").symlink_to(sut)
    (rundir / "Test" / "Modules").symlink_to(modules)
    # V01zmodload reads $ZTST_testdir/../config.modules to learn which modules
    # this build has; without it its whole %prep aborts (41 assertions never
    # run).  `make check` has it because it runs inside the build tree.
    cfg = zsh_build / "config.modules"
    if cfg.is_file():
        (rundir / "config.modules").symlink_to(cfg)

    env = {
        "HOME": os.environ.get("HOME", str(Path.home())),
        "PATH": "/usr/bin:/bin:/usr/sbin:/sbin",
        "TERM": "xterm",
        "LANG": "C",
        # Keep ztst's scratch files out of the user's real TMPPREFIX; several
        # instances of this repo run concurrently.
        "TMPPREFIX": str(run_root / "tmp" / "zsh"),
        "ZTST_continue": "1",  # report every assertion, not just the first failure
        "ZTST_verbose": "1",  # emit the per-assertion Running/successful markers
        # Test/Makefile.in:56 passes this to ztst.zsh and several tests
        # (C03traps and friends) run "$ZTST_exe -fc ..." directly.  Unset it
        # and they exit 127, which looks like a shell failure but is ours.
        "ZTST_exe": "../Src/zsh",
    }
    env.update(sut_env)
    (run_root / "tmp").mkdir(exist_ok=True)

    cmd = [str(harness), "+Z", "-f", str(zsh_build / "Test" / "ztst.zsh"), str(ztst)]
    started = time.time()
    # Own session + killpg on timeout: Y03arguments hangs by design here (the
    # shell under test exits and the driver waits forever on zpty), and killing
    # only the direct child leaves the whole pty tree spinning in the
    # background for the rest of the run.
    proc = subprocess.Popen(
        cmd,
        cwd=rundir / "Test",
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        start_new_session=True,
    )
    try:
        out, _ = proc.communicate(timeout=timeout)
        res.raw = out.decode("utf-8", "replace")
        res.exit_code = proc.returncode
    except subprocess.TimeoutExpired:
        try:
            os.killpg(os.getpgid(proc.pid), signal.SIGKILL)
        except OSError:
            proc.kill()
        # zpty's child setsid()s away from our process group, so killpg misses
        # it; it is reachable by the run directory's own Src/zsh path, which is
        # unique to this file's run.
        subprocess.run(
            ["pkill", "-9", "-f", str(rundir / "Src" / "zsh")], capture_output=True
        )
        try:
            out, _ = proc.communicate(timeout=20)
        except subprocess.TimeoutExpired:
            out = b""
        res.timed_out = True
        res.raw = out.decode("utf-8", "replace")
        res.exit_code = -9
    elapsed = time.time() - started

    for line in res.raw.splitlines():
        m = RE_FILESKIP.match(line)
        if m:
            res.unimplemented = m.group(2)

    expected = parse_expected_assertions(ztst)
    res.assertions = parse_run(res.raw, expected)
    if verbose:
        c = res.counts
        print(
            f"  {name}: exit={res.exit_code} {elapsed:5.1f}s "
            + " ".join(f"{k}={v}" for k, v in sorted(c.items())),
            file=sys.stderr,
        )
    return res


def summarise(results: list[FileResult]) -> dict:
    total: dict[str, int] = {}
    for r in results:
        for k, v in r.counts.items():
            total[k] = total.get(k, 0) + v
    return {
        "files": len(results),
        "assertions": sum(len(r.assertions) for r in results),
        "by_status": total,
    }


def render_report(label: str, meta: dict, results: list[FileResult]) -> str:
    out: list[str] = []
    out.append(f"# ztst Y-series completion suite -- {label}")
    out.append("")
    for k in sorted(meta):
        out.append(f"{k}: {meta[k]}")
    out.append("")
    summary = summarise(results)
    out.append(
        f"files={summary['files']} assertions={summary['assertions']} "
        + " ".join(f"{k}={v}" for k, v in sorted(summary["by_status"].items()))
    )
    out.append("")
    for r in results:
        c = r.counts
        head = f"## {r.name}  exit={r.exit_code}"
        if r.timed_out:
            head += " TIMED-OUT"
        if r.unimplemented:
            head += f" SKIPPED({r.unimplemented})"
        head += "  " + " ".join(f"{k}={v}" for k, v in sorted(c.items()))
        out.append(head)
        for a in r.assertions:
            mark = {
                "pass": "ok  ",
                "fail": "FAIL",
                "xfail": "xfail",
                "xpass": "XPASS",
                "skip": "skip",
                "notrun": "----",
                "unknown": "????",
            }.get(a.status, a.status)
            out.append(f"  {mark} {a.index:3d} {a.message}")
        out.append("")
    return "\n".join(out)


def render_failures(results: list[FileResult]) -> str:
    out: list[str] = []
    for r in results:
        for a in r.assertions:
            if a.status not in ("fail", "xpass", "notrun", "unknown"):
                continue
            out.append(f"=== {r.name} #{a.index} [{a.status}] {a.message}")
            if a.reason:
                out.append(f"    reason: {a.reason.splitlines()[0] if a.reason else ''}")
            for ln in a.diff:
                out.append(f"    {ln}")
            out.append("")
    return "\n".join(out)


def compare(baseline: list[FileResult], candidate: list[FileResult]) -> str:
    """Per-assertion comparison; only differences against the baseline count."""
    base = {(r.name, a.index): a for r in baseline for a in r.assertions}
    cand = {(r.name, a.index): a for r in candidate for a in r.assertions}
    keys = sorted(set(base) | set(cand))
    rows: list[str] = []
    tally: dict[str, int] = {}
    for k in keys:
        b = base.get(k)
        c = cand.get(k)
        bs = b.status if b else "absent"
        cs = c.status if c else "absent"
        if bs == "pass" and cs == "pass":
            verdict = "both-pass"
        elif bs == "pass" and cs != "pass":
            verdict = "REGRESSION"
        elif bs != "pass" and cs == "pass":
            verdict = "candidate-only-pass"
        else:
            verdict = "both-nonpass"
        tally[verdict] = tally.get(verdict, 0) + 1
        if verdict != "both-pass":
            msg = (c or b).message if (c or b) else ""
            rows.append(f"{verdict:20s} {k[0]} #{k[1]:3d} base={bs:7s} cand={cs:7s} {msg}")
    head = ["# baseline vs candidate, per assertion", ""]
    head += [f"{k}: {v}" for k, v in sorted(tally.items())]
    head += [""]
    return "\n".join(head + rows)



# ---------------------------------------------------------------------------
# Structural .ztst parsing.  Used ONLY to build reduced repro scripts; the
# upstream files themselves are read-only and are never rewritten in place.
# ---------------------------------------------------------------------------

RE_REDIR = re.compile(r"^(?:\*?[<>?]|F:)")


@dataclass
class Chunk:
    """One ``%test`` block: an indented code chunk plus its assertion header."""

    index: int
    code: list[str] = field(default_factory=list)
    status: str = "0"
    flags: str = ""
    message: str = ""
    redirs: list[str] = field(default_factory=list)


def parse_ztst_struct(path: Path) -> tuple[list[str], list[Chunk]]:
    """Split a .ztst into (lines before ``%test``, per-assertion chunks).

    Mirrors ``ZTST_test``/``ZTST_getchunk`` (``Test/ztst.zsh:242-266,437-495``):
    an indented run of lines is a code chunk, a bare ``NN[flags][:msg]`` line
    closes it, and ``<``/``>``/``?``/``*>``/``*?``/``F:`` lines carry redirects.
    """
    pre: list[str] = []
    chunks: list[Chunk] = []
    section = ""
    cur_code: list[str] = []
    cur: Chunk | None = None
    idx = 0
    for line in path.read_text(errors="replace").splitlines():
        if line.startswith("%"):
            section = line.split()[0]
            if section != "%test":
                cur = None
            if section == "%test":
                pre.append(line)
                continue
            if not chunks:
                pre.append(line)
            continue
        if section != "%test":
            if not chunks:
                pre.append(line)
            continue
        if not line.strip():
            cur_code = []
            cur = None
            continue
        m = RE_ZTST_STATUS.match(line)
        if m and cur_code:
            idx += 1
            cur = Chunk(
                index=idx,
                code=cur_code,
                status=m.group(1),
                flags=m.group(2),
                message=(m.group(3) or "").strip(),
            )
            chunks.append(cur)
            cur_code = []
            continue
        if RE_REDIR.match(line):
            if cur is not None:
                cur.redirs.append(line)
            continue
        if line[:1].isspace():
            cur_code.append(line)
            continue
        # anything else (a stray line) is kept as code so nothing is dropped
        cur_code.append(line)
    return pre, chunks


def _opens_block(line: str) -> bool:
    return line.strip().endswith("{")


def _closes_block(line: str) -> bool:
    return line.strip().startswith("}")


def group_statements(lines: list[str]) -> list[list[str]]:
    """Group lines into brace-balanced statements, so reduction cannot split one.

    Y01completion #16's code is a single ``{ ... } always { ... }`` block;
    deleting a line out of the middle of it leaves an unbalanced ``}`` behind.
    """
    out: list[list[str]] = []
    cur: list[str] = []
    depth = 0
    for ln in lines:
        cur.append(ln)
        if _opens_block(ln):
            depth += 1
        elif _closes_block(ln):
            depth -= 1
        if ln.rstrip().endswith("\\"):
            continue
        if depth <= 0:
            out.append(cur)
            cur = []
            depth = 0
    if cur:
        out.append(cur)
    return out


def divergence_signature(a: list[str], b: list[str]) -> frozenset[str]:
    """Which *kinds* of comptest line the two shells disagree about.

    ``comptest`` emits ``line: {..}{..}``, ``DESCRIPTION:{..}``, ``NO:{..}``,
    ``INSERT_POSITIONS:{..}`` and friends.  Reduction has to stay on the
    divergence it started from -- without this anchor a reducer happily deletes
    the setup that produces the interesting bug and reports whatever unrelated
    divergence is left behind (measured: Y01completion #16 slid off the ``..``
    path bug onto the unrelated ``h:`` description bug).
    """
    sa, sb = set(a), set(b)
    kinds: set[str] = set()
    for ln in (sa ^ sb):
        head = ln.split(":{", 1)[0] if ":{" in ln else ln.split(":", 1)[0]
        kinds.add(head.strip())
    return frozenset(kinds)


def split_prep(pre: list[str]) -> tuple[list[str], list[list[str]]]:
    """Pull the reducible setup out of a ``%prep`` section.

    Returns ``(pre_extras, extras)``:

    * ``pre_extras`` -- assignments that run *before* ``comptestinit`` (only
      ``ZSH_TEST_LANG=$(ZTST_find_UTF8)`` in the current Y files, and it
      matters because ``comptestinit`` exports it as ``LC_ALL``).
    * ``extras`` -- the statements inside the ``comptestinit ... && { ... }``
      block, each as a list of lines.

    Brace depth is tracked with the conservative rule "a line that ends with
    ``{`` opens, a line that starts with ``}`` closes".  A naive character
    count is wrong here: ``Y02compmatch``'s prep contains ``code="$code}"``.
    """
    pre_extras: list[str] = []
    extras: list[list[str]] = []
    init = next((i for i, ln in enumerate(pre) if "comptestinit" in ln), None)
    if init is None:
        return pre_extras, extras

    for ln in pre[:init]:
        s = ln.strip()
        # ZTST_* are the driver's own controls (ZTST_unimplemented and friends);
        # they mean nothing outside ztst.zsh, so they are not repro material.
        if re.match(r"^[A-Za-z_][A-Za-z_0-9]*=", s) and not s.startswith("ZTST_"):
            pre_extras.append(s)

    # find the block opener: either the comptestinit line itself or a later "{"
    start = None
    for i in range(init, len(pre)):
        if _opens_block(pre[i]):
            start = i + 1
            break
        if pre[i].strip() and not pre[i].strip().endswith("&&") and i > init:
            break
    if start is None:
        return pre_extras, extras

    depth = 0
    stmt: list[str] = []
    for ln in pre[start:]:
        if depth == 0 and _closes_block(ln) and not stmt:
            break
        stmt.append(ln)
        if _opens_block(ln):
            depth += 1
        elif _closes_block(ln):
            depth -= 1
        if ln.rstrip().endswith("\\"):
            continue  # backslash continuation: the statement is not over yet
        if depth <= 0:
            text = "\n".join(stmt).rstrip()
            if text.endswith("&&"):
                stmt[-1] = stmt[-1].rstrip()[:-2].rstrip()
            if any(s.strip() for s in stmt):
                extras.append(list(stmt))
            stmt = []
            depth = 0
    return pre_extras, extras


def extract_zsh_function(src: Path, name: str) -> str:
    """Lift one function verbatim out of a zsh script (read-only use)."""
    text = src.read_text(errors="replace")
    m = re.search(rf"^{re.escape(name)} *\(\) *\{{$", text, re.M)
    if not m:
        return ""
    lines = text[m.start():].splitlines()
    out = [lines[0]]
    for ln in lines[1:]:
        out.append(ln)
        if ln.rstrip() == "}":
            break
    return "\n".join(out)


# ---------------------------------------------------------------------------
# Shell-word splitting (quote aware) and $'...' tokenising, for reduction.
# ---------------------------------------------------------------------------


def split_words(line: str) -> list[str]:
    """Split a shell line into words, keeping quotes attached to their word."""
    words: list[str] = []
    cur = ""
    i = 0
    quote = ""
    n = len(line)
    while i < n:
        c = line[i]
        if quote:
            cur += c
            if c == "\\" and quote != "'" and i + 1 < n:
                cur += line[i + 1]
                i += 2
                continue
            if c == quote:
                quote = ""
            i += 1
            continue
        if c in "'\"":
            quote = c
            cur += c
            i += 1
            continue
        if c == "$" and i + 1 < n and line[i + 1] == "'":
            quote = "'"
            cur += "$'"
            i += 2
            continue
        if c == "\\" and i + 1 < n:
            cur += line[i : i + 2]
            i += 2
            continue
        if c.isspace():
            if cur:
                words.append(cur)
                cur = ""
            i += 1
            continue
        cur += c
        i += 1
    if cur:
        words.append(cur)
    return words


RE_DOLLAR_TOKEN = re.compile(r"\\C-.|\\M-.|\\x[0-9A-Fa-f]{1,2}|\\[0-7]{1,3}|\\.|.", re.S)


def tokenise_dollar_quote(body: str) -> list[str]:
    """Split the body of a ``$'...'`` literal into single keystrokes."""
    return RE_DOLLAR_TOKEN.findall(body)


# ---------------------------------------------------------------------------
# Standalone repro driver: comptest without ztst.zsh.
# ---------------------------------------------------------------------------

DRIVER_TEMPLATE = """\
# {banner}
#
# Reduced from upstream {origin}
#   assertion #{index}: {message}
#
# Run it against each shell and diff the two outputs:
#
{cmds}
#
# Everything below is derived from upstream's own {origin_base} and
# Test/comptest.  Neither upstream file is modified; this script only replays
# the reduced setup through the unmodified comptestinit/comptest driver.

emulate -R zsh
setopt extendedglob
# zsh/zpty is loaded by comptestinit, after it has pointed module_path at the
# build tree; loading it here would look in the install prefix and fail.

sut=${{1:?usage: <harness-zsh> -f $0 <shell-under-test> [<built-zsh-tree>]}}
build=${{2:-{build}}}
[[ -x $build/Src/zsh && -r $build/Test/comptest ]] || {{
  print -u2 "$0: not a built zsh source tree: $build"
  exit 2
}}

ZTST_srcdir=$build/Test
ZTST_verbose=0
ZTST_fd=2

{utf8}
{pre_extras}
work=$(mktemp -d "${{TMPDIR:-/tmp}}/ztst_repro.XXXXXX")
mkdir -p $work/Modules/zsh
for so in $build/Src/**/*.so(N); do ln -sf $so $work/Modules/zsh; done
ZTST_testdir=$work
mkdir $work/comp.tmp
cd $work/comp.tmp
. $ZTST_srcdir/comptest

comptestinit -z $sut || exit 1
{extras}
{context}
{marker}
{target}
zpty -d zsh 2>/dev/null
cd /
rm -rf $work
"""


@dataclass
class Repro:
    """One reduction attempt: what was kept, what it cost, what it shows."""

    origin: str
    index: int
    message: str
    pre_extras: list[str]
    extras: list[list[str]]
    context: list[list[str]]
    target: list[str]
    probes: int = 0
    invalid_probes: int = 0
    converged: bool = False
    baseline_diverged: bool = False
    note: str = ""
    signature: list[str] = field(default_factory=list)
    out_a: list[str] = field(default_factory=list)
    out_b: list[str] = field(default_factory=list)
    path: str = ""


# Printed between the context and the target while reducing, so the property
# being preserved is "the two shells disagree about *this assertion's* output"
# rather than "... about anything the whole script printed".  Without it the
# earlier assertions' output is part of the comparison and no context chunk can
# ever be dropped.  The emitted repro is generated without it.
TARGET_MARKER = "<<<ZTST_MIN_TARGET>>>"


def build_driver(
    rep: Repro,
    *,
    build: Path,
    utf8_fn: str,
    cmds: list[str],
    banner: str,
    marker: bool = False,
) -> str:
    def block(items) -> str:
        out: list[str] = []
        for it in items:
            if isinstance(it, str):
                out.append(it)
            else:
                out.extend(it)
        return "\n".join(ln.rstrip() for ln in out)

    need_utf8 = any("ZTST_find_UTF8" in ln for ln in rep.pre_extras)
    return DRIVER_TEMPLATE.format(
        banner=banner,
        origin=rep.origin,
        origin_base=rep.origin.split("#")[0],
        index=rep.index,
        message=rep.message or "(no message)",
        cmds="\n".join(f"#   {c}" for c in cmds),
        build=build,
        utf8=(utf8_fn if need_utf8 else ""),
        pre_extras=block(rep.pre_extras),
        extras=block(rep.extras),
        context=block(rep.context),
        marker=f"print -r -- {TARGET_MARKER!r}" if marker else "",
        target=block(rep.target),
    )


class Minimizer:
    """Reduce one failing assertion to the smallest still-diverging script."""

    def __init__(
        self,
        *,
        zsh_build: Path,
        harness: Path,
        shell_a: Path,
        shell_b: Path,
        sut_env: dict[str, str],
        budget: int,
        timeout: int,
        workdir: Path,
        verbose: bool = True,
    ) -> None:
        self.zsh_build = zsh_build
        self.harness = harness
        self.shell_a = shell_a
        self.shell_b = shell_b
        self.sut_env = sut_env
        self.budget = budget
        self.spent = 0
        self.timeout = timeout
        self.workdir = workdir
        self.workdir.mkdir(parents=True, exist_ok=True)
        self.verbose = verbose
        self.cache: dict[tuple[str, str], list[str]] = {}
        self.last_err: dict[str, str] = {}
        self.signature: frozenset[str] | None = None
        self.probe_seq = itertools.count(1)  # threads take probe ids from here
        self.invalid_probes = 0  # probes where the reference itself timed out
        self.utf8_fn = extract_zsh_function(zsh_build / "Test" / "ztst.zsh", "ZTST_find_UTF8")

    # -- probing ----------------------------------------------------------
    def _run(self, shell: Path, script: str, env_extra: dict[str, str]) -> list[str]:
        key = (str(shell), script)
        if key in self.cache:
            return self.cache[key]
        seq = next(self.probe_seq)
        path = self.workdir / f"probe-{seq:05d}.zsh"
        path.write_text(script)
        # The shell under test is reached through a per-probe symlink.  Two
        # constraints shape it:
        #
        # * The basename MUST be "zsh".  zsh picks its emulation from the FIRST
        #   CHARACTER of argv[0] (Src/options.c:533-548 -- 's' or 'b' means
        #   EMULATE_SH), so a link called "sut-00001" starts the shell under
        #   test in sh emulation and comptestinit then hangs forever waiting
        #   for a prompt.  Measured: identical scripts returned 24 lines
        #   through a link named "abc" and nothing at all through "sut-a".
        # * The *directory* is unique per probe, so a hung probe's shell can be
        #   found and killed by path.  comptest starts it inside zpty and zpty's
        #   child setsid()s to take the pty as its controlling terminal, which
        #   puts it outside our process group where killpg cannot reach it; a
        #   leaked one spins at ~100% CPU and slows every later probe.
        linkdir = self.workdir / f"p{seq:05d}"
        linkdir.mkdir(exist_ok=True)
        link = linkdir / "zsh"
        if link.is_symlink() or link.exists():
            link.unlink()
        link.symlink_to(shell)
        env = {
            "HOME": os.environ.get("HOME", "/"),
            "PATH": "/usr/bin:/bin:/usr/sbin:/sbin",
            "TERM": "xterm",
            "LANG": "C",
            "TMPDIR": str(self.workdir),
        }
        env.update(env_extra)
        # start_new_session + killpg, because a probe that hangs is the normal
        # case here (comptest blocks on zpty when the shell under test dies) and
        # killing only the direct child leaves a spinning orphan behind that
        # steals a core from every later probe.  Measured: one such orphan was
        # still at 100% CPU minutes after its runner was killed.
        proc = subprocess.Popen(
            [str(self.harness), "-f", str(path), str(link), str(self.zsh_build)],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            env=env,
            cwd=str(self.workdir),
            start_new_session=True,
        )
        try:
            so, se = proc.communicate(timeout=self.timeout)
            out = so.decode("utf-8", "replace").splitlines()
            if TARGET_MARKER in out:
                out = out[len(out) - 1 - out[::-1].index(TARGET_MARKER) + 1 :]
            self.last_err[str(shell)] = se.decode("utf-8", "replace").strip()
        except subprocess.TimeoutExpired:
            try:
                os.killpg(os.getpgid(proc.pid), signal.SIGKILL)
            except OSError:
                proc.kill()
            subprocess.run(["pkill", "-9", "-f", str(linkdir)], capture_output=True)
            try:
                proc.communicate(timeout=2)  # everything is already flushed
            except subprocess.TimeoutExpired:
                pass
            out = ["<<TIMED-OUT after %ds>>" % self.timeout]
            self.last_err[str(shell)] = "timed out"
            self.cache.pop(key, None)
        finally:
            if link.is_symlink():
                link.unlink()
            linkdir.rmdir()
            path.unlink(missing_ok=True)
        self.cache[key] = out
        return out

    def diverges(self, rep: Repro) -> tuple[bool, list[str], list[str]]:
        """True when the two shells still disagree *in the same way*.

        ``self.signature`` is fixed by the first probe; a candidate that
        diverges over different comptest line kinds is rejected, so reduction
        cannot drift onto an unrelated bug.
        """
        script = build_driver(
            rep, build=self.zsh_build, utf8_fn=self.utf8_fn, cmds=[], banner="probe",
            marker=True,
        )
        self.spent += 1
        # The two shells are independent; running them at once roughly halves
        # the wall clock of a reduction, which matters because a hung probe
        # costs the full timeout.
        with cf.ThreadPoolExecutor(max_workers=2) as pool:
            fa = pool.submit(self._run, self.shell_a, script, {})
            fb = pool.submit(self._run, self.shell_b, script, self.sut_env)
            a, b = fa.result(), fb.result()
        timed_a = bool(a) and a[0].startswith("<<TIMED-OUT")
        timed_b = bool(b) and b[0].startswith("<<TIMED-OUT")
        if timed_a and not timed_b:
            # Only the reference hung, which means the probe measured the host
            # rather than the shells (a candidate that breaks the driver hangs
            # both sides, and that is a legitimate "no divergence" answer worth
            # no retry at all).  Retry once with room to spare rather than let a
            # loaded machine silently steer the reduction.
            self.invalid_probes += 1
            saved, self.timeout = self.timeout, self.timeout * 3
            try:
                a = self._run(self.shell_a, script, {})
                b = self._run(self.shell_b, script, self.sut_env)
            finally:
                self.timeout = saved
        keep = a != b and (
            self.signature is None or divergence_signature(a, b) == self.signature
        )
        if self.verbose:
            print(
                f"    probe {self.spent:3d}/{self.budget} "
                f"{'keep' if keep else 'drop'} "
                f"(ctx={len(rep.context)} extras={len(rep.extras)} "
                f"target={sum(len(ln) for ln in rep.target)}b)",
                file=sys.stderr,
            )
        return keep, a, b

    def exhausted(self) -> bool:
        return self.spent >= self.budget

    # -- reduction --------------------------------------------------------
    def _greedy_drop(self, rep: Repro, attr: str) -> bool:
        """Remove items from ``rep.<attr>`` while divergence survives.

        Returns True if the pass ran to completion (i.e. it converged rather
        than stopping because the probe budget ran out).
        """
        items = list(getattr(rep, attr))
        if not items:
            return True
        # cheap first move: is any of it needed at all?
        setattr(rep, attr, [])
        if self.exhausted():
            setattr(rep, attr, items)
            return False
        if self.diverges(rep)[0]:
            return True
        setattr(rep, attr, items)

        i = 0
        while i < len(items):
            if self.exhausted():
                setattr(rep, attr, items)
                return False
            trial = items[:i] + items[i + 1 :]
            setattr(rep, attr, trial)
            if self.diverges(rep)[0]:
                items = trial
            else:
                i += 1
        setattr(rep, attr, items)
        return True

    def _reduce_target(self, rep: Repro) -> bool:
        """Shrink the assertion's own code: setup words first, then keystrokes."""
        ok = True
        # (a) drop whole leading setup statements (e.g. a second tst_arguments
        #     call).  Statements, not lines: a brace block must stay balanced.
        stmts = group_statements(rep.target)
        i = 0
        while i < len(stmts) - 1:
            if self.exhausted():
                rep.target = [ln for s in stmts for ln in s]
                return False
            trial = stmts[:i] + stmts[i + 1 :]
            rep.target = [ln for s in trial for ln in s]
            if self.diverges(rep)[0]:
                stmts = trial
            else:
                i += 1
        lines = [ln for s in stmts for ln in s]
        rep.target = list(lines)

        # (b) drop arguments of single-line setup statements
        single = {s[0] for s in stmts if len(s) == 1}
        for li in range(len(lines) - 1):
            if lines[li] not in single:
                continue
            words = split_words(lines[li])
            if len(words) <= 1:
                continue
            wi = 1
            while wi < len(words):
                if self.exhausted():
                    return False
                trial_words = words[:wi] + words[wi + 1 :]
                indent = lines[li][: len(lines[li]) - len(lines[li].lstrip())]
                saved = lines[li]
                lines[li] = indent + " ".join(trial_words)
                rep.target = list(lines)
                if self.diverges(rep)[0]:
                    words = trial_words
                else:
                    lines[li] = saved
                    wi += 1
            rep.target = list(lines)

        # (c) shorten the keystroke string of the last comptest/zletest line
        li_key = next(
            (i for i in range(len(lines) - 1, -1, -1) if re.search(r"\$'", lines[i])), None
        )
        if li_key is None:
            return ok
        last = lines[li_key]
        m = re.search(r"\$'((?:\\.|[^'\\])*)'", last)
        if not m:
            return ok
        toks = tokenise_dollar_quote(m.group(1))
        prefix, suffix = last[: m.start(1)], last[m.end(1) :]

        def with_tokens(ts: list[str]) -> str:
            return prefix + "".join(ts) + suffix

        # suffix truncation first: these inputs are mostly repeated <TAB>s
        n = len(toks)
        cut = n // 2
        while cut >= 1:
            if self.exhausted():
                return False
            trial = toks[: n - cut]
            lines[li_key] = with_tokens(trial)
            rep.target = list(lines)
            if trial and self.diverges(rep)[0]:
                toks = trial
                n = len(toks)
                cut = n // 2
            else:
                cut //= 2
        lines[li_key] = with_tokens(toks)
        rep.target = list(lines)

        i = 0
        while i < len(toks):
            if self.exhausted():
                return False
            trial = toks[:i] + toks[i + 1 :]
            if not trial:
                break
            lines[li_key] = with_tokens(trial)
            rep.target = list(lines)
            if self.diverges(rep)[0]:
                toks = trial
            else:
                i += 1
        lines[li_key] = with_tokens(toks)
        rep.target = list(lines)
        return ok

    def minimize(self, doc_pre: list[str], chunks: list[Chunk], origin: str, index: int) -> Repro:
        pre_extras, extras = split_prep(doc_pre)
        target = chunks[index - 1]
        rep = Repro(
            origin=f"{origin}#{index}",
            index=index,
            message=target.message,
            pre_extras=list(pre_extras),
            extras=[list(e) for e in extras],
            context=[list(c.code) for c in chunks[: index - 1]],
            target=list(target.code),
        )
        start = self.spent
        inv_start = self.invalid_probes
        self.signature = None  # the first probe defines what "diverges" means
        diverged, a, b = self.diverges(rep)
        rep.baseline_diverged = diverged
        if diverged:
            self.signature = divergence_signature(a, b)
            rep.signature = sorted(self.signature)
        if not diverged:
            if not a and not b:
                rep.note = (
                    "the generated driver produced no output from either shell, so this "
                    "says nothing about the assertion. stderr: "
                    + " | ".join(
                        f"{k.rsplit('/', 1)[-1]}: {v}"
                        for k, v in self.last_err.items()
                        if v
                    )
                )
            else:
                rep.note = (
                    "the two shells agree once the assertion is replayed standalone; "
                    "the in-suite failure depends on state this driver does not reproduce"
                )
            rep.probes = self.spent - start
            rep.invalid_probes = self.invalid_probes - inv_start
            rep.out_a, rep.out_b = a, b
            return rep

        # Order matters for cost, not just for size: the keystroke string is
        # what makes a probe hang (a stray ^D on an empty line exits the shell
        # under test and comptest then blocks on zpty until the probe timeout),
        # so shortening the input first makes every later probe cheap.
        converged = True
        converged &= self._reduce_target(rep)
        converged &= self._greedy_drop(rep, "context")
        converged &= self._greedy_drop(rep, "extras")
        converged &= self._greedy_drop(rep, "pre_extras")
        # one more sweep now that the surroundings are smaller
        if converged and not self.exhausted():
            converged &= self._reduce_target(rep)

        ok, a, b = self.diverges(rep)
        rep.out_a, rep.out_b = a, b
        rep.converged = bool(converged) and ok
        if not ok:
            rep.note = "final re-check no longer diverges (non-deterministic assertion)"
        elif not converged:
            rep.note = f"probe budget of {self.budget} exhausted; result is not 1-minimal"
        rep.probes = self.spent - start
        rep.invalid_probes = self.invalid_probes - inv_start
        return rep


def do_minimize(args, zsh_build: Path, harness: Path, sut: Path, sut_env: dict[str, str]) -> int:
    """``--minimize FILE#N`` (repeatable) or ``--minimize-from <json>``."""
    targets: list[tuple[str, int]] = []
    for spec in args.minimize:
        name, _, idx = spec.partition("#")
        if not idx.isdigit():
            die(f"--minimize wants FILE#N, got {spec!r}")
        targets.append((name, int(idx)))
    if args.minimize_from:
        data = json.loads(Path(args.minimize_from).read_text())
        for r in data["results"]:
            for a in r["assertions"]:
                if a["status"] in ("fail", "xpass"):
                    targets.append((r["name"], a["index"]))
    if args.minimize_limit:
        targets = targets[: args.minimize_limit]
    if not targets:
        die("nothing to minimize")

    repro_dir = (
        Path(args.repro_dir)
        if args.repro_dir
        else REPO / "tests" / "ztst_compsys" / "repros"
    )
    repro_dir.mkdir(parents=True, exist_ok=True)
    work = Path(tempfile.mkdtemp(prefix="ztst_min."))

    # Same reason as the run snapshot in main(): a reduction is hundreds of
    # probes long and a peer relinking target/debug/zshrs halfway through would
    # silently change what is being reduced.
    if not args.no_sut_snapshot:
        snap = work / "sut0" / "zsh"
        snap.parent.mkdir(parents=True)
        shutil.copy2(sut, snap)
        snap.chmod(0o755)
        sut_for_probes = snap
    else:
        sut_for_probes = sut

    zsh_ref = zsh_build / "Src" / "zsh"
    mini = Minimizer(
        zsh_build=zsh_build,
        harness=harness,
        shell_a=zsh_ref,
        shell_b=sut_for_probes,
        sut_env=sut_env,
        budget=args.minimize_budget,
        timeout=args.minimize_timeout,
        workdir=work,
    )

    out_report: list[str] = []
    out_report.append("# assertion -> minimal standalone repro")
    out_report.append("")
    out_report.append(f"reference shell : {zsh_ref}")
    out_report.append(f"shell under test: {sut}")
    out_report.append(f"budget per assertion: {args.minimize_budget} probe pairs")
    out_report.append(f"probe timeout   : {args.minimize_timeout}s")
    out_report.append("")

    records: list[dict] = []
    for name, index in targets:
        ztst = zsh_build / "Test" / f"{name}.ztst"
        if not ztst.is_file():
            out_report.append(f"## {name}#{index}: no such file {ztst}")
            continue
        pre, chunks = parse_ztst_struct(ztst)
        if index > len(chunks):
            out_report.append(f"## {name}#{index}: only {len(chunks)} assertions in {name}")
            continue
        mini.spent = 0
        before = list(chunks[index - 1].code)
        print(f"[minimize] {name}#{index} {chunks[index-1].message}", file=sys.stderr)
        rep = mini.minimize(pre, chunks, name, index)

        path = repro_dir / f"{name.lower()}_{index:03d}.zsh"
        cmds = [
            f"{harness} -f {path} {zsh_ref} {zsh_build} > /tmp/a.txt",
            (
                " ".join(f"{k}={v}" for k, v in sorted(sut_env.items())) + " "
                if sut_env
                else ""
            )
            + f"{harness} -f {path} {sut} {zsh_build} > /tmp/b.txt",
            "diff -u /tmp/a.txt /tmp/b.txt",
        ]
        rep.path = str(path)
        if rep.baseline_diverged:
            path.write_text(
                build_driver(
                    rep,
                    build=zsh_build,
                    utf8_fn=mini.utf8_fn,
                    cmds=cmds,
                    banner="GENERATED by scripts/ztst_compsys.py --minimize; do not edit",
                )
            )

        verdict = (
            "converged"
            if rep.converged
            else ("budget exhausted" if rep.baseline_diverged else "not reproducible standalone")
        )
        out_report.append(f"## {name}#{index}  {rep.message}")
        out_report.append(f"    reduction: {verdict}, {rep.probes} probe pairs spent")
        if rep.invalid_probes:
            out_report.append(
                f"    {rep.invalid_probes} probe(s) had the reference shell time out and "
                "were retried with a longer timeout (host was loaded)"
            )
        if rep.signature:
            out_report.append(
                "    anchored on divergence in: " + ", ".join(rep.signature)
            )
        if rep.note:
            out_report.append(f"    note: {rep.note}")
        out_report.append("")
        out_report.append("    before (upstream assertion code):")
        out_report += [f"      {ln.strip()}" for ln in before]
        out_report.append("")
        out_report.append("    after (reduced):")
        kept = (
            [f"      {ln}" for e in rep.pre_extras for ln in ([e] if isinstance(e, str) else e)]
            + [f"      {ln.strip()}" for e in rep.extras for ln in e]
            + [f"      {ln.strip()}" for c in rep.context for ln in c]
            + [f"      {ln.strip()}" for ln in rep.target]
        )
        out_report += kept or ["      (empty)"]
        out_report.append("")
        if rep.baseline_diverged:
            out_report.append(f"    repro: {path}")
            for c in cmds:
                out_report.append(f"      {c}")
            out_report.append("")
            out_report.append("    zsh:")
            out_report += [f"      {ln}" for ln in rep.out_a] or ["      (no output)"]
            out_report.append("    sut:")
            out_report += [f"      {ln}" for ln in rep.out_b] or ["      (no output)"]
        out_report.append("")
        records.append(
            {
                "origin": rep.origin,
                "message": rep.message,
                "probes": rep.probes,
                "converged": rep.converged,
                "signature": rep.signature,
                "reproducible_standalone": rep.baseline_diverged,
                "note": rep.note,
                "repro": str(path) if rep.baseline_diverged else "",
                "kept_lines": [ln.strip() for ln in kept],
                "zsh": rep.out_a,
                "sut": rep.out_b,
            }
        )

    text = "\n".join(out_report)
    print(text)
    if args.out:
        Path(args.out).write_text(text + "\n")
    if args.json:
        Path(args.json).write_text(json.dumps(records, indent=1) + "\n")
    # Anything still alive under the probe workdir escaped its process group
    # (zpty children setsid away); leave none behind to spin on the host.
    subprocess.run(["pkill", "-9", "-f", str(work)], capture_output=True)
    shutil.rmtree(work, ignore_errors=True)
    return 0


# ---------------------------------------------------------------------------
# Gate: pin the per-assertion state, then report movement in both directions.
# ---------------------------------------------------------------------------

def binary_identity(path: Path) -> dict:
    """Everything needed to say later whether this binary moved."""
    try:
        st = path.stat()
    except OSError as exc:
        return {"path": str(path), "error": str(exc)}
    h = hashlib.sha256()
    with path.open("rb") as fh:
        for block in iter(lambda: fh.read(1 << 20), b""):
            h.update(block)
    ident = {
        "path": str(path),
        "size": st.st_size,
        "mtime": time.strftime("%Y-%m-%dT%H:%M:%S", time.localtime(st.st_mtime)),
        "sha256_prefix": h.hexdigest()[:16],
    }
    for flag, key in (("--version", "version"),):
        try:
            p = subprocess.run(
                [str(path), flag],
                capture_output=True,
                timeout=30,
                env={"HOME": os.environ.get("HOME", "/"), "PATH": "/usr/bin:/bin"},
            )
            ident[key] = p.stdout.decode().strip().splitlines()[0] if p.stdout.strip() else ""
        except Exception:
            ident[key] = ""
    return ident


def identity_delta(pinned: dict, now: dict) -> list[str]:
    keys = ("size", "mtime", "sha256_prefix", "version")
    return [
        f"{k}: {pinned.get(k)!r} -> {now.get(k)!r}"
        for k in keys
        if pinned.get(k) != now.get(k)
    ]


def results_to_map(results: list[FileResult]) -> dict[str, dict[str, str]]:
    return {r.name: {str(a.index): a.status for a in r.assertions} for r in results}


def gate_compare(pinned: dict, now: dict) -> tuple[dict[str, list[str]], dict[str, int]]:
    """Per-assertion verdicts between a pinned map and a fresh one."""
    verdicts: dict[str, list[str]] = {
        "REGRESSED": [],
        "FIXED": [],
        "CHANGED": [],
        "NEW": [],
        "MISSING": [],
    }
    tally = {"UNCHANGED": 0}
    files = sorted(set(pinned) | set(now))
    for f in files:
        p = pinned.get(f, {})
        n = now.get(f, {})
        for idx in sorted(set(p) | set(n), key=lambda s: int(s)):
            ps = p.get(idx)
            ns = n.get(idx)
            if ps is None:
                verdicts["NEW"].append(f"{f} #{idx} now={ns}")
            elif ns is None:
                verdicts["MISSING"].append(f"{f} #{idx} was={ps}")
            elif ps == ns:
                tally["UNCHANGED"] += 1
            elif ps == "pass":
                verdicts["REGRESSED"].append(f"{f} #{idx} was=pass now={ns}")
            elif ns == "pass":
                verdicts["FIXED"].append(f"{f} #{idx} was={ps} now=pass")
            else:
                verdicts["CHANGED"].append(f"{f} #{idx} was={ps} now={ns}")
    for k, v in verdicts.items():
        tally[k] = len(v)
    return verdicts, tally


def render_gate(pinned_doc: dict, now_map: dict, now_ident: dict, meta: dict) -> tuple[str, int]:
    out: list[str] = []
    out.append("# ztst_compsys gate")
    out.append("")
    out.append(f"pinned at   : {pinned_doc.get('pinned_at')}")
    out.append(f"pinned sut  : {pinned_doc.get('sut', {}).get('version')} "
               f"{pinned_doc.get('sut', {}).get('sha256_prefix')}")
    out.append(f"current sut : {now_ident.get('version')} {now_ident.get('sha256_prefix')}")
    delta = identity_delta(pinned_doc.get("sut", {}), now_ident)
    out.append("binary moved: " + ("yes" if delta else "no"))
    for d in delta:
        out.append(f"  {d}")
    out.append(f"zsh build   : {meta.get('zsh_build_version')} ({meta.get('zsh_build')})")
    pinned_timeout = pinned_doc.get("timeout")
    if pinned_timeout is not None and pinned_timeout != meta.get("timeout"):
        out.append(
            f"per-file timeout: pinned {pinned_timeout}s, this run {meta.get('timeout')}s "
            "-- a shorter one turns hung files' assertions into CHANGED"
        )
    out.append("")

    verdicts, tally = gate_compare(pinned_doc.get("suite", {}), now_map)
    order = ["UNCHANGED", "REGRESSED", "FIXED", "CHANGED", "NEW", "MISSING"]
    out.append(" ".join(f"{k}={tally.get(k, 0)}" for k in order))
    out.append("")
    for k in order[1:]:
        if not verdicts[k]:
            continue
        out.append(f"## {k} ({len(verdicts[k])})")
        out += [f"  {ln}" for ln in verdicts[k]]
        if k == "FIXED":
            out.append(
                "  -> the binary moved since the pin, so these are plausibly real fixes"
                if delta
                else "  -> the binary did NOT move since the pin; a FIXED verdict with an"
                " unchanged binary means the run is flaky, not that anything was fixed"
            )
        out.append("")

    if tally.get("REGRESSED"):
        code = EXIT_REGRESSED
        out.append("verdict: REGRESSED")
    elif any(tally.get(k) for k in ("FIXED", "CHANGED", "NEW", "MISSING")):
        code = EXIT_MOVED
        out.append("verdict: MOVED (no regression)")
    else:
        code = EXIT_UNCHANGED
        out.append("verdict: UNCHANGED")
    return "\n".join(out), code


# ---------------------------------------------------------------------------
# Wider suite: every non-Y .ztst, with the shell under test as its own harness.
# ---------------------------------------------------------------------------


def discover_core_tests(zsh_build: Path) -> list[str]:
    """Every .ztst in the tree that is not part of the Y completion series."""
    return sorted(
        p.stem
        for p in (zsh_build / "Test").glob("*.ztst")
        if not p.stem.startswith("Y0")
    )


def main() -> int:
    ap = _Parser(description=__doc__.split("\n")[0])
    ap.add_argument("--sut", help="shell binary under test (default: target/debug/zshrs)")
    ap.add_argument("--label", help="label for the report (default: derived from --sut)")
    ap.add_argument("--baseline", action="store_true", help="run the zsh baseline instead")
    ap.add_argument("--zsh-build", help="built zsh source tree (Src/zsh + Completion + Test)")
    ap.add_argument("--harness", help="zsh binary that interprets ztst.zsh (needs zsh/zpty)")
    ap.add_argument("--tests", help=f"comma-separated subset of {','.join(ALL_TESTS)}")
    ap.add_argument("--timeout", type=int, default=600, help="per-file timeout in seconds")
    ap.add_argument(
        "--fx",
        choices=["off", "on"],
        default="off",
        help="zshrs native ZLE effects; 'off' exports ZSHRS_NATIVE_ZLE_FX=0 (default)",
    )
    ap.add_argument(
        "--sut-env",
        action="append",
        default=[],
        metavar="K=V",
        help="extra environment for the shell under test (repeatable)",
    )
    ap.add_argument("--out", help="write the human-readable report here")
    ap.add_argument("--json", help="write the machine-readable result here")
    ap.add_argument("--failures", help="write the failure detail here")
    ap.add_argument("--compare-to", help="a --json from a previous run to diff against")
    ap.add_argument("--keep", action="store_true", help="keep the scratch run directory")

    g = ap.add_argument_group("wider suite (shell parity, NOT compsys)")
    g.add_argument(
        "--core",
        action="store_true",
        help="run every non-Y .ztst with the shell under test as its own ztst harness",
    )
    g.add_argument("--core-tests", help="comma-separated subset of the non-Y files")
    g.add_argument("--core-timeout", type=int, default=180, help="per-file timeout for --core")

    ap.add_argument(
        "--no-sut-snapshot",
        action="store_true",
        help="run the shell binary in place instead of copying it for the run "
             "(a concurrent rebuild can then corrupt the results)",
    )

    g = ap.add_argument_group("gate")
    g.add_argument("--pin", action="store_true",
                   help="write this run's per-assertion state as the pin")
    g.add_argument("--gate", action="store_true", help="compare this run against the pin")
    g.add_argument("--gate-file",
                   help="pin location (default: tests/ztst_compsys/gate[_core].json)")

    g = ap.add_argument_group("assertion -> minimal repro")
    g.add_argument("--minimize", action="append", default=[], metavar="FILE#N",
                   help="reduce this assertion to a standalone repro (repeatable)")
    g.add_argument("--minimize-from", metavar="JSON",
                   help="reduce every failing assertion recorded in a previous --json")
    g.add_argument("--minimize-limit", type=int, default=0, help="stop after N assertions")
    g.add_argument("--minimize-budget", type=int, default=150,
                   help="probe pairs allowed per assertion before reduction gives up")
    g.add_argument("--minimize-timeout", type=int, default=10,
                   help="per-probe timeout; a probe that hangs costs this in full")
    g.add_argument("--repro-dir", help="where to write the generated .zsh repros")
    args = ap.parse_args()

    zsh_build = find_zsh_build(args.zsh_build)
    harness = Path(args.harness).expanduser() if args.harness else zsh_build / "Src" / "zsh"
    if not harness.is_file():
        die(f"harness zsh not found: {harness}")

    if args.baseline:
        sut = zsh_build / "Src" / "zsh"
        label = args.label or f"zsh baseline ({zsh_build})"
    else:
        sut = Path(args.sut).expanduser() if args.sut else REPO / "target" / "debug" / "zshrs"
        label = args.label or f"candidate {sut}"
    sut = sut.resolve()
    if not sut.is_file():
        die(f"shell under test not found: {sut}")

    if args.pin and args.gate:
        die("--pin and --gate together would only compare a run against itself")

    core_mode = bool(args.core or args.core_tests)
    if core_mode:
        tests = args.core_tests.split(",") if args.core_tests else discover_core_tests(zsh_build)
        file_harness = None  # the snapshotted sut, resolved once it exists
        timeout = args.core_timeout
    else:
        tests = args.tests.split(",") if args.tests else ALL_TESTS
        file_harness = harness
        timeout = args.timeout

    sut_env: dict[str, str] = {}
    if args.fx == "off":
        sut_env["ZSHRS_NATIVE_ZLE_FX"] = "0"
    for kv in args.sut_env:
        k, _, v = kv.partition("=")
        sut_env[k] = v

    if args.minimize or args.minimize_from:
        return do_minimize(args, zsh_build, harness, sut, sut_env)

    run_root = Path(tempfile.mkdtemp(prefix="ztst_compsys."))
    modules = run_root / "Modules"
    nmods = collect_modules(zsh_build, modules)

    # Up to 16 instances of this repo build concurrently, so target/debug/zshrs
    # can be rewritten underneath a run.  Measured: a gate run that started at
    # 06:21 read the binary while a peer relinked it at 06:25 and reported
    # Y04regexargs as REGRESSED (the whole file hung); the next run over the
    # finished binary was UNCHANGED.  Copy it once up front so every file in a
    # run sees the same bytes.  The basename must stay "zsh": zsh picks its
    # emulation from argv[0]'s first character (Src/options.c:533-548).
    run_sut = sut
    if not args.no_sut_snapshot:
        snap = run_root / "sut" / "zsh"
        snap.parent.mkdir()
        shutil.copy2(sut, snap)
        snap.chmod(0o755)
        run_sut = snap

    def version_of(binary: Path) -> str:
        try:
            p = subprocess.run(
                [str(binary), "-f", "-c", "print -r -- $ZSH_VERSION $ZSH_PATCHLEVEL"],
                capture_output=True,
                timeout=30,
                env={"HOME": os.environ.get("HOME", "/"), "PATH": "/usr/bin:/bin"},
            )
            return p.stdout.decode().strip() or "?"
        except Exception as exc:  # pragma: no cover - diagnostic only
            return f"<{exc}>"

    if core_mode:
        # A non-Y .ztst has no comptestinit: its assertions run *in* the ztst
        # harness, so the shell under test has to be the harness for those.
        file_harness = run_sut

    meta = {
        "mode": "core (shell parity)" if core_mode else "compsys (Y series)",
        "sut_snapshot": "yes" if run_sut != sut else "no (--no-sut-snapshot)",
        "sut": str(sut),
        "sut_version": version_of(sut),
        "harness": str(file_harness),
        "harness_version": version_of(file_harness),
        "zsh_build": str(zsh_build),
        "zsh_build_version": next(
            (
                ln.split("=", 1)[1]
                for ln in (zsh_build / "Config" / "version.mk").read_text().splitlines()
                if ln.startswith("VERSION=")
            ),
            "?",
        ),
        "modules_linked": nmods,
        "fx": args.fx,
        "sut_env": " ".join(f"{k}={v}" for k, v in sorted(sut_env.items())) or "(none)",
        "tests": ",".join(tests),
        "timeout": timeout,
        "date": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
    }

    print(f"[ztst_compsys] {label}", file=sys.stderr)
    for k, v in meta.items():
        print(f"  {k}: {v}", file=sys.stderr)

    results = [
        run_one(
            name=name,
            sut=run_sut,
            zsh_build=zsh_build,
            harness=file_harness,
            modules=modules,
            run_root=run_root,
            timeout=timeout,
            sut_env=sut_env,
            verbose=True,
        )
        for name in tests
    ]

    report = render_report(label, meta, results)
    print(report)
    if args.out:
        Path(args.out).write_text(report + "\n")
    if args.failures:
        Path(args.failures).write_text(render_failures(results) + "\n")
    if args.json:
        Path(args.json).write_text(
            json.dumps(
                {"label": label, "meta": meta, "results": [asdict(r) for r in results]},
                indent=1,
            )
            + "\n"
        )
    if args.compare_to:
        prev = json.loads(Path(args.compare_to).read_text())
        prev_results = [
            FileResult(
                name=r["name"],
                exit_code=r["exit_code"],
                timed_out=r["timed_out"],
                unimplemented=r["unimplemented"],
                assertions=[Assertion(**a) for a in r["assertions"]],
            )
            for r in prev["results"]
        ]
        print()
        print(compare(prev_results, results))

    gate_code: int | None = None
    if args.pin or args.gate:
        gate_path = (
            Path(args.gate_file)
            if args.gate_file
            else (
                REPO / "tests" / "ztst_compsys"
                / ("gate_core.json" if core_mode else "gate.json")
            )
        )
        now_map = results_to_map(results)
        now_ident = binary_identity(sut)
        if args.pin:
            doc = {
                "schema": 1,
                "mode": meta["mode"],
                "pinned_at": meta["date"],
                "sut": now_ident,
                "sut_zsh_version": meta["sut_version"],
                "zsh_build": {"path": str(zsh_build), "version": meta["zsh_build_version"]},
                "fx": args.fx,
                "sut_env": meta["sut_env"],
                "tests": tests,
                # Y03arguments hangs on purpose; which of its assertions come
                # out "notrun" depends on when the runner gives up, so the
                # timeout is part of the pinned state.
                "timeout": timeout,
                "suite": now_map,
            }
            gate_path.parent.mkdir(parents=True, exist_ok=True)
            gate_path.write_text(json.dumps(doc, indent=1) + "\n")
            print(f"\n[gate] pinned {sum(len(v) for v in now_map.values())} assertions "
                  f"across {len(now_map)} files to {gate_path}", file=sys.stderr)
            # Pinning succeeded; the failures it recorded are the point of it,
            # not an error, so do not hand the caller the "tests failed" code.
            gate_code = EXIT_UNCHANGED
        if args.gate:
            if not gate_path.is_file():
                print(f"[gate] no pin at {gate_path}; run with --pin first", file=sys.stderr)
                return EXIT_RUNNER_FAILED
            pinned_doc = json.loads(gate_path.read_text())
            skipped = sorted(set(pinned_doc.get("suite", {})) - set(now_map))
            pinned_doc["suite"] = {
                k: v for k, v in pinned_doc.get("suite", {}).items() if k in now_map
            }
            text, gate_code = render_gate(pinned_doc, now_map, now_ident, meta)
            print()
            print(text)
            if skipped:
                print()
                print(f"out of scope for this run ({len(skipped)} pinned files not run): "
                      + " ".join(skipped))

    if not args.keep:
        shutil.rmtree(run_root, ignore_errors=True)
    else:
        print(f"[ztst_compsys] run dir kept: {run_root}", file=sys.stderr)

    if gate_code is not None:
        return gate_code
    summary = summarise(results)
    return 0 if summary["by_status"].get("fail", 0) == 0 else 1


if __name__ == "__main__":
    try:
        sys.exit(main())
    except SystemExit:
        raise
    except Exception:  # the runner itself broke: never mistakable for a result
        import traceback

        traceback.print_exc()
        sys.exit(EXIT_RUNNER_FAILED)

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
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass, field, asdict
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent

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
    sys.exit(
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
    }
    env.update(sut_env)
    (run_root / "tmp").mkdir(exist_ok=True)

    cmd = [str(harness), "+Z", "-f", str(zsh_build / "Test" / "ztst.zsh"), str(ztst)]
    started = time.time()
    try:
        proc = subprocess.run(
            cmd,
            cwd=rundir / "Test",
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            timeout=timeout,
        )
        res.raw = proc.stdout.decode("utf-8", "replace")
        res.exit_code = proc.returncode
    except subprocess.TimeoutExpired as exc:
        res.timed_out = True
        res.raw = (exc.stdout or b"").decode("utf-8", "replace")
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


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
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
    args = ap.parse_args()

    zsh_build = find_zsh_build(args.zsh_build)
    harness = Path(args.harness).expanduser() if args.harness else zsh_build / "Src" / "zsh"
    if not harness.is_file():
        sys.exit(f"harness zsh not found: {harness}")

    if args.baseline:
        sut = zsh_build / "Src" / "zsh"
        label = args.label or f"zsh baseline ({zsh_build})"
    else:
        sut = Path(args.sut).expanduser() if args.sut else REPO / "target" / "debug" / "zshrs"
        label = args.label or f"candidate {sut}"
    sut = sut.resolve()
    if not sut.is_file():
        sys.exit(f"shell under test not found: {sut}")

    tests = args.tests.split(",") if args.tests else ALL_TESTS

    sut_env: dict[str, str] = {}
    if args.fx == "off":
        sut_env["ZSHRS_NATIVE_ZLE_FX"] = "0"
    for kv in args.sut_env:
        k, _, v = kv.partition("=")
        sut_env[k] = v

    run_root = Path(tempfile.mkdtemp(prefix="ztst_compsys."))
    modules = run_root / "Modules"
    nmods = collect_modules(zsh_build, modules)

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

    meta = {
        "sut": str(sut),
        "sut_version": version_of(sut),
        "harness": str(harness),
        "harness_version": version_of(harness),
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
        "date": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
    }

    print(f"[ztst_compsys] {label}", file=sys.stderr)
    for k, v in meta.items():
        print(f"  {k}: {v}", file=sys.stderr)

    results = [
        run_one(
            name=name,
            sut=sut,
            zsh_build=zsh_build,
            harness=harness,
            modules=modules,
            run_root=run_root,
            timeout=args.timeout,
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

    if not args.keep:
        shutil.rmtree(run_root, ignore_errors=True)
    else:
        print(f"[ztst_compsys] run dir kept: {run_root}", file=sys.stderr)

    summary = summarise(results)
    return 0 if summary["by_status"].get("fail", 0) == 0 else 1


if __name__ == "__main__":
    sys.exit(main())

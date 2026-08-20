#!/usr/bin/env python3
"""parity_matrix.py — run the completion parity harness across the full
(case x keystroke-sequence x zstyle-combo) matrix and report a grid.

    scripts/parity_matrix.py                        # quick profile
    scripts/parity_matrix.py --profile standard
    scripts/parity_matrix.py --profile full         # everything (hours)
    scripts/parity_matrix.py --combos full,drop-menu --sequences tab1,tab2

Each cell is one `comptab_parity.py` (or `compsys_parity.py`) run: a real pty,
both shells booted with the SAME combo fixture, keystrokes replayed, screens
diffed. A cell is PASS only if the two grids are byte-identical.

Failing cells are written to an output directory as ready-to-run commands, so
a single divergence can be replayed in isolation without re-running the matrix.

Interpreter note: `python3` on this host may resolve to `pythonrs`, which does
not implement the typing/dataclass surface these harnesses use. The driver
finds a CPython explicitly and says which one it picked.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shlex
import shutil
import subprocess
import sys
import time

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SCRIPTS = os.path.join(REPO, "scripts")
COMBOS = os.path.join(SCRIPTS, "parity_combos")

sys.path.insert(0, SCRIPTS)
from parity_corpus import CASES, DEFAULT_SEQUENCES, KEY_SEQUENCES, cases_by_tag  # noqa: E402

# name -> (combos, sequences, tag, skip_optional)
PROFILES = {
    # smoke: does the engine agree at all, under the real config. Carries ALL
    # FOUR arrow directions — a `tab_down`-only battery misses the
    # step-back-off-the-first-entry list-clear divergence entirely.
    "quick": (["full"],
              ["tab1", "tab2", "tab_down", "tab_up", "tab_right", "tab_left",
               "tab_down_up", "tab_ctrl_g"],
              None, True),
    # the axes most likely to change rendering, on the default battery
    "standard": (
        ["full", "none", "minimal", "drop-menu", "drop-format", "drop-listcolors",
         "drop-groupname", "force-menu-select", "force-listpacked-on"],
        list(DEFAULT_SEQUENCES),
        None,
        True,
    ),
    # everything the generator emitted, every sequence, every case
    "full": (None, list(KEY_SEQUENCES), None, False),
}


def find_cpython() -> str:
    """A CPython that can run the harnesses. `python3` may be pythonrs."""
    cands = [
        sys.executable,
        "/opt/homebrew/bin/python3",
        "/usr/local/bin/python3",
        "/usr/bin/python3",
    ]
    for c in cands:
        if not c or not os.path.exists(c):
            continue
        try:
            out = subprocess.run(
                [c, "-c", "import sys,dataclasses;print(sys.version_info[:2])"],
                capture_output=True, text=True, timeout=15,
            )
        except Exception:
            continue
        if out.returncode == 0 and out.stdout.strip().startswith("("):
            return c
    sys.exit("parity_matrix: no usable CPython found (tried: %s)" % ", ".join(map(str, cands)))


def available_combos() -> list[str]:
    if not os.path.isdir(COMBOS):
        sys.exit(f"no combo dir: {COMBOS} (run scripts/gen_parity_combos.py)")
    return sorted(
        os.path.splitext(f)[0] for f in os.listdir(COMBOS) if f.endswith(".zsh")
    )


SUMMARY_RE = re.compile(r"^# (\d+) passed, (\d+) failed, (\d+) cell")
FAILCELL_RE = re.compile(r"^#\s+--case (.*) --keys (\S+)$")


def _scrape(log: str, rc: int) -> tuple[int, int, list[tuple[str, str]]]:
    """Legacy stdout scrape — the fallback when the harness produced no JSON.

    Kept because a harness that died before writing its JSON still printed
    whatever it got through, and losing that is worse than parsing it loosely.
    `_collect` prefers the JSON.
    """
    passed = failed = 0
    cells: list[tuple[str, str]] = []
    for line in open(log, errors="replace"):
        m = SUMMARY_RE.match(line)
        if m:
            passed, failed = int(m.group(1)), int(m.group(2))
        m = FAILCELL_RE.match(line)
        if m:
            cells.append((m.group(1), m.group(2)))
    if rc not in (0, 1):
        # 0 = all pass, 1 = divergences. Anything else is the harness itself
        # failing (missing binary, no pty, import error) — never silently a pass.
        cells.append(("<harness exited %d>" % rc, "-"))
        failed = max(failed, 1)
    return passed, failed, cells


def _collect(log: str, jsonpath: str, rc: int
             ) -> tuple[int, int, list[tuple[str, str]], list[dict]]:
    """One harness run's numbers, from its JSON when it wrote one.

    Scraping stdout meant every count depended on prose staying byte-stable and
    on no completion listing ever printing a line that looked like a summary.
    The JSON carries the per-cell verdicts directly, so the matrix aggregates
    the same objects the harness scored rather than a re-parse of its report.

    A missing or unreadable JSON is never treated as a clean run: the stdout
    scrape is used and, if that yields nothing while the harness exited
    abnormally, the run is counted as a failure.
    """
    results: list[dict] = []
    if jsonpath and os.path.exists(jsonpath):
        try:
            with open(jsonpath) as f:
                doc = json.load(f)
            results = doc.get("results", [])
            summ = doc.get("summary", {})
            passed = int(summ.get("passed", 0))
            failed = int(summ.get("failed", 0))
            cells = [(r["buffer"], ",".join(r.get("keys", [])))
                     for r in results if r.get("status") != "PASS"]
            if rc not in (0, 1):
                cells.append(("<harness exited %d>" % rc, "-"))
                failed = max(failed, 1)
            return passed, failed, cells, results
        except (ValueError, KeyError, OSError) as exc:
            print(f"# warning: unreadable json {jsonpath}: {exc} — falling back "
                  f"to the stdout scrape", file=sys.stderr)
    p_, f_, c_ = _scrape(log, rc)
    if not p_ and not f_ and rc != 0:
        f_ = max(f_, 1)
        c_.append(("<no results parsed from %s>" % os.path.basename(log), "-"))
    return p_, f_, c_, results


def run_combo(py: str, harness: str, combo: str, sequences: list[str],
              tag: str | None, skip_optional: bool, extra: list[str],
              logdir: str, cases: list, jobs: int = 1
              ) -> tuple[int, int, list[tuple[str, str]], str, list[dict]]:
    """One combo, optionally sharded across `jobs` concurrent harness runs.

    Sharding is by CASE: every cell is an independent pty pair either way, so
    splitting the case list changes only how many run at once. At --jobs 1 this
    is a single process and the log name is unchanged.
    """
    script = os.path.join(SCRIPTS, "comptab_parity.py" if harness == "native"
                          else "compsys_parity.py")

    def base_cmd() -> list[str]:
        c = [py, script, "--zstyle", os.path.join(COMBOS, f"{combo}.zsh"),
             "--sequences", ",".join(sequences)]
        if harness == "native":
            c += ["--mode", "native"]
        return c

    if jobs <= 1:
        cmd = base_cmd()
        if tag:
            cmd += ["--tag", tag]
        if skip_optional:
            cmd += ["--skip-optional"]
        log = os.path.join(logdir, f"{harness}.{combo}.log")
        js = log[:-4] + ".json"
        cmd += ["--json", js] + extra
        with open(log, "w") as f:
            f.write("$ " + " ".join(cmd) + "\n\n")
            f.flush()
            rc = subprocess.run(cmd, stdout=f, stderr=subprocess.STDOUT).returncode
        p_, f_, cells, results = _collect(log, js, rc)
        return p_, f_, cells, log, results

    # Round-robin so a slow tag (huge listings) spreads over the shards instead
    # of landing entirely on one.
    shards: list[list] = [[] for _ in range(jobs)]
    for i, c in enumerate(cases):
        shards[i % jobs].append(c)
    shards = [sh for sh in shards if sh]

    procs = []
    for i, sh in enumerate(shards):
        corpus = os.path.join(logdir, f"{harness}.{combo}.shard{i}.cases")
        with open(corpus, "w") as f:
            for c in sh:
                f.write(c.buffer + "\n")
        log = os.path.join(logdir, f"{harness}.{combo}.shard{i}.log")
        js = log[:-4] + ".json"
        cmd = base_cmd() + ["--corpus", corpus, "--json", js] + extra
        fh = open(log, "w")
        fh.write("$ " + " ".join(cmd) + "\n\n")
        fh.flush()
        procs.append((subprocess.Popen(cmd, stdout=fh, stderr=subprocess.STDOUT),
                      fh, log, js))

    passed = failed = 0
    cells: list[tuple[str, str]] = []
    results: list[dict] = []
    for proc, fh, log, js in procs:
        rc = proc.wait()
        fh.close()
        p_, f_, c_, r_ = _collect(log, js, rc)
        passed += p_
        failed += f_
        cells += c_
        results += r_
    return (passed, failed, cells,
            os.path.join(logdir, f"{harness}.{combo}.shard*.log"), results)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--profile", choices=sorted(PROFILES), default="quick")
    ap.add_argument("--harness", choices=("native", "zsh", "both"), default="native",
                    help="native = zshrs -f -i (the binary you launch); "
                         "zsh = zshrs --zsh emulation path")
    ap.add_argument("--combos", default=None,
                    help="comma-separated combo names, or 'all' (default: from profile)")
    ap.add_argument("--sequences", default=None,
                    help="comma-separated sequence names (default: from profile)")
    ap.add_argument("--tag", default=None, help="restrict to cases with this tag")
    ap.add_argument("--skip-optional", action="store_true", default=None)
    ap.add_argument("--outdir", default=None, help="where logs + repro commands go")
    ap.add_argument("--random-combos", type=int, default=0, metavar="N",
                    help="after the named combos, fuzz N RANDOM subsets of the "
                         "full fixture. The named combos only label the known "
                         "axes; this is what tests the actual bar — that ANY "
                         "combination is byte-identical. Diverging subsets are "
                         "shrunk to the minimal statement set.")
    ap.add_argument("--combo-keep", type=float, default=0.5)
    ap.add_argument("--seed", type=int, default=0)
    ap.add_argument("--combo-sequence", default="tab1",
                    help="key sequence the random combos are judged on")
    ap.add_argument("--jobs", type=int, default=1, metavar="N",
                    help="run N harness processes concurrently, sharding the CASE "
                         "list across them. Each cell boots two shells through "
                         "compinit (~30s), so a full-sequence sweep is hours at "
                         "--jobs 1. Cells stay independent — every one is a fresh "
                         "pty pair — but heavy load can turn a marginal render into "
                         "a FLAKY, which the harness still counts as a failure.")
    ap.add_argument("--list-combos", action="store_true")
    ap.add_argument("--dry-run", action="store_true", help="print the plan and exit")
    ap.add_argument("extra", nargs="*", help="extra args passed through to the harness")
    args = ap.parse_args()

    if args.list_combos:
        for c in available_combos():
            print(c)
        return 0

    p_combos, p_seqs, p_tag, p_skip = PROFILES[args.profile]
    combos = (available_combos() if (args.combos == "all" or (args.combos is None and p_combos is None))
              else [c.strip() for c in args.combos.split(",")] if args.combos
              else p_combos)
    sequences = ([s.strip() for s in args.sequences.split(",")] if args.sequences
                 else p_seqs)
    tag = args.tag if args.tag is not None else p_tag
    skip_optional = args.skip_optional if args.skip_optional is not None else p_skip

    missing = [c for c in combos if not os.path.exists(os.path.join(COMBOS, f"{c}.zsh"))]
    if missing:
        sys.exit("no such combo(s): %s (scripts/gen_parity_combos.py)" % ", ".join(missing))
    unknown = [s for s in sequences if s not in KEY_SEQUENCES]
    if unknown:
        sys.exit("unknown sequence(s): %s" % ", ".join(unknown))

    py = find_cpython()
    harnesses = ["native", "zsh"] if args.harness == "both" else [args.harness]
    cases = [c for c in cases_by_tag(tag) if not (skip_optional and "optional" in c.tags)]
    per_combo = len(cases) * len(sequences)
    total = per_combo * len(combos) * len(harnesses)

    outdir = args.outdir or os.path.join(REPO, "target", "parity-matrix")
    os.makedirs(outdir, exist_ok=True)

    print(f"# python   : {py}")
    print(f"# profile  : {args.profile}")
    print(f"# harness  : {', '.join(harnesses)}")
    print(f"# combos   : {len(combos)} ({', '.join(combos[:8])}"
          + (", ..." if len(combos) > 8 else "") + ")")
    print(f"# cases    : {len(cases)}" + (f" (tag={tag})" if tag else "")
          + (" [-optional]" if skip_optional else ""))
    print(f"# sequences: {len(sequences)} ({', '.join(sequences[:8])}"
          + (", ..." if len(sequences) > 8 else "") + ")")
    print(f"# cells    : {per_combo}/combo, {total} total")
    # No hardcoded seconds-per-cell here. Each cell boots two fresh interactive
    # shells through compinit, so the rate depends on the machine, the dump and
    # how loaded the box is; a literal baked into this line goes stale silently
    # and has already been wrong by 4x. The real rate is measured from the
    # first completed combo and the remaining time is projected from it.
    print(f"# outdir   : {outdir}")
    print()
    if args.dry_run:
        return 0

    started = time.monotonic()
    grid = []
    all_fail = []
    all_results = []
    runs = [(h, c) for h in harnesses for c in combos]
    for idx, (harness, combo) in enumerate(runs):
        t0 = time.monotonic()
        passed, failed, cells, log, results = run_combo(
            py, harness, combo, sequences, tag, skip_optional, args.extra,
            outdir, cases, args.jobs)
        dt = time.monotonic() - t0
        status = "PASS" if failed == 0 else "FAIL"
        print(f"{status:4s} {harness:6s} {combo:28s} "
              f"{passed:4d} pass {failed:4d} fail  {dt:6.1f}s  {os.path.basename(log)}")
        if idx == 0 and len(runs) > 1 and per_combo:
            rate = dt / per_combo
            left = (len(runs) - 1) * per_combo * rate
            print(f"# measured : {rate:.1f}s/cell on this run -> "
                  f"~{left / 60:.0f} min left for {len(runs) - 1} more combo run(s)")
        sys.stdout.flush()
        grid.append((harness, combo, passed, failed))
        for buf, keys in cells:
            all_fail.append((harness, combo, buf, keys))
        for r in results:
            all_results.append(dict(r, harness=harness, combo=combo))

    random_rc = 0
    if args.random_combos:
        fixture = os.path.join(SCRIPTS, "parity_zstyle.zsh")
        log = os.path.join(outdir, "random-combos.log")
        cmd = [py, os.path.join(SCRIPTS, "comptab_parity.py"),
               "--zstyle", fixture,
               "--random-combos", str(args.random_combos),
               "--combo-keep", str(args.combo_keep),
               "--seed", str(args.seed),
               "--combo-sequence", args.combo_sequence]
        if tag:
            cmd += ["--tag", tag]
        if skip_optional:
            cmd += ["--skip-optional"]
        print()
        print(f"# random-combo fuzz: {args.random_combos} subsets of "
              f"{len(open(fixture).readlines())}-line fixture -> {log}")
        sys.stdout.flush()
        with open(log, "w") as f:
            f.write("$ " + " ".join(cmd) + "\n\n")
            f.flush()
            random_rc = subprocess.run(cmd, stdout=f, stderr=subprocess.STDOUT).returncode
        for line in open(log, errors="replace"):
            if line.startswith(("FAIL combo", "     minimal", "     config-", "# ")):
                print(line.rstrip())

    elapsed = time.monotonic() - started
    tot_pass = sum(g[2] for g in grid)
    tot_fail = sum(g[3] for g in grid)
    print()
    print(f"# {tot_pass} passed, {tot_fail} failed across {len(grid)} combo run(s) "
          f"in {elapsed / 60:.1f} min")

    if all_fail:
        repro = os.path.join(outdir, "repro.sh")
        with open(repro, "w") as f:
            f.write("#!/bin/sh\n# Replay each failing cell on its own.\n")
            for harness, combo, buf, keys in all_fail:
                script = "comptab_parity.py" if harness == "native" else "compsys_parity.py"
                # The buffer is shell text — `ls /usr/{b`, `echo $path[`,
                # `ls "/us` — so it MUST be quoted here. Unquoted, the replay
                # line word-split into a different case than the one that
                # failed, or did not parse at all.
                f.write(
                    f"{py} {os.path.join(SCRIPTS, script)} "
                    f"--zstyle {os.path.join(COMBOS, combo + '.zsh')} "
                    f"--case {shlex.quote(buf)} --keys {shlex.quote(keys)}\n"
                )
        os.chmod(repro, 0o755)
        print(f"# {len(all_fail)} failing cell(s); replay individually: {repro}")

    # One document for the whole matrix, so two runs at two commits can be
    # diffed directly (`jq -S '.results|map({key:(.harness+"/"+.combo+"/"+.id),
    # value:.status})|from_entries'`) instead of eyeballing two logs.
    summary_path = os.path.join(outdir, "matrix.json")
    with open(summary_path, "w") as f:
        json.dump({
            "schema": "parity-matrix/1",
            "profile": args.profile,
            "harnesses": harnesses,
            "combos": combos,
            "sequences": sequences,
            "tag": tag,
            "skip_optional": skip_optional,
            "summary": {"passed": tot_pass, "failed": tot_fail,
                        "combo_runs": len(grid),
                        "elapsed_seconds": round(elapsed, 1)},
            "combo_grid": [{"harness": h, "combo": c, "passed": p, "failed": f}
                           for h, c, p, f in grid],
            "results": all_results,
        }, f, indent=2)
    print(f"# matrix json: {summary_path}")

    # A failing combo whose PEERS pass isolates the axis, so surface that.
    failing = sorted({c for _h, c, _p, f in grid if f}) if grid else []
    passing = sorted({c for _h, c, _p, f in grid if not f})
    if failing:
        print(f"# combos with divergences: {', '.join(failing)}")
    if passing:
        print(f"# combos fully clean     : {', '.join(passing)}")
    return 1 if (tot_fail or random_rc) else 0


if __name__ == "__main__":
    raise SystemExit(main())

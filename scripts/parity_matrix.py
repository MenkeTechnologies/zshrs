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
import os
import re
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


def run_combo(py: str, harness: str, combo: str, sequences: list[str],
              tag: str | None, skip_optional: bool, extra: list[str],
              logdir: str, cases: list, jobs: int = 1
              ) -> tuple[int, int, list[tuple[str, str]], str]:
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
        cmd += extra
        log = os.path.join(logdir, f"{harness}.{combo}.log")
        with open(log, "w") as f:
            f.write("$ " + " ".join(cmd) + "\n\n")
            f.flush()
            rc = subprocess.run(cmd, stdout=f, stderr=subprocess.STDOUT).returncode
        p_, f_, cells = _scrape(log, rc)
        return p_, f_, cells, log

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
        cmd = base_cmd() + ["--corpus", corpus] + extra
        log = os.path.join(logdir, f"{harness}.{combo}.shard{i}.log")
        fh = open(log, "w")
        fh.write("$ " + " ".join(cmd) + "\n\n")
        fh.flush()
        procs.append((subprocess.Popen(cmd, stdout=fh, stderr=subprocess.STDOUT),
                      fh, log))

    passed = failed = 0
    cells: list[tuple[str, str]] = []
    for proc, fh, log in procs:
        rc = proc.wait()
        fh.close()
        p_, f_, c_ = _scrape(log, rc)
        passed += p_
        failed += f_
        cells += c_
    return passed, failed, cells, os.path.join(logdir, f"{harness}.{combo}.shard*.log")


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
    # Each cell boots two fresh interactive shells through compinit and waits
    # for both screens to settle. Measured ~30s/cell on this host with the
    # user's real dump; the boot dominates, not the keystrokes.
    # ~30s/cell is the measured floor (two shells booted through compinit per
    # cell); --jobs runs that many cells at once, so wall time divides by it.
    print(f"# estimate : ~{total * 30 / 60 / max(1, args.jobs):.0f} min "
          f"at ~30s/cell across {max(1, args.jobs)} job(s)")
    print(f"# outdir   : {outdir}")
    print()
    if args.dry_run:
        return 0

    started = time.monotonic()
    grid = []
    all_fail = []
    for harness in harnesses:
        for combo in combos:
            t0 = time.monotonic()
            passed, failed, cells, log = run_combo(
                py, harness, combo, sequences, tag, skip_optional, args.extra,
                outdir, cases, args.jobs)
            dt = time.monotonic() - t0
            status = "PASS" if failed == 0 else "FAIL"
            print(f"{status:4s} {harness:6s} {combo:28s} "
                  f"{passed:4d} pass {failed:4d} fail  {dt:6.1f}s  {os.path.basename(log)}")
            sys.stdout.flush()
            grid.append((harness, combo, passed, failed))
            for buf, keys in cells:
                all_fail.append((harness, combo, buf, keys))

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
                f.write(
                    f"{py} {os.path.join(SCRIPTS, script)} "
                    f"--zstyle {os.path.join(COMBOS, combo + '.zsh')} "
                    f"--case {buf} --keys {keys}\n"
                )
        os.chmod(repro, 0o755)
        print(f"# {len(all_fail)} failing cell(s); replay individually: {repro}")

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

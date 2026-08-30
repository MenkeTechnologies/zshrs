#!/usr/bin/env python3
"""Replay every checked-in completion-parity fixture and report what changed.

    scripts/compsys_regressions.py                    # every fixture
    scripts/compsys_regressions.py --list             # what is pinned, no shells
    scripts/compsys_regressions.py --only cc_match_set --only equals_word_line_rewrite
    scripts/compsys_regressions.py --variants         # replay the extra witnesses too

WHY THIS EXISTS
---------------
Two rounds of fuzzing minimised roughly a dozen divergences, and every one of
those reproducers was written under `target/` or `$TMPDIR`. `.gitignore:1` is
`/target`, so `cargo clean` — or the OS reaping /var/folders — destroyed the
entire evidence base. `tests/compsys_fixtures/*.json` is the durable form: one
file per divergence, carrying the buffer, the keys, the zstyle body or the
completer source, and the zsh-vs-zshrs difference AS OBSERVED. This script
replays them.

WHAT A VERDICT MEANS
--------------------
    STILL-DIVERGES  the fixture reproduces, with the fingerprint it recorded
    NOW-PASSES      the two shells now agree — the fixture is a false claim and
                    must be moved out of the bug set (this is a FAILURE of the
                    run, not a success: a fixture asserting a bug that no longer
                    exists is exactly the stale-ledger problem this replaces)
    TIMEOUT         a side ran out of measurement budget, so nothing was proven
    ERROR           the harness could not run the fixture, or refused to score it

Exit status is 0 only when every fixture STILL-DIVERGES with the fingerprint it
recorded. A behaviour change in EITHER direction is non-zero, because both
directions mean the checked-in evidence no longer describes reality.

HOW IT RUNS THEM
----------------
It shells out. It never imports `compsys_spec_fuzz`, `comptab_parity` or
`compsys_parity`: those three files are edited constantly, and their stdout
format has already changed mid-session, so this script talks to them only
through their `--json` documents and their exit status. Each fixture names the
harness that owns it:

    compsys_spec_fuzz   a hermetic generated completer — the fixture carries the
                        completer source, which is written back out as a
                        `--replay` reproducer in a temp dir
    comptab_parity      a real command against the user's dump — the fixture
                        carries the buffer, the keys and any zstyle statements

Each fixture boots two real shells on ptys, so budget 5-25s per fixture and
about the same again per variant. Runs are serial on purpose: the ledger this
replaces recorded that at --jobs 8..10 roughly 80% of its `failures` were the
debug build missing the harness's per-key budget rather than a divergence.
"""

import argparse
import glob
import json
import os
import shlex
import subprocess
import sys
import tempfile

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
FIXTURE_DIR = os.path.join(REPO, "tests", "compsys_fixtures")
SELF = os.path.relpath(os.path.abspath(__file__), REPO)

# spec-fuzz fixtures are replayed through a file in the shape `write_fixture()`
# emits; `read_fixture()` needs only the `@` headers and this heredoc marker.
HEREDOC = "SPEC_FUZZ_COMPLETER"

STILL, PASSES, TIMEOUT, ERROR = ("STILL-DIVERGES", "NOW-PASSES", "TIMEOUT", "ERROR")

HARNESS_SCRIPT = {
    "compsys_spec_fuzz": os.path.join(REPO, "scripts", "compsys_spec_fuzz.py"),
    "comptab_parity": os.path.join(REPO, "scripts", "comptab_parity.py"),
}


class Result:
    def __init__(self, fid, harness, verdict, detail, note="", command=""):
        self.fid = fid
        self.harness = harness
        self.verdict = verdict
        self.detail = detail
        self.note = note            # e.g. fingerprint drift, harness stderr tail
        self.command = command

    @property
    def ok(self):
        """A run only counts as unchanged when it diverged AND the shape held."""
        return self.verdict == STILL and not self.note


# ── fixtures ─────────────────────────────────────────────────────────────────

def load_fixtures(only):
    out = []
    for path in sorted(glob.glob(os.path.join(FIXTURE_DIR, "*.json"))):
        with open(path) as f:
            doc = json.load(f)
        if doc.get("schema") != "compsys-fixture/1":
            sys.exit("%s: unknown schema %r" % (path, doc.get("schema")))
        if only and doc["id"] not in only:
            continue
        doc["_path"] = path
        out.append(doc)
    if only:
        missing = only - {d["id"] for d in out}
        if missing:
            sys.exit("no such fixture: %s" % ", ".join(sorted(missing)))
    return out


def write_spec_reproducer(directory, run):
    """Re-materialise a `compsys_spec_fuzz --replay` file from the fixture.

    Only the `@` headers and the completer heredoc are read back by the
    harness, so this writes exactly those. The rest of what `write_fixture()`
    emits (the throwaway ZDOTDIR, the fpath list, the `exec zsh`) is for a
    human running the file by hand; the harness rebuilds all of it.
    """
    spec = run["spec"]
    path = os.path.join(directory, "%s.min.zsh" % spec["cmd"])
    with open(path, "w") as f:
        f.write("#!/usr/bin/env zsh\n"
                "# regenerated by %s from the checked-in fixture\n" % SELF)
        f.write("# @seed %d\n# @case %d\n# @cmd %s\n# @kind %s\n"
                % (spec.get("seed", 0), spec.get("case", -1), spec["cmd"],
                   spec.get("kind", "replay")))
        f.write("# @buffer %s\n# @keys %s\n"
                % (run["buffer"], ",".join(run["keys"])))
        for line in spec.get("setup", []):
            f.write("# @setup %s\n" % line)
        f.write("\ncat >$_d/fpath/_%s <<'%s'\n%s%s\n"
                % (spec["cmd"], HEREDOC, spec["completer"], HEREDOC))
    return path


def build_command(run, harness, directory, json_path):
    """The exact argv that replays one cell, plus the human-readable form."""
    script = HARNESS_SCRIPT[harness]
    argv = [sys.executable, script]
    if harness == "compsys_spec_fuzz":
        argv += ["--replay", write_spec_reproducer(directory, run)]
    else:
        argv += ["--case", run["buffer"], "--keys", ",".join(run["keys"])]
        statements = run.get("zstyle") or []
        if statements:
            zpath = os.path.join(directory, "zstyle.zsh")
            with open(zpath, "w") as f:
                f.write("\n".join(statements) + "\n")
            argv += ["--zstyle", zpath]
        if run.get("rows"):
            argv += ["--rows", str(run["rows"])]
        if run.get("cols"):
            argv += ["--cols", str(run["cols"])]
    argv += run.get("flags", [])
    argv += ["--json", json_path]
    return argv


# ── running one cell ─────────────────────────────────────────────────────────

def score(fid, run, harness, fingerprint, timeout, keep):
    """Run one cell (a fixture, or one of its variants) and return a Result.

    `fingerprint` is the shape this particular cell recorded, which is NOT
    always the fixture's: a variant is a different command reaching the same
    defect, and comptab_parity fingerprints the failure SHAPE, so the same
    defect seen through two commands legitimately carries two fingerprints.
    Pass None to record the verdict without asserting a shape.
    """
    directory = tempfile.mkdtemp(prefix="compsys_regressions_")
    json_path = os.path.join(directory, "result.json")
    argv = build_command(run, harness, directory, json_path)
    human = " ".join(shlex.quote(a) for a in argv)
    try:
        proc = subprocess.run(argv, cwd=REPO, capture_output=True, text=True,
                              timeout=timeout)
    except subprocess.TimeoutExpired:
        return Result(fid, harness, TIMEOUT,
                      "harness did not finish within %ds" % timeout,
                      command=human)
    if not os.path.exists(json_path):
        tail = (proc.stderr or proc.stdout or "").strip().splitlines()[-3:]
        return Result(fid, harness, ERROR,
                      "harness wrote no --json (exit %d)" % proc.returncode,
                      note=" / ".join(tail), command=human)
    try:
        with open(json_path) as f:
            got = json.load(f)
        results = got["results"]
    except (ValueError, KeyError) as exc:
        return Result(fid, harness, ERROR, "unreadable --json: %r" % exc,
                      command=human)
    if len(results) != 1:
        return Result(fid, harness, ERROR,
                      "expected 1 result, harness reported %d" % len(results),
                      command=human)
    r = results[0]
    status = r.get("status", "?")
    detail = (r.get("detail") or "").strip()

    if status in ("FAIL", "FLAKY"):
        verdict, note = STILL, ""
        # A fingerprint is comptab_parity's stable id for the failure SHAPE.
        # Still failing under a DIFFERENT shape is still a behaviour change:
        # the pinned evidence no longer describes what the shells do.
        if fingerprint is not None:
            now = r.get("fingerprint")
            if now != fingerprint:
                note = "fingerprint drift: recorded %s, now %s" % (fingerprint, now)
    elif status.startswith("PASS"):
        verdict, note = PASSES, "fixture asserts a divergence that no longer reproduces"
    elif status == "TIMEOUT":
        verdict, note = TIMEOUT, "; ".join(r.get("timeouts") or [])[:80]
    elif status == "SKIP":
        verdict = ERROR
        note = "harness skipped: %s" % (r.get("skip_reason") or detail or "?")
    else:
        verdict, note = ERROR, "unknown harness status %r" % status

    if keep:
        note = (note + "  " if note else "") + "artifacts: %s" % directory
    return Result(fid, harness, verdict, detail or status, note=note, command=human)


# ── reporting ────────────────────────────────────────────────────────────────

def print_listing(fixtures):
    print("# %d fixture(s) in %s" % (len(fixtures),
                                     os.path.relpath(FIXTURE_DIR, REPO)))
    for doc in fixtures:
        run = doc["run"]
        print("%-40s %-18s fp=%-11s variants=%d"
              % (doc["id"], doc["harness"], doc.get("fingerprint") or "-",
                 len(doc.get("variants") or [])))
        print("    %s" % doc["title"])
        print("    buffer=%r keys=%s%s"
              % (run["buffer"], ",".join(run["keys"]),
                 "  zstyle=%d stmt" % len(run["zstyle"]) if run.get("zstyle") else ""))
        print("    confirmed %s at %s" % (doc["confirmed"]["date"],
                                          doc["confirmed"]["commit"]))


def main():
    ap = argparse.ArgumentParser(
        description="replay the checked-in completion-parity fixtures")
    ap.add_argument("--only", action="append", default=[], metavar="ID",
                    help="run just this fixture (repeatable)")
    ap.add_argument("--variants", action="store_true",
                    help="also replay each fixture's extra witnesses; roughly "
                         "doubles the run time and is the honest way to check "
                         "that a whole family still diverges, not just its "
                         "smallest member")
    ap.add_argument("--timeout", type=float, default=300.0, metavar="SECS",
                    help="per-cell wall clock before the cell is scored TIMEOUT")
    ap.add_argument("--keep", action="store_true",
                    help="keep each cell's temp dir and name it in the report")
    ap.add_argument("--list", action="store_true",
                    help="print what is pinned and exit; boots no shells")
    args = ap.parse_args()

    fixtures = load_fixtures(set(args.only))
    if not fixtures:
        sys.exit("no fixtures under %s" % FIXTURE_DIR)
    if args.list:
        print_listing(fixtures)
        return 0

    print("# %s — %d fixture(s)%s" % (SELF, len(fixtures),
                                      ", variants included" if args.variants else ""))
    print("# fixtures: %s" % os.path.relpath(FIXTURE_DIR, REPO))
    print()
    print("%-52s %-15s %s" % ("FIXTURE", "VERDICT", "DETAIL"))
    print("-" * 108)
    sys.stdout.flush()

    results = []
    for doc in fixtures:
        r = score(doc["id"], doc["run"], doc["harness"], doc.get("fingerprint"),
                  args.timeout, args.keep)
        results.append(r)
        print("%-52s %-15s %s" % (r.fid, r.verdict, r.detail[:38]))
        if r.note:
            print("%-52s %-15s %s" % ("", "", r.note))
        sys.stdout.flush()
        if not args.variants:
            continue
        for i, variant in enumerate(doc.get("variants") or []):
            # A variant carries its own run fields; anything it omits falls
            # back to the fixture's, so a variant that only changes the buffer
            # does not have to restate the completer.
            run = dict(doc["run"])
            run.update({k: v for k, v in variant.items()
                        if k in ("buffer", "keys", "flags", "zstyle", "spec",
                                 "rows", "cols")})
            vr = score("  %s/variant%d" % (doc["id"], i), run, doc["harness"],
                       variant.get("fingerprint"), args.timeout, args.keep)
            results.append(vr)
            print("%-52s %-15s %s" % (vr.fid, vr.verdict, vr.detail[:38]))
            if vr.note:
                print("%-52s %-15s %s" % ("", "", vr.note))
            sys.stdout.flush()

    counts = {}
    for r in results:
        counts[r.verdict] = counts.get(r.verdict, 0) + 1
    changed = [r for r in results if not r.ok]

    print()
    print("=" * 108)
    print("# %d cell(s): %s"
          % (len(results),
             ", ".join("%d %s" % (counts[k], k) for k in sorted(counts))))
    if changed:
        print("# %d cell(s) no longer match the checked-in evidence:" % len(changed))
        for r in changed:
            print("#   %-52s %-15s %s" % (r.fid.strip(), r.verdict,
                                          r.note or r.detail))
            print("#     %s" % r.command)
        print("#")
        print("# A NOW-PASSES is not good news to ignore: the fixture claims a "
              "divergence")
        print("# that no longer exists. Move it out of the bug set with the run "
              "that shows")
        print("# it passing, the way scripts/comptab_divergent_cases.txt does.")
    else:
        print("# every fixture still diverges with the shape it recorded")
    return 1 if changed else 0


if __name__ == "__main__":
    sys.exit(main())

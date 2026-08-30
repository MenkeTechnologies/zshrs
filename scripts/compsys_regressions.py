#!/usr/bin/env python3
"""One gate over the checked-in completion-parity evidence base.

    scripts/compsys_regressions.py                  # the full sweep
    scripts/compsys_regressions.py --quick          # the short subset
    scripts/compsys_regressions.py --list           # what is pinned, no shells
    scripts/compsys_regressions.py --json out.json  # machine-readable result
    scripts/compsys_regressions.py --only cc_match_set --only equals_word_line_rewrite
    scripts/compsys_regressions.py --reference-defects   # incl. the zsh-crash guard

Exit status
    0   every attempted cell still behaves the way the fixtures record
    1   something MOVED — a fixture no longer diverges, a control that used to
        agree now diverges, a fingerprint drifted, or the pinned upstream zsh
        crash stopped happening
    2   the RUNNER could not answer — a harness errored, a cell ran out of
        budget, the zshrs binary is missing, a fixture is unreadable. 2 wins
        over 1 when both happen: an incomplete run cannot certify the rest.

WHY THIS EXISTS
---------------
Two rounds of fuzzing minimised roughly a dozen divergences, and every one of
those reproducers was written under `target/` or `$TMPDIR`. `.gitignore:1` is
`/target`, so `cargo clean` — or the OS reaping /var/folders — destroys the
entire evidence base. That is not hypothetical: during round 4 a peer instance
deleted `target/` to recover a full disk and took every round-3 reproducer with
it mid-session. `tests/compsys_fixtures/*.json` is the durable form: one file
per finding, carrying the buffer, the keys, the zstyle body or the completer
source, and the difference AS OBSERVED. This script replays them.

WHAT A VERDICT MEANS
--------------------
    STILL-DIVERGES  the fixture reproduces, with the shape it recorded
    NOW-PASSES      the two shells now agree. A FAILURE of the run either way,
                    but the report says WHICH kind: if the zshrs binary is
                    unchanged since the fixture's stamp the fixture was wrong;
                    if the binary was rebuilt, someone most likely fixed the
                    bug and the fixture should be retired with that run
    CONTROL-HOLDS   a cell the fixture pins as AGREEING still agrees. Controls
                    are what make a fixture's variable the variable: the
                    continuation fixtures each carry the same completion on one
                    physical line, the widget fixtures carry the same completer
                    through the default TAB binding
    CONTROL-MOVED   that control now diverges, so the fixture no longer
                    isolates what it claims to isolate
    REF-CRASHES     the pinned UPSTREAM zsh defect still reproduces
    REF-SURVIVES    it does not — the reference shell survived; see the fixture
    SKIPPED         an opt-in cell that was not run (see --reference-defects)
    TIMEOUT         a side ran out of measurement budget, so nothing was proven
    ERROR           the harness could not run the cell, or refused to score it

HOW IT RUNS THEM
----------------
It shells out. It never imports `compsys_spec_fuzz`, `comptab_parity` or
`compsys_parity`: those three files are edited constantly — twice during round 4
a replay hit one of them mid-edit and died in its own argument parsing — and
their stdout format has already changed under this script once. It talks to
them only through their `--json` documents and their exit status. Each fixture
names the harness that owns it:

    compsys_spec_fuzz     a hermetic generated completer — the fixture carries
                          the completer source and the widget declaration,
                          written back out as a `--replay` reproducer
    comptab_parity        a real command against the user's dump — buffer, keys
                          and any zstyle statements
    compsys_parity        a `--lockstep` cell — same, and the only harness whose
                          buffer may contain a newline (the continuation cells)
    zsh_reference_probe   tests/compsys_fixtures/zsh_reference_probe.py, which
                          asks only about the REFERENCE shell and never boots
                          zshrs at all
    winch_probe           tests/compsys_fixtures/winch_probe.py — completes,
                          then CHANGES THE WINDOW SIZE mid-session. The three
                          sibling harnesses set the geometry once before the
                          shell boots and never touch it again, so none of them
                          can reach a defect that needs a resize
    shell_probe           tests/compsys_fixtures/shell_probe.py — one script,
                          two shells, NO pty. For a finding whose reproducer is
                          a script rather than a keystroke: a parameter bug that
                          reprices what compsys renders is isolated better by
                          two lines of `print -l` than by a screen that also
                          carries a prompt, a geometry and a listing layout

`--harness-dir DIR` points the first three at another copy of the scripts (e.g.
`git show HEAD:scripts/comptab_parity.py > $DIR/comptab_parity.py`) for when the
working tree is mid-edit. The directory used is recorded in the JSON document,
because a verdict is only as identified as the harness that produced it. Every
harness that boots zshrs is passed `--zshrs` explicitly, so a copy living
outside the repo still runs the intended binary — each harness otherwise
resolves `target/debug/zshrs` relative to its OWN file and a copy under /tmp
finds nothing at all — and so the binary the report stamps is the binary the
cells ran.

Each cell boots two real shells on ptys, so budget 8-25s per cell. `--jobs`
runs cells concurrently — they are independent pty pairs — but the DEFAULT IS 1
on evidence: the ledger this replaces recorded that at --jobs 8..10 roughly 80%
of its `failures` were the debug build missing the harness's per-key budget
rather than a divergence, and compsys_parity refuses --jobs > 1 outright while
measuring. The jobs value is written into the JSON so a result gathered under
load says so.
"""

import argparse
import glob
import hashlib
import json
import os
import shlex
import subprocess
import sys
import tempfile
import time

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
FIXTURE_DIR = os.path.join(REPO, "tests", "compsys_fixtures")
SELF = os.path.relpath(os.path.abspath(__file__), REPO)
SCHEMAS = ("compsys-fixture/1", "compsys-fixture/2")
# The two documents in the fixture directory that are NOT fixtures: the group
# table the consolidated report renders from, and the last full gate run, kept
# beside the evidence so every number in that report is re-derivable from a
# file. Skipped by NAME — an unknown schema in an actual fixture still stops
# the run, which is the point of validating at all.
NON_FIXTURE_FILES = ("groups.json", "last_gate.json")

# spec-fuzz fixtures are replayed through a file in the shape `write_fixture()`
# emits; `read_fixture()` needs only the `@` headers and this heredoc marker.
HEREDOC = "SPEC_FUZZ_COMPLETER"

STILL = "STILL-DIVERGES"
PASSES = "NOW-PASSES"
CONTROL_OK = "CONTROL-HOLDS"
CONTROL_MOVED = "CONTROL-MOVED"
REF_CRASHES = "REF-CRASHES"
REF_SURVIVES = "REF-SURVIVES"
SKIPPED = "SKIPPED"
TIMEOUT = "TIMEOUT"
ERROR = "ERROR"

UNCHANGED = (STILL, CONTROL_OK, REF_CRASHES)
MOVED = (PASSES, CONTROL_MOVED, REF_SURVIVES)
RUNNER_FAILED = (TIMEOUT, ERROR)

SIBLING_HARNESSES = ("compsys_spec_fuzz", "comptab_parity", "compsys_parity")
PROBE_HARNESS = "zsh_reference_probe"
# Replayers that live in the fixture directory, not in scripts/: they are part
# of the evidence base and are versioned with it, so --harness-dir does not
# point at them.
SHELL_HARNESS = "shell_probe"
WINCH_HARNESS = "winch_probe"
LOCAL_HARNESSES = (PROBE_HARNESS, SHELL_HARNESS, WINCH_HARNESS)
# Both of these take the fixture's whole `run` block as a JSON file rather than
# a flag per field, so a control or a variant that overrides one field needs no
# plumbing here at all.
RUN_BLOCK_HARNESSES = (SHELL_HARNESS, WINCH_HARNESS)

# The --quick subset. Chosen for COVERAGE (one cell per harness, plus the two
# cheapest high-signal fixtures) and for run time, not by any measurement — it
# is a smoke test, and a green --quick is never evidence that the full sweep is
# green. Add an id here when a new fixture covers a harness or a defect class
# nothing else in this list reaches.
QUICK_IDS = (
    "equals_word_line_rewrite",           # comptab_parity, ~9s, zsh -f defaults
    "compstate_old_list_shown",           # compsys_spec_fuzz, the widget axis
    "multiline_dquote_parameter_list",    # compsys_parity, a continuation cell
    "cc_match_set",                       # comptab_parity, a match-set diff
)


# ── binary identity ──────────────────────────────────────────────────────────
#
# A fixture asserts that two shells disagree. When one of them is rebuilt from
# newer source — and this tree gets zshrs fixes committed to main all day — a
# fixture can flip to NOW-PASSES for an entirely legitimate reason. That is a
# different fact from "the fixture was wrong", and it is cheap to tell apart, so
# the runner identifies the binary it ran and compares it with the one each
# fixture was confirmed against.

def binary_identity(path, want_hash=True):
    if not os.path.exists(path):
        return None
    st = os.stat(path)
    ident = {
        "path": os.path.relpath(path, REPO) if path.startswith(REPO) else path,
        "size": st.st_size,
        "mtime": time.strftime("%F %T", time.localtime(st.st_mtime)),
        "version": None,
        "sha256_16": None,
    }
    try:
        out = subprocess.run([path, "--version"], capture_output=True,
                             text=True, timeout=20)
        ident["version"] = (out.stdout or out.stderr).strip().splitlines()[0]
    except Exception:
        pass
    if want_hash:
        h = hashlib.sha256()
        with open(path, "rb") as f:
            for block in iter(lambda: f.read(1 << 20), b""):
                h.update(block)
        ident["sha256_16"] = h.hexdigest()[:16]
    return ident


def binary_drift(stamped, current):
    """(changed, human) — did the binary move since this fixture was confirmed?

    Returns changed=None when the fixture predates binary stamping (schema 1),
    because "unknown" must not be reported as "unchanged".
    """
    if not stamped:
        return None, ("this fixture carries no binary stamp (schema 1), so "
                      "whether the binary changed since it was confirmed is "
                      "unknown")
    if not current:
        return None, "no zshrs binary present to compare against"
    for key in ("sha256_16", "size"):
        if stamped.get(key) and current.get(key):
            if stamped[key] != current[key]:
                return True, ("zshrs binary CHANGED since the stamp (%s %s -> %s"
                              ", mtime %s -> %s): a rebuild is the likely reason"
                              " this no longer reproduces"
                              % (key, stamped[key], current[key],
                                 stamped.get("mtime"), current.get("mtime")))
            return False, ("zshrs binary UNCHANGED since the stamp (%s %s): the "
                           "fixture is asserting something the shells do not do"
                           % (key, stamped[key]))
    return None, "the stamp carries no comparable field"


# ── fixtures ─────────────────────────────────────────────────────────────────

class Cell:
    """One replay: a fixture, one of its variants, or one of its controls."""

    def __init__(self, fid, role, harness, run, expect, fingerprint, note="",
                 fixture=None):
        self.fid = fid
        self.role = role                 # fixture | variant | control
        self.harness = harness
        self.run = run
        self.expect = expect             # diverges | agrees | reference-crash
        self.fingerprint = fingerprint
        self.note = note
        self.fixture = fixture           # the owning document


class Result:
    def __init__(self, cell, verdict, detail, note="", command="", raw=None):
        self.cell = cell
        self.verdict = verdict
        self.detail = detail
        self.note = note                 # drift, harness stderr tail, ...
        self.command = command
        self.raw = raw or {}
        self.attempts = []          # verdicts of every attempt, when >1 was run

    @property
    def ok(self):
        return self.verdict in UNCHANGED and not self.note.startswith("fingerprint drift")

    def to_json(self):
        return {
            "id": self.cell.fid,
            "role": self.cell.role,
            "harness": self.cell.harness,
            "expect": self.cell.expect,
            "verdict": self.verdict,
            "detail": self.detail,
            "note": self.note,
            "command": self.command,
            "fingerprint_recorded": self.cell.fingerprint,
            "fingerprint_now": self.raw.get("fingerprint"),
            "attempts": self.attempts,
        }


def load_fixtures(only):
    out = []
    for path in sorted(glob.glob(os.path.join(FIXTURE_DIR, "*.json"))):
        if os.path.basename(path) in NON_FIXTURE_FILES:
            continue
        with open(path) as f:
            try:
                doc = json.load(f)
            except ValueError as exc:
                sys.exit("%s: unreadable fixture: %s" % (path, exc))
        if doc.get("schema") not in SCHEMAS:
            sys.exit("%s: unknown schema %r (have %s)"
                     % (path, doc.get("schema"), ", ".join(SCHEMAS)))
        if only and doc["id"] not in only:
            continue
        doc["_path"] = path
        out.append(doc)
    if only:
        missing = only - {d["id"] for d in out}
        if missing:
            sys.exit("no such fixture: %s" % ", ".join(sorted(missing)))
    return out


def merged(base, override, keys=("buffer", "keys", "flags", "zstyle",
                                 "zstyle_file", "rows", "cols", "word",
                                 "control_word", "trials", "script", "files",
                                 "dirs", "argv", "env", "compare_stderr",
                                 "zsh_flags", "zshrs_flags", "new_rows",
                                 "new_cols")):
    """A variant/control restates only what it changes; `spec` merges per field.

    Merging `spec` rather than replacing it is what lets a control say
    `{"widget": "default", "setup": []}` and inherit the completer verbatim —
    which is the whole point of a control: one variable moved, nothing else.
    """
    run = dict(base)
    run.update({k: v for k, v in override.items() if k in keys})
    if "spec" in override:
        spec = dict(base.get("spec") or {})
        spec.update(override["spec"])
        run["spec"] = spec
    return run


def cells_of(doc, want_variants, want_controls, want_reference):
    """Every cell a fixture contributes to this run, in report order."""
    expect = doc.get("expect", "diverges")
    out = []
    if expect == "reference-crash" and not (want_reference or doc.get("default_run")):
        out.append(Cell(doc["id"], "fixture", doc["harness"], doc["run"], expect,
                        doc.get("fingerprint"), note="opt-in", fixture=doc))
        return out
    out.append(Cell(doc["id"], "fixture", doc["harness"], doc["run"], expect,
                    doc.get("fingerprint"), fixture=doc))
    if want_controls:
        for i, ctl in enumerate(doc.get("controls") or []):
            out.append(Cell("  %s/control%d" % (doc["id"], i), "control",
                            doc["harness"], merged(doc["run"], ctl), "agrees",
                            ctl.get("fingerprint"), note=ctl.get("note", ""),
                            fixture=doc))
    if want_variants:
        for i, var in enumerate(doc.get("variants") or []):
            out.append(Cell("  %s/variant%d" % (doc["id"], i), "variant",
                            doc["harness"], merged(doc["run"], var), expect,
                            var.get("fingerprint"), note=var.get("note", ""),
                            fixture=doc))
    return out


# ── building one harness invocation ──────────────────────────────────────────

def write_spec_reproducer(directory, run):
    """Re-materialise a `compsys_spec_fuzz --replay` file from the fixture.

    Only the `@` headers and the completer heredoc are read back by the
    harness, so this writes exactly those. `@widget` and the `@setup` lines are
    load-bearing: the widget declaration IS the variable in three of the
    fixtures, and a reproducer without them silently replays the default TAB
    binding — which is those fixtures' own control, i.e. it would report a
    green PASS for the wrong reason.
    """
    spec = run["spec"]
    path = os.path.join(directory, "%s.min.zsh" % spec["cmd"])
    with open(path, "w") as f:
        f.write("#!/usr/bin/env zsh\n"
                "# regenerated by %s from the checked-in fixture\n" % SELF)
        f.write("# @seed %d\n# @case %d\n# @cmd %s\n# @kind %s\n# @widget %s\n"
                % (spec.get("seed", 0), spec.get("case", -1), spec["cmd"],
                   spec.get("kind", "replay"), spec.get("widget", "default")))
        f.write("# @buffer %s\n# @keys %s\n"
                % (run["buffer"], ",".join(run["keys"])))
        for line in spec.get("setup", []):
            f.write("# @setup %s\n" % line)
        f.write("\ncat >$_d/fpath/_%s <<'%s'\n%s%s\n"
                % (spec["cmd"], HEREDOC, spec["completer"], HEREDOC))
    return path


def zstyle_argv(run, directory, harness):
    """--zstyle / --no-zstyle for one cell.

    `zstyle_file` names a file in the repo (the checked-in fixture); `zstyle` is
    a list of statements written to a temp file. compsys_parity DEFAULTS to the
    repo fixture, so a cell that needs no styles must say `--no-zstyle` out
    loud rather than by omission.
    """
    if run.get("zstyle_file"):
        return ["--zstyle", os.path.join(REPO, run["zstyle_file"])]
    statements = run.get("zstyle") or []
    if statements:
        path = os.path.join(directory, "zstyle.zsh")
        with open(path, "w") as f:
            f.write("\n".join(statements) + "\n")
        return ["--zstyle", path]
    if harness == "compsys_parity":
        return ["--no-zstyle"]
    return []


def build_command(cell, directory, json_path, harness_script, zshrs):
    """The exact argv that replays one cell.

    `zshrs` is handed to every harness that boots it. Without that the flag was
    a STAMP ONLY: each harness resolved its own `target/debug/zshrs` relative to
    its own file, so `--zshrs /opt/homebrew/bin/zshrs` produced a report that
    identified the Homebrew binary while every cell had actually run the debug
    one — the report attributing results to a binary that did not produce them
    is the one failure this file's binary-identity code exists to prevent. It
    also repairs the `--harness-dir` workflow the README documents: a copy of
    the harnesses taken outside the repo (`git show HEAD:scripts/... > /tmp/h`)
    computes REPO from its own path and cannot find the binary at all, which is
    a hard ERROR on every cell.

    `zsh_reference_probe` is the exception: it boots only zsh and has no such
    flag (tests/compsys_fixtures/zsh_reference_probe.py:165-177).
    """
    run, harness = cell.run, cell.harness
    argv = [sys.executable, harness_script[harness]]
    if harness == "compsys_spec_fuzz":
        argv += ["--replay", write_spec_reproducer(directory, run)]
    elif harness == PROBE_HARNESS:
        argv += ["--word", run["word"], "--trials", str(run.get("trials", 3))]
        if run.get("control_word"):
            argv += ["--control", run["control_word"]]
    elif harness in RUN_BLOCK_HARNESSES:
        # The whole `run` block IS the reproducer for these cells, so it is
        # handed over verbatim rather than flattened into flags.
        path = os.path.join(directory, "run.json")
        with open(path, "w") as f:
            json.dump(run, f)
        argv += ["--run", path]
    else:
        argv += ["--case", run["buffer"], "--keys", ",".join(run["keys"])]
        if harness == "compsys_parity":
            argv += ["--lockstep"]
        argv += zstyle_argv(run, directory, harness)
        if run.get("rows"):
            argv += ["--rows", str(run["rows"])]
        if run.get("cols"):
            argv += ["--cols", str(run["cols"])]
    if harness != PROBE_HARNESS:
        argv += ["--zshrs", zshrs]
    argv += run.get("flags", [])
    argv += ["--json", json_path]
    return argv


# ── scoring one cell ─────────────────────────────────────────────────────────

def score_probe(cell, got):
    """The reference-defect fixture: a claim about zsh, scored on its own terms."""
    subject, control = got.get("subject") or {}, got.get("control")
    crashed, n = subject.get("crashed", 0), subject.get("n", 0)
    if control and control.get("crashed"):
        return (ERROR, "the CONTROL word crashed too (%d/%d)"
                % (control["crashed"], control["n"]),
                "a machine that kills shells on the control word cannot "
                "measure this; nothing is proven either way")
    detail = "%d/%d trial(s) killed by a signal%s" % (
        crashed, n, " (%s)" % ",".join(subject.get("signals") or [])
        if subject.get("signals") else "")
    if crashed:
        return REF_CRASHES, detail, ""
    return (REF_SURVIVES, detail,
            "the pinned UPSTREAM zsh defect did not reproduce: the reference "
            "shell survived every trial. Re-read the fixture before deleting "
            "anything — a zsh upgrade would explain it, and the seven cells it "
            "covers could then re-enter the sweep")


def parity_detail(r):
    """One line describing a harness result, whichever harness produced it.

    comptab_parity and compsys_spec_fuzz write a `detail` string;
    compsys_parity's lockstep document does not, and printing a bare `FAIL`
    for a 24-row divergence loses the only number in the report that says how
    big it is.
    """
    detail = (r.get("detail") or "").strip()
    if detail:
        return detail
    if r.get("diff_rows") is not None:
        fd = r.get("first_diff") or {}
        return ("%d row(s) differ from step #%s (%s), first row %s col %s"
                % (len(r["diff_rows"]), r.get("fail_step"),
                   r.get("fail_key"), fd.get("row"), fd.get("col")))
    return r.get("status", "?")


def score_parity(cell, r, current_binary):
    """A two-shell cell, scored against what the fixture expects of it."""
    status = r.get("status", "?")
    detail = parity_detail(r)
    agrees = cell.expect == "agrees"

    if status in ("FAIL", "FLAKY"):
        if agrees:
            return (CONTROL_MOVED, detail or status,
                    "this cell is pinned as AGREEING — it is what makes the "
                    "fixture's variable the variable — and it now diverges, so "
                    "the fixture no longer isolates what it claims to")
        note = ""
        # A fingerprint is comptab_parity's stable id for the failure SHAPE.
        # Still failing under a DIFFERENT shape is still a behaviour change:
        # the pinned evidence no longer describes what the shells do.
        if cell.fingerprint is not None:
            now = r.get("fingerprint")
            if now != cell.fingerprint:
                note = ("fingerprint drift: recorded %s, now %s"
                        % (cell.fingerprint, now))
        return STILL, detail or status, note
    if status.startswith("PASS"):
        if agrees:
            return CONTROL_OK, detail or status, ""
        changed, human = binary_drift(
            (cell.fixture.get("confirmed") or {}).get("zshrs_binary"),
            current_binary)
        lead = ("fixture asserts a divergence that no longer reproduces; %s"
                % human)
        return PASSES, detail or status, lead
    if status == "TIMEOUT":
        return TIMEOUT, detail or status, "; ".join(r.get("timeouts") or [])[:120]
    if status in ("REF-CRASHED", "TEST-CRASHED"):
        return (ERROR, detail or status,
                "a shell CRASHED, so no comparison happened — this is neither a "
                "pass nor a divergence. If it is the REFERENCE shell, see "
                "reference_crash_uppercase_autoload.json")
    if status == "SKIP":
        return ERROR, detail or status, ("harness skipped: %s"
                                         % (r.get("skip_reason") or detail or "?"))
    return ERROR, detail or status, "unknown harness status %r" % status


def score(cell, args, harness_script, current_binary):
    directory = tempfile.mkdtemp(prefix="compsys_regressions_")
    json_path = os.path.join(directory, "result.json")
    argv = build_command(cell, directory, json_path, harness_script,
                         args.zshrs)
    human = " ".join(shlex.quote(a) for a in argv)
    try:
        proc = subprocess.run(argv, cwd=REPO, capture_output=True, text=True,
                              timeout=args.timeout)
    except subprocess.TimeoutExpired:
        return Result(cell, TIMEOUT,
                      "harness did not finish within %ds" % args.timeout,
                      command=human)
    if not os.path.exists(json_path):
        tail = (proc.stderr or proc.stdout or "").strip().splitlines()[-3:]
        return Result(cell, ERROR,
                      "harness wrote no --json (exit %d)" % proc.returncode,
                      note=" / ".join(tail), command=human)
    try:
        with open(json_path) as f:
            got = json.load(f)
    except ValueError as exc:
        return Result(cell, ERROR, "unreadable --json: %r" % exc, command=human)

    if cell.harness == PROBE_HARNESS:
        verdict, detail, note = score_probe(cell, got)
        raw = got.get("subject") or {}
    else:
        results = got.get("results")
        if not isinstance(results, list) or len(results) != 1:
            return Result(cell, ERROR,
                          "expected 1 result, harness reported %s"
                          % (len(results) if isinstance(results, list) else "none"),
                          command=human)
        raw = results[0]
        verdict, detail, note = score_parity(cell, raw, current_binary)

    if args.keep:
        note = (note + "  " if note else "") + "artifacts: %s" % directory
    return Result(cell, verdict, detail, note=note, command=human, raw=raw)


def score_replicated(cell, args, harness_script, current_binary):
    """Score a cell, and REPLICATE any verdict that says the evidence moved.

    Not a softener — the opposite. A single-shot verdict is not reliable on this
    machine: the round-4 full sweep ran at a load average of 45 (sixteen peer
    sessions, cargo builds, other fuzzers) and scored
    `narrow_terminal_error_redraw` NOW-PASSES with a zero-row diff, i.e. both
    shells drew the same incomplete screen. Re-run three times immediately
    afterwards it went TIMEOUT, STILL-DIVERGES, STILL-DIVERGES. Reporting the
    first of those as "the fixture is a false claim" would have retired a live
    finding on a scheduling artefact.

    So: a verdict that the evidence is UNCHANGED is taken at face value (it is
    the harder result to produce by accident — the two shells had to disagree in
    the recorded shape). A verdict that something MOVED, or that the cell could
    not be scored at all, is re-run up to `--confirm-moved` times, and every
    attempt is reported. A cell that needed a re-run is named in the summary
    even when it ends up unchanged: intermittency is itself a fact about the
    measurement and must not be swallowed.
    """
    attempts = []
    r = score(cell, args, harness_script, current_binary)
    attempts.append(r)
    while (r.verdict in MOVED or r.verdict in RUNNER_FAILED) \
            and len(attempts) <= args.confirm_moved:
        r = score(cell, args, harness_script, current_binary)
        attempts.append(r)
        if r.verdict in UNCHANGED:
            break
    final = attempts[-1]
    if len(attempts) > 1:
        final.attempts = [a.verdict for a in attempts]
        seq = " -> ".join(final.attempts)
        if final.verdict in UNCHANGED:
            final.note = ("NONDETERMINISTIC: %s. The recorded behaviour did "
                          "reproduce, so the evidence stands, but this cell is "
                          "marginal on this machine — %s"
                          % (seq, attempts[0].note or attempts[0].detail))
        else:
            final.note = ("%s (re-ran %d times, it never came back). %s"
                          % (seq, len(attempts) - 1, final.note))
    return final


# ── reporting ────────────────────────────────────────────────────────────────

def print_listing(fixtures):
    print("# %d fixture(s) in %s" % (len(fixtures),
                                     os.path.relpath(FIXTURE_DIR, REPO)))
    for doc in fixtures:
        run = doc["run"]
        opt = "" if doc.get("expect") != "reference-crash" else "  [opt-in]"
        print("%-40s %-20s %-16s fp=%-11s variants=%d controls=%d%s"
              % (doc["id"], doc["harness"], doc.get("expect", "diverges"),
                 doc.get("fingerprint") or "-",
                 len(doc.get("variants") or []),
                 len(doc.get("controls") or []), opt))
        print("    %s" % doc["title"])
        if "script" in run:
            print("    script=%d line(s): %s"
                  % (len(run["script"]), " ; ".join(run["script"])[:96]))
        elif "word" in run:
            print("    word=%r control=%r trials=%s"
                  % (run["word"], run.get("control_word"), run.get("trials")))
        else:
            print("    buffer=%r keys=%s%s%s"
                  % (run["buffer"], ",".join(run["keys"]),
                     "  zstyle=%d stmt" % len(run["zstyle"])
                     if run.get("zstyle") else "",
                     "  zstyle_file=%s" % run["zstyle_file"]
                     if run.get("zstyle_file") else ""))
        c = doc.get("confirmed") or {}
        print("    confirmed %s at %s%s"
              % (c.get("date"), c.get("commit"),
                 "  binary %s %s" % (c["zshrs_binary"].get("sha256_16"),
                                     c["zshrs_binary"].get("mtime"))
                 if c.get("zshrs_binary") else "  (no binary stamp)"))


def main():
    ap = argparse.ArgumentParser(
        description="replay the checked-in completion-parity fixtures and "
                    "report whether the pinned evidence still holds")
    ap.add_argument("--only", action="append", default=[], metavar="ID",
                    help="run just this fixture (repeatable)")
    ap.add_argument("--quick", action="store_true",
                    help="run the short subset (%s) — a smoke test, never "
                         "evidence that the full sweep is green"
                         % ", ".join(QUICK_IDS))
    ap.add_argument("--variants", action="store_true",
                    help="also replay each fixture's extra witnesses; roughly "
                         "doubles the run time and is the honest way to check "
                         "that a whole family still diverges, not just its "
                         "smallest member")
    ap.add_argument("--no-controls", dest="controls", action="store_false",
                    default=True,
                    help="skip the cells a fixture pins as AGREEING. They are "
                         "on by default because a fixture whose control has "
                         "started diverging is no longer isolating anything")
    ap.add_argument("--reference-defects", action="store_true",
                    help="also run the fixtures that assert an UPSTREAM defect "
                         "in the reference shell. Off by default: they crash a "
                         "shell on purpose and make the OS write a crash report "
                         "every trial, and they can prove nothing about zshrs. "
                         "Skipped cells are reported and counted as neither "
                         "unchanged nor moved")
    ap.add_argument("--jobs", type=int, default=1, metavar="N",
                    help="cells to run concurrently. DEFAULT 1 on evidence: at "
                         "--jobs 8..10 roughly 80%% of the old ledger's "
                         "`failures` were the debug build missing a per-key "
                         "budget under load, not divergences. The value used is "
                         "recorded in --json")
    ap.add_argument("--confirm-moved", type=int, default=2, metavar="N",
                    help="extra attempts for a cell that scores as MOVED or "
                         "could not be scored. A single-shot verdict is not "
                         "reliable here: the round-4 sweep scored a live "
                         "fixture NOW-PASSES at load average 45 and it "
                         "reproduced on the very next run. Every attempt is "
                         "reported; 0 disables replication")
    ap.add_argument("--timeout", type=float, default=300.0, metavar="SECS",
                    help="per-cell wall clock before the cell is scored TIMEOUT")
    ap.add_argument("--harness-dir", default=os.path.join(REPO, "scripts"),
                    metavar="DIR",
                    help="where to find the sibling harnesses (default: "
                         "scripts/). Point it at a copy when the working tree "
                         "is mid-edit: git show HEAD:scripts/comptab_parity.py "
                         "> $DIR/comptab_parity.py")
    ap.add_argument("--zshrs", default=os.path.join(REPO, "target", "debug", "zshrs"),
                    help="the zshrs every cell runs, and whose identity is "
                         "stamped into the report. Point it at an OLDER build "
                         "to check that a fixture retired as a guard (expect "
                         "`agrees`) really would catch the bug coming back: "
                         "the guard should report CONTROL-MOVED there")
    ap.add_argument("--no-binary-hash", dest="binary_hash",
                    action="store_false", default=True,
                    help="identify the binary by size+mtime+version only")
    ap.add_argument("--keep", action="store_true",
                    help="keep each cell's temp dir and name it in the report")
    ap.add_argument("--json", default=None, metavar="PATH",
                    help="write the machine-readable result document")
    ap.add_argument("--list", action="store_true",
                    help="print what is pinned and exit; boots no shells")
    args = ap.parse_args()

    fixtures = load_fixtures(set(args.only))
    if args.quick:
        fixtures = [d for d in fixtures if d["id"] in QUICK_IDS]
    if not fixtures:
        sys.exit("no fixtures under %s" % FIXTURE_DIR)
    if args.list:
        print_listing(fixtures)
        return 0

    harness_script = {name: os.path.join(args.harness_dir, "%s.py" % name)
                      for name in SIBLING_HARNESSES}
    for name in LOCAL_HARNESSES:
        harness_script[name] = os.path.join(FIXTURE_DIR, "%s.py" % name)

    cells = []
    for doc in fixtures:
        cells += cells_of(doc, args.variants, args.controls,
                          args.reference_defects)
    needs_shells = [c for c in cells if c.note != "opt-in"]

    # Preflight. A missing binary is a RUNNER failure, not a fixture result —
    # and it is a real one: `target/` was deleted out from under this round.
    current_binary = binary_identity(args.zshrs, args.binary_hash)
    if current_binary is None and any(c.harness != PROBE_HARNESS
                                      for c in needs_shells):
        print("# %s: %s does not exist — nothing can be replayed.\n"
              "# (`cargo clean`, or a peer instance reclaiming disk, removes "
              "target/ wholesale.)" % (SELF, args.zshrs), file=sys.stderr)
        return 2
    for name in sorted({c.harness for c in needs_shells}):
        if not os.path.exists(harness_script[name]):
            print("# %s: harness %s not found at %s"
                  % (SELF, name, harness_script[name]), file=sys.stderr)
            return 2

    started = time.time()
    print("# %s — %d fixture(s), %d cell(s)%s%s"
          % (SELF, len(fixtures), len(cells),
             ", variants included" if args.variants else "",
             ", controls included" if args.controls else ""))
    print("# fixtures : %s" % os.path.relpath(FIXTURE_DIR, REPO))
    print("# harnesses: %s" % args.harness_dir)
    print("# zshrs    : %s" % (
        "%s  %s  %s bytes  %s" % (current_binary["path"],
                                  current_binary.get("version"),
                                  current_binary["size"],
                                  current_binary.get("sha256_16") or
                                  current_binary["mtime"])
        if current_binary else "<absent>"))
    print("# jobs     : %d%s" % (args.jobs,
                                 "   (concurrent — a marginal cell can miss a "
                                 "per-key budget under load)"
                                 if args.jobs > 1 else ""))
    print()
    print("%-52s %-15s %s" % ("CELL", "VERDICT", "DETAIL"))
    print("-" * 108)
    sys.stdout.flush()

    results = []

    def emit(r):
        print("%-52s %-15s %s" % (r.cell.fid, r.verdict, r.detail[:38]))
        if r.note:
            print("%-52s %-15s %s" % ("", "", r.note))
        sys.stdout.flush()

    def run_cell(cell):
        if cell.note == "opt-in":
            return Result(cell, SKIPPED, "not run by default",
                          note="an UPSTREAM reference-shell defect; run it with "
                               "--reference-defects")
        return score_replicated(cell, args, harness_script, current_binary)

    if args.jobs > 1:
        from concurrent.futures import ThreadPoolExecutor
        with ThreadPoolExecutor(max_workers=args.jobs) as pool:
            for r in pool.map(run_cell, cells):
                results.append(r)
                emit(r)
    else:
        for cell in cells:
            r = run_cell(cell)
            results.append(r)
            emit(r)

    counts = {}
    for r in results:
        counts[r.verdict] = counts.get(r.verdict, 0) + 1
    moved = [r for r in results if r.verdict in MOVED or
             r.note.startswith("fingerprint drift")]
    failed = [r for r in results if r.verdict in RUNNER_FAILED]
    skipped = [r for r in results if r.verdict == SKIPPED]
    unchanged = [r for r in results if r.ok]

    rc = 2 if failed else (1 if moved else 0)

    print()
    print("=" * 108)
    print("# %d cell(s) in %.1fs: %s"
          % (len(results), time.time() - started,
             ", ".join("%d %s" % (counts[k], k) for k in sorted(counts))))
    print("# %d unchanged, %d moved, %d could not be scored, %d skipped"
          % (len(unchanged), len(moved), len(failed), len(skipped)))
    for label, group in (("no longer match the checked-in evidence", moved),
                         ("could not be scored", failed)):
        if not group:
            continue
        print("# %d cell(s) %s:" % (len(group), label))
        for r in group:
            print("#   %-52s %-15s %s" % (r.cell.fid.strip(), r.verdict,
                                          r.note or r.detail))
            print("#     %s" % r.command)
    flaky = [r for r in results if r.attempts and r.verdict in UNCHANGED]
    if flaky:
        print("# %d cell(s) NEEDED A RE-RUN — marginal on this machine, "
              "reported unchanged because the recorded behaviour did reproduce:"
              % len(flaky))
        for r in flaky:
            print("#   %-52s %s" % (r.cell.fid.strip(), " -> ".join(r.attempts)))
    if skipped:
        print("# %d opt-in cell(s) skipped (--reference-defects runs them): %s"
              % (len(skipped), ", ".join(r.cell.fid for r in skipped)))
    if moved:
        print("#")
        print("# A NOW-PASSES is not good news to ignore: the fixture claims a "
              "divergence that")
        print("# no longer exists. The note above says whether the zshrs binary "
              "moved since the")
        print("# fixture was stamped — unchanged binary means the fixture was "
              "wrong, a rebuilt")
        print("# one most likely means somebody fixed the bug. Retire it with "
              "the run that shows")
        print("# the change, the way scripts/comptab_divergent_cases.txt does.")
    if not moved and not failed:
        print("# every attempted cell still behaves the way the fixtures record")
    print("# exit %d" % rc)

    if args.json:
        doc = {
            "schema": "compsys-regressions/1",
            "runner": SELF,
            "started": time.strftime("%FT%T", time.localtime(started)),
            "seconds": round(time.time() - started, 1),
            "argv": sys.argv[1:],
            "harness_dir": args.harness_dir,
            "jobs": args.jobs,
            "quick": args.quick,
            "variants": args.variants,
            "controls": args.controls,
            "reference_defects": args.reference_defects,
            "zshrs_binary": current_binary,
            "fixtures": [d["id"] for d in fixtures],
            "load_average": os.getloadavg(),
            "summary": {
                "cells": len(results),
                "replicated": len(flaky),
                "unchanged": len(unchanged),
                "moved": len(moved),
                "unscored": len(failed),
                "skipped": len(skipped),
                "by_verdict": counts,
            },
            "results": [r.to_json() for r in results],
            "exit": rc,
        }
        with open(args.json, "w") as f:
            json.dump(doc, f, indent=1)
            f.write("\n")
        print("# json: %s" % args.json)
    return rc


if __name__ == "__main__":
    sys.exit(main())
